//! Embedded Bevy renderer bridge — M1 POC of
//! `docs/bevy-viewport-integration.md`.
//!
//! One windowless Bevy `App` lives inside the `MultiPipeline`, initialized
//! with `RenderCreation::Manual` on iced's own wgpu device (published by the
//! iced fork through `iced::wgpu_external`), with winit disabled and its
//! `SubApps` pumped manually. `Primitive::prepare` pumps it only for shaded
//! 3D viewports on frames that actually re-render — 2D-only drawings and
//! cached frames never touch Bevy.
//!
//! Per viewport slot the bridge owns an offscreen color target (an
//! OCS-created wgpu texture registered via `ManualTextureViews`, so Bevy
//! renders straight into it — zero copies) and a `Camera3d` whose custom
//! projection mirrors the OCS camera exactly, wrapped in a z-flip: Bevy's
//! pipelines are reversed-z, so `z_bevy = 1 - z_ocs` and the composite pass
//! (`bevy_composite.wgsl`) recovers the OCS depth losslessly.
//!
//! Runtime opt-in: build with `--features bevy3d` and set `OCS_BEVY3D=1`.

use std::sync::Arc;

use bevy::app::{App, PluginGroup};
use bevy::prelude::*;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, WgpuWrapper,
};
use bevy::render::settings::{RenderCreation, RenderResources};
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::view::ViewDepthStencilTexture;
use bevy::render::sync_world::MainEntity;
use bevy::render::{RenderApp, RenderPlugin};
use bevy::camera::{ManualTextureViewHandle, RenderTarget};
use bevy::window::ExitCondition;
use iced::wgpu;
use rustc_hash::FxHashMap;

/// `OCS_BEVY3D=1` opts the running process into the POC path; a `bevy3d`
/// build without it behaves exactly like a stock build.
pub fn runtime_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OCS_BEVY3D").is_some_and(|v| v != "0"))
}

/// Camera state for one viewport, captured while the scene builds its
/// `ViewportData` (where the authoritative OCS `Camera` is at hand).
#[derive(Debug, Clone)]
pub struct BevyCamData {
    /// Full-precision eye; the POC narrows it to f32 (the M2 mirror layer
    /// will rebase chunks before this matters).
    pub eye: glam::DVec3,
    /// Camera-to-world rotation — OCS and Bevy share the -Z-forward/+Y-up
    /// camera convention, so the arcball quaternion maps across unchanged.
    pub rotation: glam::Quat,
    /// Orbit pivot; the POC test solid spawns here.
    pub target: glam::DVec3,
    pub distance: f32,
    /// Reversed-z clip-from-view: `reverse_z(crop * OPENGL_TO_WGPU * proj)`.
    pub clip_from_view: glam::Mat4,
    /// Far plane in world units (Bevy uses it for light clustering).
    pub far: f32,
}

/// Post-multiply flip mapping wgpu NDC z to its complement: `z' = w - z`,
/// i.e. `z'_ndc = 1 - z_ndc` exactly (w untouched). Bevy clears depth to 0
/// and tests `GreaterEqual`; wrapping the OCS projection in this flip makes
/// both renderers rasterize bit-identical footprints with exactly
/// complementary depth values.
pub fn reverse_z(clip_from_view: glam::Mat4) -> glam::Mat4 {
    glam::Mat4::from_cols(
        glam::vec4(1.0, 0.0, 0.0, 0.0),
        glam::vec4(0.0, 1.0, 0.0, 0.0),
        glam::vec4(0.0, 0.0, -1.0, 0.0),
        glam::vec4(0.0, 0.0, 1.0, 1.0),
    ) * clip_from_view
}

/// The OCS camera handed to Bevy as-is: a fixed clip-from-view matrix. The
/// bridge recomputes it every sync, so `update` (Bevy's aspect callback) has
/// nothing to do.
#[derive(Debug, Clone)]
struct OcsProjection {
    clip_from_view: glam::Mat4,
    far: f32,
}

impl bevy::camera::CameraProjection for OcsProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        self.clip_from_view
    }

    fn get_clip_from_view_for_sub(&self, _sub_view: &bevy::camera::SubCameraView) -> Mat4 {
        self.clip_from_view
    }

    fn update(&mut self, _width: f32, _height: f32) {}

    fn far(&self) -> f32 {
        self.far
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [glam::Vec3A; 8] {
        // Unproject the NDC rect at the two requested view-space depths.
        let inv = self.clip_from_view.inverse();
        let ndc_z = |z_view: f32| {
            let clip = self.clip_from_view * glam::vec4(0.0, 0.0, z_view, 1.0);
            if clip.w.abs() < 1e-20 {
                0.0
            } else {
                clip.z / clip.w
            }
        };
        let (zn, zf) = (ndc_z(z_near), ndc_z(z_far));
        let corner =
            |x: f32, y: f32, z: f32| glam::Vec3A::from(inv.project_point3(glam::vec3(x, y, z)));
        [
            corner(1.0, -1.0, zn),
            corner(1.0, 1.0, zn),
            corner(-1.0, 1.0, zn),
            corner(-1.0, -1.0, zn),
            corner(1.0, -1.0, zf),
            corner(1.0, 1.0, zf),
            corner(-1.0, 1.0, zf),
            corner(-1.0, -1.0, zf),
        ]
    }
}

/// One viewport slot's Bevy-side state.
struct BridgeView {
    camera: Entity,
    handle: ManualTextureViewHandle,
    /// OCS-owned color target Bevy renders into (kept alive here; the
    /// `ManualTextureViews` entry and the composite bind group hold views).
    _color_texture: wgpu::Texture,
    color_view: wgpu::TextureView,
    size: (u32, u32),
}

/// Views the composite pass binds for one pumped frame.
pub struct BevyFrame {
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
}

pub struct BevyBridge {
    sub_apps: bevy::app::SubApps,
    views: FxHashMap<u64, BridgeView>,
    next_view_id: u32,
    scene_spawned: bool,
    headlight: Option<Entity>,
}

impl BevyBridge {
    /// Build the windowless Bevy app on iced's device. Returns `None` when
    /// the compositor has not published its GPU handles yet (never the case
    /// by the time a shader Primitive prepares, but harmless to skip).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let shared = iced::wgpu_external::gpu_handles()?;
        let render_resources = RenderResources(
            RenderDevice::from(device.clone()),
            RenderQueue(Arc::new(WgpuWrapper::new(queue.clone()))),
            RenderAdapterInfo(WgpuWrapper::new(shared.adapter.get_info())),
            RenderAdapter(Arc::new(WgpuWrapper::new(shared.adapter.clone()))),
            RenderInstance(Arc::new(WgpuWrapper::new(shared.instance.clone()))),
        );

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                // No window: iced owns the event loop; DefaultPlugins carries
                // no winit because the `bevy_winit` feature is off.
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..Default::default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Manual(Box::new(render_resources)),
                    // Predictable first frame for the POC; async compilation
                    // + preloading is an M3 concern.
                    synchronous_pipeline_compilation: true,
                    debug_flags: Default::default(),
                }),
        );
        // No runner: finish + cleanup, then take the sub-apps and pump them
        // manually (the externally-driven-headless pattern, bevy #22551).
        app.finish();
        app.cleanup();
        let sub_apps = std::mem::take(app.sub_apps_mut());

        Some(Self {
            sub_apps,
            views: FxHashMap::default(),
            next_view_id: 1,
            scene_spawned: false,
            headlight: None,
        })
    }

    /// Mirror the OCS camera into this slot's Bevy camera, pump one frame,
    /// and hand back the color + depth views for the composite pass.
    pub fn sync_and_pump(
        &mut self,
        device: &wgpu::Device,
        slot_key: u64,
        size: (u32, u32),
        cam: &BevyCamData,
    ) -> Option<BevyFrame> {
        self.ensure_scene(cam);
        self.ensure_view(device, slot_key, size);

        let camera_entity = self.views.get(&slot_key)?.camera;
        let color_view = self.views.get(&slot_key)?.color_view.clone();

        {
            let views = &self.views;
            let headlight = self.headlight;
            let world = self.sub_apps.main.world_mut();
            // Exactly one bridge camera renders per pump: prepare() is called
            // per widget (per pane), so each pump draws only its own slot.
            for (key, view) in views.iter() {
                if let Some(mut camera) = world.get_mut::<Camera>(view.camera) {
                    camera.is_active = *key == slot_key;
                }
            }
            let mut entity = world.entity_mut(camera_entity);
            if let Some(mut transform) = entity.get_mut::<Transform>() {
                *transform = Transform::from_translation(cam.eye.as_vec3())
                    .with_rotation(cam.rotation);
            }
            if let Some(mut projection) = entity.get_mut::<Projection>() {
                *projection = Projection::custom(OcsProjection {
                    clip_from_view: cam.clip_from_view,
                    far: cam.far.max(1.0),
                });
            }
            // Headlight: follow the view so the POC solid always reads.
            if let Some(light) = headlight {
                if let Some(mut transform) = world.get_mut::<Transform>(light) {
                    *transform = Transform::from_rotation(cam.rotation);
                }
            }
        }

        self.sub_apps.update();

        // Bevy's depth texture for this view, straight from the render world
        // (entities survive until the next extract clears them). Usage
        // includes TEXTURE_BINDING via `Camera3d::depth_texture_usages`.
        let render_app = self
            .sub_apps
            .sub_apps
            .get_mut(&bevy::app::AppLabel::intern(&RenderApp))?;
        let world = render_app.world_mut();
        let mut query = world.query::<(&MainEntity, &ViewDepthStencilTexture)>();
        let mut depth_view = None;
        for (main_entity, depth) in query.iter(world) {
            if main_entity.id() == camera_entity {
                // Bevy wraps its resources; deref down to the raw wgpu
                // texture so the view is bindable by the OCS pipelines.
                let raw_texture: &wgpu::Texture = depth.texture();
                depth_view =
                    Some(raw_texture.create_view(&wgpu::TextureViewDescriptor::default()));
                break;
            }
        }
        Some(BevyFrame {
            color_view,
            depth_view: depth_view?,
        })
    }

    /// Spawn the hardcoded M1 test scene the first time a 3D viewport pumps:
    /// one solid at the orbit target, sized from the fitted distance, plus a
    /// camera-following directional light.
    fn ensure_scene(&mut self, cam: &BevyCamData) {
        if self.scene_spawned {
            return;
        }
        self.scene_spawned = true;
        let center = cam.target.as_vec3();
        let scale = (cam.distance * 0.25).max(1e-3);
        let world = self.sub_apps.main.world_mut();
        let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.0, 1.0, 1.0));
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgb(0.75, 0.49, 0.17),
                perceptual_roughness: 0.4,
                metallic: 0.05,
                ..Default::default()
            });
        world.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(center).with_scale(Vec3::splat(scale)),
        ));
        self.headlight = Some(
            world
                .spawn((
                    DirectionalLight {
                        illuminance: 6_000.0,
                        ..Default::default()
                    },
                    Transform::from_rotation(cam.rotation),
                ))
                .id(),
        );
    }

    /// Create or resize this slot's offscreen color target and camera. The
    /// target is an OCS texture registered through `ManualTextureViews`, so
    /// Bevy renders directly into memory the composite pass can bind.
    fn ensure_view(&mut self, device: &wgpu::Device, slot_key: u64, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if let Some(view) = self.views.get(&slot_key) {
            if view.size == size {
                return;
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bevy3d.color_target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let existing = self.views.get(&slot_key).map(|view| (view.handle, view.camera));
        let next_view_id = &mut self.next_view_id;
        let world = self.sub_apps.main.world_mut();
        let (handle, camera) = match existing {
            Some(pair) => pair,
            None => {
                let handle = ManualTextureViewHandle(*next_view_id);
                *next_view_id += 1;
                let camera = world
                    .spawn((
                        Camera3d {
                            // Bindable depth so the OCS overlay passes can
                            // occlude against the shaded solids.
                            depth_texture_usages: (wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING)
                                .into(),
                            ..Default::default()
                        },
                        Camera {
                            // Transparent clear: the composite keys coverage
                            // off depth, and OCS keeps its own background.
                            clear_color: ClearColorConfig::Custom(Color::NONE),
                            ..Default::default()
                        },
                        RenderTarget::TextureView(handle),
                        // CAD-style fill light so faces pointing away from
                        // the headlight still read.
                        AmbientLight {
                            color: Color::WHITE,
                            brightness: 400.0,
                            ..Default::default()
                        },
                        Msaa::Off,
                        Transform::IDENTITY,
                    ))
                    .id();
                (handle, camera)
            }
        };
        world.resource_mut::<ManualTextureViews>().insert(
            handle,
            ManualTextureView {
                texture_view: color_view.clone().into(),
                size: UVec2::new(size.0, size.1),
                view_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            },
        );
        self.views.insert(
            slot_key,
            BridgeView {
                camera,
                handle,
                _color_texture: texture,
                color_view,
                size,
            },
        );
    }
}

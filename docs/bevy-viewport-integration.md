# Embedding Bevy as the 3D shaded-solid renderer

Status: **in progress — M0 done, M1 implemented (visual verification
pending), M2–M4 not started.** Feasibility review and integration plan for
handing the 3D *shaded-solid* rendering layer to an embedded Bevy renderer
while iced stays the application host and every CAD-grade pipeline (wide
lines, linetypes, hatches, wipeout, SDF text, snapping) stays in-house.

The decisions below were settled one-by-one with the maintainer in a design
review on 2026-08-07. Every technical claim was verified against the two
local checkouts:

- OpenCADStudio @ `9c9a202c`
- Bevy checkout `D:\work\plant-code\cad\bevy`, branch `main` @ `e8b3598ff`
  (version `0.20.0-dev`, clean, no local patches)

## Goal / motivation

Get modern 3D rendering (PBR materials, clustered lights, SSAO/TAA, shadows)
and **large-model performance** — the target is *tens of millions of
triangles* (a full plant unit) at interactive walkthrough rates — without
maintaining that renderer ourselves. Bevy's GPU-driven pipeline, mesh/material
asset model, and render-graph do that work upstream.

## Non-goals

- Replacing the 2D drafting pipelines. Wide lines, linetypes, hatches,
  wipeout, clip masks, SDF text are CAD-grade and stay as-is.
- Migrating the application to Bevy ECS, Bevy UI, or Bevy's windowing. iced
  keeps the window, the event loop, and all widgets.
- GPU picking. Picking/snapping stays CPU-side in `src/scene/pick/`.
- Replacing the ViewCube (stays on the in-house pipeline).

## Verified facts

### OpenCADStudio render architecture

- **The seam**: the viewport is an iced `shader::Program` widget
  (`src/scene/view/viewport_pane.rs`). `Scene::build_viewports` /
  `build_viewport_for_pane` produce a `Primitive` (`src/scene/view/render.rs`);
  each viewport draws into its own scissor rect via a dedicated inner
  `Pipeline` held by the `MultiPipeline`. iced hands its own wgpu
  device/queue to `prepare()`/`render()` — this is where Bevy's output can be
  composited without any cross-device copy.
- **In-house renderer**: `src/scene/pipeline/mod.rs` (~4,600 lines) plus 17
  WGSL shaders in `src/shaders/`: background, blit, clip_mask, face3d,
  hatch + hatch_texture, image, mesh + mesh_cull (GPU culling), shadow
  (2048 px shadow map), text (SDF), viewcube (+text/composite), wipeout,
  wire + wire_indexed. Already tiered three ways by device capability:
  native / WebGPU / WebGL2 (packed compat mode when storage buffers are
  unavailable).
- **Precision model**: meshes carry double-single vertex pairs
  (`verts` + `verts_low`, `src/scene/model/mesh_model.rs`) and the camera
  target is `f64` (`src/scene/view/camera.rs`) so UTM-scale coordinates stay
  stable. Any replacement renderer must preserve this.
- **LOD**: per-mesh 3-level ladder (projected diagonal >200 px / 50–200 px /
  <50 px, `MeshLodSet`) plus view-dependent silhouettes (`CurvedGen`,
  DISPSILH).
- **Resolved versions** (Cargo.lock): wgpu **29.0.4**, winit 0.30.8. iced is
  pinned to git rev `23604ff`; the wasm build uses iced's `webgl` feature.

### Bevy (local checkout, main @ `e8b3598ff`)

- **wgpu 30** on main (`crates/bevy_render/Cargo.toml:87`). The released
  0.19.0 (2026-06) uses wgpu 29.0.3 — same major as iced today — but per
  decision 4 we track main instead and lift iced to wgpu 30.
- **External device injection is first-class**:
  `RenderCreation::Manual(Box<RenderResources>)`
  (`crates/bevy_render/src/settings.rs:222`) initializes the renderer from an
  externally created instance/adapter/device/queue → sharing iced's device is
  a supported configuration, not a hack.
- **Externally driven headless rendering is an official example**:
  `examples/app/externally_driven_headless_renderer.rs` (merged 2026-01,
  #22551, present since v0.19.0). Pattern: disable `WinitPlugin`,
  `WindowPlugin { primary_window: None, exit_condition: DontExit }`, call
  `app.finish()` + `app.cleanup()`, take `std::mem::take(app.sub_apps_mut())`,
  then pump `SubApps::update()` manually once per frame; the camera renders to
  an `Image::new_target_texture(..)` render target.
- **wgpu 29 → 30 cost datum**: Bevy's own bump was commit `5036d978a` —
  38 files, +78/−59, short breaking list (`get_mapped_range` returns
  `Result`, `SurfaceTexture::present` moved to `Queue::present`, optional
  vertex-buffer slots).
- **License**: MIT/Apache-2.0 — no obstacle inside a GPL-3.0 project.

## Decision log (settled 2026-08-07)

| # | Question | Decision |
|---|----------|----------|
| 1 | Motivation | 3D capability + large-model performance. 2D drafting rendering unchanged. |
| 2 | Platforms | Desktop **and** web must both ship. WebGL2 degradation accepted; WebGPU wanted. |
| 3 | Embedding shape | **Texture embedding**: iced stays host; Bevy runs windowless on the *shared* wgpu device, renders to an offscreen texture composited into the viewport. |
| 4 | Version strategy | Track the local **bevy main** checkout (wgpu 30) and maintain our own iced wgpu-30 port, rather than pinning bevy 0.19 (wgpu 29). |
| 5 | Rendering split | Bevy draws the **shaded-solid layer only**. Wireframes, text, snap markers, 2D entities, hatches, ViewCube stay on in-house pipelines, composited with a **shared depth buffer**. |
| 6 | Scale target | Tens of millions of triangles; whole plant unit walkthrough stays interactive. |
| 7 | Web delivery | **Two wasm artifacts** — WebGPU-first + WebGL2 fallback — selected by a front-end loader via `navigator.gpu`. |
| 8 | Sequencing | Port iced to wgpu 30 **first**, then build the POC directly on bevy main (no throwaway 0.19 prototype). |

## Architecture

### Embedding mechanism

```text
iced event loop (winit, unchanged)
  └─ ViewportPane (shader::Program)
       └─ Primitive::prepare(device, queue, ...)        ← iced's wgpu device
            ├─ BevyBridge (once): RenderPlugin {
            │     render_creation: RenderCreation::Manual(   ← same device/queue
            │         instance/adapter/device/queue from iced)
            │   }, WinitPlugin disabled, no primary window,
            │   SubApps taken and pumped manually
            ├─ per frame, when 3D scene/camera dirty:
            │     sync camera → bevy Camera3d (custom projection)
            │     apply mirror deltas (add/modify/delete)
            │     SubApps::update()   → offscreen color + depth textures
            └─ Primitive::render(...):
                  OCS overlay passes (wire/text/snap/2D/silhouette)
                    bind bevy's depth texture for occlusion
                  blit bevy color + overlays into the viewport scissor rect
```

Key properties:

- One `wgpu::Device`/`Queue` for everything → Bevy's output textures are
  directly bindable by the in-house pipelines. No CPU round-trip, no
  cross-device sync.
- Bevy is *pulled*, not free-running: `SubApps::update()` runs only when the
  3D viewport actually needs a new frame (camera moved / mirror dirty /
  resize). 2D-only redraws never touch Bevy.
- `synchronous_pipeline_compilation: true` for the POC (predictable first
  frame); revisit async compilation + preloading in M3.

### Rendering split

| Layer | Renderer |
|-------|----------|
| Shaded solids (PBR, lights, shadows, later SSAO/TAA) | **Bevy** |
| Wireframe edges, wide lines, linetypes | in-house (`wire*`) |
| SDF text, annotations | in-house (`text`) |
| Snap markers, selection highlight, grips | in-house |
| 2D entities shown inside the 3D view | in-house |
| Hatches, wipeout, clip masks, images | in-house |
| View-dependent silhouettes (DISPSILH) | in-house |
| ViewCube | in-house |

Compositing rule: Bevy renders color + depth offscreen; every in-house
overlay pass depth-tests against **Bevy's depth texture**, so linework and
solids occlude each other correctly in a single composite.

### Mirror layer (OCS scene → Bevy ECS)

- **Retained mirror**, never rebuilt per frame. Stable map: entity handle
  (`MeshModel.name`, the decimal handle string) ↔ `bevy::Entity`.
- Tessellation output (`convert/truck_tess.rs`, `MeshLodSet`) registered as
  **shared `Mesh` assets**: block references and multiply-referenced solids
  share one mesh with per-instance transforms.
- Change propagation is incremental: add/modify/delete/transform deltas from
  the command layer patch the mirror; a transform-only edit touches only a
  `Transform` component.
- Materials: layer/entity material and the per-triangle
  `triangle_material_handles` map into a `StandardMaterial` table; the active
  visual style (2D wireframe / shaded / x-ray …) drives material swaps and
  layer visibility toggles. X-ray and wireframe styles remain in-house
  overlay work.

### Precision (UTM-scale coordinates)

Bevy's world space is plain `f32`; feeding UTM-magnitude coordinates in
directly would jitter. The mirror **rebases geometry into spatial chunks**:

- each chunk gets a local origin; vertex positions are made chunk-relative in
  `f64` during mirroring, then narrowed to `f32`;
- the camera transform is rebased against the same origin each frame (the
  OCS camera already keeps an `f64` target, so no precision is lost before
  the subtraction);
- chunk size is an open tuning question (see below) — too large re-introduces
  jitter, too small multiplies draw batches.

This mirrors what the in-house pipeline achieves with its double-single
(`verts`/`verts_low`) encoding, moved one level up into the mirror.

### Camera and picking

- The OCS `Camera` (arcball, `f64` target, per-frame ortho depth-range from
  `model_bounds`) stays **authoritative**. Each frame the mirror computes
  view/projection in rebased space and writes them to the Bevy camera as a
  custom projection. No Bevy input or camera-controller plugins.
- Picking, snapping, grips, selection stay exactly where they are
  (`src/scene/pick/`), CPU-side, driven by the authoritative camera.
  `bevy_picking` is not used.

## Web build (dual artifacts)

- On wasm, Bevy's WebGL2 vs WebGPU backend is a **compile-time** choice, so
  one artifact cannot serve both. We ship two:
  - **WebGPU artifact**: Bevy with the webgpu backend; iced drops its
    `webgl` feature so both stacks share the one WebGPU device. Near-desktop
    feature set; this is the artifact the 3D goals actually apply to.
  - **WebGL2 artifact**: current iced `webgl` + Bevy webgl2. Accepted
    downgrades: no compute (no GPU-driven path), simplified shadows, LOD
    caps. The tens-of-millions target explicitly does **not** hold here.
- A small front-end loader picks the artifact via `navigator.gpu` feature
  detection.
- Escape hatch if `RenderCreation::Manual` device sharing proves fragile on
  the GL backend: the WebGL2 artifact alone may keep the legacy in-house
  shaded-mesh path (no Bevy) — the split in decision 5 keeps that door open,
  since the in-house pipelines remain complete.

## Milestones

### M0 — iced on wgpu 30

Fork iced at rev `23604ff`, bump `iced_wgpu` to wgpu 30 (breaking list per
Bevy's own migration: `get_mapped_range` → `Result`, `Queue::present`,
optional vertex-buffer slots). Apply the same call-site fixes to the in-house
pipeline (~4,600 lines + 17 shaders use `iced::wgpu` directly).

*Accept:* native + wasm(webgl) builds pass; existing scenes render with no
visual regression; fork rebase procedure documented.

*Status 2026-08-07:* implemented — see `docs/wgpu-30-fork.md` (forks, full
breaking-point list, rebase procedure). The port also required forking
`cryoglyph` (iced's text renderer, also on wgpu 29) and moving the `web-sys`
pin to the wasm-bindgen 0.2.115 generation. Native check, wasm check,
naga-30 shader validation tests, and a runtime startup smoke all pass; the
per-drawing visual regression (human eyes) is still pending.

### M1 — embed POC on bevy main (desktop)

Bevy main as a path dependency. `BevyBridge` built inside the viewport
`Primitive`: `RenderCreation::Manual` with iced's device/queue, WinitPlugin
disabled, `SubApps` pumped manually. One hardcoded solid rendered to an
offscreen target; OCS wire overlay binds Bevy's depth; camera synced from the
OCS `Camera`.

*Accept:* orbit a test solid with the in-house wireframe overlay occluding
correctly; zero CPU copies per frame; survives viewport resize and pane
splits; 2D-only drawings never pump Bevy.

*Status 2026-08-10:* implemented behind the `bevy3d` cargo feature +
`OCS_BEVY3D=1` env opt-in; native `cargo check` passes with and without the
feature. Architecture as planned, with the details settled during
implementation:

- **Bridge** (`src/scene/pipeline/bevy_bridge.rs`): one windowless Bevy app
  per `MultiPipeline` (no winit — the `bevy_winit` feature is simply off),
  booted via `RenderCreation::Manual` from the GPU handles the iced fork now
  publishes (`iced::wgpu_external`, see `wgpu-30-fork.md`). Pumped from
  `Primitive::prepare`, gated on the scene-render-cache signature, so only
  shaded 3D viewports on frames that actually re-render touch Bevy.
- **Targets**: per-slot OCS-created color texture registered through
  `ManualTextureViews` (Bevy renders straight into it); Bevy's own depth
  texture is fetched from the render world after each pump
  (`Camera3d::depth_texture_usages` += `TEXTURE_BINDING`). Zero CPU copies.
- **Camera**: `Projection::custom` mirrors the exact OCS projection
  (including the off-canvas sub-rect crop) wrapped in an NDC z-flip
  (`z' = w − z`), because Bevy is reversed-z. Bevy depth is therefore
  exactly `1 − z_ocs`.
- **Composite** (`src/shaders/bevy_composite.wgsl`): fullscreen pass between
  the background/hatch pass and the solid/wire passes, `textureLoad`s Bevy
  color + depth, discards uncovered pixels (reversed-z clear = 0), writes
  `frag_depth = 1 − d`. Every later overlay pass depth-tests against the
  solids with no pipeline changes.
- The M1 test solid is hardcoded in the bridge: a cube at the camera target
  sized from the fitted distance, headlight + per-camera ambient.

Remaining for the accept gate: the human-eyes runtime pass (orbit + overlay
occlusion + resize + pane splits) on a real drawing.

### M2 — mirror layer

Handle ↔ Entity map, truck tessellation → shared `Mesh` assets, incremental
add/modify/delete/transform, material/visual-style mapping, chunk rebasing.

*Accept:* open a real DWG with solids; edits reflect in the shaded layer
within one frame; visual parity checklist against the current shaded mode
passes; UTM-located drawing shows no jitter.

### M3 — scale performance

Instance sharing for block references, Bevy's GPU-driven multi-draw path on
native/WebGPU, integration with (or replacement of) the in-house LOD ladder,
async pipeline compilation + shader preloading.

*Accept:* a tens-of-millions-of-triangles plant model orbits/walks at
interactive rates (≥30 fps) on the reference desktop GPU (to be nominated).

### M4 — web dual artifacts

Trunk builds both wasm artifacts; WebGPU artifact moves iced off `webgl`;
WebGL2 artifact wires the degraded tier (or the legacy-path escape hatch);
loader selects by `navigator.gpu`.

*Accept:* the M1 scene runs in Chrome (WebGPU) and Firefox (WebGL2
fallback); auto-selection verified on both.

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| iced wgpu-30 fork maintenance until upstream catches up | ongoing rebase tax | keep the port minimal (`iced_wgpu` only); document rebase steps in M0; upstream the patch as a PR when iced starts its own bump |
| bevy main API churn (0.20-dev) | recurring breakage | pin to a known-good commit, advance deliberately; migration guides ship in-tree (`_release-content/`) |
| `Manual` device injection fragile on wasm/WebGL2 | web fallback broken | escape hatch: WebGL2 artifact keeps the legacy in-house shaded path; WebGPU artifact is the primary web target |
| Depth-texture sharing constraints on WebGL2 (sampling depth is restricted) | overlay occlusion wrong on the fallback tier | depth copy to a color target, or reconstruct from linear depth in overlay shaders — WebGL2 tier only |
| Chunk-rebase seams (cracks between chunks at extreme zoom) | visual artifacts on UTM drawings | chunk-size experiments in M2 accept gate; chunks aligned to drawing extents, not a fixed grid |
| First-frame pipeline-compilation stutter | poor first impression | synchronous compilation for POC; async + preload in M3 |
| Full-mirror build time on first open (truck tessellation throughput) | slow open on big DWGs | reuse existing `MeshLodSet` output (tessellation already happens today); mirror construction is copy + upload, streamed by chunk |
| Bevy renders linework poorly (out of scope creep) | scope creep | decision 5 is a hard boundary: linework/text never move to Bevy in this effort |

## Open questions (not blocking start)

- Chunk size / origin policy for the rebase (needs experiments with real
  UTM-located drawings in M2).
- Whether Bevy `visibility_range` replaces the in-house LOD ladder or the
  mirror keeps driving LOD selection per `MeshLodSet` (M3).
- SSAO/TAA/clipping-plane rollout order after M3.
- Reference hardware for the M3 acceptance test.

## References

- Seam: `src/scene/view/viewport_pane.rs`, `src/scene/view/render.rs`,
  `src/scene/pipeline/mod.rs`
- Precision: `src/scene/model/mesh_model.rs`, `src/scene/view/camera.rs`
- Bevy manual renderer init: `crates/bevy_render/src/settings.rs`
  (`RenderCreation::Manual`)
- Bevy externally driven headless rendering:
  `examples/app/externally_driven_headless_renderer.rs` (#22551)
- Bevy's own wgpu 29→30 bump: commit `5036d978a` (cost datum for the iced
  port)

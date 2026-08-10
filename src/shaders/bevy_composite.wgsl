// Composite the embedded Bevy renderer's offscreen frame (bevy3d feature)
// into the viewport's MSAA color + depth buffers.
//
// Bevy renders the shaded-solid layer to a single-sample color target plus
// its own reversed-z depth texture, both sized exactly to the viewport (the
// OCS attachments are 128-px-grid over-allocated, so coordinates match only
// inside the set_viewport rect — which is where this pass draws). Coverage
// keys off depth: cleared reversed-z is 0.0 (= far), so any pixel the solid
// touched carries depth > 0. The fragment writes the OCS-convention depth
// `1 - z_bevy` (exact complement of the flipped projection the bridge hands
// Bevy), letting every later wire / text / mesh pass depth-test against the
// solids as if the in-house renderer had drawn them.

@group(0) @binding(0) var bevy_color: texture_2d<f32>;
@group(0) @binding(1) var bevy_depth: texture_depth_2d;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    // Fullscreen triangle.
    var out: VsOut;
    let x = f32(i32(idx & 1u) * 4 - 1);
    let y = f32(i32(idx >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let dims = vec2<i32>(textureDimensions(bevy_depth));
    let px = clamp(vec2<i32>(in.pos.xy), vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    let d = textureLoad(bevy_depth, px, 0);
    if (d <= 0.0) {
        discard;
    }
    var out: FsOut;
    out.color = textureLoad(bevy_color, px, 0);
    out.depth = 1.0 - d;
    return out;
}

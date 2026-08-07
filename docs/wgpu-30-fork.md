# wgpu-30 forks of iced and cryoglyph

Status: **active.** As of 2026-08-07, OpenCADStudio builds against local forks
of `iced` and `cryoglyph` that were ported to **wgpu 30**, instead of the
upstream git pins. This is milestone **M0** of the Bevy viewport integration
(`docs/bevy-viewport-integration.md`): Bevy main tracks wgpu 30, and sharing a
`wgpu::Device` between iced and Bevy requires both stacks on the same wgpu
major. Upstream iced master was still on wgpu 29 when this port was made, so
we maintain the port ourselves until upstream catches up.

## The forks

| Repo | Path | Branch | Base |
|------|------|--------|------|
| iced (iced-rs/iced) | `../iced` | `wgpu-30-port` | rev `23604ff` (the rev OCS previously pinned) |
| cryoglyph (iced-rs/cryoglyph, iced's text renderer) | `../cryoglyph` | `wgpu-30-port` | rev `53ba3e8` (the rev iced pins) |

OCS `Cargo.toml` consumes them as path dependencies (`iced`, `iced_core`, and
the `[patch.crates-io]` entries for `iced_core`/`iced_widget` all point at
`../iced`; the iced fork itself points `cryoglyph` at `../cryoglyph`).

Only `iced_wgpu` (plus one workspace version line) actually touches wgpu —
every other iced crate is feature plumbing. `iced_tiny_skia` uses softbuffer
and is unaffected.

## What the port changed (wgpu 29 → 30)

Breaking changes applied, per the official v30.0.0 changelog (2026-07-01):

1. **`VertexState::buffers` is now `&[Option<VertexBufferLayout>]`** — wrap
   layouts in `Some(...)`. iced_wgpu ×5 (triangle solid/gradient, quad
   solid/gradient, image), cryoglyph ×1 (stored field type changed too),
   OCS ×19 (`scene/pipeline/{mod,viewcube,text_gpu,hatch_gpu}`), examples ×1.
   Empty `&[]` sites need no change.
2. **`get_mapped_range(_mut)` returns `Result`** — `.expect(...)` at sites
   that map right after `mapped_at_creation`/`poll(Wait)`. iced_wgpu ×2,
   cryoglyph ×1, OCS ×4 (`wire_gpu`, `wire_arena`).
3. **`SurfaceTexture::present()` → `Queue::present(texture)`** — iced_wgpu
   compositor ×1 (queue reached via the renderer's `Engine`), integration
   example ×1. OCS never touches the surface.
4. **`SurfaceConfiguration` gained required `color_space`** —
   `SurfaceColorSpace::Auto` reproduces the old behavior. iced_wgpu ×1,
   examples ×2.
5. **`RequestAdapterOptions` gained required `apply_limit_buckets`** — set
   `false`. iced_wgpu ×2. (Found by compile; not in the "major changes"
   changelog section.)
6. **WGSL: integer inter-stage IO must be explicitly `@interpolate(flat)`** —
   zero changes needed: all integer varyings in iced/cryoglyph/OCS shaders
   were already flat-annotated or are vertex inputs (exempt). OCS's
   `naga` dev-dependency was bumped 27 → 30 so the shader-validation tests
   enforce the same rules as the runtime (`tests/hatch_shader_lod.rs`,
   `tests/mesh_shader_limits.rs` — both pass).

Not hit by any of the three codebases: `TryFrom<BufferSlice>` for
`BufferBinding`/`BindingResource`, `BufferSlice::size()` → `u64`,
`dispatch` → `dispatch_workgroups` rename (OCS already used the new name),
`TextureUsages::TRANSIENT_ATTACHMENT`, `CLIP_DISTANCES`, `map_label`
signatures, `enable wgpu_binding_array`.

### wasm: the web-sys generation moved too

7. **`web-sys` pin bumped `=0.3.85` → `=0.3.92` in both iced and OCS.**
   wgpu 30's *vendored* WebGPU bindings are generated against the
   wasm-bindgen **0.2.115** generation (its Cargo metadata still claims
   0.2.108 works — it does not: ~180 type errors in the webgpu backend).
   web-sys 0.3.92 pairs with wasm-bindgen 0.2.115. Consequence for web
   builds: **`wasm-bindgen-cli` must match 0.2.115** at `trunk build` time
   (recent trunk auto-manages this; a manually installed CLI needs
   `cargo install wasm-bindgen-cli --version 0.2.115`).
   - Do **not** "fix" the initial `web_sys::VideoFrame` error by adding
     `--cfg=web_sys_unstable_apis`: in 0.3.92 VideoFrame is stable, and the
     cfg *swaps canvas 2D signatures* (`get_image_data`/`put_image_data`
     f64 → i32), which breaks `softbuffer` 0.4.8.
8. Fixed in passing — two **pre-existing** wasm regressions unrelated to
   wgpu (from the P&ID work; nobody had run the wasm check since):
   - `PID_SEMANTICS_XDATA_APP` hoisted from the native-only `io::pid`
     module to `io::mod` so the web build can read P&ID XDATA out of DWGs
     (`pid_semantics_section` is target-independent).
   - `import_file_as_block` (block palette) is now cfg-split; the web
     variant returns an error string, as path-based loading cannot exist
     in the browser.

## Rebase procedure (when picking up new upstream iced)

1. `git -C ../iced fetch origin && git rebase origin/master wgpu-30-port`
   (same for `../cryoglyph` if its pin moved — check iced's `Cargo.toml`).
2. If upstream is still on wgpu 29, re-resolve conflicts in favor of the
   patterns above, then re-audit any **new** code upstream added:
   - `rg 'buffers: &\[' --type rust` — new pipelines need `Some(...)`
   - `rg 'get_mapped_range'` — new mappings need `Result` handling
   - `rg 'SurfaceConfiguration|RequestAdapterOptions'` — new required fields
   - `rg '\.present\('` — surface presents go through the queue
   - `rg '@location\([^)]*\).*(u32|i32)' -g '*.wgsl'` — integer varyings in
     new shaders need `@interpolate(flat)` unless they are vertex inputs
3. Build gates, in order:
   - `cargo check -p iced_wgpu` (in `../iced`)
   - `cargo check` (in OpenCADStudio, native)
   - `cargo test --test hatch_shader_lod --test mesh_shader_limits`
   - wasm: `cargo check --target wasm32-unknown-unknown`
4. **When upstream iced ships its own wgpu-30 support**: drop both forks,
   repoint `Cargo.toml` at the upstream git rev, and delete this fork's
   local-path patches. Diff our fork against upstream's port first — if they
   made different choices (e.g. error handling at map sites), OCS call sites
   may need a touch-up.

## M0 verification record (2026-08-07)

- `cargo check -p iced_wgpu`: pass
- OCS `cargo check` (native): pass, no new warnings
- Shader validation tests under naga 30: 4/4 pass
- Runtime smoke: OCS starts on wgpu 30, full start-page UI renders (quads,
  gradients, images, cryoglyph CJK text), no panics over 14 s — verified via
  screenshot
- `cargo check --target wasm32-unknown-unknown --no-default-features`
  (the Trunk feature set): pass, after the web-sys generation bump above
- Pending: per-drawing visual regression pass (human eyes); a real
  `trunk build` (needs wasm-bindgen-cli 0.2.115 on the build machine)

//! The bevy3d composite shader must parse and validate under naga even in
//! builds where the feature is off (the .wgsl ships unconditionally and is
//! only include_str!-ed behind the feature gate).

const BEVY_COMPOSITE_SHADER: &str = include_str!("../src/shaders/bevy_composite.wgsl");

#[test]
fn bevy_composite_wgsl_validates() {
    let module = naga::front::wgsl::parse_str(BEVY_COMPOSITE_SHADER)
        .expect("bevy_composite WGSL must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("bevy_composite WGSL must validate");
}

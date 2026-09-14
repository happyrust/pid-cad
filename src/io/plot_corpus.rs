// Synthetic plot pages the backend tests share.
//
// One corpus, two guards: `pdf_export::tests` runs it through the frozen
// pre-extraction exporter and the new `PdfSink` and compares the `Op` streams,
// and `svg_export::tests` runs it through `SvgSink` and compares the parsed
// SVG against the emitter's own operation stream. Both need the same pages —
// every rotation, every dash flavour, every CTB override, text, hatches,
// wipeouts, two render groups — so a case added for one backend hardens the
// other. Test-only.

use crate::io::plot_emit::{GlyphSnapshot, PlotPage};
use crate::io::plot_style::PlotStyleTable;
use crate::io::plot_types::{PdfPlotOptions, PlotGroupSplits, PlotWire};
use crate::scene::model::hatch_model::{plot_style_fill_pattern, HatchModel, HatchPattern};
use crate::scene::WireModel;

pub struct Case {
    pub name: &'static str,
    pub wires: Vec<PlotWire>,
    pub hatches: Vec<HatchModel>,
    pub wipeouts: Vec<HatchModel>,
    pub paper: (f32, f32),
    pub offset: (f64, f64),
    pub rotation_deg: i32,
    pub scale: f32,
    pub clip: Option<(f32, f32, f32, f32)>,
    pub plot_style: Option<PlotStyleTable>,
    pub options: PdfPlotOptions,
}

impl Case {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            wires: Vec::new(),
            hatches: Vec::new(),
            wipeouts: Vec::new(),
            paper: (297.0, 210.0),
            offset: (0.0, 0.0),
            rotation_deg: 0,
            scale: 1.0,
            clip: None,
            plot_style: None,
            options: PdfPlotOptions::default(),
        }
    }

    pub fn page(&self) -> PlotPage<'_> {
        PlotPage {
            wires: &self.wires,
            hatches: &self.hatches,
            wipeouts: &self.wipeouts,
            paper_w: self.paper.0,
            paper_h: self.paper.1,
            offset_x: self.offset.0,
            offset_y: self.offset.1,
            rotation_deg: self.rotation_deg,
            scale: self.scale,
            clip: self.clip,
            plot_style: self.plot_style.as_ref(),
            options: self.options,
        }
    }

    /// The same page as one entry of a multi-page job.
    pub fn page_input(&self) -> crate::io::plot_types::PdfPageInput {
        crate::io::plot_types::PdfPageInput {
            wires: std::sync::Arc::new(self.wires.clone()),
            hatches: self.hatches.clone(),
            wipeouts: self.wipeouts.clone(),
            paper_w: self.paper.0 as f64,
            paper_h: self.paper.1 as f64,
            offset_x: self.offset.0,
            offset_y: self.offset.1,
            rotation_deg: self.rotation_deg,
            scale: self.scale,
            clip: self.clip,
            options: self.options,
            plot_style: self.plot_style.clone(),
        }
    }

    pub fn clone_for(&self, name: &'static str) -> Self {
        Self {
            name,
            wires: self.wires.clone(),
            hatches: self.hatches.clone(),
            wipeouts: self.wipeouts.clone(),
            paper: self.paper,
            offset: self.offset,
            rotation_deg: self.rotation_deg,
            scale: self.scale,
            clip: self.clip,
            plot_style: self.plot_style.clone(),
            options: self.options,
        }
    }
}

pub fn wire(name: &str, points: Vec<[f32; 3]>, color: [f32; 4], depth: f32) -> PlotWire {
    PlotWire {
        wire: WireModel::solid(name.into(), points, color, false),
        draw_depth: depth,
    }
}

pub fn hatch(
    name: &str,
    boundary: Vec<[f32; 2]>,
    pattern: HatchPattern,
    color: [f32; 4],
) -> HatchModel {
    HatchModel {
        pattern_origin: None,
        render_instance: None,
        world_origin: [0.0, 0.0],
        boundary: std::sync::Arc::new(boundary),
        boundary_wcs: None,
        fill_plane: None,
        fill_plane_boundary: None,
        boundary_exterior: None,
        boundary_sources: None,
        boundary_paths: None,
        style: acadrust::entities::HatchStyleType::Normal,
        pattern,
        name: name.into(),
        color,
        aci: 0,
        line_weight_px: 1.5,
        angle_offset: 0.0,
        scale: 1.0,
        draw_depth: 0.5,
    }
}

pub fn square(x: f32, y: f32, size: f32) -> Vec<[f32; 2]> {
    vec![
        [x, y],
        [x + size, y],
        [x + size, y + size],
        [x, y + size],
        [x, y],
    ]
}

/// A wire carrying the SDF glyph quads for `text` in the embedded "txt" stroke
/// font, laid out into the process-wide atlas `emit_text` reads.
///
/// The quads' keys are only good until the atlas is re-scaled, and other tests
/// bake into the same atlas in parallel; a test that needs the text to
/// *survive* the plot takes [`text_wire_with_snapshot`] and hands the snapshot
/// to the emitter.
pub fn text_wire(text: &str, origin: [f64; 3]) -> PlotWire {
    text_wire_with_snapshot(text, origin).0
}

/// [`text_wire`] plus the glyph snapshot taken under the same lock as the
/// layout — so the snapshot is guaranteed to have every key the quads carry,
/// whatever another test bakes a moment later.
pub fn text_wire_with_snapshot(text: &str, origin: [f64; 3]) -> (PlotWire, GlyphSnapshot) {
    use crate::scene::pipeline::text_gpu::push_glyph_vertices;
    use crate::scene::text::{glyph_quads::layout_glyph_quads, sdf_atlas};
    let (quads, snapshot) = {
        let mut atlas = sdf_atlas::text_atlas().lock().unwrap();
        let quads = layout_glyph_quads(&mut atlas, 10.0, 0.0, 1.0, 0.0, 1.0, "txt", false, text);
        (quads, GlyphSnapshot::of(&atlas))
    };
    assert!(!quads.is_empty(), "stroke glyphs laid out for {text:?}");
    let mut verts = Vec::new();
    push_glyph_vertices(&mut verts, &quads, origin, 1.0, [1.0, 0.0, 0.0, 1.0], 0.0);
    let wire = PlotWire {
        wire: WireModel {
            text_verts: verts,
            ..WireModel::solid("t".into(), Vec::new(), WireModel::WHITE, false)
        },
        draw_depth: 0.0,
    };
    (wire, snapshot)
}

/// A CTB that exercises every override the emitter reads: colour, pen,
/// screening, cap / join, and a fill style that turns solids into patterns.
pub fn styled_ctb() -> PlotStyleTable {
    let mut ctb = PlotStyleTable::identity("test.ctb");
    // ACI 1: red → plotted dark grey, pen index 12, 60 % screen, butt / miter.
    let e = &mut ctb.aci_entries[1];
    e.color = Some([40, 40, 40]);
    e.lineweight = 12;
    e.screening = 60;
    e.end_style = 0;
    e.join_style = 0;
    // ACI 2: square caps, bevel joins, thick pen, no colour override.
    let e = &mut ctb.aci_entries[2];
    e.lineweight = 20;
    e.end_style = 1;
    e.join_style = 1;
    // ACI 3: grayscale policy (colour derived from the ACI palette).
    let e = &mut ctb.aci_entries[3];
    e.color_policy = 2 | 1;
    // ACI 4: fill style 65 → wire fills and solid hatches become a pattern.
    let e = &mut ctb.aci_entries[4];
    e.fill_style = 65;
    e.lineweight = 5;
    ctb
}

pub fn corpus() -> Vec<Case> {
    let mut cases = Vec::new();

    // Plain polyline with a pen-up, white (→ black on paper).
    let mut c = Case::new("polyline with pen-up");
    c.wires.push(wire(
        "a",
        vec![
            [10.0, 10.0, 0.0],
            [100.0, 10.0, 0.0],
            [f32::NAN, f32::NAN, 0.0],
            [10.0, 50.0, 0.0],
            [100.0, 80.0, 0.0],
        ],
        WireModel::WHITE,
        0.2,
    ));
    cases.push(c);

    // Every rotation, at a non-unit scale, with an offset and a clip.
    for rotation in [0, 90, 180, 270] {
        let mut c = Case::new("rotation / scale / clip");
        c.rotation_deg = rotation;
        c.scale = 0.5;
        c.offset = (-1234.5, 678.25);
        c.clip = Some((5.0, 7.5, 200.0, 120.0));
        c.paper = if rotation % 180 == 0 {
            (297.0, 210.0)
        } else {
            (210.0, 297.0)
        };
        c.wires.push(wire(
            "diag",
            vec![[1240.0, -670.0, 0.0], [1400.0, -600.0, 0.0]],
            [0.2, 0.9, 0.3, 1.0],
            0.1,
        ));
        cases.push(c);
    }

    // Linetypes: a plain dash pattern, a dash-dot pattern longer than six
    // entries, and a stationed pattern that is emitted as visible ranges.
    let mut c = Case::new("dash patterns");
    let mut dashed = wire(
        "dashed",
        vec![[0.0, 0.0, 0.0], [120.0, 0.0, 0.0]],
        WireModel::WHITE,
        0.3,
    );
    dashed.wire.pattern_length = 12.0;
    dashed.wire.pattern = [6.0, -3.0, 0.0, -3.0, 0.0, 0.0, 0.0, 0.0];
    c.wires.push(dashed);
    let mut long = wire(
        "long",
        vec![[0.0, 5.0, 0.0], [120.0, 5.0, 0.0]],
        WireModel::WHITE,
        0.3,
    );
    long.wire.pattern_length = 20.0;
    long.wire.pattern = [4.0, -2.0, 1.0, -2.0, 1.0, -2.0, 4.0, -4.0];
    c.wires.push(long);
    let mut stationed = wire(
        "stationed",
        vec![[0.0, 10.0, 0.0], [60.0, 10.0, 0.0], [60.0, 40.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.3,
    );
    stationed.wire.pattern_length = 10.0;
    stationed.wire.pattern = [5.0, -2.5, 0.0, -2.5, 0.0, 0.0, 0.0, 0.0];
    stationed.wire.pattern_stations = vec![0.0, 60.0, 90.0, 0.0];
    c.wires.push(stationed);
    // A reverse-direction stationed segment (end station < start station).
    let mut reverse = wire(
        "reverse",
        vec![[60.0, 60.0, 0.0], [0.0, 60.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.3,
    );
    reverse.wire.pattern_length = 10.0;
    reverse.wire.pattern = [5.0, -5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    reverse.wire.pattern_stations = vec![60.0, 0.0, 0.0];
    c.wires.push(reverse);
    // Back to solid after a dashed wire: the dash reset must be emitted.
    c.wires.push(wire(
        "solid-after",
        vec![[0.0, 70.0, 0.0], [120.0, 70.0, 0.0]],
        WireModel::WHITE,
        0.4,
    ));
    cases.push(c);

    // Pen widths: object weights, a wide polyline, scale_lineweights on
    // and off, object_lineweights off.
    for (scale_lw, object_lw) in [(false, true), (true, true), (false, false)] {
        let mut c = Case::new("pen widths");
        c.scale = 2.0;
        c.options.scale_lineweights = scale_lw;
        c.options.object_lineweights = object_lw;
        let mut thin = wire(
            "thin",
            vec![[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]],
            WireModel::WHITE,
            0.1,
        );
        thin.wire.line_weight_px = 0.5;
        c.wires.push(thin);
        let mut thick = wire(
            "thick",
            vec![[0.0, 5.0, 0.0], [50.0, 5.0, 0.0]],
            WireModel::WHITE,
            0.1,
        );
        thick.wire.line_weight_px = 4.0;
        c.wires.push(thick);
        let mut band = wire(
            "band",
            vec![[0.0, 10.0, 0.0], [50.0, 10.0, 0.0]],
            [0.1, 0.4, 0.9, 1.0],
            0.1,
        );
        band.wire.world_width = 3.0;
        band.wire.pattern_length = 8.0;
        band.wire.pattern = [4.0, -4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        c.wires.push(band);
        cases.push(c);
    }

    // Colours: near-white / near-yellow / near-cyan adaptation, a genuine
    // colour, a transparent wire that is skipped, and the paper helpers.
    let mut c = Case::new("colour adaptation");
    for (i, color) in [
        [0.95, 0.95, 0.95, 1.0],
        [0.9, 0.8, 0.1, 1.0],
        [0.1, 0.9, 0.9, 1.0],
        [0.6, 0.2, 0.7, 1.0],
        [0.6, 0.2, 0.7, 0.005],
        [0.6, 0.2, 0.7, 0.5],
    ]
    .into_iter()
    .enumerate()
    {
        c.wires.push(wire(
            "c",
            vec![[0.0, i as f32 * 5.0, 0.0], [30.0, i as f32 * 5.0, 0.0]],
            color,
            0.2,
        ));
    }
    c.wires.push(wire(
        "__paper_boundary__",
        vec![[0.0, 0.0, 0.0], [297.0, 210.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    c.wires.push(wire(
        "paper_printable_area",
        vec![[5.0, 5.0, 0.0], [290.0, 200.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    let mut t = c.clone_for("colour adaptation, transparency on");
    t.options.transparency = true;
    cases.push(c);
    cases.push(t);

    // Viewport hatch dots: coincident points become round dots.
    let mut c = Case::new("viewport hatch dots");
    c.scale = 0.75;
    c.wires.push(wire(
        "viewport_hatch_pattern",
        vec![
            [10.0, 10.0, 0.0],
            [10.0, 10.0, 0.0],
            [f32::NAN, f32::NAN, 0.0],
            [20.0, 10.0, 0.0],
            [25.0, 10.0, 0.0],
        ],
        WireModel::WHITE,
        0.1,
    ));
    cases.push(c);

    // Wire fills (a solid arrowhead) with and without the low residual.
    let mut c = Case::new("wire fills");
    let mut filled = wire(
        "fill",
        vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        [0.9, 0.9, 0.95, 1.0],
        0.6,
    );
    filled.wire.fill_tris = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [5.0, 8.0, 0.0]];
    filled.wire.fill_tris_low = vec![[1e-4, 0.0, 0.0], [0.0, 2e-4, 0.0], [0.0, 0.0, 0.0]];
    c.wires.push(filled);
    c.offset = (100.25, 50.125);
    cases.push(c);

    // Hatches: solid, pattern, gradient; a multi-ring boundary (island);
    // an ACI-7 solid that plots black on paper like any light fill (it was
    // kept white until 2026-09-10); a wipeout; degenerate rings.
    let mut c = Case::new("hatches");
    c.hatches.push(hatch(
        "SOLID",
        square(10.0, 10.0, 20.0),
        HatchPattern::Solid,
        [0.3, 0.6, 0.2, 1.0],
    ));
    let mut island = square(40.0, 10.0, 30.0);
    island.push([f32::NAN, f32::NAN]);
    island.extend(square(50.0, 20.0, 10.0));
    island.push([f32::NAN, f32::NAN]);
    island.extend([[0.0, 0.0], [1.0, 1.0]]); // fewer than 3 points: dropped
    c.hatches.push(hatch(
        "SOLID",
        island,
        HatchPattern::Solid,
        [0.95, 0.95, 0.95, 1.0],
    ));
    let mut white7 = hatch(
        "SOLID",
        square(80.0, 10.0, 10.0),
        HatchPattern::Solid,
        [1.0, 1.0, 1.0, 1.0],
    );
    white7.aci = 7;
    c.hatches.push(white7);
    c.hatches.push(hatch(
        "ANSI31",
        square(100.0, 10.0, 25.0),
        plot_style_fill_pattern(65).unwrap(),
        [0.9, 0.5, 0.1, 1.0],
    ));
    c.hatches.push(hatch(
        "AR-CONC-ish",
        square(130.0, 10.0, 25.0),
        plot_style_fill_pattern(71).unwrap(),
        [0.2, 0.2, 0.9, 1.0],
    ));
    c.hatches.push(hatch(
        "GRADIENT",
        square(160.0, 10.0, 20.0),
        HatchPattern::Gradient {
            angle_deg: 30.0,
            color2: [0.95, 0.95, 0.2, 1.0],
            kind: crate::scene::model::hatch_model::GradientKind::Linear,
            invert: false,
            shift: 0.0,
        },
        [0.2, 0.4, 0.8, 1.0],
    ));
    c.hatches.push(hatch(
        "SOLID",
        square(0.0, 0.0, 5.0),
        HatchPattern::Solid,
        [0.5, 0.5, 0.5, 0.0],
    ));
    c.wipeouts.push(hatch(
        "WIPEOUT_FILL",
        square(15.0, 15.0, 10.0),
        HatchPattern::Solid,
        [0.1, 0.1, 0.1, 1.0],
    ));
    let mut origin = hatch(
        "SOLID",
        square(1.0, 1.0, 4.0),
        HatchPattern::Solid,
        [0.7, 0.1, 0.1, 1.0],
    );
    origin.world_origin = [500_000.25, 4_500_000.5];
    c.hatches.push(origin);
    c.offset = (-500_000.0, -4_500_000.0);
    let mut m = c.clone_for("hatches, merge_lines");
    m.options.merge_lines = true;
    cases.push(c);
    cases.push(m);

    // CTB: colour / pen / screening / cap / join overrides, grayscale
    // policy, and the fill-style pattern conversion for fills and hatches.
    let mut c = Case::new("ctb");
    c.plot_style = Some(styled_ctb());
    for aci in 1..=4u8 {
        let mut w = wire(
            "aci",
            vec![[0.0, aci as f32 * 6.0, 0.0], [80.0, aci as f32 * 6.0, 0.0]],
            [0.9, 0.2, 0.2, 1.0],
            0.2,
        );
        w.wire.aci = aci;
        c.wires.push(w);
    }
    let mut styled_fill = wire(
        "fill4",
        vec![[0.0, 40.0, 0.0], [10.0, 40.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.6,
    );
    styled_fill.wire.aci = 4;
    styled_fill.wire.fill_tris = vec![[0.0, 40.0, 0.0], [20.0, 40.0, 0.0], [10.0, 55.0, 0.0]];
    c.wires.push(styled_fill);
    let mut plain_fill = wire(
        "fill1",
        vec![[30.0, 40.0, 0.0], [40.0, 40.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.6,
    );
    plain_fill.wire.aci = 1;
    plain_fill.wire.fill_tris = vec![[30.0, 40.0, 0.0], [50.0, 40.0, 0.0], [40.0, 55.0, 0.0]];
    c.wires.push(plain_fill);
    let mut solid4 = hatch(
        "SOLID",
        square(60.0, 40.0, 15.0),
        HatchPattern::Solid,
        [0.9, 0.2, 0.2, 1.0],
    );
    solid4.aci = 4;
    c.hatches.push(solid4);
    let mut pat1 = hatch(
        "ANSI31",
        square(80.0, 40.0, 15.0),
        plot_style_fill_pattern(66).unwrap(),
        [0.9, 0.2, 0.2, 1.0],
    );
    pat1.aci = 1;
    c.hatches.push(pat1);
    let mut text = text_wire("CTB", [0.0, 70.0, 0.0]);
    text.wire.aci = 1;
    c.wires.push(text);
    let mut s = c.clone_for("ctb, scale_lineweights");
    s.scale = 0.5;
    s.options.scale_lineweights = true;
    cases.push(c);
    cases.push(s);

    // Text from the process atlas, plus a quad whose key the atlas does not
    // have (skipped) and a decoration bar.
    let mut c = Case::new("text");
    c.wires.push(text_wire("HELLO WORLD", [20.0, 20.0, 0.0]));
    let mut dashed_before = wire(
        "dashed",
        vec![[0.0, 0.0, 0.0], [120.0, 0.0, 0.0]],
        WireModel::WHITE,
        0.0,
    );
    dashed_before.wire.pattern_length = 12.0;
    dashed_before.wire.pattern = [6.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    c.wires.push(dashed_before);
    let mut unknown = text_wire("X", [0.0, 0.0, 0.0]);
    for v in &mut unknown.wire.text_verts {
        v.uv = [0.987, 0.654];
    }
    c.wires.push(unknown);
    let mut bar = text_wire("I", [40.0, 40.0, 0.0]);
    let solid_uv = crate::scene::text::sdf_atlas::text_atlas()
        .lock()
        .unwrap()
        .solid_uv();
    for v in &mut bar.wire.text_verts {
        v.uv = solid_uv;
    }
    c.wires.push(bar);
    c.offset = (12.5, -7.25);
    cases.push(c);

    // Two render groups: every kind of item in both, with depths that
    // interleave across the split so the sort has to stay per group.
    let mut c = Case::new("two groups");
    c.wires.push(wire(
        "g1-low",
        vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        WireModel::WHITE,
        0.9,
    ));
    let mut g1_fill = wire(
        "g1-fill",
        vec![[0.0, 1.0, 0.0], [10.0, 1.0, 0.0]],
        [0.5, 0.5, 0.9, 1.0],
        0.1,
    );
    g1_fill.wire.fill_tris = vec![[0.0, 1.0, 0.0], [10.0, 1.0, 0.0], [5.0, 6.0, 0.0]];
    c.wires.push(g1_fill);
    c.wires.push(text_wire("G1", [0.0, 10.0, 0.0]));
    c.wires.push(wire(
        "g2-high",
        vec![[0.0, 20.0, 0.0], [10.0, 20.0, 0.0]],
        WireModel::WHITE,
        0.05,
    ));
    c.wires.push(text_wire("G2", [0.0, 30.0, 0.0]));
    c.hatches.push(hatch(
        "SOLID",
        square(20.0, 0.0, 5.0),
        HatchPattern::Solid,
        [0.2, 0.7, 0.2, 1.0],
    ));
    c.hatches.push(hatch(
        "SOLID",
        square(20.0, 10.0, 5.0),
        HatchPattern::Solid,
        [0.7, 0.2, 0.2, 1.0],
    ));
    c.wipeouts.push(hatch(
        "WIPEOUT_FILL",
        square(2.0, 2.0, 3.0),
        HatchPattern::Solid,
        [0.0, 0.0, 0.0, 1.0],
    ));
    c.wipeouts.push(hatch(
        "WIPEOUT_FILL",
        square(2.0, 22.0, 3.0),
        HatchPattern::Solid,
        [0.0, 0.0, 0.0, 1.0],
    ));
    c.options.group_splits = PlotGroupSplits {
        wires: 3,
        hatches: 1,
        wipeouts: 1,
    };
    // Splits past the end must clamp, not panic.
    let mut clamped = c.clone_for("two groups, splits past the end");
    clamped.options.group_splits = PlotGroupSplits {
        wires: 99,
        hatches: 99,
        wipeouts: 99,
    };
    let mut m = c.clone_for("two groups, merge_lines + stamp");
    m.options.merge_lines = true;
    m.options.stamp = true;
    m.rotation_deg = 90;
    m.paper = (210.0, 297.0);
    m.clip = Some((0.0, 0.0, 100.0, 100.0));
    cases.push(c);
    cases.push(clamped);
    cases.push(m);

    // R4: the first group ends on a CTB butt / miter wire, the second opens
    // with a plain wire that is round by default. Nothing between the groups
    // restores the graphics state, so the emitter has to say Round again —
    // and until 2026-09-08 it believed it already had. A second CTB wire
    // follows so the stream toggles back, which is the normal case.
    let mut c = Case::new("two groups, ctb cap across the split");
    c.plot_style = Some(styled_ctb());
    let mut butt = wire(
        "g1-butt",
        vec![[0.0, 0.0, 0.0], [40.0, 0.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.5,
    );
    butt.wire.aci = 1;
    c.wires.push(butt);
    c.wires.push(wire(
        "g2-round",
        vec![[0.0, 10.0, 0.0], [40.0, 10.0, 0.0]],
        WireModel::WHITE,
        0.5,
    ));
    let mut butt_again = wire(
        "g2-butt",
        vec![[0.0, 20.0, 0.0], [40.0, 20.0, 0.0]],
        [0.9, 0.2, 0.2, 1.0],
        0.6,
    );
    butt_again.wire.aci = 1;
    c.wires.push(butt_again);
    c.options.group_splits = PlotGroupSplits {
        wires: 1,
        ..Default::default()
    };
    cases.push(c);

    // An empty page still gets its background and state ops.
    cases.push(Case::new("empty page"));

    cases
}

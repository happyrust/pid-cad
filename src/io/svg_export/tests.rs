// Acceptance for the SVG backend (§8 of docs/plans/2026-09-07-dxf-to-svg-export.md).
//
// The second layer is the one that matters: every page of the shared corpus is
// written to SVG, the *written file* is parsed back by usvg — a third-party
// parser, not this module — and every shape it resolves is compared, in sheet
// millimetres, against what the emitter said to draw. Comparing the two sinks'
// operation streams would only prove they were handed the same input; this
// compares the artefact.
//
// The third layer rasterises with resvg and checks a handful of things a
// structural comparison cannot see: that a 100 mm line is 100 mm long and the
// right way up, that a wipeout masks, that a mesh fill has no anti-aliasing
// seam. Both layers carry fault injection, because a check that never fails is
// not a check: the mutations below (delete a path, move a point, drop the
// even-odd rule, flip the page matrix, remove a glyph) must all be caught.
//
// There is no PDF-versus-SVG raster comparison here. That needs a pinned PDF
// rasteriser, which this tree does not have; §8's third layer stays open for
// P3 and the plan says so.

use super::*;
use crate::io::plot_corpus::{corpus, hatch, square, wire, Case};
use crate::io::plot_emit::{PlotAssets, RecordingSink};
use crate::scene::model::hatch_model::HatchPattern;
use crate::scene::WireModel;
use resvg::{tiny_skia, usvg};

// ── Affine helpers ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Mat {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Mat {
    const ID: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// `self` applied after `inner`.
    fn concat(self, inner: Mat) -> Mat {
        Mat {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    fn apply(self, x: f64, y: f64) -> [f64; 2] {
        [
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        ]
    }

    fn scale(self) -> f64 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    fn of(t: tiny_skia::Transform) -> Mat {
        Mat {
            a: t.sx as f64,
            b: t.ky as f64,
            c: t.kx as f64,
            d: t.sy as f64,
            e: t.tx as f64,
            f: t.ty as f64,
        }
    }
}

// ── One drawn thing, in sheet millimetres ─────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Paint {
    Fill { color: [u8; 3], rule: FillRule },
    Stroke { color: [u8; 3] },
}

#[derive(Clone, Debug)]
struct Shape {
    paint: Paint,
    rings: Vec<Vec<[f64; 2]>>,
    /// Stroke width in mm of paper.
    width_mm: f64,
    /// Dash run lengths in mm of paper.
    dash_mm: Vec<f64>,
    cap: LineCap,
    join: LineJoin,
    multiply: bool,
    /// Clip paths in force, excluding the page clip.
    clips: usize,
    /// A mesh fill is compared by area and extent, not ring for ring: the
    /// writer is free to collapse the triangles into their outline, which is
    /// the whole point of `FillMesh`, so the rings legitimately differ.
    mesh: bool,
}

impl Shape {
    fn area(&self) -> f64 {
        self.rings
            .iter()
            .map(|ring| {
                let mut sum = 0.0;
                for i in 0..ring.len() {
                    let p = ring[i];
                    let q = ring[(i + 1) % ring.len()];
                    sum += p[0] * q[1] - q[0] * p[1];
                }
                sum * 0.5
            })
            .sum()
    }

    fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in self.rings.iter().flatten() {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        b
    }
}

// ── Expected: the emitter's own operation stream, resolved to paper ───────

#[derive(Clone)]
struct ExpectedState {
    ctm: Mat,
    stroke: [f32; 3],
    fill: [f32; 3],
    width_pt: f32,
    cap: LineCap,
    join: LineJoin,
    dash: Vec<i64>,
    blend: PlotBlend,
    clips: usize,
}

/// The emitter's own account of the page, drawn from `assets` — the same
/// assets the page under test was written from, or the two can disagree about
/// which glyphs exist (see `shared_assets`).
fn record_with(page: &PlotPage<'_>, assets: &PlotAssets) -> Vec<PlotOp> {
    let mut sink = RecordingSink::default();
    match crate::io::plot_emit::emit_plot_content(page, assets, &mut sink) {
        Ok(_) => {}
        Err(never) => match never {},
    }
    sink.ops
}

/// Where the operation stream says the ink goes, in sheet mm with Y down —
/// the same space the SVG root user units live in.
fn expected_shapes(ops: &[PlotOp], paper_h: f32) -> Vec<Shape> {
    let flip = |p: [f64; 2]| [p[0] * PT_TO_MM, paper_h as f64 - p[1] * PT_TO_MM];
    let mut state = ExpectedState {
        ctm: Mat::ID,
        stroke: [0.0; 3],
        fill: [0.0; 3],
        width_pt: 1.0,
        cap: LineCap::Round,
        join: LineJoin::Round,
        dash: Vec::new(),
        blend: PlotBlend::Normal,
        clips: 0,
    };
    let mut stack: Vec<ExpectedState> = Vec::new();
    let mut shapes = Vec::new();
    for op in ops {
        let paper = |points: &[PlotPoint], state: &ExpectedState| -> Vec<[f64; 2]> {
            points
                .iter()
                .map(|p| flip(state.ctm.apply(p.x as f64, p.y as f64)))
                .collect()
        };
        match op {
            PlotOp::Save => stack.push(state.clone()),
            PlotOp::Restore => state = stack.pop().expect("balanced graphics state"),
            PlotOp::Concat([a, b, c, d, e, f]) => {
                state.ctm = state.ctm.concat(Mat {
                    a: *a as f64,
                    b: *b as f64,
                    c: *c as f64,
                    d: *d as f64,
                    e: *e as f64,
                    f: *f as f64,
                })
            }
            PlotOp::Blend(blend) => state.blend = *blend,
            PlotOp::LineCap(cap) => state.cap = *cap,
            PlotOp::LineJoin(join) => state.join = *join,
            PlotOp::StrokeColor(c) => state.stroke = *c,
            PlotOp::FillColor(c) => state.fill = *c,
            PlotOp::StrokeWidthPt(w) => state.width_pt = *w,
            PlotOp::Dash { lengths, .. } => state.dash = lengths.clone(),
            PlotOp::Clip { .. } => state.clips += 1,
            PlotOp::FillRect {
                x,
                y,
                width,
                height,
            } => {
                let corners = [
                    PlotPoint { x: *x, y: *y },
                    PlotPoint {
                        x: x + width,
                        y: *y,
                    },
                    PlotPoint {
                        x: x + width,
                        y: y + height,
                    },
                    PlotPoint {
                        x: *x,
                        y: y + height,
                    },
                ];
                shapes.push(fill_shape(&state, vec![paper(&corners, &state)], false));
            }
            PlotOp::Stroke { points, .. } => {
                let scale = state.ctm.scale();
                shapes.push(Shape {
                    paint: Paint::Stroke {
                        color: channels(state.stroke),
                    },
                    rings: vec![paper(points, &state)],
                    width_mm: state.width_pt as f64 * scale * PT_TO_MM,
                    dash_mm: state
                        .dash
                        .iter()
                        .map(|l| *l as f64 * scale * PT_TO_MM)
                        .collect(),
                    cap: state.cap,
                    join: state.join,
                    multiply: state.blend == PlotBlend::Multiply,
                    clips: state.clips,
                    mesh: false,
                });
            }
            PlotOp::Fill { rings, rule } => {
                let rings = rings.iter().map(|r| paper(r, &state)).collect();
                let mut shape = fill_shape(&state, rings, false);
                shape.paint = Paint::Fill {
                    color: channels(state.fill),
                    rule: *rule,
                };
                shapes.push(shape);
            }
            PlotOp::FillMesh { tris } => {
                let rings = tris.iter().map(|t| paper(t, &state)).collect();
                shapes.push(fill_shape(&state, rings, true));
            }
            PlotOp::BuiltinText { .. } => unreachable!("the corpus stamp cases are excluded"),
        }
    }
    shapes
}

fn fill_shape(state: &ExpectedState, rings: Vec<Vec<[f64; 2]>>, mesh: bool) -> Shape {
    Shape {
        paint: Paint::Fill {
            color: channels(state.fill),
            rule: FillRule::NonZero,
        },
        rings,
        width_mm: 0.0,
        dash_mm: Vec::new(),
        cap: state.cap,
        join: state.join,
        multiply: state.blend == PlotBlend::Multiply,
        clips: state.clips,
        mesh,
    }
}

// ── Actual: the written file, parsed by usvg ──────────────────────────────

fn parse(svg: &str) -> usvg::Tree {
    usvg::Tree::from_str(svg, &usvg::Options::default()).expect("the document parses")
}

fn actual_shapes(tree: &usvg::Tree, paper_w: f32) -> Vec<Shape> {
    // usvg resolves the document to CSS pixels: `width="297mm"` becomes
    // 297·96/25.4 px and the viewBox scale rides in every node's absolute
    // transform. Scale back, so the shapes come out in the document's own user
    // units — which, this being the point of D2, are millimetres of paper.
    let factor = paper_w as f64 / tree.size().width() as f64;
    let to_mm = Mat {
        a: factor,
        d: factor,
        ..Mat::ID
    };
    let mut out = Vec::new();
    collect(tree.root(), to_mm, false, 0, &mut out);
    out
}

fn collect(group: &usvg::Group, to_mm: Mat, multiply: bool, clips: usize, out: &mut Vec<Shape>) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => {
                // The page clip is the writer's own frame, not something the
                // emitter asked for; every other clip is.
                let page_clip = g.clip_path().is_some_and(|clip| clip.id() == "page-clip");
                let clips = clips + usize::from(g.clip_path().is_some() && !page_clip);
                let multiply = multiply || g.blend_mode() == usvg::BlendMode::Multiply;
                collect(g, to_mm, multiply, clips, out);
            }
            usvg::Node::Path(path) => {
                let abs = to_mm.concat(Mat::of(path.abs_transform()));
                let scale = abs.scale();
                let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
                for segment in path.data().segments() {
                    match segment {
                        tiny_skia::PathSegment::MoveTo(p) => {
                            rings.push(vec![abs.apply(p.x as f64, p.y as f64)])
                        }
                        tiny_skia::PathSegment::LineTo(p) => rings
                            .last_mut()
                            .expect("a sub-path starts with a move")
                            .push(abs.apply(p.x as f64, p.y as f64)),
                        tiny_skia::PathSegment::Close => {}
                        other => panic!("unexpected curve in the output: {other:?}"),
                    }
                }
                let paint = match (path.fill(), path.stroke()) {
                    (Some(fill), None) => Paint::Fill {
                        color: paint_color(fill.paint()),
                        rule: match fill.rule() {
                            usvg::FillRule::NonZero => FillRule::NonZero,
                            usvg::FillRule::EvenOdd => FillRule::EvenOdd,
                        },
                    },
                    (None, Some(stroke)) => Paint::Stroke {
                        color: paint_color(stroke.paint()),
                    },
                    (fill, stroke) => panic!(
                        "a path must be filled or stroked, never both or neither \
                         (fill {}, stroke {})",
                        fill.is_some(),
                        stroke.is_some()
                    ),
                };
                let stroke = path.stroke();
                out.push(Shape {
                    paint,
                    rings,
                    width_mm: stroke.map_or(0.0, |s| s.width().get() as f64 * scale),
                    dash_mm: stroke
                        .and_then(|s| s.dasharray())
                        .map(|d| d.iter().map(|v| *v as f64 * scale).collect())
                        .unwrap_or_default(),
                    cap: stroke.map_or(LineCap::Round, |s| match s.linecap() {
                        usvg::LineCap::Butt => LineCap::Butt,
                        usvg::LineCap::Round => LineCap::Round,
                        usvg::LineCap::Square => LineCap::Square,
                    }),
                    join: stroke.map_or(LineJoin::Round, |s| match s.linejoin() {
                        usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                        usvg::LineJoin::Round => LineJoin::Round,
                        usvg::LineJoin::Bevel => LineJoin::Bevel,
                    }),
                    multiply,
                    clips,
                    mesh: false,
                });
            }
            other => panic!("unexpected node in the output: {other:?}"),
        }
    }
}

fn paint_color(paint: &usvg::Paint) -> [u8; 3] {
    match paint {
        usvg::Paint::Color(c) => [c.red, c.green, c.blue],
        other => panic!("unexpected paint server in the output: {other:?}"),
    }
}

// ── The comparison ────────────────────────────────────────────────────────

/// Sheet tolerance. D2 budgets 0.001 mm per axis for serialisation; the rest
/// is the f32 arithmetic usvg does on the way back.
const TOL_MM: f64 = 0.002;

/// The tolerance for a coordinate of that magnitude.
///
/// On the sheet it is the budget above. Off the sheet it cannot be: SVG
/// consumers parse into f32 (usvg does), so a point half a kilometre from the
/// page — which the corpus has, to exercise the emitter's f64 cancellation of
/// a UTM world origin — is quantised by the format itself, not by the writer.
/// Ink that lands on the paper is what the budget is about.
fn tol_at(mm: f64) -> f64 {
    TOL_MM.max(4.0 * mm.abs() * f32::EPSILON as f64)
}

fn compare(expected: &[Shape], actual: &[Shape]) -> Result<(), String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "{} shapes drawn, {} in the file",
            expected.len(),
            actual.len()
        ));
    }
    for (i, (want, got)) in expected.iter().zip(actual).enumerate() {
        let fault = |what: String| Err(format!("shape #{i}: {what}"));
        match (&want.paint, &got.paint) {
            (
                Paint::Fill {
                    color: wc,
                    rule: wr,
                },
                Paint::Fill {
                    color: gc,
                    rule: gr,
                },
            ) => {
                if wr != gr {
                    return fault(format!("fill rule {wr:?} became {gr:?}"));
                }
                if !near_color(*wc, *gc) {
                    return fault(format!("fill {wc:?} became {gc:?}"));
                }
            }
            (Paint::Stroke { color: wc }, Paint::Stroke { color: gc }) => {
                if !near_color(*wc, *gc) {
                    return fault(format!("stroke {wc:?} became {gc:?}"));
                }
                if (want.width_mm - got.width_mm).abs() > TOL_MM {
                    return fault(format!(
                        "pen {:.6} mm became {:.6} mm",
                        want.width_mm, got.width_mm
                    ));
                }
                if !same_dash(&want.dash_mm, &got.dash_mm) {
                    return fault(format!("dash {:?} became {:?}", want.dash_mm, got.dash_mm));
                }
                if want.cap != got.cap || want.join != got.join {
                    return fault(format!(
                        "cap/join {:?}/{:?} became {:?}/{:?}",
                        want.cap, want.join, got.cap, got.join
                    ));
                }
            }
            (w, g) => return fault(format!("{w:?} became {g:?}")),
        }
        if want.multiply != got.multiply {
            return fault(format!(
                "multiply {} became {}",
                want.multiply, got.multiply
            ));
        }
        if want.clips != got.clips {
            return fault(format!("{} clips became {}", want.clips, got.clips));
        }
        if want.mesh {
            // A mesh may be written as its outline, so compare what the
            // outline must preserve: the ink's extent and its signed area
            // (holes included, since they subtract).
            if (want.area() - got.area()).abs() > TOL_MM * 10.0 {
                return fault(format!(
                    "mesh area {:.6} mm² became {:.6} mm²",
                    want.area(),
                    got.area()
                ));
            }
            let (wb, gb) = (want.bbox(), got.bbox());
            if wb.iter().zip(gb).any(|(w, g)| (w - g).abs() > tol_at(*w)) {
                return fault(format!("mesh extent {wb:?} became {gb:?}"));
            }
            continue;
        }
        // A closed ring that ends where it began carries no extra information,
        // and whether the repeat survives the round trip is up to the writer.
        let closed = matches!(want.paint, Paint::Fill { .. });
        let (want_rings, got_rings) = (closed_rings(want, closed), closed_rings(got, closed));
        if want_rings.len() != got_rings.len()
            || want_rings.iter().map(Vec::len).sum::<usize>()
                != got_rings.iter().map(Vec::len).sum::<usize>()
        {
            return fault(format!(
                "{} rings / {} points became {} rings / {} points",
                want_rings.len(),
                want_rings.iter().map(Vec::len).sum::<usize>(),
                got_rings.len(),
                got_rings.iter().map(Vec::len).sum::<usize>()
            ));
        }
        for (wr, gr) in want_rings.iter().zip(&got_rings) {
            for (wp, gp) in wr.iter().zip(gr) {
                if (wp[0] - gp[0]).abs() > tol_at(wp[0]) || (wp[1] - gp[1]).abs() > tol_at(wp[1]) {
                    return fault(format!("point {wp:?} became {gp:?}"));
                }
            }
        }
    }
    Ok(())
}

/// A shape's rings with the redundant closing point dropped, for shapes whose
/// rings are closed.
fn closed_rings(shape: &Shape, closed: bool) -> Vec<Vec<[f64; 2]>> {
    shape
        .rings
        .iter()
        .map(|ring| {
            let mut ring = ring.clone();
            if closed && ring.len() > 1 {
                let (first, last) = (ring[0], ring[ring.len() - 1]);
                if (first[0] - last[0]).abs() < 1e-9 && (first[1] - last[1]).abs() < 1e-9 {
                    ring.pop();
                }
            }
            ring
        })
        .collect()
}

fn near_color(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (*x as i16 - y as i16).abs() <= 1)
}

fn same_dash(want: &[f64], got: &[f64]) -> bool {
    // SVG repeats an odd-length dash list to make it even; usvg may have done
    // that already.
    let doubled: Vec<f64> = want.iter().chain(want).copied().collect();
    let matches = |reference: &[f64]| {
        reference.len() == got.len()
            && reference
                .iter()
                .zip(got)
                .all(|(w, g)| (w - g).abs() <= TOL_MM)
    };
    matches(want) || (want.len() % 2 == 1 && matches(&doubled))
}

// ── Layer 2: every corpus page ────────────────────────────────────────────

/// The corpus keeps a text quad whose key the atlas does not have, on purpose:
/// it is how the traversal's skip gets exercised. And the corpus lays its text
/// out into the process-wide atlas without a snapshot of its own, so a test
/// elsewhere in the binary that resets the atlas (every `OpenCADStudio::new`
/// applies TEXTFILL, which does) or grows it makes the corpus text stale in
/// flight. Writing corpus pages is therefore the lenient caller's job — the
/// strict one refuses them, which is its own test, on pages that carry their
/// own snapshot.
fn lenient() -> SvgOptions {
    SvgOptions {
        missing_glyphs: MissingGlyphs::Report,
        ..Default::default()
    }
}

fn lenient_files() -> SvgWriteOptions {
    SvgWriteOptions {
        svg: lenient(),
        ..Default::default()
    }
}

fn write(case: &Case) -> (String, SvgReport) {
    write_with(case, &PlotAssets::default())
}

fn write_with(case: &Case, assets: &PlotAssets) -> (String, SvgReport) {
    svg_page_to_string(&case.page(), assets, &lenient())
        .unwrap_or_else(|e| panic!("{}: {e}", case.name))
}

/// One glyph snapshot for both halves of a comparison. Written from one
/// snapshot and recorded from another, a page can legitimately differ by
/// exactly its glyphs — a test elsewhere resets or grows the shared atlas in
/// between — and the comparison would blame the writer. Drawn from the same
/// snapshot, the two sides agree on which glyphs exist and the comparison is
/// about the serialisation, which is what it is for.
fn shared_assets() -> PlotAssets {
    PlotAssets {
        glyphs: crate::io::plot_emit::GlyphSnapshot::capture(),
        ..Default::default()
    }
}

#[test]
fn the_written_svg_draws_what_the_emitter_asked_for() {
    let cases = corpus();
    assert!(cases.len() >= 20, "corpus shrank to {}", cases.len());
    let mut checked = 0;
    for case in &cases {
        if case.options.stamp {
            continue; // refused, and covered by its own test
        }
        let assets = shared_assets();
        let (svg, _) = write_with(case, &assets);
        let expected = expected_shapes(&record_with(&case.page(), &assets), case.paper.1);
        let actual = actual_shapes(&parse(&svg), case.paper.0);
        if let Err(why) = compare(&expected, &actual) {
            panic!("{}: {why}", case.name);
        }
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} pages compared");
}

// A check that cannot fail proves nothing: break the file in five ways the
// comparison is supposed to notice, and require it to notice.
#[test]
fn the_comparison_catches_a_broken_file() {
    let case = corpus()
        .into_iter()
        .find(|c| c.name == "hatches")
        .expect("the hatch page is in the corpus");
    let assets = shared_assets();
    let (svg, _) = write_with(&case, &assets);
    let expected = expected_shapes(&record_with(&case.page(), &assets), case.paper.1);
    compare(&expected, &actual_shapes(&parse(&svg), case.paper.0))
        .expect("the unmutated file passes");

    let first_path = svg.find("<path").expect("the page has paths");
    let end = svg[first_path..].find("/>").unwrap() + first_path + 2;
    let mutations = [
        (
            "a deleted path",
            format!("{}{}", &svg[..first_path], &svg[end..]),
        ),
        ("a moved point", move_first_point(&svg)),
        (
            "a lost even-odd rule",
            svg.replacen(" fill-rule=\"evenodd\"", "", 1),
        ),
        (
            "an unflipped page",
            svg.replacen("0,0,-0.352777", "0,0,0.352777", 1),
        ),
        (
            "a fatter pen",
            svg.replace("stroke-width=\"", "stroke-width=\"9"),
        ),
    ];
    for (what, mutated) in mutations {
        assert_ne!(mutated, svg, "the {what} mutation did not apply");
        let actual = actual_shapes(&parse(&mutated), case.paper.0);
        assert!(
            compare(&expected, &actual).is_err(),
            "{what} went unnoticed"
        );
    }
}

/// Push the first coordinate of the first sub-path 5 points sideways.
fn move_first_point(svg: &str) -> String {
    let at = svg.find("d=\"M").expect("a path") + 4;
    let end = at + svg[at..].find(',').expect("an x,y pair");
    let x: f64 = svg[at..end].parse().expect("a number");
    format!("{}{}{}", &svg[..at], x + 5.0, &svg[end..])
}

// ── R4: cap and join across the render groups ─────────────────────────────

#[test]
fn a_ctb_cap_left_by_the_first_group_does_not_leak_into_the_second() {
    // Layer 2 cannot see this: it compares the file with the emitter's own
    // stream, and when the emitter forgets to say Round both sides agree on
    // the wrong cap. So this asks the written file directly. Three strokes,
    // one per 10 mm: the CTB wire ending group one is butt / miter; the plain
    // wire opening group two is round by default and must come out round on
    // paper; the CTB wire after it is butt / miter again.
    let case = corpus()
        .into_iter()
        .find(|c| c.name == "two groups, ctb cap across the split")
        .unwrap();
    let (svg, _) = write(&case);
    let mut strokes: Vec<Shape> = actual_shapes(&parse(&svg), case.paper.0)
        .into_iter()
        .filter(|s| matches!(s.paint, Paint::Stroke { .. }))
        .collect();
    assert_eq!(strokes.len(), 3, "three wires, three strokes");
    // Document order is draw order; sort by the wire's height on the sheet
    // (Y down, so the lowest wire has the largest y) to name them.
    strokes.sort_by(|a, b| b.rings[0][0][1].total_cmp(&a.rings[0][0][1]));
    let caps: Vec<(LineCap, LineJoin)> = strokes.iter().map(|s| (s.cap, s.join)).collect();
    assert_eq!(
        caps,
        [
            (LineCap::Butt, LineJoin::Miter),
            (LineCap::Round, LineJoin::Round),
            (LineCap::Butt, LineJoin::Miter),
        ],
        "g1-butt, g2-round, g2-butt"
    );
}

// ── The serialisation contract (§8) ───────────────────────────────────────

#[test]
fn the_document_is_self_contained_and_says_its_physical_size() {
    let mut case = Case::new("A3 landscape");
    case.paper = (420.0, 297.0);
    case.wires.push(wire(
        "a",
        vec![[10.0, 10.0, 0.0], [400.0, 200.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    let (svg, _) = write(&case);

    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg "));
    assert!(svg.contains("xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.contains("width=\"420mm\" height=\"297mm\""));
    assert!(svg.contains("viewBox=\"0 0 420 297\""));
    // No DTD, no scripting, nothing to fetch, no font to find.
    for forbidden in [
        "<!DOCTYPE",
        "<script",
        "<text",
        "xlink:href",
        "href=",
        "<image",
        "font-family",
    ] {
        assert!(!svg.contains(forbidden), "{forbidden} in the output");
    }
    // Every path says which half of the paint it is not using — the other
    // half it inherits from the group its run shares.
    for path in svg.match_indices("<path").map(|(i, _)| &svg[i..]) {
        let tag = &path[..path.find("/>").unwrap()];
        assert_ne!(
            tag.contains("fill=\"none\""),
            tag.contains("stroke=\"none\""),
            "a path must be a stroke or a fill, not both or neither: {tag}"
        );
    }
    // And the paint itself is on the groups, not repeated on every leaf.
    assert!(
        svg.contains("<g stroke=\"#"),
        "no shared paint group in the output"
    );
    let ids: Vec<&str> = svg.match_indices("id=\"").map(|(i, _)| &svg[i..]).collect();
    let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "duplicate ids");

    let tree = parse(&svg);
    let mm_per_px = 25.4 / 96.0;
    assert!((tree.size().width() as f64 * mm_per_px - 420.0).abs() < 0.01);
    assert!((tree.size().height() as f64 * mm_per_px - 297.0).abs() < 0.01);
}

#[test]
fn the_stamp_is_refused_rather_than_approximated() {
    let mut case = Case::new("stamp");
    case.options.stamp = true;
    let error = svg_page_to_string(
        &case.page(),
        &PlotAssets {
            stamp_label: Some("PINNED".into()),
            ..Default::default()
        },
        &SvgOptions::default(),
    )
    .expect_err("a stamped page cannot be written");
    assert!(matches!(error, SvgError::Unsupported(_)), "{error}");
    assert!(error.to_string().contains("stamp"), "{error}");
}

#[test]
fn an_impossible_page_is_refused() {
    let mut broken: Vec<(&str, Case)> = Vec::new();

    let mut case = Case::new("bad");
    case.paper = (297.0, 0.0);
    broken.push(("a page with no height", case));

    let mut case = Case::new("bad");
    case.scale = -1.0;
    broken.push(("a negative plot scale", case));

    let mut case = Case::new("bad");
    case.clip = Some((0.0, f32::NAN, 10.0, 10.0));
    broken.push(("a NaN clip rectangle", case));

    // NaN is the emitter's pen-up sentinel and never reaches a coordinate, but
    // an infinity would, and there is no number to write for it.
    let mut case = Case::new("bad");
    case.wires.push(wire(
        "runaway",
        vec![[f32::INFINITY, 0.0, 0.0], [10.0, 0.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    broken.push(("an infinite coordinate", case));

    for (what, case) in broken {
        let error =
            svg_page_to_string(&case.page(), &PlotAssets::default(), &SvgOptions::default())
                .expect_err(what);
        assert!(matches!(error, SvgError::Invalid(_)), "{what}: {error}");
    }
}

#[test]
fn merge_lines_multiplies_on_the_leaves_inside_an_isolated_page() {
    let case = corpus()
        .into_iter()
        .find(|c| c.name == "hatches, merge_lines")
        .expect("the merge_lines page is in the corpus");
    let (svg, report) = write(&case);
    assert!(report.needs_mix_blend_mode);
    assert!(
        svg.contains("isolation:isolate"),
        "the page is not isolated"
    );
    // On the leaves, as a CSS property: `mix-blend-mode` does not inherit, and
    // librsvg only reads the style, not a same-named XML attribute.
    assert!(
        svg.contains("<path style=\"mix-blend-mode:multiply\"")
            || svg.contains(" style=\"mix-blend-mode:multiply\" d=")
    );
    let tree = parse(&svg);
    let shapes = actual_shapes(&tree, case.paper.0);
    assert!(shapes.iter().any(|s| s.multiply), "nothing multiplies");
    // The wipeout drops back to normal inside a multiplied page, so a page
    // that multiplies everything would be wrong.
    assert!(
        shapes.iter().any(|s| !s.multiply),
        "the wipeout should not multiply"
    );
}

#[test]
fn a_page_without_blending_stays_plain() {
    let case = corpus()
        .into_iter()
        .find(|c| c.name == "hatches")
        .expect("the hatch page is in the corpus");
    let (svg, report) = write(&case);
    assert!(!report.needs_mix_blend_mode);
    assert!(!svg.contains("mix-blend-mode"));
    assert!(!svg.contains("isolation"));
}

// ── Numbers (D2) ──────────────────────────────────────────────────────────

#[test]
fn numbers_are_plain_decimal_and_lose_nothing_that_matters() {
    let mut s = String::new();
    for (value, decimals, want) in [
        (0.0, 4, "0"),
        (-0.0, 4, "0"),
        (-0.00001, 4, "0"),
        (1.5, 4, "1.5"),
        (1.50009, 4, "1.5001"),
        (-12.25, 4, "-12.25"),
        (1e-9, 12, "0.000000001"),
        (297.0, 4, "297"),
    ] {
        s.clear();
        num(&mut s, value, decimals);
        assert_eq!(s, want, "{value} at {decimals} decimals");
        assert!(!s.contains('e'), "exponent notation in {s}");
    }
}

#[test]
fn the_coordinate_precision_follows_the_plot_scale() {
    // ½·10⁻ᵈ points, magnified by the scale, must stay inside 0.001 mm —
    // and must not spend a digit it does not need, since nine tenths of the
    // file is coordinates.
    for scale in [0.1, 1.0, 2.0, 50.0, 100.0, 1000.0] {
        let d = auto_decimals(scale);
        let worst = 0.5 * 10f64.powi(-(d as i32)) * scale.max(1.0) * PT_TO_MM;
        assert!(
            worst <= COORD_BUDGET_MM,
            "scale {scale}: {d} decimals leaves {worst} mm of error"
        );
        let one_fewer = 0.5 * 10f64.powi(-(d as i32 - 1)) * scale.max(1.0) * PT_TO_MM;
        assert!(
            d == 9 || one_fewer > COORD_BUDGET_MM,
            "scale {scale}: {d} decimals is one more than the budget needs"
        );
    }
    assert_eq!(auto_decimals(1.0), 3, "a 1:1 plot needs three decimals");
    // And the option overrides it.
    let mut case = Case::new("coarse");
    case.wires.push(wire(
        "a",
        vec![[1.234_567, 2.345_678, 0.0], [10.0, 10.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    let (svg, _) = svg_page_to_string(
        &case.page(),
        &PlotAssets::default(),
        &SvgOptions {
            decimals: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(svg.contains("M3.5,6.6"), "one decimal place: {svg}");
}

// ── R5: a mesh is one shape ───────────────────────────────────────────────

fn pt(x: f32, y: f32) -> PlotPoint {
    PlotPoint { x, y }
}

#[test]
fn a_two_triangle_square_comes_out_as_one_square() {
    // The shared diagonal must cancel: what is left is the outline.
    let tris = [
        [pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0)],
        [pt(0.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)],
    ];
    let rings = mesh_outline(&tris).expect("a clean manifold");
    assert_eq!(rings.len(), 1);
    assert_eq!(rings[0].len(), 4, "{:?}", rings[0]);
    let corners: Vec<(f32, f32)> = rings[0].iter().map(|p| (p.x, p.y)).collect();
    for corner in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        assert!(
            corners.contains(&corner),
            "{corner:?} missing from {corners:?}"
        );
    }
}

#[test]
fn a_glyph_counter_survives_as_its_own_ring() {
    // A square annulus: outer ring counter-clockwise, hole clockwise, meshed
    // into eight triangles. The outline must keep both rings and the hole must
    // keep the opposite winding, or a non-zero fill would swallow it.
    let outer = [pt(0.0, 0.0), pt(30.0, 0.0), pt(30.0, 30.0), pt(0.0, 30.0)];
    let inner = [
        pt(10.0, 10.0),
        pt(20.0, 10.0),
        pt(20.0, 20.0),
        pt(10.0, 20.0),
    ];
    let mut tris = Vec::new();
    for i in 0..4 {
        let (o0, o1) = (outer[i], outer[(i + 1) % 4]);
        let (i0, i1) = (inner[i], inner[(i + 1) % 4]);
        tris.push([o0, o1, i1]);
        tris.push([o0, i1, i0]);
    }
    let rings = mesh_outline(&tris).expect("a clean manifold");
    assert_eq!(rings.len(), 2, "outline and counter");
    let area = |ring: &Vec<PlotPoint>| {
        let mut sum = 0.0;
        for i in 0..ring.len() {
            let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
            sum += (p.x * q.y - q.x * p.y) as f64;
        }
        sum * 0.5
    };
    let areas: Vec<f64> = rings.iter().map(area).collect();
    assert!(
        areas.iter().any(|a| (*a - 900.0).abs() < 1e-3),
        "no 30×30 outline in {areas:?}"
    );
    assert!(
        areas.iter().any(|a| (*a + 100.0).abs() < 1e-3),
        "no 10×10 hole wound the other way in {areas:?}"
    );
}

#[test]
fn a_mesh_that_is_not_a_manifold_falls_back_instead_of_guessing() {
    // Two triangles meeting at a single point: the walk would have to guess
    // which edge leaves the shared vertex.
    let bowtie = [
        [pt(0.0, 0.0), pt(10.0, 0.0), pt(5.0, 5.0)],
        [pt(5.0, 5.0), pt(10.0, 10.0), pt(0.0, 10.0)],
    ];
    assert!(mesh_outline(&bowtie).is_none());
    // A degenerate triangle is not a mesh either.
    let degenerate = [[pt(0.0, 0.0), pt(10.0, 0.0), pt(0.0, 0.0)]];
    assert!(mesh_outline(&degenerate).is_none());

    // The page still draws it — as one path of triangles, not one path each.
    let mut case = Case::new("bowtie");
    let mut w = wire(
        "bowtie",
        vec![[0.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
        [0.2, 0.2, 0.2, 1.0],
        0.0,
    );
    w.wire.fill_tris = vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [5.0, 5.0, 0.0],
        [5.0, 5.0, 0.0],
        [10.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
    ];
    case.wires.push(w);
    let (svg, report) = write(&case);
    assert_eq!(report.mesh_fallbacks, 1);
    assert_eq!(report.mesh_outlines, 0);
    let shapes = actual_shapes(&parse(&svg), case.paper.0);
    let mesh = shapes
        .iter()
        .find(|s| s.rings.len() == 2)
        .expect("both triangles in one path");
    assert!((mesh.area().abs() - 50.0).abs() < 0.1, "{}", mesh.area());
}

#[test]
fn the_corpus_exercises_mesh_outlines() {
    let outlined: usize = corpus()
        .iter()
        .filter(|c| !c.options.stamp)
        .map(|c| write(c).1.mesh_outlines)
        .sum();
    assert!(outlined >= 4, "only {outlined} mesh fills in the corpus");
}

// ── R1: text the atlas cannot supply ──────────────────────────────────────

/// A page whose text quads point at atlas tiles that do not exist — the state
/// a page reaches on its own when the atlas grows between layout and plot.
fn page_with_unknown_glyphs() -> Case {
    let mut case = Case::new("lost text");
    let mut wire = crate::io::plot_corpus::text_wire("LOST", [20.0, 20.0, 0.0]);
    for vertex in &mut wire.wire.text_verts {
        vertex.uv = [0.123_456, 0.654_321];
    }
    wire.wire.name = "the-label".into();
    case.wires.push(wire);
    case
}

#[test]
fn a_page_missing_its_text_is_refused_by_default_and_says_which_wire() {
    let case = page_with_unknown_glyphs();
    let error = svg_page_to_string(&case.page(), &PlotAssets::default(), &SvgOptions::default())
        .expect_err("a page short of text must not come back as success");
    match &error {
        SvgError::MissingGlyphs { count, first, .. } => {
            assert!(*count >= 4, "four letters went missing, not {count}");
            let (wire, _quad) = first.as_ref().expect("the first one is named");
            assert_eq!(wire, "the-label");
        }
        other => panic!("expected a missing-glyph refusal, got {other}"),
    }
    assert!(error.to_string().contains("the-label"), "{error}");
}

#[test]
fn the_lenient_caller_gets_the_page_and_the_count() {
    let case = page_with_unknown_glyphs();
    let (svg, report) = svg_page_to_string(
        &case.page(),
        &PlotAssets::default(),
        &SvgOptions {
            missing_glyphs: MissingGlyphs::Report,
            ..Default::default()
        },
    )
    .expect("the lenient caller asked for the page anyway");
    assert!(report.missing_glyphs >= 4, "{report:?}");
    assert!(svg.contains("<svg"), "still a document");
}

// The point of the snapshot: what the page draws is decided once, at a moment
// the caller chooses, and the live atlas has no say afterwards. Here the live
// atlas holds every glyph the page needs and the snapshot holds none — an
// empty atlas of its own — and the page is refused: the emitter drew from the
// snapshot it was given, not from the atlas it could have reached for.
#[test]
fn the_page_is_drawn_from_the_snapshot_it_was_given_not_the_live_atlas() {
    use crate::io::plot_emit::GlyphSnapshot;
    use crate::scene::text::sdf_atlas::GlyphAtlas;

    let mut case = Case::new("snapshot decides");
    case.wires.push(crate::io::plot_corpus::text_wire(
        "HELLO",
        [20.0, 20.0, 0.0],
    ));
    let empty = GlyphSnapshot::of(&GlyphAtlas::new(64, 64));
    assert_eq!(empty.glyphs(), 0);
    assert!(
        empty.missing_in(case.wires.iter().map(|w| &w.wire)) >= 5,
        "the check should find every letter missing from an empty snapshot"
    );

    let error = svg_page_to_string(
        &case.page(),
        &PlotAssets {
            glyphs: Some(empty),
            ..Default::default()
        },
        &SvgOptions::default(),
    )
    .expect_err("the live atlas has the glyphs; the snapshot given does not");
    assert!(
        matches!(error, SvgError::MissingGlyphs { count, .. } if count >= 5),
        "{error}"
    );
}

// And the converse, which is what closes R1's window: a snapshot taken under
// the same lock as the layout has every key the quads carry, so the page is
// whole under the strict default no matter what the rest of this suite bakes
// into the shared atlas while the page is being written. Before the snapshot
// existed this scenario was a flake here, with the text silently gone.
#[test]
fn a_snapshot_taken_with_the_layout_keeps_the_page_whole() {
    let (wire, snapshot) =
        crate::io::plot_corpus::text_wire_with_snapshot("HELLO", [20.0, 20.0, 0.0]);
    let mut case = Case::new("snapshot");
    case.wires.push(wire);
    assert!(snapshot.glyphs() > 0, "the snapshot is empty");
    assert_eq!(snapshot.missing_in(case.wires.iter().map(|w| &w.wire)), 0);

    // Bake more into the shared atlas in the meantime.
    for text in ["MORE", "GLYPHS", "STILL MORE", "AND MORE AGAIN"] {
        let _ = crate::io::plot_corpus::text_wire(text, [0.0, 0.0, 0.0]);
    }

    let (svg, report) = svg_page_to_string(
        &case.page(),
        &PlotAssets {
            glyphs: Some(snapshot),
            ..Default::default()
        },
        &SvgOptions::default(),
    )
    .expect("the snapshot has what the page needs, so the strict default passes");
    assert_eq!(report.missing_glyphs, 0);
    assert!(
        svg.matches("<path").count() >= 5,
        "the page lost its glyphs anyway"
    );
}

// ── Files: naming, overwriting, publishing (D3) ───────────────────────────

/// A directory of this test's own, removed on the way out.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ocs-svg-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn job(cases: &[&str]) -> Vec<crate::io::plot_types::PdfPageInput> {
    let corpus = corpus();
    cases
        .iter()
        .map(|name| {
            corpus
                .iter()
                .find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("{name} is in the corpus"))
                .page_input()
        })
        .collect()
}

#[test]
fn one_page_keeps_its_name_and_several_are_all_numbered() {
    let base = std::path::Path::new("/plots/site plan.svg");
    assert_eq!(page_paths(base, 1), vec![base.to_path_buf()]);
    let three = page_paths(base, 3);
    let names: Vec<String> = three
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    // All of them, including the first: a script should not have to special
    // case page one, and three digits sort as text.
    assert_eq!(
        names,
        [
            "site plan-001.svg",
            "site plan-002.svg",
            "site plan-003.svg"
        ]
    );
    assert!(three.iter().all(|p| p.parent() == base.parent()));
}

#[test]
fn a_case_only_difference_is_the_same_file() {
    let paths: Vec<std::path::PathBuf> = ["a/Plan.svg", "a/other.svg", "a/plan.svg"]
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    assert_eq!(first_case_insensitive_clash(&paths), Some((0, 2)));
    assert_eq!(first_case_insensitive_clash(&paths[..2]), None);
}

#[test]
fn a_multi_page_job_writes_numbered_files_that_parse() {
    let scratch = Scratch::new("multi");
    let pages = job(&["polyline with pen-up", "dash patterns", "text"]);
    let batch = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &lenient_files(),
    )
    .unwrap();
    assert_eq!(batch.pages.len(), 3);
    assert!(!batch.dry_run);
    assert_eq!(
        scratch.files(),
        ["plot-001.svg", "plot-002.svg", "plot-003.svg"],
        "no leftovers from the temporary files either"
    );
    for outcome in &batch.pages {
        let text = std::fs::read_to_string(&outcome.path).unwrap();
        assert_eq!(text.len(), outcome.bytes);
        parse(&text); // it is a document, not just bytes
        assert!(outcome.report.elements > 0);
    }
}

#[test]
fn an_existing_file_stops_the_job_until_it_is_forced() {
    let scratch = Scratch::new("overwrite");
    let pages = job(&["polyline with pen-up", "dash patterns"]);
    std::fs::write(scratch.join("plot-002.svg"), b"last week's issue").unwrap();

    let error = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &SvgWriteOptions::default(),
    )
    .expect_err("the second page is already there");
    assert!(error.to_string().contains("plot-002.svg"), "{error}");
    assert_eq!(
        scratch.files(),
        ["plot-002.svg"],
        "page one must not have been written either"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.join("plot-002.svg")).unwrap(),
        "last week's issue"
    );

    let forced = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &SvgWriteOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(forced.pages.len(), 2);
    assert!(std::fs::read_to_string(scratch.join("plot-002.svg"))
        .unwrap()
        .starts_with("<?xml"));
}

#[test]
fn a_page_that_cannot_be_written_stops_the_job_before_any_file_moves() {
    let scratch = Scratch::new("refused");
    let mut pages = job(&["polyline with pen-up", "dash patterns"]);
    pages[1].options.stamp = true;

    let error = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &SvgWriteOptions::default(),
    )
    .expect_err("the stamp is not supported");
    match &error {
        SvgError::Page { page, source } => {
            assert_eq!(*page, 2);
            assert!(matches!(**source, SvgError::Unsupported(_)), "{source}");
        }
        other => panic!("expected a page error, got {other}"),
    }
    assert!(
        scratch.files().is_empty(),
        "page one was published anyway: {:?}",
        scratch.files()
    );
}

#[test]
fn a_failure_after_the_first_page_says_what_is_already_on_disk() {
    // Page two's target is a non-empty directory: it cannot be replaced by a
    // file, and by then page one has been published. A file each is not a
    // transaction, so the error has to say so rather than imply a rollback.
    let scratch = Scratch::new("partial");
    let blocked = scratch.join("plot-002.svg");
    std::fs::create_dir_all(&blocked).unwrap();
    std::fs::write(blocked.join("occupied"), b"x").unwrap();

    let error = export_svg_pages(
        &job(&["polyline with pen-up", "dash patterns", "text"]),
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &SvgWriteOptions {
            force: true,
            ..lenient_files()
        },
    )
    .expect_err("page two cannot be written");
    match &error {
        SvgError::Partial {
            published, failed, ..
        } => {
            assert_eq!(published.len(), 1);
            assert!(published[0].ends_with("plot-001.svg"), "{published:?}");
            assert!(failed.ends_with("plot-002.svg"), "{failed:?}");
        }
        other => panic!("expected a partial-publication error, got {other}"),
    }
    assert!(error.to_string().contains("plot-001.svg"), "{error}");
    assert!(
        scratch.files().contains(&"plot-001.svg".to_string()),
        "page one should still be there: {:?}",
        scratch.files()
    );
    assert!(
        !scratch.files().contains(&"plot-003.svg".to_string()),
        "the job should have stopped"
    );
}

#[test]
fn an_svgz_target_gets_gzip_and_an_svg_target_never_does() {
    let scratch = Scratch::new("svgz");
    let pages = job(&["hatches"]);
    let plain = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &SvgWriteOptions::default(),
    )
    .unwrap();
    let zipped = export_svg_pages(
        &pages,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svgz"),
        &SvgWriteOptions::default(),
    )
    .unwrap();

    let raw = std::fs::read(&plain.pages[0].path).unwrap();
    let gz = std::fs::read(&zipped.pages[0].path).unwrap();
    assert_eq!(&raw[..5], b"<?xml", "an .svg must not be gzip");
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "an .svgz must be gzip");
    assert!(
        gz.len() * 2 < raw.len(),
        "gzip saved almost nothing: {} → {}",
        raw.len(),
        gz.len()
    );
    assert_eq!(
        zipped.pages[0].bytes,
        gz.len(),
        "the report counts the file"
    );
    // The one consumer in this tree reads it, which is what `.svgz` is for.
    usvg::Tree::from_data(&gz, &usvg::Options::default())
        .expect("usvg reads the compressed document");
}

#[test]
fn a_dry_run_renders_and_resolves_but_writes_nothing() {
    let scratch = Scratch::new("dry");
    // Lenient about text: the ctb page's glyphs live in the shared atlas, and
    // whether they survive another test's reset is R1's business, not this.
    let dry = SvgWriteOptions {
        dry_run: true,
        ..lenient_files()
    };
    let batch = export_svg_pages(
        &job(&["hatches", "ctb"]),
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &dry,
    )
    .unwrap();
    assert!(batch.dry_run);
    assert_eq!(batch.pages.len(), 2);
    assert!(batch.pages[0].path.ends_with("plot-001.svg"));
    assert!(batch.pages.iter().all(|p| p.bytes > 0));
    assert!(scratch.files().is_empty(), "{:?}", scratch.files());

    // The rehearsal still finds what the real run would refuse.
    let mut stamped = job(&["hatches"]);
    stamped[0].options.stamp = true;
    assert!(export_svg_pages(
        &stamped,
        None,
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &dry
    )
    .is_err());
}

#[test]
fn a_page_style_table_wins_over_the_job_wide_one() {
    // D4's multi-page rule, kept from the PDF path: the page's own CTB first,
    // the job's as the fallback.
    let scratch = Scratch::new("ctb");
    let mut pages = job(&["ctb", "ctb"]);
    pages[1].plot_style = None;
    let styled = crate::io::plot_corpus::styled_ctb();
    // The ctb page carries text laid out into the shared atlas, which another
    // test may reset or grow before this page is written; that is R1's
    // subject, not this test's, so write leniently (see `lenient`).
    let batch = export_svg_pages(
        &pages,
        Some(&styled),
        &PlotAssets::default(),
        &scratch.join("plot.svg"),
        &lenient_files(),
    )
    .unwrap();
    let with_page_style = std::fs::read_to_string(&batch.pages[0].path).unwrap();
    let with_job_style = std::fs::read_to_string(&batch.pages[1].path).unwrap();
    assert_eq!(
        with_page_style, with_job_style,
        "the same table by either route must draw the same page"
    );
    // And it is really the table talking, not both pages ignoring it.
    let mut plain = job(&["ctb"]);
    plain[0].plot_style = None;
    let bare = export_svg_pages(
        &plain,
        None,
        &PlotAssets::default(),
        &scratch.join("bare.svg"),
        &lenient_files(),
    )
    .unwrap();
    assert_ne!(
        with_page_style,
        std::fs::read_to_string(&bare.pages[0].path).unwrap(),
        "the CTB changed nothing"
    );
}

// ── In memory: the web build's download (P4.1) ────────────────────────────

#[test]
fn the_web_download_is_the_file_the_desktop_would_have_written() {
    // The web build has no filesystem; it takes the document from
    // `svg_job_to_string` and hands it to the browser. That wrapper must not
    // be a second writer: for the same page, job-wide style and options, its
    // bytes are the bytes `export_svg_pages` put on disk. Three pages that
    // exercise different parts of the writer — plain strokes, a page-level
    // CTB (so `as_plot_page`'s priority rule is on the line too), text from
    // the shared snapshot.
    let scratch = Scratch::new("web");
    let assets = shared_assets();
    let styled = crate::io::plot_corpus::styled_ctb();
    for (index, name) in ["polyline with pen-up", "ctb", "text"]
        .into_iter()
        .enumerate()
    {
        let pages = job(&[name]);
        let batch = export_svg_pages(
            &pages,
            Some(&styled),
            &assets,
            &scratch.join(&format!("plot-{index}.svg")),
            &lenient_files(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let on_disk = std::fs::read(&batch.pages[0].path).unwrap();
        let (downloaded, report) = svg_job_to_string(&pages, Some(&styled), &assets, &lenient())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            downloaded.as_bytes(),
            on_disk.as_slice(),
            "{name}: the download and the file differ"
        );
        assert_eq!(report, batch.pages[0].report, "{name}");
        assert!(report.elements > 0, "{name}: an empty page proves nothing");
    }
}

#[test]
fn the_web_download_is_one_page_and_refuses_a_longer_job() {
    // A download has no numbering. A job of several pages is refused and the
    // error says how many there were — not quietly cut down to page one; an
    // empty job is refused the way the files refuse it.
    let two = job(&["polyline with pen-up", "dash patterns"]);
    let error = svg_job_to_string(&two, None, &PlotAssets::default(), &lenient())
        .expect_err("two pages are not one download");
    assert!(matches!(error, SvgError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("this job has 2"), "{error}");

    let error = svg_job_to_string(&[], None, &PlotAssets::default(), &lenient())
        .expect_err("nothing to download");
    assert!(error.to_string().contains("no pages"), "{error}");
}

// ── Layer 3: what the picture looks like ──────────────────────────────────

/// Render at `px_per_mm`, on white, and return the pixmap.
fn render(svg: &str, px_per_mm: f32) -> tiny_skia::Pixmap {
    crate::io::raster_compare::render_svg(svg, px_per_mm)
}

/// 0 = white, 1 = black.
fn ink(pixmap: &tiny_skia::Pixmap, x: u32, y: u32) -> f64 {
    let p = pixmap.pixel(x, y).expect("inside the pixmap");
    1.0 - (p.red() as f64 + p.green() as f64 + p.blue() as f64) / (3.0 * 255.0)
}

/// Mean ink over a rectangle of pixels.
fn ink_in(pixmap: &tiny_skia::Pixmap, x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0_f64;
    for y in y0..y1.min(pixmap.height()) {
        for x in x0..x1.min(pixmap.width()) {
            sum += ink(pixmap, x, y);
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

// The one thing no assertion replaces: looking at the page. Writes every
// corpus page next to a PNG of it, for the eyeball check §8's third layer asks
// for on real drawings.
#[test]
#[ignore = "writes sample pages to the temp directory to look at"]
fn dump_the_corpus_for_a_human() {
    let dir = std::env::temp_dir().join("ocs-svg-corpus");
    std::fs::create_dir_all(&dir).unwrap();
    for (i, case) in corpus().iter().filter(|c| !c.options.stamp).enumerate() {
        let stem = format!("{i:02}-{}", case.name.replace([' ', '/', ','], "_"));
        let (svg, report) = write(case);
        std::fs::write(dir.join(format!("{stem}.svg")), &svg).unwrap();
        render(&svg, 6.0)
            .save_png(dir.join(format!("{stem}.png")))
            .unwrap();
        println!("{stem}: {report:?}");
    }
    println!("wrote {}", dir.display());
}

// Rasterise SVGs this crate wrote, for the eyeball pass §8's third layer asks
// for on real drawings: `OCS_SVG_PREVIEW=<dir or file> cargo test --lib
// preview_written_svgs -- --ignored --nocapture`.
#[test]
#[ignore = "renders the SVGs named by OCS_SVG_PREVIEW to PNGs beside them"]
fn preview_written_svgs() {
    let Ok(target) = std::env::var("OCS_SVG_PREVIEW") else {
        println!("set OCS_SVG_PREVIEW to a .svg file or a directory of them");
        return;
    };
    let target = std::path::PathBuf::from(target);
    let files: Vec<std::path::PathBuf> = if target.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().is_some_and(|e| e == "svg")).then_some(path)
            })
            .collect();
        files.sort();
        files
    } else {
        vec![target]
    };
    let dpi: f32 = std::env::var("OCS_SVG_PREVIEW_DPI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150.0);
    for file in files {
        let svg = std::fs::read_to_string(&file).unwrap();
        let png = file.with_extension("png");
        let pixmap = render(&svg, dpi / 25.4);
        pixmap.save_png(&png).unwrap();
        println!(
            "{} → {} ({}×{} px, svg {} KiB)",
            file.file_name().unwrap().to_string_lossy(),
            png.file_name().unwrap().to_string_lossy(),
            pixmap.width(),
            pixmap.height(),
            svg.len() / 1024
        );
    }
}

#[test]
fn a_hundred_millimetre_line_is_a_hundred_millimetres_from_the_top() {
    // The one test that would catch a wrong unit, a wrong page origin or a
    // missing Y flip, all three.
    let mut case = Case::new("ruler");
    case.paper = (297.0, 210.0);
    case.wires.push(wire(
        "ruler",
        vec![[50.0, 160.0, 0.0], [150.0, 160.0, 0.0]],
        WireModel::WHITE,
        0.0,
    ));
    let ppmm = 4.0;
    let pixmap = render(&write(&case).0, ppmm);
    let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0u32, u32::MAX, 0u32);
    for y in 0..pixmap.height() {
        for x in 0..pixmap.width() {
            if ink(&pixmap, x, y) > 0.5 {
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    let mm = |v: u32| v as f64 / ppmm as f64;
    assert!((mm(x0) - 50.0).abs() < 0.5, "left edge at {} mm", mm(x0));
    assert!((mm(x1) - 150.0).abs() < 0.5, "right edge at {} mm", mm(x1));
    // CAD y = 160 on a 210 mm sheet is 50 mm from the top of the image.
    let mid = mm((y0 + y1) / 2);
    assert!(
        (mid - 50.0).abs() < 0.5,
        "the line sits {mid} mm from the top"
    );
}

#[test]
fn a_wipeout_masks_the_ink_under_it() {
    let mut case = Case::new("wipeout");
    case.paper = (100.0, 100.0);
    case.hatches.push(hatch(
        "SOLID",
        square(20.0, 20.0, 60.0),
        HatchPattern::Solid,
        [0.1, 0.1, 0.1, 1.0],
    ));
    let mut cover = hatch(
        "WIPEOUT_FILL",
        square(30.0, 30.0, 20.0),
        HatchPattern::Solid,
        [0.0, 0.0, 0.0, 1.0],
    );
    // Deeper than the black fill, so the sort paints it afterwards.
    cover.draw_depth = 0.6;
    case.wipeouts.push(cover);
    let ppmm = 4.0;
    let pixmap = render(&write(&case).0, ppmm);
    let px = |mm: f64| (mm * ppmm as f64) as u32;
    // Inside the wipeout: paper white. Outside it, still inside the hatch: ink.
    assert!(
        ink_in(&pixmap, px(33.0), px(53.0), px(47.0), px(67.0)) < 0.02,
        "the wipeout did not mask"
    );
    assert!(
        ink_in(&pixmap, px(60.0), px(30.0), px(75.0), px(45.0)) > 0.8,
        "the hatch is missing"
    );
}

#[test]
fn a_mesh_fill_has_no_seam_where_its_triangles_meet() {
    // Two triangles sharing a diagonal. Filled separately, every renderer that
    // anti-aliases per shape leaves a pale line along the join (R5); as one
    // path there is nothing to leave it on.
    let mut case = Case::new("seam");
    case.paper = (60.0, 60.0);
    let mut w = wire(
        "solid",
        vec![[10.0, 10.0, 0.0], [50.0, 50.0, 0.0]],
        [0.0, 0.0, 0.0, 1.0],
        0.0,
    );
    w.wire.color = [0.05, 0.05, 0.05, 1.0];
    w.wire.fill_tris = vec![
        [10.0, 10.0, 0.0],
        [50.0, 10.0, 0.0],
        [50.0, 50.0, 0.0],
        [10.0, 10.0, 0.0],
        [50.0, 50.0, 0.0],
        [10.0, 50.0, 0.0],
    ];
    // Nothing else on the page: the wire itself would cross the diagonal.
    w.wire.points = vec![[10.0, 10.0, 0.0]];
    case.wires.push(w);
    let (svg, report) = write(&case);
    assert_eq!(report.mesh_outlines, 1, "the square should be one outline");
    let ppmm = 8.0;
    let pixmap = render(&svg, ppmm);
    // Walk the diagonal from (12,12) to (48,48) in CAD mm; in image space the
    // square spans the same x and the mirrored y.
    let mut palest: f64 = 1.0;
    for step in 0..=36 {
        let mm = 12.0 + step as f64;
        let x = (mm * ppmm as f64) as u32;
        let y = ((60.0 - mm) * ppmm as f64) as u32;
        palest = palest.min(ink(&pixmap, x, y));
    }
    assert!(
        palest > 0.9,
        "a seam runs along the diagonal: palest pixel {palest}"
    );
}

#[test]
fn a_missing_glyph_hides_from_a_whole_page_rate_but_not_from_the_local_check() {
    // Why §8 refuses a whole-page difference rate as the only judge: drop a
    // word from an A3 sheet and the page is still 99.9 % identical.
    //
    // The text is plotted from the snapshot taken with its layout, under the
    // strict default: this test used to retry around R1's silent skip when
    // another test grew the shared atlas between the two, and now it cannot
    // happen — a page short of its glyphs would be refused, not retried.
    let (wire, snapshot) =
        crate::io::plot_corpus::text_wire_with_snapshot("HELLO", [20.0, 100.0, 0.0]);
    let mut case = Case::new("text");
    case.paper = (297.0, 210.0);
    case.wires.push(wire);
    let assets = PlotAssets {
        glyphs: Some(snapshot),
        ..Default::default()
    };
    let (svg, _) = svg_page_to_string(&case.page(), &assets, &SvgOptions::default())
        .expect("the page's own snapshot has every glyph");
    let first = svg.find("<path").expect("glyph paths");
    let end = svg[first..].find("/>").unwrap() + first + 2;
    let without = format!("{}{}", &svg[..first], &svg[end..]);

    let ppmm = 8.0;
    let (whole, broken) = (render(&svg, ppmm), render(&without, ppmm));
    let mut differing = 0u64;
    let mut total = 0u64;
    let mut region = [u32::MAX, u32::MAX, 0u32, 0u32];
    for y in 0..whole.height() {
        for x in 0..whole.width() {
            total += 1;
            if (ink(&whole, x, y) - ink(&broken, x, y)).abs() > 0.05 {
                differing += 1;
                region = [
                    region[0].min(x),
                    region[1].min(y),
                    region[2].max(x + 1),
                    region[3].max(y + 1),
                ];
            }
        }
    }
    let page_rate = differing as f64 / total as f64;
    assert!(differing > 0, "the mutation changed nothing");
    assert!(
        page_rate < 0.001,
        "the whole-page rate was supposed to be tiny, it is {page_rate}"
    );
    // The structural comparison, which does not average over the page, says no.
    let expected = expected_shapes(&record_with(&case.page(), &assets), case.paper.1);
    assert!(compare(&expected, &actual_shapes(&parse(&without), case.paper.0)).is_err());
    // And so does the raster, once the difference is measured where it is
    // instead of spread over a sheet: the ink in the affected region is gone,
    // and the local rate is orders of magnitude above the page rate.
    let before = ink_in(&whole, region[0], region[1], region[2], region[3]);
    let after = ink_in(&broken, region[0], region[1], region[2], region[3]);
    let local_rate =
        differing as f64 / (((region[2] - region[0]) * (region[3] - region[1])) as f64).max(1.0);
    assert!(
        before - after > 0.1,
        "local ink barely moved: {before} → {after}"
    );
    assert!(
        local_rate > 100.0 * page_rate,
        "the local rate {local_rate} is not far above the page rate {page_rate}"
    );
}

// ── Layer 3, the other half: PDF and SVG as pictures (P5.2) ───────────────
//
// The same page goes out both doors — export_pdf_pages and the SVG writer —
// and an external rasteriser (mutool / pdftoppm, never in the dependency
// tree) turns the PDF into pixels next to resvg's rendering of the SVG.
// `raster_compare` counts the pixels one picture cannot explain in the other.
// Without a rasteriser on the machine these tests print SKIPPED and check
// nothing: a loud skip, not a quiet pass (`#[ignore]` cannot be decided at
// runtime).

fn rasterizer_or_skip(what: &str) -> Option<crate::io::raster_compare::PdfRasterizer> {
    let found = crate::io::raster_compare::PdfRasterizer::discover();
    if found.is_none() {
        eprintln!(
            "SKIPPED {what}: no PDF rasteriser. Install one (e.g. `winget \
             install oschwartz10612.Poppler`) or point OCS_PDF_RASTERIZER at \
             mutool / pdftoppm — the PDF↔SVG raster comparison checked nothing."
        );
    }
    found
}

/// The case's page, out the PDF door and back as pixels.
fn pdf_raster(
    tool: &crate::io::raster_compare::PdfRasterizer,
    case: &Case,
    dir: &std::path::Path,
    dpi: f32,
) -> tiny_skia::Pixmap {
    let pdf = dir.join(format!(
        "{}.pdf",
        case.name.replace([' ', '/', ',', ':'], "_")
    ));
    crate::io::pdf_export::export_pdf_pages(&[case.page_input()], &pdf, None)
        .unwrap_or_else(|e| panic!("{}: pdf export: {e}", case.name));
    tool.rasterize(&pdf, dpi, dir)
        .unwrap_or_else(|e| panic!("{}: {e}", case.name))
}

/// A page has text quads, whose glyphs the PDF exporter snapshots at write
/// time while the SVG side pins its own — under the parallel test runner the
/// shared atlas can turn over in between, so the routine comparison leaves
/// text pages to the single-threaded evidence run.
fn has_text(case: &Case) -> bool {
    case.wires.iter().any(|w| !w.wire.text_verts.is_empty())
}

// The whole corpus, PDF against SVG, at the pixels. 300 dpi: the plan states
// its 1 px shift tolerance at 600 dpi, so the same pixel at half the density
// is spatially stricter, and the run stays inside a couple of minutes.
#[test]
fn the_pdf_and_the_svg_rasterise_to_the_same_picture() {
    let Some(tool) = rasterizer_or_skip("corpus raster comparison") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("ocs-raster-cmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dpi = 300.0;
    let cfg = crate::io::raster_compare::RasterCmp::default();
    let mut compared = 0;
    let mut worst_clean = 0u32;
    for case in &corpus() {
        if case.options.stamp || has_text(case) {
            continue;
        }
        // Weights-off pages draw everything at the 0.1 pt hairline sentinel:
        // 0.83 px even at 600 dpi. Below one device pixel the engines lay
        // the same ink at different sub-pixel phases — poppler one 83% row,
        // resvg 8% + 76% across two — and no value tolerance short of one
        // that would also excuse wrong flat colours absorbs that. The width
        // itself is a number layers 1 and 2 pin exactly, and this page's
        // geometry rides above the floor in the other two pen-width
        // variants; the evidence run records the sub-pixel pair anyway.
        if !case.options.object_lineweights {
            continue;
        }
        // The merge_lines page is the far-from-origin case: its plot window
        // sits at world (500000, 4500000) mm, so every multiply fill but one
        // lands ~1.3e7 SVG units off the sheet, cut away by the page clip.
        // resvg 0.45.1 drops the one on-sheet fill in exactly that
        // arrangement — an `isolation:isolate` layer whose content bounding
        // box spans the far-off geometry, composited through an ancestor
        // `clip-path` — leaving white where poppler paints red from the
        // equivalent PDF. Take away any one ingredient and it paints: the
        // same page renders correctly with the isolation attribute removed
        // or with the page clip removed, and the minimal pair kept in
        // docs/evidence/2026-09-09-svg-pdf-raster/ differs only by the clip
        // wrapper. The SVG itself is valid: layer 2 parses the multiply
        // structure back (`merge_lines_multiplies_on_the_leaves_inside_an_
        // isolated_page`), and the 600 dpi evidence run records the pair
        // with a diff map. A renderer limitation, not an export fault, so
        // the routine gate excuses it exactly as it does the sub-pixel
        // hairlines.
        if case.options.merge_lines {
            continue;
        }
        let pdf_pixels = pdf_raster(&tool, case, &dir, dpi);
        let svg_pixels = render(&write(case).0, dpi / 25.4);
        let verdict = crate::io::raster_compare::compare(&pdf_pixels, &svg_pixels, &cfg)
            .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert!(
            verdict.ok(),
            "{}: {} tiles over budget ({} defect px total, worst tile {:?})",
            case.name,
            verdict.failing_tiles(),
            verdict.defects_total,
            verdict.worst,
        );
        worst_clean = worst_clean.max(verdict.worst.2);
        compared += 1;
    }
    assert!(compared >= 15, "the corpus shrank to {compared} pages");
    eprintln!(
        "PDF↔SVG raster: {compared} pages agree via {} ({}); worst clean tile \
         {worst_clean} defect px against a budget of {}",
        tool.exe.display(),
        tool.version,
        cfg.tile_budget,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// The planted faults the plan names: content missing, the wrong scale, a
// dash pattern off its phase, a wipeout drawn in the wrong order. Each pair
// is checked clean first — the PDF of the good page against the SVG of the
// good page — so a catch is the fault's doing, not the pairing's.
#[test]
fn the_raster_comparison_catches_the_planted_faults() {
    let Some(tool) = rasterizer_or_skip("raster fault injection") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("ocs-raster-faults-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dpi = 300.0;
    let cfg = crate::io::raster_compare::RasterCmp::default();

    let lines = |name: &'static str, keep_both: bool| {
        let mut case = Case::new(name);
        case.paper = (150.0, 100.0);
        case.wires.push(wire(
            "keep",
            vec![[10.0, 30.0, 0.0], [140.0, 30.0, 0.0]],
            WireModel::WHITE,
            0.0,
        ));
        if keep_both {
            case.wires.push(wire(
                "lose",
                vec![[10.0, 60.0, 0.0], [140.0, 60.0, 0.0]],
                WireModel::WHITE,
                0.0,
            ));
        }
        case
    };
    let scaled = |name: &'static str, scale: f32| {
        let mut case = lines(name, true);
        case.scale = scale;
        case
    };
    let dashed = |name: &'static str, pattern: [f64; 2]| {
        let mut case = Case::new(name);
        case.paper = (150.0, 100.0);
        let mut dashed = wire(
            "dashed",
            vec![[10.0, 50.0, 0.0], [140.0, 50.0, 0.0]],
            WireModel::WHITE,
            0.0,
        );
        dashed.wire.pattern_length = (pattern[0] - pattern[1]) as f32;
        dashed.wire.pattern = [
            pattern[0] as f32,
            pattern[1] as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        case.wires.push(dashed);
        case
    };
    let covered = |name: &'static str, wipeout_first: bool| {
        let mut case = Case::new(name);
        case.paper = (150.0, 100.0);
        case.hatches.push(hatch(
            "ink",
            square(40.0, 30.0, 40.0),
            HatchPattern::Solid,
            [0.05, 0.05, 0.05, 1.0],
        ));
        case.wipeouts.push(hatch(
            "WIPEOUT",
            square(50.0, 40.0, 20.0),
            HatchPattern::Solid,
            [1.0, 1.0, 1.0, 1.0],
        ));
        // Two render groups; the second is drawn after the first, so which
        // group holds the wipeout decides whether it masks the ink or the
        // ink paints over it.
        case.options.group_splits = if wipeout_first {
            crate::io::plot_types::PlotGroupSplits {
                wires: 0,
                hatches: 0,
                wipeouts: 1,
            }
        } else {
            crate::io::plot_types::PlotGroupSplits {
                wires: 0,
                hatches: 1,
                wipeouts: 0,
            }
        };
        case
    };

    // (told the PDF, told the SVG, what went wrong)
    let faults: [(Case, Case, &str); 4] = [
        (
            lines("fault-missing", true),
            lines("fault-missing-svg", false),
            "a whole line is missing",
        ),
        (
            scaled("fault-scale", 1.0),
            scaled("fault-scale-svg", 1.02),
            "the drawing is 2% too large",
        ),
        (
            dashed("fault-dash", [6.0, -6.0]),
            dashed("fault-dash-svg", [3.0, -3.0]),
            "the dashes land where the gaps should be",
        ),
        (
            covered("fault-wipeout", false),
            covered("fault-wipeout-svg", true),
            "the wipeout is drawn before the ink it should cover",
        ),
    ];
    for (good, bad, what) in &faults {
        let pdf_pixels = pdf_raster(&tool, good, &dir, dpi);
        let clean = crate::io::raster_compare::compare(
            &pdf_pixels,
            &render(&write(good).0, dpi / 25.4),
            &cfg,
        )
        .unwrap();
        assert!(
            clean.ok(),
            "{}: the clean pairing already fails ({} defects, worst {:?})",
            good.name,
            clean.defects_total,
            clean.worst,
        );
        let verdict = crate::io::raster_compare::compare(
            &pdf_pixels,
            &render(&write(bad).0, dpi / 25.4),
            &cfg,
        )
        .unwrap();
        assert!(
            !verdict.ok(),
            "not caught: {what} ({} defect px, worst tile {:?})",
            verdict.defects_total,
            verdict.worst,
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// The evidence run: the whole corpus, text pages included, at the plan's
// 600 dpi, with a diff map written for anything that differs. Run it alone —
// `cargo test --lib dump_raster_evidence -- --ignored --nocapture
// --test-threads=1` — so the shared glyph atlas holds still between the two
// exports of a text page.
#[test]
#[ignore = "writes the PDF↔SVG raster evidence table and diff maps to docs/evidence"]
fn dump_raster_evidence() {
    let Some(tool) = rasterizer_or_skip("raster evidence") else {
        return;
    };
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("evidence")
        .join("2026-09-09-svg-pdf-raster");
    std::fs::create_dir_all(&out).unwrap();
    let work = std::env::temp_dir().join(format!("ocs-raster-evidence-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let dpi = 600.0;
    let cfg = crate::io::raster_compare::RasterCmp::default();
    let mut rows = vec![format!(
        "# tool: {} ({}), -thinlinemode shape; resvg {}; {} dpi; shift {} px, \
         value tol {}/255, tile {} px, budget {} px, conflation ink {}/255 \
         within {} px",
        tool.exe.display(),
        tool.version,
        "0.45.1",
        dpi,
        cfg.shift_px,
        cfg.value_tol,
        cfg.tile_px,
        cfg.tile_budget,
        cfg.seam_ink,
        cfg.seam_reach_px,
    )];
    rows.push("case\tdefect_px\tworst_tile\tfailing_tiles\tverdict\tnote".into());
    for case in &corpus() {
        if case.options.stamp {
            continue;
        }
        let label = if case.name == "pen widths" {
            // Three variants share the name; the flags tell them apart.
            format!(
                "{} (scale_lw={}, object_lw={})",
                case.name, case.options.scale_lineweights, case.options.object_lineweights
            )
        } else {
            case.name.to_string()
        };
        let note = if case.options.merge_lines {
            "resvg 0.45.1 drops the on-sheet multiply fill when an isolated \
             layer's bbox spans the far-off geometry and an ancestor \
             clip-path cuts the page; poppler paints it; excused by the \
             routine gate (see its comment), minimal pair in this folder"
        } else if case.options.object_lineweights {
            ""
        } else {
            "sub-pixel hairlines (0.1 pt = 0.83 px): below the raster floor, \
             phase differs by engine policy; width pinned at layers 1-2"
        };
        let assets = shared_assets();
        let pdf_pixels = pdf_raster(&tool, case, &work, dpi);
        let svg_pixels = render(&write_with(case, &assets).0, dpi / 25.4);
        let verdict = crate::io::raster_compare::compare(&pdf_pixels, &svg_pixels, &cfg).unwrap();
        rows.push(format!(
            "{}\t{}\t{:?}\t{}\t{}\t{}",
            label,
            verdict.defects_total,
            verdict.worst,
            verdict.failing_tiles(),
            if verdict.ok() { "ok" } else { "FAIL" },
            note,
        ));
        if verdict.defects_total > 0 {
            let map = crate::io::raster_compare::diff_map(&pdf_pixels, &verdict);
            map.save_png(out.join(format!(
                "diff-{}.png",
                label.replace([' ', '/', ',', ':', '(', ')', '='], "_")
            )))
            .unwrap();
        }
    }
    let table = out.join("corpus-600dpi.tsv");
    std::fs::write(&table, rows.join("\n") + "\n").unwrap();
    println!("wrote {}", table.display());
    let _ = std::fs::remove_dir_all(&work);
}

// The other half of the evidence — the three real sheets — lives with the
// headless plot tests in `app::automation`, which can open a drawing; the
// pages go out both doors there exactly as the corpus pages do here.

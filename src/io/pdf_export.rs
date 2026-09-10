// PDF export — writes plotted pages to a PDF file with printpdf.
//
// What goes on the page is decided by `io::plot_emit::emit_plot_content`, the
// backend-independent traversal (grouping, depth order, CTB, pens, dashes,
// hatch segments, glyph geometry). This module owns only what is PDF-specific:
// `PdfSink`, which turns that operation stream into `printpdf::Op`s; the
// document / page / graphics-state plumbing; saving; and the save dialog.
//
// Coordinate system: CAD uses mm units with origin at bottom-left and Y up.
// printpdf's Point::new(Mm, Mm) also has origin at bottom-left, so no Y-flip
// is needed — the emitter shifts the coordinates by (offset_x, offset_y) to
// place the drawing origin at the paper origin.
//
// The web build has no `printpdf` (it pulls a wasm-incompatible `memchr` via
// lopdf → nom_locate) and no filesystem, so PDF export is native-only; the web
// build gets stubs so the call sites still compile.

pub use crate::io::plot_types::{PdfPageInput, PdfPlotOptions, PlotGroupSplits, PlotWire};

#[cfg(not(target_arch = "wasm32"))]
use crate::io::plot_emit::{
    emit_plot_content, BuiltinFace, FillRule, LineCap, LineJoin, PlotAssets, PlotBlend, PlotOp,
    PlotPage, PlotPoint, PlotSink,
};
use crate::io::plot_style::PlotStyleTable;
use crate::scene::model::hatch_model::HatchModel;
#[cfg(not(target_arch = "wasm32"))]
use printpdf::{
    BlendMode, BuiltinFont, Color, CurTransMat, ExtendedGraphicsState, ExtendedGraphicsStateId,
    Line, LineCapStyle, LineDashPattern, LineJoinStyle, LinePoint, Mm, Op, PaintMode, PdfDocument,
    PdfFontHandle, PdfPage, PdfSaveOptions, Point, Polygon, PolygonRing, Pt, Rect, Rgb, TextItem,
    WindingOrder,
};
use std::path::Path;

#[cfg(target_arch = "wasm32")]
pub fn export_pdf(
    _wires: &[PlotWire],
    _hatches: &[HatchModel],
    _wipeouts: &[HatchModel],
    _paper_w: f64,
    _paper_h: f64,
    _offset_x: f64,
    _offset_y: f64,
    _rotation_deg: i32,
    _scale: f32,
    _clip: Option<(f32, f32, f32, f32)>,
    _path: &Path,
    _plot_style: Option<&PlotStyleTable>,
    _options: PdfPlotOptions,
) -> Result<(), String> {
    Err("PDF export is not available in the web version.".into())
}

#[cfg(target_arch = "wasm32")]
pub fn export_pdf_pages(
    _pages: &[PdfPageInput],
    _path: &Path,
    _plot_style: Option<&PlotStyleTable>,
) -> Result<(), String> {
    Err("PDF export is not available in the web version.".into())
}

#[cfg(target_arch = "wasm32")]
pub async fn pick_pdf_path_owned(_stem: String) -> Option<std::path::PathBuf> {
    None
}

// ── Public entry point ────────────────────────────────────────────────────

/// Export `wires` to a PDF file.
///
/// - `paper_w` / `paper_h`: page dimensions in mm (already swapped for 90°/270° by caller).
/// - `offset_x` / `offset_y`: added to every wire coordinate so the drawing
///   origin maps to the bottom-left corner of the page.
/// - `rotation_deg`: 0 | 90 | 180 | 270 — rotates the entire drawing on the page.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub fn export_pdf(
    wires: &[PlotWire],
    hatches: &[HatchModel],
    wipeouts: &[HatchModel],
    paper_w: f64,
    paper_h: f64,
    offset_x: f64,
    offset_y: f64,
    rotation_deg: i32,
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    path: &Path,
    plot_style: Option<&PlotStyleTable>,
    options: PdfPlotOptions,
) -> Result<(), String> {
    let bytes = build_pdf(
        wires,
        hatches,
        wipeouts,
        paper_w as f32,
        paper_h as f32,
        offset_x,
        offset_y,
        rotation_deg,
        scale,
        clip,
        plot_style,
        options,
    );
    write_pdf_atomically(path, &bytes)
}

/// Export several independently sized pages into one PDF file.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_pdf_pages(
    pages: &[PdfPageInput],
    path: &Path,
    plot_style: Option<&PlotStyleTable>,
) -> Result<(), String> {
    if pages.is_empty() {
        return Err("No pages were selected.".into());
    }
    let bytes = build_pdf_pages(pages, plot_style);
    write_pdf_atomically(path, &bytes)
}

/// Write a complete PDF beside the destination, then replace it atomically.
#[cfg(not(target_arch = "wasm32"))]
fn write_pdf_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp_path = super::save_temp_path(path);
    if let Err(error) = std::fs::write(&temp_path, bytes) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to write PDF data: {error}"));
    }
    if let Err(error) = super::replace_save_file(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to replace PDF file: {error}"));
    }
    Ok(())
}

/// Show a parented PDF save-file dialog and return the chosen path.
///
/// The parent comes from `iced::window::run`, keeping the portal request tied
/// to the visible app window on Wayland instead of silently resolving to
/// `None` on desktops that reject a parentless save dialog (#537).
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_pdf_path_owned(
    stem: String,
    parent: &dyn iced::window::Window,
) -> Option<std::path::PathBuf> {
    let path = crate::sys::blocking_file_dialog()
        .set_parent(parent)
        .set_title(crate::t!("Export as PDF").as_ref())
        .set_file_name(&format!("{stem}.pdf"))
        .add_filter(crate::t!("PDF Files").as_ref(), &["pdf"])
        .add_filter(crate::t!("All Files").as_ref(), &["*"])
        .save_file()?;
    crate::config::remember_dialog_dir(&path);
    Some(path)
}

// ── PDF builder ───────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn build_pdf(
    wires: &[PlotWire],
    hatches: &[HatchModel],
    wipeouts: &[HatchModel],
    paper_w: f32,
    paper_h: f32,
    // Absolute-world offsets, kept in f64: at UTM the drawing sits at ~5e5/4.5e6
    // where an f32 has ~0.03 m / ~0.5 m of resolution, so an f32 offset is itself
    // already quantised before it can cancel the coordinate it is meant to cancel.
    ox: f64,
    oy: f64,
    rotation_deg: i32,
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    plot_style: Option<&PlotStyleTable>,
    options: PdfPlotOptions,
) -> Vec<u8> {
    let mut doc = PdfDocument::new("Open CAD Studio Export");
    append_pdf_page(
        &mut doc,
        &PlotPage {
            wires,
            hatches,
            wipeouts,
            paper_w,
            paper_h,
            offset_x: ox,
            offset_y: oy,
            rotation_deg,
            scale,
            clip,
            plot_style,
            options,
        },
    );
    let mut warnings = Vec::new();
    doc.save(&PdfSaveOptions::default(), &mut warnings)
}

#[cfg(not(target_arch = "wasm32"))]
fn build_pdf_pages(pages: &[PdfPageInput], plot_style: Option<&PlotStyleTable>) -> Vec<u8> {
    let mut doc = PdfDocument::new("Open CAD Studio Export");
    for page in pages {
        append_pdf_page(&mut doc, &page.as_plot_page(plot_style));
    }
    let mut warnings = Vec::new();
    doc.save(&PdfSaveOptions::default(), &mut warnings)
}

/// Run the shared emitter for one page and append the result to `doc`.
#[cfg(not(target_arch = "wasm32"))]
fn append_pdf_page(doc: &mut PdfDocument, page: &PlotPage<'_>) {
    let ops = page_ops(doc, page, &PlotAssets::default());
    doc.pages
        .push(PdfPage::new(Mm(page.paper_w), Mm(page.paper_h), ops));
}

/// The `Op`s of one page: the shared traversal, translated by `PdfSink`.
#[cfg(not(target_arch = "wasm32"))]
fn page_ops(doc: &mut PdfDocument, page: &PlotPage<'_>, assets: &PlotAssets) -> Vec<Op> {
    let mut sink = PdfSink::new(doc);
    // The PDF backend has always published a page whose text the atlas could
    // not supply; the report says so, and turning that into a refusal is a
    // change to PDF behaviour, not to the SVG work that added the report.
    match emit_plot_content(page, assets, &mut sink) {
        Ok(_report) => {}
        Err(never) => match never {},
    }
    sink.finish()
}

// ── PlotSink → printpdf ───────────────────────────────────────────────────

/// Translates the emitter's operation stream into `printpdf::Op`s, one to one
/// (`BuiltinText` expands to the text-section ops the stamp always used).
///
/// Points arrive in PDF points already — `plot_emit` converts sheet mm with
/// printpdf's own `Mm → Pt` factor — so the sink wraps them in `Pt` and the
/// result is bit-identical to what `Point::new(Mm(..))` produced. The two
/// graphics states `merge_lines` needs are registered on the document the
/// first time a `Blend` is requested, multiply before normal, the order the
/// exporter registered them in.
#[cfg(not(target_arch = "wasm32"))]
pub struct PdfSink<'d> {
    doc: &'d mut PdfDocument,
    ops: Vec<Op>,
    blend_states: Option<(ExtendedGraphicsStateId, ExtendedGraphicsStateId)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'d> PdfSink<'d> {
    pub fn new(doc: &'d mut PdfDocument) -> Self {
        Self {
            doc,
            ops: Vec::new(),
            blend_states: None,
        }
    }

    /// The collected ops, in emission order.
    pub fn finish(self) -> Vec<Op> {
        self.ops
    }

    fn blend_state(&mut self, blend: PlotBlend) -> ExtendedGraphicsStateId {
        let (multiply, normal) = self.blend_states.get_or_insert_with(|| {
            let multiply = self.doc.add_graphics_state(
                ExtendedGraphicsState::default().with_blend_mode(BlendMode::multiply()),
            );
            let normal = self.doc.add_graphics_state(
                ExtendedGraphicsState::default().with_blend_mode(BlendMode::normal()),
            );
            (multiply, normal)
        });
        match blend {
            PlotBlend::Multiply => multiply.clone(),
            PlotBlend::Normal => normal.clone(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rgb(color: [f32; 3]) -> Color {
    Color::Rgb(Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
        icc_profile: None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn line_points(points: &[PlotPoint]) -> Vec<LinePoint> {
    points
        .iter()
        .map(|p| LinePoint {
            p: Point {
                x: Pt(p.x),
                y: Pt(p.y),
            },
            bezier: false,
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn polygon(rings: Vec<Vec<PlotPoint>>, mode: PaintMode, rule: FillRule) -> Polygon {
    Polygon {
        rings: rings
            .iter()
            .map(|ring| PolygonRing {
                points: line_points(ring),
            })
            .collect(),
        mode,
        winding_order: match rule {
            FillRule::NonZero => WindingOrder::NonZero,
            FillRule::EvenOdd => WindingOrder::EvenOdd,
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PlotSink for PdfSink<'_> {
    type Error = std::convert::Infallible;

    fn emit(&mut self, op: PlotOp) -> Result<(), Self::Error> {
        match op {
            PlotOp::Save => self.ops.push(Op::SaveGraphicsState),
            PlotOp::Restore => self.ops.push(Op::RestoreGraphicsState),
            PlotOp::Concat(matrix) => self.ops.push(Op::SetTransformationMatrix {
                matrix: CurTransMat::Raw(matrix),
            }),
            PlotOp::Blend(blend) => {
                let gs = self.blend_state(blend);
                self.ops.push(Op::LoadGraphicsState { gs });
            }
            PlotOp::LineCap(cap) => self.ops.push(Op::SetLineCapStyle {
                cap: match cap {
                    LineCap::Butt => LineCapStyle::Butt,
                    LineCap::Round => LineCapStyle::Round,
                    LineCap::Square => LineCapStyle::ProjectingSquare,
                },
            }),
            PlotOp::LineJoin(join) => self.ops.push(Op::SetLineJoinStyle {
                join: match join {
                    LineJoin::Miter => LineJoinStyle::Miter,
                    LineJoin::Round => LineJoinStyle::Round,
                    LineJoin::Bevel => LineJoinStyle::Bevel,
                },
            }),
            PlotOp::StrokeColor(color) => self.ops.push(Op::SetOutlineColor { col: rgb(color) }),
            PlotOp::FillColor(color) => self.ops.push(Op::SetFillColor { col: rgb(color) }),
            PlotOp::StrokeWidthPt(pt) => self.ops.push(Op::SetOutlineThickness { pt: Pt(pt) }),
            PlotOp::Dash { lengths, phase } => self.ops.push(Op::SetLineDashPattern {
                dash: LineDashPattern::from_array(&lengths, phase),
            }),
            PlotOp::FillRect {
                x,
                y,
                width,
                height,
            } => self.ops.push(Op::DrawRectangle {
                rectangle: Rect {
                    x: Pt(x),
                    y: Pt(y),
                    width: Pt(width),
                    height: Pt(height),
                    mode: None,
                    winding_order: None,
                },
            }),
            PlotOp::Stroke { points, closed } => self.ops.push(Op::DrawLine {
                line: Line {
                    points: line_points(&points),
                    is_closed: closed,
                },
            }),
            PlotOp::Fill { rings, rule } => self.ops.push(Op::DrawPolygon {
                polygon: polygon(rings, PaintMode::Fill, rule),
            }),
            // PDF has no mesh primitive, and the exporter always drew glyph and
            // wire fills a triangle at a time — so expand the batch in order.
            PlotOp::FillMesh { tris } => {
                self.ops.extend(tris.into_iter().map(|tri| Op::DrawPolygon {
                    polygon: polygon(vec![tri.to_vec()], PaintMode::Fill, FillRule::NonZero),
                }))
            }
            PlotOp::Clip { rings, rule } => self.ops.push(Op::DrawPolygon {
                polygon: polygon(rings, PaintMode::Clip, rule),
            }),
            PlotOp::BuiltinText {
                face,
                size_pt,
                at,
                color,
                text,
            } => self.ops.extend([
                Op::StartTextSection,
                Op::SetTextCursor {
                    pos: Point {
                        x: Pt(at.x),
                        y: Pt(at.y),
                    },
                },
                Op::SetFont {
                    font: PdfFontHandle::Builtin(match face {
                        BuiltinFace::Helvetica => BuiltinFont::Helvetica,
                    }),
                    size: Pt(size_pt),
                },
                Op::SetFillColor { col: rgb(color) },
                Op::ShowText {
                    items: vec![TextItem::Text(text)],
                },
                Op::EndTextSection,
            ]),
            // A named group is structure for a backend that has it (SVG);
            // a PDF content stream has nowhere to put the name.
            PlotOp::BeginGroup { .. } | PlotOp::EndGroup => {}
        }
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod legacy_reference;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::io::plot_corpus::{corpus, text_wire, Case};
    use crate::io::plot_emit::{geometry_pt, GEOMETRY_MM_TO_PT};
    use crate::scene::WireModel;

    #[test]
    fn atomic_write_replaces_an_existing_pdf() {
        let path = std::env::temp_dir().join(format!(
            "ocs-pdf-replace-{}-{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"old").unwrap();

        write_pdf_atomically(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn clip_and_scale_emit_pdf_bytes() {
        let w = PlotWire {
            wire: WireModel::solid(
                "test".into(),
                vec![[0.0, 0.0, 0.0], [50.0, 50.0, 0.0]],
                WireModel::WHITE,
                false,
            ),
            draw_depth: 0.0,
        };
        let bytes = build_pdf(
            &[w],
            &[],
            &[],
            210.0,
            297.0,
            0.0,
            0.0,
            0,
            2.0,
            Some((10.0, 10.0, 100.0, 100.0)),
            None,
            PdfPlotOptions::default(),
        );
        // A valid PDF is produced (starts with the PDF header) and is non-trivial.
        assert!(bytes.starts_with(b"%PDF"), "not a PDF");
        assert!(bytes.len() > 200, "suspiciously small: {}", bytes.len());
    }

    // End-to-end: a page whose only content is SDF text produces a larger PDF
    // than the same page with the text stripped — proving text reaches the file.
    #[test]
    fn text_grows_the_pdf_vs_no_text() {
        let wire = text_wire("HELLO", [20.0, 20.0, 0.0]);
        let mut blank = wire.clone();
        blank.wire.text_verts.clear();

        let with_text = build_pdf(
            &[wire],
            &[],
            &[],
            210.0,
            297.0,
            0.0,
            0.0,
            0,
            1.0,
            None,
            None,
            PdfPlotOptions::default(),
        );
        let no_text = build_pdf(
            &[blank],
            &[],
            &[],
            210.0,
            297.0,
            0.0,
            0.0,
            0,
            1.0,
            None,
            None,
            PdfPlotOptions::default(),
        );
        assert!(with_text.starts_with(b"%PDF"));
        assert!(
            with_text.len() > no_text.len(),
            "text did not add content: {} !> {}",
            with_text.len(),
            no_text.len()
        );
    }

    // ── The extraction contract ───────────────────────────────────────────

    // `plot_emit` converts geometry with its own copy of printpdf's Mm → Pt
    // factor so the sink can wrap `Pt` directly. Pin it to the library, bit
    // for bit, across the magnitudes a sheet coordinate takes.
    #[test]
    fn geometry_points_match_printpdf_mm_to_pt_bit_for_bit() {
        let mut v = -1200.0_f32;
        let mut checked = 0;
        while v <= 1200.0 {
            let lib: Pt = Mm(v).into();
            assert_eq!(
                lib.0.to_bits(),
                geometry_pt(v).to_bits(),
                "Mm({v}) → Pt differs: printpdf {} vs plot_emit {}",
                lib.0,
                geometry_pt(v)
            );
            v += 0.37;
            checked += 1;
        }
        assert!(checked > 6000);
        assert_eq!(Pt::from(Mm(1.0)).0, GEOMETRY_MM_TO_PT);
    }

    // The emitter says "solid" as an empty dash array; the exporter said it as
    // `LineDashPattern::default()`. They must be the same op.
    #[test]
    fn an_empty_dash_array_is_the_default_dash_pattern() {
        assert_eq!(
            LineDashPattern::from_array(&[], 0),
            LineDashPattern::default()
        );
    }

    /// A `Vec<Op>` with the two things that legitimately differ between runs
    /// replaced by stable stand-ins: the random graphics-state names (mapped
    /// to their order of first use) and the stamp's clock/user text.
    fn normalized(ops: Vec<Op>) -> Vec<Op> {
        let mut seen: Vec<String> = Vec::new();
        ops.into_iter()
            .map(|op| match op {
                Op::LoadGraphicsState { gs } => {
                    let index = match seen.iter().position(|s| *s == gs.0) {
                        Some(i) => i,
                        None => {
                            seen.push(gs.0.clone());
                            seen.len() - 1
                        }
                    };
                    Op::LoadGraphicsState {
                        gs: ExtendedGraphicsStateId(format!("gs{index}")),
                    }
                }
                Op::ShowText { .. } => Op::ShowText {
                    items: vec![TextItem::Text("<stamp>".into())],
                },
                other => other,
            })
            .collect()
    }

    /// The page as the pre-extraction exporter drew it, frozen in
    /// `legacy_reference`.
    fn legacy_ops(case: &Case) -> Vec<Op> {
        let mut doc = PdfDocument::new("legacy");
        legacy_reference::legacy_page_ops(
            &mut doc,
            &case.wires,
            &case.hatches,
            &case.wipeouts,
            case.paper.0,
            case.paper.1,
            case.offset.0,
            case.offset.1,
            case.rotation_deg,
            case.scale,
            case.clip,
            case.plot_style.as_ref(),
            case.options,
        )
    }

    /// The same page through the shared emitter and `PdfSink`.
    fn new_ops(case: &Case) -> Vec<Op> {
        let mut doc = PdfDocument::new("new");
        page_ops(&mut doc, &case.page(), &PlotAssets::default())
    }

    /// Bit-exact text of each op. printpdf's own `PartialEq` is no use for a
    /// regression guard: `Pt` rounds to 1/1000 before comparing, and `Point`
    /// calls a zero coordinate unequal to itself (`is_normal()` is false for
    /// 0.0). Rust's float `Debug` is the shortest round-tripping form, so two
    /// ops print the same if and only if every field, floats included, is the
    /// same to the bit (`-0.0` prints as `-0.0`).
    fn exact(ops: &[Op]) -> Vec<String> {
        ops.iter().map(|op| format!("{op:?}")).collect()
    }

    /// Run `attempt` under one atlas state. The legacy and the shared emitter
    /// each read the process-wide glyph atlas on their own; a parallel test
    /// growing the atlas between those two reads re-keys every tile and the
    /// emitters no longer see the same glyphs — a difference in the *fixture*,
    /// not in the code under test. Retry until the atlas generation holds
    /// still across the whole attempt; then both emitters read the same state
    /// (missing the same stale quads symmetrically, at worst).
    fn with_stable_atlas<T>(mut attempt: impl FnMut() -> T) -> T {
        use crate::scene::text::sdf_atlas;
        for _ in 0..4 {
            let generation = sdf_atlas::generation();
            let value = attempt();
            if sdf_atlas::generation() == generation {
                return value;
            }
        }
        attempt()
    }

    // The guard behind the extraction: on every case the shared emitter, run
    // through `PdfSink`, yields exactly the `Op` stream the original exporter
    // built — same ops, same order, same floats to the bit.
    #[test]
    fn emitter_through_pdf_sink_matches_the_frozen_exporter_op_for_op() {
        let cases = corpus();
        assert!(cases.len() >= 20, "corpus shrank to {}", cases.len());
        for case in &cases {
            let (legacy, new) = with_stable_atlas(|| {
                (
                    exact(&normalized(legacy_ops(case))),
                    exact(&normalized(new_ops(case))),
                )
            });
            assert!(
                !legacy.is_empty(),
                "{}: the frozen exporter emitted nothing",
                case.name
            );
            if legacy != new {
                let first = legacy
                    .iter()
                    .zip(new.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or(legacy.len().min(new.len()));
                panic!(
                    "{}: op streams differ (legacy {} ops, new {} ops); first difference at #{first}:\n  legacy: {}\n  new:    {}",
                    case.name,
                    legacy.len(),
                    new.len(),
                    legacy.get(first).map_or("<none>", String::as_str),
                    new.get(first).map_or("<none>", String::as_str)
                );
            }
        }
    }

    // R4. The comparison above cannot see this one: the frozen exporter had
    // the same blind spot, so it asks the stream what cap each line is
    // actually drawn with. The first group ends on a CTB butt / miter wire;
    // nothing between the groups restores the graphics state; the plain wire
    // that opens the second group is round by default and must be drawn
    // round — which takes saying so again.
    #[test]
    fn the_second_group_says_round_again_after_a_ctb_butt() {
        let case = corpus()
            .into_iter()
            .find(|c| c.name == "two groups, ctb cap across the split")
            .unwrap();
        let mut cap = LineCapStyle::Butt;
        let mut join = LineJoinStyle::Miter;
        let mut stack = Vec::new();
        let mut drawn = Vec::new();
        for op in new_ops(&case) {
            match op {
                Op::SetLineCapStyle { cap: c } => cap = c,
                Op::SetLineJoinStyle { join: j } => join = j,
                Op::SaveGraphicsState => stack.push((cap, join)),
                Op::RestoreGraphicsState => (cap, join) = stack.pop().unwrap(),
                Op::DrawLine { .. } => drawn.push((cap, join)),
                _ => {}
            }
        }
        assert_eq!(
            drawn,
            [
                (LineCapStyle::Butt, LineJoinStyle::Miter),
                (LineCapStyle::Round, LineJoinStyle::Round),
                (LineCapStyle::Butt, LineJoinStyle::Miter),
            ],
            "g1-butt, g2-round, g2-butt"
        );
    }

    // The corpus must actually reach every op kind the exporter can emit,
    // otherwise the comparison above proves less than it claims.
    #[test]
    fn the_corpus_exercises_every_op_kind() {
        let mut kinds = std::collections::HashSet::new();
        for case in corpus() {
            for op in new_ops(&case) {
                kinds.insert(std::mem::discriminant(&op));
            }
        }
        let expected: Vec<Op> = vec![
            Op::SaveGraphicsState,
            Op::RestoreGraphicsState,
            Op::SetTransformationMatrix {
                matrix: CurTransMat::Raw([1.0; 6]),
            },
            Op::LoadGraphicsState {
                gs: ExtendedGraphicsStateId("x".into()),
            },
            Op::SetLineCapStyle {
                cap: LineCapStyle::Butt,
            },
            Op::SetLineJoinStyle {
                join: LineJoinStyle::Bevel,
            },
            Op::SetOutlineColor { col: rgb([0.0; 3]) },
            Op::SetFillColor { col: rgb([0.0; 3]) },
            Op::SetOutlineThickness { pt: Pt(1.0) },
            Op::SetLineDashPattern {
                dash: LineDashPattern::default(),
            },
            Op::DrawRectangle {
                rectangle: Rect {
                    x: Pt(0.0),
                    y: Pt(0.0),
                    width: Pt(1.0),
                    height: Pt(1.0),
                    mode: None,
                    winding_order: None,
                },
            },
            Op::DrawLine {
                line: Line {
                    points: Vec::new(),
                    is_closed: false,
                },
            },
            Op::DrawPolygon {
                polygon: polygon(Vec::new(), PaintMode::Fill, FillRule::NonZero),
            },
            Op::StartTextSection,
            Op::SetTextCursor {
                pos: Point {
                    x: Pt(0.0),
                    y: Pt(0.0),
                },
            },
            Op::SetFont {
                font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
                size: Pt(6.0),
            },
            Op::ShowText { items: Vec::new() },
            Op::EndTextSection,
        ];
        for op in expected {
            assert!(
                kinds.contains(&std::mem::discriminant(&op)),
                "no case emits {op:?}"
            );
        }
    }

    // A stamp pinned through `PlotAssets` is deterministic; the live one
    // carries the clock and user, which is why `normalized` masks it.
    #[test]
    fn a_pinned_stamp_label_reaches_the_page_verbatim() {
        let mut case = Case::new("stamp");
        case.options.stamp = true;
        let mut doc = PdfDocument::new("stamp");
        let ops = page_ops(
            &mut doc,
            &case.page(),
            &PlotAssets {
                stamp_label: Some("PINNED".into()),
                ..Default::default()
            },
        );
        let shown: Vec<&Op> = ops
            .iter()
            .filter(|op| matches!(op, Op::ShowText { .. }))
            .collect();
        assert_eq!(shown.len(), 1);
        assert_eq!(
            shown[0],
            &Op::ShowText {
                items: vec![TextItem::Text("PINNED".into())],
            }
        );
        assert_eq!(
            ops.last(),
            Some(&Op::RestoreGraphicsState),
            "the stamp closes with the exporter's Restore"
        );
    }

    /// Split a saved PDF at its trailer `/ID [(..) (..)]`: the bytes before,
    /// the array itself, the bytes after.
    fn split_at_trailer_id(bytes: &[u8]) -> (&[u8], &[u8], &[u8]) {
        let start = bytes
            .windows(3)
            .position(|w| w == b"/ID")
            .expect("trailer /ID present");
        let end = start + bytes[start..].iter().position(|&c| c == b']').unwrap() + 1;
        (&bytes[..start], &bytes[start..end], &bytes[end..])
    }

    // What "PDF bytes unchanged" can mean with printpdf 0.9.1. Its "random"
    // identifiers come from a process-global xorshift counter: every `save`
    // draws two fresh strings for the trailer /ID, so two exports in one
    // process are never byte-identical — but they are identical everywhere
    // else, and the Nth export of a process is reproducible across processes.
    // (With `merge_lines` the graphics-state names, drawn from the same
    // counter, also reach the content stream; that case is covered at the op
    // level instead.)
    #[test]
    fn pdf_bytes_repeat_except_for_the_random_trailer_id() {
        let case = &corpus()[0];
        let render = || {
            build_pdf(
                &case.wires,
                &case.hatches,
                &case.wipeouts,
                case.paper.0,
                case.paper.1,
                case.offset.0,
                case.offset.1,
                case.rotation_deg,
                case.scale,
                case.clip,
                case.plot_style.as_ref(),
                case.options,
            )
        };
        let a = render();
        let b = render();
        let (a_before, a_id, a_after) = split_at_trailer_id(&a);
        let (b_before, b_id, b_after) = split_at_trailer_id(&b);
        assert_eq!(a_before, b_before, "bytes before the trailer /ID differ");
        assert_ne!(a_id, b_id, "the /ID array should be the part that moves");
        assert_eq!(a_after, b_after, "bytes after the trailer /ID differ");
    }

    // Whole-file guard, as far as the library allows it: for every case whose
    // content stream carries no graphics-state name, the frozen exporter's ops
    // and the shared emitter's ops save to the same PDF bytes apart from the
    // trailer /ID.
    #[test]
    fn saved_pdf_bytes_match_the_frozen_exporter_apart_from_the_trailer_id() {
        let mut compared = 0;
        for case in corpus()
            .iter()
            .filter(|c| !c.options.merge_lines && !c.options.stamp)
        {
            let save = |ops: Vec<Op>| {
                let mut doc = PdfDocument::new("Open CAD Studio Export");
                doc.pages
                    .push(PdfPage::new(Mm(case.paper.0), Mm(case.paper.1), ops));
                doc.save(&PdfSaveOptions::default(), &mut Vec::new())
            };
            let (legacy, new) = with_stable_atlas(|| (save(legacy_ops(case)), save(new_ops(case))));
            let (l_before, _, l_after) = split_at_trailer_id(&legacy);
            let (n_before, _, n_after) = split_at_trailer_id(&new);
            assert!(
                l_before == n_before && l_after == n_after,
                "{}: saved bytes differ beyond the trailer /ID",
                case.name
            );
            compared += 1;
        }
        assert!(compared >= 15, "only {compared} cases compared");
    }
}

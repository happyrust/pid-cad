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
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
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
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())
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
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())
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
        .set_title("Export as PDF")
        .set_file_name(&format!("{stem}.pdf"))
        .add_filter("PDF Files", &["pdf"])
        .add_filter("All Files", &["*"])
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
        append_pdf_page(
            &mut doc,
            &PlotPage {
                wires: &page.wires,
                hatches: &page.hatches,
                wipeouts: &page.wipeouts,
                paper_w: page.paper_w as f32,
                paper_h: page.paper_h as f32,
                offset_x: page.offset_x,
                offset_y: page.offset_y,
                rotation_deg: page.rotation_deg,
                scale: page.scale,
                clip: page.clip,
                plot_style: page.plot_style.as_ref().or(plot_style),
                options: page.options,
            },
        );
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
    match emit_plot_content(page, assets, &mut sink) {
        Ok(()) => {}
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
        }
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod legacy_reference;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::io::plot_emit::{geometry_pt, GEOMETRY_MM_TO_PT};
    use crate::scene::model::hatch_model::{plot_style_fill_pattern, HatchPattern};
    use crate::scene::WireModel;

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

    // Build a WireModel carrying the SDF glyph quads for `text` in the embedded
    // "txt" stroke font, laid out into the process-wide atlas emit_text reads.
    fn text_wire(text: &str, origin: [f64; 3]) -> PlotWire {
        use crate::scene::pipeline::text_gpu::push_glyph_vertices;
        use crate::scene::text::{glyph_quads::layout_glyph_quads, sdf_atlas};
        let quads = {
            let mut atlas = sdf_atlas::text_atlas().lock().unwrap();
            layout_glyph_quads(&mut atlas, 10.0, 0.0, 1.0, 0.0, 1.0, "txt", false, text)
        };
        assert!(!quads.is_empty(), "stroke glyphs laid out for {text:?}");
        let mut verts = Vec::new();
        push_glyph_vertices(&mut verts, &quads, origin, 1.0, [1.0, 0.0, 0.0, 1.0], 0.0);
        PlotWire {
            wire: WireModel {
                text_verts: verts,
                ..WireModel::solid("t".into(), Vec::new(), WireModel::WHITE, false)
            },
            draw_depth: 0.0,
        }
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

    struct Case {
        name: &'static str,
        wires: Vec<PlotWire>,
        hatches: Vec<HatchModel>,
        wipeouts: Vec<HatchModel>,
        paper: (f32, f32),
        offset: (f64, f64),
        rotation_deg: i32,
        scale: f32,
        clip: Option<(f32, f32, f32, f32)>,
        plot_style: Option<PlotStyleTable>,
        options: PdfPlotOptions,
    }

    impl Case {
        fn new(name: &'static str) -> Self {
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

        fn page(&self) -> PlotPage<'_> {
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

        fn legacy_ops(&self) -> Vec<Op> {
            let mut doc = PdfDocument::new("legacy");
            legacy_reference::legacy_page_ops(
                &mut doc,
                &self.wires,
                &self.hatches,
                &self.wipeouts,
                self.paper.0,
                self.paper.1,
                self.offset.0,
                self.offset.1,
                self.rotation_deg,
                self.scale,
                self.clip,
                self.plot_style.as_ref(),
                self.options,
            )
        }

        fn new_ops(&self) -> Vec<Op> {
            let mut doc = PdfDocument::new("new");
            page_ops(&mut doc, &self.page(), &PlotAssets::default())
        }
    }

    fn wire(name: &str, points: Vec<[f32; 3]>, color: [f32; 4], depth: f32) -> PlotWire {
        PlotWire {
            wire: WireModel::solid(name.into(), points, color, false),
            draw_depth: depth,
        }
    }

    fn hatch(
        name: &str,
        boundary: Vec<[f32; 2]>,
        pattern: HatchPattern,
        color: [f32; 4],
    ) -> HatchModel {
        HatchModel {
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

    fn square(x: f32, y: f32, size: f32) -> Vec<[f32; 2]> {
        vec![
            [x, y],
            [x + size, y],
            [x + size, y + size],
            [x, y + size],
            [x, y],
        ]
    }

    // A CTB that exercises every override the emitter reads: colour, pen,
    // screening, cap / join, and a fill style that turns solids into patterns.
    fn styled_ctb() -> PlotStyleTable {
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

    fn corpus() -> Vec<Case> {
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
        // an ACI-7 solid that must stay white; a wipeout; degenerate rings.
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

        // An empty page still gets its background and state ops.
        cases.push(Case::new("empty page"));

        cases
    }

    impl Case {
        fn clone_for(&self, name: &'static str) -> Self {
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

    /// Bit-exact text of each op. printpdf's own `PartialEq` is no use for a
    /// regression guard: `Pt` rounds to 1/1000 before comparing, and `Point`
    /// calls a zero coordinate unequal to itself (`is_normal()` is false for
    /// 0.0). Rust's float `Debug` is the shortest round-tripping form, so two
    /// ops print the same if and only if every field, floats included, is the
    /// same to the bit (`-0.0` prints as `-0.0`).
    fn exact(ops: &[Op]) -> Vec<String> {
        ops.iter().map(|op| format!("{op:?}")).collect()
    }

    // The guard behind the extraction: on every case the shared emitter, run
    // through `PdfSink`, yields exactly the `Op` stream the original exporter
    // built — same ops, same order, same floats to the bit.
    #[test]
    fn emitter_through_pdf_sink_matches_the_frozen_exporter_op_for_op() {
        let cases = corpus();
        assert!(cases.len() >= 20, "corpus shrank to {}", cases.len());
        for case in &cases {
            let legacy = exact(&normalized(case.legacy_ops()));
            let new = exact(&normalized(case.new_ops()));
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

    // The corpus must actually reach every op kind the exporter can emit,
    // otherwise the comparison above proves less than it claims.
    #[test]
    fn the_corpus_exercises_every_op_kind() {
        let mut kinds = std::collections::HashSet::new();
        for case in corpus() {
            for op in case.new_ops() {
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
            let legacy = save(case.legacy_ops());
            let new = save(case.new_ops());
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

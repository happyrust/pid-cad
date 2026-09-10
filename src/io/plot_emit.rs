// Paper-space plot emitter: one traversal, many backends.
//
// `emit_plot_content` walks a plotted page exactly the way the PDF exporter
// always has — the same two render groups, the same depth sort, the same
// colour / pen-width / dash caches, the same CTB resolution, the same recovery
// of glyph geometry from the SDF atlas — and hands the result to a `PlotSink`
// as a stream of small drawing operations instead of building `printpdf::Op`s
// directly. The PDF backend (`pdf_export::PdfSink`) is one sink; an SVG backend
// can be another, without a second copy of this file deciding what goes on the
// paper.
//
// P0 contract (docs/plans/2026-09-07-dxf-to-svg-export.md, D0): this module is
// a *mechanical* extraction of `pdf_export::append_pdf_page` and its helpers.
// Algorithms, tolerances, branches, state caches and emission order are the
// ones the PDF exporter had; `pdf_export::legacy_reference` keeps the original
// verbatim and the tests there assert the two produce identical `Op` streams.
// A behaviour change to the traversal is a separate commit that moves both.
//
// Coordinates. `PlotPoint` is in PDF points, because every backend-independent
// quantity in the old exporter already was (pen widths, dash lengths, the CTM
// translation, the clip rectangle). Geometry that used to go through
// `printpdf::Point::new(Mm(..))` is converted with `GEOMETRY_MM_TO_PT`, which
// is printpdf's own `From<Mm> for Pt` factor, so the PDF sink builds `Pt`
// values directly and lands on the same bits. `MM_TO_PT` (2.834645) is the
// factor the exporter used everywhere else; the two differ in the seventh
// digit, and unifying them is a behaviour change, not part of the extraction.
// The page origin is bottom-left with Y up, as in PDF and CAD; a backend with
// another convention applies its own transform on top.

use crate::io::plot_style::PlotStyleTable;
use crate::io::plot_types::{PdfPlotOptions, PlotWire};
use crate::scene::model::hatch_model::{HatchModel, HatchPattern};
use crate::scene::WireModel;

/// mm to PDF points (1 mm = 2.834645 pt) for pen widths, dashes, the CTM
/// translation and the clip rectangle — the constant the exporter always used
/// for those.
pub const MM_TO_PT: f32 = 2.834645;
/// mm to PDF points for *geometry*: printpdf's `From<Mm> for Pt` factor, which
/// every coordinate built with `Point::new(Mm(..))` went through. Kept separate
/// from `MM_TO_PT` so the PDF sink reproduces the exporter bit for bit;
/// `pdf_export::tests` pins it against the library.
pub const GEOMETRY_MM_TO_PT: f32 = 2.834_646;
/// `wire.line_weight_px` is the on-screen pixel weight. Convert the 96-dpi
/// pixels to points while retaining the viewport's lineweight visibility
/// boost, so "As displayed" output has the same visual hierarchy as the
/// canvas instead of making 0.35 mm ByLayer outlines look half as thick.
const LW_PX_TO_PT: f32 = MM_TO_PT / (96.0 / 25.4);
const SCREEN_DOT_MM: f32 = 25.4 / 96.0;

/// Sheet-mm geometry coordinate → points, the way `Point::new(Mm(v))` did it.
#[inline]
pub fn geometry_pt(mm: f32) -> f32 {
    mm * GEOMETRY_MM_TO_PT
}

/// A point on the page, in PDF points, origin bottom-left, Y up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlotPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    /// PDF "projecting square".
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlotBlend {
    Normal,
    Multiply,
}

/// A device font a backend may have built in. Not glyph geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFace {
    Helvetica,
}

/// One drawing operation. Coordinates and lengths are in PDF points.
///
/// The variants map one-to-one onto what the PDF exporter emitted; state ops
/// (`Save` / `Restore` / colours / pen / dash / cap / join / `Blend` /
/// `Concat`) follow PDF graphics-state semantics — they persist until changed
/// or until the enclosing `Restore`.
#[derive(Clone, Debug, PartialEq)]
pub enum PlotOp {
    Save,
    Restore,
    /// Concatenate the matrix `[a b c d e f]` (translation in points) onto the
    /// current transform.
    Concat([f32; 6]),
    Blend(PlotBlend),
    LineCap(LineCap),
    LineJoin(LineJoin),
    StrokeColor([f32; 3]),
    FillColor([f32; 3]),
    StrokeWidthPt(f32),
    /// Dash run lengths in whole points (`[]` = solid) and the phase offset.
    Dash {
        lengths: Vec<i64>,
        phase: i64,
    },
    /// Axis-aligned rectangle filled with the current fill colour.
    FillRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// Stroke an open (or closed) polyline with the current pen.
    Stroke {
        points: Vec<PlotPoint>,
        closed: bool,
    },
    /// Fill one or more rings with the current fill colour.
    Fill {
        rings: Vec<Vec<PlotPoint>>,
        rule: FillRule,
    },
    /// A triangle mesh that is semantically *one* filled shape — a glyph, or a
    /// wire's solid fill. The triangles tile the shape, so a backend that can
    /// draw them as a single path should, and one that cannot expands them in
    /// order into one non-zero fill per triangle (what `PdfSink` does, which is
    /// what the exporter always emitted). The batch exists because filling the
    /// triangles separately leaves anti-aliasing seams along the shared edges
    /// in renderers that composite per shape (R5 of the SVG plan).
    FillMesh {
        tris: Vec<[PlotPoint; 3]>,
    },
    /// Intersect the clip region with these rings.
    Clip {
        rings: Vec<Vec<PlotPoint>>,
        rule: FillRule,
    },
    /// Device text in a built-in face — the plot stamp. A backend that cannot
    /// honour the face must refuse it rather than approximate.
    BuiltinText {
        face: BuiltinFace,
        size_pt: f32,
        at: PlotPoint,
        color: [f32; 3],
        text: String,
    },
    /// Everything up to the matching `EndGroup` is one thing on the sheet
    /// — a P&ID symbol and the tag lettered beside it ([`PlotGroup`]) — for
    /// a backend that can hang a name on a group of elements (SVG wraps them
    /// in one `<g tagName="…">`). Structure, not graphics state: nothing
    /// about the pen changes, and the Save / Restore pairs the drawing
    /// inside opens are closed inside. A backend with nowhere to put the
    /// name (PDF) ignores both.
    BeginGroup {
        tag: String,
    },
    EndGroup,
}

/// Entities that plot as one named group.
///
/// The name is a P&ID tag (`BUV-3101`, `XV-0407A`) and the members are the
/// wires drawn for the symbol that carries it and for its tag lettering, by
/// [`WireModel::name`] — the entity handle in decimal, which is the name the
/// scene gives every entity wire, a block reference's expansion included.
/// `pid_legend::plot_groups` builds these from a recognition, disjoint; a
/// wire that two groups do name draws with the later of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlotGroup {
    pub tag: String,
    pub members: Vec<String>,
}

/// Receives the operation stream of one page.
pub trait PlotSink {
    type Error;
    fn emit(&mut self, op: PlotOp) -> Result<(), Self::Error>;
}

/// Sink that keeps every operation — for tests and for structural checks.
#[derive(Debug, Default)]
pub struct RecordingSink {
    pub ops: Vec<PlotOp>,
}

impl PlotSink for RecordingSink {
    type Error = std::convert::Infallible;

    fn emit(&mut self, op: PlotOp) -> Result<(), Self::Error> {
        self.ops.push(op);
        Ok(())
    }
}

/// One page's inputs, borrowed. `paper_w` / `paper_h` are the effective page
/// size in mm, already swapped for 90° / 270° by the caller. `offset_x` /
/// `offset_y` are added to every world coordinate so the drawing origin lands
/// on the page origin; they stay f64 because at UTM scale an f32 offset is
/// itself quantised before it can cancel the coordinate it is meant to cancel.
#[derive(Clone, Copy)]
pub struct PlotPage<'a> {
    pub wires: &'a [PlotWire],
    pub hatches: &'a [HatchModel],
    pub wipeouts: &'a [HatchModel],
    pub paper_w: f32,
    pub paper_h: f32,
    pub offset_x: f64,
    pub offset_y: f64,
    /// 0 | 90 | 180 | 270 — rotates the entire drawing on the page.
    pub rotation_deg: i32,
    pub scale: f32,
    pub clip: Option<(f32, f32, f32, f32)>,
    pub plot_style: Option<&'a PlotStyleTable>,
    pub options: PdfPlotOptions,
}

/// The baked glyph geometry a page's text needs, lifted out of the
/// process-wide atlas.
///
/// `wire.text_verts` carries glyph *quads*, keyed by the atlas tile each was
/// baked into; the outlines live in the atlas. A key is only valid until the
/// atlas grows and re-scales its tiles, so a page that lays out its text and
/// then plots it while something else bakes a glyph looks up keys that no
/// longer exist — and the exporter's answer to that has always been to skip
/// the quad without a word (R1). Taking one snapshot and plotting from it
/// closes that window: whatever it holds is what the page draws.
#[derive(Clone, Debug)]
pub struct GlyphSnapshot {
    table: std::collections::HashMap<u64, crate::scene::text::sdf_atlas::GlyphExport>,
    solid_key: u64,
}

impl GlyphSnapshot {
    /// Snapshot the process-wide atlas. `None` when its lock is poisoned —
    /// the state in which the exporter has always drawn no text at all.
    pub fn capture() -> Option<Self> {
        let atlas = crate::scene::text::sdf_atlas::text_atlas().lock().ok()?;
        Some(Self::of(&atlas))
    }

    /// Snapshot an atlas the caller is already holding.
    ///
    /// Laying text out and taking the snapshot under the same lock is the one
    /// way to be certain the keys the quads carry are keys the snapshot has:
    /// nothing can bake, grow or rewind the atlas in between. `capture` is
    /// this with a lock of its own.
    pub fn of(atlas: &crate::scene::text::sdf_atlas::GlyphAtlas) -> Self {
        Self {
            table: atlas.export_table(),
            solid_key: crate::scene::text::sdf_atlas::uv_key(atlas.solid_uv()),
        }
    }

    pub fn glyphs(&self) -> usize {
        self.table.len()
    }

    /// The visible glyph quads on `wires` this snapshot has no geometry for —
    /// exactly the quads `emit_text` would have to skip.
    ///
    /// Zero means the text and the snapshot come from the same atlas state.
    /// Anything else means the atlas was re-scaled between laying the text
    /// out and taking the snapshot, and the text has to be laid out again
    /// before a page drawn from it can be whole.
    pub fn missing_in<'a>(&self, wires: impl IntoIterator<Item = &'a WireModel>) -> usize {
        use crate::scene::text::sdf_atlas::uv_key;
        wires
            .into_iter()
            .flat_map(|wire| wire.text_verts.as_chunks::<6>().0)
            // Mirrors `emit_text`: an invisible quad is never looked up, and
            // the decoration bar's solid texel is not a glyph.
            .filter(|quad| quad[0].color[3] >= 0.01)
            .filter(|quad| {
                let key = uv_key([quad[5].uv[0], quad[5].uv[1]]);
                key != self.solid_key && !self.table.contains_key(&key)
            })
            .count()
    }
}

/// Inputs the emitter would otherwise read from the environment.
#[derive(Clone, Debug, Default)]
pub struct PlotAssets {
    /// Text of the plot stamp. `None` = "Open CAD Studio | <user> | <unix
    /// seconds>", read from the clock and `USER` / `USERNAME` at emit time.
    pub stamp_label: Option<String>,
    /// Glyph geometry for this page. `None` = take one snapshot when the
    /// page's first text item is reached, and use it for the whole page.
    pub glyphs: Option<GlyphSnapshot>,
    /// The sheet's tagged symbols, each drawn as one group
    /// ([`PlotOp::BeginGroup`]). Empty = no grouping, and the page is drawn
    /// in plain depth order.
    pub groups: Vec<PlotGroup>,
}

impl PlotAssets {
    /// Assets for a job whose pages are already laid out: one glyph snapshot
    /// shared by every page, taken now, while the caller still holds the
    /// thread that laid the text out and nothing else has had the chance to
    /// bake into the atlas (R1). A job with no text on any page takes none —
    /// `export_table` re-resolves a font per family under the atlas lock, and
    /// a page without text should not pay for it.
    ///
    /// Whether the snapshot actually covers the text is a separate question,
    /// answered by [`Self::stale_glyphs`].
    pub fn for_pages(pages: &[crate::io::plot_types::PdfPageInput]) -> Self {
        let has_text = pages
            .iter()
            .any(|page| page.wires.iter().any(|wire| !wire.text_verts.is_empty()));
        Self {
            stamp_label: None,
            glyphs: has_text.then(GlyphSnapshot::capture).flatten(),
            groups: Vec::new(),
        }
    }

    /// Glyph quads on `pages` the snapshot cannot draw — the count a strict
    /// backend would refuse the job over. Zero when there is no snapshot: the
    /// emitter will take its own then, and report on that one.
    pub fn stale_glyphs(&self, pages: &[crate::io::plot_types::PdfPageInput]) -> usize {
        let Some(snapshot) = &self.glyphs else {
            return 0;
        };
        pages
            .iter()
            .map(|page| snapshot.missing_in(page.wires.iter().map(|wire| &wire.wire)))
            .sum()
    }
}

/// A glyph quad the atlas had no geometry for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingGlyph {
    /// The wire the quad belongs to, as the scene names it.
    pub wire: String,
    /// Its index among that wire's quads.
    pub quad: usize,
}

/// What the traversal could and could not put on the page.
///
/// The sink sees drawing operations, so it cannot tell a page with no text
/// from a page whose text was dropped; this is how it finds out. A backend
/// that must not hand back a quietly incomplete drawing checks it (R1).
#[derive(Clone, Debug, Default)]
pub struct PlotReport {
    /// Wires carrying text that the page drew from.
    pub text_items: usize,
    /// Glyph quads skipped because the snapshot had no geometry for them.
    pub missing_glyphs: usize,
    /// The first one, to point at.
    pub first_missing: Option<MissingGlyph>,
    /// The atlas could not be read at all, so no text was drawn.
    pub atlas_unavailable: bool,
}

impl PlotReport {
    /// Whether text was lost — the thing a strict backend refuses to publish.
    pub fn text_is_incomplete(&self) -> bool {
        self.atlas_unavailable || self.missing_glyphs > 0
    }
}

// ── Page traversal ────────────────────────────────────────────────────────

/// Emit one page. This is `append_pdf_page` minus the `PdfPage`; see the
/// module comment for what "mechanical" commits this function to.
pub fn emit_plot_content<S: PlotSink>(
    page: &PlotPage<'_>,
    assets: &PlotAssets,
    sink: &mut S,
) -> Result<PlotReport, S::Error> {
    // One snapshot for the page, taken when its first text item is reached so
    // a page without text never pays for it (`export_table` reparses a font
    // per family under the atlas lock).
    let mut glyphs = GlyphState::new(assets.glyphs.clone());
    let PlotPage {
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
    } = *page;

    // White page background.
    sink.emit(PlotOp::FillColor([1.0, 1.0, 1.0]))?;
    sink.emit(PlotOp::FillRect {
        x: 0.0,
        y: 0.0,
        width: geometry_pt(paper_w),
        height: geometry_pt(paper_h),
    })?;

    // `merge_lines` multiplies overlapping ink for the whole page; wipeouts
    // switch back to Normal for their own fill (see `emit_hatch`).
    let normal_blend = options.merge_lines;
    if options.merge_lines {
        sink.emit(PlotOp::Save)?;
        sink.emit(PlotOp::Blend(PlotBlend::Multiply))?;
    }

    // Round line caps/joins for CAD aesthetics.
    sink.emit(PlotOp::LineCap(LineCap::Round))?;
    sink.emit(PlotOp::LineJoin(LineJoin::Round))?;

    // Apply rotation/scale/clip transform if needed.
    // PDF uses mm-based coordinate system with origin at bottom-left.
    // We save state, apply a CTM (+ optional clip path), then restore after drawing.
    let needs_state = rotation_deg != 0 || (scale - 1.0).abs() > 1e-6 || clip.is_some();
    if needs_state {
        let (cos_a, sin_a, tx, ty) = match rotation_deg {
            // `paper_w`/`paper_h` are already the effective, rotation-swapped
            // page dimensions. A 90° turn maps x' = page_w - y, y' = x;
            // 270° maps x' = y, y' = page_h - x.
            90 => (0.0_f64, 1.0_f64, paper_w as f64, 0.0),
            180 => (-1.0_f64, 0.0_f64, paper_w as f64, paper_h as f64),
            270 => (0.0_f64, -1.0_f64, 0.0, paper_h as f64),
            _ => (1.0_f64, 0.0_f64, 0.0, 0.0),
        };
        let s = scale as f64;
        // PDF CTM: [a b c d e f] = [cos*s sin*s -sin*s cos*s tx ty]
        sink.emit(PlotOp::Save)?;
        // Convert mm translation to points (1 mm = 2.834645 pt). The literal is
        // an f64 here, as it was: `MM_TO_PT as f64` would not be the same value.
        let tx_pt = (tx * 2.834645) as f32;
        let ty_pt = (ty * 2.834645) as f32;
        sink.emit(PlotOp::Concat([
            (cos_a * s) as f32,
            (sin_a * s) as f32,
            (-(sin_a) * s) as f32,
            (cos_a * s) as f32,
            tx_pt,
            ty_pt,
        ]))?;
        // Clip rectangle (mm), applied in the pre-scale coordinate space so it
        // matches the wires drawn under the same CTM.
        if let Some((cx, cy, cw, ch)) = clip {
            sink.emit(PlotOp::Clip {
                rings: vec![vec![
                    PlotPoint {
                        x: cx * MM_TO_PT,
                        y: cy * MM_TO_PT,
                    },
                    PlotPoint {
                        x: (cx + cw) * MM_TO_PT,
                        y: cy * MM_TO_PT,
                    },
                    PlotPoint {
                        x: (cx + cw) * MM_TO_PT,
                        y: (cy + ch) * MM_TO_PT,
                    },
                    PlotPoint {
                        x: cx * MM_TO_PT,
                        y: (cy + ch) * MM_TO_PT,
                    },
                ]],
                rule: FillRule::NonZero,
            })?;
        }
    }

    let (first_wires, second_wires) = wires.split_at(options.group_splits.wires.min(wires.len()));
    let (first_hatches, second_hatches) =
        hatches.split_at(options.group_splits.hatches.min(hatches.len()));
    let (first_wipeouts, second_wipeouts) =
        wipeouts.split_at(options.group_splits.wipeouts.min(wipeouts.len()));
    // Cap and join are the graphics state's, and nothing between the two
    // render groups saves or restores it: the second group starts with
    // whatever the first left set. So the tracker lives outside the loop,
    // primed with the Round the prelude just emitted, and a group boundary
    // is not a reset. (Colour, pen and dash below are simply forgotten at
    // each group, which costs a redundant op at worst; a cap presumed Round
    // at the boundary let a first group that ended butt / miter draw the
    // second group's round wires butt / miter — R4 of
    // docs/plans/2026-09-08-svg-export-next-steps.md.)
    let mut last_cap = Some(LineCap::Round);
    let mut last_join = Some(LineJoin::Round);
    // Which group, if any, a wire or hatch belongs to, by name.
    let group_of: std::collections::HashMap<&str, usize> = assets
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g, group)| group.members.iter().map(move |m| (m.as_str(), g)))
        .collect();
    for (wires, hatches, wipeouts) in [
        (first_wires, first_hatches, first_wipeouts),
        (second_wires, second_hatches, second_wipeouts),
    ] {
        #[derive(Clone, Copy)]
        enum DrawItem<'a> {
            WireFill(&'a PlotWire),
            Hatch(&'a HatchModel),
            Wire(&'a PlotWire),
            Text(&'a PlotWire),
        }

        impl DrawItem<'_> {
            fn name(&self) -> &str {
                match self {
                    DrawItem::WireFill(w) | DrawItem::Wire(w) | DrawItem::Text(w) => &w.name,
                    DrawItem::Hatch(h) => &h.name,
                }
            }
        }

        let mut draw_items = Vec::with_capacity(wires.len() * 2 + hatches.len() + wipeouts.len());
        let mut sequence = 0usize;
        for wire in wires {
            if !wire.fill_tris.is_empty() {
                draw_items.push((wire.draw_depth, 0u8, sequence, DrawItem::WireFill(wire)));
                sequence += 1;
            }
            draw_items.push((wire.draw_depth, 2u8, sequence, DrawItem::Wire(wire)));
            sequence += 1;
            if !wire.text_verts.is_empty() {
                draw_items.push((wire.draw_depth, 3u8, sequence, DrawItem::Text(wire)));
                sequence += 1;
            }
        }
        for hatch in wipeouts.iter().chain(hatches.iter()) {
            draw_items.push((hatch.draw_depth, 1u8, sequence, DrawItem::Hatch(hatch)));
            sequence += 1;
        }
        draw_items.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });

        // A group's items are drawn together, in the place of its first,
        // so a sink that nests them wraps each symbol once; a symbol's own
        // items keep their order among themselves, and everything else keeps
        // the depth order above. The tag lettered beside a valve is drawn
        // with the valve rather than at its own depth — the one visible
        // effect, and only on what it overlaps. With no groups the order is
        // exactly the sort's.
        let order: Vec<(Option<usize>, usize)> = if group_of.is_empty() {
            (0..draw_items.len()).map(|i| (None, i)).collect()
        } else {
            let of: Vec<Option<usize>> = draw_items
                .iter()
                .map(|(_, _, _, item)| group_of.get(item.name()).copied())
                .collect();
            let mut members: Vec<Vec<usize>> = vec![Vec::new(); assets.groups.len()];
            for (i, g) in of.iter().enumerate() {
                if let Some(g) = g {
                    members[*g].push(i);
                }
            }
            let mut order = Vec::with_capacity(draw_items.len());
            for (i, g) in of.iter().enumerate() {
                match g {
                    None => order.push((None, i)),
                    Some(g) => order.extend(
                        std::mem::take(&mut members[*g])
                            .into_iter()
                            .map(|j| (Some(*g), j)),
                    ),
                }
            }
            order
        };

        let mut last_color: Option<[f32; 3]> = None;
        let mut last_lw: Option<f32> = None;
        // Current dash array (empty = solid). Tracked so the dash op is only
        // re-emitted when it actually changes between wires.
        let mut last_dash: Option<Vec<i64>> = None;
        let mut open_group: Option<usize> = None;

        for (group, index) in order {
            if group != open_group {
                if open_group.is_some() {
                    sink.emit(PlotOp::EndGroup)?;
                }
                if let Some(g) = group {
                    sink.emit(PlotOp::BeginGroup {
                        tag: assets.groups[g].tag.clone(),
                    })?;
                }
                open_group = group;
            }
            let item = draw_items[index].3;
            let wire = match item {
                DrawItem::WireFill(wire) => {
                    emit_wire_fills(
                        sink,
                        std::slice::from_ref(&wire.wire),
                        ox,
                        oy,
                        plot_style,
                        scale,
                        options,
                        normal_blend,
                    )?;
                    last_color = None;
                    last_lw = None;
                    last_dash = None;
                    continue;
                }
                DrawItem::Hatch(hatch) => {
                    emit_hatch(
                        sink,
                        hatch,
                        ox,
                        oy,
                        plot_style,
                        scale,
                        options,
                        normal_blend,
                    )?;
                    last_color = None;
                    last_lw = None;
                    last_dash = None;
                    continue;
                }
                DrawItem::Text(wire) => {
                    emit_text(
                        sink,
                        std::slice::from_ref(&wire.wire),
                        ox,
                        oy,
                        scale,
                        plot_style,
                        options,
                        &mut glyphs,
                    )?;
                    last_color = None;
                    last_lw = None;
                    last_dash = None;
                    continue;
                }
                DrawItem::Wire(wire) => wire,
            };
            let [mut r, mut g, mut b, a] = wire.color;
            if a < 0.01 {
                continue;
            }
            // Skip screen-only paper helpers. The page supplies its own white
            // boundary, and the printable-area rectangle is a UI guide, not ink.
            if matches!(
                wire.name.as_str(),
                "__paper_boundary__" | "paper_printable_area"
            ) {
                continue;
            }
            // Apply CTB plot style table overrides (color + lineweight).
            let mut lw_override: Option<f32> = None;
            let mut screening = 1.0;
            let mut color_overridden = false;
            let mut cap = None;
            let mut join = None;
            if let Some(ctb) = plot_style {
                if wire.aci > 0 {
                    if let Some([cr, cg, cb]) = ctb.resolve_color(wire.aci) {
                        r = cr;
                        g = cg;
                        b = cb;
                        color_overridden = true;
                    }
                    lw_override = ctb
                        .resolve_lineweight(wire.aci)
                        .map(|mm| (mm * MM_TO_PT).max(0.1));
                    screening = ctb.resolve_screening(wire.aci);
                    if let Some(entry) = ctb.aci_entries.get(wire.aci as usize) {
                        cap = match entry.end_style {
                            0 => Some(LineCap::Butt),
                            1 | 3 => Some(LineCap::Square),
                            2 => Some(LineCap::Round),
                            _ => None,
                        };
                        join = match entry.join_style {
                            0 => Some(LineJoin::Miter),
                            1 | 3 => Some(LineJoin::Bevel),
                            2 => Some(LineJoin::Round),
                            _ => None,
                        };
                    }
                }
            }
            // Near-white and near-yellow (viewport active border) → dark grey for print
            // (only when no CTB override was applied).
            if !color_overridden {
                let is_light = r > 0.80 && g > 0.80 && b > 0.80;
                let is_yellow = r > 0.80 && g > 0.70 && b < 0.30;
                let is_cyan = r < 0.30 && g > 0.70 && b > 0.70;
                if is_light || is_yellow {
                    r = 0.0;
                    g = 0.0;
                    b = 0.0;
                } else if is_cyan {
                    // Viewport border: print as dark blue.
                    r = 0.0;
                    g = 0.15;
                    b = 0.50;
                }
            }
            [r, g, b] = plotted_color([r, g, b], a, screening, options);

            let cap = cap.unwrap_or(LineCap::Round);
            if last_cap != Some(cap) {
                sink.emit(PlotOp::LineCap(cap))?;
                last_cap = Some(cap);
            }
            let join = join.unwrap_or(LineJoin::Round);
            if last_join != Some(join) {
                sink.emit(PlotOp::LineJoin(join))?;
                last_join = Some(join);
            }

            if last_color
                .map(|c| {
                    (c[0] - r).abs() > 0.01 || (c[1] - g).abs() > 0.01 || (c[2] - b).abs() > 0.01
                })
                .unwrap_or(true)
            {
                sink.emit(PlotOp::StrokeColor([r, g, b]))?;
                sink.emit(PlotOp::FillColor([r, g, b]))?;
                last_color = Some([r, g, b]);
            }

            // A wide polyline whose width VARIES along it — the flow arrow on
            // a P&ID is one: a two-vertex polyline from width 0 to width w —
            // carries a width per point in `taper_widths`, and `world_width`
            // is only the widest of them. Stroking the centre-line at that
            // weight (the constant-band path below) prints the arrow as a
            // 3 mm pill with round caps where the drawing has a triangle.
            // Fill the band the widths describe instead, one quad per
            // segment with the joints mitred. The linetype is not applied to
            // it; a dashed taper is not something the drawings have.
            if wire.world_width > 0.0
                && !wire.taper_widths.is_empty()
                && wire.taper_widths.len() == wire.points.len()
            {
                let rings =
                    tapered_band_rings(wire, ox, oy, paper_h as f64 / scale.max(1e-6) as f64);
                if !rings.is_empty() {
                    sink.emit(PlotOp::Fill {
                        rings,
                        rule: FillRule::NonZero,
                    })?;
                }
                continue;
            }

            // Line weight: style override or object weight. Normal output divides
            // by the page transform so physical pen widths stay constant; the
            // scale-lineweights option deliberately keeps the transformed width.
            //
            // A wide polyline is the exception: its band is a geometric width in
            // drawing units, so it must SCALE with the plot (no `/ scale`). Stroke
            // the centre-line at `world_width`, converted mm → pt exactly like the
            // geometry coordinates so the CTM scale renders the band at its true
            // size; the linetype dash pattern below then strokes it dashed. This
            // replaces the model-space hatch band that the shader-band change
            // dropped, and overrides any CTB pen weight (the width is geometry, not
            // a lineweight).
            let pen_divisor = if options.scale_lineweights {
                1.0
            } else {
                scale.max(1e-6)
            };
            let lw_pt = if wire.world_width > 0.0 {
                wire.world_width * MM_TO_PT
            } else {
                let physical = if options.object_lineweights {
                    lw_override.unwrap_or_else(|| (wire.line_weight_px * LW_PX_TO_PT).max(0.1))
                } else {
                    0.1
                };
                physical / pen_divisor
            };
            if last_lw.map(|l| (l - lw_pt).abs() > 0.01).unwrap_or(true) {
                sink.emit(PlotOp::StrokeWidthPt(lw_pt))?;
                last_lw = Some(lw_pt);
            }

            // Linetype dash pattern. Without this every wire exported as a solid
            // line regardless of its linetype (dashed / centre / dash-dot). (#155)
            let dash_arr = dash_array_from_pattern(wire.pattern_length, &wire.pattern, MM_TO_PT);
            let stationed = !dash_arr.is_empty() && wire.pattern_stations.len() > wire.points.len();
            if stationed {
                if last_dash.as_ref().is_none_or(|dash| !dash.is_empty()) {
                    sink.emit(PlotOp::Dash {
                        lengths: Vec::new(),
                        phase: 0,
                    })?;
                    last_dash = Some(Vec::new());
                }
                for index in 0..wire.points.len().saturating_sub(1) {
                    if !wire.points[index][0].is_finite() || !wire.points[index + 1][0].is_finite()
                    {
                        continue;
                    }
                    let start = wire.point_world(index, paper_h as f64 / scale.max(1e-6) as f64);
                    let end = wire.point_world(index + 1, paper_h as f64 / scale.max(1e-6) as f64);
                    for [from, to] in visible_station_ranges(
                        wire.pattern_stations[index],
                        wire.pattern_stations[index + 1],
                        wire.pattern_length,
                        &wire.pattern,
                    ) {
                        let point = |t: f32| PlotPoint {
                            x: geometry_pt((start.x + (end.x - start.x) * t as f64 + ox) as f32),
                            y: geometry_pt((start.y + (end.y - start.y) * t as f64 + oy) as f32),
                        };
                        flush_line(sink, &[point(from), point(to)], None)?;
                    }
                }
                continue;
            }
            if last_dash.as_deref() != Some(dash_arr.as_slice()) {
                sink.emit(PlotOp::Dash {
                    lengths: dash_arr.clone(),
                    phase: 0,
                })?;
                last_dash = Some(dash_arr.clone());
            }

            // Emit segments (NaN = pen-up). Points are the "high" half of a
            // double-single pair; fold in the `points_low` residual and cancel the
            // offset in f64 before narrowing. Dropping the residual (or narrowing
            // first) snaps a UTM drawing onto the f32 grid — ~3 cm across, ~50 cm
            // along northing — which is exactly the distortion the plot showed while
            // low-coordinate drawings came out clean. The result is a sheet-mm value
            // in single digits, so f32 is lossless from here.
            let mut segment: Vec<PlotPoint> = Vec::new();
            let dot_radius = (wire.name == "viewport_hatch_pattern")
                .then_some(SCREEN_DOT_MM * MM_TO_PT / (2.0 * scale.max(1e-6)));
            for (pi, &[x, y, _z]) in wire.points.iter().enumerate() {
                if x.is_nan() || y.is_nan() {
                    flush_line(sink, &segment, dot_radius)?;
                    segment.clear();
                } else {
                    let point = wire.point_world(pi, paper_h as f64 / scale.max(1e-6) as f64);
                    let wx = (point.x + ox) as f32;
                    let wy = (point.y + oy) as f32;
                    segment.push(PlotPoint {
                        x: geometry_pt(wx),
                        y: geometry_pt(wy),
                    });
                }
            }
            flush_line(sink, &segment, dot_radius)?;
        }
        if open_group.is_some() {
            sink.emit(PlotOp::EndGroup)?;
        }
    }

    if needs_state {
        sink.emit(PlotOp::Restore)?;
    }
    if options.merge_lines {
        sink.emit(PlotOp::Restore)?;
    }
    if options.stamp {
        emit_plot_stamp(sink, assets)?;
    }
    Ok(glyphs.report)
}

/// The page's glyph snapshot and what it could not supply.
struct GlyphState {
    snapshot: Option<GlyphSnapshot>,
    /// The snapshot was asked for and could not be taken.
    tried: bool,
    report: PlotReport,
}

impl GlyphState {
    fn new(snapshot: Option<GlyphSnapshot>) -> Self {
        Self {
            tried: snapshot.is_some(),
            snapshot,
            report: PlotReport::default(),
        }
    }

    /// The page's snapshot, taken now if this is its first text item, with
    /// the report to record what it cannot supply.
    fn ensure(&mut self) -> Option<(&GlyphSnapshot, &mut PlotReport)> {
        if !self.tried {
            self.tried = true;
            self.snapshot = GlyphSnapshot::capture();
            if self.snapshot.is_none() {
                self.report.atlas_unavailable = true;
            }
        }
        let snapshot = self.snapshot.as_ref()?;
        Some((snapshot, &mut self.report))
    }
}

// ── Helpers shared by the passes ──────────────────────────────────────────

/// Build a dash array (in points) from a WireModel linetype pattern.
///
/// `pattern` holds the linetype run lengths in paper-mm: positive = dash,
/// negative = gap, exactly 0 = a dot, and trailing zeros are padding — so the
/// real length is the index of the last non-zero element + 1 (same convention
/// the wire shader uses). Returns an empty vec for a solid line. printpdf's
/// `LineDashPattern` holds at most six entries, so longer patterns are
/// truncated to three dash/gap pairs.
fn dash_array_from_pattern(pattern_length: f32, pattern: &[f32; 8], mm_to_pt: f32) -> Vec<i64> {
    if pattern_length <= 1e-6 {
        return Vec::new();
    }
    let count = match pattern.iter().rposition(|&v| v != 0.0) {
        Some(i) => (i + 1).min(6),
        None => return Vec::new(),
    };
    pattern[..count]
        .iter()
        // Round to whole points (printpdf dash entries are integers) and keep a
        // 1 pt floor so a zero-length dot still prints as a short mark.
        .map(|&v| (((v.abs() * mm_to_pt).round()) as i64).max(1))
        .collect()
}

fn visible_station_ranges(
    start: f32,
    end: f32,
    pattern_length: f32,
    pattern: &[f32; 8],
) -> Vec<[f32; 2]> {
    let count = pattern
        .iter()
        .rposition(|value| *value != 0.0)
        .map_or(0, |i| i + 1);
    if count == 0 || pattern_length <= 1e-6 {
        return vec![[0.0, 1.0]];
    }
    let dot = 1.0 / MM_TO_PT;
    let mut elements: Vec<(f32, bool)> = pattern[..count]
        .iter()
        .map(|value| (if *value == 0.0 { dot } else { value.abs() }, *value >= 0.0))
        .collect();
    let total: f32 = elements.iter().map(|(length, _)| *length).sum();
    if total <= 1e-6 {
        return vec![[0.0, 1.0]];
    }
    let factor = pattern_length / total;
    for (length, _) in &mut elements {
        *length *= factor;
    }
    let delta = end - start;
    let state = |station: f32, forward: bool| {
        let mut phase = station.rem_euclid(pattern_length);
        if !forward && phase <= 1e-6 {
            phase = pattern_length;
        }
        let mut offset = 0.0;
        if forward {
            for &(length, drawn) in &elements {
                let end = offset + length;
                if phase < end - 1e-6 {
                    return (drawn, end - phase);
                }
                offset = end;
            }
            (elements[0].1, elements[0].0)
        } else {
            offset = pattern_length;
            for &(length, drawn) in elements.iter().rev() {
                offset -= length;
                if phase > offset + 1e-6 {
                    return (drawn, phase - offset);
                }
            }
            let last = elements[elements.len() - 1];
            (last.1, last.0)
        }
    };
    if delta.abs() <= 1e-6 {
        return state(start, true)
            .0
            .then_some([0.0, 1.0])
            .into_iter()
            .collect();
    }

    let mut ranges = Vec::new();
    let mut t = 0.0;
    while t < 1.0 - 1e-6 {
        let station = start + delta * t;
        let (drawn, remaining) = state(station, delta > 0.0);
        let next = (t + remaining / delta.abs()).clamp(t + 1e-6, 1.0);
        if drawn {
            ranges.push([t, next]);
        }
        t = next;
    }
    ranges
}

fn flush_line<S: PlotSink>(
    sink: &mut S,
    pts: &[PlotPoint],
    dot_radius: Option<f32>,
) -> Result<(), S::Error> {
    if pts.len() < 2 {
        return Ok(());
    }
    if let Some(radius) = dot_radius {
        let first = pts[0];
        let coincident = pts
            .iter()
            .skip(1)
            .all(|point| (point.x - first.x).abs() <= 1e-6 && (point.y - first.y).abs() <= 1e-6);
        if coincident {
            return emit_round_dot(sink, first, radius);
        }
    }
    sink.emit(PlotOp::Stroke {
        points: pts.to_vec(),
        closed: false,
    })
}

fn emit_round_dot<S: PlotSink>(
    sink: &mut S,
    center: PlotPoint,
    radius: f32,
) -> Result<(), S::Error> {
    const SIDES: usize = 12;
    let points = (0..SIDES)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / SIDES as f32;
            PlotPoint {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
            }
        })
        .collect();
    sink.emit(PlotOp::Fill {
        rings: vec![points],
        rule: FillRule::NonZero,
    })
}

/// The miter limit SVG and PDF apply to strokes (4: a joint may reach four
/// stroke widths from the centre-line), as the cosine of half the turn at
/// which a band joint stops being mitred.
const MITRE_COS_LIMIT: f64 = 0.25;

/// The band of a tapered polyline as fill rings, in points: one quad per
/// segment, its corners the two endpoints pushed out by half their own width
/// along the segment's normal. Where two segments meet, the corner is the
/// mitre of the two offsets, so neighbouring quads share an edge and the
/// band shows no notch on the outside of a bend and no seam on the inside; a
/// bend too sharp for a mitre falls back to a bevel with a wedge over the
/// gap. `NaN` points split the run, as they do for a stroke, and a run that
/// returns to its first point is joined there too. Every ring is wound the
/// same way, so a non-zero fill of all of them together is their union.
fn tapered_band_rings(wire: &WireModel, ox: f64, oy: f64, view_height: f64) -> Vec<Vec<PlotPoint>> {
    let mut rings = Vec::new();
    let mut run: Vec<([f64; 2], f64)> = Vec::new();
    for (index, &[x, y, _z]) in wire.points.iter().enumerate() {
        if x.is_nan() || y.is_nan() {
            band_run_rings(&run, &mut rings);
            run.clear();
            continue;
        }
        let point = wire.point_world(index, view_height);
        let p = [point.x + ox, point.y + oy];
        let w = wire.taper_widths[index].max(0.0) as f64;
        // A zero-length segment is a step in width, not a segment: keep the
        // vertex once, at the wider of the two.
        if let Some((last, last_w)) = run.last_mut() {
            if (last[0] - p[0]).abs() <= 1e-9 && (last[1] - p[1]).abs() <= 1e-9 {
                *last_w = last_w.max(w);
                continue;
            }
        }
        run.push((p, w));
    }
    band_run_rings(&run, &mut rings);
    rings
}

/// One `NaN`-free run of `tapered_band_rings`: vertices in sheet mm, each
/// with the band's full width there.
fn band_run_rings(run: &[([f64; 2], f64)], out: &mut Vec<Vec<PlotPoint>>) {
    let n = run.len();
    if n < 2 {
        return;
    }
    let same =
        |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() <= 1e-9 && (a[1] - b[1]).abs() <= 1e-9;
    let closed = n >= 4 && same(run[0].0, run[n - 1].0);
    // Unit direction of the segment leaving vertex `i`.
    let dir = |i: usize| -> [f64; 2] {
        let (a, _) = run[i];
        let (b, _) = run[i + 1];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        [dx / len, dy / len]
    };
    let left_of = |d: [f64; 2]| -> [f64; 2] { [-d[1], d[0]] };

    // The left-side offset of each vertex as the end of the segment coming
    // in and as the start of the one going out — one and the same at a
    // mitred joint; the right side is the negation.
    let mut offsets: Vec<([f64; 2], [f64; 2])> = Vec::with_capacity(n);
    let mut bevelled: Vec<usize> = Vec::new();
    for (i, &(_, width)) in run.iter().enumerate() {
        let h = width * 0.5;
        let incoming = if i > 0 {
            Some(dir(i - 1))
        } else if closed {
            Some(dir(n - 2))
        } else {
            None
        };
        let outgoing = if i + 1 < n {
            Some(dir(i))
        } else if closed {
            Some(dir(0))
        } else {
            None
        };
        let scaled = |normal: [f64; 2]| [normal[0] * h, normal[1] * h];
        offsets.push(match (incoming, outgoing) {
            (Some(d0), Some(d1)) => {
                let n0 = left_of(d0);
                let n1 = left_of(d1);
                let m = [n0[0] + n1[0], n0[1] + n1[1]];
                let len = (m[0] * m[0] + m[1] * m[1]).sqrt();
                // The cosine of half the turn; the mitre is 1/cos as long as
                // the plain offset.
                let cos_half = if len > 1e-12 {
                    (m[0] * n0[0] + m[1] * n0[1]) / len
                } else {
                    0.0
                };
                if cos_half >= MITRE_COS_LIMIT {
                    let k = h / (cos_half * len);
                    let mitre = [m[0] * k, m[1] * k];
                    (mitre, mitre)
                } else {
                    if h > 1e-9 {
                        bevelled.push(i);
                    }
                    (scaled(n0), scaled(n1))
                }
            }
            (Some(d0), None) => {
                let offset = scaled(left_of(d0));
                (offset, offset)
            }
            (None, Some(d1)) => {
                let offset = scaled(left_of(d1));
                (offset, offset)
            }
            (None, None) => ([0.0, 0.0], [0.0, 0.0]),
        });
    }

    for i in 0..n - 1 {
        let (a, _) = run[i];
        let (b, _) = run[i + 1];
        let oa = offsets[i].1;
        let ob = offsets[i + 1].0;
        push_band_ring(
            out,
            &[
                [a[0] + oa[0], a[1] + oa[1]],
                [b[0] + ob[0], b[1] + ob[1]],
                [b[0] - ob[0], b[1] - ob[1]],
                [a[0] - oa[0], a[1] - oa[1]],
            ],
        );
    }
    for i in bevelled {
        let (p, _) = run[i];
        let (o_in, o_out) = offsets[i];
        for sign in [1.0, -1.0] {
            push_band_ring(
                out,
                &[
                    p,
                    [p[0] + sign * o_in[0], p[1] + sign * o_in[1]],
                    [p[0] + sign * o_out[0], p[1] + sign * o_out[1]],
                ],
            );
        }
    }
}

/// One ring of a band in sheet mm → points, wound anticlockwise; a ring
/// with no area (both ends of a segment at width 0) is dropped.
fn push_band_ring(out: &mut Vec<Vec<PlotPoint>>, corners: &[[f64; 2]]) {
    let mut area2 = 0.0;
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % corners.len()];
        area2 += a[0] * b[1] - b[0] * a[1];
    }
    if area2.abs() <= 1e-12 {
        return;
    }
    let mut ring: Vec<PlotPoint> = corners
        .iter()
        .map(|&[x, y]| PlotPoint {
            x: geometry_pt(x as f32),
            y: geometry_pt(y as f32),
        })
        .collect();
    if area2 < 0.0 {
        ring.reverse();
    }
    out.push(ring);
}

fn plotted_color(rgb: [f32; 3], alpha: f32, screening: f32, options: PdfPlotOptions) -> [f32; 3] {
    let amount = screening.clamp(0.0, 1.0)
        * if options.transparency {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
    [
        1.0 - (1.0 - rgb[0]) * amount,
        1.0 - (1.0 - rgb[1]) * amount,
        1.0 - (1.0 - rgb[2]) * amount,
    ]
}

#[allow(clippy::too_many_arguments)]
fn emit_wire_fills<S: PlotSink>(
    sink: &mut S,
    wires: &[WireModel],
    ox: f64,
    oy: f64,
    plot_style: Option<&PlotStyleTable>,
    scale: f32,
    options: PdfPlotOptions,
    normal_blend: bool,
) -> Result<(), S::Error> {
    for wire in wires {
        if wire.fill_tris.is_empty() {
            continue;
        }
        let styled_pattern = plot_style.and_then(|table| {
            (wire.aci > 0)
                .then(|| table.aci_entries.get(wire.aci as usize))
                .flatten()
                .and_then(|entry| {
                    (65..=72)
                        .contains(&entry.fill_style)
                        .then(|| {
                            crate::scene::model::hatch_model::plot_style_fill_pattern(
                                entry.fill_style,
                            )
                        })
                        .flatten()
                })
        });
        if let Some(pattern) = styled_pattern {
            for (triangle_index, triangle) in wire.fill_tris.chunks_exact(3).enumerate() {
                let mut boundary = Vec::with_capacity(4);
                for (point_index, point) in triangle.iter().enumerate() {
                    let index = triangle_index * 3 + point_index;
                    let low = wire.fill_tris_low.get(index).copied().unwrap_or([0.0; 3]);
                    boundary.push([point[0] + low[0], point[1] + low[1]]);
                }
                boundary.push(boundary[0]);
                let hatch = HatchModel {
                    render_instance: wire.render_instance.clone(),
                    world_origin: [0.0, 0.0],
                    boundary: std::sync::Arc::new(boundary),
                    boundary_wcs: None,
                    fill_plane: None,
                    fill_plane_boundary: None,
                    boundary_exterior: None,
                    boundary_sources: None,
                    boundary_paths: None,
                    style: acadrust::entities::HatchStyleType::Normal,
                    pattern: pattern.clone(),
                    name: "PLOTSTYLE".to_string(),
                    color: wire.color,
                    aci: wire.aci,
                    line_weight_px: wire.line_weight_px,
                    angle_offset: 0.0,
                    scale: 1.0 / scale.max(1.0e-6),
                    draw_depth: wire.depth_override.unwrap_or(0.0),
                };
                emit_hatch(
                    sink,
                    &hatch,
                    ox,
                    oy,
                    plot_style,
                    scale,
                    options,
                    normal_blend,
                )?;
            }
            continue;
        }
        let [mut r, mut g, mut b, a] = wire.color;
        if a < 0.01 {
            continue;
        }
        let mut screening = 1.0;
        // An MTEXT background mask set to "use the drawing window colour"
        // (fill flag 0x02) reaches the plot in the screen's canvas colour —
        // that is what the cache's `canvas_color` records — and on paper the
        // window colour is the sheet. AutoCAD masks it with paper white, so
        // this does too, outside the CTB and the adaptation below: a dark
        // canvas colour would otherwise print as it is, and a light one would
        // be adapted to black — either way a black box behind the glyphs
        // (SP02-05's material table, the `个` cell).
        let paints_canvas = wire
            .bg_adapt
            .as_deref()
            .is_some_and(|adapt| adapt.canvas_color);
        if paints_canvas {
            [r, g, b] = [1.0, 1.0, 1.0];
        } else {
            let mut color_overridden = false;
            if let Some(table) = plot_style {
                if wire.aci > 0 {
                    if let Some(color) = table.resolve_color(wire.aci) {
                        [r, g, b] = color;
                        color_overridden = true;
                    }
                    screening = table.resolve_screening(wire.aci);
                }
            }
            if !color_overridden {
                [r, g, b] = adapt_text_color([r, g, b]);
            }
        }
        [r, g, b] = plotted_color([r, g, b], a, screening, options);
        sink.emit(PlotOp::FillColor([r, g, b]))?;
        let mut tris = Vec::with_capacity(wire.fill_tris.len() / 3);
        for (triangle_index, triangle) in wire.fill_tris.chunks_exact(3).enumerate() {
            let mut points = [PlotPoint { x: 0.0, y: 0.0 }; 3];
            for (point_index, &[x, y, _]) in triangle.iter().enumerate() {
                let index = triangle_index * 3 + point_index;
                let low = wire.fill_tris_low.get(index).copied().unwrap_or([0.0; 3]);
                points[point_index] = PlotPoint {
                    x: geometry_pt((x as f64 + low[0] as f64 + ox) as f32),
                    y: geometry_pt((y as f64 + low[1] as f64 + oy) as f32),
                };
            }
            tris.push(points);
        }
        if !tris.is_empty() {
            // Under `merge_lines` the page multiplies ink, and white times
            // anything is that thing: a mask would hide nothing. Paint it in
            // Normal like a wipeout does.
            let mask_under_multiply = paints_canvas && normal_blend;
            if mask_under_multiply {
                sink.emit(PlotOp::Save)?;
                sink.emit(PlotOp::Blend(PlotBlend::Normal))?;
            }
            sink.emit(PlotOp::FillMesh { tris })?;
            if mask_under_multiply {
                sink.emit(PlotOp::Restore)?;
            }
        }
    }
    Ok(())
}

/// "Open CAD Studio | <user> | <unix seconds>" — the stamp the exporter has
/// always printed. Reads the clock and the environment, so two exports of the
/// same page differ; `PlotAssets::stamp_label` pins it for tests.
fn live_stamp_label() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    // `SystemTime::now` panics on wasm32-unknown-unknown; the web build has no
    // PDF export to stamp, and an SVG backend refuses `BuiltinText` anyway.
    #[cfg(target_arch = "wasm32")]
    let timestamp = 0u64;
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    format!("Open CAD Studio | {user} | {timestamp}")
}

fn emit_plot_stamp<S: PlotSink>(sink: &mut S, assets: &PlotAssets) -> Result<(), S::Error> {
    let label = assets.stamp_label.clone().unwrap_or_else(live_stamp_label);
    sink.emit(PlotOp::Save)?;
    sink.emit(PlotOp::BuiltinText {
        face: BuiltinFace::Helvetica,
        size_pt: 6.0,
        at: PlotPoint {
            x: geometry_pt(4.0),
            y: geometry_pt(3.0),
        },
        color: [0.25, 0.25, 0.25],
        text: label,
    })?;
    sink.emit(PlotOp::Restore)
}

/// Emit a single hatch / wipeout as a filled (or stroked, for pattern fills)
/// polygon. NaN sentinels in `hatch.boundary` split the path into multiple
/// rings so islands and holes render correctly under the even-odd rule.
/// Mirrors `scene::paper_canvas::draw_hatch`: solid → fill, pattern → outline,
/// gradient → solid fill of the averaged colour.
#[allow(clippy::too_many_arguments)]
fn emit_hatch<S: PlotSink>(
    sink: &mut S,
    hatch: &HatchModel,
    ox: f64,
    oy: f64,
    plot_style: Option<&PlotStyleTable>,
    scale: f32,
    options: PdfPlotOptions,
    normal_blend: bool,
) -> Result<(), S::Error> {
    if hatch.boundary.is_empty() {
        return Ok(());
    }
    let mut styled_hatch = None;
    if let Some(table) = plot_style {
        if hatch.aci > 0 && matches!(hatch.pattern, HatchPattern::Solid) {
            if let Some(pattern) = table.aci_entries.get(hatch.aci as usize).and_then(|entry| {
                crate::scene::model::hatch_model::plot_style_fill_pattern(entry.fill_style)
            }) {
                let mut model = hatch.clone();
                model.pattern = pattern;
                model.scale = 1.0 / scale.max(1.0e-6);
                styled_hatch = Some(model);
            }
        }
    }
    let hatch = styled_hatch.as_ref().unwrap_or(hatch);
    let [mut r, mut g, mut b, a] = hatch.color;
    if a < 0.01 {
        return Ok(());
    }
    // Adapt hatch fills to the white sheet, mirroring the wire pass: colours
    // arrive adapted to the (dark) screen background, so a white/ACI-7 fill
    // would vanish white-on-white on paper. Force near-white/near-yellow → black
    // and near-cyan → dark blue for a readable white-sheet result.
    // Genuine colours are untouched; WIPEOUTS keep their paper-white mask.
    // An ACI-7 solid fill is no exception (it was one from 2026-08-01 to
    // 2026-09-10, upstream #618 "match the viewport"): colour 7 is the
    // foreground colour, white on the dark screen and black on paper, as
    // AutoCAD plots it -- the CPECC sheets' pipe-junction dots and valve
    // actuator blocks are ByLayer on colour-7 layers and came out as white
    // holes. A drawing that wants a paper-white mask draws a WIPEOUT.
    let is_wipeout = hatch.name == "WIPEOUT_FILL";
    let mut screening = 1.0;
    let mut lw_override = None;
    let mut color_overridden = false;
    if !is_wipeout {
        if let Some(table) = plot_style {
            if hatch.aci > 0 {
                if let Some([cr, cg, cb]) = table.resolve_color(hatch.aci) {
                    r = cr;
                    g = cg;
                    b = cb;
                    color_overridden = true;
                }
                screening = table.resolve_screening(hatch.aci);
                lw_override = table
                    .resolve_lineweight(hatch.aci)
                    .map(|mm| (mm * MM_TO_PT).max(0.1));
            }
        }
    }
    if is_wipeout {
        // Screen wipeouts match the configured canvas colour; printed
        // wipeouts must mask with the white paper colour.
        r = 1.0;
        g = 1.0;
        b = 1.0;
    } else if !color_overridden {
        let is_light = r > 0.80 && g > 0.80 && b > 0.80;
        let is_yellow = r > 0.80 && g > 0.70 && b < 0.30;
        let is_cyan = r < 0.30 && g > 0.70 && b > 0.70;
        if is_light || is_yellow {
            r = 0.0;
            g = 0.0;
            b = 0.0;
        } else if is_cyan {
            r = 0.0;
            g = 0.15;
            b = 0.50;
        }
    }
    [r, g, b] = plotted_color([r, g, b], a, screening, options);
    // `boundary` holds f32 offsets from the f64 `world_origin`, so resolve the
    // pair in f64 and only narrow once the offset has cancelled — casting
    // `world_origin` to f32 first re-introduces the ~0.5 m UTM quantisation the
    // boundary-relative encoding exists to avoid.
    let (world_ox, world_oy) = (hatch.world_origin[0], hatch.world_origin[1]);

    // Split the boundary into rings on every NaN-NaN separator.
    let mut rings: Vec<Vec<PlotPoint>> = Vec::new();
    let mut current: Vec<PlotPoint> = Vec::new();
    for &[bx, by] in hatch.boundary.iter() {
        if bx.is_nan() || by.is_nan() {
            if current.len() >= 3 {
                rings.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            continue;
        }
        let px = (bx as f64 + world_ox + ox) as f32;
        let py = (by as f64 + world_oy + oy) as f32;
        current.push(PlotPoint {
            x: geometry_pt(px),
            y: geometry_pt(py),
        });
    }
    if current.len() >= 3 {
        rings.push(current);
    }
    if rings.is_empty() {
        return Ok(());
    }

    // Solid keeps its colour; gradient averages the two stops (matching
    // paper_canvas); a pattern fill never fills, it is stroked below.
    let fill_color = match &hatch.pattern {
        HatchPattern::Solid | HatchPattern::Pattern(_) => [r, g, b],
        HatchPattern::Gradient { color2, .. } => {
            let second = if color_overridden {
                [r, g, b]
            } else {
                plotted_color(
                    adapt_text_color([color2[0], color2[1], color2[2]]),
                    color2[3],
                    screening,
                    options,
                )
            };
            [
                (r + second[0]) * 0.5,
                (g + second[1]) * 0.5,
                (b + second[2]) * 0.5,
            ]
        }
    };

    // Pattern hatches: the family lines are clipped to the boundary by
    // `pattern_segments_for_plot` and each segment is stroked. The polygon
    // outline itself is skipped — pattern hatches in real DXF do not draw
    // their boundary as part of the fill.
    if matches!(hatch.pattern, HatchPattern::Pattern(_)) {
        let physical = if options.object_lineweights {
            lw_override.unwrap_or_else(|| (hatch.line_weight_px * LW_PX_TO_PT).max(0.1))
        } else {
            0.1
        };
        let divisor = if options.scale_lineweights {
            1.0
        } else {
            scale.max(1e-6)
        };
        let segments = hatch.pattern_segments_for_plot();
        if segments.is_empty() {
            return Ok(());
        }
        sink.emit(PlotOp::StrokeColor([r, g, b]))?;
        sink.emit(PlotOp::FillColor([r, g, b]))?;
        sink.emit(PlotOp::StrokeWidthPt(physical / divisor))?;
        // Pattern dashes are already materialized by `pattern_segments`.
        // Clear any linetype left by the preceding paper/model render group.
        sink.emit(PlotOp::Dash {
            lengths: Vec::new(),
            phase: 0,
        })?;
        for [a, b_pt] in segments {
            // `pattern_segments` returns absolute world f64; cancel the offset
            // before narrowing, as everywhere else in this file.
            let (ax, ay) = ((a[0] + ox) as f32, (a[1] + oy) as f32);
            let (bx, by) = ((b_pt[0] + ox) as f32, (b_pt[1] + oy) as f32);
            let points = [
                PlotPoint {
                    x: geometry_pt(ax),
                    y: geometry_pt(ay),
                },
                PlotPoint {
                    x: geometry_pt(bx),
                    y: geometry_pt(by),
                },
            ];
            let dot_radius = SCREEN_DOT_MM * MM_TO_PT / (2.0 * scale.max(1e-6));
            flush_line(sink, &points, Some(dot_radius))?;
        }
        return Ok(());
    }

    // Solid / gradient: filled polygon path.
    sink.emit(PlotOp::FillColor(fill_color))?;
    if is_wipeout && normal_blend {
        sink.emit(PlotOp::Save)?;
        sink.emit(PlotOp::Blend(PlotBlend::Normal))?;
    }
    sink.emit(PlotOp::Fill {
        rings,
        rule: FillRule::EvenOdd,
    })?;
    if is_wipeout && normal_blend {
        sink.emit(PlotOp::Restore)?;
    }
    Ok(())
}

// ── Text (SDF glyph quads → vector strokes / fills) ────────────────────────

/// Absolute world XY of a glyph vertex (double-single high + low parts folded).
///
/// The fold must happen in f64: the pair exists because the absolute coordinate
/// does not fit an f32, so `pos + pos_low` evaluated in f32 rounds straight back
/// to `pos` and throws away the residual it was carrying.
fn glyph_world_xy(v: &crate::scene::pipeline::text_gpu::TextVertex) -> [f64; 2] {
    [
        v.pos[0] as f64 + v.pos_low[0] as f64,
        v.pos[1] as f64 + v.pos_low[1] as f64,
    ]
}

/// Adapt a text colour to the white sheet, mirroring the wire/hatch passes:
/// near-white / near-yellow (colour-7-on-white) → black, near-cyan → dark blue.
fn adapt_text_color([r, g, b]: [f32; 3]) -> [f32; 3] {
    let is_light = r > 0.80 && g > 0.80 && b > 0.80;
    let is_yellow = r > 0.80 && g > 0.70 && b < 0.30;
    let is_cyan = r < 0.30 && g > 0.70 && b > 0.70;
    if is_light || is_yellow {
        [0.0, 0.0, 0.0]
    } else if is_cyan {
        [0.0, 0.15, 0.50]
    } else {
        [r, g, b]
    }
}

/// Re-emit every wire's SDF text as vector geometry.
///
/// Each visible glyph rides on `wire.text_verts` as one 6-vertex quad (two
/// triangles) whose corners are the glyph's atlas `plane` rect run through the
/// text transform. We recover the glyph's outline / fill from the atlas by the
/// quad's `uv_min` and map it into that quad by affine interpolation of the
/// plane rect — so a stroke (LFF) font emits polylines and a filled TrueType
/// glyph emits filled triangles, exactly where the SDF quad sits.
///
/// Geometry comes from the page's [`GlyphSnapshot`] — one for the whole page,
/// so a glyph baked while the page is being written cannot move the tiles out
/// from under it. A quad the snapshot has no geometry for is still skipped,
/// as the exporter has always skipped it, but now it is *counted* and the
/// first one is named, so a backend can refuse to hand back a page that is
/// quietly missing its text (R1).
#[allow(clippy::too_many_arguments)]
fn emit_text<S: PlotSink>(
    sink: &mut S,
    wires: &[WireModel],
    ox: f64,
    oy: f64,
    scale: f32,
    plot_style: Option<&PlotStyleTable>,
    options: PdfPlotOptions,
    glyphs: &mut GlyphState,
) -> Result<(), S::Error> {
    use crate::scene::text::sdf_atlas;

    if wires.iter().all(|w| w.text_verts.is_empty()) {
        return Ok(());
    }
    let Some((snapshot, report)) = glyphs.ensure() else {
        return Ok(());
    };
    let (table, solid_key) = (&snapshot.table, snapshot.solid_key);
    report.text_items += 1;

    // The dash pattern is persistent graphics state and the wire pass above
    // only re-emits it on change, so whatever the last wire needed is still
    // active here — without this reset a drawing whose last wire carries a
    // HIDDEN/CENTER linetype prints its glyph outlines dashed.
    sink.emit(PlotOp::Dash {
        lengths: Vec::new(),
        phase: 0,
    })?;

    for wire in wires {
        let verts = &wire.text_verts;
        if verts.is_empty() {
            continue;
        }
        // Mirror the wire pass: indexed style color, screening, and pen width.
        let mut ctb_color: Option<[f32; 3]> = None;
        let mut lw_override: Option<f32> = None;
        let mut screening = 1.0;
        if let Some(ctb) = plot_style {
            if wire.aci > 0 {
                ctb_color = ctb.resolve_color(wire.aci);
                lw_override = options
                    .object_lineweights
                    .then(|| {
                        ctb.resolve_lineweight(wire.aci).map(|mm| {
                            let divisor = if options.scale_lineweights {
                                1.0
                            } else {
                                scale.max(1e-6)
                            };
                            (mm * MM_TO_PT).max(0.1) / divisor
                        })
                    })
                    .flatten();
                screening = ctb.resolve_screening(wire.aci);
            }
        }
        let mut gi = 0;
        while gi + 6 <= verts.len() {
            let quad = &verts[gi..gi + 6];
            gi += 6;

            let a = quad[0].color[3];
            if a < 0.01 {
                continue;
            }
            // A CTB colour override wins over the white-sheet adaptation, exactly
            // as in the wire pass — else a monochrome.ctb plot plots the lines
            // black and leaves the text on its screen colour.
            let rgb = ctb_color.unwrap_or_else(|| {
                adapt_text_color([quad[0].color[0], quad[0].color[1], quad[0].color[2]])
            });
            let [r, g, b] = plotted_color(rgb, a, screening, options);

            // Quad corners in world XY: verts run [bl, br, tr, bl, tr, tl].
            let bl = glyph_world_xy(&quad[0]);
            let br = glyph_world_xy(&quad[1]);
            let tr = glyph_world_xy(&quad[2]);
            let tl = glyph_world_xy(&quad[5]);
            // `tl` carries uv = (uv_min.x, uv_min.y) — the atlas tile key.
            let key = sdf_atlas::uv_key([quad[5].uv[0], quad[5].uv[1]]);

            // Cancel the offset in f64, then narrow: the sheet-mm result is a
            // small number even when the world coordinate is UTM-scale.
            let point = |wx: f64, wy: f64| PlotPoint {
                x: geometry_pt((wx + ox) as f32),
                y: geometry_pt((wy + oy) as f32),
            };

            if let Some(ge) = table.get(&key) {
                // Affine basis of the quad: plane_min → bl, +x → br, +y → tl.
                // The glyph-space maths is small and stays f32; only the lift into
                // world coordinates needs f64.
                let (pmin, pmax) = (ge.plane_min, ge.plane_max);
                let (sx, sy) = (pmax[0] - pmin[0], pmax[1] - pmin[1]);
                if sx.abs() < 1e-9 || sy.abs() < 1e-9 {
                    continue;
                }
                let map = |p: [f32; 2]| -> PlotPoint {
                    let u = ((p[0] - pmin[0]) / sx) as f64;
                    let v = ((p[1] - pmin[1]) / sy) as f64;
                    let wx = bl[0] + u * (br[0] - bl[0]) + v * (tl[0] - bl[0]);
                    let wy = bl[1] + u * (br[1] - bl[1]) + v * (tl[1] - bl[1]);
                    point(wx, wy)
                };

                if !ge.fill_tris.is_empty() {
                    // Filled TrueType glyph: the triangulation of one glyph is
                    // one shape (`FillMesh`), which the PDF sink expands back
                    // into a filled triangle per triple.
                    sink.emit(PlotOp::FillColor([r, g, b]))?;
                    let tris: Vec<[PlotPoint; 3]> = ge
                        .fill_tris
                        .chunks_exact(3)
                        .map(|tri| [map(tri[0]), map(tri[1]), map(tri[2])])
                        .collect();
                    if !tris.is_empty() {
                        sink.emit(PlotOp::FillMesh { tris })?;
                    }
                } else {
                    // Stroke (LFF/SHX pen) font or hollow glyph: polylines.
                    // Match the SDF atlas' nominal glyph-space pen instead of
                    // borrowing the entity lineweight: Roman Duplex and similar
                    // multi-stroke faces rely on that band to close the narrow
                    // gaps between parallel centrelines. An explicit CTB
                    // lineweight still wins and stays absolute under the plot CTM.
                    sink.emit(PlotOp::StrokeColor([r, g, b]))?;
                    let pen = if let Some(ctb_pen) = lw_override {
                        if ge.bold {
                            ctb_pen * 1.7
                        } else {
                            ctb_pen
                        }
                    } else {
                        let glyph_unit_mm = (((tl[0] - bl[0]).powi(2) + (tl[1] - bl[1]).powi(2))
                            .sqrt()
                            / sy.abs() as f64) as f32;
                        (2.0 * sdf_atlas::stroke_pen_half_units(ge.bold) * glyph_unit_mm * MM_TO_PT)
                            .max(0.1)
                    };
                    sink.emit(PlotOp::StrokeWidthPt(pen))?;
                    for stroke in &ge.strokes {
                        if stroke.len() < 2 {
                            continue;
                        }
                        sink.emit(PlotOp::Stroke {
                            points: stroke.iter().map(|&p| map(p)).collect(),
                            closed: false,
                        })?;
                    }
                }
            } else if key == solid_key {
                // Decoration bar (underline / overline / strike): the quad is a
                // solid-texel rectangle — fill it directly from its corners.
                sink.emit(PlotOp::FillColor([r, g, b]))?;
                sink.emit(PlotOp::Fill {
                    rings: vec![[bl, br, tr, tl]
                        .iter()
                        .map(|&c| point(c[0], c[1]))
                        .collect()],
                    rule: FillRule::NonZero,
                })?;
            } else {
                // The snapshot has no geometry under this quad's key. The page
                // is short a glyph; say which one rather than let a backend
                // hand back a drawing with a hole in its text (R1).
                report.missing_glyphs += 1;
                report.first_missing.get_or_insert_with(|| MissingGlyph {
                    wire: wire.name.clone(),
                    quad: gi / 6 - 1,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::plot_corpus::Case;
    use crate::io::plot_types::PlotWire;
    use crate::scene::model::wire_model::BgAdaptInputs;
    use crate::scene::WireModel;

    /// The fill-only wire the cache builds for an MTEXT background mask on
    /// "drawing window colour": its colour is the canvas, and `canvas_color`
    /// records that it is.
    fn canvas_mask(canvas: [f32; 4]) -> PlotWire {
        let mut wire = WireModel::solid("mask".into(), Vec::new(), canvas, false);
        wire.fill_tris = vec![[10.0, 10.0, 0.0], [60.0, 10.0, 0.0], [60.0, 30.0, 0.0]];
        wire.bg_adapt = Some(Box::new(BgAdaptInputs {
            raw_color: canvas,
            canvas_color: true,
            ..Default::default()
        }));
        PlotWire {
            wire,
            draw_depth: 0.0,
        }
    }

    fn ops_for(case: &Case) -> Vec<PlotOp> {
        let mut sink = RecordingSink::default();
        match emit_plot_content(&case.page(), &PlotAssets::default(), &mut sink) {
            Ok(_) => {}
            Err(never) => match never {},
        }
        sink.ops
    }

    /// Index of the one mesh on the page and the fill colour in force there.
    fn mesh_and_fill(ops: &[PlotOp]) -> (usize, [f32; 3]) {
        let at = ops
            .iter()
            .position(|op| matches!(op, PlotOp::FillMesh { .. }))
            .expect("the mask's mesh");
        let fill = ops[..at]
            .iter()
            .rev()
            .find_map(|op| match op {
                PlotOp::FillColor(c) => Some(*c),
                _ => None,
            })
            .expect("a fill colour before the mesh");
        (at, fill)
    }

    /// A dark canvas used to print as it was — a black box behind the glyphs
    /// (SP02-05's material table). On paper the window colour is the sheet.
    #[test]
    fn mtext_canvas_mask_plots_paper_white() {
        let mut case = Case::new("mask");
        case.wires.push(canvas_mask([0.13, 0.13, 0.13, 1.0]));
        let ops = ops_for(&case);
        let (at, fill) = mesh_and_fill(&ops);
        assert_eq!(fill, [1.0, 1.0, 1.0]);
        // Nothing to switch out of without merge_lines: the colour is set and
        // the mesh follows, no blend switch between them.
        assert!(matches!(ops[at - 1], PlotOp::FillColor(_)), "{:?}", ops[at - 1]);
    }

    /// A light canvas would otherwise go through the light-to-dark adaptation
    /// and come out black just the same.
    #[test]
    fn mtext_canvas_mask_is_not_adapted_to_black() {
        let mut case = Case::new("mask-light");
        case.wires.push(canvas_mask([0.96, 0.96, 0.96, 1.0]));
        let (_, fill) = mesh_and_fill(&ops_for(&case));
        assert_eq!(fill, [1.0, 1.0, 1.0]);
    }

    /// Under `merge_lines` the page multiplies; white times ink is ink, so the
    /// mask paints in Normal for its own fill, as a wipeout does.
    #[test]
    fn mtext_canvas_mask_paints_normal_under_merge_lines() {
        let mut case = Case::new("mask-merge");
        case.options.merge_lines = true;
        case.wires.push(canvas_mask([0.13, 0.13, 0.13, 1.0]));
        let ops = ops_for(&case);
        let (at, fill) = mesh_and_fill(&ops);
        assert_eq!(fill, [1.0, 1.0, 1.0]);
        assert_eq!(ops[at - 2], PlotOp::Save);
        assert_eq!(ops[at - 1], PlotOp::Blend(PlotBlend::Normal));
        assert_eq!(ops[at + 1], PlotOp::Restore);
    }

    /// A wide polyline whose width varies, as the scene hands it to the plot:
    /// a width per point and the widest of them in `world_width`.
    fn tapered(points: Vec<[f32; 3]>, widths: Vec<f32>) -> PlotWire {
        let mut wire = crate::io::plot_corpus::wire("taper", points, WireModel::WHITE, 0.1);
        wire.wire.world_width = widths.iter().copied().fold(0.0, f32::max);
        wire.wire.taper_widths = widths;
        wire
    }

    fn fill_rings(ops: &[PlotOp]) -> Vec<Vec<PlotPoint>> {
        ops.iter()
            .filter_map(|op| match op {
                PlotOp::Fill { rings, .. } => Some(rings.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn near(p: PlotPoint, x_mm: f32, y_mm: f32) -> bool {
        (p.x - geometry_pt(x_mm)).abs() < 1e-3 && (p.y - geometry_pt(y_mm)).abs() < 1e-3
    }

    /// The flow arrow on a P&ID pipe: a two-vertex polyline from width 0 to
    /// width 1.2. It fills the triangle its widths describe — apex at the
    /// zero end, a 1.2 mm base at the other — and is not stroked at its
    /// widest width, which printed it as a round-capped pill.
    #[test]
    fn a_tapered_polyline_fills_its_triangle_instead_of_a_pill() {
        let mut case = Case::new("taper");
        case.wires.push(tapered(
            vec![[10.0, 20.0, 0.0], [10.0, 17.0, 0.0]],
            vec![0.0, 1.2],
        ));
        let ops = ops_for(&case);
        assert!(
            !ops.iter().any(|op| matches!(op, PlotOp::Stroke { .. })),
            "the arrow was stroked: {ops:?}"
        );
        let rings = fill_rings(&ops);
        assert_eq!(rings.len(), 1, "{rings:?}");
        let ring = &rings[0];
        assert!(
            ring.iter().any(|&p| near(p, 10.0, 20.0)),
            "no apex at the zero-width end: {ring:?}"
        );
        let base: Vec<PlotPoint> = ring
            .iter()
            .copied()
            .filter(|p| (p.y - geometry_pt(17.0)).abs() < 1e-3)
            .collect();
        assert_eq!(base.len(), 2, "{ring:?}");
        assert!(
            ((base[0].x - base[1].x).abs() - geometry_pt(1.2)).abs() < 1e-3,
            "base is not 1.2 mm wide: {base:?}"
        );
        // The fill is in the wire's colour — white adapted to ink — and the
        // pen state was not touched for it.
        let fill_at = ops
            .iter()
            .position(|op| matches!(op, PlotOp::Fill { .. }))
            .unwrap();
        assert!(ops[..fill_at]
            .iter()
            .any(|op| *op == PlotOp::FillColor([0.0, 0.0, 0.0])));
        assert!(!ops[..fill_at]
            .iter()
            .any(|op| matches!(op, PlotOp::StrokeWidthPt(_))));
    }

    /// At a bend the two quads meet on the mitred offsets: they share an
    /// edge, so the band has neither a notch on the outside of the corner nor
    /// a seam on the inside.
    #[test]
    fn a_tapered_polyline_is_mitred_where_it_bends() {
        let mut case = Case::new("taper-bend");
        case.wires.push(tapered(
            vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]],
            vec![2.0, 2.0, 1.0],
        ));
        let rings = fill_rings(&ops_for(&case));
        assert_eq!(rings.len(), 2, "{rings:?}");
        // The mitre of a right angle at half-width 1 lies 1 mm out along
        // both normals: (9, 1) inside the corner and (11, -1) outside.
        for corner in [(9.0, 1.0), (11.0, -1.0)] {
            for ring in &rings {
                assert!(
                    ring.iter().any(|&p| near(p, corner.0, corner.1)),
                    "{corner:?} missing from {ring:?}"
                );
            }
        }
        // The far end tapers to width 1: half a millimetre either side.
        assert!(rings[1].iter().any(|&p| near(p, 9.5, 10.0)));
        assert!(rings[1].iter().any(|&p| near(p, 10.5, 10.0)));
    }

    /// A polyline of one width still strokes its centre-line at that width,
    /// dashed by its linetype, as it always has.
    #[test]
    fn a_constant_band_still_strokes_its_centre_line() {
        let mut case = Case::new("band");
        let mut band = crate::io::plot_corpus::wire(
            "band",
            vec![[0.0, 10.0, 0.0], [50.0, 10.0, 0.0]],
            WireModel::WHITE,
            0.1,
        );
        band.wire.world_width = 3.0;
        case.wires.push(band);
        let ops = ops_for(&case);
        assert!(fill_rings(&ops).is_empty());
        assert!(ops.iter().any(
            |op| matches!(op, PlotOp::StrokeWidthPt(w) if (*w - 3.0 * MM_TO_PT).abs() < 1e-3)
        ));
        assert!(ops.iter().any(|op| matches!(op, PlotOp::Stroke { .. })));
    }

    /// The strokes and group marks of a page, in order: what a sink that
    /// nests groups would see, with the pen state left out.
    fn structure(ops: &[PlotOp]) -> Vec<String> {
        ops.iter()
            .filter_map(|op| match op {
                PlotOp::BeginGroup { tag } => Some(format!("<{tag}>")),
                PlotOp::EndGroup => Some("</>".to_string()),
                PlotOp::Stroke { points, .. } => Some(format!("stroke@{}", points[0].x)),
                _ => None,
            })
            .collect()
    }

    /// A tagged symbol's wires are drawn together where its first one falls
    /// in the depth order, wrapped in one group; the wires between them
    /// move after the group, and wires of no group draw as before.
    #[test]
    fn a_group_draws_its_members_together_where_the_first_one_falls() {
        use crate::io::plot_corpus::wire;
        let mut case = Case::new("groups");
        let black = [0.0, 0.0, 0.0, 1.0];
        // Depth order: the valve body (10), an unrelated line (11), the
        // valve's tag (12), another valve (13) and its tag (14), a last line.
        for (name, depth) in [
            ("10", 0.0),
            ("11", 1.0),
            ("12", 2.0),
            ("13", 3.0),
            ("14", 4.0),
            ("15", 5.0),
        ] {
            let x = depth * 10.0 + 10.0;
            case.wires.push(wire(
                name,
                vec![[x, 10.0, 0.0], [x, 20.0, 0.0]],
                black,
                depth,
            ));
        }
        let assets = PlotAssets {
            groups: vec![
                PlotGroup {
                    tag: "BUV-3101".into(),
                    members: vec!["10".into(), "12".into()],
                },
                PlotGroup {
                    tag: "BUV-3102".into(),
                    members: vec!["14".into(), "13".into()],
                },
                PlotGroup {
                    tag: "absent".into(),
                    members: vec!["99".into()],
                },
            ],
            ..Default::default()
        };
        let mut sink = RecordingSink::default();
        match emit_plot_content(&case.page(), &assets, &mut sink) {
            Ok(_) => {}
            Err(never) => match never {},
        }
        let pt = |x: f32| format!("stroke@{}", geometry_pt(x));
        assert_eq!(
            structure(&sink.ops),
            [
                "<BUV-3101>".to_string(),
                pt(10.0),
                pt(30.0),
                "</>".to_string(),
                pt(20.0),
                "<BUV-3102>".to_string(),
                pt(40.0),
                pt(50.0),
                "</>".to_string(),
                pt(60.0),
            ],
            "a group with no member on the page leaves no mark"
        );
        // Without groups the page is the plain depth order, unmarked.
        let plain = ops_for(&case);
        assert_eq!(
            structure(&plain),
            [10.0, 20.0, 30.0, 40.0, 50.0, 60.0].map(pt)
        );
    }

    /// The rule is keyed on `canvas_color`, not on the colour: an ordinary
    /// light fill still adapts to black, and is not wrapped in a blend switch.
    #[test]
    fn an_ordinary_light_fill_still_adapts_to_black() {
        let mut case = Case::new("fill");
        case.options.merge_lines = true;
        let mut wire = WireModel::solid("fill".into(), Vec::new(), [0.96, 0.96, 0.96, 1.0], false);
        wire.fill_tris = vec![[10.0, 10.0, 0.0], [60.0, 10.0, 0.0], [60.0, 30.0, 0.0]];
        case.wires.push(PlotWire {
            wire,
            draw_depth: 0.0,
        });
        let ops = ops_for(&case);
        let (at, fill) = mesh_and_fill(&ops);
        assert_eq!(fill, [0.0, 0.0, 0.0]);
        assert!(matches!(ops[at - 1], PlotOp::FillColor(_)), "{:?}", ops[at - 1]);
    }

    /// The fill colour in force at each solid `Fill` on the page, in order.
    fn fill_colours(ops: &[PlotOp]) -> Vec<[f32; 3]> {
        let mut current = None;
        let mut out = Vec::new();
        for op in ops {
            match op {
                PlotOp::FillColor(c) => current = Some(*c),
                PlotOp::Fill { .. } => out.push(current.expect("a fill colour before the fill")),
                _ => {}
            }
        }
        out
    }

    /// Colour 7 is the foreground colour: white on the dark screen, black on
    /// paper, as AutoCAD plots it. A solid hatch that is ACI 7 -- its own
    /// colour or ByLayer on a colour-7 layer, the CPECC sheets' pipe-junction
    /// dots -- therefore fills black on the sheet like any other light fill;
    /// only a WIPEOUT masks with the paper's white.
    #[test]
    fn an_aci_7_solid_fill_plots_black_and_only_a_wipeout_stays_white() {
        use crate::io::plot_corpus::{hatch, square};
        let mut case = Case::new("aci7");
        let mut dot = hatch(
            "SOLID",
            square(10.0, 10.0, 5.0),
            HatchPattern::Solid,
            [1.0, 1.0, 1.0, 1.0],
        );
        dot.aci = 7;
        case.hatches.push(dot);
        let mut mask = hatch(
            "WIPEOUT_FILL",
            square(30.0, 10.0, 5.0),
            HatchPattern::Solid,
            [1.0, 1.0, 1.0, 1.0],
        );
        mask.aci = 7;
        case.hatches.push(mask);
        case.hatches.push(hatch(
            "SOLID",
            square(50.0, 10.0, 5.0),
            HatchPattern::Solid,
            [1.0, 0.0, 0.0, 1.0],
        ));
        let fills = fill_colours(&ops_for(&case));
        assert_eq!(
            fills,
            [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 0.0]],
            "ACI-7 dot black, wipeout white, a genuine colour untouched"
        );
    }
}

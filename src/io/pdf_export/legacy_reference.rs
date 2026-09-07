// Frozen copy of the PDF page emitter as it was before the traversal moved to
// `io::plot_emit` (P0 of docs/plans/2026-09-07-dxf-to-svg-export.md). Test-only.
//
// This is `append_pdf_page` and every helper it called, verbatim, with one
// change: the function returns the page's `Op`s instead of pushing a `PdfPage`
// onto the document. `pdf_export::tests` runs it next to the new
// `plot_emit` → `PdfSink` path on the same inputs and asserts the two `Op`
// streams are identical. Do not "fix" or tidy anything here; when the shared
// emitter's behaviour is deliberately changed, this file changes with it in
// the same commit, or the comparison is retired.

// Frozen means frozen: the lints below fire on idioms the original used.
#![allow(
    clippy::too_many_arguments,
    clippy::chunks_exact_to_as_chunks,
    clippy::clone_on_copy
)]

use super::{PdfPlotOptions, PlotWire};
use crate::io::plot_style::PlotStyleTable;
use crate::scene::model::hatch_model::{HatchModel, HatchPattern};
use crate::scene::WireModel;
use printpdf::{
    BlendMode, BuiltinFont, Color, ExtendedGraphicsState, ExtendedGraphicsStateId, Line,
    LineCapStyle, LineDashPattern, LineJoinStyle, LinePoint, Mm, Op, PaintMode, PdfDocument,
    PdfFontHandle, Point, Polygon, PolygonRing, Pt, Rgb, TextItem, WindingOrder,
};

const MM_TO_PT: f32 = 2.834645;
const LW_PX_TO_PT: f32 = MM_TO_PT / (96.0 / 25.4);
const SCREEN_DOT_MM: f32 = 25.4 / 96.0;
pub(super) fn legacy_page_ops(
    doc: &mut PdfDocument,
    wires: &[PlotWire],
    hatches: &[HatchModel],
    wipeouts: &[HatchModel],
    paper_w: f32,
    paper_h: f32,
    ox: f64,
    oy: f64,
    rotation_deg: i32,
    scale: f32,
    clip: Option<(f32, f32, f32, f32)>,
    plot_style: Option<&PlotStyleTable>,
    options: PdfPlotOptions,
) -> Vec<Op> {
    let mut ops: Vec<Op> = Vec::new();

    // White page background.
    ops.push(Op::SetFillColor {
        col: Color::Rgb(Rgb {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            icc_profile: None,
        }),
    });
    ops.push(Op::DrawRectangle {
        rectangle: printpdf::Rect::from_wh(Mm(paper_w).into(), Mm(paper_h).into()),
    });

    let normal_blend = if options.merge_lines {
        let merge = doc.add_graphics_state(
            ExtendedGraphicsState::default().with_blend_mode(BlendMode::multiply()),
        );
        let normal = doc.add_graphics_state(
            ExtendedGraphicsState::default().with_blend_mode(BlendMode::normal()),
        );
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::LoadGraphicsState { gs: merge });
        Some(normal)
    } else {
        None
    };

    // Round line caps/joins for CAD aesthetics.
    ops.push(Op::SetLineCapStyle {
        cap: LineCapStyle::Round,
    });
    ops.push(Op::SetLineJoinStyle {
        join: LineJoinStyle::Round,
    });

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
        ops.push(Op::SaveGraphicsState);
        // Convert mm translation to points (1 mm = 2.834645 pt).
        let tx_pt = (tx * 2.834645) as f32;
        let ty_pt = (ty * 2.834645) as f32;
        ops.push(Op::SetTransformationMatrix {
            matrix: printpdf::CurTransMat::Raw([
                (cos_a * s) as f32,
                (sin_a * s) as f32,
                (-(sin_a) * s) as f32,
                (cos_a * s) as f32,
                tx_pt,
                ty_pt,
            ]),
        });
        // Clip rectangle (mm), applied in the pre-scale coordinate space so it
        // matches the wires drawn under the same CTM.
        if let Some((cx, cy, cw, ch)) = clip {
            ops.push(Op::DrawPolygon {
                polygon: Polygon {
                    rings: vec![PolygonRing {
                        points: vec![
                            LinePoint {
                                p: Point {
                                    x: Pt(cx * MM_TO_PT),
                                    y: Pt(cy * MM_TO_PT),
                                },
                                bezier: false,
                            },
                            LinePoint {
                                p: Point {
                                    x: Pt((cx + cw) * MM_TO_PT),
                                    y: Pt(cy * MM_TO_PT),
                                },
                                bezier: false,
                            },
                            LinePoint {
                                p: Point {
                                    x: Pt((cx + cw) * MM_TO_PT),
                                    y: Pt((cy + ch) * MM_TO_PT),
                                },
                                bezier: false,
                            },
                            LinePoint {
                                p: Point {
                                    x: Pt(cx * MM_TO_PT),
                                    y: Pt((cy + ch) * MM_TO_PT),
                                },
                                bezier: false,
                            },
                        ],
                    }],
                    mode: PaintMode::Clip,
                    winding_order: WindingOrder::NonZero,
                },
            });
        }
    }

    let (first_wires, second_wires) = wires.split_at(options.group_splits.wires.min(wires.len()));
    let (first_hatches, second_hatches) =
        hatches.split_at(options.group_splits.hatches.min(hatches.len()));
    let (first_wipeouts, second_wipeouts) =
        wipeouts.split_at(options.group_splits.wipeouts.min(wipeouts.len()));
    for (wires, hatches, wipeouts) in [
        (first_wires, first_hatches, first_wipeouts),
        (second_wires, second_hatches, second_wipeouts),
    ] {
        enum DrawItem<'a> {
            WireFill(&'a PlotWire),
            Hatch(&'a HatchModel),
            Wire(&'a PlotWire),
            Text(&'a PlotWire),
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

        let mut last_color: Option<[f32; 3]> = None;
        let mut last_lw: Option<f32> = None;
        let mut last_cap = Some(LineCapStyle::Round);
        let mut last_join = Some(LineJoinStyle::Round);
        // Current PDF dash array (empty = solid). Tracked so the dash op is only
        // re-emitted when it actually changes between wires.
        let mut last_dash: Option<Vec<i64>> = None;

        for (_, _, _, item) in draw_items {
            let wire = match item {
                DrawItem::WireFill(wire) => {
                    emit_wire_fills(
                        &mut ops,
                        std::slice::from_ref(&wire.wire),
                        ox,
                        oy,
                        plot_style,
                        scale,
                        options,
                        normal_blend.as_ref(),
                    );
                    last_color = None;
                    last_lw = None;
                    last_dash = None;
                    continue;
                }
                DrawItem::Hatch(hatch) => {
                    emit_hatch(
                        &mut ops,
                        hatch,
                        ox,
                        oy,
                        plot_style,
                        scale,
                        options,
                        normal_blend.as_ref(),
                    );
                    last_color = None;
                    last_lw = None;
                    last_dash = None;
                    continue;
                }
                DrawItem::Text(wire) => {
                    emit_text(
                        &mut ops,
                        std::slice::from_ref(&wire.wire),
                        ox,
                        oy,
                        scale,
                        plot_style,
                        options,
                    );
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
            // Skip screen-only paper helpers. The PDF page supplies its own white
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
                            0 => Some(LineCapStyle::Butt),
                            1 | 3 => Some(LineCapStyle::ProjectingSquare),
                            2 => Some(LineCapStyle::Round),
                            _ => None,
                        };
                        join = match entry.join_style {
                            0 => Some(LineJoinStyle::Miter),
                            1 | 3 => Some(LineJoinStyle::Bevel),
                            2 => Some(LineJoinStyle::Round),
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

            let cap = cap.unwrap_or(LineCapStyle::Round);
            if last_cap != Some(cap) {
                ops.push(Op::SetLineCapStyle { cap });
                last_cap = Some(cap);
            }
            let join = join.unwrap_or(LineJoinStyle::Round);
            if last_join != Some(join) {
                ops.push(Op::SetLineJoinStyle { join });
                last_join = Some(join);
            }

            if last_color
                .map(|c| {
                    (c[0] - r).abs() > 0.01 || (c[1] - g).abs() > 0.01 || (c[2] - b).abs() > 0.01
                })
                .unwrap_or(true)
            {
                let color = Color::Rgb(Rgb {
                    r,
                    g,
                    b,
                    icc_profile: None,
                });
                ops.push(Op::SetOutlineColor { col: color.clone() });
                ops.push(Op::SetFillColor { col: color });
                last_color = Some([r, g, b]);
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
                ops.push(Op::SetOutlineThickness { pt: Pt(lw_pt) });
                last_lw = Some(lw_pt);
            }

            // Linetype dash pattern. Without this every wire exported as a solid
            // line regardless of its linetype (dashed / centre / dash-dot). (#155)
            let dash_arr = dash_array_from_pattern(wire.pattern_length, &wire.pattern, MM_TO_PT);
            let stationed = !dash_arr.is_empty() && wire.pattern_stations.len() > wire.points.len();
            if stationed {
                if last_dash.as_ref().is_none_or(|dash| !dash.is_empty()) {
                    ops.push(Op::SetLineDashPattern {
                        dash: LineDashPattern::default(),
                    });
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
                        let point = |t: f32| LinePoint {
                            p: Point::new(
                                Mm((start.x + (end.x - start.x) * t as f64 + ox) as f32),
                                Mm((start.y + (end.y - start.y) * t as f64 + oy) as f32),
                            ),
                            bezier: false,
                        };
                        flush_line(&mut ops, &[point(from), point(to)], None);
                    }
                }
                continue;
            }
            if last_dash.as_deref() != Some(dash_arr.as_slice()) {
                let dash = if dash_arr.is_empty() {
                    LineDashPattern::default()
                } else {
                    LineDashPattern::from_array(&dash_arr, 0)
                };
                ops.push(Op::SetLineDashPattern { dash });
                last_dash = Some(dash_arr.clone());
            }

            // Emit segments (NaN = pen-up). Points are the "high" half of a
            // double-single pair; fold in the `points_low` residual and cancel the
            // offset in f64 before narrowing. Dropping the residual (or narrowing
            // first) snaps a UTM drawing onto the f32 grid — ~3 cm across, ~50 cm
            // along northing — which is exactly the distortion the plot showed while
            // low-coordinate drawings came out clean. The result is a sheet-mm value
            // in single digits, so f32 is lossless from here.
            let mut segment: Vec<LinePoint> = Vec::new();
            let dot_radius = (wire.name == "viewport_hatch_pattern")
                .then_some(Pt(SCREEN_DOT_MM * MM_TO_PT / (2.0 * scale.max(1e-6))));
            for (pi, &[x, y, _z]) in wire.points.iter().enumerate() {
                if x.is_nan() || y.is_nan() {
                    flush_line(&mut ops, &segment, dot_radius);
                    segment.clear();
                } else {
                    let point = wire.point_world(pi, paper_h as f64 / scale.max(1e-6) as f64);
                    let wx = (point.x + ox) as f32;
                    let wy = (point.y + oy) as f32;
                    segment.push(LinePoint {
                        p: Point::new(Mm(wx), Mm(wy)),
                        bezier: false,
                    });
                }
            }
            flush_line(&mut ops, &segment, dot_radius);
        }
    }

    if needs_state {
        ops.push(Op::RestoreGraphicsState);
    }
    if options.merge_lines {
        ops.push(Op::RestoreGraphicsState);
    }
    if options.stamp {
        emit_plot_stamp(&mut ops);
    }
    ops
}

/// Build a PDF dash array (in points) from a WireModel linetype pattern.
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

fn flush_line(ops: &mut Vec<Op>, pts: &[LinePoint], dot_radius: Option<Pt>) {
    if pts.len() < 2 {
        return;
    }
    if let Some(radius) = dot_radius {
        let first = pts[0].p;
        let coincident = pts.iter().skip(1).all(|point| {
            (point.p.x.0 - first.x.0).abs() <= 1e-6 && (point.p.y.0 - first.y.0).abs() <= 1e-6
        });
        if coincident {
            emit_round_dot(ops, first, radius);
            return;
        }
    }
    ops.push(Op::DrawLine {
        line: Line {
            points: pts.to_vec(),
            is_closed: false,
        },
    });
}

fn emit_round_dot(ops: &mut Vec<Op>, center: Point, radius: Pt) {
    const SIDES: usize = 12;
    let points = (0..SIDES)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / SIDES as f32;
            LinePoint {
                p: Point {
                    x: Pt(center.x.0 + radius.0 * angle.cos()),
                    y: Pt(center.y.0 + radius.0 * angle.sin()),
                },
                bezier: false,
            }
        })
        .collect();
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing { points }],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        },
    });
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

fn emit_wire_fills(
    ops: &mut Vec<Op>,
    wires: &[WireModel],
    ox: f64,
    oy: f64,
    plot_style: Option<&PlotStyleTable>,
    scale: f32,
    options: PdfPlotOptions,
    normal_blend: Option<&ExtendedGraphicsStateId>,
) {
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
                    ops,
                    &hatch,
                    ox,
                    oy,
                    plot_style,
                    scale,
                    options,
                    normal_blend,
                );
            }
            continue;
        }
        let [mut r, mut g, mut b, a] = wire.color;
        if a < 0.01 {
            continue;
        }
        let mut screening = 1.0;
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
        [r, g, b] = plotted_color([r, g, b], a, screening, options);
        ops.push(Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r,
                g,
                b,
                icc_profile: None,
            }),
        });
        for (triangle_index, triangle) in wire.fill_tris.chunks_exact(3).enumerate() {
            let mut points = Vec::with_capacity(3);
            for (point_index, &[x, y, _]) in triangle.iter().enumerate() {
                let index = triangle_index * 3 + point_index;
                let low = wire.fill_tris_low.get(index).copied().unwrap_or([0.0; 3]);
                points.push(LinePoint {
                    p: Point::new(
                        Mm((x as f64 + low[0] as f64 + ox) as f32),
                        Mm((y as f64 + low[1] as f64 + oy) as f32),
                    ),
                    bezier: false,
                });
            }
            ops.push(Op::DrawPolygon {
                polygon: Polygon {
                    rings: vec![PolygonRing { points }],
                    mode: PaintMode::Fill,
                    winding_order: WindingOrder::NonZero,
                },
            });
        }
    }
}

fn emit_plot_stamp(ops: &mut Vec<Op>) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into());
    let label = format!("Open CAD Studio | {user} | {timestamp}");
    ops.extend([
        Op::SaveGraphicsState,
        Op::StartTextSection,
        Op::SetTextCursor {
            pos: Point::new(Mm(4.0), Mm(3.0)),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
            size: Pt(6.0),
        },
        Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r: 0.25,
                g: 0.25,
                b: 0.25,
                icc_profile: None,
            }),
        },
        Op::ShowText {
            items: vec![TextItem::Text(label)],
        },
        Op::EndTextSection,
        Op::RestoreGraphicsState,
    ]);
}

/// Emit a single hatch / wipeout as a filled (or stroked, for pattern fills)
/// polygon. NaN sentinels in `hatch.boundary` split the path into multiple
/// rings so islands and holes render correctly under the even-odd rule.
/// Mirrors `scene::paper_canvas::draw_hatch`: solid → fill, pattern → outline,
/// gradient → solid fill of the averaged colour.
fn emit_hatch(
    ops: &mut Vec<Op>,
    hatch: &HatchModel,
    ox: f64,
    oy: f64,
    plot_style: Option<&PlotStyleTable>,
    scale: f32,
    options: PdfPlotOptions,
    normal_blend: Option<&ExtendedGraphicsStateId>,
) {
    if hatch.boundary.is_empty() {
        return;
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
        return;
    }
    // Adapt hatch fills to the white sheet, mirroring the wire pass: colours
    // arrive adapted to the (dark) screen background, so a white/ACI-7 fill
    // would vanish white-on-white on paper. Force near-white/near-yellow → black
    // and near-cyan → dark blue for a readable white-sheet result.
    // Genuine colours are untouched; WIPEOUTS keep their paper-white mask.
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
    } else if !color_overridden && !(hatch.aci == 7 && matches!(hatch.pattern, HatchPattern::Solid))
    {
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
    let mut rings: Vec<PolygonRing> = Vec::new();
    let mut current: Vec<LinePoint> = Vec::new();
    for &[bx, by] in hatch.boundary.iter() {
        if bx.is_nan() || by.is_nan() {
            if current.len() >= 3 {
                rings.push(PolygonRing {
                    points: std::mem::take(&mut current),
                });
            } else {
                current.clear();
            }
            continue;
        }
        let px = (bx as f64 + world_ox + ox) as f32;
        let py = (by as f64 + world_oy + oy) as f32;
        current.push(LinePoint {
            p: Point::new(Mm(px), Mm(py)),
            bezier: false,
        });
    }
    if current.len() >= 3 {
        rings.push(PolygonRing { points: current });
    }
    if rings.is_empty() {
        return;
    }

    let (paint_mode, fill_color) = match &hatch.pattern {
        HatchPattern::Solid => (PaintMode::Fill, [r, g, b]),
        HatchPattern::Pattern(_) => {
            // Pattern fills are emitted as raster line segments below; the
            // outline polygon path itself is skipped because pattern
            // hatches in real DXF do not draw their boundary as part of
            // the fill.
            (PaintMode::Clip, [r, g, b]) // sentinel — handled below
        }
        HatchPattern::Gradient { color2, .. } => {
            // PDF gradients are stored in resource dictionaries; for the
            // fast path we average the two colours, matching paper_canvas.
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
            let avg = [
                (r + second[0]) * 0.5,
                (g + second[1]) * 0.5,
                (b + second[2]) * 0.5,
            ];
            (PaintMode::Fill, avg)
        }
    };

    // Pattern hatches: rasterise the family lines clipped to the boundary
    // and emit each as a stroked line. Skips the polygon outline entirely.
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
            return;
        }
        let color = Color::Rgb(Rgb {
            r,
            g,
            b,
            icc_profile: None,
        });
        ops.push(Op::SetOutlineColor { col: color.clone() });
        ops.push(Op::SetFillColor { col: color });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(physical / divisor),
        });
        // Pattern dashes are already materialized by `pattern_segments`.
        // Clear any linetype left by the preceding paper/model render group.
        ops.push(Op::SetLineDashPattern {
            dash: LineDashPattern::default(),
        });
        for [a, b_pt] in segments {
            // `pattern_segments` returns absolute world f64; cancel the offset
            // before narrowing, as everywhere else in this file.
            let (ax, ay) = ((a[0] + ox) as f32, (a[1] + oy) as f32);
            let (bx, by) = ((b_pt[0] + ox) as f32, (b_pt[1] + oy) as f32);
            let points = vec![
                LinePoint {
                    p: Point::new(Mm(ax), Mm(ay)),
                    bezier: false,
                },
                LinePoint {
                    p: Point::new(Mm(bx), Mm(by)),
                    bezier: false,
                },
            ];
            let dot_radius = Pt(SCREEN_DOT_MM * MM_TO_PT / (2.0 * scale.max(1e-6)));
            flush_line(ops, &points, Some(dot_radius));
        }
        return;
    }

    // Solid / gradient: filled polygon path.
    if matches!(paint_mode, PaintMode::Fill | PaintMode::FillStroke) {
        ops.push(Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r: fill_color[0],
                g: fill_color[1],
                b: fill_color[2],
                icc_profile: None,
            }),
        });
    }
    if is_wipeout {
        if let Some(gs) = normal_blend {
            ops.push(Op::SaveGraphicsState);
            ops.push(Op::LoadGraphicsState { gs: gs.clone() });
        }
    }
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings,
            mode: paint_mode,
            winding_order: WindingOrder::EvenOdd,
        },
    });
    if is_wipeout && normal_blend.is_some() {
        ops.push(Op::RestoreGraphicsState);
    }
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
fn emit_text(
    ops: &mut Vec<Op>,
    wires: &[WireModel],
    ox: f64,
    oy: f64,
    scale: f32,
    plot_style: Option<&PlotStyleTable>,
    options: PdfPlotOptions,
) {
    use crate::scene::text::sdf_atlas;

    if wires.iter().all(|w| w.text_verts.is_empty()) {
        return;
    }
    // Snapshot the atlas' baked-glyph geometry once; drop the lock before use.
    let (table, solid_key) = {
        let Ok(atlas) = sdf_atlas::text_atlas().lock() else {
            return;
        };
        (atlas.export_table(), sdf_atlas::uv_key(atlas.solid_uv()))
    };

    // `Op::SetLineDashPattern` is persistent graphics state and the wire pass
    // above only re-emits it on change, so whatever the last wire needed is
    // still active here — without this reset a drawing whose last wire carries a
    // HIDDEN/CENTER linetype prints its glyph outlines dashed.
    ops.push(Op::SetLineDashPattern {
        dash: LineDashPattern::default(),
    });

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
            let point = |wx: f64, wy: f64| Point::new(Mm((wx + ox) as f32), Mm((wy + oy) as f32));

            if let Some(ge) = table.get(&key) {
                // Affine basis of the quad: plane_min → bl, +x → br, +y → tl.
                // The glyph-space maths is small and stays f32; only the lift into
                // world coordinates needs f64.
                let (pmin, pmax) = (ge.plane_min, ge.plane_max);
                let (sx, sy) = (pmax[0] - pmin[0], pmax[1] - pmin[1]);
                if sx.abs() < 1e-9 || sy.abs() < 1e-9 {
                    continue;
                }
                let map = |p: [f32; 2]| -> Point {
                    let u = ((p[0] - pmin[0]) / sx) as f64;
                    let v = ((p[1] - pmin[1]) / sy) as f64;
                    let wx = bl[0] + u * (br[0] - bl[0]) + v * (tl[0] - bl[0]);
                    let wy = bl[1] + u * (br[1] - bl[1]) + v * (tl[1] - bl[1]);
                    point(wx, wy)
                };

                if !ge.fill_tris.is_empty() {
                    // Filled TrueType glyph: one filled triangle per triple.
                    ops.push(Op::SetFillColor {
                        col: Color::Rgb(Rgb {
                            r,
                            g,
                            b,
                            icc_profile: None,
                        }),
                    });
                    for tri in ge.fill_tris.chunks_exact(3) {
                        ops.push(Op::DrawPolygon {
                            polygon: Polygon {
                                rings: vec![PolygonRing {
                                    points: tri
                                        .iter()
                                        .map(|&p| LinePoint {
                                            p: map(p),
                                            bezier: false,
                                        })
                                        .collect(),
                                }],
                                mode: PaintMode::Fill,
                                winding_order: WindingOrder::NonZero,
                            },
                        });
                    }
                } else {
                    // Stroke (LFF/SHX pen) font or hollow glyph: polylines.
                    // Match the SDF atlas' nominal glyph-space pen instead of
                    // borrowing the entity lineweight: Roman Duplex and similar
                    // multi-stroke faces rely on that band to close the narrow
                    // gaps between parallel centrelines. An explicit CTB
                    // lineweight still wins and stays absolute under the plot CTM.
                    ops.push(Op::SetOutlineColor {
                        col: Color::Rgb(Rgb {
                            r,
                            g,
                            b,
                            icc_profile: None,
                        }),
                    });
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
                    ops.push(Op::SetOutlineThickness { pt: Pt(pen) });
                    for stroke in &ge.strokes {
                        if stroke.len() < 2 {
                            continue;
                        }
                        ops.push(Op::DrawLine {
                            line: Line {
                                points: stroke
                                    .iter()
                                    .map(|&p| LinePoint {
                                        p: map(p),
                                        bezier: false,
                                    })
                                    .collect(),
                                is_closed: false,
                            },
                        });
                    }
                }
            } else if key == solid_key {
                // Decoration bar (underline / overline / strike): the quad is a
                // solid-texel rectangle — fill it directly from its corners.
                ops.push(Op::SetFillColor {
                    col: Color::Rgb(Rgb {
                        r,
                        g,
                        b,
                        icc_profile: None,
                    }),
                });
                ops.push(Op::DrawPolygon {
                    polygon: Polygon {
                        rings: vec![PolygonRing {
                            points: [bl, br, tr, tl]
                                .iter()
                                .map(|&c| LinePoint {
                                    p: point(c[0], c[1]),
                                    bezier: false,
                                })
                                .collect(),
                        }],
                        mode: PaintMode::Fill,
                        winding_order: WindingOrder::NonZero,
                    },
                });
            }
        }
    }
}

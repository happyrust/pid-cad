//! Multi-sheet P&ID Master Line List (Piping Schedule) builder, cross-tab `From -> To`
//! endpoint resolver, inline valve/specialty counter, and unterminated-line auditor
//! (`PIDLINELIST` & `--export-lines`, `L3-3` / `L3-4` / `L3-6` / `L6-2` / `#167`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::{parse_sppid_line_number, ParsedLineNumber};
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::find::match_query_rank;
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::classify_valve_role_and_subtype;

/// Canonical 12-column header order shared by CSV and TSV Master Line List exports.
pub const LINE_LIST_CSV_HEADERS: [&str; 12] = [
    "LineNumber",
    "NominalDiameter",
    "FluidCode",
    "SeqNo",
    "PipingSpec",
    "Insulation",
    "FromEndpoint",
    "ToEndpoint",
    "TerminationStatus",
    "DrawingCodes",
    "InlineValves",
    "SegmentCount",
];

/// Endpoint termination completeness status for a pipeline in the Master Line List.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineTerminationStatus {
    BothTerminated,
    SingleEnded,
    Unterminated,
}

impl LineTerminationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BothTerminated => "both_terminated",
            Self::SingleEnded => "single_ended",
            Self::Unterminated => "unterminated",
        }
    }
}

/// One aggregated process or utility pipeline row in `PidLineListReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidPipelineRow {
    pub line_number: String,
    pub nominal_diameter: String,
    pub fluid_code: String,
    pub seq_no: String,
    pub piping_spec: String,
    pub insulation: String,
    pub from_endpoint: String,
    pub to_endpoint: String,
    pub status: LineTerminationStatus,
    pub primary_tab: usize,
    pub drawing_codes: Vec<String>,
    pub segment_count: usize,
    pub inline_valves_count: usize,
    pub inline_specialties_count: usize,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Summary statistics across all aggregated pipelines in `PIDLINELIST`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidLineListSummary {
    pub sheets_scanned: usize,
    pub total_pipelines: usize,
    pub multi_sheet_pipelines: usize,
    pub both_terminated_count: usize,
    pub single_ended_count: usize,
    pub unterminated_count: usize,
    pub total_inline_valves: usize,
}

/// Complete `PIDLINELIST` Master Line List report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidLineListReport {
    pub summary: PidLineListSummary,
    pub pipelines: Vec<PidPipelineRow>,
}

type Pt2 = (f64, f64);

#[derive(Clone, Copy)]
struct Span2 {
    a: Pt2,
    b: Pt2,
}

#[derive(Clone, Copy)]
struct BBox2 {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LlKind {
    Equipment { is_driver_or_pump: bool },
    Nozzle,
    Valve,
    InlineSpecialtyOrInst,
    Opc { is_paired: bool },
    PipeSegment,
}

struct LlNode {
    tab_index: usize,
    drawing_code: String,
    kind: LlKind,
    raw_label: Option<String>,
    parsed_line: Option<ParsedLineNumber>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct LlGraph {
    nodes: Vec<LlNode>,
    adj: Vec<Vec<usize>>,
}

fn parse_csv_handles(raw: &str) -> Vec<Handle> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(parse_handle)
        .collect()
}

fn parse_pair_f64(s: &str) -> Option<Pt2> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn parse_bbox_str(s: &str) -> Option<BBox2> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(BBox2 {
        min_x: parts[0].trim().parse().ok()?,
        min_y: parts[1].trim().parse().ok()?,
        max_x: parts[2].trim().parse().ok()?,
        max_y: parts[3].trim().parse().ok()?,
    })
}

fn point_to_span_dist(p: Pt2, s: Span2) -> f64 {
    let vx = s.b.0 - s.a.0;
    let vy = s.b.1 - s.a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return (p.0 - s.a.0).hypot(p.1 - s.a.1);
    }
    let t = (((p.0 - s.a.0) * vx + (p.1 - s.a.1) * vy) / len2).clamp(0.0, 1.0);
    (p.0 - (s.a.0 + t * vx)).hypot(p.1 - (s.a.1 + t * vy))
}

fn orient2d(a: Pt2, b: Pt2, c: Pt2) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn segments_cross(s1: Span2, s2: Span2) -> bool {
    let o1 = orient2d(s1.a, s1.b, s2.a);
    let o2 = orient2d(s1.a, s1.b, s2.b);
    let o3 = orient2d(s2.a, s2.b, s1.a);
    let o4 = orient2d(s2.a, s2.b, s1.b);
    (o1 * o2 < 0.0) && (o3 * o4 < 0.0)
}

fn spans_touch(s1: Span2, s2: Span2, tol: f64) -> bool {
    point_to_span_dist(s1.a, s2) <= tol
        || point_to_span_dist(s1.b, s2) <= tol
        || point_to_span_dist(s2.a, s1) <= tol
        || point_to_span_dist(s2.b, s1) <= tol
        || segments_cross(s1, s2)
}

fn point_in_bbox(p: Pt2, bb: BBox2, margin: f64) -> bool {
    p.0 >= bb.min_x - margin
        && p.0 <= bb.max_x + margin
        && p.1 >= bb.min_y - margin
        && p.1 <= bb.max_y + margin
}

fn bboxes_overlap(a: BBox2, b: BBox2, margin: f64) -> bool {
    a.min_x - margin <= b.max_x
        && a.max_x + margin >= b.min_x
        && a.min_y - margin <= b.max_y
        && a.max_y + margin >= b.min_y
}

fn span_touches_bbox(s: Span2, bb: BBox2, margin: f64) -> bool {
    let s_bb = BBox2 {
        min_x: s.a.0.min(s.b.0),
        min_y: s.a.1.min(s.b.1),
        max_x: s.a.0.max(s.b.0),
        max_y: s.a.1.max(s.b.1),
    };
    if !bboxes_overlap(s_bb, bb, margin) {
        return false;
    }
    if point_in_bbox(s.a, bb, margin) || point_in_bbox(s.b, bb, margin) {
        return true;
    }
    let x0 = bb.min_x - margin;
    let y0 = bb.min_y - margin;
    let x1 = bb.max_x + margin;
    let y1 = bb.max_y + margin;
    let edges = [
        Span2 {
            a: (x0, y0),
            b: (x1, y0),
        },
        Span2 {
            a: (x1, y0),
            b: (x1, y1),
        },
        Span2 {
            a: (x1, y1),
            b: (x0, y1),
        },
        Span2 {
            a: (x0, y1),
            b: (x0, y0),
        },
    ];
    edges.iter().any(|&e| spans_touch(s, e, 1e-6))
}

fn clean_drawing_stem(raw: &str) -> String {
    raw.strip_suffix(".dxf")
        .or_else(|| raw.strip_suffix(".DXF"))
        .or_else(|| raw.strip_suffix(".dwg"))
        .or_else(|| raw.strip_suffix(".DWG"))
        .or_else(|| raw.strip_suffix(".pid"))
        .or_else(|| raw.strip_suffix(".PID"))
        .unwrap_or(raw)
        .to_string()
}

fn classify_ll_kind(cls: &str, type_code: &str, desc_up: &str, tag_opt: Option<&str>) -> LlKind {
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        LlKind::Opc { is_paired: false }
    } else if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        LlKind::Nozzle
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        let is_pump = cls == "PIDMechanicalEquipment"
            || desc_up.contains("PUMP")
            || desc_up.contains("COMPRESSOR")
            || desc_up.contains("BLOWER")
            || tag_opt.is_some_and(|t| {
                let u = t.trim().to_ascii_uppercase();
                u.starts_with("P-") || u.starts_with("C-") || u.starts_with("K-")
            });
        LlKind::Equipment {
            is_driver_or_pump: is_pump,
        }
    } else if classify_valve_role_and_subtype(cls, type_code, desc_up, tag_opt).is_some() {
        LlKind::Valve
    } else {
        LlKind::InlineSpecialtyOrInst
    }
}

fn entity_center_and_bbox(doc: &Document, h: Handle, upm: f64) -> Option<(Pt2, BBox2)> {
    match doc.get_entity(h)? {
        EntityType::Insert(ins) => {
            let (x, y) = (ins.insert_point.x, ins.insert_point.y);
            let hw = 6.0 * upm * ins.x_scale().abs().max(0.5);
            let hh = 6.0 * upm * ins.y_scale().abs().max(0.5);
            Some((
                (x, y),
                BBox2 {
                    min_x: x - hw,
                    min_y: y - hh,
                    max_x: x + hw,
                    max_y: y + hh,
                },
            ))
        }
        EntityType::Circle(c) => {
            let (x, y, r) = (c.center.x, c.center.y, c.radius.abs());
            Some((
                (x, y),
                BBox2 {
                    min_x: x - r,
                    min_y: y - r,
                    max_x: x + r,
                    max_y: y + r,
                },
            ))
        }
        EntityType::Line(l) => {
            let min_x = l.start.x.min(l.end.x);
            let max_x = l.start.x.max(l.end.x);
            let min_y = l.start.y.min(l.end.y);
            let max_y = l.start.y.max(l.end.y);
            Some((
                ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
                BBox2 {
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                },
            ))
        }
        EntityType::LwPolyline(pl) if !pl.vertices.is_empty() => {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for v in &pl.vertices {
                min_x = min_x.min(v.location.x);
                min_y = min_y.min(v.location.y);
                max_x = max_x.max(v.location.x);
                max_y = max_y.max(v.location.y);
            }
            Some((
                ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
                BBox2 {
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                },
            ))
        }
        _ => None,
    }
}

fn spans_bbox_and_center(spans: &[Span2]) -> (Pt2, BBox2) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for s in spans {
        min_x = min_x.min(s.a.0).min(s.b.0);
        min_y = min_y.min(s.a.1).min(s.b.1);
        max_x = max_x.max(s.a.0).max(s.b.0);
        max_y = max_y.max(s.a.1).max(s.b.1);
    }
    if !min_x.is_finite() {
        return (
            (0.0, 0.0),
            BBox2 {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 0.0,
                max_y: 0.0,
            },
        );
    }
    (
        ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
        BBox2 {
            min_x,
            min_y,
            max_x,
            max_y,
        },
    )
}

fn extract_authored_ll_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<LlNode>,
) {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let rec_opt = (!has_native_pid_semantics(doc)).then(|| {
        recognise_sppid_sheet(
            doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        )
    });
    let default_auto_lbl = rec_opt
        .as_ref()
        .and_then(|r| r.pipe_labels.first())
        .map(|l| l.raw_text.trim().trim_end_matches('-').to_string());
    let mut rec_sym_by_h: BTreeMap<Handle, &super::recognize::ConvertedSymbol> = BTreeMap::new();
    if let Some(ref rec) = rec_opt {
        for sym in &rec.symbols {
            if let Some(rh) = sym.entity_handle {
                rec_sym_by_h.insert(Handle::new(rh), sym);
            }
        }
    }

    let resolve_span_lbl = |sp: Span2, cand: Option<&str>| -> Option<String> {
        let Some(ref rec) = rec_opt else {
            return cand.map(str::to_string);
        };
        if rec.pipe_labels.is_empty() {
            return cand.map(str::to_string);
        }
        let mut near_lbl: Option<(f64, &str)> = None;
        for pl in &rec.pipe_labels {
            let d = point_to_span_dist(pl.at, sp);
            if d <= 36.0 * upm && near_lbl.is_none_or(|(bd, _)| d < bd) {
                near_lbl = Some((d, pl.raw_text.as_str()));
            }
        }
        if let Some(c) = cand {
            if default_auto_lbl.as_deref() == Some(c) {
                let near_default = rec.pipe_labels.iter().any(|pl| {
                    pl.raw_text.trim().trim_end_matches('-') == c
                        && point_to_span_dist(pl.at, sp) <= 40.0 * upm
                });
                if !near_default {
                    return near_lbl.map(|(_, t)| t.trim().trim_end_matches('-').to_string());
                }
            }
            return Some(c.to_string());
        }
        near_lbl.map(|(_, t)| t.trim().trim_end_matches('-').to_string())
    };

    let mut pipe_indices_by_h: BTreeMap<Handle, Vec<usize>> = BTreeMap::new();
    let mut promoted_sym_handles: BTreeSet<Handle> = BTreeSet::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            let base_lbl = pmap
                .get("_line_label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_end_matches('-').to_string());
            let mut span_items: Vec<(Span2, Option<String>)> = Vec::new();
            if let Some(raw_segs) = pmap.get("_segments").and_then(Value::as_str) {
                for entry in raw_segs.split(';').filter(|s| !s.trim().is_empty()) {
                    let (c_str, sub_lbl) = match entry.split_once('|') {
                        Some((a, b)) => (a, Some(b.trim().trim_end_matches('-').to_string())),
                        None => (entry, base_lbl.clone()),
                    };
                    if let Some(bb) = parse_bbox_str(c_str) {
                        span_items.push((
                            Span2 {
                                a: (bb.min_x, bb.min_y),
                                b: (bb.max_x, bb.max_y),
                            },
                            sub_lbl.filter(|s| !s.is_empty()),
                        ));
                    }
                }
            }
            if span_items.is_empty() {
                if let EntityType::Line(ln) = ent {
                    span_items.push((
                        Span2 {
                            a: (ln.start.x, ln.start.y),
                            b: (ln.end.x, ln.end.y),
                        },
                        base_lbl.clone(),
                    ));
                } else if let EntityType::LwPolyline(pl) = ent {
                    for w in pl.vertices.windows(2) {
                        span_items.push((
                            Span2 {
                                a: (w[0].location.x, w[0].location.y),
                                b: (w[1].location.x, w[1].location.y),
                            },
                            base_lbl.clone(),
                        ));
                    }
                }
            }
            for (sp, slbl) in span_items {
                let resolved = resolve_span_lbl(sp, slbl.as_deref());
                let parsed = resolved.as_deref().and_then(parse_sppid_line_number);
                let (center, bbox) = spans_bbox_and_center(&[sp]);
                let idx = nodes.len();
                nodes.push(LlNode {
                    tab_index,
                    drawing_code: drawing_code.to_string(),
                    kind: LlKind::PipeSegment,
                    raw_label: resolved,
                    parsed_line: parsed,
                    primary_handle: h,
                    member_handles: vec![h],
                    center,
                    bbox,
                    ports: Vec::new(),
                    spans: vec![sp],
                });
                pipe_indices_by_h.entry(h).or_default().push(idx);
            }
            continue;
        }

        let mut members = vec![h];
        for key in ["_member_handles", "_tag_handles"] {
            if let Some(s) = pmap.get(key).and_then(Value::as_str) {
                for mh in parse_csv_handles(s) {
                    if !members.contains(&mh) {
                        members.push(mh);
                    }
                }
            }
        }
        for &mh in &members {
            promoted_sym_handles.insert(mh);
        }
        let rec_sym = rec_sym_by_h.get(&h).copied();
        let fallback_cb = entity_center_and_bbox(doc, h, upm);
        let center = pmap
            .get("_at")
            .and_then(Value::as_str)
            .and_then(parse_pair_f64)
            .or_else(|| rec_sym.map(|s| s.at))
            .or_else(|| fallback_cb.map(|(c, _)| c))
            .unwrap_or((0.0, 0.0));
        let bbox = pmap
            .get("_bbox")
            .and_then(Value::as_str)
            .and_then(parse_bbox_str)
            .filter(|b| (b.max_x - b.min_x).abs() > 1e-9 || (b.max_y - b.min_y).abs() > 1e-9)
            .or_else(|| {
                rec_sym.and_then(|s| {
                    let (x0, y0, x1, y1) = s.bbox;
                    ((x1 - x0).abs() > 1e-9 || (y1 - y0).abs() > 1e-9).then_some(BBox2 {
                        min_x: x0.min(x1),
                        min_y: y0.min(y1),
                        max_x: x0.max(x1),
                        max_y: y0.max(y1),
                    })
                })
            })
            .or_else(|| fallback_cb.map(|(_, b)| b))
            .unwrap_or(BBox2 {
                min_x: center.0 - 5.0 * upm,
                min_y: center.1 - 5.0 * upm,
                max_x: center.0 + 5.0 * upm,
                max_y: center.1 + 5.0 * upm,
            });
        let cls = pmap
            .get("_sppid_class")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&auth.type_code);
        let src_blk = match ent {
            EntityType::Insert(ins) => ins.block_name.as_str(),
            _ => pmap
                .get("_source_block")
                .and_then(Value::as_str)
                .unwrap_or(""),
        };
        let sym_path = pmap
            .get("_symbol_path")
            .and_then(Value::as_str)
            .unwrap_or("");
        let desc_up = format!("{src_blk} {sym_path}").to_ascii_uppercase();
        let tag_opt = if !auth.tag.trim().is_empty() {
            Some(auth.tag.trim().to_string())
        } else {
            pmap.get("_to_from_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let kind = classify_ll_kind(cls, &auth.type_code, &desc_up, tag_opt.as_deref());
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }
        if ports.is_empty() {
            if let Some(s) = rec_sym {
                ports.extend_from_slice(&s.ports);
            }
        }

        nodes.push(LlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            raw_label: tag_opt,
            parsed_line: None,
            primary_handle: h,
            member_handles: members,
            center,
            bbox,
            ports,
            spans: Vec::new(),
        });
    }

    let topo = TopologyTable::load(doc);
    let run_to_line: BTreeMap<String, String> = topo
        .pipe_runs
        .iter()
        .map(|r| (r.tag.clone(), r.line_tag.clone()))
        .collect();
    for seg in &topo.pipe_segments {
        let Some(sh) = parse_handle(&seg.handle) else {
            continue;
        };
        let lt = run_to_line
            .get(&seg.run_tag)
            .map(|s| s.trim_end_matches('-').to_string())
            .filter(|s| !s.is_empty());
        if let Some(indices) = pipe_indices_by_h.get(&sh) {
            for &idx in indices {
                if nodes[idx].parsed_line.is_none() {
                    let sp = nodes[idx].spans[0];
                    let resolved = resolve_span_lbl(sp, lt.as_deref());
                    nodes[idx].parsed_line = resolved.as_deref().and_then(parse_sppid_line_number);
                    nodes[idx].raw_label = resolved;
                }
            }
        } else {
            let sp = Span2 {
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            };
            let resolved = resolve_span_lbl(sp, lt.as_deref());
            let parsed = resolved.as_deref().and_then(parse_sppid_line_number);
            let (center, bbox) = spans_bbox_and_center(&[sp]);
            let idx = nodes.len();
            nodes.push(LlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: LlKind::PipeSegment,
                raw_label: resolved,
                parsed_line: parsed,
                primary_handle: sh,
                member_handles: vec![sh],
                center,
                bbox,
                ports: Vec::new(),
                spans: vec![sp],
            });
            pipe_indices_by_h.entry(sh).or_default().push(idx);
        }
    }

    if let Some(ref rec) = rec_opt {
        for sym in &rec.symbols {
            let Some(raw_h) = sym
                .entity_handle
                .or_else(|| sym.member_handles.first().copied())
            else {
                continue;
            };
            let primary_h = Handle::new(raw_h);
            if promoted_sym_handles.contains(&primary_h)
                || pipe_indices_by_h.contains_key(&primary_h)
                || sym
                    .member_handles
                    .iter()
                    .any(|&mr| promoted_sym_handles.contains(&Handle::new(mr)))
            {
                continue;
            }
            let mut members = vec![primary_h];
            for &mr in sym.member_handles.iter().chain(sym.tag_handles.iter()) {
                let mh = Handle::new(mr);
                if !members.contains(&mh) {
                    members.push(mh);
                }
            }
            let (x0, y0, x1, y1) = sym.bbox;
            let bbox = if (x1 - x0).abs() > 1e-9 || (y1 - y0).abs() > 1e-9 {
                BBox2 {
                    min_x: x0.min(x1),
                    min_y: y0.min(y1),
                    max_x: x0.max(x1),
                    max_y: y0.max(y1),
                }
            } else {
                entity_center_and_bbox(doc, primary_h, upm)
                    .map(|(_, b)| b)
                    .unwrap_or(BBox2 {
                        min_x: sym.at.0 - 5.0 * upm,
                        min_y: sym.at.1 - 5.0 * upm,
                        max_x: sym.at.0 + 5.0 * upm,
                        max_y: sym.at.1 + 5.0 * upm,
                    })
            };
            let tag_opt = sym
                .tag
                .as_deref()
                .or(sym.to_from_text.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let desc_up = format!(
                "{} {} {}",
                sym.source_block,
                sym.layer,
                sym.symbol_path.as_deref().unwrap_or("")
            )
            .to_ascii_uppercase();
            let kind = classify_ll_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref());
            nodes.push(LlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind,
                raw_label: tag_opt,
                parsed_line: None,
                primary_handle: primary_h,
                member_handles: members,
                center: sym.at,
                bbox,
                ports: sym.ports.clone(),
                spans: Vec::new(),
            });
        }

        for ps in &rec.pipe_segments {
            let Some(rh) = ps.entity_handle else {
                continue;
            };
            let h = Handle::new(rh);
            if promoted_sym_handles.contains(&h) || pipe_indices_by_h.contains_key(&h) {
                continue;
            }
            let sp = Span2 {
                a: ps.start,
                b: ps.end,
            };
            let lbl_opt = resolve_span_lbl(sp, None);
            let parsed = lbl_opt.as_deref().and_then(parse_sppid_line_number);
            let (center, bbox) = spans_bbox_and_center(&[sp]);
            nodes.push(LlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: LlKind::PipeSegment,
                raw_label: lbl_opt,
                parsed_line: parsed,
                primary_handle: h,
                member_handles: vec![h],
                center,
                bbox,
                ports: Vec::new(),
                spans: vec![sp],
            });
        }
    }
}

fn extract_unpromoted_ll_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<LlNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_ll_nodes(tab_index, drawing_code, &scratch, nodes);
        return;
    }

    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );

    for sym in &rec.symbols {
        let Some(raw_h) = sym
            .entity_handle
            .or_else(|| sym.member_handles.first().copied())
        else {
            continue;
        };
        let primary_h = Handle::new(raw_h);
        let mut members = vec![primary_h];
        for &mr in sym.member_handles.iter().chain(sym.tag_handles.iter()) {
            let mh = Handle::new(mr);
            if !members.contains(&mh) {
                members.push(mh);
            }
        }
        let (x0, y0, x1, y1) = sym.bbox;
        let bbox = if (x1 - x0).abs() > 1e-9 || (y1 - y0).abs() > 1e-9 {
            BBox2 {
                min_x: x0.min(x1),
                min_y: y0.min(y1),
                max_x: x0.max(x1),
                max_y: y0.max(y1),
            }
        } else {
            entity_center_and_bbox(doc, primary_h, upm)
                .map(|(_, b)| b)
                .unwrap_or(BBox2 {
                    min_x: sym.at.0 - 5.0 * upm,
                    min_y: sym.at.1 - 5.0 * upm,
                    max_x: sym.at.0 + 5.0 * upm,
                    max_y: sym.at.1 + 5.0 * upm,
                })
        };
        let tag_opt = sym
            .tag
            .as_deref()
            .or(sym.to_from_text.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let desc_up = format!(
            "{} {} {}",
            sym.source_block,
            sym.layer,
            sym.symbol_path.as_deref().unwrap_or("")
        )
        .to_ascii_uppercase();
        let kind = classify_ll_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref());

        nodes.push(LlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            raw_label: tag_opt,
            parsed_line: None,
            primary_handle: primary_h,
            member_handles: members,
            center: sym.at,
            bbox,
            ports: sym.ports.clone(),
            spans: Vec::new(),
        });
    }

    for ps in &rec.pipe_segments {
        let Some(rh) = ps.entity_handle else {
            continue;
        };
        let sp = Span2 {
            a: ps.start,
            b: ps.end,
        };
        let mut best_lbl: Option<(f64, &str)> = None;
        for lbl in &rec.pipe_labels {
            let d = point_to_span_dist(lbl.at, sp);
            if d <= 36.0 * upm && best_lbl.is_none_or(|(bd, _)| d < bd) {
                best_lbl = Some((d, lbl.raw_text.as_str()));
            }
        }
        let lbl_opt = best_lbl.map(|(_, t)| t.trim().trim_end_matches('-').to_string());
        let parsed = lbl_opt.as_deref().and_then(parse_sppid_line_number);
        let h = Handle::new(rh);
        let (center, bbox) = spans_bbox_and_center(&[sp]);
        nodes.push(LlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind: LlKind::PipeSegment,
            raw_label: lbl_opt,
            parsed_line: parsed,
            primary_handle: h,
            member_handles: vec![h],
            center,
            bbox,
            ports: Vec::new(),
            spans: vec![sp],
        });
    }
}

fn add_undirected_edge(adj: &mut [Vec<usize>], a: usize, b: usize) {
    if a == b {
        return;
    }
    if !adj[a].contains(&b) {
        adj[a].push(b);
    }
    if !adj[b].contains(&a) {
        adj[b].push(a);
    }
}

fn merge_opc_into_ll_sheet(
    c: &OpcConnectorItem,
    drawing_code: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<LlNode>,
) -> usize {
    let paired = c.status == OpcStatus::Paired;
    for idx in sheet_range {
        if nodes[idx].kind == LlKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = matches!(nodes[idx].kind, LlKind::Opc { .. })
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = LlKind::Opc { is_paired: paired };
            for &mh in &c.member_handles {
                if !nodes[idx].member_handles.contains(&mh) {
                    nodes[idx].member_handles.push(mh);
                }
            }
            if nodes[idx].raw_label.is_none() {
                nodes[idx].raw_label = c.target_dwg_raw.clone().or_else(|| c.line_label.clone());
            }
            return idx;
        }
    }

    let idx = nodes.len();
    nodes.push(LlNode {
        tab_index: c.tab_index,
        drawing_code: drawing_code.to_string(),
        kind: LlKind::Opc { is_paired: paired },
        raw_label: c.target_dwg_raw.clone().or_else(|| c.line_label.clone()),
        parsed_line: None,
        primary_handle: c.primary_handle,
        member_handles: c.member_handles.clone(),
        center: (c.x, c.y),
        bbox: BBox2 {
            min_x: c.x - 6.0 * upm,
            min_y: c.y - 6.0 * upm,
            max_x: c.x + 6.0 * upm,
            max_y: c.y + 6.0 * upm,
        },
        ports: vec![(c.x, c.y)],
        spans: Vec::new(),
    });
    idx
}

fn wire_ll_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[LlNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;
    let opc_tol = 14.0 * upm;
    let equip_tol = 16.0 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == LlKind::PipeSegment {
            pipe_nodes.push(idx);
        } else {
            sym_nodes.push(idx);
        }
    }

    for (pi, &na) in pipe_nodes.iter().enumerate() {
        for &nb in &pipe_nodes[(pi + 1)..] {
            if !bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, pipe_tol) {
                continue;
            }
            let mut connected = false;
            for &sa in &nodes[na].spans {
                for &sb in &nodes[nb].spans {
                    if spans_touch(sa, sb, tight_tol) {
                        connected = true;
                        break;
                    }
                    if spans_touch(sa, sb, pipe_tol) {
                        let pairs = [(sa.a, sb.a), (sa.a, sb.b), (sa.b, sb.a), (sa.b, sb.b)];
                        let blocked_by_sym = pairs
                            .iter()
                            .filter(|&&(p1, p2)| (p1.0 - p2.0).hypot(p1.1 - p2.1) <= pipe_tol * 1.5)
                            .any(|&(p1, p2)| {
                                let mid = ((p1.0 + p2.0) * 0.5, (p1.1 + p2.1) * 0.5);
                                sym_nodes.iter().any(|&sn| {
                                    !matches!(nodes[sn].kind, LlKind::Opc { .. })
                                        && point_in_bbox(mid, nodes[sn].bbox, 1.5 * upm)
                                })
                            });
                        if !blocked_by_sym {
                            connected = true;
                            break;
                        }
                    }
                }
                if connected {
                    break;
                }
            }
            if connected {
                add_undirected_edge(adj, na, nb);
            }
        }
    }

    for &pn in &pipe_nodes {
        for &sn in &sym_nodes {
            let margin = match nodes[sn].kind {
                LlKind::Equipment { .. } => 12.0 * upm,
                LlKind::Opc { .. } => opc_tol,
                _ => pipe_tol,
            };
            let hit_bbox = nodes[pn]
                .spans
                .iter()
                .any(|&sp| span_touches_bbox(sp, nodes[sn].bbox, margin));
            let hit_port = !hit_bbox
                && nodes[pn].spans.iter().any(|&sp| {
                    point_to_span_dist(nodes[sn].center, sp) <= sym_tol
                        || nodes[sn]
                            .ports
                            .iter()
                            .any(|&pt| point_to_span_dist(pt, sp) <= sym_tol)
                });
            if hit_bbox || hit_port {
                add_undirected_edge(adj, pn, sn);
            }
        }
    }

    for &sn in &sym_nodes {
        if matches!(nodes[sn].kind, LlKind::Opc { .. }) && adj[sn].is_empty() {
            let mut best: Option<(f64, usize)> = None;
            for &pn in &pipe_nodes {
                for &sp in &nodes[pn].spans {
                    let d = point_to_span_dist(nodes[sn].center, sp);
                    if d <= 42.0 * upm && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, pn));
                    }
                }
            }
            if let Some((_, pn)) = best {
                add_undirected_edge(adj, sn, pn);
            }
        }
    }

    for (si, &na) in sym_nodes.iter().enumerate() {
        for &nb in &sym_nodes[(si + 1)..] {
            let ka = nodes[na].kind;
            let kb = nodes[nb].kind;
            let eq_nozzle = (ka == LlKind::Nozzle && matches!(kb, LlKind::Equipment { .. }))
                || (kb == LlKind::Nozzle && matches!(ka, LlKind::Equipment { .. }));
            if eq_nozzle && bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, equip_tol) {
                add_undirected_edge(adj, na, nb);
            }
        }
    }
}

fn propagate_ll_pipe_labels(graph: &mut LlGraph) {
    let mut updates: Vec<(usize, String, ParsedLineNumber)> = Vec::new();
    for idx in 0..graph.nodes.len() {
        if graph.nodes[idx].kind != LlKind::PipeSegment || graph.nodes[idx].parsed_line.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeMap<String, (String, ParsedLineNumber)> = BTreeMap::new();

        while let Some(cur) = q.pop_front() {
            if graph.nodes[cur].kind == LlKind::PipeSegment {
                let parsed_opt = graph.nodes[cur].parsed_line.as_ref();
                if let Some(parsed) = parsed_opt {
                    let raw = graph.nodes[cur].raw_label.clone().unwrap_or_default();
                    found.insert(parsed.pipeline_name(), (raw, parsed.clone()));
                    continue;
                }
            }
            if matches!(
                graph.nodes[cur].kind,
                LlKind::Equipment { .. } | LlKind::Opc { is_paired: false }
            ) {
                continue;
            }
            for &nxt in &graph.adj[cur] {
                if visited.len() < 18 && visited.insert(nxt) {
                    q.push_back(nxt);
                }
            }
        }
        let only_opt = (found.len() == 1)
            .then(|| found.into_values().next())
            .flatten();
        if let Some((raw, parsed)) = only_opt {
            updates.push((idx, raw, parsed));
        }
    }
    for (idx, raw, parsed) in updates {
        graph.nodes[idx].raw_label = Some(raw);
        graph.nodes[idx].parsed_line = Some(parsed);
    }
}

fn build_ll_graph(sheets: &[OpcSheetInput<'_>]) -> LlGraph {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut tab_drawings: BTreeMap<usize, String> = BTreeMap::new();
    for c in &opc_rep.connectors {
        tab_drawings
            .entry(c.tab_index)
            .or_insert_with(|| c.drawing_code.clone());
    }
    let mut nodes: Vec<LlNode> = Vec::new();
    let mut sheet_ranges: Vec<(f64, std::ops::Range<usize>)> = Vec::new();
    let mut opc_lookup: BTreeMap<(usize, Handle), usize> = BTreeMap::new();

    for sh in sheets {
        let dwg_code = tab_drawings
            .entry(sh.tab_index)
            .or_insert_with(|| clean_drawing_stem(sh.fallback_name))
            .clone();
        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_ll_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        } else {
            extract_unpromoted_ll_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        }
        let base_end = nodes.len();
        for c in opc_rep
            .connectors
            .iter()
            .filter(|c| c.tab_index == sh.tab_index)
        {
            let opc_idx =
                merge_opc_into_ll_sheet(c, &dwg_code, upm, start_idx..base_end, &mut nodes);
            opc_lookup.insert((c.tab_index, c.primary_handle), opc_idx);
        }
        let final_end = nodes.len();
        sheet_ranges.push((upm, start_idx..final_end));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_ll_local_edges(&indices, *upm, &nodes, &mut adj);
    }

    for c in &opc_rep.connectors {
        if c.status != OpcStatus::Paired {
            continue;
        }
        let Some(ref peer) = c.peer else {
            continue;
        };
        let Some(&na) = opc_lookup.get(&(c.tab_index, c.primary_handle)) else {
            continue;
        };
        let Some(&nb) = opc_lookup.get(&(peer.tab_index, peer.primary_handle)) else {
            continue;
        };
        add_undirected_edge(&mut adj, na, nb);
    }

    let mut graph = LlGraph { nodes, adj };
    propagate_ll_pipe_labels(&mut graph);
    graph
}

fn push_unique_handles(dst: &mut Vec<Handle>, src: &[Handle]) {
    for &h in src {
        if !dst.contains(&h) {
            dst.push(h);
        }
    }
}

/// Aggregate all labeled pipelines across `sheets`, trace cross-sheet `From -> To` endpoints,
/// count inline valves & specialties, and flag unterminated lines (`#167`).
pub fn analyze_pid_line_list(sheets: &[OpcSheetInput<'_>]) -> PidLineListReport {
    let graph = build_ll_graph(sheets);

    let mut group_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, nd) in graph.nodes.iter().enumerate() {
        if nd.kind != LlKind::PipeSegment {
            continue;
        }
        if let Some(ref parsed) = nd.parsed_line {
            group_map
                .entry(parsed.pipeline_name())
                .or_default()
                .push(idx);
        }
    }

    let mut out_lines: Vec<PidPipelineRow> = Vec::with_capacity(group_map.len());
    let mut total_inline_valves = 0usize;

    for (pname, seg_indices) in group_map {
        let lead_idx = seg_indices[0];
        let lead_nd = &graph.nodes[lead_idx];
        let parsed = lead_nd.parsed_line.clone().unwrap();

        let seg_set: BTreeSet<usize> = seg_indices.iter().copied().collect();
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        for &si in &seg_indices {
            if visited.insert(si) {
                q.push_back(si);
            }
        }

        // Track `(priority_rank, tab_index, x_coord_bits, label)` for deterministic From -> To ordering
        // Rank 0 = driver/pump equipment (natural From), Rank 1 = external OSBL OPC, Rank 2 = vessel/header
        let mut terminal_map: BTreeMap<String, (u8, usize, i64)> = BTreeMap::new();
        let mut valve_nodes: BTreeSet<usize> = BTreeSet::new();
        let mut spec_nodes: BTreeSet<usize> = BTreeSet::new();
        let mut dwg_set: BTreeSet<String> = BTreeSet::new();
        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();

        while let Some(cur) = q.pop_front() {
            let cnd = &graph.nodes[cur];
            dwg_set.insert(cnd.drawing_code.clone());
            push_unique_handles(
                handles_by_tab.entry(cnd.tab_index).or_default(),
                &cnd.member_handles,
            );

            if cnd.kind == LlKind::Valve {
                valve_nodes.insert(cur);
            } else if cnd.kind == LlKind::InlineSpecialtyOrInst {
                spec_nodes.insert(cur);
            }

            for &nxt in &graph.adj[cur] {
                let nnd = &graph.nodes[nxt];
                match nnd.kind {
                    LlKind::Equipment { is_driver_or_pump } => {
                        let etag = nnd
                            .raw_label
                            .clone()
                            .unwrap_or_else(|| format!("EQUIP@{:X}", nnd.primary_handle.value()));
                        let rank = if is_driver_or_pump { 0 } else { 2 };
                        terminal_map.entry(etag).or_insert((
                            rank,
                            nnd.tab_index,
                            (nnd.center.0 * 10.0) as i64,
                        ));
                    }
                    LlKind::Opc { is_paired: false } => {
                        let otag = nnd
                            .raw_label
                            .as_deref()
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| format!("OPC:{s}"))
                            .unwrap_or_else(|| format!("OPC:{}", nnd.drawing_code));
                        terminal_map.entry(otag).or_insert((
                            1,
                            nnd.tab_index,
                            (nnd.center.0 * 10.0) as i64,
                        ));
                    }
                    LlKind::PipeSegment => {
                        if seg_set.contains(&nxt) {
                            if visited.insert(nxt) {
                                q.push_back(nxt);
                            }
                        } else if let Some(ref other_p) = nnd.parsed_line {
                            let htag = format!("HDR:{}", other_p.pipeline_name());
                            terminal_map.entry(htag).or_insert((
                                2,
                                nnd.tab_index,
                                (nnd.center.0 * 10.0) as i64,
                            ));
                        } else if visited.insert(nxt) {
                            q.push_back(nxt);
                        }
                    }
                    LlKind::Valve
                    | LlKind::InlineSpecialtyOrInst
                    | LlKind::Nozzle
                    | LlKind::Opc { is_paired: true } => {
                        if visited.insert(nxt) {
                            q.push_back(nxt);
                        }
                    }
                }
            }
        }

        let mut sorted_terms: Vec<(u8, usize, i64, String)> = terminal_map
            .into_iter()
            .map(|(lbl, (rk, ti, x))| (rk, ti, x, lbl))
            .collect();
        sorted_terms.sort();

        let (from_endpoint, to_endpoint, status) = if sorted_terms.len() >= 2 {
            (
                sorted_terms[0].3.clone(),
                sorted_terms[1].3.clone(),
                LineTerminationStatus::BothTerminated,
            )
        } else if sorted_terms.len() == 1 {
            (
                sorted_terms[0].3.clone(),
                "OPEN_OR_BRANCH".to_string(),
                LineTerminationStatus::SingleEnded,
            )
        } else {
            (
                String::new(),
                String::new(),
                LineTerminationStatus::Unterminated,
            )
        };

        total_inline_valves += valve_nodes.len();
        out_lines.push(PidPipelineRow {
            line_number: pname,
            nominal_diameter: parsed.nominal_diameter_attr,
            fluid_code: parsed.fluid_code.to_ascii_uppercase(),
            seq_no: parsed.seq_no,
            piping_spec: parsed.materials_class,
            insulation: parsed.insulation.unwrap_or_default(),
            from_endpoint,
            to_endpoint,
            status,
            primary_tab: lead_nd.tab_index,
            drawing_codes: dwg_set.into_iter().collect(),
            segment_count: seg_indices.len(),
            inline_valves_count: valve_nodes.len(),
            inline_specialties_count: spec_nodes.len(),
            x: lead_nd.center.0,
            y: lead_nd.center.1,
            primary_handle: lead_nd.primary_handle,
            handles_by_tab,
        });
    }

    let multi_sheet_pipelines = out_lines
        .iter()
        .filter(|l| l.drawing_codes.len() > 1)
        .count();
    let both_terminated_count = out_lines
        .iter()
        .filter(|l| l.status == LineTerminationStatus::BothTerminated)
        .count();
    let single_ended_count = out_lines
        .iter()
        .filter(|l| l.status == LineTerminationStatus::SingleEnded)
        .count();
    let unterminated_count = out_lines
        .iter()
        .filter(|l| l.status == LineTerminationStatus::Unterminated)
        .count();

    PidLineListReport {
        summary: PidLineListSummary {
            sheets_scanned: sheets.len(),
            total_pipelines: out_lines.len(),
            multi_sheet_pipelines,
            both_terminated_count,
            single_ended_count,
            unterminated_count,
            total_inline_valves,
        },
        pipelines: out_lines,
    }
}

/// Match whether `row` matches user `query` (by line number, sequence number, or endpoint tag).
pub fn pipeline_row_matches_query(row: &PidPipelineRow, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    let qu = q.to_ascii_uppercase();
    row.line_number.to_ascii_uppercase().contains(&qu)
        || row.seq_no.to_ascii_uppercase().contains(&qu)
        || match_query_rank(&row.line_number, q).is_some()
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and build a Master Line List (`#167`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_lines_from_path(input_path: &Path) -> Result<PidLineListReport, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    if input_path.is_file() {
        files.push(input_path.to_path_buf());
    } else if input_path.is_dir() {
        let rd = std::fs::read_dir(input_path)
            .map_err(|e| format!("read dir {}: {e}", input_path.display()))?;
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "dxf" | "dwg" | "pid")
                && std::fs::metadata(&p).is_ok_and(|m| m.len() > 0)
            {
                files.push(p);
            }
        }
        files.sort();
    } else {
        return Err(format!(
            "input path does not exist: {}",
            input_path.display()
        ));
    }

    if files.is_empty() {
        return Err(format!(
            "no non-empty .dxf/.dwg/.pid sheets found in {}",
            input_path.display()
        ));
    }

    let mut loaded: Vec<(String, Document)> = Vec::with_capacity(files.len());
    for fp in &files {
        let doc = crate::io::load_file(fp).map_err(|e| format!("load {}: {e}", fp.display()))?;
        let nm = fp
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SHEET")
            .to_string();
        loaded.push((nm, doc));
    }

    let inputs: Vec<OpcSheetInput<'_>> = loaded
        .iter()
        .enumerate()
        .map(|(idx, (nm, doc))| OpcSheetInput {
            tab_index: idx,
            fallback_name: nm.as_str(),
            document: doc,
        })
        .collect();
    Ok(analyze_pid_line_list(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidLineListReport` as delimited Master Line List text with UTF-8 BOM.
pub fn format_pid_line_list_delimited(report: &PidLineListReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = LINE_LIST_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for r in &report.pipelines {
        let dwgs = r.drawing_codes.join(";");
        let vc = r.inline_valves_count.to_string();
        let sc = r.segment_count.to_string();
        let cells = [
            r.line_number.as_str(),
            r.nominal_diameter.as_str(),
            r.fluid_code.as_str(),
            r.seq_no.as_str(),
            r.piping_spec.as_str(),
            r.insulation.as_str(),
            r.from_endpoint.as_str(),
            r.to_endpoint.as_str(),
            r.status.as_str(),
            dwgs.as_str(),
            vc.as_str(),
            sc.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidLineListReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#167`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_line_list_file(report: &PidLineListReport, out_path: &Path) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let ext = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("json")
        .to_ascii_lowercase();
    let payload = match ext.as_str() {
        "json" => format!(
            "{}\n",
            serde_json::to_string_pretty(report)
                .map_err(|e| format!("serialize line list JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_line_list_delimited(report, '\t'),
        "csv" => format_pid_line_list_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported line list extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

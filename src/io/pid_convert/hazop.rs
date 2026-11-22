//! Multi-sheet P&ID automated HAZOP review node partitioner, equipment-pipeline-safeguard
//! aggregator, and IEC 61882 worksheet exporter (`PIDHAZOP` & `--export-hazop`,
//! `L3-3` / `L3-4` / `L3-5` / `L3-6` / `L6-2` / `#160`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::external_props::normalize_lookup_key;
use super::find::match_query_rank;
use super::loops::{analyze_pid_loops, parse_isa_instrument_tag};
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Canonical 11-column header order shared by CSV and TSV HAZOP Node & Safeguard Worksheets.
pub const HAZOP_CSV_HEADERS: [&str; 11] = [
    "NodeId",
    "EquipmentTag",
    "EquipmentClass",
    "PrimaryDrawing",
    "SpanDrawings",
    "SafeguardStatus",
    "ConnectedPipelines",
    "Valves",
    "SafeguardInstruments",
    "SafeguardLoops",
    "ApplicableGuidewords",
];

/// Safeguard coverage status of an automated HAZOP equipment review node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HazopSafeguardStatus {
    Protected,
    Unprotected,
}

impl HazopSafeguardStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Protected => "protected",
            Self::Unprotected => "unprotected",
        }
    }
}

/// One equipment-centric HAZOP review node with aggregated lines, valves, and ISA-5.1 safeguards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidHazopNode {
    pub node_id: String,
    pub tab_index: usize,
    pub primary_drawing: String,
    pub span_drawings: Vec<String>,
    pub equipment_tag: String,
    pub equipment_class: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    pub status: HazopSafeguardStatus,
    pub pipe_segments_count: usize,
    pub connected_pipelines: Vec<String>,
    pub valves: Vec<String>,
    pub safeguard_instruments: Vec<String>,
    pub safeguard_loops: Vec<String>,
    pub applicable_guidewords: Vec<String>,
    pub diagnostic_note: String,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Summary statistics across all partitioned HAZOP review nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidHazopSummary {
    pub sheets_scanned: usize,
    pub total_nodes: usize,
    pub multi_sheet_nodes: usize,
    pub protected_nodes: usize,
    pub unprotected_nodes: usize,
    pub total_linked_pipelines: usize,
    pub total_linked_safeguards: usize,
}

/// Complete HAZOP Node & Safeguard Worksheet report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidHazopReport {
    pub summary: PidHazopSummary,
    pub nodes: Vec<PidHazopNode>,
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
enum HzKind {
    Equipment,
    Nozzle,
    Valve,
    ReliefDevice,
    Instrument,
    PassiveFitting,
    Opc,
    PipeSegment,
}

struct HzNode {
    tab_index: usize,
    drawing_name: String,
    kind: HzKind,
    sppid_class: String,
    tag_or_label: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct HzGraph {
    nodes: Vec<HzNode>,
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

fn is_passive_fitting(blk_upper: &str) -> bool {
    blk_upper.contains("REDUCER")
        || blk_upper.contains("FLANGE")
        || blk_upper.contains("CAP")
        || blk_upper.contains("SPACER")
        || blk_upper.contains("TRAP")
        || blk_upper.contains("STRAINER")
        || blk_upper.contains("FUNNEL")
        || blk_upper.contains("HOSE")
        || blk_upper.contains("SIGHT")
        || blk_upper.contains("ARRESTER")
}

fn is_relief_device(blk_upper: &str, tag_opt: Option<&str>) -> bool {
    if blk_upper.contains("RELIEF") || blk_upper.contains("RUPTURE") || blk_upper.contains("SAFETY")
    {
        return true;
    }
    let Some(t) = tag_opt else {
        return false;
    };
    let letters: String = t
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(letters.as_str(), "PSV" | "PRV" | "PSE" | "TSV" | "RD")
}

fn classify_hz_kind(cls: &str, type_code: &str, blk_upper: &str, tag_opt: Option<&str>) -> HzKind {
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        HzKind::Opc
    } else if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        HzKind::Nozzle
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        HzKind::Equipment
    } else if is_relief_device(blk_upper, tag_opt) {
        HzKind::ReliefDevice
    } else if matches!(
        cls,
        "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
    ) || type_code.eq_ignore_ascii_case("instrument")
    {
        HzKind::Instrument
    } else if cls == "PIDPipingComponent" || type_code.eq_ignore_ascii_case("valve") {
        if is_passive_fitting(blk_upper) {
            HzKind::PassiveFitting
        } else {
            HzKind::Valve
        }
    } else {
        HzKind::PassiveFitting
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
        EntityType::Text(t) => {
            let (x, y) = (t.insertion_point.x, t.insertion_point.y);
            Some((
                (x, y),
                BBox2 {
                    min_x: x - 2.0 * upm,
                    min_y: y - 2.0 * upm,
                    max_x: x + 2.0 * upm,
                    max_y: y + 2.0 * upm,
                },
            ))
        }
        EntityType::MText(mt) => {
            let (x, y) = (mt.insertion_point.x, mt.insertion_point.y);
            Some((
                (x, y),
                BBox2 {
                    min_x: x - 2.0 * upm,
                    min_y: y - 2.0 * upm,
                    max_x: x + 2.0 * upm,
                    max_y: y + 2.0 * upm,
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

fn extract_authored_hz_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<HzNode>,
) {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let mut owned_handles: BTreeSet<Handle> = BTreeSet::new();
    let mut pipe_by_handle: BTreeMap<Handle, usize> = BTreeMap::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        owned_handles.insert(h);
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            let lbl = pmap
                .get("_line_label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_end_matches('-').to_string());
            let mut spans = Vec::new();
            if let Some(raw_segs) = pmap.get("_segments").and_then(Value::as_str) {
                for entry in raw_segs.split(';').filter(|s| !s.trim().is_empty()) {
                    let c_str = entry.split('|').next().unwrap_or(entry);
                    if let Some(bb) = parse_bbox_str(c_str) {
                        spans.push(Span2 {
                            a: (bb.min_x, bb.min_y),
                            b: (bb.max_x, bb.max_y),
                        });
                    }
                }
            }
            if spans.is_empty() {
                if let EntityType::Line(ln) = ent {
                    spans.push(Span2 {
                        a: (ln.start.x, ln.start.y),
                        b: (ln.end.x, ln.end.y),
                    });
                } else if let EntityType::LwPolyline(pl) = ent {
                    for w in pl.vertices.windows(2) {
                        spans.push(Span2 {
                            a: (w[0].location.x, w[0].location.y),
                            b: (w[1].location.x, w[1].location.y),
                        });
                    }
                }
            }
            if !spans.is_empty() {
                let (center, bbox) = spans_bbox_and_center(&spans);
                let idx = nodes.len();
                nodes.push(HzNode {
                    tab_index,
                    drawing_name: drawing_name.to_string(),
                    kind: HzKind::PipeSegment,
                    sppid_class: "PIDPipingNetworkSegment".to_string(),
                    tag_or_label: lbl,
                    primary_handle: h,
                    member_handles: vec![h],
                    center,
                    bbox,
                    ports: Vec::new(),
                    spans,
                });
                pipe_by_handle.insert(h, idx);
            }
            continue;
        }

        let mut members = vec![h];
        for key in ["_member_handles", "_tag_handles"] {
            if let Some(s) = pmap.get(key).and_then(Value::as_str) {
                for mh in parse_csv_handles(s) {
                    owned_handles.insert(mh);
                    if !members.contains(&mh) {
                        members.push(mh);
                    }
                }
            }
        }
        let fallback_cb = entity_center_and_bbox(doc, h, upm);
        let center = pmap
            .get("_at")
            .and_then(Value::as_str)
            .and_then(parse_pair_f64)
            .or_else(|| fallback_cb.map(|(c, _)| c))
            .unwrap_or((0.0, 0.0));
        let bbox = pmap
            .get("_bbox")
            .and_then(Value::as_str)
            .and_then(parse_bbox_str)
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
            .unwrap_or(&auth.type_code)
            .to_string();
        let blk_upper = match ent {
            EntityType::Insert(ins) => ins.block_name.to_ascii_uppercase(),
            _ => pmap
                .get("_source_block")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_uppercase(),
        };
        let tag_opt = if !auth.tag.trim().is_empty() {
            Some(auth.tag.trim().to_string())
        } else {
            pmap.get("_to_from_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let kind = classify_hz_kind(&cls, &auth.type_code, &blk_upper, tag_opt.as_deref());
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(HzNode {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind,
            sppid_class: cls,
            tag_or_label: tag_opt,
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
        if let Some(&idx) = pipe_by_handle.get(&sh) {
            if nodes[idx].tag_or_label.is_none() {
                nodes[idx].tag_or_label = lt;
            }
        } else {
            owned_handles.insert(sh);
            let sp = Span2 {
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            };
            let (center, bbox) = spans_bbox_and_center(&[sp]);
            let idx = nodes.len();
            nodes.push(HzNode {
                tab_index,
                drawing_name: drawing_name.to_string(),
                kind: HzKind::PipeSegment,
                sppid_class: "PIDPipingNetworkSegment".to_string(),
                tag_or_label: lt,
                primary_handle: sh,
                member_handles: vec![sh],
                center,
                bbox,
                ports: Vec::new(),
                spans: vec![sp],
            });
            pipe_by_handle.insert(sh, idx);
        }
    }

    if pipe_by_handle.is_empty() {
        for ent in doc.model_space_entities() {
            let eh = ent.common().handle;
            if owned_handles.contains(&eh) {
                continue;
            }
            let layer_lc = ent.common().layer.to_ascii_lowercase();
            if matches!(
                layer_lc.as_str(),
                "default"
                    | "drawingborder"
                    | "notes"
                    | "labels"
                    | "title"
                    | "border"
                    | "hiddenobjects"
            ) {
                continue;
            }
            if let EntityType::Line(ln) = ent {
                let len = (ln.start.x - ln.end.x).hypot(ln.start.y - ln.end.y);
                if len >= 2.0 * upm && len <= 800.0 * upm {
                    let sp = Span2 {
                        a: (ln.start.x, ln.start.y),
                        b: (ln.end.x, ln.end.y),
                    };
                    let (center, bbox) = spans_bbox_and_center(&[sp]);
                    nodes.push(HzNode {
                        tab_index,
                        drawing_name: drawing_name.to_string(),
                        kind: HzKind::PipeSegment,
                        sppid_class: "PIDPipingNetworkSegment".to_string(),
                        tag_or_label: None,
                        primary_handle: eh,
                        member_handles: vec![eh],
                        center,
                        bbox,
                        ports: Vec::new(),
                        spans: vec![sp],
                    });
                }
            }
        }
    }
}

fn extract_unpromoted_hz_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<HzNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_hz_nodes(tab_index, drawing_name, &scratch, nodes);
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
        let kind = classify_hz_kind(
            &sym.sppid_class,
            "",
            &sym.source_block.to_ascii_uppercase(),
            tag_opt.as_deref(),
        );

        nodes.push(HzNode {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind,
            sppid_class: sym.sppid_class.clone(),
            tag_or_label: tag_opt,
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
        let h = Handle::new(rh);
        let (center, bbox) = spans_bbox_and_center(&[sp]);
        nodes.push(HzNode {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: HzKind::PipeSegment,
            sppid_class: "PIDPipingNetworkSegment".to_string(),
            tag_or_label: lbl_opt,
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

fn merge_opc_into_hz_sheet(
    c: &OpcConnectorItem,
    drawing_name: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<HzNode>,
) -> usize {
    for idx in sheet_range {
        if nodes[idx].kind == HzKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = nodes[idx].kind == HzKind::Opc
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = HzKind::Opc;
            for &mh in &c.member_handles {
                if !nodes[idx].member_handles.contains(&mh) {
                    nodes[idx].member_handles.push(mh);
                }
            }
            if nodes[idx].tag_or_label.is_none() {
                nodes[idx].tag_or_label = c.target_dwg_raw.clone().or_else(|| c.line_label.clone());
            }
            return idx;
        }
    }

    let idx = nodes.len();
    nodes.push(HzNode {
        tab_index: c.tab_index,
        drawing_name: drawing_name.to_string(),
        kind: HzKind::Opc,
        sppid_class: "PIDBranchPoint".to_string(),
        tag_or_label: c.target_dwg_raw.clone().or_else(|| c.line_label.clone()),
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

fn wire_hz_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[HzNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;
    let inst_pipe_tol = 10.0 * upm;
    let opc_tol = 14.0 * upm;
    let equip_tol = 16.0 * upm;
    let equip_inst_tol = 12.0 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == HzKind::PipeSegment {
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
                                    nodes[sn].kind != HzKind::Opc
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
                HzKind::Equipment => 12.0 * upm,
                HzKind::Opc => opc_tol,
                HzKind::Instrument | HzKind::ReliefDevice => inst_pipe_tol,
                _ => pipe_tol,
            };
            let port_tol = match nodes[sn].kind {
                HzKind::Instrument | HzKind::ReliefDevice => inst_pipe_tol,
                _ => sym_tol,
            };
            let hit_bbox = nodes[pn]
                .spans
                .iter()
                .any(|&sp| span_touches_bbox(sp, nodes[sn].bbox, margin));
            let hit_port = !hit_bbox
                && nodes[pn].spans.iter().any(|&sp| {
                    point_to_span_dist(nodes[sn].center, sp) <= port_tol
                        || nodes[sn]
                            .ports
                            .iter()
                            .any(|&pt| point_to_span_dist(pt, sp) <= port_tol)
                });
            if hit_bbox || hit_port {
                add_undirected_edge(adj, pn, sn);
            }
        }
    }

    for &sn in &sym_nodes {
        if nodes[sn].kind == HzKind::Opc && adj[sn].is_empty() {
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
            let connected = if (ka == HzKind::Nozzle && kb == HzKind::Equipment)
                || (kb == HzKind::Nozzle && ka == HzKind::Equipment)
            {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, equip_tol)
            } else if (ka == HzKind::Equipment
                && matches!(kb, HzKind::Instrument | HzKind::ReliefDevice))
                || (kb == HzKind::Equipment
                    && matches!(ka, HzKind::Instrument | HzKind::ReliefDevice))
            {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, equip_inst_tol)
            } else if ka != HzKind::Equipment && kb != HzKind::Equipment {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, 4.0 * upm)
            } else {
                false
            };
            if connected {
                add_undirected_edge(adj, na, nb);
            }
        }
    }
}

fn build_hz_graph(sheets: &[OpcSheetInput<'_>]) -> HzGraph {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut tab_drawings: BTreeMap<usize, String> = BTreeMap::new();
    for c in &opc_rep.connectors {
        tab_drawings
            .entry(c.tab_index)
            .or_insert_with(|| c.drawing_code.clone());
    }
    let mut nodes: Vec<HzNode> = Vec::new();
    let mut sheet_ranges: Vec<(usize, f64, std::ops::Range<usize>)> = Vec::new();
    let mut opc_lookup: BTreeMap<(usize, Handle), usize> = BTreeMap::new();

    for sh in sheets {
        let fallback_clean = sh
            .fallback_name
            .strip_suffix(".dxf")
            .or_else(|| sh.fallback_name.strip_suffix(".DXF"))
            .or_else(|| sh.fallback_name.strip_suffix(".dwg"))
            .or_else(|| sh.fallback_name.strip_suffix(".DWG"))
            .or_else(|| sh.fallback_name.strip_suffix(".pid"))
            .or_else(|| sh.fallback_name.strip_suffix(".PID"))
            .unwrap_or(sh.fallback_name)
            .to_string();
        let dwg_code = tab_drawings
            .entry(sh.tab_index)
            .or_insert(fallback_clean)
            .clone();

        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_hz_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        } else {
            extract_unpromoted_hz_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        }
        let base_end = nodes.len();
        for c in opc_rep
            .connectors
            .iter()
            .filter(|c| c.tab_index == sh.tab_index)
        {
            let opc_idx =
                merge_opc_into_hz_sheet(c, &dwg_code, upm, start_idx..base_end, &mut nodes);
            opc_lookup.insert((c.tab_index, c.primary_handle), opc_idx);
        }
        let final_end = nodes.len();
        sheet_ranges.push((sh.tab_index, upm, start_idx..final_end));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (_, upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_hz_local_edges(&indices, *upm, &nodes, &mut adj);
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

    HzGraph { nodes, adj }
}

fn derive_guidewords(equip_class: &str, measured_vars: &BTreeSet<String>) -> Vec<String> {
    let mut gw = vec![
        "No/Less/More Flow".to_string(),
        "Reverse Flow".to_string(),
        "High/Low Pressure".to_string(),
    ];
    if equip_class == "PIDProcessVessel" || measured_vars.contains("L") {
        gw.push("High/Low Level".to_string());
    }
    if matches!(
        equip_class,
        "PIDHeatTransferEquipment" | "PIDHeatTransferEquip"
    ) || measured_vars.contains("T")
    {
        gw.push("High/Low Temperature".to_string());
    }
    if equip_class == "PIDHeatTransferEquipment" || equip_class == "PIDHeatTransferEquip" {
        gw.push("Tube Rupture / Composition".to_string());
    }
    gw
}

fn push_unique_handles(dst: &mut Vec<Handle>, src: &[Handle]) {
    for &h in src {
        if !dst.contains(&h) {
            dst.push(h);
        }
    }
}

/// Partition open `sheets` into equipment-centric HAZOP review nodes, cross-reference ISA-5.1
/// safeguard loops, and flag unprotected process equipment nodes (`#160`).
pub fn analyze_pid_hazop_nodes(sheets: &[OpcSheetInput<'_>]) -> PidHazopReport {
    let graph = build_hz_graph(sheets);
    let loop_rep = analyze_pid_loops(sheets);

    let mut loops_by_num: BTreeMap<String, usize> = BTreeMap::new();
    for (li, lp) in loop_rep.loops.iter().enumerate() {
        loops_by_num.insert(lp.loop_number.to_ascii_uppercase(), li);
    }

    // Group equipment seeds by (tab_index, normalized_tag) when tagged, or keep untagged only if connected
    let mut grouped_seeds: BTreeMap<(usize, String), Vec<usize>> = BTreeMap::new();
    for (idx, nd) in graph.nodes.iter().enumerate() {
        if nd.kind != HzKind::Equipment {
            continue;
        }
        if let Some(ref t) = nd.tag_or_label {
            let nk = normalize_lookup_key(t);
            if !nk.is_empty() {
                grouped_seeds
                    .entry((nd.tab_index, format!("0:{nk}")))
                    .or_default()
                    .push(idx);
                continue;
            }
        }
        if !graph.adj[idx].is_empty() {
            grouped_seeds
                .entry((nd.tab_index, format!("1:{:08X}", nd.primary_handle.value())))
                .or_default()
                .push(idx);
        }
    }

    // Assign each nozzle to its owning equipment group so peer nozzles do not leak safeguards across vessels
    let mut seed_group_of_equip: BTreeMap<usize, usize> = BTreeMap::new();
    let seed_vec: Vec<Vec<usize>> = grouped_seeds.into_values().collect();
    for (gi, eq_indices) in seed_vec.iter().enumerate() {
        for &ei in eq_indices {
            seed_group_of_equip.insert(ei, gi);
        }
    }
    let mut nozzle_owner_group: BTreeMap<usize, usize> = BTreeMap::new();
    for (ni, nd) in graph.nodes.iter().enumerate() {
        if nd.kind != HzKind::Nozzle {
            continue;
        }
        let mut best: Option<(f64, usize)> = None;
        for &nb in &graph.adj[ni] {
            if graph.nodes[nb].kind != HzKind::Equipment {
                continue;
            }
            let Some(&gi) = seed_group_of_equip.get(&nb) else {
                continue;
            };
            let d = (nd.center.0 - graph.nodes[nb].center.0)
                .hypot(nd.center.1 - graph.nodes[nb].center.1);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, gi));
            }
        }
        if let Some((_, gi)) = best {
            nozzle_owner_group.insert(ni, gi);
        }
    }

    let mut out_nodes: Vec<PidHazopNode> = Vec::with_capacity(seed_vec.len());
    let mut all_linked_pipes: BTreeSet<String> = BTreeSet::new();
    let mut all_linked_safeguards: BTreeSet<String> = BTreeSet::new();

    for (gi, eq_indices) in seed_vec.iter().enumerate() {
        let lead_idx = eq_indices[0];
        let lead = &graph.nodes[lead_idx];
        let equip_tag = lead
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("UNTAGGED-{:X}", lead.primary_handle.value()));

        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        for &ei in eq_indices {
            if visited.insert(ei) {
                queue.push_back(ei);
            }
        }

        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
        let mut span_dwgs: BTreeSet<String> = BTreeSet::new();
        span_dwgs.insert(lead.drawing_name.clone());

        let mut pipe_segs = 0usize;
        let mut pipelines: BTreeSet<String> = BTreeSet::new();
        let mut valves: BTreeSet<String> = BTreeSet::new();
        let mut safeguards: BTreeSet<String> = BTreeSet::new();
        let mut loop_nums: BTreeSet<String> = BTreeSet::new();
        let mut measured_vars: BTreeSet<String> = BTreeSet::new();

        while let Some(cur) = queue.pop_front() {
            let cnd = &graph.nodes[cur];
            span_dwgs.insert(cnd.drawing_name.clone());
            push_unique_handles(
                handles_by_tab.entry(cnd.tab_index).or_default(),
                &cnd.member_handles,
            );

            match cnd.kind {
                HzKind::PipeSegment => {
                    pipe_segs += 1;
                    if let Some(ref lbl) = cnd.tag_or_label {
                        pipelines.insert(lbl.clone());
                    }
                }
                HzKind::Valve => {
                    if let Some(ref vt) = cnd.tag_or_label {
                        valves.insert(vt.clone());
                        if let Some(parsed) = parse_isa_instrument_tag(vt, &cnd.sppid_class) {
                            safeguards.insert(vt.clone());
                            loop_nums.insert(parsed.loop_number.to_ascii_uppercase());
                            measured_vars.insert(parsed.measured_var);
                        }
                    }
                }
                HzKind::ReliefDevice => {
                    let rtag = cnd
                        .tag_or_label
                        .clone()
                        .unwrap_or_else(|| format!("RELIEF@{:X}", cnd.primary_handle.value()));
                    safeguards.insert(rtag);
                    measured_vars.insert("P".to_string());
                }
                HzKind::Instrument => {
                    if let Some(ref it) = cnd.tag_or_label {
                        safeguards.insert(it.clone());
                        if let Some(parsed) = parse_isa_instrument_tag(it, &cnd.sppid_class) {
                            loop_nums.insert(parsed.loop_number.to_ascii_uppercase());
                            measured_vars.insert(parsed.measured_var);
                        }
                    }
                }
                _ => {}
            }

            // Stop expansion if `cur` is a peer equipment or a nozzle belonging to a peer equipment
            if cnd.kind == HzKind::Equipment && seed_group_of_equip.get(&cur) != Some(&gi) {
                continue;
            }
            if cnd.kind == HzKind::Nozzle
                && nozzle_owner_group
                    .get(&cur)
                    .is_some_and(|&owner| owner != gi)
            {
                continue;
            }

            for &nxt in &graph.adj[cur] {
                if graph.nodes[nxt].kind == HzKind::Equipment
                    && seed_group_of_equip.get(&nxt) != Some(&gi)
                {
                    // Stop at peer equipment boundary without absorbing peer equipment or its instruments
                    continue;
                }
                if graph.nodes[nxt].kind == HzKind::Nozzle
                    && nozzle_owner_group
                        .get(&nxt)
                        .is_some_and(|&owner| owner != gi)
                {
                    continue;
                }
                if visited.insert(nxt) {
                    queue.push_back(nxt);
                }
            }
        }

        // Cross-reference ISA-5.1 instrument loops so remote DCS controllers/alarms/actuators join the node
        for lnum in &loop_nums {
            if let Some(&li) = loops_by_num.get(lnum) {
                let lp = &loop_rep.loops[li];
                measured_vars.insert(lp.measured_variable.clone());
                for mem in &lp.members {
                    safeguards.insert(mem.item_tag.clone());
                    span_dwgs.insert(mem.drawing_name.clone());
                    if !mem.member_handles.is_empty() {
                        push_unique_handles(
                            handles_by_tab.entry(mem.tab_index).or_default(),
                            &mem.member_handles,
                        );
                    }
                }
            }
        }

        for p in &pipelines {
            all_linked_pipes.insert(p.clone());
        }
        for s in &safeguards {
            all_linked_safeguards.insert(s.clone());
        }

        let status = if safeguards.is_empty() {
            HazopSafeguardStatus::Unprotected
        } else {
            HazopSafeguardStatus::Protected
        };
        let diagnostic_note = if status == HazopSafeguardStatus::Protected {
            format!(
                "已关联 {} 条工艺管线、{} 台阀门与 {} 个检测/控制/泄放保护元件。",
                pipelines.len(),
                valves.len(),
                safeguards.len()
            )
        } else {
            format!(
                "盲区预警：设备 {} 连接 {} 段工艺管线但未检出关联的检测/报警/泄放保护仪表。",
                equip_tag, pipe_segs
            )
        };

        let mut span_vec: Vec<String> = span_dwgs.into_iter().collect();
        if let Some(pos) = span_vec
            .iter()
            .position(|d| d == &lead.drawing_name)
            .filter(|&p| p != 0)
        {
            let primary = span_vec.remove(pos);
            span_vec.insert(0, primary);
        }

        let node_id = format!("HAZOP-{:02}", out_nodes.len() + 1);
        out_nodes.push(PidHazopNode {
            node_id,
            tab_index: lead.tab_index,
            primary_drawing: lead.drawing_name.clone(),
            span_drawings: span_vec,
            equipment_tag: equip_tag,
            equipment_class: lead.sppid_class.clone(),
            primary_handle_hex: format!("{:X}", lead.primary_handle.value()),
            x: lead.center.0,
            y: lead.center.1,
            status,
            pipe_segments_count: pipe_segs,
            connected_pipelines: pipelines.into_iter().collect(),
            valves: valves.into_iter().collect(),
            safeguard_instruments: safeguards.into_iter().collect(),
            safeguard_loops: loop_nums.into_iter().collect(),
            applicable_guidewords: derive_guidewords(&lead.sppid_class, &measured_vars),
            diagnostic_note,
            primary_handle: lead.primary_handle,
            handles_by_tab,
        });
    }

    let multi_sheet_nodes = out_nodes
        .iter()
        .filter(|n| n.span_drawings.len() > 1)
        .count();
    let protected_nodes = out_nodes
        .iter()
        .filter(|n| n.status == HazopSafeguardStatus::Protected)
        .count();
    let unprotected_nodes = out_nodes
        .iter()
        .filter(|n| n.status == HazopSafeguardStatus::Unprotected)
        .count();

    PidHazopReport {
        summary: PidHazopSummary {
            sheets_scanned: sheets.len(),
            total_nodes: out_nodes.len(),
            multi_sheet_nodes,
            protected_nodes,
            unprotected_nodes,
            total_linked_pipelines: all_linked_pipes.len(),
            total_linked_safeguards: all_linked_safeguards.len(),
        },
        nodes: out_nodes,
    }
}

/// Match whether HAZOP node `nd` corresponds to user `query` (by `node_id` or `equipment_tag`).
pub fn hazop_node_matches_query(nd: &PidHazopNode, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if nd.node_id.eq_ignore_ascii_case(q) {
        return true;
    }
    if q.parse::<usize>()
        .is_ok_and(|num| nd.node_id == format!("HAZOP-{num:02}"))
    {
        return true;
    }
    match_query_rank(&nd.equipment_tag, q).is_some()
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and extract a unified HAZOP report (`#160`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_hazop_from_path(input_path: &Path) -> Result<PidHazopReport, String> {
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
    Ok(analyze_pid_hazop_nodes(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidHazopReport` as delimited HAZOP Node & Safeguard Worksheet text with UTF-8 BOM.
pub fn format_pid_hazop_delimited(report: &PidHazopReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = HAZOP_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for nd in &report.nodes {
        let cells = [
            nd.node_id.as_str(),
            nd.equipment_tag.as_str(),
            nd.equipment_class.as_str(),
            nd.primary_drawing.as_str(),
            &nd.span_drawings.join(";"),
            nd.status.as_str(),
            &nd.connected_pipelines.join(";"),
            &nd.valves.join(";"),
            &nd.safeguard_instruments.join(";"),
            &nd.safeguard_loops.join(";"),
            &nd.applicable_guidewords.join(";"),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidHazopReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#160`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_hazop_file(report: &PidHazopReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize hazop JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_hazop_delimited(report, '\t'),
        "csv" => format_pid_hazop_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported HAZOP report extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

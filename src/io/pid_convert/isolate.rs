//! Multi-sheet P&ID equipment energy-isolation boundary (LOTO shutoff valves & unvalved
//! hazard) solver and LOTO checklist exporter (`PIDISOLATE`, `L3-3` / `L3-4` / `L2-2` / `L6-2` / `#155`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::external_props::normalize_lookup_key;
use super::find::match_query_rank;
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Canonical 9-column header order shared by CSV and TSV LOTO isolation checklists.
pub const ISOLATION_CSV_HEADERS: [&str; 9] = [
    "RecordType",
    "TagOrLabel",
    "CategoryOrHazard",
    "SPPIDClass",
    "DrawingCode",
    "LineNumber",
    "ActionOrMessage",
    "X",
    "Y",
];

/// Classification of an un-isolated branch hazard discovered during `PIDISOLATE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationHazardKind {
    UnvalvedPeerEquipment,
    UnvalvedExternalOpc,
    UnvalvedOpenDeadEnd,
}

impl IsolationHazardKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnvalvedPeerEquipment => "unvalved_peer_equipment",
            Self::UnvalvedExternalOpc => "unvalved_external_opc",
            Self::UnvalvedOpenDeadEnd => "unvalved_open_dead_end",
        }
    }
}

/// One first-line shutoff valve on the energy-isolation boundary (`LOTO`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidIsolationValve {
    pub tab_index: usize,
    pub drawing_name: String,
    pub valve_tag: String,
    pub sppid_class: String,
    pub line_label: Option<String>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// One unvalved direct connection or open dead-end hazard breaching isolation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidIsolationHazard {
    pub kind: IsolationHazardKind,
    pub tab_index: usize,
    pub drawing_name: String,
    pub target_tag_or_label: Option<String>,
    pub message: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Summary statistics for an equipment isolation analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidIsolationSummary {
    pub target_tag: String,
    pub target_drawing: String,
    pub fully_isolatable: bool,
    pub crossed_tabs: usize,
    pub boundary_valves_count: usize,
    pub envelope_pipe_segments: usize,
    pub hazard_count: usize,
    pub unvalved_peer_equipment: usize,
    pub unvalved_external_opcs: usize,
    pub unvalved_open_dead_ends: usize,
}

/// Complete LOTO isolation boundary report produced by `PIDISOLATE`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidIsolationReport {
    pub summary: PidIsolationSummary,
    pub boundary_valves: Vec<PidIsolationValve>,
    pub hazards: Vec<PidIsolationHazard>,
    #[serde(skip)]
    pub envelope_handles_by_tab: BTreeMap<usize, Vec<Handle>>,
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
enum IsoKind {
    Equipment,
    Valve,
    PassiveFittingOrInst,
    Nozzle,
    Opc,
    PipeSegment,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Local,
    PairedOpc,
}

#[derive(Clone, Copy)]
struct GraphEdge {
    to: usize,
    kind: EdgeKind,
}

struct IsoNode {
    tab_index: usize,
    drawing_name: String,
    kind: IsoKind,
    sppid_class: String,
    tag_or_label: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
    upm: f64,
}

struct IsoGraph {
    nodes: Vec<IsoNode>,
    adj: Vec<Vec<GraphEdge>>,
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

fn is_inline_control_valve(blk_upper: &str, tag_opt: Option<&str>) -> bool {
    if blk_upper.contains("VALVE")
        || blk_upper.contains("BUTTERFLY")
        || blk_upper.contains("MOV")
        || blk_upper.contains("DAMPER")
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
    matches!(
        letters.as_str(),
        "XV" | "FV" | "PV" | "LV" | "TV" | "HV" | "BDV" | "SDV" | "MOV" | "GOV" | "BUV"
    )
}

fn classify_iso_kind(
    cls: &str,
    type_code: &str,
    blk_upper: &str,
    tag_opt: Option<&str>,
) -> IsoKind {
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        IsoKind::Opc
    } else if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        IsoKind::Nozzle
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        IsoKind::Equipment
    } else if cls == "PIDPipingComponent" || type_code.eq_ignore_ascii_case("valve") {
        if is_passive_fitting(blk_upper) {
            IsoKind::PassiveFittingOrInst
        } else {
            IsoKind::Valve
        }
    } else if cls == "PIDInlineInstrument" && is_inline_control_valve(blk_upper, tag_opt) {
        IsoKind::Valve
    } else {
        IsoKind::PassiveFittingOrInst
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

fn extract_authored_iso_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<IsoNode>,
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
                nodes.push(IsoNode {
                    tab_index,
                    drawing_name: drawing_name.to_string(),
                    kind: IsoKind::PipeSegment,
                    sppid_class: "PIDPipingNetworkSegment".to_string(),
                    tag_or_label: lbl,
                    primary_handle: h,
                    member_handles: vec![h],
                    center,
                    bbox,
                    ports: Vec::new(),
                    spans,
                    upm,
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
        let kind = classify_iso_kind(&cls, &auth.type_code, &blk_upper, tag_opt.as_deref());
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(IsoNode {
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
            upm,
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
            nodes.push(IsoNode {
                tab_index,
                drawing_name: drawing_name.to_string(),
                kind: IsoKind::PipeSegment,
                sppid_class: "PIDPipingNetworkSegment".to_string(),
                tag_or_label: lt,
                primary_handle: sh,
                member_handles: vec![sh],
                center,
                bbox,
                ports: Vec::new(),
                spans: vec![sp],
                upm,
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
                    nodes.push(IsoNode {
                        tab_index,
                        drawing_name: drawing_name.to_string(),
                        kind: IsoKind::PipeSegment,
                        sppid_class: "PIDPipingNetworkSegment".to_string(),
                        tag_or_label: None,
                        primary_handle: eh,
                        member_handles: vec![eh],
                        center,
                        bbox,
                        ports: Vec::new(),
                        spans: vec![sp],
                        upm,
                    });
                }
            }
        }
    }
}

fn extract_unpromoted_iso_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<IsoNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_iso_nodes(tab_index, drawing_name, &scratch, nodes);
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
        let kind = classify_iso_kind(
            &sym.sppid_class,
            "",
            &sym.source_block.to_ascii_uppercase(),
            tag_opt.as_deref(),
        );

        nodes.push(IsoNode {
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
            upm,
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
        nodes.push(IsoNode {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: IsoKind::PipeSegment,
            sppid_class: "PIDPipingNetworkSegment".to_string(),
            tag_or_label: lbl_opt,
            primary_handle: h,
            member_handles: vec![h],
            center,
            bbox,
            ports: Vec::new(),
            spans: vec![sp],
            upm,
        });
    }
}

fn add_undirected_edge(adj: &mut [Vec<GraphEdge>], a: usize, b: usize, kind: EdgeKind) {
    if a == b {
        return;
    }
    if !adj[a].iter().any(|e| e.to == b) {
        adj[a].push(GraphEdge { to: b, kind });
    }
    if !adj[b].iter().any(|e| e.to == a) {
        adj[b].push(GraphEdge { to: a, kind });
    }
}

fn merge_opc_into_iso_sheet(
    c: &OpcConnectorItem,
    drawing_name: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<IsoNode>,
) -> usize {
    for idx in sheet_range {
        if nodes[idx].kind == IsoKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = nodes[idx].kind == IsoKind::Opc
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = IsoKind::Opc;
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
    nodes.push(IsoNode {
        tab_index: c.tab_index,
        drawing_name: drawing_name.to_string(),
        kind: IsoKind::Opc,
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
        upm,
    });
    idx
}

fn wire_iso_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[IsoNode],
    adj: &mut [Vec<GraphEdge>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;
    let opc_tol = 14.0 * upm;
    let equip_tol = 16.0 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == IsoKind::PipeSegment {
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
                                    nodes[sn].kind != IsoKind::Opc
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
                add_undirected_edge(adj, na, nb, EdgeKind::Local);
            }
        }
    }

    for &pn in &pipe_nodes {
        for &sn in &sym_nodes {
            let margin = match nodes[sn].kind {
                IsoKind::Equipment => 12.0 * upm,
                IsoKind::Opc => opc_tol,
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
                add_undirected_edge(adj, pn, sn, EdgeKind::Local);
            }
        }
    }

    for &sn in &sym_nodes {
        if nodes[sn].kind == IsoKind::Opc && adj[sn].is_empty() {
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
                add_undirected_edge(adj, sn, pn, EdgeKind::Local);
            }
        }
    }

    for (si, &na) in sym_nodes.iter().enumerate() {
        for &nb in &sym_nodes[(si + 1)..] {
            let ka = nodes[na].kind;
            let kb = nodes[nb].kind;
            let connected = if (ka == IsoKind::Nozzle && kb == IsoKind::Equipment)
                || (kb == IsoKind::Nozzle && ka == IsoKind::Equipment)
            {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, equip_tol)
            } else if ka != IsoKind::Equipment && kb != IsoKind::Equipment {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, 4.0 * upm)
            } else {
                false
            };
            if connected {
                add_undirected_edge(adj, na, nb, EdgeKind::Local);
            }
        }
    }
}

fn build_iso_graph(sheets: &[OpcSheetInput<'_>], allow_cross_tab: bool) -> IsoGraph {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut nodes: Vec<IsoNode> = Vec::new();
    let mut sheet_ranges: Vec<(usize, f64, std::ops::Range<usize>)> = Vec::new();
    let mut opc_lookup: BTreeMap<(usize, Handle), usize> = BTreeMap::new();

    for sh in sheets {
        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_iso_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        } else {
            extract_unpromoted_iso_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        }
        let base_end = nodes.len();
        for c in opc_rep
            .connectors
            .iter()
            .filter(|c| c.tab_index == sh.tab_index)
        {
            let opc_idx =
                merge_opc_into_iso_sheet(c, sh.fallback_name, upm, start_idx..base_end, &mut nodes);
            opc_lookup.insert((c.tab_index, c.primary_handle), opc_idx);
        }
        let final_end = nodes.len();
        sheet_ranges.push((sh.tab_index, upm, start_idx..final_end));
    }

    let mut adj: Vec<Vec<GraphEdge>> = vec![Vec::new(); nodes.len()];
    for (_, upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_iso_local_edges(&indices, *upm, &nodes, &mut adj);
    }

    if allow_cross_tab {
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
            add_undirected_edge(&mut adj, na, nb, EdgeKind::PairedOpc);
        }
    }

    IsoGraph { nodes, adj }
}

fn nearest_line_label_for_valve(graph: &IsoGraph, v_idx: usize) -> Option<String> {
    for edge in &graph.adj[v_idx] {
        let nd = &graph.nodes[edge.to];
        if nd.kind == IsoKind::PipeSegment && nd.tag_or_label.is_some() {
            return nd.tag_or_label.clone();
        }
    }
    None
}

fn pass_through_valves_on_pipe(graph: &IsoGraph, pipe_idx: usize) -> Vec<usize> {
    let pnode = &graph.nodes[pipe_idx];
    let tol = 4.0 * pnode.upm;
    let mut out = Vec::new();
    for edge in &graph.adj[pipe_idx] {
        let nb = edge.to;
        if graph.nodes[nb].kind != IsoKind::Valve {
            continue;
        }
        let c = graph.nodes[nb].center;
        let interior = pnode.spans.iter().any(|&sp| {
            let vx = sp.b.0 - sp.a.0;
            let vy = sp.b.1 - sp.a.1;
            let len2 = vx * vx + vy * vy;
            if len2 <= 1e-9 {
                false
            } else {
                let t = ((c.0 - sp.a.0) * vx + (c.1 - sp.a.1) * vy) / len2;
                t > 0.04 && t < 0.96 && point_to_span_dist(c, sp) <= tol
            }
        });
        if interior {
            out.push(nb);
        }
    }
    out
}

fn execute_isolation_bfs(graph: &IsoGraph, origin_idx: usize) -> PidIsolationReport {
    let mut origin_set = BTreeSet::new();
    origin_set.insert(origin_idx);
    let origin_tab = graph.nodes[origin_idx].tab_index;
    let origin_upm = graph.nodes[origin_idx].upm;

    // Include directly attached nozzles of target equipment in origin_set
    if graph.nodes[origin_idx].kind == IsoKind::Equipment {
        for (idx, nd) in graph.nodes.iter().enumerate() {
            if nd.tab_index == origin_tab
                && nd.kind == IsoKind::Nozzle
                && bboxes_overlap(nd.bbox, graph.nodes[origin_idx].bbox, 16.0 * origin_upm)
            {
                origin_set.insert(idx);
            }
        }
    }

    let mut visited = vec![false; graph.nodes.len()];
    let mut entry_from: Vec<Option<Pt2>> = vec![None; graph.nodes.len()];
    let mut queue = VecDeque::new();
    let mut envelope_indices = Vec::new();
    for &oi in &origin_set {
        visited[oi] = true;
        entry_from[oi] = Some(graph.nodes[oi].center);
        queue.push_back(oi);
        envelope_indices.push(oi);
    }

    let mut valve_set = BTreeSet::new();
    let mut peer_equip_set = BTreeSet::new();
    let mut ext_opc_set = BTreeSet::new();

    while let Some(cur) = queue.pop_front() {
        let cur_entry = entry_from[cur].unwrap_or(graph.nodes[cur].center);
        let inline_vs = if graph.nodes[cur].kind == IsoKind::PipeSegment {
            pass_through_valves_on_pipe(graph, cur)
        } else {
            Vec::new()
        };
        for &iv in &inline_vs {
            valve_set.insert(iv);
        }

        for edge in &graph.adj[cur] {
            let nb = edge.to;
            if origin_set.contains(&nb) {
                continue;
            }
            // If cur had an inline pass-through valve between cur_entry and nb, block beyond it
            if !inline_vs.is_empty() && nb != cur {
                let nb_c = graph.nodes[nb].center;
                let blocked_beyond = inline_vs.iter().any(|&iv| {
                    let vc = graph.nodes[iv].center;
                    let v1 = (vc.0 - cur_entry.0, vc.1 - cur_entry.1);
                    let v2 = (nb_c.0 - vc.0, nb_c.1 - vc.1);
                    (v1.0 * v2.0 + v1.1 * v2.1) > 0.5 * graph.nodes[cur].upm
                });
                if blocked_beyond {
                    continue;
                }
            }

            match graph.nodes[nb].kind {
                IsoKind::Valve => {
                    valve_set.insert(nb);
                }
                IsoKind::Equipment => {
                    peer_equip_set.insert(nb);
                }
                IsoKind::Opc => {
                    let has_paired_bridge =
                        graph.adj[nb].iter().any(|e| e.kind == EdgeKind::PairedOpc);
                    if has_paired_bridge {
                        if !visited[nb] {
                            visited[nb] = true;
                            entry_from[nb] = Some(graph.nodes[nb].center);
                            envelope_indices.push(nb);
                            queue.push_back(nb);
                        }
                    } else {
                        ext_opc_set.insert(nb);
                    }
                }
                IsoKind::PipeSegment | IsoKind::Nozzle | IsoKind::PassiveFittingOrInst => {
                    if !visited[nb] {
                        visited[nb] = true;
                        entry_from[nb] = Some(graph.nodes[cur].center);
                        envelope_indices.push(nb);
                        queue.push_back(nb);
                    }
                }
            }
        }
    }

    // Check open pipe dead-ends inside the envelope (excluding pipes already truncated by pass-through valve)
    let mut open_dead_ends: Vec<(usize, Pt2)> = Vec::new();
    for &ei in &envelope_indices {
        if graph.nodes[ei].kind != IsoKind::PipeSegment {
            continue;
        }
        if !pass_through_valves_on_pipe(graph, ei).is_empty() {
            continue;
        }
        let upm = graph.nodes[ei].upm;
        let mut reported = false;
        for &sp in &graph.nodes[ei].spans {
            for pt in [sp.a, sp.b] {
                let closed_by_neighbor = graph.adj[ei].iter().any(|edge| {
                    let ond = &graph.nodes[edge.to];
                    if ond.kind == IsoKind::PipeSegment {
                        ond.spans
                            .iter()
                            .any(|&osp| point_to_span_dist(pt, osp) <= 6.5 * upm)
                    } else {
                        point_in_bbox(pt, ond.bbox, 14.0 * upm)
                            || (pt.0 - ond.center.0).hypot(pt.1 - ond.center.1) <= 14.0 * upm
                    }
                });
                if !closed_by_neighbor && !reported {
                    reported = true;
                    open_dead_ends.push((ei, pt));
                }
            }
        }
    }

    let mut visited_tabs = BTreeSet::new();
    let mut envelope_handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
    let mut envelope_pipe_segments = 0usize;
    for &ei in &envelope_indices {
        let nd = &graph.nodes[ei];
        visited_tabs.insert(nd.tab_index);
        if nd.kind == IsoKind::PipeSegment {
            envelope_pipe_segments += 1;
        }
        let h_vec = envelope_handles_by_tab.entry(nd.tab_index).or_default();
        for &mh in &nd.member_handles {
            if !h_vec.contains(&mh) {
                h_vec.push(mh);
            }
        }
    }

    let mut boundary_valves = Vec::new();
    for &vi in &valve_set {
        let nd = &graph.nodes[vi];
        visited_tabs.insert(nd.tab_index);
        let vtag = nd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("UNTAGGED@{:.0},{:.0}", nd.center.0, nd.center.1));
        boundary_valves.push(PidIsolationValve {
            tab_index: nd.tab_index,
            drawing_name: nd.drawing_name.clone(),
            valve_tag: vtag,
            sppid_class: nd.sppid_class.clone(),
            line_label: nearest_line_label_for_valve(graph, vi),
            primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
            x: nd.center.0,
            y: nd.center.1,
            primary_handle: nd.primary_handle,
            member_handles: nd.member_handles.clone(),
        });
    }
    boundary_valves.sort_by(|a, b| {
        (a.tab_index, &a.valve_tag, &a.primary_handle_hex).cmp(&(
            b.tab_index,
            &b.valve_tag,
            &b.primary_handle_hex,
        ))
    });

    let mut hazards = Vec::new();
    for &pi in &peer_equip_set {
        let nd = &graph.nodes[pi];
        visited_tabs.insert(nd.tab_index);
        let eq_lbl = nd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| "UNTAGGED_EQUIP".to_string());
        hazards.push(PidIsolationHazard {
            kind: IsolationHazardKind::UnvalvedPeerEquipment,
            tab_index: nd.tab_index,
            drawing_name: nd.drawing_name.clone(),
            target_tag_or_label: nd.tag_or_label.clone(),
            message: format!("支路未经任何切断阀直连相邻设备 {eq_lbl}"),
            primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
            x: nd.center.0,
            y: nd.center.1,
            primary_handle: nd.primary_handle,
            member_handles: nd.member_handles.clone(),
        });
    }
    for &oi in &ext_opc_set {
        let nd = &graph.nodes[oi];
        visited_tabs.insert(nd.tab_index);
        let tgt = nd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| "EXTERNAL".to_string());
        hazards.push(PidIsolationHazard {
            kind: IsolationHazardKind::UnvalvedExternalOpc,
            tab_index: nd.tab_index,
            drawing_name: nd.drawing_name.clone(),
            target_tag_or_label: nd.tag_or_label.clone(),
            message: format!("支路未经切断阀直通外部未配对页间连接符 {tgt}"),
            primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
            x: nd.center.0,
            y: nd.center.1,
            primary_handle: nd.primary_handle,
            member_handles: nd.member_handles.clone(),
        });
    }
    for &(pi, pt) in &open_dead_ends {
        let nd = &graph.nodes[pi];
        hazards.push(PidIsolationHazard {
            kind: IsolationHazardKind::UnvalvedOpenDeadEnd,
            tab_index: nd.tab_index,
            drawing_name: nd.drawing_name.clone(),
            target_tag_or_label: nd.tag_or_label.clone(),
            message: format!(
                "隔离包络内管段 {:X} 在 ({:.1}, {:.1}) 处存在无阀敞口端",
                nd.primary_handle.value(),
                pt.0,
                pt.1
            ),
            primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
            x: pt.0,
            y: pt.1,
            primary_handle: nd.primary_handle,
            member_handles: nd.member_handles.clone(),
        });
    }

    let origin_node = &graph.nodes[origin_idx];
    let target_tag = origin_node
        .tag_or_label
        .clone()
        .unwrap_or_else(|| format!("HANDLE:{:X}", origin_node.primary_handle.value()));
    let unvalved_peer_equipment = peer_equip_set.len();
    let unvalved_external_opcs = ext_opc_set.len();
    let unvalved_open_dead_ends = open_dead_ends.len();
    let hazard_count = hazards.len();

    PidIsolationReport {
        summary: PidIsolationSummary {
            target_tag,
            target_drawing: origin_node.drawing_name.clone(),
            fully_isolatable: hazard_count == 0 && !boundary_valves.is_empty(),
            crossed_tabs: visited_tabs.len(),
            boundary_valves_count: boundary_valves.len(),
            envelope_pipe_segments,
            hazard_count,
            unvalved_peer_equipment,
            unvalved_external_opcs,
            unvalved_open_dead_ends,
        },
        boundary_valves,
        hazards,
        envelope_handles_by_tab,
    }
}

/// Solve the LOTO isolation boundary for the currently `selected` entity on `active_tab` (`#155`).
pub fn solve_pid_isolation_from_handles(
    active_tab: usize,
    selected: &[Handle],
    allow_cross_tab: bool,
    sheets: &[OpcSheetInput<'_>],
) -> Result<PidIsolationReport, String> {
    if selected.is_empty() {
        return Err("请先选中目标设备，或指定设备位号（用法：PIDISOLATE [<设备位号>] [ALLTABS] [VALVES | ENVELOPE | HAZARDS | NEXT] [EXPORT \"<.json|.csv>\"]）。".to_string());
    }
    let sel_set: BTreeSet<Handle> = selected.iter().copied().collect();
    let graph = build_iso_graph(sheets, allow_cross_tab);
    let mut cands: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(idx, nd)| {
            (nd.tab_index == active_tab
                && (sel_set.contains(&nd.primary_handle)
                    || nd.member_handles.iter().any(|mh| sel_set.contains(mh))))
            .then_some(idx)
        })
        .collect();
    cands.sort_by_key(|&idx| u8::from(graph.nodes[idx].kind != IsoKind::Equipment));
    let Some(&origin_idx) = cands.first() else {
        return Err("当前选中的图元不是有效的 P&ID 设备或工艺节点。".to_string());
    };
    Ok(execute_isolation_bfs(&graph, origin_idx))
}

/// Solve the LOTO isolation boundary for a target tag `query` (`#155`).
pub fn solve_pid_isolation_by_query(
    query: &str,
    active_tab: usize,
    allow_cross_tab: bool,
    sheets: &[OpcSheetInput<'_>],
) -> Result<PidIsolationReport, String> {
    let graph = build_iso_graph(sheets, allow_cross_tab);
    let mut matches: Vec<(u8, u8, u8, usize)> = Vec::new();
    for (idx, nd) in graph.nodes.iter().enumerate() {
        let Some(ref cand) = nd.tag_or_label else {
            continue;
        };
        if normalize_lookup_key(cand).is_empty() {
            continue;
        }
        if let Some(rank) = match_query_rank(cand, query) {
            let tab_pen = u8::from(nd.tab_index != active_tab);
            let kind_pen = match nd.kind {
                IsoKind::Equipment => 0,
                IsoKind::PipeSegment => 2,
                _ => 1,
            };
            matches.push((rank, kind_pen, tab_pen, idx));
        }
    }
    matches.sort();
    let Some(&(_, _, _, origin_idx)) = matches.first() else {
        return Err(format!("未找到匹配 \"{query}\" 的目标设备或节点。"));
    };
    Ok(execute_isolation_bfs(&graph, origin_idx))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidIsolationReport` as delimited LOTO checklist text (CSV or TSV) with UTF-8 BOM.
pub fn format_pid_isolation_delimited(report: &PidIsolationReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = ISOLATION_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for v in &report.boundary_valves {
        let ln = v.line_label.as_deref().unwrap_or("");
        let xs = format!("{:.1}", v.x);
        let ys = format!("{:.1}", v.y);
        let cells = [
            "boundary_valve",
            v.valve_tag.as_str(),
            "LOTO_Shutoff",
            v.sppid_class.as_str(),
            v.drawing_name.as_str(),
            ln,
            "CLOSE & LOCK (LOTO)",
            xs.as_str(),
            ys.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }

    for hz in &report.hazards {
        let tgt = hz.target_tag_or_label.as_deref().unwrap_or("");
        let xs = format!("{:.1}", hz.x);
        let ys = format!("{:.1}", hz.y);
        let cells = [
            "hazard",
            tgt,
            hz.kind.as_str(),
            "",
            hz.drawing_name.as_str(),
            "",
            hz.message.as_str(),
            xs.as_str(),
            ys.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidIsolationReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#155`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_isolation_file(
    report: &PidIsolationReport,
    out_path: &Path,
) -> Result<(), String> {
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
                .map_err(|e| format!("serialize isolation JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_isolation_delimited(report, '\t'),
        "csv" => format_pid_isolation_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported isolation output extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

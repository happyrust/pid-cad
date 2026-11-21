//! Multi-sheet P&ID topological connectivity subnet & shortest process path tracer
//! (`PIDTRACE`, `L3-3` / `L3-4` / `L2-2` / `L6-2` / `#153`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

/// Operational mode of `PIDTRACE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceMode {
    Subnet,
    ShortestPath,
}

/// Functional engineering classification of a graph node in `PIDTRACE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceNodeKind {
    Equipment,
    Valve,
    Instrument,
    Nozzle,
    Opc,
    PipeSegment,
}

/// Ordered topological step along a traced subnet or shortest path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    pub step_index: usize,
    pub tab_index: usize,
    pub drawing_name: String,
    pub node_kind: TraceNodeKind,
    pub sppid_class: String,
    pub tag_or_label: Option<String>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
}

/// Structured connectivity & shortest-path report returned by `PIDTRACE`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceReport {
    pub mode: TraceMode,
    pub reachable: bool,
    pub crossed_tabs: usize,
    pub opc_bridges_traversed: usize,
    pub total_nodes_visited: usize,
    pub pipe_segments_count: usize,
    pub equipment_tags: Vec<String>,
    pub valve_tags: Vec<String>,
    pub instrument_tags: Vec<String>,
    pub pipeline_labels: Vec<String>,
    pub opc_targets: Vec<String>,
    pub path_steps: Vec<TraceStep>,
    #[serde(skip)]
    pub tab_handles: BTreeMap<usize, Vec<Handle>>,
    #[serde(skip)]
    pub focus_tab: usize,
    #[serde(skip)]
    pub focus_xy: (f64, f64),
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
enum EdgeKind {
    Local,
    PairedOpc,
}

#[derive(Clone, Copy)]
struct GraphEdge {
    to: usize,
    kind: EdgeKind,
}

struct GraphNode {
    tab_index: usize,
    drawing_name: String,
    kind: TraceNodeKind,
    sppid_class: String,
    tag_or_label: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct WorkspaceGraph {
    nodes: Vec<GraphNode>,
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

fn classify_symbol_kind(cls: &str, type_code: &str) -> TraceNodeKind {
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        TraceNodeKind::Opc
    } else if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        TraceNodeKind::Nozzle
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        TraceNodeKind::Equipment
    } else if matches!(
        cls,
        "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
    ) || type_code.eq_ignore_ascii_case("instrument")
    {
        TraceNodeKind::Instrument
    } else {
        TraceNodeKind::Valve
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

fn extract_authored_sheet_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<GraphNode>,
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
                nodes.push(GraphNode {
                    tab_index,
                    drawing_name: drawing_name.to_string(),
                    kind: TraceNodeKind::PipeSegment,
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
            .unwrap_or(&auth.type_code)
            .to_string();
        let kind = classify_symbol_kind(&cls, &auth.type_code);
        let tag_opt = if !auth.tag.trim().is_empty() {
            Some(auth.tag.trim().to_string())
        } else {
            pmap.get("_to_from_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(GraphNode {
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
            nodes.push(GraphNode {
                tab_index,
                drawing_name: drawing_name.to_string(),
                kind: TraceNodeKind::PipeSegment,
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
                    nodes.push(GraphNode {
                        tab_index,
                        drawing_name: drawing_name.to_string(),
                        kind: TraceNodeKind::PipeSegment,
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

fn extract_unpromoted_sheet_nodes(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    nodes: &mut Vec<GraphNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_sheet_nodes(tab_index, drawing_name, &scratch, nodes);
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
        let kind = classify_symbol_kind(&sym.sppid_class, "");
        let tag_opt = sym
            .tag
            .as_deref()
            .or(sym.to_from_text.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        nodes.push(GraphNode {
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
        nodes.push(GraphNode {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: TraceNodeKind::PipeSegment,
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

fn merge_opc_connector_into_sheet(
    c: &OpcConnectorItem,
    drawing_name: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<GraphNode>,
) -> usize {
    for idx in sheet_range {
        if nodes[idx].kind == TraceNodeKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = nodes[idx].kind == TraceNodeKind::Opc
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = TraceNodeKind::Opc;
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
    nodes.push(GraphNode {
        tab_index: c.tab_index,
        drawing_name: drawing_name.to_string(),
        kind: TraceNodeKind::Opc,
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

fn wire_sheet_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[GraphNode],
    adj: &mut [Vec<GraphEdge>],
) {
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;
    let opc_tol = 14.0 * upm;
    let equip_tol = 16.0 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == TraceNodeKind::PipeSegment {
            pipe_nodes.push(idx);
        } else {
            sym_nodes.push(idx);
        }
    }

    // 1. Pipe <-> Pipe (tight <=2.0 mm snap always connects; 2.0..=6.5 mm micro-gap bridge
    //    only connects when no inline symbol occupies the gap between the segments)
    let tight_tol = 2.0 * upm;
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
                                    nodes[sn].kind != TraceNodeKind::Opc
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

    // 2. Pipe <-> Symbol
    for &pn in &pipe_nodes {
        for &sn in &sym_nodes {
            let margin = match nodes[sn].kind {
                TraceNodeKind::Equipment => 12.0 * upm,
                TraceNodeKind::Opc => opc_tol,
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

    // 3. Fallback OPC -> nearest pipe segment within 42 mm if OPC had no direct geometric touch
    for &sn in &sym_nodes {
        if nodes[sn].kind == TraceNodeKind::Opc && adj[sn].is_empty() {
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

    // 4. Nozzle <-> Equipment & abutting symbols
    for (si, &na) in sym_nodes.iter().enumerate() {
        for &nb in &sym_nodes[(si + 1)..] {
            let ka = nodes[na].kind;
            let kb = nodes[nb].kind;
            let connected = if (ka == TraceNodeKind::Nozzle && kb == TraceNodeKind::Equipment)
                || (kb == TraceNodeKind::Nozzle && ka == TraceNodeKind::Equipment)
            {
                bboxes_overlap(nodes[na].bbox, nodes[nb].bbox, equip_tol)
            } else if ka != TraceNodeKind::Equipment && kb != TraceNodeKind::Equipment {
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

fn build_workspace_graph(sheets: &[OpcSheetInput<'_>], allow_cross_tab: bool) -> WorkspaceGraph {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut nodes: Vec<GraphNode> = Vec::new();
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
            extract_authored_sheet_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        } else {
            extract_unpromoted_sheet_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        }
        let base_end = nodes.len();
        for c in opc_rep
            .connectors
            .iter()
            .filter(|c| c.tab_index == sh.tab_index)
        {
            let opc_idx = merge_opc_connector_into_sheet(
                c,
                sh.fallback_name,
                upm,
                start_idx..base_end,
                &mut nodes,
            );
            opc_lookup.insert((c.tab_index, c.primary_handle), opc_idx);
        }
        let final_end = nodes.len();
        sheet_ranges.push((sh.tab_index, upm, start_idx..final_end));
    }

    let mut adj: Vec<Vec<GraphEdge>> = vec![Vec::new(); nodes.len()];
    for (_, upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_sheet_local_edges(&indices, *upm, &nodes, &mut adj);
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

    WorkspaceGraph { nodes, adj }
}

fn push_unique_str(vec: &mut Vec<String>, val: &str) {
    let trimmed = val.trim();
    if !trimmed.is_empty() && !vec.iter().any(|x| x == trimmed) {
        vec.push(trimmed.to_string());
    }
}

fn assemble_report(
    graph: &WorkspaceGraph,
    mode: TraceMode,
    reachable: bool,
    ordered_nodes: &[usize],
    opc_bridges_traversed: usize,
    focus_tab: usize,
) -> TraceReport {
    let mut visited_tabs = BTreeSet::new();
    let mut pipe_segments_count = 0usize;
    let mut equipment_tags = Vec::new();
    let mut valve_tags = Vec::new();
    let mut instrument_tags = Vec::new();
    let mut pipeline_labels = Vec::new();
    let mut opc_targets = Vec::new();
    let mut path_steps = Vec::new();
    let mut tab_handles: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();

    let mut focus_sum = (0.0, 0.0);
    let mut focus_cnt = 0usize;

    for &ni in ordered_nodes {
        let nd = &graph.nodes[ni];
        visited_tabs.insert(nd.tab_index);
        if nd.tab_index == focus_tab {
            focus_sum.0 += nd.center.0;
            focus_sum.1 += nd.center.1;
            focus_cnt += 1;
        }
        let h_vec = tab_handles.entry(nd.tab_index).or_default();
        for &mh in &nd.member_handles {
            if !h_vec.contains(&mh) {
                h_vec.push(mh);
            }
        }

        match nd.kind {
            TraceNodeKind::PipeSegment => {
                pipe_segments_count += 1;
                if let Some(ref lbl) = nd.tag_or_label {
                    push_unique_str(&mut pipeline_labels, lbl);
                }
            }
            TraceNodeKind::Equipment => {
                if let Some(ref t) = nd.tag_or_label {
                    push_unique_str(&mut equipment_tags, t);
                }
            }
            TraceNodeKind::Valve => {
                if let Some(ref t) = nd.tag_or_label {
                    push_unique_str(&mut valve_tags, t);
                }
            }
            TraceNodeKind::Instrument => {
                if let Some(ref t) = nd.tag_or_label {
                    push_unique_str(&mut instrument_tags, t);
                }
            }
            TraceNodeKind::Opc => {
                if let Some(ref t) = nd.tag_or_label {
                    push_unique_str(&mut opc_targets, t);
                }
            }
            TraceNodeKind::Nozzle => {}
        }

        if mode == TraceMode::ShortestPath || nd.kind != TraceNodeKind::PipeSegment {
            path_steps.push(TraceStep {
                step_index: path_steps.len(),
                tab_index: nd.tab_index,
                drawing_name: nd.drawing_name.clone(),
                node_kind: nd.kind,
                sppid_class: nd.sppid_class.clone(),
                tag_or_label: nd.tag_or_label.clone(),
                primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
                x: nd.center.0,
                y: nd.center.1,
            });
        }
    }

    let focus_xy = if focus_cnt > 0 {
        (
            focus_sum.0 / (focus_cnt as f64),
            focus_sum.1 / (focus_cnt as f64),
        )
    } else {
        ordered_nodes
            .last()
            .map(|&ni| graph.nodes[ni].center)
            .unwrap_or((0.0, 0.0))
    };

    TraceReport {
        mode,
        reachable,
        crossed_tabs: visited_tabs.len(),
        opc_bridges_traversed,
        total_nodes_visited: ordered_nodes.len(),
        pipe_segments_count,
        equipment_tags,
        valve_tags,
        instrument_tags,
        pipeline_labels,
        opc_targets,
        path_steps,
        tab_handles,
        focus_tab,
        focus_xy,
    }
}

fn run_subnet_bfs(
    graph: &WorkspaceGraph,
    seeds: &[usize],
    default_focus_tab: usize,
) -> TraceReport {
    let mut visited = vec![false; graph.nodes.len()];
    let mut queue = VecDeque::new();
    for &s in seeds {
        if s < graph.nodes.len() && !visited[s] {
            visited[s] = true;
            queue.push_back(s);
        }
    }
    let mut ordered = Vec::new();
    let mut opc_bridges = 0usize;
    let mut seen_opc_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();

    while let Some(cur) = queue.pop_front() {
        ordered.push(cur);
        for edge in &graph.adj[cur] {
            if edge.kind == EdgeKind::PairedOpc {
                let key = (cur.min(edge.to), cur.max(edge.to));
                if seen_opc_pairs.insert(key) {
                    opc_bridges += 1;
                }
            }
            if !visited[edge.to] {
                visited[edge.to] = true;
                queue.push_back(edge.to);
            }
        }
    }

    let focus_tab = seeds
        .first()
        .map(|&s| graph.nodes[s].tab_index)
        .unwrap_or(default_focus_tab);
    assemble_report(
        graph,
        TraceMode::Subnet,
        !ordered.is_empty(),
        &ordered,
        opc_bridges,
        focus_tab,
    )
}

fn rank_matching_nodes(
    graph: &WorkspaceGraph,
    query: &str,
    preferred_tab: usize,
) -> Vec<(u8, usize)> {
    let mut matches = Vec::new();
    for (idx, nd) in graph.nodes.iter().enumerate() {
        let Some(ref cand) = nd.tag_or_label else {
            continue;
        };
        if normalize_lookup_key(cand).is_empty() {
            continue;
        }
        if let Some(rank) = match_query_rank(cand, query) {
            matches.push((rank, idx));
        }
    }
    matches.sort_by(|&(ra, ia), &(rb, ib)| {
        let pa = u8::from(graph.nodes[ia].tab_index != preferred_tab);
        let pb = u8::from(graph.nodes[ib].tab_index != preferred_tab);
        let sym_a = u8::from(graph.nodes[ia].kind == TraceNodeKind::PipeSegment);
        let sym_b = u8::from(graph.nodes[ib].kind == TraceNodeKind::PipeSegment);
        (ra, pa, sym_a, ia).cmp(&(rb, pb, sym_b, ib))
    });
    matches
}

/// Trace the connected P&ID subnet starting from `selected` handles on `active_tab` (`#153`).
pub fn trace_subnet_from_handles(
    active_tab: usize,
    selected: &[Handle],
    allow_cross_tab: bool,
    sheets: &[OpcSheetInput<'_>],
) -> Result<TraceReport, String> {
    if selected.is_empty() {
        return Err("请先在画布选中起点符号/管段，或指定起点位号（用法：PIDTRACE [<起点位号> [<终点位号>]] [ALLTABS] [EXPORT \"<json>\"]）。".to_string());
    }
    let sel_set: BTreeSet<Handle> = selected.iter().copied().collect();
    let graph = build_workspace_graph(sheets, allow_cross_tab);
    let seeds: Vec<usize> = graph
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
    if seeds.is_empty() {
        return Err("当前选中的图元不属于已识别的 P&ID 符号或工艺管段。".to_string());
    }
    Ok(run_subnet_bfs(&graph, &seeds, active_tab))
}

/// Trace the connected P&ID subnet starting from a tag or line-number `query` (`#153`).
pub fn trace_subnet_by_query(
    query: &str,
    active_tab: usize,
    allow_cross_tab: bool,
    sheets: &[OpcSheetInput<'_>],
) -> Result<TraceReport, String> {
    let graph = build_workspace_graph(sheets, allow_cross_tab);
    let matches = rank_matching_nodes(&graph, query, active_tab);
    let Some(&(best_rank, first_idx)) = matches.first() else {
        return Err(format!("未找到匹配起点 \"{query}\" 的 P&ID 符号或管线。"));
    };
    let origin_tab = graph.nodes[first_idx].tab_index;
    let first_kind = graph.nodes[first_idx].kind;
    let seeds: Vec<usize> = if first_kind == TraceNodeKind::PipeSegment {
        matches
            .iter()
            .filter(|&&(r, idx)| {
                r == best_rank
                    && graph.nodes[idx].tab_index == origin_tab
                    && graph.nodes[idx].kind == TraceNodeKind::PipeSegment
            })
            .map(|&(_, idx)| idx)
            .collect()
    } else {
        vec![first_idx]
    };
    Ok(run_subnet_bfs(&graph, &seeds, origin_tab))
}

/// Find the shortest topological process path between `from_query` and `to_query` (`#153`).
pub fn trace_shortest_path_by_query(
    from_query: &str,
    to_query: &str,
    active_tab: usize,
    allow_cross_tab: bool,
    sheets: &[OpcSheetInput<'_>],
) -> Result<TraceReport, String> {
    let graph = build_workspace_graph(sheets, allow_cross_tab);
    let from_matches = rank_matching_nodes(&graph, from_query, active_tab);
    let Some(&(best_from_rank, first_from)) = from_matches.first() else {
        return Err(format!("未找到起点 \"{from_query}\"。"));
    };
    let to_matches = rank_matching_nodes(&graph, to_query, active_tab);
    let Some(&(best_to_rank, _)) = to_matches.first() else {
        return Err(format!("未找到终点 \"{to_query}\"。"));
    };

    let from_tab = graph.nodes[first_from].tab_index;
    let start_sources: Vec<usize> = from_matches
        .iter()
        .filter(|&&(r, idx)| r == best_from_rank && graph.nodes[idx].tab_index == from_tab)
        .map(|&(_, idx)| idx)
        .collect();
    let target_set: BTreeSet<usize> = to_matches
        .iter()
        .filter(|&&(r, _)| r <= best_to_rank.saturating_add(1))
        .map(|&(_, idx)| idx)
        .collect();

    let mut prev: Vec<Option<(usize, EdgeKind)>> = vec![None; graph.nodes.len()];
    let mut visited = vec![false; graph.nodes.len()];
    let mut queue = VecDeque::new();
    for &s in &start_sources {
        visited[s] = true;
        queue.push_back(s);
    }

    let mut reached_target: Option<usize> = None;
    while let Some(cur) = queue.pop_front() {
        if target_set.contains(&cur) && !start_sources.contains(&cur) {
            reached_target = Some(cur);
            break;
        }
        for edge in &graph.adj[cur] {
            if !visited[edge.to] {
                visited[edge.to] = true;
                prev[edge.to] = Some((cur, edge.kind));
                queue.push_back(edge.to);
            }
        }
    }

    if reached_target.is_none() {
        reached_target = start_sources
            .iter()
            .copied()
            .find(|s| target_set.contains(s));
    }

    let Some(end_idx) = reached_target else {
        return Ok(TraceReport {
            mode: TraceMode::ShortestPath,
            reachable: false,
            crossed_tabs: 0,
            opc_bridges_traversed: 0,
            total_nodes_visited: 0,
            pipe_segments_count: 0,
            equipment_tags: Vec::new(),
            valve_tags: Vec::new(),
            instrument_tags: Vec::new(),
            pipeline_labels: Vec::new(),
            opc_targets: Vec::new(),
            path_steps: Vec::new(),
            tab_handles: BTreeMap::new(),
            focus_tab: active_tab,
            focus_xy: (0.0, 0.0),
        });
    };

    let mut rev_path = vec![end_idx];
    let mut opc_bridges = 0usize;
    let mut cur = end_idx;
    while !start_sources.contains(&cur) {
        let Some((p, ek)) = prev[cur] else {
            break;
        };
        if ek == EdgeKind::PairedOpc {
            opc_bridges += 1;
        }
        rev_path.push(p);
        cur = p;
    }
    rev_path.reverse();

    // Include inline valves/instruments mounted directly on untrimmed pass-through pipe segments
    let mut enriched_path = Vec::with_capacity(rev_path.len() + 4);
    let mut in_path: BTreeSet<usize> = rev_path.iter().copied().collect();
    for &ni in &rev_path {
        enriched_path.push(ni);
        if graph.nodes[ni].kind == TraceNodeKind::PipeSegment {
            for edge in &graph.adj[ni] {
                let nb = edge.to;
                if in_path.contains(&nb) {
                    continue;
                }
                if matches!(
                    graph.nodes[nb].kind,
                    TraceNodeKind::Valve | TraceNodeKind::Instrument
                ) {
                    let c = graph.nodes[nb].center;
                    let inline_on_span = graph.nodes[ni].spans.iter().any(|&sp| {
                        let vx = sp.b.0 - sp.a.0;
                        let vy = sp.b.1 - sp.a.1;
                        let len2 = vx * vx + vy * vy;
                        if len2 <= 1e-9 {
                            false
                        } else {
                            let t = ((c.0 - sp.a.0) * vx + (c.1 - sp.a.1) * vy) / len2;
                            t > 0.03 && t < 0.97 && point_to_span_dist(c, sp) <= 4.0
                        }
                    });
                    if inline_on_span {
                        in_path.insert(nb);
                        enriched_path.push(nb);
                    }
                }
            }
        }
    }

    let dest_tab = graph.nodes[end_idx].tab_index;
    Ok(assemble_report(
        &graph,
        TraceMode::ShortestPath,
        true,
        &enriched_path,
        opc_bridges,
        dest_tab,
    ))
}

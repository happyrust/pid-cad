//! Multi-sheet P&ID Equipment Nozzle Schedule extractor, orientation classifier,
//! connected-pipeline spec & root-valve resolver, and spare/orphan nozzle auditor
//! (`PIDNOZZLE` & `--export-nozzles`, `L3-5` / `L3-6` / `L2-2` / `L6-2` / `#164`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::{format_nominal_diameter_attr, parse_sppid_line_number};
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::find::match_query_rank;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::classify_valve_role_and_subtype;

/// Canonical 10-column header order shared by CSV and TSV Equipment Nozzle Schedules.
pub const NOZZLE_CSV_HEADERS: [&str; 10] = [
    "DrawingCode",
    "EquipmentTag",
    "NozzleId",
    "Status",
    "Orientation",
    "Implicit",
    "NominalDiameter",
    "PipingSpec",
    "ConnectedLine",
    "RootValve",
];

/// Connection state of an extracted or synthesized equipment nozzle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NozzleStatus {
    Connected,
    SpareOrManway,
    OrphanNozzle,
}

impl NozzleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::SpareOrManway => "spare_or_manway",
            Self::OrphanNozzle => "orphan_nozzle",
        }
    }
}

/// Geometric side of the owning equipment shell where a nozzle sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NozzleOrientation {
    Top,
    Bottom,
    Left,
    Right,
    Unknown,
}

impl NozzleOrientation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
            Self::Unknown => "unknown",
        }
    }
}

/// One explicit or implicit equipment nozzle entry in `PidNozzleReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidNozzleItem {
    pub tab_index: usize,
    pub drawing_code: String,
    pub equipment_tag: String,
    pub equipment_class: String,
    pub nozzle_id: String,
    pub status: NozzleStatus,
    pub orientation: NozzleOrientation,
    pub implicit: bool,
    pub nominal_diameter: String,
    pub piping_spec: String,
    pub fluid_code: String,
    pub connected_line: String,
    pub root_valve: Option<String>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Summary statistics for `PIDNOZZLE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidNozzleSummary {
    pub sheets_scanned: usize,
    pub equipment_with_nozzles: usize,
    pub total_nozzles: usize,
    pub explicit_nozzles: usize,
    pub implicit_nozzles: usize,
    pub connected_nozzles: usize,
    pub spare_nozzles: usize,
    pub orphan_nozzles: usize,
}

/// Complete `PIDNOZZLE` Equipment Nozzle Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidNozzleReport {
    pub summary: PidNozzleSummary,
    pub nozzles: Vec<PidNozzleItem>,
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
enum NzKind {
    Equipment,
    Nozzle,
    Valve,
    PassiveOrOpc,
    PipeSegment,
}

struct NzNode {
    tab_index: usize,
    drawing_code: String,
    kind: NzKind,
    sppid_class: String,
    tag_or_label: Option<String>,
    own_dn: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
    upm: f64,
}

struct NzGraph {
    nodes: Vec<NzNode>,
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

fn closest_point_on_span(p: Pt2, s: Span2) -> Pt2 {
    let vx = s.b.0 - s.a.0;
    let vy = s.b.1 - s.a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return s.a;
    }
    let t = (((p.0 - s.a.0) * vx + (p.1 - s.a.1) * vy) / len2).clamp(0.0, 1.0);
    (s.a.0 + t * vx, s.a.1 + t * vy)
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

fn classify_nz_kind(cls: &str, type_code: &str, blk_upper: &str, tag_opt: Option<&str>) -> NzKind {
    if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        NzKind::Nozzle
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        NzKind::Equipment
    } else if classify_valve_role_and_subtype(cls, type_code, blk_upper, tag_opt).is_some() {
        NzKind::Valve
    } else {
        NzKind::PassiveOrOpc
    }
}

fn compute_orientation(eq_center: Pt2, eq_bbox: BBox2, pt: Pt2) -> NozzleOrientation {
    let hw = ((eq_bbox.max_x - eq_bbox.min_x) * 0.5).max(1e-3);
    let hh = ((eq_bbox.max_y - eq_bbox.min_y) * 0.5).max(1e-3);
    let rx = (pt.0 - eq_center.0) / hw;
    let ry = (pt.1 - eq_center.1) / hh;
    if rx.abs() >= ry.abs() {
        if rx >= 0.0 {
            NozzleOrientation::Right
        } else {
            NozzleOrientation::Left
        }
    } else if ry >= 0.0 {
        NozzleOrientation::Top
    } else {
        NozzleOrientation::Bottom
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

fn extract_authored_nz_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<NzNode>,
) {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let mut pipe_by_handle: BTreeMap<Handle, usize> = BTreeMap::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
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
                nodes.push(NzNode {
                    tab_index,
                    drawing_code: drawing_code.to_string(),
                    kind: NzKind::PipeSegment,
                    sppid_class: "PIDPipingNetworkSegment".to_string(),
                    tag_or_label: lbl,
                    own_dn: None,
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
        let tag_opt = (!auth.tag.trim().is_empty()).then(|| auth.tag.trim().to_string());
        let own_dn = pmap
            .get("NominalDiameter")
            .or_else(|| pmap.get("nominal_diameter"))
            .and_then(Value::as_str)
            .and_then(format_nominal_diameter_attr);
        let kind = classify_nz_kind(&cls, &auth.type_code, &blk_upper, tag_opt.as_deref());
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(NzNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            sppid_class: cls,
            tag_or_label: tag_opt,
            own_dn,
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
            let sp = Span2 {
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            };
            let (center, bbox) = spans_bbox_and_center(&[sp]);
            let idx = nodes.len();
            nodes.push(NzNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: NzKind::PipeSegment,
                sppid_class: "PIDPipingNetworkSegment".to_string(),
                tag_or_label: lt,
                own_dn: None,
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
}

fn extract_unpromoted_nz_nodes(
    tab_index: usize,
    fallback_name: &str,
    doc: &Document,
    nodes: &mut Vec<NzNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_nz_nodes(
            tab_index,
            &clean_drawing_stem(fallback_name),
            &scratch,
            nodes,
        );
        return;
    }

    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );
    let drawing_code = rec
        .title_block
        .drawing_number
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| clean_drawing_stem(fallback_name));

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
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let own_dn = sym
            .nominal_diameter
            .as_deref()
            .and_then(format_nominal_diameter_attr);
        let kind = classify_nz_kind(
            &sym.sppid_class,
            "",
            &sym.source_block.to_ascii_uppercase(),
            tag_opt.as_deref(),
        );

        nodes.push(NzNode {
            tab_index,
            drawing_code: drawing_code.clone(),
            kind,
            sppid_class: sym.sppid_class.clone(),
            tag_or_label: tag_opt,
            own_dn,
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
        nodes.push(NzNode {
            tab_index,
            drawing_code: drawing_code.clone(),
            kind: NzKind::PipeSegment,
            sppid_class: "PIDPipingNetworkSegment".to_string(),
            tag_or_label: lbl_opt,
            own_dn: None,
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

fn wire_nz_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[NzNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == NzKind::PipeSegment {
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
                                    nodes[sn].kind != NzKind::Equipment
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
                NzKind::Equipment => 12.0 * upm,
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
}

fn propagate_nz_pipe_labels(graph: &mut NzGraph) {
    let mut updates: Vec<(usize, String)> = Vec::new();
    for idx in 0..graph.nodes.len() {
        if graph.nodes[idx].kind != NzKind::PipeSegment || graph.nodes[idx].tag_or_label.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeSet<String> = BTreeSet::new();
        while let Some(cur) = q.pop_front() {
            if graph.nodes[cur].kind == NzKind::PipeSegment {
                let valid_lbl = graph.nodes[cur]
                    .tag_or_label
                    .as_deref()
                    .filter(|s| parse_sppid_line_number(s).is_some());
                if let Some(lbl) = valid_lbl {
                    found.insert(lbl.to_string());
                    continue;
                }
            }
            if matches!(
                graph.nodes[cur].kind,
                NzKind::Equipment | NzKind::PassiveOrOpc
            ) {
                continue;
            }
            for &nxt in &graph.adj[cur] {
                if visited.len() < 16 && visited.insert(nxt) {
                    q.push_back(nxt);
                }
            }
        }
        let only_opt = (found.len() == 1)
            .then(|| found.into_iter().next())
            .flatten();
        if let Some(only) = only_opt {
            updates.push((idx, only));
        }
    }
    for (idx, lbl) in updates {
        graph.nodes[idx].tag_or_label = Some(lbl);
    }
}

fn build_nz_graph(sheets: &[OpcSheetInput<'_>]) -> NzGraph {
    let mut nodes: Vec<NzNode> = Vec::new();
    let mut sheet_ranges: Vec<(f64, std::ops::Range<usize>)> = Vec::new();

    for sh in sheets {
        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_nz_nodes(
                sh.tab_index,
                &clean_drawing_stem(sh.fallback_name),
                sh.document,
                &mut nodes,
            );
        } else {
            extract_unpromoted_nz_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        }
        let end_idx = nodes.len();
        sheet_ranges.push((upm, start_idx..end_idx));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_nz_local_edges(&indices, *upm, &nodes, &mut adj);
    }

    let mut graph = NzGraph { nodes, adj };
    propagate_nz_pipe_labels(&mut graph);
    graph
}

struct BranchSpec {
    has_pipe: bool,
    line_number: String,
    nominal_diameter: String,
    fluid_code: String,
    piping_spec: String,
    root_valve: Option<String>,
}

fn trace_nozzle_branch_spec(graph: &NzGraph, start_idx: usize) -> BranchSpec {
    let mut visited: BTreeSet<usize> = BTreeSet::new();
    let mut q: VecDeque<(usize, usize)> = VecDeque::new();
    visited.insert(start_idx);
    q.push_back((start_idx, 0));

    let mut has_pipe = graph.nodes[start_idx].kind == NzKind::PipeSegment;
    let mut line_opt: Option<String> = None;
    let mut root_valve: Option<String> = None;

    while let Some((cur, depth)) = q.pop_front() {
        let cnd = &graph.nodes[cur];
        if cnd.kind == NzKind::PipeSegment {
            has_pipe = true;
            if line_opt.is_none() {
                line_opt = cnd
                    .tag_or_label
                    .as_deref()
                    .filter(|s| parse_sppid_line_number(s).is_some())
                    .map(str::to_string);
            }
        }
        if depth > 0 && root_valve.is_none() && cnd.kind == NzKind::Valve {
            root_valve = cnd.tag_or_label.clone();
        }
        if depth >= 6 || (depth > 0 && matches!(cnd.kind, NzKind::Equipment | NzKind::Nozzle)) {
            continue;
        }
        for &nxt in &graph.adj[cur] {
            if graph.nodes[nxt].kind == NzKind::Equipment {
                continue;
            }
            if visited.insert(nxt) {
                q.push_back((nxt, depth + 1));
            }
        }
    }

    let line_number = line_opt.unwrap_or_default();
    let parsed = parse_sppid_line_number(&line_number);
    let nominal_diameter = parsed
        .as_ref()
        .map(|p| p.nominal_diameter_attr.clone())
        .unwrap_or_default();
    let fluid_code = parsed
        .as_ref()
        .map(|p| p.fluid_code.to_ascii_uppercase())
        .unwrap_or_default();
    let piping_spec = parsed
        .as_ref()
        .map(|p| p.materials_class.clone())
        .unwrap_or_default();

    BranchSpec {
        has_pipe,
        line_number,
        nominal_diameter,
        fluid_code,
        piping_spec,
        root_valve,
    }
}

/// Extract explicit `PIDNozzle` symbols and synthesize implicit shell connections across `sheets`,
/// resolving nozzle orientations, connected line specs, root valves, spares, and orphans (`#164`).
pub fn analyze_pid_equipment_nozzles(sheets: &[OpcSheetInput<'_>]) -> PidNozzleReport {
    let graph = build_nz_graph(sheets);

    let equip_indices: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.kind == NzKind::Equipment).then_some(i))
        .collect();

    let mut explicit_by_equip: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut orphan_indices: Vec<usize> = Vec::new();

    for (ni, nd) in graph.nodes.iter().enumerate() {
        if nd.kind != NzKind::Nozzle {
            continue;
        }
        let margin = 16.0 * nd.upm;
        let mut best_eq: Option<(f64, usize)> = None;
        for &ei in &equip_indices {
            if graph.nodes[ei].tab_index != nd.tab_index {
                continue;
            }
            if bboxes_overlap(nd.bbox, graph.nodes[ei].bbox, margin) {
                let d = (nd.center.0 - graph.nodes[ei].center.0)
                    .hypot(nd.center.1 - graph.nodes[ei].center.1);
                if best_eq.is_none_or(|(bd, _)| d < bd) {
                    best_eq = Some((d, ei));
                }
            }
        }
        if let Some((_, ei)) = best_eq {
            explicit_by_equip.entry(ei).or_default().push(ni);
        } else {
            orphan_indices.push(ni);
        }
    }

    let mut out_nozzles: Vec<PidNozzleItem> = Vec::new();
    let mut equip_with_nozzles_set: BTreeSet<(usize, String)> = BTreeSet::new();

    // 1. Emit owned explicit + synthesized implicit nozzles per equipment
    for &ei in &equip_indices {
        let eq = &graph.nodes[ei];
        let eq_tag = eq
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("UNTAGGED-{:X}", eq.primary_handle.value()));

        let exp_list = explicit_by_equip.get(&ei).cloned().unwrap_or_default();
        let mut seq = 0usize;
        let mut covered_pts: Vec<Pt2> = Vec::new();

        for &ni in &exp_list {
            let nnd = &graph.nodes[ni];
            covered_pts.push(nnd.center);
            seq += 1;
            let nid = nnd
                .tag_or_label
                .clone()
                .unwrap_or_else(|| format!("N{seq}"));
            let orient = compute_orientation(eq.center, eq.bbox, nnd.center);
            let bspec = trace_nozzle_branch_spec(&graph, ni);
            let status = if bspec.has_pipe {
                NozzleStatus::Connected
            } else {
                NozzleStatus::SpareOrManway
            };
            let dn = if !bspec.nominal_diameter.is_empty() {
                bspec.nominal_diameter
            } else {
                nnd.own_dn.clone().unwrap_or_default()
            };

            equip_with_nozzles_set.insert((eq.tab_index, eq_tag.clone()));
            out_nozzles.push(PidNozzleItem {
                tab_index: nnd.tab_index,
                drawing_code: nnd.drawing_code.clone(),
                equipment_tag: eq_tag.clone(),
                equipment_class: eq.sppid_class.clone(),
                nozzle_id: nid,
                status,
                orientation: orient,
                implicit: false,
                nominal_diameter: dn,
                piping_spec: bspec.piping_spec,
                fluid_code: bspec.fluid_code,
                connected_line: bspec.line_number,
                root_valve: bspec.root_valve,
                primary_handle_hex: format!("{:X}", nnd.primary_handle.value()),
                x: nnd.center.0,
                y: nnd.center.1,
                primary_handle: nnd.primary_handle,
                member_handles: nnd.member_handles.clone(),
            });
        }

        // Synthesize implicit nozzles for pipe segments touching `eq` away from any explicit nozzle
        for &nb in &graph.adj[ei] {
            let pnd = &graph.nodes[nb];
            if pnd.kind != NzKind::PipeSegment || pnd.spans.is_empty() {
                continue;
            }
            let sp = pnd.spans[0];
            let touch_pt = closest_point_on_span(eq.center, sp);
            let near_explicit = exp_list.iter().any(|&ni| {
                let c = graph.nodes[ni].center;
                (c.0 - touch_pt.0).hypot(c.1 - touch_pt.1) <= 14.0 * eq.upm
            });
            if near_explicit {
                continue;
            }
            let near_existing_imp = covered_pts
                .iter()
                .any(|&c| (c.0 - touch_pt.0).hypot(c.1 - touch_pt.1) <= 6.0 * eq.upm);
            if near_existing_imp {
                continue;
            }
            covered_pts.push(touch_pt);
            seq += 1;
            let orient = compute_orientation(eq.center, eq.bbox, touch_pt);
            let bspec = trace_nozzle_branch_spec(&graph, nb);

            let mut mems = pnd.member_handles.clone();
            if !mems.contains(&eq.primary_handle) {
                mems.push(eq.primary_handle);
            }
            equip_with_nozzles_set.insert((eq.tab_index, eq_tag.clone()));
            out_nozzles.push(PidNozzleItem {
                tab_index: pnd.tab_index,
                drawing_code: pnd.drawing_code.clone(),
                equipment_tag: eq_tag.clone(),
                equipment_class: eq.sppid_class.clone(),
                nozzle_id: format!("N{seq}"),
                status: NozzleStatus::Connected,
                orientation: orient,
                implicit: true,
                nominal_diameter: bspec.nominal_diameter,
                piping_spec: bspec.piping_spec,
                fluid_code: bspec.fluid_code,
                connected_line: bspec.line_number,
                root_valve: bspec.root_valve,
                primary_handle_hex: format!("{:X}", pnd.primary_handle.value()),
                x: touch_pt.0,
                y: touch_pt.1,
                primary_handle: pnd.primary_handle,
                member_handles: mems,
            });
        }
    }

    // 2. Emit unowned orphan explicit nozzles
    for (oi, &ni) in orphan_indices.iter().enumerate() {
        let nnd = &graph.nodes[ni];
        let bspec = trace_nozzle_branch_spec(&graph, ni);
        let dn = if !bspec.nominal_diameter.is_empty() {
            bspec.nominal_diameter
        } else {
            nnd.own_dn.clone().unwrap_or_default()
        };
        let nid = nnd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("ORPHAN-{}", oi + 1));
        out_nozzles.push(PidNozzleItem {
            tab_index: nnd.tab_index,
            drawing_code: nnd.drawing_code.clone(),
            equipment_tag: String::new(),
            equipment_class: String::new(),
            nozzle_id: nid,
            status: NozzleStatus::OrphanNozzle,
            orientation: NozzleOrientation::Unknown,
            implicit: false,
            nominal_diameter: dn,
            piping_spec: bspec.piping_spec,
            fluid_code: bspec.fluid_code,
            connected_line: bspec.line_number,
            root_valve: bspec.root_valve,
            primary_handle_hex: format!("{:X}", nnd.primary_handle.value()),
            x: nnd.center.0,
            y: nnd.center.1,
            primary_handle: nnd.primary_handle,
            member_handles: nnd.member_handles.clone(),
        });
    }

    let implicit_nozzles = out_nozzles.iter().filter(|n| n.implicit).count();
    let explicit_nozzles = out_nozzles.len().saturating_sub(implicit_nozzles);
    let connected_nozzles = out_nozzles
        .iter()
        .filter(|n| n.status == NozzleStatus::Connected)
        .count();
    let spare_nozzles = out_nozzles
        .iter()
        .filter(|n| n.status == NozzleStatus::SpareOrManway)
        .count();
    let orphan_nozzles = out_nozzles
        .iter()
        .filter(|n| n.status == NozzleStatus::OrphanNozzle)
        .count();

    PidNozzleReport {
        summary: PidNozzleSummary {
            sheets_scanned: sheets.len(),
            equipment_with_nozzles: equip_with_nozzles_set.len(),
            total_nozzles: out_nozzles.len(),
            explicit_nozzles,
            implicit_nozzles,
            connected_nozzles,
            spare_nozzles,
            orphan_nozzles,
        },
        nozzles: out_nozzles,
    }
}

/// Check whether nozzle `nz` matches equipment query `query`.
pub fn nozzle_matches_equipment(nz: &PidNozzleItem, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() || nz.equipment_tag.is_empty() {
        return false;
    }
    match_query_rank(&nz.equipment_tag, q).is_some()
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and extract an Equipment Nozzle Schedule (`#164`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_nozzles_from_path(input_path: &Path) -> Result<PidNozzleReport, String> {
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
    Ok(analyze_pid_equipment_nozzles(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidNozzleReport` as delimited Equipment Nozzle Schedule text with UTF-8 BOM.
pub fn format_pid_nozzle_delimited(report: &PidNozzleReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = NOZZLE_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for nz in &report.nozzles {
        let imp_str = if nz.implicit { "true" } else { "false" };
        let cells = [
            nz.drawing_code.as_str(),
            nz.equipment_tag.as_str(),
            nz.nozzle_id.as_str(),
            nz.status.as_str(),
            nz.orientation.as_str(),
            imp_str,
            nz.nominal_diameter.as_str(),
            nz.piping_spec.as_str(),
            nz.connected_line.as_str(),
            nz.root_valve.as_deref().unwrap_or(""),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidNozzleReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#164`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_nozzle_file(report: &PidNozzleReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize nozzle JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_nozzle_delimited(report, '\t'),
        "csv" => format_pid_nozzle_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported nozzle schedule extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

//! Multi-sheet P&ID Piping Specialty Items (SP-Items) & Fittings Schedule extractor,
//! dual-size reducer resolver (`LargeDN x SmallDN`), and MTO rollup exporter
//! (`PIDSPECIALTY` & `--export-specialties`, `L3-3` / `L3-4` / `L3-6` / `L6-2` / `#166`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::classify_valve_role_and_subtype;

/// Canonical 10-column header order shared by CSV and TSV Piping Specialty Schedules.
pub const SPECIALTY_CSV_HEADERS: [&str; 10] = [
    "RecordKind",
    "TagOrRollupKey",
    "Category",
    "Subtype",
    "SizeSpec",
    "PipingSpec",
    "FluidOrQuantity",
    "LineOrSampleTags",
    "DrawingCodes",
    "AnchorXY",
];

/// High-level piping procurement category for non-valve inline components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialtyCategory {
    SpecialtyItem,
    ReducerFitting,
    FlangeOrCap,
}

impl SpecialtyCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpecialtyItem => "specialty_item",
            Self::ReducerFitting => "reducer_fitting",
            Self::FlangeOrCap => "flange_or_cap",
        }
    }
}

/// Detailed functional subtype for a piping specialty item or inline fitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecialtySubtype {
    StrainerFilter,
    FlameArrester,
    FlexibleHoseJoint,
    BlindOrificeSpacer,
    SteamTrapOrSight,
    MiscSpecialty,
    Reducer,
    FlangeCap,
}

impl SpecialtySubtype {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrainerFilter => "strainer_filter",
            Self::FlameArrester => "flame_arrester",
            Self::FlexibleHoseJoint => "flexible_hose_joint",
            Self::BlindOrificeSpacer => "blind_orifice_spacer",
            Self::SteamTrapOrSight => "steam_trap_or_sight",
            Self::MiscSpecialty => "misc_specialty",
            Self::Reducer => "reducer",
            Self::FlangeCap => "flange_cap",
        }
    }
}

/// One extracted piping specialty item or inline fitting row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidSpecialtyItem {
    pub tab_index: usize,
    pub drawing_code: String,
    pub item_tag: String,
    pub category: SpecialtyCategory,
    pub subtype: SpecialtySubtype,
    pub source_block: String,
    pub spec_resolved: bool,
    pub size_spec: String,
    pub piping_spec: String,
    pub fluid_code: String,
    pub line_number: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// One aggregated procurement rollup row `(category, subtype, size_spec, piping_spec) -> Qty`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidSpecialtyRollupRow {
    pub category: SpecialtyCategory,
    pub subtype: SpecialtySubtype,
    pub size_spec: String,
    pub piping_spec: String,
    pub quantity: usize,
    pub drawing_codes: Vec<String>,
    pub sample_tags: Vec<String>,
}

/// Summary statistics for `PIDSPECIALTY`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidSpecialtySummary {
    pub sheets_scanned: usize,
    pub total_items: usize,
    pub specialty_items_count: usize,
    pub reducers_count: usize,
    pub flanges_or_caps_count: usize,
    pub spec_resolved_count: usize,
    pub rollup_groups_count: usize,
}

/// Complete `PIDSPECIALTY` Specialty Items & Fittings Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidSpecialtyReport {
    pub summary: PidSpecialtySummary,
    pub mto_rollup: Vec<PidSpecialtyRollupRow>,
    pub items: Vec<PidSpecialtyItem>,
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
enum SpKind {
    Component(SpecialtyCategory, SpecialtySubtype),
    ValveOrInst,
    EquipmentOrOpc,
    PipeSegment,
}

struct SpNode {
    tab_index: usize,
    drawing_code: String,
    kind: SpKind,
    source_block: String,
    tag_or_label: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct SpGraph {
    nodes: Vec<SpNode>,
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

fn approx_diameter_mm(dn_attr: &str) -> f64 {
    let s = dn_attr.trim();
    if let Some(num) = s
        .strip_suffix(" mm")
        .and_then(|v| v.trim().parse::<f64>().ok())
    {
        return num;
    }
    if let Some(raw_in) = s.strip_suffix(" in").map(str::trim) {
        if let Ok(v) = raw_in.parse::<f64>() {
            return v * 25.4;
        }
        if let Some((whole, frac)) = raw_in.split_once('-') {
            if let Some((n, d)) = frac.split_once('/') {
                let w = whole.parse::<f64>().unwrap_or(0.0);
                let nf = n.parse::<f64>().unwrap_or(0.0);
                let df = d.parse::<f64>().unwrap_or(1.0).max(1.0);
                return (w + nf / df) * 25.4;
            }
        } else if let Some((n, d)) = raw_in.split_once('/') {
            let nf = n.parse::<f64>().unwrap_or(0.0);
            let df = d.parse::<f64>().unwrap_or(1.0).max(1.0);
            return (nf / df) * 25.4;
        }
    }
    0.0
}

/// Classify a non-valve `PIDPipingComponent` into `(SpecialtyCategory, SpecialtySubtype)`.
pub fn classify_specialty_component(
    cls: &str,
    type_code: &str,
    blk_upper: &str,
    tag_opt: Option<&str>,
) -> Option<(SpecialtyCategory, SpecialtySubtype)> {
    // Strictly orthogonal to `PIDVALVE`: never double-count valves
    if classify_valve_role_and_subtype(cls, type_code, blk_upper, tag_opt).is_some() {
        return None;
    }

    let is_piping_comp = cls == "PIDPipingComponent"
        || type_code.eq_ignore_ascii_case("fitting")
        || type_code.eq_ignore_ascii_case("specialty");
    if !is_piping_comp {
        return None;
    }

    if blk_upper.contains("REDUCER") {
        return Some((SpecialtyCategory::ReducerFitting, SpecialtySubtype::Reducer));
    }

    if blk_upper.contains("STRAINER")
        || blk_upper.contains("FILTER")
        || blk_upper.contains("SEPARATOR")
    {
        return Some((
            SpecialtyCategory::SpecialtyItem,
            SpecialtySubtype::StrainerFilter,
        ));
    }

    if blk_upper.contains("ARRESTOR") || blk_upper.contains("ARRESTER") {
        return Some((
            SpecialtyCategory::SpecialtyItem,
            SpecialtySubtype::FlameArrester,
        ));
    }

    if blk_upper.contains("HOSE")
        || blk_upper.contains("EXPANSION")
        || blk_upper.contains("BELLOW")
        || blk_upper.contains("COUPLING")
        || blk_upper.contains("LOADING-ARM")
        || blk_upper.contains("INSULATION JOINT")
    {
        return Some((
            SpecialtyCategory::SpecialtyItem,
            SpecialtySubtype::FlexibleHoseJoint,
        ));
    }

    if blk_upper.contains("SPECTACLE")
        || blk_upper.contains("SPACER")
        || blk_upper.contains("FLANGE BLIND")
        || blk_upper.contains("ORIFICE")
        || blk_upper.contains("RESTRICTION")
    {
        return Some((
            SpecialtyCategory::SpecialtyItem,
            SpecialtySubtype::BlindOrificeSpacer,
        ));
    }

    if blk_upper.contains("TRAP") || blk_upper.contains("SIGHT") {
        return Some((
            SpecialtyCategory::SpecialtyItem,
            SpecialtySubtype::SteamTrapOrSight,
        ));
    }

    if blk_upper.contains("FLANGE")
        || blk_upper.contains("PIPE-CAP")
        || blk_upper == "CAP"
        || blk_upper == "RC"
        || blk_upper == "PIPE-UNKNOWN"
        || blk_upper == "FOAM-PORT"
    {
        return Some((SpecialtyCategory::FlangeOrCap, SpecialtySubtype::FlangeCap));
    }

    Some((
        SpecialtyCategory::SpecialtyItem,
        SpecialtySubtype::MiscSpecialty,
    ))
}

fn classify_sp_kind(cls: &str, type_code: &str, blk_upper: &str, tag_opt: Option<&str>) -> SpKind {
    if let Some((cat, sub)) = classify_specialty_component(cls, type_code, blk_upper, tag_opt) {
        return SpKind::Component(cat, sub);
    }
    if cls == "PIDBranchPoint"
        || type_code.eq_ignore_ascii_case("opc")
        || matches!(
            cls,
            "PIDProcessVessel"
                | "PIDMechanicalEquipment"
                | "PIDHeatTransferEquipment"
                | "PIDHeatTransferEquip"
                | "PIDProcessEquipment"
        )
        || type_code.eq_ignore_ascii_case("equipment")
    {
        SpKind::EquipmentOrOpc
    } else {
        SpKind::ValveOrInst
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

fn extract_authored_sp_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<SpNode>,
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
                nodes.push(SpNode {
                    tab_index,
                    drawing_code: drawing_code.to_string(),
                    kind: SpKind::PipeSegment,
                    source_block: String::new(),
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
            .unwrap_or(&auth.type_code);
        let src_blk = match ent {
            EntityType::Insert(ins) => ins.block_name.clone(),
            _ => pmap
                .get("_source_block")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        let tag_opt = (!auth.tag.trim().is_empty()).then(|| auth.tag.trim().to_string());
        let sym_path = pmap
            .get("_symbol_path")
            .and_then(Value::as_str)
            .unwrap_or("");
        let desc_up = format!("{src_blk} {sym_path}").to_ascii_uppercase();
        let kind = classify_sp_kind(cls, &auth.type_code, &desc_up, tag_opt.as_deref());
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(SpNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            source_block: src_blk,
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
            let sp = Span2 {
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            };
            let (center, bbox) = spans_bbox_and_center(&[sp]);
            let idx = nodes.len();
            nodes.push(SpNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: SpKind::PipeSegment,
                source_block: String::new(),
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
}

fn extract_unpromoted_sp_nodes(
    tab_index: usize,
    fallback_name: &str,
    doc: &Document,
    nodes: &mut Vec<SpNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_sp_nodes(
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
        let desc_up = format!(
            "{} {} {}",
            sym.source_block,
            sym.layer,
            sym.symbol_path.as_deref().unwrap_or("")
        )
        .to_ascii_uppercase();
        let kind = classify_sp_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref());

        nodes.push(SpNode {
            tab_index,
            drawing_code: drawing_code.clone(),
            kind,
            source_block: sym.source_block.clone(),
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
        nodes.push(SpNode {
            tab_index,
            drawing_code: drawing_code.clone(),
            kind: SpKind::PipeSegment,
            source_block: String::new(),
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

fn wire_sp_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[SpNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == SpKind::PipeSegment {
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
                                sym_nodes
                                    .iter()
                                    .any(|&sn| point_in_bbox(mid, nodes[sn].bbox, 1.5 * upm))
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
                SpKind::EquipmentOrOpc => 12.0 * upm,
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

fn is_reducer_kind(kind: SpKind) -> bool {
    matches!(
        kind,
        SpKind::Component(SpecialtyCategory::ReducerFitting, SpecialtySubtype::Reducer)
    )
}

fn propagate_sp_pipe_labels(graph: &mut SpGraph) {
    let mut updates: Vec<(usize, String)> = Vec::new();
    for idx in 0..graph.nodes.len() {
        if graph.nodes[idx].kind != SpKind::PipeSegment || graph.nodes[idx].tag_or_label.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeSet<String> = BTreeSet::new();
        while let Some(cur) = q.pop_front() {
            if graph.nodes[cur].kind == SpKind::PipeSegment {
                let valid_lbl = graph.nodes[cur]
                    .tag_or_label
                    .as_deref()
                    .filter(|s| parse_sppid_line_number(s).is_some());
                if let Some(lbl) = valid_lbl {
                    found.insert(lbl.to_string());
                    continue;
                }
            }
            // Stop at Equipment/OPC AND at Reducers so distinct sizes across a reducer stay separate
            if graph.nodes[cur].kind == SpKind::EquipmentOrOpc
                || is_reducer_kind(graph.nodes[cur].kind)
            {
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

fn build_sp_graph(sheets: &[OpcSheetInput<'_>]) -> SpGraph {
    let mut nodes: Vec<SpNode> = Vec::new();
    let mut sheet_ranges: Vec<(f64, std::ops::Range<usize>)> = Vec::new();

    for sh in sheets {
        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_sp_nodes(
                sh.tab_index,
                &clean_drawing_stem(sh.fallback_name),
                sh.document,
                &mut nodes,
            );
        } else {
            extract_unpromoted_sp_nodes(sh.tab_index, sh.fallback_name, sh.document, &mut nodes);
        }
        let end_idx = nodes.len();
        sheet_ranges.push((upm, start_idx..end_idx));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_sp_local_edges(&indices, *upm, &nodes, &mut adj);
    }

    let mut graph = SpGraph { nodes, adj };
    propagate_sp_pipe_labels(&mut graph);
    graph
}

fn trace_branch_line(graph: &SpGraph, blocked_root: usize, branch_start: usize) -> Option<String> {
    let mut visited: BTreeSet<usize> = BTreeSet::new();
    visited.insert(blocked_root);
    visited.insert(branch_start);
    let mut q: VecDeque<(usize, usize)> = VecDeque::new();
    q.push_back((branch_start, 1));

    while let Some((cur, depth)) = q.pop_front() {
        if graph.nodes[cur].kind == SpKind::PipeSegment {
            let valid_lbl = graph.nodes[cur]
                .tag_or_label
                .as_deref()
                .filter(|s| parse_sppid_line_number(s).is_some());
            if let Some(lbl) = valid_lbl {
                return Some(lbl.to_string());
            }
        }
        if depth >= 6
            || graph.nodes[cur].kind == SpKind::EquipmentOrOpc
            || is_reducer_kind(graph.nodes[cur].kind)
        {
            continue;
        }
        for &nxt in &graph.adj[cur] {
            if visited.insert(nxt) {
                q.push_back((nxt, depth + 1));
            }
        }
    }
    None
}

/// Extract all non-valve piping specialty items and inline fittings across `sheets`,
/// resolving dual-diameter reducer specs (`LargeDN x SmallDN`) and MTO rollups (`#166`).
pub fn analyze_pid_specialty_items(sheets: &[OpcSheetInput<'_>]) -> PidSpecialtyReport {
    let graph = build_sp_graph(sheets);
    let mut items: Vec<PidSpecialtyItem> = Vec::new();

    struct RollupAccum {
        quantity: usize,
        drawings: BTreeSet<String>,
        sample_tags: Vec<String>,
    }
    let mut rollup_map: BTreeMap<
        (SpecialtyCategory, SpecialtySubtype, String, String),
        RollupAccum,
    > = BTreeMap::new();

    for (si, nd) in graph.nodes.iter().enumerate() {
        let SpKind::Component(cat, sub) = nd.kind else {
            continue;
        };

        let mut branch_lines: Vec<String> = Vec::new();
        for &nb in &graph.adj[si] {
            if let Some(lbl) =
                trace_branch_line(&graph, si, nb).filter(|l| !branch_lines.contains(l))
            {
                branch_lines.push(lbl);
            }
        }

        let mut sizes: Vec<String> = Vec::new();
        let mut first_spec = String::new();
        let mut first_fluid = String::new();
        let mut first_line = String::new();

        for lbl in &branch_lines {
            if let Some(parsed) = parse_sppid_line_number(lbl) {
                if first_line.is_empty() {
                    first_line = lbl.clone();
                }
                if first_spec.is_empty() && !parsed.materials_class.is_empty() {
                    first_spec = parsed.materials_class;
                }
                if first_fluid.is_empty() && !parsed.fluid_code.is_empty() {
                    first_fluid = parsed.fluid_code.to_ascii_uppercase();
                }
                if !parsed.nominal_diameter_attr.is_empty()
                    && !sizes.contains(&parsed.nominal_diameter_attr)
                {
                    sizes.push(parsed.nominal_diameter_attr);
                }
            }
        }

        sizes.sort_by(|a, b| {
            approx_diameter_mm(b)
                .partial_cmp(&approx_diameter_mm(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });

        let size_spec = if sub == SpecialtySubtype::Reducer && sizes.len() >= 2 {
            format!("{} x {}", sizes[0], sizes[1])
        } else {
            sizes.first().cloned().unwrap_or_default()
        };
        let spec_resolved = !size_spec.is_empty();

        let itag = nd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("SP-{:X}", nd.primary_handle.value()));

        if spec_resolved {
            let key = (cat, sub, size_spec.clone(), first_spec.clone());
            let acc = rollup_map.entry(key).or_insert_with(|| RollupAccum {
                quantity: 0,
                drawings: BTreeSet::new(),
                sample_tags: Vec::new(),
            });
            acc.quantity += 1;
            acc.drawings.insert(nd.drawing_code.clone());
            if acc.sample_tags.len() < 6 && !acc.sample_tags.contains(&itag) {
                acc.sample_tags.push(itag.clone());
            }
        }

        items.push(PidSpecialtyItem {
            tab_index: nd.tab_index,
            drawing_code: nd.drawing_code.clone(),
            item_tag: itag,
            category: cat,
            subtype: sub,
            source_block: nd.source_block.clone(),
            spec_resolved,
            size_spec,
            piping_spec: first_spec,
            fluid_code: first_fluid,
            line_number: first_line,
            primary_handle_hex: format!("{:X}", nd.primary_handle.value()),
            x: nd.center.0,
            y: nd.center.1,
            primary_handle: nd.primary_handle,
            member_handles: nd.member_handles.clone(),
        });
    }

    let mut mto_rollup: Vec<PidSpecialtyRollupRow> = rollup_map
        .into_iter()
        .map(
            |((category, subtype, size_spec, piping_spec), acc)| PidSpecialtyRollupRow {
                category,
                subtype,
                size_spec,
                piping_spec,
                quantity: acc.quantity,
                drawing_codes: acc.drawings.into_iter().collect(),
                sample_tags: acc.sample_tags,
            },
        )
        .collect();
    mto_rollup.sort_by(|a, b| {
        b.quantity
            .cmp(&a.quantity)
            .then_with(|| a.subtype.cmp(&b.subtype))
            .then_with(|| a.size_spec.cmp(&b.size_spec))
    });

    let specialty_items_count = items
        .iter()
        .filter(|i| i.category == SpecialtyCategory::SpecialtyItem)
        .count();
    let reducers_count = items
        .iter()
        .filter(|i| i.category == SpecialtyCategory::ReducerFitting)
        .count();
    let flanges_or_caps_count = items
        .iter()
        .filter(|i| i.category == SpecialtyCategory::FlangeOrCap)
        .count();
    let spec_resolved_count = items.iter().filter(|i| i.spec_resolved).count();

    PidSpecialtyReport {
        summary: PidSpecialtySummary {
            sheets_scanned: sheets.len(),
            total_items: items.len(),
            specialty_items_count,
            reducers_count,
            flanges_or_caps_count,
            spec_resolved_count,
            rollup_groups_count: mto_rollup.len(),
        },
        mto_rollup,
        items,
    }
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and extract a Piping Specialty Schedule (`#166`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_specialties_from_path(input_path: &Path) -> Result<PidSpecialtyReport, String> {
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
    Ok(analyze_pid_specialty_items(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidSpecialtyReport` as delimited Specialty Rollup + Item Schedule text with UTF-8 BOM.
pub fn format_pid_specialty_delimited(report: &PidSpecialtyReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = SPECIALTY_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for r in &report.mto_rollup {
        let key = format!("{}|{}|{}", r.subtype.as_str(), r.size_spec, r.piping_spec);
        let qty = r.quantity.to_string();
        let samples = r.sample_tags.join(";");
        let dwgs = r.drawing_codes.join(";");
        let cells = [
            "SPECIALTY_ROLLUP",
            key.as_str(),
            r.category.as_str(),
            r.subtype.as_str(),
            r.size_spec.as_str(),
            r.piping_spec.as_str(),
            qty.as_str(),
            samples.as_str(),
            dwgs.as_str(),
            "",
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }

    for it in &report.items {
        let xy = format!("{:.1},{:.1}", it.x, it.y);
        let cells = [
            "SPECIALTY_ITEM",
            it.item_tag.as_str(),
            it.category.as_str(),
            it.subtype.as_str(),
            it.size_spec.as_str(),
            it.piping_spec.as_str(),
            it.fluid_code.as_str(),
            it.line_number.as_str(),
            it.drawing_code.as_str(),
            xy.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidSpecialtyReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#166`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_specialty_file(
    report: &PidSpecialtyReport,
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
                .map_err(|e| format!("serialize specialty JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_specialty_delimited(report, '\t'),
        "csv" => format_pid_specialty_delimited(report, ','),
        other => {
            return Err(format!(
            "unsupported specialty schedule extension '.{other}' (expected .json, .csv, or .tsv)"
        ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

//! Multi-sheet P&ID utility service classifier, consumption summary aggregator,
//! and battery-limit external tie-in schedule exporter (`PIDUTILITY` & `--export-utilities`,
//! `L3-3` / `L3-4` / `L3-6` / `L6-2` / `#161`).

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
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Canonical 10-column header order shared by CSV and TSV Utility & Battery-Limit Schedules.
pub const UTILITY_CSV_HEADERS: [&str; 10] = [
    "RecordKind",
    "IdOrService",
    "Category",
    "DrawingCodes",
    "LineOrCount",
    "NominalDiameter",
    "PipingSpec",
    "ExternalTarget",
    "BoundaryValveOrMeter",
    "AnchorXY",
];

/// Engineering service category classified from a P&ID line-number fluid service code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FluidCategory {
    UtilitySteamCondensate,
    UtilityWater,
    UtilityAirGas,
    UtilityDrainRelief,
    ProcessFluid,
}

impl FluidCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UtilitySteamCondensate => "utility_steam_condensate",
            Self::UtilityWater => "utility_water",
            Self::UtilityAirGas => "utility_air_gas",
            Self::UtilityDrainRelief => "utility_drain_relief",
            Self::ProcessFluid => "process_fluid",
        }
    }

    pub fn is_utility(self) -> bool {
        !matches!(self, Self::ProcessFluid)
    }
}

/// Classify a P&ID line-number `fluid_code` into utility sub-systems vs main process service.
pub fn classify_fluid_service(fluid_code: &str) -> FluidCategory {
    let code = fluid_code.trim().to_ascii_uppercase();
    match code.as_str() {
        "LS" | "LP" | "LPS" | "LLPS" | "MS" | "MP" | "MPS" | "HS" | "HP" | "HPS" | "SS" | "STM"
        | "SC" | "LC" | "MC" | "LPC" | "MPC" | "HPC" | "BFW" | "COND" | "CD" => {
            FluidCategory::UtilitySteamCondensate
        }
        "CW" | "CWS" | "CWR" | "CCWS" | "CCWR" | "CHWS" | "CHWR" | "PW" | "DW" | "FW" | "FWS"
        | "RW" | "SW" | "TW" | "UW" | "WW" | "IW" | "DM" | "DMW" | "RO" | "CI" | "WS" | "WR"
        | "HWS" | "HWR" => FluidCategory::UtilityWater,
        "IA" | "PA" | "CA" | "AIR" | "N2" | "GN" | "LN" | "HN" | "MN" | "FG" | "NG" | "IG"
        | "O2" | "GOX" => FluidCategory::UtilityAirGas,
        "BD" | "CBD" | "OBD" | "DR" | "OWS" | "CDR" | "ODR" | "FL" | "FLH" | "FLL" | "VNT"
        | "VT" | "ATM" | "SLOP" | "VD" | "SD" | "HD" | "LD" | "RV" | "SV" => {
            FluidCategory::UtilityDrainRelief
        }
        _ => FluidCategory::ProcessFluid,
    }
}

/// Aggregated summary row for one fluid service code across open sheets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidServiceSummaryRow {
    pub fluid_code: String,
    pub category: FluidCategory,
    pub is_utility: bool,
    pub pipeline_count: usize,
    pub segment_count: usize,
    pub nominal_sizes: Vec<String>,
    pub piping_specs: Vec<String>,
    pub drawing_codes: Vec<String>,
    pub sample_lines: Vec<String>,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// One external battery-limit / off-unit `OPC` tie-in interface row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidBatteryLimitInterface {
    pub interface_id: String,
    pub tab_index: usize,
    pub drawing_code: String,
    pub external_target: String,
    pub line_number: String,
    pub fluid_code: String,
    pub category: FluidCategory,
    pub is_utility: bool,
    pub nominal_diameter: String,
    pub piping_spec: String,
    pub nearest_valve_or_meter: Option<String>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Summary statistics for `PIDUTILITY`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidUtilitySummary {
    pub sheets_scanned: usize,
    pub total_services: usize,
    pub utility_services_count: usize,
    pub process_services_count: usize,
    pub labeled_pipelines_count: usize,
    pub utility_pipelines_count: usize,
    pub battery_limit_count: usize,
    pub utility_battery_limits: usize,
}

/// Complete `PIDUTILITY` Utility & Battery-Limit Interface Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidUtilityReport {
    pub summary: PidUtilitySummary,
    pub service_groups: Vec<PidServiceSummaryRow>,
    pub battery_limits: Vec<PidBatteryLimitInterface>,
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
enum UtKind {
    Equipment,
    ValveOrMeter,
    PassiveFitting,
    Opc,
    PipeSegment,
}

struct UtNode {
    tab_index: usize,
    drawing_code: String,
    kind: UtKind,
    tag_or_label: Option<String>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct UtGraph {
    nodes: Vec<UtNode>,
    adj: Vec<Vec<usize>>,
    opc_node_by_key: BTreeMap<(usize, Handle), usize>,
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

fn classify_ut_kind(cls: &str, type_code: &str, blk_upper: &str) -> UtKind {
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        UtKind::Opc
    } else if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || type_code.eq_ignore_ascii_case("equipment")
    {
        UtKind::Equipment
    } else if matches!(cls, "PIDInlineInstrument" | "PIDInstrument")
        || type_code.eq_ignore_ascii_case("instrument")
    {
        UtKind::ValveOrMeter
    } else if cls == "PIDPipingComponent" || type_code.eq_ignore_ascii_case("valve") {
        if is_passive_fitting(blk_upper) {
            UtKind::PassiveFitting
        } else {
            UtKind::ValveOrMeter
        }
    } else {
        UtKind::PassiveFitting
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

fn extract_authored_ut_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<UtNode>,
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
                nodes.push(UtNode {
                    tab_index,
                    drawing_code: drawing_code.to_string(),
                    kind: UtKind::PipeSegment,
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
        let kind = classify_ut_kind(cls, &auth.type_code, &blk_upper);
        let mut ports = Vec::new();
        if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
            for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                if let Some(pt) = parse_pair_f64(item) {
                    ports.push(pt);
                }
            }
        }

        nodes.push(UtNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
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
            nodes.push(UtNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: UtKind::PipeSegment,
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

fn extract_unpromoted_ut_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    nodes: &mut Vec<UtNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_ut_nodes(tab_index, drawing_code, &scratch, nodes);
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
        let kind = classify_ut_kind(&sym.sppid_class, "", &sym.source_block.to_ascii_uppercase());

        nodes.push(UtNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
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
        nodes.push(UtNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind: UtKind::PipeSegment,
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

fn merge_opc_into_ut_sheet(
    c: &OpcConnectorItem,
    drawing_code: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<UtNode>,
) -> usize {
    for idx in sheet_range {
        if nodes[idx].kind == UtKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = nodes[idx].kind == UtKind::Opc
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = UtKind::Opc;
            for &mh in &c.member_handles {
                if !nodes[idx].member_handles.contains(&mh) {
                    nodes[idx].member_handles.push(mh);
                }
            }
            if nodes[idx].tag_or_label.is_none() {
                nodes[idx].tag_or_label = c.line_label.clone().or_else(|| c.target_dwg_raw.clone());
            }
            return idx;
        }
    }

    let idx = nodes.len();
    nodes.push(UtNode {
        tab_index: c.tab_index,
        drawing_code: drawing_code.to_string(),
        kind: UtKind::Opc,
        tag_or_label: c.line_label.clone().or_else(|| c.target_dwg_raw.clone()),
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

fn wire_ut_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[UtNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;
    let opc_tol = 14.0 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == UtKind::PipeSegment {
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
                                    nodes[sn].kind != UtKind::Opc
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
                UtKind::Equipment => 12.0 * upm,
                UtKind::Opc => opc_tol,
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
        if nodes[sn].kind == UtKind::Opc && adj[sn].is_empty() {
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
}

fn propagate_unambiguous_pipe_labels(graph: &mut UtGraph) {
    let mut updates: Vec<(usize, String)> = Vec::new();
    for idx in 0..graph.nodes.len() {
        if graph.nodes[idx].kind != UtKind::PipeSegment || graph.nodes[idx].tag_or_label.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeSet<String> = BTreeSet::new();
        while let Some(cur) = q.pop_front() {
            if graph.nodes[cur].kind == UtKind::PipeSegment {
                let valid_lbl = graph.nodes[cur]
                    .tag_or_label
                    .as_deref()
                    .filter(|s| parse_sppid_line_number(s).is_some());
                if let Some(lbl) = valid_lbl {
                    found.insert(lbl.to_string());
                    continue;
                }
            }
            if matches!(graph.nodes[cur].kind, UtKind::Equipment | UtKind::Opc) {
                continue;
            }
            for &nxt in &graph.adj[cur] {
                if visited.len() < 14 && visited.insert(nxt) {
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

fn build_ut_graph(sheets: &[OpcSheetInput<'_>], opc_connectors: &[OpcConnectorItem]) -> UtGraph {
    let mut tab_drawings: BTreeMap<usize, String> = BTreeMap::new();
    for c in opc_connectors {
        tab_drawings
            .entry(c.tab_index)
            .or_insert_with(|| c.drawing_code.clone());
    }
    let mut nodes: Vec<UtNode> = Vec::new();
    let mut sheet_ranges: Vec<(usize, f64, std::ops::Range<usize>)> = Vec::new();
    let mut opc_node_by_key: BTreeMap<(usize, Handle), usize> = BTreeMap::new();

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
            extract_authored_ut_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        } else {
            extract_unpromoted_ut_nodes(sh.tab_index, &dwg_code, sh.document, &mut nodes);
        }
        let base_end = nodes.len();
        for c in opc_connectors
            .iter()
            .filter(|c| c.tab_index == sh.tab_index)
        {
            let opc_idx =
                merge_opc_into_ut_sheet(c, &dwg_code, upm, start_idx..base_end, &mut nodes);
            opc_node_by_key.insert((c.tab_index, c.primary_handle), opc_idx);
        }
        let final_end = nodes.len();
        sheet_ranges.push((sh.tab_index, upm, start_idx..final_end));
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (_, upm, range) in &sheet_ranges {
        let indices: Vec<usize> = range.clone().collect();
        wire_ut_local_edges(&indices, *upm, &nodes, &mut adj);
    }

    let mut graph = UtGraph {
        nodes,
        adj,
        opc_node_by_key,
    };
    propagate_unambiguous_pipe_labels(&mut graph);
    graph
}

fn push_unique_handles(dst: &mut Vec<Handle>, src: &[Handle]) {
    for &h in src {
        if !dst.contains(&h) {
            dst.push(h);
        }
    }
}

/// Analyze open `sheets` to classify utility vs process fluid services and extract external
/// battery-limit `OPC` interfaces with back-traced line specs and boundary valves (`#161`).
pub fn analyze_pid_utilities_and_interfaces(sheets: &[OpcSheetInput<'_>]) -> PidUtilityReport {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let graph = build_ut_graph(sheets, &opc_rep.connectors);

    struct ServiceAccum {
        category: FluidCategory,
        lines: BTreeSet<String>,
        segment_count: usize,
        sizes: BTreeSet<String>,
        specs: BTreeSet<String>,
        drawings: BTreeSet<String>,
        handles_by_tab: BTreeMap<usize, Vec<Handle>>,
    }

    let mut svc_map: BTreeMap<String, ServiceAccum> = BTreeMap::new();
    let mut all_labeled_lines: BTreeSet<String> = BTreeSet::new();
    let mut utility_labeled_lines: BTreeSet<String> = BTreeSet::new();

    for nd in &graph.nodes {
        if nd.kind != UtKind::PipeSegment {
            continue;
        }
        let Some(ref raw_lbl) = nd.tag_or_label else {
            continue;
        };
        let Some(parsed) = parse_sppid_line_number(raw_lbl) else {
            continue;
        };
        let fcode = parsed.fluid_code.trim().to_ascii_uppercase();
        if fcode.is_empty() {
            continue;
        }
        let cat = classify_fluid_service(&fcode);
        let pname = parsed.pipeline_name();
        all_labeled_lines.insert(pname.clone());
        if cat.is_utility() {
            utility_labeled_lines.insert(pname.clone());
        }

        let entry = svc_map.entry(fcode).or_insert_with(|| ServiceAccum {
            category: cat,
            lines: BTreeSet::new(),
            segment_count: 0,
            sizes: BTreeSet::new(),
            specs: BTreeSet::new(),
            drawings: BTreeSet::new(),
            handles_by_tab: BTreeMap::new(),
        });
        entry.lines.insert(pname);
        entry.segment_count += 1;
        if !parsed.nominal_diameter_attr.is_empty() {
            entry.sizes.insert(parsed.nominal_diameter_attr);
        }
        if !parsed.materials_class.is_empty() {
            entry.specs.insert(parsed.materials_class);
        }
        entry.drawings.insert(nd.drawing_code.clone());
        push_unique_handles(
            entry.handles_by_tab.entry(nd.tab_index).or_default(),
            &nd.member_handles,
        );
    }

    let mut service_groups: Vec<PidServiceSummaryRow> = svc_map
        .into_iter()
        .map(|(fluid_code, acc)| PidServiceSummaryRow {
            fluid_code,
            category: acc.category,
            is_utility: acc.category.is_utility(),
            pipeline_count: acc.lines.len(),
            segment_count: acc.segment_count,
            nominal_sizes: acc.sizes.into_iter().collect(),
            piping_specs: acc.specs.into_iter().collect(),
            drawing_codes: acc.drawings.into_iter().collect(),
            sample_lines: acc.lines.into_iter().take(6).collect(),
            handles_by_tab: acc.handles_by_tab,
        })
        .collect();
    service_groups.sort_by(|a, b| {
        b.is_utility
            .cmp(&a.is_utility)
            .then_with(|| a.fluid_code.cmp(&b.fluid_code))
    });

    // Extract battery-limit interfaces: OPCs pointing outside the open drawing set
    let mut battery_limits: Vec<PidBatteryLimitInterface> = Vec::new();
    for c in &opc_rep.connectors {
        if c.status == OpcStatus::Paired || c.target_open_tab.is_some() {
            continue;
        }
        if !matches!(
            c.status,
            OpcStatus::ExternalUnopened | OpcStatus::MissingTarget
        ) {
            continue;
        }

        let mut trace_handles = c.member_handles.clone();
        let mut traced_line: Option<String> = c
            .line_label
            .as_deref()
            .filter(|l| parse_sppid_line_number(l).is_some())
            .map(|l| l.trim().trim_end_matches('-').to_string());
        let mut nearest_vm: Option<String> = None;

        if let Some(&start_idx) = graph.opc_node_by_key.get(&(c.tab_index, c.primary_handle)) {
            let mut visited: BTreeSet<usize> = BTreeSet::new();
            let mut q: VecDeque<(usize, usize)> = VecDeque::new();
            visited.insert(start_idx);
            q.push_back((start_idx, 0));

            while let Some((cur, depth)) = q.pop_front() {
                let cnd = &graph.nodes[cur];
                if depth > 0 {
                    push_unique_handles(&mut trace_handles, &cnd.member_handles);
                }
                if traced_line.is_none() && cnd.kind == UtKind::PipeSegment {
                    traced_line = cnd
                        .tag_or_label
                        .as_deref()
                        .filter(|s| parse_sppid_line_number(s).is_some())
                        .map(str::to_string);
                }
                if nearest_vm.is_none() && cnd.kind == UtKind::ValveOrMeter {
                    nearest_vm = cnd.tag_or_label.clone();
                }
                if depth >= 6 || (depth > 0 && matches!(cnd.kind, UtKind::Equipment | UtKind::Opc))
                {
                    continue;
                }
                for &nxt in &graph.adj[cur] {
                    if visited.insert(nxt) {
                        q.push_back((nxt, depth + 1));
                    }
                }
            }
        }

        let line_number = traced_line.unwrap_or_default();
        let parsed_opt = parse_sppid_line_number(&line_number);
        let fluid_code = parsed_opt
            .as_ref()
            .map(|p| p.fluid_code.to_ascii_uppercase())
            .unwrap_or_default();
        let category = if fluid_code.is_empty() {
            FluidCategory::ProcessFluid
        } else {
            classify_fluid_service(&fluid_code)
        };
        let nominal_diameter = parsed_opt
            .as_ref()
            .map(|p| p.nominal_diameter_attr.clone())
            .unwrap_or_default();
        let piping_spec = parsed_opt
            .as_ref()
            .map(|p| p.materials_class.clone())
            .unwrap_or_default();
        let external_target = c
            .target_dwg_raw
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "BATTERY_LIMIT".to_string());

        let interface_id = format!("BL-{:02}", battery_limits.len() + 1);
        battery_limits.push(PidBatteryLimitInterface {
            interface_id,
            tab_index: c.tab_index,
            drawing_code: c.drawing_code.clone(),
            external_target,
            line_number,
            fluid_code,
            category,
            is_utility: category.is_utility(),
            nominal_diameter,
            piping_spec,
            nearest_valve_or_meter: nearest_vm,
            primary_handle_hex: c.primary_handle_hex.clone(),
            x: c.x,
            y: c.y,
            primary_handle: c.primary_handle,
            member_handles: trace_handles,
        });
    }

    let utility_services_count = service_groups.iter().filter(|s| s.is_utility).count();
    let process_services_count = service_groups.len().saturating_sub(utility_services_count);
    let utility_battery_limits = battery_limits.iter().filter(|b| b.is_utility).count();

    PidUtilityReport {
        summary: PidUtilitySummary {
            sheets_scanned: sheets.len(),
            total_services: service_groups.len(),
            utility_services_count,
            process_services_count,
            labeled_pipelines_count: all_labeled_lines.len(),
            utility_pipelines_count: utility_labeled_lines.len(),
            battery_limit_count: battery_limits.len(),
            utility_battery_limits,
        },
        service_groups,
        battery_limits,
    }
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and extract a unified Utility & Battery-Limit report (`#161`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_utilities_from_path(input_path: &Path) -> Result<PidUtilityReport, String> {
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
    Ok(analyze_pid_utilities_and_interfaces(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidUtilityReport` as delimited Utility & Battery-Limit Schedule text with UTF-8 BOM.
pub fn format_pid_utility_delimited(report: &PidUtilityReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = UTILITY_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for sg in &report.service_groups {
        let cnt_str = format!(
            "pipelines={};segments={}",
            sg.pipeline_count, sg.segment_count
        );
        let sizes = sg.nominal_sizes.join(";");
        let specs = sg.piping_specs.join(";");
        let dwgs = sg.drawing_codes.join(";");
        let sample = sg.sample_lines.join(";");
        let cells = [
            "SERVICE_GROUP",
            sg.fluid_code.as_str(),
            sg.category.as_str(),
            dwgs.as_str(),
            cnt_str.as_str(),
            sizes.as_str(),
            specs.as_str(),
            "",
            sample.as_str(),
            "",
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }

    for bl in &report.battery_limits {
        let xy = format!("{:.1},{:.1}", bl.x, bl.y);
        let cells = [
            "BATTERY_LIMIT",
            bl.interface_id.as_str(),
            bl.category.as_str(),
            bl.drawing_code.as_str(),
            bl.line_number.as_str(),
            bl.nominal_diameter.as_str(),
            bl.piping_spec.as_str(),
            bl.external_target.as_str(),
            bl.nearest_valve_or_meter.as_deref().unwrap_or(""),
            xy.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidUtilityReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#161`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_utility_file(report: &PidUtilityReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize utility JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_utility_delimited(report, '\t'),
        "csv" => format_pid_utility_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported utility report extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

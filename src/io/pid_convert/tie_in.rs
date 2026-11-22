//! P&ID Revamp Tie-In & Battery-Limit Hookup Schedule (`SH/T 3005` / `PIP PIC001`),
//! positive-isolation (`DBB / Spectacle Blind / Single Valve / Unisolated Hazard`) grading,
//! and interactive `PIDTIEIN` / CLI `--export-tieins` support (`L3-3 / L3-4 / L3-6 / L6-2` / `#175`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::{parse_sppid_line_number, ParsedLineNumber};
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::{analyze_open_sheets_opc, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::specialty::{classify_specialty_component, SpecialtySubtype};
use super::valve_mto::{classify_valve_role_and_subtype, ValveCategory, ValveSubtype};

/// Fixed 13-column CSV/TSV header for `PIDTIEIN EXPORT` and `--export-tieins`.
pub const TIE_IN_CSV_HEADERS: [&str; 13] = [
    "TieInId",
    "SourceKind",
    "CalloutOrTarget",
    "DrawingCode",
    "LineNumber",
    "NominalDiameter",
    "FluidCode",
    "PipingSpec",
    "Insulation",
    "IsolationReadiness",
    "IsolationValves",
    "BlindOrSpadeItems",
    "AnchorEquipmentOrHeader",
];

/// Origin category of a P&ID tie-in or boundary interface point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TieInSourceKind {
    ExplicitCallout,
    BatteryLimitOpc,
}

impl TieInSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitCallout => "explicit_callout",
            Self::BatteryLimitOpc => "battery_limit_opc",
        }
    }
}

/// Construction & turnaround positive-isolation readiness grade (`PIP PIC001`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TieInIsolationReadiness {
    DbbOrBlindReady,
    SingleValveShutdownOnly,
    UnisolatedDirectHazard,
}

impl TieInIsolationReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DbbOrBlindReady => "dbb_or_blind_ready",
            Self::SingleValveShutdownOnly => "single_valve_shutdown_only",
            Self::UnisolatedDirectHazard => "unisolated_direct_hazard",
        }
    }
}

/// One analyzed revamp tie-in or external battery-limit hookup point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidTieInRow {
    pub tie_in_id: String,
    pub source_kind: TieInSourceKind,
    pub callout_or_target: String,
    pub tab_index: usize,
    pub drawing_code: String,
    pub line_number: String,
    pub nominal_diameter: String,
    pub fluid_code: String,
    pub piping_spec: String,
    pub insulation: String,
    pub isolation_readiness: TieInIsolationReadiness,
    pub isolation_valves: Vec<String>,
    pub blind_or_spade_items: Vec<String>,
    pub anchor_equipment_or_header: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Summary statistics across all extracted tie-in points.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidTieInSummary {
    pub sheets_scanned: usize,
    pub total_tie_ins: usize,
    pub explicit_callouts_count: usize,
    pub battery_limit_opc_count: usize,
    pub dbb_or_blind_ready_count: usize,
    pub single_valve_count: usize,
    pub unisolated_hazard_count: usize,
}

/// Complete `PIDTIEIN` Tie-In & Battery-Limit Hookup Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidTieInReport {
    pub summary: PidTieInSummary,
    pub tie_ins: Vec<PidTieInRow>,
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
enum TiNodeKind {
    Equipment,
    IsolationValve,
    SpectacleOrBlind,
    OtherFitting,
    PipeSegment,
}

struct TiNode {
    kind: TiNodeKind,
    tag_or_label: Option<String>,
    parsed_line: Option<ParsedLineNumber>,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
}

struct ExplicitCalloutHit {
    handle: Handle,
    at: Pt2,
    label: String,
}

struct SheetTiGraph {
    drawing_code: String,
    upm: f64,
    nodes: Vec<TiNode>,
    adj: Vec<Vec<usize>>,
    callouts: Vec<ExplicitCalloutHit>,
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

/// Check whether a drawing annotation string represents an explicit revamp Tie-In callout.
pub fn is_explicit_tie_in_text(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 28 || parse_sppid_line_number(trimmed).is_some() {
        return false;
    }
    let up = trimmed.to_ascii_uppercase();
    if up.starts_with("TP-")
        || up.starts_with("TP_")
        || up.starts_with("TI-P-")
        || up.contains("TIE-IN")
        || up.contains("TIE IN")
        || trimmed.contains("碰头点")
        || trimmed.contains("碰头接点")
    {
        return true;
    }
    if let Some(rest) = up.strip_prefix("TP") {
        return !rest.is_empty()
            && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
            && rest.chars().all(|c| c.is_ascii_alphanumeric());
    }
    false
}

fn classify_ti_node_kind(
    cls: &str,
    type_code: &str,
    desc_up: &str,
    tag_opt: Option<&str>,
) -> TiNodeKind {
    if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || matches!(
        type_code.trim().to_ascii_uppercase().as_str(),
        "VESSEL" | "PUMP" | "TANK" | "EQUIPMENT" | "HEAT_EXCHANGER"
    ) {
        return TiNodeKind::Equipment;
    }

    if classify_specialty_component(cls, type_code, desc_up, tag_opt).is_some_and(|(_, sub)| {
        matches!(
            sub,
            SpecialtySubtype::BlindOrificeSpacer | SpecialtySubtype::FlangeCap
        )
    }) {
        return TiNodeKind::SpectacleOrBlind;
    }

    if classify_valve_role_and_subtype(cls, type_code, desc_up, tag_opt).is_some_and(
        |(cat, sub)| {
            (cat == ValveCategory::ManualBulk && sub != ValveSubtype::CheckValve)
                || cat == ValveCategory::ActuatedControl
        },
    ) {
        return TiNodeKind::IsolationValve;
    }

    TiNodeKind::OtherFitting
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

fn build_sheet_ti_graph(fallback_code: &str, doc: &Document) -> SheetTiGraph {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let working_doc;
    let eff_doc = if !doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some())
        && has_native_pid_semantics(doc)
    {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        working_doc = scratch;
        &working_doc
    } else {
        doc
    };

    let has_author = eff_doc
        .model_space_entities()
        .any(|e| PidAuthor::load(eff_doc, e.common().handle).is_some());
    let rec_opt = (!has_native_pid_semantics(eff_doc)).then(|| {
        recognise_sppid_sheet(
            eff_doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        )
    });
    let drawing_code = rec_opt
        .as_ref()
        .and_then(|r| r.title_block.drawing_number.as_deref())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback_code.to_string());

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

    let mut nodes: Vec<TiNode> = Vec::new();
    let mut pipe_indices_by_h: BTreeMap<Handle, Vec<usize>> = BTreeMap::new();
    let mut promoted_sym_handles: BTreeSet<Handle> = BTreeSet::new();

    if has_author {
        for ent in eff_doc.model_space_entities() {
            let h = ent.common().handle;
            let Some(auth) = PidAuthor::load(eff_doc, h) else {
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
                    nodes.push(TiNode {
                        kind: TiNodeKind::PipeSegment,
                        tag_or_label: resolved,
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
            let fallback_cb = entity_center_and_bbox(eff_doc, h, upm);
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
            let tag_opt = (!auth.tag.trim().is_empty()).then(|| auth.tag.trim().to_string());
            if tag_opt.as_deref().is_some_and(is_explicit_tie_in_text) {
                continue;
            }
            let kind = classify_ti_node_kind(cls, &auth.type_code, &desc_up, tag_opt.as_deref());
            let mut ports = Vec::new();
            if let Some(ps) = pmap.get("_ports").and_then(Value::as_str) {
                for item in ps.split(';').filter(|s| !s.trim().is_empty()) {
                    if let Some(pt) = parse_pair_f64(item) {
                        ports.push(pt);
                    }
                }
            }
            if let Some(s) = rec_sym.filter(|_| ports.is_empty()) {
                ports.extend_from_slice(&s.ports);
            }

            nodes.push(TiNode {
                kind,
                tag_or_label: tag_opt,
                parsed_line: None,
                primary_handle: h,
                member_handles: members,
                center,
                bbox,
                ports,
                spans: Vec::new(),
            });
        }

        let topo = TopologyTable::load(eff_doc);
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
                        nodes[idx].parsed_line =
                            resolved.as_deref().and_then(parse_sppid_line_number);
                        nodes[idx].tag_or_label = resolved;
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
                nodes.push(TiNode {
                    kind: TiNodeKind::PipeSegment,
                    tag_or_label: resolved,
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
            let tag_opt = sym
                .tag
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if tag_opt.as_deref().is_some_and(is_explicit_tie_in_text) {
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
                entity_center_and_bbox(eff_doc, primary_h, upm)
                    .map(|(_, b)| b)
                    .unwrap_or(BBox2 {
                        min_x: sym.at.0 - 5.0 * upm,
                        min_y: sym.at.1 - 5.0 * upm,
                        max_x: sym.at.0 + 5.0 * upm,
                        max_y: sym.at.1 + 5.0 * upm,
                    })
            };
            let desc_up = format!(
                "{} {} {}",
                sym.source_block,
                sym.layer,
                sym.symbol_path.as_deref().unwrap_or("")
            )
            .to_ascii_uppercase();
            let kind = classify_ti_node_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref());
            nodes.push(TiNode {
                kind,
                tag_or_label: tag_opt,
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
            nodes.push(TiNode {
                kind: TiNodeKind::PipeSegment,
                tag_or_label: lbl_opt,
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

    // Collect explicit Tie-In text callouts
    let mut callouts: Vec<ExplicitCalloutHit> = Vec::new();
    let mut seen_labels: BTreeSet<String> = BTreeSet::new();
    for ent in eff_doc.model_space_entities() {
        let h = ent.common().handle;
        match ent {
            EntityType::Text(t) => {
                let v = t.value.trim();
                if is_explicit_tie_in_text(v) && seen_labels.insert(v.to_ascii_uppercase()) {
                    callouts.push(ExplicitCalloutHit {
                        handle: h,
                        at: (t.insertion_point.x, t.insertion_point.y),
                        label: v.to_string(),
                    });
                }
            }
            EntityType::MText(mt) => {
                let cleaned = mt.value.replace("\\P", " ");
                let v = cleaned.trim();
                if is_explicit_tie_in_text(v) && seen_labels.insert(v.to_ascii_uppercase()) {
                    callouts.push(ExplicitCalloutHit {
                        handle: h,
                        at: (mt.insertion_point.x, mt.insertion_point.y),
                        label: v.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    // Wire local edges
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for (idx, nd) in nodes.iter().enumerate() {
        if nd.kind == TiNodeKind::PipeSegment {
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
                        let blocked = pairs
                            .iter()
                            .filter(|&&(p1, p2)| (p1.0 - p2.0).hypot(p1.1 - p2.1) <= pipe_tol * 1.5)
                            .any(|&(p1, p2)| {
                                let mid = ((p1.0 + p2.0) * 0.5, (p1.1 + p2.1) * 0.5);
                                sym_nodes.iter().any(|&sn| {
                                    nodes[sn].kind != TiNodeKind::Equipment
                                        && point_in_bbox(mid, nodes[sn].bbox, 1.5 * upm)
                                })
                            });
                        if !blocked {
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
                add_undirected_edge(&mut adj, na, nb);
            }
        }
    }

    for &pn in &pipe_nodes {
        for &sn in &sym_nodes {
            let margin = match nodes[sn].kind {
                TiNodeKind::Equipment => 12.0 * upm,
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
                add_undirected_edge(&mut adj, pn, sn);
            }
        }
    }

    // Propagate unambiguous pipe labels across connected segments
    let mut updates: Vec<(usize, String, ParsedLineNumber)> = Vec::new();
    for (idx, nd) in nodes.iter().enumerate() {
        if nd.kind != TiNodeKind::PipeSegment || nd.parsed_line.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeMap<String, (String, ParsedLineNumber)> = BTreeMap::new();
        while let Some(cur) = q.pop_front() {
            let cnd = &nodes[cur];
            if let (TiNodeKind::PipeSegment, Some(raw), Some(parsed)) =
                (cnd.kind, &cnd.tag_or_label, &cnd.parsed_line)
            {
                found.insert(parsed.pipeline_name(), (raw.clone(), parsed.clone()));
                continue;
            }
            if cnd.kind == TiNodeKind::Equipment {
                continue;
            }
            for &nxt in &adj[cur] {
                if visited.len() < 20 && visited.insert(nxt) {
                    q.push_back(nxt);
                }
            }
        }
        if let Some((_, (raw, parsed))) = (found.len() == 1)
            .then(|| found.into_iter().next())
            .flatten()
        {
            updates.push((idx, raw, parsed));
        }
    }
    for (idx, raw, parsed) in updates {
        nodes[idx].tag_or_label = Some(raw);
        nodes[idx].parsed_line = Some(parsed);
    }

    SheetTiGraph {
        drawing_code,
        upm,
        nodes,
        adj,
        callouts,
    }
}

struct InwardTrace {
    line_number: String,
    nominal_diameter: String,
    fluid_code: String,
    piping_spec: String,
    insulation: String,
    isolation_valves: Vec<String>,
    blind_or_spade_items: Vec<String>,
    anchor_equipment_or_header: String,
    extra_handles: Vec<Handle>,
}

fn trace_tie_in_inward(sg: &SheetTiGraph, pt: Pt2, seed_handles: &[Handle]) -> InwardTrace {
    let mut start_pipe: Option<(f64, usize)> = None;
    for (idx, nd) in sg.nodes.iter().enumerate() {
        if nd.kind != TiNodeKind::PipeSegment {
            continue;
        }
        for &sp in &nd.spans {
            let d = point_to_span_dist(pt, sp);
            if d <= 20.0 * sg.upm && start_pipe.is_none_or(|(bd, _)| d < bd) {
                start_pipe = Some((d, idx));
            }
        }
    }

    let mut visited: BTreeSet<usize> = BTreeSet::new();
    let mut q: VecDeque<(usize, bool)> = VecDeque::new();
    if let Some((_, pidx)) = start_pipe {
        visited.insert(pidx);
        q.push_back((pidx, true));
    } else {
        for (idx, nd) in sg.nodes.iter().enumerate() {
            if nd.member_handles.iter().any(|mh| seed_handles.contains(mh))
                || (nd.center.0 - pt.0).hypot(nd.center.1 - pt.1) <= 14.0 * sg.upm
            {
                visited.insert(idx);
                q.push_back((idx, true));
            }
        }
    }

    let mut first_parsed: Option<ParsedLineNumber> = None;
    let mut isolation_valves: Vec<String> = Vec::new();
    let mut seen_valves: BTreeSet<String> = BTreeSet::new();
    let mut blind_items: Vec<String> = Vec::new();
    let mut seen_blinds: BTreeSet<String> = BTreeSet::new();
    let mut anchor_target = String::new();
    let mut extra_handles: Vec<Handle> = Vec::new();

    while let Some((cur, in_series)) = q.pop_front() {
        let cnd = &sg.nodes[cur];
        for &mh in &cnd.member_handles {
            if !extra_handles.contains(&mh) && extra_handles.len() < 16 {
                extra_handles.push(mh);
            }
        }

        match cnd.kind {
            TiNodeKind::Equipment => {
                if anchor_target.is_empty() {
                    anchor_target = cnd
                        .tag_or_label
                        .clone()
                        .unwrap_or_else(|| format!("EQUIP-{:X}", cnd.primary_handle.value()));
                }
                continue;
            }
            TiNodeKind::IsolationValve => {
                if in_series {
                    let vtag = cnd
                        .tag_or_label
                        .clone()
                        .unwrap_or_else(|| format!("VALVE-{:X}", cnd.primary_handle.value()));
                    if seen_valves.insert(vtag.clone()) {
                        isolation_valves.push(vtag);
                    }
                }
            }
            TiNodeKind::SpectacleOrBlind => {
                if in_series {
                    let btag = cnd
                        .tag_or_label
                        .clone()
                        .unwrap_or_else(|| format!("BLIND-{:X}", cnd.primary_handle.value()));
                    if seen_blinds.insert(btag.clone()) {
                        blind_items.push(btag);
                    }
                }
            }
            TiNodeKind::PipeSegment => {
                if let Some(p) = cnd.parsed_line.as_ref().filter(|_| first_parsed.is_none()) {
                    first_parsed = Some(p.clone());
                } else if let Some((_, cp)) = first_parsed
                    .as_ref()
                    .zip(cnd.parsed_line.as_ref())
                    .filter(|(fp, cp)| {
                        anchor_target.is_empty() && fp.pipeline_name() != cp.pipeline_name()
                    })
                {
                    anchor_target = format!("HDR:{}", cp.pipeline_name());
                }
            }
            TiNodeKind::OtherFitting => {}
        }

        let pipe_deg = sg.adj[cur]
            .iter()
            .filter(|&&nb| sg.nodes[nb].kind == TiNodeKind::PipeSegment)
            .count();
        let next_in_series = in_series && pipe_deg <= 2;

        for &nxt in &sg.adj[cur] {
            if visited.len() < 32 && visited.insert(nxt) {
                q.push_back((nxt, next_in_series));
            }
        }
    }

    let (line_number, nominal_diameter, fluid_code, piping_spec, insulation) = match first_parsed {
        Some(p) => (
            p.pipeline_name(),
            p.nominal_diameter_attr,
            p.fluid_code.to_ascii_uppercase(),
            p.materials_class,
            p.insulation.unwrap_or_default(),
        ),
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
    };

    InwardTrace {
        line_number,
        nominal_diameter,
        fluid_code,
        piping_spec,
        insulation,
        isolation_valves,
        blind_or_spade_items: blind_items,
        anchor_equipment_or_header: anchor_target,
        extra_handles,
    }
}

/// Check whether a `PidTieInRow` matches a user tie-in callout or pipeline query.
pub fn tie_in_row_matches_query(row: &PidTieInRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    let q_comp: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let matches_s = |s: &str| {
        let su = s.to_ascii_uppercase();
        let sc: String = su.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        su.contains(&q) || (!q_comp.is_empty() && sc.contains(&q_comp))
    };
    matches_s(&row.tie_in_id)
        || matches_s(&row.callout_or_target)
        || matches_s(&row.line_number)
        || matches_s(&row.anchor_equipment_or_header)
}

/// Extract all explicit revamp Tie-In callouts and external Battery-Limit `OPC` hookups
/// across `sheets`, grading positive-isolation readiness (`#175`).
pub fn analyze_pid_tie_ins(sheets: &[OpcSheetInput<'_>]) -> PidTieInReport {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut graphs: BTreeMap<usize, SheetTiGraph> = BTreeMap::new();
    for sh in sheets {
        graphs.insert(
            sh.tab_index,
            build_sheet_ti_graph(&clean_drawing_stem(sh.fallback_name), sh.document),
        );
    }

    let mut tie_ins: Vec<PidTieInRow> = Vec::new();
    let mut explicit_pts_by_tab: BTreeMap<usize, Vec<Pt2>> = BTreeMap::new();

    // 1. Explicit Tie-In callouts (TP-*, TIE-IN, 碰头点)
    for (&tab_idx, sg) in &graphs {
        for co in &sg.callouts {
            explicit_pts_by_tab.entry(tab_idx).or_default().push(co.at);
            let tr = trace_tie_in_inward(sg, co.at, &[co.handle]);
            let readiness = if (!tr.isolation_valves.is_empty()
                && !tr.blind_or_spade_items.is_empty())
                || tr.isolation_valves.len() >= 2
            {
                TieInIsolationReadiness::DbbOrBlindReady
            } else if tr.isolation_valves.len() == 1 || !tr.blind_or_spade_items.is_empty() {
                TieInIsolationReadiness::SingleValveShutdownOnly
            } else {
                TieInIsolationReadiness::UnisolatedDirectHazard
            };

            let mut mems = vec![co.handle];
            for h in tr.extra_handles {
                if !mems.contains(&h) {
                    mems.push(h);
                }
            }

            tie_ins.push(PidTieInRow {
                tie_in_id: String::new(),
                source_kind: TieInSourceKind::ExplicitCallout,
                callout_or_target: co.label.clone(),
                tab_index: tab_idx,
                drawing_code: sg.drawing_code.clone(),
                line_number: tr.line_number,
                nominal_diameter: tr.nominal_diameter,
                fluid_code: tr.fluid_code,
                piping_spec: tr.piping_spec,
                insulation: tr.insulation,
                isolation_readiness: readiness,
                isolation_valves: tr.isolation_valves,
                blind_or_spade_items: tr.blind_or_spade_items,
                anchor_equipment_or_header: tr.anchor_equipment_or_header,
                primary_handle_hex: format!("{:X}", co.handle.value()),
                x: co.at.0,
                y: co.at.1,
                primary_handle: co.handle,
                member_handles: mems,
            });
        }
    }

    // 2. External unpaired Battery-Limit OPCs
    for c in &opc_rep.connectors {
        if c.status == OpcStatus::Paired {
            continue;
        }
        let Some(sg) = graphs.get(&c.tab_index) else {
            continue;
        };
        let near_explicit = explicit_pts_by_tab.get(&c.tab_index).is_some_and(|pts| {
            pts.iter()
                .any(|p| (p.0 - c.x).hypot(p.1 - c.y) <= 18.0 * sg.upm)
        });
        if near_explicit {
            continue;
        }

        let tr = trace_tie_in_inward(sg, (c.x, c.y), &c.member_handles);
        let readiness = if (!tr.isolation_valves.is_empty() && !tr.blind_or_spade_items.is_empty())
            || tr.isolation_valves.len() >= 2
        {
            TieInIsolationReadiness::DbbOrBlindReady
        } else if tr.isolation_valves.len() == 1 || !tr.blind_or_spade_items.is_empty() {
            TieInIsolationReadiness::SingleValveShutdownOnly
        } else {
            TieInIsolationReadiness::UnisolatedDirectHazard
        };

        let mut mems = c.member_handles.clone();
        if !mems.contains(&c.primary_handle) {
            mems.insert(0, c.primary_handle);
        }
        for h in tr.extra_handles {
            if !mems.contains(&h) {
                mems.push(h);
            }
        }

        let target_lbl = c
            .target_dwg_raw
            .clone()
            .or_else(|| c.line_label.clone())
            .unwrap_or_else(|| "OSBL".to_string());

        tie_ins.push(PidTieInRow {
            tie_in_id: String::new(),
            source_kind: TieInSourceKind::BatteryLimitOpc,
            callout_or_target: target_lbl,
            tab_index: c.tab_index,
            drawing_code: sg.drawing_code.clone(),
            line_number: tr.line_number,
            nominal_diameter: tr.nominal_diameter,
            fluid_code: tr.fluid_code,
            piping_spec: tr.piping_spec,
            insulation: tr.insulation,
            isolation_readiness: readiness,
            isolation_valves: tr.isolation_valves,
            blind_or_spade_items: tr.blind_or_spade_items,
            anchor_equipment_or_header: tr.anchor_equipment_or_header,
            primary_handle_hex: format!("{:X}", c.primary_handle.value()),
            x: c.x,
            y: c.y,
            primary_handle: c.primary_handle,
            member_handles: mems,
        });
    }

    tie_ins.sort_by(|a, b| {
        a.tab_index
            .cmp(&b.tab_index)
            .then_with(|| a.source_kind.cmp(&b.source_kind))
            .then_with(|| a.callout_or_target.cmp(&b.callout_or_target))
            .then_with(|| a.primary_handle_hex.cmp(&b.primary_handle_hex))
    });

    for (i, row) in tie_ins.iter_mut().enumerate() {
        row.tie_in_id = format!("TI-{:03}", i + 1);
    }

    let explicit_callouts_count = tie_ins
        .iter()
        .filter(|r| r.source_kind == TieInSourceKind::ExplicitCallout)
        .count();
    let battery_limit_opc_count = tie_ins
        .iter()
        .filter(|r| r.source_kind == TieInSourceKind::BatteryLimitOpc)
        .count();
    let dbb_or_blind_ready_count = tie_ins
        .iter()
        .filter(|r| r.isolation_readiness == TieInIsolationReadiness::DbbOrBlindReady)
        .count();
    let single_valve_count = tie_ins
        .iter()
        .filter(|r| r.isolation_readiness == TieInIsolationReadiness::SingleValveShutdownOnly)
        .count();
    let unisolated_hazard_count = tie_ins
        .iter()
        .filter(|r| r.isolation_readiness == TieInIsolationReadiness::UnisolatedDirectHazard)
        .count();

    PidTieInReport {
        summary: PidTieInSummary {
            sheets_scanned: sheets.len(),
            total_tie_ins: tie_ins.len(),
            explicit_callouts_count,
            battery_limit_opc_count,
            dbb_or_blind_ready_count,
            single_valve_count,
            unisolated_hazard_count,
        },
        tie_ins,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidTieInReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_tie_in_delimited(report: &PidTieInReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = TIE_IN_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for r in &report.tie_ins {
        let cols = [
            r.tie_in_id.clone(),
            r.source_kind.as_str().to_string(),
            r.callout_or_target.clone(),
            r.drawing_code.clone(),
            r.line_number.clone(),
            r.nominal_diameter.clone(),
            r.fluid_code.clone(),
            r.piping_spec.clone(),
            r.insulation.clone(),
            r.isolation_readiness.as_str().to_string(),
            r.isolation_valves.join(";"),
            r.blind_or_spade_items.join(";"),
            r.anchor_equipment_or_header.clone(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidTieInReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_tie_in_file(
    report: &PidTieInReport,
    out_path: &std::path::Path,
) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let ext = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("json")
        .to_ascii_lowercase();

    match ext.as_str() {
        "csv" => {
            let body = format_pid_tie_in_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_tie_in_delimited(report, '\t');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        _ => {
            let txt = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
            std::fs::write(out_path, format!("{txt}\n")).map_err(|e| e.to_string())
        }
    }
}

/// Headless CLI helper: load a single `.dxf`/`.dwg`/`.pid` file or directory of sheets
/// and build a `PidTieInReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_tie_ins_from_path(input: &std::path::Path) -> Result<PidTieInReport, String> {
    let mut loaded: Vec<(String, Document)> = Vec::new();
    if input.is_file() {
        let nm = input
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sheet.dxf")
            .to_string();
        let doc = crate::io::load_file(input).map_err(|e| format!("{}: {e}", input.display()))?;
        loaded.push((nm, doc));
    } else if input.is_dir() {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(input)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|de| de.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension().and_then(|s| s.to_str()).is_some_and(|ext| {
                        matches!(ext.to_ascii_lowercase().as_str(), "dxf" | "dwg" | "pid")
                    })
            })
            .collect();
        entries.sort();
        for p in entries {
            let nm = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("sheet.dxf")
                .to_string();
            let doc = crate::io::load_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            loaded.push((nm, doc));
        }
    } else {
        return Err(format!("input path not found: {}", input.display()));
    }

    let refs: Vec<OpcSheetInput<'_>> = loaded
        .iter()
        .enumerate()
        .map(|(idx, (nm, doc))| OpcSheetInput {
            tab_index: idx,
            fallback_name: nm.as_str(),
            document: doc,
        })
        .collect();
    Ok(analyze_pid_tie_ins(&refs))
}

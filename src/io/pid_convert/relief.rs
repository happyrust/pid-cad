//! P&ID Overpressure & Emergency Relief Device Schedule (`API 520/521` / `SH/T 3006`),
//! bidirectional inlet/outlet piping & protected-equipment tracing, flare vs atmosphere
//! discharge classification, car-sealed-open (`CSO / LO`) block-valve compliance auditing,
//! and interactive `PIDRELIEF` / CLI `--export-relief` support (`L3-2 / L3-3 / L3-6 / L6-2` / `#170`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::{parse_sppid_line_number, ParsedLineNumber};
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::{classify_valve_role_and_subtype, ValveCategory, ValveSubtype};

/// Fixed 14-column CSV/TSV header for `PIDRELIEF EXPORT` and `--export-relief`.
pub const RELIEF_CSV_HEADERS: [&str; 14] = [
    "DeviceTag",
    "DeviceType",
    "DrawingCode",
    "ProtectedEquipment",
    "InletLine",
    "InletDN",
    "InletSpec",
    "OutletLine",
    "OutletDN",
    "OutletSpec",
    "Destination",
    "IsolationValves",
    "ComplianceStatus",
    "SetPressure",
];

/// Subtype of an overpressure / vacuum / emergency relief device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliefDeviceType {
    SafetyReliefValve,
    RuptureDisc,
    BreatherValve,
    EmergencyBlowdown,
}

impl ReliefDeviceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SafetyReliefValve => "safety_relief_valve",
            Self::RuptureDisc => "rupture_disc",
            Self::BreatherValve => "breather_valve",
            Self::EmergencyBlowdown => "emergency_blowdown",
        }
    }
}

/// Classified destination of the relief device's discharge stream (`API 521`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliefDestinationKind {
    FlareHeader,
    Atmosphere,
    ClosedSystem,
    Unknown,
}

impl ReliefDestinationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlareHeader => "flare_header",
            Self::Atmosphere => "atmosphere",
            Self::ClosedSystem => "closed_system",
            Self::Unknown => "unknown",
        }
    }
}

/// `API 520 / ASME VIII` isolation block-valve & connectivity compliance status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReliefComplianceStatus {
    Compliant,
    UnsealedBlockValve,
    UnconnectedRelief,
}

impl ReliefComplianceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compliant => "compliant",
            Self::UnsealedBlockValve => "unsealed_block_valve",
            Self::UnconnectedRelief => "unconnected_relief",
        }
    }

    pub fn is_violation(self) -> bool {
        !matches!(self, Self::Compliant)
    }
}

/// One analyzed pressure-relief device row in `PidReliefReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidReliefDeviceRow {
    pub device_tag: String,
    pub device_type: ReliefDeviceType,
    pub tab_index: usize,
    pub drawing_code: String,
    pub protected_equipment: String,
    pub inlet_line: String,
    pub inlet_dn: String,
    pub inlet_spec: String,
    pub outlet_line: String,
    pub outlet_dn: String,
    pub outlet_spec: String,
    pub destination: ReliefDestinationKind,
    pub isolation_valves: Vec<String>,
    pub compliance_status: ReliefComplianceStatus,
    pub set_pressure: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Summary statistics across all extracted relief devices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidReliefSummary {
    pub sheets_scanned: usize,
    pub total_devices: usize,
    pub safety_valves_count: usize,
    pub rupture_discs_count: usize,
    pub breather_valves_count: usize,
    pub blowdown_valves_count: usize,
    pub flare_discharges: usize,
    pub atmosphere_discharges: usize,
    pub closed_discharges: usize,
    pub compliant_count: usize,
    pub unsealed_valve_violations: usize,
    pub unconnected_count: usize,
}

/// Complete `PIDRELIEF` Overpressure Relief Schedule & Compliance report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidReliefReport {
    pub summary: PidReliefSummary,
    pub devices: Vec<PidReliefDeviceRow>,
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
enum RlKind {
    ReliefDevice(ReliefDeviceType),
    ManualBlockValve { is_cso_or_lo: bool },
    Equipment,
    OtherFitting,
    Opc { is_paired: bool },
    PipeSegment,
}

struct RlNode {
    tab_index: usize,
    drawing_code: String,
    kind: RlKind,
    tag_or_label: Option<String>,
    parsed_line: Option<ParsedLineNumber>,
    set_pressure: String,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    bbox: BBox2,
    ports: Vec<Pt2>,
    spans: Vec<Span2>,
    upm: f64,
}

struct SheetNoteText {
    at: Pt2,
    text: String,
}

struct RlGraph {
    nodes: Vec<RlNode>,
    adj: Vec<Vec<usize>>,
    notes_by_tab: BTreeMap<usize, Vec<SheetNoteText>>,
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

fn extract_tag_alpha_prefix(tag: &str) -> String {
    for seg in tag.trim().split(['-', '_', ' ']).filter(|s| !s.is_empty()) {
        if seg.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let alpha: String = seg
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        if !alpha.is_empty() {
            return alpha.to_ascii_uppercase();
        }
    }
    String::new()
}

/// Classify whether a symbol represents an overpressure/vacuum/emergency relief device.
pub fn classify_relief_device_type(
    cls: &str,
    type_code: &str,
    desc_hint_upper: &str,
    tag_opt: Option<&str>,
) -> Option<ReliefDeviceType> {
    if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
            | "PIDNozzle"
            | "PIDBranchPoint"
            | "PIDNote"
            | "PIDControlSystemFunction"
            | "PIDPipingNetworkSegment"
    ) || matches!(
        type_code.trim().to_ascii_uppercase().as_str(),
        "VESSEL" | "PUMP" | "TANK" | "EQUIPMENT" | "HEAT_EXCHANGER" | "NOZZLE" | "OPC" | "NOTE"
    ) {
        return None;
    }

    let pfx = tag_opt.map(extract_tag_alpha_prefix).unwrap_or_default();

    if matches!(pfx.as_str(), "PSE" | "RD") || desc_hint_upper.contains("RUPTURE") {
        return Some(ReliefDeviceType::RuptureDisc);
    }
    if pfx == "PVSV" || desc_hint_upper.contains("BREATHER") {
        return Some(ReliefDeviceType::BreatherValve);
    }
    if pfx == "BDV" || desc_hint_upper.contains("BLOWDOWN VALVE") {
        return Some(ReliefDeviceType::EmergencyBlowdown);
    }
    if matches!(pfx.as_str(), "PSV" | "PRV" | "TSV")
        || desc_hint_upper.contains("SAFETY VALVE")
        || desc_hint_upper.contains("RELIEF VALVE")
        || desc_hint_upper.contains("RELIEF DEVICE")
        || desc_hint_upper.contains("RELIEF-VALVE")
        || (matches!(cls, "PIDPipingComponent" | "PIDInlineInstrument")
            && matches!(pfx.as_str(), "SV" | "RV"))
    {
        return Some(ReliefDeviceType::SafetyReliefValve);
    }

    None
}

fn is_cso_or_lo_text(raw: &str) -> bool {
    let up = raw.trim().to_ascii_uppercase();
    if up.is_empty() {
        return false;
    }
    if up.contains("CAR SEALED OPEN")
        || up.contains("CAR-SEALED OPEN")
        || up.contains("LOCKED OPEN")
        || raw.contains("铅封开")
        || raw.contains("锁开")
    {
        return true;
    }
    for tok in up.split(|c: char| !c.is_ascii_alphanumeric() && c != '.') {
        let cleaned = tok.trim_matches('.');
        if matches!(cleaned, "CSO" | "C.S.O" | "LO" | "L.O") {
            return true;
        }
    }
    false
}

fn text_destination_hint(raw: &str) -> Option<ReliefDestinationKind> {
    if parse_sppid_line_number(raw.trim()).is_some() {
        return None;
    }
    let up = raw.trim().to_ascii_uppercase();
    if up == "ATM"
        || up.contains("TO ATM")
        || up.contains("ATMOSPHERE")
        || up.contains("SAFE LOCATION")
        || raw.contains("大气")
        || raw.contains("安全高处")
    {
        return Some(ReliefDestinationKind::Atmosphere);
    }
    if up.contains("FLARE") || raw.contains("火炬") || up.contains("BLOWDOWN HEADER") {
        return Some(ReliefDestinationKind::FlareHeader);
    }
    None
}

fn collect_sheet_notes(doc: &Document) -> Vec<SheetNoteText> {
    let mut notes = Vec::new();
    for ent in doc.model_space_entities() {
        match ent {
            EntityType::Text(t) => {
                let v = t.value.trim();
                if !v.is_empty() {
                    notes.push(SheetNoteText {
                        at: (t.insertion_point.x, t.insertion_point.y),
                        text: v.to_string(),
                    });
                }
            }
            EntityType::MText(mt) => {
                let cleaned = mt.value.replace("\\P", " ");
                let v = cleaned.trim();
                if !v.is_empty() {
                    notes.push(SheetNoteText {
                        at: (mt.insertion_point.x, mt.insertion_point.y),
                        text: v.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    notes
}

fn has_cso_annotation(
    tag_opt: Option<&str>,
    center: Pt2,
    upm: f64,
    notes: &[SheetNoteText],
    props_cso: bool,
) -> bool {
    props_cso
        || tag_opt.is_some_and(is_cso_or_lo_text)
        || notes.iter().any(|n| {
            is_cso_or_lo_text(&n.text) && (n.at.0 - center.0).hypot(n.at.1 - center.1) <= 18.0 * upm
        })
}

fn classify_rl_kind(
    cls: &str,
    type_code: &str,
    desc_up: &str,
    tag_opt: Option<&str>,
    is_cso: bool,
) -> RlKind {
    if let Some(rt) = classify_relief_device_type(cls, type_code, desc_up, tag_opt) {
        return RlKind::ReliefDevice(rt);
    }
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
        return RlKind::Equipment;
    }
    if cls == "PIDBranchPoint" || type_code.eq_ignore_ascii_case("opc") {
        return RlKind::Opc { is_paired: false };
    }
    if classify_valve_role_and_subtype(cls, type_code, desc_up, tag_opt).is_some_and(
        |(cat, sub)| cat == ValveCategory::ManualBulk && sub != ValveSubtype::CheckValve,
    ) {
        return RlKind::ManualBlockValve {
            is_cso_or_lo: is_cso,
        };
    }
    RlKind::OtherFitting
}

fn extract_authored_rl_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    notes: &[SheetNoteText],
    nodes: &mut Vec<RlNode>,
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
                nodes.push(RlNode {
                    tab_index,
                    drawing_code: drawing_code.to_string(),
                    kind: RlKind::PipeSegment,
                    tag_or_label: resolved,
                    parsed_line: parsed,
                    set_pressure: String::new(),
                    primary_handle: h,
                    member_handles: vec![h],
                    center,
                    bbox,
                    ports: Vec::new(),
                    spans: vec![sp],
                    upm,
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
        let tag_opt = (!auth.tag.trim().is_empty()).then(|| auth.tag.trim().to_string());
        let props_cso = pmap
            .values()
            .filter_map(Value::as_str)
            .any(is_cso_or_lo_text);
        let set_pressure = ["SetPressure", "ReliefPressure", "整定压力", "开启压力"]
            .iter()
            .find_map(|k| {
                pmap.get(*k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_default();

        let is_cso = has_cso_annotation(tag_opt.as_deref(), center, upm, notes, props_cso);
        let kind = classify_rl_kind(cls, &auth.type_code, &desc_up, tag_opt.as_deref(), is_cso);
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

        nodes.push(RlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            tag_or_label: tag_opt,
            parsed_line: None,
            set_pressure,
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
        if let Some(indices) = pipe_indices_by_h.get(&sh) {
            for &idx in indices {
                if nodes[idx].parsed_line.is_none() {
                    let sp = nodes[idx].spans[0];
                    let resolved = resolve_span_lbl(sp, lt.as_deref());
                    nodes[idx].parsed_line = resolved.as_deref().and_then(parse_sppid_line_number);
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
            nodes.push(RlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: RlKind::PipeSegment,
                tag_or_label: resolved,
                parsed_line: parsed,
                set_pressure: String::new(),
                primary_handle: sh,
                member_handles: vec![sh],
                center,
                bbox,
                ports: Vec::new(),
                spans: vec![sp],
                upm,
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
            let is_cso = has_cso_annotation(tag_opt.as_deref(), sym.at, upm, notes, false);
            let kind = classify_rl_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref(), is_cso);
            nodes.push(RlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind,
                tag_or_label: tag_opt,
                parsed_line: None,
                set_pressure: String::new(),
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
            nodes.push(RlNode {
                tab_index,
                drawing_code: drawing_code.to_string(),
                kind: RlKind::PipeSegment,
                tag_or_label: lbl_opt,
                parsed_line: parsed,
                set_pressure: String::new(),
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
}

fn extract_unpromoted_rl_nodes(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    notes: &[SheetNoteText],
    nodes: &mut Vec<RlNode>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_rl_nodes(tab_index, drawing_code, &scratch, notes, nodes);
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
        let is_cso = has_cso_annotation(tag_opt.as_deref(), sym.at, upm, notes, false);
        let kind = classify_rl_kind(&sym.sppid_class, "", &desc_up, tag_opt.as_deref(), is_cso);

        nodes.push(RlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind,
            tag_or_label: tag_opt,
            parsed_line: None,
            set_pressure: String::new(),
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
        let parsed = lbl_opt.as_deref().and_then(parse_sppid_line_number);
        let h = Handle::new(rh);
        let (center, bbox) = spans_bbox_and_center(&[sp]);
        nodes.push(RlNode {
            tab_index,
            drawing_code: drawing_code.to_string(),
            kind: RlKind::PipeSegment,
            tag_or_label: lbl_opt,
            parsed_line: parsed,
            set_pressure: String::new(),
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

fn merge_opc_into_rl_sheet(
    c: &OpcConnectorItem,
    drawing_code: &str,
    upm: f64,
    sheet_range: std::ops::Range<usize>,
    nodes: &mut Vec<RlNode>,
) -> usize {
    let paired = c.status == OpcStatus::Paired;
    for idx in sheet_range {
        if nodes[idx].kind == RlKind::PipeSegment {
            continue;
        }
        let handle_overlap = nodes[idx].primary_handle == c.primary_handle
            || nodes[idx]
                .member_handles
                .iter()
                .any(|mh| c.member_handles.contains(mh));
        let near_opc = matches!(nodes[idx].kind, RlKind::Opc { .. })
            && (nodes[idx].center.0 - c.x).hypot(nodes[idx].center.1 - c.y) <= 8.0 * upm;
        if handle_overlap || near_opc {
            nodes[idx].kind = RlKind::Opc { is_paired: paired };
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
    nodes.push(RlNode {
        tab_index: c.tab_index,
        drawing_code: drawing_code.to_string(),
        kind: RlKind::Opc { is_paired: paired },
        tag_or_label: c.target_dwg_raw.clone().or_else(|| c.line_label.clone()),
        parsed_line: None,
        set_pressure: String::new(),
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

fn wire_rl_local_edges(
    sheet_indices: &[usize],
    upm: f64,
    nodes: &[RlNode],
    adj: &mut [Vec<usize>],
) {
    let tight_tol = 2.0 * upm;
    let pipe_tol = 6.5 * upm;
    let sym_tol = 8.5 * upm;

    let mut pipe_nodes = Vec::new();
    let mut sym_nodes = Vec::new();
    for &idx in sheet_indices {
        if nodes[idx].kind == RlKind::PipeSegment {
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
                                    nodes[sn].kind != RlKind::Equipment
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
                add_undirected_edge(adj, na, nb);
            }
        }
    }

    for &pn in &pipe_nodes {
        for &sn in &sym_nodes {
            let margin = match nodes[sn].kind {
                RlKind::Equipment => 12.0 * upm,
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

    // Direct relief-to-equipment mounting (e.g. breather valve mounted directly on tank roof)
    for (ai, &sa) in sym_nodes.iter().enumerate() {
        for &sb in &sym_nodes[(ai + 1)..] {
            let pair_ok = matches!(
                (nodes[sa].kind, nodes[sb].kind),
                (RlKind::ReliefDevice(_), RlKind::Equipment)
                    | (RlKind::Equipment, RlKind::ReliefDevice(_))
            );
            if pair_ok && bboxes_overlap(nodes[sa].bbox, nodes[sb].bbox, 10.0 * upm) {
                add_undirected_edge(adj, sa, sb);
            }
        }
    }
}

fn propagate_rl_pipe_labels(graph: &mut RlGraph) {
    let mut updates: Vec<(usize, String, ParsedLineNumber)> = Vec::new();
    for idx in 0..graph.nodes.len() {
        if graph.nodes[idx].kind != RlKind::PipeSegment || graph.nodes[idx].parsed_line.is_some() {
            continue;
        }
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut q: VecDeque<usize> = VecDeque::new();
        visited.insert(idx);
        q.push_back(idx);
        let mut found: BTreeMap<String, (String, ParsedLineNumber)> = BTreeMap::new();
        while let Some(cur) = q.pop_front() {
            let cnd = &graph.nodes[cur];
            if let (RlKind::PipeSegment, Some(raw), Some(parsed)) =
                (cnd.kind, &cnd.tag_or_label, &cnd.parsed_line)
            {
                found.insert(parsed.pipeline_name(), (raw.clone(), parsed.clone()));
                continue;
            }
            if matches!(
                cnd.kind,
                RlKind::Equipment | RlKind::ReliefDevice(_) | RlKind::Opc { is_paired: false }
            ) {
                continue;
            }
            for &nxt in &graph.adj[cur] {
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
        graph.nodes[idx].tag_or_label = Some(raw);
        graph.nodes[idx].parsed_line = Some(parsed);
    }
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

fn build_rl_graph(sheets: &[OpcSheetInput<'_>]) -> RlGraph {
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut tab_drawings: BTreeMap<usize, String> = BTreeMap::new();
    for c in &opc_rep.connectors {
        tab_drawings
            .entry(c.tab_index)
            .or_insert_with(|| c.drawing_code.clone());
    }
    let mut nodes: Vec<RlNode> = Vec::new();
    let mut notes_by_tab: BTreeMap<usize, Vec<SheetNoteText>> = BTreeMap::new();
    let mut sheet_ranges: BTreeMap<usize, (f64, std::ops::Range<usize>)> = BTreeMap::new();

    for sh in sheets {
        let start_idx = nodes.len();
        let upm = crate::io::pid_legend::guess_units_per_mm(sh.document).max(1e-6);
        let dcode = tab_drawings
            .entry(sh.tab_index)
            .or_insert_with(|| clean_drawing_stem(sh.fallback_name))
            .clone();
        let notes = collect_sheet_notes(sh.document);
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_rl_nodes(sh.tab_index, &dcode, sh.document, &notes, &mut nodes);
        } else {
            extract_unpromoted_rl_nodes(sh.tab_index, &dcode, sh.document, &notes, &mut nodes);
        }
        notes_by_tab.insert(sh.tab_index, notes);
        let end_idx = nodes.len();
        sheet_ranges.insert(sh.tab_index, (upm, start_idx..end_idx));
    }

    let mut opc_node_map: BTreeMap<(usize, Handle), usize> = BTreeMap::new();
    for c in &opc_rep.connectors {
        if let Some((upm, range)) = sheet_ranges.get(&c.tab_index).cloned() {
            let idx = merge_opc_into_rl_sheet(c, &c.drawing_code, upm, range.clone(), &mut nodes);
            let new_end = range.end.max(idx + 1);
            sheet_ranges.insert(c.tab_index, (upm, range.start..new_end));
            opc_node_map.insert((c.tab_index, c.primary_handle), idx);
        }
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for &(upm, ref range) in sheet_ranges.values() {
        let indices: Vec<usize> = range.clone().collect();
        wire_rl_local_edges(&indices, upm, &nodes, &mut adj);
    }

    for c in &opc_rep.connectors {
        if c.status != OpcStatus::Paired {
            continue;
        }
        let Some(&ni) = opc_node_map.get(&(c.tab_index, c.primary_handle)) else {
            continue;
        };
        let Some(ref peer) = c.peer else {
            continue;
        };
        if let Some(&pi) = opc_node_map.get(&(peer.tab_index, peer.primary_handle)) {
            add_undirected_edge(&mut adj, ni, pi);
        }
    }

    let mut graph = RlGraph {
        nodes,
        adj,
        notes_by_tab,
    };
    propagate_rl_pipe_labels(&mut graph);
    graph
}

struct TracedBranch {
    reached_equip: Option<String>,
    line_name: String,
    nominal_diameter: String,
    dn_mm: f64,
    fluid_code: String,
    piping_spec: String,
    series_valves: Vec<(String, bool)>,
    note_dest_hint: Option<ReliefDestinationKind>,
    handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

fn parse_dn_numeric_mm(dn_attr: &str) -> f64 {
    dn_attr
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn classify_fluid_destination(fluid_code: &str) -> Option<ReliefDestinationKind> {
    let f = fluid_code.trim().to_ascii_uppercase();
    if matches!(
        f.as_str(),
        "FL" | "FLH" | "FLL" | "RV" | "SV" | "BD" | "CBD" | "SLOP"
    ) {
        Some(ReliefDestinationKind::FlareHeader)
    } else if matches!(f.as_str(), "ATM" | "VT" | "VNT" | "AIR") {
        Some(ReliefDestinationKind::Atmosphere)
    } else {
        None
    }
}

fn trace_relief_branch(graph: &RlGraph, relief_idx: usize, start_nb: usize) -> TracedBranch {
    let mut visited: BTreeSet<usize> = BTreeSet::new();
    visited.insert(relief_idx);
    visited.insert(start_nb);

    let mut q: VecDeque<(usize, bool)> = VecDeque::new();
    // `in_series` stays true until the path branches at a degree >= 3 pipe tee
    q.push_back((start_nb, true));

    let mut reached_equip: Option<String> = None;
    let mut first_parsed: Option<ParsedLineNumber> = None;
    let mut first_raw_line: Option<String> = None;
    let mut series_valves: Vec<(String, bool)> = Vec::new();
    let mut seen_valves: BTreeSet<String> = BTreeSet::new();
    let mut note_dest_hint: Option<ReliefDestinationKind> = None;
    let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();

    while let Some((cur, in_series)) = q.pop_front() {
        let cnd = &graph.nodes[cur];
        let dst_h = handles_by_tab.entry(cnd.tab_index).or_default();
        for &mh in &cnd.member_handles {
            if !dst_h.contains(&mh) {
                dst_h.push(mh);
            }
        }

        match cnd.kind {
            RlKind::Equipment => {
                if reached_equip.is_none() {
                    reached_equip = cnd
                        .tag_or_label
                        .clone()
                        .or_else(|| Some(format!("UNTAGGED-{:X}", cnd.primary_handle.value())));
                }
                continue;
            }
            RlKind::ReliefDevice(_) => {
                continue;
            }
            RlKind::ManualBlockValve { is_cso_or_lo } => {
                if in_series {
                    let vtag = cnd
                        .tag_or_label
                        .clone()
                        .unwrap_or_else(|| format!("VALVE-{:X}", cnd.primary_handle.value()));
                    if seen_valves.insert(vtag.clone()) {
                        series_valves.push((vtag, is_cso_or_lo));
                    }
                }
            }
            RlKind::PipeSegment => {
                if let Some(p) = cnd.parsed_line.as_ref().filter(|_| first_parsed.is_none()) {
                    first_parsed = Some(p.clone());
                    first_raw_line = Some(p.pipeline_name());
                }
                if let Some(notes) = graph
                    .notes_by_tab
                    .get(&cnd.tab_index)
                    .filter(|_| note_dest_hint.is_none())
                {
                    note_dest_hint = notes.iter().find_map(|n| {
                        cnd.spans
                            .iter()
                            .any(|&sp| point_to_span_dist(n.at, sp) <= 18.0 * cnd.upm)
                            .then(|| text_destination_hint(&n.text))
                            .flatten()
                    });
                }
            }
            RlKind::Opc { is_paired } => {
                if let Some(lbl) = cnd
                    .tag_or_label
                    .as_deref()
                    .filter(|_| note_dest_hint.is_none())
                {
                    note_dest_hint = text_destination_hint(lbl);
                }
                if !is_paired {
                    continue;
                }
            }
            RlKind::OtherFitting => {}
        }

        let deg = graph.adj[cur].len();
        let next_in_series = in_series && deg <= 2;

        for &nxt in &graph.adj[cur] {
            if visited.len() < 40 && visited.insert(nxt) {
                q.push_back((nxt, next_in_series));
            }
        }
    }

    let (line_name, nominal_diameter, dn_mm, fluid_code, piping_spec) = match first_parsed {
        Some(p) => {
            let mm = parse_dn_numeric_mm(&p.nominal_diameter_attr);
            (
                first_raw_line.unwrap_or_else(|| p.pipeline_name()),
                p.nominal_diameter_attr,
                mm,
                p.fluid_code.to_ascii_uppercase(),
                p.materials_class,
            )
        }
        None => (
            String::new(),
            String::new(),
            0.0,
            String::new(),
            String::new(),
        ),
    };

    TracedBranch {
        reached_equip,
        line_name,
        nominal_diameter,
        dn_mm,
        fluid_code,
        piping_spec,
        series_valves,
        note_dest_hint,
        handles_by_tab,
    }
}

fn branch_inlet_score(b: &TracedBranch) -> i32 {
    let mut score = 0i32;
    if classify_fluid_destination(&b.fluid_code).is_some() || b.note_dest_hint.is_some() {
        score -= 60;
    }
    if b.reached_equip.is_some() {
        score += 50;
    }
    if !b.fluid_code.is_empty() && classify_fluid_destination(&b.fluid_code).is_none() {
        score += 20;
    }
    score
}

/// Check whether a `PidReliefDeviceRow` matches a user tag or equipment query.
pub fn relief_row_matches_query(row: &PidReliefDeviceRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    let q_comp: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let tag_up = row.device_tag.to_ascii_uppercase();
    let tag_comp: String = tag_up
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let eq_up = row.protected_equipment.to_ascii_uppercase();
    tag_up.contains(&q)
        || (!q_comp.is_empty() && tag_comp.contains(&q_comp))
        || (!eq_up.is_empty() && eq_up.contains(&q))
}

/// Analyze all overpressure/vacuum/emergency relief devices across `sheets`, tracing
/// protected equipment, inlet/outlet specs, discharge destinations, and `CSO/LO` compliance (`#170`).
pub fn analyze_pid_relief_devices(sheets: &[OpcSheetInput<'_>]) -> PidReliefReport {
    let graph = build_rl_graph(sheets);
    let mut devices: Vec<PidReliefDeviceRow> = Vec::new();

    for (ri, rnd) in graph.nodes.iter().enumerate() {
        let RlKind::ReliefDevice(dtype) = rnd.kind else {
            continue;
        };
        let device_tag = rnd
            .tag_or_label
            .clone()
            .unwrap_or_else(|| format!("RELIEF-{:X}", rnd.primary_handle.value()));

        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
        handles_by_tab
            .entry(rnd.tab_index)
            .or_default()
            .extend_from_slice(&rnd.member_handles);

        let nbs = &graph.adj[ri];
        if nbs.is_empty() {
            devices.push(PidReliefDeviceRow {
                device_tag,
                device_type: dtype,
                tab_index: rnd.tab_index,
                drawing_code: rnd.drawing_code.clone(),
                protected_equipment: String::new(),
                inlet_line: String::new(),
                inlet_dn: String::new(),
                inlet_spec: String::new(),
                outlet_line: String::new(),
                outlet_dn: String::new(),
                outlet_spec: String::new(),
                destination: ReliefDestinationKind::Unknown,
                isolation_valves: Vec::new(),
                compliance_status: ReliefComplianceStatus::UnconnectedRelief,
                set_pressure: rnd.set_pressure.clone(),
                primary_handle_hex: format!("{:X}", rnd.primary_handle.value()),
                x: rnd.center.0,
                y: rnd.center.1,
                primary_handle: rnd.primary_handle,
                handles_by_tab,
            });
            continue;
        }

        let mut branches: Vec<TracedBranch> = nbs
            .iter()
            .map(|&nb| trace_relief_branch(&graph, ri, nb))
            .collect();

        branches.sort_by(|a, b| {
            branch_inlet_score(b)
                .cmp(&branch_inlet_score(a))
                .then_with(|| {
                    let da = if a.dn_mm > 0.0 { a.dn_mm } else { 99999.0 };
                    let db = if b.dn_mm > 0.0 { b.dn_mm } else { 99999.0 };
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let near_dev_dest = graph.notes_by_tab.get(&rnd.tab_index).and_then(|notes| {
            notes.iter().find_map(|n| {
                ((n.at.0 - rnd.center.0).hypot(n.at.1 - rnd.center.1) <= 22.0 * rnd.upm)
                    .then(|| text_destination_hint(&n.text))
                    .flatten()
            })
        });

        let (inlet_opt, outlet_opt) = if branches.len() >= 2 {
            let mut it = branches.into_iter();
            let in_b = it.next();
            let out_b = it.next();
            (in_b, out_b)
        } else {
            let only = branches.into_iter().next().unwrap();
            if branch_inlet_score(&only) >= 0 {
                (Some(only), None)
            } else {
                (None, Some(only))
            }
        };

        let mut isolation_valves: Vec<String> = Vec::new();
        let mut any_unsealed = false;

        for b in [inlet_opt.as_ref(), outlet_opt.as_ref()]
            .into_iter()
            .flatten()
        {
            for (&ti, hs) in &b.handles_by_tab {
                let dst = handles_by_tab.entry(ti).or_default();
                for &h in hs {
                    if !dst.contains(&h) {
                        dst.push(h);
                    }
                }
            }
            for (vtag, is_cso) in &b.series_valves {
                if *is_cso {
                    isolation_valves.push(format!("{vtag}(CSO)"));
                } else {
                    any_unsealed = true;
                    isolation_valves.push(format!("{vtag}(UNSEALED)"));
                }
            }
        }

        let (protected_equipment, inlet_line, inlet_dn, inlet_spec) = match inlet_opt {
            Some(ref ib) => (
                ib.reached_equip.clone().unwrap_or_default(),
                ib.line_name.clone(),
                ib.nominal_diameter.clone(),
                ib.piping_spec.clone(),
            ),
            None => (String::new(), String::new(), String::new(), String::new()),
        };

        let (outlet_line, outlet_dn, outlet_spec, destination) = match outlet_opt {
            Some(ref ob) => {
                let dest = classify_fluid_destination(&ob.fluid_code)
                    .or(ob.note_dest_hint)
                    .or(near_dev_dest)
                    .or_else(|| {
                        ob.reached_equip
                            .as_ref()
                            .map(|_| ReliefDestinationKind::ClosedSystem)
                    })
                    .unwrap_or_else(|| {
                        if dtype == ReliefDeviceType::BreatherValve {
                            ReliefDestinationKind::Atmosphere
                        } else {
                            ReliefDestinationKind::Unknown
                        }
                    });
                (
                    ob.line_name.clone(),
                    ob.nominal_diameter.clone(),
                    ob.piping_spec.clone(),
                    dest,
                )
            }
            None => {
                let dest = near_dev_dest.unwrap_or_else(|| {
                    if dtype == ReliefDeviceType::BreatherValve {
                        ReliefDestinationKind::Atmosphere
                    } else {
                        ReliefDestinationKind::Unknown
                    }
                });
                (String::new(), String::new(), String::new(), dest)
            }
        };

        let compliance_status = if any_unsealed {
            ReliefComplianceStatus::UnsealedBlockValve
        } else {
            ReliefComplianceStatus::Compliant
        };

        devices.push(PidReliefDeviceRow {
            device_tag,
            device_type: dtype,
            tab_index: rnd.tab_index,
            drawing_code: rnd.drawing_code.clone(),
            protected_equipment,
            inlet_line,
            inlet_dn,
            inlet_spec,
            outlet_line,
            outlet_dn,
            outlet_spec,
            destination,
            isolation_valves,
            compliance_status,
            set_pressure: rnd.set_pressure.clone(),
            primary_handle_hex: format!("{:X}", rnd.primary_handle.value()),
            x: rnd.center.0,
            y: rnd.center.1,
            primary_handle: rnd.primary_handle,
            handles_by_tab,
        });
    }

    devices.sort_by(|a, b| {
        a.tab_index
            .cmp(&b.tab_index)
            .then_with(|| a.device_tag.cmp(&b.device_tag))
    });

    let safety_valves_count = devices
        .iter()
        .filter(|d| d.device_type == ReliefDeviceType::SafetyReliefValve)
        .count();
    let rupture_discs_count = devices
        .iter()
        .filter(|d| d.device_type == ReliefDeviceType::RuptureDisc)
        .count();
    let breather_valves_count = devices
        .iter()
        .filter(|d| d.device_type == ReliefDeviceType::BreatherValve)
        .count();
    let blowdown_valves_count = devices
        .iter()
        .filter(|d| d.device_type == ReliefDeviceType::EmergencyBlowdown)
        .count();
    let flare_discharges = devices
        .iter()
        .filter(|d| d.destination == ReliefDestinationKind::FlareHeader)
        .count();
    let atmosphere_discharges = devices
        .iter()
        .filter(|d| d.destination == ReliefDestinationKind::Atmosphere)
        .count();
    let closed_discharges = devices
        .iter()
        .filter(|d| d.destination == ReliefDestinationKind::ClosedSystem)
        .count();
    let compliant_count = devices
        .iter()
        .filter(|d| d.compliance_status == ReliefComplianceStatus::Compliant)
        .count();
    let unsealed_valve_violations = devices
        .iter()
        .filter(|d| d.compliance_status == ReliefComplianceStatus::UnsealedBlockValve)
        .count();
    let unconnected_count = devices
        .iter()
        .filter(|d| d.compliance_status == ReliefComplianceStatus::UnconnectedRelief)
        .count();

    PidReliefReport {
        summary: PidReliefSummary {
            sheets_scanned: sheets.len(),
            total_devices: devices.len(),
            safety_valves_count,
            rupture_discs_count,
            breather_valves_count,
            blowdown_valves_count,
            flare_discharges,
            atmosphere_discharges,
            closed_discharges,
            compliant_count,
            unsealed_valve_violations,
            unconnected_count,
        },
        devices,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidReliefReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_relief_delimited(report: &PidReliefReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = RELIEF_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for d in &report.devices {
        let cols = [
            d.device_tag.clone(),
            d.device_type.as_str().to_string(),
            d.drawing_code.clone(),
            d.protected_equipment.clone(),
            d.inlet_line.clone(),
            d.inlet_dn.clone(),
            d.inlet_spec.clone(),
            d.outlet_line.clone(),
            d.outlet_dn.clone(),
            d.outlet_spec.clone(),
            d.destination.as_str().to_string(),
            d.isolation_valves.join(";"),
            d.compliance_status.as_str().to_string(),
            d.set_pressure.clone(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidReliefReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_relief_file(
    report: &PidReliefReport,
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
            let body = format_pid_relief_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_relief_delimited(report, '\t');
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
/// and build a `PidReliefReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_relief_from_path(input: &std::path::Path) -> Result<PidReliefReport, String> {
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
    Ok(analyze_pid_relief_devices(&refs))
}

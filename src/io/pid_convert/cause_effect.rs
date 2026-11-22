//! P&ID Safety Instrumented System (SIS / ESD) Cause & Effect Matrix (`IEC 61511` / `SH/T 3018`),
//! cross-sheet loop & signal-line union-find linking, trip-chain completeness auditing,
//! and interactive `PIDCAUSE` / CLI `--export-cause-effect` support (`L3-2 / L3-6 / L6-2` / `#172`).

use std::collections::{BTreeMap, BTreeSet};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Fixed 12-column CSV/TSV header for `PIDCAUSE EXPORT` and `--export-cause-effect`.
pub const CAUSE_EFFECT_CSV_HEADERS: [&str; 12] = [
    "InterlockId",
    "LoopNumber",
    "Status",
    "TriggerKind",
    "DrawingCodes",
    "ProtectedSource",
    "CauseTags",
    "LogicSolvers",
    "EffectActions",
    "FeedbackTags",
    "CrossedSheets",
    "DiagnosticNote",
];

/// Initiating trigger category for a SIS / ESD safety interlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseTriggerKind {
    ProcessTripSensor,
    EmergencyHandSwitch,
    MixedSensorAndSwitch,
    UnassignedTrigger,
}

impl CauseTriggerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProcessTripSensor => "process_trip_sensor",
            Self::EmergencyHandSwitch => "emergency_hand_switch",
            Self::MixedSensorAndSwitch => "mixed_sensor_and_switch",
            Self::UnassignedTrigger => "unassigned_trigger",
        }
    }
}

/// Completeness status of a `Cause -> Logic -> Effect` safety interlock row (`IEC 61511`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterlockStatus {
    CompleteInterlock,
    MissingActuator,
    OrphanActuator,
}

impl InterlockStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CompleteInterlock => "complete_interlock",
            Self::MissingActuator => "missing_actuator",
            Self::OrphanActuator => "orphan_actuator",
        }
    }

    pub fn is_incomplete(self) -> bool {
        !matches!(self, Self::CompleteInterlock)
    }
}

/// One safety interlock row in the SIS / ESD Cause & Effect Matrix (`PIDCAUSE`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidCauseEffectRow {
    pub interlock_id: String,
    pub loop_number: String,
    pub status: InterlockStatus,
    pub trigger_kind: CauseTriggerKind,
    pub drawing_codes: Vec<String>,
    pub crossed_sheets: usize,
    pub protected_source: String,
    pub cause_tags: Vec<String>,
    pub logic_solvers: Vec<String>,
    pub effect_actions: Vec<String>,
    pub feedback_tags: Vec<String>,
    pub diagnostic_note: String,
    pub primary_tab: usize,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Summary statistics across all extracted SIS / ESD Cause & Effect rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidCauseEffectSummary {
    pub sheets_scanned: usize,
    pub total_interlocks: usize,
    pub multi_sheet_interlocks: usize,
    pub complete_count: usize,
    pub missing_actuator_count: usize,
    pub orphan_actuator_count: usize,
    pub total_cause_sensors: usize,
    pub total_trip_actuators: usize,
}

/// Complete SIS / ESD Cause & Effect Matrix report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidCauseEffectReport {
    pub summary: PidCauseEffectSummary,
    pub interlocks: Vec<PidCauseEffectRow>,
}

type Pt2 = (f64, f64);

#[derive(Clone, Copy)]
struct Span2 {
    a: Pt2,
    b: Pt2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CeSymRole {
    ProcessTripCause,
    UnconditionalSwitchCause,
    ConditionalHandSwitch,
    LogicSolver,
    ShutdownValveClose,
    BlowdownValveOpen,
    PositionFeedback,
    RotatingEquipment,
    StaticEquipment,
}

struct CeSymNode {
    tab_index: usize,
    drawing_code: String,
    tag: String,
    loop_num: String,
    role: CeSymRole,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    upm: f64,
}

struct SheetPipeInfo {
    spans: Vec<(Span2, String)>,
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

fn entity_center(doc: &Document, h: Handle) -> Pt2 {
    match doc.get_entity(h) {
        Some(EntityType::Insert(ins)) => (ins.insert_point.x, ins.insert_point.y),
        Some(EntityType::Circle(c)) => (c.center.x, c.center.y),
        Some(EntityType::Line(l)) => ((l.start.x + l.end.x) * 0.5, (l.start.y + l.end.y) * 0.5),
        Some(EntityType::LwPolyline(pl)) if !pl.vertices.is_empty() => {
            let mut sx = 0.0;
            let mut sy = 0.0;
            for v in &pl.vertices {
                sx += v.location.x;
                sy += v.location.y;
            }
            let n = pl.vertices.len() as f64;
            (sx / n, sy / n)
        }
        _ => (0.0, 0.0),
    }
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

fn spans_touch(s1: Span2, s2: Span2, tol: f64) -> bool {
    point_to_span_dist(s1.a, s2) <= tol
        || point_to_span_dist(s1.b, s2) <= tol
        || point_to_span_dist(s2.a, s1) <= tol
        || point_to_span_dist(s2.b, s1) <= tol
}

/// Decompose a P&ID instrument / valve / equipment tag into `(ALPHA_PREFIX, DIGITS_CORE)`.
pub fn parse_sis_tag_prefix_and_loop(raw_tag: &str) -> Option<(String, String)> {
    let trimmed = raw_tag.trim();
    if trimmed.is_empty() || parse_sppid_line_number(trimmed).is_some() {
        return None;
    }
    let mut pfx = String::new();
    let mut digits = String::new();
    let mut in_pfx = false;
    let mut in_digits = false;

    for seg in trimmed.split(['-', '_', ' ']).filter(|s| !s.is_empty()) {
        if !in_pfx && seg.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        for ch in seg.chars() {
            if !in_digits && ch.is_ascii_alphabetic() {
                in_pfx = true;
                pfx.push(ch.to_ascii_uppercase());
            } else if in_pfx && ch.is_ascii_digit() {
                in_digits = true;
                digits.push(ch);
            } else if in_digits {
                break;
            }
        }
        if in_pfx && in_digits {
            break;
        }
    }

    if pfx.is_empty() {
        None
    } else {
        Some((pfx, digits))
    }
}

fn is_process_trip_prefix(pfx: &str) -> bool {
    if matches!(pfx, "GDS" | "FD" | "ASHH" | "ASLL") {
        return true;
    }
    let Some(first) = pfx.chars().next() else {
        return false;
    };
    if !matches!(first, 'P' | 'L' | 'T' | 'F' | 'A') {
        return false;
    }
    let rest = &pfx[1..];
    let rest = rest.strip_prefix('D').unwrap_or(rest);
    matches!(
        rest,
        "SH" | "SHH"
            | "SL"
            | "SLL"
            | "SHL"
            | "ZT"
            | "ZA"
            | "ZAH"
            | "ZAL"
            | "ZAHH"
            | "ZALL"
            | "ZSH"
            | "ZSL"
            | "ZSHH"
            | "ZSLL"
            | "ZIC"
    ) && pfx != "ZIC"
}

fn classify_ce_role(cls: &str, type_code: &str, desc_up: &str, pfx: &str) -> Option<CeSymRole> {
    if matches!(cls, "PIDMechanicalEquipment")
        || type_code.eq_ignore_ascii_case("PUMP")
        || (cls == "PIDProcessEquipment"
            && (matches!(pfx, "P" | "K" | "B") || desc_up.contains("PUMP")))
    {
        return Some(CeSymRole::RotatingEquipment);
    }
    if matches!(
        cls,
        "PIDProcessVessel"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || matches!(
        type_code.trim().to_ascii_uppercase().as_str(),
        "VESSEL" | "TANK" | "EQUIPMENT" | "HEAT_EXCHANGER"
    ) {
        return Some(CeSymRole::StaticEquipment);
    }

    if matches!(pfx, "BDV" | "EDV") {
        return Some(CeSymRole::BlowdownValveOpen);
    }
    if matches!(
        pfx,
        "ESDV" | "SDV" | "XV" | "ESV" | "ZV" | "EZV" | "XZV" | "UV" | "SOV" | "MOV" | "POV"
    ) {
        return Some(CeSymRole::ShutdownValveClose);
    }
    if matches!(
        pfx,
        "ZSC" | "ZSO" | "ZIC" | "ZIO" | "ZS" | "ZI" | "ZSH" | "ZSL"
    ) {
        return Some(CeSymRole::PositionFeedback);
    }
    if matches!(pfx, "SIS" | "UZ" | "PZ" | "LZ" | "TZ" | "FZ" | "AZ" | "UK")
        || (pfx == "I" && cls == "PIDControlSystemFunction")
    {
        return Some(CeSymRole::LogicSolver);
    }
    if matches!(pfx, "ESD" | "ESDB" | "HSC" | "HSO" | "HSZ" | "HZ" | "UZS") {
        return Some(CeSymRole::UnconditionalSwitchCause);
    }
    if pfx == "HS" {
        return Some(CeSymRole::ConditionalHandSwitch);
    }
    if is_process_trip_prefix(pfx) {
        return Some(CeSymRole::ProcessTripCause);
    }

    None
}

fn is_signal_entity(ent: &EntityType, auth_opt: Option<&PidAuthor>) -> bool {
    if auth_opt.is_some_and(|a| a.type_code.eq_ignore_ascii_case("SIGNAL_LINE")) {
        return true;
    }
    let layer_up = ent.common().layer.trim().to_ascii_uppercase();
    let ltype_up = ent.common().linetype.trim().to_ascii_uppercase();
    if matches!(
        layer_up.as_str(),
        "12" | "SIGNAL" | "INST" | "INSTRUMENT" | "SIS" | "ESD"
    ) || layer_up.starts_with("INST")
        || layer_up.starts_with("SIG")
    {
        return true;
    }
    ltype_up.contains("DASH") || ltype_up.contains("HIDDEN") || ltype_up.contains("DOT")
}

fn collect_signal_spans(doc: &Document, sym_handles: &BTreeSet<Handle>) -> Vec<Span2> {
    let mut out = Vec::new();
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        if sym_handles.contains(&h) {
            continue;
        }
        let auth = PidAuthor::load(doc, h);
        if !is_signal_entity(ent, auth.as_ref()) {
            continue;
        }
        match ent {
            EntityType::Line(ln) => {
                out.push(Span2 {
                    a: (ln.start.x, ln.start.y),
                    b: (ln.end.x, ln.end.y),
                });
            }
            EntityType::LwPolyline(pl) => {
                for w in pl.vertices.windows(2) {
                    out.push(Span2 {
                        a: (w[0].location.x, w[0].location.y),
                        b: (w[1].location.x, w[1].location.y),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn extract_sheet_ce_data(
    tab_index: usize,
    fallback_name: &str,
    doc: &Document,
    nodes: &mut Vec<CeSymNode>,
    signal_spans_by_tab: &mut BTreeMap<usize, Vec<Span2>>,
    pipes_by_tab: &mut BTreeMap<usize, SheetPipeInfo>,
) {
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
        .unwrap_or_else(|| clean_drawing_stem(fallback_name));

    let mut sym_handles: BTreeSet<Handle> = BTreeSet::new();

    if has_author {
        for ent in eff_doc.model_space_entities() {
            let h = ent.common().handle;
            let Some(auth) = PidAuthor::load(eff_doc, h) else {
                continue;
            };
            if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
                continue;
            }
            let pmap: serde_json::Map<String, Value> =
                serde_json::from_str(&auth.props).unwrap_or_default();
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
                sym_handles.insert(mh);
            }
            let tag = auth.tag.trim();
            let Some((pfx, loop_num)) = parse_sis_tag_prefix_and_loop(tag) else {
                continue;
            };
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
            let desc_up = src_blk.to_ascii_uppercase();
            let Some(role) = classify_ce_role(cls, &auth.type_code, &desc_up, &pfx) else {
                continue;
            };
            let center = pmap
                .get("_at")
                .and_then(Value::as_str)
                .and_then(parse_pair_f64)
                .unwrap_or_else(|| entity_center(eff_doc, h));

            nodes.push(CeSymNode {
                tab_index,
                drawing_code: drawing_code.clone(),
                tag: tag.to_string(),
                loop_num,
                role,
                primary_handle: h,
                member_handles: members,
                center,
                upm,
            });
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
            if sym_handles.contains(&primary_h)
                || sym
                    .member_handles
                    .iter()
                    .any(|&mr| sym_handles.contains(&Handle::new(mr)))
            {
                continue;
            }
            let mut members = vec![primary_h];
            for &mr in sym.member_handles.iter().chain(sym.tag_handles.iter()) {
                let mh = Handle::new(mr);
                if !members.contains(&mh) {
                    members.push(mh);
                }
                sym_handles.insert(mh);
            }
            sym_handles.insert(primary_h);

            let Some(tag_raw) = sym.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            let Some((pfx, loop_num)) = parse_sis_tag_prefix_and_loop(tag_raw) else {
                continue;
            };
            let desc_up = format!(
                "{} {} {}",
                sym.source_block,
                sym.layer,
                sym.symbol_path.as_deref().unwrap_or("")
            )
            .to_ascii_uppercase();
            let Some(role) = classify_ce_role(&sym.sppid_class, "", &desc_up, &pfx) else {
                continue;
            };

            nodes.push(CeSymNode {
                tab_index,
                drawing_code: drawing_code.clone(),
                tag: tag_raw.to_string(),
                loop_num,
                role,
                primary_handle: primary_h,
                member_handles: members,
                center: sym.at,
                upm,
            });
        }

        let mut pipe_spans = Vec::new();
        for ps in &rec.pipe_segments {
            let sp = Span2 {
                a: ps.start,
                b: ps.end,
            };
            let mut best_lbl: Option<(f64, &str)> = None;
            for pl in &rec.pipe_labels {
                let d = point_to_span_dist(pl.at, sp);
                if d <= 40.0 * upm && best_lbl.is_none_or(|(bd, _)| d < bd) {
                    best_lbl = Some((d, pl.raw_text.as_str()));
                }
            }
            let lbl = best_lbl
                .and_then(|(_, t)| parse_sppid_line_number(t.trim().trim_end_matches('-')))
                .map(|p| p.pipeline_name())
                .unwrap_or_default();
            pipe_spans.push((sp, lbl));
        }
        pipes_by_tab.insert(tab_index, SheetPipeInfo { spans: pipe_spans });
    }

    signal_spans_by_tab.insert(tab_index, collect_signal_spans(eff_doc, &sym_handles));
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let root = self.find(self.parent[x]);
            self.parent[x] = root;
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

fn is_sis_loop_joinable(role: CeSymRole) -> bool {
    matches!(
        role,
        CeSymRole::ProcessTripCause
            | CeSymRole::UnconditionalSwitchCause
            | CeSymRole::ConditionalHandSwitch
            | CeSymRole::LogicSolver
            | CeSymRole::ShutdownValveClose
            | CeSymRole::BlowdownValveOpen
            | CeSymRole::PositionFeedback
    )
}

/// Check whether a `PidCauseEffectRow` matches a user tag or loop query.
pub fn cause_effect_row_matches_query(row: &PidCauseEffectRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    let q_comp: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let matches_str = |s: &str| {
        let su = s.to_ascii_uppercase();
        let sc: String = su.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        su.contains(&q) || (!q_comp.is_empty() && sc.contains(&q_comp))
    };

    matches_str(&row.interlock_id)
        || matches_str(&row.loop_number)
        || matches_str(&row.protected_source)
        || row.cause_tags.iter().any(|s| matches_str(s))
        || row.logic_solvers.iter().any(|s| matches_str(s))
        || row.effect_actions.iter().any(|s| matches_str(s))
        || row.feedback_tags.iter().any(|s| matches_str(s))
}

/// Extract the SIS / ESD Cause & Effect Matrix across `sheets`, linking multi-sheet loops
/// and signal-line topologies and auditing incomplete trip chains (`#172`).
pub fn analyze_pid_cause_effect(sheets: &[OpcSheetInput<'_>]) -> PidCauseEffectReport {
    let mut nodes: Vec<CeSymNode> = Vec::new();
    let mut signal_spans_by_tab: BTreeMap<usize, Vec<Span2>> = BTreeMap::new();
    let mut pipes_by_tab: BTreeMap<usize, SheetPipeInfo> = BTreeMap::new();

    for sh in sheets {
        extract_sheet_ce_data(
            sh.tab_index,
            sh.fallback_name,
            sh.document,
            &mut nodes,
            &mut signal_spans_by_tab,
            &mut pipes_by_tab,
        );
    }

    let n = nodes.len();
    let mut dsu = DisjointSet::new(n);

    // 1. Union SIS-joinable symbols sharing the same non-empty loop_num across all open sheets
    let mut by_loop: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, nd) in nodes.iter().enumerate() {
        if is_sis_loop_joinable(nd.role) && !nd.loop_num.is_empty() {
            by_loop.entry(nd.loop_num.clone()).or_default().push(idx);
        }
    }
    for indices in by_loop.values() {
        if let Some(&first) = indices.first() {
            for &other in &indices[1..] {
                dsu.union(first, other);
            }
        }
    }

    // 2. Union symbols connected via signal lines on each sheet (excluding StaticEquipment vessels
    // so signal Impulse lines touching vessel walls record vessel as protected_source without
    // merging two independent vessel instruments!)
    for (&tab_idx, spans) in &signal_spans_by_tab {
        if spans.is_empty() {
            continue;
        }
        let m = spans.len();
        let mut sig_dsu = DisjointSet::new(m);
        let sheet_upm = nodes
            .iter()
            .find(|nd| nd.tab_index == tab_idx)
            .map(|nd| nd.upm)
            .unwrap_or(1.0);

        for i in 0..m {
            for j in (i + 1)..m {
                if spans_touch(spans[i], spans[j], 2.5 * sheet_upm) {
                    sig_dsu.union(i, j);
                }
            }
        }

        let mut net_to_syms: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (ni, nd) in nodes.iter().enumerate() {
            if nd.tab_index != tab_idx || nd.role == CeSymRole::StaticEquipment {
                continue;
            }
            for (si, &sp) in spans.iter().enumerate() {
                if point_to_span_dist(nd.center, sp) <= 9.0 * nd.upm {
                    let root_s = sig_dsu.find(si);
                    net_to_syms.entry(root_s).or_default().push(ni);
                }
            }
        }

        for syms in net_to_syms.values() {
            if let Some(&first) = syms.first() {
                for &other in &syms[1..] {
                    dsu.union(first, other);
                }
            }
        }
    }

    let static_equips: Vec<&CeSymNode> = nodes
        .iter()
        .filter(|nd| nd.role == CeSymRole::StaticEquipment)
        .collect();

    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (idx, nd) in nodes.iter().enumerate() {
        if nd.role == CeSymRole::StaticEquipment {
            continue;
        }
        let root = dsu.find(idx);
        clusters.entry(root).or_default().push(idx);
    }

    let mut rows: Vec<PidCauseEffectRow> = Vec::new();

    for member_indices in clusters.values() {
        let has_core_sis = member_indices.iter().any(|&i| {
            matches!(
                nodes[i].role,
                CeSymRole::ProcessTripCause
                    | CeSymRole::UnconditionalSwitchCause
                    | CeSymRole::LogicSolver
                    | CeSymRole::ShutdownValveClose
                    | CeSymRole::BlowdownValveOpen
            )
        });
        if !has_core_sis {
            continue;
        }

        let mut cause_tags: BTreeSet<String> = BTreeSet::new();
        let mut logic_solvers: BTreeSet<String> = BTreeSet::new();
        let mut effect_actions: BTreeSet<String> = BTreeSet::new();
        let mut feedback_tags: BTreeSet<String> = BTreeSet::new();
        let mut drawing_set: BTreeSet<String> = BTreeSet::new();
        let mut tab_set: BTreeSet<usize> = BTreeSet::new();
        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
        let mut has_proc_sensor = false;
        let mut has_switch = false;
        let mut primary_node: Option<&CeSymNode> = None;
        let mut loop_number = String::new();
        let mut protected_source = String::new();

        for &idx in member_indices {
            let nd = &nodes[idx];
            drawing_set.insert(nd.drawing_code.clone());
            tab_set.insert(nd.tab_index);
            let dst = handles_by_tab.entry(nd.tab_index).or_default();
            for &mh in &nd.member_handles {
                if !dst.contains(&mh) {
                    dst.push(mh);
                }
            }
            if loop_number.is_empty() && !nd.loop_num.is_empty() {
                loop_number = nd.loop_num.clone();
            }

            match nd.role {
                CeSymRole::ProcessTripCause => {
                    has_proc_sensor = true;
                    cause_tags.insert(nd.tag.clone());
                    if primary_node.is_none() {
                        primary_node = Some(nd);
                    }
                }
                CeSymRole::UnconditionalSwitchCause | CeSymRole::ConditionalHandSwitch => {
                    has_switch = true;
                    cause_tags.insert(nd.tag.clone());
                    if primary_node.is_none() {
                        primary_node = Some(nd);
                    }
                }
                CeSymRole::LogicSolver => {
                    logic_solvers.insert(nd.tag.clone());
                    if primary_node.is_none() {
                        primary_node = Some(nd);
                    }
                }
                CeSymRole::ShutdownValveClose => {
                    effect_actions.insert(format!("{}(CLOSE_ISOLATE)", nd.tag));
                    if primary_node.is_none() {
                        primary_node = Some(nd);
                    }
                }
                CeSymRole::BlowdownValveOpen => {
                    effect_actions.insert(format!("{}(OPEN_BLOWDOWN)", nd.tag));
                    if primary_node.is_none() {
                        primary_node = Some(nd);
                    }
                }
                CeSymRole::RotatingEquipment => {
                    effect_actions.insert(format!("{}(TRIP_STOP)", nd.tag));
                }
                CeSymRole::PositionFeedback => {
                    feedback_tags.insert(nd.tag.clone());
                }
                CeSymRole::StaticEquipment => {}
            }

            if protected_source.is_empty() {
                if let Some(eq) = static_equips.iter().find(|eq| {
                    eq.tab_index == nd.tab_index
                        && (eq.center.0 - nd.center.0).hypot(eq.center.1 - nd.center.1)
                            <= 65.0 * nd.upm
                }) {
                    protected_source = eq.tag.clone();
                } else if let Some((_, lbl)) = pipes_by_tab.get(&nd.tab_index).and_then(|pinfo| {
                    pinfo.spans.iter().find(|(sp, lbl)| {
                        !lbl.is_empty() && point_to_span_dist(nd.center, *sp) <= 18.0 * nd.upm
                    })
                }) {
                    protected_source = lbl.clone();
                }
            }
        }

        let Some(anchor) = primary_node.or_else(|| member_indices.first().map(|&i| &nodes[i]))
        else {
            continue;
        };

        let status = match (!cause_tags.is_empty(), !effect_actions.is_empty()) {
            (true, true) => InterlockStatus::CompleteInterlock,
            (true, false) => InterlockStatus::MissingActuator,
            (false, true) => InterlockStatus::OrphanActuator,
            (false, false) => InterlockStatus::MissingActuator,
        };

        let trigger_kind = match (has_proc_sensor, has_switch) {
            (true, true) => CauseTriggerKind::MixedSensorAndSwitch,
            (true, false) => CauseTriggerKind::ProcessTripSensor,
            (false, true) => CauseTriggerKind::EmergencyHandSwitch,
            (false, false) => CauseTriggerKind::UnassignedTrigger,
        };

        let diagnostic_note = match status {
            InterlockStatus::CompleteInterlock => {
                "触发源与安全切断/停机执行元件已形成闭环".to_string()
            }
            InterlockStatus::MissingActuator => {
                "检测到 SIS/ESD 联锁触发测点或逻辑块，但未关联切断阀或停机执行元件".to_string()
            }
            InterlockStatus::OrphanActuator => {
                "检测到紧急切断/泄放执行阀，但未发现同回路或信号线连接的触发信号源".to_string()
            }
        };

        rows.push(PidCauseEffectRow {
            interlock_id: String::new(),
            loop_number,
            status,
            trigger_kind,
            drawing_codes: drawing_set.into_iter().collect(),
            crossed_sheets: tab_set.len(),
            protected_source,
            cause_tags: cause_tags.into_iter().collect(),
            logic_solvers: logic_solvers.into_iter().collect(),
            effect_actions: effect_actions.into_iter().collect(),
            feedback_tags: feedback_tags.into_iter().collect(),
            diagnostic_note,
            primary_tab: anchor.tab_index,
            primary_handle_hex: format!("{:X}", anchor.primary_handle.value()),
            x: anchor.center.0,
            y: anchor.center.1,
            primary_handle: anchor.primary_handle,
            handles_by_tab,
        });
    }

    rows.sort_by(|a, b| {
        a.primary_tab
            .cmp(&b.primary_tab)
            .then_with(|| a.loop_number.cmp(&b.loop_number))
            .then_with(|| a.primary_handle_hex.cmp(&b.primary_handle_hex))
    });

    for (i, row) in rows.iter_mut().enumerate() {
        row.interlock_id = format!("SIS-{:03}", i + 1);
    }

    let multi_sheet_interlocks = rows.iter().filter(|r| r.crossed_sheets > 1).count();
    let complete_count = rows
        .iter()
        .filter(|r| r.status == InterlockStatus::CompleteInterlock)
        .count();
    let missing_actuator_count = rows
        .iter()
        .filter(|r| r.status == InterlockStatus::MissingActuator)
        .count();
    let orphan_actuator_count = rows
        .iter()
        .filter(|r| r.status == InterlockStatus::OrphanActuator)
        .count();
    let total_cause_sensors = rows.iter().map(|r| r.cause_tags.len()).sum();
    let total_trip_actuators = rows.iter().map(|r| r.effect_actions.len()).sum();

    PidCauseEffectReport {
        summary: PidCauseEffectSummary {
            sheets_scanned: sheets.len(),
            total_interlocks: rows.len(),
            multi_sheet_interlocks,
            complete_count,
            missing_actuator_count,
            orphan_actuator_count,
            total_cause_sensors,
            total_trip_actuators,
        },
        interlocks: rows,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidCauseEffectReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_cause_effect_delimited(report: &PidCauseEffectReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = CAUSE_EFFECT_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for r in &report.interlocks {
        let cols = [
            r.interlock_id.clone(),
            r.loop_number.clone(),
            r.status.as_str().to_string(),
            r.trigger_kind.as_str().to_string(),
            r.drawing_codes.join(";"),
            r.protected_source.clone(),
            r.cause_tags.join(";"),
            r.logic_solvers.join(";"),
            r.effect_actions.join(";"),
            r.feedback_tags.join(";"),
            r.crossed_sheets.to_string(),
            r.diagnostic_note.clone(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidCauseEffectReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_cause_effect_file(
    report: &PidCauseEffectReport,
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
            let body = format_pid_cause_effect_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_cause_effect_delimited(report, '\t');
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
/// and build a `PidCauseEffectReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_cause_effect_from_path(
    input: &std::path::Path,
) -> Result<PidCauseEffectReport, String> {
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
    Ok(analyze_pid_cause_effect(&refs))
}

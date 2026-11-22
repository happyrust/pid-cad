//! P&ID Instrument Index & DCS/SIS Hardware I/O Count Schedule (`SH/T 3005` / `HG/T 20507`),
//! `+20%` spare cabinet channel sizing, LY/LV converter de-duplication, process hookup
//! verification, and interactive `PIDIO` / CLI `--export-io` support (`L3-2 / L3-6 / L6-2` / `#173`).

use std::collections::{BTreeMap, BTreeSet};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::cause_effect::{analyze_pid_cause_effect, parse_sis_tag_prefix_and_loop};
use super::class_map::default_legend_class_map;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::{classify_valve_role_and_subtype, ValveCategory};

/// Fixed 12-column CSV/TSV header for `PIDIO EXPORT` and `--export-io`.
pub const IO_LIST_CSV_HEADERS: [&str; 12] = [
    "ItemTag",
    "FunctionPrefix",
    "LoopNumber",
    "DrawingCode",
    "ControlSystem",
    "IoChannel",
    "HardwarePoints",
    "MountStatus",
    "MountedOn",
    "ProcessDN",
    "PipingSpec",
    "SppidClass",
];

/// Control system allocation for an instrument or actuated element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlSystemKind {
    Dcs,
    SisEsd,
    Local,
}

impl ControlSystemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dcs => "dcs",
            Self::SisEsd => "sis_esd",
            Self::Local => "local",
        }
    }
}

/// Hardware I/O card channel type (or non-wired soft/local category).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IoChannelKind {
    #[serde(rename = "AI")]
    Ai,
    #[serde(rename = "AO")]
    Ao,
    #[serde(rename = "DI")]
    Di,
    #[serde(rename = "DO")]
    Do,
    #[serde(rename = "SOFT")]
    SoftFunction,
    #[serde(rename = "LOCAL_NONE")]
    LocalNone,
}

impl IoChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ai => "AI",
            Self::Ao => "AO",
            Self::Di => "DI",
            Self::Do => "DO",
            Self::SoftFunction => "SOFT",
            Self::LocalNone => "LOCAL_NONE",
        }
    }

    pub fn hardware_points(self) -> usize {
        match self {
            Self::Ai | Self::Ao | Self::Di | Self::Do => 1,
            Self::SoftFunction | Self::LocalNone => 0,
        }
    }
}

/// Field mounting & process-tap connectivity status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountStatus {
    Mounted,
    PanelOrSoft,
    UnconnectedField,
}

impl MountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mounted => "mounted",
            Self::PanelOrSoft => "panel_or_soft",
            Self::UnconnectedField => "unconnected_field",
        }
    }
}

/// One instrument or actuated valve row in `PidIoListReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidIoItemRow {
    pub item_tag: String,
    pub function_prefix: String,
    pub loop_number: String,
    pub tab_index: usize,
    pub drawing_code: String,
    pub control_system: ControlSystemKind,
    pub io_channel: IoChannelKind,
    pub hardware_points: usize,
    pub mount_status: MountStatus,
    pub mounted_on: String,
    pub process_dn: String,
    pub piping_spec: String,
    pub sppid_class: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Cabinet hardware I/O summary including `+20%` EPC spare channel sizing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidIoSummary {
    pub sheets_scanned: usize,
    pub total_items: usize,
    pub dcs_ai: usize,
    pub dcs_ao: usize,
    pub dcs_di: usize,
    pub dcs_do: usize,
    pub dcs_total_hw: usize,
    pub dcs_with_20pct_spare: usize,
    pub sis_ai: usize,
    pub sis_ao: usize,
    pub sis_di: usize,
    pub sis_do: usize,
    pub sis_total_hw: usize,
    pub sis_with_20pct_spare: usize,
    pub local_gauges_count: usize,
    pub soft_functions_count: usize,
    pub unconnected_field_count: usize,
}

/// Complete Instrument Index & Hardware I/O Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidIoListReport {
    pub summary: PidIoSummary,
    pub items: Vec<PidIoItemRow>,
}

type Pt2 = (f64, f64);

#[derive(Clone, Copy)]
struct Span2 {
    a: Pt2,
    b: Pt2,
}

struct RawIoCandidate {
    tab_index: usize,
    drawing_code: String,
    tag: String,
    pfx: String,
    loop_num: String,
    sppid_class: String,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: Pt2,
    upm: f64,
}

type PipeSpanEntry = (Span2, String, String, String);

struct SheetContext {
    pipes: Vec<PipeSpanEntry>,
    equips: Vec<(Pt2, String)>,
    signals: Vec<Span2>,
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

fn is_io_relevant_symbol(cls: &str, type_code: &str, desc_up: &str, tag: &str, pfx: &str) -> bool {
    if matches!(
        cls,
        "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
    ) || matches!(
        type_code.trim().to_ascii_uppercase().as_str(),
        "INSTRUMENT" | "INLINE_INSTRUMENT" | "CONTROL_FUNCTION"
    ) {
        return true;
    }
    if matches!(
        pfx,
        "ESDV"
            | "SDV"
            | "XV"
            | "ESV"
            | "BDV"
            | "EDV"
            | "ZV"
            | "EZV"
            | "XZV"
            | "UV"
            | "SOV"
            | "MOV"
            | "POV"
            | "PV"
            | "LV"
            | "FV"
            | "TV"
            | "AV"
    ) {
        return true;
    }
    classify_valve_role_and_subtype(cls, type_code, desc_up, Some(tag))
        .is_some_and(|(cat, _)| cat == ValveCategory::ActuatedControl)
}

fn is_equipment_class(cls: &str, type_code: &str) -> bool {
    matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquipment"
            | "PIDHeatTransferEquip"
            | "PIDProcessEquipment"
    ) || matches!(
        type_code.trim().to_ascii_uppercase().as_str(),
        "VESSEL" | "PUMP" | "TANK" | "EQUIPMENT" | "HEAT_EXCHANGER"
    )
}

fn extract_sheet_io_candidates(
    tab_index: usize,
    fallback_name: &str,
    doc: &Document,
    candidates: &mut Vec<RawIoCandidate>,
    ctx_by_tab: &mut BTreeMap<usize, SheetContext>,
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
    let mut equips: Vec<(Pt2, String)> = Vec::new();
    let mut pipes: Vec<PipeSpanEntry> = Vec::new();

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

            let cls = pmap
                .get("_sppid_class")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&auth.type_code)
                .to_string();
            let center = pmap
                .get("_at")
                .and_then(Value::as_str)
                .and_then(parse_pair_f64)
                .unwrap_or_else(|| entity_center(eff_doc, h));
            let tag = auth.tag.trim();

            if is_equipment_class(&cls, &auth.type_code) {
                let etag = if tag.is_empty() {
                    format!("EQUIP-{:X}", h.value())
                } else {
                    tag.to_string()
                };
                equips.push((center, etag));
                continue;
            }

            let Some((pfx, loop_num)) = parse_sis_tag_prefix_and_loop(tag) else {
                continue;
            };
            if loop_num.is_empty() {
                continue;
            }
            let src_blk = match ent {
                EntityType::Insert(ins) => ins.block_name.as_str(),
                _ => pmap
                    .get("_source_block")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            };
            let desc_up = src_blk.to_ascii_uppercase();
            if !is_io_relevant_symbol(&cls, &auth.type_code, &desc_up, tag, &pfx) {
                continue;
            }

            candidates.push(RawIoCandidate {
                tab_index,
                drawing_code: drawing_code.clone(),
                tag: tag.to_string(),
                pfx,
                loop_num,
                sppid_class: cls,
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

            if is_equipment_class(&sym.sppid_class, "") {
                let etag = sym
                    .tag
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("EQUIP-{raw_h:X}"));
                equips.push((sym.at, etag));
                continue;
            }

            let Some(tag_raw) = sym.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            let Some((pfx, loop_num)) = parse_sis_tag_prefix_and_loop(tag_raw) else {
                continue;
            };
            if loop_num.is_empty() {
                continue;
            }
            let desc_up = format!(
                "{} {} {}",
                sym.source_block,
                sym.layer,
                sym.symbol_path.as_deref().unwrap_or("")
            )
            .to_ascii_uppercase();
            if !is_io_relevant_symbol(&sym.sppid_class, "", &desc_up, tag_raw, &pfx) {
                continue;
            }

            candidates.push(RawIoCandidate {
                tab_index,
                drawing_code: drawing_code.clone(),
                tag: tag_raw.to_string(),
                pfx,
                loop_num,
                sppid_class: sym.sppid_class.clone(),
                primary_handle: primary_h,
                member_handles: members,
                center: sym.at,
                upm,
            });
        }

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
            let (lname, dn, spec) = best_lbl
                .and_then(|(_, t)| parse_sppid_line_number(t.trim().trim_end_matches('-')))
                .map(|p| {
                    (
                        p.pipeline_name(),
                        p.nominal_diameter_attr,
                        p.materials_class,
                    )
                })
                .unwrap_or_default();
            pipes.push((sp, lname, dn, spec));
        }
    }

    let mut signals = Vec::new();
    for ent in eff_doc.model_space_entities() {
        let h = ent.common().handle;
        if sym_handles.contains(&h) {
            continue;
        }
        let auth = PidAuthor::load(eff_doc, h);
        if !is_signal_entity(ent, auth.as_ref()) {
            continue;
        }
        match ent {
            EntityType::Line(ln) => {
                signals.push(Span2 {
                    a: (ln.start.x, ln.start.y),
                    b: (ln.end.x, ln.end.y),
                });
            }
            EntityType::LwPolyline(pl) => {
                for w in pl.vertices.windows(2) {
                    signals.push(Span2 {
                        a: (w[0].location.x, w[0].location.y),
                        b: (w[1].location.x, w[1].location.y),
                    });
                }
            }
            _ => {}
        }
    }

    ctx_by_tab.insert(
        tab_index,
        SheetContext {
            pipes,
            equips,
            signals,
        },
    );
}

fn classify_system_and_channel(
    pfx: &str,
    loop_num: &str,
    sppid_class: &str,
    is_sis: bool,
    has_paired_control_valve: bool,
) -> (ControlSystemKind, IoChannelKind, bool) {
    if matches!(
        pfx,
        "PG" | "TG" | "LG" | "FG" | "PDG" | "LDG" | "FE" | "TE" | "TW" | "RO" | "FO" | "SG"
    ) || (matches!(pfx, "PI" | "TI" | "LI" | "FI") && sppid_class != "PIDControlSystemFunction")
    {
        return (ControlSystemKind::Local, IoChannelKind::LocalNone, false);
    }

    let sys = if is_sis
        || matches!(
            pfx,
            "ESDV" | "SDV" | "XV" | "ESV" | "BDV" | "EDV" | "ESD" | "SIS" | "UZ" | "PZ" | "LZ"
        ) {
        ControlSystemKind::SisEsd
    } else {
        ControlSystemKind::Dcs
    };

    if matches!(pfx, "PY" | "LY" | "FY" | "TY" | "AY") {
        if !loop_num.is_empty() && has_paired_control_valve {
            return (sys, IoChannelKind::SoftFunction, true);
        }
        return (sys, IoChannelKind::Ao, false);
    }

    if matches!(
        pfx,
        "ESDV"
            | "SDV"
            | "XV"
            | "ESV"
            | "BDV"
            | "EDV"
            | "ZV"
            | "EZV"
            | "XZV"
            | "UV"
            | "SOV"
            | "MOV"
            | "POV"
    ) {
        return (sys, IoChannelKind::Do, false);
    }

    if matches!(
        pfx,
        "PV" | "LV" | "FV" | "TV" | "AV" | "PDV" | "LDV" | "FDV" | "TDV" | "HV"
    ) {
        return (ControlSystemKind::Dcs, IoChannelKind::Ao, false);
    }

    if pfx.ends_with('T')
        && pfx
            .chars()
            .next()
            .is_some_and(|c| matches!(c, 'P' | 'L' | 'F' | 'T' | 'A' | 'W' | 'S' | 'V' | 'I' | 'D'))
    {
        let tx_sys = if is_sis || pfx.contains('Z') {
            ControlSystemKind::SisEsd
        } else {
            ControlSystemKind::Dcs
        };
        return (tx_sys, IoChannelKind::Ai, false);
    }

    let is_panel_btn = matches!(
        pfx,
        "HS" | "HSC" | "HSO" | "HSZ" | "ESD" | "ESDB" | "PB" | "UZS"
    );
    let is_switch = is_panel_btn
        || matches!(pfx, "ZSC" | "ZSO" | "ZS" | "ZSH" | "ZSL" | "GDS" | "FD")
        || (pfx
            .chars()
            .next()
            .is_some_and(|c| matches!(c, 'P' | 'L' | 'T' | 'F' | 'A'))
            && (pfx.contains("SH") || pfx.contains("SL") || pfx.contains("ZS")));

    if is_switch {
        return (sys, IoChannelKind::Di, is_panel_btn);
    }

    (sys, IoChannelKind::SoftFunction, true)
}

fn resolve_mount_connection(
    cand: &RawIoCandidate,
    is_panel_or_soft: bool,
    ctx_opt: Option<&SheetContext>,
) -> (MountStatus, String, String, String) {
    if is_panel_or_soft {
        return (
            MountStatus::PanelOrSoft,
            String::new(),
            String::new(),
            String::new(),
        );
    }
    let Some(ctx) = ctx_opt else {
        return (
            MountStatus::UnconnectedField,
            String::new(),
            String::new(),
            String::new(),
        );
    };

    // 1. Direct pipe proximity
    let mut best_pipe: Option<(f64, &PipeSpanEntry)> = None;
    for p in &ctx.pipes {
        let d = point_to_span_dist(cand.center, p.0);
        if d <= 16.0 * cand.upm && best_pipe.is_none_or(|(bd, _)| d < bd) {
            best_pipe = Some((d, p));
        }
    }
    if let Some((_, p)) = best_pipe {
        let m_on = if p.1.is_empty() {
            "PIPE".to_string()
        } else {
            p.1.clone()
        };
        return (MountStatus::Mounted, m_on, p.2.clone(), p.3.clone());
    }

    // 2. Direct equipment proximity
    if let Some((_, etag)) = ctx
        .equips
        .iter()
        .find(|(ec, _)| (ec.0 - cand.center.0).hypot(ec.1 - cand.center.1) <= 22.0 * cand.upm)
    {
        return (
            MountStatus::Mounted,
            etag.clone(),
            String::new(),
            String::new(),
        );
    }

    // 3. Connected impulse/signal line touching equipment or pipe
    let mut visited_sig: BTreeSet<usize> = BTreeSet::new();
    let mut stack: Vec<usize> = Vec::new();
    for (si, &sp) in ctx.signals.iter().enumerate() {
        if point_to_span_dist(cand.center, sp) <= 9.0 * cand.upm {
            visited_sig.insert(si);
            stack.push(si);
        }
    }
    while let Some(cur) = stack.pop() {
        let csp = ctx.signals[cur];
        if let Some((_, etag)) = ctx
            .equips
            .iter()
            .find(|(ec, _)| point_to_span_dist(*ec, csp) <= 22.0 * cand.upm)
        {
            return (
                MountStatus::Mounted,
                etag.clone(),
                String::new(),
                String::new(),
            );
        }
        if let Some(p) = ctx
            .pipes
            .iter()
            .find(|p| spans_touch(csp, p.0, 6.0 * cand.upm))
        {
            let m_on = if p.1.is_empty() {
                "PIPE".to_string()
            } else {
                p.1.clone()
            };
            return (MountStatus::Mounted, m_on, p.2.clone(), p.3.clone());
        }
        for (ni, &nsp) in ctx.signals.iter().enumerate() {
            if visited_sig.len() < 24
                && spans_touch(csp, nsp, 2.5 * cand.upm)
                && visited_sig.insert(ni)
            {
                stack.push(ni);
            }
        }
    }

    (
        MountStatus::UnconnectedField,
        String::new(),
        String::new(),
        String::new(),
    )
}

fn with_20pct_spare(hw: usize) -> usize {
    if hw == 0 {
        0
    } else {
        (hw * 6).div_ceil(5)
    }
}

/// Check whether a `PidIoItemRow` matches a user tag or mounted-process query.
pub fn io_item_matches_query(row: &PidIoItemRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    let q_comp: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let tag_up = row.item_tag.to_ascii_uppercase();
    let tag_comp: String = tag_up
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    tag_up.contains(&q)
        || (!q_comp.is_empty() && tag_comp.contains(&q_comp))
        || row.mounted_on.to_ascii_uppercase().contains(&q)
}

/// Build the Instrument Index & DCS/SIS Hardware I/O Count Schedule across `sheets` (`#173`).
pub fn analyze_pid_io_list(sheets: &[OpcSheetInput<'_>]) -> PidIoListReport {
    let ce_rep = analyze_pid_cause_effect(sheets);
    let mut sis_tags_upper: BTreeSet<String> = BTreeSet::new();
    let mut sis_loops: BTreeSet<String> = BTreeSet::new();
    for il in &ce_rep.interlocks {
        if !il.loop_number.is_empty() {
            sis_loops.insert(il.loop_number.clone());
        }
        for t in il
            .cause_tags
            .iter()
            .chain(il.logic_solvers.iter())
            .chain(il.feedback_tags.iter())
        {
            sis_tags_upper.insert(t.to_ascii_uppercase());
        }
        for ea in &il.effect_actions {
            let base = ea.split('(').next().unwrap_or(ea).trim();
            sis_tags_upper.insert(base.to_ascii_uppercase());
        }
    }

    let mut candidates: Vec<RawIoCandidate> = Vec::new();
    let mut ctx_by_tab: BTreeMap<usize, SheetContext> = BTreeMap::new();
    for sh in sheets {
        extract_sheet_io_candidates(
            sh.tab_index,
            sh.fallback_name,
            sh.document,
            &mut candidates,
            &mut ctx_by_tab,
        );
    }

    let control_valve_loops: BTreeSet<String> = candidates
        .iter()
        .filter(|c| {
            matches!(
                c.pfx.as_str(),
                "PV" | "LV" | "FV" | "TV" | "AV" | "PDV" | "LDV" | "FDV" | "TDV"
            )
        })
        .map(|c| c.loop_num.clone())
        .collect();

    let mut items: Vec<PidIoItemRow> = Vec::new();
    for cand in &candidates {
        let is_sis = sis_tags_upper.contains(&cand.tag.to_ascii_uppercase())
            || sis_loops.contains(&cand.loop_num);
        let has_cv = control_valve_loops.contains(&cand.loop_num);
        let (sys, chan, is_panel_or_soft) = classify_system_and_channel(
            &cand.pfx,
            &cand.loop_num,
            &cand.sppid_class,
            is_sis,
            has_cv,
        );
        let (mount_status, mounted_on, process_dn, piping_spec) =
            resolve_mount_connection(cand, is_panel_or_soft, ctx_by_tab.get(&cand.tab_index));

        items.push(PidIoItemRow {
            item_tag: cand.tag.clone(),
            function_prefix: cand.pfx.clone(),
            loop_number: cand.loop_num.clone(),
            tab_index: cand.tab_index,
            drawing_code: cand.drawing_code.clone(),
            control_system: sys,
            io_channel: chan,
            hardware_points: chan.hardware_points(),
            mount_status,
            mounted_on,
            process_dn,
            piping_spec,
            sppid_class: cand.sppid_class.clone(),
            primary_handle_hex: format!("{:X}", cand.primary_handle.value()),
            x: cand.center.0,
            y: cand.center.1,
            primary_handle: cand.primary_handle,
            member_handles: cand.member_handles.clone(),
        });
    }

    items.sort_by(|a, b| {
        a.tab_index
            .cmp(&b.tab_index)
            .then_with(|| a.loop_number.cmp(&b.loop_number))
            .then_with(|| a.item_tag.cmp(&b.item_tag))
    });

    let count_sc = |s: ControlSystemKind, c: IoChannelKind| {
        items
            .iter()
            .filter(|r| r.control_system == s && r.io_channel == c)
            .count()
    };

    let dcs_ai = count_sc(ControlSystemKind::Dcs, IoChannelKind::Ai);
    let dcs_ao = count_sc(ControlSystemKind::Dcs, IoChannelKind::Ao);
    let dcs_di = count_sc(ControlSystemKind::Dcs, IoChannelKind::Di);
    let dcs_do = count_sc(ControlSystemKind::Dcs, IoChannelKind::Do);
    let dcs_total_hw = dcs_ai + dcs_ao + dcs_di + dcs_do;

    let sis_ai = count_sc(ControlSystemKind::SisEsd, IoChannelKind::Ai);
    let sis_ao = count_sc(ControlSystemKind::SisEsd, IoChannelKind::Ao);
    let sis_di = count_sc(ControlSystemKind::SisEsd, IoChannelKind::Di);
    let sis_do = count_sc(ControlSystemKind::SisEsd, IoChannelKind::Do);
    let sis_total_hw = sis_ai + sis_ao + sis_di + sis_do;

    let local_gauges_count = items
        .iter()
        .filter(|r| r.io_channel == IoChannelKind::LocalNone)
        .count();
    let soft_functions_count = items
        .iter()
        .filter(|r| r.io_channel == IoChannelKind::SoftFunction)
        .count();
    let unconnected_field_count = items
        .iter()
        .filter(|r| r.mount_status == MountStatus::UnconnectedField)
        .count();

    PidIoListReport {
        summary: PidIoSummary {
            sheets_scanned: sheets.len(),
            total_items: items.len(),
            dcs_ai,
            dcs_ao,
            dcs_di,
            dcs_do,
            dcs_total_hw,
            dcs_with_20pct_spare: with_20pct_spare(dcs_total_hw),
            sis_ai,
            sis_ao,
            sis_di,
            sis_do,
            sis_total_hw,
            sis_with_20pct_spare: with_20pct_spare(sis_total_hw),
            local_gauges_count,
            soft_functions_count,
            unconnected_field_count,
        },
        items,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidIoListReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_io_list_delimited(report: &PidIoListReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = IO_LIST_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for r in &report.items {
        let cols = [
            r.item_tag.clone(),
            r.function_prefix.clone(),
            r.loop_number.clone(),
            r.drawing_code.clone(),
            r.control_system.as_str().to_string(),
            r.io_channel.as_str().to_string(),
            r.hardware_points.to_string(),
            r.mount_status.as_str().to_string(),
            r.mounted_on.clone(),
            r.process_dn.clone(),
            r.piping_spec.clone(),
            r.sppid_class.clone(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidIoListReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_io_list_file(
    report: &PidIoListReport,
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
            let body = format_pid_io_list_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_io_list_delimited(report, '\t');
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
/// and build a `PidIoListReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_io_list_from_path(input: &std::path::Path) -> Result<PidIoListReport, String> {
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
    Ok(analyze_pid_io_list(&refs))
}

//! ISA-5.1 Instrument Loop cross-tab aggregator, signal-chain completeness auditor,
//! and Loop Index exporter (`PIDLOOP` & `--export-loops`, `L3-2` / `L3-6` / `L2-2` / `L6-2` / `#156`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

#[cfg(not(target_arch = "wasm32"))]
use super::bom::extract_bom_from_batch_dir;
use super::class_map::default_legend_class_map;
use super::find::match_query_rank;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Canonical 10-column header order shared by CSV and TSV Instrument Loop Index exports.
pub const LOOP_CSV_HEADERS: [&str; 10] = [
    "LoopNumber",
    "MeasuredVariable",
    "LoopStatus",
    "MemberCount",
    "DrawingCodes",
    "Sensors",
    "Controllers",
    "FinalElements",
    "DiagnosticNote",
    "AnchorXY",
];

/// Functional ISA-5.1 role of an instrument or control valve within a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsaLoopRole {
    Sensor,
    Controller,
    FinalElement,
}

/// Signal-chain completeness status of an aggregated ISA-5.1 instrument loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopStatus {
    Incomplete,
    CompleteClosedLoop,
    MonitoringLoop,
}

impl LoopStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::CompleteClosedLoop => "complete_closed_loop",
            Self::MonitoringLoop => "monitoring_loop",
        }
    }
}

/// Decomposed ISA-5.1 tag components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedIsaTag {
    pub raw_tag: String,
    pub func_letters: String,
    pub measured_var: String,
    pub loop_number: String,
    pub suffix: String,
    pub role: IsaLoopRole,
}

/// One instrument or actuated valve belonging to an aggregated loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidLoopMember {
    pub tab_index: usize,
    pub drawing_name: String,
    pub item_tag: String,
    pub func_letters: String,
    pub role: IsaLoopRole,
    pub sppid_class: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Option<Handle>,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// One aggregated ISA-5.1 instrument loop spanning one or more sheets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidInstrumentLoop {
    pub loop_number: String,
    pub measured_variable: String,
    pub status: LoopStatus,
    pub diagnostic_note: String,
    pub crossed_tabs: usize,
    pub drawing_codes: Vec<String>,
    pub sensors: Vec<String>,
    pub controllers: Vec<String>,
    pub final_elements: Vec<String>,
    pub members: Vec<PidLoopMember>,
}

/// Summary statistics across all aggregated instrument loops.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidLoopSummary {
    pub sheets_scanned: usize,
    pub total_instruments_grouped: usize,
    pub total_loops: usize,
    pub multi_sheet_loops: usize,
    pub complete_closed_loops: usize,
    pub monitoring_loops: usize,
    pub incomplete_loops: usize,
}

/// Complete ISA-5.1 Instrument Loop Index report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidLoopReport {
    pub summary: PidLoopSummary,
    pub loops: Vec<PidInstrumentLoop>,
}

fn is_actuated_final_prefix(func: &str) -> bool {
    matches!(
        func,
        "PV" | "FV"
            | "LV"
            | "TV"
            | "AV"
            | "XV"
            | "HV"
            | "BDV"
            | "SDV"
            | "MOV"
            | "GOV"
            | "PCV"
            | "FCV"
            | "LCV"
            | "TCV"
            | "PDV"
            | "PY"
            | "FY"
            | "LY"
            | "TY"
            | "UY"
    )
}

/// Parse an engineering tag into ISA-5.1 `(func_letters, measured_var, loop_number, role)`
/// if `sppid_class` and prefix represent an instrument or actuated control/interlock valve.
pub fn parse_isa_instrument_tag(raw_tag: &str, sppid_class: &str) -> Option<ParsedIsaTag> {
    let trimmed = raw_tag.trim();
    if trimmed.is_empty() {
        return None;
    }
    let up = trimmed.to_ascii_uppercase();
    if up.starts_with("DWG-") || up.starts_with("OPC:") || up.contains('"') {
        return None;
    }

    let func_letters: String = up.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if func_letters.is_empty() || func_letters.len() > 6 {
        return None;
    }

    let is_inst_class = matches!(
        sppid_class,
        "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
    ) || sppid_class.eq_ignore_ascii_case("instrument");
    let is_valve_class =
        sppid_class == "PIDPipingComponent" || sppid_class.eq_ignore_ascii_case("valve");

    if !is_inst_class && !(is_valve_class && is_actuated_final_prefix(&func_letters)) {
        return None;
    }

    let rest = up[func_letters.len()..]
        .trim_start_matches(|c: char| c == '-' || c == '_' || c.is_whitespace());
    if rest.is_empty() {
        return None;
    }

    let chars: Vec<char> = rest.chars().collect();
    let mut num_end = 0usize;
    while num_end < chars.len() {
        let ch = chars[num_end];
        if ch.is_ascii_digit()
            || (ch == '-'
                && num_end > 0
                && chars.get(num_end + 1).is_some_and(|nx| nx.is_ascii_digit()))
        {
            num_end += 1;
        } else {
            break;
        }
    }

    let loop_number: String = chars[..num_end].iter().collect();
    let digit_cnt = loop_number.chars().filter(|c| c.is_ascii_digit()).count();
    if digit_cnt < 2 {
        return None;
    }
    let suffix: String = chars[num_end..]
        .iter()
        .collect::<String>()
        .trim_matches(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .to_string();

    let measured_var = if func_letters.starts_with("PD") && func_letters != "PDV" {
        "PD".to_string()
    } else if func_letters.starts_with("FD") && func_letters != "FDV" {
        "FD".to_string()
    } else if func_letters.starts_with("TD") && func_letters != "TDV" {
        "TD".to_string()
    } else if func_letters.starts_with("LD") && func_letters != "LDV" {
        "LD".to_string()
    } else {
        func_letters[..1].to_string()
    };

    let is_relief = matches!(
        func_letters.as_str(),
        "PSV" | "TSV" | "PRV" | "PSE" | "PVRV"
    );
    let is_final = !is_relief
        && (is_actuated_final_prefix(&func_letters)
            || (sppid_class == "PIDInlineInstrument"
                && (func_letters.ends_with('V') || func_letters.ends_with('Y'))));
    let is_sensor = !is_final
        && (func_letters.ends_with('T')
            || func_letters.ends_with('E')
            || func_letters.ends_with('G')
            || is_relief
            || matches!(
                func_letters.as_str(),
                "PI" | "TI" | "LI" | "FI" | "PDI" | "AI" | "SI" | "WI" | "ZI"
            ));

    let role = if is_final {
        IsaLoopRole::FinalElement
    } else if is_sensor {
        IsaLoopRole::Sensor
    } else {
        IsaLoopRole::Controller
    };

    Some(ParsedIsaTag {
        raw_tag: trimmed.to_string(),
        func_letters,
        measured_var,
        loop_number,
        suffix,
        role,
    })
}

fn parse_csv_handles(raw: &str) -> Vec<Handle> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(parse_handle)
        .collect()
}

fn parse_pair_f64(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn entity_xy(doc: &Document, h: Handle) -> Option<(f64, f64)> {
    match doc.get_entity(h)? {
        EntityType::Insert(ins) => Some((ins.insert_point.x, ins.insert_point.y)),
        EntityType::Circle(c) => Some((c.center.x, c.center.y)),
        EntityType::Line(l) => Some(((l.start.x + l.end.x) * 0.5, (l.start.y + l.end.y) * 0.5)),
        EntityType::LwPolyline(pl) if !pl.vertices.is_empty() => {
            let n = pl.vertices.len() as f64;
            let sx: f64 = pl.vertices.iter().map(|v| v.location.x).sum();
            let sy: f64 = pl.vertices.iter().map(|v| v.location.y).sum();
            Some((sx / n, sy / n))
        }
        EntityType::Text(t) => Some((t.insertion_point.x, t.insertion_point.y)),
        EntityType::MText(mt) => Some((mt.insertion_point.x, mt.insertion_point.y)),
        _ => None,
    }
}

fn collect_authored_loop_members(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    out: &mut Vec<(ParsedIsaTag, PidLoopMember)>,
) {
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT
            || auth.type_code == TYPE_PIPE_SEGMENT
            || auth.tag.trim().is_empty()
        {
            continue;
        }
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();
        let cls = pmap
            .get("_sppid_class")
            .and_then(Value::as_str)
            .unwrap_or(&auth.type_code);
        let Some(parsed) = parse_isa_instrument_tag(&auth.tag, cls) else {
            continue;
        };
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
        let center = pmap
            .get("_at")
            .and_then(Value::as_str)
            .and_then(parse_pair_f64)
            .or_else(|| entity_xy(doc, h))
            .unwrap_or((0.0, 0.0));
        let mem = PidLoopMember {
            tab_index,
            drawing_name: drawing_name.to_string(),
            item_tag: parsed.raw_tag.clone(),
            func_letters: parsed.func_letters.clone(),
            role: parsed.role,
            sppid_class: cls.to_string(),
            primary_handle_hex: format!("{:X}", h.value()),
            x: center.0,
            y: center.1,
            primary_handle: Some(h),
            member_handles: members,
        };
        out.push((parsed, mem));
    }
}

fn collect_unpromoted_loop_members(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    out: &mut Vec<(ParsedIsaTag, PidLoopMember)>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        collect_authored_loop_members(tab_index, drawing_name, &scratch, out);
        return;
    }

    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );
    for sym in &rec.symbols {
        let Some(tag_str) = sym.tag.as_deref() else {
            continue;
        };
        let Some(parsed) = parse_isa_instrument_tag(tag_str, &sym.sppid_class) else {
            continue;
        };
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
        let mem = PidLoopMember {
            tab_index,
            drawing_name: drawing_name.to_string(),
            item_tag: parsed.raw_tag.clone(),
            func_letters: parsed.func_letters.clone(),
            role: parsed.role,
            sppid_class: sym.sppid_class.clone(),
            primary_handle_hex: format!("{:X}", primary_h.value()),
            x: sym.at.0,
            y: sym.at.1,
            primary_handle: Some(primary_h),
            member_handles: members,
        };
        out.push((parsed, mem));
    }
}

fn push_unique(vec: &mut Vec<String>, s: &str) {
    if !s.is_empty() && !vec.iter().any(|x| x == s) {
        vec.push(s.to_string());
    }
}

fn finalize_loop_groups(
    sheets_scanned: usize,
    entries: Vec<(ParsedIsaTag, PidLoopMember)>,
) -> PidLoopReport {
    let total_instruments_grouped = entries.len();
    let mut by_loop: BTreeMap<String, Vec<(ParsedIsaTag, PidLoopMember)>> = BTreeMap::new();
    for pair in entries {
        by_loop
            .entry(pair.0.loop_number.clone())
            .or_default()
            .push(pair);
    }

    let mut loops = Vec::new();
    for (loop_num, mut group) in by_loop {
        group.sort_by(|a, b| {
            (a.1.role, a.1.tab_index, &a.1.item_tag).cmp(&(b.1.role, b.1.tab_index, &b.1.item_tag))
        });

        let mut measured_variable = String::new();
        for (ptag, _) in &group {
            if !matches!(ptag.measured_var.as_str(), "X" | "U" | "H") {
                measured_variable = ptag.measured_var.clone();
                break;
            }
        }
        if measured_variable.is_empty() {
            measured_variable = group
                .first()
                .map(|(ptag, _)| ptag.measured_var.clone())
                .unwrap_or_default();
        }

        let mut tabs_set = BTreeSet::new();
        let mut drawing_codes = Vec::new();
        let mut sensors = Vec::new();
        let mut controllers = Vec::new();
        let mut final_elements = Vec::new();
        let mut members = Vec::new();

        let mut needs_sensor_ctrl = false;
        let mut needs_sensor_valve = false;

        for (ptag, mem) in group {
            tabs_set.insert(mem.tab_index);
            push_unique(&mut drawing_codes, &mem.drawing_name);
            match ptag.role {
                IsaLoopRole::Sensor => push_unique(&mut sensors, &mem.item_tag),
                IsaLoopRole::Controller => {
                    if ptag.func_letters.contains('C') && !ptag.func_letters.starts_with('H') {
                        needs_sensor_ctrl = true;
                    }
                    push_unique(&mut controllers, &mem.item_tag);
                }
                IsaLoopRole::FinalElement => {
                    if matches!(
                        ptag.func_letters.as_str(),
                        "PV" | "FV" | "LV" | "TV" | "AV" | "PDV"
                    ) {
                        needs_sensor_valve = true;
                    }
                    push_unique(&mut final_elements, &mem.item_tag);
                }
            }
            members.push(mem);
        }

        let has_sensor = !sensors.is_empty();
        let has_final = !final_elements.is_empty();
        let (status, diagnostic_note) = if (needs_sensor_ctrl || needs_sensor_valve) && !has_sensor
        {
            (
                LoopStatus::Incomplete,
                "控制回路缺少一次测量/变送元件（Sensor）".to_string(),
            )
        } else if has_sensor && has_final {
            (
                LoopStatus::CompleteClosedLoop,
                "检测-控制/联锁-最终执行闭环完整".to_string(),
            )
        } else {
            (
                LoopStatus::MonitoringLoop,
                "测量指示/开关监测回路正常".to_string(),
            )
        };

        loops.push(PidInstrumentLoop {
            loop_number: loop_num,
            measured_variable,
            status,
            diagnostic_note,
            crossed_tabs: tabs_set.len(),
            drawing_codes,
            sensors,
            controllers,
            final_elements,
            members,
        });
    }

    loops.sort_by(|a, b| (a.status, &a.loop_number).cmp(&(b.status, &b.loop_number)));

    let multi_sheet_loops = loops.iter().filter(|l| l.crossed_tabs > 1).count();
    let complete_closed_loops = loops
        .iter()
        .filter(|l| l.status == LoopStatus::CompleteClosedLoop)
        .count();
    let monitoring_loops = loops
        .iter()
        .filter(|l| l.status == LoopStatus::MonitoringLoop)
        .count();
    let incomplete_loops = loops
        .iter()
        .filter(|l| l.status == LoopStatus::Incomplete)
        .count();

    PidLoopReport {
        summary: PidLoopSummary {
            sheets_scanned,
            total_instruments_grouped,
            total_loops: loops.len(),
            multi_sheet_loops,
            complete_closed_loops,
            monitoring_loops,
            incomplete_loops,
        },
        loops,
    }
}

/// Analyze and aggregate ISA-5.1 instrument loops across open `sheets` (`#156`).
pub fn analyze_pid_loops(sheets: &[OpcSheetInput<'_>]) -> PidLoopReport {
    let mut entries = Vec::new();
    for sh in sheets {
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            collect_authored_loop_members(
                sh.tab_index,
                sh.fallback_name,
                sh.document,
                &mut entries,
            );
        } else {
            collect_unpromoted_loop_members(
                sh.tab_index,
                sh.fallback_name,
                sh.document,
                &mut entries,
            );
        }
    }
    finalize_loop_groups(sheets.len(), entries)
}

/// Match whether `lp` corresponds to user `query` (by loop number or member instrument tag).
pub fn loop_matches_query(lp: &PidInstrumentLoop, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if parse_isa_instrument_tag(q, "PIDInstrument")
        .is_some_and(|p| lp.loop_number.eq_ignore_ascii_case(&p.loop_number))
    {
        return true;
    }
    let q_up = q.to_ascii_uppercase();
    if lp.loop_number.eq_ignore_ascii_case(&q_up) || lp.loop_number.contains(&q_up) {
        return true;
    }
    lp.members
        .iter()
        .any(|m| match_query_rank(&m.item_tag, q).is_some())
}

/// Extract an ISA-5.1 Instrument Loop Index from `_Data.xml` files in `out_dir` (`#156`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_loops_from_batch_dir(out_dir: &Path) -> Result<PidLoopReport, String> {
    let bom = extract_bom_from_batch_dir(out_dir)?;
    let mut entries = Vec::new();
    for row in &bom.rows {
        let Some(parsed) = parse_isa_instrument_tag(&row.item_tag, &row.sppid_class) else {
            continue;
        };
        for (di, dwg) in row.drawing_codes.iter().enumerate() {
            let mem = PidLoopMember {
                tab_index: di,
                drawing_name: dwg.clone(),
                item_tag: parsed.raw_tag.clone(),
                func_letters: parsed.func_letters.clone(),
                role: parsed.role,
                sppid_class: row.sppid_class.clone(),
                primary_handle_hex: String::new(),
                x: 0.0,
                y: 0.0,
                primary_handle: None,
                member_handles: Vec::new(),
            };
            entries.push((parsed.clone(), mem));
        }
    }
    Ok(finalize_loop_groups(bom.summary.sheets_scanned, entries))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidLoopReport` as delimited Loop Index text (CSV or TSV) with UTF-8 BOM.
pub fn format_pid_loops_delimited(report: &PidLoopReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = LOOP_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for lp in &report.loops {
        let anchor = lp
            .members
            .first()
            .map(|m| format!("{:.1},{:.1}", m.x, m.y))
            .unwrap_or_default();
        let cells = [
            lp.loop_number.as_str(),
            lp.measured_variable.as_str(),
            lp.status.as_str(),
            &lp.members.len().to_string(),
            &lp.drawing_codes.join(";"),
            &lp.sensors.join(";"),
            &lp.controllers.join(";"),
            &lp.final_elements.join(";"),
            lp.diagnostic_note.as_str(),
            anchor.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidLoopReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#156`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_loops_file(report: &PidLoopReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize loops JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_loops_delimited(report, '\t'),
        "csv" => format_pid_loops_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported loop index extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

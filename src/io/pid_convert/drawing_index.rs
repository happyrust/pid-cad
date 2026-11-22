//! Multi-sheet P&ID Master Drawing Index extractor, title-block document-control auditor
//! (`ok` / `missing_drawing_number` / `duplicate_drawing_number`), and cross-tab jump resolver
//! (`PIDINDEX` & `--export-index`, `L3-6` / `L4-6` / `L6-1` / `L6-2` / `#165`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::entities::EntityType;
use document::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::valve_mto::classify_valve_role_and_subtype;

/// Canonical 11-column header order shared by CSV and TSV Master Drawing Index exports.
pub const DRAWING_INDEX_CSV_HEADERS: [&str; 11] = [
    "SeqNo",
    "FileLabel",
    "DrawingNumber",
    "DrawingTitle",
    "Revision",
    "DocControlStatus",
    "EquipmentCount",
    "ValveCount",
    "InstrumentCount",
    "PipelineCount",
    "OpcCount",
];

/// Document-control audit status for a sheet in the Master Drawing Index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingIndexStatus {
    Ok,
    MissingDrawingNumber,
    DuplicateDrawingNumber,
}

impl DrawingIndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::MissingDrawingNumber => "missing_drawing_number",
            Self::DuplicateDrawingNumber => "duplicate_drawing_number",
        }
    }
}

/// One sheet entry in `PidDrawingIndexReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidDrawingIndexRow {
    pub seq_no: usize,
    pub tab_index: usize,
    pub file_label: String,
    pub drawing_number: String,
    pub drawing_title: String,
    pub revision: String,
    pub status: DrawingIndexStatus,
    pub equipment_count: usize,
    pub valve_count: usize,
    pub instrument_count: usize,
    pub pipeline_count: usize,
    pub opc_count: usize,
}

/// Aggregate summary statistics across all indexed drawings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidDrawingIndexSummary {
    pub total_sheets: usize,
    pub ok_sheets: usize,
    pub missing_number_sheets: usize,
    pub duplicate_number_sheets: usize,
    pub total_equipment: usize,
    pub total_valves: usize,
    pub total_instruments: usize,
    pub total_pipelines: usize,
    pub total_opcs: usize,
}

/// Complete `PIDINDEX` Master Drawing Index report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidDrawingIndexReport {
    pub summary: PidDrawingIndexSummary,
    pub sheets: Vec<PidDrawingIndexRow>,
}

struct SheetCounts {
    equipment: usize,
    valves: usize,
    instruments: usize,
    pipelines: usize,
    opcs: usize,
}

fn norm_dwg_key(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn count_authored_sheet(doc: &Document) -> SheetCounts {
    let mut equipment = 0usize;
    let mut valves = 0usize;
    let mut instruments = 0usize;
    let mut opcs = 0usize;
    let mut pipe_lines: BTreeSet<String> = BTreeSet::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            if let Some(lbl) = pmap
                .get("_line_label")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                if let Some(parsed) = parse_sppid_line_number(lbl) {
                    pipe_lines.insert(parsed.pipeline_name());
                } else {
                    pipe_lines.insert(lbl.trim().trim_end_matches('-').to_string());
                }
            }
            continue;
        }

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
        let tag_opt = (!auth.tag.trim().is_empty()).then_some(auth.tag.as_str());

        if cls == "PIDBranchPoint" || auth.type_code.eq_ignore_ascii_case("opc") {
            opcs += 1;
        } else if matches!(
            cls,
            "PIDProcessVessel"
                | "PIDMechanicalEquipment"
                | "PIDHeatTransferEquipment"
                | "PIDHeatTransferEquip"
                | "PIDProcessEquipment"
        ) || auth.type_code.eq_ignore_ascii_case("equipment")
        {
            equipment += 1;
        } else if classify_valve_role_and_subtype(cls, &auth.type_code, &blk_upper, tag_opt)
            .is_some()
        {
            valves += 1;
        } else if matches!(
            cls,
            "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
        ) || auth.type_code.eq_ignore_ascii_case("instrument")
        {
            instruments += 1;
        }
    }

    let topo = TopologyTable::load(doc);
    for pl in &topo.pipe_lines {
        if !pl.tag.trim().is_empty() {
            pipe_lines.insert(pl.tag.trim().trim_end_matches('-').to_string());
        }
    }

    SheetCounts {
        equipment,
        valves,
        instruments,
        pipelines: pipe_lines.len(),
        opcs,
    }
}

/// Extract title-block metadata, per-sheet engineering object counts, and document-control
/// status (`ok` / `missing_drawing_number` / `duplicate_drawing_number`) across `sheets` (`#165`).
pub fn analyze_pid_drawing_index(sheets: &[OpcSheetInput<'_>]) -> PidDrawingIndexReport {
    struct RawEntry {
        tab_index: usize,
        file_label: String,
        drawing_number: String,
        drawing_title: String,
        revision: String,
        counts: SheetCounts,
    }

    let mut raw_rows: Vec<RawEntry> = Vec::with_capacity(sheets.len());
    let mut num_freq: BTreeMap<String, usize> = BTreeMap::new();

    for sh in sheets {
        let rec = recognise_sppid_sheet(
            sh.document,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        let dwg_num = rec.title_block.drawing_number.unwrap_or_default();
        let dwg_title = rec.title_block.drawing_title.unwrap_or_default();
        let rev = rec.title_block.revision.unwrap_or_default();

        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());

        let counts = if has_author {
            count_authored_sheet(sh.document)
        } else if has_native_pid_semantics(sh.document) {
            let mut scratch = sh.document.clone();
            let _ = promote_semantics_to_author(&mut scratch, None);
            count_authored_sheet(&scratch)
        } else {
            let mut equipment = 0usize;
            let mut valves = 0usize;
            let mut instruments = 0usize;
            let mut opcs = 0usize;
            for sym in &rec.symbols {
                let cls = sym.sppid_class.as_str();
                let blk_up = sym.source_block.to_ascii_uppercase();
                let tag_opt = sym.tag.as_deref();
                if cls == "PIDBranchPoint" {
                    opcs += 1;
                } else if matches!(
                    cls,
                    "PIDProcessVessel"
                        | "PIDMechanicalEquipment"
                        | "PIDHeatTransferEquipment"
                        | "PIDHeatTransferEquip"
                        | "PIDProcessEquipment"
                ) {
                    equipment += 1;
                } else if classify_valve_role_and_subtype(cls, "", &blk_up, tag_opt).is_some() {
                    valves += 1;
                } else if matches!(
                    cls,
                    "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction"
                ) {
                    instruments += 1;
                }
            }
            let mut pipe_lines: BTreeSet<String> = BTreeSet::new();
            for lbl in &rec.pipe_labels {
                if let Some(parsed) = parse_sppid_line_number(&lbl.raw_text) {
                    pipe_lines.insert(parsed.pipeline_name());
                } else {
                    pipe_lines.insert(lbl.raw_text.trim().trim_end_matches('-').to_string());
                }
            }
            SheetCounts {
                equipment,
                valves,
                instruments,
                pipelines: pipe_lines.len(),
                opcs,
            }
        };

        let nk = norm_dwg_key(&dwg_num);
        if !nk.is_empty() {
            *num_freq.entry(nk).or_default() += 1;
        }

        raw_rows.push(RawEntry {
            tab_index: sh.tab_index,
            file_label: sh.fallback_name.to_string(),
            drawing_number: dwg_num,
            drawing_title: dwg_title,
            revision: rev,
            counts,
        });
    }

    let mut out_rows: Vec<PidDrawingIndexRow> = Vec::with_capacity(raw_rows.len());
    let mut ok_sheets = 0usize;
    let mut missing_number_sheets = 0usize;
    let mut duplicate_number_sheets = 0usize;
    let mut total_equipment = 0usize;
    let mut total_valves = 0usize;
    let mut total_instruments = 0usize;
    let mut total_pipelines = 0usize;
    let mut total_opcs = 0usize;

    for (idx, r) in raw_rows.into_iter().enumerate() {
        let nk = norm_dwg_key(&r.drawing_number);
        let status = if nk.is_empty() {
            missing_number_sheets += 1;
            DrawingIndexStatus::MissingDrawingNumber
        } else if num_freq.get(&nk).copied().unwrap_or(0) > 1 {
            duplicate_number_sheets += 1;
            DrawingIndexStatus::DuplicateDrawingNumber
        } else {
            ok_sheets += 1;
            DrawingIndexStatus::Ok
        };

        total_equipment += r.counts.equipment;
        total_valves += r.counts.valves;
        total_instruments += r.counts.instruments;
        total_pipelines += r.counts.pipelines;
        total_opcs += r.counts.opcs;

        out_rows.push(PidDrawingIndexRow {
            seq_no: idx + 1,
            tab_index: r.tab_index,
            file_label: r.file_label,
            drawing_number: r.drawing_number,
            drawing_title: r.drawing_title,
            revision: r.revision,
            status,
            equipment_count: r.counts.equipment,
            valve_count: r.counts.valves,
            instrument_count: r.counts.instruments,
            pipeline_count: r.counts.pipelines,
            opc_count: r.counts.opcs,
        });
    }

    PidDrawingIndexReport {
        summary: PidDrawingIndexSummary {
            total_sheets: out_rows.len(),
            ok_sheets,
            missing_number_sheets,
            duplicate_number_sheets,
            total_equipment,
            total_valves,
            total_instruments,
            total_pipelines,
            total_opcs,
        },
        sheets: out_rows,
    }
}

/// Match whether `row` matches user `query` (by sequence number, drawing number, title, or file name).
pub fn drawing_index_matches_query(row: &PidDrawingIndexRow, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    if q.parse::<usize>().is_ok_and(|n| n == row.seq_no) {
        return true;
    }
    let q_up = q.to_uppercase();
    row.drawing_number.to_uppercase().contains(&q_up)
        || row.drawing_title.to_uppercase().contains(&q_up)
        || row.file_label.to_uppercase().contains(&q_up)
}

/// Load a single CAD/PID file or directory of CAD/PID sheets and build a Master Drawing Index (`#165`).
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_drawing_index_from_path(input_path: &Path) -> Result<PidDrawingIndexReport, String> {
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
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sheet.dxf")
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
    Ok(analyze_pid_drawing_index(&inputs))
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidDrawingIndexReport` as delimited Master Drawing Index text with UTF-8 BOM.
pub fn format_pid_drawing_index_delimited(report: &PidDrawingIndexReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = DRAWING_INDEX_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for r in &report.sheets {
        let seq = r.seq_no.to_string();
        let eq = r.equipment_count.to_string();
        let vv = r.valve_count.to_string();
        let ins = r.instrument_count.to_string();
        let pl = r.pipeline_count.to_string();
        let opc = r.opc_count.to_string();
        let cells = [
            seq.as_str(),
            r.file_label.as_str(),
            r.drawing_number.as_str(),
            r.drawing_title.as_str(),
            r.revision.as_str(),
            r.status.as_str(),
            eq.as_str(),
            vv.as_str(),
            ins.as_str(),
            pl.as_str(),
            opc.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidDrawingIndexReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#165`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_drawing_index_file(
    report: &PidDrawingIndexReport,
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
                .map_err(|e| format!("serialize drawing index JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_drawing_index_delimited(report, '\t'),
        "csv" => format_pid_drawing_index_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported drawing index extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

//! P&ID Piping Insulation & Heat-Tracing Schedule (`SH/T 3022` / `GB/T 8175`),
//! category & thickness decomposition (`H40 / EH50 / C40 / ST / PP / IH`), MTO rollup
//! with removable valve insulation box counts, run-consistency auditing, and interactive
//! `PIDINSUL` / CLI `--export-insulation` support (`L3-3 / L3-6 / L6-2` / `#176`).

use std::collections::{BTreeMap, BTreeSet};

use document::{Document, Handle};
use serde::{Deserialize, Serialize};

use super::line_list::analyze_pid_line_list;
use super::opc::OpcSheetInput;

/// Fixed 13-column CSV/TSV header for `PIDINSUL EXPORT` and `--export-insulation`.
pub const INSULATION_CSV_HEADERS: [&str; 13] = [
    "LineNumber",
    "NominalDiameter",
    "FluidCode",
    "PipingSpec",
    "RawInsulation",
    "Category",
    "CodePrefix",
    "ThicknessMm",
    "HeatTraced",
    "AuditStatus",
    "DrawingCodes",
    "InlineValveBoxes",
    "SegmentCount",
];

/// Standard EPC thermal insulation / heat-tracing category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsulationCategory {
    HotInsulation,
    ColdInsulation,
    ElectricHeatTracing,
    SteamOrFluidTracing,
    PersonnelProtection,
    AcousticOrJacket,
}

impl InsulationCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HotInsulation => "hot_insulation",
            Self::ColdInsulation => "cold_insulation",
            Self::ElectricHeatTracing => "electric_heat_tracing",
            Self::SteamOrFluidTracing => "steam_or_fluid_tracing",
            Self::PersonnelProtection => "personnel_protection",
            Self::AcousticOrJacket => "acoustic_or_jacket",
        }
    }
}

/// Thickness & cross-segment consistency audit status for an insulated pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsulationAuditStatus {
    OkWithThickness,
    MissingThickness,
    InconsistentAcrossRun,
}

impl InsulationAuditStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OkWithThickness => "ok_with_thickness",
            Self::MissingThickness => "missing_thickness",
            Self::InconsistentAcrossRun => "inconsistent_across_run",
        }
    }

    pub fn is_anomaly(self) -> bool {
        !matches!(self, Self::OkWithThickness)
    }
}

/// Decomposed thermal insulation / tracing code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedInsulationSpec {
    pub raw_code: String,
    pub code_prefix: String,
    pub thickness_mm: Option<u32>,
    pub category: InsulationCategory,
    pub is_heat_traced: bool,
}

/// One insulated or heat-traced pipeline entry in `PidInsulationReport`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidInsulatedLineRow {
    pub line_number: String,
    pub nominal_diameter: String,
    pub fluid_code: String,
    pub seq_no: String,
    pub piping_spec: String,
    pub raw_insulation: String,
    pub observed_codes: Vec<String>,
    pub category: InsulationCategory,
    pub code_prefix: String,
    pub thickness_mm: Option<u32>,
    pub is_heat_traced: bool,
    pub status: InsulationAuditStatus,
    pub drawing_codes: Vec<String>,
    pub inline_valve_boxes: usize,
    pub segment_count: usize,
    pub primary_tab: usize,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Aggregated MTO procurement rollup row grouped by `(category, code_prefix, nominal_diameter, thickness_mm)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidInsulationRollupRow {
    pub category: InsulationCategory,
    pub code_prefix: String,
    pub nominal_diameter: String,
    pub thickness_mm: Option<u32>,
    pub is_heat_traced: bool,
    pub pipeline_count: usize,
    pub segment_count: usize,
    pub valve_boxes_count: usize,
}

/// Summary statistics across all scanned and insulated pipelines.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidInsulationSummary {
    pub sheets_scanned: usize,
    pub total_pipelines_scanned: usize,
    pub insulated_pipelines_count: usize,
    pub uninsulated_pipelines_count: usize,
    pub heat_traced_pipelines_count: usize,
    pub hot_insulation_count: usize,
    pub cold_insulation_count: usize,
    pub electric_tracing_count: usize,
    pub steam_fluid_tracing_count: usize,
    pub personnel_protection_count: usize,
    pub acoustic_jacket_count: usize,
    pub missing_thickness_count: usize,
    pub inconsistent_run_count: usize,
    pub total_insulated_valve_boxes: usize,
    pub mto_groups_count: usize,
}

/// Complete `PIDINSUL` Insulation & Heat-Tracing Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidInsulationReport {
    pub summary: PidInsulationSummary,
    pub mto_rollup: Vec<PidInsulationRollupRow>,
    pub insulated_lines: Vec<PidInsulatedLineRow>,
}

/// Decompose a P&ID line-number insulation suffix (e.g. `EH50`, `H40`, `C40`, `PP25`, `ST-30`, `IH`).
pub fn parse_insulation_spec(raw: &str) -> Option<ParsedInsulationSpec> {
    let trimmed = raw.trim().trim_matches(['-', '_', ' ']);
    if trimmed.is_empty() {
        return None;
    }
    let up = trimmed.to_ascii_uppercase();
    if matches!(up.as_str(), "N" | "NI" | "NONE" | "NO") {
        return None;
    }

    let mut code_prefix = String::new();
    let mut digits = String::new();
    let mut in_digits = false;

    for ch in up.chars() {
        if !in_digits && ch.is_ascii_alphabetic() {
            code_prefix.push(ch);
        } else if ch.is_ascii_digit() {
            in_digits = true;
            digits.push(ch);
        } else if in_digits {
            break;
        }
    }

    if code_prefix.is_empty() {
        return None;
    }

    let thickness_mm = (!digits.is_empty())
        .then(|| digits.parse::<u32>().ok())
        .flatten();

    let (category, is_heat_traced) = match code_prefix.as_str() {
        "EH" | "ET" | "EHT" | "E" => (InsulationCategory::ElectricHeatTracing, true),
        "ST" | "S" | "WT" | "OT" | "HT" | "MT" | "LT" => {
            (InsulationCategory::SteamOrFluidTracing, true)
        }
        "C" | "IC" | "CC" | "CS" => (InsulationCategory::ColdInsulation, false),
        "PP" | "P" | "FP" => (InsulationCategory::PersonnelProtection, false),
        "A" | "AN" | "AC" | "J" | "SJ" | "WJ" => (
            InsulationCategory::AcousticOrJacket,
            matches!(code_prefix.as_str(), "J" | "SJ" | "WJ"),
        ),
        _ => (InsulationCategory::HotInsulation, false),
    };

    Some(ParsedInsulationSpec {
        raw_code: trimmed.to_string(),
        code_prefix,
        thickness_mm,
        category,
        is_heat_traced,
    })
}

/// Check whether a `PidInsulatedLineRow` matches a user insulation code or line query.
pub fn insulated_line_matches_query(row: &PidInsulatedLineRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    row.raw_insulation.to_ascii_uppercase().contains(&q)
        || row.code_prefix.eq_ignore_ascii_case(&q)
        || row.line_number.to_ascii_uppercase().contains(&q)
        || row.seq_no.to_ascii_uppercase().contains(&q)
}

type RollupKey = (InsulationCategory, String, String, Option<u32>, bool);

/// Build the Piping Insulation & Heat-Tracing Schedule across `sheets`, rolling up MTO
/// quantities and auditing missing thicknesses or cross-segment mismatches (`#176`).
pub fn analyze_pid_insulation(sheets: &[OpcSheetInput<'_>]) -> PidInsulationReport {
    let line_rep = analyze_pid_line_list(sheets);

    // Group `PidPipelineRow`s by core 4-part identity `(dn, fluid, seq_no, spec)` when `seq_no` is non-empty
    // so cross-sheet continuations where one sheet omitted `-H40` merge and flag `InconsistentAcrossRun`.
    let mut core_groups: BTreeMap<String, Vec<&super::line_list::PidPipelineRow>> = BTreeMap::new();
    for pl in &line_rep.pipelines {
        let key = if pl.seq_no.is_empty() {
            format!("RAW:{}", pl.line_number)
        } else {
            format!(
                "SEQ:{}:{}:{}:{}",
                pl.nominal_diameter, pl.fluid_code, pl.seq_no, pl.piping_spec
            )
        };
        core_groups.entry(key).or_default().push(pl);
    }

    let total_pipelines_scanned = core_groups.len();
    let mut insulated_lines: Vec<PidInsulatedLineRow> = Vec::new();
    let mut rollup_map: BTreeMap<RollupKey, (usize, usize, usize)> = BTreeMap::new();

    for group in core_groups.values() {
        let mut observed_codes: BTreeSet<String> = BTreeSet::new();
        let mut best_insul_row: Option<(&super::line_list::PidPipelineRow, ParsedInsulationSpec)> =
            None;

        for &pl in group {
            let clean_ins = pl.insulation.trim().to_string();
            observed_codes.insert(clean_ins.clone());
            if let Some(parsed) = parse_insulation_spec(&clean_ins) {
                let replace = match best_insul_row {
                    None => true,
                    Some((_, ref cur_p)) => {
                        cur_p.thickness_mm.is_none() && parsed.thickness_mm.is_some()
                    }
                };
                if replace {
                    best_insul_row = Some((pl, parsed));
                }
            }
        }

        let Some((chosen_pl, spec)) = best_insul_row else {
            continue;
        };

        let mut drawing_set: BTreeSet<String> = BTreeSet::new();
        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
        let mut inline_valve_boxes = 0usize;
        let mut segment_count = 0usize;

        for &pl in group {
            for dc in &pl.drawing_codes {
                drawing_set.insert(dc.clone());
            }
            inline_valve_boxes += pl.inline_valves_count;
            segment_count += pl.segment_count;
            for (&ti, hs) in &pl.handles_by_tab {
                let dst = handles_by_tab.entry(ti).or_default();
                for &h in hs {
                    if !dst.contains(&h) {
                        dst.push(h);
                    }
                }
            }
        }

        let status = if observed_codes.len() > 1 {
            InsulationAuditStatus::InconsistentAcrossRun
        } else if spec.thickness_mm.is_none()
            && spec.category != InsulationCategory::AcousticOrJacket
        {
            InsulationAuditStatus::MissingThickness
        } else {
            InsulationAuditStatus::OkWithThickness
        };

        let rkey: RollupKey = (
            spec.category,
            spec.code_prefix.clone(),
            chosen_pl.nominal_diameter.clone(),
            spec.thickness_mm,
            spec.is_heat_traced,
        );
        let entry = rollup_map.entry(rkey).or_default();
        entry.0 += 1;
        entry.1 += segment_count;
        entry.2 += inline_valve_boxes;

        insulated_lines.push(PidInsulatedLineRow {
            line_number: chosen_pl.line_number.clone(),
            nominal_diameter: chosen_pl.nominal_diameter.clone(),
            fluid_code: chosen_pl.fluid_code.clone(),
            seq_no: chosen_pl.seq_no.clone(),
            piping_spec: chosen_pl.piping_spec.clone(),
            raw_insulation: spec.raw_code,
            observed_codes: observed_codes.into_iter().collect(),
            category: spec.category,
            code_prefix: spec.code_prefix,
            thickness_mm: spec.thickness_mm,
            is_heat_traced: spec.is_heat_traced,
            status,
            drawing_codes: drawing_set.into_iter().collect(),
            inline_valve_boxes,
            segment_count,
            primary_tab: chosen_pl.primary_tab,
            primary_handle_hex: format!("{:X}", chosen_pl.primary_handle.value()),
            x: chosen_pl.x,
            y: chosen_pl.y,
            primary_handle: chosen_pl.primary_handle,
            handles_by_tab,
        });
    }

    insulated_lines.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.line_number.cmp(&b.line_number))
    });

    let mto_rollup: Vec<PidInsulationRollupRow> = rollup_map
        .into_iter()
        .map(
            |(
                (category, code_prefix, nominal_diameter, thickness_mm, is_heat_traced),
                (p, s, v),
            )| {
                PidInsulationRollupRow {
                    category,
                    code_prefix,
                    nominal_diameter,
                    thickness_mm,
                    is_heat_traced,
                    pipeline_count: p,
                    segment_count: s,
                    valve_boxes_count: v,
                }
            },
        )
        .collect();

    let insulated_pipelines_count = insulated_lines.len();
    let uninsulated_pipelines_count =
        total_pipelines_scanned.saturating_sub(insulated_pipelines_count);
    let heat_traced_pipelines_count = insulated_lines.iter().filter(|r| r.is_heat_traced).count();
    let count_cat =
        |c: InsulationCategory| insulated_lines.iter().filter(|r| r.category == c).count();
    let hot_insulation_count = count_cat(InsulationCategory::HotInsulation);
    let cold_insulation_count = count_cat(InsulationCategory::ColdInsulation);
    let electric_tracing_count = count_cat(InsulationCategory::ElectricHeatTracing);
    let steam_fluid_tracing_count = count_cat(InsulationCategory::SteamOrFluidTracing);
    let personnel_protection_count = count_cat(InsulationCategory::PersonnelProtection);
    let acoustic_jacket_count = count_cat(InsulationCategory::AcousticOrJacket);
    let missing_thickness_count = insulated_lines
        .iter()
        .filter(|r| r.status == InsulationAuditStatus::MissingThickness)
        .count();
    let inconsistent_run_count = insulated_lines
        .iter()
        .filter(|r| r.status == InsulationAuditStatus::InconsistentAcrossRun)
        .count();
    let total_insulated_valve_boxes = insulated_lines.iter().map(|r| r.inline_valve_boxes).sum();

    PidInsulationReport {
        summary: PidInsulationSummary {
            sheets_scanned: sheets.len(),
            total_pipelines_scanned,
            insulated_pipelines_count,
            uninsulated_pipelines_count,
            heat_traced_pipelines_count,
            hot_insulation_count,
            cold_insulation_count,
            electric_tracing_count,
            steam_fluid_tracing_count,
            personnel_protection_count,
            acoustic_jacket_count,
            missing_thickness_count,
            inconsistent_run_count,
            total_insulated_valve_boxes,
            mto_groups_count: mto_rollup.len(),
        },
        mto_rollup,
        insulated_lines,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidInsulationReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_insulation_delimited(report: &PidInsulationReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = INSULATION_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for r in &report.insulated_lines {
        let cols = [
            r.line_number.clone(),
            r.nominal_diameter.clone(),
            r.fluid_code.clone(),
            r.piping_spec.clone(),
            r.raw_insulation.clone(),
            r.category.as_str().to_string(),
            r.code_prefix.clone(),
            r.thickness_mm.map(|v| v.to_string()).unwrap_or_default(),
            r.is_heat_traced.to_string(),
            r.status.as_str().to_string(),
            r.drawing_codes.join(";"),
            r.inline_valve_boxes.to_string(),
            r.segment_count.to_string(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidInsulationReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_insulation_file(
    report: &PidInsulationReport,
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
            let body = format_pid_insulation_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_insulation_delimited(report, '\t');
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
/// and build a `PidInsulationReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_insulation_from_path(
    input: &std::path::Path,
) -> Result<PidInsulationReport, String> {
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
    Ok(analyze_pid_insulation(&refs))
}

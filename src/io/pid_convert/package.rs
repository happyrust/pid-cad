//! One-click multi-sheet & batch Digital Handover Package builder and delivery gate
//! (`PIDPACKAGE` & `--export-package`, `L6-1` / `L6-2` / `L6-3` / `L3-7` / `L4-6` / `#158`).

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeSet;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::Document;
use serde::{Deserialize, Serialize};

use super::drc::DrcSummary;
use super::export::ConvertOptions;

/// Input descriptor for one open drawing sheet included in `PIDPACKAGE`.
pub struct PackageSheetInput<'a> {
    pub tab_index: usize,
    pub drawing_name: &'a str,
    pub document: &'a Document,
}

/// Configuration for building a Digital Handover Package (`#158`).
#[derive(Debug, Clone, Default)]
pub struct PackageOptions {
    pub convert: ConvertOptions,
    pub verify_strict: bool,
    pub strict_gate: bool,
}

/// Cryptographic file inventory entry inside `handover_manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverFileEntry {
    pub rel_path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Per-sheet `PIDDRC` summary recorded inside `04_drc_and_spec_audit.json` and manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoverSheetDrcEntry {
    pub drawing_stem: String,
    pub summary: DrcSummary,
}

/// Top-level `handover_manifest.json` emitted by `PIDPACKAGE` / `--export-package`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoverManifest {
    pub schema_version: u32,
    pub plant: String,
    pub handover_ready: bool,
    pub strict_vendor_passed: bool,
    pub sheets_exported: usize,
    pub pid_files_written: usize,
    pub pdf_files_plotted: usize,
    pub bom_unique_rows: usize,
    pub instrument_loops_count: usize,
    pub incomplete_loops_count: usize,
    pub total_valves: usize,
    pub valve_mto_groups: usize,
    pub utility_services_count: usize,
    pub battery_limit_interfaces: usize,
    pub hazop_nodes_count: usize,
    pub unprotected_hazop_nodes: usize,
    pub opc_reciprocal_pairs: usize,
    pub opc_one_way_open: usize,
    pub drc_total_errors: usize,
    pub drc_total_warnings: usize,
    pub spec_total_findings: usize,
    pub conversion_errors: Vec<String>,
    pub manifest_sha256: String,
    pub files: Vec<HandoverFileEntry>,
}

#[cfg(not(target_arch = "wasm32"))]
fn strip_cad_ext(name: &str) -> String {
    let trimmed = name.trim();
    for ext in [".dxf", ".dwg", ".pid", ".DXF", ".DWG", ".PID"] {
        if let Some(stem) = trimmed.strip_suffix(ext) {
            return stem.trim().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_stem(raw: &str, fallback_idx: usize, used: &mut BTreeSet<String>) -> String {
    let stripped = strip_cad_ext(raw);
    let cleaned: String = stripped
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if cleaned.trim_matches('_').is_empty() {
        format!("SHEET-{:02}", fallback_idx + 1)
    } else {
        cleaned
    };
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2usize;
    loop {
        let cand = format!("{base}_{n}");
        if used.insert(cand.clone()) {
            return cand;
        }
        n += 1;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_package_files_recursive(
    root: &Path,
    dir: &Path,
    out: &mut Vec<HandoverFileEntry>,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let rd = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();

    for p in paths {
        if p.is_dir() {
            collect_package_files_recursive(root, &p, out)?;
        } else if p.is_file() {
            let rel = p
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "handover_manifest.json" {
                continue;
            }
            let raw = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            let sha = format!("{:x}", Sha256::digest(&raw));
            out.push(HandoverFileEntry {
                rel_path: rel,
                bytes: raw.len() as u64,
                sha256: sha,
            });
        }
    }
    Ok(())
}

/// Build a complete Digital Handover Package in `out_dir` from open `sheets` (`#158`).
#[cfg(not(target_arch = "wasm32"))]
pub fn export_handover_package_from_sheets(
    sheets: &[PackageSheetInput<'_>],
    out_dir: &Path,
    pkg_opts: &PackageOptions,
) -> Result<HandoverManifest, String> {
    use sha2::{Digest, Sha256};

    use super::bom::{extract_bom_from_batch_dir, write_bom_schedule_file};
    use super::drc::audit_sheet_drc;
    use super::export::{export_author_document_to_dir, export_document_to_dir};
    use super::hazop::{analyze_pid_hazop_nodes, write_pid_hazop_file};
    use super::loops::{analyze_pid_loops, write_pid_loops_file};
    use super::opc::{analyze_open_sheets_opc, OpcSheetInput};
    use super::promote::summarize_author_review;
    use super::spec_check::audit_pid_spec_consistency;
    use super::utility::{analyze_pid_utilities_and_interfaces, write_pid_utility_file};
    use super::valve_mto::{analyze_pid_valve_mto, write_pid_valve_file};

    if sheets.is_empty() {
        return Err("没有可打包导出的有效图纸。".to_string());
    }

    let drawings_dir = out_dir.join("drawings");
    let schedules_dir = out_dir.join("schedules");
    let audits_dir = out_dir.join("audits");
    for d in [&drawings_dir, &schedules_dir, &audits_dir] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).map_err(|e| format!("create dir {}: {e}", d.display()))?;
    }

    let mut used_stems = BTreeSet::new();
    let mut stems = Vec::with_capacity(sheets.len());
    for (idx, sh) in sheets.iter().enumerate() {
        stems.push(sanitize_stem(sh.drawing_name, idx, &mut used_stems));
    }

    // 1. Export each sheet to drawings/ with StrictVendor (+ optional .pid & PDF)
    let mut sheets_exported = 0usize;
    let mut pid_files_written = 0usize;
    let mut pdf_files_plotted = 0usize;
    let mut strict_vendor_passed = true;
    let mut conversion_errors = Vec::new();

    for (sh, stem) in sheets.iter().zip(stems.iter()) {
        let is_authored = summarize_author_review(sh.document).total_authored_symbols > 0;
        let res = if is_authored {
            export_author_document_to_dir(sh.document, stem, &drawings_dir, &pkg_opts.convert, true)
        } else {
            export_document_to_dir(sh.document, stem, &drawings_dir, &pkg_opts.convert, true)
        };
        match res {
            Ok(rep) => {
                sheets_exported += 1;
                if let Some(ref ver) = rep.pid_verification {
                    pid_files_written += 1;
                    if ver.b_level_pdf_bytes.is_some() {
                        pdf_files_plotted += 1;
                    }
                }
            }
            Err(err) => {
                strict_vendor_passed = false;
                conversion_errors.push(format!("{stem}: {err}"));
            }
        }
    }

    // 2. Consolidated BOM & Line Schedule
    let bom_rep = extract_bom_from_batch_dir(&drawings_dir)?;
    write_bom_schedule_file(&bom_rep, &schedules_dir.join("01_bom_schedule.csv"))?;
    write_bom_schedule_file(&bom_rep, &schedules_dir.join("01_bom_schedule.json"))?;

    // 3. Multi-sheet OpcSheetInputs for Loops, OPC, DRC, and Spec audits
    let opc_inputs: Vec<OpcSheetInput<'_>> = sheets
        .iter()
        .zip(stems.iter())
        .map(|(sh, stem)| OpcSheetInput {
            tab_index: sh.tab_index,
            fallback_name: stem.as_str(),
            document: sh.document,
        })
        .collect();

    let loops_rep = analyze_pid_loops(&opc_inputs);
    write_pid_loops_file(&loops_rep, &schedules_dir.join("02_instrument_loops.csv"))?;
    write_pid_loops_file(&loops_rep, &schedules_dir.join("02_instrument_loops.json"))?;

    let valve_rep = analyze_pid_valve_mto(&opc_inputs);
    write_pid_valve_file(&valve_rep, &schedules_dir.join("03_valve_mto.csv"))?;
    write_pid_valve_file(&valve_rep, &schedules_dir.join("03_valve_mto.json"))?;

    let util_rep = analyze_pid_utilities_and_interfaces(&opc_inputs);
    write_pid_utility_file(
        &util_rep,
        &schedules_dir.join("04_utility_and_battery_limits.csv"),
    )?;
    write_pid_utility_file(
        &util_rep,
        &schedules_dir.join("04_utility_and_battery_limits.json"),
    )?;

    let opc_rep = analyze_open_sheets_opc(&opc_inputs);
    let opc_json = serde_json::to_string_pretty(&opc_rep).map_err(|e| e.to_string())?;
    std::fs::write(
        audits_dir.join("03_opc_pairing.json"),
        format!("{opc_json}\n"),
    )
    .map_err(|e| e.to_string())?;

    let mut drc_sheets = Vec::new();
    let mut drc_total_errors = 0usize;
    let mut drc_total_warnings = 0usize;
    for (sh, stem) in sheets.iter().zip(stems.iter()) {
        let drc = audit_sheet_drc(sh.tab_index, &opc_inputs);
        drc_total_errors += drc.summary.error_count;
        drc_total_warnings += drc.summary.warning_count;
        drc_sheets.push(serde_json::json!({
            "drawing_stem": stem,
            "summary": drc.summary,
            "findings": drc.findings,
        }));
    }
    let spec_rep = audit_pid_spec_consistency(&opc_inputs);
    let combined_audit = serde_json::json!({
        "drc_total_errors": drc_total_errors,
        "drc_total_warnings": drc_total_warnings,
        "drc_sheets": drc_sheets,
        "spec_consistency": spec_rep,
    });
    let audit_txt = serde_json::to_string_pretty(&combined_audit).map_err(|e| e.to_string())?;
    std::fs::write(
        audits_dir.join("04_drc_and_spec_audit.json"),
        format!("{audit_txt}\n"),
    )
    .map_err(|e| e.to_string())?;

    let hazop_rep = analyze_pid_hazop_nodes(&opc_inputs);
    write_pid_hazop_file(&hazop_rep, &audits_dir.join("05_hazop_worksheet.csv"))?;
    write_pid_hazop_file(&hazop_rep, &audits_dir.join("05_hazop_worksheet.json"))?;

    // 4. Cryptographic inventory & handover_manifest.json
    let mut files = Vec::new();
    collect_package_files_recursive(out_dir, out_dir, &mut files)?;
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    let mut digest_acc = Sha256::new();
    for f in &files {
        digest_acc.update(format!("{}:{}:{}\n", f.rel_path, f.bytes, f.sha256).as_bytes());
    }
    let manifest_sha256 = format!("{:x}", digest_acc.finalize());

    let handover_ready =
        strict_vendor_passed && drc_total_errors == 0 && conversion_errors.is_empty();

    let manifest = HandoverManifest {
        schema_version: 2,
        plant: pkg_opts.convert.plant.clone(),
        handover_ready,
        strict_vendor_passed,
        sheets_exported,
        pid_files_written,
        pdf_files_plotted,
        bom_unique_rows: bom_rep.summary.total_unique_rows,
        instrument_loops_count: loops_rep.summary.total_loops,
        incomplete_loops_count: loops_rep.summary.incomplete_loops,
        total_valves: valve_rep.summary.total_valves,
        valve_mto_groups: valve_rep.summary.mto_groups_count,
        utility_services_count: util_rep.summary.utility_services_count,
        battery_limit_interfaces: util_rep.summary.battery_limit_count,
        hazop_nodes_count: hazop_rep.summary.total_nodes,
        unprotected_hazop_nodes: hazop_rep.summary.unprotected_nodes,
        opc_reciprocal_pairs: opc_rep.reciprocal_pairs,
        opc_one_way_open: opc_rep.one_way_open,
        drc_total_errors,
        drc_total_warnings,
        spec_total_findings: spec_rep.summary.total_findings,
        conversion_errors,
        manifest_sha256,
        files,
    };

    let man_txt = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(
        out_dir.join("handover_manifest.json"),
        format!("{man_txt}\n"),
    )
    .map_err(|e| e.to_string())?;

    if pkg_opts.strict_gate && !manifest.handover_ready {
        return Err(format!(
            "STRICT 交付门禁未通过（handover_ready=false：drc_errors={}，strict_vendor_passed={}，errors={:?}）。",
            manifest.drc_total_errors, manifest.strict_vendor_passed, manifest.conversion_errors
        ));
    }

    Ok(manifest)
}

/// Build a Digital Handover Package in `out_dir` from a single CAD/PID file or directory (`#158`).
#[cfg(not(target_arch = "wasm32"))]
pub fn export_handover_package_from_path(
    in_path: &Path,
    out_dir: &Path,
    pkg_opts: &PackageOptions,
) -> Result<HandoverManifest, String> {
    let mut cad_files: Vec<PathBuf> = Vec::new();
    if in_path.is_file() {
        cad_files.push(in_path.to_path_buf());
    } else if in_path.is_dir() {
        let rd = std::fs::read_dir(in_path)
            .map_err(|e| format!("read input dir {}: {e}", in_path.display()))?;
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "dxf" | "dwg" | "pid") {
                cad_files.push(p);
            }
        }
        cad_files.sort();
    } else {
        return Err(format!("输入路径不存在：{}", in_path.display()));
    }

    if cad_files.is_empty() {
        return Err(format!(
            "未在 {} 中找到可处理的 .dxf/.dwg/.pid 文件。",
            in_path.display()
        ));
    }

    let mut loaded: Vec<(String, Document)> = Vec::with_capacity(cad_files.len());
    for fp in &cad_files {
        let doc = crate::io::load_file(fp).map_err(|e| format!("load {}: {e}", fp.display()))?;
        let nm = fp
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sheet.dxf")
            .to_string();
        loaded.push((nm, doc));
    }

    let inputs: Vec<PackageSheetInput<'_>> = loaded
        .iter()
        .enumerate()
        .map(|(idx, (nm, doc))| PackageSheetInput {
            tab_index: idx,
            drawing_name: nm.as_str(),
            document: doc,
        })
        .collect();

    export_handover_package_from_sheets(&inputs, out_dir, pkg_opts)
}

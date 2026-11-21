//! Isomorphic Equipment / Instrument / Valve / Pipeline Bill-of-Materials & Line-Schedule
//! extractor (`PIDBOM` & `--export-schedule`, `L3-6` / `L4-6` / `L6-1` / `#150`).
//!
//! Emits UTF-8 BOM `.csv`, `.tsv`, or `.json` schedules whose column headers and `"items"`
//! map match `external_props` (`#145`) 1-for-1, enabling zero-retyping roundtrips:
//! extract schedule from drawings -> fill temperature/pressure/material in spreadsheet ->
//! feed back via `PIDCONVERT ... PROPS` or `PIDSAVEPID ... PROPS`.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::Document;
use serde::{Deserialize, Serialize};

use crate::io::pid_publish::parse_sppid_line_number;

use super::export::{convert_document_to_publish, ConvertOptions};
use super::external_props::normalize_lookup_key;
use super::promote::summarize_author_review;

/// Canonical 14-column header order shared by CSV and TSV schedule exports.
pub const BOM_CSV_HEADERS: [&str; 14] = [
    "ItemTag",
    "Category",
    "SPPIDClass",
    "DrawingCode",
    "Occurrences",
    "Description",
    "NominalDiameter",
    "PipingMaterialsClass",
    "FluidCode",
    "InsulSpec",
    "PressureMax",
    "PhaseTemperatureMax",
    "PressureBase",
    "PhaseTemperatureBase",
];

/// Input descriptor for one open drawing sheet scanned by `PIDBOM`.
pub struct BomSheetInput<'a> {
    pub drawing_name: &'a str,
    pub document: &'a Document,
}

/// One deduplicated engineering item or pipeline row in the exported schedule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomScheduleRow {
    pub item_tag: String,
    pub category: String,
    pub sppid_class: String,
    pub drawing_codes: Vec<String>,
    pub occurrences: usize,
    pub description: String,
    pub nominal_diameter: String,
    pub piping_materials_class: String,
    pub fluid_code: String,
    pub insul_spec: String,
    pub pressure_max: String,
    pub phase_temperature_max: String,
    pub pressure_base: String,
    pub phase_temperature_base: String,
}

/// Aggregate counts across an extracted BOM & Line Schedule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomScheduleSummary {
    pub sheets_scanned: usize,
    pub total_unique_rows: usize,
    pub equipment_count: usize,
    pub instrument_count: usize,
    pub valve_count: usize,
    pub pipeline_count: usize,
}

/// Complete extracted schedule report ready for `.csv` / `.tsv` / `.json` serialization.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomScheduleReport {
    pub summary: BomScheduleSummary,
    pub rows: Vec<BomScheduleRow>,
}

#[derive(Default)]
struct RawXmlItem {
    uid: String,
    class_name: String,
    name: String,
    item_tag: String,
    description: String,
    nominal_diameter: String,
    piping_materials_class: String,
    fluid_code: String,
    insul_spec: String,
    pressure_max: String,
    phase_temperature_max: String,
    pressure_base: String,
    phase_temperature_base: String,
}

fn classify_bom_category(sppid_class: &str) -> Option<&'static str> {
    match sppid_class {
        "PIDProcessVessel" | "PIDMechanicalEquipment" | "PIDHeatTransferEquip" => Some("Equipment"),
        "PIDInstrument" | "PIDInlineInstrument" | "PIDControlSystemFunction" => Some("Instrument"),
        "PIDPipingComponent" => Some("Valve"),
        "PIDPipeline" => Some("Pipeline"),
        _ => None,
    }
}

fn merge_nonempty(dst: &mut String, src: &str) {
    if dst.is_empty() && !src.trim().is_empty() {
        *dst = src.trim().to_string();
    }
}

fn strip_known_ext(name: &str) -> String {
    let trimmed = name.trim();
    for ext in [".dxf", ".dwg", ".pid", ".DXF", ".DWG", ".PID"] {
        if let Some(stem) = trimmed.strip_suffix(ext) {
            return stem.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn unescape_xml_attr(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    raw.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_xml_tag_attrs(inner: &str) -> (&str, Vec<(&str, String)>) {
    let trimmed = inner.trim().trim_end_matches('/').trim();
    let split_pos = trimmed
        .find(|c: char| c.is_whitespace())
        .unwrap_or(trimmed.len());
    let tag = &trimmed[..split_pos];
    let mut rest = trimmed[split_pos..].trim();
    let mut attrs = Vec::new();

    while !rest.is_empty() {
        let Some(eq_idx) = rest.find('=') else {
            break;
        };
        let key = rest[..eq_idx].trim();
        let after_eq = rest[(eq_idx + 1)..].trim_start();
        let Some(quote_ch) = after_eq.chars().next().filter(|&c| c == '"' || c == '\'') else {
            break;
        };
        let val_start = quote_ch.len_utf8();
        let Some(end_q) = after_eq[val_start..].find(quote_ch) else {
            break;
        };
        let raw_val = &after_eq[val_start..(val_start + end_q)];
        if !key.is_empty() {
            attrs.push((key, unescape_xml_attr(raw_val)));
        }
        rest = after_eq[(val_start + end_q + quote_ch.len_utf8())..].trim_start();
    }
    (tag, attrs)
}

/// Parse one SPPID `_Data.xml` payload and merge its tagged items/pipelines into `acc`.
pub fn ingest_data_xml_into_bom(
    drawing_label: &str,
    data_xml: &[u8],
    acc: &mut BTreeMap<String, BomScheduleRow>,
) -> Result<String, String> {
    let body = data_xml
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(data_xml);
    let text = std::str::from_utf8(body).map_err(|e| format!("invalid UTF-8 in _Data.xml: {e}"))?;

    let mut doc_name = strip_known_ext(drawing_label);
    let mut cur_obj: Option<RawXmlItem> = None;
    let mut cur_rel: Option<(String, String, String)> = None;
    let mut in_rel = false;

    let mut items_by_uid: BTreeMap<String, RawXmlItem> = BTreeMap::new();
    let mut pipe_to_conns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut conn_to_ppts: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let mut cursor = text;
    while let Some(lt) = cursor.find('<') {
        let after_lt = &cursor[(lt + 1)..];
        let Some(gt) = after_lt.find('>') else {
            break;
        };
        let raw_inner = after_lt[..gt].trim();
        cursor = &after_lt[(gt + 1)..];

        if raw_inner.is_empty() || raw_inner.starts_with('?') || raw_inner.starts_with('!') {
            continue;
        }

        if let Some(end_tag_raw) = raw_inner.strip_prefix('/') {
            let end_tag = end_tag_raw.trim();
            if in_rel && end_tag == "Rel" {
                in_rel = false;
                if let Some((u1, u2, def)) = cur_rel.take() {
                    if def == "PipingConnectors" && !u1.is_empty() && !u2.is_empty() {
                        pipe_to_conns.entry(u1).or_default().push(u2);
                    } else if def == "ProcessPointCollection" && !u1.is_empty() && !u2.is_empty() {
                        conn_to_ppts.entry(u1).or_default().push(u2);
                    }
                }
            } else if cur_obj.as_ref().is_some_and(|o| o.class_name == end_tag) {
                if let Some(obj) = cur_obj.take() {
                    if !obj.uid.is_empty() {
                        items_by_uid.insert(obj.uid.clone(), obj);
                    }
                }
            }
            continue;
        }

        let is_self_closing = raw_inner.ends_with('/');
        let (tag, attrs) = parse_xml_tag_attrs(raw_inner);
        if tag == "Container" {
            for (k, v) in attrs {
                if k == "DocName" && !v.trim().is_empty() {
                    doc_name = v.trim().to_string();
                }
            }
        } else if tag == "Rel" && !is_self_closing {
            in_rel = true;
            cur_rel = Some((String::new(), String::new(), String::new()));
        } else if !is_self_closing && cur_obj.is_none() && !in_rel {
            cur_obj = Some(RawXmlItem {
                class_name: tag.to_string(),
                ..Default::default()
            });
        } else if let Some(ref mut obj) = cur_obj {
            apply_iface_attrs(tag, &attrs, obj);
        } else if in_rel {
            apply_rel_attrs(tag, &attrs, &mut cur_rel);
        }
    }

    let effective_dwg = if doc_name.is_empty() {
        strip_known_ext(drawing_label)
    } else {
        doc_name.clone()
    };

    let pipeline_uids: Vec<String> = items_by_uid
        .iter()
        .filter(|(_, it)| it.class_name == "PIDPipeline")
        .map(|(u, _)| u.clone())
        .collect();

    for puid in pipeline_uids {
        let mut nd = String::new();
        let mut pmc = String::new();
        let mut ins = String::new();
        let mut pmax = String::new();
        let mut tmax = String::new();
        let mut pbase = String::new();
        let mut tbase = String::new();

        if let Some(conns) = pipe_to_conns.get(&puid) {
            for cuid in conns {
                if let Some(cit) = items_by_uid.get(cuid) {
                    merge_nonempty(&mut nd, &cit.nominal_diameter);
                    merge_nonempty(&mut pmc, &cit.piping_materials_class);
                    merge_nonempty(&mut ins, &cit.insul_spec);
                }
                if let Some(ppts) = conn_to_ppts.get(cuid) {
                    for ppuid in ppts {
                        if let Some(ppit) = items_by_uid.get(ppuid) {
                            merge_nonempty(&mut pmax, &ppit.pressure_max);
                            merge_nonempty(&mut tmax, &ppit.phase_temperature_max);
                            merge_nonempty(&mut pbase, &ppit.pressure_base);
                            merge_nonempty(&mut tbase, &ppit.phase_temperature_base);
                        }
                    }
                }
            }
        }

        if let Some(pit) = items_by_uid.get_mut(&puid) {
            merge_nonempty(&mut pit.nominal_diameter, &nd);
            merge_nonempty(&mut pit.piping_materials_class, &pmc);
            merge_nonempty(&mut pit.insul_spec, &ins);
            merge_nonempty(&mut pit.pressure_max, &pmax);
            merge_nonempty(&mut pit.phase_temperature_max, &tmax);
            merge_nonempty(&mut pit.pressure_base, &pbase);
            merge_nonempty(&mut pit.phase_temperature_base, &tbase);

            let tag_candidate = if !pit.item_tag.is_empty() {
                pit.item_tag.clone()
            } else {
                pit.name.clone()
            };
            if let Some(parsed) = parse_sppid_line_number(&tag_candidate) {
                merge_nonempty(&mut pit.nominal_diameter, &parsed.nominal_diameter_attr);
                merge_nonempty(&mut pit.fluid_code, &parsed.fluid_code);
                merge_nonempty(&mut pit.piping_materials_class, &parsed.materials_class);
                if let Some(ref ins_parsed) = parsed.insulation {
                    merge_nonempty(&mut pit.insul_spec, ins_parsed);
                }
            }
        }
    }

    for item in items_by_uid.values() {
        let Some(cat) = classify_bom_category(&item.class_name) else {
            continue;
        };
        let raw_tag = if !item.item_tag.trim().is_empty() {
            item.item_tag.trim()
        } else {
            item.name.trim()
        };
        if raw_tag.is_empty() {
            continue;
        }
        let norm_k = normalize_lookup_key(raw_tag);
        if norm_k.is_empty() {
            continue;
        }

        let entry = acc
            .entry(format!("{cat}:{norm_k}"))
            .or_insert_with(|| BomScheduleRow {
                item_tag: raw_tag.trim_end_matches('-').to_string(),
                category: cat.to_string(),
                sppid_class: item.class_name.clone(),
                ..Default::default()
            });
        entry.occurrences += 1;
        if !effective_dwg.is_empty() && !entry.drawing_codes.contains(&effective_dwg) {
            entry.drawing_codes.push(effective_dwg.clone());
        }
        merge_nonempty(&mut entry.description, &item.description);
        merge_nonempty(&mut entry.nominal_diameter, &item.nominal_diameter);
        merge_nonempty(
            &mut entry.piping_materials_class,
            &item.piping_materials_class,
        );
        merge_nonempty(&mut entry.fluid_code, &item.fluid_code);
        merge_nonempty(&mut entry.insul_spec, &item.insul_spec);
        merge_nonempty(&mut entry.pressure_max, &item.pressure_max);
        merge_nonempty(
            &mut entry.phase_temperature_max,
            &item.phase_temperature_max,
        );
        merge_nonempty(&mut entry.pressure_base, &item.pressure_base);
        merge_nonempty(
            &mut entry.phase_temperature_base,
            &item.phase_temperature_base,
        );
    }

    Ok(effective_dwg)
}

fn apply_iface_attrs(tag: &str, attrs: &[(&str, String)], obj: &mut RawXmlItem) {
    for (k, raw_v) in attrs {
        let val = raw_v.trim();
        if val.is_empty() {
            continue;
        }
        match (tag, *k) {
            ("IObject", "UID") => obj.uid = val.to_string(),
            ("IObject", "Name") => obj.name = val.to_string(),
            ("IObject", "Description") => obj.description = val.to_string(),
            ("IPlantItem", "ItemTag") => obj.item_tag = val.to_string(),
            ("IPipeCrossSectionItem", "NominalDiameter") => obj.nominal_diameter = val.to_string(),
            ("IPipingSpecifiedItem", "PipingMaterialsClass") => {
                obj.piping_materials_class = val.to_string()
            }
            ("IFluidSystem", "FluidCode") => obj.fluid_code = val.to_string(),
            ("IInsulatedItem", "InsulSpec") => obj.insul_spec = val.to_string(),
            ("IBulkMaxProcessPoint", "PressureMax") => obj.pressure_max = val.to_string(),
            ("IBulkMaxProcessPoint", "PhaseTemperatureMax") => {
                obj.phase_temperature_max = val.to_string()
            }
            ("IBulkBaseProcessPoint", "PressureBase") => obj.pressure_base = val.to_string(),
            ("IBulkBaseProcessPoint", "PhaseTemperatureBase") => {
                obj.phase_temperature_base = val.to_string()
            }
            _ => {}
        }
    }
}

fn apply_rel_attrs(
    tag: &str,
    attrs: &[(&str, String)],
    cur_rel: &mut Option<(String, String, String)>,
) {
    if tag != "IRel" {
        return;
    }
    if let Some((ref mut u1, ref mut u2, ref mut def)) = cur_rel {
        for (k, raw_v) in attrs {
            let val = raw_v.trim().to_string();
            match *k {
                "UID1" => *u1 = val,
                "UID2" => *u2 = val,
                "DefUID" => *def = val,
                _ => {}
            }
        }
    }
}

fn finalize_bom_report(
    sheets_scanned: usize,
    acc: BTreeMap<String, BomScheduleRow>,
) -> BomScheduleReport {
    let mut rows: Vec<BomScheduleRow> = acc.into_values().collect();
    let cat_rank = |c: &str| -> u8 {
        match c {
            "Equipment" => 0,
            "Pipeline" => 1,
            "Instrument" => 2,
            "Valve" => 3,
            _ => 4,
        }
    };
    rows.sort_by(|a, b| {
        (cat_rank(&a.category), &a.item_tag).cmp(&(cat_rank(&b.category), &b.item_tag))
    });

    let mut summary = BomScheduleSummary {
        sheets_scanned,
        total_unique_rows: rows.len(),
        ..Default::default()
    };
    for r in &rows {
        match r.category.as_str() {
            "Equipment" => summary.equipment_count += 1,
            "Instrument" => summary.instrument_count += 1,
            "Valve" => summary.valve_count += 1,
            "Pipeline" => summary.pipeline_count += 1,
            _ => {}
        }
    }
    BomScheduleReport { summary, rows }
}

/// Extract a unified BOM & Line Schedule across one or more open `Document` sheets (`#150`).
pub fn extract_bom_from_documents(
    sheets: &[BomSheetInput<'_>],
    opts: &ConvertOptions,
) -> Result<BomScheduleReport, String> {
    let mut acc: BTreeMap<String, BomScheduleRow> = BTreeMap::new();
    let mut xml_opts = opts.clone();
    xml_opts.seed_pid_path = None;
    xml_opts.verify_plot_pdf = false;

    for sh in sheets {
        let stem = strip_known_ext(sh.drawing_name);
        let eff_stem = if stem.is_empty() {
            "SHEET"
        } else {
            stem.as_str()
        };

        #[cfg(not(target_arch = "wasm32"))]
        let data_xml = if summarize_author_review(sh.document).total_authored_symbols > 0 {
            super::promote::save_author_document_to_dual(sh.document, eff_stem, &xml_opts)?
                .publish
                .data_xml
        } else {
            convert_document_to_publish(sh.document, eff_stem, &xml_opts)?
                .0
                .data_xml
        };

        #[cfg(target_arch = "wasm32")]
        let data_xml = {
            let _ = summarize_author_review(sh.document);
            convert_document_to_publish(sh.document, eff_stem, &xml_opts)?
                .0
                .data_xml
        };

        ingest_data_xml_into_bom(eff_stem, &data_xml, &mut acc)?;
    }

    Ok(finalize_bom_report(sheets.len(), acc))
}

/// Scan all `<stem>_Data.xml` files in `out_dir` and build a consolidated BOM & Line Schedule.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_bom_from_batch_dir(out_dir: &Path) -> Result<BomScheduleReport, String> {
    let rd = std::fs::read_dir(out_dir)
        .map_err(|e| format!("read dir {} failed: {e}", out_dir.display()))?;
    let mut xml_paths: Vec<std::path::PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.ends_with("_Data.xml"))
        })
        .collect();
    xml_paths.sort();

    let mut acc: BTreeMap<String, BomScheduleRow> = BTreeMap::new();
    let mut scanned = BTreeSet::new();
    for xp in &xml_paths {
        let fname = xp.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let stem = fname.strip_suffix("_Data.xml").unwrap_or("SHEET");
        let raw = std::fs::read(xp).map_err(|e| format!("read {}: {e}", xp.display()))?;
        let dwg = ingest_data_xml_into_bom(stem, &raw, &mut acc)?;
        scanned.insert(dwg);
    }
    Ok(finalize_bom_report(xml_paths.len().max(scanned.len()), acc))
}

fn escape_delimited_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `BomScheduleReport` as delimited text (`,` for CSV or `\t` for TSV) with UTF-8 BOM.
pub fn format_bom_delimited(report: &BomScheduleReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let header_line: Vec<String> = BOM_CSV_HEADERS
        .iter()
        .map(|h| escape_delimited_cell(h, delim))
        .collect();
    out.push_str(&header_line.join(&delim.to_string()));
    out.push_str("\r\n");

    for r in &report.rows {
        let cells = [
            r.item_tag.as_str(),
            r.category.as_str(),
            r.sppid_class.as_str(),
            &r.drawing_codes.join(";"),
            &r.occurrences.to_string(),
            r.description.as_str(),
            r.nominal_diameter.as_str(),
            r.piping_materials_class.as_str(),
            r.fluid_code.as_str(),
            r.insul_spec.as_str(),
            r.pressure_max.as_str(),
            r.phase_temperature_max.as_str(),
            r.pressure_base.as_str(),
            r.phase_temperature_base.as_str(),
        ];
        let escaped: Vec<String> = cells
            .iter()
            .map(|c| escape_delimited_cell(c, delim))
            .collect();
        out.push_str(&escaped.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Format `BomScheduleReport` as JSON containing `summary`, `rows`, and `external_props`-compatible `items`.
pub fn format_bom_json(report: &BomScheduleReport) -> Result<String, String> {
    let mut items_map = serde_json::Map::new();
    for r in &report.rows {
        let mut obj = serde_json::Map::new();
        obj.insert("Category".to_string(), serde_json::json!(r.category));
        obj.insert("SPPIDClass".to_string(), serde_json::json!(r.sppid_class));
        obj.insert(
            "DrawingCode".to_string(),
            serde_json::json!(r.drawing_codes.join(";")),
        );
        obj.insert("Description".to_string(), serde_json::json!(r.description));
        obj.insert(
            "NominalDiameter".to_string(),
            serde_json::json!(r.nominal_diameter),
        );
        obj.insert(
            "PipingMaterialsClass".to_string(),
            serde_json::json!(r.piping_materials_class),
        );
        obj.insert("FluidCode".to_string(), serde_json::json!(r.fluid_code));
        obj.insert("InsulSpec".to_string(), serde_json::json!(r.insul_spec));
        obj.insert("PressureMax".to_string(), serde_json::json!(r.pressure_max));
        obj.insert(
            "PhaseTemperatureMax".to_string(),
            serde_json::json!(r.phase_temperature_max),
        );
        obj.insert(
            "PressureBase".to_string(),
            serde_json::json!(r.pressure_base),
        );
        obj.insert(
            "PhaseTemperatureBase".to_string(),
            serde_json::json!(r.phase_temperature_base),
        );
        items_map.insert(r.item_tag.clone(), serde_json::Value::Object(obj));
    }

    let root = serde_json::json!({
        "summary": report.summary,
        "rows": report.rows,
        "items": items_map,
    });
    serde_json::to_string_pretty(&root).map_err(|e| format!("serialize BOM JSON: {e}"))
}

/// Write `BomScheduleReport` to `.csv`, `.tsv`, or `.json` based on `out_path` extension.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_bom_schedule_file(report: &BomScheduleReport, out_path: &Path) -> Result<(), String> {
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let ext = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("csv")
        .to_ascii_lowercase();
    let payload = match ext.as_str() {
        "json" => format!("{}\n", format_bom_json(report)?),
        "tsv" | "txt" => format_bom_delimited(report, '\t'),
        "csv" => format_bom_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported BOM schedule extension '.{other}' (expected .csv, .tsv, or .json)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

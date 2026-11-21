//! External Equipment-List / Line-List / Instrument-List property overlay
//! (`L3-6` / `RES-11` / `V1-03` safeguard / `#145`).
//!
//! Loads tabular engineering schedules from `.xlsx` / `.xlsm` / `.xls` (all
//! worksheets via `calamine`), `.csv`, `.tsv`, or `.json`, resolves both
//! explicit `<Interface>.<Attribute>` headers and bilingual engineering column
//! aliases (`位号`, `设备名称`, `设计压力`, `设计温度`, `公称直径`, `管道等级`, `保温规格`,
//! `介质`), routes line-level temperature/pressure attributes onto child
//! `PIDProcessPoint` (`.PPT`) items, and gates every write against
//! `assets/pid-publish-spec.json` so `StrictVendor` compliance is never broken.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::io::pid_publish::{
    format_nominal_diameter_attr, parse_sppid_line_number, ExportDrawing, ExportItem,
};
use crate::io::pid_publish_check::default_publish_spec;

/// One row of normalized external attributes keyed by raw or canonical tag/line number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalItemRecord {
    pub raw_key: String,
    pub fields: BTreeMap<String, String>,
}

/// Compiled external property lookup table across one or more sheets/files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalPropertyTable {
    pub records_by_norm_key: BTreeMap<String, ExternalItemRecord>,
    pub total_rows_loaded: usize,
}

/// Observable summary of applying an `ExternalPropertyTable` to one `ExportDrawing`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalPropsReport {
    pub table_rows_loaded: usize,
    pub matched_drawing_items: usize,
    pub process_points_updated: usize,
    pub attributes_injected: usize,
    pub ignored_disallowed_columns: usize,
}

pub fn normalize_lookup_key(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('-')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

fn is_key_header(col: &str) -> bool {
    let norm: String = col
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect::<String>()
        .to_uppercase();
    matches!(
        norm.as_str(),
        "ITEMTAG"
            | "TAG"
            | "NAME"
            | "LINENUMBER"
            | "LINE"
            | "PIPELINE"
            | "位号"
            | "设备位号"
            | "仪表位号"
            | "管线号"
            | "线号"
            | "管道号"
    )
}

impl ExternalPropertyTable {
    fn upsert_row(&mut self, raw_key: &str, fields: BTreeMap<String, String>) {
        let k0 = normalize_lookup_key(raw_key);
        if k0.is_empty() || fields.is_empty() {
            return;
        }
        self.total_rows_loaded += 1;

        let mut keys = vec![k0];
        if let Some(parsed) = parse_sppid_line_number(raw_key) {
            let pk = normalize_lookup_key(&parsed.pipeline_name());
            if !pk.is_empty() && !keys.contains(&pk) {
                keys.push(pk);
            }
        }
        // Also index alphanumeric-only form (e.g. `BV-0201501A` <-> `BV0201501A`) when unambiguous
        let alnum: String = raw_key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_uppercase();
        if alnum.len() >= 4 && !keys.contains(&alnum) {
            keys.push(alnum);
        }

        for k in keys {
            let entry = self
                .records_by_norm_key
                .entry(k)
                .or_insert_with(|| ExternalItemRecord {
                    raw_key: raw_key.trim().to_string(),
                    fields: BTreeMap::new(),
                });
            for (fk, fv) in &fields {
                if !fv.trim().is_empty() {
                    entry.fields.insert(fk.clone(), fv.trim().to_string());
                }
            }
        }
    }

    fn ingest_matrix_rows(&mut self, rows: &[Vec<String>]) {
        let Some(header_idx) = rows
            .iter()
            .position(|r| r.iter().any(|c| !c.trim().is_empty()))
        else {
            return;
        };
        let headers: Vec<String> = rows[header_idx]
            .iter()
            .map(|h| h.trim().trim_start_matches('\u{feff}').to_string())
            .collect();
        let key_col = headers.iter().position(|h| is_key_header(h)).unwrap_or(0);
        if key_col >= headers.len() {
            return;
        }

        for row in &rows[(header_idx + 1)..] {
            let Some(raw_k) = row.get(key_col).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                continue;
            };
            let mut fmap = BTreeMap::new();
            for (ci, h) in headers.iter().enumerate() {
                if ci == key_col || h.is_empty() {
                    continue;
                }
                if let Some(val) = row.get(ci).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    fmap.insert(h.clone(), val.to_string());
                }
            }
            self.upsert_row(raw_k, fmap);
        }
    }
}

fn split_delimited_line(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    let _ = chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == delim {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

/// Parse an `ExternalPropertyTable` from delimited text (`,` for CSV, `\t` for TSV).
pub fn parse_external_property_delimited(
    raw: &str,
    delimiter: char,
) -> Result<ExternalPropertyTable, String> {
    let clean = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let rows: Vec<Vec<String>> = clean
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| split_delimited_line(l, delimiter))
        .collect();
    let mut table = ExternalPropertyTable::default();
    table.ingest_matrix_rows(&rows);
    Ok(table)
}

/// Parse an `ExternalPropertyTable` from JSON text (`{"items": {"TAG": {...}}}` or `[{"ItemTag": "...", ...}]`).
pub fn parse_external_property_json(raw: &str) -> Result<ExternalPropertyTable, String> {
    let clean = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let val: serde_json::Value =
        serde_json::from_str(clean).map_err(|e| format!("invalid external-props JSON: {e}"))?;
    let mut table = ExternalPropertyTable::default();

    let map_obj_opt = val.as_object().and_then(|o| {
        o.get("items")
            .and_then(serde_json::Value::as_object)
            .or(Some(o))
    });

    if let Some(obj) = map_obj_opt {
        for (k, v) in obj {
            if let Some(sub) = v.as_object() {
                let mut fmap = BTreeMap::new();
                for (fk, fv) in sub {
                    let s = match fv {
                        serde_json::Value::String(st) => st.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => {
                            if *b {
                                "True".to_string()
                            } else {
                                "False".to_string()
                            }
                        }
                        _ => continue,
                    };
                    if !s.trim().is_empty() {
                        fmap.insert(fk.clone(), s);
                    }
                }
                table.upsert_row(k, fmap);
            }
        }
        return Ok(table);
    }

    if let Some(arr) = val.as_array() {
        for item in arr {
            let Some(sub) = item.as_object() else {
                continue;
            };
            let Some(key_field) = sub.keys().find(|k| is_key_header(k)).cloned() else {
                continue;
            };
            let Some(raw_k) = sub.get(&key_field).and_then(serde_json::Value::as_str) else {
                continue;
            };
            let mut fmap = BTreeMap::new();
            for (fk, fv) in sub {
                if fk == &key_field {
                    continue;
                }
                if let Some(s) = fv.as_str().filter(|s| !s.trim().is_empty()) {
                    fmap.insert(fk.clone(), s.to_string());
                }
            }
            table.upsert_row(raw_k, fmap);
        }
        return Ok(table);
    }

    Err("external-props JSON must be an object map or array of row objects".to_string())
}

/// Load an `ExternalPropertyTable` from `.xlsx` / `.xlsm` / `.xls`, `.csv`, `.tsv`, or `.json`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_external_property_table(path: &Path) -> Result<ExternalPropertyTable, String> {
    if !path.is_file() {
        return Err(format!(
            "external property file does not exist: {}",
            path.display()
        ));
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "json" => {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            parse_external_property_json(&raw)
        }
        "csv" => {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            parse_external_property_delimited(&raw, ',')
        }
        "tsv" | "txt" => {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            parse_external_property_delimited(&raw, '\t')
        }
        "xlsx" | "xlsm" | "xls" | "ods" => {
            use calamine::{open_workbook_auto, Reader};
            let mut wb = open_workbook_auto(path)
                .map_err(|e| format!("open workbook {}: {e}", path.display()))?;
            let names: Vec<String> = wb.sheet_names().to_vec();
            let mut table = ExternalPropertyTable::default();
            for sname in names {
                let range = wb
                    .worksheet_range(&sname)
                    .map_err(|e| format!("read worksheet '{sname}' in {}: {e}", path.display()))?;
                let rows: Vec<Vec<String>> = range
                    .rows()
                    .map(|r| r.iter().map(ToString::to_string).collect())
                    .collect();
                table.ingest_matrix_rows(&rows);
            }
            Ok(table)
        }
        other => Err(format!(
            "unsupported external-props file extension '.{other}' (expected .xlsx/.xlsm/.xls/.csv/.tsv/.json)"
        )),
    }
}

enum ResolvedColumn {
    Direct(String, String),
    ProcessPoint(String, String),
    Unknown,
}

fn resolve_column_target(col: &str) -> ResolvedColumn {
    let trimmed = col.trim();
    if let Some((iface, attr)) = trimmed.split_once('.') {
        let iface = iface.trim();
        let attr = attr.trim();
        if iface.is_empty() || attr.is_empty() {
            return ResolvedColumn::Unknown;
        }
        if iface.starts_with("IBulk") && iface.ends_with("ProcessPoint") {
            return ResolvedColumn::ProcessPoint(iface.to_string(), attr.to_string());
        }
        return ResolvedColumn::Direct(iface.to_string(), attr.to_string());
    }

    let key: String = trimmed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .collect::<String>()
        .to_uppercase();

    match key.as_str() {
        "DESCRIPTION" | "DESC" | "名称" | "设备名称" | "仪表名称" | "阀门名称" | "说明" => {
            ResolvedColumn::Direct("IObject".to_string(), "Description".to_string())
        }
        "EQTYPEDESCRIPTION" | "设备类型描述" | "设备型式" => {
            ResolvedColumn::Direct("IEquipment".to_string(), "EqTypeDescription".to_string())
        }
        "EQUIPMENTTRIMSPEC" | "设备内件等级" => {
            ResolvedColumn::Direct("IEquipment".to_string(), "EquipmentTrimSpec".to_string())
        }
        "LONGMATERIALDESCRIPTION" | "MATERIAL" | "材质" | "主体材质" => {
            ResolvedColumn::Direct(
                "ISpecifiedMatlItem".to_string(),
                "LongMaterialDescription".to_string(),
            )
        }
        "INTERNALDIAMETER" | "PROCESSVESSELINTERNALDIAMETER" | "内径" => ResolvedColumn::Direct(
            "IProcessVessel".to_string(),
            "ProcessVessel_InternalDiameter".to_string(),
        ),
        "NOMINALDIAMETER" | "DN" | "SIZE" | "公称直径" | "管径" => ResolvedColumn::Direct(
            "IPipeCrossSectionItem".to_string(),
            "NominalDiameter".to_string(),
        ),
        "PIPINGMATERIALSCLASS" | "SPEC" | "PIPESPEC" | "管道等级" | "材质等级" => {
            ResolvedColumn::Direct(
                "IPipingSpecifiedItem".to_string(),
                "PipingMaterialsClass".to_string(),
            )
        }
        "INSULSPEC" | "INSULATION" | "保温规格" | "绝热规格" | "保温" => {
            ResolvedColumn::Direct("IInsulatedItem".to_string(), "InsulSpec".to_string())
        }
        "FLUIDCODE" | "FLUID" | "介质" | "物料代号" => {
            ResolvedColumn::Direct("IFluidSystem".to_string(), "FluidCode".to_string())
        }
        "FLUIDSYSTEM" | "流体系统" => {
            ResolvedColumn::Direct("IFluidSystem".to_string(), "FluidSystem".to_string())
        }
        "RELIEFPRESSURE" | "整定压力" | "起跳压力" => ResolvedColumn::Direct(
            "IPressureReliefItem".to_string(),
            "ReliefPressure".to_string(),
        ),
        "PRESSUREMAX" | "DESIGNMAXPRESSURE" | "DESIGNPRESSURE" | "设计压力" | "最高压力" => {
            ResolvedColumn::ProcessPoint(
                "IBulkMaxProcessPoint".to_string(),
                "PressureMax".to_string(),
            )
        }
        "PHASETEMPERATUREMAX" | "DESIGNMAXTEMP" | "DESIGNTEMP" | "设计温度" | "最高温度" => {
            ResolvedColumn::ProcessPoint(
                "IBulkMaxProcessPoint".to_string(),
                "PhaseTemperatureMax".to_string(),
            )
        }
        "PRESSUREBASE" | "OPERATINGPRESSURE" | "NORMALPRESSURE" | "操作压力" | "工作压力" => {
            ResolvedColumn::ProcessPoint(
                "IBulkBaseProcessPoint".to_string(),
                "PressureBase".to_string(),
            )
        }
        "PHASETEMPERATUREBASE" | "OPERATINGTEMP" | "NORMALTEMP" | "操作温度" | "工作温度" => {
            ResolvedColumn::ProcessPoint(
                "IBulkBaseProcessPoint".to_string(),
                "PhaseTemperatureBase".to_string(),
            )
        }
        "PRESSUREMIN" | "MINPRESSURE" | "最低压力" => ResolvedColumn::ProcessPoint(
            "IBulkMinProcessPoint".to_string(),
            "PressureMin".to_string(),
        ),
        "PHASETEMPERATUREMIN" | "MINTEMP" | "最低温度" => ResolvedColumn::ProcessPoint(
            "IBulkMinProcessPoint".to_string(),
            "PhaseTemperatureMin".to_string(),
        ),
        _ => ResolvedColumn::Unknown,
    }
}

fn try_inject_allowed_attr(item: &mut ExportItem, iface: &str, attr: &str, raw_val: &str) -> bool {
    let spec = default_publish_spec();
    let Some(cls_spec) = spec.data_classes.get(&item.class) else {
        return false;
    };
    if !cls_spec.allowed_interfaces.iter().any(|i| i == iface) {
        return false;
    }
    let allowed_on_iface = cls_spec
        .interface_attributes
        .get(iface)
        .is_some_and(|s| s.allowed.iter().any(|a| a == attr));
    if !allowed_on_iface {
        return false;
    }

    let final_val = if iface == "IPipeCrossSectionItem" && attr == "NominalDiameter" {
        format_nominal_diameter_attr(raw_val).unwrap_or_else(|| raw_val.trim().to_string())
    } else {
        raw_val.trim().to_string()
    };

    if iface == "IObject" && attr == "Description" {
        item.description = Some(final_val.clone());
    }
    item.attributes
        .insert((iface.to_string(), attr.to_string()), final_val);
    true
}

/// Apply `table` onto `dwg` (`PIDProcessVessel`, `PIDPipingComponent`, `PIDInstrument`,
/// `PIDPipeline`, `PIDPipingConnector`, and associated `.PPT` `PIDProcessPoint`s).
pub fn apply_external_properties_to_drawing(
    dwg: &mut ExportDrawing,
    table: &ExternalPropertyTable,
) -> ExternalPropsReport {
    let mut report = ExternalPropsReport {
        table_rows_loaded: table.total_rows_loaded,
        ..Default::default()
    };
    if table.records_by_norm_key.is_empty() {
        return report;
    }

    let uid_to_idx: BTreeMap<String, usize> = dwg
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.uid.clone(), i))
        .collect();

    let mut pipe_to_conns: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut conn_to_ppts: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for rel in &dwg.relations {
        if rel.def_uid == "PipingConnectors" {
            if let Some(&ci) = uid_to_idx.get(&rel.uid1) {
                pipe_to_conns.entry(rel.uid2.clone()).or_default().push(ci);
            }
        } else if rel.def_uid == "ProcessPointCollection" {
            if let Some(&pi) = uid_to_idx.get(&rel.uid1) {
                conn_to_ppts.entry(rel.uid2.clone()).or_default().push(pi);
            }
        }
    }

    let mut touched_ppts: BTreeSet<usize> = BTreeSet::new();
    let item_count = dwg.items.len();

    for idx in 0..item_count {
        let cls = dwg.items[idx].class.clone();
        if matches!(
            cls.as_str(),
            "PIDDrawing" | "PIDRepresentation" | "PIDProcessPoint"
        ) {
            continue;
        }
        let Some(name_str) = dwg.items[idx].name.clone() else {
            continue;
        };
        let norm_k = normalize_lookup_key(&name_str);
        let rec_opt = table.records_by_norm_key.get(&norm_k).or_else(|| {
            let alnum: String = name_str
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_uppercase();
            table.records_by_norm_key.get(&alnum)
        });
        let Some(rec) = rec_opt else {
            continue;
        };

        report.matched_drawing_items += 1;
        let item_uid = dwg.items[idx].uid.clone();

        for (col_name, val) in &rec.fields {
            match resolve_column_target(col_name) {
                ResolvedColumn::Direct(iface, attr) => {
                    let mut applied_any =
                        try_inject_allowed_attr(&mut dwg.items[idx], &iface, &attr, val);
                    if cls == "PIDPipeline" {
                        if let Some(conn_indices) = pipe_to_conns.get(&item_uid) {
                            for &ci in conn_indices {
                                if try_inject_allowed_attr(&mut dwg.items[ci], &iface, &attr, val) {
                                    applied_any = true;
                                }
                            }
                        }
                    }
                    if applied_any {
                        report.attributes_injected += 1;
                    } else {
                        report.ignored_disallowed_columns += 1;
                    }
                }
                ResolvedColumn::ProcessPoint(iface, attr) => {
                    let mut target_ppts = Vec::new();
                    if cls == "PIDPipeline" {
                        if let Some(conn_indices) = pipe_to_conns.get(&item_uid) {
                            for &ci in conn_indices {
                                let cuid = &dwg.items[ci].uid;
                                if let Some(ppts) = conn_to_ppts.get(cuid) {
                                    target_ppts.extend(ppts.iter().copied());
                                }
                            }
                        }
                    } else if cls == "PIDPipingConnector" {
                        if let Some(ppts) = conn_to_ppts.get(&item_uid) {
                            target_ppts.extend(ppts.iter().copied());
                        }
                    }
                    let mut applied_ppt = false;
                    for pi in target_ppts {
                        if try_inject_allowed_attr(&mut dwg.items[pi], &iface, &attr, val) {
                            touched_ppts.insert(pi);
                            applied_ppt = true;
                        }
                    }
                    if applied_ppt {
                        report.attributes_injected += 1;
                    } else {
                        report.ignored_disallowed_columns += 1;
                    }
                }
                ResolvedColumn::Unknown => {
                    report.ignored_disallowed_columns += 1;
                }
            }
        }
    }

    report.process_points_updated = touched_ppts.len();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_publish::{
        export_drawing_bundle, parse_sppid_line_number, pipeline_uid, synthesize_piping_network,
        ExportItem, PipeRunInput, SymbolEndpointRef, UidAllocator,
    };
    use crate::io::pid_publish_check::{
        check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
    };

    #[cfg(not(target_arch = "wasm32"))]
    fn write_minimal_two_sheet_xlsx(path: &Path, equip_rows: &[&[&str]], line_rows: &[&[&str]]) {
        use std::io::Write as _;
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opt = zip::write::SimpleFileOptions::default();

        let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;
        zip.start_file("[Content_Types].xml", opt).unwrap();
        zip.write_all(content_types.as_bytes()).unwrap();

        let root_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
        zip.start_file("_rels/.rels", opt).unwrap();
        zip.write_all(root_rels.as_bytes()).unwrap();

        let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="EquipmentList" sheetId="1" r:id="rId1"/>
    <sheet name="LineList" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
        zip.start_file("xl/workbook.xml", opt).unwrap();
        zip.write_all(workbook.as_bytes()).unwrap();

        let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
        zip.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        zip.write_all(wb_rels.as_bytes()).unwrap();

        fn render_sheet_xml(rows: &[&[&str]]) -> String {
            let mut body = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
            );
            for (ri, r) in rows.iter().enumerate() {
                body.push_str(&format!(r#"<row r="{}">"#, ri + 1));
                for (ci, val) in r.iter().enumerate() {
                    let col_ch = (b'A' + (ci as u8)) as char;
                    let esc = val
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    body.push_str(&format!(
                        r#"<c r="{col_ch}{}" t="inlineStr"><is><t>{esc}</t></is></c>"#,
                        ri + 1
                    ));
                }
                body.push_str("</row>");
            }
            body.push_str("</sheetData></worksheet>");
            body
        }

        zip.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        zip.write_all(render_sheet_xml(equip_rows).as_bytes())
            .unwrap();
        zip.start_file("xl/worksheets/sheet2.xml", opt).unwrap();
        zip.write_all(render_sheet_xml(line_rows).as_bytes())
            .unwrap();
        zip.finish().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_multisheet_xlsx_csv_and_json_routes_process_point_temp_pressure_and_keeps_strict_vendor(
    ) {
        let dir = std::env::temp_dir().join(format!("ocs_extprops_145_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let xlsx_path = dir.join("schedules.xlsx");
        write_minimal_two_sheet_xlsx(
            &xlsx_path,
            &[
                &[
                    "设备位号",
                    "设备名称",
                    "IEquipment.EqTypeDescription",
                    "ISpecifiedMatlItem.LongMaterialDescription",
                    "UnrelatedExcelNote",
                ],
                &[
                    "V-1001",
                    "Primary Flash Separator",
                    "Vertical Two-Phase Drum",
                    "Q345R Normalized",
                    "Ignored safely",
                ],
            ],
            &[
                &[
                    "管线号",
                    "介质",
                    "管道等级",
                    "保温规格",
                    "设计压力",
                    "设计温度",
                    "操作压力",
                    "操作温度",
                ],
                &[
                    "80-PG-2001-B9-",
                    "PG",
                    "C12",
                    "H50",
                    "4.0 MPa",
                    "150 C",
                    "2.8 MPa",
                    "95 C",
                ],
            ],
        );

        let table = load_external_property_table(&xlsx_path).expect("load 2-sheet .xlsx");
        assert_eq!(table.total_rows_loaded, 2);

        // Build a drawing with V-1001 + synthesized pipe run 80-PG-2001-B9-
        let mut alloc = UidAllocator::new("P01", "EXT-145");
        let doc_uid = alloc.alloc_primary_uid(None, "doc:EXT-145");
        let mut dwg = ExportDrawing::new("P01", &doc_uid, "EXT-145");

        let v_uid = alloc.alloc_primary_uid(None, "sym:V-1001");
        let v_goid = alloc.alloc_graphic_oid(None);
        dwg.items.push(ExportItem {
            uid: v_uid.clone(),
            class: "PIDProcessVessel".to_string(),
            name: Some("V-1001".to_string()),
            variant_id: Some("PIDProcessVessel#01".to_string()),
            graphic_oids: vec![v_goid],
            ..Default::default()
        });

        let endpoints = vec![SymbolEndpointRef {
            symbol_uid: v_uid,
            sppid_class: "PIDProcessVessel".to_string(),
            bbox: (40.0, 90.0, 60.0, 110.0),
            ports: vec![(60.0, 100.0)],
        }];
        let c_goid = alloc.alloc_graphic_oid(None);
        let runs = vec![PipeRunInput {
            start: (60.0, 100.0),
            end: (140.0, 100.0),
            line_label: Some("80-PG-2001-B9-".to_string()),
            graphic_oid: Some(c_goid),
        }];
        let _ = synthesize_piping_network(&mut dwg, &mut alloc, &endpoints, &runs, 2.0).unwrap();
        if let Some(parsed) = parse_sppid_line_number("80-PG-2001-B9-") {
            let pname = parsed.pipeline_name();
            if dwg
                .items
                .iter()
                .all(|i| !(i.class == "PIDPipeline" && i.name.as_deref() == Some(pname.as_str())))
            {
                dwg.items.push(ExportItem {
                    uid: pipeline_uid("P01", &pname),
                    class: "PIDPipeline".to_string(),
                    name: Some(pname),
                    ..Default::default()
                });
            }
        }

        let rep = apply_external_properties_to_drawing(&mut dwg, &table);
        assert!(rep.matched_drawing_items >= 2, "{rep:?}");
        assert!(rep.process_points_updated >= 1, "{rep:?}");
        assert!(rep.attributes_injected >= 7, "{rep:?}");
        assert!(rep.ignored_disallowed_columns >= 1, "{rep:?}");

        let bundle = export_drawing_bundle(&dwg).expect("export bundle with external props");
        let xml = String::from_utf8_lossy(&bundle.data_xml);
        assert!(xml.contains("Primary Flash Separator"));
        assert!(xml.contains("Vertical Two-Phase Drum"));
        assert!(xml.contains("Q345R Normalized"));
        assert!(xml.contains(r#"PressureMax="4.0 MPa""#));
        assert!(xml.contains(r#"PhaseTemperatureMax="150 C""#));
        assert!(xml.contains(r#"PressureBase="2.8 MPa""#));
        assert!(xml.contains(r#"PhaseTemperatureBase="95 C""#));
        assert!(xml.contains(r#"PipingMaterialsClass="C12""#));
        assert!(xml.contains(r#"InsulSpec="H50""#));
        assert!(!xml.contains("Ignored safely"));

        let viols = check_publish_xml_bytes(
            "EXT-145_Data.xml",
            PublishXmlKind::Data,
            &bundle.data_xml,
            default_publish_spec(),
            &CheckOptions {
                expected_plant: Some("P01".to_string()),
                ..Default::default()
            },
        );
        assert!(viols.is_empty(), "StrictVendor violations: {viols:?}");

        // CSV & JSON parsers also work equivalently
        let csv_tbl = parse_external_property_delimited(
            "ItemTag,Description,NominalDiameter\nBV-01,\"Main Isolation, Inlet\",100\n",
            ',',
        )
        .unwrap();
        assert_eq!(csv_tbl.total_rows_loaded, 1);

        let json_tbl = parse_external_property_json(
            r#"{"items":{"P-0201A":{"Description":"Feed Pump","IEquipment.EqTypeDescription":"Centrifugal"}}}"#,
        )
        .unwrap();
        assert_eq!(json_tbl.total_rows_loaded, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

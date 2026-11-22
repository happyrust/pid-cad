//! P&ID Master Equipment Schedule (`HG/T 20559` / `SH/T 3122`), multi-sheet duplicate &
//! untagged equipment verification, cross-discipline nozzle/line/utility/safeguard linkage,
//! and interactive `PIDEQUIP` / CLI `--export-equipment` support (`L3-5 / L3-6 / L2-2 / L6-2` / `#169`).

use std::collections::{BTreeMap, BTreeSet};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::hazop::analyze_pid_hazop_nodes;
use super::line_list::analyze_pid_line_list;
use super::nozzle::analyze_pid_equipment_nozzles;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};
use super::utility::classify_fluid_service;

/// Fixed 13-column CSV/TSV header for `PIDEQUIP EXPORT` and `--export-equipment`.
pub const EQUIPMENT_CSV_HEADERS: [&str; 13] = [
    "EquipmentTag",
    "Description",
    "Category",
    "SppidClass",
    "TagStatus",
    "DrawingCodes",
    "Occurrences",
    "NozzleCount",
    "ConnectedLines",
    "UtilityServices",
    "Safeguards",
    "DesignPressure",
    "DesignTemperature",
];

/// Standard EPC process equipment discipline category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentCategory {
    VesselOrColumn,
    RotatingMachinery,
    HeatTransfer,
    StorageTank,
    PackageOrMisc,
}

impl EquipmentCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VesselOrColumn => "vessel_or_column",
            Self::RotatingMachinery => "rotating_machinery",
            Self::HeatTransfer => "heat_transfer",
            Self::StorageTank => "storage_tank",
            Self::PackageOrMisc => "package_or_misc",
        }
    }
}

/// Tag completeness and cross-sheet uniqueness audit status for one equipment entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentTagStatus {
    Ok,
    Untagged,
    DuplicateAcrossSheets,
}

impl EquipmentTagStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Untagged => "untagged",
            Self::DuplicateAcrossSheets => "duplicate_across_sheets",
        }
    }
}

/// One aggregated equipment entry in the Master Equipment Schedule (`PIDEQUIP`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidEquipmentRow {
    pub equipment_tag: String,
    pub description: String,
    pub category: EquipmentCategory,
    pub sppid_class: String,
    pub tag_status: EquipmentTagStatus,
    pub drawing_codes: Vec<String>,
    pub occurrence_count: usize,
    pub nozzle_count: usize,
    pub connected_lines: Vec<String>,
    pub utility_services: Vec<String>,
    pub relief_or_safeguards: Vec<String>,
    pub design_pressure: String,
    pub design_temperature: String,
    pub primary_tab: usize,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub handles_by_tab: BTreeMap<usize, Vec<Handle>>,
}

/// Summary statistics across all extracted equipment items.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidEquipmentSummary {
    pub sheets_scanned: usize,
    pub total_equipment: usize,
    pub vessels_count: usize,
    pub rotating_count: usize,
    pub heat_transfer_count: usize,
    pub tanks_count: usize,
    pub package_misc_count: usize,
    pub untagged_count: usize,
    pub duplicate_tag_count: usize,
    pub total_connected_nozzles: usize,
}

/// Complete Master Equipment Schedule report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidEquipmentReport {
    pub summary: PidEquipmentSummary,
    pub equipment: Vec<PidEquipmentRow>,
}

struct RawEquipOcc {
    tab_index: usize,
    drawing_code: String,
    tag_opt: Option<String>,
    description: String,
    sppid_class: String,
    category: EquipmentCategory,
    design_pressure: String,
    design_temperature: String,
    primary_handle: Handle,
    member_handles: Vec<Handle>,
    center: (f64, f64),
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

fn parse_pair_f64(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn entity_center(doc: &Document, h: Handle) -> (f64, f64) {
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

fn extract_tag_alpha_prefix(tag: &str) -> String {
    let trimmed = tag.trim();
    for seg in trimmed.split(['-', '_', ' ']).filter(|s| !s.is_empty()) {
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

fn is_equipment_class_or_type(cls: &str, type_code: &str) -> bool {
    if cls == "PIDNozzle" || type_code.eq_ignore_ascii_case("nozzle") {
        return false;
    }
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

/// Classify an equipment symbol into one of the five EPC equipment categories.
pub fn classify_equipment_category(
    cls: &str,
    type_code: &str,
    desc_hint_upper: &str,
    tag_opt: Option<&str>,
) -> EquipmentCategory {
    let pfx = tag_opt.map(extract_tag_alpha_prefix).unwrap_or_default();
    let tc_up = type_code.trim().to_ascii_uppercase();

    if matches!(pfx.as_str(), "TK" | "TANK")
        || tc_up == "TANK"
        || (pfx.is_empty()
            && (desc_hint_upper.contains("STORAGE TANK")
                || desc_hint_upper.contains("CONE ROOF")
                || desc_hint_upper.contains("FLOATING ROOF")
                || desc_hint_upper.contains("TANK")
                || desc_hint_upper.contains("SILO")))
    {
        return EquipmentCategory::StorageTank;
    }

    if matches!(cls, "PIDHeatTransferEquipment" | "PIDHeatTransferEquip")
        || tc_up == "HEAT_EXCHANGER"
        || matches!(pfx.as_str(), "E" | "H" | "A" | "EA" | "WHB")
        || desc_hint_upper.contains("EXCHANG")
        || desc_hint_upper.contains("COOLER")
        || desc_hint_upper.contains("HEATER")
        || desc_hint_upper.contains("CONDENS")
        || desc_hint_upper.contains("REBOIL")
        || desc_hint_upper.contains("FURNACE")
    {
        return EquipmentCategory::HeatTransfer;
    }

    if cls == "PIDMechanicalEquipment"
        || tc_up == "PUMP"
        || matches!(pfx.as_str(), "P" | "K" | "B" | "AG" | "BL" | "MP")
        || desc_hint_upper.contains("PUMP")
        || desc_hint_upper.contains("COMPRESS")
        || desc_hint_upper.contains("BLOWER")
        || desc_hint_upper.contains("FAN")
        || desc_hint_upper.contains("AGITATOR")
        || desc_hint_upper.contains("TURBINE")
        || desc_hint_upper.contains("CENTRIFUG")
    {
        return EquipmentCategory::RotatingMachinery;
    }

    if cls == "PIDProcessVessel"
        || tc_up == "VESSEL"
        || matches!(pfx.as_str(), "V" | "T" | "D" | "R" | "C" | "S" | "KO")
        || desc_hint_upper.contains("VESSEL")
        || desc_hint_upper.contains("DRUM")
        || desc_hint_upper.contains("COLUMN")
        || desc_hint_upper.contains("TOWER")
        || desc_hint_upper.contains("REACTOR")
        || desc_hint_upper.contains("SEPARATOR")
        || desc_hint_upper.contains("SCRUBBER")
        || desc_hint_upper.contains("ACCUMULATOR")
    {
        return EquipmentCategory::VesselOrColumn;
    }

    EquipmentCategory::PackageOrMisc
}

fn pick_map_str(pmap: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for &k in keys {
        if let Some(s) = pmap
            .get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

fn extract_authored_equip_occs(
    tab_index: usize,
    fallback_code: &str,
    doc: &Document,
    out: &mut Vec<RawEquipOcc>,
) {
    let rec_opt = (!has_native_pid_semantics(doc)).then(|| {
        recognise_sppid_sheet(
            doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        )
    });
    let drawing_code = rec_opt
        .as_ref()
        .and_then(|r| r.title_block.drawing_number.as_deref())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback_code)
        .to_string();

    let mut rec_sym_by_h: BTreeMap<Handle, &super::recognize::ConvertedSymbol> = BTreeMap::new();
    if let Some(ref rec) = rec_opt {
        for sym in &rec.symbols {
            if let Some(rh) = sym.entity_handle {
                rec_sym_by_h.insert(Handle::new(rh), sym);
            }
        }
    }

    let mut promoted_handles: BTreeSet<Handle> = BTreeSet::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            promoted_handles.insert(h);
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
            promoted_handles.insert(mh);
        }

        let cls = pmap
            .get("_sppid_class")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&auth.type_code)
            .to_string();
        if !is_equipment_class_or_type(&cls, &auth.type_code) {
            continue;
        }

        let rec_sym = rec_sym_by_h.get(&h).copied();
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
        let desc = pick_map_str(
            &pmap,
            &[
                "Description",
                "description",
                "说明",
                "设备名称",
                "Service",
                "_description",
            ],
        )
        .or_else(|| {
            rec_sym
                .and_then(|s| s.description.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default();
        let design_pressure = pick_map_str(
            &pmap,
            &[
                "DesignPressure",
                "MaxDesignPressure",
                "设计压力",
                "design_pressure",
            ],
        )
        .unwrap_or_default();
        let design_temperature = pick_map_str(
            &pmap,
            &[
                "DesignTemperature",
                "MaxDesignTemperature",
                "设计温度",
                "design_temperature",
            ],
        )
        .unwrap_or_default();

        let tag_opt = (!auth.tag.trim().is_empty()).then(|| auth.tag.trim().to_string());
        let hint_up = format!("{src_blk} {sym_path} {desc}").to_ascii_uppercase();
        let category =
            classify_equipment_category(&cls, &auth.type_code, &hint_up, tag_opt.as_deref());
        let center = pmap
            .get("_at")
            .and_then(Value::as_str)
            .and_then(parse_pair_f64)
            .or_else(|| rec_sym.map(|s| s.at))
            .unwrap_or_else(|| entity_center(doc, h));

        out.push(RawEquipOcc {
            tab_index,
            drawing_code: drawing_code.clone(),
            tag_opt,
            description: desc,
            sppid_class: cls,
            category,
            design_pressure,
            design_temperature,
            primary_handle: h,
            member_handles: members,
            center,
        });
    }

    if let Some(ref rec) = rec_opt {
        for sym in &rec.symbols {
            if !is_equipment_class_or_type(&sym.sppid_class, "") {
                continue;
            }
            let Some(raw_h) = sym
                .entity_handle
                .or_else(|| sym.member_handles.first().copied())
            else {
                continue;
            };
            let primary_h = Handle::new(raw_h);
            if promoted_handles.contains(&primary_h)
                || sym
                    .member_handles
                    .iter()
                    .any(|&mr| promoted_handles.contains(&Handle::new(mr)))
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
            let tag_opt = sym
                .tag
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let desc = sym
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("")
                .to_string();
            let hint_up = format!(
                "{} {} {} {}",
                sym.source_block,
                sym.layer,
                sym.symbol_path.as_deref().unwrap_or(""),
                desc
            )
            .to_ascii_uppercase();
            let category =
                classify_equipment_category(&sym.sppid_class, "", &hint_up, tag_opt.as_deref());

            out.push(RawEquipOcc {
                tab_index,
                drawing_code: drawing_code.clone(),
                tag_opt,
                description: desc,
                sppid_class: sym.sppid_class.clone(),
                category,
                design_pressure: String::new(),
                design_temperature: String::new(),
                primary_handle: primary_h,
                member_handles: members,
                center: sym.at,
            });
        }
    }
}

fn extract_unpromoted_equip_occs(
    tab_index: usize,
    fallback_name: &str,
    doc: &Document,
    out: &mut Vec<RawEquipOcc>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        extract_authored_equip_occs(tab_index, &clean_drawing_stem(fallback_name), &scratch, out);
        return;
    }

    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );
    let drawing_code = rec
        .title_block
        .drawing_number
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| clean_drawing_stem(fallback_name));

    for sym in &rec.symbols {
        if !is_equipment_class_or_type(&sym.sppid_class, "") {
            continue;
        }
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
        let tag_opt = sym
            .tag
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let desc = sym
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();
        let hint_up = format!(
            "{} {} {} {}",
            sym.source_block,
            sym.layer,
            sym.symbol_path.as_deref().unwrap_or(""),
            desc
        )
        .to_ascii_uppercase();
        let category =
            classify_equipment_category(&sym.sppid_class, "", &hint_up, tag_opt.as_deref());

        out.push(RawEquipOcc {
            tab_index,
            drawing_code: drawing_code.clone(),
            tag_opt,
            description: desc,
            sppid_class: sym.sppid_class.clone(),
            category,
            design_pressure: String::new(),
            design_temperature: String::new(),
            primary_handle: primary_h,
            member_handles: members,
            center: sym.at,
        });
    }
}

fn canonical_line_name(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('-');
    parse_sppid_line_number(trimmed)
        .map(|p| p.pipeline_name())
        .unwrap_or_else(|| trimmed.to_string())
}

fn is_relief_valve_tag(tag: &str) -> bool {
    let pfx = extract_tag_alpha_prefix(tag);
    matches!(
        pfx.as_str(),
        "PSV" | "PRV" | "BDV" | "PSE" | "TSV" | "PVSV" | "RV" | "SV"
    )
}

/// Check whether a `PidEquipmentRow` matches a user tag or keyword query.
pub fn equipment_row_matches_query(row: &PidEquipmentRow, query: &str) -> bool {
    let q = query.trim().to_ascii_uppercase();
    if q.is_empty() {
        return false;
    }
    let q_comp: String = q.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let tag_up = row.equipment_tag.to_ascii_uppercase();
    let tag_comp: String = tag_up
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    tag_up.contains(&q)
        || (!q_comp.is_empty() && tag_comp.contains(&q_comp))
        || row.description.to_ascii_uppercase().contains(&q)
}

/// Build the Master Equipment Schedule across `sheets`, linking nozzles, pipelines,
/// utility services, and HAZOP safeguards while auditing untagged & duplicate equipment (`#169`).
pub fn analyze_pid_equipment_schedule(sheets: &[OpcSheetInput<'_>]) -> PidEquipmentReport {
    let mut occs: Vec<RawEquipOcc> = Vec::new();
    for sh in sheets {
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            extract_authored_equip_occs(
                sh.tab_index,
                &clean_drawing_stem(sh.fallback_name),
                sh.document,
                &mut occs,
            );
        } else {
            extract_unpromoted_equip_occs(sh.tab_index, sh.fallback_name, sh.document, &mut occs);
        }
    }

    let nz_rep = analyze_pid_equipment_nozzles(sheets);
    let line_rep = analyze_pid_line_list(sheets);
    let hazop_rep = analyze_pid_hazop_nodes(sheets);

    let mut grouped: BTreeMap<String, Vec<RawEquipOcc>> = BTreeMap::new();
    for occ in occs {
        let key = match occ.tag_opt {
            Some(ref t) => format!("TAG:{}", t.to_ascii_uppercase()),
            None => format!(
                "UNTAG:{:04}:{:X}",
                occ.tab_index,
                occ.primary_handle.value()
            ),
        };
        grouped.entry(key).or_default().push(occ);
    }

    let mut rows: Vec<PidEquipmentRow> = Vec::new();

    for (_key, group) in grouped {
        let first = &group[0];
        let (equipment_tag, tag_status) = match first.tag_opt {
            Some(ref t) => {
                let st = if group.len() > 1 {
                    EquipmentTagStatus::DuplicateAcrossSheets
                } else {
                    EquipmentTagStatus::Ok
                };
                (t.clone(), st)
            }
            None => (
                format!("UNTAGGED-{:X}", first.primary_handle.value()),
                EquipmentTagStatus::Untagged,
            ),
        };

        let mut drawing_set: BTreeSet<String> = BTreeSet::new();
        let mut handles_by_tab: BTreeMap<usize, Vec<Handle>> = BTreeMap::new();
        let mut description = String::new();
        let mut design_pressure = String::new();
        let mut design_temperature = String::new();

        for g in &group {
            drawing_set.insert(g.drawing_code.clone());
            if description.is_empty() && !g.description.is_empty() {
                description = g.description.clone();
            }
            if design_pressure.is_empty() && !g.design_pressure.is_empty() {
                design_pressure = g.design_pressure.clone();
            }
            if design_temperature.is_empty() && !g.design_temperature.is_empty() {
                design_temperature = g.design_temperature.clone();
            }
            let dst = handles_by_tab.entry(g.tab_index).or_default();
            for &mh in &g.member_handles {
                if !dst.contains(&mh) {
                    dst.push(mh);
                }
            }
        }

        let tag_up = equipment_tag.to_ascii_uppercase();
        let mut nozzle_count = 0usize;
        let mut lines_set: BTreeSet<String> = BTreeSet::new();
        let mut utils_set: BTreeSet<String> = BTreeSet::new();
        let mut safeguards_set: BTreeSet<String> = BTreeSet::new();

        for nz in &nz_rep.nozzles {
            let nz_eq_up = nz.equipment_tag.to_ascii_uppercase();
            let matched = if tag_status == EquipmentTagStatus::Untagged {
                nz.tab_index == first.tab_index && nz_eq_up == tag_up
            } else {
                nz_eq_up == tag_up
            };
            if !matched {
                continue;
            }
            nozzle_count += 1;
            if !nz.connected_line.is_empty() {
                lines_set.insert(canonical_line_name(&nz.connected_line));
            }
            if !nz.fluid_code.is_empty() && classify_fluid_service(&nz.fluid_code).is_utility() {
                utils_set.insert(nz.fluid_code.to_ascii_uppercase());
            }
            if let Some(rv) = nz.root_valve.as_deref().filter(|v| is_relief_valve_tag(v)) {
                safeguards_set.insert(rv.to_string());
            }
        }

        if tag_status != EquipmentTagStatus::Untagged {
            for pl in &line_rep.pipelines {
                if pl.from_endpoint.eq_ignore_ascii_case(&equipment_tag)
                    || pl.to_endpoint.eq_ignore_ascii_case(&equipment_tag)
                {
                    lines_set.insert(canonical_line_name(&pl.line_number));
                    if !pl.fluid_code.is_empty()
                        && classify_fluid_service(&pl.fluid_code).is_utility()
                    {
                        utils_set.insert(pl.fluid_code.to_ascii_uppercase());
                    }
                }
            }

            for hn in &hazop_rep.nodes {
                if !hn.equipment_tag.eq_ignore_ascii_case(&equipment_tag) {
                    continue;
                }
                for cl in &hn.connected_pipelines {
                    lines_set.insert(canonical_line_name(cl));
                    if let Some(parsed) = parse_sppid_line_number(cl) {
                        let fc = parsed.fluid_code.to_ascii_uppercase();
                        if !fc.is_empty() && classify_fluid_service(&fc).is_utility() {
                            utils_set.insert(fc);
                        }
                    }
                }
                for sg in &hn.safeguard_instruments {
                    if !sg.trim().is_empty() {
                        safeguards_set.insert(sg.trim().to_string());
                    }
                }
                for v in &hn.valves {
                    if is_relief_valve_tag(v) {
                        safeguards_set.insert(v.trim().to_string());
                    }
                }
            }
        }

        rows.push(PidEquipmentRow {
            equipment_tag,
            description,
            category: first.category,
            sppid_class: first.sppid_class.clone(),
            tag_status,
            drawing_codes: drawing_set.into_iter().collect(),
            occurrence_count: group.len(),
            nozzle_count,
            connected_lines: lines_set.into_iter().collect(),
            utility_services: utils_set.into_iter().collect(),
            relief_or_safeguards: safeguards_set.into_iter().collect(),
            design_pressure,
            design_temperature,
            primary_tab: first.tab_index,
            primary_handle_hex: format!("{:X}", first.primary_handle.value()),
            x: first.center.0,
            y: first.center.1,
            primary_handle: first.primary_handle,
            handles_by_tab,
        });
    }

    rows.sort_by(|a, b| {
        let a_untag = a.tag_status == EquipmentTagStatus::Untagged;
        let b_untag = b.tag_status == EquipmentTagStatus::Untagged;
        a_untag
            .cmp(&b_untag)
            .then_with(|| a.equipment_tag.cmp(&b.equipment_tag))
            .then_with(|| a.primary_tab.cmp(&b.primary_tab))
    });

    let vessels_count = rows
        .iter()
        .filter(|r| r.category == EquipmentCategory::VesselOrColumn)
        .count();
    let rotating_count = rows
        .iter()
        .filter(|r| r.category == EquipmentCategory::RotatingMachinery)
        .count();
    let heat_transfer_count = rows
        .iter()
        .filter(|r| r.category == EquipmentCategory::HeatTransfer)
        .count();
    let tanks_count = rows
        .iter()
        .filter(|r| r.category == EquipmentCategory::StorageTank)
        .count();
    let package_misc_count = rows
        .iter()
        .filter(|r| r.category == EquipmentCategory::PackageOrMisc)
        .count();
    let untagged_count = rows
        .iter()
        .filter(|r| r.tag_status == EquipmentTagStatus::Untagged)
        .count();
    let duplicate_tag_count = rows
        .iter()
        .filter(|r| r.tag_status == EquipmentTagStatus::DuplicateAcrossSheets)
        .count();
    let total_connected_nozzles = rows.iter().map(|r| r.nozzle_count).sum();

    PidEquipmentReport {
        summary: PidEquipmentSummary {
            sheets_scanned: sheets.len(),
            total_equipment: rows.len(),
            vessels_count,
            rotating_count,
            heat_transfer_count,
            tanks_count,
            package_misc_count,
            untagged_count,
            duplicate_tag_count,
            total_connected_nozzles,
        },
        equipment: rows,
    }
}

fn csv_escape_field(val: &str, sep: char) -> String {
    if val.contains(sep) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidEquipmentReport` as delimited `.csv` / `.tsv` text.
pub fn format_pid_equipment_delimited(report: &PidEquipmentReport, sep: char) -> String {
    let mut out = String::new();
    let hdr: Vec<String> = EQUIPMENT_CSV_HEADERS
        .iter()
        .map(|h| csv_escape_field(h, sep))
        .collect();
    out.push_str(&hdr.join(&sep.to_string()));
    out.push('\n');

    for r in &report.equipment {
        let cols = [
            r.equipment_tag.clone(),
            r.description.clone(),
            r.category.as_str().to_string(),
            r.sppid_class.clone(),
            r.tag_status.as_str().to_string(),
            r.drawing_codes.join(";"),
            r.occurrence_count.to_string(),
            r.nozzle_count.to_string(),
            r.connected_lines.join(";"),
            r.utility_services.join(";"),
            r.relief_or_safeguards.join(";"),
            r.design_pressure.clone(),
            r.design_temperature.clone(),
        ];
        let escaped: Vec<String> = cols.iter().map(|c| csv_escape_field(c, sep)).collect();
        out.push_str(&escaped.join(&sep.to_string()));
        out.push('\n');
    }
    out
}

/// Write `PidEquipmentReport` to `.json`, `.csv` (with UTF-8 BOM), or `.tsv` (with UTF-8 BOM).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_equipment_file(
    report: &PidEquipmentReport,
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
            let body = format_pid_equipment_delimited(report, ',');
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            std::fs::write(out_path, bytes).map_err(|e| e.to_string())
        }
        "tsv" => {
            let body = format_pid_equipment_delimited(report, '\t');
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
/// and build a `PidEquipmentReport`.
#[cfg(not(target_arch = "wasm32"))]
pub fn extract_equipment_from_path(input: &std::path::Path) -> Result<PidEquipmentReport, String> {
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
    Ok(analyze_pid_equipment_schedule(&refs))
}

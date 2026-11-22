//! Semantic P&ID sheet revision diff & Engineering Change Notice (ECN/DCN) schedule exporter
//! (`PIDDIFF` & `--diff-with`, `L3-8` / `L2-2` / `L3-6` / `L6-2` / `#154`).
//!
//! Compares a baseline revision against a current revision at the engineering-model level
//! across raw `.dxf`/`.dwg`, `.pid`, and promoted `PID_AUTHOR` sheets, filtering out pure
//! graphical layout nudges while reporting `Added`, `Removed`, and `Modified` equipment,
//! valves, instruments, pipelines, and off-page connectors.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::{Document, Handle};
use serde::{Deserialize, Serialize};

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;

use super::bom::{extract_bom_from_documents, BomScheduleRow, BomSheetInput};
use super::export::ConvertOptions;
use super::external_props::normalize_lookup_key;
use super::find::{search_pid_entities, PidFindKind};
use super::opc::{analyze_open_sheets_opc, OpcConnectorItem, OpcSheetInput};

/// Canonical 9-column header order shared by CSV and TSV revision-diff exports.
pub const DIFF_CSV_HEADERS: [&str; 9] = [
    "ChangeKind",
    "ItemTag",
    "Category",
    "SPPIDClass",
    "ChangedFields",
    "OldSummary",
    "NewSummary",
    "X",
    "Y",
];

/// Classification of one semantic engineering revision difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Added,
    Modified,
    Removed,
}

impl DiffKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Removed => "removed",
        }
    }
}

/// One attribute-level `old -> new` delta inside a `Modified` item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFieldChange {
    pub field: String,
    pub old_value: String,
    pub new_value: String,
}

/// One semantic revision diff entry with optional current-sheet canvas handles & focus point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidDiffItem {
    pub kind: DiffKind,
    pub item_tag: String,
    pub category: String,
    pub sppid_class: String,
    pub field_changes: Vec<DiffFieldChange>,
    pub old_summary: String,
    pub new_summary: String,
    pub primary_handle_hex: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    #[serde(skip)]
    pub primary_handle: Option<Handle>,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
}

/// Aggregated revision comparison counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidDiffSummary {
    pub baseline_drawing: String,
    pub current_drawing: String,
    pub total_changes: usize,
    pub added_count: usize,
    pub removed_count: usize,
    pub modified_count: usize,
    pub unchanged_count: usize,
}

/// Complete semantic diff report between two P&ID sheet revisions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidDiffReport {
    pub summary: PidDiffSummary,
    pub items: Vec<PidDiffItem>,
}

fn cat_sort_rank(cat: &str) -> u8 {
    match cat {
        "Equipment" => 0,
        "Pipeline" => 1,
        "Valve" => 2,
        "Instrument" => 3,
        "OffPageConnector" => 4,
        _ => 5,
    }
}

fn summarize_bom_row(row: &BomScheduleRow) -> String {
    let mut parts = vec![format!("class={}", row.sppid_class)];
    if row.occurrences > 1 {
        parts.push(format!("qty={}", row.occurrences));
    }
    if !row.nominal_diameter.is_empty() {
        parts.push(format!("dn={}", row.nominal_diameter));
    }
    if !row.piping_materials_class.is_empty() {
        parts.push(format!("spec={}", row.piping_materials_class));
    }
    if !row.fluid_code.is_empty() {
        parts.push(format!("fluid={}", row.fluid_code));
    }
    if !row.insul_spec.is_empty() {
        parts.push(format!("insul={}", row.insul_spec));
    }
    if !row.pressure_max.is_empty() || !row.phase_temperature_max.is_empty() {
        parts.push(format!(
            "design={}/{}",
            row.pressure_max, row.phase_temperature_max
        ));
    }
    if !row.description.is_empty() {
        parts.push(format!("desc={}", row.description));
    }
    parts.join(", ")
}

fn compare_bom_rows(old_r: &BomScheduleRow, new_r: &BomScheduleRow) -> Vec<DiffFieldChange> {
    let mut changes = Vec::new();
    let pairs = [
        (
            "LineNumber",
            if old_r.category == "Pipeline"
                && normalize_lookup_key(&old_r.item_tag) != normalize_lookup_key(&new_r.item_tag)
            {
                old_r.item_tag.as_str()
            } else {
                ""
            },
            if old_r.category == "Pipeline"
                && normalize_lookup_key(&old_r.item_tag) != normalize_lookup_key(&new_r.item_tag)
            {
                new_r.item_tag.as_str()
            } else {
                ""
            },
        ),
        (
            "SPPIDClass",
            old_r.sppid_class.as_str(),
            new_r.sppid_class.as_str(),
        ),
        (
            "Description",
            old_r.description.as_str(),
            new_r.description.as_str(),
        ),
        (
            "NominalDiameter",
            old_r.nominal_diameter.as_str(),
            new_r.nominal_diameter.as_str(),
        ),
        (
            "PipingMaterialsClass",
            old_r.piping_materials_class.as_str(),
            new_r.piping_materials_class.as_str(),
        ),
        (
            "FluidCode",
            old_r.fluid_code.as_str(),
            new_r.fluid_code.as_str(),
        ),
        (
            "InsulSpec",
            old_r.insul_spec.as_str(),
            new_r.insul_spec.as_str(),
        ),
        (
            "PressureMax",
            old_r.pressure_max.as_str(),
            new_r.pressure_max.as_str(),
        ),
        (
            "PhaseTemperatureMax",
            old_r.phase_temperature_max.as_str(),
            new_r.phase_temperature_max.as_str(),
        ),
        (
            "PressureBase",
            old_r.pressure_base.as_str(),
            new_r.pressure_base.as_str(),
        ),
        (
            "PhaseTemperatureBase",
            old_r.phase_temperature_base.as_str(),
            new_r.phase_temperature_base.as_str(),
        ),
    ];
    for (field, ov, nv) in pairs {
        if ov.trim() != nv.trim() {
            changes.push(DiffFieldChange {
                field: field.to_string(),
                old_value: ov.trim().to_string(),
                new_value: nv.trim().to_string(),
            });
        }
    }
    if old_r.occurrences != new_r.occurrences {
        changes.push(DiffFieldChange {
            field: "Occurrences".to_string(),
            old_value: old_r.occurrences.to_string(),
            new_value: new_r.occurrences.to_string(),
        });
    }
    changes
}

fn pipeline_core_seq_key(tag: &str) -> Option<(String, String)> {
    let parsed = parse_sppid_line_number(tag)?;
    if parsed.fluid_code.trim().is_empty() || parsed.seq_no.trim().is_empty() {
        return None;
    }
    Some((
        parsed.fluid_code.trim().to_ascii_uppercase(),
        parsed.seq_no.trim().to_ascii_uppercase(),
    ))
}

fn attach_current_canvas_handles(item: &mut PidDiffItem, curr_name: &str, curr_doc: &Document) {
    let want_pipe = item.category == "Pipeline";
    let hits = search_pid_entities(
        &item.item_tag,
        0,
        &[OpcSheetInput {
            tab_index: 0,
            fallback_name: curr_name,
            document: curr_doc,
        }],
    );
    let chosen = hits
        .iter()
        .find(|h| (h.kind == PidFindKind::Pipeline) == want_pipe)
        .or_else(|| hits.first());
    if let Some(hit) = chosen {
        item.primary_handle = Some(hit.primary_handle);
        item.member_handles = hit.member_handles.clone();
        item.primary_handle_hex = Some(hit.primary_handle_hex.clone());
        item.x = Some(hit.x);
        item.y = Some(hit.y);
    }
}

fn norm_opc_str(opt: Option<&str>) -> String {
    opt.map(normalize_lookup_key).unwrap_or_default()
}

fn opc_display_tag(c: &OpcConnectorItem) -> String {
    match (c.target_dwg_raw.as_deref(), c.line_label.as_deref()) {
        (Some(t), Some(l)) if !t.trim().is_empty() && !l.trim().is_empty() => {
            format!("OPC:{t} ({l})")
        }
        (Some(t), _) if !t.trim().is_empty() => format!("OPC:{t}"),
        (_, Some(l)) if !l.trim().is_empty() => format!("OPC:{l}"),
        _ => format!("OPC@{:.0},{:.0}", c.x, c.y),
    }
}

fn summarize_opc(c: &OpcConnectorItem) -> String {
    let tgt = c.target_dwg_raw.as_deref().unwrap_or("-");
    let ln = c.line_label.as_deref().unwrap_or("-");
    format!("target={tgt}, line={ln}")
}

fn diff_opc_lists(
    base_opcs: &[OpcConnectorItem],
    curr_opcs: &[OpcConnectorItem],
    upm: f64,
    unchanged_count: &mut usize,
    out_items: &mut Vec<PidDiffItem>,
) {
    let pos_tol = 35.0 * upm;
    let mut base_used = vec![false; base_opcs.len()];
    let mut curr_used = vec![false; curr_opcs.len()];

    // Pass 1: exact semantic match (same target drawing + same line label or canonical pipeline seq)
    for (ci, cc) in curr_opcs.iter().enumerate() {
        let ct = norm_opc_str(cc.target_dwg_raw.as_deref());
        let cl = norm_opc_str(cc.line_label.as_deref());
        let c_seq = cc.line_label.as_deref().and_then(pipeline_core_seq_key);
        let mut best: Option<(f64, usize)> = None;
        for (bi, bc) in base_opcs.iter().enumerate() {
            if base_used[bi] {
                continue;
            }
            let bt = norm_opc_str(bc.target_dwg_raw.as_deref());
            let bl = norm_opc_str(bc.line_label.as_deref());
            let b_seq = bc.line_label.as_deref().and_then(pipeline_core_seq_key);
            let same_line = bl == cl || (c_seq.is_some() && c_seq == b_seq);
            if bt == ct && same_line && (!ct.is_empty() || !cl.is_empty()) {
                let d = (cc.x - bc.x).hypot(cc.y - bc.y);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, bi));
                }
            }
        }
        if let Some((_, bi)) = best {
            base_used[bi] = true;
            curr_used[ci] = true;
            *unchanged_count += 1;
        }
    }

    // Pass 2: pair modified OPCs occupying the same sheet slot (<=35 mm) or same pipeline sequence
    for (ci, cc) in curr_opcs.iter().enumerate() {
        if curr_used[ci] {
            continue;
        }
        let c_seq = cc.line_label.as_deref().and_then(pipeline_core_seq_key);
        let mut best: Option<(f64, usize)> = None;
        for (bi, bc) in base_opcs.iter().enumerate() {
            if base_used[bi] {
                continue;
            }
            let d = (cc.x - bc.x).hypot(cc.y - bc.y);
            let b_seq = bc.line_label.as_deref().and_then(pipeline_core_seq_key);
            let same_seq = c_seq.is_some() && c_seq == b_seq;
            if (d <= pos_tol || same_seq) && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, bi));
            }
        }
        let Some((_, bi)) = best else {
            continue;
        };
        base_used[bi] = true;
        curr_used[ci] = true;
        let bc = &base_opcs[bi];

        let mut fchanges = Vec::new();
        let ot = bc.target_dwg_raw.as_deref().unwrap_or("").trim();
        let nt = cc.target_dwg_raw.as_deref().unwrap_or("").trim();
        if norm_opc_str(Some(ot)) != norm_opc_str(Some(nt)) {
            fchanges.push(DiffFieldChange {
                field: "TargetDrawing".to_string(),
                old_value: ot.to_string(),
                new_value: nt.to_string(),
            });
        }
        let ol = bc.line_label.as_deref().unwrap_or("").trim();
        let nl = cc.line_label.as_deref().unwrap_or("").trim();
        let b_seq = bc.line_label.as_deref().and_then(pipeline_core_seq_key);
        let same_pipe_seq = c_seq.is_some() && c_seq == b_seq;
        if !same_pipe_seq && norm_opc_str(Some(ol)) != norm_opc_str(Some(nl)) {
            fchanges.push(DiffFieldChange {
                field: "LineLabel".to_string(),
                old_value: ol.to_string(),
                new_value: nl.to_string(),
            });
        }

        if fchanges.is_empty() {
            *unchanged_count += 1;
        } else {
            out_items.push(PidDiffItem {
                kind: DiffKind::Modified,
                item_tag: opc_display_tag(cc),
                category: "OffPageConnector".to_string(),
                sppid_class: "PIDBranchPoint".to_string(),
                field_changes: fchanges,
                old_summary: summarize_opc(bc),
                new_summary: summarize_opc(cc),
                primary_handle_hex: Some(cc.primary_handle_hex.clone()),
                x: Some(cc.x),
                y: Some(cc.y),
                primary_handle: Some(cc.primary_handle),
                member_handles: cc.member_handles.clone(),
            });
        }
    }

    // Pass 3: remaining unmatched OPCs -> Added / Removed
    for (ci, cc) in curr_opcs.iter().enumerate() {
        if !curr_used[ci] {
            out_items.push(PidDiffItem {
                kind: DiffKind::Added,
                item_tag: opc_display_tag(cc),
                category: "OffPageConnector".to_string(),
                sppid_class: "PIDBranchPoint".to_string(),
                field_changes: Vec::new(),
                old_summary: String::new(),
                new_summary: summarize_opc(cc),
                primary_handle_hex: Some(cc.primary_handle_hex.clone()),
                x: Some(cc.x),
                y: Some(cc.y),
                primary_handle: Some(cc.primary_handle),
                member_handles: cc.member_handles.clone(),
            });
        }
    }
    for (bi, bc) in base_opcs.iter().enumerate() {
        if !base_used[bi] {
            out_items.push(PidDiffItem {
                kind: DiffKind::Removed,
                item_tag: opc_display_tag(bc),
                category: "OffPageConnector".to_string(),
                sppid_class: "PIDBranchPoint".to_string(),
                field_changes: Vec::new(),
                old_summary: summarize_opc(bc),
                new_summary: String::new(),
                primary_handle_hex: None,
                x: Some(bc.x),
                y: Some(bc.y),
                primary_handle: None,
                member_handles: Vec::new(),
            });
        }
    }
}

fn overlay_author_props_onto_bom_map(
    doc: &Document,
    map: &mut BTreeMap<(String, String), BomScheduleRow>,
) {
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let nk = normalize_lookup_key(&auth.tag);
        if nk.is_empty() {
            continue;
        }
        let Ok(pmap) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&auth.props)
        else {
            continue;
        };
        for ((_, k_nk), row) in map.iter_mut() {
            if k_nk != &nk {
                continue;
            }
            if let Some(v) = pmap
                .get("说明")
                .or_else(|| pmap.get("Description"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                row.description = v.trim().to_string();
            }
            if let Some(v) = pmap
                .get("公称直径")
                .or_else(|| pmap.get("NominalDiameter"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                row.nominal_diameter = v.trim().to_string();
            }
            if let Some(v) = pmap
                .get("管道等级")
                .or_else(|| pmap.get("PipingMaterialsClass"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                row.piping_materials_class = v.trim().to_string();
            }
            if let Some(v) = pmap
                .get("流体代号")
                .or_else(|| pmap.get("FluidCode"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                row.fluid_code = v.trim().to_string();
            }
            if let Some(v) = pmap
                .get("保温规格")
                .or_else(|| pmap.get("InsulSpec"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                row.insul_spec = v.trim().to_string();
            }
        }
    }
}

/// Compare `base_doc` (Baseline) against `curr_doc` (Current) at the P&ID engineering model level (`#154`).
pub fn diff_pid_documents(
    base_name: &str,
    base_doc: &Document,
    curr_name: &str,
    curr_doc: &Document,
    opts: &ConvertOptions,
) -> Result<PidDiffReport, String> {
    // Use a common canonical sheet stem during BOM extraction so filename suffixes
    // (e.g. `*-R0.dxf` vs `*-RA.dxf`) never create synthetic pipeline-name mismatches.
    let base_bom = extract_bom_from_documents(
        &[BomSheetInput {
            drawing_name: "DIFF_SHEET",
            document: base_doc,
        }],
        opts,
    )?;
    let curr_bom = extract_bom_from_documents(
        &[BomSheetInput {
            drawing_name: "DIFF_SHEET",
            document: curr_doc,
        }],
        opts,
    )?;

    let mut base_map: BTreeMap<(String, String), BomScheduleRow> = BTreeMap::new();
    for r in base_bom.rows {
        let nk = normalize_lookup_key(&r.item_tag);
        if !nk.is_empty() {
            base_map.insert((r.category.clone(), nk), r);
        }
    }
    let mut curr_map: BTreeMap<(String, String), BomScheduleRow> = BTreeMap::new();
    for r in curr_bom.rows {
        let nk = normalize_lookup_key(&r.item_tag);
        if !nk.is_empty() {
            curr_map.insert((r.category.clone(), nk), r);
        }
    }
    overlay_author_props_onto_bom_map(base_doc, &mut base_map);
    overlay_author_props_onto_bom_map(curr_doc, &mut curr_map);

    let mut matched_base: BTreeSet<(String, String)> = BTreeSet::new();
    let mut matched_curr: BTreeSet<(String, String)> = BTreeSet::new();
    let mut items: Vec<PidDiffItem> = Vec::new();
    let mut unchanged_count = 0usize;

    // 1. Exact `(category, normalized_tag)` match
    for (k, curr_r) in &curr_map {
        let Some(base_r) = base_map.get(k) else {
            continue;
        };
        matched_base.insert(k.clone());
        matched_curr.insert(k.clone());
        let fchanges = compare_bom_rows(base_r, curr_r);
        if fchanges.is_empty() {
            unchanged_count += 1;
        } else {
            let mut di = PidDiffItem {
                kind: DiffKind::Modified,
                item_tag: curr_r.item_tag.clone(),
                category: curr_r.category.clone(),
                sppid_class: curr_r.sppid_class.clone(),
                field_changes: fchanges,
                old_summary: summarize_bom_row(base_r),
                new_summary: summarize_bom_row(curr_r),
                primary_handle_hex: None,
                x: None,
                y: None,
                primary_handle: None,
                member_handles: Vec::new(),
            };
            attach_current_canvas_handles(&mut di, curr_name, curr_doc);
            items.push(di);
        }
    }

    // 2. Pair unmatched Pipelines sharing canonical `(fluid_code, seq_no)` (e.g. spec/DN upgrade)
    let mut unmatched_base_pipes: BTreeMap<(String, String), Vec<(String, String)>> =
        BTreeMap::new();
    for (k, r) in &base_map {
        if r.category == "Pipeline" && !matched_base.contains(k) {
            if let Some(seq_k) = pipeline_core_seq_key(&r.item_tag) {
                unmatched_base_pipes
                    .entry(seq_k)
                    .or_default()
                    .push(k.clone());
            }
        }
    }
    let mut unmatched_curr_pipes: BTreeMap<(String, String), Vec<(String, String)>> =
        BTreeMap::new();
    for (k, r) in &curr_map {
        if r.category == "Pipeline" && !matched_curr.contains(k) {
            if let Some(seq_k) = pipeline_core_seq_key(&r.item_tag) {
                unmatched_curr_pipes
                    .entry(seq_k)
                    .or_default()
                    .push(k.clone());
            }
        }
    }
    for (seq_k, c_keys) in &unmatched_curr_pipes {
        let Some(b_keys) = unmatched_base_pipes.get(seq_k) else {
            continue;
        };
        if c_keys.len() == 1 && b_keys.len() == 1 {
            let bk = &b_keys[0];
            let ck = &c_keys[0];
            let base_r = &base_map[bk];
            let curr_r = &curr_map[ck];
            matched_base.insert(bk.clone());
            matched_curr.insert(ck.clone());
            let fchanges = compare_bom_rows(base_r, curr_r);
            if fchanges.is_empty() {
                unchanged_count += 1;
            } else {
                let mut di = PidDiffItem {
                    kind: DiffKind::Modified,
                    item_tag: curr_r.item_tag.clone(),
                    category: curr_r.category.clone(),
                    sppid_class: curr_r.sppid_class.clone(),
                    field_changes: fchanges,
                    old_summary: summarize_bom_row(base_r),
                    new_summary: summarize_bom_row(curr_r),
                    primary_handle_hex: None,
                    x: None,
                    y: None,
                    primary_handle: None,
                    member_handles: Vec::new(),
                };
                attach_current_canvas_handles(&mut di, curr_name, curr_doc);
                items.push(di);
            }
        }
    }

    // 3. Remaining unmatched BOM rows -> Added / Removed
    for (k, curr_r) in &curr_map {
        if matched_curr.contains(k) {
            continue;
        }
        let mut di = PidDiffItem {
            kind: DiffKind::Added,
            item_tag: curr_r.item_tag.clone(),
            category: curr_r.category.clone(),
            sppid_class: curr_r.sppid_class.clone(),
            field_changes: Vec::new(),
            old_summary: String::new(),
            new_summary: summarize_bom_row(curr_r),
            primary_handle_hex: None,
            x: None,
            y: None,
            primary_handle: None,
            member_handles: Vec::new(),
        };
        attach_current_canvas_handles(&mut di, curr_name, curr_doc);
        items.push(di);
    }
    for (k, base_r) in &base_map {
        if matched_base.contains(k) {
            continue;
        }
        items.push(PidDiffItem {
            kind: DiffKind::Removed,
            item_tag: base_r.item_tag.clone(),
            category: base_r.category.clone(),
            sppid_class: base_r.sppid_class.clone(),
            field_changes: Vec::new(),
            old_summary: summarize_bom_row(base_r),
            new_summary: String::new(),
            primary_handle_hex: None,
            x: None,
            y: None,
            primary_handle: None,
            member_handles: Vec::new(),
        });
    }

    // 4. Off-Page Connectors (`PIDBranchPoint`) semantic diff
    let base_opc_rep = analyze_open_sheets_opc(&[OpcSheetInput {
        tab_index: 0,
        fallback_name: base_name,
        document: base_doc,
    }]);
    let curr_opc_rep = analyze_open_sheets_opc(&[OpcSheetInput {
        tab_index: 0,
        fallback_name: curr_name,
        document: curr_doc,
    }]);
    let upm = crate::io::pid_legend::guess_units_per_mm(curr_doc).max(1e-6);
    diff_opc_lists(
        &base_opc_rep.connectors,
        &curr_opc_rep.connectors,
        upm,
        &mut unchanged_count,
        &mut items,
    );

    items.sort_by(|a, b| {
        (a.kind, cat_sort_rank(&a.category), &a.item_tag).cmp(&(
            b.kind,
            cat_sort_rank(&b.category),
            &b.item_tag,
        ))
    });

    let added_count = items.iter().filter(|i| i.kind == DiffKind::Added).count();
    let modified_count = items
        .iter()
        .filter(|i| i.kind == DiffKind::Modified)
        .count();
    let removed_count = items.iter().filter(|i| i.kind == DiffKind::Removed).count();

    Ok(PidDiffReport {
        summary: PidDiffSummary {
            baseline_drawing: base_name.to_string(),
            current_drawing: curr_name.to_string(),
            total_changes: items.len(),
            added_count,
            removed_count,
            modified_count,
            unchanged_count,
        },
        items,
    })
}

/// Load two CAD/PID files from disk and compute their semantic revision diff (`#154`).
#[cfg(not(target_arch = "wasm32"))]
pub fn diff_pid_files(
    base_path: &Path,
    curr_path: &Path,
    opts: &ConvertOptions,
) -> Result<PidDiffReport, String> {
    let base_doc = crate::io::load_file(base_path)
        .map_err(|e| format!("load baseline {}: {e}", base_path.display()))?;
    let curr_doc = crate::io::load_file(curr_path)
        .map_err(|e| format!("load current {}: {e}", curr_path.display()))?;
    let b_name = base_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("baseline");
    let c_name = curr_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("current");
    diff_pid_documents(b_name, &base_doc, c_name, &curr_doc, opts)
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidDiffReport` as delimited ECN/DCN schedule text (CSV or TSV) with UTF-8 BOM.
pub fn format_pid_diff_delimited(report: &PidDiffReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let header_line: Vec<String> = DIFF_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&header_line.join(&delim.to_string()));
    out.push_str("\r\n");

    for it in &report.items {
        let changed_fields = it
            .field_changes
            .iter()
            .map(|fc| format!("{}:{}->{}", fc.field, fc.old_value, fc.new_value))
            .collect::<Vec<_>>()
            .join("; ");
        let xs = it.x.map(|v| format!("{v:.1}")).unwrap_or_default();
        let ys = it.y.map(|v| format!("{v:.1}")).unwrap_or_default();
        let cells = [
            it.kind.as_str(),
            it.item_tag.as_str(),
            it.category.as_str(),
            it.sppid_class.as_str(),
            changed_fields.as_str(),
            it.old_summary.as_str(),
            it.new_summary.as_str(),
            xs.as_str(),
            ys.as_str(),
        ];
        let escaped: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&escaped.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidDiffReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#154`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_diff_file(report: &PidDiffReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize diff JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_diff_delimited(report, '\t'),
        "csv" => format_pid_diff_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported diff output extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

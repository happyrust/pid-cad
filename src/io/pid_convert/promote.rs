//! Zero-conflict `PID_AUTHOR` promotion, `TopologyTable` synchronization,
//! post-edit cleanup, and save-back-to-`.pid` engine (`L2-1 M1..M5` / `L2-5` /
//! `L2-6` / `L2-7` / `L6-2` / `#134`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use document::xdata::XDataValue;
use document::{Document, EntityType, Handle};
use serde::{Deserialize, Serialize};

use super::class_map::default_legend_class_map;
#[cfg(not(target_arch = "wasm32"))]
use super::export::{
    compute_representation_uid_map, load_configured_seed, pick_seed_style_ids,
    resolve_effective_class_map, verify_dual_bundle, DualConvertBundle,
};
use super::export::{ConvertOptions, ConvertSheetReport, DeadEndAttribution, PipeNetworkCensus};
use super::recognize::{recognise_sppid_sheet, ConfidenceSummary, ConfidenceTier, TitleBlockInfo};
use crate::io::pid_legend::guess_units_per_mm;
use crate::io::pid_publish::{
    default_class_map, export_drawing_bundle_with_map, parse_sppid_line_number, pipeline_uid,
    populate_item_tag_attributes, synthesize_piping_network, ExportDrawing, ExportItem,
    PipeRunInput, SymbolEndpointRef, UidAllocator, VariantResolutionSummary,
};
use crate::io::PID_SEMANTICS_XDATA_APP;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{
    handle_text, parse_handle, ConnectorKind, ConnectorRecord, FittingRecord, SegmentEnd,
    TopologyTable, ROLE_CONNECTOR_IN, ROLE_CONNECTOR_OUT, ROLE_FITTING, ROLE_PIPE_SEGMENT,
    TYPE_PIPE_SEGMENT, VALVE_PORT_NAMES,
};

/// Summary of promoting recognized or imported P&ID entities onto `PID_AUTHOR` (`M1` / `M2` / `L2-6`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteReport {
    pub newly_promoted: usize,
    pub updated_in_place: usize,
    pub manual_tags_preserved: usize,
    pub pipe_lines_promoted: usize,
    pub pipe_runs_promoted: usize,
    pub pipe_segments_promoted: usize,
    pub fittings_promoted: usize,
    pub connectors_promoted: usize,
    pub unmapped_classes: BTreeMap<String, usize>,
}

/// Summary of synchronizing `PID_AUTHOR` and `TopologyTable` after CAD edits (`L2-4` / `L2-5`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorSyncReport {
    pub pruned_records: usize,
    pub cleared_dangling_peers: usize,
    pub updated_moved_endpoints: usize,
    pub duplicated_tags: usize,
}

/// Sheet-level three-tier confidence and review status of `PID_AUTHOR` entities (`L3-7` / `#136`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorReviewStats {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub confirmed: usize,
    pub manual_tags: usize,
    pub duplicated_tags: usize,
    pub total_authored_symbols: usize,
}

/// Summarize `PID_AUTHOR` review buckets across `doc` (`L3-7` / `#136`).
pub fn summarize_author_review(doc: &Document) -> AuthorReviewStats {
    let mut stats = AuthorReviewStats::default();
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            continue;
        }
        stats.total_authored_symbols += 1;
        if auth.tag_source == "manual" {
            stats.manual_tags += 1;
        }
        if auth.tag_status == "duplicated" {
            stats.duplicated_tags += 1;
        }
        let pmap = parse_props_map(&auth.props);
        match pmap.get("_confidence_tier").and_then(|v| v.as_str()) {
            Some("confirmed") => stats.confirmed += 1,
            Some("medium") => stats.medium += 1,
            Some("low") => stats.low += 1,
            _ => stats.high += 1,
        }
    }
    stats
}

/// Filter `PID_AUTHOR` symbol handles on `doc` by review bucket (`HIGH`, `MEDIUM`, `LOW`, `CONFIRMED`, `DUPLICATES`, `ALL`).
pub fn filter_author_handles(doc: &Document, filter: &str) -> Vec<Handle> {
    let key = filter.trim().to_ascii_uppercase();
    let mut out = Vec::new();
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            continue;
        }
        let pmap = parse_props_map(&auth.props);
        let tier = pmap
            .get("_confidence_tier")
            .and_then(|v| v.as_str())
            .unwrap_or("high");
        let matched = match key.as_str() {
            "ALL" => true,
            "HIGH" => tier.eq_ignore_ascii_case("high"),
            "MEDIUM" | "MED" => tier.eq_ignore_ascii_case("medium"),
            "LOW" => tier.eq_ignore_ascii_case("low"),
            "CONFIRMED" => tier.eq_ignore_ascii_case("confirmed"),
            "DUPLICATES" | "DUP" | "DUPLICATED" => auth.tag_status == "duplicated",
            _ => false,
        };
        if matched {
            out.push(h);
        }
    }
    out
}

/// Expand primary `PID_AUTHOR` handles to include any multi-entity `_member_handles` on `doc` (`#139`).
pub fn expand_author_member_handles(doc: &Document, handles: &[Handle]) -> Vec<Handle> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for &h in handles {
        if seen.insert(h) {
            out.push(h);
        }
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap = parse_props_map(&auth.props);
        if let Some(raw_mem) = pmap.get("_member_handles").and_then(|v| v.as_str()) {
            for hex in raw_mem.split(',') {
                if let Ok(v) = u64::from_str_radix(hex.trim(), 16) {
                    let mh = Handle::new(v);
                    if doc.get_entity(mh).is_some() && seen.insert(mh) {
                        out.push(mh);
                    }
                }
            }
        }
    }
    out
}

/// Resolve selected entity handles (including individual strokes of an exploded symbol)
/// back to their owning primary `PID_AUTHOR` handles (`#139`).
pub fn resolve_owning_author_handles(doc: &Document, selected: &[Handle]) -> Vec<Handle> {
    let mut member_to_owner: BTreeMap<Handle, Handle> = BTreeMap::new();
    for ent in doc.model_space_entities() {
        let oh = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, oh) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            continue;
        }
        let pmap = parse_props_map(&auth.props);
        if let Some(raw_mem) = pmap.get("_member_handles").and_then(|v| v.as_str()) {
            for hex in raw_mem.split(',') {
                if let Ok(v) = u64::from_str_radix(hex.trim(), 16) {
                    member_to_owner.entry(Handle::new(v)).or_insert(oh);
                }
            }
        }
    }

    let mut seen = BTreeSet::new();
    let mut owners = Vec::new();
    for &h in selected {
        let target = if PidAuthor::load(doc, h).is_some() {
            Some(h)
        } else {
            member_to_owner.get(&h).copied()
        };
        if let Some(th) = target {
            if seen.insert(th) {
                owners.push(th);
            }
        }
    }
    owners
}

/// Disambiguate duplicate `PID_AUTHOR` symbol tags in deterministic handle order (`PIDREVIEW DEDUP` / `#139`).
pub fn deduplicate_author_tags(doc: &mut Document) -> usize {
    let handles: Vec<Handle> = doc
        .model_space_entities()
        .map(|e| e.common().handle)
        .collect();
    let mut seen_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut used_tags: BTreeSet<(String, String)> = BTreeSet::new();
    for &h in &handles {
        if let Some(auth) = PidAuthor::load(doc, h) {
            if !auth.tag.is_empty()
                && !matches!(
                    auth.topology.as_str(),
                    ROLE_PIPE_SEGMENT | ROLE_CONNECTOR_IN | ROLE_CONNECTOR_OUT
                )
            {
                used_tags.insert((auth.type_code, auth.tag));
            }
        }
    }

    let mut renamed = 0usize;
    for h in handles {
        let Some(mut auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.tag.is_empty()
            || matches!(
                auth.topology.as_str(),
                ROLE_PIPE_SEGMENT | ROLE_CONNECTOR_IN | ROLE_CONNECTOR_OUT
            )
        {
            continue;
        }
        let key = (auth.type_code.clone(), auth.tag.clone());
        let occ = seen_counts.entry(key).or_default();
        *occ += 1;
        if *occ == 1 {
            continue;
        }
        let mut suffix = *occ;
        let new_tag = loop {
            let cand = format!("{}-{suffix}", auth.tag);
            if used_tags.insert((auth.type_code.clone(), cand.clone())) {
                break cand;
            }
            suffix += 1;
        };
        auth.tag = new_tag;
        auth.tag_source = "manual".to_string();
        auth.tag_status = "ok".to_string();
        auth.store(doc, h);
        renamed += 1;
    }
    let _ = refresh_duplicate_tag_statuses(doc);
    renamed
}

/// Mark `handles` as human-reviewed (`tag_source = "manual"`, `_confidence_tier = "confirmed"`).
pub fn accept_author_handles(doc: &mut Document, handles: &[Handle]) -> usize {
    let owners = resolve_owning_author_handles(doc, handles);
    let mut count = 0usize;
    for h in owners {
        let Some(mut auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            continue;
        }
        auth.tag_source = "manual".to_string();
        let mut pmap = parse_props_map(&auth.props);
        pmap.insert(
            "_confidence_tier".to_string(),
            serde_json::Value::String("confirmed".to_string()),
        );
        auth.props = serde_json::Value::Object(pmap).to_string();
        auth.store(doc, h);
        count += 1;
    }
    let _ = refresh_duplicate_tag_statuses(doc);
    count
}

/// Remove `PID_AUTHOR` from `handles` and prune any topology bindings referencing them (`L3-7` REJECT).
pub fn reject_author_handles(doc: &mut Document, handles: &[Handle]) -> (usize, AuthorSyncReport) {
    let owners = resolve_owning_author_handles(doc, handles);
    let mut removed = 0usize;
    for h in owners {
        if PidAuthor::load(doc, h).is_some() {
            crate::scene::view::dispatch::set_entity_xdata(
                doc,
                h,
                crate::pid::persist::PID_AUTHOR_APP,
                None,
            );
            removed += 1;
        }
    }
    let sync = sync_author_after_edit(doc);
    (removed, sync)
}

fn set_text_entity_value(doc: &mut Document, h: Handle, desired: &str) -> bool {
    let needs_update = match doc.get_entity(h) {
        Some(EntityType::Text(t)) => t.value != desired,
        Some(EntityType::MText(mt)) => mt.value != desired,
        _ => false,
    };
    if !needs_update {
        return false;
    }
    match doc.get_entity_mut(h) {
        Some(EntityType::Text(t)) => {
            t.value = desired.to_string();
            true
        }
        Some(EntityType::MText(mt)) => {
            mt.value = desired.to_string();
            true
        }
        _ => false,
    }
}

/// Synchronize paired DWG/DXF `TEXT` / `MTEXT` entities with current `PID_AUTHOR` tags (`L2-4`).
pub fn sync_paired_canvas_tag_texts(doc: &mut Document) -> Vec<Handle> {
    let mut updates: Vec<(Vec<Handle>, String, String)> = Vec::new();
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap = parse_props_map(&auth.props);
        let Some(raw_th) = pmap.get("_tag_handles").and_then(|v| v.as_str()) else {
            continue;
        };
        let ths: Vec<Handle> = raw_th
            .split(',')
            .filter_map(|hex| u64::from_str_radix(hex.trim(), 16).ok().map(Handle::new))
            .filter(|&th| th != h)
            .collect();
        if ths.is_empty() {
            continue;
        }
        updates.push((ths, auth.tag, auth.tag_status));
    }

    let mut modified = Vec::new();
    for (ths, tag, status) in updates {
        let duplicated = status == "duplicated";
        if ths.len() == 2 {
            let (top_val, bot_val) = if duplicated {
                ("？".to_string(), String::new())
            } else if let Some((l, r)) = tag.split_once('-') {
                (l.to_string(), r.to_string())
            } else {
                (tag, String::new())
            };
            if set_text_entity_value(doc, ths[0], &top_val) {
                modified.push(ths[0]);
            }
            if set_text_entity_value(doc, ths[1], &bot_val) {
                modified.push(ths[1]);
            }
        } else {
            let shown = if duplicated { "？".to_string() } else { tag };
            if set_text_entity_value(doc, ths[0], &shown) {
                modified.push(ths[0]);
            }
        }
    }
    modified
}

fn tier_rank(t: ConfidenceTier) -> u8 {
    match t {
        ConfidenceTier::High => 2,
        ConfidenceTier::Medium => 1,
        ConfidenceTier::Low => 0,
    }
}

/// Map an SPPID class (plus optional block hint) to a canonical `PID_AUTHOR` `type` code (`M1`).
pub fn sppid_class_to_author_type(sppid_class: &str, block_hint: &str) -> Option<&'static str> {
    let hint = block_hint.to_ascii_uppercase();
    match sppid_class {
        "PIDProcessVessel" => Some("VESSEL"),
        "PIDMechanicalEquipment" => {
            if hint.contains("PUMP") {
                Some("PUMP")
            } else {
                Some("EQUIPMENT")
            }
        }
        "PIDProcessEquipment" => {
            if hint.contains("TANK") {
                Some("TANK")
            } else {
                Some("EQUIPMENT")
            }
        }
        "PIDHeatTransferEquipment" => Some("HEAT_EXCHANGER"),
        "PIDPipingComponent" => {
            if hint.contains("VALVE") || hint.contains("COCK") || hint.contains("DAMPER") {
                Some("VALVE")
            } else {
                Some("FITTING")
            }
        }
        "PIDNozzle" => Some("NOZZLE"),
        "PIDInstrument" => Some("INSTRUMENT"),
        "PIDInlineInstrument" => Some("INLINE_INSTRUMENT"),
        "PIDControlSystemFunction" => Some("CONTROL_FUNCTION"),
        "PIDBranchPoint" | "PIDPipeline" => Some("PIPE_LINE"),
        "PIDPipingConnector" => Some("PIPE_SEGMENT"),
        "PIDSignalConnector" => Some("SIGNAL_LINE"),
        "PIDNote" => Some("NOTE"),
        _ => None,
    }
}

fn author_type_to_sppid_fallback(type_code: &str) -> Option<(&'static str, &'static str)> {
    match type_code.trim().to_ascii_uppercase().as_str() {
        "VESSEL" => Some(("PIDProcessVessel", "PIDProcessVessel#01")),
        "PUMP" => Some(("PIDMechanicalEquipment", "PIDMechanicalEquipment#01")),
        "TANK" | "EQUIPMENT" => Some(("PIDProcessEquipment", "PIDProcessEquipment#01")),
        "HEAT_EXCHANGER" => Some(("PIDHeatTransferEquipment", "PIDHeatTransferEquipment#01")),
        "VALVE" | "FITTING" => Some(("PIDPipingComponent", "PIDPipingComponent#01")),
        "NOZZLE" => Some(("PIDNozzle", "PIDNozzle#01")),
        "INSTRUMENT" => Some(("PIDInstrument", "PIDInstrument#01")),
        "INLINE_INSTRUMENT" => Some(("PIDInlineInstrument", "PIDInlineInstrument#01")),
        "CONTROL_FUNCTION" => Some(("PIDControlSystemFunction", "PIDControlSystemFunction#01")),
        "NOTE" => Some(("PIDNote", "PIDNote#01")),
        _ => None,
    }
}

fn parse_props_map(raw: &str) -> serde_json::Map<String, serde_json::Value> {
    if raw.trim().is_empty() {
        return serde_json::Map::new();
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

fn point_to_seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let l2 = vx * vx + vy * vy;
    if l2 <= 1e-12 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / l2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * vx)).hypot(p.1 - (a.1 + t * vy))
}

fn refresh_duplicate_tag_statuses(doc: &mut Document) -> usize {
    let handles: Vec<Handle> = doc
        .model_space_entities()
        .map(|e| e.common().handle)
        .collect();
    let mut loaded: Vec<(Handle, PidAuthor)> = Vec::new();
    let mut tag_counts: BTreeMap<(String, String), usize> = BTreeMap::new();

    for h in handles {
        if let Some(auth) = PidAuthor::load(doc, h) {
            if !auth.tag.is_empty()
                && !matches!(
                    auth.topology.as_str(),
                    ROLE_PIPE_SEGMENT | ROLE_CONNECTOR_IN | ROLE_CONNECTOR_OUT
                )
            {
                *tag_counts
                    .entry((auth.type_code.clone(), auth.tag.clone()))
                    .or_default() += 1;
            }
            loaded.push((h, auth));
        }
    }

    let mut dup_entities = 0usize;
    for (h, mut auth) in loaded {
        if auth.tag.is_empty()
            || matches!(
                auth.topology.as_str(),
                ROLE_PIPE_SEGMENT | ROLE_CONNECTOR_IN | ROLE_CONNECTOR_OUT
            )
        {
            if auth.tag_status != "ok" {
                auth.tag_status = "ok".to_string();
                auth.store(doc, h);
            }
            continue;
        }
        let cnt = tag_counts
            .get(&(auth.type_code.clone(), auth.tag.clone()))
            .copied()
            .unwrap_or(0);
        let desired = if cnt > 1 {
            dup_entities += 1;
            "duplicated"
        } else {
            "ok"
        };
        if auth.tag_status != desired {
            auth.tag_status = desired.to_string();
            auth.store(doc, h);
        }
    }
    dup_entities
}

/// Idempotently promote DWG/DXF recognition results on `doc` into `PID_AUTHOR` + `TopologyTable` (`M1..M3`).
pub fn promote_sheet_to_author(
    doc: &mut Document,
    opts: &ConvertOptions,
) -> Result<PromoteReport, String> {
    let map = default_legend_class_map();
    let rec = recognise_sppid_sheet(doc, map, opts.thresholds);
    let upm = rec.units_per_mm.max(1e-6);
    let snap_tol = opts.snap_tol_mm * upm;

    let mut report = PromoteReport::default();

    struct PromotedSym {
        handle: Handle,
        sppid_class: String,
        source_block: String,
        tag: Option<String>,
        to_from_text: Option<String>,
        ports: Vec<(f64, f64)>,
    }
    let mut promoted_syms: Vec<PromotedSym> = Vec::new();
    let mut symbol_member_handles: BTreeSet<Handle> = BTreeSet::new();

    for sym in &rec.symbols {
        if !sym.known || tier_rank(sym.tier) < tier_rank(opts.min_tier) {
            if !sym.known {
                *report
                    .unmapped_classes
                    .entry(sym.source_block.clone())
                    .or_default() += 1;
            }
            continue;
        }
        let Some(type_code) = sppid_class_to_author_type(&sym.sppid_class, &sym.source_block)
        else {
            *report
                .unmapped_classes
                .entry(sym.sppid_class.clone())
                .or_default() += 1;
            continue;
        };
        let Some(raw_h) = sym.entity_handle else {
            continue;
        };
        let handle = Handle::new(raw_h);
        if doc.get_entity(handle).is_none() {
            continue;
        }
        symbol_member_handles.insert(handle);
        for &mh in &sym.member_handles {
            symbol_member_handles.insert(Handle::new(mh));
        }

        let existing = PidAuthor::load(doc, handle);
        let is_update = existing.is_some();
        let mut author = existing.unwrap_or_default();

        author.type_code = type_code.to_string();
        if author.tag_source == "manual" {
            report.manual_tags_preserved += 1;
        } else {
            author.tag = sym.tag.clone().unwrap_or_default();
            author.tag_source = "auto".to_string();
        }
        if author.tag_status.is_empty() {
            author.tag_status = "ok".to_string();
        }

        let mut pmap = parse_props_map(&author.props);
        let already_confirmed = pmap
            .get("_confidence_tier")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "confirmed");
        pmap.insert(
            "_source".to_string(),
            serde_json::Value::String("convert:dxf".to_string()),
        );
        pmap.insert(
            "_confidence".to_string(),
            serde_json::Value::String(format!("{:.2}", sym.confidence)),
        );
        if !already_confirmed {
            pmap.insert("_confidence_tier".to_string(), serde_json::json!(sym.tier));
        }
        pmap.insert(
            "_sppid_class".to_string(),
            serde_json::Value::String(sym.sppid_class.clone()),
        );
        pmap.insert(
            "_at".to_string(),
            serde_json::Value::String(format!("{:.4},{:.4}", sym.at.0, sym.at.1)),
        );
        pmap.insert(
            "_bbox".to_string(),
            serde_json::Value::String(format!(
                "{:.4},{:.4},{:.4},{:.4}",
                sym.bbox.0, sym.bbox.1, sym.bbox.2, sym.bbox.3
            )),
        );
        if sym.member_handles.len() > 1 {
            let mem_hex = sym
                .member_handles
                .iter()
                .map(|mh| format!("{mh:X}"))
                .collect::<Vec<_>>()
                .join(",");
            pmap.insert(
                "_member_handles".to_string(),
                serde_json::Value::String(mem_hex),
            );
        }
        if !sym.tag_handles.is_empty() {
            let joined = sym
                .tag_handles
                .iter()
                .map(|th| format!("{th:X}"))
                .collect::<Vec<_>>()
                .join(",");
            pmap.insert(
                "_tag_handles".to_string(),
                serde_json::Value::String(joined),
            );
        }
        if let Some(ref vid) = sym.variant_id {
            pmap.insert(
                "_variant_id".to_string(),
                serde_json::Value::String(vid.clone()),
            );
        }
        if let Some(ref sp) = sym.symbol_path {
            pmap.insert(
                "_symbol_path".to_string(),
                serde_json::Value::String(sp.clone()),
            );
        }
        if let Some(ref tf) = sym.to_from_text {
            pmap.insert(
                "_to_from_text".to_string(),
                serde_json::Value::String(tf.clone()),
            );
            pmap.entry("对端图号".to_string())
                .or_insert_with(|| serde_json::Value::String(tf.clone()));
        }
        if let Some(ref nd) = sym.nominal_diameter {
            pmap.insert(
                "_nominal_diameter".to_string(),
                serde_json::Value::String(nd.clone()),
            );
            pmap.entry("公称直径".to_string())
                .or_insert_with(|| serde_json::Value::String(nd.clone()));
        }
        if let Some(ref desc) = sym.description {
            pmap.entry("说明".to_string())
                .or_insert_with(|| serde_json::Value::String(desc.clone()));
        }
        author.props = serde_json::Value::Object(pmap).to_string();

        if matches!(
            sym.sppid_class.as_str(),
            "PIDPipingComponent" | "PIDInlineInstrument"
        ) {
            author.topology = ROLE_FITTING.to_string();
            author.ports = VALVE_PORT_NAMES.join(",");
        } else if sym.sppid_class == "PIDBranchPoint" {
            let is_in = sym
                .description
                .as_deref()
                .is_some_and(|d| d.to_ascii_uppercase().contains("FROM") || d.contains("来自"));
            author.topology = if is_in {
                ROLE_CONNECTOR_IN.to_string()
            } else {
                ROLE_CONNECTOR_OUT.to_string()
            };
            author.ports = "连接点1".to_string();
        } else if author.ports.is_empty() && !sym.ports.is_empty() {
            author.ports = "连接点1,连接点2".to_string();
        }

        author.store(doc, handle);
        if is_update {
            report.updated_in_place += 1;
        } else {
            report.newly_promoted += 1;
        }

        promoted_syms.push(PromotedSym {
            handle,
            sppid_class: sym.sppid_class.clone(),
            source_block: sym.source_block.clone(),
            tag: if author.tag.is_empty() {
                None
            } else {
                Some(author.tag.clone())
            },
            to_from_text: sym.to_from_text.clone(),
            ports: sym.ports.clone(),
        });
    }

    // Build M3 `TopologyTable` from pipe line entities on `opts.pipe_layers`
    // (or from `rec.pipe_segments` for native AutoCAD `LwPolyline` / custom pipe layers),
    // strictly excluding member strokes of promoted symbols (`#139`).
    let pipe_layer_set: BTreeSet<String> = opts
        .pipe_layers
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .collect();
    let mut raw_lines = Vec::new();
    for ent in doc.model_space_entities() {
        let eh = ent.common().handle;
        if symbol_member_handles.contains(&eh) {
            continue;
        }
        let lu = ent.common().layer.trim().to_ascii_uppercase();
        let on_pipe = pipe_layer_set.contains(&lu)
            || (pipe_layer_set.contains("PIPE") && lu.starts_with("PIPE"));
        if !on_pipe {
            continue;
        }
        if let EntityType::Line(ln) = ent {
            let a = (ln.start.x, ln.start.y);
            let b = (ln.end.x, ln.end.y);
            if (a.0 - b.0).hypot(a.1 - b.1) >= 2.0 * upm {
                raw_lines.push((eh, a, b));
            }
        }
    }

    if raw_lines.is_empty() && !rec.pipe_segments.is_empty() {
        let fallback_pipe_h = doc.model_space_entities().find_map(|ent| {
            let eh = ent.common().handle;
            if symbol_member_handles.contains(&eh) {
                return None;
            }
            matches!(ent, EntityType::Line(_) | EntityType::LwPolyline(_)).then_some(eh)
        });
        for ps in &rec.pipe_segments {
            if (ps.start.0 - ps.end.0).hypot(ps.start.1 - ps.end.1) < 2.0 * upm {
                continue;
            }
            let cand_h = ps
                .entity_handle
                .map(Handle::new)
                .filter(|h| !symbol_member_handles.contains(h) && doc.get_entity(*h).is_some())
                .or(fallback_pipe_h);
            if let Some(eh) = cand_h {
                raw_lines.push((eh, ps.start, ps.end));
            }
        }
    }

    let mut topo = TopologyTable::default();
    let mut line_groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let default_line_label = rec
        .pipe_labels
        .first()
        .map(|l| l.raw_text.clone())
        .unwrap_or_else(|| "50-PG-0001-B9-".to_string());

    for &(lh, a, b) in &raw_lines {
        let mut best_lbl: Option<(f64, &str)> = None;
        for lb in &rec.pipe_labels {
            let d = point_to_seg_dist(lb.at, a, b);
            if d <= 40.0 * upm && best_lbl.is_none_or(|(bd, _)| d < bd) {
                best_lbl = Some((d, lb.raw_text.as_str()));
            }
        }
        let chosen = best_lbl
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| default_line_label.clone());
        line_groups.entry(chosen).or_default().push((lh, a, b));
    }

    let snap_endpoint = |pt: (f64, f64)| -> SegmentEnd {
        let mut best: Option<(f64, Handle, usize)> = None;
        for ps in &promoted_syms {
            for (pi, &sp) in ps.ports.iter().enumerate() {
                let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                if d <= snap_tol && best.is_none_or(|(bd, _, _)| d < bd) {
                    best = Some((d, ps.handle, pi));
                }
            }
        }
        if let Some((_, sh, pi)) = best {
            let port_name = if pi % 2 == 0 {
                VALVE_PORT_NAMES[0]
            } else {
                VALVE_PORT_NAMES[1]
            };
            SegmentEnd::joined([pt.0, pt.1], sh, port_name)
        } else {
            SegmentEnd::open([pt.0, pt.1])
        }
    };

    let mut fitting_bound_segs: BTreeMap<(Handle, String), String> = BTreeMap::new();
    let mut per_handle_subsegs: BTreeMap<Handle, Vec<String>> = BTreeMap::new();

    for (line_tag, segs) in &line_groups {
        let Some(&(first_h, _, _)) = segs.first() else {
            continue;
        };
        let mut line_props = BTreeMap::new();
        if let Some(parsed) = parse_sppid_line_number(line_tag) {
            line_props.insert("size".to_string(), parsed.nominal_diameter_attr);
            line_props.insert("fluid".to_string(), parsed.fluid_code);
            line_props.insert("spec".to_string(), parsed.materials_class);
        }
        topo.append_line(line_tag, first_h, line_props.clone());
        report.pipe_lines_promoted += 1;

        let run_tag = format!("{line_tag}-01");
        topo.append_run(line_tag, &run_tag, first_h, line_props, Vec::new());
        report.pipe_runs_promoted += 1;

        for &(sh, a, b) in segs {
            let start_end = snap_endpoint(a);
            let end_end = snap_endpoint(b);
            for se in [&start_end, &end_end] {
                if let Some(ref peer_str) = se.peer {
                    if let Some((h_str, p_name)) = peer_str.split_once(':') {
                        if let Some(sym_h) = parse_handle(h_str) {
                            fitting_bound_segs
                                .entry((sym_h, p_name.to_string()))
                                .or_insert_with(|| handle_text(sh));
                        }
                    }
                }
            }
            topo.append_segment(&run_tag, sh, start_end, end_end);
            report.pipe_segments_promoted += 1;
            per_handle_subsegs.entry(sh).or_default().push(format!(
                "{:.4},{:.4},{:.4},{:.4}|{}",
                a.0, a.1, b.0, b.1, line_tag
            ));
        }
    }

    for (sh, encoded_list) in per_handle_subsegs {
        let mut seg_auth = PidAuthor::load(doc, sh).unwrap_or_default();
        seg_auth.type_code = TYPE_PIPE_SEGMENT.to_string();
        seg_auth.topology = ROLE_PIPE_SEGMENT.to_string();
        let mut smap = parse_props_map(&seg_auth.props);
        if let Some(first_enc) = encoded_list.first() {
            if let Some((_, lbl)) = first_enc.split_once('|') {
                smap.insert(
                    "_line_label".to_string(),
                    serde_json::Value::String(lbl.to_string()),
                );
            }
        }
        let non_line = !matches!(doc.get_entity(sh), Some(EntityType::Line(_)));
        if non_line || encoded_list.len() > 1 {
            smap.insert(
                "_segments".to_string(),
                serde_json::Value::String(encoded_list.join(";")),
            );
        } else {
            smap.remove("_segments");
        }
        seg_auth.props = serde_json::Value::Object(smap).to_string();
        seg_auth.store(doc, sh);
    }

    let first_run_tag = topo
        .pipe_runs
        .first()
        .map(|r| r.tag.clone())
        .unwrap_or_default();
    let first_line_tag = topo
        .pipe_lines
        .first()
        .map(|l| l.tag.clone())
        .unwrap_or_default();

    for ps in &promoted_syms {
        if matches!(
            ps.sppid_class.as_str(),
            "PIDPipingComponent" | "PIDInlineInstrument"
        ) {
            let mut ports_map = BTreeMap::new();
            for pname in VALVE_PORT_NAMES {
                let bound = fitting_bound_segs
                    .get(&(ps.handle, pname.to_string()))
                    .cloned();
                ports_map.insert(pname.to_string(), bound);
            }
            topo.fittings.push(FittingRecord {
                handle: handle_text(ps.handle),
                run_tag: first_run_tag.clone(),
                fitting_type: ps.source_block.to_ascii_lowercase().replace(' ', "_"),
                tag: ps.tag.clone(),
                ports: ports_map,
            });
            report.fittings_promoted += 1;
        } else if ps.sppid_class == "PIDBranchPoint" {
            let mut rec_conn = ConnectorRecord::new(
                handle_text(ps.handle),
                ConnectorKind::PipeOut,
                first_line_tag.clone(),
            );
            rec_conn.peer_drawing = ps.to_from_text.clone();
            topo.upsert_connector(rec_conn);
            report.connectors_promoted += 1;
        }
    }

    topo.store(doc);
    let _ = refresh_duplicate_tag_statuses(doc);
    Ok(report)
}

fn infer_sppid_from_semantics(kv: &BTreeMap<String, String>) -> (String, String) {
    if let Some(cls) = kv.get("class") {
        if cls.starts_with("PID") {
            let hint = kv.get("instance").map(String::as_str).unwrap_or("");
            let auth_ty = sppid_class_to_author_type(cls, hint).unwrap_or("EQUIPMENT");
            return (cls.clone(), auth_ty.to_string());
        }
    }
    let inst = kv
        .get("instance")
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();
    if inst.contains("VESSEL") || inst.contains("DRUM") || inst.contains("COLUMN") {
        ("PIDProcessVessel".to_string(), "VESSEL".to_string())
    } else if inst.contains("PUMP") || inst.contains("COMPRESSOR") {
        ("PIDMechanicalEquipment".to_string(), "PUMP".to_string())
    } else if inst.contains("VALVE") {
        ("PIDPipingComponent".to_string(), "VALVE".to_string())
    } else if inst.contains("INSTRUMENT") {
        ("PIDInstrument".to_string(), "INSTRUMENT".to_string())
    } else {
        ("PIDProcessEquipment".to_string(), "EQUIPMENT".to_string())
    }
}

/// Whether `doc` carries native `.pid`-imported `PID_SEMANTICS` records (as opposed to `legend:*` hits) (`L2-6` / `#142`).
pub fn has_native_pid_semantics(doc: &Document) -> bool {
    for ent in doc.model_space_entities() {
        let Some(rec) = ent
            .common()
            .extended_data
            .get_record(PID_SEMANTICS_XDATA_APP)
        else {
            continue;
        };
        let mut has_kv = false;
        let mut is_legend = false;
        for v in &rec.values {
            if let XDataValue::String(s) = v {
                if s.contains('=') {
                    has_kv = true;
                }
                if s.starts_with("resolved=legend:") {
                    is_legend = true;
                }
            }
        }
        if has_kv && !is_legend {
            return true;
        }
    }
    false
}

fn entity_geom_bounds(ent: &EntityType) -> Option<(f64, f64, f64, f64)> {
    match ent {
        EntityType::Insert(ins) => {
            let (x, y) = (ins.insert_point.x, ins.insert_point.y);
            Some((x - 3.0, y - 3.0, x + 3.0, y + 3.0))
        }
        EntityType::Line(ln) => Some((
            ln.start.x.min(ln.end.x),
            ln.start.y.min(ln.end.y),
            ln.start.x.max(ln.end.x),
            ln.start.y.max(ln.end.y),
        )),
        EntityType::Circle(c) => Some((
            c.center.x - c.radius,
            c.center.y - c.radius,
            c.center.x + c.radius,
            c.center.y + c.radius,
        )),
        EntityType::Arc(a) => Some((
            a.center.x - a.radius,
            a.center.y - a.radius,
            a.center.x + a.radius,
            a.center.y + a.radius,
        )),
        EntityType::LwPolyline(pl) if !pl.vertices.is_empty() => {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for v in &pl.vertices {
                min_x = min_x.min(v.location.x);
                min_y = min_y.min(v.location.y);
                max_x = max_x.max(v.location.x);
                max_y = max_y.max(v.location.y);
            }
            Some((min_x, min_y, max_x, max_y))
        }
        EntityType::Point(p) => Some((p.location.x, p.location.y, p.location.x, p.location.y)),
        _ => None,
    }
}

fn polyline_or_line_segments(ent: &EntityType) -> Vec<((f64, f64), (f64, f64))> {
    match ent {
        EntityType::Line(ln) => vec![((ln.start.x, ln.start.y), (ln.end.x, ln.end.y))],
        EntityType::LwPolyline(pl) if pl.vertices.len() >= 2 => {
            let mut out = Vec::with_capacity(pl.vertices.len());
            for win in pl.vertices.windows(2) {
                out.push((
                    (win[0].location.x, win[0].location.y),
                    (win[1].location.x, win[1].location.y),
                ));
            }
            if pl.is_closed && pl.vertices.len() >= 3 {
                let last = pl.vertices.last().unwrap();
                let first = &pl.vertices[0];
                out.push((
                    (last.location.x, last.location.y),
                    (first.location.x, first.location.y),
                ));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn resolve_catalog_symbol_variant(
    explicit_cls: Option<&str>,
    stem_hint: Option<&str>,
    sym_path_hint: Option<&str>,
) -> (String, String, Option<String>) {
    let legend_map = default_legend_class_map();
    let class_map = default_class_map();

    if let Some(stem) = stem_hint.map(str::trim).filter(|s| !s.is_empty()) {
        if let super::class_map::BlockResolution::Mapped(mb) = legend_map.resolve_block(stem, "") {
            if explicit_cls.is_none_or(|c| c == mb.class) {
                return (
                    mb.class,
                    mb.variant_id,
                    mb.symbol_path.or_else(|| sym_path_hint.map(str::to_string)),
                );
            }
        }
    }

    let cls = explicit_cls.unwrap_or("PIDProcessEquipment").to_string();
    if let Some(stem) = stem_hint.map(str::trim).filter(|s| !s.is_empty()) {
        let stem_up = stem.to_ascii_uppercase();
        if let Some(v) = class_map.variants.iter().find(|v| {
            v.class == cls
                && v.symbol_paths.iter().any(|sp| {
                    std::path::Path::new(&sp.replace('\\', "/"))
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|st| st.trim().eq_ignore_ascii_case(&stem_up))
                })
        }) {
            return (
                cls,
                v.variant_id.clone(),
                v.symbol_paths
                    .first()
                    .cloned()
                    .or_else(|| sym_path_hint.map(str::to_string)),
            );
        }
    }

    if let Some(v) = class_map.variants.iter().find(|v| v.class == cls) {
        let sp = sym_path_hint
            .map(str::to_string)
            .or_else(|| v.symbol_paths.first().cloned());
        return (cls, v.variant_id.clone(), sp);
    }

    (
        cls.clone(),
        format!("{cls}#01"),
        sym_path_hint.map(str::to_string),
    )
}

/// Promote imported `.pid` entities carrying `PID_SEMANTICS` into `PID_AUTHOR` (`L2-6` / `#142`).
///
/// Multi-stroke `.pid` symbol placements sharing the same `oid` are merged onto one
/// primary handle (recording `_member_handles`, `_tag_handles`, `_bbox`, and `_at`),
/// matched against `LegendClassMap` / `ClassMapSpec` via `PID-SYMBOL-LABEL` stems for
/// zero-fallback `variant_id` resolution, and `.pid` `Line`/`LwPolyline` pipes are
/// promoted into `PIPE_SEGMENT` + `TopologyTable`.
pub fn promote_semantics_to_author(
    doc: &mut Document,
    filter_handles: Option<&BTreeSet<Handle>>,
) -> PromoteReport {
    struct RawSemEnt {
        handle: Handle,
        kv: BTreeMap<String, String>,
        is_text: bool,
        is_sym_label: bool,
        text_val: Option<String>,
        bounds: Option<(f64, f64, f64, f64)>,
        anchor: Option<(f64, f64)>,
        segs: Vec<((f64, f64), (f64, f64))>,
        is_poly: bool,
    }

    let mut raw_ents: Vec<RawSemEnt> = Vec::new();
    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        if let Some(allowed) = filter_handles {
            if !allowed.contains(&h) {
                continue;
            }
        }
        let Some(rec) = ent
            .common()
            .extended_data
            .get_record(PID_SEMANTICS_XDATA_APP)
        else {
            continue;
        };
        let mut kv = BTreeMap::new();
        for v in &rec.values {
            if let XDataValue::String(s) = v {
                if let Some((k, val)) = s.split_once('=') {
                    kv.insert(k.to_string(), val.to_string());
                }
            }
        }
        if kv.is_empty() {
            continue;
        }
        if kv.get("resolved").is_some_and(|r| r.starts_with("legend:")) {
            continue;
        }
        let layer_up = ent.common().layer.trim().to_ascii_uppercase();
        let role = kv.get("role").map(String::as_str).unwrap_or("");
        let is_sym_label = role == "symbol-label" || layer_up == "PID-SYMBOL-LABEL";
        let (is_text, text_val) = match ent {
            EntityType::Text(t) => (true, Some(t.value.clone())),
            EntityType::MText(mt) => (true, Some(mt.value.clone())),
            _ => (false, None),
        };
        raw_ents.push(RawSemEnt {
            handle: h,
            kv,
            is_text,
            is_sym_label,
            text_val,
            bounds: entity_geom_bounds(ent),
            anchor: entity_world_anchor(ent),
            segs: polyline_or_line_segments(ent),
            is_poly: matches!(ent, EntityType::LwPolyline(_)),
        });
    }

    // Group entities that carry an `oid`; keep `oid`-less entities as singleton groups
    let mut group_order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, item) in raw_ents.iter().enumerate() {
        let key = match item.kv.get("oid").map(String::as_str) {
            Some(oid) if !oid.trim().is_empty() => format!("oid:{}", oid.trim()),
            _ => format!("single:{:X}", item.handle.value()),
        };
        if !groups.contains_key(&key) {
            group_order.push(key.clone());
        }
        groups.entry(key).or_default().push(idx);
    }

    type Seg2d = ((f64, f64), (f64, f64));
    struct SemPipeItem {
        handle: Handle,
        segs: Vec<Seg2d>,
        is_poly: bool,
    }
    fn segs_total_len(segs: &[Seg2d]) -> f64 {
        segs.iter()
            .map(|&(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
            .sum()
    }

    let mut report = PromoteReport::default();
    let mut standalone_label_texts: BTreeMap<(String, String), Vec<Handle>> = BTreeMap::new();
    let mut promoted_sym_owners: Vec<(Handle, Vec<(f64, f64)>)> = Vec::new();
    let mut pipe_candidates: Vec<(String, SemPipeItem)> = Vec::new();

    // First pass: collect standalone text-only `oid` groups so they can bind to matching symbol owners
    for key in &group_order {
        let indices = &groups[key];
        let rep_kv = &raw_ents[indices[0]].kv;
        let cls = rep_kv.get("class").map(String::as_str).unwrap_or("");
        if matches!(
            cls,
            "PIDPipingPort"
                | "PIDSignalPort"
                | "PIDProcessPoint"
                | "PIDPipingBranchPoint"
                | "PIDSignalBranchPoint"
                | "PIDDrawing"
                | "Plant"
                | "PIDAreaBreak"
                | "PIDHistory"
                | "PIDSignalConnector"
        ) {
            continue;
        }
        let has_geom = indices
            .iter()
            .any(|&i| !raw_ents[i].is_text && !raw_ents[i].is_sym_label);
        if !has_geom && cls != "PIDNote" && !cls.is_empty() {
            if let Some(lbl) = rep_kv.get("label").filter(|s| !s.trim().is_empty()) {
                for &i in indices {
                    if raw_ents[i].is_text && !raw_ents[i].is_sym_label {
                        standalone_label_texts
                            .entry((cls.to_string(), lbl.clone()))
                            .or_default()
                            .push(raw_ents[i].handle);
                    }
                }
            }
        }
    }

    for key in &group_order {
        let indices = &groups[key];
        let rep_kv = &raw_ents[indices[0]].kv;
        let cls = rep_kv.get("class").map(String::as_str).unwrap_or("");
        let role0 = rep_kv.get("role").map(String::as_str).unwrap_or("");

        if matches!(
            cls,
            "PIDPipingPort"
                | "PIDSignalPort"
                | "PIDProcessPoint"
                | "PIDPipingBranchPoint"
                | "PIDSignalBranchPoint"
                | "PIDDrawing"
                | "Plant"
                | "PIDAreaBreak"
                | "PIDHistory"
                | "PIDSignalConnector"
        ) || matches!(
            role0,
            "frame"
                | "point-ok"
                | "point-warning"
                | "point-error"
                | "point-approved"
                | "connectivity"
        ) {
            continue;
        }

        // Process pipe group (`PIDPipingConnector` / `PIDPipeline`)
        if matches!(cls, "PIDPipingConnector" | "PIDPipeline") {
            let best_pipe_idx = indices
                .iter()
                .copied()
                .filter(|&i| !raw_ents[i].segs.is_empty() && !raw_ents[i].is_sym_label)
                .max_by(|&a, &b| {
                    let pa = raw_ents[a].is_poly;
                    let pb = raw_ents[b].is_poly;
                    pa.cmp(&pb).then_with(|| {
                        segs_total_len(&raw_ents[a].segs)
                            .total_cmp(&segs_total_len(&raw_ents[b].segs))
                    })
                });
            if let Some(pi) = best_pipe_idx {
                let line_lbl = rep_kv
                    .get("label")
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| "50-PG-0001-B9-".to_string());
                pipe_candidates.push((
                    line_lbl,
                    SemPipeItem {
                        handle: raw_ents[pi].handle,
                        segs: raw_ents[pi].segs.clone(),
                        is_poly: raw_ents[pi].is_poly,
                    },
                ));
            }
            continue;
        }

        let geom_indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| !raw_ents[i].is_text && !raw_ents[i].is_sym_label)
            .collect();

        let primary_idx = if let Some(&gi) = geom_indices.first() {
            gi
        } else if cls == "PIDNote" {
            match indices
                .iter()
                .copied()
                .find(|&i| raw_ents[i].is_text && !raw_ents[i].is_sym_label)
            {
                Some(ti) => ti,
                None => continue,
            }
        } else {
            continue;
        };

        let primary_h = raw_ents[primary_idx].handle;
        let sym_label_stem = indices.iter().find_map(|&i| {
            raw_ents[i]
                .is_sym_label
                .then(|| {
                    raw_ents[i]
                        .text_val
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .to_string()
                })
                .filter(|s| !s.is_empty())
        });

        let inst_raw = indices
            .iter()
            .find_map(|&i| raw_ents[i].kv.get("instance").cloned());
        let inst_sym_path = inst_raw
            .as_deref()
            .filter(|s| s.trim().to_ascii_lowercase().ends_with(".sym"));
        let inst_stem = inst_sym_path.and_then(|sp| {
            std::path::Path::new(&sp.replace('\\', "/"))
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        });

        let inferred_cls_opt = if !cls.is_empty() && cls.starts_with("PID") {
            Some(cls.to_string())
        } else if sym_label_stem.is_none() && inst_stem.is_none() {
            Some(infer_sppid_from_semantics(rep_kv).0)
        } else {
            None
        };

        let stem_ref = sym_label_stem.as_deref().or(inst_stem.as_deref());
        let (sppid_cls, variant_id, resolved_sym_path) =
            resolve_catalog_symbol_variant(inferred_cls_opt.as_deref(), stem_ref, inst_sym_path);
        let auth_ty = sppid_class_to_author_type(&sppid_cls, stem_ref.unwrap_or(""))
            .unwrap_or("EQUIPMENT")
            .to_string();

        let label_opt = indices
            .iter()
            .find_map(|&i| raw_ents[i].kv.get("label").cloned())
            .filter(|s| !s.trim().is_empty());
        let oid_opt = indices
            .iter()
            .find_map(|&i| raw_ents[i].kv.get("oid").cloned())
            .filter(|s| !s.trim().is_empty());

        let mut union_bbox: Option<(f64, f64, f64, f64)> = None;
        for &gi in &geom_indices {
            if let Some(b) = raw_ents[gi].bounds {
                union_bbox = Some(match union_bbox {
                    Some(u) => (u.0.min(b.0), u.1.min(b.1), u.2.max(b.2), u.3.max(b.3)),
                    None => b,
                });
            }
        }
        let bbox = union_bbox.unwrap_or_else(|| {
            let a = raw_ents[primary_idx].anchor.unwrap_or((50.0, 50.0));
            (a.0 - 3.0, a.1 - 3.0, a.0 + 3.0, a.1 + 3.0)
        });
        let center_at = ((bbox.0 + bbox.2) * 0.5, (bbox.1 + bbox.3) * 0.5);

        let mut tag_handles: Vec<Handle> = indices
            .iter()
            .copied()
            .filter(|&i| {
                raw_ents[i].is_text && !raw_ents[i].is_sym_label && raw_ents[i].handle != primary_h
            })
            .map(|i| raw_ents[i].handle)
            .collect();
        if tag_handles.is_empty() {
            if let Some(ref lbl) = label_opt {
                if let Some(extra) =
                    standalone_label_texts.remove(&(sppid_cls.clone(), lbl.clone()))
                {
                    tag_handles.extend(extra);
                }
            }
        }

        let existing = PidAuthor::load(doc, primary_h);
        let is_update = existing.is_some();
        let mut author = existing.unwrap_or_default();
        author.type_code = auth_ty.clone();

        if author.tag_source == "manual" {
            report.manual_tags_preserved += 1;
        } else if let Some(ref lbl) = label_opt {
            author.tag = lbl.clone();
            author.tag_source = "auto".to_string();
        } else if author.tag_source.is_empty() {
            author.tag_source = "auto".to_string();
        }
        if author.tag_status.is_empty() {
            author.tag_status = "ok".to_string();
        }

        if matches!(
            sppid_cls.as_str(),
            "PIDPipingComponent" | "PIDInlineInstrument"
        ) {
            author.topology = ROLE_FITTING.to_string();
            author.ports = VALVE_PORT_NAMES.join(",");
        } else if sppid_cls == "PIDBranchPoint" {
            author.topology = ROLE_CONNECTOR_OUT.to_string();
            author.ports = "连接点1".to_string();
        } else if author.ports.is_empty() {
            author.ports = "连接点1,连接点2".to_string();
        }

        let mut pmap = parse_props_map(&author.props);
        pmap.insert(
            "_source".to_string(),
            serde_json::Value::String("import:pid".to_string()),
        );
        pmap.insert(
            "_confidence".to_string(),
            serde_json::Value::String("1.00".to_string()),
        );
        if !pmap
            .get("_confidence_tier")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "confirmed")
        {
            pmap.insert(
                "_confidence_tier".to_string(),
                serde_json::Value::String("high".to_string()),
            );
        }
        pmap.insert(
            "_sppid_class".to_string(),
            serde_json::Value::String(sppid_cls.clone()),
        );
        pmap.insert(
            "_variant_id".to_string(),
            serde_json::Value::String(variant_id),
        );
        pmap.insert(
            "_at".to_string(),
            serde_json::Value::String(format!("{:.4},{:.4}", center_at.0, center_at.1)),
        );
        pmap.insert(
            "_bbox".to_string(),
            serde_json::Value::String(format!(
                "{:.4},{:.4},{:.4},{:.4}",
                bbox.0, bbox.1, bbox.2, bbox.3
            )),
        );
        if geom_indices.len() > 1 {
            let mem_hex = geom_indices
                .iter()
                .map(|&gi| format!("{:X}", raw_ents[gi].handle.value()))
                .collect::<Vec<_>>()
                .join(",");
            pmap.insert(
                "_member_handles".to_string(),
                serde_json::Value::String(mem_hex),
            );
        }
        if !tag_handles.is_empty() {
            let th_hex = tag_handles
                .iter()
                .map(|th| format!("{:X}", th.value()))
                .collect::<Vec<_>>()
                .join(",");
            pmap.insert(
                "_tag_handles".to_string(),
                serde_json::Value::String(th_hex),
            );
        }
        if let Some(oid_str) = oid_opt {
            pmap.insert(
                "_graphic_oid".to_string(),
                serde_json::Value::String(oid_str),
            );
        }
        if let Some(sp) = resolved_sym_path {
            pmap.insert("_symbol_path".to_string(), serde_json::Value::String(sp));
        }
        if sppid_cls == "PIDBranchPoint" {
            if let Some(ref tf) = label_opt {
                pmap.entry("对端图号".to_string())
                    .or_insert_with(|| serde_json::Value::String(tf.clone()));
            }
        }
        author.props = serde_json::Value::Object(pmap).to_string();
        author.store(doc, primary_h);

        if is_update {
            report.updated_in_place += 1;
        } else {
            report.newly_promoted += 1;
        }

        let ports = vec![
            (bbox.0, center_at.1),
            (bbox.2, center_at.1),
            (center_at.0, bbox.1),
            (center_at.0, bbox.3),
            center_at,
        ];
        promoted_sym_owners.push((primary_h, ports));
    }

    if !pipe_candidates.is_empty() {
        let upm = guess_units_per_mm(doc).max(1e-6);
        let snap_tol = 4.0 * upm;
        let mut topo = TopologyTable::load(doc);
        let mut by_line: BTreeMap<String, Vec<SemPipeItem>> = BTreeMap::new();
        for (lbl, pitem) in pipe_candidates {
            by_line.entry(lbl).or_default().push(pitem);
        }

        let snap_pt = |pt: (f64, f64)| -> SegmentEnd {
            let mut best: Option<(f64, Handle, usize)> = None;
            for (sh, ports) in &promoted_sym_owners {
                for (pi, &sp) in ports.iter().enumerate() {
                    let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                    if d <= snap_tol && best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, *sh, pi));
                    }
                }
            }
            if let Some((_, sh, pi)) = best {
                let pname = if pi % 2 == 0 {
                    VALVE_PORT_NAMES[0]
                } else {
                    VALVE_PORT_NAMES[1]
                };
                SegmentEnd::joined([pt.0, pt.1], sh, pname)
            } else {
                SegmentEnd::open([pt.0, pt.1])
            }
        };

        for (line_tag, items) in &by_line {
            let Some(first_item) = items.first() else {
                continue;
            };
            let first_h = first_item.handle;
            let mut lprops = BTreeMap::new();
            if let Some(parsed) = parse_sppid_line_number(line_tag) {
                lprops.insert("size".to_string(), parsed.nominal_diameter_attr);
                lprops.insert("fluid".to_string(), parsed.fluid_code);
                lprops.insert("spec".to_string(), parsed.materials_class);
            }
            if topo.pipe_lines.iter().all(|l| l.tag != *line_tag) {
                topo.append_line(line_tag, first_h, lprops.clone());
                report.pipe_lines_promoted += 1;
            }
            let run_tag = format!("{line_tag}-01");
            if topo.pipe_runs.iter().all(|r| r.tag != run_tag) {
                topo.append_run(line_tag, &run_tag, first_h, lprops, Vec::new());
                report.pipe_runs_promoted += 1;
            }

            for pitem in items {
                let ph = pitem.handle;
                let segs = &pitem.segs;
                let is_poly = pitem.is_poly;
                let mut enc = Vec::new();
                for &(a, b) in segs {
                    if (a.0 - b.0).hypot(a.1 - b.1) < 0.2 * upm {
                        continue;
                    }
                    topo.append_segment(&run_tag, ph, snap_pt(a), snap_pt(b));
                    report.pipe_segments_promoted += 1;
                    enc.push(format!(
                        "{:.4},{:.4},{:.4},{:.4}|{}",
                        a.0, a.1, b.0, b.1, line_tag
                    ));
                }
                if !enc.is_empty() {
                    let mut sa = PidAuthor::load(doc, ph).unwrap_or_default();
                    sa.type_code = TYPE_PIPE_SEGMENT.to_string();
                    sa.topology = ROLE_PIPE_SEGMENT.to_string();
                    let mut smap = parse_props_map(&sa.props);
                    smap.insert(
                        "_line_label".to_string(),
                        serde_json::Value::String(line_tag.clone()),
                    );
                    if is_poly || enc.len() > 1 {
                        smap.insert(
                            "_segments".to_string(),
                            serde_json::Value::String(enc.join(";")),
                        );
                    }
                    sa.props = serde_json::Value::Object(smap).to_string();
                    sa.store(doc, ph);
                }
            }
        }
        topo.store(doc);
    }

    let _ = refresh_duplicate_tag_statuses(doc);
    report
}

fn entity_world_anchor(ent: &EntityType) -> Option<(f64, f64)> {
    match ent {
        EntityType::Insert(ins) => Some((ins.insert_point.x, ins.insert_point.y)),
        EntityType::Circle(c) => Some((c.center.x, c.center.y)),
        EntityType::Arc(a) => Some((a.center.x, a.center.y)),
        EntityType::Text(t) => Some((t.insertion_point.x, t.insertion_point.y)),
        EntityType::MText(mt) => Some((mt.insertion_point.x, mt.insertion_point.y)),
        EntityType::Point(p) => Some((p.location.x, p.location.y)),
        EntityType::Line(ln) => {
            Some(((ln.start.x + ln.end.x) * 0.5, (ln.start.y + ln.end.y) * 0.5))
        }
        EntityType::LwPolyline(pl) if !pl.vertices.is_empty() => {
            let n = pl.vertices.len() as f64;
            let sx: f64 = pl.vertices.iter().map(|v| v.location.x).sum();
            let sy: f64 = pl.vertices.iter().map(|v| v.location.y).sum();
            Some((sx / n, sy / n))
        }
        _ => None,
    }
}

fn parse_pair_f64(raw: &str) -> Option<(f64, f64)> {
    let mut it = raw.split(',');
    let x = it.next()?.trim().parse::<f64>().ok()?;
    let y = it.next()?.trim().parse::<f64>().ok()?;
    Some((x, y))
}

fn parse_quad_f64(raw: &str) -> Option<(f64, f64, f64, f64)> {
    let mut it = raw.split(',');
    let a = it.next()?.trim().parse::<f64>().ok()?;
    let b = it.next()?.trim().parse::<f64>().ok()?;
    let c = it.next()?.trim().parse::<f64>().ok()?;
    let d = it.next()?.trim().parse::<f64>().ok()?;
    Some((a, b, c, d))
}

/// Reconcile `TopologyTable` and `PID_AUTHOR` tag uniqueness after moving/deleting/editing entities (`L2-4 / L2-5`).
pub fn sync_author_after_edit(doc: &mut Document) -> AuthorSyncReport {
    let mut topo = TopologyTable::load(doc);
    let mut pruned_records = topo.prune(doc);

    let before_fit = topo.fittings.len();
    topo.fittings
        .retain(|f| parse_handle(&f.handle).is_some_and(|h| PidAuthor::load(doc, h).is_some()));
    pruned_records += before_fit.saturating_sub(topo.fittings.len());

    let before_conn = topo.connectors.len();
    topo.connectors
        .retain(|c| parse_handle(&c.handle).is_some_and(|h| PidAuthor::load(doc, h).is_some()));
    pruned_records += before_conn.saturating_sub(topo.connectors.len());

    let mut cleared_dangling_peers = 0usize;
    let mut updated_moved_endpoints = 0usize;

    for seg in &mut topo.pipe_segments {
        for end_slot in [&mut seg.start, &mut seg.end] {
            let Some(peer_str) = end_slot.peer.clone() else {
                continue;
            };
            let Some((h_hex, _port_name)) = peer_str.split_once(':') else {
                end_slot.peer = None;
                cleared_dangling_peers += 1;
                continue;
            };
            let Some(peer_h) = parse_handle(h_hex) else {
                end_slot.peer = None;
                cleared_dangling_peers += 1;
                continue;
            };
            if PidAuthor::load(doc, peer_h).is_none() {
                end_slot.peer = None;
                cleared_dangling_peers += 1;
                continue;
            }
            let Some(peer_ent) = doc.get_entity(peer_h) else {
                end_slot.peer = None;
                cleared_dangling_peers += 1;
                continue;
            };
            if let Some(anchor) = entity_world_anchor(peer_ent) {
                let d = (end_slot.point[0] - anchor.0).hypot(end_slot.point[1] - anchor.1);
                if d > 25.0 {
                    end_slot.point = [anchor.0, anchor.1];
                    updated_moved_endpoints += 1;
                }
            }
        }
    }

    topo.store(doc);
    let duplicated_tags = refresh_duplicate_tag_statuses(doc);

    AuthorSyncReport {
        pruned_records,
        cleared_dangling_peers,
        updated_moved_endpoints,
        duplicated_tags,
    }
}

/// Extract unified `ExportDrawing` + `DualConvertBundle` directly from `PID_AUTHOR` + `TopologyTable`
/// and write synchronized `_Data.xml` + `_Meta.xml` + `.pid` (`M5` / `L2-7` / `L6-2`).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_author_document_to_dual(
    doc: &Document,
    doc_name: &str,
    opts: &ConvertOptions,
) -> Result<DualConvertBundle, String> {
    use crate::io::pid_write::{
        apply_increments_to_seed, AddedAttributeSpec, AddedLineSpec, AddedSymbolSpec,
        AddedTextSpec, PidWriteIncrements, SeedOidPool,
    };
    use pid_parse::{build_normalized_geometry, PidWriter, WritePlan};

    let upm = guess_units_per_mm(doc).max(1e-6);
    let mut seed_opt = if opts.wants_dual_pid() {
        Some(load_configured_seed(doc, opts)?)
    } else {
        None
    };

    let mut alloc = UidAllocator::new(&opts.plant, doc_name);
    if let Some((ref seed_pkg, _)) = seed_opt {
        let pool = SeedOidPool::from_seed(seed_pkg);
        let max_oid = pool.used_oids().iter().copied().max().unwrap_or(99);
        alloc.reserve_graphic_oids(max_oid.saturating_add(1).max(100), pool.used_oids());
    }

    let doc_uid = alloc.alloc_primary_uid(None, &format!("author_doc:{doc_name}"));
    let mut dwg = ExportDrawing::new(&opts.plant, &doc_uid, doc_name);
    let mut exported_by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut endpoints: Vec<SymbolEndpointRef> = Vec::new();

    struct AuthorSymDraw {
        graphic_oid: u32,
        at_mm: (f64, f64),
        sym_path: String,
        label: Option<String>,
    }
    let mut sym_draws: Vec<AuthorSymDraw> = Vec::new();
    let mut run_inputs: Vec<PipeRunInput> = Vec::new();
    let mut seg_draws = Vec::new();

    let handles_and_geom: Vec<_> = doc
        .model_space_entities()
        .map(|ent| {
            let h = ent.common().handle;
            let anchor = entity_world_anchor(ent);
            let segs = polyline_or_line_segments(ent);
            let blk = match ent {
                EntityType::Insert(ins) => ins.block_name.clone(),
                EntityType::Line(_) => "LINE".to_string(),
                EntityType::Circle(_) => "CIRCLE".to_string(),
                EntityType::LwPolyline(_) => "LWPOLYLINE".to_string(),
                EntityType::Text(_) => "TEXT".to_string(),
                _ => String::new(),
            };
            (h, blk, anchor, segs)
        })
        .collect();

    for (h, blk_name, pt_opt, poly_segs) in handles_and_geom {
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap = parse_props_map(&auth.props);

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            if let Some(raw_segs) = pmap.get("_segments").and_then(|v| v.as_str()) {
                for entry in raw_segs.split(';').filter(|s| !s.trim().is_empty()) {
                    let (coords_str, lbl_opt) = match entry.split_once('|') {
                        Some((c, l)) => (c, (!l.trim().is_empty()).then(|| l.trim().to_string())),
                        None => (entry, None),
                    };
                    if let Some((ax, ay, bx, by)) = parse_quad_f64(coords_str) {
                        let goid = alloc.alloc_graphic_oid(None);
                        run_inputs.push(PipeRunInput {
                            start: (ax, ay),
                            end: (bx, by),
                            line_label: lbl_opt,
                            graphic_oid: Some(goid),
                        });
                        seg_draws.push((goid, (ax / upm, ay / upm), (bx / upm, by / upm)));
                    }
                }
            } else if !poly_segs.is_empty() {
                let lbl = pmap
                    .get("_line_label")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                for (a, b) in poly_segs {
                    let goid = alloc.alloc_graphic_oid(None);
                    run_inputs.push(PipeRunInput {
                        start: a,
                        end: b,
                        line_label: lbl.clone(),
                        graphic_oid: Some(goid),
                    });
                    seg_draws.push((goid, (a.0 / upm, a.1 / upm), (b.0 / upm, b.1 / upm)));
                }
            }
            continue;
        }

        let stored_at = pmap
            .get("_at")
            .and_then(|v| v.as_str())
            .and_then(parse_pair_f64);
        let Some(at) = stored_at.or(pt_opt) else {
            continue;
        };
        let explicit_cls = pmap
            .get("_sppid_class")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let (sppid_cls, default_vid) = match explicit_cls {
            Some(c) => {
                let vid = format!("{c}#01");
                (c, vid)
            }
            None => match author_type_to_sppid_fallback(&auth.type_code) {
                Some((c, v)) => (c.to_string(), v.to_string()),
                None => continue,
            },
        };
        let variant_id = pmap
            .get("_variant_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or(default_vid);
        let sym_path = pmap
            .get("_symbol_path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                default_class_map()
                    .variants
                    .iter()
                    .find(|v| v.class == sppid_cls && v.variant_id == variant_id)
                    .and_then(|v| v.symbol_paths.first().cloned())
            })
            .unwrap_or_else(|| format!("Symbols\\{blk_name}.sym"));

        let uid = alloc.alloc_primary_uid(None, &format!("author_sym:{:X}", h.value()));
        let goid = alloc.alloc_graphic_oid(None);

        let mut attrs: BTreeMap<(String, String), String> = BTreeMap::new();
        let tf_txt = pmap
            .get("对端图号")
            .or_else(|| pmap.get("_to_from_text"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if sppid_cls == "PIDBranchPoint" {
            if let Some(ref tf) = tf_txt {
                attrs.insert(
                    ("IOffpageConnector".to_string(), "ToFromText".to_string()),
                    tf.clone(),
                );
            }
        }
        if let Some(nd) = pmap
            .get("公称直径")
            .or_else(|| pmap.get("_nominal_diameter"))
            .and_then(|v| v.as_str())
        {
            attrs.insert(
                (
                    "IPipeCrossSectionItem".to_string(),
                    "NominalDiameter".to_string(),
                ),
                nd.to_string(),
            );
        }

        let name_opt = if auth.tag.trim().is_empty() {
            None
        } else {
            Some(auth.tag.clone())
        };
        let mut item = ExportItem {
            uid: uid.clone(),
            class: sppid_cls.clone(),
            name: name_opt.clone(),
            variant_id: Some(variant_id),
            symbol_path: Some(sym_path.clone()),
            graphic_oids: vec![goid],
            attributes: attrs,
            ..Default::default()
        };
        populate_item_tag_attributes(&mut item);

        *exported_by_class.entry(sppid_cls.clone()).or_default() += 1;
        let bbox = pmap
            .get("_bbox")
            .and_then(|v| v.as_str())
            .and_then(parse_quad_f64)
            .unwrap_or_else(|| {
                let r = 4.0 * upm;
                (at.0 - r, at.1 - r, at.0 + r, at.1 + r)
            });
        let ports = vec![
            (bbox.0, at.1),
            (bbox.2, at.1),
            (at.0, bbox.1),
            (at.0, bbox.3),
            at,
        ];
        endpoints.push(SymbolEndpointRef {
            symbol_uid: uid,
            sppid_class: sppid_cls,
            bbox,
            ports,
        });
        sym_draws.push(AuthorSymDraw {
            graphic_oid: goid,
            at_mm: (at.0 / upm, at.1 / upm),
            sym_path,
            label: name_opt.or(tf_txt),
        });
        dwg.items.push(item);
    }

    let mut piping_stats = synthesize_piping_network(
        &mut dwg,
        &mut alloc,
        &endpoints,
        &run_inputs,
        opts.snap_tol_mm * upm * 2.5,
    )?;

    // Ensure lines in TopologyTable are also represented as PIDPipeline items
    let topo = TopologyTable::load(doc);
    let mut existing_pipes: BTreeSet<String> = dwg
        .items
        .iter()
        .filter(|i| i.class == "PIDPipeline")
        .filter_map(|i| i.name.clone())
        .collect();
    for pl in &topo.pipe_lines {
        if let Some(parsed) = parse_sppid_line_number(&pl.tag) {
            let pname = parsed.pipeline_name();
            if existing_pipes.insert(pname.clone()) {
                let puid = pipeline_uid(&dwg.plant, &pname);
                dwg.items.push(ExportItem {
                    uid: puid,
                    class: "PIDPipeline".to_string(),
                    name: Some(pname),
                    ..Default::default()
                });
                piping_stats.pipelines += 1;
            }
        }
    }

    let sym_total: usize = exported_by_class.values().sum();
    let mut report = ConvertSheetReport {
        drawing_name: doc_name.to_string(),
        doc_uid: doc_uid.clone(),
        plant: opts.plant.clone(),
        title_block: TitleBlockInfo::default(),
        confidence_summary: ConfidenceSummary {
            high: sym_total,
            medium: 0,
            low: 0,
            ignored_blocks: 0,
            unmapped_blocks: 0,
        },
        exported_by_class,
        variant_resolution: VariantResolutionSummary::default(),
        piping_synthesis: piping_stats,
        pipe_network_census: PipeNetworkCensus {
            dead_end_attribution: DeadEndAttribution {
                snapped_to_port: run_inputs.len() * 2,
                ..Default::default()
            },
            ..Default::default()
        },
        offpage_connectors: Vec::new(),
        total_items_written: dwg.items.len(),
        total_relations_written: dwg.relations.len(),
        skipped_items: Vec::new(),
        pid_verification: None,
    };

    let eff_map = resolve_effective_class_map(opts)?;
    let (mut publish, var_res) = export_drawing_bundle_with_map(&dwg, &eff_map)?;
    report.variant_resolution = var_res;

    let pid_bytes = if let Some((ref mut seed_pkg, ref inferred_tpl)) = seed_opt {
        let seed_geom = build_normalized_geometry(&seed_pkg.parsed);
        let page_mm = seed_geom.page_dimensions_mm.unwrap_or((841.0, 594.0));
        let (line_style_id, text_style_id) = pick_seed_style_ids(seed_pkg);
        let rep_uid_map = compute_representation_uid_map(&dwg);
        let item_by_goid: BTreeMap<u32, &ExportItem> = dwg
            .items
            .iter()
            .flat_map(|it| it.graphic_oids.iter().copied().map(move |oid| (oid, it)))
            .collect();

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for sd in &sym_draws {
            min_x = min_x.min(sd.at_mm.0);
            min_y = min_y.min(sd.at_mm.1);
            max_x = max_x.max(sd.at_mm.0);
            max_y = max_y.max(sd.at_mm.1);
        }
        for &(_, a, b) in &seg_draws {
            for p in [a, b] {
                min_x = min_x.min(p.0);
                min_y = min_y.min(p.1);
                max_x = max_x.max(p.0);
                max_y = max_y.max(p.1);
            }
        }
        let margin = 20.0;
        let avail_w = (page_mm.0 - 2.0 * margin).max(40.0);
        let avail_h = (page_mm.1 - 2.0 * margin).max(40.0);
        let fits_directly = min_x.is_finite()
            && min_x >= 10.0
            && min_y >= 10.0
            && max_x <= page_mm.0 - 10.0
            && max_y <= page_mm.1 - 10.0;
        let span_x = (max_x - min_x).max(1.0);
        let span_y = (max_y - min_y).max(1.0);
        let fit_scale = (avail_w / span_x).min(avail_h / span_y).clamp(0.05, 1.0);
        let off_x = margin + (avail_w - span_x * fit_scale) * 0.5;
        let off_y = margin + (avail_h - span_y * fit_scale) * 0.5;

        let map_pt = |pt: (f64, f64)| -> (f64, f64) {
            if fits_directly || !min_x.is_finite() {
                (
                    pt.0.clamp(10.0, (page_mm.0 - 10.0).max(20.0)),
                    pt.1.clamp(10.0, (page_mm.1 - 10.0).max(20.0)),
                )
            } else {
                (
                    ((pt.0 - min_x) * fit_scale + off_x).clamp(10.0, (page_mm.0 - 10.0).max(20.0)),
                    ((pt.1 - min_y) * fit_scale + off_y).clamp(10.0, (page_mm.1 - 10.0).max(20.0)),
                )
            }
        };

        let mut inc = PidWriteIncrements {
            doc_uid: Some(doc_uid.clone()),
            plant_name: Some(opts.plant.clone()),
            template_name: inferred_tpl.clone(),
            ..Default::default()
        };

        for sd in &sym_draws {
            let p = map_pt(sd.at_mm);
            inc.symbols.push(AddedSymbolSpec {
                preferred_oid: Some(sd.graphic_oid),
                symbol_rel_path: sd.sym_path.clone(),
                insertion_mm: p,
                rotation_rad: 0.0,
                scale: if fits_directly { 1.0 } else { fit_scale },
                style_ref: line_style_id,
            });
            if let Some(ref lbl) = sd.label {
                inc.texts.push(AddedTextSpec {
                    preferred_oid: None,
                    text: lbl.clone(),
                    insertion_mm: (p.0, (p.1 + 3.0).min(page_mm.1 - 4.0)),
                    rotation_rad: 0.0,
                    style_index: text_style_id,
                });
            }
            if let Some(item) = item_by_goid.get(&sd.graphic_oid) {
                let rep_uid = rep_uid_map
                    .get(&(item.uid.clone(), sd.graphic_oid))
                    .cloned()
                    .ok_or_else(|| format!("missing rep UID for {}", item.uid))?;
                inc.attributes.push(AddedAttributeSpec {
                    described_graphic_oid: sd.graphic_oid,
                    model_uid: item.uid.clone(),
                    representation_uid: rep_uid,
                    sppid_class: item.class.clone(),
                    drawing_no: doc_name.to_string(),
                    item_tag: item.name.clone(),
                });
            }
        }

        for &(goid, a_mm, b_mm) in &seg_draws {
            inc.lines.push(AddedLineSpec {
                preferred_oid: Some(goid),
                start_mm: map_pt(a_mm),
                end_mm: map_pt(b_mm),
                style_index: line_style_id,
            });
            if let Some(item) = item_by_goid.get(&goid) {
                let rep_uid = rep_uid_map
                    .get(&(item.uid.clone(), goid))
                    .cloned()
                    .ok_or_else(|| format!("missing rep UID for {}", item.uid))?;
                inc.attributes.push(AddedAttributeSpec {
                    described_graphic_oid: goid,
                    model_uid: item.uid.clone(),
                    representation_uid: rep_uid,
                    sppid_class: item.class.clone(),
                    drawing_no: doc_name.to_string(),
                    item_tag: item.name.clone(),
                });
            }
        }

        let sym_dir = opts
            .sym_library_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        apply_increments_to_seed(seed_pkg, &inc, sym_dir.as_deref())?;
        let raw_pid = PidWriter::write_to_bytes(seed_pkg, &WritePlan::default())
            .map_err(|e| format!("write .pid bytes for {doc_name}: {e}"))?;

        let ver = verify_dual_bundle(&doc_uid, &publish.data_xml, &raw_pid, None, doc_name)?;
        report.pid_verification = Some(ver);
        Some(raw_pid)
    } else {
        None
    };

    publish.coverage_json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("serialize coverage report: {e}"))?;

    Ok(DualConvertBundle {
        publish,
        pid_bytes,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_publish_check::{
        check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
    };
    use crate::pid::persist::{PID_AUTHOR_APP, PROPS_REF_KEY, XDATA_SOFT_LIMIT};
    use crate::scene::view::dispatch::set_entity_xdata;
    use document::entities::{Block, BlockEnd, Insert, Line, Point, Text};
    use document::tables::BlockRecord;
    use document::types::Vector3;

    fn define_block(doc: &mut Document, name: &str, w: f64, h: f64) {
        if doc.block_records.get(name).is_some() {
            return;
        }
        let next = doc.next_handle();
        let br_h = Handle::new(next);
        let b_h = Handle::new(next + 1);
        let e_h = Handle::new(next + 2);
        let mut rec = BlockRecord::new(name);
        rec.handle = br_h;
        rec.block_entity_handle = b_h;
        rec.block_end_handle = e_h;
        doc.block_records.add(rec).unwrap();
        let mut blk = Block::new(name, Vector3::ZERO);
        blk.common.handle = b_h;
        blk.common.owner_handle = br_h;
        doc.add_entity(EntityType::Block(blk)).unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = e_h;
        end.common.owner_handle = br_h;
        doc.add_entity(EntityType::BlockEnd(end)).unwrap();

        let mut ln = Line::from_points(
            Vector3::new(-w * 0.5, -h * 0.5, 0.0),
            Vector3::new(w * 0.5, h * 0.5, 0.0),
        );
        ln.common.owner_handle = br_h;
        doc.add_entity(EntityType::Line(ln)).unwrap();
        for px in [-w * 0.5, w * 0.5] {
            let mut pt = Point::new();
            pt.location = Vector3::new(px, 0.0, 0.0);
            pt.common.owner_handle = br_h;
            doc.add_entity(EntityType::Point(pt)).unwrap();
        }
    }

    fn build_author_test_sheet() -> (Document, Handle, Handle, Handle) {
        let mut doc = Document::new();
        define_block(&mut doc, "METERING PUMP", 12.0, 10.0);
        define_block(&mut doc, "BALL VALVE TYPE 2", 6.0, 4.0);
        define_block(&mut doc, "_OFF-UNIT SQ", 10.0, 6.0);

        let mut pump = Insert::new("METERING PUMP", Vector3::new(40.0, 100.0, 0.0));
        pump.common.layer = "10".to_string();
        let pump_h = doc.add_entity(EntityType::Insert(pump)).unwrap();
        let mut pt = Text::with_value("P-0201A", Vector3::new(38.0, 110.0, 0.0)).with_height(2.5);
        pt.common.layer = "10".to_string();
        doc.add_entity(EntityType::Text(pt)).unwrap();

        let mut valve = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 100.0, 0.0));
        valve.common.layer = "11".to_string();
        let valve_h = doc.add_entity(EntityType::Insert(valve)).unwrap();
        let mut vt =
            Text::with_value("BV0201501A", Vector3::new(96.0, 106.0, 0.0)).with_height(2.5);
        vt.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(vt)).unwrap();

        let mut opc = Insert::new("_OFF-UNIT SQ", Vector3::new(170.0, 100.0, 0.0));
        opc.common.layer = "11".to_string();
        let opc_h = doc.add_entity(EntityType::Insert(opc)).unwrap();
        let mut ot =
            Text::with_value("DWG-0201-09", Vector3::new(165.0, 106.0, 0.0)).with_height(2.5);
        ot.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(ot)).unwrap();

        for (a, b) in [
            ((46.0, 100.0), (97.0, 100.0)),
            ((103.0, 100.0), (165.0, 100.0)),
        ] {
            let mut ln =
                Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
            ln.common.layer = "11".to_string();
            doc.add_entity(EntityType::Line(ln)).unwrap();
        }
        let mut lt =
            Text::with_value("50-PG-02011-B9-", Vector3::new(60.0, 103.0, 0.0)).with_height(2.5);
        lt.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(lt)).unwrap();

        (doc, pump_h, valve_h, opc_h)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn synthetic_blank_seed_pid() -> Vec<u8> {
        use pid_parse::{PidDocument, PidPackage, PidWriter, RawStream, WritePlan};

        fn wrap_psm(type_code: u16, payload: &[u8]) -> Vec<u8> {
            let mut v = Vec::with_capacity(6 + payload.len());
            v.extend_from_slice(&0x0001u16.to_le_bytes());
            v.extend_from_slice(&type_code.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
            v
        }
        fn make_layer(oid: u32, name: &str) -> Vec<u8> {
            let u16s: Vec<u16> = name.encode_utf16().collect();
            let mut p = Vec::new();
            p.extend_from_slice(&oid.to_le_bytes());
            p.extend_from_slice(&[0u8; 8]);
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&6u32.to_le_bytes());
            p.extend_from_slice(&(u16s.len() as u16).to_le_bytes());
            for u in u16s {
                p.extend_from_slice(&u.to_le_bytes());
            }
            p.extend_from_slice(&0u16.to_le_bytes());
            wrap_psm(0x0081, &p)
        }
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());

        let mut cluster0 = 0x6C90_F544u32.to_le_bytes().to_vec();
        cluster0.extend_from_slice(&2u32.to_le_bytes());
        cluster0.extend_from_slice(&make_layer(8, "Default"));
        cluster0.extend_from_slice(&make_layer(12, "Labels"));

        let mut streams = BTreeMap::new();
        for (p, data) in [
            ("/Sheet6", empty_chain.clone()),
            ("/PSMcluster0", cluster0),
            ("/Unclustered Dynamic Attributes", empty_chain.clone()),
            ("/StyleCluster", empty_chain),
        ] {
            streams.insert(
                p.to_string(),
                RawStream {
                    path: p.to_string(),
                    data,
                    modified: false,
                },
            );
        }
        streams.insert(
            "/TaggedTxtData/Revision".to_string(),
            RawStream {
                path: "/TaggedTxtData/Revision".to_string(),
                data: Vec::new(),
                modified: false,
            },
        );
        let mut roots = 0x746F_6F72u32.to_le_bytes().to_vec();
        roots.extend_from_slice(&[0u8; 8]);
        streams.insert(
            "/PSMroots".to_string(),
            RawStream {
                path: "/PSMroots".to_string(),
                data: roots,
                modified: false,
            },
        );
        let mut tseg = 0x6765_7374u32.to_le_bytes().to_vec();
        tseg.extend_from_slice(&0u16.to_le_bytes());
        tseg.extend_from_slice(&4u16.to_le_bytes());
        tseg.extend_from_slice(&32u16.to_le_bytes());
        tseg.extend_from_slice(&0u16.to_le_bytes());
        streams.insert(
            "/PSMspacemap/0x00000000".to_string(),
            RawStream {
                path: "/PSMspacemap/0x00000000".to_string(),
                data: tseg,
                modified: false,
            },
        );
        let pkg = PidPackage::new(None, streams, PidDocument::default());
        PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap()
    }

    #[test]
    fn m1_m2_promotion_is_idempotent_protects_manual_tags_spills_large_props_and_isolates_purge() {
        let (mut doc, pump_h, valve_h, _opc_h) = build_author_test_sheet();
        set_entity_xdata(
            &mut doc,
            pump_h,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("class=计量泵".to_string()),
                XDataValue::String("resolved=legend:block".to_string()),
            ]),
        );

        let r1 = promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote 1");
        assert!(r1.newly_promoted >= 3);
        assert_eq!(r1.updated_in_place, 0);

        let mut v_auth = PidAuthor::load(&doc, valve_h).expect("valve author");
        assert_eq!(v_auth.tag, "BV0201501A");
        v_auth.tag = "BV-MANUAL-999".to_string();
        v_auth.tag_source = "manual".to_string();
        let mut p_json: serde_json::Value = serde_json::from_str(&v_auth.props).unwrap();
        p_json["custom_note"] = serde_json::Value::String("X".repeat(XDATA_SOFT_LIMIT + 300));
        v_auth.props = serde_json::to_string(&p_json).unwrap();
        v_auth.store(&mut doc, valve_h);

        let raw_rec = doc
            .get_entity(valve_h)
            .unwrap()
            .common()
            .extended_data
            .get_record(PID_AUTHOR_APP)
            .unwrap();
        assert!(raw_rec.values.iter().any(|v| matches!(
            v,
            XDataValue::String(s) if s.starts_with(&format!("{PROPS_REF_KEY}="))
        )));

        let r2 = promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote 2");
        assert_eq!(r2.newly_promoted, 0);
        assert!(r2.updated_in_place >= 3);
        assert_eq!(r2.manual_tags_preserved, 1);

        let v_after = PidAuthor::load(&doc, valve_h).expect("valve after re-promote");
        assert_eq!(v_after.tag, "BV-MANUAL-999");
        assert_eq!(v_after.tag_source, "manual");
        let after_json: serde_json::Value = serde_json::from_str(&v_after.props).unwrap();
        assert_eq!(
            after_json["custom_note"].as_str().map(str::len),
            Some(XDATA_SOFT_LIMIT + 300)
        );

        crate::io::pid_legend::purge(&mut doc);
        assert!(doc
            .get_entity(pump_h)
            .unwrap()
            .common()
            .extended_data
            .get_record(PID_SEMANTICS_XDATA_APP)
            .is_none());
        assert_eq!(PidAuthor::load(&doc, pump_h).unwrap().tag, "P-0201A");
    }

    #[test]
    fn m3_l2_5_topology_sync_clears_dangling_peers_on_delete_updates_move_and_flags_duplicate_tags()
    {
        let (mut doc, pump_h, valve_h, _opc_h) = build_author_test_sheet();
        let rep = promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote");
        assert!(rep.pipe_lines_promoted >= 1);
        assert!(rep.pipe_segments_promoted >= 2);

        let topo_before = TopologyTable::load(&doc);
        assert!(!topo_before.pipe_segments.is_empty());
        let valve_hex = format!("{:X}", valve_h.value());
        assert!(topo_before.pipe_segments.iter().any(|s| {
            s.start
                .peer
                .as_deref()
                .is_some_and(|p| p.starts_with(&valve_hex))
                || s.end
                    .peer
                    .as_deref()
                    .is_some_and(|p| p.starts_with(&valve_hex))
        }));

        let mut dup_pump = Insert::new("METERING PUMP", Vector3::new(40.0, 160.0, 0.0));
        dup_pump.common.layer = "10".to_string();
        let dup_h = doc.add_entity(EntityType::Insert(dup_pump)).unwrap();
        let mut dup_auth = PidAuthor::new("PUMP", "P-0201A");
        dup_auth.tag_source = "manual".to_string();
        dup_auth.store(&mut doc, dup_h);

        if let Some(EntityType::Insert(ins)) = doc.get_entity_mut(pump_h) {
            ins.insert_point = Vector3::new(85.0, 100.0, 0.0);
        }
        let _ = doc.remove_entity(valve_h);

        let sync = sync_author_after_edit(&mut doc);
        assert!(sync.pruned_records >= 1);
        assert!(sync.cleared_dangling_peers >= 1);
        assert!(sync.updated_moved_endpoints >= 1);
        assert!(sync.duplicated_tags >= 2);
        assert_eq!(
            PidAuthor::load(&doc, pump_h).unwrap().tag_status,
            "duplicated"
        );

        let topo_after = TopologyTable::load(&doc);
        assert!(topo_after.fittings.iter().all(|f| f.handle != valve_hex));
        assert!(topo_after.pipe_segments.iter().all(|s| {
            !s.start
                .peer
                .as_deref()
                .is_some_and(|p| p.starts_with(&valve_hex))
                && !s
                    .end
                    .peer
                    .as_deref()
                    .is_some_and(|p| p.starts_with(&valve_hex))
        }));
    }

    #[test]
    fn l2_6_promotes_imported_pid_semantics_without_xml_selectively_and_in_batch() {
        let mut doc = Document::new();
        define_block(&mut doc, "SYM_CACHE_1", 6.0, 4.0);
        let mut i1 = Insert::new("SYM_CACHE_1", Vector3::new(20.0, 20.0, 0.0));
        i1.common.layer = "Equipment".to_string();
        let h1 = doc.add_entity(EntityType::Insert(i1)).unwrap();

        let mut i2 = Insert::new("SYM_CACHE_1", Vector3::new(60.0, 20.0, 0.0));
        i2.common.layer = "Default".to_string();
        let h2 = doc.add_entity(EntityType::Insert(i2)).unwrap();

        set_entity_xdata(
            &mut doc,
            h1,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=symbol".to_string()),
                XDataValue::String("instance=Equipment\\Vessels\\Vertical Drum.sym".to_string()),
            ]),
        );
        set_entity_xdata(
            &mut doc,
            h2,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=symbol".to_string()),
                XDataValue::String("class=PIDPipingComponent".to_string()),
                XDataValue::String("label=BV-1001".to_string()),
                XDataValue::String("oid=12345".to_string()),
            ]),
        );

        let mut only_h1 = BTreeSet::new();
        only_h1.insert(h1);
        let sel = promote_semantics_to_author(&mut doc, Some(&only_h1));
        assert_eq!(sel.newly_promoted, 1);
        assert!(PidAuthor::load(&doc, h1).is_some());
        assert!(PidAuthor::load(&doc, h2).is_none());

        let all = promote_semantics_to_author(&mut doc, None);
        assert_eq!(all.newly_promoted, 1);
        assert_eq!(all.updated_in_place, 1);
        assert_eq!(PidAuthor::load(&doc, h2).unwrap().tag, "BV-1001");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn m5_l2_7_l6_2_edits_tag_adds_and_deletes_symbol_then_saves_back_to_pid_and_passes_ab_roundtrip(
    ) {
        let (mut doc, pump_h, valve_h, opc_h) = build_author_test_sheet();
        promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote");

        let mut p_auth = PidAuthor::load(&doc, pump_h).unwrap();
        p_auth.tag = "P-0201Z-EDITED".to_string();
        p_auth.tag_source = "manual".to_string();
        p_auth.store(&mut doc, pump_h);

        let mut new_v = Insert::new("BALL VALVE TYPE 2", Vector3::new(130.0, 100.0, 0.0));
        new_v.common.layer = "11".to_string();
        let new_vh = doc.add_entity(EntityType::Insert(new_v)).unwrap();
        let mut nv_auth = PidAuthor::new("VALVE", "BV-NEW-777");
        nv_auth.tag_source = "manual".to_string();
        nv_auth.props = serde_json::json!({
            "_sppid_class": "PIDPipingComponent",
            "_variant_id": "PIDPipingComponent#01",
            "_symbol_path": "Piping\\Valves\\2 Way Common\\Ball Valve Type 2.sym"
        })
        .to_string();
        nv_auth.store(&mut doc, new_vh);

        let _ = doc.remove_entity(opc_h);
        let _ = valve_h;
        sync_author_after_edit(&mut doc);

        let opts = ConvertOptions {
            seed_pid_bytes: Some(synthetic_blank_seed_pid()),
            verify_plot_pdf: true,
            ..Default::default()
        };
        let dual = save_author_document_to_dual(&doc, "AUTHOR-SAVE-01", &opts)
            .expect("save author doc to dual");

        let data_str = String::from_utf8_lossy(&dual.publish.data_xml);
        assert!(data_str.contains("P-0201Z-EDITED"));
        assert!(data_str.contains("BV-NEW-777"));
        assert!(!data_str.contains("DWG-0201-09"));

        let spec = default_publish_spec();
        let chk_opts = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..Default::default()
        };
        assert!(check_publish_xml_bytes(
            "AUTHOR-SAVE-01_Data.xml",
            PublishXmlKind::Data,
            &dual.publish.data_xml,
            spec,
            &chk_opts,
        )
        .is_empty());

        let pid_bytes = dual.pid_bytes.as_ref().expect(".pid bytes");
        let tmp = std::env::temp_dir().join(format!("ocs_author_save_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let ver = verify_dual_bundle(
            &dual.report.doc_uid,
            &dual.publish.data_xml,
            pid_bytes,
            Some(&tmp),
            "AUTHOR-SAVE-01",
        )
        .expect("verify saved .pid");
        assert!(ver.is_cross_file_and_a_valid(), "{ver:?}");
        assert!(ver.is_b_plot_valid(), "{ver:?}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real DXF + real blank .pid seed promote -> edit -> save-back -> a/b roundtrip"]
    fn real_sheet_promote_edit_and_save_back_to_pid_roundtrip() {
        let Ok(dxf_path) = std::env::var("OCS_PID_REAL_DXF") else {
            println!("OCS_PID_REAL_DXF unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").ok();
        let out_dir = std::env::var("OCS_PID_REAL134_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real134"));
        let _ = std::fs::create_dir_all(&out_dir);

        let mut doc = crate::io::load_file(std::path::Path::new(&dxf_path)).expect("load real DXF");
        let prom1 = promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote");
        let prom2 =
            promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("re-promote");
        assert_eq!(prom2.newly_promoted, 0);
        assert_eq!(prom2.updated_in_place, prom1.newly_promoted);

        // Edit first promoted valve/instrument tag and add one new valve symbol
        let first_sym_h = doc
            .model_space_entities()
            .find_map(|e| {
                let h = e.common().handle;
                let a = PidAuthor::load(&doc, h)?;
                (a.type_code == "VALVE" && !a.tag.is_empty()).then_some(h)
            })
            .expect("find promoted valve");
        let mut edited = PidAuthor::load(&doc, first_sym_h).unwrap();
        edited.tag = "BV-EDITED-134".to_string();
        edited.tag_source = "manual".to_string();
        edited.store(&mut doc, first_sym_h);

        let mut added = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 150.0, 0.0));
        added.common.layer = "11".to_string();
        let added_h = doc.add_entity(EntityType::Insert(added)).unwrap();
        let mut added_auth = PidAuthor::new("VALVE", "BV-ADDED-134");
        added_auth.tag_source = "manual".to_string();
        added_auth.props = serde_json::json!({
            "_sppid_class": "PIDPipingComponent",
            "_variant_id": "PIDPipingComponent#01",
            "_symbol_path": "Piping\\Valves\\2 Way Common\\Ball Valve Type 2.sym"
        })
        .to_string();
        added_auth.store(&mut doc, added_h);

        let sync = sync_author_after_edit(&mut doc);
        let opts = ConvertOptions {
            seed_pid_path: Some(seed_zip),
            sym_library_dir: sym_dir,
            verify_plot_pdf: true,
            ..Default::default()
        };
        let dual = save_author_document_to_dual(&doc, "REAL-AUTHOR-134", &opts)
            .expect("save real author sheet");
        let pid_bytes = dual.pid_bytes.as_ref().expect("real .pid bytes");
        let ver = verify_dual_bundle(
            &dual.report.doc_uid,
            &dual.publish.data_xml,
            pid_bytes,
            Some(&out_dir),
            "REAL-AUTHOR-134",
        )
        .expect("verify real .pid");
        assert!(ver.is_cross_file_and_a_valid(), "{ver:?}");
        assert!(ver.is_b_plot_valid(), "{ver:?}");

        let summary = serde_json::json!({
            "promote_pass1": prom1,
            "promote_pass2_idempotent": prom2,
            "sync_report": sync,
            "items_written": dual.report.total_items_written,
            "relations_written": dual.report.total_relations_written,
            "dual_verify": ver,
        });
        std::fs::write(
            out_dir.join("real134_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&summary).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn promotes_and_saves_native_autocad_circles_strokes_and_polylines_to_valid_dual_bundle() {
        use document::entities::{Circle, LwPolyline, LwVertex};

        let mut doc = Document::new();
        // Sheet frame so `guess_units_per_mm` detects 1.0 unit/mm (420 x 297 A3)
        let mut frame = LwPolyline::new();
        frame.is_closed = true;
        frame.common.layer = "PUB_TITLE".to_string();
        for (x, y) in [(0.0, 0.0), (420.0, 0.0), (420.0, 297.0), (0.0, 297.0)] {
            frame.vertices.push(LwVertex::from_coords(x, y));
        }
        doc.add_entity(EntityType::LwPolyline(frame)).unwrap();

        // 1. Loose Circle storage tank `TK-1001` (`r = 12.0`)
        let mut tank_c = Circle::from_coords(40.0, 100.0, 0.0, 12.0);
        tank_c.common.layer = "0".to_string();
        let tank_h = doc.add_entity(EntityType::Circle(tank_c)).unwrap();
        let mut tank_t =
            Text::with_value("TK-1001", Vector3::new(34.0, 99.0, 0.0)).with_height(2.5);
        tank_t.common.layer = "TEXT".to_string();
        doc.add_entity(EntityType::Text(tank_t)).unwrap();

        // 2. Exploded 4-stroke bowtie ball valve on `DEVICE` (two identical placements so shape dictionary / tag_classes triggers)
        for &(cx, tag_str) in &[(90.0, "BV3101"), (150.0, "BV3102")] {
            for (a, b) in [
                ((cx - 3.0, 98.0), (cx - 3.0, 102.0)),
                ((cx - 3.0, 102.0), (cx + 3.0, 98.0)),
                ((cx + 3.0, 98.0), (cx + 3.0, 102.0)),
                ((cx + 3.0, 102.0), (cx - 3.0, 98.0)),
            ] {
                let mut st =
                    Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
                st.common.layer = "DEVICE".to_string();
                doc.add_entity(EntityType::Line(st)).unwrap();
            }
            let mut vt =
                Text::with_value(tag_str, Vector3::new(cx - 3.0, 104.0, 0.0)).with_height(2.2);
            vt.common.layer = "TEXT".to_string();
            doc.add_entity(EntityType::Text(vt)).unwrap();
        }

        // 3. Loose Circle instrument bubble `PI-3102` (`r = 3.0`) with two inner Text lines
        let mut bub_c = Circle::from_coords(120.0, 118.0, 0.0, 3.0);
        bub_c.common.layer = "YIBIAO".to_string();
        let bub_h = doc.add_entity(EntityType::Circle(bub_c)).unwrap();
        let mut bub_t1 = Text::with_value("PI", Vector3::new(119.0, 118.8, 0.0)).with_height(1.8);
        bub_t1.common.layer = "TEXT".to_string();
        let bub_t1_h = doc.add_entity(EntityType::Text(bub_t1)).unwrap();
        let mut bub_t2 = Text::with_value("3102", Vector3::new(118.4, 116.4, 0.0)).with_height(1.8);
        bub_t2.common.layer = "TEXT".to_string();
        let bub_t2_h = doc.add_entity(EntityType::Text(bub_t2)).unwrap();

        // 4. Multi-vertex `LwPolyline` process pipe on `"0"` (`shapes.pipe_layers`) with 2-part line label `80-FS`
        let mut pipe_pl = LwPolyline::new();
        pipe_pl.common.layer = "0".to_string();
        for (x, y) in [(52.0, 100.0), (70.0, 100.0), (87.0, 100.0)] {
            pipe_pl.vertices.push(LwVertex::from_coords(x, y));
        }
        doc.add_entity(EntityType::LwPolyline(pipe_pl)).unwrap();

        let mut pipe_pl2 = LwPolyline::new();
        pipe_pl2.common.layer = "0".to_string();
        for (x, y) in [(93.0, 100.0), (147.0, 100.0)] {
            pipe_pl2.vertices.push(LwVertex::from_coords(x, y));
        }
        doc.add_entity(EntityType::LwPolyline(pipe_pl2)).unwrap();

        let mut pl_lbl = Text::with_value("80-FS", Vector3::new(62.0, 102.5, 0.0)).with_height(2.2);
        pl_lbl.common.layer = "TEXT".to_string();
        doc.add_entity(EntityType::Text(pl_lbl)).unwrap();

        let prom =
            promote_sheet_to_author(&mut doc, &ConvertOptions::default()).expect("promote sheet");
        assert!(
            prom.newly_promoted >= 4,
            "expected tank + bubble + 2 exploded valves promoted, got {prom:?}"
        );
        assert!(
            prom.pipe_segments_promoted >= 2,
            "expected LwPolyline pipe segments promoted, got {prom:?}"
        );
        assert!(PidAuthor::load(&doc, tank_h).is_some());
        assert!(PidAuthor::load(&doc, bub_h).is_some());

        // Edit inner-text circle bubble tag on `PID_AUTHOR` and verify two-line inner `Text` sync
        let mut bub_auth = PidAuthor::load(&doc, bub_h).unwrap();
        bub_auth.tag = "PI-3999".to_string();
        bub_auth.tag_source = "manual".to_string();
        bub_auth.store(&mut doc, bub_h);
        let synced = sync_paired_canvas_tag_texts(&mut doc);
        assert!(
            synced.contains(&bub_t1_h) || synced.contains(&bub_t2_h),
            "expected inner circle texts synced, got {synced:?}"
        );
        if let Some(EntityType::Text(t2)) = doc.get_entity(bub_t2_h) {
            assert_eq!(t2.value, "3999");
        }

        // Save back via `save_author_document_to_dual` (`PIDSAVEPID` engine) into synthetic seed `.pid`
        let tmp = std::env::temp_dir().join(format!("ocs_prom139_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let seed_path = tmp.join("seed139.pid");
        std::fs::write(&seed_path, synthetic_blank_seed_pid()).unwrap();

        let opts = ConvertOptions {
            plant: "P01".to_string(),
            seed_pid_path: Some(seed_path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let dual =
            save_author_document_to_dual(&doc, "LEGACY-PROM-139", &opts).expect("save dual 139");
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(
            dual.report.exported_by_class.get("PIDProcessVessel"),
            Some(&1),
            "Circle tank must not be dropped on PIDSAVEPID: {:?}",
            dual.report.exported_by_class
        );
        assert_eq!(
            dual.report.exported_by_class.get("PIDInstrument"),
            Some(&1),
            "Circle bubble must not be dropped on PIDSAVEPID: {:?}",
            dual.report.exported_by_class
        );
        assert_eq!(
            dual.report.exported_by_class.get("PIDPipingComponent"),
            Some(&2),
            "Exploded stroke valves must not be dropped on PIDSAVEPID: {:?}",
            dual.report.exported_by_class
        );
        assert!(
            dual.report.piping_synthesis.pipelines >= 1,
            "2-part line 80-FS on LwPolyline must synthesize PIDPipeline"
        );
        assert_eq!(dual.report.variant_resolution.class_default_fallback, 0);

        let spec = default_publish_spec();
        let chk = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..CheckOptions::default()
        };
        assert!(check_publish_xml_bytes(
            "LEGACY-PROM-139_Data.xml",
            PublishXmlKind::Data,
            &dual.publish.data_xml,
            spec,
            &chk
        )
        .is_empty());
        let ver = dual
            .report
            .pid_verification
            .as_ref()
            .expect("pid_verification");
        assert!(ver.is_cross_file_and_a_valid(), "{ver:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l2_6_real_pid_stroke_group_merges_by_oid_resolves_exact_variant_and_preserves_polyline_pipes(
    ) {
        use document::entities::{Circle, LwPolyline, LwVertex};

        let mut doc = Document::new();

        // 1. Multi-stroke imported `.pid` ball valve (`oid=501`): 4 body lines + `PID-SYMBOL-LABEL` + visible tag text
        let valve_sem = vec![
            XDataValue::String("role=symbol".to_string()),
            XDataValue::String("class=PIDPipingComponent".to_string()),
            XDataValue::String("label=BV-5001".to_string()),
            XDataValue::String("oid=501".to_string()),
            XDataValue::String("instance=Width:6.00;Height:4.00".to_string()),
            XDataValue::String("resolved=direct".to_string()),
        ];
        for (a, b) in [
            ((77.0, 98.0), (77.0, 102.0)),
            ((77.0, 102.0), (83.0, 98.0)),
            ((83.0, 98.0), (83.0, 102.0)),
            ((83.0, 102.0), (77.0, 98.0)),
        ] {
            let mut ln =
                Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
            ln.common.layer = "Default".to_string();
            let h = doc.add_entity(EntityType::Line(ln)).unwrap();
            set_entity_xdata(
                &mut doc,
                h,
                PID_SEMANTICS_XDATA_APP,
                Some(valve_sem.clone()),
            );
        }
        let mut sym_lbl =
            Text::with_value("Ball Valve Type 2", Vector3::new(80.0, 96.0, 0.0)).with_height(1.8);
        sym_lbl.common.layer = "PID-SYMBOL-LABEL".to_string();
        let sym_lbl_h = doc.add_entity(EntityType::Text(sym_lbl)).unwrap();
        set_entity_xdata(
            &mut doc,
            sym_lbl_h,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=symbol-label".to_string()),
                XDataValue::String("class=PIDPipingComponent".to_string()),
                XDataValue::String("label=BV-5001".to_string()),
                XDataValue::String("oid=501".to_string()),
                XDataValue::String("resolved=direct".to_string()),
            ]),
        );

        let mut vis_tag =
            Text::with_value("BV-5001", Vector3::new(78.0, 104.0, 0.0)).with_height(2.5);
        vis_tag.common.layer = "Labels".to_string();
        let vis_tag_h = doc.add_entity(EntityType::Text(vis_tag)).unwrap();
        set_entity_xdata(
            &mut doc,
            vis_tag_h,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=text".to_string()),
                XDataValue::String("class=PIDPipingComponent".to_string()),
                XDataValue::String("label=BV-5001".to_string()),
                XDataValue::String("oid=501".to_string()),
                XDataValue::String("resolved=dependency:500".to_string()),
            ]),
        );

        // 2. Multi-entity imported `.pid` pressure gauge (`oid=502`): Circle + pointer Line + `PID-SYMBOL-LABEL`
        let mut pg_c = Circle::new();
        pg_c.center = Vector3::new(140.0, 100.0, 0.0);
        pg_c.radius = 4.0;
        pg_c.common.layer = "Default".to_string();
        let pg_c_h = doc.add_entity(EntityType::Circle(pg_c)).unwrap();
        let pg_sem = vec![
            XDataValue::String("role=symbol".to_string()),
            XDataValue::String("class=PIDInstrument".to_string()),
            XDataValue::String("label=PG-5002".to_string()),
            XDataValue::String("oid=502".to_string()),
            XDataValue::String("instance=Diameter:8.00".to_string()),
            XDataValue::String("resolved=direct".to_string()),
        ];
        set_entity_xdata(
            &mut doc,
            pg_c_h,
            PID_SEMANTICS_XDATA_APP,
            Some(pg_sem.clone()),
        );
        let mut pg_ptr = Line::from_points(
            Vector3::new(138.0, 98.0, 0.0),
            Vector3::new(142.0, 102.0, 0.0),
        );
        pg_ptr.common.layer = "Default".to_string();
        let pg_ptr_h = doc.add_entity(EntityType::Line(pg_ptr)).unwrap();
        set_entity_xdata(&mut doc, pg_ptr_h, PID_SEMANTICS_XDATA_APP, Some(pg_sem));

        let mut pg_lbl =
            Text::with_value("PG-Pressure Gauge", Vector3::new(140.0, 94.0, 0.0)).with_height(1.8);
        pg_lbl.common.layer = "PID-SYMBOL-LABEL".to_string();
        let pg_lbl_h = doc.add_entity(EntityType::Text(pg_lbl)).unwrap();
        set_entity_xdata(
            &mut doc,
            pg_lbl_h,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=symbol-label".to_string()),
                XDataValue::String("class=PIDInstrument".to_string()),
                XDataValue::String("label=PG-5002".to_string()),
                XDataValue::String("oid=502".to_string()),
                XDataValue::String("resolved=direct".to_string()),
            ]),
        );

        // 3. Derived non-symbol point (`PIDPipingPort`) must NOT be promoted as an authored symbol
        let mut port_pt = Point::new();
        port_pt.location = Vector3::new(77.0, 100.0, 0.0);
        port_pt.common.layer = "PID-POINT".to_string();
        let port_h = doc.add_entity(EntityType::Point(port_pt)).unwrap();
        set_entity_xdata(
            &mut doc,
            port_h,
            PID_SEMANTICS_XDATA_APP,
            Some(vec![
                XDataValue::String("role=point-ok".to_string()),
                XDataValue::String("class=PIDPipingPort".to_string()),
                XDataValue::String("oid=601".to_string()),
            ]),
        );

        // 4. Multi-vertex `LwPolyline` process pipe (`oid=701`) + complex-dash tick `Line` sharing `oid=701`
        let mut pipe_pl = LwPolyline::new();
        pipe_pl.common.layer = "Process".to_string();
        for (x, y) in [(30.0, 100.0), (77.0, 100.0), (136.0, 100.0)] {
            pipe_pl.vertices.push(LwVertex::from_coords(x, y));
        }
        let pipe_pl_h = doc.add_entity(EntityType::LwPolyline(pipe_pl)).unwrap();
        let pipe_sem = vec![
            XDataValue::String("role=geometry".to_string()),
            XDataValue::String("class=PIDPipingConnector".to_string()),
            XDataValue::String("label=80-PG-5001-B9-".to_string()),
            XDataValue::String("oid=701".to_string()),
        ];
        set_entity_xdata(
            &mut doc,
            pipe_pl_h,
            PID_SEMANTICS_XDATA_APP,
            Some(pipe_sem.clone()),
        );
        let mut tick = Line::from_points(
            Vector3::new(50.0, 99.5, 0.0),
            Vector3::new(50.0, 100.5, 0.0),
        );
        tick.common.layer = "Process".to_string();
        let tick_h = doc.add_entity(EntityType::Line(tick)).unwrap();
        set_entity_xdata(&mut doc, tick_h, PID_SEMANTICS_XDATA_APP, Some(pipe_sem));

        let rep = promote_semantics_to_author(&mut doc, None);
        let rev = summarize_author_review(&doc);
        assert_eq!(
            rev.total_authored_symbols, 2,
            "multi-stroke .pid symbols sharing oid must merge into 2 authored symbols, got {rev:?} ({rep:?})"
        );
        assert_eq!(
            rev.duplicated_tags, 0,
            "merged strokes must not trigger fake duplicate tags"
        );
        assert!(
            rep.pipe_segments_promoted >= 1,
            "imported .pid LwPolyline pipe must promote into TopologyTable"
        );

        // Edit valve tag -> visible label text updates, while PID-SYMBOL-LABEL template name stays untouched
        let valve_owner = filter_author_handles(&doc, "ALL")
            .into_iter()
            .find(|&h| PidAuthor::load(&doc, h).is_some_and(|a| a.tag == "BV-5001"))
            .expect("merged BV-5001 owner");
        let mut v_auth = PidAuthor::load(&doc, valve_owner).unwrap();
        v_auth.tag = "BV-5001-EDITED".to_string();
        v_auth.tag_source = "manual".to_string();
        v_auth.store(&mut doc, valve_owner);
        let _ = sync_paired_canvas_tag_texts(&mut doc);

        match doc.get_entity(vis_tag_h) {
            Some(EntityType::Text(t)) => assert_eq!(t.value, "BV-5001-EDITED"),
            other => panic!("expected visible Text, got {other:?}"),
        }
        match doc.get_entity(sym_lbl_h) {
            Some(EntityType::Text(t)) => assert_eq!(t.value, "Ball Valve Type 2"),
            other => panic!("expected symbol-label Text, got {other:?}"),
        }

        let opts = ConvertOptions {
            plant: "P01".to_string(),
            seed_pid_bytes: Some(synthetic_blank_seed_pid()),
            ..Default::default()
        };
        let dual =
            save_author_document_to_dual(&doc, "PID-IMPORT-142", &opts).expect("save dual 142");
        assert_eq!(dual.report.variant_resolution.class_default_fallback, 0);
        assert_eq!(dual.report.variant_resolution.exact_variant_id, 2);
        assert!(dual.report.piping_synthesis.pipelines >= 1);
        let xml_txt = String::from_utf8_lossy(&dual.publish.data_xml);
        assert!(xml_txt.contains("BV-5001-EDITED"));
        assert!(xml_txt.contains("PG-5002"));
        assert!(dual
            .report
            .pid_verification
            .as_ref()
            .is_some_and(super::super::export::DualVerifyReport::is_cross_file_and_a_valid));
    }
}

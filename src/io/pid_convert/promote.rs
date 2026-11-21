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
    compute_representation_uid_map, load_configured_seed, pick_seed_style_ids, verify_dual_bundle,
    DualConvertBundle,
};
use super::export::{ConvertOptions, ConvertSheetReport, DeadEndAttribution, PipeNetworkCensus};
use super::recognize::{recognise_sppid_sheet, ConfidenceSummary, ConfidenceTier, TitleBlockInfo};
use crate::io::pid_legend::guess_units_per_mm;
use crate::io::pid_publish::{
    export_drawing_bundle, parse_sppid_line_number, pipeline_uid, populate_item_tag_attributes,
    synthesize_piping_network, ExportDrawing, ExportItem, PipeRunInput, SymbolEndpointRef,
    UidAllocator,
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
        pmap.insert(
            "_source".to_string(),
            serde_json::Value::String("convert:dxf".to_string()),
        );
        pmap.insert("_confidence".to_string(), serde_json::json!(sym.confidence));
        pmap.insert("_confidence_tier".to_string(), serde_json::json!(sym.tier));
        pmap.insert(
            "_sppid_class".to_string(),
            serde_json::Value::String(sym.sppid_class.clone()),
        );
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
        }
        if let Some(ref nd) = sym.nominal_diameter {
            pmap.insert(
                "_nominal_diameter".to_string(),
                serde_json::Value::String(nd.clone()),
            );
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
    let pipe_layer_set: BTreeSet<String> = opts
        .pipe_layers
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .collect();
    let mut raw_lines = Vec::new();
    for ent in doc.model_space_entities() {
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
                raw_lines.push((ent.common().handle, a, b));
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

            let mut seg_auth = PidAuthor::load(doc, sh).unwrap_or_default();
            seg_auth.type_code = TYPE_PIPE_SEGMENT.to_string();
            seg_auth.topology = ROLE_PIPE_SEGMENT.to_string();
            let mut smap = parse_props_map(&seg_auth.props);
            smap.insert(
                "_line_label".to_string(),
                serde_json::Value::String(line_tag.clone()),
            );
            seg_auth.props = serde_json::Value::Object(smap).to_string();
            seg_auth.store(doc, sh);
        }
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

/// Promote imported `.pid` entities carrying `PID_SEMANTICS` into `PID_AUTHOR` (`L2-6`).
pub fn promote_semantics_to_author(
    doc: &mut Document,
    filter_handles: Option<&BTreeSet<Handle>>,
) -> PromoteReport {
    let mut candidates: Vec<(Handle, BTreeMap<String, String>)> = Vec::new();
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
        if !kv.is_empty() {
            candidates.push((h, kv));
        }
    }

    let mut report = PromoteReport::default();
    for (h, kv) in candidates {
        let (sppid_cls, auth_ty) = infer_sppid_from_semantics(&kv);
        let existing = PidAuthor::load(doc, h);
        let is_update = existing.is_some();
        let mut author = existing.unwrap_or_default();
        author.type_code = auth_ty;

        if author.tag_source == "manual" {
            report.manual_tags_preserved += 1;
        } else if let Some(lbl) = kv.get("label") {
            author.tag = lbl.clone();
            author.tag_source = "auto".to_string();
        } else if author.tag_source.is_empty() {
            author.tag_source = "auto".to_string();
        }
        if author.tag_status.is_empty() {
            author.tag_status = "ok".to_string();
        }

        let mut pmap = parse_props_map(&author.props);
        pmap.insert(
            "_source".to_string(),
            serde_json::Value::String("import:pid".to_string()),
        );
        pmap.insert(
            "_sppid_class".to_string(),
            serde_json::Value::String(sppid_cls.clone()),
        );
        pmap.insert(
            "_variant_id".to_string(),
            serde_json::Value::String(format!("{sppid_cls}#01")),
        );
        if let Some(oid_str) = kv.get("oid") {
            pmap.insert(
                "_graphic_oid".to_string(),
                serde_json::Value::String(oid_str.clone()),
            );
        }
        if let Some(inst_str) = kv.get("instance") {
            pmap.insert(
                "_symbol_path".to_string(),
                serde_json::Value::String(inst_str.clone()),
            );
        }
        author.props = serde_json::Value::Object(pmap).to_string();
        author.store(doc, h);

        if is_update {
            report.updated_in_place += 1;
        } else {
            report.newly_promoted += 1;
        }
    }
    let _ = refresh_duplicate_tag_statuses(doc);
    report
}

fn entity_world_anchor(ent: &EntityType) -> Option<(f64, f64)> {
    match ent {
        EntityType::Insert(ins) => Some((ins.insert_point.x, ins.insert_point.y)),
        EntityType::Text(t) => Some((t.insertion_point.x, t.insertion_point.y)),
        EntityType::MText(mt) => Some((mt.insertion_point.x, mt.insertion_point.y)),
        _ => None,
    }
}

/// Reconcile `TopologyTable` and `PID_AUTHOR` tag uniqueness after moving/deleting/editing entities (`L2-4 / L2-5`).
pub fn sync_author_after_edit(doc: &mut Document) -> AuthorSyncReport {
    let mut topo = TopologyTable::load(doc);
    let pruned_records = topo.prune(doc);

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
            match ent {
                EntityType::Insert(ins) => (
                    h,
                    ins.block_name.clone(),
                    Some((ins.insert_point.x, ins.insert_point.y)),
                    None,
                ),
                EntityType::Text(t) => (
                    h,
                    "TEXT".to_string(),
                    Some((t.insertion_point.x, t.insertion_point.y)),
                    None,
                ),
                EntityType::Line(ln) => (
                    h,
                    "LINE".to_string(),
                    None,
                    Some(((ln.start.x, ln.start.y), (ln.end.x, ln.end.y))),
                ),
                _ => (h, String::new(), None, None),
            }
        })
        .collect();

    for (h, blk_name, pt_opt, seg_opt) in handles_and_geom {
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap = parse_props_map(&auth.props);

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            if let Some((a, b)) = seg_opt {
                let goid = alloc.alloc_graphic_oid(None);
                let lbl = pmap
                    .get("_line_label")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                run_inputs.push(PipeRunInput {
                    start: a,
                    end: b,
                    line_label: lbl,
                    graphic_oid: Some(goid),
                });
                seg_draws.push((goid, (a.0 / upm, a.1 / upm), (b.0 / upm, b.1 / upm)));
            }
            continue;
        }

        let Some(at) = pt_opt else {
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
            .unwrap_or_else(|| format!("Symbols\\{blk_name}.sym"));

        let uid = alloc.alloc_primary_uid(None, &format!("author_sym:{:X}", h.value()));
        let goid = alloc.alloc_graphic_oid(None);

        let mut attrs: BTreeMap<(String, String), String> = BTreeMap::new();
        let tf_txt = pmap
            .get("_to_from_text")
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
        if let Some(nd) = pmap.get("_nominal_diameter").and_then(|v| v.as_str()) {
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
        let r = 4.0 * upm;
        let bbox = (at.0 - r, at.1 - r, at.0 + r, at.1 + r);
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

    let mut publish = export_drawing_bundle(&dwg)?;

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

        let clamp_pt = |pt: (f64, f64)| -> (f64, f64) {
            (
                pt.0.clamp(10.0, (page_mm.0 - 10.0).max(20.0)),
                pt.1.clamp(10.0, (page_mm.1 - 10.0).max(20.0)),
            )
        };

        let mut inc = PidWriteIncrements {
            doc_uid: Some(doc_uid.clone()),
            plant_name: Some(opts.plant.clone()),
            template_name: inferred_tpl.clone(),
            ..Default::default()
        };

        for sd in &sym_draws {
            let p = clamp_pt(sd.at_mm);
            inc.symbols.push(AddedSymbolSpec {
                preferred_oid: Some(sd.graphic_oid),
                symbol_rel_path: sd.sym_path.clone(),
                insertion_mm: p,
                rotation_rad: 0.0,
                scale: 1.0,
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
                start_mm: clamp_pt(a_mm),
                end_mm: clamp_pt(b_mm),
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
}

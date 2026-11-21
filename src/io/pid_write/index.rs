//! Seed-plus-increment `.pid` stream assembler, `JSheetLayer` / `PSMspacemap` /
//! `JSitesList` index synchronizer, and post-write index consistency checker
//! (`L5-4` / `L5-5` / `#129`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use pid_parse::parsers::attribute_rows::decode_attribute_row;
use pid_parse::parsers::cluster_header::decode_psm_cluster0_body_records;
use pid_parse::parsers::jsites_list::{parse_jsites_list, JSITES_LIST_MAGIC};
use pid_parse::parsers::psm_tables::{parse_psm_space_map, SPACE_MAP_MAGIC};
use pid_parse::parsers::sheet_layers::PSM_TYPE_CODE_JSHEET_LAYER;
use pid_parse::parsers::sheet_records::{
    decode_iglines, decode_iglinestrings, decode_igsymbols, decode_igtextboxes, jsheet_oids,
    parse_live_psm_header, sheet_record_starts, SheetChain,
};
use pid_parse::{PidPackage, PidWriter, WritePlan};
use serde::{Deserialize, Serialize};

use super::oid::SeedOidPool;
use super::records::{
    build_fallback_jsite_streams, encode_freeform_attr_row_record, encode_igline2d_record,
    encode_iglinestring2d_record, encode_igsymbol2d_record, encode_igtextbox_record,
    encode_jproperties_for_sym_path, mm_to_sheet_m, SymbolRecordParams,
};
use super::seed::stamp_seed_drawing_identity;

const CHAIN_MAGIC: u32 = 0x6C90_F544;
const SPACE_MAP_ENTRY_LIVE: u32 = 0x0002_0000;
const GRAPHIC_LAYER_WRITING_TYPES: &[u16] = &[
    0x0013, 0x0018, 0x0020, 0x003D, 0x004D, 0x0059, 0x005D, 0x005E, 0x0061, 0x0084, 0x00CE, 0x0115,
];

/// Line primitive to add onto the sheet (`mm` coordinates converted to sheet metres).
#[derive(Debug, Clone)]
pub struct AddedLineSpec {
    pub preferred_oid: Option<u32>,
    pub start_mm: (f64, f64),
    pub end_mm: (f64, f64),
    pub style_index: u32,
}

/// Polyline primitive (`>= 2` vertices in `mm`) to add onto the sheet.
#[derive(Debug, Clone)]
pub struct AddedPolylineSpec {
    pub preferred_oid: Option<u32>,
    pub vertices_mm: Vec<(f64, f64)>,
    pub style_index: u32,
}

/// Text annotation (`mm` insertion + radians rotation) to add onto the sheet.
#[derive(Debug, Clone)]
pub struct AddedTextSpec {
    pub preferred_oid: Option<u32>,
    pub text: String,
    pub insertion_mm: (f64, f64),
    pub rotation_rad: f64,
    pub style_index: u32,
}

/// Symbol placement (`0x00CE igSymbol2d` + embedded `/JSite<id>` body cache).
#[derive(Debug, Clone)]
pub struct AddedSymbolSpec {
    pub preferred_oid: Option<u32>,
    pub symbol_rel_path: String,
    pub insertion_mm: (f64, f64),
    pub rotation_rad: f64,
    pub scale: f64,
    pub style_ref: u32,
}

/// `0x0089 FreeFormAttrSet` (`P&IDAttributes`) linking a `GraphicOID` to its XML `UID`.
#[derive(Debug, Clone)]
pub struct AddedAttributeSpec {
    pub described_graphic_oid: u32,
    pub model_uid: String,
    pub representation_uid: String,
    pub sppid_class: String,
    pub drawing_no: String,
    pub item_tag: Option<String>,
}

/// Declarative batch of increments to apply onto a seed `PidPackage`.
#[derive(Debug, Clone, Default)]
pub struct PidWriteIncrements {
    pub doc_uid: Option<String>,
    pub plant_name: Option<String>,
    pub template_name: Option<String>,
    pub lines: Vec<AddedLineSpec>,
    pub polylines: Vec<AddedPolylineSpec>,
    pub texts: Vec<AddedTextSpec>,
    pub symbols: Vec<AddedSymbolSpec>,
    pub attributes: Vec<AddedAttributeSpec>,
}

/// Summary of OIDs allocated and records appended during `apply_increments_to_seed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidWriteStats {
    pub line_oids: Vec<u32>,
    pub polyline_oids: Vec<u32>,
    pub text_oids: Vec<u32>,
    pub symbol_oids: Vec<u32>,
    pub attribute_row_oids: Vec<u32>,
    pub cloned_jsite_ids: Vec<u32>,
}

/// Post-write index & record-chain consistency check report (`L5-5`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCheckReport {
    pub chain_fault_count: usize,
    pub duplicate_oids: Vec<u32>,
    pub dangling_spacemap_refs: Vec<String>,
    pub layer_count_mismatches: Vec<String>,
    pub missing_jsites: Vec<u32>,
    pub decoded_lines: usize,
    pub decoded_polylines: usize,
    pub decoded_texts: usize,
    pub decoded_symbols: usize,
    pub decoded_attr_rows: usize,
}

impl IndexCheckReport {
    pub fn is_consistent(&self) -> bool {
        self.chain_fault_count == 0
            && self.duplicate_oids.is_empty()
            && self.dangling_spacemap_refs.is_empty()
            && self.layer_count_mismatches.is_empty()
            && self.missing_jsites.is_empty()
    }
}

fn append_to_chain_stream(existing: Option<&[u8]>, appended: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = match existing {
        Some(raw) if !raw.is_empty() => {
            let walk = SheetChain::walk(raw);
            if walk.wrong_magic().is_some() || walk.stop.is_some() {
                return Err(format!(
                    "target chain stream is not a clean 0x6C90F544 record chain (len={}, end={})",
                    raw.len(),
                    walk.end
                ));
            }
            raw.to_vec()
        }
        _ => {
            let mut init = CHAIN_MAGIC.to_le_bytes().to_vec();
            init.extend_from_slice(&0u32.to_le_bytes());
            init
        }
    };
    out.extend_from_slice(appended);
    let final_walk = SheetChain::walk(&out);
    if final_walk.stop.is_some() {
        return Err("appended records broke 0x6C90F544 chain framing".into());
    }
    let live = u32::try_from(final_walk.live_records).unwrap_or(u32::MAX);
    out[4..8].copy_from_slice(&live.to_le_bytes());
    Ok(out)
}

fn update_psmcluster0_layer_counts(
    cluster_bytes: &[u8],
    layer_deltas: &BTreeMap<u32, u32>,
) -> Vec<u8> {
    let mut updated = cluster_bytes.to_vec();
    if layer_deltas.is_empty() {
        return updated;
    }
    for rec in decode_psm_cluster0_body_records(cluster_bytes) {
        if rec.type_code != PSM_TYPE_CODE_JSHEET_LAYER || rec.raw_payload.len() < 16 {
            continue;
        }
        let oid = u32::from_le_bytes([
            rec.raw_payload[0],
            rec.raw_payload[1],
            rec.raw_payload[2],
            rec.raw_payload[3],
        ]);
        if let Some(&delta) = layer_deltas.get(&oid) {
            if delta > 0 {
                let count_off = rec.byte_range.start + 6 + 12;
                if count_off + 4 <= updated.len() {
                    let old_cnt = u32::from_le_bytes([
                        updated[count_off],
                        updated[count_off + 1],
                        updated[count_off + 2],
                        updated[count_off + 3],
                    ]);
                    let new_cnt = old_cnt.saturating_add(delta);
                    updated[count_off..count_off + 4].copy_from_slice(&new_cnt.to_le_bytes());
                }
            }
        }
    }
    updated
}

fn ensure_jsites_list_contains(existing: Option<&[u8]>, new_sites: &[u32]) -> Vec<u8> {
    let mut entries: Vec<u32> = existing
        .and_then(parse_jsites_list)
        .map(|d| d.entries)
        .unwrap_or_default();
    for &sid in new_sites {
        if !entries.contains(&sid) {
            entries.push(sid);
        }
    }
    let mut buf = Vec::with_capacity(8 + entries.len() * 4);
    buf.extend_from_slice(&JSITES_LIST_MAGIC);
    buf.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for sid in entries {
        buf.extend_from_slice(&sid.to_le_bytes());
    }
    buf
}

fn resolve_or_clone_symbol_jsite(
    seed: &mut PidPackage,
    pool: &mut SeedOidPool,
    sym_rel_path: &str,
    sym_library_dir: Option<&Path>,
    cache: &mut BTreeMap<String, (u32, u32)>,
    cloned_ids: &mut Vec<u32>,
) -> Result<(u32, u32), String> {
    let norm_key = sym_rel_path
        .trim()
        .trim_start_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_uppercase();
    if let Some(&hit) = cache.get(&norm_key) {
        return Ok(hit);
    }

    // Reuse an existing JSite in the seed if its symbol_path ends with `norm_key`
    for js in &seed.parsed.jsites {
        let Some(num_str) = js.name.strip_prefix("JSite") else {
            continue;
        };
        let Ok(site_id) = num_str.parse::<u32>() else {
            continue;
        };
        if let Some(ref sp) = js.symbol_path {
            if sp
                .replace('/', "\\")
                .to_ascii_uppercase()
                .ends_with(&norm_key)
            {
                let def_sheet = js
                    .nested_geometry
                    .as_ref()
                    .and_then(|ng| ng.sheets.first().copied())
                    .unwrap_or(6);
                cache.insert(norm_key, (site_id, def_sheet));
                return Ok((site_id, def_sheet));
            }
        }
    }

    let site_id = pool.alloc_oid(None);
    let mut loaded_from_sym = None;

    if let Some(lib_dir) = sym_library_dir {
        let mut candidate = lib_dir.to_path_buf();
        for part in sym_rel_path
            .trim()
            .trim_start_matches(['\\', '/'])
            .split(['\\', '/'])
        {
            if !part.is_empty() {
                candidate.push(part);
            }
        }
        if candidate.is_file() {
            if let Ok(sym_pkg) = PidPackage::from_path(&candidate) {
                let def_sheet = sym_pkg
                    .get_stream("/PSMcluster0")
                    .and_then(|s| jsheet_oids(&s.data).into_iter().next())
                    .unwrap_or(6);
                let mut copied = Vec::new();
                for (sub_path, raw) in &sym_pkg.streams {
                    let leaf = sub_path.trim_start_matches('/');
                    if leaf == "\x05SummaryInformation" || leaf == "\x05DocumentSummaryInformation"
                    {
                        continue;
                    }
                    copied.push((format!("/JSite{site_id}{sub_path}"), raw.data.clone()));
                }
                copied.push((
                    format!("/JSite{site_id}/JProperties"),
                    encode_jproperties_for_sym_path(sym_rel_path),
                ));
                loaded_from_sym = Some((def_sheet, copied));
            }
        }
    }

    let (def_sheet_oid, site_streams) =
        loaded_from_sym.unwrap_or_else(|| build_fallback_jsite_streams(site_id, sym_rel_path));
    for (p, bytes) in site_streams {
        seed.replace_stream(p, bytes);
    }
    cloned_ids.push(site_id);
    cache.insert(norm_key, (site_id, def_sheet_oid));
    Ok((site_id, def_sheet_oid))
}

fn update_spacemap_segment(
    existing: Option<&[u8]>,
    max_index_used: u16,
    incoming_by_index: &BTreeMap<u16, Vec<(u32, u16)>>,
) -> Result<Vec<u8>, String> {
    let required_next = max_index_used
        .checked_add(1)
        .filter(|&n| n <= 0x2000)
        .ok_or_else(|| format!("PSMspacemap segment index overflow ({max_index_used})"))?;

    let (simple_cap, mut next_free, mut free_list, mut entry_map) =
        if let Some(sm) = existing.and_then(parse_psm_space_map) {
            let mut emap: BTreeMap<u16, (u16, Vec<(u32, u16)>)> = BTreeMap::new();
            for e in sm.entries {
                let live: Vec<(u32, u16)> = e
                    .members
                    .into_iter()
                    .take(usize::from(e.live_member_count))
                    .filter(|m| m.value != 0 && m.tag != 0)
                    .map(|m| (m.value, m.tag))
                    .collect();
                emap.insert(e.index, (e.form, live));
            }
            (
                sm.simple_slot_capacity.max(4),
                sm.next_free_index,
                sm.free_list,
                emap,
            )
        } else {
            (4u16, 1u16, Vec::new(), BTreeMap::new())
        };

    if required_next > next_free {
        next_free = required_next;
    }
    // Ensure newly used indices are not left in free_list
    free_list.retain(|&idx| !incoming_by_index.contains_key(&idx) && idx < next_free);

    for (&target_idx, new_edges) in incoming_by_index {
        let slot = entry_map
            .entry(target_idx)
            .or_insert_with(|| (2u16, Vec::new()));
        for &(val, tag) in new_edges {
            if !slot.1.contains(&(val, tag)) {
                slot.1.push((val, tag));
            }
        }
    }

    let entry_count = u16::try_from(entry_map.len())
        .map_err(|_| "PSMspacemap entry count overflow".to_string())?;
    let free_len =
        u16::try_from(free_list.len()).map_err(|_| "PSMspacemap free_list overflow".to_string())?;

    let mut out = Vec::new();
    out.extend_from_slice(&SPACE_MAP_MAGIC.to_le_bytes());
    out.extend_from_slice(&entry_count.to_le_bytes());
    out.extend_from_slice(&simple_cap.to_le_bytes());
    out.extend_from_slice(&next_free.to_le_bytes());
    out.extend_from_slice(&free_len.to_le_bytes());
    for f in free_list {
        out.extend_from_slice(&f.to_le_bytes());
    }

    for (idx, (form, live_members)) in entry_map {
        let form_bits = u32::from(form) | (SPACE_MAP_ENTRY_LIVE >> 16);
        let head = (form_bits << 16) | u32::from(idx);
        let live_cnt = u16::try_from(live_members.len())
            .map_err(|_| "PSMspacemap member count overflow".to_string())?;
        let cap = live_cnt.max(2);
        out.extend_from_slice(&head.to_le_bytes());
        out.extend_from_slice(&live_cnt.to_le_bytes());
        out.extend_from_slice(&cap.to_le_bytes());
        for &(val, tag) in &live_members {
            out.extend_from_slice(&val.to_le_bytes());
            out.extend_from_slice(&tag.to_le_bytes());
        }
        for _ in live_members.len()..usize::from(cap) {
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        }
    }

    Ok(out)
}

/// Apply `increments` onto `seed`, synchronizing `/Sheet*`, `/Unclustered Dynamic Attributes`,
/// `/PSMcluster0` (`JSheetLayer` object counts), `/PSMspacemap/*`, `/JSite*/*`, `/JSitesList`,
/// and `/\x05DocumentSummaryInformation` Section 2.
pub fn apply_increments_to_seed(
    seed: &mut PidPackage,
    increments: &PidWriteIncrements,
    sym_library_dir: Option<&Path>,
) -> Result<PidWriteStats, String> {
    if let Some(ref duid) = increments.doc_uid {
        let plant = increments.plant_name.as_deref().unwrap_or("P01");
        stamp_seed_drawing_identity(seed, duid, plant, increments.template_name.as_deref())?;
    }

    let mut pool = SeedOidPool::from_seed(seed);
    let sheet_path = pool.targets().sheet_stream_path.clone();
    let default_layer = pool.targets().default_layer_oid;
    let labels_layer = pool.targets().labels_layer_oid;

    let mut stats = PidWriteStats::default();
    let mut sheet_appended = Vec::new();
    let mut da_appended = Vec::new();
    let mut layer_deltas: BTreeMap<u32, u32> = BTreeMap::new();
    let mut incoming_edges: BTreeMap<u32, Vec<(u32, u16)>> = BTreeMap::new();
    let mut sym_site_cache: BTreeMap<String, (u32, u32)> = BTreeMap::new();

    for ln in &increments.lines {
        let oid = pool.alloc_oid(ln.preferred_oid);
        let rec = encode_igline2d_record(
            oid,
            default_layer,
            ln.style_index,
            (mm_to_sheet_m(ln.start_mm.0), mm_to_sheet_m(ln.start_mm.1)),
            (mm_to_sheet_m(ln.end_mm.0), mm_to_sheet_m(ln.end_mm.1)),
        )?;
        sheet_appended.extend_from_slice(&rec);
        *layer_deltas.entry(default_layer).or_default() += 1;
        stats.line_oids.push(oid);
    }

    for pl in &increments.polylines {
        let oid = pool.alloc_oid(pl.preferred_oid);
        let verts_m: Vec<(f64, f64)> = pl
            .vertices_mm
            .iter()
            .map(|&(x, y)| (mm_to_sheet_m(x), mm_to_sheet_m(y)))
            .collect();
        let rec = encode_iglinestring2d_record(oid, default_layer, pl.style_index, &verts_m)?;
        sheet_appended.extend_from_slice(&rec);
        *layer_deltas.entry(default_layer).or_default() += 1;
        stats.polyline_oids.push(oid);
    }

    for tx in &increments.texts {
        let oid = pool.alloc_oid(tx.preferred_oid);
        let rec = encode_igtextbox_record(
            oid,
            0,
            labels_layer,
            tx.style_index,
            &tx.text,
            (
                mm_to_sheet_m(tx.insertion_mm.0),
                mm_to_sheet_m(tx.insertion_mm.1),
            ),
            tx.rotation_rad,
        )?;
        sheet_appended.extend_from_slice(&rec);
        *layer_deltas.entry(labels_layer).or_default() += 1;
        stats.text_oids.push(oid);
    }

    for sym in &increments.symbols {
        let oid = pool.alloc_oid(sym.preferred_oid);
        let (site_id, def_sheet_oid) = resolve_or_clone_symbol_jsite(
            seed,
            &mut pool,
            &sym.symbol_rel_path,
            sym_library_dir,
            &mut sym_site_cache,
            &mut stats.cloned_jsite_ids,
        )?;
        let (sin_v, cos_v) = sym.rotation_rad.sin_cos();
        let rec = encode_igsymbol2d_record(&SymbolRecordParams {
            oid,
            sheet_layer_oid: default_layer,
            style_ref: sym.style_ref,
            jsite_ref: site_id,
            transform: [cos_v, -sin_v, sin_v, cos_v],
            insertion_m: (
                mm_to_sheet_m(sym.insertion_mm.0),
                mm_to_sheet_m(sym.insertion_mm.1),
            ),
            uniform_scale: sym.scale,
            definition_sheet_ref: def_sheet_oid,
            definition_site_ref: site_id,
        })?;
        sheet_appended.extend_from_slice(&rec);
        *layer_deltas.entry(default_layer).or_default() += 1;
        incoming_edges.entry(site_id).or_default().push((oid, 181));
        if pool.is_used(2) {
            incoming_edges.entry(site_id).or_default().push((2, 182));
        }
        stats.symbol_oids.push(oid);
    }

    for attr in &increments.attributes {
        if !pool.is_used(attr.described_graphic_oid) {
            return Err(format!(
                "AddedAttributeSpec describes unknown GraphicOID {}",
                attr.described_graphic_oid
            ));
        }
        let row_oid = pool.alloc_oid(None);
        let mut fields = vec![
            ("ModelID", attr.model_uid.as_str()),
            ("DrawingID", attr.representation_uid.as_str()),
            ("ModelItemType", attr.sppid_class.as_str()),
            ("DrawingNo", attr.drawing_no.as_str()),
        ];
        if let Some(ref tag) = attr.item_tag {
            if !tag.is_empty() {
                fields.push(("ItemTag", tag.as_str()));
            }
        }
        let rec = encode_freeform_attr_row_record(
            row_oid,
            attr.described_graphic_oid,
            "P&IDAttributes",
            &fields,
        )?;
        da_appended.extend_from_slice(&rec);
        incoming_edges
            .entry(attr.described_graphic_oid)
            .or_default()
            .push((row_oid, 190));
        stats.attribute_row_oids.push(row_oid);
    }

    if !sheet_appended.is_empty() {
        let new_sheet = append_to_chain_stream(
            seed.get_stream(&sheet_path).map(|s| s.data.as_slice()),
            &sheet_appended,
        )?;
        seed.replace_stream(sheet_path, new_sheet);
    }

    if !da_appended.is_empty() {
        let da_path = "/Unclustered Dynamic Attributes";
        let new_da = append_to_chain_stream(
            seed.get_stream(da_path).map(|s| s.data.as_slice()),
            &da_appended,
        )?;
        seed.replace_stream(da_path, new_da);
    }

    if !layer_deltas.is_empty() {
        if let Some(raw_cluster) = seed.get_stream("/PSMcluster0").map(|s| s.data.clone()) {
            let new_cluster = update_psmcluster0_layer_counts(&raw_cluster, &layer_deltas);
            seed.replace_stream("/PSMcluster0", new_cluster);
        }
    }

    if !stats.cloned_jsite_ids.is_empty() {
        let new_list = ensure_jsites_list_contains(
            seed.get_stream("/JSitesList").map(|s| s.data.as_slice()),
            &stats.cloned_jsite_ids,
        );
        seed.replace_stream("/JSitesList", new_list);
    }

    // Synchronize `/PSMspacemap/0x...` per 8,192-OID segment
    let mut segments_touched: BTreeSet<u32> = pool.used_oids().iter().map(|&id| id >> 13).collect();
    for &target_oid in incoming_edges.keys() {
        segments_touched.insert(target_oid >> 13);
    }
    for seg in segments_touched {
        let sm_path = format!("/PSMspacemap/0x{:08x}", seg << 13);
        let max_idx = pool.max_allocated_in_segment(seg).unwrap_or(1);
        let seg_edges: BTreeMap<u16, Vec<(u32, u16)>> = incoming_edges
            .iter()
            .filter(|(&target_oid, _)| (target_oid >> 13) == seg)
            .map(|(&target_oid, edges)| ((target_oid & 0x1FFF) as u16, edges.clone()))
            .collect();
        if !seg_edges.is_empty() || seed.get_stream(&sm_path).is_some() || seg == 0 {
            let new_sm = update_spacemap_segment(
                seed.get_stream(&sm_path).map(|s| s.data.as_slice()),
                max_idx,
                &seg_edges,
            )?;
            seed.replace_stream(sm_path, new_sm);
        }
    }

    // Refresh `seed.parsed` via in-memory CFB round-trip so sequential calls see live state.
    let cfb_bytes = PidWriter::write_to_bytes(seed, &WritePlan::default())
        .map_err(|e| format!("serialize updated seed package: {e}"))?;
    *seed = PidPackage::from_bytes(&cfb_bytes).map_err(|e| format!("reparse updated seed: {e}"))?;

    Ok(stats)
}

/// Serialize `pkg` via `PidWriter::write_to_bytes`, re-parse via `PidPackage::from_bytes`,
/// and verify all 5 Q9 index & chain invariants (`L5-5`).
pub fn verify_package_index_consistency(pkg: &PidPackage) -> Result<IndexCheckReport, String> {
    let bytes = PidWriter::write_to_bytes(pkg, &WritePlan::default())
        .map_err(|e| format!("write_to_bytes: {e}"))?;
    let reparsed = PidPackage::from_bytes(&bytes).map_err(|e| format!("reparse: {e}"))?;

    let mut report = IndexCheckReport {
        chain_fault_count: reparsed.parsed.chain_faults.len(),
        ..Default::default()
    };

    let mut seen_oids: BTreeSet<u32> = BTreeSet::new();
    let mut dup_oids: BTreeSet<u32> = BTreeSet::new();
    let mut actual_layer_counts: BTreeMap<u32, u32> = BTreeMap::new();
    let mut referenced_jsites: BTreeSet<u32> = BTreeSet::new();

    for (path, raw) in &reparsed.streams {
        let trimmed = path.trim_start_matches('/');
        if trimmed.contains('/') {
            continue;
        }
        if trimmed.starts_with("Sheet") {
            let walk = SheetChain::walk(&raw.data);
            if !walk.is_sound() {
                report.chain_fault_count += 1;
            }
            for &at in &walk.starts {
                if let Some(hdr) = parse_live_psm_header(&raw.data, at) {
                    if let Some(b) = raw.data.get(hdr.body_start..hdr.body_start + 12) {
                        let oid = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        let layer_ref = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
                        if oid > 0 && !seen_oids.insert(oid) {
                            dup_oids.insert(oid);
                        }
                        if GRAPHIC_LAYER_WRITING_TYPES.contains(&hdr.type_code) && layer_ref > 0 {
                            *actual_layer_counts.entry(layer_ref).or_default() += 1;
                        }
                    }
                }
            }
            report.decoded_lines += decode_iglines(&raw.data).len();
            report.decoded_polylines += decode_iglinestrings(&raw.data).len();
            report.decoded_texts += decode_igtextboxes(&raw.data).len();
            let syms = decode_igsymbols(&raw.data);
            for s in &syms {
                referenced_jsites.insert(s.jsite_ref);
            }
            report.decoded_symbols += syms.len();
        } else if trimmed == "Unclustered Dynamic Attributes" {
            let walk = SheetChain::walk(&raw.data);
            if !walk.is_sound() {
                report.chain_fault_count += 1;
            }
            for at in sheet_record_starts(&raw.data) {
                if let Some(hdr) = parse_live_psm_header(&raw.data, at) {
                    if let Some(payload) = raw
                        .data
                        .get(hdr.body_start..hdr.body_start + hdr.bytes_to_follow as usize)
                    {
                        if payload.len() >= 4 {
                            let oid = u32::from_le_bytes([
                                payload[0], payload[1], payload[2], payload[3],
                            ]);
                            if oid > 0 && !seen_oids.insert(oid) {
                                dup_oids.insert(oid);
                            }
                        }
                        if hdr.type_code == 0x0089 && decode_attribute_row(payload).is_some() {
                            report.decoded_attr_rows += 1;
                        }
                    }
                }
            }
        } else if trimmed == "PSMcluster0" {
            for rec in decode_psm_cluster0_body_records(&raw.data) {
                if rec.raw_payload.len() >= 4 {
                    let oid = u32::from_le_bytes([
                        rec.raw_payload[0],
                        rec.raw_payload[1],
                        rec.raw_payload[2],
                        rec.raw_payload[3],
                    ]);
                    if oid > 0 && !seen_oids.insert(oid) {
                        dup_oids.insert(oid);
                    }
                }
            }
        }
    }

    report.duplicate_oids = dup_oids.into_iter().collect();

    // Check top-level `/PSMspacemap/0x...` segment invariants
    let mut seg_bounds: BTreeMap<u32, (u16, BTreeSet<u16>)> = BTreeMap::new();
    for (sm_path, sm) in &reparsed.parsed.psm_space_maps {
        if let Some(rest) = sm_path.strip_prefix("/PSMspacemap/0x") {
            if let Ok(raw_seg) = u32::from_str_radix(rest, 16) {
                let seg = raw_seg >> 13;
                let free_set: BTreeSet<u16> = sm.free_list.iter().copied().collect();
                seg_bounds.insert(seg, (sm.next_free_index, free_set));
            }
        }
    }
    for (sm_path, sm) in &reparsed.parsed.psm_space_maps {
        let Some(rest) = sm_path.strip_prefix("/PSMspacemap/0x") else {
            continue;
        };
        let Ok(raw_seg) = u32::from_str_radix(rest, 16) else {
            continue;
        };
        let seg = raw_seg >> 13;
        if sm.trailing_bytes != 0 {
            report.dangling_spacemap_refs.push(format!(
                "{sm_path}: trailing_bytes={} (expected 0)",
                sm.trailing_bytes
            ));
        }
        let free_set: BTreeSet<u16> = sm.free_list.iter().copied().collect();
        for entry in &sm.entries {
            if entry.index >= sm.next_free_index || free_set.contains(&entry.index) {
                report.dangling_spacemap_refs.push(format!(
                    "{sm_path}: entry index {} invalid (next_free={})",
                    entry.index, sm.next_free_index
                ));
            }
            for m in entry
                .members
                .iter()
                .take(usize::from(entry.live_member_count))
            {
                if m.value == 0 && m.tag == 0 {
                    continue;
                }
                let ref_seg = m.value >> 13;
                let ref_idx = (m.value & 0x1FFF) as u16;
                match seg_bounds.get(&ref_seg) {
                    Some((nf, fset)) if ref_idx < *nf && !fset.contains(&ref_idx) => {}
                    _ => {
                        report.dangling_spacemap_refs.push(format!(
                            "{sm_path}: entry {} (seg {seg}) has dangling referrer oid={} (tag={})",
                            entry.index, m.value, m.tag
                        ));
                    }
                }
            }
        }
    }

    // Check root JSheetLayer object counts (`+12`)
    if let Some(root_layers) = reparsed.parsed.sheet_layers.get("/") {
        for layer in root_layers {
            let actual = actual_layer_counts.get(&layer.oid).copied().unwrap_or(0);
            if layer.object_count != actual {
                report.layer_count_mismatches.push(format!(
                    "layer '{}' (oid={}): stated={} vs actual={}",
                    layer.name, layer.oid, layer.object_count, actual
                ));
            }
        }
    }

    // Check JSite existence for all placed symbols
    let present_jsites: BTreeSet<u32> = reparsed
        .parsed
        .jsites
        .iter()
        .filter_map(|js| js.name.strip_prefix("JSite")?.parse::<u32>().ok())
        .collect();
    for jid in referenced_jsites {
        if !present_jsites.contains(&jid) {
            report.missing_jsites.push(jid);
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_write::{load_seed_from_bytes, load_seed_from_refdata_zip};
    use pid_parse::{build_normalized_geometry, PidDocument, RawStream};

    fn minimal_seed() -> PidPackage {
        let mut streams = BTreeMap::new();
        let put = |s: &mut BTreeMap<String, RawStream>, path: &str, data: Vec<u8>| {
            s.insert(
                path.to_string(),
                RawStream {
                    path: path.to_string(),
                    data,
                    modified: false,
                },
            );
        };
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());
        put(&mut streams, "/Sheet6", empty_chain.clone());
        put(
            &mut streams,
            "/Unclustered Dynamic Attributes",
            empty_chain.clone(),
        );
        put(&mut streams, "/StyleCluster", empty_chain);

        let mut tseg = 0x6765_7374u32.to_le_bytes().to_vec();
        tseg.extend_from_slice(&0u16.to_le_bytes());
        tseg.extend_from_slice(&4u16.to_le_bytes());
        tseg.extend_from_slice(&20u16.to_le_bytes());
        tseg.extend_from_slice(&0u16.to_le_bytes());
        put(&mut streams, "/PSMspacemap/0x00000000", tseg);

        let raw = PidPackage::new(None, streams, PidDocument::default());
        let bytes = PidWriter::write_to_bytes(&raw, &WritePlan::default()).unwrap();
        PidPackage::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn encodes_all_five_record_families_roundtrips_and_passes_index_self_check() {
        let mut pkg = minimal_seed();
        let inc = PidWriteIncrements {
            doc_uid: Some("11223344556677889900AABBCCDDEEFF".to_string()),
            plant_name: Some("P01".to_string()),
            template_name: Some("A1-W-New.igr".to_string()),
            lines: vec![AddedLineSpec {
                preferred_oid: Some(150),
                start_mm: (100.0, 200.0),
                end_mm: (300.0, 200.0),
                style_index: 1,
            }],
            polylines: vec![AddedPolylineSpec {
                preferred_oid: Some(151),
                vertices_mm: vec![(300.0, 200.0), (450.0, 200.0), (450.0, 320.0)],
                style_index: 1,
            }],
            texts: vec![AddedTextSpec {
                preferred_oid: Some(152),
                text: "BV0201501B".to_string(),
                insertion_mm: (295.0, 205.0),
                rotation_rad: 0.0,
                style_index: 1,
            }],
            symbols: vec![AddedSymbolSpec {
                preferred_oid: Some(153),
                symbol_rel_path: r"Piping\Valves\2 Way Common\Ball Valve Type 2.sym".to_string(),
                insertion_mm: (300.0, 200.0),
                rotation_rad: 0.0,
                scale: 1.0,
                style_ref: 1,
            }],
            attributes: vec![AddedAttributeSpec {
                described_graphic_oid: 153,
                model_uid: "AABBCCDDEEFF00112233445566778899".to_string(),
                representation_uid: "99887766554433221100FFEEDDCCBBAA".to_string(),
                sppid_class: "PIDPipingComponent".to_string(),
                drawing_no: "SYNTH-01".to_string(),
                item_tag: Some("BV0201501B".to_string()),
            }],
        };

        let stats = apply_increments_to_seed(&mut pkg, &inc, None).expect("apply increments");
        assert_eq!(stats.line_oids, vec![150]);
        assert_eq!(stats.polyline_oids, vec![151]);
        assert_eq!(stats.text_oids, vec![152]);
        assert_eq!(stats.symbol_oids, vec![153]);
        assert_eq!(stats.attribute_row_oids.len(), 1);
        assert_eq!(stats.cloned_jsite_ids.len(), 1);

        let chk = verify_package_index_consistency(&pkg).expect("index check");
        assert!(chk.is_consistent(), "index check failed: {chk:?}");
        assert_eq!(chk.decoded_lines, 1);
        assert_eq!(chk.decoded_polylines, 1);
        assert_eq!(chk.decoded_texts, 1);
        assert_eq!(chk.decoded_symbols, 1);
        assert_eq!(chk.decoded_attr_rows, 1);

        let out_bytes = PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap();
        let reparsed = load_seed_from_bytes(&out_bytes).unwrap();
        let da_bytes = &reparsed
            .get_stream("/Unclustered Dynamic Attributes")
            .unwrap()
            .data;
        let starts = sheet_record_starts(da_bytes);
        assert_eq!(starts.len(), 1);
        let hdr = parse_live_psm_header(da_bytes, starts[0]).unwrap();
        let payload = &da_bytes[hdr.body_start..hdr.body_start + hdr.bytes_to_follow as usize];
        let row = decode_attribute_row(payload).expect("decode 0x0089 row");
        assert_eq!(row.described, 153);
        assert_eq!(
            row.text("ModelID"),
            Some("AABBCCDDEEFF00112233445566778899")
        );
        assert_eq!(row.text("ItemTag"), Some("BV0201501B"));
    }

    #[test]
    fn detects_dangling_spacemap_reference_when_corrupted() {
        let mut pkg = minimal_seed();
        let inc = PidWriteIncrements {
            lines: vec![AddedLineSpec {
                preferred_oid: Some(120),
                start_mm: (10.0, 10.0),
                end_mm: (80.0, 10.0),
                style_index: 1,
            }],
            ..Default::default()
        };
        apply_increments_to_seed(&mut pkg, &inc, None).unwrap();

        let mut sm = pkg
            .get_stream("/PSMspacemap/0x00000000")
            .unwrap()
            .data
            .clone();
        let old_cnt = u16::from_le_bytes([sm[4], sm[5]]);
        sm[4..6].copy_from_slice(&(old_cnt + 1).to_le_bytes());
        let head: u32 = 0x0002_0000 | 120;
        sm.extend_from_slice(&head.to_le_bytes());
        sm.extend_from_slice(&1u16.to_le_bytes());
        sm.extend_from_slice(&1u16.to_le_bytes());
        sm.extend_from_slice(&7999u32.to_le_bytes());
        sm.extend_from_slice(&190u16.to_le_bytes());
        pkg.replace_stream("/PSMspacemap/0x00000000", sm);

        let chk = verify_package_index_consistency(&pkg).expect("run check");
        assert!(!chk.is_consistent());
        assert!(!chk.dangling_spacemap_refs.is_empty());
    }

    #[test]
    #[ignore = "adds geometry, text, real .sym symbol, and attributes to real A1-W-New.pid seed"]
    fn real_a1_seed_incremental_write_and_normalized_geometry_roundtrip() {
        let Ok(zip_path) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("未设置种子压缩包（OCS_PID_SEED_ZIP），跳过真实种子增补测试。");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR")
            .ok()
            .map(std::path::PathBuf::from);
        let mut pkg =
            load_seed_from_refdata_zip(Path::new(&zip_path), "A1-W-New.pid").expect("load A1 seed");

        let before_geom = build_normalized_geometry(&pkg.parsed);
        let inc = PidWriteIncrements {
            doc_uid: Some("ABCDEF0123456789ABCDEF0123456789".to_string()),
            plant_name: Some("P01".to_string()),
            template_name: Some("A1-W - New.igr".to_string()),
            lines: vec![AddedLineSpec {
                preferred_oid: None,
                start_mm: (120.0, 300.0),
                end_mm: (280.0, 300.0),
                style_index: 1,
            }],
            polylines: vec![AddedPolylineSpec {
                preferred_oid: None,
                vertices_mm: vec![(280.0, 300.0), (420.0, 300.0), (420.0, 380.0)],
                style_index: 1,
            }],
            texts: vec![AddedTextSpec {
                preferred_oid: None,
                text: "BV0201501B".to_string(),
                insertion_mm: (275.0, 308.0),
                rotation_rad: 0.0,
                style_index: 1,
            }],
            symbols: vec![AddedSymbolSpec {
                preferred_oid: None,
                symbol_rel_path: r"Piping\Valves\2 Way Common\Ball Valve Type 2.sym".to_string(),
                insertion_mm: (280.0, 300.0),
                rotation_rad: 0.0,
                scale: 1.0,
                style_ref: 1,
            }],
            attributes: Vec::new(),
        };

        let stats = apply_increments_to_seed(&mut pkg, &inc, sym_dir.as_deref()).unwrap();
        let sym_oid = stats.symbol_oids[0];
        let attr_inc = PidWriteIncrements {
            attributes: vec![AddedAttributeSpec {
                described_graphic_oid: sym_oid,
                model_uid: "1234567890ABCDEF1234567890ABCDEF".to_string(),
                representation_uid: "FEDCBA0987654321FEDCBA0987654321".to_string(),
                sppid_class: "PIDPipingComponent".to_string(),
                drawing_no: "A1-SYNTH".to_string(),
                item_tag: Some("BV0201501B".to_string()),
            }],
            ..Default::default()
        };
        apply_increments_to_seed(&mut pkg, &attr_inc, sym_dir.as_deref()).unwrap();

        let chk = verify_package_index_consistency(&pkg).expect("verify real A1 seed");
        assert!(
            chk.is_consistent(),
            "real A1 seed index check failed: {chk:?}"
        );

        let out_bytes = PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap();
        let reparsed = load_seed_from_bytes(&out_bytes).unwrap();
        let after_geom = build_normalized_geometry(&reparsed.parsed);
        assert!(
            after_geom.entities.len() >= before_geom.entities.len() + 4,
            "expected >= 4 added normalized geometry entities, before={} after={}",
            before_geom.entities.len(),
            after_geom.entities.len()
        );
        println!(
            "real_a1_seed entities_before={} entities_after={} chk={chk:?}",
            before_geom.entities.len(),
            after_geom.entities.len()
        );
    }
}

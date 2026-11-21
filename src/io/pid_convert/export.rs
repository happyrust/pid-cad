//! Single-sheet and batch `DWG/DXF -> SmartPlant publish XML + Coverage report`
//! conversion pipeline (`L4-6` / `L4-7` / `#127`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::Document;
use serde::{Deserialize, Serialize};

use super::class_map::default_legend_class_map;
use super::recognize::{
    recognise_sppid_sheet, ConfidenceSummary, ConfidenceThresholds, ConfidenceTier, PipeLabelHit,
    PipeSegmentHit,
};
use crate::io::pid_publish::{
    export_drawing_bundle, parse_sppid_line_number, pipeline_uid, populate_item_tag_attributes,
    synthesize_piping_network, ExportBundle, ExportDrawing, ExportItem, PipeRunInput,
    PipingSynthesisStats, SymbolEndpointRef, UidAllocator, UnmappedItem,
};

/// Options controlling DWG/DXF-to-publish conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertOptions {
    pub plant: String,
    pub min_tier: ConfidenceTier,
    pub thresholds: ConfidenceThresholds,
    pub snap_tol_mm: f64,
    pub pipe_layers: Vec<String>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            plant: "P01".to_string(),
            min_tier: ConfidenceTier::Medium,
            thresholds: ConfidenceThresholds::default(),
            snap_tol_mm: 1.8,
            pipe_layers: vec!["11".to_string(), "PIPE".to_string(), "0".to_string()],
        }
    }
}

/// Structured per-sheet conversion & coverage report written to `<stem>_Coverage.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvertSheetReport {
    pub drawing_name: String,
    pub doc_uid: String,
    pub plant: String,
    pub confidence_summary: ConfidenceSummary,
    pub exported_by_class: BTreeMap<String, usize>,
    pub piping_synthesis: PipingSynthesisStats,
    pub total_items_written: usize,
    pub total_relations_written: usize,
    pub skipped_items: Vec<UnmappedItem>,
}

/// Summary of converting a single file or a directory of DWG/DXF sheets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchConvertSummary {
    pub files_converted: usize,
    pub files_failed: usize,
    pub total_items_written: usize,
    pub total_relations_written: usize,
    pub sheets: Vec<ConvertSheetReport>,
    pub errors: Vec<String>,
}

fn tier_rank(t: ConfidenceTier) -> u8 {
    match t {
        ConfidenceTier::High => 2,
        ConfidenceTier::Medium => 1,
        ConfidenceTier::Low => 0,
    }
}

fn is_pipe_layer(layer: &str, patterns: &[String]) -> bool {
    let lu = layer.trim().to_ascii_uppercase();
    patterns.iter().any(|p| {
        let pu = p.trim().to_ascii_uppercase();
        lu == pu || (pu == "PIPE" && lu.starts_with("PIPE"))
    })
}

fn point_to_segment_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    let proj = (a.0 + t * vx, a.1 + t * vy);
    (p.0 - proj.0).hypot(p.1 - proj.1)
}

struct ChainedRun {
    points: Vec<(f64, f64)>,
}

/// Chain 2-degree connected pipe segments into logical pipe runs, cutting at
/// symbol ports and 3+-way junctions.
fn chain_pipe_segments(
    segments: &[PipeSegmentHit],
    pipe_layers: &[String],
    port_points: &[(f64, f64)],
    upm: f64,
    port_tol_mm: f64,
) -> Vec<ChainedRun> {
    let filtered: Vec<&PipeSegmentHit> = segments
        .iter()
        .filter(|s| is_pipe_layer(&s.layer, pipe_layers))
        .collect();
    if filtered.is_empty() {
        return Vec::new();
    }

    let node_tol = 0.4 * upm;
    let port_tol = port_tol_mm * upm;
    let mut node_coords: Vec<(f64, f64)> = Vec::new();
    let intern_node = |pt: (f64, f64), nodes: &mut Vec<(f64, f64)>| -> usize {
        for (idx, &existing) in nodes.iter().enumerate() {
            if (pt.0 - existing.0).hypot(pt.1 - existing.1) <= node_tol {
                return idx;
            }
        }
        let idx = nodes.len();
        nodes.push(pt);
        idx
    };

    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut adj: Vec<Vec<(usize, usize)>> = Vec::new();
    for seg in filtered {
        let u = intern_node(seg.start, &mut node_coords);
        let v = intern_node(seg.end, &mut node_coords);
        while adj.len() <= u.max(v) {
            adj.push(Vec::new());
        }
        if u == v {
            continue;
        }
        let ei = edges.len();
        edges.push((u, v));
        adj[u].push((v, ei));
        adj[v].push((u, ei));
    }

    let v_count = node_coords.len();
    let mut is_stop = vec![false; v_count];
    for (vi, &nc) in node_coords.iter().enumerate() {
        if adj.get(vi).map_or(0, Vec::len) != 2 {
            is_stop[vi] = true;
            continue;
        }
        if port_points
            .iter()
            .any(|&pp| (nc.0 - pp.0).hypot(nc.1 - pp.1) <= port_tol)
        {
            is_stop[vi] = true;
        }
    }

    let mut used_edge = vec![false; edges.len()];
    let mut out = Vec::new();

    // First walk from every stop vertex
    for start_v in 0..v_count {
        if !is_stop[start_v] {
            continue;
        }
        for &(next_v, first_ei) in &adj[start_v] {
            if used_edge[first_ei] {
                continue;
            }
            used_edge[first_ei] = true;
            let mut pts = vec![node_coords[start_v], node_coords[next_v]];
            let mut cur = next_v;
            while !is_stop[cur] {
                let Some(&(nxt, nei)) = adj[cur].iter().find(|&&(_, ei)| !used_edge[ei]) else {
                    break;
                };
                used_edge[nei] = true;
                pts.push(node_coords[nxt]);
                cur = nxt;
            }
            out.push(ChainedRun { points: pts });
        }
    }

    // Walk any remaining isolated 2-degree loops
    for start_ei in 0..edges.len() {
        if used_edge[start_ei] {
            continue;
        }
        used_edge[start_ei] = true;
        let (u, v) = edges[start_ei];
        let mut pts = vec![node_coords[u], node_coords[v]];
        let mut cur = v;
        while cur != u {
            let Some(&(nxt, nei)) = adj[cur].iter().find(|&&(_, ei)| !used_edge[ei]) else {
                break;
            };
            used_edge[nei] = true;
            pts.push(node_coords[nxt]);
            cur = nxt;
        }
        out.push(ChainedRun { points: pts });
    }

    out
}

fn nearest_label_for_run(
    run: &ChainedRun,
    labels: &[PipeLabelHit],
    max_dist: f64,
) -> Option<String> {
    let mut best: Option<(f64, &str)> = None;
    for lb in labels {
        let d = run
            .points
            .windows(2)
            .map(|w| point_to_segment_dist(lb.at, w[0], w[1]))
            .fold(f64::INFINITY, f64::min);
        if d <= max_dist && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, lb.raw_text.as_str()));
        }
    }
    best.map(|(_, s)| s.to_string())
}

/// Convert an in-memory `Document` into `(_Data.xml, _Meta.xml, _Coverage.json)` bytes.
pub fn convert_document_to_publish(
    doc: &Document,
    doc_name: &str,
    opts: &ConvertOptions,
) -> Result<(ExportBundle, ConvertSheetReport), String> {
    let map = default_legend_class_map();
    let rec = recognise_sppid_sheet(doc, map, opts.thresholds);
    let upm = rec.units_per_mm.max(1e-6);

    let mut alloc = UidAllocator::new(&opts.plant, doc_name);
    let doc_uid = alloc.alloc_primary_uid(None, &format!("drawing:{doc_name}"));
    let mut dwg = ExportDrawing::new(&opts.plant, &doc_uid, doc_name);

    let mut exported_by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut endpoints: Vec<SymbolEndpointRef> = Vec::new();
    let mut all_ports: Vec<(f64, f64)> = Vec::new();

    for (idx, sym) in rec.symbols.iter().enumerate() {
        if !sym.known || tier_rank(sym.tier) < tier_rank(opts.min_tier) {
            dwg.unmapped.push(UnmappedItem {
                source_id: format!("sym#{idx}@({:.2},{:.2})", sym.at.0, sym.at.1),
                raw_class: sym.source_block.clone(),
                tag: sym.tag.clone(),
                reason: if !sym.known {
                    "unmapped_block".to_string()
                } else {
                    format!("below_min_confidence_tier({:?})", sym.tier)
                },
            });
            continue;
        }

        let stable_key = format!(
            "sym:{}:{}:{idx}:{:.2},{:.2}",
            sym.sppid_class,
            sym.tag.as_deref().unwrap_or(""),
            sym.at.0,
            sym.at.1
        );
        let uid = alloc.alloc_primary_uid(None, &stable_key);
        let goid = alloc.alloc_graphic_oid(None);

        let mut attrs: BTreeMap<(String, String), String> = BTreeMap::new();
        if sym.sppid_class == "PIDBranchPoint" {
            if let Some(ref tf) = sym.to_from_text {
                attrs.insert(
                    ("IOffpageConnector".to_string(), "ToFromText".to_string()),
                    tf.clone(),
                );
            }
        }

        let mut item = ExportItem {
            uid: uid.clone(),
            class: sym.sppid_class.clone(),
            name: sym.tag.clone(),
            description: sym.description.clone(),
            variant_id: sym.variant_id.clone(),
            symbol_path: sym.symbol_path.clone(),
            graphic_oids: vec![goid],
            attributes: attrs,
        };
        populate_item_tag_attributes(&mut item);

        *exported_by_class
            .entry(sym.sppid_class.clone())
            .or_default() += 1;
        all_ports.extend(sym.ports.iter().copied());
        endpoints.push(SymbolEndpointRef {
            symbol_uid: uid,
            sppid_class: sym.sppid_class.clone(),
            bbox: sym.bbox,
            ports: sym.ports.clone(),
        });
        dwg.items.push(item);
    }

    let chained = chain_pipe_segments(
        &rec.pipe_segments,
        &opts.pipe_layers,
        &all_ports,
        upm,
        opts.snap_tol_mm,
    );
    let label_dist = 18.0 * upm;
    let run_inputs: Vec<PipeRunInput> = chained
        .iter()
        .filter_map(|cr| {
            let start = *cr.points.first()?;
            let end = *cr.points.last()?;
            let line_label = nearest_label_for_run(cr, &rec.pipe_labels, label_dist);
            Some(PipeRunInput {
                start,
                end,
                line_label,
                graphic_oid: Some(alloc.alloc_graphic_oid(None)),
            })
        })
        .collect();

    let mut piping_stats = synthesize_piping_network(
        &mut dwg,
        &mut alloc,
        &endpoints,
        &run_inputs,
        opts.snap_tol_mm * upm,
    )?;

    // Ensure any standalone line label on the sheet whose pipeline wasn't touched by a stroke still creates a PIDPipeline.
    let mut existing_pipelines: BTreeSet<String> = dwg
        .items
        .iter()
        .filter(|i| i.class == "PIDPipeline")
        .filter_map(|i| i.name.clone())
        .collect();
    for lb in &rec.pipe_labels {
        if let Some(parsed) = parse_sppid_line_number(&lb.raw_text) {
            let pname = parsed.pipeline_name();
            if existing_pipelines.insert(pname.clone()) {
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

    let mut bundle = export_drawing_bundle(&dwg)?;
    let report = ConvertSheetReport {
        drawing_name: doc_name.to_string(),
        doc_uid,
        plant: opts.plant.clone(),
        confidence_summary: rec.confidence_summary,
        exported_by_class,
        piping_synthesis: piping_stats,
        total_items_written: dwg.items.len(),
        total_relations_written: dwg.relations.len(),
        skipped_items: dwg.unmapped,
    };
    bundle.coverage_json = serde_json::to_string_pretty(&report)
        .map_err(|e| format!("serialize coverage report: {e}"))?;

    Ok((bundle, report))
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_input_drawings(input: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if input.is_file() {
        out.push(input.to_path_buf());
        return Ok(());
    }
    let rd = std::fs::read_dir(input).map_err(|e| format!("read dir {}: {e}", input.display()))?;
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect_input_drawings(&p, out)?;
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dxf") || ext.eq_ignore_ascii_case("dwg"))
            && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
        {
            out.push(p);
        }
    }
    Ok(())
}

/// Convert a single `.dxf`/`.dwg` file or all non-empty `.dxf`/`.dwg` files in a
/// directory, writing `<stem>_Data.xml`, `<stem>_Meta.xml`, and `<stem>_Coverage.json`
/// into `out_dir`.
#[cfg(not(target_arch = "wasm32"))]
pub fn convert_path_to_dir(
    input: &Path,
    out_dir: &Path,
    opts: &ConvertOptions,
) -> Result<BatchConvertSummary, String> {
    if !input.exists() {
        return Err(format!("input path does not exist: {}", input.display()));
    }
    let mut files = Vec::new();
    collect_input_drawings(input, &mut files)?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("create out dir {}: {e}", out_dir.display()))?;

    let mut summary = BatchConvertSummary::default();
    for f in files {
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SHEET")
            .to_string();
        match crate::io::load_file(&f) {
            Ok(doc) => match convert_document_to_publish(&doc, &stem, opts) {
                Ok((bundle, rep)) => {
                    let data_p = out_dir.join(format!("{stem}_Data.xml"));
                    let meta_p = out_dir.join(format!("{stem}_Meta.xml"));
                    let cov_p = out_dir.join(format!("{stem}_Coverage.json"));
                    if let Err(e) = std::fs::write(&data_p, &bundle.data_xml)
                        .and_then(|_| std::fs::write(&meta_p, &bundle.meta_xml))
                        .and_then(|_| std::fs::write(&cov_p, format!("{}\n", bundle.coverage_json)))
                    {
                        summary.files_failed += 1;
                        summary.errors.push(format!("{stem}: write failed: {e}"));
                    } else {
                        summary.files_converted += 1;
                        summary.total_items_written += rep.total_items_written;
                        summary.total_relations_written += rep.total_relations_written;
                        summary.sheets.push(rep);
                    }
                }
                Err(err) => {
                    summary.files_failed += 1;
                    summary
                        .errors
                        .push(format!("{stem}: convert failed: {err}"));
                }
            },
            Err(err) => {
                summary.files_failed += 1;
                summary.errors.push(format!("{stem}: load failed: {err}"));
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_publish_check::{
        check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
    };
    use document::entities::{Block, BlockEnd, Insert, Line, Point, Text};
    use document::tables::BlockRecord;
    use document::types::Vector3;
    use document::{EntityType, Handle};

    fn define_block(doc: &mut Document, name: &str, w: f64, h: f64) {
        if doc.block_records.get(name).is_some() {
            return;
        }
        let next = doc.next_handle();
        let br_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let end_handle = Handle::new(next + 2);
        let mut record = BlockRecord::new(name);
        record.handle = br_handle;
        record.block_entity_handle = block_handle;
        record.block_end_handle = end_handle;
        doc.block_records.add(record).unwrap();
        let mut block = Block::new(name, Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = br_handle;
        doc.add_entity(EntityType::Block(block)).unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = br_handle;
        doc.add_entity(EntityType::BlockEnd(end)).unwrap();

        let mut ln = Line::from_points(
            Vector3::new(-w / 2.0, -h / 2.0, 0.0),
            Vector3::new(w / 2.0, h / 2.0, 0.0),
        );
        ln.common.owner_handle = br_handle;
        doc.add_entity(EntityType::Line(ln)).unwrap();

        for px in [-w / 2.0, w / 2.0] {
            let mut pt = Point::new();
            pt.location = Vector3::new(px, 0.0, 0.0);
            pt.common.owner_handle = br_handle;
            doc.add_entity(EntityType::Point(pt)).unwrap();
        }
    }

    fn build_synthetic_sheet() -> Document {
        let mut doc = Document::new();
        define_block(&mut doc, "BALL VALVE TYPE 2", 6.0, 4.0);
        define_block(&mut doc, "PG-PRESSURE GAUGE", 8.0, 8.0);
        define_block(&mut doc, "_OFF-UNIT SQ", 10.0, 6.0);
        define_block(&mut doc, "UNMAPPED_CUSTOM_SYM", 6.0, 6.0);

        let mut v_ins = Insert::new("BALL VALVE TYPE 2", Vector3::new(50.0, 100.0, 0.0));
        v_ins.common.layer = "11".to_string();
        doc.add_entity(EntityType::Insert(v_ins)).unwrap();

        let mut v_txt =
            Text::with_value("BV 0201501B", Vector3::new(45.0, 104.0, 0.0)).with_height(2.5);
        v_txt.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(v_txt)).unwrap();

        let mut pg_ins = Insert::new("PG-PRESSURE GAUGE", Vector3::new(50.0, 130.0, 0.0));
        pg_ins.common.layer = "12".to_string();
        doc.add_entity(EntityType::Insert(pg_ins)).unwrap();
        let mut pg_t1 = Text::with_value("PG", Vector3::new(48.0, 131.4, 0.0)).with_height(2.5);
        pg_t1.common.layer = "12".to_string();
        doc.add_entity(EntityType::Text(pg_t1)).unwrap();
        let mut pg_t2 =
            Text::with_value("0201501A", Vector3::new(44.0, 126.1, 0.0)).with_height(2.5);
        pg_t2.common.layer = "12".to_string();
        doc.add_entity(EntityType::Text(pg_t2)).unwrap();

        let mut opc_ins = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 100.0, 0.0));
        opc_ins.common.layer = "11".to_string();
        doc.add_entity(EntityType::Insert(opc_ins)).unwrap();
        let mut opc_t =
            Text::with_value("DWG-0001-05", Vector3::new(88.0, 99.0, 0.0)).with_height(2.5);
        opc_t.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(opc_t)).unwrap();

        let mut u_ins = Insert::new("UNMAPPED_CUSTOM_SYM", Vector3::new(200.0, 200.0, 0.0));
        u_ins.common.layer = "11".to_string();
        doc.add_entity(EntityType::Insert(u_ins)).unwrap();

        for (a, b) in [
            ((10.0, 80.0), (10.0, 100.0)),
            ((10.0, 100.0), (47.0, 100.0)),
            ((53.0, 100.0), (95.0, 100.0)),
        ] {
            let mut pl =
                Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
            pl.common.layer = "11".to_string();
            doc.add_entity(EntityType::Line(pl)).unwrap();
        }
        let mut ln_txt = Text::with_value("25-CI-0201505-B9-EH40", Vector3::new(20.0, 103.0, 0.0))
            .with_height(2.5);
        ln_txt.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(ln_txt)).unwrap();

        doc
    }

    #[test]
    fn converts_synthetic_sheet_to_strict_valid_xml_and_conserved_coverage_report() {
        let doc = build_synthetic_sheet();
        let opts = ConvertOptions::default();
        let (bundle, report) =
            convert_document_to_publish(&doc, "SYNTH-CONV-01", &opts).expect("convert sheet");

        let spec = default_publish_spec();
        let chk_opts = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..Default::default()
        };
        let vd = check_publish_xml_bytes(
            "SYNTH-CONV-01_Data.xml",
            PublishXmlKind::Data,
            &bundle.data_xml,
            spec,
            &chk_opts,
        );
        let vm = check_publish_xml_bytes(
            "SYNTH-CONV-01_Meta.xml",
            PublishXmlKind::Meta,
            &bundle.meta_xml,
            spec,
            &chk_opts,
        );
        assert!(vd.is_empty(), "Data violations: {vd:?}");
        assert!(vm.is_empty(), "Meta violations: {vm:?}");

        let exported_syms: usize = report.exported_by_class.values().sum();
        assert_eq!(exported_syms, 3);
        assert_eq!(report.skipped_items.len(), 1);
        assert_eq!(
            report.confidence_summary.total_symbols(),
            exported_syms + report.skipped_items.len()
        );
        assert!(report.piping_synthesis.pipelines >= 1);
        assert!(report.piping_synthesis.piping_connectors >= 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn converts_directory_of_dxf_files_to_publish_triples() {
        let doc = build_synthetic_sheet();
        let tmp = std::env::temp_dir().join(format!("ocs_pid_conv_test_{}", std::process::id()));
        let in_dir = tmp.join("in");
        let out_dir = tmp.join("out");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&in_dir).unwrap();

        let dxf_path = in_dir.join("UNIT_SHEET_01.dxf");
        crate::io::save(&doc, &dxf_path).unwrap();
        std::fs::write(in_dir.join("EMPTY_0BYTE.dxf"), b"").unwrap();

        let summary =
            convert_path_to_dir(&in_dir, &out_dir, &ConvertOptions::default()).expect("batch");
        assert_eq!(summary.files_converted, 1);
        assert_eq!(summary.files_failed, 0);
        assert!(out_dir.join("UNIT_SHEET_01_Data.xml").is_file());
        assert!(out_dir.join("UNIT_SHEET_01_Meta.xml").is_file());
        assert!(out_dir.join("UNIT_SHEET_01_Coverage.json").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

//! Single-sheet and batch `DWG/DXF -> SmartPlant publish XML + .pid + Coverage report`
//! conversion pipeline (`L4-6` / `L4-7` / `L5-6` / `L5-8` / `L6-1` / `#127` / `#130`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use document::Document;
use serde::{Deserialize, Serialize};

use super::class_map::default_legend_class_map;
use super::recognize::{
    recognise_sppid_sheet, ConfidenceSummary, ConfidenceThresholds, ConfidenceTier, PipeLabelHit,
    PipeSegmentHit, SheetRecognition, TitleBlockInfo,
};
use crate::io::pid_publish::{
    export_drawing_bundle, is_valid_hex32, parse_sppid_line_number, pipeline_uid,
    populate_item_tag_attributes, synthesize_piping_network, ExportBundle, ExportDrawing,
    ExportItem, PipeRunInput, PipingSynthesisStats, SymbolEndpointRef, UidAllocator, UnmappedItem,
};

/// Options controlling DWG/DXF-to-publish (and optional `.pid` dual-output) conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertOptions {
    pub plant: String,
    pub min_tier: ConfidenceTier,
    pub thresholds: ConfidenceThresholds,
    pub snap_tol_mm: f64,
    pub pipe_layers: Vec<String>,
    #[serde(default)]
    pub seed_pid_path: Option<String>,
    #[serde(skip)]
    pub seed_pid_bytes: Option<Vec<u8>>,
    #[serde(default)]
    pub sym_library_dir: Option<String>,
    #[serde(default)]
    pub verify_plot_pdf: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            plant: "P01".to_string(),
            min_tier: ConfidenceTier::Medium,
            thresholds: ConfidenceThresholds::default(),
            snap_tol_mm: 1.8,
            pipe_layers: vec!["11".to_string(), "PIPE".to_string(), "0".to_string()],
            seed_pid_path: None,
            seed_pid_bytes: None,
            sym_library_dir: None,
            verify_plot_pdf: false,
        }
    }
}

impl ConvertOptions {
    pub fn wants_dual_pid(&self) -> bool {
        self.seed_pid_bytes.is_some()
            || self
                .seed_pid_path
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
    }
}

/// Cross-file (`_Data.xml <-> .pid`), `a`-level (re-read + Q9 index check), and
/// `b`-level (CAD load + `pid-plot` PDF render) verification report (`#130`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DualVerifyReport {
    pub xml_rep_count: usize,
    pub matched_graphic_oids: usize,
    pub matched_model_uids: usize,
    pub matched_representation_uids: usize,
    pub missing_graphic_oids_in_pid: Vec<u32>,
    pub missing_model_uids_in_pid: Vec<String>,
    pub mismatched_links: Vec<String>,
    pub spid_matches_doc_uid: bool,
    pub a_level_index_consistent: bool,
    pub a_level_normalized_entities: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_level_cad_entities: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_level_pdf_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_level_pdf_primitives: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_level_pdf_on_page: Option<u64>,
    pub sppid_native_c1_status: String,
}

impl DualVerifyReport {
    pub fn is_cross_file_and_a_valid(&self) -> bool {
        self.xml_rep_count > 0
            && self.matched_graphic_oids == self.xml_rep_count
            && self.matched_model_uids == self.xml_rep_count
            && self.matched_representation_uids == self.xml_rep_count
            && self.missing_graphic_oids_in_pid.is_empty()
            && self.missing_model_uids_in_pid.is_empty()
            && self.mismatched_links.is_empty()
            && self.spid_matches_doc_uid
            && self.a_level_index_consistent
            && self.a_level_normalized_entities > 0
    }

    pub fn is_b_plot_valid(&self) -> bool {
        self.b_level_cad_entities.is_some_and(|n| n > 0)
            && self.b_level_pdf_bytes.is_some_and(|n| n > 0)
            && self.b_level_pdf_primitives.is_some_and(|n| n > 0)
            && self.b_level_pdf_on_page.is_some_and(|n| n > 0)
    }
}

/// Conserved endpoint attribution across all exported pipe runs (`L3-3` / `#133`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadEndAttribution {
    pub snapped_to_port: usize,
    pub snapped_to_offpage: usize,
    pub snapped_to_tee: usize,
    pub touching_equipment_shell: usize,
    pub open_free_end: usize,
}

impl DeadEndAttribution {
    pub fn total_endpoints(&self) -> usize {
        self.snapped_to_port
            + self.snapped_to_offpage
            + self.snapped_to_tee
            + self.touching_equipment_shell
            + self.open_free_end
    }
}

/// Per-sheet pipe network chaining, family breakdown, and dead-end census (`L3-3` / `#133`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeNetworkCensus {
    pub runs_by_layer: BTreeMap<String, usize>,
    pub labeled_runs: usize,
    pub unlabeled_runs: usize,
    pub collinear_bridges: usize,
    pub filtered_micro_stubs: usize,
    pub dead_end_attribution: DeadEndAttribution,
}

/// One off-page connector summary on a converted sheet (`L3-4` / `#133`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffpageConnectorEntry {
    pub uid: String,
    pub target_dwg_number: Option<String>,
    pub description: Option<String>,
    pub line_label: Option<String>,
}

/// A matched reciprocal cross-sheet off-page connector pair (`L3-4` / `#133`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossSheetOpcPair {
    pub sheet_a_index: usize,
    pub sheet_a_dwg: String,
    pub uid_a: String,
    pub sheet_b_index: usize,
    pub sheet_b_dwg: String,
    pub uid_b: String,
    pub line_label: Option<String>,
}

/// Batch-level cross-sheet off-page connector pairing report (`L3-4` / `#133`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossSheetOpcReport {
    pub total_offpage_connectors: usize,
    pub with_target_dwg: usize,
    pub reciprocal_paired_connectors: usize,
    pub one_way_in_batch: usize,
    pub external_target_dwg: usize,
    pub without_target_dwg: usize,
    pub pairs: Vec<CrossSheetOpcPair>,
}

fn norm_dwg_code(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('-')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

fn expand_target_dwg_codes(raw: &str) -> Vec<String> {
    let clean = norm_dwg_code(raw);
    if clean.is_empty() {
        return Vec::new();
    }
    if !clean.contains('/') {
        return vec![clean];
    }
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let first = parts[0].to_string();
    let mut out = vec![first.clone()];
    if let Some((prefix, _)) = first.rsplit_once('-') {
        for &suffix in &parts[1..] {
            if suffix.starts_with("DWG-") || suffix.contains('-') {
                out.push(suffix.to_string());
            } else {
                out.push(format!("{prefix}-{suffix}"));
            }
        }
    }
    out
}

/// Pair off-page connectors across a batch of converted sheet reports (`L3-4` / `#133`).
pub fn pair_cross_sheet_connectors(sheets: &[ConvertSheetReport]) -> CrossSheetOpcReport {
    let mut dwg_to_sheets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut sheet_dwg_code: Vec<String> = Vec::with_capacity(sheets.len());

    for (si, sh) in sheets.iter().enumerate() {
        let code = sh
            .title_block
            .drawing_number
            .as_deref()
            .map(norm_dwg_code)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| norm_dwg_code(&sh.drawing_name));
        if !code.is_empty() {
            dwg_to_sheets.entry(code.clone()).or_default().push(si);
        }
        sheet_dwg_code.push(code);
    }

    #[derive(Clone)]
    struct FlatOpc {
        sheet_idx: usize,
        uid: String,
        targets: Vec<String>,
        line_label: Option<String>,
    }

    let mut flat: Vec<FlatOpc> = Vec::new();
    let mut without_target_dwg = 0usize;
    for (si, sh) in sheets.iter().enumerate() {
        for opc in &sh.offpage_connectors {
            let targets = opc
                .target_dwg_number
                .as_deref()
                .map(expand_target_dwg_codes)
                .unwrap_or_default();
            if targets.is_empty() {
                without_target_dwg += 1;
            }
            flat.push(FlatOpc {
                sheet_idx: si,
                uid: opc.uid.clone(),
                targets,
                line_label: opc.line_label.clone(),
            });
        }
    }

    let total_offpage_connectors = flat.len();
    let with_target_dwg = total_offpage_connectors.saturating_sub(without_target_dwg);

    // Build candidate reciprocal pairs: i < j where i targets j's sheet AND j targets i's sheet.
    let mut cand_pairs: Vec<(u8, usize, usize)> = Vec::new();
    for i in 0..flat.len() {
        if flat[i].targets.is_empty() {
            continue;
        }
        let code_i = &sheet_dwg_code[flat[i].sheet_idx];
        if code_i.is_empty() {
            continue;
        }
        for j in (i + 1)..flat.len() {
            if flat[i].sheet_idx == flat[j].sheet_idx || flat[j].targets.is_empty() {
                continue;
            }
            let code_j = &sheet_dwg_code[flat[j].sheet_idx];
            if code_j.is_empty() {
                continue;
            }
            let i_to_j = flat[i].targets.iter().any(|t| t == code_j);
            let j_to_i = flat[j].targets.iter().any(|t| t == code_i);
            if i_to_j && j_to_i {
                let rank = match (&flat[i].line_label, &flat[j].line_label) {
                    (Some(la), Some(lb)) if !la.is_empty() && la == lb => 0,
                    (None, None) => 1,
                    _ => 2,
                };
                cand_pairs.push((rank, i, j));
            }
        }
    }
    cand_pairs.sort_by_key(|&(rank, i, j)| (rank, i, j));

    let mut paired_mask = vec![false; flat.len()];
    let mut pairs = Vec::new();
    for (_, i, j) in cand_pairs {
        if paired_mask[i] || paired_mask[j] {
            continue;
        }
        paired_mask[i] = true;
        paired_mask[j] = true;
        pairs.push(CrossSheetOpcPair {
            sheet_a_index: flat[i].sheet_idx,
            sheet_a_dwg: sheet_dwg_code[flat[i].sheet_idx].clone(),
            uid_a: flat[i].uid.clone(),
            sheet_b_index: flat[j].sheet_idx,
            sheet_b_dwg: sheet_dwg_code[flat[j].sheet_idx].clone(),
            uid_b: flat[j].uid.clone(),
            line_label: flat[i]
                .line_label
                .clone()
                .or_else(|| flat[j].line_label.clone()),
        });
    }

    let mut one_way_in_batch = 0usize;
    let mut external_target_dwg = 0usize;
    for (idx, opc) in flat.iter().enumerate() {
        if paired_mask[idx] || opc.targets.is_empty() {
            continue;
        }
        let in_batch = opc.targets.iter().any(|t| {
            dwg_to_sheets
                .get(t)
                .is_some_and(|owners| owners.iter().any(|&si| si != opc.sheet_idx))
        });
        if in_batch {
            one_way_in_batch += 1;
        } else {
            external_target_dwg += 1;
        }
    }

    CrossSheetOpcReport {
        total_offpage_connectors,
        with_target_dwg,
        reciprocal_paired_connectors: pairs.len() * 2,
        one_way_in_batch,
        external_target_dwg,
        without_target_dwg,
        pairs,
    }
}

/// Structured per-sheet conversion & coverage report written to `<stem>_Coverage.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvertSheetReport {
    pub drawing_name: String,
    pub doc_uid: String,
    pub plant: String,
    #[serde(default)]
    pub title_block: TitleBlockInfo,
    pub confidence_summary: ConfidenceSummary,
    pub exported_by_class: BTreeMap<String, usize>,
    pub piping_synthesis: PipingSynthesisStats,
    #[serde(default)]
    pub pipe_network_census: PipeNetworkCensus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offpage_connectors: Vec<OffpageConnectorEntry>,
    pub total_items_written: usize,
    pub total_relations_written: usize,
    pub skipped_items: Vec<UnmappedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_verification: Option<DualVerifyReport>,
}

/// Summary of converting a single file or a directory of DWG/DXF sheets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchConvertSummary {
    pub files_converted: usize,
    pub files_failed: usize,
    #[serde(default)]
    pub pid_files_written: usize,
    #[serde(default)]
    pub pdf_files_plotted: usize,
    pub total_items_written: usize,
    pub total_relations_written: usize,
    #[serde(default)]
    pub cross_sheet_opc_pairing: CrossSheetOpcReport,
    pub sheets: Vec<ConvertSheetReport>,
    pub errors: Vec<String>,
}

/// Dual-output bundle (`_Data.xml`, `_Meta.xml`, optional `.pid` bytes, and `_Coverage.json`).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct DualConvertBundle {
    pub publish: ExportBundle,
    pub pid_bytes: Option<Vec<u8>>,
    pub report: ConvertSheetReport,
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
    layer: String,
}

fn point_in_bbox(p: (f64, f64), bb: (f64, f64, f64, f64), margin: f64) -> bool {
    p.0 >= bb.0 - margin && p.0 <= bb.2 + margin && p.1 >= bb.1 - margin && p.1 <= bb.3 + margin
}

fn polyline_length(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// Chain 2-degree connected pipe segments into logical pipe runs, bridging
/// collinear line-jump gaps, cutting at symbol ports and 3+-way junctions, and
/// filtering isolated micro-stubs (`L3-3` / `#133`).
fn chain_pipe_segments(
    segments: &[PipeSegmentHit],
    pipe_layers: &[String],
    port_points: &[(f64, f64)],
    symbol_bboxes: &[(f64, f64, f64, f64)],
    upm: f64,
    port_tol_mm: f64,
) -> (Vec<ChainedRun>, usize, usize) {
    let filtered: Vec<&PipeSegmentHit> = segments
        .iter()
        .filter(|s| is_pipe_layer(&s.layer, pipe_layers))
        .collect();
    if filtered.is_empty() {
        return (Vec::new(), 0, 0);
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

    let mut edges: Vec<(usize, usize, String)> = Vec::new();
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
        edges.push((u, v, seg.layer.trim().to_string()));
        adj[u].push((v, ei));
        adj[v].push((u, ei));
    }

    let near_any_port = |pt: (f64, f64)| -> bool {
        port_points
            .iter()
            .any(|&pp| (pt.0 - pp.0).hypot(pt.1 - pp.1) <= port_tol)
    };

    // Collinear micro-gap bridging across line jumps / inline label breaks (`0.4..=6.5 mm`).
    let deg1_nodes: Vec<usize> = (0..node_coords.len())
        .filter(|&vi| adj.get(vi).map_or(0, Vec::len) == 1 && !near_any_port(node_coords[vi]))
        .collect();
    let mut bridge_cands: Vec<(i64, usize, usize, String)> = Vec::new();
    let min_seg_for_bridge = 2.0 * upm;
    let max_gap = 6.5 * upm;
    let max_lat = 0.45 * upm;

    for idx_a in 0..deg1_nodes.len() {
        let u = deg1_nodes[idx_a];
        let (u_nb, u_ei) = adj[u][0];
        let pu = node_coords[u];
        let p_unb = node_coords[u_nb];
        let len_u = (pu.0 - p_unb.0).hypot(pu.1 - p_unb.1);
        if len_u < min_seg_for_bridge {
            continue;
        }
        // Outward ray direction from `u` away from `u_nb`
        let du = ((pu.0 - p_unb.0) / len_u, (pu.1 - p_unb.1) / len_u);
        let lay_u = &edges[u_ei].2;

        for &v in &deg1_nodes[(idx_a + 1)..] {
            let (v_nb, v_ei) = adj[v][0];
            if u_nb == v || v_nb == u || &edges[v_ei].2 != lay_u {
                continue;
            }
            let pv = node_coords[v];
            let gap_vec = (pv.0 - pu.0, pv.1 - pu.1);
            let gap_len = gap_vec.0.hypot(gap_vec.1);
            if !(node_tol..=max_gap).contains(&gap_len) {
                continue;
            }
            let p_vnb = node_coords[v_nb];
            let len_v = (pv.0 - p_vnb.0).hypot(pv.1 - p_vnb.1);
            if len_v < min_seg_for_bridge {
                continue;
            }
            // Outward ray from `v` away from `v_nb` must oppose `du` (`du · dv <= -0.990`)
            let dv = ((pv.0 - p_vnb.0) / len_v, (pv.1 - p_vnb.1) / len_v);
            let anti_dot = -(du.0 * dv.0 + du.1 * dv.1);
            let forward_proj = (gap_vec.0 * du.0 + gap_vec.1 * du.1) / gap_len;
            let lat_off = (gap_vec.0 * (-du.1) + gap_vec.1 * du.0).abs();
            if anti_dot < 0.990 || forward_proj < 0.990 || lat_off > max_lat {
                continue;
            }
            let mid = ((pu.0 + pv.0) * 0.5, (pu.1 + pv.1) * 0.5);
            if symbol_bboxes
                .iter()
                .any(|&bb| point_in_bbox(mid, bb, 0.2 * upm))
            {
                continue;
            }
            bridge_cands.push(((gap_len * 1000.0) as i64, u, v, lay_u.clone()));
        }
    }
    bridge_cands.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut bridged_node = vec![false; node_coords.len()];
    let mut collinear_bridges = 0usize;
    for (_, u, v, lay) in bridge_cands {
        if bridged_node[u] || bridged_node[v] {
            continue;
        }
        bridged_node[u] = true;
        bridged_node[v] = true;
        let ei = edges.len();
        edges.push((u, v, lay));
        adj[u].push((v, ei));
        adj[v].push((u, ei));
        collinear_bridges += 1;
    }

    let v_count = node_coords.len();
    let mut is_stop = vec![false; v_count];
    for (vi, &nc) in node_coords.iter().enumerate() {
        if adj.get(vi).map_or(0, Vec::len) != 2 || near_any_port(nc) {
            is_stop[vi] = true;
        }
    }

    let mut used_edge = vec![false; edges.len()];
    let mut raw_runs = Vec::new();

    for start_v in 0..v_count {
        if !is_stop[start_v] {
            continue;
        }
        for &(next_v, first_ei) in &adj[start_v] {
            if used_edge[first_ei] {
                continue;
            }
            used_edge[first_ei] = true;
            let lay = edges[first_ei].2.clone();
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
            raw_runs.push((
                start_v,
                cur,
                ChainedRun {
                    points: pts,
                    layer: lay,
                },
            ));
        }
    }

    for start_ei in 0..edges.len() {
        if used_edge[start_ei] {
            continue;
        }
        used_edge[start_ei] = true;
        let (u, v, ref lay) = edges[start_ei];
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
        raw_runs.push((
            u,
            cur,
            ChainedRun {
                points: pts,
                layer: lay.clone(),
            },
        ));
    }

    let min_stub_len = 2.0 * upm;
    let max_arrow_span = 2.4 * upm;
    let mut out = Vec::new();
    let mut filtered_micro_stubs = 0usize;
    for (u_idx, v_idx, cr) in raw_runs {
        let len = polyline_length(&cr.points);
        let span = if cr.points.is_empty() {
            0.0
        } else {
            let mut x0 = f64::INFINITY;
            let mut y0 = f64::INFINITY;
            let mut x1 = f64::NEG_INFINITY;
            let mut y1 = f64::NEG_INFINITY;
            for &(px, py) in &cr.points {
                x0 = x0.min(px);
                y0 = y0.min(py);
                x1 = x1.max(px);
                y1 = y1.max(py);
            }
            (x1 - x0).hypot(y1 - y0)
        };
        let deg_u = adj.get(u_idx).map_or(0, Vec::len);
        let deg_v = adj.get(v_idx).map_or(0, Vec::len);
        let anchored = deg_u >= 3
            || deg_v >= 3
            || cr.points.first().is_some_and(|&p| near_any_port(p))
            || cr.points.last().is_some_and(|&p| near_any_port(p));
        if (len < min_stub_len || span < max_arrow_span) && !anchored {
            filtered_micro_stubs += cr.points.len().saturating_sub(1).max(1);
            continue;
        }
        out.push(cr);
    }

    (out, collinear_bridges, filtered_micro_stubs)
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

struct PlacedSymbolRef {
    sym_idx: usize,
    graphic_oid: u32,
}

struct PlacedRunRef {
    points: Vec<(f64, f64)>,
    graphic_oid: u32,
}

struct CoreConversionState {
    rec: SheetRecognition,
    dwg: ExportDrawing,
    report: ConvertSheetReport,
    placed_symbols: Vec<PlacedSymbolRef>,
    placed_runs: Vec<PlacedRunRef>,
}

fn build_core_conversion(
    doc: &Document,
    doc_name: &str,
    opts: &ConvertOptions,
    seed_reserved_oids: Option<(u32, &BTreeSet<u32>)>,
) -> Result<CoreConversionState, String> {
    let map = default_legend_class_map();
    let rec = recognise_sppid_sheet(doc, map, opts.thresholds);
    let upm = rec.units_per_mm.max(1e-6);

    let effective_doc_name: String =
        if doc_name.trim().is_empty() || doc_name.trim().eq_ignore_ascii_case("untitled") {
            rec.title_block
                .drawing_title
                .clone()
                .or_else(|| rec.title_block.drawing_number.clone())
                .unwrap_or_else(|| doc_name.to_string())
        } else {
            doc_name.to_string()
        };

    let mut alloc = UidAllocator::new(&opts.plant, &effective_doc_name);
    if let Some((min_next, reserved)) = seed_reserved_oids {
        alloc.reserve_graphic_oids(min_next, reserved);
    }

    let doc_uid = alloc.alloc_primary_uid(None, &format!("drawing:{effective_doc_name}"));
    let mut dwg = ExportDrawing::new(&opts.plant, &doc_uid, &effective_doc_name);

    let mut exported_by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut endpoints: Vec<SymbolEndpointRef> = Vec::new();
    let mut all_ports: Vec<(f64, f64)> = Vec::new();
    let mut placed_symbols: Vec<PlacedSymbolRef> = Vec::new();

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
        if sym.sppid_class == "PIDNozzle" {
            if let Some(ref nd) = sym.nominal_diameter {
                attrs.insert(
                    (
                        "IPipeCrossSectionItem".to_string(),
                        "NominalDiameter".to_string(),
                    ),
                    nd.clone(),
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
        placed_symbols.push(PlacedSymbolRef {
            sym_idx: idx,
            graphic_oid: goid,
        });
        dwg.items.push(item);
    }

    let sym_bboxes: Vec<(f64, f64, f64, f64)> = endpoints
        .iter()
        .filter(|ep| {
            !matches!(
                ep.sppid_class.as_str(),
                "PIDProcessVessel"
                    | "PIDMechanicalEquipment"
                    | "PIDProcessEquipment"
                    | "PIDHeatTransferEquipment"
            )
        })
        .map(|ep| ep.bbox)
        .collect();
    let (chained, collinear_bridges, filtered_micro_stubs) = chain_pipe_segments(
        &rec.pipe_segments,
        &opts.pipe_layers,
        &all_ports,
        &sym_bboxes,
        upm,
        opts.snap_tol_mm,
    );
    let label_dist = 18.0 * upm;
    let mut run_labels: Vec<Option<String>> = chained
        .iter()
        .map(|cr| nearest_label_for_run(cr, &rec.pipe_labels, label_dist))
        .collect();

    // Propagate unambiguous line label across shared junctions on the same layer (`L3-3`).
    let junc_tol = opts.snap_tol_mm * upm;
    for _pass in 0..2 {
        let mut next_labels = run_labels.clone();
        for (ri, cr) in chained.iter().enumerate() {
            if run_labels[ri].is_some() {
                continue;
            }
            let Some(&s_pt) = cr.points.first() else {
                continue;
            };
            let Some(&e_pt) = cr.points.last() else {
                continue;
            };
            let mut neighbor_set: BTreeSet<&str> = BTreeSet::new();
            for (oj, ocr) in chained.iter().enumerate() {
                if oj == ri || ocr.layer != cr.layer {
                    continue;
                }
                let Some(ref olbl) = run_labels[oj] else {
                    continue;
                };
                let (Some(&os), Some(&oe)) = (ocr.points.first(), ocr.points.last()) else {
                    continue;
                };
                let touches = (s_pt.0 - os.0).hypot(s_pt.1 - os.1) <= junc_tol
                    || (s_pt.0 - oe.0).hypot(s_pt.1 - oe.1) <= junc_tol
                    || (e_pt.0 - os.0).hypot(e_pt.1 - os.1) <= junc_tol
                    || (e_pt.0 - oe.0).hypot(e_pt.1 - oe.1) <= junc_tol;
                if touches {
                    neighbor_set.insert(olbl.as_str());
                }
            }
            if neighbor_set.len() == 1 {
                if let Some(&only) = neighbor_set.iter().next() {
                    next_labels[ri] = Some(only.to_string());
                }
            }
        }
        run_labels = next_labels;
    }

    let mut runs_by_layer: BTreeMap<String, usize> = BTreeMap::new();
    let mut labeled_runs = 0usize;
    let mut unlabeled_runs = 0usize;
    let mut run_inputs: Vec<PipeRunInput> = Vec::new();
    let mut placed_runs: Vec<PlacedRunRef> = Vec::new();
    for (cr, line_label) in chained.iter().zip(run_labels) {
        let Some(&start) = cr.points.first() else {
            continue;
        };
        let Some(&end) = cr.points.last() else {
            continue;
        };
        *runs_by_layer.entry(cr.layer.clone()).or_default() += 1;
        if line_label.is_some() {
            labeled_runs += 1;
        } else {
            unlabeled_runs += 1;
        }
        let rgoid = alloc.alloc_graphic_oid(None);
        run_inputs.push(PipeRunInput {
            start,
            end,
            line_label,
            graphic_oid: Some(rgoid),
        });
        placed_runs.push(PlacedRunRef {
            points: cr.points.clone(),
            graphic_oid: rgoid,
        });
    }

    let mut piping_stats = synthesize_piping_network(
        &mut dwg,
        &mut alloc,
        &endpoints,
        &run_inputs,
        opts.snap_tol_mm * upm,
    )?;

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

    // Compute conserved dead-end attribution (`2 * run_inputs.len()`) and off-page connector entries (`L3-3 / L3-4`).
    let snap_tol = opts.snap_tol_mm * upm;
    let mut dead_end_attribution = DeadEndAttribution::default();
    let mut unsnapped_pts: Vec<(usize, bool, (f64, f64))> = Vec::new();
    let mut opc_line_label: BTreeMap<String, String> = BTreeMap::new();

    let classify_snap = |pt: (f64, f64)| -> Option<(bool, &str)> {
        let mut best_opc: Option<(f64, &str)> = None;
        let mut best_port: Option<(f64, &str)> = None;
        for ep in &endpoints {
            if ep.sppid_class == "PIDBranchPoint" {
                for &sp in &ep.ports {
                    let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                    if d <= snap_tol && best_opc.is_none_or(|(bd, _)| d < bd) {
                        best_opc = Some((d, ep.symbol_uid.as_str()));
                    }
                }
            } else if matches!(
                ep.sppid_class.as_str(),
                "PIDPipingComponent" | "PIDInlineInstrument" | "PIDNozzle" | "PIDProcessEquipment"
            ) {
                for &sp in &ep.ports {
                    let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                    if d <= snap_tol && best_port.is_none_or(|(bd, _)| d < bd) {
                        best_port = Some((d, ep.symbol_uid.as_str()));
                    }
                }
            }
        }
        match (best_opc, best_port) {
            (Some((do_, ouid)), Some((dp, puid))) => {
                if do_ <= dp {
                    Some((true, ouid))
                } else {
                    Some((false, puid))
                }
            }
            (Some((_, ouid)), None) => Some((true, ouid)),
            (None, Some((_, puid))) => Some((false, puid)),
            (None, None) => None,
        }
    };

    for (ri, r) in run_inputs.iter().enumerate() {
        for (is_end2, pt) in [(false, r.start), (true, r.end)] {
            if let Some((is_opc, uid_str)) = classify_snap(pt) {
                if is_opc {
                    dead_end_attribution.snapped_to_offpage += 1;
                    if let Some(ref lbl) = r.line_label {
                        opc_line_label
                            .entry(uid_str.to_string())
                            .or_insert_with(|| lbl.clone());
                    }
                } else {
                    dead_end_attribution.snapped_to_port += 1;
                }
            } else {
                unsnapped_pts.push((ri, is_end2, pt));
            }
        }
    }

    let mut used_un = vec![false; unsnapped_pts.len()];
    for i in 0..unsnapped_pts.len() {
        if used_un[i] {
            continue;
        }
        let anchor = unsnapped_pts[i].2;
        let mut group = vec![i];
        used_un[i] = true;
        for j in (i + 1)..unsnapped_pts.len() {
            if !used_un[j]
                && (unsnapped_pts[j].2 .0 - anchor.0).hypot(unsnapped_pts[j].2 .1 - anchor.1)
                    <= snap_tol
            {
                used_un[j] = true;
                group.push(j);
            }
        }
        let distinct_runs: BTreeSet<usize> = group.iter().map(|&gi| unsnapped_pts[gi].0).collect();
        if distinct_runs.len() >= 3 {
            dead_end_attribution.snapped_to_tee += group.len();
        } else {
            for &gi in &group {
                let pt = unsnapped_pts[gi].2;
                let near_eq = endpoints.iter().any(|ep| {
                    matches!(
                        ep.sppid_class.as_str(),
                        "PIDProcessVessel"
                            | "PIDMechanicalEquipment"
                            | "PIDProcessEquipment"
                            | "PIDHeatTransferEquipment"
                    ) && point_in_bbox(pt, ep.bbox, 3.0 * snap_tol)
                });
                if near_eq {
                    dead_end_attribution.touching_equipment_shell += 1;
                } else {
                    dead_end_attribution.open_free_end += 1;
                }
            }
        }
    }

    let mut offpage_connectors: Vec<OffpageConnectorEntry> = Vec::new();
    for ps in &placed_symbols {
        let sym = &rec.symbols[ps.sym_idx];
        if sym.sppid_class != "PIDBranchPoint" {
            continue;
        }
        let Some(ep_item) = dwg
            .items
            .iter()
            .find(|it| it.graphic_oids.contains(&ps.graphic_oid))
        else {
            continue;
        };
        offpage_connectors.push(OffpageConnectorEntry {
            uid: ep_item.uid.clone(),
            target_dwg_number: sym.to_from_text.clone(),
            description: sym.description.clone(),
            line_label: opc_line_label.get(&ep_item.uid).cloned(),
        });
    }

    let pipe_network_census = PipeNetworkCensus {
        runs_by_layer,
        labeled_runs,
        unlabeled_runs,
        collinear_bridges,
        filtered_micro_stubs,
        dead_end_attribution,
    };

    let report = ConvertSheetReport {
        drawing_name: effective_doc_name,
        doc_uid,
        plant: opts.plant.clone(),
        title_block: rec.title_block.clone(),
        confidence_summary: rec.confidence_summary.clone(),
        exported_by_class,
        piping_synthesis: piping_stats,
        pipe_network_census,
        offpage_connectors,
        total_items_written: dwg.items.len(),
        total_relations_written: dwg.relations.len(),
        skipped_items: dwg.unmapped.clone(),
        pid_verification: None,
    };

    Ok(CoreConversionState {
        rec,
        dwg,
        report,
        placed_symbols,
        placed_runs,
    })
}

/// Convert an in-memory `Document` into `(_Data.xml, _Meta.xml, _Coverage.json)` bytes.
pub fn convert_document_to_publish(
    doc: &Document,
    doc_name: &str,
    opts: &ConvertOptions,
) -> Result<(ExportBundle, ConvertSheetReport), String> {
    let state = build_core_conversion(doc, doc_name, opts, None)?;
    let mut bundle = export_drawing_bundle(&state.dwg)?;
    bundle.coverage_json = serde_json::to_string_pretty(&state.report)
        .map_err(|e| format!("serialize coverage report: {e}"))?;
    Ok((bundle, state.report))
}

#[cfg(not(target_arch = "wasm32"))]
fn measure_doc_extent_mm(doc: &Document) -> (f64, f64) {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut consider = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() && x.abs() < 1e7 && y.abs() < 1e7 {
            min_x = min_x.min(x / upm);
            min_y = min_y.min(y / upm);
            max_x = max_x.max(x / upm);
            max_y = max_y.max(y / upm);
        }
    };
    for ent in doc.model_space_entities() {
        match ent {
            document::EntityType::Insert(ins) => consider(ins.insert_point.x, ins.insert_point.y),
            document::EntityType::Line(ln) => {
                consider(ln.start.x, ln.start.y);
                consider(ln.end.x, ln.end.y);
            }
            document::EntityType::LwPolyline(pl) => {
                for v in &pl.vertices {
                    consider(v.location.x, v.location.y);
                }
            }
            document::EntityType::Text(t) => consider(t.insertion_point.x, t.insertion_point.y),
            _ => {}
        }
    }
    if min_x.is_finite() && max_x >= min_x {
        ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0))
    } else {
        (841.0, 594.0)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_configured_seed(
    doc: &Document,
    opts: &ConvertOptions,
) -> Result<(pid_parse::PidPackage, Option<String>), String> {
    use crate::io::pid_write::{
        list_seeds_in_refdata_zip, load_seed_from_bytes, load_seed_from_path,
        load_seed_from_refdata_zip,
    };

    if let Some(ref raw) = opts.seed_pid_bytes {
        let pkg = load_seed_from_bytes(raw)?;
        return Ok((pkg, None));
    }

    let spec = opts
        .seed_pid_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "no seed_pid_path or seed_pid_bytes configured".to_string())?;

    if let Some((zip_part, entry_part)) = spec.split_once("::") {
        let pkg = load_seed_from_refdata_zip(Path::new(zip_part), entry_part.trim())?;
        let tpl = infer_igr_for_seed_name(entry_part.trim());
        return Ok((pkg, tpl));
    }

    let path = Path::new(spec);
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        let (w_mm, h_mm) = measure_doc_extent_mm(doc);
        let preferred = if w_mm <= 430.0 && h_mm <= 305.0 {
            "A3-W-New.pid"
        } else if w_mm <= 605.0 && h_mm <= 430.0 {
            "A2-W-New.pid"
        } else if w_mm <= 855.0 && h_mm <= 605.0 {
            "A1-W-New.pid"
        } else {
            "A0-W-New.pid"
        };
        let available = list_seeds_in_refdata_zip(path)?;
        let chosen = available
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(preferred))
            .or_else(|| {
                available
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case("A1-W-New.pid"))
            })
            .or_else(|| available.first())
            .map(|(n, _)| n.clone())
            .ok_or_else(|| format!("no .pid seeds found in {}", path.display()))?;
        let pkg = load_seed_from_refdata_zip(path, &chosen)?;
        let tpl = infer_igr_for_seed_name(&chosen);
        return Ok((pkg, tpl));
    }

    let pkg = load_seed_from_path(path)?;
    let tpl = path
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(infer_igr_for_seed_name);
    Ok((pkg, tpl))
}

#[cfg(not(target_arch = "wasm32"))]
fn infer_igr_for_seed_name(seed_name: &str) -> Option<String> {
    let stem = Path::new(seed_name).file_stem()?.to_str()?;
    if let Some(prefix) = stem.strip_suffix("-New") {
        Some(format!("{prefix} - New.igr"))
    } else {
        Some(format!("{stem}.igr"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pick_seed_style_ids(seed: &pid_parse::PidPackage) -> (u32, u32) {
    use pid_parse::parsers::sheet_records::{parse_live_psm_header, SheetChain};

    let mut line_style_id: Option<u32> = None;
    let mut text_style_id: Option<u32> = None;
    if let Some(sc) = seed.get_stream("/StyleCluster") {
        for start in SheetChain::walk(&sc.data).starts {
            let Some(hdr) = parse_live_psm_header(&sc.data, start) else {
                continue;
            };
            let Some(id_bytes) = sc.data.get(hdr.body_start + 14..hdr.body_start + 18) else {
                continue;
            };
            let style_id = u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);
            if style_id == 0 {
                continue;
            }
            if hdr.type_code == 0x002E && line_style_id.is_none() {
                line_style_id = Some(style_id);
            } else if hdr.type_code == 0x002C && text_style_id.is_none() {
                text_style_id = Some(style_id);
            }
        }
    }
    (line_style_id.unwrap_or(1), text_style_id.unwrap_or(1))
}

#[cfg(not(target_arch = "wasm32"))]
struct SheetCoordMapper {
    upm: f64,
    scale: f64,
    offset_x_mm: f64,
    offset_y_mm: f64,
}

#[cfg(not(target_arch = "wasm32"))]
impl SheetCoordMapper {
    fn from_state_and_page(state: &CoreConversionState, page_mm: (f64, f64)) -> Self {
        let upm = state.rec.units_per_mm.max(1e-6);
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut add_pt = |pt: (f64, f64)| {
            let x = pt.0 / upm;
            let y = pt.1 / upm;
            if x.is_finite() && y.is_finite() {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        };
        for ps in &state.placed_symbols {
            add_pt(state.rec.symbols[ps.sym_idx].at);
        }
        for pr in &state.placed_runs {
            for &p in &pr.points {
                add_pt(p);
            }
        }
        for lb in &state.rec.pipe_labels {
            add_pt(lb.at);
        }

        if !min_x.is_finite() || max_x < min_x {
            return Self {
                upm,
                scale: 1.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            };
        }

        let margin = 10.0;
        let (pw, ph) = page_mm;
        if min_x >= 2.0 && min_y >= 2.0 && max_x <= pw - 2.0 && max_y <= ph - 2.0 {
            return Self {
                upm,
                scale: 1.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            };
        }

        let span_x = (max_x - min_x).max(1.0);
        let span_y = (max_y - min_y).max(1.0);
        let avail_w = (pw - 2.0 * margin).max(50.0);
        let avail_h = (ph - 2.0 * margin).max(50.0);
        let fit_scale = (avail_w / span_x).min(avail_h / span_y).min(1.0);
        Self {
            upm,
            scale: fit_scale,
            offset_x_mm: margin - min_x * fit_scale,
            offset_y_mm: margin - min_y * fit_scale,
        }
    }

    fn map_pt(&self, dxf_pt: (f64, f64)) -> (f64, f64) {
        let x_mm = (dxf_pt.0 / self.upm) * self.scale + self.offset_x_mm;
        let y_mm = (dxf_pt.1 / self.upm) * self.scale + self.offset_y_mm;
        (x_mm, y_mm)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn compute_representation_uid_map(dwg: &ExportDrawing) -> BTreeMap<(String, u32), String> {
    let mut rep_alloc = UidAllocator::new(&dwg.plant, &dwg.doc_name);
    let _ = rep_alloc.alloc_primary_uid(Some(&dwg.doc_uid), "doc");
    for item in &dwg.items {
        if is_valid_hex32(&item.uid) {
            let _ = rep_alloc.alloc_primary_uid(Some(&item.uid), &item.uid);
        }
    }
    let mut out = BTreeMap::new();
    for item in &dwg.items {
        for &oid in &item.graphic_oids {
            let rep_uid = rep_alloc.alloc_primary_uid(None, &format!("rep:{}:{oid}", item.uid));
            out.insert((item.uid.clone(), oid), rep_uid);
        }
    }
    out
}

/// Convert an in-memory `Document` into publish XML plus synchronized `.pid` bytes (`#130`).
#[cfg(not(target_arch = "wasm32"))]
pub fn convert_document_to_dual(
    doc: &Document,
    doc_name: &str,
    opts: &ConvertOptions,
) -> Result<DualConvertBundle, String> {
    use crate::io::pid_write::{
        apply_increments_to_seed, AddedAttributeSpec, AddedLineSpec, AddedPolylineSpec,
        AddedSymbolSpec, AddedTextSpec, PidWriteIncrements, SeedOidPool,
    };
    use pid_parse::{build_normalized_geometry, PidWriter, WritePlan};

    if !opts.wants_dual_pid() {
        let (publish, report) = convert_document_to_publish(doc, doc_name, opts)?;
        return Ok(DualConvertBundle {
            publish,
            pid_bytes: None,
            report,
        });
    }

    let (mut seed_pkg, inferred_tpl) = load_configured_seed(doc, opts)?;
    let pool = SeedOidPool::from_seed(&seed_pkg);
    let max_seed_oid = pool.used_oids().iter().copied().max().unwrap_or(99);
    let min_next_oid = max_seed_oid.saturating_add(1).max(100);

    let mut state =
        build_core_conversion(doc, doc_name, opts, Some((min_next_oid, pool.used_oids())))?;

    let seed_geom = build_normalized_geometry(&seed_pkg.parsed);
    let page_mm = seed_geom.page_dimensions_mm.unwrap_or((841.0, 594.0));
    let mapper = SheetCoordMapper::from_state_and_page(&state, page_mm);
    let (line_style_id, text_style_id) = pick_seed_style_ids(&seed_pkg);
    let rep_uid_map = compute_representation_uid_map(&state.dwg);

    let item_by_goid: BTreeMap<u32, &ExportItem> = state
        .dwg
        .items
        .iter()
        .flat_map(|it| it.graphic_oids.iter().copied().map(move |oid| (oid, it)))
        .collect();

    let mut inc = PidWriteIncrements {
        doc_uid: Some(state.report.doc_uid.clone()),
        plant_name: Some(opts.plant.clone()),
        template_name: inferred_tpl,
        ..Default::default()
    };

    for ps in &state.placed_symbols {
        let sym = &state.rec.symbols[ps.sym_idx];
        let pt_mm = mapper.map_pt(sym.at);
        let rel_sym = sym
            .symbol_path
            .clone()
            .unwrap_or_else(|| format!("Symbols\\{}.sym", sym.source_block));
        inc.symbols.push(AddedSymbolSpec {
            preferred_oid: Some(ps.graphic_oid),
            symbol_rel_path: rel_sym,
            insertion_mm: pt_mm,
            rotation_rad: 0.0,
            scale: mapper.scale,
            style_ref: line_style_id,
        });

        if let Some(label_txt) = sym.tag.as_deref().or(sym.to_from_text.as_deref()) {
            inc.texts.push(AddedTextSpec {
                preferred_oid: None,
                text: label_txt.to_string(),
                insertion_mm: (pt_mm.0, (pt_mm.1 + 3.0).min(page_mm.1 - 4.0)),
                rotation_rad: 0.0,
                style_index: text_style_id,
            });
        }

        if let Some(item) = item_by_goid.get(&ps.graphic_oid) {
            let rep_uid = rep_uid_map
                .get(&(item.uid.clone(), ps.graphic_oid))
                .cloned()
                .ok_or_else(|| format!("missing representation UID for {}", item.uid))?;
            inc.attributes.push(AddedAttributeSpec {
                described_graphic_oid: ps.graphic_oid,
                model_uid: item.uid.clone(),
                representation_uid: rep_uid,
                sppid_class: item.class.clone(),
                drawing_no: doc_name.to_string(),
                item_tag: item.name.clone(),
            });
        }
    }

    for pr in &state.placed_runs {
        let mapped_pts: Vec<(f64, f64)> = pr.points.iter().map(|&p| mapper.map_pt(p)).collect();
        if mapped_pts.len() == 2 {
            inc.lines.push(AddedLineSpec {
                preferred_oid: Some(pr.graphic_oid),
                start_mm: mapped_pts[0],
                end_mm: mapped_pts[1],
                style_index: line_style_id,
            });
        } else if mapped_pts.len() >= 3 {
            inc.polylines.push(AddedPolylineSpec {
                preferred_oid: Some(pr.graphic_oid),
                vertices_mm: mapped_pts,
                style_index: line_style_id,
            });
        }

        if let Some(item) = item_by_goid.get(&pr.graphic_oid) {
            let rep_uid = rep_uid_map
                .get(&(item.uid.clone(), pr.graphic_oid))
                .cloned()
                .ok_or_else(|| format!("missing representation UID for {}", item.uid))?;
            inc.attributes.push(AddedAttributeSpec {
                described_graphic_oid: pr.graphic_oid,
                model_uid: item.uid.clone(),
                representation_uid: rep_uid,
                sppid_class: item.class.clone(),
                drawing_no: doc_name.to_string(),
                item_tag: item.name.clone(),
            });
        }
    }

    for lb in &state.rec.pipe_labels {
        let pt_mm = mapper.map_pt(lb.at);
        inc.texts.push(AddedTextSpec {
            preferred_oid: None,
            text: lb.raw_text.clone(),
            insertion_mm: pt_mm,
            rotation_rad: 0.0,
            style_index: text_style_id,
        });
    }

    let sym_dir = opts
        .sym_library_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    apply_increments_to_seed(&mut seed_pkg, &inc, sym_dir.as_deref())?;

    let pid_bytes = PidWriter::write_to_bytes(&seed_pkg, &WritePlan::default())
        .map_err(|e| format!("write .pid bytes for {doc_name}: {e}"))?;

    let ver = verify_dual_bundle(
        &state.report.doc_uid,
        &export_drawing_bundle(&state.dwg)?.data_xml,
        &pid_bytes,
        None,
        doc_name,
    )?;
    state.report.pid_verification = Some(ver);

    let mut publish = export_drawing_bundle(&state.dwg)?;
    publish.coverage_json = serde_json::to_string_pretty(&state.report)
        .map_err(|e| format!("serialize coverage report: {e}"))?;

    Ok(DualConvertBundle {
        publish,
        pid_bytes: Some(pid_bytes),
        report: state.report,
    })
}

#[cfg(not(target_arch = "wasm32"))]
type XmlRepLink = (String, u32, String);

#[cfg(not(target_arch = "wasm32"))]
fn extract_attr_value<'a>(line: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let idx = line.find(&needle)?;
    let rest = &line[idx + needle.len()..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_xml_rep_links(data_xml: &[u8]) -> Result<Vec<XmlRepLink>, String> {
    let body = data_xml
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(data_xml);
    let text = std::str::from_utf8(body).map_err(|e| format!("invalid UTF-8 in _Data.xml: {e}"))?;

    let mut rep_goid: BTreeMap<String, u32> = BTreeMap::new();
    let mut rep_to_model: BTreeMap<String, String> = BTreeMap::new();

    let mut in_rep = false;
    let mut cur_rep_uid: Option<String> = None;
    let mut cur_goid: Option<u32> = None;

    let mut in_rel = false;
    let mut cur_uid1: Option<String> = None;
    let mut cur_uid2: Option<String> = None;
    let mut cur_def: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line == "<PIDRepresentation>" {
            in_rep = true;
            cur_rep_uid = None;
            cur_goid = None;
        } else if line == "</PIDRepresentation>" {
            if let (Some(ru), Some(go)) = (cur_rep_uid.take(), cur_goid.take()) {
                rep_goid.insert(ru, go);
            }
            in_rep = false;
        } else if in_rep {
            if line.starts_with("<IObject ") {
                cur_rep_uid = extract_attr_value(line, "UID").map(str::to_string);
            } else if line.starts_with("<IDrawingRepresentation ") {
                cur_goid = extract_attr_value(line, "GraphicOID").and_then(|s| s.parse().ok());
            }
        } else if line == "<Rel>" {
            in_rel = true;
            cur_uid1 = None;
            cur_uid2 = None;
            cur_def = None;
        } else if line == "</Rel>" {
            if cur_def.as_deref() == Some("DrawingItems") {
                if let (Some(u1), Some(u2)) = (cur_uid1.take(), cur_uid2.take()) {
                    rep_to_model.insert(u1, u2);
                }
            }
            in_rel = false;
        } else if in_rel && line.starts_with("<IRel ") {
            cur_uid1 = extract_attr_value(line, "UID1").map(str::to_string);
            cur_uid2 = extract_attr_value(line, "UID2").map(str::to_string);
            cur_def = extract_attr_value(line, "DefUID").map(str::to_string);
        }
    }

    let mut out = Vec::with_capacity(rep_goid.len());
    for (rep_uid, goid) in rep_goid {
        let model_uid = rep_to_model.get(&rep_uid).cloned().unwrap_or_default();
        out.push((rep_uid, goid, model_uid));
    }
    Ok(out)
}

/// Verify cross-file `UID`/`GraphicOID` 100% alignment, `a`-level re-read/index
/// consistency, and optional `b`-level CAD load + `pid-plot` PDF rendering (`#130`).
#[cfg(not(target_arch = "wasm32"))]
pub fn verify_dual_bundle(
    doc_uid: &str,
    data_xml: &[u8],
    pid_bytes: &[u8],
    plot_work_dir: Option<&Path>,
    stem: &str,
) -> Result<DualVerifyReport, String> {
    use crate::io::pid_write::{load_seed_from_bytes, verify_package_index_consistency};
    use pid_parse::parsers::attribute_rows::decode_attribute_row;
    use pid_parse::parsers::sheet_records::{
        decode_iglines, decode_iglinestrings, decode_igsymbols, decode_igtextboxes,
        parse_live_psm_header, SheetChain,
    };
    use pid_parse::{build_normalized_geometry, SummaryPropertyValue};

    let rep_links = parse_xml_rep_links(data_xml)?;
    let reparsed = load_seed_from_bytes(pid_bytes)?;

    let spid_matches_doc_uid = reparsed
        .parsed
        .summary
        .as_ref()
        .and_then(|s| s.user_properties.get("_SPID"))
        .is_some_and(|v| {
            matches!(
                v,
                SummaryPropertyValue::Lpstr(t) | SummaryPropertyValue::Lpwstr(t) if t == doc_uid
            )
        });

    let mut sheet_goids: BTreeSet<u32> = BTreeSet::new();
    for (path, raw) in &reparsed.streams {
        let trimmed = path.trim_start_matches('/');
        if trimmed.contains('/') || !trimmed.starts_with("Sheet") {
            continue;
        }
        for r in decode_iglines(&raw.data) {
            sheet_goids.insert(r.oid);
        }
        for r in decode_iglinestrings(&raw.data) {
            sheet_goids.insert(r.oid);
        }
        for r in decode_igsymbols(&raw.data) {
            sheet_goids.insert(r.oid);
        }
        for r in decode_igtextboxes(&raw.data) {
            sheet_goids.insert(r.oid);
        }
    }

    let mut attr_by_oid: BTreeMap<u32, (String, String)> = BTreeMap::new();
    if let Some(uda) = reparsed.get_stream("/Unclustered Dynamic Attributes") {
        for start in SheetChain::walk(&uda.data).starts {
            let Some(hdr) = parse_live_psm_header(&uda.data, start) else {
                continue;
            };
            if hdr.type_code != 0x0089 {
                continue;
            }
            let Some(payload) = uda
                .data
                .get(hdr.body_start..hdr.body_start + hdr.bytes_to_follow as usize)
            else {
                continue;
            };
            if let Some(row) = decode_attribute_row(payload) {
                let model_id = row.text("ModelID").unwrap_or_default().to_string();
                let drawing_id = row.text("DrawingID").unwrap_or_default().to_string();
                attr_by_oid.insert(row.described, (model_id, drawing_id));
            }
        }
    }

    let mut matched_graphic_oids = 0usize;
    let mut matched_model_uids = 0usize;
    let mut matched_representation_uids = 0usize;
    let mut missing_graphic_oids_in_pid = Vec::new();
    let mut missing_model_uids_in_pid = Vec::new();
    let mut mismatched_links = Vec::new();

    for (rep_uid, goid, model_uid) in &rep_links {
        let on_sheet = sheet_goids.contains(goid);
        let attr_entry = attr_by_oid.get(goid);
        if on_sheet && attr_entry.is_some() {
            matched_graphic_oids += 1;
        } else {
            missing_graphic_oids_in_pid.push(*goid);
        }

        if let Some((pid_model_uid, pid_rep_uid)) = attr_entry {
            if pid_model_uid == model_uid {
                matched_model_uids += 1;
            } else {
                missing_model_uids_in_pid.push(model_uid.clone());
                mismatched_links.push(format!(
                    "GraphicOID {goid}: XML ModelID={model_uid} != .pid ModelID={pid_model_uid}"
                ));
            }
            if pid_rep_uid == rep_uid {
                matched_representation_uids += 1;
            } else {
                mismatched_links.push(format!(
                    "GraphicOID {goid}: XML RepUID={rep_uid} != .pid DrawingID={pid_rep_uid}"
                ));
            }
        } else {
            missing_model_uids_in_pid.push(model_uid.clone());
        }
    }

    let idx_chk = verify_package_index_consistency(&reparsed)?;
    let norm_geom = build_normalized_geometry(&reparsed.parsed);

    let mut b_level_cad_entities = None;
    let mut b_level_pdf_bytes = None;
    let mut b_level_pdf_primitives = None;
    let mut b_level_pdf_on_page = None;

    if let Some(work_dir) = plot_work_dir {
        std::fs::create_dir_all(work_dir)
            .map_err(|e| format!("create plot dir {}: {e}", work_dir.display()))?;
        let pid_path = work_dir.join(format!("{stem}.pid"));
        if !pid_path.is_file() {
            std::fs::write(&pid_path, pid_bytes)
                .map_err(|e| format!("write {}: {e}", pid_path.display()))?;
        }

        let cad_doc = crate::io::load_file(&pid_path)
            .map_err(|e| format!("b-level CAD load failed for {}: {e}", pid_path.display()))?;
        b_level_cad_entities = Some(cad_doc.model_space_entities().count());

        let pdf_path = work_dir.join(format!("{stem}.pidplot.pdf"));
        let receipt = crate::io::pid_pdf::plot_pdf(&pid_path, &pdf_path)?;
        let pdf_raw = std::fs::read(&pdf_path)
            .map_err(|e| format!("read plotted PDF {}: {e}", pdf_path.display()))?;
        if !pdf_raw.starts_with(b"%PDF-") {
            return Err(format!(
                "plotted PDF {} missing %PDF- header",
                pdf_path.display()
            ));
        }
        b_level_pdf_bytes = Some(receipt.bytes);
        b_level_pdf_primitives = Some(receipt.primitives);
        b_level_pdf_on_page = Some(receipt.ledger.on_page);
    }

    let sppid_native_c1_status = if std::env::var_os("OCS_PID_SPPID_RUNNER").is_some() {
        "runner_configured".to_string()
    } else {
        "unverified_no_sppid_runner".to_string()
    };

    Ok(DualVerifyReport {
        xml_rep_count: rep_links.len(),
        matched_graphic_oids,
        matched_model_uids,
        matched_representation_uids,
        missing_graphic_oids_in_pid,
        missing_model_uids_in_pid,
        mismatched_links,
        spid_matches_doc_uid,
        a_level_index_consistent: idx_chk.is_consistent(),
        a_level_normalized_entities: norm_geom.entities.len(),
        b_level_cad_entities,
        b_level_pdf_bytes,
        b_level_pdf_primitives,
        b_level_pdf_on_page,
        sppid_native_c1_status,
    })
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
/// directory, writing `<stem>_Data.xml`, `<stem>_Meta.xml`, `<stem>_Coverage.json`,
/// and (when a seed `.pid` is configured) `<stem>.pid` + optional `<stem>.pidplot.pdf`.
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
            Ok(doc) => match convert_document_to_dual(&doc, &stem, opts) {
                Ok(mut dual) => {
                    let data_p = out_dir.join(format!("{stem}_Data.xml"));
                    let meta_p = out_dir.join(format!("{stem}_Meta.xml"));
                    let cov_p = out_dir.join(format!("{stem}_Coverage.json"));
                    let mut write_res = std::fs::write(&data_p, &dual.publish.data_xml)
                        .and_then(|_| std::fs::write(&meta_p, &dual.publish.meta_xml))
                        .map_err(|e| format!("write xml failed: {e}"));

                    if write_res.is_ok() {
                        if let Some(ref pid_raw) = dual.pid_bytes {
                            let pid_p = out_dir.join(format!("{stem}.pid"));
                            write_res = std::fs::write(&pid_p, pid_raw)
                                .map_err(|e| format!("write .pid failed: {e}"));
                            if write_res.is_ok() && opts.verify_plot_pdf {
                                match verify_dual_bundle(
                                    &dual.report.doc_uid,
                                    &dual.publish.data_xml,
                                    pid_raw,
                                    Some(out_dir),
                                    &stem,
                                ) {
                                    Ok(ver) => {
                                        dual.report.pid_verification = Some(ver);
                                        if let Ok(updated_cov) =
                                            serde_json::to_string_pretty(&dual.report)
                                        {
                                            dual.publish.coverage_json = updated_cov;
                                        }
                                    }
                                    Err(err) => {
                                        write_res = Err(format!("plot/verify failed: {err}"));
                                    }
                                }
                            }
                        }
                    }

                    if write_res.is_ok() {
                        write_res =
                            std::fs::write(&cov_p, format!("{}\n", dual.publish.coverage_json))
                                .map_err(|e| format!("write coverage failed: {e}"));
                    }

                    match write_res {
                        Ok(()) => {
                            summary.files_converted += 1;
                            if dual.pid_bytes.is_some() {
                                summary.pid_files_written += 1;
                                if opts.verify_plot_pdf {
                                    summary.pdf_files_plotted += 1;
                                }
                            }
                            summary.total_items_written += dual.report.total_items_written;
                            summary.total_relations_written += dual.report.total_relations_written;
                            summary.sheets.push(dual.report);
                        }
                        Err(err) => {
                            summary.files_failed += 1;
                            summary.errors.push(format!("{stem}: {err}"));
                        }
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
    summary.cross_sheet_opc_pairing = pair_cross_sheet_connectors(&summary.sheets);
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

    #[cfg(not(target_arch = "wasm32"))]
    fn build_synthetic_seed_pid_bytes() -> Vec<u8> {
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

        let mut streams = BTreeMap::new();
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());

        let mut cluster0 = 0x6C90_F544u32.to_le_bytes().to_vec();
        cluster0.extend_from_slice(&2u32.to_le_bytes());
        cluster0.extend_from_slice(&make_layer(8, "Default"));
        cluster0.extend_from_slice(&make_layer(12, "Labels"));

        streams.insert(
            "/Sheet6".to_string(),
            RawStream {
                path: "/Sheet6".to_string(),
                data: empty_chain.clone(),
                modified: false,
            },
        );
        streams.insert(
            "/PSMcluster0".to_string(),
            RawStream {
                path: "/PSMcluster0".to_string(),
                data: cluster0,
                modified: false,
            },
        );
        streams.insert(
            "/Unclustered Dynamic Attributes".to_string(),
            RawStream {
                path: "/Unclustered Dynamic Attributes".to_string(),
                data: empty_chain.clone(),
                modified: false,
            },
        );
        streams.insert(
            "/StyleCluster".to_string(),
            RawStream {
                path: "/StyleCluster".to_string(),
                data: empty_chain,
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
        streams.insert(
            "/TaggedTxtData/Revision".to_string(),
            RawStream {
                path: "/TaggedTxtData/Revision".to_string(),
                data: Vec::new(),
                modified: false,
            },
        );

        let pkg = PidPackage::new(None, streams, PidDocument::default());
        PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap()
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

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dual_converts_synthetic_sheet_with_100pct_uid_graphic_oid_and_ab_roundtrip() {
        let doc = build_synthetic_sheet();
        let seed_bytes = build_synthetic_seed_pid_bytes();
        let opts = ConvertOptions {
            seed_pid_bytes: Some(seed_bytes),
            verify_plot_pdf: true,
            ..Default::default()
        };

        let dual = convert_document_to_dual(&doc, "SYNTH-DUAL-01", &opts).expect("dual convert");
        let pid_bytes = dual
            .pid_bytes
            .as_ref()
            .expect("dual convert must emit .pid bytes");

        let spec = default_publish_spec();
        let chk_opts = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..Default::default()
        };
        assert!(check_publish_xml_bytes(
            "SYNTH-DUAL-01_Data.xml",
            PublishXmlKind::Data,
            &dual.publish.data_xml,
            spec,
            &chk_opts,
        )
        .is_empty());

        let tmp = std::env::temp_dir().join(format!("ocs_pid_dual_verify_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let ver = verify_dual_bundle(
            &dual.report.doc_uid,
            &dual.publish.data_xml,
            pid_bytes,
            Some(&tmp),
            "SYNTH-DUAL-01",
        )
        .expect("verify dual bundle");

        assert!(ver.xml_rep_count >= 5, "unexpected rep count: {ver:?}");
        assert!(
            ver.is_cross_file_and_a_valid(),
            "cross-file or a-level check failed: {ver:?}"
        );
        assert!(
            ver.is_b_plot_valid(),
            "b-level CAD load or PDF plot check failed: {ver:?}"
        );
        assert_eq!(ver.sppid_native_c1_status, "unverified_no_sppid_runner");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn batch_converts_directory_to_quadruple_outputs_with_pid_and_pdf() {
        let doc = build_synthetic_sheet();
        let seed_bytes = build_synthetic_seed_pid_bytes();
        let tmp = std::env::temp_dir().join(format!("ocs_pid_dual_batch_{}", std::process::id()));
        let in_dir = tmp.join("in");
        let out_dir = tmp.join("out");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&in_dir).unwrap();

        let seed_path = tmp.join("seed_blank.pid");
        std::fs::write(&seed_path, &seed_bytes).unwrap();

        let dxf_path = in_dir.join("UNIT_DUAL_01.dxf");
        crate::io::save(&doc, &dxf_path).unwrap();
        std::fs::write(in_dir.join("EMPTY_0BYTE.dxf"), b"").unwrap();

        let opts = ConvertOptions {
            seed_pid_path: Some(seed_path.to_string_lossy().into_owned()),
            verify_plot_pdf: true,
            ..Default::default()
        };

        let summary = convert_path_to_dir(&in_dir, &out_dir, &opts).expect("dual batch convert");
        assert_eq!(summary.files_converted, 1);
        assert_eq!(summary.files_failed, 0);
        assert_eq!(summary.pid_files_written, 1);
        assert_eq!(summary.pdf_files_plotted, 1);
        assert!(out_dir.join("UNIT_DUAL_01_Data.xml").is_file());
        assert!(out_dir.join("UNIT_DUAL_01_Meta.xml").is_file());
        assert!(out_dir.join("UNIT_DUAL_01.pid").is_file());
        assert!(out_dir.join("UNIT_DUAL_01_Coverage.json").is_file());
        assert!(out_dir.join("UNIT_DUAL_01.pidplot.pdf").is_file());

        let rep = &summary.sheets[0];
        let pver = rep
            .pid_verification
            .as_ref()
            .expect("sheet report carries pid_verification");
        assert!(pver.is_cross_file_and_a_valid(), "{pver:?}");
        assert!(pver.is_b_plot_valid(), "{pver:?}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bridges_collinear_pipe_gaps_filters_micro_stubs_and_snaps_cardinal_valve_ports() {
        use document::entities::{Block, BlockEnd, Insert, Line, Text};
        use document::tables::BlockRecord;
        use document::types::Vector3;
        use document::{EntityType, Handle};

        let mut doc = Document::new();
        // Define a valve block WITHOUT explicit Point entities (only stroke lines, 6mm wide)
        let br_h = Handle::new(100);
        let b_h = Handle::new(101);
        let e_h = Handle::new(102);
        let mut rec = BlockRecord::new("BALL VALVE TYPE 2");
        rec.handle = br_h;
        rec.block_entity_handle = b_h;
        rec.block_end_handle = e_h;
        doc.block_records.add(rec).unwrap();
        let mut blk = Block::new("BALL VALVE TYPE 2", Vector3::ZERO);
        blk.common.handle = b_h;
        blk.common.owner_handle = br_h;
        doc.add_entity(EntityType::Block(blk)).unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = e_h;
        end.common.owner_handle = br_h;
        doc.add_entity(EntityType::BlockEnd(end)).unwrap();
        let mut stroke =
            Line::from_points(Vector3::new(-3.0, -2.0, 0.0), Vector3::new(3.0, 2.0, 0.0));
        stroke.common.owner_handle = br_h;
        doc.add_entity(EntityType::Line(stroke)).unwrap();

        let mut v_ins = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 100.0, 0.0));
        v_ins.common.layer = "11".to_string();
        doc.add_entity(EntityType::Insert(v_ins)).unwrap();

        // Upstream run broken by a 3.0mm collinear line-jump gap: (20,100)->(50,100) and (53,100)->(97,100)
        // Downstream run from right edge of valve: (103,100)->(160,100)
        // Plus two 0.8mm isolated arrow-barb micro-stubs on layer "11" far from ports
        for (a, b) in [
            ((20.0, 100.0), (50.0, 100.0)),
            ((53.0, 100.0), (97.0, 100.0)),
            ((103.0, 100.0), (160.0, 100.0)),
            ((60.0, 100.4), (60.6, 100.0)),
            ((60.0, 99.6), (60.6, 100.0)),
        ] {
            let mut ln =
                Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
            ln.common.layer = "11".to_string();
            doc.add_entity(EntityType::Line(ln)).unwrap();
        }
        let mut lbl =
            Text::with_value("50-PG-02011-B9-", Vector3::new(30.0, 102.5, 0.0)).with_height(2.5);
        lbl.common.layer = "11".to_string();
        doc.add_entity(EntityType::Text(lbl)).unwrap();

        let (_bundle, rep) =
            convert_document_to_publish(&doc, "SYNTH-CHAIN-133", &ConvertOptions::default())
                .expect("convert");

        assert_eq!(rep.pipe_network_census.collinear_bridges, 1);
        assert!(rep.pipe_network_census.filtered_micro_stubs >= 2);
        assert_eq!(rep.piping_synthesis.piping_connectors, 2);
        assert_eq!(rep.piping_synthesis.piping_ports, 2);
        assert_eq!(
            rep.pipe_network_census
                .dead_end_attribution
                .total_endpoints(),
            rep.piping_synthesis.piping_connectors * 2
        );
    }

    #[test]
    fn pairs_reciprocal_one_way_and_external_offpage_connectors_across_sheets() {
        let s1 = ConvertSheetReport {
            drawing_name: "SHEET_A".to_string(),
            title_block: TitleBlockInfo {
                drawing_number: Some("DWG-0201集01-01".to_string()),
                ..Default::default()
            },
            offpage_connectors: vec![
                OffpageConnectorEntry {
                    uid: "OPC_A1".to_string(),
                    target_dwg_number: Some("DWG-0201集 01-02".to_string()),
                    description: None,
                    line_label: Some("50-PG-02011-B9-".to_string()),
                },
                OffpageConnectorEntry {
                    uid: "OPC_A2".to_string(),
                    target_dwg_number: Some("DWG-9999-EXT".to_string()),
                    description: None,
                    line_label: None,
                },
            ],
            ..Default::default()
        };
        let s2 = ConvertSheetReport {
            drawing_name: "SHEET_B".to_string(),
            title_block: TitleBlockInfo {
                drawing_number: Some("DWG-0201集01-02".to_string()),
                ..Default::default()
            },
            offpage_connectors: vec![
                OffpageConnectorEntry {
                    uid: "OPC_B1".to_string(),
                    target_dwg_number: Some("DWG-0201集01-01".to_string()),
                    description: None,
                    line_label: Some("50-PG-02011-B9-".to_string()),
                },
                OffpageConnectorEntry {
                    uid: "OPC_B2".to_string(),
                    target_dwg_number: Some("DWG-0201集01-01".to_string()),
                    description: None,
                    line_label: None,
                },
            ],
            ..Default::default()
        };

        let paired = pair_cross_sheet_connectors(&[s1, s2]);
        assert_eq!(paired.total_offpage_connectors, 4);
        assert_eq!(paired.reciprocal_paired_connectors, 2);
        assert_eq!(paired.one_way_in_batch, 1);
        assert_eq!(paired.external_target_dwg, 1);
        assert_eq!(paired.pairs.len(), 1);
        assert_eq!(paired.pairs[0].uid_a, "OPC_A1");
        assert_eq!(paired.pairs[0].uid_b, "OPC_B1");
    }
}

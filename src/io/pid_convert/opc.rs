//! Multi-sheet off-page connector (`Off-Page Connector` / `PIDBranchPoint`) extraction,
//! reciprocal pairing audit, and cross-tab jump target resolution (`L3-4` / `L2-2` / `#149`).

use std::collections::{BTreeMap, BTreeSet};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable};

use super::class_map::default_legend_class_map;
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds, SheetRecognition};

/// Input descriptor for one open drawing tab analyzed by `PIDOPC`.
pub struct OpcSheetInput<'a> {
    pub tab_index: usize,
    pub fallback_name: &'a str,
    pub document: &'a Document,
}

/// Pairing classification of an off-page connector across open workspace tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcStatus {
    Paired,
    OneWayOpen,
    ExternalUnopened,
    MissingTarget,
}

/// Resolved peer connector target on another open tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpcPeerRef {
    pub tab_index: usize,
    pub drawing_code: String,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    pub line_label: Option<String>,
}

/// One analyzed off-page connector on an open drawing tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpcConnectorItem {
    pub tab_index: usize,
    pub drawing_code: String,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    pub target_dwg_raw: Option<String>,
    pub expanded_targets: Vec<String>,
    pub line_label: Option<String>,
    pub status: OpcStatus,
    pub peer: Option<OpcPeerRef>,
    pub target_open_tab: Option<usize>,
}

/// Multi-tab off-page connector audit report produced by `analyze_open_sheets_opc`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceOpcReport {
    pub open_sheets: usize,
    pub total_connectors: usize,
    pub reciprocal_pairs: usize,
    pub paired_connectors: usize,
    pub one_way_open: usize,
    pub external_unopened: usize,
    pub missing_target: usize,
    pub connectors: Vec<OpcConnectorItem>,
}

fn norm_dwg_code(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('-')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase()
}

fn strip_known_ext(name: &str) -> &str {
    let trimmed = name.trim();
    for ext in [".dxf", ".dwg", ".pid", ".DXF", ".DWG", ".PID"] {
        if let Some(stem) = trimmed.strip_suffix(ext) {
            return stem.trim();
        }
    }
    trimmed
}

fn extract_leading_dwg_token(stem: &str) -> Option<String> {
    let first = stem.split_whitespace().next().unwrap_or("").trim();
    let up = first.to_uppercase();
    if up.starts_with("DWG-") && up.len() > 4 {
        Some(norm_dwg_code(first))
    } else {
        None
    }
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

fn point_to_segment_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * vx)).hypot(p.1 - (a.1 + t * vy))
}

fn entity_xy(ent: &EntityType) -> Option<(f64, f64)> {
    match ent {
        EntityType::Insert(ins) => Some((ins.insert_point.x, ins.insert_point.y)),
        EntityType::Circle(c) => Some((c.center.x, c.center.y)),
        EntityType::Arc(a) => Some((a.center.x, a.center.y)),
        EntityType::Line(l) => Some(((l.start.x + l.end.x) * 0.5, (l.start.y + l.end.y) * 0.5)),
        EntityType::LwPolyline(pl) if !pl.vertices.is_empty() => {
            let n = pl.vertices.len() as f64;
            let sx: f64 = pl.vertices.iter().map(|v| v.location.x).sum();
            let sy: f64 = pl.vertices.iter().map(|v| v.location.y).sum();
            Some((sx / n, sy / n))
        }
        EntityType::Text(t) => Some((t.insertion_point.x, t.insertion_point.y)),
        EntityType::MText(mt) => Some((mt.insertion_point.x, mt.insertion_point.y)),
        _ => None,
    }
}

fn nearest_rec_line_label(rec: &SheetRecognition, pt: (f64, f64)) -> Option<String> {
    let mut best_seg_idx: Option<(usize, f64)> = None;
    for (si, seg) in rec.pipe_segments.iter().enumerate() {
        let d0 = (pt.0 - seg.start.0).hypot(pt.1 - seg.start.1);
        let d1 = (pt.0 - seg.end.0).hypot(pt.1 - seg.end.1);
        let d = d0.min(d1);
        if d <= 36.0 && best_seg_idx.is_none_or(|(_, bd)| d < bd) {
            best_seg_idx = Some((si, d));
        }
    }
    if let Some((si, _)) = best_seg_idx {
        let seg = &rec.pipe_segments[si];
        let mut best_lbl: Option<(&str, f64)> = None;
        for lbl in &rec.pipe_labels {
            let d = point_to_segment_dist(lbl.at, seg.start, seg.end);
            if d <= 28.0 && best_lbl.is_none_or(|(_, bd)| d < bd) {
                best_lbl = Some((lbl.raw_text.as_str(), d));
            }
        }
        if let Some((txt, _)) = best_lbl {
            return Some(txt.trim().to_string());
        }
    }
    None
}

fn parse_csv_handles(raw: &str) -> Vec<Handle> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(parse_handle)
        .collect()
}

fn collect_sheet_codes(fallback_name: &str, rec: &SheetRecognition) -> (String, BTreeSet<String>) {
    let mut codes = BTreeSet::new();
    let stem = strip_known_ext(fallback_name);
    let tb_code = rec
        .title_block
        .drawing_number
        .as_deref()
        .map(norm_dwg_code)
        .filter(|s| !s.is_empty());
    if let Some(ref c) = tb_code {
        codes.insert(c.clone());
    }
    if let Some(lead) = extract_leading_dwg_token(stem) {
        codes.insert(lead);
    }
    let full_norm = norm_dwg_code(stem);
    if !full_norm.is_empty() {
        codes.insert(full_norm.clone());
    }
    let primary = tb_code
        .or_else(|| extract_leading_dwg_token(stem))
        .unwrap_or(full_norm);
    (primary, codes)
}

fn extract_sheet_connectors(
    tab_index: usize,
    drawing_code: &str,
    doc: &Document,
    rec: &SheetRecognition,
) -> Vec<OpcConnectorItem> {
    let has_any_author = doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some());

    let mut out = Vec::new();
    if has_any_author {
        let topo = TopologyTable::load(doc);
        let run_to_line: BTreeMap<String, String> = topo
            .pipe_runs
            .iter()
            .map(|r| (r.tag.clone(), r.line_tag.clone()))
            .collect();

        let mut seen_handles = BTreeSet::new();
        for ent in doc.model_space_entities() {
            let h = ent.common().handle;
            let Some(auth) = PidAuthor::load(doc, h) else {
                continue;
            };
            let pmap: serde_json::Map<String, Value> =
                serde_json::from_str(&auth.props).unwrap_or_default();
            let sppid_cls = pmap
                .get("_sppid_class")
                .and_then(Value::as_str)
                .unwrap_or("");
            if sppid_cls != "PIDBranchPoint" && auth.type_code != "offpage_connector" {
                continue;
            }
            seen_handles.insert(h);

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
            let (x, y) = entity_xy(ent).unwrap_or((0.0, 0.0));
            let target_raw = pmap
                .get("IOffpageConnector.ToFromText")
                .or_else(|| pmap.get("ToFromText"))
                .or_else(|| pmap.get("_to_from_text"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    let t = auth.tag.trim();
                    if t.to_ascii_uppercase().starts_with("DWG-") {
                        Some(t.to_string())
                    } else {
                        None
                    }
                });

            let mut line_label = None;
            let mut best_d = 40.0_f64;
            for seg in &topo.pipe_segments {
                let d0 = (seg.start.point[0] - x).hypot(seg.start.point[1] - y);
                let d1 = (seg.end.point[0] - x).hypot(seg.end.point[1] - y);
                let d = d0.min(d1);
                if d <= best_d {
                    if let Some(lt) = run_to_line.get(&seg.run_tag).filter(|s| !s.is_empty()) {
                        best_d = d;
                        line_label = Some(lt.clone());
                    }
                }
            }
            if line_label.is_none() {
                line_label = nearest_rec_line_label(rec, (x, y));
            }

            let expanded_targets = target_raw
                .as_deref()
                .map(expand_target_dwg_codes)
                .unwrap_or_default();
            out.push(OpcConnectorItem {
                tab_index,
                drawing_code: drawing_code.to_string(),
                primary_handle: h,
                member_handles: members,
                primary_handle_hex: format!("{:X}", h.value()),
                x,
                y,
                target_dwg_raw: target_raw,
                expanded_targets,
                line_label,
                status: OpcStatus::MissingTarget,
                peer: None,
                target_open_tab: None,
            });
        }

        for conn in &topo.connectors {
            let Some(ch) = parse_handle(&conn.handle) else {
                continue;
            };
            if seen_handles.contains(&ch) {
                continue;
            }
            let Some(ent) = doc.get_entity(ch) else {
                continue;
            };
            let (x, y) = entity_xy(ent).unwrap_or((0.0, 0.0));
            let target_raw = conn
                .peer_drawing
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let expanded_targets = target_raw
                .as_deref()
                .map(expand_target_dwg_codes)
                .unwrap_or_default();
            out.push(OpcConnectorItem {
                tab_index,
                drawing_code: drawing_code.to_string(),
                primary_handle: ch,
                member_handles: vec![ch],
                primary_handle_hex: format!("{:X}", ch.value()),
                x,
                y,
                target_dwg_raw: target_raw,
                expanded_targets,
                line_label: if conn.line_tag.trim().is_empty() {
                    None
                } else {
                    Some(conn.line_tag.clone())
                },
                status: OpcStatus::MissingTarget,
                peer: None,
                target_open_tab: None,
            });
        }
        return out;
    }

    for sym in &rec.symbols {
        if sym.sppid_class != "PIDBranchPoint" {
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
        for &mh_raw in sym.member_handles.iter().chain(sym.tag_handles.iter()) {
            let mh = Handle::new(mh_raw);
            if !members.contains(&mh) {
                members.push(mh);
            }
        }
        let target_raw = sym
            .to_from_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let expanded_targets = target_raw
            .as_deref()
            .map(expand_target_dwg_codes)
            .unwrap_or_default();
        let line_label = nearest_rec_line_label(rec, sym.at);
        out.push(OpcConnectorItem {
            tab_index,
            drawing_code: drawing_code.to_string(),
            primary_handle: primary_h,
            member_handles: members,
            primary_handle_hex: format!("{:X}", primary_h.value()),
            x: sym.at.0,
            y: sym.at.1,
            target_dwg_raw: target_raw,
            expanded_targets,
            line_label,
            status: OpcStatus::MissingTarget,
            peer: None,
            target_open_tab: None,
        });
    }
    out
}

/// Analyze and pair off-page connectors across all open drawing sheets (`#149`).
pub fn analyze_open_sheets_opc(sheets: &[OpcSheetInput<'_>]) -> WorkspaceOpcReport {
    let class_map = default_legend_class_map();
    let thresholds = ConfidenceThresholds::default();

    let mut sheet_codes_by_pos: Vec<BTreeSet<String>> = Vec::with_capacity(sheets.len());
    let mut code_to_tabs: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut connectors: Vec<OpcConnectorItem> = Vec::new();
    let mut conn_sheet_pos: Vec<usize> = Vec::new();

    for (pos, sh) in sheets.iter().enumerate() {
        let rec = recognise_sppid_sheet(sh.document, class_map, thresholds);
        let (primary_code, codes) = collect_sheet_codes(sh.fallback_name, &rec);
        for c in &codes {
            code_to_tabs
                .entry(c.clone())
                .or_default()
                .push((pos, sh.tab_index));
        }
        let items = extract_sheet_connectors(sh.tab_index, &primary_code, sh.document, &rec);
        for item in items {
            connectors.push(item);
            conn_sheet_pos.push(pos);
        }
        sheet_codes_by_pos.push(codes);
    }

    let mut cand_pairs: Vec<(u8, usize, usize)> = Vec::new();
    for i in 0..connectors.len() {
        if connectors[i].expanded_targets.is_empty() {
            continue;
        }
        let pos_i = conn_sheet_pos[i];
        let codes_i = &sheet_codes_by_pos[pos_i];
        if codes_i.is_empty() {
            continue;
        }
        for j in (i + 1)..connectors.len() {
            let pos_j = conn_sheet_pos[j];
            if pos_i == pos_j || connectors[j].expanded_targets.is_empty() {
                continue;
            }
            let codes_j = &sheet_codes_by_pos[pos_j];
            if codes_j.is_empty() {
                continue;
            }
            let i_hits_j = connectors[i]
                .expanded_targets
                .iter()
                .any(|t| codes_j.contains(t));
            let j_hits_i = connectors[j]
                .expanded_targets
                .iter()
                .any(|t| codes_i.contains(t));
            if i_hits_j && j_hits_i {
                let rank = match (&connectors[i].line_label, &connectors[j].line_label) {
                    (Some(la), Some(lb)) if !la.is_empty() && la == lb => 0,
                    (None, None) => 1,
                    _ => 2,
                };
                cand_pairs.push((rank, i, j));
            }
        }
    }
    cand_pairs.sort_by_key(|&(rank, i, j)| (rank, i, j));

    let mut paired_mask = vec![false; connectors.len()];
    let mut reciprocal_pairs = 0usize;
    for (_, i, j) in cand_pairs {
        if paired_mask[i] || paired_mask[j] {
            continue;
        }
        paired_mask[i] = true;
        paired_mask[j] = true;
        reciprocal_pairs += 1;

        let peer_for_i = OpcPeerRef {
            tab_index: connectors[j].tab_index,
            drawing_code: connectors[j].drawing_code.clone(),
            primary_handle: connectors[j].primary_handle,
            member_handles: connectors[j].member_handles.clone(),
            primary_handle_hex: connectors[j].primary_handle_hex.clone(),
            x: connectors[j].x,
            y: connectors[j].y,
            line_label: connectors[j].line_label.clone(),
        };
        let peer_for_j = OpcPeerRef {
            tab_index: connectors[i].tab_index,
            drawing_code: connectors[i].drawing_code.clone(),
            primary_handle: connectors[i].primary_handle,
            member_handles: connectors[i].member_handles.clone(),
            primary_handle_hex: connectors[i].primary_handle_hex.clone(),
            x: connectors[i].x,
            y: connectors[i].y,
            line_label: connectors[i].line_label.clone(),
        };
        connectors[i].status = OpcStatus::Paired;
        connectors[i].target_open_tab = Some(connectors[j].tab_index);
        connectors[i].peer = Some(peer_for_i);

        connectors[j].status = OpcStatus::Paired;
        connectors[j].target_open_tab = Some(connectors[i].tab_index);
        connectors[j].peer = Some(peer_for_j);
    }

    let mut one_way_open = 0usize;
    let mut external_unopened = 0usize;
    let mut missing_target = 0usize;
    for (idx, item) in connectors.iter_mut().enumerate() {
        if paired_mask[idx] {
            continue;
        }
        if item.expanded_targets.is_empty() {
            item.status = OpcStatus::MissingTarget;
            missing_target += 1;
            continue;
        }
        let open_hit = item.expanded_targets.iter().find_map(|t| {
            code_to_tabs.get(t).and_then(|owners| {
                owners
                    .iter()
                    .find(|&&(pos, _)| pos != conn_sheet_pos[idx])
                    .map(|&(_, tab_idx)| tab_idx)
            })
        });
        if let Some(other_tab) = open_hit {
            item.status = OpcStatus::OneWayOpen;
            item.target_open_tab = Some(other_tab);
            one_way_open += 1;
        } else {
            item.status = OpcStatus::ExternalUnopened;
            external_unopened += 1;
        }
    }

    WorkspaceOpcReport {
        open_sheets: sheets.len(),
        total_connectors: connectors.len(),
        reciprocal_pairs,
        paired_connectors: reciprocal_pairs * 2,
        one_way_open,
        external_unopened,
        missing_target,
        connectors,
    }
}

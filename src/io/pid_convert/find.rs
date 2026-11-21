//! Cross-tab P&ID tag and full-pipeline search & focus resolver
//! (`PIDFIND`, `L2-2` / `L3-2` / `L3-3` / `L6-2` / `#152`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::external_props::normalize_lookup_key;
use super::opc::OpcSheetInput;
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Kind of matched P&ID target returned by `search_pid_entities`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PidFindKind {
    Symbol,
    Pipeline,
}

/// One ranked search hit across open drawing tabs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidFindHit {
    pub tab_index: usize,
    pub drawing_name: String,
    pub kind: PidFindKind,
    pub matched_tag: String,
    pub sppid_class: String,
    pub match_rank: u8,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
}

type Pt2 = (f64, f64);

#[derive(Clone, Copy)]
struct RawPipeSeg {
    handle: Handle,
    a: Pt2,
    b: Pt2,
}

struct PipelineAcc {
    display_tag: String,
    handles: Vec<Handle>,
    sum_x: f64,
    sum_y: f64,
    pt_count: usize,
}

fn alnum_upper(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

fn raw_match_rank(candidate: &str, query: &str) -> Option<u8> {
    let q_norm = normalize_lookup_key(query);
    if q_norm.is_empty() {
        return None;
    }
    let c_norm = normalize_lookup_key(candidate);
    if c_norm.is_empty() {
        return None;
    }
    if c_norm == q_norm {
        return Some(0);
    }
    let q_alnum = alnum_upper(query);
    let c_alnum = alnum_upper(candidate);
    if !q_alnum.is_empty() && c_alnum == q_alnum {
        return Some(1);
    }
    if c_norm.starts_with(&q_norm) || (!q_alnum.is_empty() && c_alnum.starts_with(&q_alnum)) {
        return Some(2);
    }
    if c_norm.contains(&q_norm)
        || (!q_alnum.is_empty() && q_alnum.len() >= 3 && c_alnum.contains(&q_alnum))
    {
        return Some(3);
    }
    None
}

/// Rank how closely `candidate` matches `query` (`0` = exact, `1` = alnum-exact, `2` = prefix, `3` = substring),
/// including imperial/metric `parse_sppid_line_number` canonical pipeline equivalence.
pub fn match_query_rank(candidate: &str, query: &str) -> Option<u8> {
    let mut best = raw_match_rank(candidate, query);
    let cand_pipe = parse_sppid_line_number(candidate).map(|p| p.pipeline_name());
    let query_pipe = parse_sppid_line_number(query).map(|p| p.pipeline_name());
    if let Some(ref cp) = cand_pipe {
        if let Some(r) = raw_match_rank(cp, query) {
            best = Some(best.map_or(r, |b| b.min(r)));
        }
        if let Some(ref qp) = query_pipe {
            if let Some(r) = raw_match_rank(cp, qp) {
                best = Some(best.map_or(r, |b| b.min(r)));
            }
        }
    } else if let Some(ref qp) = query_pipe {
        if let Some(r) = raw_match_rank(candidate, qp) {
            best = Some(best.map_or(r, |b| b.min(r)));
        }
    }
    best
}

fn parse_csv_handles(raw: &str) -> Vec<Handle> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(parse_handle)
        .collect()
}

fn parse_pair_f64(s: &str) -> Option<Pt2> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn entity_xy(doc: &Document, h: Handle) -> Option<Pt2> {
    match doc.get_entity(h)? {
        EntityType::Insert(ins) => Some((ins.insert_point.x, ins.insert_point.y)),
        EntityType::Circle(c) => Some((c.center.x, c.center.y)),
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

fn point_to_seg_dist(p: Pt2, a: Pt2, b: Pt2) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * vx)).hypot(p.1 - (a.1 + t * vy))
}

fn segs_touch(s1: RawPipeSeg, s2: RawPipeSeg, tol: f64) -> bool {
    point_to_seg_dist(s1.a, s2.a, s2.b) <= tol
        || point_to_seg_dist(s1.b, s2.a, s2.b) <= tol
        || point_to_seg_dist(s2.a, s1.a, s1.b) <= tol
        || point_to_seg_dist(s2.b, s1.a, s1.b) <= tol
}

fn expand_connected_pipe_segs(
    all_segs: &[RawPipeSeg],
    seed_indices: &[usize],
    explicit_other_mask: &[bool],
    tol: f64,
) -> Vec<usize> {
    let mut visited = vec![false; all_segs.len()];
    let mut queue = VecDeque::new();
    for &si in seed_indices {
        if si < all_segs.len() && !visited[si] {
            visited[si] = true;
            queue.push_back(si);
        }
    }
    let mut out = Vec::new();
    while let Some(cur) = queue.pop_front() {
        out.push(cur);
        for (nxt, &cand) in all_segs.iter().enumerate() {
            if visited[nxt] || explicit_other_mask[nxt] {
                continue;
            }
            if segs_touch(all_segs[cur], cand, tol) {
                visited[nxt] = true;
                queue.push_back(nxt);
            }
        }
    }
    out
}

fn collect_authored_hits(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    query: &str,
    hits: &mut Vec<PidFindHit>,
) {
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let conn_tol = 6.5 * upm;

    let mut all_segs: Vec<RawPipeSeg> = Vec::new();
    let mut seg_labels: Vec<Option<String>> = Vec::new();

    for ent in doc.model_space_entities() {
        let h = ent.common().handle;
        let Some(auth) = PidAuthor::load(doc, h) else {
            continue;
        };
        let pmap: serde_json::Map<String, Value> =
            serde_json::from_str(&auth.props).unwrap_or_default();

        if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
            let lbl = pmap
                .get("_line_label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if let EntityType::Line(ln) = ent {
                all_segs.push(RawPipeSeg {
                    handle: h,
                    a: (ln.start.x, ln.start.y),
                    b: (ln.end.x, ln.end.y),
                });
                seg_labels.push(lbl);
            } else if let EntityType::LwPolyline(pl) = ent {
                for w in pl.vertices.windows(2) {
                    all_segs.push(RawPipeSeg {
                        handle: h,
                        a: (w[0].location.x, w[0].location.y),
                        b: (w[1].location.x, w[1].location.y),
                    });
                    seg_labels.push(lbl.clone());
                }
            }
            continue;
        }

        let cand_tag = if !auth.tag.trim().is_empty() {
            auth.tag.trim().to_string()
        } else if let Some(tf) = pmap.get("_to_from_text").and_then(Value::as_str) {
            tf.trim().to_string()
        } else {
            continue;
        };
        let Some(rank) = match_query_rank(&cand_tag, query) else {
            continue;
        };

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
        let center = pmap
            .get("_at")
            .and_then(Value::as_str)
            .and_then(parse_pair_f64)
            .or_else(|| entity_xy(doc, h))
            .unwrap_or((0.0, 0.0));
        let cls = pmap
            .get("_sppid_class")
            .and_then(Value::as_str)
            .unwrap_or(&auth.type_code)
            .to_string();

        hits.push(PidFindHit {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: PidFindKind::Symbol,
            matched_tag: cand_tag,
            sppid_class: cls,
            match_rank: rank,
            primary_handle: h,
            member_handles: members,
            primary_handle_hex: format!("{:X}", h.value()),
            x: center.0,
            y: center.1,
        });
    }

    let topo = TopologyTable::load(doc);
    let run_to_line: BTreeMap<String, String> = topo
        .pipe_runs
        .iter()
        .map(|r| (r.tag.clone(), r.line_tag.clone()))
        .collect();
    for seg in &topo.pipe_segments {
        let Some(sh) = parse_handle(&seg.handle) else {
            continue;
        };
        let lt = run_to_line.get(&seg.run_tag).cloned();
        if let Some(pos) = all_segs.iter().position(|s| s.handle == sh) {
            if seg_labels[pos].is_none() {
                seg_labels[pos] = lt;
            }
        } else {
            all_segs.push(RawPipeSeg {
                handle: sh,
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            });
            seg_labels.push(lt);
        }
    }

    // Also include unlabeled model-space lines so multi-segment runs propagate cleanly
    let known_handles: BTreeSet<Handle> = all_segs.iter().map(|s| s.handle).collect();
    for ent in doc.model_space_entities() {
        let eh = ent.common().handle;
        if known_handles.contains(&eh) || PidAuthor::load(doc, eh).is_some() {
            continue;
        }
        if let EntityType::Line(ln) = ent {
            if (ln.start.x - ln.end.x).hypot(ln.start.y - ln.end.y) >= 2.0 * upm {
                all_segs.push(RawPipeSeg {
                    handle: eh,
                    a: (ln.start.x, ln.start.y),
                    b: (ln.end.x, ln.end.y),
                });
                seg_labels.push(None);
            }
        }
    }

    let mut groups: BTreeMap<String, (String, Vec<usize>)> = BTreeMap::new();
    for (idx, lbl_opt) in seg_labels.iter().enumerate() {
        if let Some(ref lbl) = lbl_opt {
            let nk = normalize_lookup_key(lbl);
            if !nk.is_empty() {
                let entry = groups
                    .entry(nk)
                    .or_insert_with(|| (lbl.trim_end_matches('-').to_string(), Vec::new()));
                entry.1.push(idx);
            }
        }
    }

    for (nk, (disp_tag, seeds)) in groups {
        let Some(rank) = match_query_rank(&disp_tag, query) else {
            continue;
        };
        let other_mask: Vec<bool> = seg_labels
            .iter()
            .map(|lo| {
                lo.as_deref()
                    .map(normalize_lookup_key)
                    .is_some_and(|ok| !ok.is_empty() && ok != nk)
            })
            .collect();
        let expanded_idx = expand_connected_pipe_segs(&all_segs, &seeds, &other_mask, conn_tol);
        let mut handles = Vec::new();
        let mut sx = 0.0;
        let mut sy = 0.0;
        let mut n = 0usize;
        for si in expanded_idx {
            let sg = all_segs[si];
            if !handles.contains(&sg.handle) {
                handles.push(sg.handle);
            }
            sx += (sg.a.0 + sg.b.0) * 0.5;
            sy += (sg.a.1 + sg.b.1) * 0.5;
            n += 1;
        }
        if let Some(&first_h) = handles.first() {
            let cx = if n > 0 { sx / (n as f64) } else { 0.0 };
            let cy = if n > 0 { sy / (n as f64) } else { 0.0 };
            hits.push(PidFindHit {
                tab_index,
                drawing_name: drawing_name.to_string(),
                kind: PidFindKind::Pipeline,
                matched_tag: disp_tag,
                sppid_class: "PIDPipeline".to_string(),
                match_rank: rank,
                primary_handle: first_h,
                member_handles: handles,
                primary_handle_hex: format!("{:X}", first_h.value()),
                x: cx,
                y: cy,
            });
        }
    }
}

fn collect_unpromoted_hits(
    tab_index: usize,
    drawing_name: &str,
    doc: &Document,
    query: &str,
    hits: &mut Vec<PidFindHit>,
) {
    if has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        collect_authored_hits(tab_index, drawing_name, &scratch, query, hits);
        return;
    }

    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let conn_tol = 6.5 * upm;
    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );

    for sym in &rec.symbols {
        let Some(cand_tag) = sym.tag.as_deref().or(sym.to_from_text.as_deref()) else {
            continue;
        };
        let Some(rank) = match_query_rank(cand_tag, query) else {
            continue;
        };
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
        hits.push(PidFindHit {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: PidFindKind::Symbol,
            matched_tag: cand_tag.trim().to_string(),
            sppid_class: sym.sppid_class.clone(),
            match_rank: rank,
            primary_handle: primary_h,
            member_handles: members,
            primary_handle_hex: format!("{:X}", primary_h.value()),
            x: sym.at.0,
            y: sym.at.1,
        });
    }

    let all_segs: Vec<RawPipeSeg> = rec
        .pipe_segments
        .iter()
        .filter_map(|ps| {
            ps.entity_handle.map(|rh| RawPipeSeg {
                handle: Handle::new(rh),
                a: ps.start,
                b: ps.end,
            })
        })
        .collect();

    let mut seg_direct_lbl: Vec<Option<String>> = vec![None; all_segs.len()];
    for (si, sg) in all_segs.iter().enumerate() {
        let mut best: Option<(f64, &str)> = None;
        for lbl in &rec.pipe_labels {
            let d = point_to_seg_dist(lbl.at, sg.a, sg.b);
            if d <= 36.0 * upm && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, lbl.raw_text.as_str()));
            }
        }
        if let Some((_, txt)) = best {
            seg_direct_lbl[si] = Some(txt.trim().to_string());
        }
    }

    let mut pipe_map: BTreeMap<String, PipelineAcc> = BTreeMap::new();
    for lbl in &rec.pipe_labels {
        let nk = normalize_lookup_key(&lbl.raw_text);
        if nk.is_empty() {
            continue;
        }
        let seeds: Vec<usize> = seg_direct_lbl
            .iter()
            .enumerate()
            .filter_map(|(si, lo)| {
                lo.as_deref()
                    .is_some_and(|s| normalize_lookup_key(s) == nk)
                    .then_some(si)
            })
            .collect();
        let other_mask: Vec<bool> = seg_direct_lbl
            .iter()
            .map(|lo| {
                lo.as_deref()
                    .map(normalize_lookup_key)
                    .is_some_and(|ok| !ok.is_empty() && ok != nk)
            })
            .collect();
        let expanded = expand_connected_pipe_segs(&all_segs, &seeds, &other_mask, conn_tol);

        let entry = pipe_map.entry(nk).or_insert_with(|| PipelineAcc {
            display_tag: lbl.raw_text.trim().trim_end_matches('-').to_string(),
            handles: Vec::new(),
            sum_x: 0.0,
            sum_y: 0.0,
            pt_count: 0,
        });
        for si in expanded {
            let sg = all_segs[si];
            if !entry.handles.contains(&sg.handle) {
                entry.handles.push(sg.handle);
                entry.sum_x += (sg.a.0 + sg.b.0) * 0.5;
                entry.sum_y += (sg.a.1 + sg.b.1) * 0.5;
                entry.pt_count += 1;
            }
        }
        if entry.handles.is_empty() {
            if let Some(txt_h) = doc.model_space_entities().find_map(|e| match e {
                EntityType::Text(t) if t.value.trim() == lbl.raw_text.trim() => {
                    Some(t.common.handle)
                }
                EntityType::MText(mt) if mt.value.trim() == lbl.raw_text.trim() => {
                    Some(mt.common.handle)
                }
                _ => None,
            }) {
                entry.handles.push(txt_h);
                entry.sum_x += lbl.at.0;
                entry.sum_y += lbl.at.1;
                entry.pt_count += 1;
            }
        }
    }

    for acc in pipe_map.into_values() {
        let Some(rank) = match_query_rank(&acc.display_tag, query) else {
            continue;
        };
        let Some(&first_h) = acc.handles.first() else {
            continue;
        };
        let cx = if acc.pt_count > 0 {
            acc.sum_x / (acc.pt_count as f64)
        } else {
            0.0
        };
        let cy = if acc.pt_count > 0 {
            acc.sum_y / (acc.pt_count as f64)
        } else {
            0.0
        };
        hits.push(PidFindHit {
            tab_index,
            drawing_name: drawing_name.to_string(),
            kind: PidFindKind::Pipeline,
            matched_tag: acc.display_tag,
            sppid_class: "PIDPipeline".to_string(),
            match_rank: rank,
            primary_handle: first_h,
            member_handles: acc.handles,
            primary_handle_hex: format!("{:X}", first_h.value()),
            x: cx,
            y: cy,
        });
    }
}

/// Search symbols and full pipelines matching `query` across `sheets` (`#152`).
///
/// Results are ordered by `match_rank` (`0..=3`), preferring `preferred_tab` on ties.
pub fn search_pid_entities(
    query: &str,
    preferred_tab: usize,
    sheets: &[OpcSheetInput<'_>],
) -> Vec<PidFindHit> {
    let mut hits = Vec::new();
    for sh in sheets {
        let has_author = sh
            .document
            .model_space_entities()
            .any(|e| PidAuthor::load(sh.document, e.common().handle).is_some());
        if has_author {
            collect_authored_hits(
                sh.tab_index,
                sh.fallback_name,
                sh.document,
                query,
                &mut hits,
            );
        } else {
            collect_unpromoted_hits(
                sh.tab_index,
                sh.fallback_name,
                sh.document,
                query,
                &mut hits,
            );
        }
    }

    hits.sort_by(|a, b| {
        let pref_a = u8::from(a.tab_index != preferred_tab);
        let pref_b = u8::from(b.tab_index != preferred_tab);
        (
            a.match_rank,
            pref_a,
            a.kind,
            &a.matched_tag,
            a.tab_index,
            a.primary_handle.value(),
        )
            .cmp(&(
                b.match_rank,
                pref_b,
                b.kind,
                &b.matched_tag,
                b.tab_index,
                b.primary_handle.value(),
            ))
    });
    hits
}

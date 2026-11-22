//! Multi-sheet P&ID piping materials class (`Spec-Break`), nominal diameter (`Reducer`),
//! and cross-tab `Off-Page Connector` specification consistency auditor
//! (`PIDSPEC`, `L3-3` / `L3-4` / `L3-6` / `L6-2` / `#157`).

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::io::pid_publish::parse_sppid_line_number;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::opc::{analyze_open_sheets_opc, OpcSheetInput, OpcStatus};
use super::promote::{has_native_pid_semantics, promote_semantics_to_author};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds};

/// Canonical 9-column header order shared by CSV and TSV piping spec consistency reports.
pub const SPEC_CSV_HEADERS: [&str; 9] = [
    "FindingKind",
    "DrawingCode",
    "LeftLine",
    "RightLine",
    "LeftSpecDN",
    "RightSpecDN",
    "Message",
    "X",
    "Y",
];

/// Category of a piping specification / diameter consistency finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecFindingKind {
    MissingReducer,
    UnmarkedSpecBreak,
    OpcSpecMismatch,
}

impl SpecFindingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingReducer => "missing_reducer",
            Self::UnmarkedSpecBreak => "unmarked_spec_break",
            Self::OpcSpecMismatch => "opc_spec_mismatch",
        }
    }
}

/// One actionable piping size or material-class mismatch finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidSpecFinding {
    pub kind: SpecFindingKind,
    pub tab_index: usize,
    pub drawing_name: String,
    pub left_line: String,
    pub right_line: String,
    pub left_dn: String,
    pub right_dn: String,
    pub left_spec: String,
    pub right_spec: String,
    pub message: String,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
    #[serde(skip)]
    pub peer_tab_handles: Option<(usize, Vec<Handle>)>,
}

/// Summary counts across a `PIDSPEC` consistency check.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidSpecSummary {
    pub sheets_scanned: usize,
    pub labeled_pipe_segments: usize,
    pub total_findings: usize,
    pub missing_reducers: usize,
    pub unmarked_spec_breaks: usize,
    pub opc_spec_mismatches: usize,
}

/// Complete `PIDSPEC` piping specification consistency report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PidSpecReport {
    pub summary: PidSpecSummary,
    pub findings: Vec<PidSpecFinding>,
}

type Pt2 = (f64, f64);

#[derive(Clone, Copy)]
struct Span2 {
    a: Pt2,
    b: Pt2,
}

#[derive(Clone, Copy)]
struct BBox2 {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

struct SpecPipeSeg {
    handle: Handle,
    span: Span2,
    raw_label: String,
    dn: String,
    spec: String,
}

struct SymBoundary {
    bbox: BBox2,
    can_change_spec: bool,
}

struct SheetPipeSet {
    upm: f64,
    segs: Vec<SpecPipeSeg>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct OpcPairKey {
    a: (usize, u64),
    b: (usize, u64),
}

fn parse_bbox_str(s: &str) -> Option<BBox2> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(BBox2 {
        min_x: parts[0].trim().parse().ok()?,
        min_y: parts[1].trim().parse().ok()?,
        max_x: parts[2].trim().parse().ok()?,
        max_y: parts[3].trim().parse().ok()?,
    })
}

fn point_to_span_dist(p: Pt2, s: Span2) -> f64 {
    let vx = s.b.0 - s.a.0;
    let vy = s.b.1 - s.a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-12 {
        return (p.0 - s.a.0).hypot(p.1 - s.a.1);
    }
    let t = (((p.0 - s.a.0) * vx + (p.1 - s.a.1) * vy) / len2).clamp(0.0, 1.0);
    (p.0 - (s.a.0 + t * vx)).hypot(p.1 - (s.a.1 + t * vy))
}

fn point_in_bbox(p: Pt2, bb: BBox2, margin: f64) -> bool {
    p.0 >= bb.min_x - margin
        && p.0 <= bb.max_x + margin
        && p.1 >= bb.min_y - margin
        && p.1 <= bb.max_y + margin
}

fn is_interior_on_span(p: Pt2, s: Span2, tol: f64) -> bool {
    let vx = s.b.0 - s.a.0;
    let vy = s.b.1 - s.a.1;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-9 {
        return false;
    }
    let t = ((p.0 - s.a.0) * vx + (p.1 - s.a.1) * vy) / len2;
    t > 0.08 && t < 0.92 && point_to_span_dist(p, s) <= tol
}

fn collect_canvas_line_texts(doc: &Document) -> Vec<(Pt2, String)> {
    let mut out = Vec::new();
    for ent in doc.model_space_entities() {
        match ent {
            EntityType::Text(t) => {
                let s = t.value.trim();
                if parse_sppid_line_number(s).is_some() {
                    out.push(((t.insertion_point.x, t.insertion_point.y), s.to_string()));
                }
            }
            EntityType::MText(mt) => {
                let s = mt.value.trim();
                if parse_sppid_line_number(s).is_some() {
                    out.push(((mt.insertion_point.x, mt.insertion_point.y), s.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

fn nearest_canvas_line_label(texts: &[(Pt2, String)], sp: Span2, max_d: f64) -> Option<String> {
    let mut best: Option<(f64, &str)> = None;
    for (pt, val) in texts {
        let d = point_to_span_dist(*pt, sp);
        if d <= max_d && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, val.as_str()));
        }
    }
    best.map(|(_, s)| s.trim_end_matches('-').to_string())
}

fn entity_bbox(doc: &Document, h: Handle, upm: f64) -> Option<BBox2> {
    match doc.get_entity(h)? {
        EntityType::Insert(ins) => {
            let (x, y) = (ins.insert_point.x, ins.insert_point.y);
            let hw = 5.0 * upm * ins.x_scale().abs().max(0.5);
            let hh = 5.0 * upm * ins.y_scale().abs().max(0.5);
            Some(BBox2 {
                min_x: x - hw,
                min_y: y - hh,
                max_x: x + hw,
                max_y: y + hh,
            })
        }
        EntityType::Circle(c) => {
            let (x, y, r) = (c.center.x, c.center.y, c.radius.abs());
            Some(BBox2 {
                min_x: x - r,
                min_y: y - r,
                max_x: x + r,
                max_y: y + r,
            })
        }
        _ => None,
    }
}

fn extract_sheet_pipes_and_boundaries(doc: &Document) -> (f64, Vec<SpecPipeSeg>, Vec<SymBoundary>) {
    let has_author = doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some());
    if !has_author && has_native_pid_semantics(doc) {
        let mut scratch = doc.clone();
        let _ = promote_semantics_to_author(&mut scratch, None);
        return extract_sheet_pipes_and_boundaries(&scratch);
    }

    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let line_texts = collect_canvas_line_texts(doc);
    let mut pipes = Vec::new();
    let mut syms = Vec::new();

    if has_author {
        let mut seen_pipe_handles = BTreeSet::new();
        for ent in doc.model_space_entities() {
            let h = ent.common().handle;
            let Some(auth) = PidAuthor::load(doc, h) else {
                continue;
            };
            let pmap: serde_json::Map<String, Value> =
                serde_json::from_str(&auth.props).unwrap_or_default();

            if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
                seen_pipe_handles.insert(h);
                let fallback_lbl = pmap
                    .get("_line_label")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let mut spans = Vec::new();
                if let EntityType::Line(ln) = ent {
                    spans.push(Span2 {
                        a: (ln.start.x, ln.start.y),
                        b: (ln.end.x, ln.end.y),
                    });
                } else if let EntityType::LwPolyline(pl) = ent {
                    for w in pl.vertices.windows(2) {
                        spans.push(Span2 {
                            a: (w[0].location.x, w[0].location.y),
                            b: (w[1].location.x, w[1].location.y),
                        });
                    }
                }
                for sp in spans {
                    let lbl_opt = nearest_canvas_line_label(&line_texts, sp, 32.0 * upm)
                        .or_else(|| fallback_lbl.clone());
                    let Some(lbl) = lbl_opt else {
                        continue;
                    };
                    let Some(parsed) = parse_sppid_line_number(&lbl) else {
                        continue;
                    };
                    pipes.push(SpecPipeSeg {
                        handle: h,
                        span: sp,
                        raw_label: lbl,
                        dn: parsed.nominal_diameter_attr,
                        spec: parsed.materials_class.to_ascii_uppercase(),
                    });
                }
                continue;
            }

            let blk_up = match ent {
                EntityType::Insert(ins) => ins.block_name.to_ascii_uppercase(),
                _ => pmap
                    .get("_source_block")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_uppercase(),
            };
            let is_reducer_only = blk_up.contains("REDUCER")
                && !blk_up.contains("VALVE")
                && !blk_up.contains("FLANGE");
            if let Some(bb) = pmap
                .get("_bbox")
                .and_then(Value::as_str)
                .and_then(parse_bbox_str)
                .or_else(|| entity_bbox(doc, h, upm))
            {
                syms.push(SymBoundary {
                    bbox: bb,
                    can_change_spec: !is_reducer_only,
                });
            }
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
            if seen_pipe_handles.contains(&sh) {
                continue;
            }
            let sp = Span2 {
                a: (seg.start.point[0], seg.start.point[1]),
                b: (seg.end.point[0], seg.end.point[1]),
            };
            let lbl_opt = nearest_canvas_line_label(&line_texts, sp, 32.0 * upm)
                .or_else(|| run_to_line.get(&seg.run_tag).cloned());
            let Some(lbl) = lbl_opt else {
                continue;
            };
            let Some(parsed) = parse_sppid_line_number(&lbl) else {
                continue;
            };
            pipes.push(SpecPipeSeg {
                handle: sh,
                span: sp,
                raw_label: lbl.trim_end_matches('-').to_string(),
                dn: parsed.nominal_diameter_attr,
                spec: parsed.materials_class.to_ascii_uppercase(),
            });
        }

        return (upm, pipes, syms);
    }

    let rec = recognise_sppid_sheet(
        doc,
        default_legend_class_map(),
        ConfidenceThresholds::default(),
    );
    for sym in &rec.symbols {
        let (x0, y0, x1, y1) = sym.bbox;
        let bb = if (x1 - x0).abs() > 1e-9 || (y1 - y0).abs() > 1e-9 {
            BBox2 {
                min_x: x0.min(x1),
                min_y: y0.min(y1),
                max_x: x0.max(x1),
                max_y: y0.max(y1),
            }
        } else {
            BBox2 {
                min_x: sym.at.0 - 5.0 * upm,
                min_y: sym.at.1 - 5.0 * upm,
                max_x: sym.at.0 + 5.0 * upm,
                max_y: sym.at.1 + 5.0 * upm,
            }
        };
        let blk_up = sym.source_block.to_ascii_uppercase();
        let is_reducer_only =
            blk_up.contains("REDUCER") && !blk_up.contains("VALVE") && !blk_up.contains("FLANGE");
        syms.push(SymBoundary {
            bbox: bb,
            can_change_spec: !is_reducer_only,
        });
    }

    for ps in &rec.pipe_segments {
        let Some(rh) = ps.entity_handle else {
            continue;
        };
        let sp = Span2 {
            a: ps.start,
            b: ps.end,
        };
        let mut best_lbl: Option<(f64, &str)> = None;
        for lbl in &rec.pipe_labels {
            let d = point_to_span_dist(lbl.at, sp);
            if d <= 36.0 * upm && best_lbl.is_none_or(|(bd, _)| d < bd) {
                best_lbl = Some((d, lbl.raw_text.as_str()));
            }
        }
        let Some((_, raw_t)) = best_lbl else {
            continue;
        };
        let clean_lbl = raw_t.trim().trim_end_matches('-').to_string();
        let Some(parsed) = parse_sppid_line_number(&clean_lbl) else {
            continue;
        };
        pipes.push(SpecPipeSeg {
            handle: Handle::new(rh),
            span: sp,
            raw_label: clean_lbl,
            dn: parsed.nominal_diameter_attr,
            spec: parsed.materials_class.to_ascii_uppercase(),
        });
    }

    (upm, pipes, syms)
}

fn closest_junction(s1: Span2, s2: Span2, tol: f64) -> Option<Pt2> {
    let pairs = [(s1.a, s2.a), (s1.a, s2.b), (s1.b, s2.a), (s1.b, s2.b)];
    let mut best: Option<(f64, Pt2)> = None;
    for (p1, p2) in pairs {
        let d = (p1.0 - p2.0).hypot(p1.1 - p2.1);
        if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, ((p1.0 + p2.0) * 0.5, (p1.1 + p2.1) * 0.5)));
        }
    }
    if best.is_some() {
        return best.map(|(_, p)| p);
    }
    if let Some(pt) = [s1.a, s1.b]
        .into_iter()
        .find(|&pt| point_to_span_dist(pt, s2) <= tol)
    {
        return Some(pt);
    }
    [s2.a, s2.b]
        .into_iter()
        .find(|&pt| point_to_span_dist(pt, s1) <= tol)
}

/// Audit piping nominal diameter transitions, material-class breaks, and paired OPC spec
/// consistency across `sheets` (`#157`).
pub fn audit_pid_spec_consistency(sheets: &[OpcSheetInput<'_>]) -> PidSpecReport {
    let mut findings = Vec::new();
    let mut labeled_pipe_segments = 0usize;
    let mut pipes_by_tab: BTreeMap<usize, SheetPipeSet> = BTreeMap::new();

    for sh in sheets {
        let (upm, pipes, syms) = extract_sheet_pipes_and_boundaries(sh.document);
        labeled_pipe_segments += pipes.len();
        let junc_tol = 3.5 * upm;
        let sym_margin = 5.5 * upm;

        let mut seen_red_pairs: BTreeSet<(u64, u64)> = BTreeSet::new();
        let mut seen_sb_pairs: BTreeSet<(u64, u64)> = BTreeSet::new();

        for (i, pa) in pipes.iter().enumerate() {
            for pb in &pipes[(i + 1)..] {
                if pa.handle == pb.handle {
                    continue;
                }
                let Some(jpt) = closest_junction(pa.span, pb.span, junc_tol) else {
                    continue;
                };
                let h_pair = (
                    pa.handle.value().min(pb.handle.value()),
                    pa.handle.value().max(pb.handle.value()),
                );

                // 1. Check MissingReducer (1-to-1 butt joint with DN mismatch and no symbol/tee)
                if !pa.dn.is_empty() && !pb.dn.is_empty() && pa.dn != pb.dn {
                    let mut touching_handles = BTreeSet::new();
                    for pk in &pipes {
                        if point_to_span_dist(jpt, pk.span) <= junc_tol {
                            touching_handles.insert(pk.handle.value());
                        }
                    }
                    let is_tee = touching_handles.len() >= 3
                        || is_interior_on_span(jpt, pa.span, junc_tol)
                        || is_interior_on_span(jpt, pb.span, junc_tol);
                    let has_any_sym = syms
                        .iter()
                        .any(|sb| point_in_bbox(jpt, sb.bbox, sym_margin));

                    if !is_tee && !has_any_sym && seen_red_pairs.insert(h_pair) {
                        findings.push(PidSpecFinding {
                            kind: SpecFindingKind::MissingReducer,
                            tab_index: sh.tab_index,
                            drawing_name: sh.fallback_name.to_string(),
                            left_line: pa.raw_label.clone(),
                            right_line: pb.raw_label.clone(),
                            left_dn: pa.dn.clone(),
                            right_dn: pb.dn.clone(),
                            left_spec: pa.spec.clone(),
                            right_spec: pb.spec.clone(),
                            message: format!(
                                "异径管段直连（{} vs {}）在 ({:.1}, {:.1}) 处缺少大小头变径器",
                                pa.dn, pb.dn, jpt.0, jpt.1
                            ),
                            primary_handle_hex: format!("{:X}", pa.handle.value()),
                            x: jpt.0,
                            y: jpt.1,
                            primary_handle: pa.handle,
                            member_handles: vec![pa.handle, pb.handle],
                            peer_tab_handles: None,
                        });
                    }
                }

                // 2. Check UnmarkedSpecBreak (materials class change with no valve/flange/boundary)
                if !pa.spec.is_empty() && !pb.spec.is_empty() && pa.spec != pb.spec {
                    let has_spec_sym = syms
                        .iter()
                        .any(|sb| sb.can_change_spec && point_in_bbox(jpt, sb.bbox, sym_margin));
                    if !has_spec_sym && seen_sb_pairs.insert(h_pair) {
                        findings.push(PidSpecFinding {
                            kind: SpecFindingKind::UnmarkedSpecBreak,
                            tab_index: sh.tab_index,
                            drawing_name: sh.fallback_name.to_string(),
                            left_line: pa.raw_label.clone(),
                            right_line: pb.raw_label.clone(),
                            left_dn: pa.dn.clone(),
                            right_dn: pb.dn.clone(),
                            left_spec: pa.spec.clone(),
                            right_spec: pb.spec.clone(),
                            message: format!(
                                "不同管道材质等级（{} vs {}）在 ({:.1}, {:.1}) 处无分界阀/法兰直连",
                                pa.spec, pb.spec, jpt.0, jpt.1
                            ),
                            primary_handle_hex: format!("{:X}", pa.handle.value()),
                            x: jpt.0,
                            y: jpt.1,
                            primary_handle: pa.handle,
                            member_handles: vec![pa.handle, pb.handle],
                            peer_tab_handles: None,
                        });
                    }
                }
            }
        }

        pipes_by_tab.insert(sh.tab_index, SheetPipeSet { upm, segs: pipes });
    }

    // 3. Cross-sheet Paired OPC spec/diameter consistency check
    let opc_rep = analyze_open_sheets_opc(sheets);
    let mut seen_opc_pairs: BTreeSet<OpcPairKey> = BTreeSet::new();
    for c in &opc_rep.connectors {
        if c.status != OpcStatus::Paired {
            continue;
        }
        let Some(ref peer) = c.peer else {
            continue;
        };
        let k1 = (c.tab_index, c.primary_handle.value());
        let k2 = (peer.tab_index, peer.primary_handle.value());
        let pair_key = if k1 <= k2 {
            OpcPairKey { a: k1, b: k2 }
        } else {
            OpcPairKey { a: k2, b: k1 }
        };
        if !seen_opc_pairs.insert(pair_key) {
            continue;
        }

        let resolve_opc_line = |tab: usize, pt: Pt2, explicit: Option<&str>| -> Option<String> {
            if let Some(s) = explicit.filter(|s| parse_sppid_line_number(s).is_some()) {
                return Some(s.trim_end_matches('-').to_string());
            }
            let pset = pipes_by_tab.get(&tab)?;
            let mut best: Option<(f64, &str)> = None;
            for sg in &pset.segs {
                let d = point_to_span_dist(pt, sg.span);
                if d <= 40.0 * pset.upm && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, sg.raw_label.as_str()));
                }
            }
            best.map(|(_, s)| s.to_string())
        };

        let Some(lbl_a) = resolve_opc_line(c.tab_index, (c.x, c.y), c.line_label.as_deref()) else {
            continue;
        };
        let Some(lbl_b) =
            resolve_opc_line(peer.tab_index, (peer.x, peer.y), peer.line_label.as_deref())
        else {
            continue;
        };
        let Some(pa) = parse_sppid_line_number(&lbl_a) else {
            continue;
        };
        let Some(pb) = parse_sppid_line_number(&lbl_b) else {
            continue;
        };

        let dn_diff = !pa.nominal_diameter_attr.is_empty()
            && !pb.nominal_diameter_attr.is_empty()
            && pa.nominal_diameter_attr != pb.nominal_diameter_attr;
        let spec_diff = !pa.materials_class.is_empty()
            && !pb.materials_class.is_empty()
            && !pa.materials_class.eq_ignore_ascii_case(&pb.materials_class);

        if dn_diff || spec_diff {
            findings.push(PidSpecFinding {
                kind: SpecFindingKind::OpcSpecMismatch,
                tab_index: c.tab_index,
                drawing_name: c.drawing_code.clone(),
                left_line: lbl_a,
                right_line: lbl_b,
                left_dn: pa.nominal_diameter_attr,
                right_dn: pb.nominal_diameter_attr,
                left_spec: pa.materials_class.to_ascii_uppercase(),
                right_spec: pb.materials_class.to_ascii_uppercase(),
                message: format!(
                    "跨图互逆连接符（{} <-> {}）两端管径或材质等级不一致",
                    c.drawing_code, peer.drawing_code
                ),
                primary_handle_hex: c.primary_handle_hex.clone(),
                x: c.x,
                y: c.y,
                primary_handle: c.primary_handle,
                member_handles: c.member_handles.clone(),
                peer_tab_handles: Some((peer.tab_index, peer.member_handles.clone())),
            });
        }
    }

    findings.sort_by(|a, b| {
        (a.kind, a.tab_index, &a.primary_handle_hex).cmp(&(
            b.kind,
            b.tab_index,
            &b.primary_handle_hex,
        ))
    });

    let missing_reducers = findings
        .iter()
        .filter(|f| f.kind == SpecFindingKind::MissingReducer)
        .count();
    let unmarked_spec_breaks = findings
        .iter()
        .filter(|f| f.kind == SpecFindingKind::UnmarkedSpecBreak)
        .count();
    let opc_spec_mismatches = findings
        .iter()
        .filter(|f| f.kind == SpecFindingKind::OpcSpecMismatch)
        .count();

    PidSpecReport {
        summary: PidSpecSummary {
            sheets_scanned: sheets.len(),
            labeled_pipe_segments,
            total_findings: findings.len(),
            missing_reducers,
            unmarked_spec_breaks,
            opc_spec_mismatches,
        },
        findings,
    }
}

fn escape_cell(val: &str, delim: char) -> String {
    if val.contains(delim) || val.contains('"') || val.contains('\n') || val.contains('\r') {
        format!("\"{}\"", val.replace('"', "\"\""))
    } else {
        val.to_string()
    }
}

/// Format `PidSpecReport` as delimited text (CSV or TSV) with UTF-8 BOM.
pub fn format_pid_spec_delimited(report: &PidSpecReport, delim: char) -> String {
    let mut out = String::from("\u{feff}");
    let hdr: Vec<String> = SPEC_CSV_HEADERS
        .iter()
        .map(|h| escape_cell(h, delim))
        .collect();
    out.push_str(&hdr.join(&delim.to_string()));
    out.push_str("\r\n");

    for f in &report.findings {
        let l_sd = format!("{}/{}", f.left_dn, f.left_spec);
        let r_sd = format!("{}/{}", f.right_dn, f.right_spec);
        let xs = format!("{:.1}", f.x);
        let ys = format!("{:.1}", f.y);
        let cells = [
            f.kind.as_str(),
            f.drawing_name.as_str(),
            f.left_line.as_str(),
            f.right_line.as_str(),
            l_sd.as_str(),
            r_sd.as_str(),
            f.message.as_str(),
            xs.as_str(),
            ys.as_str(),
        ];
        let esc: Vec<String> = cells.iter().map(|c| escape_cell(c, delim)).collect();
        out.push_str(&esc.join(&delim.to_string()));
        out.push_str("\r\n");
    }
    out
}

/// Write `PidSpecReport` to `.json`, `.csv`, or `.tsv` based on `out_path` extension (`#157`).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_pid_spec_file(report: &PidSpecReport, out_path: &Path) -> Result<(), String> {
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
                .map_err(|e| format!("serialize spec JSON: {e}"))?
        ),
        "tsv" | "txt" => format_pid_spec_delimited(report, '\t'),
        "csv" => format_pid_spec_delimited(report, ','),
        other => {
            return Err(format!(
                "unsupported spec report extension '.{other}' (expected .json, .csv, or .tsv)"
            ))
        }
    };
    std::fs::write(out_path, payload).map_err(|e| format!("write {}: {e}", out_path.display()))
}

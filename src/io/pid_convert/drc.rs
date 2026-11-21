//! Pre-delivery P&ID Design-Rule Check (`PIDDRC`, `L3-3` / `L3-5` / `L3-7` / `L6-2` / `#151`).
//!
//! Audits an open drawing sheet (either raw `.dxf`/`.dwg` or promoted `PID_AUTHOR` model)
//! across five actionable engineering checks:
//! 1. `DuplicateTag` (`Error`): conflicting duplicate item tags blocking export;
//! 2. `OneWayOpc` (`Error`): one-way broken off-page connector targeting another open tab;
//! 3. `OrphanNozzle` (`Warning`): `PIDNozzle` not attached to any equipment shell;
//! 4. `OpenPipeDeadEnd` (`Warning`): disconnected open pipe segment endpoint;
//! 5. `UntaggedComponent` (`Warning`): untagged equipment/valve/instrument not yet accepted.

use std::collections::BTreeMap;

use document::entities::EntityType;
use document::{Document, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::pid::persist::PidAuthor;
use crate::pid::topology::{parse_handle, TopologyTable, ROLE_PIPE_SEGMENT, TYPE_PIPE_SEGMENT};

use super::class_map::default_legend_class_map;
use super::external_props::normalize_lookup_key;
use super::opc::{analyze_open_sheets_opc, OpcSheetInput, OpcStatus};
use super::recognize::{recognise_sppid_sheet, ConfidenceThresholds, ConfidenceTier};

/// Severity level of a `PIDDRC` finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrcSeverity {
    Error,
    Warning,
}

/// Specific rule category of a `PIDDRC` finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrcKind {
    DuplicateTag,
    OneWayOpc,
    OrphanNozzle,
    OpenPipeDeadEnd,
    UntaggedComponent,
}

/// One actionable design-rule finding with canvas entity handles and focus coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrcFinding {
    pub severity: DrcSeverity,
    pub kind: DrcKind,
    #[serde(skip)]
    pub primary_handle: Handle,
    #[serde(skip)]
    pub member_handles: Vec<Handle>,
    pub primary_handle_hex: String,
    pub x: f64,
    pub y: f64,
    pub tag_or_label: Option<String>,
    pub message: String,
}

/// Aggregated counts and export-readiness verdict for one sheet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrcSummary {
    pub ready_for_export: bool,
    pub total_findings: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub duplicate_tags: usize,
    pub one_way_opcs: usize,
    pub orphan_nozzles: usize,
    pub open_pipe_dead_ends: usize,
    pub untagged_components: usize,
}

/// Full `PIDDRC` report for an open sheet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SheetDrcReport {
    pub summary: DrcSummary,
    pub findings: Vec<DrcFinding>,
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

fn parse_quad_f64(s: &str) -> Option<(f64, f64, f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    Some((
        parts[0].trim().parse().ok()?,
        parts[1].trim().parse().ok()?,
        parts[2].trim().parse().ok()?,
        parts[3].trim().parse().ok()?,
    ))
}

type Pt2 = (f64, f64);
type BBox = (f64, f64, f64, f64);

struct EntityBounds {
    center: Pt2,
    bbox: BBox,
}

#[derive(Clone, Copy)]
struct SegSpan {
    handle: Handle,
    a: Pt2,
    b: Pt2,
}

struct AuthorSymEntry {
    handle: Handle,
    members: Vec<Handle>,
    auth: PidAuthor,
    pmap: serde_json::Map<String, Value>,
    center: Pt2,
    blk_upper: String,
}

fn entity_bbox_and_center(doc: &Document, h: Handle) -> Option<EntityBounds> {
    let ent = doc.get_entity(h)?;
    match ent {
        EntityType::Insert(ins) => {
            let (x, y) = (ins.insert_point.x, ins.insert_point.y);
            let hw = 8.0 * ins.x_scale().abs().max(0.5);
            let hh = 8.0 * ins.y_scale().abs().max(0.5);
            Some(EntityBounds {
                center: (x, y),
                bbox: (x - hw, y - hh, x + hw, y + hh),
            })
        }
        EntityType::Circle(c) => {
            let (x, y, r) = (c.center.x, c.center.y, c.radius.abs());
            Some(EntityBounds {
                center: (x, y),
                bbox: (x - r, y - r, x + r, y + r),
            })
        }
        EntityType::Line(l) => {
            let min_x = l.start.x.min(l.end.x);
            let max_x = l.start.x.max(l.end.x);
            let min_y = l.start.y.min(l.end.y);
            let max_y = l.start.y.max(l.end.y);
            Some(EntityBounds {
                center: ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
                bbox: (min_x, min_y, max_x, max_y),
            })
        }
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
            Some(EntityBounds {
                center: ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5),
                bbox: (min_x, min_y, max_x, max_y),
            })
        }
        EntityType::Text(t) => {
            let (x, y) = (t.insertion_point.x, t.insertion_point.y);
            Some(EntityBounds {
                center: (x, y),
                bbox: (x - 2.0, y - 2.0, x + 2.0, y + 2.0),
            })
        }
        EntityType::MText(mt) => {
            let (x, y) = (mt.insertion_point.x, mt.insertion_point.y);
            Some(EntityBounds {
                center: (x, y),
                bbox: (x - 2.0, y - 2.0, x + 2.0, y + 2.0),
            })
        }
        _ => None,
    }
}

fn point_in_bbox(p: (f64, f64), bb: (f64, f64, f64, f64), margin: f64) -> bool {
    p.0 >= bb.0 - margin && p.0 <= bb.2 + margin && p.1 >= bb.1 - margin && p.1 <= bb.3 + margin
}

fn is_equipment_class(cls: &str, type_code: &str) -> bool {
    matches!(
        cls,
        "PIDProcessVessel" | "PIDMechanicalEquipment" | "PIDHeatTransferEquip"
    ) || type_code.eq_ignore_ascii_case("equipment")
}

fn wants_component_tag(cls: &str, type_code: &str, blk_upper: &str) -> bool {
    if matches!(
        blk_upper,
        s if s.contains("REDUCER")
            || s.contains("CAP")
            || s.contains("FLANGE")
            || s.contains("SPECTACLE")
            || s.contains("SPACER")
            || s.contains("BLIND")
            || s.contains("TRAP")
            || s.contains("STRAINER")
    ) {
        return false;
    }
    matches!(
        cls,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDHeatTransferEquip"
            | "PIDInstrument"
            | "PIDInlineInstrument"
            | "PIDPipingComponent"
    ) || matches!(
        type_code.to_ascii_lowercase().as_str(),
        "equipment" | "valve" | "instrument"
    )
}

fn collect_open_pipe_dead_ends(
    seg_list: &[SegSpan],
    snap_targets: &[Pt2],
    equip_bboxes: &[BBox],
    snap_tol: f64,
    findings: &mut Vec<DrcFinding>,
) {
    let mut unsnapped: Vec<(usize, Pt2)> = Vec::new();
    for (si, seg) in seg_list.iter().enumerate() {
        for pt in [seg.a, seg.b] {
            let near_sym = snap_targets
                .iter()
                .any(|&sp| (pt.0 - sp.0).hypot(pt.1 - sp.1) <= snap_tol);
            let near_eq = equip_bboxes
                .iter()
                .any(|&bb| point_in_bbox(pt, bb, snap_tol * 1.25));
            if !near_sym && !near_eq {
                unsnapped.push((si, pt));
            }
        }
    }

    let mut reported_seg = vec![false; seg_list.len()];
    for (idx, &(si, pt)) in unsnapped.iter().enumerate() {
        let mut connected_other = false;
        for (jdx, &(sj, qt)) in unsnapped.iter().enumerate() {
            if idx != jdx && si != sj && (pt.0 - qt.0).hypot(pt.1 - qt.1) <= snap_tol {
                connected_other = true;
                break;
            }
        }
        // Also check if endpoint lands on the interior of another pipe segment (T-junction)
        if !connected_other {
            for (sj, oseg) in seg_list.iter().enumerate() {
                if si == sj {
                    continue;
                }
                let sa = oseg.a;
                let sb = oseg.b;
                let vx = sb.0 - sa.0;
                let vy = sb.1 - sa.1;
                let len2 = vx * vx + vy * vy;
                let d = if len2 <= 1e-12 {
                    (pt.0 - sa.0).hypot(pt.1 - sa.1)
                } else {
                    let t = (((pt.0 - sa.0) * vx + (pt.1 - sa.1) * vy) / len2).clamp(0.0, 1.0);
                    (pt.0 - (sa.0 + t * vx)).hypot(pt.1 - (sa.1 + t * vy))
                };
                if d <= snap_tol {
                    connected_other = true;
                    break;
                }
            }
        }
        if !connected_other && !reported_seg[si] {
            reported_seg[si] = true;
            let h = seg_list[si].handle;
            findings.push(DrcFinding {
                severity: DrcSeverity::Warning,
                kind: DrcKind::OpenPipeDeadEnd,
                primary_handle: h,
                member_handles: vec![h],
                primary_handle_hex: format!("{:X}", h.value()),
                x: pt.0,
                y: pt.1,
                tag_or_label: None,
                message: format!(
                    "管段 {:X} 在 ({:.1}, {:.1}) 处存在未接端口/三通/设备的开放断头",
                    h.value(),
                    pt.0,
                    pt.1
                ),
            });
        }
    }
}

/// Run the full five-rule pre-delivery `PIDDRC` check on `active_tab` (`#151`).
pub fn audit_sheet_drc(active_tab: usize, open_sheets: &[OpcSheetInput<'_>]) -> SheetDrcReport {
    let Some(active_in) = open_sheets.iter().find(|s| s.tab_index == active_tab) else {
        return SheetDrcReport {
            summary: DrcSummary {
                ready_for_export: true,
                ..Default::default()
            },
            findings: Vec::new(),
        };
    };
    let doc = active_in.document;
    let upm = crate::io::pid_legend::guess_units_per_mm(doc).max(1e-6);
    let snap_tol = 6.5 * upm;
    let equip_margin = 16.0 * upm;

    let mut findings: Vec<DrcFinding> = Vec::new();

    // 1. Cross-tab one-way broken off-page connectors (`Error`)
    let opc_rep = analyze_open_sheets_opc(open_sheets);
    for c in opc_rep
        .connectors
        .iter()
        .filter(|c| c.tab_index == active_tab)
    {
        if c.status == OpcStatus::OneWayOpen {
            let tgt = c.target_dwg_raw.clone().unwrap_or_default();
            findings.push(DrcFinding {
                severity: DrcSeverity::Error,
                kind: DrcKind::OneWayOpc,
                primary_handle: c.primary_handle,
                member_handles: c.member_handles.clone(),
                primary_handle_hex: c.primary_handle_hex.clone(),
                x: c.x,
                y: c.y,
                tag_or_label: c.target_dwg_raw.clone(),
                message: format!("页间连接符指向已打开图纸 {tgt}，但对端缺少指回本图的互逆连接符"),
            });
        }
    }

    let has_any_author = doc
        .model_space_entities()
        .any(|e| PidAuthor::load(doc, e.common().handle).is_some());

    if has_any_author {
        let mut equip_bboxes: Vec<BBox> = Vec::new();
        let mut snap_targets: Vec<Pt2> = Vec::new();
        let mut seg_list: Vec<SegSpan> = Vec::new();
        let mut sym_records: Vec<AuthorSymEntry> = Vec::new();
        let mut tag_counts: BTreeMap<(String, String), usize> = BTreeMap::new();

        for ent in doc.model_space_entities() {
            let h = ent.common().handle;
            let Some(auth) = PidAuthor::load(doc, h) else {
                continue;
            };
            let pmap: serde_json::Map<String, Value> =
                serde_json::from_str(&auth.props).unwrap_or_default();

            if auth.topology == ROLE_PIPE_SEGMENT || auth.type_code == TYPE_PIPE_SEGMENT {
                if let Some(raw_segs) = pmap.get("_segments").and_then(Value::as_str) {
                    for entry in raw_segs.split(';').filter(|s| !s.trim().is_empty()) {
                        let c_str = entry.split('|').next().unwrap_or(entry);
                        if let Some((ax, ay, bx, by)) = parse_quad_f64(c_str) {
                            seg_list.push(SegSpan {
                                handle: h,
                                a: (ax, ay),
                                b: (bx, by),
                            });
                        }
                    }
                } else if let EntityType::Line(ln) = ent {
                    seg_list.push(SegSpan {
                        handle: h,
                        a: (ln.start.x, ln.start.y),
                        b: (ln.end.x, ln.end.y),
                    });
                } else if let EntityType::LwPolyline(pl) = ent {
                    for w in pl.vertices.windows(2) {
                        seg_list.push(SegSpan {
                            handle: h,
                            a: (w[0].location.x, w[0].location.y),
                            b: (w[1].location.x, w[1].location.y),
                        });
                    }
                }
                continue;
            }

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

            let mut union_bb: Option<BBox> = pmap
                .get("_bbox")
                .and_then(Value::as_str)
                .and_then(parse_quad_f64);
            let mut anchor_pt = pmap
                .get("_at")
                .and_then(Value::as_str)
                .and_then(parse_pair_f64);

            for &mh in &members {
                if let Some(eb) = entity_bbox_and_center(doc, mh) {
                    if anchor_pt.is_none() {
                        anchor_pt = Some(eb.center);
                    }
                    let bb = eb.bbox;
                    union_bb = Some(match union_bb {
                        Some(cur) => (
                            cur.0.min(bb.0),
                            cur.1.min(bb.1),
                            cur.2.max(bb.2),
                            cur.3.max(bb.3),
                        ),
                        None => bb,
                    });
                }
            }
            let center = anchor_pt.unwrap_or((0.0, 0.0));
            let bb = union_bb.unwrap_or((
                center.0 - 4.0 * upm,
                center.1 - 4.0 * upm,
                center.0 + 4.0 * upm,
                center.1 + 4.0 * upm,
            ));

            let cls = pmap
                .get("_sppid_class")
                .and_then(Value::as_str)
                .unwrap_or("");
            if is_equipment_class(cls, &auth.type_code) {
                equip_bboxes.push(bb);
            }
            snap_targets.push(center);
            snap_targets.push(((bb.0 + bb.2) * 0.5, bb.1));
            snap_targets.push(((bb.0 + bb.2) * 0.5, bb.3));
            snap_targets.push((bb.0, (bb.1 + bb.3) * 0.5));
            snap_targets.push((bb.2, (bb.1 + bb.3) * 0.5));

            if !auth.tag.trim().is_empty() {
                let nk = normalize_lookup_key(&auth.tag);
                if !nk.is_empty() {
                    let group = if !cls.is_empty() {
                        cls.to_string()
                    } else {
                        auth.type_code.clone()
                    };
                    *tag_counts.entry((group, nk)).or_insert(0) += 1;
                }
            }

            let blk_name = match ent {
                EntityType::Insert(ins) => ins.block_name.to_ascii_uppercase(),
                _ => pmap
                    .get("_source_block")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_uppercase(),
            };
            sym_records.push(AuthorSymEntry {
                handle: h,
                members,
                auth,
                pmap,
                center,
                blk_upper: blk_name,
            });
        }

        let topo = TopologyTable::load(doc);
        for seg in &topo.pipe_segments {
            if let Some(sh) = parse_handle(&seg.handle) {
                if !seg_list.iter().any(|s| s.handle == sh) {
                    seg_list.push(SegSpan {
                        handle: sh,
                        a: (seg.start.point[0], seg.start.point[1]),
                        b: (seg.end.point[0], seg.end.point[1]),
                    });
                }
            }
        }

        for AuthorSymEntry {
            handle: h,
            members,
            auth,
            pmap,
            center,
            blk_upper,
        } in sym_records
        {
            let cls = pmap
                .get("_sppid_class")
                .and_then(Value::as_str)
                .unwrap_or("");
            let tier = pmap
                .get("_confidence_tier")
                .and_then(Value::as_str)
                .unwrap_or("");
            let nk = normalize_lookup_key(&auth.tag);
            let group = if !cls.is_empty() {
                cls.to_string()
            } else {
                auth.type_code.clone()
            };
            let is_dup = auth.tag_status == "duplicated"
                || (!nk.is_empty() && tag_counts.get(&(group, nk)).copied().unwrap_or(0) > 1);

            if is_dup {
                findings.push(DrcFinding {
                    severity: DrcSeverity::Error,
                    kind: DrcKind::DuplicateTag,
                    primary_handle: h,
                    member_handles: members.clone(),
                    primary_handle_hex: format!("{:X}", h.value()),
                    x: center.0,
                    y: center.1,
                    tag_or_label: Some(auth.tag.clone()),
                    message: format!("重复位号冲突：{}（句柄 {:X}）", auth.tag, h.value()),
                });
            }

            if cls == "PIDNozzle" || auth.type_code.eq_ignore_ascii_case("nozzle") {
                let attached = equip_bboxes
                    .iter()
                    .any(|&ebb| point_in_bbox(center, ebb, equip_margin));
                if !attached {
                    findings.push(DrcFinding {
                        severity: DrcSeverity::Warning,
                        kind: DrcKind::OrphanNozzle,
                        primary_handle: h,
                        member_handles: members.clone(),
                        primary_handle_hex: format!("{:X}", h.value()),
                        x: center.0,
                        y: center.1,
                        tag_or_label: (!auth.tag.is_empty()).then(|| auth.tag.clone()),
                        message: format!(
                            "孤立管口（句柄 {:X}）未挂接到任何容器/设备壳体",
                            h.value()
                        ),
                    });
                }
            } else if auth.tag.trim().is_empty()
                && tier != "confirmed"
                && auth.tag_source != "manual"
                && wants_component_tag(cls, &auth.type_code, &blk_upper)
            {
                findings.push(DrcFinding {
                    severity: DrcSeverity::Warning,
                    kind: DrcKind::UntaggedComponent,
                    primary_handle: h,
                    member_handles: members,
                    primary_handle_hex: format!("{:X}", h.value()),
                    x: center.0,
                    y: center.1,
                    tag_or_label: None,
                    message: format!(
                        "组件缺少位号且尚未人工确认（类={}，句柄 {:X}）",
                        if cls.is_empty() { &auth.type_code } else { cls },
                        h.value()
                    ),
                });
            }
        }

        collect_open_pipe_dead_ends(
            &seg_list,
            &snap_targets,
            &equip_bboxes,
            snap_tol,
            &mut findings,
        );
    } else {
        let rec = recognise_sppid_sheet(
            doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        let equip_bboxes: Vec<(f64, f64, f64, f64)> = rec
            .symbols
            .iter()
            .filter(|s| is_equipment_class(&s.sppid_class, ""))
            .map(|s| s.bbox)
            .collect();

        let mut snap_targets: Vec<(f64, f64)> = Vec::new();
        let mut tag_counts: BTreeMap<(String, String), usize> = BTreeMap::new();
        for s in &rec.symbols {
            snap_targets.push(s.at);
            for &p in &s.ports {
                snap_targets.push(p);
            }
            if let Some(ref t) = s.tag {
                let nk = normalize_lookup_key(t);
                if !nk.is_empty() {
                    *tag_counts.entry((s.sppid_class.clone(), nk)).or_insert(0) += 1;
                }
            }
        }

        for s in &rec.symbols {
            let Some(raw_h) = s
                .entity_handle
                .or_else(|| s.member_handles.first().copied())
            else {
                continue;
            };
            let h = Handle::new(raw_h);
            let mut members = vec![h];
            for &mr in s.member_handles.iter().chain(s.tag_handles.iter()) {
                let mh = Handle::new(mr);
                if !members.contains(&mh) {
                    members.push(mh);
                }
            }

            if let Some(ref t) = s.tag {
                let nk = normalize_lookup_key(t);
                if !nk.is_empty()
                    && tag_counts
                        .get(&(s.sppid_class.clone(), nk))
                        .copied()
                        .unwrap_or(0)
                        > 1
                {
                    findings.push(DrcFinding {
                        severity: DrcSeverity::Error,
                        kind: DrcKind::DuplicateTag,
                        primary_handle: h,
                        member_handles: members.clone(),
                        primary_handle_hex: format!("{:X}", h.value()),
                        x: s.at.0,
                        y: s.at.1,
                        tag_or_label: Some(t.clone()),
                        message: format!("重复位号冲突：{t}（句柄 {:X}）", h.value()),
                    });
                }
            }

            if s.sppid_class == "PIDNozzle" {
                let attached = equip_bboxes
                    .iter()
                    .any(|&ebb| point_in_bbox(s.at, ebb, equip_margin));
                if !attached {
                    findings.push(DrcFinding {
                        severity: DrcSeverity::Warning,
                        kind: DrcKind::OrphanNozzle,
                        primary_handle: h,
                        member_handles: members.clone(),
                        primary_handle_hex: format!("{:X}", h.value()),
                        x: s.at.0,
                        y: s.at.1,
                        tag_or_label: s.tag.clone(),
                        message: format!("孤立管口（句柄 {:X}）未挂接到任何设备壳体", h.value()),
                    });
                }
            } else if s.tag.is_none()
                && s.tier != ConfidenceTier::High
                && wants_component_tag(&s.sppid_class, "", &s.source_block.to_ascii_uppercase())
            {
                findings.push(DrcFinding {
                    severity: DrcSeverity::Warning,
                    kind: DrcKind::UntaggedComponent,
                    primary_handle: h,
                    member_handles: members,
                    primary_handle_hex: format!("{:X}", h.value()),
                    x: s.at.0,
                    y: s.at.1,
                    tag_or_label: None,
                    message: format!("组件缺少位号（类={}，句柄 {:X}）", s.sppid_class, h.value()),
                });
            }
        }

        let seg_list: Vec<SegSpan> = rec
            .pipe_segments
            .iter()
            .filter_map(|ps| {
                ps.entity_handle.map(|rh| SegSpan {
                    handle: Handle::new(rh),
                    a: ps.start,
                    b: ps.end,
                })
            })
            .collect();
        collect_open_pipe_dead_ends(
            &seg_list,
            &snap_targets,
            &equip_bboxes,
            snap_tol,
            &mut findings,
        );
    }

    findings.sort_by(|a, b| {
        (a.severity, a.kind, a.primary_handle.value()).cmp(&(
            b.severity,
            b.kind,
            b.primary_handle.value(),
        ))
    });

    let mut summary = DrcSummary {
        total_findings: findings.len(),
        ..Default::default()
    };
    for f in &findings {
        match f.severity {
            DrcSeverity::Error => summary.error_count += 1,
            DrcSeverity::Warning => summary.warning_count += 1,
        }
        match f.kind {
            DrcKind::DuplicateTag => summary.duplicate_tags += 1,
            DrcKind::OneWayOpc => summary.one_way_opcs += 1,
            DrcKind::OrphanNozzle => summary.orphan_nozzles += 1,
            DrcKind::OpenPipeDeadEnd => summary.open_pipe_dead_ends += 1,
            DrcKind::UntaggedComponent => summary.untagged_components += 1,
        }
    }
    summary.ready_for_export = summary.error_count == 0;

    SheetDrcReport { summary, findings }
}

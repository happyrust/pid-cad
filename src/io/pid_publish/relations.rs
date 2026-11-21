//! Piping network, port, process-point, branch-point, and equipment-nozzle
//! synthesis (`L4-4` / `#126`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::model::{ExportDrawing, ExportItem, ExportRelation};
use super::names::{parse_sppid_line_number, populate_item_tag_attributes};
use super::uid::{branch_point_uid, pipeline_uid, port_uid, process_point_uid, UidAllocator};

/// Symbol connection endpoints available for snapping pipe run ends.
#[derive(Debug, Clone)]
pub struct SymbolEndpointRef {
    pub symbol_uid: String,
    pub sppid_class: String,
    pub bbox: (f64, f64, f64, f64),
    pub ports: Vec<(f64, f64)>,
}

/// One traced pipe run segment to synthesize into SPPID piping objects & relations.
#[derive(Debug, Clone)]
pub struct PipeRunInput {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub line_label: Option<String>,
    pub graphic_oid: Option<u32>,
}

/// Counts of synthesized piping objects and relations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipingSynthesisStats {
    pub pipelines: usize,
    pub piping_connectors: usize,
    pub process_points: usize,
    pub piping_ports: usize,
    pub piping_branch_points: usize,
    pub equipment_nozzle_links: usize,
    pub endpoint_connections: usize,
}

fn can_own_piping_port(sppid_class: &str) -> bool {
    matches!(
        sppid_class,
        "PIDPipingComponent" | "PIDInlineInstrument" | "PIDNozzle" | "PIDProcessEquipment"
    )
}

fn is_equipment_class(sppid_class: &str) -> bool {
    matches!(
        sppid_class,
        "PIDProcessVessel"
            | "PIDMechanicalEquipment"
            | "PIDProcessEquipment"
            | "PIDHeatTransferEquipment"
    )
}

fn bbox_gap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let dx = if a.2 < b.0 {
        b.0 - a.2
    } else if b.2 < a.0 {
        a.0 - b.2
    } else {
        0.0
    };
    let dy = if a.3 < b.1 {
        b.1 - a.3
    } else if b.3 < a.1 {
        a.1 - b.3
    } else {
        0.0
    };
    dx.hypot(dy)
}

#[derive(Debug, Clone)]
enum SnappedTarget {
    BranchSymbol(String),
    PortSymbol { symbol_idx: usize, port_idx: usize },
    TeeCluster(usize),
}

/// Synthesize SPPID piping objects (`PIDPipeline`, `PIDPipingConnector`,
/// `PIDProcessPoint`, `PIDPipingPort`, `PIDPipingBranchPoint`) and relations
/// (`EquipmentComponentComposition`, `PipingConnectors`, `ProcessPointCollection`,
/// `PipingPortComposition`, `PipingEnd1Conn`, `PipingEnd2Conn`) into `dwg`.
pub fn synthesize_piping_network(
    dwg: &mut ExportDrawing,
    alloc: &mut UidAllocator,
    endpoints: &[SymbolEndpointRef],
    runs: &[PipeRunInput],
    snap_tol: f64,
) -> Result<PipingSynthesisStats, String> {
    let tol = snap_tol.max(1e-6);
    let mut stats = PipingSynthesisStats::default();

    // Ensure existing drawing items have their segmented tag attributes populated.
    for item in &mut dwg.items {
        populate_item_tag_attributes(item);
    }

    let existing_uids: BTreeSet<String> = dwg.items.iter().map(|i| i.uid.clone()).collect();
    let mut existing_rels: BTreeSet<(String, String, String)> = dwg
        .relations
        .iter()
        .map(|r| (r.def_uid.clone(), r.uid1.clone(), r.uid2.clone()))
        .collect();

    // 1. Link PIDNozzle -> nearest touching Equipment (`EquipmentComponentComposition`).
    let equip_indices: Vec<usize> = endpoints
        .iter()
        .enumerate()
        .filter(|(_, e)| is_equipment_class(&e.sppid_class))
        .map(|(idx, _)| idx)
        .collect();

    for ep in endpoints {
        if ep.sppid_class != "PIDNozzle" {
            continue;
        }
        let mut best: Option<(f64, &str)> = None;
        for &ei in &equip_indices {
            let gap = bbox_gap(ep.bbox, endpoints[ei].bbox);
            if gap <= tol * 4.0 && best.is_none_or(|(bg, _)| gap < bg) {
                best = Some((gap, endpoints[ei].symbol_uid.as_str()));
            }
        }
        if let Some((_, eq_uid)) = best {
            let key = (
                "EquipmentComponentComposition".to_string(),
                ep.symbol_uid.clone(),
                eq_uid.to_string(),
            );
            if existing_rels.insert(key) {
                dwg.relations.push(ExportRelation {
                    def_uid: "EquipmentComponentComposition".to_string(),
                    uid1: ep.symbol_uid.clone(),
                    uid2: eq_uid.to_string(),
                    order_value: None,
                });
                stats.equipment_nozzle_links += 1;
            }
        }
    }

    // 2. Create `PIDPipeline`, `PIDPipingConnector`, and `PIDProcessPoint` items.
    let mut pipeline_by_name: BTreeMap<String, String> = BTreeMap::new();
    for item in &dwg.items {
        if item.class == "PIDPipeline" {
            if let Some(n) = &item.name {
                pipeline_by_name.insert(n.clone(), item.uid.clone());
            }
        }
    }

    let mut conn_uids: Vec<String> = Vec::with_capacity(runs.len());
    let mut conn_diameters: Vec<Option<String>> = Vec::with_capacity(runs.len());

    for (ri, run) in runs.iter().enumerate() {
        let conn_uid = alloc.alloc_primary_uid(
            None,
            &format!(
                "pipe_conn:{ri}:{:.3},{:.3}->{:.3},{:.3}",
                run.start.0, run.start.1, run.end.0, run.end.1
            ),
        );
        let parsed = run.line_label.as_deref().and_then(parse_sppid_line_number);

        let mut attrs: BTreeMap<(String, String), String> = BTreeMap::new();
        let conn_name = if let Some(ref pl) = parsed {
            let p_name = pl.pipeline_name();
            let p_uid = match pipeline_by_name.get(&p_name) {
                Some(u) => u.clone(),
                None => {
                    let u = pipeline_uid(&dwg.plant, &p_name);
                    if !existing_uids.contains(&u) {
                        dwg.items.push(ExportItem {
                            uid: u.clone(),
                            class: "PIDPipeline".to_string(),
                            name: Some(p_name.clone()),
                            ..Default::default()
                        });
                        stats.pipelines += 1;
                    }
                    pipeline_by_name.insert(p_name.clone(), u.clone());
                    u
                }
            };

            let rel_key = (
                "PipingConnectors".to_string(),
                conn_uid.clone(),
                p_uid.clone(),
            );
            if existing_rels.insert(rel_key) {
                dwg.relations.push(ExportRelation {
                    def_uid: "PipingConnectors".to_string(),
                    uid1: conn_uid.clone(),
                    uid2: p_uid,
                    order_value: None,
                });
            }

            attrs.insert(
                (
                    "IPipingConnector".to_string(),
                    "PipingConnectorType".to_string(),
                ),
                "@EE690".to_string(),
            );
            attrs.insert(
                (
                    "IPipeCrossSectionItem".to_string(),
                    "NominalDiameter".to_string(),
                ),
                pl.nominal_diameter_attr.clone(),
            );
            attrs.insert(
                (
                    "IPipingSpecifiedItem".to_string(),
                    "PipingMaterialsClass".to_string(),
                ),
                pl.materials_class.clone(),
            );
            attrs.insert(
                (
                    "INamedPipingConnector".to_string(),
                    "PipingConnectorSeqNo".to_string(),
                ),
                pl.seq_no.clone(),
            );
            if let Some(ref ins) = pl.insulation {
                attrs.insert(
                    ("IInsulatedItem".to_string(), "InsulSpec".to_string()),
                    ins.clone(),
                );
            }
            conn_diameters.push(Some(pl.nominal_diameter_attr.clone()));
            Some(pl.connector_name())
        } else {
            conn_diameters.push(None);
            None
        };

        let graphic_oids = run.graphic_oid.into_iter().collect();
        dwg.items.push(ExportItem {
            uid: conn_uid.clone(),
            class: "PIDPipingConnector".to_string(),
            name: conn_name,
            graphic_oids,
            attributes: attrs,
            ..Default::default()
        });
        stats.piping_connectors += 1;

        let ppt_uid = process_point_uid(&conn_uid)?;
        dwg.items.push(ExportItem {
            uid: ppt_uid.clone(),
            class: "PIDProcessPoint".to_string(),
            ..Default::default()
        });
        stats.process_points += 1;

        dwg.relations.push(ExportRelation {
            def_uid: "ProcessPointCollection".to_string(),
            uid1: ppt_uid,
            uid2: conn_uid.clone(),
            order_value: None,
        });

        conn_uids.push(conn_uid);
    }

    // 3. Snap run endpoints to symbol ports (`PIDPipingPort` / `PIDBranchPoint`) or tee clusters (`PIDPipingBranchPoint`).
    let snap_symbol = |pt: (f64, f64)| -> Option<SnappedTarget> {
        let mut best_branch: Option<(f64, &str)> = None;
        let mut best_port: Option<(f64, usize, usize)> = None;
        for (si, ep) in endpoints.iter().enumerate() {
            if ep.sppid_class == "PIDBranchPoint" {
                for &sp in &ep.ports {
                    let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                    if d <= tol && best_branch.is_none_or(|(bd, _)| d < bd) {
                        best_branch = Some((d, ep.symbol_uid.as_str()));
                    }
                }
            } else if can_own_piping_port(&ep.sppid_class) {
                for (pi, &sp) in ep.ports.iter().enumerate() {
                    let d = (pt.0 - sp.0).hypot(pt.1 - sp.1);
                    if d <= tol && best_port.is_none_or(|(bd, _, _)| d < bd) {
                        best_port = Some((d, si, pi));
                    }
                }
            }
        }
        match (best_branch, best_port) {
            (Some((db, buid)), Some((dp, si, pi))) => {
                if db <= dp {
                    Some(SnappedTarget::BranchSymbol(buid.to_string()))
                } else {
                    Some(SnappedTarget::PortSymbol {
                        symbol_idx: si,
                        port_idx: pi,
                    })
                }
            }
            (Some((_, buid)), None) => Some(SnappedTarget::BranchSymbol(buid.to_string())),
            (None, Some((_, si, pi))) => Some(SnappedTarget::PortSymbol {
                symbol_idx: si,
                port_idx: pi,
            }),
            (None, None) => None,
        }
    };

    let mut end1_targets: Vec<Option<SnappedTarget>> =
        runs.iter().map(|r| snap_symbol(r.start)).collect();
    let mut end2_targets: Vec<Option<SnappedTarget>> =
        runs.iter().map(|r| snap_symbol(r.end)).collect();

    // Cluster unsnapped run endpoints to detect 3-way (or more) pipe junctions (`PIDPipingBranchPoint`).
    #[derive(Clone, Copy)]
    struct FreeEnd {
        run_idx: usize,
        is_end2: bool,
        pt: (f64, f64),
    }

    let mut free_ends: Vec<FreeEnd> = Vec::new();
    for (ri, run) in runs.iter().enumerate() {
        if end1_targets[ri].is_none() {
            free_ends.push(FreeEnd {
                run_idx: ri,
                is_end2: false,
                pt: run.start,
            });
        }
        if end2_targets[ri].is_none() {
            free_ends.push(FreeEnd {
                run_idx: ri,
                is_end2: true,
                pt: run.end,
            });
        }
    }

    let mut visited = vec![false; free_ends.len()];
    let mut tee_uids: Vec<String> = Vec::new();
    let mut bpt_owner_used: BTreeSet<usize> = BTreeSet::new();

    for i in 0..free_ends.len() {
        if visited[i] {
            continue;
        }
        let anchor = free_ends[i].pt;
        let mut members = vec![i];
        visited[i] = true;
        for j in (i + 1)..free_ends.len() {
            if !visited[j]
                && (free_ends[j].pt.0 - anchor.0).hypot(free_ends[j].pt.1 - anchor.1) <= tol
            {
                visited[j] = true;
                members.push(j);
            }
        }
        // Distinct runs meeting at this junction
        let distinct_runs: BTreeSet<usize> =
            members.iter().map(|&m| free_ends[m].run_idx).collect();
        if distinct_runs.len() >= 3 {
            let owner_ri = distinct_runs
                .iter()
                .copied()
                .find(|ri| !bpt_owner_used.contains(ri));
            let bpt_uid = if let Some(ri) = owner_ri {
                bpt_owner_used.insert(ri);
                branch_point_uid(&conn_uids[ri])?
            } else {
                let aux = alloc
                    .alloc_primary_uid(None, &format!("tee_owner:{:.3},{:.3}", anchor.0, anchor.1));
                branch_point_uid(&aux)?
            };

            dwg.items.push(ExportItem {
                uid: bpt_uid.clone(),
                class: "PIDPipingBranchPoint".to_string(),
                ..Default::default()
            });
            stats.piping_branch_points += 1;

            let cluster_id = tee_uids.len();
            tee_uids.push(bpt_uid);
            for &m in &members {
                let fe = free_ends[m];
                if fe.is_end2 {
                    end2_targets[fe.run_idx] = Some(SnappedTarget::TeeCluster(cluster_id));
                } else {
                    end1_targets[fe.run_idx] = Some(SnappedTarget::TeeCluster(cluster_id));
                }
            }
        }
    }

    // Materialize `PIDPipingPort` items on demand (1-indexed port numbers per symbol).
    let mut created_ports: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let mut next_port_num: BTreeMap<usize, u32> = BTreeMap::new();

    let mut resolve_target_uid = |target: &SnappedTarget,
                                  diam: Option<&str>,
                                  dwg: &mut ExportDrawing,
                                  stats: &mut PipingSynthesisStats|
     -> Result<String, String> {
        match target {
            SnappedTarget::BranchSymbol(uid) => Ok(uid.clone()),
            SnappedTarget::TeeCluster(cid) => Ok(tee_uids[*cid].clone()),
            SnappedTarget::PortSymbol {
                symbol_idx,
                port_idx,
            } => {
                if let Some(existing) = created_ports.get(&(*symbol_idx, *port_idx)) {
                    return Ok(existing.clone());
                }
                let pnum = {
                    let slot = next_port_num.entry(*symbol_idx).or_insert(1);
                    let current = *slot;
                    *slot = slot.saturating_add(1);
                    current
                };
                let owner_uid = &endpoints[*symbol_idx].symbol_uid;
                let p_uid = port_uid(owner_uid, pnum)?;
                let mut p_attrs = BTreeMap::new();
                if let Some(nd) = diam {
                    p_attrs.insert(
                        (
                            "IPipeCrossSectionItem".to_string(),
                            "NominalDiameter".to_string(),
                        ),
                        nd.to_string(),
                    );
                }
                dwg.items.push(ExportItem {
                    uid: p_uid.clone(),
                    class: "PIDPipingPort".to_string(),
                    name: Some(pnum.to_string()),
                    attributes: p_attrs,
                    ..Default::default()
                });
                dwg.relations.push(ExportRelation {
                    def_uid: "PipingPortComposition".to_string(),
                    uid1: p_uid.clone(),
                    uid2: owner_uid.clone(),
                    order_value: None,
                });
                stats.piping_ports += 1;
                created_ports.insert((*symbol_idx, *port_idx), p_uid.clone());
                Ok(p_uid)
            }
        }
    };

    for ri in 0..runs.len() {
        let diam = conn_diameters[ri].as_deref();
        if let Some(ref t1) = end1_targets[ri] {
            let u1 = resolve_target_uid(t1, diam, dwg, &mut stats)?;
            let key = (
                "PipingEnd1Conn".to_string(),
                u1.clone(),
                conn_uids[ri].clone(),
            );
            if existing_rels.insert(key) {
                dwg.relations.push(ExportRelation {
                    def_uid: "PipingEnd1Conn".to_string(),
                    uid1: u1,
                    uid2: conn_uids[ri].clone(),
                    order_value: None,
                });
                stats.endpoint_connections += 1;
            }
        }
        if let Some(ref t2) = end2_targets[ri] {
            let u1 = resolve_target_uid(t2, diam, dwg, &mut stats)?;
            let key = (
                "PipingEnd2Conn".to_string(),
                u1.clone(),
                conn_uids[ri].clone(),
            );
            if existing_rels.insert(key) {
                dwg.relations.push(ExportRelation {
                    def_uid: "PipingEnd2Conn".to_string(),
                    uid1: u1,
                    uid2: conn_uids[ri].clone(),
                    order_value: None,
                });
                stats.endpoint_connections += 1;
            }
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_publish::export_drawing_bundle;
    use crate::io::pid_publish_check::{
        check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
    };

    #[test]
    fn synthesizes_pipelines_connectors_ports_tees_nozzles_and_passes_strict_validator() {
        let mut alloc = UidAllocator::new("P01", "DWG-REL-TEST");
        let doc_uid = alloc.alloc_primary_uid(None, "drawing:DWG-REL-TEST");
        let mut dwg = ExportDrawing::new("P01", &doc_uid, "DWG-REL-TEST");

        let vessel_uid = alloc.alloc_primary_uid(None, "eq:V-02019");
        let nozzle_uid = alloc.alloc_primary_uid(None, "noz:N1");
        let valve_uid = alloc.alloc_primary_uid(None, "val:BV0201901");
        let opc_uid = alloc.alloc_primary_uid(None, "opc:1");

        dwg.items.push(ExportItem {
            uid: vessel_uid.clone(),
            class: "PIDProcessVessel".to_string(),
            name: Some("V-02019".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        });
        dwg.items.push(ExportItem {
            uid: nozzle_uid.clone(),
            class: "PIDNozzle".to_string(),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        });
        dwg.items.push(ExportItem {
            uid: valve_uid.clone(),
            class: "PIDPipingComponent".to_string(),
            name: Some("BV0201901".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        });
        dwg.items.push(ExportItem {
            uid: opc_uid.clone(),
            class: "PIDBranchPoint".to_string(),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        });

        let endpoints = vec![
            SymbolEndpointRef {
                symbol_uid: vessel_uid,
                sppid_class: "PIDProcessVessel".to_string(),
                bbox: (0.0, 90.0, 20.0, 110.0),
                ports: vec![(20.0, 100.0)],
            },
            SymbolEndpointRef {
                symbol_uid: nozzle_uid,
                sppid_class: "PIDNozzle".to_string(),
                bbox: (19.0, 99.0, 23.0, 101.0),
                ports: vec![(23.0, 100.0)],
            },
            SymbolEndpointRef {
                symbol_uid: valve_uid,
                sppid_class: "PIDPipingComponent".to_string(),
                bbox: (48.0, 98.0, 52.0, 102.0),
                ports: vec![(48.0, 100.0), (52.0, 100.0)],
            },
            SymbolEndpointRef {
                symbol_uid: opc_uid,
                sppid_class: "PIDBranchPoint".to_string(),
                bbox: (99.0, 98.0, 105.0, 102.0),
                ports: vec![(100.0, 100.0)],
            },
        ];

        let runs = vec![
            PipeRunInput {
                start: (23.0, 100.0),
                end: (48.0, 100.0),
                line_label: Some("25-CI-0201505-B9-EH40".to_string()),
                graphic_oid: Some(alloc.alloc_graphic_oid(None)),
            },
            PipeRunInput {
                start: (52.0, 100.0),
                end: (75.0, 100.0),
                line_label: Some("25-CI-0201505-B9-EH40".to_string()),
                graphic_oid: Some(alloc.alloc_graphic_oid(None)),
            },
            PipeRunInput {
                start: (75.0, 100.0),
                end: (100.0, 100.0),
                line_label: Some("25-CI-0201505-B9-EH40".to_string()),
                graphic_oid: Some(alloc.alloc_graphic_oid(None)),
            },
            PipeRunInput {
                start: (75.0, 100.0),
                end: (75.0, 140.0),
                line_label: Some("15-CI-0201506-B9-".to_string()),
                graphic_oid: Some(alloc.alloc_graphic_oid(None)),
            },
        ];

        let stats =
            synthesize_piping_network(&mut dwg, &mut alloc, &endpoints, &runs, 1.5).unwrap();
        assert_eq!(stats.pipelines, 2);
        assert_eq!(stats.piping_connectors, 4);
        assert_eq!(stats.process_points, 4);
        assert_eq!(stats.piping_ports, 3);
        assert_eq!(stats.piping_branch_points, 1);
        assert_eq!(stats.equipment_nozzle_links, 1);
        assert!(stats.endpoint_connections >= 6);

        let bundle = export_drawing_bundle(&dwg).unwrap();
        let spec = default_publish_spec();
        let opts = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..Default::default()
        };
        let vd = check_publish_xml_bytes(
            "synth_Data.xml",
            PublishXmlKind::Data,
            &bundle.data_xml,
            spec,
            &opts,
        );
        let vm = check_publish_xml_bytes(
            "synth_Meta.xml",
            PublishXmlKind::Meta,
            &bundle.meta_xml,
            spec,
            &opts,
        );
        assert!(vd.is_empty(), "Data violations: {vd:?}");
        assert!(vm.is_empty(), "Meta violations: {vm:?}");
    }
}

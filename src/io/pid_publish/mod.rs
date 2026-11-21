//! Standalone SmartPlant P&ID publish XML exporter from the unified P&ID
//! export model (`L4-1` / `L4-2` / `L4-3` / `#123`).

pub mod classes;
pub mod model;
pub mod names;
pub mod relations;
pub mod uid;
pub mod write;

pub use classes::{default_class_map, materialize_export_item, resolve_variant};
pub use model::{ExportBundle, ExportDrawing, ExportItem, ExportRelation, UnmappedItem};
pub use names::{
    format_nominal_diameter_attr, format_nominal_diameter_from_code,
    format_nominal_diameter_in_name, parse_sppid_line_number, populate_item_tag_attributes,
    ParsedLineNumber,
};
pub use relations::{
    synthesize_piping_network, PipeRunInput, PipingSynthesisStats, SymbolEndpointRef,
};
pub use uid::{
    branch_point_uid, derive_meta_envelope_uids, is_valid_hex32, pipeline_uid, port_uid,
    process_point_uid, relation_uid, MetaEnvelopeUids, UidAllocator,
};
pub use write::export_drawing_bundle;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_publish_check::{
        check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
    };

    #[test]
    fn uid_allocator_and_derived_uids_follow_sppid_grammar() {
        let mut alloc = UidAllocator::new("P01", "DWG-SYNTH-01");
        let u1 = alloc.alloc_primary_uid(None, "eq:V-101");
        let u2 = alloc.alloc_primary_uid(None, "eq:V-102");
        assert!(is_valid_hex32(&u1));
        assert!(is_valid_hex32(&u2));
        assert_ne!(u1, u2);

        // Same key collision automatically salts to a distinct UID
        let u1_dup = alloc.alloc_primary_uid(None, "eq:V-101");
        assert!(is_valid_hex32(&u1_dup));
        assert_ne!(u1, u1_dup);

        assert_eq!(port_uid(&u1, 1).unwrap(), format!("{u1}.1"));
        assert!(port_uid(&u1, 0).is_err());
        assert_eq!(branch_point_uid(&u1).unwrap(), format!("{u1}.BPT"));
        assert_eq!(process_point_uid(&u1).unwrap(), format!("{u1}.PPT"));

        let puid = pipeline_uid("P01", "50 mm-PG-1001-B2");
        assert!(is_valid_hex32(&puid));
        assert_eq!(puid, pipeline_uid("P01", "50 mm-PG-1001-B2"));

        assert_eq!(
            relation_uid("PipingConnectors", &u1, &puid).unwrap(),
            format!("PCN-{u1}-{puid}")
        );

        let oid1 = alloc.alloc_graphic_oid(None);
        let oid2 = alloc.alloc_graphic_oid(Some(oid1));
        assert_ne!(oid1, oid2);
    }

    #[test]
    fn exported_complex_drawing_passes_strict_vendor_validator_zero_violations() {
        let mut alloc = UidAllocator::new("P01", "SYNTH-DWG-100");
        let doc_uid = alloc.alloc_primary_uid(None, "drawing");
        let mut dwg = ExportDrawing::new("P01", &doc_uid, "SYNTH-DWG-100");
        dwg.doc_title = "Synthetic Process Flow & Control".to_string();

        let vessel_uid = alloc.alloc_primary_uid(None, "vessel:V-101");
        let nozzle_uid = alloc.alloc_primary_uid(None, "nozzle:N1");
        let nozzle_port = port_uid(&nozzle_uid, 1).unwrap();
        let valve_uid = alloc.alloc_primary_uid(None, "valve:BV-101");
        let valve_p1 = port_uid(&valve_uid, 1).unwrap();
        let valve_p2 = port_uid(&valve_uid, 2).unwrap();
        let conn_uid = alloc.alloc_primary_uid(None, "conn:1");
        let ppt_uid = process_point_uid(&conn_uid).unwrap();
        let pipe_uid = pipeline_uid("P01", "50 mm-PG-1001-B2");
        let instr_uid = alloc.alloc_primary_uid(None, "instr:PT-201");
        let sig_port = port_uid(&instr_uid, 1).unwrap();
        let opc_uid = alloc.alloc_primary_uid(None, "opc:1");
        let note_uid = alloc.alloc_primary_uid(None, "note:1");

        let mut vessel = ExportItem {
            uid: vessel_uid.clone(),
            class: "PIDProcessVessel".to_string(),
            name: Some("V-101".to_string()),
            description: Some("Flash Drum".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };
        vessel.attributes.insert(
            ("INamedEquipment".to_string(), "EquipPrefix".to_string()),
            "V".to_string(),
        );
        vessel.attributes.insert(
            ("INamedEquipment".to_string(), "EquipSeqNo".to_string()),
            "101".to_string(),
        );

        let nozzle = ExportItem {
            uid: nozzle_uid.clone(),
            class: "PIDNozzle".to_string(),
            name: Some("N1".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };

        let n_port = ExportItem {
            uid: nozzle_port.clone(),
            class: "PIDPipingPort".to_string(),
            name: Some("1".to_string()),
            ..Default::default()
        };

        let valve = ExportItem {
            uid: valve_uid.clone(),
            class: "PIDPipingComponent".to_string(),
            name: Some("BV-101".to_string()),
            symbol_path: Some(r"Piping\Valves\2 Way Common\Ball Valve Type 2.sym".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };

        let vp1 = ExportItem {
            uid: valve_p1.clone(),
            class: "PIDPipingPort".to_string(),
            name: Some("1".to_string()),
            ..Default::default()
        };
        let vp2 = ExportItem {
            uid: valve_p2.clone(),
            class: "PIDPipingPort".to_string(),
            name: Some("2".to_string()),
            ..Default::default()
        };

        let mut conn = ExportItem {
            uid: conn_uid.clone(),
            class: "PIDPipingConnector".to_string(),
            name: Some(String::new()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None), alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };
        conn.attributes.insert(
            (
                "IPipingConnector".to_string(),
                "PipingConnectorSeqNo".to_string(),
            ),
            "1001".to_string(),
        );

        let ppt = ExportItem {
            uid: ppt_uid.clone(),
            class: "PIDProcessPoint".to_string(),
            ..Default::default()
        };

        let pipeline = ExportItem {
            uid: pipe_uid.clone(),
            class: "PIDPipeline".to_string(),
            name: Some("50 mm-PG-1001-B2".to_string()),
            ..Default::default()
        };

        let mut instr = ExportItem {
            uid: instr_uid.clone(),
            class: "PIDInstrument".to_string(),
            name: Some("PT-201".to_string()),
            symbol_path: Some(
                r"Instrumentation\Off-Line\Without Implied Components\Pressure\PT-Pressure Transmitter.sym".to_string(),
            ),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };
        instr.attributes.insert(
            (
                "INamedInstrument".to_string(),
                "MeasuredVariable".to_string(),
            ),
            "PT".to_string(),
        );
        instr.attributes.insert(
            (
                "INamedInstrument".to_string(),
                "InstrTagSequenceNo".to_string(),
            ),
            "201".to_string(),
        );

        let sport = ExportItem {
            uid: sig_port.clone(),
            class: "PIDSignalPort".to_string(),
            name: Some("1".to_string()),
            ..Default::default()
        };

        let mut opc = ExportItem {
            uid: opc_uid.clone(),
            class: "PIDBranchPoint".to_string(),
            name: Some("OPC-01".to_string()),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };
        opc.attributes.insert(
            ("IOffpageConnector".to_string(), "ToFromText".to_string()),
            "TO DWG-101".to_string(),
        );

        let note = ExportItem {
            uid: note_uid.clone(),
            class: "PIDNote".to_string(),
            graphic_oids: vec![alloc.alloc_graphic_oid(None)],
            ..Default::default()
        };

        dwg.items = vec![
            vessel, nozzle, n_port, valve, vp1, vp2, conn, ppt, pipeline, instr, sport, opc, note,
        ];
        dwg.relations = vec![
            ExportRelation {
                def_uid: "EquipmentComponentComposition".to_string(),
                uid1: nozzle_uid.clone(),
                uid2: vessel_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingPortComposition".to_string(),
                uid1: nozzle_port.clone(),
                uid2: nozzle_uid,
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingPortComposition".to_string(),
                uid1: valve_p1.clone(),
                uid2: valve_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingPortComposition".to_string(),
                uid1: valve_p2.clone(),
                uid2: valve_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingEnd1Conn".to_string(),
                uid1: nozzle_port,
                uid2: conn_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingTapOrFitting".to_string(),
                uid1: valve_p1,
                uid2: conn_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingTapOrFitting".to_string(),
                uid1: valve_p2,
                uid2: conn_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "ProcessPointCollection".to_string(),
                uid1: ppt_uid,
                uid2: conn_uid.clone(),
                order_value: None,
            },
            ExportRelation {
                def_uid: "PipingConnectors".to_string(),
                uid1: conn_uid,
                uid2: pipe_uid,
                order_value: None,
            },
            ExportRelation {
                def_uid: "SignalPortComposition".to_string(),
                uid1: sig_port,
                uid2: instr_uid,
                order_value: None,
            },
            ExportRelation {
                def_uid: "PBSItemNotes".to_string(),
                uid1: note_uid,
                uid2: vessel_uid,
                order_value: None,
            },
        ];
        dwg.unmapped.push(UnmappedItem {
            source_id: "ent#42".to_string(),
            raw_class: "UNKNOWN_CUSTOM".to_string(),
            tag: Some("X-999".to_string()),
            reason: "no class-map entry".to_string(),
        });

        let bundle1 = export_drawing_bundle(&dwg).unwrap();
        let bundle2 = export_drawing_bundle(&dwg).unwrap();
        assert_eq!(bundle1, bundle2, "export must be byte-deterministic");

        let spec = default_publish_spec();
        let opts = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..Default::default()
        };

        let v_data = check_publish_xml_bytes(
            "SYNTH_Data.xml",
            PublishXmlKind::Data,
            &bundle1.data_xml,
            spec,
            &opts,
        );
        assert!(v_data.is_empty(), "Data.xml strict violations: {v_data:?}");

        let v_meta = check_publish_xml_bytes(
            "SYNTH_Meta.xml",
            PublishXmlKind::Meta,
            &bundle1.meta_xml,
            spec,
            &opts,
        );
        assert!(v_meta.is_empty(), "Meta.xml strict violations: {v_meta:?}");

        let cov: serde_json::Value = serde_json::from_str(&bundle1.coverage_json).unwrap();
        assert_eq!(cov["unmapped_count"], 1);
        assert_eq!(cov["items_emitted"], 13);
    }

    #[test]
    fn rejects_unknown_class_or_dangling_relation_endpoint() {
        let mut alloc = UidAllocator::new("P01", "BAD-DWG");
        let doc_uid = alloc.alloc_primary_uid(None, "doc");
        let mut dwg = ExportDrawing::new("P01", &doc_uid, "BAD-DWG");
        dwg.items.push(ExportItem {
            uid: alloc.alloc_primary_uid(None, "x"),
            class: "NotARealSppidClass".to_string(),
            ..Default::default()
        });
        assert!(export_drawing_bundle(&dwg).is_err());

        dwg.items.clear();
        dwg.relations.push(ExportRelation {
            def_uid: "PipingConnectors".to_string(),
            uid1: alloc.alloc_primary_uid(None, "dangling1"),
            uid2: alloc.alloc_primary_uid(None, "dangling2"),
            order_value: None,
        });
        assert!(export_drawing_bundle(&dwg).is_err());
    }

    #[test]
    #[ignore = "loads a real SPPID _Data.xml into ExportDrawing, re-exports, and verifies 0 strict violations"]
    fn real_sample_roundtrip_passes_strict_validator() {
        let Ok(sample_xml_path) = std::env::var("OCS_PID_PUBLISH_SAMPLE_XML") else {
            println!("未设置语料样本（OCS_PID_PUBLISH_SAMPLE_XML），跳过真实单图导出验证。");
            return;
        };
        let out_dir = std::env::var("OCS_PID_PUBLISH_OUT_DIR").unwrap_or_default();
        let raw = std::fs::read(&sample_xml_path).unwrap();
        let body = if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
            &raw[3..]
        } else {
            &raw[..]
        };
        let text = std::str::from_utf8(body).unwrap();
        let parse_str = if let Some(rest) = text.strip_prefix("<?xml version =") {
            format!("<?xml  version={rest}")
        } else {
            text.to_string()
        };
        let doc = roxmltree::Document::parse(&parse_str).unwrap();
        let root = doc.root_element();
        let plant = root.attribute("Plant").unwrap_or("P01");
        let doc_uid = root.attribute("DocUID").unwrap();
        let doc_name = root.attribute("DocName").unwrap_or("SAMPLE");

        let mut dwg = ExportDrawing::new(plant, doc_uid, doc_name);
        let mut rep_oids: std::collections::BTreeMap<String, u32> =
            std::collections::BTreeMap::new();
        let mut item_reps: std::collections::BTreeMap<String, Vec<u32>> =
            std::collections::BTreeMap::new();

        // First pass: representations and DrawingItems
        for elem in root.children().filter(|n| n.is_element()) {
            match elem.tag_name().name() {
                "PIDRepresentation" => {
                    let u = elem
                        .children()
                        .find(|c| c.is_element() && c.tag_name().name() == "IObject")
                        .and_then(|c| c.attribute("UID"))
                        .unwrap_or("");
                    let oid = elem
                        .children()
                        .find(|c| c.is_element() && c.tag_name().name() == "IDrawingRepresentation")
                        .and_then(|c| c.attribute("GraphicOID"))
                        .and_then(|s| s.parse::<u32>().ok());
                    if let Some(o) = oid {
                        rep_oids.insert(u.to_string(), o);
                    }
                }
                "Rel" => {
                    if let Some(ir) = elem
                        .children()
                        .find(|c| c.is_element() && c.tag_name().name() == "IRel")
                    {
                        let def = ir.attribute("DefUID").unwrap_or("");
                        let u1 = ir.attribute("UID1").unwrap_or("");
                        let u2 = ir.attribute("UID2").unwrap_or("");
                        if def == "DrawingItems" {
                            if let Some(&oid) = rep_oids.get(u1) {
                                item_reps.entry(u2.to_string()).or_default().push(oid);
                            }
                        } else if def != "DwgRepresentationComposition" {
                            dwg.relations.push(ExportRelation {
                                def_uid: def.to_string(),
                                uid1: u1.to_string(),
                                uid2: u2.to_string(),
                                order_value: ir.attribute("OrderValue").map(str::to_string),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: PIDDrawing attributes and domain items
        for elem in root.children().filter(|n| n.is_element()) {
            let tag = elem.tag_name().name();
            if tag == "PIDRepresentation" || tag == "Rel" {
                continue;
            }
            if tag == "PIDDrawing" {
                for sub in elem.children().filter(|c| c.is_element()) {
                    let iname = sub.tag_name().name();
                    for attr in sub.attributes() {
                        if iname == "IObject" && attr.name() == "UID" {
                            continue;
                        }
                        dwg.drawing_attributes.insert(
                            (iname.to_string(), attr.name().to_string()),
                            attr.value().to_string(),
                        );
                    }
                }
                continue;
            }

            let mut uid = String::new();
            let mut name = None;
            let mut desc = None;
            let mut attrs = std::collections::BTreeMap::new();
            for sub in elem.children().filter(|c| c.is_element()) {
                let iname = sub.tag_name().name();
                for a in sub.attributes() {
                    if iname == "IObject" {
                        match a.name() {
                            "UID" => uid = a.value().to_string(),
                            "Name" => name = Some(a.value().to_string()),
                            "Description" => desc = Some(a.value().to_string()),
                            other => {
                                attrs.insert(
                                    (iname.to_string(), other.to_string()),
                                    a.value().to_string(),
                                );
                            }
                        }
                    } else {
                        attrs.insert(
                            (iname.to_string(), a.name().to_string()),
                            a.value().to_string(),
                        );
                    }
                }
            }
            let graphic_oids = item_reps.remove(&uid).unwrap_or_default();
            dwg.items.push(ExportItem {
                uid,
                class: tag.to_string(),
                name,
                description: desc,
                variant_id: None,
                symbol_path: None,
                graphic_oids,
                attributes: attrs,
            });
        }

        let bundle = export_drawing_bundle(&dwg).unwrap();
        let spec = default_publish_spec();
        let opts = CheckOptions {
            expected_plant: Some(plant.to_string()),
            ..Default::default()
        };
        let vd = check_publish_xml_bytes(
            "out_Data.xml",
            PublishXmlKind::Data,
            &bundle.data_xml,
            spec,
            &opts,
        );
        let vm = check_publish_xml_bytes(
            "out_Meta.xml",
            PublishXmlKind::Meta,
            &bundle.meta_xml,
            spec,
            &opts,
        );
        assert!(vd.is_empty(), "Data violations: {vd:?}");
        assert!(vm.is_empty(), "Meta violations: {vm:?}");

        if !out_dir.is_empty() {
            let p = std::path::PathBuf::from(&out_dir);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("reexported_Data.xml"), &bundle.data_xml).unwrap();
            std::fs::write(p.join("reexported_Meta.xml"), &bundle.meta_xml).unwrap();
            std::fs::write(p.join("reexported_Coverage.json"), &bundle.coverage_json).unwrap();
        }
        println!(
            "re-exported items={} rels_in={} data_bytes={} meta_bytes={}",
            dwg.items.len(),
            dwg.relations.len(),
            bundle.data_xml.len(),
            bundle.meta_xml.len()
        );
    }
}

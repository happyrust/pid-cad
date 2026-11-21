//! Strict vendor-format (`UTF-8 BOM` + `CRLF` + `0/3/6/3/2` space indent)
//! `_Data.xml`, `_Meta.xml`, and `_Coverage.json` writer (`L4-1` / `#123`).

use std::collections::BTreeMap;

use serde_json::json;

use super::classes::{materialize_class_interfaces, materialize_export_item, OrderedInterfaces};
use super::model::{ExportBundle, ExportDrawing};
use super::uid::{derive_meta_envelope_uids, is_valid_hex32, relation_uid, UidAllocator};
use crate::io::pid_publish_check::default_publish_spec;

fn escape_xml_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\r' => out.push_str("&#xD;"),
            '\n' => out.push_str("&#xA;"),
            '\t' => out.push_str("&#x9;"),
            c if (c as u32) < 0x20 => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

fn push_crlf_line(buf: &mut Vec<u8>, line: &str) {
    buf.extend_from_slice(line.as_bytes());
    buf.extend_from_slice(b"\r\n");
}

fn write_object_block(buf: &mut Vec<u8>, tag: &str, interfaces: &OrderedInterfaces) {
    push_crlf_line(buf, &format!("   <{tag}>"));
    for (iface, attrs) in interfaces {
        if attrs.is_empty() {
            push_crlf_line(buf, &format!("      <{iface} />"));
        } else {
            let formatted_attrs = attrs
                .iter()
                .map(|(k, v)| format!("{k}=\"{}\"", escape_xml_attr(v)))
                .collect::<Vec<_>>()
                .join(" ");
            push_crlf_line(buf, &format!("      <{iface} {formatted_attrs} />"));
        }
    }
    push_crlf_line(buf, &format!("   </{tag}>"));
}

fn write_rel_block(
    buf: &mut Vec<u8>,
    rel_uid: &str,
    uid1: &str,
    uid2: &str,
    def_uid: &str,
    order_value: Option<&str>,
) {
    push_crlf_line(buf, "   <Rel>");
    push_crlf_line(
        buf,
        &format!("      <IObject UID=\"{}\" />", escape_xml_attr(rel_uid)),
    );
    if let Some(ord) = order_value {
        push_crlf_line(
            buf,
            &format!(
                "      <IRel UID1=\"{}\" UID2=\"{}\" DefUID=\"{}\" OrderValue=\"{}\" />",
                escape_xml_attr(uid1),
                escape_xml_attr(uid2),
                escape_xml_attr(def_uid),
                escape_xml_attr(ord)
            ),
        );
    } else {
        push_crlf_line(
            buf,
            &format!(
                "      <IRel UID1=\"{}\" UID2=\"{}\" DefUID=\"{}\" />",
                escape_xml_attr(uid1),
                escape_xml_attr(uid2),
                escape_xml_attr(def_uid)
            ),
        );
    }
    push_crlf_line(buf, "   </Rel>");
}

fn format_container_open(
    order: &[String],
    fixed: &BTreeMap<String, String>,
    plant: &str,
    doc_uid: &str,
    doc_name: &str,
) -> String {
    let mut parts = Vec::with_capacity(order.len());
    for key in order {
        let val = match key.as_str() {
            "Plant" => plant,
            "DocUID" => doc_uid,
            "DocName" => doc_name,
            other => fixed.get(other).map_or("", String::as_str),
        };
        parts.push(format!("{key}=\"{}\"", escape_xml_attr(val)));
    }
    format!("<Container {}>", parts.join(" "))
}

pub fn export_drawing_bundle(drawing: &ExportDrawing) -> Result<ExportBundle, String> {
    if drawing.plant.trim().is_empty() {
        return Err("ExportDrawing.plant must not be empty".to_string());
    }
    if !is_valid_hex32(&drawing.doc_uid) {
        return Err(format!(
            "ExportDrawing.doc_uid must be 32-char uppercase hex, got '{}'",
            drawing.doc_uid
        ));
    }

    let spec = default_publish_spec();
    let mut rep_alloc = UidAllocator::new(&drawing.plant, &drawing.doc_name);
    let _ = rep_alloc.alloc_primary_uid(Some(&drawing.doc_uid), "doc");

    let mut uid_to_class: BTreeMap<String, String> = BTreeMap::new();
    uid_to_class.insert(drawing.doc_uid.clone(), "PIDDrawing".to_string());

    for item in &drawing.items {
        if !spec.data_classes.contains_key(&item.class)
            || item.class == "PIDDrawing"
            || item.class == "PIDRepresentation"
        {
            return Err(format!(
                "unsupported or reserved item class '{}' on UID '{}'",
                item.class, item.uid
            ));
        }
        if is_valid_hex32(&item.uid) {
            let _ = rep_alloc.alloc_primary_uid(Some(&item.uid), &item.uid);
        }
        if uid_to_class
            .insert(item.uid.clone(), item.class.clone())
            .is_some()
        {
            return Err(format!("duplicate object UID '{}'", item.uid));
        }
    }

    // Build `_Data.xml`
    let mut data_buf = Vec::with_capacity(64 * 1024);
    data_buf.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    push_crlf_line(&mut data_buf, &spec.format.xml_declaration);
    push_crlf_line(
        &mut data_buf,
        &format_container_open(
            &spec.data_container.attribute_order,
            &spec.data_container.fixed_attributes,
            &drawing.plant,
            &drawing.doc_uid,
            &drawing.doc_name,
        ),
    );

    // 1. First element: `PIDDrawing`
    let dwg_cls_spec = spec
        .data_classes
        .get("PIDDrawing")
        .ok_or_else(|| "missing PIDDrawing in spec".to_string())?;
    let mut dwg_overrides = drawing.drawing_attributes.clone();
    dwg_overrides
        .entry(("IDocument".to_string(), "DocTitle".to_string()))
        .or_insert_with(|| drawing.doc_title.clone());
    let dwg_ifaces = materialize_class_interfaces(
        "PIDDrawing",
        dwg_cls_spec,
        &drawing.doc_uid,
        Some(&drawing.doc_name),
        Some(""),
        None,
        &dwg_overrides,
    );
    write_object_block(&mut data_buf, "PIDDrawing", &dwg_ifaces);

    // 2. Synthesize `PIDRepresentation` elements + DRC / DRI relations for items with `graphic_oids`
    let mut auto_rels: Vec<(String, String, String, String)> = Vec::new();
    let mut reps_emitted = 0usize;
    for item in &drawing.items {
        for &oid in &item.graphic_oids {
            let rep_uid = rep_alloc.alloc_primary_uid(None, &format!("rep:{}:{oid}", item.uid));
            uid_to_class.insert(rep_uid.clone(), "PIDRepresentation".to_string());
            let rep_ifaces = vec![
                (
                    "IObject".to_string(),
                    vec![("UID".to_string(), rep_uid.clone())],
                ),
                (
                    "IDrawingRepresentation".to_string(),
                    vec![("GraphicOID".to_string(), oid.to_string())],
                ),
            ];
            write_object_block(&mut data_buf, "PIDRepresentation", &rep_ifaces);
            reps_emitted += 1;

            let drc_uid = relation_uid("DwgRepresentationComposition", &drawing.doc_uid, &rep_uid)?;
            auto_rels.push((
                drc_uid,
                drawing.doc_uid.clone(),
                rep_uid.clone(),
                "DwgRepresentationComposition".to_string(),
            ));
            let dri_uid = relation_uid("DrawingItems", &rep_uid, &item.uid)?;
            auto_rels.push((
                dri_uid,
                rep_uid,
                item.uid.clone(),
                "DrawingItems".to_string(),
            ));
        }
    }

    // 3. Emit all domain items
    let mut class_counts: BTreeMap<String, usize> = BTreeMap::new();
    for item in &drawing.items {
        let ifaces = materialize_export_item(item)?;
        write_object_block(&mut data_buf, &item.class, &ifaces);
        *class_counts.entry(item.class.clone()).or_default() += 1;
    }

    // 4. Emit synthesized DRC/DRI relations followed by explicit domain relations
    for (ruid, u1, u2, def_uid) in &auto_rels {
        write_rel_block(&mut data_buf, ruid, u1, u2, def_uid, None);
    }

    for rel in &drawing.relations {
        let rel_spec = spec
            .data_relations
            .get(&rel.def_uid)
            .ok_or_else(|| format!("unknown DefUID '{}'", rel.def_uid))?;
        let Some(c1) = uid_to_class.get(&rel.uid1) else {
            return Err(format!(
                "dangling relation UID1 '{}' on '{}'",
                rel.uid1, rel.def_uid
            ));
        };
        let Some(c2) = uid_to_class.get(&rel.uid2) else {
            return Err(format!(
                "dangling relation UID2 '{}' on '{}'",
                rel.uid2, rel.def_uid
            ));
        };
        let allowed = rel_spec
            .allowed_endpoints
            .iter()
            .any(|pair| pair[0] == *c1 && pair[1] == *c2);
        if !allowed {
            return Err(format!(
                "disallowed endpoints {} -> {} for relation '{}'",
                c1, c2, rel.def_uid
            ));
        }
        let ruid = relation_uid(&rel.def_uid, &rel.uid1, &rel.uid2)?;
        write_rel_block(
            &mut data_buf,
            &ruid,
            &rel.uid1,
            &rel.uid2,
            &rel.def_uid,
            rel.order_value.as_deref(),
        );
    }

    push_crlf_line(&mut data_buf, "  </Container>");

    // Build `_Meta.xml`
    let meta_uids = derive_meta_envelope_uids(&drawing.plant, &drawing.doc_uid);
    let mut meta_buf = Vec::with_capacity(2048);
    meta_buf.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    push_crlf_line(&mut meta_buf, &spec.format.xml_declaration);
    push_crlf_line(
        &mut meta_buf,
        &format_container_open(
            &spec.meta_container.attribute_order,
            &spec.meta_container.fixed_attributes,
            &drawing.plant,
            &drawing.doc_uid,
            &drawing.doc_name,
        ),
    );

    let dv_spec = spec
        .meta_classes
        .get("DocumentVersion")
        .ok_or_else(|| "missing DocumentVersion in spec".to_string())?;
    let mut dv_ov = BTreeMap::new();
    dv_ov.insert(
        ("IDocumentVersion".to_string(), "DocRevision".to_string()),
        "0".to_string(),
    );
    dv_ov.insert(
        ("IDocumentVersion".to_string(), "DocVersion".to_string()),
        "1".to_string(),
    );
    dv_ov.insert(
        ("IDocumentVersion".to_string(), "DocVersionDate".to_string()),
        drawing.doc_version_date.clone(),
    );
    let dv_ifaces = materialize_class_interfaces(
        "DocumentVersion",
        dv_spec,
        &meta_uids.doc_version_uid,
        Some(&drawing.doc_name),
        None,
        None,
        &dv_ov,
    );
    write_object_block(&mut meta_buf, "DocumentVersion", &dv_ifaces);
    write_rel_block(
        &mut meta_buf,
        &meta_uids.rel_versioned_doc_uid,
        &drawing.doc_uid,
        &meta_uids.doc_version_uid,
        "VersionedDoc",
        None,
    );

    let dr_spec = spec
        .meta_classes
        .get("DocumentRevision")
        .ok_or_else(|| "missing DocumentRevision in spec".to_string())?;
    let mut dr_ov = BTreeMap::new();
    dr_ov.insert(
        (
            "IDocumentRevision".to_string(),
            "MajorRev_ForRevise".to_string(),
        ),
        "0".to_string(),
    );
    dr_ov.insert(
        (
            "IDocumentRevision".to_string(),
            "MinorRev_ForRevise".to_string(),
        ),
        String::new(),
    );
    let dr_ifaces = materialize_class_interfaces(
        "DocumentRevision",
        dr_spec,
        &meta_uids.doc_revision_uid,
        Some(&drawing.doc_name),
        None,
        None,
        &dr_ov,
    );
    write_object_block(&mut meta_buf, "DocumentRevision", &dr_ifaces);
    write_rel_block(
        &mut meta_buf,
        &meta_uids.rel_revised_doc_uid,
        &meta_uids.doc_revision_uid,
        &drawing.doc_uid,
        "RevisedDocument",
        None,
    );

    let f_spec = spec
        .meta_classes
        .get("File")
        .ok_or_else(|| "missing File in spec".to_string())?;
    let mut f_ov = BTreeMap::new();
    f_ov.insert(
        ("IFile".to_string(), "FilePath".to_string()),
        drawing.meta_file_path.clone(),
    );
    let file_name = format!("{}_Data.xml", drawing.doc_name);
    let f_ifaces = materialize_class_interfaces(
        "File",
        f_spec,
        &meta_uids.file_uid,
        Some(&file_name),
        Some(""),
        None,
        &f_ov,
    );
    write_object_block(&mut meta_buf, "File", &f_ifaces);
    write_rel_block(
        &mut meta_buf,
        &meta_uids.rel_file_comp_uid,
        &meta_uids.file_uid,
        &meta_uids.doc_version_uid,
        "FileComposition",
        None,
    );

    push_crlf_line(&mut meta_buf, "  </Container>");

    let total_rels = auto_rels.len() + drawing.relations.len();
    let coverage = json!({
        "plant": drawing.plant,
        "doc_uid": drawing.doc_uid,
        "doc_name": drawing.doc_name,
        "items_emitted": drawing.items.len(),
        "representations_emitted": reps_emitted,
        "relations_emitted": total_rels,
        "class_counts": class_counts,
        "unmapped_count": drawing.unmapped.len(),
        "unmapped": drawing.unmapped,
    });
    let coverage_json = serde_json::to_string_pretty(&coverage)
        .map_err(|e| format!("failed to serialize coverage JSON: {e}"))?;

    Ok(ExportBundle {
        data_xml: data_buf,
        meta_xml: meta_buf,
        coverage_json,
    })
}

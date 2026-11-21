//! Object-by-object `.pid` ↔ `_Data.xml` join and SPPID class-variant catalog
//! builder (`L0-9` / `#119`), producing `assets/pid-class-map.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use pid_parse::symbol_library::library_relative_path;
use pid_parse::{
    build_normalized_geometry, NormalizedPidGeometry, ParseOptions, PidGeometryConfidence,
    PidGraphicEntity, PidGraphicKind, PidParser,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassVariantEntry {
    pub variant_id: String,
    pub class: String,
    pub instance_count: usize,
    pub interfaces: Vec<String>,
    pub codelist_values: BTreeMap<String, String>,
    pub graphic_kinds: Vec<String>,
    pub symbol_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassVariantSummaryRow {
    pub class: String,
    pub instance_count: usize,
    pub variant_count: usize,
    pub linked_graphic_instances: usize,
    pub derived_non_graphic_instances: usize,
    pub unresolved_oid_instances: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassMapCatalog {
    pub schema_version: u32,
    pub drawings_analyzed: usize,
    pub total_objects: usize,
    pub total_variants: usize,
    pub class_summaries: Vec<ClassVariantSummaryRow>,
    pub variants: Vec<ClassVariantEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingObjectLink {
    pub uid: String,
    pub class: String,
    pub interfaces: Vec<String>,
    pub codelist_values: BTreeMap<String, String>,
    pub graphic_oids: Vec<u32>,
    pub graphic_kinds: Vec<String>,
    pub symbol_paths: Vec<String>,
    pub unlinked_reason: Option<&'static str>,
}

fn kind_tag(kind: &PidGraphicKind) -> &'static str {
    match kind {
        PidGraphicKind::SymbolInstance { .. } => "symbol",
        PidGraphicKind::Line { .. } => "line",
        PidGraphicKind::Polyline { .. } => "polyline",
        PidGraphicKind::Circle { .. } => "circle",
        PidGraphicKind::Arc { .. } => "arc",
        PidGraphicKind::Text { .. } => "text",
        PidGraphicKind::Point { .. } => "point",
        _ => "other",
    }
}

fn sanitized_library_sym_path(raw: &str) -> Option<String> {
    let rel = library_relative_path(raw)?;
    let clean = rel
        .trim()
        .trim_start_matches(['\\', '/'])
        .replace('/', "\\");
    if clean.is_empty() || !clean.is_ascii() || clean.contains(':') || clean.starts_with("\\\\") {
        return None;
    }
    Some(clean)
}

fn parse_xml_text(bytes: &[u8]) -> Result<String, String> {
    let body = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    let text = std::str::from_utf8(body).map_err(|e| format!("invalid UTF-8 XML: {e}"))?;
    if let Some(rest) = text.strip_prefix("<?xml version =") {
        Ok(format!("<?xml  version={rest}"))
    } else {
        Ok(text.to_string())
    }
}

pub fn link_drawing_objects(
    geometry: &NormalizedPidGeometry,
    data_xml_bytes: &[u8],
) -> Result<Vec<DrawingObjectLink>, String> {
    let xml_str = parse_xml_text(data_xml_bytes)?;
    let doc = roxmltree::Document::parse(&xml_str).map_err(|e| format!("XML parse error: {e}"))?;
    let root = doc.root_element();

    let mut oid_entities: BTreeMap<u32, Vec<&PidGraphicEntity>> = BTreeMap::new();
    for entity in &geometry.entities {
        if entity.confidence == PidGeometryConfidence::Decoded {
            if let Some(oid) = entity.graphic_oid {
                oid_entities.entry(oid).or_default().push(entity);
            }
        }
    }

    let mut rep_to_oid: BTreeMap<&str, u32> = BTreeMap::new();
    let mut item_to_reps: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut item_nodes: Vec<roxmltree::Node<'_, '_>> = Vec::new();

    for elem in root.children().filter(|n| n.is_element()) {
        let tag = elem.tag_name().name();
        match tag {
            "PIDDrawing" => {}
            "PIDRepresentation" => {
                let uid = elem
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "IObject")
                    .and_then(|c| c.attribute("UID"))
                    .unwrap_or("");
                let oid = elem
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "IDrawingRepresentation")
                    .and_then(|c| c.attribute("GraphicOID"))
                    .and_then(|s| s.parse::<u32>().ok());
                if !uid.is_empty() {
                    if let Some(oid_val) = oid {
                        rep_to_oid.insert(uid, oid_val);
                    }
                }
            }
            "Rel" => {
                if let Some(irel) = elem
                    .children()
                    .find(|c| c.is_element() && c.tag_name().name() == "IRel")
                {
                    if irel.attribute("DefUID") == Some("DrawingItems") {
                        let u1 = irel.attribute("UID1").unwrap_or("");
                        let u2 = irel.attribute("UID2").unwrap_or("");
                        if !u1.is_empty() && !u2.is_empty() {
                            item_to_reps.entry(u2).or_default().push(u1);
                        }
                    }
                }
            }
            _ => {
                item_nodes.push(elem);
            }
        }
    }

    let mut links = Vec::with_capacity(item_nodes.len());
    for elem in item_nodes {
        let class = elem.tag_name().name().to_string();
        let mut uid = String::new();
        let mut ifaces_set: BTreeSet<String> = BTreeSet::new();
        let mut codelist_values: BTreeMap<String, String> = BTreeMap::new();

        for sub in elem.children().filter(|c| c.is_element()) {
            let iname = sub.tag_name().name();
            ifaces_set.insert(iname.to_string());
            if iname == "IObject" {
                if let Some(u) = sub.attribute("UID") {
                    uid = u.to_string();
                }
            }
            for attr in sub.attributes() {
                let val = attr.value();
                if val.starts_with('@') && val.is_ascii() {
                    codelist_values.insert(format!("{iname}.{}", attr.name()), val.to_string());
                }
            }
        }

        let mut graphic_oids: Vec<u32> = item_to_reps
            .get(uid.as_str())
            .into_iter()
            .flatten()
            .filter_map(|rep_uid| rep_to_oid.get(rep_uid).copied())
            .collect();
        graphic_oids.sort_unstable();
        graphic_oids.dedup();

        let mut kinds_set: BTreeSet<String> = BTreeSet::new();
        let mut syms_set: BTreeSet<String> = BTreeSet::new();
        for &oid in &graphic_oids {
            if let Some(ents) = oid_entities.get(&oid) {
                for ent in ents {
                    kinds_set.insert(kind_tag(&ent.kind).to_string());
                    if let PidGraphicKind::SymbolInstance {
                        symbol_path: Some(raw_sym),
                        ..
                    } = &ent.kind
                    {
                        if let Some(clean_sym) = sanitized_library_sym_path(raw_sym) {
                            syms_set.insert(clean_sym);
                        }
                    }
                }
            }
        }

        let unlinked_reason = if graphic_oids.is_empty() {
            Some("derived_model_object_without_drawing_representation")
        } else if kinds_set.is_empty() {
            Some("graphic_oid_not_in_decoded_sheet_entities")
        } else {
            None
        };

        links.push(DrawingObjectLink {
            uid,
            class,
            interfaces: ifaces_set.into_iter().collect(),
            codelist_values,
            graphic_oids,
            graphic_kinds: kinds_set.into_iter().collect(),
            symbol_paths: syms_set.into_iter().collect(),
            unlinked_reason,
        });
    }

    Ok(links)
}

#[derive(Default)]
struct VariantAccum {
    instance_count: usize,
    graphic_kinds: BTreeSet<String>,
    symbol_paths: BTreeSet<String>,
}

#[derive(Default)]
struct ClassStats {
    instance_count: usize,
    linked_graphic: usize,
    derived_non_graphic: usize,
    unresolved_oid: usize,
}

type VariantKey = (String, Vec<String>, BTreeMap<String, String>);

fn aggregate_links(
    drawings_analyzed: usize,
    per_drawing_links: &[Vec<DrawingObjectLink>],
) -> ClassMapCatalog {
    let mut variants_map: BTreeMap<VariantKey, VariantAccum> = BTreeMap::new();
    let mut class_stats: BTreeMap<String, ClassStats> = BTreeMap::new();
    let mut total_objects = 0usize;

    for links in per_drawing_links {
        for link in links {
            total_objects += 1;
            let st = class_stats.entry(link.class.clone()).or_default();
            st.instance_count += 1;
            match link.unlinked_reason {
                None => st.linked_graphic += 1,
                Some("derived_model_object_without_drawing_representation") => {
                    st.derived_non_graphic += 1;
                }
                Some(_) => st.unresolved_oid += 1,
            }

            let key: VariantKey = (
                link.class.clone(),
                link.interfaces.clone(),
                link.codelist_values.clone(),
            );
            let slot = variants_map.entry(key).or_default();
            slot.instance_count += 1;
            if link.graphic_kinds.is_empty() {
                if let Some(reason) = link.unlinked_reason {
                    slot.graphic_kinds.insert(reason.to_string());
                }
            } else {
                slot.graphic_kinds
                    .extend(link.graphic_kinds.iter().cloned());
            }
            slot.symbol_paths.extend(link.symbol_paths.iter().cloned());
        }
    }

    // Group variants by class, ordered by instance_count descending within each class.
    type ClassVariantGroup = Vec<(Vec<String>, BTreeMap<String, String>, VariantAccum)>;
    let mut by_class: BTreeMap<String, ClassVariantGroup> = BTreeMap::new();
    for ((cls, ifaces, codes), acc) in variants_map {
        by_class.entry(cls).or_default().push((ifaces, codes, acc));
    }

    let mut class_summaries = Vec::with_capacity(class_stats.len());
    let mut variants = Vec::new();

    for (cls, st) in class_stats {
        let mut group = by_class.remove(&cls).unwrap_or_default();
        group.sort_by(|a, b| {
            b.2.instance_count
                .cmp(&a.2.instance_count)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
        });
        class_summaries.push(ClassVariantSummaryRow {
            class: cls.clone(),
            instance_count: st.instance_count,
            variant_count: group.len(),
            linked_graphic_instances: st.linked_graphic,
            derived_non_graphic_instances: st.derived_non_graphic,
            unresolved_oid_instances: st.unresolved_oid,
        });
        for (idx, (ifaces, codes, acc)) in group.into_iter().enumerate() {
            variants.push(ClassVariantEntry {
                variant_id: format!("{cls}#{:02}", idx + 1),
                class: cls.clone(),
                instance_count: acc.instance_count,
                interfaces: ifaces,
                codelist_values: codes,
                graphic_kinds: acc.graphic_kinds.into_iter().collect(),
                symbol_paths: acc.symbol_paths.into_iter().collect(),
            });
        }
    }

    ClassMapCatalog {
        schema_version: 1,
        drawings_analyzed,
        total_objects,
        total_variants: variants.len(),
        class_summaries,
        variants,
    }
}

pub fn build_catalog_from_pairs(
    pairs: &[(NormalizedPidGeometry, Vec<u8>)],
) -> Result<(ClassMapCatalog, Vec<(usize, DrawingObjectLink)>), String> {
    let mut per_drawing = Vec::with_capacity(pairs.len());
    let mut flat = Vec::new();
    for (idx, (geom, xml)) in pairs.iter().enumerate() {
        let links = link_drawing_objects(geom, xml)?;
        for l in &links {
            flat.push((idx, l.clone()));
        }
        per_drawing.push(links);
    }
    let catalog = aggregate_links(pairs.len(), &per_drawing);
    Ok((catalog, flat))
}

pub fn build_catalog_from_disk_pairs(
    pairs: &[(PathBuf, PathBuf)],
) -> Result<(ClassMapCatalog, String), String> {
    let parser = PidParser::with_options(ParseOptions::geometry());
    let mut per_drawing = Vec::with_capacity(pairs.len());
    let mut unresolved_reasons: BTreeMap<(&'static str, String), usize> = BTreeMap::new();

    for (pid_path, xml_path) in pairs {
        let parsed = parser
            .parse_file(pid_path)
            .map_err(|e| format!("cannot parse '{}': {e}", pid_path.display()))?;
        let geom = build_normalized_geometry(&parsed);
        let xml_bytes = std::fs::read(xml_path)
            .map_err(|e| format!("cannot read '{}': {e}", xml_path.display()))?;
        let links = link_drawing_objects(&geom, &xml_bytes)?;
        for l in &links {
            if let Some(reason) = l.unlinked_reason {
                *unresolved_reasons
                    .entry((reason, l.class.clone()))
                    .or_default() += 1;
            }
        }
        per_drawing.push(links);
    }

    let catalog = aggregate_links(pairs.len(), &per_drawing);

    let mut report = String::new();
    let _ = writeln!(
        report,
        "# `.pid` 与 `_Data.xml` 逐对象对应及类变体统计报告（#119）\n"
    );
    let _ = writeln!(report, "- 分析成对图纸数：`{}`", catalog.drawings_analyzed);
    let _ = writeln!(
        report,
        "- 模型对象总数（不含 `PIDDrawing`/`PIDRepresentation`）：`{}`",
        catalog.total_objects
    );
    let _ = writeln!(
        report,
        "- 「接口集 + 码表值」类变体总数：**`{}`**\n",
        catalog.total_variants
    );
    let _ = writeln!(
        report,
        "| 对象类 | 实例数 | 类变体数 | 已连到图元实例数 | 派生无图形实例数 | 有 `GraphicOID` 未命中根图元数 |"
    );
    let _ = writeln!(report, "|---|---:|---:|---:|---:|---:|");
    for s in &catalog.class_summaries {
        let _ = writeln!(
            report,
            "| `{}` | {} | {} | {} | {} | {} |",
            s.class,
            s.instance_count,
            s.variant_count,
            s.linked_graphic_instances,
            s.derived_non_graphic_instances,
            s.unresolved_oid_instances,
        );
    }
    let _ = writeln!(report, "\n## 未直接连到根图纸解码图元的分类原因\n");
    let _ = writeln!(report, "| 原因分类 | 对象类 | 实例数 |");
    let _ = writeln!(report, "|---|---|---:|");
    for ((reason, cls), count) in unresolved_reasons {
        let _ = writeln!(report, "| `{reason}` | `{cls}` | {count} |");
    }

    Ok((catalog, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pid_parse::{PidPoint, PidSymbolDefinitionRef};

    const DOC_UID: &str = "0102030405060708090A0B0C0D0E0F10";
    const REP_VALVE: &str = "1112131415161718191A1B1C1D1E1F20";
    const REP_PIPE: &str = "1112131415161718191A1B1C1D1E1F21";
    const VALVE_UID: &str = "2122232425262728292A2B2C2D2E2F30";
    const CONN_UID: &str = "3132333435363738393A3B3C3D3E3F40";
    const PIPELINE_UID: &str = "4142434445464748494A4B4C4D4E4F50";

    fn synth_geometry() -> NormalizedPidGeometry {
        let provenance = || pid_parse::PidGraphicProvenance {
            stream_path: None,
            byte_range: None,
            record_id: None,
            record_kind: None,
            field_x: None,
            note: None,
        };
        NormalizedPidGeometry {
            entities: vec![
                PidGraphicEntity {
                    id: "1".to_string(),
                    drawing_id: None,
                    graphic_oid: Some(501),
                    source_layer: None,
                    kind: PidGraphicKind::SymbolInstance {
                        insertion: PidPoint { x: 1.0, y: 2.0 },
                        symbol_path: Some(
                            r"\\SERVER\Plant\Ref\Symbols\Piping\Valves\Gate Valve.sym".to_string(),
                        ),
                        rotation: 0.0,
                        scale: [1.0, 1.0],
                        definition: Some(PidSymbolDefinitionRef { site: 1, sheet: 1 }),
                    },
                    coordinate_context: Default::default(),
                    source: provenance(),
                    confidence: PidGeometryConfidence::Decoded,
                },
                PidGraphicEntity {
                    id: "2".to_string(),
                    drawing_id: None,
                    graphic_oid: Some(502),
                    source_layer: None,
                    kind: PidGraphicKind::Line {
                        start: PidPoint { x: 0.0, y: 0.0 },
                        end: PidPoint { x: 5.0, y: 0.0 },
                    },
                    coordinate_context: Default::default(),
                    source: provenance(),
                    confidence: PidGeometryConfidence::Decoded,
                },
            ],
            ..Default::default()
        }
    }

    fn synth_xml() -> Vec<u8> {
        let lines = [
            r#"<?xml version ="1.0" encoding="UTF-8"?>"#,
            &format!(r#"<Container DocUID="{DOC_UID}" DocName="SYNTH">"#),
            "   <PIDDrawing>",
            &format!(r#"      <IObject UID="{DOC_UID}" Name="SYNTH" />"#),
            "   </PIDDrawing>",
            "   <PIDRepresentation>",
            &format!(r#"      <IObject UID="{REP_VALVE}" />"#),
            r#"      <IDrawingRepresentation GraphicOID="501" />"#,
            "   </PIDRepresentation>",
            "   <PIDRepresentation>",
            &format!(r#"      <IObject UID="{REP_PIPE}" />"#),
            r#"      <IDrawingRepresentation GraphicOID="502" />"#,
            "   </PIDRepresentation>",
            "   <PIDPipingComponent>",
            &format!(r#"      <IObject UID="{VALVE_UID}" Name="XV-101" />"#),
            r#"      <IPipingComponent PipingComponentType1="@EE3F8" PipingComponentType2="@EE410" />"#,
            "   </PIDPipingComponent>",
            "   <PIDPipingConnector>",
            &format!(r#"      <IObject UID="{CONN_UID}" Name="" />"#),
            r#"      <IPipingConnector FlowDirection="@ForwardFlow" PipingConnectorSeqNo="1001" />"#,
            "   </PIDPipingConnector>",
            "   <PIDPipeline>",
            &format!(r#"      <IObject UID="{PIPELINE_UID}" Name="50 mm-PG-1001-B2" />"#),
            r#"      <IPipeline FluidSystem="@{78398AB4-9F3D-11D6-BDA7-00104BCC2B69}" />"#,
            "   </PIDPipeline>",
            "   <Rel>",
            &format!(r#"      <IObject UID="DRI-{REP_VALVE}-{VALVE_UID}" />"#),
            &format!(
                r#"      <IRel UID1="{REP_VALVE}" UID2="{VALVE_UID}" DefUID="DrawingItems" />"#
            ),
            "   </Rel>",
            "   <Rel>",
            &format!(r#"      <IObject UID="DRI-{REP_PIPE}-{CONN_UID}" />"#),
            &format!(r#"      <IRel UID1="{REP_PIPE}" UID2="{CONN_UID}" DefUID="DrawingItems" />"#),
            "   </Rel>",
            "  </Container>",
        ];
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice((lines.join("\r\n") + "\r\n").as_bytes());
        raw
    }

    #[test]
    fn links_objects_to_graphics_and_extracts_sanitized_variants() {
        let geom = synth_geometry();
        let xml = synth_xml();

        let links = link_drawing_objects(&geom, &xml).unwrap();
        assert_eq!(links.len(), 3);

        let valve = links.iter().find(|l| l.uid == VALVE_UID).unwrap();
        assert_eq!(valve.class, "PIDPipingComponent");
        assert_eq!(valve.graphic_oids, vec![501]);
        assert_eq!(valve.graphic_kinds, vec!["symbol"]);
        assert_eq!(
            valve.symbol_paths,
            vec![r"Piping\Valves\Gate Valve.sym".to_string()]
        );
        assert!(valve.unlinked_reason.is_none());

        let pipeline = links.iter().find(|l| l.uid == PIPELINE_UID).unwrap();
        assert_eq!(
            pipeline.unlinked_reason,
            Some("derived_model_object_without_drawing_representation")
        );

        let (catalog, _all_links) = build_catalog_from_pairs(&[(geom, xml)]).unwrap();
        assert_eq!(catalog.drawings_analyzed, 1);
        assert_eq!(catalog.total_variants, 3);

        let json_str = serde_json::to_string_pretty(&catalog).unwrap();
        assert!(!json_str.contains("XV-101"));
        assert!(!json_str.contains("50 mm-PG-1001-B2"));
        assert!(!json_str.contains("SERVER"));
        assert!(!json_str.contains(VALVE_UID));
        assert!(json_str.contains(r"Piping\\Valves\\Gate Valve.sym"));
        assert!(json_str.contains("@EE3F8"));
    }

    #[test]
    #[ignore = "scans 94 real (.pid, _Data.xml) pairs and writes assets/pid-class-map.json + evidence report"]
    fn real_corpus_build_class_variant_map() {
        let Ok(export_dir) = std::env::var("OCS_PID_VARIANT_EXPORT_DIR") else {
            println!("未设置语料目录（OCS_PID_VARIANT_EXPORT_DIR），跳过类变体目录生成。");
            return;
        };
        let write_spec_path = std::env::var("OCS_PID_VARIANT_SPEC_OUT").unwrap_or_default();
        let evidence_out = std::env::var("OCS_PID_VARIANT_REPORT_OUT").unwrap_or_default();

        let mut pids = Vec::new();
        let mut stack = vec![PathBuf::from(&export_dir)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pid")) {
                    pids.push(p);
                }
            }
        }
        pids.sort();
        let mut pairs = Vec::new();
        for pid_path in pids {
            let stem = pid_path.file_stem().unwrap().to_string_lossy();
            let xml_path = pid_path.with_file_name(format!("{stem}_Data.xml"));
            if xml_path.is_file() {
                pairs.push((pid_path, xml_path));
            }
        }
        assert!(
            !pairs.is_empty(),
            "no (.pid, _Data.xml) pairs in {export_dir}"
        );

        let (catalog, report_md) = build_catalog_from_disk_pairs(&pairs).unwrap();
        println!(
            "pairs={} objects={} variants={}",
            catalog.drawings_analyzed, catalog.total_objects, catalog.total_variants
        );
        for row in &catalog.class_summaries {
            println!(
                "  {}: instances={} variants={} linked={} derived={} unresolved_oid={}",
                row.class,
                row.instance_count,
                row.variant_count,
                row.linked_graphic_instances,
                row.derived_non_graphic_instances,
                row.unresolved_oid_instances
            );
        }

        let json_text = format!("{}\n", serde_json::to_string_pretty(&catalog).unwrap());
        assert!(
            !json_text.contains(r"\\\\"),
            "UNC prefix leaked into catalog!"
        );
        assert!(
            !json_text.contains(":\\"),
            "Drive path leaked into catalog!"
        );
        assert!(json_text.is_ascii(), "non-ASCII text leaked into catalog!");

        if !write_spec_path.is_empty() {
            std::fs::write(&write_spec_path, &json_text).unwrap();
        }
        if !evidence_out.is_empty() {
            let p = PathBuf::from(&evidence_out);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, report_md).unwrap();
        }
        assert!(catalog.total_variants >= 200);
    }
}

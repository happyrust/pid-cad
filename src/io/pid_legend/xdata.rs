//! Publish recognition results as entity XDATA.
//!
//! The native `.pid` importer and legend recognition deliberately share the
//! `PID_SEMANTICS` application. Records whose `resolved` value starts with
//! `legend:` belong to this module; authored `.pid` records are left intact.

use std::collections::{BTreeMap, BTreeSet};

use acadrust::xdata::{ExtendedDataRecord, XDataValue};
use acadrust::{CadDocument, Handle};

use super::{Recognition, Recognized};
use crate::io::PID_SEMANTICS_XDATA_APP;

#[derive(Debug, Clone, Default)]
struct DesiredRecord {
    class: String,
    labels: BTreeSet<String>,
    lines: BTreeSet<String>,
    resolved: Option<String>,
    pipe: bool,
}

impl DesiredRecord {
    fn for_symbol(symbol: &Recognized) -> Self {
        let mut labels = BTreeSet::new();
        if let Some(tag) = symbol.tag.as_deref().filter(|tag| !tag.is_empty()) {
            labels.insert(tag.to_string());
        }
        Self {
            // `class` is the semantic type shown in Properties. Recognition's
            // `label` is that human-readable type; its `class` is the stable
            // rule key used for layer names and reports.
            class: symbol.label.clone(),
            labels,
            lines: symbol.lines.iter().cloned().collect(),
            resolved: Some(resolved_by(symbol)),
            pipe: false,
        }
    }

    fn for_pipe() -> Self {
        Self {
            class: "PIDPipeline".to_string(),
            resolved: Some("legend:pipe".to_string()),
            pipe: true,
            ..Self::default()
        }
    }

    fn into_xdata(self) -> ExtendedDataRecord {
        let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
        push_pair(&mut record, "class", &self.class);
        for label in self.labels {
            push_pair(&mut record, "label", &label);
        }
        if !self.lines.is_empty() {
            push_pair(
                &mut record,
                "lines",
                &self.lines.into_iter().collect::<Vec<_>>().join(","),
            );
        }
        if let Some(resolved) = self.resolved {
            push_pair(&mut record, "resolved", &resolved);
        }
        record
    }
}

fn push_pair(record: &mut ExtendedDataRecord, key: &str, value: &str) {
    if !value.is_empty() {
        record.add_value(XDataValue::String(format!("{key}={value}")));
    }
}

fn resolved_by(symbol: &Recognized) -> String {
    if let Some(group) = &symbol.group {
        return format!("legend:group:{}", group.name);
    }
    if let Some(id) = symbol.source.strip_prefix("shape ") {
        return format!("legend:shape:{id}");
    }
    if symbol.source.starts_with("circle ") {
        return "legend:circle".to_string();
    }
    "legend:block".to_string()
}

fn is_legend_record(record: &ExtendedDataRecord) -> bool {
    record.values.iter().any(|value| {
        matches!(
            value,
            XDataValue::String(value)
                if value
                    .strip_prefix("resolved=")
                    .is_some_and(|resolved| resolved.starts_with("legend:"))
        )
    })
}

fn desired_records(recognition: &Recognition) -> BTreeMap<Handle, ExtendedDataRecord> {
    let mut desired = BTreeMap::<Handle, DesiredRecord>::new();
    for symbol in &recognition.symbols {
        let record = DesiredRecord::for_symbol(symbol);
        for handle in &symbol.handles {
            // Recognition owns every symbol entity once. If malformed input
            // does overlap, keep the first symbol rather than turning its body
            // into a pipeline below.
            desired.entry(*handle).or_insert_with(|| record.clone());
        }
    }
    for run in &recognition.pipes.runs {
        for handle in &run.handles {
            use std::collections::btree_map::Entry;
            match desired.entry(*handle) {
                Entry::Occupied(mut entry) if entry.get().pipe => {
                    entry.get_mut().labels.extend(run.lines.iter().cloned());
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(entry) => {
                    let mut record = DesiredRecord::for_pipe();
                    record.labels.extend(run.lines.iter().cloned());
                    entry.insert(record);
                }
            }
        }
    }
    desired
        .into_iter()
        .map(|(handle, record)| (handle, record.into_xdata()))
        .collect()
}

fn ensure_app_id(document: &mut CadDocument) -> u64 {
    if !document.app_ids.contains(PID_SEMANTICS_XDATA_APP) {
        let mut app = acadrust::tables::AppId::new(PID_SEMANTICS_XDATA_APP);
        app.handle = document.allocate_handle();
        let _ = document.app_ids.add(app);
    }
    document
        .app_ids
        .get(PID_SEMANTICS_XDATA_APP)
        .map(|app| app.handle.value())
        .unwrap_or(0)
}

/// Write `recognition` onto the entities it describes.
///
/// Existing legend-owned records are updated or removed so running
/// `PIDLEGEND ON` twice never leaves stale metadata. A record authored by the
/// `.pid` importer (`resolved=direct` / `dependency:…`) wins over recognition
/// on the same entity and is not overwritten. Returns the number of entity
/// records changed.
pub fn attach(document: &mut CadDocument, recognition: &Recognition) -> usize {
    let desired = desired_records(recognition);
    let app_handle = if desired.is_empty() {
        document
            .app_ids
            .get(PID_SEMANTICS_XDATA_APP)
            .map(|app| app.handle.value())
    } else {
        Some(ensure_app_id(document))
    };
    let mut changed = 0;

    for entity in document.entities_mut() {
        let handle = entity.common().handle;
        let wanted = desired.get(&handle);
        let xdata = &mut entity.common_mut().extended_data;
        let current = xdata.get_record(PID_SEMANTICS_XDATA_APP).cloned();

        if current
            .as_ref()
            .is_some_and(|record| !is_legend_record(record))
        {
            // Published SmartPlant identity is richer and must not be covered
            // by a second producer sharing the same APPID.
            continue;
        }

        let record_changed = match (current.as_ref(), wanted) {
            (Some(current), Some(wanted)) => *current != *wanted,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if !record_changed {
            continue;
        }

        match wanted {
            Some(record) => xdata.upsert_record(record.clone()),
            None => {
                xdata.remove_record(PID_SEMANTICS_XDATA_APP);
            }
        }
        if let Some(app_handle) = app_handle {
            xdata.raw_dwg_eed.retain(|(app, _)| *app != app_handle);
        }
        changed += 1;
    }
    changed
}

/// Handles currently carrying legend-owned `PID_SEMANTICS` records, sorted.
pub fn attached_handles(document: &CadDocument) -> Vec<Handle> {
    let mut handles: Vec<Handle> = document
        .entities()
        .filter(|entity| {
            entity
                .common()
                .extended_data
                .get_record(PID_SEMANTICS_XDATA_APP)
                .is_some_and(is_legend_record)
        })
        .map(|entity| entity.common().handle)
        .collect();
    handles.sort_unstable_by_key(|handle| handle.value());
    handles
}

/// Remove legend-owned XDATA without touching `.pid` identity or GROUP
/// descriptions. Returns the number of entity records removed.
pub fn purge(document: &mut CadDocument) -> usize {
    let app_handle = document
        .app_ids
        .get(PID_SEMANTICS_XDATA_APP)
        .map(|app| app.handle.value());
    let mut removed = 0;
    for entity in document.entities_mut() {
        let xdata = &mut entity.common_mut().extended_data;
        if !xdata
            .get_record(PID_SEMANTICS_XDATA_APP)
            .is_some_and(is_legend_record)
        {
            continue;
        }
        xdata.remove_record(PID_SEMANTICS_XDATA_APP);
        if let Some(app_handle) = app_handle {
            xdata.raw_dwg_eed.retain(|(app, _)| *app != app_handle);
        }
        removed += 1;
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_legend::{GroupOrigin, GroupTag, TagSource};
    use crate::io::pid_pipes::{End, Pipes, Run};
    use crate::scene::Scene;
    use acadrust::entities::{EntityType, Line};
    use acadrust::objects::ObjectType;
    use acadrust::types::Vector3;

    fn line(y: f64) -> EntityType {
        EntityType::Line(Line::from_points(
            Vector3::new(0.0, y, 0.0),
            Vector3::new(10.0, y, 0.0),
        ))
    }

    fn symbol(handle: Handle, tag: &str, source: &str, group: Option<GroupOrigin>) -> Recognized {
        Recognized {
            class: "butterfly".to_string(),
            label: "蝶阀".to_string(),
            color: [255, 140, 0],
            at: (5.0, 0.0),
            bbox: (0.0, 0.0, 10.0, 1.0),
            source: source.to_string(),
            group,
            known: true,
            inner_text: Vec::new(),
            tag: Some(tag.to_string()),
            tag_distance_mm: Some(0.0),
            wants_tag: true,
            report_untagged: true,
            lines: vec!["100-FW".to_string()],
            handles: vec![handle],
            tag_handles: Vec::new(),
        }
    }

    fn strings(document: &CadDocument, handle: Handle) -> Vec<String> {
        document
            .get_entity(handle)
            .and_then(|entity| {
                entity
                    .common()
                    .extended_data
                    .get_record(PID_SEMANTICS_XDATA_APP)
            })
            .map(|record| {
                record
                    .values
                    .iter()
                    .filter_map(|value| match value {
                        XDataValue::String(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn resolved_value_distinguishes_block_circle_shape_and_manual_group_sources() {
        let handle = Handle::new(1);
        assert_eq!(
            resolved_by(&symbol(handle, "BUV-1", "$VALVE$00000316", None)),
            "legend:block"
        );
        assert_eq!(
            resolved_by(&symbol(handle, "XV-1", "circle r=2.5", None)),
            "legend:circle"
        );
        assert_eq!(
            resolved_by(&symbol(handle, "BV1", "shape 5311cc3f", None)),
            "legend:shape:5311cc3f"
        );
        assert_eq!(
            resolved_by(&symbol(
                handle,
                "BUV-1",
                "group *A1",
                Some(GroupOrigin {
                    name: "*A1".to_string(),
                    tag_source: TagSource::Auto,
                }),
            )),
            "legend:group:*A1"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recognition_xdata_round_trips_and_purge_keeps_authored_identity_and_groups() {
        let mut scene = Scene::new();
        let grouped = scene.add_entity(line(0.0));
        let pipe = scene.add_entity(line(2.0));
        let authored = scene.add_entity(line(4.0));
        let group = scene.create_group("VALVE-1".to_string(), vec![grouped]);
        scene.set_group_tag(group, Some(&GroupTag::manual("BUV-3201")));

        let mut authored_record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
        for pair in ["class=PIDProcessVessel", "label=V-1", "resolved=direct"] {
            authored_record.add_value(XDataValue::String(pair.to_string()));
        }
        scene
            .document
            .get_entity_mut(authored)
            .unwrap()
            .common_mut()
            .extended_data
            .add_record(authored_record);

        let recognition = Recognition {
            symbols: vec![
                symbol(
                    grouped,
                    "BUV-3201",
                    "group VALVE-1",
                    Some(GroupOrigin {
                        name: "VALVE-1".to_string(),
                        tag_source: TagSource::Manual,
                    }),
                ),
                symbol(authored, "BUV-9999", "$VALVE$00000316", None),
            ],
            pipes: Pipes {
                runs: vec![Run {
                    numbers: vec!["100-FW".to_string(), "200-FW".to_string()],
                    lines: vec!["100-FW".to_string(), "200-FW".to_string()],
                    segments: 1,
                    length_mm: 10.0,
                    ends: [End::Open, End::Open],
                    path: vec![(0.0, 2.0), (10.0, 2.0)],
                    handles: vec![pipe],
                }],
                ..Pipes::default()
            },
            ..Recognition::default()
        };

        assert_eq!(attach(&mut scene.document, &recognition), 2);
        assert_eq!(attach(&mut scene.document, &recognition), 0);
        assert!(scene.document.app_ids.contains(PID_SEMANTICS_XDATA_APP));
        assert_eq!(
            strings(&scene.document, grouped),
            [
                "class=蝶阀",
                "label=BUV-3201",
                "lines=100-FW",
                "resolved=legend:group:VALVE-1",
            ]
        );
        assert_eq!(
            strings(&scene.document, pipe),
            [
                "class=PIDPipeline",
                "label=100-FW",
                "label=200-FW",
                "resolved=legend:pipe",
            ]
        );
        assert_eq!(
            strings(&scene.document, authored),
            ["class=PIDProcessVessel", "label=V-1", "resolved=direct",],
            "recognition does not cover published .pid identity"
        );
        assert_eq!(attached_handles(&scene.document), vec![grouped, pipe]);

        for ext in ["dxf", "dwg"] {
            let bytes = crate::io::save_to_bytes(&scene.document, ext, scene.document.version)
                .unwrap_or_else(|error| panic!("{ext}: {error}"));
            let mut back = crate::io::load_bytes(&format!("legend-xdata.{ext}"), bytes)
                .unwrap_or_else(|error| panic!("{ext}: {error}"));
            assert_eq!(
                strings(&back, grouped),
                strings(&scene.document, grouped),
                "{ext}"
            );
            assert_eq!(
                strings(&back, pipe),
                strings(&scene.document, pipe),
                "{ext}"
            );
            assert_eq!(
                strings(&back, authored),
                strings(&scene.document, authored),
                "{ext}"
            );

            assert_eq!(purge(&mut back), 2, "{ext}");
            assert!(strings(&back, grouped).is_empty(), "{ext}");
            assert!(strings(&back, pipe).is_empty(), "{ext}");
            assert_eq!(
                strings(&back, authored),
                strings(&scene.document, authored),
                "{ext}: authored identity survives purge"
            );
            let group = back
                .objects
                .values()
                .find_map(|object| match object {
                    ObjectType::Group(group) => Some(group),
                    _ => None,
                })
                .expect("manual group survives");
            assert_eq!(
                GroupTag::parse(&group.description),
                Some(GroupTag::manual("BUV-3201")),
                "{ext}: purge never touches the GROUP description"
            );

            // A second save proves raw DWG EED captured on reload cannot bring
            // the purged legend record back.
            let bytes = crate::io::save_to_bytes(&back, ext, back.version)
                .unwrap_or_else(|error| panic!("{ext} after purge: {error}"));
            let purged = crate::io::load_bytes(&format!("legend-purged.{ext}"), bytes)
                .unwrap_or_else(|error| panic!("{ext} after purge: {error}"));
            assert!(strings(&purged, grouped).is_empty(), "{ext}");
            assert!(strings(&purged, pipe).is_empty(), "{ext}");
            assert_eq!(
                strings(&purged, authored),
                strings(&back, authored),
                "{ext}"
            );
        }
    }
}

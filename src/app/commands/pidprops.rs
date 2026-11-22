//! The Properties panel's `P&ID 属性` group: the typed author record
//! (`PID_AUTHOR`) a placed symbol carries, shown and edited in place.
//!
//! Row ids are fixed by the architecture: `pid_type`, `pid_tag`,
//! `pid_tag_source`, `pid_tag_status`, then one `pid_props_<key>` row per
//! `props` key, and an empty one for each property a clicked rule-check result
//! highlights that the record lacks (panel spec §3.6). Only the tag and the
//! props rows are editable; a tag edit makes the tag `manual`. Every commit is one undo step that also re-runs the
//! uniqueness check and redraws the canvas tag labels
//! ([`OpenCADStudio::refresh_pid_tags`]), so a conflict mark (`？`) and the
//! label follow the edit and roll back with it.

use super::*;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::{
    apply, join_props, propagate_line_to_runs, propagate_run_to_segments, set_prop_manual,
    split_props, LINE_FIELDS,
};
use crate::pid::tag_engine::{TagView, TAG_SOURCE_MANUAL};
use crate::pid::topology::{parse_handle, TopologyTable};
use crate::scene::model::object::{PropSection, PropValue, Property};
use crate::t;

/// Title of the author group. Kept apart from the recognizer's `P&ID` group
/// (`PID_SEMANTICS` / marked GROUPs), which merges rows by that exact title.
pub(in crate::app) const PID_AUTHOR_SECTION: &str = "P&ID 属性";

pub(in crate::app) const TAG_FIELD: &str = "pid_tag";
pub(in crate::app) const PROPS_FIELD_PREFIX: &str = "pid_props_";

/// `Property::field` is `&'static str`, but props keys are data. Each distinct
/// row id is leaked once and reused, so repeated refreshes do not grow memory.
fn props_field(key: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static FIELDS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let id = format!("{PROPS_FIELD_PREFIX}{key}");
    let mut fields = FIELDS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = fields.get(id.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(id.into_boxed_str());
    fields.insert(leaked);
    leaked
}

/// The `P&ID 属性` group for `handle`, or `None` when it carries no
/// `PID_AUTHOR` record. Each property of `highlighted` the record has no value
/// for gets an empty row after the record's own, so its highlight shows and
/// filling it in adds the property; the tag has its row already.
pub(in crate::app) fn pid_author_section(
    document: &document::Document,
    handle: document::Handle,
    highlighted: &[String],
) -> Option<PropSection> {
    let author = PidAuthor::load(document, handle)?;
    let display = TagView::new(
        author.tag.clone(),
        author.tag_source.clone(),
        author.tag_status.clone(),
    )
    .display;
    let mut props = vec![
        crate::entities::common::ro_prop(t!("Type").as_ref(), "pid_type", author.type_code),
        Property {
            label: t!("Item tag").into_owned(),
            field: TAG_FIELD,
            value: PropValue::PlainText(display),
        },
        crate::entities::common::ro_prop(
            t!("Tag source").as_ref(),
            "pid_tag_source",
            author.tag_source,
        ),
        crate::entities::common::ro_prop(
            t!("Status").as_ref(),
            "pid_tag_status",
            author.tag_status,
        ),
    ];
    let (values, _) = split_props(&author.props);
    let mut missing: Vec<&str> = Vec::new();
    for name in highlighted {
        if name != super::pidverify::TAG_NAME
            && !values.contains_key(name.as_str())
            && !missing.contains(&name.as_str())
        {
            missing.push(name);
        }
    }
    for (key, value) in values {
        props.push(Property {
            field: props_field(&key),
            label: key,
            value: PropValue::PlainText(value),
        });
    }
    props.extend(missing.into_iter().map(|name| Property {
        field: props_field(name),
        label: name.to_owned(),
        value: PropValue::PlainText(String::new()),
    }));
    Some(PropSection {
        title: PID_AUTHOR_SECTION.to_string(),
        props,
    })
}

impl OpenCADStudio {
    /// Commit a `P&ID 属性` row edit on every target that carries an author
    /// record. Returns `false` when the field is not an author row or no
    /// target carries a record, so the caller can fall through to its other
    /// handlers (the recognizer group shares the `pid_tag` id).
    pub(in crate::app) fn commit_pid_author_property(
        &mut self,
        i: usize,
        handles: &[document::Handle],
        field: &str,
        value: &str,
    ) -> bool {
        let key = match field {
            TAG_FIELD => None,
            _ => match field.strip_prefix(PROPS_FIELD_PREFIX) {
                Some(key) if !key.is_empty() => Some(key.to_string()),
                _ => return false,
            },
        };
        let authored: Vec<(document::Handle, PidAuthor)> = handles
            .iter()
            .filter_map(|handle| {
                PidAuthor::load(&self.tabs[i].scene.document, *handle).map(|a| (*handle, a))
            })
            .collect();
        if authored.is_empty() {
            return false;
        }
        let changed: Vec<(document::Handle, PidAuthor)> = authored
            .into_iter()
            .filter(|(_, author)| match &key {
                None => {
                    // Committing the row unchanged — including the `？` it
                    // shows for a conflict — is not a manual retag.
                    let shown = TagView::new(
                        author.tag.clone(),
                        author.tag_source.clone(),
                        author.tag_status.clone(),
                    )
                    .display;
                    value != author.tag && value != shown
                }
                // A property the record lacks entered empty, as its
                // highlighted row shows it, is left out.
                Some(key) => {
                    split_props(&author.props)
                        .0
                        .get(key)
                        .map_or("", String::as_str)
                        != value
                }
            })
            .collect();
        if changed.is_empty() {
            return true;
        }
        self.push_undo_snapshot(i, "PIDPROP");
        let document = &mut self.tabs[i].scene.document;
        for (handle, mut author) in changed.iter().cloned() {
            match &key {
                None => {
                    author.tag = value.to_string();
                    author.tag_source = TAG_SOURCE_MANUAL.to_string();
                    author.store(document, handle);
                }
                // A pipe run's hand-edited line field must stop taking the
                // line's value, so it gets the override mark; nothing else
                // inherits props, and marking them would only bloat the record.
                Some(key) if author.type_code == "PIPE_RUN" => {
                    let _ = set_prop_manual(document, handle, key, value);
                }
                Some(key) => {
                    let (mut values, marks) = split_props(&author.props);
                    values.insert(key.clone(), value.to_string());
                    author.props = join_props(&values, &marks);
                    author.store(document, handle);
                }
            }
        }
        let edited: Vec<document::Handle> = changed.iter().map(|(handle, _)| *handle).collect();
        let name = key.as_deref().unwrap_or(super::pidverify::TAG_NAME);
        self.pid_verify_highlight_filled(i, &edited, name, value);
        self.refresh_pid_tags(i, &edited);
        // 手工改管路字段之后，管路当前值要立刻下传给它的管段
        // （`VAL-PIPE-010`）；改管线字段同理级联到管路再级联到管段。
        self.propagate_pid_chain(i, &edited);
        self.tabs[i].dirty = true;
        true
    }

    /// `PIDSETPROP [<位号>] <属性> <值>` — set one property of a P&ID object
    /// from the command line, the way a row of the `P&ID 属性` group does:
    /// one undo step, the same carry-down to runs and segments, the same
    /// broadcast (`VAL-CROSS-001`).
    ///
    /// With a tag, the object is the one carrying it (as `PIDTAG` finds a
    /// tag: exactly, else the one spelling of it in another case). Without,
    /// it is what is selected; and with nothing selected, the symbol placed
    /// last on this drawing — so `PIDINSERT` followed by `PIDSETPROP` sets
    /// a property of what was just placed.
    pub(in crate::app) fn dispatch_pidsetprop(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDSETPROP") {
            return None;
        }
        let args: Vec<&str> = words.collect();
        let (tag, key, value) = match args.as_slice() {
            [key, value] => (None, *key, *value),
            [tag, key, value] => (Some(*tag), *key, *value),
            _ => {
                self.command_line.push_error(
                    "PIDSETPROP: 写成 PIDSETPROP <属性> <值>（作用于选中的对象），\
                     或 PIDSETPROP <位号> <属性> <值>。",
                );
                return Some(Task::none());
            }
        };
        let document = &self.tabs[i].scene.document;
        let authored = |handle: &document::Handle| PidAuthor::load(document, *handle);
        let handles: Vec<document::Handle> = match tag {
            Some(tag) => {
                let carrying: Vec<(document::Handle, String)> = document
                    .entities()
                    .filter_map(|entity| {
                        let handle = entity.common().handle;
                        let author = authored(&handle)?;
                        author.tag.eq_ignore_ascii_case(tag).then_some((handle, author.tag))
                    })
                    .collect();
                // The exact spelling wins; otherwise the letters must name
                // one spelling, or it is not known which object is meant.
                let exact: Vec<document::Handle> = carrying
                    .iter()
                    .filter(|(_, spelled)| spelled == tag)
                    .map(|(handle, _)| *handle)
                    .collect();
                let mut spellings: Vec<&String> = carrying.iter().map(|(_, spelled)| spelled).collect();
                spellings.sort();
                spellings.dedup();
                if !exact.is_empty() {
                    exact
                } else if spellings.len() == 1 {
                    carrying.iter().map(|(handle, _)| *handle).collect()
                } else {
                    Vec::new()
                }
            }
            None => {
                let selected: Vec<document::Handle> = self.tabs[i]
                    .scene
                    .selected_handles_in_order()
                    .into_iter()
                    .filter(|handle| authored(handle).is_some())
                    .collect();
                if !selected.is_empty() {
                    selected
                } else {
                    // Nothing selected: the symbol placed last on this
                    // drawing, while it is still there.
                    self.pid_last_placed
                        .filter(|(tab, handle)| *tab == self.tabs[i].id && authored(handle).is_some())
                        .map(|(_, handle)| handle)
                        .into_iter()
                        .collect()
                }
            }
        };
        if handles.is_empty() {
            let why = match tag {
                Some(tag) => format!("没有位号为 {tag} 的对象（或有几种写法）"),
                None => "没有选中带 P&ID 属性的对象，本图上也没有刚放置的符号；先选中一个，或写出位号".to_owned(),
            };
            self.command_line.push_error(&format!("PIDSETPROP: {why}。"));
            return Some(Task::none());
        }
        let named = PidAuthor::load(&self.tabs[i].scene.document, handles[0])
            .map(|author| author.tag)
            .unwrap_or_default();
        let field = format!("{PROPS_FIELD_PREFIX}{key}");
        if !self.commit_pid_author_property(i, &handles, &field, value) {
            self.command_line
                .push_error(&format!("PIDSETPROP: 属性 {key} 没有设置。"));
            return Some(Task::none());
        }
        self.command_line
            .push_info(&format!("PIDSETPROP: {named} 的 {key} = {value}"));
        Some(self.finish_dispatch(cmd))
    }

    /// 属性下传重算：`edited` 里的管线/管路按「管线 → 管路 → 管段」级联。
    ///
    /// 唯一的重算入口。写 `PID_AUTHOR` XDATA 的每条路径（REST `xdata_set`、
    /// 属性面板提交、`PIDPIPE` 创建管路与管段）都在写完自己的记录之后调它，
    /// 因此不存在独立的 propagate op（`architecture.md` A.7）。
    ///
    /// 返回本批实际被改写的实体 handle，供调用方刷新画布与撤销步。调用方
    /// 负责 undo 快照的包围（一次操作一个快照）。
    pub(in crate::app) fn propagate_pid_chain(
        &mut self,
        i: usize,
        edited: &[document::Handle],
    ) -> Vec<document::Handle> {
        if self.tabs[i].is_start {
            return Vec::new();
        }
        let mut written: Vec<document::Handle> = Vec::new();
        {
            let document = &mut self.tabs[i].scene.document;
            let mut table = TopologyTable::load(document);
            table.prune(document);
            // 表里的 `props` / `overrides` 是 XDATA 的副本；编辑刚发生，
            // 先对齐，否则会拿旧值去比、错判成「无变化」。
            table.sync_props_from_document(document);

            let mut lines: Vec<String> = Vec::new();
            let mut runs: Vec<String> = Vec::new();
            for &handle in edited {
                if let Some(tag) = table.line_tag_of_handle(handle) {
                    if !lines.contains(&tag) {
                        lines.push(tag);
                    }
                } else if let Some(tag) = table.run_tag_of_handle(handle) {
                    if !runs.contains(&tag) {
                        runs.push(tag);
                    }
                }
            }

            // 第 1 环：管线 → 管路。
            for line_tag in &lines {
                let Some(line_handle) = table
                    .line_by_tag(line_tag)
                    .and_then(|line| parse_handle(&line.handle))
                else {
                    continue;
                };
                let run_handles = table.run_handles_of(line_tag);
                if run_handles.is_empty() {
                    continue;
                }
                let batch = propagate_line_to_runs(document, line_handle, &run_handles);
                if batch.is_empty() {
                    continue;
                }
                apply(document, &batch);
                for write in &batch.writes {
                    written.push(write.handle);
                    // 刚跟随的管路要接着把值送到自己的管段（第 2 环）。
                    if let Some(tag) = table.run_tag_of_handle(write.handle) {
                        if !runs.contains(&tag) {
                            runs.push(tag);
                        }
                    }
                }
            }

            // 第 2 环：管路 → 管段。管段不直连管线，始终取管路当前值。
            for run_tag in &runs {
                let Some(run_handle) = table
                    .pipe_runs
                    .iter()
                    .find(|run| &run.tag == run_tag)
                    .and_then(|run| parse_handle(&run.handle))
                else {
                    continue;
                };
                let segments = table.segment_handles_of(run_tag);
                if segments.is_empty() {
                    continue;
                }
                let batch = propagate_run_to_segments(document, run_handle, &segments);
                if batch.is_empty() {
                    continue;
                }
                apply(document, &batch);
                written.extend(batch.writes.iter().map(|write| write.handle));
            }

            if !table.is_empty() {
                table.sync_props_from_document(document);
                table.store(document);
            }
        }
        written.sort();
        written.dedup();
        if !written.is_empty() {
            let changes: Vec<_> = written
                .iter()
                .map(|handle| (*handle, crate::scene::ChangeKind::Modified))
                .collect();
            self.tabs[i].scene.bump_entities(&changes);
        }
        written
    }
}

/// 一条管路的手工改值若动了某个下传字段，就给它打上覆盖标记。
///
/// 「手工改」的判据是「新值与旧值不同」而不是「来自哪个入口」：属性面板
/// 提交与 REST `xdata_set` 都是用户手工改值，两者必须同语义
/// （`VAL-PIPE-008`）。下传写入走 `PidAuthor::store`，不经这里，因此不会
/// 被误打标记。
pub(in crate::app) fn mark_manual_run_overrides(
    document: &mut document::Document,
    handle: document::Handle,
    before: &std::collections::BTreeMap<String, String>,
) {
    let Some(author) = PidAuthor::load(document, handle) else {
        return;
    };
    if !author.type_code.eq_ignore_ascii_case(crate::pid::topology::TYPE_PIPE_RUN) {
        return;
    }
    let (props, mut marks) = split_props(&author.props);
    let mut added = false;
    for field in LINE_FIELDS {
        let now = props.get(field);
        let was = before.get(field);
        if now != was && now.is_some() {
            marks.insert(field.to_string());
            added = true;
        }
    }
    if !added {
        return;
    }
    let mut author = author;
    author.props = join_props(&props, &marks);
    author.store(document, handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.control_state();
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        if req.get("request_id").is_none() {
            req["request_id"] = json!(format!("pidprops-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
        }
        let (response, task) = app.control_request(req.clone());
        app.drive_headless_task(task).unwrap();
        if matches!(response["status"].as_str(), Some("accepted" | "running")) {
            app.control_request(json!({"op":"operation","request_id":req["request_id"]})).0
        } else {
            response
        }
    }

    /// A new drawing with one placed vessel carrying two props; its handle.
    fn placed_vessel(app: &mut OpenCADStudio, x: f64) -> document::Handle {
        let started = request(app, json!({"op":"run","cmd":"PIDINSERT vessel"}));
        assert_eq!(started["ok"], true, "{started}");
        let picked = request(
            app,
            json!({"op":"input","kind":"point","point":[x,150.0,0.0],"space":"wcs"}),
        );
        assert_eq!(picked["ok"], true, "{picked}");
        request(app, json!({"op":"input","kind":"enter"}));
        let document = &mut app.tabs[app.active_tab].scene.document;
        let handle = document
            .entities()
            .filter_map(|entity| match entity {
                document::EntityType::Insert(insert) => Some(insert.common.handle),
                _ => None,
            })
            .last()
            .expect("a placement");
        let mut author = PidAuthor::load(document, handle).expect("author record");
        author.props = r#"{"设计压力":"1.6MPa","设计温度":"120℃"}"#.to_string();
        author.store(document, handle);
        handle
    }

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    fn hex(handle: document::Handle) -> String {
        format!("{:X}", handle.value())
    }

    fn select(app: &mut OpenCADStudio, handle: document::Handle) {
        let _ = app.automation_op(r#"{"op":"select","clear":true}"#);
        let selected = request(app, json!({"op":"select","handles":[hex(handle)]}));
        assert_eq!(selected["ok"], true, "{selected}");
    }

    fn author_rows(app: &mut OpenCADStudio) -> Option<Vec<Value>> {
        let read = request(app, json!({"op":"properties"}));
        let sections = read["sections"]
            .as_array()
            .or_else(|| read["result"]["sections"].as_array())
            .cloned()
            .unwrap_or_default();
        sections
            .into_iter()
            .find(|section| section["title"] == PID_AUTHOR_SECTION)
            .map(|section| section["properties"].as_array().cloned().unwrap_or_default())
    }

    fn row_value(rows: &[Value], id: &str) -> Option<String> {
        rows.iter()
            .find(|row| row["id"] == id)
            .and_then(|row| row["value"].as_str().map(str::to_owned))
    }

    fn author(app: &OpenCADStudio, handle: document::Handle) -> PidAuthor {
        PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author record")
    }

    fn set(app: &mut OpenCADStudio, field: &str, value: &str) {
        let done = request(app, json!({"op":"property","field":field,"value":value}));
        assert_eq!(done["ok"], true, "property {field}={value}: {done}");
    }

    /// `VAL-DRAW-024` / `025`: the group shows for an authored symbol only,
    /// with the four fixed rows and one row per props key, each equal to
    /// the stored value.
    #[test]
    fn the_group_shows_the_author_record_and_only_for_authored_entities() {
        let mut app = app();
        let vessel = placed_vessel(&mut app, 200.0);
        select(&mut app, vessel);
        let rows = author_rows(&mut app).expect("P&ID 属性 group for an authored symbol");
        let stored = author(&app, vessel);
        assert_eq!(row_value(&rows, "pid_type").as_deref(), Some(stored.type_code.as_str()));
        assert_eq!(row_value(&rows, "pid_tag").as_deref(), Some(stored.tag.as_str()));
        assert_eq!(row_value(&rows, "pid_tag_source").as_deref(), Some(stored.tag_source.as_str()));
        assert_eq!(row_value(&rows, "pid_tag_status").as_deref(), Some(stored.tag_status.as_str()));
        assert_eq!(row_value(&rows, "pid_props_设计压力").as_deref(), Some("1.6MPa"));
        assert_eq!(row_value(&rows, "pid_props_设计温度").as_deref(), Some("120℃"));
        assert!(rows.len() >= 4 + 2, "rows: {rows:?}");
        assert!(rows.iter().any(|row| row["label"] == "设计压力"));

        let line = app
            .commit_entity_handle(document::EntityType::Line(document::entities::Line::from_points(
                document::types::Vector3::ZERO,
                document::types::Vector3::new(10.0, 0.0, 0.0),
            )))
            .expect("line commits");
        select(&mut app, line);
        assert!(author_rows(&mut app).is_none(), "a plain LINE has no P&ID 属性 group");
    }

    /// `VAL-DRAW-026`: a props edit lands in XDATA, other keys untouched.
    #[test]
    fn a_props_edit_lands_in_xdata_and_leaves_the_other_keys() {
        let mut app = app();
        let vessel = placed_vessel(&mut app, 200.0);
        select(&mut app, vessel);
        set(&mut app, "pid_props_设计压力", "2.5MPa");
        let (values, marks) = split_props(&author(&app, vessel).props);
        assert_eq!(values.get("设计压力").map(String::as_str), Some("2.5MPa"));
        assert_eq!(values.get("设计温度").map(String::as_str), Some("120℃"));
        assert_eq!(values.len(), 2);
        assert!(marks.is_empty(), "equipment props carry no override mark");
        let rows = author_rows(&mut app).unwrap();
        assert_eq!(row_value(&rows, "pid_props_设计压力").as_deref(), Some("2.5MPa"));
    }

    /// `VAL-DRAW-027` / `028`: a tag edit makes the tag manual, relabels the
    /// canvas, and undo / redo move tag, source and label together.
    #[test]
    fn a_tag_edit_is_manual_and_undoes_with_its_label() {
        let mut app = app();
        let vessel = placed_vessel(&mut app, 200.0);
        let before = author(&app, vessel);
        assert_eq!((before.tag.as_str(), before.tag_source.as_str()), ("P-1001", "auto"));
        select(&mut app, vessel);
        set(&mut app, "pid_tag", "P-9001");
        let after = author(&app, vessel);
        assert_eq!(after.tag, "P-9001");
        assert_eq!(after.tag_source, "manual");
        assert_eq!(after.tag_status, "ok");
        assert_eq!(label(&app, vessel).as_deref(), Some("P-9001"));

        let undone = request(&mut app, json!({"op":"undo"}));
        assert_eq!(undone["ok"], true, "{undone}");
        let back = author(&app, vessel);
        assert_eq!((back.tag.as_str(), back.tag_source.as_str()), ("P-1001", "auto"));
        assert_eq!(label(&app, vessel).as_deref(), Some("P-1001"));
        select(&mut app, vessel);
        let rows = author_rows(&mut app).unwrap();
        assert_eq!(row_value(&rows, "pid_tag").as_deref(), Some("P-1001"));

        let redone = request(&mut app, json!({"op":"redo"}));
        assert_eq!(redone["ok"], true, "{redone}");
        let again = author(&app, vessel);
        assert_eq!((again.tag.as_str(), again.tag_source.as_str()), ("P-9001", "manual"));
    }

    /// A tag edited into a clash shows `？` in the group and on the canvas.
    #[test]
    fn a_conflicting_tag_edit_shows_the_conflict_mark_in_the_group() {
        let mut app = app();
        let first = placed_vessel(&mut app, 200.0);
        let second = placed_vessel(&mut app, 400.0);
        select(&mut app, second);
        let taken = author(&app, first).tag;
        set(&mut app, "pid_tag", &taken);
        let edited = author(&app, second);
        assert_eq!(edited.tag, "P-1001");
        assert_eq!(edited.tag_status, "duplicated");
        assert_eq!(author(&app, first).tag_status, "ok");
        let rows = author_rows(&mut app).unwrap();
        assert_eq!(row_value(&rows, "pid_tag").as_deref(), Some("？"));
        assert_eq!(row_value(&rows, "pid_tag_status").as_deref(), Some("duplicated"));
        assert_eq!(label(&app, second).as_deref(), Some("？"));
    }

    fn label(app: &OpenCADStudio, handle: document::Handle) -> Option<String> {
        match app.tabs[app.active_tab].scene.document.get_entity(handle) {
            Some(document::EntityType::Insert(insert)) => insert
                .attributes
                .iter()
                .find(|attribute| attribute.tag == super::super::pidpalette::PID_TAG_ATTRIBUTE)
                .map(|attribute| attribute.value.clone()),
            _ => None,
        }
    }

    // ── 属性下传：属性面板这条通道 ────────────────────────────────────────

    use crate::pid::propagate::{overrides_of, props_of, LINE_FIELDS};
    use crate::pid::topology::{parse_handle, TopologyTable};

    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 3]]) {
        let started = request(app, json!({"op":"run","cmd":cmd}));
        assert_eq!(started["ok"], true, "{cmd} 应启动：{started}");
        for point in points {
            request(
                app,
                json!({"op":"input","kind":"point","point":point,"space":"wcs"}),
            );
        }
        request(app, json!({"op":"input","kind":"enter"}));
    }

    fn field(app: &OpenCADStudio, handle: document::Handle, name: &str) -> String {
        props_of(&app.tabs[app.active_tab].scene.document, handle)
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    fn marks(app: &OpenCADStudio, handle: document::Handle) -> Vec<String> {
        overrides_of(&app.tabs[app.active_tab].scene.document, handle)
            .into_iter()
            .collect()
    }

    /// 属性面板改管路字段也走同一条下传链：打覆盖标记、管线改值不穿透、
    /// 管段跟随管路（`VAL-PIPE-008` / `009` / `010` 的面板通道）。
    ///
    /// 面板只列 `props` 里已有的键，所以先经契约的 REST 写入口
    /// （`PUT …/xdata/PID_AUTHOR` 的等价 op）把管线的初值写上，再用面板改值。
    #[test]
    fn a_run_edit_from_the_property_panel_marks_the_override_and_cascades() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let table = TopologyTable::load(&app.tabs[app.active_tab].scene.document);
        let line = parse_handle(&table.pipe_lines[0].handle).expect("handle");
        put_props(
            &mut app,
            line,
            &[("管道等级", "A1A"), ("公称直径", "DN50")],
        );

        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let table = TopologyTable::load(&app.tabs[app.active_tab].scene.document);
        let run = parse_handle(&table.pipe_runs[0].handle).expect("handle");
        let segment = parse_handle(&table.pipe_segments[0].handle).expect("handle");
        assert_eq!(field(&app, run, "管道等级"), "A1A");
        assert_eq!(field(&app, run, "公称直径"), "DN50");

        // 面板改管路的公称直径。
        select(&mut app, run);
        set(&mut app, "pid_props_公称直径", "DN80");
        assert_eq!(field(&app, run, "公称直径"), "DN80");
        assert_eq!(marks(&app, run), vec!["公称直径".to_string()]);
        assert_eq!(field(&app, segment, "公称直径"), "DN80", "管段跟随管路");

        // 管线改值：被覆盖的字段不穿透，未覆盖的仍下传。
        put_props(&mut app, line, &[("公称直径", "DN150"), ("管道等级", "A2B")]);
        assert_eq!(field(&app, run, "公称直径"), "DN80");
        assert_eq!(field(&app, run, "管道等级"), "A2B");
        assert_eq!(field(&app, segment, "公称直径"), "DN80");
        assert_eq!(field(&app, segment, "管道等级"), "A2B");
        assert!(marks(&app, segment).is_empty(), "管段无覆盖标记");
    }

    /// 契约的 `PUT /api/v1/entities/{h}/xdata/PID_AUTHOR` 等价写法：只发
    /// `props` 时按字段合并，其余八键与未提及的属性键都不丢。
    fn put_props(app: &mut OpenCADStudio, handle: document::Handle, pairs: &[(&str, &str)]) {
        let mut props = serde_json::Map::new();
        for (key, value) in pairs {
            props.insert((*key).to_string(), json!(value));
        }
        let payload = serde_json::Value::Object(props).to_string();
        let done = request(
            app,
            json!({
                "op":"xdata_set",
                "handles":[hex(handle)],
                "app":"PID_AUTHOR",
                "data":[{"code":1000,"value":format!("props={payload}")}],
            }),
        );
        assert_eq!(done["ok"], true, "写 props 应成功：{done}");
    }
}

//! The Properties panel's `P&ID 属性` group: the typed author record
//! (`PID_AUTHOR`) a placed symbol carries, shown and edited in place.
//!
//! Row ids are fixed by the architecture: `pid_type`, `pid_tag`,
//! `pid_tag_source`, `pid_tag_status`, then one `pid_props_<key>` row per
//! `props` key. Only the tag and the props rows are editable; a tag edit makes
//! the tag `manual`. Every commit is one undo step that also re-runs the
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
pub(in crate::app) const PID_EVIDENCE_SECTION: &str = "P&ID 来源证据";

inventory::submit!(crate::command::CommandRegistration {
    names: &[
        "PIDPROMOTE",
        "PIDREVIEW",
        "PIDSAVEPID",
        "PIDCONVERT",
        "PIDOPC",
        "PIDBOM",
        "PIDDRC",
        "PIDFIND",
        "PIDTRACE",
        "PIDDIFF",
        "PIDISOLATE",
        "PIDLOOP",
        "PIDSPEC",
        "PIDPACKAGE",
        "PIDRENUM",
        "PIDHAZOP",
        "PIDUTILITY",
        "PIDVALVE",
        "PIDNOZZLE",
        "PIDINDEX",
        "PIDSPECIALTY",
        "PIDLINELIST",
        "PIDEQUIP",
        "PIDRELIEF",
        "PIDCAUSE",
        "PIDIO"
    ]
});

const TAG_FIELD: &str = "pid_tag";
const PROPS_FIELD_PREFIX: &str = "pid_props_";

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

/// The `P&ID 属性` group (plus read-only `P&ID 来源证据` when promoted/imported)
/// for `handle`, or empty when it carries no `PID_AUTHOR` record (`L2-1 M4` / `#136`).
pub(in crate::app) fn pid_author_section(
    document: &document::Document,
    handle: document::Handle,
) -> Vec<PropSection> {
    let resolved_owner = PidAuthor::load(document, handle).or_else(|| {
        crate::io::pid_convert::resolve_owning_author_handles(document, &[handle])
            .first()
            .and_then(|&oh| PidAuthor::load(document, oh))
    });
    let Some(author) = resolved_owner else {
        return Vec::new();
    };
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
    for (key, value) in &values {
        if key.starts_with('_') {
            continue;
        }
        props.push(Property {
            field: props_field(key),
            label: key.clone(),
            value: PropValue::PlainText(value.clone()),
        });
    }
    let mut sections = vec![PropSection {
        title: PID_AUTHOR_SECTION.to_string(),
        props,
    }];

    if values.contains_key("_source") || values.contains_key("_sppid_class") {
        let mut ev_props = Vec::new();
        for (label, field_id, key) in [
            ("来源", "pid_ev_source", "_source"),
            ("置信度", "pid_ev_confidence", "_confidence"),
            ("置信度分档", "pid_ev_tier", "_confidence_tier"),
            ("SPPID 类", "pid_ev_sppid_class", "_sppid_class"),
            ("变体编码", "pid_ev_variant_id", "_variant_id"),
            ("符号路径", "pid_ev_symbol_path", "_symbol_path"),
            ("GraphicOID", "pid_ev_graphic_oid", "_graphic_oid"),
        ] {
            if let Some(val) = values.get(key).filter(|v| !v.is_empty()) {
                ev_props.push(crate::entities::common::ro_prop(
                    label,
                    field_id,
                    val.clone(),
                ));
            }
        }
        if !ev_props.is_empty() {
            sections.push(PropSection {
                title: PID_EVIDENCE_SECTION.to_string(),
                props: ev_props,
            });
        }
    }
    sections
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
                Some(key) if !key.is_empty() && !key.starts_with('_') => Some(key.to_string()),
                _ => return false,
            },
        };
        let owner_handles = crate::io::pid_convert::resolve_owning_author_handles(
            &self.tabs[i].scene.document,
            handles,
        );
        let authored: Vec<(document::Handle, PidAuthor)> = owner_handles
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
                Some(key) => {
                    split_props(&author.props).0.get(key).map(String::as_str) != Some(value)
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
        self.refresh_pid_tags(i, &edited);
        let synced_texts =
            crate::io::pid_convert::sync_paired_canvas_tag_texts(&mut self.tabs[i].scene.document);
        if !synced_texts.is_empty() {
            let text_changes: Vec<_> = synced_texts
                .into_iter()
                .map(|handle| (handle, crate::scene::ChangeKind::Modified))
                .collect();
            self.tabs[i].scene.bump_entities(&text_changes);
        }
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
    pub(in crate::app) fn dispatch_pidsetprop(
        &mut self,
        cmd: &str,
        i: usize,
    ) -> Option<Task<Message>> {
        if let Some(t) = self.dispatch_pidpromote_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidreview_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidsavepid_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidconvert_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidopc_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidbom_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_piddrc_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidfind_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidtrace_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_piddiff_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidisolate_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidloop_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidspec_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidpackage_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidrenum_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidhazop_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidutility_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidvalve_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidnozzle_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidindex_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidspecialty_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidlinelist_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidequip_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidrelief_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidcause_cmd(cmd, i) {
            return Some(t);
        }
        if let Some(t) = self.dispatch_pidio_cmd(cmd, i) {
            return Some(t);
        }
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
                        author
                            .tag
                            .eq_ignore_ascii_case(tag)
                            .then_some((handle, author.tag))
                    })
                    .collect();
                // The exact spelling wins; otherwise the letters must name
                // one spelling, or it is not known which object is meant.
                let exact: Vec<document::Handle> = carrying
                    .iter()
                    .filter(|(_, spelled)| spelled == tag)
                    .map(|(handle, _)| *handle)
                    .collect();
                let mut spellings: Vec<&String> =
                    carrying.iter().map(|(_, spelled)| spelled).collect();
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
                        .filter(|(tab, handle)| {
                            *tab == self.tabs[i].id && authored(handle).is_some()
                        })
                        .map(|(_, handle)| handle)
                        .into_iter()
                        .collect()
                }
            }
        };
        if handles.is_empty() {
            let why = match tag {
                Some(tag) => format!("没有位号为 {tag} 的对象（或有几种写法）"),
                None => {
                    "没有选中带 P&ID 属性的对象，本图上也没有刚放置的符号；先选中一个，或写出位号"
                        .to_owned()
                }
            };
            self.command_line
                .push_error(&format!("PIDSETPROP: {why}。"));
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

    fn dispatch_pidpromote_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDPROMOTE") {
            return None;
        }
        let mode = words.next().unwrap_or("ALL").to_ascii_uppercase();
        self.push_undo_snapshot(i, "PIDPROMOTE");
        let selected: std::collections::BTreeSet<document::Handle> = self.tabs[i]
            .scene
            .selected_handles_in_order()
            .into_iter()
            .collect();

        let report = match mode.as_str() {
            "SEMANTICS" => {
                let filter = (!selected.is_empty()).then_some(&selected);
                crate::io::pid_convert::promote_semantics_to_author(
                    &mut self.tabs[i].scene.document,
                    filter,
                )
            }
            "SELECTED" if !selected.is_empty() => {
                crate::io::pid_convert::promote_semantics_to_author(
                    &mut self.tabs[i].scene.document,
                    Some(&selected),
                )
            }
            _ if crate::io::pid_convert::has_native_pid_semantics(&self.tabs[i].scene.document) => {
                crate::io::pid_convert::promote_semantics_to_author(
                    &mut self.tabs[i].scene.document,
                    None,
                )
            }
            _ => {
                let opts = crate::io::pid_convert::ConvertOptions::default();
                match crate::io::pid_convert::promote_sheet_to_author(
                    &mut self.tabs[i].scene.document,
                    &opts,
                ) {
                    Ok(mut rep) => {
                        if rep.newly_promoted == 0 && rep.updated_in_place == 0 {
                            rep = crate::io::pid_convert::promote_semantics_to_author(
                                &mut self.tabs[i].scene.document,
                                None,
                            );
                        }
                        rep
                    }
                    Err(err) => {
                        self.command_line.push_error(&format!("PIDPROMOTE: {err}"));
                        return Some(Task::none());
                    }
                }
            }
        };

        let authored_handles: Vec<(document::Handle, crate::scene::ChangeKind)> = self.tabs[i]
            .scene
            .document
            .model_space_entities()
            .filter_map(|ent| {
                let h = ent.common().handle;
                PidAuthor::load(&self.tabs[i].scene.document, h)
                    .map(|_| (h, crate::scene::ChangeKind::Modified))
            })
            .collect();
        if !authored_handles.is_empty() {
            self.tabs[i].scene.bump_entities(&authored_handles);
        }
        self.tabs[i].dirty = true;
        self.refresh_properties();
        let rev = crate::io::pid_convert::summarize_author_review(&self.tabs[i].scene.document);
        self.command_line.push_output(&format!(
            "PIDPROMOTE: 新增升格 {} 个，原位更新 {} 个，保护手工位号 {} 个；置信度 high={} medium={} low={}（管段={}）。",
            report.newly_promoted,
            report.updated_in_place,
            report.manual_tags_preserved,
            rev.high,
            rev.medium,
            rev.low,
            report.pipe_segments_promoted,
        ));
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidreview_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDREVIEW") {
            return None;
        }
        let action = words.next().unwrap_or("SUMMARY").to_ascii_uppercase();
        match action.as_str() {
            "SUMMARY" => {
                let rev =
                    crate::io::pid_convert::summarize_author_review(&self.tabs[i].scene.document);
                self.command_line.push_output(&format!(
                    "PIDREVIEW: 总符号={} | high={} medium={} low={} confirmed={} | 手工位号={} 重号冲突={}",
                    rev.total_authored_symbols,
                    rev.high,
                    rev.medium,
                    rev.low,
                    rev.confirmed,
                    rev.manual_tags,
                    rev.duplicated_tags,
                ));
            }
            "HIGH" | "MEDIUM" | "MED" | "LOW" | "CONFIRMED" | "DUPLICATES" | "DUP" | "ALL" => {
                let matched = crate::io::pid_convert::filter_author_handles(
                    &self.tabs[i].scene.document,
                    &action,
                );
                let count = matched.len();
                let expanded = crate::io::pid_convert::expand_author_member_handles(
                    &self.tabs[i].scene.document,
                    &matched,
                );
                self.tabs[i].scene.deselect_all();
                if !expanded.is_empty() {
                    self.tabs[i].scene.select_entities(&expanded);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDREVIEW {action}: 已选中 {count} 个符号（共 {} 个图元）。",
                    expanded.len()
                ));
            }
            "DEDUP" => {
                self.push_undo_snapshot(i, "PIDREVIEW_DEDUP");
                let renamed = crate::io::pid_convert::deduplicate_author_tags(
                    &mut self.tabs[i].scene.document,
                );
                let synced_texts = crate::io::pid_convert::sync_paired_canvas_tag_texts(
                    &mut self.tabs[i].scene.document,
                );
                if renamed > 0 || !synced_texts.is_empty() {
                    let changes: Vec<_> = self.tabs[i]
                        .scene
                        .document
                        .model_space_entities()
                        .filter_map(|e| {
                            let h = e.common().handle;
                            (PidAuthor::load(&self.tabs[i].scene.document, h).is_some()
                                || synced_texts.contains(&h))
                            .then_some((h, crate::scene::ChangeKind::Modified))
                        })
                        .collect();
                    if !changes.is_empty() {
                        self.tabs[i].scene.bump_entities(&changes);
                    }
                    self.tabs[i].dirty = true;
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDREVIEW DEDUP: 已自动消解 {renamed} 个重复位号冲突。"
                ));
            }
            "ACCEPT" => {
                let selected = self.tabs[i].scene.selected_handles_in_order();
                if selected.is_empty() {
                    self.command_line.push_error(
                        "PIDREVIEW ACCEPT: 请先选中要确认的对象（如先运行 PIDREVIEW MEDIUM）。",
                    );
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "PIDREVIEW_ACCEPT");
                let accepted = crate::io::pid_convert::accept_author_handles(
                    &mut self.tabs[i].scene.document,
                    &selected,
                );
                let changes: Vec<_> = selected
                    .iter()
                    .map(|&h| (h, crate::scene::ChangeKind::Modified))
                    .collect();
                self.tabs[i].scene.bump_entities(&changes);
                self.tabs[i].dirty = true;
                self.refresh_properties();
                self.command_line
                    .push_output(&format!("PIDREVIEW ACCEPT: 已人工确认 {accepted} 个对象。"));
            }
            "REJECT" => {
                let selected = self.tabs[i].scene.selected_handles_in_order();
                if selected.is_empty() {
                    self.command_line
                        .push_error("PIDREVIEW REJECT: 请先选中要剔除语义的对象。");
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "PIDREVIEW_REJECT");
                let (removed, sync) = crate::io::pid_convert::reject_author_handles(
                    &mut self.tabs[i].scene.document,
                    &selected,
                );
                let changes: Vec<_> = selected
                    .iter()
                    .map(|&h| (h, crate::scene::ChangeKind::Modified))
                    .collect();
                self.tabs[i].scene.bump_entities(&changes);
                self.tabs[i].dirty = true;
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDREVIEW REJECT: 已移除 {} 个对象的 P&ID 语义（清理拓扑记录 {} 条、悬挂端点 {} 处）。",
                    removed, sync.pruned_records, sync.cleared_dangling_peers
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDREVIEW: 未知子命令 {other}（支持 SUMMARY / HIGH / MEDIUM / LOW / DUPLICATES / ACCEPT / REJECT / DEDUP）。"
                ));
                return Some(Task::none());
            }
        }
        Some(self.finish_dispatch(cmd))
    }

    #[cfg(target_arch = "wasm32")]
    fn dispatch_pidsavepid_cmd(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDSAVEPID") {
            return None;
        }
        self.command_line
            .push_error("PIDSAVEPID: Web 版本不支持直接写出本地 .pid 文件。");
        Some(Task::none())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch_pidsavepid_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDSAVEPID") {
            return None;
        }
        let mut target_opt: Option<std::path::PathBuf> = None;
        let mut seed_opt: Option<String> = None;
        let mut symdir_opt: Option<String> = None;
        let mut props_opt: Option<String> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            if tok.eq_ignore_ascii_case("SEED") {
                if let Some(val) = tokens.get(idx + 1) {
                    seed_opt = Some(val.clone());
                    idx += 2;
                    continue;
                }
            } else if tok.eq_ignore_ascii_case("SYMDIR") {
                if let Some(val) = tokens.get(idx + 1) {
                    symdir_opt = Some(val.clone());
                    idx += 2;
                    continue;
                }
            } else if tok.eq_ignore_ascii_case("PROPS") {
                if let Some(val) = tokens.get(idx + 1) {
                    props_opt = Some(val.clone());
                    idx += 2;
                    continue;
                }
            } else if target_opt.is_none() {
                target_opt = Some(std::path::PathBuf::from(tok));
            }
            idx += 1;
        }

        let target_pid = match target_opt.or_else(|| self.pid_source(i)) {
            Some(mut p) => {
                if p.extension().is_none() {
                    p.set_extension("pid");
                }
                p
            }
            None => {
                self.command_line.push_error(
                    "PIDSAVEPID: 请指定输出路径，例如 PIDSAVEPID \"out.pid\" [SEED \"seed.pid\"]。",
                );
                return Some(Task::none());
            }
        };

        if let Err(err) = crate::pid::propagate::gate_save(&self.tabs[i].scene.document) {
            self.command_line.push_error(&format!(
                "PIDSAVEPID: 存在重复位号冲突（{}），已阻断保存：{err}",
                err.code()
            ));
            return Some(Task::none());
        }

        let _ = crate::io::pid_convert::sync_author_after_edit(&mut self.tabs[i].scene.document);
        let fallback_seed = seed_opt
            .or_else(|| std::env::var("OCS_PID_SEED_ZIP").ok())
            .or_else(|| self.pid_source(i).map(|p| p.to_string_lossy().into_owned()));
        let doc_stem = target_pid
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let out_dir = target_pid
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        if let Err(err) = std::fs::create_dir_all(&out_dir) {
            self.command_line.push_error(&format!(
                "PIDSAVEPID: 无法创建目录 {}: {err}",
                out_dir.display()
            ));
            return Some(Task::none());
        }

        let opts = crate::io::pid_convert::ConvertOptions {
            seed_pid_path: fallback_seed,
            sym_library_dir: symdir_opt.or_else(|| std::env::var("OCS_PID_SYM_DIR").ok()),
            external_props_path: props_opt,
            verify_plot_pdf: false,
            ..Default::default()
        };
        let dual = match crate::io::pid_convert::save_author_document_to_dual(
            &self.tabs[i].scene.document,
            &doc_stem,
            &opts,
        ) {
            Ok(b) => b,
            Err(err) => {
                self.command_line
                    .push_error(&format!("PIDSAVEPID: 存回失败：{err}"));
                return Some(Task::none());
            }
        };

        let Some(ref pid_bytes) = dual.pid_bytes else {
            self.command_line.push_error(
                "PIDSAVEPID: 未配置种子模板 .pid（可通过 SEED 参数或 OCS_PID_SEED_ZIP 指定）。",
            );
            return Some(Task::none());
        };
        if let Err(err) = std::fs::write(&target_pid, pid_bytes) {
            self.command_line
                .push_error(&format!("PIDSAVEPID: 写入 .pid 失败：{err}"));
            return Some(Task::none());
        }
        let data_xml_path = out_dir.join(format!("{doc_stem}_Data.xml"));
        let meta_xml_path = out_dir.join(format!("{doc_stem}_Meta.xml"));
        let cov_path = out_dir.join(format!("{doc_stem}_Coverage.json"));
        let _ = std::fs::write(&data_xml_path, &dual.publish.data_xml);
        let _ = std::fs::write(&meta_xml_path, &dual.publish.meta_xml);
        let _ = std::fs::write(&cov_path, &dual.publish.coverage_json);

        if self.tabs[i].current_path.as_deref() == Some(target_pid.as_path()) {
            self.tabs[i].dirty = false;
        }
        self.command_line.push_output(&format!(
            "PIDSAVEPID: 已写出 {}（{} 字节，对象={}，关系={}）及同名 _Data.xml / _Meta.xml。",
            target_pid.display(),
            pid_bytes.len(),
            dual.report.total_items_written,
            dual.report.total_relations_written,
        ));
        Some(self.finish_dispatch(cmd))
    }

    #[cfg(target_arch = "wasm32")]
    fn dispatch_pidconvert_cmd(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDCONVERT") {
            return None;
        }
        self.command_line
            .push_error("PIDCONVERT: Web 版本不支持直接写出本地转换产物。");
        Some(Task::none())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch_pidconvert_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDCONVERT") {
            return None;
        }
        if tokens.len() == 1 {
            self.command_line.push_info(
                "PIDCONVERT: 用法 PIDCONVERT \"<输出目录|输出.pid|HERE>\" [STEM <主干>] [PLANT <装置>] [SEED \"<种子.pid|zip>\"] [DUAL] [SYMDIR \"<目录>\"] [CLASSMAP \"<json>\"] [VERIFY]，或 PIDCONVERT BATCH \"<输入路径>\" \"<输出目录>\" [SUMMARY \"<json>\"] [...]",
            );
            return Some(Task::none());
        }
        if tokens
            .get(1)
            .is_some_and(|tok| tok.eq_ignore_ascii_case("BATCH"))
        {
            return self.dispatch_pidconvert_batch(cmd, &tokens[2..]);
        }
        self.dispatch_pidconvert_single(cmd, i, &tokens[1..])
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch_pidconvert_single(
        &mut self,
        cmd: &str,
        i: usize,
        rest: &[String],
    ) -> Option<Task<Message>> {
        let args = match parse_pidconvert_args(rest) {
            Ok(a) => a,
            Err(err) => {
                self.command_line.push_error(&format!("PIDCONVERT: {err}"));
                return Some(Task::none());
            }
        };
        let Some(target_raw) = args.positional.first() else {
            self.command_line
                .push_error("PIDCONVERT: 请指定输出目录、输出 .pid 路径或 HERE。");
            return Some(Task::none());
        };

        let (out_dir, inferred_stem, target_wants_pid) = if target_raw.eq_ignore_ascii_case("HERE")
        {
            let Some(ref cur) = self.tabs[i].current_path else {
                self.command_line
                    .push_error("PIDCONVERT HERE: 当前图纸未关联本地路径，请显式指定输出目录。");
                return Some(Task::none());
            };
            let dir = cur
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf();
            let st = cur.file_stem().and_then(|s| s.to_str()).map(str::to_string);
            (dir, st, false)
        } else {
            let target_p = std::path::PathBuf::from(target_raw);
            let ext = target_p.extension().and_then(|s| s.to_str()).unwrap_or("");
            if ext.eq_ignore_ascii_case("pid") {
                let dir = target_p
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf();
                let st = target_p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                (dir, st, true)
            } else if ext.eq_ignore_ascii_case("xml") {
                let dir = target_p
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf();
                let raw_st = target_p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("SHEET");
                let clean_st = raw_st
                    .strip_suffix("_Data")
                    .or_else(|| raw_st.strip_suffix("_Meta"))
                    .unwrap_or(raw_st)
                    .to_string();
                (dir, Some(clean_st), false)
            } else {
                let st = self.tabs[i]
                    .current_path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
                (target_p, st, false)
            }
        };

        let doc_stem = args
            .stem
            .or(inferred_stem)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "SHEET".to_string());

        let seed_pid_path = if args.seed.is_some() {
            args.seed
        } else if target_wants_pid || args.force_dual {
            match std::env::var("OCS_PID_SEED_ZIP")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| self.pid_source(i).map(|p| p.to_string_lossy().into_owned()))
            {
                Some(s) => Some(s),
                None => {
                    self.command_line.push_error(
                        "PIDCONVERT: 写出 .pid 需要指定 SEED \"<种子.pid|zip>\" 或配置 OCS_PID_SEED_ZIP。",
                    );
                    return Some(Task::none());
                }
            }
        } else {
            None
        };

        let wants_dual = seed_pid_path.is_some();
        let opts = crate::io::pid_convert::ConvertOptions {
            plant: args.plant.unwrap_or_else(|| "P01".to_string()),
            seed_pid_path,
            sym_library_dir: args
                .symdir
                .or_else(|| std::env::var("OCS_PID_SYM_DIR").ok()),
            class_map_path: args.classmap,
            external_props_path: args.props,
            verify_plot_pdf: args.verify && wants_dual,
            ..Default::default()
        };

        let use_authored = !args.raw
            && crate::io::pid_convert::summarize_author_review(&self.tabs[i].scene.document)
                .total_authored_symbols
                > 0;

        let report = if use_authored {
            if let Err(err) = crate::pid::propagate::gate_save(&self.tabs[i].scene.document) {
                self.command_line.push_error(&format!(
                    "PIDCONVERT: 存在重复位号冲突（{}），已阻断导出：{err}",
                    err.code()
                ));
                return Some(Task::none());
            }
            let mut synced_doc = self.tabs[i].scene.document.clone();
            let _ = crate::io::pid_convert::sync_author_after_edit(&mut synced_doc);
            match crate::io::pid_convert::export_author_document_to_dir(
                &synced_doc,
                &doc_stem,
                &out_dir,
                &opts,
                args.verify,
            ) {
                Ok(rep) => rep,
                Err(err) => {
                    self.command_line
                        .push_error(&format!("PIDCONVERT: 导出失败：{err}"));
                    return Some(Task::none());
                }
            }
        } else {
            match crate::io::pid_convert::export_document_to_dir(
                &self.tabs[i].scene.document,
                &doc_stem,
                &out_dir,
                &opts,
                args.verify,
            ) {
                Ok(rep) => rep,
                Err(err) => {
                    self.command_line
                        .push_error(&format!("PIDCONVERT: 导出失败：{err}"));
                    return Some(Task::none());
                }
            }
        };

        let exact_cnt = report.variant_resolution.exact_variant_id
            + report.variant_resolution.symbol_path_match;
        let fb_cnt = report.variant_resolution.class_default_fallback
            + report.variant_resolution.unmapped_class_variant;
        self.command_line.push_output(&format!(
            "PIDCONVERT: 已导出 {} -> {}（对象={}，关系={}，精确变体={}，兜底={}，双产物={}，自检={}）。",
            doc_stem,
            out_dir.display(),
            report.total_items_written,
            report.total_relations_written,
            exact_cnt,
            fb_cnt,
            wants_dual,
            args.verify,
        ));
        Some(self.finish_dispatch(cmd))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch_pidconvert_batch(&mut self, cmd: &str, rest: &[String]) -> Option<Task<Message>> {
        let args = match parse_pidconvert_args(rest) {
            Ok(a) => a,
            Err(err) => {
                self.command_line
                    .push_error(&format!("PIDCONVERT BATCH: {err}"));
                return Some(Task::none());
            }
        };
        if args.positional.len() < 2 {
            self.command_line.push_error(
                "PIDCONVERT BATCH: 用法 PIDCONVERT BATCH \"<输入文件或目录>\" \"<输出目录>\" [PLANT <装置>] [SEED \"<种子>\"] [DUAL] [SYMDIR \"<目录>\"] [CLASSMAP \"<json>\"] [VERIFY] [SUMMARY \"<json>\"]。",
            );
            return Some(Task::none());
        }
        let in_path = std::path::PathBuf::from(&args.positional[0]);
        let out_dir = std::path::PathBuf::from(&args.positional[1]);

        let seed_pid_path = if args.seed.is_some() {
            args.seed
        } else if args.force_dual {
            match std::env::var("OCS_PID_SEED_ZIP")
                .ok()
                .filter(|s| !s.trim().is_empty())
            {
                Some(s) => Some(s),
                None => {
                    self.command_line.push_error(
                        "PIDCONVERT BATCH: 指定 DUAL 时需提供 SEED 参数或配置 OCS_PID_SEED_ZIP。",
                    );
                    return Some(Task::none());
                }
            }
        } else {
            None
        };

        let wants_dual = seed_pid_path.is_some();
        let opts = crate::io::pid_convert::ConvertOptions {
            plant: args.plant.unwrap_or_else(|| "P01".to_string()),
            seed_pid_path,
            sym_library_dir: args
                .symdir
                .or_else(|| std::env::var("OCS_PID_SYM_DIR").ok()),
            class_map_path: args.classmap,
            external_props_path: args.props,
            verify_plot_pdf: args.verify && wants_dual,
            ..Default::default()
        };

        let summary = match crate::io::pid_convert::convert_path_to_dir(&in_path, &out_dir, &opts) {
            Ok(s) => s,
            Err(err) => {
                self.command_line
                    .push_error(&format!("PIDCONVERT BATCH: {err}"));
                return Some(Task::none());
            }
        };

        if summary.files_converted == 0 || summary.files_failed > 0 {
            self.command_line.push_error(&format!(
                "PIDCONVERT BATCH: 转换存在失败或无有效图纸（成功={}，失败={}）：{:?}",
                summary.files_converted, summary.files_failed, summary.errors
            ));
            return Some(Task::none());
        }

        if args.verify {
            match crate::io::pid_convert::verify_batch_dir_strict(&out_dir, &opts.plant, &summary) {
                Ok(()) => {}
                Err(err) => {
                    self.command_line
                        .push_error(&format!("PIDCONVERT BATCH VERIFY: {err}"));
                    return Some(Task::none());
                }
            }
        }

        if let Some(ref sjson) = args.summary {
            if let Some(parent) = sjson.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            match serde_json::to_string_pretty(&summary)
                .map_err(|e| e.to_string())
                .and_then(|txt| {
                    std::fs::write(sjson, format!("{txt}\n")).map_err(|e| e.to_string())
                }) {
                Ok(()) => {}
                Err(err) => {
                    self.command_line.push_error(&format!(
                        "PIDCONVERT BATCH: 写出汇总 JSON {} 失败：{err}",
                        sjson.display()
                    ));
                    return Some(Task::none());
                }
            }
        }

        let exact_cnt = summary.variant_resolution.exact_variant_id
            + summary.variant_resolution.symbol_path_match;
        let fb_cnt = summary.variant_resolution.class_default_fallback
            + summary.variant_resolution.unmapped_class_variant;
        self.command_line.push_output(&format!(
            "PIDCONVERT BATCH: 成功转换 {} 张图纸（.pid={}，PDF={}，对象={}，关系={}，精确变体={}，兜底={}）-> {}。",
            summary.files_converted,
            summary.pid_files_written,
            summary.pdf_files_plotted,
            summary.total_items_written,
            summary.total_relations_written,
            exact_cnt,
            fb_cnt,
            out_dir.display(),
        ));
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidopc_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{analyze_open_sheets_opc, OpcSheetInput, OpcStatus};

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDOPC") {
            return None;
        }
        let action = tokens
            .get(1)
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_else(|| "SUMMARY".to_string());

        let tab_names: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_start)
            .map(|(ti, t)| {
                let nm = t
                    .current_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| t.tab_title.clone());
                (ti, nm)
            })
            .collect();
        let inputs: Vec<OpcSheetInput<'_>> = tab_names
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();
        let rep = analyze_open_sheets_opc(&inputs);

        match action.as_str() {
            "SUMMARY" => {
                let here_total = rep.connectors.iter().filter(|c| c.tab_index == i).count();
                let here_paired = rep
                    .connectors
                    .iter()
                    .filter(|c| c.tab_index == i && c.status == OpcStatus::Paired)
                    .count();
                self.command_line.push_output(&format!(
                    "PIDOPC: 打开图纸={} | 总连接符={}（互逆配对={} 对，单向断链={}，外部未打开={}，缺目标图号={}）| 当前图连接符={}（已配对={}）。",
                    rep.open_sheets,
                    rep.total_connectors,
                    rep.reciprocal_pairs,
                    rep.one_way_open,
                    rep.external_unopened,
                    rep.missing_target,
                    here_total,
                    here_paired,
                ));
            }
            "ALL" | "PAIRED" | "UNPAIRED" | "BROKEN" => {
                let mut count = 0usize;
                let mut handles = Vec::new();
                for c in rep.connectors.iter().filter(|c| c.tab_index == i) {
                    let keep = match action.as_str() {
                        "ALL" => true,
                        "PAIRED" => c.status == OpcStatus::Paired,
                        _ => matches!(c.status, OpcStatus::OneWayOpen | OpcStatus::MissingTarget),
                    };
                    if !keep {
                        continue;
                    }
                    count += 1;
                    for &mh in &c.member_handles {
                        if !handles.contains(&mh) {
                            handles.push(mh);
                        }
                    }
                }
                self.tabs[i].scene.deselect_all();
                if !handles.is_empty() {
                    self.tabs[i].scene.select_entities(&handles);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDOPC {action}: 已选中 {count} 个页间连接符（共 {} 个图元）。",
                    handles.len()
                ));
            }
            "JUMP" => {
                let here_conns: Vec<_> =
                    rep.connectors.iter().filter(|c| c.tab_index == i).collect();
                let chosen = if let Some(q) = tokens.get(2) {
                    let qup = q.trim().to_ascii_uppercase();
                    if let Ok(ord) = qup.parse::<usize>() {
                        ord.checked_sub(1).and_then(|k| here_conns.get(k).copied())
                    } else {
                        here_conns.into_iter().find(|c| {
                            c.primary_handle_hex.eq_ignore_ascii_case(&qup)
                                || c.target_dwg_raw
                                    .as_deref()
                                    .is_some_and(|t| t.to_ascii_uppercase().contains(&qup))
                                || c.line_label
                                    .as_deref()
                                    .is_some_and(|l| l.to_ascii_uppercase().contains(&qup))
                        })
                    }
                } else {
                    let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[i]
                        .scene
                        .selected_entities()
                        .into_iter()
                        .map(|(h, _)| h)
                        .collect();
                    here_conns
                        .into_iter()
                        .find(|c| c.member_handles.iter().any(|h| sel_set.contains(h)))
                };

                let Some(conn) = chosen else {
                    self.command_line.push_error(
                        "PIDOPC JUMP: 请先在本图选中一个页间连接符（如先运行 PIDOPC PAIRED）。",
                    );
                    return Some(Task::none());
                };

                if let Some(ref peer) = conn.peer {
                    let peer_tab = peer.tab_index;
                    let peer_handles = peer.member_handles.clone();
                    let peer_x = peer.x;
                    let peer_y = peer.y;
                    let peer_dwg = peer.drawing_code.clone();

                    if peer_tab < self.tabs.len() && peer_tab != self.active_tab {
                        let _ = self.update(Message::TabSwitch(peer_tab));
                        self.active_tab = peer_tab;
                    }
                    if peer_tab < self.tabs.len() {
                        self.tabs[peer_tab].scene.deselect_all();
                        if !peer_handles.is_empty() {
                            self.tabs[peer_tab].scene.select_entities(&peer_handles);
                        }
                        let zoomed = self.tabs[peer_tab].scene.zoom_to_entities(&peer_handles);
                        if !zoomed {
                            let mut cam = self.tabs[peer_tab].scene.camera.borrow_mut();
                            let z = cam.target.z;
                            cam.target = glam::DVec3::new(peer_x, peer_y, z);
                        }
                        self.refresh_properties();
                    }
                    self.command_line.push_output(&format!(
                        "PIDOPC JUMP: 已跳转至对端图纸 {peer_dwg}（坐标 {:.1}, {:.1}）。",
                        peer_x, peer_y
                    ));
                } else if conn.target_open_tab.is_some() {
                    self.command_line.push_error(
                        "PIDOPC JUMP: 目标图纸已在标签页中打开，但缺少指回本图的互逆页间连接符（单向断链）。",
                    );
                    return Some(Task::none());
                } else {
                    let tgt = conn.target_dwg_raw.as_deref().unwrap_or("<未标注图号>");
                    self.command_line.push_error(&format!(
                        "PIDOPC JUMP: 目标图纸 {tgt} 当前未在标签页中打开。"
                    ));
                    return Some(Task::none());
                }
            }
            "EXPORT" => {
                #[cfg(target_arch = "wasm32")]
                {
                    self.command_line
                        .push_error("PIDOPC EXPORT: Web 版本不支持直接写出本地报告文件。");
                    return Some(Task::none());
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let Some(out_raw) = tokens.get(2) else {
                        self.command_line
                            .push_error("PIDOPC EXPORT: 用法 PIDOPC EXPORT \"<输出.json>\"。");
                        return Some(Task::none());
                    };
                    let out_p = std::path::PathBuf::from(out_raw);
                    if let Some(parent) = out_p.parent().filter(|p| !p.as_os_str().is_empty()) {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match serde_json::to_string_pretty(&rep)
                        .map_err(|e| e.to_string())
                        .and_then(|s| std::fs::write(&out_p, format!("{s}\n")).map_err(|e| e.to_string()))
                    {
                        Ok(()) => self.command_line.push_output(&format!(
                            "PIDOPC EXPORT: 已导出跨标签页连接符报告 -> {}（总连接符={}，互逆配对={} 对）。",
                            out_p.display(),
                            rep.total_connectors,
                            rep.reciprocal_pairs,
                        )),
                        Err(err) => {
                            self.command_line
                                .push_error(&format!("PIDOPC EXPORT: {err}"));
                            return Some(Task::none());
                        }
                    }
                }
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDOPC: 未知子命令 {other}（支持 SUMMARY / ALL / PAIRED / UNPAIRED / JUMP / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidbom_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{extract_bom_from_documents, BomSheetInput, ConvertOptions};

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDBOM") {
            return None;
        }

        let mut all_tabs = false;
        let mut plant = "P01".to_string();
        let mut class_map_opt: Option<String> = None;
        let mut out_path_opt: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            if tok.eq_ignore_ascii_case("ALLTABS") || tok.eq_ignore_ascii_case("ALL") {
                all_tabs = true;
                idx += 1;
            } else if tok.eq_ignore_ascii_case("SUMMARY") {
                idx += 1;
            } else if tok.eq_ignore_ascii_case("PLANT") {
                if let Some(val) = tokens.get(idx + 1) {
                    plant = val.clone();
                    idx += 2;
                } else {
                    self.command_line
                        .push_error("PIDBOM: PLANT 后需指定装置代号。");
                    return Some(Task::none());
                }
            } else if tok.eq_ignore_ascii_case("CLASSMAP") {
                if let Some(val) = tokens.get(idx + 1) {
                    class_map_opt = Some(val.clone());
                    idx += 2;
                } else {
                    self.command_line
                        .push_error("PIDBOM: CLASSMAP 后需指定规则文件路径。");
                    return Some(Task::none());
                }
            } else if out_path_opt.is_none() {
                out_path_opt = Some(std::path::PathBuf::from(tok));
                idx += 1;
            } else {
                self.command_line
                    .push_error(&format!("PIDBOM: 无法识别的参数 {tok}。"));
                return Some(Task::none());
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<BomSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| BomSheetInput {
                drawing_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();
        let opts = ConvertOptions {
            plant,
            class_map_path: class_map_opt,
            ..Default::default()
        };

        let report = match extract_bom_from_documents(&sheet_inputs, &opts) {
            Ok(r) => r,
            Err(err) => {
                self.command_line.push_error(&format!("PIDBOM: {err}"));
                return Some(Task::none());
            }
        };

        if let Some(out_p) = out_path_opt {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = out_p;
                self.command_line
                    .push_error("PIDBOM: Web 版本不支持直接写出本地清册文件。");
                return Some(Task::none());
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Err(err) = crate::io::pid_convert::write_bom_schedule_file(&report, &out_p) {
                    self.command_line.push_error(&format!("PIDBOM: {err}"));
                    return Some(Task::none());
                }
                self.command_line.push_output(&format!(
                    "PIDBOM: 已导出清册 -> {}（扫描图纸={}，去重总条目={} | 设备={} 管线={} 仪表={} 阀门={}）。",
                    out_p.display(),
                    report.summary.sheets_scanned,
                    report.summary.total_unique_rows,
                    report.summary.equipment_count,
                    report.summary.pipeline_count,
                    report.summary.instrument_count,
                    report.summary.valve_count,
                ));
            }
        } else {
            self.command_line.push_output(&format!(
                "PIDBOM: 扫描图纸={} | 去重总条目={}（设备={}，管线={}，仪表={}，阀门={}）。",
                report.summary.sheets_scanned,
                report.summary.total_unique_rows,
                report.summary.equipment_count,
                report.summary.pipeline_count,
                report.summary.instrument_count,
                report.summary.valve_count,
            ));
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_piddrc_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            audit_sheet_drc, DrcFinding, DrcKind, DrcSeverity, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDDRC") {
            return None;
        }
        let action = tokens
            .get(1)
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_else(|| "SUMMARY".to_string());

        let tab_names: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.is_start)
            .map(|(ti, t)| {
                let nm = t
                    .current_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| t.tab_title.clone());
                (ti, nm)
            })
            .collect();
        let inputs: Vec<OpcSheetInput<'_>> = tab_names
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();
        let rep = audit_sheet_drc(i, &inputs);

        let filter_pred = |f: &DrcFinding, flt: &str| -> bool {
            match flt {
                "ALL" => true,
                "ERRORS" | "ERROR" => f.severity == DrcSeverity::Error,
                "WARNINGS" | "WARNING" | "WARN" => f.severity == DrcSeverity::Warning,
                "DUPLICATES" | "DUP" => f.kind == DrcKind::DuplicateTag,
                "ONEWAYOPC" | "OPC" => f.kind == DrcKind::OneWayOpc,
                "ORPHANNOZZLES" | "NOZZLES" => f.kind == DrcKind::OrphanNozzle,
                "DEADENDS" | "DEADEND" => f.kind == DrcKind::OpenPipeDeadEnd,
                "UNTAGGED" => f.kind == DrcKind::UntaggedComponent,
                _ => false,
            }
        };

        match action.as_str() {
            "SUMMARY" => {
                let s = &rep.summary;
                self.command_line.push_output(&format!(
                    "PIDDRC: 可导出={} | 总项={}（错误={}：重号={} 单向连接符={} | 警告={}：孤立管口={} 悬空管端={} 无位号组件={}）。",
                    s.ready_for_export,
                    s.total_findings,
                    s.error_count,
                    s.duplicate_tags,
                    s.one_way_opcs,
                    s.warning_count,
                    s.orphan_nozzles,
                    s.open_pipe_dead_ends,
                    s.untagged_components,
                ));
            }
            "ALL" | "ERRORS" | "ERROR" | "WARNINGS" | "WARNING" | "WARN" | "DUPLICATES" | "DUP"
            | "ONEWAYOPC" | "OPC" | "ORPHANNOZZLES" | "NOZZLES" | "DEADENDS" | "DEADEND"
            | "UNTAGGED" => {
                let mut count = 0usize;
                let mut handles = Vec::new();
                for f in &rep.findings {
                    if !filter_pred(f, action.as_str()) {
                        continue;
                    }
                    count += 1;
                    for &mh in &f.member_handles {
                        if !handles.contains(&mh) {
                            handles.push(mh);
                        }
                    }
                }
                self.tabs[i].scene.deselect_all();
                if !handles.is_empty() {
                    self.tabs[i].scene.select_entities(&handles);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDDRC {action}: 已选中 {count} 处待核查项（共 {} 个图元）。",
                    handles.len()
                ));
            }
            "NEXT" => {
                let sub_flt = tokens
                    .get(2)
                    .map(|s| s.to_ascii_uppercase())
                    .unwrap_or_else(|| "ALL".to_string());
                let pool: Vec<&DrcFinding> = rep
                    .findings
                    .iter()
                    .filter(|f| filter_pred(f, sub_flt.as_str()))
                    .collect();
                if pool.is_empty() {
                    self.command_line
                        .push_output(&format!("PIDDRC NEXT {sub_flt}: 当前无匹配待核查项。"));
                    return Some(self.finish_dispatch(cmd));
                }

                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = pool.iter().position(|f| {
                    sel_set.len() == f.member_handles.len()
                        && f.member_handles.iter().all(|h| sel_set.contains(h))
                });
                let next_idx = cur_pos.map(|p| (p + 1) % pool.len()).unwrap_or(0);
                let target = pool[next_idx];
                let target_handles = target.member_handles.clone();
                let tx = target.x;
                let ty = target.y;
                let msg = target.message.clone();

                self.tabs[i].scene.deselect_all();
                if !target_handles.is_empty() {
                    self.tabs[i].scene.select_entities(&target_handles);
                }
                let zoomed = self.tabs[i].scene.zoom_to_entities(&target_handles);
                if !zoomed {
                    let mut cam = self.tabs[i].scene.camera.borrow_mut();
                    let z = cam.target.z;
                    cam.target = glam::DVec3::new(tx, ty, z);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDDRC NEXT [{}/{}]: {msg}",
                    next_idx + 1,
                    pool.len()
                ));
            }
            "EXPORT" => {
                #[cfg(target_arch = "wasm32")]
                {
                    self.command_line
                        .push_error("PIDDRC EXPORT: Web 版本不支持直接写出本地报告文件。");
                    return Some(Task::none());
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let Some(out_raw) = tokens.get(2) else {
                        self.command_line
                            .push_error("PIDDRC EXPORT: 用法 PIDDRC EXPORT \"<输出.json>\"。");
                        return Some(Task::none());
                    };
                    let out_p = std::path::PathBuf::from(out_raw);
                    if let Some(parent) = out_p.parent().filter(|p| !p.as_os_str().is_empty()) {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match serde_json::to_string_pretty(&rep)
                        .map_err(|e| e.to_string())
                        .and_then(|s| {
                            std::fs::write(&out_p, format!("{s}\n")).map_err(|e| e.to_string())
                        }) {
                        Ok(()) => self.command_line.push_output(&format!(
                            "PIDDRC EXPORT: 已导出体检报告 -> {}（错误={}，警告={}）。",
                            out_p.display(),
                            rep.summary.error_count,
                            rep.summary.warning_count,
                        )),
                        Err(err) => {
                            self.command_line
                                .push_error(&format!("PIDDRC EXPORT: {err}"));
                            return Some(Task::none());
                        }
                    }
                }
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDDRC: 未知子命令 {other}（支持 SUMMARY / ALL / ERRORS / WARNINGS / DUPLICATES / ORPHANNOZZLES / DEADENDS / UNTAGGED / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidfind_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{search_pid_entities, OpcSheetInput};

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDFIND") {
            return None;
        }

        let mut all_tabs = false;
        let mut query_opt: Option<String> = None;
        for tok in tokens.iter().skip(1) {
            if tok.eq_ignore_ascii_case("ALLTABS") || tok.eq_ignore_ascii_case("ALL") {
                all_tabs = true;
            } else if tok.eq_ignore_ascii_case("NEXT") {
                // Stateless cycle advancement is enabled automatically when re-running PIDFIND
            } else if query_opt.is_none() {
                query_opt = Some(tok.clone());
            }
        }

        let Some(query) = query_opt.filter(|q| !q.trim().is_empty()) else {
            self.command_line
                .push_error("PIDFIND: 用法 PIDFIND \"<位号或线号关键字>\" [ALLTABS] [NEXT]。");
            return Some(Task::none());
        };

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();
        let hits = search_pid_entities(&query, i, &sheet_inputs);
        if hits.is_empty() {
            self.command_line
                .push_error(&format!("PIDFIND: 未找到匹配 \"{query}\" 的符号或管线。"));
            return Some(Task::none());
        }

        let cur_tab = self.active_tab;
        let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
            .scene
            .selected_entities()
            .into_iter()
            .map(|(h, _)| h)
            .collect();
        let cur_pos = hits.iter().position(|h| {
            h.tab_index == cur_tab
                && !sel_set.is_empty()
                && sel_set.len() == h.member_handles.len()
                && h.member_handles.iter().all(|mh| sel_set.contains(mh))
        });
        let next_idx = cur_pos.map(|p| (p + 1) % hits.len()).unwrap_or(0);
        let chosen = &hits[next_idx];
        let dest_tab = chosen.tab_index;
        let dest_handles = chosen.member_handles.clone();
        let dx = chosen.x;
        let dy = chosen.y;
        let mtag = chosen.matched_tag.clone();
        let cls = chosen.sppid_class.clone();
        let dwg = chosen.drawing_name.clone();

        if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
            let _ = self.update(Message::TabSwitch(dest_tab));
            self.active_tab = dest_tab;
        }
        if dest_tab < self.tabs.len() {
            self.tabs[dest_tab].scene.deselect_all();
            if !dest_handles.is_empty() {
                self.tabs[dest_tab].scene.select_entities(&dest_handles);
            }
            let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(&dest_handles);
            if !zoomed {
                let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                let z = cam.target.z;
                cam.target = glam::DVec3::new(dx, dy, z);
            }
            self.refresh_properties();
        }
        self.command_line.push_output(&format!(
            "PIDFIND [{}/{}]: 已定位 {mtag}（{cls}，图元={}，图纸={dwg}，坐标 {:.1}, {:.1}）。",
            next_idx + 1,
            hits.len(),
            dest_handles.len(),
            dx,
            dy,
        ));
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidtrace_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            trace_shortest_path_by_query, trace_subnet_by_query, trace_subnet_from_handles,
            OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDTRACE") {
            return None;
        }

        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;
        let mut positional: Vec<String> = Vec::new();
        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            if tok.eq_ignore_ascii_case("ALLTABS") || tok.eq_ignore_ascii_case("ALL") {
                all_tabs = true;
                idx += 1;
            } else if tok.eq_ignore_ascii_case("EXPORT") {
                let Some(next_tok) = tokens.get(idx + 1) else {
                    self.command_line
                        .push_error("PIDTRACE EXPORT: 缺少输出 JSON 文件路径。");
                    return Some(Task::none());
                };
                export_path = Some(std::path::PathBuf::from(next_tok));
                idx += 2;
            } else if tok.eq_ignore_ascii_case("TO") && positional.len() == 1 {
                idx += 1;
            } else {
                positional.push(tok.clone());
                idx += 1;
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let trace_res = match positional.len() {
            0 => {
                let sel_handles: Vec<document::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                trace_subnet_from_handles(i, &sel_handles, all_tabs, &sheet_inputs)
            }
            1 => trace_subnet_by_query(&positional[0], i, all_tabs, &sheet_inputs),
            _ => trace_shortest_path_by_query(
                &positional[0],
                &positional[1],
                i,
                all_tabs,
                &sheet_inputs,
            ),
        };

        let rep = match trace_res {
            Ok(r) => r,
            Err(err) => {
                self.command_line.push_error(&format!("PIDTRACE: {err}"));
                return Some(Task::none());
            }
        };

        if let Some(ref out_p) = export_path {
            if let Some(parent) = out_p.parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(parent);
            }
            match serde_json::to_string_pretty(&rep)
                .map_err(|e| e.to_string())
                .and_then(|s| std::fs::write(out_p, format!("{s}\n")).map_err(|e| e.to_string()))
            {
                Ok(()) => {}
                Err(err) => {
                    self.command_line
                        .push_error(&format!("PIDTRACE EXPORT: {err}"));
                    return Some(Task::none());
                }
            }
        }

        if rep.reachable {
            for (&ti, handles) in &rep.tab_handles {
                if ti < self.tabs.len() {
                    self.tabs[ti].scene.deselect_all();
                    if !handles.is_empty() {
                        self.tabs[ti].scene.select_entities(handles);
                    }
                }
            }
            let dest_tab = rep.focus_tab;
            if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                let _ = self.update(Message::TabSwitch(dest_tab));
                self.active_tab = dest_tab;
            }
            if dest_tab < self.tabs.len() {
                let zoomed = rep.tab_handles.get(&dest_tab).is_some_and(|hs| {
                    !hs.is_empty() && self.tabs[dest_tab].scene.zoom_to_entities(hs)
                });
                if !zoomed {
                    let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                    let z = cam.target.z;
                    cam.target = glam::DVec3::new(rep.focus_xy.0, rep.focus_xy.1, z);
                }
                self.refresh_properties();
            }
            self.command_line.push_output(&format!(
                "PIDTRACE: 连通追踪完成（跨图纸={}，穿图OPC={}，节点={}，管段={}，设备={}，阀门={}，仪表={}）。",
                rep.crossed_tabs,
                rep.opc_bridges_traversed,
                rep.total_nodes_visited,
                rep.pipe_segments_count,
                rep.equipment_tags.len(),
                rep.valve_tags.len(),
                rep.instrument_tags.len(),
            ));
        } else {
            self.command_line
                .push_output("PIDTRACE: 起点与终点之间不存在连通工艺路径（reachable=false）。");
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_piddiff_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            diff_pid_documents, write_pid_diff_file, ConvertOptions, DiffKind,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDDIFF") {
            return None;
        }

        let mut base_spec: Option<String> = None;
        let mut action: Option<String> = None;
        let mut export_path: Option<std::path::PathBuf> = None;
        let mut plant = "P01".to_string();
        let mut classmap: Option<String> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            if tok.eq_ignore_ascii_case("EXPORT") {
                let Some(next_tok) = tokens.get(idx + 1) else {
                    self.command_line
                        .push_error("PIDDIFF EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                    return Some(Task::none());
                };
                export_path = Some(std::path::PathBuf::from(next_tok));
                idx += 2;
            } else if tok.eq_ignore_ascii_case("PLANT") {
                if let Some(p) = tokens.get(idx + 1) {
                    plant = p.clone();
                    idx += 2;
                } else {
                    idx += 1;
                }
            } else if tok.eq_ignore_ascii_case("CLASSMAP") {
                if let Some(cm) = tokens.get(idx + 1) {
                    classmap = Some(cm.clone());
                    idx += 2;
                } else {
                    idx += 1;
                }
            } else if matches!(
                tok.to_ascii_uppercase().as_str(),
                "SUMMARY" | "ALL" | "ADDED" | "MODIFIED" | "NEXT"
            ) && base_spec.is_some()
            {
                action = Some(tok.to_ascii_uppercase());
                idx += 1;
            } else if base_spec.is_none() {
                base_spec = Some(tok.clone());
                idx += 1;
            } else {
                action = Some(tok.to_ascii_uppercase());
                idx += 1;
            }
        }

        let Some(base_raw) = base_spec.filter(|s| !s.trim().is_empty()) else {
            self.command_line.push_error(
                "PIDDIFF: 用法 PIDDIFF \"<基准图纸路径或已打开标签页>\" [SUMMARY | ALL | ADDED | MODIFIED | NEXT] [EXPORT \"<.json|.csv|.tsv>\"]。",
            );
            return Some(Task::none());
        };

        let curr_name = self.tabs[i]
            .current_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| self.tabs[i].tab_title.clone());

        let matched_tab = self
            .tabs
            .iter()
            .enumerate()
            .filter(|&(ti, t)| ti != i && !t.is_start)
            .find_map(|(ti, t)| {
                let file_nm = t
                    .current_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let title_match = t.tab_title.eq_ignore_ascii_case(&base_raw)
                    || file_nm.eq_ignore_ascii_case(&base_raw);
                title_match.then_some((
                    ti,
                    if !file_nm.is_empty() {
                        file_nm.to_string()
                    } else {
                        t.tab_title.clone()
                    },
                ))
            });

        let opts = ConvertOptions {
            plant,
            class_map_path: classmap,
            ..Default::default()
        };

        let rep_res = if let Some((bi, b_nm)) = matched_tab {
            diff_pid_documents(
                &b_nm,
                &self.tabs[bi].scene.document,
                &curr_name,
                &self.tabs[i].scene.document,
                &opts,
            )
        } else {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let bp = std::path::Path::new(&base_raw);
                match crate::io::load_file(bp) {
                    Ok(b_doc) => {
                        let b_nm = bp.file_name().and_then(|s| s.to_str()).unwrap_or(&base_raw);
                        diff_pid_documents(
                            b_nm,
                            &b_doc,
                            &curr_name,
                            &self.tabs[i].scene.document,
                            &opts,
                        )
                    }
                    Err(err) => Err(format!("无法加载基准图纸 {base_raw}：{err}")),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                Err(format!("未找到已打开的基准标签页 {base_raw}。"))
            }
        };

        let rep = match rep_res {
            Ok(r) => r,
            Err(err) => {
                self.command_line.push_error(&format!("PIDDIFF: {err}"));
                return Some(Task::none());
            }
        };

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_diff_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDDIFF EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDDIFF: 基准={} vs 当前={} | 总变更={}（新增={}，修改={}，删除={}，未变={}）。",
                    rep.summary.baseline_drawing,
                    rep.summary.current_drawing,
                    rep.summary.total_changes,
                    rep.summary.added_count,
                    rep.summary.modified_count,
                    rep.summary.removed_count,
                    rep.summary.unchanged_count,
                ));
            }
            "ALL" | "ADDED" | "MODIFIED" => {
                let mut handles = Vec::new();
                for it in &rep.items {
                    let keep = match eff_action {
                        "ADDED" => it.kind == DiffKind::Added,
                        "MODIFIED" => it.kind == DiffKind::Modified,
                        _ => it.kind != DiffKind::Removed,
                    };
                    if keep {
                        for &mh in &it.member_handles {
                            if !handles.contains(&mh) {
                                handles.push(mh);
                            }
                        }
                    }
                }
                self.tabs[i].scene.deselect_all();
                if !handles.is_empty() {
                    self.tabs[i].scene.select_entities(&handles);
                    let _ = self.tabs[i].scene.zoom_to_entities(&handles);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDDIFF {eff_action}: 已高亮选中当前图纸 {} 个变更图元。",
                    handles.len()
                ));
            }
            "NEXT" => {
                let candidates: Vec<_> = rep
                    .items
                    .iter()
                    .filter(|it| !it.member_handles.is_empty())
                    .collect();
                if candidates.is_empty() {
                    self.command_line
                        .push_output("PIDDIFF NEXT: 当前图纸上没有可定位的新增或修改图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[i]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = candidates.iter().position(|it| {
                    !sel_set.is_empty()
                        && sel_set.len() == it.member_handles.len()
                        && it.member_handles.iter().all(|mh| sel_set.contains(mh))
                });
                let next_idx = cur_pos.map(|p| (p + 1) % candidates.len()).unwrap_or(0);
                let chosen = candidates[next_idx];
                self.tabs[i].scene.deselect_all();
                self.tabs[i].scene.select_entities(&chosen.member_handles);
                let zoomed = self.tabs[i].scene.zoom_to_entities(&chosen.member_handles);
                if !zoomed {
                    if let (Some(dx), Some(dy)) = (chosen.x, chosen.y) {
                        let mut cam = self.tabs[i].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(dx, dy, z);
                    }
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDDIFF NEXT [{}/{}]: {} {}（{}）。",
                    next_idx + 1,
                    candidates.len(),
                    chosen.kind.as_str(),
                    chosen.item_tag,
                    chosen.category,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDDIFF: 未知子命令 {other}（支持 SUMMARY / ALL / ADDED / MODIFIED / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidisolate_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            solve_pid_isolation_by_query, solve_pid_isolation_from_handles,
            write_pid_isolation_file, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDISOLATE") {
            return None;
        }

        let mut all_tabs = false;
        let mut action: Option<String> = None;
        let mut export_path: Option<std::path::PathBuf> = None;
        let mut query_opt: Option<String> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            if up == "ALLTABS" {
                all_tabs = true;
                idx += 1;
            } else if up == "EXPORT" {
                let Some(next_tok) = tokens.get(idx + 1) else {
                    self.command_line
                        .push_error("PIDISOLATE EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                    return Some(Task::none());
                };
                export_path = Some(std::path::PathBuf::from(next_tok));
                idx += 2;
            } else if matches!(
                up.as_str(),
                "SUMMARY" | "VALVES" | "ENVELOPE" | "HAZARDS" | "ALL" | "NEXT"
            ) {
                action = Some(up);
                idx += 1;
            } else if query_opt.is_none() {
                query_opt = Some(tok.clone());
                idx += 1;
            } else {
                action = Some(up);
                idx += 1;
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let iso_res = if let Some(ref q) = query_opt {
            solve_pid_isolation_by_query(q, i, all_tabs, &sheet_inputs)
        } else {
            let sel_handles: Vec<document::Handle> = self.tabs[i]
                .scene
                .selected_entities()
                .into_iter()
                .map(|(h, _)| h)
                .collect();
            solve_pid_isolation_from_handles(i, &sel_handles, all_tabs, &sheet_inputs)
        };

        let rep = match iso_res {
            Ok(r) => r,
            Err(err) => {
                self.command_line.push_error(&format!("PIDISOLATE: {err}"));
                return Some(Task::none());
            }
        };

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_isolation_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDISOLATE EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDISOLATE: 目标={} | 可完全隔离={} | 跨图纸={} | 首道隔离阀={} | 包络管段={} | 无阀风险={}（直连设备={}，未配对OPC={}，敞口端={}）。",
                    rep.summary.target_tag,
                    rep.summary.fully_isolatable,
                    rep.summary.crossed_tabs,
                    rep.summary.boundary_valves_count,
                    rep.summary.envelope_pipe_segments,
                    rep.summary.hazard_count,
                    rep.summary.unvalved_peer_equipment,
                    rep.summary.unvalved_external_opcs,
                    rep.summary.unvalved_open_dead_ends,
                ));
            }
            "VALVES" | "HAZARDS" | "ENVELOPE" | "ALL" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                if matches!(eff_action, "ENVELOPE" | "ALL") {
                    for (&ti, hs) in &rep.envelope_handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                if matches!(eff_action, "VALVES" | "ALL") {
                    for v in &rep.boundary_valves {
                        let dst = by_tab.entry(v.tab_index).or_default();
                        for &h in &v.member_handles {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                if matches!(eff_action, "HAZARDS" | "ALL") {
                    for hz in &rep.hazards {
                        let dst = by_tab.entry(hz.tab_index).or_default();
                        for &h in &hz.member_handles {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }

                for (ti, _) in &target_tabs {
                    if *ti < self.tabs.len() {
                        self.tabs[*ti].scene.deselect_all();
                    }
                }
                let mut total_sel = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        total_sel += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDISOLATE {eff_action}: 已跨打开图纸高亮选中 {total_sel} 个图元。"
                ));
            }
            "NEXT" => {
                struct StepTarget {
                    tab_index: usize,
                    handles: Vec<document::Handle>,
                    x: f64,
                    y: f64,
                    desc: String,
                }
                let mut steps = Vec::new();
                for v in &rep.boundary_valves {
                    if !v.member_handles.is_empty() {
                        steps.push(StepTarget {
                            tab_index: v.tab_index,
                            handles: v.member_handles.clone(),
                            x: v.x,
                            y: v.y,
                            desc: format!("valve {} ({})", v.valve_tag, v.drawing_name),
                        });
                    }
                }
                for hz in &rep.hazards {
                    if !hz.member_handles.is_empty() {
                        steps.push(StepTarget {
                            tab_index: hz.tab_index,
                            handles: hz.member_handles.clone(),
                            x: hz.x,
                            y: hz.y,
                            desc: format!("hazard {} ({})", hz.kind.as_str(), hz.drawing_name),
                        });
                    }
                }
                if steps.is_empty() {
                    self.command_line
                        .push_output("PIDISOLATE NEXT: 没有可巡检的隔离阀或风险点。");
                    return Some(self.finish_dispatch(cmd));
                }

                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = steps.iter().position(|st| {
                    st.tab_index == cur_tab
                        && !sel_set.is_empty()
                        && sel_set.len() == st.handles.len()
                        && st.handles.iter().all(|h| sel_set.contains(h))
                });
                let next_idx = cur_pos.map(|p| (p + 1) % steps.len()).unwrap_or(0);
                let chosen = &steps[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab].scene.deselect_all();
                    self.tabs[dest_tab].scene.select_entities(&chosen.handles);
                    let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(&chosen.handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDISOLATE NEXT [{}/{}]: {}。",
                    next_idx + 1,
                    steps.len(),
                    chosen.desc,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDISOLATE: 未知子命令 {other}（支持 SUMMARY / VALVES / ENVELOPE / HAZARDS / ALL / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidloop_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_loops, loop_matches_query, write_pid_loops_file, LoopStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDLOOP") {
            return None;
        }

        let mut all_tabs = false;
        let mut action: Option<String> = None;
        let mut export_path: Option<std::path::PathBuf> = None;
        let mut query_opt: Option<String> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            if up == "ALLTABS" {
                all_tabs = true;
                idx += 1;
            } else if up == "EXPORT" {
                let Some(next_tok) = tokens.get(idx + 1) else {
                    self.command_line
                        .push_error("PIDLOOP EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                    return Some(Task::none());
                };
                export_path = Some(std::path::PathBuf::from(next_tok));
                idx += 2;
            } else if matches!(
                up.as_str(),
                "SUMMARY" | "INCOMPLETE" | "COMPLETE" | "ALL" | "NEXT"
            ) {
                action = Some(up);
                idx += 1;
            } else if query_opt.is_none() {
                query_opt = Some(tok.clone());
                idx += 1;
            } else {
                action = Some(up);
                idx += 1;
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_loops(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_loops_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDLOOP EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let default_action = if query_opt.is_some() && export_path.is_none() {
            "ALL"
        } else {
            "SUMMARY"
        };
        let eff_action = action.as_deref().unwrap_or(default_action);

        let filtered_loops: Vec<_> = rep
            .loops
            .iter()
            .filter(|lp| {
                let q_ok = query_opt
                    .as_deref()
                    .is_none_or(|q| loop_matches_query(lp, q));
                let st_ok = match eff_action {
                    "INCOMPLETE" => lp.status == LoopStatus::Incomplete,
                    "COMPLETE" => lp.status == LoopStatus::CompleteClosedLoop,
                    _ => true,
                };
                q_ok && st_ok
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDLOOP: 扫描图纸={} | 归组仪表={} | 总回路={}（跨图回路={}，完整闭环={}，监测回路={}，残缺异常={}）。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_instruments_grouped,
                    rep.summary.total_loops,
                    rep.summary.multi_sheet_loops,
                    rep.summary.complete_closed_loops,
                    rep.summary.monitoring_loops,
                    rep.summary.incomplete_loops,
                ));
            }
            "ALL" | "INCOMPLETE" | "COMPLETE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for lp in &filtered_loops {
                    for mem in &lp.members {
                        let dst = by_tab.entry(mem.tab_index).or_default();
                        for &h in &mem.member_handles {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for (ti, _) in &target_tabs {
                    if *ti < self.tabs.len() {
                        self.tabs[*ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDLOOP {eff_action}: 匹配 {} 个回路，跨打开图纸高亮 {} 个图元。",
                    filtered_loops.len(),
                    sel_cnt
                ));
            }
            "NEXT" => {
                let members: Vec<_> = filtered_loops
                    .iter()
                    .flat_map(|lp| lp.members.iter().map(move |m| (lp.loop_number.as_str(), m)))
                    .filter(|(_, m)| !m.member_handles.is_empty())
                    .collect();
                if members.is_empty() {
                    self.command_line
                        .push_output("PIDLOOP NEXT: 未找到可巡检的回路仪表图元。");
                    return Some(self.finish_dispatch(cmd));
                }

                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = members.iter().position(|(_, m)| {
                    m.tab_index == cur_tab
                        && !sel_set.is_empty()
                        && sel_set.len() == m.member_handles.len()
                        && m.member_handles.iter().all(|h| sel_set.contains(h))
                });
                let next_idx = cur_pos.map(|p| (p + 1) % members.len()).unwrap_or(0);
                let (lp_no, chosen) = members[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab].scene.deselect_all();
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDLOOP NEXT [{}/{}]: 回路 {lp_no} -> {}（{}）。",
                    next_idx + 1,
                    members.len(),
                    chosen.item_tag,
                    chosen.drawing_name,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDLOOP: 未知子命令 {other}（支持 SUMMARY / INCOMPLETE / COMPLETE / ALL / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidspec_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            audit_pid_spec_consistency, write_pid_spec_file, OpcSheetInput, SpecFindingKind,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDSPEC") {
            return None;
        }

        let mut all_tabs = false;
        let mut action: Option<String> = None;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            if up == "ALLTABS" {
                all_tabs = true;
                idx += 1;
            } else if up == "EXPORT" {
                let Some(next_tok) = tokens.get(idx + 1) else {
                    self.command_line
                        .push_error("PIDSPEC EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                    return Some(Task::none());
                };
                export_path = Some(std::path::PathBuf::from(next_tok));
                idx += 2;
            } else {
                action = Some(up);
                idx += 1;
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = audit_pid_spec_consistency(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_spec_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDSPEC EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDSPEC: 扫描图纸={} | 带线号管段={} | 规格问题={}（异径缺变径器={}，跨等级无分界={}，跨图OPC失配={}）。",
                    rep.summary.sheets_scanned,
                    rep.summary.labeled_pipe_segments,
                    rep.summary.total_findings,
                    rep.summary.missing_reducers,
                    rep.summary.unmarked_spec_breaks,
                    rep.summary.opc_spec_mismatches,
                ));
            }
            "ALL" | "REDUCERS" | "SPECBREAKS" | "OPCMISMATCH" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for f in &rep.findings {
                    let keep = match eff_action {
                        "REDUCERS" => f.kind == SpecFindingKind::MissingReducer,
                        "SPECBREAKS" => f.kind == SpecFindingKind::UnmarkedSpecBreak,
                        "OPCMISMATCH" => f.kind == SpecFindingKind::OpcSpecMismatch,
                        _ => true,
                    };
                    if !keep {
                        continue;
                    }
                    let dst = by_tab.entry(f.tab_index).or_default();
                    for &h in &f.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                    if let Some((pti, ref phs)) = f.peer_tab_handles {
                        let pdst = by_tab.entry(pti).or_default();
                        for &ph in phs {
                            if !pdst.contains(&ph) {
                                pdst.push(ph);
                            }
                        }
                    }
                }
                for (ti, _) in &target_tabs {
                    if *ti < self.tabs.len() {
                        self.tabs[*ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDSPEC {eff_action}: 已跨打开图纸高亮选中 {sel_cnt} 个规格问题图元。"
                ));
            }
            "NEXT" => {
                let cands: Vec<_> = rep
                    .findings
                    .iter()
                    .filter(|f| !f.member_handles.is_empty())
                    .collect();
                if cands.is_empty() {
                    self.command_line
                        .push_output("PIDSPEC NEXT: 未检出管道规格或管径失配问题。");
                    return Some(self.finish_dispatch(cmd));
                }

                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = cands.iter().position(|f| {
                    f.tab_index == cur_tab
                        && !sel_set.is_empty()
                        && sel_set.len() == f.member_handles.len()
                        && f.member_handles.iter().all(|h| sel_set.contains(h))
                });
                let next_idx = cur_pos.map(|p| (p + 1) % cands.len()).unwrap_or(0);
                let chosen = cands[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab].scene.deselect_all();
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDSPEC NEXT [{}/{}]: {} -> {}。",
                    next_idx + 1,
                    cands.len(),
                    chosen.kind.as_str(),
                    chosen.message,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDSPEC: 未知子命令 {other}（支持 SUMMARY / ALL / REDUCERS / SPECBREAKS / OPCMISMATCH / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    #[cfg(target_arch = "wasm32")]
    fn dispatch_pidpackage_cmd(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDPACKAGE") {
            return None;
        }
        self.command_line
            .push_error("PIDPACKAGE: Web 版本不支持直接写出本地交付包。");
        Some(Task::none())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch_pidpackage_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            export_handover_package_from_sheets, ConvertOptions, PackageOptions, PackageSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDPACKAGE") {
            return None;
        }

        let mut out_dir_opt: Option<std::path::PathBuf> = None;
        let mut all_tabs = false;
        let mut plant = "P01".to_string();
        let mut seed: Option<String> = None;
        let mut force_dual = false;
        let mut symdir: Option<String> = None;
        let mut classmap: Option<String> = None;
        let mut props: Option<String> = None;
        let mut verify = false;
        let mut strict_gate = false;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "DUAL" => {
                    force_dual = true;
                    idx += 1;
                }
                "VERIFY" => {
                    verify = true;
                    idx += 1;
                }
                "STRICT" => {
                    strict_gate = true;
                    idx += 1;
                }
                "PLANT" => {
                    let Some(v) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDPACKAGE PLANT: 缺少装置名称。");
                        return Some(Task::none());
                    };
                    plant = v.clone();
                    idx += 2;
                }
                "SEED" => {
                    let Some(v) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDPACKAGE SEED: 缺少种子文件路径。");
                        return Some(Task::none());
                    };
                    seed = Some(v.clone());
                    idx += 2;
                }
                "SYMDIR" => {
                    let Some(v) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDPACKAGE SYMDIR: 缺少符号库目录路径。");
                        return Some(Task::none());
                    };
                    symdir = Some(v.clone());
                    idx += 2;
                }
                "CLASSMAP" => {
                    let Some(v) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDPACKAGE CLASSMAP: 缺少映射规则文件路径。");
                        return Some(Task::none());
                    };
                    classmap = Some(v.clone());
                    idx += 2;
                }
                "PROPS" => {
                    let Some(v) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDPACKAGE PROPS: 缺少外置工程表路径。");
                        return Some(Task::none());
                    };
                    props = Some(v.clone());
                    idx += 2;
                }
                _ => {
                    if out_dir_opt.is_none() {
                        out_dir_opt = Some(std::path::PathBuf::from(tok));
                    }
                    idx += 1;
                }
            }
        }

        let Some(out_dir) = out_dir_opt else {
            self.command_line.push_error(
                "PIDPACKAGE: 用法 PIDPACKAGE \"<输出目录>\" [ALLTABS] [PLANT <装置>] [SEED \"<种子>\"] [SYMDIR \"<目录>\"] [CLASSMAP \"<json>\"] [PROPS \"<外置表>\"] [VERIFY] [STRICT]。",
            );
            return Some(Task::none());
        };

        let seed_pid_path = if seed.is_some() {
            seed
        } else if force_dual {
            std::env::var("OCS_PID_SEED_ZIP")
                .ok()
                .filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        let wants_dual = seed_pid_path.is_some();

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<PackageSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| PackageSheetInput {
                tab_index: *ti,
                drawing_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let pkg_opts = PackageOptions {
            convert: ConvertOptions {
                plant,
                seed_pid_path,
                sym_library_dir: symdir.or_else(|| std::env::var("OCS_PID_SYM_DIR").ok()),
                class_map_path: classmap,
                external_props_path: props,
                verify_plot_pdf: verify && wants_dual,
                ..Default::default()
            },
            verify_strict: true,
            strict_gate,
        };

        match export_handover_package_from_sheets(&sheet_inputs, &out_dir, &pkg_opts) {
            Ok(man) => {
                self.command_line.push_output(&format!(
                    "PIDPACKAGE: 交付包(v{})已生成 -> {}（就绪={}，图纸={}，.pid={}，PDF={}，设备={}，管线={}，管口={}，阀门={}，泄压={}，联锁={}，硬I/O={}，特殊件={}，文件数={}）。",
                    man.schema_version,
                    out_dir.display(),
                    man.handover_ready,
                    man.sheets_exported,
                    man.pid_files_written,
                    man.pdf_files_plotted,
                    man.master_equipment_count,
                    man.master_pipelines_count,
                    man.equipment_nozzles_count,
                    man.total_valves,
                    man.relief_devices_count,
                    man.sis_interlocks_count,
                    man.dcs_total_hw_io + man.sis_total_hw_io,
                    man.piping_specialties_count,
                    man.files.len(),
                ));
                Some(self.finish_dispatch(cmd))
            }
            Err(err) => {
                self.command_line.push_error(&format!("PIDPACKAGE: {err}"));
                Some(Task::none())
            }
        }
    }

    fn dispatch_pidrenum_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            plan_or_apply_sheet_renumber, write_pid_renumber_file, PidRenumberReport,
            PidRenumberSummary, RenumberOptions,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDRENUM") {
            return None;
        }

        let mut all_tabs = false;
        let mut selected_only = false;
        let mut dry_run = false;
        let mut force = false;
        let mut export_path: Option<std::path::PathBuf> = None;
        let mut positional: Vec<String> = Vec::new();

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SELECTED" => {
                    selected_only = true;
                    idx += 1;
                }
                "DRYRUN" => {
                    dry_run = true;
                    idx += 1;
                }
                "FORCE" => {
                    force = true;
                    idx += 1;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDRENUM EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    positional.push(tok.clone());
                    idx += 1;
                }
            }
        }

        if positional.len() < 2 {
            self.command_line.push_error(
                "PIDRENUM: 用法 PIDRENUM \"<旧编码>\" \"<新编码>\" [DRYRUN] [SELECTED | ALLTABS] [FORCE] [EXPORT \"<.json|.csv|.tsv>\"]。",
            );
            return Some(Task::none());
        }
        let from_pat = positional[0].clone();
        let to_pat = positional[1].clone();

        let target_tabs: Vec<(usize, String)> = if all_tabs && !selected_only {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sel_vec: Option<Vec<document::Handle>> = selected_only.then(|| {
            self.tabs[i]
                .scene
                .selected_entities()
                .into_iter()
                .map(|(h, _)| h)
                .collect()
        });

        // Phase 1: pre-flight across all target tabs without mutating documents or undo stack
        let pre_opts = RenumberOptions {
            from_pattern: from_pat.clone(),
            to_pattern: to_pat.clone(),
            dry_run: true,
            force,
        };
        let mut pre_items = Vec::new();
        let mut pre_conflicts = Vec::new();
        let mut pre_pipes = 0usize;
        for &(ti, ref nm) in &target_tabs {
            let sel_slice = if ti == i { sel_vec.as_deref() } else { None };
            let out = plan_or_apply_sheet_renumber(
                nm,
                &mut self.tabs[ti].scene.document,
                sel_slice,
                &pre_opts,
            );
            pre_pipes += out.pipelines_renamed;
            pre_items.extend(out.items);
            pre_conflicts.extend(out.conflicts);
        }

        let pre_syms = pre_items
            .iter()
            .filter(|it| it.entity_kind == "symbol")
            .count();
        let can_apply = !dry_run && (pre_conflicts.is_empty() || force) && !pre_items.is_empty();

        if !can_apply {
            let rep = PidRenumberReport {
                summary: PidRenumberSummary {
                    from_pattern: from_pat,
                    to_pattern: to_pat,
                    dry_run,
                    applied: false,
                    sheets_processed: target_tabs.len(),
                    symbols_renamed: pre_syms,
                    pipelines_renamed: pre_pipes,
                    canvas_texts_synced: 0,
                    conflict_count: pre_conflicts.len(),
                },
                items: pre_items,
                conflicts: pre_conflicts,
            };
            if let Some(ref out_p) = export_path {
                #[cfg(not(target_arch = "wasm32"))]
                if let Err(err) = write_pid_renumber_file(&rep, out_p) {
                    self.command_line
                        .push_error(&format!("PIDRENUM EXPORT: {err}"));
                    return Some(Task::none());
                }
            }
            if !dry_run && rep.summary.conflict_count > 0 {
                self.command_line.push_error(&format!(
                    "PIDRENUM: 检测到 {} 处重号冲突，已阻断改写（可加 DRYRUN 查看明细或 FORCE 强制执行）。",
                    rep.summary.conflict_count
                ));
                return Some(Task::none());
            }
            self.command_line.push_output(&format!(
                "PIDRENUM 预演: 符号={}，管线={}，冲突={}（未写入画布）。",
                rep.summary.symbols_renamed,
                rep.summary.pipelines_renamed,
                rep.summary.conflict_count,
            ));
            return Some(self.finish_dispatch(cmd));
        }

        // Phase 2: push one undo snapshot per affected tab and apply atomically
        let live_opts = RenumberOptions {
            from_pattern: from_pat.clone(),
            to_pattern: to_pat.clone(),
            dry_run: false,
            force,
        };
        let mut all_items = Vec::new();
        let mut all_conflicts = Vec::new();
        let mut total_pipes = 0usize;
        let mut total_synced = 0usize;

        for &(ti, ref nm) in &target_tabs {
            let sel_slice = if ti == i { sel_vec.as_deref() } else { None };
            self.push_undo_snapshot(ti, "PIDRENUM");
            let out = plan_or_apply_sheet_renumber(
                nm,
                &mut self.tabs[ti].scene.document,
                sel_slice,
                &live_opts,
            );
            if !out.modified_handles.is_empty() {
                total_synced += out.modified_handles.len();
                let changes: Vec<_> = out
                    .modified_handles
                    .iter()
                    .map(|&h| (h, crate::scene::ChangeKind::Modified))
                    .collect();
                self.tabs[ti].scene.bump_entities(&changes);
                self.tabs[ti].dirty = true;
            }
            total_pipes += out.pipelines_renamed;
            all_items.extend(out.items);
            all_conflicts.extend(out.conflicts);
        }
        self.refresh_properties();

        let syms_cnt = all_items
            .iter()
            .filter(|it| it.entity_kind == "symbol")
            .count();
        let rep = PidRenumberReport {
            summary: PidRenumberSummary {
                from_pattern: from_pat,
                to_pattern: to_pat,
                dry_run: false,
                applied: true,
                sheets_processed: target_tabs.len(),
                symbols_renamed: syms_cnt,
                pipelines_renamed: total_pipes,
                canvas_texts_synced: total_synced,
                conflict_count: all_conflicts.len(),
            },
            items: all_items,
            conflicts: all_conflicts,
        };
        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_renumber_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDRENUM EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        self.command_line.push_output(&format!(
            "PIDRENUM: 已重编号 {} 个符号与 {} 条管线（同步画布图元={}）。",
            rep.summary.symbols_renamed, rep.summary.pipelines_renamed, total_synced,
        ));
        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidhazop_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_hazop_nodes, hazop_node_matches_query, write_pid_hazop_file,
            HazopSafeguardStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDHAZOP") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut node_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "UNPROTECTED" | "PROTECTED" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "NODE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDHAZOP NODE: 缺少设备位号或节点编号。");
                        return Some(Task::none());
                    };
                    action = Some("NODE".to_string());
                    node_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDHAZOP EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("NODE".to_string());
                        node_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_hazop_nodes(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_hazop_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDHAZOP EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .nodes
            .iter()
            .filter(|nd| {
                let q_ok = node_query
                    .as_deref()
                    .is_none_or(|q| hazop_node_matches_query(nd, q));
                let st_ok = match eff_action {
                    "UNPROTECTED" => nd.status == HazopSafeguardStatus::Unprotected,
                    "PROTECTED" => nd.status == HazopSafeguardStatus::Protected,
                    _ => true,
                };
                q_ok && st_ok
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDHAZOP: 扫描图纸={} | HAZOP节点={}（跨图节点={}，具备保护层={}，盲区无保护={}）| 关联管线={} | 关联保护仪表={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_nodes,
                    rep.summary.multi_sheet_nodes,
                    rep.summary.protected_nodes,
                    rep.summary.unprotected_nodes,
                    rep.summary.total_linked_pipelines,
                    rep.summary.total_linked_safeguards,
                ));
            }
            "ALL" | "UNPROTECTED" | "PROTECTED" | "NODE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for nd in &filtered {
                    for (&ti, hs) in &nd.handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "NODE"
                    && filtered.len() == 1
                    && filtered[0].tab_index < self.tabs.len()
                    && filtered[0].tab_index != self.active_tab
                {
                    let dt = filtered[0].tab_index;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDHAZOP {eff_action}: 匹配 {} 个审查节点，跨打开图纸高亮 {} 个图元。",
                    filtered.len(),
                    sel_cnt
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDHAZOP NEXT: 未找到可巡检的 HAZOP 审查节点。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|nd| nd.tab_index == cur_tab && sel_set.contains(&nd.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    if let Some(hs) = chosen.handles_by_tab.get(&dest_tab) {
                        self.tabs[dest_tab].scene.select_entities(hs);
                        let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(hs);
                        if !zoomed {
                            let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                            let z = cam.target.z;
                            cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                        }
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDHAZOP NEXT [{}/{}]: {} {}（{}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.node_id,
                    chosen.equipment_tag,
                    chosen.primary_drawing,
                    chosen.status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDHAZOP: 未知子命令 {other}（支持 SUMMARY / ALL / UNPROTECTED / PROTECTED / NODE / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidutility_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_utilities_and_interfaces, write_pid_utility_file, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDUTILITY") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut service_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "UTILITIES" | "BATTERYLIMIT" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "SERVICE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDUTILITY SERVICE: 缺少介质代号（如 N2、LS、CW）。");
                        return Some(Task::none());
                    };
                    action = Some("SERVICE".to_string());
                    service_query = Some(q.to_ascii_uppercase());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDUTILITY EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("SERVICE".to_string());
                        service_query = Some(up);
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_utilities_and_interfaces(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_utility_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDUTILITY EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDUTILITY: 扫描图纸={} | 流体介质={}（公用工程={}，主工艺={}）| 标记管线={} | 界区外接接口={}（公用工程接口={}）。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_services,
                    rep.summary.utility_services_count,
                    rep.summary.process_services_count,
                    rep.summary.labeled_pipelines_count,
                    rep.summary.battery_limit_count,
                    rep.summary.utility_battery_limits,
                ));
            }
            "ALL" | "UTILITIES" | "SERVICE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                let mut matched_svcs = 0usize;
                for sg in &rep.service_groups {
                    let keep = match eff_action {
                        "UTILITIES" => sg.is_utility,
                        "SERVICE" => service_query
                            .as_deref()
                            .is_some_and(|q| sg.fluid_code.eq_ignore_ascii_case(q)),
                        _ => true,
                    };
                    if !keep {
                        continue;
                    }
                    matched_svcs += 1;
                    for (&ti, hs) in &sg.handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDUTILITY {eff_action}: 匹配 {matched_svcs} 类介质，跨打开图纸高亮 {sel_cnt} 个管段图元。"
                ));
            }
            "BATTERYLIMIT" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for bl in &rep.battery_limits {
                    let dst = by_tab.entry(bl.tab_index).or_default();
                    for &h in &bl.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDUTILITY BATTERYLIMIT: 匹配 {} 个界区接口，高亮 {} 个图元。",
                    rep.battery_limits.len(),
                    sel_cnt
                ));
            }
            "NEXT" => {
                if rep.battery_limits.is_empty() {
                    self.command_line
                        .push_output("PIDUTILITY NEXT: 未检出外接界区接口（Battery Limit）。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = rep
                    .battery_limits
                    .iter()
                    .position(|bl| bl.tab_index == cur_tab && sel_set.contains(&bl.primary_handle));
                let next_idx = cur_pos
                    .map(|p| (p + 1) % rep.battery_limits.len())
                    .unwrap_or(0);
                let chosen = &rep.battery_limits[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDUTILITY NEXT [{}/{}]: {} -> {}（{}，规格 {} / {}）。",
                    next_idx + 1,
                    rep.battery_limits.len(),
                    chosen.interface_id,
                    chosen.external_target,
                    chosen.drawing_code,
                    chosen.nominal_diameter,
                    chosen.piping_spec,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDUTILITY: 未知子命令 {other}（支持 SUMMARY / ALL / UTILITIES / BATTERYLIMIT / SERVICE / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidvalve_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_valve_mto, write_pid_valve_file, OpcSheetInput, ValveCategory,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDVALVE") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut type_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "MANUAL" | "ACTUATED" | "RELIEF" | "UNRESOLVED" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "TYPE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDVALVE TYPE: 缺少阀型关键字（如 BALL、GATE、CHECK）。");
                        return Some(Task::none());
                    };
                    action = Some("TYPE".to_string());
                    type_query = Some(q.to_ascii_lowercase());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDVALVE EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TYPE".to_string());
                        type_query = Some(tok.to_ascii_lowercase());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_valve_mto(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_valve_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDVALVE EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .valves
            .iter()
            .filter(|v| match eff_action {
                "MANUAL" => v.category == ValveCategory::ManualBulk,
                "ACTUATED" => v.category == ValveCategory::ActuatedControl,
                "RELIEF" => v.category == ValveCategory::SafetyRelief,
                "UNRESOLVED" => !v.spec_resolved,
                "TYPE" => type_query
                    .as_deref()
                    .is_some_and(|q| v.valve_subtype.as_str().contains(q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDVALVE: 扫描图纸={} | 总阀门={}（手动大宗阀={}，自控阀={}，安全阀={}）| 已继承规格={} | 待补规格={} | MTO汇总行={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_valves,
                    rep.summary.manual_bulk_valves,
                    rep.summary.actuated_control_valves,
                    rep.summary.safety_relief_valves,
                    rep.summary.spec_resolved_valves,
                    rep.summary.unresolved_valves,
                    rep.summary.mto_groups_count,
                ));
            }
            "ALL" | "MANUAL" | "ACTUATED" | "RELIEF" | "UNRESOLVED" | "TYPE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for v in &filtered {
                    let dst = by_tab.entry(v.tab_index).or_default();
                    for &h in &v.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDVALVE {eff_action}: 匹配 {} 台阀门，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDVALVE NEXT: 未找到可巡检的阀门图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|v| v.tab_index == cur_tab && sel_set.contains(&v.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDVALVE NEXT [{}/{}]: {}（{}，{}，规格={} / {}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.valve_tag,
                    chosen.valve_subtype.as_str(),
                    chosen.drawing_code,
                    chosen.nominal_diameter,
                    chosen.piping_spec,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDVALVE: 未知子命令 {other}（支持 SUMMARY / ALL / MANUAL / ACTUATED / RELIEF / UNRESOLVED / TYPE / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidnozzle_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_equipment_nozzles, nozzle_matches_equipment, write_pid_nozzle_file,
            NozzleStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDNOZZLE") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut equip_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "CONNECTED" | "SPARE" | "ORPHAN" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "EQUIPMENT" | "EQUIP" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDNOZZLE EQUIPMENT: 缺少设备位号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("EQUIPMENT".to_string());
                    equip_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDNOZZLE EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("EQUIPMENT".to_string());
                        equip_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_equipment_nozzles(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_nozzle_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDNOZZLE EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .nozzles
            .iter()
            .filter(|nz| match eff_action {
                "CONNECTED" => nz.status == NozzleStatus::Connected,
                "SPARE" => nz.status == NozzleStatus::SpareOrManway,
                "ORPHAN" => nz.status == NozzleStatus::OrphanNozzle,
                "EQUIPMENT" => equip_query
                    .as_deref()
                    .is_some_and(|q| nozzle_matches_equipment(nz, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDNOZZLE: 扫描图纸={} | 带管口设备={} | 总管口={}（显式={}，隐式合成={}，已接管={}，备用盲口={}，孤立管口={}）。",
                    rep.summary.sheets_scanned,
                    rep.summary.equipment_with_nozzles,
                    rep.summary.total_nozzles,
                    rep.summary.explicit_nozzles,
                    rep.summary.implicit_nozzles,
                    rep.summary.connected_nozzles,
                    rep.summary.spare_nozzles,
                    rep.summary.orphan_nozzles,
                ));
            }
            "ALL" | "CONNECTED" | "SPARE" | "ORPHAN" | "EQUIPMENT" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for nz in &filtered {
                    let dst = by_tab.entry(nz.tab_index).or_default();
                    for &h in &nz.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "EQUIPMENT"
                    && !filtered.is_empty()
                    && filtered[0].tab_index < self.tabs.len()
                    && filtered[0].tab_index != self.active_tab
                {
                    let dt = filtered[0].tab_index;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDNOZZLE {eff_action}: 匹配 {} 个管口，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDNOZZLE NEXT: 未找到可巡检的管口图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|nz| nz.tab_index == cur_tab && sel_set.contains(&nz.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDNOZZLE NEXT [{}/{}]: {} {}（{}，方位={}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.equipment_tag,
                    chosen.nozzle_id,
                    chosen.drawing_code,
                    chosen.orientation.as_str(),
                    chosen.status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDNOZZLE: 未知子命令 {other}（支持 SUMMARY / ALL / CONNECTED / SPARE / ORPHAN / EQUIPMENT / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidindex_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_drawing_index, drawing_index_matches_query, write_pid_drawing_index_file,
            DrawingIndexStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDINDEX") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut issues_only = false;
        let mut jump_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" => {
                    action = Some(up);
                    idx += 1;
                }
                "ISSUES" => {
                    issues_only = true;
                    if action.is_none() {
                        action = Some("ISSUES".to_string());
                    }
                    idx += 1;
                }
                "NEXT" => {
                    action = Some("NEXT".to_string());
                    idx += 1;
                }
                "JUMP" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDINDEX JUMP: 缺少图号、图名或序号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("JUMP".to_string());
                    jump_query = Some(q.clone());
                    all_tabs = true;
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDINDEX EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("JUMP".to_string());
                        jump_query = Some(tok.clone());
                        all_tabs = true;
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_drawing_index(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_drawing_index_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDINDEX EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .sheets
            .iter()
            .filter(|r| !issues_only || r.status != DrawingIndexStatus::Ok)
            .collect();

        match eff_action {
            "SUMMARY" | "ALL" | "ISSUES" => {
                self.command_line.push_output(&format!(
                    "PIDINDEX: 图纸={}（正常={}，缺图号={}，重图号={}）| 设备={} | 阀门={} | 仪表={} | 管线={} | 连接符={}。",
                    rep.summary.total_sheets,
                    rep.summary.ok_sheets,
                    rep.summary.missing_number_sheets,
                    rep.summary.duplicate_number_sheets,
                    rep.summary.total_equipment,
                    rep.summary.total_valves,
                    rep.summary.total_instruments,
                    rep.summary.total_pipelines,
                    rep.summary.total_opcs,
                ));
            }
            "JUMP" => {
                let q = jump_query.as_deref().unwrap_or("");
                let Some(chosen) = rep
                    .sheets
                    .iter()
                    .find(|r| drawing_index_matches_query(r, q))
                else {
                    self.command_line
                        .push_error(&format!("PIDINDEX JUMP: 未找到匹配 \"{q}\" 的图纸。"));
                    return Some(Task::none());
                };
                if chosen.tab_index < self.tabs.len() && chosen.tab_index != self.active_tab {
                    let _ = self.update(Message::TabSwitch(chosen.tab_index));
                    self.active_tab = chosen.tab_index;
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDINDEX JUMP: 已切换至图纸 #{} {} {}（{}）。",
                    chosen.seq_no, chosen.drawing_number, chosen.drawing_title, chosen.file_label
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDINDEX NEXT: 未检出待巡检的图纸目录条目。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let cur_pos = filtered.iter().position(|r| r.tab_index == cur_tab);
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                if chosen.tab_index < self.tabs.len() && chosen.tab_index != self.active_tab {
                    let _ = self.update(Message::TabSwitch(chosen.tab_index));
                    self.active_tab = chosen.tab_index;
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDINDEX NEXT [{}/{}]: {}（{}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.drawing_number,
                    chosen.file_label,
                    chosen.status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDINDEX: 未知子命令 {other}（支持 SUMMARY / ALL / ISSUES / JUMP / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidspecialty_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_specialty_items, write_pid_specialty_file, OpcSheetInput, SpecialtyCategory,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDSPECIALTY") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut type_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "SPECIALTIES" | "REDUCERS" | "FITTINGS" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "TYPE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line.push_error(
                            "PIDSPECIALTY TYPE: 缺少子型关键字（如 STRAINER、REDUCER、HOSE）。",
                        );
                        return Some(Task::none());
                    };
                    action = Some("TYPE".to_string());
                    type_query = Some(q.to_ascii_lowercase());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line.push_error(
                            "PIDSPECIALTY EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。",
                        );
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TYPE".to_string());
                        type_query = Some(tok.to_ascii_lowercase());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_specialty_items(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_specialty_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDSPECIALTY EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .items
            .iter()
            .filter(|it| match eff_action {
                "SPECIALTIES" => it.category == SpecialtyCategory::SpecialtyItem,
                "REDUCERS" => it.category == SpecialtyCategory::ReducerFitting,
                "FITTINGS" => it.category != SpecialtyCategory::SpecialtyItem,
                "TYPE" => type_query
                    .as_deref()
                    .is_some_and(|q| it.subtype.as_str().contains(q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDSPECIALTY: 扫描图纸={} | 非阀组件={}（管道特殊件={}，异径大小头={}，法兰/管帽={}）| 已继承规格={} | 汇总行={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_items,
                    rep.summary.specialty_items_count,
                    rep.summary.reducers_count,
                    rep.summary.flanges_or_caps_count,
                    rep.summary.spec_resolved_count,
                    rep.summary.rollup_groups_count,
                ));
            }
            "ALL" | "SPECIALTIES" | "REDUCERS" | "FITTINGS" | "TYPE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for it in &filtered {
                    let dst = by_tab.entry(it.tab_index).or_default();
                    for &h in &it.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDSPECIALTY {eff_action}: 匹配 {} 件特殊件/管件，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDSPECIALTY NEXT: 未找到可巡检的特殊件或管件图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|it| it.tab_index == cur_tab && sel_set.contains(&it.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDSPECIALTY NEXT [{}/{}]: {}（{}，{}，规格={} / {}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.item_tag,
                    chosen.subtype.as_str(),
                    chosen.drawing_code,
                    chosen.size_spec,
                    chosen.piping_spec,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDSPECIALTY: 未知子命令 {other}（支持 SUMMARY / ALL / SPECIALTIES / REDUCERS / FITTINGS / TYPE / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidlinelist_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_line_list, pipeline_row_matches_query, write_pid_line_list_file,
            LineTerminationStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDLINELIST") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut service_query: Option<String> = None;
        let mut line_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "UNTERMINATED" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "SERVICE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDLINELIST SERVICE: 缺少介质代号（如 PG、LS、N2）。");
                        return Some(Task::none());
                    };
                    action = Some("SERVICE".to_string());
                    service_query = Some(q.to_ascii_uppercase());
                    idx += 2;
                }
                "LINE" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDLINELIST LINE: 缺少管线号或序号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("LINE".to_string());
                    line_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line.push_error(
                            "PIDLINELIST EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。",
                        );
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("LINE".to_string());
                        line_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_line_list(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_line_list_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDLINELIST EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .pipelines
            .iter()
            .filter(|r| match eff_action {
                "UNTERMINATED" => r.status == LineTerminationStatus::Unterminated,
                "SERVICE" => service_query
                    .as_deref()
                    .is_some_and(|q| r.fluid_code.eq_ignore_ascii_case(q)),
                "LINE" => line_query
                    .as_deref()
                    .is_some_and(|q| pipeline_row_matches_query(r, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDLINELIST: 扫描图纸={} | 总管线={}（跨图管线={}，双端闭环={}，单端={}，悬空孤立={}）| 在线阀门={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_pipelines,
                    rep.summary.multi_sheet_pipelines,
                    rep.summary.both_terminated_count,
                    rep.summary.single_ended_count,
                    rep.summary.unterminated_count,
                    rep.summary.total_inline_valves,
                ));
            }
            "ALL" | "UNTERMINATED" | "SERVICE" | "LINE" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for r in &filtered {
                    for (&ti, hs) in &r.handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "LINE"
                    && filtered.len() == 1
                    && filtered[0].primary_tab < self.tabs.len()
                    && filtered[0].primary_tab != self.active_tab
                {
                    let dt = filtered[0].primary_tab;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDLINELIST {eff_action}: 匹配 {} 条管线，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDLINELIST NEXT: 未找到可巡检的管线图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|r| r.primary_tab == cur_tab && sel_set.contains(&r.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.primary_tab;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    if let Some(hs) = chosen.handles_by_tab.get(&dest_tab) {
                        self.tabs[dest_tab].scene.select_entities(hs);
                        let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(hs);
                        if !zoomed {
                            let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                            let z = cam.target.z;
                            cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                        }
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDLINELIST NEXT [{}/{}]: {}（{} -> {}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.line_number,
                    chosen.from_endpoint,
                    chosen.to_endpoint,
                    chosen.status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDLINELIST: 未知子命令 {other}（支持 SUMMARY / ALL / UNTERMINATED / SERVICE / LINE / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidequip_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_equipment_schedule, equipment_row_matches_query, write_pid_equipment_file,
            EquipmentCategory, EquipmentTagStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDEQUIP") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut tag_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "VESSELS" | "ROTATING" | "EXCHANGERS" | "TANKS"
                | "UNTAGGED" | "DUPLICATES" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "TAG" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDEQUIP TAG: 缺少设备位号关键字（如 V-1001、P-101）。");
                        return Some(Task::none());
                    };
                    action = Some("TAG".to_string());
                    tag_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDEQUIP EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TAG".to_string());
                        tag_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_equipment_schedule(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_equipment_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDEQUIP EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .equipment
            .iter()
            .filter(|r| match eff_action {
                "VESSELS" => r.category == EquipmentCategory::VesselOrColumn,
                "ROTATING" => r.category == EquipmentCategory::RotatingMachinery,
                "EXCHANGERS" => r.category == EquipmentCategory::HeatTransfer,
                "TANKS" => r.category == EquipmentCategory::StorageTank,
                "UNTAGGED" => r.tag_status == EquipmentTagStatus::Untagged,
                "DUPLICATES" => r.tag_status == EquipmentTagStatus::DuplicateAcrossSheets,
                "TAG" => tag_query
                    .as_deref()
                    .is_some_and(|q| equipment_row_matches_query(r, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDEQUIP: 扫描图纸={} | 总设备={}（塔/容器={}，动设备={}，换热={}，储罐={}，成套/其他={}）| 缺位号={}，跨图重号={}，管口累计={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_equipment,
                    rep.summary.vessels_count,
                    rep.summary.rotating_count,
                    rep.summary.heat_transfer_count,
                    rep.summary.tanks_count,
                    rep.summary.package_misc_count,
                    rep.summary.untagged_count,
                    rep.summary.duplicate_tag_count,
                    rep.summary.total_connected_nozzles,
                ));
            }
            "ALL" | "VESSELS" | "ROTATING" | "EXCHANGERS" | "TANKS" | "UNTAGGED" | "DUPLICATES"
            | "TAG" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for r in &filtered {
                    for (&ti, hs) in &r.handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "TAG"
                    && filtered.len() == 1
                    && filtered[0].primary_tab < self.tabs.len()
                    && filtered[0].primary_tab != self.active_tab
                {
                    let dt = filtered[0].primary_tab;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDEQUIP {eff_action}: 匹配 {} 台设备，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDEQUIP NEXT: 未找到可巡检的设备图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|r| r.primary_tab == cur_tab && sel_set.contains(&r.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.primary_tab;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    if let Some(hs) = chosen.handles_by_tab.get(&dest_tab) {
                        self.tabs[dest_tab].scene.select_entities(hs);
                        let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(hs);
                        if !zoomed {
                            let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                            let z = cam.target.z;
                            cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                        }
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDEQUIP NEXT [{}/{}]: {}（类别={}，管口={}，管线={}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.equipment_tag,
                    chosen.category.as_str(),
                    chosen.nozzle_count,
                    chosen.connected_lines.len(),
                    chosen.tag_status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDEQUIP: 未知子命令 {other}（支持 SUMMARY / ALL / VESSELS / ROTATING / EXCHANGERS / TANKS / UNTAGGED / DUPLICATES / TAG / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidrelief_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_relief_devices, relief_row_matches_query, write_pid_relief_file,
            OpcSheetInput, ReliefDestinationKind,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDRELIEF") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut tag_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "VIOLATIONS" | "FLARE" | "ATMOSPHERE" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "TAG" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDRELIEF TAG: 缺少泄放装置位号或被保护设备位号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("TAG".to_string());
                    tag_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDRELIEF EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TAG".to_string());
                        tag_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_relief_devices(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_relief_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDRELIEF EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .devices
            .iter()
            .filter(|d| match eff_action {
                "VIOLATIONS" => d.compliance_status.is_violation(),
                "FLARE" => d.destination == ReliefDestinationKind::FlareHeader,
                "ATMOSPHERE" => d.destination == ReliefDestinationKind::Atmosphere,
                "TAG" => tag_query
                    .as_deref()
                    .is_some_and(|q| relief_row_matches_query(d, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDRELIEF: 扫描图纸={} | 泄放装置={}（安全阀={}，爆破片={}，呼吸阀={}，紧急泄放={}）| 去向[火炬={}，大气={}，闭式={}] | 合规={}，未铅封截断阀违例={}，悬空={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_devices,
                    rep.summary.safety_valves_count,
                    rep.summary.rupture_discs_count,
                    rep.summary.breather_valves_count,
                    rep.summary.blowdown_valves_count,
                    rep.summary.flare_discharges,
                    rep.summary.atmosphere_discharges,
                    rep.summary.closed_discharges,
                    rep.summary.compliant_count,
                    rep.summary.unsealed_valve_violations,
                    rep.summary.unconnected_count,
                ));
            }
            "ALL" | "VIOLATIONS" | "FLARE" | "ATMOSPHERE" | "TAG" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for d in &filtered {
                    let hs = d
                        .handles_by_tab
                        .get(&d.tab_index)
                        .cloned()
                        .unwrap_or_else(|| vec![d.primary_handle]);
                    let dst = by_tab.entry(d.tab_index).or_default();
                    for h in [d.primary_handle].into_iter().chain(hs.into_iter().take(0)) {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "TAG"
                    && filtered.len() == 1
                    && filtered[0].tab_index < self.tabs.len()
                    && filtered[0].tab_index != self.active_tab
                {
                    let dt = filtered[0].tab_index;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDRELIEF {eff_action}: 匹配 {} 台泄放装置，高亮 {sel_cnt} 个符号图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDRELIEF NEXT: 未找到可巡检的泄放保护装置。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|d| d.tab_index == cur_tab && sel_set.contains(&d.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    let hs = vec![chosen.primary_handle];
                    self.tabs[dest_tab].scene.select_entities(&hs);
                    let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(&hs);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDRELIEF NEXT [{}/{}]: {}（保护={}，去向={}，合规={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.device_tag,
                    chosen.protected_equipment,
                    chosen.destination.as_str(),
                    chosen.compliance_status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDRELIEF: 未知子命令 {other}（支持 SUMMARY / ALL / VIOLATIONS / FLARE / ATMOSPHERE / TAG / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidcause_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_cause_effect, cause_effect_row_matches_query, write_pid_cause_effect_file,
            InterlockStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDCAUSE") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut tag_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "COMPLETE" | "INCOMPLETE" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "TAG" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDCAUSE TAG: 缺少联锁测点、逻辑块或切断阀位号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("TAG".to_string());
                    tag_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDCAUSE EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TAG".to_string());
                        tag_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_cause_effect(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_cause_effect_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDCAUSE EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .interlocks
            .iter()
            .filter(|r| match eff_action {
                "COMPLETE" => r.status == InterlockStatus::CompleteInterlock,
                "INCOMPLETE" => r.status.is_incomplete(),
                "TAG" => tag_query
                    .as_deref()
                    .is_some_and(|q| cause_effect_row_matches_query(r, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDCAUSE: 扫描图纸={} | 安全联锁={}（跨图={}，完整闭环={}，缺执行元件={}，孤立切断阀={}）| 触发源={}，执行机构={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_interlocks,
                    rep.summary.multi_sheet_interlocks,
                    rep.summary.complete_count,
                    rep.summary.missing_actuator_count,
                    rep.summary.orphan_actuator_count,
                    rep.summary.total_cause_sensors,
                    rep.summary.total_trip_actuators,
                ));
            }
            "ALL" | "COMPLETE" | "INCOMPLETE" | "TAG" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for r in &filtered {
                    for (&ti, hs) in &r.handles_by_tab {
                        let dst = by_tab.entry(ti).or_default();
                        for &h in hs {
                            if !dst.contains(&h) {
                                dst.push(h);
                            }
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "TAG"
                    && filtered.len() == 1
                    && filtered[0].primary_tab < self.tabs.len()
                    && filtered[0].primary_tab != self.active_tab
                {
                    let dt = filtered[0].primary_tab;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDCAUSE {eff_action}: 匹配 {} 条安全联锁，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDCAUSE NEXT: 未找到可巡检的安全联锁行。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|r| r.primary_tab == cur_tab && sel_set.contains(&r.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.primary_tab;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    if let Some(hs) = chosen.handles_by_tab.get(&dest_tab) {
                        self.tabs[dest_tab].scene.select_entities(hs);
                        let zoomed = self.tabs[dest_tab].scene.zoom_to_entities(hs);
                        if !zoomed {
                            let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                            let z = cam.target.z;
                            cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                        }
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDCAUSE NEXT [{}/{}]: {}（触发={:?}，动作={:?}，状态={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.interlock_id,
                    chosen.cause_tags,
                    chosen.effect_actions,
                    chosen.status.as_str(),
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDCAUSE: 未知子命令 {other}（支持 SUMMARY / ALL / COMPLETE / INCOMPLETE / TAG / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

        Some(self.finish_dispatch(cmd))
    }

    fn dispatch_pidio_cmd(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        use crate::io::pid_convert::{
            analyze_pid_io_list, io_item_matches_query, write_pid_io_list_file, ControlSystemKind,
            MountStatus, OpcSheetInput,
        };

        let tokens = tokenize_command_words(cmd);
        let first = tokens.first()?;
        if !first.eq_ignore_ascii_case("PIDIO") {
            return None;
        }

        let mut action: Option<String> = None;
        let mut channel_query: Option<String> = None;
        let mut tag_query: Option<String> = None;
        let mut all_tabs = false;
        let mut export_path: Option<std::path::PathBuf> = None;

        let mut idx = 1usize;
        while idx < tokens.len() {
            let tok = &tokens[idx];
            let up = tok.to_ascii_uppercase();
            match up.as_str() {
                "ALLTABS" => {
                    all_tabs = true;
                    idx += 1;
                }
                "SUMMARY" | "ALL" | "DCS" | "SIS" | "LOCAL" | "UNCONNECTED" | "NEXT" => {
                    action = Some(up);
                    idx += 1;
                }
                "CHANNEL" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line.push_error(
                            "PIDIO CHANNEL: 缺少信号类型（AI / AO / DI / DO / SOFT）。",
                        );
                        return Some(Task::none());
                    };
                    action = Some("CHANNEL".to_string());
                    channel_query = Some(q.to_ascii_uppercase());
                    idx += 2;
                }
                "TAG" => {
                    let Some(q) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDIO TAG: 缺少仪表或程控阀位号关键字。");
                        return Some(Task::none());
                    };
                    action = Some("TAG".to_string());
                    tag_query = Some(q.clone());
                    idx += 2;
                }
                "EXPORT" => {
                    let Some(next_tok) = tokens.get(idx + 1) else {
                        self.command_line
                            .push_error("PIDIO EXPORT: 缺少输出文件路径（.json/.csv/.tsv）。");
                        return Some(Task::none());
                    };
                    export_path = Some(std::path::PathBuf::from(next_tok));
                    idx += 2;
                }
                _ => {
                    if action.is_none() {
                        action = Some("TAG".to_string());
                        tag_query = Some(tok.clone());
                    }
                    idx += 1;
                }
            }
        }

        let target_tabs: Vec<(usize, String)> = if all_tabs {
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| !t.is_start)
                .map(|(ti, t)| {
                    let nm = t
                        .current_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| t.tab_title.clone());
                    (ti, nm)
                })
                .collect()
        } else {
            let nm = self.tabs[i]
                .current_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| self.tabs[i].tab_title.clone());
            vec![(i, nm)]
        };

        let sheet_inputs: Vec<OpcSheetInput<'_>> = target_tabs
            .iter()
            .map(|(ti, nm)| OpcSheetInput {
                tab_index: *ti,
                fallback_name: nm.as_str(),
                document: &self.tabs[*ti].scene.document,
            })
            .collect();

        let rep = analyze_pid_io_list(&sheet_inputs);

        if let Some(ref out_p) = export_path {
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(err) = write_pid_io_list_file(&rep, out_p) {
                self.command_line
                    .push_error(&format!("PIDIO EXPORT: {err}"));
                return Some(Task::none());
            }
        }

        let eff_action = action.as_deref().unwrap_or("SUMMARY");
        let filtered: Vec<_> = rep
            .items
            .iter()
            .filter(|r| match eff_action {
                "DCS" => r.control_system == ControlSystemKind::Dcs,
                "SIS" => r.control_system == ControlSystemKind::SisEsd,
                "LOCAL" => r.control_system == ControlSystemKind::Local,
                "UNCONNECTED" => r.mount_status == MountStatus::UnconnectedField,
                "CHANNEL" => channel_query
                    .as_deref()
                    .is_some_and(|q| r.io_channel.as_str().eq_ignore_ascii_case(q)),
                "TAG" => tag_query
                    .as_deref()
                    .is_some_and(|q| io_item_matches_query(r, q)),
                _ => true,
            })
            .collect();

        match eff_action {
            "SUMMARY" => {
                self.command_line.push_output(&format!(
                    "PIDIO: 扫描图纸={} | 仪表/执行器={} | DCS硬点={}(+20%裕量={})[AI={},AO={},DI={},DO={}] | SIS硬点={}(+20%裕量={})[AI={},DI={},DO={}] | 就地表={}，软功能={}，未接管={}。",
                    rep.summary.sheets_scanned,
                    rep.summary.total_items,
                    rep.summary.dcs_total_hw,
                    rep.summary.dcs_with_20pct_spare,
                    rep.summary.dcs_ai,
                    rep.summary.dcs_ao,
                    rep.summary.dcs_di,
                    rep.summary.dcs_do,
                    rep.summary.sis_total_hw,
                    rep.summary.sis_with_20pct_spare,
                    rep.summary.sis_ai,
                    rep.summary.sis_di,
                    rep.summary.sis_do,
                    rep.summary.local_gauges_count,
                    rep.summary.soft_functions_count,
                    rep.summary.unconnected_field_count,
                ));
            }
            "ALL" | "DCS" | "SIS" | "LOCAL" | "UNCONNECTED" | "CHANNEL" | "TAG" => {
                let mut by_tab: std::collections::BTreeMap<usize, Vec<document::Handle>> =
                    std::collections::BTreeMap::new();
                for r in &filtered {
                    let dst = by_tab.entry(r.tab_index).or_default();
                    for &h in &r.member_handles {
                        if !dst.contains(&h) {
                            dst.push(h);
                        }
                    }
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                let mut sel_cnt = 0usize;
                for (&ti, hs) in &by_tab {
                    if ti < self.tabs.len() && !hs.is_empty() {
                        self.tabs[ti].scene.select_entities(hs);
                        sel_cnt += hs.len();
                    }
                }
                if eff_action == "TAG"
                    && filtered.len() == 1
                    && filtered[0].tab_index < self.tabs.len()
                    && filtered[0].tab_index != self.active_tab
                {
                    let dt = filtered[0].tab_index;
                    let _ = self.update(Message::TabSwitch(dt));
                    self.active_tab = dt;
                }
                if let Some(cur_hs) = by_tab.get(&self.active_tab).filter(|h| !h.is_empty()) {
                    let _ = self.tabs[self.active_tab].scene.zoom_to_entities(cur_hs);
                }
                self.refresh_properties();
                self.command_line.push_output(&format!(
                    "PIDIO {eff_action}: 匹配 {} 台仪表/执行元件，跨打开图纸高亮 {sel_cnt} 个图元。",
                    filtered.len()
                ));
            }
            "NEXT" => {
                if filtered.is_empty() {
                    self.command_line
                        .push_output("PIDIO NEXT: 未找到可巡检的仪表图元。");
                    return Some(self.finish_dispatch(cmd));
                }
                let cur_tab = self.active_tab;
                let sel_set: std::collections::BTreeSet<document::Handle> = self.tabs[cur_tab]
                    .scene
                    .selected_entities()
                    .into_iter()
                    .map(|(h, _)| h)
                    .collect();
                let cur_pos = filtered
                    .iter()
                    .position(|r| r.tab_index == cur_tab && sel_set.contains(&r.primary_handle));
                let next_idx = cur_pos.map(|p| (p + 1) % filtered.len()).unwrap_or(0);
                let chosen = filtered[next_idx];
                let dest_tab = chosen.tab_index;

                if dest_tab < self.tabs.len() && dest_tab != self.active_tab {
                    let _ = self.update(Message::TabSwitch(dest_tab));
                    self.active_tab = dest_tab;
                }
                for &(ti, _) in &target_tabs {
                    if ti < self.tabs.len() {
                        self.tabs[ti].scene.deselect_all();
                    }
                }
                if dest_tab < self.tabs.len() {
                    self.tabs[dest_tab]
                        .scene
                        .select_entities(&chosen.member_handles);
                    let zoomed = self.tabs[dest_tab]
                        .scene
                        .zoom_to_entities(&chosen.member_handles);
                    if !zoomed {
                        let mut cam = self.tabs[dest_tab].scene.camera.borrow_mut();
                        let z = cam.target.z;
                        cam.target = glam::DVec3::new(chosen.x, chosen.y, z);
                    }
                    self.refresh_properties();
                }
                self.command_line.push_output(&format!(
                    "PIDIO NEXT [{}/{}]: {}（系统={}，通道={}，取源={}）。",
                    next_idx + 1,
                    filtered.len(),
                    chosen.item_tag,
                    chosen.control_system.as_str(),
                    chosen.io_channel.as_str(),
                    chosen.mounted_on,
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDIO: 未知子命令 {other}（支持 SUMMARY / ALL / DCS / SIS / LOCAL / UNCONNECTED / CHANNEL / TAG / NEXT / EXPORT）。"
                ));
                return Some(Task::none());
            }
        }

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
    if !author
        .type_code
        .eq_ignore_ascii_case(crate::pid::topology::TYPE_PIPE_RUN)
    {
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

#[cfg(not(target_arch = "wasm32"))]
fn tokenize_command_words(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in cmd.chars() {
        match quote {
            Some(q) if ch == q => {
                quote = None;
            }
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
            }
            None if ch.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct PidConvertArgs {
    positional: Vec<String>,
    stem: Option<String>,
    plant: Option<String>,
    seed: Option<String>,
    symdir: Option<String>,
    classmap: Option<String>,
    props: Option<String>,
    summary: Option<std::path::PathBuf>,
    force_dual: bool,
    verify: bool,
    raw: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_pidconvert_args(tokens: &[String]) -> Result<PidConvertArgs, String> {
    let mut args = PidConvertArgs::default();
    let mut idx = 0usize;
    while idx < tokens.len() {
        let tok = &tokens[idx];
        let upper = tok.to_ascii_uppercase();
        match upper.as_str() {
            "STEM" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "STEM 缺少主干名称参数".to_string())?;
                args.stem = Some(val.clone());
                idx += 2;
            }
            "PLANT" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "PLANT 缺少装置名称参数".to_string())?;
                args.plant = Some(val.clone());
                idx += 2;
            }
            "SEED" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "SEED 缺少种子文件路径参数".to_string())?;
                args.seed = Some(val.clone());
                idx += 2;
            }
            "SYMDIR" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "SYMDIR 缺少符号目录参数".to_string())?;
                args.symdir = Some(val.clone());
                idx += 2;
            }
            "CLASSMAP" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "CLASSMAP 缺少映射 JSON 路径参数".to_string())?;
                args.classmap = Some(val.clone());
                idx += 2;
            }
            "PROPS" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "PROPS 缺少外置工程表文件路径参数".to_string())?;
                args.props = Some(val.clone());
                idx += 2;
            }
            "SUMMARY" => {
                let val = tokens
                    .get(idx + 1)
                    .ok_or_else(|| "SUMMARY 缺少输出 JSON 路径参数".to_string())?;
                args.summary = Some(std::path::PathBuf::from(val));
                idx += 2;
            }
            "DUAL" => {
                args.force_dual = true;
                idx += 1;
            }
            "VERIFY" => {
                args.verify = true;
                idx += 1;
            }
            "RAW" => {
                args.raw = true;
                idx += 1;
            }
            _ => {
                args.positional.push(tok.clone());
                idx += 1;
            }
        }
    }
    Ok(args)
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
            req["request_id"] = json!(format!(
                "pidprops-{}",
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
        }
        let (response, task) = app.control_request(req.clone());
        app.drive_headless_task(task).unwrap();
        if matches!(response["status"].as_str(), Some("accepted" | "running")) {
            app.control_request(json!({"op":"operation","request_id":req["request_id"]}))
                .0
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
            .map(|section| {
                section["properties"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
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
        assert_eq!(
            row_value(&rows, "pid_type").as_deref(),
            Some(stored.type_code.as_str())
        );
        assert_eq!(
            row_value(&rows, "pid_tag").as_deref(),
            Some(stored.tag.as_str())
        );
        assert_eq!(
            row_value(&rows, "pid_tag_source").as_deref(),
            Some(stored.tag_source.as_str())
        );
        assert_eq!(
            row_value(&rows, "pid_tag_status").as_deref(),
            Some(stored.tag_status.as_str())
        );
        assert_eq!(
            row_value(&rows, "pid_props_设计压力").as_deref(),
            Some("1.6MPa")
        );
        assert_eq!(
            row_value(&rows, "pid_props_设计温度").as_deref(),
            Some("120℃")
        );
        assert!(rows.len() >= 4 + 2, "rows: {rows:?}");
        assert!(rows.iter().any(|row| row["label"] == "设计压力"));

        let line = app
            .commit_entity_handle(document::EntityType::Line(
                document::entities::Line::from_points(
                    document::types::Vector3::ZERO,
                    document::types::Vector3::new(10.0, 0.0, 0.0),
                ),
            ))
            .expect("line commits");
        select(&mut app, line);
        assert!(
            author_rows(&mut app).is_none(),
            "a plain LINE has no P&ID 属性 group"
        );
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
        assert_eq!(
            row_value(&rows, "pid_props_设计压力").as_deref(),
            Some("2.5MPa")
        );
    }

    /// `VAL-DRAW-027` / `028`: a tag edit makes the tag manual, relabels the
    /// canvas, and undo / redo move tag, source and label together.
    #[test]
    fn a_tag_edit_is_manual_and_undoes_with_its_label() {
        let mut app = app();
        let vessel = placed_vessel(&mut app, 200.0);
        let before = author(&app, vessel);
        assert_eq!(
            (before.tag.as_str(), before.tag_source.as_str()),
            ("P-1001", "auto")
        );
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
        assert_eq!(
            (back.tag.as_str(), back.tag_source.as_str()),
            ("P-1001", "auto")
        );
        assert_eq!(label(&app, vessel).as_deref(), Some("P-1001"));
        select(&mut app, vessel);
        let rows = author_rows(&mut app).unwrap();
        assert_eq!(row_value(&rows, "pid_tag").as_deref(), Some("P-1001"));

        let redone = request(&mut app, json!({"op":"redo"}));
        assert_eq!(redone["ok"], true, "{redone}");
        let again = author(&app, vessel);
        assert_eq!(
            (again.tag.as_str(), again.tag_source.as_str()),
            ("P-9001", "manual")
        );
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
        assert_eq!(
            row_value(&rows, "pid_tag_status").as_deref(),
            Some("duplicated")
        );
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
        put_props(&mut app, line, &[("管道等级", "A1A"), ("公称直径", "DN50")]);

        draw(
            &mut app,
            "PIDPIPE run",
            &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]],
        );
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
        put_props(
            &mut app,
            line,
            &[("公称直径", "DN150"), ("管道等级", "A2B")],
        );
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

    // ── #136: L2-1 M4 / L2-2 / L2-3 / L2-4 / L3-7 / L6-2 交互面板与命令闭环 ──

    fn define_sppid_block(doc: &mut document::Document, name: &str, w: f64, h: f64) {
        use document::entities::{Block, BlockEnd, Line, Point};
        use document::tables::BlockRecord;
        use document::types::Vector3;
        if doc.block_records.get(name).is_some() {
            return;
        }
        let next = doc.next_handle();
        let br_h = document::Handle::new(next);
        let b_h = document::Handle::new(next + 1);
        let e_h = document::Handle::new(next + 2);
        let mut rec = BlockRecord::new(name);
        rec.handle = br_h;
        rec.block_entity_handle = b_h;
        rec.block_end_handle = e_h;
        doc.block_records.add(rec).unwrap();
        let mut blk = Block::new(name, Vector3::ZERO);
        blk.common.handle = b_h;
        blk.common.owner_handle = br_h;
        doc.add_entity(document::EntityType::Block(blk)).unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = e_h;
        end.common.owner_handle = br_h;
        doc.add_entity(document::EntityType::BlockEnd(end)).unwrap();
        let mut ln = Line::from_points(
            Vector3::new(-w * 0.5, -h * 0.5, 0.0),
            Vector3::new(w * 0.5, h * 0.5, 0.0),
        );
        ln.common.owner_handle = br_h;
        doc.add_entity(document::EntityType::Line(ln)).unwrap();
        for px in [-w * 0.5, w * 0.5] {
            let mut pt = Point::new();
            pt.location = Vector3::new(px, 0.0, 0.0);
            pt.common.owner_handle = br_h;
            doc.add_entity(document::EntityType::Point(pt)).unwrap();
        }
    }

    fn seed_dxf_sheet_in_app(
        app: &mut OpenCADStudio,
    ) -> (
        document::Handle,
        document::Handle,
        document::Handle,
        document::Handle,
        document::Handle,
        document::Handle,
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        let doc = &mut app.tabs[app.active_tab].scene.document;
        define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
        define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
        define_sppid_block(doc, "GATE VALVE", 6.0, 4.0);

        // High-confidence tagged valve + single-line DWG TEXT
        let mut v = Insert::new("BALL VALVE TYPE 2", Vector3::new(50.0, 100.0, 0.0));
        v.common.layer = "11".to_string();
        let valve_h = doc.add_entity(document::EntityType::Insert(v)).unwrap();
        let mut vt =
            Text::with_value("BV0201501A", Vector3::new(47.0, 106.0, 0.0)).with_height(2.5);
        vt.common.layer = "11".to_string();
        let valve_txt_h = doc.add_entity(document::EntityType::Text(vt)).unwrap();

        // High-confidence 2-line instrument bubble (PT / 02013)
        let mut inst = Insert::new(
            "DCS FUNC ACCESS IN PRIME LOC",
            Vector3::new(110.0, 100.0, 0.0),
        );
        inst.common.layer = "12".to_string();
        let inst_h = doc.add_entity(document::EntityType::Insert(inst)).unwrap();
        let mut top_t = Text::with_value("PT", Vector3::new(108.5, 102.0, 0.0)).with_height(2.2);
        top_t.common.layer = "12".to_string();
        let inst_top_h = doc.add_entity(document::EntityType::Text(top_t)).unwrap();
        let mut bot_t = Text::with_value("02013", Vector3::new(107.0, 97.5, 0.0)).with_height(2.2);
        bot_t.common.layer = "12".to_string();
        let inst_bot_h = doc.add_entity(document::EntityType::Text(bot_t)).unwrap();

        // Medium-confidence untagged valve + connected pipe segment
        let mut gv = Insert::new("BALL VALVE TYPE 2", Vector3::new(170.0, 100.0, 0.0));
        gv.common.layer = "11".to_string();
        let med_gv_h = doc.add_entity(document::EntityType::Insert(gv)).unwrap();
        let mut seg = Line::from_points(
            Vector3::new(53.0, 100.0, 0.0),
            Vector3::new(167.0, 100.0, 0.0),
        );
        seg.common.layer = "11".to_string();
        doc.add_entity(document::EntityType::Line(seg)).unwrap();
        let mut line_lbl =
            Text::with_value("50-PG-02011-B9-", Vector3::new(90.0, 103.0, 0.0)).with_height(2.5);
        line_lbl.common.layer = "11".to_string();
        doc.add_entity(document::EntityType::Text(line_lbl))
            .unwrap();

        (
            valve_h,
            valve_txt_h,
            inst_h,
            inst_top_h,
            inst_bot_h,
            med_gv_h,
        )
    }

    fn section_rows(app: &mut OpenCADStudio, section_title: &str) -> Option<Vec<Value>> {
        let read = request(app, json!({"op":"properties"}));
        let sections = read["sections"]
            .as_array()
            .or_else(|| read["result"]["sections"].as_array())
            .cloned()
            .unwrap_or_default();
        sections
            .into_iter()
            .find(|section| section["title"] == section_title)
            .map(|section| {
                section["properties"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
    }

    fn dwg_text_val(app: &OpenCADStudio, h: document::Handle) -> String {
        match app.tabs[app.active_tab].scene.document.get_entity(h) {
            Some(document::EntityType::Text(t)) => t.value.clone(),
            Some(document::EntityType::MText(mt)) => mt.value.clone(),
            other => panic!("expected Text/MText, got {other:?}"),
        }
    }

    #[test]
    fn l2_m4_l2_3_promoted_symbol_separates_readonly_evidence_group_and_preserves_overflow_metadata_on_edit(
    ) {
        let mut app = app();
        let (valve_h, _, _, _, _, _) = seed_dxf_sheet_in_app(&mut app);
        let _ = app.run_command_line("PIDPROMOTE");

        // Add >2048B business prop so XRecord overflow is exercised too
        {
            let doc = &mut app.tabs[app.active_tab].scene.document;
            let mut auth = PidAuthor::load(doc, valve_h).expect("promoted valve has PID_AUTHOR");
            let mut pmap: serde_json::Map<String, Value> =
                serde_json::from_str(&auth.props).unwrap();
            pmap.insert("公称直径".to_string(), json!("50 mm"));
            pmap.insert("备注长文".to_string(), json!("N".repeat(2400)));
            auth.props = Value::Object(pmap).to_string();
            auth.store(doc, valve_h);
        }

        select(&mut app, valve_h);
        let edit_rows = author_rows(&mut app).expect("editable P&ID 属性 group");
        let ev_rows = section_rows(&mut app, PID_EVIDENCE_SECTION)
            .expect("read-only P&ID 来源证据 group must exist for promoted symbols");

        // Internal `_` keys must NOT leak into editable P&ID 属性 rows
        assert!(
            edit_rows
                .iter()
                .all(|r| !r["label"].as_str().unwrap_or("").starts_with('_')),
            "internal keys leaked into editable rows: {edit_rows:?}"
        );
        assert_eq!(
            row_value(&edit_rows, "pid_props_公称直径").as_deref(),
            Some("50 mm")
        );
        assert_eq!(
            row_value(&ev_rows, "pid_ev_source").as_deref(),
            Some("convert:dxf")
        );
        assert_eq!(
            row_value(&ev_rows, "pid_ev_sppid_class").as_deref(),
            Some("PIDPipingComponent")
        );

        // Editing a business property preserves overflowed internal evidence keys
        set(&mut app, "pid_props_公称直径", "80 mm");
        let after = author(&app, valve_h);
        let after_map: serde_json::Map<String, Value> = serde_json::from_str(&after.props).unwrap();
        assert_eq!(
            after_map.get("公称直径").and_then(Value::as_str),
            Some("80 mm")
        );
        assert_eq!(
            after_map.get("_sppid_class").and_then(Value::as_str),
            Some("PIDPipingComponent")
        );
        assert_eq!(
            after_map
                .get("备注长文")
                .and_then(Value::as_str)
                .map(str::len),
            Some(2400)
        );
    }

    #[test]
    fn l2_4_editing_promoted_tag_updates_paired_single_and_two_line_dwg_texts_and_undoes_atomically(
    ) {
        let mut app = app();
        let (valve_h, valve_txt_h, inst_h, inst_top_h, inst_bot_h, _) =
            seed_dxf_sheet_in_app(&mut app);
        let _ = app.run_command_line("PIDPROMOTE");

        // 1. Single-line valve tag edit updates paired DWG Text and UNDO restores both
        select(&mut app, valve_h);
        set(&mut app, "pid_tag", "BV-9901A");
        assert_eq!(author(&app, valve_h).tag, "BV-9901A");
        assert_eq!(author(&app, valve_h).tag_source, "manual");
        assert_eq!(dwg_text_val(&app, valve_txt_h), "BV-9901A");

        let _ = request(&mut app, json!({"op":"undo"}));
        assert_eq!(author(&app, valve_h).tag, "BV0201501A");
        assert_eq!(dwg_text_val(&app, valve_txt_h), "BV0201501A");

        let _ = request(&mut app, json!({"op":"redo"}));
        assert_eq!(dwg_text_val(&app, valve_txt_h), "BV-9901A");

        // 2. Two-line instrument bubble tag edit splits across top & bottom DWG Texts
        select(&mut app, inst_h);
        set(&mut app, "pid_tag", "FT-08888");
        assert_eq!(author(&app, inst_h).tag, "FT-08888");
        assert_eq!(dwg_text_val(&app, inst_top_h), "FT");
        assert_eq!(dwg_text_val(&app, inst_bot_h), "08888");

        let _ = request(&mut app, json!({"op":"undo"}));
        assert_eq!(dwg_text_val(&app, inst_top_h), "PT");
        assert_eq!(dwg_text_val(&app, inst_bot_h), "02013");
    }

    #[test]
    fn l3_7_pidpromote_and_pidreview_filter_accept_reject_and_undo_in_app() {
        let mut app = app();
        let (valve_h, _, _, _, _, med_gv_h) = seed_dxf_sheet_in_app(&mut app);
        let _ = app.run_command_line("PIDPROMOTE");

        // Medium-confidence untagged gate valve is selected by PIDREVIEW MEDIUM
        let _ = app.run_command_line("PIDREVIEW MEDIUM");
        let sel = app.tabs[app.active_tab].scene.selected_handles_in_order();
        assert_eq!(sel, vec![med_gv_h]);

        // PIDREVIEW ACCEPT marks it manual + confirmed
        let _ = app.run_command_line("PIDREVIEW ACCEPT");
        let accepted = author(&app, med_gv_h);
        assert_eq!(accepted.tag_source, "manual");
        let acc_map: serde_json::Map<String, Value> =
            serde_json::from_str(&accepted.props).unwrap();
        assert_eq!(
            acc_map.get("_confidence_tier").and_then(Value::as_str),
            Some("confirmed")
        );

        // Undo ACCEPT -> back to medium, then REJECT removes PID_AUTHOR and prunes topology peer
        let _ = request(&mut app, json!({"op":"undo"}));
        select(&mut app, med_gv_h);
        let _ = app.run_command_line("PIDREVIEW REJECT");
        assert!(
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, med_gv_h).is_none(),
            "rejected symbol must lose PID_AUTHOR"
        );
        let med_hex = hex(med_gv_h);
        let topo = TopologyTable::load(&app.tabs[app.active_tab].scene.document);
        assert!(topo.fittings.iter().all(|f| f.handle != med_hex));
        assert!(PidAuthor::load(&app.tabs[app.active_tab].scene.document, valve_h).is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l6_2_pidsavepid_blocks_duplicate_tags_then_writes_valid_dual_pid_and_xml_after_fix() {
        use pid_parse::{PidDocument, PidPackage, PidWriter, RawStream, WritePlan};
        fn wrap_psm(type_code: u16, payload: &[u8]) -> Vec<u8> {
            let mut v = Vec::with_capacity(6 + payload.len());
            v.extend_from_slice(&0x0001u16.to_le_bytes());
            v.extend_from_slice(&type_code.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
            v
        }
        fn make_layer(oid: u32, name: &str) -> Vec<u8> {
            let u16s: Vec<u16> = name.encode_utf16().collect();
            let mut p = Vec::new();
            p.extend_from_slice(&oid.to_le_bytes());
            p.extend_from_slice(&[0u8; 8]);
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&6u32.to_le_bytes());
            p.extend_from_slice(&(u16s.len() as u16).to_le_bytes());
            for u in u16s {
                p.extend_from_slice(&u.to_le_bytes());
            }
            p.extend_from_slice(&0u16.to_le_bytes());
            wrap_psm(0x0081, &p)
        }
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());
        let mut cluster0 = 0x6C90_F544u32.to_le_bytes().to_vec();
        cluster0.extend_from_slice(&2u32.to_le_bytes());
        cluster0.extend_from_slice(&make_layer(8, "Default"));
        cluster0.extend_from_slice(&make_layer(12, "Labels"));
        let mut streams = std::collections::BTreeMap::new();
        for (p, data) in [
            ("/Sheet6", empty_chain.clone()),
            ("/PSMcluster0", cluster0),
            ("/Unclustered Dynamic Attributes", empty_chain.clone()),
            ("/StyleCluster", empty_chain),
        ] {
            streams.insert(
                p.to_string(),
                RawStream {
                    path: p.to_string(),
                    data,
                    modified: false,
                },
            );
        }
        streams.insert(
            "/TaggedTxtData/Revision".to_string(),
            RawStream {
                path: "/TaggedTxtData/Revision".to_string(),
                data: Vec::new(),
                modified: false,
            },
        );
        let mut roots = 0x746F_6F72u32.to_le_bytes().to_vec();
        roots.extend_from_slice(&[0u8; 8]);
        streams.insert(
            "/PSMroots".to_string(),
            RawStream {
                path: "/PSMroots".to_string(),
                data: roots,
                modified: false,
            },
        );
        let mut tseg = 0x6765_7374u32.to_le_bytes().to_vec();
        tseg.extend_from_slice(&0u16.to_le_bytes());
        tseg.extend_from_slice(&4u16.to_le_bytes());
        tseg.extend_from_slice(&32u16.to_le_bytes());
        tseg.extend_from_slice(&0u16.to_le_bytes());
        streams.insert(
            "/PSMspacemap/0x00000000".to_string(),
            RawStream {
                path: "/PSMspacemap/0x00000000".to_string(),
                data: tseg,
                modified: false,
            },
        );
        let seed_bytes = PidWriter::write_to_bytes(
            &PidPackage::new(None, streams, PidDocument::default()),
            &WritePlan::default(),
        )
        .unwrap();

        let dir = std::env::temp_dir().join(format!("ocs_pidsavepid_136_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seed_path = dir.join("seed.pid");
        std::fs::write(&seed_path, seed_bytes).unwrap();
        let target_pid = dir.join("OUT-136.pid");

        let mut app = app();
        let (valve_h, _, _, _, _, med_gv_h) = seed_dxf_sheet_in_app(&mut app);
        let _ = app.run_command_line("PIDPROMOTE");

        // Create duplicate tag conflict on med_gv_h -> PIDSAVEPID must block without writing file
        select(&mut app, med_gv_h);
        let taken = author(&app, valve_h).tag;
        set(&mut app, "pid_tag", &taken);
        assert_eq!(author(&app, med_gv_h).tag_status, "duplicated");

        let cmd_blocked = format!(
            "PIDSAVEPID \"{}\" SEED \"{}\"",
            target_pid.display(),
            seed_path.display()
        );
        let _ = app.run_command_line(&cmd_blocked);
        assert!(
            !target_pid.exists(),
            "PIDSAVEPID must refuse saving while duplicated tags exist"
        );

        // Fix duplicate tag -> PIDSAVEPID writes valid .pid + _Data.xml + _Meta.xml
        set(&mut app, "pid_tag", "GV-0201999");
        assert_eq!(author(&app, med_gv_h).tag_status, "ok");
        let _ = app.run_command_line(&cmd_blocked);
        assert!(target_pid.exists(), "PIDSAVEPID must write target .pid");
        let data_xml = dir.join("OUT-136_Data.xml");
        assert!(data_xml.exists(), "PIDSAVEPID must write _Data.xml");
        let xml_text = std::fs::read_to_string(&data_xml).unwrap();
        assert!(xml_text.contains("GV-0201999"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real production DXF inside OpenCADStudio through PIDPROMOTE -> PIDREVIEW -> property edit -> PIDSAVEPID"]
    fn real_sheet_in_app_promote_review_edit_and_pidsavepid_roundtrip() {
        let Ok(dxf_path) = std::env::var("OCS_PID_REAL_DXF") else {
            println!("OCS_PID_REAL_DXF unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL136_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real136"));
        let _ = std::fs::create_dir_all(&out_dir);

        let mut app = app();
        let loaded = crate::io::load_file(std::path::Path::new(&dxf_path)).expect("load real DXF");
        app.tabs[app.active_tab].scene.document = loaded;

        let _ = app.run_command_line("PIDPROMOTE");
        let stats_before = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert!(stats_before.total_authored_symbols > 0);

        let _ = app.run_command_line("PIDREVIEW MEDIUM");
        let med_selected = app.tabs[app.active_tab]
            .scene
            .selected_handles_in_order()
            .len();
        assert_eq!(med_selected, stats_before.medium);
        if med_selected > 0 {
            let _ = app.run_command_line("PIDREVIEW ACCEPT");
        }
        let stats_after = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(stats_after.medium, 0);
        assert_eq!(stats_after.confirmed, med_selected);

        // Pick a promoted symbol that has a paired DWG text handle and edit its tag via panel op
        let target_sym = app.tabs[app.active_tab]
            .scene
            .document
            .model_space_entities()
            .find_map(|e| {
                let h = e.common().handle;
                let a = PidAuthor::load(&app.tabs[app.active_tab].scene.document, h)?;
                let m: serde_json::Map<String, Value> = serde_json::from_str(&a.props).ok()?;
                let th = m.get("_tag_handles")?.as_str()?.to_string();
                (!a.tag.is_empty() && !th.is_empty()).then_some((h, th))
            })
            .expect("find promoted symbol with paired DWG text");
        select(&mut app, target_sym.0);
        let ev_rows = section_rows(&mut app, PID_EVIDENCE_SECTION)
            .expect("real promoted symbol has P&ID 来源证据 section");
        assert_eq!(
            row_value(&ev_rows, "pid_ev_source").as_deref(),
            Some("convert:dxf")
        );
        set(&mut app, "pid_tag", "BV-INAPP-136");

        let first_txt_hex = target_sym.1.split(',').next().unwrap();
        let first_txt_h = document::Handle::new(u64::from_str_radix(first_txt_hex, 16).unwrap());
        assert!(dwg_text_val(&app, first_txt_h).contains("BV"));

        let target_pid = out_dir.join("REAL-INAPP-136.pid");
        let save_cmd = if sym_dir.trim().is_empty() {
            format!(
                "PIDSAVEPID \"{}\" SEED \"{}\"",
                target_pid.display(),
                seed_zip
            )
        } else {
            format!(
                "PIDSAVEPID \"{}\" SEED \"{}\" SYMDIR \"{}\"",
                target_pid.display(),
                seed_zip,
                sym_dir
            )
        };
        let _ = app.run_command_line(&save_cmd);
        assert!(target_pid.exists(), "REAL-INAPP-136.pid must exist");
        let data_xml = out_dir.join("REAL-INAPP-136_Data.xml");
        let cov_json = out_dir.join("REAL-INAPP-136_Coverage.json");
        assert!(data_xml.exists());
        assert!(cov_json.exists());
        let xml_str = std::fs::read_to_string(&data_xml).unwrap();
        assert!(xml_str.contains("BV-INAPP-136"));

        let cov_val: Value =
            serde_json::from_str(&std::fs::read_to_string(&cov_json).unwrap()).unwrap();
        let doc_uid = cov_val["doc_uid"]
            .as_str()
            .expect("doc_uid in coverage")
            .to_string();
        let pid_raw = std::fs::read(&target_pid).unwrap();
        let xml_raw = std::fs::read(&data_xml).unwrap();
        let dual_ver = crate::io::pid_convert::verify_dual_bundle(
            &doc_uid,
            &xml_raw,
            &pid_raw,
            Some(&out_dir),
            "REAL-INAPP-136",
        )
        .expect("verify_dual_bundle on PIDSAVEPID output");
        assert!(dual_ver.is_cross_file_and_a_valid(), "{dual_ver:?}");
        assert!(dual_ver.is_b_plot_valid(), "{dual_ver:?}");

        let summary = json!({
            "stats_before": stats_before,
            "med_selected_and_accepted": med_selected,
            "stats_after": stats_after,
            "edited_tag": "BV-INAPP-136",
            "pid_bytes": pid_raw.len(),
            "data_xml_bytes": xml_raw.len(),
            "dual_verify": dual_ver,
        });
        std::fs::write(
            out_dir.join("real136_summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn pidreview_expands_exploded_strokes_accepts_member_stroke_and_dedups_conflicts() {
        use document::entities::{Circle, Line, LwPolyline, LwVertex, Text};
        use document::types::Vector3;
        use document::EntityType;

        let mut app = app();
        {
            let doc = &mut app.tabs[app.active_tab].scene.document;
            let mut frame = LwPolyline::new();
            frame.is_closed = true;
            frame.common.layer = "PUB_TITLE".to_string();
            for (x, y) in [(0.0, 0.0), (420.0, 0.0), (420.0, 297.0), (0.0, 297.0)] {
                frame.vertices.push(LwVertex::from_coords(x, y));
            }
            doc.add_entity(EntityType::LwPolyline(frame)).unwrap();

            // Two exploded 4-stroke bowtie valves sharing duplicate tag `BV3101`
            for &cx in &[80.0, 130.0] {
                for (a, b) in [
                    ((cx - 3.0, 98.0), (cx - 3.0, 102.0)),
                    ((cx - 3.0, 102.0), (cx + 3.0, 98.0)),
                    ((cx + 3.0, 98.0), (cx + 3.0, 102.0)),
                    ((cx + 3.0, 102.0), (cx - 3.0, 98.0)),
                ] {
                    let mut st =
                        Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
                    st.common.layer = "DEVICE".to_string();
                    doc.add_entity(EntityType::Line(st)).unwrap();
                }
                let mut vt =
                    Text::with_value("BV3101", Vector3::new(cx - 3.0, 104.0, 0.0)).with_height(2.2);
                vt.common.layer = "TEXT".to_string();
                doc.add_entity(EntityType::Text(vt)).unwrap();
            }

            // One untagged loose `Circle` pump (`r = 5.5` -> `Medium` tier)
            let mut pump_c = Circle::from_coords(190.0, 100.0, 0.0, 5.5);
            pump_c.common.layer = "0".to_string();
            doc.add_entity(EntityType::Circle(pump_c)).unwrap();
        }

        let _ = app.run_command_line("PIDPROMOTE");
        let rev0 = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(rev0.total_authored_symbols, 3);
        assert_eq!(rev0.medium, 1);
        assert_eq!(rev0.duplicated_tags, 2);

        // `PIDREVIEW DUPLICATES` expands `_member_handles` (2 valves x 2 member handles = 4 entities > 2 primary handles)
        let _ = app.run_command_line("PIDREVIEW DUPLICATES");
        let dup_selected = app.tabs[app.active_tab].scene.selected_handles_in_order();
        assert_eq!(dup_selected.len(), 4);

        // Selecting ONLY a non-primary member stroke of the first exploded valve and running `PIDREVIEW ACCEPT` resolves to its owner
        let dup_owners = crate::io::pid_convert::filter_author_handles(
            &app.tabs[app.active_tab].scene.document,
            "DUPLICATES",
        );
        let non_primary_stroke = *dup_selected
            .iter()
            .find(|h| !dup_owners.contains(h))
            .expect("member stroke distinct from primary handle");
        select(&mut app, non_primary_stroke);
        let _ = app.run_command_line("PIDREVIEW ACCEPT");

        let rev1 = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(rev1.confirmed, 1);

        // `PIDREVIEW DEDUP` resolves the duplicate `BV3101` conflict in one undoable step
        let _ = app.run_command_line("PIDREVIEW DEDUP");
        let rev2 = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(rev2.duplicated_tags, 0);
        assert!(crate::pid::propagate::gate_save(&app.tabs[app.active_tab].scene.document).is_ok());

        let _ = request(&mut app, json!({"op":"undo"}));
        let rev_undo = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(rev_undo.duplicated_tags, 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs all 12 real native AutoCAD legacy sheets inside OpenCADStudio through PIDPROMOTE -> PIDREVIEW -> PIDSAVEPID"]
    fn real_legacy_12_in_app_promote_review_dedup_and_pidsavepid_roundtrip() {
        use crate::io::pid_publish_check::{
            check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
        };
        use sha2::{Digest, Sha256};

        let Ok(legacy_dir) = std::env::var("OCS_PID_LEGACY12_DIR") else {
            println!("OCS_PID_LEGACY12_DIR unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL139_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real139"));
        let _ = std::fs::create_dir_all(&out_dir);

        let mut dxf_files: Vec<std::path::PathBuf> = std::fs::read_dir(&legacy_dir)
            .expect("read legacy dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
                    && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
            })
            .collect();
        dxf_files.sort();
        assert_eq!(dxf_files.len(), 12, "expected 12 legacy DXF files");

        let spec = default_publish_spec();
        let chk = CheckOptions {
            expected_plant: Some("P01".to_string()),
            ..CheckOptions::default()
        };

        let mut sheet_rows = Vec::new();
        let mut tot_promoted = 0usize;
        let mut tot_confirmed = 0usize;
        let mut tot_deduped = 0usize;
        let mut tot_exact_variants = 0usize;
        let mut tot_fallbacks = 0usize;
        let mut tot_items = 0usize;
        let mut tot_relations = 0usize;
        let mut tot_xml_reps = 0usize;
        let mut tot_matched_oids = 0usize;
        let mut tot_pdf_prims = 0usize;

        for (idx, path) in dxf_files.iter().enumerate() {
            let lid = format!("L{:02}", idx + 1);
            let raw_dxf = std::fs::read(path).expect("read dxf bytes");
            let sha_hex = format!("{:x}", Sha256::digest(&raw_dxf));
            let sha_pfx = sha_hex[..12].to_string();

            let mut app = app();
            let loaded = crate::io::load_file(path).expect("load legacy dxf");
            app.tabs[app.active_tab].scene.document = loaded;

            let _ = app.run_command_line("PIDPROMOTE");
            let stats_before = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert!(
                stats_before.total_authored_symbols > 0,
                "{lid}: expected promoted symbols > 0"
            );

            let _ = app.run_command_line("PIDREVIEW MEDIUM");
            let med_entities_selected = app.tabs[app.active_tab]
                .scene
                .selected_handles_in_order()
                .len();
            assert!(med_entities_selected >= stats_before.medium);
            if stats_before.medium > 0 {
                let _ = app.run_command_line("PIDREVIEW ACCEPT");
            }

            let dup_before = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            )
            .duplicated_tags;
            if dup_before > 0 {
                let _ = app.run_command_line("PIDREVIEW DEDUP");
            }
            let stats_after = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert_eq!(
                stats_after.medium, 0,
                "{lid}: medium must be 0 after ACCEPT"
            );
            assert_eq!(
                stats_after.duplicated_tags, 0,
                "{lid}: duplicated_tags must be 0 after DEDUP"
            );

            let sheet_out = out_dir.join(&lid);
            let _ = std::fs::create_dir_all(&sheet_out);
            let target_pid = sheet_out.join(format!("{lid}.pid"));
            let save_cmd = if sym_dir.trim().is_empty() {
                format!(
                    "PIDSAVEPID \"{}\" SEED \"{}\"",
                    target_pid.display(),
                    seed_zip
                )
            } else {
                format!(
                    "PIDSAVEPID \"{}\" SEED \"{}\" SYMDIR \"{}\"",
                    target_pid.display(),
                    seed_zip,
                    sym_dir
                )
            };
            let _ = app.run_command_line(&save_cmd);
            assert!(target_pid.exists(), "{lid}: target .pid must exist");

            let data_xml_p = sheet_out.join(format!("{lid}_Data.xml"));
            let meta_xml_p = sheet_out.join(format!("{lid}_Meta.xml"));
            let cov_json_p = sheet_out.join(format!("{lid}_Coverage.json"));
            let data_raw = std::fs::read(&data_xml_p).expect("read _Data.xml");
            let meta_raw = std::fs::read(&meta_xml_p).expect("read _Meta.xml");
            let pid_raw = std::fs::read(&target_pid).expect("read .pid");

            let v_data = check_publish_xml_bytes(
                &format!("{lid}_Data.xml"),
                PublishXmlKind::Data,
                &data_raw,
                spec,
                &chk,
            );
            let v_meta = check_publish_xml_bytes(
                &format!("{lid}_Meta.xml"),
                PublishXmlKind::Meta,
                &meta_raw,
                spec,
                &chk,
            );
            assert!(
                v_data.is_empty(),
                "{lid} _Data.xml spec violations: {v_data:?}"
            );
            assert!(
                v_meta.is_empty(),
                "{lid} _Meta.xml spec violations: {v_meta:?}"
            );

            let cov_val: Value =
                serde_json::from_str(&std::fs::read_to_string(&cov_json_p).unwrap()).unwrap();
            let doc_uid = cov_val["doc_uid"].as_str().unwrap().to_string();
            let exact_v = cov_val["variant_resolution"]["exact_variant_id"]
                .as_u64()
                .unwrap_or(0) as usize;
            let fb_v = cov_val["variant_resolution"]["class_default_fallback"]
                .as_u64()
                .unwrap_or(0) as usize;
            let items_w = cov_val["total_items_written"].as_u64().unwrap_or(0) as usize;
            let rels_w = cov_val["total_relations_written"].as_u64().unwrap_or(0) as usize;
            assert_eq!(fb_v, 0, "{lid}: fallback must be 0");
            assert_eq!(
                exact_v, stats_after.total_authored_symbols,
                "{lid}: all promoted symbols must be preserved on PIDSAVEPID"
            );

            let dual_ver = crate::io::pid_convert::verify_dual_bundle(
                &doc_uid,
                &data_raw,
                &pid_raw,
                Some(&sheet_out),
                &lid,
            )
            .expect("verify_dual_bundle");
            assert!(dual_ver.is_cross_file_and_a_valid(), "{lid}: {dual_ver:?}");
            assert!(dual_ver.is_b_plot_valid(), "{lid}: {dual_ver:?}");

            tot_promoted += stats_after.total_authored_symbols;
            tot_confirmed += stats_after.confirmed;
            tot_deduped += dup_before;
            tot_exact_variants += exact_v;
            tot_fallbacks += fb_v;
            tot_items += items_w;
            tot_relations += rels_w;
            tot_xml_reps += dual_ver.xml_rep_count;
            tot_matched_oids += dual_ver.matched_graphic_oids;
            tot_pdf_prims += dual_ver.b_level_pdf_primitives.unwrap_or(0);

            sheet_rows.push(json!({
                "id": lid,
                "dxf_sha256_prefix": sha_pfx,
                "authored_symbols": stats_after.total_authored_symbols,
                "high_before": stats_before.high,
                "medium_accepted": stats_after.confirmed,
                "med_entities_selected": med_entities_selected,
                "duplicates_before_dedup": dup_before,
                "exact_variants_saved": exact_v,
                "fallback": fb_v,
                "items_written": items_w,
                "relations_written": rels_w,
                "xml_reps": dual_ver.xml_rep_count,
                "matched_oids": dual_ver.matched_graphic_oids,
                "a_valid": dual_ver.a_level_index_consistent,
                "pdf_bytes": dual_ver.b_level_pdf_bytes,
                "pdf_prims": dual_ver.b_level_pdf_primitives,
            }));
        }

        let summary = json!({
            "sheets_tested": sheet_rows.len(),
            "total_authored_symbols": tot_promoted,
            "total_medium_confirmed": tot_confirmed,
            "total_duplicates_before_dedup": tot_deduped,
            "total_exact_variants_saved": tot_exact_variants,
            "total_fallbacks": tot_fallbacks,
            "total_items_written": tot_items,
            "total_relations_written": tot_relations,
            "total_xml_reps": tot_xml_reps,
            "total_matched_oids": tot_matched_oids,
            "total_pdf_primitives": tot_pdf_prims,
            "sheets": sheet_rows,
        });
        std::fs::write(
            out_dir.join("real139_legacy12_inapp_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&summary).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn make_synthetic_seed_pid_bytes() -> Vec<u8> {
        use pid_parse::{PidDocument, PidPackage, PidWriter, RawStream, WritePlan};
        fn wrap_psm(type_code: u16, payload: &[u8]) -> Vec<u8> {
            let mut v = Vec::with_capacity(6 + payload.len());
            v.extend_from_slice(&0x0001u16.to_le_bytes());
            v.extend_from_slice(&type_code.to_le_bytes());
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(payload);
            v
        }
        fn make_layer(oid: u32, name: &str) -> Vec<u8> {
            let u16s: Vec<u16> = name.encode_utf16().collect();
            let mut p = Vec::new();
            p.extend_from_slice(&oid.to_le_bytes());
            p.extend_from_slice(&[0u8; 8]);
            p.extend_from_slice(&0u16.to_le_bytes());
            p.extend_from_slice(&6u32.to_le_bytes());
            p.extend_from_slice(&(u16s.len() as u16).to_le_bytes());
            for u in u16s {
                p.extend_from_slice(&u.to_le_bytes());
            }
            p.extend_from_slice(&0u16.to_le_bytes());
            wrap_psm(0x0081, &p)
        }
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());
        let mut cluster0 = 0x6C90_F544u32.to_le_bytes().to_vec();
        cluster0.extend_from_slice(&2u32.to_le_bytes());
        cluster0.extend_from_slice(&make_layer(8, "Default"));
        cluster0.extend_from_slice(&make_layer(12, "Labels"));
        let mut streams = std::collections::BTreeMap::new();
        for (p, data) in [
            ("/Sheet6", empty_chain.clone()),
            ("/PSMcluster0", cluster0),
            ("/Unclustered Dynamic Attributes", empty_chain.clone()),
            ("/StyleCluster", empty_chain),
        ] {
            streams.insert(
                p.to_string(),
                RawStream {
                    path: p.to_string(),
                    data,
                    modified: false,
                },
            );
        }
        streams.insert(
            "/TaggedTxtData/Revision".to_string(),
            RawStream {
                path: "/TaggedTxtData/Revision".to_string(),
                data: Vec::new(),
                modified: false,
            },
        );
        let mut roots = 0x746F_6F72u32.to_le_bytes().to_vec();
        roots.extend_from_slice(&[0u8; 8]);
        streams.insert(
            "/PSMroots".to_string(),
            RawStream {
                path: "/PSMroots".to_string(),
                data: roots,
                modified: false,
            },
        );
        let mut tseg = 0x6765_7374u32.to_le_bytes().to_vec();
        tseg.extend_from_slice(&0u16.to_le_bytes());
        tseg.extend_from_slice(&4u16.to_le_bytes());
        tseg.extend_from_slice(&32u16.to_le_bytes());
        tseg.extend_from_slice(&0u16.to_le_bytes());
        streams.insert(
            "/PSMspacemap/0x00000000".to_string(),
            RawStream {
                path: "/PSMspacemap/0x00000000".to_string(),
                data: tseg,
                modified: false,
            },
        );
        PidWriter::write_to_bytes(
            &PidPackage::new(None, streams, PidDocument::default()),
            &WritePlan::default(),
        )
        .unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l4_6_pidconvert_exports_current_unpromoted_sheet_xml_and_dual_verify_without_mutating_canvas(
    ) {
        use crate::io::pid_publish_check::{
            check_publish_xml_bytes, default_publish_spec, CheckOptions, PublishXmlKind,
        };

        let root = std::env::temp_dir().join(format!("ocs_pidconvert_141_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let seed_path = root.join("seed.pid");
        std::fs::write(&seed_path, make_synthetic_seed_pid_bytes()).unwrap();

        let mut app = app();
        let (valve_h, _, _, _, _, _) = seed_dxf_sheet_in_app(&mut app);
        app.tabs[app.active_tab].current_path = Some(root.join("S141-A.dxf"));
        app.tabs[app.active_tab].dirty = false;

        // 1. XML-only export without seed or prior PIDPROMOTE; VERIFY enforces StrictVendor
        let xml_dir = root.join("xml_only");
        let cmd_xml = format!("PIDCONVERT \"{}\" PLANT P02 VERIFY", xml_dir.display());
        let _ = app.run_command_line(&cmd_xml);

        let data_xml = xml_dir.join("S141-A_Data.xml");
        let meta_xml = xml_dir.join("S141-A_Meta.xml");
        let cov_json = xml_dir.join("S141-A_Coverage.json");
        assert!(data_xml.is_file(), "PIDCONVERT must write S141-A_Data.xml");
        assert!(meta_xml.is_file(), "PIDCONVERT must write S141-A_Meta.xml");
        assert!(
            cov_json.is_file(),
            "PIDCONVERT must write S141-A_Coverage.json"
        );
        assert!(
            !xml_dir.join("S141-A.pid").exists(),
            "directory PIDCONVERT without SEED/DUAL must stay XML-only"
        );
        assert!(
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, valve_h).is_none(),
            "PIDCONVERT must be read-only on active canvas without injecting PID_AUTHOR"
        );
        assert!(!app.tabs[app.active_tab].dirty);

        let xml_raw = std::fs::read(&data_xml).unwrap();
        let xml_str = std::str::from_utf8(&xml_raw).unwrap();
        assert!(xml_str.contains("BV0201501A"));
        assert!(xml_str.contains("PT-02013"));
        let v_p02 = check_publish_xml_bytes(
            "S141-A_Data.xml",
            PublishXmlKind::Data,
            &xml_raw,
            default_publish_spec(),
            &CheckOptions {
                expected_plant: Some("P02".to_string()),
                ..CheckOptions::default()
            },
        );
        assert!(v_p02.is_empty(), "{v_p02:?}");

        // 2. Dual `.pid` + XML + `.pidplot.pdf` with SEED + VERIFY
        let dual_target = root.join("dual").join("S141-DUAL.pid");
        let cmd_dual = format!(
            "PIDCONVERT \"{}\" SEED \"{}\" VERIFY",
            dual_target.display(),
            seed_path.display()
        );
        let _ = app.run_command_line(&cmd_dual);
        assert!(dual_target.is_file(), "PIDCONVERT must write S141-DUAL.pid");
        let dual_pdf = root.join("dual").join("S141-DUAL.pidplot.pdf");
        assert!(
            dual_pdf.is_file(),
            "PIDCONVERT VERIFY with SEED must write b-level .pidplot.pdf"
        );
        let dual_cov: Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("dual").join("S141-DUAL_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            dual_cov["pid_verification"]["a_level_index_consistent"].as_bool(),
            Some(true)
        );
        assert!(
            dual_cov["pid_verification"]["b_level_pdf_primitives"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );

        // 3. CLASSMAP: invalid path blocks output; valid external class-map succeeds
        let bad_dir = root.join("bad_map");
        let cmd_bad = format!(
            "PIDCONVERT \"{}\" CLASSMAP \"{}\"",
            bad_dir.display(),
            root.join("missing_map.json").display()
        );
        let _ = app.run_command_line(&cmd_bad);
        assert!(
            !bad_dir.join("S141-A_Data.xml").exists(),
            "missing CLASSMAP must block export"
        );

        let custom_map_path = root.join("custom-class-map.json");
        std::fs::write(
            &custom_map_path,
            include_str!("../../../assets/pid-class-map.json"),
        )
        .unwrap();
        let cm_dir = root.join("cm_ok");
        let cmd_cm = format!(
            "PIDCONVERT \"{}\" STEM S141-CM CLASSMAP \"{}\" VERIFY",
            cm_dir.display(),
            custom_map_path.display()
        );
        let _ = app.run_command_line(&cmd_cm);
        assert!(cm_dir.join("S141-CM_Data.xml").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l4_6_pidconvert_batch_converts_mixed_dxf_and_binary_dwg_directory_with_summary_and_verify() {
        let root =
            std::env::temp_dir().join(format!("ocs_pidconvert_batch_141_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let in_dir = root.join("in_sheets");
        let out_dir = root.join("out_bundles");
        std::fs::create_dir_all(&in_dir).unwrap();

        let seed_path = root.join("seed.pid");
        std::fs::write(&seed_path, make_synthetic_seed_pid_bytes()).unwrap();

        let mut tmp_app = app();
        let _ = seed_dxf_sheet_in_app(&mut tmp_app);
        let dxf_a = in_dir.join("BATCH-01.dxf");
        crate::io::save_as_version(
            &tmp_app.tabs[tmp_app.active_tab].scene.document,
            &dxf_a,
            document::DxfVersion::AC1032,
        )
        .unwrap();

        let dwg_b = in_dir.join("BATCH-02.dwg");
        crate::io::pid_convert::save_cad_file_as_binary_dwg(&dxf_a, &dwg_b).unwrap();
        assert!(std::fs::read(&dwg_b).unwrap().starts_with(b"AC10"));

        // 0-byte empty DXF must be skipped gracefully
        std::fs::write(in_dir.join("EMPTY-SKIP.dxf"), b"").unwrap();

        let summary_path = root.join("batch_summary.json");
        let mut runner = app();
        let cmd = format!(
            "PIDCONVERT BATCH \"{}\" \"{}\" SEED \"{}\" VERIFY SUMMARY \"{}\"",
            in_dir.display(),
            out_dir.display(),
            seed_path.display(),
            summary_path.display()
        );
        let _ = runner.run_command_line(&cmd);

        assert!(summary_path.is_file(), "BATCH SUMMARY JSON must be written");
        let sum: Value =
            serde_json::from_str(&std::fs::read_to_string(&summary_path).unwrap()).unwrap();
        assert_eq!(sum["files_converted"].as_u64(), Some(2));
        assert_eq!(sum["files_failed"].as_u64(), Some(0));
        assert_eq!(sum["pid_files_written"].as_u64(), Some(2));
        assert_eq!(sum["pdf_files_plotted"].as_u64(), Some(2));

        for stem in ["BATCH-01", "BATCH-02"] {
            assert!(out_dir.join(format!("{stem}_Data.xml")).is_file());
            assert!(out_dir.join(format!("{stem}_Meta.xml")).is_file());
            assert!(out_dir.join(format!("{stem}_Coverage.json")).is_file());
            assert!(out_dir.join(format!("{stem}.pid")).is_file());
            assert!(out_dir.join(format!("{stem}.pidplot.pdf")).is_file());
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real SPPID DXF, native AutoCAD legacy sheets, and binary AC1032 .dwg inside OpenCADStudio via PIDCONVERT and PIDCONVERT BATCH"]
    fn real_multi_source_in_app_pidconvert_single_and_batch_roundtrip() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_med) = std::env::var("OCS_PID_REAL141_SPPID_MED") else {
            println!("OCS_PID_REAL141_SPPID_MED unset, skipping.");
            return;
        };
        let Ok(sppid_small) = std::env::var("OCS_PID_REAL141_SPPID_SMALL") else {
            println!("OCS_PID_REAL141_SPPID_SMALL unset, skipping.");
            return;
        };
        let Ok(leg_block) = std::env::var("OCS_PID_REAL141_LEG_BLOCK") else {
            println!("OCS_PID_REAL141_LEG_BLOCK unset, skipping.");
            return;
        };
        let Ok(leg_shape) = std::env::var("OCS_PID_REAL141_LEG_SHAPE") else {
            println!("OCS_PID_REAL141_LEG_SHAPE unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL141_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real141"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sha12 = |p: &std::path::Path| -> String {
            let b = std::fs::read(p).expect("read source bytes");
            let full = format!("{:x}", Sha256::digest(&b));
            full[..12].to_string()
        };

        // 1. Single-sheet in-app `PIDCONVERT` on median real SPPID DXF without prior `PIDPROMOTE`
        let sppid_med_p = std::path::PathBuf::from(&sppid_med);
        let sppid_med_sha = sha12(&sppid_med_p);
        let single_dir = out_dir.join("single_sppid");
        let single_pid = single_dir.join("INAPP-SINGLE-01.pid");

        let mut app_single = app();
        let loaded_med = crate::io::load_file(&sppid_med_p).expect("load median SPPID DXF");
        app_single.tabs[app_single.active_tab].scene.document = loaded_med;
        app_single.tabs[app_single.active_tab].current_path = Some(out_dir.join("VIRTUAL-MED.dxf"));
        app_single.tabs[app_single.active_tab].dirty = false;

        let single_cmd = if sym_dir.trim().is_empty() {
            format!(
                "PIDCONVERT \"{}\" PLANT P01 SEED \"{}\" VERIFY",
                single_pid.display(),
                seed_zip
            )
        } else {
            format!(
                "PIDCONVERT \"{}\" PLANT P01 SEED \"{}\" SYMDIR \"{}\" VERIFY",
                single_pid.display(),
                seed_zip,
                sym_dir
            )
        };
        let _ = app_single.run_command_line(&single_cmd);
        assert!(single_pid.is_file(), "single PIDCONVERT must write .pid");
        assert!(
            single_dir.join("INAPP-SINGLE-01.pidplot.pdf").is_file(),
            "single PIDCONVERT VERIFY must write .pidplot.pdf"
        );
        assert!(
            !app_single.tabs[app_single.active_tab].dirty,
            "PIDCONVERT must not dirty active canvas"
        );
        assert_eq!(
            crate::io::pid_convert::summarize_author_review(
                &app_single.tabs[app_single.active_tab].scene.document
            )
            .total_authored_symbols,
            0,
            "PIDCONVERT must not mutate active document XDATA"
        );

        let single_cov: Value = serde_json::from_str(
            &std::fs::read_to_string(single_dir.join("INAPP-SINGLE-01_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            single_cov["variant_resolution"]["class_default_fallback"].as_u64(),
            Some(0)
        );
        assert_eq!(
            single_cov["pid_verification"]["a_level_index_consistent"].as_bool(),
            Some(true)
        );
        assert_eq!(
            single_cov["pid_verification"]["matched_graphic_oids"].as_u64(),
            single_cov["pid_verification"]["xml_rep_count"].as_u64()
        );

        // 2. Batch in-app `PIDCONVERT BATCH` across 4 real prototypes (.dxf + binary AC1032 .dwg) + 0-byte empty skip
        let mixed_in = out_dir.join("mixed_in");
        let batch_out = out_dir.join("batch_out");
        std::fs::create_dir_all(&mixed_in).unwrap();

        let f1 = mixed_in.join("BATCH-01-SPPID.dxf");
        let f2 = mixed_in.join("BATCH-02-LEG-BLK.dxf");
        let f3 = mixed_in.join("BATCH-03-LEG-EXP.dxf");
        let f4 = mixed_in.join("BATCH-04-BIN-DWG.dwg");
        std::fs::copy(&sppid_small, &f1).unwrap();
        std::fs::copy(&leg_block, &f2).unwrap();
        std::fs::copy(&leg_shape, &f3).unwrap();
        crate::io::pid_convert::save_cad_file_as_binary_dwg(std::path::Path::new(&leg_block), &f4)
            .expect("save real legacy sheet as binary AC1032 .dwg");
        assert!(std::fs::read(&f4).unwrap().starts_with(b"AC10"));
        std::fs::write(mixed_in.join("EMPTY-00.dxf"), b"").unwrap();

        let batch_sum_p = out_dir.join("batch_summary.json");
        let mut app_batch = app();
        let batch_cmd = if sym_dir.trim().is_empty() {
            format!(
                "PIDCONVERT BATCH \"{}\" \"{}\" PLANT P01 SEED \"{}\" VERIFY SUMMARY \"{}\"",
                mixed_in.display(),
                batch_out.display(),
                seed_zip,
                batch_sum_p.display()
            )
        } else {
            format!(
                "PIDCONVERT BATCH \"{}\" \"{}\" PLANT P01 SEED \"{}\" SYMDIR \"{}\" VERIFY SUMMARY \"{}\"",
                mixed_in.display(),
                batch_out.display(),
                seed_zip,
                sym_dir,
                batch_sum_p.display()
            )
        };
        let _ = app_batch.run_command_line(&batch_cmd);
        assert!(
            batch_sum_p.is_file(),
            "PIDCONVERT BATCH must write summary JSON"
        );

        let bsum: Value =
            serde_json::from_str(&std::fs::read_to_string(&batch_sum_p).unwrap()).unwrap();
        assert_eq!(bsum["files_converted"].as_u64(), Some(4));
        assert_eq!(bsum["files_failed"].as_u64(), Some(0));
        assert_eq!(bsum["pid_files_written"].as_u64(), Some(4));
        assert_eq!(bsum["pdf_files_plotted"].as_u64(), Some(4));
        assert_eq!(
            bsum["variant_resolution"]["class_default_fallback"].as_u64(),
            Some(0)
        );

        let mut batch_rows = Vec::new();
        for (stem, src_p) in [
            ("BATCH-01-SPPID", &f1),
            ("BATCH-02-LEG-BLK", &f2),
            ("BATCH-03-LEG-EXP", &f3),
            ("BATCH-04-BIN-DWG", &f4),
        ] {
            let cov: Value = serde_json::from_str(
                &std::fs::read_to_string(batch_out.join(format!("{stem}_Coverage.json"))).unwrap(),
            )
            .unwrap();
            let exact_v = cov["variant_resolution"]["exact_variant_id"]
                .as_u64()
                .unwrap_or(0)
                + cov["variant_resolution"]["symbol_path_match"]
                    .as_u64()
                    .unwrap_or(0);
            let reps = cov["pid_verification"]["xml_rep_count"]
                .as_u64()
                .unwrap_or(0);
            let matched = cov["pid_verification"]["matched_graphic_oids"]
                .as_u64()
                .unwrap_or(0);
            assert_eq!(matched, reps, "{stem}: GraphicOID mismatch");
            batch_rows.push(json!({
                "stem": stem,
                "src_sha256_prefix": sha12(src_p),
                "exact_variants": exact_v,
                "fallback": cov["variant_resolution"]["class_default_fallback"],
                "items_written": cov["total_items_written"],
                "relations_written": cov["total_relations_written"],
                "xml_reps": reps,
                "matched_oids": matched,
                "a_valid": cov["pid_verification"]["a_level_index_consistent"],
                "pdf_prims": cov["pid_verification"]["b_level_pdf_primitives"],
            }));
        }

        let final_report = json!({
            "single_sppid": {
                "stem": "INAPP-SINGLE-01",
                "src_sha256_prefix": sppid_med_sha,
                "exact_variants": single_cov["variant_resolution"]["exact_variant_id"].as_u64().unwrap_or(0)
                    + single_cov["variant_resolution"]["symbol_path_match"].as_u64().unwrap_or(0),
                "fallback": single_cov["variant_resolution"]["class_default_fallback"],
                "items_written": single_cov["total_items_written"],
                "relations_written": single_cov["total_relations_written"],
                "xml_reps": single_cov["pid_verification"]["xml_rep_count"],
                "matched_oids": single_cov["pid_verification"]["matched_graphic_oids"],
                "a_valid": single_cov["pid_verification"]["a_level_index_consistent"],
                "pdf_prims": single_cov["pid_verification"]["b_level_pdf_primitives"],
            },
            "batch_mixed_4": {
                "files_converted": bsum["files_converted"],
                "files_failed": bsum["files_failed"],
                "total_items_written": bsum["total_items_written"],
                "total_relations_written": bsum["total_relations_written"],
                "sheets": batch_rows,
            }
        });
        std::fs::write(
            out_dir.join("real141_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&final_report).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l6_2_in_app_pid_import_promote_panel_edit_pidinsert_and_pidconvert_roundtrip() {
        use crate::io::PID_SEMANTICS_XDATA_APP;
        use crate::scene::view::dispatch::set_entity_xdata;
        use document::entities::{Line, LwPolyline, LwVertex, Text};
        use document::types::Vector3;
        use document::xdata::XDataValue;
        use document::EntityType;

        let root = std::env::temp_dir().join(format!("ocs_pidimport_142_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let seed_path = root.join("seed142.pid");
        std::fs::write(&seed_path, make_synthetic_seed_pid_bytes()).unwrap();

        let mut app = app();
        let (stroke_h1, vis_tag_h) = {
            let doc = &mut app.tabs[app.active_tab].scene.document;
            let sem = vec![
                XDataValue::String("role=symbol".to_string()),
                XDataValue::String("class=PIDPipingComponent".to_string()),
                XDataValue::String("label=BV-6101".to_string()),
                XDataValue::String("oid=801".to_string()),
                XDataValue::String("resolved=direct".to_string()),
            ];
            let mut h_first = None;
            for (a, b) in [
                ((47.0, 98.0), (47.0, 102.0)),
                ((47.0, 102.0), (53.0, 98.0)),
                ((53.0, 98.0), (53.0, 102.0)),
                ((53.0, 102.0), (47.0, 98.0)),
            ] {
                let mut ln =
                    Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
                ln.common.layer = "Default".to_string();
                let h = doc.add_entity(EntityType::Line(ln)).unwrap();
                set_entity_xdata(doc, h, PID_SEMANTICS_XDATA_APP, Some(sem.clone()));
                if h_first.is_none() {
                    h_first = Some(h);
                }
            }
            let mut slbl = Text::with_value("Ball Valve Type 2", Vector3::new(50.0, 96.0, 0.0))
                .with_height(1.8);
            slbl.common.layer = "PID-SYMBOL-LABEL".to_string();
            let slbl_h = doc.add_entity(EntityType::Text(slbl)).unwrap();
            set_entity_xdata(
                doc,
                slbl_h,
                PID_SEMANTICS_XDATA_APP,
                Some(vec![
                    XDataValue::String("role=symbol-label".to_string()),
                    XDataValue::String("class=PIDPipingComponent".to_string()),
                    XDataValue::String("label=BV-6101".to_string()),
                    XDataValue::String("oid=801".to_string()),
                ]),
            );
            let mut vtxt =
                Text::with_value("BV-6101", Vector3::new(48.0, 104.0, 0.0)).with_height(2.5);
            vtxt.common.layer = "Labels".to_string();
            let vtxt_h = doc.add_entity(EntityType::Text(vtxt)).unwrap();
            set_entity_xdata(
                doc,
                vtxt_h,
                PID_SEMANTICS_XDATA_APP,
                Some(vec![
                    XDataValue::String("role=text".to_string()),
                    XDataValue::String("class=PIDPipingComponent".to_string()),
                    XDataValue::String("label=BV-6101".to_string()),
                    XDataValue::String("oid=801".to_string()),
                    XDataValue::String("resolved=dependency:800".to_string()),
                ]),
            );
            let mut pl = LwPolyline::new();
            pl.common.layer = "Process".to_string();
            for (x, y) in [(10.0, 100.0), (47.0, 100.0), (95.0, 100.0)] {
                pl.vertices.push(LwVertex::from_coords(x, y));
            }
            let pl_h = doc.add_entity(EntityType::LwPolyline(pl)).unwrap();
            set_entity_xdata(
                doc,
                pl_h,
                PID_SEMANTICS_XDATA_APP,
                Some(vec![
                    XDataValue::String("role=geometry".to_string()),
                    XDataValue::String("class=PIDPipingConnector".to_string()),
                    XDataValue::String("label=50-PG-6101-B9-".to_string()),
                    XDataValue::String("oid=802".to_string()),
                ]),
            );
            (h_first.unwrap(), vtxt_h)
        };

        // 1. Bare `PIDPROMOTE` must detect native `.pid` semantics, merge by `oid`, and leave 0 duplicates
        let _ = app.run_command_line("PIDPROMOTE");
        let rev0 = crate::io::pid_convert::summarize_author_review(
            &app.tabs[app.active_tab].scene.document,
        );
        assert_eq!(rev0.total_authored_symbols, 1);
        assert_eq!(rev0.duplicated_tags, 0);

        // 2. Edit tag via property panel -> paired visible Text updates
        select(&mut app, stroke_h1);
        set(&mut app, "pid_tag", "BV-PANEL-142");
        assert_eq!(dwg_text_val(&app, vis_tag_h), "BV-PANEL-142");

        // 3. Place a brand-new vessel via `PIDINSERT vessel`
        let vessel_h = placed_vessel(&mut app, 130.0);
        let vessel_tag = author(&app, vessel_h).tag;
        assert!(!vessel_tag.is_empty());

        // 4. `PIDCONVERT` (XML-only, no SEED) must export edited `PID_AUTHOR` + newly placed vessel with 0 fallback
        let xml_dir = root.join("xml_authored");
        let cmd_xml = format!("PIDCONVERT \"{}\" STEM AUTH-142 VERIFY", xml_dir.display());
        let _ = app.run_command_line(&cmd_xml);
        let auth_xml = xml_dir.join("AUTH-142_Data.xml");
        assert!(auth_xml.is_file(), "PIDCONVERT must export authored XML");
        let xml_str = std::fs::read_to_string(&auth_xml).unwrap();
        assert!(
            xml_str.contains("BV-PANEL-142"),
            "PIDCONVERT must preserve panel-edited PID_AUTHOR tag"
        );
        assert!(
            xml_str.contains(&vessel_tag),
            "PIDCONVERT must include newly placed PIDINSERT symbol"
        );
        let cov_val: Value = serde_json::from_str(
            &std::fs::read_to_string(xml_dir.join("AUTH-142_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            cov_val["variant_resolution"]["class_default_fallback"].as_u64(),
            Some(0)
        );

        // 5. `PIDSAVEPID` dual save-back succeeds and passes a/b verification
        let out_pid = root.join("dual").join("AUTH-DUAL-142.pid");
        let cmd_save = format!(
            "PIDSAVEPID \"{}\" SEED \"{}\"",
            out_pid.display(),
            seed_path.display()
        );
        let _ = app.run_command_line(&cmd_save);
        assert!(out_pid.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real .pid production/reference sheets in OpenCADStudio -> PIDPROMOTE -> panel tag edit + PIDINSERT -> PIDCONVERT XML & PIDSAVEPID dual roundtrip"]
    fn real_pid_files_in_app_promote_edit_pidinsert_and_dual_save_roundtrip() {
        use sha2::{Digest, Sha256};

        let Ok(raw_list) = std::env::var("OCS_PID_REAL142_PIDS") else {
            println!("OCS_PID_REAL142_PIDS unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL142_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real142"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let pid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&raw_list)
            .filter(|p| p.is_file())
            .collect();
        assert!(!pid_paths.is_empty(), "expected >= 1 real .pid path");

        let mut rows = Vec::new();
        let mut tot_promoted = 0usize;
        let mut tot_exact = 0usize;
        let mut tot_fallbacks = 0usize;
        let mut tot_items = 0usize;
        let mut tot_rels = 0usize;
        let mut tot_reps = 0usize;
        let mut tot_matched = 0usize;
        let mut tot_pdf_prims = 0usize;

        for (idx, p) in pid_paths.iter().enumerate() {
            let pid_id = format!("P{:02}", idx + 1);
            let raw_bytes = std::fs::read(p).expect("read real .pid bytes");
            let sha_full = format!("{:x}", Sha256::digest(&raw_bytes));
            let sha_pfx = sha_full[..12].to_string();

            let mut app = app();
            let loaded = crate::io::load_file(p).expect("load real .pid in app");
            let model_ents = loaded.model_space_entities().count();
            app.tabs[app.active_tab].scene.document = loaded;

            // 1. Default `PIDPROMOTE` auto-detects native `.pid` semantics and merges multi-stroke placements by `oid`
            let _ = app.run_command_line("PIDPROMOTE");
            let rev0 = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert!(
                rev0.total_authored_symbols > 0,
                "{pid_id}: expected promoted symbols > 0"
            );
            assert!(
                rev0.total_authored_symbols < model_ents,
                "{pid_id}: multi-stroke placements must merge ({} < {})",
                rev0.total_authored_symbols,
                model_ents
            );

            if rev0.duplicated_tags > 0 {
                let _ = app.run_command_line("PIDREVIEW DEDUP");
            }
            let rev1 = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert_eq!(rev1.duplicated_tags, 0, "{pid_id}: duplicates must be 0");

            // 2. Select a promoted symbol (prefer one carrying paired `_tag_handles`) and edit its tag via property panel
            let target_sym = app.tabs[app.active_tab]
                .scene
                .document
                .model_space_entities()
                .find_map(|e| {
                    let h = e.common().handle;
                    let a = PidAuthor::load(&app.tabs[app.active_tab].scene.document, h)?;
                    if a.topology == crate::pid::topology::ROLE_PIPE_SEGMENT
                        || a.type_code == crate::pid::topology::TYPE_PIPE_SEGMENT
                        || a.tag.is_empty()
                    {
                        return None;
                    }
                    let m: serde_json::Map<String, Value> = serde_json::from_str(&a.props).ok()?;
                    let th = m
                        .get("_tag_handles")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    (!th.is_empty()).then_some((h, th))
                })
                .or_else(|| {
                    crate::io::pid_convert::filter_author_handles(
                        &app.tabs[app.active_tab].scene.document,
                        "ALL",
                    )
                    .into_iter()
                    .next()
                    .map(|h| (h, String::new()))
                })
                .expect("at least one promoted symbol");

            select(&mut app, target_sym.0);
            let ev_rows = section_rows(&mut app, PID_EVIDENCE_SECTION)
                .expect("promoted .pid symbol must expose P&ID 来源证据 section");
            assert_eq!(
                row_value(&ev_rows, "pid_ev_source").as_deref(),
                Some("import:pid")
            );

            let edited_tag = format!("PID142-ED-{}", idx + 1);
            set(&mut app, "pid_tag", &edited_tag);
            if let Some(first_hex) = target_sym.1.split(',').find(|s| !s.trim().is_empty()) {
                let th = document::Handle::new(u64::from_str_radix(first_hex, 16).unwrap());
                assert!(dwg_text_val(&app, th).contains("PID142"));
            }

            // 3. Place a brand-new vessel via `PIDINSERT vessel`
            let new_vessel_h = placed_vessel(&mut app, 220.0);
            let new_vessel_tag = author(&app, new_vessel_h).tag;
            assert!(!new_vessel_tag.is_empty());

            let sheet_dir = out_dir.join(&pid_id);
            std::fs::create_dir_all(&sheet_dir).unwrap();

            // 4. `PIDCONVERT` XML-only export (no SEED) preserves both edited tag and newly placed vessel
            let xml_only_dir = sheet_dir.join("xml_only");
            let xml_cmd = format!(
                "PIDCONVERT \"{}\" STEM {pid_id}-XML VERIFY",
                xml_only_dir.display()
            );
            let _ = app.run_command_line(&xml_cmd);
            let xml_only_data =
                std::fs::read_to_string(xml_only_dir.join(format!("{pid_id}-XML_Data.xml")))
                    .expect("read authored XML");
            assert!(xml_only_data.contains(&edited_tag));
            assert!(xml_only_data.contains(&new_vessel_tag));

            // 5. `PIDSAVEPID` dual `.pid` + XML save-back + `StrictVendor` + `a/b` verification
            let dual_pid = sheet_dir.join(format!("{pid_id}.pid"));
            let save_cmd = if sym_dir.trim().is_empty() {
                format!(
                    "PIDSAVEPID \"{}\" SEED \"{}\"",
                    dual_pid.display(),
                    seed_zip
                )
            } else {
                format!(
                    "PIDSAVEPID \"{}\" SEED \"{}\" SYMDIR \"{}\"",
                    dual_pid.display(),
                    seed_zip,
                    sym_dir
                )
            };
            let _ = app.run_command_line(&save_cmd);
            assert!(dual_pid.is_file(), "{pid_id}: saved .pid must exist");

            let data_raw =
                std::fs::read(sheet_dir.join(format!("{pid_id}_Data.xml"))).expect("read _Data");
            let meta_raw =
                std::fs::read(sheet_dir.join(format!("{pid_id}_Meta.xml"))).expect("read _Meta");
            let pid_raw = std::fs::read(&dual_pid).expect("read saved .pid");
            crate::io::pid_convert::verify_published_xml_pair(&pid_id, "P01", &data_raw, &meta_raw)
                .expect("StrictVendor zero violations");

            let cov_val: Value = serde_json::from_str(
                &std::fs::read_to_string(sheet_dir.join(format!("{pid_id}_Coverage.json")))
                    .unwrap(),
            )
            .unwrap();
            let doc_uid = cov_val["doc_uid"].as_str().unwrap().to_string();
            let exact_v = cov_val["variant_resolution"]["exact_variant_id"]
                .as_u64()
                .unwrap_or(0) as usize
                + cov_val["variant_resolution"]["symbol_path_match"]
                    .as_u64()
                    .unwrap_or(0) as usize;
            let fb_v = cov_val["variant_resolution"]["class_default_fallback"]
                .as_u64()
                .unwrap_or(0) as usize;
            let items_w = cov_val["total_items_written"].as_u64().unwrap_or(0) as usize;
            let rels_w = cov_val["total_relations_written"].as_u64().unwrap_or(0) as usize;
            assert_eq!(fb_v, 0, "{pid_id}: fallback must be 0");

            let dual_ver = crate::io::pid_convert::verify_dual_bundle(
                &doc_uid,
                &data_raw,
                &pid_raw,
                Some(&sheet_dir),
                &pid_id,
            )
            .expect("verify_dual_bundle");
            assert!(
                dual_ver.is_cross_file_and_a_valid(),
                "{pid_id}: {dual_ver:?}"
            );
            assert!(dual_ver.is_b_plot_valid(), "{pid_id}: {dual_ver:?}");

            tot_promoted += rev1.total_authored_symbols + 1;
            tot_exact += exact_v;
            tot_fallbacks += fb_v;
            tot_items += items_w;
            tot_rels += rels_w;
            tot_reps += dual_ver.xml_rep_count;
            tot_matched += dual_ver.matched_graphic_oids;
            tot_pdf_prims += dual_ver.b_level_pdf_primitives.unwrap_or(0);

            rows.push(json!({
                "id": pid_id,
                "src_pid_sha256_prefix": sha_pfx,
                "raw_model_entities": model_ents,
                "promoted_symbols_merged": rev1.total_authored_symbols,
                "duplicates_before_dedup": rev0.duplicated_tags,
                "edited_tag": edited_tag,
                "inserted_vessel_tag": new_vessel_tag,
                "exact_variants_saved": exact_v,
                "fallback": fb_v,
                "items_written": items_w,
                "relations_written": rels_w,
                "xml_reps": dual_ver.xml_rep_count,
                "matched_oids": dual_ver.matched_graphic_oids,
                "a_valid": dual_ver.a_level_index_consistent,
                "pdf_prims": dual_ver.b_level_pdf_primitives,
            }));
        }

        let summary = json!({
            "pid_sheets_tested": rows.len(),
            "total_authored_plus_inserted": tot_promoted,
            "total_exact_variants_saved": tot_exact,
            "total_fallbacks": tot_fallbacks,
            "total_items_written": tot_items,
            "total_relations_written": tot_rels,
            "total_xml_reps": tot_reps,
            "total_matched_oids": tot_matched,
            "total_pdf_primitives": tot_pdf_prims,
            "sheets": rows,
        });
        std::fs::write(
            out_dir.join("real142_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&summary).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn l3_6_pidconvert_and_pidsavepid_apply_external_schedule_props_with_strict_vendor_check() {
        let root = std::env::temp_dir().join(format!("ocs_pidprops_145_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let csv_path = root.join("schedule.csv");
        std::fs::write(
            &csv_path,
            "位号,名称,设计压力,设计温度,操作压力,操作温度,管道等级\n\
             BV0201501A,Inlet Block Valve,,,,,\n\
             50-PG-02011-B9-,,3.5 MPa,130 C,2.2 MPa,75 C,B9-EXT\n",
        )
        .unwrap();

        let mut app = app();
        let _ = seed_dxf_sheet_in_app(&mut app);

        let out_dir = root.join("conv_props");
        let cmd = format!(
            "PIDCONVERT \"{}\" STEM EXT-CSV-145 PROPS \"{}\" VERIFY",
            out_dir.display(),
            csv_path.display()
        );
        let _ = app.run_command_line(&cmd);

        let data_xml = std::fs::read_to_string(out_dir.join("EXT-CSV-145_Data.xml"))
            .expect("EXT-CSV-145_Data.xml");
        assert!(data_xml.contains("Inlet Block Valve"));
        assert!(data_xml.contains(r#"PressureMax="3.5 MPa""#));
        assert!(data_xml.contains(r#"PhaseTemperatureMax="130 C""#));
        assert!(data_xml.contains(r#"PressureBase="2.2 MPa""#));
        assert!(data_xml.contains(r#"PhaseTemperatureBase="75 C""#));
        assert!(data_xml.contains(r#"PipingMaterialsClass="B9-EXT""#));

        let cov: Value = serde_json::from_str(
            &std::fs::read_to_string(out_dir.join("EXT-CSV-145_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert!(
            cov["external_props_applied"]["attributes_injected"]
                .as_u64()
                .unwrap_or(0)
                >= 5
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real production DXF in OpenCADStudio and exports dual bundle with multisheet .xlsx Equipment/Line schedules"]
    fn real_sheet_in_app_pidconvert_with_external_xlsx_schedule_roundtrip() {
        use sha2::{Digest, Sha256};

        let Ok(dxf_p) = std::env::var("OCS_PID_REAL145_DXF") else {
            println!("OCS_PID_REAL145_DXF unset, skipping.");
            return;
        };
        let Ok(xlsx_p) = std::env::var("OCS_PID_REAL145_XLSX") else {
            println!("OCS_PID_REAL145_XLSX unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL145_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real145"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let dxf_bytes = std::fs::read(&dxf_p).expect("read dxf");
        let sha_pfx = format!("{:x}", Sha256::digest(&dxf_bytes))[..12].to_string();

        let mut app = app();
        app.tabs[app.active_tab].scene.document =
            crate::io::load_file(std::path::Path::new(&dxf_p)).expect("load real DXF");

        // 1. Baseline without PROPS
        let base_dir = out_dir.join("without_props");
        let cmd_base = format!(
            "PIDCONVERT \"{}\" STEM REAL145-BASE VERIFY",
            base_dir.display()
        );
        let _ = app.run_command_line(&cmd_base);
        let base_xml =
            std::fs::read_to_string(base_dir.join("REAL145-BASE_Data.xml")).expect("base xml");
        let base_nonempty_pmax = base_xml.matches(r#"PressureMax=""#).count()
            - base_xml.matches(r#"PressureMax="""#).count();

        // 2. Dual export with multisheet `.xlsx` PROPS + VERIFY
        let with_dir = out_dir.join("with_xlsx_props");
        let target_pid = with_dir.join("REAL145-PROPS.pid");
        let cmd_with = if sym_dir.trim().is_empty() {
            format!(
                "PIDCONVERT \"{}\" SEED \"{}\" PROPS \"{}\" VERIFY",
                target_pid.display(),
                seed_zip,
                xlsx_p
            )
        } else {
            format!(
                "PIDCONVERT \"{}\" SEED \"{}\" SYMDIR \"{}\" PROPS \"{}\" VERIFY",
                target_pid.display(),
                seed_zip,
                sym_dir,
                xlsx_p
            )
        };
        let _ = app.run_command_line(&cmd_with);
        assert!(target_pid.is_file(), "REAL145-PROPS.pid must exist");
        assert!(with_dir.join("REAL145-PROPS.pidplot.pdf").is_file());

        let with_xml =
            std::fs::read_to_string(with_dir.join("REAL145-PROPS_Data.xml")).expect("with xml");
        let with_nonempty_pmax = with_xml.matches(r#"PressureMax=""#).count()
            - with_xml.matches(r#"PressureMax="""#).count();
        assert!(
            with_nonempty_pmax > base_nonempty_pmax,
            "XLSX LineSchedule must populate non-empty PressureMax on .PPT ({with_nonempty_pmax} vs {base_nonempty_pmax})"
        );

        let cov: Value = serde_json::from_str(
            &std::fs::read_to_string(with_dir.join("REAL145-PROPS_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            cov["variant_resolution"]["class_default_fallback"].as_u64(),
            Some(0)
        );
        assert_eq!(
            cov["pid_verification"]["matched_graphic_oids"].as_u64(),
            cov["pid_verification"]["xml_rep_count"].as_u64()
        );

        let summary = json!({
            "src_dxf_sha256_prefix": sha_pfx,
            "nonempty_pressure_max_before": base_nonempty_pmax,
            "nonempty_pressure_max_after": with_nonempty_pmax,
            "external_props_applied": cov["external_props_applied"],
            "items_written": cov["total_items_written"],
            "relations_written": cov["total_relations_written"],
            "xml_reps": cov["pid_verification"]["xml_rep_count"],
            "matched_oids": cov["pid_verification"]["matched_graphic_oids"],
            "a_valid": cov["pid_verification"]["a_level_index_consistent"],
            "pdf_prims": cov["pid_verification"]["b_level_pdf_primitives"],
        });
        std::fs::write(
            out_dir.join("real145_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&summary).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs all VENDOR_REF reference .pid files plus XML-free bare BACKUP_ORACLE .pid files inside OpenCADStudio via PIDPROMOTE -> PIDREVIEW DEDUP -> PIDSAVEPID"]
    fn real_vendor_ref_all_and_bare_backup_pids_in_app_roundtrip() {
        use sha2::{Digest, Sha256};

        let Ok(vendor_raw) = std::env::var("OCS_PID_REAL146_VENDOR_PIDS") else {
            println!("OCS_PID_REAL146_VENDOR_PIDS unset, skipping.");
            return;
        };
        let Ok(bare_raw) = std::env::var("OCS_PID_REAL146_BARE_PIDS") else {
            println!("OCS_PID_REAL146_BARE_PIDS unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL146_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real146"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let vendor_paths: Vec<std::path::PathBuf> = std::env::split_paths(&vendor_raw)
            .filter(|p| p.is_file())
            .collect();
        let bare_paths: Vec<std::path::PathBuf> = std::env::split_paths(&bare_raw)
            .filter(|p| p.is_file())
            .collect();
        assert!(!vendor_paths.is_empty());
        assert!(!bare_paths.is_empty());

        let mut all_cases: Vec<(String, String, std::path::PathBuf)> = Vec::new();
        for (i, p) in vendor_paths.into_iter().enumerate() {
            all_cases.push((
                format!("V{:02}", i + 1),
                "vendor_ref_with_xml".to_string(),
                p,
            ));
        }
        for (i, p) in bare_paths.into_iter().enumerate() {
            // Confirm companion _Data.xml does NOT exist next to bare backup `.pid`
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            assert!(
                !p.with_file_name(format!("{stem}_Data.xml")).exists(),
                "expected XML-free bare .pid"
            );
            all_cases.push((format!("B{:02}", i + 1), "bare_pid_no_xml".to_string(), p));
        }

        let mut rows = Vec::new();
        let mut tot_syms = 0usize;
        let mut tot_exact = 0usize;
        let mut tot_fb = 0usize;
        let mut tot_items = 0usize;
        let mut tot_rels = 0usize;
        let mut tot_reps = 0usize;
        let mut tot_matched = 0usize;

        for (cid, kind, p) in &all_cases {
            let raw_b = std::fs::read(p).expect("read .pid");
            let sha_pfx = format!("{:x}", Sha256::digest(&raw_b))[..12].to_string();

            let mut app = app();
            let loaded = crate::io::load_file(p).expect("load .pid in app");
            let raw_ents = loaded.model_space_entities().count();
            app.tabs[app.active_tab].scene.document = loaded;

            let _ = app.run_command_line("PIDPROMOTE");
            let rev0 = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert!(
                rev0.total_authored_symbols > 0 && rev0.total_authored_symbols < raw_ents,
                "{cid} ({kind}): expected merged symbols in 1..{raw_ents}, got {}",
                rev0.total_authored_symbols
            );

            if rev0.duplicated_tags > 0 {
                let _ = app.run_command_line("PIDREVIEW DEDUP");
            }
            let rev1 = crate::io::pid_convert::summarize_author_review(
                &app.tabs[app.active_tab].scene.document,
            );
            assert_eq!(rev1.duplicated_tags, 0);

            let sdir = out_dir.join(cid);
            std::fs::create_dir_all(&sdir).unwrap();
            let out_pid = sdir.join(format!("{cid}.pid"));
            let save_cmd = if sym_dir.trim().is_empty() {
                format!("PIDSAVEPID \"{}\" SEED \"{}\"", out_pid.display(), seed_zip)
            } else {
                format!(
                    "PIDSAVEPID \"{}\" SEED \"{}\" SYMDIR \"{}\"",
                    out_pid.display(),
                    seed_zip,
                    sym_dir
                )
            };
            let _ = app.run_command_line(&save_cmd);
            assert!(out_pid.is_file(), "{cid}: saved .pid must exist");

            let data_raw = std::fs::read(sdir.join(format!("{cid}_Data.xml"))).unwrap();
            let meta_raw = std::fs::read(sdir.join(format!("{cid}_Meta.xml"))).unwrap();
            let pid_raw = std::fs::read(&out_pid).unwrap();
            crate::io::pid_convert::verify_published_xml_pair(cid, "P01", &data_raw, &meta_raw)
                .expect("StrictVendor clean");

            let cov: Value = serde_json::from_str(
                &std::fs::read_to_string(sdir.join(format!("{cid}_Coverage.json"))).unwrap(),
            )
            .unwrap();
            let doc_uid = cov["doc_uid"].as_str().unwrap().to_string();
            let exact_v = cov["variant_resolution"]["exact_variant_id"]
                .as_u64()
                .unwrap_or(0) as usize
                + cov["variant_resolution"]["symbol_path_match"]
                    .as_u64()
                    .unwrap_or(0) as usize;
            let fb_v = cov["variant_resolution"]["class_default_fallback"]
                .as_u64()
                .unwrap_or(0) as usize;
            assert_eq!(fb_v, 0, "{cid}: fallback must be 0");

            let ver = crate::io::pid_convert::verify_dual_bundle(
                &doc_uid,
                &data_raw,
                &pid_raw,
                Some(&sdir),
                cid,
            )
            .expect("verify_dual_bundle");
            assert!(ver.is_cross_file_and_a_valid(), "{cid}: {ver:?}");
            assert!(ver.is_b_plot_valid(), "{cid}: {ver:?}");

            let iw = cov["total_items_written"].as_u64().unwrap_or(0) as usize;
            let rw = cov["total_relations_written"].as_u64().unwrap_or(0) as usize;
            tot_syms += rev1.total_authored_symbols;
            tot_exact += exact_v;
            tot_fb += fb_v;
            tot_items += iw;
            tot_rels += rw;
            tot_reps += ver.xml_rep_count;
            tot_matched += ver.matched_graphic_oids;

            rows.push(json!({
                "id": cid,
                "kind": kind,
                "src_pid_sha256_prefix": sha_pfx,
                "raw_model_entities": raw_ents,
                "promoted_symbols": rev1.total_authored_symbols,
                "exact_variants": exact_v,
                "fallback": fb_v,
                "items_written": iw,
                "relations_written": rw,
                "xml_reps": ver.xml_rep_count,
                "matched_oids": ver.matched_graphic_oids,
                "a_valid": ver.a_level_index_consistent,
                "pdf_prims": ver.b_level_pdf_primitives,
            }));
        }

        let summary = json!({
            "sheets_tested": rows.len(),
            "total_promoted_symbols": tot_syms,
            "total_exact_variants": tot_exact,
            "total_fallbacks": tot_fb,
            "total_items_written": tot_items,
            "total_relations_written": tot_rels,
            "total_xml_reps": tot_reps,
            "total_matched_oids": tot_matched,
            "sheets": rows,
        });
        std::fs::write(
            out_dir.join("real146_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&summary).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidopc_pairs_multi_tab_connectors_selects_broken_and_jumps_camera_to_peer() {
        use document::entities::{EntityType, Insert, Line, Text};
        use document::types::Vector3;
        use document::Document;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        fn build_opc_sheet(
            dwg_no: &str,
            connectors: &[(f64, f64, &str, Option<&str>)],
        ) -> (Document, Vec<document::Handle>) {
            let mut doc = Document::new();
            define_sppid_block(&mut doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut border = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            border.common.layer = "Default".to_string();
            doc.add_entity(EntityType::Line(border)).unwrap();

            let mut tb = Text::with_value(dwg_no, Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(EntityType::Text(tb)).unwrap();

            let mut handles = Vec::new();
            for &(x, y, target_dwg, line_opt) in connectors {
                let mut ins = Insert::new("_OFF-UNIT SQ", Vector3::new(x, y, 0.0));
                ins.common.layer = "11".to_string();
                let h = doc.add_entity(EntityType::Insert(ins)).unwrap();
                handles.push(h);

                let mut tf = Text::with_value(target_dwg, Vector3::new(x - 10.0, y - 1.0, 0.0))
                    .with_height(2.5);
                tf.common.layer = "11".to_string();
                doc.add_entity(EntityType::Text(tf)).unwrap();

                if let Some(ln_lbl) = line_opt {
                    let mut pipe = Line::from_points(
                        Vector3::new(x + 6.0, y, 0.0),
                        Vector3::new(x + 80.0, y, 0.0),
                    );
                    pipe.common.layer = "1".to_string();
                    doc.add_entity(EntityType::Line(pipe)).unwrap();

                    let mut ltxt = Text::with_value(ln_lbl, Vector3::new(x + 30.0, y + 2.0, 0.0))
                        .with_height(2.5);
                    ltxt.common.layer = "11".to_string();
                    doc.add_entity(EntityType::Text(ltxt)).unwrap();
                }
            }
            (doc, handles)
        }

        let (doc_a, handles_a) = build_opc_sheet(
            "DWG-2101-01",
            &[
                // Paired with Sheet B
                (120.0, 300.0, "DWG-2101-02", Some("100-PW-2101-CS150")),
                // One-way broken (Sheet B is open, but has no return connector for this branch)
                (120.0, 150.0, "DWG-2101-02", None),
                // External unopened sheet
                (120.0, 50.0, "DWG-2101-99", None),
            ],
        );
        let (doc_b, handles_b) = build_opc_sheet(
            "DWG-2101-02",
            &[(860.0, 640.0, "DWG-2101-01", Some("100-PW-2101-CS150"))],
        );

        let mut app = app();
        let tab_a = app.active_tab;
        app.tabs[tab_a].scene.document = doc_a;
        app.tabs[tab_a].tab_title = "DWG-2101-01.dxf".to_string();

        let _ = app.update(Message::TabNew);
        let tab_b = app.active_tab;
        assert_ne!(tab_a, tab_b);
        app.tabs[tab_b].scene.document = doc_b;
        app.tabs[tab_b].tab_title = "DWG-2101-02.dxf".to_string();

        // Switch back to Tab A
        let _ = app.update(Message::TabSwitch(tab_a));
        assert_eq!(app.active_tab, tab_a);

        // 1. Export cross-tab OPC audit JSON BEFORE PIDPROMOTE (read-only raw CAD mode)
        let tmp = std::env::temp_dir().join(format!("ocs_pidopc_149_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let exp_json = tmp.join("opc_report.json");
        let _ = app.run_command_line(&format!("PIDOPC EXPORT \"{}\"", exp_json.display()));
        assert!(exp_json.is_file(), "PIDOPC EXPORT must write report JSON");
        let rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&exp_json).unwrap()).unwrap();
        assert_eq!(rep["open_sheets"].as_u64(), Some(2));
        assert_eq!(rep["total_connectors"].as_u64(), Some(4));
        assert_eq!(rep["reciprocal_pairs"].as_u64(), Some(1));
        assert_eq!(rep["one_way_open"].as_u64(), Some(1));
        assert_eq!(rep["external_unopened"].as_u64(), Some(1));

        // 2. `PIDOPC UNPAIRED` selects broken one-way connector `handles_a[1]` on active sheet, NOT paired `handles_a[0]`
        let _ = app.run_command_line("PIDOPC UNPAIRED");
        assert!(has_sel(&app, tab_a, handles_a[1]));
        assert!(!has_sel(&app, tab_a, handles_a[0]));

        // 3. `PIDOPC PAIRED` selects `handles_a[0]`, then `PIDOPC JUMP` switches to Tab B, selects `handles_b[0]`, and centers camera
        let _ = app.run_command_line("PIDOPC PAIRED");
        assert!(has_sel(&app, tab_a, handles_a[0]));

        let _ = app.run_command_line("PIDOPC JUMP");
        assert_eq!(
            app.active_tab, tab_b,
            "PIDOPC JUMP must switch active tab to peer sheet"
        );
        assert!(
            has_sel(&app, tab_b, handles_b[0]),
            "PIDOPC JUMP must select peer connector on target tab"
        );
        let cam_b = app.tabs[tab_b].scene.camera.borrow().target;
        assert!(
            (cam_b.x - 860.0).abs() < 25.0 && (cam_b.y - 640.0).abs() < 25.0,
            "camera target must center on peer connector at (860, 640), got {cam_b:?}"
        );

        // 4. Promote both tabs to `PID_AUTHOR` and jump back from Tab B -> Tab A
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab_a));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab_b));
        app.tabs[tab_b].scene.deselect_all();
        app.tabs[tab_b].scene.select_entities(&[handles_b[0]]);
        let _ = app.run_command_line("PIDOPC JUMP");
        assert_eq!(
            app.active_tab, tab_a,
            "reverse PIDOPC JUMP after PIDPROMOTE must return to Tab A"
        );
        let cam_a = app.tabs[tab_a].scene.camera.borrow().target;
        assert!(
            (cam_a.x - 120.0).abs() < 25.0 && (cam_a.y - 300.0).abs() < 25.0,
            "camera target must center on Tab A connector at (120, 300), got {cam_a:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into multi-tab OpenCADStudio and verifies PIDOPC SUMMARY / PAIRED / JUMP"]
    fn real_multi_tab_pidopc_cross_sheet_pairing_and_jump() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL149_SPPID_PAIR") else {
            println!("OCS_PID_REAL149_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL149_LEGACY_PAIR") else {
            println!("OCS_PID_REAL149_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL149_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real149"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2, "expected 2 paired SPPID DXF sheets");
        assert_eq!(legacy_paths.len(), 2, "expected 2 paired legacy DXF sheets");

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();

        for (idx, (_, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].current_path = Some((*p).clone());
            app.tabs[ti].tab_title = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("sheet.dxf")
                .to_string();
            tab_indices.push(ti);
        }

        let raw_exp = out_dir.join("raw_opc_audit.json");
        let _ = app.run_command_line(&format!("PIDOPC EXPORT \"{}\"", raw_exp.display()));
        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&raw_exp).unwrap()).unwrap();
        let _ = std::fs::remove_file(&raw_exp);

        let open_sheets = rep["open_sheets"].as_u64().unwrap_or(0);
        let total_connectors = rep["total_connectors"].as_u64().unwrap_or(0);
        let reciprocal_pairs = rep["reciprocal_pairs"].as_u64().unwrap_or(0);
        let one_way_open = rep["one_way_open"].as_u64().unwrap_or(0);
        let external_unopened = rep["external_unopened"].as_u64().unwrap_or(0);

        assert_eq!(open_sheets, 4);
        assert!(
            reciprocal_pairs >= 2,
            "expected >=2 reciprocal pairs across SPPID + legacy pairs, got {reciprocal_pairs}"
        );

        // Jump 1: SP01 -> SP02 -> PIDPROMOTE -> reverse jump SP02 -> SP01
        let _ = app.update(Message::TabSwitch(tab_indices[0]));
        let _ = app.run_command_line("PIDOPC PAIRED");
        assert!(!app.tabs[tab_indices[0]]
            .scene
            .selected_entities()
            .is_empty());
        let _ = app.run_command_line("PIDOPC JUMP");
        assert_eq!(app.active_tab, tab_indices[1]);
        assert!(!app.tabs[tab_indices[1]]
            .scene
            .selected_entities()
            .is_empty());

        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab_indices[0]));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab_indices[1]));
        let _ = app.run_command_line("PIDOPC PAIRED");
        let _ = app.run_command_line("PIDOPC JUMP");
        assert_eq!(app.active_tab, tab_indices[0]);

        // Jump 2: Native AutoCAD legacy LG01 -> LG02
        let _ = app.update(Message::TabSwitch(tab_indices[2]));
        let _ = app.run_command_line("PIDOPC PAIRED");
        assert!(!app.tabs[tab_indices[2]]
            .scene
            .selected_entities()
            .is_empty());
        let _ = app.run_command_line("PIDOPC JUMP");
        assert_eq!(app.active_tab, tab_indices[3]);
        assert!(!app.tabs[tab_indices[3]]
            .scene
            .selected_entities()
            .is_empty());

        let sanitized = json!({
            "open_sheets": open_sheets,
            "sheets": [
                {"id": "SP01", "kind": "sppid_dxf", "sha256_prefix": sha_prefixes[0]},
                {"id": "SP02", "kind": "sppid_dxf", "sha256_prefix": sha_prefixes[1]},
                {"id": "LG01", "kind": "legacy_autocad_dxf", "sha256_prefix": sha_prefixes[2]},
                {"id": "LG02", "kind": "legacy_autocad_dxf", "sha256_prefix": sha_prefixes[3]},
            ],
            "total_connectors": total_connectors,
            "reciprocal_pairs": reciprocal_pairs,
            "one_way_open": one_way_open,
            "external_unopened": external_unopened,
            "sppid_roundtrip_jump_verified": true,
            "legacy_jump_verified": true,
        });
        std::fs::write(
            out_dir.join("real149_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidbom_exports_isomorphic_schedule_and_roundtrips_through_pidconvert_props() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        let mut app = app();
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-150-01.dxf".to_string();
        let _ = seed_dxf_sheet_in_app(&mut app);

        // Open a second tab sharing pipeline `50-PG-02011-B9-` plus a new valve `BV0201509B`
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-150-02.dxf".to_string();
        {
            let doc2 = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc2, "BALL VALVE TYPE 2", 6.0, 4.0);
            let mut v2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(80.0, 120.0, 0.0));
            v2.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Insert(v2)).unwrap();
            let mut vt2 =
                Text::with_value("BV0201509B", Vector3::new(76.0, 126.0, 0.0)).with_height(2.5);
            vt2.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(vt2)).unwrap();

            let mut seg2 = Line::from_points(
                Vector3::new(20.0, 120.0, 0.0),
                Vector3::new(140.0, 120.0, 0.0),
            );
            seg2.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Line(seg2)).unwrap();
            let mut ll2 = Text::with_value("50-PG-02011-B9-", Vector3::new(40.0, 123.0, 0.0))
                .with_height(2.5);
            ll2.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(ll2)).unwrap();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidbom_150_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let bom_csv = tmp.join("bom_all.csv");
        let bom_json = tmp.join("bom_all.json");
        let _ = app.run_command_line(&format!("PIDBOM \"{}\" ALLTABS", bom_csv.display()));
        let _ = app.run_command_line(&format!("PIDBOM \"{}\" ALLTABS", bom_json.display()));
        assert!(bom_csv.is_file(), "PIDBOM must write CSV schedule");
        assert!(bom_json.is_file(), "PIDBOM must write JSON schedule");

        let jb: Value = serde_json::from_str(&std::fs::read_to_string(&bom_json).unwrap()).unwrap();
        assert!(jb["summary"]["total_unique_rows"].as_u64().unwrap_or(0) >= 4);
        assert_eq!(jb["summary"]["sheets_scanned"].as_u64(), Some(2));

        // Unmodified exported CSV parses cleanly via `load_external_property_table`
        let parsed_tbl =
            crate::io::pid_convert::load_external_property_table(&bom_csv).expect("load BOM CSV");
        assert!(parsed_tbl.total_rows_loaded >= 4);

        // Simulate process engineer filling in Description + PressureMax + PhaseTemperatureMax in CSV
        let csv_raw = std::fs::read_to_string(&bom_csv).unwrap();
        let mut edited_lines = Vec::new();
        for (li, line) in csv_raw.lines().enumerate() {
            if li == 0 {
                edited_lines.push(line.to_string());
                continue;
            }
            let mut cols: Vec<String> = line.split(',').map(str::to_string).collect();
            if cols.first().is_some_and(|k| k.contains("BV0201501A")) && cols.len() >= 13 {
                cols[5] = "ISOLATION BALL VALVE".to_string();
            } else if cols.first().is_some_and(|k| k.contains("02011")) && cols.len() >= 13 {
                cols[9] = "HL-40".to_string();
                cols[10] = "4.0 MPa".to_string();
                cols[11] = "150 C".to_string();
            }
            edited_lines.push(cols.join(","));
        }
        let edited_csv = tmp.join("bom_filled.csv");
        std::fs::write(&edited_csv, format!("{}\n", edited_lines.join("\r\n"))).unwrap();

        // Feed filled CSV back into `PIDCONVERT ... PROPS ... VERIFY` (`StrictVendor` 0 violations!)
        let conv_dir = tmp.join("conv_out");
        let _ = app.run_command_line(&format!(
            "PIDCONVERT \"{}\" STEM BOM-RT-150 PROPS \"{}\" VERIFY",
            conv_dir.display(),
            edited_csv.display()
        ));
        let data_xml_p = conv_dir.join("BOM-RT-150_Data.xml");
        assert!(
            data_xml_p.is_file(),
            "PIDCONVERT with filled BOM PROPS must succeed"
        );
        let xml_txt = std::fs::read_to_string(&data_xml_p).unwrap();
        assert!(xml_txt.contains("ISOLATION BALL VALVE"));
        assert!(xml_txt.contains("PressureMax=\"4.0 MPa\""));
        assert!(xml_txt.contains("PhaseTemperatureMax=\"150 C\""));

        let cov: Value = serde_json::from_str(
            &std::fs::read_to_string(conv_dir.join("BOM-RT-150_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert!(
            cov["external_props_applied"]["process_points_updated"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real SPPID and native AutoCAD sheets in multi-tab OpenCADStudio, runs PIDBOM ALLTABS export, fills process temperatures/pressures, and verifies PIDCONVERT PROPS dual roundtrip"]
    fn real_multi_source_pidbom_alltabs_schedule_export_and_props_roundtrip() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_p_raw) = std::env::var("OCS_PID_REAL150_SPPID_DXF") else {
            println!("OCS_PID_REAL150_SPPID_DXF unset, skipping.");
            return;
        };
        let Ok(legacy_p_raw) = std::env::var("OCS_PID_REAL150_LEGACY_DXF") else {
            println!("OCS_PID_REAL150_LEGACY_DXF unset, skipping.");
            return;
        };
        let Ok(seed_zip) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("OCS_PID_SEED_ZIP unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_SYM_DIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL150_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real150"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_p = std::path::PathBuf::from(&sppid_p_raw);
        let legacy_p = std::path::PathBuf::from(&legacy_p_raw);
        let sppid_sha =
            format!("{:x}", Sha256::digest(std::fs::read(&sppid_p).unwrap()))[..12].to_string();
        let legacy_sha =
            format!("{:x}", Sha256::digest(std::fs::read(&legacy_p).unwrap()))[..12].to_string();

        let mut app = app();
        let tab_sp = app.active_tab;
        app.tabs[tab_sp].scene.document = crate::io::load_file(&sppid_p).unwrap();
        app.tabs[tab_sp].tab_title = "REAL150-SP01.dxf".to_string();

        let _ = app.update(Message::TabNew);
        let tab_lg = app.active_tab;
        app.tabs[tab_lg].scene.document = crate::io::load_file(&legacy_p).unwrap();
        app.tabs[tab_lg].tab_title = "REAL150-LG01.dxf".to_string();

        let _ = app.update(Message::TabSwitch(tab_sp));

        let bom_csv = out_dir.join("real150_alltabs.csv");
        let bom_json = out_dir.join("real150_alltabs.json");
        let _ = app.run_command_line(&format!("PIDBOM \"{}\" ALLTABS", bom_csv.display()));
        let _ = app.run_command_line(&format!("PIDBOM \"{}\" ALLTABS", bom_json.display()));
        assert!(bom_csv.is_file() && bom_json.is_file());

        let jb: Value = serde_json::from_str(&std::fs::read_to_string(&bom_json).unwrap()).unwrap();
        let sum = &jb["summary"];
        assert_eq!(sum["sheets_scanned"].as_u64(), Some(2));
        assert!(sum["total_unique_rows"].as_u64().unwrap_or(0) > 20);
        assert!(sum["pipeline_count"].as_u64().unwrap_or(0) > 0);

        // Fill design pressure/temperature on all pipeline rows in the exported CSV and feed back to `PIDCONVERT`
        let raw_csv = std::fs::read_to_string(&bom_csv).unwrap();
        let mut filled_lines = Vec::new();
        let mut filled_pipelines = 0usize;
        for (li, ln) in raw_csv.lines().enumerate() {
            if li == 0 {
                filled_lines.push(ln.to_string());
                continue;
            }
            let mut cols: Vec<String> = ln.split(',').map(str::to_string).collect();
            if cols.get(1).map(|s| s.as_str()) == Some("Pipeline") && cols.len() >= 14 {
                cols[9] = "HL-50".to_string();
                cols[10] = "6.4 MPa".to_string();
                cols[11] = "180 C".to_string();
                filled_pipelines += 1;
            }
            filled_lines.push(cols.join(","));
        }
        assert!(filled_pipelines > 0);
        let filled_csv = out_dir.join("real150_filled.csv");
        std::fs::write(&filled_csv, format!("{}\n", filled_lines.join("\r\n"))).unwrap();
        let _ = std::fs::remove_file(&bom_csv);
        let _ = std::fs::remove_file(&bom_json);

        let dual_dir = out_dir.join("dual_out");
        std::fs::create_dir_all(&dual_dir).unwrap();
        let target_pid = dual_dir.join("REAL150-RT.pid");
        let conv_cmd = if sym_dir.trim().is_empty() {
            format!(
                "PIDCONVERT \"{}\" SEED \"{}\" PROPS \"{}\" VERIFY",
                target_pid.display(),
                seed_zip,
                filled_csv.display()
            )
        } else {
            format!(
                "PIDCONVERT \"{}\" SEED \"{}\" SYMDIR \"{}\" PROPS \"{}\" VERIFY",
                target_pid.display(),
                seed_zip,
                sym_dir,
                filled_csv.display()
            )
        };
        let _ = app.run_command_line(&conv_cmd);
        let _ = std::fs::remove_file(&filled_csv);
        assert!(target_pid.is_file(), "PIDCONVERT dual output must exist");

        let data_xml = std::fs::read(dual_dir.join("REAL150-RT_Data.xml")).unwrap();
        let meta_xml = std::fs::read(dual_dir.join("REAL150-RT_Meta.xml")).unwrap();
        crate::io::pid_convert::verify_published_xml_pair(
            "REAL150-RT",
            "P01",
            &data_xml,
            &meta_xml,
        )
        .expect("StrictVendor clean");

        let cov: Value = serde_json::from_str(
            &std::fs::read_to_string(dual_dir.join("REAL150-RT_Coverage.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            cov["variant_resolution"]["class_default_fallback"].as_u64(),
            Some(0)
        );
        assert!(
            cov["external_props_applied"]["process_points_updated"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert_eq!(
            cov["pid_verification"]["matched_graphic_oids"].as_u64(),
            cov["pid_verification"]["xml_rep_count"].as_u64()
        );

        let sanitized = json!({
            "sppid_src_sha256_prefix": sppid_sha,
            "legacy_src_sha256_prefix": legacy_sha,
            "bom_summary": sum,
            "filled_pipeline_rows": filled_pipelines,
            "external_props_applied": cov["external_props_applied"],
            "xml_reps": cov["pid_verification"]["xml_rep_count"],
            "matched_oids": cov["pid_verification"]["matched_graphic_oids"],
            "a_valid": cov["pid_verification"]["a_level_index_consistent"],
            "pdf_prims": cov["pid_verification"]["b_level_pdf_primitives"],
        });
        std::fs::write(
            out_dir.join("real150_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn piddrc_detects_duplicates_orphan_nozzles_deadends_steps_camera_and_clears_after_fix() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, h: document::Handle) -> bool {
            app.tabs[app.active_tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();
        let (v1_h, _v2_h, untagged_h, nozzle_h, dead_pipe_h) = {
            let doc = &mut app.tabs[app.active_tab].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "GATE VALVE", 6.0, 4.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);

            // Duplicate tag `BV5101` on two ball valves -> DuplicateTag (Error)
            let mut v1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 200.0, 0.0));
            v1.common.layer = "11".to_string();
            let v1_h = doc.add_entity(document::EntityType::Insert(v1)).unwrap();
            let mut vt1 =
                Text::with_value("BV5101", Vector3::new(96.0, 206.0, 0.0)).with_height(2.5);
            vt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt1)).unwrap();

            let mut v2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 200.0, 0.0));
            v2.common.layer = "11".to_string();
            let v2_h = doc.add_entity(document::EntityType::Insert(v2)).unwrap();
            let mut vt2 =
                Text::with_value("BV5101", Vector3::new(176.0, 206.0, 0.0)).with_height(2.5);
            vt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt2)).unwrap();

            // Untagged ball valve -> UntaggedComponent (Warning)
            let mut gv = Insert::new("BALL VALVE TYPE 2", Vector3::new(320.0, 200.0, 0.0));
            gv.common.layer = "11".to_string();
            let untagged_h = doc.add_entity(document::EntityType::Insert(gv)).unwrap();

            // Isolated nozzle far from any vessel -> OrphanNozzle (Warning)
            let mut nz = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(650.0, 520.0, 0.0));
            nz.common.layer = "10".to_string();
            let nozzle_h = doc.add_entity(document::EntityType::Insert(nz)).unwrap();

            // Open-ended disconnected pipe segment -> OpenPipeDeadEnd (Warning)
            let mut dp = Line::from_points(
                Vector3::new(800.0, 100.0, 0.0),
                Vector3::new(900.0, 100.0, 0.0),
            );
            dp.common.layer = "1".to_string();
            let dead_pipe_h = doc.add_entity(document::EntityType::Line(dp)).unwrap();
            let mut ltxt = Text::with_value("50-PW-5109-CS150", Vector3::new(830.0, 103.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            (v1_h, v2_h, untagged_h, nozzle_h, dead_pipe_h)
        };

        let _ = app.run_command_line("PIDPROMOTE");

        let tmp = std::env::temp_dir().join(format!("ocs_piddrc_151_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let drc1_p = tmp.join("drc1.json");
        let _ = app.run_command_line(&format!("PIDDRC EXPORT \"{}\"", drc1_p.display()));
        assert!(drc1_p.is_file(), "PIDDRC EXPORT must write DRC report JSON");
        let rep1: Value = serde_json::from_str(&std::fs::read_to_string(&drc1_p).unwrap()).unwrap();
        assert_eq!(rep1["summary"]["ready_for_export"].as_bool(), Some(false));
        assert!(rep1["summary"]["error_count"].as_u64().unwrap_or(0) >= 1);
        assert!(rep1["summary"]["duplicate_tags"].as_u64().unwrap_or(0) >= 1);
        assert_eq!(rep1["summary"]["orphan_nozzles"].as_u64(), Some(1));
        assert_eq!(rep1["summary"]["untagged_components"].as_u64(), Some(1));
        assert!(rep1["summary"]["open_pipe_dead_ends"].as_u64().unwrap_or(0) >= 1);

        // Category selection filters
        let _ = app.run_command_line("PIDDRC ERRORS");
        assert!(has_sel(&app, v1_h));
        let _ = app.run_command_line("PIDDRC ORPHANNOZZLES");
        assert!(has_sel(&app, nozzle_h));
        let _ = app.run_command_line("PIDDRC UNTAGGED");
        assert!(has_sel(&app, untagged_h));
        let _ = app.run_command_line("PIDDRC DEADENDS");
        assert!(has_sel(&app, dead_pipe_h));

        // `PIDDRC NEXT ORPHANNOZZLES` steps cursor and centers viewport camera on `(650, 520)`
        let _ = app.run_command_line("PIDDRC NEXT ORPHANNOZZLES");
        assert!(has_sel(&app, nozzle_h));
        let cam_nz = app.tabs[app.active_tab].scene.camera.borrow().target;
        assert!(
            (cam_nz.x - 650.0).abs() < 25.0 && (cam_nz.y - 520.0).abs() < 25.0,
            "PIDDRC NEXT ORPHANNOZZLES must center camera on (650, 520), got {cam_nz:?}"
        );

        // Resolve duplicate tags + accept untagged gate valve -> errors & untagged warnings drop to 0
        let _ = app.run_command_line("PIDREVIEW DEDUP");
        app.tabs[app.active_tab].scene.deselect_all();
        app.tabs[app.active_tab]
            .scene
            .select_entities(&[untagged_h]);
        let _ = app.run_command_line("PIDREVIEW ACCEPT");

        let drc2_p = tmp.join("drc2.json");
        let _ = app.run_command_line(&format!("PIDDRC EXPORT \"{}\"", drc2_p.display()));
        let rep2: Value = serde_json::from_str(&std::fs::read_to_string(&drc2_p).unwrap()).unwrap();
        assert_eq!(rep2["summary"]["error_count"].as_u64(), Some(0));
        assert_eq!(rep2["summary"]["ready_for_export"].as_bool(), Some(true));
        assert_eq!(rep2["summary"]["duplicate_tags"].as_u64(), Some(0));
        assert_eq!(rep2["summary"]["untagged_components"].as_u64(), Some(0));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real SPPID and native AutoCAD sheets inside OpenCADStudio through PIDDRC audit, PIDREVIEW DEDUP remediation, and PIDDRC NEXT camera stepping"]
    fn real_multi_source_piddrc_audit_and_next_step_focus() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL151_SPPID_DXF") else {
            println!("OCS_PID_REAL151_SPPID_DXF unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL151_LEGACY_DXF") else {
            println!("OCS_PID_REAL151_LEGACY_DXF unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL151_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real151"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let cases = [
            ("SP01", "sppid_dxf", std::path::PathBuf::from(&sppid_raw)),
            (
                "LG01",
                "legacy_autocad_dxf",
                std::path::PathBuf::from(&legacy_raw),
            ),
        ];

        let mut rows = Vec::new();
        for (cid, kind, path) in &cases {
            let sha_pfx =
                format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))[..12].to_string();
            let mut app = app();
            app.tabs[app.active_tab].scene.document = crate::io::load_file(path).unwrap();
            app.tabs[app.active_tab].tab_title = format!("{cid}.dxf");

            let _ = app.run_command_line("PIDPROMOTE");
            let before_p = out_dir.join(format!("{cid}_before.json"));
            let _ = app.run_command_line(&format!("PIDDRC EXPORT \"{}\"", before_p.display()));
            let rep_before: Value =
                serde_json::from_str(&std::fs::read_to_string(&before_p).unwrap()).unwrap();
            let _ = std::fs::remove_file(&before_p);

            if rep_before["summary"]["duplicate_tags"]
                .as_u64()
                .unwrap_or(0)
                > 0
            {
                let _ = app.run_command_line("PIDREVIEW DEDUP");
            }

            let after_p = out_dir.join(format!("{cid}_after.json"));
            let _ = app.run_command_line(&format!("PIDDRC EXPORT \"{}\"", after_p.display()));
            let rep_after: Value =
                serde_json::from_str(&std::fs::read_to_string(&after_p).unwrap()).unwrap();
            let _ = std::fs::remove_file(&after_p);

            assert_eq!(rep_after["summary"]["error_count"].as_u64(), Some(0));
            assert_eq!(
                rep_after["summary"]["ready_for_export"].as_bool(),
                Some(true)
            );

            if rep_after["summary"]["total_findings"].as_u64().unwrap_or(0) > 0 {
                let _ = app.run_command_line("PIDDRC NEXT");
                assert!(!app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty());
            }

            rows.push(json!({
                "id": cid,
                "kind": kind,
                "sha256_prefix": sha_pfx,
                "before": rep_before["summary"],
                "after": rep_after["summary"],
            }));
        }

        std::fs::write(
            out_dir.join("real151_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({"sheets": rows})).unwrap()
            ),
        )
        .unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidfind_searches_tags_and_full_pipelines_across_tabs_and_focuses_camera() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-152-01.dxf".to_string();
        let (_, _, inst_h, inst_top_h, inst_bot_h, _) = seed_dxf_sheet_in_app(&mut app);

        // Tab 2: promoted `PID_AUTHOR` sheet with valve `BV0209901B` at (740, 580) and 2-segment pipeline `100-PW-6205-CS150`
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-152-02.dxf".to_string();
        let (v2_h, seg_a_h, seg_b_h) = {
            let doc2 = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc2, "BALL VALVE TYPE 2", 6.0, 4.0);

            let mut v2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(740.0, 580.0, 0.0));
            v2.common.layer = "11".to_string();
            let v2_h = doc2.add_entity(document::EntityType::Insert(v2)).unwrap();
            let mut vt2 =
                Text::with_value("BV0209901B", Vector3::new(736.0, 586.0, 0.0)).with_height(2.5);
            vt2.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(vt2)).unwrap();

            let mut sa = Line::from_points(
                Vector3::new(400.0, 300.0, 0.0),
                Vector3::new(500.0, 300.0, 0.0),
            );
            sa.common.layer = "1".to_string();
            let seg_a_h = doc2.add_entity(document::EntityType::Line(sa)).unwrap();

            let mut sb = Line::from_points(
                Vector3::new(500.0, 300.0, 0.0),
                Vector3::new(500.0, 420.0, 0.0),
            );
            sb.common.layer = "1".to_string();
            let seg_b_h = doc2.add_entity(document::EntityType::Line(sb)).unwrap();

            let mut ll = Text::with_value("100-PW-6205-CS150", Vector3::new(430.0, 303.0, 0.0))
                .with_height(2.5);
            ll.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(ll)).unwrap();

            (v2_h, seg_a_h, seg_b_h)
        };
        let _ = app.run_command_line("PIDPROMOTE");

        // Switch back to unpromoted Tab 1 and search across tabs with hyphenated spelling `BV-0209901B`
        let _ = app.update(Message::TabSwitch(tab1));
        assert_eq!(app.active_tab, tab1);

        let _ = app.run_command_line("PIDFIND BV-0209901B ALLTABS");
        assert_eq!(
            app.active_tab, tab2,
            "PIDFIND ALLTABS must switch to Tab 2 where BV0209901B lives"
        );
        assert!(has_sel(&app, tab2, v2_h));
        let cam2 = app.tabs[tab2].scene.camera.borrow().target;
        assert!(
            (cam2.x - 740.0).abs() < 25.0 && (cam2.y - 580.0).abs() < 25.0,
            "camera must center on (740, 580), got {cam2:?}"
        );

        // Search pipeline substring `6205` -> selects ALL segments (`seg_a_h` AND `seg_b_h`) of `100-PW-6205-CS150`
        let _ = app.run_command_line("PIDFIND 6205");
        assert!(has_sel(&app, tab2, seg_a_h));
        assert!(has_sel(&app, tab2, seg_b_h));

        // Search `PT-02013 ALLTABS` from Tab 2 -> jumps back to unpromoted Tab 1 and selects instrument + 2-line texts
        let _ = app.run_command_line("PIDFIND PT-02013 ALLTABS");
        assert_eq!(app.active_tab, tab1);
        assert!(has_sel(&app, tab1, inst_h));
        assert!(has_sel(&app, tab1, inst_top_h));
        assert!(has_sel(&app, tab1, inst_bot_h));
        let cam1 = app.tabs[tab1].scene.camera.borrow().target;
        assert!(
            (cam1.x - 110.0).abs() < 25.0 && (cam1.y - 100.0).abs() < 25.0,
            "camera must center on (110, 100), got {cam1:?}"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real SPPID DXF, native AutoCAD legacy DXF, and real .pid file into 3 tabs and verifies PIDFIND ALLTABS cross-tab symbol & full-pipeline search/focus"]
    fn real_multi_source_pidfind_cross_tab_tag_and_pipeline_jump() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL152_SPPID_DXF") else {
            println!("OCS_PID_REAL152_SPPID_DXF unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL152_LEGACY_DXF") else {
            println!("OCS_PID_REAL152_LEGACY_DXF unset, skipping.");
            return;
        };
        let Ok(pid_raw) = std::env::var("OCS_PID_REAL152_PID_FILE") else {
            println!("OCS_PID_REAL152_PID_FILE unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL152_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real152"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let files = [
            ("SP01", std::path::PathBuf::from(&sppid_raw)),
            ("LG01", std::path::PathBuf::from(&legacy_raw)),
            ("PD01", std::path::PathBuf::from(&pid_raw)),
        ];

        let mut app = app();
        let mut tabs = Vec::new();
        let mut shas = Vec::new();
        for (idx, (cid, p)) in files.iter().enumerate() {
            shas.push(format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tabs.push(ti);
        }

        // Promote PD01 (.pid) so all three representations coexist (unpromoted SPPID DXF, unpromoted Legacy DXF, promoted .pid)
        let _ = app.update(Message::TabSwitch(tabs[2]));
        let _ = app.run_command_line("PIDPROMOTE");

        let sheet_boms: Vec<_> = tabs
            .iter()
            .map(|&ti| {
                crate::io::pid_convert::extract_bom_from_documents(
                    &[crate::io::pid_convert::BomSheetInput {
                        drawing_name: "S",
                        document: &app.tabs[ti].scene.document,
                    }],
                    &crate::io::pid_convert::ConvertOptions::default(),
                )
                .unwrap()
            })
            .collect();

        let pick_unique = |target_idx: usize, want_cat: &str| -> String {
            let candidates = sheet_boms[target_idx]
                .rows
                .iter()
                .filter(|r| r.category == want_cat)
                .chain(sheet_boms[target_idx].rows.iter());
            for row in candidates {
                let unique = sheet_boms.iter().enumerate().all(|(oi, ob)| {
                    oi == target_idx
                        || ob.rows.iter().all(|orow| {
                            crate::io::pid_convert::match_query_rank(&orow.item_tag, &row.item_tag)
                                .is_none()
                        })
                });
                if unique {
                    return row.item_tag.clone();
                }
            }
            sheet_boms[target_idx].rows[0].item_tag.clone()
        };

        let sp_pipe_tag = pick_unique(0, "Pipeline");
        let lg_sym_tag = pick_unique(1, "Instrument");
        let pd_sym_tag = pick_unique(2, "Equipment");

        // 1. From Tab 2 (PD01), `PIDFIND <sp_pipe_tag> ALLTABS` jumps to Tab 0 (SP01) and selects multi-segment pipeline
        let _ = app.update(Message::TabSwitch(tabs[2]));
        let _ = app.run_command_line(&format!("PIDFIND \"{sp_pipe_tag}\" ALLTABS"));
        assert_eq!(app.active_tab, tabs[0]);
        let sp_pipe_sel = app.tabs[tabs[0]].scene.selected_entities().len();
        assert!(sp_pipe_sel >= 1);

        // 2. From Tab 0 (SP01), `PIDFIND <lg_sym_tag> ALLTABS` jumps to Tab 1 (LG01)
        let _ = app.run_command_line(&format!("PIDFIND \"{lg_sym_tag}\" ALLTABS"));
        assert_eq!(app.active_tab, tabs[1]);
        let lg_sel = app.tabs[tabs[1]].scene.selected_entities().len();
        assert!(lg_sel >= 1);

        // 3. From Tab 1 (LG01), `PIDFIND <pd_sym_tag> ALLTABS` jumps to Tab 2 (PD01)
        let _ = app.run_command_line(&format!("PIDFIND \"{pd_sym_tag}\" ALLTABS"));
        assert_eq!(app.active_tab, tabs[2]);
        let pd_sel = app.tabs[tabs[2]].scene.selected_entities().len();
        assert!(pd_sel >= 1);

        let sanitized = json!({
            "tabs_loaded": 3,
            "sppid_sha256_prefix": shas[0],
            "legacy_sha256_prefix": shas[1],
            "pid_sha256_prefix": shas[2],
            "sppid_pipeline_entities_selected": sp_pipe_sel,
            "legacy_symbol_entities_selected": lg_sel,
            "pid_symbol_entities_selected": pd_sel,
            "all_cross_tab_jumps_verified": true,
        });
        std::fs::write(
            out_dir.join("real152_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidtrace_traces_connected_subnet_and_cross_tab_opc_shortest_path() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-153-01.dxf".to_string();
        let (pump_h, v1_h, pipe1_h, opc1_h, iso_v_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut border = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            border.common.layer = "Default".to_string();
            doc.add_entity(document::EntityType::Line(border)).unwrap();
            let mut tb =
                Text::with_value("DWG-153-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Pump P-1531A at (80, 300)
            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(80.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            let pump_h = doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1531A", Vector3::new(75.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            // Pipe 1: (84, 300) -> (160, 300)
            let mut p1 = Line::from_points(
                Vector3::new(84.0, 300.0, 0.0),
                Vector3::new(160.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            let pipe1_h = doc.add_entity(document::EntityType::Line(p1)).unwrap();
            let mut l1 = Text::with_value("100-PW-1531-CS150", Vector3::new(105.0, 303.0, 0.0))
                .with_height(2.5);
            l1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(l1)).unwrap();

            // Inline Valve BV153101 at (163, 300)
            let mut v1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(163.0, 300.0, 0.0));
            v1.common.layer = "11".to_string();
            let v1_h = doc.add_entity(document::EntityType::Insert(v1)).unwrap();
            let mut vt1 =
                Text::with_value("BV153101", Vector3::new(159.0, 306.0, 0.0)).with_height(2.5);
            vt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt1)).unwrap();

            // Pipe 2: (166, 300) -> (254, 300)
            let mut p2 = Line::from_points(
                Vector3::new(166.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();

            // Paired OPC at (260, 300) -> DWG-153-02
            let mut opc = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc.common.layer = "11".to_string();
            let opc1_h = doc.add_entity(document::EntityType::Insert(opc)).unwrap();
            let mut tf =
                Text::with_value("DWG-153-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            tf.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf)).unwrap();

            // Isolated disconnected valve BV999999 at (800, 700)
            let mut iv = Insert::new("BALL VALVE TYPE 2", Vector3::new(800.0, 700.0, 0.0));
            iv.common.layer = "11".to_string();
            let iso_v_h = doc.add_entity(document::EntityType::Insert(iv)).unwrap();
            let mut ivt =
                Text::with_value("BV999999", Vector3::new(796.0, 706.0, 0.0)).with_height(2.5);
            ivt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ivt)).unwrap();

            (pump_h, v1_h, pipe1_h, opc1_h, iso_v_h)
        };

        // Tab 2: Return OPC at (120, 420) -> BV153201 -> Vessel V-1532 at (356, 420)
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-153-02.dxf".to_string();
        let (v2_h, drum_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut border = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            border.common.layer = "Default".to_string();
            doc.add_entity(document::EntityType::Line(border)).unwrap();
            let mut tb =
                Text::with_value("DWG-153-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(120.0, 420.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut tf2 =
                Text::with_value("DWG-153-01", Vector3::new(110.0, 419.0, 0.0)).with_height(2.5);
            tf2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(126.0, 420.0, 0.0),
                Vector3::new(240.0, 420.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PW-1531-CS150", Vector3::new(160.0, 423.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut v2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(243.0, 420.0, 0.0));
            v2.common.layer = "11".to_string();
            let v2_h = doc.add_entity(document::EntityType::Insert(v2)).unwrap();
            let mut vt2 =
                Text::with_value("BV153201", Vector3::new(239.0, 426.0, 0.0)).with_height(2.5);
            vt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt2)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(246.0, 420.0, 0.0),
                Vector3::new(350.0, 420.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(356.0, 420.0, 0.0));
            drum.common.layer = "10".to_string();
            let drum_h = doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1532", Vector3::new(352.0, 435.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            (v2_h, drum_h)
        };

        let tmp = std::env::temp_dir().join(format!("ocs_pidtrace_153_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Select pump P-1531A on Tab 1 and trace connected subnet across tabs
        let _ = app.update(Message::TabSwitch(tab1));
        app.tabs[tab1].scene.deselect_all();
        app.tabs[tab1].scene.select_entities(&[pump_h]);
        let subnet_p = tmp.join("subnet.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE ALLTABS EXPORT \"{}\"",
            subnet_p.display()
        ));
        assert!(
            subnet_p.is_file(),
            "PIDTRACE EXPORT must write subnet report JSON"
        );
        let sub_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&subnet_p).unwrap()).unwrap();
        assert_eq!(sub_rep["reachable"].as_bool(), Some(true));
        assert_eq!(sub_rep["crossed_tabs"].as_u64(), Some(2));
        assert!(sub_rep["opc_bridges_traversed"].as_u64().unwrap_or(0) >= 1);
        let eq_tags: Vec<&str> = sub_rep["equipment_tags"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(eq_tags.contains(&"P-1531A") && eq_tags.contains(&"V-1532"));
        let v_tags: Vec<&str> = sub_rep["valve_tags"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(v_tags.contains(&"BV153101") && v_tags.contains(&"BV153201"));
        assert!(!v_tags.contains(&"BV999999"));
        assert!(has_sel(&app, tab1, pump_h));
        assert!(has_sel(&app, tab1, v1_h));
        assert!(has_sel(&app, tab1, pipe1_h));
        assert!(has_sel(&app, tab1, opc1_h));
        assert!(!has_sel(&app, tab1, iso_v_h));

        // 2. Shortest process path from P-1531A to V-1532 across paired OPC
        let path_p = tmp.join("path.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE P-1531A V-1532 ALLTABS EXPORT \"{}\"",
            path_p.display()
        ));
        let path_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&path_p).unwrap()).unwrap();
        assert_eq!(path_rep["mode"].as_str(), Some("shortest_path"));
        assert_eq!(path_rep["reachable"].as_bool(), Some(true));
        assert_eq!(path_rep["crossed_tabs"].as_u64(), Some(2));
        assert_eq!(path_rep["opc_bridges_traversed"].as_u64(), Some(1));
        assert_eq!(app.active_tab, tab2);
        assert!(has_sel(&app, tab2, v2_h));
        assert!(has_sel(&app, tab2, drum_h));

        // 3. Disconnected valve BV999999 must be reported unreachable from P-1531A
        let unreach_p = tmp.join("unreach.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE P-1531A BV999999 ALLTABS EXPORT \"{}\"",
            unreach_p.display()
        ));
        let unreach_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&unreach_p).unwrap()).unwrap();
        assert_eq!(unreach_rep["reachable"].as_bool(), Some(false));

        // 4. Promote both sheets to PID_AUTHOR and verify shortest path parity
        let _ = app.update(Message::TabSwitch(tab1));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab2));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab1));
        let prom_p = tmp.join("promoted_path.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE P-1531A V-1532 ALLTABS EXPORT \"{}\"",
            prom_p.display()
        ));
        let prom_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&prom_p).unwrap()).unwrap();
        assert_eq!(prom_rep["reachable"].as_bool(), Some(true));
        assert_eq!(prom_rep["crossed_tabs"].as_u64(), Some(2));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDTRACE ALLTABS cross-tab subnet & shortest-path tracing"]
    fn real_multi_tab_pidtrace_cross_sheet_subnet_and_shortest_path() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL153_SPPID_PAIR") else {
            println!("OCS_PID_REAL153_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL153_LEGACY_PAIR") else {
            println!("OCS_PID_REAL153_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL153_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real153"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            &sppid_paths[0],
            &sppid_paths[1],
            &legacy_paths[0],
            &legacy_paths[1],
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, p) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].current_path = Some((*p).clone());
            app.tabs[ti].tab_title = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("sheet.dxf")
                .to_string();
            tab_indices.push(ti);
        }

        // 1. SPPID paired sheets (Tab 0 <-> Tab 1): select paired OPC on Tab 0 and run PIDTRACE ALLTABS
        let _ = app.update(Message::TabSwitch(tab_indices[0]));
        let _ = app.run_command_line("PIDOPC PAIRED");
        let sp_paired_sel: Vec<document::Handle> = app.tabs[tab_indices[0]]
            .scene
            .selected_entities()
            .into_iter()
            .map(|(h, _)| h)
            .collect();
        assert!(!sp_paired_sel.is_empty());
        app.tabs[tab_indices[0]].scene.deselect_all();
        app.tabs[tab_indices[0]]
            .scene
            .select_entities(&[sp_paired_sel[0]]);

        let sp_sub_p = out_dir.join("sp_subnet_tmp.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE ALLTABS EXPORT \"{}\"",
            sp_sub_p.display()
        ));
        let sp_sub: Value =
            serde_json::from_str(&std::fs::read_to_string(&sp_sub_p).unwrap()).unwrap();
        let _ = std::fs::remove_file(&sp_sub_p);

        assert_eq!(sp_sub["reachable"].as_bool(), Some(true));
        assert!(
            sp_sub["crossed_tabs"].as_u64().unwrap_or(0) >= 2,
            "SPPID paired trace must span >=2 open tabs, got {:?}",
            sp_sub["crossed_tabs"]
        );
        assert!(sp_sub["opc_bridges_traversed"].as_u64().unwrap_or(0) >= 1);

        // Also verify shortest-path query across Tab 0 -> Tab 1 using steps discovered on each tab
        let steps = sp_sub["path_steps"].as_array().unwrap();
        let start_tag_opt = steps.iter().find_map(|st| {
            (st["tab_index"].as_u64() == Some(tab_indices[0] as u64)
                && st["node_kind"].as_str() != Some("opc"))
            .then(|| st["tag_or_label"].as_str())
            .flatten()
            .map(str::to_string)
        });
        let end_tag_opt = steps.iter().find_map(|st| {
            (st["tab_index"].as_u64() == Some(tab_indices[1] as u64)
                && st["node_kind"].as_str() != Some("opc"))
            .then(|| st["tag_or_label"].as_str())
            .flatten()
            .map(str::to_string)
        });

        let mut shortest_path_verified = false;
        let mut sp_path_steps_count = 0u64;
        let cross_pair = start_tag_opt
            .zip(end_tag_opt)
            .filter(|(s, e)| crate::io::pid_convert::match_query_rank(s, e).is_none());
        if let Some((s_tag, e_tag)) = cross_pair {
            let _ = app.update(Message::TabSwitch(tab_indices[0]));
            let sp_path_p = out_dir.join("sp_path_tmp.json");
            let _ = app.run_command_line(&format!(
                "PIDTRACE \"{s_tag}\" \"{e_tag}\" ALLTABS EXPORT \"{}\"",
                sp_path_p.display()
            ));
            let sp_path: Value =
                serde_json::from_str(&std::fs::read_to_string(&sp_path_p).unwrap()).unwrap();
            let _ = std::fs::remove_file(&sp_path_p);
            assert_eq!(sp_path["reachable"].as_bool(), Some(true));
            assert!(sp_path["crossed_tabs"].as_u64().unwrap_or(0) >= 2);
            sp_path_steps_count = sp_path["path_steps"]
                .as_array()
                .map(|a| a.len() as u64)
                .unwrap_or(0);
            shortest_path_verified = true;
        }

        // 2. Native AutoCAD legacy paired sheets (Tab 2 <-> Tab 3): select paired OPC on Tab 2 and run PIDTRACE ALLTABS
        let _ = app.update(Message::TabSwitch(tab_indices[2]));
        let _ = app.run_command_line("PIDOPC PAIRED");
        let lg_paired_sel: Vec<document::Handle> = app.tabs[tab_indices[2]]
            .scene
            .selected_entities()
            .into_iter()
            .map(|(h, _)| h)
            .collect();
        assert!(!lg_paired_sel.is_empty());
        app.tabs[tab_indices[2]].scene.deselect_all();
        app.tabs[tab_indices[2]]
            .scene
            .select_entities(&[lg_paired_sel[0]]);

        let lg_sub_p = out_dir.join("lg_subnet_tmp.json");
        let _ = app.run_command_line(&format!(
            "PIDTRACE ALLTABS EXPORT \"{}\"",
            lg_sub_p.display()
        ));
        let lg_sub: Value =
            serde_json::from_str(&std::fs::read_to_string(&lg_sub_p).unwrap()).unwrap();
        let _ = std::fs::remove_file(&lg_sub_p);

        assert_eq!(lg_sub["reachable"].as_bool(), Some(true));
        assert!(
            lg_sub["crossed_tabs"].as_u64().unwrap_or(0) >= 2,
            "Legacy paired trace must span >=2 open tabs, got {:?}",
            lg_sub["crossed_tabs"]
        );
        assert!(lg_sub["opc_bridges_traversed"].as_u64().unwrap_or(0) >= 1);

        let sanitized = json!({
            "tabs_loaded": 4,
            "sppid_sha256_prefixes": [&sha_prefixes[0], &sha_prefixes[1]],
            "legacy_sha256_prefixes": [&sha_prefixes[2], &sha_prefixes[3]],
            "sppid_subnet": {
                "crossed_tabs": sp_sub["crossed_tabs"],
                "opc_bridges_traversed": sp_sub["opc_bridges_traversed"],
                "total_nodes_visited": sp_sub["total_nodes_visited"],
                "pipe_segments_count": sp_sub["pipe_segments_count"],
                "equipment_count": sp_sub["equipment_tags"].as_array().map(Vec::len).unwrap_or(0),
                "valve_count": sp_sub["valve_tags"].as_array().map(Vec::len).unwrap_or(0),
                "instrument_count": sp_sub["instrument_tags"].as_array().map(Vec::len).unwrap_or(0),
                "pipeline_count": sp_sub["pipeline_labels"].as_array().map(Vec::len).unwrap_or(0),
            },
            "sppid_cross_tab_shortest_path": {
                "verified": shortest_path_verified,
                "path_steps_count": sp_path_steps_count,
            },
            "legacy_subnet": {
                "crossed_tabs": lg_sub["crossed_tabs"],
                "opc_bridges_traversed": lg_sub["opc_bridges_traversed"],
                "total_nodes_visited": lg_sub["total_nodes_visited"],
                "pipe_segments_count": lg_sub["pipe_segments_count"],
                "equipment_count": lg_sub["equipment_tags"].as_array().map(Vec::len).unwrap_or(0),
                "valve_count": lg_sub["valve_tags"].as_array().map(Vec::len).unwrap_or(0),
                "instrument_count": lg_sub["instrument_tags"].as_array().map(Vec::len).unwrap_or(0),
                "pipeline_count": lg_sub["pipeline_labels"].as_array().map(Vec::len).unwrap_or(0),
            },
        });
        std::fs::write(
            out_dir.join("real153_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn piddiff_ignores_layout_shifts_detects_added_removed_modified_and_steps_camera() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: Baseline Rev 0 ("DWG-154-R0.dxf")
        let tab_r0 = app.active_tab;
        app.tabs[tab_r0].tab_title = "DWG-154-R0.dxf".to_string();
        {
            let doc = &mut app.tabs[tab_r0].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-154-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut p0 = Insert::new("CENTRIFUGAL PUMP", Vector3::new(80.0, 200.0, 0.0));
            p0.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(p0)).unwrap();
            let mut pt0 =
                Text::with_value("P-1541", Vector3::new(76.0, 208.0, 0.0)).with_height(2.5);
            pt0.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt0)).unwrap();

            let mut v1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(160.0, 200.0, 0.0));
            v1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(v1)).unwrap();
            let mut vt1 =
                Text::with_value("BV154101", Vector3::new(156.0, 206.0, 0.0)).with_height(2.5);
            vt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt1)).unwrap();

            // Valve BV154102 -> will be REMOVED in Rev A
            let mut v2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(240.0, 200.0, 0.0));
            v2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(v2)).unwrap();
            let mut vt2 =
                Text::with_value("BV154102", Vector3::new(236.0, 206.0, 0.0)).with_height(2.5);
            vt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt2)).unwrap();

            // Pipeline 100-PW-1541-CS150 -> spec will change to SS300 in Rev A
            let mut ln = Line::from_points(
                Vector3::new(84.0, 200.0, 0.0),
                Vector3::new(304.0, 200.0, 0.0),
            );
            ln.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(ln)).unwrap();
            let mut lt = Text::with_value("100-PW-1541-CS150", Vector3::new(180.0, 203.0, 0.0))
                .with_height(2.5);
            lt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(lt)).unwrap();

            // OPC targeting DWG-154-02 -> will change target to DWG-154-05 in Rev A
            let mut opc = Insert::new("_OFF-UNIT SQ", Vector3::new(310.0, 200.0, 0.0));
            opc.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc)).unwrap();
            let mut oft =
                Text::with_value("DWG-154-02", Vector3::new(300.0, 199.0, 0.0)).with_height(2.5);
            oft.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft)).unwrap();
        }

        // Tab 2: Current Rev A ("DWG-154-RA.dxf")
        let _ = app.update(Message::TabNew);
        let tab_ra = app.active_tab;
        app.tabs[tab_ra].tab_title = "DWG-154-RA.dxf".to_string();
        let (pump_ra_h, seg1_h, seg2_h, opc_ra_h, drum_ra_h) = {
            let doc = &mut app.tabs[tab_ra].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut tb =
                Text::with_value("DWG-154-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Pump P-1541 shifted purely graphically (+8, +4) -> MUST NOT trigger false diff
            let mut p0 = Insert::new("CENTRIFUGAL PUMP", Vector3::new(88.0, 204.0, 0.0));
            p0.common.layer = "10".to_string();
            let pump_ra_h = doc.add_entity(document::EntityType::Insert(p0)).unwrap();
            let mut pt0 =
                Text::with_value("P-1541", Vector3::new(84.0, 212.0, 0.0)).with_height(2.5);
            pt0.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt0)).unwrap();

            // Valve BV154101 unchanged
            let mut v1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(160.0, 200.0, 0.0));
            v1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(v1)).unwrap();
            let mut vt1 =
                Text::with_value("BV154101", Vector3::new(156.0, 206.0, 0.0)).with_height(2.5);
            vt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt1)).unwrap();

            // Pipeline 100-PW-1541 upgraded from CS150 -> SS300 across 2 connected segments
            let mut s1 = Line::from_points(
                Vector3::new(92.0, 200.0, 0.0),
                Vector3::new(220.0, 200.0, 0.0),
            );
            s1.common.layer = "1".to_string();
            let seg1_h = doc.add_entity(document::EntityType::Line(s1)).unwrap();

            let mut s2 = Line::from_points(
                Vector3::new(220.0, 200.0, 0.0),
                Vector3::new(304.0, 200.0, 0.0),
            );
            s2.common.layer = "1".to_string();
            let seg2_h = doc.add_entity(document::EntityType::Line(s2)).unwrap();

            let mut lt = Text::with_value("100-PW-1541-SS300", Vector3::new(180.0, 203.0, 0.0))
                .with_height(2.5);
            lt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(lt)).unwrap();

            // OPC retargeted to DWG-154-05 -> Modified
            let mut opc = Insert::new("_OFF-UNIT SQ", Vector3::new(310.0, 200.0, 0.0));
            opc.common.layer = "11".to_string();
            let opc_ra_h = doc.add_entity(document::EntityType::Insert(opc)).unwrap();
            let mut oft =
                Text::with_value("DWG-154-05", Vector3::new(300.0, 199.0, 0.0)).with_height(2.5);
            oft.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft)).unwrap();

            // Newly added Vessel V-1549 at (520, 460) -> Added
            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(520.0, 460.0, 0.0));
            drum.common.layer = "10".to_string();
            let drum_ra_h = doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1549", Vector3::new(516.0, 475.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            (pump_ra_h, seg1_h, seg2_h, opc_ra_h, drum_ra_h)
        };

        let tmp = std::env::temp_dir().join(format!("ocs_piddiff_154_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let json_p = tmp.join("diff.json");
        let csv_p = tmp.join("diff.csv");
        let _ = app.run_command_line(&format!(
            "PIDDIFF \"DWG-154-R0.dxf\" EXPORT \"{}\"",
            json_p.display()
        ));
        assert!(
            json_p.is_file(),
            "PIDDIFF EXPORT must write diff JSON report"
        );
        let _ = app.run_command_line(&format!(
            "PIDDIFF \"DWG-154-R0.dxf\" EXPORT \"{}\"",
            csv_p.display()
        ));
        let csv_raw = std::fs::read_to_string(&csv_p).unwrap();
        assert!(csv_raw.starts_with('\u{feff}') && csv_raw.contains("V-1549"));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
        assert_eq!(rep["summary"]["added_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["removed_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["modified_count"].as_u64(), Some(2));
        assert!(rep["summary"]["unchanged_count"].as_u64().unwrap_or(0) >= 2);

        // Category canvas selection filters
        let _ = app.run_command_line("PIDDIFF \"DWG-154-R0.dxf\" ADDED");
        assert!(has_sel(&app, tab_ra, drum_ra_h));
        assert!(!has_sel(&app, tab_ra, pump_ra_h));

        let _ = app.run_command_line("PIDDIFF \"DWG-154-R0.dxf\" MODIFIED");
        assert!(has_sel(&app, tab_ra, seg1_h));
        assert!(has_sel(&app, tab_ra, seg2_h));
        assert!(has_sel(&app, tab_ra, opc_ra_h));
        assert!(!has_sel(&app, tab_ra, pump_ra_h));

        // Stateless NEXT stepping moves viewport camera onto first active change (V-1549 at 520, 460)
        app.tabs[tab_ra].scene.deselect_all();
        let _ = app.run_command_line("PIDDIFF \"DWG-154-R0.dxf\" NEXT");
        assert!(has_sel(&app, tab_ra, drum_ra_h));
        let cam = app.tabs[tab_ra].scene.camera.borrow().target;
        assert!(
            (cam.x - 520.0).abs() < 30.0 && (cam.y - 460.0).abs() < 30.0,
            "camera must focus on added vessel V-1549 at (520, 460), got {cam:?}"
        );

        // Cross-representation parity: promote Rev A to PID_AUTHOR and diff against unpromoted Rev 0
        let _ = app.run_command_line("PIDPROMOTE");
        let prom_json = tmp.join("diff_promoted.json");
        let _ = app.run_command_line(&format!(
            "PIDDIFF \"DWG-154-R0.dxf\" EXPORT \"{}\"",
            prom_json.display()
        ));
        let prom_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&prom_json).unwrap()).unwrap();
        assert_eq!(prom_rep["summary"]["added_count"].as_u64(), Some(1));
        assert_eq!(prom_rep["summary"]["removed_count"].as_u64(), Some(1));
        assert_eq!(prom_rep["summary"]["modified_count"].as_u64(), Some(2));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real SPPID and native AutoCAD sheets inside OpenCADStudio through layout-shift zero-false-positive verification, simulated revision edits, PIDDIFF EXPORT, and PIDDIFF NEXT"]
    fn real_multi_source_piddiff_revision_audit_and_next_step_focus() {
        use document::entities::Insert;
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL154_SPPID_DXF") else {
            println!("OCS_PID_REAL154_SPPID_DXF unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL154_LEGACY_DXF") else {
            println!("OCS_PID_REAL154_LEGACY_DXF unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL154_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real154"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let cases = [
            ("SP01", "sppid_dxf", std::path::PathBuf::from(&sppid_raw)),
            (
                "LG01",
                "legacy_autocad_dxf",
                std::path::PathBuf::from(&legacy_raw),
            ),
        ];

        let mut rows = Vec::new();
        for (cid, kind, path) in &cases {
            let sha_pfx =
                format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))[..12].to_string();
            let mut app = app();

            let tab_r0 = app.active_tab;
            app.tabs[tab_r0].scene.document = crate::io::load_file(path).unwrap();
            app.tabs[tab_r0].tab_title = format!("{cid}-R0.dxf");

            let _ = app.update(Message::TabNew);
            let tab_ra = app.active_tab;
            app.tabs[tab_ra].scene.document = crate::io::load_file(path).unwrap();
            app.tabs[tab_ra].tab_title = format!("{cid}-RA.dxf");

            // 1. Verify zero false positives on pure layout coordinate nudge
            let first_ins_h = app.tabs[tab_ra]
                .scene
                .document
                .model_space_entities()
                .find_map(|e| match e {
                    document::EntityType::Insert(ins) => Some(ins.common.handle),
                    _ => None,
                });
            if let Some(ih) = first_ins_h {
                if let Some(document::EntityType::Insert(ins)) =
                    app.tabs[tab_ra].scene.document.get_entity_mut(ih)
                {
                    ins.insert_point.x += 0.05;
                    ins.insert_point.y += 0.05;
                }
            }

            let nudge_p = out_dir.join(format!("{cid}_nudge.json"));
            let _ = app.run_command_line(&format!(
                "PIDDIFF \"{cid}-R0.dxf\" EXPORT \"{}\"",
                nudge_p.display()
            ));
            let nudge_rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&nudge_p).unwrap()).unwrap();
            let _ = std::fs::remove_file(&nudge_p);
            assert_eq!(
                nudge_rep["summary"]["total_changes"].as_u64(),
                Some(0),
                "{cid}: pure graphical layout nudge must produce 0 semantic diffs"
            );
            let baseline_unchanged = nudge_rep["summary"]["unchanged_count"]
                .as_u64()
                .unwrap_or(0);
            assert!(baseline_unchanged >= 15);

            // 2. Promote Rev A and apply 3 real engineering changes (1 Modified, 1 Removed, 1 Added)
            app.tabs[tab_ra].scene.document = crate::io::load_file(path).unwrap();
            let _ = app.run_command_line("PIDPROMOTE");

            let authored_tagged: Vec<document::Handle> = app.tabs[tab_ra]
                .scene
                .document
                .model_space_entities()
                .filter_map(|e| {
                    let h = e.common().handle;
                    let a = PidAuthor::load(&app.tabs[tab_ra].scene.document, h)?;
                    (!a.tag.trim().is_empty() && a.type_code != "pipe_segment").then_some(h)
                })
                .collect();
            assert!(authored_tagged.len() >= 2);

            // Modify first tagged item's Description via PIDSETPROP
            app.tabs[tab_ra].scene.deselect_all();
            app.tabs[tab_ra]
                .scene
                .select_entities(&[authored_tagged[0]]);
            let _ = app.run_command_line("PIDSETPROP 说明 REV_A_UPGRADE");

            // Reject/remove second unique-tag item
            let mod_tag = PidAuthor::load(&app.tabs[tab_ra].scene.document, authored_tagged[0])
                .unwrap()
                .tag;
            let rem_cand = authored_tagged
                .iter()
                .copied()
                .find(|&h| {
                    let a = PidAuthor::load(&app.tabs[tab_ra].scene.document, h).unwrap();
                    a.tag != mod_tag && a.tag_status == "ok"
                })
                .unwrap_or(authored_tagged[1]);
            app.tabs[tab_ra].scene.deselect_all();
            app.tabs[tab_ra].scene.select_entities(&[rem_cand]);
            let _ = app.run_command_line("PIDREVIEW REJECT");

            // Add a new authored vessel V-9954
            {
                let doc = &mut app.tabs[tab_ra].scene.document;
                define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
                let mut drum =
                    Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(450.0, 380.0, 0.0));
                drum.common.layer = "10".to_string();
                let dh = doc.add_entity(document::EntityType::Insert(drum)).unwrap();
                let mut auth = PidAuthor::load(doc, authored_tagged[0]).unwrap();
                auth.type_code = "equipment".to_string();
                auth.tag = "V-9954".to_string();
                auth.tag_source = "manual".to_string();
                auth.tag_status = "ok".to_string();
                auth.props = json!({
                    "_sppid_class": "PIDProcessVessel",
                    "_variant_id": "PIDProcessVessel#01",
                    "_at": "450.0,380.0",
                    "_confidence_tier": "confirmed",
                })
                .to_string();
                auth.store(doc, dh);
            }

            let rev_json_p = out_dir.join(format!("{cid}_rev.json"));
            let rev_csv_p = out_dir.join(format!("{cid}_rev.csv"));
            let _ = app.run_command_line(&format!(
                "PIDDIFF \"{cid}-R0.dxf\" EXPORT \"{}\"",
                rev_json_p.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDDIFF \"{cid}-R0.dxf\" EXPORT \"{}\"",
                rev_csv_p.display()
            ));
            let rev_rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&rev_json_p).unwrap()).unwrap();
            let rev_csv = std::fs::read_to_string(&rev_csv_p).unwrap();
            let _ = std::fs::remove_file(&rev_json_p);
            let _ = std::fs::remove_file(&rev_csv_p);

            assert!(rev_csv.starts_with('\u{feff}') && rev_csv.contains("V-9954"));
            assert!(rev_rep["summary"]["added_count"].as_u64().unwrap_or(0) >= 1);
            assert!(rev_rep["summary"]["removed_count"].as_u64().unwrap_or(0) >= 1);
            assert!(rev_rep["summary"]["modified_count"].as_u64().unwrap_or(0) >= 1);

            app.tabs[tab_ra].scene.deselect_all();
            let _ = app.run_command_line(&format!("PIDDIFF \"{cid}-R0.dxf\" NEXT"));
            assert!(!app.tabs[tab_ra].scene.selected_entities().is_empty());

            rows.push(json!({
                "id": cid,
                "kind": kind,
                "sha256_prefix": sha_pfx,
                "layout_nudge_false_positives": nudge_rep["summary"]["total_changes"],
                "baseline_unchanged_items": baseline_unchanged,
                "revision_diff_summary": {
                    "total_changes": rev_rep["summary"]["total_changes"],
                    "added_count": rev_rep["summary"]["added_count"],
                    "modified_count": rev_rep["summary"]["modified_count"],
                    "removed_count": rev_rep["summary"]["removed_count"],
                    "unchanged_count": rev_rep["summary"]["unchanged_count"],
                }
            }));
        }

        std::fs::write(
            out_dir.join("real154_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "sheets": rows })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidisolate_solves_loto_boundary_valves_across_tabs_and_flags_unvalved_hazards() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-155-01.dxf with target Pump P-1551 at (100, 300)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-155-01.dxf".to_string();
        let (v_suction_h, v_secondary_h, unvalved_pipe_h, hazard_drum_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut tb =
                Text::with_value("DWG-155-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Target Pump P-1551 at (100, 300)
            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1551", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            // Branch A (Suction): (50, 300)..(96, 300) -> First Valve BV155101 at (47, 300) -> (10, 300)..(44, 300) -> Secondary Valve BV155100 at (7, 300)
            let mut ps1 = Line::from_points(
                Vector3::new(50.0, 300.0, 0.0),
                Vector3::new(96.0, 300.0, 0.0),
            );
            ps1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(ps1)).unwrap();

            let mut vs1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(47.0, 300.0, 0.0));
            vs1.common.layer = "11".to_string();
            let v_suction_h = doc.add_entity(document::EntityType::Insert(vs1)).unwrap();
            let mut vst1 =
                Text::with_value("BV155101", Vector3::new(43.0, 306.0, 0.0)).with_height(2.5);
            vst1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vst1)).unwrap();

            let mut ps0 = Line::from_points(
                Vector3::new(10.0, 300.0, 0.0),
                Vector3::new(44.0, 300.0, 0.0),
            );
            ps0.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(ps0)).unwrap();

            let mut vs0 = Insert::new("BALL VALVE TYPE 2", Vector3::new(7.0, 300.0, 0.0));
            vs0.common.layer = "11".to_string();
            let v_secondary_h = doc.add_entity(document::EntityType::Insert(vs0)).unwrap();
            let mut vst0 =
                Text::with_value("BV155100", Vector3::new(3.0, 306.0, 0.0)).with_height(2.5);
            vst0.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vst0)).unwrap();

            // Branch B (Discharge): (104, 300)..(254, 300) -> Paired OPC at (260, 300) -> DWG-155-02
            let mut pd = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            pd.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pd)).unwrap();
            let mut ltxt = Text::with_value("100-PW-1551-CS150", Vector3::new(160.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut tf1 =
                Text::with_value("DWG-155-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            tf1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf1)).unwrap();

            // Branch C (Unvalved Hazard): (100, 304)..(100, 440) directly into Drum V-1559 at (100, 452)
            let mut ph = Line::from_points(
                Vector3::new(100.0, 304.0, 0.0),
                Vector3::new(100.0, 440.0, 0.0),
            );
            ph.common.layer = "1".to_string();
            let unvalved_pipe_h = doc.add_entity(document::EntityType::Line(ph)).unwrap();

            let mut hd = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(100.0, 452.0, 0.0));
            hd.common.layer = "10".to_string();
            let hazard_drum_h = doc.add_entity(document::EntityType::Insert(hd)).unwrap();
            let mut hdt =
                Text::with_value("V-1559", Vector3::new(96.0, 468.0, 0.0)).with_height(2.5);
            hdt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(hdt)).unwrap();

            (v_suction_h, v_secondary_h, unvalved_pipe_h, hazard_drum_h)
        };

        // Tab 2: DWG-155-02.dxf with return OPC -> First Discharge Valve BV155201 -> Downstream Drum V-1552
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-155-02.dxf".to_string();
        let v_disch_tab2_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut tb =
                Text::with_value("DWG-155-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(120.0, 420.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut tf2 =
                Text::with_value("DWG-155-01", Vector3::new(110.0, 419.0, 0.0)).with_height(2.5);
            tf2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(126.0, 420.0, 0.0),
                Vector3::new(220.0, 420.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PW-1551-CS150", Vector3::new(160.0, 423.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut vd = Insert::new("BALL VALVE TYPE 2", Vector3::new(223.0, 420.0, 0.0));
            vd.common.layer = "11".to_string();
            let v_disch_tab2_h = doc.add_entity(document::EntityType::Insert(vd)).unwrap();
            let mut vdt =
                Text::with_value("BV155201", Vector3::new(219.0, 426.0, 0.0)).with_height(2.5);
            vdt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vdt)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(226.0, 420.0, 0.0),
                Vector3::new(340.0, 420.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            let mut d2 = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(346.0, 420.0, 0.0));
            d2.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(d2)).unwrap();
            let mut dt2 =
                Text::with_value("V-1552", Vector3::new(342.0, 435.0, 0.0)).with_height(2.5);
            dt2.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt2)).unwrap();

            v_disch_tab2_h
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidisolate_155_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Solve LOTO isolation boundary for P-1551 across tabs (before adding valve on Branch C)
        let iso1_json = tmp.join("iso1.json");
        let iso1_csv = tmp.join("iso1.csv");
        let _ = app.run_command_line(&format!(
            "PIDISOLATE P-1551 ALLTABS EXPORT \"{}\"",
            iso1_json.display()
        ));
        assert!(
            iso1_json.is_file(),
            "PIDISOLATE EXPORT must write LOTO JSON report"
        );
        let _ = app.run_command_line(&format!(
            "PIDISOLATE P-1551 ALLTABS EXPORT \"{}\"",
            iso1_csv.display()
        ));
        let csv_txt = std::fs::read_to_string(&iso1_csv).unwrap();
        assert!(
            csv_txt.starts_with('\u{feff}')
                && csv_txt.contains("BV155101")
                && csv_txt.contains("BV155201")
        );

        let rep1: Value =
            serde_json::from_str(&std::fs::read_to_string(&iso1_json).unwrap()).unwrap();
        assert_eq!(rep1["summary"]["fully_isolatable"].as_bool(), Some(false));
        assert_eq!(rep1["summary"]["crossed_tabs"].as_u64(), Some(2));
        assert_eq!(rep1["summary"]["boundary_valves_count"].as_u64(), Some(2));
        assert_eq!(rep1["summary"]["unvalved_peer_equipment"].as_u64(), Some(1));

        let v_tags: Vec<&str> = rep1["boundary_valves"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v["valve_tag"].as_str())
            .collect();
        assert!(v_tags.contains(&"BV155101") && v_tags.contains(&"BV155201"));
        assert!(!v_tags.contains(&"BV155100"));

        // 2. Canvas category filters (VALVES & HAZARDS)
        let _ = app.run_command_line("PIDISOLATE P-1551 ALLTABS VALVES");
        assert!(has_sel(&app, tab1, v_suction_h));
        assert!(!has_sel(&app, tab1, v_secondary_h));
        assert!(has_sel(&app, tab2, v_disch_tab2_h));

        let _ = app.run_command_line("PIDISOLATE P-1551 ALLTABS HAZARDS");
        assert!(has_sel(&app, tab1, hazard_drum_h));

        // 3. Remediate Branch C by splitting pipe and inserting isolation valve BV155103, promote both tabs, and verify fully_isolatable == true
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let _ = doc.remove_entity(unvalved_pipe_h);
            let mut pc1 = Line::from_points(
                Vector3::new(100.0, 304.0, 0.0),
                Vector3::new(100.0, 367.0, 0.0),
            );
            pc1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pc1)).unwrap();

            let mut vc = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 370.0, 0.0));
            vc.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(vc)).unwrap();
            let mut vct =
                Text::with_value("BV155103", Vector3::new(105.0, 370.0, 0.0)).with_height(2.5);
            vct.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vct)).unwrap();

            let mut pc2 = Line::from_points(
                Vector3::new(100.0, 373.0, 0.0),
                Vector3::new(100.0, 440.0, 0.0),
            );
            pc2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pc2)).unwrap();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab2));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab1));

        let iso2_json = tmp.join("iso2.json");
        let _ = app.run_command_line(&format!(
            "PIDISOLATE P-1551 ALLTABS EXPORT \"{}\"",
            iso2_json.display()
        ));
        let rep2: Value =
            serde_json::from_str(&std::fs::read_to_string(&iso2_json).unwrap()).unwrap();
        assert_eq!(rep2["summary"]["fully_isolatable"].as_bool(), Some(true));
        assert_eq!(rep2["summary"]["hazard_count"].as_u64(), Some(0));
        assert_eq!(rep2["summary"]["boundary_valves_count"].as_u64(), Some(3));

        // Verify NEXT steps across tabs (switching from Tab 1 to Tab 2 when stepping to BV155201)
        app.tabs[tab1].scene.deselect_all();
        app.tabs[tab2].scene.deselect_all();
        let mut visited_tab2 = false;
        for _ in 0..4 {
            let _ = app.run_command_line("PIDISOLATE P-1551 ALLTABS NEXT");
            if app.active_tab == tab2 {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDISOLATE NEXT must cycle across tabs onto Tab 2 valve BV155201"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDISOLATE ALLTABS equipment LOTO boundary & hazard analysis"]
    fn real_multi_tab_pidisolate_cross_sheet_equipment_loto_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL155_SPPID_PAIR") else {
            println!("OCS_PID_REAL155_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL155_LEGACY_PAIR") else {
            println!("OCS_PID_REAL155_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL155_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real155"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            &sppid_paths[0],
            &sppid_paths[1],
            &legacy_paths[0],
            &legacy_paths[1],
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, p) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].current_path = Some((*p).clone());
            app.tabs[ti].tab_title = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("sheet.dxf")
                .to_string();
            tab_indices.push(ti);
        }

        let pick_best_equip = |app_ref: &OpenCADStudio, tabs: &[usize]| -> (usize, String) {
            for &ti in tabs {
                let bom = crate::io::pid_convert::extract_bom_from_documents(
                    &[crate::io::pid_convert::BomSheetInput {
                        drawing_name: "S",
                        document: &app_ref.tabs[ti].scene.document,
                    }],
                    &crate::io::pid_convert::ConvertOptions::default(),
                )
                .unwrap();
                let mut best: Option<(usize, String)> = None;
                for r in bom.rows.iter().filter(|r| r.category == "Equipment") {
                    let inputs: Vec<_> = tabs
                        .iter()
                        .map(|&t| crate::io::pid_convert::OpcSheetInput {
                            tab_index: t,
                            fallback_name: app_ref.tabs[t].tab_title.as_str(),
                            document: &app_ref.tabs[t].scene.document,
                        })
                        .collect();
                    if let Ok(iso) = crate::io::pid_convert::solve_pid_isolation_by_query(
                        &r.item_tag,
                        ti,
                        true,
                        &inputs,
                    ) {
                        let score = iso.summary.boundary_valves_count * 10
                            + iso.summary.envelope_pipe_segments;
                        if best.as_ref().is_none_or(|(bs, _)| score > *bs) {
                            best = Some((score, r.item_tag.clone()));
                        }
                    }
                }
                if let Some((sc, tag)) = best {
                    if sc > 0 {
                        return (ti, tag);
                    }
                }
                if let Some(first_eq) = bom.rows.iter().find(|r| r.category == "Equipment") {
                    return (ti, first_eq.item_tag.clone());
                }
            }
            panic!("expected at least one equipment item across paired sheets");
        };

        // 1. SPPID paired sheets (Tab 0 / Tab 1)
        let (sp_tab, sp_eq) = pick_best_equip(&app, &[tab_indices[0], tab_indices[1]]);
        let _ = app.update(Message::TabSwitch(sp_tab));
        let sp_json_p = out_dir.join("sp_iso_tmp.json");
        let sp_csv_p = out_dir.join("sp_iso_tmp.csv");
        let _ = app.run_command_line(&format!(
            "PIDISOLATE \"{sp_eq}\" ALLTABS EXPORT \"{}\"",
            sp_json_p.display()
        ));
        let _ = app.run_command_line(&format!(
            "PIDISOLATE \"{sp_eq}\" ALLTABS EXPORT \"{}\"",
            sp_csv_p.display()
        ));
        let sp_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&sp_json_p).unwrap()).unwrap();
        let sp_csv = std::fs::read_to_string(&sp_csv_p).unwrap();
        let _ = std::fs::remove_file(&sp_json_p);
        let _ = std::fs::remove_file(&sp_csv_p);
        assert!(sp_csv.starts_with('\u{feff}'));
        assert!(
            sp_rep["summary"]["boundary_valves_count"]
                .as_u64()
                .unwrap_or(0)
                + sp_rep["summary"]["envelope_pipe_segments"]
                    .as_u64()
                    .unwrap_or(0)
                >= 1
        );
        let _ = app.run_command_line(&format!("PIDISOLATE \"{sp_eq}\" ALLTABS NEXT"));

        // 2. Native AutoCAD legacy paired sheets (Tab 2 / Tab 3)
        let (lg_tab, lg_eq) = pick_best_equip(&app, &[tab_indices[2], tab_indices[3]]);
        let _ = app.update(Message::TabSwitch(lg_tab));
        let lg_json_p = out_dir.join("lg_iso_tmp.json");
        let _ = app.run_command_line(&format!(
            "PIDISOLATE \"{lg_eq}\" ALLTABS EXPORT \"{}\"",
            lg_json_p.display()
        ));
        let lg_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&lg_json_p).unwrap()).unwrap();
        let _ = std::fs::remove_file(&lg_json_p);
        assert!(
            lg_rep["summary"]["boundary_valves_count"]
                .as_u64()
                .unwrap_or(0)
                + lg_rep["summary"]["envelope_pipe_segments"]
                    .as_u64()
                    .unwrap_or(0)
                + lg_rep["summary"]["hazard_count"].as_u64().unwrap_or(0)
                >= 1
        );

        let sanitized = json!({
            "tabs_loaded": 4,
            "sppid_sha256_prefixes": [&sha_prefixes[0], &sha_prefixes[1]],
            "legacy_sha256_prefixes": [&sha_prefixes[2], &sha_prefixes[3]],
            "sppid_equipment_isolation": {
                "fully_isolatable": sp_rep["summary"]["fully_isolatable"],
                "crossed_tabs": sp_rep["summary"]["crossed_tabs"],
                "boundary_valves_count": sp_rep["summary"]["boundary_valves_count"],
                "envelope_pipe_segments": sp_rep["summary"]["envelope_pipe_segments"],
                "hazard_count": sp_rep["summary"]["hazard_count"],
                "unvalved_peer_equipment": sp_rep["summary"]["unvalved_peer_equipment"],
                "unvalved_external_opcs": sp_rep["summary"]["unvalved_external_opcs"],
                "unvalved_open_dead_ends": sp_rep["summary"]["unvalved_open_dead_ends"],
            },
            "legacy_equipment_isolation": {
                "fully_isolatable": lg_rep["summary"]["fully_isolatable"],
                "crossed_tabs": lg_rep["summary"]["crossed_tabs"],
                "boundary_valves_count": lg_rep["summary"]["boundary_valves_count"],
                "envelope_pipe_segments": lg_rep["summary"]["envelope_pipe_segments"],
                "hazard_count": lg_rep["summary"]["hazard_count"],
                "unvalved_peer_equipment": lg_rep["summary"]["unvalved_peer_equipment"],
                "unvalved_external_opcs": lg_rep["summary"]["unvalved_external_opcs"],
                "unvalved_open_dead_ends": lg_rep["summary"]["unvalved_open_dead_ends"],
            },
        });
        std::fs::write(
            out_dir.join("real155_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidloop_aggregates_isa_loops_across_tabs_detects_incomplete_and_steps_camera() {
        use document::entities::{Insert, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-156-01.dxf
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-156-01.dxf".to_string();
        let (pt_h, pic_h, manual_bv_h, lic_bad_h, lv_bad_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "LV-LEVEL CONTROL VALVE", 6.0, 6.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);

            // Manual ball valve BV0201399 -> MUST NOT enter any instrument loop
            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(60.0, 200.0, 0.0));
            bv.common.layer = "11".to_string();
            let manual_bv_h = doc.add_entity(document::EntityType::Insert(bv)).unwrap();
            let mut bvt =
                Text::with_value("BV0201399", Vector3::new(56.0, 206.0, 0.0)).with_height(2.5);
            bvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt)).unwrap();

            // Loop 02013 Sensor: PT-02013 at (110, 200)
            let mut pt_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(110.0, 200.0, 0.0),
            );
            pt_ins.common.layer = "12".to_string();
            let pt_h = doc
                .add_entity(document::EntityType::Insert(pt_ins))
                .unwrap();
            let mut pt_t1 =
                Text::with_value("PT", Vector3::new(108.5, 202.0, 0.0)).with_height(2.2);
            pt_t1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pt_t1)).unwrap();
            let mut pt_t2 =
                Text::with_value("02013", Vector3::new(107.0, 197.5, 0.0)).with_height(2.2);
            pt_t2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pt_t2)).unwrap();

            // Loop 02013 Controller: PIC-02013 at (110, 260)
            let mut pic_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(110.0, 260.0, 0.0),
            );
            pic_ins.common.layer = "12".to_string();
            let pic_h = doc
                .add_entity(document::EntityType::Insert(pic_ins))
                .unwrap();
            let mut pic_t1 =
                Text::with_value("PIC", Vector3::new(108.0, 262.0, 0.0)).with_height(2.2);
            pic_t1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pic_t1)).unwrap();
            let mut pic_t2 =
                Text::with_value("02013", Vector3::new(107.0, 257.5, 0.0)).with_height(2.2);
            pic_t2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pic_t2)).unwrap();

            // Incomplete Loop 02099: LIC-02099 + LV-02099 WITHOUT any LT-02099 primary sensor!
            let mut lic_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(400.0, 300.0, 0.0),
            );
            lic_ins.common.layer = "12".to_string();
            let lic_bad_h = doc
                .add_entity(document::EntityType::Insert(lic_ins))
                .unwrap();
            let mut lic_t1 =
                Text::with_value("LIC", Vector3::new(398.0, 302.0, 0.0)).with_height(2.2);
            lic_t1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lic_t1)).unwrap();
            let mut lic_t2 =
                Text::with_value("02099", Vector3::new(397.0, 297.5, 0.0)).with_height(2.2);
            lic_t2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lic_t2)).unwrap();

            let mut lv_ins = Insert::new("LV-LEVEL CONTROL VALVE", Vector3::new(400.0, 220.0, 0.0));
            lv_ins.common.layer = "12".to_string();
            let lv_bad_h = doc
                .add_entity(document::EntityType::Insert(lv_ins))
                .unwrap();
            let mut lv_t =
                Text::with_value("LV-02099", Vector3::new(395.0, 228.0, 0.0)).with_height(2.5);
            lv_t.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lv_t)).unwrap();

            (pt_h, pic_h, manual_bv_h, lic_bad_h, lv_bad_h)
        };

        // Tab 2: DWG-156-02.dxf with Final Element PV-02013 at (620, 480) completing Loop 02013 across tabs!
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-156-02.dxf".to_string();
        let pv_tab2_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "LV-LEVEL CONTROL VALVE", 6.0, 6.0);
            let mut pv_ins = Insert::new("LV-LEVEL CONTROL VALVE", Vector3::new(620.0, 480.0, 0.0));
            pv_ins.common.layer = "12".to_string();
            let pv_h = doc
                .add_entity(document::EntityType::Insert(pv_ins))
                .unwrap();
            let mut pv_t =
                Text::with_value("PV-02013", Vector3::new(615.0, 488.0, 0.0)).with_height(2.5);
            pv_t.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pv_t)).unwrap();
            pv_h
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidloop_156_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Export multi-tab Loop Index JSON & CSV
        let lp_json = tmp.join("loops1.json");
        let lp_csv = tmp.join("loops1.csv");
        let _ = app.run_command_line(&format!("PIDLOOP ALLTABS EXPORT \"{}\"", lp_json.display()));
        assert!(
            lp_json.is_file(),
            "PIDLOOP EXPORT must write loop index JSON"
        );
        let _ = app.run_command_line(&format!("PIDLOOP ALLTABS EXPORT \"{}\"", lp_csv.display()));
        let csv_raw = std::fs::read_to_string(&lp_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("02013")
                && csv_raw.contains("02099")
        );

        let rep1: Value =
            serde_json::from_str(&std::fs::read_to_string(&lp_json).unwrap()).unwrap();
        assert_eq!(rep1["summary"]["total_loops"].as_u64(), Some(2));
        assert_eq!(rep1["summary"]["complete_closed_loops"].as_u64(), Some(1));
        assert_eq!(rep1["summary"]["incomplete_loops"].as_u64(), Some(1));

        // 2. Highlight specific loop `02013` across tabs & verify manual ball valve excluded
        let _ = app.run_command_line("PIDLOOP 02013 ALLTABS");
        assert!(has_sel(&app, tab1, pt_h));
        assert!(has_sel(&app, tab1, pic_h));
        assert!(!has_sel(&app, tab1, manual_bv_h));
        assert!(!has_sel(&app, tab1, lic_bad_h));
        assert!(has_sel(&app, tab2, pv_tab2_h));

        // 3. Highlight INCOMPLETE loops -> selects LIC-02099 & LV-02099
        let _ = app.run_command_line("PIDLOOP ALLTABS INCOMPLETE");
        assert!(has_sel(&app, tab1, lic_bad_h));
        assert!(has_sel(&app, tab1, lv_bad_h));
        assert!(!has_sel(&app, tab1, pt_h));

        // 4. Stateless NEXT stepping across Loop 02013 jumps onto Tab 2 PV-02013 at (620, 480)
        app.tabs[tab1].scene.deselect_all();
        app.tabs[tab2].scene.deselect_all();
        let mut reached_tab2_pv = false;
        for _ in 0..3 {
            let _ = app.run_command_line("PIDLOOP 02013 ALLTABS NEXT");
            if app.active_tab == tab2 {
                let cam = app.tabs[tab2].scene.camera.borrow().target;
                assert!((cam.x - 620.0).abs() < 30.0 && (cam.y - 480.0).abs() < 30.0);
                reached_tab2_pv = true;
            }
        }
        assert!(
            reached_tab2_pv,
            "PIDLOOP 02013 NEXT must step onto Tab 2 PV-02013"
        );

        // 5. Add missing LT-02099 sensor on Tab 1, promote both tabs, and verify incomplete_loops == 0
        let _ = app.update(Message::TabSwitch(tab1));
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut lt_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(350.0, 300.0, 0.0),
            );
            lt_ins.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lt_ins))
                .unwrap();
            let mut lt_t1 =
                Text::with_value("LT", Vector3::new(348.5, 302.0, 0.0)).with_height(2.2);
            lt_t1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lt_t1)).unwrap();
            let mut lt_t2 =
                Text::with_value("02099", Vector3::new(347.0, 297.5, 0.0)).with_height(2.2);
            lt_t2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lt_t2)).unwrap();
        }
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab2));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab1));

        let lp2_json = tmp.join("loops2.json");
        let _ = app.run_command_line(&format!(
            "PIDLOOP ALLTABS EXPORT \"{}\"",
            lp2_json.display()
        ));
        let rep2: Value =
            serde_json::from_str(&std::fs::read_to_string(&lp2_json).unwrap()).unwrap();
        assert_eq!(rep2["summary"]["incomplete_loops"].as_u64(), Some(0));
        assert_eq!(rep2["summary"]["complete_closed_loops"].as_u64(), Some(2));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDLOOP ALLTABS ISA-5.1 loop index aggregation, completeness check, and NEXT stepping"]
    fn real_multi_tab_pidloop_cross_sheet_isa_aggregation_and_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL156_SPPID_PAIR") else {
            println!("OCS_PID_REAL156_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL156_LEGACY_PAIR") else {
            println!("OCS_PID_REAL156_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL156_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real156"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let run_pair_audit = |label: &str, pair: &[std::path::PathBuf]| -> Value {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in pair.iter().enumerate() {
                shas.push(
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string(),
                );
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].tab_title = format!("{label}_{idx}.dxf");
            }

            let jp = out_dir.join(format!("{label}_loops_tmp.json"));
            let cp = out_dir.join(format!("{label}_loops_tmp.csv"));
            let _ = app.run_command_line(&format!("PIDLOOP ALLTABS EXPORT \"{}\"", jp.display()));
            let _ = app.run_command_line(&format!("PIDLOOP ALLTABS EXPORT \"{}\"", cp.display()));
            let rep: Value = serde_json::from_str(&std::fs::read_to_string(&jp).unwrap()).unwrap();
            let csv_s = std::fs::read_to_string(&cp).unwrap();
            let _ = std::fs::remove_file(&jp);
            let _ = std::fs::remove_file(&cp);

            assert!(csv_s.starts_with('\u{feff}'));
            assert!(
                rep["summary"]["total_loops"].as_u64().unwrap_or(0) >= 5,
                "{label}: expected >=5 instrument loops across paired sheets"
            );

            let _ = app.run_command_line("PIDLOOP ALLTABS NEXT");
            assert!(!app.tabs[app.active_tab]
                .scene
                .selected_entities()
                .is_empty());

            json!({
                "sha256_prefixes": shas,
                "summary": rep["summary"],
            })
        };

        let sp_res = run_pair_audit("SPPID", &sppid_paths);
        let lg_res = run_pair_audit("LEGACY", &legacy_paths);

        std::fs::write(
            out_dir.join("real156_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "sppid_pair": sp_res,
                    "legacy_pair": lg_res,
                }))
                .unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidspec_detects_missing_reducers_spec_breaks_and_opc_mismatches_then_clears_after_fix() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-157-01.dxf
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-157-01.dxf".to_string();
        let (tee_branch_h, red_a_h, red_b_h, sb_c_h, sb_d_h, opc1_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CONCENTRIC REDUCER", 6.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-157-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // 1. Legitimate 3-way reducing tee at (150, 100): 100 + 100 header + 50 branch -> MUST NOT false-flag!
            let mut th1 = Line::from_points(
                Vector3::new(50.0, 100.0, 0.0),
                Vector3::new(150.0, 100.0, 0.0),
            );
            th1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(th1)).unwrap();
            let mut th1_t = Text::with_value("100-PW-1570-CS150", Vector3::new(80.0, 103.0, 0.0))
                .with_height(2.5);
            th1_t.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(th1_t)).unwrap();

            let mut th2 = Line::from_points(
                Vector3::new(150.0, 100.0, 0.0),
                Vector3::new(250.0, 100.0, 0.0),
            );
            th2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(th2)).unwrap();
            let mut th2_t = Text::with_value("100-PW-1570-CS150", Vector3::new(180.0, 103.0, 0.0))
                .with_height(2.5);
            th2_t.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(th2_t)).unwrap();

            let mut tb_ln = Line::from_points(
                Vector3::new(150.0, 100.0, 0.0),
                Vector3::new(150.0, 180.0, 0.0),
            );
            tb_ln.common.layer = "1".to_string();
            let tee_branch_h = doc.add_entity(document::EntityType::Line(tb_ln)).unwrap();
            let mut tb_t = Text::with_value("50-PW-1579-CS150", Vector3::new(153.0, 140.0, 0.0))
                .with_height(2.5);
            tb_t.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tb_t)).unwrap();

            // 2. MissingReducer at (150, 300): DN100 butt-connected 1-to-1 to DN50 with no reducer!
            let mut ra = Line::from_points(
                Vector3::new(50.0, 300.0, 0.0),
                Vector3::new(150.0, 300.0, 0.0),
            );
            ra.common.layer = "1".to_string();
            let red_a_h = doc.add_entity(document::EntityType::Line(ra)).unwrap();
            let mut rat = Text::with_value("100-PW-1571-CS150", Vector3::new(80.0, 303.0, 0.0))
                .with_height(2.5);
            rat.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(rat)).unwrap();

            let mut rb = Line::from_points(
                Vector3::new(150.0, 300.0, 0.0),
                Vector3::new(250.0, 300.0, 0.0),
            );
            rb.common.layer = "1".to_string();
            let red_b_h = doc.add_entity(document::EntityType::Line(rb)).unwrap();
            let mut rbt = Text::with_value("50-PW-1571-CS150", Vector3::new(180.0, 303.0, 0.0))
                .with_height(2.5);
            rbt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(rbt)).unwrap();

            // 3. UnmarkedSpecBreak at (150, 420): CS150 butt-connected to SS300 with no valve/flange!
            let mut sc = Line::from_points(
                Vector3::new(50.0, 420.0, 0.0),
                Vector3::new(150.0, 420.0, 0.0),
            );
            sc.common.layer = "1".to_string();
            let sb_c_h = doc.add_entity(document::EntityType::Line(sc)).unwrap();
            let mut sct = Text::with_value("100-PW-1572-CS150", Vector3::new(80.0, 423.0, 0.0))
                .with_height(2.5);
            sct.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(sct)).unwrap();

            let mut sd = Line::from_points(
                Vector3::new(150.0, 420.0, 0.0),
                Vector3::new(250.0, 420.0, 0.0),
            );
            sd.common.layer = "1".to_string();
            let sb_d_h = doc.add_entity(document::EntityType::Line(sd)).unwrap();
            let mut sdt = Text::with_value("100-PW-1572-SS300", Vector3::new(180.0, 423.0, 0.0))
                .with_height(2.5);
            sdt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(sdt)).unwrap();

            // 4. Paired OPC at (320, 520) -> DWG-157-02 with 100-PW-1573-CS150
            let mut po1 = Line::from_points(
                Vector3::new(220.0, 520.0, 0.0),
                Vector3::new(314.0, 520.0, 0.0),
            );
            po1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(po1)).unwrap();
            let mut pot1 = Text::with_value("100-PW-1573-CS150", Vector3::new(250.0, 523.0, 0.0))
                .with_height(2.5);
            pot1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(pot1)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(320.0, 520.0, 0.0));
            opc1.common.layer = "11".to_string();
            let opc1_h = doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-157-02", Vector3::new(310.0, 519.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();

            (tee_branch_h, red_a_h, red_b_h, sb_c_h, sb_d_h, opc1_h)
        };

        // Tab 2: DWG-157-02.dxf with mismatched peer OPC line `80-PW-1573-SS300`
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-157-02.dxf".to_string();
        let tab2_lbl_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-157-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(120.0, 520.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-157-01", Vector3::new(110.0, 519.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut po2 = Line::from_points(
                Vector3::new(126.0, 520.0, 0.0),
                Vector3::new(230.0, 520.0, 0.0),
            );
            po2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(po2)).unwrap();
            let mut pot2 = Text::with_value("80-PW-1573-SS300", Vector3::new(160.0, 523.0, 0.0))
                .with_height(2.5);
            pot2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(pot2)).unwrap()
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidspec_157_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let sp1_json = tmp.join("spec1.json");
        let sp1_csv = tmp.join("spec1.csv");
        let _ = app.run_command_line(&format!(
            "PIDSPEC ALLTABS EXPORT \"{}\"",
            sp1_json.display()
        ));
        assert!(
            sp1_json.is_file(),
            "PIDSPEC EXPORT must write spec consistency JSON"
        );
        let _ = app.run_command_line(&format!("PIDSPEC ALLTABS EXPORT \"{}\"", sp1_csv.display()));
        let csv_s = std::fs::read_to_string(&sp1_csv).unwrap();
        assert!(csv_s.starts_with('\u{feff}') && csv_s.contains("missing_reducer"));

        let rep1: Value =
            serde_json::from_str(&std::fs::read_to_string(&sp1_json).unwrap()).unwrap();
        assert_eq!(rep1["summary"]["missing_reducers"].as_u64(), Some(1));
        assert_eq!(rep1["summary"]["unmarked_spec_breaks"].as_u64(), Some(1));
        assert_eq!(rep1["summary"]["opc_spec_mismatches"].as_u64(), Some(1));
        assert_eq!(rep1["summary"]["total_findings"].as_u64(), Some(3));

        // Category canvas filters
        let _ = app.run_command_line("PIDSPEC ALLTABS REDUCERS");
        assert!(has_sel(&app, tab1, red_a_h) && has_sel(&app, tab1, red_b_h));
        assert!(!has_sel(&app, tab1, tee_branch_h));

        let _ = app.run_command_line("PIDSPEC ALLTABS SPECBREAKS");
        assert!(has_sel(&app, tab1, sb_c_h) && has_sel(&app, tab1, sb_d_h));

        let _ = app.run_command_line("PIDSPEC ALLTABS OPCMISMATCH");
        assert!(has_sel(&app, tab1, opc1_h));

        // Stateless NEXT stepping focuses camera
        app.tabs[tab1].scene.deselect_all();
        let _ = app.run_command_line("PIDSPEC ALLTABS NEXT");
        assert!(!app.tabs[app.active_tab]
            .scene
            .selected_entities()
            .is_empty());

        // Remediate all 3 findings, promote both tabs, and verify total_findings == 0!
        {
            let doc = &mut app.tabs[tab1].scene.document;
            // Insert CONCENTRIC REDUCER at (150, 300)
            let mut red_sym = Insert::new("CONCENTRIC REDUCER", Vector3::new(150.0, 300.0, 0.0));
            red_sym.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(red_sym))
                .unwrap();

            // Insert BALL VALVE TYPE 2 boundary at (150, 420)
            let mut sb_v = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 420.0, 0.0));
            sb_v.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(sb_v)).unwrap();
        }
        if let Some(document::EntityType::Text(t)) =
            app.tabs[tab2].scene.document.get_entity_mut(tab2_lbl_h)
        {
            t.value = "100-PW-1573-CS150".to_string();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab2));
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.update(Message::TabSwitch(tab1));

        let sp2_json = tmp.join("spec2.json");
        let _ = app.run_command_line(&format!(
            "PIDSPEC ALLTABS EXPORT \"{}\"",
            sp2_json.display()
        ));
        let rep2: Value =
            serde_json::from_str(&std::fs::read_to_string(&sp2_json).unwrap()).unwrap();
        assert_eq!(rep2["summary"]["total_findings"].as_u64(), Some(0));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDSPEC ALLTABS piping spec & diameter consistency audit"]
    fn real_multi_tab_pidspec_cross_sheet_piping_spec_consistency_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL157_SPPID_PAIR") else {
            println!("OCS_PID_REAL157_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL157_LEGACY_PAIR") else {
            println!("OCS_PID_REAL157_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL157_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real157"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let run_pair_spec = |label: &str, pair: &[std::path::PathBuf]| -> Value {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in pair.iter().enumerate() {
                shas.push(
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string(),
                );
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].current_path = Some(p.clone());
                app.tabs[ti].tab_title = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sheet.dxf")
                    .to_string();
            }

            let jp = out_dir.join(format!("{label}_spec_tmp.json"));
            let cp = out_dir.join(format!("{label}_spec_tmp.csv"));
            let _ = app.run_command_line(&format!("PIDSPEC ALLTABS EXPORT \"{}\"", jp.display()));
            let _ = app.run_command_line(&format!("PIDSPEC ALLTABS EXPORT \"{}\"", cp.display()));
            let rep: Value = serde_json::from_str(&std::fs::read_to_string(&jp).unwrap()).unwrap();
            let csv_s = std::fs::read_to_string(&cp).unwrap();
            let _ = std::fs::remove_file(&jp);
            let _ = std::fs::remove_file(&cp);

            assert!(csv_s.starts_with('\u{feff}'));
            assert!(
                rep["summary"]["labeled_pipe_segments"]
                    .as_u64()
                    .unwrap_or(0)
                    >= 10,
                "{label}: expected >=10 labeled pipe segments"
            );

            if rep["summary"]["total_findings"].as_u64().unwrap_or(0) > 0 {
                let _ = app.run_command_line("PIDSPEC ALLTABS NEXT");
                assert!(!app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty());
            }

            json!({
                "sha256_prefixes": shas,
                "summary": rep["summary"],
            })
        };

        let sp_res = run_pair_spec("SPPID", &sppid_paths);
        let lg_res = run_pair_spec("LEGACY", &legacy_paths);

        std::fs::write(
            out_dir.join("real157_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "sppid_pair": sp_res,
                    "legacy_pair": lg_res,
                }))
                .unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidpackage_builds_verified_handover_bundle_blocks_strict_errors_and_hashes_all_artifacts() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let mut app = app();

        // Tab 1: DWG-158-01.dxf (initially contains a duplicate valve tag BV5801!)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-158-01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-158-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Duplicate tag BV5801 on two valves -> DRC Error
            for x in [100.0, 180.0] {
                let mut v = Insert::new("BALL VALVE TYPE 2", Vector3::new(x, 200.0, 0.0));
                v.common.layer = "11".to_string();
                doc.add_entity(document::EntityType::Insert(v)).unwrap();
                let mut vt =
                    Text::with_value("BV5801", Vector3::new(x - 4.0, 206.0, 0.0)).with_height(2.5);
                vt.common.layer = "11".to_string();
                doc.add_entity(document::EntityType::Text(vt)).unwrap();
            }

            // Loop 02013 Sensor PT-02013 + Controller PIC-02013
            let mut pt_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(110.0, 320.0, 0.0),
            );
            pt_ins.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(pt_ins))
                .unwrap();
            let mut pt1 = Text::with_value("PT", Vector3::new(108.5, 322.0, 0.0)).with_height(2.2);
            pt1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pt1)).unwrap();
            let mut pt2 =
                Text::with_value("02013", Vector3::new(107.0, 317.5, 0.0)).with_height(2.2);
            pt2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pt2)).unwrap();

            let mut pic_ins = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(110.0, 360.0, 0.0),
            );
            pic_ins.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(pic_ins))
                .unwrap();
            let mut pic1 =
                Text::with_value("PIC", Vector3::new(108.0, 362.0, 0.0)).with_height(2.2);
            pic1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pic1)).unwrap();
            let mut pic2 =
                Text::with_value("02013", Vector3::new(107.0, 357.5, 0.0)).with_height(2.2);
            pic2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pic2)).unwrap();

            // Paired OPC -> DWG-158-02
            let mut p1 = Line::from_points(
                Vector3::new(210.0, 200.0, 0.0),
                Vector3::new(304.0, 200.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();
            let mut lt1 = Text::with_value("100-PW-5801-CS150", Vector3::new(240.0, 203.0, 0.0))
                .with_height(2.5);
            lt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(lt1)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(310.0, 200.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-158-02", Vector3::new(300.0, 199.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();
        }

        // Tab 2: DWG-158-02.dxf with PV-02013 + Vessel V-5802 + Return OPC -> DWG-158-01
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-158-02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "LV-LEVEL CONTROL VALVE", 6.0, 6.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-158-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(120.0, 200.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-158-01", Vector3::new(110.0, 199.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(126.0, 200.0, 0.0),
                Vector3::new(260.0, 200.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut lt2 = Text::with_value("100-PW-5801-CS150", Vector3::new(160.0, 203.0, 0.0))
                .with_height(2.5);
            lt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(lt2)).unwrap();

            let mut pv = Insert::new("LV-LEVEL CONTROL VALVE", Vector3::new(200.0, 200.0, 0.0));
            pv.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(pv)).unwrap();
            let mut pvt =
                Text::with_value("PV-02013", Vector3::new(195.0, 208.0, 0.0)).with_height(2.5);
            pvt.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pvt)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(266.0, 200.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-5802", Vector3::new(262.0, 215.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidpackage_158_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Run PIDPACKAGE with STRICT while Tab 1 still has DuplicateTag BV5801 -> handover_ready == false
        let bad_dir = tmp.join("pkg_blocked");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS VERIFY STRICT",
            bad_dir.display()
        ));
        let bad_manifest_p = bad_dir.join("handover_manifest.json");
        assert!(
            bad_manifest_p.is_file(),
            "PIDPACKAGE must write handover_manifest.json"
        );
        let bad_man: Value =
            serde_json::from_str(&std::fs::read_to_string(&bad_manifest_p).unwrap()).unwrap();
        assert_eq!(bad_man["handover_ready"].as_bool(), Some(false));
        assert!(bad_man["drc_total_errors"].as_u64().unwrap_or(0) >= 1);

        // 2. Fix DuplicateTag via PIDPROMOTE + PIDREVIEW DEDUP, then build clean verified package
        let _ = app.run_command_line("PIDPROMOTE");
        let _ = app.run_command_line("PIDREVIEW DEDUP");

        let ok_dir = tmp.join("pkg_ready");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS VERIFY STRICT",
            ok_dir.display()
        ));
        let ok_manifest_p = ok_dir.join("handover_manifest.json");
        assert!(ok_manifest_p.is_file());
        let ok_man: Value =
            serde_json::from_str(&std::fs::read_to_string(&ok_manifest_p).unwrap()).unwrap();
        assert_eq!(ok_man["handover_ready"].as_bool(), Some(true));
        assert_eq!(ok_man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(ok_man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(ok_man["sheets_exported"].as_u64(), Some(2));

        // Verify expected sub-artifacts & cryptographic SHA-256 self-consistency
        for rel in [
            "schedules/01_bom_schedule.csv",
            "schedules/01_bom_schedule.json",
            "schedules/02_instrument_loops.csv",
            "schedules/02_instrument_loops.json",
            "audits/03_opc_pairing.json",
            "audits/04_drc_and_spec_audit.json",
        ] {
            assert!(ok_dir.join(rel).is_file(), "missing artifact {rel}");
        }

        let files_arr = ok_man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 10);
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(
                !rel.contains(':') && !rel.contains('\\'),
                "manifest rel_path must be clean POSIX relative path: {rel}"
            );
            let fp = ok_dir.join(rel);
            let raw = std::fs::read(&fp).unwrap();
            assert_eq!(entry["bytes"].as_u64(), Some(raw.len() as u64));
            let actual_sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["sha256"].as_str(), Some(actual_sha.as_str()));
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and builds a full dual-artifact Digital Handover Package via PIDPACKAGE ALLTABS SEED VERIFY STRICT"]
    fn real_multi_tab_pidpackage_dual_handover_bundle_and_sha256_self_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL158_SPPID_PAIR") else {
            println!("OCS_PID_REAL158_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL158_LEGACY_PAIR") else {
            println!("OCS_PID_REAL158_LEGACY_PAIR unset, skipping.");
            return;
        };
        let Ok(seed_spec) = std::env::var("OCS_PID_REAL158_SEED") else {
            println!("OCS_PID_REAL158_SEED unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_REAL158_SYMDIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL158_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real158"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        // Pre-remediate any duplicate tags via PIDPROMOTE + PIDREVIEW DEDUP if present so STRICT passes cleanly
        for &ti in &tab_indices {
            let _ = app.update(Message::TabSwitch(ti));
            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");
        }
        let _ = app.update(Message::TabSwitch(tab_indices[0]));

        let pkg_dir = out_dir.join("pkg_tmp");
        let sym_arg = if sym_dir.trim().is_empty() {
            String::new()
        } else {
            format!(" SYMDIR \"{sym_dir}\"")
        };
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 SEED \"{seed_spec}\"{sym_arg} VERIFY STRICT",
            pkg_dir.display()
        ));

        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(
            man_p.is_file(),
            "PIDPACKAGE must write handover_manifest.json"
        );
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(man["sheets_exported"].as_u64(), Some(4));
        assert_eq!(man["pid_files_written"].as_u64(), Some(4));
        assert_eq!(man["pdf_files_plotted"].as_u64(), Some(4));
        assert!(man["opc_reciprocal_pairs"].as_u64().unwrap_or(0) >= 2);
        assert!(man["bom_unique_rows"].as_u64().unwrap_or(0) >= 50);
        assert!(man["instrument_loops_count"].as_u64().unwrap_or(0) >= 10);

        let files_arr = man["files"].as_array().unwrap();
        let mut total_bytes = 0u64;
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            total_bytes += b;
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
        }

        let sanitized = json!({
            "source_sha256_prefixes": sha_prefixes,
            "handover_ready": man["handover_ready"],
            "strict_vendor_passed": man["strict_vendor_passed"],
            "sheets_exported": man["sheets_exported"],
            "pid_files_written": man["pid_files_written"],
            "pdf_files_plotted": man["pdf_files_plotted"],
            "bom_unique_rows": man["bom_unique_rows"],
            "instrument_loops_count": man["instrument_loops_count"],
            "opc_reciprocal_pairs": man["opc_reciprocal_pairs"],
            "drc_total_errors": man["drc_total_errors"],
            "drc_total_warnings": man["drc_total_warnings"],
            "spec_total_findings": man["spec_total_findings"],
            "total_package_files": files_arr.len(),
            "total_package_bytes": total_bytes,
            "manifest_sha256": man["manifest_sha256"],
        });

        let _ = std::fs::remove_dir_all(&pkg_dir);
        std::fs::write(
            out_dir.join("real158_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidrenum_dryruns_blocks_conflicts_updates_single_two_line_and_pipe_texts_and_undoes() {
        use document::entities::{Insert, Text};
        use document::types::Vector3;

        let mut app = app();
        let tab = app.active_tab;
        app.tabs[tab].tab_title = "DWG-159-01.dxf".to_string();
        let (valve_h, valve_txt_h, inst_h, inst_top_h, inst_bot_h, _) =
            seed_dxf_sheet_in_app(&mut app);

        // Add an existing BV0301501A valve to test pre-flight collision blocking
        {
            let doc = &mut app.tabs[tab].scene.document;
            let mut v3 = Insert::new("BALL VALVE TYPE 2", Vector3::new(320.0, 260.0, 0.0));
            v3.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(v3)).unwrap();
            let mut vt3 =
                Text::with_value("BV0301501A", Vector3::new(316.0, 266.0, 0.0)).with_height(2.5);
            vt3.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt3)).unwrap();
        }

        let _ = app.run_command_line("PIDPROMOTE");

        let tmp = std::env::temp_dir().join(format!("ocs_pidrenum_159_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Pre-flight conflict check: selecting only BV0201501A and renaming 0201 -> 0301 collides with existing BV0301501A
        app.tabs[tab].scene.deselect_all();
        app.tabs[tab].scene.select_entities(&[valve_h]);
        let conf_json = tmp.join("conflict.json");
        let _ = app.run_command_line(&format!(
            "PIDRENUM 0201 0301 SELECTED EXPORT \"{}\"",
            conf_json.display()
        ));
        assert!(
            conf_json.is_file(),
            "PIDRENUM EXPORT must write report JSON"
        );
        let conf_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&conf_json).unwrap()).unwrap();
        assert_eq!(conf_rep["summary"]["applied"].as_bool(), Some(false));
        assert!(conf_rep["summary"]["conflict_count"].as_u64().unwrap_or(0) >= 1);
        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, valve_h)
                .unwrap()
                .tag,
            "BV0201501A"
        );

        // 2. DRYRUN 0201 -> 0401 previews symbol + pipeline renames without mutating canvas
        let dry_json = tmp.join("dry.json");
        let _ = app.run_command_line(&format!(
            "PIDRENUM 0201 0401 DRYRUN EXPORT \"{}\"",
            dry_json.display()
        ));
        let dry_rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&dry_json).unwrap()).unwrap();
        assert_eq!(dry_rep["summary"]["dry_run"].as_bool(), Some(true));
        assert_eq!(dry_rep["summary"]["applied"].as_bool(), Some(false));
        assert_eq!(dry_rep["summary"]["symbols_renamed"].as_u64(), Some(2));
        assert!(
            dry_rep["summary"]["pipelines_renamed"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );
        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, valve_h)
                .unwrap()
                .tag,
            "BV0201501A"
        );

        // 3. Live apply 0201 -> 0401 updates PidAuthor, single-line Text, 2-line bubble Text, and pipeline label
        let live_csv = tmp.join("live.csv");
        let _ = app.run_command_line(&format!(
            "PIDRENUM 0201 0401 EXPORT \"{}\"",
            live_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&live_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("BV0401501A")
                && csv_raw.contains("PT-04013")
        );

        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, valve_h)
                .unwrap()
                .tag,
            "BV0401501A"
        );
        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, inst_h)
                .unwrap()
                .tag,
            "PT-04013"
        );
        if let Some(document::EntityType::Text(t)) =
            app.tabs[tab].scene.document.get_entity(valve_txt_h)
        {
            assert_eq!(t.value, "BV0401501A");
        } else {
            panic!("expected single-line valve text");
        }
        if let Some(document::EntityType::Text(top)) =
            app.tabs[tab].scene.document.get_entity(inst_top_h)
        {
            assert_eq!(top.value, "PT");
        }
        if let Some(document::EntityType::Text(bot)) =
            app.tabs[tab].scene.document.get_entity(inst_bot_h)
        {
            assert_eq!(bot.value, "04013");
        } else {
            panic!("expected 2-line bottom instrument text");
        }

        // Renamed pipeline 50-PG-04011-B9- is discoverable via PIDFIND
        app.tabs[tab].scene.deselect_all();
        let _ = app.run_command_line("PIDFIND 04011");
        assert!(!app.tabs[tab].scene.selected_entities().is_empty());

        // 4. Single-step UNDO reverts all renumbered symbols, canvas texts, and pipelines atomically
        let undone = request(&mut app, json!({"op": "undo"}));
        assert_eq!(undone["ok"], true);
        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, valve_h)
                .unwrap()
                .tag,
            "BV0201501A"
        );
        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, inst_h)
                .unwrap()
                .tag,
            "PT-02013"
        );
        if let Some(document::EntityType::Text(t)) =
            app.tabs[tab].scene.document.get_entity(valve_txt_h)
        {
            assert_eq!(t.value, "BV0201501A");
        }
        if let Some(document::EntityType::Text(bot)) =
            app.tabs[tab].scene.document.get_entity(inst_bot_h)
        {
            assert_eq!(bot.value, "02013");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "runs real SPPID and native AutoCAD sheets inside OpenCADStudio through PIDRENUM train cloning, StrictVendor XML/dual export verification, and single-step UNDO rollback"]
    fn real_multi_source_pidrenum_train_clone_export_and_undo() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL159_SPPID_DXF") else {
            println!("OCS_PID_REAL159_SPPID_DXF unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL159_LEGACY_DXF") else {
            println!("OCS_PID_REAL159_LEGACY_DXF unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL159_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real159"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let cases = [
            ("SP01", "sppid_dxf", std::path::PathBuf::from(&sppid_raw)),
            (
                "LG01",
                "legacy_autocad_dxf",
                std::path::PathBuf::from(&legacy_raw),
            ),
        ];

        let mut rows = Vec::new();
        for (cid, kind, path) in &cases {
            let sha_pfx =
                format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))[..12].to_string();
            let mut app = app();
            let tab = app.active_tab;
            app.tabs[tab].scene.document = crate::io::load_file(path).unwrap();
            app.tabs[tab].tab_title = format!("{cid}.dxf");

            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");

            // Discover most frequent 2-digit numeric prefix in tags on this sheet so Train clone renumbers a realistic batch
            let mut sub_freq: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for ent in app.tabs[tab].scene.document.model_space_entities() {
                if let Some(a) = PidAuthor::load(&app.tabs[tab].scene.document, ent.common().handle)
                {
                    let digits: String = a.tag.chars().filter(|c| c.is_ascii_digit()).collect();
                    if digits.len() >= 2 {
                        *sub_freq.entry(digits[..2].to_string()).or_default() += 1;
                    }
                }
            }
            let from_code = sub_freq
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(k, _)| k)
                .unwrap_or_else(|| "01".to_string());
            let existing_tags: Vec<String> = app.tabs[tab]
                .scene
                .document
                .model_space_entities()
                .filter_map(|e| {
                    PidAuthor::load(&app.tabs[tab].scene.document, e.common().handle).map(|a| a.tag)
                })
                .collect();
            let to_code = ["98", "97", "96", "89", "79", "987"]
                .into_iter()
                .find(|cand| *cand != from_code && !existing_tags.iter().any(|t| t.contains(cand)))
                .unwrap_or("9876");

            let sample_before: Vec<(document::Handle, String)> = app.tabs[tab]
                .scene
                .document
                .model_space_entities()
                .filter_map(|e| {
                    let h = e.common().handle;
                    let a = PidAuthor::load(&app.tabs[tab].scene.document, h)?;
                    (!a.tag.is_empty()).then_some((h, a.tag))
                })
                .take(10)
                .collect();

            let rn_json = out_dir.join(format!("{cid}_renum.json"));
            let rn_csv = out_dir.join(format!("{cid}_renum.csv"));
            let _ = app.run_command_line(&format!(
                "PIDRENUM \"{from_code}\" \"{to_code}\" EXPORT \"{}\"",
                rn_json.display()
            ));
            let _ = app.run_command_line(&format!("PIDBOM \"{}\"", rn_csv.display()));
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&rn_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&rn_json);
            let _ = std::fs::remove_file(&rn_csv);

            assert_eq!(rep["summary"]["applied"].as_bool(), Some(true));
            assert!(rep["summary"]["symbols_renamed"].as_u64().unwrap_or(0) >= 5);

            // Verify renumbered sheet exports clean StrictVendor XML
            let exp_dir = out_dir.join(format!("{cid}_exp"));
            let _ = app.run_command_line(&format!(
                "PIDCONVERT \"{}\" STEM {cid} PLANT P01 VERIFY",
                exp_dir.display()
            ));
            assert!(exp_dir.join(format!("{cid}_Data.xml")).is_file());
            let _ = std::fs::remove_dir_all(&exp_dir);

            // Verify single-step UNDO restores original tags
            let undone = request(&mut app, json!({"op": "undo"}));
            assert_eq!(undone["ok"], true);
            for (h, orig_tag) in &sample_before {
                let cur = PidAuthor::load(&app.tabs[tab].scene.document, *h)
                    .unwrap()
                    .tag;
                assert_eq!(&cur, orig_tag);
            }

            rows.push(json!({
                "id": cid,
                "kind": kind,
                "sha256_prefix": sha_pfx,
                "symbols_renamed": rep["summary"]["symbols_renamed"],
                "pipelines_renamed": rep["summary"]["pipelines_renamed"],
                "canvas_texts_synced": rep["summary"]["canvas_texts_synced"],
                "strict_vendor_export_verified": true,
                "single_step_undo_verified": true,
            }));
        }

        std::fs::write(
            out_dir.join("real159_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "sheets": rows })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidhazop_partitions_multi_tab_equipment_nodes_links_safeguards_flags_unprotected_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-160-01.dxf with protected Pump P-1601 (crosses OPC to Tab 2) and unprotected Drum V-1609
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-160-01.dxf".to_string();
        let (pump_h, unprot_drum_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut tb =
                Text::with_value("DWG-160-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Pump P-1601 at (100, 300)
            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            let pump_h = doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1601", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            // Discharge line 100-PW-1601-CS150: (104, 300)..(147, 300) -> BV160101 -> (153, 300)..(254, 300) -> OPC
            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(147.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut v1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 300.0, 0.0));
            v1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(v1)).unwrap();
            let mut vt1 =
                Text::with_value("BV160101", Vector3::new(146.0, 306.0, 0.0)).with_height(2.5);
            vt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vt1)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(153.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PW-1601-CS150", Vector3::new(195.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            // Pressure transmitter PT-1601 tapped onto discharge line at (185, 308)
            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(185.0, 308.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut itop = Text::with_value("PT", Vector3::new(183.5, 310.0, 0.0)).with_height(2.2);
            itop.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(itop)).unwrap();
            let mut ibot =
                Text::with_value("1601", Vector3::new(182.5, 305.5, 0.0)).with_height(2.2);
            ibot.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ibot)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut tf1 =
                Text::with_value("DWG-160-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            tf1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf1)).unwrap();

            // Unprotected Drum V-1609 at (100, 480) with process pipe 80-CW-1609-CS150 and ZERO safeguards
            let mut ud = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(100.0, 480.0, 0.0));
            ud.common.layer = "10".to_string();
            let unprot_drum_h = doc.add_entity(document::EntityType::Insert(ud)).unwrap();
            let mut udt =
                Text::with_value("V-1609", Vector3::new(96.0, 496.0, 0.0)).with_height(2.5);
            udt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(udt)).unwrap();

            let mut upipe = Line::from_points(
                Vector3::new(108.0, 480.0, 0.0),
                Vector3::new(210.0, 480.0, 0.0),
            );
            upipe.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(upipe)).unwrap();
            let mut ulbl = Text::with_value("80-CW-1609-CS150", Vector3::new(145.0, 483.0, 0.0))
                .with_height(2.5);
            ulbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ulbl)).unwrap();

            (pump_h, unprot_drum_h)
        };

        // Tab 2: DWG-160-02.dxf with return OPC -> PIC-1601 + BV160201 -> Downstream Drum V-1602 + LT-1602
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-160-02.dxf".to_string();
        let (v_tab2_h, drum2_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);

            let mut tb =
                Text::with_value("DWG-160-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(120.0, 420.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut tf2 =
                Text::with_value("DWG-160-01", Vector3::new(110.0, 419.0, 0.0)).with_height(2.5);
            tf2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tf2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(126.0, 420.0, 0.0),
                Vector3::new(217.0, 420.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PW-1601-CS150", Vector3::new(155.0, 423.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            // Remote DCS controller PIC-1601 on Tab 2 linked via ISA-5.1 loop 1601
            let mut pic = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(180.0, 460.0, 0.0),
            );
            pic.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(pic)).unwrap();
            let mut ptop =
                Text::with_value("PIC", Vector3::new(178.0, 462.0, 0.0)).with_height(2.2);
            ptop.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ptop)).unwrap();
            let mut pbot =
                Text::with_value("1601", Vector3::new(177.5, 457.5, 0.0)).with_height(2.2);
            pbot.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pbot)).unwrap();

            let mut vd = Insert::new("BALL VALVE TYPE 2", Vector3::new(220.0, 420.0, 0.0));
            vd.common.layer = "11".to_string();
            let v_tab2_h = doc.add_entity(document::EntityType::Insert(vd)).unwrap();
            let mut vdt =
                Text::with_value("BV160201", Vector3::new(216.0, 426.0, 0.0)).with_height(2.5);
            vdt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(vdt)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(223.0, 420.0, 0.0),
                Vector3::new(266.0, 420.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            // Downstream Drum V-1602 at (274, 420) with its own level transmitter LT-1602 at (284, 420)
            let mut d2 = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(274.0, 420.0, 0.0));
            d2.common.layer = "10".to_string();
            let drum2_h = doc.add_entity(document::EntityType::Insert(d2)).unwrap();
            let mut d2t =
                Text::with_value("V-1602", Vector3::new(270.0, 436.0, 0.0)).with_height(2.5);
            d2t.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(d2t)).unwrap();

            let mut lt = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(286.0, 420.0, 0.0),
            );
            lt.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lt)).unwrap();
            let mut ltop = Text::with_value("LT", Vector3::new(284.5, 422.0, 0.0)).with_height(2.2);
            ltop.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ltop)).unwrap();
            let mut lbot =
                Text::with_value("1602", Vector3::new(283.5, 417.5, 0.0)).with_height(2.2);
            lbot.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lbot)).unwrap();

            (v_tab2_h, drum2_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidhazop_160_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let hz_json = tmp.join("hazop.json");
        let hz_csv = tmp.join("hazop.csv");
        let _ = app.run_command_line(&format!(
            "PIDHAZOP ALLTABS EXPORT \"{}\"",
            hz_json.display()
        ));
        assert!(hz_json.is_file(), "PIDHAZOP EXPORT must write hazop.json");
        let _ = app.run_command_line(&format!(
            "PIDHAZOP SUMMARY ALLTABS EXPORT \"{}\"",
            hz_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&hz_csv).unwrap();
        assert!(csv_raw.starts_with('\u{feff}') && csv_raw.contains("HAZOP-01"));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&hz_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_nodes"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["protected_nodes"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["unprotected_nodes"].as_u64(), Some(1));
        assert!(rep["summary"]["multi_sheet_nodes"].as_u64().unwrap_or(0) >= 1);

        let nodes = rep["nodes"].as_array().unwrap();
        let p_node = nodes
            .iter()
            .find(|n| n["equipment_tag"].as_str() == Some("P-1601"))
            .expect("must have P-1601 HAZOP node");
        assert_eq!(p_node["status"].as_str(), Some("protected"));
        let p_spans: Vec<&str> = p_node["span_drawings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(p_spans.contains(&"DWG-160-01") && p_spans.contains(&"DWG-160-02"));
        let p_insts: Vec<&str> = p_node["safeguard_instruments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(p_insts.contains(&"PT-1601") && p_insts.contains(&"PIC-1601"));
        assert!(
            !p_insts.contains(&"LT-1602"),
            "peer equipment boundary must isolate V-1602's LT-1602 from P-1601's node"
        );

        let u_node = nodes
            .iter()
            .find(|n| n["equipment_tag"].as_str() == Some("V-1609"))
            .expect("must have V-1609 HAZOP node");
        assert_eq!(u_node["status"].as_str(), Some("unprotected"));

        // Highlight UNPROTECTED selects V-1609 and NOT P-1601
        let _ = app.run_command_line("PIDHAZOP UNPROTECTED ALLTABS");
        assert!(has_sel(&app, tab1, unprot_drum_h));
        assert!(!has_sel(&app, tab1, pump_h));

        // Highlight NODE P-1601 ALLTABS selects P-1601 on Tab 1 and BV160201 across paired OPC on Tab 2
        let _ = app.run_command_line("PIDHAZOP NODE P-1601 ALLTABS");
        assert!(has_sel(&app, tab1, pump_h));
        assert!(has_sel(&app, tab2, v_tab2_h));

        // Stateless NEXT steps across all 3 HAZOP nodes and switches tab to Tab 2 for V-1602
        let mut visited_tab2 = false;
        for _ in 0..3 {
            let _ = app.run_command_line("PIDHAZOP NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, drum2_h) {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDHAZOP NEXT ALLTABS must focus V-1602 on Tab 2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDHAZOP ALLTABS automated HAZOP node partitioning, safeguard loop linkage, and NEXT stepping"]
    fn real_multi_tab_pidhazop_cross_sheet_node_partition_and_safeguard_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL160_SPPID_PAIR") else {
            println!("OCS_PID_REAL160_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL160_LEGACY_PAIR") else {
            println!("OCS_PID_REAL160_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL160_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real160"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let hz_json = out_dir.join(format!("{gid}_hazop.json"));
            let hz_csv = out_dir.join(format!("{gid}_hazop.csv"));
            let _ = app.run_command_line(&format!(
                "PIDHAZOP ALLTABS EXPORT \"{}\"",
                hz_json.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDHAZOP SUMMARY ALLTABS EXPORT \"{}\"",
                hz_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&hz_csv).unwrap();
            assert!(csv_txt.starts_with('\u{feff}') && csv_txt.contains("NodeId,EquipmentTag"));
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&hz_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&hz_json);
            let _ = std::fs::remove_file(&hz_csv);

            let total_nodes = rep["summary"]["total_nodes"].as_u64().unwrap_or(0);
            assert!(
                total_nodes >= 1,
                "{gid} must partition >= 1 equipment HAZOP node"
            );

            // Step through first 2 nodes via PIDHAZOP NEXT ALLTABS
            let _ = app.run_command_line("PIDHAZOP NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDHAZOP NEXT ALLTABS must select target HAZOP node envelope"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "total_nodes": rep["summary"]["total_nodes"],
                "multi_sheet_nodes": rep["summary"]["multi_sheet_nodes"],
                "protected_nodes": rep["summary"]["protected_nodes"],
                "unprotected_nodes": rep["summary"]["unprotected_nodes"],
                "total_linked_pipelines": rep["summary"]["total_linked_pipelines"],
                "total_linked_safeguards": rep["summary"]["total_linked_safeguards"],
            }));
        }

        std::fs::write(
            out_dir.join("real160_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidutility_classifies_utility_services_extracts_battery_limits_with_boundary_valves_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-161-01.dxf with OSBL Nitrogen utility header (50-N2-16101-CS150 + BV16101) and internal paired PG process line
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-161-01.dxf".to_string();
        let (n2_pipe_h, pg_pipe_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-161-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // External Battery-Limit OPC DWG-OSBL-901 -> (46..97, 200) -> BV16101 -> (103..180, 200) labeled 50-N2-16101-CS150
            let mut bl1 = Insert::new("_OFF-UNIT SQ", Vector3::new(40.0, 200.0, 0.0));
            bl1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bl1)).unwrap();
            let mut blt1 =
                Text::with_value("DWG-OSBL-901", Vector3::new(30.0, 199.0, 0.0)).with_height(2.5);
            blt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(blt1)).unwrap();

            let mut np1 = Line::from_points(
                Vector3::new(46.0, 200.0, 0.0),
                Vector3::new(97.0, 200.0, 0.0),
            );
            np1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(np1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 200.0, 0.0));
            bv1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV16101", Vector3::new(96.0, 206.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();

            let mut np2 = Line::from_points(
                Vector3::new(103.0, 200.0, 0.0),
                Vector3::new(185.0, 200.0, 0.0),
            );
            np2.common.layer = "1".to_string();
            let n2_pipe_h = doc.add_entity(document::EntityType::Line(np2)).unwrap();
            let mut nlbl = Text::with_value("50-N2-16101-CS150", Vector3::new(130.0, 203.0, 0.0))
                .with_height(2.5);
            nlbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(nlbl)).unwrap();

            // Internal paired process line 100-PG-16102-CS150 -> DWG-161-02 (must NOT be counted as battery limit)
            let mut pp = Line::from_points(
                Vector3::new(80.0, 320.0, 0.0),
                Vector3::new(254.0, 320.0, 0.0),
            );
            pp.common.layer = "1".to_string();
            let pg_pipe_h = doc.add_entity(document::EntityType::Line(pp)).unwrap();
            let mut plbl = Text::with_value("100-PG-16102-CS150", Vector3::new(140.0, 323.0, 0.0))
                .with_height(2.5);
            plbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(plbl)).unwrap();

            let mut opc_in1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 320.0, 0.0));
            opc_in1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc_in1))
                .unwrap();
            let mut opct1 =
                Text::with_value("DWG-161-02", Vector3::new(250.0, 319.0, 0.0)).with_height(2.5);
            opct1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(opct1)).unwrap();

            (n2_pipe_h, pg_pipe_h)
        };

        // Tab 2: DWG-161-02.dxf with paired PG continuation + OSBL Low-Pressure Steam header (80-LS-16103-CS300 + BV16103)
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-161-02.dxf".to_string();
        let (ls_pipe_h, bl_ls_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-161-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc_in2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 320.0, 0.0));
            opc_in2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc_in2))
                .unwrap();
            let mut opct2 =
                Text::with_value("DWG-161-01", Vector3::new(90.0, 319.0, 0.0)).with_height(2.5);
            opct2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(opct2)).unwrap();

            let mut pp2 = Line::from_points(
                Vector3::new(106.0, 320.0, 0.0),
                Vector3::new(200.0, 320.0, 0.0),
            );
            pp2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pp2)).unwrap();
            let mut plbl2 = Text::with_value("100-PG-16102-CS150", Vector3::new(140.0, 323.0, 0.0))
                .with_height(2.5);
            plbl2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(plbl2)).unwrap();

            // OSBL Steam header 80-LS-16103-CS300 -> BV16103 -> DWG-OSBL-902
            let mut sp1 = Line::from_points(
                Vector3::new(120.0, 460.0, 0.0),
                Vector3::new(197.0, 460.0, 0.0),
            );
            sp1.common.layer = "1".to_string();
            let ls_pipe_h = doc.add_entity(document::EntityType::Line(sp1)).unwrap();
            let mut slbl = Text::with_value("80-LS-16103-CS300", Vector3::new(145.0, 463.0, 0.0))
                .with_height(2.5);
            slbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(slbl)).unwrap();

            let mut bv2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(200.0, 460.0, 0.0));
            bv2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv2)).unwrap();
            let mut bvt2 =
                Text::with_value("BV16103", Vector3::new(196.0, 466.0, 0.0)).with_height(2.5);
            bvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt2)).unwrap();

            let mut sp2 = Line::from_points(
                Vector3::new(203.0, 460.0, 0.0),
                Vector3::new(254.0, 460.0, 0.0),
            );
            sp2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(sp2)).unwrap();

            let mut bl2 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 460.0, 0.0));
            bl2.common.layer = "11".to_string();
            let bl_ls_h = doc.add_entity(document::EntityType::Insert(bl2)).unwrap();
            let mut blt2 =
                Text::with_value("DWG-OSBL-902", Vector3::new(250.0, 459.0, 0.0)).with_height(2.5);
            blt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(blt2)).unwrap();

            (ls_pipe_h, bl_ls_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidutility_161_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let ut_json = tmp.join("utility.json");
        let ut_csv = tmp.join("utility.csv");
        let _ = app.run_command_line(&format!(
            "PIDUTILITY ALLTABS EXPORT \"{}\"",
            ut_json.display()
        ));
        assert!(
            ut_json.is_file(),
            "PIDUTILITY EXPORT must write utility.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDUTILITY SUMMARY ALLTABS EXPORT \"{}\"",
            ut_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&ut_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("SERVICE_GROUP")
                && csv_raw.contains("BATTERY_LIMIT")
        );

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&ut_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["utility_services_count"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["process_services_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["battery_limit_count"].as_u64(), Some(2));

        let bls = rep["battery_limits"].as_array().unwrap();
        let bl_n2 = bls
            .iter()
            .find(|b| b["fluid_code"].as_str() == Some("N2"))
            .expect("must have N2 battery limit");
        assert_eq!(bl_n2["nominal_diameter"].as_str(), Some("50 mm"));
        assert_eq!(bl_n2["piping_spec"].as_str(), Some("CS150"));
        assert_eq!(bl_n2["nearest_valve_or_meter"].as_str(), Some("BV16101"));

        let bl_ls = bls
            .iter()
            .find(|b| b["fluid_code"].as_str() == Some("LS"))
            .expect("must have LS battery limit");
        assert_eq!(bl_ls["nominal_diameter"].as_str(), Some("80 mm"));
        assert_eq!(bl_ls["piping_spec"].as_str(), Some("CS300"));
        assert_eq!(bl_ls["nearest_valve_or_meter"].as_str(), Some("BV16103"));

        // PIDUTILITY UTILITIES ALLTABS selects utility lines (N2 + LS) and excludes process line PG
        let _ = app.run_command_line("PIDUTILITY UTILITIES ALLTABS");
        assert!(has_sel(&app, tab1, n2_pipe_h));
        assert!(has_sel(&app, tab2, ls_pipe_h));
        assert!(!has_sel(&app, tab1, pg_pipe_h));

        // PIDUTILITY SERVICE N2 ALLTABS selects only N2
        let _ = app.run_command_line("PIDUTILITY SERVICE N2 ALLTABS");
        assert!(has_sel(&app, tab1, n2_pipe_h));
        assert!(!has_sel(&app, tab2, ls_pipe_h));

        // Stateless NEXT steps across battery-limit interfaces and switches to Tab 2
        let mut visited_tab2_bl = false;
        for _ in 0..2 {
            let _ = app.run_command_line("PIDUTILITY NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, bl_ls_h) {
                visited_tab2_bl = true;
            }
        }
        assert!(
            visited_tab2_bl,
            "PIDUTILITY NEXT ALLTABS must step to LS battery-limit interface on Tab 2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDUTILITY ALLTABS utility service classification, battery-limit tie-in extraction, and NEXT stepping"]
    fn real_multi_tab_pidutility_services_and_battery_limit_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL161_SPPID_PAIR") else {
            println!("OCS_PID_REAL161_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL161_LEGACY_PAIR") else {
            println!("OCS_PID_REAL161_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL161_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real161"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let ut_json = out_dir.join(format!("{gid}_utility.json"));
            let ut_csv = out_dir.join(format!("{gid}_utility.csv"));
            let _ = app.run_command_line(&format!(
                "PIDUTILITY ALLTABS EXPORT \"{}\"",
                ut_json.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDUTILITY SUMMARY ALLTABS EXPORT \"{}\"",
                ut_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&ut_csv).unwrap();
            assert!(csv_txt.starts_with('\u{feff}') && csv_txt.contains("RecordKind,IdOrService"));
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&ut_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&ut_json);
            let _ = std::fs::remove_file(&ut_csv);

            assert!(
                rep["summary"]["total_services"].as_u64().unwrap_or(0) >= 1,
                "{gid} must classify >= 1 fluid service"
            );
            assert!(
                rep["summary"]["battery_limit_count"].as_u64().unwrap_or(0) >= 1,
                "{gid} must extract >= 1 external battery-limit interface"
            );

            let _ = app.run_command_line("PIDUTILITY NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDUTILITY NEXT ALLTABS must select battery-limit interface entities"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "total_services": rep["summary"]["total_services"],
                "utility_services_count": rep["summary"]["utility_services_count"],
                "process_services_count": rep["summary"]["process_services_count"],
                "labeled_pipelines_count": rep["summary"]["labeled_pipelines_count"],
                "utility_pipelines_count": rep["summary"]["utility_pipelines_count"],
                "battery_limit_count": rep["summary"]["battery_limit_count"],
                "utility_battery_limits": rep["summary"]["utility_battery_limits"],
            }));
        }

        std::fs::write(
            out_dir.join("real161_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidvalve_inherits_pipe_specs_rolls_up_mto_flags_unresolved_and_steps_camera() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-162-01.dxf with BV16201A + FV16201 on 100-PG-16201-CS150, a non-valve reducer, and unresolved GV16299
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-162-01.dxf".to_string();
        let (bv1_h, fv_h, gv_unres_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "GATE VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "CONCENTRIC REDUCER", 5.0, 4.0);

            let mut tb =
                Text::with_value("DWG-162-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(40.0, 200.0, 0.0),
                Vector3::new(97.0, 200.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 200.0, 0.0));
            bv1.common.layer = "11".to_string();
            let bv1_h = doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV16201A", Vector3::new(95.0, 206.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(103.0, 200.0, 0.0),
                Vector3::new(177.0, 200.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PG-16201-CS150", Vector3::new(125.0, 203.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut fv = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 200.0, 0.0));
            fv.common.layer = "11".to_string();
            let fv_h = doc.add_entity(document::EntityType::Insert(fv)).unwrap();
            let mut fvt =
                Text::with_value("FV16201", Vector3::new(176.0, 206.0, 0.0)).with_height(2.5);
            fvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(fvt)).unwrap();

            let mut p3 = Line::from_points(
                Vector3::new(183.0, 200.0, 0.0),
                Vector3::new(247.0, 200.0, 0.0),
            );
            p3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p3)).unwrap();

            // Non-valve fitting must be excluded from Valve MTO
            let mut red = Insert::new("CONCENTRIC REDUCER", Vector3::new(250.0, 200.0, 0.0));
            red.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(red)).unwrap();

            // Unlabeled branch with Gate Valve GV16299 -> unresolved spec
            let mut up = Line::from_points(
                Vector3::new(60.0, 360.0, 0.0),
                Vector3::new(117.0, 360.0, 0.0),
            );
            up.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(up)).unwrap();
            let mut gv = Insert::new("GATE VALVE TYPE 2", Vector3::new(120.0, 360.0, 0.0));
            gv.common.layer = "11".to_string();
            let gv_unres_h = doc.add_entity(document::EntityType::Insert(gv)).unwrap();
            let mut gvt =
                Text::with_value("GV16299", Vector3::new(116.0, 366.0, 0.0)).with_height(2.5);
            gvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(gvt)).unwrap();

            (bv1_h, fv_h, gv_unres_h)
        };

        // Tab 2: DWG-162-02.dxf with matching 100 mm CS150 Ball Valve BV16202A (rolls up to Qty=2!)
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-162-02.dxf".to_string();
        let bv2_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);

            let mut tb =
                Text::with_value("DWG-162-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(80.0, 260.0, 0.0),
                Vector3::new(137.0, 260.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-16202-CS150", Vector3::new(95.0, 263.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut bv2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(140.0, 260.0, 0.0));
            bv2.common.layer = "11".to_string();
            let bv2_h = doc.add_entity(document::EntityType::Insert(bv2)).unwrap();
            let mut bvt2 =
                Text::with_value("BV16202A", Vector3::new(135.0, 266.0, 0.0)).with_height(2.5);
            bvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt2)).unwrap();

            bv2_h
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidvalve_162_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let v_json = tmp.join("valves.json");
        let v_csv = tmp.join("valves.csv");
        let _ = app.run_command_line(&format!("PIDVALVE ALLTABS EXPORT \"{}\"", v_json.display()));
        assert!(v_json.is_file(), "PIDVALVE EXPORT must write valves.json");
        let _ = app.run_command_line(&format!(
            "PIDVALVE SUMMARY ALLTABS EXPORT \"{}\"",
            v_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&v_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("VALVE_ITEM")
                && csv_raw.contains("MTO_ROLLUP")
        );

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&v_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_valves"].as_u64(), Some(4));
        assert_eq!(rep["summary"]["manual_bulk_valves"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["actuated_control_valves"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["spec_resolved_valves"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["unresolved_valves"].as_u64(), Some(1));

        let rollup = rep["mto_rollup"].as_array().unwrap();
        let ball_row = rollup
            .iter()
            .find(|r| {
                r["valve_subtype"].as_str() == Some("ball_valve")
                    && r["nominal_diameter"].as_str() == Some("100 mm")
                    && r["piping_spec"].as_str() == Some("CS150")
            })
            .expect("must roll up 2x 100 mm CS150 ball valves across Tab 1 & Tab 2");
        assert_eq!(ball_row["quantity"].as_u64(), Some(2));

        // Filter UNRESOLVED selects GV16299 only
        let _ = app.run_command_line("PIDVALVE UNRESOLVED ALLTABS");
        assert!(has_sel(&app, tab1, gv_unres_h));
        assert!(!has_sel(&app, tab1, bv1_h));

        // Filter ACTUATED selects FV16201 only
        let _ = app.run_command_line("PIDVALVE ACTUATED ALLTABS");
        assert!(has_sel(&app, tab1, fv_h));
        assert!(!has_sel(&app, tab1, bv1_h));

        // Stateless NEXT steps across all 4 valves including BV16202A on Tab 2
        let mut visited_tab2 = false;
        for _ in 0..4 {
            let _ = app.run_command_line("PIDVALVE NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, bv2_h) {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDVALVE NEXT ALLTABS must visit BV16202A on Tab 2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDVALVE ALLTABS connected-pipeline spec inheritance, bulk MTO rollup, and NEXT stepping"]
    fn real_multi_tab_pidvalve_spec_inheritance_and_mto_rollup_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL162_SPPID_PAIR") else {
            println!("OCS_PID_REAL162_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL162_LEGACY_PAIR") else {
            println!("OCS_PID_REAL162_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL162_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real162"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let v_json = out_dir.join(format!("{gid}_valves.json"));
            let v_csv = out_dir.join(format!("{gid}_valves.csv"));
            let _ =
                app.run_command_line(&format!("PIDVALVE ALLTABS EXPORT \"{}\"", v_json.display()));
            let _ = app.run_command_line(&format!(
                "PIDVALVE SUMMARY ALLTABS EXPORT \"{}\"",
                v_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&v_csv).unwrap();
            assert!(
                csv_txt.starts_with('\u{feff}') && csv_txt.contains("RecordKind,TagOrRollupKey")
            );
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&v_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&v_json);
            let _ = std::fs::remove_file(&v_csv);

            assert!(
                rep["summary"]["total_valves"].as_u64().unwrap_or(0) >= 5,
                "{gid} must extract >= 5 valves"
            );
            assert!(
                rep["summary"]["spec_resolved_valves"].as_u64().unwrap_or(0) >= 1,
                "{gid} must inherit pipeline specs on >= 1 valve"
            );
            assert!(
                rep["summary"]["mto_groups_count"].as_u64().unwrap_or(0) >= 1,
                "{gid} must aggregate >= 1 Valve MTO procurement row"
            );

            let _ = app.run_command_line("PIDVALVE NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDVALVE NEXT ALLTABS must select target valve entity"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "total_valves": rep["summary"]["total_valves"],
                "manual_bulk_valves": rep["summary"]["manual_bulk_valves"],
                "actuated_control_valves": rep["summary"]["actuated_control_valves"],
                "safety_relief_valves": rep["summary"]["safety_relief_valves"],
                "spec_resolved_valves": rep["summary"]["spec_resolved_valves"],
                "unresolved_valves": rep["summary"]["unresolved_valves"],
                "mto_groups_count": rep["summary"]["mto_groups_count"],
            }));
        }

        std::fs::write(
            out_dir.join("real162_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidpackage_v2_emits_valve_mto_utility_and_hazop_artifacts_and_verifies_all_sha256_entries() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let mut app = app();

        // Tab 1: DWG-163-01.dxf with OSBL N2 utility tie-in + Pump P-1631 discharging via paired OPC to Tab 2
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-163-01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-163-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // OSBL N2 utility header
            let mut bl = Insert::new("_OFF-UNIT SQ", Vector3::new(40.0, 180.0, 0.0));
            bl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bl)).unwrap();
            let mut blt =
                Text::with_value("DWG-OSBL-901", Vector3::new(30.0, 179.0, 0.0)).with_height(2.5);
            blt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(blt)).unwrap();

            let mut up1 = Line::from_points(
                Vector3::new(46.0, 180.0, 0.0),
                Vector3::new(97.0, 180.0, 0.0),
            );
            up1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(up1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(100.0, 180.0, 0.0));
            bv1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV16301", Vector3::new(96.0, 186.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();

            let mut up2 = Line::from_points(
                Vector3::new(103.0, 180.0, 0.0),
                Vector3::new(175.0, 180.0, 0.0),
            );
            up2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(up2)).unwrap();
            let mut ulbl = Text::with_value("50-N2-16301-CS150", Vector3::new(125.0, 183.0, 0.0))
                .with_height(2.5);
            ulbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ulbl)).unwrap();

            // Pump P-1631 + process line 100-PG-16302-CS150 + BV16302 + PT-1632 -> paired OPC
            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 320.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1631", Vector3::new(95.0, 328.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut pp1 = Line::from_points(
                Vector3::new(104.0, 320.0, 0.0),
                Vector3::new(147.0, 320.0, 0.0),
            );
            pp1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pp1)).unwrap();

            let mut bv2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 320.0, 0.0));
            bv2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv2)).unwrap();
            let mut bvt2 =
                Text::with_value("BV16302", Vector3::new(146.0, 326.0, 0.0)).with_height(2.5);
            bvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt2)).unwrap();

            let mut pp2 = Line::from_points(
                Vector3::new(153.0, 320.0, 0.0),
                Vector3::new(254.0, 320.0, 0.0),
            );
            pp2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pp2)).unwrap();
            let mut plbl = Text::with_value("100-PG-16302-CS150", Vector3::new(190.0, 323.0, 0.0))
                .with_height(2.5);
            plbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(plbl)).unwrap();

            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(185.0, 328.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut it1 = Text::with_value("PT", Vector3::new(183.5, 330.0, 0.0)).with_height(2.2);
            it1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it1)).unwrap();
            let mut it2 =
                Text::with_value("1632", Vector3::new(182.5, 325.5, 0.0)).with_height(2.2);
            it2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it2)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 320.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-163-02", Vector3::new(250.0, 319.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();
        }

        // Tab 2: DWG-163-02.dxf with return OPC -> BV16303 -> Drum V-1632 + LT-1632
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-163-02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-163-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 320.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-163-01", Vector3::new(90.0, 319.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 320.0, 0.0),
                Vector3::new(177.0, 320.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-16302-CS150", Vector3::new(125.0, 323.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut bv3 = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 320.0, 0.0));
            bv3.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv3)).unwrap();
            let mut bvt3 =
                Text::with_value("BV16303", Vector3::new(176.0, 326.0, 0.0)).with_height(2.5);
            bvt3.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt3)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(183.0, 320.0, 0.0),
                Vector3::new(240.0, 320.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 320.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1632", Vector3::new(244.0, 336.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            let mut lt = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(260.0, 320.0, 0.0),
            );
            lt.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lt)).unwrap();
            let mut ltop = Text::with_value("LT", Vector3::new(258.5, 322.0, 0.0)).with_height(2.2);
            ltop.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ltop)).unwrap();
            let mut lbot =
                Text::with_value("1632", Vector3::new(257.5, 317.5, 0.0)).with_height(2.2);
            lbot.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lbot)).unwrap();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidpackage_163_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let pkg_dir = tmp.join("pkg_v2");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS VERIFY",
            pkg_dir.display()
        ));
        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();

        assert!(
            man["schema_version"].as_u64().unwrap_or(0) >= 2,
            "PIDPACKAGE v2+ must emit schema_version >= 2 with Valve MTO, Utility, and HAZOP schedules"
        );
        assert_eq!(man["total_valves"].as_u64(), Some(3));
        assert_eq!(man["utility_services_count"].as_u64(), Some(1));
        assert_eq!(man["battery_limit_interfaces"].as_u64(), Some(1));
        assert_eq!(man["hazop_nodes_count"].as_u64(), Some(2));
        assert_eq!(man["unprotected_hazop_nodes"].as_u64(), Some(0));

        for rel in [
            "schedules/03_valve_mto.csv",
            "schedules/03_valve_mto.json",
            "schedules/04_utility_and_battery_limits.csv",
            "schedules/04_utility_and_battery_limits.json",
            "audits/05_hazop_worksheet.csv",
            "audits/05_hazop_worksheet.json",
        ] {
            assert!(pkg_dir.join(rel).is_file(), "missing v2 artifact {rel}");
        }

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 18);
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and builds the v2 32-file five-discipline Digital Handover Package via PIDPACKAGE ALLTABS SEED VERIFY STRICT"]
    fn real_multi_tab_pidpackage_v2_five_discipline_handover_bundle_and_sha256_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL163_SPPID_PAIR") else {
            println!("OCS_PID_REAL163_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL163_LEGACY_PAIR") else {
            println!("OCS_PID_REAL163_LEGACY_PAIR unset, skipping.");
            return;
        };
        let Ok(seed_spec) = std::env::var("OCS_PID_REAL163_SEED") else {
            println!("OCS_PID_REAL163_SEED unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_REAL163_SYMDIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL163_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real163"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        for &ti in &tab_indices {
            let _ = app.update(Message::TabSwitch(ti));
            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");
        }
        let _ = app.update(Message::TabSwitch(tab_indices[0]));

        let pkg_dir = out_dir.join("pkg_v2_tmp");
        let sym_arg = if sym_dir.trim().is_empty() {
            String::new()
        } else {
            format!(" SYMDIR \"{sym_dir}\"")
        };
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 SEED \"{seed_spec}\"{sym_arg} VERIFY STRICT",
            pkg_dir.display()
        ));

        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert!(man["schema_version"].as_u64().unwrap_or(0) >= 2);
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(man["sheets_exported"].as_u64(), Some(4));
        assert_eq!(man["pid_files_written"].as_u64(), Some(4));
        assert_eq!(man["pdf_files_plotted"].as_u64(), Some(4));
        assert!(man["total_valves"].as_u64().unwrap_or(0) >= 100);
        assert!(man["battery_limit_interfaces"].as_u64().unwrap_or(0) >= 10);
        assert!(man["hazop_nodes_count"].as_u64().unwrap_or(0) >= 10);

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 32);
        let mut total_bytes = 0u64;
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            total_bytes += b;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let sanitized = json!({
            "source_sha256_prefixes": sha_prefixes,
            "schema_version": man["schema_version"],
            "handover_ready": man["handover_ready"],
            "strict_vendor_passed": man["strict_vendor_passed"],
            "sheets_exported": man["sheets_exported"],
            "pid_files_written": man["pid_files_written"],
            "pdf_files_plotted": man["pdf_files_plotted"],
            "bom_unique_rows": man["bom_unique_rows"],
            "instrument_loops_count": man["instrument_loops_count"],
            "total_valves": man["total_valves"],
            "valve_mto_groups": man["valve_mto_groups"],
            "utility_services_count": man["utility_services_count"],
            "battery_limit_interfaces": man["battery_limit_interfaces"],
            "hazop_nodes_count": man["hazop_nodes_count"],
            "unprotected_hazop_nodes": man["unprotected_hazop_nodes"],
            "opc_reciprocal_pairs": man["opc_reciprocal_pairs"],
            "total_files": files_arr.len(),
            "total_bytes": total_bytes,
            "manifest_sha256_prefix": &expected_man_sha[..12],
        });
        let _ = std::fs::remove_dir_all(&pkg_dir);
        std::fs::write(
            out_dir.join("real163_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidnozzle_extracts_explicit_and_implicit_nozzles_orientations_specs_spare_and_orphans_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: DWG-164-01.dxf with Drum V-1641 (right connected nozzle + top spare nozzle) and an orphan nozzle
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-164-01.dxf".to_string();
        let (nz_right_h, nz_top_spare_h, nz_orphan_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);

            let mut tb =
                Text::with_value("DWG-164-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // Drum V-1641 at (200, 300)
            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(200.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1641", Vector3::new(196.0, 322.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            // Right Connected Nozzle at (208, 300) -> pipe -> BV16401 -> 100-PG-16401-CS150
            let mut nzr = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(208.0, 300.0, 0.0));
            nzr.common.layer = "10".to_string();
            let nz_right_h = doc.add_entity(document::EntityType::Insert(nzr)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(210.0, 300.0, 0.0),
                Vector3::new(267.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(270.0, 300.0, 0.0));
            bv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv)).unwrap();
            let mut bvt =
                Text::with_value("BV16401", Vector3::new(266.0, 306.0, 0.0)).with_height(2.5);
            bvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(273.0, 300.0, 0.0),
                Vector3::new(340.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PG-16401-CS150", Vector3::new(290.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            // Top Spare Nozzle at (200, 314) without pipe
            let mut nzt = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(200.0, 314.0, 0.0));
            nzt.common.layer = "10".to_string();
            let nz_top_spare_h = doc.add_entity(document::EntityType::Insert(nzt)).unwrap();

            // Orphan Nozzle at (500, 500) far from any equipment
            let mut nzo = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(500.0, 500.0, 0.0));
            nzo.common.layer = "10".to_string();
            let nz_orphan_h = doc.add_entity(document::EntityType::Insert(nzo)).unwrap();

            (nz_right_h, nz_top_spare_h, nz_orphan_h)
        };

        // Tab 2: DWG-164-02.dxf with legacy-style implicit suction connection onto Pump P-1642
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-164-02.dxf".to_string();
        let imp_pipe_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);

            let mut tb =
                Text::with_value("DWG-164-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(150.0, 260.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1642", Vector3::new(146.0, 268.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut sp = Line::from_points(
                Vector3::new(80.0, 260.0, 0.0),
                Vector3::new(145.0, 260.0, 0.0),
            );
            sp.common.layer = "1".to_string();
            let imp_pipe_h = doc.add_entity(document::EntityType::Line(sp)).unwrap();
            let mut slbl = Text::with_value("80-CW-16402-CS150", Vector3::new(95.0, 263.0, 0.0))
                .with_height(2.5);
            slbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(slbl)).unwrap();

            imp_pipe_h
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidnozzle_164_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let nz_json = tmp.join("nozzles.json");
        let nz_csv = tmp.join("nozzles.csv");
        let _ = app.run_command_line(&format!(
            "PIDNOZZLE ALLTABS EXPORT \"{}\"",
            nz_json.display()
        ));
        assert!(
            nz_json.is_file(),
            "PIDNOZZLE EXPORT must write nozzles.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDNOZZLE SUMMARY ALLTABS EXPORT \"{}\"",
            nz_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&nz_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("V-1641")
                && csv_raw.contains("P-1642")
        );

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&nz_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_nozzles"].as_u64(), Some(4));
        assert_eq!(rep["summary"]["connected_nozzles"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["spare_nozzles"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["orphan_nozzles"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["implicit_nozzles"].as_u64(), Some(1));

        let rows = rep["nozzles"].as_array().unwrap();
        let v_right = rows
            .iter()
            .find(|r| {
                r["equipment_tag"].as_str() == Some("V-1641")
                    && r["status"].as_str() == Some("connected")
            })
            .expect("must have V-1641 connected right nozzle");
        assert_eq!(v_right["orientation"].as_str(), Some("right"));
        assert_eq!(v_right["nominal_diameter"].as_str(), Some("100 mm"));
        assert_eq!(v_right["piping_spec"].as_str(), Some("CS150"));
        assert_eq!(v_right["root_valve"].as_str(), Some("BV16401"));

        let p_left = rows
            .iter()
            .find(|r| r["equipment_tag"].as_str() == Some("P-1642"))
            .expect("must have P-1642 implicit left nozzle");
        assert_eq!(p_left["orientation"].as_str(), Some("left"));
        assert_eq!(p_left["nominal_diameter"].as_str(), Some("80 mm"));

        // Filter SPARE, ORPHAN, and EQUIPMENT V-1641
        let _ = app.run_command_line("PIDNOZZLE SPARE ALLTABS");
        assert!(has_sel(&app, tab1, nz_top_spare_h));
        assert!(!has_sel(&app, tab1, nz_right_h));

        let _ = app.run_command_line("PIDNOZZLE ORPHAN ALLTABS");
        assert!(has_sel(&app, tab1, nz_orphan_h));
        assert!(!has_sel(&app, tab1, nz_top_spare_h));

        let _ = app.run_command_line("PIDNOZZLE EQUIPMENT V-1641 ALLTABS");
        assert!(has_sel(&app, tab1, nz_right_h));
        assert!(has_sel(&app, tab1, nz_top_spare_h));
        assert!(!has_sel(&app, tab1, nz_orphan_h));

        // Stateless NEXT steps across all 4 nozzles including Tab 2 implicit nozzle
        let mut visited_tab2 = false;
        for _ in 0..4 {
            let _ = app.run_command_line("PIDNOZZLE NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, imp_pipe_h) {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDNOZZLE NEXT ALLTABS must visit Tab 2 implicit nozzle"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDNOZZLE ALLTABS explicit & implicit nozzle extraction, orientation & spec resolution, and NEXT stepping"]
    fn real_multi_tab_pidnozzle_explicit_and_implicit_schedule_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL164_SPPID_PAIR") else {
            println!("OCS_PID_REAL164_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL164_LEGACY_PAIR") else {
            println!("OCS_PID_REAL164_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL164_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real164"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let nz_json = out_dir.join(format!("{gid}_nozzles.json"));
            let nz_csv = out_dir.join(format!("{gid}_nozzles.csv"));
            let _ = app.run_command_line(&format!(
                "PIDNOZZLE ALLTABS EXPORT \"{}\"",
                nz_json.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDNOZZLE SUMMARY ALLTABS EXPORT \"{}\"",
                nz_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&nz_csv).unwrap();
            assert!(
                csv_txt.starts_with('\u{feff}')
                    && csv_txt.contains("DrawingCode,EquipmentTag,NozzleId")
            );
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&nz_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&nz_json);
            let _ = std::fs::remove_file(&nz_csv);

            assert!(
                rep["summary"]["total_nozzles"].as_u64().unwrap_or(0) >= 4,
                "{gid} must extract >= 4 equipment nozzles"
            );
            assert!(
                rep["summary"]["connected_nozzles"].as_u64().unwrap_or(0) >= 1,
                "{gid} must resolve >= 1 connected nozzle"
            );

            let _ = app.run_command_line("PIDNOZZLE NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDNOZZLE NEXT ALLTABS must select target nozzle entity"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "equipment_with_nozzles": rep["summary"]["equipment_with_nozzles"],
                "total_nozzles": rep["summary"]["total_nozzles"],
                "explicit_nozzles": rep["summary"]["explicit_nozzles"],
                "implicit_nozzles": rep["summary"]["implicit_nozzles"],
                "connected_nozzles": rep["summary"]["connected_nozzles"],
                "spare_nozzles": rep["summary"]["spare_nozzles"],
                "orphan_nozzles": rep["summary"]["orphan_nozzles"],
            }));
        }

        std::fs::write(
            out_dir.join("real164_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidindex_extracts_title_blocks_detects_missing_and_duplicate_numbers_and_jumps_tabs() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn add_a1_frame(doc: &mut document::Document) {
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();
        }

        let mut app = app();

        // Tab 1: valid sheet DWG-165-01 ("反应工段管道及仪表流程图", rev 0A)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "sheet_01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            add_a1_frame(doc);
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);

            for (val, x, y) in [
                ("DWG-165-01", 1100.0, 24.0),
                ("反应工段管道及仪表流程图", 1040.0, 42.0),
                ("版次", 1080.0, 14.0),
                ("0A", 1094.0, 14.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(150.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1651", Vector3::new(145.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(200.0, 300.0, 0.0));
            bv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv)).unwrap();
        }

        // Tab 2: duplicate DWG-165-DUP #1
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "sheet_02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            add_a1_frame(doc);
            for (val, x, y) in [
                ("DWG-165-DUP", 1100.0, 24.0),
                ("精馏工段管道及仪表流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }
        }

        // Tab 3: duplicate DWG-165-DUP #2
        let _ = app.update(Message::TabNew);
        let tab3 = app.active_tab;
        app.tabs[tab3].tab_title = "sheet_03.dxf".to_string();
        {
            let doc = &mut app.tabs[tab3].scene.document;
            add_a1_frame(doc);
            let mut t =
                Text::with_value("DWG-165-DUP", Vector3::new(1100.0, 24.0, 0.0)).with_height(3.5);
            t.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(t)).unwrap();
        }

        // Tab 4: missing drawing number in title block
        let _ = app.update(Message::TabNew);
        let tab4 = app.active_tab;
        app.tabs[tab4].tab_title = "sheet_04.dxf".to_string();
        {
            let doc = &mut app.tabs[tab4].scene.document;
            add_a1_frame(doc);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(200.0, 400.0, 0.0));
            bv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv)).unwrap();
        }

        let tmp = std::env::temp_dir().join(format!("ocs_pidindex_165_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let idx_json = tmp.join("index.json");
        let idx_csv = tmp.join("index.csv");
        let _ = app.run_command_line(&format!(
            "PIDINDEX ALLTABS EXPORT \"{}\"",
            idx_json.display()
        ));
        assert!(idx_json.is_file(), "PIDINDEX EXPORT must write index.json");
        let _ = app.run_command_line(&format!(
            "PIDINDEX SUMMARY ALLTABS EXPORT \"{}\"",
            idx_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&idx_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("DWG-165-01")
                && csv_raw.contains("反应工段管道及仪表流程图")
        );

        let rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&idx_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_sheets"].as_u64(), Some(4));
        assert_eq!(rep["summary"]["ok_sheets"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["duplicate_number_sheets"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["missing_number_sheets"].as_u64(), Some(1));

        let rows = rep["sheets"].as_array().unwrap();
        assert_eq!(rows[0]["drawing_number"].as_str(), Some("DWG-165-01"));
        assert_eq!(
            rows[0]["drawing_title"].as_str(),
            Some("反应工段管道及仪表流程图")
        );
        assert_eq!(rows[0]["revision"].as_str(), Some("0A"));
        assert_eq!(rows[0]["equipment_count"].as_u64(), Some(1));
        assert_eq!(rows[0]["valve_count"].as_u64(), Some(1));

        // From Tab 4, JUMP by Chinese title keyword switches immediately to Tab 1
        assert_eq!(app.active_tab, tab4);
        let _ = app.run_command_line("PIDINDEX JUMP 反应工段 ALLTABS");
        assert_eq!(app.active_tab, tab1);

        // ISSUES NEXT steps through the 3 issue tabs (tab2, tab3, tab4)
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..3 {
            let _ = app.run_command_line("PIDINDEX ISSUES NEXT ALLTABS");
            visited.insert(app.active_tab);
        }
        assert!(visited.contains(&tab2) && visited.contains(&tab3) && visited.contains(&tab4));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDINDEX ALLTABS title-block Master Drawing Index extraction and cross-tab JUMP"]
    fn real_multi_tab_pidindex_master_drawing_index_and_jump_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL165_SPPID_PAIR") else {
            println!("OCS_PID_REAL165_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL165_LEGACY_PAIR") else {
            println!("OCS_PID_REAL165_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL165_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real165"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let all_files = [
            ("SP01", &sp_paths[0]),
            ("SP02", &sp_paths[1]),
            ("LG01", &lg_paths[0]),
            ("LG02", &lg_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        let idx_json = out_dir.join("master_index.json");
        let idx_csv = out_dir.join("master_index.csv");
        let _ = app.run_command_line(&format!(
            "PIDINDEX ALLTABS EXPORT \"{}\"",
            idx_json.display()
        ));
        let _ = app.run_command_line(&format!(
            "PIDINDEX SUMMARY ALLTABS EXPORT \"{}\"",
            idx_csv.display()
        ));
        let csv_txt = std::fs::read_to_string(&idx_csv).unwrap();
        assert!(
            csv_txt.starts_with('\u{feff}') && csv_txt.contains("SeqNo,FileLabel,DrawingNumber")
        );
        let rep: Value =
            serde_json::from_str(&std::fs::read_to_string(&idx_json).unwrap()).unwrap();
        let _ = std::fs::remove_file(&idx_json);
        let _ = std::fs::remove_file(&idx_csv);

        assert_eq!(rep["summary"]["total_sheets"].as_u64(), Some(4));
        assert!(rep["summary"]["ok_sheets"].as_u64().unwrap_or(0) >= 2);
        assert!(rep["summary"]["total_equipment"].as_u64().unwrap_or(0) >= 15);
        assert!(rep["summary"]["total_valves"].as_u64().unwrap_or(0) >= 100);

        // Verify PIDINDEX JUMP switches active tab to SP01 (sequence 1 / SP01.dxf)
        let _ = app.update(Message::TabSwitch(tab_indices[3]));
        let _ = app.run_command_line("PIDINDEX JUMP SP01.dxf ALLTABS");
        assert_eq!(app.active_tab, tab_indices[0]);

        let sanitized_rows: Vec<Value> = rep["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                json!({
                    "id": all_files[i].0,
                    "sha256_prefix": sha_prefixes[i],
                    "has_drawing_number": !r["drawing_number"].as_str().unwrap_or("").is_empty(),
                    "has_drawing_title": !r["drawing_title"].as_str().unwrap_or("").is_empty(),
                    "status": r["status"],
                    "equipment_count": r["equipment_count"],
                    "valve_count": r["valve_count"],
                    "instrument_count": r["instrument_count"],
                    "pipeline_count": r["pipeline_count"],
                    "opc_count": r["opc_count"],
                })
            })
            .collect();

        std::fs::write(
            out_dir.join("real165_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "summary": rep["summary"],
                    "sheets": sanitized_rows,
                }))
                .unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidspecialty_extracts_strainers_dual_size_reducers_and_spectacle_blinds_excluding_valves_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: Y-STRAINER SR-16601 on 100 mm line + CONCENTRIC REDUCER bridging 100 mm -> 50 mm + BALL VALVE (excluded)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-166-01.dxf".to_string();
        let (strainer_h, red_h, bv_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "Y-STRAINER", 6.0, 4.0);
            define_sppid_block(doc, "CONCENTRIC REDUCER", 5.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);

            let mut tb =
                Text::with_value("DWG-166-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(40.0, 200.0, 0.0),
                Vector3::new(97.0, 200.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut st = Insert::new("Y-STRAINER", Vector3::new(100.0, 200.0, 0.0));
            st.common.layer = "11".to_string();
            let strainer_h = doc.add_entity(document::EntityType::Insert(st)).unwrap();
            let mut stt =
                Text::with_value("SR-16601", Vector3::new(95.0, 206.0, 0.0)).with_height(2.5);
            stt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(stt)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(103.0, 200.0, 0.0),
                Vector3::new(177.0, 200.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut l100 = Text::with_value("100-PG-16601-CS150", Vector3::new(125.0, 203.0, 0.0))
                .with_height(2.5);
            l100.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(l100)).unwrap();

            let mut red = Insert::new("CONCENTRIC REDUCER", Vector3::new(180.0, 200.0, 0.0));
            red.common.layer = "11".to_string();
            let red_h = doc.add_entity(document::EntityType::Insert(red)).unwrap();

            let mut p3 = Line::from_points(
                Vector3::new(183.0, 200.0, 0.0),
                Vector3::new(267.0, 200.0, 0.0),
            );
            p3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p3)).unwrap();
            let mut l50 = Text::with_value("50-PG-16602-CS150", Vector3::new(205.0, 203.0, 0.0))
                .with_height(2.5);
            l50.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(l50)).unwrap();

            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(270.0, 200.0, 0.0));
            bv.common.layer = "11".to_string();
            let bv_h = doc.add_entity(document::EntityType::Insert(bv)).unwrap();

            (strainer_h, red_h, bv_h)
        };

        // Tab 2: BLANK OPEN SPECTACLE on 80-N2-16603-CS150
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-166-02.dxf".to_string();
        let spec_blind_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BLANK OPEN SPECTACLE", 4.0, 6.0);

            let mut tb =
                Text::with_value("DWG-166-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(80.0, 260.0, 0.0),
                Vector3::new(138.0, 260.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("80-N2-16603-CS150", Vector3::new(95.0, 263.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut sb = Insert::new("BLANK OPEN SPECTACLE", Vector3::new(140.0, 260.0, 0.0));
            sb.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(sb)).unwrap()
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidspec_item_166_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let sp_json = tmp.join("specialties.json");
        let sp_csv = tmp.join("specialties.csv");
        let _ = app.run_command_line(&format!(
            "PIDSPECIALTY ALLTABS EXPORT \"{}\"",
            sp_json.display()
        ));
        assert!(
            sp_json.is_file(),
            "PIDSPECIALTY EXPORT must write specialties.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDSPECIALTY SUMMARY ALLTABS EXPORT \"{}\"",
            sp_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&sp_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("SPECIALTY_ITEM")
                && csv_raw.contains("100 mm x 50 mm")
        );

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&sp_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_items"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["specialty_items_count"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["reducers_count"].as_u64(), Some(1));

        let items = rep["items"].as_array().unwrap();
        let red_item = items
            .iter()
            .find(|it| it["subtype"].as_str() == Some("reducer"))
            .expect("must have reducer item");
        assert_eq!(red_item["size_spec"].as_str(), Some("100 mm x 50 mm"));
        assert_eq!(red_item["piping_spec"].as_str(), Some("CS150"));

        // Filter REDUCERS selects red_h only; SPECIALTIES selects strainer + spectacle blind and excludes red_h & bv_h
        let _ = app.run_command_line("PIDSPECIALTY REDUCERS ALLTABS");
        assert!(has_sel(&app, tab1, red_h));
        assert!(!has_sel(&app, tab1, strainer_h));
        assert!(!has_sel(&app, tab1, bv_h));

        let _ = app.run_command_line("PIDSPECIALTY SPECIALTIES ALLTABS");
        assert!(has_sel(&app, tab1, strainer_h));
        assert!(has_sel(&app, tab2, spec_blind_h));
        assert!(!has_sel(&app, tab1, red_h));

        // Stateless NEXT steps across all 3 items including Tab 2 spectacle blind
        let mut visited_tab2 = false;
        for _ in 0..3 {
            let _ = app.run_command_line("PIDSPECIALTY NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, spec_blind_h) {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDSPECIALTY NEXT ALLTABS must visit Tab 2 spectacle blind"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDSPECIALTY ALLTABS specialty item & reducer dual-size extraction and NEXT stepping"]
    fn real_multi_tab_pidspecialty_items_and_reducers_schedule_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL166_SPPID_PAIR") else {
            println!("OCS_PID_REAL166_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL166_LEGACY_PAIR") else {
            println!("OCS_PID_REAL166_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL166_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real166"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let sp_json = out_dir.join(format!("{gid}_specialties.json"));
            let sp_csv = out_dir.join(format!("{gid}_specialties.csv"));
            let _ = app.run_command_line(&format!(
                "PIDSPECIALTY ALLTABS EXPORT \"{}\"",
                sp_json.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDSPECIALTY SUMMARY ALLTABS EXPORT \"{}\"",
                sp_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&sp_csv).unwrap();
            assert!(
                csv_txt.starts_with('\u{feff}') && csv_txt.contains("RecordKind,TagOrRollupKey")
            );
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&sp_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&sp_json);
            let _ = std::fs::remove_file(&sp_csv);

            assert!(
                rep["summary"]["total_items"].as_u64().unwrap_or(0) >= 1,
                "{gid} must extract >= 1 piping specialty or inline fitting"
            );

            let _ = app.run_command_line("PIDSPECIALTY NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDSPECIALTY NEXT ALLTABS must select target specialty item"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "total_items": rep["summary"]["total_items"],
                "specialty_items_count": rep["summary"]["specialty_items_count"],
                "reducers_count": rep["summary"]["reducers_count"],
                "flanges_or_caps_count": rep["summary"]["flanges_or_caps_count"],
                "spec_resolved_count": rep["summary"]["spec_resolved_count"],
                "rollup_groups_count": rep["summary"]["rollup_groups_count"],
            }));
        }

        std::fs::write(
            out_dir.join("real166_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidlinelist_traces_cross_tab_from_to_endpoints_counts_valves_flags_unterminated_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let mut app = app();

        // Tab 1: P-1671 -> BV16701 -> 100-PG-16701-CS150-IH -> paired OPC to Tab 2, plus floating 50-N2-16799-CS150
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-167-01.dxf".to_string();
        let (p_tab1_h, unterm_pipe_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-167-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1671", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(147.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 300.0, 0.0));
            bv1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV16701", Vector3::new(146.0, 306.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(153.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            let p_tab1_h = doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt =
                Text::with_value("100-PG-16701-CS150-IH", Vector3::new(185.0, 303.0, 0.0))
                    .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-167-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();

            // Unterminated floating line 50-N2-16799-CS150
            let mut up = Line::from_points(
                Vector3::new(120.0, 480.0, 0.0),
                Vector3::new(220.0, 480.0, 0.0),
            );
            up.common.layer = "1".to_string();
            let unterm_pipe_h = doc.add_entity(document::EntityType::Line(up)).unwrap();
            let mut ulbl = Text::with_value("50-N2-16799-CS150", Vector3::new(145.0, 483.0, 0.0))
                .with_height(2.5);
            ulbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ulbl)).unwrap();

            (p_tab1_h, unterm_pipe_h)
        };

        // Tab 2: return OPC -> BV16702 -> V-1672, plus OSBL 80-LS-16703-CS300 -> V-1672
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-167-02.dxf".to_string();
        let (p_tab2_h, ls_pipe_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-167-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-167-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(177.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            let p_tab2_h = doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-16701-CS150-IH", Vector3::new(120.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut bv2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 300.0, 0.0));
            bv2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv2)).unwrap();
            let mut bvt2 =
                Text::with_value("BV16702", Vector3::new(176.0, 306.0, 0.0)).with_height(2.5);
            bvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt2)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(183.0, 300.0, 0.0),
                Vector3::new(240.0, 300.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1672", Vector3::new(244.0, 316.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            // OSBL Steam line 80-LS-16703-CS300 from DWG-OSBL-901 into V-1672 bottom
            let mut bl = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 180.0, 0.0));
            bl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bl)).unwrap();
            let mut blt =
                Text::with_value("DWG-OSBL-901", Vector3::new(90.0, 179.0, 0.0)).with_height(2.5);
            blt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(blt)).unwrap();

            let mut sp1 = Line::from_points(
                Vector3::new(106.0, 180.0, 0.0),
                Vector3::new(248.0, 180.0, 0.0),
            );
            sp1.common.layer = "1".to_string();
            let ls_pipe_h = doc.add_entity(document::EntityType::Line(sp1)).unwrap();
            let mut slbl = Text::with_value("80-LS-16703-CS300", Vector3::new(150.0, 183.0, 0.0))
                .with_height(2.5);
            slbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(slbl)).unwrap();

            let mut sp2 = Line::from_points(
                Vector3::new(248.0, 180.0, 0.0),
                Vector3::new(248.0, 286.0, 0.0),
            );
            sp2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(sp2)).unwrap();

            (p_tab2_h, ls_pipe_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        app.active_tab = tab1;

        let tmp = std::env::temp_dir().join(format!("ocs_pidlinelist_167_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let ll_json = tmp.join("lines.json");
        let ll_csv = tmp.join("lines.csv");
        let _ = app.run_command_line(&format!(
            "PIDLINELIST ALLTABS EXPORT \"{}\"",
            ll_json.display()
        ));
        assert!(
            ll_json.is_file(),
            "PIDLINELIST EXPORT must write lines.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDLINELIST SUMMARY ALLTABS EXPORT \"{}\"",
            ll_csv.display()
        ));
        let csv_raw = std::fs::read_to_string(&ll_csv).unwrap();
        assert!(
            csv_raw.starts_with('\u{feff}')
                && csv_raw.contains("P-1671")
                && csv_raw.contains("V-1672")
        );

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&ll_json).unwrap()).unwrap();
        assert_eq!(rep["summary"]["total_pipelines"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["both_terminated_count"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["unterminated_count"].as_u64(), Some(1));
        assert!(
            rep["summary"]["multi_sheet_pipelines"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );

        let lines = rep["pipelines"].as_array().unwrap();
        let pg_line = lines
            .iter()
            .find(|l| l["seq_no"].as_str() == Some("16701"))
            .expect("must have 16701 pipeline");
        assert_eq!(pg_line["nominal_diameter"].as_str(), Some("100 mm"));
        assert_eq!(pg_line["fluid_code"].as_str(), Some("PG"));
        assert_eq!(pg_line["piping_spec"].as_str(), Some("CS150"));
        assert_eq!(pg_line["insulation"].as_str(), Some("IH"));
        assert_eq!(pg_line["inline_valves_count"].as_u64(), Some(2));
        let ends = [
            pg_line["from_endpoint"].as_str().unwrap_or(""),
            pg_line["to_endpoint"].as_str().unwrap_or(""),
        ];
        assert!(ends.contains(&"P-1671") && ends.contains(&"V-1672"));

        // Filter UNTERMINATED selects unterm_pipe_h only; LINE 16701 selects across Tab 1 & Tab 2
        let _ = app.run_command_line("PIDLINELIST UNTERMINATED ALLTABS");
        assert!(has_sel(&app, tab1, unterm_pipe_h));
        assert!(!has_sel(&app, tab1, p_tab1_h));

        let _ = app.run_command_line("PIDLINELIST LINE 16701 ALLTABS");
        assert!(has_sel(&app, tab1, p_tab1_h));
        assert!(has_sel(&app, tab2, p_tab2_h));

        // Stateless NEXT steps across all 3 lines including Tab 2 steam line
        let mut visited_tab2 = false;
        for _ in 0..3 {
            let _ = app.run_command_line("PIDLINELIST NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, ls_pipe_h) {
                visited_tab2 = true;
            }
        }
        assert!(
            visited_tab2,
            "PIDLINELIST NEXT ALLTABS must visit Tab 2 steam line"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDLINELIST ALLTABS Master Line List From-To tracing and NEXT stepping"]
    fn real_multi_tab_pidlinelist_cross_sheet_from_to_schedule_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sp_raw) = std::env::var("OCS_PID_REAL167_SPPID_PAIR") else {
            println!("OCS_PID_REAL167_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(lg_raw) = std::env::var("OCS_PID_REAL167_LEGACY_PAIR") else {
            println!("OCS_PID_REAL167_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL167_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real167"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sp_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sp_raw).collect();
        let lg_paths: Vec<std::path::PathBuf> = std::env::split_paths(&lg_raw).collect();
        assert_eq!(sp_paths.len(), 2);
        assert_eq!(lg_paths.len(), 2);

        let groups = [
            ("SP01_SP02", "sppid_pair", &sp_paths),
            ("LG01_LG02", "legacy_autocad_pair", &lg_paths),
        ];

        let mut out_groups = Vec::new();
        for (gid, kind, pair) in &groups {
            let mut sha_pair = Vec::new();
            let mut app = app();
            for (pi, p) in pair.iter().enumerate() {
                let sha_pfx =
                    format!("{:x}", Sha256::digest(std::fs::read(p).unwrap()))[..12].to_string();
                sha_pair.push(sha_pfx);
                let ti = if pi == 0 {
                    app.active_tab
                } else {
                    let _ = app.update(Message::TabNew);
                    app.active_tab
                };
                app.tabs[ti].scene.document = crate::io::load_file(p).unwrap();
                app.tabs[ti].tab_title = format!("{gid}_{}.dxf", pi + 1);
            }

            let ll_json = out_dir.join(format!("{gid}_lines.json"));
            let ll_csv = out_dir.join(format!("{gid}_lines.csv"));
            let _ = app.run_command_line(&format!(
                "PIDLINELIST ALLTABS EXPORT \"{}\"",
                ll_json.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDLINELIST SUMMARY ALLTABS EXPORT \"{}\"",
                ll_csv.display()
            ));
            let csv_txt = std::fs::read_to_string(&ll_csv).unwrap();
            assert!(
                csv_txt.starts_with('\u{feff}')
                    && csv_txt.contains("LineNumber,NominalDiameter,FluidCode")
            );
            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&ll_json).unwrap()).unwrap();
            let _ = std::fs::remove_file(&ll_json);
            let _ = std::fs::remove_file(&ll_csv);

            assert!(
                rep["summary"]["total_pipelines"].as_u64().unwrap_or(0) >= 5,
                "{gid} must extract >= 5 pipelines"
            );
            assert!(
                rep["summary"]["both_terminated_count"]
                    .as_u64()
                    .unwrap_or(0)
                    >= 1,
                "{gid} must resolve From -> To endpoints on >= 1 pipeline"
            );

            let _ = app.run_command_line("PIDLINELIST NEXT ALLTABS");
            assert!(
                !app.tabs[app.active_tab]
                    .scene
                    .selected_entities()
                    .is_empty(),
                "PIDLINELIST NEXT ALLTABS must select target pipeline entities"
            );

            out_groups.push(json!({
                "group": gid,
                "kind": kind,
                "sha256_prefixes": sha_pair,
                "sheets_scanned": rep["summary"]["sheets_scanned"],
                "total_pipelines": rep["summary"]["total_pipelines"],
                "multi_sheet_pipelines": rep["summary"]["multi_sheet_pipelines"],
                "both_terminated_count": rep["summary"]["both_terminated_count"],
                "single_ended_count": rep["summary"]["single_ended_count"],
                "unterminated_count": rep["summary"]["unterminated_count"],
                "total_inline_valves": rep["summary"]["total_inline_valves"],
            }));
        }

        std::fs::write(
            out_dir.join("real167_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidpackage_v3_emits_all_eight_epc_schedules_and_verifies_26_file_sha256_manifest() {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let mut app = app();

        // Tab 1: DWG-168-01.dxf with A1 frame, Pump P-1681, Y-Strainer, Valve, PT-1681, and paired OPC
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-168-01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "Y-STRAINER", 6.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-168-01", 1100.0, 24.0),
                ("反应进料泵管道及仪表流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1681", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(137.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut st = Insert::new("Y-STRAINER", Vector3::new(140.0, 300.0, 0.0));
            st.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(st)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(143.0, 300.0, 0.0),
                Vector3::new(177.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();

            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 300.0, 0.0));
            bv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv)).unwrap();
            let mut bvt =
                Text::with_value("BV16801", Vector3::new(176.0, 306.0, 0.0)).with_height(2.5);
            bvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt)).unwrap();

            let mut p3 = Line::from_points(
                Vector3::new(183.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p3)).unwrap();
            let mut ltxt = Text::with_value("100-PG-16801-CS150", Vector3::new(200.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(210.0, 308.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut it1 = Text::with_value("PT", Vector3::new(208.5, 310.0, 0.0)).with_height(2.2);
            it1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it1)).unwrap();
            let mut it2 =
                Text::with_value("1681", Vector3::new(207.5, 305.5, 0.0)).with_height(2.2);
            it2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it2)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-168-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();
        }

        // Tab 2: DWG-168-02.dxf with return OPC -> Drum V-1682 + Explicit Nozzle + LT-1682
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-168-02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-168-02", 1100.0, 24.0),
                ("反应器缓冲罐管道及仪表流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-168-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(238.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-16801-CS150", Vector3::new(150.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut nz = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(240.0, 300.0, 0.0));
            nz.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(nz)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1682", Vector3::new(244.0, 316.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            let mut lt = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(260.0, 300.0, 0.0),
            );
            lt.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lt)).unwrap();
            let mut ltop = Text::with_value("LT", Vector3::new(258.5, 302.0, 0.0)).with_height(2.2);
            ltop.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ltop)).unwrap();
            let mut lbot =
                Text::with_value("1682", Vector3::new(257.5, 297.5, 0.0)).with_height(2.2);
            lbot.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lbot)).unwrap();
        }

        let _ = app.update(Message::TabSwitch(tab1));
        let tmp = std::env::temp_dir().join(format!("ocs_pidpackage_168_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let pkg_dir = tmp.join("pkg_v3");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS VERIFY STRICT",
            pkg_dir.display()
        ));
        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();

        assert!(
            man["schema_version"].as_u64().unwrap_or(0) >= 3,
            "PIDPACKAGE v3+ must emit schema_version >= 3 with all 8+ EPC schedules"
        );
        assert_eq!(man["drawing_index_issues"].as_u64(), Some(0));
        assert_eq!(man["master_pipelines_count"].as_u64(), Some(1));
        assert_eq!(man["unterminated_pipelines"].as_u64(), Some(0));
        assert!(man["equipment_nozzles_count"].as_u64().unwrap_or(0) >= 2);
        assert_eq!(man["piping_specialties_count"].as_u64(), Some(1));

        for rel in [
            "schedules/00_drawing_index.csv",
            "schedules/00_drawing_index.json",
            "schedules/05_line_list.csv",
            "schedules/05_line_list.json",
            "schedules/06_equipment_nozzles.csv",
            "schedules/06_equipment_nozzles.json",
            "schedules/07_piping_specialties.csv",
            "schedules/07_piping_specialties.json",
        ] {
            assert!(pkg_dir.join(rel).is_file(), "missing v3 artifact {rel}");
        }

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 26);
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and builds the v3 40-file 8-schedule + 3-audit EPC Digital Handover Package via PIDPACKAGE ALLTABS SEED VERIFY STRICT"]
    fn real_multi_tab_pidpackage_v3_eight_schedule_40_file_epc_bundle_and_sha256_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL168_SPPID_PAIR") else {
            println!("OCS_PID_REAL168_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL168_LEGACY_PAIR") else {
            println!("OCS_PID_REAL168_LEGACY_PAIR unset, skipping.");
            return;
        };
        let Ok(seed_spec) = std::env::var("OCS_PID_REAL168_SEED") else {
            println!("OCS_PID_REAL168_SEED unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_REAL168_SYMDIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL168_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real168"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        for &ti in &tab_indices {
            let _ = app.update(Message::TabSwitch(ti));
            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");
        }
        let _ = app.update(Message::TabSwitch(tab_indices[0]));

        let pkg_dir = out_dir.join("pkg_v3_tmp");
        let sym_arg = if sym_dir.trim().is_empty() {
            String::new()
        } else {
            format!(" SYMDIR \"{sym_dir}\"")
        };
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 SEED \"{seed_spec}\"{sym_arg} VERIFY STRICT",
            pkg_dir.display()
        ));

        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert!(man["schema_version"].as_u64().unwrap_or(0) >= 3);
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(man["sheets_exported"].as_u64(), Some(4));
        assert_eq!(man["pid_files_written"].as_u64(), Some(4));
        assert_eq!(man["pdf_files_plotted"].as_u64(), Some(4));
        assert!(man["master_pipelines_count"].as_u64().unwrap_or(0) >= 40);
        assert!(man["equipment_nozzles_count"].as_u64().unwrap_or(0) >= 100);
        assert!(man["piping_specialties_count"].as_u64().unwrap_or(0) >= 30);

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 40);
        let mut total_bytes = 0u64;
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            total_bytes += b;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let sanitized = json!({
            "source_sha256_prefixes": sha_prefixes,
            "schema_version": man["schema_version"],
            "handover_ready": man["handover_ready"],
            "strict_vendor_passed": man["strict_vendor_passed"],
            "sheets_exported": man["sheets_exported"],
            "pid_files_written": man["pid_files_written"],
            "pdf_files_plotted": man["pdf_files_plotted"],
            "bom_unique_rows": man["bom_unique_rows"],
            "instrument_loops_count": man["instrument_loops_count"],
            "master_pipelines_count": man["master_pipelines_count"],
            "equipment_nozzles_count": man["equipment_nozzles_count"],
            "total_valves": man["total_valves"],
            "piping_specialties_count": man["piping_specialties_count"],
            "battery_limit_interfaces": man["battery_limit_interfaces"],
            "hazop_nodes_count": man["hazop_nodes_count"],
            "opc_reciprocal_pairs": man["opc_reciprocal_pairs"],
            "total_files": files_arr.len(),
            "total_bytes": total_bytes,
            "manifest_sha256_prefix": &expected_man_sha[..12],
        });
        let _ = std::fs::remove_dir_all(&pkg_dir);
        std::fs::write(
            out_dir.join("real168_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidequip_builds_master_equipment_schedule_links_nozzles_lines_utilities_safeguards_flags_duplicates_and_untagged_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let tmp = std::env::temp_dir().join("ocs_pidequip_test_169");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1: P-1691 (Pump + Design P/T) -> BV16901 -> 100-PG-16901-CS150 -> OPC to Tab 2,
        // plus E-1692 (Exchanger, CW utility 80-CW-16902-CS150, duplicated on Tab 2),
        // plus untagged package equipment on layer 10.
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-169-01.dxf".to_string();
        let (pump_h, ex1_h, untag_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "HEAT EXCHANGER", 12.0, 12.0);
            define_sppid_block(doc, "PARAMETRIC BLACK BOX", 10.0, 10.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-169-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            let pump_h = doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1691", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(147.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(150.0, 300.0, 0.0));
            bv1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV16901", Vector3::new(146.0, 306.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(153.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PG-16901-CS150", Vector3::new(185.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-169-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();

            // Exchanger E-1692 with Cooling Water 80-CW-16902-CS150
            let mut ex1 = Insert::new("HEAT EXCHANGER", Vector3::new(140.0, 460.0, 0.0));
            ex1.common.layer = "10".to_string();
            let ex1_h = doc.add_entity(document::EntityType::Insert(ex1)).unwrap();
            let mut ext1 =
                Text::with_value("E-1692", Vector3::new(135.0, 470.0, 0.0)).with_height(2.5);
            ext1.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(ext1)).unwrap();

            let mut cw_ln = Line::from_points(
                Vector3::new(146.0, 460.0, 0.0),
                Vector3::new(220.0, 460.0, 0.0),
            );
            cw_ln.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(cw_ln)).unwrap();
            let mut cw_lbl = Text::with_value("80-CW-16902-CS150", Vector3::new(165.0, 463.0, 0.0))
                .with_height(2.5);
            cw_lbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(cw_lbl)).unwrap();

            // Untagged package equipment far away from any tag text
            let mut untag = Insert::new("PARAMETRIC BLACK BOX", Vector3::new(420.0, 520.0, 0.0));
            untag.common.layer = "10".to_string();
            let untag_h = doc.add_entity(document::EntityType::Insert(untag)).unwrap();

            (pump_h, ex1_h, untag_h)
        };

        // Tab 2: return OPC -> Nozzle -> V-1693 (Vessel + PT-1693 safeguard),
        // plus TK-1694 (Storage Tank), plus duplicate E-1692.
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-169-02.dxf".to_string();
        let (drum_h, tank_h, ex2_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "HEAT EXCHANGER", 12.0, 12.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-169-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-169-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(238.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-16901-CS150", Vector3::new(145.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(190.0, 308.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut it1 = Text::with_value("PT", Vector3::new(188.5, 310.0, 0.0)).with_height(2.2);
            it1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it1)).unwrap();
            let mut it2 =
                Text::with_value("1693", Vector3::new(187.5, 305.5, 0.0)).with_height(2.2);
            it2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it2)).unwrap();

            let mut nz = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(240.0, 300.0, 0.0));
            nz.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(nz)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            let drum_h = doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1693", Vector3::new(244.0, 316.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            let mut tank = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(400.0, 300.0, 0.0));
            tank.common.layer = "10".to_string();
            let tank_h = doc.add_entity(document::EntityType::Insert(tank)).unwrap();
            let mut tkt =
                Text::with_value("TK-1694", Vector3::new(394.0, 314.0, 0.0)).with_height(2.5);
            tkt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(tkt)).unwrap();

            let mut ex2 = Insert::new("HEAT EXCHANGER", Vector3::new(400.0, 480.0, 0.0));
            ex2.common.layer = "10".to_string();
            let ex2_h = doc.add_entity(document::EntityType::Insert(ex2)).unwrap();
            let mut ext2 =
                Text::with_value("E-1692", Vector3::new(395.0, 490.0, 0.0)).with_height(2.5);
            ext2.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(ext2)).unwrap();

            (drum_h, tank_h, ex2_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let json_p = tmp.join("equip.json");
        let csv_p = tmp.join("equip.csv");
        let _ = app.run_command_line(&format!("PIDEQUIP ALLTABS EXPORT \"{}\"", json_p.display()));
        assert!(
            json_p.is_file(),
            "PIDEQUIP EXPORT must write Master Equipment Schedule equip.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDEQUIP SUMMARY ALLTABS EXPORT \"{}\"",
            csv_p.display()
        ));
        assert!(csv_p.is_file());
        let csv_bytes = std::fs::read(&csv_p).unwrap();
        assert!(csv_bytes.starts_with(&[0xEF, 0xBB, 0xBF]));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
        assert_eq!(rep["summary"]["sheets_scanned"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["total_equipment"].as_u64(), Some(5));
        assert_eq!(rep["summary"]["vessels_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["rotating_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["heat_transfer_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["tanks_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["package_misc_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["untagged_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["duplicate_tag_count"].as_u64(), Some(1));

        let rows = rep["equipment"].as_array().unwrap();
        assert_eq!(rows.len(), 5);

        let v_row = rows
            .iter()
            .find(|r| r["equipment_tag"].as_str() == Some("V-1693"))
            .expect("V-1693 row");
        assert_eq!(v_row["category"].as_str(), Some("vessel_or_column"));
        assert_eq!(v_row["tag_status"].as_str(), Some("ok"));
        assert!(v_row["nozzle_count"].as_u64().unwrap_or(0) >= 1);
        assert!(v_row["connected_lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("16901")));
        assert!(v_row["relief_or_safeguards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("1693")));

        let ex_row = rows
            .iter()
            .find(|r| r["equipment_tag"].as_str() == Some("E-1692"))
            .expect("E-1692 row");
        assert_eq!(ex_row["category"].as_str(), Some("heat_transfer"));
        assert_eq!(
            ex_row["tag_status"].as_str(),
            Some("duplicate_across_sheets")
        );
        assert_eq!(ex_row["occurrence_count"].as_u64(), Some(2));
        assert!(ex_row["utility_services"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("CW")));

        let _ = app.run_command_line("PIDEQUIP ROTATING ALLTABS");
        assert!(has_sel(&app, tab1, pump_h));

        let _ = app.run_command_line("PIDEQUIP UNTAGGED ALLTABS");
        assert!(has_sel(&app, tab1, untag_h));

        let _ = app.run_command_line("PIDEQUIP DUPLICATES ALLTABS");
        assert!(has_sel(&app, tab1, ex1_h) && has_sel(&app, tab2, ex2_h));

        let _ = app.run_command_line("PIDEQUIP TAG V-1693 ALLTABS");
        assert_eq!(app.active_tab, tab2);
        assert!(has_sel(&app, tab2, drum_h));

        let mut visited_tk = false;
        for _ in 0..6 {
            let _ = app.run_command_line("PIDEQUIP NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, tank_h) {
                visited_tk = true;
                break;
            }
        }
        assert!(
            visited_tk,
            "PIDEQUIP NEXT ALLTABS must visit TK-1694 on Tab 2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDEQUIP ALLTABS Master Equipment Schedule extraction and NEXT stepping"]
    fn real_multi_tab_pidequip_master_equipment_schedule_and_next_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL169_SPPID_PAIR") else {
            println!("OCS_PID_REAL169_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL169_LEGACY_PAIR") else {
            println!("OCS_PID_REAL169_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL169_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real169"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let groups = [("SP01_SP02", &sppid_paths), ("LG01_LG02", &legacy_paths)];
        let mut out_groups = Vec::new();

        for (gname, paths) in groups {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in paths.iter().enumerate() {
                let raw_b = std::fs::read(p).unwrap();
                shas.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].tab_title = format!("{gname}_{idx}.dxf");
            }

            let json_p = out_dir.join(format!("{gname}_equip.json"));
            let csv_p = out_dir.join(format!("{gname}_equip.csv"));
            let _ =
                app.run_command_line(&format!("PIDEQUIP ALLTABS EXPORT \"{}\"", json_p.display()));
            let _ = app.run_command_line(&format!(
                "PIDEQUIP SUMMARY ALLTABS EXPORT \"{}\"",
                csv_p.display()
            ));
            assert!(json_p.is_file() && csv_p.is_file());
            let csv_raw = std::fs::read(&csv_p).unwrap();
            assert!(csv_raw.starts_with(&[0xEF, 0xBB, 0xBF]));

            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
            let tot = rep["summary"]["total_equipment"].as_u64().unwrap_or(0);
            assert!(tot >= 4, "{gname} must extract >= 4 core equipment items");

            let _ = app.run_command_line("PIDEQUIP NEXT ALLTABS");
            let cur_tab = app.active_tab;
            assert!(
                !app.tabs[cur_tab].scene.selected_entities().is_empty(),
                "PIDEQUIP NEXT ALLTABS must select target equipment entities"
            );

            let _ = std::fs::remove_file(&json_p);
            let _ = std::fs::remove_file(&csv_p);

            out_groups.push(json!({
                "group": gname,
                "sha256_prefixes": shas,
                "total_equipment": rep["summary"]["total_equipment"],
                "vessels_count": rep["summary"]["vessels_count"],
                "rotating_count": rep["summary"]["rotating_count"],
                "heat_transfer_count": rep["summary"]["heat_transfer_count"],
                "tanks_count": rep["summary"]["tanks_count"],
                "package_misc_count": rep["summary"]["package_misc_count"],
                "untagged_count": rep["summary"]["untagged_count"],
                "duplicate_tag_count": rep["summary"]["duplicate_tag_count"],
                "total_connected_nozzles": rep["summary"]["total_connected_nozzles"],
            }));
        }

        std::fs::write(
            out_dir.join("real169_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidrelief_traces_protected_equipment_flare_and_atm_destinations_audits_cso_vs_unsealed_block_valves_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let tmp = std::env::temp_dir().join("ocs_pidrelief_test_170");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1:
        // 1) V-1701 -> 80-PG-17001-CS300 -> BV1701 ("CSO") -> PSV-1701 -> 100-FL-17002-CS150 -> OPC to Tab 2 (Compliant, Flare)
        // 2) E-1702 -> 50-PG-17003-CS300 -> BV1702 (unsealed!) -> PSV-1702 -> 80-ATM-17004-CS150 (UnsealedBlockValve, Atmosphere)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-170-01.dxf".to_string();
        let (psv1_h, psv2_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "HEAT EXCHANGER", 12.0, 12.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "SAFETY VALVE TYPE 1", 6.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-170-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            // V-1701 at (100, 300)
            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(100.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1701", Vector3::new(95.0, 316.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(157.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();
            let mut l1 = Text::with_value("80-PG-17001-CS300", Vector3::new(115.0, 303.0, 0.0))
                .with_height(2.5);
            l1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(l1)).unwrap();

            let mut bv1 = Insert::new("BALL VALVE TYPE 2", Vector3::new(160.0, 300.0, 0.0));
            bv1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv1)).unwrap();
            let mut bvt1 =
                Text::with_value("BV1701", Vector3::new(156.0, 306.0, 0.0)).with_height(2.5);
            bvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt1)).unwrap();
            let mut cso_txt =
                Text::with_value("CSO", Vector3::new(158.0, 294.0, 0.0)).with_height(2.2);
            cso_txt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(cso_txt)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(163.0, 300.0, 0.0),
                Vector3::new(197.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();

            let mut psv1 = Insert::new("SAFETY VALVE TYPE 1", Vector3::new(200.0, 300.0, 0.0));
            psv1.common.layer = "11".to_string();
            let psv1_h = doc.add_entity(document::EntityType::Insert(psv1)).unwrap();
            let mut psvt1 =
                Text::with_value("PSV-1701", Vector3::new(195.0, 308.0, 0.0)).with_height(2.5);
            psvt1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(psvt1)).unwrap();

            let mut p3 = Line::from_points(
                Vector3::new(203.0, 300.0, 0.0),
                Vector3::new(274.0, 300.0, 0.0),
            );
            p3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p3)).unwrap();
            let mut l2 = Text::with_value("100-FL-17002-CS150", Vector3::new(220.0, 303.0, 0.0))
                .with_height(2.5);
            l2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(l2)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(280.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-170-02", Vector3::new(270.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();

            // Second train: E-1702 -> 50-PG-17003-CS300 -> BV1702 (NO CSO!) -> PSV-1702 -> 80-ATM-17004-CS150
            let mut ex = Insert::new("HEAT EXCHANGER", Vector3::new(100.0, 480.0, 0.0));
            ex.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(ex)).unwrap();
            let mut ext =
                Text::with_value("E-1702", Vector3::new(95.0, 490.0, 0.0)).with_height(2.5);
            ext.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(ext)).unwrap();

            let mut q1 = Line::from_points(
                Vector3::new(106.0, 480.0, 0.0),
                Vector3::new(157.0, 480.0, 0.0),
            );
            q1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(q1)).unwrap();
            let mut ql1 = Text::with_value("50-PG-17003-CS300", Vector3::new(115.0, 483.0, 0.0))
                .with_height(2.5);
            ql1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ql1)).unwrap();

            let mut bv2 = Insert::new("BALL VALVE TYPE 2", Vector3::new(160.0, 480.0, 0.0));
            bv2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv2)).unwrap();
            let mut bvt2 =
                Text::with_value("BV1702", Vector3::new(156.0, 486.0, 0.0)).with_height(2.5);
            bvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt2)).unwrap();

            let mut q2 = Line::from_points(
                Vector3::new(163.0, 480.0, 0.0),
                Vector3::new(197.0, 480.0, 0.0),
            );
            q2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(q2)).unwrap();

            let mut psv2 = Insert::new("SAFETY VALVE TYPE 1", Vector3::new(200.0, 480.0, 0.0));
            psv2.common.layer = "11".to_string();
            let psv2_h = doc.add_entity(document::EntityType::Insert(psv2)).unwrap();
            let mut psvt2 =
                Text::with_value("PSV-1702", Vector3::new(195.0, 488.0, 0.0)).with_height(2.5);
            psvt2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(psvt2)).unwrap();

            let mut q3 = Line::from_points(
                Vector3::new(203.0, 480.0, 0.0),
                Vector3::new(270.0, 480.0, 0.0),
            );
            q3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(q3)).unwrap();
            let mut ql2 = Text::with_value("80-ATM-17004-CS150", Vector3::new(215.0, 483.0, 0.0))
                .with_height(2.5);
            ql2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ql2)).unwrap();

            (psv1_h, psv2_h)
        };

        // Tab 2: return flare OPC + TK-1703 with Breather Valve PVSV-1703 to ATM + orphan PSE-1799
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-170-02.dxf".to_string();
        let (pvsv_h, pse_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 16.0, 24.0);
            define_sppid_block(doc, "BREATHER VALVE TYPE 1", 6.0, 8.0);
            define_sppid_block(doc, "SAFETY VALVE TYPE 1", 6.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-170-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-170-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut fl_ln = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(220.0, 300.0, 0.0),
            );
            fl_ln.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(fl_ln)).unwrap();
            let mut fl_lbl =
                Text::with_value("100-FL-17002-CS150", Vector3::new(135.0, 303.0, 0.0))
                    .with_height(2.5);
            fl_lbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(fl_lbl)).unwrap();

            // Storage tank TK-1703 -> 100-PG-17005-CS150 -> PVSV-1703 ("TO ATM")
            let mut tk = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(340.0, 300.0, 0.0));
            tk.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(tk)).unwrap();
            let mut tkt =
                Text::with_value("TK-1703", Vector3::new(334.0, 316.0, 0.0)).with_height(2.5);
            tkt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(tkt)).unwrap();

            let mut tk_ln = Line::from_points(
                Vector3::new(340.0, 312.0, 0.0),
                Vector3::new(340.0, 356.0, 0.0),
            );
            tk_ln.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(tk_ln)).unwrap();
            let mut tk_lbl =
                Text::with_value("100-PG-17005-CS150", Vector3::new(344.0, 330.0, 0.0))
                    .with_height(2.5);
            tk_lbl.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(tk_lbl)).unwrap();

            let mut pvsv = Insert::new("BREATHER VALVE TYPE 1", Vector3::new(340.0, 360.0, 0.0));
            pvsv.common.layer = "11".to_string();
            let pvsv_h = doc.add_entity(document::EntityType::Insert(pvsv)).unwrap();
            let mut pvsv_t =
                Text::with_value("PVSV-1703", Vector3::new(346.0, 360.0, 0.0)).with_height(2.5);
            pvsv_t.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(pvsv_t)).unwrap();
            let mut atm_t =
                Text::with_value("TO ATM", Vector3::new(336.0, 370.0, 0.0)).with_height(2.2);
            atm_t.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(atm_t)).unwrap();

            // Unconnected rupture disc PSE-1799
            let mut pse = Insert::new("SAFETY VALVE TYPE 1", Vector3::new(520.0, 480.0, 0.0));
            pse.common.layer = "11".to_string();
            let pse_h = doc.add_entity(document::EntityType::Insert(pse)).unwrap();
            let mut pset =
                Text::with_value("PSE-1799", Vector3::new(515.0, 488.0, 0.0)).with_height(2.5);
            pset.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(pset)).unwrap();

            (pvsv_h, pse_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let json_p = tmp.join("relief.json");
        let csv_p = tmp.join("relief.csv");
        let _ = app.run_command_line(&format!(
            "PIDRELIEF ALLTABS EXPORT \"{}\"",
            json_p.display()
        ));
        assert!(
            json_p.is_file(),
            "PIDRELIEF EXPORT must write Pressure Relief Device Schedule relief.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDRELIEF SUMMARY ALLTABS EXPORT \"{}\"",
            csv_p.display()
        ));
        assert!(csv_p.is_file());
        let csv_bytes = std::fs::read(&csv_p).unwrap();
        assert!(csv_bytes.starts_with(&[0xEF, 0xBB, 0xBF]));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
        assert_eq!(rep["summary"]["sheets_scanned"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["total_devices"].as_u64(), Some(4));
        assert_eq!(rep["summary"]["compliant_count"].as_u64(), Some(2));
        assert_eq!(
            rep["summary"]["unsealed_valve_violations"].as_u64(),
            Some(1)
        );
        assert_eq!(rep["summary"]["unconnected_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["flare_discharges"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["atmosphere_discharges"].as_u64(), Some(2));

        let devs = rep["devices"].as_array().unwrap();
        let r1 = devs
            .iter()
            .find(|d| d["device_tag"].as_str() == Some("PSV-1701"))
            .expect("PSV-1701");
        assert_eq!(r1["device_type"].as_str(), Some("safety_relief_valve"));
        assert_eq!(r1["protected_equipment"].as_str(), Some("V-1701"));
        assert_eq!(r1["inlet_dn"].as_str(), Some("80 mm"));
        assert_eq!(r1["outlet_dn"].as_str(), Some("100 mm"));
        assert_eq!(r1["destination"].as_str(), Some("flare_header"));
        assert_eq!(r1["compliance_status"].as_str(), Some("compliant"));

        let r2 = devs
            .iter()
            .find(|d| d["device_tag"].as_str() == Some("PSV-1702"))
            .expect("PSV-1702");
        assert_eq!(r2["protected_equipment"].as_str(), Some("E-1702"));
        assert_eq!(r2["destination"].as_str(), Some("atmosphere"));
        assert_eq!(
            r2["compliance_status"].as_str(),
            Some("unsealed_block_valve")
        );

        let r3 = devs
            .iter()
            .find(|d| d["device_tag"].as_str() == Some("PVSV-1703"))
            .expect("PVSV-1703");
        assert_eq!(r3["device_type"].as_str(), Some("breather_valve"));
        assert_eq!(r3["protected_equipment"].as_str(), Some("TK-1703"));
        assert_eq!(r3["destination"].as_str(), Some("atmosphere"));
        assert_eq!(r3["compliance_status"].as_str(), Some("compliant"));

        let r4 = devs
            .iter()
            .find(|d| d["device_tag"].as_str() == Some("PSE-1799"))
            .expect("PSE-1799");
        assert_eq!(r4["device_type"].as_str(), Some("rupture_disc"));
        assert_eq!(r4["compliance_status"].as_str(), Some("unconnected_relief"));

        let _ = app.run_command_line("PIDRELIEF FLARE ALLTABS");
        assert!(has_sel(&app, tab1, psv1_h) && !has_sel(&app, tab1, psv2_h));

        let _ = app.run_command_line("PIDRELIEF VIOLATIONS ALLTABS");
        assert!(has_sel(&app, tab1, psv2_h) && has_sel(&app, tab2, pse_h));

        let _ = app.run_command_line("PIDRELIEF TAG PVSV-1703 ALLTABS");
        assert_eq!(app.active_tab, tab2);
        assert!(has_sel(&app, tab2, pvsv_h));

        let mut visited_pse = false;
        for _ in 0..5 {
            let _ = app.run_command_line("PIDRELIEF NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, pse_h) {
                visited_pse = true;
                break;
            }
        }
        assert!(
            visited_pse,
            "PIDRELIEF NEXT ALLTABS must visit PSE-1799 on Tab 2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads real SPPID production sheets and native AutoCAD legacy sheets with PSV safety relief valves into OpenCADStudio and verifies PIDRELIEF ALLTABS schedule and NEXT stepping"]
    fn real_multi_tab_pidrelief_schedule_destination_and_cso_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL170_SPPID_PAIR") else {
            println!("OCS_PID_REAL170_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL170_LEGACY_SHEETS") else {
            println!("OCS_PID_REAL170_LEGACY_SHEETS unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL170_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real170"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert!(!sppid_paths.is_empty() && !legacy_paths.is_empty());

        let groups = [("SP01_SP02", &sppid_paths), ("LG_PSV_SET", &legacy_paths)];
        let mut out_groups = Vec::new();

        for (gname, paths) in groups {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in paths.iter().enumerate() {
                let raw_b = std::fs::read(p).unwrap();
                shas.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].tab_title = format!("{gname}_{idx}.dxf");
            }

            let json_p = out_dir.join(format!("{gname}_relief.json"));
            let csv_p = out_dir.join(format!("{gname}_relief.csv"));
            let _ = app.run_command_line(&format!(
                "PIDRELIEF ALLTABS EXPORT \"{}\"",
                json_p.display()
            ));
            let _ = app.run_command_line(&format!(
                "PIDRELIEF SUMMARY ALLTABS EXPORT \"{}\"",
                csv_p.display()
            ));
            assert!(json_p.is_file() && csv_p.is_file());
            let csv_raw = std::fs::read(&csv_p).unwrap();
            assert!(csv_raw.starts_with(&[0xEF, 0xBB, 0xBF]));

            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
            let tot = rep["summary"]["total_devices"].as_u64().unwrap_or(0);
            assert!(
                tot >= 2,
                "{gname} must extract >= 2 relief devices, got {tot}"
            );

            let _ = app.run_command_line("PIDRELIEF NEXT ALLTABS");
            let cur_tab = app.active_tab;
            assert!(
                !app.tabs[cur_tab].scene.selected_entities().is_empty(),
                "PIDRELIEF NEXT ALLTABS must select target relief device"
            );

            let _ = std::fs::remove_file(&json_p);
            let _ = std::fs::remove_file(&csv_p);

            out_groups.push(json!({
                "group": gname,
                "sha256_prefixes": shas,
                "total_devices": rep["summary"]["total_devices"],
                "safety_valves_count": rep["summary"]["safety_valves_count"],
                "rupture_discs_count": rep["summary"]["rupture_discs_count"],
                "breather_valves_count": rep["summary"]["breather_valves_count"],
                "blowdown_valves_count": rep["summary"]["blowdown_valves_count"],
                "flare_discharges": rep["summary"]["flare_discharges"],
                "atmosphere_discharges": rep["summary"]["atmosphere_discharges"],
                "closed_discharges": rep["summary"]["closed_discharges"],
                "compliant_count": rep["summary"]["compliant_count"],
                "unsealed_valve_violations": rep["summary"]["unsealed_valve_violations"],
                "unconnected_count": rep["summary"]["unconnected_count"],
            }));
        }

        std::fs::write(
            out_dir.join("real170_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidpackage_v4_emits_all_ten_epc_schedules_blocks_unsealed_psv_strict_gate_and_verifies_30_file_sha256(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let tmp = std::env::temp_dir().join("ocs_pidpackage_v4_test_171");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1: DWG-171-01.dxf with P-1711 -> Y-STRAINER -> 100-PG-17101-CS150 -> PT-1711 -> OPC to Tab 2
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-171-01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "Y-STRAINER", 6.0, 6.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-171-01", 1100.0, 24.0),
                ("高压进料泵单元管道及仪表流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1711", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(137.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut st = Insert::new("Y-STRAINER", Vector3::new(140.0, 300.0, 0.0));
            st.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(st)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(143.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PG-17101-CS150", Vector3::new(185.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(210.0, 308.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut it1 = Text::with_value("PT", Vector3::new(208.5, 310.0, 0.0)).with_height(2.2);
            it1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it1)).unwrap();
            let mut it2 =
                Text::with_value("1711", Vector3::new(207.5, 305.5, 0.0)).with_height(2.2);
            it2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it2)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-171-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();
        }

        // Tab 2: DWG-171-02.dxf with return OPC -> Nozzle -> V-1712, plus relief branch with unsealed BV1711 -> PSV-1711 -> 100-FL-17103-CS150
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-171-02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "SAFETY VALVE TYPE 1", 6.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-171-02", 1100.0, 24.0),
                ("高压闪蒸罐及火炬泄压流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-171-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(238.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-17101-CS150", Vector3::new(150.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut nz = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(240.0, 300.0, 0.0));
            nz.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(nz)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1712", Vector3::new(244.0, 282.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            // Top relief riser: V-1712 -> 80-PG-17102-CS300 -> BV1711 (unsealed initially!) -> PSV-1711 -> 100-FL-17103-CS150
            let mut r1 = Line::from_points(
                Vector3::new(254.0, 310.0, 0.0),
                Vector3::new(297.0, 310.0, 0.0),
            );
            r1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(r1)).unwrap();
            let mut rl1 = Text::with_value("80-PG-17102-CS300", Vector3::new(260.0, 313.0, 0.0))
                .with_height(2.5);
            rl1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(rl1)).unwrap();

            let mut bv = Insert::new("BALL VALVE TYPE 2", Vector3::new(300.0, 310.0, 0.0));
            bv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(bv)).unwrap();
            let mut bvt =
                Text::with_value("BV1711", Vector3::new(296.0, 316.0, 0.0)).with_height(2.5);
            bvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(bvt)).unwrap();

            let mut r2 = Line::from_points(
                Vector3::new(303.0, 310.0, 0.0),
                Vector3::new(337.0, 310.0, 0.0),
            );
            r2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(r2)).unwrap();

            let mut psv = Insert::new("SAFETY VALVE TYPE 1", Vector3::new(340.0, 310.0, 0.0));
            psv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(psv)).unwrap();
            let mut psvt =
                Text::with_value("PSV-1711", Vector3::new(335.0, 318.0, 0.0)).with_height(2.5);
            psvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(psvt)).unwrap();

            let mut r3 = Line::from_points(
                Vector3::new(343.0, 310.0, 0.0),
                Vector3::new(410.0, 310.0, 0.0),
            );
            r3.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(r3)).unwrap();
            let mut rl2 = Text::with_value("100-FL-17103-CS150", Vector3::new(355.0, 313.0, 0.0))
                .with_height(2.5);
            rl2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(rl2)).unwrap();
        }

        // Phase A: STRICT gate must block handover_ready when BV1711 lacks CSO/LO
        let _ = app.update(Message::TabSwitch(tab1));
        let unsafe_dir = tmp.join("pkg_v4_unsafe");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 VERIFY STRICT",
            unsafe_dir.display()
        ));
        let unsafe_man_p = unsafe_dir.join("handover_manifest.json");
        assert!(unsafe_man_p.is_file());
        let unsafe_man: Value =
            serde_json::from_str(&std::fs::read_to_string(&unsafe_man_p).unwrap()).unwrap();
        assert!(
            unsafe_man["schema_version"].as_u64().unwrap_or(0) >= 4,
            "PIDPACKAGE v4+ must emit schema_version >= 4 with all 10+ EPC schedules and API 520 relief gate"
        );
        assert_eq!(
            unsafe_man["unsealed_relief_valve_violations"].as_u64(),
            Some(1)
        );
        assert_eq!(unsafe_man["handover_ready"].as_bool(), Some(false));

        // Phase B: Add CSO annotation to BV1711 on Tab 2 and verify full 30-file green handover
        {
            let doc2 = &mut app.tabs[tab2].scene.document;
            let mut cso_t =
                Text::with_value("CSO", Vector3::new(298.0, 304.0, 0.0)).with_height(2.2);
            cso_t.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(cso_t)).unwrap();
        }

        let safe_dir = tmp.join("pkg_v4_safe");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 VERIFY STRICT",
            safe_dir.display()
        ));
        let man_p = safe_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert!(man["schema_version"].as_u64().unwrap_or(0) >= 4);
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["unsealed_relief_valve_violations"].as_u64(), Some(0));
        assert_eq!(man["master_equipment_count"].as_u64(), Some(2));
        assert_eq!(man["relief_devices_count"].as_u64(), Some(1));

        for rel in [
            "schedules/00_drawing_index.csv",
            "schedules/00_drawing_index.json",
            "schedules/01_bom_schedule.csv",
            "schedules/01_bom_schedule.json",
            "schedules/02_instrument_loops.csv",
            "schedules/02_instrument_loops.json",
            "schedules/03_valve_mto.csv",
            "schedules/03_valve_mto.json",
            "schedules/04_utility_and_battery_limits.csv",
            "schedules/04_utility_and_battery_limits.json",
            "schedules/05_line_list.csv",
            "schedules/05_line_list.json",
            "schedules/06_equipment_nozzles.csv",
            "schedules/06_equipment_nozzles.json",
            "schedules/07_piping_specialties.csv",
            "schedules/07_piping_specialties.json",
            "schedules/08_equipment_schedule.csv",
            "schedules/08_equipment_schedule.json",
            "schedules/09_relief_devices.csv",
            "schedules/09_relief_devices.json",
            "audits/03_opc_pairing.json",
            "audits/04_drc_and_spec_audit.json",
            "audits/05_hazop_worksheet.csv",
            "audits/05_hazop_worksheet.json",
        ] {
            assert!(safe_dir.join(rel).is_file(), "missing {rel}");
        }

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 30);
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            let raw = std::fs::read(safe_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and builds the v4 44-file 10-schedule + 3-audit EPC Digital Handover Package via PIDPACKAGE ALLTABS SEED VERIFY STRICT"]
    fn real_multi_tab_pidpackage_v4_ten_schedule_44_file_epc_bundle_and_sha256_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL171_SPPID_PAIR") else {
            println!("OCS_PID_REAL171_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL171_LEGACY_PAIR") else {
            println!("OCS_PID_REAL171_LEGACY_PAIR unset, skipping.");
            return;
        };
        let Ok(seed_spec) = std::env::var("OCS_PID_REAL171_SEED") else {
            println!("OCS_PID_REAL171_SEED unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_REAL171_SYMDIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL171_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real171"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        for &ti in &tab_indices {
            let _ = app.update(Message::TabSwitch(ti));
            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");
        }
        let _ = app.update(Message::TabSwitch(tab_indices[0]));

        let pkg_dir = out_dir.join("pkg_v4_tmp");
        let sym_arg = if sym_dir.trim().is_empty() {
            String::new()
        } else {
            format!(" SYMDIR \"{sym_dir}\"")
        };
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 SEED \"{seed_spec}\"{sym_arg} VERIFY STRICT",
            pkg_dir.display()
        ));

        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert!(man["schema_version"].as_u64().unwrap_or(0) >= 4);
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(man["unsealed_relief_valve_violations"].as_u64(), Some(0));
        assert_eq!(man["sheets_exported"].as_u64(), Some(4));
        assert_eq!(man["pid_files_written"].as_u64(), Some(4));
        assert_eq!(man["pdf_files_plotted"].as_u64(), Some(4));
        assert!(man["master_equipment_count"].as_u64().unwrap_or(0) >= 20);
        assert!(man["relief_devices_count"].as_u64().unwrap_or(0) >= 4);

        let files_arr = man["files"].as_array().unwrap();
        assert!(files_arr.len() >= 44);
        let mut total_bytes = 0u64;
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            total_bytes += b;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let sanitized = json!({
            "source_sha256_prefixes": sha_prefixes,
            "schema_version": man["schema_version"],
            "handover_ready": man["handover_ready"],
            "strict_vendor_passed": man["strict_vendor_passed"],
            "sheets_exported": man["sheets_exported"],
            "pid_files_written": man["pid_files_written"],
            "pdf_files_plotted": man["pdf_files_plotted"],
            "bom_unique_rows": man["bom_unique_rows"],
            "master_equipment_count": man["master_equipment_count"],
            "master_pipelines_count": man["master_pipelines_count"],
            "equipment_nozzles_count": man["equipment_nozzles_count"],
            "total_valves": man["total_valves"],
            "relief_devices_count": man["relief_devices_count"],
            "unsealed_relief_valve_violations": man["unsealed_relief_valve_violations"],
            "piping_specialties_count": man["piping_specialties_count"],
            "instrument_loops_count": man["instrument_loops_count"],
            "battery_limit_interfaces": man["battery_limit_interfaces"],
            "hazop_nodes_count": man["hazop_nodes_count"],
            "opc_reciprocal_pairs": man["opc_reciprocal_pairs"],
            "total_files": files_arr.len(),
            "total_bytes": total_bytes,
            "manifest_sha256_prefix": &expected_man_sha[..12],
        });
        let _ = std::fs::remove_dir_all(&pkg_dir);
        std::fs::write(
            out_dir.join("real171_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidcause_extracts_sis_esd_matrix_links_signal_wired_cross_loop_trips_flags_incomplete_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let tmp = std::env::temp_dir().join("ocs_pidcause_test_172");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1:
        // - V-1721 + P-1721 on 100-PG-17201-CS150 -> OPC to Tab 2
        // - PZT-1721 wired via signal line (layer 12) -> UZ-1720 -> P-1721 (and paired with ESDV-1721 + ZSC-1721 on Tab 2!)
        // - LSHH-1798 on V-1721 with no actuator (missing_actuator)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-172-01.dxf".to_string();
        let (pzt_h, lshh_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-172-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(100.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1721", Vector3::new(95.0, 280.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(176.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(180.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1721", Vector3::new(175.0, 290.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(184.0, 300.0, 0.0),
                Vector3::new(274.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();
            let mut ltxt = Text::with_value("100-PG-17201-CS150", Vector3::new(205.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(280.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-172-02", Vector3::new(270.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();

            // PZT-1721 at (100, 340) -> signal line -> UZ-1720 at (180, 340) -> signal line -> P-1721 at (180, 300)
            let mut pzt = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(100.0, 340.0, 0.0),
            );
            pzt.common.layer = "12".to_string();
            let pzt_h = doc.add_entity(document::EntityType::Insert(pzt)).unwrap();
            let mut pz1 = Text::with_value("PZT", Vector3::new(98.0, 342.0, 0.0)).with_height(2.2);
            pz1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pz1)).unwrap();
            let mut pz2 = Text::with_value("1721", Vector3::new(97.5, 337.5, 0.0)).with_height(2.2);
            pz2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pz2)).unwrap();

            let mut sig0 = Line::from_points(
                Vector3::new(100.0, 312.0, 0.0),
                Vector3::new(100.0, 336.0, 0.0),
            );
            sig0.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Line(sig0)).unwrap();

            let mut sig1 = Line::from_points(
                Vector3::new(104.0, 340.0, 0.0),
                Vector3::new(176.0, 340.0, 0.0),
            );
            sig1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Line(sig1)).unwrap();

            let mut uz = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(180.0, 340.0, 0.0),
            );
            uz.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(uz)).unwrap();
            let mut uz1 = Text::with_value("UZ", Vector3::new(178.5, 342.0, 0.0)).with_height(2.2);
            uz1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(uz1)).unwrap();
            let mut uz2 =
                Text::with_value("1720", Vector3::new(177.5, 337.5, 0.0)).with_height(2.2);
            uz2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(uz2)).unwrap();

            let mut sig2 = Line::from_points(
                Vector3::new(180.0, 336.0, 0.0),
                Vector3::new(180.0, 304.0, 0.0),
            );
            sig2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Line(sig2)).unwrap();

            // Incomplete initiator LSHH-1798 at (100, 460) without any actuator
            let mut lshh = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(100.0, 460.0, 0.0),
            );
            lshh.common.layer = "12".to_string();
            let lshh_h = doc.add_entity(document::EntityType::Insert(lshh)).unwrap();
            let mut ls1 = Text::with_value("LSHH", Vector3::new(97.5, 462.0, 0.0)).with_height(2.2);
            ls1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ls1)).unwrap();
            let mut ls2 = Text::with_value("1798", Vector3::new(97.5, 457.5, 0.0)).with_height(2.2);
            ls2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ls2)).unwrap();

            (pzt_h, lshh_h)
        };

        // Tab 2:
        // - Return OPC -> 100-PG-17201-CS150 -> ESDV-1721 + ZSC-1721
        // - Orphan actuator XV-1799 on 80-PG-17202-CS150 without initiator
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-172-02.dxf".to_string();
        let (esdv_h, xv_orphan_h) = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            let mut tb =
                Text::with_value("DWG-172-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-172-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(177.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-17201-CS150", Vector3::new(125.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut esdv = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 300.0, 0.0));
            esdv.common.layer = "11".to_string();
            let esdv_h = doc.add_entity(document::EntityType::Insert(esdv)).unwrap();
            let mut esdvt =
                Text::with_value("ESDV-1721", Vector3::new(175.0, 292.0, 0.0)).with_height(2.5);
            esdvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(esdvt)).unwrap();

            let mut zsc = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(180.0, 325.0, 0.0),
            );
            zsc.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(zsc)).unwrap();
            let mut zs1 = Text::with_value("ZSC", Vector3::new(178.0, 327.0, 0.0)).with_height(2.2);
            zs1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(zs1)).unwrap();
            let mut zs2 =
                Text::with_value("1721", Vector3::new(177.5, 322.5, 0.0)).with_height(2.2);
            zs2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(zs2)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(183.0, 300.0, 0.0),
                Vector3::new(260.0, 300.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            // Orphan XV-1799 on 80-PG-17202-CS150
            let mut qa = Line::from_points(
                Vector3::new(120.0, 480.0, 0.0),
                Vector3::new(177.0, 480.0, 0.0),
            );
            qa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(qa)).unwrap();
            let mut qla = Text::with_value("80-PG-17202-CS150", Vector3::new(130.0, 483.0, 0.0))
                .with_height(2.5);
            qla.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(qla)).unwrap();

            let mut xvo = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 480.0, 0.0));
            xvo.common.layer = "11".to_string();
            let xv_orphan_h = doc.add_entity(document::EntityType::Insert(xvo)).unwrap();
            let mut xvot =
                Text::with_value("XV-1799", Vector3::new(175.0, 487.0, 0.0)).with_height(2.5);
            xvot.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(xvot)).unwrap();

            (esdv_h, xv_orphan_h)
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let json_p = tmp.join("cause_effect.json");
        let csv_p = tmp.join("cause_effect.csv");
        let _ = app.run_command_line(&format!("PIDCAUSE ALLTABS EXPORT \"{}\"", json_p.display()));
        assert!(
            json_p.is_file(),
            "PIDCAUSE EXPORT must write SIS/ESD Cause & Effect Matrix cause_effect.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDCAUSE SUMMARY ALLTABS EXPORT \"{}\"",
            csv_p.display()
        ));
        assert!(csv_p.is_file());
        let csv_bytes = std::fs::read(&csv_p).unwrap();
        assert!(csv_bytes.starts_with(&[0xEF, 0xBB, 0xBF]));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
        assert_eq!(rep["summary"]["sheets_scanned"].as_u64(), Some(2));
        assert_eq!(rep["summary"]["total_interlocks"].as_u64(), Some(3));
        assert_eq!(rep["summary"]["complete_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["missing_actuator_count"].as_u64(), Some(1));
        assert_eq!(rep["summary"]["orphan_actuator_count"].as_u64(), Some(1));

        let rows = rep["interlocks"].as_array().unwrap();
        let comp = rows
            .iter()
            .find(|r| r["status"].as_str() == Some("complete_interlock"))
            .expect("complete interlock row");
        assert!(comp["cause_tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("1721")));
        assert!(comp["logic_solvers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("1720")));
        let effs = comp["effect_actions"].as_array().unwrap();
        assert!(effs
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("ESDV-1721")));
        assert!(effs
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("P-1721")));
        assert!(comp["feedback_tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("1721")));

        let _ = app.run_command_line("PIDCAUSE COMPLETE ALLTABS");
        assert!(has_sel(&app, tab1, pzt_h) && has_sel(&app, tab2, esdv_h));

        let _ = app.run_command_line("PIDCAUSE INCOMPLETE ALLTABS");
        assert!(has_sel(&app, tab1, lshh_h) && has_sel(&app, tab2, xv_orphan_h));

        let _ = app.run_command_line("PIDCAUSE TAG XV-1799 ALLTABS");
        assert_eq!(app.active_tab, tab2);
        assert!(has_sel(&app, tab2, xv_orphan_h));

        let mut visited_orphan = false;
        for _ in 0..5 {
            let _ = app.run_command_line("PIDCAUSE NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, xv_orphan_h) {
                visited_orphan = true;
                break;
            }
        }
        assert!(visited_orphan, "PIDCAUSE NEXT ALLTABS must visit XV-1799");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDCAUSE ALLTABS SIS/ESD Cause & Effect Matrix extraction and NEXT stepping"]
    fn real_multi_tab_pidcause_sis_esd_matrix_and_next_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL172_SPPID_PAIR") else {
            println!("OCS_PID_REAL172_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL172_LEGACY_PAIR") else {
            println!("OCS_PID_REAL172_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL172_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real172"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let groups = [("SP01_SP02", &sppid_paths), ("LG01_LG02", &legacy_paths)];
        let mut out_groups = Vec::new();

        for (gname, paths) in groups {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in paths.iter().enumerate() {
                let raw_b = std::fs::read(p).unwrap();
                shas.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].tab_title = format!("{gname}_{idx}.dxf");
            }

            let json_p = out_dir.join(format!("{gname}_cause.json"));
            let csv_p = out_dir.join(format!("{gname}_cause.csv"));
            let _ =
                app.run_command_line(&format!("PIDCAUSE ALLTABS EXPORT \"{}\"", json_p.display()));
            let _ = app.run_command_line(&format!(
                "PIDCAUSE SUMMARY ALLTABS EXPORT \"{}\"",
                csv_p.display()
            ));
            assert!(json_p.is_file() && csv_p.is_file());
            let csv_raw = std::fs::read(&csv_p).unwrap();
            assert!(csv_raw.starts_with(&[0xEF, 0xBB, 0xBF]));

            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
            let tot = rep["summary"]["total_interlocks"].as_u64().unwrap_or(0);
            assert!(
                tot >= 2,
                "{gname} must extract >= 2 SIS/ESD interlocks, got {tot}"
            );

            let _ = app.run_command_line("PIDCAUSE NEXT ALLTABS");
            let cur_tab = app.active_tab;
            assert!(
                !app.tabs[cur_tab].scene.selected_entities().is_empty(),
                "PIDCAUSE NEXT ALLTABS must select target interlock entities"
            );

            let _ = std::fs::remove_file(&json_p);
            let _ = std::fs::remove_file(&csv_p);

            out_groups.push(json!({
                "group": gname,
                "sha256_prefixes": shas,
                "total_interlocks": rep["summary"]["total_interlocks"],
                "multi_sheet_interlocks": rep["summary"]["multi_sheet_interlocks"],
                "complete_count": rep["summary"]["complete_count"],
                "missing_actuator_count": rep["summary"]["missing_actuator_count"],
                "orphan_actuator_count": rep["summary"]["orphan_actuator_count"],
                "total_cause_sensors": rep["summary"]["total_cause_sensors"],
                "total_trip_actuators": rep["summary"]["total_trip_actuators"],
            }));
        }

        std::fs::write(
            out_dir.join("real172_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidio_classifies_dcs_sis_local_channels_avoids_ly_lv_double_count_computes_20pct_spare_flags_unconnected_and_steps_camera(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;

        fn has_sel(app: &OpenCADStudio, tab: usize, h: document::Handle) -> bool {
            app.tabs[tab]
                .scene
                .selected_entities()
                .into_iter()
                .any(|(sh, _)| sh == h)
        }

        let tmp = std::env::temp_dir().join("ocs_pidio_test_173");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1:
        // - V-1731 + 100-PG-17301-CS150
        // - DCS loop 1731: LT-1731 (AI) + LICA-1731 (SOFT) + LY-1731 (SOFT, paired with LV!) + LV-1731 (AO)
        // - Local gauge PG-1731 (LOCAL_NONE) on 100-PG-17301-CS150
        // - Floating unconnected PT-1799 (AI, unconnected_field)
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-173-01.dxf".to_string();
        let (lt_h, pg_h, pt_unconn_h) = {
            let doc = &mut app.tabs[tab1].scene.document;
            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);

            let mut tb =
                Text::with_value("DWG-173-01", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(100.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1731", Vector3::new(95.0, 280.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();

            // LT-1731 connected via impulse line to V-1731
            let mut imp = Line::from_points(
                Vector3::new(100.0, 312.0, 0.0),
                Vector3::new(100.0, 336.0, 0.0),
            );
            imp.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Line(imp)).unwrap();
            let mut lt = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(100.0, 340.0, 0.0),
            );
            lt.common.layer = "12".to_string();
            let lt_h = doc.add_entity(document::EntityType::Insert(lt)).unwrap();
            let mut lt1 = Text::with_value("LT", Vector3::new(98.5, 342.0, 0.0)).with_height(2.2);
            lt1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lt1)).unwrap();
            let mut lt2 = Text::with_value("1731", Vector3::new(97.5, 337.5, 0.0)).with_height(2.2);
            lt2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lt2)).unwrap();

            // LICA-1731 (DCS soft controller)
            let mut lica = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(145.0, 340.0, 0.0),
            );
            lica.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lica)).unwrap();
            let mut lc1 =
                Text::with_value("LICA", Vector3::new(142.5, 342.0, 0.0)).with_height(2.2);
            lc1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lc1)).unwrap();
            let mut lc2 =
                Text::with_value("1731", Vector3::new(142.5, 337.5, 0.0)).with_height(2.2);
            lc2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(lc2)).unwrap();

            // Process line 100-PG-17301-CS150 with LY-1731 + LV-1731 + PG-1731
            let mut p1 = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(187.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();
            let mut ltxt = Text::with_value("100-PG-17301-CS150", Vector3::new(120.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut lv = Insert::new("BALL VALVE TYPE 2", Vector3::new(190.0, 300.0, 0.0));
            lv.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(lv)).unwrap();
            let mut lvt =
                Text::with_value("LV-1731", Vector3::new(185.0, 292.0, 0.0)).with_height(2.5);
            lvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(lvt)).unwrap();

            let mut ly = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(190.0, 320.0, 0.0),
            );
            ly.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(ly)).unwrap();
            let mut ly1 = Text::with_value("LY", Vector3::new(188.5, 322.0, 0.0)).with_height(2.2);
            ly1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ly1)).unwrap();
            let mut ly2 =
                Text::with_value("1731", Vector3::new(187.5, 317.5, 0.0)).with_height(2.2);
            ly2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ly2)).unwrap();

            let mut p2 = Line::from_points(
                Vector3::new(193.0, 300.0, 0.0),
                Vector3::new(280.0, 300.0, 0.0),
            );
            p2.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p2)).unwrap();

            let mut pg = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(240.0, 306.0, 0.0),
            );
            pg.common.layer = "12".to_string();
            let pg_h = doc.add_entity(document::EntityType::Insert(pg)).unwrap();
            let mut pg1 = Text::with_value("PG", Vector3::new(238.5, 308.0, 0.0)).with_height(2.2);
            pg1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pg1)).unwrap();
            let mut pg2 =
                Text::with_value("1731", Vector3::new(237.5, 303.5, 0.0)).with_height(2.2);
            pg2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pg2)).unwrap();

            // Unconnected field transmitter PT-1799 far from any pipe/equipment
            let mut ptu = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(500.0, 500.0, 0.0),
            );
            ptu.common.layer = "12".to_string();
            let pt_unconn_h = doc.add_entity(document::EntityType::Insert(ptu)).unwrap();
            let mut pu1 = Text::with_value("PT", Vector3::new(498.5, 502.0, 0.0)).with_height(2.2);
            pu1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pu1)).unwrap();
            let mut pu2 =
                Text::with_value("1799", Vector3::new(497.5, 497.5, 0.0)).with_height(2.2);
            pu2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(pu2)).unwrap();

            (lt_h, pg_h, pt_unconn_h)
        };

        // Tab 2: SIS ESD loop 1732 on 80-PG-17302-CS300: PSHH-1732 (DI) + HSC-1732 (DI) + ZSC-1732 (DI) + ESDV-1732 (DO)
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-173-02.dxf".to_string();
        let esdv_h = {
            let doc = &mut app.tabs[tab2].scene.document;
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);

            let mut tb =
                Text::with_value("DWG-173-02", Vector3::new(1120.0, 24.0, 0.0)).with_height(3.5);
            tb.common.layer = "Labels".to_string();
            doc.add_entity(document::EntityType::Text(tb)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(100.0, 300.0, 0.0),
                Vector3::new(177.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("80-PG-17302-CS300", Vector3::new(115.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut pshh = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(140.0, 306.0, 0.0),
            );
            pshh.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(pshh)).unwrap();
            let mut ps1 =
                Text::with_value("PSHH", Vector3::new(137.5, 308.0, 0.0)).with_height(2.2);
            ps1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ps1)).unwrap();
            let mut ps2 =
                Text::with_value("1732", Vector3::new(137.5, 303.5, 0.0)).with_height(2.2);
            ps2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ps2)).unwrap();

            let mut hsc = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(140.0, 350.0, 0.0),
            );
            hsc.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(hsc)).unwrap();
            let mut hs1 = Text::with_value("HSC", Vector3::new(138.0, 352.0, 0.0)).with_height(2.2);
            hs1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(hs1)).unwrap();
            let mut hs2 =
                Text::with_value("1732", Vector3::new(137.5, 347.5, 0.0)).with_height(2.2);
            hs2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(hs2)).unwrap();

            let mut esdv = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 300.0, 0.0));
            esdv.common.layer = "11".to_string();
            let esdv_h = doc.add_entity(document::EntityType::Insert(esdv)).unwrap();
            let mut esdvt =
                Text::with_value("ESDV-1732", Vector3::new(175.0, 292.0, 0.0)).with_height(2.5);
            esdvt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(esdvt)).unwrap();

            let mut zsc = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(180.0, 306.0, 0.0),
            );
            zsc.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(zsc)).unwrap();
            let mut zs1 = Text::with_value("ZSC", Vector3::new(178.0, 308.0, 0.0)).with_height(2.2);
            zs1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(zs1)).unwrap();
            let mut zs2 =
                Text::with_value("1732", Vector3::new(177.5, 303.5, 0.0)).with_height(2.2);
            zs2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(zs2)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(183.0, 300.0, 0.0),
                Vector3::new(260.0, 300.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            esdv_h
        };

        let _ = app.update(Message::TabSwitch(tab1));
        let json_p = tmp.join("io_list.json");
        let csv_p = tmp.join("io_list.csv");
        let _ = app.run_command_line(&format!("PIDIO ALLTABS EXPORT \"{}\"", json_p.display()));
        assert!(
            json_p.is_file(),
            "PIDIO EXPORT must write Instrument Index & I/O Count Schedule io_list.json"
        );
        let _ = app.run_command_line(&format!(
            "PIDIO SUMMARY ALLTABS EXPORT \"{}\"",
            csv_p.display()
        ));
        assert!(csv_p.is_file());
        let csv_bytes = std::fs::read(&csv_p).unwrap();
        assert!(csv_bytes.starts_with(&[0xEF, 0xBB, 0xBF]));

        let rep: Value = serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
        let s = &rep["summary"];
        assert_eq!(s["sheets_scanned"].as_u64(), Some(2));
        assert_eq!(s["total_items"].as_u64(), Some(10));
        assert_eq!(s["dcs_ai"].as_u64(), Some(2));
        assert_eq!(s["dcs_ao"].as_u64(), Some(1));
        assert_eq!(s["dcs_total_hw"].as_u64(), Some(3));
        assert_eq!(s["dcs_with_20pct_spare"].as_u64(), Some(4));
        assert_eq!(s["sis_di"].as_u64(), Some(3));
        assert_eq!(s["sis_do"].as_u64(), Some(1));
        assert_eq!(s["sis_total_hw"].as_u64(), Some(4));
        assert_eq!(s["sis_with_20pct_spare"].as_u64(), Some(5));
        assert_eq!(s["local_gauges_count"].as_u64(), Some(1));
        assert_eq!(s["soft_functions_count"].as_u64(), Some(2));
        assert_eq!(s["unconnected_field_count"].as_u64(), Some(1));

        let _ = app.run_command_line("PIDIO DCS ALLTABS");
        assert!(has_sel(&app, tab1, lt_h));

        let _ = app.run_command_line("PIDIO LOCAL ALLTABS");
        assert!(has_sel(&app, tab1, pg_h));

        let _ = app.run_command_line("PIDIO UNCONNECTED ALLTABS");
        assert!(has_sel(&app, tab1, pt_unconn_h) && !has_sel(&app, tab1, lt_h));

        let _ = app.run_command_line("PIDIO CHANNEL DO ALLTABS");
        assert!(has_sel(&app, tab2, esdv_h));

        let _ = app.run_command_line("PIDIO TAG ESDV-1732 ALLTABS");
        assert_eq!(app.active_tab, tab2);
        assert!(has_sel(&app, tab2, esdv_h));

        let mut visited_esdv = false;
        for _ in 0..12 {
            let _ = app.run_command_line("PIDIO NEXT ALLTABS");
            if app.active_tab == tab2 && has_sel(&app, tab2, esdv_h) {
                visited_esdv = true;
                break;
            }
        }
        assert!(visited_esdv, "PIDIO NEXT ALLTABS must visit ESDV-1732");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and verifies PIDIO ALLTABS Instrument Index & DCS/SIS I/O sizing and NEXT stepping"]
    fn real_multi_tab_pidio_instrument_index_and_cabinet_sizing_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL173_SPPID_PAIR") else {
            println!("OCS_PID_REAL173_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL173_LEGACY_PAIR") else {
            println!("OCS_PID_REAL173_LEGACY_PAIR unset, skipping.");
            return;
        };
        let out_dir = std::env::var("OCS_PID_REAL173_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real173"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let groups = [("SP01_SP02", &sppid_paths), ("LG01_LG02", &legacy_paths)];
        let mut out_groups = Vec::new();

        for (gname, paths) in groups {
            let mut app = app();
            let mut shas = Vec::new();
            for (idx, p) in paths.iter().enumerate() {
                let raw_b = std::fs::read(p).unwrap();
                shas.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
                if idx > 0 {
                    let _ = app.update(Message::TabNew);
                }
                let ti = app.active_tab;
                app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
                app.tabs[ti].tab_title = format!("{gname}_{idx}.dxf");
            }

            let json_p = out_dir.join(format!("{gname}_io.json"));
            let csv_p = out_dir.join(format!("{gname}_io.csv"));
            let _ = app.run_command_line(&format!("PIDIO ALLTABS EXPORT \"{}\"", json_p.display()));
            let _ = app.run_command_line(&format!(
                "PIDIO SUMMARY ALLTABS EXPORT \"{}\"",
                csv_p.display()
            ));
            assert!(json_p.is_file() && csv_p.is_file());
            let csv_raw = std::fs::read(&csv_p).unwrap();
            assert!(csv_raw.starts_with(&[0xEF, 0xBB, 0xBF]));

            let rep: Value =
                serde_json::from_str(&std::fs::read_to_string(&json_p).unwrap()).unwrap();
            let tot = rep["summary"]["total_items"].as_u64().unwrap_or(0);
            assert!(
                tot >= 20,
                "{gname} must extract >= 20 instruments/actuators, got {tot}"
            );

            let _ = app.run_command_line("PIDIO NEXT ALLTABS");
            let cur_tab = app.active_tab;
            assert!(
                !app.tabs[cur_tab].scene.selected_entities().is_empty(),
                "PIDIO NEXT ALLTABS must select target instrument entities"
            );

            let _ = std::fs::remove_file(&json_p);
            let _ = std::fs::remove_file(&csv_p);

            out_groups.push(json!({
                "group": gname,
                "sha256_prefixes": shas,
                "total_items": rep["summary"]["total_items"],
                "dcs_total_hw": rep["summary"]["dcs_total_hw"],
                "dcs_with_20pct_spare": rep["summary"]["dcs_with_20pct_spare"],
                "sis_total_hw": rep["summary"]["sis_total_hw"],
                "sis_with_20pct_spare": rep["summary"]["sis_with_20pct_spare"],
                "local_gauges_count": rep["summary"]["local_gauges_count"],
                "soft_functions_count": rep["summary"]["soft_functions_count"],
                "unconnected_field_count": rep["summary"]["unconnected_field_count"],
            }));
        }

        std::fs::write(
            out_dir.join("real173_summary.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({ "groups": out_groups })).unwrap()
            ),
        )
        .unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidpackage_v5_emits_eleven_schedules_and_four_audits_blocks_incomplete_sis_and_verifies_34_file_sha256(
    ) {
        use document::entities::{Insert, Line, Text};
        use document::types::Vector3;
        use sha2::{Digest, Sha256};

        let tmp = std::env::temp_dir().join("ocs_pidpackage_v5_test_174");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let mut app = app();

        // Tab 1: DWG-174-01.dxf with P-1741 -> 100-PG-17401-CS150 -> PT-1741 (DCS AI) + LSHH-1742 (SIS DI, missing ESDV initially!) -> OPC to Tab 2
        let tab1 = app.active_tab;
        app.tabs[tab1].tab_title = "DWG-174-01.dxf".to_string();
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "CENTRIFUGAL PUMP", 8.0, 8.0);
            define_sppid_block(doc, "DCS FUNC ACCESS IN PRIME LOC", 8.0, 8.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-174-01", 1100.0, 24.0),
                ("凝析油外输泵及联锁保护流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut pump = Insert::new("CENTRIFUGAL PUMP", Vector3::new(100.0, 300.0, 0.0));
            pump.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(pump)).unwrap();
            let mut pt =
                Text::with_value("P-1741", Vector3::new(95.0, 308.0, 0.0)).with_height(2.5);
            pt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(pt)).unwrap();

            let mut p1 = Line::from_points(
                Vector3::new(104.0, 300.0, 0.0),
                Vector3::new(254.0, 300.0, 0.0),
            );
            p1.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(p1)).unwrap();
            let mut ltxt = Text::with_value("100-PG-17401-CS150", Vector3::new(155.0, 303.0, 0.0))
                .with_height(2.5);
            ltxt.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(ltxt)).unwrap();

            let mut inst = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(180.0, 306.0, 0.0),
            );
            inst.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(inst)).unwrap();
            let mut it1 = Text::with_value("PT", Vector3::new(178.5, 308.0, 0.0)).with_height(2.2);
            it1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it1)).unwrap();
            let mut it2 =
                Text::with_value("1741", Vector3::new(177.5, 303.5, 0.0)).with_height(2.2);
            it2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(it2)).unwrap();

            let mut lshh = Insert::new(
                "DCS FUNC ACCESS IN PRIME LOC",
                Vector3::new(220.0, 306.0, 0.0),
            );
            lshh.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Insert(lshh)).unwrap();
            let mut ls1 =
                Text::with_value("LSHH", Vector3::new(217.5, 308.0, 0.0)).with_height(2.2);
            ls1.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ls1)).unwrap();
            let mut ls2 =
                Text::with_value("1742", Vector3::new(217.5, 303.5, 0.0)).with_height(2.2);
            ls2.common.layer = "12".to_string();
            doc.add_entity(document::EntityType::Text(ls2)).unwrap();

            let mut opc1 = Insert::new("_OFF-UNIT SQ", Vector3::new(260.0, 300.0, 0.0));
            opc1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc1)).unwrap();
            let mut oft1 =
                Text::with_value("DWG-174-02", Vector3::new(250.0, 299.0, 0.0)).with_height(2.5);
            oft1.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft1)).unwrap();
        }

        // Tab 2: DWG-174-02.dxf with return OPC -> 100-PG-17401-CS150 -> Nozzle -> V-1742
        let _ = app.update(Message::TabNew);
        let tab2 = app.active_tab;
        app.tabs[tab2].tab_title = "DWG-174-02.dxf".to_string();
        {
            let doc = &mut app.tabs[tab2].scene.document;
            let mut f1 = Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1189.0, 841.0, 0.0),
            );
            f1.common.layer = "DrawingBorder".to_string();
            doc.add_entity(document::EntityType::Line(f1)).unwrap();

            define_sppid_block(doc, "1TO1PARAMETRIC V DRUM", 12.0, 24.0);
            define_sppid_block(doc, "FLANGED NOZZLE WITH BLIND", 4.0, 4.0);
            define_sppid_block(doc, "BALL VALVE TYPE 2", 6.0, 4.0);
            define_sppid_block(doc, "_OFF-UNIT SQ", 12.0, 8.0);

            for (val, x, y) in [
                ("DWG-174-02", 1100.0, 24.0),
                ("凝析油缓冲罐及紧急切断流程图", 1040.0, 42.0),
            ] {
                let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(3.5);
                t.common.layer = "Labels".to_string();
                doc.add_entity(document::EntityType::Text(t)).unwrap();
            }

            let mut opc2 = Insert::new("_OFF-UNIT SQ", Vector3::new(100.0, 300.0, 0.0));
            opc2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Insert(opc2)).unwrap();
            let mut oft2 =
                Text::with_value("DWG-174-01", Vector3::new(90.0, 299.0, 0.0)).with_height(2.5);
            oft2.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(oft2)).unwrap();

            let mut pa = Line::from_points(
                Vector3::new(106.0, 300.0, 0.0),
                Vector3::new(177.0, 300.0, 0.0),
            );
            pa.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pa)).unwrap();
            let mut la = Text::with_value("100-PG-17401-CS150", Vector3::new(125.0, 303.0, 0.0))
                .with_height(2.5);
            la.common.layer = "11".to_string();
            doc.add_entity(document::EntityType::Text(la)).unwrap();

            let mut pb = Line::from_points(
                Vector3::new(183.0, 300.0, 0.0),
                Vector3::new(238.0, 300.0, 0.0),
            );
            pb.common.layer = "1".to_string();
            doc.add_entity(document::EntityType::Line(pb)).unwrap();

            let mut nz = Insert::new("FLANGED NOZZLE WITH BLIND", Vector3::new(240.0, 300.0, 0.0));
            nz.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(nz)).unwrap();

            let mut drum = Insert::new("1TO1PARAMETRIC V DRUM", Vector3::new(248.0, 300.0, 0.0));
            drum.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Insert(drum)).unwrap();
            let mut dt =
                Text::with_value("V-1742", Vector3::new(244.0, 282.0, 0.0)).with_height(2.5);
            dt.common.layer = "10".to_string();
            doc.add_entity(document::EntityType::Text(dt)).unwrap();
        }

        // Phase A: STRICT gate must block handover_ready when LSHH-1742 lacks a trip actuator (IEC 61511)
        let _ = app.update(Message::TabSwitch(tab1));
        let inc_dir = tmp.join("pkg_v5_incomplete");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 VERIFY STRICT",
            inc_dir.display()
        ));
        let inc_man_p = inc_dir.join("handover_manifest.json");
        assert!(inc_man_p.is_file());
        let inc_man: Value =
            serde_json::from_str(&std::fs::read_to_string(&inc_man_p).unwrap()).unwrap();
        assert_eq!(
            inc_man["schema_version"].as_u64(),
            Some(5),
            "PIDPACKAGE v5 must emit schema_version = 5 with 11 EPC schedules, 4 audits, and IEC 61511 SIS gate"
        );
        assert_eq!(inc_man["incomplete_sis_interlocks"].as_u64(), Some(1));
        assert_eq!(inc_man["handover_ready"].as_bool(), Some(false));

        // Phase B: Add ESDV-1742 on Tab 2 to close the SIS interlock loop and verify full 34-file bundle
        {
            let doc2 = &mut app.tabs[tab2].scene.document;
            let mut esdv = Insert::new("BALL VALVE TYPE 2", Vector3::new(180.0, 300.0, 0.0));
            esdv.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Insert(esdv)).unwrap();
            let mut esdvt =
                Text::with_value("ESDV-1742", Vector3::new(175.0, 306.0, 0.0)).with_height(2.5);
            esdvt.common.layer = "11".to_string();
            doc2.add_entity(document::EntityType::Text(esdvt)).unwrap();
        }

        let ok_dir = tmp.join("pkg_v5_complete");
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 VERIFY STRICT",
            ok_dir.display()
        ));
        let man_p = ok_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert_eq!(man["schema_version"].as_u64(), Some(5));
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["sis_interlocks_count"].as_u64(), Some(1));
        assert_eq!(man["incomplete_sis_interlocks"].as_u64(), Some(0));
        assert_eq!(man["dcs_total_hw_io"].as_u64(), Some(1));
        assert_eq!(man["sis_total_hw_io"].as_u64(), Some(2));

        for rel in [
            "schedules/10_instrument_io_schedule.csv",
            "schedules/10_instrument_io_schedule.json",
            "audits/06_sis_cause_effect_matrix.csv",
            "audits/06_sis_cause_effect_matrix.json",
        ] {
            assert!(ok_dir.join(rel).is_file(), "missing {rel}");
        }

        let files_arr = man["files"].as_array().unwrap();
        assert_eq!(files_arr.len(), 34);
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            let raw = std::fs::read(ok_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "loads 4 real cross-sheet paired SPPID DXFs and native AutoCAD legacy DXFs into OpenCADStudio and builds the v5 48-file 11-schedule + 4-audit EPC Digital Handover Package via PIDPACKAGE ALLTABS SEED VERIFY STRICT"]
    fn real_multi_tab_pidpackage_v5_eleven_schedule_48_file_epc_bundle_and_sha256_audit() {
        use sha2::{Digest, Sha256};

        let Ok(sppid_raw) = std::env::var("OCS_PID_REAL174_SPPID_PAIR") else {
            println!("OCS_PID_REAL174_SPPID_PAIR unset, skipping.");
            return;
        };
        let Ok(legacy_raw) = std::env::var("OCS_PID_REAL174_LEGACY_PAIR") else {
            println!("OCS_PID_REAL174_LEGACY_PAIR unset, skipping.");
            return;
        };
        let Ok(seed_spec) = std::env::var("OCS_PID_REAL174_SEED") else {
            println!("OCS_PID_REAL174_SEED unset, skipping.");
            return;
        };
        let sym_dir = std::env::var("OCS_PID_REAL174_SYMDIR").unwrap_or_default();
        let out_dir = std::env::var("OCS_PID_REAL174_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("ocs_real174"));
        let _ = std::fs::remove_dir_all(&out_dir);
        std::fs::create_dir_all(&out_dir).unwrap();

        let sppid_paths: Vec<std::path::PathBuf> = std::env::split_paths(&sppid_raw)
            .filter(|p| p.is_file())
            .collect();
        let legacy_paths: Vec<std::path::PathBuf> = std::env::split_paths(&legacy_raw)
            .filter(|p| p.is_file())
            .collect();
        assert_eq!(sppid_paths.len(), 2);
        assert_eq!(legacy_paths.len(), 2);

        let all_files = [
            ("SP01", &sppid_paths[0]),
            ("SP02", &sppid_paths[1]),
            ("LG01", &legacy_paths[0]),
            ("LG02", &legacy_paths[1]),
        ];

        let mut app = app();
        let mut tab_indices = Vec::new();
        let mut sha_prefixes = Vec::new();
        for (idx, (cid, p)) in all_files.iter().enumerate() {
            let raw_b = std::fs::read(p).unwrap();
            sha_prefixes.push(format!("{:x}", Sha256::digest(&raw_b))[..12].to_string());
            if idx > 0 {
                let _ = app.update(Message::TabNew);
            }
            let ti = app.active_tab;
            app.tabs[ti].scene.document = crate::io::load_file(p).expect("load DXF");
            app.tabs[ti].tab_title = format!("{cid}.dxf");
            tab_indices.push(ti);
        }

        for &ti in &tab_indices {
            let _ = app.update(Message::TabSwitch(ti));
            let _ = app.run_command_line("PIDPROMOTE");
            let _ = app.run_command_line("PIDREVIEW DEDUP");
        }
        let _ = app.update(Message::TabSwitch(tab_indices[0]));

        let pkg_dir = out_dir.join("pkg_v5_tmp");
        let sym_arg = if sym_dir.trim().is_empty() {
            String::new()
        } else {
            format!(" SYMDIR \"{sym_dir}\"")
        };
        let _ = app.run_command_line(&format!(
            "PIDPACKAGE \"{}\" ALLTABS PLANT P01 SEED \"{seed_spec}\"{sym_arg} VERIFY STRICT",
            pkg_dir.display()
        ));

        let man_p = pkg_dir.join("handover_manifest.json");
        assert!(man_p.is_file());
        let man: Value = serde_json::from_str(&std::fs::read_to_string(&man_p).unwrap()).unwrap();
        assert_eq!(man["schema_version"].as_u64(), Some(5));
        assert_eq!(man["handover_ready"].as_bool(), Some(true));
        assert_eq!(man["strict_vendor_passed"].as_bool(), Some(true));
        assert_eq!(man["drc_total_errors"].as_u64(), Some(0));
        assert_eq!(man["unsealed_relief_valve_violations"].as_u64(), Some(0));
        assert_eq!(man["incomplete_sis_interlocks"].as_u64(), Some(0));
        assert_eq!(man["sheets_exported"].as_u64(), Some(4));
        assert_eq!(man["pid_files_written"].as_u64(), Some(4));
        assert_eq!(man["pdf_files_plotted"].as_u64(), Some(4));
        assert!(man["sis_interlocks_count"].as_u64().unwrap_or(0) >= 15);
        assert!(man["sis_total_hw_io"].as_u64().unwrap_or(0) >= 60);

        let files_arr = man["files"].as_array().unwrap();
        assert_eq!(files_arr.len(), 48);
        let mut total_bytes = 0u64;
        let mut digest_acc = Sha256::new();
        for entry in files_arr {
            let rel = entry["rel_path"].as_str().unwrap();
            assert!(!rel.contains(':') && !rel.contains('\\'));
            let raw = std::fs::read(pkg_dir.join(rel)).unwrap();
            let b = raw.len() as u64;
            total_bytes += b;
            let sha = format!("{:x}", Sha256::digest(&raw));
            assert_eq!(entry["bytes"].as_u64(), Some(b));
            assert_eq!(entry["sha256"].as_str(), Some(sha.as_str()));
            digest_acc.update(format!("{rel}:{b}:{sha}\n").as_bytes());
        }
        let expected_man_sha = format!("{:x}", digest_acc.finalize());
        assert_eq!(
            man["manifest_sha256"].as_str(),
            Some(expected_man_sha.as_str())
        );

        let sanitized = json!({
            "source_sha256_prefixes": sha_prefixes,
            "schema_version": man["schema_version"],
            "handover_ready": man["handover_ready"],
            "strict_vendor_passed": man["strict_vendor_passed"],
            "sheets_exported": man["sheets_exported"],
            "pid_files_written": man["pid_files_written"],
            "pdf_files_plotted": man["pdf_files_plotted"],
            "bom_unique_rows": man["bom_unique_rows"],
            "master_equipment_count": man["master_equipment_count"],
            "master_pipelines_count": man["master_pipelines_count"],
            "equipment_nozzles_count": man["equipment_nozzles_count"],
            "total_valves": man["total_valves"],
            "relief_devices_count": man["relief_devices_count"],
            "sis_interlocks_count": man["sis_interlocks_count"],
            "incomplete_sis_interlocks": man["incomplete_sis_interlocks"],
            "dcs_total_hw_io": man["dcs_total_hw_io"],
            "sis_total_hw_io": man["sis_total_hw_io"],
            "piping_specialties_count": man["piping_specialties_count"],
            "instrument_loops_count": man["instrument_loops_count"],
            "battery_limit_interfaces": man["battery_limit_interfaces"],
            "hazop_nodes_count": man["hazop_nodes_count"],
            "opc_reciprocal_pairs": man["opc_reciprocal_pairs"],
            "total_files": files_arr.len(),
            "total_bytes": total_bytes,
            "manifest_sha256_prefix": &expected_man_sha[..12],
        });
        let _ = std::fs::remove_dir_all(&pkg_dir);
        std::fs::write(
            out_dir.join("real174_summary.json"),
            format!("{}\n", serde_json::to_string_pretty(&sanitized).unwrap()),
        )
        .unwrap();
    }
}

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
    names: &["PIDPROMOTE", "PIDREVIEW", "PIDSAVEPID"]
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
    let Some(author) = PidAuthor::load(document, handle) else {
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
                self.tabs[i].scene.deselect_all();
                if !matched.is_empty() {
                    self.tabs[i].scene.select_entities(&matched);
                }
                self.refresh_properties();
                self.command_line
                    .push_output(&format!("PIDREVIEW {action}: 已选中 {count} 个对象。"));
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
                    "PIDREVIEW: 未知子命令 {other}（支持 SUMMARY / HIGH / MEDIUM / LOW / DUPLICATES / ACCEPT / REJECT）。"
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
}

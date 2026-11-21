//! What a drawing has that the server never took, and the way out of a
//! drawing apart from the room (`SPEC-collab-offline-sync.md` §4.4, §4.5,
//! §5.5): `PIDCOLLAB UNSYNCED [CLEAR]`, `EXPORT <path>` and `DETACH
//! [FORCE]`, the four fields `GET /api/v1/collab/status` says where the
//! drawing stands with, and the collaboration panel's line that says it to
//! the person (§5.6).
//!
//! A drawing's queue is the session's own once the session has read the
//! drawing in (`seeded`), and otherwise what its file brought. Its unsynced
//! list is its file's with what this run added behind it, in the order a
//! save writes them.

use serde_json::{json, Value};

use crate::app::OpenCADStudio;
use crate::pid::collab_state::{self, Diverged};
use crate::ui::window::pid_collab::SyncLine;

/// §4.4's `state` of a step still in the queue.
const QUEUED: &str = "queued";

impl OpenCADStudio {
    /// The body of `GET /api/v1/collab/status`: the session's own
    /// (`State::status_json`), and where the drawing stands with the room
    /// (§4.5) — the session's drawing, or with none settled the one in
    /// front.
    pub(in crate::app) fn collab_status_json(&self) -> Value {
        let mut status = self.collab.status_json();
        let tab = self.status_tab();
        let (align, why) = self.align(tab);
        let (code, message) = why.unzip();
        status["align"] = json!(align);
        status["align_code"] = json!(code);
        status["align_message"] = json!(message);
        status["unsynced_count"] = json!(tab.map_or(0, |i| self.unsynced_list(i).len()));
        status
    }

    /// What the panel says under its banner (§5.6), from what the status
    /// says: why the drawing is apart from the room and how many of its
    /// changes never reached it (`queued_ops` + `unsynced_count`), or else
    /// how many steps wait (`queued_ops`). Nothing with no collaboration
    /// configured.
    pub(in crate::app) fn collab_sync_line(&self) -> Option<SyncLine> {
        if !self.collab.wanted() {
            return None;
        }
        let tab = self.status_tab();
        let steps = self.collab.outbox.len();
        match self.align(tab) {
            ("diverged" | "refused", Some((_, reason))) => Some(SyncLine::Apart {
                reason,
                entries: steps + tab.map_or(0, |i| self.unsynced_list(i).len()),
            }),
            _ if steps > 0 => Some(SyncLine::Waiting {
                connected: self.collab.connected,
                steps,
            }),
            _ => None,
        }
    }

    /// The drawing the status speaks of: the session's, or with none
    /// settled the one in front.
    fn status_tab(&self) -> Option<usize> {
        match self.collab.document {
            Some(id) => self.tabs.iter().position(|tab| tab.id == id),
            None => self.front_tab(),
        }
    }

    /// `PIDCOLLAB UNSYNCED`: the queue and the unsynced list of the drawing
    /// in front, how many of each, and a line for every entry.
    pub(super) fn list_unsynced(&mut self) {
        let Some(i) = self.drawing_for("PIDCOLLAB UNSYNCED") else {
            return;
        };
        let queued = self.queued_entries(i);
        let listed: Vec<Value> = self.unsynced_list(i).iter().map(listed_entry).collect();
        let count = |state: &str| {
            listed
                .iter()
                .filter(|entry| entry["state"] == state)
                .count()
        };
        let refused = count(collab_state::REFUSED);
        let unqueued = count(collab_state::UNQUEUED);
        let mut head = format!(
            "PIDCOLLAB UNSYNCED: 排队 {} 条，被拒 {refused} 条，没排上队 {unqueued} 条",
            queued.len()
        );
        let other = listed.len() - refused - unqueued;
        if other > 0 {
            head.push_str(&format!("，其他 {other} 条"));
        }
        self.command_line.push_info(&head);
        for entry in queued.iter().chain(&listed) {
            self.command_line
                .push_info(&format!("  {}", entry_line(entry)));
        }
    }

    /// `PIDCOLLAB UNSYNCED CLEAR`: the drawing in front's unsynced list
    /// emptied — what its file brought, from the next save on, and what
    /// this run added. The queue is left as it is, and so is the mark edits
    /// no queue took gave the drawing: it is as apart from the room as it
    /// was, and only `DETACH` takes a mark off (§4.1).
    pub(super) fn clear_unsynced(&mut self) {
        let Some(i) = self.drawing_for("PIDCOLLAB UNSYNCED CLEAR") else {
            return;
        };
        let id = self.tabs[i].id;
        let cleared = self.unsynced_list(i).len();
        let mark = self.collab.unqueued_for_save(id).map(|(_, mark)| mark);
        if let Some(mark) = mark {
            self.collab.unqueued_marks.entry(id).or_insert(mark);
        }
        self.collab.unqueued.remove(&id);
        self.collab.refused.remove(&id);
        collab_state::store_unsynced(&mut self.tabs[i].scene.document, &[]);
        let queued = self.queued_entries(i).len();
        self.command_line.push_info(&format!(
            "PIDCOLLAB UNSYNCED CLEAR: 清掉了 {cleared} 条未同步的记录，保存后生效；排队的 {queued} 条不动"
        ));
    }

    /// `PIDCOLLAB EXPORT <path>`: the drawing in front's queue and unsynced
    /// list written to `path` as §4.4 has it. A path that cannot be written
    /// is said, and nothing else is done.
    pub(super) fn export_unsynced(&mut self, typed: &str) {
        let path = typed.trim().trim_matches('"');
        if path.is_empty() {
            self.command_line
                .push_error("PIDCOLLAB EXPORT: 要写成 PIDCOLLAB EXPORT <路径>");
            return;
        }
        let Some(i) = self.drawing_for("PIDCOLLAB EXPORT") else {
            return;
        };
        let export = self.unsynced_export(i);
        let entries = export["entries"].as_array().map_or(0, Vec::len);
        let written = serde_json::to_vec_pretty(&export)
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write(path, bytes).map_err(|error| error.to_string()));
        match written {
            Ok(()) => self
                .command_line
                .push_info(&format!("PIDCOLLAB EXPORT: 导出了 {entries} 条到 {path}")),
            Err(why) => self
                .command_line
                .push_error(&format!("PIDCOLLAB EXPORT: 写不进 {path}：{why}")),
        }
    }

    /// `PIDCOLLAB DETACH [FORCE]`: the drawing in front's three records
    /// taken off, so that it is saved as a drawing never synced (§4.5,
    /// §5.5). They go from the drawing being edited at once, not only from
    /// the copy the next save writes, so that what reads them — the next
    /// join, the status, the list — takes the drawing as never synced from
    /// now on; like the records themselves, that marks nothing changed and
    /// goes on no undo stack.
    ///
    /// Refused against a session the room takes as aligned, and — unless
    /// `force` — while something is queued or listed: what goes with the
    /// records is not to be had back.
    pub(super) fn detach(&mut self, force: bool) {
        let Some(i) = self.drawing_for("PIDCOLLAB DETACH") else {
            return;
        };
        let id = self.tabs[i].id;
        // With no drawing settled for it yet, a session that is up is the
        // drawing in front's.
        let session = self
            .collab
            .document
            .map_or(self.collab.session_id.is_some(), |document| document == id);
        if session && self.align(Some(i)).0 == "ok" {
            self.command_line.push_error(
                "PIDCOLLAB DETACH: 这张图正对着一个对上了的会话，不能去掉同步记录；先断开协同再用",
            );
            return;
        }
        let queued = self.queued_entries(i).len();
        let listed = self.unsynced_list(i).len();
        if (queued > 0 || listed > 0) && !force {
            self.command_line.push_error(&format!(
                "PIDCOLLAB DETACH: 这张图还有排队 {queued} 条、未同步 {listed} 条修改没有到服务器，\
                 去掉记录就找不回了；先用 PIDCOLLAB EXPORT <路径> 导出，再用 PIDCOLLAB DETACH FORCE"
            ));
            return;
        }
        let record = collab_state::load_state(&self.tabs[i].scene.document);
        let sessions = session
            || record.is_some_and(|record| {
                self.collab.document.is_none() && record.drawing_id == self.collab.drawing_id
            });
        Self::without_collab_records(&mut self.tabs[i].scene.document);
        self.collab.forget_drawing(id, sessions);
        self.command_line.push_info(&format!(
            "PIDCOLLAB DETACH: 去掉了这张图的同步记录（连同排队 {queued} 条、未同步 {listed} 条），\
             保存后生效；之后它按没有同步过的图对待"
        ));
    }

    /// The drawing in front, for `command`; `None`, said, when there is
    /// none.
    fn drawing_for(&mut self, command: &str) -> Option<usize> {
        let found = self.front_tab();
        if found.is_none() {
            self.command_line
                .push_error(&format!("{command}: 没有打开的图纸"));
        }
        found
    }

    fn front_tab(&self) -> Option<usize> {
        let i = self.active_tab;
        self.tabs.get(i).filter(|tab| !tab.is_start).map(|_| i)
    }

    /// Whether the session has read tab `i`'s drawing in: its queue and
    /// where it was aligned to are then the session's.
    fn read_in(&self, i: usize) -> bool {
        self.collab.document == Some(self.tabs[i].id) && self.collab.seeded
    }

    /// Where tab `tab`'s drawing stands with the room, as §4.5 names it,
    /// and why when that is not where it should be. The drawing's own state
    /// comes first: one that knows it is apart is `diverged` on any
    /// connection.
    fn align(&self, tab: Option<usize>) -> (&'static str, Option<(String, String)>) {
        if let Some(apart) = tab.and_then(|i| self.diverged_of(i)) {
            return ("diverged", Some((apart.code, apart.message)));
        }
        if self.collab.session_id.is_some() {
            return match &self.collab.not_aligned {
                Some(held) => ("refused", Some((held.code.clone(), held.message.clone()))),
                None => ("ok", None),
            };
        }
        let recorded = tab.is_some_and(|i| {
            collab_state::load_state(&self.tabs[i].scene.document).is_some()
                || (self.collab.document == Some(self.tabs[i].id)
                    && self.collab.had_session
                    && self.collab.repo_id.is_some())
        });
        (if recorded { "offline" } else { "none" }, None)
    }

    /// What tab `i`'s drawing is known to be apart from the room for: the
    /// session's word for its own drawing, else the drawing's record, else
    /// the edits no queue took on it.
    fn diverged_of(&self, i: usize) -> Option<Diverged> {
        let id = self.tabs[i].id;
        (self.collab.document == Some(id))
            .then(|| self.collab.diverged_record())
            .flatten()
            .or_else(|| collab_state::load_diverged(&self.tabs[i].scene.document))
            .or_else(|| self.collab.unqueued_for_save(id).map(|(_, mark)| mark))
    }

    /// The steps of tab `i`'s drawing still to go to the room, as §4.4's
    /// `queued` entries: the session's queue once it has read the drawing
    /// in, else its file's. Those that could not be read are there as they
    /// were found.
    fn queued_entries(&self, i: usize) -> Vec<Value> {
        if self.read_in(i) {
            let ops = self.collab.outbox.iter().map(|entry| {
                queued_entry(
                    &entry.op_id,
                    &entry.label,
                    entry.user_id.as_deref(),
                    entry.queued_at,
                    &entry.payload,
                )
            });
            let unreadable = self.collab.unreadable_queue.iter().map(unreadable_queued);
            return ops.chain(unreadable).collect();
        }
        let read = collab_state::read_queue(&self.tabs[i].scene.document);
        let ops = read.ops.iter().map(|op| {
            queued_entry(
                &op.op_id,
                &op.label,
                op.user_id.as_deref(),
                op.queued_at,
                &op.payload,
            )
        });
        ops.chain(read.unreadable.iter().map(unreadable_queued))
            .collect()
    }

    /// Tab `i`'s drawing's unsynced list: its file's, then what this run
    /// added — edits no queue took, and restored steps the room refused.
    fn unsynced_list(&self, i: usize) -> Vec<Value> {
        let id = self.tabs[i].id;
        let mut list = collab_state::read_unsynced(&self.tabs[i].scene.document);
        list.extend(self.collab.unqueued.get(&id).into_iter().flatten().cloned());
        list.extend(self.collab.refused.get(&id).into_iter().flatten().cloned());
        list
    }

    /// §4.4's file for tab `i`'s drawing. Where it was aligned to is the
    /// session's word once the session has read it in, else its record's.
    fn unsynced_export(&self, i: usize) -> Value {
        let (drawing_id, repo_id, revision) = if self.read_in(i) {
            let revision = self
                .collab
                .repo_id
                .as_ref()
                .map(|_| self.collab.base_revision);
            (
                Some(self.collab.drawing_id.clone()),
                self.collab.repo_id.clone(),
                revision,
            )
        } else {
            match collab_state::load_state(&self.tabs[i].scene.document) {
                Some(record) => (
                    Some(record.drawing_id),
                    Some(record.repo_id),
                    Some(record.revision),
                ),
                None => (None, None, None),
            }
        };
        let diverged = self
            .diverged_of(i)
            .map(|apart| json!({"code": apart.code, "message": apart.message, "at": apart.at}));
        let entries: Vec<Value> = self
            .queued_entries(i)
            .into_iter()
            .chain(self.unsynced_list(i).iter().map(listed_entry))
            .collect();
        json!({
            "version": 1,
            "kind": "collab_unsynced",
            "drawing_id": drawing_id,
            "repo_id": repo_id,
            "revision": revision,
            "exported_at": ocs_collab::ids::now_unix(),
            "diverged": diverged,
            "entries": entries,
        })
    }
}

/// A step of the queue as §4.4's entry.
fn queued_entry(
    op_id: &str,
    label: &str,
    user_id: Option<&str>,
    at: Option<i64>,
    payload: &Value,
) -> Value {
    json!({
        "state": QUEUED,
        "op_id": op_id,
        "label": label,
        "user_id": user_id,
        "at": at,
        "changes": changes_of(payload),
    })
}

/// An entry of the queue that could not be read, as it was found.
fn unreadable_queued(raw: &Value) -> Value {
    json!({"state": QUEUED, "raw": raw, "changes": []})
}

/// An entry of the unsynced list as §4.4's entry: what the drawing keeps
/// of it, its payload laid out as `changes`. One that could not be read is
/// there as it was found, with no state.
fn listed_entry(kept: &Value) -> Value {
    let Some(fields) = kept.as_object() else {
        return json!({"raw": kept, "changes": []});
    };
    let mut entry = fields.clone();
    let payload = entry.remove("payload").unwrap_or_default();
    entry.insert("changes".to_owned(), Value::Array(changes_of(&payload)));
    Value::Object(entry)
}

/// Every change a payload carries, in the shape the protocol states them:
/// its own body first, then the ones riding along in its `changes`
/// (`broadcast::payload_for`).
fn changes_of(payload: &Value) -> Vec<Value> {
    let Some(body) = payload.as_object() else {
        return Vec::new();
    };
    let mut first = body.clone();
    let rest = match first.remove("changes") {
        Some(Value::Array(rest)) => rest,
        _ => Vec::new(),
    };
    std::iter::once(Value::Object(first)).chain(rest).collect()
}

/// A line of `PIDCOLLAB UNSYNCED` for an entry in §4.4's shape.
fn entry_line(entry: &Value) -> String {
    let state = match entry["state"].as_str() {
        Some(QUEUED) => "排队",
        Some(collab_state::REFUSED) => "被拒",
        Some(collab_state::UNQUEUED) => "没排上队",
        _ => "其他",
    };
    if entry.get("raw").is_some() {
        return format!("{state} 读不出的一条");
    }
    let mut line = format!("{state} {}", entry["label"].as_str().unwrap_or_default());
    if let Some(op_id) = entry["op_id"].as_str() {
        line.push_str(&format!(" {op_id}"));
    }
    if let Some(user) = entry["user_id"].as_str() {
        line.push_str(&format!("（{user}）"));
    }
    let message = entry["message"]
        .as_str()
        .filter(|message| !message.is_empty());
    match (entry["code"].as_str(), message) {
        (Some(code), Some(message)) => line.push_str(&format!("：{code} {message}")),
        (Some(code), None) => line.push_str(&format!("：{code}")),
        (None, Some(message)) => line.push_str(&format!("：{message}")),
        (None, None) => {}
    }
    match entry["rolled_back"].as_bool() {
        Some(true) => line.push_str("，已撤回"),
        Some(false) => line.push_str("，没有撤回"),
        None => {}
    }
    line
}

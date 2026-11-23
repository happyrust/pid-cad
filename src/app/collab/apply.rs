//! App-side glue for change broadcast: the outbound hooks every edit passes
//! through, the inbound fold that writes a peer's changes onto the open
//! drawing, and the read-only gate that keeps a downgraded session from
//! writing at all.
//!
//! The pure halves live in [`super::broadcast`]; this file only wires them to
//! `OpenCADStudio` and to the scene's redraw bookkeeping.

use crate::app::OpenCADStudio;
use crate::pid::collab_state::{REMOTE_UNAPPLIED, ROLLBACK_FAILED};

use super::broadcast;

/// What an undo or redo on the session's drawing keeps from before its steps
/// are applied, for `replay_remote_after`.
pub(in crate::app) struct HistoryGuard {
    /// The lowest `remote_mark` among the steps: the peers' changes later
    /// than this are the ones the steps' images do not hold.
    mark: u64,
    /// Every entity the steps or those later changes touch, as it was.
    before: Vec<(
        document::Handle,
        Option<std::sync::Arc<document::EntityType>>,
    )>,
    /// The properties of those of `before` whose record only points at them
    /// (`PidAuthor::store`), read while the drawing still kept them as they
    /// were: a step puts the drawing's records back whole, and the peers'
    /// later changes are then written over that.
    kept: Vec<(document::Handle, String)>,
    /// Where the drawing gave handles out from before the steps are
    /// applied, with its blocks and the applications its entities carry
    /// data for. A step that changed the drawing's structure puts all three
    /// back as they were when it was made; the entities the peers have made
    /// since are in no step's image and stay, so none of the three may go
    /// back behind them (`keep_structure_ahead_of_peers`).
    next_handle: u64,
    blocks: document::tables::Table<document::tables::BlockRecord>,
    app_ids: document::tables::Table<document::tables::AppId>,
}

impl OpenCADStudio {
    /// Broadcast the semantic content of one undo step to the room.
    ///
    /// Called from `push_undo_entry`, the single convergence point of every
    /// edit (`architecture.md` §5.3), so no per-command hook can be missed.
    pub(in crate::app) fn broadcast_history_delta(
        &mut self,
        i: usize,
        delta: &crate::app::tab_document::DeltaSnapshot,
    ) {
        // The step has been made: the drawing is as it left it, which is
        // what says what a run it touched is like now. What the drawing's
        // records held before it, the step keeps.
        let document = &self.tabs[i].scene.document;
        let changes = broadcast::collect_step_changes(
            document,
            &delta.entities,
            &broadcast::Left::Records(delta.records_across(document)),
        );
        self.broadcast_changes(
            i,
            changes,
            vec![delta.collab_step],
            true,
            delta.label.as_str(),
        );
    }

    /// The tab holding the drawing the session speaks for, if it is open.
    ///
    /// Which tab that is gets settled the first time the session needs a
    /// document — to send an edit, or to take a peer's. An editor started on
    /// a file has a session for that file: it is the tab showing it, and
    /// until the file has loaded there is none — not the drawing that
    /// happens to be in front meanwhile. Started on no file, it is the
    /// drawing in front at that moment. From then on it is that tab whatever
    /// is in front, until the tab is closed; a session whose drawing was
    /// closed has no document, and says so rather than move on to the next
    /// one.
    ///
    /// A drawing whose sync record names another drawing than the session's
    /// is not it (`record_allows`), and leaves the session without one.
    pub(in crate::app) fn collab_tab(&mut self) -> Option<usize> {
        if let Some(id) = self.collab.document {
            let i = self.tabs.iter().position(|tab| tab.id == id)?;
            return self.record_allows(i).then_some(i);
        }
        let i = match self.collab.document_path.clone() {
            Some(path) => self.tab_showing(&path)?,
            None => self.active_tab,
        };
        if self.tabs.get(i).is_some_and(|tab| !tab.is_start) && self.record_allows(i) {
            self.collab.document = Some(self.tabs[i].id);
            return Some(i);
        }
        None
    }

    /// Whether tab `i`'s drawing may be the session's, by what its sync
    /// record names (`SPEC-collab-offline-sync.md` §5.2, step one). Read
    /// once for a tab, and only with collaboration configured. No record,
    /// or one that names the session's drawing, allows it. One that names
    /// another: with no drawing number given for this run and no session up
    /// yet, the session takes the record's — over the settings file's and
    /// the file name's; otherwise the drawing is not the session's, the
    /// command line says whose it is, and its record is left as it was.
    fn record_allows(&mut self, i: usize) -> bool {
        let id = self.tabs[i].id;
        if self.collab.foreign_tab == Some(id) {
            return false;
        }
        if self.collab.record_read == Some(id) || !self.collab.wanted() {
            return true;
        }
        self.collab.record_read = Some(id);
        let Some(record) = crate::pid::collab_state::load_state(&self.tabs[i].scene.document)
        else {
            return true;
        };
        if record.drawing_id == self.collab.drawing_id {
            return true;
        }
        let session_up = self.collab.had_session || self.collab.session_id.is_some();
        if !self.collab.drawing_given && !session_up {
            self.command_line.push_info(&format!(
                "协同：本次会话的图号按这份图的同步记录取 {}",
                record.drawing_id
            ));
            self.collab.drawing_id = record.drawing_id;
            #[cfg(not(target_arch = "wasm32"))]
            self.collab.sync_client_config();
            return true;
        }
        let said = format!(
            "协同：这份图记着它属于 {}，本次会话是 {}：会话不在它上面起，图里的记录不动",
            record.drawing_id, self.collab.drawing_id
        );
        self.command_line.push_warning(&said);
        self.collab.last_error = Some(said);
        self.collab.foreign_tab = Some(id);
        if self.collab.document == Some(id) {
            self.collab.document = None;
        }
        false
    }

    /// The drawing of this session was closed: retire it. The transport
    /// sends `close`, the server ends the session and releases its claims,
    /// and this client stays disconnected until a drawing is opened (#96).
    pub(in crate::app) fn end_collab_if_drawing_closed(&mut self, tab: u64) {
        let session_drawing = match self.collab.document {
            Some(open) => open == tab,
            None => {
                self.collab.had_session
                    && self.tabs.iter().filter(|open| !open.is_start).count() == 1
                    && self.tabs.iter().any(|open| open.id == tab && !open.is_start)
            }
        };
        if !session_drawing {
            return;
        }
        if !self.collab.had_session && self.collab.session_id.is_none() && !self.collab.connected
        {
            return;
        }
        self.collab.document = None;
        self.collab.held_closed = true;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(link) = &self.collab.link {
                link.request_hold();
            } else {
                super::transport::request_hold();
            }
        }
        if self.collab.connected || self.collab.session_id.is_some() {
            self.collab.apply(super::CollabEvent::Disconnected {
                reason: "图纸已关闭，编辑会话已结束".to_owned(),
                will_retry: false,
            });
        }
    }

    /// A drawing was opened after its session had been ended by a close.
    /// The transport may join again (#96).
    pub(in crate::app) fn resume_collab_after_drawing_opened(&mut self) {
        if !self.collab.held_closed {
            return;
        }
        self.collab.held_closed = false;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(link) = &self.collab.link {
                link.release_hold();
            } else {
                super::transport::release_hold();
            }
        }
    }

    /// Whether tab `i` holds the session's drawing, settling it on `i` when
    /// the session has no document yet and `i` is the drawing in front.
    pub(in crate::app) fn is_collab_tab(&mut self, i: usize) -> bool {
        self.collab_tab() == Some(i)
    }

    /// What the tabs are — which they are, and the file each shows — as one
    /// number: how `answer_collab_tags` tells whether they have changed
    /// since it last looked for the tab showing the session's file.
    fn collab_tabs_print(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut print = std::collections::hash_map::DefaultHasher::new();
        for tab in &self.tabs {
            tab.id.hash(&mut print);
            tab.current_path.hash(&mut print);
        }
        print.finish()
    }

    /// Answer the transport's question about the tags the session's drawing
    /// carries (`CollabEvent::TagsWanted`), if it can be answered now. It is
    /// gone back to after every message until it has been (`update`).
    ///
    /// The answer is of the session's drawing, whichever drawing is in
    /// front:
    ///
    /// * it is there — its tags, as the room last heard of them
    ///   (`broadcast::existing_tags`). The answer is of the drawing the
    ///   `hello` speaks for: changes of the room's that were waiting for
    ///   the drawing are written to it first, and a step of this person's
    ///   that is still open on it is waited for;
    /// * it was closed — none. A session whose drawing is gone speaks for no
    ///   drawing, and does not come by another;
    /// * the drawing it was taken to be turned out to be another's, by its
    ///   sync record (`record_allows`) — as if it had none yet;
    /// * the editor was started on a file that has no tab yet — nothing, as
    ///   long as the launch is still opening what it was started on: the
    ///   dial waits for the drawing (as long as `transport::TAGS_WAIT`).
    ///   Once the launch is through and the file has not come, the transport
    ///   is told not to wait;
    /// * started on no file, the session has no drawing until it is first
    ///   used, and being asked is not a use: the transport is told not to
    ///   wait.
    ///
    /// Told not to wait, the transport says `hello` without the tags. They
    /// are said under the same number once the session has come by its
    /// drawing, and the transport joins again to tell the server.
    pub(in crate::app) fn answer_collab_tags(&mut self) {
        enum Drawing {
            Here(usize),
            Closed,
            NotYet,
        }
        let Some(mut asked) = self.collab.tags_wanted else {
            return;
        };
        let drawing = if self.collab.document.is_some() {
            match self.collab_tab() {
                Some(i) => Drawing::Here(i),
                // Its record named another drawing (`record_allows`): the
                // session has none yet.
                None if self.collab.document.is_none() => Drawing::NotYet,
                None => Drawing::Closed,
            }
        } else if self.collab.document_path.is_some() {
            // Finding the tab that shows a file asks the file system: only
            // when the tabs are not what they were at the last look.
            let print = self.collab_tabs_print();
            let found = if asked.looked == Some(print) {
                None
            } else {
                asked.looked = Some(print);
                self.collab_tab()
            };
            let launching = self.opening.is_some()
                || !self.pending_opens.is_empty()
                || !self.collab.opens_settled;
            match found {
                Some(i) => Drawing::Here(i),
                None if launching => {
                    self.collab.tags_wanted = Some(asked);
                    return;
                }
                None => Drawing::NotYet,
            }
        } else {
            Drawing::NotYet
        };
        match drawing {
            Drawing::Here(i) => {
                self.collab.tags_wanted = Some(asked);
                if !self.collab.pending_remote.is_empty() {
                    // What the room did while this drawing was not there to
                    // take it is written now, as on any frame — not left to
                    // the next one, which may be long in coming while a
                    // `hello` is held for this answer. A question that had
                    // been put off is answered on the way, before the
                    // writing (`apply_remote_changes`).
                    self.apply_remote_changes();
                    self.sync_claim_colors();
                    self.refresh_collab_view();
                    if self.collab.tags_wanted.is_none() {
                        return;
                    }
                }
                let step_open = self.tabs[i].history.pending.is_some();
                if step_open || !self.collab.pending_remote.is_empty() {
                    return;
                }
                // What the file was saved aligned to (`REQ-COLL-015`): it
                // seeds the baseline once — so edits made while this copy
                // was closed queue like a session's leftovers — and goes
                // back as the `hello`'s claim. Seeded ahead of the tags: a
                // symbol a step of the file's queue placed is not the
                // room's yet, and is not listed.
                self.seed_collab_from_file(i);
                self.collab.tags_wanted = None;
                // A queue that is not this person's to send: the `hello`
                // claims nothing and lists nothing.
                if let Some(apart) = self.collab.judge_queue() {
                    self.collab
                        .say_tags(asked.ask, super::TagsSaid::Apart(apart));
                    return;
                }
                let tags = broadcast::existing_tags(
                    &self.tabs[i].scene.document,
                    self.collab.outbox.iter().map(|entry| &entry.payload),
                );
                // The claim is the file's own word: on a first dial it is
                // all this copy has; afterwards `seen` is fresher and the
                // transport prefers it.
                let claim = crate::pid::collab_state::load_state(&self.tabs[i].scene.document)
                    .filter(|record| record.drawing_id == self.collab.drawing_id)
                    .map(|record| super::FileClaim {
                        drawing_id: record.drawing_id,
                        repo_id: record.repo_id,
                        revision: record.revision,
                    });
                let said = if self.collab.detached {
                    super::TagsSaid::Detached(tags)
                } else {
                    super::TagsSaid::Tags(tags, claim)
                };
                self.collab.say_tags(asked.ask, said);
            }
            Drawing::Closed => {
                self.collab.tags_wanted = None;
                // The queue outlives the drawing's tab, and is judged the
                // same without it.
                let said = match self.collab.judge_queue() {
                    Some(apart) => super::TagsSaid::Apart(apart),
                    None if self.collab.detached => super::TagsSaid::Detached(Vec::new()),
                    None => super::TagsSaid::Tags(Vec::new(), None),
                };
                self.collab.say_tags(asked.ask, said);
            }
            Drawing::NotYet => {
                if !asked.put_off {
                    asked.put_off = true;
                    self.collab.say_tags(asked.ask, super::TagsSaid::NotYet);
                }
                self.collab.tags_wanted = Some(asked);
            }
        }
    }

    /// Seed the baseline once from what tab `i`'s file was saved aligned to
    /// (`REQ-COLL-015`), including the ops the room never answered before
    /// the copy was closed, and whether it knows it is apart from the room
    /// (`diverged`). Once only, for the session's drawing: a drawing with
    /// no record has nothing to seed. One read in after a session came up
    /// without it keeps what its record says, and the connection is held
    /// apart (`State::seed_file_state`).
    fn seed_collab_from_file(&mut self, i: usize) {
        if self.collab.seeded {
            return;
        }
        let Some(record) = crate::pid::collab_state::load_state(&self.tabs[i].scene.document)
        else {
            self.collab.seeded = true;
            return;
        };
        // A record for another drawing says nothing of this session's.
        if record.drawing_id != self.collab.drawing_id {
            return;
        }
        let claim = super::FileClaim {
            drawing_id: record.drawing_id,
            repo_id: record.repo_id,
            revision: record.revision,
        };
        let queue = crate::pid::collab_state::read_queue(&self.tabs[i].scene.document);
        let ops = queue
            .ops
            .into_iter()
            .map(|queued| super::Outgoing {
                op_id: queued.op_id,
                payload: queued.payload,
                base_revision: queued.base_revision,
                steps: Vec::new(),
                on: queued.on,
                sent: false,
                label: queued.label,
                restored: true,
                user_id: queued.user_id,
                queued_at: queued.queued_at,
                inverse: queued.inverse,
                inverse_read: true,
            })
            .collect();
        let diverged = crate::pid::collab_state::load_diverged(&self.tabs[i].scene.document);
        if self.collab.seed_file_state(&claim, ops, queue.unreadable) {
            self.collab.saved_diverged = diverged;
            if let Some(late) = &self.collab.opened_late {
                self.command_line.push_warning(&late.said());
            }
        }
    }

    /// Queue one `op` carrying the `changes` made on tab `i`, or stay quiet.
    /// Six things silence it:
    ///
    /// * there has never been a session — the offline-first invariant
    ///   (`VAL-CLI-017`): a client that has not joined yet has nobody to
    ///   tell, and when it joins its drawing is taken to be what the room
    ///   has;
    /// * the changes come from a remote op being folded in
    ///   (`applying_remote`), which must not echo back to the peer that sent
    ///   it;
    /// * the step is being taken back because the server refused it
    ///   (`rolling_back`): the room never had it;
    /// * the step is being taken back to read what takes a queued op back
    ///   out (`read_queued_inverses`): what it says is kept for that op;
    /// * tab `i` is not the session's drawing: an edit to another sheet is
    ///   that sheet's own, and would reach the room as a change to this one;
    /// * the session is read-only, which may not write. The command gate
    ///   refuses a write before it happens, so a change that got here anyway
    ///   came in by a door the gate does not watch; it stays in this drawing
    ///   only, and the command line says so.
    ///
    /// Of these, an edit to a drawing with a sync record that no queue takes
    /// — there is no session to be had, the drawing is not the session's,
    /// the session may not write, or the queue would not take it — is kept
    /// on the drawing's unsynced list (`keep_unqueued`).
    ///
    /// Everything handed over in one call goes out as one op, which the server
    /// takes or refuses as a whole. `steps` are the undo steps it speaks
    /// for and `on` whether it put them on the drawing or took them off,
    /// which is how a refused op finds its step again; `label` is what the
    /// step is called.
    ///
    /// With the session down (after there has been one) the op waits in the
    /// queue and goes out when a session is up again (`VAL-CROSS-008`) —
    /// it used to be dropped, and the step stayed in this drawing only.
    pub(in crate::app) fn broadcast_changes(
        &mut self,
        i: usize,
        changes: Vec<broadcast::Change>,
        steps: Vec<u64>,
        on: bool,
        label: &str,
    ) {
        if self.applying_remote {
            return;
        }
        // Ahead of `rolling_back`, which is up while the steps are read.
        if let Some(read) = self.collab.reading_inverse.as_mut() {
            read.extend(changes);
            return;
        }
        if self.collab.rolling_back {
            return;
        }
        if !self.collab.connected && !self.collab.had_session {
            // A copy the file says was in a room still queues
            // (`REQ-COLL-015`): seed its baseline first, so this edit is a
            // session's leftover, not a stranger's. With no collaboration
            // configured no drawing is the session's.
            let wanted = self.collab.wanted();
            if wanted && self.is_collab_tab(i) {
                self.seed_collab_from_file(i);
            }
            if !self.collab.had_session {
                let why = if wanted {
                    "不是本次会话的图"
                } else {
                    "本次运行没有配置协同"
                };
                self.keep_unqueued(i, &changes, label, why);
                return;
            }
        }
        let Some(payload) = broadcast::payload_for(&changes) else {
            return;
        };
        if !self.is_collab_tab(i) {
            self.keep_unqueued(i, &changes, label, "不是本次会话的图");
            return;
        }
        if self.collab.readonly {
            self.command_line.push_warning(&format!(
                "协同：当前会话只读，本地的 {} 项变更没有广播，本图与其他人的图已不一致",
                changes.len()
            ));
            self.keep_unqueued(i, &changes, label, "当前会话只读");
            return;
        }
        match self.collab.queue_op(payload, steps, on, label) {
            Ok(super::Queued::Sent) => {
                self.command_line
                    .push_info(&format!("协同：已广播 {} 项变更", changes.len()));
            }
            Ok(super::Queued::Behind(ahead)) => {
                self.command_line.push_info(&format!(
                    "协同：{} 项变更已排队，前面还有 {ahead} 步在等服务器答复",
                    changes.len()
                ));
            }
            Ok(super::Queued::Offline(waiting)) => {
                self.command_line.push_warning(&format!(
                    "协同：未连接服务器，这一步已排队（共 {waiting} 步），重新连上后提交"
                ));
            }
            Ok(super::Queued::NotAligned(waiting)) => {
                self.command_line.push_warning(&format!(
                    "协同：这次连接与房间对不上，这一步已排队（共 {waiting} 步），不提交"
                ));
            }
            Err(reason) => {
                self.command_line
                    .push_warning(&format!("协同：变更未广播（{reason}）"));
                self.keep_unqueued(i, &changes, label, &reason);
            }
        }
    }

    /// Keep `changes` made on tab `i` that no queue took — `why` says why —
    /// on the drawing's unsynced list, when the drawing has a sync record:
    /// a save writes them there and marks the drawing `UNQUEUED_EDITS`
    /// (`SPEC-collab-offline-sync.md` §4.1, §5.1). The room will never see
    /// them, and the drawing is no longer what it has. A drawing with no
    /// record was never in a room, and changes that say nothing of the
    /// drawing's objects (a move) are none of the room's: neither is kept.
    /// The command line says so the first time for each drawing.
    fn keep_unqueued(&mut self, i: usize, changes: &[broadcast::Change], label: &str, why: &str) {
        let Some(payload) = broadcast::payload_for(changes) else {
            return;
        };
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        let id = tab.id;
        let session_drawing = self.collab.document == Some(id)
            && self.collab.had_session
            && self.collab.repo_id.is_some();
        let record = crate::pid::collab_state::load_state(&tab.scene.document);
        if record.is_none() && !session_drawing {
            return;
        }
        let user = &self.collab.user_id;
        let entry = crate::pid::collab_state::Unsynced {
            state: crate::pid::collab_state::UNQUEUED.to_owned(),
            op_id: None,
            label: label.to_owned(),
            payload,
            user_id: (!user.is_empty()).then(|| user.clone()),
            at: ocs_collab::ids::now_unix(),
            code: None,
            message: why.to_owned(),
            rolled_back: None,
        };
        let Ok(entry) = serde_json::to_value(entry) else {
            return;
        };
        let kept = self.collab.unqueued.entry(id).or_default();
        kept.push(entry);
        if kept.len() == 1 {
            self.command_line.push_warning(&format!(
                "协同：这份图带着同步记录，这一步没有排上队（{why}），记进了未同步清单；\
                 保存之后这份图不能再与服务器同步"
            ));
        }
    }

    /// Take back the step of every op the server refused whole, say so, and
    /// let the queue move again (`VAL-CROSS-008`; plan §2.5).
    ///
    /// Called right after the session's state has taken an event, like
    /// `apply_remote_changes`. A refused step used to stay in this drawing,
    /// with a line on the command line and the two drawings different from
    /// then on.
    pub(in crate::app) fn settle_rejected_ops(&mut self) {
        // What the server said of an op whose step stays: in the words such
        // a refusal has always been reported in.
        for (code, message) in std::mem::take(&mut self.collab.refusals_to_say) {
            self.command_line
                .push_warning(&format!("协同：服务器拒绝了请求（{code}：{message}）"));
        }
        if self.collab.rejections.is_empty() {
            return;
        }
        for rejection in std::mem::take(&mut self.collab.rejections) {
            self.take_back_rejected(rejection);
        }
        self.collab.release_hold();
    }

    /// Put the drawing back to what the room has after `rejection`: the
    /// step it spoke for is taken off again (or, for a step that was taken
    /// back, put on again), together with whatever this person did on top
    /// of it — none of which the room has seen, the ops going out one at a
    /// time. Those steps land on the redo (or undo) stack: they can be done
    /// again, as new ops.
    ///
    /// What the queue holds behind the refused op is gone through in the
    /// order it was made (`collab::Refused::account`): an op that moves the refused
    /// steps again, or was made on top of them, is not sent — the room has
    /// nothing of them to change; an op made when the refused steps were
    /// out of the drawing again is this person's own business and goes out
    /// as it would have.
    fn take_back_rejected(&mut self, rejection: super::Rejection) {
        let super::Rejection {
            entry,
            code,
            message,
        } = rejection;
        let tab = self.collab_tab();
        if let Some(i) = tab {
            // A step still open is this person's too, made on top of the
            // rest: it closes here, as it does when a peer's change
            // arrives, and goes back with the refused one if it is above it.
            self.finish_pending_history(i);
        }
        // An op restored from the file has no undo step to take back, only
        // its inverse.
        if let (Some(i), true) = (tab, entry.restored) {
            self.take_back_restored(i, entry, &code, &message);
            return;
        }
        let account = super::Refused::account(&entry, &self.collab.outbox);
        let (moved, outcome): (Vec<u64>, &str) = match tab {
            None => (Vec::new(), "协同的图纸已不在，没有可撤回的"),
            Some(_) if entry.steps.is_empty() => (Vec::new(), "它不对应撤销栈里的一步，没有撤回"),
            Some(_) if account.mixed => {
                self.collab.mark_diverged(
                    ROLLBACK_FAILED,
                    format!(
                        "服务器拒绝了「{}」（{code}），而它之后有一次撤销或重做把它和房间已有的步骤一起动了，\
                         无法只撤回被拒的部分",
                        entry.label
                    ),
                );
                (Vec::new(), "无法只撤回被拒的部分，本图与房间已不一致")
            }
            // This person took it back (or put it on again) themselves
            // meanwhile: the drawing is what the room has already.
            Some(_) if account.in_effect.is_empty() => {
                (Vec::new(), "本图已经是房间里的样子，不需要再动")
            }
            Some(i) => match self.take_back_steps(i, entry.on, &account.in_effect) {
                Some(steps) => (steps, if entry.on { "已撤回" } else { "已恢复" }),
                None => {
                    self.collab.mark_diverged(
                        ROLLBACK_FAILED,
                        format!(
                            "服务器拒绝了「{}」（{code}），而这一步已不在撤销栈里，无法撤回",
                            entry.label
                        ),
                    );
                    (
                        Vec::new(),
                        "这一步已不在撤销栈里，无法撤回，本图与房间已不一致",
                    )
                }
            },
        };
        // What the queue still holds for the refused steps and for what was
        // made on top of them is not sent.
        let before = self.collab.outbox.len();
        if !account.mixed {
            self.collab
                .outbox
                .retain(|queued| queued.sent || !account.void.contains(&queued.op_id));
        }
        let dropped = before - self.collab.outbox.len();
        let also = moved.len().saturating_sub(entry.steps.len().max(1));
        let label = if entry.label.is_empty() { "这一步" } else { entry.label.as_str() };
        let mut text = format!("协同：服务器没有接受「{label}」（{code}：{message}）：{outcome}");
        if also > 0 {
            text.push_str(&format!("，连同它之后的 {also} 步（可以重做）"));
        }
        if dropped > 0 {
            text.push_str(&format!("；队列里 {dropped} 个还没发出的操作一并作废"));
        }
        // A warning, not an error: no command that is running now caused
        // it, and whoever drives the editor reads the command line's last
        // error as the outcome of the command they ran.
        self.command_line.push_warning(&text);
        self.collab.last_error = Some(text.clone());
        self.log_refusal(serde_json::json!({
            "op_id": entry.op_id,
            "code": code,
            "label": entry.label,
            "outcome": outcome,
            "steps_moved": moved.len(),
            "ops_dropped": dropped,
            "message": message,
        }));
        self.refresh_properties();
    }

    /// Keep `entry` in the refusal log, which holds the last eight.
    fn log_refusal(&mut self, entry: serde_json::Value) {
        self.collab.rejection_log.push(entry);
        if self.collab.rejection_log.len() > 8 {
            self.collab.rejection_log.remove(0);
        }
    }

    /// Move back the steps of tab `i` that `in_effect` names — off the
    /// drawing for steps that put something on it (`on`), on again for
    /// steps that took something off — from the top of the stack down to
    /// the deepest of them, if every one of them is still there. The steps
    /// moved, from the top.
    fn take_back_steps(&mut self, i: usize, on: bool, in_effect: &[u64]) -> Option<Vec<u64>> {
        use crate::app::tab_document::HistorySnapshot;
        let step_of = |snapshot: &HistorySnapshot| match snapshot {
            HistorySnapshot::Delta(delta) => delta.collab_step,
            _ => 0,
        };
        let history = &self.tabs[i].history;
        let stack = if on {
            &history.undo_stack
        } else {
            &history.redo_stack
        };
        let from_top: Vec<u64> = stack.iter().rev().map(step_of).collect();
        let depth = in_effect
            .iter()
            .map(|step| from_top.iter().position(|there| there == step))
            .collect::<Option<Vec<usize>>>()?
            .into_iter()
            .max()?;
        let steps = from_top[..=depth].to_vec();
        self.replay_history_quietly(i, on, steps.len());
        Some(steps)
    }

    /// Take back an op restored from the drawing's file that the room
    /// refused whole (`SPEC-collab-offline-sync.md` §5.4).
    ///
    /// Its undo steps died with the process that made them. What it kept
    /// is its inverse, written the way a peer's change is: item by item,
    /// each only where the drawing is still as the op left it — what
    /// somebody else did there since is the room's, and stays. A refusal
    /// that yields to the room (`OP_SUPERSEDED`, `OBJECT_DELETED`) takes
    /// nothing back: what the room has is in the drawing already.
    ///
    /// What waits behind it and builds on it (`collab::Refused::built_on`)
    /// goes with it, from the last back: this run's steps by the undo stack,
    /// with whatever this run did on top of them, as for any step of this
    /// run's; restored ones by their own inverses. The rest goes out as it
    /// would have. Each op that goes is said on the command line and
    /// logged; a restored one also goes on the unsynced list, there being
    /// no redo to bring it back. One that cannot be taken back marks the
    /// drawing `ROLLBACK_FAILED`.
    fn take_back_restored(&mut self, i: usize, entry: super::Outgoing, code: &str, message: &str) {
        use ocs_collab::protocol::codes;
        let built_on = super::Refused::built_on(&entry, &self.collab.outbox);
        // What became of each op that goes along: the outcome, whether what
        // a restored one did is out of the drawing, and the steps moved.
        let mut along: std::collections::HashMap<String, (String, Option<bool>, usize)> =
            std::collections::HashMap::new();
        // This run's steps were made last: they go first.
        let first_of_this_run = self
            .collab
            .outbox
            .iter()
            .position(|queued| !queued.restored && built_on.contains(&queued.op_id));
        if let Some(at) = first_of_this_run {
            let first = self.collab.outbox[at].clone();
            let after: std::collections::VecDeque<super::Outgoing> =
                self.collab.outbox.iter().skip(at + 1).cloned().collect();
            let account = super::Refused::account(&first, &after);
            if account.mixed {
                self.collab.mark_diverged(
                    ROLLBACK_FAILED,
                    format!(
                        "服务器拒绝了重开前排队的「{}」（{code}），而建在它上面的「{}」之后有一次撤销或重做\
                         把它和别的步骤一起动了，无法只撤回这一部分",
                        entry.label, first.label
                    ),
                );
            } else {
                let moved = if account.in_effect.is_empty() {
                    Some(Vec::new())
                } else {
                    self.take_back_steps(i, first.on, &account.in_effect)
                };
                let outcome = match &moved {
                    Some(steps) if steps.is_empty() => "本图里已经没有它，不需要再动".to_owned(),
                    Some(steps) => format!("已用撤销栈退回（共 {} 步，可以重做）", steps.len()),
                    None => {
                        self.collab.mark_diverged(
                            ROLLBACK_FAILED,
                            format!(
                                "服务器拒绝了重开前排队的「{}」（{code}），而建在它上面的「{}」已不在撤销栈里，\
                                 无法撤回",
                                entry.label, first.label
                            ),
                        );
                        "已不在撤销栈里，无法撤回，本图与房间已不一致".to_owned()
                    }
                };
                let steps_moved = moved.as_ref().map_or(0, Vec::len);
                for op_id in std::iter::once(&first.op_id).chain(&account.void) {
                    along.insert(op_id.clone(), (outcome.clone(), None, steps_moved));
                }
            }
        }
        // Then the restored ones built on it, from the last back.
        let restored: Vec<super::Outgoing> = self
            .collab
            .outbox
            .iter()
            .filter(|queued| queued.restored && built_on.contains(&queued.op_id))
            .cloned()
            .collect();
        for queued in restored.iter().rev() {
            let (outcome, rolled_back) = self.take_back_by_inverse(i, queued, false, code);
            along.insert(queued.op_id.clone(), (outcome, Some(rolled_back), 0));
        }
        let yields = code == codes::OP_SUPERSEDED || code == codes::OBJECT_DELETED;
        let (outcome, rolled_back) = self.take_back_by_inverse(i, &entry, yields, code);

        // Said and kept in the order they were queued, the refused op first.
        let went: Vec<super::Outgoing> = self
            .collab
            .outbox
            .iter()
            .filter(|queued| !queued.sent && along.contains_key(&queued.op_id))
            .cloned()
            .collect();
        self.collab
            .outbox
            .retain(|queued| queued.sent || !along.contains_key(&queued.op_id));
        let label = |op: &super::Outgoing| {
            if op.label.is_empty() {
                "这一步".to_owned()
            } else {
                op.label.clone()
            }
        };
        let mut text = format!(
            "协同：服务器没有接受重开前排队的「{}」（{code}：{message}）：{outcome}",
            label(&entry)
        );
        if !went.is_empty() {
            text.push_str(&format!(
                "；排在它后面、建在它上面的 {} 个操作一并作废",
                went.len()
            ));
        }
        // A warning, not an error, as for a step of this run's.
        self.command_line.push_warning(&text);
        self.collab.last_error = Some(text);
        self.log_refusal(serde_json::json!({
            "op_id": entry.op_id,
            "code": code,
            "label": entry.label,
            "outcome": outcome,
            "steps_moved": 0,
            "ops_dropped": went.len(),
            "message": message,
        }));
        self.keep_refused(i, &entry, code, message, rolled_back);
        let why = format!("建在被拒的「{}」（{code}）上，一并作废", label(&entry));
        for queued in &went {
            let Some((outcome, rolled_back, steps_moved)) = along.get(&queued.op_id).cloned()
            else {
                continue;
            };
            self.command_line
                .push_warning(&format!("协同：「{}」{why}：{outcome}", label(queued)));
            self.log_refusal(serde_json::json!({
                "op_id": queued.op_id,
                "code": code,
                "label": queued.label,
                "outcome": outcome,
                "steps_moved": steps_moved,
                "ops_dropped": 0,
                "message": why,
            }));
            if let Some(rolled_back) = rolled_back {
                self.keep_refused(i, queued, code, &why, rolled_back);
            }
        }
        self.refresh_properties();
    }

    /// Take restored op `entry` back out of tab `i` by its inverse (§5.4),
    /// unless the refusal `yields` to the room, whose state is in the
    /// drawing already. What became of it, and whether what it did is out
    /// of the drawing. One that cannot be taken back marks the drawing.
    fn take_back_by_inverse(
        &mut self,
        i: usize,
        entry: &super::Outgoing,
        yields: bool,
        code: &str,
    ) -> (String, bool) {
        if yields {
            return (
                "让位：房间里的样子已经在本图里，不需要再动".to_owned(),
                true,
            );
        }
        match self.write_inverse(i, entry) {
            Ok((_, 0)) => ("已按反向修改撤回".to_owned(), true),
            Ok((0, passed)) => (
                format!("{passed} 项已被别人改过，保留别人的，本图已是房间的样子"),
                true,
            ),
            Ok((_, passed)) => (
                format!("已按反向修改撤回，其中 {passed} 项已被别人改过，保留别人的"),
                true,
            ),
            Err(why) => {
                self.collab.mark_diverged(
                    ROLLBACK_FAILED,
                    format!(
                        "服务器拒绝了重开前排队的「{}」（{code}），{why}，无法在本图撤回",
                        entry.label
                    ),
                );
                (format!("{why}，无法撤回，本图与房间已不一致"), false)
            }
        }
    }

    /// Write onto tab `i`, the way a peer's change is written, the items of
    /// restored op `entry`'s inverse for which the drawing is still as the
    /// op left it: every change of the op about the same object holds
    /// (`broadcast::still_as_written`). How many were written and how many
    /// passed over, or why it cannot be done.
    fn write_inverse(
        &mut self,
        i: usize,
        entry: &super::Outgoing,
    ) -> Result<(usize, usize), String> {
        let Some(inverse) = &entry.inverse else {
            return Err("它存进图里时还没有算出反向修改".to_owned());
        };
        let read = broadcast::changes_from_payload(inverse);
        if !read.unread.is_empty() {
            return Err(format!(
                "它的反向修改里有本客户端不认识的变更（{}）",
                read.unread.join("、")
            ));
        }
        let said = broadcast::changes_from_payload(std::slice::from_ref(&entry.payload)).changes;
        let document = &self.tabs[i].scene.document;
        let (held, passed): (Vec<broadcast::Change>, Vec<broadcast::Change>) =
            read.changes.into_iter().partition(|undo| {
                let names = undo.names();
                said.iter()
                    .filter(|change| change.names().iter().any(|name| names.contains(name)))
                    .all(|change| broadcast::still_as_written(document, change))
            });
        let applied = self.write_as_received(i, &held);
        if !applied.unresolved.is_empty() {
            return Err(format!(
                "它的反向修改有 {} 项写不进本图（{}）",
                applied.unresolved.len(),
                applied.unresolved.join("、")
            ));
        }
        Ok((held.len(), passed.len()))
    }

    /// Put restored op `gone` on the unsynced list of tab `i`'s drawing as
    /// `refused`, with what the room said and whether what it did is out of
    /// the drawing: there is no redo to bring it back, and this is what is
    /// left of it (§5.4). A save writes it into the file.
    fn keep_refused(
        &mut self,
        i: usize,
        gone: &super::Outgoing,
        code: &str,
        message: &str,
        rolled_back: bool,
    ) {
        let entry = crate::pid::collab_state::Unsynced {
            state: crate::pid::collab_state::REFUSED.to_owned(),
            op_id: Some(gone.op_id.clone()),
            label: gone.label.clone(),
            payload: gone.payload.clone(),
            user_id: gone.user_id.clone(),
            at: ocs_collab::ids::now_unix(),
            code: Some(code.to_owned()),
            message: message.to_owned(),
            rolled_back: Some(rolled_back),
        };
        let Ok(entry) = serde_json::to_value(entry) else {
            return;
        };
        let id = self.tabs[i].id;
        self.collab.refused.entry(id).or_default().push(entry);
    }

    /// Take the top `steps` steps of tab `i` back (`undo`) or put them on
    /// again, without a word to the room and whatever the session may or
    /// may not write: the drawing is being put back to what the room has.
    /// The peers' changes made since those steps are written again
    /// afterwards, as after any undo (`replay_remote_after`).
    fn replay_history_quietly(&mut self, i: usize, undo: bool, steps: usize) {
        let in_front = self.active_tab;
        self.active_tab = i;
        self.collab.rolling_back = true;
        if undo {
            self.undo_steps_reported(steps, false);
        } else {
            self.redo_steps(steps);
        }
        self.collab.rolling_back = false;
        self.active_tab = in_front;
    }

    /// Read what takes the steps of each op still waiting to go out back out
    /// of the drawing (`Outgoing::inverse`; `SPEC-collab-offline-sync.md`
    /// §4.1), which a refusal heard after this process is gone needs (§5.4).
    /// It is what the room would be told on their being taken back — undone,
    /// for an op that put them on; put on again, for one that took them off
    /// — and is read by doing just that, quietly, and putting them back
    /// (`inverse_of`).
    ///
    /// Run after every message (`update`), and does something only while
    /// nothing else is on hand: no command under way and no step open in the
    /// session's drawing, no text being edited in place, no peer's change
    /// being written, no step being rolled back. What taking steps back and
    /// putting them on again moves besides the drawing is put back as it
    /// was: the selection, what is highlighted, the layout, the layer and
    /// what is isolated, whether the drawing is changed and its revision,
    /// and the view. The geometry epoch is not: what was built for the
    /// drawing meanwhile is built for it again.
    ///
    /// An op on the wire is left alone, its answer being on its way. One
    /// whose steps are not all where it left them, or that takes nothing
    /// back, has none; neither is looked at again.
    pub(in crate::app) fn read_queued_inverses(&mut self) {
        let unread = |entry: &super::Outgoing| {
            !entry.sent && !entry.restored && !entry.inverse_read && !entry.steps.is_empty()
        };
        if self.applying_remote
            || self.collab.rolling_back
            || self.graph_undo_open
            || !self.collab.outbox.iter().any(unread)
        {
            return;
        }
        let Some(i) = self.collab_tab() else {
            return;
        };
        if self.tabs[i].active_cmd.is_some()
            || self.tabs[i].history.pending.is_some()
            || self.mtext_editor.is_some()
            || self.table_cell_editor.is_some()
        {
            return;
        }
        let tab = &self.tabs[i];
        let selected = tab.scene.selected_handles_in_order();
        let constraint = tab.scene.selected_constraint;
        let hover = tab.scene.hover_highlight;
        let layout = tab.scene.current_layout.clone();
        let layer = tab.active_layer.clone();
        let isolated = tab.layiso_state.clone();
        let dirty = tab.dirty;
        let revision = tab.edit_revision;
        let camera = tab.scene.camera.borrow().clone();
        let camera_generation = tab.scene.camera_generation;
        for at in 0..self.collab.outbox.len() {
            if !unread(&self.collab.outbox[at]) {
                continue;
            }
            let entry = &self.collab.outbox[at];
            let (steps, on) = (entry.steps.clone(), entry.on);
            let inverse = self
                .inverse_of(i, &steps, on)
                .filter(|changes| !changes.is_empty())
                .map(|changes| changes.iter().map(broadcast::Change::to_json).collect());
            let entry = &mut self.collab.outbox[at];
            entry.inverse = inverse;
            entry.inverse_read = true;
        }
        let tab = &mut self.tabs[i];
        tab.scene.replace_selection_exact(&selected);
        tab.scene.selected_constraint = constraint;
        tab.scene.set_hover_highlight(hover);
        tab.scene.current_layout = layout;
        tab.active_layer = layer;
        tab.layiso_state = isolated;
        tab.dirty = dirty;
        tab.edit_revision = revision;
        *tab.scene.camera.borrow_mut() = camera;
        tab.scene.camera_generation = camera_generation;
        self.refresh_properties();
    }

    /// What taking `steps` of tab `i` back out of the drawing tells the
    /// room, in the order it tells it: undoing them, when they are on it
    /// (`on`), or putting them on again. The stack they are on is gone down
    /// one step at a time to the deepest of them and put back whole after;
    /// of what that says, only what their own steps say is kept. `None`
    /// when they are not all on that stack.
    fn inverse_of(&mut self, i: usize, steps: &[u64], on: bool) -> Option<Vec<broadcast::Change>> {
        use crate::app::tab_document::HistorySnapshot;
        let history = &self.tabs[i].history;
        let stack = if on {
            &history.undo_stack
        } else {
            &history.redo_stack
        };
        let from_top: Vec<bool> = stack
            .iter()
            .rev()
            .map(|snapshot| match snapshot {
                HistorySnapshot::Delta(delta) => steps.contains(&delta.collab_step),
                _ => false,
            })
            .collect();
        if from_top.iter().filter(|&&own| own).count() != steps.len() {
            return None;
        }
        let depth = from_top.iter().rposition(|&own| own)? + 1;
        let mut said = Vec::new();
        for &own in &from_top[..depth] {
            if own {
                self.collab.reading_inverse = Some(Vec::new());
            }
            self.replay_history_quietly(i, on, 1);
            said.extend(self.collab.reading_inverse.take().into_iter().flatten());
        }
        self.replay_history_quietly(i, !on, depth);
        Some(said)
    }

    /// Fold every remote `op_applied` the state is holding onto the open
    /// drawing.
    ///
    /// Runs right after `collab.apply` so the drawing catches up on the same
    /// frame the roster and baseline do (`VAL-CLI-009`).
    ///
    /// A peer's change is not a step on this person's undo stack: undo takes
    /// back what they did themselves, and never a colleague's edit (which it
    /// used to, the newest step being whatever arrived last — and it went
    /// out as this person's op). Nor does it clear their redo stack. What
    /// keeps a local undo from wiping a peer's change off an entity both
    /// touched is `replay_remote_after`.
    ///
    /// A local step still open when a change arrives is closed first, as
    /// the user's own: it is queued like any other step. Nothing of the
    /// queue goes out until the peers' changes are written, or one that
    /// could not be has marked the drawing diverged (`State::pump`).
    ///
    /// While the session's drawing is not among the tabs — not loaded yet, or
    /// closed — the ops stay in `pending_remote`, in order, and `on_tick`
    /// comes back here every frame until there is a drawing to put them on.
    ///
    /// An op this drawing cannot follow — a change shape it does not know, or
    /// an object it does not have — is reported (`last_apply_error`, and on
    /// the command line) rather than passed over: from there on the two
    /// drawings differ, and nobody should have to find that out later.
    pub(in crate::app) fn apply_remote_changes(&mut self) {
        // First what the server refused of this session's own: those steps
        // go back before anything a peer did is written on top of them.
        self.settle_rejected_ops();
        if self.collab.pending_remote.is_empty() {
            return;
        }
        // The session's drawing, wherever it is among the tabs — not the tab
        // in front, which may be another sheet carrying the same tags.
        let Some(i) = self.collab_tab() else {
            // Nowhere to put them yet. They stay queued: a session comes up
            // before the drawing it was started on has finished loading, and
            // what arrives in between must reach the drawing when it is there
            // (`on_tick` comes back here). Said once per change in the count,
            // not once per frame.
            let waiting = self.collab.pending_remote.len();
            let reason = if self.collab.document.is_some() {
                format!("协同的图纸已经关闭，{waiting} 个远端操作留在队列里，未应用")
            } else {
                format!("协同的图纸还没有打开，{waiting} 个远端操作在等它")
            };
            if self.collab.last_apply_error.as_deref() != Some(reason.as_str()) {
                self.report_apply_error(reason);
            }
            return;
        };
        // A drawing with a sync record that comes only now, after the
        // session came up without it, keeps what its record says: what the
        // room did meanwhile was not done to it (`seed_collab_from_file`).
        self.seed_collab_from_file(i);
        if self.collab.pending_remote.is_empty() {
            return;
        }
        // The user's own step, if one is open, ends here and is queued as
        // theirs — before the flag below is raised, which would keep it in.
        // Left open it would record the peer's writes as part of itself.
        // It ends while the peers' changes are still waiting to be written,
        // which keeps the queue from going out ahead of them.
        self.finish_pending_history(i);
        let pending = std::mem::take(&mut self.collab.pending_remote);
        let mut total = 0usize;
        let mut failure: Option<String> = None;
        // A question about the drawing's tags that was put off — the session
        // had no drawing when it was asked — is answered before the peers'
        // changes are written: what the room itself told is no news to it.
        // (One that was not put off is answered when they have been
        // written: the `hello` it is for names the revision they bring the
        // drawing to.)
        if self.collab.tags_wanted.is_some_and(|asked| asked.put_off) {
            self.answer_collab_tags();
        }
        // Raised while the peers' changes are written: nothing done here is
        // this client's to send.
        self.applying_remote = true;
        for remote in pending {
            let read = broadcast::changes_from_payload(&remote.changes);
            if !read.unread.is_empty() {
                // Said, whether or not the rest of the op can be applied:
                // the room has these changes and this drawing does not.
                failure.get_or_insert(format!(
                    "操作 {} 里有 {} 项变更本客户端不认识（{}），没有应用",
                    remote.op_id,
                    read.unread.len(),
                    read.unread.join("、")
                ));
            }
            let parsed = read.changes;
            if parsed.is_empty() {
                continue;
            }
            // Capture pre-apply values so we can emit clear toasts (#59, #60, #61).
            let pre_values: Vec<Option<String>> = parsed
                .iter()
                .map(|c| {
                    let doc = &self.tabs[i].scene.document;
                    if c.kind == broadcast::KIND_SET_PROPERTY {
                        doc.entities().find_map(|e| {
                            let a = crate::pid::persist::PidAuthor::load(doc, e.common().handle)?;
                            if a.tag != c.object_ref {
                                return None;
                            }
                            let (props, _) = crate::pid::propagate::split_props(&a.props);
                            Some(props.get(&c.field).cloned().unwrap_or_default())
                        })
                    } else if c.kind == broadcast::KIND_SET_TAG
                        || c.kind == broadcast::KIND_REASSIGN_TAG
                    {
                        Some(c.object_ref.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let applied = self.write_as_received(i, &parsed);
            if !applied.unresolved.is_empty() {
                failure.get_or_insert(format!(
                    "操作 {}：位号 {} 在本图里对不上一个对象（没有，或者有好几个），这部分变更未应用",
                    remote.op_id,
                    applied.unresolved.join("、")
                ));
            }
            self.emit_remote_change_toasts(
                &remote.op_id,
                remote.user_id.as_deref(),
                remote.display_name.as_deref(),
                &remote.changes,
                &parsed,
                &pre_values,
            );
            total += applied.handles.len() + applied.erased.len();
        }
        self.applying_remote = false;
        self.prune_remote_log(i);
        if total > 0 {
            self.refresh_properties();
            self.command_line
                .push_info(&format!("协同：已应用远端变更（{total} 个实体）"));
        }
        match failure {
            Some(reason) => {
                // The room has a change this drawing does not. Its baseline
                // moved all the same — that is the server's count — so the
                // session is marked, and writes nothing more on top of the
                // difference (`State::request_op`).
                let first = self.collab.diverged.is_none();
                self.collab.mark_diverged(REMOTE_UNAPPLIED, reason.clone());
                self.report_apply_error(reason);
                if first {
                    self.command_line.push_warning(
                        "协同：本图与房间已不一致，此后的本地修改不再提交；\
                         请取得最新图纸后重新打开",
                    );
                }
            }
            None if total > 0 => self.collab.last_apply_error = None,
            None => {}
        }
        // The queue waited for these to be written (`State::pump`): it
        // moves now, unless one could not be and the drawing is marked.
        self.collab.pump();
    }

    /// Emit user-facing toasts for applied remote changes:
    /// - `#59`: peer edited property or tag (multiple edits in one op merged)
    /// - `#60`: own newly placed symbol had its tag reassigned by the server
    /// - `#61`: upstream connector property arrived from another drawing
    /// - `#65`: clear unpaired connector entry when `pair_connector` succeeds
    fn emit_remote_change_toasts(
        &mut self,
        op_id: &str,
        author_user_id: Option<&str>,
        author_display_name: Option<&str>,
        raw_changes: &[serde_json::Value],
        parsed: &[broadcast::Change],
        pre_values: &[Option<String>],
    ) {
        use super::connectors::KIND_PAIR_CONNECTOR;
        use super::pipes::Pipe;
        use super::toasts::ToastKind;

        // Clear unpaired connector notifications as soon as pairing succeeds (#65).
        for change in parsed {
            if change.kind == KIND_PAIR_CONNECTOR {
                if let Some(Pipe::Connector(said)) = &change.pipe {
                    if said.peer.is_some() {
                        self.collab
                            .clear_unpaired_connector(&change.object_ref, &said.connector);
                    }
                }
            }
        }

        // Tag reassignments on our own op (#60).
        for change in parsed {
            if change.kind == broadcast::KIND_REASSIGN_TAG {
                self.collab.push_toast(
                    ToastKind::Warning,
                    format!(
                        "位号冲突，服务器已将 {} 改派为 {}",
                        change.object_ref, change.value
                    ),
                );
            }
        }

        // Upstream connector propagation (`link-<source_drawing>-...` or `source_drawing` field) (#61).
        let link_prefix = ocs_collab::link::LINK_SESSION_PREFIX;
        let source_drawing = raw_changes
            .iter()
            .find_map(|v| {
                v["source_drawing"]
                    .as_str()
                    .or_else(|| v["from_drawing"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| {
                op_id.strip_prefix("link-").and_then(|rest| {
                    // Server formats propagated op_ids as `link-<from>-r<rev>-<n>` or `link:<from>:...`
                    rest.split("-r")
                        .next()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
            })
            .or_else(|| {
                op_id
                    .starts_with(link_prefix)
                    .then(|| "上游图纸".to_owned())
            });

        // Resolve author label from typed OpApplied metadata (#59), falling
        // back to per-change JSON fields, then `"同事"`.
        let author_raw = {
            let uid = author_user_id.unwrap_or("").trim();
            let dn = author_display_name.unwrap_or("").trim();
            if !dn.is_empty() && !uid.is_empty() {
                Some(super::toasts::format_user_label(uid, dn))
            } else if !uid.is_empty() {
                Some(self.collab.format_user(uid))
            } else if !dn.is_empty() {
                Some(dn.to_owned())
            } else {
                raw_changes.iter().find_map(|v| {
                    let uid = v["user_id"].as_str().unwrap_or("");
                    let name = v["display_name"].as_str().unwrap_or("");
                    if !uid.is_empty() || !name.is_empty() {
                        Some(super::toasts::format_user_label(uid, name))
                    } else {
                        None
                    }
                })
            }
        };

        // Collect property & tag edits.
        let mut edits: Vec<(String, String, String, String)> = Vec::new();
        for (idx, change) in parsed.iter().enumerate() {
            if change.kind == broadcast::KIND_SET_PROPERTY {
                let old = pre_values
                    .get(idx)
                    .and_then(|o| o.clone())
                    .unwrap_or_default();
                edits.push((
                    change.object_ref.clone(),
                    change.field.clone(),
                    if old.is_empty() { "（空）".to_owned() } else { old },
                    if change.value.is_empty() {
                        "（空）".to_owned()
                    } else {
                        change.value.clone()
                    },
                ));
            } else if change.kind == broadcast::KIND_SET_TAG {
                edits.push((
                    change.object_ref.clone(),
                    "位号".to_owned(),
                    change.object_ref.clone(),
                    change.value.clone(),
                ));
            }
        }
        if edits.is_empty() {
            return;
        }
        // Ruling ops already emitted their own conflict-resolved toast (#62).
        if op_id.starts_with("resolve-") {
            return;
        }
        let (first_obj, first_field, first_old, first_new) = &edits[0];
        if let Some(src) = source_drawing {
            let msg = if edits.len() == 1 {
                format!("来自上游图纸 {src}：{first_obj} 的 {first_field}：{first_old} → {first_new}")
            } else {
                format!(
                    "来自上游图纸 {src}：已更新 {first_obj} 等 {} 项属性（{first_field}：{first_old} → {first_new}）",
                    edits.len()
                )
            };
            self.collab.push_toast(ToastKind::Info, msg);
            return;
        }
        let who = author_raw.unwrap_or_else(|| "同事".to_owned());
        let msg = if edits.len() == 1 {
            format!("{who} 修改了 {first_obj} 的 {first_field}：{first_old} → {first_new}")
        } else {
            format!(
                "{who} 修改了 {first_obj} 等 {} 处属性/位号（首项 {first_field}：{first_old} → {first_new}）",
                edits.len()
            )
        };
        self.collab.push_toast(ToastKind::Info, msg);
    }

    /// Write `changes` onto tab `i` the way a peer's change is written:
    /// nothing of it is this client's to send, and each change is kept, with
    /// the entities it was written to, for the undo and redo that come later
    /// (`replay_remote_after`).
    fn write_as_received(&mut self, i: usize, changes: &[broadcast::Change]) -> broadcast::Applied {
        let receiving = std::mem::replace(&mut self.applying_remote, true);
        let applied = self.write_remote_changes(i, changes);
        for (change, handles) in &applied.resolved {
            self.collab.remote_seq += 1;
            self.collab.remote_log.push(super::AppliedRemote {
                seq: self.collab.remote_seq,
                handles: handles.clone(),
                change: change.clone(),
            });
        }
        if !applied.is_empty() {
            self.settle_remote_writes(i, &applied.handles, &applied.retagged);
        }
        self.applying_remote = receiving;
        applied
    }

    /// Write one op's changes onto tab `i`, in the order it states them.
    ///
    /// A change to an object that is there is the document's business
    /// (`broadcast::apply_change`). A symbol placed or an object erased is
    /// the app's: placing takes the symbol library and the scene, the way a
    /// local `PIDINSERT` does.
    fn write_remote_changes(&mut self, i: usize, changes: &[broadcast::Change]) -> broadcast::Applied {
        let mut applied = broadcast::Applied::default();
        for change in changes {
            if change.is_structural() {
                self.write_remote_structure(i, change, &mut applied);
            } else {
                broadcast::apply_change(&mut self.tabs[i].scene.document, change, &mut applied);
            }
        }
        applied
    }

    /// Place the symbol a peer placed, or erase the object a peer erased.
    ///
    /// A peer's placement is always a new symbol here, whatever else carries
    /// its tag and whatever sits under the handle the sender names. Two
    /// people who each placed a symbol under one tag have two symbols, and
    /// it is the server that gives one of them another tag
    /// (`VAL-CROSS-010`). And two drawings opened from one file give their
    /// next entity the same handle, so the sender's handle carrying that
    /// tag here means this session placed one too — not that it is the
    /// same symbol. (It was read that way once, and the peer's symbol was
    /// then never placed.) This session's own placements do not come by
    /// this road at all: an op of its own is told by its id.
    ///
    /// What cannot be done — a symbol this program's library does not have,
    /// an erased object this drawing never had or has two of — goes into
    /// `applied.unresolved`, and from there to the person and the session's
    /// state, like any change the drawing could not follow.
    fn write_remote_structure(
        &mut self,
        i: usize,
        change: &broadcast::Change,
        applied: &mut broadcast::Applied,
    ) {
        use crate::pid::persist::PidAuthor;
        // What a pipe line or a run is made of: the drawing is brought to
        // it (`super::pipes`).
        if change.pipe.is_some() {
            self.write_remote_pipe(i, change, applied);
            return;
        }
        if change.kind == broadcast::KIND_DELETE_OBJECT {
            // A run goes with everything that is its: the segments and the
            // fittings have no name of their own to be erased by. (The
            // fittings that show the run's tag are why this is asked
            // before the tag is resolved like any other.)
            let targets = super::pipes::run_with_parts(
                &self.tabs[i].scene.document,
                &change.object_ref,
            )
            .unwrap_or_else(|| broadcast::resolve(&self.tabs[i].scene.document, change));
            if targets.is_empty() {
                applied.unresolved.push(if change.object_ref.is_empty() {
                    broadcast::UNTAGGED.to_owned()
                } else {
                    change.object_ref.clone()
                });
                return;
            }
            self.tabs[i].scene.erase_entities(&targets);
            self.tabs[i].dirty = true;
            self.tabs[i].edit_revision = self.tabs[i].edit_revision.wrapping_add(1);
            applied.erased.extend(targets.iter().copied());
            // Kept: a local undo may put the entity back from an older
            // image, and it is then erased again (`replay_remote_after`).
            applied.resolved.push((change.clone(), targets));
            return;
        }
        let Some(placed) = &change.placed else {
            applied.unresolved.push(change.object_ref.clone());
            return;
        };
        // The block comes into being with the first placement, as it does
        // for a local one.
        if !self.tabs[i].scene.document.block_records.contains(&placed.block) {
            self.pid_symbols.instantiate(&mut self.tabs[i].scene);
        }
        if !self.tabs[i].scene.document.block_records.contains(&placed.block) {
            applied.unresolved.push(format!(
                "{}（符号 {} 不在本机的符号库里）",
                change.object_ref, placed.block
            ));
            return;
        }
        let [x, y, z] = placed.position;
        let insert = document::entities::Insert::new(
            placed.block.clone(),
            document::types::Vector3::new(x, y, z),
        )
        .with_uniform_scale(placed.scale)
        .with_rotation(placed.rotation);
        let handle = self.tabs[i]
            .scene
            .add_entity(document::EntityType::Insert(insert));
        if handle.is_null() {
            applied.unresolved.push(change.object_ref.clone());
            return;
        }
        let document = &mut self.tabs[i].scene.document;
        PidAuthor {
            type_code: placed.type_code.clone(),
            tag: change.object_ref.clone(),
            tag_source: placed.tag_source.clone(),
            tag_status: "ok".to_owned(),
            ports: placed.ports.clone(),
            props: placed.props.clone(),
            ..PidAuthor::default()
        }
        .store(document, handle);
        crate::app::commands::pidpalette::sync_pid_tag_attribute(document, handle);
        applied.handles.push(handle);
        // Its label and its uniqueness status are worked out like a retag's.
        applied.retagged.push(handle);
    }

    /// What follows a peer's change having been written to `handles` on tab
    /// `i`, outside any undo step.
    ///
    /// What hangs off the changed objects is this drawing's own to work out,
    /// as after a local edit: a pipe line's field goes down to its runs and
    /// their segments (the sender states only the tagged objects it changed
    /// — a segment has no tag to name it by), and a tag that moved takes its
    /// label and its uniqueness status with it.
    fn settle_remote_writes(
        &mut self,
        i: usize,
        handles: &[document::Handle],
        retagged: &[document::Handle],
    ) {
        let carried_down = self.propagate_pid_chain(i, handles);
        let changes: Vec<(document::Handle, crate::scene::ChangeKind)> = handles
            .iter()
            .chain(&carried_down)
            .map(|handle| (*handle, crate::scene::ChangeKind::Modified))
            .collect();
        self.tabs[i].scene.bump_entities(&changes);
        self.tabs[i].dirty = true;
        if !retagged.is_empty() {
            self.refresh_pid_tags(i, retagged);
        }
        // The drawing changed without a step being pushed, which is what
        // normally moves this on.
        self.tabs[i].edit_revision = self.tabs[i].edit_revision.wrapping_add(1);
    }

    /// The `remote_mark` of a step being pushed on tab `i`: how many of the
    /// peers' changes the session's drawing has taken so far. A step on any
    /// other drawing has none to count.
    pub(in crate::app) fn collab_remote_mark(&self, i: usize) -> u64 {
        match self.collab.document {
            Some(id) if self.tabs.get(i).is_some_and(|tab| tab.id == id) => self.collab.remote_seq,
            _ => 0,
        }
    }

    /// Before the top `steps` steps of tab `i`'s undo stack (or redo stack)
    /// are applied: what [`Self::replay_remote_after`] will need afterwards.
    /// `None` when no peer's change is later than those steps — an undo then
    /// is what it always was.
    pub(in crate::app) fn history_guard(
        &self,
        i: usize,
        undo: bool,
        steps: usize,
    ) -> Option<HistoryGuard> {
        let tab = self.tabs.get(i)?;
        if self.collab.remote_log.is_empty() || self.collab.document != Some(tab.id) {
            return None;
        }
        let stack = if undo {
            &tab.history.undo_stack
        } else {
            &tab.history.redo_stack
        };
        let mut mark = None;
        let mut handles: Vec<document::Handle> = Vec::new();
        for snapshot in stack.iter().rev().take(steps) {
            if let crate::app::tab_document::HistorySnapshot::Delta(delta) = snapshot {
                mark = Some(mark.map_or(delta.remote_mark, |lowest: u64| lowest.min(delta.remote_mark)));
                handles.extend(delta.entities.iter().map(|(handle, _, _)| *handle));
            }
        }
        let mark = mark?;
        let later = self.collab.remote_log.iter().filter(|entry| entry.seq > mark);
        let before_later = handles.len();
        handles.extend(later.flat_map(|entry| entry.handles.iter().copied()));
        if handles.len() == before_later {
            return None;
        }
        handles.sort();
        handles.dedup();
        let document = &tab.scene.document;
        let before: Vec<_> = handles
            .into_iter()
            .map(|handle| (handle, document.get_entity_arc(handle)))
            .collect();
        let kept = before
            .iter()
            .filter_map(|(handle, image)| {
                let image = image.as_deref()?;
                let props = crate::pid::persist::PidAuthor::overflowed_props(document, image)?;
                Some((*handle, props))
            })
            .collect();
        Some(HistoryGuard {
            mark,
            before,
            kept,
            next_handle: document.next_handle(),
            blocks: document.block_records.clone(),
            app_ids: document.app_ids.clone(),
        })
    }

    /// After local steps were taken back or put on again on tab `i`: keep
    /// what the drawing's structure owes to the peers' later changes.
    ///
    /// A step that changed the structure — a table, a block, the topology
    /// record — holds an image of all of it and puts that back whole. The
    /// peers' entities made since are still in the drawing, so:
    ///
    /// * handles go on being given out from where they were: one given out
    ///   twice replaces the entity that had it;
    /// * a block the image does not know, and whose entities are in the
    ///   drawing, stays defined — a symbol whose block is gone draws
    ///   nothing, and defining the library again doubles it; a block the
    ///   step itself defined, whose entities went with the step, does not
    ///   come back;
    /// * every block keeps the entities it listed (model space lists the
    ///   peers' entities; a listed handle that is gone is tolerated, as it
    ///   is after any undo);
    /// * an application registered since stays registered: entities still
    ///   carry its data.
    ///
    /// Returns whether a block came back, for the scene to be told.
    fn keep_structure_ahead_of_peers(&mut self, i: usize, guard: &HistoryGuard) -> bool {
        use document::tables::TableEntry;
        let document = &mut self.tabs[i].scene.document;
        document.header.handle_seed = document.header.handle_seed.max(guard.next_handle);
        let mut restored = false;
        for record in guard.blocks.iter() {
            let alive = record
                .entity_handles
                .iter()
                .any(|handle| document.get_entity(*handle).is_some());
            match document.block_records.get_mut(record.name()) {
                Some(live) => {
                    let known: std::collections::HashSet<document::Handle> =
                        live.entity_handles.iter().copied().collect();
                    live.entity_handles.extend(
                        record
                            .entity_handles
                            .iter()
                            .copied()
                            .filter(|handle| !known.contains(handle)),
                    );
                }
                None if alive => {
                    document.block_records.add_or_replace(record.clone());
                    restored = true;
                }
                None => {}
            }
        }
        for app in guard.app_ids.iter() {
            if !document.app_ids.contains(app.name()) {
                document.app_ids.add_or_replace(app.clone());
            }
        }
        restored
    }

    /// Right after one of the steps a [`HistoryGuard`] was taken for has
    /// been applied on tab `i`: the entities that are not the step's keep
    /// the properties they had.
    ///
    /// Properties too long for an entity's record are kept in a record of
    /// the drawing's structure (`PidAuthor::store`), and a step that changed
    /// the structure puts all of that back as it was when the step was made
    /// — every entity's properties, not those of its own entities alone.
    /// What a peer's change has since written there, of an entity the step
    /// never touched, goes with the rest: a peer's change is no step, and in
    /// no step's image. The entity still points at its properties, and they
    /// are not there, or are an older set — and the peer's change written
    /// again over that (`replay_remote_after`) would leave the entity the
    /// one field, and tell the room that the others are gone.
    ///
    /// The step now holds the structure it replaced, so the properties are
    /// read there and written back. The step's own entities have what the
    /// step gives them.
    pub(in crate::app) fn keep_props_ahead_of_peers(
        &mut self,
        i: usize,
        guard: &HistoryGuard,
        step: &crate::app::tab_document::DeltaSnapshot,
    ) {
        use crate::app::tab_document::StructureSnapshot;
        use crate::pid::persist::PidAuthor;
        let Some(replaced) = step.structure.as_ref().and_then(StructureSnapshot::whole) else {
            return;
        };
        let own: std::collections::HashSet<document::Handle> =
            step.entities.iter().map(|(handle, _, _)| *handle).collect();
        let document = &self.tabs[i].scene.document;
        let taken: Vec<(document::Handle, String)> = document
            .entities()
            .filter_map(|entity| {
                let handle = entity.common().handle;
                if own.contains(&handle) {
                    return None;
                }
                let had = PidAuthor::overflowed_props(replaced, entity)?;
                let has = PidAuthor::overflowed_props(document, entity);
                (has.as_ref() != Some(&had)).then_some((handle, had))
            })
            .collect();
        if taken.is_empty() {
            return;
        }
        let document = &mut self.tabs[i].scene.document;
        // A record made anew takes a handle: not one the drawing has given
        // out since the structure now in place was its own
        // (`keep_structure_ahead_of_peers`).
        document.header.handle_seed = document.header.handle_seed.max(guard.next_handle);
        for (handle, props) in &taken {
            PidAuthor::write_overflowed_props(document, *handle, props);
        }
    }

    /// After local steps were taken back or put on again on tab `i`: write
    /// the peers' changes that came later than those steps once more, and
    /// say what the whole thing amounts to for the room.
    ///
    /// A step restores the entities it touched to images from when it was
    /// made. A peer's change made since to one of those entities is in
    /// neither image, so it is written again here — to the entities it was
    /// written to when it arrived, in the order the changes came. What is
    /// then sent to the room is this drawing before the undo against this
    /// drawing now: only what really changed. A field a peer has since set
    /// again stays as the peer set it, and nothing is said of it.
    pub(in crate::app) fn replay_remote_after(
        &mut self,
        i: usize,
        guard: HistoryGuard,
    ) -> Vec<broadcast::Change> {
        if self.keep_structure_ahead_of_peers(i, &guard) {
            // Symbols whose block is defined again are drawn again.
            let scene = &mut self.tabs[i].scene;
            scene.invalidate_dependency_index();
            scene.rebuild_derived_caches();
        }
        let later: Vec<super::AppliedRemote> = self
            .collab
            .remote_log
            .iter()
            .filter(|entry| entry.seq > guard.mark)
            .cloned()
            .collect();
        let mut applied = broadcast::Applied::default();
        let mut targets: Vec<document::Handle> = Vec::new();
        // An object a peer erased since: if the step put its entities back
        // from an older image, they go again.
        let mut erased_since: Vec<document::Handle> = Vec::new();
        {
            let document = &mut self.tabs[i].scene.document;
            for entry in &later {
                // What a line or a run is made of is put right further
                // down, from the drawing as a whole.
                if entry.change.pipe.is_some() {
                    continue;
                }
                let here = entry
                    .handles
                    .iter()
                    .copied()
                    .filter(|handle| document.get_entity(*handle).is_some());
                if entry.change.kind == broadcast::KIND_DELETE_OBJECT {
                    erased_since.extend(here);
                    continue;
                }
                broadcast::write_to(document, &entry.handles, &entry.change, &mut applied);
                targets.extend(
                    entry
                        .handles
                        .iter()
                        .copied()
                        .filter(|handle| document.get_entity(*handle).is_some()),
                );
            }
        }
        if !erased_since.is_empty() {
            erased_since.sort();
            erased_since.dedup();
            self.tabs[i].scene.erase_entities(&erased_since);
            targets.retain(|handle| !erased_since.contains(handle));
        }
        // What the peers have said since of pipe lines and runs. The step
        // put the topology table back as it was when the step was made —
        // without their runs' records — and may have put back parts their
        // accounts had done away with. Each line's stretches are made sure
        // of again and each run is aligned again to the last account given
        // of it; what a later change erased is not brought back.
        let erased_later = |tag: &str, seq: u64| {
            later.iter().any(|entry| {
                entry.seq > seq
                    && entry.change.kind == broadcast::KIND_DELETE_OBJECT
                    && entry.change.object_ref == tag
            })
        };
        let again: Vec<broadcast::Change> = later
            .iter()
            .enumerate()
            .filter(|(index, entry)| match &entry.change.pipe {
                Some(super::pipes::Pipe::Line(_)) => !erased_later(&entry.change.object_ref, entry.seq),
                Some(super::pipes::Pipe::Run(said)) => {
                    !erased_later(&entry.change.object_ref, entry.seq)
                        && !erased_later(&said.line_tag, entry.seq)
                        && !later[index + 1..].iter().any(|next| {
                            matches!(next.change.pipe, Some(super::pipes::Pipe::Run(_)))
                                && next.change.object_ref == entry.change.object_ref
                        })
                }
                // What was said of connectors is followed again in the
                // order it was said: each is a thing made sure of — it is
                // there, it is gone, it has this other end — and following
                // it twice changes nothing.
                Some(super::pipes::Pipe::Connector(_)) => true,
                None => false,
            })
            .map(|(_, entry)| entry.change.clone())
            .collect();
        if !again.is_empty() {
            let mut restated = broadcast::Applied::default();
            for change in &again {
                self.write_remote_pipe(i, change, &mut restated);
            }
            targets.extend(restated.handles);
            applied.retagged.extend(restated.retagged);
        }
        targets.sort();
        targets.dedup();
        // Carried down again whether or not anything had to be rewritten:
        // the step may have put back an entity that only *derives* from one
        // a peer changed — a segment under a line whose field moved — and
        // that is in no entry of the log.
        self.settle_remote_writes(i, &targets, &applied.retagged);
        self.refresh_properties();
        let document = &self.tabs[i].scene.document;
        let images: Vec<_> = guard
            .before
            .into_iter()
            .map(|(handle, before)| (handle, before, document.get_entity_arc(handle)))
            .collect();
        broadcast::collect_step_changes(document, &images, &broadcast::Left::Read(&guard.kept))
    }

    /// Drop the part of `remote_log` no step can ask for any more: changes
    /// no later than the oldest step still on tab `i`'s undo or redo stack.
    fn prune_remote_log(&mut self, i: usize) {
        const KEEP_WITHOUT_ASKING: usize = 1024;
        if self.collab.remote_log.len() <= KEEP_WITHOUT_ASKING {
            return;
        }
        let history = &self.tabs[i].history;
        let oldest = history
            .undo_stack
            .iter()
            .chain(&history.redo_stack)
            .filter_map(|snapshot| match snapshot {
                crate::app::tab_document::HistorySnapshot::Delta(delta) => Some(delta.remote_mark),
                _ => None,
            })
            .min()
            .unwrap_or(self.collab.remote_seq);
        self.collab.remote_log.retain(|entry| entry.seq > oldest);
    }

    /// Record why a remote change did not reach this drawing, where the status
    /// endpoint and the person at the command line both see it.
    fn report_apply_error(&mut self, reason: String) {
        self.command_line.push_warning(&format!("协同：{reason}"));
        self.collab.last_apply_error = Some(reason);
    }

    /// Whether a command may not run because the session is read-only.
    ///
    /// Navigation and inspection keep working — a read-only participant still
    /// has to look around the drawing — while anything that would change the
    /// document is refused (`VAL-CLI-011`). The allowlist is deliberately
    /// explicit: a new drawing command defaults to refused, which is the safe
    /// direction for a write gate.
    ///
    /// Five verbs read or write by what follows them, and are let through
    /// only in the forms that read: `PIDTAG` with one word jumps to a tag
    /// (two rename it -- unless all that follows the verb is the tag of an
    /// imported symbol on the sheet, which may hold spaces, and naming it is
    /// a jump as well), `PIDLEGEND` reports, lists and exports (`ON`
    /// draws the legend, `OFF` and `PURGE` erase it — and the bare verb
    /// defaults to `ON`), `PIDMASTERDATA` reports, bare or as `STATUS`,
    /// and exports (`INIT`, `IMPORT` and `EMBED` write master data),
    /// `PIDCHECK` checks, lists its rules and exports (`ENABLE` and
    /// `DISABLE` rewrite the project rule pack, `WAIVE` and `UNWAIVE` the
    /// drawing's waivers), and `PIDPUBLISH` checks, lists packages and
    /// verifies one (`CHECK`, `LIST`, `VERIFY`); any other form publishes
    /// (`SPEC-pid-publish.md` §5.2).
    ///
    /// This list is a courtesy, not the lock: a command that does write is
    /// better turned away before it prompts for anything than after. What
    /// keeps a read-only session from changing its drawing is
    /// [`Self::refuses_step`], at the one place every edit passes through;
    /// a verb wrongly let through here still changes nothing. Three things
    /// are written outside the drawing: the project file `PIDMASTERDATA`
    /// writes beside it, the project rule pack `PIDCHECK ENABLE` and
    /// `DISABLE` rewrite, and the package `PIDPUBLISH` publishes; no step
    /// carries any of them, so for them this gate is the only one.
    ///
    /// The gate is on the session's drawing. Another drawing open beside it
    /// is not in the session, and its commands are its own; while the session
    /// has no document yet, the drawing in front is the one it would take.
    pub(in crate::app) fn readonly_refuses_command(&self, cmd: &str) -> bool {
        if !self.collab.readonly {
            return false;
        }
        let in_front = self.tabs.get(self.active_tab).map(|tab| tab.id);
        if self.collab.document.is_some() && self.collab.document != in_front {
            return false;
        }
        let words: Vec<&str> = cmd.split_whitespace().collect();
        let verb = words.first().copied().unwrap_or("").to_ascii_uppercase();
        let option = words.get(1).copied().unwrap_or("").to_ascii_uppercase();
        match verb.as_str() {
            "PIDTAG" => {
                // All that follows the verb, as the command reads it.
                let named = cmd
                    .trim_start()
                    .split_once(char::is_whitespace)
                    .map_or("", |(_, rest)| rest);
                words.len() > 2 && !self.pidtag_names_an_imported_tag(self.active_tab, named)
            }
            "PIDLEGEND" => !matches!(option.as_str(), "REPORT" | "LIST" | "EXPORT"),
            "PIDMASTERDATA" => !matches!(option.as_str(), "" | "STATUS" | "EXPORT"),
            "PIDCHECK" => matches!(option.as_str(), "ENABLE" | "DISABLE" | "WAIVE" | "UNWAIVE"),
            "PIDPUBLISH" => !matches!(option.as_str(), "CHECK" | "LIST" | "VERIFY"),
            _ => !READONLY_ALLOWED.contains(&verb.as_str()),
        }
    }

    /// Turn `cmd` away if this session may not write and `cmd` would.
    ///
    /// Says so on the command line under the code the refusing tier calls
    /// for (`State::refusal`) — which is what a caller over REST is answered
    /// with — and tells the server, so the attempt is on record there with
    /// the object it was aimed at (`VAL-CROSS-004`, `-011`).
    pub(in crate::app) fn refuse_if_readonly(&mut self, cmd: &str) -> bool {
        if !self.readonly_refuses_command(cmd) {
            return false;
        }
        let (code, level) = self.collab.refusal();
        let level_zh = match level.as_str() {
            "role" => "角色权限",
            "pbs" => "PBS 节点 ACL",
            _ => "图纸状态",
        };
        let msg = format!("协同：当前会话只读（{level_zh}），拒绝执行 {cmd}");
        self.command_line.push_error_coded(&code, &msg);
        self.collab.push_toast(super::toasts::ToastKind::Error, msg);
        let object_ref = self.attempted_object(cmd);
        self.collab.report_attempt(&object_ref, cmd.trim());
        true
    }

    /// The tag of the object a refused command was aimed at, for the record:
    /// the tag `PIDTAG` names, else that of the first selected object that
    /// has one. Empty when the command names nothing and nothing is selected.
    fn attempted_object(&mut self, cmd: &str) -> String {
        let words: Vec<&str> = cmd.split_whitespace().collect();
        if words.first().is_some_and(|verb| verb.eq_ignore_ascii_case("PIDTAG")) {
            if let Some(tag) = words.get(1) {
                return self.canonical_tag(tag);
            }
        }
        let Some(i) = self.collab_tab() else {
            return String::new();
        };
        let tab = &self.tabs[i];
        tab.scene
            .selected_handles_in_order()
            .into_iter()
            .filter_map(|handle| crate::pid::persist::PidAuthor::load(&tab.scene.document, handle))
            .map(|author| author.tag)
            .find(|tag| !tag.is_empty())
            .unwrap_or_default()
    }

    /// Whether a step that has just been made on tab `i` is one this session
    /// may not make: it is read-only, the tab holds its drawing, and the step
    /// is not a peer's change being written.
    ///
    /// This is the lock. Every edit — a command, a row of the properties
    /// panel, a grip, a drop from the palette, a write over REST — ends as
    /// one step handed to `push_undo_entry`, and that is where this is
    /// asked. A step that may not be made is taken back there
    /// ([`Self::take_back_refused_step`]) instead of being kept.
    pub(in crate::app) fn refuses_step(&mut self, i: usize) -> bool {
        self.collab.readonly && !self.applying_remote && self.is_collab_tab(i)
    }

    /// The claim that keeps this session from changing what the images in
    /// `entities` show changed on tab `i`: the first tag among them that
    /// somebody else holds, and who.
    ///
    /// A claim is on a tag, and the tag stands for whatever object carries
    /// it — before the step or after it. So a retag is caught by the tag it
    /// takes from an object and by the tag it gives one, and a held symbol
    /// moved or erased is caught although neither says anything in P&ID
    /// terms (`VAL-CROSS-002`).
    ///
    /// What this client knows of the claims is what it has been told: the
    /// ones standing when it joined, and the holder of any it asked for and
    /// was refused (a grant is told to the session that asked, not to the
    /// room). One granted to somebody since is for the server to refuse.
    pub(in crate::app) fn claim_against(
        &mut self,
        i: usize,
        entities: &broadcast::EntityImages,
    ) -> Option<(String, String)> {
        if self.applying_remote || self.collab.claims.is_empty() || !self.is_collab_tab(i) {
            return None;
        }
        let own = self.collab.user_id.as_str();
        entities
            .iter()
            .flat_map(|(_, before, after)| [before.as_deref(), after.as_deref()])
            .flatten()
            .filter_map(crate::pid::persist::PidAuthor::from_entity)
            .filter(|author| !author.tag.is_empty())
            .find_map(|author| {
                let holder = self.collab.claims.get(&author.tag)?;
                (holder != own).then(|| (author.tag.clone(), holder.clone()))
            })
    }

    /// Whether an undo (`undo`) or a redo of `steps` steps on tab `i` would
    /// change an object somebody else holds — said and put on record when
    /// it would. Taking a step back is a write like the step was.
    pub(in crate::app) fn claim_refuses_replay(&mut self, i: usize, undo: bool, steps: usize) -> bool {
        use crate::app::tab_document::HistorySnapshot;
        // A step the server refused goes back whoever holds what: the
        // drawing is being put back to what the room has.
        if self.collab.rolling_back {
            return false;
        }
        let held = {
            let history = &self.tabs[i].history;
            let stack = if undo { &history.undo_stack } else { &history.redo_stack };
            let images: Vec<_> = stack
                .iter()
                .rev()
                .take(steps)
                .filter_map(|snapshot| match snapshot {
                    HistorySnapshot::Delta(delta) => Some(delta.entities.clone()),
                    _ => None,
                })
                .collect();
            images
                .iter()
                .find_map(|entities| self.claim_against(i, entities))
        };
        let Some((tag, holder)) = held else {
            return false;
        };
        let what = if undo { "UNDO" } else { "REDO" };
        self.command_line.push_error_coded(
            ocs_collab::protocol::codes::CLAIM_HELD,
            &format!("协同：位号 {tag} 已被 {holder} 占用，拒绝执行 {what}"),
        );
        self.collab.report_attempt(&tag, what);
        true
    }

    /// Say that a step was taken back because it touched `tag`, which
    /// `holder` holds, and put the attempt on record under that tag.
    pub(in crate::app) fn report_claimed_step(
        &mut self,
        changes: &[broadcast::Change],
        label: &str,
        tag: &str,
        holder: &str,
    ) {
        let holder_shown = self.collab.format_user(holder);
        let msg = format!("位号 {tag} 已被 {holder_shown} 占用，这一步修改（{label}）已撤回");
        self.command_line
            .push_error_coded(ocs_collab::protocol::codes::CLAIM_HELD, &format!("协同：{msg}"));
        self.collab
            .push_toast(super::toasts::ToastKind::Warning, msg);
        self.collab.report_attempt(tag, &tried_text(changes, label));
    }

    /// The tier (`role` / `pbs` / `state`) behind `code`, when `code` is what
    /// the read-only gate has just answered a write with. `None` for any
    /// other failure code.
    pub(in crate::app) fn collab_refusal_level(&self, code: &str) -> Option<String> {
        if !self.collab.readonly {
            return None;
        }
        let (refusal, level) = self.collab.refusal();
        (refusal == code).then_some(level)
    }

    /// Whether an undo or a redo on tab `i` is refused because the session
    /// may not write — said and put on record when it is.
    pub(in crate::app) fn refuses_history_replay(&mut self, i: usize, what: &str) -> bool {
        // Not a write of this session's: a step the server refused is
        // being taken back (`replay_history_quietly`).
        if self.collab.rolling_back || !self.refuses_step(i) {
            return false;
        }
        let (code, _level) = self.collab.refusal();
        self.command_line.push_error_coded(
            &code,
            &format!("协同：当前会话只读，拒绝执行 {what}"),
        );
        self.collab.report_attempt("", what);
        true
    }

    /// Say that a step was taken back, and put the attempt on record.
    /// `changes` is what the step would have told the room; `label` is the
    /// step's own name, for a step that says nothing in P&ID terms.
    pub(in crate::app) fn report_refused_step(
        &mut self,
        changes: &[broadcast::Change],
        label: &str,
    ) {
        let (code, level) = self.collab.refusal();
        let level_zh = match level.as_str() {
            "role" => "角色权限",
            "pbs" => "PBS 节点 ACL",
            _ => "图纸状态",
        };
        let msg = format!("当前会话只读（{level_zh}），这一步修改（{label}）已撤回");
        self.command_line
            .push_error_coded(&code, &format!("协同：{msg}"));
        self.collab
            .push_toast(super::toasts::ToastKind::Error, msg);
        let object_ref = changes
            .first()
            .map(|first| first.object_ref.clone())
            .unwrap_or_default();
        self.collab
            .report_attempt(&object_ref, &tried_text(changes, label));
    }
}

/// What a step that was taken back had tried, for the record: what it would
/// have told the room, or its own name when it says nothing in P&ID terms.
fn tried_text(changes: &[broadcast::Change], label: &str) -> String {
    if changes.is_empty() {
        return label.to_owned();
    }
    changes
        .iter()
        .map(|change| {
            format!(
                "{} {}.{} = {}",
                change.kind, change.object_ref, change.field, change.value
            )
        })
        .collect::<Vec<_>>()
        .join("；")
}

/// Commands a read-only session may still run: looking around, inspecting, and
/// the collaboration panel itself. Everything else is a write.
///
/// `PIDLEGEND`, `PIDMASTERDATA`, `PIDCHECK` and `PIDPUBLISH` are here for
/// their reading forms only (see `readonly_refuses_command`). Not here,
/// though they once were: `MEASURE` (places points along an object), `FIND`
/// (opens find-and-replace), `PIDGROUP` (every one of its forms regroups or
/// retags) and `AUDIT` (applies its fixes when answered yes). `PIDSNAPSHOT` and
/// `PIDDIFF` write files on this machine, never the drawing, so every one of
/// their forms is here. So are `PIDVERIFY`'s, which only show, hide or turn
/// the verify-and-deliver panel.
pub(super) const READONLY_ALLOWED: &[&str] = &[
    "ZOOM",
    "ZO",
    "PAN",
    "P",
    "REGEN",
    "RE",
    "REGENALL",
    "UCS",
    "UCSICON",
    "ORTHO",
    "OSNAP",
    "GRID",
    "SNAP",
    "LIST",
    "LI",
    "DIST",
    "DI",
    "AREA",
    "AA",
    "ID",
    "MEASUREGEOM",
    "PROPERTIES",
    "PROPS",
    "PR",
    "QSELECT",
    "QS",
    "SELECTALL",
    "HELP",
    "PIDLEGEND",
    "PIDMASTERDATA",
    "PIDINDEX",
    "PIDLOCATE",
    "PIDCHECK",
    "PIDBOM",
    "PIDSNAPSHOT",
    "PIDDIFF",
    "PIDPUBLISH",
    "PIDVERIFY",
    "PIDLINE",
    "PIDTAG",
    "PIDTOPOLOGY",
    "PIDPALETTE",
    "PIDCOLLAB",
    "PIDCLAIM",
    "PIDRELEASE",
    "PIDSCREEN",
    "PIDPLOT",
    "PLOT",
    "PRINT",
    "PREVIEW",
    "PERF",
    "LAYERS",
    "LA",
    "LAYER",
    "LINETYPE",
    "LT",
    "SAVEAS",
    "QUIT",
    "EXIT",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::collab::CollabEvent;
    use crate::pid::persist::PidAuthor;
    use document::entities::Insert;
    use document::types::Vector3;

    fn app_with_vessel(tag: &str) -> (OpenCADStudio, document::Handle) {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let handle = {
            let document = &mut app.tabs[i].scene.document;
            let handle = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::ZERO,
                )))
                .expect("entity");
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: tag.to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                props: r#"{"设计压力":"1.6MPa"}"#.to_owned(),
                ..PidAuthor::default()
            }
            .store(document, handle);
            handle
        };
        (app, handle)
    }

    /// VAL-CLI-009: a remote `op_applied` naming a tag rewrites the local
    /// document, and the label follows.
    #[test]
    fn a_remote_tag_change_reaches_the_local_document() {
        let (mut app, handle) = app_with_vessel("P-1001");
        app.collab.user_id = "u2".to_owned();
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-1".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_tag",
                "object_ref": "P-1001",
                "handle": format!("{:X}", handle.value()),
                "field": "tag",
                "value": "P-1009",
                "old_value": "P-1001",
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();

        let author =
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author");
        assert_eq!(author.tag, "P-1009");
        assert_eq!(app.collab.base_revision, 1);
        assert_eq!(app.collab.applied_ops, 1);
        assert!(app.collab.pending_remote.is_empty());
    }

    /// VAL-CLI-010: the baseline moves with the broadcast even for a remote op.
    #[test]
    fn a_remote_op_moves_the_baseline_to_the_server_revision() {
        let (mut app, handle) = app_with_vessel("P-1001");
        // The session is at revision 1; the room's next op is 2.
        app.collab.base_revision = 1;
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-2".to_owned(),
            revision: 2,
            changes: vec![serde_json::json!({
                "kind": "set_property",
                "object_ref": "P-1001",
                "handle": format!("{:X}", handle.value()),
                "field": "设计压力",
                "value": "2.5MPa",
            })],
            user_id: None,
            display_name: None,
        });
        assert_eq!(app.collab.base_revision, 2);
        app.apply_remote_changes();
        let author =
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author");
        assert!(author.props.contains("2.5MPa"), "{}", author.props);
        assert_eq!(app.collab.diverged, None, "the next revision is no gap");
    }

    /// Our own op comes back from the room broadcast; applying it again would
    /// double-count and, worse, re-open an undo step for a change already in
    /// the document.
    #[test]
    fn our_own_op_is_not_applied_twice() {
        let (mut app, handle) = app_with_vessel("P-1001");
        app.collab.user_id = "u1".to_owned();
        app.collab.own_ops.push_back("op-mine".to_owned());
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-mine".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_tag",
                "object_ref": "P-1001",
                "handle": format!("{:X}", handle.value()),
                "field": "tag",
                "value": "P-1009",
            })],
            user_id: None,
            display_name: None,
        });
        assert!(app.collab.pending_remote.is_empty(), "自己的 op 不该重放");
        assert_eq!(app.collab.base_revision, 1, "基线仍要跟着走");
        assert!(app.collab.own_ops.is_empty(), "已回来的 op id 要出队");
    }

    /// Applying a remote change must not echo back to the room.
    ///
    /// The assertion is on `own_ops` rather than the outbound queue: the queue
    /// is process-global and other tests in this binary write to it, while
    /// `own_ops` only grows when *this* app queues an op.
    #[test]
    fn folding_a_remote_change_does_not_broadcast_it_back() {
        let (mut app, handle) = app_with_vessel("P-1001");
        app.collab.connected = true;
        app.collab.session_id = Some("s-2".to_owned());
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-3".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_tag",
                "object_ref": "P-1001",
                "handle": format!("{:X}", handle.value()),
                "field": "tag",
                "value": "P-1009",
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert!(
            app.collab.own_ops.is_empty(),
            "远端变更不应回灌成自己的 op：{:?}",
            app.collab.own_ops
        );
        // And the fold really did land, so the assertion above is not vacuous.
        assert_eq!(
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle)
                .expect("author")
                .tag,
            "P-1009"
        );
    }

    /// VAL-CLI-011: a downgraded session refuses a write command and leaves
    /// the record alone.
    #[test]
    fn a_readonly_session_refuses_writes_and_keeps_inspection() {
        let (mut app, _) = app_with_vessel("P-1001");
        // A two-word PIDTAG is a retag, which is a write.
        assert!(!app.readonly_refuses_command("PIDTAG P-1001 P-1009"));

        app.collab.readonly = true;
        assert!(app.readonly_refuses_command("PIDTAG P-1001 P-1009"));
        assert!(app.readonly_refuses_command("LINE 0,0 10,0"));
        assert!(app.readonly_refuses_command("ERASE"));
        // Looking around and reading stay available.
        assert!(!app.readonly_refuses_command("ZOOM EXTENTS"));
        assert!(!app.readonly_refuses_command("PIDCOLLAB ON"));
        assert!(!app.readonly_refuses_command("LIST"));
        // Jumping to a tag by name is navigation, not a write.
        assert!(!app.readonly_refuses_command("PIDTAG P-1001"));
    }

    /// What the last undo step of the active drawing would put on the wire.
    fn last_step_payload(app: &OpenCADStudio) -> serde_json::Value {
        let i = app.active_tab;
        let Some(crate::app::tab_document::HistorySnapshot::Delta(delta)) =
            app.tabs[i].history.undo_stack.last()
        else {
            panic!("the last undo step is not a delta");
        };
        broadcast::payload_for(&broadcast::collect_changes(&delta.entities))
            .expect("the step carries a semantic change")
    }

    /// The server and every other client know an object by its tag
    /// (`architecture.md` §3.2), so that is what an op names it by; the entity
    /// handle rides along only as a hint for peers holding the same file.
    #[test]
    fn a_local_retag_names_its_object_by_the_tag_it_had() {
        let (mut app, handle) = app_with_vessel("P-1001");
        let _ = app.run_command_line("PIDTAG P-1001 P-1009");
        app.finish_all_pending_history();

        let payload = last_step_payload(&app);
        assert_eq!(payload["kind"], "set_tag");
        assert_eq!(payload["object_ref"], "P-1001", "{payload}");
        assert_eq!(payload["field"], "tag");
        assert_eq!(payload["value"], "P-1009");
        assert_eq!(payload["old_value"], "P-1001", "{payload}");
        assert_eq!(
            payload["handle"],
            format!("{:X}", handle.value()),
            "{payload}"
        );
    }

    /// A step that changes several properties of one symbol broadcasts all of
    /// them, or the peers keep whatever the others were.
    #[test]
    fn every_property_a_step_changes_is_broadcast() {
        let (mut app, handle) = app_with_vessel("P-1001");
        // The contract's own write channel for properties:
        // `PUT /api/v1/entities/{h}/xdata/PID_AUTHOR` carrying `props`.
        let state = app.control_state();
        let (response, task) = app.control_request(serde_json::json!({
            "protocol": 1,
            "op": "xdata_set",
            "request_id": "collab-props-1",
            "client_id": "test",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "handles": [format!("{:X}", handle.value())],
            "app": "PID_AUTHOR",
            "data": [{
                "code": 1000,
                "value": r#"props={"设计压力":"2.5MPa","设计温度":"120℃"}"#,
            }],
        }));
        app.drive_headless_task(task).unwrap();
        assert_eq!(response["ok"], true, "{response}");
        app.finish_all_pending_history();

        let payload = last_step_payload(&app);
        let mut fields = vec![payload["field"].as_str().unwrap_or("").to_owned()];
        fields.extend(
            payload["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|change| change["field"].as_str().unwrap_or("").to_owned()),
        );
        fields.sort();
        assert_eq!(fields, ["设计压力", "设计温度"], "{payload}");
        assert_eq!(payload["object_ref"], "P-1001", "{payload}");
    }

    /// A change that names its object by tag alone — what a client with no
    /// drawing of its own sends — is applied to the symbol carrying that tag.
    #[test]
    fn a_remote_change_naming_only_a_tag_reaches_the_symbol_carrying_it() {
        let (mut app, handle) = app_with_vessel("P-1001");
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-9".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_property",
                "object_ref": "P-1001",
                "field": "设计压力",
                "value": "2.5MPa",
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();

        let author =
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author");
        assert!(author.props.contains("2.5MPa"), "{}", author.props);
        assert_eq!(app.collab.last_apply_error, None);
    }

    /// Undoing a change puts the old value back in this drawing, so the room
    /// has to hear about it as well; redoing it likewise.
    #[test]
    fn undo_and_redo_are_broadcast_like_the_edit_they_replay() {
        let (mut app, _) = app_with_vessel("P-1001");
        app.collab.connected = true;
        app.collab.session_id = Some("s-1".to_owned());

        let _ = app.run_command_line("PIDTAG P-1001 P-1009");
        app.finish_all_pending_history();
        assert_eq!(app.collab.own_ops.len(), 1, "the edit itself");

        let _ = app.automation_op(r#"{"op":"undo"}"#);
        assert_eq!(app.collab.own_ops.len(), 2, "the undo");

        let _ = app.automation_op(r#"{"op":"redo"}"#);
        assert_eq!(app.collab.own_ops.len(), 3, "the redo");
    }

    /// The gate runs before the command body, so a refused write leaves the
    /// stored tag untouched.
    #[test]
    fn a_refused_write_leaves_the_tag_unchanged() {
        let (mut app, handle) = app_with_vessel("P-1001");
        app.collab.readonly = true;
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG P-1001 P-1009"}"#);
        assert_eq!(refused["ok"], false, "{refused}");
        let author =
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author");
        assert_eq!(author.tag, "P-1001", "只读拒绝后位号不得改写");
    }

    /// Open another drawing in a tab of its own, put a vessel carrying `tag`
    /// on it and bring it to the front. Returns the tab and the vessel.
    fn another_drawing_in_front(app: &mut OpenCADStudio, tag: &str) -> (usize, document::Handle) {
        let id = app.push_test_document();
        let tab = app
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .expect("the new tab");
        let document = &mut app.tabs[tab].scene.document;
        let handle = document
            .add_entity(document::EntityType::Insert(Insert::new(
                "PID_SYM_equipment_vessel",
                Vector3::ZERO,
            )))
            .expect("entity");
        PidAuthor {
            type_code: "EQUIPMENT".to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            ..PidAuthor::default()
        }
        .store(document, handle);
        app.active_tab = tab;
        (tab, handle)
    }

    fn remote_retag(app: &mut OpenCADStudio, op_id: &str, revision: i64, old: &str, new: &str) {
        app.collab.apply(CollabEvent::OpApplied {
            op_id: op_id.to_owned(),
            revision,
            changes: vec![serde_json::json!({
                "kind": "set_tag",
                "object_ref": old,
                "field": "tag",
                "value": new,
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
    }

    /// The session speaks for one drawing. A peer's change belongs on the tab
    /// holding that drawing, not on whichever tab is in front when it
    /// arrives — which may well carry the same tag: it can be a copy of the
    /// same sheet.
    #[test]
    fn a_remote_change_lands_on_the_sessions_drawing_not_on_the_tab_in_front() {
        let (mut app, vessel) = app_with_vessel("P-1001");
        let session_tab = app.active_tab;
        // The first change arrives with the session's drawing in front.
        remote_retag(&mut app, "op-peer-a", 1, "P-1001", "P-1005");
        assert_eq!(
            PidAuthor::load(&app.tabs[session_tab].scene.document, vessel)
                .expect("author")
                .tag,
            "P-1005"
        );

        // Then the person opens a copy of the sheet and works in that.
        let (copy_tab, copy_vessel) = another_drawing_in_front(&mut app, "P-1005");
        remote_retag(&mut app, "op-peer-b", 2, "P-1005", "P-1009");

        assert_eq!(
            PidAuthor::load(&app.tabs[copy_tab].scene.document, copy_vessel)
                .expect("author")
                .tag,
            "P-1005",
            "the tab in front is not the session's drawing and must be left alone"
        );
        assert_eq!(
            PidAuthor::load(&app.tabs[session_tab].scene.document, vessel)
                .expect("author")
                .tag,
            "P-1009",
            "the session's drawing takes the change while it is in the background"
        );
        assert_eq!(app.collab.last_apply_error, None);
    }

    /// And the other way: an edit made on another drawing is that drawing's
    /// business. Broadcast, it would tell the room the session's drawing
    /// changed.
    #[test]
    fn an_edit_on_another_drawing_is_not_broadcast_as_the_sessions() {
        let (mut app, _) = app_with_vessel("P-1001");
        app.collab.connected = true;
        app.collab.session_id = Some("s-1".to_owned());
        let _ = app.run_command_line("PIDTAG P-1001 P-1005");
        app.finish_all_pending_history();
        assert_eq!(app.collab.own_ops.len(), 1, "the session's drawing");

        let (copy_tab, copy_vessel) = another_drawing_in_front(&mut app, "P-2001");
        let _ = app.run_command_line("PIDTAG P-2001 P-2009");
        app.finish_all_pending_history();
        assert_eq!(
            PidAuthor::load(&app.tabs[copy_tab].scene.document, copy_vessel)
                .expect("author")
                .tag,
            "P-2009",
            "the edit itself goes through"
        );
        assert_eq!(
            app.collab.own_ops.len(),
            1,
            "an edit to another drawing went out as this session's: {:?}",
            app.collab.own_ops
        );

        // Taking that edit back and putting it on again is the other
        // drawing's business too.
        let _ = app.automation_op(r#"{"op":"undo"}"#);
        assert_eq!(
            PidAuthor::load(&app.tabs[copy_tab].scene.document, copy_vessel)
                .expect("author")
                .tag,
            "P-2001",
            "the undo itself goes through"
        );
        assert_eq!(app.collab.own_ops.len(), 1, "its undo went out");
        let _ = app.automation_op(r#"{"op":"redo"}"#);
        assert_eq!(
            PidAuthor::load(&app.tabs[copy_tab].scene.document, copy_vessel)
                .expect("author")
                .tag,
            "P-2009"
        );
        assert_eq!(app.collab.own_ops.len(), 1, "its redo went out");
    }

    /// The session comes up before the drawing it was started on has finished
    /// loading. What a peer does in between waits for the drawing; it is not
    /// thrown away, and the next frame after the drawing is there applies it.
    #[test]
    fn a_change_that_arrives_before_the_drawing_is_open_waits_for_it() {
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start, "only the start page so far");
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-early".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_tag", "object_ref": "P-1001", "field": "tag", "value": "P-1009",
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert_eq!(app.collab.pending_remote.len(), 1, "kept, not dropped");
        assert_eq!(app.collab.document, None, "the start page is not a drawing");
        let said = app.collab.last_apply_error.clone().expect("it says it is waiting");
        // Asked again on the next frame, it has nothing new to say.
        app.apply_remote_changes();
        assert_eq!(app.collab.last_apply_error.as_deref(), Some(said.as_str()));
        assert_eq!(app.collab.pending_remote.len(), 1);

        // The drawing opens.
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let tab = app.active_tab;
        let vessel = {
            let document = &mut app.tabs[tab].scene.document;
            let handle = document
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::ZERO,
                )))
                .expect("entity");
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1001".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, handle);
            handle
        };
        let _ = app.update(crate::app::Message::Tick(iced::time::Instant::now()));

        assert_eq!(
            PidAuthor::load(&app.tabs[tab].scene.document, vessel)
                .expect("author")
                .tag,
            "P-1009",
            "the frame after the drawing opened applies what was waiting"
        );
        assert!(app.collab.pending_remote.is_empty());
        assert_eq!(app.collab.last_apply_error, None);
        assert_eq!(app.collab.document, Some(app.tabs[tab].id));
    }

    // ── Undo takes back this person's steps, not a peer's ────────────────

    /// A second vessel on the drawing `app_with_vessel` made.
    fn add_vessel(app: &mut OpenCADStudio, tag: &str, props: &str) -> document::Handle {
        let i = app.active_tab;
        let document = &mut app.tabs[i].scene.document;
        let handle = document
            .add_entity(document::EntityType::Insert(Insert::new(
                "PID_SYM_equipment_vessel",
                Vector3::new(40.0, 0.0, 0.0),
            )))
            .expect("entity");
        PidAuthor {
            type_code: "EQUIPMENT".to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            props: props.to_owned(),
            ..PidAuthor::default()
        }
        .store(document, handle);
        handle
    }

    fn connected(app: &mut OpenCADStudio) {
        app.collab.connected = true;
        app.collab.session_id = Some("s-1".to_owned());
    }

    /// A local property edit, through the contract's own write channel
    /// (`xdata_set` carrying the whole `props`).
    fn put_props(app: &mut OpenCADStudio, handle: document::Handle, props: &str) {
        let state = app.control_state();
        let (response, task) = app.control_request(serde_json::json!({
            "protocol": 1,
            "op": "xdata_set",
            "request_id": format!("collab-props-{}", app.collab.sent.len()),
            "client_id": "test",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "handles": [format!("{:X}", handle.value())],
            "app": "PID_AUTHOR",
            "data": [{"code": 1000, "value": format!("props={props}")}],
        }));
        app.drive_headless_task(task).unwrap();
        assert_eq!(response["ok"], true, "{response}");
        app.finish_all_pending_history();
    }

    fn remote(app: &mut OpenCADStudio, revision: i64, changes: Vec<serde_json::Value>) {
        app.collab.apply(CollabEvent::OpApplied {
            op_id: format!("op-peer-{revision}"),
            revision,
            changes,
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
    }

    fn author(app: &OpenCADStudio, handle: document::Handle) -> PidAuthor {
        PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle).expect("author")
    }

    fn prop(app: &OpenCADStudio, handle: document::Handle, key: &str) -> String {
        crate::pid::propagate::split_props(&author(app, handle).props)
            .0
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// Every change the last op this session queued states, flattened.
    fn last_sent(app: &OpenCADStudio) -> Vec<broadcast::Change> {
        let payload = app.collab.sent.last().expect("an op was queued").clone();
        broadcast::changes_from_payload(&[payload]).changes
    }

    fn undo(app: &mut OpenCADStudio) {
        let _ = app.automation_op(r#"{"op":"undo"}"#);
    }

    fn redo(app: &mut OpenCADStudio) {
        let _ = app.automation_op(r#"{"op":"redo"}"#);
    }

    /// A peer's change is not a step on this person's undo stack. Undo takes
    /// back what they did themselves — it used to take back whatever arrived
    /// last, a colleague's edit, and send that out as this person's op.
    #[test]
    fn undo_takes_back_this_persons_step_and_leaves_a_peers_change_alone() {
        let (mut app, pump) = app_with_vessel("P-1001");
        let vessel = add_vessel(&mut app, "V-1001", "{}");
        connected(&mut app);
        let _ = app.run_command_line("PIDTAG P-1001 P-1005");
        app.finish_all_pending_history();
        assert_eq!(app.collab.sent.len(), 1);
        let steps = app.tabs[app.active_tab].history.undo_stack.len();

        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "V-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        );
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa");
        assert_eq!(
            app.tabs[app.active_tab].history.undo_stack.len(),
            steps,
            "a peer's change must not become a step of this person's"
        );

        undo(&mut app);
        assert_eq!(author(&app, pump).tag, "P-1001", "this person's retag is taken back");
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa", "the peer's change stays");
        assert_eq!(app.collab.sent.len(), 2, "the undo goes out");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(
            (said[0].kind.as_str(), said[0].object_ref.as_str(), said[0].value.as_str()),
            ("set_tag", "P-1005", "P-1001")
        );

        // Nothing of this person's is left to take back, and the peer's
        // change is not there to be taken.
        undo(&mut app);
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa");
        assert_eq!(app.collab.sent.len(), 2);
    }

    /// One object, two people, two fields. Taking back one's own field puts
    /// back the entity as it was before one's step — an image the peer's
    /// field is not in — so the peer's change is written again afterwards,
    /// and only one's own field is said to have moved. Redo likewise.
    #[test]
    fn undo_puts_back_ones_own_field_and_keeps_the_field_a_peer_changed() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa","设计温度":"80℃"}"#);
        connected(&mut app);
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa","设计温度":"80℃"}"#);
        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计温度", "value": "120℃",
            })],
        );

        undo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.0MPa", "one's own field goes back");
        assert_eq!(prop(&app, pump, "设计温度"), "120℃", "the peer's field stays");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(
            (said[0].field.as_str(), said[0].value.as_str()),
            ("设计压力", "1.0MPa")
        );

        redo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa");
        assert_eq!(prop(&app, pump, "设计温度"), "120℃", "and stays through the redo");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(
            (said[0].field.as_str(), said[0].value.as_str()),
            ("设计压力", "1.6MPa")
        );
    }

    /// The same field. The peer wrote it after this person did, so the
    /// peer's value is what the room has; taking one's own write back
    /// changes nothing here and says nothing there. Said, it would put the
    /// old value over the peer's for everyone.
    #[test]
    fn undoing_a_value_a_peer_has_since_overwritten_changes_nothing_and_says_nothing() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa"}"#);
        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        );
        let sent = app.collab.sent.len();

        undo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "2.5MPa");
        assert_eq!(app.collab.sent.len(), sent, "{:?}", app.collab.sent.last());
    }

    /// A peer's change used to clear this person's redo stack, being pushed
    /// as a step of theirs. It is not a step, and the redo is still there.
    #[test]
    fn a_peers_change_leaves_this_persons_redo_in_place() {
        let (mut app, pump) = app_with_vessel("P-1001");
        let vessel = add_vessel(&mut app, "V-1001", "{}");
        connected(&mut app);
        let _ = app.run_command_line("PIDTAG P-1001 P-1005");
        app.finish_all_pending_history();
        undo(&mut app);
        assert_eq!(author(&app, pump).tag, "P-1001");

        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "V-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        );
        assert_eq!(
            app.tabs[app.active_tab].history.redo_stack.len(),
            1,
            "the redo was thrown away"
        );
        redo(&mut app);
        assert_eq!(author(&app, pump).tag, "P-1005");
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa");
    }

    /// A peer renamed the object after this person edited it. The undo finds
    /// it all the same — by the entity, not the name it had — keeps the new
    /// name, and tells the room about the object under the name it has now.
    #[test]
    fn an_undo_after_a_peers_retag_names_the_object_by_the_tag_it_has_now() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa"}"#);
        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_tag", "object_ref": "P-1001", "field": "tag", "value": "P-1009",
            })],
        );

        undo(&mut app);
        assert_eq!(author(&app, pump).tag, "P-1009", "the peer's name for it stays");
        assert_eq!(prop(&app, pump, "设计压力"), "1.0MPa");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(
            (said[0].object_ref.as_str(), said[0].field.as_str(), said[0].value.as_str()),
            ("P-1009", "设计压力", "1.0MPa")
        );
    }

    /// A step of this person's still open when a peer's change arrives ends
    /// there and goes out as theirs. It used to be closed inside the fold,
    /// where nothing is sent: the edit stayed in this drawing only.
    #[test]
    fn a_step_still_open_when_a_peers_change_arrives_goes_out_as_this_persons() {
        let (mut app, pump) = app_with_vessel("P-1001");
        let vessel = add_vessel(&mut app, "V-1001", "{}");
        connected(&mut app);
        let i = app.active_tab;
        app.push_undo_snapshot(i, "TEST");
        {
            let document = &mut app.tabs[i].scene.document;
            let mut record = PidAuthor::load(document, pump).expect("author");
            record.tag = "P-1005".to_owned();
            record.store(document, pump);
        }
        assert!(app.collab.sent.is_empty(), "the step is still open");

        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "V-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        );
        assert_eq!(app.collab.sent.len(), 1, "this person's step went out");
        let said = last_sent(&app);
        assert_eq!(
            (said[0].kind.as_str(), said[0].object_ref.as_str(), said[0].value.as_str()),
            ("set_tag", "P-1001", "P-1005")
        );
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa");
        // Their step alone is on the stack; undo takes it back and leaves
        // the peer's change.
        undo(&mut app);
        assert_eq!(author(&app, pump).tag, "P-1001");
        assert_eq!(prop(&app, vessel, "设计压力"), "2.5MPa");
    }

    // ── A session that may not write ─────────────────────────────────────

    /// Put the session in the state a `readonly_downgrade` from the server
    /// leaves it in, refused at `level`.
    fn told_it_may_not_write(app: &mut OpenCADStudio, level: &str, code: &str) {
        app.collab.connected = true;
        app.collab.session_id = Some("s-2".to_owned());
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "测试".to_owned(),
            holder: None,
            level: Some(level.to_owned()),
            code: Some(code.to_owned()),
        });
        assert!(app.collab.readonly);
    }

    /// The commands that write and were once let through are refused; the
    /// forms that only read are not.
    #[test]
    fn commands_that_write_are_refused_and_the_forms_that_read_are_not() {
        let (mut app, _) = app_with_vessel("P-1001");
        app.collab.readonly = true;
        for cmd in [
            "PIDLEGEND ON",
            "PIDLEGEND OFF",
            "PIDLEGEND PURGE",
            // Bare, it prompts with ON as the default.
            "PIDLEGEND",
            "PIDGROUP",
            "PIDGROUP GROUP",
            "PIDGROUP TAG P-1",
            "PIDGROUP AUTO",
            "PIDGROUP OFF",
            "AUDIT",
            "AUDIT Yes",
            "MEASURE",
            "FIND",
            "PIDMASTERDATA INIT",
            "PIDMASTERDATA INIT DRAWING",
            "PIDMASTERDATA IMPORT md.json",
            "pidmasterdata embed",
            "PIDCHECK ENABLE DR-014",
            "pidcheck disable DR-014",
            "PIDCHECK WAIVE DR-014 PIPE_LINE|L-1 放空管",
            "PIDCHECK UNWAIVE DR-014 PIPE_LINE|L-1",
            "PIDPUBLISH",
            "PIDPUBLISH D:/out",
            "pidpublish REV 3",
            "PIDPUBLISH ALLOW_OFFLINE",
        ] {
            assert!(app.readonly_refuses_command(cmd), "{cmd} must be refused");
        }
        for cmd in [
            "PIDLEGEND REPORT",
            "pidlegend list",
            "PIDLEGEND EXPORT out.json",
            "PIDMASTERDATA",
            "PIDMASTERDATA STATUS",
            "pidmasterdata export md.json",
            "PIDINDEX",
            "PIDINDEX DUPLICATES",
            "pidindex links",
            "PIDINDEX CANCEL",
            "PIDLOCATE P-1001",
            "pidlocate P-1002 PID-1102",
            "PIDCHECK",
            "PIDCHECK PROJECT",
            "pidcheck pbs U-100",
            "PIDCHECK RULES",
            "PIDCHECK EXPORT drc.json",
            "PIDBOM",
            "PIDBOM PROJECT",
            "pidbom pbs U-100",
            "PIDBOM EXPORT bom.csv",
            "PIDBOM EXPORT bom.xlsx PBS U-100",
            "PIDSNAPSHOT",
            "PIDSNAPSHOT SAVE before review",
            "pidsnapshot list",
            "PIDSNAPSHOT EXPORT 1 snap.json",
            "PIDSNAPSHOT FILE snap.json",
            "PIDDIFF 1",
            "piddiff 1 LIVE EXPORT diff.json",
            "PIDPUBLISH CHECK",
            "pidpublish check D:/out REV 3",
            "PIDPUBLISH LIST",
            "pidpublish verify D:/out/PID-1101-r001",
            "PIDLINE",
            "PIDTAG P-1001",
            "MEASUREGEOM",
            "ZOOM EXTENTS",
        ] {
            assert!(!app.readonly_refuses_command(cmd), "{cmd} only reads");
        }
    }

    /// A tag the plant's data gives may hold spaces, and naming it is a
    /// jump however many words it is (plan 2026-11-20-pid-symbol-identity,
    /// SI-10): a read-only session is let through with it, in quotes or
    /// not, finds the symbol, and leaves the drawing as it was. What
    /// renames, and words that are no tag of an imported symbol on the
    /// sheet, are refused as they were.
    #[test]
    fn a_readonly_session_may_jump_to_an_imported_tag_that_holds_spaces() {
        use crate::app::pid_symbol_block_tests::{opened_with, reference, restate, VALVE};

        let mut app = opened_with(crate::io::pid::sheet_of_a_tagged_valve_and_its_labels());
        let i = app.active_tab;
        let valve = reference(&app, VALVE, (100.0, 100.0));
        restate(
            &mut app,
            valve,
            &[
                ("label", "LG-Magnetic Float Gauge-0203"),
                ("tag_source", "database"),
            ],
        );
        app.collab.document = Some(app.tabs[i].id);
        app.collab.readonly = true;
        let steps = app.tabs[i].history.undo_stack.len();
        let entities = app.tabs[i].scene.document.entities().count();

        for cmd in [
            "PIDTAG LG-Magnetic Float Gauge-0203",
            "PIDTAG lg-magnetic  float gauge-0203",
            r#"PIDTAG "LG-Magnetic Float Gauge-0203""#,
        ] {
            assert!(!app.readonly_refuses_command(cmd), "{cmd} only reads");
            app.tabs[i].scene.deselect_all();
            let _ = app.run_command_line(cmd);
            app.finish_all_pending_history();
            assert!(
                app.tabs[i]
                    .scene
                    .selected_handles_in_order()
                    .contains(&valve),
                "{cmd} finds the valve"
            );
            assert_eq!(
                app.tabs[i].history.undo_stack.len(),
                steps,
                "{cmd} made a step in a session that may not write"
            );
            assert_eq!(
                app.tabs[i].scene.document.entities().count(),
                entities,
                "{cmd} added or removed an entity"
            );
        }
        for cmd in [
            "PIDTAG GV-101 GV-109",
            "PIDTAG LG-Magnetic Float",
            "PIDTAG LG-Magnetic Float Gauge-0204",
            r#"PIDTAG "LG-Magnetic Float Gauge-0204""#,
        ] {
            assert!(app.readonly_refuses_command(cmd), "{cmd} must be refused");
        }
    }

    /// What the list lets through, run as it would be typed, leaves the
    /// drawing as it was: no step on the undo stack, the same entities, the
    /// same record. (`QUIT`, `EXIT` and `SAVEAS` are not run here: the first
    /// two leave the program and the third asks for a file name.)
    #[test]
    fn every_command_a_readonly_session_may_run_leaves_the_drawing_as_it_was() {
        for verb in READONLY_ALLOWED {
            if matches!(*verb, "QUIT" | "EXIT" | "SAVEAS") {
                continue;
            }
            let (mut app, vessel) = app_with_vessel("P-1001");
            let i = app.active_tab;
            app.collab.document = Some(app.tabs[i].id);
            app.collab.readonly = true;
            let steps = app.tabs[i].history.undo_stack.len();
            let entities = app.tabs[i].scene.document.entities().count();
            let record = author(&app, vessel);

            let cmd = match *verb {
                "PIDLEGEND" => "PIDLEGEND REPORT".to_owned(),
                "PIDTAG" => "PIDTAG P-1001".to_owned(),
                "PIDLOCATE" => "PIDLOCATE P-1001".to_owned(),
                "PIDPUBLISH" => "PIDPUBLISH CHECK".to_owned(),
                other => other.to_owned(),
            };
            assert!(!app.readonly_refuses_command(&cmd), "{cmd} is on the list");
            let _ = app.run_command_line(&cmd);
            app.finish_all_pending_history();

            let i = app.active_tab;
            assert_eq!(
                app.tabs[i].history.undo_stack.len(),
                steps,
                "{cmd} made a step in a session that may not write"
            );
            assert_eq!(
                app.tabs[i].scene.document.entities().count(),
                entities,
                "{cmd} added or removed an entity"
            );
            assert_eq!(author(&app, vessel), record, "{cmd} rewrote the record");
        }
    }

    /// The lock itself. An edit that reaches the drawing by a door the
    /// command gate does not watch is taken back as its step closes: the
    /// drawing and both history stacks are as they were, nothing goes to the
    /// room, the caller is told it was refused and by which tier, and the
    /// attempt is on record with the object it was aimed at.
    #[test]
    fn a_write_that_gets_past_the_command_gate_is_taken_back() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        // A step of one's own from before, and one taken back, so there is
        // an undo stack and a redo stack to be left alone.
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa"}"#);
        put_props(&mut app, pump, r#"{"设计压力":"2.0MPa"}"#);
        undo(&mut app);
        let i = app.active_tab;
        let undo_steps = app.tabs[i].history.undo_stack.len();
        let redo_steps = app.tabs[i].history.redo_stack.len();
        assert_eq!((undo_steps, redo_steps), (1, 1));
        let sent = app.collab.sent.len();

        told_it_may_not_write(&mut app, "role", "PERM_DENIED_ROLE");

        // Over REST: the contract's own write channel for properties.
        let state = app.control_state();
        let (response, task) = app.control_request(serde_json::json!({
            "protocol": 1,
            "op": "xdata_set",
            "request_id": "refused-1",
            "client_id": "test",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "handles": [format!("{:X}", pump.value())],
            "app": "PID_AUTHOR",
            "data": [{"code": 1000, "value": r#"props={"设计压力":"9.9MPa"}"#}],
        }));
        app.drive_headless_task(task).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["code"], "PERM_DENIED_ROLE", "{response}");
        assert_eq!(response["level"], "role", "{response}");
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa", "the write was taken back");

        // By a door of its own: a step opened, the record rewritten, closed.
        app.push_undo_snapshot(i, "PANEL");
        {
            let document = &mut app.tabs[i].scene.document;
            let mut record = PidAuthor::load(document, pump).expect("author");
            record.tag = "P-1009".to_owned();
            record.store(document, pump);
        }
        app.finish_all_pending_history();
        assert_eq!(author(&app, pump).tag, "P-1001", "the retag was taken back");

        assert_eq!(app.tabs[i].history.undo_stack.len(), undo_steps);
        assert_eq!(app.tabs[i].history.redo_stack.len(), redo_steps);
        assert_eq!(app.collab.sent.len(), sent, "nothing goes to the room");
        let attempts = &app.collab.attempts;
        assert_eq!(attempts.len(), 2, "{attempts:?}");
        assert_eq!(attempts[0].0, "P-1001");
        assert!(attempts[0].1.contains("设计压力"), "{attempts:?}");
        assert_eq!(attempts[1].0, "P-1001");
        assert!(attempts[1].1.contains("P-1009"), "{attempts:?}");

        // Nor does it take its own earlier steps back, or put them on again.
        undo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa");
        redo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa");
        assert_eq!(app.tabs[i].history.undo_stack.len(), undo_steps);
        assert_eq!(app.tabs[i].history.redo_stack.len(), redo_steps);
    }

    /// A command the gate turns away answers its caller with the code and
    /// the tier of the refusal, and reports the attempt with the object the
    /// command names — spelled as the drawing spells it.
    #[test]
    fn a_refused_command_says_which_tier_refused_it_and_is_reported() {
        let (mut app, pump) = app_with_vessel("P-1001");
        told_it_may_not_write(&mut app, "role", "PERM_DENIED_ROLE");
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG p-1001 P-1009"}"#);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "PERM_DENIED_ROLE", "{refused}");
        assert_eq!(refused["level"], "role", "{refused}");
        assert_eq!(author(&app, pump).tag, "P-1001");
        assert_eq!(
            app.collab.attempts,
            [("P-1001".to_owned(), "PIDTAG p-1001 P-1009".to_owned())]
        );

        // At the state tier the code is the one the gate has always given.
        let (mut app, _) = app_with_vessel("P-1001");
        told_it_may_not_write(&mut app, "state", "PERM_DENIED_STATE");
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG P-1001 P-1009"}"#);
        assert_eq!(refused["code"], "COLLAB_READONLY", "{refused}");
        assert_eq!(refused["level"], "state", "{refused}");
    }

    /// A session that may not write does not come to write by losing its
    /// socket. It used to: `readonly` was cleared on every disconnect.
    #[test]
    fn a_session_that_may_not_write_still_may_not_while_disconnected() {
        let (mut app, pump) = app_with_vessel("P-1001");
        let i = app.active_tab;
        app.collab.document = Some(app.tabs[i].id);
        told_it_may_not_write(&mut app, "role", "PERM_DENIED_ROLE");
        app.collab.apply(CollabEvent::Disconnected {
            reason: "read error: ConnectionReset".to_owned(),
            will_retry: true,
        });
        assert!(app.collab.readonly, "the downgrade was dropped with the socket");
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG P-1001 P-1009"}"#);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(author(&app, pump).tag, "P-1001");

        // A new session decides afresh: the server says so right behind the
        // welcome if it still may not, and says nothing if it may.
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-3".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        assert!(!app.collab.readonly);
    }

    fn select(app: &mut OpenCADStudio, handle: document::Handle) {
        let _ = app.automation_op(r#"{"op":"select","clear":true}"#);
        let state = app.control_state();
        let (response, task) = app.control_request(serde_json::json!({
            "protocol": 1,
            "op": "select",
            "request_id": format!("collab-select-{:X}", handle.value()),
            "client_id": "test",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "handles": [format!("{:X}", handle.value())],
        }));
        app.drive_headless_task(task).unwrap();
        assert_eq!(response["ok"], true, "{response}");
    }

    /// The Properties panel's rows for the drawing in front, as they stand.
    fn panel_rows(app: &OpenCADStudio) -> Vec<crate::scene::model::object::Property> {
        app.tabs[app.active_tab]
            .properties
            .sections
            .iter()
            .flat_map(|section| section.props.iter().cloned())
            .collect()
    }

    fn panel_row(app: &OpenCADStudio, label: &str) -> crate::scene::model::object::PropValue {
        panel_rows(app)
            .into_iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no row labelled {label}"))
            .value
    }

    /// What the Properties panel shows of an object in a session that may
    /// not write: its values, as values. An input that takes what is typed
    /// and then gives it back — the step is undone as it closes — is worse
    /// than no input. The panel changes over when the session is told, with
    /// nobody touching the selection; a peer's change still shows in it; and
    /// a drawing the session is not on keeps its inputs.
    #[test]
    fn a_session_that_may_not_write_shows_an_objects_properties_as_values() {
        use crate::scene::model::object::PropValue;
        let is_value = |value: &PropValue| {
            matches!(value, PropValue::ReadOnly(_) | PropValue::ReadOnlyWithTooltip { .. })
        };
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        let session_tab = app.active_tab;
        app.collab.document = Some(app.tabs[session_tab].id);
        select(&mut app, pump);
        app.refresh_properties();
        assert_eq!(panel_row(&app, "设计压力"), PropValue::PlainText("1.0MPa".to_owned()));
        assert!(
            panel_rows(&app).iter().any(|row| !is_value(&row.value)),
            "a session that writes has inputs"
        );

        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::ReadonlyDowngrade {
            reason: "角色无编辑权限".to_owned(),
            holder: None,
            level: Some("role".to_owned()),
            code: Some("PERM_DENIED_ROLE".to_owned()),
        }));
        let rows = panel_rows(&app);
        assert!(!rows.is_empty());
        for row in &rows {
            assert!(is_value(&row.value), "{} is still an input: {:?}", row.label, row.value);
        }
        assert_eq!(panel_row(&app, "设计压力"), PropValue::ReadOnly("1.0MPa".to_owned()));

        // A peer's change shows, and is still not an input.
        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::OpApplied {
            op_id: "op-peer-1".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa",
            })],
            user_id: None,
            display_name: None,
        }));
        assert_eq!(prop(&app, pump, "设计压力"), "2.5MPa");
        assert_eq!(panel_row(&app, "设计压力"), PropValue::ReadOnly("2.5MPa".to_owned()));

        // Another drawing is not the session's, and is edited as ever.
        let (_, other) = another_drawing_in_front(&mut app, "P-2001");
        select(&mut app, other);
        app.refresh_properties();
        assert!(
            panel_rows(&app).iter().any(|row| !is_value(&row.value)),
            "a drawing the session is not on lost its inputs"
        );

        // Told it may write again — a new session the server does not
        // downgrade — the inputs come back, again without being asked.
        app.active_tab = session_tab;
        app.refresh_properties();
        assert_eq!(panel_row(&app, "设计压力"), PropValue::ReadOnly("2.5MPa".to_owned()));
        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::Connected {
            session_id: "s-9".to_owned(),
            base_revision: 1,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        }));
        assert_eq!(panel_row(&app, "设计压力"), PropValue::PlainText("2.5MPa".to_owned()));
    }

    /// `VAL-CROSS-007`, this client's part. A write that tied with a peer's
    /// stays in this drawing while the entry waits — it is what this person
    /// wrote — and the panel lists the entry. The ruling arrives as an op:
    /// the field takes the ruled value here too, the Properties panel shows
    /// it, and the entry is off the panel.
    #[test]
    fn a_ruling_reaches_the_drawing_and_takes_the_entry_off_the_panel() {
        use crate::app::collab::PendingConflict;
        use crate::scene::model::object::PropValue;
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa"}"#);
        assert_eq!(app.collab.sent.len(), 1, "this person's write went out");

        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::Conflict(
            PendingConflict {
                conflict_id: "c-1".to_owned(),
                object_ref: "P-1001".to_owned(),
                field: "设计压力".to_owned(),
                value_a: "1.6MPa".to_owned(),
                value_b: "2.5MPa".to_owned(),
            },
        )));
        assert_eq!(app.pid_collab_view.conflicts.len(), 1, "the panel was not told");
        assert_eq!(app.pid_collab_view.conflicts[0].subject(), "P-1001 · 设计压力");
        assert_eq!(app.pid_collab_view.conflicts[0].values(), "1.6MPa ↔ 2.5MPa");
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa");
        assert_eq!(app.collab.base_revision, 0, "a conflict is not a revision");

        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::OpApplied {
            op_id: "resolve-c-1".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力",
                "value": "2.5MPa", "old_value": "1.0MPa",
            })],
            user_id: None,
            display_name: None,
        }));
        assert_eq!(prop(&app, pump, "设计压力"), "2.5MPa");
        assert!(app.pid_collab_view.conflicts.is_empty(), "the entry is still listed");
        assert_eq!(app.collab.last_apply_error, None);
        assert_eq!(app.collab.diverged, None);
        select(&mut app, pump);
        app.refresh_properties();
        assert_eq!(panel_row(&app, "设计压力"), PropValue::PlainText("2.5MPa".to_owned()));
    }

    // ── Symbols that come and go ─────────────────────────────────────────

    const GATE_VALVE: &str = "PID_SYM_valve_gate_valve";

    /// Every symbol on the drawing in front as `(tag, handle, x, y)`.
    fn symbols(app: &OpenCADStudio) -> Vec<(String, document::Handle, f64, f64)> {
        let document = &app.tabs[app.active_tab].scene.document;
        let mut found: Vec<_> = document
            .entities()
            .filter_map(|entity| {
                let document::EntityType::Insert(insert) = entity else {
                    return None;
                };
                let author = PidAuthor::load(document, insert.common.handle)?;
                Some((
                    author.tag,
                    insert.common.handle,
                    insert.insert_point.x,
                    insert.insert_point.y,
                ))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        found
    }

    fn tags(app: &OpenCADStudio) -> Vec<String> {
        symbols(app).into_iter().map(|(tag, ..)| tag).collect()
    }

    fn sorted(tags: &[&str]) -> Vec<String> {
        let mut tags: Vec<String> = tags.iter().map(|tag| (*tag).to_owned()).collect();
        tags.sort();
        tags
    }

    fn peers_valve(tag: &str, x: f64) -> serde_json::Value {
        serde_json::json!({
            "kind": "insert_symbol", "object_ref": tag, "tag": tag, "type": "VALVE",
            "block": GATE_VALVE, "position": [x, 200.0, 0.0], "rotation": 0.0, "scale": 1.0,
            "ports": "in,out", "props": {"公称直径": "DN50"}, "tag_source": "auto",
            // The sender's own handle for it: nothing of this drawing's.
            "handle": "FFF0",
        })
    }

    /// A symbol a peer placed is placed here — where they put it, as what
    /// they put, under the tag the room knows it by — and one they erased is
    /// erased here. Neither is a step of this person's. A new symbol used
    /// to exist in its maker's drawing only: nothing was said of it, and
    /// every later change to it found nothing to change in anybody else's.
    #[test]
    fn a_peers_new_symbol_is_placed_here_and_one_they_erase_goes() {
        let (mut app, _) = app_with_vessel("P-1001");
        connected(&mut app);
        let i = app.active_tab;
        let steps = app.tabs[i].history.undo_stack.len();

        remote(&mut app, 1, vec![peers_valve("V-1001", 100.0)]);
        assert_eq!(app.collab.last_apply_error, None);
        assert_eq!(app.collab.diverged, None);
        let placed: Vec<_> = symbols(&app)
            .into_iter()
            .filter(|(tag, ..)| tag == "V-1001")
            .collect();
        assert_eq!(placed.len(), 1, "{placed:?}");
        let (_, valve, x, y) = placed[0].clone();
        assert_eq!((x, y), (100.0, 200.0));
        let record = author(&app, valve);
        assert_eq!(record.type_code, "VALVE");
        assert_eq!(record.tag_source, "auto");
        assert_eq!(prop(&app, valve, "公称直径"), "DN50");
        assert!(
            app.tabs[i].scene.document.block_records.contains(GATE_VALVE),
            "the symbol's block came with it"
        );
        assert_eq!(app.tabs[i].history.undo_stack.len(), steps, "not this person's step");
        assert!(app.collab.sent.is_empty(), "nor said back to the room");

        // A change to it now finds it, like any object's.
        remote(
            &mut app,
            2,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "V-1001", "field": "公称直径", "value": "DN80",
            })],
        );
        assert_eq!(prop(&app, valve, "公称直径"), "DN80");

        remote(
            &mut app,
            3,
            vec![serde_json::json!({"kind": "delete_object", "object_ref": "V-1001"})],
        );
        assert_eq!(tags(&app), ["P-1001"], "the erased symbol is still here");
        assert_eq!(app.collab.last_apply_error, None);

        // Erasing what this drawing does not have is said, like any change
        // it could not follow.
        remote(
            &mut app,
            4,
            vec![serde_json::json!({"kind": "delete_object", "object_ref": "V-7777"})],
        );
        assert!(app.collab.diverged.is_some());
    }

    /// `VAL-CROSS-001`: a symbol placed from the command line goes to the
    /// room as what it is and where it stands; the property set on it next
    /// goes as a change to that tag; and taking the placement back says the
    /// symbol is gone.
    #[test]
    fn placing_a_symbol_and_setting_its_property_are_said_to_the_room() {
        let (mut app, _) = app_with_vessel("P-1001");
        connected(&mut app);

        let placed = app.automation_op(r#"{"op":"run","cmd":"PIDINSERT VALVE 100,200"}"#);
        assert_eq!(placed["ok"], true, "{placed}");
        let valves: Vec<_> = symbols(&app)
            .into_iter()
            .filter(|(tag, ..)| tag != "P-1001")
            .collect();
        assert_eq!(valves.len(), 1, "{valves:?}");
        let (tag, valve, x, y) = valves[0].clone();
        assert!(!tag.is_empty(), "the valve has no tag");
        assert_eq!((x, y), (100.0, 200.0));
        assert_eq!(author(&app, valve).type_code, "VALVE");

        assert_eq!(app.collab.sent.len(), 1, "the placement was not said");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0].kind, "insert_symbol");
        assert_eq!(said[0].object_ref, tag);
        assert_eq!(said[0].handles, [valve]);
        let what = said[0].placed.as_ref().expect("what was placed");
        assert_eq!(what.type_code, "VALVE");
        assert_eq!(what.position, [100.0, 200.0, 0.0]);
        let wire = app.collab.sent.last().expect("payload");
        assert_eq!(wire["tag"], tag.as_str(), "the key the server reads a new symbol by");
        assert_eq!(wire["type"], "VALVE");

        // Said of what was just placed, with nothing named.
        let set = app.automation_op(r#"{"op":"run","cmd":"PIDSETPROP 公称直径 DN50"}"#);
        assert_eq!(set["ok"], true, "{set}");
        assert_eq!(prop(&app, valve, "公称直径"), "DN50");
        assert_eq!(app.collab.sent.len(), 2);
        let said = last_sent(&app);
        assert_eq!(
            (said[0].kind.as_str(), said[0].object_ref.as_str(), said[0].field.as_str(), said[0].value.as_str()),
            ("set_property", tag.as_str(), "公称直径", "DN50")
        );

        // By tag, whatever is selected.
        let set = app.automation_op(r#"{"op":"run","cmd":"PIDSETPROP p-1001 设计温度 80℃"}"#);
        assert_eq!(set["ok"], true, "{set}");
        assert_eq!(app.collab.sent.len(), 3);
        let said = last_sent(&app);
        assert_eq!(
            (said[0].object_ref.as_str(), said[0].field.as_str(), said[0].value.as_str()),
            ("P-1001", "设计温度", "80℃")
        );
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDSETPROP Z-9 设计温度 80℃"}"#);
        assert_eq!(refused["ok"], false, "{refused}");

        // Back out of all three; the last one takes the valve away.
        undo(&mut app);
        undo(&mut app);
        undo(&mut app);
        assert_eq!(tags(&app), ["P-1001"]);
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!((said[0].kind.as_str(), said[0].object_ref.as_str()), ("delete_object", tag.as_str()));
        redo(&mut app);
        assert_eq!(last_sent(&app)[0].kind, "insert_symbol");
    }

    /// `VAL-CROSS-010`, this session's part. Two people each place a valve
    /// and each numbers it the same. The peer's reaches the room first and
    /// arrives here under that tag; this session's own op then comes back
    /// saying the server took the symbol under the next one. The symbol
    /// this session placed is renamed — that one, not the peer's, though
    /// both carried the tag — and it is still a tag nobody typed. Taking
    /// the placement back afterwards erases it by the name it has now.
    #[test]
    fn a_symbol_the_server_renumbered_is_renamed_where_it_was_placed() {
        let (mut app, _) = app_with_vessel("P-1001");
        connected(&mut app);
        let placed = app.automation_op(r#"{"op":"run","cmd":"PIDINSERT VALVE 100,200"}"#);
        assert_eq!(placed["ok"], true, "{placed}");
        let (tag, own, ..) = symbols(&app)
            .into_iter()
            .find(|(tag, ..)| tag != "P-1001")
            .expect("the valve");
        let op_id = app.collab.own_ops.back().cloned().expect("the op in flight");

        // The peer's valve, under the same tag, got there first — and under
        // the same handle: both opened the same file, so both drawings gave
        // their next entity the same one. That says nothing about it being
        // this session's valve (it was once taken to, and the peer's valve
        // was then never placed here).
        let mut peers = peers_valve(&tag, 300.0);
        peers["handle"] = serde_json::json!(format!("{:X}", own.value()));
        remote(&mut app, 1, vec![peers]);
        assert_eq!(tags(&app), sorted(&["P-1001", &tag, &tag]));

        // This session's own op, as the server took it.
        let mut echoed = app.collab.sent.last().expect("payload").clone();
        echoed["object_ref"] = serde_json::json!("V-9002");
        echoed["tag"] = serde_json::json!("V-9002");
        echoed["reassigned_from"] = serde_json::json!(tag);
        let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::OpApplied {
            op_id,
            revision: 2,
            changes: vec![echoed],
            user_id: None,
            display_name: None,
        }));
        assert_eq!(author(&app, own).tag, "V-9002", "this session's symbol was not renamed");
        assert_eq!(author(&app, own).tag_source, "auto", "nobody typed that tag");
        assert_eq!(tags(&app), sorted(&["P-1001", "V-9002", &tag]));
        assert_eq!(app.collab.last_apply_error, None);
        assert_eq!(app.collab.diverged, None);

        undo(&mut app);
        assert_eq!(tags(&app), sorted(&["P-1001", &tag]), "the peer's valve stays");
        let said = last_sent(&app);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(
            (said[0].kind.as_str(), said[0].object_ref.as_str()),
            ("delete_object", "V-9002"),
            "erased by the name it has now, not the one it was placed under"
        );
    }

    // ── A tag somebody else holds ────────────────────────────────────────

    /// What a refused claim tells this session: who holds the tag.
    fn held_by(app: &mut OpenCADStudio, tag: &str, holder: &str) {
        app.collab.apply(CollabEvent::ClaimDenied {
            tag: tag.to_owned(),
            holder: holder.to_owned(),
            expires_at: 1_900_000_000,
        });
        assert_eq!(app.collab.claims.get(tag).map(String::as_str), Some(holder));
    }

    /// `VAL-CROSS-002`: a write to an object whose tag another user holds
    /// is taken back as its step closes, whatever door it came in by. The
    /// caller is told so under the code for a held tag, the drawing and
    /// the history are as they were, nothing goes to the room, and the
    /// attempt is reported under the tag. A claim used to keep out a second
    /// claim and nothing else.
    #[test]
    fn a_write_to_a_tag_somebody_else_holds_is_taken_back() {
        let (mut app, other) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        app.collab.user_id = "u2".to_owned();
        // A step of this person's own, so there is a stack to be left alone.
        put_props(&mut app, other, r#"{"设计压力":"3.0MPa"}"#);
        let i = app.active_tab;
        assert_eq!(app.collab.sent.len(), 1);
        assert_eq!(app.tabs[i].history.undo_stack.len(), 1);
        held_by(&mut app, "P-1001", "u1");
        held_by(&mut app, "P-2000", "u1");

        // Over REST, the contract's write channel for properties.
        let state = app.control_state();
        let (response, task) = app.control_request(serde_json::json!({
            "protocol": 1,
            "op": "xdata_set",
            "request_id": "held-1",
            "client_id": "test",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "handles": [format!("{:X}", pump.value())],
            "app": "PID_AUTHOR",
            "data": [{"code": 1000, "value": r#"props={"设计压力":"9.9MPa"}"#}],
        }));
        app.drive_headless_task(task).unwrap();
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(response["code"], "CLAIM_HELD", "{response}");
        assert!(response["level"].is_null(), "a claim is not a tier: {response}");
        assert_eq!(prop(&app, pump, "设计压力"), "1.0MPa", "the write was taken back");

        // By a command: the tag itself, and another object given a held tag.
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG P-1001 P-1009"}"#);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "CLAIM_HELD", "{refused}");
        assert_eq!(author(&app, pump).tag, "P-1001");
        let refused = app.automation_op(r#"{"op":"run","cmd":"PIDTAG X-1 P-2000"}"#);
        assert_eq!(refused["code"], "CLAIM_HELD", "{refused}");
        assert_eq!(author(&app, other).tag, "X-1");

        assert_eq!(app.collab.sent.len(), 1, "nothing goes to the room");
        assert_eq!(app.tabs[i].history.undo_stack.len(), 1);
        assert_eq!(app.tabs[i].history.redo_stack.len(), 0);
        let reported: Vec<&str> = app
            .collab
            .attempts
            .iter()
            .map(|(object_ref, _)| object_ref.as_str())
            .collect();
        assert_eq!(reported, ["P-1001", "P-1001", "P-2000"]);
        assert!(app.collab.attempts[0].1.contains("设计压力"), "{:?}", app.collab.attempts);

        // An object nobody holds is written as ever.
        put_props(&mut app, other, r#"{"设计压力":"3.5MPa"}"#);
        assert_eq!(app.collab.sent.len(), 2);

        // Let go of, the tag is anybody's; held by this person, it is theirs.
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".to_owned(),
            reason: "released".to_owned(),
        });
        put_props(&mut app, pump, r#"{"设计压力":"2.5MPa"}"#);
        assert_eq!(prop(&app, pump, "设计压力"), "2.5MPa");
        app.collab.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        put_props(&mut app, pump, r#"{"设计压力":"2.6MPa"}"#);
        assert_eq!(prop(&app, pump, "设计压力"), "2.6MPa");
        assert_eq!(app.collab.sent.len(), 4);
    }

    /// Taking a step back is a write like any other: an undo or a redo that
    /// would change an object somebody has claimed since is refused, stacks
    /// untouched, until the claim is gone.
    #[test]
    fn undo_does_not_reach_into_a_tag_somebody_has_since_claimed() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        app.collab.user_id = "u2".to_owned();
        put_props(&mut app, pump, r#"{"设计压力":"1.6MPa"}"#);
        let i = app.active_tab;
        held_by(&mut app, "P-1001", "u1");

        undo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.6MPa", "the undo went through");
        assert_eq!(app.tabs[i].history.undo_stack.len(), 1);
        assert_eq!(app.tabs[i].history.redo_stack.len(), 0);
        assert_eq!(app.collab.sent.len(), 1, "nothing goes to the room");
        assert_eq!(app.collab.attempts.len(), 1, "{:?}", app.collab.attempts);
        assert_eq!(app.collab.attempts[0].0, "P-1001");

        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".to_owned(),
            reason: "expired".to_owned(),
        });
        undo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.0MPa");

        // And the same on the way forward.
        held_by(&mut app, "P-1001", "u1");
        redo(&mut app);
        assert_eq!(prop(&app, pump, "设计压力"), "1.0MPa", "the redo went through");
        assert_eq!(app.tabs[i].history.redo_stack.len(), 1);
    }

    /// The holder's own change arrives like any peer's: a claim keeps this
    /// person's hands off the object, not the object from following the room.
    #[test]
    fn a_held_tag_still_takes_the_changes_its_holder_makes() {
        let (mut app, _) = app_with_vessel("X-1");
        let pump = add_vessel(&mut app, "P-1001", r#"{"设计压力":"1.0MPa"}"#);
        connected(&mut app);
        app.collab.user_id = "u2".to_owned();
        held_by(&mut app, "P-1001", "u1");
        remote(
            &mut app,
            1,
            vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        );
        assert_eq!(prop(&app, pump, "设计压力"), "2.5MPa");
        assert_eq!(app.collab.last_apply_error, None);
    }

    /// A peer's change this drawing cannot follow — here, to an object it
    /// does not have — leaves it short of the room. The baseline moved all
    /// the same, so nothing else would stop the next local edit from going
    /// out on top of the difference: the session is marked and sends no more.
    #[test]
    fn a_change_this_drawing_cannot_follow_stops_the_sessions_writes() {
        let (mut app, vessel) = app_with_vessel("P-1001");
        app.collab.connected = true;
        app.collab.session_id = Some("s-1".to_owned());
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-9001", "field": "设计压力", "value": "2.5MPa",
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert_eq!(app.collab.base_revision, 1, "the baseline is the server's count");
        let reason = app.collab.diverged.clone().expect("the session is marked");
        assert!(reason.contains("P-9001"), "{reason}");
        assert_eq!(app.collab.status_json()["diverged"], reason.as_str());

        // The local edit still happens — it is this person's drawing — but
        // it does not go out.
        let _ = app.run_command_line("PIDTAG P-1001 P-1005");
        app.finish_all_pending_history();
        assert_eq!(
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, vessel)
                .expect("author")
                .tag,
            "P-1005"
        );
        assert!(
            app.collab.own_ops.is_empty(),
            "an op went out from a drawing known to differ: {:?}",
            app.collab.own_ops
        );
    }

    /// An op this client can read only in part is applied as far as it can
    /// be, and said to have left the drawing short: the change it could not
    /// read took effect in the room all the same. It used to be passed over
    /// in silence as long as anything else in the op could be applied.
    #[test]
    fn an_op_read_only_in_part_is_applied_as_far_as_it_goes_and_said_to_be_short() {
        let (mut app, vessel) = app_with_vessel("P-1001");
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: vec![
                serde_json::json!({
                    "kind": "set_tag", "object_ref": "P-1001", "field": "tag", "value": "P-1009",
                }),
                serde_json::json!({"kind": "insert_symbol", "object_ref": "V-1001"}),
            ],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert_eq!(
            PidAuthor::load(&app.tabs[app.active_tab].scene.document, vessel)
                .expect("author")
                .tag,
            "P-1009",
            "what could be read is applied"
        );
        let said = app.collab.last_apply_error.clone().expect("it is reported");
        assert!(said.contains("insert_symbol"), "{said}");
        assert!(app.collab.diverged.is_some(), "and the session is marked");
    }

    /// An op none of whose changes this client can read is the same thing: the
    /// room changed in a way this drawing did not.
    #[test]
    fn an_op_this_client_cannot_read_marks_the_session_too() {
        let (mut app, _) = app_with_vessel("P-1001");
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: vec![serde_json::json!({"kind": "something_new", "object_ref": "P-1001"})],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert!(app.collab.diverged.is_some());
        assert!(app.collab.last_apply_error.is_some());
    }

    /// An editor started on a file has a session for that file. While it is
    /// still loading, a drawing that happens to be in front — another file of
    /// the same launch, a new one — is not it: the change waits, and goes to
    /// the tab showing the file once there is one.
    #[test]
    fn a_session_started_on_a_file_waits_for_that_file_not_the_tab_in_front() {
        let dir = std::env::temp_dir().join(format!("ocs-collab-bind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("PID-1101.dwg");
        std::fs::write(&file, b"").expect("the file the launch named");

        let mut app = OpenCADStudio::new_for_test();
        app.collab.document_path = Some(file.clone());
        // Something else is open and in front before the file has loaded.
        let (other_tab, other_vessel) = another_drawing_in_front(&mut app, "P-1001");
        remote_retag(&mut app, "op-early", 1, "P-1001", "P-1009");
        assert_eq!(app.collab.pending_remote.len(), 1, "it waits for the file");
        assert_eq!(app.collab.document, None, "the tab in front is not the session's");
        assert_eq!(
            PidAuthor::load(&app.tabs[other_tab].scene.document, other_vessel)
                .expect("author")
                .tag,
            "P-1001"
        );

        // The file finishes loading into a tab of its own, behind.
        let (file_tab, file_vessel) = another_drawing_in_front(&mut app, "P-1001");
        app.tabs[file_tab].current_path = Some(file);
        app.active_tab = other_tab;
        let _ = app.update(crate::app::Message::Tick(iced::time::Instant::now()));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(app.collab.document, Some(app.tabs[file_tab].id));
        assert!(app.collab.pending_remote.is_empty());
        assert_eq!(
            PidAuthor::load(&app.tabs[file_tab].scene.document, file_vessel)
                .expect("author")
                .tag,
            "P-1009",
            "the change reaches the file the session was started on"
        );
        assert_eq!(
            PidAuthor::load(&app.tabs[other_tab].scene.document, other_vessel)
                .expect("author")
                .tag,
            "P-1001",
            "and no other drawing"
        );
    }

    /// Once the session's drawing is closed there is nowhere to put a peer's
    /// change. It stays queued and is said to be; it does not move on to
    /// whatever drawing is open instead.
    #[test]
    fn a_change_for_a_closed_drawing_stays_queued_and_touches_no_other() {
        let (mut app, _) = app_with_vessel("P-1001");
        remote_retag(&mut app, "op-peer-a", 1, "P-1001", "P-1005");
        let session_id = app.tabs[app.active_tab].id;
        let (_, copy_vessel) = another_drawing_in_front(&mut app, "P-1005");
        let copy_id = app.tabs[app.active_tab].id;
        // The session's drawing is closed; the copy is what is left. (Saved
        // first as far as the tab is concerned, so the close is not held up
        // by the question about unsaved changes.)
        let session_tab = app
            .tabs
            .iter()
            .position(|tab| tab.id == session_id)
            .expect("the session's tab");
        app.tabs[session_tab].dirty = false;
        let _ = app.update(crate::app::Message::TabClose(session_id));
        assert!(
            app.tabs.iter().all(|tab| tab.id != session_id),
            "the session's drawing is closed"
        );
        let copy_tab = app
            .tabs
            .iter()
            .position(|tab| tab.id == copy_id)
            .expect("the copy is still open");
        remote_retag(&mut app, "op-peer-b", 2, "P-1005", "P-1009");
        assert_eq!(app.collab.pending_remote.len(), 1);
        assert!(app
            .collab
            .last_apply_error
            .as_deref()
            .is_some_and(|said| said.contains("已经关闭")));
        assert_eq!(
            PidAuthor::load(&app.tabs[copy_tab].scene.document, copy_vessel)
                .expect("author")
                .tag,
            "P-1005"
        );
    }

    /// A claim is on a tag of the session's drawing. Another sheet open
    /// beside it that carries the same tag is not occupied by anyone.
    #[test]
    fn a_claim_colours_the_sessions_drawing_only() {
        let (mut app, vessel) = app_with_vessel("P-1001");
        let session_tab = app.active_tab;
        app.collab.document = Some(app.tabs[session_tab].id);
        let (copy_tab, _) = another_drawing_in_front(&mut app, "P-1001");
        app.collab.claims.insert("P-1001".to_owned(), "u1".to_owned());
        app.sync_claim_colors();
        let coloured = app.tabs[session_tab]
            .scene
            .claim_display
            .as_ref()
            .expect("the session's drawing shows the claim");
        assert!(coloured.handles.contains(&vessel));
        assert!(
            app.tabs[copy_tab].scene.claim_display.is_none(),
            "a sheet outside the session was coloured as occupied"
        );
    }

    /// A read-only session may not write to its drawing. Another drawing
    /// open beside it is not in the session and stays writable.
    #[test]
    fn a_readonly_session_leaves_other_drawings_writable() {
        let (mut app, _) = app_with_vessel("P-1001");
        // Settle which drawing the session is about, then lose write access.
        remote_retag(&mut app, "op-peer-a", 1, "P-1001", "P-1005");
        app.collab.readonly = true;
        assert!(app.readonly_refuses_command("PIDTAG P-1005 P-1009"));

        let _ = another_drawing_in_front(&mut app, "P-2001");
        assert!(
            !app.readonly_refuses_command("PIDTAG P-2001 P-2009"),
            "a drawing outside the session was refused a write"
        );
    }
}

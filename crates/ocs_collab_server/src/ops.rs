//! Op intake: the three-step judgment (`ocs_collab::conflict`) wired to the
//! store, the room and the tie window.
//!
//! An op used to be taken only when its `base_revision` was the drawing's
//! current revision, and refused otherwise. Field writes are now judged
//! instead (`architecture.md` §3.3):
//!
//! 1. made against an older baseline, on a field nobody else wrote since: it
//!    merges, and takes the next revision;
//! 2. made against an older baseline, on a field another session wrote since:
//!    it yields — `error` `OP_SUPERSEDED` to its sender, the value it would
//!    have overwritten in the audit log, no conflict entry;
//! 3. two of them made against the current revision, same field, different
//!    values, arriving inside the tie window: neither takes effect, a
//!    conflict entry is made and the room is told (`conflict`).
//!
//! Step 3 needs the first of the two not to have taken effect when the second
//! arrives, so with a tie window configured (`--tie-window-ms`) an op made
//! against the current revision is *held* for that long before it commits.
//! With no window (the default) nothing is held, every op commits on arrival,
//! and two writes to one field are always decided by arrival: the second is
//! behind the first.
//!
//! Ops of any other kind are opaque here and keep the old rule: current
//! revision or `STALE_BASE_REVISION`.

use std::collections::HashMap;
use std::time::Duration;

use ocs_collab::conflict::{
    Change, DrawingModel, Verdict, KIND_DELETE_OBJECT, KIND_PAIR_CONNECTOR, KIND_SET_TAG,
};
use ocs_collab::link::{Rooms, LINK_SESSION_PREFIX};
use ocs_collab::protocol::codes;
use ocs_collab::{new_conflict_id, ServerMessage, Session};
use serde_json::Value;
use tokio::time::Instant;

use crate::store::{ConflictRow, NewAuditRow, OpCommit, OpRecord};
use crate::ws::{self, Reaction};
use crate::AppState;

/// `conflicts.status` of an entry nobody has ruled on.
pub const STATUS_PENDING: &str = "待裁决";

/// What the server knows of one drawing, and the ops it is holding.
#[derive(Debug, Default)]
pub struct DrawingOps {
    pub model: DrawingModel,
    /// Ops made against the current revision, inside the tie window, in the
    /// order they arrived.
    pub held: Vec<HeldOp>,
}

/// An op waiting out the tie window.
#[derive(Debug, Clone)]
pub struct HeldOp {
    pub op_id: String,
    pub session: Session,
    pub peer_addr: String,
    pub base_revision: i64,
    pub changes: Vec<Change>,
    /// Unix seconds it arrived at; the audit rows it writes carry this.
    pub received_at: i64,
    pub due: Instant,
    /// Sent again while the op of its id that came first was held: it
    /// waits behind that one, ties with nothing, and is answered with what
    /// came of it (`OFF-4`).
    pub repeat: bool,
}

/// What handling one op came to.
#[derive(Debug, Default)]
pub struct Outcome {
    pub reactions: Vec<Reaction>,
    /// The revision the drawing moved to, when it moved.
    pub revision: Option<i64>,
}

impl DrawingOps {
    /// The drawing as the op log has it: every applied change replayed in
    /// order, then the revision the `drawings` row says.
    ///
    /// The tags clients listed as in their own copies ([`report_tags`]) go
    /// in between the ops, each where it was heard: behind the op that took
    /// the drawing to the revision it was heard at, and ahead of the next —
    /// which may be the one that takes it away again.
    fn load(state: &AppState, drawing_id: &str) -> Self {
        let mut model = DrawingModel::default();
        if let Some(store) = &state.store {
            let reported = store.reported_tags(drawing_id).unwrap_or_else(|err| {
                eprintln!("[ocs_collab_server] 读取 {drawing_id} 上报的位号失败：{err}");
                Vec::new()
            });
            let mut reported = reported.into_iter().peekable();
            match store.load_ops(drawing_id, 0) {
                Ok(ops) => {
                    for op in ops {
                        while let Some((tag, at)) = reported.next_if(|(_, at)| *at < op.revision) {
                            model.report_tags([tag], at);
                        }
                        // A pairing is the server's to say: one a client
                        // sent was passed on as it came and is not one.
                        let servers = op.session_id.starts_with(LINK_SESSION_PREFIX);
                        for entry in &op.changes {
                            for change in Change::list(entry) {
                                if change.kind == KIND_PAIR_CONNECTOR && !servers {
                                    continue;
                                }
                                let name = change.object_ref.clone();
                                model.commit(&change, &name, op.revision, &op.session_id, &op.user_id);
                            }
                        }
                    }
                }
                Err(err) => eprintln!("[ocs_collab_server] 读取 {drawing_id} 的操作日志失败：{err}"),
            }
            for (tag, at) in reported {
                model.report_tags([tag], at);
            }
        }
        model.set_revision(ws::current_revision(state, drawing_id));
        Self {
            model,
            held: Vec::new(),
        }
    }

    /// A held op of another session that contends with `change` for the same
    /// thing with something else, and the change of it that does.
    fn held_against(&self, change: &Change, session_id: &str) -> Option<(&HeldOp, &Change)> {
        self.held
            .iter()
            .filter(|held| !held.repeat && held.session.session_id != session_id)
            .find_map(|held| {
                held.changes
                    .iter()
                    .find(|theirs| self.contend(theirs, change))
                    .map(|theirs| (held, theirs))
            })
    }

    /// Step 3's condition: both changes stake a claim to the same field of
    /// the same object — or to the connections of the same pipe line — with
    /// different values.
    fn contend(&self, a: &Change, b: &Change) -> bool {
        match (self.model.stake(a), self.model.stake(b)) {
            (Some(a), Some(b)) => a.0 == b.0 && a.1 == b.1 && a.2 != b.2,
            _ => false,
        }
    }
}

/// An op as the judgment sees it, whether it just arrived or was held.
struct Incoming<'a> {
    session: &'a Session,
    peer_addr: &'a str,
    op_id: &'a str,
    base_revision: i64,
    now: i64,
}

/// Whether op `op_id` from `session` is one its drawing has had before, and
/// if so the answer to it (`OFF-4`). Asked before anything else is asked of
/// an op: one taken before was allowed when it came, and a refusal now
/// would have its sender take back a step the room has. [`receive`] asks
/// again under the lock the judgment runs under, for two sendings of one
/// op that cross.
pub fn heard_before(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    base_revision: i64,
    op_id: &str,
    payload: &Value,
    now: i64,
) -> Option<Outcome> {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(session.drawing_id.clone())
        .or_insert_with(|| DrawingOps::load(state, &session.drawing_id));
    let incoming = Incoming {
        session,
        peer_addr,
        op_id,
        base_revision,
        now,
    };
    heard(state, drawing, &incoming, payload)
}

/// [`heard_before`] under the lock. An op that took effect, or waits in a
/// conflict entry, is answered with what came of it. One whose first
/// sending is still held in the tie window waits behind it, ties with
/// nothing, and is answered when that one is through ([`flush_due`]).
/// `None` for an op the drawing has not had, or refused: it is judged like
/// any other.
fn heard(
    state: &AppState,
    drawing: &mut DrawingOps,
    incoming: &Incoming<'_>,
    payload: &Value,
) -> Option<Outcome> {
    if let Some(said) = taken_before(state, incoming.session, incoming.op_id) {
        return Some(said_again(incoming.session, incoming.op_id, said));
    }
    let due = drawing
        .held
        .iter()
        .find(|held| held.op_id == incoming.op_id)?
        .due;
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={} -> 重复提交：先到的那次还在并列窗口内，排在它后面等它的结果 at={}",
        incoming.session.session_id, incoming.session.user_id, incoming.op_id, incoming.now
    );
    drawing.held.push(HeldOp {
        op_id: incoming.op_id.to_owned(),
        session: incoming.session.clone(),
        peer_addr: incoming.peer_addr.to_owned(),
        base_revision: incoming.base_revision,
        changes: Change::list(payload),
        received_at: incoming.now,
        due,
        repeat: true,
    });
    Some(Outcome::default())
}

/// What came of op `op_id` on `session`'s drawing the first time: the
/// `conflict` of every entry that holds it and waits for a ruling, then the
/// `op_applied` its row of the op log says — the revision and the changes
/// it went out with. `None` when neither has it. Without a database there
/// is no log to ask.
fn taken_before(state: &AppState, session: &Session, op_id: &str) -> Option<Vec<ServerMessage>> {
    let store = state.store.as_ref()?;
    let drawing_id = &session.drawing_id;
    let entries = store
        .pending_conflicts_holding(drawing_id, op_id)
        .unwrap_or_else(|err| {
            eprintln!("[ocs_collab_server] 按编号查 {drawing_id} 的冲突条目失败：{err}");
            Vec::new()
        });
    let logged = store.logged_op(drawing_id, op_id).unwrap_or_else(|err| {
        eprintln!("[ocs_collab_server] 按编号查 {drawing_id} 的操作日志失败：{err}");
        None
    });
    let mut said: Vec<ServerMessage> = entries
        .into_iter()
        .map(|(row, op_ids)| ServerMessage::Conflict {
            conflict_id: row.conflict_id,
            object_ref: row.object_ref,
            field: row.field,
            value_a: row.value_a,
            value_b: row.value_b,
            op_ids,
        })
        .collect();
    said.extend(logged.map(|op| ServerMessage::OpApplied {
        op_id: op.op_id,
        revision: op.revision,
        changes: op.changes,
    }));
    (!said.is_empty()).then_some(said)
}

/// The answer to op `op_id` sent again by `session`: what came of it, said
/// to that session alone — not judged, not logged, not told to the room. It
/// goes by the session's own channel, which says it after what the room
/// was told before ([`ws::earlier_in_the_room`]).
fn said_again(session: &Session, op_id: &str, said: Vec<ServerMessage>) -> Outcome {
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={op_id} -> 重复提交：不裁决、不入日志、不广播，只回发送者当初的结果（{} 条）",
        session.session_id,
        session.user_id,
        said.len()
    );
    Outcome {
        reactions: said
            .into_iter()
            .map(|message| Reaction::SendTo {
                session_id: session.session_id.clone(),
                message,
            })
            .collect(),
        revision: None,
    }
}

/// Take one op from `session`.
///
/// Synchronous on purpose: the judgment, the commit and the model update run
/// under one lock with no await in between, so two ops on one drawing cannot
/// interleave. The caller moves the session registry afterwards.
pub fn receive(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    base_revision: i64,
    op_id: String,
    payload: Value,
    now: i64,
) -> Outcome {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(session.drawing_id.clone())
        .or_insert_with(|| DrawingOps::load(state, &session.drawing_id));
    let current = drawing.model.revision();
    let incoming = Incoming {
        session,
        peer_addr,
        op_id: &op_id,
        base_revision,
        now,
    };

    if let Some(outcome) = heard(state, drawing, &incoming, &payload) {
        return outcome;
    }
    if base_revision > current {
        return stale(&incoming, current);
    }
    let changes = Change::list(&payload);
    if changes.is_empty() || !changes.iter().all(Change::is_understood) {
        // Not something the judgment has a rule for.
        if base_revision != current {
            return stale(&incoming, current);
        }
        return commit_opaque(state, drawing, &incoming, &payload);
    }

    if base_revision == current {
        // Step 3: another session's op made against this same revision is
        // still inside the window and stakes a different claim to something
        // this one stakes a claim to.
        let tied = drawing.held.iter().position(|held| {
            !held.repeat
                && held.session.session_id != session.session_id
                && held.base_revision == base_revision
                && held
                    .changes
                    .iter()
                    .any(|theirs| changes.iter().any(|ours| drawing.contend(theirs, ours)))
        });
        if let Some(index) = tied {
            let first = drawing.held.remove(index);
            return tie(state, drawing, first, &incoming, changes);
        }
        // Only what could tie waits: a deletion or a new symbol contends
        // for nothing and commits at once.
        let could_tie = changes
            .iter()
            .any(|change| drawing.model.stake(change).is_some());
        if could_tie && !state.tie_window.is_zero() {
            println!(
                "[ocs_collab_server] 入站 op session={} user={} op_id={op_id} base_revision={base_revision} -> 并列窗口内暂存 {} 毫秒 at={now}",
                session.session_id,
                session.user_id,
                state.tie_window.as_millis()
            );
            drawing.held.push(HeldOp {
                op_id,
                session: session.clone(),
                peer_addr: peer_addr.to_owned(),
                base_revision,
                changes,
                received_at: now,
                due: Instant::now() + state.tie_window,
                repeat: false,
            });
            return Outcome::default();
        }
    }
    let mut outcome = judge_and_commit(state, drawing, &incoming, changes);
    follow_connectors(state, &mut drawings, session, peer_addr, now, &mut outcome);
    outcome
}

/// The drawings as [`ocs_collab::link::follow`] reaches them: the models
/// the server holds (loading one from its op log when it is first asked
/// for), and commits of the server's own made the way a session's are —
/// one revision, the audit rows, the op-log row, the room told, all under
/// the lock the caller holds.
struct LinkedRooms<'a> {
    state: &'a AppState,
    drawings: &'a mut HashMap<String, DrawingOps>,
    /// The session whose op all this follows from: the ops made here are on
    /// its user's account.
    by: &'a Session,
    peer_addr: &'a str,
    now: i64,
    /// Every `op_applied` told to a room on the way.
    told: Vec<Reaction>,
}

impl Rooms for LinkedRooms<'_> {
    fn drawings(&mut self) -> Vec<String> {
        let mut ids: Vec<String> = self.drawings.keys().cloned().collect();
        if let Some(store) = &self.state.store {
            match store.drawing_ids() {
                Ok(known) => ids.extend(known),
                Err(err) => eprintln!("[ocs_collab_server] 读取图纸清单失败：{err}"),
            }
        }
        ids.sort();
        ids.dedup();
        ids
    }

    fn model(&mut self, drawing_id: &str) -> &DrawingModel {
        let state = self.state;
        &self
            .drawings
            .entry(drawing_id.to_owned())
            .or_insert_with(|| DrawingOps::load(state, drawing_id))
            .model
    }

    fn commit(
        &mut self,
        drawing_id: &str,
        op_id: &str,
        cause: &str,
        changes: Vec<Value>,
    ) -> Option<i64> {
        let state = self.state;
        let drawing = self
            .drawings
            .entry(drawing_id.to_owned())
            .or_insert_with(|| DrawingOps::load(state, drawing_id));
        // Nobody's session: the consequence, on this drawing, of an op the
        // drawing `cause` took. It is on that op's user's account.
        let session = Session {
            session_id: format!("{LINK_SESSION_PREFIX}{cause}"),
            drawing_id: drawing_id.to_owned(),
            ..self.by.clone()
        };
        let incoming = Incoming {
            session: &session,
            peer_addr: self.peer_addr,
            op_id,
            base_revision: drawing.model.revision(),
            now: self.now,
        };
        let listed: Vec<Change> = changes
            .iter()
            .filter_map(|raw| Change::list(raw).into_iter().next())
            .collect();
        let rows = match &state.store {
            Some(store) => listed
                .iter()
                .map(|change| {
                    let name = if change.object_ref.is_empty() {
                        change.text("line_tag").unwrap_or_default()
                    } else {
                        change.object_ref.clone()
                    };
                    let text = format!(
                        "{}（沿连接符，起因在 {cause}）",
                        ws::describe_change(&change.kind, &name, &change.raw)
                    );
                    ws::audit_row(
                        Some(store),
                        &session,
                        &change.kind,
                        &name,
                        text,
                        self.peer_addr,
                        self.now,
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        match commit(state, drawing, &incoming, rows, changes) {
            Ok((revision, message)) => {
                for change in &listed {
                    let name = change.object_ref.clone();
                    drawing
                        .model
                        .commit(change, &name, revision, &session.session_id, &session.user_id);
                }
                println!(
                    "[ocs_collab_server] 沿连接符：{op_id} 在 {drawing_id} 提交为修订 {revision}（起因在 {cause}，{} 条变更）",
                    listed.len()
                );
                self.told.push(Reaction::Published {
                    drawing_id: drawing_id.to_owned(),
                    message,
                });
                Some(revision)
            }
            Err(_) => {
                eprintln!("[ocs_collab_server] 沿连接符：{op_id} 在 {drawing_id} 没有提交成");
                None
            }
        }
    }
}

/// After an op of `by`'s took effect (`outcome` says at which revision and
/// what the room was told): pair the connectors it made with their other
/// ends, and carry what it wrote on a line that flows out to the line it
/// flows in on (`VAL-CROSS-009`). What that committed on other drawings —
/// and on this one — is added to `outcome.reactions` as told to their
/// rooms, which is also what moves those drawings' sessions on.
fn follow_connectors(
    state: &AppState,
    drawings: &mut HashMap<String, DrawingOps>,
    by: &Session,
    peer_addr: &str,
    now: i64,
    outcome: &mut Outcome,
) {
    let Some(revision) = outcome.revision else {
        return;
    };
    let told: Vec<Value> = outcome
        .reactions
        .iter()
        .find_map(|reaction| match reaction {
            Reaction::Published {
                message:
                    ServerMessage::OpApplied {
                        revision: at,
                        changes,
                        ..
                    },
                ..
            } if *at == revision => Some(changes.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if told.is_empty() {
        return;
    }
    let mut rooms = LinkedRooms {
        state,
        drawings,
        by,
        peer_addr,
        now,
        told: Vec::new(),
    };
    ocs_collab::link::follow(&mut rooms, &by.drawing_id, revision, &told);
    outcome.reactions.append(&mut rooms.told);
}

/// Commit every held op whose window has passed. Returns, per drawing that
/// moved, what to publish and the revision it reached.
pub fn flush_due(state: &AppState, now: Instant) -> Vec<(String, Outcome)> {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let mut outcomes = Vec::new();
    // Whose op each outcome is of, for what follows from it on other
    // drawings — looked at once every drawing's held ops are through.
    let mut made_by: Vec<(Session, String, i64)> = Vec::new();
    for (drawing_id, drawing) in drawings.iter_mut() {
        while let Some(index) = drawing.held.iter().position(|held| held.due <= now) {
            let held = drawing.held.remove(index);
            // Sent again behind its first sending, which is through by now:
            // answered with what came of it. Judged only when nothing did —
            // the first was refused.
            if held.repeat {
                if let Some(said) = taken_before(state, &held.session, &held.op_id) {
                    outcomes.push((
                        drawing_id.clone(),
                        said_again(&held.session, &held.op_id, said),
                    ));
                    made_by.push((
                        held.session.clone(),
                        held.peer_addr.clone(),
                        held.received_at,
                    ));
                    continue;
                }
            }
            let incoming = Incoming {
                session: &held.session,
                peer_addr: &held.peer_addr,
                op_id: &held.op_id,
                base_revision: held.base_revision,
                now: held.received_at,
            };
            let mut outcome = judge_and_commit(state, drawing, &incoming, held.changes.clone());
            // Nobody is on the other end of a `Send` here; address the
            // session instead.
            for reaction in &mut outcome.reactions {
                if let Reaction::Send(message) = reaction {
                    *reaction = Reaction::SendTo {
                        session_id: held.session.session_id.clone(),
                        message: message.clone(),
                    };
                }
            }
            outcomes.push((drawing_id.clone(), outcome));
            made_by.push((held.session.clone(), held.peer_addr.clone(), held.received_at));
        }
    }
    for ((_, outcome), (session, peer_addr, received_at)) in outcomes.iter_mut().zip(&made_by) {
        follow_connectors(state, &mut drawings, session, peer_addr, *received_at, outcome);
    }
    outcomes
}

/// The revision each drawing was taken to by the `op_applied`s among
/// `reactions`: the op's own drawing, and the drawings a connector carried
/// it on to. The sessions of each are at that revision from then on.
pub fn revisions_reached(reactions: &[Reaction]) -> Vec<(String, i64)> {
    reactions
        .iter()
        .filter_map(|reaction| match reaction {
            Reaction::Published {
                drawing_id,
                message: ServerMessage::OpApplied { revision, .. },
            } => Some((drawing_id.clone(), *revision)),
            _ => None,
        })
        .collect()
}

/// [`flush_due`], with its results delivered: publishes go to the room,
/// errors to the session they are for, and the registry follows the revision.
pub async fn flush_and_deliver(state: &AppState) {
    for (drawing_id, outcome) in flush_due(state, Instant::now()) {
        if let Some(revision) = outcome.revision {
            state
                .sessions
                .lock()
                .await
                .advance_base_revision(&drawing_id, revision);
        }
        // And the drawings a connector carried it on to.
        for (drawing_id, revision) in revisions_reached(&outcome.reactions) {
            state
                .sessions
                .lock()
                .await
                .advance_base_revision(&drawing_id, revision);
        }
        for reaction in outcome.reactions {
            match reaction {
                Reaction::Publish {
                    drawing_id,
                    message,
                } => {
                    state.bus.publish(&drawing_id, &message);
                }
                Reaction::SendTo {
                    session_id,
                    message,
                } => {
                    let sender = state
                        .direct
                        .lock()
                        .expect("direct mutex poisoned")
                        .get(&session_id)
                        .cloned();
                    if let Some(sender) = sender {
                        let _ = sender.send(message);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The drawing's objects as the server knows them, for the semantic read-back
/// (`GET /drawing/{id}/objects`): `(revision, items)`, one item per object —
/// or only `object_ref`'s, when one is named.
pub fn objects(
    state: &AppState,
    drawing_id: &str,
    object_ref: Option<&str>,
) -> (i64, Vec<Value>) {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(drawing_id.to_owned())
        .or_insert_with(|| DrawingOps::load(state, drawing_id));
    let items = drawing
        .model
        .objects()
        .filter(|(tag, _)| object_ref.is_none_or(|wanted| wanted == tag.as_str()))
        .map(|(tag, object)| {
            let fields: serde_json::Map<String, Value> = object
                .fields
                .iter()
                .map(|(field, state)| {
                    (
                        field.clone(),
                        serde_json::json!({
                            "value": state.value,
                            "revision": state.revision,
                            "user_id": state.user_id,
                        }),
                    )
                })
                .collect();
            serde_json::json!({ "object_ref": tag, "fields": fields })
        })
        .collect();
    (drawing.model.revision(), items)
}

/// Registers the tags a client says the objects of its own copy of
/// `drawing_id` carry (`hello.existing_tags`): no op told of them, and the
/// server gives none of them to a symbol somebody else places. `drawings`
/// is the caller's lock on the drawings; `seen` is the revision the
/// client's copy is at, `current` the one the drawing is at.
///
/// The tags that are news are kept in the store with `current` — the
/// point of the op log they came in at — so that a server that starts
/// again has them ([`DrawingOps::load`]). Returns how many were news.
pub fn report_tags(
    state: &AppState,
    drawings: &mut HashMap<String, DrawingOps>,
    drawing_id: &str,
    tags: &[String],
    seen: i64,
    current: i64,
) -> usize {
    // Nothing listed: the drawing is not read for it.
    if tags.is_empty() {
        return 0;
    }
    let drawing = drawings
        .entry(drawing_id.to_owned())
        .or_insert_with(|| DrawingOps::load(state, drawing_id));
    let news = drawing.model.report_tags(tags, seen);
    if news.is_empty() {
        return 0;
    }
    if let Some(store) = &state.store {
        if let Err(err) = store.report_tags(drawing_id, &news, current) {
            eprintln!("[ocs_collab_server] 记录 {drawing_id} 上报的位号失败：{err}");
        }
    }
    news.len()
}

/// The tags in use in some client's own copy of the drawing that no op
/// told of ([`report_tags`]), in order — or `object_ref` alone, when one is
/// named and is one of them. With the objects [`objects`] lists, these are
/// the tags a new symbol is not given.
pub fn reported_tags(state: &AppState, drawing_id: &str, object_ref: Option<&str>) -> Vec<String> {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(drawing_id.to_owned())
        .or_insert_with(|| DrawingOps::load(state, drawing_id));
    drawing
        .model
        .reported_tags()
        .filter(|tag| object_ref.is_none_or(|wanted| wanted == tag.as_str()))
        .cloned()
        .collect()
}

/// The three rulings (`architecture.md` §2.5).
pub const RESOLUTION_A: &str = "采用甲方";
pub const RESOLUTION_B: &str = "采用乙方";
pub const RESOLUTION_MANUAL: &str = "人工改值";
/// `conflicts.status` once ruled on.
pub const STATUS_RESOLVED: &str = "已裁决";

/// A ruling that was carried out.
#[derive(Debug)]
pub struct Resolved {
    /// The entry as it stands now.
    pub row: ConflictRow,
    /// What the field (or the line's connections) was set to.
    pub final_value: String,
    /// The revision the drawing moved to when the ruling was written back.
    pub revision: i64,
    /// The `op_applied` that told the room (it has gone out already).
    pub message: ServerMessage,
}

/// Why a ruling was not carried out.
#[derive(Debug)]
pub enum ResolveError {
    /// No entry has that id.
    Unknown,
    /// It was ruled on before; the entry is returned as it stands.
    AlreadyResolved(Box<ConflictRow>),
    /// The request does not say a ruling the entry can take.
    Invalid(String),
    /// The database refused.
    Store(String),
}

/// Carry out a ruling on a pending conflict entry.
///
/// A ruling is more than a mark on the entry: the value it picks is written
/// to the drawing as an op of the server's own — the revision moves and
/// `op_applied` goes out — so every client on the drawing has it at once and
/// one that is away gets it when it catches up. The op, its audit row and
/// the op-log row are one transaction; the entry is marked after it.
///
/// For a field the value is `value_a`, `value_b` or the one given; for a
/// pipe line's connections `value_a` and `value_b` are the two changes that
/// tied, and the one picked is applied (a value given by hand has to be such
/// a change too).
pub fn resolve(
    state: &AppState,
    conflict_id: &str,
    resolution: &str,
    custom: Option<&str>,
    resolved_by: &str,
    now: i64,
) -> Result<Resolved, ResolveError> {
    let Some(store) = &state.store else {
        return Err(ResolveError::Store("数据库不可用".to_owned()));
    };
    let row = store
        .conflict(conflict_id)
        .map_err(ResolveError::Store)?
        .ok_or(ResolveError::Unknown)?;
    if row.status != STATUS_PENDING {
        return Err(ResolveError::AlreadyResolved(Box::new(row)));
    }
    let final_value = match resolution {
        RESOLUTION_A => row.value_a.clone(),
        RESOLUTION_B => row.value_b.clone(),
        RESOLUTION_MANUAL => custom
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ResolveError::Invalid("人工改值需要给出 value".to_owned()))?,
        other => {
            return Err(ResolveError::Invalid(format!(
                "resolution 只能是 {RESOLUTION_A} / {RESOLUTION_B} / {RESOLUTION_MANUAL}，收到 {other}"
            )))
        }
    };

    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(row.drawing_id.clone())
        .or_insert_with(|| DrawingOps::load(state, &row.drawing_id));
    let payload = if row.field == ocs_collab::conflict::FIELD_TOPOLOGY {
        serde_json::from_str::<Value>(&final_value)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                ResolveError::Invalid("拓扑冲突的裁决值必须是一个拓扑操作（JSON 对象）".to_owned())
            })?
    } else {
        let mut change = serde_json::json!({
            "kind": if row.field == ocs_collab::conflict::FIELD_TAG { KIND_SET_TAG } else { "set_property" },
            "object_ref": row.object_ref,
            "field": row.field,
            "value": final_value,
        });
        if let Some(current) = drawing.model.field(&row.object_ref, &row.field) {
            change["old_value"] = Value::String(current.value.clone());
        }
        change
    };
    let changes: Vec<Change> = Change::list(&payload)
        .iter()
        .map(|change| drawing.model.completed(change))
        .collect();
    if changes.is_empty() || !changes.iter().all(Change::is_understood) {
        return Err(ResolveError::Invalid(
            "裁决值不是服务器认识的操作".to_owned(),
        ));
    }

    // The ruling is made knowing the drawing as it is: nothing to be behind.
    let expected = drawing.model.revision();
    let session_id = format!("adjudication:{conflict_id}");
    let applied: Vec<(Change, String)> = changes
        .into_iter()
        .map(|change| {
            let name = match drawing.model.judge(&change, expected, &session_id) {
                Verdict::Apply { object_ref, .. } => object_ref,
                _ => change.subject(),
            };
            (change, name)
        })
        .collect();
    let broadcast: Vec<Value> = applied
        .iter()
        .map(|(change, name)| change.broadcast_form(name))
        .collect();
    let company = store
        .user_role(resolved_by)
        .ok()
        .flatten()
        .map(|(_, company)| company)
        .filter(|company| !company.is_empty())
        .unwrap_or_else(|| "未知".to_owned());
    let audit = NewAuditRow {
        module: "冲突裁决".to_owned(),
        action: "conflict_resolved".to_owned(),
        at: now,
        object_ref: row.object_ref.clone(),
        user_id: resolved_by.to_owned(),
        company,
        ip: "127.0.0.1".to_owned(),
        workspace: "默认工作区".to_owned(),
        change_text: format!(
            "裁决 {conflict_id}：{}.{} 甲方 {}，乙方 {}，{resolution}，最终 {final_value}",
            row.object_ref, row.field, row.value_a, row.value_b
        ),
    };
    let op_id = format!("resolve-{conflict_id}");
    let record = OpRecord {
        op_id: op_id.clone(),
        session_id: session_id.clone(),
        user_id: resolved_by.to_owned(),
        base_revision: expected,
        at: now,
        changes: Value::Array(broadcast.clone()).to_string(),
    };
    let revision = match store.apply_op(&row.drawing_id, expected, &[audit], Some(&record)) {
        Ok(OpCommit::Applied { revision, .. }) => revision,
        Ok(OpCommit::Stale { current }) => {
            drawing.model.set_revision(current);
            return Err(ResolveError::Store(format!(
                "图面修订已到 {current}，请重试"
            )));
        }
        Err(err) => return Err(ResolveError::Store(err)),
    };
    for (change, name) in &applied {
        drawing
            .model
            .commit(change, name, revision, &session_id, resolved_by);
    }
    drawing.model.set_revision(revision);

    let custom_value = (resolution == RESOLUTION_MANUAL).then_some(final_value.as_str());
    match store.resolve_conflict(conflict_id, resolution, custom_value, resolved_by) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("[ocs_collab_server] 冲突 {conflict_id} 写回之后标记失败：条目已不是待裁决")
        }
        Err(err) => return Err(ResolveError::Store(err)),
    }
    println!(
        "[ocs_collab_server] 裁决 conflict={conflict_id} by={resolved_by} resolution={resolution} object_ref={} field={} -> 广播 op_applied revision={revision} at={now}",
        row.object_ref, row.field
    );
    let row = ConflictRow {
        status: STATUS_RESOLVED.to_owned(),
        resolution: Some(resolution.to_owned()),
        resolved_value: custom_value.map(str::to_owned),
        resolved_by: Some(resolved_by.to_owned()),
        ..row
    };
    let message = ServerMessage::OpApplied {
        op_id,
        revision,
        changes: broadcast,
    };
    // A ruling is an op like any other, and is told to the room in its turn.
    announce(state, &row.drawing_id, &message);
    Ok(Resolved {
        row,
        final_value,
        revision,
        message,
    })
}

/// The drawing's pipe topology as the server knows it, for
/// `GET /drawing/{id}/topology`: `(revision, {pipe_lines, pipe_runs,
/// connectors})`. Ops still inside the tie window, and ops a conflict entry
/// is holding back, are not in it: it is what has taken effect.
pub fn topology(state: &AppState, drawing_id: &str) -> (i64, Value) {
    let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
    let drawing = drawings
        .entry(drawing_id.to_owned())
        .or_insert_with(|| DrawingOps::load(state, drawing_id));
    (drawing.model.revision(), drawing.model.topology())
}

/// How often the flush loop looks, for a given window: often enough that an
/// op is not held noticeably longer than asked.
pub fn flush_interval(window: Duration) -> Duration {
    (window / 4).clamp(Duration::from_millis(5), Duration::from_millis(50))
}

fn stale(incoming: &Incoming<'_>, current: i64) -> Outcome {
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} -> error {} current={current} at={}",
        incoming.session.session_id,
        incoming.session.user_id,
        incoming.op_id,
        incoming.base_revision,
        codes::STALE_BASE_REVISION,
        incoming.now
    );
    Outcome {
        reactions: vec![Reaction::Send(answer(
            incoming,
            codes::STALE_BASE_REVISION,
            format!(
                "图面当前修订为 {current}，操作基于 {}",
                incoming.base_revision
            ),
            true,
        ))],
        revision: None,
    }
}

/// The `error` that answers an op: which op it is about, and — `rejected`
/// — whether nothing of that op took effect, so that its sender can take
/// the step back (`VAL-CROSS-008`). An error about one change of an op
/// whose other changes may still take effect is not `rejected`; the caller
/// that finds nothing took effect after all says so with [`reject_whole`].
fn answer(incoming: &Incoming<'_>, code: &str, message: String, rejected: bool) -> ServerMessage {
    ServerMessage::Error {
        code: code.to_owned(),
        message,
        level: None,
        holder: None,
        op_id: Some(incoming.op_id.to_owned()),
        rejected: rejected.then_some(true),
    }
}

/// Every error among `reactions` now says that nothing of its op took
/// effect.
fn reject_whole(reactions: &mut [Reaction]) {
    for reaction in reactions {
        if let Reaction::Send(ServerMessage::Error { rejected, .. })
        | Reaction::SendTo {
            message: ServerMessage::Error { rejected, .. },
            ..
        } = reaction
        {
            *rejected = Some(true);
        }
    }
}

/// An op the judgment does not read: taken whole at the current revision, as
/// before, and logged for replay.
fn commit_opaque(
    state: &AppState,
    drawing: &mut DrawingOps,
    incoming: &Incoming<'_>,
    payload: &Value,
) -> Outcome {
    let changes = ws::op_changes(payload);
    let rows = match &state.store {
        Some(store) => vec![ws::op_audit_row(
            store,
            incoming.session,
            payload,
            incoming.op_id,
            incoming.peer_addr,
            incoming.now,
        )],
        None => Vec::new(),
    };
    match commit(state, drawing, incoming, rows, changes) {
        Ok((revision, message)) => {
            log_applied(incoming, payload, revision);
            Outcome {
                reactions: vec![Reaction::Published {
                    drawing_id: incoming.session.drawing_id.clone(),
                    message,
                }],
                revision: Some(revision),
            }
        }
        Err(outcome) => outcome,
    }
}

/// Steps 1 and 2 for every change of a field-writing op, then the commit of
/// the ones that take effect.
fn judge_and_commit(
    state: &AppState,
    drawing: &mut DrawingOps,
    incoming: &Incoming<'_>,
    changes: Vec<Change>,
) -> Outcome {
    let session = incoming.session;
    let mut reactions = Vec::new();
    let mut applied: Vec<(Change, String, Option<String>)> = Vec::new();
    let mut behind_topology = None;
    // An op that makes, erases or reconnects something is one step of
    // somebody's drawing and is taken whole or not at all: half of it — the
    // field written, the fitting refused — is a drawing nobody made, and
    // nothing its sender could take back. Field writes alone keep being
    // judged one by one (`architecture.md` §3.3, step 1).
    let structural = changes.iter().any(|change| !change.writes_a_field());
    for change in changes {
        let change = drawing.model.completed(&change);
        // A held op was made against a newer revision than this one: what
        // it stakes its claim with is what this change is behind.
        if incoming.base_revision < drawing.model.revision() {
            if let Some((held, theirs)) = drawing.held_against(&change, &session.session_id) {
                if change.changes_topology() {
                    behind_topology = Some(drawing.model.revision());
                    continue;
                }
                let object_ref = drawing
                    .model
                    .current_name(&change.object_ref, incoming.base_revision);
                let winner = (
                    theirs.value.clone(),
                    drawing.model.revision() + 1,
                    held.session.user_id.clone(),
                );
                reactions.push(yielded(state, incoming, &change, &object_ref, winner));
                continue;
            }
        }
        match drawing
            .model
            .judge(&change, incoming.base_revision, &session.session_id)
        {
            Verdict::Apply {
                object_ref,
                replaces,
            } => applied.push((change, object_ref, replaces)),
            Verdict::Yield {
                object_ref,
                winner,
                winner_revision,
                winner_user,
            } => reactions.push(yielded(
                state,
                incoming,
                &change,
                &object_ref,
                (winner, winner_revision, winner_user),
            )),
            Verdict::Gone {
                object_ref,
                deleted_revision,
                deleted_by,
                last_value,
            } => reactions.push(refused(
                state,
                incoming,
                codes::OBJECT_DELETED,
                "change_rejected_deleted",
                &object_ref,
                format!(
                    "{} {object_ref}.{}: 提交 {} 未生效，对象已在修订 {deleted_revision} 被 {deleted_by} 删除；删除前的值 {}",
                    change.kind,
                    change.field,
                    change.value,
                    last_value.as_deref().unwrap_or("(未知)")
                ),
            )),
            Verdict::Referenced { object_ref, by } => reactions.push(refused(
                state,
                incoming,
                codes::OBJECT_REFERENCED,
                "delete_refused",
                &object_ref,
                format!(
                    "{} {object_ref}: 被 {} 引用为管路端点，不能删除",
                    change.kind,
                    by.join("、")
                ),
            )),
            Verdict::BehindTopology {
                touched_revision, ..
            } => behind_topology = Some(touched_revision),
            Verdict::DuplicateInflow { line_tag } => reactions.push(refused(
                state,
                incoming,
                codes::DUPLICATE_INFLOW_CONNECTOR,
                "connector_refused",
                &line_tag,
                format!(
                    "{} {line_tag}: 这条线在本图上已有流入连接符，同一线号同图至多一个",
                    change.kind
                ),
            )),
        }
    }
    if let Some(revision) = behind_topology {
        // Connections do not merge: the sender looks at the line as it is
        // now and asks again, like any op the server cannot merge.
        let mut outcome = stale(incoming, revision.max(drawing.model.revision()));
        reactions.append(&mut outcome.reactions);
    }
    // Everything in `reactions` so far is a change that was turned away.
    if !reactions.is_empty() && (structural || applied.is_empty()) {
        reject_whole(&mut reactions);
        println!(
            "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} -> 整个 op 未生效（{} 条变更被拒{}） at={}",
            session.session_id,
            session.user_id,
            incoming.op_id,
            incoming.base_revision,
            reactions.len(),
            if applied.is_empty() { "" } else { "，带结构的 op 不拆开生效" },
            incoming.now
        );
        return Outcome {
            reactions,
            revision: None,
        };
    }
    if applied.is_empty() {
        return Outcome {
            reactions,
            revision: None,
        };
    }

    let rows = match &state.store {
        Some(store) => applied
            .iter()
            .map(|(change, object_ref, replaces)| {
                let mut described = change.raw.clone();
                if described.get("old_value").is_none() {
                    if let Some(replaces) = replaces {
                        described["old_value"] = Value::String(replaces.clone());
                    }
                }
                let mut text = ws::describe_change(&change.kind, object_ref, &described);
                if change.kind == KIND_DELETE_OBJECT {
                    // A deletion's record says what went with the object.
                    let carried: Vec<String> = drawing
                        .model
                        .object(object_ref)
                        .map(|object| {
                            object
                                .fields
                                .iter()
                                .map(|(field, state)| format!("{field}={}", state.value))
                                .collect()
                        })
                        .unwrap_or_default();
                    if !carried.is_empty() {
                        text = format!("{text}（删除前：{}）", carried.join("；"));
                    }
                }
                ws::audit_row(
                    Some(store),
                    session,
                    &change.kind,
                    object_ref,
                    text,
                    incoming.peer_addr,
                    incoming.now,
                )
            })
            .collect(),
        None => Vec::new(),
    };
    let broadcast: Vec<Value> = applied
        .iter()
        .map(|(change, object_ref, _)| change.broadcast_form(object_ref))
        .collect();
    match commit(state, drawing, incoming, rows, broadcast) {
        Ok((revision, message)) => {
            for (change, object_ref, _) in &applied {
                drawing.model.commit(
                    change,
                    object_ref,
                    revision,
                    &session.session_id,
                    &session.user_id,
                );
            }
            log_applied(incoming, &applied[0].0.raw, revision);
            reactions.push(Reaction::Published {
                drawing_id: session.drawing_id.clone(),
                message,
            });
            Outcome {
                reactions,
                revision: Some(revision),
            }
        }
        Err(mut outcome) => {
            // The commit did not happen: nothing of the op took effect,
            // whatever was said of single changes before.
            reactions.append(&mut outcome.reactions);
            reject_whole(&mut reactions);
            Outcome {
                reactions,
                revision: None,
            }
        }
    }
}

/// The revision bump, the audit rows and the op-log row as one unit, and the
/// `op_applied` to publish for it.
fn commit(
    state: &AppState,
    drawing: &mut DrawingOps,
    incoming: &Incoming<'_>,
    rows: Vec<NewAuditRow>,
    changes: Vec<Value>,
) -> Result<(i64, ServerMessage), Outcome> {
    let session = incoming.session;
    let expected = drawing.model.revision();
    let revision = match &state.store {
        Some(store) => {
            let record = OpRecord {
                op_id: incoming.op_id.to_owned(),
                session_id: session.session_id.clone(),
                user_id: session.user_id.clone(),
                base_revision: incoming.base_revision,
                at: incoming.now,
                changes: Value::Array(changes.clone()).to_string(),
            };
            match store.apply_op(&session.drawing_id, expected, &rows, Some(&record)) {
                Ok(OpCommit::Applied { revision, .. }) => revision,
                Ok(OpCommit::Stale { current }) => {
                    // The row moved without this model (another process on
                    // the same file). Take its word and have the client retry.
                    drawing.model.set_revision(current);
                    return Err(stale(incoming, current));
                }
                Err(err) => {
                    eprintln!("[ocs_collab_server] 提交操作失败：{err}");
                    return Err(Outcome {
                        reactions: vec![Reaction::Send(answer(
                            incoming,
                            codes::INVALID_MESSAGE,
                            "提交操作失败".to_owned(),
                            true,
                        ))],
                        revision: None,
                    });
                }
            }
        }
        None => {
            let next = expected + 1;
            state
                .revisions
                .lock()
                .expect("revisions mutex poisoned")
                .insert(session.drawing_id.clone(), next);
            next
        }
    };
    drawing.model.set_revision(revision);
    let message = ServerMessage::OpApplied {
        op_id: incoming.op_id.to_owned(),
        revision,
        changes,
    };
    announce(state, &session.drawing_id, &message);
    Ok((revision, message))
}

/// Tell the room of an op that has just taken effect.
///
/// Called with the drawing still locked, by the code that moved its
/// revision: the next op cannot commit — and so cannot be announced — before
/// this one has been, and the room hears revisions in the order they were
/// made. A client counts on that (a revision that is not the next one means
/// it missed a change). Announced by each connection after the lock is gone,
/// two ops made a moment apart could go out newest first.
fn announce(state: &AppState, drawing_id: &str, message: &ServerMessage) {
    state.bus.publish(drawing_id, message);
}

fn log_applied(incoming: &Incoming<'_>, first: &Value, revision: i64) {
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} kind={} object_ref={} -> 广播 op_applied revision={revision} at={}",
        incoming.session.session_id,
        incoming.session.user_id,
        incoming.op_id,
        incoming.base_revision,
        first.get("kind").and_then(Value::as_str).unwrap_or("op"),
        first.get("object_ref").and_then(Value::as_str).unwrap_or(""),
        incoming.now
    );
}

/// A change that does not happen for a reason the sender has to hear and the
/// log has to keep: an `error` with `code` back to the sender, and one audit
/// row under `action` with the same words.
fn refused(
    state: &AppState,
    incoming: &Incoming<'_>,
    code: &str,
    action: &str,
    object_ref: &str,
    text: String,
) -> Reaction {
    let session = incoming.session;
    if let Some(store) = &state.store {
        let row = ws::audit_row(
            Some(store),
            session,
            action,
            object_ref,
            text.clone(),
            incoming.peer_addr,
            incoming.now,
        );
        if let Err(err) = store.audit_append(&row) {
            eprintln!("[ocs_collab_server] 写入拒绝审计失败：{err}");
        }
    }
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} object_ref={object_ref} -> error {code} at={}",
        session.session_id, session.user_id, incoming.op_id, incoming.base_revision, incoming.now
    );
    Reaction::Send(answer(
        incoming,
        code,
        format!("操作 {}：{text}", incoming.op_id),
        false,
    ))
}

/// Step 2's bookkeeping: the audit row saying what was not overwritten, and
/// the `error` telling the sender.
fn yielded(
    state: &AppState,
    incoming: &Incoming<'_>,
    change: &Change,
    object_ref: &str,
    (winner, winner_revision, winner_user): (String, i64, String),
) -> Reaction {
    let session = incoming.session;
    let what = format!("{}.{}", object_ref, change.field);
    let text = format!(
        "{} {what}: {} 让位于 {winner}（基线 {} 落后于修订 {winner_revision}，{winner_user} 先写入）",
        change.kind, change.value, incoming.base_revision
    );
    if let Some(store) = &state.store {
        let action = if change.kind == KIND_SET_TAG {
            "tag_superseded"
        } else {
            "property_superseded"
        };
        let row = ws::audit_row(
            Some(store),
            session,
            action,
            object_ref,
            text,
            incoming.peer_addr,
            incoming.now,
        );
        if let Err(err) = store.audit_append(&row) {
            eprintln!("[ocs_collab_server] 写入让位审计失败：{err}");
        }
    }
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} kind={} object_ref={object_ref} field={} -> error {} winner_revision={winner_revision} at={}",
        session.session_id,
        session.user_id,
        incoming.op_id,
        incoming.base_revision,
        change.kind,
        change.field,
        codes::OP_SUPERSEDED,
        incoming.now
    );
    Reaction::Send(answer(
        incoming,
        codes::OP_SUPERSEDED,
        format!(
            "操作 {} 的基线 {} 落后：{what} 已在修订 {winner_revision} 被 {winner_user} 改为 {winner}，本次提交的 {} 未生效",
            incoming.op_id, incoming.base_revision, change.value
        ),
        false,
    ))
}

/// Step 3: `first` was held, `second` just arrived, both against one
/// revision, and they write some field differently. Neither write takes
/// effect; one conflict entry per such field, and the room is told. What else
/// the two ops carry is unaffected and commits now.
fn tie(
    state: &AppState,
    drawing: &mut DrawingOps,
    first: HeldOp,
    second: &Incoming<'_>,
    second_changes: Vec<Change>,
) -> Outcome {
    let drawing_id = second.session.drawing_id.clone();
    let first_incoming = Incoming {
        session: &first.session,
        peer_addr: &first.peer_addr,
        op_id: &first.op_id,
        base_revision: first.base_revision,
        now: first.received_at,
    };
    let mut reactions = Vec::new();
    let (mut first_rest, mut second_rest) = (Vec::new(), second_changes.clone());
    for ours in &first.changes {
        let Some(theirs) = second_changes
            .iter()
            .find(|theirs| drawing.contend(ours, theirs))
        else {
            first_rest.push(ours.clone());
            continue;
        };
        second_rest.retain(|change| change != theirs);
        // What they contend for, and what each would have made of it: a
        // field and two values, or a pipe line's connections and the two
        // changes to them.
        let (Some((object_ref, field, value_a)), Some((_, _, value_b))) =
            (drawing.model.stake(ours), drawing.model.stake(theirs))
        else {
            continue;
        };
        let conflict_id = new_conflict_id();
        if let Some(store) = &state.store {
            let row = ConflictRow {
                conflict_id: conflict_id.clone(),
                drawing_id: drawing_id.clone(),
                object_ref: object_ref.clone(),
                field: field.clone(),
                value_a: value_a.clone(),
                value_b: value_b.clone(),
                base_revision: first.base_revision,
                status: STATUS_PENDING.to_owned(),
                resolution: None,
                resolved_value: None,
                resolved_by: None,
            };
            let held = [first.op_id.clone(), second.op_id.to_owned()];
            if let Err(err) = store.insert_conflict_holding(&row, &held) {
                eprintln!("[ocs_collab_server] 写入冲突条目失败：{err}");
            }
            // Both attempts go on record, each under the person who made it.
            for (who, kind, value, other) in [
                (&first_incoming, &ours.kind, &value_a, &value_b),
                (second, &theirs.kind, &value_b, &value_a),
            ] {
                let text = format!(
                    "{kind} {object_ref}.{field}: 提交 {value}，与 {other} 并列（基线 {}），待裁决 {conflict_id}",
                    first.base_revision
                );
                let row = ws::audit_row(
                    Some(store),
                    who.session,
                    "conflict",
                    &object_ref,
                    text,
                    who.peer_addr,
                    who.now,
                );
                if let Err(err) = store.audit_append(&row) {
                    eprintln!("[ocs_collab_server] 写入冲突审计失败：{err}");
                }
            }
        }
        println!(
            "[ocs_collab_server] 入站 op session={} user={} op_id={} base_revision={} kind={} object_ref={} field={} -> conflict {conflict_id} 与 session={} op_id={} 并列 at={}",
            second.session.session_id,
            second.session.user_id,
            second.op_id,
            second.base_revision,
            theirs.kind,
            object_ref,
            field,
            first.session.session_id,
            first.op_id,
            second.now
        );
        reactions.push(Reaction::Publish {
            drawing_id: drawing_id.clone(),
            message: ServerMessage::Conflict {
                conflict_id,
                object_ref,
                field,
                value_a,
                value_b,
                // Neither op gets any other answer for this change.
                op_ids: vec![first.op_id.clone(), second.op_id.to_owned()],
            },
        });
    }

    let mut revision = None;
    for (who, rest) in [(&first_incoming, first_rest), (second, second_rest)] {
        if rest.is_empty() {
            continue;
        }
        let mut outcome = judge_and_commit(state, drawing, who, rest);
        // `first` has no socket in this call; address its session.
        if std::ptr::eq(who, &first_incoming) {
            for reaction in &mut outcome.reactions {
                if let Reaction::Send(message) = reaction {
                    *reaction = Reaction::SendTo {
                        session_id: first.session.session_id.clone(),
                        message: message.clone(),
                    };
                }
            }
        }
        revision = outcome.revision.or(revision);
        reactions.append(&mut outcome.reactions);
    }
    Outcome {
        reactions,
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AuditQuery, Store};
    use crate::ws::{handle_message, Connection};
    use ocs_collab::{ClientMessage, SessionConfig};
    use serde_json::json;

    const T0: i64 = 1_700_000_000;
    const DRAWING: &str = "PID-1101";

    fn state() -> AppState {
        AppState::new(
            Some(Store::open_in_memory().expect("memory store")),
            None,
            ":memory:",
            SessionConfig::default(),
        )
    }

    /// A server started with `--tie-window-ms`.
    fn windowed() -> AppState {
        let mut state = state();
        state.tie_window = Duration::from_millis(500);
        state
    }

    async fn join(state: &AppState, user_id: &str) -> Connection {
        let mut conn = Connection::new("10.0.0.7");
        let hello = ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: DRAWING.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since: None,
            existing_tags: Vec::new(),
            repo_id: None,
        };
        handle_message(state, &mut conn, hello, T0).await;
        assert!(conn.session.is_some(), "hello must open a session");
        conn
    }

    fn set(field: &str, value: &str) -> Value {
        json!({"kind": "set_property", "object_ref": "P-1001", "field": field, "value": value})
    }

    async fn submit(
        state: &AppState,
        conn: &mut Connection,
        base_revision: i64,
        op_id: &str,
        payload: Value,
    ) -> Vec<Reaction> {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let message = ClientMessage::Op {
            session_id,
            base_revision,
            op_id: op_id.into(),
            payload,
        };
        handle_message(state, conn, message, T0 + 1).await
    }

    /// `(op_id, revision, changes)` of the `op_applied` among `reactions`.
    fn applied(reactions: &[Reaction]) -> Option<(String, i64, Vec<Value>)> {
        reactions.iter().find_map(|reaction| match reaction {
            Reaction::Published {
                message:
                    ServerMessage::OpApplied {
                        op_id,
                        revision,
                        changes,
                    },
                ..
            } => Some((op_id.clone(), *revision, changes.clone())),
            _ => None,
        })
    }

    /// `(code, message)` of every `error` sent back.
    fn errors(reactions: &[Reaction]) -> Vec<(String, String)> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Send(ServerMessage::Error { code, message, .. })
                | Reaction::SendTo {
                    message: ServerMessage::Error { code, message, .. },
                    ..
                } => Some((code.clone(), message.clone())),
                _ => None,
            })
            .collect()
    }

    fn conflicts_published(reactions: &[Reaction]) -> Vec<ServerMessage> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Publish { message, .. }
                    if matches!(message, ServerMessage::Conflict { .. }) =>
                {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn conflict_rows(state: &AppState) -> Vec<ConflictRow> {
        state
            .store
            .as_ref()
            .expect("store")
            .list_conflicts(Some(DRAWING), None)
            .expect("conflicts")
    }

    /// The value the server holds for `field` of P-1001.
    fn value_of(state: &AppState, field: &str) -> Option<String> {
        let (_, items) = objects(state, DRAWING, Some("P-1001"));
        items
            .first()
            .and_then(|item| item["fields"][field]["value"].as_str().map(str::to_owned))
    }

    /// The room hears of an op inside the step that made it, not when the
    /// connection that sent it gets round to its reactions: by the time
    /// `receive` has returned, the `op_applied` is on the room's channel.
    /// Two connections each publishing their own op afterwards could do so
    /// in either order, and the room would hear revision 2 before 1 — which
    /// a client reads, rightly, as a change it never got.
    #[tokio::test]
    async fn an_applied_op_is_announced_before_the_drawing_is_unlocked() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let mut room = state.bus.subscribe(DRAWING);

        // Neither call's reactions are applied — nothing below publishes.
        let first = submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        let second = submit(&state, &mut s2, 1, "op-2", set("公称直径", "DN50")).await;

        let heard: Vec<(String, i64)> = std::iter::from_fn(|| room.try_recv())
            .filter_map(|message| match message {
                ServerMessage::OpApplied { op_id, revision, .. } => Some((op_id, revision)),
                _ => None,
            })
            .collect();
        assert_eq!(
            heard,
            [("op-1".to_owned(), 1), ("op-2".to_owned(), 2)],
            "the room hears each op as it commits, in revision order"
        );
        // And the caller is not asked to publish them a second time.
        for reactions in [&first, &second] {
            assert!(
                !reactions.iter().any(|reaction| matches!(
                    reaction,
                    Reaction::Publish {
                        message: ServerMessage::OpApplied { .. },
                        ..
                    }
                )),
                "{reactions:?}"
            );
        }
    }

    /// A session that joins is told a revision and from then on hears every
    /// op after it. The two are one step: the subscription exists by the
    /// time `hello` has read the revision, before any of its reactions is
    /// applied — an op that commits right then is not lost in between.
    #[tokio::test]
    async fn a_session_that_joins_hears_the_op_right_after_the_revision_it_was_told() {
        let state = state();
        let mut s1 = join(&state, "u1").await;

        let mut late = Connection::new("10.0.0.8");
        let hello = ClientMessage::Hello {
            user_id: "u2".into(),
            drawing_id: DRAWING.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since: None,
            existing_tags: Vec::new(),
            repo_id: None,
        };
        let welcome = handle_message(&state, &mut late, hello, T0).await;
        let told = welcome
            .iter()
            .find_map(|reaction| match reaction {
                Reaction::Send(ServerMessage::Welcome { base_revision, .. }) => Some(*base_revision),
                _ => None,
            })
            .expect("welcome");
        assert_eq!(told, 0);

        // Before a single reaction of that `hello` has been applied — which
        // is when the subscription used to be taken out — an op commits.
        let _ = submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        let mut joined = late.joined.take().expect("hello joined the room itself");
        let next = std::iter::from_fn(|| joined.try_recv()).find_map(|message| match message {
            ServerMessage::OpApplied { op_id, revision, .. } => Some((op_id, revision)),
            _ => None,
        });
        assert_eq!(
            next,
            Some(("op-1".to_owned(), 1)),
            "the op after the revision the session was told must reach it"
        );
    }

    // `VAL-CONF-001`: s1 writes one field and gets revision 1; s2, still at
    // baseline 0, writes another field of the same object. Both stand.
    #[tokio::test]
    async fn a_write_behind_the_revision_to_a_field_nobody_else_wrote_merges() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;

        let first = submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        assert_eq!(applied(&first).map(|(_, revision, _)| revision), Some(1));

        let second = submit(&state, &mut s2, 0, "op-2", set("公称直径", "DN50")).await;
        let (op_id, revision, changes) = applied(&second).expect("merged, so applied");
        assert_eq!((op_id.as_str(), revision), ("op-2", 2));
        assert_eq!(changes[0]["field"], "公称直径");
        assert!(errors(&second).is_empty(), "{second:?}");
        assert!(conflicts_published(&second).is_empty());
        assert!(conflict_rows(&state).is_empty());

        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("1.6MPa"));
        assert_eq!(value_of(&state, "公称直径").as_deref(), Some("DN50"));
    }

    // `VAL-CONF-002` and `-003`: the same field. s2 is behind and yields: an
    // `error` saying so, no conflict entry, the first value stays, and the
    // audit log keeps the value that did not take effect beside the one that
    // did.
    #[tokio::test]
    async fn a_write_behind_the_revision_to_a_field_someone_else_wrote_yields() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        let second = submit(&state, &mut s2, 0, "op-2", set("设计压力", "2.5MPa")).await;
        assert!(applied(&second).is_none(), "{second:?}");
        let refused = errors(&second);
        assert_eq!(refused.len(), 1, "{second:?}");
        assert_eq!(refused[0].0, codes::OP_SUPERSEDED);
        assert!(refused[0].1.contains("1.6MPa") && refused[0].1.contains("2.5MPa"));
        assert!(conflict_rows(&state).is_empty());
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("1.6MPa"));

        let store = state.store.as_ref().expect("store");
        let page = store
            .audit_query(&AuditQuery {
                object_ref: Some("P-1001".into()),
                ..AuditQuery::default()
            })
            .expect("audit");
        let row = page
            .items
            .iter()
            .find(|row| row.action == "property_superseded")
            .expect("the yield is on record");
        assert_eq!(row.user_id, "u2");
        assert!(row.change_text.contains("2.5MPa"), "{}", row.change_text);
        assert!(row.change_text.contains("1.6MPa"), "{}", row.change_text);
        for column in [
            &row.module,
            &row.action,
            &row.object_ref,
            &row.user_id,
            &row.company,
            &row.ip,
            &row.workspace,
            &row.change_text,
        ] {
            assert!(!column.is_empty(), "{row:?}");
        }
        assert!(row.at > 0);
        // The revision did not move for a write that did not happen.
        assert_eq!(store.drawing(DRAWING).expect("drawing").expect("row").2, 1);
    }

    // `VAL-CONF-005`: the field was written at revision 3; a write carrying
    // baseline 1 yields, whatever happened in between.
    #[tokio::test]
    async fn a_write_two_revisions_behind_yields() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        submit(&state, &mut s1, 1, "op-2", set("绝热等级", "H")).await;
        let third = submit(&state, &mut s1, 2, "op-3", set("公称直径", "DN50")).await;
        assert_eq!(applied(&third).map(|(_, revision, _)| revision), Some(3));

        let behind = submit(&state, &mut s2, 1, "op-4", set("公称直径", "DN80")).await;
        assert!(applied(&behind).is_none());
        assert_eq!(errors(&behind)[0].0, codes::OP_SUPERSEDED);
        assert!(conflict_rows(&state).is_empty());
        assert_eq!(value_of(&state, "公称直径").as_deref(), Some("DN50"));
    }

    // `VAL-CONF-004`: with a tie window, two sessions write one field
    // differently against the same revision. Neither takes effect; there is
    // one entry, `value_a` the first to arrive, and the room is told.
    #[tokio::test]
    async fn two_writes_against_one_revision_inside_the_window_make_a_conflict_entry() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;

        let first = submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        assert!(first.is_empty(), "held, so nothing yet: {first:?}");

        let second = submit(&state, &mut s2, 0, "op-2", set("设计压力", "2.5MPa")).await;
        assert!(applied(&second).is_none(), "{second:?}");
        let told = conflicts_published(&second);
        assert_eq!(told.len(), 1, "{second:?}");

        let rows = conflict_rows(&state);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.drawing_id, DRAWING);
        assert_eq!(row.object_ref, "P-1001");
        assert_eq!(row.field, "设计压力");
        assert_eq!(row.value_a, "1.6MPa");
        assert_eq!(row.value_b, "2.5MPa");
        assert_eq!(row.base_revision, 0);
        assert_eq!(row.status, STATUS_PENDING);
        assert_eq!(row.resolution, None);
        assert_eq!(row.resolved_by, None);
        match &told[0] {
            ServerMessage::Conflict {
                conflict_id,
                object_ref,
                field,
                value_a,
                value_b,
                ..
            } => {
                assert_eq!(conflict_id, &row.conflict_id);
                assert_eq!(
                    (object_ref.as_str(), field.as_str(), value_a.as_str(), value_b.as_str()),
                    ("P-1001", "设计压力", "1.6MPa", "2.5MPa")
                );
            }
            other => panic!("expected a conflict, got {other:?}"),
        }

        // Neither side's value is the drawing's, and the revision is where
        // it was. Letting the window pass changes nothing: the held op is
        // gone.
        assert_eq!(value_of(&state, "设计压力"), None);
        let later = flush_due(&state, Instant::now() + Duration::from_secs(5));
        assert!(later.is_empty(), "{later:?}");
        assert_eq!(objects(&state, DRAWING, None).0, 0);

        // Both attempts are on record under the people who made them.
        let store = state.store.as_ref().expect("store");
        let page = store
            .audit_query(&AuditQuery {
                object_ref: Some("P-1001".into()),
                ..AuditQuery::default()
            })
            .expect("audit");
        let mut users: Vec<&str> = page
            .items
            .iter()
            .filter(|row| row.action == "conflict")
            .map(|row| row.user_id.as_str())
            .collect();
        users.sort();
        assert_eq!(users, ["u1", "u2"]);
    }

    // The window is a wait, not a refusal: an op nobody ties with commits
    // when it passes, and its sender is told then.
    #[tokio::test]
    async fn a_held_op_commits_once_its_window_has_passed() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let held = submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        assert!(held.is_empty());
        assert!(flush_due(&state, Instant::now()).is_empty(), "not yet due");

        let due = flush_due(&state, Instant::now() + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, DRAWING);
        assert_eq!(due[0].1.revision, Some(1));
        let (op_id, revision, _) = applied(&due[0].1.reactions).expect("applied");
        assert_eq!((op_id.as_str(), revision), ("op-1", 1));
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("1.6MPa"));
    }

    // Inside the window two sessions writing different fields tie on
    // nothing; both commit, in the order they arrived.
    #[tokio::test]
    async fn inside_the_window_writes_to_different_fields_both_commit() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        let second = submit(&state, &mut s2, 0, "op-2", set("公称直径", "DN50")).await;
        assert!(second.is_empty(), "held as well: {second:?}");

        let due = flush_due(&state, Instant::now() + Duration::from_secs(1));
        let revisions: Vec<(String, i64)> = due
            .iter()
            .filter_map(|(_, outcome)| applied(&outcome.reactions))
            .map(|(op_id, revision, _)| (op_id, revision))
            .collect();
        assert_eq!(revisions, [("op-1".to_owned(), 1), ("op-2".to_owned(), 2)]);
        assert!(conflict_rows(&state).is_empty());
    }

    // A write made against an older revision does not tie with a held one —
    // the held one was made against a newer revision — it yields to it.
    #[tokio::test]
    async fn a_write_behind_the_revision_yields_to_a_held_one() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-0", set("绝热等级", "H")).await;
        flush_due(&state, Instant::now() + Duration::from_secs(1));
        // s1 is at revision 1 and writes; s2 is still at 0.
        let held = submit(&state, &mut s1, 1, "op-1", set("设计压力", "1.6MPa")).await;
        assert!(held.is_empty());
        let behind = submit(&state, &mut s2, 0, "op-2", set("设计压力", "2.5MPa")).await;
        assert_eq!(errors(&behind)[0].0, codes::OP_SUPERSEDED);
        assert!(conflict_rows(&state).is_empty());
    }

    // A session's second edit, sent before its first came back, carries the
    // older baseline. It is the newer intent, not a lagging one.
    #[tokio::test]
    async fn a_sessions_second_edit_sent_before_the_first_came_back_is_taken() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        let second = submit(&state, &mut s1, 0, "op-2", set("设计压力", "2.5MPa")).await;
        assert_eq!(applied(&second).map(|(_, revision, _)| revision), Some(2));
        assert!(errors(&second).is_empty());
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("2.5MPa"));
    }

    // An op the three steps are not defined for keeps the old rule.
    #[tokio::test]
    async fn an_op_of_another_kind_behind_the_revision_is_still_refused_as_stale() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        let unknown = json!({"kind": "set_legend_standard", "value": "HG/T 20519"});
        let behind = submit(&state, &mut s2, 0, "op-2", unknown.clone()).await;
        assert_eq!(errors(&behind)[0].0, codes::STALE_BASE_REVISION);
        let current = submit(&state, &mut s2, 1, "op-3", unknown).await;
        assert_eq!(applied(&current).map(|(_, revision, _)| revision), Some(2));
    }

    fn audit_of(state: &AppState, object_ref: &str) -> Vec<crate::store::AuditRow> {
        state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                object_ref: Some(object_ref.into()),
                ..AuditQuery::default()
            })
            .expect("audit")
            .items
    }

    fn listed(state: &AppState, object_ref: &str) -> bool {
        !objects(state, DRAWING, Some(object_ref)).1.is_empty()
    }

    // `VAL-CONF-006` and `-007`, the object level: s1 deletes P-1002; s2,
    // who has not seen that, changes one of its fields. The deletion stands,
    // s2 is told with the value the field held, there is no conflict entry,
    // and the log has both the deletion and the change that did not happen.
    #[tokio::test]
    async fn a_change_to_an_object_deleted_meanwhile_does_not_happen() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let on = |field: &str, value: &str| {
            json!({"kind": "set_property", "object_ref": "P-1002", "field": field, "value": value})
        };
        submit(&state, &mut s1, 0, "op-1", on("设计压力", "1.0MPa")).await;
        assert!(listed(&state, "P-1002"));

        let delete = json!({"kind": "delete_object", "object_ref": "P-1002"});
        let deleted = submit(&state, &mut s1, 1, "op-2", delete).await;
        assert_eq!(applied(&deleted).map(|(_, revision, _)| revision), Some(2));

        let late = submit(&state, &mut s2, 1, "op-3", on("设计压力", "2.5MPa")).await;
        assert!(applied(&late).is_none(), "{late:?}");
        let told = errors(&late);
        assert_eq!(told.len(), 1, "{late:?}");
        assert_eq!(told[0].0, codes::OBJECT_DELETED);
        assert!(told[0].1.contains("1.0MPa"), "the value it held: {}", told[0].1);
        assert!(!listed(&state, "P-1002"), "the deletion was not rolled back");

        let store = state.store.as_ref().expect("store");
        let about: Vec<ConflictRow> = conflict_rows(&state)
            .into_iter()
            .filter(|row| row.object_ref == "P-1002")
            .collect();
        assert!(about.is_empty(), "the object level makes no conflict entry");
        let rows = audit_of(&state, "P-1002");
        let deletion = rows
            .iter()
            .find(|row| row.action == "delete_object")
            .expect("the deletion is on record");
        assert_eq!(deletion.user_id, "u1");
        assert!(deletion.change_text.contains("1.0MPa"), "{}", deletion.change_text);
        let refusal = rows
            .iter()
            .find(|row| row.action == "change_rejected_deleted")
            .expect("the change that did not happen is on record");
        assert_eq!(refusal.user_id, "u2");
        assert!(refusal.change_text.contains("1.0MPa"), "{}", refusal.change_text);
        assert!(refusal.change_text.contains("2.5MPa"), "{}", refusal.change_text);
        assert_eq!(store.drawing(DRAWING).expect("drawing").expect("row").2, 2);
    }

    // `VAL-CONF-017`: P-1003 is at an end of a run of L-1001. Deleting it is
    // refused, the refusal names the line, the object stays, and the attempt
    // is on record.
    #[tokio::test]
    async fn deleting_an_object_a_pipe_run_ends_on_is_refused() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let vessel =
            json!({"kind": "set_property", "object_ref": "P-1003", "field": "设计压力", "value": "1.0MPa"});
        submit(&state, &mut s1, 0, "op-1", vessel).await;
        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001",
            "points": [[0, 0], [100, 0]], "from": "P-1003"});
        let added = submit(&state, &mut s1, 1, "op-2", run).await;
        let (_, revision, changes) = applied(&added).expect("the run is added");
        assert_eq!(revision, 2);
        assert_eq!(changes[0]["tag"], "L-1001-01", "the server names the run");
        assert_eq!(changes[0]["object_ref"], "L-1001");

        let delete = json!({"kind": "delete_object", "object_ref": "P-1003"});
        let refused = submit(&state, &mut s2, 2, "op-3", delete).await;
        assert!(applied(&refused).is_none(), "{refused:?}");
        let told = errors(&refused);
        assert_eq!(told[0].0, codes::OBJECT_REFERENCED);
        assert!(told[0].1.contains("L-1001"), "names what holds it: {}", told[0].1);
        assert!(listed(&state, "P-1003"), "still there");
        let rows = audit_of(&state, "P-1003");
        let attempt = rows
            .iter()
            .find(|row| row.action == "delete_refused")
            .expect("the refused attempt is on record");
        assert_eq!(attempt.user_id, "u2");
        assert!(attempt.change_text.contains("L-1001"));
    }

    // `VAL-CONF-008` and `-009`, the topology level: with a tie window, s1
    // adds a run to L-1001 and s2 inserts a fitting on the same line, both
    // against one revision. One entry for the line's topology, the room is
    // told, and the line is connected up exactly as it was before either.
    #[tokio::test]
    async fn two_changes_to_one_lines_connections_inside_the_window_make_a_conflict_entry() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let line = json!({"kind": "create_pipe_line", "tag": "L-1001", "props": {"管道等级": "A1A"}});
        let created = submit(&state, &mut s1, 0, "op-0", line).await;
        assert_eq!(
            applied(&created).map(|(_, revision, _)| revision),
            Some(1),
            "creating the line contends for nothing and is not held"
        );
        let before = topology(&state, DRAWING);
        assert_eq!(before.1["pipe_lines"], json!([{"tag": "L-1001", "runs": []}]));

        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001", "points": [[0, 0], [100, 0]]});
        let held = submit(&state, &mut s1, 1, "op-1", run).await;
        assert!(held.is_empty(), "{held:?}");
        let fitting = json!({"kind": "insert_fitting", "run_tag": "L-1001-01",
            "fitting_type": "gate_valve", "at": [50, 0]});
        let second = submit(&state, &mut s2, 1, "op-2", fitting).await;
        assert!(applied(&second).is_none(), "{second:?}");
        assert_eq!(conflicts_published(&second).len(), 1);

        let rows: Vec<ConflictRow> = conflict_rows(&state)
            .into_iter()
            .filter(|row| row.object_ref == "L-1001")
            .collect();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.field, "topology");
        assert!(row.value_a.contains("add_pipe_run"), "{}", row.value_a);
        assert!(row.value_b.contains("insert_fitting"), "{}", row.value_b);
        assert_eq!(row.base_revision, 1);
        assert_eq!(row.status, STATUS_PENDING);

        // Neither side's version: the line is as it was, now and after the
        // window has passed.
        assert_eq!(topology(&state, DRAWING), before);
        assert!(flush_due(&state, Instant::now() + Duration::from_secs(5)).is_empty());
        assert_eq!(topology(&state, DRAWING), before);
    }

    // Without a window nothing ties, and connections do not merge either: a
    // change made before another session's change to the same line is
    // behind, and its sender is asked to look again.
    #[tokio::test]
    async fn a_connection_change_behind_another_on_the_same_line_is_stale() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001", "points": [[0, 0], [100, 0]]});
        submit(&state, &mut s1, 0, "op-1", run).await;
        let fitting = json!({"kind": "insert_fitting", "run_tag": "L-1001-01", "at": [50, 0]});

        let behind = submit(&state, &mut s2, 0, "op-2", fitting.clone()).await;
        assert_eq!(errors(&behind)[0].0, codes::STALE_BASE_REVISION);
        assert!(conflict_rows(&state).is_empty());

        let seen = submit(&state, &mut s2, 1, "op-3", fitting).await;
        assert_eq!(applied(&seen).map(|(_, revision, _)| revision), Some(2));
        // A line nobody else touched merges like a field nobody else wrote.
        let elsewhere = json!({"kind": "add_pipe_run", "line_tag": "L-1002", "points": []});
        let merged = submit(&state, &mut s2, 0, "op-4", elsewhere).await;
        assert_eq!(applied(&merged).map(|(_, revision, _)| revision), Some(3));
        let (_, shape) = topology(&state, DRAWING);
        assert_eq!(shape["pipe_runs"].as_array().map(Vec::len), Some(2));
        assert_eq!(shape["pipe_runs"][0]["fittings"][0]["at"], json!([50, 0]));
    }

    // The topology is in the op log like everything else.
    #[tokio::test]
    async fn the_topology_is_rebuilt_from_the_op_log() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001",
            "points": [[0, 0], [100, 0]], "from": "P-1003"});
        submit(&state, &mut s1, 0, "op-1", run).await;
        submit(
            &state,
            &mut s1,
            1,
            "op-2",
            json!({"kind": "delete_object", "object_ref": "P-9"}),
        )
        .await;
        let before = topology(&state, DRAWING);
        state.drawings.lock().expect("drawings").clear();
        assert_eq!(topology(&state, DRAWING), before);
        // And the reference survived with it.
        let mut s2 = join(&state, "u2").await;
        let delete = json!({"kind": "delete_object", "object_ref": "P-1003"});
        let refused = submit(&state, &mut s2, 2, "op-3", delete).await;
        assert_eq!(errors(&refused)[0].0, codes::OBJECT_REFERENCED);
    }

    // One undo step is one op. What merges of it is applied and what is
    // behind yields, change by change; the revision moves once.
    #[tokio::test]
    async fn a_batched_op_applies_what_merges_and_yields_the_rest() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        let mut batch = set("设计压力", "2.5MPa");
        batch["changes"] = json!([set("公称直径", "DN50"), set("绝热等级", "H")]);
        let outcome = submit(&state, &mut s2, 0, "op-2", batch).await;
        let (_, revision, changes) = applied(&outcome).expect("two of three apply");
        assert_eq!(revision, 2);
        let fields: Vec<&str> = changes
            .iter()
            .map(|change| change["field"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(fields, ["公称直径", "绝热等级"]);
        assert_eq!(errors(&outcome).len(), 1);
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("1.6MPa"));
        assert_eq!(value_of(&state, "绝热等级").as_deref(), Some("H"));
    }

    // What the server knows of a drawing is the op log. Forget the model —
    // as a restart does — and a write behind the revision is judged the same.
    #[tokio::test]
    async fn the_drawing_is_rebuilt_from_the_op_log() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        submit(
            &state,
            &mut s1,
            1,
            "op-2",
            json!({"kind": "set_tag", "object_ref": "P-1001", "value": "P-1009"}),
        )
        .await;

        state.drawings.lock().expect("drawings").clear();

        let (revision, items) = objects(&state, DRAWING, None);
        assert_eq!(revision, 2);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["object_ref"], "P-1009");
        assert_eq!(items[0]["fields"]["设计压力"]["value"], "1.6MPa");

        // s2 never saw the retag: it still says P-1001, and is still behind
        // on the field s1 wrote.
        let behind = submit(&state, &mut s2, 0, "op-3", set("设计压力", "2.5MPa")).await;
        assert_eq!(errors(&behind)[0].0, codes::OP_SUPERSEDED);
        // A field s1 did not write merges — and goes out under the tag the
        // object has now, so that everyone can find it.
        let merged = submit(&state, &mut s2, 0, "op-4", set("公称直径", "DN50")).await;
        let (_, revision, changes) = applied(&merged).expect("merged");
        assert_eq!(revision, 3);
        assert_eq!(changes[0]["object_ref"], "P-1009");
    }

    // Without a database the judgment still runs on what this process saw.
    #[tokio::test]
    async fn without_a_database_writes_are_still_merged_and_yielded() {
        let state = AppState::new(
            None,
            Some("no database".into()),
            "missing.db",
            SessionConfig::default(),
        );
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;
        let merged = submit(&state, &mut s2, 0, "op-2", set("公称直径", "DN50")).await;
        assert_eq!(applied(&merged).map(|(_, revision, _)| revision), Some(2));
        let behind = submit(&state, &mut s2, 0, "op-3", set("设计压力", "2.5MPa")).await;
        assert_eq!(errors(&behind)[0].0, codes::OP_SUPERSEDED);
    }

    #[test]
    fn the_flush_loop_looks_often_enough_for_the_window() {
        assert_eq!(flush_interval(Duration::from_millis(8)), Duration::from_millis(5));
        assert_eq!(flush_interval(Duration::from_millis(100)), Duration::from_millis(25));
        assert_eq!(flush_interval(Duration::from_secs(3)), Duration::from_millis(50));
    }

    // ── `VAL-CROSS-008`: coming back, and being refused ──────────────────

    /// A `hello` from a session that says where its drawing was.
    /// A `hello` from a copy that was here before: it claims this
    /// repository's id and the revision its drawing had reached
    /// (`REQ-COLL-015`).
    async fn come_back(state: &AppState, user_id: &str, since: Option<i64>) -> Vec<Reaction> {
        let mut conn = Connection::new("10.0.0.9");
        let hello = ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: DRAWING.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since,
            existing_tags: Vec::new(),
            repo_id: Some(state.repo_id.clone()),
        };
        handle_message(state, &mut conn, hello, T0 + 5).await
    }

    /// What a `hello` was answered with on its own socket: the revision the
    /// room is at, where the replay starts, and the ops replayed, in order.
    fn welcomed(reactions: &[Reaction]) -> (i64, Option<i64>, Vec<(String, i64, Vec<Value>)>) {
        let mut told = None;
        let mut replayed = Vec::new();
        for reaction in reactions {
            match reaction {
                Reaction::Send(ServerMessage::Welcome { base_revision, replay_from, .. }) => {
                    assert!(replayed.is_empty(), "the replay comes behind the welcome");
                    told = Some((*base_revision, *replay_from));
                }
                Reaction::Send(ServerMessage::OpApplied { op_id, revision, changes }) => {
                    replayed.push((op_id.clone(), *revision, changes.clone()));
                }
                _ => {}
            }
        }
        let (base_revision, replay_from) = told.expect("welcome");
        (base_revision, replay_from, replayed)
    }

    /// A session that was away says the revision its drawing had reached,
    /// and is sent every op the room applied after it — each as the room
    /// first heard it, behind the `welcome` and ahead of anything else. It
    /// used to be welcomed at the room's revision with nothing in between,
    /// and could only mark itself as no longer matching the room.
    ///
    /// Nothing is replayed to a session that does not ask, to one that names
    /// a revision this drawing has not reached, or when there is no log to
    /// replay from; such a session finds out for itself that it is behind.
    #[tokio::test]
    async fn collab_offline_queue_a_session_that_comes_back_is_sent_what_it_missed() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut said = Vec::new();
        for (revision, (field, value)) in
            [("设计压力", "1.6MPa"), ("公称直径", "DN50"), ("绝热等级", "H")].into_iter().enumerate()
        {
            let op_id = format!("op-{}", revision + 1);
            let reactions = submit(&state, &mut s1, revision as i64, &op_id, set(field, value)).await;
            said.push(applied(&reactions).expect("applied"));
        }

        let (base, from, replayed) = welcomed(&come_back(&state, "u2", Some(1)).await);
        assert_eq!((base, from), (3, Some(1)));
        assert_eq!(replayed, said[1..], "revisions 2 and 3, as the room heard them");

        let (base, from, replayed) = welcomed(&come_back(&state, "u3", Some(3)).await);
        assert_eq!((base, from), (3, Some(3)), "nothing missed is still an answer");
        assert!(replayed.is_empty());
        let (base, from, replayed) = welcomed(&come_back(&state, "u4", Some(0)).await);
        assert_eq!((base, from, replayed.len()), (3, Some(0), 3));

        // Not asked; and a revision this drawing never reached.
        for since in [None, Some(7), Some(-1)] {
            let (base, from, replayed) = welcomed(&come_back(&state, "u5", since).await);
            assert_eq!((base, from), (3, None), "{since:?}");
            assert!(replayed.is_empty(), "{since:?}");
        }

        // With no database there is no log: nothing can be replayed, except
        // to a session that missed nothing.
        let bare = AppState::new(None, Some("no database".into()), "missing.db", SessionConfig::default());
        let mut only = join(&bare, "u1").await;
        submit(&bare, &mut only, 0, "op-1", set("设计压力", "1.6MPa")).await;
        assert_eq!(welcomed(&come_back(&bare, "u2", Some(0)).await).1, None);
        assert_eq!(welcomed(&come_back(&bare, "u2", Some(1)).await).1, Some(1));
    }

    /// The replay and the subscription are one step: an op that commits
    /// right behind the `hello` reaches the room's channel and is not in
    /// the replay, and one that committed right before it is in the replay
    /// and not on the channel a second time.
    #[tokio::test]
    async fn collab_offline_queue_the_replay_and_the_room_overlap_nowhere() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        let mut late = Connection::new("10.0.0.9");
        let hello = ClientMessage::Hello {
            user_id: "u2".into(),
            drawing_id: DRAWING.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since: Some(0),
            existing_tags: Vec::new(),
            repo_id: Some(state.repo_id.clone()),
        };
        let reactions = handle_message(&state, &mut late, hello, T0 + 5).await;
        let (_, _, replayed) = welcomed(&reactions);
        assert_eq!(replayed.iter().map(|(op_id, ..)| op_id.as_str()).collect::<Vec<_>>(), ["op-1"]);
        submit(&state, &mut s1, 1, "op-2", set("公称直径", "DN50")).await;

        let mut joined = late.joined.take().expect("the subscription hello took out");
        let heard: Vec<String> = std::iter::from_fn(|| joined.try_recv())
            .filter_map(|message| match message {
                ServerMessage::OpApplied { op_id, .. } => Some(op_id),
                _ => None,
            })
            .collect();
        assert_eq!(heard, ["op-2"], "only what came after the replay");
    }

    /// `(code, op_id, rejected)` of every `error` sent back.
    fn answers(reactions: &[Reaction]) -> Vec<(String, Option<String>, Option<bool>)> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Send(ServerMessage::Error { code, op_id, rejected, .. })
                | Reaction::SendTo {
                    message: ServerMessage::Error { code, op_id, rejected, .. },
                    ..
                } => Some((code.clone(), op_id.clone(), *rejected)),
                _ => None,
            })
            .collect()
    }

    /// An `error` that answers an op names the op, and says whether nothing
    /// of it took effect — which is what lets its sender take that step,
    /// and no other, back. It used to carry a code and a sentence.
    #[tokio::test]
    async fn collab_offline_queue_an_error_names_its_op_and_says_whether_nothing_took_effect() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        // Behind another session's write to the same field: the one change
        // the op carries yields, so nothing of it took effect.
        let behind = submit(&state, &mut s2, 0, "op-2", set("设计压力", "2.5MPa")).await;
        assert_eq!(
            answers(&behind),
            [(codes::OP_SUPERSEDED.to_owned(), Some("op-2".to_owned()), Some(true))]
        );
        assert!(applied(&behind).is_none());

        // Two field writes, one behind: that one is answered without
        // `rejected`, and the other takes effect under the same op id.
        let mut both = set("设计压力", "4.0MPa");
        both["changes"] = json!([set("公称直径", "DN80")]);
        let partly = submit(&state, &mut s2, 0, "op-3", both).await;
        assert_eq!(answers(&partly), [(codes::OP_SUPERSEDED.to_owned(), Some("op-3".to_owned()), None)]);
        let (op_id, revision, changes) = applied(&partly).expect("the other field merged");
        assert_eq!((op_id.as_str(), revision, changes.len()), ("op-3", 2, 1));
        assert_eq!(value_of(&state, "公称直径").as_deref(), Some("DN80"));

        // A baseline the drawing has not reached, and an op the judgment
        // does not read made against an old one: refused whole.
        let ahead = submit(&state, &mut s2, 9, "op-4", set("设计压力", "4.0MPa")).await;
        assert_eq!(
            answers(&ahead),
            [(codes::STALE_BASE_REVISION.to_owned(), Some("op-4".to_owned()), Some(true))]
        );
        let opaque = submit(&state, &mut s2, 0, "op-5", json!({"kind": "something_else", "tag": "X"})).await;
        assert_eq!(
            answers(&opaque),
            [(codes::STALE_BASE_REVISION.to_owned(), Some("op-5".to_owned()), Some(true))]
        );
    }

    /// An op that makes, erases or reconnects something is taken whole or
    /// not at all. One of its changes refused used to leave the rest in
    /// effect — the symbol placed, say, and the field it was to carry not
    /// written — which is a step nobody made and nobody could take back.
    /// Field writes alone are still judged one by one (the test above).
    #[tokio::test]
    async fn collab_offline_queue_an_op_that_carries_structure_is_taken_whole_or_not_at_all() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-1", set("设计压力", "1.6MPa")).await;

        let mut step = json!({"kind": "insert_symbol", "type": "VALVE", "tag": "V-1001", "position": [5, 5]});
        step["changes"] = json!([set("设计压力", "2.5MPa")]);
        let refused = submit(&state, &mut s2, 0, "op-2", step.clone()).await;
        assert_eq!(
            answers(&refused),
            [(codes::OP_SUPERSEDED.to_owned(), Some("op-2".to_owned()), Some(true))]
        );
        assert!(applied(&refused).is_none(), "the symbol went in though the op was refused");
        let (revision, items) = objects(&state, DRAWING, Some("V-1001"));
        assert_eq!((revision, items.len()), (1, 0), "{items:?}");

        // Made against the drawing as it is, the same step goes in whole.
        let taken = submit(&state, &mut s2, 1, "op-3", step).await;
        assert!(answers(&taken).is_empty(), "{taken:?}");
        assert_eq!(applied(&taken).map(|(_, revision, changes)| (revision, changes.len())), Some((2, 2)));
        assert_eq!(objects(&state, DRAWING, Some("V-1001")).1.len(), 1);

        // A change to a line's connections behind another session's, with
        // a field write beside it: neither happens.
        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001", "tag": "L-1001-01", "points": [[0, 0], [9, 0]]});
        submit(&state, &mut s1, 2, "op-4", run).await;
        let mut late = json!({"kind": "insert_fitting", "line_tag": "L-1001", "run_tag": "L-1001-01",
            "fitting_type": "gate_valve", "at": [5, 0]});
        late["changes"] = json!([set("绝热等级", "H")]);
        let refused = submit(&state, &mut s2, 2, "op-5", late).await;
        assert_eq!(
            answers(&refused),
            [(codes::STALE_BASE_REVISION.to_owned(), Some("op-5".to_owned()), Some(true))]
        );
        assert!(applied(&refused).is_none());
        assert_eq!(value_of(&state, "绝热等级"), None);
    }

    /// A conflict entry names the two ops that tied: neither gets an
    /// `op_applied` or an `error` for it, and a client that waits for its
    /// op to be answered before it sends the next would wait for good.
    #[tokio::test]
    async fn collab_offline_queue_a_conflict_names_the_two_ops_that_tied() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        let held = submit(&state, &mut s1, 0, "op-a", set("设计压力", "1.6MPa")).await;
        assert!(held.is_empty(), "held inside the window: {held:?}");
        let tied = submit(&state, &mut s2, 0, "op-b", set("设计压力", "2.5MPa")).await;
        let told = conflicts_published(&tied);
        assert_eq!(told.len(), 1, "{tied:?}");
        match &told[0] {
            ServerMessage::Conflict { op_ids, .. } => assert_eq!(op_ids, &["op-a", "op-b"]),
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert!(answers(&tied).is_empty() && applied(&tied).is_none());
    }

    // ── Connectors between drawings (`VAL-CROSS-009`) ────────────────────

    const OTHER: &str = "PID-1102";

    async fn join_on(state: &AppState, user_id: &str, drawing_id: &str) -> Connection {
        let mut conn = Connection::new("10.0.0.7");
        let hello = ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: drawing_id.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since: None,
            existing_tags: Vec::new(),
            repo_id: None,
        };
        handle_message(state, &mut conn, hello, T0).await;
        assert!(conn.session.is_some(), "hello must open a session");
        conn
    }

    fn outflow(line: &str, to: &str) -> Value {
        json!({"kind": "create_connector", "line_tag": line, "connector": "pipe_out",
               "at": [120.0, 0.0], "target_drawing": to})
    }

    fn inflow(line: &str, x: f64) -> Value {
        json!({"kind": "create_connector", "line_tag": line, "connector": "pipe_in", "at": [x, 0.0]})
    }

    fn line_field(line: &str, field: &str, value: &str) -> Value {
        json!({"kind": "set_property", "object_ref": line, "field": field, "value": value})
    }

    /// Every `op_applied` among `reactions` as `(drawing, op_id, revision,
    /// kinds of its changes)`.
    fn told(reactions: &[Reaction]) -> Vec<(String, String, i64, Vec<String>)> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Published {
                    drawing_id,
                    message:
                        ServerMessage::OpApplied {
                            op_id,
                            revision,
                            changes,
                        },
                } => Some((
                    drawing_id.clone(),
                    op_id.clone(),
                    *revision,
                    changes
                        .iter()
                        .map(|change| change["kind"].as_str().unwrap_or_default().to_owned())
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    }

    fn line_value(state: &AppState, drawing: &str, line: &str, field: &str) -> Option<String> {
        let (_, items) = objects(state, drawing, Some(line));
        items
            .first()
            .and_then(|item| item["fields"][field]["value"].as_str().map(str::to_owned))
    }

    fn op_log(state: &AppState, drawing: &str) -> Vec<crate::store::StoredOp> {
        state
            .store
            .as_ref()
            .expect("store")
            .load_ops(drawing, 0)
            .expect("op log")
    }

    async fn base_of(state: &AppState, conn: &Connection) -> i64 {
        let session_id = &conn.session.as_ref().expect("session").session_id;
        state
            .sessions
            .lock()
            .await
            .get(session_id)
            .expect("registered")
            .base_revision
    }

    /// `VAL-CROSS-009`: u1 on PID-1101 makes an outflow connector on L-1001
    /// that names PID-1102; u2 on PID-1102 makes the inflow on L-1001. The
    /// server pairs them: each room is told of its own end by an op of the
    /// server's, the op logs of both drawings have it, and the sessions of
    /// both are at the revisions those ops took.
    #[tokio::test]
    async fn collab_connector_cross_drawing_the_server_pairs_an_outflow_with_the_inflow_on_the_drawing_it_names() {
        let state = state();
        let mut u1 = join_on(&state, "u1", DRAWING).await;
        let mut u2 = join_on(&state, "u2", OTHER).await;

        let first = submit(&state, &mut u1, 0, "op-out", outflow("L-1001", OTHER)).await;
        assert_eq!(
            told(&first),
            [(DRAWING.to_owned(), "op-out".to_owned(), 1, vec!["create_connector".to_owned()])],
            "nothing to pair with yet"
        );

        let second = submit(&state, &mut u2, 0, "op-in", inflow("L-1001", 20.0)).await;
        let said = told(&second);
        assert_eq!(said.len(), 3, "{said:?}");
        assert_eq!(said[0], (OTHER.to_owned(), "op-in".to_owned(), 1, vec!["create_connector".to_owned()]));
        let here = said.iter().find(|(drawing, op_id, ..)| drawing == OTHER && op_id.starts_with("pair-"));
        let there = said.iter().find(|(drawing, op_id, ..)| drawing == DRAWING && op_id.starts_with("pair-"));
        assert_eq!(here.map(|(_, _, revision, kinds)| (*revision, kinds.clone())), Some((2, vec!["pair_connector".to_owned()])));
        assert_eq!(there.map(|(_, _, revision, kinds)| (*revision, kinds.clone())), Some((2, vec!["pair_connector".to_owned()])));

        // What each room was told names the other end.
        let pair_on = |drawing: &str| -> Value {
            second
                .iter()
                .find_map(|reaction| match reaction {
                    Reaction::Published {
                        drawing_id,
                        message: ServerMessage::OpApplied { op_id, changes, .. },
                    } if drawing_id == drawing && op_id.starts_with("pair-") => Some(changes[0].clone()),
                    _ => None,
                })
                .expect("a pairing op")
        };
        let on_first = pair_on(DRAWING);
        assert_eq!(
            (&on_first["connector"], &on_first["peer_drawing"], &on_first["peer_connector"], &on_first["peer_line_tag"]),
            (&json!("pipe_out"), &json!(OTHER), &json!("pipe_in"), &json!("L-1001")),
            "{on_first}"
        );
        let on_second = pair_on(OTHER);
        assert_eq!(
            (&on_second["connector"], &on_second["peer_drawing"], &on_second["peer_connector"]),
            (&json!("pipe_in"), &json!(DRAWING), &json!("pipe_out")),
            "{on_second}"
        );

        // In both op logs, under a session that is nobody's.
        for drawing in [DRAWING, OTHER] {
            let log = op_log(&state, drawing);
            assert_eq!(log.len(), 2, "{drawing}: {log:?}");
            assert_eq!(log[1].session_id, format!("link:{OTHER}"), "{drawing}");
            assert_eq!(log[1].user_id, "u2", "on the account of whoever's op it follows from");
        }
        let (_, topology_here) = topology(&state, DRAWING);
        assert_eq!(topology_here["connectors"][0]["peer_drawing"], OTHER, "{topology_here}");
        assert_eq!((base_of(&state, &u1).await, base_of(&state, &u2).await), (2, 2));
    }

    /// What the line that flows out carries reaches the line that flows in:
    /// with the pairing what it has by then, and afterwards every write of
    /// one of the fields that go along a pipe connector — as an op of the
    /// server's in the other drawing's room and op log. Nothing else of the
    /// line goes, and nothing comes back.
    #[tokio::test]
    async fn collab_connector_cross_drawing_a_field_of_the_outflow_line_is_written_on_the_inflow_line() {
        let state = state();
        let mut u1 = join_on(&state, "u1", DRAWING).await;
        let mut u2 = join_on(&state, "u2", OTHER).await;
        submit(&state, &mut u1, 0, "op-dn", line_field("L-1001", "公称直径", "DN50")).await;
        submit(&state, &mut u1, 1, "op-out", outflow("L-1001", OTHER)).await;
        let paired = submit(&state, &mut u2, 0, "op-in", inflow("L-1001", 20.0)).await;
        let with_pairing = told(&paired)
            .into_iter()
            .find(|(drawing, op_id, ..)| drawing == OTHER && op_id.starts_with("pair-"))
            .expect("the pairing op on the inflow side");
        assert_eq!(with_pairing.3, ["pair_connector", "set_property"], "carried with the pairing");
        assert_eq!(line_value(&state, OTHER, "L-1001", "公称直径").as_deref(), Some("DN50"));

        let written = submit(&state, &mut u1, 3, "op-grade", line_field("L-1001", "管道等级", "A1A")).await;
        let said = told(&written);
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!((said[0].0.as_str(), said[0].1.as_str()), (DRAWING, "op-grade"));
        assert_eq!(said[1].0, OTHER);
        assert!(said[1].1.starts_with("carry-PID-1101-"), "{said:?}");
        assert_eq!(said[1].3, ["set_property"]);
        assert_eq!(line_value(&state, OTHER, "L-1001", "管道等级").as_deref(), Some("A1A"));
        let log = op_log(&state, OTHER);
        let last = log.last().expect("an op");
        assert_eq!((last.session_id.as_str(), last.user_id.as_str()), ("link:PID-1101", "u1"));
        assert_eq!(last.changes, [line_field("L-1001", "管道等级", "A1A")]);
        assert_eq!(base_of(&state, &u2).await, last.revision, "the other room's sessions moved on");
        // On record on the drawing it arrived on.
        let audit = state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                object_ref: Some("L-1001".into()),
                ..AuditQuery::default()
            })
            .expect("audit");
        assert!(
            audit.items.iter().any(|row| row.change_text.contains("沿连接符") && row.change_text.contains("A1A")),
            "{:?}",
            audit.items
        );

        // A field that does not go along a connector stays; so does a
        // write on the inflow side.
        let before = op_log(&state, OTHER).len();
        submit(&state, &mut u1, 4, "op-note", line_field("L-1001", "备注", "x")).await;
        assert_eq!(op_log(&state, OTHER).len(), before);
        let base = base_of(&state, &u2).await;
        let back = submit(&state, &mut u2, base, "op-back", line_field("L-1001", "管道等级", "C1")).await;
        assert_eq!(told(&back).len(), 1, "nothing flows back: {back:?}");
        assert_eq!(line_value(&state, DRAWING, "L-1001", "管道等级").as_deref(), Some("A1A"));
    }

    /// A second inflow connector on a line of a drawing that has one is
    /// refused by the server too — two people making one each at the same
    /// moment both pass their own editor's check — and refused whole, by
    /// name, so that the step is taken back.
    #[tokio::test]
    async fn collab_connector_cross_drawing_the_second_inflow_on_a_line_is_refused_whole() {
        let state = state();
        let mut u1 = join_on(&state, "u1", OTHER).await;
        let mut u2 = join_on(&state, "u2", OTHER).await;
        submit(&state, &mut u1, 0, "op-in-1", inflow("L-1001", 20.0)).await;
        let refused = submit(&state, &mut u2, 1, "op-in-2", inflow("L-1001", 60.0)).await;
        assert!(applied(&refused).is_none(), "{refused:?}");
        let said = answers(&refused);
        assert_eq!(said.len(), 1, "{refused:?}");
        assert_eq!(
            said[0],
            (codes::DUPLICATE_INFLOW_CONNECTOR.to_owned(), Some("op-in-2".to_owned()), Some(true))
        );
        // Another line, and an outflow on the same line, are nobody's
        // duplicate.
        assert!(applied(&submit(&state, &mut u2, 1, "op-in-3", inflow("L-1002", 20.0)).await).is_some());
        assert!(applied(&submit(&state, &mut u2, 2, "op-out", outflow("L-1001", DRAWING)).await).is_some());
    }

    /// The pairing is in the op logs, so a server that starts again has it:
    /// what is written on the outflow line afterwards still arrives. And a
    /// connector taken away leaves its other end unpaired, after which
    /// nothing is carried.
    #[tokio::test]
    async fn collab_connector_cross_drawing_a_pairing_outlives_a_restart_and_ends_with_its_connector() {
        let state = state();
        let mut u1 = join_on(&state, "u1", DRAWING).await;
        let mut u2 = join_on(&state, "u2", OTHER).await;
        submit(&state, &mut u2, 0, "op-in", inflow("L-1001", 20.0)).await;
        submit(&state, &mut u1, 0, "op-out", outflow("L-1001", OTHER)).await;

        // What a restart does to what the server knows of the drawings:
        // it is read again from the op log.
        state.drawings.lock().expect("drawings").clear();
        let written = submit(&state, &mut u1, 2, "op-grade", line_field("L-1001", "管道等级", "A1A")).await;
        assert_eq!(told(&written).len(), 2, "{written:?}");
        assert_eq!(line_value(&state, OTHER, "L-1001", "管道等级").as_deref(), Some("A1A"));

        // A pairing a client claims is not one.
        let mut u3 = join_on(&state, "u3", "PID-1103").await;
        submit(&state, &mut u3, 0, "op-in-3", inflow("L-1001", 20.0)).await;
        let forged = json!({"kind": "pair_connector", "line_tag": "L-1001", "connector": "pipe_in",
            "peer_drawing": DRAWING, "peer_line_tag": "L-1001", "peer_connector": "pipe_out"});
        submit(&state, &mut u3, 1, "op-forged", forged).await;
        state.drawings.lock().expect("drawings").clear();
        let (_, third) = topology(&state, "PID-1103");
        assert!(third["connectors"][0]["peer_drawing"].is_null(), "{third}");

        let gone = json!({"kind": "delete_connector", "line_tag": "L-1001",
                          "connector": "pipe_out", "at": [120.0, 0.0]});
        let base = base_of(&state, &u1).await;
        let removed = submit(&state, &mut u1, base, "op-gone", gone).await;
        let said = told(&removed);
        assert_eq!(said.len(), 2, "{said:?}");
        assert_eq!((said[1].0.as_str(), said[1].3.as_slice()), (OTHER, &["pair_connector".to_owned()][..]));
        let (_, other_side) = topology(&state, OTHER);
        assert!(other_side["connectors"][0]["peer_drawing"].is_null(), "{other_side}");
        let base = base_of(&state, &u1).await;
        let later = submit(&state, &mut u1, base, "op-grade-2", line_field("L-1001", "管道等级", "B2")).await;
        assert_eq!(told(&later).len(), 1, "{later:?}");
        assert_eq!(line_value(&state, OTHER, "L-1001", "管道等级").as_deref(), Some("A1A"));
    }

    // ── The tags of a client's own copy (`hello.existing_tags`) ──────────

    /// A `hello` from `user_id`, whose own copy of the drawing — at
    /// revision `since` of the room, when it says so — has objects under
    /// `tags`.
    async fn join_with(
        state: &AppState,
        user_id: &str,
        since: Option<i64>,
        tags: &[&str],
    ) -> Connection {
        let mut conn = Connection::new("10.0.0.8");
        let hello = ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: DRAWING.into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since,
            existing_tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            repo_id: Some(state.repo_id.clone()),
        };
        handle_message(state, &mut conn, hello, T0).await;
        assert!(conn.session.is_some(), "hello must open a session");
        conn
    }

    /// `conn` places a valve it calls `tag`: the tag the room hears it by.
    async fn place(state: &AppState, conn: &mut Connection, op_id: &str, tag: &str) -> String {
        let valve = json!({"kind": "insert_symbol", "type": "VALVE", "tag": tag,
            "position": [0, 0]});
        let base = base_of(state, conn).await;
        let reactions = submit(state, conn, base, op_id, valve).await;
        let (_, _, changes) = applied(&reactions).expect("the valve is placed");
        changes[0]["tag"].as_str().expect("its tag").to_owned()
    }

    /// `conn` deletes what goes by `tag`.
    async fn remove(state: &AppState, conn: &mut Connection, op_id: &str, tag: &str) {
        let delete = json!({"kind": "delete_object", "object_ref": tag});
        let base = base_of(state, conn).await;
        let reactions = submit(state, conn, base, op_id, delete).await;
        assert!(applied(&reactions).is_some(), "{reactions:?}");
    }

    /// The tags the server holds on a client's word alone.
    fn on_word(state: &AppState) -> Vec<String> {
        reported_tags(state, DRAWING, None)
    }

    /// Two people open a file each and work on one drawing. u1's has two
    /// valves in it that never went through the room; u2's has neither.
    /// u1 lists them as it joins, and the first valve u2 places — V-1001,
    /// for all u2's copy knows — goes out under a tag neither copy has. It
    /// used to keep V-1001, and u1's copy had two of them from then on.
    ///
    /// Once an op deletes what went by a tag, nobody's copy has it and it
    /// can be given out.
    #[tokio::test]
    async fn collab_existing_tags_a_symbol_is_not_given_a_tag_another_copy_has() {
        let state = state();
        let mut u1 = join_with(&state, "u1", None, &["V-1001", "V-1002"]).await;
        let mut u2 = join(&state, "u2").await;
        assert_eq!(on_word(&state), ["V-1001", "V-1002"]);

        assert_eq!(place(&state, &mut u2, "op-1", "V-1001").await, "V-1003");

        remove(&state, &mut u1, "op-2", "V-1002").await;
        assert_eq!(on_word(&state), ["V-1001"]);
        assert_eq!(place(&state, &mut u2, "op-3", "V-1002").await, "V-1002");
    }

    /// What the server knows of a drawing is the op log, and what clients
    /// said is in their own copies. A server that starts again has both:
    /// the tags are kept beside the log, each at the point of the log it
    /// was heard at, so that what an op took away after that is free again
    /// and what was heard of after that is not.
    #[tokio::test]
    async fn collab_existing_tags_are_held_again_after_a_restart() {
        let state = state();
        let tags = ["V-1001", "V-1002", "V-1003", "V-1004"];
        let mut u1 = join_with(&state, "u1", None, &tags).await;
        // V-1002 and V-1003 are deleted. A third copy that always had a
        // V-1003 of its own then joins. And a write that says nothing new
        // drops the record that V-1002 was ever deleted: only its place in
        // the log tells that the tag is free.
        remove(&state, &mut u1, "op-1", "V-1002").await;
        remove(&state, &mut u1, "op-2", "V-1003").await;
        let _u3 = join_with(&state, "u3", None, &["V-1003"]).await;
        let same = json!({"kind": "set_tag", "object_ref": "V-1002", "value": "V-1002"});
        submit(&state, &mut u1, 2, "op-3", same).await;
        assert_eq!(on_word(&state), ["V-1001", "V-1003", "V-1004"]);

        state.drawings.lock().expect("drawings").clear();

        assert_eq!(on_word(&state), ["V-1001", "V-1003", "V-1004"]);
        let mut u2 = join(&state, "u2").await;
        assert_eq!(place(&state, &mut u2, "op-4", "V-1001").await, "V-1002");
        assert_eq!(place(&state, &mut u2, "op-5", "V-1003").await, "V-1005");
    }

    /// A copy that comes back lists what it has — and is behind: the room
    /// deleted two valves since, and the replay that follows its welcome
    /// takes them out of that copy too. Their tags are not registered, or
    /// they would be held for objects nobody has. A copy that is sent no
    /// replay keeps what it lists, and the tags stay taken.
    #[tokio::test]
    async fn collab_existing_tags_a_copy_that_is_behind_holds_no_tag_the_replay_takes_away() {
        let state = state();
        let mut u1 = join(&state, "u1").await;
        place(&state, &mut u1, "op-1", "V-1001").await;
        place(&state, &mut u1, "op-2", "V-1002").await;
        remove(&state, &mut u1, "op-3", "V-1001").await;
        remove(&state, &mut u1, "op-4", "V-1002").await;

        // u2 left at revision 2 with both valves, and is sent op-3 and op-4.
        let _u2 = join_with(&state, "u2", Some(2), &["V-1001", "V-1002"]).await;
        assert!(on_word(&state).is_empty(), "{:?}", on_word(&state));
        // u3's file has a V-1002 that was never the room's; it is sent no
        // replay and its tags stay taken. u4 names a revision this drawing
        // has not reached — the alignment is refused outright, and a
        // refused copy's tags are registered for nobody.
        let _u3 = join_with(&state, "u3", None, &["V-1002"]).await;
        let _u4 = join_with(&state, "u4", Some(9), &["V-1003"]).await;
        assert_eq!(on_word(&state), ["V-1002"]);

        assert_eq!(place(&state, &mut u1, "op-5", "V-1001").await, "V-1001");
        assert_eq!(place(&state, &mut u1, "op-6", "V-1002").await, "V-1003");
    }

    /// Without a database the tags are still told apart, for as long as
    /// this process lives.
    #[tokio::test]
    async fn collab_existing_tags_are_told_apart_without_a_database() {
        let state = AppState::new(
            None,
            Some("no database".into()),
            "missing.db",
            SessionConfig::default(),
        );
        let _u1 = join_with(&state, "u1", None, &["V-1001"]).await;
        let mut u2 = join(&state, "u2").await;
        assert_eq!(place(&state, &mut u2, "op-1", "V-1001").await, "V-1002");
    }

    fn valve(tag: &str) -> Value {
        json!({"kind": "insert_symbol", "type": "VALVE", "tag": tag, "position": [0, 0]})
    }

    /// What `reactions` say to `conn`'s session alone.
    fn said_to(reactions: &[Reaction], conn: &Connection) -> Vec<ServerMessage> {
        let own = &conn.session.as_ref().expect("session").session_id;
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::SendTo {
                    session_id,
                    message,
                } if session_id == own => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    /// `(op_id, revision)` of every `op_applied` the room was told since
    /// `room` subscribed.
    fn heard_in(room: &mut crate::bus::Subscription) -> Vec<(String, i64)> {
        std::iter::from_fn(|| room.try_recv())
            .filter_map(|message| match message {
                ServerMessage::OpApplied {
                    op_id, revision, ..
                } => Some((op_id, revision)),
                _ => None,
            })
            .collect()
    }

    async fn claim(state: &AppState, conn: &mut Connection, tag: &str) {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let claim = ClientMessage::Claim {
            session_id,
            tag: tag.into(),
        };
        let reactions = handle_message(state, conn, claim, T0).await;
        assert!(
            reactions.iter().any(|reaction| matches!(
                reaction,
                Reaction::Send(ServerMessage::ClaimGranted { .. })
            )),
            "{reactions:?}"
        );
    }

    async fn release(state: &AppState, conn: &mut Connection, tag: &str) {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let release = ClientMessage::Release {
            session_id,
            tag: tag.into(),
        };
        handle_message(state, conn, release, T0).await;
    }

    /// `OFF-4`: the same op twice — a copy that sent it again, not knowing
    /// the first had got through. It takes effect once: one valve, one
    /// revision, one row of the op log, one `op_applied` for the room. Its
    /// sender alone is told again what came of it: the revision of that row
    /// and the changes it went out with.
    #[tokio::test]
    async fn duplicate_op_a_valve_sent_twice_is_placed_once() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let _s2 = join(&state, "u2").await;
        let mut room = state.bus.subscribe(DRAWING);

        let first = submit(&state, &mut s1, 0, "op-r-1-1", valve("V-1001")).await;
        let (_, revision, changes) = applied(&first).expect("placed");
        assert_eq!(revision, 1);
        let again = submit(&state, &mut s1, 0, "op-r-1-1", valve("V-1001")).await;

        assert_eq!(objects(&state, DRAWING, None).0, 1, "one valve");
        assert_eq!(crate::ws::current_revision(&state, DRAWING), 1);
        let log: Vec<(String, i64)> = op_log(&state, DRAWING)
            .into_iter()
            .map(|op| (op.op_id, op.revision))
            .collect();
        assert_eq!(log, [("op-r-1-1".to_owned(), 1)]);
        assert_eq!(
            heard_in(&mut room),
            [("op-r-1-1".to_owned(), 1)],
            "the room hears it once"
        );
        let session_id = s1.session.as_ref().expect("session").session_id.clone();
        assert_eq!(
            again,
            [Reaction::SendTo {
                session_id,
                message: ServerMessage::OpApplied {
                    op_id: "op-r-1-1".into(),
                    revision: 1,
                    changes,
                },
            }],
            "its sender alone, told what it was told the first time"
        );
    }

    /// `OFF-4`: one queue in two copies — a file saved with steps still to
    /// send, opened in two places — and both copies send it, one after the
    /// other. Each step takes effect once, at the revision the first copy
    /// got it in at; the second copy is told each step's revision for an
    /// answer, and is refused nothing.
    #[tokio::test]
    async fn duplicate_op_one_queue_sent_from_two_copies_takes_effect_once() {
        let state = state();
        let mut first = join(&state, "u1").await;
        let mut second = join(&state, "u1").await;
        let queue = [
            ("op-r-1-1", set("设计压力", "1.6MPa")),
            ("op-r-1-2", set("设计压力", "2.0MPa")),
            ("op-r-1-3", valve("V-1001")),
        ];
        for (op_id, payload) in &queue {
            let reactions = submit(&state, &mut first, 0, op_id, payload.clone()).await;
            assert!(applied(&reactions).is_some(), "{op_id}: {reactions:?}");
        }

        let mut told = Vec::new();
        for (op_id, payload) in &queue {
            let reactions = submit(&state, &mut second, 0, op_id, payload.clone()).await;
            assert!(errors(&reactions).is_empty(), "{op_id}: {reactions:?}");
            for message in said_to(&reactions, &second) {
                if let ServerMessage::OpApplied {
                    op_id, revision, ..
                } = message
                {
                    told.push((op_id, revision));
                }
            }
        }
        let expected: Vec<(String, i64)> = vec![
            ("op-r-1-1".into(), 1),
            ("op-r-1-2".into(), 2),
            ("op-r-1-3".into(), 3),
        ];
        assert_eq!(told, expected);
        let log: Vec<(String, i64)> = op_log(&state, DRAWING)
            .into_iter()
            .map(|op| (op.op_id, op.revision))
            .collect();
        assert_eq!(log, expected);
        assert_eq!(crate::ws::current_revision(&state, DRAWING), 3);
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("2.0MPa"));
    }

    /// `OFF-4`: only what the op log has is answered from it. An op refused
    /// the first time — somebody held the tag — is not in the log, and sent
    /// again it is judged again: the tag is free now, and it takes effect.
    #[tokio::test]
    async fn duplicate_op_one_refused_before_is_judged_again() {
        let state = state();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        claim(&state, &mut s2, "P-1001").await;
        let refused = submit(&state, &mut s1, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        assert_eq!(errors(&refused)[0].0, codes::CLAIM_HELD, "{refused:?}");
        assert!(op_log(&state, DRAWING).is_empty());

        release(&state, &mut s2, "P-1001").await;
        let again = submit(&state, &mut s1, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        let (_, revision, _) = applied(&again).expect("judged, and taken");
        assert_eq!(revision, 1);
        assert_eq!(value_of(&state, "设计压力").as_deref(), Some("1.6MPa"));
    }

    /// `OFF-4`: an op that went into a conflict entry, sent again while the
    /// entry waits for its ruling, is answered with the entry as it was told
    /// the first time — not judged again, not held, no second entry.
    #[tokio::test]
    async fn duplicate_op_one_in_a_conflict_entry_is_answered_with_the_entry() {
        let state = windowed();
        let mut s1 = join(&state, "u1").await;
        let mut s2 = join(&state, "u2").await;
        submit(&state, &mut s1, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        let tied = submit(&state, &mut s2, 0, "op-r-2-1", set("设计压力", "2.5MPa")).await;
        let entry = conflicts_published(&tied).pop().expect("a conflict entry");

        let again = submit(&state, &mut s1, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        assert_eq!(said_to(&again, &s1), [entry], "{again:?}");
        let later = flush_due(&state, Instant::now() + Duration::from_secs(5));
        assert!(later.is_empty(), "nothing held: {later:?}");
        assert_eq!(conflict_rows(&state).len(), 1);
        assert_eq!(value_of(&state, "设计压力"), None);
        assert!(op_log(&state, DRAWING).is_empty());
    }

    /// `OFF-4`: the op log outlives the process. A server started again on
    /// the same database answers an op it took before from the log.
    #[tokio::test]
    async fn duplicate_op_one_taken_before_a_restart_is_answered_after_it() {
        let dir = std::env::temp_dir().join("ocs_collab_ops_tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("restart-{}.db", uuid::Uuid::new_v4()));
        let started = || {
            let store = Store::open(&path).expect("store");
            AppState::new(
                Some(store),
                None,
                path.display().to_string(),
                SessionConfig::default(),
            )
        };

        let before = started();
        let mut s1 = join(&before, "u1").await;
        let first = submit(&before, &mut s1, 0, "op-r-1-1", valve("V-1001")).await;
        let (_, _, changes) = applied(&first).expect("placed");
        drop(s1);
        drop(before);

        let after = started();
        let mut s2 = join(&after, "u1").await;
        let again = submit(&after, &mut s2, 0, "op-r-1-1", valve("V-1001")).await;
        assert_eq!(
            said_to(&again, &s2),
            [ServerMessage::OpApplied {
                op_id: "op-r-1-1".into(),
                revision: 1,
                changes,
            }],
            "{again:?}"
        );
        assert_eq!(op_log(&after, DRAWING).len(), 1);
        assert_eq!(objects(&after, DRAWING, None).0, 1);
        drop(after);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// `OFF-4`: two copies send one op inside the tie window. The second
    /// waits behind the first and ties with nothing; when the window has
    /// passed the first takes effect, and the second is answered with it.
    #[tokio::test]
    async fn duplicate_op_sent_again_inside_the_window_is_answered_with_the_first() {
        let state = windowed();
        let mut first = join(&state, "u1").await;
        let mut second = join(&state, "u1").await;
        let held = submit(&state, &mut first, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        assert!(held.is_empty(), "{held:?}");
        let again = submit(
            &state,
            &mut second,
            0,
            "op-r-1-1",
            set("设计压力", "1.6MPa"),
        )
        .await;
        assert!(again.is_empty(), "{again:?}");

        let reactions: Vec<Reaction> = flush_due(&state, Instant::now() + Duration::from_secs(1))
            .into_iter()
            .flat_map(|(_, outcome)| outcome.reactions)
            .collect();
        assert!(errors(&reactions).is_empty(), "{reactions:?}");
        let (_, revision, changes) = applied(&reactions).expect("the first takes effect");
        assert_eq!(revision, 1);
        assert_eq!(
            said_to(&reactions, &second),
            [ServerMessage::OpApplied {
                op_id: "op-r-1-1".into(),
                revision: 1,
                changes,
            }],
            "{reactions:?}"
        );
        assert_eq!(op_log(&state, DRAWING).len(), 1);
        assert!(conflict_rows(&state).is_empty());
    }

    /// `OFF-4`: an op taken before is answered before anything else is asked
    /// of it. Since it took effect somebody else has taken the tag it
    /// wrote; a copy that sends it again is told it took effect, not refused
    /// — a refusal would have that copy take back a step the room has.
    #[tokio::test]
    async fn duplicate_op_a_tag_taken_since_does_not_refuse_an_op_taken_before() {
        let state = state();
        let mut first = join(&state, "u1").await;
        let mut second = join(&state, "u1").await;
        let mut other = join(&state, "u2").await;
        let taken = submit(&state, &mut first, 0, "op-r-1-1", set("设计压力", "1.6MPa")).await;
        let (_, _, changes) = applied(&taken).expect("taken");
        claim(&state, &mut other, "P-1001").await;

        let again = submit(
            &state,
            &mut second,
            0,
            "op-r-1-1",
            set("设计压力", "1.6MPa"),
        )
        .await;
        assert!(errors(&again).is_empty(), "{again:?}");
        assert_eq!(
            said_to(&again, &second),
            [ServerMessage::OpApplied {
                op_id: "op-r-1-1".into(),
                revision: 1,
                changes,
            }]
        );
    }
}

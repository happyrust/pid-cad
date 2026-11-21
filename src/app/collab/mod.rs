//! Desktop collaboration client: the state the GUI shows, the message stream
//! that feeds it, and the synchronous WebSocket transport underneath.
//!
//! The transport is deliberately synchronous (`tungstenite` + `std::thread` +
//! an iced futures channel): the desktop build must stay free of an async
//! runtime (see `architecture.md` §5.2 and `VAL-CLI-019`). Everything the GUI
//! needs to know arrives as a [`CollabEvent`] on the subscription channel, so
//! no collab code ever touches the socket from the UI thread.
//!
//! This module owns *state* only, plus the outbound request queue that lets the
//! UI ask the transport thread to send `claim` / `release` / `op` frames. The
//! semantic half of a broadcast — turning a local edit into an `op` payload and
//! a peer's payload back into a local edit — lives in [`broadcast`], as pure
//! functions over the document so both directions are testable without a
//! socket.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::pid::collab_state::Diverged;

// The claim commands and the panel glue are shared by both builds (the panel
// exists on the web build as a hint); only the transport itself is desktop-only,
// so `request_claim` / `request_release` carry a web stub below.
pub(super) mod broadcast;
mod apply;
mod claims;
pub(super) mod connectors;
mod leaving;
pub(super) mod pipes;
mod unsynced;
#[cfg(not(target_arch = "wasm32"))]
mod messages;
#[cfg(not(target_arch = "wasm32"))]
mod transport;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_align_refused;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_connector_cross_drawing;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_diverged_persist;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_duplicate_op;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_existing_tags;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_exit_prompt;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_inverse;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_large_props;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_offline_queue;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_op_ids;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_pipe_ops;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_restored_rollback;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_save_exits;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod collab_unsynced;

pub(crate) use leaving::{Leaving, UnsyncedQuestion};

/// A server address as it may be shown — in the log, on the command line,
/// in `/collab/status`: without whatever stands before an `@` in its
/// authority. A user name and a password in a URL are credentials wherever
/// the URL goes, and those three places are read by people and kept in
/// files. The protocol has no use for them (a session is who `hello` says
/// it is), so nothing is lost by not showing them.
pub(super) fn shown_address(address: &str) -> String {
    let Some((scheme, rest)) = address.split_once("://") else {
        return address.to_owned();
    };
    let authority = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..authority].rfind('@') {
        Some(at) => format!("{scheme}://***@{}", &rest[at + 1..]),
        None => address.to_owned(),
    }
}

/// One participant on the drawing, as the server reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    pub session_id: String,
    pub user_id: String,
    pub display_name: String,
}

/// What the transport thread reports to the UI thread.
///
/// One variant per server message that changes visible state, plus the three
/// connection-lifecycle variants the UI needs to draw "connecting", "offline"
/// and "connected" honestly, and the one question the transport puts to the
/// UI ([`CollabEvent::TagsWanted`]).
#[derive(Clone, Debug, PartialEq)]
pub enum CollabEvent {
    /// A connection attempt is about to start; `attempt` counts from zero for
    /// the first try after a successful session.
    Connecting { attempt: u32 },
    /// `welcome` arrived: the session exists and this is its baseline.
    Connected {
        session_id: String,
        base_revision: i64,
        peers: Vec<PeerInfo>,
        claims: Vec<(String, String)>,
        /// `welcome.repo_id`: which repository this room is of, saved with
        /// the drawing file so the next open can prove it is coming back
        /// to the same one (`REQ-COLL-015`).
        repo_id: String,
    },
    /// `welcome.align_refused`, said ahead of the `Connected` it comes with:
    /// the server opened the session but cannot take this copy as
    /// consistent — wrong repository, revision ahead, or the log cannot
    /// replay what it missed (`REQ-COLL-015`). Nothing of the queue goes out
    /// on that connection, and nothing the room does comes into the drawing.
    AlignRefused {
        code: String,
        message: String,
    },
    /// The socket ended. `reason` names the branch that ended it (`read error:
    /// ConnectionReset` vs `close frame`), which is what `VAL-CLI-020` reads.
    Disconnected { reason: String, will_retry: bool },
    PeerJoined(PeerInfo),
    PeerLeft { session_id: String },
    ClaimGranted { tag: String },
    ClaimDenied {
        tag: String,
        holder: String,
        expires_at: i64,
    },
    ClaimReleased { tag: String, reason: String },
    ReadonlyDowngrade {
        reason: String,
        holder: Option<String>,
        /// The tier of the permission chain that refused (`role` / `pbs` /
        /// `state`) and its `PERM_DENIED_*` code, when the server said.
        level: Option<String>,
        code: Option<String>,
    },
    /// `op_applied`: the baseline moved forward, and — when somebody else made
    /// the change — here is what changed, so the local document can follow
    /// (`VAL-CLI-009`). An op this client sent comes back too (the room
    /// broadcast includes the originator), which is why `op_id` travels with
    /// it: the app recognises its own id and only moves the baseline.
    OpApplied {
        op_id: String,
        revision: i64,
        changes: Vec<Value>,
    },
    /// `conflict`: two people wrote one field differently from one baseline,
    /// neither write took effect, and the entry waits for a ruling
    /// (`VAL-CROSS-007`). The whole room is told, the two who wrote and
    /// everybody else alike.
    Conflict(PendingConflict),
    ServerError { code: String, message: String },
    /// `welcome.replay_from`, said ahead of the [`CollabEvent::Connected`]
    /// it belongs to: the server replays every op after revision `from`,
    /// so the session is taken at the revision its drawing has, and what it
    /// missed arrives as `op_applied` like any other (`VAL-CROSS-008`).
    Resuming { from: i64 },
    /// When each claim a `welcome` listed runs out, in Unix seconds — said
    /// behind the `Connected` that lists who holds them.
    ClaimsExpire(Vec<(String, i64)>),
    /// An `error` that answers an op of this session's by its id.
    /// `rejected`: nothing of the op took effect, and the step it spoke for
    /// is taken back. Otherwise it is about one change of an op whose other
    /// changes come as `op_applied`.
    OpRefused {
        op_id: String,
        code: String,
        message: String,
        rejected: bool,
    },
    /// The ops a `conflict` says tied: neither is answered any other way.
    OpsInConflict { op_ids: Vec<String> },
    /// Not from the server: the transport is about to say `hello`, and asks
    /// which tags the session's drawing carries, to say them with it
    /// (`hello.existing_tags`). Asked through here, behind every change
    /// already handed over, so that the answer is of the drawing as those
    /// changes left it. `ask` numbers the question, and the answer names it
    /// (`State::say_tags`).
    TagsWanted { ask: u64 },
}

/// One op this session has made that the room has not answered yet.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub op_id: String,
    pub payload: Value,
    /// The revision the step was made against. It goes out with the op —
    /// not the revision the room is at when it is finally sent — so that a
    /// step made while the session was away is judged as what it is: made
    /// without having seen what the room did meanwhile.
    pub base_revision: i64,
    /// The undo steps it speaks for (`DeltaSnapshot::collab_step`): how the
    /// step is found again if the op is refused.
    pub steps: Vec<u64>,
    /// Whether it put those steps on the drawing (a step made, or put on
    /// again) or took them off (a step taken back).
    pub on: bool,
    /// Whether it is on the wire of the session that is up now.
    pub sent: bool,
    /// What the step is called, for the message that says it was refused.
    pub label: String,
    /// Whether it was read back from the drawing file it was saved into
    /// (`PID_AUTHOR_COLLAB_QUEUE`). A restored op's undo steps died with
    /// the process that made them — `steps` is empty — so a refusal takes
    /// it back by its `inverse` instead, and marks the drawing diverged
    /// when it has none rather than pretend it could (`REQ-COLL-015`;
    /// `SPEC-collab-offline-sync.md` §5.4).
    pub restored: bool,
    /// Who queued it: the user id in effect when the step was made, or what
    /// the file said of a restored op — nothing, for one a version from
    /// before saved. Only the person who queued an op sends it
    /// (`State::judge_queue`).
    pub user_id: Option<String>,
    /// When it was queued, in Unix seconds; for a restored op, what the file
    /// said.
    pub queued_at: Option<i64>,
    /// What takes its steps back out of the drawing again — for an op that
    /// put them on, what undoing them tells the room; for one that took them
    /// off, what putting them on again does — in the order they are to be
    /// applied (`SPEC-collab-offline-sync.md` §4.1). It goes into the file
    /// with the op, so that a refusal heard after the process that made the
    /// steps is gone can still take them back (§5.4). Read while the editor
    /// has nothing else on hand (`OpenCADStudio::read_queued_inverses`);
    /// `None` until then, and for an op it could not be read for. Never sent.
    pub inverse: Option<Vec<Value>>,
    /// Whether `inverse` has been looked for: an op it could not be read for
    /// is not tried again.
    pub inverse_read: bool,
}

/// An op of this session's the server refused whole, for the app to take
/// the step back by.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejection {
    pub entry: Outgoing,
    pub code: String,
    pub message: String,
}

/// What the ops queued behind a refused op come to, for taking its step
/// back (`OpenCADStudio::settle_rejected_ops`).
///
/// The room has the refused op's steps as they were before it. Of what
/// this person did afterwards — none of it sent, one op being on the wire
/// at a time — some was done to those steps or on top of them, and the
/// room has nothing of that to change; the rest was done when they were
/// out of the drawing again, and is this person's own business as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refused {
    /// The steps that have to move back, because in this drawing they are
    /// still the way the room refused: the refused op's own, unless this
    /// person has moved them back already, and those made on top of them.
    pub in_effect: Vec<u64>,
    /// The queued ops that are not to be sent, by id.
    pub void: Vec<String>,
    /// An op behind the refused one moved refused steps and steps the room
    /// has in one go — an undo of several steps across the refused one. It
    /// cannot be said in part, and saying it whole would tell the room to
    /// take back what it never had.
    pub mixed: bool,
}

impl Refused {
    /// Go through `queue` — what waits behind `refused`, oldest first — and
    /// say which of it falls with the refused op.
    ///
    /// The refused op moved its steps one way: on to the drawing, or off
    /// it. While any of them is still that way, whatever else this person
    /// moved the same way was done on top of them — a new step made on a
    /// drawing that had the refused one, an older step taken back after the
    /// refused undo — and falls with them. An op that only moves steps
    /// already fallen (the refused step taken back by hand, put on again)
    /// falls too, and leaves them the way it put them. Anything else stands.
    pub fn account(refused: &Outgoing, queue: &std::collections::VecDeque<Outgoing>) -> Self {
        use std::collections::BTreeSet;
        let way = refused.on;
        let mut fallen: BTreeSet<u64> = refused.steps.iter().copied().collect();
        let mut in_effect = fallen.clone();
        let mut out = Self::default();
        for queued in queue {
            if queued.sent || queued.steps.is_empty() {
                continue;
            }
            let all = queued.steps.iter().all(|step| fallen.contains(step));
            let any = queued.steps.iter().any(|step| fallen.contains(step));
            if all {
                for step in &queued.steps {
                    if queued.on == way {
                        in_effect.insert(*step);
                    } else {
                        in_effect.remove(step);
                    }
                }
                out.void.push(queued.op_id.clone());
            } else if queued.on == way && !in_effect.is_empty() {
                fallen.extend(queued.steps.iter().copied());
                in_effect.extend(queued.steps.iter().copied());
                out.void.push(queued.op_id.clone());
            } else if any {
                out.mixed = true;
                break;
            }
        }
        out.in_effect = in_effect.into_iter().collect();
        out
    }

    /// What waits in `queue` behind `refused`, an op restored from the
    /// drawing's file, and is built on it — by id, in the order queued
    /// (`SPEC-collab-offline-sync.md` §5.4). Such an op names, by tag, what
    /// the refused op made, renamed or erased, or the line or run it was
    /// about (`broadcast::Change::built_names`) — or what an op built on it
    /// did, which is built on it too. A restored op has no undo steps to
    /// go by (`account`), so the names are all there is.
    pub fn built_on(
        refused: &Outgoing,
        queue: &std::collections::VecDeque<Outgoing>,
    ) -> Vec<String> {
        use std::collections::BTreeSet;
        let said = |op: &Outgoing| {
            broadcast::changes_from_payload(std::slice::from_ref(&op.payload)).changes
        };
        let mut names: BTreeSet<String> = said(refused)
            .iter()
            .flat_map(broadcast::Change::built_names)
            .collect();
        let mut along = Vec::new();
        if names.is_empty() {
            return along;
        }
        for queued in queue.iter().filter(|queued| !queued.sent) {
            let changes = said(queued);
            if changes
                .iter()
                .any(|change| change.names().iter().any(|name| names.contains(name)))
            {
                names.extend(changes.iter().flat_map(broadcast::Change::built_names));
                along.push(queued.op_id.clone());
            }
        }
        along
    }
}

/// Where an op stands right after it was queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queued {
    /// On the wire.
    Sent,
    /// Behind this many ops the room has yet to answer.
    Behind(usize),
    /// No session is up; this many ops wait for the next one.
    Offline(usize),
    /// The session is up but not aligned with the room; this many ops wait
    /// for a connection that is.
    NotAligned(usize),
}

/// Why this connection is not taken as aligned with the room: the
/// `welcome` refused it, the room answered an op `NOT_ALIGNED`, or the room
/// moved on with no replay. Judged afresh on every connection.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct NotAligned {
    pub code: String,
    pub message: String,
}

impl NotAligned {
    /// What the command line says of it.
    pub(super) fn said(&self) -> String {
        format!(
            "协同：这次连接与房间对不上（{}：{}），本地修改照常排队、不提交；需要取得最新图纸",
            self.code, self.message
        )
    }

    /// The reason, as the panel shows it.
    fn reason(&self) -> String {
        format!("{}（{}）", self.message, self.code)
    }
}

/// What a copy holds itself apart for before its `hello`
/// (`State::judge_queue`). The client's own codes: no server sends them.
///
/// Steps somebody else queued, or that say nobody.
const QUEUED_BY_OTHERS: &str = "QUEUED_BY_OTHERS";
/// Two steps under one name.
const QUEUE_DUPLICATE_ID: &str = "QUEUE_DUPLICATE_ID";
/// Entries of the queue that could not be read.
const QUEUE_UNREADABLE: &str = "QUEUE_UNREADABLE";
/// A drawing with a sync record read in after the session came up without
/// it: that session never held the record against the room.
const OPENED_AFTER_SESSION: &str = "OPENED_AFTER_SESSION";

/// Whether `code`, on an `error` that names no op, can only be the refusal
/// of an op. A server from before `error.op_id` answers a refused op this
/// way; with one op on the wire at a time it can only mean that one.
fn refuses_an_op(code: &str) -> bool {
    use ocs_collab::protocol::codes;
    [
        codes::STALE_BASE_REVISION,
        codes::OP_SUPERSEDED,
        codes::OBJECT_DELETED,
        codes::OBJECT_REFERENCED,
        codes::CLAIM_HELD,
        codes::PERM_DENIED_ROLE,
        codes::PERM_DENIED_PBS,
        codes::PERM_DENIED_STATE,
    ]
    .contains(&code)
}

/// A conflict entry awaiting a ruling, as the server announced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingConflict {
    pub conflict_id: String,
    pub object_ref: String,
    pub field: String,
    pub value_a: String,
    pub value_b: String,
}

/// The prefix of the `op_id` a ruling is applied under: the rest of the id
/// is the conflict entry it rules on (`ocs_collab_server`'s `resolve`).
const RULING_OP_PREFIX: &str = "resolve-";

/// What this session has to do to its own drawing when an op of its own
/// comes back: a retag for each symbol it placed that the server took under
/// another tag (`reassigned_from` says the tag it was sent with). Each names
/// the entity the symbol was placed as, which is how it is told from a
/// peer's symbol that got the tag first.
fn reassignments(changes: &[Value]) -> Vec<Value> {
    let nested = changes
        .iter()
        .filter_map(|change| change.get("changes").and_then(Value::as_array))
        .flatten();
    changes
        .iter()
        .chain(nested)
        .filter(|change| change["kind"] == broadcast::KIND_INSERT_SYMBOL)
        .filter_map(|change| {
            let from = change["reassigned_from"].as_str()?;
            let to = change["object_ref"].as_str().or(change["tag"].as_str())?;
            (from != to).then(|| {
                json!({
                    "kind": broadcast::KIND_REASSIGN_TAG,
                    "object_ref": from,
                    "field": "tag",
                    "value": to,
                    "handle": change["handle"],
                })
            })
        })
        .collect()
}

/// Everything the transport thread needs to open a session. Copied out of the
/// app state before the thread starts, so the thread never reads app memory.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ClientConfig {
    pub server_url: String,
    pub user_id: String,
    pub drawing_id: String,
    pub mode: String,
    pub client_version: String,
}

/// What the drawing file was saved aligned to (`REQ-COLL-015`): the
/// repository its collaboration records belong to and the revision it had
/// applied when it was last saved. A file with no record has none — a
/// drawing that was never in a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileClaim {
    pub drawing_id: String,
    pub repo_id: String,
    pub revision: i64,
}

/// What the UI answers when the transport asks which tags the session's
/// drawing carries ([`CollabEvent::TagsWanted`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TagsSaid {
    /// The drawing is there, and these are its tags — none, for a drawing
    /// that carries none or one that has been closed. The second slot is
    /// what the file was saved aligned to, for the `hello` to claim.
    Tags(Vec<String>, Option<FileClaim>),
    /// The drawing is there, and its queue is not this person's to send
    /// (`State::judge_queue`): the `hello` claims nothing — neither the
    /// file's word nor where the last session left the drawing — and lists
    /// no tags, and the connection is held apart, for this reason, whatever
    /// the `welcome` says.
    Apart(NotAligned),
    /// The drawing is there, these are its tags, and it has been taken off
    /// the room it was in (`PIDCOLLAB DETACH`) since a session was last up:
    /// the `hello` claims nothing — not where the last session left the
    /// drawing either — and the connection is not held apart.
    Detached(Vec<String>),
    /// The session has no drawing yet and none is on its way: the `hello`
    /// need not wait. The tags follow under the same number once the
    /// session has come by its drawing.
    NotYet,
}

/// A question of the transport's about the drawing's tags that the UI has
/// not answered with the tags yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TagsAsked {
    /// The number the answer goes back under.
    pub ask: u64,
    /// The transport has been told not to wait ([`TagsSaid::NotYet`]).
    pub put_off: bool,
    /// What the tabs were — which they are, and the file each shows — when
    /// the one showing the session's file was last looked for and not found
    /// (`OpenCADStudio::collab_tabs_print`), so that it is looked for again
    /// only once they have changed: finding a file's tab asks the file
    /// system, and the question is gone back to after every message.
    pub looked: Option<u64>,
}

/// The GUI's view of the collaboration session.
#[derive(Clone, Debug, Default)]
pub(super) struct State {
    /// Server URL in effect (CLI wins over the saved config). Empty means the
    /// user never configured collaboration, and nothing connects at all.
    pub server_url: String,
    /// Server URL as persisted in `settings.json`. `--collab` overrides
    /// `server_url` for this run only; persisting the override would make a
    /// later launch without the flag connect to whatever the last scripted
    /// run used.
    pub configured_server_url: String,
    pub user_id: String,
    /// User id as persisted in `settings.json` (same reasoning as above).
    pub configured_user_id: String,
    pub drawing_id: String,
    /// Drawing id as persisted in `settings.json`.
    pub configured_drawing_id: String,
    pub mode: String,
    /// Mode as persisted in `settings.json`.
    pub configured_mode: String,
    pub connected: bool,
    pub session_id: Option<String>,
    pub base_revision: i64,
    pub peers: Vec<PeerInfo>,
    /// tag → holder user id, exactly the shape `GET /api/v1/collab/status`
    /// publishes (see `architecture.md` A.5).
    pub claims: BTreeMap<String, String>,
    /// This session may not write to its drawing. Set by the server
    /// (`readonly_downgrade`), kept while the connection is down — a session
    /// that may not write does not come to write by losing its socket — and
    /// decided afresh by the server each time a session comes up.
    pub readonly: bool,
    pub readonly_holder: Option<String>,
    /// Which tier of the permission chain refused (`role` / `pbs` /
    /// `state`), when the server said; a refused write is answered with it.
    pub readonly_level: Option<String>,
    /// The `PERM_DENIED_*` code of that tier.
    pub readonly_code: Option<String>,
    /// Last transport-level failure, for the command line and diagnostics.
    pub last_error: Option<String>,
    /// How many reconnect attempts have failed since the last live session.
    pub reconnect_attempts: u32,
    /// Last claim/release outcome worth telling the user: a `claim_denied`
    /// names who holds the tag and until when, and the panel shows it.
    pub last_claim_message: Option<String>,
    /// `op_id`s this session submitted, most recent last. An `op_applied`
    /// carrying one of them is our own change coming back from the room
    /// broadcast, so it must not be applied to the local document a second
    /// time. Bounded: only the ids still in flight matter.
    pub own_ops: std::collections::VecDeque<String>,
    /// How many remote operations have been applied to the local document.
    /// `VAL-CLI-009` reads this through `/collab/status` to tell "the broadcast
    /// arrived" from "the broadcast was folded in".
    pub applied_ops: u64,
    /// Set when a remote change could not be applied (no drawing open, or a
    /// change shape this client does not understand). The panel reports it
    /// rather than silently diverging.
    pub last_apply_error: Option<String>,
    /// Remote operations the UI thread still has to fold into the open
    /// document. `apply()` cannot do it: it holds no document. The app drains
    /// this right after every event (`OpenCADStudio::apply_remote_changes`).
    pub pending_remote: Vec<RemoteChange>,
    /// The tab holding the drawing this session speaks for, by document id.
    ///
    /// A session is about one drawing (`--drawing`); an editor has many tabs.
    /// A peer's change goes onto this tab and nowhere else, and only this
    /// tab's edits go out — otherwise a change meant for one sheet lands on
    /// whichever sheet is in front, which may carry the same tags. `None`
    /// until the session first needs a document; see
    /// `OpenCADStudio::collab_tab`.
    pub document: Option<u64>,
    /// The file the editor was started on, when it was started on one with a
    /// session configured: the session is that file's. While `document` is
    /// unset the session waits for the tab showing this file instead of
    /// taking the drawing in front.
    pub document_path: Option<std::path::PathBuf>,
    /// `drawing_id` was given for this run — `--drawing`, or `PIDCOLLAB
    /// DRAWING` with no session up — rather than read from the settings or
    /// taken from the file's name. A drawing whose record names another
    /// drawing is then not the session's; otherwise the session takes the
    /// record's (`SPEC-collab-offline-sync.md` §5.2, step one).
    pub drawing_given: bool,
    /// The tab whose drawing's record names another drawing than this
    /// session's, by document id, once it has been turned down for it.
    pub foreign_tab: Option<u64>,
    /// The tab whose drawing's record has been held against `drawing_id`,
    /// by document id (`OpenCADStudio::record_allows`).
    record_read: Option<u64>,
    /// The drawing's record has been read into the session
    /// (`seed_file_state`).
    seeded: bool,
    /// Why the drawing is held apart for having been read into the session
    /// after one was up: that session never judged it, and the room it
    /// joined does not know where the drawing is (`seed_file_state`).
    opened_late: Option<NotAligned>,
    /// Edits that changed what a drawing with a sync record says of its
    /// objects and that no queue took, by the tab's document id, as entries
    /// of the unsynced list (`collab_state::Unsynced`). The room will never
    /// see them, so a save puts them on the drawing's record and marks it
    /// `UNQUEUED_EDITS` (`SPEC-collab-offline-sync.md` §5.1).
    pub unqueued: BTreeMap<u64, Vec<Value>>,
    /// Ops restored from a drawing's file that the room refused, or that
    /// went with one it refused, by the tab's document id, as `refused`
    /// entries of the unsynced list. What they did is out of the drawing
    /// again, or never was, and there is no redo to bring it back: the
    /// entry is what is left of it. A save puts them on the drawing's
    /// record; they do not mark it (§5.4).
    pub refused: BTreeMap<u64, Vec<Value>>,
    /// The `UNQUEUED_EDITS` mark of a drawing whose unsynced list was
    /// cleared (`PIDCOLLAB UNSYNCED CLEAR`), by the tab's document id. The
    /// entries go; that edits were made which no queue took stays true, and
    /// a save keeps saying so until the drawing is taken off its room
    /// (§4.5).
    pub unqueued_marks: BTreeMap<u64, Diverged>,
    /// Why this drawing can no longer be taken to be what the room has: a
    /// revision that never arrived, or a peer's change that could not be
    /// applied here. After a peer's change that could not be applied the
    /// baseline still reads as current — it is the server's number, not a
    /// statement about this drawing — so this is what keeps the session from
    /// writing on top of the difference. `None` while nothing is known to be
    /// missing.
    pub diverged: Option<String>,
    /// The code `diverged` was marked with, and when, in Unix seconds: with
    /// it, what the drawing's record is saved with (`diverged_record`).
    pub diverged_as: Option<(&'static str, i64)>,
    /// A revision that skipped one has come: from then on the baseline
    /// stays the last revision this drawing has all of, which is what its
    /// record is saved with.
    revision_gap: bool,
    /// What the drawing's record said of its divergence when the session
    /// read it (`collab_state::load_diverged`). A drawing that knows it is
    /// apart is held apart on every connection and says nothing of where it
    /// is (`judge_queue`); a save keeps the mark.
    pub saved_diverged: Option<Diverged>,
    /// Why the connection that is up is not taken as aligned with the room,
    /// while it is not. Unlike `diverged` it is of the connection, not of
    /// the drawing: nothing of the outbox goes out and nothing the room does
    /// comes in, the baseline and the repository stay what the drawing
    /// said, and the next connection is judged afresh.
    pub not_aligned: Option<NotAligned>,
    /// Whether a session has been up before in this run, so that
    /// `base_revision` is a revision this drawing was actually at. One held
    /// apart from the room does not count.
    pub had_session: bool,
    /// The session's drawing was taken off the room it was in (`PIDCOLLAB
    /// DETACH`) since a session was last up: the next `hello` claims nothing
    /// of where the drawing is, not even where the last session left it
    /// ([`TagsSaid::Detached`]).
    pub detached: bool,
    /// Which repository the room this drawing belongs to is of — the
    /// `repo_id` the last `welcome` that took the drawing as aligned named,
    /// or the one the drawing file was saved with when it is a copy that
    /// was aligned before (`REQ-COLL-015`). Written into the file on save;
    /// `None` for a drawing no repository has ever claimed.
    pub repo_id: Option<String>,
    /// The conflict entries the room was told of and that no ruling has
    /// arrived for, oldest first. While one stands, the field it names has
    /// the value it had before either write — except in the drawing of
    /// whoever made one of them, which keeps what its owner wrote until the
    /// ruling says otherwise.
    ///
    /// Kept across a reconnect: the entry is still waiting on the server,
    /// and a `welcome` does not list them.
    pub conflicts: Vec<PendingConflict>,
    /// The peers' changes this drawing has taken, in the order it took them,
    /// each with the entities it was written to.
    ///
    /// A peer's change is not a step on this person's undo stack — undo
    /// takes back what they did themselves. But a local step keeps images of
    /// the entities it touched, from the moment it was made, and taking it
    /// back puts those images in the drawing: whatever a peer did to the
    /// same entities since is not in them. So after an undo or a redo the
    /// changes that came later than the step are written again, from here
    /// (`OpenCADStudio::replay_remote_after`).
    pub remote_log: Vec<AppliedRemote>,
    /// How many changes `remote_log` has ever held: the `seq` of the newest.
    pub remote_seq: u64,
    /// The ops this session has made and the room has not answered, oldest
    /// first. One is on the wire at a time: the first goes out, and the
    /// next when that one has been answered — applied, refused, or held in
    /// a conflict entry. A step made with no session up waits here too, and
    /// goes out when a session is up again and has caught up
    /// (`VAL-CROSS-008`).
    ///
    /// One at a time because a refused step is taken back, and steps are
    /// taken back from the top of the undo stack: with one op on the wire,
    /// whatever this person did after the refused step has not been sent
    /// yet, and is taken back with it without the room having to be told
    /// anything.
    pub outbox: std::collections::VecDeque<Outgoing>,
    /// The revision a replay is to reach, while the server is sending what
    /// this session missed. Nothing of the outbox goes out before then.
    pub catching_up: Option<i64>,
    /// `welcome.replay_from`, between the event that says it and the
    /// `Connected` it belongs to.
    resume_from: Option<i64>,
    /// `welcome.align_refused`, the same way.
    align_refused: Option<NotAligned>,
    /// Ops the server refused whole, for the app to take their steps back
    /// (`OpenCADStudio::settle_rejected_ops`).
    pub rejections: Vec<Rejection>,
    /// The last few refusals as they were dealt with, for the status.
    pub rejection_log: Vec<Value>,
    /// What the server said of an op that takes no step back — one change
    /// of it gave way and the rest took effect — as `(code, message)`, for
    /// the app to say on the command line.
    pub refusals_to_say: Vec<(String, String)>,
    /// Raised between a refusal and the app having taken the step back:
    /// the ops behind it may speak for steps that go with it.
    hold: bool,
    /// Raised while steps are being taken back (or put on again) because
    /// the server refused them: nothing is said to the room for it, and
    /// neither the read-only gate nor a claim stands in the way — the
    /// drawing is being put back to what the room has.
    pub rolling_back: bool,
    /// Held while what takes a queued op's steps back out is being read
    /// (`OpenCADStudio::read_queued_inverses`): what taking them back says
    /// to the room is kept here instead of being said.
    reading_inverse: Option<Vec<broadcast::Change>>,
    /// The last number given to an undo step (`next_step`).
    step_seq: u64,
    /// This run's name, under which it names its ops
    /// (`ocs_collab::ids::op_id`): drawn with the first op it names, so no
    /// other start of the program — this person's or anybody's — names a
    /// step alike, whether or not a session was ever up.
    run_id: String,
    /// How many ops this run has named.
    ops_named: u64,
    /// The entries of the queue the drawing file was saved with that could
    /// not be read as ops, as they were found (`collab_state::QueueRead`).
    /// A save puts them back; while there is one, no connection claims
    /// alignment (`judge_queue`).
    pub unreadable_queue: Vec<Value>,
    /// Why this copy holds itself apart, as it said in its last answer to
    /// the transport's question (`TagsSaid::Apart`): the connection that
    /// answer was for is held apart for it, whatever the `welcome` says.
    apart_before_hello: Option<NotAligned>,
    /// tag → when the claim on it runs out, in Unix seconds, for the claims
    /// the server has said it of: those standing at the last `welcome`, and
    /// one this session asked for and was refused. A claim this session was
    /// granted is not in here — `claim_granted` does not say.
    pub claim_expiry: BTreeMap<String, i64>,
    /// The tags this session has asked to claim, less those it gave back,
    /// was refused, or saw let go of: what it holds, or waits to hear of.
    /// They go with the session — the server lets go of a session's claims
    /// when it ends — unless the session is left to tell the drawing's tags
    /// (`rejoining`).
    own_claims: std::collections::BTreeSet<String>,
    /// Raised when the drawing's tags are said to a session that is up: the
    /// transport leaves it to say them in a `hello`, and dials again at once
    /// (`transport::run_session`). That leave is none of the person's
    /// doing, so what the session held by claim is asked for again in the
    /// one that dial brings up. Lowered there — or when that dial fails:
    /// from then on it is a drop like any other, and whose the tags are
    /// when a session is back is for the room to say.
    rejoining: bool,
    /// The transport's question about the drawing's tags, while the UI has
    /// yet to answer it with them (`OpenCADStudio::answer_collab_tags`).
    pub tags_wanted: Option<TagsAsked>,
    /// The launch is through with the drawings it was started on: each has
    /// been opened or given up on. A session started on a file that still
    /// has no drawing then will not get it from the launch.
    pub opens_settled: bool,
    /// The payload of every op this state queued, for tests to read: the
    /// outbound queue itself is the process's, and other tests write to it.
    #[cfg(test)]
    pub sent: Vec<Value>,
    /// Every refused write this state reported, `(object_ref, text)`: for
    /// tests, for the same reason as `sent`.
    #[cfg(test)]
    pub attempts: Vec<(String, String)>,
    /// Every answer this state gave about the drawing's tags, `(ask, said)`:
    /// for tests, for the same reason as `sent`.
    #[cfg(test)]
    pub tags_said: Vec<(u64, TagsSaid)>,
    /// Every claim this state asked the transport for, `(session, tag)`: for
    /// tests, for the same reason as `sent`.
    #[cfg(test)]
    pub claims_asked: Vec<(String, String)>,
}

/// One change of a peer's as this drawing took it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedRemote {
    /// Its place in the order they were taken, from 1.
    pub seq: u64,
    /// The entities it was written to.
    pub handles: Vec<document::Handle>,
    pub change: broadcast::Change,
}

/// One remote `op_applied` waiting to be applied to the local document.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteChange {
    pub op_id: String,
    pub changes: Vec<Value>,
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            mode: DEFAULT_MODE.to_owned(),
            ..Self::default()
        }
    }

    /// Whether a session should be (or is being) maintained. Without a server
    /// URL nothing connects — the offline-first invariant of `VAL-CLI-017`.
    pub(super) fn wanted(&self) -> bool {
        !self.server_url.trim().is_empty()
    }

    /// The `(code, level)` a write refused by the read-only gate is answered
    /// with: the tier's own `PERM_DENIED_*` code when a role or a PBS entry
    /// refused, and `COLLAB_READONLY` at the state tier — the drawing is
    /// 只读, or somebody else is its one editor — which is what the gate has
    /// always answered there (`VAL-CLI-011`).
    pub(super) fn refusal(&self) -> (String, String) {
        let level = self.readonly_level.clone().unwrap_or_else(|| "state".to_owned());
        let code = match (level.as_str(), &self.readonly_code) {
            ("state", _) | (_, None) => "COLLAB_READONLY".to_owned(),
            (_, Some(code)) => code.clone(),
        };
        (code, level)
    }

    /// Record that this drawing is no longer known to match the room, as
    /// `code` (one of `collab_state`'s). The first reason is kept: it is
    /// where the two parted.
    pub(super) fn mark_diverged(&mut self, code: &'static str, reason: String) {
        if self.diverged.is_none() {
            self.last_error = Some(format!("已与房间不一致：{reason}"));
            self.diverged = Some(reason);
            self.diverged_as = Some((code, ocs_collab::ids::now_unix()));
        }
    }

    /// The divergence the drawing's record is to be saved with: the one it
    /// was read with, else the one marked in this run.
    pub(super) fn diverged_record(&self) -> Option<Diverged> {
        self.saved_diverged.clone().or_else(|| {
            let (code, at) = self.diverged_as?;
            Some(Diverged {
                code: code.to_owned(),
                message: self.diverged.clone()?,
                at,
            })
        })
    }

    /// The edits no queue took on the drawing of tab `tab` (by document id),
    /// with the mark a save gives the drawing for them: `UNQUEUED_EDITS`,
    /// dated by the first. A drawing whose list was cleared keeps the mark
    /// it had then, with what no queue took since. `None` while there are
    /// none.
    pub(super) fn unqueued_for_save(&self, tab: u64) -> Option<(&[Value], Diverged)> {
        let entries = self.unqueued.get(&tab).map_or(&[][..], Vec::as_slice);
        if let Some(kept) = self.unqueued_marks.get(&tab) {
            return Some((entries, kept.clone()));
        }
        let first = entries.first()?;
        let mark = Diverged {
            code: crate::pid::collab_state::UNQUEUED_EDITS.to_owned(),
            message: format!(
                "有 {} 步修改没有排上队，服务器上没有：{}",
                entries.len(),
                first["message"].as_str().unwrap_or_default()
            ),
            at: first["at"].as_i64().unwrap_or_default(),
        };
        Some((entries, mark))
    }

    /// Forget that the drawing of tab `tab` (by document id) was ever in a
    /// room (`PIDCOLLAB DETACH`): its unsynced list and the mark it kept and,
    /// when it is the `session`'s drawing, the queue, the divergence, the
    /// repository and the revision the session had it at. The next `hello`
    /// claims nothing of where it is.
    pub(super) fn forget_drawing(&mut self, tab: u64, session: bool) {
        self.unqueued.remove(&tab);
        self.refused.remove(&tab);
        self.unqueued_marks.remove(&tab);
        if self.foreign_tab == Some(tab) {
            self.foreign_tab = None;
        }
        if self.record_read == Some(tab) {
            self.record_read = None;
        }
        if !session {
            return;
        }
        self.outbox.clear();
        self.unreadable_queue.clear();
        self.diverged = None;
        self.diverged_as = None;
        self.saved_diverged = None;
        self.revision_gap = false;
        self.opened_late = None;
        self.repo_id = None;
        self.base_revision = 0;
        self.had_session = false;
        // There is no record left to read in.
        self.seeded = true;
        self.detached = true;
    }

    /// Why the drawing and the room do not match, as the panel and the
    /// status bar show it: the drawing's divergence, or else this
    /// connection's not being aligned. `None` while they match.
    pub(super) fn mismatch(&self) -> Option<String> {
        self.diverged
            .clone()
            .or_else(|| self.not_aligned.as_ref().map(NotAligned::reason))
    }

    /// Take this connection as not aligned with the room, for `reason`.
    fn hold_apart(&mut self, reason: NotAligned) {
        self.last_error = Some(format!("本次连接与房间对不上：{}", reason.reason()));
        self.catching_up = None;
        for entry in &mut self.outbox {
            entry.sent = false;
        }
        self.not_aligned = Some(reason);
    }

    /// Seed the session's baseline from what the drawing file was saved
    /// with (`REQ-COLL-015`): it was in this repository at this revision,
    /// so it counts as a copy that was here before — `had_session` is what
    /// lets its offline edits queue, and the `hello` claims these (the
    /// transport is handed the claim as `FileClaim` in the same answer).
    ///
    /// `ops` are the queued-but-unanswered ops the file was saved with:
    /// they re-enter the outbox as `restored` — their undo steps died with
    /// the process that made them, so a refusal marks the drawing diverged
    /// rather than pretending a rollback.
    ///
    /// Once only; whether this was the time is the answer. A drawing read
    /// in after a session came up without it is seeded all the same — what
    /// its record says is the drawing's — but that session never held the
    /// record against the room, and the room's changes that were waiting
    /// for a drawing were made on top of a revision this one may not be
    /// at: they are dropped, and the connection is held apart for the rest
    /// of the run (`opened_late`).
    pub(super) fn seed_file_state(
        &mut self,
        claim: &FileClaim,
        ops: Vec<Outgoing>,
        unreadable: Vec<Value>,
    ) -> bool {
        if self.seeded {
            return false;
        }
        self.seeded = true;
        if self.had_session {
            let late = NotAligned {
                code: OPENED_AFTER_SESSION.to_owned(),
                message: "这份图是会话连上之后才读进来的，这次连接没有按它的同步记录核对过；\
                          用这份图重新启动程序再连接"
                    .to_owned(),
            };
            self.pending_remote.clear();
            if self.session_id.is_some() {
                self.hold_apart(late.clone());
            }
            self.opened_late = Some(late);
        }
        self.repo_id = Some(claim.repo_id.clone());
        self.base_revision = claim.revision;
        self.had_session = true;
        self.outbox.extend(ops);
        self.unreadable_queue = unreadable;
        true
    }

    /// What this copy finds against itself before it says what it is
    /// (`SPEC-collab-offline-sync.md` §5.2, step two): a drawing that knows
    /// it is apart from the room — its record says so, this run found it so,
    /// or edits no queue took were made on it — for the reason it would be
    /// saved with; a drawing read in after the session came up
    /// (`seed_file_state`); a queue that is not this person's to send —
    /// steps somebody else queued, or that say nobody; entries that could
    /// not be read; two steps under one name. A reconnection in the same run
    /// is judged the same way.
    pub(super) fn judge_queue(&self) -> Option<NotAligned> {
        let unqueued = || Some(self.unqueued_for_save(self.document?)?.1);
        if let Some(apart) = self.diverged_record().or_else(unqueued) {
            return Some(NotAligned {
                code: apart.code,
                message: apart.message,
            });
        }
        if let Some(late) = &self.opened_late {
            return Some(late.clone());
        }
        let mut others: Vec<&str> = Vec::new();
        let mut nobody = false;
        for entry in &self.outbox {
            match entry.user_id.as_deref() {
                Some(user) if user == self.user_id => {}
                Some(user) if !others.contains(&user) => others.push(user),
                Some(_) => {}
                None => nobody = true,
            }
        }
        if !others.is_empty() || nobody {
            let message = match (others.is_empty(), nobody) {
                (false, false) => format!("图里带着 {} 排队未提交的修改", others.join("、")),
                (false, true) => format!(
                    "图里带着 {} 排队未提交的修改，还有的没有记下是谁排的",
                    others.join("、")
                ),
                (true, _) => "图里带着排队未提交的修改，没有记下是谁排的".to_owned(),
            };
            return Some(NotAligned {
                code: QUEUED_BY_OTHERS.to_owned(),
                message,
            });
        }
        if !self.unreadable_queue.is_empty() {
            return Some(NotAligned {
                code: QUEUE_UNREADABLE.to_owned(),
                message: format!(
                    "图里排队未提交的修改有 {} 条读不出来",
                    self.unreadable_queue.len()
                ),
            });
        }
        let mut names = std::collections::BTreeSet::new();
        let twice = self
            .outbox
            .iter()
            .find(|entry| !names.insert(entry.op_id.as_str()))?;
        Some(NotAligned {
            code: QUEUE_DUPLICATE_ID.to_owned(),
            message: format!("图里排队未提交的修改有两条都叫 {}", twice.op_id),
        })
    }

    /// Turn the holder a denial names into the user the panel should show.
    ///
    /// The server names the holding *session* (`VAL-SRV-011`), while the panel
    /// and `VAL-CLI-005` speak of the holding *user*: the roster already maps
    /// session → user, so resolve through it and fall back to the raw value
    /// when the holder is not (or no longer) on the drawing.
    ///
    /// The user id wins over the display name: `GET /api/v1/collab/status`
    /// publishes `tag → user id`, and `VAL-CLI-005` accepts either, so keeping
    /// one spelling everywhere is what lets the panel and the endpoint agree.
    fn resolve_holder(&self, holder: &str) -> String {
        if let Some(peer) = self.peers.iter().find(|peer| peer.session_id == holder) {
            if !peer.user_id.is_empty() {
                return peer.user_id.clone();
            }
            if !peer.display_name.is_empty() {
                return peer.display_name.clone();
            }
        }
        if self.session_id.as_deref() == Some(holder) && !self.user_id.is_empty() {
            return self.user_id.clone();
        }
        holder.to_owned()
    }

    /// Who holds the edit right of a drawing this session was downgraded on,
    /// as the user the roster knows the holding session by.
    ///
    /// `readonly_holder` keeps what the server sent — a session id, so the
    /// same user's second session is not mistaken for the holder — while the
    /// panel and whoever reads the status want the person
    /// (`VAL-CLI-012`). `None` when nobody holds it, or nothing was downgraded.
    pub(super) fn readonly_holder_user(&self) -> Option<String> {
        self.readonly_holder
            .as_deref()
            .map(|holder| self.resolve_holder(holder))
    }

    /// Take the config the transport thread will use.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn client_config(&self) -> ClientConfig {
        ClientConfig {
            server_url: self.server_url.clone(),
            user_id: self.user_id.clone(),
            drawing_id: self.drawing_id.clone(),
            mode: if self.mode.is_empty() {
                DEFAULT_MODE.to_owned()
            } else {
                self.mode.clone()
            },
            client_version: env!("OCS_APP_VERSION").to_owned(),
        }
    }

    /// Publish the current config to the process-global the subscription
    /// worker reads when it starts its thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn sync_client_config(&self) {
        transport::set_client_config(self.client_config());
    }

    /// Answer question `ask` of the transport's about the drawing's tags.
    pub(super) fn say_tags(&mut self, ask: u64, said: TagsSaid) {
        // Tags said to a session that is up are told by leaving it.
        let tags_to_tell = matches!(
            &said,
            TagsSaid::Tags(tags, _) | TagsSaid::Detached(tags) if !tags.is_empty()
        );
        if tags_to_tell && self.session_id.is_some() {
            self.rejoining = true;
        }
        match &said {
            TagsSaid::Apart(reason) => {
                // Said to a session that is up, the answer came too late
                // for its `hello`: it is held apart from now, and the
                // transport leaves it to join again claiming nothing.
                if self.session_id.is_some() && self.not_aligned.is_none() {
                    self.hold_apart(reason.clone());
                }
                self.apart_before_hello = Some(reason.clone());
            }
            TagsSaid::Tags(..) | TagsSaid::Detached(_) | TagsSaid::NotYet => {
                self.apart_before_hello = None
            }
        }
        #[cfg(test)]
        self.tags_said.push((ask, said.clone()));
        #[cfg(not(target_arch = "wasm32"))]
        transport::answer_tags(ask, said);
        // (Unused on the web build, which has no socket to say them on.)
        #[cfg(target_arch = "wasm32")]
        let _ = (ask, said);
    }

    /// Fold one transport event into the visible state.
    pub(super) fn apply(&mut self, event: CollabEvent) {
        match event {
            CollabEvent::Connecting { attempt } => {
                self.connected = false;
                self.reconnect_attempts = attempt;
            }
            CollabEvent::TagsWanted { ask } => {
                // The latest question is the one that stands: whatever was
                // said to an earlier one went with the dial that asked it.
                self.tags_wanted = Some(TagsAsked {
                    ask,
                    ..TagsAsked::default()
                });
            }
            CollabEvent::Connected {
                session_id,
                base_revision,
                peers,
                claims,
                repo_id,
            } => {
                // Each connection is judged afresh. One the `welcome`
                // refused is held apart (`REQ-COLL-015`), and so is one
                // this copy held apart before its `hello` — for that.
                let refused = self.align_refused.take();
                let refused = self.apart_before_hello.clone().or(refused);
                // The server replays what this session missed, from the
                // revision its drawing has: the baseline stays where it is
                // and each replayed op moves it on, like any op. Nothing
                // of this session's goes out until the replay is through.
                let replay = self
                    .resume_from
                    .take()
                    .filter(|from| self.had_session && *from == self.base_revision);
                let apart = match (refused, replay) {
                    (Some(refused), _) => Some(refused),
                    (None, Some(from)) => {
                        self.catching_up = (base_revision > from).then_some(base_revision);
                        None
                    }
                    // Coming back to a room that moved on while this
                    // drawing was away, with no replay: what was applied in
                    // between is not in this drawing, so it is held apart
                    // too, and keeps its own baseline.
                    (None, None) if self.had_session && base_revision > self.base_revision => {
                        Some(NotAligned {
                            code: ocs_collab::protocol::codes::REPLAY_GAP.to_owned(),
                            message: format!(
                                "房间已从修订 {} 走到 {base_revision}，其间的变更没有补发给这份图纸",
                                self.base_revision
                            ),
                        })
                    }
                    // A room that is *behind* is one whose server started
                    // over; this drawing joins it as it joined the first
                    // time, taken to be what the room has.
                    (None, None) => {
                        self.base_revision = base_revision;
                        self.catching_up = None;
                        None
                    }
                };
                if apart.is_none() {
                    self.had_session = true;
                    self.detached = false;
                    // Which repository this room is of; what the drawing
                    // file is saved with, so its next open can say where it
                    // was. One that is held apart keeps what it had.
                    self.repo_id = Some(repo_id).filter(|repo| !repo.is_empty());
                }
                self.connected = true;
                // The others on the drawing. `welcome.peers` does not list
                // the session it welcomes; a server that still does (it
                // used to) is read the same way.
                self.peers = peers
                    .into_iter()
                    .filter(|peer| peer.session_id != session_id)
                    .collect();
                self.session_id = Some(session_id);
                self.claims = claims.into_iter().collect();
                // A fresh session re-derives write access from the server: a
                // session that may not write is told so right behind this
                // `welcome`, and one that hears nothing may.
                self.readonly = false;
                self.readonly_holder = None;
                self.readonly_level = None;
                self.readonly_code = None;
                self.reconnect_attempts = 0;
                self.last_error = None;
                // The ops the old session left unanswered are still this
                // person's steps: none is on this session's wire yet, and
                // they go out again from the first — unless the replay
                // shows the room took one before the socket dropped.
                // (They used to be forgotten here, with their steps left in
                // the drawing.)
                for entry in &mut self.outbox {
                    entry.sent = false;
                }
                if let Some(apart) = apart {
                    self.hold_apart(apart);
                }
                self.pump();
                // The session before this one was left to tell the drawing's
                // tags: what it held by claim is asked for again.
                if std::mem::take(&mut self.rejoining) {
                    for tag in std::mem::take(&mut self.own_claims) {
                        // (Refused for an empty tag alone, and none is.)
                        let _ = self.request_claim(&tag);
                    }
                }
            }
            CollabEvent::AlignRefused { code, message } => {
                self.align_refused = Some(NotAligned { code, message });
            }
            CollabEvent::Resuming { from } => {
                self.resume_from = Some(from);
            }
            CollabEvent::ClaimsExpire(claims) => {
                self.claim_expiry = claims.into_iter().collect();
            }
            CollabEvent::Disconnected { reason, will_retry } => {
                // The claims this session asked for went with it — unless
                // it was left to tell the drawing's tags, and is being
                // dialled for again (`rejoining`).
                let left_to_tell = self.rejoining && self.session_id.is_some() && will_retry;
                if !left_to_tell {
                    self.rejoining = false;
                    self.own_claims.clear();
                }
                self.connected = false;
                self.session_id = None;
                self.peers.clear();
                self.claims.clear();
                self.claim_expiry.clear();
                self.catching_up = None;
                self.resume_from = None;
                self.align_refused = None;
                self.not_aligned = None;
                for entry in &mut self.outbox {
                    entry.sent = false;
                }
                // `readonly` stays as it is. It used to be cleared here, so
                // a session that had been told it may not write could write
                // again the moment its network blinked.
                self.last_error = Some(if will_retry {
                    format!("{reason}（将自动重连）")
                } else {
                    reason
                });
            }
            CollabEvent::PeerJoined(peer) => {
                if let Some(existing) = self
                    .peers
                    .iter_mut()
                    .find(|p| p.session_id == peer.session_id)
                {
                    *existing = peer;
                } else {
                    self.peers.push(peer);
                }
            }
            CollabEvent::PeerLeft { session_id } => {
                self.peers.retain(|p| p.session_id != session_id);
            }
            CollabEvent::ClaimGranted { tag } => {
                let holder = self.user_id.clone();
                self.last_claim_message = Some(format!("已占用位号 {tag}"));
                // Whatever was known of an earlier claim on it is over.
                self.claim_expiry.remove(&tag);
                self.own_claims.insert(tag.clone());
                self.claims.insert(tag, holder);
            }
            CollabEvent::ClaimDenied {
                tag,
                holder,
                expires_at,
            } => {
                // Refused: not a claim of this session's any more.
                self.own_claims.remove(&tag);
                // The claim stays with its holder; record who it is so the
                // panel can grey the row out and say until when.
                let holder = self.resolve_holder(&holder);
                self.last_claim_message =
                    Some(format!("位号 {tag} 已被 {holder} 占用，到期时间 {expires_at}"));
                self.last_error = Some(format!("位号 {tag} 已被占用，到期时间 {expires_at}"));
                self.claim_expiry.insert(tag.clone(), expires_at);
                self.claims.insert(tag, holder);
            }
            CollabEvent::ClaimReleased { tag, reason } => {
                self.last_claim_message = Some(format!("位号 {tag} 占用已释放（{reason}）"));
                self.own_claims.remove(&tag);
                self.claims.remove(&tag);
                self.claim_expiry.remove(&tag);
            }
            CollabEvent::ReadonlyDowngrade {
                reason,
                holder,
                level,
                code,
            } => {
                // The server tells every session on a drawing that went 只读
                // who holds the edit right — the holder too. A session that
                // finds its own id there is the one that goes on writing.
                let holds = holder
                    .as_deref()
                    .is_some_and(|holder| self.session_id.as_deref() == Some(holder));
                self.readonly = !holds;
                self.readonly_holder = holder;
                self.readonly_level = if holds { None } else { level };
                self.readonly_code = if holds { None } else { code };
                self.last_error = if holds {
                    None
                } else {
                    Some(format!("只读降级：{reason}"))
                };
            }
            CollabEvent::OpApplied {
                op_id,
                revision,
                changes,
            } => {
                // What the room does is not written into a drawing whose
                // connection is held apart, and its baseline stays where
                // the drawing is.
                if self.not_aligned.is_some() {
                    return;
                }
                // Said again: a revision this drawing is already at. Applying
                // it twice is not harmless (a rename finds its old tag gone)
                // and the baseline does not go backwards.
                if revision <= self.base_revision {
                    self.answered(&op_id);
                    self.pump();
                    return;
                }
                // Every op the room applies comes here, one revision after the
                // other. One that is not the next means something in between
                // never arrived: this drawing is short a change, and its
                // baseline stays where the drawing has all of the room's.
                if revision != self.base_revision + 1 {
                    self.revision_gap = true;
                    self.mark_diverged(
                        crate::pid::collab_state::REVISION_GAP,
                        format!(
                            "收到修订 {revision}，本地基线是 {}：中间的变更没有收到",
                            self.base_revision
                        ),
                    );
                }
                // A ruling settles the entry its id names. Its changes are a
                // peer's like any other — the ruling was made on the server
                // — and reach the drawing below.
                if let Some(ruled) = op_id.strip_prefix(RULING_OP_PREFIX) {
                    self.conflicts.retain(|entry| entry.conflict_id != ruled);
                }
                // The room broadcast reaches the originator too; its own op is
                // already in the local document, so only the baseline moves.
                // (Its own, too, when it comes in a replay: sent before the
                // socket dropped, taken by the room, and never heard of
                // again until now. It is not sent a second time.)
                let before = self.base_revision;
                let under_an_earlier_session = self
                    .outbox
                    .iter()
                    .any(|entry| entry.op_id == op_id && !entry.sent);
                let mine = self.answered(&op_id);
                if mine && under_an_earlier_session {
                    // The room took it from a session of this client's that
                    // is over, so the server reads it as somebody else's
                    // when it judges what this session sends: a step made
                    // on top of it would give way to it. Every step still
                    // queued was made after it, in this drawing; one whose
                    // baseline is the revision right before it has seen
                    // everything up to it, and says so. (One made against
                    // an older revision has not seen what others did in
                    // between, and keeps its baseline.)
                    for entry in &mut self.outbox {
                        if entry.base_revision == before {
                            entry.base_revision = revision;
                        }
                    }
                }
                if mine {
                    // Already in this drawing — except for a symbol the
                    // server took under another tag than it was sent with,
                    // because somebody else's had that one first. That one
                    // is renamed here, where it was placed.
                    let renames = reassignments(&changes);
                    if !renames.is_empty() {
                        self.pending_remote.push(RemoteChange {
                            op_id,
                            changes: renames,
                        });
                    }
                } else {
                    self.applied_ops = self.applied_ops.saturating_add(1);
                    self.pending_remote.push(RemoteChange { op_id, changes });
                }
                if !self.revision_gap {
                    self.base_revision = revision;
                }
                if self.catching_up.is_some_and(|target| revision >= target) {
                    self.catching_up = None;
                }
                self.pump();
            }
            CollabEvent::OpRefused {
                op_id,
                code,
                message,
                rejected,
            } => {
                // Not judged, so not refused either: the step stays queued
                // for a connection that aligns, and nothing is taken back.
                if code == ocs_collab::protocol::codes::NOT_ALIGNED {
                    self.hold_apart(NotAligned { code, message });
                    return;
                }
                self.last_error = Some(format!("{code}: {message}"));
                // Nothing of it took effect: the step goes back
                // (`OpenCADStudio::settle_rejected_ops`), and until it has,
                // nothing behind it goes out — it may speak for a step that
                // goes back with it. An error about one change of an op is
                // word only: the rest of the op is on its way as
                // `op_applied`, which is its answer.
                let refused = if rejected {
                    self.own_ops.retain(|own| own != &op_id);
                    self.outbox
                        .iter()
                        .position(|entry| entry.op_id == op_id)
                        .and_then(|at| self.outbox.remove(at))
                } else {
                    None
                };
                match refused {
                    Some(entry) => {
                        self.rejections.push(Rejection { entry, code, message });
                        self.hold = true;
                    }
                    // Word only — or the refusal of an op this state no
                    // longer has, which leaves it nothing to take back.
                    None => self.refusals_to_say.push((code, message)),
                }
            }
            CollabEvent::OpsInConflict { op_ids } => {
                // Held in a conflict entry: answered, as far as the queue
                // goes. What this drawing shows of it stays until the
                // ruling comes, as before.
                for op_id in &op_ids {
                    self.answered(op_id);
                }
                self.pump();
            }
            CollabEvent::Conflict(entry) => {
                // Told once per entry; a second telling is the same entry.
                if !self
                    .conflicts
                    .iter()
                    .any(|known| known.conflict_id == entry.conflict_id)
                {
                    self.conflicts.push(entry);
                }
            }
            CollabEvent::ServerError { code, message } => {
                self.last_error = Some(format!("{code}: {message}"));
                // A server that does not say which op it refuses (one from
                // before `error.op_id`): with one op on the wire it can
                // only mean that one. The step stays in this drawing, as
                // it always did with such a server; the queue moves on.
                if refuses_an_op(&code) {
                    if let Some(at) = self.outbox.iter().position(|entry| entry.sent) {
                        if let Some(entry) = self.outbox.remove(at) {
                            self.own_ops.retain(|own| own != &entry.op_id);
                        }
                    }
                    self.pump();
                }
            }
        }
    }

    /// The next number for an undo step (`DeltaSnapshot::collab_step`), by
    /// which the op that speaks for the step finds it again.
    pub(super) fn next_step(&mut self) -> u64 {
        self.step_seq += 1;
        self.step_seq
    }

    /// The room has answered op `op_id` — applied it, or holds it in a
    /// conflict entry: it leaves the queue. Returns whether it was an op of
    /// this session's.
    fn answered(&mut self, op_id: &str) -> bool {
        let known = self.own_ops.iter().any(|own| own == op_id)
            || self.outbox.iter().any(|entry| entry.op_id == op_id);
        self.own_ops.retain(|own| own != op_id);
        self.outbox.retain(|entry| entry.op_id != op_id);
        known
    }

    /// The app has taken back the steps of every refused op: the queue
    /// moves again.
    pub(super) fn release_hold(&mut self) {
        self.hold = false;
        self.pump();
    }

    /// Put the first op of the queue on the wire, when there is a session
    /// to send it on, the session has caught up with the room and written
    /// what it heard of it into the drawing, and no other op of this
    /// session's is waiting for its answer. Heard is not enough: a change
    /// the drawing cannot take marks it diverged, and nothing is to go out
    /// ahead of that (`apply_remote_changes` comes back here once it has
    /// written them).
    fn pump(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.hold
                || !self.connected
                || self.catching_up.is_some()
                || !self.pending_remote.is_empty()
                || self.diverged.is_some()
                || self.not_aligned.is_some()
            {
                return;
            }
            let Some(session_id) = self.session_id.clone() else {
                return;
            };
            if self.outbox.iter().any(|entry| entry.sent) {
                return;
            }
            let room = self.base_revision;
            let Some(entry) = self.outbox.front_mut() else {
                return;
            };
            // The revision the step was made against — or the room's own,
            // when that is lower: a server that started over counts from
            // nought again, and refuses a baseline it has not reached.
            let base = entry.base_revision.min(room);
            if transport::request_op(&session_id, base, &entry.op_id, entry.payload.clone()).is_ok() {
                entry.base_revision = base;
                entry.sent = true;
            }
        }
    }

    /// The body of `GET /api/v1/collab/status` (`architecture.md` A.5).
    pub(super) fn status_json(&self) -> Value {
        json!({
            "ok": true,
            "configured": self.wanted(),
            "connected": self.connected,
            "server_url": shown_address(&self.server_url),
            "user_id": self.user_id,
            "drawing_id": self.drawing_id,
            "session_id": self.session_id,
            // Which repository this drawing is aligned to — that of the
            // last `welcome` that took it as aligned, or the file's saved
            // claim (`REQ-COLL-015`).
            "repo_id": self.repo_id,
            "base_revision": self.base_revision,
            "peers": self
                .peers
                .iter()
                .map(|p| json!({
                    "session_id": p.session_id,
                    "user_id": p.user_id,
                    "display_name": p.display_name,
                }))
                .collect::<Vec<_>>(),
            "claims": self
                .claims
                .iter()
                .map(|(tag, holder)| (tag.clone(), json!(holder)))
                .collect::<serde_json::Map<String, Value>>(),
            // `peers` is the others (A.5); this is everybody on the drawing,
            // this session included — what the panel's header counts.
            "online": if self.session_id.is_some() { self.peers.len() + 1 } else { 0 },
            "readonly": self.readonly,
            "readonly_level": self.readonly_level,
            "readonly_holder": self.readonly_holder,
            "readonly_holder_user": self.readonly_holder_user(),
            "reconnect_attempts": self.reconnect_attempts,
            "last_error": self.last_error,
            "last_claim_message": self.last_claim_message,
            "applied_ops": self.applied_ops,
            "pending_remote": self.pending_remote.len(),
            "last_apply_error": self.last_apply_error,
            "document_id": self.document,
            "diverged": self.diverged,
            // `{code, message}` while the connection that is up is held
            // apart from the room.
            "not_aligned": self.not_aligned,
            // tag → when the claim on it runs out (Unix seconds), where the
            // server has said.
            "claim_expiry": self
                .claim_expiry
                .iter()
                .map(|(tag, expires_at)| (tag.clone(), json!(expires_at)))
                .collect::<serde_json::Map<String, Value>>(),
            // Ops of this session's the room has not answered: the one on
            // the wire and the ones waiting behind it, or for a session.
            "queued_ops": self.outbox.len(),
            // The revision a replay is to reach, while one is going on.
            "catching_up": self.catching_up,
            // The last few ops the server refused, and what was done.
            "rejections": self.rejection_log,
            // The entries awaiting a ruling, as the room was told of them.
            "conflicts": self
                .conflicts
                .iter()
                .map(|entry| json!({
                    "conflict_id": entry.conflict_id,
                    "object_ref": entry.object_ref,
                    "field": entry.field,
                    "value_a": entry.value_a,
                    "value_b": entry.value_b,
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// Queue one `op` for the transport thread.
    ///
    /// `payload` is the P&ID semantic change (the same shape `op_applied`
    /// echoes back in `changes`). `base_revision` is whatever the last
    /// `welcome` / `op_applied` established — the server refuses a stale one
    /// (`STALE_BASE_REVISION`), and the next broadcast repairs it.
    ///
    /// The op id is remembered so the room broadcast of our own change does not
    /// get applied to the local document twice.
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn request_op(&mut self, payload: Value) -> Result<(), String> {
        self.queue_op(payload, Vec::new(), true, "").map(|_| ())
    }

    /// Queue one op that speaks for the undo steps `steps` — put on the
    /// drawing (`on`) or taken off it — and send it if it is its turn.
    ///
    /// With a session up it goes out at once, or behind the op the room
    /// has yet to answer. With none up — after there has been one — it
    /// waits for the next (`VAL-CROSS-008`), and with one that is held
    /// apart from the room, for one that aligns. A client that never had a
    /// session queues nothing: when it first joins, its drawing is taken to
    /// be what the room has.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn queue_op(
        &mut self,
        payload: Value,
        steps: Vec<u64>,
        on: bool,
        label: &str,
    ) -> Result<Queued, String> {
        if self.readonly {
            return Err("只读会话不能提交变更".to_owned());
        }
        // Writing on top of a drawing known to be short of the room's changes
        // spreads the difference: the server would take the op, its baseline
        // being the server's own number.
        if let Some(reason) = &self.diverged {
            return Err(format!(
                "本图与房间已不一致（{reason}），已停止提交变更，需要重新同步"
            ));
        }
        if self.session_id.is_none() && !self.had_session {
            return Err("未连接协同服务器".to_owned());
        }
        let op_id = self.name_op();
        #[cfg(test)]
        self.sent.push(payload.clone());
        self.outbox.push_back(Outgoing {
            op_id: op_id.clone(),
            payload,
            base_revision: self.base_revision,
            steps,
            on,
            sent: false,
            label: label.to_owned(),
            restored: false,
            user_id: Some(self.user_id.clone()),
            queued_at: Some(ocs_collab::ids::now_unix()),
            inverse: None,
            inverse_read: false,
        });
        if self.own_ops.len() >= 64 {
            self.own_ops.pop_front();
        }
        self.own_ops.push_back(op_id);
        self.pump();
        let sent = self.outbox.back().is_some_and(|entry| entry.sent);
        Ok(match (self.session_id.is_some() && self.connected, sent) {
            (true, true) => Queued::Sent,
            (true, false) if self.not_aligned.is_some() => Queued::NotAligned(self.outbox.len()),
            (true, false) => Queued::Behind(self.outbox.len().saturating_sub(1)),
            (false, _) => Queued::Offline(self.outbox.len()),
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn queue_op(
        &mut self,
        _payload: Value,
        _steps: Vec<u64>,
        _on: bool,
        _label: &str,
    ) -> Result<Queued, String> {
        Err("网页版不支持协同变更".to_owned())
    }

    /// The id of the next op this run makes: `op-<run id>-<n>`, from 1.
    #[cfg(not(target_arch = "wasm32"))]
    fn name_op(&mut self) -> String {
        if self.run_id.is_empty() {
            self.run_id = ocs_collab::ids::new_run_id();
        }
        self.ops_named += 1;
        ocs_collab::ids::op_id(&self.run_id, self.ops_named)
    }

    /// Ask the transport to send one `claim` / `release` for `tag`.
    ///
    /// Returns why nothing was sent when the client is not in a session — the
    /// caller reports that instead of leaving the click silently ignored. The
    /// request travels through the transport's outbound queue; the server's
    /// answer arrives later as a `CollabEvent`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn request_claim(&mut self, tag: &str) -> Result<(), String> {
        let Some(session_id) = self.session_id.clone() else {
            return Err("未连接协同服务器".to_owned());
        };
        // A claim speaks for edits that would go to the room; a connection
        // held apart sends none.
        if let Some(apart) = &self.not_aligned {
            return Err(format!(
                "这次连接与房间对不上：{}，不申请占用",
                apart.reason()
            ));
        }
        transport::request_claim(&session_id, tag)?;
        #[cfg(test)]
        self.claims_asked.push((session_id, tag.trim().to_owned()));
        self.own_claims.insert(tag.trim().to_owned());
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn request_release(&mut self, tag: &str) -> Result<(), String> {
        let Some(session_id) = self.session_id.clone() else {
            return Err("未连接协同服务器".to_owned());
        };
        transport::request_release(&session_id, tag)?;
        self.own_claims.remove(tag.trim());
        Ok(())
    }

    /// Tell the server of a write this session turned away itself, so that
    /// it is on record there (`audit_log`): what was tried, on which object.
    /// With no session up there is nobody to tell, and nothing is queued for
    /// later — the record would arrive under another session's name.
    pub(super) fn report_attempt(&mut self, object_ref: &str, text: &str) {
        // (Both unused on the web build, which has no socket to report on.)
        let _ = (object_ref, text);
        #[cfg(test)]
        self.attempts.push((object_ref.to_owned(), text.to_owned()));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(session_id) = self.session_id.clone() {
            transport::request_attempt(&session_id, object_ref, text);
        }
    }

    /// The web build has no collaboration socket, so a claim request can never
    /// be sent. Reported as an error rather than ignored, so the panel says why.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn request_claim(&self, _tag: &str) -> Result<(), String> {
        Err("网页版不支持协同占用".to_owned())
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn request_release(&self, _tag: &str) -> Result<(), String> {
        Err("网页版不支持协同占用".to_owned())
    }
}

/// Session mode used when neither the CLI nor the config names one. The server
/// treats any mode other than `只读` as an editing session.
pub(super) const DEFAULT_MODE: &str = "协同编辑";

/// Subscribe the app to the collaboration stream.
///
/// `iced::Subscription::run` identifies a subscription by the worker's function
/// pointer, so the worker must be a bare `fn` — a closure would re-subscribe on
/// every frame.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn subscribe() -> iced::Subscription<CollabEvent> {
    iced::Subscription::run(worker)
}

#[cfg(not(target_arch = "wasm32"))]
fn worker() -> impl iced::futures::Stream<Item = CollabEvent> {
    // The receiving half of a futures channel already is the stream iced asks
    // for, so the worker has no future of its own to drive: the transport
    // thread holds the sending half, and the stream ends when that thread
    // does — at quit, or once the subscription itself is gone. Nothing under
    // this module is asynchronous code (`VAL-CLI-019`).
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel::<CollabEvent>(64);
    eprintln!("[collab] 订阅已启动，准备连接");
    // The app's client: it reads the published config, drains the queue the
    // UI fills, and quits on a signal that is this thread's alone —
    // `shutdown` raises it through the registration below, and nothing else
    // in the process can.
    let link = transport::Link::app();
    let thread_link = link.clone();
    let handle = std::thread::spawn(move || {
        if let Err(error) = transport::run_client(&mut sender, &thread_link) {
            eprintln!("[collab] 传输层退出：{error}");
        }
    });
    // The quit path needs this handle to wait for the goodbye frame, so the
    // thread that owns the socket is the one the app joins.
    transport::register_worker(handle, link);
    receiver
}

/// Close the collaboration socket before the process exits.
///
/// A process that just dies leaves the server counting down the 90 s heartbeat
/// timeout; saying goodbye retires the session and tells the other peers at
/// once (`VAL-CLI-016`). Blocking here is bounded by the transport's own
/// budget, so a wedged socket cannot stall the quit.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn shutdown() {
    transport::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, user: &str) -> PeerInfo {
        PeerInfo {
            session_id: id.to_owned(),
            user_id: user.to_owned(),
            display_name: user.to_owned(),
        }
    }

    /// VAL-CLI-019: nothing under `src/app/collab/` is asynchronous code. The
    /// contract checks that by searching the sources for four spellings, so
    /// this searches the same files for the same four — written in pieces
    /// here, or the test would find itself.
    #[test]
    fn nothing_under_this_module_is_asynchronous_code() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app")
            .join("collab");
        let needles = [
            ["async", " fn"].concat(),
            [".aw", "ait"].concat(),
            ["tok", "io"].concat(),
            ["async", "_std"].concat(),
        ];
        // Every source file under the folder, however deep: the contract's
        // search is recursive, and a submodule added later must not slip by.
        fn sources_under(folder: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(folder).expect("a module folder") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    sources_under(&path, out);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    out.push(path);
                }
            }
        }
        let mut paths = Vec::new();
        sources_under(&folder, &mut paths);
        let mut read = 0;
        for path in paths {
            let source = std::fs::read_to_string(&path).expect("a source file");
            read += 1;
            for (number, line) in source.lines().enumerate() {
                for needle in &needles {
                    assert!(
                        !line.contains(needle.as_str()),
                        "{}:{}: {needle:?} in `{}`",
                        path.display(),
                        number + 1,
                        line.trim()
                    );
                }
            }
        }
        assert!(read >= 6, "only {read} source files were read in {folder:?}");
    }

    #[test]
    fn no_server_url_means_no_connection_is_wanted() {
        let mut state = State::new();
        assert!(!state.wanted(), "an unconfigured client must stay offline");
        assert_eq!(state.status_json()["connected"], false);
        assert_eq!(state.status_json()["server_url"], "");

        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        assert!(state.wanted());
    }

    fn conflict(id: &str) -> CollabEvent {
        CollabEvent::Conflict(PendingConflict {
            conflict_id: id.to_owned(),
            object_ref: "P-1001".to_owned(),
            field: "设计压力".to_owned(),
            value_a: "1.6MPa".to_owned(),
            value_b: "2.5MPa".to_owned(),
        })
    }

    /// `VAL-CROSS-007`: a conflict the room is told of is kept, and
    /// published, until the ruling on it comes — as the `op_applied` whose
    /// id names the entry. Nothing else takes it off: not another op, not a
    /// dropped socket (it is still waiting on the server), not being told
    /// of it twice.
    #[test]
    fn a_conflict_is_kept_until_its_ruling_arrives() {
        let mut state = State::new();
        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        assert_eq!(state.status_json()["conflicts"], json!([]));

        state.apply(conflict("c-1"));
        state.apply(conflict("c-1"));
        state.apply(conflict("c-2"));
        let published = state.status_json()["conflicts"].clone();
        assert_eq!(
            published,
            json!([
                {"conflict_id": "c-1", "object_ref": "P-1001", "field": "设计压力",
                 "value_a": "1.6MPa", "value_b": "2.5MPa"},
                {"conflict_id": "c-2", "object_ref": "P-1001", "field": "设计压力",
                 "value_a": "1.6MPa", "value_b": "2.5MPa"},
            ])
        );

        // Somebody's op, and a lost socket: both entries are still waiting.
        state.apply(CollabEvent::OpApplied {
            op_id: "op-s-2-1".to_owned(),
            revision: 1,
            changes: Vec::new(),
        });
        state.apply(CollabEvent::Disconnected {
            reason: "read error: ConnectionReset".to_owned(),
            will_retry: true,
        });
        state.apply(CollabEvent::Connected {
            session_id: "s-3".to_owned(),
            base_revision: 1,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        assert_eq!(state.conflicts.len(), 2);

        // The ruling on the first.
        state.apply(CollabEvent::OpApplied {
            op_id: "resolve-c-1".to_owned(),
            revision: 2,
            changes: vec![json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        });
        let left: Vec<&str> = state
            .conflicts
            .iter()
            .map(|entry| entry.conflict_id.as_str())
            .collect();
        assert_eq!(left, ["c-2"]);
        assert_eq!(
            state.pending_remote.last().map(|remote| remote.op_id.as_str()),
            Some("resolve-c-1"),
            "the ruling is a change like any other, for the drawing to take"
        );
    }

    /// An address is shown without what stands before the `@` of its
    /// authority, and otherwise as it is — an `@` further along is part of
    /// the path, not a credential.
    #[test]
    fn an_address_is_shown_without_its_credentials() {
        for (address, shown) in [
            ("ws://127.0.0.1:3240/ws", "ws://127.0.0.1:3240/ws"),
            ("ws://someone:hunter2@127.0.0.1:3240/ws", "ws://***@127.0.0.1:3240/ws"),
            ("ws://someone@host/ws", "ws://***@host/ws"),
            // A password may itself hold an `@`: the last one ends it.
            ("ws://a:p@ss@host:1/ws", "ws://***@host:1/ws"),
            ("ws://host/ws?at=a@b", "ws://host/ws?at=a@b"),
            ("ws://host/room@2", "ws://host/room@2"),
            ("", ""),
            ("not an address", "not an address"),
        ] {
            assert_eq!(shown_address(address), shown, "{address}");
        }

        // Where the status endpoint publishes it.
        let mut state = State::new();
        state.server_url = "ws://someone:hunter2@127.0.0.1:3240/ws".to_owned();
        let status = state.status_json().to_string();
        assert!(!status.contains("hunter2"), "{status}");
        assert!(status.contains("127.0.0.1:3240"), "{status}");
    }

    #[test]
    fn welcome_fills_the_published_status_shape() {
        let mut state = State::new();
        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        state.user_id = "u1".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-2", "u2")],
            claims: vec![("P-1001".to_owned(), "u1".to_owned())],
        });

        let status = state.status_json();
        assert_eq!(status["connected"], true);
        assert_eq!(status["server_url"], "ws://127.0.0.1:3240/ws");
        assert_eq!(status["session_id"], "s-1");
        assert_eq!(status["base_revision"], 0);
        assert_eq!(status["peers"][0]["user_id"], "u2");
        assert_eq!(status["claims"]["P-1001"], "u1");
        assert_eq!(status["readonly"], false);
        assert!(status["readonly_holder"].is_null());
    }

    #[test]
    fn disconnect_clears_the_session_but_keeps_the_configured_url() {
        let mut state = State::new();
        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 3,
            repo_id: "r-test".into(),
            peers: vec![peer("s-2", "u2")],
            claims: vec![("P-1001".to_owned(), "u1".to_owned())],
        });
        state.apply(CollabEvent::Disconnected {
            reason: "read error: ConnectionReset".to_owned(),
            will_retry: true,
        });

        let status = state.status_json();
        assert_eq!(status["connected"], false);
        assert!(status["session_id"].is_null());
        assert_eq!(status["peers"].as_array().unwrap().len(), 0);
        assert_eq!(status["claims"].as_object().unwrap().len(), 0);
        assert_eq!(status["server_url"], "ws://127.0.0.1:3240/ws");
        assert!(status["last_error"]
            .as_str()
            .unwrap()
            .contains("ConnectionReset"));
    }

    #[test]
    fn peer_join_and_leave_track_the_roster_without_duplicates() {
        let mut state = State::new();
        state.apply(CollabEvent::PeerJoined(peer("s-2", "u2")));
        state.apply(CollabEvent::PeerJoined(peer("s-2", "u2")));
        assert_eq!(state.peers.len(), 1);

        state.apply(CollabEvent::PeerLeft {
            session_id: "s-2".to_owned(),
        });
        assert!(state.peers.is_empty());
    }

    #[test]
    fn readonly_downgrade_records_the_holder() {
        let mut state = State::new();
        state.apply(CollabEvent::ReadonlyDowngrade {
            reason: "另一会话进入单人编辑".to_owned(),
            holder: Some("s-9".to_owned()),
            level: None,
            code: None,
        });
        assert!(state.readonly);
        assert_eq!(state.readonly_holder.as_deref(), Some("s-9"));
    }

    /// VAL-CLI-012: the server names the holding *session*; whoever reads the
    /// status wants to know which *user* that is, so the roster resolves it and
    /// the status publishes both.
    #[test]
    fn a_downgrade_publishes_the_holder_as_the_user_the_roster_knows() {
        let mut state = State::new();
        state.user_id = "u2".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-1", "u1"), peer("s-2", "u2")],
            claims: Vec::new(),
        });
        state.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸已切换为只读".to_owned(),
            holder: Some("s-1".to_owned()),
            level: None,
            code: None,
        });

        let status = state.status_json();
        assert_eq!(status["readonly"], true);
        assert_eq!(status["readonly_holder"], "s-1");
        assert_eq!(status["readonly_holder_user"], "u1");
    }

    #[test]
    fn claim_events_keep_the_holder_map_in_step() {
        let mut state = State::new();
        state.user_id = "u1".to_owned();
        state.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        assert_eq!(state.claims.get("P-1001").map(String::as_str), Some("u1"));

        state.apply(CollabEvent::ClaimDenied {
            tag: "P-1002".to_owned(),
            holder: "u2".to_owned(),
            expires_at: 1_700_000_300,
        });
        assert_eq!(state.claims.get("P-1002").map(String::as_str), Some("u2"));

        state.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".to_owned(),
            reason: "released".to_owned(),
        });
        assert!(!state.claims.contains_key("P-1001"));
    }

    /// VAL-CLI-004: a granted claim puts this session's own user id in the
    /// published `claims` map, which is exactly what `/collab/status` reads.
    #[test]
    fn a_granted_claim_publishes_this_sessions_user_id() {
        let mut state = State::new();
        state.user_id = "u1".to_owned();
        state.apply(CollabEvent::ClaimGranted {
            tag: "P-1001".to_owned(),
        });
        let status = state.status_json();
        assert_eq!(status["claims"]["P-1001"], "u1");
        assert!(state.last_claim_message.is_some());
    }

    /// VAL-CLI-005: a denial names the holder, so the panel can grey the row
    /// and say who owns it — and the map keeps that holder, not this session.
    #[test]
    fn a_denied_claim_records_the_holder_and_never_claims_it_for_us() {
        let mut state = State::new();
        state.user_id = "u2".to_owned();
        state.apply(CollabEvent::ClaimDenied {
            tag: "P-1001".to_owned(),
            holder: "u1".to_owned(),
            expires_at: 1_700_000_300,
        });
        assert_eq!(state.claims.get("P-1001").map(String::as_str), Some("u1"));
        let message = state.last_claim_message.as_deref().unwrap();
        assert!(message.contains("u1"), "{message}");
        assert!(message.contains("1700000300"), "{message}");
    }

    /// The server names the holding *session*; the panel and the published
    /// `claims` map speak of the holding *user*, so the roster resolves it.
    #[test]
    fn a_denial_naming_a_session_publishes_that_sessions_user() {
        let mut state = State::new();
        state.user_id = "u2".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-2", "u2"), peer("s-1", "u1")],
            claims: Vec::new(),
        });
        state.apply(CollabEvent::ClaimDenied {
            tag: "P-1001".to_owned(),
            holder: "s-1".to_owned(),
            expires_at: 1_700_000_300,
        });
        assert_eq!(state.claims.get("P-1001").map(String::as_str), Some("u1"));
        assert_eq!(state.status_json()["claims"]["P-1001"], "u1");
    }

    /// VAL-CLI-006 / 007: either release path drops the tag from the map, and
    /// the reason survives in the message the panel shows.
    #[test]
    fn a_release_clears_the_tag_and_keeps_its_reason() {
        for reason in ["released", "expired"] {
            let mut state = State::new();
            state.user_id = "u1".to_owned();
            state.apply(CollabEvent::ClaimGranted {
                tag: "P-1001".to_owned(),
            });
            state.apply(CollabEvent::ClaimReleased {
                tag: "P-1001".to_owned(),
                reason: reason.to_owned(),
            });
            assert!(!state.claims.contains_key("P-1001"), "{reason}");
            assert!(
                state
                    .last_claim_message
                    .as_deref()
                    .is_some_and(|message| message.contains(reason)),
                "{reason}: {:?}",
                state.last_claim_message
            );
        }
    }

    /// A claim request without a session is refused rather than silently
    /// dropped: the panel reports why the click did nothing.
    #[test]
    fn a_claim_without_a_session_is_refused_with_a_reason() {
        let mut state = State::new();
        assert!(state.request_claim("P-1001").is_err());
        assert!(state.request_release("P-1001").is_err());
    }

    /// `welcome.peers` is the others on the drawing: the first session in
    /// finds it empty, the second finds the first (`VAL-CLI-013`,
    /// `VAL-CROSS-006`). The published roster is the same list
    /// (`architecture.md` A.5), and the head count beside it is everybody —
    /// this session too. A server that still lists the session it welcomes,
    /// as it used to, is read the same way.
    #[test]
    fn the_roster_is_the_others_and_the_head_count_includes_this_session() {
        let mut state = State::new();
        assert_eq!(state.status_json()["online"], 0, "no session, nobody");
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        let status = state.status_json();
        assert_eq!(status["peers"].as_array().map(Vec::len), Some(0));
        assert_eq!(status["online"], 1);

        state.apply(CollabEvent::PeerJoined(peer("s-2", "u2")));
        let status = state.status_json();
        assert_eq!(status["peers"].as_array().map(Vec::len), Some(1));
        assert_eq!(status["peers"][0]["user_id"], "u2");
        assert_eq!(status["online"], 2);

        // The second one in: the welcome lists the first.
        let mut second = State::new();
        second.apply(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-1", "u1")],
            claims: Vec::new(),
        });
        assert_eq!(second.peers, [peer("s-1", "u1")]);

        // The old form, with the welcomed session in its own list.
        let mut old = State::new();
        old.apply(CollabEvent::Connected {
            session_id: "s-2".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-1", "u1"), peer("s-2", "u2")],
            claims: Vec::new(),
        });
        assert_eq!(old.peers, [peer("s-1", "u1")]);
        assert_eq!(old.status_json()["online"], 2);
    }

    /// A drawing that goes 只读 keeps one writer, and the server tells every
    /// session who that is — the holder too. A session that finds its own
    /// id named goes on writing; it used to be told nothing, and one that
    /// had been downgraded before and was later made holder never knew.
    #[test]
    fn a_downgrade_naming_this_session_as_the_holder_leaves_it_writing() {
        let mut state = State::new();
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision: 0,
            repo_id: "r-test".into(),
            peers: vec![peer("s-1", "u1"), peer("s-2", "u2")],
            claims: Vec::new(),
        });
        // Downgraded: somebody else holds the edit right.
        state.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸已切换为只读".to_owned(),
            holder: Some("s-2".to_owned()),
            level: None,
            code: None,
        });
        assert!(state.readonly);

        // The drawing is settled again, and this time this session holds it.
        state.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸已切换为只读".to_owned(),
            holder: Some("s-1".to_owned()),
            level: None,
            code: None,
        });
        assert!(!state.readonly, "the holder goes on writing");
        assert_eq!(state.readonly_holder.as_deref(), Some("s-1"));
        assert_eq!(state.last_error, None);
        assert_eq!(state.status_json()["readonly"], false);
    }

    fn joined_at(base_revision: i64) -> State {
        let mut state = State::new();
        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        state.user_id = "u1".to_owned();
        state.apply(CollabEvent::Connected {
            session_id: "s-1".to_owned(),
            base_revision,
            peers: Vec::new(),
            claims: Vec::new(),
            repo_id: "r-test".into(),
        });
        state
    }

    /// `REQ-COLL-015`: a copy the file says was in a room seeds its
    /// baseline once — its edits then queue like a session's leftovers,
    /// ahead of any session — and a refused alignment holds the connection
    /// apart (`OFF-2`): the session is up, the room is seen, nothing is
    /// submitted, and the copy keeps what its file said.
    #[test]
    fn collab_align_a_seeded_copy_queues_and_a_refusal_holds_it_apart() {
        let mut state = State::new();
        state.server_url = "ws://127.0.0.1:3240/ws".to_owned();
        state.drawing_id = "PID-1101".to_owned();
        let claim = FileClaim {
            drawing_id: "PID-1101".into(),
            repo_id: "r-9".into(),
            revision: 42,
        };
        let restored = Outgoing {
            op_id: "op-file-r-9-1".into(),
            payload: json!({"kind": "set_property"}),
            base_revision: 42,
            steps: Vec::new(),
            on: true,
            sent: false,
            label: "离线改动".into(),
            restored: true,
            user_id: None,
            queued_at: None,
            inverse: None,
            inverse_read: true,
        };
        state.seed_file_state(&claim, vec![restored], Vec::new());
        assert!(state.had_session);
        assert_eq!(state.base_revision, 42);
        assert_eq!(state.repo_id.as_deref(), Some("r-9"));
        assert_eq!(state.outbox.len(), 1);
        // An edit made offline, before any session of this run, queues.
        assert!(matches!(
            state.queue_op(json!({"kind": "set_property"}), Vec::new(), true, "再改"),
            Ok(Queued::Offline(2))
        ));
        // Seeding is once: a second call would have been a no-op anyway.
        state.seed_file_state(&claim, Vec::new(), Vec::new());
        assert_eq!(state.outbox.len(), 2);

        // The refusal is said ahead of the session it comes with.
        state.apply(CollabEvent::AlignRefused {
            code: "REPO_MISMATCH".into(),
            message: "这份文件属于另一个服务器库".into(),
        });
        state.apply(CollabEvent::Connected {
            session_id: "s-9".to_owned(),
            base_revision: 50,
            peers: Vec::new(),
            claims: Vec::new(),
            repo_id: "r-other".into(),
        });
        assert_eq!(
            state.not_aligned.as_ref().map(|apart| apart.code.as_str()),
            Some("REPO_MISMATCH")
        );
        assert_eq!(state.diverged, None, "held apart, not parted for good");
        assert_eq!(
            (state.repo_id.as_deref(), state.base_revision),
            (Some("r-9"), 42),
            "the room that refused the copy is not the copy's"
        );
        assert!(matches!(
            state.queue_op(json!({"kind": "set_property"}), Vec::new(), true, "再改"),
            Ok(Queued::NotAligned(3))
        ));
        assert!(
            state.outbox.iter().all(|entry| !entry.sent),
            "nothing of the queue goes out"
        );
    }

    fn applied(op_id: &str, revision: i64) -> CollabEvent {
        CollabEvent::OpApplied {
            op_id: op_id.to_owned(),
            revision,
            changes: vec![serde_json::json!({
                "kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa",
            })],
        }
    }

    /// The room's ops arrive one revision after the other. One that is not
    /// the next means a change in between never got here: the session is
    /// marked `REVISION_GAP`, sends nothing on top of a drawing it knows to
    /// be short, and its baseline stays the last revision it has all of.
    #[test]
    fn a_revision_that_skips_one_marks_the_session_and_stops_its_ops() {
        let mut state = joined_at(3);
        state.apply(applied("op-a", 4));
        assert_eq!(state.diverged, None);
        assert!(state
            .request_op(serde_json::json!({"kind": "set_tag"}))
            .is_ok());

        state.apply(applied("op-c", 6));
        assert_eq!(state.base_revision, 4, "the baseline does not pass the gap");
        let reason = state.diverged.clone().expect("marked");
        assert!(reason.contains('6') && reason.contains('4'), "{reason}");
        let code = state.diverged_as.map(|(code, _)| code);
        assert_eq!(code, Some(crate::pid::collab_state::REVISION_GAP));
        assert_eq!(state.status_json()["diverged"], reason.as_str());
        let refused = state
            .request_op(serde_json::json!({"kind": "set_tag"}))
            .expect_err("no op goes out from a drawing known to be short");
        assert!(refused.contains("不一致"), "{refused}");

        // The first reason stays: it is where the two parted.
        state.apply(applied("op-e", 9));
        assert_eq!(state.diverged.as_deref(), Some(reason.as_str()));
        assert_eq!(state.base_revision, 4);
    }

    /// A revision this drawing is already at is not applied a second time,
    /// and does not take the baseline back.
    #[test]
    fn a_revision_already_seen_is_dropped() {
        let mut state = joined_at(3);
        state.apply(applied("op-a", 4));
        state.apply(applied("op-b", 5));
        assert_eq!(state.pending_remote.len(), 2);

        state.apply(applied("op-a", 4));
        state.apply(applied("op-old", 2));
        assert_eq!(state.pending_remote.len(), 2, "nothing queued twice");
        assert_eq!(state.base_revision, 5);
        assert_eq!(state.applied_ops, 2);
        assert_eq!(state.diverged, None);
    }

    /// Coming back to a room that moved on while the session was away, with
    /// no replay: what it applied meanwhile is not in this drawing, so the
    /// connection is held apart (`OFF-2`) — the drawing keeps its own
    /// baseline, and is not parted from the room for good. Coming back to
    /// the same revision, nothing was missed; to a lower one, the server
    /// started over and this is a first join again.
    #[test]
    fn coming_back_to_a_room_that_moved_on_holds_the_connection_apart() {
        let reconnect_at = |revision: i64| {
            let mut state = joined_at(3);
            state.apply(CollabEvent::Disconnected {
                reason: "read error: ConnectionReset".to_owned(),
                will_retry: true,
            });
            state.apply(CollabEvent::Connected {
                session_id: "s-2".to_owned(),
                base_revision: revision,
                repo_id: "r-test".into(),
                peers: Vec::new(),
                claims: Vec::new(),
            });
            state
        };
        let same = reconnect_at(3);
        assert_eq!(
            (same.not_aligned.clone(), same.diverged.clone()),
            (None, None)
        );
        let restarted = reconnect_at(0);
        assert_eq!(
            (restarted.not_aligned.clone(), restarted.base_revision),
            (None, 0)
        );
        let moved_on = reconnect_at(5);
        let apart = moved_on.not_aligned.clone().expect("held apart");
        assert_eq!(apart.code, "REPLAY_GAP");
        assert!(
            apart.message.contains('3') && apart.message.contains('5'),
            "{apart:?}"
        );
        assert_eq!(
            moved_on.base_revision, 3,
            "the room's baseline is not taken"
        );
        assert_eq!(moved_on.diverged, None);
        // The first join of a run is taken at whatever the room is at.
        let first = joined_at(7);
        assert_eq!(
            (first.not_aligned.clone(), first.diverged.clone()),
            (None, None)
        );
    }
}

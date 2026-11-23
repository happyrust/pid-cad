//! Steps made while the session is away wait their turn and go out when a
//! session is up again; what the room did meanwhile comes first; a step the
//! server refuses is taken back (`VAL-CROSS-008`, plan §2 of
//! `docs/plans/2026-11-12-collab-completion.md`).
//!
//! Editors in one process with a stand-in for the server between them: the
//! server's own judgment of each change (`ocs_collab::conflict::
//! DrawingModel`, the code the server runs) under the server's rule for an
//! op as a whole (plan §2.3, which the server's own tests hold the server
//! to). After each exchange the drawings are compared with each other and
//! with what the room has.

use crate::app::collab::{messages, CollabEvent, Outgoing, Refused};
use crate::app::OpenCADStudio;
use crate::pid::collab_state::REMOTE_UNAPPLIED;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use document::entities::Insert;
use document::types::Vector3;
use document::{EntityType, Handle};
use ocs_collab::conflict::{Change as Judged, DrawingModel, Verdict};
use ocs_collab::protocol::codes;
use serde_json::{json, Value};
use std::collections::BTreeMap;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const PUMP: &str = "P-1001";
const VESSEL: &str = "V-1001";
const PRESSURE: &str = "设计压力";
const TEMPERATURE: &str = "设计温度";

/// An editor on a new drawing with a pump and a vessel on it — the same
/// drawing in every editor of a test, as if each had opened the same file.
fn editor(user: &str) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    draw(
        &mut app,
        PUMP,
        0.0,
        r#"{"设计压力":"1.0MPa","设计温度":"80℃"}"#,
    );
    draw(&mut app, VESSEL, 40.0, "{}");
    app.collab.user_id = user.to_owned();
    app
}

/// An object put straight onto the drawing in front: no step of anyone's,
/// and nothing the room hears of.
fn draw(app: &mut OpenCADStudio, tag: &str, x: f64, props: &str) {
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    let handle = document
        .add_entity(EntityType::Insert(Insert::new(
            "PID_SYM_equipment_vessel",
            Vector3::new(x, 0.0, 0.0),
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
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
        .unwrap_or_else(|| panic!("no object tagged {tag}"))
}

fn author(app: &OpenCADStudio, tag: &str) -> PidAuthor {
    PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle_of(app, tag)).expect("author")
}

fn prop(app: &OpenCADStudio, tag: &str, field: &str) -> String {
    split_props(&author(app, tag).props)
        .0
        .get(field)
        .cloned()
        .unwrap_or_default()
}

/// Every tagged object of the drawing in front with what it says.
fn picture(app: &OpenCADStudio) -> BTreeMap<String, BTreeMap<String, String>> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .map(|author| (author.tag.clone(), split_props(&author.props).0))
        .collect()
}

/// One step of this person's: `fields` of the object tagged `tag` written
/// through the contract's own write channel for properties (`xdata_set`
/// carrying the whole `props`).
fn set(app: &mut OpenCADStudio, tag: &str, fields: &[(&str, &str)]) {
    let handle = handle_of(app, tag);
    let mut props: serde_json::Map<String, Value> =
        serde_json::from_str(&author(app, tag).props).unwrap_or_default();
    for (field, value) in fields {
        props.insert((*field).to_owned(), json!(value));
    }
    let props = Value::Object(props).to_string();
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (response, task) = app.control_request(json!({
        "protocol": 1,
        "op": "xdata_set",
        "request_id": format!("collab-queue-{serial}"),
        "client_id": "test",
        "document_id": state["document_id"],
        "revision": state["revision"],
        "handles": [format!("{:X}", handle.value())],
        "app": "PID_AUTHOR",
        "data": [{"code": 1000, "value": format!("props={props}")}],
    }));
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    assert_eq!(response["ok"], true, "{response}");
    app.finish_all_pending_history();
}

fn undo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"undo"}"#);
}

fn redo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"redo"}"#);
}

/// `(steps that can be taken back, steps that can be put on again)`.
fn stacks(app: &OpenCADStudio) -> (usize, usize) {
    let history = &app.tabs[app.active_tab].history;
    (history.undo_stack.len(), history.redo_stack.len())
}

/// What the command line has said.
fn said(app: &OpenCADStudio) -> String {
    app.command_line.history_plain_text()
}

/// One event from the socket, taken the way the app takes it
/// (`Message::CollabEvent`): into the session's state, then onto the
/// drawing.
fn hear(app: &mut OpenCADStudio, event: CollabEvent) {
    app.collab.apply(event);
    app.apply_remote_changes();
}

/// The room: the server's picture of the drawing, its revision, what it has
/// told so far, and the claims only it knows of.
#[derive(Default)]
struct Room {
    model: DrawingModel,
    revision: i64,
    /// Every `op_applied` it told, oldest first: `(revision, op_id,
    /// changes)` — its `ops` table, which a replay reads.
    log: Vec<(i64, String, Vec<Value>)>,
    /// tag → the session holding it.
    claims: BTreeMap<String, String>,
}

/// What the sender of an op hears back.
struct Answer {
    /// `(code, message, rejected)` of each `error`.
    errors: Vec<(String, String, bool)>,
    /// The `op_applied`, when anything took effect.
    applied: Option<(i64, Vec<Value>)>,
}

impl Room {
    /// What the server makes of one op (plan §2.3): a claim somebody else
    /// holds refuses it whole; an op that carries anything but field writes
    /// takes effect whole or not at all; of an op of field writes, each is
    /// judged for itself, and the op is refused whole only when none took
    /// effect.
    fn take(&mut self, session: &str, base: i64, op_id: &str, payload: &Value) -> Answer {
        let changes = Judged::list(payload);
        assert!(
            !changes.is_empty() && changes.iter().all(Judged::is_understood),
            "the server has no rule for {payload}"
        );
        assert!(
            base <= self.revision,
            "an op against revision {base} of a room at {}: the server answers STALE_BASE_REVISION",
            self.revision
        );
        let held = changes
            .iter()
            .map(Judged::subject)
            .find(|tag| self.claims.get(tag).is_some_and(|holder| holder != session));
        if let Some(tag) = held {
            return Answer {
                errors: vec![(
                    codes::CLAIM_HELD.to_owned(),
                    format!("位号 {tag} 已被占用"),
                    true,
                )],
                applied: None,
            };
        }
        let structural = changes.iter().any(|change| !change.writes_a_field());
        let mut refused = Vec::new();
        let mut taken = Vec::new();
        for change in changes {
            let change = self.model.completed(&change);
            match self.model.judge(&change, base, session) {
                Verdict::Apply { object_ref, .. } => taken.push((change, object_ref)),
                Verdict::Yield {
                    object_ref, winner, ..
                } => refused.push((
                    codes::OP_SUPERSEDED.to_owned(),
                    format!("{object_ref} 的 {} 已被改为 {winner}", change.field),
                )),
                Verdict::Gone { object_ref, .. } => refused.push((
                    codes::OBJECT_DELETED.to_owned(),
                    format!("{object_ref} 已被删除"),
                )),
                Verdict::Referenced { object_ref, .. } => refused.push((
                    codes::OBJECT_REFERENCED.to_owned(),
                    format!("{object_ref} 仍被引用"),
                )),
                other @ (Verdict::BehindTopology { .. } | Verdict::DuplicateInflow { .. }) => {
                    panic!("these tests draw no pipes: {other:?}")
                }
            }
        }
        let whole = !refused.is_empty() && (structural || taken.is_empty());
        let errors = refused
            .into_iter()
            .map(|(code, message)| (code, message, whole))
            .collect();
        if whole {
            return Answer {
                errors,
                applied: None,
            };
        }
        let revision = self.revision + 1;
        let mut told = Vec::new();
        for (change, object_ref) in taken {
            self.model
                .commit(&change, &object_ref, revision, session, session);
            told.push(change.broadcast_form(&object_ref));
        }
        self.revision = revision;
        self.log.push((revision, op_id.to_owned(), told.clone()));
        Answer {
            errors,
            applied: Some((revision, told)),
        }
    }

    /// What the room has for a field.
    fn value(&self, tag: &str, field: &str) -> Option<String> {
        self.model.field(tag, field).map(|state| state.value.clone())
    }
}

/// The first session of `app`'s: it joins the room as it is.
fn join(app: &mut OpenCADStudio, session: &str, room: &Room) {
    hear(
        app,
        CollabEvent::Connected {
            session_id: session.to_owned(),
            base_revision: room.revision,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        },
    );
}

/// The socket is gone.
fn drop_session(app: &mut OpenCADStudio) {
    hear(
        app,
        CollabEvent::Disconnected {
            reason: "connection reset".to_owned(),
            will_retry: true,
        },
    );
}

/// A new session of `app`'s, welcomed with a replay from the revision its
/// drawing has. The replay itself is `replay_one`.
fn welcome_back(app: &mut OpenCADStudio, session: &str, room: &Room) {
    let from = app.collab.base_revision;
    hear(app, CollabEvent::Resuming { from });
    hear(
        app,
        CollabEvent::Connected {
            session_id: session.to_owned(),
            base_revision: room.revision,
            repo_id: "r-test".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        },
    );
}

/// The next op the room has told that `app` has not heard: `false` when it
/// has heard them all.
fn replay_one(app: &mut OpenCADStudio, room: &Room) -> bool {
    let next = room
        .log
        .iter()
        .find(|(revision, ..)| *revision > app.collab.base_revision)
        .cloned();
    match next {
        Some((revision, op_id, changes)) => {
            hear(
                app,
                CollabEvent::OpApplied {
                    op_id,
                    revision,
                    changes,
                },
            );
            true
        }
        None => false,
    }
}

/// A new session with everything it missed.
fn come_back(app: &mut OpenCADStudio, session: &str, room: &Room) {
    welcome_back(app, session, room);
    while replay_one(app, room) {}
}

/// The op `app` has on the wire, as the server reads it: `(op_id,
/// base_revision, payload)`.
fn on_the_wire(app: &OpenCADStudio) -> Option<(String, i64, Value)> {
    let mut sent = app.collab.outbox.iter().filter(|entry| entry.sent);
    let first = sent.next()?;
    assert!(sent.next().is_none(), "more than one op on the wire");
    Some((first.op_id.clone(), first.base_revision, first.payload.clone()))
}

/// The room answers the op `from` has on the wire: `from` hears the answer,
/// and every one of `others` that is in a session hears what the room was
/// told. Returns the codes of the errors in the answer.
fn exchange(
    room: &mut Room,
    from: &mut OpenCADStudio,
    others: &mut [&mut OpenCADStudio],
) -> Vec<String> {
    let session = from.collab.session_id.clone().expect("a session");
    let (op_id, base, payload) = on_the_wire(from).expect("nothing on the wire");
    let answer = room.take(&session, base, &op_id, &payload);
    let said: Vec<String> = answer.errors.iter().map(|(code, ..)| code.clone()).collect();
    for (code, message, rejected) in answer.errors {
        hear(
            from,
            CollabEvent::OpRefused {
                op_id: op_id.clone(),
                code,
                message,
                rejected,
            },
        );
    }
    if let Some((revision, changes)) = answer.applied {
        hear(
            from,
            CollabEvent::OpApplied {
                op_id: op_id.clone(),
                revision,
                changes: changes.clone(),
            },
        );
        for other in others.iter_mut().filter(|other| other.collab.connected) {
            hear(
                other,
                CollabEvent::OpApplied {
                    op_id: op_id.clone(),
                    revision,
                    changes: changes.clone(),
                },
            );
            assert_eq!(other.collab.diverged, None, "a peer could not follow");
        }
    }
    said
}

/// Two editors in the room, each in its first session.
fn two_in_the_room() -> (OpenCADStudio, OpenCADStudio, Room) {
    let (mut a, mut b) = (editor("u1"), editor("u2"));
    let room = Room::default();
    join(&mut a, "s-a1", &room);
    join(&mut b, "s-b1", &room);
    (a, b, room)
}

// ── Steps made with no session up ────────────────────────────────────────

/// `VAL-CROSS-008`: what is done with the session away is kept, in order,
/// and goes out when a session is up again — one op at a time, the next
/// when the room has answered the one before. It used to be dropped where
/// it was made, and the two drawings differed from then on.
#[test]
fn steps_made_with_the_session_away_wait_and_go_out_in_order_one_at_a_time() {
    let (mut a, mut b, mut room) = two_in_the_room();

    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut a, PUMP, &[(TEMPERATURE, "120℃")]);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    assert_eq!(a.collab.outbox.len(), 3, "each step waits");
    assert!(a.collab.outbox.iter().all(|entry| !entry.sent));
    assert!(said(&a).contains("已排队"), "{}", said(&a));
    assert_eq!(a.collab.status_json()["queued_ops"], 3);
    assert_eq!(room.revision, 0, "nothing has reached the room");
    assert_eq!(prop(&b, PUMP, PRESSURE), "1.0MPa");

    come_back(&mut a, "s-a2", &room);
    for left in [3, 2, 1] {
        assert_eq!(a.collab.outbox.len(), left);
        assert!(
            a.collab.outbox.front().is_some_and(|entry| entry.sent),
            "the oldest is the one on the wire"
        );
        assert_eq!(
            a.collab.outbox.iter().filter(|entry| entry.sent).count(),
            1,
            "one op on the wire at a time"
        );
        assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    }
    assert!(a.collab.outbox.is_empty());
    assert_eq!(a.collab.status_json()["queued_ops"], 0);

    // In the order they were made.
    let told: Vec<(String, String)> = room
        .log
        .iter()
        .map(|(_, _, changes)| {
            (
                changes[0]["object_ref"].as_str().unwrap_or_default().to_owned(),
                changes[0]["field"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        told,
        [
            (PUMP.to_owned(), PRESSURE.to_owned()),
            (PUMP.to_owned(), TEMPERATURE.to_owned()),
            (VESSEL.to_owned(), PRESSURE.to_owned()),
        ]
    );
    assert_eq!(picture(&a), picture(&b), "the peer has all three");
    assert_eq!(room.value(VESSEL, PRESSURE).as_deref(), Some("0.6MPa"));
    assert_eq!(a.collab.diverged, None);
}

/// A client that has never been in a session has nobody to tell
/// (`VAL-CLI-017`): its steps are its own, nothing waits, and when it joins
/// its drawing is taken to be what the room has — as before.
#[test]
fn a_client_that_never_joined_queues_nothing() {
    let mut a = editor("u1");
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    assert!(a.collab.outbox.is_empty());
    assert!(a.collab.sent.is_empty());
    assert!(!said(&a).contains("协同"), "{}", said(&a));

    let room = Room::default();
    join(&mut a, "s-a1", &room);
    assert!(a.collab.outbox.is_empty());
    assert!(on_the_wire(&a).is_none());
}

/// `VAL-CROSS-008`: both away, each changes a different field of one
/// object. Whoever is back first sends; the other is first told what the
/// room did meanwhile — the server replays it, from the revision the
/// drawing has — and only then sends its own, against the revision it was
/// made at. The two fields merge and the two drawings are the same.
#[test]
fn what_the_room_did_meanwhile_arrives_before_anything_is_sent() {
    let (mut a, mut b, mut room) = two_in_the_room();
    drop_session(&mut a);
    drop_session(&mut b);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut b, PUMP, &[(TEMPERATURE, "120℃")]);

    come_back(&mut b, "s-b2", &room);
    assert_eq!(exchange(&mut room, &mut b, &mut [&mut a]), Vec::<String>::new());
    assert_eq!(room.revision, 1);
    assert_eq!(a.collab.base_revision, 0, "the one still away has not heard");

    welcome_back(&mut a, "s-a2", &room);
    assert_eq!(a.collab.catching_up, Some(1));
    assert_eq!(a.collab.status_json()["catching_up"], 1);
    assert_eq!(a.collab.base_revision, 0, "the baseline is where the drawing is");
    assert!(on_the_wire(&a).is_none(), "nothing goes out before the replay is through");
    assert_eq!(a.collab.diverged, None, "a replay is not a gap");

    assert!(replay_one(&mut a, &room));
    assert_eq!(prop(&a, PUMP, TEMPERATURE), "120℃", "the peer's change is here");
    assert_eq!(a.collab.base_revision, 1);
    assert_eq!(a.collab.catching_up, None);
    let (_, base, _) = on_the_wire(&a).expect("its own step goes out now");
    assert_eq!(base, 0, "against the revision it was made at");

    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    assert_eq!(room.revision, 2);
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(prop(&b, PUMP, PRESSURE), "1.6MPa");
    assert_eq!(prop(&a, PUMP, TEMPERATURE), "120℃");
    assert_eq!(room.value(PUMP, PRESSURE).as_deref(), Some("1.6MPa"));
    assert_eq!(room.value(PUMP, TEMPERATURE).as_deref(), Some("120℃"));
    assert_eq!((a.collab.diverged.clone(), b.collab.diverged.clone()), (None, None));
}

/// The room took an op and the socket dropped before the answer came. The
/// replay names it; it is not applied to the drawing it came from, and not
/// sent a second time. A step made on top of it afterwards does not give
/// way to it — the server reads the earlier session as somebody else.
#[test]
fn an_op_the_room_took_before_the_socket_dropped_is_not_sent_again() {
    let (mut a, mut b, mut room) = two_in_the_room();
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    let (op_id, base, payload) = on_the_wire(&a).expect("on the wire");
    let answer = room.take("s-a1", base, &op_id, &payload);
    let (revision, changes) = answer.applied.expect("the room takes it");
    hear(
        &mut b,
        CollabEvent::OpApplied {
            op_id: op_id.clone(),
            revision,
            changes,
        },
    );
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "2.5MPa")]);
    assert_eq!(a.collab.outbox.len(), 2, "one unanswered, one waiting");

    welcome_back(&mut a, "s-a2", &room);
    assert!(on_the_wire(&a).is_none());
    let peers_ops = a.collab.applied_ops;
    assert!(replay_one(&mut a, &room));
    assert_eq!(a.collab.applied_ops, peers_ops, "its own op is not a peer's");
    assert_eq!(
        prop(&a, PUMP, PRESSURE),
        "2.5MPa",
        "its own earlier op is not written over its later step"
    );
    assert_eq!(a.collab.outbox.len(), 1, "the op the room took has left the queue");
    let (_, base, _) = on_the_wire(&a).expect("the later step goes out");
    assert_eq!(base, 1, "it was made on top of the op at revision 1");

    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    assert_eq!(room.revision, 2, "two steps, two revisions: each took effect once");
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(room.value(PUMP, PRESSURE).as_deref(), Some("2.5MPa"));
}

/// A step made before another's op — not on top of one's own — keeps the
/// baseline it was made at: what the other did in between is something it
/// has not seen, and the server is to judge it so.
#[test]
fn only_a_step_made_right_on_top_of_ones_own_op_takes_its_revision() {
    let (mut a, mut b, mut room) = two_in_the_room();
    // b's op first, which a does not hear; then a's, which the room takes
    // at revision 2 without a hearing of it either.
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    let (op_id, base, payload) = on_the_wire(&a).expect("on the wire");
    set(&mut b, VESSEL, &[(PRESSURE, "0.6MPa")]);
    let (b_op, b_base, b_payload) = on_the_wire(&b).expect("on the wire");
    assert!(room.take("s-b1", b_base, &b_op, &b_payload).applied.is_some());
    assert!(room.take("s-a1", base, &op_id, &payload).applied.is_some());
    drop_session(&mut a);
    set(&mut a, PUMP, &[(TEMPERATURE, "120℃")]);

    come_back(&mut a, "s-a2", &room);
    let (_, base, _) = on_the_wire(&a).expect("the later step goes out");
    assert_eq!(
        base, 0,
        "revision 1 was somebody else's and came between: the step has not seen it"
    );
    assert_eq!(prop(&a, VESSEL, PRESSURE), "0.6MPa", "the replay brought it");
}

/// Back without a replay — a server that keeps no log — to a room that
/// moved on: what happened in between cannot be had, the session says so,
/// and the queue is kept and not sent (it would be written on top of the
/// gap). It is the connection that is held apart (`OFF-2`), not the drawing
/// parted for good: the next connection is judged afresh.
#[test]
fn back_to_a_room_that_moved_on_with_no_replay_the_queue_stays_where_it_is() {
    let (mut a, mut b, mut room) = two_in_the_room();
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut b, PUMP, &[(TEMPERATURE, "120℃")]);
    assert_eq!(
        exchange(&mut room, &mut b, &mut [&mut a]),
        Vec::<String>::new()
    );

    join(&mut a, "s-a2", &room);
    let apart = a.collab.not_aligned.clone().expect("the gap is known");
    assert_eq!(apart.code, codes::REPLAY_GAP);
    assert_eq!(a.collab.diverged, None);
    assert!(on_the_wire(&a).is_none());
    assert_eq!(a.collab.outbox.len(), 1);
    assert_eq!(a.collab.status_json()["queued_ops"], 1);
}

/// What the room did meanwhile is written into the drawing before any of
/// the queue goes out (`SPEC-collab-offline-sync.md` §5.3): heard is not
/// enough, the drawing is not where the room is until then.
#[test]
fn the_queue_waits_for_the_rooms_changes_to_be_written_not_just_heard() {
    let (mut a, mut b, mut room) = two_in_the_room();
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut b, VESSEL, &[(PRESSURE, "0.6MPa")]);
    assert_eq!(
        exchange(&mut room, &mut b, &mut [&mut a]),
        Vec::<String>::new()
    );
    welcome_back(&mut a, "s-a2", &room);

    // The replay in the two halves the app takes it in (`hear`): into the
    // session's state, then onto the drawing.
    let (revision, op_id, changes) = room.log[0].clone();
    a.collab.apply(CollabEvent::OpApplied {
        op_id,
        revision,
        changes,
    });
    assert_eq!(a.collab.catching_up, None, "the replay is through");
    assert_eq!(prop(&a, VESSEL, PRESSURE), "", "and not written yet");
    assert!(on_the_wire(&a).is_none(), "nothing goes out before it is");

    a.apply_remote_changes();
    assert_eq!(prop(&a, VESSEL, PRESSURE), "0.6MPa");
    let (_, base, _) = on_the_wire(&a).expect("its own step goes out now");
    assert_eq!(base, 0, "against the revision it was made at");
    assert_eq!(
        exchange(&mut room, &mut a, &mut [&mut b]),
        Vec::<String>::new()
    );
    assert_eq!(picture(&a), picture(&b));
}

/// What the room did meanwhile and this drawing cannot follow — here a
/// change to an object it does not have — parts it from the room, and the
/// queue stays where it is: none of it goes out on top of the difference
/// (OS-10). It used to go out as soon as the replay had been heard, before
/// the change was found to be one the drawing could not take.
#[test]
fn a_replay_the_drawing_cannot_follow_sends_none_of_the_queue() {
    let (mut a, mut b, mut room) = two_in_the_room();
    draw(&mut b, "V-9001", 80.0, "{}");
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut b, "V-9001", &[(PRESSURE, "0.6MPa")]);
    assert_eq!(
        exchange(&mut room, &mut b, &mut [&mut a]),
        Vec::<String>::new()
    );

    come_back(&mut a, "s-a2", &room);
    assert_eq!(
        a.collab.diverged_as.map(|(code, _)| code),
        Some(REMOTE_UNAPPLIED)
    );
    assert!(on_the_wire(&a).is_none(), "none of the queue goes out");
    assert_eq!(a.collab.status_json()["queued_ops"], 1);
    assert_eq!(
        prop(&a, PUMP, PRESSURE),
        "1.6MPa",
        "the step stays on the drawing"
    );
}

/// A step of this person's still open when the replay arrives is closed
/// before the room's changes are written, as theirs, and goes to the back
/// of the queue: it does not send the queue out ahead of them. When they
/// cannot be written, none of the queue goes out.
#[test]
fn a_step_left_open_over_a_replay_sends_none_of_the_queue_ahead_of_it() {
    let (mut a, mut b, mut room) = two_in_the_room();
    draw(&mut b, "V-9001", 80.0, "{}");
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut b, "V-9001", &[(PRESSURE, "0.6MPa")]);
    assert_eq!(
        exchange(&mut room, &mut b, &mut [&mut a]),
        Vec::<String>::new()
    );
    welcome_back(&mut a, "s-a2", &room);

    let tab = a.active_tab;
    let pump = handle_of(&a, PUMP);
    a.push_undo_snapshot(tab, "TEST");
    {
        let document = &mut a.tabs[tab].scene.document;
        let mut record = PidAuthor::load(document, pump).expect("author");
        record.props = r#"{"设计压力":"1.6MPa","设计温度":"120℃"}"#.to_owned();
        record.store(document, pump);
    }

    assert!(replay_one(&mut a, &room));
    assert_eq!(
        a.collab.diverged_as.map(|(code, _)| code),
        Some(REMOTE_UNAPPLIED)
    );
    assert_eq!(stacks(&a), (2, 0), "the open step was closed");
    assert_eq!(a.collab.outbox.len(), 2, "and queued behind the other");
    assert!(on_the_wire(&a).is_none(), "none of the queue goes out");
}

// ── A step the server refuses ────────────────────────────────────────────

/// `VAL-CROSS-008`: the server refuses an op whole — here for a claim this
/// client had not heard of — and the step it spoke for is taken back: the
/// drawing is what the room has, the step can be put on again, and the
/// command line says which step, why, and under which code. It used to
/// stay, with a line on the command line and the drawings different.
#[test]
fn a_step_the_server_refuses_is_taken_back() {
    let (mut a, b, mut room) = two_in_the_room();
    room.claims.insert(VESSEL.to_owned(), "s-b1".to_owned());
    let before = picture(&a);

    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    assert_eq!(stacks(&a), (1, 0));
    assert_eq!(exchange(&mut room, &mut a, &mut []), [codes::CLAIM_HELD]);

    assert_eq!(picture(&a), before, "the drawing is as the room has it");
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(stacks(&a), (0, 1), "taken back, and it can be put on again");
    assert!(a.collab.outbox.is_empty());
    assert_eq!(room.revision, 0);
    assert_eq!(a.collab.diverged, None);
    let words = said(&a);
    assert!(words.contains(codes::CLAIM_HELD), "{words}");
    assert!(words.contains("已撤回"), "{words}");
    let status = a.collab.status_json();
    let logged = &status["rejections"];
    assert_eq!(logged.as_array().map(Vec::len), Some(1), "{logged}");
    assert_eq!(logged[0]["code"], codes::CLAIM_HELD, "{logged}");
    assert_eq!(logged[0]["steps_moved"], 1, "{logged}");

    // Nothing was said to the room for the taking back, and the next step
    // goes out as any other.
    assert!(on_the_wire(&a).is_none());
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    assert!(on_the_wire(&a).is_some());
}

/// What was done on top of the refused step goes back with it — the room
/// had seen none of it, one op being on the wire at a time — and the ops
/// that spoke for it are not sent. Put on again, the steps are new ops.
#[test]
fn the_steps_made_on_top_of_a_refused_step_go_back_with_it() {
    let (mut a, mut b, mut room) = two_in_the_room();
    room.claims.insert(PUMP.to_owned(), "s-b1".to_owned());
    let before = picture(&a);

    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    set(&mut a, VESSEL, &[(TEMPERATURE, "60℃")]);
    assert_eq!(a.collab.outbox.len(), 3);
    assert!(said(&a).contains("已排队"), "{}", said(&a));
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::CLAIM_HELD]);

    assert_eq!(picture(&a), before);
    assert_eq!(stacks(&a), (0, 3));
    assert!(a.collab.outbox.is_empty(), "the two behind it are not sent");
    assert!(on_the_wire(&a).is_none());
    assert_eq!(room.revision, 0);
    let status = a.collab.status_json();
    let logged = &status["rejections"][0];
    assert_eq!(logged["steps_moved"], 3, "{logged}");
    assert_eq!(logged["ops_dropped"], 2, "{logged}");
    assert!(said(&a).contains("可以重做"), "{}", said(&a));

    // The claim is let go of; the three steps are put on again, and go to
    // the room as three new ops.
    room.claims.clear();
    for _ in 0..3 {
        redo(&mut a);
    }
    assert_eq!(stacks(&a), (3, 0));
    while on_the_wire(&a).is_some() {
        assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    }
    assert_eq!(room.revision, 3);
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(prop(&b, VESSEL, TEMPERATURE), "60℃");
}

/// The refused step had been taken back by hand before the answer came:
/// the drawing is what the room has already, nothing moves, and the op
/// that spoke for the taking back is not sent — the room never had the
/// step.
#[test]
fn a_refused_step_already_taken_back_by_hand_needs_nothing() {
    let (mut a, _b, mut room) = two_in_the_room();
    room.claims.insert(PUMP.to_owned(), "s-b1".to_owned());
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    undo(&mut a);
    assert_eq!(a.collab.outbox.len(), 2, "the step, and its being taken back");
    assert_eq!(stacks(&a), (0, 1));

    assert_eq!(exchange(&mut room, &mut a, &mut []), [codes::CLAIM_HELD]);
    assert_eq!(prop(&a, PUMP, PRESSURE), "1.0MPa");
    assert_eq!(stacks(&a), (0, 1), "nothing moved");
    assert!(a.collab.outbox.is_empty());
    assert_eq!(room.revision, 0);
    assert_eq!(a.collab.diverged, None);
    let status = a.collab.status_json();
    assert_eq!(status["rejections"][0]["steps_moved"], 0, "{status}");
}

/// A step taken back, and the server refuses that: the step is put on
/// again, and the drawing is what the room still has.
#[test]
fn a_refused_taking_back_is_put_on_again() {
    let (mut a, mut b, mut room) = two_in_the_room();
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    room.claims.insert(PUMP.to_owned(), "s-b1".to_owned());

    undo(&mut a);
    assert_eq!(prop(&a, PUMP, PRESSURE), "1.0MPa");
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::CLAIM_HELD]);

    assert_eq!(prop(&a, PUMP, PRESSURE), "1.6MPa", "the room still has it");
    assert_eq!(stacks(&a), (1, 0));
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(room.value(PUMP, PRESSURE).as_deref(), Some("1.6MPa"));
    assert!(said(&a).contains("已恢复"), "{}", said(&a));
    assert!(on_the_wire(&a).is_none(), "nothing is said to the room for it");
}

/// `VAL-CROSS-008`, the other ending: away, this person writes a field
/// somebody else writes meanwhile. Back, the step is behind and gives way
/// (`OP_SUPERSEDED`): it is taken back, and the field is what the other
/// wrote — not what it was before either of them.
#[test]
fn a_field_somebody_else_wrote_meanwhile_ends_at_their_value() {
    let (mut a, mut b, mut room) = two_in_the_room();
    drop_session(&mut a);
    set(&mut b, PUMP, &[(PRESSURE, "2.5MPa")]);
    assert_eq!(exchange(&mut room, &mut b, &mut [&mut a]), Vec::<String>::new());
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);

    come_back(&mut a, "s-a2", &room);
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::OP_SUPERSEDED]);

    assert_eq!(prop(&a, PUMP, PRESSURE), "2.5MPa", "the value that won");
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(room.value(PUMP, PRESSURE).as_deref(), Some("2.5MPa"));
    assert_eq!(room.revision, 1);
    assert_eq!(stacks(&a), (0, 1));
    assert!(said(&a).contains(codes::OP_SUPERSEDED), "{}", said(&a));
    assert_eq!(a.collab.diverged, None);
}

/// One step, two fields; one of them gives way and the other takes
/// effect. The step stays — the op was not refused — and the field that
/// gave way is the other person's value, which the replay brought.
#[test]
fn an_op_that_gives_way_in_part_keeps_its_step() {
    let (mut a, mut b, mut room) = two_in_the_room();
    drop_session(&mut a);
    set(&mut b, PUMP, &[(PRESSURE, "2.5MPa")]);
    assert_eq!(exchange(&mut room, &mut b, &mut [&mut a]), Vec::<String>::new());
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa"), (TEMPERATURE, "120℃")]);

    come_back(&mut a, "s-a2", &room);
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::OP_SUPERSEDED]);

    assert_eq!(stacks(&a), (1, 0), "the step is not taken back");
    assert_eq!(room.revision, 2, "the other field took effect");
    assert_eq!(prop(&a, PUMP, PRESSURE), "2.5MPa");
    assert_eq!(prop(&a, PUMP, TEMPERATURE), "120℃");
    assert_eq!(picture(&a), picture(&b));
    assert!(a.collab.outbox.is_empty());
    assert!(a.collab.status_json()["rejections"].as_array().is_some_and(Vec::is_empty));
    assert!(said(&a).contains(codes::OP_SUPERSEDED), "{}", said(&a));
}

/// A server from before `error.op_id` refuses an op without naming it.
/// With one op on the wire it can only mean that one: it leaves the queue,
/// the next goes out — and the step stays, as it always did with such a
/// server, which gave no word on whether any of it took effect.
#[test]
fn an_error_that_names_no_op_takes_nothing_back() {
    let (mut a, _b, _room) = two_in_the_room();
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    let (first, ..) = on_the_wire(&a).expect("on the wire");

    hear(
        &mut a,
        CollabEvent::ServerError {
            code: codes::OP_SUPERSEDED.to_owned(),
            message: "落后".to_owned(),
        },
    );
    assert_eq!(stacks(&a), (2, 0), "nothing is taken back");
    assert_eq!(prop(&a, PUMP, PRESSURE), "1.6MPa");
    assert_eq!(a.collab.outbox.len(), 1);
    let (second, ..) = on_the_wire(&a).expect("the next goes out");
    assert_ne!(first, second);

    // An error that is not about an op leaves the queue alone.
    hear(
        &mut a,
        CollabEvent::ServerError {
            code: codes::INVALID_MESSAGE.to_owned(),
            message: "…".to_owned(),
        },
    );
    assert_eq!(on_the_wire(&a).map(|(op_id, ..)| op_id), Some(second));
}

/// Two ops tie and go into a conflict entry: neither gets an `op_applied`
/// nor an `error`. The `conflict` names them, which is the answer the queue
/// waits for; the value stays in this drawing until the ruling, as before.
#[test]
fn a_conflict_that_names_the_op_lets_the_queue_move() {
    let (mut a, _b, _room) = two_in_the_room();
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    let (first, ..) = on_the_wire(&a).expect("on the wire");

    let frame = json!({
        "type": "conflict", "conflict_id": "c-1", "object_ref": PUMP, "field": PRESSURE,
        "value_a": "1.6MPa", "value_b": "2.5MPa", "op_ids": [first, "op-s-b1-9"],
    })
    .to_string();
    let reading = messages::read(&frame);
    let messages::Frame::Event(entry) = reading.frame else {
        panic!("a conflict is an event");
    };
    hear(&mut a, entry);
    for event in reading.after {
        hear(&mut a, event);
    }

    assert_eq!(a.collab.conflicts.len(), 1);
    assert_eq!(prop(&a, PUMP, PRESSURE), "1.6MPa", "kept until the ruling");
    assert_eq!(stacks(&a), (2, 0));
    assert_eq!(a.collab.outbox.len(), 1);
    assert!(on_the_wire(&a).is_some(), "the next goes out");
}

// ── What falls with a refused op ─────────────────────────────────────────

fn queued(op_id: &str, steps: &[u64], on: bool) -> Outgoing {
    Outgoing {
        op_id: op_id.to_owned(),
        payload: json!({}),
        base_revision: 0,
        steps: steps.to_vec(),
        on,
        sent: false,
        label: String::new(),
        restored: false,
        user_id: None,
        queued_at: None,
        inverse: None,
        inverse_read: false,
    }
}

fn account(refused: &Outgoing, behind: &[Outgoing]) -> Refused {
    Refused::account(refused, &behind.iter().cloned().collect())
}

/// The ops queued behind a refused one, gone through in order: what was
/// done to the refused step or on top of it falls with it; what was done
/// once it was out of the drawing again stands.
#[test]
fn what_was_made_on_top_of_a_refused_step_falls_with_it_and_the_rest_stands() {
    let refused = queued("op-1", &[1], true);

    // Nothing behind it: the step itself goes back.
    let alone = account(&refused, &[]);
    assert_eq!((alone.in_effect, alone.void.len(), alone.mixed), (vec![1], 0, false));

    // Two steps on top, the second taken back by hand and put on again.
    let on_top = account(
        &refused,
        &[
            queued("op-2", &[2], true),
            queued("op-3", &[3], true),
            queued("op-4", &[3], false),
            queued("op-5", &[3], true),
        ],
    );
    assert_eq!(on_top.in_effect, [1, 2, 3]);
    assert_eq!(on_top.void, ["op-2", "op-3", "op-4", "op-5"]);

    // Taken back by hand, then an older step taken back and a new one
    // made: neither has anything of the refused step in it.
    let after = account(
        &refused,
        &[
            queued("op-2", &[1], false),
            queued("op-3", &[7], false),
            queued("op-4", &[9], true),
        ],
    );
    assert_eq!(after.in_effect, Vec::<u64>::new());
    assert_eq!(after.void, ["op-2"]);
    assert!(!after.mixed);

    // Made on top of it, both taken back by hand: nothing left to move,
    // and none of the three is sent.
    let all_back = account(
        &refused,
        &[
            queued("op-2", &[2], true),
            queued("op-3", &[2], false),
            queued("op-4", &[1], false),
        ],
    );
    assert_eq!(all_back.in_effect, Vec::<u64>::new());
    assert_eq!(all_back.void, ["op-2", "op-3", "op-4"]);

    // One undo across the refused step and one the room has: it cannot be
    // said in part.
    let across = account(&refused, &[queued("op-2", &[1, 7], false)]);
    assert!(across.mixed);

    // An op on the wire is not behind anything, and one that speaks for
    // no step is nobody's to judge.
    let mut wire = queued("op-2", &[2], true);
    wire.sent = true;
    let untouched = account(&refused, &[wire, queued("op-3", &[], true)]);
    assert_eq!(untouched.void, Vec::<String>::new());
}

/// The same for a refused taking back: older steps taken back after it
/// were taken back on top of it and go on again with it; the step put on
/// again by hand needs nothing.
#[test]
fn what_was_taken_back_after_a_refused_taking_back_goes_on_again_with_it() {
    let refused = queued("op-1", &[5], false);
    let further = account(&refused, &[queued("op-2", &[4], false)]);
    assert_eq!(further.in_effect, [4, 5]);
    assert_eq!(further.void, ["op-2"]);

    let by_hand = account(&refused, &[queued("op-2", &[5], true)]);
    assert_eq!(by_hand.in_effect, Vec::<u64>::new());
    assert_eq!(by_hand.void, ["op-2"]);

    // A new step made after it stands — and has cleared the redo stack the
    // refused step was on, which the app finds when it looks for it.
    let new_step = account(&refused, &[queued("op-2", &[9], true)]);
    assert_eq!(new_step.in_effect, [5]);
    assert_eq!(new_step.void, Vec::<String>::new());
}

/// Away, this person makes three steps and takes the last two back by
/// hand. Back, the first is refused: it goes back, and nothing of the
/// other two is sent — they were made on top of it.
#[test]
fn steps_made_on_a_refused_step_and_taken_back_by_hand_are_not_sent_either() {
    let (mut a, mut b, mut room) = two_in_the_room();
    room.claims.insert(PUMP.to_owned(), "s-b1".to_owned());
    let before = picture(&a);
    drop_session(&mut a);
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    set(&mut a, VESSEL, &[(TEMPERATURE, "60℃")]);
    undo(&mut a);
    undo(&mut a);
    assert_eq!(a.collab.outbox.len(), 5);
    assert_eq!(stacks(&a), (1, 2));

    come_back(&mut a, "s-a2", &room);
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::CLAIM_HELD]);
    assert_eq!(picture(&a), before);
    assert_eq!(stacks(&a), (0, 3));
    assert!(a.collab.outbox.is_empty(), "{:?}", a.collab.outbox);
    assert_eq!(room.revision, 0);
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(a.collab.diverged, None);
}

/// A refused step taken back by hand, and a step made since — which
/// cleared the redo stack the refused one was on. Nothing has to move (the
/// drawing has not had the refused step since), and the later step, made
/// without it, goes to the room.
#[test]
fn a_step_made_after_the_refused_one_was_taken_back_by_hand_still_goes_out() {
    let (mut a, mut b, mut room) = two_in_the_room();
    room.claims.insert(PUMP.to_owned(), "s-b1".to_owned());
    set(&mut a, PUMP, &[(PRESSURE, "1.6MPa")]);
    undo(&mut a);
    set(&mut a, VESSEL, &[(PRESSURE, "0.6MPa")]);
    assert_eq!(a.collab.outbox.len(), 3);

    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), [codes::CLAIM_HELD]);
    assert_eq!(a.collab.diverged, None, "nothing is missing");
    assert_eq!(a.collab.outbox.len(), 1, "the step made since stands");
    assert_eq!(exchange(&mut room, &mut a, &mut [&mut b]), Vec::<String>::new());
    assert_eq!(picture(&a), picture(&b));
    assert_eq!(prop(&b, VESSEL, PRESSURE), "0.6MPa");
    assert_eq!(prop(&b, PUMP, PRESSURE), "1.0MPa");
}

// ── The frames ───────────────────────────────────────────────────────────

/// A `welcome` that announces a replay says so ahead of the session it
/// brings; one that does not says nothing more than it did.
#[test]
fn a_welcome_says_where_the_replay_starts_ahead_of_the_session() {
    let frame = r#"{"type":"welcome","session_id":"s-2","base_revision":9,"peers":[],
        "claims":[],"replay_from":6}"#;
    let reading = messages::read(frame);
    assert_eq!(reading.before, [CollabEvent::Resuming { from: 6 }]);
    assert!(matches!(
        reading.frame,
        messages::Frame::Event(CollabEvent::Connected { base_revision: 9, .. })
    ));

    let plain = r#"{"type":"welcome","session_id":"s-2","base_revision":9,"peers":[],"claims":[]}"#;
    assert!(messages::read(plain).before.is_empty());
}

/// An `error` that names an op is the answer to that op: refused whole
/// (`rejected`), or word of one change of it.
#[test]
fn an_error_that_names_an_op_is_that_ops_answer() {
    let whole = r#"{"type":"error","code":"CLAIM_HELD","message":"已被占用",
        "op_id":"op-s1-12","rejected":true}"#;
    assert!(matches!(
        messages::read(whole).frame,
        messages::Frame::Event(CollabEvent::OpRefused { ref op_id, ref code, rejected: true, .. })
            if op_id == "op-s1-12" && code == "CLAIM_HELD"
    ));
    let part = r#"{"type":"error","code":"OP_SUPERSEDED","message":"落后","op_id":"op-s1-12"}"#;
    assert!(matches!(
        messages::read(part).frame,
        messages::Frame::Event(CollabEvent::OpRefused { rejected: false, .. })
    ));
    // Without an op it is what it always was.
    let bare = r#"{"type":"error","code":"OP_SUPERSEDED","message":"落后"}"#;
    assert!(matches!(
        messages::read(bare).frame,
        messages::Frame::Event(CollabEvent::ServerError { .. })
    ));
}

/// A drawing somebody else holds refuses the op by name and is, as it
/// always was, the word that this session may not write.
#[test]
fn an_op_refused_because_the_drawing_is_held_still_downgrades_the_session() {
    let frame = r#"{"type":"error","code":"PERM_DENIED_STATE","message":"已有活动会话",
        "level":"state","holder":"s-9","op_id":"op-s1-3","rejected":true}"#;
    let reading = messages::read(frame);
    assert!(matches!(
        reading.before.as_slice(),
        [CollabEvent::ReadonlyDowngrade { holder: Some(holder), .. }] if holder == "s-9"
    ));
    assert!(matches!(
        reading.frame,
        messages::Frame::Event(CollabEvent::OpRefused { rejected: true, .. })
    ));
}

/// `SESSION_EXPIRED`: the server has let go of the session and takes no
/// second `hello` on the socket, so the socket is given up and the client
/// joins again. It was an error on the command line, with the socket left
/// open and good for nothing.
#[test]
fn a_session_the_server_let_go_of_is_to_be_joined_again() {
    for code in [codes::SESSION_EXPIRED, codes::SESSION_UNKNOWN] {
        let frame = json!({"type": "error", "code": code, "message": "会话已过期"}).to_string();
        assert!(
            matches!(messages::read(&frame).frame, messages::Frame::Rejoin(ref said) if said == code),
            "{code}"
        );
    }
}

// ── When a claim runs out ────────────────────────────────────────────────

/// The server says when each claim runs out, in `welcome` and when it
/// refuses a claim. The client used to drop the time on the floor; it is
/// kept with the claim and shown in the status.
#[test]
fn the_time_a_claim_runs_out_is_kept_with_it() {
    let mut a = editor("u1");
    let frame = r#"{"type":"welcome","session_id":"s-1","base_revision":0,"peers":[],
        "claims":[{"tag":"P-1001","session_id":"s-2","user_id":"u2","expires_at":1700000300}]}"#;
    let reading = messages::read(frame);
    let messages::Frame::Event(connected) = reading.frame else {
        panic!("welcome is an event");
    };
    hear(&mut a, connected);
    for event in reading.after {
        hear(&mut a, event);
    }
    assert_eq!(a.collab.claims.get(PUMP).map(String::as_str), Some("u2"));
    assert_eq!(a.collab.claim_expiry.get(PUMP), Some(&1_700_000_300));
    assert_eq!(a.collab.status_json()["claim_expiry"][PUMP], 1_700_000_300);

    // A claim this session asked for and was refused says it too.
    hear(
        &mut a,
        CollabEvent::ClaimDenied {
            tag: VESSEL.to_owned(),
            holder: "s-2".to_owned(),
            expires_at: 1_700_000_900,
        },
    );
    assert_eq!(a.collab.claim_expiry.get(VESSEL), Some(&1_700_000_900));

    // Let go of, the time goes with the claim; and with the session, all.
    hear(
        &mut a,
        CollabEvent::ClaimReleased {
            tag: PUMP.to_owned(),
            reason: "expired".to_owned(),
        },
    );
    assert_eq!(a.collab.claim_expiry.get(PUMP), None);
    drop_session(&mut a);
    assert!(a.collab.claim_expiry.is_empty());
}

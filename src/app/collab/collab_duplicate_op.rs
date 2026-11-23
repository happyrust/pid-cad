//! A step the room already has, sent again (`OFF-4`;
//! `SPEC-collab-offline-sync.md` §5.3).
//!
//! The server knows a step by its op id. One it took before is not judged
//! again: the copy that sent it alone is told what came of it — the
//! `op_applied` it went out with, at the revision it took effect at, or the
//! `conflict` entry it went into while that waits for a ruling. To the copy
//! that is the room's word on its own step: off the queue, and the next one
//! goes out. The revision it names is one the drawing has heard already, or
//! is past; the baseline does not go back for it, and the drawing is not
//! taken for one that missed something.
//!
//! The frames are the server's own, read the way the transport reads them
//! (`messages::read`) and taken the way the app takes them
//! (`Message::CollabEvent`). Which frames the server sends, to whom and in
//! what order is held by `ocs_collab_server`'s `duplicate_op` tests.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{CollabEvent, TagsSaid};
use crate::app::{Message, OpenCADStudio};
use crate::pid::collab_state::{self, PID_AUTHOR_COLLAB_QUEUE};
use crate::pid::persist::PidAuthor;
use crate::pid::FileCollabState;
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::EntityType;
use serde_json::{json, Value};

const DRAWING: &str = "PID-1101";
const SYMBOL: &str = "PID_SYM_equipment_vessel";
const PUMP: &str = "P-1001";
const PRESSURE: &str = "设计压力";
/// What the drawing file says of the room it was in, and how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;

/// An editor, as u1, on a drawing with a pump on it, as opened from a file
/// that says the drawing was in room `FILE_REPO` at `FILE_REVISION` and
/// that brings the steps `queue` names, each one of u1's. Never connected.
fn drawing_in_a_room(queue: &[&str]) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    let i = app.active_tab;
    let outline = Line::from_points(Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0));
    app.tabs[i]
        .scene
        .define_block_raw(SYMBOL, Vector3::ZERO, vec![EntityType::Line(outline)]);
    let document = &mut app.tabs[i].scene.document;
    let handle = document
        .add_entity(EntityType::Insert(Insert::new(SYMBOL, Vector3::ZERO)))
        .expect("entity");
    PidAuthor {
        type_code: "EQUIPMENT".to_owned(),
        tag: PUMP.to_owned(),
        tag_source: "auto".to_owned(),
        tag_status: "ok".to_owned(),
        props: r#"{"设计压力":"1.0MPa"}"#.to_owned(),
        ..PidAuthor::default()
    }
    .store(document, handle);
    collab_state::store_state(
        document,
        Some(&FileCollabState {
            drawing_id: DRAWING.to_owned(),
            repo_id: FILE_REPO.to_owned(),
            revision: FILE_REVISION,
        }),
    );
    let entries: Vec<Value> = queue.iter().map(|op_id| entry(op_id)).collect();
    crate::pid::graph_data::store(document, PID_AUTHOR_COLLAB_QUEUE, &entries);
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
    app.collab.document = Some(app.tabs[i].id);
    app
}

/// A queue entry as the file holds it: step `op_id`, queued by u1.
fn entry(op_id: &str) -> Value {
    json!({
        "op_id": op_id,
        "payload": {"kind": "set_property", "object_ref": PUMP, "field": PRESSURE, "value": "1.2MPa"},
        "base_revision": FILE_REVISION,
        "on": true,
        "label": "改设计压力",
        "user_id": "u1",
        "queued_at": 1_794_900_000,
    })
}

/// The copy as it comes back to its room: the `hello` says what the file
/// says, and the room, at `revision`, welcomes it as session `session` and
/// replays what it missed. The replay itself is the caller's to send.
fn back_in_the_room(app: &mut OpenCADStudio, session: &str, revision: i64) {
    let _ = app.update(Message::CollabEvent(CollabEvent::TagsWanted { ask: 1 }));
    assert!(
        app.collab
            .tags_said
            .iter()
            .any(|(ask, said)| *ask == 1 && matches!(said, TagsSaid::Tags(_, Some(_)))),
        "the hello claims what the file says: {:?}",
        app.collab.tags_said
    );
    frame(
        app,
        &json!({
            "type": "welcome",
            "session_id": session,
            "base_revision": revision,
            "peers": [],
            "claims": [],
            "repo_id": FILE_REPO,
            "replay_from": FILE_REVISION,
        })
        .to_string(),
    );
}

/// One frame from the server, read the way the transport reads it, and
/// what it says taken the way the app takes it, in the order the transport
/// hands it on.
fn frame(app: &mut OpenCADStudio, text: &str) {
    let reading = messages::read(text);
    let Frame::Event(event) = reading.frame else {
        panic!("not a frame the UI hears: {text}");
    };
    for event in reading
        .before
        .into_iter()
        .chain([event])
        .chain(reading.after)
    {
        let _ = app.update(Message::CollabEvent(event));
    }
}

/// The room's word that it took step `op_id` at `revision`: the broadcast
/// of it, or the answer to the copy that sent it again — the frame is the
/// same.
fn taken(op_id: &str, revision: i64) -> String {
    json!({"type": "op_applied", "op_id": op_id, "revision": revision, "changes": []}).to_string()
}

/// The room's word that steps `op_ids` went into conflict entry
/// `conflict_id`.
fn held_in(conflict_id: &str, op_ids: &[&str]) -> String {
    json!({
        "type": "conflict",
        "conflict_id": conflict_id,
        "object_ref": PUMP,
        "field": PRESSURE,
        "value_a": "1.2MPa",
        "value_b": "2.5MPa",
        "op_ids": op_ids,
    })
    .to_string()
}

/// The op the drawing has on the wire.
fn on_the_wire(app: &OpenCADStudio) -> Option<String> {
    let mut sent = app.collab.outbox.iter().filter(|entry| entry.sent);
    let first = sent.next()?;
    assert!(sent.next().is_none(), "more than one op on the wire");
    Some(first.op_id.clone())
}

/// The names of the queued steps, in order.
fn queued(app: &OpenCADStudio) -> Vec<String> {
    app.collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.clone())
        .collect()
}

/// The room took op-r-a-1 before, at `FILE_REVISION + 1`, and two steps of
/// somebody else's since. The copy has heard those and is past it; the
/// answer to op-r-a-1, sent again, names the revision it took effect at.
/// The step leaves the queue and the next goes out. The baseline stays
/// where the drawing is, and nothing says the drawing missed anything.
#[test]
fn collab_duplicate_op_an_answer_behind_the_drawing_settles_the_step_and_moves_nothing_back() {
    let mut app = drawing_in_a_room(&["op-r-a-1", "op-r-a-2"]);
    back_in_the_room(&mut app, "s-1", FILE_REVISION + 2);
    frame(&mut app, &taken("op-r-b-1", FILE_REVISION + 1));
    frame(&mut app, &taken("op-r-b-2", FILE_REVISION + 2));
    assert_eq!(app.collab.base_revision, FILE_REVISION + 2);
    assert_eq!(on_the_wire(&app).as_deref(), Some("op-r-a-1"));

    frame(&mut app, &taken("op-r-a-1", FILE_REVISION + 1));

    assert_eq!(queued(&app), ["op-r-a-2"]);
    assert_eq!(on_the_wire(&app).as_deref(), Some("op-r-a-2"));
    assert_eq!(app.collab.base_revision, FILE_REVISION + 2);
    assert_eq!(app.collab.diverged, None);
    assert!(app.collab.not_aligned.is_none());
}

/// One queue in two copies, both back in the room at once, both sending
/// it. The room takes each step from the first copy; the second hears each
/// one taken — it is its own step, too — and then the answer to the one it
/// sent itself, which names a revision it is already at or past. Both
/// queues end empty, at the room's revision, neither copy taken for one
/// that missed something.
#[test]
fn collab_duplicate_op_two_copies_of_one_queue_both_end_empty() {
    let mut first = drawing_in_a_room(&["op-r-a-1", "op-r-a-2"]);
    let mut second = drawing_in_a_room(&["op-r-a-1", "op-r-a-2"]);
    back_in_the_room(&mut first, "s-1", FILE_REVISION);
    back_in_the_room(&mut second, "s-2", FILE_REVISION);
    assert_eq!(on_the_wire(&first).as_deref(), Some("op-r-a-1"));
    assert_eq!(on_the_wire(&second).as_deref(), Some("op-r-a-1"));

    // The room takes op-r-a-1 from the first copy and tells both; the
    // second's own sending of it is answered after that.
    for copy in [&mut first, &mut second] {
        frame(copy, &taken("op-r-a-1", FILE_REVISION + 1));
    }
    frame(&mut second, &taken("op-r-a-1", FILE_REVISION + 1));
    assert_eq!(on_the_wire(&first).as_deref(), Some("op-r-a-2"));
    assert_eq!(on_the_wire(&second).as_deref(), Some("op-r-a-2"));

    for copy in [&mut first, &mut second] {
        frame(copy, &taken("op-r-a-2", FILE_REVISION + 2));
    }
    frame(&mut second, &taken("op-r-a-2", FILE_REVISION + 2));

    for copy in [&first, &second] {
        assert!(copy.collab.outbox.is_empty(), "{:?}", queued(copy));
        assert_eq!(copy.collab.base_revision, FILE_REVISION + 2);
        assert_eq!(copy.collab.diverged, None);
    }
}

/// The step went into a conflict entry, the room was told, and the answer
/// to the copy that sends it again is that entry once more. The step leaves
/// the queue when the room is told; the answer is one entry, not two, and
/// the next step goes out.
#[test]
fn collab_duplicate_op_an_entry_said_again_is_one_entry() {
    let mut app = drawing_in_a_room(&["op-r-a-1", "op-r-a-2"]);
    back_in_the_room(&mut app, "s-1", FILE_REVISION);
    assert_eq!(on_the_wire(&app).as_deref(), Some("op-r-a-1"));

    let entry = held_in("c-1", &["op-r-a-1", "op-r-b-1"]);
    frame(&mut app, &entry);
    assert_eq!(queued(&app), ["op-r-a-2"]);
    assert_eq!(on_the_wire(&app).as_deref(), Some("op-r-a-2"));

    frame(&mut app, &entry);
    let entries: Vec<&str> = app
        .collab
        .conflicts
        .iter()
        .map(|entry| entry.conflict_id.as_str())
        .collect();
    assert_eq!(entries, ["c-1"]);
    assert_eq!(queued(&app), ["op-r-a-2"]);
    assert_eq!(on_the_wire(&app).as_deref(), Some("op-r-a-2"));
    assert_eq!(app.collab.base_revision, FILE_REVISION);
}

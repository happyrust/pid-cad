//! A connection the server does not take as consistent (`OFF-2`;
//! `SPEC-collab-offline-sync.md` §5.2; `REQ-COLL-015` acceptance 3): the
//! room is of another repository, the drawing is ahead of the room, or the
//! room's log cannot replay what the drawing missed. On such a connection
//! nothing of the drawing's queue goes out, nothing the room does is
//! written into it, no claim is asked for, and a save keeps what the file
//! said. The drawing goes on being edited, and its steps go on queueing.
//! Each connection is judged afresh.
//!
//! The frames are the server's own, read the way the transport reads them
//! (`messages::read`) and taken the way the app takes them
//! (`Message::CollabEvent`). The server's half — the refusal in its
//! `welcome`, `NOT_ALIGNED` for an op — is held by the server's own tests.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::CollabEvent;
use crate::app::pid_shell::chrome::SyncStatus;
use crate::app::{Message, OpenCADStudio};
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::{FileCollabState, QueuedOp};
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::{EntityType, Handle};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";
const SYMBOL: &str = "PID_SYM_equipment_vessel";
const PUMP: &str = "P-1001";
const PRESSURE: &str = "设计压力";
/// What the drawing file says of the room it was in, and how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;
/// The step the file brought, which the room never answered.
const FILE_OP: &str = "op-file";
/// The three ways a `welcome` refuses the drawing.
const REFUSALS: [&str; 3] = ["REPO_MISMATCH", "REVISION_AHEAD", "REPLAY_GAP"];

/// An editor on a drawing with a pump on it, as opened from a file that
/// says the drawing was in room `FILE_REPO` at `FILE_REVISION` and that
/// brings one step the room never answered — and edited once since. The
/// file's word is taken up with that edit, so the queue is the file's step
/// and this one. The pump's block is defined, so that the drawing can be
/// saved as it is.
fn opened_from_file() -> OpenCADStudio {
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
    crate::pid::collab_state::store_state(
        document,
        Some(&FileCollabState {
            drawing_id: DRAWING.to_owned(),
            repo_id: FILE_REPO.to_owned(),
            revision: FILE_REVISION,
        }),
    );
    crate::pid::collab_state::store_queue(
        document,
        &[QueuedOp {
            op_id: FILE_OP.to_owned(),
            payload: json!({
                "kind": "set_property",
                "object_ref": PUMP,
                "field": PRESSURE,
                "value": "1.0MPa",
            }),
            base_revision: FILE_REVISION,
            on: true,
            label: "改设计压力".to_owned(),
            user_id: Some("u1".to_owned()),
            queued_at: Some(1_794_900_000),
            inverse: None,
        }],
    );
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
    set(&mut app, "1.6MPa");
    assert_eq!(app.collab.outbox.len(), 2, "the file's step and this one");
    app
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
        .unwrap_or_else(|| panic!("no object tagged {tag}"))
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

/// One step of this person's: the pump's design pressure written through
/// the contract's own write channel for properties (`xdata_set` carrying
/// the whole `props`).
fn set(app: &mut OpenCADStudio, value: &str) {
    let handle = handle_of(app, PUMP);
    let props = json!({ PRESSURE: value }).to_string();
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (response, task) = app.control_request(json!({
        "protocol": 1,
        "op": "xdata_set",
        "request_id": format!("collab-align-{serial}"),
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

/// A control request the way a client sends it, through to its answer.
fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("test");
    req["request_id"] = json!(format!("collab-align-{serial}"));
    let (response, task) = app.control_request(req.clone());
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    if matches!(response["status"].as_str(), Some("accepted" | "running")) {
        app.control_request(json!({"op": "operation", "request_id": req["request_id"]}))
            .0
    } else {
        response
    }
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

/// A `welcome` to session `session` from a room of `repo` at `revision`,
/// with whatever else it says (`replay_from`, `align_refused`).
fn welcome(session: &str, repo: &str, revision: i64, more: Value) -> String {
    let mut frame = json!({
        "type": "welcome",
        "session_id": session,
        "base_revision": revision,
        "peers": [],
        "claims": [],
        "repo_id": repo,
    });
    if let (Some(frame), Some(more)) = (frame.as_object_mut(), more.as_object()) {
        frame.extend(more.clone());
    }
    frame.to_string()
}

/// The room a refusal comes from: of another repository for
/// `REPO_MISMATCH`, behind the drawing for `REVISION_AHEAD`, ahead of it
/// with a log that no longer holds the difference for `REPLAY_GAP`.
fn room_of(code: &str) -> (&'static str, i64) {
    match code {
        "REPO_MISMATCH" => ("r-other", FILE_REVISION),
        "REVISION_AHEAD" => (FILE_REPO, FILE_REVISION - 12),
        _ => (FILE_REPO, FILE_REVISION + 8),
    }
}

/// The server's answer to a drawing it does not take as consistent: the
/// session is up, and its `welcome` says why. Returns the frame and the
/// reason it gives.
fn refused(code: &str) -> (String, String) {
    let (repo, revision) = room_of(code);
    let message = format!("这份图纸与房间对不上（{code}）");
    let frame = welcome(
        "s-1",
        repo,
        revision,
        json!({"align_refused": {"code": code, "message": message}}),
    );
    (frame, message)
}

/// The server's answer to a drawing it takes as consistent: the room is
/// where the file says, and it replays from there — nothing, in this case.
fn taken_back(session: &str) -> String {
    welcome(
        session,
        FILE_REPO,
        FILE_REVISION,
        json!({"replay_from": FILE_REVISION}),
    )
}

/// The op the drawing has on the wire: `(op_id, base_revision)`.
fn on_the_wire(app: &OpenCADStudio) -> Option<(String, i64)> {
    let mut sent = app.collab.outbox.iter().filter(|entry| entry.sent);
    let first = sent.next()?;
    assert!(sent.next().is_none(), "more than one op on the wire");
    Some((first.op_id.clone(), first.base_revision))
}

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-align-refused-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// None of the three refusals lets anything of the queue out — neither the
/// step the file brought nor the one made since — and the drawing goes on
/// being edited, its steps queueing behind them. Nor is it parted from the
/// room for good: that is for a drawing that missed what the room did, and
/// this one is only not taken on this connection.
#[test]
fn nothing_of_the_queue_goes_out_and_edits_go_on_queueing() {
    for code in REFUSALS {
        let mut app = opened_from_file();
        frame(&mut app, &refused(code).0);
        assert_eq!(on_the_wire(&app), None, "{code}: nothing goes out");
        set(&mut app, "2.5MPa");
        assert_eq!(app.collab.outbox.len(), 3, "{code}: the edit is queued");
        assert_eq!(on_the_wire(&app), None, "{code}: and stays there");
        let status = app.collab.status_json();
        assert_eq!(status["not_aligned"]["code"], code, "{status}");
        assert_eq!(status["diverged"], Value::Null, "{code}: {status}");
    }
}

/// What the room does is not written into the drawing: a peer's change
/// arrives, and the drawing and its baseline stay as they were.
#[test]
fn what_the_room_does_is_not_written_into_the_drawing() {
    for code in REFUSALS {
        let mut app = opened_from_file();
        frame(&mut app, &refused(code).0);
        let drawing = picture(&app);
        let (_, room) = room_of(code);
        let peer = json!({
            "type": "op_applied",
            "op_id": "op-peer",
            "revision": room + 1,
            "changes": [{
                "kind": "set_property",
                "object_ref": PUMP,
                "field": PRESSURE,
                "value": "9.9MPa",
            }],
        });
        frame(&mut app, &peer.to_string());
        assert_eq!(picture(&app), drawing, "{code}");
        assert_eq!(app.collab.base_revision, FILE_REVISION, "{code}");
    }
}

/// A save made on such a connection keeps what the file said — the
/// repository and the revision the drawing was opened with — and every
/// queued step: the room that refused the drawing is not the drawing's.
#[test]
fn a_save_keeps_what_the_file_said_and_every_queued_step() {
    let scratch = Scratch::new();
    let path = scratch.0.join("PID-1101.dwg");
    let mut app = opened_from_file();
    frame(&mut app, &refused("REPO_MISMATCH").0);
    set(&mut app, "2.5MPa");
    let queued: Vec<String> = app
        .collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.clone())
        .collect();
    assert_eq!(queued.len(), 3, "{queued:?}");

    let saved = request(&mut app, json!({"op": "save", "path": path}));
    assert_eq!(saved["ok"], true, "{saved}");

    let bytes = std::fs::read(&path).expect("the saved drawing");
    let back = crate::io::load_bytes("PID-1101.dwg", bytes).expect("the saved drawing reads");
    let record = crate::pid::collab_state::load_state(&back).expect("the drawing's record");
    assert_eq!(
        (record.repo_id.as_str(), record.revision),
        (FILE_REPO, FILE_REVISION),
        "the room that refused the drawing is not the drawing's"
    );
    let kept: Vec<String> = crate::pid::collab_state::load_queue(&back)
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(kept, queued);
}

/// An op the server answers with `NOT_ALIGNED` stays queued, off the wire,
/// and nothing is taken back — it was not judged, only not taken on this
/// connection. The connection is not aligned from then on: the steps
/// behind it wait too.
#[test]
fn an_op_answered_not_aligned_stays_queued_and_nothing_is_taken_back() {
    let mut app = opened_from_file();
    frame(&mut app, &taken_back("s-1"));
    let (head, _) = on_the_wire(&app).expect("the head goes out on a connection that is taken");
    let (drawing, steps) = (picture(&app), stacks(&app));
    let answer = json!({
        "type": "error",
        "code": "NOT_ALIGNED",
        "message": "这次连接与房间对不上，不收这份图纸的修改",
        "op_id": head,
    });
    frame(&mut app, &answer.to_string());
    assert_eq!(on_the_wire(&app), None);
    let queued: Vec<&str> = app
        .collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.as_str())
        .collect();
    assert_eq!(queued.len(), 2, "{queued:?}");
    assert_eq!(queued[0], head, "{queued:?}");
    assert_eq!(picture(&app), drawing, "nothing is taken back");
    assert_eq!(stacks(&app), steps, "nothing is taken back");
    let status = app.collab.status_json();
    assert_eq!(status["not_aligned"]["code"], "NOT_ALIGNED", "{status}");
    set(&mut app, "2.5MPa");
    assert_eq!(app.collab.outbox.len(), 3);
    assert_eq!(on_the_wire(&app), None);
}

/// Back to a room that has moved on, with neither a replay nor a refusal:
/// the drawing is not taken as the room's either. It does not take the
/// room's baseline, sends nothing, and is not parted from the room for
/// good.
#[test]
fn a_room_that_moved_on_without_a_replay_is_not_aligned_either() {
    let mut app = opened_from_file();
    frame(
        &mut app,
        &welcome("s-1", FILE_REPO, FILE_REVISION + 8, json!({})),
    );
    assert_eq!(on_the_wire(&app), None);
    assert_eq!(
        app.collab.base_revision, FILE_REVISION,
        "the room's baseline is not taken"
    );
    let status = app.collab.status_json();
    assert!(status["not_aligned"].is_object(), "{status}");
    assert_eq!(status["diverged"], Value::Null, "{status}");
    set(&mut app, "2.5MPa");
    assert_eq!(app.collab.outbox.len(), 3);
    assert_eq!(on_the_wire(&app), None);
}

/// The person is told why, and what to do about it — on the command line
/// and in the panel — and the status bar says 不一致.
#[test]
fn the_person_is_told_why_and_to_get_the_latest_drawing() {
    for code in REFUSALS {
        let mut app = opened_from_file();
        let (welcome, message) = refused(code);
        frame(&mut app, &welcome);
        let said = said(&app);
        assert!(said.contains(&message), "{code}: {said}");
        assert!(said.contains("需要取得最新图纸"), "{code}: {said}");
        let panel = app.pid_collab_view.diverged.clone().unwrap_or_default();
        assert!(panel.contains(&message), "{code}: the panel says {panel:?}");
        assert_eq!(SyncStatus::of(&app), SyncStatus::Diverged, "{code}");
    }
}

/// No claim is asked for — a claim is the first step of a write, and
/// nothing this drawing writes is taken — while who holds what in the room
/// still shows.
#[test]
fn no_claim_is_asked_for_and_the_rooms_claims_still_show() {
    let mut app = opened_from_file();
    let (repo, revision) = room_of("REPO_MISMATCH");
    let claims = json!({"claims": [{
        "tag": "V-1001",
        "session_id": "s-9",
        "user_id": "u2",
        "expires_at": 1_700_000_300,
    }], "align_refused": {"code": "REPO_MISMATCH", "message": "对不上"}});
    frame(&mut app, &welcome("s-1", repo, revision, claims));
    assert_eq!(
        app.collab.claims.get("V-1001").map(String::as_str),
        Some("u2")
    );
    let asked = app.collab.request_claim(PUMP);
    assert!(asked.is_err(), "{asked:?}");
    assert!(
        app.collab.claims_asked.is_empty(),
        "{:?}",
        app.collab.claims_asked
    );
}

/// Each connection is judged afresh: after a refusal, a connection the
/// server takes as consistent sends the queue as usual, from the head.
#[test]
fn the_next_connection_is_judged_afresh() {
    for code in REFUSALS {
        let mut app = opened_from_file();
        frame(&mut app, &refused(code).0);
        let _ = app.update(Message::CollabEvent(CollabEvent::Disconnected {
            reason: "read error: ConnectionReset".to_owned(),
            will_retry: true,
        }));
        frame(&mut app, &taken_back("s-2"));
        let status = app.collab.status_json();
        assert_eq!(status["not_aligned"], Value::Null, "{code}: {status}");
        assert_eq!(
            on_the_wire(&app),
            Some((FILE_OP.to_owned(), FILE_REVISION)),
            "{code}"
        );
    }
}

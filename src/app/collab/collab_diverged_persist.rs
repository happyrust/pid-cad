//! The drawing's own word on whether it still matches the room, kept with
//! it (`OFF-5`; `SPEC-collab-offline-sync.md` §4.1, §5.1, §5.2).
//!
//! * A drawing found apart from the room in a run is saved saying so
//!   (`diverged`), and the next time it joins it says nothing of where it
//!   is: the connection is held apart, for the reason the record gives.
//! * An edit that changed what a drawing with a sync record says of its
//!   objects, and that no queue took — the drawing is not the session's,
//!   no collaboration is configured, the session may not write — goes on
//!   the drawing's unsynced list, and the drawing is saved marked
//!   `UNQUEUED_EDITS`. One that only moved something is no such edit.
//! * The session takes its drawing number from the drawing's record unless
//!   it was given one for this run; a drawing whose record names another
//!   than the one given is not the session's, and its record is left as
//!   it was.
//! * A revision that skips one leaves the baseline at the last revision the
//!   drawing has all of.
//! * A drawing read in after a session came up without it keeps what its
//!   record says — its queue too — and the connection is held apart.
//!
//! Driven the way the app is: the transport's question and the server's
//! frames taken as the app takes them, edits through the contract's write
//! channel, saves through `save`.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{broadcast, CollabEvent, FileClaim, NotAligned, TagsSaid};
use crate::app::{Message, OpenCADStudio};
use crate::pid::collab_state::{
    self, Diverged, REMOTE_UNAPPLIED, REVISION_GAP, UNQUEUED, UNQUEUED_EDITS,
};
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::{FileCollabState, QueuedOp};
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::path::PathBuf;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";
const SYMBOL: &str = "PID_SYM_equipment_vessel";
const PUMP: &str = "P-1001";
const PRESSURE: &str = "设计压力";
/// What the drawing files of these tests say of the room they were in, and
/// how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;
/// The step a file brought, which the room never answered.
const FILE_OP: &str = "op-file";
/// What the command line warns of, once, for an edit no queue took.
const NO_LONGER_SYNCED: &str = "保存之后这份图不能再与服务器同步";

/// A file's record: `drawing`, in `FILE_REPO` at `FILE_REVISION`.
fn record_of(drawing: &str) -> FileCollabState {
    FileCollabState {
        drawing_id: drawing.to_owned(),
        repo_id: FILE_REPO.to_owned(),
        revision: FILE_REVISION,
    }
}

/// The step a file brought: this person's, queued at `FILE_REVISION`.
fn file_op() -> QueuedOp {
    QueuedOp {
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
    }
}

/// An editor with a drawing in front that has a pump on it. No
/// collaboration is configured.
fn editor() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    with_a_pump(&mut app);
    app
}

/// Another drawing, in a tab of its own, in front, with a pump on it.
fn another_drawing(app: &mut OpenCADStudio) {
    let _ = app.update(Message::TabNew);
    with_a_pump(app);
}

/// A pump put on the drawing in front. Its block is defined, so that the
/// drawing can be saved as it is.
fn with_a_pump(app: &mut OpenCADStudio) {
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
}

/// The drawing in front as a file leaves it that was saved with `record`
/// — marked `diverged`, when that is given — and with `queue`.
fn file_says(
    app: &mut OpenCADStudio,
    record: &FileCollabState,
    diverged: Option<&Diverged>,
    queue: &[QueuedOp],
) {
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    collab_state::store_state_diverged(document, Some(record), diverged);
    collab_state::store_queue(document, queue);
}

/// Collaboration configured for this run, with `drawing` the drawing
/// number in effect: as the settings file left it, unless a test says it
/// was given.
fn configured(app: &mut OpenCADStudio, drawing: &str) {
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = drawing.to_owned();
}

/// [`configured`], with the session's drawing the one in front, the way
/// `PIDCOLLAB SERVER` takes it.
fn in_a_room(app: &mut OpenCADStudio, drawing: &str) {
    configured(app, drawing);
    let i = app.active_tab;
    app.collab.document = Some(app.tabs[i].id);
}

/// The transport's question about the drawing's tags, number `ask`, and
/// the app's answer to it.
fn asked(app: &mut OpenCADStudio, ask: u64) -> TagsSaid {
    let _ = app.update(Message::CollabEvent(CollabEvent::TagsWanted { ask }));
    let (to, said) = app.collab.tags_said.last().cloned().expect("an answer");
    assert_eq!(to, ask);
    said
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
        .unwrap_or_else(|| panic!("no object tagged {tag}"))
}

/// The pump's design pressure, as the drawing in front says it.
fn pressure(app: &OpenCADStudio) -> String {
    let document = &app.tabs[app.active_tab].scene.document;
    let author = PidAuthor::load(document, handle_of(app, PUMP)).expect("the pump");
    let mut props = split_props(&author.props).0;
    props.remove(PRESSURE).unwrap_or_default()
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
        "request_id": format!("collab-diverged-{serial}"),
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

/// The pump moved, and nothing else done to it.
fn moved(app: &mut OpenCADStudio) {
    let handle = handle_of(app, PUMP);
    let steps = app.tabs[app.active_tab].history.undo_stack.len();
    let select = json!({"op": "select", "handles": [format!("{:X}", handle.value())]});
    let _ = app.automation_op(&select.to_string());
    let moved = app.automation_op(r#"{"op":"run","cmd":"MOVE 0,0,0 5,5,0"}"#);
    assert_eq!(moved["ok"], true, "{moved}");
    app.finish_all_pending_history();
    let history = &app.tabs[app.active_tab].history;
    assert_eq!(history.undo_stack.len(), steps + 1, "the move is a step");
}

/// A control request the way a client sends it, through to its answer.
fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("test");
    req["request_id"] = json!(format!("collab-diverged-{serial}"));
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

/// The drawing in front saved through `save`, and read back.
fn saved(app: &mut OpenCADStudio, scratch: &Scratch) -> Document {
    let path = scratch.0.join("PID-1101.dwg");
    let answer = request(app, json!({"op": "save", "path": path}));
    assert_eq!(answer["ok"], true, "{answer}");
    let bytes = std::fs::read(&path).expect("the saved drawing");
    crate::io::load_bytes("PID-1101.dwg", bytes).expect("the saved drawing reads")
}

/// The `state` of every entry of a saved drawing's unsynced list.
fn unsynced(back: &Document) -> Vec<String> {
    collab_state::read_unsynced(back)
        .iter()
        .map(|entry| entry["state"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The op ids of a saved drawing's queue.
fn queue(back: &Document) -> Vec<String> {
    collab_state::load_queue(back)
        .into_iter()
        .map(|op| op.op_id)
        .collect()
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
/// with whatever else it says (`replay_from`).
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

/// The room taking the drawing where its file says it is, with nothing to
/// replay.
fn taken_back() -> String {
    welcome(
        "s-1",
        FILE_REPO,
        FILE_REVISION,
        json!({"replay_from": FILE_REVISION}),
    )
}

/// A peer's op the room applied as `revision`: the pump's pressure set to
/// `value`.
fn peer(op_id: &str, revision: i64, value: &str) -> String {
    json!({
        "type": "op_applied",
        "op_id": op_id,
        "revision": revision,
        "changes": [{
            "kind": "set_property",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value": value,
        }],
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

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-diverged-persist-{}-{serial}",
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

/// A revision that skips one marks the drawing `REVISION_GAP` and leaves
/// the baseline at the last revision it has all of; a save writes the mark
/// into the record, with the reason the run gave, and the record's
/// revision is that baseline.
#[test]
fn a_drawing_marked_in_the_run_is_saved_marked_short_of_the_gap() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[]);
    in_a_room(&mut app, DRAWING);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    frame(&mut app, &taken_back());
    assert_eq!(app.collab.not_aligned, None, "the room takes the drawing");
    frame(&mut app, &peer("op-peer-1", FILE_REVISION + 1, "2.0MPa"));
    assert_eq!(app.collab.base_revision, FILE_REVISION + 1);

    frame(&mut app, &peer("op-peer-3", FILE_REVISION + 3, "3.0MPa"));
    let reason = app.collab.diverged.clone().expect("the gap marks it");
    let base = app.collab.base_revision;
    assert_eq!(base, FILE_REVISION + 1, "short of the gap");

    let back = saved(&mut app, &scratch);
    let marked = collab_state::load_diverged(&back).expect("saved marked");
    assert_eq!(
        (marked.code.as_str(), marked.message.as_str()),
        (REVISION_GAP, reason.as_str())
    );
    assert!(marked.at > 0, "{marked:?}");
    let record = collab_state::load_state(&back).expect("the drawing's record");
    assert_eq!(record.revision, FILE_REVISION + 1);
}

/// A drawing saved marked says nothing of where it is when it joins: the
/// answer to the transport's question holds it apart, for the reason the
/// record gives — so the `hello` carries no `repo_id`, no `since` and no
/// tags — and nothing of its queue goes out. A save keeps the mark.
#[test]
fn a_drawing_saved_marked_says_nothing_of_where_it_is_and_sends_nothing() {
    let scratch = Scratch::new();
    let mark = Diverged {
        code: REMOTE_UNAPPLIED.to_owned(),
        message: "操作 op-9：位号 P-1009 在本图里对不上一个对象".to_owned(),
        at: 1_794_900_000,
    };
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), Some(&mark), &[file_op()]);
    in_a_room(&mut app, DRAWING);

    let apart = NotAligned {
        code: mark.code.clone(),
        message: mark.message.clone(),
    };
    assert_eq!(asked(&mut app, 1), TagsSaid::Apart(apart.clone()));
    let joined = welcome("s-1", FILE_REPO, FILE_REVISION + 3, json!({}));
    frame(&mut app, &joined);
    assert_eq!(app.collab.not_aligned, Some(apart));
    assert_eq!(on_the_wire(&app), None, "nothing of the queue goes out");
    assert_eq!(app.collab.base_revision, FILE_REVISION);

    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_diverged(&back), Some(mark));
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
    assert_eq!(queue(&back), [FILE_OP]);
}

/// A drawing with a sync record edited in a run with no collaboration: the
/// edit is kept on its unsynced list, the command line says once that the
/// drawing can no longer be synced, and a save marks it `UNQUEUED_EDITS`.
/// Its record's three and its queue are what the file said.
#[test]
fn an_edit_with_no_collaboration_configured_is_kept_and_marks_the_drawing() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[file_op()]);
    set(&mut app, "1.6MPa");
    set(&mut app, "2.5MPa");
    let told = said(&app);
    assert_eq!(told.matches(NO_LONGER_SYNCED).count(), 1, "{told}");

    let back = saved(&mut app, &scratch);
    assert_eq!(unsynced(&back), [UNQUEUED, UNQUEUED]);
    let first = &collab_state::read_unsynced(&back)[0];
    assert!(first["payload"].to_string().contains("1.6MPa"), "{first}");
    let marked = collab_state::load_diverged(&back).expect("saved marked");
    assert_eq!(marked.code, UNQUEUED_EDITS);
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
    assert_eq!(queue(&back), [FILE_OP]);
}

/// A step that only moves something says nothing of the drawing's objects:
/// nothing is kept, and the drawing is not marked.
#[test]
fn a_move_with_no_collaboration_configured_marks_nothing() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[]);
    moved(&mut app);
    assert!(!said(&app).contains(NO_LONGER_SYNCED), "{}", said(&app));

    let back = saved(&mut app, &scratch);
    assert!(unsynced(&back).is_empty());
    assert_eq!(collab_state::load_diverged(&back), None);
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
}

/// An edit to a drawing with a sync record that is not the session's — the
/// session is up on another — is kept the same way, and the record is the
/// file's own.
#[test]
fn an_edit_to_a_drawing_that_is_not_the_sessions_is_kept_and_marks_it() {
    let scratch = Scratch::new();
    let mut app = editor();
    in_a_room(&mut app, DRAWING);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, None)));
    let joined = welcome("s-1", FILE_REPO, FILE_REVISION, json!({}));
    frame(&mut app, &joined);
    assert!(app.collab.connected && app.collab.not_aligned.is_none());

    another_drawing(&mut app);
    let front = app.tabs[app.active_tab].id;
    assert_ne!(app.collab.document, Some(front), "not the session's");
    file_says(&mut app, &record_of("PID-2202"), None, &[]);
    set(&mut app, "1.6MPa");
    assert!(app.collab.outbox.is_empty(), "nothing queued for the room");
    assert_eq!(said(&app).matches(NO_LONGER_SYNCED).count(), 1);

    let back = saved(&mut app, &scratch);
    assert_eq!(unsynced(&back), [UNQUEUED]);
    let marked = collab_state::load_diverged(&back).expect("saved marked");
    assert_eq!(marked.code, UNQUEUED_EDITS);
    assert_eq!(collab_state::load_state(&back), Some(record_of("PID-2202")));
}

/// A change that reaches a read-only session by a door the command gate
/// does not watch stays in this drawing only: it is kept the same way, and
/// the drawing is saved marked, where the room last had it.
#[test]
fn a_change_that_reaches_a_readonly_session_is_kept_and_marks_the_drawing() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[]);
    in_a_room(&mut app, DRAWING);
    let _ = asked(&mut app, 1);
    frame(&mut app, &taken_back());
    app.collab.readonly = true;

    let change = json!({
        "kind": "set_property",
        "object_ref": PUMP,
        "field": PRESSURE,
        "value": "9.9MPa",
    });
    let changes = broadcast::changes_from_payload(&[change]).changes;
    let i = app.active_tab;
    app.broadcast_changes(i, changes, Vec::new(), true, "改设计压力");
    assert!(app.collab.outbox.is_empty(), "nothing queued for the room");
    assert_eq!(said(&app).matches(NO_LONGER_SYNCED).count(), 1);

    let back = saved(&mut app, &scratch);
    assert_eq!(unsynced(&back), [UNQUEUED]);
    let marked = collab_state::load_diverged(&back).expect("saved marked");
    assert_eq!(marked.code, UNQUEUED_EDITS);
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
}

/// With no drawing number given for this run, the session's is the one
/// the drawing's record names — also over another the settings file kept.
#[test]
fn without_a_drawing_number_given_the_session_takes_the_drawings() {
    for kept in ["", "PID-SETTINGS"] {
        let mut app = editor();
        file_says(&mut app, &record_of(DRAWING), None, &[file_op()]);
        in_a_room(&mut app, kept);
        let said = asked(&mut app, 1);
        assert_eq!(app.collab.drawing_id, DRAWING, "{kept:?}");
        let claim = FileClaim {
            drawing_id: DRAWING.to_owned(),
            repo_id: FILE_REPO.to_owned(),
            revision: FILE_REVISION,
        };
        assert!(
            matches!(&said, TagsSaid::Tags(_, Some(made)) if *made == claim),
            "{kept:?}: {said:?}"
        );
        assert_eq!(app.collab.outbox.len(), 1, "{kept:?}: the file's queue");
    }
}

/// A drawing number given for this run that the drawing's record does not
/// carry: the session does not take the drawing — it has none yet, and may
/// come by the one it is for — the command line says whose it is, and a
/// save leaves its record's three as they were opened. Its edits are ones
/// no queue takes.
#[test]
fn a_drawing_number_given_that_the_record_does_not_carry_leaves_it_alone() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[file_op()]);
    in_a_room(&mut app, "PID-9999");
    app.collab.drawing_given = true;

    assert_eq!(asked(&mut app, 1), TagsSaid::NotYet);
    assert_eq!(app.collab.document, None, "the session has no drawing");
    let told = said(&app);
    assert!(
        told.contains("这份图记着它属于 PID-1101，本次会话是 PID-9999"),
        "{told}"
    );
    set(&mut app, "1.6MPa");
    assert!(app.collab.outbox.is_empty(), "the file's step neither");

    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
    assert_eq!(queue(&back), [FILE_OP]);
    assert_eq!(unsynced(&back), [UNQUEUED]);
    let marked = collab_state::load_diverged(&back).expect("saved marked");
    assert_eq!(marked.code, UNQUEUED_EDITS);
}

/// A drawing saved before the record had `diverged` and an unsynced list is
/// read as having neither: it says where it is, as it always did.
#[test]
fn a_drawing_from_before_reads_as_having_neither() {
    let mut app = editor();
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    collab_state::store_state(document, Some(&record_of(DRAWING)));
    collab_state::store_queue(document, &[file_op()]);
    assert_eq!(collab_state::load_diverged(document), None);
    assert!(collab_state::read_unsynced(document).is_empty());

    in_a_room(&mut app, DRAWING);
    let said = asked(&mut app, 1);
    assert!(matches!(said, TagsSaid::Tags(_, Some(_))), "{said:?}");
}

/// A drawing read in after a session came up without it: what its record
/// says is kept — the queue it brought, where it was — the room's changes
/// that were waiting for a drawing are not written into it, and the
/// connection is held apart, for the session never judged it. A save keeps
/// the file's word and the queue.
#[test]
fn a_drawing_read_in_after_the_session_came_up_keeps_its_record() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[file_op()]);
    configured(&mut app, DRAWING);
    assert_eq!(asked(&mut app, 1), TagsSaid::NotYet);
    frame(&mut app, &welcome("s-1", "r-room", 50, json!({})));
    assert!(app.collab.connected);

    frame(&mut app, &peer("op-peer", 51, "9.9MPa"));
    assert_eq!(pressure(&app), "1.0MPa", "the room's change is not written");
    let apart = app.collab.not_aligned.clone().expect("held apart");
    let queued: Vec<&str> = app
        .collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.as_str())
        .collect();
    assert_eq!(queued, [FILE_OP]);
    assert_eq!(app.collab.repo_id.as_deref(), Some(FILE_REPO));
    assert_eq!(app.collab.base_revision, FILE_REVISION);
    let answer = app.collab.tags_said.last().cloned();
    assert_eq!(answer, Some((1, TagsSaid::Apart(apart))));

    set(&mut app, "1.6MPa");
    assert_eq!(app.collab.outbox.len(), 2);
    assert_eq!(on_the_wire(&app), None);
    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_state(&back), Some(record_of(DRAWING)));
    let kept = queue(&back);
    assert_eq!((kept.len(), kept[0].as_str()), (2, FILE_OP), "{kept:?}");
}

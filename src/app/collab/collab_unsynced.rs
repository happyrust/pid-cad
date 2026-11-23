//! What a drawing has that the server never took, and the way out of a
//! drawing apart from the room (`OFF-9`; `SPEC-collab-offline-sync.md`
//! §4.4, §4.5, §5.5).
//!
//! * `PIDCOLLAB UNSYNCED` lists, for the drawing in front, the steps still
//!   queued and the entries of its unsynced list — refused, unqueued — as
//!   many as the drawing has; `UNSYNCED CLEAR` empties the list and leaves
//!   the queue, and the drawing's mark, as they were.
//! * `PIDCOLLAB EXPORT <path>` writes both out as §4.4 says, with no server
//!   address in it; a path that cannot be written is said, and the drawing
//!   is left as it was.
//! * `PIDCOLLAB DETACH` takes the drawing's three records off at the next
//!   save, after which it joins as a drawing never synced. It is refused
//!   while something is queued or listed, unless `FORCE` is given, and
//!   against a session that is aligned.
//! * `GET /api/v1/collab/status` says where the drawing stands with the
//!   room (`align`), why (`align_code`, `align_message`), and how long its
//!   unsynced list is (`unsynced_count`), next to what it said before.
//!
//! Driven the way the app is: commands through `run`, the status through
//! the control channel, the transport's question and the server's frames
//! taken as the app takes them, saves through `save`.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{CollabEvent, TagsSaid};
use crate::app::{Message, OpenCADStudio};
use crate::pid::collab_state::{
    self, Diverged, Unsynced, REFUSED, REMOTE_UNAPPLIED, UNQUEUED, UNQUEUED_EDITS,
};
use crate::pid::persist::PidAuthor;
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
const SERVER: &str = "ws://127.0.0.1:3240/ws";
/// What the drawing files of these tests say of the room they were in, and
/// how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;

fn record() -> FileCollabState {
    FileCollabState {
        drawing_id: DRAWING.to_owned(),
        repo_id: FILE_REPO.to_owned(),
        revision: FILE_REVISION,
    }
}

/// Two steps the file's queue brought, this person's. The second changed
/// two fields, and carries both the way one step is sent.
fn file_queue() -> Vec<QueuedOp> {
    let one = QueuedOp {
        op_id: "op-file-1".to_owned(),
        payload: json!({
            "kind": "set_property",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value": "1.2MPa",
        }),
        base_revision: FILE_REVISION,
        on: true,
        label: "改设计压力".to_owned(),
        user_id: Some("u1".to_owned()),
        queued_at: Some(1_794_900_000),
        inverse: None,
    };
    let two = QueuedOp {
        op_id: "op-file-2".to_owned(),
        payload: json!({
            "kind": "set_property",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value": "1.4MPa",
            "changes": [{
                "kind": "set_property",
                "object_ref": PUMP,
                "field": "设计温度",
                "value": "80℃",
            }],
        }),
        label: "改两项".to_owned(),
        queued_at: Some(1_794_900_060),
        ..one.clone()
    };
    vec![one, two]
}

/// The file's unsynced list: a step the room refused, taken back, and an
/// edit no queue took.
fn file_list() -> Vec<Value> {
    let refused = Unsynced {
        state: REFUSED.to_owned(),
        op_id: Some("op-refused".to_owned()),
        label: "PIDSETPROP".to_owned(),
        payload: json!({
            "kind": "set_property",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value": "1.6MPa",
        }),
        user_id: Some("u1".to_owned()),
        at: 1_794_903_600,
        code: Some("CLAIM_HELD".to_owned()),
        message: "位号 P-1001 由 u2 占用".to_owned(),
        rolled_back: Some(true),
    };
    let unqueued = Unsynced {
        state: UNQUEUED.to_owned(),
        op_id: None,
        label: "改设计压力".to_owned(),
        payload: json!({
            "kind": "set_property",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value": "1.8MPa",
        }),
        user_id: Some("u1".to_owned()),
        at: 1_794_903_700,
        code: None,
        message: "这张图不是本次会话的图".to_owned(),
        rolled_back: None,
    };
    [refused, unqueued]
        .iter()
        .map(|entry| serde_json::to_value(entry).expect("an entry"))
        .collect()
}

/// What a drawing says that the room's change could not be written into.
fn apart() -> Diverged {
    Diverged {
        code: REMOTE_UNAPPLIED.to_owned(),
        message: "操作 op-9：位号 P-1009 在本图里对不上一个对象".to_owned(),
        at: 1_794_900_000,
    }
}

/// An editor with a drawing in front that has a pump on it. No
/// collaboration is configured.
fn editor() -> OpenCADStudio {
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
    app
}

/// The drawing in front as a file leaves it that was saved with the
/// record — marked, when `mark` is given — and with `queue` and `list`.
fn file_says(app: &mut OpenCADStudio, mark: Option<&Diverged>, queue: &[QueuedOp], list: &[Value]) {
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    collab_state::store_state_diverged(document, Some(&record()), mark);
    collab_state::store_queue(document, queue);
    collab_state::store_unsynced(document, list);
}

/// Collaboration configured for this run, with the session's drawing the
/// one in front, the way `PIDCOLLAB SERVER` takes it.
fn in_a_room(app: &mut OpenCADStudio) {
    app.collab.server_url = SERVER.to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
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

fn run(app: &mut OpenCADStudio, cmd: &str) {
    let _ = app.automation_op(&json!({"op": "run", "cmd": cmd}).to_string());
}

/// What the command line has said.
fn said(app: &OpenCADStudio) -> String {
    app.command_line.history_plain_text()
}

/// `GET /api/v1/collab/status`, through the control channel it is served on.
fn status(app: &mut OpenCADStudio) -> Value {
    app.control_request(json!({"protocol": 1, "op": "collab_status"}))
        .0
}

/// One frame from the server, taken the way the app takes it.
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

fn dropped(app: &mut OpenCADStudio) {
    let _ = app.update(Message::CollabEvent(CollabEvent::Disconnected {
        reason: "连接断了".to_owned(),
        will_retry: true,
    }));
}

/// A `welcome` to session `session` from a room of `FILE_REPO` at
/// `revision`, with whatever else it says.
fn welcome(session: &str, revision: i64, more: Value) -> String {
    let mut frame = json!({
        "type": "welcome",
        "session_id": session,
        "base_revision": revision,
        "peers": [],
        "claims": [],
        "repo_id": FILE_REPO,
    });
    if let (Some(frame), Some(more)) = (frame.as_object_mut(), more.as_object()) {
        frame.extend(more.clone());
    }
    frame.to_string()
}

/// The room taking the drawing where its file says it is.
fn taken_back() -> String {
    welcome("s-1", FILE_REVISION, json!({"replay_from": FILE_REVISION}))
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
        .unwrap_or_else(|| panic!("no object tagged {tag}"))
}

/// A control request the way a client sends it, through to its answer.
fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("test");
    req["request_id"] = json!(format!("collab-unsynced-{serial}"));
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

/// One step of this person's: the pump's design pressure written through
/// the contract's own write channel for properties.
fn set(app: &mut OpenCADStudio, value: &str) {
    let handle = handle_of(app, PUMP);
    let props = json!({ PRESSURE: value }).to_string();
    let answer = request(
        app,
        json!({
            "op": "xdata_set",
            "handles": [format!("{:X}", handle.value())],
            "app": "PID_AUTHOR",
            "data": [{"code": 1000, "value": format!("props={props}")}],
        }),
    );
    assert_eq!(answer["ok"], true, "{answer}");
    app.finish_all_pending_history();
}

/// The drawing in front saved through `save` to `scratch`, and read back.
fn saved(app: &mut OpenCADStudio, scratch: &Scratch) -> Document {
    let path = scratch.drawing();
    let answer = request(app, json!({"op": "save", "path": path}));
    assert_eq!(answer["ok"], true, "{answer}");
    let bytes = std::fs::read(&path).expect("the saved drawing");
    crate::io::load_bytes("PID-1101.dwg", bytes).expect("the saved drawing reads")
}

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-unsynced-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn drawing(&self) -> PathBuf {
        self.0.join("PID-1101.dwg")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The file's queue and list, on a drawing that is not the session's: the
/// list says how many of each there are, and every one of them.
#[test]
fn the_list_says_what_is_queued_refused_and_unqueued() {
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &file_list());
    run(&mut app, "PIDCOLLAB UNSYNCED");
    let said = said(&app);
    assert!(
        said.contains("排队 2 条，被拒 1 条，没排上队 1 条"),
        "{said}"
    );
    for listed in [
        "op-file-1",
        "op-file-2",
        "op-refused",
        "CLAIM_HELD",
        "这张图不是本次会话的图",
    ] {
        assert!(said.contains(listed), "{listed} is not listed: {said}");
    }
}

/// The session's drawing lists the queue the session holds: the file's two
/// steps, read into it, and the one made since, offline.
#[test]
fn the_sessions_drawing_lists_its_queue_as_it_stands() {
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &[]);
    in_a_room(&mut app);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    set(&mut app, "2.0MPa");
    assert_eq!(app.collab.outbox.len(), 3, "{:?}", app.collab.outbox);
    run(&mut app, "PIDCOLLAB UNSYNCED");
    let said = said(&app);
    assert!(
        said.contains("排队 3 条，被拒 0 条，没排上队 0 条"),
        "{said}"
    );
    assert_eq!(status(&mut app)["queued_ops"], 3);
}

/// The export is §4.4's: the drawing's record, when it was written, the
/// queue and then the list, each entry with its changes laid out one by
/// one. The server's address is configured, and not in it.
#[test]
fn the_export_has_the_queue_and_the_list_and_no_server_address() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &file_list());
    in_a_room(&mut app);
    let path = scratch.0.join("unsynced.json");
    run(&mut app, &format!("PIDCOLLAB EXPORT {}", path.display()));
    let text = std::fs::read_to_string(&path).expect("the export");
    assert!(
        !text.contains("127.0.0.1") && !text.contains("ws://"),
        "{text}"
    );
    let export: Value = serde_json::from_str(&text).expect("the export is JSON");
    assert_eq!(export["version"], 1);
    assert_eq!(export["kind"], "collab_unsynced");
    assert_eq!(export["drawing_id"], DRAWING);
    assert_eq!(export["repo_id"], FILE_REPO);
    assert_eq!(export["revision"], FILE_REVISION);
    assert!(
        export["exported_at"].as_i64().is_some_and(|at| at > 0),
        "{export}"
    );
    assert_eq!(export["diverged"], Value::Null);
    let entries = export["entries"].as_array().expect("entries");
    let states: Vec<&str> = entries
        .iter()
        .map(|entry| entry["state"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(states, ["queued", "queued", "refused", "unqueued"]);
    assert_eq!(entries[0]["op_id"], "op-file-1");
    assert_eq!(entries[0]["label"], "改设计压力");
    assert_eq!(entries[0]["user_id"], "u1");
    assert_eq!(entries[0]["at"], 1_794_900_000);
    assert_eq!(
        entries[0]["changes"],
        json!([{"kind": "set_property", "object_ref": PUMP, "field": PRESSURE, "value": "1.2MPa"}])
    );
    let two = &entries[1]["changes"];
    assert_eq!(two.as_array().map(Vec::len), Some(2), "{two}");
    assert!(two[0].get("changes").is_none(), "{two}");
    assert_eq!(two[1]["field"], "设计温度");
    assert_eq!(entries[2]["op_id"], "op-refused");
    assert_eq!(entries[2]["code"], "CLAIM_HELD");
    assert_eq!(entries[2]["message"], "位号 P-1001 由 u2 占用");
    assert_eq!(entries[2]["rolled_back"], true);
    assert_eq!(entries[2]["at"], 1_794_903_600);
    assert_eq!(entries[2]["changes"][0]["value"], "1.6MPa");
    assert_eq!(entries[3]["message"], "这张图不是本次会话的图");
    assert!(entries[3].get("op_id").is_none(), "{}", entries[3]);
    assert!(said(&app).contains("4 条"), "{}", said(&app));
}

/// A path that cannot be written — a directory — is said on the command
/// line, and the drawing is as it was: its records, its undo stack, and
/// whether it has unsaved changes.
#[test]
fn an_export_that_cannot_be_written_says_so_and_changes_nothing() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &file_list());
    let i = app.active_tab;
    let steps = app.tabs[i].history.undo_stack.len();
    let dirty = app.tabs[i].dirty;
    run(
        &mut app,
        &format!("PIDCOLLAB EXPORT {}", scratch.0.display()),
    );
    let said = said(&app);
    assert!(said.contains("写不进"), "{said}");
    let document = &app.tabs[i].scene.document;
    assert_eq!(collab_state::read_queue(document).ops.len(), 2);
    assert_eq!(collab_state::read_unsynced(document).len(), 2);
    assert_eq!(app.tabs[i].history.undo_stack.len(), steps);
    assert_eq!(app.tabs[i].dirty, dirty);
}

/// `UNSYNCED CLEAR` empties the list — the file's entries and the edit no
/// queue took in this run — and leaves the queue as it was. That edit
/// still marks the drawing `UNQUEUED_EDITS`: only `DETACH` takes a mark
/// off.
#[test]
fn clearing_the_list_leaves_the_queue_and_the_mark() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &file_list());
    set(&mut app, "2.0MPa");
    assert_eq!(status(&mut app)["unsynced_count"], 3);
    run(&mut app, "PIDCOLLAB UNSYNCED CLEAR");
    assert_eq!(status(&mut app)["unsynced_count"], 0);
    run(&mut app, "PIDCOLLAB UNSYNCED");
    let said = said(&app);
    assert!(
        said.contains("排队 2 条，被拒 0 条，没排上队 0 条"),
        "{said}"
    );

    let back = saved(&mut app, &scratch);
    assert!(collab_state::read_unsynced(&back).is_empty());
    let queued: Vec<String> = collab_state::load_queue(&back)
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(queued, ["op-file-1", "op-file-2"]);
    let mark = collab_state::load_diverged(&back).expect("still marked");
    assert_eq!(mark.code, UNQUEUED_EDITS);
}

/// `DETACH` with steps queued and listed is refused: the command line says
/// how many, and how to go on. The records are saved as they were.
#[test]
fn detach_with_something_unsynced_is_refused_and_says_why() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &file_list());
    run(&mut app, "PIDCOLLAB DETACH");
    let said = said(&app);
    for told in [
        "排队 2 条",
        "未同步 2 条",
        "PIDCOLLAB EXPORT",
        "PIDCOLLAB DETACH FORCE",
    ] {
        assert!(said.contains(told), "{told}: {said}");
    }
    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_state(&back), Some(record()));
    assert_eq!(collab_state::read_queue(&back).ops.len(), 2);
    assert_eq!(collab_state::read_unsynced(&back).len(), 2);
}

/// `DETACH FORCE` on a drawing that knows it is apart, with steps queued
/// and listed: from then on it reads as never synced, the next save writes
/// none of the three records, and the drawing opened again joins as one
/// never synced — its `hello` claims nothing, so it carries no `since`.
#[test]
fn detach_force_takes_the_three_records_off_at_the_next_save() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, Some(&apart()), &file_queue(), &file_list());
    run(&mut app, "PIDCOLLAB DETACH FORCE");
    let now = status(&mut app);
    assert_eq!(now["align"], "none", "{now}");
    assert_eq!(now["unsynced_count"], 0, "{now}");

    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_state(&back), None);
    assert_eq!(collab_state::load_diverged(&back), None);
    assert!(collab_state::read_queue(&back).ops.is_empty());
    assert!(collab_state::read_unsynced(&back).is_empty());

    let mut again = OpenCADStudio::new_for_test();
    again.backup_on_save = false;
    let opened = again.automation_op(&json!({"op": "open", "path": scratch.drawing()}).to_string());
    assert_eq!(opened["ok"], true, "{opened}");
    in_a_room(&mut again);
    assert!(matches!(asked(&mut again, 1), TagsSaid::Tags(_, None)));
}

/// With nothing queued and nothing listed, `DETACH` needs no `FORCE`.
#[test]
fn detach_with_nothing_unsynced_needs_no_force() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, None, &[], &[]);
    run(&mut app, "PIDCOLLAB DETACH");
    assert!(said(&app).contains("保存后生效"), "{}", said(&app));
    let back = saved(&mut app, &scratch);
    assert_eq!(collab_state::load_state(&back), None);
}

/// A drawing whose session was up in this run, and then lost: after
/// `DETACH FORCE` the transport's next question is answered `Detached` —
/// the `hello` claims nothing, not even where that session left the
/// drawing — until a session is up again; from then on the drawing is in
/// that room, and is answered as any other. The queue went with the
/// records.
#[test]
fn detach_after_a_session_in_this_run_claims_nothing_on_the_next_join() {
    let mut app = editor();
    file_says(&mut app, None, &file_queue(), &[]);
    in_a_room(&mut app);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    frame(&mut app, &taken_back());
    assert_eq!(status(&mut app)["align"], "ok");
    dropped(&mut app);
    assert_eq!(status(&mut app)["align"], "offline");

    run(&mut app, "PIDCOLLAB DETACH FORCE");
    assert!(app.collab.outbox.is_empty(), "{:?}", app.collab.outbox);
    assert!(matches!(asked(&mut app, 2), TagsSaid::Detached(_)));
    frame(&mut app, &welcome("s-2", 50, json!({})));
    assert_eq!(status(&mut app)["align"], "ok");
    dropped(&mut app);
    assert!(matches!(asked(&mut app, 3), TagsSaid::Tags(_, None)));
}

/// Against a session the room has taken as aligned, `DETACH` is refused,
/// `FORCE` or not, and the drawing keeps its record.
#[test]
fn detach_against_an_aligned_session_is_refused() {
    let mut app = editor();
    file_says(&mut app, None, &[], &[]);
    in_a_room(&mut app);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    frame(&mut app, &taken_back());
    run(&mut app, "PIDCOLLAB DETACH FORCE");
    assert!(said(&app).contains("对上了的会话"), "{}", said(&app));
    let i = app.active_tab;
    assert_eq!(
        collab_state::load_state(&app.tabs[i].scene.document),
        Some(record())
    );
    assert_eq!(status(&mut app)["align"], "ok");
}

/// A subcommand `PIDCOLLAB` does not know is answered with the ones it
/// does, the four new ones among them.
#[test]
fn a_mistyped_subcommand_lists_the_four_new_ones() {
    let mut app = editor();
    run(&mut app, "PIDCOLLAB LIST");
    let said = said(&app);
    for listed in [
        "UNSYNCED",
        "UNSYNCED CLEAR",
        "EXPORT <路径>",
        "DETACH [FORCE]",
    ] {
        assert!(said.contains(listed), "{listed}: {said}");
    }
}

/// A drawing started from a template is one no room has seen. The template
/// brings its tables, styles and objects, and none of the three records a
/// template saved from a synced drawing carries.
#[test]
fn a_drawing_started_from_a_template_has_none_of_its_records() {
    let scratch = Scratch::new();
    let mut app = editor();
    file_says(&mut app, Some(&apart()), &file_queue(), &file_list());
    let template = saved(&mut app, &scratch);
    assert!(collab_state::load_state(&template).is_some());

    let mut app = OpenCADStudio::new_for_test();
    let started =
        app.automation_op(&json!({"op": "new", "template": scratch.drawing()}).to_string());
    assert!(started.get("error").is_none(), "{started}");
    let i = app.active_tab;
    let document = &app.tabs[i].scene.document;
    assert!(
        document.entities().count() > 0,
        "the template's objects are not there"
    );
    assert!(collab_state::load_state(document).is_none());
    assert!(collab_state::load_diverged(document).is_none());
    assert!(collab_state::load_queue(document).is_empty());
    assert!(collab_state::read_unsynced(document).is_empty());
    let status = status(&mut app);
    assert_eq!(status["align"], "none", "{status}");
    assert_eq!(status["unsynced_count"], 0, "{status}");
}

/// The four fields `GET /api/v1/collab/status` gained, in each of the five
/// places a drawing can stand with the room; and the ones it had.
mod collab_status {
    use super::*;

    fn align(status: &Value) -> (&str, &Value, &Value) {
        (
            status["align"].as_str().unwrap_or("?"),
            &status["align_code"],
            &status["align_message"],
        )
    }

    /// No record and no session: `none`, nothing to say why, nothing
    /// unsynced.
    #[test]
    fn a_drawing_never_synced_and_not_connected_reads_none() {
        let mut app = editor();
        let status = status(&mut app);
        assert_eq!(
            align(&status),
            ("none", &Value::Null, &Value::Null),
            "{status}"
        );
        assert_eq!(status["unsynced_count"], 0);
    }

    /// A record and no session: `offline`. The list is counted.
    #[test]
    fn a_drawing_with_a_record_and_no_connection_reads_offline() {
        let mut app = editor();
        file_says(&mut app, None, &file_queue(), &file_list());
        let status = status(&mut app);
        assert_eq!(
            align(&status),
            ("offline", &Value::Null, &Value::Null),
            "{status}"
        );
        assert_eq!(status["unsynced_count"], 2);
    }

    /// A session the room took the drawing into: `ok`.
    #[test]
    fn a_session_the_room_takes_reads_ok() {
        let mut app = editor();
        file_says(&mut app, None, &[], &[]);
        in_a_room(&mut app);
        let _ = asked(&mut app, 1);
        frame(&mut app, &taken_back());
        let status = status(&mut app);
        assert_eq!(
            align(&status),
            ("ok", &Value::Null, &Value::Null),
            "{status}"
        );
    }

    /// A connection the room would not take as aligned: `refused`, for the
    /// reason the connection is held apart for.
    #[test]
    fn a_connection_the_room_will_not_take_reads_refused_and_why() {
        let mut app = editor();
        file_says(&mut app, None, &[], &[]);
        in_a_room(&mut app);
        let _ = asked(&mut app, 1);
        let refusing = welcome(
            "s-1",
            50,
            json!({"align_refused": {"code": "REPO_MISMATCH", "message": "协同库标识不一致"}}),
        );
        frame(&mut app, &refusing);
        let held = app.collab.not_aligned.clone().expect("held apart");
        assert_eq!(held.code, "REPO_MISMATCH");
        let status = status(&mut app);
        assert_eq!(
            align(&status),
            ("refused", &json!(held.code), &json!(held.message)),
            "{status}"
        );
    }

    /// A drawing that knows it is apart: `diverged`, for the reason its
    /// record gives — offline, and on a connection held apart for it.
    #[test]
    fn a_drawing_that_knows_it_is_apart_reads_diverged_and_why() {
        let mark = apart();
        let mut app = editor();
        file_says(&mut app, Some(&mark), &[], &[]);
        let offline = status(&mut app);
        assert_eq!(
            align(&offline),
            ("diverged", &json!(mark.code), &json!(mark.message)),
            "{offline}"
        );
        in_a_room(&mut app);
        let _ = asked(&mut app, 1);
        frame(&mut app, &welcome("s-1", 50, json!({})));
        let connected = status(&mut app);
        assert_eq!(
            align(&connected),
            ("diverged", &json!(mark.code), &json!(mark.message)),
            "{connected}"
        );
    }

    /// The fields that were there read as they did: the body is the
    /// session's own, with the four added.
    #[test]
    fn the_fields_that_were_there_read_as_before() {
        let mut app = editor();
        file_says(&mut app, None, &file_queue(), &file_list());
        in_a_room(&mut app);
        let _ = asked(&mut app, 1);
        frame(&mut app, &taken_back());
        let mut status = status(&mut app);
        let before = app.collab.status_json();
        let added = status.as_object_mut().expect("an object");
        for field in ["align", "align_code", "align_message", "unsynced_count"] {
            assert!(added.remove(field).is_some(), "{field} is missing");
        }
        assert_eq!(status, before);
    }
}

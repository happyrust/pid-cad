//! Every way the editor writes a whole drawing to a file writes the sync
//! records the way the drawing stands when it is written (`OFF-6`;
//! `SPEC-collab-offline-sync.md` §5.1, "保存写什么" and "哪些出口").
//!
//! * A save, a Save As in the background, an autosave, SAVEALL, the
//!   automation's save — to a template too — and the contract's verified
//!   save write the revision and the queue the drawing has now, not the
//!   ones it was opened with.
//! * DWG and DXF carry the three records alike.
//! * A block written out is another drawing: it carries none of them.
//! * A drawing with no record that was never in a room is written without
//!   them.
//! * Only the copy that is written gets them: the drawing being edited is
//!   not marked changed, its undo stack gains nothing, and its own records
//!   are the ones it was opened with.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{CollabEvent, TagsSaid};
use crate::app::{Message, OpenCADStudio, SaveContinuation, SavePurpose};
use crate::modules::insert::block_cmdline::WblockSource;
use crate::pid::collab_state::{self, Diverged, REMOTE_UNAPPLIED, UNQUEUED, UNQUEUED_EDITS};
use crate::pid::persist::PidAuthor;
use crate::pid::{FileCollabState, QueuedOp};
use crate::ui::window::block_definition::BlockObjectMode;
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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

/// Collaboration configured for this run, with `drawing` its drawing
/// number and the drawing in front the session's, the way `PIDCOLLAB
/// SERVER` takes it.
fn in_a_room(app: &mut OpenCADStudio, drawing: &str) {
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = drawing.to_owned();
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
        "request_id": format!("collab-save-exits-{serial}"),
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

/// The pump moved with MOVE. Unlike a property written through the
/// contract, a command marks the drawing changed.
fn moved(app: &mut OpenCADStudio) {
    let handle = handle_of(app, PUMP);
    let select = json!({"op": "select", "handles": [format!("{:X}", handle.value())]});
    let _ = app.automation_op(&select.to_string());
    let moved = app.automation_op(r#"{"op":"run","cmd":"MOVE 0,0,0 5,5,0"}"#);
    assert_eq!(moved["ok"], true, "{moved}");
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
    req["request_id"] = json!(format!("collab-save-exits-{serial}"));
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

/// The room taking the drawing where its file says it is, with nothing to
/// replay.
fn taken_back() -> String {
    json!({
        "type": "welcome",
        "session_id": "s-1",
        "base_revision": FILE_REVISION,
        "peers": [],
        "claims": [],
        "repo_id": FILE_REPO,
        "replay_from": FILE_REVISION,
    })
    .to_string()
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

/// A drawing whose file put it in `FILE_REPO` at `FILE_REVISION` with
/// nothing queued, taken back by the room. The room has applied a peer's
/// step since, and a step of this person's is queued and unanswered: what
/// the drawing says of the room is no longer what its file said.
fn moved_on() -> OpenCADStudio {
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), None, &[]);
    in_a_room(&mut app, DRAWING);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    frame(&mut app, &taken_back());
    frame(&mut app, &peer("op-peer-1", FILE_REVISION + 1, "2.0MPa"));
    set(&mut app, "1.6MPa");
    assert_eq!(app.collab.base_revision, FILE_REVISION + 1);
    assert_eq!(app.collab.outbox.len(), 1, "a step of this person's queued");
    app
}

/// The record and the queue a write has to give the drawing in `app`: the
/// way it stands.
fn as_it_stands(app: &OpenCADStudio) -> (FileCollabState, Vec<String>) {
    let record = FileCollabState {
        drawing_id: app.collab.drawing_id.clone(),
        repo_id: app.collab.repo_id.clone().expect("a repository"),
        revision: app.collab.base_revision,
    };
    let queue = app
        .collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.clone())
        .collect();
    (record, queue)
}

/// `path` read back, the way the editor reads a file. A template and an
/// autosave are DWG.
fn read(path: &Path) -> Document {
    let bytes = std::fs::read(path).expect("the written drawing");
    let dxf = path.extension().is_some_and(|ext| ext == "dxf");
    let name = if dxf { "back.dxf" } else { "back.dwg" };
    crate::io::load_bytes(name, bytes).expect("the written drawing reads")
}

/// The op ids of a written drawing's queue.
fn queue(back: &Document) -> Vec<String> {
    collab_state::load_queue(back)
        .into_iter()
        .map(|op| op.op_id)
        .collect()
}

/// The `state` of every entry of a written drawing's unsynced list.
fn unsynced(back: &Document) -> Vec<String> {
    collab_state::read_unsynced(back)
        .iter()
        .map(|entry| entry["state"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// What the three records of `back` say, item by item.
fn three(back: &Document) -> Value {
    let record = collab_state::load_state(back);
    let diverged = collab_state::load_diverged(back);
    let queue: Vec<Value> = collab_state::load_queue(back)
        .iter()
        .map(|op| serde_json::to_value(op).expect("an op"))
        .collect();
    json!({
        "record": record.map(|record| (record.drawing_id, record.repo_id, record.revision)),
        "diverged": diverged.map(|mark| (mark.code, mark.message, mark.at)),
        "queue": queue,
        "unsynced": collab_state::read_unsynced(back),
    })
}

/// `back` was written with the record and the queue the drawing in `app`
/// has now, and with nothing marked and nothing kept.
fn written_as_it_stands(back: &Document, app: &OpenCADStudio) {
    let (record, queued) = as_it_stands(app);
    assert_eq!(collab_state::load_state(back), Some(record));
    assert_eq!(queue(back), queued);
    assert_eq!(collab_state::load_diverged(back), None);
    assert!(unsynced(back).is_empty());
}

/// `back` carries none of the three records.
fn written_without_them(back: &Document, what: &str) {
    assert_eq!(collab_state::load_state(back), None, "{what}");
    assert!(queue(back).is_empty(), "{what}");
    assert!(collab_state::read_unsynced(back).is_empty(), "{what}");
}

/// The drawing in front saved through the contract's `op`, as `path`.
fn saved_through(app: &mut OpenCADStudio, op: &str, path: &Path) {
    let answer = request(app, json!({"op": op, "path": path}));
    assert_eq!(answer["ok"], true, "{op}: {answer}");
}

/// The drawing in front saved through the automation's `save`, as `path`.
fn saved_by_automation(app: &mut OpenCADStudio, path: &Path) {
    let saved = app.automation_op(&json!({"op": "save", "path": path}).to_string());
    assert_eq!(saved["ok"], true, "{saved}");
}

/// The drawing in front saved as `path` the way Save As saves it: in the
/// background.
fn saved_as(app: &mut OpenCADStudio, path: &Path) {
    let i = app.active_tab;
    let version = app.tabs[i].scene.document.version;
    app.prepare_native_save(i);
    let task = app.queue_native_save(
        i,
        path.to_path_buf(),
        version,
        SavePurpose::SaveAs,
        SaveContinuation::None,
        true,
        true,
    );
    app.drive_headless_task(task)
        .expect("a task that needs no window");
}

/// After a save, the drawing being edited is as it was: not marked
/// changed, no more than `steps` steps to undo, and its own records the
/// ones it was opened with.
fn left_as_it_was(app: &OpenCADStudio, steps: usize, what: &str) {
    let tab = &app.tabs[app.active_tab];
    assert!(!tab.dirty, "{what}: marked changed");
    assert_eq!(tab.history.undo_stack.len(), steps, "{what}");
    let document = &tab.scene.document;
    let record = collab_state::load_state(document);
    assert_eq!(record, Some(record_of(DRAWING)), "{what}");
    assert!(collab_state::load_queue(document).is_empty(), "{what}");
}

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-save-exits-{}-{serial}",
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

/// A save writes the record and the queue the drawing has now: the
/// revision the room took it to since it was opened, and the step still
/// queued.
#[test]
fn a_save_writes_them_as_the_drawing_stands() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    let path = scratch.0.join("save.dwg");
    saved_through(&mut app, "save", &path);
    written_as_it_stands(&read(&path), &app);
}

/// Save As, which runs in the background, the same.
#[test]
fn a_save_as_in_the_background_writes_them_too() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    let path = scratch.0.join("save-as.dwg");
    saved_as(&mut app, &path);
    written_as_it_stands(&read(&path), &app);
}

/// The autosave's copy the same, with the steps made since the last save.
/// The autosave only copies a drawing marked changed, hence the move.
#[test]
fn an_autosave_writes_them_too() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    saved_through(&mut app, "save", &scratch.0.join("PID-1101.dwg"));
    set(&mut app, "2.5MPa");
    moved(&mut app);
    let i = app.active_tab;
    assert!(app.tabs[i].dirty, "the move marks the drawing changed");
    assert!(app.collab.outbox.len() > 1, "a step queued since the save");
    let target = app.autosave_target(i);
    let task = app.update(Message::AutoSave);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    written_as_it_stands(&read(&target), &app);
}

/// SAVEALL writes every drawing that has a file the way it stands: the
/// session's with where the room has taken it since its last save, and
/// another drawing with the record its file gave it, marked for the edit
/// no queue took, which it keeps.
#[test]
fn saveall_writes_each_drawing_as_it_stands() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    let session = scratch.0.join("PID-1101.dwg");
    saved_through(&mut app, "save", &session);
    another_drawing(&mut app);
    file_says(&mut app, &record_of("PID-1102"), None, &[]);
    let other = scratch.0.join("PID-1102.dwg");
    saved_through(&mut app, "save", &other);
    frame(&mut app, &peer("op-peer-2", FILE_REVISION + 2, "3.0MPa"));
    set(&mut app, "4.0MPa");

    let ran = app.automation_op(r#"{"op":"run","cmd":"SAVEALL"}"#);
    assert_eq!(ran["ok"], true, "{ran}");
    assert_eq!(app.collab.base_revision, FILE_REVISION + 2);
    written_as_it_stands(&read(&session), &app);
    let back = read(&other);
    assert_eq!(collab_state::load_state(&back), Some(record_of("PID-1102")));
    assert!(queue(&back).is_empty());
    assert_eq!(unsynced(&back), [UNQUEUED]);
    let marked = collab_state::load_diverged(&back).expect("marked");
    assert_eq!(marked.code, UNQUEUED_EDITS);
}

/// The automation's save writes them, to a drawing and to a template.
#[test]
fn the_automations_save_writes_them_to_a_drawing_and_to_a_template() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    let drawing = scratch.0.join("automation.dwg");
    saved_by_automation(&mut app, &drawing);
    written_as_it_stands(&read(&drawing), &app);
    let template = scratch.0.join("automation.dwt");
    saved_by_automation(&mut app, &template);
    written_as_it_stands(&read(&template), &app);
}

/// The contract's verified save writes the revision and the queue the
/// drawing has now, not the ones it was opened with.
#[test]
fn a_verified_save_writes_them_as_the_drawing_stands() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    let path = scratch.0.join("verified.dwg");
    saved_through(&mut app, "save_verified", &path);
    written_as_it_stands(&read(&path), &app);
}

/// DWG and DXF carry the three alike: a drawing whose file marked it,
/// queued a step and kept an entry on its unsynced list, edited in a run
/// with no collaboration, reads back the same from either — and the way
/// it stood.
#[test]
fn dwg_and_dxf_carry_the_three_alike() {
    let scratch = Scratch::new();
    let mark = Diverged {
        code: REMOTE_UNAPPLIED.to_owned(),
        message: "操作 op-9：位号 P-1009 在本图里对不上一个对象".to_owned(),
        at: 1_794_900_000,
    };
    let refused = json!({
        "state": "refused",
        "op_id": "op-old",
        "label": "改设计压力",
        "payload": {"changes": []},
        "at": 1_794_800_000,
        "code": "PERMISSION_DENIED",
        "message": "没有改这个对象的权限",
        "rolled_back": true,
    });
    let mut app = editor();
    file_says(&mut app, &record_of(DRAWING), Some(&mark), &[file_op()]);
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    collab_state::store_unsynced(document, std::slice::from_ref(&refused));
    set(&mut app, "1.6MPa");

    let dwg = scratch.0.join("PID-1101.dwg");
    saved_through(&mut app, "save", &dwg);
    let dxf = scratch.0.join("PID-1101.dxf");
    saved_through(&mut app, "save", &dxf);
    let (dwg, dxf) = (read(&dwg), read(&dxf));
    assert_eq!(three(&dwg), three(&dxf), "DWG and DXF alike");
    assert_eq!(collab_state::load_state(&dwg), Some(record_of(DRAWING)));
    assert_eq!(collab_state::load_diverged(&dwg), Some(mark));
    assert_eq!(queue(&dwg), [FILE_OP]);
    assert_eq!(unsynced(&dwg), ["refused", UNQUEUED]);
    assert_eq!(collab_state::read_unsynced(&dwg)[0], refused);
}

/// A block written out is another drawing: the whole drawing written out
/// (`WBLOCK *`), objects written out, and objects the contract's `wblock`
/// writes onto a template whose file had records carry none of them.
#[test]
fn a_block_written_out_carries_none_of_them() {
    let scratch = Scratch::new();
    let template = scratch.0.join("template.dwg");
    let mut base = Document::new();
    collab_state::store_state(&mut base, Some(&record_of("PID-2001")));
    collab_state::store_queue(&mut base, &[file_op()]);
    crate::io::save(&base, &template).expect("the template");
    let mut app = moved_on();
    let i = app.active_tab;
    let unit = app.tabs[i].scene.document.header.insertion_units;
    let pump = handle_of(&app, PUMP);

    let whole = scratch.0.join("whole.dwg");
    let task = app.wblock_write(i, WblockSource::Drawing, whole.clone(), unit);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    let objects = scratch.0.join("objects.dwg");
    let source = WblockSource::Objects {
        handles: vec![pump],
        base: Default::default(),
        mode: BlockObjectMode::Retain,
    };
    let task = app.wblock_write(i, source, objects.clone(), unit);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    let onto = scratch.0.join("onto-a-template.dwg");
    let written = request(
        &mut app,
        json!({
            "op": "wblock",
            "path": onto,
            "handles": [format!("{:X}", pump.value())],
            "template": template,
        }),
    );
    assert_eq!(written["ok"], true, "{written}");

    written_without_them(&read(&whole), "WBLOCK *");
    written_without_them(&read(&objects), "WBLOCK of objects");
    written_without_them(&read(&onto), "wblock onto a template");
}

/// A drawing with no record that was never in a room is written without
/// them, whichever way it is written.
#[test]
fn a_drawing_never_in_a_room_is_written_without_them() {
    let scratch = Scratch::new();
    let mut app = editor();
    set(&mut app, "1.6MPa");
    let save = scratch.0.join("save.dwg");
    saved_through(&mut app, "save", &save);
    let verified = scratch.0.join("verified.dwg");
    saved_through(&mut app, "save_verified", &verified);
    let template = scratch.0.join("template.dwt");
    saved_by_automation(&mut app, &template);
    let save_as = scratch.0.join("save-as.dwg");
    saved_as(&mut app, &save_as);

    written_without_them(&read(&save), "save");
    written_without_them(&read(&verified), "save_verified");
    written_without_them(&read(&template), "a template");
    written_without_them(&read(&save_as), "Save As");
}

/// Writing them touches only the copy that is written: after each way of
/// saving, the drawing being edited is not marked changed, has no more
/// steps to undo, and its own records are the ones it was opened with.
#[test]
fn writing_them_leaves_the_drawing_being_edited_as_it_was() {
    let scratch = Scratch::new();
    let mut app = moved_on();
    saved_through(&mut app, "save", &scratch.0.join("save.dwg"));
    let i = app.active_tab;
    let steps = app.tabs[i].history.undo_stack.len();
    left_as_it_was(&app, steps, "save");

    saved_through(&mut app, "save_verified", &scratch.0.join("verified.dwg"));
    left_as_it_was(&app, steps, "save_verified");
    saved_by_automation(&mut app, &scratch.0.join("automation.dwt"));
    left_as_it_was(&app, steps, "a template");
    saved_as(&mut app, &scratch.0.join("save-as.dwg"));
    left_as_it_was(&app, steps, "Save As");
}

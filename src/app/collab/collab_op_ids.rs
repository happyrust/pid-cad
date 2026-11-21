//! Op ids no two runs share, and a queue that says whose each step is
//! (`OFF-3`; `SPEC-collab-offline-sync.md` §4.1, §4.2 and step two of
//! §5.2).
//!
//! A step is named `op-<run id>-<n>`: the run id is drawn once a run, and
//! each run counts from 1. The name owes nothing to a session, so a step
//! made offline is named like one made online, and two starts — of one
//! person or of two — never name two steps alike. A step restored from the
//! drawing file keeps the name it was given.
//!
//! Each queued step says who queued it and when. Before a connection says
//! what the drawing is, the copy looks at its queue. Steps somebody else
//! queued (or that say nobody), two steps under one name, or entries it
//! could not read: the `hello` claims nothing and lists no tags, and the
//! connection is held apart — that queue is not this person's to send.
//!
//! The frames are the server's own, read the way the transport reads them
//! (`messages::read`) and taken the way the app takes them
//! (`Message::CollabEvent`). What the `hello` says for an answer that holds
//! the connection apart is held by the transport's own tests.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{CollabEvent, TagsSaid};
use crate::app::{Message, OpenCADStudio};
use crate::pid::collab_state::{self, PID_AUTHOR_COLLAB_QUEUE};
use crate::pid::persist::PidAuthor;
use crate::pid::FileCollabState;
use document::entities::{Insert, Line};
use document::objects::XRecordEntry;
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";
const SYMBOL: &str = "PID_SYM_equipment_vessel";
const PUMP: &str = "P-1001";
const PRESSURE: &str = "设计压力";
/// What the drawing file says of the room it was in, and how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;

/// An editor, as `user`, on a drawing with a pump on it, as opened from a
/// file that says the drawing was in room `FILE_REPO` at `FILE_REVISION`
/// and that brings `queue` — the queue record's entries as the file holds
/// them. Never connected. The pump's block is defined, so that the drawing
/// can be saved as it is.
fn drawing_in_a_room(user: &str, queue: &[Value]) -> OpenCADStudio {
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
    crate::pid::graph_data::store(document, PID_AUTHOR_COLLAB_QUEUE, queue);
    as_user(&mut app, user);
    app.collab.document = Some(app.tabs[i].id);
    app
}

/// Another start of the program, as `user`, on the drawing saved at `path`.
fn reopened(path: &Path, user: &str) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let opened = app.automation_op(&json!({"op": "open", "path": path}).to_string());
    assert_eq!(opened["ok"], true, "{opened}");
    as_user(&mut app, user);
    app.collab.document = Some(app.tabs[app.active_tab].id);
    app
}

fn as_user(app: &mut OpenCADStudio, user: &str) {
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = user.to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
}

/// A queue entry as the file holds it: step `op_id`, queued by `user`.
fn entry(op_id: &str, user: &str) -> Value {
    json!({
        "op_id": op_id,
        "payload": {"kind": "set_property", "object_ref": PUMP, "field": PRESSURE, "value": "1.2MPa"},
        "base_revision": FILE_REVISION,
        "on": true,
        "label": "改设计压力",
        "user_id": user,
        "queued_at": 1_794_900_000,
    })
}

/// A queue entry as a version from before `user_id` and `queued_at` saved
/// it, with an id of the kind it gave.
fn old_entry(op_id: &str) -> Value {
    json!({
        "op_id": op_id,
        "payload": {"kind": "set_property", "object_ref": PUMP, "field": PRESSURE, "value": "1.2MPa"},
        "base_revision": FILE_REVISION,
        "on": true,
        "label": "改设计压力",
    })
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
        .unwrap_or_else(|| panic!("no object tagged {tag}"))
}

/// Every tag an object of the drawing in front carries.
fn tags_on(app: &OpenCADStudio) -> Vec<String> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .map(|author| author.tag)
        .filter(|tag| !tag.is_empty())
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
        "request_id": format!("collab-op-ids-{serial}"),
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

/// Save the drawing in front to `path`, the way a client asks for it.
fn save(app: &mut OpenCADStudio, path: &Path) {
    let state = app.control_state();
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let request_id = format!("collab-op-ids-{serial}");
    let (response, task) = app.control_request(json!({
        "protocol": 1,
        "op": "save",
        "path": path,
        "request_id": request_id,
        "client_id": "test",
        "document_id": state["document_id"],
        "revision": state["revision"],
    }));
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    let response = if matches!(response["status"].as_str(), Some("accepted" | "running")) {
        app.control_request(json!({"op": "operation", "request_id": request_id}))
            .0
    } else {
        response
    };
    assert_eq!(response["ok"], true, "{response}");
}

/// The drawing saved at `path`, as the next open reads it.
fn read_back(path: &Path) -> Document {
    let bytes = std::fs::read(path).expect("the saved drawing");
    crate::io::load_bytes("PID-1101.dwg", bytes).expect("the saved drawing reads")
}

/// The transport asks which tags the drawing carries, under number `ask`,
/// and the answer the app gives.
fn ask(app: &mut OpenCADStudio, ask: u64) -> TagsSaid {
    let _ = app.update(Message::CollabEvent(CollabEvent::TagsWanted { ask }));
    app.collab
        .tags_said
        .iter()
        .rev()
        .find(|(answered, _)| *answered == ask)
        .map(|(_, said)| said.clone())
        .unwrap_or_else(|| {
            panic!(
                "question {ask} was not answered: {:?}",
                app.collab.tags_said
            )
        })
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

/// A `welcome` to session `session` from the file's room at `revision`,
/// with whatever else it says (`replay_from`).
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

/// The room's word that it took step `op_id` at `revision`: the broadcast
/// of it, which reaches the one who sent it too.
fn taken(op_id: &str, revision: i64) -> String {
    json!({"type": "op_applied", "op_id": op_id, "revision": revision, "changes": []}).to_string()
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

/// `(run, n)` of a step named `op-<run>-<n>`.
fn run_and_count(op_id: &str) -> (String, u64) {
    let named = op_id
        .strip_prefix("op-")
        .and_then(|rest| rest.rsplit_once('-'))
        .and_then(|(run, n)| Some((run.to_owned(), n.parse().ok()?)));
    named.unwrap_or_else(|| panic!("not op-<run>-<n>: {op_id}"))
}

/// Whether `run` is a run id: `r-` and a UUID.
fn is_a_run_id(run: &str) -> bool {
    run.strip_prefix("r-")
        .is_some_and(|uuid| uuid.len() == 36 && uuid.split('-').map(str::len).eq([8, 4, 4, 4, 12]))
}

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ocs-collab-op-ids-{}-{serial}", std::process::id()));
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

// ── The names ──────────────────────────────────────────────────────────────────────

/// Two starts in a row, neither ever connected, each makes two steps: four
/// names, two runs, each counting from 1. The steps the first start made
/// keep their names in the second. A third start finds all four under four
/// names, and once it is back in the room every one of them goes out, once,
/// in the order it was made — none taken for another when the room
/// answers.
#[test]
fn two_starts_name_their_steps_apart_and_every_step_gets_through() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut first = drawing_in_a_room("u1", &[]);
    set(&mut first, "1.6MPa");
    set(&mut first, "2.0MPa");
    let made_first = queued(&first);
    save(&mut first, &path);
    drop(first);

    let mut second = reopened(&path, "u1");
    set(&mut second, "2.5MPa");
    set(&mut second, "3.0MPa");
    let all = queued(&second);
    assert_eq!(all.len(), 4, "{all:?}");
    assert_eq!(all[..2], made_first[..], "restored steps keep their names");
    let named: Vec<(String, u64)> = all.iter().map(|id| run_and_count(id)).collect();
    assert!(
        named.iter().all(|(run, _)| is_a_run_id(run)),
        "named by a run: {all:?}"
    );
    assert_eq!(named[0].0, named[1].0, "one run: {all:?}");
    assert_eq!(named[2].0, named[3].0, "one run: {all:?}");
    assert_ne!(named[0].0, named[2].0, "two starts: {all:?}");
    let counts: Vec<u64> = named.iter().map(|(_, n)| *n).collect();
    assert_eq!(counts, [1, 2, 1, 2], "each run counts from 1: {all:?}");
    save(&mut second, &path);
    drop(second);

    let mut third = reopened(&path, "u1");
    assert!(
        matches!(ask(&mut third, 1), TagsSaid::Tags(_, Some(_))),
        "the hello claims what the file says: {:?}",
        third.collab.tags_said
    );
    assert_eq!(queued(&third), all);
    frame(
        &mut third,
        &welcome("s-1", FILE_REVISION, json!({"replay_from": FILE_REVISION})),
    );
    let mut sent = Vec::new();
    for revision in FILE_REVISION + 1..=FILE_REVISION + 4 {
        let head = on_the_wire(&third).unwrap_or_else(|| panic!("nothing went out after {sent:?}"));
        frame(&mut third, &taken(&head, revision));
        sent.push(head);
    }
    assert_eq!(sent, all, "each went out once, in order");
    assert!(third.collab.outbox.is_empty(), "{:?}", queued(&third));
}

/// Two people on copies of one drawing, each offline: their steps are named
/// by two runs, and each step says whose it is.
#[test]
fn two_people_offline_name_their_steps_apart() {
    let mut mine = drawing_in_a_room("u1", &[]);
    let mut theirs = drawing_in_a_room("u2", &[]);
    set(&mut mine, "1.6MPa");
    set(&mut theirs, "1.6MPa");
    let (mine_named, theirs_named) = (queued(&mine), queued(&theirs));
    let (my_run, _) = run_and_count(&mine_named[0]);
    let (their_run, _) = run_and_count(&theirs_named[0]);
    assert!(is_a_run_id(&my_run), "{mine_named:?}");
    assert!(is_a_run_id(&their_run), "{theirs_named:?}");
    assert_ne!(my_run, their_run);
    let whose = |app: &OpenCADStudio| {
        app.collab
            .outbox
            .iter()
            .map(|entry| entry.user_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(whose(&mine), [Some("u1".to_owned())]);
    assert_eq!(whose(&theirs), [Some("u2".to_owned())]);
}

/// A restored step goes out under the name it was made with; and one the
/// room turns out to have taken before the copy was closed — it comes back
/// in the replay — leaves the queue without being sent again.
#[test]
fn a_restored_step_goes_out_under_its_own_name_and_a_replay_of_it_settles_it() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut first = drawing_in_a_room("u1", &[]);
    set(&mut first, "1.6MPa");
    set(&mut first, "2.0MPa");
    let made = queued(&first);
    save(&mut first, &path);
    drop(first);

    let mut second = reopened(&path, "u1");
    let _ = ask(&mut second, 1);
    frame(
        &mut second,
        &welcome(
            "s-1",
            FILE_REVISION + 1,
            json!({"replay_from": FILE_REVISION}),
        ),
    );
    assert_eq!(
        on_the_wire(&second),
        None,
        "not before the replay is through"
    );
    frame(&mut second, &taken(&made[0], FILE_REVISION + 1));
    assert_eq!(queued(&second), made[1..]);
    assert_eq!(on_the_wire(&second), Some(made[1].clone()));
}

// ── Whose, and when ──────────────────────────────────────────────────────────────

/// A queued step saved into the drawing and read back says the same of who
/// queued it and when; an entry a version from before saved — saying
/// neither — still reads, as one that says nobody.
#[test]
fn who_queued_a_step_and_when_survive_a_save_and_an_old_entry_still_reads() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut app = drawing_in_a_room("u1", &[old_entry("op-file-r-file-1")]);
    set(&mut app, "1.6MPa");
    let made = app.collab.outbox.back().expect("the step").clone();
    assert_eq!(made.user_id.as_deref(), Some("u1"));
    let when = made.queued_at.expect("when it was queued");
    assert!(
        when > 1_704_067_200 && when < 4_102_444_800,
        "Unix seconds: {when}"
    );
    save(&mut app, &path);

    let back = collab_state::read_queue(&read_back(&path));
    assert!(back.unreadable.is_empty(), "{:?}", back.unreadable);
    let read: Vec<(&str, Option<&str>, Option<i64>)> = back
        .ops
        .iter()
        .map(|op| (op.op_id.as_str(), op.user_id.as_deref(), op.queued_at))
        .collect();
    assert_eq!(
        read,
        [
            ("op-file-r-file-1", None, None),
            (made.op_id.as_str(), Some("u1"), Some(when)),
        ]
    );
}

/// The queue somebody else left in the drawing is not this person's to
/// send. The `hello` claims nothing, the server takes the session as it
/// takes any copy that claims nothing, and the copy holds the connection
/// apart itself: nothing goes out, this person's own steps queue behind,
/// and they are told whose steps those are.
#[test]
fn somebody_elses_queue_is_not_sent_and_the_person_is_told_whose_it_is() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut theirs = drawing_in_a_room("u1", &[]);
    set(&mut theirs, "1.6MPa");
    save(&mut theirs, &path);
    drop(theirs);

    let mut mine = reopened(&path, "u2");
    let TagsSaid::Apart(apart) = ask(&mut mine, 1) else {
        panic!("the hello claims: {:?}", mine.collab.tags_said);
    };
    assert_eq!(apart.code, super::QUEUED_BY_OTHERS);
    assert!(apart.message.contains("u1"), "{apart:?}");

    frame(&mut mine, &welcome("s-1", FILE_REVISION, json!({})));
    assert_eq!(on_the_wire(&mine), None);
    let status = mine.collab.status_json();
    assert_eq!(
        status["not_aligned"]["code"],
        super::QUEUED_BY_OTHERS,
        "{status}"
    );
    let said = mine.command_line.history_plain_text();
    assert!(said.contains("u1"), "{said}");
    assert!(said.contains("需要取得最新图纸"), "{said}");
    set(&mut mine, "2.5MPa");
    assert_eq!(mine.collab.outbox.len(), 2, "this person's step queues");
    assert_eq!(on_the_wire(&mine), None, "and stays there");
}

/// A step the file says nobody queued is not this person's either.
#[test]
fn a_step_that_says_nobody_queued_it_is_not_sent() {
    let mut app = drawing_in_a_room("u1", &[old_entry("op-file-r-file-1")]);
    let TagsSaid::Apart(apart) = ask(&mut app, 1) else {
        panic!("the hello claims: {:?}", app.collab.tags_said);
    };
    assert_eq!(apart.code, super::QUEUED_BY_OTHERS);
    frame(&mut app, &welcome("s-1", FILE_REVISION, json!({})));
    assert_eq!(on_the_wire(&app), None);
}

// ── A queue that cannot be sent as it is ────────────────────────────────────────

/// Two steps under one name — two runs of a version that counted from 1
/// each start — would be taken for one when the room answers either: the
/// connection is held apart, and the person is told which name.
#[test]
fn two_steps_under_one_name_hold_the_connection_apart() {
    let twice = "op-file-r-file-1";
    let mut app = drawing_in_a_room("u1", &[entry(twice, "u1"), entry(twice, "u1")]);
    let TagsSaid::Apart(apart) = ask(&mut app, 1) else {
        panic!("the hello claims: {:?}", app.collab.tags_said);
    };
    assert_eq!(apart.code, super::QUEUE_DUPLICATE_ID);
    assert!(apart.message.contains(twice), "{apart:?}");
    frame(&mut app, &welcome("s-1", FILE_REVISION, json!({})));
    assert_eq!(on_the_wire(&app), None);
    let status = app.collab.status_json();
    assert_eq!(
        status["not_aligned"]["code"],
        super::QUEUE_DUPLICATE_ID,
        "{status}"
    );
}

/// An entry of the queue that cannot be read is a step the room would never
/// hear of: the connection is held apart. The entry is kept as it was
/// found, and a save puts it back, so the next open finds the queue short
/// again rather than whole.
#[test]
fn an_entry_that_cannot_be_read_holds_the_connection_apart_and_is_saved_back() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let unreadable = json!({"op_id": 7, "payload": "?"});
    let mut app = drawing_in_a_room("u1", &[entry("op-a", "u1"), unreadable.clone()]);
    let TagsSaid::Apart(apart) = ask(&mut app, 1) else {
        panic!("the hello claims: {:?}", app.collab.tags_said);
    };
    assert_eq!(apart.code, super::QUEUE_UNREADABLE);
    frame(&mut app, &welcome("s-1", FILE_REVISION, json!({})));
    assert_eq!(on_the_wire(&app), None);
    save(&mut app, &path);

    let back = collab_state::read_queue(&read_back(&path));
    let ops: Vec<&str> = back.ops.iter().map(|op| op.op_id.as_str()).collect();
    assert_eq!(ops, ["op-a"]);
    assert_eq!(back.unreadable, [unreadable]);
}

/// What is not JSON at all is kept too, as the text it is, and reads back
/// as unreadable again.
#[test]
fn text_that_is_not_json_is_kept_as_it_was_found() {
    let mut doc = Document::new();
    crate::pid::graph_data::store(&mut doc, PID_AUTHOR_COLLAB_QUEUE, &[entry("op-a", "u1")]);
    let owner = doc.header.model_space_block_handle;
    doc.xrecord_mut(owner, PID_AUTHOR_COLLAB_QUEUE)
        .expect("the queue record")
        .entries
        .push(XRecordEntry::string(1, "{\"op_id\": \"op-b\", \"pay"));
    let read = collab_state::read_queue(&doc);
    assert_eq!(read.ops.len(), 1);
    assert_eq!(read.unreadable, [json!("{\"op_id\": \"op-b\", \"pay")]);

    collab_state::store_queue_keeping(&mut doc, &read.ops, &read.unreadable);
    let again = collab_state::read_queue(&doc);
    assert_eq!(again.ops.len(), 1);
    assert_eq!(again.unreadable, read.unreadable);
}

// ── The tags the hello lists ─────────────────────────────────────────────────────

/// R2 of `SPEC-collab-offline-sync.md` §11.2: the first `hello` after a
/// start lists the drawing's tags with the queue the file brought already
/// in it. A symbol placed by a step the room has not heard of is not
/// listed — listed, its tag would be taken by the time the step arrives,
/// and the server would give the symbol another.
#[test]
fn a_symbol_the_file_still_has_to_send_is_not_listed() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut first = drawing_in_a_room("u1", &[]);
    let placed = first.automation_op(r#"{"op":"run","cmd":"PIDINSERT VALVE 100,200"}"#);
    assert_eq!(placed["ok"], true, "{placed}");
    first.finish_all_pending_history();
    assert_eq!(first.collab.outbox.len(), 1, "the placing waits");
    let valve = tags_on(&first)
        .into_iter()
        .find(|tag| tag != PUMP)
        .expect("the valve's tag");
    save(&mut first, &path);
    drop(first);

    let mut second = reopened(&path, "u1");
    let TagsSaid::Tags(listed, _) = ask(&mut second, 1) else {
        panic!("the hello claims nothing: {:?}", second.collab.tags_said);
    };
    assert_eq!(listed, [PUMP], "{valve} is not the room's yet");
}

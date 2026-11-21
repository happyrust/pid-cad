//! Restored ops the room refuses (`OFF-8`; `SPEC-collab-offline-sync.md`
//! §5.4).
//!
//! Steps queued while the session was down go into the file with the
//! drawing, each with its inverse (`OFF-7`), and come back with it when it
//! is opened again — with no undo step behind them. When the room refuses
//! one, its inverse takes it back: item by item, each only where the
//! drawing is still as the step left it, written the way a peer's change
//! is and said to nobody. A refusal that yields to the room
//! (`OP_SUPERSEDED`, `OBJECT_DELETED`) takes nothing back. What was queued
//! behind it on the objects it made, renamed or erased goes with it, from
//! the last back; the rest goes out. Each op that goes is said on the
//! command line and logged as a refusal, and a restored one is put on the
//! unsynced list (`refused`), which a save writes into the file. One with
//! no inverse cannot be taken back: the drawing is marked
//! `ROLLBACK_FAILED`.
//!
//! The steps are made for real in one start of the program, offline, and
//! saved; another start opens the file and comes into the room, where the
//! server's frames are read the way the transport reads them.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::CollabEvent;
use crate::app::{Message, OpenCADStudio};
use crate::pid::collab_state::{self, ROLLBACK_FAILED};
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::topology::handle_text;
use crate::pid::FileCollabState;
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";
/// The block the two pumps are drawn with: not one of the symbol library's.
const PUMP_BLOCK: &str = "PID_TEST_pump";
const PUMP: &str = "P-1001";
const OTHER_PUMP: &str = "P-1002";
const PRESSURE: &str = "设计压力";
const TEMPERATURE: &str = "设计温度";
/// What the drawing file says of the room it was in, and how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;

// ── One start, offline ───────────────────────────────────────────────────

/// An editor, as `u1`, on a drawing with two pumps on it, each with a
/// design pressure of 1.0MPa, from a file that says the drawing was in room
/// [`FILE_REPO`] at [`FILE_REVISION`]. Never connected: every step made in
/// it is queued.
fn offline() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    let i = app.active_tab;
    let outline = Line::from_points(Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0));
    app.tabs[i]
        .scene
        .define_block_raw(PUMP_BLOCK, Vector3::ZERO, vec![EntityType::Line(outline)]);
    let document = &mut app.tabs[i].scene.document;
    for (tag, x) in [(PUMP, 0.0), (OTHER_PUMP, 50.0)] {
        let handle = document
            .add_entity(EntityType::Insert(Insert::new(
                PUMP_BLOCK,
                Vector3::new(x, 0.0, 0.0),
            )))
            .expect("entity");
        PidAuthor {
            type_code: "EQUIPMENT".to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            props: r#"{"设计压力":"1.0MPa"}"#.to_owned(),
            ..PidAuthor::default()
        }
        .store(document, handle);
    }
    collab_state::store_state(
        document,
        Some(&FileCollabState {
            drawing_id: DRAWING.to_owned(),
            repo_id: FILE_REPO.to_owned(),
            revision: FILE_REVISION,
        }),
    );
    as_user(&mut app);
    app.collab.document = Some(app.tabs[i].id);
    app
}

fn as_user(app: &mut OpenCADStudio) {
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
}

/// An [`offline`] drawing after `steps`, every op they queued with its
/// inverse read.
fn made_offline(steps: impl FnOnce(&mut OpenCADStudio)) -> OpenCADStudio {
    let mut app = offline();
    steps(&mut app);
    app.read_queued_inverses();
    assert!(
        app.collab
            .outbox
            .iter()
            .all(|entry| entry.inverse.is_some()),
        "every inverse read: {:?}",
        app.collab.outbox
    );
    app
}

/// The ops [`made_offline`] by `steps`, saved into `path`: their ids, in
/// order.
fn saved_offline(path: &Path, steps: impl FnOnce(&mut OpenCADStudio)) -> Vec<String> {
    let mut app = made_offline(steps);
    let made = queued(&app);
    save(&mut app, path);
    made
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert(
            "request_id".into(),
            json!(format!("collab-restored-{serial}")),
        );
    }
    let (response, task) = app.control_request(req);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    response
}

/// Run a command that asks for nothing more; the step is closed.
fn run(app: &mut OpenCADStudio, cmd: &str) {
    let done = request(app, json!({"op":"run","cmd":cmd}));
    app.finish_all_pending_history();
    assert_eq!(done["ok"], true, "{cmd}: {done}");
}

/// `field` of the object tagged `tag` set to `value`, as one step.
fn set(app: &mut OpenCADStudio, tag: &str, field: &str, value: &str) {
    run(app, &format!("PIDSETPROP {tag} {field} {value}"));
}

fn retag(app: &mut OpenCADStudio, from: &str, to: &str) {
    run(app, &format!("PIDTAG {from} {to}"));
}

/// A valve placed with PIDINSERT: its tag.
fn place_valve(app: &mut OpenCADStudio) -> String {
    let before = tags(app);
    run(app, "PIDINSERT VALVE 100,200");
    let mut placed: Vec<String> = tags(app)
        .into_iter()
        .filter(|tag| !before.contains(tag))
        .collect();
    assert_eq!(placed.len(), 1, "one valve placed: {placed:?}");
    placed.remove(0)
}

fn erase(app: &mut OpenCADStudio, tag: &str) {
    let handle = handle_of(app, tag).unwrap_or_else(|| panic!("no object tagged {tag}"));
    let selected = request(app, json!({"op":"select","handles":[handle_text(handle)]}));
    assert_eq!(selected["ok"], true, "{selected}");
    let erased = request(app, json!({"op":"run","cmd":"ERASE"}));
    assert_eq!(erased["ok"], true, "{erased}");
    app.finish_all_pending_history();
}

/// Save the drawing in front to `path`.
fn save(app: &mut OpenCADStudio, path: &Path) {
    let saved = app.automation_op(&json!({"op": "save", "path": path}).to_string());
    assert_eq!(saved["ok"], true, "{saved}");
}

/// The drawing saved at `path`, as the next open reads it.
fn read_back(path: &Path) -> Document {
    let bytes = std::fs::read(path).expect("the saved drawing");
    crate::io::load_bytes("PID-1101.dwg", bytes).expect("the saved drawing reads")
}

// ── Another start, in the room ──────────────────────────────────────────

/// Another start of the program on the drawing saved at `path`, in the
/// room: the drawing's queue restored, and the session welcomed at the
/// file's revision and as many more as `replay` has changes, each of which
/// comes as a peer's op. The first restored op is then on the wire.
fn back_in_the_room(path: &Path, replay: &[Value]) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let opened = app.automation_op(&json!({"op": "open", "path": path}).to_string());
    assert_eq!(opened["ok"], true, "{opened}");
    as_user(&mut app);
    app.collab.document = Some(app.tabs[app.active_tab].id);
    let _ = app.update(Message::CollabEvent(CollabEvent::TagsWanted { ask: 1 }));
    assert!(
        !app.collab.outbox.is_empty() && app.collab.outbox.iter().all(|entry| entry.restored),
        "the queue restored: {:?}",
        app.collab.outbox
    );
    let revision = FILE_REVISION + replay.len() as i64;
    frame(
        &mut app,
        &json!({
            "type": "welcome",
            "session_id": "s-1",
            "base_revision": revision,
            "peers": [],
            "claims": [],
            "repo_id": FILE_REPO,
            "replay_from": FILE_REVISION,
        })
        .to_string(),
    );
    for (n, change) in replay.iter().enumerate() {
        frame(
            &mut app,
            &json!({
                "type": "op_applied",
                "op_id": format!("op-peer-{n}"),
                "revision": FILE_REVISION + 1 + n as i64,
                "changes": [change],
            })
            .to_string(),
        );
    }
    assert_eq!(app.collab.last_apply_error, None, "the replay was written");
    app
}

/// One frame from the server, read the way the transport reads it, and
/// what it says taken the way the app takes it.
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

/// The server refuses the op on the wire, whole, with `code`: the op.
fn refuse(app: &mut OpenCADStudio, code: &str) -> String {
    let op_id = on_the_wire(app).expect("an op on the wire");
    frame(
        app,
        &json!({
            "type": "error",
            "code": code,
            "message": format!("测试拒绝 {code}"),
            "op_id": op_id,
            "rejected": true,
        })
        .to_string(),
    );
    op_id
}

/// A peer's change of `field` of `tag` to `value`, as the room says it.
fn peer_set(tag: &str, field: &str, value: &str) -> Value {
    json!({"kind": "set_property", "object_ref": tag, "field": field, "value": value})
}

/// A peer's erasing of `tag`, as the room says it.
fn peer_erase(tag: &str) -> Value {
    json!({"kind": "delete_object", "object_ref": tag})
}

// ── What is there to see ────────────────────────────────────────────────

/// The op the drawing has on the wire.
fn on_the_wire(app: &OpenCADStudio) -> Option<String> {
    let mut sent = app.collab.outbox.iter().filter(|entry| entry.sent);
    let first = sent.next()?;
    assert!(sent.next().is_none(), "more than one op on the wire");
    Some(first.op_id.clone())
}

/// The ids of the queued ops, in order.
fn queued(app: &OpenCADStudio) -> Vec<String> {
    app.collab
        .outbox
        .iter()
        .map(|entry| entry.op_id.clone())
        .collect()
}

fn handle_of(app: &OpenCADStudio, tag: &str) -> Option<Handle> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .find(|handle| PidAuthor::load(document, *handle).is_some_and(|author| author.tag == tag))
}

/// Every tag an object of the drawing in front carries, each once, in
/// order.
fn tags(app: &OpenCADStudio) -> Vec<String> {
    let document = &app.tabs[app.active_tab].scene.document;
    let mut tags: Vec<String> = document
        .entities()
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .map(|author| author.tag)
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// What `field` of the object tagged `tag` holds.
fn value_of(app: &OpenCADStudio, tag: &str, field: &str) -> Option<String> {
    let handle = handle_of(app, tag).unwrap_or_else(|| panic!("no object tagged {tag}"));
    let document = &app.tabs[app.active_tab].scene.document;
    let author = PidAuthor::load(document, handle).expect("a record");
    split_props(&author.props).0.get(field).cloned()
}

/// The refused entries of the unsynced list the drawing in front has in
/// this run, as `(op, code, rolled back)`.
fn refused(app: &OpenCADStudio) -> Vec<(String, String, Option<bool>)> {
    let id = app.tabs[app.active_tab].id;
    app.collab
        .refused
        .get(&id)
        .into_iter()
        .flatten()
        .map(|entry| {
            assert_eq!(entry["state"], "refused", "{entry}");
            (
                entry["op_id"].as_str().unwrap_or_default().to_owned(),
                entry["code"].as_str().unwrap_or_default().to_owned(),
                entry["rolled_back"].as_bool(),
            )
        })
        .collect()
}

/// What the refusal log says, as `(op, code)`.
fn logged(app: &OpenCADStudio) -> Vec<(String, String)> {
    app.collab
        .rejection_log
        .iter()
        .map(|entry| {
            (
                entry["op_id"].as_str().unwrap_or_default().to_owned(),
                entry["code"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

fn said(app: &OpenCADStudio) -> String {
    app.command_line.history_plain_text()
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-restored-{}-{serial}",
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

// ── Refused, and taken back ─────────────────────────────────────────────

/// `step` made offline, and behind it a step on the other pump that has
/// nothing to do with it; saved, opened again and back in the room, where
/// the server refuses the first with `code`. It is taken back, and the one
/// behind goes out. The editor, as the refusal left it.
#[track_caller]
fn refused_and_taken_back(code: &str, step: impl FnOnce(&mut OpenCADStudio)) -> OpenCADStudio {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let made = saved_offline(&path, |app| {
        step(app);
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    assert_eq!(made.len(), 2, "two ops queued: {made:?}");
    let mut app = back_in_the_room(&path, &[]);
    assert_eq!(on_the_wire(&app), Some(made[0].clone()));
    assert_eq!(refuse(&mut app, code), made[0]);

    assert_eq!(app.collab.diverged, None, "{code}: taken back, not apart");
    assert_eq!(app.collab.last_apply_error, None, "{code}");
    assert_eq!(
        on_the_wire(&app),
        Some(made[1].clone()),
        "{code}: the one behind goes out"
    );
    assert_eq!(queued(&app), made[1..], "{code}");
    assert_eq!(
        value_of(&app, OTHER_PUMP, TEMPERATURE).as_deref(),
        Some("80C"),
        "{code}: the one behind stays"
    );
    assert_eq!(
        refused(&app),
        [(made[0].clone(), code.to_owned(), Some(true))]
    );
    assert_eq!(logged(&app), owned(&[(made[0].as_str(), code)]));
    assert!(
        said(&app).contains(code),
        "{code}: the command line says so: {}",
        said(&app)
    );
    app
}

#[test]
fn claim_held_takes_a_property_change_back() {
    let app = refused_and_taken_back("CLAIM_HELD", |app| set(app, PUMP, PRESSURE, "1.6MPa"));
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("1.0MPa"));
}

#[test]
fn perm_denied_role_takes_a_placed_symbol_back() {
    let app = refused_and_taken_back("PERM_DENIED_ROLE", |app| {
        place_valve(app);
    });
    assert_eq!(tags(&app), [PUMP, OTHER_PUMP]);
}

#[test]
fn object_referenced_puts_an_erased_symbol_back() {
    let app = refused_and_taken_back("OBJECT_REFERENCED", |app| erase(app, PUMP));
    assert_eq!(tags(&app), [PUMP, OTHER_PUMP]);
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("1.0MPa"));
}

#[test]
fn stale_base_revision_takes_a_retag_back() {
    let app = refused_and_taken_back("STALE_BASE_REVISION", |app| {
        retag(app, PUMP, "P-1009");
    });
    assert_eq!(tags(&app), [PUMP, OTHER_PUMP]);
}

// ── Yielding to the room ────────────────────────────────────────────────

/// The pump's design pressure changed offline, and behind it a step on the
/// other pump; back in the room, the replay brings `replay`, and the server
/// refuses the change with `code`. The editor, and the two ops.
fn the_pressure_refused_after(replay: &[Value], code: &str) -> (OpenCADStudio, Vec<String>) {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let made = saved_offline(&path, |app| {
        set(app, PUMP, PRESSURE, "1.6MPa");
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    let mut app = back_in_the_room(&path, replay);
    assert_eq!(refuse(&mut app, code), made[0]);
    (app, made)
}

/// The room took somebody else's value: it is in the drawing already, and
/// nothing is taken back.
#[test]
fn op_superseded_takes_nothing_back() {
    let (app, made) =
        the_pressure_refused_after(&[peer_set(PUMP, PRESSURE, "2.5MPa")], "OP_SUPERSEDED");
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("2.5MPa"));
    assert_eq!(app.collab.diverged, None);
    assert_eq!(app.collab.last_apply_error, None);
    assert_eq!(
        refused(&app),
        [(made[0].clone(), "OP_SUPERSEDED".to_owned(), Some(true))]
    );
    assert_eq!(on_the_wire(&app), Some(made[1].clone()));
}

/// The object is gone in the room, and so in the drawing: nothing to take
/// back, and nothing said of a change that found no object.
#[test]
fn object_deleted_takes_nothing_back() {
    let (app, made) = the_pressure_refused_after(&[peer_erase(PUMP)], "OBJECT_DELETED");
    assert_eq!(tags(&app), [OTHER_PUMP]);
    assert_eq!(app.collab.diverged, None);
    assert_eq!(app.collab.last_apply_error, None);
    assert_eq!(
        refused(&app),
        [(made[0].clone(), "OBJECT_DELETED".to_owned(), Some(true))]
    );
    assert_eq!(on_the_wire(&app), Some(made[1].clone()));
}

/// Somebody changed the same field meanwhile: the drawing has their value,
/// not the one this step wrote, and it is not changed back to the value
/// from before the step.
#[test]
fn somebody_elses_value_on_the_same_field_stays() {
    let (app, made) =
        the_pressure_refused_after(&[peer_set(PUMP, PRESSURE, "2.5MPa")], "CLAIM_HELD");
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("2.5MPa"));
    assert_eq!(app.collab.diverged, None);
    assert_eq!(
        refused(&app),
        [(made[0].clone(), "CLAIM_HELD".to_owned(), Some(true))]
    );
    assert_eq!(on_the_wire(&app), Some(made[1].clone()));
}

// ── What was built on it ────────────────────────────────────────────────

/// A valve placed offline, its pressure set, and a step on the other pump;
/// saved in `scratch`, opened again, back in the room, and the placing
/// refused. The editor, and the three ops.
fn a_valve_and_what_was_built_on_it_refused(scratch: &Scratch) -> (OpenCADStudio, Vec<String>) {
    let path = scratch.drawing();
    let made = saved_offline(&path, |app| {
        let valve = place_valve(app);
        set(app, &valve, PRESSURE, "0.6MPa");
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    assert_eq!(made.len(), 3, "three ops queued: {made:?}");
    let mut app = back_in_the_room(&path, &[]);
    assert_eq!(refuse(&mut app, "PERM_DENIED_ROLE"), made[0]);
    (app, made)
}

/// The step on the valve goes with the valve, from the last back, each
/// logged and put on the unsynced list; the step on the other pump goes
/// out.
#[test]
fn what_was_queued_on_the_refused_op_goes_with_it_and_the_rest_goes_out() {
    let scratch = Scratch::new();
    let (app, made) = a_valve_and_what_was_built_on_it_refused(&scratch);
    assert_eq!(tags(&app), [PUMP, OTHER_PUMP], "the valve is gone");
    assert_eq!(app.collab.diverged, None);
    assert_eq!(app.collab.last_apply_error, None);
    assert_eq!(queued(&app), made[2..]);
    assert_eq!(on_the_wire(&app), Some(made[2].clone()));
    assert_eq!(
        value_of(&app, OTHER_PUMP, TEMPERATURE).as_deref(),
        Some("80C")
    );
    assert_eq!(
        refused(&app),
        [
            (made[0].clone(), "PERM_DENIED_ROLE".to_owned(), Some(true)),
            (made[1].clone(), "PERM_DENIED_ROLE".to_owned(), Some(true)),
        ]
    );
    assert_eq!(
        logged(&app),
        owned(&[
            (made[0].as_str(), "PERM_DENIED_ROLE"),
            (made[1].as_str(), "PERM_DENIED_ROLE"),
        ])
    );
}

/// A step made after the drawing was opened again, on the valve whose
/// placing is then refused: it goes back by the undo stack, as any step of
/// this run does, and can be done again; it is logged, and is not on the
/// unsynced list. The restored step on the other pump goes out.
#[test]
fn a_step_made_after_reopening_on_the_refused_op_goes_back_by_the_undo_stack() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut valve = String::new();
    let made = saved_offline(&path, |app| {
        valve = place_valve(app);
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    let mut app = back_in_the_room(&path, &[]);
    assert_eq!(on_the_wire(&app), Some(made[0].clone()));
    set(&mut app, &valve, PRESSURE, "0.6MPa");
    let all = queued(&app);
    assert_eq!(all.len(), 3, "the new step is queued behind: {all:?}");
    let i = app.active_tab;
    let redo_before = app.tabs[i].history.redo_stack.len();

    assert_eq!(refuse(&mut app, "CLAIM_HELD"), made[0]);
    assert_eq!(tags(&app), [PUMP, OTHER_PUMP], "the valve is gone");
    assert_eq!(
        app.tabs[i].history.redo_stack.len(),
        redo_before + 1,
        "the new step can be done again"
    );
    assert_eq!(app.collab.diverged, None);
    assert_eq!(queued(&app), made[1..]);
    assert_eq!(on_the_wire(&app), Some(made[1].clone()));
    assert_eq!(
        refused(&app),
        [(made[0].clone(), "CLAIM_HELD".to_owned(), Some(true))]
    );
    assert_eq!(
        logged(&app),
        owned(&[
            (made[0].as_str(), "CLAIM_HELD"),
            (all[2].as_str(), "CLAIM_HELD")
        ])
    );
}

// ── Not taken back ──────────────────────────────────────────────────────

/// Held in a conflict entry, the op is answered as far as the queue goes:
/// it leaves the queue, what it did stays in the drawing until the ruling,
/// and the one behind goes out.
#[test]
fn a_restored_op_in_a_conflict_stays_on_the_drawing_and_leaves_the_queue() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let made = saved_offline(&path, |app| {
        set(app, PUMP, PRESSURE, "1.6MPa");
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    let mut app = back_in_the_room(&path, &[]);
    assert_eq!(on_the_wire(&app), Some(made[0].clone()));
    frame(
        &mut app,
        &json!({
            "type": "conflict",
            "conflict_id": "c-1",
            "object_ref": PUMP,
            "field": PRESSURE,
            "value_a": "1.6MPa",
            "value_b": "2.5MPa",
            "op_ids": [made[0]],
        })
        .to_string(),
    );
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("1.6MPa"));
    assert_eq!(app.collab.conflicts.len(), 1);
    assert_eq!(queued(&app), made[1..]);
    assert_eq!(on_the_wire(&app), Some(made[1].clone()));
    assert!(refused(&app).is_empty(), "{:?}", refused(&app));
    assert!(logged(&app).is_empty(), "{:?}", logged(&app));
    assert_eq!(app.collab.diverged, None);
}

/// An op saved before its inverse was read cannot be taken back: the
/// drawing is marked, its entry on the unsynced list says it was not taken
/// back, and so does the file once it is saved.
#[test]
fn a_restored_op_without_an_inverse_cannot_be_taken_back() {
    let scratch = Scratch::new();
    let path = scratch.drawing();
    let mut first = made_offline(|app| {
        set(app, PUMP, PRESSURE, "1.6MPa");
        set(app, OTHER_PUMP, TEMPERATURE, "80C");
    });
    for entry in first.collab.outbox.iter_mut() {
        entry.inverse = None;
    }
    let made = queued(&first);
    save(&mut first, &path);
    drop(first);

    let mut app = back_in_the_room(&path, &[]);
    assert!(
        app.collab
            .outbox
            .iter()
            .all(|entry| entry.inverse.is_none()),
        "saved without: {:?}",
        app.collab.outbox
    );
    assert_eq!(refuse(&mut app, "CLAIM_HELD"), made[0]);
    assert_eq!(
        app.collab.diverged_as.map(|(code, _)| code),
        Some(ROLLBACK_FAILED)
    );
    assert_eq!(value_of(&app, PUMP, PRESSURE).as_deref(), Some("1.6MPa"));
    assert_eq!(
        refused(&app),
        [(made[0].clone(), "CLAIM_HELD".to_owned(), Some(false))]
    );
    assert_eq!(logged(&app), owned(&[(made[0].as_str(), "CLAIM_HELD")]));

    let again = scratch.0.join("again.dwg");
    save(&mut app, &again);
    let back = read_back(&again);
    assert_eq!(
        collab_state::load_diverged(&back).map(|mark| mark.code),
        Some(ROLLBACK_FAILED.to_owned())
    );
    let list = collab_state::read_unsynced(&back);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["state"], "refused");
    assert_eq!(list[0]["op_id"], made[0].as_str());
    assert_eq!(list[0]["rolled_back"], false);
}

// ── In the file ─────────────────────────────────────────────────────────

/// Every op that went is on the unsynced list in the file once the drawing
/// is saved, and in the refusal log; the drawing is not marked for it.
#[test]
fn what_is_refused_is_on_the_unsynced_list_after_a_save() {
    let scratch = Scratch::new();
    let (mut app, made) = a_valve_and_what_was_built_on_it_refused(&scratch);
    let again = scratch.0.join("again.dwg");
    save(&mut app, &again);
    assert_eq!(logged(&app).len(), 2, "the log is as it was");

    let back = read_back(&again);
    assert_eq!(collab_state::load_diverged(&back), None, "not marked");
    let list: Vec<(String, String, String, Value)> = collab_state::read_unsynced(&back)
        .iter()
        .map(|entry| {
            (
                entry["state"].as_str().unwrap_or_default().to_owned(),
                entry["op_id"].as_str().unwrap_or_default().to_owned(),
                entry["code"].as_str().unwrap_or_default().to_owned(),
                entry["rolled_back"].clone(),
            )
        })
        .collect();
    assert_eq!(
        list,
        [
            (
                "refused".to_owned(),
                made[0].clone(),
                "PERM_DENIED_ROLE".to_owned(),
                json!(true)
            ),
            (
                "refused".to_owned(),
                made[1].clone(),
                "PERM_DENIED_ROLE".to_owned(),
                json!(true)
            ),
        ]
    );
    let queue: Vec<String> = collab_state::load_queue(&back)
        .into_iter()
        .map(|op| op.op_id)
        .collect();
    assert_eq!(
        queue,
        made[2..],
        "the one that went out is still unanswered"
    );
}

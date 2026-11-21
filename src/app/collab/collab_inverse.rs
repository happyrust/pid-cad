//! What takes a queued op back out of the drawing (`OFF-7`;
//! `SPEC-collab-offline-sync.md` §4.1, §5.4 and `OS-16`).
//!
//! Each op this session queues keeps, beside what it says, what undoes it:
//! its `inverse`, in the shape of the changes the client sends. It is read
//! while the editor has nothing else on hand, by taking the op's steps back
//! and putting them on again without a word to anyone, and goes into the
//! file with the op.
//!
//! Every kind of change the client sends gets a step made while the session
//! is down. Its inverse is read; the op leaves the queue, as a refused one
//! does; and the inverse is written back the way a peer's change comes in.
//! The drawing is then what it was before the step: the entities that carry
//! an author record, and the topology table — neither by handle, as an
//! entity written back has whatever handle the drawing gives it.
//!
//! Then what reading leaves as it was — the stacks, the selection, the view,
//! whether the drawing is changed, what goes out, a command under way — and
//! the inverse in the file.

use crate::app::collab::{CollabEvent, Outgoing};
use crate::app::tab_document::HistorySnapshot;
use crate::app::{Message, OpenCADStudio};
use crate::pid::persist::{PidAuthor, XDATA_SOFT_LIMIT};
use crate::pid::propagate::{join_props, split_props};
use crate::pid::topology::{
    handle_text, parse_handle, TopologyTable, ROLE_PIPE_LINE, ROLE_PIPE_RUN, ROLE_PIPE_SEGMENT,
    TYPE_VALVE,
};
use crate::pid::QueuedOp;
use document::entities::Insert;
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";
const LINE: [[f64; 2]; 2] = [[0.0, 0.0], [200.0, 0.0]];
const VALVE_AT: [[f64; 2]; 1] = [[100.0, 0.0]];
/// The pipe line [`LINE`] draws on a drawing that has none.
const LINE_TAG: &str = "L-1001";
const GRADE: &str = "管道等级";
const PRESSURE: &str = "设计压力";
/// The symbol [`place_equipment`] places, and its tag.
const BLOCK: &str = "PID_SYM_equipment_vessel";
const EQUIPMENT: &str = "E-9001";
/// The one of [`many_props`] a step changes.
const FIELD: &str = "字段60";

// ── The editor and what is done in it ───────────────────────────────────

/// An editor on a new drawing with no session yet: what is done in it stays
/// in it.
fn drawing() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
    app
}

/// A session comes up on the drawing.
fn connected(app: &mut OpenCADStudio) {
    app.collab.apply(CollabEvent::Connected {
        session_id: "s-1".to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    });
}

/// A session comes up on the drawing and drops: from here on every step
/// waits in the queue.
fn session_dropped(app: &mut OpenCADStudio) {
    connected(app);
    app.collab.apply(CollabEvent::Disconnected {
        reason: "connection reset".to_owned(),
        will_retry: true,
    });
    assert!(
        app.collab.outbox.is_empty(),
        "nothing done before was queued"
    );
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert(
            "request_id".into(),
            json!(format!("collab-inverse-{serial}")),
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

/// Start `cmd`, give it `points`, press Enter; the step is closed.
fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
    let started = request(app, json!({"op":"run","cmd":cmd}));
    assert_eq!(started["ok"], true, "{cmd}: {started}");
    for point in points {
        let fed = request(
            app,
            json!({"op":"input","kind":"point","point":[point[0], point[1], 0.0],"space":"wcs"}),
        );
        assert_eq!(fed["ok"], true, "{cmd}: {fed}");
    }
    let done = request(app, json!({"op":"input","kind":"enter"}));
    app.finish_all_pending_history();
    assert_eq!(done["ok"], true, "{cmd}: {done}");
}

fn select(app: &mut OpenCADStudio, handles: &[Handle]) {
    let list: Vec<String> = handles.iter().map(|handle| handle_text(*handle)).collect();
    let selected = request(app, json!({"op":"select","handles":list}));
    assert_eq!(selected["ok"], true, "{selected}");
}

fn erase(app: &mut OpenCADStudio, handles: &[Handle]) {
    assert!(!handles.is_empty(), "nothing to erase");
    select(app, handles);
    let erased = request(app, json!({"op":"run","cmd":"ERASE"}));
    assert_eq!(erased["ok"], true, "{erased}");
    app.finish_all_pending_history();
}

fn undo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"undo"}"#);
}

/// The entities of the drawing in front whose record `keep`s.
fn handles_where(app: &OpenCADStudio, keep: impl Fn(&PidAuthor) -> bool) -> Vec<Handle> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .filter(|handle| PidAuthor::load(document, *handle).is_some_and(|author| keep(&author)))
        .collect()
}

/// A valve placed with PIDINSERT.
fn place_valve(app: &mut OpenCADStudio) {
    run(app, "PIDINSERT VALVE 100,200");
}

/// The tag of the one symbol on the drawing in front.
fn symbol_tag(app: &OpenCADStudio) -> String {
    let document = &app.tabs[app.active_tab].scene.document;
    let tags: Vec<String> = document
        .entities()
        .filter(|entity| matches!(entity, EntityType::Insert(_)))
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .map(|author| author.tag)
        .filter(|tag| !tag.is_empty())
        .collect();
    assert_eq!(tags.len(), 1, "one symbol: {tags:?}");
    tags[0].clone()
}

/// The valve's design pressure set to `value`.
fn set_pressure(app: &mut OpenCADStudio, value: &str) {
    let tag = symbol_tag(app);
    run(app, &format!("PIDSETPROP {tag} {PRESSURE} {value}"));
}

/// `tag` with its number moved on by eight: a tag nothing else has.
fn another(tag: &str) -> String {
    let (prefix, digits) = tag.rsplit_once('-').expect("a tag ending in a number");
    let n: u32 = digits.parse().expect("a tag ending in a number");
    format!("{prefix}-{}", n + 8)
}

fn is_connector(author: &PidAuthor) -> bool {
    author.topology.starts_with("connector_")
}

/// A line drawn, and a connector where it leaves for another drawing.
fn line_and_connector(app: &mut OpenCADStudio) {
    draw(app, "PIDPIPE", &LINE);
    draw(app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]]);
}

/// A line drawn and a run on it.
fn line_and_run(app: &mut OpenCADStudio) {
    draw(app, "PIDPIPE", &LINE);
    draw(app, "PIDPIPE run", &LINE);
}

/// Sixty properties, some three and a half kilobytes of them: more than an
/// entity's own record keeps.
fn many_props() -> BTreeMap<String, String> {
    (1..=60)
        .map(|n| {
            (
                format!("字段{n:02}"),
                format!("值{n:02}-{}", "x".repeat(40)),
            )
        })
        .collect()
}

/// [`EQUIPMENT`] placed with [`many_props`], as one step.
fn place_equipment(app: &mut OpenCADStudio) {
    let props = join_props(&many_props(), &BTreeSet::new());
    assert!(props.len() > XDATA_SOFT_LIMIT, "the fixture");
    let i = app.active_tab;
    if !app.tabs[i].scene.document.block_records.contains(BLOCK) {
        app.pid_symbols.instantiate(&mut app.tabs[i].scene);
    }
    let ports = app.pid_symbols.symbol_ports(BLOCK).join(",");
    app.push_undo_snapshot(i, "PASTECLIP");
    let handle = app.tabs[i].scene.add_entity(EntityType::Insert(Insert::new(
        BLOCK,
        Vector3::new(300.0, 50.0, 0.0),
    )));
    assert!(!handle.is_null(), "the symbol was not added");
    let document = &mut app.tabs[i].scene.document;
    PidAuthor {
        type_code: "EQUIPMENT".to_owned(),
        tag: EQUIPMENT.to_owned(),
        tag_source: "manual".to_owned(),
        tag_status: "ok".to_owned(),
        props,
        ports,
        ..PidAuthor::default()
    }
    .store(document, handle);
    crate::app::commands::pidpalette::sync_pid_tag_attribute(document, handle);
    app.tabs[i].dirty = true;
    app.finish_all_pending_history();
}

/// What [`EQUIPMENT`] has for properties, and whether they are in the
/// entity's own record.
fn equipment_props(app: &OpenCADStudio) -> (BTreeMap<String, String>, bool) {
    let found = handles_where(app, |author| author.tag == EQUIPMENT);
    assert_eq!(found.len(), 1, "one {EQUIPMENT}: {found:?}");
    let document = &app.tabs[app.active_tab].scene.document;
    let record = PidAuthor::load(document, found[0]).expect("a record");
    let entity = document.get_entity(found[0]).expect("the entity");
    (
        split_props(&record.props).0,
        PidAuthor::props_are_inline(entity),
    )
}

/// A JSON object of texts, as the map it is.
fn texts(object: &Value) -> BTreeMap<String, String> {
    object
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

// ── The drawing as the comparison sees it ────────────────────────────────

fn number(value: f64) -> String {
    // Coordinates in these drawings are whole or halves; rounding keeps
    // `-0` and float noise out of the comparison.
    format!("{:.4}", if value.abs() < 1e-9 { 0.0 } else { value })
}

fn at(x: f64, y: f64) -> String {
    format!("{},{}", number(x), number(y))
}

/// What an entity is to the comparison: its kind and where it is.
fn shape(entity: &EntityType) -> String {
    match entity {
        EntityType::Line(line) => format!(
            "line {} {}",
            at(line.start.x, line.start.y),
            at(line.end.x, line.end.y)
        ),
        EntityType::Point(point) => format!("point {}", at(point.location.x, point.location.y)),
        EntityType::Insert(insert) => format!(
            "insert {} {}",
            insert.block_name,
            at(insert.insert_point.x, insert.insert_point.y)
        ),
        EntityType::Circle(circle) => format!("circle {}", at(circle.center.x, circle.center.y)),
        _ => "other".to_owned(),
    }
}

/// What `text` names in `document`, when it is the handle of an entity
/// there: the entity's tag, or what it is and where.
fn named(document: &Document, text: &str) -> Option<String> {
    let handle = parse_handle(text)?;
    let entity = document.get_entity(handle)?;
    let tag = PidAuthor::load(document, handle)
        .map(|author| author.tag)
        .unwrap_or_default();
    Some(if tag.is_empty() { shape(entity) } else { tag })
}

/// The fields of the topology table that hold text, not handles.
const TEXT_FIELDS: [&str; 8] = [
    "tag",
    "line_tag",
    "run_tag",
    "props",
    "overrides",
    "fitting_type",
    "kind",
    "peer_drawing",
];

/// `value` with every handle in it outside [`TEXT_FIELDS`] — alone, or
/// before a port — replaced by what it names, and every list in an order of
/// its own.
fn without_handles(document: &Document, value: &mut Value) {
    match value {
        Value::String(text) => {
            let renamed = match text.split_once(':') {
                Some((handle, port)) => {
                    named(document, handle).map(|name| format!("{name}:{port}"))
                }
                None => named(document, text),
            };
            if let Some(renamed) = renamed {
                *text = renamed;
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                without_handles(document, item);
            }
            items.sort_by_cached_key(|item| item.to_string());
        }
        Value::Object(fields) => {
            for (key, item) in fields.iter_mut() {
                if !TEXT_FIELDS.contains(&key.as_str()) {
                    without_handles(document, item);
                }
            }
        }
        _ => {}
    }
}

/// The drawing in front as the comparison sees it: every entity that
/// carries an author record — what it is, where, what it says, its owner by
/// what the owner is — and the topology table as far as it speaks of
/// entities that are there, neither by handle.
fn picture(app: &OpenCADStudio) -> (Vec<String>, Value) {
    let document = &app.tabs[app.active_tab].scene.document;
    let mut rows: Vec<String> = document
        .entities()
        .filter_map(|entity| {
            let author = PidAuthor::load(document, entity.common().handle)?;
            let (props, marks) = split_props(&author.props);
            let owner = named(document, &author.owner).unwrap_or_else(|| author.owner.clone());
            Some(format!(
                "{} | type={} tag={} topology={} owner={owner} props={props:?} overrides={marks:?}",
                shape(entity),
                author.type_code,
                author.tag,
                author.topology,
            ))
        })
        .collect();
    rows.sort();
    let mut table = TopologyTable::load(document);
    table.prune(document);
    let mut table = serde_json::to_value(&table).expect("the table as JSON");
    without_handles(document, &mut table);
    (rows, table)
}

/// What taking steps back and putting them on again could move besides what
/// the drawing says: the stacks, the selection, the view and the rest of
/// what the editor shows, whether the drawing is changed and its revision —
/// and the drawing's entities by handle.
fn surroundings(app: &OpenCADStudio) -> Value {
    let tab = &app.tabs[app.active_tab];
    let stack = |snapshots: &[HistorySnapshot]| -> Vec<String> {
        snapshots
            .iter()
            .map(|snapshot| match snapshot {
                HistorySnapshot::Delta(delta) => format!(
                    "step {} of {} entities",
                    delta.collab_step,
                    delta.entities.len()
                ),
                _ => "another kind of step".to_owned(),
            })
            .collect()
    };
    let scene = &tab.scene;
    let camera = scene.camera.borrow();
    let mut entities: Vec<String> = scene
        .document
        .entities()
        .map(|entity| format!("{} {}", handle_text(entity.common().handle), shape(entity)))
        .collect();
    entities.sort();
    let mut selected: Vec<String> = scene
        .selected
        .iter()
        .map(|handle| handle_text(*handle))
        .collect();
    selected.sort();
    json!({
        "undo": stack(&tab.history.undo_stack),
        "redo": stack(&tab.history.redo_stack),
        "selected": selected,
        "hover": scene.hover_highlight.map(handle_text),
        "layout": scene.current_layout,
        "layer": tab.active_layer,
        "layiso": tab.layiso_state.is_some(),
        "dirty": tab.dirty,
        "camera": format!(
            "{:?} {:?} {} {} {} {}",
            camera.target, camera.rotation, camera.distance, camera.fov_y, camera.yaw, camera.pitch
        ),
        "camera_generation": scene.camera_generation,
        "edit_revision": tab.edit_revision,
        "entities": entities,
    })
}

// ── One step, its inverse, and the inverse written back ──────────────────

/// The one op waiting in the queue.
#[track_caller]
fn the_queued(app: &OpenCADStudio) -> &Outgoing {
    assert_eq!(
        app.collab.outbox.len(),
        1,
        "one op waits: {:?}",
        app.collab.outbox
    );
    let entry = &app.collab.outbox[0];
    assert!(!entry.sent, "it waits");
    entry
}

/// The kinds of change `payload` says, in order.
fn kinds(payload: &Value) -> Vec<String> {
    let first = payload["kind"].as_str().map(str::to_owned);
    let rest = payload["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| change["kind"].as_str().map(str::to_owned));
    first.into_iter().chain(rest).collect()
}

/// The kinds of change an inverse says, in order.
fn inverse_kinds(inverse: &[Value]) -> Vec<&str> {
    inverse
        .iter()
        .map(|change| change["kind"].as_str().unwrap_or_default())
        .collect()
}

/// What a refusal of the op waiting comes to once its inverse is all there
/// is to go by: the op leaves the queue, and the inverse is written the way
/// a peer's change comes in. The inverse.
#[track_caller]
fn taken_back_by_its_inverse(app: &mut OpenCADStudio) -> Vec<Value> {
    let entry = app.collab.outbox.pop_front().expect("the op waiting");
    let Some(inverse) = entry.inverse else {
        panic!("no inverse was read for {}", entry.payload);
    };
    let revision = app.collab.base_revision + 1;
    app.collab.apply(CollabEvent::OpApplied {
        op_id: "op-inverse".to_owned(),
        revision,
        changes: inverse.clone(),
    });
    app.apply_remote_changes();
    assert_eq!(
        app.collab.last_apply_error, None,
        "the inverse could not be written: {inverse:?}"
    );
    assert_eq!(app.collab.diverged, None);
    inverse
}

/// What a [`case`] leaves.
struct Case {
    /// The editor, its drawing as the inverse left it.
    app: OpenCADStudio,
    /// What the op's inverse says, change by change.
    inverse: Vec<Value>,
    /// Whether the op put its steps on the drawing.
    on: bool,
    /// The drawing before the step (`picture`).
    before: (Vec<String>, Value),
}

/// One step made by `step` while the session is down, on a drawing
/// `set_up` made before: the op it queued says `kind`; its inverse is read
/// and written back.
#[track_caller]
fn taken_back(
    set_up: impl FnOnce(&mut OpenCADStudio),
    step: impl FnOnce(&mut OpenCADStudio),
    kind: &str,
) -> Case {
    let mut app = drawing();
    set_up(&mut app);
    session_dropped(&mut app);
    let before = picture(&app);
    step(&mut app);
    let queued = the_queued(&app);
    let (said, on) = (kinds(&queued.payload), queued.on);
    assert!(
        said.iter().any(|said| said == kind),
        "the step says {said:?}, not {kind}"
    );
    app.read_queued_inverses();
    let inverse = taken_back_by_its_inverse(&mut app);
    Case {
        app,
        inverse,
        on,
        before,
    }
}

/// A step [`taken_back`] by its inverse: the drawing is as it was before
/// the step.
#[track_caller]
fn case(
    set_up: impl FnOnce(&mut OpenCADStudio),
    step: impl FnOnce(&mut OpenCADStudio),
    kind: &str,
) -> Case {
    let case = taken_back(set_up, step, kind);
    assert_eq!(
        picture(&case.app),
        case.before,
        "{kind}: written back, the inverse {:?} left the drawing otherwise",
        case.inverse
    );
    case
}

// ── Every kind the client sends ───────────────────────────────────────────

#[test]
fn a_property_changed() {
    let case = case(
        |app| {
            place_valve(app);
            set_pressure(app, "1.0MPa");
        },
        |app| set_pressure(app, "1.6MPa"),
        "set_property",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["set_property"]);
    assert_eq!(case.inverse[0]["value"], "1.0MPa", "{:?}", case.inverse);
}

#[test]
fn a_retag() {
    let case = case(
        place_valve,
        |app| {
            let tag = symbol_tag(app);
            run(app, &format!("PIDTAG {tag} {}", another(&tag)));
        },
        "set_tag",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["set_tag"]);
}

#[test]
fn a_symbol_placed() {
    let case = case(|_| {}, place_valve, "insert_symbol");
    assert_eq!(inverse_kinds(&case.inverse), ["delete_object"]);
}

#[test]
fn a_symbol_erased() {
    let case = case(
        place_valve,
        |app| {
            let tag = symbol_tag(app);
            let valve = handles_where(app, |author| author.tag == tag);
            erase(app, &valve);
        },
        "delete_object",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["insert_symbol"]);
}

#[test]
fn a_connector_made() {
    let case = case(
        |app| draw(app, "PIDPIPE", &LINE),
        |app| draw(app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]]),
        "create_connector",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["delete_connector"]);
}

#[test]
fn a_connector_erased() {
    let case = case(
        line_and_connector,
        |app| {
            let connector = handles_where(app, is_connector);
            erase(app, &connector);
        },
        "delete_connector",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["create_connector"]);
}

#[test]
fn a_pipe_line_drawn() {
    let case = case(
        |_| {},
        |app| draw(app, "PIDPIPE", &LINE),
        "create_pipe_line",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["delete_object"]);
}

/// A run drawn is taken out again as erasing it would take it out: the
/// line's run counter stays where the run moved it, a run's number not
/// being given back however the run went (§11.10). Nothing else differs.
#[test]
fn a_run_drawn() {
    let case = taken_back(
        |app| draw(app, "PIDPIPE", &LINE),
        |app| draw(app, "PIDPIPE run", &LINE),
        "add_pipe_run",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["delete_object"]);
    let (mut after, before) = (picture(&case.app), case.before);
    let counter = |seen: &(Vec<String>, Value)| seen.1["pipe_lines"][0]["next_run_ordinal"].clone();
    assert_eq!((counter(&before), counter(&after)), (json!(1), json!(2)));
    after.1["pipe_lines"][0]["next_run_ordinal"] = json!(1);
    assert_eq!(after, before);
}

#[test]
fn a_fitting_put_on_a_run() {
    let case = case(
        line_and_run,
        |app| draw(app, "PIDFITTING gate_valve", &VALVE_AT),
        "insert_fitting",
    );
    assert_eq!(
        inverse_kinds(&case.inverse),
        ["delete_object", "update_pipe_run"]
    );
}

/// A line's property is its run's too: the step says both, and so does
/// its inverse.
#[test]
fn a_line_property_carried_to_its_run() {
    let case = case(
        |app| {
            line_and_run(app);
            run(app, &format!("PIDSETPROP {LINE_TAG} {GRADE} A1A"));
        },
        |app| run(app, &format!("PIDSETPROP {LINE_TAG} {GRADE} B2B")),
        "set_property",
    );
    assert_eq!(
        inverse_kinds(&case.inverse),
        ["set_property", "set_property"]
    );
    for change in &case.inverse {
        assert_eq!(change["value"], "A1A", "{change}");
    }
}

/// A run that lost a segment and kept the rest is said as it stands.
#[test]
fn a_run_changed() {
    let case = case(
        |app| {
            line_and_run(app);
            draw(app, "PIDFITTING gate_valve", &VALVE_AT);
        },
        |app| {
            let segments = handles_where(app, |author| author.topology == ROLE_PIPE_SEGMENT);
            erase(app, &segments[..1]);
        },
        "update_pipe_run",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["update_pipe_run"]);
}

#[test]
fn a_pipe_line_erased() {
    let case = case(
        |app| draw(app, "PIDPIPE", &LINE),
        |app| {
            let line = handles_where(app, |author| author.topology == ROLE_PIPE_LINE);
            erase(app, &line);
        },
        "delete_object",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["create_pipe_line"]);
}

#[test]
fn a_run_erased_with_its_segments() {
    let case = case(
        line_and_run,
        |app| {
            let run = handles_where(app, |author| {
                author.topology == ROLE_PIPE_RUN || author.topology == ROLE_PIPE_SEGMENT
            });
            erase(app, &run);
        },
        "delete_object",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["add_pipe_run"]);
}

#[test]
fn a_tagged_fitting_erased() {
    let case = case(
        |app| {
            line_and_run(app);
            draw(app, "PIDFITTING gate_valve", &VALVE_AT);
        },
        |app| {
            let valve = handles_where(app, |author| author.type_code == TYPE_VALVE);
            erase(app, &valve);
        },
        "delete_object",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["insert_fitting"]);
}

/// An op that took a step back: its inverse puts the step on again, read
/// off the redo stack.
#[test]
fn a_step_taken_back() {
    let case = case(line_and_connector, undo, "delete_connector");
    assert!(!case.on);
    assert_eq!(inverse_kinds(&case.inverse), ["create_connector"]);
}

// ── Properties past what an entity's own record keeps ────────────────────

/// One of them changed: the inverse gives it back the value it had.
#[test]
fn one_of_many_properties_changed() {
    let case = case(
        place_equipment,
        |app| run(app, &format!("PIDSETPROP {EQUIPMENT} {FIELD} 改过的值")),
        "set_property",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["set_property"]);
    assert_eq!(
        (
            case.inverse[0]["field"].as_str(),
            case.inverse[0]["value"].as_str()
        ),
        (Some(FIELD), many_props().get(FIELD).map(String::as_str)),
    );
    assert_eq!(equipment_props(&case.app), (many_props(), false));
}

/// The symbol erased: the inverse places it again with every one of them,
/// and they are kept outside the entity's record, as before.
#[test]
fn a_symbol_with_many_properties_erased() {
    let case = case(
        place_equipment,
        |app| {
            let equipment = handles_where(app, |author| author.tag == EQUIPMENT);
            erase(app, &equipment);
        },
        "delete_object",
    );
    assert_eq!(inverse_kinds(&case.inverse), ["insert_symbol"]);
    assert_eq!(
        texts(&case.inverse[0]["props"]),
        many_props(),
        "what the inverse says of them"
    );
    assert_eq!(equipment_props(&case.app), (many_props(), false));
}

// ── What reading leaves as it was ───────────────────────────────────────

fn payloads(app: &OpenCADStudio) -> Vec<String> {
    app.collab
        .outbox
        .iter()
        .map(|entry| entry.payload.to_string())
        .collect()
}

/// For each op in the queue: whether it has an inverse, and whether one was
/// looked for.
fn read(app: &OpenCADStudio) -> Vec<(bool, bool)> {
    app.collab
        .outbox
        .iter()
        .map(|entry| (entry.inverse.is_some(), entry.inverse_read))
        .collect()
}

/// Reading takes the steps back and puts them on again, and leaves what that
/// moves besides the drawing as it was: both stacks, the selection, the
/// view, whether the drawing is changed. It sends nothing, and what each op
/// says stays as it was. An op whose step was taken back since has no
/// inverse; the op that took it back has one, read off the redo stack.
#[test]
fn reading_leaves_the_stacks_the_selection_and_the_view_as_they_were() {
    let mut app = drawing();
    place_valve(&mut app);
    session_dropped(&mut app);
    set_pressure(&mut app, "1.6MPa");
    draw(&mut app, "PIDPIPE", &LINE);
    undo(&mut app);
    assert_eq!(
        app.collab.outbox.len(),
        3,
        "a change, a line, the line taken back"
    );
    // Since then the valve was selected, the view moved, the drawing saved.
    let tag = symbol_tag(&app);
    let valve = handles_where(&app, |author| author.tag == tag);
    select(&mut app, &valve);
    let i = app.active_tab;
    app.tabs[i].scene.camera.borrow_mut().distance = 1234.5;
    app.tabs[i].scene.camera_generation += 1;
    app.tabs[i].last_synced_camera_gen = app.tabs[i].scene.camera_generation;
    app.tabs[i].dirty = false;
    // The editor read them as it went (`the_next_message_reads_it`): read
    // them again from the start, the valve selected and the view moved.
    for entry in &mut app.collab.outbox {
        entry.inverse = None;
        entry.inverse_read = false;
    }
    let before = surroundings(&app);
    let (said, sent) = (payloads(&app), app.collab.sent.len());

    app.read_queued_inverses();

    assert_eq!(surroundings(&app), before);
    assert_eq!(payloads(&app), said, "what the ops say");
    assert_eq!(app.collab.sent.len(), sent, "an op was sent");
    assert_eq!(read(&app), [(true, true), (false, true), (true, true)]);
}

/// Nothing is read while a command is under way — the step it is making
/// would be taken back under it — and the op is read once it is done.
#[test]
fn nothing_is_read_while_a_command_is_under_way() {
    let mut app = drawing();
    place_valve(&mut app);
    session_dropped(&mut app);
    set_pressure(&mut app, "1.6MPa");
    let started = request(&mut app, json!({"op":"run","cmd":"PIDPIPE"}));
    assert_eq!(started["ok"], true, "{started}");
    let i = app.active_tab;
    assert!(
        app.tabs[i].active_cmd.is_some(),
        "PIDPIPE waits for its points"
    );
    let before = surroundings(&app);

    app.read_queued_inverses();

    assert_eq!(surroundings(&app), before);
    assert_eq!(read(&app), [(false, false)], "read under a running command");
    let cancelled = request(&mut app, json!({"op":"cancel"}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    app.finish_all_pending_history();
    assert!(app.tabs[i].active_cmd.is_none(), "the command is done");
    app.read_queued_inverses();
    assert_eq!(
        read(&app),
        [(true, true)],
        "not read once the command was done"
    );
}

/// Nobody has to ask: the next message the editor takes reads it, with or
/// without a window to tick.
#[test]
fn the_next_message_reads_it() {
    let mut app = drawing();
    place_valve(&mut app);
    session_dropped(&mut app);
    set_pressure(&mut app, "1.6MPa");
    assert_eq!(read(&app), [(false, false)], "read before a message came");
    let _ = app.update(Message::Tick(iced::time::Instant::now()));
    assert_eq!(read(&app), [(true, true)]);
}

/// Online, an op on the wire is left alone, its answer being on its way;
/// one queued behind it is read like any other. What either says stays as
/// it was, and the one behind goes out as it was queued.
#[test]
fn an_op_on_the_wire_is_left_alone_and_what_goes_out_is_as_it_was() {
    let mut app = drawing();
    connected(&mut app);
    place_valve(&mut app);
    set_pressure(&mut app, "1.6MPa");
    let on_the_wire: Vec<bool> = app.collab.outbox.iter().map(|entry| entry.sent).collect();
    assert_eq!(on_the_wire, [true, false], "one on the wire, one behind it");
    let (said, sent) = (payloads(&app), app.collab.sent.len());

    app.read_queued_inverses();

    assert_eq!(read(&app), [(false, false), (true, true)]);
    assert_eq!(payloads(&app), said, "what the ops say");
    assert_eq!(app.collab.sent.len(), sent, "an op was sent");
    let first = app.collab.outbox[0].op_id.clone();
    app.collab.apply(CollabEvent::OpApplied {
        op_id: first,
        revision: 1,
        changes: Vec::new(),
    });
    assert_eq!(app.collab.outbox.len(), 1);
    assert!(app.collab.outbox[0].sent, "the one behind went out");
    assert_eq!(payloads(&app), said[1..], "what went out");
}

// ── In the file ──────────────────────────────────────────────────────────

/// A directory of its own under the temp directory, gone with the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ocs-collab-inverse-{}-{serial}",
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

/// The inverse goes into the file with its op, and comes back with it when
/// the drawing is opened again — as the file says it, not read again.
#[test]
fn the_inverse_goes_into_the_file_with_its_op_and_comes_back() {
    let scratch = Scratch::new();
    let path = scratch.0.join("queued.dwg");
    let mut app = drawing();
    place_valve(&mut app);
    session_dropped(&mut app);
    set_pressure(&mut app, "1.6MPa");
    app.read_queued_inverses();
    let entry = the_queued(&app).clone();
    let inverse = entry.inverse.clone().expect("an inverse was read");
    let saved = app.automation_op(&json!({"op": "save", "path": path}).to_string());
    assert_eq!(saved["ok"], true, "{saved}");

    let bytes = std::fs::read(&path).expect("the saved drawing");
    let back = crate::io::load_bytes("queued.dwg", bytes).expect("the saved drawing reads");
    let queue = crate::pid::collab_state::load_queue(&back);
    assert_eq!(queue.len(), 1, "the queue in the file");
    assert_eq!(queue[0].op_id, entry.op_id);
    assert_eq!(
        queue[0].inverse.as_ref(),
        Some(&inverse),
        "the inverse in the file"
    );

    let mut again = OpenCADStudio::new_for_test();
    again.backup_on_save = false;
    let opened = again.automation_op(&json!({"op": "open", "path": path}).to_string());
    assert_eq!(opened["ok"], true, "{opened}");
    again.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    again.collab.user_id = "u1".to_owned();
    again.collab.drawing_id = DRAWING.to_owned();
    again.collab.document = Some(again.tabs[again.active_tab].id);
    let _ = again.update(Message::CollabEvent(CollabEvent::TagsWanted { ask: 1 }));
    let restored: Vec<(String, bool, Option<Vec<Value>>, bool)> = again
        .collab
        .outbox
        .iter()
        .map(|entry| {
            (
                entry.op_id.clone(),
                entry.restored,
                entry.inverse.clone(),
                entry.inverse_read,
            )
        })
        .collect();
    assert_eq!(restored, [(entry.op_id, true, Some(inverse), true)]);
}

/// An op a version from before saved has no inverse: it reads as one
/// without, and one without is written the way that version wrote it.
#[test]
fn an_op_saved_without_an_inverse_reads_and_writes_as_before() {
    let old = json!({
        "op_id": "op-old",
        "payload": {"kind": "set_property", "object_ref": "P-1001", "field": PRESSURE, "value": "1.6MPa"},
        "base_revision": 3,
        "on": true,
        "label": "改属性",
    });
    let op: QueuedOp = serde_json::from_value(old.clone()).expect("an op as it was written before");
    assert_eq!(op.inverse, None);
    assert_eq!(serde_json::to_value(&op).expect("the op written"), old);
}

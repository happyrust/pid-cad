//! Objects whose properties are too long for the entity's own record: what
//! a step that brings one into the drawing, or changes a property of one,
//! says of it, and what a peer has once it has followed (plan T2.4 ③; tasks
//! `CC-1` and `CC-2` of `docs/plans/2026-11-16-pid-dev-spec/todo.md`).
//!
//! `PidAuthor::store` keeps a `props` text that would take the record past
//! `XDATA_SOFT_LIMIT` in an XRecord of the drawing, and leaves the record a
//! key to it. A step's images are of entities, so the image of such an
//! object only points at its properties; the drawing the step was made in
//! holds them.
//!
//! Symbols first. Three tests are of the reading alone, the fourth of a
//! peer's writing; the next two are two editors in one process with the
//! server's own judgment between them (`ocs_collab::conflict::DrawingModel`,
//! the code the server runs), as in `collab_pipe_ops`: what one places,
//! erases, takes back and makes again goes through the room to the other,
//! and the two drawings — and the room — are then asked for the symbol.
//!
//! Then pipe lines, runs and what is fitted on them, which
//! `super::pipes::statements` says off the drawing: the reading of a line
//! alone, and each of the three erased and put back between two editors.
//!
//! Then a property of one changed once it is there. The value it had is no
//! more in an image than the one it has: the step keeps the drawing's
//! records of its other side beside its images (`DeltaSnapshot::structure`),
//! which the first test of the part makes sure of. Three are of the reading
//! alone, given where the properties of the two sides are. The rest are
//! between two editors again: the change, its being taken back and made
//! again, across the size an entity's record keeps in both directions, past
//! a change of a peer's, and on a pipe line.
//!
//! Last, the symbols such a step is not of. The records it keeps are the
//! drawing's, every symbol's properties among them, and taking it back puts
//! them all back: a symbol a peer has changed or placed since is not to lose
//! its properties to that, nor the room to be told that it has
//! (`OpenCADStudio::keep_props_ahead_of_peers`). Three between two editors:
//! a symbol whose properties a peer's change took out of its record, one a
//! peer placed, and two steps taken back at once.

use crate::app::collab::{broadcast, CollabEvent};
use crate::app::tab_document::{HistorySnapshot, StructureSnapshot};
use crate::app::OpenCADStudio;
use crate::pid::persist::{props_ref_key, PidAuthor, XDATA_SOFT_LIMIT};
use crate::pid::propagate::{join_props, split_props};
use crate::pid::topology::{
    handle_text, ROLE_PIPE_LINE, ROLE_PIPE_RUN, ROLE_PIPE_SEGMENT, TYPE_PIPE_LINE, TYPE_VALVE,
};
use document::entities::{Insert, Line};
use document::objects::XRecordValue;
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use ocs_collab::conflict::{Change as Judged, DrawingModel, Verdict};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const BLOCK: &str = "PID_SYM_equipment_vessel";
const TYPE: &str = "EQUIPMENT";
const TAG: &str = "E-9001";
const AT: [f64; 2] = [100.0, 50.0];

/// Sixty properties, some three and a half kilobytes of them: well past
/// what an entity's own record keeps.
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

/// The author record of a symbol carrying `props` under [`TAG`].
fn record_with(props: &BTreeMap<String, String>, ports: String) -> PidAuthor {
    PidAuthor {
        type_code: TYPE.to_owned(),
        tag: TAG.to_owned(),
        tag_source: "manual".to_owned(),
        tag_status: "ok".to_owned(),
        props: join_props(props, &BTreeSet::new()),
        ports,
        ..PidAuthor::default()
    }
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

/// A drawing holding the symbol with `props`, the symbol's handle, and the
/// image a step has of it.
fn drawing_with(props: &BTreeMap<String, String>) -> (Document, Handle, Arc<EntityType>) {
    let mut doc = Document::new();
    let handle = doc
        .add_entity(EntityType::Insert(Insert::new(
            BLOCK,
            Vector3::new(AT[0], AT[1], 0.0),
        )))
        .expect("entity");
    record_with(props, String::new()).store(&mut doc, handle);
    let image = doc.get_entity_arc(handle).expect("entity");
    assert!(
        !PidAuthor::props_are_inline(&image),
        "the fixture's properties must be too long for the entity's record"
    );
    (doc, handle, image)
}

/// What a step that brought the entity of `image` into `doc` says. There was
/// no such entity on the side the step left, so nothing is read there:
/// `doc`'s own records stand for that side's.
fn said_placing(doc: &Document, handle: Handle, image: Arc<EntityType>) -> Vec<broadcast::Change> {
    broadcast::collect_step_changes(
        doc,
        &[(handle, None, Some(image))],
        &broadcast::Left::Records(doc),
    )
}

/// `said` is one placement of the symbol under [`TAG`] carrying `props` —
/// as the change is kept, and as it goes on the wire.
#[track_caller]
fn assert_placed_with(said: &[broadcast::Change], props: &BTreeMap<String, String>) {
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(
        (said[0].kind.as_str(), said[0].object_ref.as_str()),
        (broadcast::KIND_INSERT_SYMBOL, TAG)
    );
    let placed = said[0].placed.as_ref().expect("what was placed");
    assert_eq!(
        &split_props(&placed.props).0,
        props,
        "the properties the change carries"
    );
    assert_eq!(
        &texts(&said[0].to_json()["props"]),
        props,
        "the properties on the wire"
    );
}

/// A step that placed such a symbol says it with every property it has:
/// they are read off the drawing the step left, which holds them.
#[test]
fn collab_large_props_a_symbol_placed_with_them_is_said_with_every_one() {
    let props = many_props();
    let (doc, handle, image) = drawing_with(&props);
    let said = said_placing(&doc, handle, image);
    assert_placed_with(&said, &props);
}

/// An erase taken back puts the symbol in the drawing again, which to the
/// room is a placement: said the same way, off the drawing without the
/// erase.
#[test]
fn collab_large_props_an_erase_taken_back_says_the_symbol_with_every_one() {
    let props = many_props();
    let (doc, handle, image) = drawing_with(&props);
    let said = broadcast::collect_undone_step_changes(
        &doc,
        &[(handle, Some(image), None)],
        &broadcast::Left::Records(&doc),
    );
    assert_placed_with(&said, &props);
}

/// Where the drawing does not hold what the image points at, the symbol is
/// still said — as starting with no properties, which is all that is known
/// of them.
#[test]
fn collab_large_props_a_symbol_whose_properties_the_drawing_lacks_is_still_said() {
    let (_, handle, image) = drawing_with(&many_props());
    // A drawing that holds the entity, record and all, but not the XRecord
    // the record points at.
    let mut bare = Document::new();
    let copied = bare.add_entity((*image).clone()).expect("entity");
    let copy = bare.get_entity_arc(copied).expect("entity");
    assert!(!PidAuthor::props_are_inline(&copy));
    // And one that does not hold the entity at all.
    let empty = Document::new();
    for said in [
        said_placing(&bare, copied, copy),
        said_placing(&empty, handle, image),
    ] {
        assert_placed_with(&said, &BTreeMap::new());
    }
}

/// An editor on a new drawing, in a session.
fn editor(session: &str) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.connected = true;
    app.collab.session_id = Some(session.to_owned());
    app.collab.user_id = session.to_owned();
    app
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert("request_id".into(), json!(format!("collab-large-{serial}")));
    }
    let (response, task) = app.control_request(req);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    response
}

/// The symbol comes into the drawing in front with `props` in one step,
/// the way a paste of one does: the entity, its record and the XRecord the
/// record points at are all the step's. Its handle.
fn place(app: &mut OpenCADStudio, props: &BTreeMap<String, String>) -> Handle {
    place_as(app, TAG, AT, props)
}

/// [`place`], of a symbol under `tag` standing at `at`.
fn place_as(
    app: &mut OpenCADStudio,
    tag: &str,
    at: [f64; 2],
    props: &BTreeMap<String, String>,
) -> Handle {
    let i = app.active_tab;
    if !app.tabs[i].scene.document.block_records.contains(BLOCK) {
        app.pid_symbols.instantiate(&mut app.tabs[i].scene);
    }
    let ports = app.pid_symbols.symbol_ports(BLOCK).join(",");
    app.push_undo_snapshot(i, "PASTECLIP");
    let handle = app.tabs[i].scene.add_entity(EntityType::Insert(Insert::new(
        BLOCK,
        Vector3::new(at[0], at[1], 0.0),
    )));
    assert!(!handle.is_null(), "the symbol was not added");
    let document = &mut app.tabs[i].scene.document;
    PidAuthor {
        tag: tag.to_owned(),
        ..record_with(props, ports)
    }
    .store(document, handle);
    crate::app::commands::pidpalette::sync_pid_tag_attribute(document, handle);
    app.tabs[i].dirty = true;
    app.finish_all_pending_history();
    handle
}

fn erase(app: &mut OpenCADStudio, handles: &[Handle]) {
    let list: Vec<String> = handles.iter().map(|handle| handle_text(*handle)).collect();
    let selected = request(app, json!({"op":"select","handles":list}));
    assert_eq!(selected["ok"], true, "{selected}");
    let erased = request(app, json!({"op":"run","cmd":"ERASE"}));
    assert_eq!(erased["ok"], true, "{erased}");
    app.finish_all_pending_history();
    let document = &app.tabs[app.active_tab].scene.document;
    for handle in handles {
        assert!(
            document.get_entity(*handle).is_none(),
            "{handle:?} was not erased"
        );
    }
}

fn undo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"undo"}"#);
}

fn redo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"redo"}"#);
}

/// The room: the server's picture of the drawing and its revision.
#[derive(Default)]
struct Room {
    model: DrawingModel,
    revision: i64,
}

impl Room {
    /// What the server makes of one op: the changes it tells the room, every
    /// one of them having taken effect, at the revision it moved to.
    fn take(&mut self, session: &str, base: i64, payload: &Value) -> (i64, Vec<Value>) {
        let changes = Judged::list(payload);
        assert!(
            !changes.is_empty() && changes.iter().all(Judged::is_understood),
            "the server has no rule for {payload}"
        );
        let revision = self.revision + 1;
        let mut said = Vec::new();
        for change in changes {
            let change = self.model.completed(&change);
            match self.model.judge(&change, base, session) {
                Verdict::Apply { object_ref, .. } => {
                    said.push((change, object_ref));
                }
                other => panic!("the server refused {}: {other:?}", change.raw),
            }
        }
        let mut broadcast = Vec::new();
        for (change, object_ref) in said {
            self.model
                .commit(&change, &object_ref, revision, session, session);
            broadcast.push(change.broadcast_form(&object_ref));
        }
        self.revision = revision;
        (revision, broadcast)
    }
}

/// Everything `from` has said since `seen` goes through the room: back to
/// `from` as its own op, and to `to` as a peer's.
fn relay(room: &mut Room, from: &mut OpenCADStudio, to: &mut OpenCADStudio, seen: &mut usize) {
    let session = from.collab.session_id.clone().expect("a session");
    let payloads: Vec<Value> = from.collab.sent[*seen..].to_vec();
    *seen = from.collab.sent.len();
    let own: Vec<String> = from.collab.own_ops.iter().cloned().collect();
    let first = own.len() - payloads.len();
    for (index, payload) in payloads.iter().enumerate() {
        let base = from.collab.base_revision.min(room.revision);
        let (revision, changes) = room.take(&session, base, payload);
        from.collab.apply(CollabEvent::OpApplied {
            op_id: own[first + index].clone(),
            revision,
            changes: changes.clone(),
            user_id: None,
            display_name: None,
        });
        from.apply_remote_changes();
        to.collab.apply(CollabEvent::OpApplied {
            op_id: format!("op-of-{session}-{revision}"),
            revision,
            changes,
            user_id: None,
            display_name: None,
        });
        to.apply_remote_changes();
    }
    assert_eq!(to.collab.diverged, None, "the peer could not follow");
    assert_eq!(to.collab.last_apply_error, None);
    assert_eq!(from.collab.diverged, None);
}

/// What the drawing in front holds of one symbol.
#[derive(Debug, PartialEq)]
struct Held {
    /// Its block and where it stands.
    shape: String,
    /// Its record, the properties as `join_props` writes them.
    record: PidAuthor,
    /// Whether the properties are in the entity's own record.
    inline: bool,
}

/// Every symbol of the drawing in front that carries [`TAG`].
fn held(app: &OpenCADStudio) -> Vec<Held> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .filter_map(|entity| {
            let EntityType::Insert(insert) = entity else {
                return None;
            };
            let mut record = PidAuthor::load(document, insert.common.handle)?;
            if record.tag != TAG {
                return None;
            }
            let (values, marks) = split_props(&record.props);
            record.props = join_props(&values, &marks);
            Some(Held {
                shape: format!(
                    "{} {:.4},{:.4}",
                    insert.block_name, insert.insert_point.x, insert.insert_point.y
                ),
                record,
                inline: PidAuthor::props_are_inline(entity),
            })
        })
        .collect()
}

/// Both drawings hold the symbol once and the same in everything, with
/// every one of `props`, kept outside the entity's record on both sides.
#[track_caller]
fn assert_whole(
    a: &OpenCADStudio,
    b: &OpenCADStudio,
    props: &BTreeMap<String, String>,
    when: &str,
) {
    assert_same(a, b, props, false, when);
}

/// Both drawings hold the symbol once and the same in everything, with
/// exactly `props`, kept in the entity's own record (`inline`) or outside
/// it — the same on both sides.
#[track_caller]
fn assert_same(
    a: &OpenCADStudio,
    b: &OpenCADStudio,
    props: &BTreeMap<String, String>,
    inline: bool,
    when: &str,
) {
    let (here, there) = (held(a), held(b));
    assert_eq!(here.len(), 1, "the sender's drawing {when}: {here:?}");
    let theirs = there.first().map(|held| held.record.props.clone());
    assert_eq!(
        &split_props(&theirs.unwrap_or_default()).0,
        props,
        "the peer's properties {when}"
    );
    assert_eq!(there, here, "the peer's drawing {when}");
    assert_eq!(
        there[0].inline, inline,
        "whether the peer keeps them in the entity's own record {when}"
    );
}

/// A peer's placement that carries such properties is written with every
/// one of them, and kept the way a local one is: outside the entity's
/// record.
#[test]
fn collab_large_props_a_peer_s_symbol_is_written_with_every_one_and_kept_the_same_way() {
    let props = many_props();
    let mut b = editor("s-b");
    let placement = broadcast::Change {
        kind: broadcast::KIND_INSERT_SYMBOL.to_owned(),
        object_ref: TAG.to_owned(),
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: Some(broadcast::Placed {
            block: BLOCK.to_owned(),
            type_code: TYPE.to_owned(),
            position: [AT[0], AT[1], 0.0],
            rotation: 0.0,
            scale: 1.0,
            ports: String::new(),
            props: join_props(&props, &BTreeSet::new()),
            tag_source: "manual".to_owned(),
        }),
        pipe: None,
        tag_source: None,
        overrides: None,
    };
    b.collab.apply(CollabEvent::OpApplied {
        op_id: "op-of-s-a-1".to_owned(),
        revision: 1,
        changes: vec![placement.to_json()],
        user_id: None,
        display_name: None,
    });
    b.apply_remote_changes();
    assert_eq!(b.collab.last_apply_error, None);
    assert_eq!(b.collab.diverged, None);

    let there = held(&b);
    assert_eq!(there.len(), 1, "{there:?}");
    assert_eq!(split_props(&there[0].record.props).0, props);
    assert!(
        !there[0].inline,
        "properties this long are kept outside the entity's record"
    );
}

/// One places such a symbol; the step says every property, the other's
/// drawing has the same symbol with the same record, and the room knows
/// each field. None of it is a step of the second person's, and none of it
/// goes back to the room.
#[test]
fn collab_large_props_a_symbol_placed_with_them_reaches_the_peer_and_the_room_whole() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    let steps = b.tabs[b.active_tab].history.undo_stack.len();
    let props = many_props();

    place(&mut a, &props);
    assert_eq!(a.collab.sent.len(), 1, "the placement was not said");
    let said = a.collab.sent[0].clone();
    assert_eq!(said["kind"], "insert_symbol", "{said}");
    assert_eq!(said["tag"], TAG, "{said}");
    assert_eq!(texts(&said["props"]), props, "what the step said of them");

    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &props, "after the placement");
    let known = room.model.object(TAG).expect("the room knows the symbol");
    for (field, value) in &props {
        assert_eq!(
            known.fields.get(field).map(|state| state.value.as_str()),
            Some(value.as_str()),
            "the room's {field}"
        );
    }
    assert_eq!(b.tabs[b.active_tab].history.undo_stack.len(), steps);
    assert!(b.collab.sent.is_empty(), "{:?}", b.collab.sent);
}

/// Erased and the erase taken back, then the placement itself taken back
/// and made again: each time the symbol is back at the peer it is whole.
#[test]
fn collab_large_props_taken_out_and_put_back_the_symbol_is_whole_at_the_peer_each_time() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    let props = many_props();
    let handle = place(&mut a, &props);
    relay(&mut room, &mut a, &mut b, &mut seen);

    erase(&mut a, &[handle]);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert!(held(&b).is_empty(), "the peer still has it: {:?}", held(&b));
    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &props, "after the erase was taken back");

    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert!(held(&a).is_empty(), "the placement was not taken back");
    assert!(held(&b).is_empty(), "the peer still has it: {:?}", held(&b));
    redo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &props, "after the placement was made again");
}

// ── pipe lines, runs and what is fitted on them ─────────────────────────

const LINE_TAG: &str = "L-9001";
/// Where the line the editors draw runs, and where the valve stands on it.
const LINE: &[[f64; 2]] = &[[0.0, 0.0], [200.0, 0.0]];
const VALVE_AT: &[[f64; 2]] = &[[100.0, 0.0]];

/// A drawing holding one stretch of a pipe line with `props`, the stretch's
/// handle, and the image a step has of it.
fn drawing_with_a_line(props: &BTreeMap<String, String>) -> (Document, Handle, Arc<EntityType>) {
    let mut doc = Document::new();
    let handle = doc
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(200.0, 0.0, 0.0),
        )))
        .expect("entity");
    PidAuthor {
        type_code: TYPE_PIPE_LINE.to_owned(),
        tag: LINE_TAG.to_owned(),
        tag_source: "manual".to_owned(),
        tag_status: "ok".to_owned(),
        topology: ROLE_PIPE_LINE.to_owned(),
        props: join_props(props, &BTreeSet::new()),
        ..PidAuthor::default()
    }
    .store(&mut doc, handle);
    let image = doc.get_entity_arc(handle).expect("entity");
    assert!(
        !PidAuthor::props_are_inline(&image),
        "the fixture's properties must be too long for the entity's record"
    );
    (doc, handle, image)
}

/// `said` is the line under [`LINE_TAG`] coming into being with `props`.
#[track_caller]
fn assert_line_said_with(said: &[broadcast::Change], props: &BTreeMap<String, String>) {
    assert_eq!(said.len(), 1, "{said:?}");
    let wire = said[0].to_json();
    assert_eq!(
        (&wire["kind"], &wire["tag"]),
        (&json!("create_pipe_line"), &json!(LINE_TAG)),
        "{wire}"
    );
    assert_eq!(&texts(&wire["props"]), props, "the properties on the wire");
}

/// A step that drew a stretch of such a line says the line with every
/// property it has.
#[test]
fn collab_large_props_a_line_drawn_with_them_is_said_with_every_one() {
    let props = many_props();
    let (doc, handle, image) = drawing_with_a_line(&props);
    let said = said_placing(&doc, handle, image);
    assert_line_said_with(&said, &props);
}

/// Where the drawing does not hold what the image points at, the line is
/// still said, as starting with no properties.
#[test]
fn collab_large_props_a_line_whose_properties_the_drawing_lacks_is_still_said() {
    let (_, handle, image) = drawing_with_a_line(&many_props());
    // A drawing that holds the stretch, record and all, but not the XRecord
    // the record points at.
    let mut bare = Document::new();
    let copied = bare.add_entity((*image).clone()).expect("entity");
    let copy = bare.get_entity_arc(copied).expect("entity");
    assert!(!PidAuthor::props_are_inline(&copy));
    // And one that does not hold the stretch at all.
    let empty = Document::new();
    for said in [
        said_placing(&bare, copied, copy),
        said_placing(&empty, handle, image),
    ] {
        assert_line_said_with(&said, &BTreeMap::new());
    }
}

fn is_line(author: &PidAuthor) -> bool {
    author.topology == ROLE_PIPE_LINE
}

fn is_run(author: &PidAuthor) -> bool {
    author.topology == ROLE_PIPE_RUN
}

fn is_valve(author: &PidAuthor) -> bool {
    author.type_code == TYPE_VALVE
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

/// The entities of the drawing in front whose record `keep`s.
fn handles_where(app: &OpenCADStudio, keep: fn(&PidAuthor) -> bool) -> Vec<Handle> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .filter(|handle| PidAuthor::load(document, *handle).is_some_and(|author| keep(&author)))
        .collect()
}

/// The properties of the one entity of the drawing in front whose record
/// `keep`s, and whether they are in the entity's own record.
#[track_caller]
fn props_where(
    app: &OpenCADStudio,
    keep: fn(&PidAuthor) -> bool,
) -> (BTreeMap<String, String>, bool) {
    let handles = handles_where(app, keep);
    assert_eq!(handles.len(), 1, "one such entity: {handles:?}");
    let document = &app.tabs[app.active_tab].scene.document;
    let record = PidAuthor::load(document, handles[0]).expect("a record");
    let entity = document.get_entity(handles[0]).expect("entity");
    (
        split_props(&record.props).0,
        PidAuthor::props_are_inline(entity),
    )
}

/// Every change `app` has said since the first `seen` of its ops, one by
/// one.
fn said_since(app: &OpenCADStudio, seen: usize) -> Vec<Value> {
    app.collab.sent[seen..]
        .iter()
        .flat_map(Judged::list)
        .map(|change| change.raw)
        .collect()
}

/// The one change of `kind` among `said`.
#[track_caller]
fn the_one<'a>(said: &'a [Value], kind: &str) -> &'a Value {
    let of_kind: Vec<&Value> = said
        .iter()
        .filter(|change| change["kind"] == kind)
        .collect();
    assert_eq!(of_kind.len(), 1, "one {kind} among {said:?}");
    of_kind[0]
}

/// What [`erased_and_put_back`] leaves.
struct PutBack {
    /// The one who drew, erased and took the erase back.
    a: OpenCADStudio,
    /// The peer.
    b: OpenCADStudio,
    room: Room,
    /// How many of the first one's ops the room has had: all of them.
    seen: usize,
    /// What taking the erase back said, change by change.
    said: Vec<Value>,
    /// The properties the thing has in the first drawing.
    props: BTreeMap<String, String>,
}

/// One draws what `cmds` draw and the peer follows. The thing whose record
/// `is` it then has [`many_props`] on top of its own in the first drawing —
/// by no step: the drawing has them, as one read from a file does — and is
/// erased there with whatever `goes` with it, which the peer follows; then
/// the erase is taken back, and the peer follows that.
fn erased_and_put_back(
    cmds: &[(&str, &[[f64; 2]])],
    is: fn(&PidAuthor) -> bool,
    goes: fn(&PidAuthor) -> bool,
) -> PutBack {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    for (cmd, points) in cmds {
        draw(&mut a, cmd, points);
    }
    relay(&mut room, &mut a, &mut b, &mut seen);

    let grown = handles_where(&a, is);
    assert_eq!(grown.len(), 1, "one such thing: {grown:?}");
    let props = {
        let document = &mut a.tabs[a.active_tab].scene.document;
        let mut record = PidAuthor::load(document, grown[0]).expect("a record");
        let (mut values, marks) = split_props(&record.props);
        values.extend(many_props());
        record.props = join_props(&values, &marks);
        record.store(document, grown[0]);
        values
    };
    assert_eq!(props_where(&a, is), (props.clone(), false), "the fixture");

    let gone = handles_where(&a, goes);
    erase(&mut a, &gone);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert!(handles_where(&b, is).is_empty(), "the peer still has it");

    undo(&mut a);
    assert_eq!(
        props_where(&a, is),
        (props.clone(), false),
        "the first drawing has it back as it was"
    );
    let said = said_since(&a, seen);
    relay(&mut room, &mut a, &mut b, &mut seen);
    PutBack {
        a,
        b,
        room,
        seen,
        said,
        props,
    }
}

/// A line erased and the erase taken back: the line is said with every
/// property, the peer's line has them — outside the entity's record, as
/// here — and the room knows each field.
#[test]
fn collab_large_props_a_line_put_back_is_whole_at_the_peer_and_in_the_room() {
    let back = erased_and_put_back(&[("PIDPIPE", LINE)], is_line, is_line);
    let line = the_one(&back.said, "create_pipe_line");
    assert_eq!(texts(&line["props"]), back.props, "what was said of them");
    assert_eq!(props_where(&back.b, is_line), (back.props.clone(), false));
    let tag = line["tag"].as_str().expect("the line's tag");
    let known = back.room.model.object(tag).expect("the room knows it");
    for (field, value) in &back.props {
        assert_eq!(
            known.fields.get(field).map(|state| state.value.as_str()),
            Some(value.as_str()),
            "the room's {field}"
        );
    }
}

/// A run erased with its segments and the erase taken back: the account of
/// the run is said with every property, and the peer's run has them.
#[test]
fn collab_large_props_a_run_put_back_is_whole_at_the_peer() {
    let back = erased_and_put_back(
        &[("PIDPIPE", LINE), ("PIDPIPE run", LINE)],
        is_run,
        |author| author.topology == ROLE_PIPE_RUN || author.topology == ROLE_PIPE_SEGMENT,
    );
    let run = the_one(&back.said, "add_pipe_run");
    assert_eq!(
        texts(&run["run"]["props"]),
        back.props,
        "what was said of them"
    );
    assert_eq!(props_where(&back.b, is_run), (back.props.clone(), false));
}

/// A valve erased from its run and the erase taken back: the run's account
/// says the valve with every property, and the peer's valve has them.
#[test]
fn collab_large_props_a_valve_put_back_on_its_run_is_whole_at_the_peer() {
    let back = erased_and_put_back(
        &[
            ("PIDPIPE", LINE),
            ("PIDPIPE run", LINE),
            ("PIDFITTING gate_valve", VALVE_AT),
        ],
        is_valve,
        is_valve,
    );
    let fitted = the_one(&back.said, "insert_fitting");
    let valves = fitted["run"]["fittings"].as_array().expect("the fittings");
    assert_eq!(valves.len(), 1, "{fitted}");
    assert_eq!(
        texts(&valves[0]["props"]),
        back.props,
        "what was said of them"
    );
    assert_eq!(props_where(&back.b, is_valve), (back.props.clone(), false));
}

// ── a property of one changed once it is there ──────────────────────────

/// The property the tests change, and what to.
const FIELD: &str = "字段01";
const CHANGED: &str = "改过的值";
/// The property whose value takes a record past the size an entity keeps,
/// or brings it back within.
const NOTE: &str = "备注";

/// Few enough to stay in the entity's own record.
fn few_props() -> BTreeMap<String, String> {
    many_props().into_iter().take(5).collect()
}

/// A value too long, by itself, for an entity's own record.
fn long_value() -> String {
    "y".repeat(XDATA_SOFT_LIMIT)
}

/// `props` with `field` at `value`.
fn with(props: &BTreeMap<String, String>, field: &str, value: &str) -> BTreeMap<String, String> {
    let mut props = props.clone();
    props.insert(field.to_owned(), value.to_owned());
    props
}

/// `PIDSETPROP`: `field` of the object under `tag` becomes `value`, in one
/// step.
#[track_caller]
fn set_prop(app: &mut OpenCADStudio, tag: &str, field: &str, value: &str) {
    let cmd = format!("PIDSETPROP {tag} {field} {value}");
    let set = request(app, json!({"op":"run","cmd":cmd}));
    assert_eq!(set["ok"], true, "{set}");
    app.finish_all_pending_history();
    let document = &app.tabs[app.active_tab].scene.document;
    let now: Vec<Option<String>> = document
        .entities()
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .filter(|record| record.tag == tag)
        .map(|record| split_props(&record.props).0.get(field).cloned())
        .collect();
    assert!(
        !now.is_empty() && now.iter().all(|now| now.as_deref() == Some(value)),
        "{field} of {tag} was not set: {now:?}"
    );
}

/// The properties `records` holds for `handle` in the XRecord
/// `PidAuthor::store` keeps them in. `records` is a drawing, or what a step
/// keeps of one beside its images (`Document::snapshot_structure`).
fn kept_in(records: &Document, handle: Handle) -> Option<BTreeMap<String, String>> {
    let owner = records.header.model_space_block_handle;
    let record = records.xrecord(owner, &props_ref_key(handle))?;
    let mut text = Vec::new();
    for entry in &record.entries {
        if let XRecordValue::Chunk(chunk) = &entry.value {
            text.extend_from_slice(chunk);
        }
    }
    Some(split_props(&String::from_utf8(text).ok()?).0)
}

/// The records the step on top of `stack` keeps of the drawing's other
/// side.
#[track_caller]
fn records_of(stack: &[HistorySnapshot]) -> &Document {
    let Some(HistorySnapshot::Delta(step)) = stack.last() else {
        panic!("no step on top");
    };
    let Some(StructureSnapshot::Full(records)) = &step.structure else {
        panic!("the step keeps no records of the other side");
    };
    records
}

/// What the rest stands on: the step that changes a property of such a
/// symbol keeps the drawing's records as they were before it — the XRecord
/// with the properties among them — and, once it is taken back, as they
/// were with it.
#[test]
fn collab_large_props_a_step_that_changes_one_keeps_the_records_of_its_other_side() {
    let mut a = editor("s-a");
    let props = many_props();
    let changed = with(&props, FIELD, CHANGED);
    let handle = place(&mut a, &props);
    let i = a.active_tab;

    set_prop(&mut a, TAG, FIELD, CHANGED);
    assert_eq!(
        kept_in(&a.tabs[i].scene.document, handle).as_ref(),
        Some(&changed)
    );
    assert_eq!(
        kept_in(records_of(&a.tabs[i].history.undo_stack), handle).as_ref(),
        Some(&props),
        "made: the step keeps the records from before it"
    );

    undo(&mut a);
    assert_eq!(
        kept_in(&a.tabs[i].scene.document, handle).as_ref(),
        Some(&props)
    );
    assert_eq!(
        kept_in(records_of(&a.tabs[i].history.redo_stack), handle).as_ref(),
        Some(&changed),
        "taken back: the step keeps the records with it"
    );

    redo(&mut a);
    assert_eq!(
        kept_in(&a.tabs[i].scene.document, handle).as_ref(),
        Some(&changed)
    );
    assert_eq!(
        kept_in(records_of(&a.tabs[i].history.undo_stack), handle).as_ref(),
        Some(&props),
        "made again: the step keeps the records from before it"
    );
}

/// The symbol in two drawings — as [`drawing_with`] makes it, and with
/// [`FIELD`] changed — and what a step from the one to the other has of it.
struct Sides {
    was: Document,
    now: Document,
    handle: Handle,
    images: Box<broadcast::EntityImages>,
}

/// [`Sides`] of the symbol with `props`, under `tag_now` once changed.
fn two_sides(props: &BTreeMap<String, String>, tag_now: &str) -> Sides {
    let (was, handle, before) = drawing_with(props);
    let mut now = was.clone();
    PidAuthor {
        tag: tag_now.to_owned(),
        ..record_with(&with(props, FIELD, CHANGED), String::new())
    }
    .store(&mut now, handle);
    let after = now.get_entity_arc(handle).expect("entity");
    assert!(
        !PidAuthor::props_are_inline(&after),
        "the properties are too long on both sides"
    );
    Sides {
        was,
        now,
        handle,
        images: Box::new([(handle, Some(before), Some(after))]),
    }
}

/// `said` is one thing: `field` of the object under `tag` is `value`, and
/// was `old`.
#[track_caller]
fn assert_one_property(
    said: &[broadcast::Change],
    tag: &str,
    field: &str,
    value: &str,
    old: Option<&str>,
) {
    assert_eq!(said.len(), 1, "{said:?}");
    let said = &said[0];
    assert_eq!(
        (
            said.kind.as_str(),
            said.object_ref.as_str(),
            said.field.as_str(),
            said.value.as_str(),
            said.old.as_deref()
        ),
        (broadcast::KIND_SET_PROPERTY, tag, field, value, old),
        "{said:?}"
    );
}

/// The reading alone. A step that changed one property of such a symbol
/// says that property, with what it was: what it is, read off the drawing;
/// what it was, off the records of the side the step left. The step taken
/// back says it the other way round.
#[test]
fn collab_large_props_one_changed_is_read_off_the_records_of_both_sides() {
    let props = many_props();
    let Sides {
        was, now, images, ..
    } = two_sides(&props, TAG);

    let made = broadcast::collect_step_changes(&now, &images, &broadcast::Left::Records(&was));
    assert_one_property(&made, TAG, FIELD, CHANGED, Some(&props[FIELD]));

    let taken_back =
        broadcast::collect_undone_step_changes(&was, &images, &broadcast::Left::Records(&now));
    assert_one_property(&taken_back, TAG, FIELD, &props[FIELD], Some(CHANGED));
}

/// The side left can also be given as what was read off the drawing while
/// it was on that side — the properties of an entity, by its handle — which
/// is all an undo past a peer's change has of it.
#[test]
fn collab_large_props_one_changed_is_read_off_what_was_read_of_the_side_left() {
    let props = many_props();
    let Sides {
        now,
        handle,
        images,
        ..
    } = two_sides(&props, TAG);

    let read = [(handle, join_props(&props, &BTreeSet::new()))];
    let said = broadcast::collect_step_changes(&now, &images, &broadcast::Left::Read(&read));
    assert_one_property(&said, TAG, FIELD, CHANGED, Some(&props[FIELD]));

    // What was read of another entity says nothing of this one.
    let of_another = [(Handle::new(handle.value() + 1), read[0].1.clone())];
    let said = broadcast::collect_step_changes(&now, &images, &broadcast::Left::Read(&of_another));
    assert!(said.is_empty(), "{said:?}");
}

/// Where the properties of a side are not known, nothing is said of the
/// symbol's properties: not the one as changed, and not all of them as gone
/// or as new. What the images themselves hold is still said — a retag —
/// and, the properties known, the change follows it under the new tag.
#[test]
fn collab_large_props_a_side_whose_properties_are_not_known_says_none_changed() {
    const RENAMED: &str = "E-9002";
    let props = many_props();
    let Sides {
        was, now, images, ..
    } = two_sides(&props, TAG);
    let none = Document::new();
    for (side, said) in [
        (
            "the side left, in records that hold none",
            broadcast::collect_step_changes(&now, &images, &broadcast::Left::Records(&none)),
        ),
        (
            "the side left, nothing read of it",
            broadcast::collect_step_changes(&now, &images, &broadcast::Left::Read(&[])),
        ),
        (
            "the side the drawing is on",
            broadcast::collect_step_changes(&none, &images, &broadcast::Left::Records(&was)),
        ),
    ] {
        assert!(said.is_empty(), "{side}: {said:?}");
    }

    let Sides {
        was, now, images, ..
    } = two_sides(&props, RENAMED);
    let is_the_retag = |said: &broadcast::Change| {
        (
            said.kind.as_str(),
            said.object_ref.as_str(),
            said.value.as_str(),
        ) == (broadcast::KIND_SET_TAG, TAG, RENAMED)
    };
    let said = broadcast::collect_step_changes(&now, &images, &broadcast::Left::Records(&none));
    assert!(said.len() == 1 && is_the_retag(&said[0]), "{said:?}");
    let said = broadcast::collect_step_changes(&now, &images, &broadcast::Left::Records(&was));
    assert!(said.len() == 2 && is_the_retag(&said[0]), "{said:?}");
    assert_one_property(&said[1..], RENAMED, FIELD, CHANGED, Some(&props[FIELD]));
}

/// Two editors with the room between them.
struct Pair {
    a: OpenCADStudio,
    b: OpenCADStudio,
    room: Room,
    /// How many of the first one's ops the room has had.
    seen: usize,
}

/// The first places the symbol with `props` and the second follows.
fn placed_at_both(props: &BTreeMap<String, String>) -> Pair {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    place(&mut a, props);
    relay(&mut room, &mut a, &mut b, &mut seen);
    Pair { a, b, room, seen }
}

/// What `app` has said since the first `seen` of its ops is one thing:
/// `field` of the object under `tag` is `value`, and was `old` — `None`
/// where it had none.
#[track_caller]
fn assert_said_one(
    app: &OpenCADStudio,
    seen: usize,
    tag: &str,
    field: &str,
    value: &str,
    old: Option<&str>,
) {
    let said = said_since(app, seen);
    assert_eq!(said.len(), 1, "one change, of the one property: {said:?}");
    let said = &said[0];
    assert_eq!(
        (
            &said["kind"],
            &said["object_ref"],
            &said["field"],
            &said["value"]
        ),
        (
            &json!("set_property"),
            &json!(tag),
            &json!(field),
            &json!(value)
        ),
        "{said}"
    );
    assert_eq!(said["old_value"], json!(old), "what it was: {said}");
}

/// One changes a property of such a symbol: the step says that property
/// alone, with what it was; the other's symbol has it, and every other
/// property as before; the room knows the new value. None of it is a step
/// of the second person's, and none of it goes back to the room.
#[test]
fn collab_large_props_one_changed_is_said_alone_with_what_it_was_and_the_peer_follows() {
    let props = many_props();
    let changed = with(&props, FIELD, CHANGED);
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&props);
    let steps = b.tabs[b.active_tab].history.undo_stack.len();

    set_prop(&mut a, TAG, FIELD, CHANGED);
    assert_said_one(&a, seen, TAG, FIELD, CHANGED, Some(&props[FIELD]));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &changed, "after the change");

    let known = room.model.object(TAG).expect("the room knows the symbol");
    for (field, value) in &changed {
        assert_eq!(
            known.fields.get(field).map(|state| state.value.as_str()),
            Some(value.as_str()),
            "the room's {field}"
        );
    }
    assert_eq!(b.tabs[b.active_tab].history.undo_stack.len(), steps);
    assert!(b.collab.sent.is_empty(), "{:?}", b.collab.sent);
}

/// The change taken back and made again: each says the one property with
/// what it was, and the peer's symbol goes back and comes again.
#[test]
fn collab_large_props_one_changed_taken_back_and_made_again_is_followed() {
    let props = many_props();
    let changed = with(&props, FIELD, CHANGED);
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&props);
    set_prop(&mut a, TAG, FIELD, CHANGED);
    relay(&mut room, &mut a, &mut b, &mut seen);

    undo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, &props[FIELD], Some(CHANGED));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &props, "after the change was taken back");

    redo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, CHANGED, Some(&props[FIELD]));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &changed, "after the change was made again");
}

/// A change that takes a symbol's properties past what its record keeps:
/// it, its being taken back — which brings them within the record again —
/// and its being made again each say the one property, and the peer keeps
/// them where the first does every time.
#[test]
fn collab_large_props_a_change_that_takes_them_out_of_the_record_says_the_one_property() {
    let (few, long) = (few_props(), long_value());
    let grown = with(&few, NOTE, &long);
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&few);
    assert_same(&a, &b, &few, true, "to begin with");

    set_prop(&mut a, TAG, NOTE, &long);
    assert_said_one(&a, seen, TAG, NOTE, &long, None);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &grown, false, "after the change");

    // A property that is gone travels as an empty value.
    undo(&mut a);
    assert_said_one(&a, seen, TAG, NOTE, "", Some(&long));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &few, true, "after the change was taken back");

    redo(&mut a);
    assert_said_one(&a, seen, TAG, NOTE, &long, None);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &grown, false, "after the change was made again");
}

/// And one that brings them back within it.
#[test]
fn collab_large_props_a_change_that_brings_them_into_the_record_says_the_one_property() {
    let (few, long) = (few_props(), long_value());
    let grown = with(&few, NOTE, &long);
    let short = with(&few, NOTE, "短");
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&grown);
    assert_same(&a, &b, &grown, false, "to begin with");

    set_prop(&mut a, TAG, NOTE, "短");
    assert_said_one(&a, seen, TAG, NOTE, "短", Some(&long));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &short, true, "after the change");

    undo(&mut a);
    assert_said_one(&a, seen, TAG, NOTE, &long, Some("短"));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &grown, false, "after the change was taken back");

    redo(&mut a);
    assert_said_one(&a, seen, TAG, NOTE, "短", Some(&long));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, &short, true, "after the change was made again");
}

/// A peer has changed another property of the symbol since. Taking the
/// first change back is then worked out from the drawing before and after
/// (`replay_remote_after`), not from the step: it still says the one
/// property with what it was, and leaves the peer's as the peer set it.
/// So does making it again.
#[test]
fn collab_large_props_one_changed_is_taken_back_and_made_again_past_a_peer_s_change() {
    const THEIR_FIELD: &str = "字段02";
    const THEIRS: &str = "对端改的";
    let props = many_props();
    let theirs = with(&props, THEIR_FIELD, THEIRS);
    let both = with(&theirs, FIELD, CHANGED);
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&props);
    let mut seen_of_b = 0;
    set_prop(&mut a, TAG, FIELD, CHANGED);
    relay(&mut room, &mut a, &mut b, &mut seen);
    set_prop(&mut b, TAG, THEIR_FIELD, THEIRS);
    relay(&mut room, &mut b, &mut a, &mut seen_of_b);
    assert_whole(&a, &b, &both, "after both changes");

    undo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, &props[FIELD], Some(CHANGED));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &theirs, "after the first change was taken back");

    redo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, CHANGED, Some(&props[FIELD]));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &both, "after the first change was made again");
}

/// The same on a pipe line: a change of one property is said once, of the
/// line, and the peer's line follows it there and back.
#[test]
fn collab_large_props_one_changed_on_a_line_is_said_alone_and_the_peer_follows() {
    let PutBack {
        mut a,
        mut b,
        mut room,
        mut seen,
        said,
        props,
    } = erased_and_put_back(&[("PIDPIPE", LINE)], is_line, is_line);
    let tag = the_one(&said, "create_pipe_line")["tag"]
        .as_str()
        .expect("the line's tag")
        .to_owned();
    let changed = with(&props, FIELD, CHANGED);
    assert_eq!(
        props_where(&b, is_line),
        (props.clone(), false),
        "the fixture"
    );

    set_prop(&mut a, &tag, FIELD, CHANGED);
    assert_said_one(&a, seen, &tag, FIELD, CHANGED, Some(&props[FIELD]));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(props_where(&a, is_line), (changed.clone(), false));
    assert_eq!(
        props_where(&b, is_line),
        (changed, false),
        "the peer's line after the change"
    );

    undo(&mut a);
    assert_said_one(&a, seen, &tag, FIELD, &props[FIELD], Some(CHANGED));
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(
        props_where(&b, is_line),
        (props, false),
        "the peer's line after the change was taken back"
    );
}

// ── another symbol, when a step is taken back past a peer's change ──────

/// A second symbol, beside the one under [`TAG`].
const OTHER: &str = "E-9101";
const OTHER_AT: [f64; 2] = [300.0, 50.0];

fn is_other(author: &PidAuthor) -> bool {
    author.tag == OTHER
}

/// A step taken back puts the drawing's records back as they were when it
/// was made, all of them (`StructureSnapshot::Full`) — and the properties a
/// peer has since taken out of another symbol's own record are not among
/// those. That symbol keeps every one of them all the same, here, at the
/// peer and in the room; the step taken back, and made again, says its own
/// one property and nothing of the other symbol.
#[test]
fn collab_large_props_what_a_peer_took_out_of_a_record_since_outlives_a_step_taken_back() {
    let (many, few, long) = (many_props(), few_props(), long_value());
    let grown = with(&few, NOTE, &long);
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen, mut seen_of_b) = (Room::default(), 0, 0);
    place(&mut a, &many);
    place_as(&mut a, OTHER, OTHER_AT, &few);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(props_where(&b, is_other), (few, true), "to begin with");

    // The step that is going to be taken back, and the peer's change after
    // it.
    set_prop(&mut a, TAG, FIELD, CHANGED);
    relay(&mut room, &mut a, &mut b, &mut seen);
    set_prop(&mut b, OTHER, NOTE, &long);
    relay(&mut room, &mut b, &mut a, &mut seen_of_b);
    assert_eq!(props_where(&a, is_other), (grown.clone(), false));

    undo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, &many[FIELD], Some(CHANGED));
    assert_eq!(
        props_where(&a, is_other),
        (grown.clone(), false),
        "the other symbol here, the step taken back"
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &many, "after the step was taken back");
    assert_eq!(props_where(&b, is_other), (grown.clone(), false));

    redo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, CHANGED, Some(&many[FIELD]));
    assert_eq!(
        props_where(&a, is_other),
        (grown.clone(), false),
        "the other symbol here, the step made again"
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(
        &a,
        &b,
        &with(&many, FIELD, CHANGED),
        "after the step was made again",
    );
    assert_eq!(props_where(&b, is_other), (grown.clone(), false));

    let known = room
        .model
        .object(OTHER)
        .expect("the room knows the other symbol");
    for (field, value) in &grown {
        assert_eq!(
            known.fields.get(field).map(|state| state.value.as_str()),
            Some(value.as_str()),
            "the room's {field}"
        );
    }
}

/// The same of a symbol a peer has placed since, with properties that long.
/// The peer has changed a property of the first symbol as well, which is
/// what has the step taken back the long way round (`replay_remote_after`).
#[test]
fn collab_large_props_a_symbol_a_peer_placed_since_keeps_them_when_a_step_is_taken_back() {
    const THEIR_FIELD: &str = "字段02";
    const THEIRS: &str = "对端改的";
    let many = many_props();
    let theirs = with(&many, THEIR_FIELD, THEIRS);
    let Pair {
        mut a,
        mut b,
        mut room,
        mut seen,
    } = placed_at_both(&many);
    let mut seen_of_b = 0;
    set_prop(&mut a, TAG, FIELD, CHANGED);
    relay(&mut room, &mut a, &mut b, &mut seen);
    place_as(&mut b, OTHER, OTHER_AT, &many);
    set_prop(&mut b, TAG, THEIR_FIELD, THEIRS);
    relay(&mut room, &mut b, &mut a, &mut seen_of_b);
    assert_eq!(props_where(&a, is_other), (many.clone(), false));

    undo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, &many[FIELD], Some(CHANGED));
    assert_eq!(
        props_where(&a, is_other),
        (many.clone(), false),
        "the peer's symbol here, the step taken back"
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &theirs, "after the step was taken back");

    redo(&mut a);
    assert_said_one(&a, seen, TAG, FIELD, CHANGED, Some(&many[FIELD]));
    assert_eq!(
        props_where(&a, is_other),
        (many.clone(), false),
        "the peer's symbol here, the step made again"
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(
        &a,
        &b,
        &with(&theirs, FIELD, CHANGED),
        "after the step was made again",
    );
    assert_eq!(props_where(&b, is_other), (many, false));
}

/// Two steps taken back at once. The newer had brought the other symbol's
/// properties back within its record, after a peer had taken them out of
/// it; the older is of the first symbol, and the records it puts back are
/// from before any of that. The other symbol still gets back what it had
/// before the newer step, and that one property is all that is said of it.
#[test]
fn collab_large_props_two_steps_taken_back_at_once_leave_the_other_symbol_what_it_had() {
    let (many, few, long) = (many_props(), few_props(), long_value());
    let grown = with(&few, NOTE, &long);
    let short = with(&few, NOTE, "短");
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen, mut seen_of_b) = (Room::default(), 0, 0);
    place(&mut a, &many);
    place_as(&mut a, OTHER, OTHER_AT, &few);
    relay(&mut room, &mut a, &mut b, &mut seen);

    // The older step; the peer's change; the newer step.
    set_prop(&mut a, TAG, FIELD, CHANGED);
    relay(&mut room, &mut a, &mut b, &mut seen);
    set_prop(&mut b, OTHER, NOTE, &long);
    relay(&mut room, &mut b, &mut a, &mut seen_of_b);
    set_prop(&mut a, OTHER, NOTE, "短");
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(props_where(&a, is_other), (short.clone(), true));
    assert_eq!(props_where(&b, is_other), (short, true));

    a.undo_steps(2);
    let mut said: Vec<[String; 4]> = said_since(&a, seen)
        .iter()
        .map(|change| {
            assert_eq!(change["kind"], "set_property", "{change}");
            ["object_ref", "field", "value", "old_value"]
                .map(|key| change[key].as_str().unwrap_or_default().to_owned())
        })
        .collect();
    said.sort();
    assert_eq!(
        said,
        [
            [TAG, FIELD, many[FIELD].as_str(), CHANGED].map(str::to_owned),
            [OTHER, NOTE, long.as_str(), "短"].map(str::to_owned),
        ],
        "what taking both steps back said"
    );
    assert_eq!(
        props_where(&a, is_other),
        (grown.clone(), false),
        "the other symbol here, both steps taken back"
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_whole(&a, &b, &many, "after both steps were taken back");
    assert_eq!(props_where(&b, is_other), (grown, false));
}

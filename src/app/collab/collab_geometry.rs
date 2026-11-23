//! Geometry edits join the collaboration stream (#94, #95).
//!
//! Dragging a valve on a run queues the run as it stands afterwards — the
//! valve and the segment ends that followed it — and a peer applying that
//! op lands on the same picture and the same topology. Undoing the drag
//! queues the run as it was, and the peer follows back.
//!
//! A symbol standing on its own queues its position, its turn and its scale
//! as three fields. Position is one field, so two people writing it are on
//! one field. A tag somebody else holds is not moved.

use crate::app::collab::CollabEvent;
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{TopologyTable, ROLE_PIPE_SEGMENT};
use document::EntityType;
use serde_json::{json, Value};

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const DRAWING: &str = "PID-1101";

fn drawing() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
    app
}

fn session_dropped(app: &mut OpenCADStudio) {
    app.collab.apply(CollabEvent::Connected {
        session_id: "s-1".to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    });
    app.collab.apply(CollabEvent::Disconnected {
        reason: "connection reset".to_owned(),
        will_retry: true,
    });
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert("request_id".into(), json!(format!("collab-geom-{serial}")));
    }
    let (response, task) = app.control_request(req);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    response
}

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

fn line_and_valve(app: &mut OpenCADStudio) {
    draw(app, "PIDPIPE", &[[0.0, 0.0], [200.0, 0.0]]);
    draw(app, "PIDPIPE run", &[[0.0, 0.0], [200.0, 0.0]]);
    draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
}

fn valve_handle(app: &OpenCADStudio) -> document::Handle {
    let document = &app.tabs[app.active_tab].scene.document;
    let table = TopologyTable::load(document);
    assert_eq!(table.fittings.len(), 1, "one valve");
    crate::pid::topology::parse_handle(&table.fittings[0].handle).expect("valve handle")
}

fn valve_tag(app: &OpenCADStudio) -> String {
    let document = &app.tabs[app.active_tab].scene.document;
    let handle = valve_handle(app);
    PidAuthor::load(document, handle)
        .map(|author| author.tag)
        .filter(|tag| !tag.is_empty())
        .expect("the valve has a tag")
}

fn hex(handle: document::Handle) -> String {
    format!("{:X}", handle.value())
}

fn transform(app: &mut OpenCADStudio, body: Value) {
    let done = request(app, body);
    app.finish_all_pending_history();
    assert_eq!(done["ok"], true, "{done}");
}

fn queued_since(app: &OpenCADStudio, from: usize) -> Vec<Value> {
    app.collab
        .outbox
        .iter()
        .skip(from)
        .map(|entry| entry.payload.clone())
        .collect()
}

fn is_run_update(payload: &Value) -> bool {
    let kind = |value: &Value| value["kind"].as_str() == Some("update_pipe_run");
    kind(payload)
        || payload["changes"]
            .as_array()
            .is_some_and(|rest| rest.iter().any(kind))
}

/// Valve position, segment ends, and the topology table without handles.
fn picture(app: &OpenCADStudio) -> (f64, Vec<String>, Vec<String>) {
    let document = &app.tabs[app.active_tab].scene.document;
    let handle = valve_handle(app);
    let x = match document.get_entity(handle) {
        Some(EntityType::Insert(insert)) => insert.insert_point.x,
        other => panic!("valve is not an insert: {other:?}"),
    };
    let mut segments = Vec::new();
    for entity in document.entities() {
        let Some(author) = PidAuthor::from_entity(entity) else {
            continue;
        };
        if author.topology != ROLE_PIPE_SEGMENT {
            continue;
        }
        let EntityType::Line(line) = entity else {
            continue;
        };
        segments.push(format!(
            "{:.3},{:.3}-{:.3},{:.3}",
            line.start.x, line.start.y, line.end.x, line.end.y
        ));
    }
    segments.sort();
    let table = TopologyTable::load(document);
    let mut recorded = table
        .pipe_segments
        .iter()
        .map(|segment| {
            format!(
                "{:.3},{:.3}-{:.3},{:.3} peers={},{}",
                segment.start.point[0],
                segment.start.point[1],
                segment.end.point[0],
                segment.end.point[1],
                segment.start.peer.is_some(),
                segment.end.peer.is_some(),
            )
        })
        .collect::<Vec<_>>();
    recorded.sort();
    assert_eq!(table.fittings.len(), 1);
    assert_eq!(table.fittings[0].tag.as_deref(), Some(valve_tag(app).as_str()));
    (x, segments, recorded)
}

fn receive(app: &mut OpenCADStudio, revision: i64, payload: &Value) {
    app.collab.apply(CollabEvent::OpApplied {
        op_id: format!("op-peer-{revision}"),
        revision,
        changes: vec![payload.clone()],
        user_id: Some("u1".to_owned()),
        display_name: None,
    });
    app.apply_remote_changes();
    assert_eq!(
        app.collab.last_apply_error, None,
        "peer could not follow: {:?}",
        app.collab.diverged
    );
}

fn peer_of_the_same_valve() -> OpenCADStudio {
    let mut peer = drawing();
    line_and_valve(&mut peer);
    peer.collab.apply(CollabEvent::Connected {
        session_id: "s-2".to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    });
    peer
}

#[test]
fn dragging_a_valve_reaches_the_peer_and_undo_brings_both_back() {
    let mut author = drawing();
    session_dropped(&mut author);
    line_and_valve(&mut author);
    let before = picture(&author);
    assert!((before.0 - 100.0).abs() < 1e-3, "valve starts at 100");
    let queued = author.collab.outbox.len();
    let handle = hex(valve_handle(&author));

    transform(
        &mut author,
        json!({
            "op": "entities_transform",
            "action": "move",
            "handles": [handle],
            "vector": [40.0, 0.0, 0.0],
        }),
    );

    let added = queued_since(&author, queued);
    assert_eq!(added.len(), 1, "the drag is one op: {added:?}");
    assert!(is_run_update(&added[0]), "the run is said again: {}", added[0]);
    let dragged = picture(&author);
    assert!((dragged.0 - 140.0).abs() < 1e-3, "valve at 140, got {}", dragged.0);

    let mut peer = peer_of_the_same_valve();
    assert_eq!(picture(&peer).0, before.0);
    receive(&mut peer, 1, &added[0]);
    assert_eq!(picture(&peer), dragged, "peer valve, segments and topology");

    let queued = author.collab.outbox.len();
    let undone = author.automation_op(r#"{"op":"undo"}"#);
    assert_eq!(undone["ok"], true, "{undone}");
    author.finish_all_pending_history();
    let taken_back = queued_since(&author, queued);
    assert_eq!(taken_back.len(), 1, "undo is one op: {taken_back:?}");
    assert!(is_run_update(&taken_back[0]), "{}", taken_back[0]);
    assert_eq!(picture(&author), before, "author is back where the drag started");

    receive(&mut peer, 2, &taken_back[0]);
    assert_eq!(picture(&peer), before, "peer follows the undo");
}

#[test]
fn a_claimed_valve_cannot_be_dragged() {
    let mut author = drawing();
    session_dropped(&mut author);
    line_and_valve(&mut author);
    let tag = valve_tag(&author);
    author.collab.claims.insert(tag, "u2".to_owned());
    let queued = author.collab.outbox.len();
    let before = picture(&author);
    let handle = hex(valve_handle(&author));

    let refused = request(
        &mut author,
        json!({
            "op": "entities_transform",
            "action": "move",
            "handles": [handle],
            "vector": [40.0, 0.0, 0.0],
        }),
    );
    author.finish_all_pending_history();
    assert_eq!(refused["code"], "CLAIM_HELD", "{refused}");

    assert_eq!(author.collab.outbox.len(), queued, "a claimed valve sends nothing");
    assert_eq!(picture(&author), before, "the drag was taken back");
}

fn place_equipment(app: &mut OpenCADStudio) {
    let done = request(app, json!({"op":"run","cmd":"PIDINSERT VALVE 100,200"}));
    app.finish_all_pending_history();
    assert_eq!(done["ok"], true, "{done}");
}

fn insert_point(app: &OpenCADStudio, handle: document::Handle) -> [f64; 3] {
    match app.tabs[app.active_tab].scene.document.get_entity(handle) {
        Some(EntityType::Insert(insert)) => {
            [insert.insert_point.x, insert.insert_point.y, insert.insert_point.z]
        }
        other => panic!("not an insert: {other:?}"),
    }
}

fn symbol(app: &OpenCADStudio) -> (document::Handle, String) {
    let document = &app.tabs[app.active_tab].scene.document;
    let found: Vec<_> = document
        .entities()
        .filter_map(|entity| {
            let author = PidAuthor::from_entity(entity)?;
            if author.tag.is_empty() || !author.topology.is_empty() {
                return None;
            }
            let EntityType::Insert(insert) = entity else {
                return None;
            };
            Some((insert.common.handle, author.tag))
        })
        .collect();
    assert_eq!(found.len(), 1, "one standalone symbol");
    found.into_iter().next().unwrap()
}

fn symbol_pose(app: &OpenCADStudio) -> (String, String, String, f64) {
    let document = &app.tabs[app.active_tab].scene.document;
    let (handle, _) = symbol(app);
    let EntityType::Insert(insert) = document.get_entity(handle).unwrap() else {
        panic!("not an insert");
    };
    let label = insert
        .attributes
        .iter()
        .find(|attribute| attribute.tag == crate::app::commands::pidpalette::PID_TAG_ATTRIBUTE)
        .map(|attribute| attribute.insertion_point.x)
        .expect("the tag label");
    (
        super::broadcast::geometry_text_of_insert(insert, super::broadcast::FIELD_POSITION),
        super::broadcast::geometry_text_of_insert(insert, super::broadcast::FIELD_ROTATION),
        super::broadcast::geometry_text_of_insert(insert, super::broadcast::FIELD_SCALE),
        label,
    )
}

fn geometry_fields(payload: &Value) -> Vec<String> {
    let mut fields = Vec::new();
    let take = |value: &Value, fields: &mut Vec<String>| {
        if value["kind"].as_str() == Some("set_property") {
            if let Some(field) = value["field"].as_str() {
                if matches!(field, "position" | "rotation" | "scale") {
                    fields.push(field.to_owned());
                }
            }
        }
    };
    take(payload, &mut fields);
    if let Some(rest) = payload["changes"].as_array() {
        for change in rest {
            take(change, &mut fields);
        }
    }
    fields
}

#[test]
fn moving_turning_and_scaling_a_symbol_reaches_the_peer() {
    let mut author = drawing();
    session_dropped(&mut author);
    place_equipment(&mut author);
    let (handle, tag) = symbol(&author);
    let start = symbol_pose(&author);
    let mut sent = Vec::new();

    let step = |app: &mut OpenCADStudio, sent: &mut Vec<Value>, body: Value, field: &str| {
        let queued = app.collab.outbox.len();
        transform(app, body);
        let added = queued_since(app, queued);
        assert_eq!(added.len(), 1, "{field}: {added:?}");
        assert_eq!(geometry_fields(&added[0]), vec![field.to_owned()], "{}", added[0]);
        sent.push(added[0].clone());
    };
    step(
        &mut author,
        &mut sent,
        json!({
            "op": "entities_transform",
            "action": "move",
            "handles": [hex(handle)],
            "vector": [40.0, 0.0, 0.0],
        }),
        "position",
    );
    let (handle, _) = symbol(&author);
    let at = insert_point(&author, handle);
    step(
        &mut author,
        &mut sent,
        json!({
            "op": "entities_transform",
            "action": "rotate",
            "handles": [hex(handle)],
            "center": at,
            "angle_deg": 90.0,
        }),
        "rotation",
    );
    let (handle, _) = symbol(&author);
    let at = insert_point(&author, handle);
    step(
        &mut author,
        &mut sent,
        json!({
            "op": "entities_transform",
            "action": "scale",
            "handles": [hex(handle)],
            "center": at,
            "factor": 2.0,
        }),
        "scale",
    );
    let posed = symbol_pose(&author);
    assert_ne!(posed.0, start.0, "position moved");
    assert_ne!(posed.1, start.1, "rotation moved");
    assert_ne!(posed.2, start.2, "scale moved");
    assert!((posed.3 - start.3).abs() > 1.0, "the tag label moved with the symbol");

    let mut peer = drawing();
    place_equipment(&mut peer);
    peer.collab.apply(CollabEvent::Connected {
        session_id: "s-2".to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    });
    assert_eq!(symbol_pose(&peer).0, start.0);
    for (index, payload) in sent.iter().enumerate() {
        receive(&mut peer, (index + 1) as i64, payload);
    }
    let followed = symbol_pose(&peer);
    assert_eq!(followed.0, posed.0, "position");
    assert_eq!(followed.1, posed.1, "rotation");
    assert_eq!(followed.2, posed.2, "scale");
    assert!((followed.3 - start.3).abs() > 1.0, "peer's tag label moved");
    assert_eq!(symbol(&peer).1, tag);
}

#[test]
fn a_claimed_symbol_cannot_be_moved() {
    let mut author = drawing();
    session_dropped(&mut author);
    place_equipment(&mut author);
    let (handle, tag) = symbol(&author);
    author.collab.claims.insert(tag, "u2".to_owned());
    let queued = author.collab.outbox.len();
    let before = symbol_pose(&author);

    let refused = request(
        &mut author,
        json!({
            "op": "entities_transform",
            "action": "move",
            "handles": [hex(handle)],
            "vector": [40.0, 0.0, 0.0],
        }),
    );
    author.finish_all_pending_history();
    assert_eq!(refused["code"], "CLAIM_HELD", "{refused}");

    assert_eq!(author.collab.outbox.len(), queued);
    assert_eq!(symbol_pose(&author).0, before.0);
}

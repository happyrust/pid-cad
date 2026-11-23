//! Pipe lines, runs and what is fitted on them, said to the room and
//! followed by a peer (`VAL-CROSS-003`, plan §1 of
//! `docs/plans/2026-11-12-collab-completion.md`).
//!
//! Two editors in one process and the server's own judgment between them
//! (`ocs_collab::conflict::DrawingModel`, the code the server runs): what
//! one draws, erases, takes back and puts on again goes through the room to
//! the other, and after each step the two drawings are compared — the
//! entities that carry an author record, and the topology table as far as
//! it speaks of entities that are there. Handles are left out of both
//! sides of the comparison: an entity a peer made has whatever handle the
//! peer's drawing gave it.
//!
//! The last section is what rides with a value (review N4): the override
//! mark of a run's field changed by hand, and where a tag came from.

use crate::app::collab::{broadcast, CollabEvent};
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::topology::{handle_text, parse_handle, TopologyTable};
use document::{EntityType, Handle};
use ocs_collab::conflict::{Change as Judged, DrawingModel, Verdict};
use serde_json::{json, Value};
use std::collections::BTreeMap;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        object.insert("request_id".into(), json!(format!("collab-pipe-{serial}")));
    }
    let (response, task) = app.control_request(req);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    response
}

/// Start `cmd`, give it `points`, press Enter; the step is closed.
fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) -> Value {
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
    done
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

/// Every entity of the drawing in front that carries an author record, as
/// what it is, where it is and what it says — its owner by what the owner
/// is, not by handle. Sorted.
fn authored(app: &OpenCADStudio) -> Vec<String> {
    let document = &app.tabs[app.active_tab].scene.document;
    let name = |text: &str| -> String {
        match parse_handle(text).and_then(|handle| document.get_entity(handle).map(|e| (handle, e)))
        {
            Some((handle, entity)) => {
                let tag = PidAuthor::load(document, handle)
                    .map(|a| a.tag)
                    .unwrap_or_default();
                if tag.is_empty() {
                    shape(entity)
                } else {
                    tag
                }
            }
            None if text.is_empty() => String::new(),
            None => "(gone)".to_owned(),
        }
    };
    let mut rows: Vec<String> = document
        .entities()
        .filter_map(|entity| {
            let author = PidAuthor::load(document, entity.common().handle)?;
            let (props, marks) = split_props(&author.props);
            Some(format!(
                "{} | type={} tag={} topology={} owner={} props={:?} overrides={:?}",
                shape(entity),
                author.type_code,
                author.tag,
                author.topology,
                name(&author.owner),
                props,
                marks
            ))
        })
        .collect();
    rows.sort();
    rows
}

/// The topology table of the drawing in front as far as it speaks of
/// entities that are there, with every handle replaced by what it stands
/// for: a tag, or a kind and a place.
fn topology(app: &OpenCADStudio) -> Value {
    let document = &app.tabs[app.active_tab].scene.document;
    let mut table = TopologyTable::load(document);
    table.prune(document);
    let entity = |text: &str| parse_handle(text).and_then(|handle| document.get_entity(handle));
    let fitting_names: BTreeMap<String, String> = table
        .fittings
        .iter()
        .filter_map(|fitting| {
            let name = match &fitting.tag {
                Some(tag) => tag.clone(),
                None => format!(
                    "{} {}",
                    fitting.fitting_type,
                    shape(entity(&fitting.handle)?)
                ),
            };
            Some((fitting.handle.clone(), name))
        })
        .collect();
    let peer = |peer: &Option<String>| -> Value {
        let Some((handle, port)) = peer.as_deref().and_then(|text| text.split_once(':')) else {
            return Value::Null;
        };
        match fitting_names.get(handle) {
            Some(name) => json!(format!("{name}:{port}")),
            // A fitting that is gone: nothing is connected there any more.
            None => Value::Null,
        }
    };
    let segment = |handle: &str| -> Value {
        match table
            .pipe_segments
            .iter()
            .find(|record| record.handle == handle)
        {
            Some(record) => json!({
                "entity": entity(handle).map(shape),
                "start": at(record.start.point[0], record.start.point[1]),
                "end": at(record.end.point[0], record.end.point[1]),
                "start_peer": peer(&record.start.peer),
                "end_peer": peer(&record.end.peer),
            }),
            None => Value::Null,
        }
    };
    // (Not `next_run_ordinal`: the number the next run drawn here gets is
    // each drawing's own count. Taking a run back gives its number back in
    // the drawing that drew it and not in the peer's; the two never give
    // one number to two runs, which is what the tests below hold it to.)
    let mut lines: Vec<Value> = table
        .pipe_lines
        .iter()
        .map(|line| json!({"tag": line.tag, "runs": line.runs}))
        .collect();
    lines.sort_by_key(|line| line["tag"].as_str().unwrap_or_default().to_owned());
    let mut runs: Vec<Value> = table
        .pipe_runs
        .iter()
        .map(|run| {
            json!({
                "tag": run.tag, "line_tag": run.line_tag, "props": run.props, "overrides": run.overrides,
                "segments": run.segments.iter().map(|handle| segment(handle)).collect::<Vec<_>>(),
            })
        })
        .collect();
    runs.sort_by_key(|run| run["tag"].as_str().unwrap_or_default().to_owned());
    let mut fittings: Vec<Value> = table
        .fittings
        .iter()
        .map(|fitting| {
            json!({
                "name": fitting_names.get(&fitting.handle),
                "run_tag": fitting.run_tag, "fitting_type": fitting.fitting_type, "tag": fitting.tag,
                "ports": fitting.ports.iter().map(|(port, bound)| {
                    (port.clone(), bound.as_deref().map(|handle| segment(handle)["entity"].clone()).unwrap_or(Value::Null))
                }).collect::<serde_json::Map<String, Value>>(),
            })
        })
        .collect();
    fittings.sort_by_key(|fitting| fitting.to_string());
    json!({"pipe_lines": lines, "pipe_runs": runs, "fittings": fittings})
}

/// The two drawings are the same in everything this module compares.
#[track_caller]
fn assert_same(a: &OpenCADStudio, b: &OpenCADStudio, when: &str) {
    assert_eq!(authored(a), authored(b), "entities differ {when}");
    assert_eq!(topology(a), topology(b), "topology differs {when}");
}

fn table(app: &OpenCADStudio) -> TopologyTable {
    TopologyTable::load(&app.tabs[app.active_tab].scene.document)
}

fn handles_where(app: &OpenCADStudio, keep: impl Fn(&PidAuthor) -> bool) -> Vec<Handle> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .map(|entity| entity.common().handle)
        .filter(|handle| PidAuthor::load(document, *handle).is_some_and(|author| keep(&author)))
        .collect()
}

const LINE: [[f64; 2]; 2] = [[0.0, 0.0], [200.0, 0.0]];

/// A line, a run on it and a gate valve in the middle of the run.
fn line_run_and_valve(app: &mut OpenCADStudio) {
    assert_eq!(draw(app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(app, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(
        draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]])["ok"],
        true
    );
}

/// What is drawn is said: the line as a line with its stretch, the run as a
/// run of that line, the valve as a fitting put on that run — each with
/// what the object is like once the step is done.
#[test]
fn a_line_a_run_and_a_valve_are_said_to_the_room() {
    let mut a = editor("s-a");
    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(a.collab.sent.len(), 1, "the line was not said");
    let line = a.collab.sent[0].clone();
    assert_eq!(line["kind"], "create_pipe_line", "{line}");
    assert_eq!(line["tag"], "L-1001", "{line}");
    assert_eq!(line["points"], json!([[0.0, 0.0], [200.0, 0.0]]), "{line}");

    assert_eq!(draw(&mut a, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(a.collab.sent.len(), 2, "the run was not said");
    let run = a.collab.sent[1].clone();
    assert_eq!(run["kind"], "add_pipe_run", "{run}");
    assert_eq!(
        (&run["line_tag"], &run["tag"]),
        (&json!("L-1001"), &json!("L-1001-01")),
        "{run}"
    );
    assert_eq!(run["points"], json!([[0.0, 0.0], [200.0, 0.0]]), "{run}");
    assert_eq!(
        run["run"]["segments"].as_array().map(Vec::len),
        Some(1),
        "{run}"
    );

    assert_eq!(
        draw(&mut a, "PIDFITTING gate_valve", &[[100.0, 0.0]])["ok"],
        true
    );
    assert_eq!(a.collab.sent.len(), 3, "the valve was not said");
    let valve = a.collab.sent[2].clone();
    assert_eq!(valve["kind"], "insert_fitting", "{valve}");
    assert_eq!(valve["run_tag"], "L-1001-01", "{valve}");
    assert_eq!(valve["fitting_type"], "gate_valve", "{valve}");
    assert_eq!(valve["at"], json!([100.0, 0.0]), "{valve}");
    let tag = valve["tag"].as_str().unwrap_or_default().to_owned();
    assert!(tag.starts_with("V-"), "{valve}");
    let state = &valve["run"];
    assert_eq!(
        state["segments"].as_array().map(Vec::len),
        Some(2),
        "{valve}"
    );
    assert_eq!(
        state["fittings"].as_array().map(Vec::len),
        Some(1),
        "{valve}"
    );
    assert_eq!(state["fittings"][0]["tag"], tag.as_str(), "{valve}");
    assert_eq!(
        state["fittings"][0]["ports"],
        json!({"连接点1": 0, "连接点2": 1}),
        "{valve}"
    );
    assert_eq!(state["segments"][0]["end_peer"], "0:连接点1", "{valve}");
    assert_eq!(state["segments"][1]["start_peer"], "0:连接点2", "{valve}");
}

/// `VAL-CROSS-003`: one draws a line, a run on it and a valve in the run;
/// the other's drawing has them, and its topology says what the first's
/// says — two segments, the valve's two connection points on the ends that
/// face it, no overlap and no gap but the valve itself. None of it is a
/// step of the second person's, and none of it goes back to the room.
#[test]
fn a_peer_follows_a_line_a_run_and_a_valve() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    let steps = b.tabs[b.active_tab].history.undo_stack.len();

    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the line");
    assert_eq!(draw(&mut a, "PIDPIPE run", &LINE)["ok"], true);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the run");
    assert_eq!(
        table(&b).pipe_lines[0].next_run_ordinal,
        2,
        "the next run drawn in the peer's drawing would take this one's number"
    );
    assert_eq!(
        draw(&mut a, "PIDFITTING gate_valve", &[[100.0, 0.0]])["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the valve");
    assert_eq!(room.revision, 3, "three steps, three revisions");

    // The second drawing's own table, read as the contract reads it.
    let theirs = table(&b);
    assert_eq!(theirs.pipe_lines.len(), 1);
    assert_eq!(theirs.pipe_lines[0].runs, ["L-1001-01"]);
    assert_eq!(
        theirs.pipe_runs[0].segments.len(),
        2,
        "the run is two segments"
    );
    let ends = theirs.segment_ends_of("L-1001-01");
    let (first, second) = (&ends[0], &ends[1]);
    assert_eq!((first.0.point, second.1.point), ([0.0, 0.0], [200.0, 0.0]));
    assert!(
        first.1.point[0] < second.0.point[0],
        "the two segments overlap: {ends:?}"
    );
    assert_eq!(theirs.fittings.len(), 1);
    let valve = &theirs.fittings[0];
    assert_eq!(valve.fitting_type, "gate_valve");
    assert_eq!(
        valve.ports.get("连接点1"),
        Some(&Some(theirs.pipe_runs[0].segments[0].clone()))
    );
    assert_eq!(
        valve.ports.get("连接点2"),
        Some(&Some(theirs.pipe_runs[0].segments[1].clone()))
    );
    assert_eq!(
        first.1.peer.as_deref(),
        Some(format!("{}:连接点1", valve.handle).as_str())
    );
    assert_eq!(
        second.0.peer.as_deref(),
        Some(format!("{}:连接点2", valve.handle).as_str())
    );
    // The valve stands between the two inner ends, as wide as the gap.
    let mine = table(&a);
    assert_eq!(
        mine.segment_ends_of("L-1001-01")
            .iter()
            .map(|e| (e.0.point, e.1.point))
            .collect::<Vec<_>>(),
        ends.iter()
            .map(|e| (e.0.point, e.1.point))
            .collect::<Vec<_>>()
    );

    assert_eq!(
        b.tabs[b.active_tab].history.undo_stack.len(),
        steps,
        "not the peer's own steps"
    );
    assert!(b.collab.sent.is_empty(), "nothing goes back to the room");
    // What the server knows of it.
    let known = room.model.topology();
    assert_eq!(
        known["pipe_lines"],
        json!([{"tag": "L-1001", "runs": ["L-1001-01"]}])
    );
    assert_eq!(
        known["pipe_runs"][0]["fittings"].as_array().map(Vec::len),
        Some(1),
        "{known}"
    );
}

/// The valve erased, then the run with everything of it, then the line:
/// after each the peer's drawing is what the first one's is.
#[test]
fn a_peer_follows_a_valve_a_run_and_a_line_being_erased() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    line_run_and_valve(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "to begin with");

    let valve = handles_where(&a, |author| author.type_code == "VALVE");
    assert_eq!(valve.len(), 1);
    erase(&mut a, &valve);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the valve was erased");
    assert!(
        handles_where(&b, |author| author.type_code == "VALVE").is_empty(),
        "the peer still has the valve"
    );
    assert_eq!(
        handles_where(&b, |author| author.topology == "pipe_segment").len(),
        2,
        "the segments stay"
    );
    assert_eq!(
        room.model.topology()["pipe_runs"][0]["fittings"],
        json!([]),
        "the server still lists the valve"
    );

    let run: Vec<Handle> = handles_where(&a, |author| {
        author.topology == "pipe_run" || author.topology == "pipe_segment"
    });
    assert_eq!(run.len(), 3);
    erase(&mut a, &run);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the run was erased");
    assert!(
        handles_where(&b, |author| author.topology != "pipe_line").is_empty(),
        "the peer kept part of the run"
    );
    assert_eq!(room.model.topology()["pipe_runs"], json!([]));

    let line = handles_where(&a, |author| author.topology == "pipe_line");
    erase(&mut a, &line);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the line was erased");
    assert!(
        handles_where(&b, |_| true).is_empty(),
        "the peer kept something"
    );
}

/// Taking the three steps back one by one and putting them on again: the
/// peer's drawing follows each. Taking the valve back is not erasing it —
/// the run is one segment again.
#[test]
fn a_peer_follows_pipe_steps_taken_back_and_put_on_again() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    line_run_and_valve(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);

    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the valve was taken back");
    let ends = table(&b).segment_ends_of("L-1001-01");
    assert_eq!(ends.len(), 1, "the run is one segment again: {ends:?}");
    assert_eq!(
        (ends[0].0.point, ends[0].1.point),
        ([0.0, 0.0], [200.0, 0.0])
    );

    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the run was taken back");
    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after the line was taken back");
    assert!(handles_where(&b, |_| true).is_empty());

    for step in ["the line", "the run", "the valve"] {
        redo(&mut a);
        relay(&mut room, &mut a, &mut b, &mut seen);
        assert_same(&a, &b, &format!("after {step} was put on again"));
    }
    assert_eq!(table(&b).segment_ends_of("L-1001-01").len(), 2);
    assert_eq!(
        handles_where(&b, |author| author.type_code == "VALVE").len(),
        1
    );

    // The run's number: back in the first drawing when the run was taken
    // back, kept in the peer's; with the run on again both are past it, and
    // a run the peer draws now does not take it.
    assert_eq!(table(&a).pipe_lines[0].next_run_ordinal, 2);
    assert_eq!(table(&b).pipe_lines[0].next_run_ordinal, 2);
    let mut seen_b = 0;
    assert_eq!(
        draw(&mut b, "PIDPIPE run L-1001", &[[0.0, 40.0], [200.0, 40.0]])["ok"],
        true
    );
    assert_eq!(b.collab.sent[0]["tag"], "L-1001-02", "{}", b.collab.sent[0]);
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_same(&a, &b, "after the peer drew a run of its own");
}

/// A break in a run and a signal line are followed like the rest.
#[test]
fn a_peer_follows_a_break_and_a_signal_line() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(&mut a, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(draw(&mut a, "PIDBREAK", &[[60.0, 0.0]])["ok"], true);
    assert_eq!(
        draw(
            &mut a,
            "PIDPIPE signal",
            &[[0.0, 50.0], [80.0, 50.0], [80.0, 90.0]]
        )["ok"],
        true
    );
    assert_eq!(a.collab.sent.len(), 4, "{:?}", a.collab.sent);
    assert_eq!(a.collab.sent[2]["kind"], "insert_fitting");
    assert_eq!(a.collab.sent[2]["fitting_type"], "break");
    assert_eq!(a.collab.sent[3]["kind"], "create_pipe_line");
    assert_eq!(a.collab.sent[3]["type"], "SIGNAL_LINE");
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_same(&a, &b, "after a break and a signal line");
    let theirs = table(&b);
    assert_eq!(
        theirs.segment_ends_of("L-1001-01").len(),
        2,
        "the break splits the run"
    );
    assert!(theirs
        .pipe_segments
        .iter()
        .all(|s| s.start.peer.is_none() && s.end.peer.is_none()));
    assert_eq!(
        handles_where(&b, |author| author.topology == "signal_line").len(),
        2
    );
    assert_eq!(
        room.model.topology()["pipe_lines"],
        json!([{"tag": "L-1001", "runs": ["L-1001-01"]}]),
        "a signal line is not a pipe line to the server"
    );
}

/// A run a peer drew after a step of this person's is still whole — in the
/// drawing and in the table — when that step is taken back: the step's
/// image of the table is from before the peer's run.
#[test]
fn taking_back_ones_own_pipe_step_leaves_a_peers_later_run_whole() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen_a, mut seen_b) = (Room::default(), 0, 0);
    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    relay(&mut room, &mut a, &mut b, &mut seen_a);
    // The second person's own step: another line.
    assert_eq!(
        draw(&mut b, "PIDPIPE", &[[0.0, 300.0], [200.0, 300.0]])["ok"],
        true
    );
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    // Then the first draws a run and puts a valve in it.
    assert_eq!(draw(&mut a, "PIDPIPE run L-1001", &LINE)["ok"], true);
    assert_eq!(
        draw(&mut a, "PIDFITTING gate_valve", &[[100.0, 0.0]])["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen_a);
    assert_same(&a, &b, "before the undo");

    let entities = |app: &OpenCADStudio| app.tabs[app.active_tab].scene.document.entities().count();
    let valve_block = |app: &OpenCADStudio| {
        app.tabs[app.active_tab]
            .scene
            .document
            .block_records
            .contains("PID_SYM_valve_gate_valve")
    };
    let all = entities(&b);
    undo(&mut b);
    assert_eq!(
        entities(&b),
        all - 1,
        "taking one line back changed the drawing by more than that line"
    );
    assert!(valve_block(&b), "the peer's valve lost its block");
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_same(&a, &b, "after the second person took their line back");
    let theirs = table(&b);
    assert_eq!(
        theirs
            .pipe_lines
            .iter()
            .map(|l| l.tag.as_str())
            .collect::<Vec<_>>(),
        ["L-1001"]
    );
    assert_eq!(
        theirs.segment_ends_of("L-1001-01").len(),
        2,
        "the peer's run lost its records"
    );
    assert_eq!(theirs.fittings.len(), 1);
    redo(&mut b);
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_same(&a, &b, "after it was put on again");

    // What is drawn after such an undo gets a handle of its own. (The
    // step's image of the drawing's structure includes where handles were
    // given out from when it was made — before the peer's run — and a
    // handle given out twice replaces the entity that had it.)
    undo(&mut b);
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    let all = entities(&b);
    assert_eq!(
        draw(&mut b, "PIDPIPE", &[[0.0, 500.0], [200.0, 500.0]])["ok"],
        true
    );
    assert_eq!(
        entities(&b),
        all + 1,
        "the new line took the place of something"
    );
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_same(&a, &b, "after a line drawn following the undo");
}

/// Two people each draw a line and each numbers it the same. The tag is
/// the line: both drawings end with one line under it, drawn in both
/// places.
#[test]
fn two_lines_drawn_under_one_tag_are_one_line_in_both_drawings() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen_a, mut seen_b) = (Room::default(), 0, 0);
    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(
        draw(&mut b, "PIDPIPE", &[[0.0, 300.0], [200.0, 300.0]])["ok"],
        true
    );
    assert_eq!(a.collab.sent[0]["tag"], b.collab.sent[0]["tag"]);
    relay(&mut room, &mut a, &mut b, &mut seen_a);
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_eq!(authored(&a), authored(&b));
    assert_eq!(handles_where(&a, |author| author.tag == "L-1001").len(), 2);
    for app in [&a, &b] {
        let lines = table(app).pipe_lines;
        assert_eq!(
            lines.iter().map(|l| l.tag.as_str()).collect::<Vec<_>>(),
            ["L-1001"]
        );
    }
}

/// The statements read back as they were written, and a peer that is told
/// of a run without being told what it is like cannot follow it.
#[test]
fn the_pipe_statements_survive_the_wire() {
    let mut a = editor("s-a");
    line_run_and_valve(&mut a);
    let valve = handles_where(&a, |author| author.type_code == "VALVE");
    erase(&mut a, &valve);
    assert_eq!(a.collab.sent.len(), 4);
    for payload in &a.collab.sent {
        let read = broadcast::changes_from_payload(std::slice::from_ref(payload));
        assert!(read.unread.is_empty(), "{payload}: {:?}", read.unread);
        let again: Vec<Value> = read
            .changes
            .iter()
            .map(broadcast::Change::to_json)
            .collect();
        let reread = broadcast::changes_from_payload(&again);
        assert_eq!(read.changes, reread.changes, "{payload}");
    }
    let erased = broadcast::changes_from_payload(std::slice::from_ref(&a.collab.sent[3])).changes;
    assert_eq!(
        erased
            .iter()
            .map(|change| change.kind.as_str())
            .collect::<Vec<_>>(),
        ["delete_object", "update_pipe_run"],
        "an erased valve is its tag gone and its run as it is now"
    );

    let bare = broadcast::changes_from_payload(&[json!({
        "kind": "insert_fitting", "run_tag": "L-1001-01", "fitting_type": "gate_valve", "at": [50, 0],
    })]);
    assert_eq!(
        bare.unread,
        ["insert_fitting"],
        "a fitting with no account of its run"
    );
    // A run as `architecture.md` §3.2 writes it reads as its points.
    let plain = broadcast::changes_from_payload(&[json!({
        "kind": "add_pipe_run", "line_tag": "L-1001", "tag": "L-1001-02", "points": [[0, 0], [100, 0], [100, 50]],
    })]);
    assert!(plain.unread.is_empty(), "{plain:?}");
    assert_eq!(
        plain.changes[0].to_json()["run"]["segments"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

// ── Override marks, and where a tag came from (review N4) ────────────────

/// Run a command that asks for nothing more; the step is closed.
fn command(app: &mut OpenCADStudio, cmd: &str) -> Value {
    let done = request(app, json!({"op":"run","cmd":cmd}));
    app.finish_all_pending_history();
    done
}

const RUN: &str = "L-1001-01";
const GRADE: &str = "管道等级";

/// What run [`RUN`] says for the grade, and whether it holds it marked.
fn grade_of_run(app: &OpenCADStudio) -> (String, bool) {
    let body = handles_where(app, |author| {
        author.topology == "pipe_run" && author.tag == RUN
    });
    assert_eq!(body.len(), 1, "one run {RUN}");
    let document = &app.tabs[app.active_tab].scene.document;
    let (values, marks) = split_props(&PidAuthor::load(document, body[0]).expect("author").props);
    (
        values.get(GRADE).cloned().unwrap_or_default(),
        marks.contains(GRADE),
    )
}

fn grade(value: &str, marked: bool) -> (String, bool) {
    (value.to_owned(), marked)
}

/// A line graded `A1A` with one run on it, in both drawings.
fn a_graded_line_and_its_run(
    room: &mut Room,
    a: &mut OpenCADStudio,
    b: &mut OpenCADStudio,
    seen: &mut usize,
) {
    assert_eq!(draw(a, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(a, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(command(a, "PIDSETPROP L-1001 管道等级 A1A")["ok"], true);
    relay(room, a, b, seen);
    assert_same(a, b, "to begin with");
    assert_eq!(grade_of_run(b), grade("A1A", false));
}

/// A run's field changed by hand stops taking its line's value — in the
/// peer's drawing as in the one it was changed in. The peer used to get the
/// value without the mark: its run took the line's value the next time that
/// changed, and the first drawing's kept its own (review N4).
#[test]
fn collab_override_marks_a_peers_run_keeps_the_field_changed_by_hand() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    a_graded_line_and_its_run(&mut room, &mut a, &mut b, &mut seen);

    assert_eq!(
        command(&mut a, "PIDSETPROP L-1001-01 管道等级 B2B")["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(grade_of_run(&a), grade("B2B", true));
    assert_eq!(
        grade_of_run(&b),
        grade("B2B", true),
        "the peer's run is not marked"
    );
    assert_same(&a, &b, "after the run's grade was changed by hand");

    // The line's grade moves on: the run keeps its own, in both drawings.
    assert_eq!(
        command(&mut a, "PIDSETPROP L-1001 管道等级 C3C")["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(grade_of_run(&a), grade("B2B", true));
    assert_eq!(
        grade_of_run(&b),
        grade("B2B", true),
        "the peer's run followed its line"
    );
    assert_same(&a, &b, "after the line's grade moved on");

    // And when it is the peer that changes the line's.
    let mut seen_b = 0;
    assert_eq!(
        command(&mut b, "PIDSETPROP L-1001 管道等级 D4D")["ok"],
        true
    );
    relay(&mut room, &mut b, &mut a, &mut seen_b);
    assert_eq!(grade_of_run(&a), grade("B2B", true));
    assert_eq!(grade_of_run(&b), grade("B2B", true));
    assert_same(&a, &b, "after the peer changed the line's grade");
}

/// Taking the change back takes the mark back with it, in both drawings —
/// the run follows its line again — and putting it on again marks both.
#[test]
fn collab_override_marks_a_hand_edit_taken_back_unmarks_the_peers_run() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    a_graded_line_and_its_run(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(
        command(&mut a, "PIDSETPROP L-1001-01 管道等级 B2B")["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(
        grade_of_run(&b),
        grade("B2B", true),
        "the peer's run is not marked"
    );

    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(grade_of_run(&a), grade("A1A", false));
    assert_eq!(
        grade_of_run(&b),
        grade("A1A", false),
        "the peer's run is still marked"
    );
    assert_same(&a, &b, "after the change was taken back");

    redo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(grade_of_run(&b), grade("B2B", true));
    assert_same(&a, &b, "after the change was put on again");

    // Taken back once more, the run takes what its line is given next.
    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(
        command(&mut a, "PIDSETPROP L-1001 管道等级 C3C")["ok"],
        true
    );
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(grade_of_run(&a), grade("C3C", false));
    assert_eq!(grade_of_run(&b), grade("C3C", false));
    assert_same(&a, &b, "after the line's grade moved on");
}

/// A tag typed by hand is one somebody typed in the peer's drawing too; and
/// when the retag is taken back, the peer's symbol has the tag the rule had
/// given it as the rule's own again. It used to stay "typed by hand" there
/// (review N4).
#[test]
fn collab_tag_source_a_peers_symbol_keeps_where_its_tag_came_from() {
    let (mut a, mut b) = (editor("s-a"), editor("s-b"));
    let (mut room, mut seen) = (Room::default(), 0);
    assert_eq!(command(&mut a, "PIDINSERT VALVE 100,200")["ok"], true);
    relay(&mut room, &mut a, &mut b, &mut seen);
    // The valve's tag, and where it came from.
    let valve = |app: &OpenCADStudio| {
        let handles = handles_where(app, |author| author.type_code == "VALVE");
        assert_eq!(handles.len(), 1, "one valve");
        let document = &app.tabs[app.active_tab].scene.document;
        let author = PidAuthor::load(document, handles[0]).expect("author");
        (author.tag, author.tag_source)
    };
    let is = |tag: &str, source: &str| (tag.to_owned(), source.to_owned());
    let (tag, source) = valve(&a);
    assert_eq!(source, "auto", "a tag the rule gave");
    assert_eq!(valve(&b), is(&tag, "auto"));

    let retag = format!("PIDTAG {tag} V-9001");
    assert_eq!(command(&mut a, &retag)["ok"], true);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(valve(&a), is("V-9001", "manual"));
    assert_eq!(valve(&b), is("V-9001", "manual"));

    undo(&mut a);
    relay(&mut room, &mut a, &mut b, &mut seen);
    assert_eq!(valve(&a), is(&tag, "auto"));
    assert_eq!(
        valve(&b),
        is(&tag, "auto"),
        "the peer's valve still reads as typed by hand"
    );
    assert_same(&a, &b, "after the retag was taken back");
}

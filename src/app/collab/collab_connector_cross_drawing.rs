//! Page connectors in a session: one made or taken away is said to the
//! room and followed by a peer on the same drawing; one whose other end is
//! on another drawing is paired by the server, and what the line that
//! flows out carries reaches the line it flows in on (`VAL-CROSS-009`,
//! plan §3 of `docs/plans/2026-11-12-collab-completion.md`).
//!
//! Editors in one process, each on a drawing of its own or two on one, and
//! between them a stand-in for the server made of the server's own parts:
//! its judgment of each change (`ocs_collab::conflict::DrawingModel`) and
//! the code it pairs connectors and carries fields with
//! (`ocs_collab::link::follow`). Each drawing is a room with a revision of
//! its own.

use crate::app::collab::connectors::{
    ConnectorSaid, KIND_CREATE_CONNECTOR, KIND_DELETE_CONNECTOR, KIND_PAIR_CONNECTOR,
};
use crate::app::collab::pipes::Pipe;
use crate::app::collab::{broadcast, CollabEvent};
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::topology::{parse_handle, TopologyTable};
use document::EntityType;
use ocs_collab::conflict::{Change as Judged, DrawingModel, Verdict};
use ocs_collab::link::{follow, Rooms, LINK_SESSION_PREFIX};
use serde_json::{json, Value};
use std::collections::BTreeMap;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

const FIRST: &str = "PID-1101";
const SECOND: &str = "PID-1102";
const LINE: [[f64; 2]; 2] = [[0.0, 0.0], [200.0, 0.0]];
const GRADE: &str = "管道等级";
const BORE: &str = "公称直径";

/// An editor on a new drawing, in a session on `drawing`.
fn editor(session: &str) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.user_id = session.to_owned();
    app.collab.apply(CollabEvent::Connected {
        session_id: session.to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    });
    app
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert("request_id".into(), json!(format!("collab-connector-{serial}")));
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

fn run(app: &mut OpenCADStudio, cmd: &str) -> Value {
    let done = request(app, json!({"op":"run","cmd":cmd}));
    app.finish_all_pending_history();
    done
}

fn undo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"undo"}"#);
}

fn redo(app: &mut OpenCADStudio) {
    let _ = app.automation_op(r#"{"op":"redo"}"#);
}

/// The drawings as the server has them: a model each, and everything told
/// to each room, in order — `(drawing, op_id, revision, changes)`.
#[derive(Default)]
struct Desk {
    models: BTreeMap<String, DrawingModel>,
    told: Vec<(String, String, i64, Vec<Value>)>,
}

impl Desk {
    /// The server takes one op of a session's on `drawing` — or refuses it,
    /// with the code it refuses under — and follows what follows from it.
    fn take(
        &mut self,
        drawing: &str,
        session: &str,
        op_id: &str,
        base: i64,
        payload: &Value,
    ) -> Result<(), String> {
        let model = self.models.entry(drawing.to_owned()).or_default();
        let changes = Judged::list(payload);
        assert!(
            !changes.is_empty() && changes.iter().all(Judged::is_understood),
            "the server has no rule for {payload}"
        );
        let mut taken = Vec::new();
        for change in changes {
            let change = model.completed(&change);
            match model.judge(&change, base, session) {
                Verdict::Apply { object_ref, .. } => taken.push((change, object_ref)),
                Verdict::DuplicateInflow { .. } => {
                    return Err(ocs_collab::protocol::codes::DUPLICATE_INFLOW_CONNECTOR.to_owned())
                }
                other => panic!("the server refused {}: {other:?}", change.raw),
            }
        }
        let revision = model.revision() + 1;
        let mut said = Vec::new();
        for (change, object_ref) in taken {
            model.commit(&change, &object_ref, revision, session, session);
            said.push(change.broadcast_form(&object_ref));
        }
        model.set_revision(revision);
        self.told
            .push((drawing.to_owned(), op_id.to_owned(), revision, said.clone()));
        follow(self, drawing, revision, &said);
        Ok(())
    }

    fn value(&self, drawing: &str, line: &str, field: &str) -> Option<String> {
        Some(self.models.get(drawing)?.field(line, field)?.value.clone())
    }
}

impl Rooms for Desk {
    fn drawings(&mut self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }

    fn model(&mut self, drawing_id: &str) -> &DrawingModel {
        self.models.entry(drawing_id.to_owned()).or_default()
    }

    fn commit(
        &mut self,
        drawing_id: &str,
        op_id: &str,
        cause: &str,
        changes: Vec<Value>,
    ) -> Option<i64> {
        let session = format!("{LINK_SESSION_PREFIX}{cause}");
        let model = self.models.entry(drawing_id.to_owned()).or_default();
        let revision = model.revision() + 1;
        for raw in &changes {
            let change = Judged::list(raw).into_iter().next().expect("a change");
            let name = change.object_ref.clone();
            model.commit(&change, &name, revision, &session, "server");
        }
        model.set_revision(revision);
        self.told
            .push((drawing_id.to_owned(), op_id.to_owned(), revision, changes));
        Some(revision)
    }
}

/// An editor and the room it is in.
struct Seat {
    app: OpenCADStudio,
    drawing: &'static str,
    /// How much of what the desk told it has heard.
    heard: usize,
}

impl Seat {
    fn new(session: &str, drawing: &'static str) -> Self {
        Self {
            app: editor(session),
            drawing,
            heard: 0,
        }
    }
}

/// Every seat hears what its room has been told since it last listened.
fn listen(desk: &Desk, seats: &mut [&mut Seat]) {
    for seat in seats.iter_mut() {
        let news: Vec<_> = desk.told[seat.heard..]
            .iter()
            .filter(|(drawing, ..)| drawing == seat.drawing)
            .cloned()
            .collect();
        seat.heard = desk.told.len();
        for (_, op_id, revision, changes) in news {
            seat.app.collab.apply(CollabEvent::OpApplied {
                op_id,
                revision,
                changes,
                user_id: None,
                display_name: None,
            });
            seat.app.apply_remote_changes();
        }
        assert_eq!(seat.app.collab.diverged, None, "{} could not follow", seat.drawing);
        assert_eq!(seat.app.collab.last_apply_error, None);
    }
}

/// Everything the seat at `from` has waiting goes to the server, one op at
/// a time, and every seat hears what comes of it. Returns the code of the
/// first refusal, when there is one.
fn send(desk: &mut Desk, seats: &mut [&mut Seat], from: usize) -> Option<String> {
    loop {
        let (op_id, base, payload) = seats[from]
            .app
            .collab
            .outbox
            .iter()
            .find(|entry| entry.sent)
            .map(|entry| (entry.op_id.clone(), entry.base_revision, entry.payload.clone()))?;
        let session = seats[from].app.collab.session_id.clone().expect("a session");
        let drawing = seats[from].drawing;
        match desk.take(drawing, &session, &op_id, base, &payload) {
            Ok(()) => listen(desk, seats),
            Err(code) => {
                let seat = &mut seats[from];
                seat.app.collab.apply(CollabEvent::OpRefused {
                    op_id,
                    code: code.clone(),
                    message: "refused".to_owned(),
                    rejected: true,
                });
                seat.app.apply_remote_changes();
                return Some(code);
            }
        }
    }
}

/// Every connector of the drawing in front: its kind, its line, where it
/// stands, what its record and its entity say of the other end. Sorted.
fn connectors(app: &OpenCADStudio) -> Vec<String> {
    let document = &app.tabs[app.active_tab].scene.document;
    let mut table = TopologyTable::load(document);
    table.prune(document);
    let mut rows: Vec<String> = table
        .connectors
        .iter()
        .filter_map(|record| {
            let handle = parse_handle(&record.handle)?;
            let entity = document.get_entity(handle)?;
            let EntityType::Circle(circle) = entity else {
                return None;
            };
            let author = PidAuthor::load(document, handle)?;
            Some(format!(
                "{} on {} at {:.1},{:.1} type={} role={} to={:?} up={:?} down={:?} peer={:?}",
                record.kind,
                record.line_tag,
                circle.center.x,
                circle.center.y,
                author.type_code,
                author.topology,
                record.peer_drawing,
                record.upstream,
                record.downstream,
                author.connector_peer
            ))
        })
        .collect();
    rows.sort();
    rows
}

/// How many connector markers the drawing has, whatever the table says.
fn markers(app: &OpenCADStudio) -> usize {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .filter(|entity| {
            matches!(entity, EntityType::Circle(_))
                && PidAuthor::from_entity(entity).is_some_and(|author| author.topology.starts_with("connector_"))
        })
        .count()
}

/// What every entity that carries `tag` — or hangs under one that does —
/// says for `field`, by its role in the pipe.
fn field_by_role(app: &OpenCADStudio, field: &str) -> BTreeMap<String, String> {
    let document = &app.tabs[app.active_tab].scene.document;
    document
        .entities()
        .filter_map(|entity| {
            let author = PidAuthor::load(document, entity.common().handle)?;
            let value = split_props(&author.props).0.get(field).cloned()?;
            (!author.topology.starts_with("connector_")).then_some((author.topology, value))
        })
        .collect()
}

fn last_said(app: &OpenCADStudio) -> Value {
    app.collab.sent.last().expect("an op was queued").clone()
}

/// What a connector made says: the line, the kind, the place, the drawing
/// it leads to, and what the line carries of the fields that go along it.
/// Taken back, it is said to be gone; put on again, made again.
#[test]
fn a_connector_made_is_said_with_its_line_its_place_and_where_it_leads() {
    let mut a = editor("s-a");
    assert_eq!(draw(&mut a, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(run(&mut a, "PIDSETPROP L-1001 管道等级 A1A")["ok"], true);
    let before = a.collab.sent.len();

    let made = draw(&mut a, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]]);
    assert_eq!(made["status"], "completed", "{made}");
    assert_eq!(a.collab.sent.len(), before + 1, "the connector was not said");
    let said = last_said(&a);
    assert_eq!(said["kind"], KIND_CREATE_CONNECTOR, "{said}");
    assert_eq!(
        (&said["line_tag"], &said["connector"], &said["at"], &said["target_drawing"]),
        (&json!("L-1001"), &json!("pipe_out"), &json!([120.0, 0.0]), &json!(SECOND)),
        "{said}"
    );
    assert_eq!(said["line_props"], json!({GRADE: "A1A"}), "what the line carries: {said}");
    assert!(said.get("changes").is_none(), "one statement: {said}");

    undo(&mut a);
    let gone = last_said(&a);
    assert_eq!(gone["kind"], KIND_DELETE_CONNECTOR, "{gone}");
    assert_eq!(
        (&gone["line_tag"], &gone["connector"], &gone["at"]),
        (&json!("L-1001"), &json!("pipe_out"), &json!([120.0, 0.0])),
        "{gone}"
    );
    redo(&mut a);
    assert_eq!(last_said(&a)["kind"], KIND_CREATE_CONNECTOR);
    assert_eq!(last_said(&a)["target_drawing"], SECOND);

    // An inflow says no drawing unless it is given one.
    let inflow = draw(&mut a, "PIDCONNECTOR pipe_in", &[[20.0, 0.0]]);
    assert_eq!(inflow["status"], "completed", "{inflow}");
    assert!(last_said(&a).get("target_drawing").is_none(), "{}", last_said(&a));
}

/// The three statements survive the wire, and what a client cannot act on
/// is not read as something else.
#[test]
fn the_connector_statements_survive_the_wire() {
    let said = |kind: &str, connector: ConnectorSaid| broadcast::Change {
        kind: kind.to_owned(),
        object_ref: "L-1001".to_owned(),
        handles: Vec::new(),
        field: String::new(),
        value: String::new(),
        old: None,
        placed: None,
        pipe: Some(Pipe::Connector(connector)),
        tag_source: None,
        overrides: None,
    };
    let made = said(
        KIND_CREATE_CONNECTOR,
        ConnectorSaid {
            connector: "pipe_out".into(),
            at: Some([120.0, 0.0]),
            target_drawing: Some(SECOND.into()),
            line_props: BTreeMap::from([(GRADE.to_owned(), "A1A".to_owned())]),
            peer: None,
        },
    );
    let gone = said(
        KIND_DELETE_CONNECTOR,
        ConnectorSaid {
            connector: "signal_in".into(),
            at: Some([5.0, 5.0]),
            ..ConnectorSaid::default()
        },
    );
    let paired = said(
        KIND_PAIR_CONNECTOR,
        ConnectorSaid {
            connector: "pipe_in".into(),
            at: None,
            peer: Some((FIRST.into(), "L-1001".into(), "pipe_out".into())),
            ..ConnectorSaid::default()
        },
    );
    let unpaired = said(
        KIND_PAIR_CONNECTOR,
        ConnectorSaid {
            connector: "pipe_in".into(),
            at: Some([20.0, 0.0]),
            ..ConnectorSaid::default()
        },
    );
    for change in [made, gone, paired, unpaired] {
        let wire = change.to_json();
        assert_eq!(broadcast::Change::from_json(&wire), Some(change), "{wire}");
    }
    // The server names the line under `object_ref` as well; that is read
    // the same.
    let rewritten = json!({"kind": KIND_CREATE_CONNECTOR, "object_ref": "L-1001", "line_tag": "L-1001",
        "connector": "pipe_out", "at": [120.0, 0.0], "field": null, "value": null});
    assert!(broadcast::Change::from_json(&rewritten).is_some());
    for broken in [
        json!({"kind": KIND_CREATE_CONNECTOR, "line_tag": "L-1001", "connector": "pipe_sideways", "at": [0.0, 0.0]}),
        json!({"kind": KIND_CREATE_CONNECTOR, "line_tag": "L-1001", "connector": "pipe_out"}),
        json!({"kind": KIND_DELETE_CONNECTOR, "connector": "pipe_out", "at": [0.0, 0.0]}),
    ] {
        assert_eq!(broadcast::Change::from_json(&broken), None, "{broken}");
    }
}

/// The fields the server carries along a pipe connector are the ones the
/// editor hands down a pipe line.
#[test]
fn the_fields_that_go_along_a_pipe_connector_are_the_lines_own() {
    assert_eq!(ocs_collab::link::PIPE_FIELDS, crate::pid::propagate::LINE_FIELDS);
}

/// Two on one drawing: what one makes of connectors, takes back and puts
/// on again, the other's drawing has — the marker, what it is, and the
/// table's record of it. None of it is a step of the second person's.
#[test]
fn a_peer_on_the_same_drawing_follows_a_connector_made_and_taken_away() {
    let (mut a, mut b) = (Seat::new("s-a", FIRST), Seat::new("s-b", FIRST));
    let mut desk = Desk::default();
    assert_eq!(draw(&mut a.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    let steps = b.app.tabs[b.app.active_tab].history.undo_stack.len();

    assert_eq!(draw(&mut a.app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    assert_eq!(connectors(&b.app), connectors(&a.app));
    assert_eq!(connectors(&b.app).len(), 1);
    assert!(
        connectors(&b.app)[0].starts_with("pipe_out on L-1001 at 120.0,0.0 type=PIPE_LINE role=connector_out"),
        "{:?}",
        connectors(&b.app)
    );
    assert!(connectors(&b.app)[0].contains(r#"to=Some("PID-1102")"#), "{:?}", connectors(&b.app));
    assert_eq!(b.app.tabs[b.app.active_tab].history.undo_stack.len(), steps);
    assert!(b.app.collab.outbox.is_empty(), "nothing of it goes back to the room");

    undo(&mut a.app);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    assert_eq!((connectors(&b.app).len(), markers(&b.app)), (0, 0), "taken away here too");
    redo(&mut a.app);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    assert_eq!(connectors(&b.app), connectors(&a.app));
    assert_eq!(markers(&b.app), 1, "made once");

    // Said twice, it is still one connector.
    let again = desk.told.last().expect("an op").clone();
    b.app.collab.apply(CollabEvent::OpApplied {
        op_id: "op-again".to_owned(),
        revision: again.2 + 1,
        changes: again.3,
        user_id: None,
        display_name: None,
    });
    b.app.apply_remote_changes();
    assert_eq!(markers(&b.app), 1);
}

/// A step of one's own taken back puts the topology table back as it was
/// when the step was made — without the record of a connector a peer has
/// made since. The connector stays, its record is entered again, and no
/// second marker is put on top of it.
#[test]
fn taking_back_ones_own_step_keeps_a_peers_connector_and_its_record() {
    let (mut a, mut b) = (Seat::new("s-a", FIRST), Seat::new("s-b", FIRST));
    let mut desk = Desk::default();
    assert_eq!(draw(&mut a.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    // b's own step, which changes the drawing's structure.
    assert_eq!(draw(&mut b.app, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 1), None);
    // a's connector, made after it.
    assert_eq!(draw(&mut a.app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    assert_eq!(connectors(&b.app).len(), 1);

    undo(&mut b.app);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 1), None);
    assert_eq!(markers(&b.app), 1, "the peer's connector is still there, once");
    assert_eq!(connectors(&b.app), connectors(&a.app), "and so is its record");
    redo(&mut b.app);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 1), None);
    assert_eq!(markers(&b.app), 1);
    assert_eq!(connectors(&b.app), connectors(&a.app));
}

/// `VAL-CROSS-009`: u1 on PID-1101 makes an outflow connector on L-1001
/// towards PID-1102; u2 on PID-1102 makes the inflow on its L-1001. The
/// server pairs them and each drawing learns of its other end. What u1's
/// line carried by then is on u2's line, and what u1 writes on it
/// afterwards arrives there too — on the line, and down to its run and the
/// run's segment. A second inflow on u2's line is refused where it is made.
#[test]
fn two_drawings_are_paired_by_the_server_and_the_lines_field_arrives_on_the_other() {
    let (mut u1, mut u2) = (Seat::new("s-1", FIRST), Seat::new("s-2", SECOND));
    let mut desk = Desk::default();
    assert_eq!(draw(&mut u1.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(run(&mut u1.app, "PIDSETPROP L-1001 公称直径 DN50")["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    assert_eq!(draw(&mut u2.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(&mut u2.app, "PIDPIPE run", &LINE)["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 1), None);
    assert!(field_by_role(&u2.app, BORE).is_empty(), "the two drawings are apart so far");

    assert_eq!(
        draw(&mut u1.app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]])["status"],
        "completed"
    );
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    assert!(connectors(&u1.app)[0].contains("peer=\"\""), "not paired yet: {:?}", connectors(&u1.app));
    assert_eq!(draw(&mut u2.app, "PIDCONNECTOR pipe_in", &[[20.0, 0.0]])["status"], "completed");
    let steps = (
        u1.app.tabs[u1.app.active_tab].history.undo_stack.len(),
        u2.app.tabs[u2.app.active_tab].history.undo_stack.len(),
    );
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 1), None);

    // Each drawing has its own end, and knows where the other is.
    let (out, inn) = (connectors(&u1.app), connectors(&u2.app));
    assert_eq!((out.len(), inn.len()), (1, 1));
    assert!(out[0].starts_with("pipe_out on L-1001 at 120.0,0.0"), "{out:?}");
    assert!(
        out[0].contains(r#"to=Some("PID-1102") up=Some("L-1001") down=Some("L-1001") peer="PID-1102:L-1001/pipe_in""#),
        "{out:?}"
    );
    assert!(inn[0].starts_with("pipe_in on L-1001 at 20.0,0.0"), "{inn:?}");
    assert!(
        inn[0].contains(r#"to=Some("PID-1101") up=Some("L-1001") down=Some("L-1001") peer="PID-1101:L-1001/pipe_out""#),
        "{inn:?}"
    );
    // Told by the server, as its own ops: nobody's step.
    assert_eq!(
        (
            u1.app.tabs[u1.app.active_tab].history.undo_stack.len(),
            u2.app.tabs[u2.app.active_tab].history.undo_stack.len(),
        ),
        steps
    );
    let pairings: Vec<&str> = desk
        .told
        .iter()
        .filter(|(_, op_id, ..)| op_id.starts_with("pair-"))
        .map(|(drawing, ..)| drawing.as_str())
        .collect();
    assert_eq!(pairings.len(), 2, "{pairings:?}");
    assert!(pairings.contains(&FIRST) && pairings.contains(&SECOND));

    // What the outflow line carried when they were paired.
    let bore = field_by_role(&u2.app, BORE);
    assert_eq!(bore.get("pipe_line").map(String::as_str), Some("DN50"), "{bore:?}");
    assert_eq!(bore.get("pipe_run").map(String::as_str), Some("DN50"), "handed down: {bore:?}");
    assert_eq!(bore.get("pipe_segment").map(String::as_str), Some("DN50"), "{bore:?}");

    // What is written on it afterwards.
    assert_eq!(run(&mut u1.app, "PIDSETPROP L-1001 管道等级 A1A")["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    let grade = field_by_role(&u2.app, GRADE);
    assert_eq!(grade.get("pipe_line").map(String::as_str), Some("A1A"), "{grade:?}");
    assert_eq!(grade.get("pipe_run").map(String::as_str), Some("A1A"), "{grade:?}");
    assert_eq!(grade.get("pipe_segment").map(String::as_str), Some("A1A"), "{grade:?}");
    assert_eq!(desk.value(SECOND, "L-1001", GRADE).as_deref(), Some("A1A"));
    let (drawing, op_id, _, changes) = desk.told.last().expect("an op");
    assert_eq!(drawing, SECOND);
    assert!(op_id.starts_with("carry-PID-1101-"), "{op_id}");
    assert_eq!(changes[0]["kind"], "set_property");
    assert!(u2.app.collab.outbox.is_empty(), "what arrived is not said back");
    assert_eq!(u2.app.collab.base_revision, desk.models[SECOND].revision());

    // A second inflow on the same line: refused before anything is made.
    let before = (markers(&u2.app), u2.app.collab.sent.len());
    let refused = draw(&mut u2.app, "PIDCONNECTOR pipe_in", &[[60.0, 0.0]]);
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "DUPLICATE_INFLOW_CONNECTOR", "{refused}");
    assert_eq!((markers(&u2.app), u2.app.collab.sent.len()), before);
}

/// The outflow taken back leaves the inflow on the other drawing unpaired;
/// put on again, they are paired again.
#[test]
fn a_connector_taken_back_unpairs_the_other_drawings_end() {
    let (mut u1, mut u2) = (Seat::new("s-1", FIRST), Seat::new("s-2", SECOND));
    let mut desk = Desk::default();
    assert_eq!(draw(&mut u1.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(&mut u2.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(draw(&mut u2.app, "PIDCONNECTOR pipe_in", &[[20.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 1), None);
    assert_eq!(draw(&mut u1.app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    assert!(connectors(&u2.app)[0].contains("peer=\"PID-1101:L-1001/pipe_out\""), "{:?}", connectors(&u2.app));

    undo(&mut u1.app);
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    let alone = connectors(&u2.app);
    assert!(alone[0].contains("down=None peer=\"\""), "{alone:?}");
    assert_eq!(run(&mut u1.app, "PIDSETPROP L-1001 管道等级 A1A")["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    assert!(field_by_role(&u2.app, GRADE).is_empty(), "nothing is carried any more");

    // (The field write cleared what could be put on again; the connector
    // is made anew.)
    assert_eq!(draw(&mut u1.app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut u1, &mut u2], 0), None);
    assert!(connectors(&u2.app)[0].contains("peer=\"PID-1101:L-1001/pipe_out\""), "{:?}", connectors(&u2.app));
    assert_eq!(
        field_by_role(&u2.app, GRADE).get("pipe_line").map(String::as_str),
        Some("A1A"),
        "and what the line carries now comes with the pairing"
    );
}

/// Two on one drawing each make an inflow on the same line at the same
/// moment: each passes its own editor's check, the server takes the first
/// and refuses the second, and the second is taken back where it was made.
#[test]
fn the_second_of_two_inflows_made_at_once_is_refused_by_the_server_and_taken_back() {
    let (mut a, mut b) = (Seat::new("s-a", SECOND), Seat::new("s-b", SECOND));
    let mut desk = Desk::default();
    assert_eq!(draw(&mut a.app, "PIDPIPE", &LINE)["ok"], true);
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);

    assert_eq!(draw(&mut a.app, "PIDCONNECTOR pipe_in", &[[20.0, 0.0]])["status"], "completed");
    assert_eq!(draw(&mut b.app, "PIDCONNECTOR pipe_in", &[[60.0, 0.0]])["status"], "completed");
    assert_eq!(send(&mut desk, &mut [&mut a, &mut b], 0), None);
    assert_eq!(markers(&b.app), 2, "its own and the first one's, for the moment");
    assert_eq!(
        send(&mut desk, &mut [&mut a, &mut b], 1).as_deref(),
        Some("DUPLICATE_INFLOW_CONNECTOR")
    );
    assert_eq!(markers(&b.app), 1, "its own is taken back");
    assert_eq!(connectors(&b.app), connectors(&a.app));
    assert_eq!(b.app.collab.diverged, None);
}

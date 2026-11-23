//! One process, one editing session per open drawing (#102).
//!
//! Opening a second drawing does not reuse the first drawing's session:
//! the roster and the claims stay apart, a connector drawn on each goes
//! out on that drawing's session, and closing one drawing ends only that
//! session.

use crate::app::collab::CollabEvent;
use crate::app::{Message, OpenCADStudio};
use serde_json::{json, Value};

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn app() -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.backup_on_save = false;
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.user_id = "u1".to_owned();
    app
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert("request_id".into(), json!(format!("per-drawing-{serial}")));
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

fn connected(session: &str, peer: &str, claim: (&str, &str)) -> CollabEvent {
    CollabEvent::Connected {
        session_id: session.to_owned(),
        base_revision: 0,
        repo_id: "r-test".into(),
        peers: vec![super::PeerInfo {
            session_id: format!("{session}-peer"),
            user_id: peer.to_owned(),
            display_name: peer.to_owned(),
            disconnected: false,
            hold_until: None,
        }],
        claims: vec![(claim.0.to_owned(), claim.1.to_owned())],
    }
}

fn kinds(state: &super::State) -> Vec<String> {
    state
        .sent
        .iter()
        .filter_map(|payload| payload["kind"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn two_open_drawings_keep_their_own_sessions_and_closing_one_leaves_the_other() {
    let mut app = app();
    let first = app.active_tab;
    app.begin_drawing_session(first, "PID-1101");
    app.apply_drawing_event(first, connected("s-a", "甲", ("P-1001", "u-a")));

    let _ = app.update(Message::TabNew);
    let second = app.active_tab;
    assert_ne!(first, second);
    app.begin_drawing_session(second, "PID-1102");
    app.apply_drawing_event(second, connected("s-b", "乙", ("V-1001", "u-b")));

    let a = app.drawing_session(first).expect("session A");
    let b = app.drawing_session(second).expect("session B");
    assert_eq!(a.drawing_id, "PID-1101");
    assert_eq!(b.drawing_id, "PID-1102");
    assert_eq!(a.session_id.as_deref(), Some("s-a"));
    assert_eq!(b.session_id.as_deref(), Some("s-b"));
    assert!(a.claims.contains_key("P-1001"));
    assert!(!a.claims.contains_key("V-1001"));
    assert!(b.claims.contains_key("V-1001"));
    assert!(!b.claims.contains_key("P-1001"));
    assert_eq!(a.peers.len(), 1);
    assert_eq!(a.peers[0].user_id, "甲");
    assert_eq!(b.peers[0].user_id, "乙");

    app.active_tab = first;
    draw(&mut app, "PIDPIPE", &[[0.0, 0.0], [200.0, 0.0]]);
    draw(&mut app, "PIDCONNECTOR pipe_out PID-1102", &[[120.0, 0.0]]);
    app.active_tab = second;
    draw(&mut app, "PIDPIPE", &[[0.0, 0.0], [200.0, 0.0]]);
    draw(&mut app, "PIDCONNECTOR pipe_in", &[[20.0, 0.0]]);

    let a_kinds = kinds(app.drawing_session(first).unwrap());
    let b_kinds = kinds(app.drawing_session(second).unwrap());
    assert!(
        a_kinds.iter().any(|kind| kind == "create_connector"),
        "A's session says its connector: {a_kinds:?}"
    );
    assert!(
        b_kinds.iter().any(|kind| kind == "create_connector"),
        "B's session says its connector: {b_kinds:?}"
    );
    assert!(
        app.drawing_session(first).unwrap().sent.iter().any(|payload| {
            payload["kind"] == "create_connector" && payload["target_drawing"] == "PID-1102"
        }),
        "the outflow names B's drawing"
    );
    assert!(
        app.drawing_session(second)
            .unwrap()
            .sent
            .iter()
            .all(|payload| payload["target_drawing"] != "PID-1101" || payload["kind"] != "create_connector"),
        "B's connector is the inflow on B"
    );

    let closing = app.tabs[second].id;
    app.end_collab_if_drawing_closed(closing);
    assert!(app.drawing_session(second).is_none(), "B's session ended");
    let a = app.drawing_session(first).expect("A stays");
    assert_eq!(a.session_id.as_deref(), Some("s-a"));
    assert!(a.connected);
    assert!(a.claims.contains_key("P-1001"));
}

//! The tags the session's drawing carries, as the UI tells them to the
//! transport for the `hello` (`hello.existing_tags`; T2.4 of
//! `docs/plans/2026-11-16-pid-platform-next-steps-plan.md`).
//!
//! The server knows a drawing by the ops it applied. A drawing opened from
//! a file has objects no op ever told of, and as far as the server could
//! see their tags were free: a symbol a peer placed could be given one of
//! them. The `hello` lists them now, and these tests hold the UI's half of
//! that to its word — when the question is answered and when it waits,
//! which drawing is listed, what is left out of the listing — with the
//! server's own judgment (`ocs_collab::conflict::DrawingModel`, the code
//! the server runs) for what the listing then does in the room. The
//! transport's half, the dial that waits for the answer and the `hello`
//! that carries it, is tested where it lives (`transport`).

use crate::app::collab::{CollabEvent, TagsSaid};
use crate::app::{Message, OpenCADStudio};
use crate::pid::persist::PidAuthor;
use document::entities::Insert;
use document::types::Vector3;
use document::{EntityType, Handle};
use ocs_collab::conflict::{Change as Judged, DrawingModel, Verdict};
use serde_json::{json, Value};
use std::collections::BTreeSet;

static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A vessel under `tag` on the drawing of tab `tab`.
fn put(app: &mut OpenCADStudio, tab: usize, tag: &str) -> Handle {
    let document = &mut app.tabs[tab].scene.document;
    let handle = document
        .add_entity(EntityType::Insert(Insert::new(
            "PID_SYM_equipment_vessel",
            Vector3::ZERO,
        )))
        .expect("entity");
    PidAuthor {
        type_code: "EQUIPMENT".to_owned(),
        tag: tag.to_owned(),
        tag_source: "auto".to_owned(),
        tag_status: "ok".to_owned(),
        props: "{}".to_owned(),
        ..PidAuthor::default()
    }
    .store(document, handle);
    handle
}

/// An editor with one drawing open and in front, a vessel on it under each
/// of `tags`. Started on no file, and in no session yet.
fn editor(tags: &[&str]) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    let tab = app.active_tab;
    for tag in tags {
        put(&mut app, tab, tag);
    }
    app.collab.user_id = "u1".to_owned();
    app
}

/// Another drawing in a tab of its own, in front, a vessel on it under each
/// of `tags`. Returns the tab.
fn another_drawing(app: &mut OpenCADStudio, tags: &[&str]) -> usize {
    let id = app.push_test_document();
    let tab = app
        .tabs
        .iter()
        .position(|tab| tab.id == id)
        .expect("the new tab");
    for tag in tags {
        put(app, tab, tag);
    }
    app.active_tab = tab;
    tab
}

/// Every tag an entity of the drawing of tab `tab` carries now, each once,
/// in order.
fn tags_on(app: &OpenCADStudio, tab: usize) -> Vec<String> {
    let document = &app.tabs[tab].scene.document;
    let tags: BTreeSet<String> = document
        .entities()
        .filter_map(|entity| PidAuthor::load(document, entity.common().handle))
        .map(|author| author.tag)
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.into_iter().collect()
}

/// One event from the socket, taken the way the app takes it.
fn hear(app: &mut OpenCADStudio, event: CollabEvent) {
    let _ = app.update(Message::CollabEvent(event));
}

/// The transport asks which tags the drawing carries, under number `ask`.
fn ask(app: &mut OpenCADStudio, ask: u64) {
    hear(app, CollabEvent::TagsWanted { ask });
}

/// Some other message goes by.
fn tick(app: &mut OpenCADStudio) {
    let _ = app.update(Message::Tick(iced::time::Instant::now()));
}

fn connected(session: &str, revision: i64) -> CollabEvent {
    CollabEvent::Connected {
        session_id: session.to_owned(),
        base_revision: revision,
        peers: Vec::new(),
        claims: Vec::new(),
        repo_id: "r-test".into(),
    }
}

fn dropped() -> CollabEvent {
    CollabEvent::Disconnected {
        reason: "connection reset".to_owned(),
        will_retry: true,
    }
}

/// The room's first op: a peer gives what goes by `P-1001` the tag `P-1009`.
fn a_peers_retag() -> CollabEvent {
    CollabEvent::OpApplied {
        op_id: "op-peer-1".to_owned(),
        revision: 1,
        changes: vec![json!({
            "kind": "set_tag", "object_ref": "P-1001", "field": "tag", "value": "P-1009",
        })],
        user_id: None,
        display_name: None,
    }
}

fn tags(list: &[&str]) -> TagsSaid {
    TagsSaid::Tags(list.iter().map(|tag| (*tag).to_owned()).collect(), None)
}

/// The tags said to question `ask`.
fn listed(app: &OpenCADStudio, ask: u64) -> Vec<String> {
    app.collab
        .tags_said
        .iter()
        .rev()
        .find_map(|(answered, said)| match said {
            TagsSaid::Tags(tags, _) if *answered == ask => Some(tags.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "no tags were said to question {ask}: {:?}",
                app.collab.tags_said
            )
        })
}

/// A file a launch can name, in a directory of this test's own.
fn file_named_by_the_launch(test: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("ocs-collab-tags-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("PID-1101.dwg");
    std::fs::write(&file, b"").expect("the file the launch named");
    (dir, file)
}

fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    if let Some(object) = req.as_object_mut() {
        object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        object.insert("request_id".into(), json!(format!("collab-tags-{serial}")));
    }
    let (response, task) = app.control_request(req);
    app.drive_headless_task(task)
        .expect("a task that needs no window");
    response
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
    assert_eq!(done["ok"], true, "{cmd}: {done}");
    app.finish_all_pending_history();
}

/// A valve a peer placed under `tag`, as its op tells of it.
fn peers_valve(tag: &str) -> Value {
    json!({
        "kind": "insert_symbol", "object_ref": tag, "tag": tag, "type": "VALVE",
        "block": "PID_SYM_valve_gate_valve", "position": [100.0, 200.0, 0.0], "rotation": 0.0,
        "scale": 1.0, "ports": "in,out", "props": {"公称直径": "DN50"}, "tag_source": "auto",
        // The sender's own handle for it: nothing of this drawing's.
        "handle": "FFF0",
    })
}

/// A symbol somebody places under `tag`, as the server reads it off the op.
fn placement(tag: &str) -> Judged {
    Judged::list(&json!({"kind": "insert_symbol", "type": "VALVE", "tag": tag, "position": [0, 0]}))
        .remove(0)
}

// ── Which drawing, and when ────────────────────────────────────────────────

/// The session's drawing is there: its tags are said at once, each once and
/// in order — and those of no other drawing, whichever is in front.
#[test]
fn the_tags_of_the_sessions_drawing_are_said_and_no_other_drawings() {
    let mut app = editor(&["V-1001", "P-1001", "P-1001"]);
    let session = app.active_tab;
    app.collab.document = Some(app.tabs[session].id);
    another_drawing(&mut app, &["P-9001"]);

    ask(&mut app, 1);
    assert_eq!(app.collab.tags_said, [(1, tags(&["P-1001", "V-1001"]))]);
    assert_eq!(app.collab.tags_wanted, None, "answered");
    tick(&mut app);
    assert_eq!(app.collab.tags_said.len(), 1, "said once");
}

/// Started on no file, the session has no drawing until it is first used —
/// and the question is not a use: it does not settle the session on
/// whatever is in front. The transport is told not to wait, once. When the
/// session comes by its drawing — here a peer's change arrives — the tags
/// follow under the same number, as the drawing carried them before that
/// change was written: what the room itself told is no news to it.
#[test]
fn a_session_with_no_drawing_yet_says_so_once_and_the_tags_follow() {
    let mut app = editor(&["P-1001", "V-1001"]);
    ask(&mut app, 3);
    assert_eq!(app.collab.tags_said, [(3, TagsSaid::NotYet)]);
    assert_eq!(
        app.collab.document, None,
        "the question settled the session on a drawing"
    );
    tick(&mut app);
    tick(&mut app);
    assert_eq!(app.collab.tags_said.len(), 1, "said once");

    hear(&mut app, connected("s-1", 0));
    hear(&mut app, a_peers_retag());
    let tab = app.active_tab;
    assert_eq!(app.collab.document, Some(app.tabs[tab].id));
    assert_eq!(
        app.collab.tags_said,
        [(3, TagsSaid::NotYet), (3, tags(&["P-1001", "V-1001"]))]
    );
    assert_eq!(app.collab.tags_wanted, None, "answered");
    assert_eq!(
        tags_on(&app, tab),
        ["P-1009", "V-1001"],
        "the change itself was written all the same"
    );
}

/// A blank drawing has nothing to tell. The symbol a peer placed on it is
/// the room's own, and the answer — given before it was written — lists
/// nothing: the transport joins again for tags it could not say with the
/// `hello`, and for none it stays where it is. Asked again later, the
/// symbol is a tag of the drawing like any other.
#[test]
fn a_blank_drawing_that_takes_a_peers_symbol_has_nothing_of_its_own_to_tell() {
    let mut app = editor(&[]);
    ask(&mut app, 4);
    hear(&mut app, connected("s-1", 0));
    hear(
        &mut app,
        CollabEvent::OpApplied {
            op_id: "op-peer-1".to_owned(),
            revision: 1,
            changes: vec![peers_valve("V-1001")],
            user_id: None,
            display_name: None,
        },
    );
    assert_eq!(app.collab.last_apply_error, None);
    assert_eq!(
        tags_on(&app, app.active_tab),
        ["V-1001"],
        "the peer's symbol was not placed"
    );
    assert_eq!(
        app.collab.tags_said,
        [(4, TagsSaid::NotYet), (4, TagsSaid::Tags(Vec::new(), None))]
    );

    ask(&mut app, 5);
    assert_eq!(listed(&app, 5), ["V-1001"]);
}

/// Each dial asks anew, and the latest question is the one that stands:
/// what follows goes back under its number. Here the session comes by its
/// drawing through a step of this person's own — which the room has not
/// answered yet, so the object it retagged is listed as the room knows it.
#[test]
fn a_new_question_takes_the_place_of_the_one_before() {
    let mut app = editor(&["P-1001"]);
    ask(&mut app, 3);
    ask(&mut app, 4);
    assert_eq!(
        app.collab.tags_said,
        [(3, TagsSaid::NotYet), (4, TagsSaid::NotYet)]
    );

    hear(&mut app, connected("s-1", 0));
    let _ = app.run_command_line("PIDTAG P-1001 P-1005");
    app.finish_all_pending_history();
    assert_eq!(app.collab.sent.len(), 1, "the retag was not said");
    tick(&mut app);
    assert_eq!(tags_on(&app, app.active_tab), ["P-1005"]);
    assert_eq!(
        app.collab.tags_said.last(),
        Some(&(4, tags(&["P-1001"]))),
        "{:?}",
        app.collab.tags_said
    );
}

/// Started on a file, the session is for that file: until its drawing is
/// there nothing is said — the dial waits — whatever else is open in front.
/// Once it is there, its tags are said.
#[test]
fn a_session_started_on_a_file_answers_once_that_file_has_a_drawing() {
    let (dir, file) = file_named_by_the_launch("loading");
    let mut app = editor(&["P-9001"]);
    let other = app.active_tab;
    app.collab.document_path = Some(file.clone());

    ask(&mut app, 5);
    tick(&mut app);
    assert!(
        app.collab.tags_said.is_empty(),
        "said before the file had a drawing: {:?}",
        app.collab.tags_said
    );
    assert_eq!(
        app.collab.document, None,
        "the tab in front is not the session's"
    );

    // The file finishes loading into a tab of its own, behind.
    let loaded = another_drawing(&mut app, &["P-1001", "V-1001"]);
    app.tabs[loaded].current_path = Some(file);
    app.active_tab = other;
    tick(&mut app);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(app.collab.tags_said, [(5, tags(&["P-1001", "V-1001"]))]);
    assert_eq!(app.collab.document, Some(app.tabs[loaded].id));
}

/// A file the launch could not open brings no drawing. While the launch is
/// still at what it was started on, the question waits; once it is through
/// — every drawing opened or given up on — the transport is told not to
/// wait any longer.
#[test]
fn a_file_that_did_not_open_does_not_hold_the_hello() {
    let missing = std::env::temp_dir()
        .join(format!("ocs-collab-tags-missing-{}", std::process::id()))
        .join("PID-1101.dwg");
    let mut app = editor(&["P-9001"]);
    app.collab.document_path = Some(missing.clone());

    ask(&mut app, 6);
    tick(&mut app);
    assert!(
        app.collab.tags_said.is_empty(),
        "the launch has not tried the file yet: {:?}",
        app.collab.tags_said
    );

    // The launch tries the file, the way it tries every file it names.
    let _ = app.update(Message::OpenExternal(missing));
    assert!(app.collab.opens_settled, "the launch is through");
    assert_eq!(app.collab.tags_said, [(6, TagsSaid::NotYet)]);
    tick(&mut app);
    assert_eq!(app.collab.tags_said.len(), 1, "said once");
    assert_eq!(app.collab.document, None);
}

/// A session whose drawing was closed has no drawing to speak for, and will
/// not come by another: it lists nothing, and the dial does not wait.
#[test]
fn a_session_whose_drawing_was_closed_lists_nothing() {
    let mut app = editor(&["P-1001"]);
    let session = app.tabs[app.active_tab].id;
    app.collab.document = Some(session);
    another_drawing(&mut app, &["P-9001"]);
    let tab = app
        .tabs
        .iter()
        .position(|tab| tab.id == session)
        .expect("the session's tab");
    // (Saved as far as the tab is concerned, so the close is not held up by
    // the question about unsaved changes.)
    app.tabs[tab].dirty = false;
    let _ = app.update(Message::TabClose(session));
    assert!(
        app.tabs.iter().all(|tab| tab.id != session),
        "the session's drawing is closed"
    );

    ask(&mut app, 7);
    assert_eq!(
        app.collab.tags_said,
        [(7, TagsSaid::Tags(Vec::new(), None))]
    );
}

// ── What the room has not heard of yet ──────────────────────────────────────

/// A step still open has changed the drawing and said nothing of it yet:
/// the answer waits for it to end. By then what it did is a step the room
/// has not answered, and the listing is of the drawing without it.
#[test]
fn the_answer_waits_for_a_step_that_is_still_open() {
    let mut app = editor(&[]);
    let tab = app.active_tab;
    let pump = put(&mut app, tab, "P-1001");
    app.collab.document = Some(app.tabs[tab].id);
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, dropped());
    app.push_undo_snapshot(tab, "TEST");
    {
        let document = &mut app.tabs[tab].scene.document;
        let mut record = PidAuthor::load(document, pump).expect("author");
        record.tag = "P-1005".to_owned();
        record.store(document, pump);
    }

    app.collab.apply(CollabEvent::TagsWanted { ask: 8 });
    app.answer_collab_tags();
    assert!(
        app.collab.tags_said.is_empty(),
        "said with a step open: {:?}",
        app.collab.tags_said
    );
    assert!(app.collab.outbox.is_empty(), "the step is still open");

    // Any message ends the step, and the question is gone back to.
    tick(&mut app);
    assert_eq!(app.collab.outbox.len(), 1, "the step waits for a session");
    assert_eq!(tags_on(&app, tab), ["P-1005"]);
    assert_eq!(app.collab.tags_said, [(8, tags(&["P-1001"]))]);
}

/// What the room said while the session's drawing was not there waits for
/// it. A question that finds those changes still waiting is answered once
/// they have been written: the listing is of the drawing as the revision
/// the `hello` names left it.
#[test]
fn a_question_is_answered_after_the_rooms_changes_that_were_waiting_are_written() {
    let (dir, file) = file_named_by_the_launch("waiting");
    let mut app = editor(&["P-9001"]);
    let other = app.active_tab;
    app.collab.document_path = Some(file.clone());
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, a_peers_retag());
    assert_eq!(app.collab.pending_remote.len(), 1, "it waits for the file");
    hear(&mut app, dropped());

    // The file's drawing arrives, and with the next message the question
    // for the next `hello`.
    let loaded = another_drawing(&mut app, &["P-1001"]);
    app.tabs[loaded].current_path = Some(file);
    app.active_tab = other;
    ask(&mut app, 11);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(app.collab.pending_remote.is_empty());
    assert_eq!(app.collab.tags_said, [(11, tags(&["P-1009"]))]);
}

/// The drawing can arrive with a message that is none of the session's, and
/// no frame need follow: frames are drawn while something on screen moves.
/// The room's changes that were waiting for the drawing are written then
/// and there, and the `hello` that is held for the answer has it at once —
/// the tags as those changes left them.
#[test]
fn a_drawing_that_arrives_takes_what_waited_for_it_and_is_listed_at_once() {
    let (dir, file) = file_named_by_the_launch("arriving");
    let mut app = editor(&["P-9001"]);
    let other = app.active_tab;
    app.collab.document_path = Some(file.clone());
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, a_peers_retag());
    hear(&mut app, dropped());
    // The next dial asks while the file is still loading.
    ask(&mut app, 13);
    assert!(
        app.collab.tags_said.is_empty(),
        "said before the file had a drawing: {:?}",
        app.collab.tags_said
    );
    assert_eq!(app.collab.pending_remote.len(), 1, "it waits for the file");

    // The file's drawing arrives, behind. What goes by next is no frame,
    // and nothing from the room.
    let loaded = another_drawing(&mut app, &["P-1001"]);
    app.tabs[loaded].current_path = Some(file);
    app.active_tab = other;
    let _ = app.update(Message::Noop);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        app.collab.pending_remote.is_empty(),
        "the room's change is still waiting"
    );
    assert_eq!(tags_on(&app, loaded), ["P-1009"]);
    assert_eq!(app.collab.tags_said, [(13, tags(&["P-1009"]))]);
}

/// So for a question that was put off: the launch got no drawing out of
/// the file, the session went ahead, and the drawing came later. The
/// question is answered as the drawing arrives — before the room's changes
/// that were waiting are written to it, with the tags that are the
/// drawing's own — and they are written with no frame to wait for.
#[test]
fn a_drawing_that_arrives_late_answers_the_question_that_was_put_off() {
    let (dir, file) = file_named_by_the_launch("late");
    let mut app = editor(&["P-9001"]);
    let other = app.active_tab;
    app.collab.document_path = Some(file.clone());
    // The launch is through, and the file gave it no drawing.
    app.collab.opens_settled = true;
    ask(&mut app, 14);
    assert_eq!(app.collab.tags_said, [(14, TagsSaid::NotYet)]);
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, a_peers_retag());
    assert_eq!(app.collab.pending_remote.len(), 1, "it waits for the file");
    assert_eq!(app.collab.tags_said.len(), 1, "said once");

    let loaded = another_drawing(&mut app, &["P-1001", "E-1001"]);
    app.tabs[loaded].current_path = Some(file);
    app.active_tab = other;
    let _ = app.update(Message::Noop);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        app.collab.tags_said,
        [(14, TagsSaid::NotYet), (14, tags(&["E-1001", "P-1001"]))]
    );
    assert_eq!(app.collab.tags_wanted, None, "answered");
    assert!(
        app.collab.pending_remote.is_empty(),
        "the room's change is still waiting"
    );
    assert_eq!(tags_on(&app, loaded), ["E-1001", "P-1009"]);
}

/// What this person did while the session was away the room has not heard
/// of yet. A symbol they placed is not listed — its tag would be taken by
/// the time the step arrives, and the server would give their own symbol
/// another — and an object they retagged is listed by the tag the room
/// knows it by.
#[test]
fn steps_the_room_has_not_answered_are_not_told_ahead_of_themselves() {
    // (Vessels, under tags a valve is not numbered into.)
    let mut app = editor(&["P-1001", "E-1001"]);
    let tab = app.active_tab;
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, dropped());

    let placed = app.automation_op(r#"{"op":"run","cmd":"PIDINSERT VALVE 100,200"}"#);
    assert_eq!(placed["ok"], true, "{placed}");
    let _ = app.run_command_line("PIDTAG P-1001 P-1005");
    app.finish_all_pending_history();
    assert_eq!(app.collab.outbox.len(), 2, "both steps wait for a session");
    let now = tags_on(&app, tab);
    assert_eq!(now.len(), 3, "{now:?}");
    assert!(now.contains(&"P-1005".to_owned()), "{now:?}");

    ask(&mut app, 9);
    assert_eq!(app.collab.tags_said, [(9, tags(&["E-1001", "P-1001"]))]);
}

// ── What the listing does in the room ───────────────────────────────────────

/// What the listing is for (`VAL-CROSS-010`, the half no op tells of): this
/// drawing's own symbols came with the file, and the server — told of their
/// tags with the `hello` — gives a symbol a peer places under one of them
/// another tag. It used to take the peer's symbol under the tag this
/// drawing already had.
#[test]
fn a_peers_symbol_placed_under_a_listed_tag_is_given_another() {
    let mut app = editor(&["P-1001", "V-1001"]);
    app.collab.document = Some(app.tabs[app.active_tab].id);
    ask(&mut app, 1);
    let told = listed(&app, 1);

    // The server, as it takes the `hello` of a client joining a room that
    // nobody has written to.
    let mut room = DrawingModel::default();
    let unheard = room.completed(&placement("P-1001"));
    assert!(
        unheard.raw.get("reassigned_from").is_none(),
        "with nothing listed the tag is free"
    );
    room.report_tags(&told, 0);
    let placed = room.completed(&placement("P-1001"));
    assert_eq!(placed.raw["reassigned_from"], "P-1001");
    assert_eq!(placed.subject(), "P-1002");
}

/// A symbol placed while the session was away goes out when it is back,
/// behind the `hello`. Its tag is not in that `hello`'s listing, so the
/// server takes it under the tag it was placed with. Listed, the server
/// would have found the tag taken — by this very symbol — and given it
/// another. Once the room has answered, it is listed like any other.
#[test]
fn a_symbol_placed_while_away_keeps_its_tag_when_the_session_is_back() {
    // (Vessels, under tags a valve is not numbered into.)
    let mut app = editor(&["P-1001", "E-1001"]);
    let tab = app.active_tab;
    app.collab.document = Some(app.tabs[tab].id);
    // The first session: the room is told what the drawing carries.
    let mut room = DrawingModel::default();
    ask(&mut app, 1);
    room.report_tags(listed(&app, 1), 0);
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, dropped());

    let placed = app.automation_op(r#"{"op":"run","cmd":"PIDINSERT VALVE 100,200"}"#);
    assert_eq!(placed["ok"], true, "{placed}");
    let valve = tags_on(&app, tab)
        .into_iter()
        .find(|tag| tag != "P-1001" && tag != "E-1001")
        .expect("the valve's tag");

    // Back: the `hello` of the next session, then the step that waited.
    ask(&mut app, 2);
    let told = listed(&app, 2);
    assert_eq!(
        told,
        ["E-1001", "P-1001"],
        "the valve is not the room's yet"
    );
    room.report_tags(&told, 0);
    let waiting = app
        .collab
        .outbox
        .front()
        .expect("the step that waited")
        .clone();
    let sent = Judged::list(&waiting.payload).remove(0);
    let taken = room.completed(&sent);
    assert!(
        taken.raw.get("reassigned_from").is_none(),
        "the server gave this client's own symbol another tag: {}",
        taken.raw
    );
    assert_eq!(taken.subject(), valve);

    // Had the listing been everything the drawing carries now:
    let mut told_too_much = DrawingModel::default();
    told_too_much.report_tags(tags_on(&app, tab), 0);
    assert_eq!(
        told_too_much.completed(&sent).raw["reassigned_from"],
        valve.as_str()
    );

    // The session is up again and the room answers the step.
    hear(&mut app, CollabEvent::Resuming { from: 0 });
    hear(&mut app, connected("s-2", 0));
    hear(
        &mut app,
        CollabEvent::OpApplied {
            op_id: waiting.op_id.clone(),
            revision: 1,
            changes: vec![taken.broadcast_form(&valve)],
            user_id: None,
            display_name: None,
        },
    );
    assert!(app.collab.outbox.is_empty(), "the step is still unanswered");
    ask(&mut app, 3);
    let mut all = vec!["P-1001".to_owned(), "E-1001".to_owned(), valve];
    all.sort();
    assert_eq!(listed(&app, 3), all);
}

/// A pipe line drawn while away, with a run and a valve fitted on it, is
/// listed with everything else the drawing carries. The server gives a
/// line, a run or a fitting no other tag than the one it is sent under, so
/// the listing costs those steps nothing — the room takes each as it was
/// sent — and keeps the tags from a peer's symbol meanwhile.
#[test]
fn a_pipe_line_drawn_while_away_is_listed_and_taken_as_it_was_sent() {
    let mut app = editor(&["P-1001"]);
    let tab = app.active_tab;
    hear(&mut app, connected("s-1", 0));
    hear(&mut app, dropped());
    let line = [[0.0, 0.0], [200.0, 0.0]];
    draw(&mut app, "PIDPIPE", &line);
    draw(&mut app, "PIDPIPE run", &line);
    draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
    assert_eq!(app.collab.outbox.len(), 3, "each step waits for a session");
    let carried = tags_on(&app, tab);
    assert!(carried.len() >= 3, "the pipe has no tags: {carried:?}");

    ask(&mut app, 12);
    assert_eq!(listed(&app, 12), carried);

    let mut room = DrawingModel::default();
    room.report_tags(&carried, 0);
    let mut revision = 0;
    for waiting in &app.collab.outbox {
        revision += 1;
        for change in Judged::list(&waiting.payload) {
            let taken = room.completed(&change);
            assert_eq!(taken.raw, change.raw, "the server changed what was sent");
            match room.judge(&taken, 0, "s-2") {
                Verdict::Apply { object_ref, .. } => {
                    room.commit(&taken, &object_ref, revision, "s-2", "u1");
                }
                other => panic!("the server refused {}: {other:?}", taken.raw),
            }
        }
    }
    // Known by their steps now, not by the listing alone.
    assert_eq!(
        room.reported_tags().collect::<Vec<_>>(),
        ["P-1001"],
        "what the steps told of is the room's own"
    );
}

// ── What the session left to tell the tags held by claim ─────────────────────

/// This person asks for `tag`, the way the panel does.
fn claim(app: &mut OpenCADStudio, tag: &str) {
    let _ = app.update(Message::CollabClaim {
        tag: tag.to_owned(),
        claim: true,
    });
}

/// This person gives `tag` back.
fn release(app: &mut OpenCADStudio, tag: &str) {
    let _ = app.update(Message::CollabClaim {
        tag: tag.to_owned(),
        claim: false,
    });
}

fn granted(tag: &str) -> CollabEvent {
    CollabEvent::ClaimGranted {
        tag: tag.to_owned(),
    }
}

/// The session ends the way the transport ends it to say the drawing's tags
/// in a `hello`, and the next dial begins: question `next` is put.
fn left_to_tell_the_tags(app: &mut OpenCADStudio, next: u64) {
    hear(
        app,
        CollabEvent::Disconnected {
            reason: "要向服务器上报本图已有的位号，正在重新加入会话".to_owned(),
            will_retry: true,
        },
    );
    hear(app, CollabEvent::Connecting { attempt: 0 });
    ask(app, next);
}

/// The claims asked of the transport so far, `(session, tag)`.
fn asked(app: &OpenCADStudio) -> Vec<(&str, &str)> {
    app.collab
        .claims_asked
        .iter()
        .map(|(session, tag)| (session.as_str(), tag.as_str()))
        .collect()
}

/// A session that came up with no drawing is left when its drawing's tags
/// are said: `hello` is the only frame that carries them. Here the session
/// comes by its drawing through a claim, so the claim goes with the session
/// that is left — the server lets go of what a session held when it ends —
/// and none of that was this person's doing. The claim is asked for again
/// in the session that follows. Once: a drop after that is a drop like any
/// other.
#[test]
fn a_claim_is_asked_for_again_in_the_session_that_follows_the_one_left_to_tell_the_tags() {
    let mut app = editor(&["P-1001", "E-1001"]);
    ask(&mut app, 21);
    hear(&mut app, connected("s-1", 0));
    assert_eq!(app.collab.tags_said, [(21, TagsSaid::NotYet)]);

    claim(&mut app, "P-1001");
    assert_eq!(asked(&app), [("s-1", "P-1001")]);
    assert_eq!(
        listed(&app, 21),
        ["E-1001", "P-1001"],
        "a claim is a use of the session: the drawing in front is its drawing"
    );
    hear(&mut app, granted("P-1001"));

    left_to_tell_the_tags(&mut app, 22);
    assert_eq!(listed(&app, 22), ["E-1001", "P-1001"]);
    assert!(
        app.collab.claims.is_empty(),
        "the claims of the session that ended are still shown"
    );
    hear(&mut app, connected("s-2", 0));
    assert_eq!(
        asked(&app),
        [("s-1", "P-1001"), ("s-2", "P-1001")],
        "the claim went with the session that was left"
    );
    hear(&mut app, granted("P-1001"));
    assert_eq!(
        app.collab.claims.get("P-1001").map(String::as_str),
        Some("u1")
    );

    hear(&mut app, dropped());
    hear(&mut app, CollabEvent::Connecting { attempt: 0 });
    ask(&mut app, 23);
    hear(&mut app, connected("s-3", 0));
    assert_eq!(
        asked(&app).len(),
        2,
        "a claim was asked for again after an ordinary drop"
    );
}

/// A session that drops takes its claims with it, as before: the server
/// lets go of them, and whose the tag is when a session is back is for the
/// room to say. Here the drawing is blank — it has no tags to tell, so the
/// session is not left for them — and the socket drops by itself.
#[test]
fn a_claim_that_went_with_a_session_that_dropped_is_not_asked_for_again() {
    let mut app = editor(&[]);
    ask(&mut app, 24);
    hear(&mut app, connected("s-1", 0));
    claim(&mut app, "P-1001");
    assert!(listed(&app, 24).is_empty(), "a blank drawing told of tags");
    hear(&mut app, granted("P-1001"));

    hear(&mut app, dropped());
    hear(&mut app, CollabEvent::Connecting { attempt: 0 });
    ask(&mut app, 25);
    hear(&mut app, connected("s-2", 0));
    assert_eq!(asked(&app), [("s-1", "P-1001")]);
}

/// The session that follows is the one the very next dial brings up. When
/// that dial fails the leave has become a drop like any other: by the time
/// a session is up again the tag may be somebody else's, and nothing is
/// asked for.
#[test]
fn a_claim_is_not_asked_for_again_when_the_dial_after_the_leave_fails() {
    let mut app = editor(&["P-1001", "E-1001"]);
    ask(&mut app, 26);
    hear(&mut app, connected("s-1", 0));
    claim(&mut app, "P-1001");
    assert_eq!(listed(&app, 26), ["E-1001", "P-1001"]);

    left_to_tell_the_tags(&mut app, 27);
    hear(
        &mut app,
        CollabEvent::Disconnected {
            reason: "connect failed".to_owned(),
            will_retry: true,
        },
    );
    hear(&mut app, CollabEvent::Connecting { attempt: 1 });
    ask(&mut app, 28);
    hear(&mut app, connected("s-2", 0));
    assert_eq!(asked(&app), [("s-1", "P-1001")]);
}

/// What is asked for again is what the session held, or was still waiting
/// to hear of, when it was left: not a claim the server refused, nor one
/// this person gave back, nor one the server had let go of.
#[test]
fn what_the_session_no_longer_held_when_it_was_left_is_not_asked_for_again() {
    let all = ["P-1001", "E-1001", "E-1002", "E-1003", "E-1004"];
    let mut app = editor(&all);
    ask(&mut app, 29);
    hear(&mut app, connected("s-1", 0));
    for tag in all {
        claim(&mut app, tag);
    }
    assert_eq!(listed(&app, 29).len(), 5, "the first claim told the tags");
    // Granted.
    hear(&mut app, granted("P-1001"));
    // Refused: somebody else has it.
    hear(
        &mut app,
        CollabEvent::ClaimDenied {
            tag: "E-1001".to_owned(),
            holder: "s-9".to_owned(),
            expires_at: 99,
        },
    );
    // Granted, and given back.
    hear(&mut app, granted("E-1002"));
    release(&mut app, "E-1002");
    // Granted, and let go of by the server.
    hear(&mut app, granted("E-1003"));
    hear(
        &mut app,
        CollabEvent::ClaimReleased {
            tag: "E-1003".to_owned(),
            reason: "expired".to_owned(),
        },
    );
    // (`E-1004`: not answered yet.)

    left_to_tell_the_tags(&mut app, 30);
    hear(&mut app, connected("s-2", 0));
    assert_eq!(
        asked(&app)[all.len()..],
        [("s-2", "E-1004"), ("s-2", "P-1001")]
    );
}

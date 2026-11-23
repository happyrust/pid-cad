//! Leaving a drawing whose steps the room has not taken (`OFF-10`;
//! `SPEC-collab-offline-sync.md` §5.6).
//!
//! * Under its banner the panel says how many steps are still to go —
//!   「离线，n 步待同步」, 「已连接，n 步待同步」 — or, for a drawing apart
//!   from the room, why, and how many of its changes never reached it:
//!   「不一致：<原因>；未同步 n 条」. The numbers are the status
//!   interface's. With nothing to go, it says nothing.
//! * Quitting, or closing the session's drawing, with steps still queued
//!   asks once — after the questions about block edits and unsaved changes
//!   — whether to go: 退出 (关闭, for the drawing) goes, 取消 stays. Closing
//!   another drawing, or anything with nothing queued, asks nothing; a
//!   script is told on the command line, and goes on.
//! * With no collaboration configured, the panel and the way out are what
//!   they were.
//!
//! Driven the way the app is: the quit and the closes through the messages
//! the window and the tabs send, the answers through the prompt's buttons,
//! the transport's question and the server's frames taken as the app takes
//! them.

use crate::app::collab::messages::{self, Frame};
use crate::app::collab::{CollabEvent, Leaving, TagsSaid};
use crate::app::{Message, ModalKind, OpenCADStudio};
use crate::pid::collab_state::{self, Diverged, Unsynced, REFUSED, REMOTE_UNAPPLIED};
use crate::pid::persist::PidAuthor;
use crate::pid::{FileCollabState, QueuedOp};
use document::entities::{Insert, Line};
use document::types::Vector3;
use document::EntityType;
use serde_json::{json, Value};

const DRAWING: &str = "PID-1101";
const SYMBOL: &str = "PID_SYM_equipment_vessel";
const PUMP: &str = "P-1001";
const PRESSURE: &str = "设计压力";
const SERVER: &str = "ws://127.0.0.1:3240/ws";
/// What the drawing files of these tests say of the room they were in, and
/// how far.
const FILE_REPO: &str = "r-file";
const FILE_REVISION: i64 = 42;

fn record() -> FileCollabState {
    FileCollabState {
        drawing_id: DRAWING.to_owned(),
        repo_id: FILE_REPO.to_owned(),
        revision: FILE_REVISION,
    }
}

/// `steps` steps of this person's the file's queue brought.
fn file_queue(steps: usize) -> Vec<QueuedOp> {
    (1..=steps)
        .map(|n| QueuedOp {
            op_id: format!("op-file-{n}"),
            payload: json!({
                "kind": "set_property",
                "object_ref": PUMP,
                "field": PRESSURE,
                "value": format!("1.{n}MPa"),
            }),
            base_revision: FILE_REVISION,
            on: true,
            label: "改设计压力".to_owned(),
            user_id: Some("u1".to_owned()),
            queued_at: Some(1_794_900_000),
            inverse: None,
        })
        .collect()
}

/// The file's unsynced list: two steps the room refused.
fn file_list() -> Vec<Value> {
    ["op-refused-1", "op-refused-2"]
        .iter()
        .map(|op_id| {
            let entry = Unsynced {
                state: REFUSED.to_owned(),
                op_id: Some((*op_id).to_owned()),
                label: "PIDSETPROP".to_owned(),
                payload: json!({
                    "kind": "set_property",
                    "object_ref": PUMP,
                    "field": PRESSURE,
                    "value": "1.6MPa",
                }),
                user_id: Some("u1".to_owned()),
                at: 1_794_903_600,
                code: Some("CLAIM_HELD".to_owned()),
                message: "位号 P-1001 由 u2 占用".to_owned(),
                rolled_back: Some(true),
            };
            serde_json::to_value(entry).expect("an entry")
        })
        .collect()
}

/// What a drawing says that the room's change could not be written into.
fn apart() -> Diverged {
    Diverged {
        code: REMOTE_UNAPPLIED.to_owned(),
        message: "操作 op-9：位号 P-1009 在本图里对不上一个对象".to_owned(),
        at: 1_794_900_000,
    }
}

/// The prompt's words for two steps.
const TWO_STEPS: &str =
    "这张图还有 2 步修改没有同步到服务器。已经随图保存的，下次打开并连上服务器后会接着提交。";

/// An editor with a drawing in front that has a pump on it. No
/// collaboration is configured.
fn editor() -> OpenCADStudio {
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
    app
}

/// The drawing in front as a file leaves it that was saved with the
/// record — marked, when `mark` is given — and with `queue` and `list`.
fn file_says(app: &mut OpenCADStudio, mark: Option<&Diverged>, queue: &[QueuedOp], list: &[Value]) {
    let i = app.active_tab;
    let document = &mut app.tabs[i].scene.document;
    collab_state::store_state_diverged(document, Some(&record()), mark);
    collab_state::store_queue(document, queue);
    collab_state::store_unsynced(document, list);
}

/// Collaboration configured for this run, with the session's drawing the
/// one in front, the way `PIDCOLLAB SERVER` takes it.
fn in_a_room(app: &mut OpenCADStudio) {
    app.collab.server_url = SERVER.to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = DRAWING.to_owned();
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

/// An editor whose drawing in front is the session's, with the `steps` its
/// file's queue brought read into the session and no connection up. Saved
/// as far as the tab is concerned, so no question about unsaved changes
/// comes first.
fn queued(steps: usize) -> OpenCADStudio {
    let mut app = editor();
    file_says(&mut app, None, &file_queue(steps), &[]);
    in_a_room(&mut app);
    assert!(matches!(asked(&mut app, 1), TagsSaid::Tags(_, Some(_))));
    assert_eq!(app.collab.outbox.len(), steps, "{:?}", app.collab.outbox);
    let i = app.active_tab;
    app.tabs[i].dirty = false;
    app
}

/// What the command line has said.
fn said(app: &OpenCADStudio) -> String {
    app.command_line.history_plain_text()
}

/// `GET /api/v1/collab/status`, through the control channel it is served on.
fn status(app: &mut OpenCADStudio) -> Value {
    app.control_request(json!({"protocol": 1, "op": "collab_status"}))
        .0
}

/// One frame from the server, taken the way the app takes it.
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

/// The room taking the drawing where its file says it is.
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

/// The main window's close button, as the window says it.
fn quit(app: &mut OpenCADStudio) {
    let window = *app.main_window.get_or_insert_with(iced::window::Id::unique);
    let _ = app.update(Message::WindowCloseRequested(window));
}

/// What the question about steps not synced asks before, and how many steps
/// it says, while it is up.
fn asked_about(app: &OpenCADStudio) -> Option<(Leaving, usize)> {
    let question = app.unsynced_question.as_ref()?;
    assert_eq!(
        app.active_modal,
        Some(ModalKind::CollabUnsynced),
        "the question is the dialog that is up"
    );
    Some((question.leaving, question.steps))
}

fn open(app: &OpenCADStudio, tab: u64) -> bool {
    app.tabs.iter().any(|open| open.id == tab)
}

/// A second drawing, in a tab of its own, in front; the session's stays
/// open behind it.
fn another_drawing(app: &mut OpenCADStudio) -> u64 {
    let session = app.collab.document;
    let _ = app.update(Message::TabNew);
    let other = app.tabs[app.active_tab].id;
    assert_ne!(Some(other), session);
    assert_eq!(
        app.collab.document, session,
        "the session keeps its drawing"
    );
    other
}

// ── Quitting ───────────────────────────────────────────────────────────────────

/// Steps still queued: quitting asks, once, how many there are, and quits
/// nothing meanwhile. 取消 stays in the program, with the steps queued.
#[test]
fn quitting_with_steps_queued_asks_and_cancel_stays() {
    let mut app = queued(2);
    quit(&mut app);
    assert_eq!(asked_about(&app), Some((Leaving::Quit, 2)));
    assert!(!app.exit_requested, "nothing has quit yet");

    let _ = app.update(Message::CollabUnsyncedCancel);
    assert_eq!(asked_about(&app), None);
    assert_eq!(app.active_modal, None);
    assert!(!app.exit_requested, "取消 stays in the program");
    assert!(app.pending_close.is_none());
    assert_eq!(app.collab.outbox.len(), 2, "and the steps stay queued");
}

/// 退出 quits, and is not asked again on the way out.
#[test]
fn quitting_with_steps_queued_and_saying_quit_quits() {
    let mut app = queued(2);
    quit(&mut app);
    assert_eq!(asked_about(&app), Some((Leaving::Quit, 2)));
    let _ = app.update(Message::CollabUnsyncedLeave);
    assert!(app.exit_requested);
    assert_eq!(asked_about(&app), None);
    assert_eq!(app.active_modal, None);
}

/// Nothing queued, nothing asked.
#[test]
fn quitting_with_nothing_queued_asks_nothing() {
    let mut app = queued(0);
    quit(&mut app);
    assert_eq!(asked_about(&app), None);
    assert!(app.exit_requested);
}

/// The question comes after the block editor's and the drawing's: the
/// three in the order §5.6 gives them.
#[test]
fn the_three_questions_come_block_edits_unsaved_changes_steps_not_synced() {
    let mut app = queued(2);
    let i = app.active_tab;
    app.tabs[i].scene.define_block_raw(
        "EDIT_ME",
        Vector3::ZERO,
        vec![EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::UNIT_X,
        ))],
    );
    let reference = app.tabs[i]
        .scene
        .document
        .add_entity(EntityType::Insert(Insert::new("EDIT_ME", Vector3::ZERO)))
        .expect("a reference");
    app.tabs[i].dirty = false;
    let _ = app.dispatch_command(&format!("BEDIT_BEGIN:{}", reference.value()));
    assert!(app.tabs[i].active_block_edit.is_some());
    let drawn = app.automation_op(r#"{"op":"run","cmd":"CIRCLE 1,0 2"}"#);
    assert_eq!(drawn["ok"], true, "{drawn}");
    app.finish_pending_history(i);

    quit(&mut app);
    assert_eq!(
        app.active_modal,
        Some(ModalKind::BlockEditChanges),
        "block edits first"
    );
    assert_eq!(asked_about(&app), None);
    let _ = app.update(Message::BlockEditChangesSave);
    assert_eq!(
        app.active_modal,
        Some(ModalKind::Unsaved),
        "then the drawing's unsaved changes"
    );
    assert_eq!(asked_about(&app), None);
    let _ = app.update(Message::UnsavedDialogDiscard);
    assert!(
        matches!(asked_about(&app), Some((Leaving::Quit, steps)) if steps > 0),
        "then the steps not synced: {:?}",
        app.active_modal
    );
    assert!(!app.exit_requested);
    let _ = app.update(Message::CollabUnsyncedLeave);
    assert!(app.exit_requested);
}

/// A script cannot answer a dialog: it is told on the command line, and the
/// quit goes on.
#[test]
fn a_script_is_told_on_the_command_line_and_quits() {
    let mut app = queued(2);
    app.script_running = true;
    let _ = app.dispatch_command("QUIT");
    assert_eq!(asked_about(&app), None);
    assert!(app.exit_requested);
    assert!(said(&app).contains(TWO_STEPS), "{}", said(&app));
}

// ── Closing the session's drawing ────────────────────────────────────────────────

/// Closing the session's drawing asks the same; closing another drawing
/// does not. 取消 keeps the drawing open, 关闭 closes it.
#[test]
fn closing_the_sessions_drawing_asks_and_closing_another_does_not() {
    let mut app = queued(2);
    let session = app.tabs[app.active_tab].id;
    let other = another_drawing(&mut app);

    let _ = app.update(Message::TabClose(other));
    assert_eq!(asked_about(&app), None, "another drawing closes without it");
    assert!(!open(&app, other));

    let _ = app.update(Message::TabClose(session));
    assert_eq!(
        asked_about(&app),
        Some((
            Leaving::Close {
                tab: session,
                discard: false
            },
            2
        ))
    );
    assert!(open(&app, session), "nothing is closed while it asks");
    let _ = app.update(Message::CollabUnsyncedCancel);
    assert_eq!(asked_about(&app), None);
    assert!(open(&app, session), "取消 keeps the drawing open");

    let _ = app.update(Message::TabClose(session));
    assert!(asked_about(&app).is_some(), "asked again");
    let _ = app.update(Message::CollabUnsyncedLeave);
    assert!(!open(&app, session), "关闭 closes it");
    assert_eq!(asked_about(&app), None);
    assert!(!app.exit_requested);
}

/// A drawing with unsaved changes is asked about those first; letting them
/// go asks about the steps, and 关闭 then closes it.
#[test]
fn closing_the_sessions_drawing_with_unsaved_changes_asks_about_those_first() {
    let mut app = queued(2);
    let i = app.active_tab;
    let session = app.tabs[i].id;
    app.tabs[i].dirty = true;
    let _ = app.update(Message::TabClose(session));
    assert_eq!(app.active_modal, Some(ModalKind::Unsaved));
    assert_eq!(asked_about(&app), None);

    let _ = app.update(Message::UnsavedDialogDiscard);
    assert_eq!(
        asked_about(&app),
        Some((
            Leaving::Close {
                tab: session,
                discard: true
            },
            2
        ))
    );
    assert!(open(&app, session), "nothing is discarded while it asks");
    let _ = app.update(Message::CollabUnsyncedLeave);
    assert!(!open(&app, session));
    assert_eq!(app.active_modal, None);
}

/// Closing every drawing stops at the question, as it stops at the one
/// about unsaved changes: 取消 leaves the rest open, 关闭 goes on with them.
#[test]
fn closing_every_drawing_stops_at_the_question() {
    let mut app = queued(2);
    let session = app.tabs[app.active_tab].id;
    let other = another_drawing(&mut app);

    let _ = app.update(Message::DocTabCloseAll);
    assert!(asked_about(&app).is_some());
    assert!(open(&app, session) && open(&app, other), "the rest wait");
    let _ = app.update(Message::CollabUnsyncedCancel);
    assert!(open(&app, session) && open(&app, other), "取消 stops there");

    let _ = app.update(Message::DocTabCloseAll);
    assert!(asked_about(&app).is_some());
    let _ = app.update(Message::CollabUnsyncedLeave);
    assert!(!open(&app, session) && !open(&app, other), "关闭 goes on");
}

// ── With no collaboration ───────────────────────────────────────────────────────

/// Without collaboration the way out is what it was: the question about
/// unsaved changes, then the quit; closing asks nothing more either.
#[test]
fn without_collaboration_the_way_out_is_as_it_was() {
    let mut app = editor();
    let i = app.active_tab;
    let first = app.tabs[i].id;
    app.tabs[i].dirty = true;
    let _ = app.update(Message::TabClose(first));
    assert_eq!(app.active_modal, Some(ModalKind::Unsaved));
    let _ = app.update(Message::UnsavedDialogDiscard);
    assert_eq!(asked_about(&app), None);
    assert!(!open(&app, first));

    let i = app.active_tab;
    app.tabs[i].dirty = true;
    quit(&mut app);
    assert_eq!(app.active_modal, Some(ModalKind::Unsaved));
    let _ = app.update(Message::UnsavedDialogDiscard);
    assert_eq!(asked_about(&app), None);
    assert!(app.exit_requested);
}

// ── The panel's line ─────────────────────────────────────────────────────────────

mod pid_collab {
    use super::*;
    use crate::ui::window::pid_collab::SyncLine;

    fn line(app: &mut OpenCADStudio) -> Option<SyncLine> {
        app.refresh_collab_view();
        app.pid_collab_view.sync.clone()
    }

    /// Offline: how many steps wait, which is what the status interface
    /// says is queued.
    #[test]
    fn offline_the_line_says_how_many_steps_wait() {
        let mut app = queued(2);
        let status = status(&mut app);
        assert_eq!(status["connected"], false);
        assert_eq!(status["queued_ops"], 2);
        let line = line(&mut app).expect("a line");
        assert_eq!(
            line,
            SyncLine::Waiting {
                connected: false,
                steps: 2
            }
        );
        assert_eq!(line.text(), "离线，2 步待同步");
    }

    /// Connected with steps queued: the same, connected; and once the room
    /// has taken them, nothing.
    #[test]
    fn connected_the_line_says_how_many_steps_wait_until_they_are_taken() {
        let mut app = queued(2);
        frame(&mut app, &taken_back());
        let status_now = status(&mut app);
        assert_eq!(status_now["connected"], true);
        assert_eq!(status_now["align"], "ok");
        assert_eq!(status_now["queued_ops"], 2);
        assert_eq!(
            line(&mut app).map(|line| line.text()),
            Some("已连接，2 步待同步".to_owned())
        );

        for revision in [FILE_REVISION + 1, FILE_REVISION + 2] {
            let op_id = app
                .collab
                .outbox
                .front()
                .expect("a step on its way")
                .op_id
                .clone();
            let _ = app.update(Message::CollabEvent(CollabEvent::OpApplied {
                op_id,
                revision,
                changes: Vec::new(),
            }));
        }
        assert_eq!(status(&mut app)["queued_ops"], 0);
        assert_eq!(line(&mut app), None, "nothing waits");
    }

    /// Apart from the room: why, as the status interface says it, and how
    /// many of the drawing's changes never reached the room — the queue
    /// and the unsynced list.
    #[test]
    fn apart_the_line_says_why_and_how_many_never_reached_the_room() {
        let mut app = editor();
        file_says(&mut app, Some(&apart()), &file_queue(2), &file_list());
        in_a_room(&mut app);
        let status = status(&mut app);
        assert_eq!(status["align"], "diverged");
        assert_eq!(status["align_message"], apart().message.as_str());
        let entries = status["queued_ops"].as_u64().expect("queued_ops")
            + status["unsynced_count"].as_u64().expect("unsynced_count");
        assert!(entries > 0, "{status}");
        let line = line(&mut app).expect("a line");
        assert_eq!(
            line.text(),
            format!("不一致：{}；未同步 {entries} 条", apart().message)
        );
    }

    /// With no collaboration configured the panel has no line to draw.
    #[test]
    fn without_collaboration_there_is_no_line() {
        let mut app = editor();
        assert_eq!(line(&mut app), None);
        assert!(!app.pid_collab_view.configured());
    }
}

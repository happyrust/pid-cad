//! Looking round a drawing that is only ever read is no edit (plan
//! 2026-11-20-pid-symbol-identity, SI-9; SPEC section 11, SI-7 item 3).
//!
//! A view is kept with its drawing: once the camera has moved, the next
//! tick writes it into the document and marks the drawing modified, so that
//! the view is saved with the file (`on_tick`; the rule is of 2026-05-01).
//! A `.pid` is never saved where it lies -- Save on it is Save As -- so
//! there the mark only made a drawing nobody had touched ask to be saved:
//! after a zoom, or after PIDTAG had gone to a symbol. On a tab shown from
//! such a source the view is written into the document as before, and a
//! Save As takes it along; the drawing is not marked.
//!
//! The sheet is the importer's own, put in the tab in front the way a
//! headless open puts a drawing there. Where the tab says it was opened
//! from is set by hand, as an open sets it; no file lies there and none is
//! written. What moves the view is what a user moves it with: the wheel,
//! ZOOM, and PIDTAG going to a symbol.

use std::time::Instant;

use document::EntityType;
use glam::DVec3;
use iced::mouse::ScrollDelta;

use super::pid_symbol_block_tests::opened_with;
use super::{Message, OpenCADStudio};

/// Where a tab may say its drawing was opened from: a `.pid`, which is only
/// ever read; a drawing that is saved where it lies; and nowhere, a drawing
/// not saved yet.
const READ_ONLY: Option<&str> = Some("ocs-si9-sheet.pid");
const SAVED_IN_PLACE: Option<&str> = Some("ocs-si9-sheet.dwg");
const NOT_SAVED_YET: Option<&str> = None;

/// A way of moving the view.
pub(super) type Move = fn(&mut OpenCADStudio);

/// The ways the view is moved, by name.
const MOVES: [(&str, Move); 3] = [
    ("the wheel", wheel),
    ("ZOOM EXTENTS", zoom_extents),
    ("PIDTAG going to a symbol", go_to_the_tagged_valve),
];

/// A notch of the mouse wheel over the drawing.
pub(super) fn wheel(app: &mut OpenCADStudio) {
    app.touchpad_pan = false;
    app.ctrl_down = false;
    app.zoom_wheel_reversed = false;
    let _ = app.update(Message::ViewportScroll(ScrollDelta::Pixels {
        x: 0.0,
        y: 100.0,
    }));
}

pub(super) fn zoom_extents(app: &mut OpenCADStudio) {
    let _ = app.run_command_line("ZOOM EXTENTS");
}

/// PIDTAG with the tag of the sheet's tagged valve, which selects the valve
/// and its labels and zooms to them.
fn go_to_the_tagged_valve(app: &mut OpenCADStudio) {
    let _ = app.run_command_line("PIDTAG GV-101");
    let i = app.active_tab;
    assert!(
        !app.tabs[i].scene.selected_handles_in_order().is_empty(),
        "PIDTAG found the valve"
    );
}

/// One tick of the application's clock.
pub(super) fn tick(app: &mut OpenCADStudio) {
    let _ = app.update(Message::Tick(Instant::now()));
}

/// The document's view in step with the camera and nothing modified: two
/// ticks, then the mark taken off.
fn settle(app: &mut OpenCADStudio) {
    tick(app);
    tick(app);
    let i = app.active_tab;
    app.tabs[i].dirty = false;
    assert_eq!(
        app.tabs[i].last_synced_camera_gen, app.tabs[i].scene.camera_generation,
        "the document's view is in step with the camera"
    );
}

/// The program with the importer's sheet of a tagged valve in front, the
/// tab saying it was opened from `path`, the view settled and nothing
/// modified.
fn shown_from(path: Option<&str>) -> OpenCADStudio {
    let mut app = opened_with(crate::io::pid::sheet_of_a_tagged_valve_and_its_labels());
    let i = app.active_tab;
    app.tabs[i].current_path = path.map(|name| std::env::temp_dir().join(name));
    settle(&mut app);
    app
}

/// Move the view by `by`, and let two ticks see that it moved.
pub(super) fn moved(app: &mut OpenCADStudio, by: Move) {
    let i = app.active_tab;
    let generation = app.tabs[i].scene.camera_generation;
    by(app);
    assert!(
        app.tabs[i].scene.camera_generation > generation,
        "the view moved"
    );
    tick(app);
    tick(app);
}

/// Where the camera looks, and how much of the drawing it shows.
pub(super) fn pose(app: &OpenCADStudio) -> [f64; 3] {
    let camera = app.tabs[app.active_tab].scene.camera.borrow();
    [
        camera.target.x,
        camera.target.y,
        f64::from(camera.ortho_size()),
    ]
}

/// Whether two poses are one, to a thousandth of a millimetre.
pub(super) fn same(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

/// Ask for the tab in front to be closed, as its close button asks, and say
/// whether the program asked about saving it first. A drawing that is not
/// asked about is gone; one that is stays open for the answer.
pub(super) fn asks_to_save_on_closing(app: &mut OpenCADStudio) -> bool {
    let id = app.tabs[app.active_tab].id;
    let _ = app.update(Message::TabClose(id));
    let asked = app.pending_close.is_some();
    assert_eq!(
        app.tabs.iter().any(|tab| tab.id == id),
        asked,
        "a drawing is asked about, or it is closed"
    );
    asked
}

/// A layout with one viewport on it, and the tab inside that viewport, the
/// way MSPACE leaves it: from here on the view that moves is the viewport's.
fn step_into_a_layout_viewport(app: &mut OpenCADStudio) {
    let i = app.active_tab;
    let scene = &mut app.tabs[i].scene;
    scene.document.add_layout("Sheet").expect("a layout");
    scene.set_current_layout("Sheet".into());
    let mut viewport = document::entities::Viewport::new();
    viewport.id = 2;
    viewport.turn_on();
    viewport.width = 200.0;
    viewport.height = 150.0;
    viewport.view_height = 150.0;
    let viewport = scene.add_entity(EntityType::Viewport(Box::new(viewport)));
    scene.active_viewport = Some(viewport);
}

/// On a tab shown from a `.pid`, moving the view is no edit, whichever way
/// it is moved: the drawing is not marked modified, and it closes without
/// asking to be saved.
#[test]
fn moving_the_view_of_a_read_only_source_is_no_edit() {
    for (how, by) in MOVES {
        let mut app = shown_from(READ_ONLY);
        let i = app.active_tab;
        moved(&mut app, by);
        assert!(
            !app.tabs[i].dirty,
            "{how}: looking round a `.pid` is not an edit"
        );
        assert!(
            !asks_to_save_on_closing(&mut app),
            "{how}: nothing to save, nothing to ask"
        );
    }
}

/// The view is written into the document all the same, so a Save As takes
/// it along: with the camera put elsewhere behind the document's back, the
/// view the document holds brings back the one the user had.
#[test]
fn the_document_of_a_read_only_source_still_holds_its_view() {
    for (how, by) in MOVES {
        let mut app = shown_from(READ_ONLY);
        let i = app.active_tab;
        moved(&mut app, by);
        assert_eq!(
            app.tabs[i].last_synced_camera_gen, app.tabs[i].scene.camera_generation,
            "{how}: the tick saw the move"
        );
        let seen = pose(&app);

        app.tabs[i].scene.camera.borrow_mut().fit_to_bounds_f64(
            DVec3::new(1000.0, 1000.0, 0.0),
            DVec3::new(1040.0, 1030.0, 0.0),
            800.0 / 600.0,
        );
        assert!(!same(pose(&app), seen), "{how}: the camera is elsewhere");
        app.tabs[i].scene.restore_saved_camera_view_only();
        assert!(
            same(pose(&app), seen),
            "{how}: the user saw {seen:?}, and the document gave back {:?}",
            pose(&app)
        );
    }
}

/// The rule the exception is cut out of stands: a drawing that is saved
/// where it lies, and one not saved yet, keep their view in their file.
/// Moving it marks them modified, and closing them asks.
#[test]
fn moving_the_view_of_a_drawing_that_keeps_its_view_marks_it_modified() {
    for path in [SAVED_IN_PLACE, NOT_SAVED_YET] {
        for (how, by) in MOVES {
            let mut app = shown_from(path);
            let i = app.active_tab;
            moved(&mut app, by);
            assert!(
                app.tabs[i].dirty,
                "{how}, opened from {path:?}: the view is saved with the file"
            );
            assert!(
                asks_to_save_on_closing(&mut app),
                "{how}, opened from {path:?}: there is a view to save"
            );
        }
    }
}

/// A `.pid` that has been edited stays modified whatever the view does
/// afterwards: the tick only ever sets the mark.
#[test]
fn moving_the_view_leaves_an_edited_read_only_source_modified() {
    for (how, by) in MOVES {
        let mut app = shown_from(READ_ONLY);
        let i = app.active_tab;
        let drawn = app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 100,100"}"#);
        assert_eq!(drawn["ok"], true, "{drawn}");
        assert!(app.tabs[i].dirty, "a line drawn is an edit");

        moved(&mut app, by);
        assert!(app.tabs[i].dirty, "{how}: the edit is still to save");
        assert!(
            asks_to_save_on_closing(&mut app),
            "{how}: there is an edit to save"
        );
    }
}

/// Inside a viewport of a layout the camera is the viewport's, and the tick
/// marks the drawing without writing the main view. The rule is the same
/// there: a `.pid` is not marked, a drawing saved where it lies is.
#[test]
fn moving_the_view_inside_a_layout_viewport_follows_the_same_rule() {
    for (path, modified) in [(READ_ONLY, false), (SAVED_IN_PLACE, true)] {
        let mut app = shown_from(path);
        let i = app.active_tab;
        step_into_a_layout_viewport(&mut app);
        settle(&mut app);

        moved(&mut app, zoom_extents);
        assert!(
            app.tabs[i].scene.active_viewport.is_some(),
            "still inside the viewport"
        );
        assert_eq!(app.tabs[i].dirty, modified, "opened from {path:?}");
    }
}

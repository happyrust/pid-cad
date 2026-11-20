//! A symbol of an imported `.pid` is one object to the pointer and to the
//! editing commands (plan 2026-11-19-pid-symbol-block, SB-3; SPEC §4.1 and
//! §4.3), and one object in the Properties panel (SB-6; SPEC §4.4); and
//! legend recognition leaves it alone (SB-7; SPEC §4.5). There too a symbol
//! and its labels name each other (plan 2026-11-20-pid-symbol-identity,
//! SI-3; SPEC §4.5), on a sheet of its own.
//!
//! The sheet is the importer's own
//! (`crate::io::pid::sheet_of_two_valves_a_note_and_a_pipe`), put in the
//! active tab the way a headless open puts a loaded drawing there. What the
//! tests send is what the editor is sent: pointer moves, presses and
//! releases, the ticks of the hover and snap timers, the Layer Manager's
//! bulb, Escape and command lines. The settings are the program's defaults,
//! except the running object snaps, which the snap test sets.
use std::collections::{BTreeMap, BTreeSet};

use document::entities::Insert;
use document::{Document, EntityType, Handle};
use glam::DVec3;

use super::{Message, OpenCADStudio, HOVER_DWELL_MS, SNAP_DWELL_MS, VARIES_LABEL};
use crate::scene::model::object::PropValue;

/// The window the sheet is shown in.
const VIEW: (f32, f32) = (800.0, 600.0);
/// The names the sheet's two symbols are placed under.
pub(super) const VALVE: &str = "Ball Valve";
const NOTE: &str = "Item Note";
/// Where the sheet places them: the valve twice, the label symbol once.
const FIRST: (f64, f64) = (100.0, 100.0);
const SECOND: (f64, f64) = (170.0, 100.0);
const NOTED: (f64, f64) = (170.0, 140.0);
/// What the sheet holds, as `named` names it.
const FIRST_VALVE: &str = "Ball Valve at (100.0, 100.0)";
const SECOND_VALVE: &str = "Ball Valve at (170.0, 100.0)";
const NOTE_SYMBOL: &str = "Item Note at (170.0, 140.0)";
const PIPE: &str = "line (90.0, 80.0)-(180.0, 80.0) on Process";
/// What a click or a selection box that finds nothing selects.
pub(super) const NOTHING: [&str; 0] = [];
/// The layer every symbol's name is drawn on.
pub(super) const NAMES: &str = "PID-SYMBOL-LABEL";
/// How far apart the clicks of `clicks_take` are, in millimetres. A click
/// takes what is drawn within 8 pixels of it, 2 mm at this zoom, so nothing
/// drawn slips between two of them.
const STEP: f64 = 0.5;

/// The program with the sheet open; see `opened_with`.
fn opened() -> OpenCADStudio {
    opened_with(crate::io::pid::sheet_of_two_valves_a_note_and_a_pipe())
}

/// The program with `sheet` open, looking at (40, 40)-(240, 190) of it:
/// four pixels to the millimetre.
pub(super) fn opened_with(sheet: Document) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    put_in_front(&mut app, sheet);
    let i = app.active_tab;

    app.tabs[i].scene.selection.borrow_mut().view.vp_size = VIEW;
    app.tabs[i].scene.camera.borrow_mut().fit_to_bounds_f64(
        DVec3::new(40.0, 40.0, 0.0),
        DVec3::new(240.0, 190.0, 0.0),
        VIEW.0 / VIEW.1,
    );
    // The distances the tests click and box at are sized for this zoom.
    let millimetre = in_window(&app, (101.0, 100.0)).x - in_window(&app, (100.0, 100.0)).x;
    assert!(
        (millimetre - 4.0).abs() < 1e-3,
        "a millimetre is {millimetre} pixels wide"
    );
    app
}

/// Put `sheet` in the tab in front, in place of the drawing that is there,
/// the way a headless open puts a loaded drawing there.
pub(super) fn put_in_front(app: &mut OpenCADStudio, sheet: Document) {
    let i = app.active_tab;
    // The steps of `open_drawing_headless`, from the loaded document on.
    let tab = &mut app.tabs[i];
    tab.scene.clear();
    tab.scene.document = sheet;
    tab.scene.sync_annotation_scale_from_header();
    tab.scene.bump_layout_epoch();
    tab.scene.bump_scale_epoch();
    tab.scene.bump_ucs_epoch();
    tab.scene.load_named_parameters_from_document();
    tab.scene.load_parametric_constraints_from_document();
    super::style_ops::ensure_standard_styles(&mut tab.scene.document);
    tab.adopt_active_ucs_from_header();
    app.adopt_header_sysvars(i);
    app.tabs[i].scene.rebuild_derived_caches();
    app.tabs[i].note_document_loaded(None);
    app.refresh_layer_panel();
}

pub(super) fn document(app: &OpenCADStudio) -> &Document {
    &app.tabs[app.active_tab].scene.document
}

/// The `key=value` strings of an entity's `PID_SEMANTICS` record, in the
/// order they were written.
pub(super) fn says(entity: &EntityType) -> Vec<String> {
    entity
        .common()
        .extended_data
        .get_record(crate::io::PID_SEMANTICS_XDATA_APP)
        .into_iter()
        .flat_map(|record| record.values.iter())
        .filter_map(|value| match value {
            document::xdata::XDataValue::String(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// What the entity with this handle says.
fn says_of(app: &OpenCADStudio, handle: Handle) -> Vec<String> {
    says(
        document(app)
            .get_entity(handle)
            .expect("the entity is in the document"),
    )
}

fn insert_of(entity: &EntityType) -> Option<&Insert> {
    match entity {
        EntityType::Insert(insert) => Some(insert),
        _ => None,
    }
}

fn as_insert(app: &OpenCADStudio, handle: Handle) -> &Insert {
    document(app)
        .get_entity(handle)
        .and_then(insert_of)
        .unwrap_or_else(|| panic!("{handle:?} is not a block reference"))
}

fn is_at(insert: &Insert, at: (f64, f64)) -> bool {
    (insert.insert_point.x - at.0).abs() < 1e-6 && (insert.insert_point.y - at.1).abs() < 1e-6
}

/// The block references in model space whose record names `symbol`.
fn placed<'a>(app: &'a OpenCADStudio, symbol: &str) -> Vec<&'a Insert> {
    let wanted = format!("symbol={symbol}");
    document(app)
        .model_space_entities()
        .filter(|entity| says(entity).contains(&wanted))
        .filter_map(insert_of)
        .collect()
}

/// The one block reference of `symbol` inserted at `at`.
pub(super) fn reference(app: &OpenCADStudio, symbol: &str, at: (f64, f64)) -> Handle {
    let found: Vec<Handle> = placed(app, symbol)
        .into_iter()
        .filter(|insert| is_at(insert, at))
        .map(|insert| insert.common.handle)
        .collect();
    assert_eq!(found.len(), 1, "one block reference of {symbol} at {at:?}");
    found[0]
}

/// The one loose text of the sheet that reads `lettering`.
pub(super) fn lettered(app: &OpenCADStudio, lettering: &str) -> Handle {
    let found: Vec<Handle> = document(app)
        .model_space_entities()
        .filter(|entity| matches!(entity, EntityType::Text(text) if text.value == lettering))
        .map(|entity| entity.common().handle)
        .collect();
    assert_eq!(found.len(), 1, "one text that reads {lettering:?}");
    found[0]
}

/// A point on the lettering of a loose text: a millimetre in from its left
/// end, halfway up its height. The texts of the sheet stand on their point:
/// it is the bottom left of their lettering.
fn on_lettering(app: &OpenCADStudio, text: Handle) -> (f64, f64) {
    match document(app).get_entity(text) {
        Some(EntityType::Text(text)) => (
            text.insertion_point.x + 1.0,
            text.insertion_point.y + text.height / 2.0,
        ),
        other => panic!("{text:?} is no text: {other:?}"),
    }
}

/// The members of a block's definition, each as its `Debug` text.
fn members(app: &OpenCADStudio, block: &str) -> Vec<String> {
    document(app)
        .entities_in_block(block)
        .map(|member| format!("{member:?}"))
        .collect()
}

fn in_model_space(app: &OpenCADStudio) -> BTreeSet<Handle> {
    document(app)
        .model_space_entities()
        .map(|entity| entity.common().handle)
        .collect()
}

/// Whether the layer's bulb is lit.
pub(super) fn shown(app: &OpenCADStudio, layer: &str) -> bool {
    !document(app)
        .layers
        .get(layer)
        .expect("the layer is in the table")
        .flags
        .off
}

/// Click the bulb of a layer's row in the Layer Manager.
pub(super) fn bulb(app: &mut OpenCADStudio, layer: &str) {
    let _ = app.update(Message::LayerToggleVisible(layer.to_string()));
}

/// The handles of what is selected.
fn selected(app: &OpenCADStudio) -> BTreeSet<Handle> {
    app.tabs[app.active_tab]
        .scene
        .selected_entities()
        .into_iter()
        .map(|(handle, _)| handle)
        .collect()
}

fn only(handle: Handle) -> BTreeSet<Handle> {
    BTreeSet::from([handle])
}

fn nothing() -> BTreeSet<Handle> {
    BTreeSet::new()
}

/// The objects with these handles as the tests name them, sorted: a
/// symbol's block reference by the symbol and where it is inserted,
/// anything else by what it draws.
fn named(app: &OpenCADStudio, handles: &BTreeSet<Handle>) -> Vec<String> {
    let mut names: Vec<String> = handles
        .iter()
        .map(|handle| {
            let entity = document(app)
                .get_entity(*handle)
                .expect("what is selected is in the document");
            let symbol = says(entity)
                .iter()
                .find_map(|said| said.strip_prefix("symbol=").map(str::to_string));
            match (insert_of(entity), symbol) {
                (Some(insert), Some(symbol)) => format!(
                    "{symbol} at ({:.1}, {:.1})",
                    insert.insert_point.x, insert.insert_point.y
                ),
                _ => drawn(entity),
            }
        })
        .collect();
    names.sort();
    names
}

/// Where a point of the drawing is in the window.
fn in_window(app: &OpenCADStudio, at: (f64, f64)) -> iced::Point {
    let scene = &app.tabs[app.active_tab].scene;
    let tile = scene.active_model_tile_bounds(VIEW.0, VIEW.1);
    let local = scene
        .camera
        .borrow()
        .project(
            DVec3::new(at.0, at.1, 0.0),
            iced::Rectangle {
                x: 0.0,
                y: 0.0,
                width: tile.width,
                height: tile.height,
            },
        )
        .expect("a point of the sheet is in front of the camera");
    iced::Point::new(tile.x + local.x, tile.y + local.y)
}

/// Escape with no command running: nothing is selected and no selection
/// box is under way.
fn escape(app: &mut OpenCADStudio) {
    let _ = app.update(Message::CommandEscape);
}

/// Move the pointer to a point of the drawing.
fn move_to(app: &mut OpenCADStudio, at: (f64, f64)) -> iced::Point {
    let cursor = in_window(app, at);
    // A move that took 8 ms or more makes the next one wait as long: the
    // editor answers it on a later frame, and a test has none. These tests
    // are about what a move does once it is answered.
    app.move_pass_due = None;
    let _ = app.update(Message::ViewportMove(cursor));
    cursor
}

/// Click a point of the drawing: move there, press, release.
fn click(app: &mut OpenCADStudio, at: (f64, f64)) {
    let cursor = move_to(app, at);
    // Every click here is a single click, however soon after the one before
    // and however near it: a pair of them is a double click, which opens an
    // editor rather than selects.
    app.last_vp_click_time = None;
    let _ = app.update(Message::ViewportLeftPressAt(cursor));
    let _ = app.update(Message::ViewportLeftRelease);
}

/// What one click selects when nothing was selected before it. The
/// selection is left as the click made it.
fn pick(app: &mut OpenCADStudio, at: (f64, f64)) -> BTreeSet<Handle> {
    escape(app);
    click(app, at);
    selected(app)
}

/// What one click selects, as `named` names it.
pub(super) fn click_takes(app: &mut OpenCADStudio, at: (f64, f64)) -> Vec<String> {
    let taken = pick(app, at);
    named(app, &taken)
}

/// Everything the clicks of a grid over a rectangle select, as `named`
/// names it, each click made with nothing selected before it.
fn clicks_take(app: &mut OpenCADStudio, x: (f64, f64), y: (f64, f64)) -> Vec<String> {
    let steps = |span: (f64, f64)| ((span.1 - span.0) / STEP).round() as usize;
    let mut taken = BTreeSet::new();
    for row in 0..=steps(y) {
        for column in 0..=steps(x) {
            let at = (x.0 + column as f64 * STEP, y.0 + row as f64 * STEP);
            taken.extend(pick(app, at));
        }
    }
    escape(app);
    named(app, &taken)
}

/// What a selection box selects, as `named` names it. The box is drawn by
/// a click on each of two corners with nothing selected before it: a window
/// when the second corner is to the right of the first, a crossing box when
/// it is to the left.
fn box_takes(app: &mut OpenCADStudio, first: (f64, f64), second: (f64, f64)) -> Vec<String> {
    assert_eq!(
        pick(app, first),
        nothing(),
        "the first corner is on nothing"
    );
    click(app, second);
    let taken = selected(app);
    named(app, &taken)
}

/// What the rollover highlights once the pointer has rested on a point of
/// the drawing, as `named` names it.
fn hover(app: &mut OpenCADStudio, at: (f64, f64)) -> Option<String> {
    escape(app);
    let _ = move_to(app, at);
    // The highlight is worked out once the pointer has been still for
    // `HOVER_DWELL_MS`; that wait is taken as served.
    let dwell = app
        .hover_dwell
        .as_mut()
        .expect("a move with no command running arms the rollover");
    dwell.last_move_at =
        iced::time::Instant::now() - std::time::Duration::from_millis(HOVER_DWELL_MS as u64 * 2);
    let _ = app.update(Message::HoverDwellTick);
    let lit = app.tabs[app.active_tab].scene.hover_highlight?;
    named(app, &only(lit)).pop()
}

/// The object snap offered once the pointer has rested on a point of the
/// drawing while a command asks for a point: its kind and where it is.
fn snap(app: &mut OpenCADStudio, at: (f64, f64)) -> Option<String> {
    let _ = move_to(app, at);
    // A move during a command leaves the full snap pass to the moment the
    // pointer has been still for `SNAP_DWELL_MS`; that wait is taken as
    // served.
    let dwell = app
        .snap_dwell
        .as_mut()
        .expect("a move during a command arms the snap dwell");
    dwell.last_move_at =
        iced::time::Instant::now() - std::time::Duration::from_millis(SNAP_DWELL_MS as u64 * 2);
    let _ = app.update(Message::SnapDwellTick);
    let offered = app.tabs[app.active_tab].snap_result?;
    Some(format!(
        "{:?} at ({:.1}, {:.1})",
        offered.snap_type, offered.world.x, offered.world.y
    ))
}

/// An entity as text: what it is and where, and the layer it is on.
fn drawn(entity: &EntityType) -> String {
    let shape = match entity {
        EntityType::Line(line) => format!(
            "line ({:.1}, {:.1})-({:.1}, {:.1})",
            line.start.x, line.start.y, line.end.x, line.end.y
        ),
        EntityType::Circle(circle) => format!(
            "circle ({:.1}, {:.1}) r {:.1}",
            circle.center.x, circle.center.y, circle.radius
        ),
        EntityType::Text(text) => format!(
            "text {:?} at ({:.1}, {:.1})",
            text.value, text.insertion_point.x, text.insertion_point.y
        ),
        other => format!("{other:?}"),
    };
    format!("{shape} on {}", entity.common().layer)
}

/// A loose entity as one line of text: what `drawn` says of it, and what
/// it says itself.
fn loose(entity: &EntityType) -> String {
    format!("{} says {}", drawn(entity), says(entity).join(" "))
}

/// What a row of the Properties panel reads.
fn reads(value: &PropValue) -> String {
    match value {
        PropValue::ReadOnly(text) | PropValue::EditText(text) => text.clone(),
        other => format!("{other:?}"),
    }
}

/// The rows of the Properties panel as the last click or selection box left
/// it, group after group: the field of each row and what the row reads.
fn rows(app: &OpenCADStudio) -> Vec<(&'static str, String)> {
    app.tabs[app.active_tab]
        .properties
        .sections
        .iter()
        .flat_map(|section| &section.props)
        .map(|row| (row.field, reads(&row.value)))
        .collect()
}

/// What the row of a field reads, or `None` when the panel has no such row.
fn row(app: &OpenCADStudio, field: &str) -> Option<String> {
    rows(app)
        .into_iter()
        .find(|(of, _)| *of == field)
        .map(|(_, read)| read)
}

/// The rows of these fields, each as `field = what it reads`.
fn rows_of(app: &OpenCADStudio, fields: &[&str]) -> Vec<String> {
    fields
        .iter()
        .map(|field| match row(app, field) {
            Some(read) => format!("{field} = {read}"),
            None => format!("{field}: no such row"),
        })
        .collect()
}

/// The `P&ID` group of the panel, each row as `field = what it reads`.
fn pid_group(app: &OpenCADStudio) -> Vec<String> {
    app.tabs[app.active_tab]
        .properties
        .sections
        .iter()
        .filter(|section| section.title == "P&ID")
        .flat_map(|section| &section.props)
        .map(|row| format!("{} = {}", row.field, reads(&row.value)))
        .collect()
}

/// The fields of the rows that read `*VARIES*`: the objects selected differ
/// in them.
fn varying(app: &OpenCADStudio) -> Vec<&'static str> {
    rows(app)
        .into_iter()
        .filter(|(_, read)| read.contains(VARIES_LABEL) || read.contains("Varies"))
        .map(|(field, _)| field)
        .collect()
}

/// A click on any stroke of a symbol selects the symbol's block reference
/// and nothing else; the valve beside it is another object, and so is the
/// pipe.
#[test]
fn a_click_on_a_stroke_selects_the_symbol_and_nothing_else() {
    let mut app = opened();

    assert_eq!(
        click_takes(&mut app, (85.0, 100.0)),
        [FIRST_VALVE],
        "its left stub"
    );
    assert_eq!(
        click_takes(&mut app, (100.0, 110.0)),
        [FIRST_VALVE],
        "its ring"
    );
    assert_eq!(
        click_takes(&mut app, (115.0, 100.0)),
        [FIRST_VALVE],
        "its right stub"
    );
    assert_eq!(
        click_takes(&mut app, (155.0, 100.0)),
        [SECOND_VALVE],
        "the other's stub"
    );
    assert_eq!(
        click_takes(&mut app, (170.0, 90.0)),
        [SECOND_VALVE],
        "the other's ring"
    );
    assert_eq!(click_takes(&mut app, (135.0, 80.0)), [PIPE], "the pipe");
    assert_eq!(click_takes(&mut app, (135.0, 120.0)), NOTHING, "bare paper");
}

/// The rollover highlight under a resting pointer is the symbol's block
/// reference, whichever stroke the pointer rests on.
#[test]
fn the_rollover_highlights_the_symbol_whole() {
    let mut app = opened();

    assert_eq!(
        hover(&mut app, (85.0, 100.0)).as_deref(),
        Some(FIRST_VALVE),
        "over a stub"
    );
    assert_eq!(
        hover(&mut app, (170.0, 110.0)).as_deref(),
        Some(SECOND_VALVE),
        "over a ring"
    );
    assert_eq!(hover(&mut app, (135.0, 120.0)), None, "over bare paper");
}

/// Selected by a click on its left stub, a symbol shows one grip, at its
/// insertion point: the ends and the middle of the stub are no grips.
#[test]
fn a_selected_symbol_has_one_grip_at_its_insertion_point() {
    let mut app = opened();
    let i = app.active_tab;

    let _ = pick(&mut app, (85.0, 100.0));
    let grips: Vec<String> = app.tabs[i]
        .selected_grips
        .iter()
        .map(|grip| format!("({:.1}, {:.1})", grip.world.x, grip.world.y))
        .collect();
    assert_eq!(grips, ["(100.0, 100.0)"]);
}

/// Selected, a symbol is one object in the Properties panel: the rows of
/// its block reference -- where it is inserted, its scale, its block, its
/// rotation -- beside the `P&ID` group, which gives its role, the name of
/// its symbol, its extent and its sheet layer; and no row reads `*VARIES*`.
/// Several symbols selected follow the rule of any objects selected
/// together: a row they agree on reads the value, a row they differ on
/// reads `*VARIES*`.
#[test]
fn a_selected_symbol_shows_its_reference_and_its_pid_group_in_the_panel() {
    let mut app = opened();
    let first = reference(&app, VALVE, FIRST);
    let block = as_insert(&app, first).block_name.clone();
    let of_a_valve = [
        "pid_role = symbol",
        "pid_symbol = Ball Valve",
        "pid_extent = 40.00 × 20.00 mm",
        "pid_sheet_layer = Process",
        "pid_sheet_layer_oid = 12",
    ];

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    assert_eq!(pid_group(&app), of_a_valve);
    // A reference scaled alike on its three axes has one row for its scale.
    assert_eq!(
        rows_of(&app, &["ins_x", "ins_y", "ins_z", "u_scale", "rotation"]),
        [
            "ins_x = 100.0000",
            "ins_y = 100.0000",
            "ins_z = 0.0000",
            "u_scale = 1.0000",
            "rotation = 0",
        ]
    );
    assert!(
        row(&app, "block").is_some_and(|read| read.contains(&block)),
        "the row of its block reads {:?}",
        row(&app, "block")
    );
    assert_eq!(varying(&app), NOTHING, "one symbol: no row varies");

    assert_eq!(
        box_takes(&mut app, (75.0, 85.0), (195.0, 115.0)),
        [FIRST_VALVE, SECOND_VALVE],
        "a window around the two valves"
    );
    assert_eq!(pid_group(&app), of_a_valve, "two placements of one symbol");
    assert_eq!(
        rows_of(&app, &["ins_x", "ins_y"]),
        [
            format!("ins_x = {VARIES_LABEL}"),
            "ins_y = 100.0000".to_string()
        ],
        "they are inserted side by side"
    );

    bulb(&mut app, NAMES);
    assert_eq!(
        box_takes(&mut app, (195.0, 145.0), (165.0, 105.0)),
        [SECOND_VALVE, NOTE_SYMBOL],
        "a crossing box over the second valve and the note"
    );
    assert_eq!(
        row(&app, "pid_symbol").as_deref(),
        Some(VARIES_LABEL),
        "two symbols"
    );
}

/// A symbol and its labels name each other in the Properties panel (plan
/// 2026-11-20-pid-symbol-identity, SI-3; SPEC §4.5). The sheet is
/// `crate::io::pid::sheet_of_a_tagged_valve_and_its_labels`. Selected by a
/// click, the valve the drawing file names shows what the file states of
/// its item -- the kind it is held as, its tag and that the tag is the
/// file's -- and lists its four labels; the text of one of them names the
/// valve by its tag. The valve the file gives no name is named by what it
/// is and by its symbol; the label of the run of line says it is a line's,
/// and the label of what is no graphic of the sheet that it is another
/// object's; a text that is no label's says nothing of the kind. The rows
/// that take the sheet are shown for one object alone. Once the valve is
/// erased its labels say it is not on the sheet, and an undo gives them
/// their valve back.
#[test]
fn a_symbol_and_its_labels_name_each_other_in_the_panel() {
    let mut app = opened_with(crate::io::pid::sheet_of_a_tagged_valve_and_its_labels());
    let tagged = reference(&app, VALVE, FIRST);
    let untagged = reference(&app, VALVE, SECOND);
    let group =
        |rows: &[&str]| -> Vec<String> { rows.iter().map(|row| (*row).to_string()).collect() };
    let by_the_file = crate::t!("Item in the drawing file").into_owned();
    let on_process = ["pid_sheet_layer = Process", "pid_sheet_layer_oid = 12"];
    // What the P&ID group of a text on `Process` reads, with `more` between
    // its role and its sheet layer.
    let of_a_text = |more: &[String]| -> Vec<String> {
        let mut rows = vec!["pid_role = text".to_string()];
        rows.extend_from_slice(more);
        rows.extend(group(&on_process));
        rows
    };
    // The P&ID group once a click on the lettering of that text selected
    // it and nothing else.
    let mut of_the_text = |lettering: &str| -> Vec<String> {
        let text = lettered(&app, lettering);
        let at = on_lettering(&app, text);
        assert_eq!(pick(&mut app, at), only(text), "a click on {lettering:?}");
        pid_group(&app)
    };

    assert_eq!(
        of_the_text("DN50"),
        of_a_text(&["pid_label_of = GV-101".to_string()]),
        "a label of the valve the file names"
    );
    assert_eq!(
        of_the_text("GV-101"),
        of_a_text(&["pid_label_of = GV-101".to_string()]),
        "the label filed before its valve"
    );
    assert_eq!(
        of_the_text("DN80"),
        of_a_text(&["pid_label_of = PIDPipingComponent · Ball Valve".to_string()]),
        "the label of the valve the file gives no name"
    );
    assert_eq!(
        of_the_text("P-1001-50-CS"),
        of_a_text(&[format!("pid_label_of = {}", crate::t!("A line"))]),
        "the label of the run of line"
    );
    assert_eq!(
        of_the_text("DWG-0201"),
        of_a_text(&[format!("pid_label_of = {}", crate::t!("Another object"))]),
        "the label of what is no graphic of the sheet"
    );
    assert_eq!(of_the_text("NOTE 4"), of_a_text(&[]), "no label's");

    assert_eq!(pick(&mut app, (100.0, 110.0)), only(tagged), "its ring");
    let mut of_the_tagged = group(&[
        "pid_class = PIDPipingComponent",
        "pid_item_type = PipingComp",
        "pid_role = symbol",
        "pid_symbol = Ball Valve",
        "pid_extent = 40.00 × 20.00 mm",
        "pid_label = GV-101",
    ]);
    of_the_tagged.push(format!("pid_label_source = {}", crate::t!("Drawing file")));
    of_the_tagged.push("pid_labels = 4: GV-101 · DN50 · PN16 +1".to_string());
    of_the_tagged.push(format!("pid_resolved = {by_the_file}"));
    of_the_tagged.extend(group(&on_process));
    assert_eq!(pid_group(&app), of_the_tagged);

    assert_eq!(pick(&mut app, (170.0, 110.0)), only(untagged), "its ring");
    let mut of_the_untagged = group(&[
        "pid_class = PIDPipingComponent",
        "pid_item_type = PipingComp",
        "pid_role = symbol",
        "pid_symbol = Ball Valve",
        "pid_extent = 40.00 × 20.00 mm",
        "pid_labels = 1: DN80",
    ]);
    of_the_untagged.push(format!("pid_resolved = {by_the_file}"));
    of_the_untagged.extend(group(&on_process));
    assert_eq!(pid_group(&app), of_the_untagged);

    let together = box_takes(&mut app, (70.0, 85.0), (130.0, 155.0));
    assert_eq!(
        together.len(),
        5,
        "a window around the valve and its four labels: {together:?}"
    );
    assert_eq!(
        rows_of(&app, &["pid_labels", "pid_label_of"]),
        ["pid_labels: no such row", "pid_label_of: no such row"],
        "several objects: no row takes the sheet"
    );

    assert_eq!(pick(&mut app, (100.0, 110.0)), only(tagged));
    let _ = app.run_command_line("ERASE");
    let orphaned = lettered(&app, "DN50");
    let at = on_lettering(&app, orphaned);
    assert_eq!(pick(&mut app, at), only(orphaned));
    assert_eq!(
        pid_group(&app),
        of_a_text(&[format!("pid_label_of = {}", crate::t!("Not on the sheet"))]),
        "its valve is erased"
    );

    escape(&mut app);
    app.undo_steps(1);
    assert_eq!(pick(&mut app, at), only(orphaned));
    assert_eq!(
        pid_group(&app),
        of_a_text(&["pid_label_of = GV-101".to_string()]),
        "the valve is back"
    );
}

/// The object snaps the strokes of a symbol offer as loose entities, the
/// reference offers too: the ends of a stub, and the centre of the ring
/// with the pointer near the centre or on the ring. The running snaps are
/// set here to those two kinds.
#[test]
fn the_strokes_of_a_symbol_keep_their_object_snaps() {
    use crate::snap::SnapType;

    let mut app = opened();
    let i = app.active_tab;
    app.snapper.snap_enabled = true;
    app.snapper.enabled = [SnapType::Endpoint, SnapType::Center].into_iter().collect();
    app.snapper.grid_snap_on = false;
    app.snapper.otrack_enabled = false;
    app.ortho_mode = false;
    app.polar_mode = false;
    let _ = app.run_command_line("LINE");
    assert!(app.tabs[i].active_cmd.is_some(), "LINE asks for a point");

    assert_eq!(
        snap(&mut app, (80.5, 100.5)).as_deref(),
        Some("Endpoint at (80.0, 100.0)"),
        "the outer end of the left stub"
    );
    assert_eq!(
        snap(&mut app, (110.5, 99.5)).as_deref(),
        Some("Endpoint at (110.0, 100.0)"),
        "the inner end of the right stub"
    );
    assert_eq!(
        snap(&mut app, (100.5, 100.5)).as_deref(),
        Some("Center at (100.0, 100.0)"),
        "near the centre of the ring"
    );
    assert_eq!(
        snap(&mut app, (100.0, 90.0)).as_deref(),
        Some("Center at (100.0, 100.0)"),
        "on the ring"
    );
    assert_eq!(
        snap(&mut app, (170.5, 100.5)).as_deref(),
        Some("Center at (170.0, 100.0)"),
        "the other valve's ring"
    );
    assert_eq!(snap(&mut app, (135.0, 120.0)), None, "over bare paper");
}

/// To a selection box a symbol is one object: a window takes it when it
/// holds all of it and not when it holds a part of it, a crossing box takes
/// it by touching one stroke, and a window around the sheet counts each
/// symbol once.
#[test]
fn a_selection_box_takes_the_symbol_as_one_object() {
    let mut app = opened();

    assert_eq!(
        box_takes(&mut app, (75.0, 85.0), (125.0, 115.0)),
        [FIRST_VALVE],
        "a window around the whole valve"
    );
    assert_eq!(
        box_takes(&mut app, (75.0, 85.0), (105.0, 115.0)),
        NOTHING,
        "a window around its left half"
    );
    assert_eq!(
        box_takes(&mut app, (88.0, 104.0), (84.0, 96.0)),
        [FIRST_VALVE],
        "a crossing box over one stub"
    );
    assert_eq!(
        box_takes(&mut app, (60.0, 170.0), (225.0, 60.0)),
        [FIRST_VALVE, SECOND_VALVE, PIPE],
        "a window around the sheet"
    );
}

/// `PID-SYMBOL-LABEL` ships switched off, and a name it hides is no part of
/// what is selected: a window around what the valve shows takes the valve,
/// and where its name would be there is nothing to click and nothing to
/// cross. Switched on, the name is part of the symbol: that window no
/// longer holds all of it, and a click or a crossing box on the name takes
/// the valve.
#[test]
fn a_hidden_name_is_no_part_of_what_is_selected() {
    let mut app = opened();
    assert!(!shown(&app, NAMES), "the names start hidden");

    // The valve shows from x 80 to 120; its name starts at 120.8 and is
    // written between y 99 and 101. The clicks on the name start 4 mm clear
    // of the stub's end, twice as far as a click reaches.
    let around_the_strokes = ((75.0, 85.0), (125.0, 115.0));
    let across_the_name = ((127.0, 102.0), (123.0, 98.0));
    let over_the_name = ((124.0, 140.0), (99.25, 100.75));
    assert_eq!(
        box_takes(&mut app, around_the_strokes.0, around_the_strokes.1),
        [FIRST_VALVE],
        "hidden, the name is not what the window must hold"
    );
    assert_eq!(
        box_takes(&mut app, across_the_name.0, across_the_name.1),
        NOTHING,
        "hidden, the name is not there to cross"
    );
    assert_eq!(
        clicks_take(&mut app, over_the_name.0, over_the_name.1),
        NOTHING,
        "hidden, the name is not there to click"
    );

    bulb(&mut app, NAMES);
    assert!(shown(&app, NAMES), "the bulb switched the names on");
    assert_eq!(
        box_takes(&mut app, around_the_strokes.0, around_the_strokes.1),
        NOTHING,
        "shown, the name reaches out of that window"
    );
    assert_eq!(
        box_takes(&mut app, (75.0, 85.0), (148.0, 115.0)),
        [FIRST_VALVE],
        "a window around the valve and its name"
    );
    assert_eq!(
        box_takes(&mut app, across_the_name.0, across_the_name.1),
        [FIRST_VALVE],
        "shown, crossing the name takes the valve"
    );
    assert_eq!(
        clicks_take(&mut app, over_the_name.0, over_the_name.1),
        [FIRST_VALVE],
        "shown, a click on the name takes the valve"
    );
}

/// A symbol that draws nothing but its name is on `PID-SYMBOL-LABEL`
/// itself: hidden with the names it cannot be clicked, crossed or held by a
/// window, and shown it is selected as any symbol is.
#[test]
fn a_symbol_that_is_only_its_name_is_selected_once_names_are_shown() {
    let mut app = opened();
    let note = reference(&app, NOTE, NOTED);
    assert_eq!(as_insert(&app, note).common.layer, NAMES);

    // Its name starts 2.07 right of where it is inserted and is written
    // between y 139 and 141.
    let over_the_name = ((172.5, 190.0), (139.25, 140.75));
    let across_the_name = ((195.0, 145.0), (165.0, 135.0));
    let around_the_sheet = ((60.0, 170.0), (225.0, 60.0));
    assert_eq!(
        clicks_take(&mut app, over_the_name.0, over_the_name.1),
        NOTHING,
        "hidden, it is not there to click"
    );
    assert_eq!(
        box_takes(&mut app, across_the_name.0, across_the_name.1),
        NOTHING,
        "hidden, it is not there to cross"
    );
    assert_eq!(
        box_takes(&mut app, around_the_sheet.0, around_the_sheet.1),
        [FIRST_VALVE, SECOND_VALVE, PIPE],
        "hidden, a window around the sheet leaves it out"
    );

    bulb(&mut app, NAMES);
    assert!(shown(&app, NAMES), "the bulb switched the names on");
    assert_eq!(
        clicks_take(&mut app, over_the_name.0, over_the_name.1),
        [NOTE_SYMBOL],
        "shown, a click on its name takes it"
    );
    assert_eq!(
        box_takes(&mut app, across_the_name.0, across_the_name.1),
        [NOTE_SYMBOL],
        "shown, crossing its name takes it"
    );
    assert_eq!(
        box_takes(&mut app, around_the_sheet.0, around_the_sheet.1),
        [FIRST_VALVE, SECOND_VALVE, NOTE_SYMBOL, PIPE],
        "shown, a window around the sheet takes it too"
    );
}

/// MOVE moves the reference: its insertion point is the new one, and the
/// definition, what the reference says and the valve beside it are as they
/// were.
#[test]
fn moving_a_symbol_moves_its_reference_and_leaves_the_definition() {
    let mut app = opened();
    let i = app.active_tab;
    let first = reference(&app, VALVE, FIRST);
    let second = reference(&app, VALVE, SECOND);
    let block = as_insert(&app, first).block_name.clone();
    let definition = members(&app, &block);
    let said = says_of(&app, first);
    let blocks = document(&app).block_records.len();

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.run_command_line("MOVE");
    let _ = app.feed_script_line("0,0");
    let _ = app.feed_script_line("0,40");
    assert!(app.tabs[i].active_cmd.is_none(), "MOVE has ended");

    assert!(
        is_at(as_insert(&app, first), (100.0, 140.0)),
        "its insertion point is 40 higher: {:?}",
        as_insert(&app, first).insert_point
    );
    assert!(is_at(as_insert(&app, second), SECOND), "the other stays");
    assert_eq!(members(&app, &block), definition, "the definition");
    assert_eq!(says_of(&app, first), said, "what the reference says");
    assert_eq!(document(&app).block_records.len(), blocks);
    assert_eq!(pick(&mut app, (85.0, 100.0)), nothing(), "where it was");
    assert_eq!(pick(&mut app, (85.0, 140.0)), only(first), "where it is");
    assert_eq!(pick(&mut app, (155.0, 100.0)), only(second), "the other");
}

/// COPY makes one more reference to the same definition, saying what the
/// one it was copied from says.
#[test]
fn copying_a_symbol_makes_another_reference_that_says_the_same() {
    let mut app = opened();
    let i = app.active_tab;
    let first = reference(&app, VALVE, FIRST);
    let block = as_insert(&app, first).block_name.clone();
    let definition = members(&app, &block);
    let said = says_of(&app, first);
    let blocks = document(&app).block_records.len();
    assert_eq!(placed(&app, VALVE).len(), 2);

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.run_command_line("COPY");
    let _ = app.feed_script_line("0,0");
    let _ = app.feed_script_line("0,40");
    let _ = app.feed_script_line("");
    assert!(app.tabs[i].active_cmd.is_none(), "COPY has ended");

    assert_eq!(placed(&app, VALVE).len(), 3, "one more reference");
    let copy = reference(&app, VALVE, (100.0, 140.0));
    assert_ne!(copy, first);
    assert_eq!(as_insert(&app, copy).block_name, block, "the same block");
    assert_eq!(says_of(&app, copy), said, "the same record");
    assert!(is_at(as_insert(&app, first), FIRST), "the original stays");
    assert_eq!(members(&app, &block), definition, "the definition");
    assert_eq!(document(&app).block_records.len(), blocks, "no new block");
    assert_eq!(pick(&mut app, (85.0, 140.0)), only(copy), "the copy");
    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first), "the original");
}

/// ROTATE turns the reference about the base point: a quarter turn about
/// its own insertion point leaves it inserted where it was, turned, and the
/// definition and what it says as they were.
#[test]
fn rotating_a_symbol_turns_its_reference_and_leaves_the_definition() {
    let mut app = opened();
    let i = app.active_tab;
    let first = reference(&app, VALVE, FIRST);
    let block = as_insert(&app, first).block_name.clone();
    let definition = members(&app, &block);
    let said = says_of(&app, first);
    assert_eq!(as_insert(&app, first).rotation, 0.0);

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.run_command_line("ROTATE");
    let _ = app.feed_script_line("100,100");
    let _ = app.feed_script_line("90");
    assert!(app.tabs[i].active_cmd.is_none(), "ROTATE has ended");

    let turned = as_insert(&app, first);
    assert!(is_at(turned, FIRST), "it is inserted where it was");
    assert!(
        (turned.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "it is a quarter turn round: {}",
        turned.rotation
    );
    assert_eq!(members(&app, &block), definition, "the definition");
    assert_eq!(says_of(&app, first), said, "what the reference says");
    assert_eq!(
        click_takes(&mut app, (100.0, 85.0)),
        [FIRST_VALVE],
        "its stub, now below it"
    );
    assert_eq!(
        click_takes(&mut app, (85.0, 100.0)),
        NOTHING,
        "where the stub was"
    );
}

/// MIRROR makes a mirrored reference to the same definition, saying what
/// the one it mirrors says.
#[test]
fn mirroring_a_symbol_makes_a_mirrored_reference_that_says_the_same() {
    let mut app = opened();
    let i = app.active_tab;
    let first = reference(&app, VALVE, FIRST);
    let block = as_insert(&app, first).block_name.clone();
    let definition = members(&app, &block);
    let said = says_of(&app, first);
    let blocks = document(&app).block_records.len();

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.run_command_line("MIRROR");
    // Across the horizontal line at y = 130, keeping the original.
    let _ = app.feed_script_line("0,130");
    let _ = app.feed_script_line("10,130");
    let _ = app.feed_script_line("N");
    assert!(app.tabs[i].active_cmd.is_none(), "MIRROR has ended");

    assert_eq!(placed(&app, VALVE).len(), 3, "one more reference");
    let mirrored = reference(&app, VALVE, (100.0, 160.0));
    assert_ne!(mirrored, first);
    assert_eq!(
        as_insert(&app, mirrored).block_name,
        block,
        "the same block"
    );
    assert_eq!(says_of(&app, mirrored), said, "the same record");
    assert!(is_at(as_insert(&app, first), FIRST), "the original stays");
    assert_eq!(members(&app, &block), definition, "the definition");
    assert_eq!(document(&app).block_records.len(), blocks, "no new block");
    assert_eq!(
        click_takes(&mut app, (85.0, 160.0)),
        ["Ball Valve at (100.0, 160.0)"],
        "the mirrored one"
    );
}

/// ERASE takes the reference away and one undo brings it back, saying what
/// it said.
#[test]
fn an_erased_symbol_comes_back_whole_on_undo() {
    let mut app = opened();
    let first = reference(&app, VALVE, FIRST);
    let second = reference(&app, VALVE, SECOND);
    let said = says_of(&app, first);
    let before = in_model_space(&app);

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.run_command_line("ERASE");
    let left: Vec<Handle> = placed(&app, VALVE)
        .into_iter()
        .map(|insert| insert.common.handle)
        .collect();
    assert_eq!(left, vec![second], "only the other valve is left");
    assert_eq!(pick(&mut app, (85.0, 100.0)), nothing(), "where it was");

    escape(&mut app);
    app.undo_steps(1);
    assert_eq!(
        reference(&app, VALVE, FIRST),
        first,
        "the reference is back"
    );
    assert_eq!(says_of(&app, first), said, "saying what it said");
    assert_eq!(in_model_space(&app), before, "and nothing else changed");
    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first), "and is picked");
}

/// EXPLODE turns the reference back into loose entities where the symbol
/// was drawn. The record went with the reference: each loose entity says
/// its role and nothing more.
#[test]
fn an_exploded_symbol_is_loose_entities_that_say_only_their_role() {
    let mut app = opened();
    let first = reference(&app, VALVE, FIRST);
    let second = reference(&app, VALVE, SECOND);
    let before = in_model_space(&app);

    assert_eq!(pick(&mut app, (85.0, 100.0)), only(first));
    let _ = app.dispatch_command("EXPLODE");

    let left: Vec<Handle> = placed(&app, VALVE)
        .into_iter()
        .map(|insert| insert.common.handle)
        .collect();
    assert_eq!(left, vec![second], "only the other valve is a reference");
    assert!(
        document(&app)
            .model_space_entities()
            .all(|entity| entity.common().handle != first),
        "the reference is gone"
    );
    let mut pieces: Vec<String> = document(&app)
        .model_space_entities()
        .filter(|entity| !before.contains(&entity.common().handle))
        .map(loose)
        .collect();
    pieces.sort();
    assert_eq!(
        pieces,
        [
            "circle (100.0, 100.0) r 10.0 on Process says role=symbol",
            "line (110.0, 100.0)-(120.0, 100.0) on Process says role=symbol",
            "line (80.0, 100.0)-(90.0, 100.0) on Process says role=symbol",
            "text \"Ball Valve\" at (120.8, 99.0) on PID-SYMBOL-LABEL says role=symbol-label",
        ]
    );

    // Loose, each stroke is picked by itself.
    assert_eq!(
        click_takes(&mut app, (85.0, 100.0)),
        ["line (80.0, 100.0)-(90.0, 100.0) on Process"],
        "the stub by itself"
    );
    assert_eq!(
        click_takes(&mut app, (100.0, 110.0)),
        ["circle (100.0, 100.0) r 10.0 on Process"],
        "the ring by itself"
    );
    assert_eq!(
        click_takes(&mut app, (155.0, 100.0)),
        [SECOND_VALVE],
        "the other valve is still one"
    );
}

/// What every entity that carries a `PID_SEMANTICS` record says, by handle.
/// On the sheet as opened these are the records the import wrote: on the
/// references, on the members of their blocks and on the pipe.
fn records(app: &OpenCADStudio) -> BTreeMap<Handle, Vec<String>> {
    document(app)
        .entities()
        .map(|entity| (entity.common().handle, says(entity)))
        .filter(|(_, said)| !said.is_empty())
        .collect()
}

/// What the entities of `before` say now.
fn say_now(
    app: &OpenCADStudio,
    before: &BTreeMap<Handle, Vec<String>>,
) -> BTreeMap<Handle, Vec<String>> {
    before
        .keys()
        .map(|handle| (*handle, says_of(app, *handle)))
        .collect()
}

/// Every line the command line has shown, newest last.
fn history(app: &OpenCADStudio) -> Vec<String> {
    app.command_line
        .history
        .iter()
        .map(|entry| entry.text.clone())
        .collect()
}

/// A butterfly valve the legend rules know by its block (`$VALVE$00000316`),
/// drawn on the open sheet at (50, 50) as a bow-tie of four lines, 6 x 3 mm,
/// and made a block the way BLOCK makes one. Returns its block reference.
fn a_valve_the_legend_knows(app: &mut OpenCADStudio) -> Handle {
    use document::entities::Line;
    use document::types::{Transform, Vector3};

    let i = app.active_tab;
    let strokes: Vec<Handle> = [
        ((50.0, 50.0), (50.0, 53.0)),
        ((50.0, 53.0), (56.0, 50.0)),
        ((56.0, 50.0), (56.0, 53.0)),
        ((56.0, 53.0), (50.0, 50.0)),
    ]
    .into_iter()
    .map(|((x0, y0), (x1, y1))| {
        let mut line = Line::new();
        line.start = Vector3::new(x0, y0, 0.0);
        line.end = Vector3::new(x1, y1, 0.0);
        app.tabs[i].scene.add_entity(EntityType::Line(line))
    })
    .collect();
    let identity = Transform::identity();
    app.tabs[i]
        .scene
        .create_block_from_entities(&strokes, "$VALVE$00000316", &identity, &identity)
        .expect("four lines make a block")
}

/// Legend recognition leaves the symbols of an import alone (SB-7; SPEC
/// §4.5). A valve the legend rules know is drawn beside them, so that
/// PIDLEGEND has a symbol to recognise, a record to write and a record to
/// clear. ON then recognises that valve and nothing else: no symbol of the
/// import is boxed, none is reported as an unknown block. And through
/// REPORT, ON, OFF, ON again and PURGE, every record the import wrote says
/// what it said: on the references, on the members of their blocks and on
/// the pipe.
#[test]
fn legend_recognition_leaves_the_symbols_of_an_import_alone() {
    use crate::io::pid_legend;

    let mut app = opened();
    let i = app.active_tab;
    let first = reference(&app, VALVE, FIRST);
    let imported = records(&app);
    assert_eq!(
        imported[&first],
        [
            "sheet_layer=Process",
            "sheet_layer_oid=12",
            "role=symbol",
            "extent=40.00x20.00",
            "symbol=Ball Valve",
            "graphic=701",
        ],
        "what the first valve says"
    );
    for symbol in [reference(&app, VALVE, SECOND), reference(&app, NOTE, NOTED)] {
        assert!(imported.contains_key(&symbol), "{symbol:?} says something");
    }
    let known = a_valve_the_legend_knows(&mut app);

    let _ = app.run_command_line("PIDLEGEND REPORT");
    let reported = history(&app);
    assert!(
        !reported.iter().any(|line| line.contains("UNKNOWN BLOCK")),
        "{reported:#?}"
    );
    assert_eq!(say_now(&app, &imported), imported, "after REPORT");

    let _ = app.run_command_line("PIDLEGEND ON");
    let recognition = app.tabs[i].pid_legend().expect("ON keeps what it read");
    let recognised: Vec<Handle> = recognition
        .symbols
        .iter()
        .flat_map(|symbol| symbol.handles.iter().copied())
        .collect();
    assert_eq!(recognised, [known], "the drawn valve and nothing else");
    assert!(
        recognition.unknown_blocks.is_empty(),
        "{:?}",
        recognition.unknown_blocks
    );
    assert_eq!(
        pid_legend::legend_handles(document(&app)).len(),
        2,
        "a box and a label, for the drawn valve"
    );
    assert_eq!(
        pid_legend::attached_handles(document(&app)),
        [known],
        "ON writes its record on the drawn valve alone"
    );
    assert_eq!(say_now(&app, &imported), imported, "after ON");

    let _ = app.run_command_line("PIDLEGEND OFF");
    assert!(
        pid_legend::legend_handles(document(&app)).is_empty(),
        "OFF takes the legend away"
    );
    assert_eq!(say_now(&app, &imported), imported, "after OFF");

    let _ = app.run_command_line("PIDLEGEND ON");
    assert_eq!(
        pid_legend::attached_handles(document(&app)),
        [known],
        "ON again reads the drawn valve again"
    );
    let _ = app.run_command_line("PIDLEGEND PURGE");
    assert!(
        pid_legend::legend_handles(document(&app)).is_empty(),
        "PURGE takes the legend away"
    );
    assert!(
        pid_legend::attached_handles(document(&app)).is_empty(),
        "and clears the record ON wrote"
    );
    assert_eq!(say_now(&app, &imported), imported, "after PURGE");
}

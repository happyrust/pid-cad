//! What is drawn of a symbol of an imported `.pid`, and how it follows the
//! Layer Manager (plan 2026-11-19-pid-symbol-block, SB-4; SPEC §4.3).
//!
//! The sheet is the importer's own
//! (`crate::io::pid::sheet_of_valves_on_shown_and_hidden_layers`), opened
//! the way `pid_symbol_block_tests` opens its own: a valve on each of two
//! sheet layers the file displays, one on a layer it hides, one on a hidden
//! layer it gives no name, and a pipe. What the tests send is what the
//! Layer Manager sends: the switch of a sheet layer or of a role in its
//! sheet-layer view, the bulb and the colour of a layer in its table. What
//! they read is the strokes and the text the editor draws and picks by --
//! those of one object, checked to be what a plot takes of it, or all that
//! is drawn around a place -- and the fills a plot takes, checked to be the
//! fills a click can take. The list of fills the window is handed is
//! private to the scene and is not read.
use document::types::Color;
use document::{EntityType, Handle};
use glam::DVec3;

use super::pid_symbol_block_tests::{
    bulb, click_takes, document, opened_with, reference, says, shown, NAMES, NOTHING, VALVE,
};
use super::{Message, OpenCADStudio};
use crate::io::pid_view_filter::PidViewRow;
use crate::scene::{HatchModel, WireModel};

/// Where the sheet places the valve: on two layers the file displays, on a
/// layer it hides, and on a hidden layer it gives no name.
const ON_PROCESS: (f64, f64) = (100.0, 100.0);
const ON_INSTRUMENT: (f64, f64) = (170.0, 100.0);
const ON_DEMOLITION: (f64, f64) = (100.0, 150.0);
const ON_NO_NAME: (f64, f64) = (170.0, 150.0);
/// The layer that holds what the file hides on a layer with no name.
const HIDDEN: &str = "PID-HIDDEN";

/// What is drawn of a valve, measured from where it is placed, as `valve`
/// words it. The ring has a colour of its own, the sheet's foreground, and
/// so has the stub on the right, a green. The stub on the left is drawn by
/// layer: `LEFT_STUB` is that stub on a layer of the foreground colour. The
/// name is drawn by layer too, on layers that are grey.
const RING: &str = "stroke (-10.0, -10.0)-(10.0, 10.0) in #FFFFFF";
const LEFT_STUB: &str = "stroke (-20.0, 0.0)-(-10.0, 0.0) in #FFFFFF";
const RIGHT_STUB: &str = "stroke (10.0, 0.0)-(20.0, 0.0) in #008000";
const NAME: &str = "text \"Ball Valve\" at (20.8, -1.0) in #808080";
const STROKES: [&str; 3] = [RING, LEFT_STUB, RIGHT_STUB];
const STROKES_AND_NAME: [&str; 4] = [RING, LEFT_STUB, RIGHT_STUB, NAME];
/// The pipe, as `pipe` words it.
const PIPE: [&str; 1] = ["stroke (90.0, 80.0)-(180.0, 80.0) in #FFFFFF"];
/// The fill of each valve, as `fills` words it.
const FILL_ON_PROCESS: &str = "fill around (100.0, 100.0) on Process in #FF0000";
const FILL_ON_INSTRUMENT: &str = "fill around (170.0, 100.0) on Instrument in #FF0000";
const FILL_ON_DEMOLITION: &str = "fill around (100.0, 150.0) on Demolition in #FF0000";
const FILL_ON_HIDDEN: &str = "fill around (170.0, 150.0) on PID-HIDDEN in #FF0000";

/// The program with the sheet open.
fn opened() -> OpenCADStudio {
    opened_with(crate::io::pid::sheet_of_valves_on_shown_and_hidden_layers())
}

/// The rows of one list of the Layer Manager's sheet-layer view, each as
/// its name, the number it shows and how its switch stands.
fn rows(rows: &[PidViewRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let switch = if row.on { "on" } else { "off" };
            format!("{}: {}, {switch}", row.name, row.entities)
        })
        .collect()
}

fn sheet_layer_rows(app: &OpenCADStudio) -> Vec<String> {
    rows(&app.tabs[app.active_tab].layers.pid.layers)
}

fn role_rows(app: &OpenCADStudio) -> Vec<String> {
    rows(&app.tabs[app.active_tab].layers.pid.roles)
}

/// Click the switch of a sheet layer's row in the Layer Manager's
/// sheet-layer view.
fn sheet_layer_switch(app: &mut OpenCADStudio, layer: &str) {
    let listed = &app.tabs[app.active_tab].layers.pid.layers;
    let row = listed
        .iter()
        .position(|row| row.name == layer)
        .unwrap_or_else(|| panic!("the view lists no sheet layer {layer}: {listed:?}"));
    let _ = app.update(Message::PidSheetLayerToggle(row));
}

/// Click the switch of a role's row there.
fn role_switch(app: &mut OpenCADStudio, role: &str) {
    let listed = &app.tabs[app.active_tab].layers.pid.roles;
    let row = listed
        .iter()
        .position(|row| row.name == role)
        .unwrap_or_else(|| panic!("the view lists no role {role}: {listed:?}"));
    let _ = app.update(Message::PidRoleToggle(row));
}

/// Give a layer a colour in the Layer Manager: open the swatch of its row
/// and pick the colour.
fn layer_colour(app: &mut OpenCADStudio, layer: &str, colour: Color) {
    let _ = app.update(Message::LayerColorPickerToggle(layer.to_string()));
    let _ = app.update(Message::LayerColorSet(colour));
}

/// A colour as the hex digits of its red, green and blue.
fn hex(colour: [f32; 4]) -> String {
    let byte = |part: f32| (part * 255.0).round() as u8;
    format!(
        "#{:02X}{:02X}{:02X}",
        byte(colour[0]),
        byte(colour[1]),
        byte(colour[2])
    )
}

/// A length to a tenth of a millimetre, with no negative zero.
fn tenth(length: f64) -> f64 {
    let rounded = (length * 10.0).round() / 10.0;
    if rounded == 0.0 {
        0.0
    } else {
        rounded
    }
}

/// A line for each stroke and each text of these wires, sorted, measured
/// from `origin`: a stroke as the box it spans, a text as what it reads and
/// where it starts, each with its colour.
fn wire_lines<'a>(wires: impl Iterator<Item = &'a WireModel>, origin: (f64, f64)) -> Vec<String> {
    let mut lines = Vec::new();
    for wire in wires {
        let colour = hex(wire.color);
        for run in &wire.searchable_text {
            lines.push(format!(
                "text {:?} at ({:.1}, {:.1}) in {colour}",
                run.text,
                tenth(run.origin[0] - origin.0),
                tenth(run.origin[1] - origin.1)
            ));
        }
        // The points of a wire are its strokes end to end, a point that is
        // no number between one stroke and the next.
        let mut stroke: Vec<DVec3> = Vec::new();
        for index in 0..=wire.points.len() {
            let point = (index < wire.points.len())
                .then(|| wire.point_world(index, 1.0))
                .filter(|point| point.is_finite());
            if let Some(point) = point {
                stroke.push(point);
                continue;
            }
            if stroke.len() > 1 {
                let low = stroke
                    .iter()
                    .fold(DVec3::INFINITY, |low, point| low.min(*point));
                let high = stroke
                    .iter()
                    .fold(DVec3::NEG_INFINITY, |high, point| high.max(*point));
                lines.push(format!(
                    "stroke ({:.1}, {:.1})-({:.1}, {:.1}) in {colour}",
                    tenth(low.x - origin.0),
                    tenth(low.y - origin.1),
                    tenth(high.x - origin.0),
                    tenth(high.y - origin.1)
                ));
            }
            stroke.clear();
        }
    }
    lines.sort();
    lines
}

/// What is drawn of the object with this handle, as `wire_lines` words it:
/// the wires the editor draws and picks by. A plot is checked to take the
/// same.
fn drawn_of(app: &OpenCADStudio, handle: Handle, origin: (f64, f64)) -> Vec<String> {
    let scene = &app.tabs[app.active_tab].scene;
    // What a block reference draws goes by the reference's handle.
    let name = handle.value().to_string();
    let shown = scene.entity_wires();
    let drawn = wire_lines(
        shown
            .iter()
            .filter(|wire| wire.name == name && wire.display_visible),
        origin,
    );
    let (plotted, _) = scene.plot_wire_groups(None);
    let plotted = wire_lines(
        plotted
            .iter()
            .filter(|wire| wire.name == name && wire.plot_visible),
        origin,
    );
    assert_eq!(plotted, drawn, "a plot takes what the editor draws");
    drawn
}

/// What is drawn of the valve placed at `at`, measured from there.
fn valve(app: &OpenCADStudio, at: (f64, f64)) -> Vec<String> {
    drawn_of(app, reference(app, VALVE, at), at)
}

/// What is drawn where a valve placed at `at` shows, whatever draws it, as
/// `wire_lines` words it and measured from there: from 25 to the left of
/// `at`, past the stub, to 45 to the right, past the name, and 12 above and
/// below.
fn drawn_around(app: &OpenCADStudio, at: (f64, f64)) -> Vec<String> {
    let shown = app.tabs[app.active_tab].scene.entity_wires();
    wire_lines(
        shown.iter().filter(|wire| {
            // The box a wire spans: its least x and y, then its greatest.
            let [left, low, right, high] = wire.aabb.map(f64::from);
            wire.display_visible
                && right >= at.0 - 25.0
                && left <= at.0 + 45.0
                && high >= at.1 - 12.0
                && low <= at.1 + 12.0
        }),
        at,
    )
}

/// The stub on the left of a valve on a layer of this colour.
fn left_stub_in(colour: &str) -> String {
    format!("stroke (-20.0, 0.0)-(-10.0, 0.0) in {colour}")
}

/// The strokes of a valve on a layer of this colour.
fn strokes_on(colour: &str) -> [String; 3] {
    [
        RING.to_string(),
        left_stub_in(colour),
        RIGHT_STUB.to_string(),
    ]
}

/// What is drawn of the pipe, the one loose stroke of the sheet.
fn pipe(app: &OpenCADStudio) -> Vec<String> {
    let pipes: Vec<Handle> = document(app)
        .model_space_entities()
        .filter(|entity| matches!(entity, EntityType::Line(_)))
        .filter(|entity| says(entity).iter().any(|said| said == "role=geometry"))
        .map(|entity| entity.common().handle)
        .collect();
    assert_eq!(pipes.len(), 1, "the sheet has one pipe");
    drawn_of(app, pipes[0], (0.0, 0.0))
}

/// A line for a fill on a layer: the middle of the box it spans, the layer
/// and its colour.
fn fill_line(fill: &HatchModel, layer: &str) -> String {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for vertex in fill.boundary.iter() {
        for axis in 0..2 {
            if vertex[axis].is_finite() {
                let at = fill.world_origin[axis] + f64::from(vertex[axis]);
                low[axis] = low[axis].min(at);
                high[axis] = high[axis].max(at);
            }
        }
    }
    format!(
        "fill around ({:.1}, {:.1}) on {layer} in {}",
        tenth((low[0] + high[0]) / 2.0),
        tenth((low[1] + high[1]) / 2.0),
        hex(fill.color)
    )
}

/// The fills drawn, one line each as `fill_line` words it, sorted: the
/// fills a plot takes, each on the layer it shows on. Every fill of the
/// sheet is in a symbol, and the fills a click can take, which the scene
/// keeps by the block reference that draws them, are checked to be the
/// same, each on the layer of its reference.
fn fills(app: &OpenCADStudio) -> Vec<String> {
    let scene = &app.tabs[app.active_tab].scene;
    let mut plotted: Vec<String> = scene
        .paper_plot_hatches()
        .iter()
        .map(|fill| fill_line(fill, fill.effective_layer.as_deref().unwrap_or("no layer")))
        .collect();
    plotted.sort();
    let mut clicked = Vec::new();
    for (reference, drawn) in scene.insert_hatches_for_click().iter() {
        let layer = &document(app)
            .get_entity(*reference)
            .expect("a fill is drawn by a reference of the document")
            .common()
            .layer;
        clicked.extend(drawn.iter().map(|fill| fill_line(fill, layer)));
    }
    clicked.sort();
    assert_eq!(clicked, plotted, "a click can take the fills a plot takes");
    plotted
}

/// What is drawn of the whole sheet, a line for each of its parts: each
/// valve by the layer it is on, the pipe, the fills.
fn picture(app: &OpenCADStudio) -> Vec<String> {
    let mut parts = Vec::new();
    for (layer, at) in [
        ("Process", ON_PROCESS),
        ("Instrument", ON_INSTRUMENT),
        ("Demolition", ON_DEMOLITION),
        ("no name", ON_NO_NAME),
    ] {
        parts.push(format!("valve on {layer}: {:?}", valve(app, at)));
    }
    parts.push(format!("pipe: {:?}", pipe(app)));
    parts.push(format!("fills: {:?}", fills(app)));
    parts
}

/// As the sheet opens, the valves on the two displayed layers are drawn,
/// strokes and fill; their names are not, on a layer that ships switched
/// off. Nothing is drawn of the valves on the two hidden layers.
#[test]
fn the_sheet_opens_with_the_symbols_of_its_displayed_layers_drawn() {
    let app = opened();
    assert!(!shown(&app, NAMES), "the names start hidden");
    assert_eq!(valve(&app, ON_PROCESS), STROKES);
    assert_eq!(valve(&app, ON_INSTRUMENT), STROKES);
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        STROKES,
        "the valve is all that is drawn where it is"
    );
    assert_eq!(drawn_around(&app, ON_DEMOLITION), NOTHING);
    assert_eq!(drawn_around(&app, ON_NO_NAME), NOTHING);
    assert_eq!(pipe(&app), PIPE);
    assert_eq!(fills(&app), [FILL_ON_PROCESS, FILL_ON_INSTRUMENT]);
}

/// The sheet-layer view counts a symbol once on its sheet layer, by its
/// reference, and the parts of a symbol under their roles, a definition
/// once however many references share it. Here: two objects on `Process`,
/// a valve and the pipe; under `symbol` the four references and the three
/// strokes of each of the two definitions; a name and a fill for each
/// definition. The valve on the layer with no name is on no row of the
/// sheet layers.
#[test]
fn the_layer_manager_counts_a_symbol_once_and_its_parts_by_role() {
    let app = opened();
    assert_eq!(
        sheet_layer_rows(&app),
        ["Demolition: 1, off", "Instrument: 1, on", "Process: 2, on"]
    );
    assert_eq!(
        role_rows(&app),
        [
            "geometry: 1, on",
            "symbol: 10, on",
            "symbol-label: 2, on",
            "fill: 2, on"
        ]
    );
}

/// A sheet layer switched off takes what is on it out of the picture: the
/// valve whole, its name and its fill with it, and the pipe. The valve on
/// another layer, drawn from the same definition, stays as it was. The
/// switch is not the bulb of the layer of that name, which stays lit.
/// Switched on again, the picture is the one before.
#[test]
fn a_sheet_layer_switched_off_takes_its_symbol_out_whole() {
    let mut app = opened();
    bulb(&mut app, NAMES);
    let before = picture(&app);
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        STROKES_AND_NAME,
        "the names are on"
    );

    sheet_layer_switch(&mut app, "Process");
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        NOTHING,
        "where the valve on the layer was"
    );
    assert_eq!(pipe(&app), NOTHING, "the pipe on the layer");
    assert_eq!(
        valve(&app, ON_INSTRUMENT),
        STROKES_AND_NAME,
        "the valve on another layer"
    );
    assert_eq!(fills(&app), [FILL_ON_INSTRUMENT]);
    assert_eq!(
        sheet_layer_rows(&app),
        ["Demolition: 1, off", "Instrument: 1, on", "Process: 2, off"]
    );
    assert!(shown(&app, "Process"), "the layer of that name stays lit");

    sheet_layer_switch(&mut app, "Process");
    assert_eq!(picture(&app), before, "switched on again");
}

/// The role of fills switched off takes the fill out of every symbol and
/// leaves its outline. Switched on again, the picture is the one before.
#[test]
fn the_role_of_fills_switched_off_leaves_the_outlines() {
    let mut app = opened();
    let before = picture(&app);

    role_switch(&mut app, "fill");
    assert_eq!(fills(&app), NOTHING);
    assert_eq!(valve(&app, ON_PROCESS), STROKES);
    assert_eq!(valve(&app, ON_INSTRUMENT), STROKES);
    assert_eq!(pipe(&app), PIPE);

    role_switch(&mut app, "fill");
    assert_eq!(picture(&app), before, "switched on again");
}

/// The role of symbols switched off takes every symbol out whole: the
/// reference states that role, so the name and the fill go with the
/// strokes. What is no symbol stays. Switched on again, the picture is the
/// one before.
#[test]
fn the_role_of_symbols_switched_off_takes_every_symbol_out() {
    let mut app = opened();
    bulb(&mut app, NAMES);
    let before = picture(&app);

    role_switch(&mut app, "symbol");
    for at in [ON_PROCESS, ON_INSTRUMENT, ON_DEMOLITION, ON_NO_NAME] {
        assert_eq!(drawn_around(&app, at), NOTHING, "the valve at {at:?}");
    }
    assert_eq!(fills(&app), NOTHING);
    assert_eq!(pipe(&app), PIPE);

    role_switch(&mut app, "symbol");
    assert_eq!(picture(&app), before, "switched on again");
}

/// The names are drawn once their layer is switched on, each beside its
/// symbol, and answer to their role from then on: the role switched off
/// takes them out though their layer is still on, and takes out nothing
/// else.
#[test]
fn the_names_show_with_their_layer_and_go_with_their_role() {
    let mut app = opened();
    let unnamed = picture(&app);

    bulb(&mut app, NAMES);
    assert!(shown(&app, NAMES), "the bulb switched the names on");
    assert_eq!(valve(&app, ON_PROCESS), STROKES_AND_NAME);
    assert_eq!(valve(&app, ON_INSTRUMENT), STROKES_AND_NAME);
    let named = picture(&app);

    role_switch(&mut app, "symbol-label");
    assert!(shown(&app, NAMES), "the layer of the names is still on");
    assert_eq!(picture(&app), unnamed, "the picture without the names");

    role_switch(&mut app, "symbol-label");
    assert_eq!(picture(&app), named, "switched on again");

    bulb(&mut app, NAMES);
    assert_eq!(picture(&app), unnamed, "the layer of the names off again");
}

/// The names layer switched on shows nothing of a symbol on a hidden
/// layer, whatever would draw it: a symbol's name is drawn by its
/// reference, and neither the reference whose sheet layer is switched off
/// nor the one on `PID-HIDDEN` is drawn. Neither name is there to click.
#[test]
fn the_names_layer_shows_nothing_of_a_symbol_on_a_hidden_layer() {
    let mut app = opened();
    bulb(&mut app, NAMES);
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        STROKES_AND_NAME,
        "the names are on"
    );

    assert_eq!(drawn_around(&app, ON_DEMOLITION), NOTHING);
    assert_eq!(drawn_around(&app, ON_NO_NAME), NOTHING);
    assert_eq!(fills(&app), [FILL_ON_PROCESS, FILL_ON_INSTRUMENT]);
    // A name starts 20.8 right of where its valve is placed and is written
    // from 1 below that to 1.3 above; a click reaches 2.
    assert_eq!(
        click_takes(&mut app, (125.0, 151.0)),
        NOTHING,
        "the name on the layer that is switched off"
    );
    assert_eq!(
        click_takes(&mut app, (195.0, 151.0)),
        NOTHING,
        "the name on the layer with no name"
    );
    assert_eq!(
        click_takes(&mut app, (125.0, 101.0)),
        ["Ball Valve at (100.0, 100.0)"],
        "a name that is drawn is there to click"
    );
}

/// A hidden sheet layer switched on shows its symbol: the reference is lit
/// and the layer it is on, which the file had switched off with it, is
/// switched on. The symbol on the hidden layer with no name is no part of
/// that. Switched off again the symbol goes, and the layer keeps its bulb.
#[test]
fn a_hidden_sheet_layer_switched_on_shows_its_symbol() {
    let mut app = opened();
    assert!(!shown(&app, "Demolition"), "the layer starts switched off");
    let before = picture(&app);

    sheet_layer_switch(&mut app, "Demolition");
    assert!(shown(&app, "Demolition"), "its layer is switched on");
    assert_eq!(valve(&app, ON_DEMOLITION), STROKES);
    assert_eq!(
        fills(&app),
        [FILL_ON_PROCESS, FILL_ON_DEMOLITION, FILL_ON_INSTRUMENT]
    );
    assert_eq!(
        sheet_layer_rows(&app),
        ["Demolition: 1, on", "Instrument: 1, on", "Process: 2, on"]
    );
    assert_eq!(drawn_around(&app, ON_NO_NAME), NOTHING);
    assert!(!shown(&app, HIDDEN), "the layer of what has no layer");

    sheet_layer_switch(&mut app, "Demolition");
    assert_eq!(picture(&app), before, "switched off again");
    assert!(shown(&app, "Demolition"), "the layer keeps its bulb");
}

/// The symbol on the hidden layer with no name is on `PID-HIDDEN`, every
/// part of it by way of its reference: the bulb of that layer shows it
/// whole, its name too though the names layer is off, and what it draws by
/// layer takes that layer's grey.
#[test]
fn a_symbol_on_the_hidden_layer_with_no_name_shows_with_the_layer_that_holds_it() {
    let mut app = opened();
    assert!(!shown(&app, HIDDEN), "the layer ships switched off");
    assert_eq!(drawn_around(&app, ON_NO_NAME), NOTHING);

    bulb(&mut app, HIDDEN);
    assert!(shown(&app, HIDDEN), "the bulb switched the layer on");
    assert!(!shown(&app, NAMES), "the names layer stays off");
    let [ring, left_stub, right_stub] = strokes_on("#808080");
    assert_eq!(
        valve(&app, ON_NO_NAME),
        [ring, left_stub, right_stub, NAME.to_string()]
    );
    assert_eq!(
        fills(&app),
        [FILL_ON_PROCESS, FILL_ON_INSTRUMENT, FILL_ON_HIDDEN]
    );
    assert_eq!(
        drawn_around(&app, ON_DEMOLITION),
        NOTHING,
        "the valve on the other hidden layer"
    );
}

/// A stroke drawn by layer in a symbol takes the colour of the layer its
/// reference is on: one definition is drawn in one colour on one layer and
/// in another on the next. A stroke and a fill with a colour of their own
/// keep it.
#[test]
fn a_stroke_by_layer_takes_the_colour_of_the_layer_its_symbol_is_on() {
    let mut app = opened();

    layer_colour(&mut app, "Process", Color::Index(5));
    assert_eq!(valve(&app, ON_PROCESS), strokes_on("#0000FF"));
    assert_eq!(
        valve(&app, ON_INSTRUMENT),
        STROKES,
        "on a layer that kept its colour"
    );
    assert_eq!(
        pipe(&app),
        ["stroke (90.0, 80.0)-(180.0, 80.0) in #0000FF"],
        "the pipe, loose on the layer"
    );
    assert_eq!(fills(&app), [FILL_ON_PROCESS, FILL_ON_INSTRUMENT]);

    layer_colour(&mut app, "Instrument", Color::Index(6));
    assert_eq!(valve(&app, ON_INSTRUMENT), strokes_on("#FF00FF"));
    assert_eq!(valve(&app, ON_PROCESS), strokes_on("#0000FF"));
    assert_eq!(fills(&app), [FILL_ON_PROCESS, FILL_ON_INSTRUMENT]);
}

/// A symbol moved takes its name and its fill with it: both are drawn
/// where the symbol is now, the name is there to click, and nothing is left
/// where it was.
#[test]
fn the_name_and_the_fill_move_with_their_symbol() {
    let mut app = opened();
    let i = app.active_tab;
    bulb(&mut app, NAMES);
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        STROKES_AND_NAME,
        "the names are on"
    );

    assert_eq!(
        click_takes(&mut app, (85.0, 100.0)),
        ["Ball Valve at (100.0, 100.0)"],
        "a click on its left stub selects it"
    );
    let _ = app.run_command_line("MOVE");
    let _ = app.feed_script_line("0,0");
    let _ = app.feed_script_line("0,-40");
    assert!(app.tabs[i].active_cmd.is_none(), "MOVE has ended");

    assert_eq!(
        drawn_around(&app, (100.0, 60.0)),
        STROKES_AND_NAME,
        "where it is now, measured from there"
    );
    assert_eq!(
        valve(&app, (100.0, 60.0)),
        STROKES_AND_NAME,
        "all of it drawn by the symbol"
    );
    assert_eq!(
        drawn_around(&app, ON_PROCESS),
        NOTHING,
        "nothing is left where it was"
    );
    assert_eq!(
        fills(&app),
        [
            "fill around (100.0, 60.0) on Process in #FF0000",
            FILL_ON_INSTRUMENT
        ]
    );
    assert_eq!(
        click_takes(&mut app, (125.0, 61.0)),
        ["Ball Valve at (100.0, 60.0)"],
        "the name where it is now"
    );
    assert_eq!(
        click_takes(&mut app, (125.0, 101.0)),
        NOTHING,
        "where the name was"
    );
}

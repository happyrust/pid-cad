//! The editor's canvas: the colour it is cleared to and the grid drawn on
//! it are the design's inside the module, and the user's own in the general
//! interface.

use serde_json::Value;

use super::*;
use crate::cli::GuiConfig;
use crate::ui::style::pid_tokens::{ColorToken, PidTheme};

/// The application as a launch leaves it once the startup prompts have been
/// answered: in the module, or with `--classic` and a drawing.
fn launched_with_mode(classic: bool, mode: crate::app::config::ModelSpaceMode) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    app.default_assoc_prompted = true;
    app.donation_prompt_version = env!("OCS_APP_VERSION").to_owned();
    app.model_space.mode = mode;
    app.sync_model_space_theme(false);
    let cfg = GuiConfig {
        classic,
        new: classic,
        ..GuiConfig::default()
    };
    let first = app.open_first_screen(&cfg);
    app.drive_headless_task(first)
        .expect("the first screen opens without a window");
    app
}

fn launched(classic: bool) -> OpenCADStudio {
    launched_with_mode(classic, crate::app::config::ModelSpaceMode::default())
}

/// The design's canvas colour (`bg-canvas`) in `theme`.
fn designs_colour(theme: PidTheme) -> [f32; 4] {
    let colour = ColorToken::BgCanvas.color(theme);
    [colour.r, colour.g, colour.b, colour.a]
}

/// What the drawing in front is cleared to.
fn cleared_to(app: &OpenCADStudio) -> [f32; 4] {
    app.tabs[app.active_tab].scene.bg_color
}

/// Inside the module a drawing is cleared to the design's canvas colour —
/// the one the launch opens, one opened later, and one that was open when
/// the module came on. In the general interface it is the user's model
/// space background, which the module leaves as it is set.
#[test]
fn the_canvas_is_the_designs_colour_in_the_module_and_the_users_own_outside_it() {
    let mode = crate::app::config::ModelSpaceMode::ClassicDark;
    let general = launched_with_mode(true, mode);
    // What the general interface clears a drawing to, and the setting it
    // has for that.
    let users = cleared_to(&general);
    let setting = general.model_space.resolve_model_bg(&general.active_theme);

    let mut module = launched_with_mode(false, mode);
    assert_eq!(module.pid_shell.theme, PidTheme::Light);
    let designs = designs_colour(module.pid_shell.theme);
    assert_ne!(designs, users, "the two colours are not the same colour");
    assert_eq!(cleared_to(&module), designs, "the drawing the launch opens");
    let opened = module.update(AppMessage::TabNew);
    module
        .drive_headless_task(opened)
        .expect("a drawing opens without a window");
    assert_eq!(cleared_to(&module), designs, "a drawing opened later");
    // The user's setting is what it was: only not used here.
    assert_eq!(
        module.model_space.resolve_model_bg(&module.active_theme),
        setting
    );

    // The module coming on over an open drawing.
    let mut switched = launched_with_mode(true, mode);
    assert_eq!(cleared_to(&switched), users);
    let _ = switched.dispatch_command("PIDSCREEN editor");
    assert_eq!(switched.pid_shell.current(), Some(PidScreen::Editor));
    assert_eq!(cleared_to(&switched), designs);
    assert_eq!(
        switched
            .model_space
            .resolve_model_bg(&switched.active_theme),
        setting
    );
}

#[test]
fn the_default_theme_is_light_and_colortheme_switches_both_halves() {
    let mut app = launched(false);
    assert_eq!(app.active_theme, iced::Theme::Light);
    assert_eq!(app.pid_shell.theme, PidTheme::Light);
    assert_eq!(cleared_to(&app), designs_colour(PidTheme::Light));

    let _ = app.dispatch_command("COLORTHEME 0");
    assert_eq!(app.active_theme, iced::Theme::Dark);
    assert_eq!(app.pid_shell.theme, PidTheme::Dark);
    assert_eq!(cleared_to(&app), designs_colour(PidTheme::Dark));

    let _ = app.dispatch_command("COLORTHEME 1");
    assert_eq!(app.active_theme, iced::Theme::Light);
    assert_eq!(app.pid_shell.theme, PidTheme::Light);
    assert_eq!(cleared_to(&app), designs_colour(PidTheme::Light));
}

const PEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/pid-platform/design/pid-platform.pen"
);

/// The first node with this id under `node`, descendants of an instance
/// included.
fn node_with_id<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    match node {
        Value::Object(fields) => {
            if fields.get("id").and_then(Value::as_str) == Some(id) {
                return Some(node);
            }
            fields.values().find_map(|field| node_with_id(field, id))
        }
        Value::Array(items) => items.iter().find_map(|item| node_with_id(item, id)),
        _ => None,
    }
}

/// The grid the editor draws is the design's: lines, every fifth one
/// stronger, in the two grid variables the design's canvas names — read from
/// the design file.
#[test]
fn the_grid_is_drawn_the_way_the_design_draws_it() {
    let text = std::fs::read_to_string(PEN).unwrap_or_else(|e| panic!("{PEN}: {e}"));
    let pen: Value = serde_json::from_str(&text).expect("the design is JSON");
    let drawn = node_with_id(&pen, "eSZiQ").expect("the editor's canvas");
    // The vertical lines of the design's grid: where each is, and its fill.
    let mut lines: Vec<(f64, String)> = drawn["children"]
        .as_array()
        .expect("the canvas has children")
        .iter()
        .filter(|node| node["name"] == "Grid V")
        .map(|node| {
            (
                node["x"].as_f64().expect("a place"),
                node["fill"].as_str().expect("a fill").to_owned(),
            )
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(lines.len() > 10, "the design draws a grid: {lines:?}");
    let step = lines[1].0 - lines[0].0;
    for (index, (x, fill)) in lines.iter().enumerate() {
        assert_eq!(*x, step * index as f64, "evenly spaced");
        let major = (index as u32).is_multiple_of(canvas::GRID_MAJOR_EVERY);
        assert_eq!(
            fill,
            if major {
                "$canvas-grid-major"
            } else {
                "$canvas-grid"
            },
            "line {index} at {x}"
        );
    }
    assert_eq!(canvas::GRID_MAJOR_EVERY, 5);

    // And that is the style the module's drawing area is given in both
    // themes: the design's grid colours at their two strengths, with no axis
    // lines.
    let app = launched(false);
    assert_eq!(app.pid_shell.theme, PidTheme::Light);
    for theme in PidTheme::ALL {
        let style = canvas::grid_style(theme, 0.1);
        let minor = ColorToken::CanvasGrid.color(theme);
        let major = ColorToken::CanvasGridMajor.color(theme);
        assert_eq!([major.r, major.g, major.b], [minor.r, minor.g, minor.b]);
        assert_eq!(style.color, [minor.r, minor.g, minor.b]);
        assert_eq!(style.line_alpha, Some((minor.a, major.a)));
        assert!(!style.axes);
    }
}

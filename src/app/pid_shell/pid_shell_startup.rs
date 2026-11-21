//! The first screen of a launch, the switch between screens, the navigation,
//! and the screen list held to the design.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;
use crate::cli::GuiConfig;

// ── A launch ─────────────────────────────────────────────────────────────

/// The application as `boot` leaves it for a launch with this command line,
/// on a machine where the startup prompts have been answered: built, and its
/// first screen opened.
fn launched(cfg: &GuiConfig) -> OpenCADStudio {
    boot(cfg, false)
}

/// The same on a first launch, when a prompt is up before any message runs.
fn first_launch(cfg: &GuiConfig) -> OpenCADStudio {
    boot(cfg, true)
}

fn boot(cfg: &GuiConfig, first_launch: bool) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_for_test();
    if !first_launch {
        app.default_assoc_prompted = true;
        app.donation_prompt_version = env!("OCS_APP_VERSION").to_owned();
    }
    // In `boot`'s order: the first screen is asked for, the prompts are
    // queued (an unattended script gets none), and only then does iced run
    // the task.
    let first = app.open_first_screen(cfg);
    if cfg.script_lines.is_empty() {
        app.queue_startup_prompts();
    }
    assert_eq!(
        app.active_modal.is_some(),
        first_launch && cfg.script_lines.is_empty(),
        "a prompt is up on a first launch and on no other"
    );
    app.drive_headless_task(first)
        .expect("the first screen opens without a window");
    app
}

fn classic() -> GuiConfig {
    GuiConfig {
        classic: true,
        ..GuiConfig::default()
    }
}

/// The drawings open in the application: every tab but the Start page.
fn drawings(app: &OpenCADStudio) -> usize {
    app.tabs.iter().filter(|tab| !tab.is_start).count()
}

/// Run `cmd` as typed and give back what the command line said to it.
fn say(app: &mut OpenCADStudio, cmd: &str) -> Vec<String> {
    let before = app.command_line.history.len();
    let _ = app.dispatch_command(cmd);
    app.command_line.history[before..]
        .iter()
        .map(|entry| entry.text.clone())
        .collect()
}

// ── The window as it is drawn ────────────────────────────────────────────

/// Size the window is laid out at: the design's frame.
const WINDOW: iced::Size = iced::Size::new(1600.0, 1000.0);

/// What one drawn frame of the main window shows.
struct Frame {
    /// The rectangles the P&ID module reported, as the automation channel's
    /// `ui_bounds` lists them: `(id, [x, y, width, height])`.
    shell: Vec<(String, [f64; 4])>,
    /// Whether the general interface's Start page is in the window.
    start_page: bool,
}

/// Draw the main window once and read the frame back.
fn draw_window(app: &mut OpenCADStudio) -> Frame {
    use iced_test::core::{mouse, renderer, widget};
    use iced_test::runtime::user_interface::{Cache, UserInterface};
    use iced_test::Selector;

    crate::ui::wrap_bar::forget_bounds(BOUNDS_PREFIX);
    let mut headless = iced_test::futures::futures::executor::block_on(
        <iced::Renderer as renderer::Headless>::new(
            renderer::Settings::default(),
            std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
        ),
    )
    .expect("headless renderer");
    // The Start page's first button: only that page has it.
    let start_label = crate::tr!("start", "new-drawing");
    let start_page = {
        let mut ui = UserInterface::build(app.view_main(), WINDOW, Cache::default(), &mut headless);
        ui.draw(
            &mut headless,
            &app.active_theme,
            &renderer::Style {
                text_color: iced::Color::BLACK,
            },
            mouse::Cursor::Unavailable,
        );
        let mut find = Selector::find(start_label.as_str());
        ui.operate(&headless, &mut widget::operation::black_box(&mut find));
        matches!(
            widget::Operation::finish(&find),
            widget::operation::Outcome::Some(Some(_))
        )
    };
    // Read where an automation client reads: the control channel.
    let (answer, _) = app.control_request(json!({"op": "ui_bounds", "prefix": BOUNDS_PREFIX}));
    assert_eq!(answer["ok"], true, "{answer}");
    let shell = answer["widgets"]
        .as_array()
        .expect("a list of widgets")
        .iter()
        .map(|widget| {
            let bounds = widget["bounds"].as_array().expect("four numbers");
            let at = |i: usize| bounds[i].as_f64().expect("a number");
            (
                widget["id"].as_str().expect("an id").to_owned(),
                [at(0), at(1), at(2), at(3)],
            )
        })
        .collect();
    Frame { shell, start_page }
}

// ── The first screen ─────────────────────────────────────────────────────

#[test]
fn a_launch_without_arguments_lands_on_the_editor_with_a_drawing_to_draw_on() {
    assert_eq!(
        STARTUP_SCREEN,
        PidScreen::Editor,
        "the setting these tests are written for"
    );
    // A first launch has a prompt up before any message runs, and an open
    // dialog swallows `TabNew`: the drawing has to be there all the same.
    for app in [
        first_launch(&GuiConfig::default()),
        launched(&GuiConfig::default()),
    ] {
        assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
        assert_eq!(drawings(&app), 1, "one drawing was opened");
        let tab = &app.tabs[app.active_tab];
        assert!(!tab.is_start, "a drawing is in front, not the Start page");
        assert_eq!(tab.scene.current_layout, "Model");
        assert_eq!(tab.scene.document.entities().count(), 0, "a blank one");
    }
}

#[test]
fn the_first_screen_is_the_pid_root_view_and_the_start_page_is_not_drawn() {
    let _one = crate::headless_ui_lock();
    let mut app = first_launch(&GuiConfig::default());
    let frame = draw_window(&mut app);
    // The root view gives the screen that is showing the shell's content
    // area — the window less the title bar, the navigation and the status
    // bar — and reports it under the screen's id.
    assert_eq!(
        frame.shell,
        vec![(
            "PID_SHELL:editor".to_owned(),
            [
                f64::from(chrome::NAV_WIDTH),
                f64::from(chrome::TITLE_BAR_HEIGHT),
                f64::from(WINDOW.width - chrome::NAV_WIDTH),
                f64::from(WINDOW.height - chrome::TITLE_BAR_HEIGHT - chrome::STATUS_BAR_HEIGHT)
            ]
        )]
    );
    assert!(!frame.start_page, "the general Start page is not drawn");
}

/// The drawing the launch opened is one the existing P&ID commands draw on:
/// `PIDINSERT` through the control channel, as the palette and a script run
/// it, leaves a symbol carrying its P&ID record.
#[test]
fn the_pid_commands_draw_on_the_drawing_the_launch_opened() {
    let mut app = launched(&GuiConfig::default());
    for (op, what) in [
        (
            json!({"op": "run", "cmd": "PIDINSERT vessel"}),
            "the command starts",
        ),
        (
            json!({"op": "input", "kind": "point", "point": [200.0, 150.0, 0.0], "space": "wcs"}),
            "the point is taken",
        ),
        (
            json!({"op": "input", "kind": "enter"}),
            "Enter places the symbol",
        ),
    ] {
        let done = request(&mut app, op);
        assert_eq!(done["ok"], true, "{what}: {done}");
    }
    let document = &app.tabs[app.active_tab].scene.document;
    let placed: Vec<(String, String)> = document
        .entities()
        .filter_map(|entity| match entity {
            document::EntityType::Insert(insert) => Some((
                insert.block_name.clone(),
                crate::pid::persist::PidAuthor::load(document, insert.common.handle)
                    .map(|author| author.tag)
                    .unwrap_or_default(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(placed.len(), 1, "{placed:?}");
    assert_eq!(placed[0].0, "PID_SYM_equipment_vessel");
    assert!(!placed[0].1.is_empty(), "the symbol was given its tag");
}

/// One protocol-1 request on the control channel, waited for until it is no
/// longer `accepted` / `running`.
fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let state = app.control_state();
    req["protocol"] = json!(1);
    req["document_id"] = state["document_id"].clone();
    req["revision"] = state["revision"].clone();
    req["client_id"] = json!("pid-shell-startup");
    req["request_id"] = json!(format!(
        "pid-shell-startup-{}",
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let (response, task) = app.control_request(req.clone());
    app.drive_headless_task(task).unwrap();
    if matches!(response["status"].as_str(), Some("accepted" | "running")) {
        app.control_request(json!({"op": "operation", "request_id": req["request_id"]}))
            .0
    } else {
        response
    }
}

#[test]
fn classic_is_the_general_interface_on_its_start_page() {
    let _one = crate::headless_ui_lock();
    for mut app in [first_launch(&classic()), launched(&classic())] {
        assert_eq!(app.pid_shell.current(), None, "the P&ID module is off");
        assert_eq!(drawings(&app), 0);
        assert!(app.tabs[app.active_tab].is_start);
        let frame = draw_window(&mut app);
        assert!(frame.shell.is_empty(), "{:?}", frame.shell);
        assert!(frame.start_page, "the Start page is the first screen");
    }
}

/// `--new` and `--script` opened a drawing before, and still open one; in
/// the module that drawing is the one the editor starts with, on a first
/// launch too.
#[test]
fn a_launch_that_asks_for_a_drawing_gets_one_and_not_two() {
    let app = first_launch(&GuiConfig {
        new: true,
        ..GuiConfig::default()
    });
    assert_eq!(drawings(&app), 1, "--new in the module, first launch");
    let asks = [
        GuiConfig {
            new: true,
            ..GuiConfig::default()
        },
        GuiConfig {
            script_lines: vec!["ZOOM E".to_owned()],
            ..GuiConfig::default()
        },
        GuiConfig {
            new: true,
            ..classic()
        },
        GuiConfig {
            script_lines: vec!["ZOOM E".to_owned()],
            ..classic()
        },
    ];
    for cfg in asks {
        let app = launched(&cfg);
        assert_eq!(drawings(&app), 1, "{cfg:?}");
        assert!(!app.tabs[app.active_tab].is_start, "{cfg:?}");
        assert_eq!(app.pid_shell.current().is_some(), !cfg.classic, "{cfg:?}");
    }
}

/// A launch that names a file opens that file, in the module, with no blank
/// drawing beside it.
#[test]
fn a_named_file_is_what_opens() {
    let name = "pid-shell-startup-no-such-drawing.dxf";
    for (cfg, showing) in [
        (GuiConfig::default(), Some(PidScreen::Editor)),
        (classic(), None),
    ] {
        let app = first_launch(&GuiConfig {
            files: vec![std::env::temp_dir().join(name)],
            ..cfg
        });
        assert_eq!(app.pid_shell.current(), showing);
        assert_eq!(drawings(&app), 0, "no blank drawing was opened");
        // The path went through the door every opened file goes through,
        // which says so when the file is not there.
        assert!(
            app.command_line
                .history
                .iter()
                .any(|entry| entry.text.contains(name)),
            "the open path was handed the file"
        );
    }
}

#[test]
fn classic_is_a_launch_flag() {
    use clap::Parser;
    assert!(!crate::cli::Cli::parse_from(["OpenCADStudio"]).classic);
    let cli = crate::cli::Cli::parse_from(["OpenCADStudio", "--classic", "a.dwg"]);
    assert!(cli.classic);
    assert_eq!(cli.files, vec![PathBuf::from("a.dwg")]);
}

// ── Switching ────────────────────────────────────────────────────────────

#[test]
fn a_screen_is_shown_by_the_message_and_by_the_command() {
    // The message, from the general interface: the module comes on.
    let mut app = launched(&classic());
    let _ = app.update(AppMessage::PidShell(Message::Show(PidScreen::Editor)));
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));

    // The command, from the Start page, where no drawing is open; the id in
    // whatever case it is typed.
    let mut app = launched(&classic());
    assert!(app.tabs[app.active_tab].is_start);
    let said = say(&mut app, "PIDSCREEN");
    assert!(
        said.iter()
            .any(|line| line.contains("通用界面") && line.contains("editor")),
        "bare, in the general interface: {said:?}"
    );
    let said = say(&mut app, "PIDSCREEN Editor");
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
    assert!(
        said.iter().any(|line| line.contains("P&ID 编辑器")),
        "{said:?}"
    );
    let said = say(&mut app, "PIDSCREEN");
    assert!(
        said.iter()
            .any(|line| line.contains("当前界面 P&ID 编辑器（editor）")),
        "bare, in the module: {said:?}"
    );

    // What it cannot take leaves the screen where it was and says why.
    for (typed, why) in [
        ("PIDSCREEN nowhere", "没有叫 \"nowhere\" 的界面"),
        ("PIDSCREEN editor typelib", "只接受一个界面名"),
    ] {
        let said = say(&mut app, typed);
        assert!(
            said.iter().any(|line| line.contains(why)),
            "{typed}: {said:?}"
        );
        assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor), "{typed}");
    }
}

/// The automation channel reaches the switch as it reaches any command.
#[test]
fn the_automation_channel_runs_the_switch() {
    let mut app = launched(&classic());
    let before = app.command_line.history.len();
    let done = app.automation_op(&json!({"op": "run", "cmd": "PIDSCREEN editor"}).to_string());
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
    // The line is one command: it answers once, for the screen now showing.
    let answers: Vec<&str> = app.command_line.history[before..]
        .iter()
        .map(|entry| entry.text.as_str())
        .filter(|text| text.contains("PIDSCREEN:"))
        .collect();
    assert_eq!(answers.len(), 1, "{answers:?}");
    assert!(answers[0].contains("当前界面 P&ID 编辑器"), "{answers:?}");
}

// ── The navigation ───────────────────────────────────────────────────────

#[test]
fn the_navigation_lists_the_implemented_screens_and_no_other() {
    let listed: Vec<PidScreen> = nav().collect();
    assert!(listed.contains(&PidScreen::Editor), "the editor is there");
    for screen in PidScreen::ALL {
        assert_eq!(
            listed.contains(&screen),
            screen.view_fn().is_some(),
            "{screen:?} is listed exactly when it has a view"
        );
    }
    // In the design's order, each once.
    let in_order: Vec<PidScreen> = PidScreen::ALL
        .into_iter()
        .filter(|screen| listed.contains(screen))
        .collect();
    assert_eq!(listed, in_order);
}

#[test]
fn a_screen_that_is_not_implemented_cannot_be_shown() {
    let mut app = launched(&GuiConfig::default());
    let mut off = PidShell::default();
    for screen in PidScreen::ALL {
        if screen.is_implemented() {
            continue;
        }
        // Not by the state …
        assert!(!off.show(screen), "{screen:?}");
        assert_eq!(off.current(), None, "{screen:?}: the module stays off");
        // … not by the message a navigation entry would send …
        let before = app.command_line.history.len();
        let _ = app.update(AppMessage::PidShell(Message::Show(screen)));
        assert_eq!(
            app.pid_shell.current(),
            Some(PidScreen::Editor),
            "{screen:?}"
        );
        assert!(
            app.command_line.history[before..]
                .iter()
                .any(|entry| entry.text.contains(screen.title())),
            "{screen:?}: the refusal names the screen"
        );
        // … and not by the command.
        let said = say(&mut app, &format!("PIDSCREEN {}", screen.id()));
        assert_eq!(
            app.pid_shell.current(),
            Some(PidScreen::Editor),
            "{screen:?}"
        );
        assert!(
            said.iter().any(|line| line.contains("还没有实现")),
            "{screen:?}: {said:?}"
        );
    }
}

// ── The design ───────────────────────────────────────────────────────────

const PEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/pid-platform/design/pid-platform.pen"
);
const CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/pid-platform/prototype/specs/screen-catalog.json"
);

fn read_json(path: &str) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The first node called `name` in `node` and what it contains.
fn node_named<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    if node["name"] == name {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| node_named(child, name))
}

/// The text a navigation row or group heading shows: its one text child.
fn label_of(node: &Value) -> String {
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|child| child["type"] == "text")
        .and_then(|text| text["content"].as_str())
        .unwrap_or_else(|| panic!("{} has no text", node["name"]))
        .to_owned()
}

/// The screens, their names, their groups and their order are the design's:
/// the navigation of the shell component in the `.pen`, read here from the
/// file itself, and the screen catalog.
#[test]
fn the_screens_are_the_ones_the_design_navigates_to() {
    let coded: Vec<(String, String, String)> = PidScreen::ALL
        .into_iter()
        .map(|screen| {
            (
                screen.id().to_owned(),
                screen.title().to_owned(),
                screen.group().title().to_owned(),
            )
        })
        .collect();

    let pen = read_json(PEN);
    let shell = pen["children"]
        .as_array()
        .expect("the design has top-level nodes")
        .iter()
        .find(|node| node["name"] == "component/App Shell")
        .expect("the shell component");
    let navigation = node_named(shell, "Nav").expect("the shell's navigation");
    let mut designed = Vec::new();
    let mut group = String::new();
    for row in navigation["children"].as_array().expect("navigation rows") {
        let name = row["name"].as_str().unwrap_or_default();
        if name.starts_with("Group ") {
            group = label_of(row);
        } else if let Some(id) = name.strip_prefix("Nav ") {
            designed.push((id.to_owned(), label_of(row), group.clone()));
        }
    }
    assert_eq!(coded, designed, "the navigation of the design");

    let catalog = read_json(CATALOG);
    let catalogued: Vec<(String, String)> = catalog["screens"]
        .as_array()
        .expect("the catalog lists screens")
        .iter()
        .map(|screen| {
            (
                screen["id"].as_str().expect("an id").to_owned(),
                screen["name"].as_str().expect("a name").to_owned(),
            )
        })
        .collect();
    let named: Vec<(String, String)> = coded
        .iter()
        .map(|(id, title, _)| (id.clone(), title.clone()))
        .collect();
    assert_eq!(named, catalogued, "the screen catalog");

    for screen in PidScreen::ALL {
        assert_eq!(PidScreen::from_id(screen.id()), Some(screen));
        assert_eq!(
            PidScreen::from_id(&screen.id().to_uppercase()),
            Some(screen)
        );
        assert_eq!(bounds_id(screen), format!("PID_SHELL:{}", screen.id()));
    }
    assert_eq!(PidScreen::from_id("start"), None);
}

// ── The module docs ──────────────────────────────────────────────────────

/// The calls of "From startup to another screen" in the module docs, made.
#[test]
fn the_call_example_of_the_module_docs_runs() {
    let _one = crate::headless_ui_lock();
    // `new_for_test` is `new` on default settings instead of this machine's.
    let mut app = OpenCADStudio::new_for_test();
    let first = app.open_first_screen(&GuiConfig::default());
    assert_eq!(app.pid_shell.current(), Some(STARTUP_SCREEN));
    app.drive_headless_task(first).expect("iced runs the task");

    let _ = app.update(AppMessage::PidShell(Message::Show(PidScreen::Editor)));
    let _ = app.dispatch_command("PIDSCREEN editor");
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));

    let window = app.view_main();
    drop(window);
}

#[test]
fn pid_shell_ui3_backup_screen_is_registered_in_nav_and_draws_its_frame() {
    let _one = crate::headless_ui_lock();
    let listed: Vec<PidScreen> = nav().collect();
    assert_eq!(listed, [PidScreen::Editor, PidScreen::Backup]);
    assert_eq!(PidScreen::Backup.id(), "backup");
    assert_eq!(PidScreen::Backup.title(), "备份数据包");
    assert_eq!(PidScreen::Backup.group(), NavGroup::Design);
    assert!(!PidScreen::Backup.needs_drawing());

    let mut app = launched(&GuiConfig::default());
    let said = say(&mut app, "PIDSCREEN backup");
    assert_eq!(app.pid_shell.current(), Some(PidScreen::Backup));
    assert!(
        said.iter()
            .any(|line| line.contains("当前界面 备份数据包（backup）")),
        "{said:?}"
    );
    let frame = draw_window(&mut app);
    assert_eq!(
        frame.shell,
        vec![(
            "PID_SHELL:backup".to_owned(),
            [
                f64::from(chrome::NAV_WIDTH),
                f64::from(chrome::TITLE_BAR_HEIGHT),
                f64::from(WINDOW.width - chrome::NAV_WIDTH),
                f64::from(WINDOW.height - chrome::TITLE_BAR_HEIGHT - chrome::STATUS_BAR_HEIGHT),
            ],
        )]
    );
}

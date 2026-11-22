//! `PIDVERIFY` — shows, hides and turns the 「校验与交付」 panel
//! (`SPEC-pid-verify-deliver-panel.md` §5.2), and the program's side of the
//! panel: its messages and the view it draws.
//!
//! The panel's interface state lives on the program (`pid_verify_ui`) and
//! the panel draws the plain data `build_view` makes of it
//! (`pid_verify_view`), so whatever changes that state ends in
//! [`OpenCADStudio::refresh_pid_verify_view`]. The command never changes
//! the drawing, so a read-only session may run every form of it.

use crate::app::{Message, OpenCADStudio};
use crate::ui::dock::{DockMsg, PanelId};
use crate::ui::window::pid_verify_deliver::{self, PidVerifyMsg, Tab};
use iced::Task;

impl OpenCADStudio {
    /// Rebuild what the panel draws from its interface state.
    pub(in crate::app) fn refresh_pid_verify_view(&mut self) {
        self.pid_verify_view = pid_verify_deliver::build_view(&self.pid_verify_ui);
    }

    /// Show the panel the way its switch among the dock's panels does:
    /// docked on the right the first time, where the user left it after.
    pub(in crate::app) fn show_pid_verify_panel(&mut self) -> Task<Message> {
        self.refresh_pid_verify_view();
        self.dock_set_open(PanelId::PidVerifyDeliver, true)
    }

    /// `PIDVERIFY` (§5.2): bare or `ON` shows the panel, `OFF` hides it, and
    /// `CHECK`, `BOM` or `DELIVER` shows it on that tab. Each run says on the
    /// command line what it did.
    pub(super) fn dispatch_pidverify(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let line = cmd.trim();
        let (verb, arg) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if !verb.eq_ignore_ascii_case("PIDVERIFY") {
            return None;
        }
        let arg = arg.trim();
        if arg.eq_ignore_ascii_case("OFF") {
            let task = self.on_dock(DockMsg::Close(PanelId::PidVerifyDeliver));
            self.command_line
                .push_info("PIDVERIFY: 已隐藏「校验与交付」面板。");
            return Some(task);
        }
        if !arg.is_empty() && !arg.eq_ignore_ascii_case("ON") {
            let Some(tab) = Tab::from_key(arg) else {
                self.command_line.push_error(&format!(
                    "PIDVERIFY: 只接受 ON、OFF、CHECK、BOM、DELIVER 中的一个，实得 \"{arg}\"。"
                ));
                return Some(Task::none());
            };
            self.pid_verify_ui.tab = tab;
        }
        let task = self.show_pid_verify_panel();
        let label = self.pid_verify_ui.tab.label();
        self.command_line.push_info(&format!(
            "PIDVERIFY: 已显示「校验与交付」面板，当前页签「{label}」。"
        ));
        Some(task)
    }

    /// What the user did in the panel.
    pub(in crate::app) fn on_pid_verify(&mut self, msg: PidVerifyMsg) -> Task<Message> {
        match msg {
            PidVerifyMsg::Tab(tab) => {
                self.pid_verify_ui.tab = tab;
                self.refresh_pid_verify_view();
            }
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use crate::app::config::DockSide;
    use crate::app::{Message, OpenCADStudio};
    use crate::ui::dock::{DockMsg, PanelId};
    use crate::ui::window::pid_verify_deliver::{
        PidVerifyMsg, Tab, BOUNDS_PREFIX, PANEL_BOUNDS_ID,
    };

    const PANEL: PanelId = PanelId::PidVerifyDeliver;

    /// A program with a new drawing open and the default dock layout, which
    /// has no place for the panel yet.
    fn fresh() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        // Tests load the user's persisted config; reset the dock to a known
        // state so this stays hermetic.
        app.dock = Default::default();
        app.dock.ensure_settings();
        app
    }

    /// What one command line said, and the error it ended on.
    struct Ran {
        said: String,
        error: Option<String>,
    }

    /// Run `line` as if typed at the command line, with whatever it starts.
    fn run(app: &mut OpenCADStudio, line: &str) -> Ran {
        app.command_line.history.clear();
        app.command_line.last_error = None;
        let task = app.run_command_line(line);
        app.drive_headless_task(task).expect("the command's task");
        Ran {
            said: app
                .command_line
                .history
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            error: app.command_line.last_error.clone(),
        }
    }

    /// Whether the panel is on screen, and the tab it has in front.
    fn shown(app: &OpenCADStudio) -> (bool, Tab) {
        (app.dock_panel_visible(PANEL), app.pid_verify_view.tab)
    }

    fn undo_steps(app: &OpenCADStudio) -> usize {
        app.tabs[app.active_tab].history.undo_stack.len()
    }

    fn reported_ids() -> Vec<String> {
        crate::ui::wrap_bar::recorded_bounds()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Record bounds for two of the panel's controls and for another
    /// panel's body, as drawing them would.
    fn pretend_drawn() {
        let drawn = iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        for id in [PANEL_BOUNDS_ID, "pid-verify-tab-bom", "pid-collab-panel"] {
            crate::ui::wrap_bar::set_drawn_bounds(id, drawn);
        }
    }

    /// The ids the real view reports once it is drawn.
    fn drawn_ids(app: &OpenCADStudio) -> Vec<String> {
        crate::ui::wrap_bar::forget_bounds("");
        let size = iced::Size::new(1600.0, 900.0);
        let mut simulator =
            iced_test::Simulator::with_size(iced::Settings::default(), size, app.view_main());
        simulator
            .snapshot(&app.active_theme)
            .expect("draw the view");
        reported_ids()
    }

    #[test]
    fn pidverify_shows_hides_and_turns_the_panel_to_a_tab() {
        let mut app = fresh();
        assert_eq!(shown(&app), (false, Tab::Check));
        for (line, visible, tab) in [
            ("PIDVERIFY", true, Tab::Check),
            ("PIDVERIFY BOM", true, Tab::Bom),
            ("PIDVERIFY deliver", true, Tab::Deliver),
            ("PIDVERIFY OFF", false, Tab::Deliver),
            // Shown again, it is on the tab it was left on.
            ("PIDVERIFY ON", true, Tab::Deliver),
            ("PIDVERIFY CHECK", true, Tab::Check),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(shown(&app), (visible, tab), "{line}");
            let says = if visible {
                format!(
                    "PIDVERIFY: 已显示「校验与交付」面板，当前页签「{}」。",
                    tab.label()
                )
            } else {
                "PIDVERIFY: 已隐藏「校验与交付」面板。".to_owned()
            };
            assert!(ran.said.contains(&says), "{line}: {}", ran.said);
        }
        // The first time, it docked on the right, as the pallet menu docks it.
        assert_eq!(
            app.dock.location(PANEL).map(|(side, _)| side),
            Some(DockSide::Right)
        );
    }

    #[test]
    fn pidverify_bom_typed_at_the_command_line_runs_once() {
        // Off the whole-line list, the command line would try the bare verb
        // first and then run the whole line: two reports, the first one on
        // the rule-check tab.
        let mut app = fresh();
        let ran = run(&mut app, "PIDVERIFY BOM");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(ran.said.matches("PIDVERIFY:").count(), 1, "{}", ran.said);
        assert_eq!(shown(&app), (true, Tab::Bom));
    }

    #[test]
    fn pidverify_names_the_words_it_takes_and_shows_nothing_for_others() {
        let mut app = fresh();
        for line in ["PIDVERIFY SHOW", "PIDVERIFY BOM CHECK"] {
            let error = run(&mut app, line).error.unwrap_or_default();
            assert!(
                error.starts_with("PIDVERIFY: 只接受 ON、OFF、CHECK、BOM、DELIVER 中的一个"),
                "{line}: {error}"
            );
        }
        assert_eq!(shown(&app), (false, Tab::Check));
    }

    #[test]
    fn closing_the_panel_forgets_where_its_controls_were() {
        let mut app = fresh();
        for how in ["PIDVERIFY OFF", "the close button"] {
            let _ = run(&mut app, "PIDVERIFY");
            pretend_drawn();
            if how == "the close button" {
                let _ = app.on_dock(DockMsg::Close(PANEL));
            } else {
                let _ = run(&mut app, how);
            }
            assert!(!app.dock_panel_visible(PANEL), "{how}");
            let left = reported_ids();
            assert!(
                !left.iter().any(|id| id.starts_with(BOUNDS_PREFIX)),
                "{how}: {left:?}"
            );
            // Only its own: what another panel reported stays.
            assert!(
                left.iter().any(|id| id == "pid-collab-panel"),
                "{how}: {left:?}"
            );
        }
        crate::ui::wrap_bar::forget_bounds("pid-collab-panel");
    }

    #[test]
    fn a_read_only_session_runs_every_form() {
        let mut app = fresh();
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for line in [
            "PIDVERIFY",
            "PIDVERIFY BOM",
            "PIDVERIFY DELIVER",
            "PIDVERIFY OFF",
            "PIDVERIFY ON",
            "PIDVERIFY CHECK",
        ] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        assert_eq!(shown(&app), (true, Tab::Check));
        // Showing the panel changed nothing in the drawing.
        assert_eq!(undo_steps(&app), steps);
    }

    #[test]
    fn a_tab_turned_in_the_panel_is_where_the_command_finds_it() {
        let mut app = fresh();
        let _ = run(&mut app, "PIDVERIFY");
        let _ = app.update(Message::PidVerify(PidVerifyMsg::Tab(Tab::Deliver)));
        assert_eq!(shown(&app), (true, Tab::Deliver));
        let _ = run(&mut app, "PIDVERIFY OFF");
        let ran = run(&mut app, "PIDVERIFY");
        assert_eq!(shown(&app), (true, Tab::Deliver));
        assert!(ran.said.contains("当前页签「快照与发布」"), "{}", ran.said);
    }

    #[test]
    fn the_drawn_panel_reports_its_body_and_tabs_until_it_closes() {
        // Building and drawing the real view is deeper than a default test
        // thread's stack, and two headless renderers made at once crash the
        // process.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let mut app = fresh();
                app.win_size = (1600.0, 900.0);
                let _ = run(&mut app, "PIDVERIFY BOM");
                let drawn = drawn_ids(&app);
                for id in [
                    PANEL_BOUNDS_ID,
                    "pid-verify-tab-check",
                    "pid-verify-tab-bom",
                    "pid-verify-tab-deliver",
                ] {
                    assert!(drawn.iter().any(|seen| seen == id), "{id}: {drawn:?}");
                }
                // Closed before the next frame, the panel reports nothing.
                let _ = run(&mut app, "PIDVERIFY OFF");
                let left = reported_ids();
                assert!(
                    !left.iter().any(|id| id.starts_with(BOUNDS_PREFIX)),
                    "{left:?}"
                );
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }
}

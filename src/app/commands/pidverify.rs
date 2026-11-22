//! `PIDVERIFY` — shows, hides and turns the 「校验与交付」 panel
//! (`SPEC-pid-verify-deliver-panel.md` §5.2), and the program's side of the
//! panel: its messages, the view it draws and the panel state the
//! `pid_verify_panel` op reads off that view (§3.2).
//!
//! The panel's interface state lives on the program (`pid_verify_ui`) and
//! the panel draws the plain data `build_view` makes of it
//! (`pid_verify_view`), so whatever changes that state ends in
//! [`OpenCADStudio::refresh_pid_verify_view`]. What the view shows besides
//! — the checks kept for the drawing in front and whether a drawing they
//! read changed since, whether a scan runs — changes under other messages
//! too: while the panel is open, every message ends by rebuilding the view
//! when any of that changed ([`OpenCADStudio::sync_pid_verify_view`]). The
//! command never changes the drawing, so a read-only session may run every
//! form of it. The `action` op named `pid_verify` presses the panel with
//! these same messages (§3.3).
//!
//! The rule-check tab's buttons call what `pid_check` calls: the check of
//! the drawing in front, the check of its project after `PIDINDEX`'s scan,
//! and the export `PIDCHECK EXPORT` writes. What stops them is said in the
//! panel's message bar rather than on the command line, so an `action`
//! pressing them is not answered as failed for it (§3.7).
//!
//! A row clicked finds its object the way `PIDLOCATE` does, on whichever
//! drawing it is (`pidindex.rs`), and the waive buttons call what
//! `PIDCHECK WAIVE` and `PIDCHECK UNWAIVE` call; they too say in the
//! message bar what came of it.

use super::piddrc::Checked;
use super::pidindex::{same_file, Landed, LOCATE_NOT_FOUND};
use crate::app::{Message, OpenCADStudio};
use crate::pid::drc::{self, CheckResult};
use crate::ui::dock::{DockMsg, PanelId};
use crate::ui::window::pid_verify_deliver::{
    self, Context, DrawingResults, KeptCheck, Notice, PidVerifyMsg, ProjectResults, Tab, Waiving,
    GONE, REASON_ID, ROWS_PER_PAGE,
};
use iced::Task;
use serde_json::{json, Value};
use std::path::Path;

/// What the panel's view was built from besides its interface state: whether
/// it is on screen, the drawing tab in front and where that drawing was
/// saved, each open drawing's edit count, the last check the program made,
/// whether the session may write and whether a scan runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Inputs {
    visible: bool,
    front: Option<u64>,
    source: Option<String>,
    revisions: Vec<(u64, u64)>,
    last_check: u64,
    writable: bool,
    busy: bool,
}

impl OpenCADStudio {
    /// Rebuild what the panel draws from its interface state and the
    /// program's state around it. A check made since the last build starts
    /// the list over at its first 200 rows.
    pub(in crate::app) fn refresh_pid_verify_view(&mut self) {
        let inputs = self.pid_verify_inputs();
        if self
            .pid_verify_built_from
            .as_ref()
            .is_some_and(|built| built.last_check != inputs.last_check)
        {
            self.pid_verify_ui.more = 0;
        }
        let context = self.pid_verify_context();
        let view = {
            let (drawing, project) = self.pid_verify_kept();
            pid_verify_deliver::build_view(
                &self.pid_verify_ui,
                Some(&DrawingResults { check: drawing }),
                Some(&ProjectResults { check: project }),
                &context,
            )
        };
        self.pid_verify_view = view;
        self.pid_verify_built_from = Some(inputs);
    }

    /// After a message, rebuild the view if the panel is open and anything
    /// it was built from changed: a check made or followed, a drawing
    /// edited, another tab in front, a scan started or done.
    pub(in crate::app) fn sync_pid_verify_view(&mut self) {
        if !self.dock_panel_wanted(PanelId::PidVerifyDeliver) {
            return;
        }
        if self.pid_verify_built_from.as_ref() != Some(&self.pid_verify_inputs()) {
            self.refresh_pid_verify_view();
        }
    }

    fn pid_verify_inputs(&self) -> Inputs {
        let front = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start);
        Inputs {
            visible: self.dock_panel_visible(PanelId::PidVerifyDeliver),
            front: front.map(|tab| tab.id),
            source: front.and_then(|tab| tab.scene.document.source_path.clone()),
            revisions: self
                .tabs
                .iter()
                .map(|tab| (tab.id, tab.edit_revision))
                .collect(),
            last_check: self.pid_checks.last_serial(),
            writable: !self.readonly_refuses_command("PIDPUBLISH"),
            busy: self.pid_index.scanning(),
        }
    }

    /// The program's state the panel shows: whether it is on screen, the
    /// drawing tab in front and that drawing's number as the rule check
    /// gives it ([`drc::drawing_of`]), none on the Start page, whether the
    /// session may change that drawing, whether it was saved, whether a
    /// scan runs, and the waivers that drawing keeps.
    fn pid_verify_context(&self) -> Context {
        let drawing = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start);
        Context {
            visible: self.dock_panel_visible(PanelId::PidVerifyDeliver),
            document_id: drawing.map(|tab| tab.id),
            drawing_id: drawing.map(|tab| {
                let path = tab.scene.document.source_path.as_deref().map(Path::new);
                drc::drawing_of(path).0
            }),
            // The panel has no permissions of its own (§5.4): publishing and
            // waiving pass this gate, which only the session's own drawing
            // closes.
            writable: !self.readonly_refuses_command("PIDPUBLISH"),
            saved: drawing.is_some_and(|tab| crate::io::drawing_dir(&tab.scene.document).is_some()),
            // One scan runs at a time, and the project check starts one.
            busy: self.pid_index.scanning(),
            waivers: drawing
                .map(|tab| {
                    let kept = drc::waivers(&tab.scene.document);
                    kept.into_iter()
                        .map(|waiver| (waiver.rule_id, waiver.object_key))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The checks kept for the drawing in front (§3.1): its own, on its tab,
    /// and its project folder's; none on the Start page.
    fn pid_verify_kept(&self) -> (Option<KeptCheck<'_>>, Option<KeptCheck<'_>>) {
        let i = self.active_tab;
        let Some(tab) = self.tabs.get(i).filter(|tab| !tab.is_start) else {
            return (None, None);
        };
        let dir = crate::io::drawing_dir(&tab.scene.document);
        let project = dir.and_then(|dir| self.pid_checks.of_project(dir));
        (
            tab.pid_check
                .as_ref()
                .map(|checked| self.pid_verify_kept_check(checked, i, dir)),
            project.map(|checked| self.pid_verify_kept_check(checked, i, dir)),
        )
    }

    fn pid_verify_kept_check<'a>(
        &self,
        checked: &'a Checked,
        i: usize,
        dir: Option<&Path>,
    ) -> KeptCheck<'a> {
        KeptCheck {
            report: &checked.report,
            serial: checked.serial,
            stale: self.pid_check_stale(checked, i, dir),
        }
    }

    /// Whether a drawing `checked` read changed since (§3.1). A check of the
    /// drawing on tab `i` alone goes stale when that drawing changes; one
    /// that read its project folder `dir`, when any drawing open in that
    /// folder does, one opened after the check counting from its opening.
    /// Waiving follows the result in place and leaves it fresh; undoing that
    /// does not (§4.4).
    fn pid_check_stale(&self, checked: &Checked, i: usize, dir: Option<&Path>) -> bool {
        let changed = |id: u64, revision: u64| checked.revision_at_check(id) != revision;
        let Some(dir) = dir.filter(|_| checked.report.scope.cross_drawing) else {
            return changed(self.tabs[i].id, self.tabs[i].edit_revision);
        };
        let dir = same_file(dir);
        self.tabs
            .iter()
            .filter(|tab| !tab.is_start)
            .filter(|tab| {
                crate::io::drawing_dir(&tab.scene.document)
                    .is_some_and(|other| same_file(other) == dir)
            })
            .any(|tab| changed(tab.id, tab.edit_revision))
    }

    /// Show the panel the way its switch among the dock's panels does:
    /// docked on the right the first time, where the user left it after.
    pub(in crate::app) fn show_pid_verify_panel(&mut self) -> Task<Message> {
        let task = self.dock_set_open(PanelId::PidVerifyDeliver, true);
        self.refresh_pid_verify_view();
        task
    }

    /// `pid_verify_panel` (§3.2): what the panel shows, shown or hidden. The
    /// view is rebuilt first, so it answers for the drawing in front now;
    /// `limit` (200 when omitted, [`ROWS_PER_PAGE`]) and `offset` page each
    /// list.
    pub(in crate::app) fn control_pid_verify_panel(&mut self, req: &Value) -> Value {
        let (limit, offset) = match paging(req) {
            Ok(paging) => paging,
            Err(refused) => return refused,
        };
        self.refresh_pid_verify_view();
        pid_verify_deliver::panel_state(&self.pid_verify_view, limit, offset)
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

    /// What the user did in the panel. Hiding takes the close button's path,
    /// so the controls' positions go with the panel. A filter changed or a
    /// check pressed starts the list over at its first 200 rows.
    pub(in crate::app) fn on_pid_verify(&mut self, msg: PidVerifyMsg) -> Task<Message> {
        let ui = &mut self.pid_verify_ui;
        let task = match msg {
            PidVerifyMsg::Show => return self.show_pid_verify_panel(),
            PidVerifyMsg::Hide => {
                let task = self.on_dock(DockMsg::Close(PanelId::PidVerifyDeliver));
                self.refresh_pid_verify_view();
                return task;
            }
            PidVerifyMsg::PickExport => return self.pid_verify_pick_export(),
            PidVerifyMsg::Tab(tab) => {
                ui.tab = tab;
                Task::none()
            }
            PidVerifyMsg::CheckDrawing => {
                ui.more = 0;
                self.pid_verify_check_drawing();
                Task::none()
            }
            PidVerifyMsg::CheckProject => {
                ui.more = 0;
                self.pid_verify_check_project()
            }
            PidVerifyMsg::Severity(severity) => {
                toggle(&mut ui.severities, severity);
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::Kind(kind) => {
                match kind {
                    Some(kind) => toggle(&mut ui.kinds, kind),
                    None => ui.kinds.clear(),
                }
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::ShowWaived(on) => {
                ui.show_waived = on;
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::Fold(group) => {
                toggle(&mut ui.folded, group);
                Task::none()
            }
            PidVerifyMsg::MoreChecks => {
                ui.more = ui.more.saturating_add(1);
                Task::none()
            }
            PidVerifyMsg::Export(name) => {
                self.pid_verify_export(&name);
                Task::none()
            }
            PidVerifyMsg::Locate(index) => self.pid_verify_locate(index),
            PidVerifyMsg::AskWaiver(index) => {
                let shown = self.pid_verify_view.check.shown.as_ref();
                ui.waiving = shown.map(|shown| Waiving {
                    serial: shown.serial,
                    index,
                    reason: String::new(),
                });
                iced::widget::operation::focus(iced::widget::Id::new(REASON_ID))
            }
            PidVerifyMsg::WaiverReason(typed) => {
                if let Some(waiving) = ui.waiving.as_mut() {
                    waiving.reason = typed;
                }
                Task::none()
            }
            PidVerifyMsg::Waive(index, reason) => {
                self.pid_verify_waive(index, reason);
                Task::none()
            }
            PidVerifyMsg::CancelWaiver => {
                ui.waiving = None;
                Task::none()
            }
            PidVerifyMsg::Unwaive(index) => {
                self.pid_verify_unwaive(index);
                Task::none()
            }
            PidVerifyMsg::StaleWaivers => {
                ui.stale_open = !ui.stale_open;
                Task::none()
            }
            PidVerifyMsg::DropWaiver {
                rule_id,
                object_key,
            } => {
                self.pid_verify_drop_waiver(&rule_id, &object_key);
                Task::none()
            }
        };
        self.refresh_pid_verify_view();
        task
    }

    /// 【检查当前图】 (§4.2): what `pid_check` does with `scope: drawing`, the
    /// result kept on the drawing's tab. A rule pack that does not read is
    /// said in the message bar, under its code, and the result there was
    /// stays (§4.1).
    fn pid_verify_check_drawing(&mut self) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        match self.pidcheck_load_rules(i) {
            Ok(rules) => {
                let checked = self.pidcheck_this_drawing(i, &rules);
                self.tabs[i].pid_check = Some(checked);
                self.pid_verify_ui.notice = None;
            }
            Err((code, why)) => {
                self.pid_verify_ui.notice = Some(Notice::error(Some(code.to_owned()), why));
            }
        }
    }

    /// 【检查全项目】 (§4.2): `PIDCHECK PROJECT` — `PIDINDEX`'s scan of the
    /// drawing's folder, then the check, kept for the folder. What stops it
    /// before the scan is said in the message bar; the scan and the check
    /// say on the command line what they say for the command.
    fn pid_verify_check_project(&mut self) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Task::none();
        }
        match self.pidcheck_prepare(i, None) {
            Ok(check) => {
                self.pid_verify_ui.notice = None;
                self.pidindex_check(i, check)
            }
            Err((code, why)) => {
                self.pid_verify_ui.notice = Some(Notice::error(Some(code.to_owned()), why));
                Task::none()
            }
        }
    }

    /// 【导出…】: ask where to write the result shown, offering
    /// `<drawing>-drc.json` beside the drawing.
    fn pid_verify_pick_export(&self) -> Task<Message> {
        let dir = self
            .tabs
            .get(self.active_tab)
            .filter(|tab| !tab.is_start)
            .and_then(|tab| crate::io::drawing_dir(&tab.scene.document))
            .map(Path::to_path_buf);
        let drawing = self
            .pid_verify_view
            .drawing_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or("PID");
        let name = format!("{drawing}-drc.json");
        Task::perform(
            async move {
                let mut dialog = crate::sys::file_dialog()
                    .set_title("导出检查结果")
                    .set_file_name(&name)
                    .add_filter("JSON", &["json"]);
                if let Some(dir) = dir {
                    dialog = dialog.set_directory(dir);
                }
                dialog
                    .save_file()
                    .await
                    .map(|picked| crate::sys::handle_path(&picked))
            },
            |picked| match picked {
                Some(path) => PidVerifyMsg::Export(path.to_string_lossy().into_owned()).message(),
                None => Message::Noop,
            },
        )
    }

    /// Write the result shown to `name` as `PIDCHECK EXPORT` writes it, and
    /// say in the message bar where it went or why it did not.
    fn pid_verify_export(&mut self, name: &str) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        self.pid_verify_ui.notice = Some(match self.pidcheck_write_export(i, name) {
            Ok(said) => Notice::info(said),
            Err(why) => Notice::error(None, why),
        });
    }

    /// The result at `index` of the check the panel shows for the drawing in
    /// front — the newer of its own and its folder's (§3.1) — and that
    /// check's serial.
    fn pid_verify_result(&self, index: usize) -> Option<(u64, CheckResult)> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return None;
        }
        let checked = self.pidcheck_latest(i)?;
        let result = checked.report.results.get(index)?;
        Some((checked.serial, result.clone()))
    }

    /// A row clicked (§4.3): its object found on the drawing it is on — the
    /// one in front, the tab open on it, or opened — the way `PIDLOCATE`
    /// finds it, without a word on the command line. What came of it is
    /// said in the message bar ([`Self::pid_verify_landed`]).
    fn pid_verify_locate(&mut self, index: usize) -> Task<Message> {
        let Some((serial, result)) = self.pid_verify_result(index) else {
            return Task::none();
        };
        self.pid_verify_ui.located = Some((serial, index));
        let i = self.active_tab;
        if result.object.drawing_id == self.pidcheck_drawing_id(i) {
            let landed = self.pidlocate_result(i, &result);
            self.pid_verify_landed(landed);
            return Task::none();
        }
        let Some(dir) = crate::io::drawing_dir(&self.tabs[i].scene.document) else {
            return Task::none();
        };
        let dir = dir.to_path_buf();
        self.pidlocate_result_elsewhere(&dir, result)
    }

    /// What came of finding a row's object, in the message bar: nothing when
    /// it was found or has no tag of its own to find it by, [`GONE`] when
    /// its tag is no longer on its drawing.
    pub(super) fn pid_verify_landed(&mut self, landed: Landed) {
        self.pid_verify_ui.notice = match landed {
            Landed::Gone => Some(Notice::error(Some(LOCATE_NOT_FOUND.to_owned()), GONE)),
            Landed::Found | Landed::Spot => None,
        };
        self.refresh_pid_verify_view();
    }

    /// Why a row's object was not found — its drawing gone from the folder,
    /// or not opened — in the message bar.
    pub(super) fn pid_verify_not_located(&mut self, code: Option<&str>, why: String) {
        self.pid_verify_ui.notice = Some(Notice::error(code.map(str::to_owned), why));
        self.refresh_pid_verify_view();
    }

    /// 【确认】 under a row's reason box (§4.4): the row waived for `reason`
    /// by the entry `PIDCHECK WAIVE` calls, which follows the result in
    /// place without checking again. What the entry refuses is said in the
    /// message bar and the box stays open with what was typed; waived, the
    /// box closes.
    fn pid_verify_waive(&mut self, index: usize, reason: String) {
        let Some((serial, result)) = self.pid_verify_result(index) else {
            return;
        };
        let i = self.active_tab;
        let added = self.pidcheck_load_rules(i).and_then(|rules| {
            self.pidcheck_add_waiver(i, &rules, &result.rule_id, &result.object_key, &reason)
        });
        let ui = &mut self.pid_verify_ui;
        match added {
            Ok(_) => {
                ui.waiving = None;
                ui.notice = None;
            }
            Err((code, said)) => {
                ui.waiving = Some(Waiving {
                    serial,
                    index,
                    reason,
                });
                ui.notice = Some(Notice::error(Some(code.to_owned()), said));
            }
        }
    }

    /// 【取消豁免】 (§4.4): the row's waiver taken off the drawing in front.
    fn pid_verify_unwaive(&mut self, index: usize) {
        if let Some((_, result)) = self.pid_verify_result(index) {
            self.pid_verify_drop_waiver(&result.rule_id, &result.object_key);
        }
    }

    /// The waiver of `rule_id` on `key` taken off the drawing in front by
    /// the entry `PIDCHECK UNWAIVE` calls — one undo step, the results kept
    /// followed in place — for 【取消豁免】 and for 【删除】 beside a waiver no
    /// result matches. What the entry refuses is said in the message bar.
    fn pid_verify_drop_waiver(&mut self, rule_id: &str, key: &str) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        let dropped = self.pidcheck_remove_waiver(i, rule_id, key);
        self.pid_verify_ui.notice = dropped
            .err()
            .map(|(code, said)| Notice::error(Some(code.to_owned()), said));
    }
}

/// Switch `item` in `list`: off when it is on, on when it is off.
fn toggle<T: PartialEq>(list: &mut Vec<T>, item: T) {
    match list.iter().position(|on| *on == item) {
        Some(at) => {
            list.remove(at);
        }
        None => list.push(item),
    }
}

/// `limit` and `offset` of a `pid_verify_panel` request: rows per list, at
/// least one, and rows skipped in each. Anything but such whole numbers is
/// `invalid_request`.
fn paging(req: &Value) -> Result<(usize, usize), Value> {
    let limit = count(req, "limit", 1, "正整数（每个列表最多给几行）")?;
    let offset = count(req, "offset", 0, "不小于 0 的整数（每个列表跳过几行）")?;
    Ok((limit.unwrap_or(ROWS_PER_PAGE), offset.unwrap_or(0)))
}

/// A whole number of at least `least` that the request may leave out; the
/// refusal says what to `write`.
fn count(req: &Value, key: &str, least: u64, write: &str) -> Result<Option<usize>, Value> {
    match &req[key] {
        Value::Null => Ok(None),
        asked => match asked.as_u64() {
            Some(n) if n >= least => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
            _ => Err(refuse(
                "invalid_request",
                format!("{key} 写{write}，实得 {asked}"),
            )),
        },
    }
}

/// The control channel's refusal.
fn refuse(code: &str, error: impl Into<String>) -> Value {
    json!({ "ok": false, "status": "failed", "code": code, "error": error.into() })
}

#[cfg(test)]
mod tests {
    use crate::app::config::DockSide;
    use crate::app::{Message, OpenCADStudio};
    use crate::pid::drc;
    use crate::pid::drc::builtin_rules::PROJECT_PACK_FILE;
    use crate::ui::dock::{DockMsg, PanelId};
    use crate::ui::window::pid_verify_deliver::{
        Group, PidVerifyMsg, RowView, Tab, BOUNDS_PREFIX, CHECK_DRAWING_ID, CHECK_PROJECT_ID,
        NEVER_CHECKED, PANEL_BOUNDS_ID,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

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

    /// What `pid_verify_panel` answers over the control channel, with the
    /// request's other fields `more` (`,"limit":5` and the like).
    fn panel(app: &mut OpenCADStudio, more: &str) -> Value {
        app.automation_op(&format!(
            r#"{{"protocol":1,"op":"pid_verify_panel"{more}}}"#
        ))
    }

    /// Bring drawing `id` to the front through the control channel, driving
    /// the switch to its end.
    fn activate(app: &mut OpenCADStudio, id: u64, request_id: &str) {
        let (switched, task) = app.control_request(json!({
            "protocol": 1,
            "op": "activate",
            "document_id": id,
            "request_id": request_id,
        }));
        app.drive_headless_task(task).expect("the switch's task");
        assert_eq!(switched["ok"], true, "{switched}");
        assert_eq!(app.tabs[app.active_tab].id, id);
    }

    /// Press `value` in the panel through the `action` op (§3.3), as a
    /// script does, driving whatever it starts to its end.
    fn act(app: &mut OpenCADStudio, value: &str) -> Value {
        static PRESSED: AtomicU64 = AtomicU64::new(0);
        let request_id = format!(
            "pid-verify-action-{}",
            PRESSED.fetch_add(1, Ordering::Relaxed)
        );
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let (answer, task) = app.control_request(json!({
            "protocol": 1,
            "op": "action",
            "name": "pid_verify",
            "value": value,
            "document_id": state["document_id"],
            "request_id": request_id,
        }));
        app.drive_headless_task(task).expect("the action's task");
        if matches!(answer["status"].as_str(), Some("accepted" | "running")) {
            return app
                .control_request(json!({"op": "operation", "request_id": request_id}))
                .0;
        }
        answer
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
    fn pid_verify_panel_reads_the_panel_shown_or_hidden() {
        let mut app = fresh();
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        // Never shown, it answers all the same.
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["visible"], false, "{answer}");
        assert_eq!(answer["tab"], "check", "{answer}");
        assert_eq!(answer["document_id"], state["document_id"], "{answer}");
        // A drawing never saved has no number yet, as the rule check says.
        assert_eq!(answer["drawing_id"], "", "{answer}");
        assert_eq!(answer["writable"], true, "{answer}");
        for block in ["check", "bom", "deliver"] {
            assert!(answer[block].is_object(), "{block}: {answer}");
        }
        for (line, visible, tab) in [
            ("PIDVERIFY BOM", true, "bom"),
            ("PIDVERIFY OFF", false, "bom"),
            ("PIDVERIFY DELIVER", true, "deliver"),
        ] {
            let _ = run(&mut app, line);
            let answer = panel(&mut app, "");
            assert_eq!(answer["visible"], visible, "{line}: {answer}");
            assert_eq!(answer["tab"], tab, "{line}: {answer}");
        }
    }

    #[test]
    fn pid_verify_panel_follows_the_drawing_tab_in_front() {
        let mut app = fresh();
        let first = app.tabs[app.active_tab].id;
        let second = app.push_test_document();
        // Where the files came from, as opening them leaves it. With no
        // project file beside them, each is numbered by its file name.
        let folder =
            std::env::temp_dir().join(format!("ocs-pid-verify-panel-{}", std::process::id()));
        for (id, file) in [(first, "PID-1101.dwg"), (second, "PID-1102.dwg")] {
            let tab = app
                .tabs
                .iter_mut()
                .find(|tab| tab.id == id)
                .expect("the tab");
            tab.scene.document.source_path = Some(folder.join(file).to_string_lossy().into_owned());
        }
        for (request_id, id, number) in [
            ("pid-verify-first", first, "PID-1101"),
            ("pid-verify-second", second, "PID-1102"),
            ("pid-verify-first-again", first, "PID-1101"),
        ] {
            activate(&mut app, id, request_id);
            let answer = panel(&mut app, "");
            assert_eq!(answer["document_id"], id, "{answer}");
            assert_eq!(answer["drawing_id"], number, "{answer}");
        }
    }

    #[test]
    fn pid_verify_panel_says_whether_the_session_may_write() {
        let mut app = fresh();
        for (readonly, writable) in [(false, true), (true, false), (false, true)] {
            app.collab.readonly = readonly;
            let answer = panel(&mut app, "");
            assert_eq!(
                answer["writable"], writable,
                "read-only {readonly}: {answer}"
            );
        }
    }

    #[test]
    fn pid_verify_panel_answers_on_the_start_page() {
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start);
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["visible"], false, "{answer}");
        assert_eq!(answer["document_id"], Value::Null, "{answer}");
        assert_eq!(answer["drawing_id"], Value::Null, "{answer}");
    }

    #[test]
    fn pid_verify_panel_pages_by_whole_numbers_only() {
        let mut app = fresh();
        for more in [
            r#","limit":5"#,
            r#","offset":3"#,
            r#","limit":1,"offset":0"#,
        ] {
            let answer = panel(&mut app, more);
            assert_eq!(answer["ok"], true, "{more}: {answer}");
        }
        for more in [
            r#","limit":0"#,
            r#","limit":-1"#,
            r#","limit":2.5"#,
            r#","limit":"many""#,
            r#","offset":-1"#,
            r#","offset":"next""#,
        ] {
            let answer = panel(&mut app, more);
            assert_eq!(answer["code"], "invalid_request", "{more}: {answer}");
        }
    }

    #[test]
    fn pid_verify_panel_is_a_query_listed_among_the_capabilities() {
        let mut app = fresh();
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let listed = state["capabilities"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "pid_verify_panel"));
        assert!(listed, "{}", state["capabilities"]);
        // A query answers while automation is stopped.
        let stop = json!({
            "protocol": 1,
            "op": "stop",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "client_id": "test",
            "request_id": "pid-verify-stop",
        });
        let stopped = app.automation_op(&stop.to_string());
        assert_eq!(stopped["ok"], true, "{stopped}");
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
    }

    #[test]
    fn pid_verify_action_shows_hides_and_turns_the_panel() {
        let mut app = fresh();
        for (value, visible, tab) in [
            ("show", true, "check"),
            ("tab=bom", true, "bom"),
            ("tab=deliver", true, "deliver"),
            ("hide", false, "deliver"),
            // Hidden, the panel turns all the same, and shows on that tab.
            ("tab=check", false, "check"),
            ("show", true, "check"),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(answer["status"], "completed", "{value}: {answer}");
            // The action only presses; what it did is read off the panel.
            assert_eq!(answer["result"], json!({}), "{value}: {answer}");
            let state = panel(&mut app, "");
            assert_eq!(state["visible"], visible, "{value}: {state}");
            assert_eq!(state["tab"], tab, "{value}: {state}");
        }
        // The first time, it docked on the right, as `PIDVERIFY` docks it.
        assert_eq!(
            app.dock.location(PANEL).map(|(side, _)| side),
            Some(DockSide::Right)
        );
    }

    #[test]
    fn pid_verify_action_refuses_what_the_panel_does_not_have() {
        let mut app = fresh();
        for value in ["open", "show=1", "tab=report"] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], false, "{value}: {answer}");
            assert_eq!(answer["status"], "failed", "{value}: {answer}");
            assert_eq!(answer["code"], "invalid_value", "{value}: {answer}");
            assert!(
                answer["error"]
                    .as_str()
                    .is_some_and(|said| !said.is_empty()),
                "{value}: {answer}"
            );
            assert_eq!(answer.get("reason"), None, "{value}: {answer}");
        }
        // With no value there is nothing to press, as for every action.
        let answer = act(&mut app, "");
        assert_eq!(answer["code"], "invalid_request", "{answer}");
        // Nothing was pressed.
        let state = panel(&mut app, "");
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "check", "{state}");
    }

    #[test]
    fn pid_verify_action_is_open_to_a_read_only_session() {
        let mut app = fresh();
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for value in ["show", "tab=bom", "hide"] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
        }
        let state = panel(&mut app, "");
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "bom", "{state}");
        // Pressing the panel changed nothing in the drawing.
        assert_eq!(undo_steps(&app), steps);
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

    /// A folder of one test's own, removed after it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // Tests run in parallel: a count tells apart those of one process.
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-pid-verify-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("the scratch folder");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A control request of protocol 1 to the drawing in front.
    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        req["request_id"] = json!(format!(
            "pid-verify-request-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        app.automation_op(&req.to_string())
    }

    /// Start `cmd`, feed it `points`, then press Enter.
    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
        let started = request(app, json!({"op": "run", "cmd": cmd}));
        assert_eq!(started["ok"], true, "{cmd}: {started}");
        for [x, y] in points {
            let point =
                json!({"op": "input", "kind": "point", "point": [x, y, 0.0], "space": "wcs"});
            let fed = request(app, point);
            assert_eq!(fed["ok"], true, "{cmd}: {fed}");
        }
        request(app, json!({"op": "input", "kind": "enter"}));
    }

    /// Type `line` at the command line and press Enter, as a person does:
    /// both are messages the program handles.
    fn typed(app: &mut OpenCADStudio, line: &str) {
        let _ = app.update(Message::CommandInput(line.to_owned()));
        let task = app.update(Message::CommandSubmit);
        app.drive_headless_task(task).expect("the command's task");
    }

    /// A rule of the panel tests' own set: `expr` on each object of kind
    /// `target`.
    fn rule(id: &str, target: &str, severity: &str, expr: &str) -> Value {
        json!({
            "id": id, "name": format!("面板验收 {id}"), "set": "面板验收",
            "category": "设计类", "target": target, "severity": severity,
            "expr": expr, "scope": "项目", "source": "新建"
        })
    }

    /// The project rule pack the panel tests check with: the built-in sets
    /// off and three rules of a set of its own, so that a pump and a pipe
    /// line make one error, one warning and one note.
    fn pack() -> Value {
        json!({
            "version": 1,
            "pack": "面板验收",
            "rule_set_version": "T1",
            "sets": [
                {"name": "基础规则", "enabled": false},
                {"name": "工程扩展", "enabled": false},
                {"name": "面板验收", "enabled": true}
            ],
            "rules": [
                rule("DR-201", "管线", "错误", "管道等级 = NULL"),
                rule("DR-202", "设备", "警告", "流量 = NULL"),
                rule("DR-203", "管线", "提示", "设计压力 = NULL")
            ]
        })
    }

    /// A pump and a pipe line saved as PID-1101.dwg in `scratch`, beside
    /// [`pack`]: checked, the line has an error (DR-201) and a note (DR-203)
    /// and the pump a warning (DR-202). The panel is open.
    fn sheet(scratch: &Scratch) -> OpenCADStudio {
        let mut app = fresh();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
        fs::write(scratch.path(PROJECT_PACK_FILE), pack().to_string()).expect("the pack");
        let path = scratch.path("PID-1101.dwg");
        let bytes = crate::io::save_to_bytes(
            app.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("the drawing as DWG");
        fs::write(&path, bytes).expect("the drawing");
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(path.to_string_lossy().into_owned());
        let _ = run(&mut app, "PIDVERIFY");
        app
    }

    /// Waive the note on the line at the command line, keyed as the check
    /// kept on the drawing's tab found it.
    fn waive_the_note(app: &mut OpenCADStudio) {
        let i = app.active_tab;
        let key = app.tabs[i]
            .pid_check
            .as_ref()
            .and_then(|checked| {
                let mut found = checked.report.results.iter();
                found.find(|result| result.rule_id == "DR-203")
            })
            .map(|result| result.object_key.clone())
            .expect("the note in the check kept on the tab");
        let ran = run(app, &format!("PIDCHECK WAIVE DR-203 \"{key}\" 放空管"));
        assert_eq!(ran.error, None, "{}", ran.said);
    }

    /// The rules of the rows the panel state lists, in order.
    fn rows(state: &Value) -> Vec<String> {
        state["check"]["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row["rule_id"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the check the panel draws now is marked stale.
    fn drawn_stale(app: &OpenCADStudio) -> bool {
        let shown = app.pid_verify_view.check.shown.as_ref();
        shown.expect("a check drawn").stale
    }

    /// [`sheet`], with PID-1102.dwg beside it holding a pump of its own:
    /// checked as a project, each pump has a warning (DR-202).
    fn two_sheets(scratch: &Scratch) -> OpenCADStudio {
        let mut other = fresh();
        draw(&mut other, "PIDINSERT pump", &[[0.0, 0.0]]);
        let bytes = crate::io::save_to_bytes(
            other.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("the drawing as DWG");
        fs::write(scratch.path("PID-1102.dwg"), bytes).expect("the drawing");
        sheet(scratch)
    }

    /// The index and the label of the row of rule `rule` on drawing
    /// `drawing` in the panel state.
    fn row_of(state: &Value, rule: &str, drawing: &str) -> (usize, String) {
        let rows = state["check"]["rows"].as_array().expect("the rows");
        let row = rows
            .iter()
            .find(|row| row["rule_id"] == rule && row["drawing_id"] == drawing)
            .unwrap_or_else(|| panic!("{rule} on {drawing}: {state}"));
        let index = row["index"].as_u64().expect("the index");
        (
            usize::try_from(index).expect("an index"),
            row["label"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// The handles `get_selection` reads off the drawing in front.
    fn selection(app: &mut OpenCADStudio) -> Vec<String> {
        let answer = app.automation_op(r#"{"protocol":1,"op":"get_selection"}"#);
        let entities = answer["result"]["entities"].as_array();
        entities
            .expect("the selection")
            .iter()
            .map(|entity| entity["handle"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Where the view of the drawing in front looks.
    fn looking_at(app: &OpenCADStudio) -> glam::DVec3 {
        app.tabs[app.active_tab].scene.camera.borrow().target
    }

    /// Select nothing on the drawing in front and look far away from it.
    fn look_away(app: &mut OpenCADStudio) {
        let i = app.active_tab;
        app.tabs[i].scene.deselect_all();
        app.tabs[i].scene.camera.borrow_mut().target = glam::DVec3::new(5000.0, 5000.0, 0.0);
    }

    /// What the command line said since it was last cleared mentions
    /// `PIDLOCATE`.
    fn pidlocate_said(app: &OpenCADStudio) -> bool {
        let history = &app.command_line.history;
        history.iter().any(|entry| entry.text.contains("PIDLOCATE"))
    }

    /// The rows of the waived group the panel draws now.
    fn drawn_waived(app: &OpenCADStudio) -> Vec<usize> {
        let shown = app.pid_verify_view.check.shown.as_ref();
        let shown = shown.expect("a check drawn");
        let groups = shown.groups.iter();
        groups
            .filter(|group| group.group == Group::Waived)
            .flat_map(|group| &shown.rows[group.first..group.first + group.count])
            .map(|row| row.index)
            .collect()
    }

    /// What is typed in the reason box under the row of `index`, while it
    /// is open.
    fn drawn_reason(app: &OpenCADStudio, index: usize) -> Option<String> {
        let shown = app.pid_verify_view.check.shown.as_ref();
        let mut rows = shown.expect("a check drawn").rows.iter();
        rows.find(|row| row.index == index)
            .and_then(|row| row.reason.clone())
    }

    #[test]
    fn the_rule_check_tab_counts_as_pid_check_does_and_filters_its_list() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        let summary = app.automation_op(r#"{"protocol":1,"op":"pid_check"}"#)["summary"].clone();
        for count in ["错误", "警告", "提示", "已豁免"] {
            assert_eq!(
                state["check"]["counts"][count], summary[count],
                "{count}: {state}"
            );
        }
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 0, "已豁免": 1}),
            "{state}"
        );
        assert_eq!(rows(&state), ["DR-201", "DR-202"]);
        for (value, listed) in [
            ("severity=警告", vec!["DR-201"]),
            ("severity=警告", vec!["DR-201", "DR-202"]),
            ("show_waived=1", vec!["DR-201", "DR-202", "DR-203"]),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(rows(&panel(&mut app, "")), listed, "{value}");
        }
    }

    #[test]
    fn a_check_typed_at_the_command_line_shows_unpressed_and_goes_stale_with_an_edit() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        assert_eq!(app.pid_verify_view.empty, Some(NEVER_CHECKED));
        typed(&mut app, "PIDCHECK");
        // What the panel draws shows it already: nothing pressed, nothing
        // asked of the panel.
        let shown = app
            .pid_verify_view
            .check
            .shown
            .clone()
            .expect("the check drawn");
        let counts = shown.summary;
        assert_eq!((counts.errors, counts.warnings, counts.infos), (1, 1, 1));
        assert!(!shown.stale);
        // Another pump: the check is stale in the view drawn after the next
        // message, and in the panel state.
        draw(&mut app, "PIDINSERT pump", &[[0.0, -60.0]]);
        let _ = app.update(Message::Noop);
        assert!(drawn_stale(&app));
        assert_eq!(panel(&mut app, "")["check"]["stale"], true);
        // Checked again, it is fresh, and has found that pump.
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(!drawn_stale(&app));
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(state["check"]["counts"]["警告"], 2, "{state}");
    }

    #[test]
    fn waiving_leaves_the_check_fresh_and_undoing_the_waiver_does_not() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 1, "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        // The result follows the undo in place, but the panel cannot tell
        // what was undone.
        let _ = app.automation_op(r#"{"op":"undo"}"#);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 0, "{state}");
        assert_eq!(state["check"]["stale"], true, "{state}");
    }

    #[test]
    fn the_project_check_pressed_in_the_panel_is_shown_until_a_drawing_of_the_folder_changes() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let answer = act(&mut app, "check=project");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(app.pid_checks.of_project(&scratch.0).is_some());
        // Newer than the drawing's own, the folder's check is shown.
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        // The scan done, the button is pressable again.
        assert_eq!(state["buttons"]["check_project"], true, "{state}");
        // Waiving keeps it fresh; editing a drawing of the folder does not.
        waive_the_note(&mut app);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 1, "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        draw(&mut app, "PIDINSERT pump", &[[0.0, -60.0]]);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["stale"], true, "{state}");
    }

    #[test]
    fn the_panel_refuses_a_grey_rule_check_button_with_the_reason() {
        // Never saved, the drawing is in no project folder.
        let mut app = fresh();
        let _ = run(&mut app, "PIDVERIFY");
        for (value, reason) in [
            ("check=project", "no_project_dir"),
            ("check_export=drc.json", "no_result"),
            ("severity=警告", "no_result"),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(
                answer["code"], "PANEL_ACTION_UNAVAILABLE",
                "{value}: {answer}"
            );
            assert_eq!(answer["reason"], reason, "{value}: {answer}");
        }
        // The drawing itself checks all the same.
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["has_result"], true, "{state}");
        assert_eq!(state["buttons"]["check_project"], false, "{state}");
        assert_eq!(state["buttons"]["check_export"], true, "{state}");
    }

    #[test]
    fn the_panel_exports_the_check_shown_as_pidcheck_export_writes_it() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let _ = act(&mut app, "check=drawing");
        let answer = act(&mut app, "check_export=panel.json");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        let said = state["notice"]["text"].as_str().unwrap_or_default();
        assert!(said.contains("panel.json"), "{state}");
        let ran = run(&mut app, "PIDCHECK EXPORT command.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let written = |name: &str| fs::read(scratch.path(name)).expect("the file written");
        assert_eq!(written("panel.json"), written("command.json"));
        // What the export refuses is said in the message bar; the press was
        // made all the same.
        app.command_line.last_error = None;
        let answer = act(&mut app, "check_export=panel.txt");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.command_line.last_error, None);
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        assert!(!scratch.path("panel.txt").exists());
    }

    #[test]
    fn a_rule_pack_that_does_not_read_is_said_in_the_message_bar_and_the_check_stays() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let _ = act(&mut app, "check=drawing");
        let kept = panel(&mut app, "")["check"].clone();
        let mut broken = pack();
        broken["rules"][0]["expr"] = json!("管道等级 =");
        fs::write(scratch.path(PROJECT_PACK_FILE), broken.to_string()).expect("the pack");
        for value in ["check=drawing", "check=project"] {
            app.command_line.last_error = None;
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(app.command_line.last_error, None, "{value}");
            let state = panel(&mut app, "");
            assert_eq!(state["notice"]["level"], "error", "{value}: {state}");
            assert_eq!(
                state["notice"]["code"],
                drc::DSL_PARSE_ERROR,
                "{value}: {state}"
            );
            assert_eq!(state["check"], kept, "{value}: {state}");
        }
        // Mended, the next check clears the message.
        fs::write(scratch.path(PROJECT_PACK_FILE), pack().to_string()).expect("the pack");
        let _ = act(&mut app, "check=drawing");
        assert_eq!(panel(&mut app, "")["notice"], Value::Null);
    }

    #[test]
    fn locating_a_row_selects_and_frames_its_object_as_pidlocate_does() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let (pump, tag) = row_of(&panel(&mut app, ""), "DR-202", "PID-1101");
        assert!(!tag.is_empty());
        let ran = run(&mut app, &format!("PIDLOCATE {tag}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        let (selected, framed) = (selection(&mut app), looking_at(&app));
        assert_eq!(selected.len(), 1, "{}", ran.said);
        look_away(&mut app);
        app.command_line.history.clear();
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(selection(&mut app), selected);
        let looking = looking_at(&app);
        assert!(looking.distance(framed) < 1e-6, "{looking} ≠ {framed}");
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["located"], pump, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        // The panel finds it without a word on the command line.
        assert!(!pidlocate_said(&app));
        let answer = act(&mut app, "locate=99");
        assert_eq!(answer["code"], "PANEL_ROW_NOT_FOUND", "{answer}");
    }

    #[test]
    fn locating_a_row_on_another_drawing_opens_it_and_keeps_the_project_check_shown() {
        let scratch = Scratch::new();
        let mut app = two_sheets(&scratch);
        let first = app.active_tab;
        let tabs = app.tabs.len();
        let answer = act(&mut app, "check=project");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        let total = state["check"]["total"].clone();
        let (pump, tag) = row_of(&state, "DR-202", "PID-1102");
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let other = scratch.path("PID-1102.dwg");
        assert_eq!(app.tabs.len(), tabs + 1);
        assert_eq!(app.tab_showing(&other), Some(app.active_tab));
        let selected = selection(&mut app);
        assert_eq!(selected.len(), 1, "{selected:?}");
        let framed = looking_at(&app);
        // The project's check is still the one shown, the row clicked
        // located in it.
        let state = panel(&mut app, "");
        assert_eq!(state["drawing_id"], "PID-1102", "{state}");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["total"], total, "{state}");
        assert_eq!(state["check"]["located"], pump, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        // What PIDLOCATE finds there.
        look_away(&mut app);
        let ran = run(&mut app, &format!("PIDLOCATE {tag} PID-1102"));
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(selection(&mut app), selected, "{}", ran.said);
        assert!(looking_at(&app).distance(framed) < 1e-6, "{}", ran.said);
        // From PID-1101 again: the tab open on it is brought forward, no
        // second one opened.
        let _ = app.update(Message::TabSwitch(first));
        let i = app.tab_showing(&other).expect("the tab on PID-1102");
        app.tabs[i].scene.deselect_all();
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.tabs.len(), tabs + 1);
        assert_eq!(app.active_tab, i);
        assert_eq!(selection(&mut app), selected);
    }

    #[test]
    fn waiving_a_row_in_the_panel_moves_it_to_the_waived_group_without_checking_again() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let checks = app.pid_checks.last_serial();
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        // No reason: the waiver's entry refuses it, said in the message bar,
        // and the reason box stays open.
        let answer = act(&mut app, &format!("waive={note}:"));
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        assert_eq!(state["notice"]["code"], "WAIVER_REASON_REQUIRED", "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        assert_eq!(drawn_reason(&app, note), Some(String::new()));
        // With one: waived at once, counted, nothing checked again.
        let answer = act(&mut app, &format!("waive={note}:放空管"));
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"], Value::Null, "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 0, "已豁免": 1}),
            "{state}"
        );
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(rows(&state), ["DR-201", "DR-202"]);
        assert_eq!(app.pid_checks.last_serial(), checks);
        assert_eq!(drawn_reason(&app, note), None);
        let _ = act(&mut app, "show_waived=1");
        assert_eq!(drawn_waived(&app), [note]);
        // One undo step takes the waiver off the drawing.
        let i = app.active_tab;
        assert_eq!(drc::waivers(&app.tabs[i].scene.document).len(), 1);
        let _ = app.automation_op(r#"{"op":"undo"}"#);
        assert!(drc::waivers(&app.tabs[i].scene.document).is_empty());
    }

    #[test]
    fn unwaiving_a_row_in_the_panel_takes_the_waiver_off_the_drawing() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let _ = act(&mut app, "show_waived=1");
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        let answer = act(&mut app, &format!("unwaive={note}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(drc::waivers(&app.tabs[app.active_tab].scene.document).is_empty());
        let state = panel(&mut app, "");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        assert_eq!(state["check"]["stale"], false, "{state}");
        let answer = act(&mut app, &format!("unwaive={note}"));
        assert_eq!(answer["reason"], "not_waived", "{answer}");
    }

    #[test]
    fn the_panel_refuses_a_grey_waive_button_with_the_reason() {
        let scratch = Scratch::new();
        let mut app = two_sheets(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let _ = act(&mut app, "show_waived=1");
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        let answer = act(&mut app, &format!("waive={note}:放空管"));
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "already_waived", "{answer}");
        let _ = act(&mut app, "check=project");
        let (pump, _) = row_of(&panel(&mut app, ""), "DR-202", "PID-1102");
        let answer = act(&mut app, &format!("waive={pump}:备用泵"));
        assert_eq!(answer["reason"], "not_on_this_drawing", "{answer}");
        // A session that may not write waives nothing.
        app.collab.readonly = true;
        let state = panel(&mut app, "");
        let listed = state["check"]["rows"].as_array().expect("the rows");
        assert!(
            listed.iter().all(|row| row["can_waive"] == false),
            "{state}"
        );
        let (line, _) = row_of(&state, "DR-201", "PID-1101");
        let answer = act(&mut app, &format!("waive={line}:临时线"));
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "readonly", "{answer}");
        let i = app.active_tab;
        assert_eq!(drc::waivers(&app.tabs[i].scene.document).len(), 1);
    }

    #[test]
    fn a_waiver_no_result_matches_is_listed_and_dropped_from_the_panel() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let i = app.active_tab;
        let gone = drc::Waiver {
            rule_id: "DR-203".to_owned(),
            object_key: "管线|L-0999".to_owned(),
            reason: "已拆除".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_700_000,
        };
        drc::set_waivers(&mut app.tabs[i].scene.document, std::slice::from_ref(&gone));
        typed(&mut app, "PIDCHECK");
        assert_eq!(panel(&mut app, "")["check"]["stale_waivers"], 1);
        let _ = app.update(Message::PidVerify(PidVerifyMsg::StaleWaivers));
        let shown = app.pid_verify_view.check.shown.clone();
        let shown = shown.expect("the check drawn");
        assert!(shown.stale_open);
        let listed: Vec<(&str, bool)> = shown
            .stale_waivers
            .iter()
            .map(|waiver| (waiver.object_key.as_str(), waiver.can_drop))
            .collect();
        assert_eq!(listed, [("管线|L-0999", true)]);
        let steps = undo_steps(&app);
        let _ = app.update(Message::PidVerify(PidVerifyMsg::DropWaiver {
            rule_id: gone.rule_id.clone(),
            object_key: gone.object_key.clone(),
        }));
        assert!(drc::waivers(&app.tabs[i].scene.document).is_empty());
        assert_eq!(undo_steps(&app), steps + 1);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["stale_waivers"], 0, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
    }

    #[test]
    fn the_drawn_rule_check_tab_draws_the_reason_box_and_the_stale_waivers() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                let i = app.active_tab;
                let gone = drc::Waiver {
                    rule_id: "DR-203".to_owned(),
                    object_key: "管线|L-0999".to_owned(),
                    reason: "已拆除".to_owned(),
                    by: "张工".to_owned(),
                    at: 1_794_700_000,
                };
                drc::set_waivers(&mut app.tabs[i].scene.document, &[gone]);
                typed(&mut app, "PIDCHECK");
                let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
                for msg in [
                    PidVerifyMsg::StaleWaivers,
                    PidVerifyMsg::Locate(note),
                    PidVerifyMsg::AskWaiver(note),
                    PidVerifyMsg::WaiverReason("放空".to_owned()),
                ] {
                    let task = app.update(Message::PidVerify(msg));
                    app.drive_headless_task(task).expect("the press's task");
                }
                assert_eq!(drawn_reason(&app, note), Some("放空".to_owned()));
                let drawn = drawn_ids(&app);
                let shown = app.pid_verify_view.check.shown.clone();
                let shown = shown.expect("the check drawn");
                assert!(shown.stale_open);
                for row in &shown.rows {
                    assert!(drawn.contains(&row.bounds_id()), "{drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    #[test]
    fn the_drawn_rule_check_tab_reports_its_buttons_and_rows() {
        // As for the panel's body and tabs above.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                typed(&mut app, "PIDCHECK");
                let drawn = drawn_ids(&app);
                let shown = app
                    .pid_verify_view
                    .check
                    .shown
                    .clone()
                    .expect("the check drawn");
                assert_eq!(shown.rows.len(), 3);
                let mut ids = vec![CHECK_DRAWING_ID.to_owned(), CHECK_PROJECT_ID.to_owned()];
                ids.extend(shown.rows.iter().map(RowView::bounds_id));
                for id in ids {
                    assert!(drawn.contains(&id), "{id}: {drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }
}

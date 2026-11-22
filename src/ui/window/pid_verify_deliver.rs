//! 「校验与交付」面板 — docked (`PIDVERIFY`, `SPEC-pid-verify-deliver-panel.md`).
//!
//! One panel with three tabs over the results of the modules before it:
//! **规则检查** (the rule check and its waivers), **材料清单** (the bill of
//! materials) and **快照与发布** (snapshots, the diff of two of them, and
//! publishing). The panel has no logic of its own: each button calls the
//! entry point the command and the automation op call, so the panel and the
//! command line give the same result.
//!
//! The split is the collaboration panel's: the app turns its state into plain
//! data with the pure [`build_view`], and [`view`] only draws that
//! [`VerifyView`]. The interface reads the same `VerifyView`, so what it
//! reports is what a person sees.
//!
//! The texts are Chinese literals, as in the collaboration panel (§9 V2).

use crate::app::Message;
use crate::pid::drc::{self, CheckReport, CheckResult, Scope, ScopeKind, Severity, Summary};
use crate::ui::dock::{Chrome, PanelId};
use crate::ui::wrap_bar::PosReport;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Fill, Theme};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The panel's tabs, in the order they are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Check,
    Bom,
    Deliver,
}

impl Tab {
    pub const ALL: [Self; 3] = [Self::Check, Self::Bom, Self::Deliver];

    /// What the tab reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Check => "规则检查",
            Self::Bom => "材料清单",
            Self::Deliver => "快照与发布",
        }
    }

    /// The tab's name in `PIDVERIFY <tab>`, in the panel state and in the
    /// panel's actions (§3.2, §3.3).
    pub fn key(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Bom => "bom",
            Self::Deliver => "deliver",
        }
    }

    /// The tab `key` names, in any case.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tab| tab.key().eq_ignore_ascii_case(key))
    }

    /// `PosReport` id of the tab (§3.4).
    pub fn bounds_id(self) -> &'static str {
        match self {
            Self::Check => "pid-verify-tab-check",
            Self::Bom => "pid-verify-tab-bom",
            Self::Deliver => "pid-verify-tab-deliver",
        }
    }

    /// What the tab says while it has nothing to show.
    fn empty(self) -> &'static str {
        match self {
            Self::Check => NEVER_CHECKED,
            Self::Bom => "还没有生成材料清单。",
            Self::Deliver => "还没有快照和发布记录。",
        }
    }
}

/// What the rule-check tab says instead of a list (§4.2): never checked,
/// checked and nothing found, and everything found filtered out.
pub const NEVER_CHECKED: &str = "还没有检查过。点上面的按钮开始。";
pub const NOTHING_FOUND: &str = "没有发现问题。";
pub const ALL_FILTERED: &str = "没有符合筛选条件的结果。";

/// What the status line adds while the result shown may be out of date
/// (§3.1).
pub const STALE: &str = "图纸已改动，结果可能过期";

/// What the user did in the panel: one message per button, tag and list
/// header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidVerifyMsg {
    /// Put the panel on screen, as its switch among the dock's panels does.
    Show,
    /// Take it off screen, as its close button does.
    Hide,
    /// Bring a tab forward.
    Tab(Tab),
    /// 【检查当前图】.
    CheckDrawing,
    /// 【检查全项目】.
    CheckProject,
    /// A severity tag: on turns off, off turns on.
    Severity(Severity),
    /// A kind tag, the same way; `None` clears the kind filter.
    Kind(Option<String>),
    /// 「显示已豁免」 on or off.
    ShowWaived(bool),
    /// A group's header: folded opens, open folds.
    Fold(Group),
    /// 【再显示 200 条】 under the rule-check list.
    MoreChecks,
    /// 【导出…】: ask where to write the check result.
    PickExport,
    /// Write the check result to this file, as `PIDCHECK EXPORT` does.
    Export(String),
}

impl PidVerifyMsg {
    pub(crate) fn message(self) -> Message {
        Message::PidVerify(self)
    }
}

/// The rule-check list's groups (§4.2), in the order they are drawn: one per
/// severity, and the waived results' while they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Error,
    Warning,
    Info,
    Waived,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::Error, Self::Warning, Self::Info, Self::Waived];

    /// What its header reads, before the count.
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "错误",
            Self::Warning => "警告",
            Self::Info => "提示",
            Self::Waived => "已豁免",
        }
    }

    /// The group a result is listed in.
    fn of(result: &CheckResult) -> Self {
        match (result.waived, result.severity) {
            (true, _) => Self::Waived,
            (false, Severity::Error) => Self::Error,
            (false, Severity::Warning) => Self::Warning,
            (false, Severity::Info) => Self::Info,
        }
    }

    /// Whether `ui` lists the group: a severity's while its tag is on, the
    /// waived results' while 「显示已豁免」 is.
    fn listed(self, ui: &VerifyUi) -> bool {
        match self {
            Self::Error => ui.severities.contains(&Severity::Error),
            Self::Warning => ui.severities.contains(&Severity::Warning),
            Self::Info => ui.severities.contains(&Severity::Info),
            Self::Waived => ui.show_waived,
        }
    }
}

/// `PosReport` id of the panel body (§3.4).
pub const PANEL_BOUNDS_ID: &str = "pid-verify-panel";

/// What every `PosReport` id of the panel starts with: closing the panel
/// forgets them all.
pub const BOUNDS_PREFIX: &str = "pid-verify-";

/// `PosReport` ids of 【检查当前图】 and 【检查全项目】, and what the id of
/// each row of the rule-check list starts with, its index following (§3.4).
pub const CHECK_DRAWING_ID: &str = "pid-verify-check-drawing";
pub const CHECK_PROJECT_ID: &str = "pid-verify-check-project";
pub const CHECK_ROW_PREFIX: &str = "pid-verify-check-row-";

/// How many rows of each list the panel state returns when the request
/// does not say (`limit`, §3.2), and how many rows each group of the
/// rule-check list draws before 【再显示 200 条】 (§4.2).
pub const ROWS_PER_PAGE: usize = 200;

/// The panel's interface state (§3.1): one for the whole program, so the
/// panel stays as it was left across document switches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VerifyUi {
    pub tab: Tab,
    /// The severity tags switched on: all three at first.
    pub severities: Vec<Severity>,
    /// The kind tags switched on; none lists every kind.
    pub kinds: Vec<String>,
    /// Whether the waived results are listed, as a group of their own.
    pub show_waived: bool,
    /// The groups folded shut.
    pub folded: Vec<Group>,
    /// How many times 【再显示 200 条】 was pressed since the filters last
    /// changed: each group draws 200 rows more for each.
    pub more: usize,
    /// The last message (§3.1): what came of the last thing pressed.
    pub notice: Option<Notice>,
}

impl Default for VerifyUi {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            severities: Severity::ALL.to_vec(),
            kinds: Vec::new(),
            show_waived: false,
            folded: Vec::new(),
            more: 0,
            notice: None,
        }
    }
}

/// The panel's message bar (§3.2 `notice`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Notice {
    /// An error, or only news.
    pub error: bool,
    /// The module's error code, when it named one.
    pub code: Option<String>,
    pub text: String,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            error: false,
            code: None,
            text: text.into(),
        }
    }

    pub fn error(code: Option<String>, text: impl Into<String>) -> Self {
        Self {
            error: true,
            code,
            text: text.into(),
        }
    }

    /// As `notice` reads in the panel state.
    fn state(&self) -> Value {
        json!({
            "level": if self.error { "error" } else { "info" },
            "code": self.code,
            "text": self.text,
        })
    }
}

/// What the panel shows the state of besides its results (§5.1). The
/// program fills it in each time it rebuilds the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// Whether the panel is on screen.
    pub visible: bool,
    /// The drawing tab in front; `None` while the Start page is.
    pub document_id: Option<u64>,
    /// That drawing's number as the rule check names it: the project
    /// file's, else the file name; empty for a drawing never saved.
    pub drawing_id: Option<String>,
    /// Whether the session may change that drawing. Waiving and publishing
    /// are refused when it may not.
    pub writable: bool,
    /// Whether that drawing was saved: only then is it in a project folder
    /// to check.
    pub saved: bool,
    /// Whether something runs in the background (§4.7): the buttons that
    /// would start more are grey meanwhile.
    pub busy: bool,
}

/// A rule check the program keeps (§3.1), lent to [`build_view`].
#[derive(Debug, Clone, Copy)]
pub struct KeptCheck<'a> {
    pub report: &'a CheckReport,
    /// Which check of the program's it was: of two, the one with the larger
    /// serial is the newer.
    pub serial: u64,
    /// Whether a drawing it read changed since (§3.1).
    pub stale: bool,
}

/// The results kept with the drawing in front, on its tab (§3.1, §5.1):
/// its own rule check so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrawingResults<'a> {
    pub check: Option<KeptCheck<'a>>,
}

/// The results kept for the folder that drawing is in (§3.1, §5.1): the
/// project's or a PBS node's rule check so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectResults<'a> {
    pub check: Option<KeptCheck<'a>>,
}

/// Whether each of the panel's buttons can be pressed now (§3.2, §4.8).
/// The buttons on a row say so on the row (`can_waive`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Buttons {
    pub check_drawing: bool,
    pub check_project: bool,
    pub check_export: bool,
    pub bom_drawing: bool,
    pub bom_project: bool,
    pub bom_export: bool,
    pub snapshot_save: bool,
    pub diff: bool,
    pub diff_export: bool,
    pub publish_check: bool,
    pub publish: bool,
}

/// The rule-check tab worked out (§4.2), which its block of the panel state
/// reads (§3.2 `check`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckView {
    /// The check shown — the newer of the drawing's and its project's — and
    /// what the filters leave of it; `None` before either.
    pub shown: Option<Shown>,
    /// The severity tags switched on, in their order.
    pub severities: Vec<Severity>,
    /// The kind tags switched on, of the kinds the check found; none lists
    /// every kind.
    pub kinds_on: Vec<String>,
    pub show_waived: bool,
}

/// What the rule-check tab shows of the check it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Whether a drawing it read changed since.
    pub stale: bool,
    pub scope: ScopeKind,
    /// The status line's words for the scope, as the command line says it:
    /// 「图 PID-1101」「全项目」「PBS 节点 U-100」.
    pub scope_said: String,
    /// Unix seconds.
    pub checked_at: i64,
    pub rule_set_version: String,
    pub elapsed_ms: u64,
    /// The check's own counts, whatever the filters.
    pub summary: Summary,
    /// How many results it found, waived ones included.
    pub found: usize,
    /// The kinds of object it found results on, for the kind tags.
    pub kinds: Vec<String>,
    pub notices: Vec<drc::Notice>,
    /// How many waivers no longer match a result.
    pub stale_waivers: usize,
    /// The groups with rows the filters leave, in order.
    pub groups: Vec<GroupView>,
    /// Every row the filters leave, group after group, folded groups'
    /// included.
    pub rows: Vec<RowView>,
}

/// One group of the rule-check list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupView {
    pub group: Group,
    /// Its rows the filters leave: the header's count.
    pub count: usize,
    /// Where its rows start in [`Shown::rows`].
    pub first: usize,
    /// How many of them are drawn: none while folded, else 200 and 200 more
    /// for each 【再显示 200 条】.
    pub drawn: usize,
    pub folded: bool,
}

impl GroupView {
    /// Whether 【再显示 200 条】 is drawn under it.
    pub fn has_more(&self) -> bool {
        !self.folded && self.drawn < self.count
    }
}

/// One row of the rule-check list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    /// Where the result is in the check's `results[]`: what the panel's
    /// actions name the row by.
    pub index: usize,
    pub rule_id: String,
    pub severity: Severity,
    pub object_key: String,
    pub drawing_id: String,
    /// The object's name (§3.2 `label`): its tag, else 「种类（所属）」.
    pub label: String,
    pub message: String,
    pub fix: String,
    pub fields: Vec<String>,
    pub waived: bool,
    /// Why, by whom and when (Unix seconds) it was waived, on waived rows.
    pub waiver: Option<(String, String, i64)>,
    /// Whether the object is on the drawing in front; when it is not, the
    /// first line names the drawing it is on.
    pub here: bool,
    pub can_waive: bool,
}

impl RowView {
    /// The row's first line (§4.2): rule and object, then the drawing when
    /// the object is on another one.
    pub fn headline(&self) -> String {
        if self.here || self.drawing_id.is_empty() {
            format!("{} {}", self.rule_id, self.label)
        } else {
            format!("{} {} · {}", self.rule_id, self.label, self.drawing_id)
        }
    }

    /// The row's `PosReport` id (§3.4).
    pub fn bounds_id(&self) -> String {
        format!("{CHECK_ROW_PREFIX}{}", self.index)
    }
}

/// What the panel draws, worked out by [`build_view`]: plain data, so the
/// interface can report it and a test can compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyView {
    /// The tab in front.
    pub tab: Tab,
    /// What the tab in front says instead of a list; `None` while it draws
    /// one.
    pub empty: Option<&'static str>,
    /// Whether the panel is on screen.
    pub visible: bool,
    /// The drawing tab the panel shows the results of, and its number
    /// ([`Context`]).
    pub document_id: Option<u64>,
    pub drawing_id: Option<String>,
    /// Whether the session may change that drawing.
    pub writable: bool,
    /// Whether something runs in the background.
    pub busy: bool,
    /// Which buttons can be pressed.
    pub buttons: Buttons,
    /// The message bar.
    pub notice: Option<Notice>,
    /// The rule-check tab.
    pub check: CheckView,
}

impl Default for VerifyView {
    fn default() -> Self {
        build_view(&VerifyUi::default(), None, None, &Context::default())
    }
}

/// Interface state, the results kept with the drawing in front and with its
/// project folder, and the program's state around them → what to draw
/// (§4.1): which check is shown, what the filters leave of it and how it is
/// grouped, what each row is called, which buttons can be pressed.
pub fn build_view(
    ui: &VerifyUi,
    drawing: Option<&DrawingResults<'_>>,
    project: Option<&ProjectResults<'_>>,
    context: &Context,
) -> VerifyView {
    let kept = newer(
        drawing.and_then(|results| results.check),
        project.and_then(|results| results.check),
    );
    let check = check_view(ui, kept, context);
    let empty = match ui.tab {
        Tab::Check => check_empty(&check),
        tab => Some(tab.empty()),
    };
    VerifyView {
        tab: ui.tab,
        empty,
        visible: context.visible,
        document_id: context.document_id,
        drawing_id: context.drawing_id.clone(),
        writable: context.writable,
        busy: context.busy,
        buttons: buttons(context, &check),
        notice: ui.notice.clone(),
        check,
    }
}

/// Of the drawing's check and its project's, the newer (§3.1).
fn newer<'a>(
    drawing: Option<KeptCheck<'a>>,
    project: Option<KeptCheck<'a>>,
) -> Option<KeptCheck<'a>> {
    match (drawing, project) {
        (Some(drawing), Some(project)) if project.serial > drawing.serial => Some(project),
        (Some(drawing), _) => Some(drawing),
        (None, project) => project,
    }
}

/// Which buttons can be pressed (§4.8). Checking needs a drawing in front,
/// checking its project a saved one, and neither starts while something
/// runs; exporting needs a result shown, whatever runs.
fn buttons(context: &Context, check: &CheckView) -> Buttons {
    let drawing = context.document_id.is_some();
    Buttons {
        check_drawing: drawing && !context.busy,
        check_project: drawing && context.saved && !context.busy,
        check_export: check.shown.is_some(),
        ..Buttons::default()
    }
}

/// The rule-check tab of `kept` under the filters of `ui` (§4.2). Kind tags
/// switched on for kinds this check did not find filter nothing; when none
/// of the kinds switched on was found, every kind is listed.
fn check_view(ui: &VerifyUi, kept: Option<KeptCheck<'_>>, context: &Context) -> CheckView {
    let severities: Vec<Severity> = Severity::ALL
        .into_iter()
        .filter(|severity| ui.severities.contains(severity))
        .collect();
    let Some(kept) = kept else {
        return CheckView {
            shown: None,
            severities,
            kinds_on: Vec::new(),
            show_waived: ui.show_waived,
        };
    };
    let report = kept.report;
    let kinds: Vec<String> = report
        .results
        .iter()
        .map(|result| result.object.kind.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let kinds_on: Vec<String> = kinds
        .iter()
        .filter(|kind| ui.kinds.contains(kind))
        .cloned()
        .collect();
    let drawn = ROWS_PER_PAGE.saturating_mul(ui.more.saturating_add(1));
    let mut groups = Vec::new();
    let mut rows = Vec::new();
    for group in Group::ALL.into_iter().filter(|group| group.listed(ui)) {
        let first = rows.len();
        rows.extend(
            report
                .results
                .iter()
                .enumerate()
                .filter(|(_, result)| {
                    Group::of(result) == group
                        && (kinds_on.is_empty() || kinds_on.contains(&result.object.kind))
                })
                .map(|(index, result)| row_view(index, result, context)),
        );
        let count = rows.len() - first;
        if count == 0 {
            continue;
        }
        let folded = ui.folded.contains(&group);
        groups.push(GroupView {
            group,
            count,
            first,
            drawn: if folded { 0 } else { count.min(drawn) },
            folded,
        });
    }
    CheckView {
        shown: Some(Shown {
            stale: kept.stale,
            scope: report.scope.kind,
            scope_said: scope_said(&report.scope),
            checked_at: report.checked_at,
            rule_set_version: report.rule_set_version.clone(),
            elapsed_ms: report.elapsed_ms,
            summary: report.summary,
            found: report.results.len(),
            kinds,
            notices: report.notices.clone(),
            stale_waivers: report.stale_waivers.len(),
            groups,
            rows,
        }),
        severities,
        kinds_on,
        show_waived: ui.show_waived,
    }
}

/// What the rule-check tab says instead of a list (§4.2).
fn check_empty(check: &CheckView) -> Option<&'static str> {
    match &check.shown {
        None => Some(NEVER_CHECKED),
        Some(shown) if shown.found == 0 => Some(NOTHING_FOUND),
        Some(shown) if shown.rows.is_empty() => Some(ALL_FILTERED),
        Some(_) => None,
    }
}

/// The row of the result at `index`. It can be waived where the session may
/// write, the object is on the drawing in front and it is not waived
/// already (§4.8).
fn row_view(index: usize, result: &CheckResult, context: &Context) -> RowView {
    let here = context.drawing_id.as_deref() == Some(result.object.drawing_id.as_str());
    RowView {
        index,
        rule_id: result.rule_id.clone(),
        severity: result.severity,
        object_key: result.object_key.clone(),
        drawing_id: result.object.drawing_id.clone(),
        label: label_of(result),
        message: result.message.clone(),
        fix: result.fix.clone(),
        fields: result.fields.clone(),
        waived: result.waived,
        waiver: result
            .waiver
            .as_ref()
            .map(|waiver| (waiver.reason.clone(), waiver.by.clone(), waiver.at)),
        here,
        can_waive: context.writable && here && !result.waived,
    }
}

/// The object's name on a row (§3.2 `label`): its tag; without one
/// 「种类（所属）」; with neither, its kind.
fn label_of(result: &CheckResult) -> String {
    let object = &result.object;
    match (object.tag.as_deref(), object.owner.as_deref()) {
        (Some(tag), _) if !tag.is_empty() => tag.to_owned(),
        (_, Some(owner)) if !owner.is_empty() => format!("{}（{owner}）", object.kind),
        _ => object.kind.clone(),
    }
}

/// What a check looked at, as the command line says it.
fn scope_said(scope: &Scope) -> String {
    match scope.kind {
        ScopeKind::Drawing => match scope.drawing_id.as_deref() {
            Some(id) if !id.is_empty() => format!("图 {id}"),
            _ => "当前图".to_owned(),
        },
        ScopeKind::Pbs => format!("PBS 节点 {}", scope.pbs.as_deref().unwrap_or_default()),
        ScopeKind::Project => "全项目".to_owned(),
    }
}

/// Unix seconds as the UTC date and minute, marked UTC, as the snapshots
/// write them: the standard library has no local time zone.
fn utc_said(seconds: i64) -> String {
    let minutes = seconds.rem_euclid(86_400) / 60;
    // Days since 1970-01-01 as a civil date (Howard Hinnant's
    // civil_from_days), written out here: `ui` may not reach up into
    // `patreon` for it.
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// The panel state (§3.2) the `pid_verify_panel` op answers, read off the
/// same view the panel draws, shown or hidden. Each list gives `limit` rows
/// from `offset` on, while its `total` counts them all. Nothing reports its
/// progress yet (`busy`). `deliver.publish` stays `null` until the publish
/// half is drawn: it is read from disk and has no empty form.
pub fn panel_state(view: &VerifyView, limit: usize, offset: usize) -> Value {
    json!({
        "ok": true,
        "visible": view.visible,
        "tab": view.tab.key(),
        "document_id": view.document_id,
        "drawing_id": view.drawing_id,
        "writable": view.writable,
        "busy": null,
        "notice": view.notice.as_ref().map(Notice::state),
        "buttons": view.buttons,
        "check": paged(check_state(&view.check), limit, offset),
        "bom": paged(no_bom(), limit, offset),
        "deliver": {"snapshots": [], "picked": [], "diff": null, "publish": null}
    })
}

/// The `check` block (§3.2), read off the rule-check tab as worked out: the
/// counts are the check's own, whatever the filters, and `rows` are all the
/// filters leave, folded groups' included.
fn check_state(check: &CheckView) -> Value {
    let filters = json!({
        "severity": check.severities.iter().map(|severity| severity.name()).collect::<Vec<_>>(),
        "kinds": check.kinds_on,
        "show_waived": check.show_waived,
    });
    let Some(shown) = &check.shown else {
        return json!({
            "has_result": false,
            "stale": false,
            "scope": null,
            "checked_at": null,
            "rule_set_version": null,
            "counts": {"错误": 0, "警告": 0, "提示": 0, "已豁免": 0},
            "filters": filters,
            "kinds": [],
            "notices": [],
            "stale_waivers": 0,
            "total": 0,
            "rows": [],
            "located": null
        });
    };
    let counts = shown.summary;
    json!({
        "has_result": true,
        "stale": shown.stale,
        "scope": scope_key(shown.scope),
        "checked_at": shown.checked_at,
        "rule_set_version": shown.rule_set_version,
        "counts": {
            "错误": counts.errors,
            "警告": counts.warnings,
            "提示": counts.infos,
            "已豁免": counts.waived
        },
        "filters": filters,
        "kinds": shown.kinds,
        "notices": shown.notices,
        "stale_waivers": shown.stale_waivers,
        "total": shown.rows.len(),
        "rows": shown.rows.iter().map(row_state).collect::<Vec<_>>(),
        "located": null
    })
}

/// One row as `check.rows[]` gives it (§3.2).
fn row_state(row: &RowView) -> Value {
    json!({
        "index": row.index,
        "rule_id": row.rule_id,
        "severity": row.severity.name(),
        "object_key": row.object_key,
        "drawing_id": row.drawing_id,
        "label": row.label,
        "message": row.message,
        "fields": row.fields,
        "waived": row.waived,
        "can_waive": row.can_waive
    })
}

/// The scope as `check.scope` names it.
fn scope_key(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Drawing => "drawing",
        ScopeKind::Pbs => "pbs",
        ScopeKind::Project => "project",
    }
}

/// The `bom` block before any bill of materials is made.
fn no_bom() -> Value {
    json!({
        "has_result": false,
        "stale": false,
        "scope": null,
        "search": "",
        "error": null,
        "total": 0,
        "groups": [],
        "rows": []
    })
}

/// `block` with its `rows` cut to `limit` of them from `offset` on.
fn paged(mut block: Value, limit: usize, offset: usize) -> Value {
    if let Some(rows) = block.get_mut("rows").and_then(Value::as_array_mut) {
        let page: Vec<Value> = rows.drain(..).skip(offset).take(limit).collect();
        *rows = page;
    }
    block
}

/// Why an action (§3.3) pressed nothing (§3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// The button is grey now; `reason` is one of §3.7's (`busy`,
    /// `readonly`, `no_result` …).
    Unavailable { reason: &'static str },
    /// The row index, snapshot number or revision named is not in the
    /// panel's list now; the text says which.
    RowNotFound(String),
    /// The key is not one the panel has, or its value is not written the
    /// way the key takes it; the text says what to write.
    InvalidValue(String),
}

impl PanelError {
    /// The code the interface answers with.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "PANEL_ACTION_UNAVAILABLE",
            Self::RowNotFound(_) => "PANEL_ROW_NOT_FOUND",
            Self::InvalidValue(_) => "invalid_value",
        }
    }

    /// Why the button is grey; only `PANEL_ACTION_UNAVAILABLE` says.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Unavailable { reason } => Some(*reason),
            Self::RowNotFound(_) | Self::InvalidValue(_) => None,
        }
    }
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { reason } => write!(f, "按钮现在是灰的，没有按下去（{reason}）"),
            Self::RowNotFound(said) | Self::InvalidValue(said) => f.write_str(said),
        }
    }
}

/// The `value` of an `action` op named `pid_verify` (§3.3), one key or
/// key=value, as the message pressing that control in the panel sends.
/// `view` is what the panel draws now, so a key whose button is grey
/// there, or whose row is not in its list, is refused rather than pressed.
/// A value is read in this order: how it is written, then whether its
/// button is grey, then whether what it names is in the panel's lists.
pub fn parse_action(value: &str, view: &VerifyView) -> Result<PidVerifyMsg, PanelError> {
    let (key, written) = match value.split_once('=') {
        Some((key, written)) => (key, Some(written)),
        None => (value, None),
    };
    let invalid =
        |write: &str| PanelError::InvalidValue(format!("{key} 写{write}，实得 \"{value}\""));
    let shown = view.check.shown.as_ref();
    let checked = || {
        shown.ok_or(PanelError::Unavailable {
            reason: "no_result",
        })
    };
    match key {
        "show" | "hide" if written.is_some() => Err(PanelError::InvalidValue(format!(
            "{key} 不带值，实得 \"{value}\""
        ))),
        "show" => Ok(PidVerifyMsg::Show),
        "hide" => Ok(PidVerifyMsg::Hide),
        "tab" => {
            let name = written.unwrap_or_default();
            Tab::from_key(name).map(PidVerifyMsg::Tab).ok_or_else(|| {
                PanelError::InvalidValue(format!(
                    "tab 写 check、bom、deliver 中的一个，实得 \"{name}\""
                ))
            })
        }
        "check" => {
            let (pressed, pressable) = match written {
                Some("drawing") => (PidVerifyMsg::CheckDrawing, view.buttons.check_drawing),
                Some("project") => (PidVerifyMsg::CheckProject, view.buttons.check_project),
                _ => return Err(invalid(" drawing 或 project")),
            };
            if pressable {
                return Ok(pressed);
            }
            let reason = if view.document_id.is_none() {
                "no_drawing"
            } else if view.busy {
                "busy"
            } else {
                "no_project_dir"
            };
            Err(PanelError::Unavailable { reason })
        }
        "severity" => {
            let severity = written
                .and_then(Severity::from_name)
                .ok_or_else(|| invalid(" 错误、警告、提示 中的一个"))?;
            checked()?;
            Ok(PidVerifyMsg::Severity(severity))
        }
        "kind" => {
            let kind = written.ok_or_else(|| invalid("成 kind=<种类>，或者 kind= 清掉种类筛选"))?;
            let found = &checked()?.kinds;
            if kind.is_empty() {
                return Ok(PidVerifyMsg::Kind(None));
            }
            if !found.iter().any(|known| known == kind) {
                return Err(PanelError::InvalidValue(format!(
                    "这次检查的结果里没有种类「{kind}」；有的是：{}",
                    found.join("、")
                )));
            }
            Ok(PidVerifyMsg::Kind(Some(kind.to_owned())))
        }
        "show_waived" => {
            let on = match written {
                Some("1") => true,
                Some("0") => false,
                _ => return Err(invalid(" 1 或 0")),
            };
            checked()?;
            Ok(PidVerifyMsg::ShowWaived(on))
        }
        "check_export" => {
            let path = written
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| invalid("成 check_export=<文件>"))?;
            checked()?;
            Ok(PidVerifyMsg::Export(path.to_owned()))
        }
        "more" => {
            if written != Some("check") {
                return Err(invalid(" check（规则检查的列表再显示 200 条）"));
            }
            checked()?;
            Ok(PidVerifyMsg::MoreChecks)
        }
        _ => Err(PanelError::InvalidValue(format!(
            "面板没有 \"{key}\" 这个键"
        ))),
    }
}

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
    }
}

fn warning(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().warning.base.color),
    }
}

fn danger(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().danger.base.color),
    }
}

/// Build the docked panel: the tab strip, then the tab in front.
pub fn view<'a>(view: &'a VerifyView, width: f32, chrome: Chrome) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(
        PanelId::PidVerifyDeliver,
        PanelId::PidVerifyDeliver.title().to_string(),
        chrome,
    );
    let mut tabs = row![].spacing(2);
    for tab in Tab::ALL {
        let selected = tab == view.tab;
        tabs = tabs.push(PosReport::new(
            tab.bounds_id(),
            button(text(tab.label()).size(11))
                .on_press(PidVerifyMsg::Tab(tab).message())
                .padding([3, 8])
                .style(move |theme: &Theme, status| {
                    if selected {
                        button::primary(theme, status)
                    } else {
                        button::secondary(theme, status)
                    }
                }),
        ));
    }
    let content = match view.tab {
        Tab::Check => check_tab(view),
        Tab::Bom | Tab::Deliver => empty_text(view.empty.unwrap_or_default()),
    };
    let body = column![tabs, content].spacing(6).height(Fill);
    let reported = PosReport::new(PANEL_BOUNDS_ID, body);
    crate::ui::dock::frame(title_bar, reported, width)
}

/// What a tab says instead of a list, in the middle of it.
fn empty_text(said: &str) -> Element<'_, Message> {
    container(text(said).size(12).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill)
        .into()
}

/// The rule-check tab (§4.2): its buttons and the message bar, then what
/// the check shown found — or what the tab says instead.
fn check_tab(view: &VerifyView) -> Element<'_, Message> {
    let buttons = view.buttons;
    let actions = row![
        PosReport::new(
            CHECK_DRAWING_ID,
            small_button(
                "检查当前图",
                buttons.check_drawing.then_some(PidVerifyMsg::CheckDrawing)
            ),
        ),
        PosReport::new(
            CHECK_PROJECT_ID,
            small_button(
                "检查全项目",
                buttons.check_project.then_some(PidVerifyMsg::CheckProject)
            ),
        ),
        small_button(
            "导出…",
            buttons.check_export.then_some(PidVerifyMsg::PickExport)
        ),
    ]
    .spacing(4);
    let mut tab = column![actions].spacing(6).height(Fill);
    if let Some(notice) = &view.notice {
        tab = tab.push(notice_line(notice));
    }
    let check = &view.check;
    let Some(shown) = &check.shown else {
        return tab
            .push(empty_text(view.empty.unwrap_or(NEVER_CHECKED)))
            .into();
    };
    tab = tab.push(text(status_line(shown)).size(11).style(muted));
    if shown.stale {
        tab = tab.push(text(STALE).size(11).style(warning));
    }
    let counts = shown.summary;
    let mut severities = row![].spacing(4);
    for (severity, count) in [
        (Severity::Error, counts.errors),
        (Severity::Warning, counts.warnings),
        (Severity::Info, counts.infos),
    ] {
        severities = severities.push(tag(
            format!("{} {count}", severity.name()),
            check.severities.contains(&severity),
            PidVerifyMsg::Severity(severity),
        ));
    }
    severities = severities.push(tag(
        format!("显示已豁免（{}）", counts.waived),
        check.show_waived,
        PidVerifyMsg::ShowWaived(!check.show_waived),
    ));
    tab = tab.push(severities.wrap().vertical_spacing(4));
    if !shown.kinds.is_empty() {
        let mut kinds = row![].spacing(4);
        for kind in &shown.kinds {
            kinds = kinds.push(tag(
                kind.clone(),
                check.kinds_on.contains(kind),
                PidVerifyMsg::Kind(Some(kind.clone())),
            ));
        }
        tab = tab.push(kinds.wrap().vertical_spacing(4));
    }
    for notice in &shown.notices {
        tab = tab.push(text(notice.message.as_str()).size(11).style(muted));
    }
    if shown.stale_waivers > 0 {
        tab = tab.push(
            text(format!("有 {} 条豁免已经对不上", shown.stale_waivers))
                .size(11)
                .style(warning),
        );
    }
    match view.empty {
        Some(said) => tab.push(empty_text(said)).into(),
        None => tab.push(scrollable(check_list(shown)).height(Fill)).into(),
    }
}

/// The status line (§4.2): scope, when, rule version, how long.
fn status_line(shown: &Shown) -> String {
    format!(
        "{} · {} · 规则 {} · 用时 {} ms",
        shown.scope_said,
        utc_said(shown.checked_at),
        shown.rule_set_version,
        shown.elapsed_ms
    )
}

/// The message bar: an error in the error colour with its code, news
/// muted.
fn notice_line(notice: &Notice) -> Element<'_, Message> {
    let said = match &notice.code {
        Some(code) if !notice.text.contains(code.as_str()) => format!("{code}：{}", notice.text),
        _ => notice.text.clone(),
    };
    let style: fn(&Theme) -> iced::widget::text::Style = if notice.error { danger } else { muted };
    text(said).size(11).style(style).into()
}

/// A filter tag, drawn pressed while it is on.
fn tag<'a>(label: String, on: bool, press: PidVerifyMsg) -> Element<'a, Message> {
    button(text(label).size(11))
        .on_press(press.message())
        .padding([2, 6])
        .style(move |theme: &Theme, status| {
            if on {
                button::primary(theme, status)
            } else {
                button::secondary(theme, status)
            }
        })
        .into()
}

/// One of the panel's buttons, grey while `press` is `None`.
fn small_button(label: &str, press: Option<PidVerifyMsg>) -> Element<'_, Message> {
    button(text(label).size(11))
        .on_press_maybe(press.map(PidVerifyMsg::message))
        .padding([3, 8])
        .style(button::secondary)
        .into()
}

/// The list (§4.2): each group's header, which folds it, its rows drawn,
/// and 【再显示 200 条】 while it has more.
fn check_list(shown: &Shown) -> Element<'_, Message> {
    let mut list = column![].spacing(4).width(Fill);
    for group in &shown.groups {
        let arrow = if group.folded { "▶" } else { "▼" };
        list = list.push(
            button(
                text(format!(
                    "{arrow} {}（{}）",
                    group.group.label(),
                    group.count
                ))
                .size(12),
            )
            .on_press(PidVerifyMsg::Fold(group.group).message())
            .padding([2, 4])
            .style(button::text),
        );
        for row in &shown.rows[group.first..group.first + group.drawn] {
            list = list.push(PosReport::owned(row.bounds_id(), result_row(row)));
        }
        if group.has_more() {
            list = list.push(small_button(
                "再显示 200 条",
                Some(PidVerifyMsg::MoreChecks),
            ));
        }
    }
    list.into()
}

/// One result (§4.2): rule and object, the message, the fix in a lighter
/// colour, and on a waived row why, by whom and when.
fn result_row(row: &RowView) -> Element<'_, Message> {
    let mut lines = column![
        text(row.headline()).size(12),
        text(row.message.as_str()).size(12)
    ]
    .spacing(1);
    if !row.fix.is_empty() {
        lines = lines.push(text(row.fix.as_str()).size(11).style(muted));
    }
    if let Some((reason, by, at)) = &row.waiver {
        lines = lines.push(
            text(format!("豁免：{reason}（{by}，{}）", utc_said(*at)))
                .size(11)
                .style(muted),
        );
    }
    container(lines).padding([2, 8]).width(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_starts_on_the_rule_check_tab() {
        let view = VerifyView::default();
        assert_eq!(view.tab, Tab::Check);
        assert_eq!(view.empty, Some("还没有检查过。点上面的按钮开始。"));
    }

    #[test]
    fn what_is_drawn_follows_the_tab() {
        let drawn: Vec<(Tab, Option<&str>)> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let ui = VerifyUi {
                    tab,
                    ..VerifyUi::default()
                };
                let view = build_view(&ui, None, None, &Context::default());
                (view.tab, view.empty)
            })
            .collect();
        assert_eq!(
            drawn,
            [
                (Tab::Check, Some("还没有检查过。点上面的按钮开始。")),
                (Tab::Bom, Some("还没有生成材料清单。")),
                (Tab::Deliver, Some("还没有快照和发布记录。")),
            ]
        );
    }

    #[test]
    fn the_tabs_read_as_the_spec_names_them() {
        let labels: Vec<&str> = Tab::ALL.into_iter().map(Tab::label).collect();
        assert_eq!(labels, ["规则检查", "材料清单", "快照与发布"]);
    }

    #[test]
    fn the_controls_report_under_the_ids_the_spec_gives_them() {
        assert!(PANEL_BOUNDS_ID.starts_with(BOUNDS_PREFIX));
        let keys: Vec<&str> = Tab::ALL.into_iter().map(Tab::key).collect();
        assert_eq!(keys, ["check", "bom", "deliver"]);
        let ids: Vec<&str> = Tab::ALL.into_iter().map(Tab::bounds_id).collect();
        assert_eq!(
            ids,
            [
                "pid-verify-tab-check",
                "pid-verify-tab-bom",
                "pid-verify-tab-deliver",
            ]
        );
        assert_eq!(
            [CHECK_DRAWING_ID, CHECK_PROJECT_ID],
            ["pid-verify-check-drawing", "pid-verify-check-project"]
        );
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(shown.rows[2].bounds_id(), "pid-verify-check-row-2");
        for id in [CHECK_DRAWING_ID, CHECK_PROJECT_ID, CHECK_ROW_PREFIX] {
            assert!(id.starts_with(BOUNDS_PREFIX), "{id}");
        }
    }

    /// A drawing in front that the session may not change, never saved, the
    /// panel on screen.
    fn read_only_in_front() -> Context {
        Context {
            visible: true,
            document_id: Some(7),
            drawing_id: Some("PID-1101".to_owned()),
            writable: false,
            saved: false,
            busy: false,
        }
    }

    #[test]
    fn pid_verify_panel_answers_the_common_fields_and_the_three_blocks() {
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        let view = build_view(&ui, None, None, &read_only_in_front());
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        let mut keys: Vec<&str> = state
            .as_object()
            .map(|fields| fields.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort_unstable();
        let mut common = [
            "ok",
            "visible",
            "tab",
            "document_id",
            "drawing_id",
            "writable",
            "busy",
            "notice",
            "buttons",
            "check",
            "bom",
            "deliver",
        ];
        common.sort_unstable();
        assert_eq!(keys, common, "{state}");
        assert_eq!(state["ok"], true);
        assert_eq!(state["visible"], true);
        assert_eq!(state["tab"], "bom");
        assert_eq!(state["document_id"], 7);
        assert_eq!(state["drawing_id"], "PID-1101");
        assert_eq!(state["writable"], false);
        assert_eq!(state["busy"], Value::Null);
        assert_eq!(state["notice"], Value::Null);
        // Only 【检查当前图】 can be pressed: the drawing was never saved,
        // nothing is checked yet and the other tabs have no buttons yet.
        let mut buttons: Vec<(&str, Option<bool>)> = state["buttons"]
            .as_object()
            .map(|named| {
                named
                    .iter()
                    .map(|(name, can)| (name.as_str(), can.as_bool()))
                    .collect()
            })
            .unwrap_or_default();
        buttons.sort_unstable();
        let mut names = [
            "check_drawing",
            "check_project",
            "check_export",
            "bom_drawing",
            "bom_project",
            "bom_export",
            "snapshot_save",
            "diff",
            "diff_export",
            "publish_check",
            "publish",
        ];
        names.sort_unstable();
        let expected: Vec<(&str, Option<bool>)> = names
            .into_iter()
            .map(|name| (name, Some(name == "check_drawing")))
            .collect();
        assert_eq!(buttons, expected, "{state}");
        // Nothing checked, listed or kept yet.
        assert_eq!(
            state["check"],
            json!({
                "has_result": false,
                "stale": false,
                "scope": null,
                "checked_at": null,
                "rule_set_version": null,
                "counts": { "错误": 0, "警告": 0, "提示": 0, "已豁免": 0 },
                "filters": {
                    "severity": ["错误", "警告", "提示"],
                    "kinds": [],
                    "show_waived": false
                },
                "kinds": [],
                "notices": [],
                "stale_waivers": 0,
                "total": 0,
                "rows": [],
                "located": null
            })
        );
        assert_eq!(
            state["bom"],
            json!({
                "has_result": false,
                "stale": false,
                "scope": null,
                "search": "",
                "error": null,
                "total": 0,
                "groups": [],
                "rows": []
            })
        );
        assert_eq!(
            state["deliver"],
            json!({ "snapshots": [], "picked": [], "diff": null, "publish": null })
        );
    }

    #[test]
    fn pid_verify_panel_with_no_drawing_in_front_names_none() {
        let state = panel_state(&VerifyView::default(), ROWS_PER_PAGE, 0);
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "check", "{state}");
        assert_eq!(state["document_id"], Value::Null, "{state}");
        assert_eq!(state["drawing_id"], Value::Null, "{state}");
    }

    #[test]
    fn pid_verify_panel_pages_each_list() {
        let block = json!({ "total": 5, "rows": [0, 1, 2, 3, 4] });
        assert_eq!(
            paged(block.clone(), 2, 1),
            json!({ "total": 5, "rows": [1, 2] })
        );
        assert_eq!(paged(block.clone(), 3, 4)["rows"], json!([4]));
        assert_eq!(paged(block.clone(), 3, 9)["rows"], json!([]));
        assert_eq!(paged(block.clone(), ROWS_PER_PAGE, 0), block);
        assert_eq!(ROWS_PER_PAGE, 200);
    }

    #[test]
    fn pid_verify_action_reads_show_hide_and_the_tabs() {
        let view = VerifyView::default();
        for (value, pressed) in [
            ("show", PidVerifyMsg::Show),
            ("hide", PidVerifyMsg::Hide),
            ("tab=check", PidVerifyMsg::Tab(Tab::Check)),
            ("tab=bom", PidVerifyMsg::Tab(Tab::Bom)),
            ("tab=deliver", PidVerifyMsg::Tab(Tab::Deliver)),
            // The tab is named in any case, as `PIDVERIFY` takes it.
            ("tab=Deliver", PidVerifyMsg::Tab(Tab::Deliver)),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_names_a_wrong_key_or_value_invalid() {
        let view = VerifyView::default();
        for value in [
            "",
            "open",
            "SHOW",
            "show=1",
            "hide=",
            "tab",
            "tab=",
            "tab=report",
            "tab=bom=1",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert_eq!(refused.reason(), None, "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        // The refusal says what to write.
        assert_eq!(
            parse_action("tab=report", &view).map_err(|refused| refused.to_string()),
            Err("tab 写 check、bom、deliver 中的一个，实得 \"report\"".to_owned())
        );
    }

    #[test]
    fn pid_verify_action_errors_carry_the_codes_of_section_3_7() {
        let grey = PanelError::Unavailable { reason: "busy" };
        assert_eq!(
            (grey.code(), grey.reason()),
            ("PANEL_ACTION_UNAVAILABLE", Some("busy"))
        );
        assert!(grey.to_string().contains("busy"), "{grey}");
        let gone = PanelError::RowNotFound("规则检查的列表里没有下标 7".to_owned());
        assert_eq!((gone.code(), gone.reason()), ("PANEL_ROW_NOT_FOUND", None));
        assert_eq!(gone.to_string(), "规则检查的列表里没有下标 7");
        let wrong = PanelError::InvalidValue("tab 写 check、bom、deliver 中的一个".to_owned());
        assert_eq!((wrong.code(), wrong.reason()), ("invalid_value", None));
        assert_eq!(wrong.to_string(), "tab 写 check、bom、deliver 中的一个");
    }

    /// A result of rule `rule` on an object of `kind` tagged `tag` — none
    /// when it is empty — on drawing `drawing`.
    fn found(rule: &str, severity: Severity, kind: &str, tag: &str, drawing: &str) -> CheckResult {
        CheckResult {
            rule_id: rule.to_owned(),
            rule_name: format!("{rule} 的名字"),
            severity,
            object: drc::ObjectRef {
                kind: kind.to_owned(),
                type_code: "T".to_owned(),
                tag: (!tag.is_empty()).then(|| tag.to_owned()),
                drawing_id: drawing.to_owned(),
                owner: None,
                port: None,
                at: [0.0, 0.0],
                handle: None,
            },
            object_key: format!("{kind}|{tag}"),
            message: format!("{tag} 不对"),
            fix: format!("改 {tag}"),
            points: Vec::new(),
            fields: vec!["管道等级".to_owned()],
            waived: false,
            waiver: None,
        }
    }

    /// `result` waived by 张工 because it is a vent.
    fn waived(mut result: CheckResult) -> CheckResult {
        result.waiver = Some(drc::Waiver {
            rule_id: result.rule_id.clone(),
            object_key: result.object_key.clone(),
            reason: "放空管".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_800_000,
        });
        result.waived = true;
        result
    }

    /// A check of `scope` that found `results`, counted as the checker
    /// counts them.
    fn report(scope: ScopeKind, results: Vec<CheckResult>) -> CheckReport {
        let mut summary = Summary::default();
        for result in &results {
            let count = match (result.waived, result.severity) {
                (true, _) => &mut summary.waived,
                (false, Severity::Error) => &mut summary.errors,
                (false, Severity::Warning) => &mut summary.warnings,
                (false, Severity::Info) => &mut summary.infos,
            };
            *count += 1;
        }
        let drawing = scope == ScopeKind::Drawing;
        CheckReport {
            version: drc::REPORT_VERSION,
            rule_set_version: "1.0.0".to_owned(),
            rule_set_digest: String::new(),
            checked_at: 1_794_800_000,
            trigger: drc::Trigger::OnDemand,
            scope: Scope {
                kind: scope,
                drawing_id: drawing.then(|| "PID-1101".to_owned()),
                pbs: None,
                cross_drawing: !drawing,
            },
            elapsed_ms: 12,
            summary,
            blocked: summary.errors > 0,
            results,
            notices: Vec::new(),
            stale_waivers: Vec::new(),
        }
    }

    /// Five results on PID-1101 in the checker's order: two errors, on a
    /// pipe line and an instrument; a warning on a pipe line and a waived
    /// one on a valve; a note on a pump.
    fn five() -> CheckReport {
        report(
            ScopeKind::Drawing,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-005", Severity::Error, "仪表", "FT-1001", "PID-1101"),
                found("DR-007", Severity::Warning, "管线", "L-1002", "PID-1101"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
                found("DR-010", Severity::Info, "设备", "P-1001", "PID-1101"),
            ],
        )
    }

    /// PID-1101 in front, saved, in a session that may write, the panel on
    /// screen.
    fn in_front() -> Context {
        Context {
            visible: true,
            document_id: Some(7),
            drawing_id: Some("PID-1101".to_owned()),
            writable: true,
            saved: true,
            busy: false,
        }
    }

    /// What the panel draws with `report` kept on the drawing in front.
    fn viewed(ui: &VerifyUi, report: &CheckReport) -> VerifyView {
        let drawing = DrawingResults {
            check: Some(KeptCheck {
                report,
                serial: 1,
                stale: false,
            }),
        };
        build_view(ui, Some(&drawing), None, &in_front())
    }

    /// The groups drawn with their counts, and the results listed by index.
    fn listed(view: &VerifyView) -> (Vec<(Group, usize)>, Vec<usize>) {
        let shown = view.check.shown.as_ref().expect("a result shown");
        (
            shown
                .groups
                .iter()
                .map(|group| (group.group, group.count))
                .collect(),
            shown.rows.iter().map(|row| row.index).collect(),
        )
    }

    #[test]
    fn the_list_is_grouped_by_severity_and_each_header_counts_what_the_filters_leave() {
        use Group::{Error, Info, Warning};
        let five = five();
        let all = VerifyUi::default();
        assert_eq!(
            listed(&viewed(&all, &five)),
            (vec![(Error, 2), (Warning, 1), (Info, 1)], vec![0, 1, 2, 4])
        );
        let no_warnings = VerifyUi {
            severities: vec![Severity::Error, Severity::Info],
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&no_warnings, &five)),
            (vec![(Error, 2), (Info, 1)], vec![0, 1, 4])
        );
        // The waived results come last, as a group of their own, which the
        // severity tags leave alone.
        let waived_too = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&waived_too, &five)),
            (
                vec![(Error, 2), (Warning, 1), (Info, 1), (Group::Waived, 1)],
                vec![0, 1, 2, 4, 3]
            )
        );
        let errors_and_waived = VerifyUi {
            severities: vec![Severity::Error],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&errors_and_waived, &five)),
            (vec![(Error, 2), (Group::Waived, 1)], vec![0, 1, 3])
        );
        // A kind switched on lists that kind only, in every group.
        let pipes = VerifyUi {
            kinds: vec!["管线".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&pipes, &five)),
            (vec![(Error, 1), (Warning, 1)], vec![0, 2])
        );
        let valves = VerifyUi {
            kinds: vec!["阀门".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&valves, &five)),
            (vec![(Group::Waived, 1)], vec![3])
        );
        // A kind this check did not find has no tag to switch it off, so it
        // filters nothing.
        let gone = VerifyUi {
            kinds: vec!["换热器".to_owned()],
            ..VerifyUi::default()
        };
        let view = viewed(&gone, &five);
        assert_eq!(listed(&view).1, vec![0, 1, 2, 4]);
        assert!(view.check.kinds_on.is_empty());
        // The kind tags are the kinds found, the waived results' too.
        let shown = viewed(&all, &five).check.shown.expect("a result shown");
        assert_eq!(shown.kinds, ["仪表", "管线", "设备", "阀门"]);
        // A folded group keeps its count and draws none of its rows; the
        // panel state lists them all the same.
        let folded = VerifyUi {
            folded: vec![Error],
            ..VerifyUi::default()
        };
        let view = viewed(&folded, &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            shown.groups[0],
            GroupView {
                group: Error,
                count: 2,
                first: 0,
                drawn: 0,
                folded: true,
            }
        );
        assert_eq!(shown.groups[1].drawn, 1);
        assert_eq!(panel_state(&view, ROWS_PER_PAGE, 0)["check"]["total"], 4);
    }

    #[test]
    fn a_row_names_its_object_by_tag_else_by_kind_and_owner() {
        let tagged = found("DR-001", Severity::Error, "管线", "L-1003", "PID-1101");
        assert_eq!(label_of(&tagged), "L-1003");
        let mut segment = found("DR-004", Severity::Warning, "管段", "", "PID-1101");
        segment.object.owner = Some("L-1003-01".to_owned());
        assert_eq!(label_of(&segment), "管段（L-1003-01）");
        let bare = found("DR-009", Severity::Error, "图纸", "", "PID-1101");
        assert_eq!(label_of(&bare), "图纸");
        // The first line: rule and object, then the drawing when the object
        // is on another one.
        let project = report(
            ScopeKind::Project,
            vec![
                tagged,
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
                segment,
            ],
        );
        let kept = ProjectResults {
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
        };
        let view = build_view(&VerifyUi::default(), None, Some(&kept), &in_front());
        let shown = view.check.shown.expect("a result shown");
        let lines: Vec<String> = shown.rows.iter().map(RowView::headline).collect();
        assert_eq!(
            lines,
            [
                "DR-001 L-1003",
                "DR-102 P-1001 · PID-1102",
                "DR-004 管段（L-1003-01）"
            ]
        );
        let labels: Vec<&str> = shown.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["L-1003", "P-1001", "管段（L-1003-01）"]);
    }

    #[test]
    fn of_the_drawing_s_and_the_project_s_result_the_newer_is_shown() {
        let mine = report(
            ScopeKind::Drawing,
            vec![found(
                "DR-001",
                Severity::Error,
                "管线",
                "L-1001",
                "PID-1101",
            )],
        );
        let theirs = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
            ],
        );
        let shown = |drawing: u64, project: u64| {
            let drawing = DrawingResults {
                check: Some(KeptCheck {
                    report: &mine,
                    serial: drawing,
                    stale: false,
                }),
            };
            let project = ProjectResults {
                check: Some(KeptCheck {
                    report: &theirs,
                    serial: project,
                    stale: true,
                }),
            };
            let view = build_view(
                &VerifyUi::default(),
                Some(&drawing),
                Some(&project),
                &in_front(),
            );
            let shown = view.check.shown.expect("a result shown");
            (shown.scope, shown.stale, shown.rows.len())
        };
        assert_eq!(shown(2, 5), (ScopeKind::Project, true, 2));
        assert_eq!(shown(6, 5), (ScopeKind::Drawing, false, 1));
        // With one kept, that one; with none, none.
        let project = ProjectResults {
            check: Some(KeptCheck {
                report: &theirs,
                serial: 1,
                stale: false,
            }),
        };
        let view = build_view(
            &VerifyUi::default(),
            Some(&DrawingResults::default()),
            Some(&project),
            &in_front(),
        );
        assert_eq!(
            view.check.shown.map(|shown| shown.scope),
            Some(ScopeKind::Project)
        );
        let view = build_view(&VerifyUi::default(), None, None, &in_front());
        assert_eq!(view.check.shown, None);
    }

    #[test]
    fn each_group_draws_200_rows_and_200_more_for_each_press() {
        let many = report(
            ScopeKind::Drawing,
            (0..450)
                .map(|n| {
                    found(
                        "DR-001",
                        Severity::Error,
                        "管线",
                        &format!("L-{n:04}"),
                        "PID-1101",
                    )
                })
                .collect(),
        );
        let drawn = |more: usize| {
            let ui = VerifyUi {
                more,
                ..VerifyUi::default()
            };
            let view = viewed(&ui, &many);
            let group = view.check.shown.expect("a result shown").groups[0];
            (group.count, group.drawn, group.has_more())
        };
        assert_eq!(drawn(0), (450, 200, true));
        assert_eq!(drawn(1), (450, 400, true));
        assert_eq!(drawn(2), (450, 450, false));
        assert_eq!(drawn(3), (450, 450, false));
        // The panel state pages them all, whatever is drawn.
        let view = viewed(&VerifyUi::default(), &many);
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        assert_eq!(state["check"]["total"], 450);
        assert_eq!(state["check"]["rows"].as_array().map(Vec::len), Some(200));
        let state = panel_state(&view, ROWS_PER_PAGE, 400);
        assert_eq!(state["check"]["rows"].as_array().map(Vec::len), Some(50));
        assert_eq!(state["check"]["rows"][0]["index"], 400);
    }

    #[test]
    fn the_rule_check_tab_says_why_it_has_no_list() {
        let ui = VerifyUi::default();
        // Never checked.
        let view = build_view(&ui, None, None, &in_front());
        assert_eq!(view.empty, Some(NEVER_CHECKED));
        assert_eq!(NEVER_CHECKED, "还没有检查过。点上面的按钮开始。");
        // Checked and nothing found: the status line and the notices are
        // there all the same.
        let mut clean = report(ScopeKind::Drawing, Vec::new());
        clean.notices.push(drc::Notice {
            code: "PROJECT_RULES_SKIPPED".to_owned(),
            message: "只检查了当前图".to_owned(),
        });
        let view = viewed(&ui, &clean);
        assert_eq!(view.empty, Some("没有发现问题。"));
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            (shown.notices.len(), shown.scope_said.as_str()),
            (1, "图 PID-1101")
        );
        // Found, but all filtered out: by the tags, or for being waived.
        let none_on = VerifyUi {
            severities: Vec::new(),
            ..VerifyUi::default()
        };
        assert_eq!(
            viewed(&none_on, &five()).empty,
            Some("没有符合筛选条件的结果。")
        );
        let all_waived = report(
            ScopeKind::Drawing,
            vec![waived(found(
                "DR-008",
                Severity::Warning,
                "阀门",
                "V-1001",
                "PID-1101",
            ))],
        );
        assert_eq!(viewed(&ui, &all_waived).empty, Some(ALL_FILTERED));
        let waived_too = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(viewed(&waived_too, &all_waived).empty, None);
        // A broken rule pack: its error in the message bar, the result kept
        // as it was.
        let broken = VerifyUi {
            notice: Some(Notice::error(
                Some("DSL_PARSE_ERROR".to_owned()),
                "PIDCHECK: 项目包有错",
            )),
            ..VerifyUi::default()
        };
        let five = five();
        let view = viewed(&broken, &five);
        assert_eq!(view.empty, None);
        assert_eq!(listed(&view).1, vec![0, 1, 2, 4]);
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        assert_eq!(
            state["notice"],
            json!({ "level": "error", "code": "DSL_PARSE_ERROR", "text": "PIDCHECK: 项目包有错" })
        );
        assert_eq!(state["check"]["total"], 4);
        // The other tabs say their own.
        let bom = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        assert_eq!(viewed(&bom, &five).empty, Some("还没有生成材料清单。"));
    }

    /// Whether 【检查当前图】【检查全项目】【导出…】 can be pressed.
    fn pressable(context: &Context, kept: Option<&DrawingResults<'_>>) -> (bool, bool, bool) {
        let buttons = build_view(&VerifyUi::default(), kept, None, context).buttons;
        (
            buttons.check_drawing,
            buttons.check_project,
            buttons.check_export,
        )
    }

    #[test]
    fn the_rule_check_buttons_are_grey_as_section_4_8_says() {
        let five = five();
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
        };
        assert_eq!(pressable(&in_front(), Some(&kept)), (true, true, true));
        // Nothing checked: nothing to export.
        assert_eq!(pressable(&in_front(), None), (true, true, false));
        // Busy: no new check, but the result can be exported.
        let busy = Context {
            busy: true,
            ..in_front()
        };
        assert_eq!(pressable(&busy, Some(&kept)), (false, false, true));
        // Never saved: in no project folder.
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        assert_eq!(pressable(&unsaved, Some(&kept)), (true, false, true));
        // A read-only session checks and exports all the same (§5.4).
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(pressable(&read_only, Some(&kept)), (true, true, true));
        // The Start page has no drawing to check.
        assert_eq!(pressable(&Context::default(), None), (false, false, false));
        // The other tabs bring their own buttons.
        let buttons = build_view(&VerifyUi::default(), Some(&kept), None, &in_front()).buttons;
        assert_eq!(
            Buttons {
                check_drawing: false,
                check_project: false,
                check_export: false,
                ..buttons
            },
            Buttons::default()
        );
    }

    #[test]
    fn a_row_can_be_waived_on_this_drawing_unwaived_by_a_session_that_may_write() {
        let project = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
            ],
        );
        let kept = ProjectResults {
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
        };
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let waivable = |context: &Context| -> Vec<(usize, bool)> {
            let view = build_view(&ui, None, Some(&kept), context);
            let shown = view.check.shown.expect("a result shown");
            shown
                .rows
                .iter()
                .map(|row| (row.index, row.can_waive))
                .collect()
        };
        assert_eq!(waivable(&in_front()), [(0, true), (1, false), (2, false)]);
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(waivable(&read_only), [(0, false), (1, false), (2, false)]);
    }

    #[test]
    fn the_check_block_reads_off_the_tab_as_worked_out() {
        let mut five = five();
        five.notices.push(drc::Notice {
            code: "MASTER_DATA_MISSING".to_owned(),
            message: "项目主数据没有配置".to_owned(),
        });
        five.stale_waivers.push(drc::Waiver {
            rule_id: "DR-003".to_owned(),
            object_key: "管线|L-0999".to_owned(),
            reason: "已拆除".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_700_000,
        });
        let ui = VerifyUi {
            severities: vec![Severity::Error],
            kinds: vec!["管线".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 3,
                stale: true,
            }),
        };
        let view = build_view(&ui, Some(&kept), None, &in_front());
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        let check = &state["check"];
        assert_eq!(check["has_result"], true, "{check}");
        assert_eq!(check["stale"], true, "{check}");
        assert_eq!(check["scope"], "drawing", "{check}");
        assert_eq!(check["checked_at"], 1_794_800_000_i64, "{check}");
        assert_eq!(check["rule_set_version"], "1.0.0", "{check}");
        // The counts are the check's own, whatever the filters.
        assert_eq!(
            check["counts"],
            json!({ "错误": 2, "警告": 1, "提示": 1, "已豁免": 1 })
        );
        assert_eq!(
            check["filters"],
            json!({ "severity": ["错误"], "kinds": ["管线"], "show_waived": true })
        );
        assert_eq!(check["kinds"], json!(["仪表", "管线", "设备", "阀门"]));
        assert_eq!(
            check["notices"],
            json!([{ "code": "MASTER_DATA_MISSING", "message": "项目主数据没有配置" }])
        );
        assert_eq!(check["stale_waivers"], 1);
        assert_eq!(check["total"], 1);
        assert_eq!(
            check["rows"],
            json!([{
                "index": 0,
                "rule_id": "DR-001",
                "severity": "错误",
                "object_key": "管线|L-1001",
                "drawing_id": "PID-1101",
                "label": "L-1001",
                "message": "L-1001 不对",
                "fields": ["管道等级"],
                "waived": false,
                "can_waive": true
            }])
        );
        assert_eq!(check["located"], Value::Null);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            status_line(shown),
            "图 PID-1101 · 2026-11-16 03:33 UTC · 规则 1.0.0 · 用时 12 ms"
        );
    }

    #[test]
    fn pid_verify_action_reads_the_rule_check_keys() {
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        for (value, pressed) in [
            ("check=drawing", PidVerifyMsg::CheckDrawing),
            ("check=project", PidVerifyMsg::CheckProject),
            ("severity=错误", PidVerifyMsg::Severity(Severity::Error)),
            ("severity=提示", PidVerifyMsg::Severity(Severity::Info)),
            ("kind=管线", PidVerifyMsg::Kind(Some("管线".to_owned()))),
            ("kind=", PidVerifyMsg::Kind(None)),
            ("show_waived=1", PidVerifyMsg::ShowWaived(true)),
            ("show_waived=0", PidVerifyMsg::ShowWaived(false)),
            (
                "check_export=D:/out/drc.json",
                PidVerifyMsg::Export("D:/out/drc.json".to_owned()),
            ),
            ("more=check", PidVerifyMsg::MoreChecks),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_rule_check_button_with_the_reason() {
        let five = five();
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
        };
        let ui = VerifyUi::default();
        let busy_context = Context {
            busy: true,
            ..in_front()
        };
        let busy = build_view(&ui, Some(&kept), None, &busy_context);
        let unsaved_context = Context {
            saved: false,
            ..in_front()
        };
        let unsaved = build_view(&ui, Some(&kept), None, &unsaved_context);
        let unchecked = build_view(&ui, None, None, &in_front());
        // The Start page in front.
        let no_drawing = build_view(&ui, None, None, &Context::default());
        for (view, value, reason) in [
            (&busy, "check=drawing", "busy"),
            (&busy, "check=project", "busy"),
            (&unsaved, "check=project", "no_project_dir"),
            (&no_drawing, "check=drawing", "no_drawing"),
            (&no_drawing, "check=project", "no_drawing"),
            (&unchecked, "check_export=D:/out/drc.json", "no_result"),
            (&unchecked, "kind=管线", "no_result"),
        ] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        // Filtering, paging and exporting stay open while something runs.
        for value in [
            "severity=警告",
            "show_waived=1",
            "kind=管线",
            "kind=",
            "more=check",
            "check_export=drc.json",
        ] {
            assert!(parse_action(value, &busy).is_ok(), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_names_a_wrong_rule_check_value_invalid() {
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        for value in [
            "check",
            "check=",
            "check=all",
            "severity",
            "severity=",
            "severity=严重",
            "kind",
            "kind=换热器",
            "show_waived",
            "show_waived=2",
            "show_waived=true",
            "check_export",
            "check_export=",
            "check_export=  ",
            "more",
            "more=",
            "more=list",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            parse_action("kind=换热器", &view).map_err(|refused| refused.to_string()),
            Err("这次检查的结果里没有种类「换热器」；有的是：仪表、管线、设备、阀门".to_owned())
        );
    }
}

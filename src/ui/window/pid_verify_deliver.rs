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
use crate::ui::dock::{Chrome, PanelId};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill, Theme};
use serde_json::{json, Value};

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
            Self::Check => "还没有检查过。",
            Self::Bom => "还没有生成材料清单。",
            Self::Deliver => "还没有快照和发布记录。",
        }
    }
}

/// What the user did in the panel. Only the tab switch so far; each tab
/// brings its own buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidVerifyMsg {
    /// Bring a tab forward.
    Tab(Tab),
}

impl PidVerifyMsg {
    pub(crate) fn message(self) -> Message {
        Message::PidVerify(self)
    }
}

/// `PosReport` id of the panel body (§3.4).
pub const PANEL_BOUNDS_ID: &str = "pid-verify-panel";

/// What every `PosReport` id of the panel starts with: closing the panel
/// forgets them all.
pub const BOUNDS_PREFIX: &str = "pid-verify-";

/// How many rows of each list the panel state returns when the request
/// does not say (`limit`, §3.2).
pub const ROWS_PER_PAGE: usize = 200;

/// The panel's interface state (§3.1): one for the whole program, so the
/// panel stays as it was left across document switches. Only the tab so
/// far; the filters, the inputs and the rest come with the tabs that use
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyUi {
    pub tab: Tab,
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

/// What the panel draws, worked out by [`build_view`]: plain data, so the
/// interface can report it and a test can compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyView {
    /// The tab in front.
    pub tab: Tab,
    /// What the tab in front says while it has nothing to list.
    pub empty: &'static str,
    /// Whether the panel is on screen.
    pub visible: bool,
    /// The drawing tab the panel shows the results of, and its number
    /// ([`Context`]).
    pub document_id: Option<u64>,
    pub drawing_id: Option<String>,
    /// Whether the session may change that drawing.
    pub writable: bool,
    /// Which buttons can be pressed.
    pub buttons: Buttons,
}

impl Default for VerifyView {
    fn default() -> Self {
        build_view(&VerifyUi::default(), &Context::default())
    }
}

/// Interface state and the program's state around it → what to draw. The
/// filtering, grouping, row names and which buttons can be pressed are
/// worked out here as the tabs arrive.
pub fn build_view(ui: &VerifyUi, context: &Context) -> VerifyView {
    VerifyView {
        tab: ui.tab,
        empty: ui.tab.empty(),
        visible: context.visible,
        document_id: context.document_id,
        drawing_id: context.drawing_id.clone(),
        writable: context.writable,
        // None of the buttons is drawn yet, so none can be pressed; each tab
        // works its own out by §4.8 as it brings them.
        buttons: Buttons::default(),
    }
}

/// The panel state (§3.2) the `pid_verify_panel` op answers, read off the
/// same view the panel draws, shown or hidden. Each list gives `limit` rows
/// from `offset` on, while its `total` counts them all. Nothing runs in the
/// background yet and there is no message to show. `deliver.publish` stays
/// `null` until the publish half is drawn: it is read from disk and has no
/// empty form.
pub fn panel_state(view: &VerifyView, limit: usize, offset: usize) -> Value {
    json!({
        "ok": true,
        "visible": view.visible,
        "tab": view.tab.key(),
        "document_id": view.document_id,
        "drawing_id": view.drawing_id,
        "writable": view.writable,
        "busy": null,
        "notice": null,
        "buttons": view.buttons,
        "check": paged(no_check_result(), limit, offset),
        "bom": paged(no_bom(), limit, offset),
        "deliver": {"snapshots": [], "picked": [], "diff": null, "publish": null}
    })
}

/// The `check` block before any check has run.
fn no_check_result() -> Value {
    json!({
        "has_result": false,
        "stale": false,
        "scope": null,
        "checked_at": null,
        "rule_set_version": null,
        "counts": {"错误": 0, "警告": 0, "提示": 0, "已豁免": 0},
        "filters": {"severity": ["错误", "警告", "提示"], "kinds": [], "show_waived": false},
        "kinds": [],
        "notices": [],
        "stale_waivers": 0,
        "total": 0,
        "rows": [],
        "located": null
    })
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

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
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
        tabs = tabs.push(crate::ui::wrap_bar::PosReport::new(
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
    let content = container(text(view.empty).size(12).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill);
    let body = column![tabs, content].spacing(6).height(Fill);
    let reported = crate::ui::wrap_bar::PosReport::new(PANEL_BOUNDS_ID, body);
    crate::ui::dock::frame(title_bar, reported, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_starts_on_the_rule_check_tab() {
        let view = VerifyView::default();
        assert_eq!(view.tab, Tab::Check);
        assert_eq!(view.empty, "还没有检查过。");
    }

    #[test]
    fn what_is_drawn_follows_the_tab() {
        let drawn: Vec<(Tab, &str)> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let view = build_view(&VerifyUi { tab }, &Context::default());
                (view.tab, view.empty)
            })
            .collect();
        assert_eq!(
            drawn,
            [
                (Tab::Check, "还没有检查过。"),
                (Tab::Bom, "还没有生成材料清单。"),
                (Tab::Deliver, "还没有快照和发布记录。"),
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
    }

    /// A drawing in front that the session may not change, the panel on
    /// screen.
    fn read_only_in_front() -> Context {
        Context {
            visible: true,
            document_id: Some(7),
            drawing_id: Some("PID-1101".to_owned()),
            writable: false,
        }
    }

    #[test]
    fn pid_verify_panel_answers_the_common_fields_and_the_three_blocks() {
        let view = build_view(&VerifyUi { tab: Tab::Bom }, &read_only_in_front());
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
        // No button is drawn yet, so none can be pressed.
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
        let none: Vec<(&str, Option<bool>)> =
            names.into_iter().map(|name| (name, Some(false))).collect();
        assert_eq!(buttons, none, "{state}");
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
}

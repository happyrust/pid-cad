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

/// The panel's interface state (§3.1): one for the whole program, so the
/// panel stays as it was left across document switches. Only the tab so
/// far; the filters, the inputs and the rest come with the tabs that use
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyUi {
    pub tab: Tab,
}

/// What the panel draws, worked out by [`build_view`]: plain data, so the
/// interface can report it and a test can compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyView {
    /// The tab in front.
    pub tab: Tab,
    /// What the tab in front says while it has nothing to list.
    pub empty: &'static str,
}

impl Default for VerifyView {
    fn default() -> Self {
        build_view(&VerifyUi::default())
    }
}

/// Interface state → what to draw. The filtering, grouping, row names and
/// which buttons can be pressed are worked out here as the tabs arrive.
pub fn build_view(ui: &VerifyUi) -> VerifyView {
    VerifyView {
        tab: ui.tab,
        empty: ui.tab.empty(),
    }
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
        tabs = tabs.push(
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
        );
    }
    let body = container(text(view.empty).size(12).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill);
    crate::ui::dock::frame(
        title_bar,
        column![tabs, body].spacing(6).height(Fill),
        width,
    )
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
                let view = build_view(&VerifyUi { tab });
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
}

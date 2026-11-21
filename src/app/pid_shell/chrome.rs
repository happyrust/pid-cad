//! The shell around a screen: the title bar, the navigation and the status
//! bar of the design's `component/App Shell`.
//!
//! Every element here shows something the program has, or is not drawn. From
//! the design's shell that leaves out the project selector, the search field,
//! the notification bell, the five layer names in the status bar and the
//! "V25R4 兼容" note; `docs/plans/2026-11-11-pid-ui-overhaul-map.md`, section
//! 2, says why for each. What is drawn in their place, where there is
//! something real to draw: the collaboration server's address next to the
//! brand, and the state of the collaboration session at the foot of the
//! navigation ([`SyncStatus`]).
//!
//! Sizes are the design's; colours, fonts and radii are the design's
//! variables (`ui::style::pid_tokens`), read for the shell's theme. No colour
//! is written out here.

use iced::widget::{button, column, container, opaque, row, stack, text, Space};
use iced::{alignment, font, Background, Border, Color, Element, Fill, Font, Length, Padding};

use super::{nav, Message, NavGroup, PidScreen};
use crate::app::{Message as AppMessage, OpenCADStudio};
use crate::ui::style::pid_fonts;
use crate::ui::style::pid_tokens::{ColorToken, NumberToken, PidTheme};
use crate::ui::wrap_bar::PosReport;

/// Height of the title bar.
pub const TITLE_BAR_HEIGHT: f32 = 44.0;
/// Width of the navigation.
pub const NAV_WIDTH: f32 = 208.0;
/// Height of the status bar.
pub const STATUS_BAR_HEIGHT: f32 = 26.0;

/// What the shell's parts report their rectangles under, followed by the
/// part: `title`, `nav`, `status`, `sync`, `crumb`, `help`, `server`,
/// `avatar`, `nav:<screen id>` for a navigation entry and
/// `status:<name>` for a switch of the status bar.
pub const CHROME_BOUNDS_PREFIX: &str = "PID_CHROME:";

fn part(name: &str) -> String {
    format!("{CHROME_BOUNDS_PREFIX}{name}")
}

/// The id a navigation entry's rectangle is reported under.
pub fn nav_bounds_id(screen: PidScreen) -> String {
    part(&format!("nav:{}", screen.id()))
}

// ── The collaboration session, as the navigation's foot states it ────────

/// The state of the collaboration session. Each is read off the session
/// itself (`app::collab::State`); none is assumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// No server is named, so nothing dials out.
    Off,
    /// A server is named and the connection is being made.
    Connecting,
    /// The connection is down and has failed to come back `failed` times.
    Retrying {
        /// Reconnect attempts that have failed since the last live session.
        failed: u32,
    },
    /// Connected, and the drawing is at `revision` of the room.
    Connected {
        /// The room's revision this session has applied.
        revision: i64,
    },
    /// Connected, and this session may not write to the drawing.
    ReadOnly {
        /// The room's revision this session has applied.
        revision: i64,
    },
    /// Connected, but the drawing is no longer what the room has: the
    /// session sends nothing more.
    Diverged,
}

impl SyncStatus {
    /// The session's state. A connection that is down is said before
    /// anything else, since nothing else is current then.
    pub(crate) fn of(app: &OpenCADStudio) -> Self {
        let session = &app.collab;
        if !session.wanted() {
            SyncStatus::Off
        } else if !session.connected {
            match session.reconnect_attempts {
                0 => SyncStatus::Connecting,
                failed => SyncStatus::Retrying { failed },
            }
        } else if session.diverged.is_some() {
            SyncStatus::Diverged
        } else if session.readonly {
            SyncStatus::ReadOnly {
                revision: session.base_revision,
            }
        } else {
            SyncStatus::Connected {
                revision: session.base_revision,
            }
        }
    }

    /// What the navigation's foot says.
    pub fn label(&self) -> String {
        match self {
            SyncStatus::Off => "未连接协同服务器".to_owned(),
            SyncStatus::Connecting => "正在连接协同服务器…".to_owned(),
            SyncStatus::Retrying { failed } => format!("连接已断开，重试第 {failed} 次"),
            SyncStatus::Connected { revision } => format!("已连接 · 修订 {revision}"),
            SyncStatus::ReadOnly { revision } => format!("已连接 · 只读 · 修订 {revision}"),
            SyncStatus::Diverged => "与房间不一致，已停止同步".to_owned(),
        }
    }

    /// The colour of the dot beside it.
    pub fn dot(&self) -> ColorToken {
        match self {
            SyncStatus::Off => ColorToken::TextMuted,
            SyncStatus::Connecting | SyncStatus::ReadOnly { .. } => ColorToken::Amber,
            SyncStatus::Retrying { .. } | SyncStatus::Diverged => ColorToken::Danger,
            SyncStatus::Connected { .. } => ColorToken::Success,
        }
    }
}

// ── What the bars say ────────────────────────────────────────────────────

/// The collaboration server's address, as the title bar shows it while one
/// is named; `None` without one.
pub(crate) fn server_address(app: &OpenCADStudio) -> Option<String> {
    app.collab
        .wanted()
        .then(|| crate::app::collab::shown_address(app.collab.server_url.trim()))
}

/// The first letter of the collaboration user's id, for the avatar; `None`
/// while nobody is named.
pub(crate) fn user_initial(app: &OpenCADStudio) -> Option<String> {
    app.collab
        .user_id
        .trim()
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
}

/// The breadcrumb: the screen. The drawing in front is named once, on its
/// tab in the editor, and not here as well.
pub(crate) fn breadcrumb(_app: &OpenCADStudio, screen: PidScreen) -> String {
    screen.title().to_owned()
}

/// One part of the status bar's text.
pub(crate) struct StatusPart {
    /// What it says.
    pub label: String,
    /// For a part that is a switch: the name its rectangle is reported
    /// under (`status:<name>`), and what a press sends.
    pub switch: Option<(&'static str, AppMessage)>,
}

/// The status bar, part by part: for the editor with a drawing in front,
/// whether the drawing can be written, and the two drafting aids the design
/// names — each the switch of the aid it names.
pub(crate) fn status_parts(app: &OpenCADStudio, screen: PidScreen) -> Vec<StatusPart> {
    let on_off = |on: bool| if on { "开" } else { "关" };
    let said = |label: String| StatusPart {
        label,
        switch: None,
    };
    let mut parts = if !screen.needs_drawing() || app.tabs[app.active_tab].is_start {
        vec![said("就绪".to_owned())]
    } else {
        let writing = if app.read_only || app.collab.readonly {
            "只读"
        } else {
            "编辑中"
        };
        vec![
            said(writing.to_owned()),
            StatusPart {
                label: format!("捕捉 {}", on_off(app.snapper.snap_enabled)),
                switch: Some(("snap", AppMessage::ToggleSnapEnabled)),
            },
            StatusPart {
                label: format!("正交 {}", on_off(app.ortho_mode)),
                switch: Some(("ortho", AppMessage::ToggleOrtho)),
            },
        ]
    };
    parts.extend(super::backup_export_ui::status_bar_parts(app, screen));
    parts
}

/// The status bar's text in one line, as the tests read it.
#[cfg(test)]
pub(crate) fn status_text(app: &OpenCADStudio, screen: PidScreen) -> String {
    status_parts(app, screen)
        .into_iter()
        .map(|part| part.label)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The navigation as it is drawn: the design's groups that have an
/// implemented screen, each with its implemented screens, in the design's
/// order.
pub fn nav_groups() -> Vec<(NavGroup, Vec<PidScreen>)> {
    let mut groups: Vec<(NavGroup, Vec<PidScreen>)> = Vec::new();
    for screen in nav() {
        match groups.last_mut() {
            Some((group, screens)) if *group == screen.group() => screens.push(screen),
            _ => groups.push((screen.group(), vec![screen])),
        }
    }
    groups
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn ui_font(theme: PidTheme, weight: font::Weight) -> Font {
    pid_fonts::ui(theme, weight)
}

fn mono_font(theme: PidTheme, weight: font::Weight) -> Font {
    pid_fonts::mono(theme, weight)
}

/// A bar or a panel of the shell: the panel colour with the hairline the
/// design strokes it with. The design's stroke sits on the edge, half of it
/// outside and clipped away, so what shows is the border colour at half
/// strength.
fn panel(theme: PidTheme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
        border: Border {
            color: ColorToken::Border.color(theme).scale_alpha(0.5),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

/// A chip: the raised colour, a hairline, the small radius.
fn chip(theme: PidTheme, fill: ColorToken, stroke: Option<ColorToken>) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill.color(theme))),
        border: Border {
            color: stroke.map_or(Color::TRANSPARENT, |stroke| stroke.color(theme)),
            width: if stroke.is_some() { 1.0 } else { 0.0 },
            radius: NumberToken::RadiusSm.value(theme).into(),
        },
        ..container::Style::default()
    }
}

/// A button that is only its content: the shell's buttons paint themselves.
fn bare(_theme: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        border: Border::default(),
        shadow: iced::Shadow::default(),
        snap: false,
        ..button::Style::default()
    }
}

fn title_bar<'a>(app: &'a OpenCADStudio, screen: PidScreen) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let colour = |token: ColorToken| token.color(theme);

    let mark = container(Space::new())
        .width(22)
        .height(22)
        .style(move |_| container::Style {
            // The design's mark runs from the accent to the process-line
            // colour, top-left to bottom-right.
            background: Some(Background::Gradient(
                iced::gradient::Linear::new(iced::Degrees(135.0))
                    .add_stop(0.0, ColorToken::Accent.color(theme))
                    .add_stop(1.0, ColorToken::LineProcess.color(theme))
                    .into(),
            )),
            border: Border {
                radius: 5.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    let brand = row![
        mark,
        text("OpenCADStudio")
            .size(13)
            .font(ui_font(theme, font::Weight::Semibold))
            .color(colour(ColorToken::TextPrimary)),
        text("P&ID")
            .size(11)
            .font(mono_font(theme, font::Weight::Semibold))
            .color(colour(ColorToken::Accent)),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center);

    let separator = container(Space::new())
        .width(1)
        .height(20)
        .style(move |_| container::Style {
            background: Some(Background::Color(ColorToken::Border.color(theme))),
            ..container::Style::default()
        });

    let mut bar = row![brand, separator]
        .spacing(16)
        .align_y(alignment::Vertical::Center);
    if let Some(address) = server_address(app) {
        bar = bar.push(PosReport::owned(
            part("server"),
            container(
                text(address)
                    .size(12)
                    .font(ui_font(theme, font::Weight::Normal))
                    .color(colour(ColorToken::TextPrimary)),
            )
            .padding([6, 10])
            .style(move |_| chip(theme, ColorToken::BgRaised, Some(ColorToken::Border))),
        ));
    }
    bar = bar.push(PosReport::owned(
        part("crumb"),
        text(breadcrumb(app, screen))
            .size(12)
            .font(ui_font(theme, font::Weight::Normal))
            .color(colour(ColorToken::TextSecondary)),
    ));
    bar = bar.push(Space::new().width(Fill));
    bar = bar.push(PosReport::owned(
        part("help"),
        button(crate::ui::icons::tinted(
            include_bytes!("../../../assets/icons/ui/help.svg"),
            16.0,
            move |_| ColorToken::TextSecondary.color(theme),
        ))
        .padding(0)
        .style(bare)
        .on_press(AppMessage::Command("HELP".to_owned())),
    ));
    if let Some(initial) = user_initial(app) {
        bar = bar.push(PosReport::owned(
            part("avatar"),
            container(
                text(initial)
                    .size(11)
                    .font(ui_font(theme, font::Weight::Semibold))
                    .color(colour(ColorToken::TextPrimary)),
            )
            .width(26)
            .height(26)
            .center_x(26)
            .center_y(26)
            .style(move |_| container::Style {
                background: Some(Background::Color(ColorToken::BgHover.color(theme))),
                border: Border {
                    color: ColorToken::BorderStrong.color(theme),
                    width: 1.0,
                    radius: 13.0.into(),
                },
                ..container::Style::default()
            }),
        ));
    }

    container(bar)
        .width(Fill)
        .height(TITLE_BAR_HEIGHT)
        .padding([0, 16])
        .align_y(alignment::Vertical::Center)
        .style(move |_| panel(theme))
        .into()
}

/// The icon of a navigation entry.
fn nav_icon(screen: PidScreen) -> &'static [u8] {
    match screen {
        PidScreen::Editor => include_bytes!("../../../assets/icons/pid_palette.svg"),
        _ => include_bytes!("../../../assets/icons/ui/dot.svg"),
    }
}

fn nav_entry<'a>(theme: PidTheme, screen: PidScreen, showing: bool) -> Element<'a, AppMessage> {
    let (icon, label, weight) = if showing {
        (
            ColorToken::Accent,
            ColorToken::TextPrimary,
            font::Weight::Semibold,
        )
    } else {
        (
            ColorToken::TextSecondary,
            ColorToken::TextSecondary,
            font::Weight::Normal,
        )
    };
    let content = row![
        crate::ui::icons::tinted(nav_icon(screen), 15.0, move |_| icon.color(theme)),
        text(screen.title())
            .size(12.5)
            .font(ui_font(theme, weight))
            .color(label.color(theme)),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center);
    let entry = button(
        container(content)
            .width(Fill)
            .padding([7, 10])
            .style(move |_| {
                if showing {
                    chip(theme, ColorToken::AccentSoft, None)
                } else {
                    container::Style::default()
                }
            }),
    )
    .width(Fill)
    .padding(0)
    .style(bare)
    .on_press(AppMessage::PidShell(Message::Show(screen)));
    PosReport::owned(nav_bounds_id(screen), entry).into()
}

fn navigation<'a>(app: &'a OpenCADStudio, showing: PidScreen) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let mut entries = column![].spacing(2);
    for (group, screens) in nav_groups() {
        entries = entries.push(
            container(
                text(group.title())
                    .size(10)
                    .font(ui_font(theme, font::Weight::Semibold))
                    .color(ColorToken::TextMuted.color(theme)),
            )
            .width(Fill)
            .padding(Padding {
                top: 12.0,
                right: 10.0,
                bottom: 6.0,
                left: 10.0,
            }),
        );
        for screen in screens {
            entries = entries.push(nav_entry(theme, screen, screen == showing));
        }
    }

    let status = SyncStatus::of(app);
    let dot = status.dot();
    let sync = PosReport::owned(
        part("sync"),
        container(
            row![
                container(Space::new())
                    .width(7)
                    .height(7)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(dot.color(theme))),
                        border: Border {
                            radius: 3.5.into(),
                            ..Border::default()
                        },
                        ..container::Style::default()
                    }),
                text(status.label())
                    .size(11)
                    .font(ui_font(theme, font::Weight::Normal))
                    .color(ColorToken::TextSecondary.color(theme)),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .width(Fill)
        .padding(10)
        .style(move |_| chip(theme, ColorToken::BgRaised, None)),
    );

    container(column![entries, Space::new().height(Fill), sync])
        .width(NAV_WIDTH)
        .height(Fill)
        .padding([12, 8])
        .style(move |_| panel(theme))
        .into()
}

fn status_bar<'a>(app: &'a OpenCADStudio, screen: PidScreen) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let said = move |label: String| {
        text(label)
            .size(10)
            .font(mono_font(theme, font::Weight::Normal))
            .color(ColorToken::TextMuted.color(theme))
    };
    let mut line = row![Space::new().width(Fill)]
        .spacing(6)
        .align_y(alignment::Vertical::Center);
    for (index, status) in status_parts(app, screen).into_iter().enumerate() {
        if index > 0 {
            line = line.push(said("·".to_owned()));
        }
        line = match status.switch {
            Some((name, message)) => line.push(PosReport::owned(
                part(&format!("status:{name}")),
                button(said(status.label))
                    .padding(0)
                    .style(bare)
                    .on_press(message),
            )),
            None => line.push(said(status.label)),
        };
    }
    container(line)
        .width(Fill)
        .height(STATUS_BAR_HEIGHT)
        .padding([0, 14])
        .align_y(alignment::Vertical::Center)
        .style(move |_| panel(theme))
        .into()
}

/// `part` of the module's own interface while the general interface has a
/// dialog up in the drawing: dimmed as the dialog dims what is under it,
/// and deaf to the pointer. The dialog's own backdrop reaches no further
/// than the drawing's frame; this is the rest of the window.
pub(super) fn shielded<'a>(
    app: &OpenCADStudio,
    part: Element<'a, AppMessage>,
) -> Element<'a, AppMessage> {
    if !app.dialog_is_up() {
        return part;
    }
    let dim = container(Space::new())
        .width(Fill)
        .height(Fill)
        .style(|theme: &iced::Theme| container::Style {
            // The colour of the general interface's backdrop
            // (`ui::modal`), so that the window dims as one.
            background: Some(Background::Color(
                theme.palette().background.strongest.color.scale_alpha(0.55),
            )),
            ..container::Style::default()
        });
    stack![part, opaque(dim)].into()
}

/// The shell around `content`, which is the showing screen: the title bar
/// above, the navigation to its left, the status bar below.
pub(super) fn frame<'a>(
    app: &'a OpenCADStudio,
    screen: PidScreen,
    content: Element<'a, AppMessage>,
) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    container(column![
        PosReport::owned(part("title"), shielded(app, title_bar(app, screen))),
        row![
            PosReport::owned(part("nav"), shielded(app, navigation(app, screen))),
            container(content).width(Fill).height(Fill),
        ]
        .height(Fill),
        PosReport::owned(part("status"), shielded(app, status_bar(app, screen))),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(ColorToken::BgApp.color(theme))),
        ..container::Style::default()
    })
    .into()
}

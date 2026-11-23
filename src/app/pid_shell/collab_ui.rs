//! Collaboration UI components for the new P&ID shell (#48–#66).
//!
//! Pure view-model builders + iced widgets drawn strictly with `pid_tokens`
//! variables (zero hardcoded colours):
//! - Toolbar online avatar group & count (#49)
//! - Canvas read-only banner with verbatim reason & holder (#51)
//! - Inspector collaboration section: status, roster, 3-tier readonly breakdown,
//!   queue/replay/rejection summary, tag claims + Claim/Release, pending
//!   conflicts, and Realign recovery (#48, #52–#55, #62, #78)
//! - Inspector remote-claim notice & read-only gating (#57)
//! - Canvas claim markers (`holder · 到期 HH:MM`) and page-connector pairing
//!   badges (`未配对` / `至 <图号>` / `自 <图号>`) (#55, #56, #63)
//! - Shared stacked toast notification overlay (#50, #58–#61, #65)
//! - Inline object modification history drawer + field last-writer helpers (#66)

use iced::widget::{button, column, container, pin, row, scrollable, text, text_input, Space};
use iced::{alignment, font, Background, Border, Color, Element, Fill, Length, Point};

use super::editor::{part, Message as EditorMsg};
use super::Message as ShellMessage;
use crate::app::collab::query::{AuditItem, FieldLastWriter};
use crate::app::collab::toasts::{
    avatar_glyph, format_hhmm, format_timestamp, format_user_label, ToastKind,
};
use crate::app::collab::{HistoryFilter, State as CollabState};
use crate::app::{Message as AppMessage, OpenCADStudio};
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::{split_props, LINE_FIELDS};
use crate::pid::topology::{
    handle_text, parse_handle, ConnectorKind, TopologyTable, ROLE_SIGNAL_LINE,
};
use crate::ui::style::pid_fonts;
use crate::ui::style::pid_tokens::{
    ColorToken, NumberToken, PidTheme, CLAIM_COLOR_TOKEN, CONNECTOR_PIPE_COLOR_TOKEN,
    CONNECTOR_SIGNAL_COLOR_TOKEN,
};
use crate::ui::window::pid_collab::{
    PidCollabMsg, ADOPT_A, ADOPT_B, ADOPT_MANUAL, CONFIRM_RULING, NO_OTHERS_LABEL,
    NO_PENDING_CONFLICTS, NO_PENDING_CONFLICTS_WHY, OPEN_CONFLICT_LIST,
};
use crate::ui::wrap_bar::PosReport;

/// The five role names defined by `REQ-COLL-011` (#52).
pub(crate) const ROLE_NAMES: [&str; 5] = [
    "系统管理员",
    "项目管理员",
    "工艺设计人员",
    "校对人员",
    "只读查看者",
];

fn ui(theme: PidTheme, weight: font::Weight) -> iced::Font {
    pid_fonts::ui(theme, weight)
}

fn mono(theme: PidTheme, weight: font::Weight) -> iced::Font {
    pid_fonts::mono(theme, weight)
}

fn filled(
    theme: PidTheme,
    fill: ColorToken,
    stroke: Option<ColorToken>,
    radius: f32,
) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill.color(theme))),
        border: Border {
            color: stroke.map_or(Color::TRANSPARENT, |s| s.color(theme)),
            width: if stroke.is_some() { 1.0 } else { 0.0 },
            radius: radius.into(),
        },
        ..container::Style::default()
    }
}

fn bare(_theme: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        border: Border::default(),
        shadow: iced::Shadow::default(),
        snap: false,
        ..button::Style::default()
    }
}

fn send_editor(msg: EditorMsg) -> AppMessage {
    AppMessage::PidShell(ShellMessage::Editor(msg))
}

// ── #49: Toolbar online avatar group & count ─────────────────────────────

/// One avatar bubble in the editor toolbar (#49).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OnlineAvatar {
    pub user_id: String,
    pub label: String,
    pub glyph: String,
    pub is_self: bool,
    pub disconnected: bool,
}

/// Avatars to show in the editor toolbar: empty unless at least 2 sessions
/// are online (self + at least one peer) (#49).
pub(crate) fn online_avatars(app: &OpenCADStudio) -> Vec<OnlineAvatar> {
    let state = &app.collab;
    if state.session_id.is_none() || state.peers.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(1 + state.peers.len());
    out.push(OnlineAvatar {
        user_id: state.user_id.clone(),
        label: format_user_label(&state.user_id, &state.display_name),
        glyph: avatar_glyph(&state.user_id, &state.display_name),
        is_self: true,
        disconnected: false,
    });
    for peer in &state.peers {
        out.push(OnlineAvatar {
            user_id: peer.user_id.clone(),
            label: format_user_label(&peer.user_id, &peer.display_name),
            glyph: avatar_glyph(&peer.user_id, &peer.display_name),
            is_self: false,
            disconnected: peer.disconnected,
        });
    }
    out
}

/// Toolbar online avatar group widget; `None` when alone or offline (#49).
pub(crate) fn toolbar_online_group(app: &OpenCADStudio) -> Option<Element<'_, AppMessage>> {
    let avatars = online_avatars(app);
    if avatars.len() < 2 {
        return None;
    }
    let theme = app.pid_shell.theme;
    let total = avatars.len();
    let mut circles = row![].spacing(4).align_y(alignment::Vertical::Center);
    for avatar in avatars.into_iter().take(5) {
        let fill = if avatar.is_self {
            ColorToken::AccentSoft
        } else if avatar.disconnected {
            ColorToken::AmberSoft
        } else {
            ColorToken::BgRaised
        };
        let stroke = if avatar.is_self {
            ColorToken::Accent
        } else if avatar.disconnected {
            ColorToken::Amber
        } else {
            ColorToken::BorderStrong
        };
        let ink = if avatar.disconnected {
            ColorToken::TextMuted
        } else {
            ColorToken::TextPrimary
        };
        circles = circles.push(
            container(
                text(avatar.glyph)
                    .size(11)
                    .font(ui(theme, font::Weight::Semibold))
                    .color(ink.color(theme)),
            )
            .width(24)
            .height(24)
            .center_x(24)
            .center_y(24)
            .style(move |_| filled(theme, fill, Some(stroke), 12.0)),
        );
    }
    let count_badge = PosReport::owned(
        part("toolbar-online-count"),
        text(format!("{total} 人在线"))
            .size(11.5)
            .font(ui(theme, font::Weight::Medium))
            .color(ColorToken::TextSecondary.color(theme)),
    );
    let group = PosReport::owned(
        part("toolbar-online-avatars"),
        container(
            row![circles, count_badge]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
        )
        .padding([3, 8])
        .style(move |_| {
            filled(
                theme,
                ColorToken::BgApp,
                Some(ColorToken::Border),
                NumberToken::RadiusSm.value(theme),
            )
        }),
    );
    Some(group.into())
}

// ── #51: Read-only banner above canvas ───────────────────────────────────

/// Model of the read-only banner shown above the drawing canvas (#51).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadonlyBannerInfo {
    /// Server's verbatim reason string.
    pub reason: String,
    /// Display name / user id of whoever holds the edit right (if any).
    pub holder: Option<String>,
}

pub(crate) fn readonly_banner_info(app: &OpenCADStudio) -> Option<ReadonlyBannerInfo> {
    if !app.collab.readonly {
        return None;
    }
    let reason = app
        .collab
        .readonly_reason
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "当前会话为只读，无法修改图纸".to_owned());
    let holder = app
        .collab
        .readonly_holder
        .as_deref()
        .map(|sid| app.collab.format_session_user(sid));
    Some(ReadonlyBannerInfo { reason, holder })
}

pub(crate) fn readonly_banner(app: &OpenCADStudio) -> Option<Element<'_, AppMessage>> {
    let info = readonly_banner_info(app)?;
    let theme = app.pid_shell.theme;
    let mut line = row![
        container(
            text("只读")
                .size(11)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::Amber.color(theme)),
        )
        .padding([2, 6])
        .style(move |_| filled(theme, ColorToken::AmberSoft, Some(ColorToken::Amber), 3.0)),
        text(info.reason)
            .size(12)
            .font(ui(theme, font::Weight::Medium))
            .color(ColorToken::TextPrimary.color(theme)),
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center);
    if let Some(holder) = info.holder {
        line = line.push(Space::new().width(Fill));
        line = line.push(
            text(format!("编辑权：{holder}"))
                .size(11.5)
                .font(ui(theme, font::Weight::Medium))
                .color(ColorToken::Amber.color(theme)),
        );
    }
    let bar = PosReport::owned(
        part("readonly-banner"),
        container(line)
            .width(Fill)
            .padding([7, 12])
            .style(move |_| filled(theme, ColorToken::AmberSoft, Some(ColorToken::Amber), 0.0)),
    );
    Some(bar.into())
}

// ── #52: Three-tier read-only breakdown ──────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TierVerdict {
    Allowed,
    Denied,
    NotEvaluated,
}

impl TierVerdict {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Allowed => "允许",
            Self::Denied => "拒绝",
            Self::NotEvaluated => "未判定",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadonlyTierRow {
    pub tier: &'static str,
    pub verdict: TierVerdict,
    pub detail: Option<String>,
}

/// Build the 3-tier authorization breakdown (`角色权限` / `PBS 节点 ACL` /
/// `图纸状态`) for a read-only session (#52).
pub(crate) fn readonly_tiers(state: &CollabState) -> Vec<ReadonlyTierRow> {
    if !state.readonly {
        return Vec::new();
    }
    let level = state.readonly_level.as_deref().unwrap_or("state");
    let reason = state.readonly_reason.clone();
    match level {
        "role" => vec![
            ReadonlyTierRow {
                tier: "角色权限",
                verdict: TierVerdict::Denied,
                detail: reason.or_else(|| {
                    Some(format!(
                        "角色不允许编辑图纸（支持角色：{}）",
                        ROLE_NAMES.join("/")
                    ))
                }),
            },
            ReadonlyTierRow {
                tier: "PBS 节点 ACL",
                verdict: TierVerdict::NotEvaluated,
                detail: None,
            },
            ReadonlyTierRow {
                tier: "图纸状态",
                verdict: TierVerdict::NotEvaluated,
                detail: None,
            },
        ],
        "pbs" => {
            let mut detail = reason.unwrap_or_else(|| "所在 PBS 节点拒绝编辑".to_owned());
            if let Some(node) = &state.readonly_effective_node {
                if !node.trim().is_empty() {
                    detail = format!("{detail} · 生效节点：{node}");
                }
            }
            vec![
                ReadonlyTierRow {
                    tier: "角色权限",
                    verdict: TierVerdict::Allowed,
                    detail: None,
                },
                ReadonlyTierRow {
                    tier: "PBS 节点 ACL",
                    verdict: TierVerdict::Denied,
                    detail: Some(detail),
                },
                ReadonlyTierRow {
                    tier: "图纸状态",
                    verdict: TierVerdict::NotEvaluated,
                    detail: None,
                },
            ]
        }
        _ => {
            let mut detail = reason.unwrap_or_else(|| "图纸当前不可写入".to_owned());
            if let Some(holder) = state.readonly_holder.as_deref() {
                let who = state.format_session_user(holder);
                detail = format!("{detail} · 编辑权：{who}");
            }
            vec![
                ReadonlyTierRow {
                    tier: "角色权限",
                    verdict: TierVerdict::Allowed,
                    detail: None,
                },
                ReadonlyTierRow {
                    tier: "PBS 节点 ACL",
                    verdict: TierVerdict::Allowed,
                    detail: None,
                },
                ReadonlyTierRow {
                    tier: "图纸状态",
                    verdict: TierVerdict::Denied,
                    detail: Some(detail),
                },
            ]
        }
    }
}

// ── #57: Remote claim check on currently selected object ─────────────────

/// Returns `Some((holder_label, expires_at))` if `tag` is currently claimed by
/// another user on the session's drawing (#57).
pub(crate) fn remote_claim_on_tag(app: &OpenCADStudio, tag: &str) -> Option<(String, Option<i64>)> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    let active_id = app.tabs.get(app.active_tab)?.id;
    if let Some(doc_id) = app.collab.document {
        if doc_id != active_id {
            return None;
        }
    }
    let holder = app.collab.claims.get(tag)?;
    if holder == &app.collab.user_id {
        return None;
    }
    let holder_label = app.collab.format_user(holder);
    let expiry = app.collab.claim_expiry.get(tag).copied();
    Some((holder_label, expiry))
}

/// Top notice in the Inspector when the selected object is claimed by a peer (#57).
pub(crate) fn inspector_remote_claim_notice<'a>(
    theme: PidTheme,
    holder: &str,
    expires_at: Option<i64>,
) -> Element<'a, AppMessage> {
    let until = expires_at
        .map(|ts| format!("，到期 {}", format_hhmm(ts)))
        .unwrap_or_default();
    PosReport::owned(
        part("inspector-claim-notice"),
        container(
            text(format!("正被 {holder} 占用{until} · 属性只读"))
                .size(11.5)
                .font(ui(theme, font::Weight::Medium))
                .color(CLAIM_COLOR_TOKEN.color(theme)),
        )
        .width(Fill)
        .padding([8, 10])
        .style(move |_| {
            filled(
                theme,
                ColorToken::AmberSoft,
                Some(CLAIM_COLOR_TOKEN),
                NumberToken::RadiusSm.value(theme),
            )
        }),
    )
    .into()
}

// ── #63 & #64: Page connector section in Inspector & canvas badges ───────

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectorInspectorInfo {
    pub handle: document::Handle,
    pub kind_label: &'static str,
    pub direction_label: &'static str,
    pub line_tag: String,
    pub target_drawing: Option<String>,
    pub peer: Option<String>,
    pub paired: bool,
    pub unpaired_reason: Option<String>,
    pub propagated_fields: Vec<(String, String)>,
}

/// Inspect a selected entity as a page connector if it is recorded in
/// `TopologyTable::connectors` (#63).
pub(crate) fn inspect_connector(
    app: &OpenCADStudio,
    doc: &document::Document,
    handle: document::Handle,
    author: &PidAuthor,
) -> Option<ConnectorInspectorInfo> {
    let table = TopologyTable::load(doc);
    let htext = handle_text(handle);
    let rec = table
        .connectors
        .iter()
        .find(|c| c.handle.eq_ignore_ascii_case(&htext))?;
    let kind = rec.kind_of()?;
    let is_signal = kind.host_role() == ROLE_SIGNAL_LINE;
    let kind_label = if is_signal { "信号线" } else { "管道" };
    let direction_label = if kind.is_inflow() { "流入" } else { "流出" };
    let (own_props, _) = split_props(&author.props);
    let target_drawing = rec
        .peer_drawing
        .clone()
        .or_else(|| own_props.get("target_drawing").cloned())
        .filter(|s| !s.trim().is_empty());
    let paired = rec.peer.is_some() || rec.peer_drawing.is_some();
    let unpaired_reason = if paired {
        None
    } else {
        app.collab
            .unpaired_connectors
            .iter()
            .find(|u| u.line_tag == rec.line_tag && u.connector == rec.kind)
            .map(|u| u.reason.as_str().to_owned())
    };

    // Read carried fields from the host line (or connector props fallback).
    let host_props = doc
        .entities()
        .find_map(|e| {
            let a = PidAuthor::load(doc, e.common().handle)?;
            (a.tag == rec.line_tag && a.topology == kind.host_role())
                .then(|| split_props(&a.props).0)
        })
        .unwrap_or_default();
    let field_names: &[&str] = if is_signal {
        &ocs_collab::link::SIGNAL_FIELDS
    } else {
        &LINE_FIELDS
    };
    let propagated_fields = field_names
        .iter()
        .map(|&k| {
            let val = host_props
                .get(k)
                .or_else(|| own_props.get(k))
                .cloned()
                .unwrap_or_default();
            (k.to_owned(), val)
        })
        .collect();

    Some(ConnectorInspectorInfo {
        handle,
        kind_label,
        direction_label,
        line_tag: rec.line_tag.clone(),
        target_drawing,
        peer: rec.peer.clone(),
        paired,
        unpaired_reason,
        propagated_fields,
    })
}

/// Render the Inspector's page-connector section with 【跳转】 button (#63, #64).
pub(crate) fn connector_inspector_section<'a>(
    theme: PidTheme,
    info: &ConnectorInspectorInfo,
    draft: String,
) -> Element<'a, AppMessage> {
    let kv = |k: &'static str, v: String| {
        row![
            text(k)
                .size(11.5)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
            Space::new().width(Fill),
            text(v)
                .size(11.5)
                .font(mono(theme, font::Weight::Normal))
                .color(ColorToken::TextPrimary.color(theme)),
        ]
        .width(Fill)
    };
    let status_text = if info.paired {
        format!(
            "已配对（{}）",
            info.peer
                .as_deref()
                .or(info.target_drawing.as_deref())
                .unwrap_or("-")
        )
    } else if let Some(reason) = &info.unpaired_reason {
        format!("未配对 · {reason}")
    } else {
        "未配对".to_owned()
    };
    let mut section = column![
        row![
            text("页间连接符")
                .size(11)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextMuted.color(theme)),
            Space::new().width(Fill),
            PosReport::owned(
                part("connector-jump"),
                button(
                    container(
                        text("跳转")
                            .size(11)
                            .font(ui(theme, font::Weight::Medium))
                            .color(ColorToken::Accent.color(theme)),
                    )
                    .padding([3, 8])
                    .style(move |_| filled(
                        theme,
                        ColorToken::AccentSoft,
                        Some(ColorToken::Accent),
                        3.0
                    )),
                )
                .padding(0)
                .style(bare)
                .on_press(send_editor(EditorMsg::JumpConnector(info.handle))),
            ),
        ]
        .align_y(alignment::Vertical::Center),
        kv("种类", info.kind_label.to_owned()),
        kv("流向", info.direction_label.to_owned()),
        kv(
            "目标图纸",
            info.target_drawing
                .clone()
                .unwrap_or_else(|| "（未指定）".to_owned()),
        ),
        kv("对端状态", status_text),
        {
            let submitted = draft.clone();
            let handle = info.handle;
            text_input("目标图号", &draft)
                .on_input(move |text| {
                    send_editor(EditorMsg::FieldInput {
                        handle,
                        field: "connector_target".to_owned(),
                        text,
                    })
                })
                .on_submit(send_editor(EditorMsg::SetConnectorTarget {
                    handle,
                    drawing: submitted,
                }))
        },
        {
            let drawing = draft.clone();
            button(
                text("配对")
                    .size(11)
                    .font(ui(theme, font::Weight::Medium))
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 8])
            .style(bare)
            .on_press(send_editor(EditorMsg::SetConnectorTarget {
                handle: info.handle,
                drawing,
            }))
        },
    ]
    .spacing(6);

    if !info.propagated_fields.is_empty() {
        section = section.push(
            text("传递属性")
                .size(10.5)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextMuted.color(theme)),
        );
        for (k, v) in &info.propagated_fields {
            let shown = if v.is_empty() { "—".to_owned() } else { v.clone() };
            section = section.push(
                row![
                    text(k.clone())
                        .size(11)
                        .font(ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextSecondary.color(theme)),
                    Space::new().width(Fill),
                    text(shown)
                        .size(11)
                        .font(mono(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .width(Fill),
            );
        }
    }

    PosReport::owned(
        part("inspector-connector"),
        container(section).width(Fill).padding(16),
    )
    .into()
}

// ── #48, #52, #53, #54, #55, #62, #78: Inspector Collab Section ──────────

/// Concise collab-section status text — deliberately phrased to complement
/// (never repeat verbatim) the shell navigation foot's `SyncStatus::label()` (#48).
pub(crate) fn collab_section_status(state: &CollabState) -> (String, ColorToken) {
    if !state.connected {
        if state.last_error.as_deref() == Some("人数已满") {
            return ("人数已满".to_owned(), ColorToken::Danger);
        }
        let queued = state.outbox.len();
        let text = if queued > 0 {
            format!(
                "离线 · 重试第 {} 次 · 已排队 {queued} 步",
                state.reconnect_attempts.max(1)
            )
        } else if state.reconnect_attempts > 0 {
            format!("离线 · 正在重试第 {} 次", state.reconnect_attempts)
        } else {
            "正在建立协同会话…".to_owned()
        };
        return (text, ColorToken::Danger);
    }
    if state.mismatch().is_some() {
        return (
            format!("同步暂停 · 图号 {}", state.drawing_id),
            ColorToken::Danger,
        );
    }
    if let Some(target) = state.catching_up {
        return (
            format!("正在补发 · 目标修订 {target}"),
            ColorToken::Amber,
        );
    }
    if state.readonly {
        return (
            format!("只读会话 · 图号 {}", state.drawing_id),
            ColorToken::Amber,
        );
    }
    let queued = state.outbox.len();
    if queued > 0 {
        (
            format!("在线 · 图号 {} · 待提交 {queued} 步", state.drawing_id),
            ColorToken::Amber,
        )
    } else {
        (
            format!("在线 · 图号 {}", state.drawing_id),
            ColorToken::Success,
        )
    }
}

/// Lower half of the Inspector when a collaboration server is configured
/// (#48, #52, #53, #54, #55, #62, #78).
pub(crate) fn inspector_collab_section<'a>(
    app: &'a OpenCADStudio,
    selected_tag: Option<&str>,
) -> Option<Element<'a, AppMessage>> {
    let state = &app.collab;
    if !state.wanted() {
        return None;
    }
    let theme = app.pid_shell.theme;
    let (status_text, status_dot) = collab_section_status(state);

    let mut content = column![
        // Section header + status line (#48)
        PosReport::owned(
            part("collab-status"),
            row![
                container(Space::new())
                    .width(7)
                    .height(7)
                    .style(move |_| filled(theme, status_dot, None, 3.5)),
                text(status_text)
                    .size(11.5)
                    .font(ui(theme, font::Weight::Semibold))
                    .color(ColorToken::TextPrimary.color(theme)),
            ]
            .spacing(7)
            .align_y(alignment::Vertical::Center),
        ),
    ]
    .spacing(10);

    // Divergence / Realign box (#78)
    if let Some(reason) = state.mismatch() {
        content = content.push(
            PosReport::owned(
                part("collab-diverged"),
                container(
                    column![
                        text(format!("与房间不一致：{reason}"))
                            .size(11)
                            .font(ui(theme, font::Weight::Normal))
                            .color(ColorToken::Danger.color(theme)),
                        PosReport::owned(
                            part("collab-realign"),
                            button(
                                container(
                                    text("重新对齐")
                                        .size(11)
                                        .font(ui(theme, font::Weight::Medium))
                                        .color(ColorToken::TextOnDanger.color(theme)),
                                )
                                .padding([4, 10])
                                .style(move |_| filled(theme, ColorToken::Danger, None, 3.0)),
                            )
                            .padding(0)
                            .style(bare)
                            .on_press(PidCollabMsg::Realign.message()),
                        ),
                    ]
                    .spacing(6),
                )
                .width(Fill)
                .padding(8)
                .style(move |_| {
                    filled(
                        theme,
                        ColorToken::DangerSoft,
                        Some(ColorToken::Danger),
                        NumberToken::RadiusSm.value(theme),
                    )
                }),
            ),
        );
    }

    // Queue / Replay / Rejections summary (#53)
    if !state.outbox.is_empty() || state.catching_up.is_some() || !state.rejection_log.is_empty() {
        let mut qcol = column![].spacing(4);
        if let Some(target) = state.catching_up {
            qcol = qcol.push(
                text(format!("正在补发历史变更（至修订 {target}）…"))
                    .size(11)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::Amber.color(theme)),
            );
        }
        if !state.outbox.is_empty() {
            qcol = qcol.push(
                text(format!("已排队 {} 步修改等待提交", state.outbox.len()))
                    .size(11)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextSecondary.color(theme)),
            );
        }
        if !state.rejection_log.is_empty() {
            let latest = state
                .rejection_log
                .last()
                .and_then(|v| v["message"].as_str())
                .unwrap_or("修改被服务器拒绝");
            qcol = qcol.push(PosReport::owned(
                part("collab-rejections"),
                text(format!(
                    "被拒 {} 步 · 最近：{latest}",
                    state.rejection_log.len()
                ))
                .size(11)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::Danger.color(theme)),
            ));
        }
        content = content.push(PosReport::owned(
            part("collab-queue"),
            container(qcol).width(Fill).padding([6, 8]).style(move |_| {
                filled(
                    theme,
                    ColorToken::BgRaised,
                    Some(ColorToken::Border),
                    NumberToken::RadiusSm.value(theme),
                )
            }),
        ));
    }

    // Three-tier read-only breakdown (#52)
    let tiers = readonly_tiers(state);
    if !tiers.is_empty() {
        let mut tcol = column![text("只读原因（三级权限链）")
            .size(11)
            .font(ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextMuted.color(theme))]
        .spacing(4);
        for row_item in tiers {
            let badge_color = match row_item.verdict {
                TierVerdict::Allowed => ColorToken::Success,
                TierVerdict::Denied => ColorToken::Danger,
                TierVerdict::NotEvaluated => ColorToken::TextMuted,
            };
            tcol = tcol.push(
                row![
                    text(row_item.tier)
                        .size(11)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ColorToken::TextSecondary.color(theme)),
                    Space::new().width(Fill),
                    text(row_item.verdict.label())
                        .size(11)
                        .font(ui(theme, font::Weight::Semibold))
                        .color(badge_color.color(theme)),
                ]
                .width(Fill),
            );
            if let Some(detail) = row_item.detail {
                tcol = tcol.push(
                    text(detail)
                        .size(10.5)
                        .font(ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextMuted.color(theme)),
                );
            }
        }
        content = content.push(PosReport::owned(
            part("collab-readonly-tiers"),
            container(tcol).width(Fill).padding(8).style(move |_| {
                filled(
                    theme,
                    ColorToken::BgRaised,
                    Some(ColorToken::Amber),
                    NumberToken::RadiusSm.value(theme),
                )
            }),
        ));
    }

    // Online roster (#48): self first (marked 本机), then peers in join order.
    let online_count = if state.session_id.is_some() {
        1 + state.peers.len()
    } else {
        0
    };
    let mut roster = column![text(format!("在线用户 {online_count}"))
        .size(11)
        .font(ui(theme, font::Weight::Semibold))
        .color(ColorToken::TextMuted.color(theme))]
    .spacing(4);
    if state.session_id.is_some() {
        let own_label = format_user_label(&state.user_id, &state.display_name);
        roster = roster.push(PosReport::owned(
            part("collab-peer-self"),
            row![
                text(own_label)
                    .size(11.5)
                    .font(ui(theme, font::Weight::Medium))
                    .color(ColorToken::TextPrimary.color(theme)),
                Space::new().width(Fill),
                text("本机")
                    .size(10.5)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::Accent.color(theme)),
            ]
            .width(Fill)
            .align_y(alignment::Vertical::Center),
        ));
        for peer in &state.peers {
            let label = format_user_label(&peer.user_id, &peer.display_name);
            let is_holder = state.readonly
                && state
                    .readonly_holder
                    .as_deref()
                    .is_some_and(|h| h == peer.session_id);
            let name_color = if peer.disconnected {
                ColorToken::TextMuted
            } else {
                ColorToken::TextPrimary
            };
            let mut peer_line = row![
                text(label)
                    .size(11.5)
                    .font(ui(theme, font::Weight::Normal))
                    .color(name_color.color(theme)),
                Space::new().width(Fill),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center);
            if peer.disconnected {
                let badge = match peer.hold_until {
                    Some(ts) if ts > 0 => format!("连接中断 · 至 {}", format_hhmm(ts)),
                    _ => "连接中断".to_owned(),
                };
                peer_line = peer_line.push(PosReport::owned(
                    part(&format!("collab-peer-disconnected-{}", peer.session_id)),
                    text(badge)
                        .size(10)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ColorToken::Amber.color(theme)),
                ));
            }
            if is_holder {
                peer_line = peer_line.push(
                    text("编辑权")
                        .size(10.5)
                        .font(ui(theme, font::Weight::Semibold))
                        .color(CLAIM_COLOR_TOKEN.color(theme)),
                );
            }
            roster = roster.push(PosReport::owned(
                part(&format!("collab-peer-{}", peer.session_id)),
                peer_line,
            ));
        }
        if state.peers.is_empty() {
            roster = roster.push(PosReport::owned(
                part("collab-no-others"),
                text(NO_OTHERS_LABEL)
                    .size(11)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextMuted.color(theme)),
            ));
        }
    }
    content = content.push(PosReport::owned(part("collab-roster"), roster));

    // Pending conflicts (#62): hidden when empty.
    if !state.conflicts.is_empty() {
        let mut ccol = column![text(format!("待裁决冲突 {}", state.conflicts.len()))
            .size(11)
            .font(ui(theme, font::Weight::Semibold))
            .color(ColorToken::Danger.color(theme))]
        .spacing(6);
        for conflict in &state.conflicts {
            ccol = ccol.push(PosReport::owned(
                part(&format!("collab-conflict-{}", conflict.conflict_id)),
                container(
                    column![
                        text(format!("{} · {}", conflict.object_ref, conflict.field))
                            .size(11.5)
                            .font(mono(theme, font::Weight::Semibold))
                            .color(ColorToken::TextPrimary.color(theme)),
                        text(format!("{} ↔ {}", conflict.value_a, conflict.value_b))
                            .size(11)
                            .font(mono(theme, font::Weight::Normal))
                            .color(ColorToken::Amber.color(theme)),
                        text(format!("ID: {}", conflict.conflict_id))
                            .size(10)
                            .font(mono(theme, font::Weight::Normal))
                            .color(ColorToken::TextMuted.color(theme)),
                        button(
                            text(OPEN_CONFLICT_LIST)
                                .size(10.5)
                                .font(ui(theme, font::Weight::Medium))
                                .color(ColorToken::Accent.color(theme)),
                        )
                        .padding([2, 8])
                        .style(bare)
                        .on_press(PidCollabMsg::OpenConflictList.message()),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding(6)
                .style(move |_| {
                    filled(
                        theme,
                        ColorToken::DangerSoft,
                        Some(ColorToken::Danger),
                        3.0,
                    )
                }),
            ));
        }
        content = content.push(PosReport::owned(part("collab-conflicts"), ccol));
    }
    if let Some(items) = &app.pid_collab_conflict_list {
        let mut list = column![
            text("冲突列表")
                .size(11)
                .font(ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextPrimary.color(theme)),
            button(
                text("待裁决")
                    .size(10.5)
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 6])
            .style(bare)
            .on_press(PidCollabMsg::ShowPendingConflicts.message()),
            button(
                text("已裁决")
                    .size(10.5)
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 6])
            .style(bare)
            .on_press(PidCollabMsg::ShowRuledConflicts.message()),
        ]
        .spacing(4);
        if items.is_empty() && !app.pid_collab_list_ruled {
            list = list.push(
                text(NO_PENDING_CONFLICTS)
                    .size(11)
                    .color(ColorToken::TextPrimary.color(theme)),
            );
            list = list.push(
                text(NO_PENDING_CONFLICTS_WHY)
                    .size(10.5)
                    .color(ColorToken::TextMuted.color(theme)),
            );
        }
        for item in items {
            let mut lines = column![].spacing(1);
            for line in item.lines().into_iter().chain(
                (item.status == "已裁决")
                    .then(|| item.ruling_lines())
                    .into_iter()
                    .flatten(),
            ) {
                lines = lines.push(
                    text(line)
                        .size(10.5)
                        .font(mono(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                );
            }
            list = list.push(PosReport::owned(
                part(&format!("collab-conflict-list-{}", item.conflict_id)),
                lines,
            ));
        }
        list = list.push(
            button(
                text("返回")
                    .size(10.5)
                    .color(ColorToken::TextSecondary.color(theme)),
            )
            .padding(0)
            .style(bare)
            .on_press(PidCollabMsg::CloseConflictList.message()),
        );
        content = content.push(PosReport::owned(part("collab-conflict-list"), list));
    }
    if let Some(dialog) = &app.pid_collab_ruling {
        let item = &dialog.item;
        let mut dialog_col = column![
            text(format!("甲方的值 {}", item.value_a))
                .size(11)
                .color(ColorToken::TextPrimary.color(theme)),
            text(format!("乙方的值 {}", item.value_b))
                .size(11)
                .color(ColorToken::TextPrimary.color(theme)),
            text(format!("这条的基线修订号 {}", item.base_revision))
                .size(10.5)
                .color(ColorToken::TextMuted.color(theme)),
            button(
                text(ADOPT_A)
                    .size(10.5)
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 8])
            .style(bare)
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_A.to_owned()).message()),
            button(
                text(ADOPT_B)
                    .size(10.5)
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 8])
            .style(bare)
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_B.to_owned()).message()),
            button(
                text(ADOPT_MANUAL)
                    .size(10.5)
                    .color(ColorToken::Accent.color(theme)),
            )
            .padding([2, 8])
            .style(bare)
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_MANUAL.to_owned()).message()),
        ]
        .spacing(2);
        if dialog.choice.as_deref() == Some(ADOPT_MANUAL) {
            dialog_col = dialog_col.push(
                text_input("最终值", &dialog.manual)
                    .on_input(|value| PidCollabMsg::SetRulingManual(value).message()),
            );
        }
        if dialog.choice.is_some() {
            dialog_col = dialog_col.push(
                button(
                    text(CONFIRM_RULING)
                        .size(10.5)
                        .color(ColorToken::Accent.color(theme)),
                )
                .padding([2, 8])
                .style(bare)
                .on_press(PidCollabMsg::ConfirmRuling.message()),
            );
        }
        content = content.push(PosReport::owned(part("collab-ruling"), dialog_col));
    }

    // Tag claims list + Claim / Release action for selected object (#54, #55)
    let mut claims_col = column![].spacing(6);
    let mut claims_header = row![
        text(format!("位号占用 {}", state.claims.len()))
            .size(11)
            .font(ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextMuted.color(theme)),
        Space::new().width(Fill),
    ]
    .align_y(alignment::Vertical::Center);
    if let Some(tag) = selected_tag.filter(|t| !t.trim().is_empty() && *t != "？") {
        let held_by_self = state
            .claims
            .get(tag)
            .is_some_and(|holder| holder == &state.user_id);
        let (btn_id, btn_label, msg) = if held_by_self {
            (
                "collab-release-selected",
                "释放",
                PidCollabMsg::Release(tag.to_owned()).message(),
            )
        } else {
            (
                "collab-claim-selected",
                "占用",
                PidCollabMsg::Claim(tag.to_owned()).message(),
            )
        };
        claims_header = claims_header.push(PosReport::owned(
            part(btn_id),
            button(
                container(
                    text(btn_label)
                        .size(10.5)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ColorToken::Accent.color(theme)),
                )
                .padding([2, 8])
                .style(move |_| filled(theme, ColorToken::AccentSoft, Some(ColorToken::Accent), 3.0)),
            )
            .padding(0)
            .style(bare)
            .on_press_maybe(state.connected.then_some(msg)),
        ));
    }
    claims_col = claims_col.push(claims_header);
    for (tag, holder) in &state.claims {
        let is_self = holder == &state.user_id;
        let who = state.format_user(holder);
        let exp = state
            .claim_expiry
            .get(tag)
            .map(|ts| format!(" · 到期 {}", format_hhmm(*ts)))
            .unwrap_or_default();
        let mut crow = row![
            column![
                text(tag.clone())
                    .size(11.5)
                    .font(mono(theme, font::Weight::Semibold))
                    .color(CLAIM_COLOR_TOKEN.color(theme)),
                text(format!("{who}{exp}"))
                    .size(10.5)
                    .font(ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextSecondary.color(theme)),
            ]
            .spacing(1),
            Space::new().width(Fill),
        ]
        .align_y(alignment::Vertical::Center);
        if is_self {
            crow = crow.push(
                button(
                    container(
                        text("释放")
                            .size(10.5)
                            .font(ui(theme, font::Weight::Medium))
                            .color(ColorToken::TextSecondary.color(theme)),
                    )
                    .padding([2, 6])
                    .style(move |_| {
                        filled(theme, ColorToken::BgRaised, Some(ColorToken::Border), 3.0)
                    }),
                )
                .padding(0)
                .style(bare)
                .on_press(PidCollabMsg::Release(tag.clone()).message()),
            );
        }
        claims_col = claims_col.push(PosReport::owned(
            part(&format!("collab-claim-{tag}")),
            crow,
        ));
    }
    if state.claims.is_empty() {
        claims_col = claims_col.push(
            text("当前无位号占用")
                .size(11)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme)),
        );
    }
    content = content.push(PosReport::owned(part("collab-claims"), claims_col));

    Some(
        PosReport::owned(
            part("collab-section"),
            container(scrollable(content).height(Length::Fill))
                .width(Fill)
                .height(Length::FillPortion(2))
                .padding(14),
        )
        .into(),
    )
}

// ── #55 & #63: Canvas claim & page-connector markers ─────────────────────

/// A claim badge positioned on the drawing canvas (#55).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CanvasClaimBadge {
    pub tag: String,
    pub label: String,
    pub screen: Point,
    pub is_self: bool,
}

/// Compute screen-space claim badges (`占用者 · 到期 HH:MM`) for the active
/// drawing when it is the session's drawing (#55).
pub(crate) fn canvas_claim_badges(app: &OpenCADStudio) -> Vec<CanvasClaimBadge> {
    if app.collab.claims.is_empty() {
        return Vec::new();
    }
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Vec::new();
    };
    if tab.is_start {
        return Vec::new();
    }
    if let Some(doc_id) = app.collab.document {
        if doc_id != tab.id {
            return Vec::new();
        }
    }
    let (vw, vh) = tab.scene.selection.borrow().view.vp_size;
    let doc = &tab.scene.document;
    let mut out = Vec::new();
    for (tag, holder) in &app.collab.claims {
        let Some(world_pt) = doc.entities().find_map(|entity| {
            let author = PidAuthor::load(doc, entity.common().handle)?;
            if author.tag != *tag {
                return None;
            }
            let aabb = crate::scene::entity_aabb(entity);
            Some(glam::DVec3::new(f64::from(aabb[2]), f64::from(aabb[3]), 0.0))
        }) else {
            continue;
        };
        let screen = if vw > 0.0 && vh > 0.0 {
            let rect = iced::Rectangle::new(Point::ORIGIN, iced::Size::new(vw, vh));
            match tab.scene.camera.borrow().project(world_pt, rect) {
                Some(p) => Point::new(
                    p.x.clamp(8.0, (vw - 140.0).max(8.0)),
                    p.y.clamp(8.0, (vh - 28.0).max(8.0)),
                ),
                None => Point::new(16.0, 52.0),
            }
        } else {
            Point::new(16.0, 52.0)
        };
        let who = app.collab.format_user(holder);
        let hhmm = app
            .collab
            .claim_expiry
            .get(tag)
            .map(|ts| format_hhmm(*ts))
            .unwrap_or_else(|| "--:--".to_owned());
        out.push(CanvasClaimBadge {
            tag: tag.clone(),
            label: format!("{who} · 到期 {hhmm}"),
            screen,
            is_self: holder == &app.collab.user_id,
        });
    }
    out
}

/// A page-connector pairing badge positioned on the drawing canvas (#63).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CanvasConnectorBadge {
    pub handle: String,
    pub label: String,
    pub paired: bool,
    pub is_signal: bool,
    pub screen: Point,
}

/// Compute screen-space pairing badges (`未配对` vs `至 <图号>` / `自 <图号>`)
/// for every page connector on the active drawing (#63).
pub(crate) fn canvas_connector_badges(app: &OpenCADStudio) -> Vec<CanvasConnectorBadge> {
    let Some(tab) = app.tabs.get(app.active_tab) else {
        return Vec::new();
    };
    if tab.is_start {
        return Vec::new();
    }
    let (vw, vh) = tab.scene.selection.borrow().view.vp_size;
    let doc = &tab.scene.document;
    let table = TopologyTable::load(doc);
    let mut out = Vec::new();
    for rec in &table.connectors {
        let Some(handle) = parse_handle(&rec.handle) else {
            continue;
        };
        let Some(entity) = doc.get_entity(handle) else {
            continue;
        };
        let aabb = crate::scene::entity_aabb(entity);
        let world_pt = glam::DVec3::new(f64::from(aabb[2]), f64::from(aabb[3]), 0.0);
        let screen = if vw > 0.0 && vh > 0.0 {
            let rect = iced::Rectangle::new(Point::ORIGIN, iced::Size::new(vw, vh));
            match tab.scene.camera.borrow().project(world_pt, rect) {
                Some(p) => Point::new(
                    p.x.clamp(8.0, (vw - 120.0).max(8.0)),
                    p.y.clamp(8.0, (vh - 28.0).max(8.0)),
                ),
                None => Point::new(16.0, 84.0),
            }
        } else {
            Point::new(16.0, 84.0)
        };
        let kind = rec.kind_of().unwrap_or(ConnectorKind::PipeOut);
        let is_signal = kind.host_role() == ROLE_SIGNAL_LINE;
        let peer_drawing = rec.peer_drawing.clone().or_else(|| {
            rec.peer
                .as_deref()
                .and_then(|p| p.split_once(':'))
                .map(|(d, _)| d.to_owned())
        });
        let (paired, label) = match peer_drawing.filter(|s| !s.trim().is_empty()) {
            Some(drawing) => {
                let prefix = if kind.is_inflow() { "自" } else { "至" };
                (true, format!("{prefix} {drawing}"))
            }
            None => (false, "未配对".to_owned()),
        };
        out.push(CanvasConnectorBadge {
            handle: rec.handle.clone(),
            label,
            paired,
            is_signal,
            screen,
        });
    }
    out
}

/// Overlay layer combining canvas claim badges (#55), page-connector pairing
/// badges (#63), and stacked collaboration toasts (#50, #58–#61, #65).
pub(crate) fn canvas_collab_overlays(app: &OpenCADStudio) -> Vec<Element<'_, AppMessage>> {
    let theme = app.pid_shell.theme;
    let mut layers: Vec<Element<'_, AppMessage>> = Vec::new();

    // Claim markers (#55, #56)
    for badge in canvas_claim_badges(app) {
        let chip = PosReport::owned(
            part(&format!("canvas-claim-{}", badge.tag)),
            container(
                text(badge.label)
                    .size(10.5)
                    .font(ui(theme, font::Weight::Medium))
                    .color(CLAIM_COLOR_TOKEN.color(theme)),
            )
            .padding([2, 6])
            .style(move |_| filled(theme, ColorToken::AmberSoft, Some(CLAIM_COLOR_TOKEN), 3.0)),
        );
        layers.push(pin(chip).position(badge.screen).into());
    }

    // Page connector pairing badges (#63, #56)
    for badge in canvas_connector_badges(app) {
        let accent = if !badge.paired {
            ColorToken::TextMuted
        } else if badge.is_signal {
            CONNECTOR_SIGNAL_COLOR_TOKEN
        } else {
            CONNECTOR_PIPE_COLOR_TOKEN
        };
        let chip = PosReport::owned(
            part(&format!("canvas-connector-{}", badge.handle)),
            container(
                text(badge.label)
                    .size(10.5)
                    .font(mono(theme, font::Weight::Medium))
                    .color(accent.color(theme)),
            )
            .padding([2, 6])
            .style(move |_| filled(theme, ColorToken::BgPanel, Some(accent), 3.0)),
        );
        layers.push(pin(chip).position(badge.screen).into());
    }

    // Stacked toast notifications at top-right of canvas (#50, #58–#61, #65)
    let toasts = app.collab.toasts.items();
    if !toasts.is_empty() {
        let mut stack_col = column![].spacing(6).align_x(alignment::Horizontal::Right);
        for toast in toasts {
            let (fill, stroke, ink) = match toast.kind {
                ToastKind::Info => (ColorToken::BgPanel, ColorToken::Accent, ColorToken::TextPrimary),
                ToastKind::Success => (ColorToken::SuccessSoft, ColorToken::Success, ColorToken::TextPrimary),
                ToastKind::Warning => (ColorToken::AmberSoft, ColorToken::Amber, ColorToken::TextPrimary),
                ToastKind::Error => (ColorToken::DangerSoft, ColorToken::Danger, ColorToken::TextPrimary),
            };
            let id = toast.id;
            let card = PosReport::owned(
                part(&format!("toast-{id}")),
                container(
                    row![
                        text(toast.message.clone())
                            .size(11.5)
                            .font(ui(theme, font::Weight::Normal))
                            .color(ink.color(theme)),
                        button(
                            text("×")
                                .size(12)
                                .font(ui(theme, font::Weight::Bold))
                                .color(ColorToken::TextMuted.color(theme)),
                        )
                        .padding([0, 4])
                        .style(bare)
                        .on_press(send_editor(EditorMsg::DismissToast(id))),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([6, 10])
                .style(move |_| {
                    filled(theme, fill, Some(stroke), NumberToken::RadiusSm.value(theme))
                }),
            );
            stack_col = stack_col.push(card);
        }
        let toast_layer = container(iced::widget::opaque(stack_col))
            .width(Fill)
            .padding(iced::Padding {
                top: 12.0,
                right: 14.0,
                bottom: 0.0,
                left: 0.0,
            })
            .align_x(alignment::Horizontal::Right);
        layers.push(toast_layer.into());
    }

    layers
}

// ── #66: Field last-writer annotation & inline modification history UI ───

/// Format the last writer caption shown beside a property field label (#66).
pub(crate) fn field_writer_caption(
    app: &OpenCADStudio,
    tag: &str,
    field: &str,
) -> Option<String> {
    let w: &FieldLastWriter = app
        .collab
        .field_writers
        .get(&(tag.to_owned(), field.to_owned()))?;
    let who = if !w.display_name.trim().is_empty() {
        format_user_label(&w.user_id, &w.display_name)
    } else {
        app.collab.format_user(&w.user_id)
    };
    Some(format!("{who} · r{}", w.revision))
}

/// Client-side filtering of already-fetched audit rows by field substring and
/// action kind (#66).
pub(crate) fn filter_history_rows<'a>(
    rows: &'a [AuditItem],
    filter: &HistoryFilter,
) -> Vec<&'a AuditItem> {
    let field_q = filter.field_query.trim().to_ascii_lowercase();
    let action_q = filter.action_kind.trim().to_ascii_lowercase();
    rows.iter()
        .filter(|row| {
            let field_ok = field_q.is_empty()
                || row.change_text.to_ascii_lowercase().contains(&field_q)
                || row.object_ref.to_ascii_lowercase().contains(&field_q);
            let action_ok = action_q.is_empty()
                || row.action.to_ascii_lowercase().contains(&action_q)
                || row.action_label().to_ascii_lowercase().contains(&action_q);
            field_ok && action_ok
        })
        .collect()
}

/// Inline modification history drawer rendered inside the Inspector when opened
/// from the selected object (#66). Not mounted in the left navigation bar.
pub(crate) fn history_panel<'a>(app: &'a OpenCADStudio) -> Element<'a, AppMessage> {
    let theme = app.pid_shell.theme;
    let target = app
        .collab
        .history_target
        .as_deref()
        .unwrap_or("（未选择）");
    let filter = &app.collab.history_filter;
    let visible = filter_history_rows(&app.collab.history_rows, filter);

    let input_box = |placeholder: &'static str,
                     val: &str,
                     on_change: fn(String) -> EditorMsg| {
        container(
            text_input(placeholder, val)
                .size(11)
                .font(ui(theme, font::Weight::Normal))
                .padding([4, 6])
                .on_input(move |s| send_editor(on_change(s))),
        )
        .width(Fill)
    };

    let header = row![
        text(format!("修改历史 · {target}"))
            .size(12.5)
            .font(ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextPrimary.color(theme)),
        Space::new().width(Fill),
        button(
            text("返回")
                .size(11)
                .font(ui(theme, font::Weight::Medium))
                .color(ColorToken::Accent.color(theme)),
        )
        .padding([2, 6])
        .style(bare)
        .on_press(send_editor(EditorMsg::CloseHistory)),
    ]
    .align_y(alignment::Vertical::Center);

    let filters = column![
        row![
            input_box("用户筛选", &filter.user_id, EditorMsg::HistoryFilterUser),
            button(
                container(
                    text("查询")
                        .size(11)
                        .font(ui(theme, font::Weight::Medium))
                        .color(ColorToken::TextOnAccent.color(theme)),
                )
                .padding([4, 8])
                .style(move |_| filled(theme, ColorToken::Accent, None, 3.0)),
            )
            .padding(0)
            .style(bare)
            .on_press(send_editor(EditorMsg::RefreshHistory)),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
        row![
            input_box("起时间(Unix)", &filter.from_text, EditorMsg::HistoryFilterFrom),
            input_box("止时间(Unix)", &filter.to_text, EditorMsg::HistoryFilterTo),
        ]
        .spacing(6),
        row![
            input_box("字段筛选", &filter.field_query, EditorMsg::HistoryFilterField),
            input_box("操作种类", &filter.action_kind, EditorMsg::HistoryFilterAction),
        ]
        .spacing(6),
    ]
    .spacing(6);

    let mut list = column![].spacing(6);
    if visible.is_empty() {
        list = list.push(
            text("暂无符合条件的修改记录")
                .size(11.5)
                .font(ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme)),
        );
    } else {
        for item in visible {
            let who = format_user_label(&item.user_id, &item.display_name);
            let when = format_timestamp(item.at);
            list = list.push(
                container(
                    column![
                        row![
                            text(item.action_label().to_owned())
                                .size(11)
                                .font(ui(theme, font::Weight::Semibold))
                                .color(ColorToken::Accent.color(theme)),
                            Space::new().width(Fill),
                            text(when)
                                .size(10)
                                .font(mono(theme, font::Weight::Normal))
                                .color(ColorToken::TextMuted.color(theme)),
                        ],
                        text(who)
                            .size(11)
                            .font(ui(theme, font::Weight::Medium))
                            .color(ColorToken::TextSecondary.color(theme)),
                        text(item.change_text.clone())
                            .size(11)
                            .font(mono(theme, font::Weight::Normal))
                            .color(ColorToken::TextPrimary.color(theme)),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding(8)
                .style(move |_| {
                    filled(
                        theme,
                        ColorToken::BgRaised,
                        Some(ColorToken::Border),
                        NumberToken::RadiusSm.value(theme),
                    )
                }),
            );
        }
    }

    PosReport::owned(
        part("history-panel"),
        container(
            column![header, filters, scrollable(list).height(Fill)].spacing(10),
        )
        .width(Fill)
        .height(Fill)
        .padding(14),
    )
    .into()
}

// ── Tests covering #48–#66 ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::collab::{CollabEvent, PeerInfo, PendingConflict, UnpairedConnectorInfo, UnpairedReason};

    fn pid_app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.open_first_screen(&crate::cli::GuiConfig::default());
        app
    }

    #[test]
    fn ui_49_toolbar_online_avatars_hidden_when_alone_and_correct_for_two() {
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.user_id = "u1".into();
        app.collab.display_name = "张工".into();
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        assert!(online_avatars(&app).is_empty(), "alone -> no avatar strip");

        app.collab.apply(CollabEvent::PeerJoined(PeerInfo {
            session_id: "s-2".into(),
            user_id: "wang".into(),
            display_name: String::new(),
            disconnected: false,
            hold_until: None,
        }));
        let avatars = online_avatars(&app);
        assert_eq!(avatars.len(), 2);
        assert_eq!(avatars[0].glyph, "张");
        assert_eq!(avatars[1].glyph, "W");
    }

    #[test]
    fn a_library_mismatch_is_named_in_the_collab_zone() {
        let mut app = pid_app();
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "内置库与这张图上已有的编辑会话不一致：符号库".into(),
            holder: None,
            level: Some("state".into()),
            code: Some("PERM_DENIED_STATE".into()),
        });
        let info = readonly_banner_info(&app).expect("the zone says why it is read-only");
        assert!(info.reason.contains("符号库"), "{}", info.reason);
    }

    #[test]
    fn a_full_room_is_shown_as_人数已满() {
        let mut app = pid_app();
        app.collab.apply(CollabEvent::ServerError {
            code: ocs_collab::protocol::codes::SESSION_FULL.to_owned(),
            message: "人数已满".to_owned(),
        });
        assert_eq!(collab_section_status(&app.collab).0, "人数已满");
    }

    #[test]
    fn ui_50_peer_join_and_leave_emit_stacked_toasts() {
        let mut app = pid_app();
        app.collab.apply(CollabEvent::PeerJoined(PeerInfo {
            session_id: "s-2".into(),
            user_id: "u2".into(),
            display_name: "李工".into(),
            disconnected: false,
            hold_until: None,
        }));
        app.collab.apply(CollabEvent::PeerLeft {
            session_id: "s-2".into(),
        });
        let items = app.collab.toasts.items();
        assert_eq!(items.len(), 2);
        assert!(items[0].message.contains("李工（u2）") && items[0].message.contains("加入"));
        assert!(items[1].message.contains("李工（u2）") && items[1].message.contains("离开"));
        let status = app.collab.status_json();
        assert_eq!(status["toasts"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn ui_51_and_52_readonly_banner_and_three_tier_breakdown() {
        let mut app = pid_app();
        app.collab.user_id = "u3".into();
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-3".into(),
            base_revision: 1,
            repo_id: "r-1".into(),
            peers: vec![PeerInfo {
                session_id: "s-1".into(),
                user_id: "u1".into(),
                display_name: "张工".into(),
                disconnected: false,
                hold_until: None,
            }],
            claims: Vec::new(),
        });

        // Role tier
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "校对人员不允许编辑图纸".into(),
            holder: None,
            level: Some("role".into()),
            code: Some("PERM_DENIED_ROLE".into()),
        });
        let banner = readonly_banner_info(&app).expect("banner shown");
        assert_eq!(banner.reason, "校对人员不允许编辑图纸");
        assert_eq!(banner.holder, None);
        let tiers = readonly_tiers(&app.collab);
        assert_eq!(tiers[0].verdict, TierVerdict::Denied);
        assert_eq!(tiers[1].verdict, TierVerdict::NotEvaluated);
        assert_eq!(tiers[2].verdict, TierVerdict::NotEvaluated);

        // PBS tier with effective node
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "节点拒绝编辑".into(),
            holder: None,
            level: Some("pbs".into()),
            code: Some("PERM_DENIED_PBS".into()),
        });
        app.collab.apply(CollabEvent::ReadonlyEffectiveNode {
            node: "/装置一/单元A".into(),
        });
        let tiers = readonly_tiers(&app.collab);
        assert_eq!(tiers[0].verdict, TierVerdict::Allowed);
        assert_eq!(tiers[1].verdict, TierVerdict::Denied);
        assert!(tiers[1].detail.as_deref().unwrap().contains("/装置一/单元A"));
        assert_eq!(tiers[2].verdict, TierVerdict::NotEvaluated);

        // State tier (single editor held by u1)
        app.collab.apply(CollabEvent::ReadonlyDowngrade {
            reason: "图纸处于单人编辑".into(),
            holder: Some("s-1".into()),
            level: Some("state".into()),
            code: Some("PERM_DENIED_STATE".into()),
        });
        let banner = readonly_banner_info(&app).expect("banner shown");
        assert_eq!(banner.holder.as_deref(), Some("张工（u1）"));
        let tiers = readonly_tiers(&app.collab);
        assert_eq!(tiers[0].verdict, TierVerdict::Allowed);
        assert_eq!(tiers[1].verdict, TierVerdict::Allowed);
        assert_eq!(tiers[2].verdict, TierVerdict::Denied);

        // Recovery clears banner
        app.collab.readonly = false;
        assert!(readonly_banner_info(&app).is_none());
    }

    #[test]
    fn ui_56_claim_color_comes_from_theme_tokens_and_differs_from_connector_lines() {
        for theme in PidTheme::ALL {
            let claim = CLAIM_COLOR_TOKEN.rgba(theme);
            let pipe = CONNECTOR_PIPE_COLOR_TOKEN.rgba(theme);
            let signal = CONNECTOR_SIGNAL_COLOR_TOKEN.rgba(theme);
            assert_ne!(claim, pipe, "claim color must differ from pipe connector in {theme:?}");
            assert_ne!(claim, signal, "claim color must differ from signal connector in {theme:?}");
        }
    }

    #[test]
    fn ui_57_remote_claim_makes_selected_object_readonly_until_released() {
        let mut app = pid_app();
        app.collab.user_id = "u1".into();
        app.collab.document = Some(app.tabs[app.active_tab].id);
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: vec![PeerInfo {
                session_id: "s-2".into(),
                user_id: "u2".into(),
                display_name: "李工".into(),
                disconnected: false,
                hold_until: None,
            }],
            claims: vec![("V-1001".into(), "u2".into())],
        });
        app.collab.apply(CollabEvent::ClaimsExpire(vec![("V-1001".into(), 1_700_000_300)]));

        let remote = remote_claim_on_tag(&app, "V-1001").expect("claimed by peer u2");
        assert_eq!(remote.0, "李工（u2）");
        assert_eq!(remote.1, Some(1_700_000_300));

        // Own claim is NOT treated as remote-blocked.
        app.collab.apply(CollabEvent::ClaimGranted { tag: "P-1001".into() });
        assert!(remote_claim_on_tag(&app, "P-1001").is_none());

        // Release clears remote block.
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "V-1001".into(),
            reason: "released".into(),
        });
        assert!(remote_claim_on_tag(&app, "V-1001").is_none());
    }

    #[test]
    fn ui_58_all_five_claim_outcomes_and_reset_emit_toasts() {
        let mut app = pid_app();
        app.collab.user_id = "u1".into();
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        // 1. Granted
        app.collab.apply(CollabEvent::ClaimGranted { tag: "P-1001".into() });
        // 2. Denied
        app.collab.apply(CollabEvent::ClaimDenied {
            tag: "P-1002".into(),
            holder: "u2".into(),
            expires_at: 1_700_000_300,
        });
        // 3. Released
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".into(),
            reason: "released".into(),
        });
        // 4. Expired
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1002".into(),
            reason: "expired".into(),
        });
        // 5. Step rolled back for CLAIM_HELD
        app.report_claimed_step(&[], "PIDSETPROP", "P-1003", "u2");

        let msgs: Vec<String> = app
            .collab
            .toasts
            .items()
            .iter()
            .map(|t| t.message.clone())
            .collect();
        assert!(msgs.iter().any(|m| m.contains("已占用位号 P-1001")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("P-1002") && m.contains("到期")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("P-1001") && m.contains("释放")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("P-1002") && m.contains("到期自动释放")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("P-1003") && m.contains("已撤回")), "{msgs:?}");

        // Session reset after holding a claim notifies user that old claim was released.
        app.collab.toasts.clear();
        app.collab.apply(CollabEvent::ClaimGranted { tag: "P-1009".into() });
        app.collab.apply(CollabEvent::Disconnected {
            reason: "session expired".into(),
            will_retry: true,
        });
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-2".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });
        assert!(
            app.collab
                .toasts
                .items()
                .iter()
                .any(|t| t.message.contains("P-1009") && t.message.contains("已释放")),
            "reconnect after lost claim must notify user"
        );
    }

    #[test]
    fn ui_62_and_65_conflicts_and_unpaired_connectors_track_cleanly() {
        let mut app = pid_app();
        app.collab.apply(CollabEvent::Conflict(PendingConflict {
            conflict_id: "c-1".into(),
            object_ref: "P-1001".into(),
            field: "设计压力".into(),
            value_a: "1.6MPa".into(),
            value_b: "2.5MPa".into(),
        }));
        assert_eq!(app.collab.conflicts.len(), 1);

        // All three unpaired connector reasons (#32, #65)
        for reason in [
            UnpairedReason::TargetDrawingNotRegistered,
            UnpairedReason::NoMatchingPeerConnector,
            UnpairedReason::PeerAlreadyPaired,
        ] {
            app.collab.record_unpaired_connector(UnpairedConnectorInfo {
                line_tag: "L-1001".into(),
                connector: "pipe_out".into(),
                target_drawing: "PID-1102".into(),
                reason,
            });
            assert!(app
                .collab
                .toasts
                .items()
                .last()
                .unwrap()
                .message
                .contains(reason.as_str()));
        }
        app.collab.clear_unpaired_connector("L-1001", "pipe_out");
        assert!(app.collab.unpaired_connectors.is_empty());
    }

    /// #81: 去裁决 fills the collab-zone list from the query stand-in, and
    /// the rows sit one under the other.
    #[test]
    fn conflict_list_from_the_query_stand_in_is_drawn_row_by_row() {
        use crate::app::collab::query::{CollabQueryClient, ConflictListItem, QueryError};
        use crate::ui::window::pid_collab::PidCollabMsg;

        #[derive(Debug)]
        struct StandIn {
            items: Vec<ConflictListItem>,
        }

        impl CollabQueryClient for StandIn {
            fn fetch_pending_conflicts(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<PendingConflict>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_conflict_list(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<ConflictListItem>, QueryError> {
                Ok(self.items.clone())
            }
            fn resolve_conflict(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::ResolveOutcome, QueryError> {
                Err(QueryError::transport("stub"))
            }
            fn fetch_audit(
                &self,
                _: &str,
                _: &crate::app::collab::query::AuditFilter,
            ) -> Result<Vec<crate::app::collab::query::AuditItem>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_drawing_objects(
                &self,
                _: &str,
                drawing_id: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::DrawingObjectsPage, QueryError> {
                Ok(crate::app::collab::query::DrawingObjectsPage {
                    drawing_id: drawing_id.to_owned(),
                    revision: 0,
                    items: Vec::new(),
                })
            }
        }

        let item = ConflictListItem {
            conflict_id: "c-7f3a".to_owned(),
            object_ref: "P-1001".to_owned(),
            field: "设计压力".to_owned(),
            value_a: "1.6MPa".to_owned(),
            value_b: "2.5MPa".to_owned(),
            base_revision: 4,
            status: "待裁决".to_owned(),
            resolution: String::new(),
            resolved_value: String::new(),
            resolved_by: String::new(),
        };
        let second = ConflictListItem {
            conflict_id: "c-8a01".to_owned(),
            object_ref: "L-1001".to_owned(),
            field: "topology".to_owned(),
            value_a: "from A".to_owned(),
            value_b: "from B".to_owned(),
            base_revision: 4,
            status: "待裁决".to_owned(),
            resolution: String::new(),
            resolved_value: String::new(),
            resolved_by: String::new(),
        };
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.drawing_id = "PID-1101".into();
        app.collab.set_query_client(std::sync::Arc::new(StandIn {
            items: vec![item.clone(), second.clone()],
        }));
        let _ = app.on_pid_collab(PidCollabMsg::OpenConflictList);
        assert_eq!(
            app.pid_collab_conflict_list.as_deref(),
            Some([item.clone(), second.clone()].as_slice())
        );

        let element = inspector_collab_section(&app, None).expect("the collab zone");
        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (280.0, 900.0),
            element,
        );
        let find = |ui: &mut iced_test::simulator::Simulator<'_, AppMessage>, label: &str| {
            ui.find(label).ok().and_then(|found| found.visible_bounds())
        };
        let first = find(&mut ui, "c-7f3a").expect("first list row");
        let next = find(&mut ui, "c-8a01").expect("second list row");
        assert!(first.y < next.y, "{first:?} {next:?}");
        assert!(find(&mut ui, "基线修订号 4").is_some());
        assert!(find(&mut ui, "状态 待裁决").is_some());
    }

    /// #82: the dialog shows both values and the baseline. Confirming 采用乙方
    /// sends the same resolve call as `PIDCOLLAB RESOLVE`, and the list loses
    /// that row.
    #[test]
    fn confirming_adopt_b_sends_the_same_resolve_as_the_command() {
        use crate::app::collab::query::{
            CollabQueryClient, ConflictListItem, QueryError, ResolveOutcome,
        };
        use crate::ui::window::pid_collab::{PidCollabMsg, ADOPT_B};
        use std::sync::{Arc, Mutex};

        #[derive(Debug)]
        struct Rec {
            item: ConflictListItem,
            calls: Mutex<Vec<(String, String, Option<String>)>>,
        }

        impl CollabQueryClient for Rec {
            fn fetch_pending_conflicts(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<PendingConflict>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_conflict_list(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<ConflictListItem>, QueryError> {
                Ok(vec![self.item.clone()])
            }
            fn resolve_conflict(
                &self,
                _: &str,
                conflict_id: &str,
                resolution: &str,
                _: &str,
                value: Option<&str>,
            ) -> Result<ResolveOutcome, QueryError> {
                self.calls.lock().expect("calls").push((
                    conflict_id.to_owned(),
                    resolution.to_owned(),
                    value.map(str::to_owned),
                ));
                Ok(ResolveOutcome {
                    conflict_id: conflict_id.to_owned(),
                    status: "已裁决".to_owned(),
                    resolution: resolution.to_owned(),
                    resolved_by: "u4".to_owned(),
                    object_ref: self.item.object_ref.clone(),
                    field: self.item.field.clone(),
                    value: self.item.value_b.clone(),
                    revision: 5,
                })
            }
            fn fetch_audit(
                &self,
                _: &str,
                _: &crate::app::collab::query::AuditFilter,
            ) -> Result<Vec<crate::app::collab::query::AuditItem>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_drawing_objects(
                &self,
                _: &str,
                drawing_id: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::DrawingObjectsPage, QueryError> {
                Ok(crate::app::collab::query::DrawingObjectsPage {
                    drawing_id: drawing_id.to_owned(),
                    revision: 0,
                    items: Vec::new(),
                })
            }
        }

        let item = ConflictListItem {
            conflict_id: "c-7f3a".to_owned(),
            object_ref: "P-1001".to_owned(),
            field: "设计压力".to_owned(),
            value_a: "1.6MPa".to_owned(),
            value_b: "2.5MPa".to_owned(),
            base_revision: 4,
            status: "待裁决".to_owned(),
            resolution: String::new(),
            resolved_value: String::new(),
            resolved_by: String::new(),
        };
        let rec = Arc::new(Rec {
            item: item.clone(),
            calls: Mutex::new(Vec::new()),
        });
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.drawing_id = "PID-1101".into();
        app.collab.user_id = "u4".into();
        app.collab.set_query_client(rec.clone());
        let _ = app.on_pid_collab(PidCollabMsg::OpenConflictList);
        let _ = app.on_pid_collab(PidCollabMsg::OpenRuling(item.conflict_id.clone()));
        let _ = app.on_pid_collab(PidCollabMsg::ChooseRuling(ADOPT_B.to_owned()));

        let element = inspector_collab_section(&app, None).expect("the collab zone");
        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (280.0, 900.0),
            element,
        );
        let find = |ui: &mut iced_test::simulator::Simulator<'_, AppMessage>, label: &str| {
            ui.find(label).ok().and_then(|found| found.visible_bounds())
        };
        let party_a = find(&mut ui, "甲方的值 1.6MPa").expect("party A");
        let party_b = find(&mut ui, "乙方的值 2.5MPa").expect("party B");
        let baseline = find(&mut ui, "这条的基线修订号 4").expect("baseline");
        assert!(party_a.y < party_b.y && party_b.y < baseline.y);
        drop(ui);

        let _ = app.on_pid_collab(PidCollabMsg::ConfirmRuling);
        app.resolve_conflict_cmd("c-7f3a 采用乙方");
        let calls = rec.calls.lock().expect("calls");
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(
            calls[0], calls[1],
            "the dialog and the command send one action"
        );
        assert_eq!(calls[0].1, ADOPT_B);
        assert!(
            app.pid_collab_conflict_list
                .as_ref()
                .is_some_and(|list| list.iter().all(|row| row.conflict_id != "c-7f3a")),
            "the ruled row leaves the list"
        );
    }

    /// #83: a typed value is the same resolve as the command. An empty
    /// value, a second ruling, and a role refusal each say so and leave the
    /// row pending.
    #[test]
    fn a_manual_value_and_the_three_refusals_stay_on_the_pending_row() {
        use crate::app::collab::query::{
            CollabQueryClient, ConflictListItem, QueryError, ResolveOutcome,
        };
        use crate::ui::window::pid_collab::{PidCollabMsg, ADOPT_A, ADOPT_MANUAL};
        use std::sync::{Arc, Mutex};

        #[derive(Debug)]
        struct Rec {
            calls: Mutex<Vec<(String, String, Option<String>)>>,
        }

        impl CollabQueryClient for Rec {
            fn fetch_pending_conflicts(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<PendingConflict>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_conflict_list(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<ConflictListItem>, QueryError> {
                Ok(["c-ok", "c-dup", "c-perm"]
                    .into_iter()
                    .map(|id| ConflictListItem {
                        conflict_id: id.to_owned(),
                        object_ref: "P-1001".to_owned(),
                        field: "设计压力".to_owned(),
                        value_a: "1.6MPa".to_owned(),
                        value_b: "2.5MPa".to_owned(),
                        base_revision: 4,
                        status: "待裁决".to_owned(),
                        resolution: String::new(),
                        resolved_value: String::new(),
                        resolved_by: String::new(),
                    })
                    .collect())
            }
            fn resolve_conflict(
                &self,
                _: &str,
                conflict_id: &str,
                resolution: &str,
                _: &str,
                value: Option<&str>,
            ) -> Result<ResolveOutcome, QueryError> {
                if conflict_id == "c-dup" {
                    return Err(QueryError {
                        status: 409,
                        code: "CONFLICT_ALREADY_RESOLVED".into(),
                        message: "该冲突条目已经裁决过".into(),
                        level: None,
                        detail: None,
                    });
                }
                if conflict_id == "c-perm" {
                    return Err(QueryError {
                        status: 403,
                        code: "PERM_DENIED_ROLE".into(),
                        message: "角色不允许授权配置".into(),
                        level: Some("role".into()),
                        detail: None,
                    });
                }
                self.calls.lock().expect("calls").push((
                    conflict_id.to_owned(),
                    resolution.to_owned(),
                    value.map(str::to_owned),
                ));
                Ok(ResolveOutcome {
                    conflict_id: conflict_id.to_owned(),
                    status: "已裁决".into(),
                    resolution: resolution.to_owned(),
                    resolved_by: "u4".into(),
                    value: value.unwrap_or("").to_owned(),
                    object_ref: "P-1001".into(),
                    field: "设计压力".into(),
                    revision: 6,
                })
            }
            fn fetch_audit(
                &self,
                _: &str,
                _: &crate::app::collab::query::AuditFilter,
            ) -> Result<Vec<crate::app::collab::query::AuditItem>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_drawing_objects(
                &self,
                _: &str,
                drawing_id: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::DrawingObjectsPage, QueryError> {
                Ok(crate::app::collab::query::DrawingObjectsPage {
                    drawing_id: drawing_id.to_owned(),
                    revision: 0,
                    items: Vec::new(),
                })
            }
        }

        let still = |app: &OpenCADStudio, id: &str| {
            app.pid_collab_conflict_list.as_ref().is_some_and(|list| {
                list.iter()
                    .any(|row| row.conflict_id == id && row.status == "待裁决")
            })
        };
        let rec = Arc::new(Rec {
            calls: Mutex::new(Vec::new()),
        });
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.drawing_id = "PID-1101".into();
        app.collab.user_id = "u4".into();
        app.collab.set_query_client(rec.clone());
        let _ = app.on_pid_collab(PidCollabMsg::OpenConflictList);

        let _ = app.on_pid_collab(PidCollabMsg::OpenRuling("c-ok".into()));
        let _ = app.on_pid_collab(PidCollabMsg::ChooseRuling(ADOPT_MANUAL.into()));
        let _ = app.on_pid_collab(PidCollabMsg::ConfirmRuling);
        assert!(
            app.command_line
                .last_error
                .as_deref()
                .is_some_and(|text| text.contains("必须提供最终值")),
            "{:?}",
            app.command_line.last_error
        );
        assert!(rec.calls.lock().expect("calls").is_empty());
        assert!(still(&app, "c-ok"));

        let _ = app.on_pid_collab(PidCollabMsg::SetRulingManual("3.2MPa".into()));
        let _ = app.on_pid_collab(PidCollabMsg::ConfirmRuling);
        app.resolve_conflict_cmd("c-ok 人工改值 3.2MPa");
        let calls = rec.calls.lock().expect("calls").clone();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert_eq!(calls[0], calls[1]);
        assert_eq!(calls[0].2.as_deref(), Some("3.2MPa"));
        assert!(!still(&app, "c-ok"));

        let _ = app.on_pid_collab(PidCollabMsg::OpenRuling("c-dup".into()));
        let _ = app.on_pid_collab(PidCollabMsg::ChooseRuling(ADOPT_A.into()));
        let _ = app.on_pid_collab(PidCollabMsg::ConfirmRuling);
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("CONFLICT_ALREADY_RESOLVED")
        );
        assert!(app
            .command_line
            .last_error
            .as_deref()
            .is_some_and(|text| text.contains("已经裁决过")));
        assert!(still(&app, "c-dup"));

        let _ = app.on_pid_collab(PidCollabMsg::OpenRuling("c-perm".into()));
        let _ = app.on_pid_collab(PidCollabMsg::ChooseRuling(ADOPT_A.into()));
        let _ = app.on_pid_collab(PidCollabMsg::ConfirmRuling);
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("PERM_DENIED_ROLE")
        );
        assert!(app
            .command_line
            .last_error
            .as_deref()
            .is_some_and(|text| text.contains("角色不允许")));
        assert!(still(&app, "c-perm"));
    }

    /// #84: 待裁决 and 已裁决 are two pages of the same list. A ruled row
    /// shows the result, the value that stood, and who ruled, matching the query.
    #[test]
    fn the_conflict_list_switches_to_the_ruled_page() {
        use crate::app::collab::query::{CollabQueryClient, ConflictListItem, QueryError};
        use crate::ui::window::pid_collab::PidCollabMsg;

        #[derive(Debug)]
        struct Pages;

        fn row(status: &str) -> ConflictListItem {
            ConflictListItem {
                conflict_id: "c-7f3a".into(),
                object_ref: "P-1001".into(),
                field: "设计压力".into(),
                value_a: "1.6MPa".into(),
                value_b: "2.5MPa".into(),
                base_revision: 4,
                status: status.into(),
                resolution: if status == "已裁决" {
                    "采用乙方".into()
                } else {
                    String::new()
                },
                resolved_value: if status == "已裁决" {
                    "2.5MPa".into()
                } else {
                    String::new()
                },
                resolved_by: if status == "已裁决" {
                    "u4".into()
                } else {
                    String::new()
                },
            }
        }

        impl CollabQueryClient for Pages {
            fn fetch_pending_conflicts(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<PendingConflict>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_conflict_list(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<ConflictListItem>, QueryError> {
                Ok(vec![row("待裁决")])
            }
            fn fetch_ruled_conflicts(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<ConflictListItem>, QueryError> {
                Ok(vec![row("已裁决")])
            }
            fn resolve_conflict(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::ResolveOutcome, QueryError> {
                Err(QueryError::transport("stub"))
            }
            fn fetch_audit(
                &self,
                _: &str,
                _: &crate::app::collab::query::AuditFilter,
            ) -> Result<Vec<crate::app::collab::query::AuditItem>, QueryError> {
                Ok(Vec::new())
            }
            fn fetch_drawing_objects(
                &self,
                _: &str,
                drawing_id: &str,
                _: Option<&str>,
            ) -> Result<crate::app::collab::query::DrawingObjectsPage, QueryError> {
                Ok(crate::app::collab::query::DrawingObjectsPage {
                    drawing_id: drawing_id.into(),
                    revision: 0,
                    items: Vec::new(),
                })
            }
        }

        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.drawing_id = "PID-1101".into();
        app.collab.set_query_client(std::sync::Arc::new(Pages));
        let _ = app.on_pid_collab(PidCollabMsg::OpenConflictList);
        assert_eq!(
            app.pid_collab_conflict_list.as_ref().unwrap()[0].status,
            "待裁决"
        );
        let _ = app.on_pid_collab(PidCollabMsg::ShowRuledConflicts);
        let shown = &app.pid_collab_conflict_list.as_ref().unwrap()[0];
        assert_eq!(shown.resolution, "采用乙方");
        assert_eq!(shown.resolved_value, "2.5MPa");
        assert_eq!(shown.resolved_by, "u4");
        let element = inspector_collab_section(&app, None).unwrap();
        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (280.0, 900.0),
            element,
        );
        for label in ["结果 采用乙方", "最终值 2.5MPa", "裁决人 u4"] {
            assert!(ui.find(label).is_ok(), "missing {label}");
        }
        drop(ui);
        let _ = app.on_pid_collab(PidCollabMsg::ShowPendingConflicts);
        assert_eq!(
            app.pid_collab_conflict_list.as_ref().unwrap()[0].status,
            "待裁决"
        );
    }

    #[test]
    fn ui_66_history_filter_and_field_writer_caption() {
        let mut app = pid_app();
        app.collab.field_writers.insert(
            ("P-1001".into(), "设计压力".into()),
            FieldLastWriter {
                value: "2.5MPa".into(),
                revision: 7,
                user_id: "u2".into(),
                display_name: "李工".into(),
            },
        );
        assert_eq!(
            field_writer_caption(&app, "P-1001", "设计压力").as_deref(),
            Some("李工（u2） · r7")
        );

        let rows = vec![
            AuditItem {
                seq: 1,
                action: "set_property".into(),
                object_ref: "P-1001".into(),
                user_id: "u1".into(),
                change_text: "设计压力: 1.0 → 2.0".into(),
                ..AuditItem::default()
            },
            AuditItem {
                seq: 2,
                action: "change_superseded".into(),
                object_ref: "P-1001".into(),
                user_id: "u2".into(),
                change_text: "设计温度: 50 → 80".into(),
                ..AuditItem::default()
            },
        ];
        let filter = HistoryFilter {
            field_query: "设计温度".into(),
            action_kind: "让位".into(),
            ..HistoryFilter::default()
        };
        let filtered = filter_history_rows(&rows, &filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].seq, 2);
    }

    #[test]
    fn ui_48_shell_hides_old_dock_panel_and_renders_inspector_collab_bounds() {
        let _one = crate::headless_ui_lock();
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.user_id = "u1".into();
        app.collab.display_name = "张工".into();
        app.collab.drawing_id = "PID-1101".into();

        // Connecting in P&ID shell must NOT open legacy PanelId::PidCollab dock.
        let task = app.update(AppMessage::CollabEvent(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 4,
            repo_id: "r-1".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        }));
        let _ = app.drive_headless_task(task);
        assert!(!app.show_pid_collab, "legacy docked panel must stay closed in P&ID shell");

        // Shell nav foot and collab section status must not duplicate the exact same text.
        let nav_label = crate::app::pid_shell::chrome::SyncStatus::of(&app).label();
        let (collab_label, _) = collab_section_status(&app.collab);
        assert_ne!(nav_label, collab_label);

        // Render full window with iced_test::Simulator and verify PosReport bounds.
        crate::ui::wrap_bar::forget_bounds("");
        {
            let mut sim = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(1440.0, 900.0),
                app.view_main(),
            );
            let _ = sim.snapshot(&iced::Theme::Dark);
        }
        assert!(
            crate::ui::wrap_bar::window_bounds("PID_EDITOR:collab-section").is_some(),
            "collab-section bounds missing"
        );
        assert!(
            crate::ui::wrap_bar::window_bounds("PID_EDITOR:collab-peer-self").is_some(),
            "collab-peer-self bounds missing"
        );
        assert!(
            crate::ui::wrap_bar::window_bounds("PID_EDITOR:collab-no-others").is_some(),
            "collab-no-others bounds missing"
        );

        // Add peer -> peer row bounds appear and toolbar online avatar group appears (#49).
        app.collab.apply(CollabEvent::PeerJoined(PeerInfo {
            session_id: "s-2".into(),
            user_id: "u2".into(),
            display_name: "李工".into(),
            disconnected: false,
            hold_until: None,
        }));
        crate::ui::wrap_bar::forget_bounds("");
        {
            let mut sim2 = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(1440.0, 900.0),
                app.view_main(),
            );
            let _ = sim2.snapshot(&iced::Theme::Dark);
        }
        assert!(crate::ui::wrap_bar::window_bounds("PID_EDITOR:collab-peer-s-2").is_some());
        assert!(crate::ui::wrap_bar::window_bounds("PID_EDITOR:toolbar-online-avatars").is_some());
        assert!(crate::ui::wrap_bar::window_bounds("PID_EDITOR:toolbar-online-count").is_some());
    }

    #[test]
    fn ui_53_sync_status_shows_retrying_queued_and_catching_up() {
        use crate::app::pid_shell::chrome::SyncStatus;
        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.user_id = "u1".into();
        app.collab.drawing_id = "PID-1101".into();
        app.collab.had_session = true;
        app.collab.connected = false;
        app.collab.reconnect_attempts = 2;
        let _ = app
            .collab
            .queue_op(serde_json::json!({"kind":"set_property"}), vec![1], true, "A");
        let _ = app
            .collab
            .queue_op(serde_json::json!({"kind":"set_property"}), vec![2], true, "B");
        let st = SyncStatus::of(&app);
        assert_eq!(st, SyncStatus::RetryingQueued { failed: 2, queued: 2 });
        assert_eq!(st.label(), "连接已断开，重试第 2 次 · 已排队 2 步");

        app.collab.connected = true;
        app.collab.catching_up = Some(9);
        let st_replay = SyncStatus::of(&app);
        assert_eq!(st_replay, SyncStatus::CatchingUp { target: 9 });
        assert!(st_replay.label().contains("正在补发"));

        app.collab.catching_up = None;
        app.collab.outbox.clear();
        app.collab.base_revision = 9;
        assert_eq!(SyncStatus::of(&app), SyncStatus::Connected { revision: 9 });
    }

    #[test]
    fn ui_54_55_canvas_claim_badges_and_peer_claim_meta() {
        use document::entities::Insert;
        use document::types::Vector3;

        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.user_id = "u1".into();
        app.collab.display_name = "张工".into();
        let i = app.active_tab;
        app.collab.document = Some(app.tabs[i].id);
        {
            let doc = &mut app.tabs[i].scene.document;
            let h = doc
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::new(10.0, 20.0, 0.0),
                )))
                .unwrap();
            PidAuthor {
                type_code: "EQUIPMENT".into(),
                tag: "P-1001".into(),
                tag_source: "auto".into(),
                tag_status: "ok".into(),
                ..PidAuthor::default()
            }
            .store(doc, h);
        }
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: vec![PeerInfo {
                session_id: "s-2".into(),
                user_id: "u2".into(),
                display_name: "李工".into(),
                disconnected: false,
                hold_until: None,
            }],
            claims: Vec::new(),
        });

        // Peer u2 claims P-1001 after we joined (delivered via ClaimGranted + ClaimMeta).
        app.collab.apply(CollabEvent::ClaimGranted { tag: "P-1001".into() });
        app.collab.apply(CollabEvent::ClaimMeta {
            tag: "P-1001".into(),
            session_id: "s-2".into(),
            user_id: Some("u2".into()),
            display_name: Some("李工".into()),
            expires_at: Some(15 * 3600 + 45 * 60),
        });
        let badges = canvas_claim_badges(&app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].tag, "P-1001");
        assert!(badges[0].label.contains("李工（u2）"), "{}", badges[0].label);
        assert!(badges[0].label.contains("15:45"), "{}", badges[0].label);

        // Release removes badge immediately.
        app.collab.apply(CollabEvent::ClaimReleased {
            tag: "P-1001".into(),
            reason: "released".into(),
        });
        assert!(canvas_claim_badges(&app).is_empty());
    }

    #[test]
    fn ui_59_60_61_remote_edit_refusal_and_upstream_propagation_toasts() {
        use document::entities::Insert;
        use document::types::Vector3;

        let mut app = pid_app();
        app.collab.server_url = "ws://127.0.0.1:3240/ws".into();
        app.collab.user_id = "u1".into();
        let i = app.active_tab;
        app.collab.document = Some(app.tabs[i].id);
        {
            let doc = &mut app.tabs[i].scene.document;
            let h = doc
                .add_entity(document::EntityType::Insert(Insert::new(
                    "PID_SYM_equipment_vessel",
                    Vector3::ZERO,
                )))
                .unwrap();
            PidAuthor {
                type_code: "EQUIPMENT".into(),
                tag: "P-1001".into(),
                tag_source: "auto".into(),
                tag_status: "ok".into(),
                props: serde_json::json!({"设计压力": "1.0MPa", "设计温度": "50℃"}).to_string(),
                ..PidAuthor::default()
            }
            .store(doc, h);
        }
        app.collab.apply(CollabEvent::Connected {
            session_id: "s-1".into(),
            base_revision: 0,
            repo_id: "r-1".into(),
            peers: Vec::new(),
            claims: Vec::new(),
        });

        // #59: Peer edits one field -> toast shows old -> new.
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-1".into(),
            revision: 1,
            changes: vec![serde_json::json!({
                "kind": "set_property",
                "object_ref": "P-1001",
                "field": "设计压力",
                "value": "2.0MPa",
            })],
            user_id: Some("u2".into()),
            display_name: Some("李工".into()),
        });
        app.apply_remote_changes();
        assert!(
            app.collab.toasts.items().iter().any(|t| {
                t.message.contains("李工（u2）")
                    && t.message.contains("设计压力")
                    && t.message.contains("1.0MPa")
                    && t.message.contains("2.0MPa")
            }),
            "{:?}",
            app.collab.toasts.items()
        );

        // #59: Multiple changes in one op merge into a single toast with count.
        app.collab.toasts.clear();
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "op-peer-2".into(),
            revision: 2,
            changes: vec![
                serde_json::json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa"}),
                serde_json::json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计温度", "value": "90℃"}),
            ],
            user_id: Some("u2".into()),
            display_name: Some("李工".into()),
        });
        app.apply_remote_changes();
        assert_eq!(app.collab.toasts.items().len(), 1);
        assert!(app.collab.toasts.items()[0].message.contains("2 处"));

        // #61: Upstream connector propagation arrives even on a readonly drawing!
        app.collab.readonly = true;
        app.collab.toasts.clear();
        app.collab.apply(CollabEvent::OpApplied {
            op_id: "link-PID-1101-r3-1".into(),
            revision: 3,
            changes: vec![serde_json::json!({
                "kind": "set_property",
                "object_ref": "P-1001",
                "field": "设计压力",
                "value": "3.0MPa",
                "source_drawing": "PID-1101"
            })],
            user_id: None,
            display_name: None,
        });
        app.apply_remote_changes();
        assert!(
            app.collab.toasts.items().iter().any(|t| {
                t.message.contains("上游图纸 PID-1101")
                    && t.message.contains("2.5MPa")
                    && t.message.contains("3.0MPa")
            }),
            "{:?}",
            app.collab.toasts.items()
        );

        // #60: Superseded, object deleted, readonly refusal all produce toasts.
        app.collab.toasts.clear();
        app.collab.apply(CollabEvent::OpRefused {
            op_id: "op-mine-1".into(),
            code: ocs_collab::protocol::codes::OP_SUPERSEDED.into(),
            message: "设计压力 让位给 u2 的 3.0MPa".into(),
            rejected: false,
        });
        app.collab.apply(CollabEvent::OpRefused {
            op_id: "op-mine-2".into(),
            code: ocs_collab::protocol::codes::OBJECT_DELETED.into(),
            message: "已被 u2 删除".into(),
            rejected: false,
        });
        let _ = app.refuse_if_readonly("PIDSETPROP P-1001 设计压力 9.0MPa");
        let msgs: Vec<String> = app
            .collab
            .toasts
            .items()
            .iter()
            .map(|t| t.message.clone())
            .collect();
        assert!(msgs.iter().any(|m| m.contains("让位")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("已被删除")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("只读")), "{msgs:?}");
    }

    #[test]
    fn ui_63_and_64_connector_canvas_badge_inspector_and_jump_between_tabs() {
        use crate::pid::topology::{ConnectorRecord, ROLE_CONNECTOR_OUT};
        use document::entities::{Circle, Line};
        use document::types::Vector3;

        let mut app = pid_app();
        // Tab 1: PID-1101 with signal line S-2001 + signal_out connector.
        let tab1 = app.active_tab;
        app.tabs[tab1].current_path = Some(std::path::PathBuf::from("PID-1101.dwg"));
        let conn_handle = {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut line = Line::new();
            line.start = Vector3::new(0.0, 0.0, 0.0);
            line.end = Vector3::new(100.0, 0.0, 0.0);
            let line_h = doc.add_entity(document::EntityType::Line(line)).unwrap();
            PidAuthor {
                type_code: "SIGNAL_LINE".into(),
                tag: "S-2001".into(),
                topology: ROLE_SIGNAL_LINE.into(),
                props: serde_json::json!({"起点仪表": "PT-101", "终点仪表": "PIC-101"}).to_string(),
                ..PidAuthor::default()
            }
            .store(doc, line_h);
            let mut circle = Circle::new();
            circle.center = Vector3::new(100.0, 0.0, 0.0);
            circle.radius = 3.0;
            let ch = doc.add_entity(document::EntityType::Circle(circle)).unwrap();
            PidAuthor {
                type_code: "SIGNAL_LINE".into(),
                tag: "S-2001".into(),
                topology: ROLE_CONNECTOR_OUT.into(),
                ..PidAuthor::default()
            }
            .store(doc, ch);
            let mut tbl = TopologyTable::load(doc);
            tbl.connectors.push(ConnectorRecord::new(
                handle_text(ch),
                ConnectorKind::SignalOut,
                "S-2001".into(),
            ));
            tbl.store(doc);
            ch
        };

        // Initially unpaired -> canvas badge says "未配对", jump reports no peer.
        let badges = canvas_connector_badges(&app);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges[0].label, "未配对");
        let err = app.jump_to_connector_peer(tab1, Some(conn_handle)).unwrap_err();
        assert!(err.contains("尚未配对"), "{err}");

        // Inspector shows signal connector metadata including 起点仪表 & 终点仪表.
        app.tabs[tab1].scene.select_entity(conn_handle, true);
        let crate::app::pid_shell::editor::Inspected::Object(obj) =
            crate::app::pid_shell::editor::inspected(&app)
        else {
            panic!("expected inspected object");
        };
        let cinfo = obj.connector.expect("connector section must be present");
        assert_eq!(cinfo.kind_label, "信号线");
        assert_eq!(cinfo.direction_label, "流出");
        assert!(cinfo.propagated_fields.iter().any(|(k, v)| k == "起点仪表" && v == "PT-101"));
        assert!(cinfo.propagated_fields.iter().any(|(k, v)| k == "终点仪表" && v == "PIC-101"));

        // Pair to PID-1102 -> canvas badge becomes "至 PID-1102".
        {
            let doc = &mut app.tabs[tab1].scene.document;
            let mut tbl = TopologyTable::load(doc);
            tbl.connectors[0].peer_drawing = Some("PID-1102".into());
            tbl.connectors[0].peer = Some("PID-1102:S-2001".into());
            tbl.store(doc);
        }
        assert_eq!(canvas_connector_badges(&app)[0].label, "至 PID-1102");

        // Peer drawing not open yet -> clear error telling user how to open.
        let not_open = app.jump_to_connector_peer(tab1, Some(conn_handle)).unwrap_err();
        assert!(not_open.contains("未在本机打开"), "{not_open}");

        // Open second tab named PID-1102 containing S-2001 -> jump switches tab & selects line!
        let _ = app.update(AppMessage::TabNew);
        let tab2 = app.active_tab;
        assert_ne!(tab1, tab2, "TabNew must create a distinct second tab");
        app.tabs[tab2].current_path = Some(std::path::PathBuf::from("PID-1102.dwg"));
        let peer_line = {
            let doc = &mut app.tabs[tab2].scene.document;
            let mut line = Line::new();
            line.start = Vector3::new(0.0, 0.0, 0.0);
            line.end = Vector3::new(80.0, 0.0, 0.0);
            let lh = doc.add_entity(document::EntityType::Line(line)).unwrap();
            PidAuthor {
                type_code: "SIGNAL_LINE".into(),
                tag: "S-2001".into(),
                topology: ROLE_SIGNAL_LINE.into(),
                ..PidAuthor::default()
            }
            .store(doc, lh);
            lh
        };
        app.active_tab = tab1;
        let ok = app.jump_to_connector_peer(tab1, Some(conn_handle)).unwrap();
        assert!(ok.contains("PID-1102"), "{ok}");
        assert_eq!(app.active_tab, tab2, "jump must switch to peer drawing tab");
        assert!(app.tabs[tab2].scene.selected_handles_in_order().contains(&peer_line));
    }
}

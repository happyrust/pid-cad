//! Collaboration panel — docked (`PIDCOLLAB`).
//!
//! Two lists the user needs while a session is live:
//!
//! * **在线用户** — every peer on this drawing, this session marked, each row
//!   showing the user id and, when the server sent one, the display name. On
//!   a drawing this session was downgraded on, the row of whoever holds the
//!   edit right says so, and the banner above says this session is read-only.
//! * **位号占用** — one row per held tag: the tag, who holds it, and a button
//!   that claims it (free / held by somebody else) or releases it (held by
//!   this session). The button is the mouse front of `PIDCLAIM` / `PIDRELEASE`,
//!   so the panel and the command line share one implementation.
//!
//! Under the banner a line says what has not reached the room: how many
//! steps wait to be sent, or, for a drawing apart from the room, why, and
//! how many of its changes never got there (`SPEC-collab-offline-sync.md`
//! §5.6).
//!
//! The panel is the visible half of the claim state the scene also colours
//! (`Scene::set_claim_display`): a tag somebody else holds draws in
//! [`crate::scene::CLAIM_COLOR`], and the same tag appears here.
//!
//! Everything shown comes from [`crate::app::collab::State`]; the panel holds no
//! state of its own beyond which section is expanded, so a reconnect or a
//! server push is reflected on the next frame without any sync code.

use crate::app::Message;
use crate::ui::dock::{Chrome, PanelId};
use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Background, Border, Element, Fill, Length, Theme};

/// What the user did in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidCollabMsg {
    /// Claim `tag` for this session.
    Claim(String),
    /// Release `tag` from this session.
    Release(String),
    /// Show or hide the claims list.
    ToggleClaims,
    /// Realign this session with the room after divergence (#78).
    Realign,
    /// Open the read-only conflict list (#81).
    OpenConflictList,
    /// Leave that list.
    CloseConflictList,
    /// Open the ruling dialog for one list row (#82).
    OpenRuling(String),
    /// Pick 采用甲方 or 采用乙方.
    ChooseRuling(String),
    /// Confirm the pick. Same action as `PIDCOLLAB RESOLVE`.
    ConfirmRuling,
    /// The value typed for 人工改值 (#83).
    SetRulingManual(String),
    /// The pending page of the conflict list (#84).
    ShowPendingConflicts,
    /// The ruled page of the same list (#84).
    ShowRuledConflicts,
    /// A keystroke in the unconfigured panel's server field (#99).
    SetSetupServer(String),
    /// A keystroke in the unconfigured panel's user field (#99).
    SetSetupUser(String),
    /// A keystroke in the unconfigured panel's drawing field (#99).
    SetSetupDrawing(String),
    /// Connect with the three fields (#99).
    ApplySetup,
}

impl PidCollabMsg {
    pub(crate) fn message(self) -> Message {
        Message::PidCollab(self)
    }
}

/// `PosReport` id of the panel body, so a client can locate it on screen.
pub const PANEL_BOUNDS_ID: &str = "pid-collab-panel";

/// `PosReport` id prefix of the claim rows.
pub const ROW_BOUNDS_PREFIX: &str = "pid-collab-row-";

/// `PosReport` id of the row for `tag`.
pub fn row_bounds_id(tag: &str) -> String {
    format!("{ROW_BOUNDS_PREFIX}{tag}")
}

/// `PosReport` id of the read-only badge in the banner. Reported only while
/// the session is read-only, so its presence is the state.
pub const READONLY_BOUNDS_ID: &str = "pid-collab-readonly";

/// `PosReport` id prefix of the edit-right marker on a peer row.
pub const HOLDER_BOUNDS_PREFIX: &str = "pid-collab-holder-";

/// `PosReport` id of the edit-right marker on `user_id`'s row, so a client
/// can read off who the panel shows as the holder.
pub fn holder_bounds_id(user_id: &str) -> String {
    format!("{HOLDER_BOUNDS_PREFIX}{user_id}")
}

/// What the banner says of a read-only session.
pub const READONLY_LABEL: &str = "只读";

/// `PosReport` id of the badge a session wears once its drawing is known to
/// be short of the room's changes, or while the connection that is up is not
/// taken as aligned with the room. Reported only then, so its presence is
/// the state.
pub const DIVERGED_BOUNDS_ID: &str = "pid-collab-diverged";

/// What the banner says of such a session.
pub const DIVERGED_LABEL: &str = "不一致";

/// What the panel says to do about it, under why (`REQ-COLL-015`
/// acceptance 3): the way back is the room's own copy of the drawing.
pub const GET_LATEST_HINT: &str = "需要取得最新图纸";

/// What a peer row says of the session holding the edit right.
pub const HOLDER_LABEL: &str = "编辑权";

/// What the roster says under this session's own row when nobody else is
/// on the drawing.
pub const NO_OTHERS_LABEL: &str = "暂无其他在线用户";

/// What the panel says when no server address is set (`VAL-CLI-017`).
pub const NOT_CONFIGURED_LABEL: &str = "未配置协同服务器";

/// And how to set one, under it (#99).
pub const NOT_CONFIGURED_HINT: &str = "填写下面三项";

/// Which of the three setup fields is still empty. `None` when all three
/// are filled. The panel prints this so a person can see what is missing
/// before connecting (#99).
pub fn setup_gap(server: &str, user: &str, drawing: &str) -> Option<String> {
    let mut missing = Vec::new();
    if server.trim().is_empty() {
        missing.push("服务器地址");
    }
    if user.trim().is_empty() {
        missing.push("用户");
    }
    if drawing.trim().is_empty() {
        missing.push("图号");
    }
    if missing.is_empty() {
        None
    } else {
        Some(format!("还缺：{}", missing.join("、")))
    }
}

/// `PosReport` id of those two lines. Reported only while the panel shows
/// them, so its presence is the state.
pub const NOT_CONFIGURED_BOUNDS_ID: &str = "pid-collab-unconfigured";

/// One peer row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRow {
    pub user_id: String,
    pub display_name: String,
    pub is_self: bool,
    /// This session holds the edit right of a drawing that went read-only for
    /// the others.
    pub is_holder: bool,
    /// True while the peer's network socket dropped and its session/claims are
    /// temporarily retained (#37).
    pub disconnected: bool,
}

/// One claim row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRow {
    pub tag: String,
    pub holder: String,
    /// This session owns the tag, so the row offers "release".
    pub is_self: bool,
}

/// One conflict entry awaiting a ruling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRow {
    pub conflict_id: String,
    pub object_ref: String,
    pub field: String,
    pub value_a: String,
    pub value_b: String,
}

impl ConflictRow {
    /// What is contended: the object and the field.
    pub fn subject(&self) -> String {
        format!("{} · {}", self.object_ref, self.field)
    }

    /// The two values that were written, the first to arrive first.
    pub fn values(&self) -> String {
        format!("{} ↔ {}", self.value_a, self.value_b)
    }
}

/// `PosReport` id prefix of the conflict rows.
pub const CONFLICT_BOUNDS_PREFIX: &str = "pid-collab-conflict-";

/// `PosReport` id of the row for conflict entry `conflict_id`, so a client
/// can tell that the panel shows it (`VAL-CROSS-007`).
pub fn conflict_bounds_id(conflict_id: &str) -> String {
    format!("{CONFLICT_BOUNDS_PREFIX}{conflict_id}")
}

/// The header of the conflicts section. Drawn only while there is an entry.
pub const CONFLICTS_LABEL: &str = "待裁决冲突";

/// The button on a conflict row that opens the list (#81).
pub const OPEN_CONFLICT_LIST: &str = "去裁决";

/// What the list says when the query returned no pending conflict.
pub const NO_PENDING_CONFLICTS: &str = "没有待裁决的冲突";

/// And when one would appear. The tie window is off unless the server is
/// started with one, so a default room never makes an entry.
pub const NO_PENDING_CONFLICTS_WHY: &str = "只有两个人在同一基线修订上把同一字段写成不同的值，并且服务器开了并列窗口，才会出现。并列窗口缺省关闭。";

/// `PosReport` id of one row of the list.
pub fn conflict_list_bounds_id(conflict_id: &str) -> String {
    format!("pid-collab-conflict-list-{conflict_id}")
}

/// The two picks the ruling dialog offers (#82).
pub const ADOPT_A: &str = "采用甲方";
pub const ADOPT_B: &str = "采用乙方";
pub const ADOPT_MANUAL: &str = "人工改值";
pub const CONFIRM_RULING: &str = "确认";

/// One pending or ruled conflict row as `GET /conflicts` returns it (#81, #84).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictListItem {
    pub conflict_id: String,
    pub object_ref: String,
    pub field: String,
    pub value_a: String,
    pub value_b: String,
    pub base_revision: i64,
    pub status: String,
    /// Set once the entry is 已裁决 (#84). Empty while it is still pending.
    pub resolution: String,
    pub resolved_value: String,
    pub resolved_by: String,
}

impl ConflictListItem {
    /// The six facts the list draws, one line each, in the order a reader
    /// meets them.
    pub fn lines(&self) -> [String; 6] {
        [
            self.conflict_id.clone(),
            format!("对象 {}", self.object_ref),
            format!("字段 {}", self.field),
            format!("甲方 {} · 乙方 {}", self.value_a, self.value_b),
            format!("基线修订号 {}", self.base_revision),
            format!("状态 {}", self.status),
        ]
    }

    /// What a ruled entry adds: the result, the value that stood, and who ruled.
    pub fn ruling_lines(&self) -> [String; 3] {
        [
            format!("结果 {}", self.resolution),
            format!("最终值 {}", self.resolved_value),
            format!("裁决人 {}", self.resolved_by),
        ]
    }
}

/// The dialog for one conflict: the query row, which side is picked, and a
/// value typed by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulingDialog {
    pub item: ConflictListItem,
    pub choice: Option<String>,
    pub manual: String,
}

/// `PosReport` id of the line under the banner that says what has not
/// reached the room. Reported only while there is something to say, so its
/// presence is the state.
pub const SYNC_BOUNDS_ID: &str = "pid-collab-sync";

/// What has not reached the room, as the line under the banner says it
/// (`SPEC-collab-offline-sync.md` §5.6). The counts are the ones
/// `GET /api/v1/collab/status` gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncLine {
    /// `steps` steps queued for the room (`queued_ops`), the connection up
    /// or not.
    Waiting { connected: bool, steps: usize },
    /// The drawing is apart from the room for `reason` (`align_message`),
    /// and `entries` of its changes never reached it: the queue and the
    /// unsynced list (`queued_ops` + `unsynced_count`).
    Apart { reason: String, entries: usize },
}

impl SyncLine {
    pub fn text(&self) -> String {
        match self {
            Self::Waiting {
                connected: false,
                steps,
            } => format!("离线，{steps} 步待同步"),
            Self::Waiting {
                connected: true,
                steps,
            } => format!("已连接，{steps} 步待同步"),
            Self::Apart { reason, entries } => format!("不一致：{reason}；未同步 {entries} 条"),
        }
    }
}

/// The title of the question about steps not synced.
pub const UNSYNCED_TITLE: &str = "修改未同步";

/// What quitting, or closing the session's drawing, asks while `steps`
/// steps are still queued (§5.6).
pub fn unsynced_prompt(steps: usize) -> String {
    format!(
        "这张图还有 {steps} 步修改没有同步到服务器。已经随图保存的，下次打开并连上服务器后会接着提交。"
    )
}

/// The button that goes on with a quit.
pub const UNSYNCED_QUIT_LABEL: &str = "退出";

/// The button that goes on with closing the drawing.
pub const UNSYNCED_CLOSE_LABEL: &str = "关闭";

/// The button that stays.
pub const UNSYNCED_CANCEL_LABEL: &str = "取消";

/// Everything the panel draws, taken from the collaboration state so the view
/// function stays free of app types (and therefore testable).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollabView {
    pub connected: bool,
    pub server_url: String,
    pub session_id: Option<String>,
    pub readonly: bool,
    /// Why this drawing is not what the room has, while it is not: parted
    /// from it, or not taken on the connection that is up
    /// (`State::mismatch`). The session sends nothing meanwhile, and the
    /// panel says why and what to do about it.
    pub diverged: Option<String>,
    /// What has not reached the room, while something has not.
    pub sync: Option<SyncLine>,
    pub peers: Vec<PeerRow>,
    /// The conflict entries nobody has ruled on yet, oldest first.
    pub conflicts: Vec<ConflictRow>,
    /// The list opened from 去裁决. `None` until it is opened; `Some` empty
    /// is the list with nothing pending (#81).
    pub conflict_list: Option<Vec<ConflictListItem>>,
    /// The ruling dialog, while one list row is being ruled (#82).
    pub ruling: Option<RulingDialog>,
    pub claims: Vec<ClaimRow>,
    pub last_message: Option<String>,
    /// The claims section is expanded.
    pub claims_open: bool,
    /// What the unconfigured panel's three fields hold (#99). Kept on the
    /// view so a keystroke survives the next rebuild of the rest.
    pub setup_server: String,
    /// The user id typed beside [`Self::setup_server`].
    pub setup_user: String,
    /// The drawing id typed beside [`Self::setup_server`].
    pub setup_drawing: String,
    /// This editing session's notices, newest first (#101). Each line is
    /// the clock time and the same text as the toast.
    pub notices: Vec<String>,
}

impl CollabView {
    /// Whether the panel has anything to say at all (a configured server).
    pub fn configured(&self) -> bool {
        !self.server_url.trim().is_empty()
    }

    /// Claims held by somebody else, which is what the panel marks as
    /// other-owned (the drawing colours every held tag, its own included).
    pub fn remote_tags(&self) -> Vec<String> {
        self.claims
            .iter()
            .filter(|claim| !claim.is_self)
            .map(|claim| claim.tag.clone())
            .collect()
    }
}

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
    }
}

fn section_header<'a>(label: &'a str, count: usize) -> Element<'a, Message> {
    container(
        row![
            text(label).size(12),
            Space::new().width(Fill),
            text(format!("{count}")).size(10).style(muted),
        ]
        .align_y(iced::Center),
    )
    .width(Fill)
    .into()
}

fn status_dot<'a>(connected: bool) -> Element<'a, Message> {
    let color = if connected {
        iced::Color::from_rgb8(51, 199, 89)
    } else {
        iced::Color::from_rgb8(140, 140, 140)
    };
    container(Space::new().width(Length::Fixed(8.0)).height(Length::Fixed(8.0)))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// The colour of what the panel wants noticed: the read-only badge and the
/// edit-right marker. Taken from the design token table (`CLAIM_COLOR_TOKEN`)
/// so the drawing and panel share one theme-aware colour (#56).
fn notice(theme: &Theme) -> iced::widget::text::Style {
    let pid_theme = crate::ui::style::pid_tokens::PidTheme::from_iced(theme);
    iced::widget::text::Style {
        color: Some(crate::ui::style::pid_tokens::CLAIM_COLOR_TOKEN.color(pid_theme)),
    }
}

/// Everybody on the drawing: this session first, marked 本机, then the
/// others in the order the server reported them, with whoever holds the
/// edit right marked 编辑权. With nobody else there, it says so.
fn peer_rows<'a>(view: &'a CollabView) -> Element<'a, Message> {
    let mut rows = column![].spacing(2);
    for peer in &view.peers {
        let name = if peer.display_name.is_empty() || peer.display_name == peer.user_id {
            peer.user_id.clone()
        } else {
            format!("{}（{}）", peer.display_name, peer.user_id)
        };
        let mut line = row![text(name).size(11), Space::new().width(Fill)]
            .spacing(6)
            .align_y(iced::Center);
        if peer.disconnected {
            line = line.push(text("连接中断").size(10).style(notice));
        }
        if peer.is_holder {
            line = line.push(crate::ui::wrap_bar::PosReport::owned(
                holder_bounds_id(&peer.user_id),
                text(HOLDER_LABEL).size(10).style(notice),
            ));
        }
        if peer.is_self {
            line = line.push(text("本机").size(10).style(muted));
        }
        rows = rows.push(container(line).width(Fill).padding([2, 2]));
    }
    if view.peers.iter().all(|peer| peer.is_self) {
        rows = rows.push(text(NO_OTHERS_LABEL).size(11).style(muted));
    }
    rows.into()
}

/// One conflict entry: what is contended on the first line, in the colour
/// of things to notice, and the two values under it.
fn conflict_row<'a>(entry: &'a ConflictRow) -> Element<'a, Message> {
    let lines = column![
        text(entry.subject()).size(11).style(notice),
        text(entry.values()).size(10).style(muted),
        button(text(OPEN_CONFLICT_LIST).size(10))
            .on_press(PidCollabMsg::OpenConflictList.message())
            .style(button::secondary)
            .padding([2, 6]),
    ]
    .spacing(1);
    let reported =
        crate::ui::wrap_bar::PosReport::owned(conflict_bounds_id(&entry.conflict_id), lines);
    container(reported).width(Fill).padding([2, 2]).into()
}

/// The read-only list: one row per query item, or the empty sentence.
fn conflict_list_body<'a>(items: &'a [ConflictListItem]) -> Element<'a, Message> {
    let mut rows = column![].spacing(4);
    rows = rows.push(
        row![
            button(text("待裁决").size(10))
                .on_press(PidCollabMsg::ShowPendingConflicts.message())
                .style(button::text)
                .padding([2, 4]),
            button(text("已裁决").size(10))
                .on_press(PidCollabMsg::ShowRuledConflicts.message())
                .style(button::text)
                .padding([2, 4]),
        ]
        .spacing(6),
    );
    if items.is_empty() {
        rows = rows.push(text(NO_PENDING_CONFLICTS).size(11));
        rows = rows.push(text(NO_PENDING_CONFLICTS_WHY).size(10).style(muted));
    }
    for item in items {
        let mut lines = column![].spacing(1);
        for line in item.lines() {
            lines = lines.push(text(line).size(10));
        }
        if item.status == "已裁决" {
            for line in item.ruling_lines() {
                lines = lines.push(text(line).size(10));
            }
        }
        if item.status != "已裁决" {
            lines = lines.push(
                button(text("裁决").size(10))
                    .on_press(PidCollabMsg::OpenRuling(item.conflict_id.clone()).message())
                    .style(button::secondary)
                    .padding([2, 6]),
            );
        }
        rows = rows.push(crate::ui::wrap_bar::PosReport::owned(
            conflict_list_bounds_id(&item.conflict_id),
            lines,
        ));
    }
    rows = rows.push(
        button(text("返回").size(10))
            .on_press(PidCollabMsg::CloseConflictList.message())
            .style(button::text)
            .padding([2, 0]),
    );
    rows.into()
}

/// 采用甲方 / 采用乙方. The values and the baseline are the query row's.
fn ruling_dialog<'a>(dialog: &'a RulingDialog) -> Element<'a, Message> {
    let item = &dialog.item;
    let mut body = column![
        text(format!("甲方的值 {}", item.value_a)).size(11),
        text(format!("乙方的值 {}", item.value_b)).size(11),
        text(format!("这条的基线修订号 {}", item.base_revision))
            .size(10)
            .style(muted),
        button(text(ADOPT_A).size(10))
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_A.to_owned()).message())
            .style(button::secondary)
            .padding([2, 6]),
        button(text(ADOPT_B).size(10))
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_B.to_owned()).message())
            .style(button::secondary)
            .padding([2, 6]),
        button(text(ADOPT_MANUAL).size(10))
            .on_press(PidCollabMsg::ChooseRuling(ADOPT_MANUAL.to_owned()).message())
            .style(button::secondary)
            .padding([2, 6]),
    ]
    .spacing(2);
    if dialog.choice.as_deref() == Some(ADOPT_MANUAL) {
        body = body.push(
            text_input("最终值", &dialog.manual)
                .on_input(|value| PidCollabMsg::SetRulingManual(value).message()),
        );
    }
    if dialog.choice.is_some() {
        body = body.push(
            button(text(CONFIRM_RULING).size(10))
                .on_press(PidCollabMsg::ConfirmRuling.message())
                .style(button::secondary)
                .padding([2, 6]),
        );
    }
    container(body).width(Fill).padding([4, 2]).into()
}

/// One claim row: the tag, its holder, and the claim/release button.
fn claim_row<'a>(claim: &'a ClaimRow) -> Element<'a, Message> {
    let action = if claim.is_self {
        button(text("释放").size(10))
            .on_press(PidCollabMsg::Release(claim.tag.clone()).message())
            .style(button::secondary)
            .padding([2, 6])
    } else {
        button(text("占用").size(10))
            .on_press(PidCollabMsg::Claim(claim.tag.clone()).message())
            .style(button::secondary)
            .padding([2, 6])
    };
    let holder = if claim.is_self {
        format!("{}（本机）", claim.holder)
    } else {
        claim.holder.clone()
    };
    let row = row![
        text(claim.tag.clone()).size(11),
        Space::new().width(Fill),
        text(holder).size(10).style(muted),
        action,
    ]
    .spacing(4)
    .align_y(iced::Center);
    let reported = crate::ui::wrap_bar::PosReport::owned(row_bounds_id(&claim.tag), row);
    container(reported).width(Fill).padding([2, 2]).into()
}

/// Build the docked panel.
pub fn view<'a>(view: &'a CollabView, width: f32, chrome: Chrome) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(PanelId::PidCollab, "协同".to_string(), chrome);

    if !view.configured() {
        // The three fields are the way in (#99). The block reports where it
        // is under an id of its own, and the panel under its usual one
        // (`VAL-CLI-017`, `VAL-CROSS-012`).
        let mut lines = column![
            text(NOT_CONFIGURED_LABEL).size(12).style(muted),
            text(NOT_CONFIGURED_HINT).size(10).style(muted),
        ]
        .spacing(4)
        .width(Fill);
        if let Some(gap) = setup_gap(&view.setup_server, &view.setup_user, &view.setup_drawing) {
            lines = lines.push(text(gap).size(11).style(muted));
        }
        lines = lines.push(text("服务器").size(11));
        lines = lines.push(
            text_input("ws://主机:端口/ws", &view.setup_server)
                .on_input(|value| PidCollabMsg::SetSetupServer(value).message()),
        );
        lines = lines.push(text("用户").size(11));
        lines = lines.push(
            text_input("用户标识", &view.setup_user)
                .on_input(|value| PidCollabMsg::SetSetupUser(value).message()),
        );
        lines = lines.push(text("图号").size(11));
        lines = lines.push(
            text_input("图纸编号", &view.setup_drawing)
                .on_input(|value| PidCollabMsg::SetSetupDrawing(value).message()),
        );
        lines = lines.push(
            button(text("连接").size(11))
                .on_press(PidCollabMsg::ApplySetup.message())
                .padding([2, 8]),
        );
        let hint = container(crate::ui::wrap_bar::PosReport::new(
            NOT_CONFIGURED_BOUNDS_ID,
            lines,
        ))
        .width(Fill)
        .padding(8);
        let reported = crate::ui::wrap_bar::PosReport::new(PANEL_BOUNDS_ID, hint);
        return crate::ui::dock::frame(title_bar, reported, width);
    }

    // ── Session banner ───────────────────────────────────────────────────
    let status = if view.connected {
        "已连接"
    } else {
        "未连接"
    };
    let mut banner_items = row![
        status_dot(view.connected),
        text(status).size(11),
        Space::new().width(Fill),
    ]
    .spacing(5)
    .align_y(iced::Center);
    if view.diverged.is_some() {
        banner_items = banner_items.push(crate::ui::wrap_bar::PosReport::new(
            DIVERGED_BOUNDS_ID,
            text(DIVERGED_LABEL).size(11).style(notice),
        ));
    }
    if view.readonly {
        banner_items = banner_items.push(crate::ui::wrap_bar::PosReport::new(
            READONLY_BOUNDS_ID,
            text(READONLY_LABEL).size(11).style(notice),
        ));
    }
    let banner = container(banner_items).width(Fill).padding([2, 2]);

    let mut body = column![banner].spacing(6);
    if !view.notices.is_empty() {
        body = body.push(text("本次消息").size(12));
        for line in &view.notices {
            body = body.push(text(line.clone()).size(10).style(muted));
        }
    }
    // What has not reached the room, right under the banner (§5.6).
    if let Some(line) = &view.sync {
        let style: fn(&Theme) -> iced::widget::text::Style = match line {
            SyncLine::Waiting { .. } => muted,
            SyncLine::Apart { .. } => notice,
        };
        body = body.push(
            container(crate::ui::wrap_bar::PosReport::new(
                SYNC_BOUNDS_ID,
                text(line.text()).size(10).style(style),
            ))
            .width(Fill)
            .padding([0, 2]),
        );
    }
    if let Some(reason) = &view.diverged {
        // Under the banner rather than in the last-outcome line, which the
        // next claim would overwrite: this stays true until the drawing is
        // replaced, or a connection takes it as the room's. A line above
        // that says the drawing is apart has given the reason already.
        let reason = match &view.sync {
            Some(SyncLine::Apart { .. }) => "",
            _ => reason.as_str(),
        };
        body = body.push(
            container(
                column![
                    text(format!(
                        "本图与服务器上的图纸已不一致，本地修改不再提交。{reason}"
                    ))
                    .size(10)
                    .style(notice),
                    text(GET_LATEST_HINT).size(10).style(notice),
                ]
                .spacing(2),
            )
            .width(Fill)
            .padding([0, 2]),
        );
    }

    // ── Peers ────────────────────────────────────────────────────────────
    body = body.push(section_header("在线用户", view.peers.len()));
    body = body.push(peer_rows(view));

    // ── Conflicts awaiting a ruling ──────────────────────────────────────
    // Above the claims, and not behind a toggle: an entry here means a
    // write — possibly this person's — has not taken effect, and stays so
    // until somebody rules on it.
    if !view.conflicts.is_empty() {
        body = body.push(section_header(CONFLICTS_LABEL, view.conflicts.len()));
        let mut rows = column![].spacing(2);
        for entry in &view.conflicts {
            rows = rows.push(conflict_row(entry));
        }
        body = body.push(rows);
    }
    if let Some(items) = &view.conflict_list {
        body = body.push(section_header("冲突列表", items.len()));
        body = body.push(conflict_list_body(items));
    }
    if let Some(dialog) = &view.ruling {
        body = body.push(ruling_dialog(dialog));
    }

    // ── Claims ───────────────────────────────────────────────────────────
    let toggle_label = if view.claims_open {
        "位号占用 ▾"
    } else {
        "位号占用 ▸"
    };
    body = body.push(
        button(text(toggle_label).size(12))
            .on_press(PidCollabMsg::ToggleClaims.message())
            .style(button::text)
            .padding([2, 0])
            .width(Fill),
    );
    if view.claims_open {
        if view.claims.is_empty() {
            body = body.push(text("暂无占用").size(11).style(muted));
        } else {
            let mut claims = column![].spacing(1);
            for claim in &view.claims {
                claims = claims.push(claim_row(claim));
            }
            body = body.push(claims);
        }
    }

    // ── Last outcome ─────────────────────────────────────────────────────
    if let Some(message) = &view.last_message {
        body = body.push(
            container(text(message.clone()).size(10).style(muted))
                .width(Fill)
                .padding([2, 0]),
        );
    }

    let body = scrollable(body)
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::default(),
        ))
        .width(Fill)
        .height(Fill);

    let reported = crate::ui::wrap_bar::PosReport::new(PANEL_BOUNDS_ID, body);
    crate::ui::dock::frame(title_bar, reported, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_with(claims: Vec<ClaimRow>) -> CollabView {
        CollabView {
            connected: true,
            server_url: "ws://127.0.0.1:3240/ws".to_owned(),
            session_id: Some("s-1".to_owned()),
            claims,
            claims_open: true,
            ..CollabView::default()
        }
    }

    /// A tag this session does not hold is what the drawing colours.
    #[test]
    fn remote_tags_are_the_ones_this_session_does_not_hold() {
        let view = view_with(vec![
            ClaimRow {
                tag: "P-1001".to_owned(),
                holder: "u2".to_owned(),
                is_self: false,
            },
            ClaimRow {
                tag: "P-1002".to_owned(),
                holder: "u1".to_owned(),
                is_self: true,
            },
        ]);
        assert_eq!(view.remote_tags(), vec!["P-1001".to_string()]);
    }

    #[test]
    fn a_panel_without_a_server_url_is_not_configured() {
        let mut view = view_with(Vec::new());
        assert!(view.configured());
        view.server_url.clear();
        assert!(!view.configured());
    }

    /// Row ids carry the tag, so a client can find the row it means.
    #[test]
    fn row_bounds_ids_name_their_tag() {
        assert_eq!(row_bounds_id("P-1001"), "pid-collab-row-P-1001");
        assert!(row_bounds_id("P-1001").starts_with(ROW_BOUNDS_PREFIX));
    }

    /// The marker id carries the user, so a client can read off who the panel
    /// shows as holding the edit right.
    #[test]
    fn holder_bounds_ids_name_their_user() {
        assert_eq!(holder_bounds_id("u1"), "pid-collab-holder-u1");
        assert!(holder_bounds_id("u1").starts_with(HOLDER_BOUNDS_PREFIX));
    }

    // ── The panel as it is drawn ─────────────────────────────────────────

    /// The panel's docked width, and a height that fits what the tests show.
    const WIDTH: f32 = 248.0;
    const HEIGHT: f32 = 420.0;

    type Ui<'a> = iced_test::simulator::Simulator<'a, Message>;

    fn draw(view_model: &CollabView) -> Ui<'_> {
        iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (WIDTH, HEIGHT),
            view(view_model, WIDTH, Chrome::default()),
        )
    }

    /// Where the text `label` is drawn, or `None` when it is not.
    fn drawn(ui: &mut Ui<'_>, label: &str) -> Option<iced::Rectangle> {
        ui.find(label).ok().and_then(|found| found.visible_bounds())
    }

    fn peer(user: &str, is_self: bool, is_holder: bool) -> PeerRow {
        PeerRow {
            user_id: user.to_owned(),
            display_name: user.to_owned(),
            is_self,
            is_holder,
            disconnected: false,
        }
    }

    /// `u2`'s panel on a drawing `u1` holds the edit right of.
    fn downgraded() -> CollabView {
        CollabView {
            readonly: true,
            peers: vec![peer("u1", false, true), peer("u2", true, false)],
            ..view_with(Vec::new())
        }
    }

    /// VAL-CLI-012: a downgraded session's panel draws the read-only badge,
    /// and draws the edit-right marker on the holder's row and no other.
    #[test]
    fn a_downgraded_session_draws_the_badge_and_marks_the_holders_row() {
        let view_model = downgraded();
        let mut ui = draw(&view_model);

        let badge = drawn(&mut ui, READONLY_LABEL).expect("the read-only badge is drawn");
        let marker = drawn(&mut ui, HOLDER_LABEL).expect("the edit-right marker is drawn");
        let holder = drawn(&mut ui, "u1").expect("the holder's row is drawn");
        let own = drawn(&mut ui, "u2").expect("this session's row is drawn");

        let same_line = |a: iced::Rectangle, b: iced::Rectangle| {
            (a.center_y() - b.center_y()).abs() < a.height.max(b.height) / 2.0
        };
        assert!(
            same_line(marker, holder),
            "the marker sits on the holder's row: marker {marker:?}, u1 {holder:?}"
        );
        assert!(
            !same_line(marker, own),
            "and not on this session's: marker {marker:?}, u2 {own:?}"
        );
        assert!(
            badge.y < holder.y,
            "the badge is in the banner, above the roster: badge {badge:?}, u1 {holder:?}"
        );
    }

    /// A session that may write draws neither: the badge is the state, so it
    /// must not be there when the state is not.
    #[test]
    fn a_session_that_may_write_draws_neither() {
        let view_model = CollabView {
            peers: vec![peer("u1", true, false), peer("u2", false, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        assert!(drawn(&mut ui, "u1").is_some(), "the roster itself is drawn");
        assert_eq!(drawn(&mut ui, READONLY_LABEL), None);
        assert_eq!(drawn(&mut ui, HOLDER_LABEL), None);
        assert_eq!(drawn(&mut ui, DIVERGED_LABEL), None);
    }

    /// The first one on a drawing sees itself, marked 本机, and is told
    /// nobody else is there; with a second, the note goes and both rows are
    /// drawn (`VAL-CROSS-006`: 「0 名其他用户」→「1 名其他用户、共 2 人」).
    #[test]
    fn the_roster_shows_this_session_and_says_when_nobody_else_is_there() {
        let alone = CollabView {
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&alone);
        assert!(drawn(&mut ui, "u1").is_some());
        assert!(drawn(&mut ui, "本机").is_some());
        assert!(drawn(&mut ui, NO_OTHERS_LABEL).is_some());

        let two = CollabView {
            peers: vec![peer("u1", true, false), peer("u2", false, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&two);
        let own = drawn(&mut ui, "u1").expect("this session's row");
        let other = drawn(&mut ui, "u2").expect("the other's row");
        assert!(own.y < other.y, "this session first: {own:?}, {other:?}");
        assert_eq!(drawn(&mut ui, NO_OTHERS_LABEL), None);
    }

    /// `VAL-CLI-017`, `VAL-CROSS-012`, #99: with no server set the panel says
    /// so, names the three fields, and says which of them is empty. It does
    /// not draw the session banner or the roster.
    #[test]
    fn a_panel_with_no_server_says_so_and_how_to_set_one() {
        let view_model = CollabView::default();
        let mut ui = draw(&view_model);
        let said = drawn(&mut ui, NOT_CONFIGURED_LABEL).expect("the note is drawn");
        let how = drawn(&mut ui, NOT_CONFIGURED_HINT).expect("the hint is drawn");
        let gap = drawn(&mut ui, "还缺：服务器地址、用户、图号").expect("what is missing");
        let server = drawn(&mut ui, "ws://主机:端口/ws").expect("the server field");
        let user = drawn(&mut ui, "用户标识").expect("the user field");
        let drawing = drawn(&mut ui, "图纸编号").expect("the drawing field");
        let connect = drawn(&mut ui, "连接").expect("the connect button");
        assert!(said.y < how.y && how.y < gap.y, "{said:?} {how:?} {gap:?}");
        assert!(
            gap.y < server.y && server.y < user.y && user.y < drawing.y && drawing.y < connect.y,
            "{gap:?} {server:?} {user:?} {drawing:?} {connect:?}"
        );
        assert_eq!(drawn(&mut ui, "未连接"), None);
        assert_eq!(drawn(&mut ui, "在线用户"), None);

        let partial = CollabView {
            setup_server: "ws://127.0.0.1:3240/ws".to_owned(),
            ..CollabView::default()
        };
        let mut ui = draw(&partial);
        assert!(drawn(&mut ui, "还缺：用户、图号").is_some());
        assert_eq!(drawn(&mut ui, "服务器地址"), None);

        let ready = CollabView {
            setup_server: "ws://127.0.0.1:3240/ws".to_owned(),
            setup_user: "u1".to_owned(),
            setup_drawing: "PID-1101".to_owned(),
            ..CollabView::default()
        };
        let mut ui = draw(&ready);
        assert_eq!(drawn(&mut ui, "还缺"), None);
        assert!(drawn(&mut ui, "连接").is_some());
    }

    /// `VAL-CROSS-007`: an entry awaiting a ruling is listed under a header
    /// of its own, below the roster, saying what is contended and the two
    /// values; with no entry there is no such section.
    #[test]
    fn a_conflict_awaiting_a_ruling_is_listed_with_what_is_contended() {
        let entry = ConflictRow {
            conflict_id: "c-7f3a".to_owned(),
            object_ref: "P-1001".to_owned(),
            field: "设计压力".to_owned(),
            value_a: "1.6MPa".to_owned(),
            value_b: "2.5MPa".to_owned(),
        };
        assert_eq!(conflict_bounds_id("c-7f3a"), "pid-collab-conflict-c-7f3a");
        let view_model = CollabView {
            peers: vec![peer("u1", true, false)],
            conflicts: vec![entry.clone()],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        let header = drawn(&mut ui, CONFLICTS_LABEL).expect("the section header is drawn");
        let subject = drawn(&mut ui, "P-1001 · 设计压力").expect("what is contended is drawn");
        let values = drawn(&mut ui, "1.6MPa ↔ 2.5MPa").expect("the two values are drawn");
        let roster = drawn(&mut ui, "u1").expect("the roster is drawn");
        assert!(roster.y < header.y, "below the roster: u1 {roster:?}, header {header:?}");
        assert!(header.y < subject.y && subject.y < values.y, "{header:?} {subject:?} {values:?}");

        let none = CollabView {
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&none);
        assert_eq!(drawn(&mut ui, CONFLICTS_LABEL), None);
        let mut with_row = draw(&view_model);
        assert!(drawn(&mut with_row, OPEN_CONFLICT_LIST).is_some());
    }

    /// #81: the list opened from 去裁决 shows each query field, in order,
    /// and a second entry sits below the first. An empty query says so.
    #[test]
    fn conflict_list_rows_are_drawn_in_order() {
        let first = ConflictListItem {
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
        let view_model = CollabView {
            peers: vec![peer("u1", true, false)],
            conflict_list: Some(vec![first.clone(), second.clone()]),
            ..view_with(Vec::new())
        };
        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            (WIDTH, 720.0),
            view(&view_model, WIDTH, Chrome::default()),
        );
        let mut previous = 0.0;
        for line in first.lines() {
            let bounds = drawn(&mut ui, &line).unwrap_or_else(|| panic!("missing {line}"));
            assert!(
                bounds.y >= previous,
                "{line} at {bounds:?} is above the previous line"
            );
            previous = bounds.y;
        }
        let first_row = drawn(&mut ui, "c-7f3a").expect("first row");
        let second_row = drawn(&mut ui, "c-8a01").expect("second row");
        assert!(first_row.y < second_row.y, "{first_row:?} {second_row:?}");

        let empty = CollabView {
            peers: vec![peer("u1", true, false)],
            conflict_list: Some(Vec::new()),
            ..view_with(Vec::new())
        };
        let mut ui = draw(&empty);
        let title = drawn(&mut ui, NO_PENDING_CONFLICTS).expect("the empty list says so");
        let why = drawn(&mut ui, NO_PENDING_CONFLICTS_WHY).expect("and when one appears");
        assert!(title.y < why.y, "{title:?} {why:?}");
    }

    /// A session whose drawing is short of the room's changes says so in the
    /// banner, above the roster, for as long as it is: nothing it does goes
    /// out any more, and the person has to know why.
    #[test]
    fn a_session_that_parted_from_the_room_draws_the_badge() {
        let view_model = CollabView {
            diverged: Some("收到修订 5，本地基线是 3".to_owned()),
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        let badge = drawn(&mut ui, DIVERGED_LABEL).expect("the badge is drawn");
        let roster = drawn(&mut ui, "u1").expect("the roster is drawn");
        assert!(badge.y < roster.y, "badge {badge:?}, u1 {roster:?}");
    }

    /// `REQ-COLL-015` acceptance 3: a drawing that is not the room's —
    /// parted from it, or not taken on this connection — says what to do
    /// about it, under the badge and above the roster.
    #[test]
    fn collab_align_refused_the_panel_says_to_get_the_latest_drawing() {
        let view_model = CollabView {
            diverged: Some("协同库标识不一致：这份文件对齐的是另一个服务器库".to_owned()),
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        let badge = drawn(&mut ui, DIVERGED_LABEL).expect("the badge is drawn");
        let hint = drawn(&mut ui, "需要取得最新图纸").expect("what to do is drawn");
        let roster = drawn(&mut ui, "u1").expect("the roster is drawn");
        assert!(
            badge.y < hint.y && hint.y < roster.y,
            "badge {badge:?}, hint {hint:?}, u1 {roster:?}"
        );
    }

    /// The line under the banner (§5.6), in the words the spec gives, and
    /// the prompt about steps not synced.
    #[test]
    fn the_sync_line_and_the_prompt_read_as_the_spec_has_them() {
        let offline = SyncLine::Waiting {
            connected: false,
            steps: 2,
        };
        assert_eq!(offline.text(), "离线，2 步待同步");
        let connected = SyncLine::Waiting {
            connected: true,
            steps: 3,
        };
        assert_eq!(connected.text(), "已连接，3 步待同步");
        let apart = SyncLine::Apart {
            reason: "收到修订 5，本地基线是 3".to_owned(),
            entries: 4,
        };
        assert_eq!(
            apart.text(),
            "不一致：收到修订 5，本地基线是 3；未同步 4 条"
        );
        assert_eq!(
            unsynced_prompt(3),
            "这张图还有 3 步修改没有同步到服务器。已经随图保存的，下次打开并连上服务器后会接着提交。"
        );
    }

    /// The line is drawn under the banner and above the roster while there
    /// is something to say, and not at all once there is not.
    #[test]
    fn the_sync_line_is_drawn_under_the_banner_while_steps_wait() {
        let line = SyncLine::Waiting {
            connected: true,
            steps: 3,
        };
        let waiting = CollabView {
            sync: Some(line.clone()),
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&waiting);
        let banner = drawn(&mut ui, "已连接").expect("the banner is drawn");
        let said = drawn(&mut ui, &line.text()).expect("the line is drawn");
        let roster = drawn(&mut ui, "u1").expect("the roster is drawn");
        assert!(
            banner.y < said.y && said.y < roster.y,
            "banner {banner:?}, line {said:?}, u1 {roster:?}"
        );

        let synced = CollabView {
            sync: None,
            ..waiting.clone()
        };
        let mut ui = draw(&synced);
        assert_eq!(drawn(&mut ui, &line.text()), None);
    }

    /// #101: several notices of this session, newest first, each with its time
    /// and the same text the toast used.
    #[test]
    fn the_session_message_list_is_newest_first() {
        let view_model = CollabView {
            notices: vec![
                "00:00:03 第三条".to_owned(),
                "00:00:02 第二条".to_owned(),
                "00:00:01 第一条".to_owned(),
            ],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        let newest = drawn(&mut ui, "00:00:03 第三条").expect("the newest notice");
        let middle = drawn(&mut ui, "00:00:02 第二条").expect("the middle notice");
        let oldest = drawn(&mut ui, "00:00:01 第一条").expect("the oldest notice");
        assert!(
            newest.y < middle.y && middle.y < oldest.y,
            "{newest:?} {middle:?} {oldest:?}"
        );
        assert_eq!(view_model.notices.len(), 3);
        assert_eq!(drawn(&mut ui, "00:00:04"), None);
    }

    /// A drawing apart from the room: the line gives the reason, and the
    /// note under it says what that means without giving it a second time.
    #[test]
    fn apart_the_line_gives_the_reason_and_the_note_does_not_repeat_it() {
        let reason = "收到修订 5，本地基线是 3";
        let line = SyncLine::Apart {
            reason: reason.to_owned(),
            entries: 2,
        };
        let view_model = CollabView {
            diverged: Some(reason.to_owned()),
            sync: Some(line.clone()),
            peers: vec![peer("u1", true, false)],
            ..view_with(Vec::new())
        };
        let mut ui = draw(&view_model);
        let said = drawn(&mut ui, &line.text()).expect("the line is drawn");
        let note = drawn(&mut ui, "本图与服务器上的图纸已不一致，本地修改不再提交。")
            .expect("the note is drawn, without the reason");
        let hint = drawn(&mut ui, GET_LATEST_HINT).expect("what to do is drawn");
        assert!(
            said.y < note.y && note.y < hint.y,
            "{said:?} {note:?} {hint:?}"
        );
    }
}

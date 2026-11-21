//! 备份数据包 — screen `backup`; in the design, frames `BkF01`..`BkF11`
//! (and their light counterparts `BkL01`..`BkL11`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::{button, column, container, row, scrollable, stack, text, text_input, Space};
use iced::{alignment, font, Background, Border, Color, Element, Fill, Length, Padding, Task};

use super::{Message as ShellMessage, PidScreen, ViewFn};
use crate::app::{Message as AppMessage, OpenCADStudio};
use crate::io::pid_backup_browse::{BackupIndex, DrawingEntry, DrawingItems, FileKind, GroupNode};
use crate::ui::style::pid_fonts;
use crate::ui::style::pid_tokens::{ColorToken, NumberToken, PidTheme};
use crate::ui::window::pid_backup_browser::{self, CacheMsg, ItemsState, LoadState, PidBackupMsg};
use crate::ui::wrap_bar::PosReport;

/// Prefix of every bounds rectangle reported inside the `backup` screen.
pub const SCREEN_BOUNDS_PREFIX: &str = "PID_BACKUP_SCREEN:";

const MAX_RENDERED_ITEMS: usize = 200;

fn part_id(name: &str) -> String {
    format!("{SCREEN_BOUNDS_PREFIX}{name}")
}

/// Trim surrounding whitespace and Windows "Copy as path" double quotes.
pub fn clean_path_input(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .trim()
}

/// Injectable source picker for headless tests (`true` = folder, `false` = zip).
/// Returning `Some(Some(path))` loads `path`; `Some(None)` cancels; `None`
/// falls back to the native file dialog.
pub type SourcePicker = Arc<dyn Fn(bool) -> Option<Option<PathBuf>> + Send + Sync>;

/// Tone of an inline notice shown in the `backup` screen header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Warning,
}

/// Which detail section is shown in the right-hand inspector pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DetailTab {
    /// Show both database columns and drawing items stacked (default).
    #[default]
    All,
    /// Only database / drawing attributes.
    Properties,
    /// Only drawing model items.
    Items,
}

/// What the screen remembers between frames.
#[derive(Default)]
pub struct State {
    /// Selected plant-group path (`None` = all drawings).
    pub selected_group: Option<String>,
    /// Case-insensitive filter on drawing names.
    pub drawing_filter: String,
    /// Selected index into [`BackupIndex::drawings`].
    pub selected_drawing: Option<usize>,
    /// Active tab in the right-hand detail pane.
    pub detail_tab: DetailTab,
    /// Optional exact `ModelItemType` filter on the selected drawing's items.
    pub item_type_filter: Option<String>,
    /// Case-insensitive filter on item label / UID / class / attribute values.
    pub item_query: String,
    /// Item UIDs expanded to inspect published `interface.name = value` rows.
    pub expanded_items: HashSet<String>,
    /// Path of a drawing currently being opened from this screen.
    pub opening_drawing: Option<PathBuf>,
    /// Error text by path for drawings that failed to open from this screen.
    pub open_failures: HashMap<PathBuf, String>,
    /// Last `(drawing_idx, instant)` clicked in the list, for double-click open.
    pub last_clicked_drawing: Option<(usize, iced::time::Instant)>,
    /// Inline notice from the last secondary action (e.g. cache clear).
    pub notice: Option<(NoticeTone, String)>,
    /// True while a UI-initiated `ClearCache` awaits its outcome.
    pub pending_ui_cache_clear: bool,
    /// Optional test double replacing the native source picker dialog.
    pub source_picker: Option<SourcePicker>,
    /// Inline backup source path input for clipboard/direct opening.
    pub source_input: String,
    /// Export configuration dialogs and outcome drawers (`UI-7`, `UI-8`).
    pub export_ui: super::backup_export_ui::ExportUiState,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("selected_group", &self.selected_group)
            .field("drawing_filter", &self.drawing_filter)
            .field("selected_drawing", &self.selected_drawing)
            .field("detail_tab", &self.detail_tab)
            .field("item_type_filter", &self.item_type_filter)
            .field("item_query", &self.item_query)
            .field("notice", &self.notice)
            .field("has_source_picker", &self.source_picker.is_some())
            .field("export_ui", &self.export_ui)
            .finish()
    }
}

fn same_drawing_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return ca == cb;
    }
    false
}

impl State {
    /// Reset browse selection when a fresh backup index finishes loading.
    pub fn reset_for_loaded_package(&mut self, drawing_count: usize) {
        self.selected_group = None;
        self.drawing_filter.clear();
        self.selected_drawing = (drawing_count > 0).then_some(0);
        self.detail_tab = DetailTab::All;
        self.item_type_filter = None;
        self.item_query.clear();
        self.expanded_items.clear();
        self.opening_drawing = None;
        self.open_failures.clear();
        self.last_clicked_drawing = None;
        self.notice = None;
        self.pending_ui_cache_clear = false;
        self.export_ui.reset_for_loaded_package();
    }

    /// Whether `path` is currently being opened from the `backup` screen.
    pub fn is_opening_path(&self, path: &Path) -> bool {
        self.opening_drawing
            .as_deref()
            .is_some_and(|cur| same_drawing_path(cur, path))
    }

    /// Clear `opening_drawing` if it matches `path`, returning `true` when it did.
    pub fn complete_opening_path(&mut self, path: &Path) -> bool {
        if let Some(cur) = self.opening_drawing.clone() {
            if same_drawing_path(&cur, path) {
                self.opening_drawing = None;
                self.open_failures.remove(&cur);
                self.open_failures.remove(path);
                return true;
            }
        }
        false
    }

    /// Record an inline open failure for `path` without raising a global modal (S4).
    pub fn record_open_failure(&mut self, path: PathBuf, reason: String) {
        if self.is_opening_path(&path) {
            self.opening_drawing = None;
        }
        self.open_failures.insert(path, reason);
    }
}

/// What the screen's widgets send.
#[derive(Debug, Clone)]
pub enum Message {
    /// Pick and open a `.zip` (`false`) or folder (`true`) backup source.
    PickSource { folder: bool },
    /// Update the inline `.zip` / folder path input.
    SourceInputChanged(String),
    /// Open the `.zip` or folder typed into `source_input`.
    OpenTypedSource,
    /// Clear restored backup caches except the currently open one (S3).
    ClearCache,
    /// Switch to the editor and reveal the classic docked backup panel (S3).
    OpenClassicPanel,
    /// Filter drawings by plant-group subtree (`None` = all drawings).
    SelectGroup(Option<String>),
    /// Update the drawing-name search query.
    DrawingFilterChanged(String),
    /// Select a drawing by its index in `BackupIndex::drawings`.
    SelectDrawing(usize),
    /// Open the currently selected drawing in the P&ID editor (`UI-6`).
    OpenSelectedDrawing,
    /// Switch the right-hand detail view mode.
    SelectDetailTab(DetailTab),
    /// Filter the selected drawing's items by `ModelItemType`.
    SelectItemType(Option<String>),
    /// Update the item search query.
    ItemQueryChanged(String),
    /// Expand or collapse one item's published attributes by UID.
    ToggleItemExpanded(String),
    /// Export dialog and outcome actions (`UI-7`, `UI-8`).
    Export(super::backup_export_ui::ExportUiMessage),
}

/// The function that draws the screen.
pub(crate) const VIEW: Option<ViewFn> = Some(view);

/// Whether `index` holds a SQL Server MTF database dump.
pub fn is_sql_server_package(index: &BackupIndex) -> bool {
    index.files.iter().any(|file| {
        file.kind == FileKind::ExportDump && file.format.to_ascii_lowercase().contains("sql server")
    })
}

/// Format a `group_path` (`A / Unregistered`) with localized special group names (S2).
pub fn display_group_path(group_path: &str) -> String {
    if group_path.trim().is_empty() {
        return pid_backup_browser::screen_group_label(crate::io::pid_backup_browse::UNGROUPED)
            .to_owned();
    }
    group_path
        .split(" / ")
        .map(pid_backup_browser::screen_group_label)
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Summary title line of an open backup package.
pub fn header_title_line(index: &BackupIndex) -> String {
    if let Some(manifest) = &index.manifest {
        let plant = manifest
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("未命名工厂");
        let mut tags = Vec::new();
        if let Some(version) = manifest.version.as_deref().filter(|v| !v.trim().is_empty()) {
            tags.push(format!("版本 {version}"));
        }
        if let Some(kind) = manifest
            .backup_type
            .as_deref()
            .filter(|k| !k.trim().is_empty())
        {
            tags.push(format!("类型 {kind}"));
        }
        if let Some(date) = manifest
            .date_created
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            tags.push(date.to_owned());
        }
        if is_sql_server_package(index) {
            tags.push("SQL Server 备份".to_owned());
        }
        if tags.is_empty() {
            format!("备份数据包 · {plant}")
        } else {
            format!("备份数据包 · {plant}（{}）", tags.join(" · "))
        }
    } else {
        let folder = index
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| index.source.display().to_string());
        format!("备份数据包 · {folder}（散放图纸目录，无备份清单）")
    }
}

/// Summary metadata line of an open backup package.
pub fn header_meta_line(index: &BackupIndex) -> String {
    let mode = if index.read_only {
        "直接使用目录（只读）"
    } else {
        "已还原到缓存目录"
    };
    format!(
        "{} · {} · {} 张图纸 · {} 个包内文件 · {} 个参考数据文件 · {} 条警告",
        index.source.display(),
        mode,
        index.drawings.len(),
        index.files.len(),
        index.ref_tree.file_count(),
        index.warnings.len(),
    )
}

/// Kick off background `DrawingItems` loading for `drawing_idx` if needed.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::app) fn ensure_drawing_items_loaded(
    app: &mut OpenCADStudio,
    drawing_idx: usize,
) -> Task<AppMessage> {
    if app.pid_backup_browser.items.contains_key(&drawing_idx) {
        return Task::none();
    }
    let Some(path) = app
        .pid_backup_browser
        .index()
        .and_then(|idx| idx.drawings.get(drawing_idx))
        .and_then(|entry| entry.pid_path.clone())
    else {
        return Task::none();
    };
    app.pid_backup_browser
        .items
        .insert(drawing_idx, ItemsState::Loading);
    Task::perform(
        crate::io::pid_backup_browse::load_items_in_background(path),
        move |result| {
            AppMessage::PidBackup(PidBackupMsg::ItemsLoaded(drawing_idx, result.map(Arc::new)))
        },
    )
}

/// Ensure the currently selected (or first filtered) drawing has started loading items.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::app) fn ensure_selected_drawing_items_loaded(
    app: &mut OpenCADStudio,
) -> Task<AppMessage> {
    let Some(index) = app.pid_backup_browser.index().cloned() else {
        return Task::none();
    };
    let filtered = pid_backup_browser::filter_drawings(
        &index,
        app.pid_shell.backup.selected_group.as_deref(),
        &app.pid_shell.backup.drawing_filter,
    );
    let chosen = app
        .pid_shell
        .backup
        .selected_drawing
        .filter(|i| filtered.contains(i))
        .or_else(|| filtered.first().copied());
    if app.pid_shell.backup.selected_drawing != chosen {
        app.pid_shell.backup.selected_drawing = chosen;
        app.pid_shell.backup.item_type_filter = None;
        app.pid_shell.backup.expanded_items.clear();
    }
    match chosen {
        Some(idx) => ensure_drawing_items_loaded(app, idx),
        None => Task::none(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_drawing_at(app: &mut OpenCADStudio, drawing_idx: usize) -> Task<AppMessage> {
    let Some(path) = app
        .pid_backup_browser
        .index()
        .and_then(|idx| idx.drawings.get(drawing_idx))
        .and_then(|entry| entry.pid_path.clone())
    else {
        return Task::none();
    };
    if let Some(tab_idx) = app.tab_showing(&path) {
        app.pid_shell.backup.opening_drawing = None;
        app.pid_shell.backup.open_failures.remove(&path);
        let switch = app.update(AppMessage::TabSwitch(tab_idx));
        let _ = app.show_pid_screen(PidScreen::Editor);
        return switch;
    }
    app.pid_shell.backup.opening_drawing = Some(path.clone());
    app.pid_shell.backup.open_failures.remove(&path);
    app.update(AppMessage::OpenRecent(path))
}

/// Take one of the screen's messages.
pub(crate) fn update(app: &mut OpenCADStudio, message: Message) -> Task<AppMessage> {
    match message {
        Message::PickSource { folder } => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if app.pid_backup_export.is_running() || app.pid_backup_pdf.is_running() {
                    app.pid_shell.backup.notice = Some((
                        NoticeTone::Warning,
                        "正在后台导出备份数据包，导出完成前暂不切换备份包。".to_owned(),
                    ));
                    return Task::none();
                }
                app.pid_shell.backup.notice = None;
                if let Some(picker) = app.pid_shell.backup.source_picker.clone() {
                    match picker(folder) {
                        Some(Some(path)) => return app.load_pid_backup(path, None),
                        Some(None) => return Task::none(),
                        None => {}
                    }
                }
                app.pick_backup_source(folder)
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = folder;
                Task::none()
            }
        }
        Message::SourceInputChanged(val) => {
            app.pid_shell.backup.source_input = val;
            Task::none()
        }
        Message::OpenTypedSource => {
            let cleaned = clean_path_input(&app.pid_shell.backup.source_input).to_owned();
            if cleaned.is_empty() {
                return Task::none();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if app.pid_backup_export.is_running() || app.pid_backup_pdf.is_running() {
                    app.pid_shell.backup.notice = Some((
                        NoticeTone::Warning,
                        "正在后台导出备份数据包，导出完成前暂不切换备份包。".to_owned(),
                    ));
                    return Task::none();
                }
                app.pid_shell.backup.notice = None;
                app.load_pid_backup(PathBuf::from(cleaned), None)
            }
            #[cfg(target_arch = "wasm32")]
            {
                Task::none()
            }
        }
        Message::ClearCache => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let busy = app.pid_backup_export.is_running() || app.pid_backup_pdf.is_running();
                if busy {
                    app.pid_shell.backup.pending_ui_cache_clear = false;
                    let task = app.on_pid_backup(PidBackupMsg::Cache(CacheMsg::Clear));
                    app.pid_shell.backup.notice = Some((
                        NoticeTone::Warning,
                        "正在后台导出备份数据包，导出完成前暂不清理还原缓存。".to_owned(),
                    ));
                    task
                } else {
                    app.pid_shell.backup.pending_ui_cache_clear = true;
                    app.pid_shell.backup.notice = Some((
                        NoticeTone::Info,
                        "正在清理非当前打开备份包的还原缓存…".to_owned(),
                    ));
                    app.on_pid_backup(PidBackupMsg::Cache(CacheMsg::Clear))
                }
            }
            #[cfg(target_arch = "wasm32")]
            Task::none()
        }
        Message::OpenClassicPanel => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                if app.tabs.get(app.active_tab).is_some_and(|tab| tab.is_start) {
                    let _ = app.update(AppMessage::TabNew);
                }
                app.show_pid_backup_panel();
                let _ = app.show_pid_screen(PidScreen::Editor);
            }
            Task::none()
        }
        Message::SelectGroup(group) => {
            app.pid_shell.backup.selected_group = group;
            #[cfg(not(target_arch = "wasm32"))]
            {
                ensure_selected_drawing_items_loaded(app)
            }
            #[cfg(target_arch = "wasm32")]
            Task::none()
        }
        Message::DrawingFilterChanged(query) => {
            app.pid_shell.backup.drawing_filter = query;
            #[cfg(not(target_arch = "wasm32"))]
            {
                ensure_selected_drawing_items_loaded(app)
            }
            #[cfg(target_arch = "wasm32")]
            Task::none()
        }
        Message::SelectDrawing(idx) => {
            let now = iced::time::Instant::now();
            let double_clicked =
                app.pid_shell
                    .backup
                    .last_clicked_drawing
                    .is_some_and(|(prev, at)| {
                        prev == idx
                            && now.saturating_duration_since(at)
                                <= std::time::Duration::from_millis(500)
                    });
            app.pid_shell.backup.last_clicked_drawing = Some((idx, now));
            if app.pid_shell.backup.selected_drawing != Some(idx) {
                app.pid_shell.backup.selected_drawing = Some(idx);
                app.pid_shell.backup.item_type_filter = None;
                app.pid_shell.backup.expanded_items.clear();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let items_task = ensure_drawing_items_loaded(app, idx);
                if double_clicked {
                    Task::batch([items_task, open_drawing_at(app, idx)])
                } else {
                    items_task
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = double_clicked;
                Task::none()
            }
        }
        Message::OpenSelectedDrawing => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let Some(index) = app.pid_backup_browser.index().cloned() else {
                    return Task::none();
                };
                let filtered = pid_backup_browser::filter_drawings(
                    &index,
                    app.pid_shell.backup.selected_group.as_deref(),
                    &app.pid_shell.backup.drawing_filter,
                );
                let Some(idx) = app
                    .pid_shell
                    .backup
                    .selected_drawing
                    .filter(|i| filtered.contains(i))
                    .or_else(|| filtered.first().copied())
                else {
                    return Task::none();
                };
                open_drawing_at(app, idx)
            }
            #[cfg(target_arch = "wasm32")]
            {
                Task::none()
            }
        }
        Message::SelectDetailTab(tab) => {
            app.pid_shell.backup.detail_tab = tab;
            Task::none()
        }
        Message::SelectItemType(item_type) => {
            app.pid_shell.backup.item_type_filter = item_type;
            Task::none()
        }
        Message::ItemQueryChanged(query) => {
            app.pid_shell.backup.item_query = query;
            Task::none()
        }
        Message::ToggleItemExpanded(uid) => {
            if !app.pid_shell.backup.expanded_items.remove(&uid) {
                app.pid_shell.backup.expanded_items.insert(uid);
            }
            Task::none()
        }
        Message::Export(msg) => super::backup_export_ui::update(app, msg),
    }
}

fn card_style(theme: PidTheme, fill: ColorToken, stroke: ColorToken) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill.color(theme))),
        border: Border {
            color: stroke.color(theme),
            width: 1.0,
            radius: NumberToken::RadiusMd.value(theme).into(),
        },
        ..container::Style::default()
    }
}

fn send(msg: Message) -> AppMessage {
    AppMessage::PidShell(ShellMessage::Backup(msg))
}

fn action_button<'a>(
    theme: PidTheme,
    bounds_key: &'static str,
    label: &'static str,
    primary: bool,
    on_press: Option<Message>,
) -> Element<'a, AppMessage> {
    let enabled = on_press.is_some();
    let (bg, fg, stroke, weight) = if !enabled {
        (
            ColorToken::BgRaised,
            ColorToken::TextMuted,
            ColorToken::Border,
            font::Weight::Medium,
        )
    } else if primary {
        (
            ColorToken::Accent,
            ColorToken::TextOnAccent,
            ColorToken::Accent,
            font::Weight::Semibold,
        )
    } else {
        (
            ColorToken::BgRaised,
            ColorToken::TextPrimary,
            ColorToken::BorderStrong,
            font::Weight::Medium,
        )
    };
    let radius = NumberToken::RadiusSm.value(theme);
    let mut btn = button(
        text(label)
            .size(12)
            .font(pid_fonts::ui(theme, weight))
            .color(fg.color(theme)),
    )
    .padding(Padding::from([7, 12]))
    .style(move |_, status| {
        let fill = match status {
            button::Status::Hovered | button::Status::Pressed if enabled && !primary => {
                ColorToken::BgHover.color(theme)
            }
            _ => bg.color(theme),
        };
        button::Style {
            background: Some(Background::Color(fill)),
            text_color: fg.color(theme),
            border: Border {
                color: stroke.color(theme),
                width: 1.0,
                radius: radius.into(),
            },
            shadow: iced::Shadow::default(),
            snap: false,
        }
    });
    if let Some(msg) = on_press {
        btn = btn.on_press(send(msg));
    }

    PosReport::owned(part_id(bounds_key), btn).into()
}

fn search_field<'a>(
    theme: PidTheme,
    bounds_key: &'static str,
    placeholder: &'static str,
    value: &str,
    on_input: fn(String) -> Message,
    on_submit: Option<Message>,
) -> Element<'a, AppMessage> {
    let radius = NumberToken::RadiusSm.value(theme);
    let mut input = text_input(placeholder, value)
        .size(12)
        .font(pid_fonts::ui(theme, font::Weight::Normal))
        .padding([7, 10])
        .style(move |general: &iced::Theme, status| text_input::Style {
            background: Background::Color(ColorToken::BgInput.color(theme)),
            border: Border {
                color: ColorToken::BorderControl.color(theme),
                width: 1.0,
                radius: radius.into(),
            },
            placeholder: ColorToken::TextMuted.color(theme),
            value: ColorToken::TextPrimary.color(theme),
            selection: ColorToken::AccentSoft.color(theme),
            ..text_input::default(general, status)
        })
        .on_input(move |val| send(on_input(val)));
    if let Some(submit_msg) = on_submit {
        input = input.on_submit(send(submit_msg));
    }
    PosReport::owned(part_id(bounds_key), input).into()
}

fn export_busy(app: &OpenCADStudio) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.pid_backup_export.is_running() || app.pid_backup_pdf.is_running()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        false
    }
}

fn source_path_bar<'a>(app: &'a OpenCADStudio, theme: PidTheme) -> Element<'a, AppMessage> {
    let can_open =
        !clean_path_input(&app.pid_shell.backup.source_input).is_empty() && !export_busy(app);
    row![
        search_field(
            theme,
            "source_path_input",
            "或直接输入 / 粘贴 .zip 或备份文件夹路径…",
            &app.pid_shell.backup.source_input,
            Message::SourceInputChanged,
            can_open.then_some(Message::OpenTypedSource),
        ),
        action_button(
            theme,
            "open_source_path",
            "打开路径",
            false,
            can_open.then_some(Message::OpenTypedSource),
        ),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center)
    .into()
}

fn empty_view<'a>(app: &'a OpenCADStudio, theme: PidTheme) -> Element<'a, AppMessage> {
    let card = container(
        column![
            text("打开 SmartPlant P&ID 备份数据包")
                .size(18)
                .font(pid_fonts::ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextPrimary.color(theme)),
            text("选择本机 .zip 备份包或备份目录，按装置组浏览图纸、查看数据库字段与图内对象，并导出成对移交的 publish 文件与 PDF（无需连接协同服务器）。")
                .size(12.5)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
            row![
                action_button(
                    theme,
                    "open_zip",
                    "打开备份 zip",
                    true,
                    Some(Message::PickSource { folder: false })
                ),
                action_button(
                    theme,
                    "open_folder",
                    "打开备份文件夹",
                    true,
                    Some(Message::PickSource { folder: true })
                ),
            ]
            .spacing(10),
            source_path_bar(app, theme),
        ]
        .spacing(14),
    )
    .width(580)
    .padding([28, 32])
    .style(move |_| card_style(theme, ColorToken::BgPanel, ColorToken::Border));

    PosReport::owned(
        part_id("empty"),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .padding(32),
    )
    .into()
}

fn loading_view<'a>(
    app: &'a OpenCADStudio,
    source: &'a Path,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let secs = app
        .pid_backup_browser
        .loading_elapsed(iced::time::Instant::now())
        .map_or(0, |elapsed| elapsed.as_secs());
    let detail = format!("来源：{} · 已用 {secs} 秒", source.display());
    let card = container(
        column![
            text("正在还原并建立索引…")
                .size(18)
                .font(pid_fonts::ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextPrimary.color(theme)),
            text(detail)
                .size(12.5)
                .font(pid_fonts::mono(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
        ]
        .spacing(12),
    )
    .width(580)
    .padding([28, 32])
    .style(move |_| card_style(theme, ColorToken::BgPanel, ColorToken::Accent));

    PosReport::owned(
        part_id("loading"),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .padding(32),
    )
    .into()
}

fn error_view<'a>(
    app: &'a OpenCADStudio,
    reason: &'a str,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let src_line = app
        .pid_backup_browser
        .active_or_failed_source()
        .map(|src| format!("来源：{}", src.display()));
    let mut body = column![text("无法打开所选备份数据包")
        .size(18)
        .font(pid_fonts::ui(theme, font::Weight::Semibold))
        .color(ColorToken::Danger.color(theme)),]
    .spacing(12);
    if let Some(src_line) = src_line {
        body = body.push(
            text(src_line)
                .size(12)
                .font(pid_fonts::mono(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
        );
    }
    body = body.push(
        text(reason)
            .size(12.5)
            .font(pid_fonts::ui(theme, font::Weight::Normal))
            .color(ColorToken::TextPrimary.color(theme)),
    );
    body = body.push(
        row![
            action_button(
                theme,
                "retry_zip",
                "重新选择 zip",
                true,
                Some(Message::PickSource { folder: false })
            ),
            action_button(
                theme,
                "retry_folder",
                "重新选择文件夹",
                true,
                Some(Message::PickSource { folder: true })
            ),
        ]
        .spacing(10),
    );
    body = body.push(source_path_bar(app, theme));

    let card = container(body)
        .width(600)
        .padding([28, 32])
        .style(move |_| card_style(theme, ColorToken::DangerSoft, ColorToken::Danger));

    PosReport::owned(
        part_id("error"),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .padding(32),
    )
    .into()
}

fn package_header<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let titles = column![
        text(header_title_line(index))
            .size(15)
            .font(pid_fonts::ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextPrimary.color(theme)),
        text(header_meta_line(index))
            .size(11.5)
            .font(pid_fonts::ui(theme, font::Weight::Normal))
            .color(ColorToken::TextSecondary.color(theme)),
    ]
    .spacing(4);

    let busy = export_busy(app);
    let actions = row![
        super::backup_export_ui::header_export_buttons(app, theme),
        action_button(
            theme,
            "switch_zip",
            "切换 zip",
            false,
            (!busy).then_some(Message::PickSource { folder: false })
        ),
        action_button(
            theme,
            "switch_folder",
            "切换文件夹",
            false,
            (!busy).then_some(Message::PickSource { folder: true })
        ),
        action_button(
            theme,
            "clear_cache",
            "清理还原缓存",
            false,
            Some(Message::ClearCache)
        ),
        action_button(
            theme,
            "classic_panel",
            "在经典面板中查看库表",
            false,
            Some(Message::OpenClassicPanel)
        ),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let bar = container(
        row![titles, Space::new().width(Fill), actions]
            .spacing(12)
            .align_y(alignment::Vertical::Center),
    )
    .width(Fill)
    .padding([14, 20])
    .style(move |_| container::Style {
        background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
        border: Border {
            color: ColorToken::Border.color(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    });

    PosReport::owned(part_id("header"), bar).into()
}

fn push_group_nodes<'a>(
    out: &mut Vec<Element<'a, AppMessage>>,
    nodes: &'a [GroupNode],
    depth: usize,
    selected: Option<&str>,
    theme: PidTheme,
) {
    let radius = NumberToken::RadiusSm.value(theme);
    for node in nodes {
        let is_selected = selected == Some(node.path.as_str());
        let direct = node.drawings.len();
        let total = pid_backup_browser::group_total_drawings(node);
        let count_label = if total == direct {
            format!("{total}")
        } else {
            format!("{direct} / {total}")
        };
        let label = pid_backup_browser::screen_group_label(&node.name).to_owned();
        let indent = (depth as f32) * 14.0;
        let path_owned = node.path.clone();
        let bounds_name = format!("group:{}", node.path);
        let row_btn = button(
            row![
                Space::new().width(Length::Fixed(indent)),
                text(label)
                    .size(12)
                    .font(pid_fonts::ui(
                        theme,
                        if is_selected {
                            font::Weight::Semibold
                        } else {
                            font::Weight::Normal
                        }
                    ))
                    .color(
                        if is_selected {
                            ColorToken::TextPrimary
                        } else {
                            ColorToken::TextSecondary
                        }
                        .color(theme)
                    ),
                Space::new().width(Fill),
                text(count_label)
                    .size(11)
                    .font(pid_fonts::mono(theme, font::Weight::Normal))
                    .color(ColorToken::TextMuted.color(theme)),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center),
        )
        .width(Fill)
        .padding([6, 10])
        .style(move |_, status| {
            let bg = if is_selected {
                Some(Background::Color(ColorToken::AccentSoft.color(theme)))
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Some(Background::Color(ColorToken::BgHover.color(theme)))
            } else {
                None
            };
            button::Style {
                background: bg,
                text_color: ColorToken::TextPrimary.color(theme),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: radius.into(),
                },
                shadow: iced::Shadow::default(),
                snap: false,
            }
        })
        .on_press(send(Message::SelectGroup(Some(path_owned))));

        out.push(PosReport::owned(part_id(&bounds_name), row_btn).into());
        push_group_nodes(out, &node.children, depth + 1, selected, theme);
    }
}

fn group_tree_pane<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let selected = app.pid_shell.backup.selected_group.as_deref();
    let all_selected = selected.is_none();
    let radius = NumberToken::RadiusSm.value(theme);

    let all_btn = button(
        row![
            text("全部图纸")
                .size(12)
                .font(pid_fonts::ui(
                    theme,
                    if all_selected {
                        font::Weight::Semibold
                    } else {
                        font::Weight::Normal
                    }
                ))
                .color(
                    if all_selected {
                        ColorToken::TextPrimary
                    } else {
                        ColorToken::TextSecondary
                    }
                    .color(theme)
                ),
            Space::new().width(Fill),
            text(format!("{}", index.drawings.len()))
                .size(11)
                .font(pid_fonts::mono(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme)),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .width(Fill)
    .padding([6, 10])
    .style(move |_, status| {
        let bg = if all_selected {
            Some(Background::Color(ColorToken::AccentSoft.color(theme)))
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            Some(Background::Color(ColorToken::BgHover.color(theme)))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: ColorToken::TextPrimary.color(theme),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: radius.into(),
            },
            shadow: iced::Shadow::default(),
            snap: false,
        }
    })
    .on_press(send(Message::SelectGroup(None)));

    let mut rows: Vec<Element<'a, AppMessage>> =
        vec![PosReport::owned(part_id("group:all"), all_btn).into()];
    push_group_nodes(&mut rows, &index.plant_groups, 0, selected, theme);

    let content = column![
        text("装置组")
            .size(11)
            .font(pid_fonts::ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextMuted.color(theme)),
        scrollable(column(rows).spacing(4)).height(Fill),
    ]
    .spacing(8);

    PosReport::owned(
        part_id("group_pane"),
        container(content)
            .width(240)
            .height(Fill)
            .padding([12, 12])
            .style(move |_| container::Style {
                background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
                border: Border {
                    color: ColorToken::Border.color(theme),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .into()
}

fn drawing_list_pane<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    filtered: &[usize],
    selected_idx: Option<usize>,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let radius = NumberToken::RadiusSm.value(theme);
    let mut list_rows: Vec<Element<'a, AppMessage>> = Vec::with_capacity(filtered.len().max(1));

    if filtered.is_empty() {
        list_rows.push(
            text("没有匹配的图纸")
                .size(12)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme))
                .into(),
        );
    } else {
        for &idx in filtered {
            let entry = &index.drawings[idx];
            let is_selected = selected_idx == Some(idx);
            let status_badge = if entry.is_missing() {
                Some(("缺文件", ColorToken::Danger))
            } else if matches!(
                app.pid_backup_browser.items.get(&idx),
                Some(ItemsState::Failed(_))
            ) {
                Some(("对象读不出", ColorToken::Amber))
            } else {
                None
            };
            let grp = display_group_path(&entry.group_path);
            let mut sub_row = row![text(grp)
                .size(11)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),]
            .spacing(8)
            .align_y(alignment::Vertical::Center);
            if let Some((badge_text, badge_tok)) = status_badge {
                sub_row = sub_row.push(
                    text(badge_text)
                        .size(11)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(badge_tok.color(theme)),
                );
            }

            let row_btn = button(
                column![
                    text(entry.name.as_str())
                        .size(12.5)
                        .font(pid_fonts::ui(
                            theme,
                            if is_selected {
                                font::Weight::Semibold
                            } else {
                                font::Weight::Medium
                            }
                        ))
                        .color(ColorToken::TextPrimary.color(theme)),
                    sub_row,
                ]
                .spacing(3),
            )
            .width(Fill)
            .padding([8, 10])
            .style(move |_, status| {
                let bg = if is_selected {
                    Some(Background::Color(ColorToken::AccentSoft.color(theme)))
                } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    Some(Background::Color(ColorToken::BgHover.color(theme)))
                } else {
                    Some(Background::Color(ColorToken::BgPanel.color(theme)))
                };
                button::Style {
                    background: bg,
                    text_color: ColorToken::TextPrimary.color(theme),
                    border: Border {
                        color: ColorToken::Border.color(theme),
                        width: 1.0,
                        radius: radius.into(),
                    },
                    shadow: iced::Shadow::default(),
                    snap: false,
                }
            })
            .on_press(send(Message::SelectDrawing(idx)));

            list_rows.push(PosReport::owned(part_id(&format!("drawing:{idx}")), row_btn).into());
        }
    }

    let header_count = format!("图纸清单（{} / {}）", filtered.len(), index.drawings.len());
    let content = column![
        text(header_count)
            .size(11)
            .font(pid_fonts::ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextMuted.color(theme)),
        search_field(
            theme,
            "drawing_search",
            "按图纸名称搜索过滤…",
            &app.pid_shell.backup.drawing_filter,
            Message::DrawingFilterChanged,
            None,
        ),
        scrollable(column(list_rows).spacing(4)).height(Fill),
    ]
    .spacing(8);

    PosReport::owned(
        part_id("drawing_list_pane"),
        container(content)
            .width(380)
            .height(Fill)
            .padding([12, 12])
            .style(move |_| container::Style {
                background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
                border: Border {
                    color: ColorToken::Border.color(theme),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .into()
}

fn db_properties_card<'a>(
    entry: &'a DrawingEntry,
    items_state: Option<&'a ItemsState>,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let mut rows: Vec<Element<'a, AppMessage>> = Vec::new();
    rows.push(
        text("图纸属性（数据库登记列与图纸对象标识）")
            .size(12.5)
            .font(pid_fonts::ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextPrimary.color(theme))
            .into(),
    );
    if let Some(ItemsState::Ready(items)) = items_state {
        let uid_label = if items.drawing_uid.is_empty() {
            "（空）"
        } else {
            items.drawing_uid.as_str()
        };
        rows.push(
            text(format!(
                "DrawingNo: {} · 文件图名: {}",
                uid_label, items.drawing_name
            ))
            .size(11.5)
            .font(pid_fonts::mono(theme, font::Weight::Medium))
            .color(ColorToken::Accent.color(theme))
            .into(),
        );
    }
    if entry.meta.is_empty() {
        rows.push(
            text("（无数据库登记字段）")
                .size(11.5)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme))
                .into(),
        );
    } else {
        for (col_name, col_val) in &entry.meta {
            let shown = if col_val.is_empty() {
                "（空）"
            } else {
                col_val.as_str()
            };
            rows.push(
                row![
                    text(format!("{col_name}:"))
                        .size(11.5)
                        .font(pid_fonts::mono(theme, font::Weight::Medium))
                        .color(ColorToken::TextSecondary.color(theme)),
                    text(shown)
                        .size(11.5)
                        .font(pid_fonts::mono(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(8)
                .into(),
            );
        }
    }

    PosReport::owned(
        part_id("db_fields"),
        container(scrollable(column(rows).spacing(4)).height(160))
            .width(Fill)
            .padding([12, 14])
            .style(move |_| card_style(theme, ColorToken::BgPanel, ColorToken::Border)),
    )
    .into()
}

fn items_ready_body<'a>(
    app: &'a OpenCADStudio,
    items: &'a DrawingItems,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let counts = pid_backup_browser::item_type_counts(items);
    let active_type = app.pid_shell.backup.item_type_filter.as_deref();
    let radius = NumberToken::RadiusSm.value(theme);

    let make_chip = |bounds_name: String,
                     chip_label: String,
                     selected: bool,
                     target: Option<String>|
     -> Element<'a, AppMessage> {
        let btn = button(
            text(chip_label)
                .size(11)
                .font(pid_fonts::ui(
                    theme,
                    if selected {
                        font::Weight::Semibold
                    } else {
                        font::Weight::Normal
                    },
                ))
                .color(
                    if selected {
                        ColorToken::TextOnAccent
                    } else {
                        ColorToken::TextSecondary
                    }
                    .color(theme),
                ),
        )
        .padding([4, 9])
        .style(move |_, _| button::Style {
            background: Some(Background::Color(
                if selected {
                    ColorToken::Accent
                } else {
                    ColorToken::BgRaised
                }
                .color(theme),
            )),
            text_color: ColorToken::TextPrimary.color(theme),
            border: Border {
                color: if selected {
                    ColorToken::Accent
                } else {
                    ColorToken::Border
                }
                .color(theme),
                width: 1.0,
                radius: radius.into(),
            },
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .on_press(send(Message::SelectItemType(target)));
        PosReport::owned(part_id(&bounds_name), btn).into()
    };

    let mut chips: Vec<Element<'a, AppMessage>> = Vec::with_capacity(counts.len() + 1);
    chips.push(make_chip(
        "item_type:all".to_owned(),
        format!("全部 ({})", items.item_count),
        active_type.is_none(),
        None,
    ));
    for (ty, cnt) in counts {
        chips.push(make_chip(
            format!("item_type:{ty}"),
            format!("{ty} ({cnt})"),
            active_type == Some(ty),
            Some(ty.to_owned()),
        ));
    }

    let matched = pid_backup_browser::filter_drawing_items(
        items,
        active_type,
        &app.pid_shell.backup.item_query,
    );
    let mut item_rows: Vec<Element<'a, AppMessage>> =
        Vec::with_capacity(matched.len().min(MAX_RENDERED_ITEMS) + 1);

    if matched.is_empty() {
        item_rows.push(
            text("当前筛选条件下没有图内对象")
                .size(12)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme))
                .into(),
        );
    } else {
        for item in matched.iter().take(MAX_RENDERED_ITEMS) {
            let expanded = app.pid_shell.backup.expanded_items.contains(&item.uid);
            let cls_suffix = item
                .class
                .as_deref()
                .map(|c| format!(" · {c}"))
                .unwrap_or_default();
            let header_line = format!(
                "{} {} · {}{} · UID {}",
                if expanded { "▼" } else { "▶" },
                item.label,
                item.item_type,
                cls_suffix,
                item.uid
            );
            let uid_owned = item.uid.clone();
            let mut item_col = column![button(
                text(header_line)
                    .size(11.5)
                    .font(pid_fonts::mono(theme, font::Weight::Medium))
                    .color(ColorToken::TextPrimary.color(theme)),
            )
            .width(Fill)
            .padding([5, 8])
            .style(move |_, status| button::Style {
                background: Some(Background::Color(
                    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                        ColorToken::BgHover
                    } else {
                        ColorToken::BgRaised
                    }
                    .color(theme),
                )),
                text_color: ColorToken::TextPrimary.color(theme),
                border: Border {
                    color: ColorToken::Border.color(theme),
                    width: 1.0,
                    radius: radius.into(),
                },
                shadow: iced::Shadow::default(),
                snap: false,
            })
            .on_press(send(Message::ToggleItemExpanded(uid_owned))),]
            .spacing(3);

            if expanded {
                for attr in &item.attributes {
                    item_col = item_col.push(
                        text(format!(
                            "  {}.{} = {}",
                            attr.interface, attr.name, attr.value
                        ))
                        .size(11)
                        .font(pid_fonts::mono(theme, font::Weight::Normal))
                        .color(ColorToken::TextSecondary.color(theme)),
                    );
                }
            }
            item_rows
                .push(PosReport::owned(part_id(&format!("item:{}", item.uid)), item_col).into());
        }
        if matched.len() > MAX_RENDERED_ITEMS {
            item_rows.push(
                text(format!(
                    "已显示前 {MAX_RENDERED_ITEMS} 条（共 {} 条），可输入关键字进一步过滤。",
                    matched.len()
                ))
                .size(11)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme))
                .into(),
            );
        }
    }

    column![
        text(format!("图内对象（共 {} 个）", items.item_count))
            .size(12.5)
            .font(pid_fonts::ui(theme, font::Weight::Semibold))
            .color(ColorToken::TextPrimary.color(theme)),
        scrollable(row(chips).spacing(6)).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default()
        )),
        search_field(
            theme,
            "item_search",
            "按位号、UID、类别或属性值过滤对象…",
            &app.pid_shell.backup.item_query,
            Message::ItemQueryChanged,
            None,
        ),
        scrollable(column(item_rows).spacing(4)).height(Fill),
    ]
    .spacing(8)
    .height(Fill)
    .into()
}

fn detail_pane<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    selected_idx: Option<usize>,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let Some(idx) = selected_idx.filter(|&i| i < index.drawings.len()) else {
        return container(
            text("请从左侧选择一张图纸查看数据库字段与图内对象")
                .size(12.5)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme)),
        )
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into();
    };

    let entry = &index.drawings[idx];
    let items_state = app.pid_backup_browser.items.get(&idx);
    let is_opening = entry
        .pid_path
        .as_deref()
        .is_some_and(|p| app.pid_shell.backup.is_opening_path(p));
    let open_failure = entry
        .pid_path
        .as_ref()
        .and_then(|p| app.pid_shell.backup.open_failures.get(p));

    let cur_tab = app.pid_shell.backup.detail_tab;
    let tab_pill = |bounds_key: &'static str, label: &'static str, tab: DetailTab| {
        action_button(
            theme,
            bounds_key,
            label,
            cur_tab == tab,
            Some(Message::SelectDetailTab(tab)),
        )
    };
    let open_btn = action_button(
        theme,
        "open_drawing",
        if is_opening {
            "正在打开…"
        } else {
            "在编辑器中打开"
        },
        true,
        (!entry.is_missing() && !is_opening).then_some(Message::OpenSelectedDrawing),
    );

    let mut top = column![row![
        column![
            text(entry.name.as_str())
                .size(15)
                .font(pid_fonts::ui(theme, font::Weight::Semibold))
                .color(ColorToken::TextPrimary.color(theme)),
            text(format!("所属组：{}", display_group_path(&entry.group_path)))
                .size(11.5)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
        ]
        .spacing(2),
        Space::new().width(Fill),
        tab_pill("detail_tab:all", "全部", DetailTab::All),
        tab_pill("detail_tab:props", "仅属性", DetailTab::Properties),
        tab_pill("detail_tab:items", "仅对象", DetailTab::Items),
        open_btn,
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center),]
    .spacing(10);

    if entry.is_missing() {
        top = top.push(PosReport::owned(
            part_id("missing_file_notice"),
            container(
                text("该图纸仅在数据库中登记，备份包内缺少对应的 .pid 文件，无法在编辑器中打开。")
                    .size(12)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(ColorToken::Danger.color(theme)),
            )
            .width(Fill)
            .padding([8, 12])
            .style(move |_| card_style(theme, ColorToken::DangerSoft, ColorToken::Danger)),
        ));
    }
    if let Some(open_err) = open_failure {
        top = top.push(PosReport::owned(
            part_id("open_error"),
            container(
                column![
                    text("无法在编辑器中打开该图纸")
                        .size(12.5)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Danger.color(theme)),
                    text(open_err.as_str())
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(4),
            )
            .width(Fill)
            .padding([9, 12])
            .style(move |_| card_style(theme, ColorToken::DangerSoft, ColorToken::Danger)),
        ));
    }

    let items_card_content: Element<'a, AppMessage> = if entry.is_missing() {
        text("因缺少 .pid 图纸文件，无法读取图内对象。")
            .size(12)
            .font(pid_fonts::ui(theme, font::Weight::Normal))
            .color(ColorToken::TextSecondary.color(theme))
            .into()
    } else {
        match items_state {
            Some(ItemsState::Ready(items)) => items_ready_body(app, items, theme),
            Some(ItemsState::Failed(reason)) => PosReport::owned(
                part_id("items_error"),
                container(
                    column![
                        text("无法读取图内对象")
                            .size(12.5)
                            .font(pid_fonts::ui(theme, font::Weight::Semibold))
                            .color(ColorToken::Amber.color(theme)),
                        text(reason.as_str())
                            .size(12)
                            .font(pid_fonts::ui(theme, font::Weight::Normal))
                            .color(ColorToken::TextPrimary.color(theme)),
                    ]
                    .spacing(4),
                )
                .width(Fill)
                .padding([10, 12])
                .style(move |_| card_style(theme, ColorToken::AmberSoft, ColorToken::Amber)),
            )
            .into(),
            Some(ItemsState::Loading) => text("正在读取图内对象…")
                .size(12)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme))
                .into(),
            None => text("尚未读取图内对象")
                .size(12)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextMuted.color(theme))
                .into(),
        }
    };

    let items_card: Element<'a, AppMessage> = PosReport::owned(
        part_id("items_card"),
        container(items_card_content)
            .width(Fill)
            .height(Fill)
            .padding([12, 14])
            .style(move |_| card_style(theme, ColorToken::BgPanel, ColorToken::Border)),
    )
    .into();

    let mut pane_col = column![top].spacing(12).width(Fill).height(Fill);
    if matches!(
        app.pid_shell.backup.detail_tab,
        DetailTab::All | DetailTab::Properties
    ) {
        pane_col = pane_col.push(db_properties_card(entry, items_state, theme));
    }
    if matches!(
        app.pid_shell.backup.detail_tab,
        DetailTab::All | DetailTab::Items
    ) {
        pane_col = pane_col.push(items_card);
    }

    container(pane_col)
        .width(Fill)
        .height(Fill)
        .padding([14, 18])
        .into()
}

fn ready_view<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let mut root = column![package_header(app, index, theme)];
    if let Some(banners) = super::backup_export_ui::status_banners(app, index, theme) {
        root = root.push(banners);
    }
    if let Some((tone, msg)) = &app.pid_shell.backup.notice {
        let (bg, fg) = match tone {
            NoticeTone::Info => (ColorToken::AccentSoft, ColorToken::TextPrimary),
            NoticeTone::Warning => (ColorToken::AmberSoft, ColorToken::Amber),
        };
        root = root.push(
            container(
                text(msg.as_str())
                    .size(12)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(fg.color(theme)),
            )
            .width(Fill)
            .padding([8, 20])
            .style(move |_| container::Style {
                background: Some(Background::Color(bg.color(theme))),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            }),
        );
    }

    let filtered = pid_backup_browser::filter_drawings(
        index,
        app.pid_shell.backup.selected_group.as_deref(),
        &app.pid_shell.backup.drawing_filter,
    );
    let selected_idx = app
        .pid_shell
        .backup
        .selected_drawing
        .filter(|idx| filtered.contains(idx))
        .or_else(|| filtered.first().copied());

    let three_pane = row![
        group_tree_pane(app, index, theme),
        drawing_list_pane(app, index, &filtered, selected_idx, theme),
        detail_pane(app, index, selected_idx, theme),
    ]
    .width(Fill)
    .height(Fill);

    root = root.push(three_pane);
    let base: Element<'a, AppMessage> = root.width(Fill).height(Fill).into();
    match super::backup_export_ui::dialog_overlay(app, index, theme) {
        Some(overlay) => stack![base, overlay].width(Fill).height(Fill).into(),
        None => base,
    }
}

/// Draw the backup package screen inside the shell's content area.
pub(crate) fn view(app: &OpenCADStudio) -> Element<'_, AppMessage> {
    let theme = app.pid_shell.theme;
    let body = match &app.pid_backup_browser.load {
        LoadState::Idle => match app.pid_backup_browser.error.as_deref() {
            Some(reason) => error_view(app, reason, theme),
            None => empty_view(app, theme),
        },
        LoadState::Loading(source) => loading_view(app, source, theme),
        LoadState::Ready(index) => ready_view(app, index, theme),
    };
    container(body)
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(ColorToken::BgApp.color(theme))),
            ..container::Style::default()
        })
        .into()
}

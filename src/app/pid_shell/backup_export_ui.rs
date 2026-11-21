//! Export dialogs, live running banners, and structured outcome summaries for
//! the `backup` screen (`UI-7` and `UI-8`, `SPEC-sppid-export-ui.md`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use iced::widget::{button, column, container, opaque, row, scrollable, text, text_input, Space};
use iced::{alignment, font, Background, Border, Color, Element, Fill, Length, Padding, Task};

use super::backup::{clean_path_input, is_sql_server_package, SCREEN_BOUNDS_PREFIX};
use super::{Message as ShellMessage, PidScreen};
use crate::app::{Message as AppMessage, OpenCADStudio};
use crate::io::pid_backup_browse::BackupIndex;
use crate::io::pid_backup_export::{self, ExportError, ExportOutcome};
#[cfg(not(target_arch = "wasm32"))]
use crate::io::pid_backup_pdf::{PdfError, PdfOutcome};
use crate::ui::style::pid_fonts;
use crate::ui::style::pid_tokens::{ColorToken, NumberToken, PidTheme};
use crate::ui::wrap_bar::PosReport;

fn part_id(name: &str) -> String {
    format!("{SCREEN_BOUNDS_PREFIX}{name}")
}

/// Injectable folder picker for headless tests (`"pub_out"`, `"pub_ref"`, `"pdf_out"`).
pub type FolderPicker = Arc<dyn Fn(&str) -> Option<Option<PathBuf>> + Send + Sync>;

/// Injectable OS file/folder opener (`"打开输出目录"`, `"打开清单文件"`).
pub type PathOpener = Arc<dyn Fn(&Path) + Send + Sync>;

/// Active modal dialog inside the `backup` screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportDialog {
    Publish(PublishDialogState),
    Pdf(PdfDialogState),
}

/// Draft inputs of the publish-files export dialog (`SPEC-sppid-export-ui.md` §3.2 A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishDialogState {
    pub out_dir: String,
    pub plant: String,
    pub reference_dir: String,
    /// Cached validation result recomputed on input change in `update()`.
    pub validation: Result<(), String>,
}

impl Default for PublishDialogState {
    fn default() -> Self {
        Self {
            out_dir: String::new(),
            plant: pid_backup_export::DEFAULT_PLANT.to_owned(),
            reference_dir: String::new(),
            validation: Err("请填写或选择输出目录（必须是不存在或空的目录）。".to_owned()),
        }
    }
}

/// Draft inputs of the PDF batch export dialog (`SPEC-sppid-export-ui.md` §3.2 B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfDialogState {
    pub out_dir: String,
    /// Cached validation result recomputed on input change in `update()`.
    pub validation: Result<(), String>,
    /// Whether `out_dir` already holds publish files ready for pairing.
    pub pair_ready: bool,
}

impl Default for PdfDialogState {
    fn default() -> Self {
        Self {
            out_dir: String::new(),
            validation: Err("请填写或选择 PDF 输出目录。".to_owned()),
            pair_ready: false,
        }
    }
}

/// UI state owned by `backup::State` for the two export workflows.
#[derive(Default)]
pub struct ExportUiState {
    /// Open configuration dialog, when one is up.
    pub dialog: Option<ExportDialog>,
    /// Most recent output folder passed to publish export, offered as a one-click
    /// pairing shortcut in the PDF export dialog.
    pub last_publish_out_dir: Option<PathBuf>,
    /// Whether the publish outcome details drawer is expanded.
    pub publish_details_open: bool,
    /// Whether the PDF outcome details drawer is expanded.
    pub pdf_details_open: bool,
    /// Optional test double replacing native folder pickers.
    pub folder_picker: Option<FolderPicker>,
    /// Optional test double replacing OS file/folder opening.
    pub path_opener: Option<PathOpener>,
}

impl ExportUiState {
    /// Clear package-bound dialog/outcome state when switching to a newly loaded package.
    pub fn reset_for_loaded_package(&mut self) {
        self.dialog = None;
        self.last_publish_out_dir = None;
        self.publish_details_open = false;
        self.pdf_details_open = false;
    }
}

impl std::fmt::Debug for ExportUiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportUiState")
            .field("dialog", &self.dialog)
            .field("last_publish_out_dir", &self.last_publish_out_dir)
            .field("publish_details_open", &self.publish_details_open)
            .field("pdf_details_open", &self.pdf_details_open)
            .finish()
    }
}

/// Messages handled by the export UI module.
#[derive(Debug, Clone)]
pub enum ExportUiMessage {
    OpenPublishDialog,
    OpenPdfDialog,
    CloseDialog,
    PublishOutChanged(String),
    PublishPlantChanged(String),
    PublishRefChanged(String),
    PublishClearRef,
    BrowseFolder(&'static str),
    FolderPicked(&'static str, Option<PathBuf>),
    PdfOutChanged(String),
    PdfUseLastPublishDir,
    SubmitPublish,
    SubmitPdf,
    TogglePublishDetails,
    TogglePdfDetails,
    OpenPath(PathBuf),
}

fn send(msg: ExportUiMessage) -> AppMessage {
    AppMessage::PidShell(ShellMessage::Backup(super::backup::Message::Export(msg)))
}

/// Whether `index` has `Manifest.txt` recorded in its in-memory index.
pub fn has_manifest(index: &BackupIndex) -> bool {
    index.manifest.is_some()
        || index
            .files
            .iter()
            .any(|file| file.name.eq_ignore_ascii_case("Manifest.txt"))
}

/// Reason the "导出 publish 文件" button is disabled (`None` when enabled).
pub(crate) fn publish_disabled_reason(app: &OpenCADStudio) -> Option<&'static str> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.pid_backup_export.is_running() {
            return Some("正在导出 publish 文件…");
        }
        let Some(index) = app.pid_backup_browser.index() else {
            return Some("尚未打开备份数据包");
        };
        if !has_manifest(index) {
            return Some("当前目录不含备份清单 Manifest.txt，无法导出 publish 文件");
        }
        if !index.drawings.iter().any(|d| d.pid_path.is_some()) {
            return Some("备份包中没有带文件的 .pid 图纸");
        }
        None
    }
    #[cfg(target_arch = "wasm32")]
    Some("网页端不支持本地备份包导出")
}

/// Reason the "导出 PDF" button is disabled (`None` when enabled).
pub(crate) fn pdf_disabled_reason(app: &OpenCADStudio) -> Option<&'static str> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if app.pid_backup_pdf.is_running() {
            return Some("正在导出 PDF…");
        }
        let Some(index) = app.pid_backup_browser.index() else {
            return Some("尚未打开备份数据包");
        };
        if !index.drawings.iter().any(|d| d.pid_path.is_some()) {
            return Some("备份包中没有可出图的 .pid 文件");
        }
        None
    }
    #[cfg(target_arch = "wasm32")]
    Some("网页端不支持本地备份包导出")
}

/// Last publish output folder belonging to the currently open backup package.
pub(crate) fn resolved_last_publish_dir(app: &OpenCADStudio) -> Option<PathBuf> {
    if let Some(dir) = &app.pid_shell.backup.export_ui.last_publish_out_dir {
        return Some(dir.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(index) = app.pid_backup_browser.index() {
        if let Some(Ok(outcome)) = app.pid_backup_export.last_outcome_for(&index.backup_root) {
            if let Some(parent) = outcome.index_path.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

fn compute_publish_validation(
    app: &OpenCADStudio,
    state: &PublishDialogState,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let out_clean = clean_path_input(&state.out_dir);
        let plant_trim = state.plant.trim();
        let ref_clean = clean_path_input(&state.reference_dir);
        let ref_opt = (!ref_clean.is_empty()).then(|| PathBuf::from(ref_clean));
        if out_clean.is_empty() {
            Err("请填写或选择输出目录（必须是不存在或空的目录）。".to_owned())
        } else if plant_trim.is_empty() {
            Err("工厂名不能为空（默认 P01）。".to_owned())
        } else {
            app.check_pid_backup_export_ui(PathBuf::from(out_clean), plant_trim, ref_opt)
                .map(|_| ())
                .map_err(|err| format!("[{}] {err}", err.code()))
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, state);
        Err("网页端不可用".to_owned())
    }
}

fn compute_pdf_validation(app: &OpenCADStudio, out_raw: &str) -> (Result<(), String>, bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let out_clean = clean_path_input(out_raw);
        if out_clean.is_empty() {
            (Err("请填写或选择 PDF 输出目录。".to_owned()), false)
        } else {
            match app.check_pid_backup_pdf_ui(PathBuf::from(out_clean)) {
                Ok(req) => {
                    let pair = directory_holds_publish_files(&req.out_dir);
                    (Ok(()), pair)
                }
                Err(err) => (Err(format!("[{}] {err}", err.code())), false),
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, out_raw);
        (Err("网页端不可用".to_owned()), false)
    }
}

fn revalidate_open_dialog(app: &mut OpenCADStudio) {
    let next = match app.pid_shell.backup.export_ui.dialog.clone() {
        Some(ExportDialog::Publish(mut state)) => {
            state.validation = compute_publish_validation(app, &state);
            Some(ExportDialog::Publish(state))
        }
        Some(ExportDialog::Pdf(mut state)) => {
            let (validation, pair_ready) = compute_pdf_validation(app, &state.out_dir);
            state.validation = validation;
            state.pair_ready = pair_ready;
            Some(ExportDialog::Pdf(state))
        }
        None => None,
    };
    app.pid_shell.backup.export_ui.dialog = next;
}

fn directory_holds_publish_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        s == "_Index.json" || s.ends_with("_Data.xml") || s.ends_with("_Meta.xml")
    })
}

#[cfg(not(target_arch = "wasm32"))]
async fn pick_export_folder_async(title: &'static str) -> Option<PathBuf> {
    crate::sys::file_dialog()
        .set_title(title)
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

pub(crate) fn update(app: &mut OpenCADStudio, message: ExportUiMessage) -> Task<AppMessage> {
    match message {
        ExportUiMessage::OpenPublishDialog => {
            if publish_disabled_reason(app).is_none() {
                app.pid_shell.backup.export_ui.dialog =
                    Some(ExportDialog::Publish(PublishDialogState::default()));
                revalidate_open_dialog(app);
            }
            Task::none()
        }
        ExportUiMessage::OpenPdfDialog => {
            if pdf_disabled_reason(app).is_none() {
                app.pid_shell.backup.export_ui.dialog =
                    Some(ExportDialog::Pdf(PdfDialogState::default()));
                revalidate_open_dialog(app);
            }
            Task::none()
        }
        ExportUiMessage::CloseDialog => {
            app.pid_shell.backup.export_ui.dialog = None;
            Task::none()
        }
        ExportUiMessage::PublishOutChanged(val) => {
            if let Some(ExportDialog::Publish(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                state.out_dir = val;
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::PublishPlantChanged(val) => {
            if let Some(ExportDialog::Publish(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                state.plant = val;
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::PublishRefChanged(val) => {
            if let Some(ExportDialog::Publish(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                state.reference_dir = val;
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::PublishClearRef => {
            if let Some(ExportDialog::Publish(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                state.reference_dir.clear();
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::PdfOutChanged(val) => {
            if let Some(ExportDialog::Pdf(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                state.out_dir = val;
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::PdfUseLastPublishDir => {
            if let Some(last_dir) = resolved_last_publish_dir(app) {
                if let Some(ExportDialog::Pdf(state)) = &mut app.pid_shell.backup.export_ui.dialog {
                    state.out_dir = last_dir.display().to_string();
                }
                revalidate_open_dialog(app);
            }
            Task::none()
        }
        ExportUiMessage::BrowseFolder(slot) => {
            if let Some(picker) = app.pid_shell.backup.export_ui.folder_picker.clone() {
                if let Some(picked) = picker(slot) {
                    return update(app, ExportUiMessage::FolderPicked(slot, picked));
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let title = match slot {
                    "pub_ref" => "选择参考 _Data.xml 目录",
                    _ => "选择导出输出目录",
                };
                Task::perform(pick_export_folder_async(title), move |picked| {
                    send(ExportUiMessage::FolderPicked(slot, picked))
                })
            }
            #[cfg(target_arch = "wasm32")]
            {
                Task::none()
            }
        }
        ExportUiMessage::FolderPicked(slot, Some(path)) => {
            let text_val = path.display().to_string();
            match (&mut app.pid_shell.backup.export_ui.dialog, slot) {
                (Some(ExportDialog::Publish(state)), "pub_out") => state.out_dir = text_val,
                (Some(ExportDialog::Publish(state)), "pub_ref") => state.reference_dir = text_val,
                (Some(ExportDialog::Pdf(state)), "pdf_out") => state.out_dir = text_val,
                _ => {}
            }
            revalidate_open_dialog(app);
            Task::none()
        }
        ExportUiMessage::FolderPicked(_, None) => Task::none(),
        ExportUiMessage::SubmitPublish => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let Some(ExportDialog::Publish(state)) =
                    app.pid_shell.backup.export_ui.dialog.clone()
                else {
                    return Task::none();
                };
                let out_clean = clean_path_input(&state.out_dir);
                let plant_trim = state.plant.trim();
                let ref_clean = clean_path_input(&state.reference_dir);
                if out_clean.is_empty() || plant_trim.is_empty() {
                    revalidate_open_dialog(app);
                    return Task::none();
                }
                let ref_opt = (!ref_clean.is_empty()).then(|| PathBuf::from(ref_clean));
                let Ok(checked) = app.check_pid_backup_export_ui(
                    PathBuf::from(out_clean),
                    plant_trim,
                    ref_opt.clone(),
                ) else {
                    revalidate_open_dialog(app);
                    return Task::none();
                };
                if let Ok(task) = app.start_pid_backup_export_ui(
                    PathBuf::from(out_clean),
                    plant_trim.to_owned(),
                    ref_opt,
                ) {
                    app.pid_shell.backup.export_ui.last_publish_out_dir = Some(checked.out_dir);
                    app.pid_shell.backup.export_ui.dialog = None;
                    return task;
                }
                revalidate_open_dialog(app);
            }
            Task::none()
        }
        ExportUiMessage::SubmitPdf => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let Some(ExportDialog::Pdf(state)) = app.pid_shell.backup.export_ui.dialog.clone()
                else {
                    return Task::none();
                };
                let out_clean = clean_path_input(&state.out_dir);
                if out_clean.is_empty() {
                    revalidate_open_dialog(app);
                    return Task::none();
                }
                if let Ok(task) = app.start_pid_backup_pdf_ui(PathBuf::from(out_clean)) {
                    app.pid_shell.backup.export_ui.dialog = None;
                    return task;
                }
                revalidate_open_dialog(app);
            }
            Task::none()
        }
        ExportUiMessage::TogglePublishDetails => {
            let open = &mut app.pid_shell.backup.export_ui.publish_details_open;
            *open = !*open;
            Task::none()
        }
        ExportUiMessage::TogglePdfDetails => {
            let open = &mut app.pid_shell.backup.export_ui.pdf_details_open;
            *open = !*open;
            Task::none()
        }
        ExportUiMessage::OpenPath(path) => {
            if let Some(opener) = &app.pid_shell.backup.export_ui.path_opener {
                opener(&path);
            } else {
                let _ = open::that_detached(&path);
            }
            Task::none()
        }
    }
}

/// Global status-bar items added to `chrome::status_parts` while exports run (`U1`).
pub(crate) fn status_bar_parts(
    app: &OpenCADStudio,
    screen: PidScreen,
) -> Vec<super::chrome::StatusPart> {
    #[allow(unused_mut)]
    let mut out = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = Instant::now();
        if let Some((_, elapsed)) = app.pid_backup_export.running_status(now) {
            out.push(super::chrome::StatusPart {
                label: format!("正在导出 publish · {}s", elapsed.as_secs()),
                switch: (screen != PidScreen::Backup).then_some((
                    "backup_export",
                    AppMessage::PidShell(ShellMessage::Show(PidScreen::Backup)),
                )),
            });
        }
        if let Some((_, done, total, _)) = app.pid_backup_pdf.running_status(now) {
            out.push(super::chrome::StatusPart {
                label: format!("正在导出 PDF {done}/{total}"),
                switch: (screen != PidScreen::Backup).then_some((
                    "backup_pdf",
                    AppMessage::PidShell(ShellMessage::Show(PidScreen::Backup)),
                )),
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, screen);
    out
}

fn btn_el<'a>(
    theme: PidTheme,
    bounds_key: &'static str,
    label: &'static str,
    primary: bool,
    on_press: Option<ExportUiMessage>,
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
    let mut b = button(
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
        b = b.on_press(send(msg));
    }
    PosReport::owned(part_id(bounds_key), b).into()
}

fn dialog_input<'a>(
    theme: PidTheme,
    bounds_key: &'static str,
    placeholder: &'static str,
    value: &str,
    on_input: fn(String) -> ExportUiMessage,
) -> Element<'a, AppMessage> {
    let radius = NumberToken::RadiusSm.value(theme);
    let inp = text_input(placeholder, value)
        .size(12)
        .font(pid_fonts::mono(theme, font::Weight::Normal))
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
        .on_input(move |v| send(on_input(v)));
    PosReport::owned(part_id(bounds_key), inp).into()
}

/// Header action buttons for publish & PDF exports (`UI-7`).
pub(crate) fn header_export_buttons<'a>(
    app: &'a OpenCADStudio,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    let pub_reason = publish_disabled_reason(app);
    let pdf_reason = pdf_disabled_reason(app);
    let mut r = row![
        btn_el(
            theme,
            "export_publish",
            "导出 publish 文件",
            pub_reason.is_none(),
            pub_reason
                .is_none()
                .then_some(ExportUiMessage::OpenPublishDialog),
        ),
        btn_el(
            theme,
            "export_pdf",
            "导出 PDF",
            pdf_reason.is_none(),
            pdf_reason
                .is_none()
                .then_some(ExportUiMessage::OpenPdfDialog),
        ),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    if let Some(why) = pub_reason.filter(|w| w.contains("Manifest.txt")) {
        r = r.push(PosReport::owned(
            part_id("export_publish_disabled_hint"),
            text(why)
                .size(11)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::Amber.color(theme)),
        ));
    }
    r.into()
}

#[cfg(not(target_arch = "wasm32"))]
fn publish_outcome_banner<'a>(
    app: &'a OpenCADStudio,
    result: &'a Result<ExportOutcome, ExportError>,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    match result {
        Ok(outcome) => {
            let has_gate_diff = outcome
                .gate
                .as_ref()
                .is_some_and(|g| g.dirty > 0 || g.revision_mismatch > 0 || g.unpaired > 0);
            let has_warnings = outcome.failed > 0
                || !outcome.unwritten.is_empty()
                || !outcome.noted.is_empty()
                || !outcome.other_problems.is_empty()
                || !outcome.warnings.is_empty()
                || outcome.sql_server()
                || has_gate_diff;
            let (bg, stroke, badge_color) = if has_warnings {
                (ColorToken::AmberSoft, ColorToken::Amber, ColorToken::Amber)
            } else {
                (
                    ColorToken::SuccessSoft,
                    ColorToken::Success,
                    ColorToken::Success,
                )
            };
            let title = format!(
                "publish 文件导出完成：写出 {} 张（{} 个文件，失败计数 {}）· 用时 {:.1} 秒",
                outcome.written,
                outcome.files,
                outcome.failed,
                outcome.elapsed.as_secs_f64()
            );
            let mut sub_parts = vec![format!("清单：{}", outcome.index_path.display())];
            if outcome.without_drawing_no > 0 {
                sub_parts.push(format!("无 DrawingNo {} 张", outcome.without_drawing_no));
            }
            if let Some(gate) = &outcome.gate {
                sub_parts.push(format!(
                    "参考比对：一致 {} / 有差异 {} / 另一版 {} / 无参考 {}",
                    gate.clean, gate.dirty, gate.revision_mismatch, gate.unpaired
                ));
            }
            if outcome.sql_server() {
                sub_parts.push("该包为 SQL Server 备份，导出内容未经参考验收".to_owned());
            }
            let out_dir = outcome
                .index_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| outcome.index_path.clone());
            let details_open = app.pid_shell.backup.export_ui.publish_details_open;

            let summary_row = row![
                column![
                    text(title)
                        .size(12.5)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(badge_color.color(theme)),
                    text(sub_parts.join(" · "))
                        .size(11.5)
                        .font(pid_fonts::ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(3),
                Space::new().width(Fill),
                btn_el(
                    theme,
                    "pub_toggle_details",
                    if details_open {
                        "收起详情"
                    } else {
                        "查看详情"
                    },
                    false,
                    Some(ExportUiMessage::TogglePublishDetails),
                ),
                btn_el(
                    theme,
                    "pub_open_dir",
                    "打开输出目录",
                    false,
                    Some(ExportUiMessage::OpenPath(out_dir)),
                ),
                btn_el(
                    theme,
                    "pub_open_index",
                    "打开清单文件",
                    false,
                    Some(ExportUiMessage::OpenPath(outcome.index_path.clone())),
                ),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center);

            let mut col = column![summary_row].spacing(8);
            if details_open {
                let mut lines: Vec<Element<'a, AppMessage>> = Vec::new();
                lines.push(
                    text(format!(
                        ".pid 文件 {} 个（匹配图纸 {}，图框模板 {}，未匹配 {}）",
                        outcome.pid_entries,
                        outcome
                            .pid_entries
                            .saturating_sub(outcome.templates + outcome.unmatched.len()),
                        outcome.templates,
                        outcome.unmatched.len()
                    ))
                    .size(11.5)
                    .font(pid_fonts::mono(theme, font::Weight::Normal))
                    .color(ColorToken::TextSecondary.color(theme))
                    .into(),
                );
                if outcome.sql_server() {
                    lines.push(
                        text("SQL Server 提示：该包为 SQL Server 备份，导出内容未经参考验收。")
                            .size(11.5)
                            .font(pid_fonts::ui(theme, font::Weight::Semibold))
                            .color(ColorToken::Amber.color(theme))
                            .into(),
                    );
                }
                for note in &outcome.unwritten {
                    lines.push(
                        text(format!("未写出 [{}]: {}", note.name, note.notes.join("；")))
                            .size(11.5)
                            .font(pid_fonts::mono(theme, font::Weight::Normal))
                            .color(ColorToken::Danger.color(theme))
                            .into(),
                    );
                }
                for note in &outcome.noted {
                    lines.push(
                        text(format!(
                            "图纸说明 [{}]: {}",
                            note.name,
                            note.notes.join("；")
                        ))
                        .size(11.5)
                        .font(pid_fonts::mono(theme, font::Weight::Normal))
                        .color(ColorToken::Amber.color(theme))
                        .into(),
                    );
                }
                for prob in &outcome.other_problems {
                    lines.push(
                        text(format!("整包问题: {prob}"))
                            .size(11.5)
                            .font(pid_fonts::mono(theme, font::Weight::Normal))
                            .color(ColorToken::Amber.color(theme))
                            .into(),
                    );
                }
                for warn in &outcome.warnings {
                    lines.push(
                        text(format!("提醒: {warn}"))
                            .size(11.5)
                            .font(pid_fonts::mono(theme, font::Weight::Normal))
                            .color(ColorToken::TextSecondary.color(theme))
                            .into(),
                    );
                }
                if let Some(gate) = &outcome.gate {
                    const MAX_GATE_LINES: usize = 120;
                    for line in gate.lines.iter().take(MAX_GATE_LINES) {
                        lines.push(
                            text(format!("比对: {line}"))
                                .size(11)
                                .font(pid_fonts::mono(theme, font::Weight::Normal))
                                .color(ColorToken::TextPrimary.color(theme))
                                .into(),
                        );
                    }
                    if gate.lines.len() > MAX_GATE_LINES {
                        lines.push(
                            text(format!(
                                "已显示前 {MAX_GATE_LINES} 条（共 {} 条），完整比对明细见清单文件。",
                                gate.lines.len()
                            ))
                            .size(11)
                            .font(pid_fonts::ui(theme, font::Weight::Normal))
                            .color(ColorToken::TextMuted.color(theme))
                            .into(),
                        );
                    }
                }
                col = col.push(PosReport::owned(
                    part_id("pub_details"),
                    scrollable(column(lines).spacing(3)).height(180),
                ));
            }

            PosReport::owned(
                part_id("pub_outcome_ok"),
                container(col)
                    .width(Fill)
                    .padding([10, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(bg.color(theme))),
                        border: Border {
                            color: stroke.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
        Err(error) => {
            let row_el = row![
                column![
                    text(format!("publish 文件导出失败 [{}]", error.code()))
                        .size(12.5)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Danger.color(theme)),
                    text(error.to_string())
                        .size(11.5)
                        .font(pid_fonts::ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(3),
                Space::new().width(Fill),
                btn_el(
                    theme,
                    "pub_reconfigure",
                    "重新配置导出",
                    false,
                    publish_disabled_reason(app)
                        .is_none()
                        .then_some(ExportUiMessage::OpenPublishDialog),
                ),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center);

            PosReport::owned(
                part_id("pub_outcome_err"),
                container(row_el)
                    .width(Fill)
                    .padding([10, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::DangerSoft.color(theme))),
                        border: Border {
                            color: ColorToken::Danger.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pdf_outcome_banner<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    result: &'a Result<PdfOutcome, PdfError>,
    theme: PidTheme,
) -> Element<'a, AppMessage> {
    match result {
        Ok(outcome) => {
            let missing_pages = outcome
                .pages
                .iter()
                .filter(|p| !p.missing_symbols.is_empty())
                .count();
            let missing_total: usize = outcome.pages.iter().map(|p| p.missing_symbols.len()).sum();
            let sql = is_sql_server_package(index);
            let has_warnings =
                outcome.failed > 0 || outcome.without_file > 0 || missing_pages > 0 || sql;
            let (bg, stroke, badge_color) = if has_warnings {
                (ColorToken::AmberSoft, ColorToken::Amber, ColorToken::Amber)
            } else {
                (
                    ColorToken::SuccessSoft,
                    ColorToken::Success,
                    ColorToken::Success,
                )
            };
            let title = format!(
                "PDF 导出完成：写出 {} 份（未写出 {}，缺文件 {}）· 用时 {:.1} 秒",
                outcome.written,
                outcome.failed,
                outcome.without_file,
                outcome.elapsed.as_secs_f64()
            );
            let mut sub_parts = vec![format!("清单：{}", outcome.index_path.display())];
            if missing_pages > 0 {
                sub_parts.push(format!("缺符号 {missing_pages} 张图共 {missing_total} 处"));
            }
            if sql {
                sub_parts.push("该包为 SQL Server 备份，导出内容未经参考验收".to_owned());
            }
            let out_dir = outcome
                .index_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| outcome.index_path.clone());
            let details_open = app.pid_shell.backup.export_ui.pdf_details_open;

            let summary_row = row![
                column![
                    text(title)
                        .size(12.5)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(badge_color.color(theme)),
                    text(sub_parts.join(" · "))
                        .size(11.5)
                        .font(pid_fonts::ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(3),
                Space::new().width(Fill),
                btn_el(
                    theme,
                    "pdf_toggle_details",
                    if details_open {
                        "收起详情"
                    } else {
                        "查看详情"
                    },
                    false,
                    Some(ExportUiMessage::TogglePdfDetails),
                ),
                btn_el(
                    theme,
                    "pdf_open_dir",
                    "打开输出目录",
                    false,
                    Some(ExportUiMessage::OpenPath(out_dir)),
                ),
                btn_el(
                    theme,
                    "pdf_open_index",
                    "打开清单文件",
                    false,
                    Some(ExportUiMessage::OpenPath(outcome.index_path.clone())),
                ),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center);

            let mut col = column![summary_row].spacing(8);
            if details_open {
                let mut lines: Vec<Element<'a, AppMessage>> = Vec::new();
                if sql {
                    lines.push(
                        text("SQL Server 提示：该包为 SQL Server 备份，导出内容未经参考验收。")
                            .size(11.5)
                            .font(pid_fonts::ui(theme, font::Weight::Semibold))
                            .color(ColorToken::Amber.color(theme))
                            .into(),
                    );
                }
                for page in &outcome.pages {
                    if let Some(err) = &page.error {
                        lines.push(
                            text(format!("未写出 PDF [{}]: {err}", page.name))
                                .size(11.5)
                                .font(pid_fonts::mono(theme, font::Weight::Normal))
                                .color(ColorToken::Danger.color(theme))
                                .into(),
                        );
                    }
                    if !page.missing_symbols.is_empty() {
                        lines.push(
                            text(format!(
                                "缺符号 [{}]: {}",
                                page.name,
                                page.missing_symbols.join(", ")
                            ))
                            .size(11.5)
                            .font(pid_fonts::mono(theme, font::Weight::Normal))
                            .color(ColorToken::Amber.color(theme))
                            .into(),
                        );
                    }
                }
                if lines.is_empty() {
                    lines.push(
                        text("全部图纸均已正常写出 PDF，无缺符号与失败项。")
                            .size(11.5)
                            .font(pid_fonts::ui(theme, font::Weight::Normal))
                            .color(ColorToken::TextSecondary.color(theme))
                            .into(),
                    );
                }
                col = col.push(PosReport::owned(
                    part_id("pdf_details"),
                    scrollable(column(lines).spacing(3)).height(180),
                ));
            }

            PosReport::owned(
                part_id("pdf_outcome_ok"),
                container(col)
                    .width(Fill)
                    .padding([10, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(bg.color(theme))),
                        border: Border {
                            color: stroke.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
        Err(error) => {
            let row_el = row![
                column![
                    text(format!("PDF 导出失败 [{}]", error.code()))
                        .size(12.5)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Danger.color(theme)),
                    text(error.to_string())
                        .size(11.5)
                        .font(pid_fonts::ui(theme, font::Weight::Normal))
                        .color(ColorToken::TextPrimary.color(theme)),
                ]
                .spacing(3),
                Space::new().width(Fill),
                btn_el(
                    theme,
                    "pdf_reconfigure",
                    "重新配置导出",
                    false,
                    pdf_disabled_reason(app)
                        .is_none()
                        .then_some(ExportUiMessage::OpenPdfDialog),
                ),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center);

            PosReport::owned(
                part_id("pdf_outcome_err"),
                container(row_el)
                    .width(Fill)
                    .padding([10, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::DangerSoft.color(theme))),
                        border: Border {
                            color: ColorToken::Danger.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
    }
}

/// Running banners + latest outcome summary bars rendered below the package header (`U1`, `U3`).
pub(crate) fn status_banners<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    theme: PidTheme,
) -> Option<Element<'a, AppMessage>> {
    #[allow(unused_mut)]
    let mut bars: Vec<Element<'a, AppMessage>> = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = Instant::now();
        if let Some((req, elapsed)) = app.pid_backup_export.running_status(now) {
            let line = format!(
                "正在导出 publish 文件到 {}（工厂名 {}）· 已用 {} 秒",
                req.out_dir.display(),
                req.plant,
                elapsed.as_secs()
            );
            bars.push(
                PosReport::owned(
                    part_id("running_publish"),
                    container(
                        text(line)
                            .size(12)
                            .font(pid_fonts::ui(theme, font::Weight::Semibold))
                            .color(ColorToken::Accent.color(theme)),
                    )
                    .width(Fill)
                    .padding([9, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::AccentSoft.color(theme))),
                        border: Border {
                            color: ColorToken::Accent.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
                )
                .into(),
            );
        } else if let Some(last) = app.pid_backup_export.last_outcome_for(&index.backup_root) {
            bars.push(publish_outcome_banner(app, last, theme));
        }

        if let Some((out_dir, done, total, elapsed)) = app.pid_backup_pdf.running_status(now) {
            let line = format!(
                "正在导出 PDF 到 {} · 第 {} / {} 张 · 已用 {} 秒",
                out_dir.display(),
                done,
                total,
                elapsed.as_secs()
            );
            let ratio = if total == 0 {
                0.0
            } else {
                (done as f32 / total as f32).clamp(0.0, 1.0)
            };
            let bar_w = 220.0;
            let fill_w = (bar_w * ratio).max(2.0);
            let progress_track = container(
                container(Space::new())
                    .width(Length::Fixed(fill_w))
                    .height(Length::Fixed(6.0))
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::Accent.color(theme))),
                        border: Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 3.0.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .width(Length::Fixed(bar_w))
            .height(Length::Fixed(6.0))
            .style(move |_| container::Style {
                background: Some(Background::Color(ColorToken::BgRaised.color(theme))),
                border: Border {
                    color: ColorToken::Border.color(theme),
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..container::Style::default()
            });

            bars.push(
                PosReport::owned(
                    part_id("running_pdf"),
                    container(
                        row![
                            text(line)
                                .size(12)
                                .font(pid_fonts::ui(theme, font::Weight::Semibold))
                                .color(ColorToken::Accent.color(theme)),
                            Space::new().width(Fill),
                            progress_track,
                        ]
                        .spacing(12)
                        .align_y(alignment::Vertical::Center),
                    )
                    .width(Fill)
                    .padding([9, 20])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::AccentSoft.color(theme))),
                        border: Border {
                            color: ColorToken::Accent.color(theme),
                            width: 1.0,
                            radius: 0.0.into(),
                        },
                        ..container::Style::default()
                    }),
                )
                .into(),
            );
        } else if let Some(last) = app.pid_backup_pdf.last_outcome_for(&index.backup_root) {
            bars.push(pdf_outcome_banner(app, index, last, theme));
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, index, theme);

    (!bars.is_empty()).then(|| column(bars).width(Fill).into())
}

/// Modal dialog overlay rendered on top of the `backup` screen when open (`UI-7`).
pub(crate) fn dialog_overlay<'a>(
    app: &'a OpenCADStudio,
    index: &'a BackupIndex,
    theme: PidTheme,
) -> Option<Element<'a, AppMessage>> {
    let dialog = app.pid_shell.backup.export_ui.dialog.as_ref()?;
    let radius = NumberToken::RadiusMd.value(theme);
    let sql = is_sql_server_package(index);
    let with_files = index
        .drawings
        .iter()
        .filter(|d| d.pid_path.is_some())
        .count();
    let missing_files = index.drawings.len().saturating_sub(with_files);

    let card: Element<'a, AppMessage> = match dialog {
        ExportDialog::Publish(state) => {
            let can_submit = state.validation.is_ok();
            let mut body = column![
                text("导出 publish 文件")
                    .size(16)
                    .font(pid_fonts::ui(theme, font::Weight::Semibold))
                    .color(ColorToken::TextPrimary.color(theme)),
                text(format!("当前备份包共 {with_files} 张带文件图纸。"))
                    .size(12)
                    .font(pid_fonts::ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextSecondary.color(theme)),
                text("输出目录（必填，须为空目录或新目录，且不在备份包/还原目录内）")
                    .size(11.5)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(ColorToken::TextSecondary.color(theme)),
                row![
                    dialog_input(
                        theme,
                        "pub_out_input",
                        "例如 D:\\deliver\\plant-pair",
                        &state.out_dir,
                        ExportUiMessage::PublishOutChanged,
                    ),
                    btn_el(
                        theme,
                        "pub_browse_out",
                        "浏览…",
                        false,
                        Some(ExportUiMessage::BrowseFolder("pub_out")),
                    ),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
                text("工厂名（必填，默认 P01）")
                    .size(11.5)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(ColorToken::TextSecondary.color(theme)),
                dialog_input(
                    theme,
                    "pub_plant_input",
                    "P01",
                    &state.plant,
                    ExportUiMessage::PublishPlantChanged,
                ),
                text("参考目录（可选：与厂商参考 _Data.xml 比对，支持含子目录的验证集）")
                    .size(11.5)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(ColorToken::TextSecondary.color(theme)),
                row![
                    dialog_input(
                        theme,
                        "pub_ref_input",
                        "可选参考目录路径",
                        &state.reference_dir,
                        ExportUiMessage::PublishRefChanged,
                    ),
                    btn_el(
                        theme,
                        "pub_browse_ref",
                        "浏览…",
                        false,
                        Some(ExportUiMessage::BrowseFolder("pub_ref")),
                    ),
                    btn_el(
                        theme,
                        "pub_clear_ref",
                        "清空",
                        false,
                        Some(ExportUiMessage::PublishClearRef),
                    ),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
            ]
            .spacing(8);

            if sql {
                body = body.push(PosReport::owned(
                    part_id("sql_warning"),
                    text("SQL Server 包的导出没有验收过")
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Amber.color(theme)),
                ));
            }
            body = body.push(
                text("如需将 PDF 导出到同一交付目录配成对，请先导出 publish 文件，完成后再将 PDF 导出到该目录。")
                    .size(11.5)
                    .font(pid_fonts::ui(theme, font::Weight::Normal))
                    .color(ColorToken::TextMuted.color(theme)),
            );
            if let Err(err_msg) = &state.validation {
                body = body.push(PosReport::owned(
                    part_id("pub_error"),
                    text(err_msg.as_str())
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Medium))
                        .color(ColorToken::Danger.color(theme)),
                ));
            }

            body = body.push(
                row![
                    Space::new().width(Fill),
                    btn_el(
                        theme,
                        "pub_cancel",
                        "取消",
                        false,
                        Some(ExportUiMessage::CloseDialog),
                    ),
                    btn_el(
                        theme,
                        "pub_submit",
                        "开始导出",
                        true,
                        can_submit.then_some(ExportUiMessage::SubmitPublish),
                    ),
                ]
                .spacing(10),
            );

            PosReport::owned(
                part_id("publish_dialog"),
                container(body)
                    .width(580)
                    .padding([20, 24])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
                        border: Border {
                            color: ColorToken::BorderStrong.color(theme),
                            width: 1.0,
                            radius: radius.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
        ExportDialog::Pdf(state) => {
            let last_pub = resolved_last_publish_dir(app);
            let can_submit = state.validation.is_ok();
            let pair_ready = state.pair_ready;

            let mut body = column![
                text("导出 PDF")
                    .size(16)
                    .font(pid_fonts::ui(theme, font::Weight::Semibold))
                    .color(ColorToken::TextPrimary.color(theme)),
                text(format!(
                    "共 {} 张图纸（其中 {} 张缺文件）。",
                    index.drawings.len(),
                    missing_files
                ))
                .size(12)
                .font(pid_fonts::ui(theme, font::Weight::Normal))
                .color(ColorToken::TextSecondary.color(theme)),
                text("输出目录（必填，不可落在包或还原目录内，不可含同名 .pdf 或 _PdfIndex.json）")
                    .size(11.5)
                    .font(pid_fonts::ui(theme, font::Weight::Medium))
                    .color(ColorToken::TextSecondary.color(theme)),
                row![
                    dialog_input(
                        theme,
                        "pdf_out_input",
                        "例如 D:\\deliver\\plant-pair",
                        &state.out_dir,
                        ExportUiMessage::PdfOutChanged,
                    ),
                    btn_el(
                        theme,
                        "pdf_browse_out",
                        "浏览…",
                        false,
                        Some(ExportUiMessage::BrowseFolder("pdf_out")),
                    ),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center),
            ]
            .spacing(8);

            if last_pub.is_some() {
                body = body.push(btn_el(
                    theme,
                    "pdf_use_last_publish",
                    "使用上次 publish 输出目录",
                    false,
                    Some(ExportUiMessage::PdfUseLastPublishDir),
                ));
            }
            if sql {
                body = body.push(PosReport::owned(
                    part_id("sql_warning"),
                    text("SQL Server 包的导出没有验收过")
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Amber.color(theme)),
                ));
            }
            if pair_ready {
                body = body.push(PosReport::owned(
                    part_id("pdf_pair_ready"),
                    text("可与该目录中的 publish 文件配对输出")
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Semibold))
                        .color(ColorToken::Success.color(theme)),
                ));
            }
            if let Err(err_msg) = &state.validation {
                body = body.push(PosReport::owned(
                    part_id("pdf_error"),
                    text(err_msg.as_str())
                        .size(12)
                        .font(pid_fonts::ui(theme, font::Weight::Medium))
                        .color(ColorToken::Danger.color(theme)),
                ));
            }

            body = body.push(
                row![
                    Space::new().width(Fill),
                    btn_el(
                        theme,
                        "pdf_cancel",
                        "取消",
                        false,
                        Some(ExportUiMessage::CloseDialog),
                    ),
                    btn_el(
                        theme,
                        "pdf_submit",
                        "开始导出",
                        true,
                        can_submit.then_some(ExportUiMessage::SubmitPdf),
                    ),
                ]
                .spacing(10),
            );

            PosReport::owned(
                part_id("pdf_dialog"),
                container(body)
                    .width(580)
                    .padding([20, 24])
                    .style(move |_| container::Style {
                        background: Some(Background::Color(ColorToken::BgPanel.color(theme))),
                        border: Border {
                            color: ColorToken::BorderStrong.color(theme),
                            width: 1.0,
                            radius: radius.into(),
                        },
                        ..container::Style::default()
                    }),
            )
            .into()
        }
    };

    Some(opaque(
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(ColorToken::Overlay.color(theme))),
                ..container::Style::default()
            }),
    ))
}

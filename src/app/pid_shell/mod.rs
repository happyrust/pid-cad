//! The P&ID module: the screens of the Pencil design
//! (`docs/pid-platform/design/pid-platform.pen`), the one that is showing,
//! and the root view the main window draws while the module is on.
//!
//! A launch starts here unless it says `--classic`
//! (`OpenCADStudio::open_first_screen`): the module is on,
//! [`STARTUP_SCREEN`] is showing and, that screen being the editor, a blank
//! drawing is open to draw on. `--classic` leaves the module off, which is
//! the general CAD interface and its Start page as they were.
//!
//! # The shell
//!
//! ```text
//! pub const STARTUP_SCREEN: PidScreen     where a launch lands; change it here
//! pub enum PidScreen                      the 14 screens of the design:
//!     ALL, id(), title(), group(), from_id(), is_implemented(), needs_drawing()
//! pub enum NavGroup                       设计 / 配置 / 校验交付 / 平台: title()
//! pub fn nav() -> impl Iterator<Item = PidScreen>
//!                                         what the navigation lists: the
//!                                         implemented screens, in order
//! pub struct PidShell                     current() -> Option<PidScreen>,
//!                                         show(screen) -> bool, and one
//!                                         `State` per screen
//! pub enum Message                        Show(PidScreen), and one variant
//!                                         per screen for its own messages
//! pub fn bounds_id(screen) -> String      "PID_SHELL:<id>", see below
//! fn view(app) -> Option<Element>         the root view; `view_main` draws it
//! OpenCADStudio::open_first_screen(&mut self, &cli::GuiConfig) -> Task
//! OpenCADStudio::on_pid_shell(&mut self, Message) -> Task
//! PIDSCREEN [<id>]                        the switch as a command
//! ```
//!
//! # Layout
//!
//! ```text
//! src/app/pid_shell/
//!   mod.rs                this file: PidScreen, NavGroup, PidShell, Message,
//!                         STARTUP_SCREEN, nav(), view(), PIDSCREEN
//!   chrome.rs             the shell around a screen: title bar, navigation,
//!                         status bar; SyncStatus
//!   canvas.rs             the colour a drawing is cleared to and the grid
//!                         over it, inside the module
//!   rebase.rs             a frame of its own for the general drawing
//!                         interface inside the content area
//!   workbench.rs          01 工作台         workbench       not implemented
//!   drawings.rs           02 图纸管理       drawings        not implemented
//!   editor.rs             03 P&ID 编辑器    editor          implemented
//!   palette.rs            04 符号调色板     palette         not implemented
//!   properties.rs         05 属性面板       properties      not implemented
//!   typelib.rs            06 业务类型库     typelib         not implemented
//!   symbols.rs            07 标准图例定制   symbols         not implemented
//!   symbol_editor.rs      08 符号编辑器     symbol-editor   not implemented
//!   tag_rules.rs          09 位号规则       tag-rules       not implemented
//!   color_rules.rs        10 颜色规则       color-rules     not implemented
//!   masterdata.rs         11 主数据管理     masterdata      not implemented
//!   validation.rs         12 设计规则检查   validation      not implemented
//!   bom.rs                13 材料清单       bom             not implemented
//!   permissions.rs        14 权限管理       permissions     not implemented
//!   pid_shell_startup.rs  tests: the first screen, switching, the navigation,
//!                         the screen list against the design
//!   pid_shell_chrome.rs   tests: the bars and their sizes, the session's
//!                         state, what can be pressed, and the drawing in
//!                         its frame (pointer, snap, popups)
//!   pid_editor_screen.rs  tests: the editor's tools, palette, tabs and
//!                         inspector against the commands and the records
//!   pid_backup_in_shell.rs
//!                         tests: PIDBACKUPOPEN in the module: the backup
//!                         panel beside the drawing, a drawing opened from
//!                         it, a source that is no backup
//! ```
//!
//! # What a screen module is
//!
//! Every screen module has the same items, and the shell reaches a screen
//! through them and nothing else:
//!
//! ```text
//! pub struct State { .. }        what the screen remembers between frames;
//!                                it lives in `app.pid_shell.<screen>`
//! pub enum Message { .. }        what the screen's widgets send, wrapped as
//!                                app::Message::PidShell(Message::<Screen>(..))
//! pub(crate) const VIEW: Option<ViewFn>;
//! pub(crate) fn update(app: &mut OpenCADStudio, message: Message) -> Task<app::Message>;
//! pub(crate) fn view(app: &OpenCADStudio) -> Element<'_, app::Message>;
//! ```
//!
//! `view` exists only in a screen that is implemented, and `VIEW` is
//! `Some(view)` there. A screen that is not implemented has an empty `State`,
//! a `Message` with no variant and `VIEW = None`: it is not in [`nav`],
//! [`PidShell::show`] refuses it, and nothing is drawn for it — there is no
//! placeholder page. Implementing one is done in its own file: write `view`,
//! set `VIEW` to `Some(view)`, give `State` and `Message` what the screen
//! needs. Nothing in this file changes for that; a task that does need this
//! file to change is changing the contract and says so on its task.
//!
//! # From startup to another screen
//!
//! ```text
//! let mut app = OpenCADStudio::new();
//! // What `boot` does with the command line (`cli::gui_config()`): the
//! // module is on, the startup screen is showing and its blank drawing is
//! // open. The task is what is left for iced to run: the files to open,
//! // when the launch named any.
//! let first = app.open_first_screen(&cli::GuiConfig::default());
//! assert_eq!(app.pid_shell.current(), Some(STARTUP_SCREEN));
//!
//! // A navigation entry, a button, a shortcut:
//! let _ = app.update(app::Message::PidShell(Message::Show(PidScreen::Editor)));
//! // The same from the command line, a script (`--script`) or the automation
//! // channel's `run`; bare `PIDSCREEN` says which screen is showing:
//! let _ = app.dispatch_command("PIDSCREEN editor");
//!
//! // What the main window draws: the showing screen's `view`, or the
//! // general interface while the module is off.
//! let window = app.view_main();
//! ```
//!
//! `pid_shell_startup::the_call_example_of_the_module_docs_runs` makes these
//! calls.
//!
//! # What the shell draws
//!
//! [`view`] is the design's `component/App Shell` ([`chrome`]): the title
//! bar, the navigation — the implemented screens under the design's groups,
//! and the state of the collaboration session at its foot — and the status
//! bar, around the showing screen. A screen's `view` is given the content
//! area and nothing else.
//!
//! The editor screen ([`editor`]) draws the design's toolbar, palette and
//! inspector round the drawing. The drawing itself is the general
//! interface's drawing area — viewport, command line, dialogs — which
//! places what it places by window coordinates: it is put in a frame of its
//! own ([`rebase`]) and is told the frame's size as its window size
//! (`PidShell::editor_frame`), so that what it places lands where it
//! should. While it has a dialog up, the rest of the window is dimmed and
//! takes no press (`chrome::shielded`).
//!
//! The showing screen's rectangle is reported as `PID_SHELL:<id>`
//! ([`bounds_id`]) and the shell's parts as `PID_CHROME:<part>`
//! (`chrome::CHROME_BOUNDS_PREFIX`), which is how the automation channel's
//! `ui_bounds` — and a test — tells that the module is on, which screen is
//! drawn and where.

pub mod canvas;
pub mod chrome;
pub(crate) mod rebase;

pub mod backup;
pub mod backup_export_ui;
pub mod bom;
pub mod color_rules;
pub mod drawings;
pub mod editor;
pub mod masterdata;
pub mod palette;
pub mod permissions;
pub mod properties;
pub mod symbol_editor;
pub mod symbols;
pub mod tag_rules;
pub mod typelib;
pub mod validation;
pub mod workbench;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod pid_backup_export_ui;
#[cfg(test)]
mod pid_backup_in_shell;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod pid_backup_screen;
#[cfg(test)]
mod pid_editor_screen;
#[cfg(test)]
mod pid_editor_screen_canvas;
#[cfg(test)]
mod pid_fonts_loading;
#[cfg(test)]
mod pid_shell_chrome;
#[cfg(test)]
mod pid_shell_startup;

use iced::{Element, Task};

use super::{Message as AppMessage, OpenCADStudio};
use crate::ui::style::pid_fonts;

/// Where a launch lands. The one place to change it.
///
/// A screen that [needs a drawing](PidScreen::needs_drawing) gets a blank one
/// when the launch names no file; any other screen starts with none.
pub const STARTUP_SCREEN: PidScreen = PidScreen::Editor;

/// The function that draws a screen: a screen module's `view`.
pub(crate) type ViewFn = for<'a> fn(&'a OpenCADStudio) -> Element<'a, AppMessage>;

/// The screens of the design, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PidScreen {
    /// 01 工作台.
    Workbench,
    /// 02 图纸管理.
    Drawings,
    /// 03 P&ID 编辑器.
    Editor,
    /// 备份数据包.
    Backup,
    /// 04 符号调色板.
    Palette,
    /// 05 属性面板.
    Properties,
    /// 06 业务类型库.
    TypeLib,
    /// 07 标准图例定制.
    Symbols,
    /// 08 符号编辑器.
    SymbolEditor,
    /// 09 位号规则.
    TagRules,
    /// 10 颜色规则.
    ColorRules,
    /// 11 主数据管理.
    MasterData,
    /// 12 设计规则检查.
    Validation,
    /// 13 材料清单.
    Bom,
    /// 14 权限管理.
    Permissions,
}

/// The groups the design's navigation lists the screens under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavGroup {
    /// 设计.
    Design,
    /// 配置.
    Config,
    /// 校验交付.
    Delivery,
    /// 平台.
    Platform,
}

impl NavGroup {
    /// The group's label in the navigation.
    pub const fn title(self) -> &'static str {
        match self {
            NavGroup::Design => "设计",
            NavGroup::Config => "配置",
            NavGroup::Delivery => "校验交付",
            NavGroup::Platform => "平台",
        }
    }
}

impl PidScreen {
    /// Every screen of the design, in its order.
    pub const ALL: [PidScreen; 15] = [
        PidScreen::Workbench,
        PidScreen::Drawings,
        PidScreen::Editor,
        PidScreen::Backup,
        PidScreen::Palette,
        PidScreen::Properties,
        PidScreen::TypeLib,
        PidScreen::Symbols,
        PidScreen::SymbolEditor,
        PidScreen::TagRules,
        PidScreen::ColorRules,
        PidScreen::MasterData,
        PidScreen::Validation,
        PidScreen::Bom,
        PidScreen::Permissions,
    ];

    /// The screen's id in the screen catalog
    /// (`docs/pid-platform/prototype/specs/screen-catalog.json`) and in the
    /// design's navigation; what `PIDSCREEN` takes.
    pub const fn id(self) -> &'static str {
        match self {
            PidScreen::Workbench => "workbench",
            PidScreen::Drawings => "drawings",
            PidScreen::Editor => "editor",
            PidScreen::Backup => "backup",
            PidScreen::Palette => "palette",
            PidScreen::Properties => "properties",
            PidScreen::TypeLib => "typelib",
            PidScreen::Symbols => "symbols",
            PidScreen::SymbolEditor => "symbol-editor",
            PidScreen::TagRules => "tag-rules",
            PidScreen::ColorRules => "color-rules",
            PidScreen::MasterData => "masterdata",
            PidScreen::Validation => "validation",
            PidScreen::Bom => "bom",
            PidScreen::Permissions => "permissions",
        }
    }

    /// The screen's name, as the design's navigation labels it.
    pub const fn title(self) -> &'static str {
        match self {
            PidScreen::Workbench => "工作台",
            PidScreen::Drawings => "图纸管理",
            PidScreen::Editor => "P&ID 编辑器",
            PidScreen::Backup => "备份数据包",
            PidScreen::Palette => "符号调色板",
            PidScreen::Properties => "属性面板",
            PidScreen::TypeLib => "业务类型库",
            PidScreen::Symbols => "标准图例定制",
            PidScreen::SymbolEditor => "符号编辑器",
            PidScreen::TagRules => "位号规则",
            PidScreen::ColorRules => "颜色规则",
            PidScreen::MasterData => "主数据管理",
            PidScreen::Validation => "设计规则检查",
            PidScreen::Bom => "材料清单",
            PidScreen::Permissions => "权限管理",
        }
    }

    /// The group the design's navigation lists the screen under.
    pub const fn group(self) -> NavGroup {
        match self {
            PidScreen::Workbench
            | PidScreen::Drawings
            | PidScreen::Editor
            | PidScreen::Backup
            | PidScreen::Palette
            | PidScreen::Properties => NavGroup::Design,
            PidScreen::TypeLib
            | PidScreen::Symbols
            | PidScreen::SymbolEditor
            | PidScreen::TagRules
            | PidScreen::ColorRules
            | PidScreen::MasterData => NavGroup::Config,
            PidScreen::Validation | PidScreen::Bom => NavGroup::Delivery,
            PidScreen::Permissions => NavGroup::Platform,
        }
    }

    /// The screen with this id, whatever its letter case.
    pub fn from_id(id: &str) -> Option<PidScreen> {
        PidScreen::ALL
            .into_iter()
            .find(|screen| screen.id().eq_ignore_ascii_case(id))
    }

    /// The function that draws the screen, from the screen's module; `None`
    /// while the screen is not implemented.
    pub(crate) fn view_fn(self) -> Option<ViewFn> {
        match self {
            PidScreen::Workbench => workbench::VIEW,
            PidScreen::Drawings => drawings::VIEW,
            PidScreen::Editor => editor::VIEW,
            PidScreen::Backup => backup::VIEW,
            PidScreen::Palette => palette::VIEW,
            PidScreen::Properties => properties::VIEW,
            PidScreen::TypeLib => typelib::VIEW,
            PidScreen::Symbols => symbols::VIEW,
            PidScreen::SymbolEditor => symbol_editor::VIEW,
            PidScreen::TagRules => tag_rules::VIEW,
            PidScreen::ColorRules => color_rules::VIEW,
            PidScreen::MasterData => masterdata::VIEW,
            PidScreen::Validation => validation::VIEW,
            PidScreen::Bom => bom::VIEW,
            PidScreen::Permissions => permissions::VIEW,
        }
    }

    /// Whether the screen can be shown: it has a `view`.
    pub fn is_implemented(self) -> bool {
        self.view_fn().is_some()
    }

    /// Whether the screen works on the drawing in front, so that a launch
    /// landing on it with no file to open starts a blank drawing.
    pub const fn needs_drawing(self) -> bool {
        matches!(self, PidScreen::Editor)
    }
}

/// The screens the navigation lists, in the design's order: every
/// implemented screen and no other.
pub fn nav() -> impl Iterator<Item = PidScreen> {
    PidScreen::ALL
        .into_iter()
        .filter(|screen| screen.is_implemented())
}

/// The state of the P&ID module: which screen is showing, and each screen's
/// own state.
#[derive(Default)]
pub struct PidShell {
    /// The showing screen; `None` while the module is off and the window is
    /// the general CAD interface. Only [`PidShell::show`] sets it, so it is
    /// never a screen without a `view`.
    current: Option<PidScreen>,
    /// The theme the shell is drawn in: which value of each design variable
    /// it takes.
    pub theme: crate::ui::style::pid_tokens::PidTheme,
    /// The window's size, as the last resize said; `None` before the first.
    window: Option<(f32, f32)>,
    /// 01 工作台.
    pub workbench: workbench::State,
    /// 02 图纸管理.
    pub drawings: drawings::State,
    /// 03 P&ID 编辑器.
    pub editor: editor::State,
    /// 备份数据包.
    pub backup: backup::State,
    /// 04 符号调色板.
    pub palette: palette::State,
    /// 05 属性面板.
    pub properties: properties::State,
    /// 06 业务类型库.
    pub typelib: typelib::State,
    /// 07 标准图例定制.
    pub symbols: symbols::State,
    /// 08 符号编辑器.
    pub symbol_editor: symbol_editor::State,
    /// 09 位号规则.
    pub tag_rules: tag_rules::State,
    /// 10 颜色规则.
    pub color_rules: color_rules::State,
    /// 11 主数据管理.
    pub masterdata: masterdata::State,
    /// 12 设计规则检查.
    pub validation: validation::State,
    /// 13 材料清单.
    pub bom: bom::State,
    /// 14 权限管理.
    pub permissions: permissions::State,
}

impl PidShell {
    /// The showing screen; `None` while the module is off.
    pub fn current(&self) -> Option<PidScreen> {
        self.current
    }

    /// Show `screen`, turning the module on if it was off. A screen that is
    /// not implemented is refused: `false`, and what was showing still is.
    ///
    /// The module's typefaces go into the interface's font database here,
    /// the first time a screen is shown ([`pid_fonts::ensure_loaded`]): a
    /// session that never shows one never loads them.
    pub fn show(&mut self, screen: PidScreen) -> bool {
        if !screen.is_implemented() {
            return false;
        }
        pid_fonts::ensure_loaded();
        self.current = Some(screen);
        true
    }

    /// Take note of the window's size, and say what size the general drawing
    /// interface is to measure by: the window's while the module is off,
    /// and while it is on the size of the frame the editor screen lays the
    /// drawing out in ([`rebase::Rebase`]) — what the shell's bars and the
    /// editor's toolbar, palette, tabs and inspector leave of the window.
    pub fn editor_frame(&mut self, window: (f32, f32)) -> (f32, f32) {
        self.window = Some(window);
        if self.current.is_none() {
            return window;
        }
        (
            (window.0 - chrome::NAV_WIDTH - editor::PALETTE_WIDTH - editor::INSPECTOR_WIDTH)
                .max(1.0),
            (window.1
                - chrome::TITLE_BAR_HEIGHT
                - chrome::STATUS_BAR_HEIGHT
                - editor::TOOLBAR_HEIGHT
                - editor::TABS_HEIGHT)
                .max(1.0),
        )
    }

    /// The window's size, once a resize has said it.
    pub fn window(&self) -> Option<(f32, f32)> {
        self.window
    }
}

/// A message of the P&ID module.
#[derive(Debug, Clone)]
pub enum Message {
    /// Show a screen: what a navigation entry sends.
    Show(PidScreen),
    /// 01 工作台.
    Workbench(workbench::Message),
    /// 02 图纸管理.
    Drawings(drawings::Message),
    /// 03 P&ID 编辑器.
    Editor(editor::Message),
    /// 备份数据包.
    Backup(backup::Message),
    /// 04 符号调色板.
    Palette(palette::Message),
    /// 05 属性面板.
    Properties(properties::Message),
    /// 06 业务类型库.
    TypeLib(typelib::Message),
    /// 07 标准图例定制.
    Symbols(symbols::Message),
    /// 08 符号编辑器.
    SymbolEditor(symbol_editor::Message),
    /// 09 位号规则.
    TagRules(tag_rules::Message),
    /// 10 颜色规则.
    ColorRules(color_rules::Message),
    /// 11 主数据管理.
    MasterData(masterdata::Message),
    /// 12 设计规则检查.
    Validation(validation::Message),
    /// 13 材料清单.
    Bom(bom::Message),
    /// 14 权限管理.
    Permissions(permissions::Message),
}

/// What the showing screen's rectangle is reported under, followed by the
/// screen's id.
pub const BOUNDS_PREFIX: &str = "PID_SHELL:";

/// The id `screen`'s rectangle is reported under while it is drawn
/// (`ui::wrap_bar::recorded_bounds`, the automation channel's `ui_bounds`).
pub fn bounds_id(screen: PidScreen) -> String {
    format!("{BOUNDS_PREFIX}{}", screen.id())
}

/// The root view of the P&ID module: the shell ([`chrome`]) around the
/// showing screen, which is drawn by its module. `None` while the module is
/// off.
pub(in crate::app) fn view(app: &OpenCADStudio) -> Option<Element<'_, AppMessage>> {
    let screen = app.pid_shell.current()?;
    let draw = screen.view_fn()?;
    let content = crate::ui::wrap_bar::PosReport::owned(bounds_id(screen), draw(app));
    Some(chrome::frame(app, screen, content.into()))
}

impl OpenCADStudio {
    /// What a launch shows first, from its command line, and the task that
    /// opens it.
    ///
    /// The P&ID module on [`STARTUP_SCREEN`], unless the launch said
    /// `--classic`. Files named on the command line are opened either way.
    /// With none, a blank drawing is opened when the startup screen needs
    /// one or the launch asked for one (`--new`, a `--script` to run in it);
    /// otherwise nothing is, and the general interface is on its Start page.
    ///
    /// `boot` calls this before it queues the startup prompts, and the
    /// module's blank drawing is opened here and now rather than by a
    /// message: a first launch has a prompt up before any message runs, and
    /// an open dialog swallows `TabNew` as it swallows Ctrl+N.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn open_first_screen(
        &mut self,
        cfg: &crate::cli::GuiConfig,
    ) -> Task<AppMessage> {
        let on = !cfg.classic && self.pid_shell.show(STARTUP_SCREEN);
        if on {
            self.use_pid_canvas();
        }
        if !cfg.files.is_empty() {
            // Through `OpenExternal`, the door a second launch hands files
            // to: it checks each path, skips one already open, and queues
            // the rest behind the load in flight.
            return Task::batch(
                cfg.files
                    .iter()
                    .cloned()
                    .map(|path| Task::done(AppMessage::OpenExternal(path))),
            );
        }
        if on && STARTUP_SCREEN.needs_drawing() {
            return self.update(AppMessage::TabNew);
        }
        // The general interface, as it was.
        if cfg.new || !cfg.script_lines.is_empty() {
            return Task::done(AppMessage::TabNew);
        }
        Task::none()
    }

    /// Take a message of the P&ID module.
    pub(in crate::app) fn on_pid_shell(&mut self, message: Message) -> Task<AppMessage> {
        match message {
            Message::Show(screen) => {
                if self.show_pid_screen(screen) && screen == PidScreen::Backup {
                    #[cfg(not(target_arch = "wasm32"))]
                    return backup::ensure_selected_drawing_items_loaded(self);
                }
                Task::none()
            }
            Message::Workbench(message) => workbench::update(self, message),
            Message::Drawings(message) => drawings::update(self, message),
            Message::Editor(message) => editor::update(self, message),
            Message::Backup(message) => backup::update(self, message),
            Message::Palette(message) => palette::update(self, message),
            Message::Properties(message) => properties::update(self, message),
            Message::TypeLib(message) => typelib::update(self, message),
            Message::Symbols(message) => symbols::update(self, message),
            Message::SymbolEditor(message) => symbol_editor::update(self, message),
            Message::TagRules(message) => tag_rules::update(self, message),
            Message::ColorRules(message) => color_rules::update(self, message),
            Message::MasterData(message) => masterdata::update(self, message),
            Message::Validation(message) => validation::update(self, message),
            Message::Bom(message) => bom::update(self, message),
            Message::Permissions(message) => permissions::update(self, message),
        }
    }

    /// Show `screen`; when it cannot be shown, say why on the command line
    /// and leave what was showing.
    pub(in crate::app) fn show_pid_screen(&mut self, screen: PidScreen) -> bool {
        if !self.pid_shell.show(screen) {
            self.command_line.push_error(&format!(
                "界面「{}」（{}）还没有实现，不能切换。",
                screen.title(),
                screen.id()
            ));
            return false;
        }
        // The rectangle of the screen that was showing is no longer drawn.
        crate::ui::wrap_bar::forget_bounds(BOUNDS_PREFIX);
        // The module may just have come on over open drawings.
        self.use_pid_canvas();
        // The module may just have come on: the general interface measures
        // by the area the shell leaves it from here on.
        if let Some((width, height)) = self.pid_shell.window() {
            let _ = self.update(AppMessage::WindowResized(width, height));
        }
        true
    }

    /// `PIDSCREEN [<id>]` — the screen switch for the command line, a script
    /// and the automation channel.
    ///
    /// * bare, it says which screen is showing and which can be shown;
    /// * `PIDSCREEN <id>` shows that screen (`editor`, `typelib`, …: the ids
    ///   of the screen catalog), turning the P&ID module on if the window
    ///   was the general interface. A screen that is not implemented is
    ///   refused, and so is a word that names none.
    pub(in crate::app) fn dispatch_pidscreen(
        &mut self,
        cmd: &str,
        _i: usize,
    ) -> Option<Task<AppMessage>> {
        let mut words = cmd.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case("PIDSCREEN") {
            return None;
        }
        let showable = nav().map(PidScreen::id).collect::<Vec<_>>().join("、");
        match (words.next(), words.next()) {
            (None, _) => {
                let showing = match self.pid_shell.current() {
                    Some(screen) => format!("当前界面 {}（{}）", screen.title(), screen.id()),
                    None => "当前是通用界面，没有进入 P&ID 模块".to_owned(),
                };
                self.command_line
                    .push_info(&format!("PIDSCREEN: {showing}。可切换到：{showable}。"));
            }
            (Some(id), None) => match PidScreen::from_id(id) {
                Some(screen) => {
                    if self.show_pid_screen(screen) {
                        self.command_line.push_info(&format!(
                            "PIDSCREEN: 当前界面 {}（{}）。",
                            screen.title(),
                            screen.id()
                        ));
                        #[cfg(not(target_arch = "wasm32"))]
                        if screen == PidScreen::Backup {
                            return Some(backup::ensure_selected_drawing_items_loaded(self));
                        }
                    }
                }
                None => self.command_line.push_error(&format!(
                    "PIDSCREEN: 没有叫 \"{id}\" 的界面。可切换到：{showable}。"
                )),
            },
            (Some(_), Some(_)) => self
                .command_line
                .push_error("PIDSCREEN: 只接受一个界面名，例如 PIDSCREEN editor。"),
        }
        Some(Task::none())
    }
}

//! PIDBACKUPOPEN -- browse a SmartPlant P&ID backup in a docked panel
//! (`io::pid_backup_browse`).
//!
//! ```text
//! PIDBACKUPOPEN                      show the panel; ask for a source when it
//!                                    holds no backup yet
//! PIDBACKUPOPEN <source> [<folder>]  show the panel and index <source> (a
//!                                    backup zip or folder), restoring into
//!                                    <folder> if given
//! PIDBACKUPOPEN Select <a > b > c>   go to a node of the open backup by the
//!                                    labels on the way to it, unfolding the
//!                                    levels above it and selecting it
//! PIDBACKUPOPEN Split <percent>      give the tree that much of the panel's
//!                                    height and the details the rest
//! PIDBACKUPOPEN Press [<n>|<label>]  press a button in the details of the
//!                                    selected node: the n-th from the top
//!                                    (the first when nothing is given), or
//!                                    the one with that label
//! PIDBACKUPOPEN Cache                say where restored backups are kept
//!                                    and what the folder holds
//! PIDBACKUPOPEN Cache=<folder>       keep them in <folder> from now on
//! PIDBACKUPOPEN Cache=Default        ... in the default folder again
//! PIDBACKUPOPEN Cache=Clear          remove every restore but the one the
//!                                    open backup uses
//! ```
//!
//! `Select` and `Press` are the panel without a pointer: a script, or a
//! client on the automation channel, shows any node with the one (the walk
//! and its matching rules are `pid_backup_browser::goto`) and presses its
//! buttons with the other. A label is the stable way to name a button: the
//! numbers shift when a button comes or goes above it.
//!
//! `Cache` is written with `=` so that it is one word: with no backup in
//! the panel the command line takes the words after the verb one at a time.
//!
//! The panel is the browser: its tree, the details of the selected node and
//! the double click that opens a drawing all live in
//! [`crate::ui::window::pid_backup_browser`]. This module is only the
//! application side of it -- it shows the pallet and carries out the
//! [`Effect`] the panel's pure `update` asks for: index a backup off the UI
//! thread, read a drawing's items, open a `.pid` in a tab, ask for a source.
//!
//! With FILEDIA 0 or under a script nothing is shown: the source is asked for
//! on the command line and the drawings are opened from the panel.

use super::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::dock::PanelId;
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::window::pid_backup_browser::{self, Effect, LoadState, PidBackupMsg};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

/// `arg` without the quotes around it, when it has a pair.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn unquoted(arg: &str) -> &str {
    let quoted = arg.len() >= 2
        && ((arg.starts_with('"') && arg.ends_with('"'))
            || (arg.starts_with('\'') && arg.ends_with('\'')));
    if quoted {
        &arg[1..arg.len() - 1]
    } else {
        arg
    }
}

/// What the words after PIDBACKUPOPEN ask for.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
enum Request {
    Zip,
    Folder,
    Restore(PathBuf, Option<PathBuf>),
    /// Go to the node at this path of labels.
    Select(String),
    /// Give the tree this percentage of the panel's height; `None` when what
    /// follows the word is no percentage.
    Split(Option<f32>),
    /// Press a button in the details of the selected node: its number from
    /// one, the first when none is given; `None` when what follows the word
    /// is a number no button has.
    Press(Option<usize>),
    /// Press the button with this label, or the first whose label starts
    /// with it.
    PressNamed(String),
    /// The restore cache: say what it holds, move it, clear it.
    Cache(CacheRequest),
}

/// What the word `Cache` asks about the restore cache.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
enum CacheRequest {
    /// Say where it is and what it holds.
    Report,
    /// Remove every restore but the one the open backup uses.
    Clear,
    /// Keep restores in the default folder again.
    Default,
    /// Keep restores in this folder.
    Folder(PathBuf),
    /// What follows the word is none of these.
    Unknown,
}

/// What follows `Cache` when `rest` starts with that word: nothing, or a
/// value after `=` or a space.
#[cfg(not(target_arch = "wasm32"))]
fn cache_request(rest: &str) -> Option<CacheRequest> {
    let (word, value) = match rest.split_once(|ch: char| ch == '=' || ch.is_whitespace()) {
        Some((word, value)) => (word, value),
        None => (rest, ""),
    };
    if !word.eq_ignore_ascii_case("CACHE") {
        return None;
    }
    // `Cache = D:\x` is `Cache=D:\x` with air in it.
    let value = unquoted(value.trim().trim_start_matches('=').trim());
    Some(if value.is_empty() {
        CacheRequest::Report
    } else if value.eq_ignore_ascii_case("CLEAR") {
        CacheRequest::Clear
    } else if value.eq_ignore_ascii_case("DEFAULT") {
        CacheRequest::Default
    } else if Path::new(value).is_absolute() {
        CacheRequest::Folder(PathBuf::from(value))
    } else {
        CacheRequest::Unknown
    })
}

/// What follows `Press` when `rest` starts with that word: nothing (the
/// first button), the number of a button from one, or the label of one.
#[cfg(not(target_arch = "wasm32"))]
fn press_request(rest: &str) -> Option<Request> {
    let (word, value) = match rest.split_once(char::is_whitespace) {
        Some((word, value)) => (word, value.trim()),
        None => (rest, ""),
    };
    if !word.eq_ignore_ascii_case("PRESS") {
        return None;
    }
    Some(if value.is_empty() {
        Request::Press(Some(1))
    } else if value.bytes().all(|byte| byte.is_ascii_digit()) {
        Request::Press(value.parse::<usize>().ok().filter(|number| *number >= 1))
    } else {
        Request::PressNamed(unquoted(value).to_string())
    })
}

/// What follows `Split` when `rest` starts with that word: the percentage,
/// with or without its sign.
#[cfg(not(target_arch = "wasm32"))]
fn split_percent(rest: &str) -> Option<Option<f32>> {
    let (word, value) = match rest.split_once(char::is_whitespace) {
        Some((word, value)) => (word, value.trim()),
        None => (rest, ""),
    };
    word.eq_ignore_ascii_case("SPLIT").then(|| {
        value
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|percent| (0.0..=100.0).contains(percent))
    })
}

/// What follows `Select` (or `S`) when `rest` starts with that word.
#[cfg(not(target_arch = "wasm32"))]
fn select_path(rest: &str) -> Option<&str> {
    let (word, path) = match rest.split_once(char::is_whitespace) {
        Some((word, path)) => (word, path.trim()),
        None => (rest, ""),
    };
    ["S", "SELECT"]
        .iter()
        .any(|keyword| word.eq_ignore_ascii_case(keyword))
        .then_some(path)
}

/// Split `<source> [<folder>]`. A line that names an existing path whole is
/// the source alone, so an unquoted path with spaces still works; otherwise
/// the words split at whitespace outside quotes.
#[cfg(not(target_arch = "wasm32"))]
fn parse_request(rest: &str, exists: impl Fn(&Path) -> bool) -> Option<Request> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    if ["Z", "ZIP"]
        .iter()
        .any(|word| rest.eq_ignore_ascii_case(word))
    {
        return Some(Request::Zip);
    }
    if ["F", "FOLDER"]
        .iter()
        .any(|word| rest.eq_ignore_ascii_case(word))
    {
        return Some(Request::Folder);
    }
    let whole = unquoted(rest);
    if exists(Path::new(whole)) {
        return Some(Request::Restore(PathBuf::from(whole), None));
    }
    // After the path check: a backup that is really called `Select` wins.
    if let Some(path) = select_path(rest) {
        return Some(Request::Select(path.to_string()));
    }
    if let Some(percent) = split_percent(rest) {
        return Some(Request::Split(percent));
    }
    if let Some(request) = press_request(rest) {
        return Some(request);
    }
    if let Some(request) = cache_request(rest) {
        return Some(Request::Cache(request));
    }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for ch in rest.chars() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => word.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            None => word.push(ch),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let mut words = words.into_iter();
    let source = PathBuf::from(words.next()?);
    let rest: Vec<String> = words.collect();
    let target = (!rest.is_empty()).then(|| PathBuf::from(rest.join(" ")));
    Some(Request::Restore(source, target))
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidbackup(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let rest = cmd.strip_prefix("PIDBACKUPOPEN")?;
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return None;
        }
        Some(self.start_pid_backup(cmd, rest, i))
    }

    #[cfg(target_arch = "wasm32")]
    fn start_pid_backup(&mut self, _cmd: &str, _rest: &str, _i: usize) -> Task<Message> {
        self.command_line.push_error(
            crate::t!(
                "PIDBACKUPOPEN: opening a SmartPlant backup is not available in the web build."
            )
            .as_ref(),
        );
        Task::none()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_pid_backup(&mut self, cmd: &str, rest: &str, i: usize) -> Task<Message> {
        // The start page draws no dock, so a panel opened there would have
        // nowhere to appear. Open a blank drawing first, the path QNEW takes,
        // and the browser docks into the drawing window that replaces it.
        if self.tabs.get(i).is_some_and(|tab| tab.is_start) {
            let _ = self.update(Message::TabNew);
        }
        let i = self.active_tab;
        let on_command_line = self.file_prompts_on_command_line();
        self.show_pid_backup_panel();
        if rest.trim().is_empty() {
            // A backup already in the panel is what the user asked to see.
            if matches!(
                self.pid_backup_browser.load,
                LoadState::Ready(_) | LoadState::Loading(_)
            ) {
                return Task::none();
            }
            // Bare verb: ask. Enter on the prompt comes back with a trailing
            // space and takes the default.
            if cmd.trim_end() == cmd {
                use crate::command::FreeTextValuePromptCommand;
                let c = if on_command_line {
                    FreeTextValuePromptCommand::new(
                        "PIDBACKUPOPEN",
                        "PIDBACKUPOPEN  Backup zip file or folder:",
                    )
                } else {
                    FreeTextValuePromptCommand::new(
                        "PIDBACKUPOPEN",
                        "PIDBACKUPOPEN  Backup source [Zip/Folder] <Zip>:",
                    )
                };
                self.command_line.push_info(&c.prompt());
                self.tabs[i].active_cmd = Some(Box::new(c));
                return self.finish_dispatch(cmd);
            }
            if on_command_line {
                return Task::none();
            }
            return self.on_pid_backup(PidBackupMsg::Open);
        }
        match parse_request(rest, Path::exists) {
            Some(Request::Restore(source, target)) => self.load_pid_backup(source, target),
            Some(Request::Select(path)) => self.on_pid_backup(PidBackupMsg::Goto(path)),
            Some(Request::Split(Some(percent))) => {
                let task = self.on_pid_backup(PidBackupMsg::SetSplit(percent / 100.0));
                let share = self.pid_backup_browser.split().tree_share();
                self.command_line.push_output(
                    crate::tf!(
                        "PIDBACKUPOPEN: the tree takes {}% of the panel's height",
                        (share * 100.0).round()
                    )
                    .as_ref(),
                );
                task
            }
            Some(Request::Split(None)) => {
                let (least, most) = pid_backup_browser::split::TREE_SHARE_RANGE;
                self.command_line.push_error(
                    crate::tf!(
                        "PIDBACKUPOPEN: Split takes a percentage from {} to {}",
                        (least * 100.0).round(),
                        (most * 100.0).round()
                    )
                    .as_ref(),
                );
                Task::none()
            }
            Some(Request::Press(Some(number))) => {
                let button = pid_backup_browser::selected_buttons(&self.pid_backup_browser)
                    .into_iter()
                    .nth(number - 1);
                match button {
                    Some((label, msg)) => self.press_pid_backup_button(label, msg),
                    None => {
                        self.command_line.push_error(
                            crate::tf!(
                                "PIDBACKUPOPEN: the details of the selected node have no button {}",
                                number
                            )
                            .as_ref(),
                        );
                        Task::none()
                    }
                }
            }
            Some(Request::PressNamed(name)) => {
                let mut buttons = pid_backup_browser::selected_buttons(&self.pid_backup_browser);
                // The label itself, else the first that starts with it.
                let wanted = name.to_lowercase();
                let found = buttons
                    .iter()
                    .position(|(label, _)| label.to_lowercase() == wanted)
                    .or_else(|| {
                        buttons
                            .iter()
                            .position(|(label, _)| label.to_lowercase().starts_with(&wanted))
                    });
                match found {
                    Some(at) => {
                        let (label, msg) = buttons.swap_remove(at);
                        self.press_pid_backup_button(label, msg)
                    }
                    None => {
                        self.command_line.push_error(
                            crate::tf!(
                                "PIDBACKUPOPEN: the details of the selected node have no button \"{}\"",
                                name
                            )
                            .as_ref(),
                        );
                        Task::none()
                    }
                }
            }
            Some(Request::Cache(request)) => {
                use pid_backup_browser::CacheMsg;
                let msg = match request {
                    CacheRequest::Report => CacheMsg::Report,
                    CacheRequest::Clear => CacheMsg::Clear,
                    CacheRequest::Default => CacheMsg::Folder(None),
                    CacheRequest::Folder(folder) => CacheMsg::Folder(Some(folder)),
                    CacheRequest::Unknown => {
                        self.command_line.push_error(
                            crate::t!(
                                "PIDBACKUPOPEN: Cache takes a folder with its full path, Default or Clear"
                            )
                            .as_ref(),
                        );
                        return Task::none();
                    }
                };
                self.on_pid_backup(PidBackupMsg::Cache(msg))
            }
            Some(Request::Press(None)) => {
                self.command_line.push_error(
                    crate::t!("PIDBACKUPOPEN: Press takes the number of a button, from 1")
                        .as_ref(),
                );
                Task::none()
            }
            Some(Request::Zip | Request::Folder) if on_command_line => {
                // No dialogs here: the keyword is not a path either.
                self.command_line
                    .push_info(crate::t!("PIDBACKUPOPEN  Backup zip file or folder:").as_ref());
                Task::none()
            }
            Some(Request::Zip) => self.pick_backup_source(false),
            Some(Request::Folder) => self.pick_backup_source(true),
            None => Task::none(),
        }
    }

    /// `Press` found its button: say which, and do what a click on it does.
    /// The one exception is the button that opens a folder dialog -- on the
    /// command line the folder is named in words.
    #[cfg(not(target_arch = "wasm32"))]
    fn press_pid_backup_button(&mut self, label: String, msg: PidBackupMsg) -> Task<Message> {
        if matches!(
            msg,
            PidBackupMsg::Cache(pid_backup_browser::CacheMsg::PickFolder)
        ) {
            self.say_how_the_cache_folder_is_named();
            return Task::none();
        }
        self.command_line
            .push_output(crate::tf!("PIDBACKUPOPEN: pressed \"{}\"", label).as_ref());
        self.on_pid_backup(msg)
    }

    /// The command line's words for what the folder dialog asks.
    #[cfg(not(target_arch = "wasm32"))]
    fn say_how_the_cache_folder_is_named(&mut self) {
        self.command_line.push_info(
            crate::t!(
                "PIDBACKUPOPEN: Cache=<folder> names the folder restored backups are kept in, Cache=Default the default one"
            )
            .as_ref(),
        );
    }

    /// Show the pallet where the layout has it, docking it on the right the
    /// first time. Unlike PIDPALETTE this never closes it: the command means
    /// "show the browser", not "toggle it".
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn show_pid_backup_panel(&mut self) {
        if self.dock.location(PanelId::PidBackup).is_none() {
            self.dock
                .dock(PanelId::PidBackup, crate::app::config::DockSide::Right, 0);
        }
        self.show_pid_backup = true;
        self.dock_reveal(PanelId::PidBackup);
    }

    /// A panel message: apply it through the panel's pure state machine and
    /// carry out the effect it asks for.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn on_pid_backup(&mut self, msg: PidBackupMsg) -> Task<Message> {
        // The tags the database gives are not the panel's news: they go on
        // the drawings that were opened from it.
        if let PidBackupMsg::TagsRead { generation, result } = msg {
            self.backup_tags_read(generation, result);
            return Task::none();
        }
        // An export to PDF reads the restores while it runs: none is removed
        // from under it, whoever asks -- `Cache=Clear`, `Press`, or the
        // panel's own button.
        if matches!(
            &msg,
            PidBackupMsg::Cache(pid_backup_browser::CacheMsg::Clear)
        ) && self.pid_backup_pdf_keeps_the_cache()
        {
            return Task::none();
        }
        // A finished load, and a source the user just picked, both mean the
        // browser has something to show: reveal it, so a load that finished
        // after the pallet was closed cannot land invisibly.
        if matches!(
            &msg,
            PidBackupMsg::Loaded(_) | PidBackupMsg::SourcePicked(Some(_))
        ) {
            self.show_pid_backup_panel();
        }
        // The command line reports what a finished load found, and a read
        // that failed -- the backup's or one drawing's items; the panel's own
        // state is the answer to everything else.
        let (loaded, failure) = match &msg {
            PidBackupMsg::Loaded(Ok(_)) => (true, None),
            PidBackupMsg::Loaded(Err(error))
            | PidBackupMsg::ItemsLoaded(_, Err(error)) => (false, Some(error.clone())),
            // A read that outlived its backup is dropped by the panel, so
            // its failure is nobody's news either.
            PidBackupMsg::SectionLoaded {
                generation,
                result: Err(error),
                ..
            } if *generation == self.pid_backup_browser.generation => {
                (false, Some(error.clone()))
            }
            _ => (false, None),
        };
        let cache_folder = pid_backup_browser::cache_folder(&self.pid_backup_browser)
            .map(Path::to_path_buf);
        let effect = pid_backup_browser::update(&mut self.pid_backup_browser, msg);
        // The cache folder is a preference: it goes into the settings the
        // moment it changes, as a toggle does.
        if pid_backup_browser::cache_folder(&self.pid_backup_browser) != cache_folder.as_deref() {
            self.save_config();
        }
        // What a clearing freed, where restores are kept after a move, what
        // the cache holds when the command line asked.
        match pid_backup_browser::take_cache_outcome(&mut self.pid_backup_browser) {
            Some(Ok(said)) => self
                .command_line
                .push_output(crate::tf!("PIDBACKUPOPEN: {}", said).as_ref()),
            Some(Err(error)) => self
                .command_line
                .push_error(crate::tf!("PIDBACKUPOPEN: {}", error).as_ref()),
            None => {}
        }
        if let Some(error) = failure {
            self.command_line
                .push_error(crate::tf!("PIDBACKUPOPEN: {}", error).as_ref());
        } else if loaded {
            if let Some(index) = self.pid_backup_browser.index() {
                self.command_line.push_output(
                    crate::tf!(
                        "PIDBACKUPOPEN: {} drawing(s), {} warning(s)",
                        index.drawings.len(),
                        index.warnings.len()
                    )
                    .as_ref(),
                );
                if index.read_only {
                    self.command_line.push_info(
                        crate::t!("PIDBACKUPOPEN: the folder is used in place (read-only).")
                            .as_ref(),
                    );
                }
            }
        }
        // A walk to a node (`Select`) ends on the message that started it, or
        // on the read it had to wait for: say where it got either way.
        match pid_backup_browser::take_goto_outcome(&mut self.pid_backup_browser) {
            Some(Ok(node)) => self
                .command_line
                .push_output(crate::tf!("PIDBACKUPOPEN: selected {}", node).as_ref()),
            Some(Err(error)) => self
                .command_line
                .push_error(crate::tf!("PIDBACKUPOPEN: {}", error).as_ref()),
            None => {}
        }
        let task = self.run_pid_backup_effect(effect);
        // The selection moved: once the panel is laid out again the tree
        // scrolls to it. Without a window there is nothing to scroll.
        if pid_backup_browser::take_reveal(&mut self.pid_backup_browser) {
            return Task::batch([
                task,
                iced::advanced::widget::operate(pid_backup_browser::reveal_selected()),
            ]);
        }
        task
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run_pid_backup_effect(&mut self, effect: Effect) -> Task<Message> {
        match effect {
            Effect::None => Task::none(),
            Effect::LoadItems(drawing, path) => Task::perform(
                crate::io::pid_backup_browse::load_items_in_background(path),
                move |result| {
                    Message::PidBackup(PidBackupMsg::ItemsLoaded(drawing, result.map(Arc::new)))
                },
            ),
            // A drawing a tab shows already is brought to the front, as OPEN
            // and the sheet set's sheets do: the open itself takes no notice
            // of the tabs on the desktop, and would load the file once more.
            // One that is opened takes the tags the backup's database
            // gives the items of its symbols.
            Effect::OpenDrawing(path) => match self.tab_showing(&path) {
                Some(index) => Task::done(Message::TabSwitch(index)),
                None => self.open_from_the_backup(path),
            },
            // A picked source, a refresh: both rebuild the index from a path.
            // The panel remembers the source it was given, so a refresh keeps
            // the backup it was showing.
            Effect::Load(source) | Effect::Refresh(source) => {
                self.load_pid_backup(source, None)
            }
            Effect::PickSource => self.pick_backup_source(false),
            // What a section wants read: off the UI thread, as the items
            // are, and back to the panel under the generation it left with.
            Effect::LoadSection {
                generation,
                section,
                node,
            } => Task::perform(
                pid_backup_browser::load_section_in_background(
                    &self.pid_backup_browser,
                    section,
                    node.clone(),
                ),
                move |result| {
                    Message::PidBackup(PidBackupMsg::SectionLoaded {
                        generation,
                        section,
                        node: node.clone(),
                        result,
                    })
                },
            ),
            // The panel's progress clock: one beat, then the panel looks
            // again at what is under way.
            Effect::Tick => Task::perform(pid_backup_browser::tick(), |()| {
                Message::PidBackup(PidBackupMsg::Progress)
            }),
            // The restore cache is walked and removed off the UI thread;
            // either way the panel hears what the folder holds afterwards.
            Effect::MeasureCache {
                asked,
                root,
                current,
            } => Task::perform(
                pid_backup_browser::measure_in_background(root, current),
                move |usage| {
                    Message::PidBackup(PidBackupMsg::Cache(
                        pid_backup_browser::CacheMsg::Measured {
                            asked,
                            result: Ok(usage),
                        },
                    ))
                },
            ),
            Effect::ClearCache {
                asked,
                root,
                current,
            } => Task::perform(
                pid_backup_browser::clear_in_background(root, current),
                move |result| {
                    Message::PidBackup(PidBackupMsg::Cache(
                        pid_backup_browser::CacheMsg::Measured { asked, result },
                    ))
                },
            ),
            // No dialogs with FILEDIA 0 or under a script: the folder is
            // named on the command line there.
            Effect::PickCacheFolder if self.file_prompts_on_command_line() => {
                self.say_how_the_cache_folder_is_named();
                Task::none()
            }
            Effect::PickCacheFolder => {
                let start = pid_backup_browser::cache_folder(&self.pid_backup_browser)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(crate::io::pid_backup::default_cache_root);
                Task::perform(pick_cache_folder_async(start), |folder| {
                    Message::PidBackup(PidBackupMsg::Cache(
                        pid_backup_browser::CacheMsg::FolderPicked(folder),
                    ))
                })
            }
            Effect::Batch(effects) => Task::batch(
                effects
                    .into_iter()
                    .map(|effect| self.run_pid_backup_effect(effect))
                    .collect::<Vec<_>>(),
            ),
        }
    }

    /// Put the panel in its loading state and index `source` off the UI
    /// thread. `target` is the restore folder, when the command named one.
    #[cfg(not(target_arch = "wasm32"))]
    fn load_pid_backup(&mut self, source: PathBuf, target: Option<PathBuf>) -> Task<Message> {
        // A relative restore folder sits beside the backup.
        let target = target.map(|target| match source.parent() {
            Some(parent) if target.is_relative() => parent.join(target),
            _ => target,
        });
        // The state machine owns the loading state, so the panel shows the
        // source it is reading without the command layer writing it twice.
        if !matches!(
            self.pid_backup_browser.load,
            LoadState::Loading(ref current) if *current == source
        ) {
            let _ = pid_backup_browser::update(
                &mut self.pid_backup_browser,
                PidBackupMsg::SourcePicked(Some(source.clone())),
            );
        }
        self.command_line
            .push_output(crate::tf!("PIDBACKUPOPEN: restoring {} ...", source.display()).as_ref());
        // A backup that has to be unpacked goes into the cache folder the
        // user set, when there is one.
        let cache = pid_backup_browser::cache_folder(&self.pid_backup_browser)
            .map(Path::to_path_buf);
        Task::perform(
            crate::io::pid_backup_browse::build_index_in_background(source, target, cache),
            |result| Message::PidBackup(PidBackupMsg::Loaded(result.map(Arc::new))),
        )
    }

    /// The source dialog: a backup zip, or with `folder` a backup folder.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_backup_source(&mut self, folder: bool) -> Task<Message> {
        Task::perform(pick_backup_source_async(folder), |source| {
            Message::PidBackup(PidBackupMsg::SourcePicked(source))
        })
    }
}

/// The dialog for the backup itself, as the picker task the caller awaits.
#[cfg(not(target_arch = "wasm32"))]
async fn pick_backup_source_async(folder: bool) -> Option<PathBuf> {
    let dialog = crate::sys::file_dialog();
    let handle = if folder {
        dialog
            .set_title(crate::t!("Select P&ID Backup Folder").as_ref())
            .pick_folder()
            .await
    } else {
        dialog
            .set_title(crate::t!("Select P&ID Backup").as_ref())
            .add_filter(
                crate::t!("SmartPlant backup (*.zip)").as_ref(),
                &["zip", "ZIP"],
            )
            .pick_file()
            .await
    };
    handle.map(|handle| crate::sys::handle_path(&handle))
}

/// The dialog for the folder restored backups are kept in, opened where
/// they are kept now.
#[cfg(not(target_arch = "wasm32"))]
async fn pick_cache_folder_async(start: PathBuf) -> Option<PathBuf> {
    let mut dialog = crate::sys::file_dialog()
        .set_title(crate::t!("Select the Folder for Restored Backups").as_ref());
    if start.is_dir() {
        dialog = dialog.set_directory(&start);
    }
    // Not through `sys::handle_path`: that remembers the folder for the
    // next file dialog, and nobody opens drawings from beside the cache.
    dialog
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::ui::window::pid_backup_browser::{NodeId, Row};
    use crate::ui::command_line::EntryKind;

    fn none(_: &Path) -> bool {
        false
    }

    #[test]
    fn keywords_pick_the_dialog() {
        assert_eq!(parse_request("z", none), Some(Request::Zip));
        assert_eq!(parse_request(" Zip ", none), Some(Request::Zip));
        assert_eq!(parse_request("f", none), Some(Request::Folder));
        assert_eq!(parse_request("FOLDER", none), Some(Request::Folder));
        assert_eq!(parse_request("  ", none), None);
    }

    #[test]
    fn a_source_and_a_restore_folder_split_at_whitespace_outside_quotes() {
        assert_eq!(
            parse_request(r#""D:/my backups/a.zip" D:/out"#, none),
            Some(Request::Restore(
                PathBuf::from("D:/my backups/a.zip"),
                Some(PathBuf::from("D:/out"))
            ))
        );
        assert_eq!(
            parse_request("D:/a.zip", none),
            Some(Request::Restore(PathBuf::from("D:/a.zip"), None))
        );
    }

    #[test]
    fn an_existing_path_with_spaces_is_the_source_alone() {
        let exists = |path: &Path| path == Path::new("D:/my backups/plant");
        assert_eq!(
            parse_request("D:/my backups/plant", exists),
            Some(Request::Restore(PathBuf::from("D:/my backups/plant"), None))
        );
    }

    #[test]
    fn select_takes_the_rest_of_the_line_as_the_labels_of_a_node() {
        assert_eq!(
            parse_request("Select Plant groups > SQPlant > 00", none),
            Some(Request::Select("Plant groups > SQPlant > 00".to_string()))
        );
        assert_eq!(
            parse_request("s   A > B ", none),
            Some(Request::Select("A > B".to_string()))
        );
        // The word alone selects the backup itself.
        assert_eq!(
            parse_request("SELECT", none),
            Some(Request::Select(String::new()))
        );
        // A word that only starts like the keyword is a source, as before.
        assert_eq!(
            parse_request("selection.zip", none),
            Some(Request::Restore(PathBuf::from("selection.zip"), None))
        );
        // And a backup that is really called `Select` is opened, not walked.
        let exists = |path: &Path| path == Path::new("Select");
        assert_eq!(
            parse_request("Select", exists),
            Some(Request::Restore(PathBuf::from("Select"), None))
        );
    }

    // ── Application-level wiring ─────────────────────────────────────────
    //
    // The panel's pure functions are tested in its own module; these run the
    // real app so the message routing, the dock visibility and the command
    // line report are covered too.

    use std::sync::Arc;

    use crate::io::pid_backup_browse::{
        BackupFile, BackupIndex, DrawingEntry, FileKind, GroupNode, ManifestSummary, RefNode,
    };

    /// An app with a drawing open, so the dock has a window to appear in.
    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    /// A backup index with two plant groups, two drawings and one warning.
    fn index() -> Arc<BackupIndex> {
        let mut plant = GroupNode {
            name: "00".to_string(),
            path: "SQPlant / 00".to_string(),
            children: Vec::new(),
            drawings: vec![0, 1],
        };
        plant.children.push(GroupNode {
            name: "01".to_string(),
            path: "SQPlant / 00 / 01".to_string(),
            children: Vec::new(),
            drawings: Vec::new(),
        });
        let root = GroupNode {
            name: "SQPlant".to_string(),
            path: "SQPlant".to_string(),
            children: vec![plant],
            drawings: Vec::new(),
        };
        let drawing = |name: &str, file: Option<&str>| DrawingEntry {
            name: name.to_string(),
            pid_path: file.map(PathBuf::from),
            group_path: "SQPlant / 00".to_string(),
            meta: vec![
                ("id".to_string(), "7".to_string()),
                ("drawing_number".to_string(), name.to_string()),
            ],
        };
        Arc::new(BackupIndex {
            source: PathBuf::from("D:/backups/plant.zip"),
            restored: PathBuf::from("D:/tmp/restore"),
            backup_root: PathBuf::from("D:/tmp/restore"),
            read_only: false,
            manifest: Some(ManifestSummary {
                name: Some("Plant backup".to_string()),
                ..ManifestSummary::default()
            }),
            files: vec![BackupFile {
                name: "Manifest.txt".to_string(),
                kind: FileKind::Manifest,
                format: "Manifest".to_string(),
                size: 12,
                members: Vec::new(),
                version_line: None,
                missing: false,
                ..BackupFile::default()
            }],
            plant_groups: vec![root],
            drawings: vec![
                drawing("A-101", Some("D:/tmp/restore/00/00/a.pid")),
                drawing("B-202", None),
            ],
            ref_tree: RefNode::default(),
            warnings: vec!["no usable Export.dmp".to_string()],
            ..BackupIndex::default()
        })
    }

    /// The rows the panel would draw, as `(label, disabled)` pairs.
    fn row_labels(app: &OpenCADStudio) -> Vec<(String, bool)> {
        pid_backup_browser::visible_rows(&app.pid_backup_browser)
            .into_iter()
            .map(|row: Row| (row.label, row.disabled))
            .collect()
    }

    #[test]
    fn pidbackupopen_shows_the_panel_without_closing_it_again() {
        let mut app = app();
        assert!(!app.dock_panel_visible(PanelId::PidBackup));
        let _ = app.dispatch_command("PIDBACKUPOPEN");
        assert!(
            app.dock_panel_visible(PanelId::PidBackup),
            "PIDBACKUPOPEN 打开面板"
        );
        assert_eq!(
            app.dock.location(PanelId::PidBackup).map(|(side, _)| side),
            Some(crate::app::config::DockSide::Right),
            "首次打开停靠在右侧"
        );
        // The command shows the browser; it is not a toggle, so running it
        // again leaves the panel where it is.
        let _ = app.dispatch_command("PIDBACKUPOPEN");
        assert!(app.dock_panel_visible(PanelId::PidBackup));
    }

    #[test]
    fn a_loaded_backup_becomes_visible_and_is_reported() {
        let mut app = app();
        let start = app.command_line.history.len();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        assert!(app.dock_panel_visible(PanelId::PidBackup), "加载后面板可见");
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Ready(_)
        ));
        let report: Vec<&str> = app.command_line.history[start..]
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(
            report.iter().any(|line| line.contains('2')
                && line.contains('1')
                && line.contains("drawing")),
            "命令行报告图纸数与警告数：{report:?}"
        );
        // The tree starts on the backup itself and its three groups.
        let rows = row_labels(&app);
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[0].0, "Plant backup");
        assert!(!rows[0].1);
    }

    #[test]
    fn a_failed_load_is_reported_as_an_error() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Err("no Manifest.txt".to_string())));
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains("no Manifest.txt"), "{}", last.text);
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Idle
        ));
    }

    #[test]
    fn a_drawing_whose_items_cannot_be_read_is_reported_as_an_error() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let _ = app.on_pid_backup(PidBackupMsg::ItemsLoaded(
            0,
            Err("cannot read a.pid: no DrawingNo".to_string()),
        ));
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains("no DrawingNo"), "{}", last.text);
        // One drawing's failure is not the backup's: the tree stays.
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Ready(_)
        ));
        assert_eq!(row_labels(&app).len(), 4);
    }

    /// Showing the browser docks a panel, and the dock pans the camera so
    /// the drawing stays where it was on screen. In a drawn application the
    /// next tick took that pan for a change of view and marked the drawing
    /// modified: closing the window after PIDBACKUPOPEN asked whether to
    /// save a drawing nobody had touched.
    #[test]
    fn showing_the_panel_does_not_mark_the_drawing_modified() {
        let mut app = app();
        let i = app.active_tab;
        // A window and a viewport, as a drawn application has them.
        let _ = app.update(Message::WindowResized(1600.0, 900.0));
        app.tabs[i].scene.selection.borrow_mut().view.vp_size = (1400.0, 760.0);
        let tick = |app: &mut OpenCADStudio| {
            let _ = app.update(Message::Tick(std::time::Instant::now()));
        };
        tick(&mut app);
        tick(&mut app);
        app.tabs[i].dirty = false;
        let before = app.tabs[i].scene.camera.borrow().target;

        let _ = app.dispatch_command("PIDBACKUPOPEN");
        assert!(app.dock_panel_visible(PanelId::PidBackup));
        // The dock change is seen at the end of the next update, which pans;
        // the tick after that is where the pan became a modification.
        tick(&mut app);
        tick(&mut app);
        assert_ne!(
            app.tabs[i].scene.camera.borrow().target,
            before,
            "the dock pans the camera to keep the drawing where it was"
        );
        assert!(!app.tabs[i].dirty, "showing a panel is not an edit");
        // The document still holds the camera it is drawn with, so a later
        // save writes the view the user sees.
        assert_eq!(
            app.tabs[i].last_synced_camera_gen,
            app.tabs[i].scene.camera_generation
        );
    }

    #[test]
    fn a_select_line_goes_to_the_node_and_reports_it() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let groups = crate::t!("Plant groups").into_owned();
        // As the command line and the automation channel send it: one line,
        // words and all.
        let task = app.run_command_line(&format!("PIDBACKUPOPEN Select {groups} > SQPlant > 00"));
        app.drive_headless_task(task).expect("a walk needs no window");
        assert_eq!(
            app.pid_backup_browser.selected,
            Some(NodeId::Group(vec![0, 0]))
        );
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Output);
        assert!(last.text.contains("SQPlant > 00"), "{}", last.text);
        // The levels above it are open: the row is on screen.
        let labels: Vec<String> = row_labels(&app).into_iter().map(|(label, _)| label).collect();
        assert!(labels.contains(&"00".to_string()), "{labels:?}");
    }

    #[test]
    fn split_takes_a_percentage_for_the_tree() {
        let none = |_: &Path| false;
        assert_eq!(parse_request("Split 30", none), Some(Request::Split(Some(30.0))));
        assert_eq!(parse_request("split 42.5%", none), Some(Request::Split(Some(42.5))));
        // No number, or one that is no percentage.
        assert_eq!(parse_request("Split", none), Some(Request::Split(None)));
        assert_eq!(parse_request("Split wide", none), Some(Request::Split(None)));
        assert_eq!(parse_request("Split 130", none), Some(Request::Split(None)));
        // Only the whole word: a source called `splitter.zip` is a source.
        assert_eq!(
            parse_request("splitter.zip", none),
            Some(Request::Restore(PathBuf::from("splitter.zip"), None))
        );
        // And a backup that is really called `Split` is opened.
        let exists = |path: &Path| path == Path::new("Split");
        assert_eq!(
            parse_request("Split", exists),
            Some(Request::Restore(PathBuf::from("Split"), None))
        );
    }

    #[test]
    fn press_takes_the_number_of_a_button() {
        let none = |_: &Path| false;
        assert_eq!(parse_request("Press", none), Some(Request::Press(Some(1))));
        assert_eq!(parse_request("press 2", none), Some(Request::Press(Some(2))));
        // No such number.
        assert_eq!(parse_request("Press 0", none), Some(Request::Press(None)));
        // What is no number is the label of a button, quoted or not.
        assert_eq!(
            parse_request("Press Check every drawing", none),
            Some(Request::PressNamed("Check every drawing".to_string()))
        );
        assert_eq!(
            parse_request("press \"Open drawing\"", none),
            Some(Request::PressNamed("Open drawing".to_string()))
        );
        // Only the whole word, and a backup really called `Press` is opened.
        assert_eq!(
            parse_request("pressure.zip", none),
            Some(Request::Restore(PathBuf::from("pressure.zip"), None))
        );
        let exists = |path: &Path| path == Path::new("Press");
        assert_eq!(
            parse_request("Press", exists),
            Some(Request::Restore(PathBuf::from("Press"), None))
        );
    }

    /// A script presses what a pointer would: the button in the details of
    /// the selected node, here the one that starts the check of the
    /// drawings.
    #[test]
    fn a_press_line_presses_the_button_of_the_selected_node() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        // Nothing selected: nothing to press.
        let _ = app.dispatch_command("PIDBACKUPOPEN Press");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(app.pid_backup_browser.section_data.is_empty());

        let _ = app.on_pid_backup(PidBackupMsg::Select(NodeId::Root));
        // The backup's details: the button that moves the restore cache,
        // then the one that starts the check of the drawings.
        let buttons = pid_backup_browser::selected_buttons(&app.pid_backup_browser);
        let labels: Vec<&str> = buttons.iter().map(|(label, _)| label.as_str()).collect();
        let folder = crate::t!("Change the cache folder…");
        let check = crate::t!("Check every drawing");
        assert_eq!(labels, [folder.as_ref(), check.as_ref()]);
        let PidBackupMsg::Activate(node) = buttons[1].1.clone() else {
            panic!("the check is started by activating its node: {:?}", buttons[1].1);
        };
        // By its label, as a script names it: the press starts the check,
        // and the panel waits for its result.
        let _ = app.dispatch_command(&format!("PIDBACKUPOPEN Press {check}"));
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Output);
        assert!(last.text.contains(check.as_ref()), "{}", last.text);
        assert!(app
            .pid_backup_browser
            .section_data
            .iter()
            .any(|((_, found), _)| *found == node));
        // The first button, by number and by the start of its label, would
        // open a folder dialog under a pointer. The command line says how
        // the folder is named there instead, and nothing is asked.
        for line in ["PIDBACKUPOPEN Press", "PIDBACKUPOPEN Press change the cache"] {
            let history = app.command_line.history.len();
            let task = app.dispatch_command(line);
            app.drive_headless_task(task)
                .expect("no dialog is waited for");
            let said = app.command_line.history[history..]
                .last()
                .unwrap_or_else(|| panic!("{line} says nothing"));
            assert_eq!(said.kind, EntryKind::Info, "{line}");
            assert!(said.text.contains("Cache="), "{}", said.text);
        }
        assert_eq!(pid_backup_browser::cache_folder(&app.pid_backup_browser), None);
        // A button the details do not have: by number, by a number that is
        // none, and by a label.
        let _ = app.dispatch_command("PIDBACKUPOPEN Press 7");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains('7'), "{}", last.text);
        let _ = app.dispatch_command("PIDBACKUPOPEN Press 0");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        let _ = app.dispatch_command("PIDBACKUPOPEN Press twice");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains("twice"), "{}", last.text);
    }

    #[test]
    fn cache_takes_a_folder_default_or_clear() {
        let none = |_: &Path| false;
        let cache = |line: &str| match parse_request(line, none) {
            Some(Request::Cache(request)) => request,
            other => panic!("{line} is {other:?}"),
        };
        assert_eq!(cache("Cache"), CacheRequest::Report);
        assert_eq!(cache("cache="), CacheRequest::Report);
        assert_eq!(cache("Cache=Clear"), CacheRequest::Clear);
        assert_eq!(cache("CACHE clear"), CacheRequest::Clear);
        assert_eq!(cache("Cache=default"), CacheRequest::Default);
        let folder = std::env::temp_dir().join("pid cache");
        assert_eq!(
            cache(&format!("Cache={}", folder.display())),
            CacheRequest::Folder(folder.clone())
        );
        assert_eq!(
            cache(&format!("Cache = \"{}\"", folder.display())),
            CacheRequest::Folder(folder.clone())
        );
        assert_eq!(
            cache(&format!("Cache {}", folder.display())),
            CacheRequest::Folder(folder)
        );
        // A folder is named with its full path: a bare name would be kept
        // wherever the application happened to be started.
        assert_eq!(cache("Cache=restores"), CacheRequest::Unknown);
        // Only the whole word, and a backup really called `Cache` is opened.
        assert_eq!(
            parse_request("cached.zip", none),
            Some(Request::Restore(PathBuf::from("cached.zip"), None))
        );
        let exists = |path: &Path| path == Path::new("Cache");
        assert_eq!(
            parse_request("Cache", exists),
            Some(Request::Restore(PathBuf::from("Cache"), None))
        );
    }

    /// A zip of one drawing, as small as a backup that has to be unpacked
    /// gets.
    fn tiny_backup(path: &Path, drawing: &str) {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file(format!("00/00/{drawing}.pid"), options).unwrap();
        writer.write_all(drawing.as_bytes()).unwrap();
        writer.finish().unwrap();
    }

    /// The whole way of the cache folder: named on the command line, kept
    /// in the settings, read back by the next application, used by the
    /// restores, measured in the backup's details and cleared.
    #[test]
    fn the_cache_folder_is_kept_in_the_settings_and_restores_go_into_it() {
        let root = std::env::temp_dir()
            .join(format!("ocs-pidbackup-cache-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cache = root.join("restores");
        let first = root.join("first.zip");
        let second = root.join("second.zip");
        tiny_backup(&first, "A-101");
        tiny_backup(&second, "B-202");
        // Run a line as the command line and the automation channel send
        // it, and give what it said: the outputs and the errors.
        let run = |app: &mut OpenCADStudio, line: String| -> Vec<(EntryKind, String)> {
            let history = app.command_line.history.len();
            let task = app.run_command_line(&line);
            app.drive_headless_task(task)
                .expect("the restore cache needs no window");
            app.command_line.history[history..]
                .iter()
                .filter(|entry| matches!(entry.kind, EntryKind::Output | EntryKind::Error))
                .map(|entry| (entry.kind.clone(), entry.text.clone()))
                .collect()
        };
        let cache_text = cache.display().to_string();

        // Named on the command line: the panel has it, and so have the
        // settings the application would write.
        let mut app = app();
        assert_eq!(app.current_config().pid_backup.cache_dir, None);
        let said = run(&mut app, format!("PIDBACKUPOPEN Cache={cache_text}"));
        assert!(
            said.iter()
                .any(|(kind, text)| *kind == EntryKind::Output && text.contains(&cache_text)),
            "{said:?}"
        );
        assert_eq!(
            pid_backup_browser::cache_folder(&app.pid_backup_browser),
            Some(cache.as_path())
        );
        let saved = serde_json::to_string(&app.current_config()).unwrap();
        assert!(saved.contains("cache_dir"), "{saved}");
        // Naming a folder makes none: the first restore does.
        assert!(!cache.exists());

        // The next application starts with those settings.
        let mut app = OpenCADStudio::new_for_test();
        app.apply_config(serde_json::from_str(&saved).unwrap());
        let _ = app.automation_op(r#"{"op":"new"}"#);
        assert_eq!(
            pid_backup_browser::cache_folder(&app.pid_backup_browser),
            Some(cache.as_path())
        );
        // Its restores go into the folder, each into one of its own.
        run(&mut app, format!("PIDBACKUPOPEN {}", first.display()));
        let restored_first = app
            .pid_backup_browser
            .index()
            .expect("the first backup loads")
            .restored
            .clone();
        assert_eq!(restored_first.parent(), Some(cache.as_path()));
        assert!(restored_first.join("00/00/A-101.pid").is_file());
        run(&mut app, format!("PIDBACKUPOPEN {}", second.display()));
        let restored_second = app
            .pid_backup_browser
            .index()
            .expect("the second backup loads")
            .restored
            .clone();
        assert_eq!(restored_second.parent(), Some(cache.as_path()));
        assert_ne!(restored_first, restored_second);

        // Selecting the backup measures the folder: this backup's restore,
        // and the other one.
        let task = app.on_pid_backup(PidBackupMsg::Select(NodeId::Root));
        app.drive_headless_task(task)
            .expect("measuring needs no window");
        let usage = pid_backup_browser::cache_usage(&app.pid_backup_browser)
            .cloned()
            .expect("the cache is measured");
        assert_eq!(usage.root, cache);
        assert_eq!(
            usage.current.as_ref().map(|restore| restore.folder.clone()),
            Some(restored_second.clone())
        );
        assert_eq!(usage.others, 1);
        let buttons: Vec<String> = pid_backup_browser::selected_buttons(&app.pid_backup_browser)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(
            buttons[..3],
            [
                crate::t!("Clear the other restores").into_owned(),
                crate::t!("Change the cache folder…").into_owned(),
                crate::t!("Use the default folder").into_owned(),
            ]
        );
        // The command line says the same when asked.
        let said = run(&mut app, "PIDBACKUPOPEN Cache".to_string());
        assert!(
            said.iter()
                .any(|(kind, text)| *kind == EntryKind::Output && text.contains(&cache_text)),
            "{said:?}"
        );

        // Clearing, here by the button's label: the other restore goes,
        // the one in use stays, and both the panel and the command line
        // say what was freed.
        let said = run(
            &mut app,
            format!("PIDBACKUPOPEN Press {}", crate::t!("Clear the other restores")),
        );
        assert!(!restored_first.exists(), "the other restore is removed");
        assert!(restored_second.join("00/00/B-202.pid").is_file());
        let freed = pid_backup_browser::cache_usage(&app.pid_backup_browser)
            .and_then(|usage| usage.freed)
            .expect("the clearing says what it freed");
        assert_eq!(freed.0, 1);
        let freed_text = crate::tf!(
            "{} restore(s) removed, {} freed",
            freed.0,
            pid_backup_browser::human_size(freed.1)
        )
        .into_owned();
        assert!(
            said.iter()
                .any(|(kind, text)| *kind == EntryKind::Output && text.contains(&freed_text)),
            "{said:?}"
        );
        assert!(said.iter().all(|(kind, _)| *kind != EntryKind::Error), "{said:?}");
        // `Cache=Clear` with nothing left to remove removes nothing.
        run(&mut app, "PIDBACKUPOPEN Cache=Clear".to_string());
        assert!(restored_second.join("00/00/B-202.pid").is_file());
        assert_eq!(
            pid_backup_browser::cache_usage(&app.pid_backup_browser).and_then(|usage| usage.freed),
            Some((0, 0))
        );

        // Back to the default folder: the settings no longer name one.
        run(&mut app, "PIDBACKUPOPEN Cache=Default".to_string());
        assert_eq!(pid_backup_browser::cache_folder(&app.pid_backup_browser), None);
        assert_eq!(app.current_config().pid_backup.cache_dir, None);
        // What is no folder, word or path is refused, and nothing changes.
        let said = run(&mut app, "PIDBACKUPOPEN Cache=somewhere".to_string());
        assert!(
            said.last().is_some_and(|(kind, _)| *kind == EntryKind::Error),
            "{said:?}"
        );
        assert_eq!(pid_backup_browser::cache_folder(&app.pid_backup_browser), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_split_line_moves_the_bar_and_says_where_it_is() {
        let mut app = app();
        let default = app.pid_backup_browser.split().tree_share();
        // With a backup in the panel, as a script has it by then: the bare
        // verb asks nothing, so the command line hands the whole line over.
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let task = app.run_command_line("PIDBACKUPOPEN Split 30");
        app.drive_headless_task(task).expect("moving the bar needs no window");
        assert_eq!(app.pid_backup_browser.split().tree_share(), 0.3);
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Output);
        assert!(last.text.contains("30"), "{}", last.text);
        // Out of range is brought to the nearest share a half can have.
        let _ = app.dispatch_command("PIDBACKUPOPEN Split 1");
        let (least, _) = pid_backup_browser::split::TREE_SHARE_RANGE;
        assert_eq!(app.pid_backup_browser.split().tree_share(), least);
        let last = app.command_line.history.last().expect("一条历史");
        assert!(last.text.contains("12"), "{}", last.text);
        // What is no percentage is refused, and the bar stays.
        let _ = app.dispatch_command("PIDBACKUPOPEN Split much");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert_eq!(app.pid_backup_browser.split().tree_share(), least);
        // The bar is the user's: a backup arriving or leaving keeps it.
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let _ = app.on_pid_backup(PidBackupMsg::Close);
        assert_eq!(app.pid_backup_browser.split().tree_share(), least);
        assert_ne!(least, default);
    }

    /// A selection asks the tree to scroll to it once, whoever made it: a
    /// click, or a walk from the command line that ends after its drawing
    /// was read.
    #[test]
    fn a_selection_is_revealed_once() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        assert!(!pid_backup_browser::take_reveal(&mut app.pid_backup_browser));
        // Without a window the scroll itself has nothing to act on; the
        // request is taken with the message that made it.
        let task = app.on_pid_backup(PidBackupMsg::Select(NodeId::Groups));
        app.drive_headless_task(task).expect("a selection needs no window");
        assert!(!pid_backup_browser::take_reveal(&mut app.pid_backup_browser));
        // The panel alone: a selection sets it, unfolding does not.
        let state = &mut app.pid_backup_browser;
        pid_backup_browser::update(state, PidBackupMsg::Select(NodeId::Files));
        assert!(pid_backup_browser::take_reveal(state));
        assert!(!pid_backup_browser::take_reveal(state));
        pid_backup_browser::update(state, PidBackupMsg::Toggle(NodeId::Groups));
        assert!(!pid_backup_browser::take_reveal(state));
        pid_backup_browser::update(state, PidBackupMsg::Goto(crate::t!("Plant groups").into_owned()));
        assert!(pid_backup_browser::take_reveal(state));
    }

    #[test]
    fn a_select_line_that_names_no_node_is_reported_as_an_error() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let _ = app.dispatch_command("PIDBACKUPOPEN Select nowhere > at all");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains("nowhere"), "{}", last.text);
        assert_eq!(app.pid_backup_browser.selected, None);

        // With no backup open there is nothing to walk.
        let mut idle = self::app();
        let _ = idle.dispatch_command("PIDBACKUPOPEN Select anything");
        let last = idle.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
    }

    /// An app whose panel browses [`index`] with the test section, which
    /// hangs two nodes under every backup file: one whose read succeeds and
    /// one whose read fails.
    fn app_with_fake_section() -> OpenCADStudio {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        app.pid_backup_browser.use_fake_sections();
        app
    }

    #[test]
    fn what_a_section_wants_read_is_read_off_the_ui_thread_and_lands_in_the_panel() {
        let mut app = app_with_fake_section();
        let node = NodeId::Section(0, vec![0, 0]);
        let history = app.command_line.history.len();
        let task = app.on_pid_backup(PidBackupMsg::Select(node.clone()));
        app.drive_headless_task(task)
            .expect("a section's read needs no window");
        let details = pid_backup_browser::detail_rows(&app.pid_backup_browser, &node);
        assert!(
            details
                .iter()
                .any(|(key, value)| key == "fake data" && value.ends_with("file 0 of 1")),
            "{details:?}"
        );
        // A read that went well is the panel's news alone.
        assert_eq!(app.command_line.history.len(), history);
    }

    #[test]
    fn a_section_read_that_fails_is_reported_as_an_error() {
        let mut app = app_with_fake_section();
        let node = NodeId::Section(0, vec![0, 1]);
        let task = app.on_pid_backup(PidBackupMsg::Select(node.clone()));
        app.drive_headless_task(task)
            .expect("a section's read needs no window");
        let last = app.command_line.history.last().expect("一条历史");
        assert_eq!(last.kind, EntryKind::Error);
        assert!(last.text.contains("the fake read failed"), "{}", last.text);
        // The node says so too, and the backup stays open.
        assert!(pid_backup_browser::detail_rows(&app.pid_backup_browser, &node)
            .iter()
            .any(|(key, _)| key == "the fake read failed"));
        assert!(matches!(app.pid_backup_browser.load, LoadState::Ready(_)));
    }

    /// The whole way a script goes to read a file of a backup: open the
    /// folder, walk to the file by its label, and the text is in the panel.
    /// Nothing here uses a pointer, and the reads run on their own threads.
    #[test]
    fn a_file_reached_from_the_command_line_shows_its_text() {
        let root = std::env::temp_dir()
            .join(format!("ocs-pidbackup-command-text-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Manifest.txt"), "Name<<|>>SQPlant\n").unwrap();
        std::fs::write(
            root.join("Export.log"),
            "Connected to: Oracle Database\nExport terminated successfully without warnings.\n",
        )
        .unwrap();
        std::fs::write(root.join("Styles.spp"), [0xD0, 0xCF, 0x11, 0xE0, 0, 0, 0, 0]).unwrap();

        let mut app = app();
        let task = app.run_command_line(&format!("PIDBACKUPOPEN {}", root.display()));
        app.drive_headless_task(task)
            .expect("indexing a folder needs no window");
        let files = crate::t!("Backup files").into_owned();
        let history = app.command_line.history.len();
        let task = app.run_command_line(&format!("PIDBACKUPOPEN Select {files} > Export.log"));
        app.drive_headless_task(task)
            .expect("reading a file needs no window");
        let selected = app
            .pid_backup_browser
            .selected
            .clone()
            .expect("the walk ends on the file");
        assert!(matches!(selected, NodeId::File(_)), "{selected:?}");
        let details = pid_backup_browser::detail_rows(&app.pid_backup_browser, &selected);
        assert!(
            details.contains(&(
                "2".to_string(),
                "Export terminated successfully without warnings.".to_string()
            )),
            "{details:?}"
        );
        // The walk is reported; a read that went well is not.
        let said: Vec<&str> = app.command_line.history[history..]
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(
            said.iter().any(|line| line.contains("Export.log")),
            "{said:?}"
        );
        assert!(app.command_line.history[history..]
            .iter()
            .all(|entry| entry.kind != EntryKind::Error));

        // A file that is not text says so instead.
        let task = app.run_command_line(&format!("PIDBACKUPOPEN Select {files} > Styles.spp"));
        app.drive_headless_task(task)
            .expect("reading a file needs no window");
        let selected = app.pid_backup_browser.selected.clone().expect("a file");
        let details = pid_backup_browser::detail_rows(&app.pid_backup_browser, &selected);
        assert!(
            details.contains(&(
                crate::t!("Not a text file; its content is not shown.").into_owned(),
                String::new()
            )),
            "{details:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_section_read_for_a_backup_that_was_closed_is_not_reported() {
        let mut app = app_with_fake_section();
        let history = app.command_line.history.len();
        let _ = app.on_pid_backup(PidBackupMsg::SectionLoaded {
            generation: app.pid_backup_browser.generation + 1,
            section: 0,
            node: NodeId::Section(0, vec![0, 1]),
            result: Err("too late".to_string()),
        });
        assert_eq!(app.command_line.history.len(), history);
    }

    #[test]
    fn the_start_page_opens_a_blank_drawing_before_showing_the_panel() {
        // A fresh app is on the welcome tab, which draws no dock.
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start, "需要一个开始页");
        let tabs = app.tabs.len();
        let _ = app.dispatch_command("PIDBACKUPOPEN D:/backups/plant.zip");
        assert_eq!(app.tabs.len(), tabs + 1, "开始页执行新建了一个标签页");
        assert!(
            !app.tabs[app.active_tab].is_start,
            "新标签是空白图，不是开始页"
        );
        assert!(
            app.tabs[app.active_tab].scene.document.entities().count() == 0,
            "新图是空白图"
        );
        assert!(app.dock_panel_visible(PanelId::PidBackup), "面板可见");
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Loading(_)
        ));
    }

    #[test]
    fn activating_a_drawing_asks_for_the_drawing_to_open() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let task = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(0)));
        app.drive_headless_task(task).unwrap();
        // `OpenRecent` of a file that is not on disk reports it, which proves
        // the message reached the open path with the drawing's own path.
        let report: Vec<&str> = app.command_line.history.iter().map(|e| e.text.as_str()).collect();
        assert!(
            report.iter().any(|line| line.contains("a.pid")),
            "OpenRecent 收到了图纸路径：{report:?}"
        );
        // A drawing with no file has nothing to open.
        let start = app.command_line.history.len();
        let task = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(1)));
        app.drive_headless_task(task).unwrap();
        assert_eq!(app.command_line.history.len(), start, "缺文件图纸不可打开");
    }

    #[test]
    fn filedia_zero_asks_on_the_command_line_instead_of_a_dialog() {
        let mut app = app();
        crate::sys::set_file_dia(false);
        let start = app.command_line.history.len();
        let task = app.dispatch_command("PIDBACKUPOPEN");
        // Driving the task proves no dialog was scheduled: a dialog task
        // would never resolve without a window, and the prompt's focus is
        // the only widget action it carries.
        app.drive_headless_task(task).unwrap();
        let added: Vec<&str> = app.command_line.history[start..]
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(
            added.iter().any(|line| line.contains("zip")
                || line.contains("folder")
                || line.contains("Zip")),
            "命令行要路径：{added:?}"
        );
        assert!(app.dock_panel_visible(PanelId::PidBackup), "面板仍然显示");
        assert!(
            app.tabs[app.active_tab]
                .active_cmd
                .as_ref()
                .is_some_and(|cmd| cmd.name() == "PIDBACKUPOPEN"),
            "命令行提示接管了输入"
        );
        crate::sys::set_file_dia(true);
    }

    /// Under a script nothing asks in a dialog: the button that changes the
    /// cache folder says how the folder is named on the command line.
    #[test]
    fn under_a_script_the_cache_folder_button_opens_no_dialog() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        app.script_running = true;
        let history = app.command_line.history.len();
        let task = app.on_pid_backup(PidBackupMsg::Cache(
            pid_backup_browser::CacheMsg::PickFolder,
        ));
        // Driving the task proves no dialog was scheduled: a dialog task
        // would never resolve without a window.
        app.drive_headless_task(task)
            .expect("no dialog is waited for");
        let said: Vec<(&EntryKind, &str)> = app.command_line.history[history..]
            .iter()
            .map(|entry| (&entry.kind, entry.text.as_str()))
            .collect();
        assert!(
            said.iter()
                .any(|(kind, text)| **kind == EntryKind::Info && text.contains("Cache=")),
            "{said:?}"
        );
        assert_eq!(pid_backup_browser::cache_folder(&app.pid_backup_browser), None);
    }

    #[test]
    fn a_path_loads_the_backup_without_a_dialog() {
        let mut app = app();
        let task = app.dispatch_command("PIDBACKUPOPEN D:/backups/plant.zip");
        // The index runs on a worker thread, so the command returns a task
        // rather than a widget; either way no file dialog is involved.
        assert!(app.dock_panel_visible(PanelId::PidBackup));
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Loading(_)
        ));
        drop(task);
    }

    #[test]
    fn closing_the_backup_returns_the_panel_to_its_hint_and_reloads() {
        let mut app = app();
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        let _ = app.on_pid_backup(PidBackupMsg::Close);
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Idle
        ));
        assert!(
            pid_backup_browser::visible_rows(&app.pid_backup_browser).is_empty(),
            "关闭后树为空"
        );
        // PIDBACKUPOPEN <source> loads again.
        let _ = app.dispatch_command("PIDBACKUPOPEN D:/backups/plant.zip");
        assert!(matches!(
            app.pid_backup_browser.load,
            LoadState::Loading(_)
        ));
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index())));
        assert_eq!(row_labels(&app).len(), 4, "树重新出现");
    }

    #[test]
    fn the_application_menu_still_opens_the_browser() {
        // The menu entry is unchanged: "Open P&ID Backup..." runs
        // PIDBACKUPOPEN, which is the command that opens the panel.
        assert!(crate::command::all_registered_command_names().contains(&"PIDBACKUPOPEN"));
        let mut app = app();
        let _ = app.dispatch_command("PIDBACKUPOPEN");
        assert!(app.dock_panel_visible(PanelId::PidBackup));
    }
}

/// A walk over a real backup through the application; run with `--ignored`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod audit;

/// The tags the database of a backup gives its items, on the drawings
/// opened from the panel.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::app) mod tags;

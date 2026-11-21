//! P&ID backup browser — panel state and the pure functions behind it.
//!
//! A SmartPlant backup is four trees at once: the backup's own files (with the
//! member lists of its archives), the plant-group tree with the drawings in
//! it, the model items a drawing holds, and the `Ref` reference data. The
//! panel draws exactly one of them at a time as a flat list of rows, so the
//! list is [`visible_rows`]: a pure function of the state that walks only the
//! nodes the user expanded — a backup can carry 645 reference files and
//! thousands of model items, none of which a collapsed row may cost.
//!
//! Every click is a message, and [`update`] is the pure function that applies
//! it and says what the application should do next ([`Effect`]): read a
//! drawing's items off the UI thread, open a drawing or a symbol file in a
//! tab, ask for a source, rebuild the index. The panel therefore has no IO of
//! its own and is testable without a window, which is what
//! `visible_rows`/`update` are for.
//!
//! The state lives on the application, not on a tab, so switching documents
//! leaves the tree where the user left it.
//!
//! The three trees above are the panel's own. Every other kind of data a
//! backup holds is a [`section`]: a module beside this one that hangs its
//! rows under a row of the tree and states the details of its nodes.

mod cache;
mod database;
mod datamap;
mod diagnostics;
mod drawing_data;
#[cfg(test)]
mod drawn_tests;
mod goto;
pub mod grid;
mod manifest;
mod section;
pub mod split;
mod symbol_preview;
mod text_preview;

pub use cache::{
    cache_folder, cache_usage, clear_in_background, measure_in_background, set_cache_folder,
    take_cache_outcome, CacheMsg,
};
pub use goto::take_outcome as take_goto_outcome;
pub use section::{SectionData, SectionLoad};
pub use split::reveal_selected;
pub(crate) use symbol_preview::notes as symbol_notes;
#[cfg(test)]
pub(crate) use symbol_preview::opens as symbol_opens;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::Message;
use crate::io::pid_backup_browse::symbol_file::{SymbolDrawing, SymbolSource};
use crate::io::pid_backup_browse::{BackupIndex, DrawingItems, GroupNode, RefNode};
use crate::t;
use crate::ui::dock::{Chrome, PanelId};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, text, text_input, Space,
};
use iced::{Element, Fill, Length, Theme};

/// Where the panel is in loading a backup.
#[derive(Debug, Clone, Default)]
pub enum LoadState {
    /// Nothing loaded yet: the panel shows its "open a backup" hint.
    #[default]
    Idle,
    /// An index is being built for this source.
    Loading(PathBuf),
    /// The index is ready to browse.
    Ready(Arc<BackupIndex>),
}

/// What the panel knows about one drawing's model items.
#[derive(Debug, Clone)]
pub enum ItemsState {
    /// The read is in flight; the drawing's subtree stays empty until it lands.
    Loading,
    /// The drawing's items, grouped by `ModelItemType`.
    Ready(Arc<DrawingItems>),
    /// The read failed; the drawing's subtree stays empty.
    Failed(String),
}

/// A node of the browse tree.
///
/// Indices address the ready [`BackupIndex`]: a drawing by its position in
/// [`BackupIndex::drawings`], a file by its position in
/// [`BackupIndex::files`], a group by its path of child indices from the
/// group roots, a `Ref` directory by its path of directory indices, an item by
/// `(drawing, item-type group, item)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// The backup itself: manifest summary and warnings.
    Root,
    /// The "backup files" group.
    Files,
    /// One file of the backup root.
    File(usize),
    /// One member of an archive, `(file, member)`.
    FileMember(usize, usize),
    /// The "plant groups" group.
    Groups,
    /// One group of the plant-group tree, as the child indices from the root.
    Group(Vec<usize>),
    /// One drawing, by its index into [`BackupIndex::drawings`].
    Drawing(usize),
    /// One `ModelItemType` group of a drawing, `(drawing, group)`.
    ItemType(usize, usize),
    /// One model item, `(drawing, group, item)`.
    Item(usize, usize, usize),
    /// The "reference data" group.
    Ref,
    /// One directory of the `Ref` tree, as the child indices from its root.
    RefDir(Vec<usize>),
    /// One file of the `Ref` tree, `(directory path, file)`.
    RefFile(Vec<usize>, usize),
    /// One node of a branch a [`section`] contributes: the section's slot in
    /// the registry, and the address the section gives the node.
    Section(usize, Vec<usize>),
}

/// What the user did in the panel.
///
/// No `PartialEq`: two loads carry whole indexes, which the panel never
/// compares — the messages only travel one way.
#[derive(Debug, Clone)]
pub enum PidBackupMsg {
    /// The "open…" button: ask for a backup zip or folder.
    Open,
    /// The source dialog came back; `None` when the user cancelled.
    SourcePicked(Option<PathBuf>),
    /// The index finished building, or failed to.
    Loaded(Result<Arc<BackupIndex>, String>),
    /// One drawing's items finished reading, or failed to.
    ItemsLoaded(usize, Result<Arc<DrawingItems>, String>),
    /// What a section asked to have read about `node` arrived, or failed to.
    /// `generation` is the panel's when the read began.
    SectionLoaded {
        generation: u64,
        section: usize,
        node: NodeId,
        result: Result<SectionData, String>,
    },
    /// The expand arrow of a node.
    Toggle(NodeId),
    /// A click on a row: select it.
    Select(NodeId),
    /// A double click on a row: open the drawing or the symbol file it
    /// names.
    Activate(NodeId),
    /// The "refresh" button: rebuild the index from the current source.
    Refresh,
    /// The "close backup" button: forget the index and go back to idle.
    Close,
    /// The search box: narrow the tree to the drawings and the reference
    /// files whose name matches.
    Filter(String),
    /// Go to the node at this path of labels (`a > b > c`), unfolding the
    /// levels on the way and selecting it: `PIDBACKUPOPEN Select`.
    Goto(String),
    /// The table in the detail half: a title clicked, the rows scrolled.
    Grid(grid::GridMsg),
    /// The bar between the tree and the details was dragged.
    SplitDragged(iced::widget::pane_grid::ResizeEvent),
    /// Give the tree this share of the height (0 to 1): `PIDBACKUPOPEN
    /// Split`.
    SetSplit(f32),
    /// A beat of the progress clock ([`Effect::Tick`]): the panel is drawn
    /// anew, and looks whether there is still something to count.
    Progress,
    /// The restore cache: measured, cleared, moved to another folder.
    Cache(cache::CacheMsg),
    /// The tags the backup's database gives its plant items were read, or
    /// were not: for the drawings opened from the panel, whose symbols take
    /// them. `generation` is the panel's when the read began.
    TagsRead {
        generation: u64,
        result: Result<Arc<HashMap<String, String>>, String>,
    },
    /// The symbol file at `source` was read to be opened as a drawing
    /// ([`Effect::OpenSymbol`]), or was not; `name` is what the symbol is
    /// called. It carries no generation: what was asked for is shown
    /// whatever the panel did since, as a drawing opened from it is.
    SymbolRead {
        source: SymbolSource,
        name: String,
        result: Result<Box<SymbolDrawing>, String>,
    },
}

/// What the application should do after [`update`] applied a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Nothing to do: the state change is the whole answer.
    None,
    /// Read the items of drawing `usize` (index into
    /// [`BackupIndex::drawings`]) from the `.pid` at the path.
    LoadItems(usize, PathBuf),
    /// Open the `.pid` at the path in a tab.
    OpenDrawing(PathBuf),
    /// Open the symbol file at `source` as a drawing of its own, in a tab
    /// called `name`: read it off the UI thread and come back with
    /// [`PidBackupMsg::SymbolRead`].
    OpenSymbol { source: SymbolSource, name: String },
    /// Build the index of this source.
    Load(PathBuf),
    /// Rebuild the index from the current source.
    Refresh(PathBuf),
    /// Ask the user for a backup source.
    PickSource,
    /// Read what section `section` wants to know about `node`, off the UI
    /// thread, and come back with [`PidBackupMsg::SectionLoaded`] carrying
    /// the same `generation`.
    LoadSection {
        generation: u64,
        section: usize,
        node: NodeId,
    },
    /// Measure the restore cache in `root` off the UI thread, `current`
    /// being the restore the open backup uses, and come back with
    /// [`CacheMsg::Measured`] carrying the same `asked`.
    MeasureCache {
        asked: u64,
        root: PathBuf,
        current: Option<PathBuf>,
    },
    /// Remove every restore in `root` but `current`, off the UI thread, and
    /// come back with [`CacheMsg::Measured`] carrying the same `asked`.
    ClearCache {
        asked: u64,
        root: PathBuf,
        current: Option<PathBuf>,
    },
    /// Ask the user for the folder the restore cache is to be kept in, and
    /// come back with [`CacheMsg::FolderPicked`].
    PickCacheFolder,
    /// Wait a moment ([`tick`]) and send [`PidBackupMsg::Progress`]: the
    /// clock that keeps the panel drawn while a section counts its way
    /// through something long.
    Tick,
    /// Several of the above at once.
    Batch(Vec<Effect>),
}

/// How long a beat of the progress clock is.
const TICK: std::time::Duration = std::time::Duration::from_millis(250);

/// One beat of the progress clock: what the application waits out for
/// [`Effect::Tick`].
pub async fn tick() {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("ocs-pid-backup-tick".into())
        .spawn(move || {
            std::thread::sleep(TICK);
            let _ = sender.send(());
        });
    match spawned {
        Ok(_) => {
            let _ = receiver.await;
        }
        // No thread to sleep on: the beat is still a beat, or the clock
        // would spin.
        Err(_) => std::thread::sleep(TICK),
    }
}

/// One row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The node the row stands for.
    pub id: NodeId,
    /// Indentation level: 0 is the backup itself.
    pub depth: usize,
    /// The text the row shows.
    pub label: String,
    /// Whether the row has an expand arrow.
    pub expandable: bool,
    /// Whether the row's subtree is open.
    pub expanded: bool,
    /// Whether the row is greyed and cannot be activated (a missing drawing).
    pub disabled: bool,
}

/// The panel's state.
#[derive(Debug, Clone, Default)]
pub struct PidBackupBrowser {
    /// Loading state of the backup itself.
    pub load: LoadState,
    /// Why the last load failed, kept beside the idle state so the panel can
    /// say what went wrong after it goes back to its hint.
    pub error: Option<String>,
    /// The nodes the user opened.
    pub expanded: HashSet<NodeId>,
    /// The row the user selected.
    pub selected: Option<NodeId>,
    /// One drawing's items, by index into [`BackupIndex::drawings`].
    pub items: HashMap<usize, ItemsState>,
    /// The search box's text.
    pub filter: String,
    /// What the sections had read off the UI thread, by section slot and by
    /// the node the data is about.
    pub section_data: HashMap<(usize, NodeId), SectionLoad>,
    /// Counts the backups the panel has let go of. A read carries the count
    /// it began under, so one that outlives its backup is recognised.
    pub generation: u64,
    /// The sections this panel draws.
    sections: section::Registry,
    /// A walk to a node that is waiting for a level to be read.
    goto: Option<goto::Goto>,
    /// How the last walk ended, until the command line has reported it.
    goto_outcome: Option<goto::Outcome>,
    /// Sort and scroll of the table in the selected node's details.
    grid: grid::GridState,
    /// How the height is shared between the tree and the details. It is the
    /// user's: no backup coming or going changes it.
    split: split::Split,
    /// The selection moved and the tree has not been scrolled to it yet.
    reveal: bool,
    /// The progress clock is going: a beat is on its way. Not the last
    /// backup's to forget: the beat arrives all the same, and settles it.
    ticking: bool,
    /// The restore cache: where it is and what it was last seen to hold.
    /// The folder is the user's preference, not the backup's.
    cache: cache::CacheState,
    /// The most recent backup source path attempted or loaded, retained
    /// across a failed load so the screen can show which path failed.
    pub last_source: Option<PathBuf>,
    /// When the current `LoadState::Loading` began, for elapsed-second UI.
    pub loading_started_at: Option<iced::time::Instant>,
}

/// Whether the selection moved since this was last asked: the application
/// then has the tree scroll to it ([`reveal_selected`]).
pub fn take_reveal(state: &mut PidBackupBrowser) -> bool {
    std::mem::take(&mut state.reveal)
}

impl PidBackupBrowser {
    /// The ready index, when there is one.
    pub fn index(&self) -> Option<&Arc<BackupIndex>> {
        match &self.load {
            LoadState::Ready(index) => Some(index),
            _ => None,
        }
    }

    /// Forget everything the last backup put in the panel.
    fn clear(&mut self) {
        self.expanded.clear();
        self.selected = None;
        self.items.clear();
        self.filter.clear();
        self.section_data.clear();
        self.generation += 1;
        self.goto = None;
        self.goto_outcome = None;
        self.grid = grid::GridState::default();
        self.reveal = false;
        self.cache.forget();
    }

    /// How the height is shared between the tree and the details.
    pub fn split(&self) -> &split::Split {
        &self.split
    }

    /// Browse with the test section instead of the real ones.
    #[cfg(test)]
    pub(crate) fn use_fake_sections(&mut self) {
        self.sections = section::Registry(section::fake::FAKE_SECTIONS);
    }

    /// How long the current `LoadState::Loading` has been running at `now`.
    pub fn loading_elapsed(&self, now: iced::time::Instant) -> Option<std::time::Duration> {
        if !matches!(self.load, LoadState::Loading(_)) {
            return None;
        }
        self.loading_started_at
            .map(|started| now.saturating_duration_since(started))
    }

    /// Source path of the active or failed load (`None` when idle without error).
    pub fn active_or_failed_source(&self) -> Option<&std::path::Path> {
        match &self.load {
            LoadState::Loading(source) => Some(source.as_path()),
            LoadState::Ready(index) => Some(index.source.as_path()),
            LoadState::Idle if self.error.is_some() => self.last_source.as_deref(),
            LoadState::Idle => None,
        }
    }
}

/// Chinese display name of a plant group in the new `backup` screen (S2).
pub fn screen_group_label(name: &str) -> &str {
    match name {
        crate::io::pid_backup_browse::UNREGISTERED_GROUP => "未登记图纸",
        crate::io::pid_backup_browse::UNGROUPED => "未分组",
        other => other,
    }
}

/// Total drawings under `node` (its own `drawings` plus every descendant's).
pub fn group_total_drawings(node: &GroupNode) -> usize {
    node.drawings.len()
        + node
            .children
            .iter()
            .map(group_total_drawings)
            .sum::<usize>()
}

fn find_group_node<'a>(nodes: &'a [GroupNode], path: &str) -> Option<&'a GroupNode> {
    for node in nodes {
        if node.path == path {
            return Some(node);
        }
        if let Some(found) = find_group_node(&node.children, path) {
            return Some(found);
        }
    }
    None
}

fn collect_group_subtree(node: &GroupNode, out: &mut HashSet<usize>) {
    out.extend(node.drawings.iter().copied());
    for child in &node.children {
        collect_group_subtree(child, out);
    }
}

/// Drawing indices in `index` matching `selected_group` (its subtree when
/// `Some`) and case-insensitive substring `query` on the drawing's name.
pub fn filter_drawings(
    index: &BackupIndex,
    selected_group: Option<&str>,
    query: &str,
) -> Vec<usize> {
    let needle = query.trim().to_lowercase();
    let subtree = selected_group.and_then(|group_path| {
        let node = find_group_node(&index.plant_groups, group_path)?;
        let mut set = HashSet::new();
        collect_group_subtree(node, &mut set);
        Some(set)
    });
    let prefix = selected_group.map(|group_path| format!("{group_path} / "));

    index
        .drawings
        .iter()
        .enumerate()
        .filter(|(idx, entry)| {
            let group_ok = match (&subtree, selected_group, &prefix) {
                (Some(set), _, _) => set.contains(idx),
                (None, Some(group_path), Some(pfx)) => {
                    entry.group_path == group_path || entry.group_path.starts_with(pfx)
                }
                _ => true,
            };
            group_ok && (needle.is_empty() || entry.name.to_lowercase().contains(&needle))
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// `(item_type, count)` pairs for `items`, in `items.groups` order.
pub fn item_type_counts(items: &DrawingItems) -> Vec<(&str, usize)> {
    items
        .groups
        .iter()
        .map(|group| (group.item_type.as_str(), group.items.len()))
        .collect()
}

fn item_matches_query(item: &crate::io::pid_backup_browse::DrawingItem, needle: &str) -> bool {
    item.label.to_lowercase().contains(needle)
        || item.uid.to_lowercase().contains(needle)
        || item.item_type.to_lowercase().contains(needle)
        || item
            .class
            .as_deref()
            .is_some_and(|cls| cls.to_lowercase().contains(needle))
        || item.attributes.iter().any(|attr| {
            attr.name.to_lowercase().contains(needle) || attr.value.to_lowercase().contains(needle)
        })
}

/// Items of `items` matching optional exact `type_filter` and case-insensitive
/// `query` (against label, UID, type, class, or attribute name/value).
pub fn filter_drawing_items<'a>(
    items: &'a DrawingItems,
    type_filter: Option<&str>,
    query: &str,
) -> Vec<&'a crate::io::pid_backup_browse::DrawingItem> {
    let needle = query.trim().to_lowercase();
    let mut out = Vec::new();
    for group in &items.groups {
        if type_filter.is_some_and(|want| group.item_type != want) {
            continue;
        }
        for item in &group.items {
            if needle.is_empty() || item_matches_query(item, &needle) {
                out.push(item);
            }
        }
    }
    out
}

/// [`section::Section::load`] of one section on a worker thread: what the
/// application runs for [`Effect::LoadSection`]. The future owns what it
/// needs of `state`, so it outlives the borrow.
pub fn load_section_in_background(
    state: &PidBackupBrowser,
    section: usize,
    node: NodeId,
) -> impl std::future::Future<Output = Result<SectionData, String>> + Send + 'static {
    let index = state.index().cloned();
    let sections = state.sections;
    async move {
        let Some(index) = index else {
            return Err(t!("No backup is open.").into_owned());
        };
        section::load_in_background(sections, index, section, node).await
    }
}

/// The rows the panel shows, in display order.
///
/// Only the nodes [`PidBackupBrowser::expanded`] holds are walked, so a
/// collapsed `Ref` tree of 645 files costs one row. A non-empty filter keeps
/// what it names and what stands above that, shown open whatever the user
/// folded: the drawings whose name contains it (case-insensitively) under
/// their groups, and the files of the `Ref` tree whose name does under their
/// directories ([`push_ref_matches`]). The backup files drop out: their
/// names are not searched, and the members of the reference archives would
/// show every file found a second time.
pub fn visible_rows(state: &PidBackupBrowser) -> Vec<Row> {
    let Some(index) = state.index() else {
        return Vec::new();
    };
    let filter = section::filter_of(state);
    let filtering = !filter.is_empty();
    let mut rows = Vec::new();
    rows.push(Row {
        id: NodeId::Root,
        depth: 0,
        label: root_label(index),
        // The root always shows its three groups: folding them would leave
        // the panel with nothing to click.
        expandable: true,
        expanded: true,
        disabled: false,
    });
    if !filtering {
        push_files(state, index, &mut rows);
    }
    push_groups(state, index, &filter, filtering, &mut rows);
    if filtering {
        push_ref_matches(index, &filter, &mut rows);
    } else {
        push_ref(state, index, &mut rows);
    }
    section::splice(state, index, &filter, rows)
}

/// Apply `msg` to `state` and say what the application should do next.
///
/// Pure: no file is read, no task is started — the caller turns the returned
/// [`Effect`] into work.
pub fn update(state: &mut PidBackupBrowser, msg: PidBackupMsg) -> Effect {
    match msg {
        PidBackupMsg::Open => Effect::PickSource,
        PidBackupMsg::SourcePicked(None) => Effect::None,
        PidBackupMsg::SourcePicked(Some(source)) => {
            state.error = None;
            state.clear();
            state.last_source = Some(source.clone());
            state.loading_started_at = Some(iced::time::Instant::now());
            state.load = LoadState::Loading(source.clone());
            Effect::Load(source)
        }
        PidBackupMsg::Loaded(Ok(index)) => {
            state.error = None;
            state.clear();
            state.last_source = Some(index.source.clone());
            state.loading_started_at = None;
            state.load = LoadState::Ready(index);
            Effect::None
        }
        PidBackupMsg::Loaded(Err(error)) => {
            state.clear();
            state.loading_started_at = None;
            state.load = LoadState::Idle;
            state.error = Some(error);
            Effect::None
        }
        // What a walk to a node was waiting for may be what just arrived.
        PidBackupMsg::ItemsLoaded(drawing, Ok(items)) => {
            state.items.insert(drawing, ItemsState::Ready(items));
            goto::step(state)
        }
        PidBackupMsg::ItemsLoaded(drawing, Err(error)) => {
            state.items.insert(drawing, ItemsState::Failed(error));
            goto::step(state)
        }
        PidBackupMsg::SectionLoaded {
            generation,
            section,
            node,
            result,
        } => {
            section::loaded(state, generation, section, node, result);
            goto::step(state)
        }
        PidBackupMsg::Goto(path) => goto::start(state, &path),
        PidBackupMsg::Grid(msg) => {
            let mut grid = std::mem::take(&mut state.grid);
            grid::update(&mut grid, msg, || selected_table(state));
            state.grid = grid;
            Effect::None
        }
        // The tree's own answer first, then whatever a section now wants
        // read about the node.
        PidBackupMsg::Toggle(id) => {
            let own = toggle(state, id.clone());
            section::both(own, section::after(state, section::EventKind::Toggled, &id))
        }
        PidBackupMsg::SplitDragged(event) => {
            state.split.dragged(event);
            Effect::None
        }
        PidBackupMsg::SetSplit(share) => {
            state.split.set_tree_share(share);
            Effect::None
        }
        PidBackupMsg::Select(id) => {
            // Another node's table starts unsorted and from its top.
            if state.selected.as_ref() != Some(&id) {
                state.grid = grid::GridState::default();
            }
            state.selected = Some(id.clone());
            // Wherever the selection came from, the tree shows it.
            state.reveal = true;
            let effect = section::after(state, section::EventKind::Selected, &id);
            // The backup's own details say what the restore cache holds:
            // measured each time they are opened, as restores come and go.
            if cache::shows(&id) {
                section::both(effect, cache::measure(state))
            } else {
                effect
            }
        }
        PidBackupMsg::Activate(id) => {
            let own = activate(state, id.clone());
            section::both(own, section::after(state, section::EventKind::Activated, &id))
        }
        PidBackupMsg::Refresh => match &state.load {
            LoadState::Ready(index) => Effect::Refresh(index.source.clone()),
            LoadState::Loading(source) => {
                state.loading_started_at = Some(iced::time::Instant::now());
                Effect::Refresh(source.clone())
            }
            LoadState::Idle => Effect::None,
        },
        PidBackupMsg::Close => {
            state.load = LoadState::Idle;
            state.error = None;
            state.last_source = None;
            state.loading_started_at = None;
            state.clear();
            Effect::None
        }
        PidBackupMsg::Filter(text) => {
            state.filter = text;
            Effect::None
        }
        PidBackupMsg::Cache(msg) => cache::update(state, msg),
        // Nothing of the panel's: the application puts the tags on the
        // drawings it opened from here.
        PidBackupMsg::TagsRead { .. } => Effect::None,
        // Nor this: the application shows the symbol in a tab.
        PidBackupMsg::SymbolRead { .. } => Effect::None,
        // The message itself is what draws the panel anew; the clock goes
        // on for as long as something counts.
        PidBackupMsg::Progress => {
            state.ticking = section::in_progress(state);
            if state.ticking {
                Effect::Tick
            } else {
                Effect::None
            }
        }
    }
}

/// The root row's text: the backup's own name when the manifest states one.
fn root_label(index: &BackupIndex) -> String {
    index
        .manifest
        .as_ref()
        .and_then(|manifest| manifest.name.clone())
        .unwrap_or_else(|| t!("Untitled backup").into_owned())
}

/// A plant group's name as the tree shows it.
///
/// The index files the drawings the drawing table does not mention under
/// [`crate::io::pid_backup_browse::UNREGISTERED_GROUP`] and the ones with no
/// folder at all under [`crate::io::pid_backup_browse::UNGROUPED`]; both are
/// fixed labels the tree translates, every other name is the plant's own.
fn group_label(name: &str) -> String {
    match name {
        crate::io::pid_backup_browse::UNREGISTERED_GROUP => t!("Unregistered").into_owned(),
        crate::io::pid_backup_browse::UNGROUPED => t!("(ungrouped)").into_owned(),
        other => other.to_string(),
    }
}

/// A group's path as the details show it. The two groups the index makes
/// up have their label for a path, and it is translated as their rows are;
/// a plant's own path is shown as it is.
pub(crate) fn group_path_label(path: &str) -> String {
    group_label(path)
}

/// A backup file's row: the name it is stored under and, when the manifest
/// says what it was on the plant's server, that (`RefData~4~680 —
/// Ref\rules.rul`). The stored name alone tells no one what the file is.
fn file_label(file: &crate::io::pid_backup_browse::BackupFile) -> String {
    match file
        .record
        .as_ref()
        .filter(|record| !record.original.is_empty())
    {
        Some(record) => format!("{} — {}", file.name, record.original),
        None => file.name.clone(),
    }
}

fn push_files(state: &PidBackupBrowser, index: &BackupIndex, rows: &mut Vec<Row>) {
    let expanded = state.expanded.contains(&NodeId::Files);
    rows.push(Row {
        id: NodeId::Files,
        depth: 1,
        label: t!("Backup files").into_owned(),
        expandable: !index.files.is_empty(),
        expanded,
        disabled: false,
    });
    if !expanded {
        return;
    }
    for (at, file) in index.files.iter().enumerate() {
        let id = NodeId::File(at);
        let open = state.expanded.contains(&id);
        rows.push(Row {
            id,
            depth: 2,
            label: file_label(file),
            expandable: !file.members.is_empty(),
            expanded: open,
            disabled: false,
        });
        if !open {
            continue;
        }
        for (member, name) in file.members.iter().enumerate() {
            rows.push(Row {
                id: NodeId::FileMember(at, member),
                depth: 3,
                label: name.clone(),
                expandable: false,
                expanded: false,
                disabled: false,
            });
        }
    }
}

fn push_groups(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    filter: &str,
    filtering: bool,
    rows: &mut Vec<Row>,
) {
    let expanded = state.expanded.contains(&NodeId::Groups);
    rows.push(Row {
        id: NodeId::Groups,
        depth: 1,
        label: t!("Plant groups").into_owned(),
        expandable: !index.plant_groups.is_empty(),
        expanded: expanded || filtering,
        disabled: false,
    });
    if !(expanded || filtering) {
        return;
    }
    for (at, node) in index.plant_groups.iter().enumerate() {
        push_group(state, index, node, &[at], 2, filter, filtering, rows);
    }
}

fn push_group(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    node: &GroupNode,
    path: &[usize],
    depth: usize,
    filter: &str,
    filtering: bool,
    rows: &mut Vec<Row>,
) {
    if filtering && !group_has_match(index, node, filter) {
        return;
    }
    let id = NodeId::Group(path.to_vec());
    let open = state.expanded.contains(&id) || filtering;
    rows.push(Row {
        id,
        depth,
        label: group_label(&node.name),
        expandable: !node.children.is_empty() || !node.drawings.is_empty(),
        expanded: open,
        disabled: false,
    });
    if !open {
        return;
    }
    for (at, child) in node.children.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(at);
        push_group(
            state,
            index,
            child,
            &child_path,
            depth + 1,
            filter,
            filtering,
            rows,
        );
    }
    for &drawing in &node.drawings {
        push_drawing(state, index, drawing, depth + 1, filter, filtering, rows);
    }
}

fn push_drawing(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    drawing: usize,
    depth: usize,
    filter: &str,
    filtering: bool,
    rows: &mut Vec<Row>,
) {
    let Some(entry) = index.drawings.get(drawing) else {
        return;
    };
    if filtering && !entry.name.to_lowercase().contains(filter) {
        return;
    }
    let items = state.items.get(&drawing);
    let id = NodeId::Drawing(drawing);
    let open = state.expanded.contains(&id);
    rows.push(Row {
        id,
        depth,
        label: entry.name.clone(),
        // The arrow is what starts the read of a drawing's items, so a
        // drawing with a file has it before any item is known. It goes only
        // once the read has shown there is nothing to unfold: no items, or a
        // file that could not be read (its details say why).
        expandable: entry.pid_path.is_some()
            && match items {
                None | Some(ItemsState::Loading) => true,
                Some(ItemsState::Ready(items)) => !items.groups.is_empty(),
                Some(ItemsState::Failed(_)) => false,
            },
        expanded: open,
        disabled: entry.pid_path.is_none(),
    });
    if !open {
        return;
    }
    let Some(ItemsState::Ready(items)) = items else {
        return;
    };
    for (at, group) in items.groups.iter().enumerate() {
        let id = NodeId::ItemType(drawing, at);
        let open = state.expanded.contains(&id);
        rows.push(Row {
            id,
            depth: depth + 1,
            label: format!("{} ({})", group.item_type, group.items.len()),
            expandable: !group.items.is_empty(),
            expanded: open,
            disabled: false,
        });
        if !open {
            continue;
        }
        for (item, entry) in group.items.iter().enumerate() {
            rows.push(Row {
                id: NodeId::Item(drawing, at, item),
                depth: depth + 2,
                label: entry.label.clone(),
                expandable: false,
                expanded: false,
                disabled: false,
            });
        }
    }
}

/// Whether any drawing in `node`'s subtree matches `filter`.
fn group_has_match(index: &BackupIndex, node: &GroupNode, filter: &str) -> bool {
    node.drawings.iter().any(|drawing| {
        index
            .drawings
            .get(*drawing)
            .is_some_and(|entry| entry.name.to_lowercase().contains(filter))
    }) || node
        .children
        .iter()
        .any(|child| group_has_match(index, child, filter))
}

fn push_ref(state: &PidBackupBrowser, index: &BackupIndex, rows: &mut Vec<Row>) {
    let expanded = state.expanded.contains(&NodeId::Ref);
    let tree = &index.ref_tree;
    rows.push(Row {
        id: NodeId::Ref,
        depth: 1,
        label: t!("Reference data").into_owned(),
        expandable: !(tree.dirs.is_empty() && tree.files.is_empty()),
        expanded,
        disabled: false,
    });
    if expanded {
        push_ref_children(state, tree, &[], 2, rows);
    }
}

fn push_ref_children(
    state: &PidBackupBrowser,
    node: &RefNode,
    path: &[usize],
    depth: usize,
    rows: &mut Vec<Row>,
) {
    for (at, dir) in node.dirs.iter().enumerate() {
        let mut child_path = path.to_vec();
        child_path.push(at);
        let id = NodeId::RefDir(child_path.clone());
        let open = state.expanded.contains(&id);
        rows.push(Row {
            id,
            depth,
            label: dir.name.clone(),
            expandable: !(dir.dirs.is_empty() && dir.files.is_empty()),
            expanded: open,
            disabled: false,
        });
        if open {
            push_ref_children(state, dir, &child_path, depth + 1, rows);
        }
    }
    for (at, file) in node.files.iter().enumerate() {
        rows.push(Row {
            id: NodeId::RefFile(path.to_vec(), at),
            depth,
            label: file.name.clone(),
            expandable: false,
            expanded: false,
            disabled: false,
        });
    }
}

/// The `Ref` tree under a search: the files whose name contains `filter`
/// (lower case already, as [`section::filter_of`] gives it), each under the
/// directories above it, and the "reference data" row above them all. What
/// stays is shown open whatever the user folded, and
/// [`PidBackupBrowser::expanded`] is not touched, so the tree is the user's
/// again once the search is cleared. With no such file nothing is added, not
/// the "reference data" row either: a search for a drawing shows drawings
/// alone.
///
/// Only a file's own name is compared, as only a drawing's is: a directory
/// called after the search does not bring its files along.
fn push_ref_matches(index: &BackupIndex, filter: &str, rows: &mut Vec<Row>) {
    let at = rows.len();
    rows.push(Row {
        id: NodeId::Ref,
        depth: 1,
        label: t!("Reference data").into_owned(),
        expandable: true,
        expanded: true,
        disabled: false,
    });
    if !push_ref_matches_under(&index.ref_tree, &mut Vec::new(), 2, filter, rows) {
        rows.truncate(at);
    }
}

/// The rows of [`push_ref_matches`] under `node`, `path` being the way to
/// it; whether there are any.
///
/// One walk of the tree: a directory's row goes in before its subtree is
/// looked at and comes out again when the subtree added nothing, so a name
/// is compared once however deep it lies.
fn push_ref_matches_under(
    node: &RefNode,
    path: &mut Vec<usize>,
    depth: usize,
    filter: &str,
    rows: &mut Vec<Row>,
) -> bool {
    let mut any = false;
    for (at, dir) in node.dirs.iter().enumerate() {
        let row = rows.len();
        path.push(at);
        rows.push(Row {
            id: NodeId::RefDir(path.clone()),
            depth,
            label: dir.name.clone(),
            expandable: true,
            expanded: true,
            disabled: false,
        });
        if push_ref_matches_under(dir, path, depth + 1, filter, rows) {
            any = true;
        } else {
            rows.truncate(row);
        }
        path.pop();
    }
    for (at, file) in node.files.iter().enumerate() {
        if file.name.to_lowercase().contains(filter) {
            rows.push(Row {
                id: NodeId::RefFile(path.clone(), at),
                depth,
                label: file.name.clone(),
                expandable: false,
                expanded: false,
                disabled: false,
            });
            any = true;
        }
    }
    any
}

/// Fold or unfold a node; a drawing's first unfold reads its items.
fn toggle(state: &mut PidBackupBrowser, id: NodeId) -> Effect {
    if id == NodeId::Root {
        return Effect::None;
    }
    if let NodeId::Drawing(drawing) = id {
        let Some(path) = state
            .index()
            .and_then(|index| index.drawings.get(drawing))
            .and_then(|entry| entry.pid_path.clone())
        else {
            // A drawing with no file has nothing to read.
            return Effect::None;
        };
        if state.items.contains_key(&drawing) {
            flip(state, NodeId::Drawing(drawing));
            return Effect::None;
        }
        state.items.insert(drawing, ItemsState::Loading);
        state.expanded.insert(NodeId::Drawing(drawing));
        return Effect::LoadItems(drawing, path);
    }
    flip(state, id);
    Effect::None
}

fn flip(state: &mut PidBackupBrowser, id: NodeId) {
    if !state.expanded.remove(&id) {
        state.expanded.insert(id);
    }
}

/// Open a drawing, or a symbol file as a drawing of its own; anything else,
/// and a drawing without a file, does nothing.
///
/// A symbol file is asked for whether or not it was read for its details:
/// a double click does not wait for the thumbnail, and one that draws
/// nothing says so when it has been read to be opened.
fn activate(state: &PidBackupBrowser, id: NodeId) -> Effect {
    let Some(index) = state.index() else {
        return Effect::None;
    };
    match id {
        NodeId::Drawing(drawing) => index
            .drawings
            .get(drawing)
            .and_then(|entry| entry.pid_path.clone())
            .map_or(Effect::None, Effect::OpenDrawing),
        id => symbol_preview::opens(index, &id).map_or(Effect::None, |(source, name)| {
            Effect::OpenSymbol { source, name }
        }),
    }
}

impl PidBackupMsg {
    pub(crate) fn message(self) -> Message {
        Message::PidBackup(self)
    }
}

// ── View ──────────────────────────────────────────────────────────────────

/// `PosReport` id of the panel body, so a client can locate it on screen.
pub const PANEL_BOUNDS_ID: &str = "pid-backup-panel";

/// `PosReport` id prefix of the visible tree rows.
///
/// The suffix is the row's index among the visible rows: the tree is rebuilt
/// from [`visible_rows`] every frame, so a node's own identity (`NodeId`) can
/// name a row that is not on screen while its index always names the row a
/// screenshot shows.
pub const ROW_BOUNDS_PREFIX: &str = "pid-backup-row-";

/// Indentation one tree level costs, in pixels.
const INDENT: f32 = 12.0;

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.6)),
    }
}

/// Size of the text in the detail half.
const DETAIL_TEXT: f32 = 10.0;

/// The bar that scrolls a half of the panel. It takes its own strip beside
/// what it scrolls, a hair clear of it, for as long as it is there: laid
/// over the content it hid the end of every line that reached the edge --
/// the last characters of a long path, of the SPID.
fn scroll_bar() -> scrollable::Scrollbar {
    scrollable::Scrollbar::default().spacing(2)
}

/// Space between a key and its value.
const KEY_GAP: f32 = 6.0;

/// Width of the key column of the detail half in a panel `width` wide: a
/// good third of it, so a wider (floated) panel gives long keys more room.
pub(crate) fn key_column_width(width: f32) -> f32 {
    (width * 0.36).clamp(96.0, 240.0)
}

/// Whether `key` is too long for a key column `key_width` wide and gets a
/// line to itself, its value below it.
pub(crate) fn key_takes_its_own_line(key: &str, key_width: f32) -> bool {
    crate::ui::style::text_metrics::width(key, DETAIL_TEXT, false) > key_width
}

/// One key/value line of the detail table.
///
/// The value always starts at the value column, so the values line up. A key
/// that fits the key column sits beside its value; one that does not (the
/// published attributes are `Interface.Name`) takes the full width above
/// it. Either way both texts wrap inside their boxes: nothing is drawn over
/// anything else, however narrow the panel.
fn kv<'a>(key: String, value: String, key_width: f32) -> Element<'a, Message> {
    let wrap = iced::widget::text::Wrapping::WordOrGlyph;
    let stacked = key_takes_its_own_line(&key, key_width);
    let key = text(key).size(DETAIL_TEXT).style(muted).wrapping(wrap);
    let value = text(value).size(DETAIL_TEXT).width(Fill).wrapping(wrap);
    if stacked {
        column![
            key.width(Fill),
            row![Space::new().width(Length::Fixed(key_width + KEY_GAP)), value],
        ]
        .spacing(1)
        .into()
    } else {
        row![key.width(Length::Fixed(key_width)), value]
            .spacing(KEY_GAP)
            .align_y(iced::Top)
            .into()
    }
}

/// A byte count as the panel shows it.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn group_at<'a>(index: &'a BackupIndex, path: &[usize]) -> Option<&'a GroupNode> {
    let (first, rest) = path.split_first()?;
    let mut node = index.plant_groups.get(*first)?;
    for at in rest {
        node = node.children.get(*at)?;
    }
    Some(node)
}

fn ref_at<'a>(node: &'a RefNode, path: &[usize]) -> Option<&'a RefNode> {
    let mut node = node;
    for at in path {
        node = node.dirs.get(*at)?;
    }
    Some(node)
}

fn group_drawing_count(node: &GroupNode) -> usize {
    node.drawings.len()
        + node
            .children
            .iter()
            .map(group_drawing_count)
            .sum::<usize>()
}

/// How a tree row is drawn: the selected one on the palette's weak primary,
/// as the sheet set and the layer list mark theirs.
fn row_style(theme: &Theme, selected: bool) -> container::Style {
    let weak = theme.palette().primary.weak;
    container::Style {
        background: selected.then_some(iced::Background::Color(weak.color)),
        text_color: selected.then_some(weak.text),
        ..container::Style::default()
    }
}

/// One row of the tree: its indentation, its expand arrow and its label.
fn tree_row<'a>(row: &Row, at: usize, selected: bool) -> Element<'a, Message> {
    let arrow: Element<'a, Message> = if row.expandable {
        let icon = if row.expanded {
            crate::ui::icons::themed_arrow_down(10.0)
        } else {
            crate::ui::icons::themed_arrow_right(10.0)
        };
        button(icon)
            .on_press(PidBackupMsg::Toggle(row.id.clone()).message())
            .style(button::text)
            .padding(0)
            .into()
    } else {
        Space::new().width(Length::Fixed(12.0)).into()
    };
    let label = if row.disabled {
        text(row.label.clone()).size(11).style(muted)
    } else {
        text(row.label.clone()).size(11)
    };
    let line = row![
        Space::new().width(Length::Fixed(row.depth as f32 * INDENT)),
        arrow,
        label,
    ]
    .spacing(2)
    .align_y(iced::Center);
    // A press selects; the second press of a double click activates. The row
    // stays clickable when it is greyed (its details still explain why), and
    // `update` is what refuses to open a drawing with no file.
    let body = container(line)
        .width(Fill)
        .padding([1, 4])
        .style(move |theme| row_style(theme, selected));
    let body = if selected {
        body.id(iced::widget::Id::new(split::SELECTED_ROW))
    } else {
        body
    };
    let area = mouse_area(body)
    .interaction(iced::mouse::Interaction::Pointer)
    .on_press(PidBackupMsg::Select(row.id.clone()).message())
    .on_double_click(PidBackupMsg::Activate(row.id.clone()).message());
    crate::ui::wrap_bar::PosReport::owned(format!("{ROW_BOUNDS_PREFIX}{at}"), area).into()
}

/// One line of the detail half, before it is laid out.
///
/// The tests project these to `(key, value)` pairs (`detail_rows`), so the
/// content is asserted without a window; [`details`] draws each kind its own
/// way.
enum DetailLine {
    /// A key/value field.
    Field(String, String),
    /// A plain muted line: a warning, or "registered but missing".
    Marker(String),
    /// A section heading, set off from the fields above it.
    Heading(String),
    /// The "open drawing" button for a drawing that has a file.
    OpenDrawing(String),
    /// A button that activates a node: how a section starts something that
    /// is too slow to start by itself (a check of every drawing).
    Button(String, NodeId),
    /// A button that sends a message of the panel's own: what it does is
    /// no node's to do (clearing the restore cache).
    Action(String, PidBackupMsg),
    /// A table, drawn as a [`grid`] below the other lines. A node's details
    /// hold at most one: it takes what is left of the detail half.
    Table(std::sync::Arc<grid::Table>),
    /// What a symbol file draws, as a thumbnail among the lines.
    Sketch(std::sync::Arc<crate::io::pid_backup_browse::symbol_file::Sketch>),
}

impl DetailLine {
    /// The `(key, value)` pairs this line shows: one for every kind but a
    /// table, which gives its titles (with its row count) and then a pair
    /// per row, the first cell as the key and the others joined with ` | `.
    /// A thumbnail is a key of its own with how much it draws.
    #[cfg(test)]
    fn into_pairs(self) -> Vec<(String, String)> {
        match self {
            DetailLine::Field(key, value) => vec![(key, value)],
            DetailLine::Sketch(sketch) => vec![symbol_preview::sketch_pair(&sketch)],
            DetailLine::Marker(key)
            | DetailLine::Heading(key)
            | DetailLine::OpenDrawing(key)
            | DetailLine::Button(key, _)
            | DetailLine::Action(key, _) => {
                vec![(key, String::new())]
            }
            DetailLine::Table(table) => {
                let join = |cells: &mut dyn Iterator<Item = &str>| -> String {
                    cells.collect::<Vec<_>>().join(" | ")
                };
                let titles = join(&mut table.columns().iter().map(|column| column.title.as_str()));
                let mut pairs = vec![(titles, row_count_label(table.rows().len()))];
                for at in 0..table.rows().len() {
                    // A row shorter than the header ends where its cells do.
                    let filled = (1..table.columns().len())
                        .rev()
                        .find(|column| !table.cell(at, *column).is_empty())
                        .map_or(0, |last| last + 1);
                    let rest = join(&mut (1..filled.max(1)).map(|column| table.cell(at, column)));
                    pairs.push((table.cell(at, 0).to_string(), rest));
                }
                pairs
            }
        }
    }
}

/// "154 row(s)", above a table.
fn row_count_label(rows: usize) -> String {
    crate::tf!("{} row(s)", rows).into_owned()
}

/// The table in the details of the selected node, when they hold one.
fn selected_table(state: &PidBackupBrowser) -> Option<std::sync::Arc<grid::Table>> {
    let id = state.selected.as_ref()?;
    detail_lines(state, id).into_iter().find_map(|line| match line {
        DetailLine::Table(table) => Some(table),
        _ => None,
    })
}

/// What tells one node's table from another's, for the grid's scroll state.
fn grid_key(id: &NodeId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

/// How many lines may stand above a table at their full height. More than
/// that, and they would push the table out of the half: they get a scroll
/// bar of their own instead.
const ABOVE_TABLE_FULL: usize = 10;

/// How many lines a heading may keep beside the table it heads, itself
/// among them. A longer run scrolls with the lines above it.
const BESIDE_TABLE_MOST: usize = 6;

/// How many rows a table may have and still be given its own height and no
/// more: the lines above it are the ones that need the room then.
const SHORT_TABLE: usize = 6;

/// How the lines above a table are laid out. `headings` says of each line
/// whether it is a heading. Returns how many of the lines, from the first,
/// scroll, and whether any do.
///
/// Up to [`ABOVE_TABLE_FULL`] lines stand at their full height. More would
/// push the table out of the half, so they scroll -- all but the last
/// heading and the few lines under it, which belong with the table (a
/// report's length above its lines) and stay put right above it.
fn split_above_table(headings: &[bool]) -> (usize, bool) {
    if headings.len() <= ABOVE_TABLE_FULL {
        return (headings.len(), false);
    }
    let split = match headings.iter().rposition(|heading| *heading) {
        Some(at) if headings.len() - at <= BESIDE_TABLE_MOST => at,
        _ => headings.len(),
    };
    (split, true)
}

/// The lower half: what the selected node is.
///
/// Rendering only: the content and the shape of every line come from
/// [`detail_lines`]. The lines scroll when they outgrow the half; a table
/// among them is drawn as a grid below them, with scroll bars of its own.
/// A few lines above a table stand at their full height and the table
/// takes what they leave; many scroll in a share of the half, so the table
/// is on screen however long they are ([`split_above_table`]).
fn details<'a>(state: &'a PidBackupBrowser, width: f32) -> Element<'a, Message> {
    let hint = |label: std::borrow::Cow<'static, str>| -> Element<'a, Message> {
        text(label).size(11).style(muted).into()
    };
    // Without a backup the upper half says so, or that one is loading:
    // the details are a node's, and there is none.
    if state.index().is_none() {
        return Space::new().into();
    }
    let Some(id) = state.selected.as_ref() else {
        return hint(t!("Select a node to see its details."));
    };
    let key_width = key_column_width(width);
    let wrap = iced::widget::text::Wrapping::WordOrGlyph;
    // The lines above the table, one element each, and which are headings.
    let mut above: Vec<Element<'a, Message>> = Vec::new();
    let mut headings: Vec<bool> = Vec::new();
    let mut table = None;
    for line in detail_lines(state, id) {
        let heading = matches!(line, DetailLine::Heading(_));
        let element: Element<'a, Message> = match line {
            DetailLine::Field(key, value) => kv(key, value, key_width),
            DetailLine::Marker(note) => text(note)
                .size(DETAIL_TEXT)
                .style(muted)
                .wrapping(wrap)
                .into(),
            // Set off from the lines above it.
            DetailLine::Heading(title) => {
                column![Space::new().height(4), text(title).size(DETAIL_TEXT)]
                    .spacing(2)
                    .into()
            }
            DetailLine::OpenDrawing(label) => button(text(label).size(11))
                .on_press(PidBackupMsg::Activate(id.clone()).message())
                .padding([3, 10])
                .into(),
            DetailLine::Button(label, node) => button(text(label).size(11))
                .on_press(PidBackupMsg::Activate(node).message())
                .padding([3, 10])
                .into(),
            DetailLine::Action(label, msg) => button(text(label).size(11))
                .on_press(msg.message())
                .padding([3, 10])
                .into(),
            DetailLine::Sketch(sketch) => symbol_preview::thumbnail(sketch),
            DetailLine::Table(found) => {
                table = Some(found);
                continue;
            }
        };
        headings.push(heading);
        above.push(element);
    }
    let lines = |elements: Vec<Element<'a, Message>>| {
        iced::widget::Column::with_children(elements)
            .spacing(2)
            .width(Fill)
    };
    let scrolling = |elements: Vec<Element<'a, Message>>| {
        scrollable(lines(elements))
            .direction(scrollable::Direction::Vertical(scroll_bar()))
            .width(Fill)
    };
    let Some(table) = table else {
        return scrolling(above).height(Fill).into();
    };
    // How many rows a table of data has. The lines of a file say how many
    // they are among the fields above them.
    if table.is_sortable() {
        headings.push(false);
        above.push(
            text(row_count_label(table.rows().len()))
                .size(DETAIL_TEXT)
                .style(muted)
                .into(),
        );
    }
    let grid = grid::view(&table, &state.grid, grid_key(id), |msg| {
        PidBackupMsg::Grid(msg).message()
    });
    let (split, scrolls) = split_above_table(&headings);
    if !scrolls {
        return column![lines(above), grid]
            .spacing(2)
            .width(Fill)
            .height(Fill)
            .into();
    }
    // Many lines: they scroll, and the table keeps its place below them.
    // A short table is as tall as its rows and the lines have the rest. A
    // long table of data is what the node is about: it has two thirds of
    // the room. The lines of a text are an appendix to what stands above
    // them: they have two fifths.
    let rows = table.rows().len();
    let (lines_height, table_height) = if rows <= SHORT_TABLE {
        (
            Fill,
            // The titles, the rows, and a scroll bar under them.
            Length::Fixed(grid::HEADER_H + rows as f32 * grid::ROW_H + 14.0),
        )
    } else if table.is_sortable() {
        (Length::FillPortion(1), Length::FillPortion(2))
    } else {
        (Length::FillPortion(3), Length::FillPortion(2))
    };
    let beside = above.split_off(split);
    let mut all = column![scrolling(above).height(lines_height)]
        .spacing(2)
        .width(Fill)
        .height(Fill);
    if !beside.is_empty() {
        all = all.push(lines(beside));
    }
    all.push(container(grid).width(Fill).height(table_height))
        .into()
}

/// The lines the detail half shows for `id`, in display order.
///
/// Pure and free of `iced` types: every value is already localized and
/// formatted, so the content can be asserted without a window. An `id` the
/// index no longer holds, and an idle panel, yield no lines.
fn detail_lines(state: &PidBackupBrowser, id: &NodeId) -> Vec<DetailLine> {
    let Some(index) = state.index() else {
        return Vec::new();
    };
    let mut lines: Vec<DetailLine> = Vec::new();
    match id {
        NodeId::Root => {
            if let Some(manifest) = &index.manifest {
                for (key, value) in [
                    ("Backup name", manifest.name.clone()),
                    ("Backup version", manifest.version.clone()),
                    ("Backup type", manifest.backup_type.clone()),
                    ("Date created", manifest.date_created.clone()),
                    ("SPID", manifest.spid.clone()),
                ] {
                    if let Some(value) = value {
                        lines.push(DetailLine::Field(t!(key).into_owned(), value));
                    }
                }
            }
            lines.push(DetailLine::Field(
                t!("Backup source").into_owned(),
                index.source.display().to_string(),
            ));
            lines.push(DetailLine::Field(
                t!("Restored").into_owned(),
                index.restored.display().to_string(),
            ));
            // The marker keeps the key/value layout with an empty value: it
            // states a property of the backup, not a message.
            if index.read_only {
                lines.push(DetailLine::Field(
                    t!("Backup is read-only").into_owned(),
                    String::new(),
                ));
            }
            lines.push(DetailLine::Heading(t!("Warnings").into_owned()));
            if index.warnings.is_empty() {
                lines.push(DetailLine::Marker(t!("No warnings.").into_owned()));
            }
            for warning in &index.warnings {
                lines.push(DetailLine::Marker(warning.clone()));
            }
            cache::lines(state, index, &mut lines);
        }
        NodeId::Files => {
            lines.push(DetailLine::Field(
                t!("File count").into_owned(),
                index.files.len().to_string(),
            ));
        }
        NodeId::File(at) => {
            if let Some(file) = index.files.get(*at) {
                lines.push(DetailLine::Field(
                    t!("Backup file name").into_owned(),
                    file.name.clone(),
                ));
                if !file.format.is_empty() {
                    lines.push(DetailLine::Field(
                        t!("File format").into_owned(),
                        file.format.clone(),
                    ));
                }
                lines.push(DetailLine::Field(
                    t!("File size").into_owned(),
                    human_size(file.size),
                ));
                // Only an archive has members: a count of zero under a
                // manifest or a single reference file says nothing.
                if !file.members.is_empty() || file.format.to_ascii_lowercase().starts_with("zip") {
                    lines.push(DetailLine::Field(
                        t!("Members").into_owned(),
                        file.members.len().to_string(),
                    ));
                }
                manifest::file_record_lines(index, *at, &mut lines);
                if let Some(line) = &file.version_line {
                    lines.push(DetailLine::Field(
                        t!("Version line").into_owned(),
                        line.clone(),
                    ));
                }
                if file.missing {
                    lines.push(DetailLine::Marker(
                        t!("Registered but missing").into_owned(),
                    ));
                }
            }
        }
        NodeId::FileMember(file, member) => {
            if let Some(file) = index.files.get(*file) {
                lines.push(DetailLine::Field(
                    t!("Backup file name").into_owned(),
                    file.name.clone(),
                ));
                if let Some(name) = file.members.get(*member) {
                    lines.push(DetailLine::Field(t!("Member").into_owned(), name.clone()));
                }
            }
        }
        NodeId::Groups => {
            lines.push(DetailLine::Field(
                t!("Group count").into_owned(),
                index.plant_groups.len().to_string(),
            ));
            lines.push(DetailLine::Field(
                t!("Drawing count").into_owned(),
                index.drawings.len().to_string(),
            ));
        }
        NodeId::Group(path) => {
            if let Some(node) = group_at(index, path) {
                lines.push(DetailLine::Field(
                    t!("Group name").into_owned(),
                    group_label(&node.name),
                ));
                lines.push(DetailLine::Field(
                    t!("Group path").into_owned(),
                    group_path_label(&node.path),
                ));
                lines.push(DetailLine::Field(
                    t!("Drawing count").into_owned(),
                    group_drawing_count(node).to_string(),
                ));
            }
        }
        NodeId::Drawing(drawing) => {
            if let Some(entry) = index.drawings.get(*drawing) {
                lines.push(DetailLine::Field(
                    t!("Drawing name").into_owned(),
                    entry.name.clone(),
                ));
                lines.push(DetailLine::Field(
                    t!("Group path").into_owned(),
                    group_path_label(&entry.group_path),
                ));
                match &entry.pid_path {
                    Some(path) => {
                        lines.push(DetailLine::Field(
                            t!("File path").into_owned(),
                            path.display().to_string(),
                        ));
                        lines.push(DetailLine::OpenDrawing(
                            t!("Open drawing").into_owned(),
                        ));
                    }
                    // A row the plant knows about but the backup does not
                    // carry: there is nothing to open, so the panel says why.
                    None => {
                        lines.push(DetailLine::Marker(
                            t!("Registered but missing").into_owned(),
                        ));
                    }
                }
                // A drawing whose items could not be read has no subtree to
                // show for it; the reason must not be lost with the arrow.
                if let Some(ItemsState::Failed(error)) = state.items.get(drawing) {
                    lines.push(DetailLine::Marker(error.clone()));
                }
                // What a section offers for the drawing stands here, with
                // the button that opens it, not below the columns.
                section::early_details(state, index, id, &mut lines);
                // Every `T_DRAWING` column, empty values included: the panel
                // is the only place the backup's own metadata is visible.
                for (key, value) in &entry.meta {
                    lines.push(DetailLine::Field(key.clone(), value.clone()));
                }
            }
        }
        NodeId::ItemType(drawing, group) => {
            if let Some(ItemsState::Ready(items)) = state.items.get(drawing) {
                if let Some(group) = items.groups.get(*group) {
                    lines.push(DetailLine::Field(
                        t!("Item type").into_owned(),
                        group.item_type.clone(),
                    ));
                    lines.push(DetailLine::Field(
                        t!("Item count").into_owned(),
                        group.items.len().to_string(),
                    ));
                    // The items of the type side by side: what each is
                    // called, what it is, and the UID the rows leave out.
                    lines.push(DetailLine::Table(std::sync::Arc::new(grid::Table::new(
                        vec![
                            grid::Column::text(t!("Item name")),
                            grid::Column::text(t!("Item class")),
                            grid::Column::text(t!("Item UID")),
                        ],
                        group
                            .items
                            .iter()
                            .map(|item| {
                                vec![
                                    item.label.clone(),
                                    item.class.clone().unwrap_or_default(),
                                    item.uid.clone(),
                                ]
                            })
                            .collect(),
                    ))));
                }
            }
        }
        NodeId::Item(drawing, group, item) => {
            let found = match state.items.get(drawing) {
                Some(ItemsState::Ready(items)) => items
                    .groups
                    .get(*group)
                    .and_then(|group| group.items.get(*item)),
                _ => None,
            };
            if let Some(entry) = found {
                lines.push(DetailLine::Field(
                    t!("Item name").into_owned(),
                    entry.label.clone(),
                ));
                lines.push(DetailLine::Field(
                    t!("Item UID").into_owned(),
                    entry.uid.clone(),
                ));
                lines.push(DetailLine::Field(
                    t!("Item type").into_owned(),
                    entry.item_type.clone(),
                ));
                if let Some(class) = &entry.class {
                    lines.push(DetailLine::Field(
                        t!("Item class").into_owned(),
                        class.clone(),
                    ));
                }
                lines.push(DetailLine::Heading(
                    t!("Item attributes").into_owned(),
                ));
                for attribute in &entry.attributes {
                    lines.push(DetailLine::Field(
                        format!("{}.{}", attribute.interface, attribute.name),
                        attribute.value.clone(),
                    ));
                }
            }
        }
        NodeId::Ref => {
            lines.push(DetailLine::Field(
                t!("Reference name").into_owned(),
                index.ref_tree.name.clone(),
            ));
            lines.push(DetailLine::Field(
                t!("Reference file count").into_owned(),
                index.ref_tree.file_count().to_string(),
            ));
            // Where the tree was read from: the restore, with the backup's
            // reference data volumes unpacked, or a folder beside a backup
            // that is used where it lies.
            if !index.ref_root.as_os_str().is_empty() {
                lines.push(DetailLine::Field(
                    t!("Reference folder").into_owned(),
                    index.ref_root.display().to_string(),
                ));
            }
        }
        NodeId::RefDir(path) => {
            if let Some(node) = ref_at(&index.ref_tree, path) {
                lines.push(DetailLine::Field(
                    t!("Reference name").into_owned(),
                    node.name.clone(),
                ));
                lines.push(DetailLine::Field(
                    t!("Reference file count").into_owned(),
                    node.file_count().to_string(),
                ));
            }
        }
        NodeId::RefFile(path, at) => {
            if let Some(file) = ref_at(&index.ref_tree, path).and_then(|node| node.files.get(*at)) {
                lines.push(DetailLine::Field(
                    t!("Reference file name").into_owned(),
                    file.name.clone(),
                ));
                lines.push(DetailLine::Field(
                    t!("File size").into_owned(),
                    human_size(file.size),
                ));
            }
        }
        // A section's node: the section states all of it, below.
        NodeId::Section(..) => {}
    }
    section::details(state, index, id, &mut lines);
    lines
}

/// The `(key, value)` pairs the detail half shows for `id`, in display order.
///
/// The testable projection of [`detail_lines`]: an empty value marks a line
/// that is not a plain field (a warning, a heading, "registered but missing",
/// the "open drawing" action).
#[cfg(test)]
pub(crate) fn detail_rows(state: &PidBackupBrowser, id: &NodeId) -> Vec<(String, String)> {
    detail_lines(state, id)
        .into_iter()
        .flat_map(DetailLine::into_pairs)
        .collect()
}

/// The buttons in the details of `id`: each with the node it activates.
#[cfg(test)]
pub(crate) fn detail_buttons(state: &PidBackupBrowser, id: &NodeId) -> Vec<(String, NodeId)> {
    detail_lines(state, id)
        .into_iter()
        .filter_map(|line| match line {
            DetailLine::Button(label, node) => Some((label, node)),
            _ => None,
        })
        .collect()
}

/// The buttons in the details of the selected node, in the order they are
/// drawn: the label of each and the message a press of it sends. What
/// `PIDBACKUPOPEN Press` presses, so a script can do what a click does.
pub fn selected_buttons(state: &PidBackupBrowser) -> Vec<(String, PidBackupMsg)> {
    let Some(id) = state.selected.as_ref() else {
        return Vec::new();
    };
    detail_lines(state, id)
        .into_iter()
        .filter_map(|line| match line {
            DetailLine::Button(label, node) => Some((label, PidBackupMsg::Activate(node))),
            // The "open drawing" button activates the drawing itself.
            DetailLine::OpenDrawing(label) => Some((label, PidBackupMsg::Activate(id.clone()))),
            DetailLine::Action(label, msg) => Some((label, msg)),
            _ => None,
        })
        .collect()
}

/// Build the docked panel.
pub fn view<'a>(state: &'a PidBackupBrowser, width: f32, chrome: Chrome) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(
        PanelId::PidBackup,
        t!("P&ID Backup").into_owned(),
        chrome,
    );

    let toolbar = row![
        button(text(t!("Open…")).size(11))
            .on_press(PidBackupMsg::Open.message())
            .padding([3, 8]),
        button(text(t!("Refresh backup")).size(11))
            .on_press(PidBackupMsg::Refresh.message())
            .padding([3, 8]),
        button(text(t!("Close backup")).size(11))
            .on_press(PidBackupMsg::Close.message())
            .padding([3, 8]),
    ]
    .spacing(4)
    .align_y(iced::Center);

    let search = text_input(
        t!("Search drawings and reference files…").as_ref(),
        &state.filter,
    )
    .on_input(|text| PidBackupMsg::Filter(text).message())
    .size(11)
    .padding([3, 6]);

    // The upper half is the tree, built only from the rows the user opened;
    // the lower half is what the selected node is. The bar between them is
    // the user's to drag.
    let halves = iced::widget::pane_grid::PaneGrid::new(
        state.split.panes(),
        move |_, half, _| {
            iced::widget::pane_grid::Content::new(match half {
                split::Half::Tree => tree_half(state),
                split::Half::Details => details_half(state, width),
            })
        },
    )
    .spacing(split::BAR)
    .min_size(split::MIN_HALF)
    .on_resize(split::GRAB, |event| {
        PidBackupMsg::SplitDragged(event).message()
    })
    .width(Fill)
    .height(Fill);

    let content = column![toolbar, search, halves].spacing(4).height(Fill);
    let reported = crate::ui::wrap_bar::PosReport::new(PANEL_BOUNDS_ID, content);
    crate::ui::dock::frame(title_bar, reported, width)
}

/// The lower half: a rule that says where the bar is, then the details.
fn details_half<'a>(state: &'a PidBackupBrowser, width: f32) -> Element<'a, Message> {
    column![
        iced::widget::rule::horizontal(1),
        container(details(state, width)).width(Fill).height(Fill),
    ]
    .spacing(3)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The upper half: the tree, or what stands in for it while there is none.
fn tree_half<'a>(state: &'a PidBackupBrowser) -> Element<'a, Message> {
    match &state.load {
        LoadState::Idle => {
            let mut hint = column![
                text(t!("No backup is open.")).size(11).style(muted),
            ]
            .spacing(2);
            if let Some(error) = &state.error {
                hint = hint.push(text(error.clone()).size(10).style(muted));
            }
            container(hint).width(Fill).padding([4, 6]).into()
        }
        LoadState::Loading(source) => container(
            column![
                text(t!("Loading backup…")).size(11).style(muted),
                text(source.display().to_string()).size(10).style(muted),
            ]
            .spacing(2),
        )
        .width(Fill)
        .padding([4, 6])
        .into(),
        LoadState::Ready(_) => {
            let rows = visible_rows(state);
            let mut list = column![].spacing(1);
            for (at, row) in rows.iter().enumerate() {
                let selected = state.selected.as_ref() == Some(&row.id);
                list = list.push(tree_row(row, at, selected));
            }
            scrollable(list)
                .id(iced::widget::Id::new(split::TREE_SCROLL))
                .direction(scrollable::Direction::Vertical(scroll_bar()))
                .width(Fill)
                .height(Fill)
                .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_backup_browse::{
        BackupFile, DrawingEntry, DrawingItem, FileKind, ItemAttribute, ItemGroup, ManifestSummary,
        RefFile,
    };

    fn group(name: &str, path: &str, drawings: Vec<usize>) -> GroupNode {
        GroupNode {
            name: name.to_string(),
            path: path.to_string(),
            children: Vec::new(),
            drawings,
        }
    }

    fn drawing(name: &str, file: Option<&str>) -> DrawingEntry {
        DrawingEntry {
            name: name.to_string(),
            pid_path: file.map(PathBuf::from),
            group_path: "SQPlant / 00 / 00".to_string(),
            meta: vec![("id".to_string(), "1".to_string())],
        }
    }

    fn file(name: &str, members: &[&str]) -> BackupFile {
        BackupFile {
            name: name.to_string(),
            kind: FileKind::PlantData,
            format: "zip".to_string(),
            size: 10,
            members: members.iter().map(|m| m.to_string()).collect(),
            version_line: None,
            missing: false,
            ..BackupFile::default()
        }
    }

    fn ref_file(name: &str) -> RefFile {
        RefFile {
            name: name.to_string(),
            size: 1,
        }
    }

    /// A backup with two plant groups, three drawings (one missing its file),
    /// two backup files (one an archive with two members) and a `Ref` tree.
    fn index() -> Arc<BackupIndex> {
        // `SQPlant / 00` holds two drawings, its child `01` holds the third.
        let mut plant = group("00", "SQPlant / 00", vec![0, 2]);
        plant
            .children
            .push(group("01", "SQPlant / 00 / 01", vec![1]));
        let mut root = group("SQPlant", "SQPlant", Vec::new());
        root.children.push(plant);

        let symbols = RefNode {
            name: "Symbols".to_string(),
            dirs: Vec::new(),
            files: vec![ref_file("valve.svg"), ref_file("pump.svg")],
        };
        let ref_tree = RefNode {
            name: "Ref".to_string(),
            dirs: vec![symbols],
            files: vec![ref_file("readme.txt")],
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
            files: vec![
                file("PlantData~2~711.zip", &["00/00/a.pid", "00/00/b.pid"]),
                file("RefData~4~1.zip", &[]),
            ],
            plant_groups: vec![root],
            drawings: vec![
                drawing("A-101", Some("D:/tmp/restore/00/00/a.pid")),
                drawing("B-202", Some("D:/tmp/restore/00/00/b.pid")),
                drawing("C-303", None),
            ],
            ref_tree,
            warnings: vec!["no usable Export.dmp".to_string()],
            ..BackupIndex::default()
        })
    }

    pub(super) fn ready() -> PidBackupBrowser {
        let mut state = PidBackupBrowser::default();
        assert_eq!(
            update(&mut state, PidBackupMsg::Loaded(Ok(index()))),
            Effect::None
        );
        state
    }

    pub(super) fn items_of(uid: &str, item_type: &str) -> DrawingItems {
        DrawingItems {
            drawing_uid: "UID-1".to_string(),
            drawing_name: "A-101".to_string(),
            groups: vec![ItemGroup {
                item_type: item_type.to_string(),
                items: vec![DrawingItem {
                    uid: uid.to_string(),
                    label: "TT-101".to_string(),
                    item_type: item_type.to_string(),
                    class: Some("PIDInstrument".to_string()),
                    attributes: vec![ItemAttribute {
                        interface: "INamedInstrument".to_string(),
                        name: "MeasuredVariable".to_string(),
                        value: "T".to_string(),
                    }],
                }],
            }],
            item_count: 1,
            ..DrawingItems::default()
        }
    }

    #[test]
    fn many_lines_above_a_table_scroll_and_the_last_heading_stays_with_it() {
        let lines = |text: &str| -> Vec<bool> { text.chars().map(|ch| ch == 'H').collect() };
        // A few lines stand as they are, with or without a heading.
        assert_eq!(split_above_table(&lines("....")), (4, false));
        assert_eq!(split_above_table(&lines("..H..")), (5, false));
        assert_eq!(split_above_table(&lines("H.......")), (8, false));
        assert_eq!(split_above_table(&lines("...H.....H")), (10, false));
        assert_eq!(split_above_table(&[]), (0, false));
        // Many lines scroll, all of them when no heading says otherwise.
        assert_eq!(split_above_table(&lines("...........")), (11, true));
        // The last heading and the few lines under it stay beside the
        // table; what comes before them scrolls.
        assert_eq!(split_above_table(&lines("....H....H...")), (9, true));
        assert_eq!(split_above_table(&lines("...H.....H.")), (9, true));
        assert_eq!(split_above_table(&lines("...H.....H.....")), (9, true));
        // A heading with a long run under it does not stay: all scroll.
        assert_eq!(split_above_table(&lines("...H.....H......")), (16, true));
        assert_eq!(split_above_table(&lines("..H...........")), (14, true));
    }

    fn ids(rows: &[Row]) -> Vec<NodeId> {
        rows.iter().map(|row| row.id.clone()).collect()
    }

    #[test]
    fn a_group_row_shows_its_own_name_and_translates_the_two_fixed_ones() {
        // Every other name belongs to the plant and is shown as it is; the
        // two the index synthesises are labels the panel translates.
        assert_eq!(group_label("SQPlant"), "SQPlant");
        assert_eq!(group_label("00"), "00");
        assert_eq!(
            group_label(crate::io::pid_backup_browse::UNREGISTERED_GROUP),
            t!("Unregistered").into_owned()
        );
        assert_eq!(
            group_label(crate::io::pid_backup_browse::UNGROUPED),
            t!("(ungrouped)").into_owned()
        );
    }

    #[test]
    fn ready_state_starts_with_the_root_and_its_three_groups() {
        let rows = visible_rows(&ready());
        assert_eq!(
            ids(&rows),
            vec![NodeId::Root, NodeId::Files, NodeId::Groups, NodeId::Ref]
        );
        assert_eq!(
            rows.iter().map(|row| row.depth).collect::<Vec<_>>(),
            vec![0, 1, 1, 1]
        );
        assert!(rows[0].expanded, "the root is always open");
        assert!(rows[1].expandable && rows[2].expandable && rows[3].expandable);
        assert!(!rows[1].expanded && !rows[2].expanded && !rows[3].expanded);
    }

    #[test]
    fn toggling_a_group_shows_its_children_one_level_deeper() {
        let mut state = ready();
        assert_eq!(
            update(&mut state, PidBackupMsg::Toggle(NodeId::Files)),
            Effect::None
        );
        let rows = visible_rows(&state);
        assert_eq!(
            ids(&rows),
            vec![
                NodeId::Root,
                NodeId::Files,
                NodeId::File(0),
                NodeId::File(1),
                NodeId::Groups,
                NodeId::Ref,
            ]
        );
        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[3].depth, 2);
        // The archive has members, the plain file has none.
        assert!(rows[2].expandable);
        assert!(!rows[3].expandable);

        assert_eq!(
            update(&mut state, PidBackupMsg::Toggle(NodeId::Files)),
            Effect::None
        );
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![NodeId::Root, NodeId::Files, NodeId::Groups, NodeId::Ref]
        );
    }

    #[test]
    fn a_group_opens_its_children_and_drawings_in_order() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Groups));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Group(vec![0])));
        let rows = visible_rows(&state);
        assert_eq!(
            ids(&rows),
            vec![
                NodeId::Root,
                NodeId::Files,
                NodeId::Groups,
                NodeId::Group(vec![0]),
                NodeId::Group(vec![0, 0]),
                NodeId::Ref,
            ]
        );
        assert_eq!(rows[4].depth, 3);

        // The group holding the drawings shows them after its subgroups.
        update(
            &mut state,
            PidBackupMsg::Toggle(NodeId::Group(vec![0, 0])),
        );
        let rows = visible_rows(&state);
        let tail = ids(&rows);
        assert_eq!(
            &tail[4..],
            &[
                NodeId::Group(vec![0, 0]),
                NodeId::Group(vec![0, 0, 0]),
                NodeId::Drawing(0),
                NodeId::Drawing(2),
                NodeId::Ref,
            ]
        );
        assert_eq!(rows[6].depth, 4);
        // The drawing with no file is greyed; the one with a file is not.
        assert!(!rows[6].disabled);
        assert!(rows[7].disabled);
    }

    #[test]
    fn a_collapsed_ref_tree_costs_three_rows() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Ref));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![
                NodeId::Root,
                NodeId::Files,
                NodeId::Groups,
                NodeId::Ref,
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![], 0),
            ]
        );
        update(&mut state, PidBackupMsg::Toggle(NodeId::RefDir(vec![0])));
        let rows = visible_rows(&state);
        assert_eq!(
            &ids(&rows)[4..],
            &[
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![0], 0),
                NodeId::RefFile(vec![0], 1),
                NodeId::RefFile(vec![], 0),
            ]
        );
        assert_eq!(rows[5].depth, 3);
    }

    #[test]
    fn a_filter_keeps_matching_drawings_and_their_ancestors() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Filter("b-2".to_string()));
        let rows = visible_rows(&state);
        assert_eq!(
            ids(&rows),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Group(vec![0]),
                NodeId::Group(vec![0, 0]),
                NodeId::Group(vec![0, 0, 0]),
                NodeId::Drawing(1),
            ],
            "only the branch holding B-202 stays, opened"
        );
        assert!(rows.iter().all(|row| row.id != NodeId::Files && row.id != NodeId::Ref));

        // Clearing the filter restores the untouched tree.
        update(&mut state, PidBackupMsg::Filter(String::new()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![NodeId::Root, NodeId::Files, NodeId::Groups, NodeId::Ref]
        );
    }

    #[test]
    fn a_filter_that_matches_nothing_keeps_only_the_root_and_the_group() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Filter("zzz".to_string()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![NodeId::Root, NodeId::Groups]
        );
    }

    #[test]
    fn a_filter_keeps_the_reference_files_it_names_under_their_directories() {
        let mut state = ready();
        let unfolded = state.expanded.clone();
        update(&mut state, PidBackupMsg::Filter("valve".to_string()));
        let rows = visible_rows(&state);
        assert_eq!(
            ids(&rows),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Ref,
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![0], 0),
            ],
            "valve.svg stays, under Symbols and the reference data"
        );
        assert_eq!(
            rows[2..].iter().map(|row| row.depth).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(rows[3].label, "Symbols");
        assert_eq!(rows[4].label, "valve.svg");
        // What stands above the file is shown open without the user having
        // unfolded it.
        assert!(rows[2].expandable && rows[2].expanded);
        assert!(rows[3].expandable && rows[3].expanded);
        assert!(!rows[4].expandable);
        assert_eq!(state.expanded, unfolded);

        // Neither the case nor the spaces around the text count; a file
        // whose name does not hold the text goes, `readme.txt` here.
        update(&mut state, PidBackupMsg::Filter(" SVG ".to_string()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Ref,
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![0], 0),
                NodeId::RefFile(vec![0], 1),
            ]
        );

        // A directory none of whose files is named is not shown.
        update(&mut state, PidBackupMsg::Filter("readme".to_string()));
        let rows = visible_rows(&state);
        assert_eq!(
            ids(&rows),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Ref,
                NodeId::RefFile(vec![], 0),
            ]
        );
        assert_eq!(rows[3].depth, 2);

        // A text both a drawing and reference files hold keeps both, the
        // drawings first; the file beside the directories comes after them.
        update(&mut state, PidBackupMsg::Filter("a".to_string()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Group(vec![0]),
                NodeId::Group(vec![0, 0]),
                NodeId::Drawing(0),
                NodeId::Ref,
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![0], 0),
                NodeId::RefFile(vec![], 0),
            ]
        );
        assert_eq!(state.expanded, unfolded);
    }

    #[test]
    fn a_filter_compares_the_names_of_files_and_not_of_directories() {
        let mut state = ready();
        // `Symbols` is a directory: its name keeps neither itself nor the
        // files in it, and with no file named the reference data is not
        // shown at all.
        update(&mut state, PidBackupMsg::Filter("symbols".to_string()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![NodeId::Root, NodeId::Groups]
        );
    }

    #[test]
    fn clearing_a_filter_gives_the_tree_back_as_the_user_left_it() {
        let mut state = ready();
        // The reference data is unfolded, `Symbols` under it is not.
        update(&mut state, PidBackupMsg::Toggle(NodeId::Ref));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Groups));
        let rows = visible_rows(&state);
        let unfolded = state.expanded.clone();
        assert!(!unfolded.contains(&NodeId::RefDir(vec![0])));

        update(&mut state, PidBackupMsg::Filter("pump".to_string()));
        assert_eq!(
            ids(&visible_rows(&state)),
            vec![
                NodeId::Root,
                NodeId::Groups,
                NodeId::Ref,
                NodeId::RefDir(vec![0]),
                NodeId::RefFile(vec![0], 1),
            ],
            "the folded directory is shown open for the file in it"
        );
        assert_eq!(state.expanded, unfolded);

        update(&mut state, PidBackupMsg::Filter(String::new()));
        assert_eq!(visible_rows(&state), rows);
        assert_eq!(state.expanded, unfolded);
    }

    #[test]
    fn a_reference_file_a_filter_keeps_is_the_node_it_was() {
        let node = NodeId::RefFile(vec![0], 0);
        let mut plain = ready();
        let asked_plain = update(&mut plain, PidBackupMsg::Select(node.clone()));

        let mut searched = ready();
        update(&mut searched, PidBackupMsg::Filter("valve".to_string()));
        assert!(visible_rows(&searched).iter().any(|row| row.id == node));
        let asked = update(&mut searched, PidBackupMsg::Select(node.clone()));
        // The same read is asked for and the same details are shown.
        assert_eq!(asked, asked_plain);
        let details = detail_rows(&searched, &node);
        assert!(!details.is_empty());
        assert_eq!(details, detail_rows(&plain, &node));
        assert_eq!(searched.selected, Some(node.clone()));
        // Typing on leaves the selection where it is, shown or not.
        update(&mut searched, PidBackupMsg::Filter("pump".to_string()));
        assert!(visible_rows(&searched).iter().all(|row| row.id != node));
        assert_eq!(searched.selected, Some(node));
    }

    #[test]
    fn activating_a_drawing_opens_its_file_and_a_missing_one_does_nothing() {
        let mut state = ready();
        assert_eq!(
            update(&mut state, PidBackupMsg::Activate(NodeId::Drawing(0))),
            Effect::OpenDrawing(PathBuf::from("D:/tmp/restore/00/00/a.pid"))
        );
        assert_eq!(
            update(&mut state, PidBackupMsg::Activate(NodeId::Drawing(2))),
            Effect::None
        );
        assert_eq!(
            update(&mut state, PidBackupMsg::Activate(NodeId::Root)),
            Effect::None
        );
        assert_eq!(
            update(&mut state, PidBackupMsg::Activate(NodeId::ItemType(0, 0))),
            Effect::None
        );
    }

    #[test]
    fn the_first_expand_of_a_drawing_asks_for_its_items_once() {
        let mut state = ready();
        assert_eq!(
            update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0))),
            Effect::LoadItems(0, PathBuf::from("D:/tmp/restore/00/00/a.pid"))
        );
        assert!(matches!(state.items.get(&0), Some(ItemsState::Loading)));

        // Folding and unfolding again does not read the file a second time.
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0)));
        assert_eq!(
            update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0))),
            Effect::None
        );

        // A drawing with no file never reads anything.
        assert_eq!(
            update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(2))),
            Effect::None
        );
        assert!(!state.items.contains_key(&2));
    }

    #[test]
    fn loaded_items_show_their_type_groups_and_items() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0)));
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(items_of("UID-1", "Instrument")))),
        );
        update(&mut state, PidBackupMsg::Toggle(NodeId::Groups));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Group(vec![0])));
        update(
            &mut state,
            PidBackupMsg::Toggle(NodeId::Group(vec![0, 0])),
        );
        let rows = visible_rows(&state);
        let drawing_at = rows
            .iter()
            .position(|row| row.id == NodeId::Drawing(0))
            .expect("the drawing row");
        assert_eq!(rows[drawing_at + 1].id, NodeId::ItemType(0, 0));
        assert_eq!(rows[drawing_at + 1].depth, rows[drawing_at].depth + 1);
        assert!(
            rows[drawing_at + 1].label.contains("Instrument")
                && rows[drawing_at + 1].label.contains('1'),
            "the type group counts its items: {}",
            rows[drawing_at + 1].label
        );

        update(&mut state, PidBackupMsg::Toggle(NodeId::ItemType(0, 0)));
        let rows = visible_rows(&state);
        let type_at = rows
            .iter()
            .position(|row| row.id == NodeId::ItemType(0, 0))
            .expect("the type row");
        assert_eq!(rows[type_at + 1].id, NodeId::Item(0, 0, 0));
        // The row is the item's name; its UID is in the details.
        assert_eq!(rows[type_at + 1].label, "TT-101");
    }

    #[test]
    fn a_failed_item_read_leaves_the_drawing_without_a_subtree() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0)));
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Err("cannot read".to_string())),
        );
        update(&mut state, PidBackupMsg::Toggle(NodeId::Groups));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Group(vec![0])));
        update(
            &mut state,
            PidBackupMsg::Toggle(NodeId::Group(vec![0, 0])),
        );
        let rows = visible_rows(&state);
        let drawing_at = rows
            .iter()
            .position(|row| row.id == NodeId::Drawing(0))
            .expect("the drawing row");
        assert!(!rows[drawing_at].expandable);
        assert!(!rows
            .iter()
            .any(|row| matches!(row.id, NodeId::ItemType(..))));
        assert!(matches!(
            state.items.get(&0),
            Some(ItemsState::Failed(_))
        ));
        // The arrow is gone, so the details are where the reason is read.
        assert!(
            detail_rows(&state, &NodeId::Drawing(0))
                .contains(&("cannot read".to_string(), String::new())),
            "{:?}",
            detail_rows(&state, &NodeId::Drawing(0))
        );
        // A drawing whose items read carries no such line.
        assert!(!detail_rows(&state, &NodeId::Drawing(1))
            .iter()
            .any(|(key, _)| key == "cannot read"));
    }

    /// The row of drawing `at` in the tree opened down to the drawings.
    fn drawing_row(state: &PidBackupBrowser, at: usize) -> Row {
        visible_rows(state)
            .into_iter()
            .find(|row| row.id == NodeId::Drawing(at))
            .expect("the drawing row")
    }

    /// A state whose tree is opened down to the three drawings.
    pub(super) fn ready_with_drawings_shown() -> PidBackupBrowser {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Groups));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Group(vec![0])));
        update(
            &mut state,
            PidBackupMsg::Toggle(NodeId::Group(vec![0, 0])),
        );
        update(
            &mut state,
            PidBackupMsg::Toggle(NodeId::Group(vec![0, 0, 0])),
        );
        state
    }

    #[test]
    fn a_drawing_with_a_file_keeps_its_arrow_until_its_items_are_known() {
        let mut state = ready_with_drawings_shown();
        // Nothing read yet: the arrow is the way in, so it has to be there.
        let unread = drawing_row(&state, 0);
        assert!(unread.expandable && !unread.expanded);
        // A drawing with no file has nothing to read.
        assert!(!drawing_row(&state, 2).expandable);

        // While the read is in flight the row is open and still has it.
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0)));
        let reading = drawing_row(&state, 0);
        assert!(reading.expandable && reading.expanded);

        // Items arrived: the arrow folds and unfolds them.
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(items_of("UID-1", "Instrument")))),
        );
        assert!(drawing_row(&state, 0).expandable);

        // A drawing that turns out to hold no item has nothing to unfold.
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(1)));
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(1, Ok(Arc::new(DrawingItems::default()))),
        );
        assert!(!drawing_row(&state, 1).expandable);
    }

    /// The pure tests above send `Toggle` themselves, which is how a panel
    /// that never drew the arrow of an unread drawing went unnoticed: no
    /// click could reach a drawing's items. This one presses where the arrow
    /// is drawn.
    #[test]
    fn the_drawn_row_of_an_unread_drawing_carries_the_arrow_that_reads_it() {
        let _guard = crate::headless_ui_lock();
        let state = ready_with_drawings_shown();
        let press_left_of = |label: &str| -> Vec<Message> {
            let mut ui = iced_test::simulator::Simulator::with_size(
                iced::Settings::default(),
                (340.0, 600.0),
                view(&state, 340.0, Chrome::default()),
            );
            let bounds = ui
                .find(label)
                .unwrap_or_else(|error| panic!("the row of {label} is drawn: {error:?}"))
                .visible_bounds()
                .expect("the row is on screen");
            // The arrow (or the blank that stands in for it) sits just left
            // of the label.
            ui.point_at(iced::Point::new(bounds.x - 7.0, bounds.center_y()));
            let _ = ui.simulate(iced_test::simulator::click());
            ui.into_messages().collect()
        };

        let toggled = |messages: &[Message], drawing: usize| {
            messages.iter().any(|message| {
                matches!(
                    message,
                    Message::PidBackup(PidBackupMsg::Toggle(NodeId::Drawing(at))) if *at == drawing
                )
            })
        };
        assert!(
            toggled(&press_left_of("A-101"), 0),
            "pressing the arrow of an unread drawing asks for its items"
        );
        // A drawing without a file draws a blank there: the press selects.
        let missing = press_left_of("C-303");
        assert!(!toggled(&missing, 2), "a drawing without a file has no arrow");
        assert!(missing.iter().any(|message| matches!(
            message,
            Message::PidBackup(PidBackupMsg::Select(NodeId::Drawing(2)))
        )));
    }

    #[test]
    fn selecting_sets_the_row_and_close_forgets_everything() {
        let mut state = ready();
        assert_eq!(
            update(&mut state, PidBackupMsg::Select(NodeId::File(0))),
            Effect::None
        );
        assert_eq!(state.selected, Some(NodeId::File(0)));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));
        update(&mut state, PidBackupMsg::Toggle(NodeId::Drawing(0)));
        update(&mut state, PidBackupMsg::Filter("a".to_string()));

        assert_eq!(update(&mut state, PidBackupMsg::Close), Effect::None);
        assert!(matches!(state.load, LoadState::Idle));
        assert!(state.expanded.is_empty());
        assert_eq!(state.selected, None);
        assert!(state.items.is_empty());
        assert!(state.filter.is_empty());
        assert!(visible_rows(&state).is_empty());
    }

    #[test]
    fn a_failed_load_records_the_error_and_stays_idle() {
        let mut state = PidBackupBrowser::default();
        assert_eq!(
            update(
                &mut state,
                PidBackupMsg::Loaded(Err("no Manifest.txt".to_string()))
            ),
            Effect::None
        );
        assert!(matches!(state.load, LoadState::Idle));
        assert_eq!(state.error.as_deref(), Some("no Manifest.txt"));
        assert!(visible_rows(&state).is_empty());
    }

    #[test]
    fn a_picked_source_asks_for_its_index_and_a_cancelled_one_does_nothing() {
        let mut state = PidBackupBrowser::default();
        assert_eq!(
            update(&mut state, PidBackupMsg::SourcePicked(None)),
            Effect::None
        );
        assert!(matches!(state.load, LoadState::Idle));
        assert_eq!(
            update(
                &mut state,
                PidBackupMsg::SourcePicked(Some(PathBuf::from("D:/backups/a.zip")))
            ),
            Effect::Load(PathBuf::from("D:/backups/a.zip"))
        );
        assert!(matches!(state.load, LoadState::Loading(_)));
        assert_eq!(update(&mut state, PidBackupMsg::Open), Effect::PickSource);
    }

    #[test]
    fn refresh_and_close_ask_the_command_layer_for_the_work() {
        let mut state = ready();
        assert_eq!(
            update(&mut state, PidBackupMsg::Refresh),
            Effect::Refresh(PathBuf::from("D:/backups/plant.zip"))
        );
        // Refreshing an idle panel has no source to rebuild from.
        let mut idle = PidBackupBrowser::default();
        assert_eq!(update(&mut idle, PidBackupMsg::Refresh), Effect::None);
    }

    #[test]
    fn a_new_backup_replaces_the_old_tree() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));
        update(&mut state, PidBackupMsg::Select(NodeId::File(0)));
        update(&mut state, PidBackupMsg::Filter("a".to_string()));
        update(
            &mut state,
            PidBackupMsg::Loaded(Ok(Arc::new(BackupIndex {
                source: PathBuf::from("D:/backups/other.zip"),
                ..(*index()).clone()
            }))),
        );
        assert!(state.expanded.is_empty());
        assert_eq!(state.selected, None);
        assert!(state.filter.is_empty());
        assert_eq!(state.error, None);
    }

    /// A backup richer than [`index`]: every manifest field, a file the
    /// manifest declares but the backup does not carry, the two synthesised
    /// group names, and drawings with and without a file.
    fn detail_index() -> Arc<BackupIndex> {
        let mut plant = group("00", "SQPlant / 00", vec![0, 2]);
        plant.children.push(group("01", "SQPlant / 00 / 01", vec![1]));
        let mut root = group("SQPlant", "SQPlant", Vec::new());
        root.children.push(plant);
        let unregistered = group(
            crate::io::pid_backup_browse::UNREGISTERED_GROUP,
            crate::io::pid_backup_browse::UNREGISTERED_GROUP,
            Vec::new(),
        );
        let ungrouped = group(
            crate::io::pid_backup_browse::UNGROUPED,
            crate::io::pid_backup_browse::UNGROUPED,
            Vec::new(),
        );

        Arc::new(BackupIndex {
            source: PathBuf::from("D:/backups/plant.zip"),
            restored: PathBuf::from("D:/tmp/restore"),
            backup_root: PathBuf::from("D:/tmp/restore"),
            read_only: true,
            manifest: Some(ManifestSummary {
                name: Some("Plant backup".to_string()),
                version: Some("7".to_string()),
                date_created: Some("2026-01-02".to_string()),
                backup_type: Some("Full".to_string()),
                spid: Some("SQPlant".to_string()),
            }),
            files: vec![
                BackupFile {
                    name: "Export.dmp".to_string(),
                    kind: FileKind::ExportDump,
                    format: "Oracle exp (EXPORT:V12.01.00)".to_string(),
                    size: 2048,
                    members: Vec::new(),
                    version_line: Some("EXPORT:V12.01.00".to_string()),
                    missing: false,
                    ..BackupFile::default()
                },
                BackupFile {
                    name: "PlantData~2~711.zip".to_string(),
                    kind: FileKind::PlantData,
                    format: "zip".to_string(),
                    size: 1024,
                    members: vec!["00/00/a.pid".to_string(), "00/00/b.pid".to_string()],
                    version_line: None,
                    missing: false,
                    ..BackupFile::default()
                },
                BackupFile {
                    name: "RefData~4~824.zip".to_string(),
                    kind: FileKind::RefData,
                    format: "zip".to_string(),
                    size: 0,
                    members: Vec::new(),
                    version_line: None,
                    missing: true,
                    ..BackupFile::default()
                },
            ],
            plant_groups: vec![root, unregistered, ungrouped],
            drawings: vec![
                DrawingEntry {
                    name: "A-101".to_string(),
                    pid_path: Some(PathBuf::from("D:/tmp/restore/00/00/a.pid")),
                    group_path: "SQPlant / 00".to_string(),
                    meta: vec![
                        ("id".to_string(), "1".to_string()),
                        ("PATH".to_string(), "00/00/a.pid".to_string()),
                    ],
                },
                DrawingEntry {
                    name: "B-202".to_string(),
                    pid_path: Some(PathBuf::from("D:/tmp/restore/00/00/01/b.pid")),
                    group_path: "SQPlant / 00 / 01".to_string(),
                    meta: vec![("id".to_string(), "2".to_string())],
                },
                DrawingEntry {
                    name: "C-303".to_string(),
                    pid_path: None,
                    group_path: "SQPlant / 00".to_string(),
                    meta: vec![("id".to_string(), "3".to_string())],
                },
            ],
            ref_tree: RefNode {
                name: "Ref".to_string(),
                dirs: vec![RefNode {
                    name: "Symbols".to_string(),
                    dirs: Vec::new(),
                    files: vec![
                        RefFile {
                            name: "pump.svg".to_string(),
                            size: 4096,
                        },
                        RefFile {
                            name: "valve.svg".to_string(),
                            size: 2048,
                        },
                    ],
                }],
                files: vec![RefFile {
                    name: "readme.txt".to_string(),
                    size: 12,
                }],
            },
            warnings: vec![
                "no usable Export.dmp".to_string(),
                "one declared file is missing".to_string(),
            ],
            ..BackupIndex::default()
        })
    }

    fn detail_state() -> PidBackupBrowser {
        let mut state = PidBackupBrowser::default();
        assert_eq!(
            update(&mut state, PidBackupMsg::Loaded(Ok(detail_index()))),
            Effect::None
        );
        state
    }

    /// The detail rows as the panel shows them: `(key, value)` pairs.
    fn detail_rows_of(state: &PidBackupBrowser, id: &NodeId) -> Vec<(String, String)> {
        detail_rows(state, id)
    }

    fn detail_keys(state: &PidBackupBrowser, id: &NodeId) -> Vec<String> {
        detail_rows_of(state, id)
            .into_iter()
            .map(|(key, _)| key)
            .collect()
    }

    /// Only the rows that carry a value: the plain `(key, value)` fields.
    fn detail_fields(state: &PidBackupBrowser, id: &NodeId) -> Vec<(String, String)> {
        detail_rows_of(state, id)
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect()
    }

    /// A literal value as the detail rows carry it.
    fn plain(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn the_root_details_list_the_manifest_read_only_marker_and_every_warning() {
        let state = detail_state();
        let fields = detail_fields(&state, &NodeId::Root);
        assert_eq!(
            fields[..7],
            [
                (t!("Backup name").into_owned(), plain("Plant backup")),
                (t!("Backup version").into_owned(), plain("7")),
                (t!("Backup type").into_owned(), plain("Full")),
                (t!("Date created").into_owned(), plain("2026-01-02")),
                (t!("SPID").into_owned(), plain("SQPlant")),
                (t!("Backup source").into_owned(), plain("D:/backups/plant.zip")),
                (t!("Restored").into_owned(), plain("D:/tmp/restore")),
            ],
            "the manifest fields come first, then where the backup came from"
        );
        // The one field after them: where restores are kept.
        assert_eq!(
            fields[7..],
            [(
                t!("Cache folder").into_owned(),
                crate::tf!(
                    "{} (the default folder)",
                    crate::io::pid_backup::default_cache_root().display()
                )
                .into_owned()
            )]
        );
        let keys = detail_keys(&state, &NodeId::Root);
        assert_eq!(
            keys[..11],
            [
                t!("Backup name").into_owned(),
                t!("Backup version").into_owned(),
                t!("Backup type").into_owned(),
                t!("Date created").into_owned(),
                t!("SPID").into_owned(),
                t!("Backup source").into_owned(),
                t!("Restored").into_owned(),
                t!("Backup is read-only").into_owned(),
                t!("Warnings").into_owned(),
                plain("no usable Export.dmp"),
                plain("one declared file is missing"),
            ],
            "the read-only marker and every warning follow in order"
        );
        // Then where the restore cache is, with the button that moves it
        // (what it holds is measured when the backup is selected).
        assert_eq!(
            keys[11..14],
            [
                t!("Restore cache").into_owned(),
                t!("Cache folder").into_owned(),
                t!("Change the cache folder…").into_owned(),
            ]
        );
        // The backup's own lines end there; what a section adds to them --
        // the check of the drawings -- comes after, under its heading.
        assert_eq!(keys[14], t!("Check of the drawings").as_ref());
    }

    #[test]
    fn a_root_without_warnings_says_so() {
        let mut state = detail_state();
        update(
            &mut state,
            PidBackupMsg::Loaded(Ok(Arc::new(BackupIndex {
                warnings: Vec::new(),
                read_only: false,
                ..(*detail_index()).clone()
            }))),
        );
        let keys = detail_keys(&state, &NodeId::Root);
        assert!(keys.contains(&t!("No warnings.").into_owned()));
        assert!(!keys.contains(&t!("Backup is read-only").into_owned()));
    }

    #[test]
    fn the_files_group_details_count_the_backup_files() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Files),
            vec![(t!("File count").into_owned(), plain("3"))]
        );
    }

    #[test]
    fn a_dump_file_details_show_its_name_format_size_and_version_line() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::File(0)),
            vec![
                (t!("Backup file name").into_owned(), plain("Export.dmp")),
                (
                    t!("File format").into_owned(),
                    plain("Oracle exp (EXPORT:V12.01.00)")
                ),
                (t!("File size").into_owned(), human_size(2048)),
                (
                    t!("Version line").into_owned(),
                    plain("EXPORT:V12.01.00")
                ),
            ],
            "the dump carries its EXPORT line, and is no archive: it counts no members"
        );
    }

    #[test]
    fn an_archive_file_details_count_its_members() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::File(1)),
            vec![
                (
                    t!("Backup file name").into_owned(),
                    plain("PlantData~2~711.zip")
                ),
                (t!("File format").into_owned(), plain("zip")),
                (t!("File size").into_owned(), human_size(1024)),
                (t!("Members").into_owned(), plain("2")),
            ]
        );
    }

    #[test]
    fn a_file_the_backup_does_not_carry_says_it_is_registered_but_missing() {
        let state = detail_state();
        let rows = detail_rows_of(&state, &NodeId::File(2));
        assert_eq!(
            rows,
            vec![
                (
                    t!("Backup file name").into_owned(),
                    plain("RefData~4~824.zip")
                ),
                (t!("File format").into_owned(), plain("zip")),
                (t!("File size").into_owned(), human_size(0)),
                (t!("Members").into_owned(), plain("0")),
                (t!("Registered but missing").into_owned(), String::new()),
            ]
        );
    }

    #[test]
    fn a_file_member_details_name_its_file_and_the_member() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::FileMember(1, 1)),
            vec![
                (
                    t!("Backup file name").into_owned(),
                    plain("PlantData~2~711.zip")
                ),
                (t!("Member").into_owned(), plain("00/00/b.pid")),
            ]
        );
    }

    #[test]
    fn the_groups_group_details_count_groups_and_drawings() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Groups),
            vec![
                (t!("Group count").into_owned(), plain("3")),
                (t!("Drawing count").into_owned(), plain("3")),
            ]
        );
    }

    #[test]
    fn a_group_details_name_its_path_and_its_drawing_subtree() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Group(vec![0, 0])),
            vec![
                (t!("Group name").into_owned(), plain("00")),
                (t!("Group path").into_owned(), plain("SQPlant / 00")),
                (t!("Drawing count").into_owned(), plain("3")),
            ],
            "the count covers the group's own drawings and its child's"
        );
        assert_eq!(
            detail_rows_of(&state, &NodeId::Group(vec![0])),
            vec![
                (t!("Group name").into_owned(), plain("SQPlant")),
                (t!("Group path").into_owned(), plain("SQPlant")),
                (t!("Drawing count").into_owned(), plain("3")),
            ],
            "the parent counts the drawings of its whole subtree"
        );
    }

    #[test]
    fn a_group_details_translate_the_two_names_the_index_synthesises() {
        let state = detail_state();
        assert_eq!(
            detail_fields(&state, &NodeId::Group(vec![1]))[0],
            (
                t!("Group name").into_owned(),
                t!("Unregistered").into_owned()
            )
        );
        assert_eq!(
            detail_fields(&state, &NodeId::Group(vec![2]))[0],
            (
                t!("Group name").into_owned(),
                t!("(ungrouped)").into_owned()
            )
        );
    }

    #[test]
    fn a_drawing_details_show_its_file_path_metadata_and_open_action() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Drawing(0)),
            vec![
                (t!("Drawing name").into_owned(), plain("A-101")),
                (t!("Group path").into_owned(), plain("SQPlant / 00")),
                (
                    t!("File path").into_owned(),
                    plain("D:/tmp/restore/00/00/a.pid")
                ),
                (t!("Open drawing").into_owned(), String::new()),
                // What a section adds to a drawing that has a file: the
                // offer to import it and say how that went.
                (t!("Importing it").into_owned(), String::new()),
                (
                    t!("Imports the drawing as opening it would, without opening it: what was drawn, what was left out, and the parser's report of the file.")
                        .into_owned(),
                    String::new()
                ),
                (t!("Import and diagnose").into_owned(), String::new()),
                (plain("id"), plain("1")),
                (plain("PATH"), plain("00/00/a.pid")),
            ],
            "the file path, the open action and the import, then every T_DRAWING column"
        );
    }

    #[test]
    fn a_missing_drawing_details_say_why_and_offer_no_open_action() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Drawing(2)),
            vec![
                (t!("Drawing name").into_owned(), plain("C-303")),
                (t!("Group path").into_owned(), plain("SQPlant / 00")),
                (t!("Registered but missing").into_owned(), String::new()),
                (plain("id"), plain("3")),
            ]
        );
        let keys = detail_keys(&state, &NodeId::Drawing(2));
        assert!(!keys.contains(&t!("Open drawing").into_owned()));
        assert!(!keys.contains(&t!("File path").into_owned()));
    }

    #[test]
    fn an_item_type_details_name_it_and_count_its_items() {
        let mut state = detail_state();
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(items_of("UID-1", "Instrument")))),
        );
        assert_eq!(
            detail_rows_of(&state, &NodeId::ItemType(0, 0)),
            vec![
                (t!("Item type").into_owned(), plain("Instrument")),
                (t!("Item count").into_owned(), plain("1")),
                // The items as a table: its titles and row count, then a
                // pair per row.
                (
                    format!(
                        "{} | {} | {}",
                        t!("Item name"),
                        t!("Item class"),
                        t!("Item UID")
                    ),
                    crate::tf!("{} row(s)", 1).into_owned()
                ),
                (plain("TT-101"), plain("PIDInstrument | UID-1")),
            ]
        );
    }

    #[test]
    fn an_item_details_show_its_uid_type_class_and_every_attribute() {
        let mut state = detail_state();
        update(
            &mut state,
            PidBackupMsg::ItemsLoaded(0, Ok(Arc::new(items_of("UID-1", "Instrument")))),
        );
        assert_eq!(
            detail_rows_of(&state, &NodeId::Item(0, 0, 0)),
            vec![
                (t!("Item name").into_owned(), plain("TT-101")),
                (t!("Item UID").into_owned(), plain("UID-1")),
                (t!("Item type").into_owned(), plain("Instrument")),
                (t!("Item class").into_owned(), plain("PIDInstrument")),
                (t!("Item attributes").into_owned(), String::new()),
                (
                    plain("INamedInstrument.MeasuredVariable"),
                    plain("T")
                ),
            ]
        );
    }

    #[test]
    fn the_reference_group_details_count_every_reference_file() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::Ref),
            vec![
                (t!("Reference name").into_owned(), plain("Ref")),
                (t!("Reference file count").into_owned(), plain("3")),
            ],
            "the count covers the whole Ref subtree"
        );
    }

    #[test]
    fn a_reference_directory_details_name_it_and_count_its_subtree() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::RefDir(vec![0])),
            vec![
                (t!("Reference name").into_owned(), plain("Symbols")),
                (t!("Reference file count").into_owned(), plain("2")),
            ]
        );
    }

    #[test]
    fn a_reference_file_details_show_its_name_and_size() {
        let state = detail_state();
        assert_eq!(
            detail_rows_of(&state, &NodeId::RefFile(vec![], 0)),
            vec![
                (t!("Reference file name").into_owned(), plain("readme.txt")),
                (t!("File size").into_owned(), human_size(12)),
            ]
        );
        assert_eq!(
            detail_rows_of(&state, &NodeId::RefFile(vec![0], 1)),
            vec![
                (t!("Reference file name").into_owned(), plain("valve.svg")),
                (t!("File size").into_owned(), human_size(2048)),
            ]
        );
    }

    #[test]
    fn a_node_out_of_range_and_an_idle_panel_have_no_details() {
        let state = detail_state();
        assert!(detail_rows_of(&state, &NodeId::File(99)).is_empty());
        assert!(detail_rows_of(&state, &NodeId::Group(vec![9])).is_empty());
        assert!(detail_rows_of(&state, &NodeId::RefDir(vec![7])).is_empty());
        assert!(detail_rows_of(&state, &NodeId::Item(0, 0, 0)).is_empty());
        assert!(detail_rows_of(&PidBackupBrowser::default(), &NodeId::Root).is_empty());
    }

    #[test]
    fn pid_backup_ui1_loading_elapsed_and_failed_source_are_retained() {
        use std::time::Duration;

        let mut state = PidBackupBrowser::default();
        assert_eq!(state.loading_elapsed(iced::time::Instant::now()), None);
        assert_eq!(state.active_or_failed_source(), None);

        let src = PathBuf::from("D:/samples/bad_backup.zip");
        let _ = update(&mut state, PidBackupMsg::SourcePicked(Some(src.clone())));
        assert!(state.loading_started_at.is_some());
        let started = state.loading_started_at.expect("loading timestamp set");
        assert_eq!(
            state.loading_elapsed(started + Duration::from_secs(7)),
            Some(Duration::from_secs(7))
        );
        assert_eq!(state.active_or_failed_source(), Some(src.as_path()));

        let _ = update(
            &mut state,
            PidBackupMsg::Loaded(Err("not a backup".to_string())),
        );
        assert!(state.loading_started_at.is_none());
        assert_eq!(
            state.loading_elapsed(started + Duration::from_secs(9)),
            None
        );
        assert_eq!(
            state.active_or_failed_source(),
            Some(src.as_path()),
            "failed load keeps the attempted source path"
        );

        let _ = update(&mut state, PidBackupMsg::Close);
        assert_eq!(state.active_or_failed_source(), None);
    }

    #[test]
    fn pid_backup_ui1_screen_group_labels_translate_only_synthetic_groups() {
        use crate::io::pid_backup_browse::{UNGROUPED, UNREGISTERED_GROUP};

        assert_eq!(screen_group_label(UNREGISTERED_GROUP), "未登记图纸");
        assert_eq!(screen_group_label(UNGROUPED), "未分组");
        assert_eq!(screen_group_label("SQPlant"), "SQPlant");
        assert_eq!(screen_group_label("00"), "00");
    }

    #[test]
    fn pid_backup_ui1_filter_drawings_handles_nested_groups_and_query() {
        use crate::io::pid_backup_browse::{DrawingEntry, UNGROUPED, UNREGISTERED_GROUP};

        let index = BackupIndex {
            plant_groups: vec![
                GroupNode {
                    name: "SQPlant".into(),
                    path: "SQPlant".into(),
                    drawings: vec![0],
                    children: vec![GroupNode {
                        name: "Unit100".into(),
                        path: "SQPlant / Unit100".into(),
                        drawings: vec![1, 2],
                        children: Vec::new(),
                    }],
                },
                GroupNode {
                    name: UNREGISTERED_GROUP.into(),
                    path: UNREGISTERED_GROUP.into(),
                    drawings: vec![3],
                    children: Vec::new(),
                },
                GroupNode {
                    name: UNGROUPED.into(),
                    path: UNGROUPED.into(),
                    drawings: vec![4],
                    children: Vec::new(),
                },
            ],
            drawings: vec![
                DrawingEntry {
                    name: "PID-000-INDEX".into(),
                    pid_path: Some(PathBuf::from("0.pid")),
                    group_path: "SQPlant".into(),
                    meta: Vec::new(),
                },
                DrawingEntry {
                    name: "PID-100-A01".into(),
                    pid_path: Some(PathBuf::from("1.pid")),
                    group_path: "SQPlant / Unit100".into(),
                    meta: Vec::new(),
                },
                DrawingEntry {
                    name: "PID-100-MISSING".into(),
                    pid_path: None,
                    group_path: "SQPlant / Unit100".into(),
                    meta: Vec::new(),
                },
                DrawingEntry {
                    name: "LOOSE-a01".into(),
                    pid_path: Some(PathBuf::from("3.pid")),
                    group_path: UNREGISTERED_GROUP.into(),
                    meta: Vec::new(),
                },
                DrawingEntry {
                    name: "FLAT-999".into(),
                    pid_path: Some(PathBuf::from("4.pid")),
                    group_path: UNGROUPED.into(),
                    meta: Vec::new(),
                },
            ],
            ..BackupIndex::default()
        };

        assert_eq!(group_total_drawings(&index.plant_groups[0]), 3);
        assert_eq!(filter_drawings(&index, None, ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            filter_drawings(&index, Some("SQPlant"), ""),
            vec![0, 1, 2],
            "parent group includes nested Unit100 drawings and missing-file entries"
        );
        assert_eq!(
            filter_drawings(&index, Some("SQPlant / Unit100"), ""),
            vec![1, 2]
        );
        assert_eq!(
            filter_drawings(&index, Some(UNREGISTERED_GROUP), ""),
            vec![3]
        );
        assert_eq!(filter_drawings(&index, None, "a01"), vec![1, 3]);
        assert_eq!(filter_drawings(&index, Some("SQPlant"), "missing"), vec![2]);
    }

    #[test]
    fn pid_backup_ui1_item_counts_and_filters_match_type_and_attributes() {
        use crate::io::pid_backup_browse::{DrawingItem, ItemAttribute, ItemGroup};

        let items = DrawingItems {
            drawing_uid: "D-100".into(),
            drawing_name: "PID-100-A01".into(),
            item_count: 3,
            groups: vec![
                ItemGroup {
                    item_type: "Equipment".into(),
                    items: vec![DrawingItem {
                        uid: "EQ-1".into(),
                        label: "V-101".into(),
                        item_type: "Equipment".into(),
                        class: Some("Vessel".into()),
                        attributes: vec![ItemAttribute {
                            interface: "IEquipment".into(),
                            name: "Service".into(),
                            value: "Feed Surge Drum".into(),
                        }],
                    }],
                },
                ItemGroup {
                    item_type: "Instrument".into(),
                    items: vec![
                        DrawingItem {
                            uid: "IN-1".into(),
                            label: "PT-101A".into(),
                            item_type: "Instrument".into(),
                            class: None,
                            attributes: vec![ItemAttribute {
                                interface: "IInstrument".into(),
                                name: "MeasuredVariable".into(),
                                value: "Pressure".into(),
                            }],
                        },
                        DrawingItem {
                            uid: "IN-2".into(),
                            label: "LT-101B".into(),
                            item_type: "Instrument".into(),
                            class: None,
                            attributes: Vec::new(),
                        },
                    ],
                },
            ],
            ..DrawingItems::default()
        };

        assert_eq!(
            item_type_counts(&items),
            vec![("Equipment", 1), ("Instrument", 2)]
        );
        assert_eq!(filter_drawing_items(&items, None, "").len(), 3);
        let instruments = filter_drawing_items(&items, Some("Instrument"), "");
        assert_eq!(instruments.len(), 2);
        let by_attr = filter_drawing_items(&items, None, "surge");
        assert_eq!(by_attr.len(), 1);
        assert_eq!(by_attr[0].label, "V-101");
        let combined = filter_drawing_items(&items, Some("Instrument"), "pressure");
        assert_eq!(combined.len(), 1);
        assert_eq!(combined[0].uid, "IN-1");
    }
}

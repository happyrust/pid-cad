//! Branches of the browse tree that live in modules of their own.
//!
//! A backup holds far more kinds of data than the three trees the panel
//! started with, and every new kind used to mean a new arm in each of
//! [`super::visible_rows`], [`super::detail_lines`] and [`super::update`]. A
//! [`Section`] is one kind of data as a unit instead: the rows it hangs under
//! a row of the tree, the details of its nodes (and lines it adds to the
//! details of nodes that are not its own), and the data it wants read off the
//! UI thread. The three functions hand over to the registered sections, so
//! adding one is a file beside this one and a line in [`SECTIONS`].
//!
//! A section's nodes are [`NodeId::Section`]: its slot in the registry and a
//! path of indices only the section itself reads. The panel folds, unfolds
//! and selects them like any other node.
//!
//! Like the rest of the panel a section does no IO while the tree is drawn.
//! What it cannot answer from the index it asks for with [`Section::wants`]:
//! the panel records the node as [`SectionLoad::Loading`], the application
//! runs [`Section::load`] on a worker thread, and the result comes back as
//! [`super::PidBackupMsg::SectionLoaded`] for the section to draw.

use std::any::Any;
use std::sync::Arc;

use super::{
    database, datamap, diagnostics, drawing_data, manifest, symbol_preview, text_preview,
    DetailLine, Effect, NodeId, PidBackupBrowser, Row,
};
use crate::io::pid_backup_browse::BackupIndex;

/// The sections of the browser, in the order their rows and detail lines
/// appear. A section's position here is the slot its nodes carry.
pub(super) static SECTIONS: &[&dyn Section] = &[
    &manifest::ManifestSection,
    &text_preview::TextPreview,
    &database::DatabaseSection,
    &drawing_data::DrawingDataSection,
    &datamap::DataMapSection,
    &diagnostics::DiagnosticsSection,
    &symbol_preview::SymbolPreview,
];

/// The sections a panel state works with: [`SECTIONS`], unless a test gave
/// the state its own.
#[derive(Clone, Copy)]
pub(super) struct Registry(pub(super) &'static [&'static dyn Section]);

impl Default for Registry {
    fn default() -> Self {
        Registry(SECTIONS)
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Registry({} sections)", self.0.len())
    }
}

/// What a section read off the UI thread, as the section's own type.
#[derive(Clone)]
pub struct SectionData(Arc<dyn Any + Send + Sync>);

impl SectionData {
    /// Wrap what [`Section::load`] read.
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        SectionData(Arc::new(value))
    }

    /// The value, when it is a `T`: a section asks for the type it stored.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl std::fmt::Debug for SectionData {
    /// Never the value: what a section reads may be megabytes of a table.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SectionData(..)")
    }
}

/// Where a node's data is on its way from the worker thread.
#[derive(Debug, Clone)]
pub enum SectionLoad {
    /// The read is in flight.
    Loading,
    /// The data arrived.
    Ready(SectionData),
    /// The read failed, and why.
    Failed(String),
}

/// What a section may read while it answers.
pub(super) struct Cx<'a> {
    /// The whole panel state: what is open, what is selected, what was read.
    pub state: &'a PidBackupBrowser,
    /// The backup being browsed.
    pub index: &'a BackupIndex,
    /// The section's own slot, for the ids of its nodes.
    pub slot: usize,
    /// The search box's text, trimmed and lowercased; empty when nothing
    /// narrows the tree. No section of the panel narrows its rows by it yet.
    #[cfg_attr(not(test), allow(dead_code))]
    pub filter: &'a str,
}

impl Cx<'_> {
    /// The id of the section's node at `path`.
    pub fn node(&self, path: impl Into<Vec<usize>>) -> NodeId {
        NodeId::Section(self.slot, path.into())
    }

    /// The path of `id`, when it is one of this section's nodes.
    pub fn own<'n>(&self, id: &'n NodeId) -> Option<&'n [usize]> {
        match id {
            NodeId::Section(slot, path) if *slot == self.slot => Some(path),
            _ => None,
        }
    }

    /// Whether the user unfolded `id`.
    pub fn is_open(&self, id: &NodeId) -> bool {
        self.state.expanded.contains(id)
    }

    /// Where the data this section asked for about `node` is.
    pub fn load(&self, node: &NodeId) -> Option<&SectionLoad> {
        self.state.section_data.get(&(self.slot, node.clone()))
    }
}

/// What the user just did to a node.
#[derive(Debug, Clone, Copy)]
pub(super) struct Event<'a> {
    /// What was done.
    pub kind: EventKind,
    /// The node it was done to: any node of the tree, not only a section's.
    pub node: &'a NodeId,
}

/// The things a user does to a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EventKind {
    /// Its arrow was pressed; [`Cx::is_open`] says which way it went.
    Toggled,
    /// Its row was clicked.
    Selected,
    /// Its row was double clicked.
    Activated,
}

/// One kind of data the backup holds, as a branch of the tree.
///
/// Every method has a default that contributes nothing, so a section states
/// only what it has.
pub(super) trait Section: Sync {
    /// Whether this section hangs rows under `host`, a row that is not a
    /// section's own. A host gets an expand arrow even when the tree itself
    /// has nothing to unfold there.
    fn hosts(&self, _cx: &Cx<'_>, _host: &NodeId) -> bool {
        false
    }

    /// The section's rows under the open `host`, in display order and all of
    /// its visible subtree at once: the first level at `depth`, the rows
    /// under the ones [`Cx::is_open`] one deeper each. They follow the
    /// host's own children.
    fn rows(&self, _cx: &Cx<'_>, _host: &NodeId, _depth: usize, _rows: &mut Vec<Row>) {}

    /// The detail lines of `id`: all of them for one of the section's nodes,
    /// and whatever the section has to add for any other node.
    fn details(&self, _cx: &Cx<'_>, _id: &NodeId, _lines: &mut Vec<DetailLine>) {}

    /// What the section has to say about `id` early: lines that belong
    /// before the long list a node's own details end with. Asked for a
    /// drawing only, whose details end with every column of the drawing
    /// table: a button down there would be a screen away.
    fn early_details(&self, _cx: &Cx<'_>, _id: &NodeId, _lines: &mut Vec<DetailLine>) {}

    /// The node whose data `event` makes the section need, if any. It is
    /// read once: a node that is loading, loaded or failed is not asked for
    /// again until the backup is.
    fn wants(&self, _cx: &Cx<'_>, _event: &Event<'_>) -> Option<NodeId> {
        None
    }

    /// Read the data of `node`. Runs on a worker thread, with the index
    /// alone: the panel state stays with the UI.
    fn load(&self, _index: &BackupIndex, _node: &NodeId) -> Result<SectionData, String> {
        Err(crate::t!("this node has nothing to read").into_owned())
    }

    /// Whether what this section read is kept for the selected node only. A
    /// section that reads something big about every node the user visits
    /// (the text of a file) says so: the panel lets go of what it read about
    /// the others as the selection moves on, and reads again on the way
    /// back.
    fn keeps_only_selected(&self) -> bool {
        false
    }

    /// Whether something this section had read is still under way and its
    /// lines change while it is: a count of what is done so far. While any
    /// section says so the panel is drawn anew a few times a second
    /// ([`Effect::Tick`]); nothing else draws it while the user's hands are
    /// off it.
    ///
    /// The answer must turn false when the work ends, whether or not its
    /// result has reached the panel yet: a caller that runs the panel's
    /// effects one after the other waits for the clock to stop before it
    /// hands the result over.
    fn in_progress(&self, _cx: &Cx<'_>) -> bool {
        false
    }
}

/// Whether any section has something under way that counts as it goes.
pub(super) fn in_progress(state: &PidBackupBrowser) -> bool {
    let Some(index) = state.index() else {
        return false;
    };
    let filter = filter_of(state);
    state.sections.0.iter().enumerate().any(|(slot, section)| {
        section.in_progress(&Cx {
            state,
            index,
            slot,
            filter: &filter,
        })
    })
}

/// The search box's text as sections and the tree compare it.
pub(super) fn filter_of(state: &PidBackupBrowser) -> String {
    state.filter.trim().to_lowercase()
}

/// The tree's rows with every section's rows spliced in under their hosts.
///
/// A host's section rows follow its own subtree, so the rows of a host with
/// members stay where they were.
pub(super) fn splice(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    filter: &str,
    native: Vec<Row>,
) -> Vec<Row> {
    let sections = state.sections.0;
    if sections.is_empty() {
        return native;
    }
    let cx = |slot| Cx {
        state,
        index,
        slot,
        filter,
    };
    let mut rows = Vec::with_capacity(native.len());
    // Open hosts whose own subtree is still being copied: their depth, and
    // the section rows that go in when it ends.
    let mut waiting: Vec<(usize, Vec<Row>)> = Vec::new();
    for mut row in native {
        while waiting.last().is_some_and(|(depth, _)| row.depth <= *depth) {
            if let Some((_, mut hosted)) = waiting.pop() {
                rows.append(&mut hosted);
            }
        }
        let hosted = sections
            .iter()
            .enumerate()
            .any(|(slot, section)| section.hosts(&cx(slot), &row.id));
        if hosted {
            row.expandable = true;
            row.expanded = row.expanded || state.expanded.contains(&row.id);
            if row.expanded {
                let mut under = Vec::new();
                for (slot, section) in sections.iter().enumerate() {
                    section.rows(&cx(slot), &row.id, row.depth + 1, &mut under);
                }
                waiting.push((row.depth, under));
            }
        }
        rows.push(row);
    }
    while let Some((_, mut hosted)) = waiting.pop() {
        rows.append(&mut hosted);
    }
    rows
}

/// What the sections have to say about `id`, appended to `lines`.
pub(super) fn details(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    id: &NodeId,
    lines: &mut Vec<DetailLine>,
) {
    let filter = filter_of(state);
    for (slot, section) in state.sections.0.iter().enumerate() {
        let cx = Cx {
            state,
            index,
            slot,
            filter: &filter,
        };
        section.details(&cx, id, lines);
    }
}

/// What the sections have to say about `id` before its own long list.
pub(super) fn early_details(
    state: &PidBackupBrowser,
    index: &BackupIndex,
    id: &NodeId,
    lines: &mut Vec<DetailLine>,
) {
    let filter = filter_of(state);
    for (slot, section) in state.sections.0.iter().enumerate() {
        let cx = Cx {
            state,
            index,
            slot,
            filter: &filter,
        };
        section.early_details(&cx, id, lines);
    }
}

/// Tell the sections what the user did, and turn the data they now want into
/// the reads the application should start.
pub(super) fn after(state: &mut PidBackupBrowser, kind: EventKind, node: &NodeId) -> Effect {
    let sections = state.sections.0;
    if sections.is_empty() {
        return Effect::None;
    }
    let Some(index) = state.index().cloned() else {
        return Effect::None;
    };
    // The selection moved on: what was read for the node it left goes.
    if kind == EventKind::Selected {
        state.section_data.retain(|(slot, kept), _| {
            kept == node
                || !sections
                    .get(*slot)
                    .is_some_and(|section| section.keeps_only_selected())
        });
    }
    let filter = filter_of(state);
    let event = Event { kind, node };
    let mut effect = Effect::None;
    for (slot, section) in sections.iter().enumerate() {
        let wanted = section.wants(
            &Cx {
                state,
                index: &index,
                slot,
                filter: &filter,
            },
            &event,
        );
        if let Some(node) = wanted {
            effect = both(effect, request(state, slot, node));
        }
    }
    // A read that counts as it goes was just started: set the clock going,
    // unless it is.
    if !state.ticking && in_progress(state) {
        state.ticking = true;
        effect = both(effect, Effect::Tick);
    }
    effect
}

/// Record that section `slot` reads `node`, unless it already asked.
fn request(state: &mut PidBackupBrowser, slot: usize, node: NodeId) -> Effect {
    let key = (slot, node);
    if state.section_data.contains_key(&key) {
        return Effect::None;
    }
    state.section_data.insert(key.clone(), SectionLoad::Loading);
    Effect::LoadSection {
        generation: state.generation,
        section: slot,
        node: key.1,
    }
}

/// File what a worker thread read. A read started for a backup the panel has
/// since let go of is dropped: its node would name something else by now. So
/// is one whose node is no longer waited for.
pub(super) fn loaded(
    state: &mut PidBackupBrowser,
    generation: u64,
    section: usize,
    node: NodeId,
    result: Result<SectionData, String>,
) {
    if generation != state.generation {
        return;
    }
    // Nobody waits for it either when the selection left the node of a
    // section that keeps only the selected one's data.
    let key = (section, node);
    if !state.section_data.contains_key(&key) {
        return;
    }
    let load = match result {
        Ok(data) => SectionLoad::Ready(data),
        Err(error) => SectionLoad::Failed(error),
    };
    state.section_data.insert(key, load);
}

/// Two effects as one.
pub(super) fn both(first: Effect, second: Effect) -> Effect {
    match (first, second) {
        (Effect::None, effect) | (effect, Effect::None) => effect,
        (Effect::Batch(mut effects), Effect::Batch(more)) => {
            effects.extend(more);
            Effect::Batch(effects)
        }
        (Effect::Batch(mut effects), effect) => {
            effects.push(effect);
            Effect::Batch(effects)
        }
        (effect, Effect::Batch(more)) => {
            let mut effects = vec![effect];
            effects.extend(more);
            Effect::Batch(effects)
        }
        (first, second) => Effect::Batch(vec![first, second]),
    }
}

/// [`Section::load`] of section `slot` on a worker thread, as the drawing
/// items are read.
pub(super) async fn load_in_background(
    registry: Registry,
    index: Arc<BackupIndex>,
    slot: usize,
    node: NodeId,
) -> Result<SectionData, String> {
    let Some(section) = registry.0.get(slot).copied() else {
        return Err(crate::t!("this node has nothing to read").into_owned());
    };
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-pid-backup-section".into())
        .spawn(move || {
            let _ = sender.send(section.load(&index, &node));
        })
        .map_err(|error| {
            crate::tf!("cannot start reading the backup data: {}", error).into_owned()
        })?;
    receiver.await.unwrap_or_else(|_| {
        Err(crate::t!("the backup data read stopped before it finished").into_owned())
    })
}

/// A section for tests: rows under the second backup file and (while a
/// search narrows the tree) under the root, a line on the "backup files"
/// group, and data it reads for its first node.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    pub(crate) struct Fake {
        /// Whether it keeps what it read for the selected node only.
        forgetful: bool,
    }

    /// The registry a test state uses instead of [`SECTIONS`].
    pub(crate) static FAKE_SECTIONS: &[&dyn Section] = &[&Fake { forgetful: false }];

    /// The same section, keeping what it read for the selected node only.
    pub(crate) static FORGETFUL_SECTIONS: &[&dyn Section] = &[&Fake { forgetful: true }];

    /// What [`Fake`] reads for its first node.
    pub(crate) const LOADED: &str = "read off the UI thread";

    /// The node whose read fails.
    pub(crate) const FAILING: usize = 1;

    fn row(id: NodeId, depth: usize, label: &str, expandable: bool, expanded: bool) -> Row {
        Row {
            id,
            depth,
            label: label.to_string(),
            expandable,
            expanded,
            disabled: false,
        }
    }

    impl Section for Fake {
        fn hosts(&self, cx: &Cx<'_>, host: &NodeId) -> bool {
            match host {
                NodeId::File(file) => cx.index.files.get(*file).is_some(),
                NodeId::Root => !cx.filter.is_empty(),
                _ => false,
            }
        }

        fn rows(&self, cx: &Cx<'_>, host: &NodeId, depth: usize, rows: &mut Vec<Row>) {
            match host {
                NodeId::File(file) => {
                    let first = cx.node([*file, 0]);
                    let open = cx.is_open(&first);
                    rows.push(row(first, depth, "fake first", true, open));
                    if open {
                        rows.push(row(
                            cx.node([*file, 0, 0]),
                            depth + 1,
                            "fake leaf",
                            false,
                            false,
                        ));
                    }
                    rows.push(row(
                        cx.node([*file, FAILING]),
                        depth,
                        "fake second",
                        false,
                        false,
                    ));
                }
                NodeId::Root => rows.push(row(
                    cx.node([usize::MAX]),
                    depth,
                    &format!("fake found: {}", cx.filter),
                    false,
                    false,
                )),
                _ => {}
            }
        }

        fn details(&self, cx: &Cx<'_>, id: &NodeId, lines: &mut Vec<DetailLine>) {
            if *id == NodeId::Files {
                lines.push(DetailLine::Field(
                    "fake".to_string(),
                    "on a native node".to_string(),
                ));
            }
            let Some(path) = cx.own(id) else {
                return;
            };
            lines.push(DetailLine::Field(
                "fake path".to_string(),
                format!("{path:?}"),
            ));
            match cx.load(id) {
                None => {}
                Some(SectionLoad::Loading) => {
                    lines.push(DetailLine::Marker("fake loading".to_string()));
                }
                Some(SectionLoad::Ready(data)) => lines.push(DetailLine::Field(
                    "fake data".to_string(),
                    data.get::<String>().cloned().unwrap_or_default(),
                )),
                Some(SectionLoad::Failed(error)) => lines.push(DetailLine::Marker(error.clone())),
            }
        }

        fn wants(&self, cx: &Cx<'_>, event: &Event<'_>) -> Option<NodeId> {
            let read = event.kind == EventKind::Selected
                && cx.own(event.node).is_some_and(|path| path.len() == 2);
            read.then(|| event.node.clone())
        }

        fn load(&self, index: &BackupIndex, node: &NodeId) -> Result<SectionData, String> {
            match node {
                NodeId::Section(_, path) if path.get(1) == Some(&FAILING) => {
                    Err("the fake read failed".to_string())
                }
                NodeId::Section(_, path) => Ok(SectionData::new(format!(
                    "{LOADED}: file {} of {}",
                    path[0],
                    index.files.len()
                ))),
                _ => Err("not a fake node".to_string()),
            }
        }

        fn keeps_only_selected(&self) -> bool {
            self.forgetful
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FAILING, FAKE_SECTIONS, FORGETFUL_SECTIONS, LOADED};
    use super::*;
    use crate::app::Message;
    use crate::ui::dock::Chrome;
    use crate::ui::window::pid_backup_browser::{
        detail_rows, update, view, visible_rows, PidBackupMsg,
    };

    /// The test backup of the panel's own tests, browsed with the fake
    /// section instead of the real ones.
    fn ready() -> PidBackupBrowser {
        let mut state = super::super::tests::ready();
        state.sections = Registry(FAKE_SECTIONS);
        state
    }

    fn labels(state: &PidBackupBrowser) -> Vec<(usize, String)> {
        visible_rows(state)
            .into_iter()
            .map(|row| (row.depth, row.label))
            .collect()
    }

    fn fake(path: &[usize]) -> NodeId {
        NodeId::Section(0, path.to_vec())
    }

    #[test]
    fn a_section_gives_a_leaf_an_arrow_and_hangs_its_rows_under_it() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));
        // `RefData~4~1.zip` has no member: the tree alone would draw a leaf.
        let rows = visible_rows(&state);
        let leaf = rows
            .iter()
            .find(|row| row.id == NodeId::File(1))
            .expect("the second file");
        assert!(leaf.expandable && !leaf.expanded);
        assert!(!rows.iter().any(|row| matches!(row.id, NodeId::Section(..))));

        update(&mut state, PidBackupMsg::Toggle(NodeId::File(1)));
        assert_eq!(
            labels(&state)[3..6],
            [
                (2, "RefData~4~1.zip".to_string()),
                (3, "fake first".to_string()),
                (3, "fake second".to_string()),
            ]
        );
        // The section's own node folds and unfolds like any other.
        update(&mut state, PidBackupMsg::Toggle(fake(&[1, 0])));
        assert_eq!(labels(&state)[5], (4, "fake leaf".to_string()));
        update(&mut state, PidBackupMsg::Toggle(fake(&[1, 0])));
        assert_eq!(labels(&state)[5], (3, "fake second".to_string()));
    }

    #[test]
    fn a_sections_rows_follow_the_hosts_own_children() {
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));
        update(&mut state, PidBackupMsg::Toggle(NodeId::File(0)));
        assert_eq!(
            labels(&state)[2..8],
            [
                (2, "PlantData~2~711.zip".to_string()),
                (3, "00/00/a.pid".to_string()),
                (3, "00/00/b.pid".to_string()),
                (3, "fake first".to_string()),
                (3, "fake second".to_string()),
                (2, "RefData~4~1.zip".to_string()),
            ]
        );
    }

    #[test]
    fn the_search_text_reaches_a_section() {
        let mut state = ready();
        assert!(!labels(&state)
            .iter()
            .any(|(_, label)| label.starts_with("fake")));
        update(&mut state, PidBackupMsg::Filter("  A-1 ".to_string()));
        // Trimmed and lowercased, as the tree compares it; the row sits after
        // the root's own subtree.
        assert_eq!(
            labels(&state).last(),
            Some(&(1, "fake found: a-1".to_string()))
        );
    }

    #[test]
    fn a_section_states_the_details_of_its_nodes_and_adds_to_native_ones() {
        let state = ready();
        assert_eq!(
            detail_rows(&state, &fake(&[1, 0, 0])),
            [("fake path".to_string(), "[1, 0, 0]".to_string())]
        );
        // The native line first, then the section's.
        assert_eq!(
            detail_rows(&state, &NodeId::Files),
            [
                (crate::t!("File count").into_owned(), "2".to_string()),
                ("fake".to_string(), "on a native node".to_string()),
            ]
        );
        // A node of a section that is not registered has nothing to say.
        assert!(detail_rows(&state, &NodeId::Section(7, vec![0])).is_empty());
    }

    #[test]
    fn selecting_a_node_reads_its_data_once_and_shows_it_when_it_lands() {
        let mut state = ready();
        let node = fake(&[1, 0]);
        let effect = update(&mut state, PidBackupMsg::Select(node.clone()));
        let Effect::LoadSection {
            generation,
            section,
            node: asked,
        } = effect
        else {
            panic!("selecting the node asks for its data: {effect:?}");
        };
        assert_eq!((section, &asked), (0, &node));
        assert_eq!(state.selected, Some(node.clone()));
        assert!(detail_rows(&state, &node).contains(&("fake loading".to_string(), String::new())));
        // Asked once: a second click waits for the first read.
        assert_eq!(
            update(&mut state, PidBackupMsg::Select(node.clone())),
            Effect::None
        );

        let index = state.index().cloned().expect("a ready index");
        let result = iced::futures::executor::block_on(load_in_background(
            state.sections,
            index,
            section,
            asked.clone(),
        ));
        assert_eq!(
            update(
                &mut state,
                PidBackupMsg::SectionLoaded {
                    generation,
                    section,
                    node: asked,
                    result,
                }
            ),
            Effect::None
        );
        assert!(detail_rows(&state, &node)
            .contains(&("fake data".to_string(), format!("{LOADED}: file 1 of 2"))));
    }

    #[test]
    fn a_failed_read_is_kept_with_its_reason() {
        let mut state = ready();
        let node = fake(&[0, FAILING]);
        let Effect::LoadSection {
            generation,
            section,
            node: asked,
        } = update(&mut state, PidBackupMsg::Select(node.clone()))
        else {
            panic!("selecting the node asks for its data");
        };
        let index = state.index().cloned().expect("a ready index");
        let result = iced::futures::executor::block_on(load_in_background(
            state.sections,
            index,
            section,
            asked.clone(),
        ));
        assert_eq!(
            result.as_ref().err().map(String::as_str),
            Some("the fake read failed")
        );
        update(
            &mut state,
            PidBackupMsg::SectionLoaded {
                generation,
                section,
                node: asked,
                result,
            },
        );
        assert!(detail_rows(&state, &node)
            .contains(&("the fake read failed".to_string(), String::new())));
        // The failure stands until the backup is read again.
        assert_eq!(update(&mut state, PidBackupMsg::Select(node)), Effect::None);
    }

    #[test]
    fn data_read_for_a_backup_the_panel_let_go_of_is_dropped() {
        let mut state = ready();
        let node = fake(&[1, 0]);
        let Effect::LoadSection { generation, .. } =
            update(&mut state, PidBackupMsg::Select(node.clone()))
        else {
            panic!("selecting the node asks for its data");
        };
        // The same backup is loaded again while the read is in flight: the
        // node means the new index's file now.
        let index = state.index().cloned().expect("a ready index");
        update(&mut state, PidBackupMsg::Loaded(Ok(index)));
        state.sections = Registry(FAKE_SECTIONS);
        update(
            &mut state,
            PidBackupMsg::SectionLoaded {
                generation,
                section: 0,
                node: node.clone(),
                result: Ok(SectionData::new("stale".to_string())),
            },
        );
        assert_eq!(
            detail_rows(&state, &node),
            [("fake path".to_string(), "[1, 0]".to_string())]
        );
    }

    /// Select `node` and hand the panel what its section reads for it.
    fn select_and_read(state: &mut PidBackupBrowser, node: &NodeId) {
        let Effect::LoadSection {
            generation,
            section,
            node: asked,
        } = update(state, PidBackupMsg::Select(node.clone()))
        else {
            panic!("selecting {node:?} asks for its data");
        };
        let index = state.index().cloned().expect("a ready index");
        let result = iced::futures::executor::block_on(load_in_background(
            state.sections,
            index,
            section,
            asked.clone(),
        ));
        update(
            state,
            PidBackupMsg::SectionLoaded {
                generation,
                section,
                node: asked,
                result,
            },
        );
    }

    fn has_data(state: &PidBackupBrowser, node: &NodeId) -> bool {
        detail_rows(state, node)
            .iter()
            .any(|(key, _)| key == "fake data")
    }

    #[test]
    fn a_section_keeps_what_it_read_unless_it_says_otherwise() {
        let (first, second) = (fake(&[1, 0]), fake(&[0, 0]));
        // The default: both nodes keep their data, and neither is read again.
        let mut state = ready();
        select_and_read(&mut state, &first);
        select_and_read(&mut state, &second);
        assert!(has_data(&state, &first) && has_data(&state, &second));
        assert_eq!(
            update(&mut state, PidBackupMsg::Select(first.clone())),
            Effect::None
        );

        // A section that keeps the selected node's data only: the first
        // node's goes when the second is selected, and is read again on the
        // way back.
        let mut state = ready();
        state.sections = Registry(FORGETFUL_SECTIONS);
        select_and_read(&mut state, &first);
        assert!(has_data(&state, &first));
        // Selecting the node it is on keeps what it has.
        assert_eq!(
            update(&mut state, PidBackupMsg::Select(first.clone())),
            Effect::None
        );
        assert!(has_data(&state, &first));
        select_and_read(&mut state, &second);
        assert!(!has_data(&state, &first) && has_data(&state, &second));
        assert_eq!(state.section_data.len(), 1);
        select_and_read(&mut state, &first);
        assert!(has_data(&state, &first) && !has_data(&state, &second));
        // A node no section reads anything for lets go of it too.
        update(&mut state, PidBackupMsg::Select(NodeId::Files));
        assert!(state.section_data.is_empty());
    }

    #[test]
    fn a_read_that_lands_after_the_selection_left_is_not_kept() {
        let (first, second) = (fake(&[1, 0]), fake(&[0, 0]));
        let mut state = ready();
        state.sections = Registry(FORGETFUL_SECTIONS);
        let Effect::LoadSection { generation, .. } =
            update(&mut state, PidBackupMsg::Select(first.clone()))
        else {
            panic!("selecting the node asks for its data");
        };
        // The user moves on while the first read is in flight.
        select_and_read(&mut state, &second);
        update(
            &mut state,
            PidBackupMsg::SectionLoaded {
                generation,
                section: 0,
                node: first.clone(),
                result: Ok(SectionData::new("late".to_string())),
            },
        );
        assert!(!has_data(&state, &first) && has_data(&state, &second));
        assert_eq!(state.section_data.len(), 1);
    }

    #[test]
    fn two_effects_are_carried_together() {
        let load = |node: usize| Effect::LoadSection {
            generation: 0,
            section: 0,
            node: fake(&[node]),
        };
        assert_eq!(both(Effect::None, Effect::None), Effect::None);
        assert_eq!(both(load(1), Effect::None), load(1));
        assert_eq!(both(Effect::None, load(1)), load(1));
        assert_eq!(
            both(load(1), load(2)),
            Effect::Batch(vec![load(1), load(2)])
        );
        assert_eq!(
            both(both(load(1), load(2)), load(3)),
            Effect::Batch(vec![load(1), load(2), load(3)])
        );
    }

    /// The rows a section adds are drawn rows: the arrow it gives its host
    /// is where a press lands, and so are its own rows.
    #[test]
    fn the_drawn_arrow_of_a_host_opens_the_section_and_its_rows_take_clicks() {
        let _guard = crate::headless_ui_lock();
        let mut state = ready();
        update(&mut state, PidBackupMsg::Toggle(NodeId::Files));

        let press = |state: &PidBackupBrowser, label: &str, left_of: bool| -> Vec<PidBackupMsg> {
            let mut ui = iced_test::simulator::Simulator::with_size(
                iced::Settings::default(),
                (340.0, 600.0),
                view(state, 340.0, Chrome::default()),
            );
            let bounds = ui
                .find(label)
                .unwrap_or_else(|error| panic!("the row of {label} is drawn: {error:?}"))
                .visible_bounds()
                .expect("the row is on screen");
            let x = if left_of {
                bounds.x - 7.0
            } else {
                bounds.center_x()
            };
            ui.point_at(iced::Point::new(x, bounds.center_y()));
            let _ = ui.simulate(iced_test::simulator::click());
            ui.into_messages()
                .filter_map(|message| match message {
                    Message::PidBackup(message) => Some(message),
                    _ => None,
                })
                .collect()
        };

        // The arrow the section gave the member-less archive.
        let pressed = press(&state, "RefData~4~1.zip", true);
        assert!(
            pressed
                .iter()
                .any(|message| matches!(message, PidBackupMsg::Toggle(NodeId::File(1)))),
            "{pressed:?}"
        );
        for message in pressed {
            update(&mut state, message);
        }
        // Its rows are drawn now, and a click on one selects it.
        let clicked = press(&state, "fake second", false);
        assert!(
            clicked.iter().any(|message| matches!(
                message,
                PidBackupMsg::Select(NodeId::Section(0, path)) if *path == [1, FAILING]
            )),
            "{clicked:?}"
        );
        // The arrow of the section's own unfoldable row.
        let unfolded = press(&state, "fake first", true);
        assert!(
            unfolded.iter().any(|message| matches!(
                message,
                PidBackupMsg::Toggle(NodeId::Section(0, path)) if *path == [1, 0]
            )),
            "{unfolded:?}"
        );
    }
}

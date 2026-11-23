//! The tags the plant database of a backup gives its items, put on the
//! drawings opened from the backup panel (plan
//! 2026-11-20-pid-symbol-identity, SPEC section 4.4).
//!
//! A drawing file names an item its own way, or not at all; the tag is in
//! the database of the backup the file came out of. A drawing opened from
//! the panel has that database to ask. One opened from its file alone has
//! not, and keeps the names of the file.
//!
//! The database is tens of megabytes and takes seconds to read. It is read
//! on a worker thread, the first time a drawing is opened from a backup,
//! and that drawing does not wait for it: it is shown with the names of its
//! file, and its symbols take their tags when the read is over. Every
//! drawing opened from the backup after that takes them as it opens, before
//! it is shown.
//!
//! Taking the tags is no edit of the user's: the drawing is not marked as
//! changed for it, and no undo step holds it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::Task;

use crate::app::{Message, OpenCADStudio};
use crate::io::pid::DatabaseTagFill;
use crate::io::pid_backup_browse::exp_views::item_tags_in_background;
use crate::ui::window::pid_backup_browser::PidBackupMsg;

/// What a read of the tags of a backup's database came to: the tag of each
/// item that has one, by the item's id, or why there are none.
type TagsRead = Result<Arc<HashMap<String, String>>, String>;

/// The tags of the backup in the panel, and the drawings that wait for
/// them. A backup is told from the one before it by the panel's count of
/// the backups it has let go of (its `generation`).
#[derive(Debug, Default)]
pub(in crate::app) struct BackupTags {
    /// The drawing the panel last asked to have opened, until the open that
    /// came next after the ask is over: that one is it, or the ask came to
    /// nothing.
    asked: Option<Asked>,
    /// The backup whose tags are being read.
    reading: Option<u64>,
    /// What a read came to, and for which backup. The last one that was
    /// kept: a read is kept for the drawings to come.
    read: Option<(u64, TagsRead)>,
    /// The drawings shown while the tags of their backup were being read.
    waiting: Vec<Waiting>,
    /// The drawings that took their tags after they were shown, by their
    /// tabs, until the Properties panel has read each of them anew: what
    /// it says of a symbol selected there is from before. The panel is
    /// read for the drawing in front, so one that took its tags behind
    /// another stays here until it is in front itself.
    to_show: Vec<u64>,
}

/// A drawing the panel asked to have opened.
#[derive(Debug)]
struct Asked {
    /// Its file.
    path: PathBuf,
    /// The id the open it starts goes by.
    open_id: u64,
    /// The backup it is a drawing of.
    generation: u64,
}

/// A drawing that is shown and waits for the tags of its backup.
#[derive(Debug)]
struct Waiting {
    /// The tab it is in, by the id that stays as tabs come and go.
    tab: u64,
    /// The file that tab held when the drawing was put in it. A tab that
    /// holds another by the time the tags arrive holds another drawing.
    path: PathBuf,
    /// The backup it is a drawing of.
    generation: u64,
}

impl OpenCADStudio {
    /// Open a drawing the backup panel named, as any drawing is opened.
    /// When the backup has a database the panel reads, the drawing takes
    /// the tags it gives, and the first drawing to be opened has it read.
    pub(super) fn open_from_the_backup(&mut self, path: PathBuf) -> Task<Message> {
        let open = Task::done(Message::OpenRecent(path.clone()));
        let generation = self.pid_backup_browser.generation;
        // A backup with no database the panel reads has no tags to give.
        let index = self
            .pid_backup_browser
            .index()
            .filter(|index| index.dump_unreadable().is_none())
            .cloned();
        let Some(index) = index else {
            self.pid_backup_tags.asked = None;
            return open;
        };
        let open_id = self.open_id_to_come();
        let tags = &mut self.pid_backup_tags;
        tags.asked = Some(Asked {
            path,
            open_id,
            generation,
        });
        let read = matches!(&tags.read, Some((of, _)) if *of == generation);
        if read || tags.reading == Some(generation) {
            return open;
        }
        tags.reading = Some(generation);
        let reading = Task::perform(item_tags_in_background(index), move |result| {
            Message::PidBackup(PidBackupMsg::TagsRead { generation, result })
        });
        Task::batch([open, reading])
    }

    /// A drawing has been read into tab `i` and is about to be shown. When
    /// it is the one the backup panel asked for, its symbols take the tags
    /// of that backup's database: now, when they have been read; when they
    /// arrive, when they are being read.
    pub(in crate::app) fn tag_the_drawing_the_backup_opened(&mut self, i: usize) {
        let Some(opening) = self.opening.as_ref() else {
            return;
        };
        // An ask is for the open that came next after it, and for no other.
        // An open that began before the ask leaves it for the one to come.
        let Some(asked) = self
            .pid_backup_tags
            .asked
            .take_if(|asked| asked.open_id <= opening.id)
        else {
            return;
        };
        let asked_for = opening.id == asked.open_id
            && opening.source_path.as_deref() == Some(asked.path.as_path());
        if !asked_for {
            return;
        }
        let read = match &self.pid_backup_tags.read {
            Some((of, read)) if *of == asked.generation => Some(read.clone()),
            _ => None,
        };
        match read {
            Some(read) => self.put_the_backup_tags_on(i, &asked.path, &read),
            None if self.pid_backup_tags.reading == Some(asked.generation) => {
                let path = self.tabs[i].current_path.clone().unwrap_or(asked.path);
                self.pid_backup_tags.waiting.push(Waiting {
                    tab: self.tabs[i].id,
                    path,
                    generation: asked.generation,
                });
            }
            // No read of that backup is under way and none is kept: there
            // is nothing to wait for. An ask starts a read unless it finds
            // one.
            None => {}
        }
    }

    /// The read of a backup's tags is over. The drawings that were shown
    /// while it ran take the tags, and so does every drawing opened from
    /// that backup from now on, as it opens.
    pub(super) fn backup_tags_read(&mut self, generation: u64, read: TagsRead) {
        let tags = &mut self.pid_backup_tags;
        if tags.reading == Some(generation) {
            tags.reading = None;
        }
        let (waiting, others): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut tags.waiting)
            .into_iter()
            .partition(|waiting| waiting.generation == generation);
        tags.waiting = others;
        for waiting in waiting {
            // The tab may be closed by now, or hold another file.
            let held = self.tabs.iter().position(|tab| {
                tab.id == waiting.tab && tab.current_path.as_deref() == Some(waiting.path.as_path())
            });
            let Some(i) = held else {
                continue;
            };
            self.put_the_backup_tags_on(i, &waiting.path, &read);
            // The Properties panel is to read what is selected there anew.
            if read.is_ok() {
                self.pid_backup_tags.to_show.push(waiting.tab);
            }
        }
        // They are kept for the drawings to come: the ones opened from the
        // backup while the panel shows it, and the one that was asked for
        // from it and whose file is still being read.
        let tags = &mut self.pid_backup_tags;
        let asked_for = tags
            .asked
            .as_ref()
            .is_some_and(|asked| asked.generation == generation);
        if asked_for || generation == self.pid_backup_browser.generation {
            tags.read = Some((generation, read));
        }
    }

    /// Have the Properties panel read anew what is selected in a drawing
    /// that took its tags after it was shown: the rows of a symbol say its
    /// tag, and the P&ID inspector keeps what it looked up of a label. The
    /// panel is read for the drawing in front and for no other, so this is
    /// asked after every message: a drawing in front as its tags arrive is
    /// read at once, one behind another when it is in front itself,
    /// whatever brings it there.
    pub(in crate::app) fn show_the_tags_taken(&mut self) {
        let to_show = &mut self.pid_backup_tags.to_show;
        if to_show.is_empty() {
            return;
        }
        // A drawing that was closed has no panel left to read.
        let tabs = &self.tabs;
        to_show.retain(|tab| tabs.iter().any(|open| open.id == *tab));
        let Some(front) = tabs.get(self.active_tab).map(|tab| tab.id) else {
            return;
        };
        let listed = to_show.len();
        to_show.retain(|tab| *tab != front);
        if to_show.len() != listed {
            self.refresh_properties();
        }
    }

    /// Give the drawing in tab `i` what a read of its backup's tags came
    /// to -- the tags on its symbols, or the names of the file left as
    /// they are -- and say which on the command line. `path` is its file.
    fn put_the_backup_tags_on(&mut self, i: usize, path: &Path, read: &TagsRead) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        match read {
            Ok(tags) => {
                let fill = self.fill_with_database_tags(i, tags);
                self.command_line.push_info(
                    crate::tf!(
                        "P&ID item tags: {} symbol(s) of {} carry the tag the backup database gives their item; {} differ from the name in the drawing file",
                        fill.tagged,
                        name,
                        fill.differing
                    )
                    .as_ref(),
                );
            }
            // A warning, not an error: the drawing itself opened.
            Err(why) => self.command_line.push_warning(
                crate::tf!(
                    "P&ID item tags: the backup database gave none ({}); the symbols of {} keep the names of the drawing file",
                    why,
                    name
                )
                .as_ref(),
            ),
        }
    }

    /// Put `tags` on the symbols of the drawing in tab `i`.
    ///
    /// The drawing is taken out of its tab meanwhile. Taking the tags is no
    /// edit, and an undo step that is open on the tab hears of the entities
    /// written in the drawing the tab holds: it is to hear of none.
    fn fill_with_database_tags(
        &mut self,
        i: usize,
        tags: &HashMap<String, String>,
    ) -> DatabaseTagFill {
        let scene = &mut self.tabs[i].scene;
        let mut drawing = std::mem::replace(&mut scene.document, document::Document::new());
        let fill = crate::io::pid::fill_database_tags(&mut drawing, tags, |entity| {
            crate::pid::persist::PidAuthor::from_entity(entity).is_some()
        });
        scene.document = drawing;
        fill
    }
}

#[cfg(test)]
mod tests {
    use document::{Document, EntityType, Handle};

    use super::*;
    use crate::io::pid::{fill_database_tags, sheet_for_the_database, tags_for_that_sheet};
    use crate::io::pid_backup_browse::{
        build_index, BackupFile, BackupIndex, DrawingEntry, FileKind, GroupNode,
    };
    use crate::scene::model::object::PropValue;
    use crate::ui::command_line::EntryKind;
    use crate::ui::window::pid_backup_browser::NodeId;

    // ── A backup, its panel and a drawing of it ──────────────────────────

    /// A folder of one test's own under the temp folder. It goes when the
    /// test is over, however the test ended.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("ocs-pid-backup-tags-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a scratch folder");
            Self(root)
        }

        /// A file of the folder that is there to be opened. What is in it
        /// is not read: a test hands over the drawing itself.
        fn drawing(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, b"not read").expect("the file of a drawing");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The program with a drawing open, so that the panel has a window to
    /// be docked in.
    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    /// A backup of these drawings, in the folder of the first: the index
    /// lists an Oracle dump for it, and no such file is there to be read.
    fn backup_of(drawings: &[&Path]) -> Arc<BackupIndex> {
        let root = drawings[0]
            .parent()
            .expect("a drawing is in a folder")
            .to_path_buf();
        Arc::new(BackupIndex {
            source: root.clone(),
            restored: root.clone(),
            backup_root: root,
            read_only: true,
            files: vec![BackupFile {
                name: "Export.dmp".to_string(),
                kind: FileKind::ExportDump,
                version_line: Some("EXPORT:V12.01.00".to_string()),
                ..BackupFile::default()
            }],
            plant_groups: vec![GroupNode {
                name: "Plant".to_string(),
                path: "Plant".to_string(),
                children: Vec::new(),
                drawings: (0..drawings.len()).collect(),
            }],
            drawings: drawings
                .iter()
                .map(|file| DrawingEntry {
                    name: file
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    pid_path: Some(file.to_path_buf()),
                    group_path: "Plant".to_string(),
                    meta: Vec::new(),
                })
                .collect(),
            ..BackupIndex::default()
        })
    }

    /// Put a backup in the panel. What the panel counts it as.
    fn load(app: &mut OpenCADStudio, index: Arc<BackupIndex>) -> u64 {
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(index)));
        app.pid_backup_browser.generation
    }

    /// What running a task came to.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Ran {
        /// The files that were asked to be opened.
        opened: Vec<PathBuf>,
        /// How many reads of a backup's tags answered.
        reads: usize,
    }

    /// How a request to open a file is answered while a task is run.
    enum Opens {
        /// At once, with the next of these as what was read of the file,
        /// which itself is not read. With none left the open is cancelled.
        With(std::vec::IntoIter<Document>),
        /// By the read of the file.
        FromTheFile,
    }

    /// What an open is told when its file is read: `sheet`, as the drawing
    /// that was read of `path`.
    fn read_as(open: u64, path: PathBuf, sheet: Document) -> Message {
        let caches = crate::scene::build_derived_caches_with_progress(&sheet, &|_| {}, None);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Message::FileOpened(open, Ok((name, path, sheet, caches, None)))
    }

    /// Run `task` as the runtime runs one, the application being given
    /// every message it publishes.
    fn drive(app: &mut OpenCADStudio, task: Task<Message>, mut opens: Opens) -> Ran {
        use iced::futures::StreamExt;
        let mut ran = Ran::default();
        let mut streams = Vec::new();
        streams.extend(iced_runtime::task::into_stream(task));
        while let Some(stream) = streams.last_mut() {
            match iced::futures::executor::block_on(stream.next()) {
                Some(iced_runtime::Action::Output(Message::OpenRecent(path))) => {
                    ran.opened.push(path.clone());
                    let read = app.update_root(Message::OpenRecent(path.clone()));
                    let sheets = match &mut opens {
                        Opens::With(sheets) => sheets,
                        Opens::FromTheFile => {
                            streams.extend(iced_runtime::task::into_stream(read));
                            continue;
                        }
                    };
                    // The read of the file is not run.
                    drop(read);
                    let Some(open) = app.opening.as_ref().map(|opening| opening.id) else {
                        continue;
                    };
                    let Some(sheet) = sheets.next() else {
                        let _ = app.update_root(Message::OpenCancel);
                        continue;
                    };
                    let _ = app.update_root(read_as(open, path, sheet));
                }
                Some(iced_runtime::Action::Output(message)) => {
                    if matches!(&message, Message::PidBackup(PidBackupMsg::TagsRead { .. })) {
                        ran.reads += 1;
                    }
                    let next = app.update_root(message);
                    streams.extend(iced_runtime::task::into_stream(next));
                }
                Some(_) => {}
                None => {
                    streams.pop();
                }
            }
        }
        ran
    }

    /// Run `task`, every file it asks to have opened being answered with
    /// the next of `sheets`.
    fn run(app: &mut OpenCADStudio, task: Task<Message>, sheets: Vec<Document>) -> Ran {
        drive(app, task, Opens::With(sheets.into_iter()))
    }

    /// Double click the row of the backup's `n`-th drawing in the panel,
    /// and run what that starts; `sheet` is what is read of its file.
    fn open_from_the_panel(app: &mut OpenCADStudio, n: usize, sheet: Document) -> Ran {
        let task = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(n)));
        run(app, task, vec![sheet])
    }

    /// Open a file as OPEN opens it; `sheet` is what is read of it.
    fn open_from_its_file(app: &mut OpenCADStudio, file: &Path, sheet: Document) -> Ran {
        let open = Task::done(Message::OpenRecent(file.to_path_buf()));
        run(app, open, vec![sheet])
    }

    /// The tags of the sheet, as the read of a backup's database gives
    /// them to the application.
    fn tags_read(generation: u64) -> PidBackupMsg {
        PidBackupMsg::TagsRead {
            generation,
            result: Ok(Arc::new(tags_for_that_sheet())),
        }
    }

    /// The program with a backup of one drawing in its panel, and that
    /// drawing opened from the panel while the tags of the backup are being
    /// read: it is shown with the names of its file, and waits. With it,
    /// the folder of the backup and what the panel counts the backup as.
    fn with_a_drawing_that_waits(name: &str) -> (Scratch, OpenCADStudio, u64) {
        let scratch = Scratch::new(name);
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&drawing]));
        // Another drawing of the backup asked for the database a moment ago.
        app.pid_backup_tags.reading = Some(generation);

        let ran = open_from_the_panel(&mut app, 0, sheet_for_the_database());

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 0
            }
        );
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(drawing.as_path()));
        assert!(app.opening.is_none(), "the open is over");
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(
            app.pid_backup_tags.waiting.len(),
            1,
            "the drawing waits for the tags"
        );
        (scratch, app, generation)
    }

    // ── What a drawing says, and what is said of it ──────────────────────

    /// The symbols of the drawing in tab `i`, in the order they are filed.
    fn symbols(app: &OpenCADStudio, i: usize) -> Vec<&EntityType> {
        app.tabs[i]
            .scene
            .document
            .model_space_entities()
            .filter(|entity| matches!(entity, EntityType::Insert(_)))
            .collect()
    }

    /// What each symbol of the drawing in tab `i` says of its tag: the
    /// tag, where the tag is from, and the name the drawing file has for
    /// the item where that is kept beside the tag.
    fn tags_said_in(app: &OpenCADStudio, i: usize) -> Vec<[Option<String>; 3]> {
        symbols(app, i)
            .into_iter()
            .map(|symbol| {
                ["label", "tag_source", "label_in_file"]
                    .map(|key| crate::io::pid_identity::stated(symbol, key).map(str::to_string))
            })
            .collect()
    }

    /// The same of the drawing in front.
    fn tags_said(app: &OpenCADStudio) -> Vec<[Option<String>; 3]> {
        tags_said_in(app, app.active_tab)
    }

    /// What `tags_said` gives, written with an empty text for nothing said.
    fn tagged(symbols: &[[&str; 3]]) -> Vec<[Option<String>; 3]> {
        symbols
            .iter()
            .map(|said| said.map(|value| (!value.is_empty()).then(|| value.to_string())))
            .collect()
    }

    /// The six valves of `sheet_for_the_database` as the drawing file
    /// leaves them ...
    const BY_THE_FILE: [[&str; 3]; 6] = [
        ["GV-101", "file", ""],
        ["", "", ""],
        ["PIA-0112101B", "file", ""],
        ["N1", "file", ""],
        ["", "", ""],
        ["K-1", "file", ""],
    ];

    /// ... and with the tags of `tags_for_that_sheet` on them.
    const BY_THE_DATABASE: [[&str; 3]; 6] = [
        ["GV-101", "database", ""],
        ["BV1301102B", "database", ""],
        ["PIA-old_0112101A", "database", "PIA-0112101B"],
        ["N1", "file", ""],
        ["", "", ""],
        ["K-1", "file", ""],
    ];

    /// What the Properties panel reads of the tag of what is selected: the
    /// rows of the tag, of where it is from and of the name in the file. A
    /// row the panel has not is `None`.
    fn tag_rows(app: &OpenCADStudio) -> [Option<String>; 3] {
        ["pid_label", "pid_label_source", "pid_label_in_file"].map(|field| {
            app.tabs[app.active_tab]
                .properties
                .sections
                .iter()
                .flat_map(|section| &section.props)
                .find(|row| row.field == field)
                .map(|row| match &row.value {
                    PropValue::ReadOnly(text) | PropValue::EditText(text) => text.clone(),
                    other => format!("{other:?}"),
                })
        })
    }

    /// What `tag_rows` gives, written with an empty text for no row.
    fn rows(read: [&str; 3]) -> [Option<String>; 3] {
        read.map(|value| (!value.is_empty()).then(|| value.to_string()))
    }

    /// Empty the command line: what it holds after that is what was said
    /// from here on. It keeps its last lines only, so a count of them is no
    /// place to read on from.
    fn hush(app: &mut OpenCADStudio) {
        app.command_line.history.clear();
    }

    /// The lines about the tags of a backup that the command line holds,
    /// each with its kind, and without the mark the command line puts
    /// before a line of that kind.
    fn said_of_tags(app: &OpenCADStudio) -> Vec<(EntryKind, String)> {
        let entries = app.command_line.history.iter();
        entries
            .filter_map(|entry| {
                let at = entry.text.find("P&ID item tags")?;
                Some((entry.kind.clone(), entry.text[at..].to_string()))
            })
            .collect()
    }

    /// The line that says the symbols of a drawing took their tags.
    fn taken(tagged: usize, drawing: &str, differing: usize) -> (EntryKind, String) {
        let line = crate::tf!(
            "P&ID item tags: {} symbol(s) of {} carry the tag the backup database gives their item; {} differ from the name in the drawing file",
            tagged,
            drawing,
            differing
        );
        (EntryKind::Info, line.into_owned())
    }

    /// The line that says the database gave a drawing no tags, and why.
    fn not_given(why: &str, drawing: &str) -> (EntryKind, String) {
        let line = crate::tf!(
            "P&ID item tags: the backup database gave none ({}); the symbols of {} keep the names of the drawing file",
            why,
            drawing
        );
        (EntryKind::Warning, line.into_owned())
    }

    /// Nothing said of tags.
    const NOTHING_SAID: [(EntryKind, String); 0] = [];

    // ── The tests ────────────────────────────────────────────────────────

    /// A drawing opened from the panel when the tags of its backup have
    /// been read takes them as it opens: it is shown with them (SPEC
    /// section 4.4). One line says how many symbols took one. The drawing
    /// is as it was read for all that -- not marked as changed, with
    /// nothing to undo -- and the database is not read again.
    #[test]
    fn a_drawing_opened_from_the_panel_takes_the_tags_that_are_read() {
        let scratch = Scratch::new("read");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&drawing]));
        // An earlier drawing of the backup had the database read.
        app.pid_backup_tags.read = Some((generation, Ok(Arc::new(tags_for_that_sheet()))));
        hush(&mut app);
        let errors = app.command_line.error_revision;

        let ran = open_from_the_panel(&mut app, 0, sheet_for_the_database());

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 0
            }
        );
        let i = app.active_tab;
        assert_eq!(app.tabs[i].current_path.as_deref(), Some(drawing.as_path()));
        assert!(app.opening.is_none(), "the open is over");
        assert_eq!(tags_said(&app), tagged(&BY_THE_DATABASE));
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        assert!(!app.tabs[i].dirty, "taking the tags is no edit");
        assert!(
            app.tabs[i].history.undo_stack.is_empty(),
            "and there is nothing to undo"
        );
        assert_eq!(app.command_line.error_revision, errors);
        let tags = &app.pid_backup_tags;
        assert!(tags.asked.is_none() && tags.reading.is_none() && tags.waiting.is_empty());
    }

    /// A drawing opened from the panel while the tags of its backup are
    /// being read does not wait for them (SPEC section 4.4): it is shown
    /// with the names of its file. When the tags arrive its symbols take
    /// them, and the Properties panel reads the selected symbol anew. That
    /// is no edit: the drawing is not marked as changed, its revision does
    /// not move, and an undo step that is open on it holds nothing of it.
    #[test]
    fn a_drawing_is_shown_before_the_tags_are_read_and_takes_them_when_they_are() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("late");
        let i = app.active_tab;
        // The third valve selected: the panel reads what its record says.
        let valve: Handle = symbols(&app, i)[2].common().handle;
        app.tabs[i].scene.select_entity(valve, true);
        app.refresh_properties();
        assert_eq!(
            tag_rows(&app),
            rows(["PIA-0112101B", crate::t!("Drawing file").as_ref(), ""])
        );
        hush(&mut app);
        let errors = app.command_line.error_revision;
        let revision = app.tabs[i].edit_revision;
        // An undo step is open on the drawing when the tags arrive.
        app.push_undo_snapshot(i, "MOVE");

        let _ = app.update(Message::PidBackup(tags_read(generation)));

        assert_eq!(tags_said(&app), tagged(&BY_THE_DATABASE));
        assert_eq!(
            tag_rows(&app),
            rows([
                "PIA-old_0112101A",
                crate::t!("Backup database").as_ref(),
                "PIA-0112101B"
            ]),
            "the panel was not asked: it reads the record anew"
        );
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        assert!(!app.tabs[i].dirty, "taking the tags is no edit");
        assert_eq!(app.tabs[i].edit_revision, revision);
        assert_eq!(app.command_line.error_revision, errors);
        app.finish_all_pending_history();
        let history = &app.tabs[i].history;
        assert!(
            history.pending.is_none() && history.undo_stack.is_empty(),
            "the step that was open holds nothing of it: there is nothing to undo"
        );
        // The tags are the backup's from here on.
        let tags = &app.pid_backup_tags;
        assert!(matches!(&tags.read, Some((of, Ok(_))) if *of == generation));
        assert!(tags.asked.is_none() && tags.reading.is_none() && tags.waiting.is_empty());
        assert!(tags.to_show.is_empty(), "the panel has read the drawing");
    }

    /// The tags go to the drawing that waits for them wherever its tab is
    /// by then: another drawing may be in front. They are its backup's
    /// tags though the panel has let go of that backup since -- and are
    /// then kept for no drawing to come.
    #[test]
    fn the_tags_go_to_the_drawing_that_waits_for_them() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("behind");
        let waits = app.tabs[app.active_tab].id;
        let _ = app.update(Message::TabNew);
        assert_ne!(
            app.tabs[app.active_tab].id, waits,
            "a new drawing is in front"
        );
        let _ = app.on_pid_backup(PidBackupMsg::Close);
        assert_ne!(app.pid_backup_browser.generation, generation);
        hush(&mut app);

        let _ = app.on_pid_backup(tags_read(generation));

        let behind = app
            .tabs
            .iter()
            .position(|tab| tab.id == waits)
            .expect("the drawing is still open");
        assert_ne!(behind, app.active_tab);
        assert_eq!(tags_said_in(&app, behind), tagged(&BY_THE_DATABASE));
        assert!(!app.tabs[behind].dirty);
        assert_eq!(
            tags_said(&app),
            tagged(&[]),
            "the drawing in front has none"
        );
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        let tags = &app.pid_backup_tags;
        assert!(tags.read.is_none(), "the panel shows that backup no longer");
        assert!(tags.reading.is_none() && tags.waiting.is_empty());
    }

    /// The Properties panel is read for the drawing in front. A drawing
    /// that takes its tags behind another has its panel read anew when it
    /// is in front itself: the rows of the symbol that stayed selected in
    /// it say the tag it took. It is read that once.
    #[test]
    fn a_drawing_tagged_behind_another_shows_its_tags_once_it_is_in_front() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("shown");
        let i = app.active_tab;
        let waits = app.tabs[i].id;
        // The third valve selected: the panel reads what its record says.
        let valve: Handle = symbols(&app, i)[2].common().handle;
        app.tabs[i].scene.select_entity(valve, true);
        app.refresh_properties();
        assert_eq!(
            tag_rows(&app),
            rows(["PIA-0112101B", crate::t!("Drawing file").as_ref(), ""])
        );
        let _ = app.update(Message::TabNew);
        assert_ne!(
            app.tabs[app.active_tab].id, waits,
            "a new drawing is in front"
        );

        let _ = app.update(Message::PidBackup(tags_read(generation)));

        let behind = app
            .tabs
            .iter()
            .position(|tab| tab.id == waits)
            .expect("the drawing is still open");
        assert_ne!(behind, app.active_tab);
        assert_eq!(tags_said_in(&app, behind), tagged(&BY_THE_DATABASE));
        assert_eq!(
            app.pid_backup_tags.to_show,
            [waits],
            "its panel is to be read"
        );

        let _ = app.update(Message::TabSwitch(behind));

        assert_eq!(app.active_tab, behind);
        assert_eq!(
            tag_rows(&app),
            rows([
                "PIA-old_0112101A",
                crate::t!("Backup database").as_ref(),
                "PIA-0112101B"
            ])
        );
        assert!(app.pid_backup_tags.to_show.is_empty(), "it is read");
    }

    /// A drawing that took its tags behind another and is closed there has
    /// no panel left to read: it is forgotten.
    #[test]
    fn a_drawing_tagged_behind_another_and_closed_there_is_forgotten() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("left");
        let waits = app.tabs[app.active_tab].id;
        let _ = app.update(Message::TabNew);
        let front = app.tabs[app.active_tab].id;
        assert_ne!(front, waits, "a new drawing is in front");
        let _ = app.update(Message::PidBackup(tags_read(generation)));
        assert_eq!(app.pid_backup_tags.to_show, [waits]);

        let _ = app.update(Message::TabClose(waits));

        assert!(app.tabs.iter().all(|tab| tab.id != waits), "it is closed");
        assert_eq!(app.tabs[app.active_tab].id, front);
        assert!(app.pid_backup_tags.to_show.is_empty());
    }

    /// A read that gave no tags changed nothing in the drawing that waited
    /// for it: one line says why, and the panel is not read anew for it.
    #[test]
    fn a_drawing_given_no_tags_has_its_panel_left_as_it_is() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("given");
        hush(&mut app);

        let _ = app.on_pid_backup(PidBackupMsg::TagsRead {
            generation,
            result: Err("no dump".to_string()),
        });

        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), [not_given("no dump", "A-101.pid")]);
        let tags = &app.pid_backup_tags;
        assert!(tags.waiting.is_empty() && tags.to_show.is_empty());
    }

    /// A drawing waits for the tags of its own backup. Those of another --
    /// a read that began for the backup the panel showed before -- are not
    /// put on it and are not kept: it waits on, and takes its own when
    /// they arrive.
    #[test]
    fn a_drawing_waits_for_the_tags_of_its_own_backup() {
        let (_scratch, mut app, generation) = with_a_drawing_that_waits("other");
        let earlier = generation.wrapping_sub(1);
        hush(&mut app);

        let _ = app.on_pid_backup(tags_read(earlier));

        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        let tags = &app.pid_backup_tags;
        assert_eq!(tags.waiting.len(), 1, "it waits on");
        assert_eq!(tags.reading, Some(generation));
        assert!(tags.read.is_none(), "the panel shows that backup no longer");

        let _ = app.on_pid_backup(tags_read(generation));

        assert_eq!(tags_said(&app), tagged(&BY_THE_DATABASE));
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        let tags = &app.pid_backup_tags;
        assert!(tags.reading.is_none() && tags.waiting.is_empty());
    }

    /// The tags of a backup are kept for the drawing that was asked for
    /// from it and whose file is still being read when they arrive, though
    /// the panel has let go of that backup by then: the drawing takes them
    /// as it opens.
    #[test]
    fn tags_that_arrive_before_the_drawing_asked_for_are_kept_for_it() {
        let scratch = Scratch::new("first");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&drawing]));
        // The panel is asked for the drawing. Neither of the two reads that
        // starts is run: both are answered by hand below.
        drop(app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(0))));
        assert_eq!(app.pid_backup_tags.reading, Some(generation));
        let _ = app.on_pid_backup(PidBackupMsg::Close);
        assert_ne!(app.pid_backup_browser.generation, generation);
        hush(&mut app);

        // The tags arrive first ...
        let _ = app.on_pid_backup(tags_read(generation));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        let tags = &app.pid_backup_tags;
        assert!(
            matches!(&tags.read, Some((of, Ok(_))) if *of == generation),
            "kept for the drawing that is to come"
        );
        assert!(tags.asked.is_some() && tags.reading.is_none());

        // ... and the drawing takes them as it opens: the open the panel
        // asked for, started as the ask has it started.
        let ran = open_from_its_file(&mut app, &drawing, sheet_for_the_database());

        assert_eq!(ran.opened, [drawing.as_path()]);
        let i = app.active_tab;
        assert_eq!(app.tabs[i].current_path.as_deref(), Some(drawing.as_path()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_DATABASE));
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        assert!(!app.tabs[i].dirty, "taking the tags is no edit");
        let tags = &app.pid_backup_tags;
        assert!(tags.asked.is_none() && tags.waiting.is_empty());
    }

    /// Tags that find the tab of their drawing holding another file are
    /// put on nothing, and nothing is said of them: the drawing was saved
    /// under another name while the database was read, or another was
    /// opened into its tab. What a tab holds then is not the backup's file.
    #[test]
    fn tags_are_not_put_on_a_tab_that_holds_another_file() {
        let (scratch, mut app, generation) = with_a_drawing_that_waits("renamed");
        let i = app.active_tab;
        app.tabs[i].current_path = Some(scratch.0.join("saved-as.dwg"));
        hush(&mut app);

        let _ = app.on_pid_backup(tags_read(generation));

        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        let tags = &app.pid_backup_tags;
        assert!(tags.waiting.is_empty(), "it waits no longer");
        assert!(tags.reading.is_none());
        assert!(matches!(&tags.read, Some((of, Ok(_))) if *of == generation));
    }

    /// Tags that find their drawing closed are put on nothing, and nothing
    /// is said of them: not on the file of that drawing either, opened
    /// again from the file itself. They are the backup's tags all the same,
    /// for the drawings opened from it after that.
    #[test]
    fn tags_that_find_their_drawing_closed_are_put_on_nothing() {
        let (scratch, mut app, generation) = with_a_drawing_that_waits("closed");
        let closed = app.tabs[app.active_tab].id;
        let _ = app.update(Message::TabClose(closed));
        assert!(app.tabs.iter().all(|tab| tab.id != closed), "it is closed");
        // Its file is opened again, as OPEN opens it.
        let drawing = scratch.0.join("A-101.pid");
        let ran = open_from_its_file(&mut app, &drawing, sheet_for_the_database());
        assert_eq!(ran.opened, [drawing.as_path()]);
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(drawing.as_path()));
        hush(&mut app);

        let _ = app.on_pid_backup(tags_read(generation));

        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        assert_eq!(
            tags_said(&app),
            tagged(&BY_THE_FILE),
            "the drawing in front was opened from its file"
        );
        let tags = &app.pid_backup_tags;
        assert!(tags.waiting.is_empty(), "it waits no longer");
        assert!(tags.reading.is_none());
        assert!(matches!(&tags.read, Some((of, Ok(_))) if *of == generation));
    }

    /// A database that does not read gives no tags, and the drawing keeps
    /// the names of its file (SPEC section 4.4). One line says why, as a
    /// warning: the drawing itself opened. The database is read by what the
    /// double click starts, not by its answer -- off the thread that asks
    /// -- and it is not read again for the next drawing of the backup.
    #[test]
    fn a_database_that_does_not_read_leaves_the_names_of_the_file() {
        let scratch = Scratch::new("unread");
        let first = scratch.drawing("A-101.pid");
        let second = scratch.drawing("B-202.pid");
        let mut app = app();
        let index = backup_of(&[&first, &second]);
        let generation = load(&mut app, Arc::clone(&index));
        hush(&mut app);
        let errors = app.command_line.error_revision;

        let task = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(0)));
        assert_eq!(format!("{:?}", index.dump), "DumpCell(not read)");
        assert_eq!(app.pid_backup_tags.reading, Some(generation));
        let ran = run(&mut app, task, vec![sheet_for_the_database()]);

        assert_eq!(
            ran,
            Ran {
                opened: vec![first.clone()],
                reads: 1
            }
        );
        assert_eq!(format!("{:?}", index.dump), "DumpCell(unreadable)");
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(first.as_path()));
        assert!(!front.dirty);
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        let why = match &app.pid_backup_tags.read {
            Some((of, Err(why))) if *of == generation => why.clone(),
            other => panic!("the read failed, and that is kept: {other:?}"),
        };
        assert!(why.contains("Export.dmp"), "{why}");
        assert_eq!(said_of_tags(&app), [not_given(&why, "A-101.pid")]);
        assert_eq!(
            app.command_line.error_revision, errors,
            "the drawing opened: that is no error"
        );
        assert!(app.pid_backup_tags.reading.is_none());

        // The next drawing of the backup: the database is not read again.
        hush(&mut app);
        let ran = open_from_the_panel(&mut app, 1, sheet_for_the_database());
        assert_eq!(
            ran,
            Ran {
                opened: vec![second.clone()],
                reads: 0
            }
        );
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(second.as_path()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), [not_given(&why, "B-202.pid")]);
        assert_eq!(app.command_line.error_revision, errors);
    }

    /// The tags that were read are one backup's. A drawing opened from the
    /// backup that came into the panel after it does not take them: the
    /// database of its own backup is asked.
    #[test]
    fn the_tags_of_one_backup_are_not_put_on_a_drawing_of_the_next() {
        let scratch = Scratch::new("next");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let earlier = load(&mut app, backup_of(&[&drawing]));
        app.pid_backup_tags.read = Some((earlier, Ok(Arc::new(tags_for_that_sheet()))));
        // Another backup comes into the panel: one whose database does not
        // read.
        let generation = load(&mut app, backup_of(&[&drawing]));
        assert_ne!(generation, earlier);
        hush(&mut app);

        let ran = open_from_the_panel(&mut app, 0, sheet_for_the_database());

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 1
            },
            "the database of the backup in the panel is asked"
        );
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        let why = match &app.pid_backup_tags.read {
            Some((of, Err(why))) if *of == generation => why.clone(),
            other => panic!("what the last read came to is that backup's: {other:?}"),
        };
        assert_eq!(said_of_tags(&app), [not_given(&why, "A-101.pid")]);
    }

    /// A drawing opened from its file has no database to ask and keeps the
    /// names of the file (SPEC section 4.4), whatever the panel holds: a
    /// backup is in it, the tags of that backup are read, and the file is
    /// one of its drawings. Nor does a drawing take the tags because the
    /// panel asked for an open that came to nothing: one of the same file
    /// that was cancelled, or one of a file that is not there.
    #[test]
    fn a_drawing_opened_from_its_file_keeps_the_names_of_the_file() {
        let scratch = Scratch::new("direct");
        let first = scratch.drawing("A-101.pid");
        let second = scratch.drawing("B-202.pid");
        let third = scratch.drawing("C-303.pid");
        let gone = scratch.0.join("gone.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&first, &second, &gone]));
        app.pid_backup_tags.read = Some((generation, Ok(Arc::new(tags_for_that_sheet()))));
        hush(&mut app);
        let shown = |app: &OpenCADStudio| app.tabs[app.active_tab].current_path.clone();

        let ran = open_from_its_file(&mut app, &first, sheet_for_the_database());
        assert_eq!(ran.opened, [first.as_path()]);
        assert_eq!(shown(&app), Some(first.clone()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);

        // The panel asks for the second drawing and the open is cancelled;
        // then the same file is opened as OPEN opens it.
        let asked = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(1)));
        let ran = run(&mut app, asked, Vec::new());
        assert_eq!(ran.opened, [second.as_path()]);
        assert!(app.opening.is_none(), "the open is cancelled");
        let ran = open_from_its_file(&mut app, &second, sheet_for_the_database());
        assert_eq!(ran.opened, [second.as_path()]);
        assert_eq!(shown(&app), Some(second.clone()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert!(
            app.pid_backup_tags.asked.is_none(),
            "an ask is for the open that came next, and no other"
        );
        assert_eq!(said_of_tags(&app), NOTHING_SAID);

        // The panel asks for a drawing whose file is not there: no open
        // starts. The next open is of another file.
        let ran = open_from_the_panel(&mut app, 2, sheet_for_the_database());
        assert_eq!(ran.opened, [gone.as_path()]);
        assert!(app.opening.is_none(), "no open started");
        let ran = open_from_its_file(&mut app, &third, sheet_for_the_database());
        assert_eq!(ran.opened, [third.as_path()]);
        assert_eq!(shown(&app), Some(third.clone()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert!(app.pid_backup_tags.asked.is_none());

        assert_eq!(said_of_tags(&app), NOTHING_SAID);
    }

    /// An ask is for the open that comes next after it. A drawing whose
    /// open began before the ask and is over after it is not the one asked
    /// for: it keeps the names of its file, and leaves the ask for the open
    /// that is still to come.
    #[test]
    fn an_open_that_began_before_the_ask_leaves_it_for_the_one_to_come() {
        let scratch = Scratch::new("earlier");
        let earlier = scratch.drawing("X-000.pid");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&drawing]));
        app.pid_backup_tags.read = Some((generation, Ok(Arc::new(tags_for_that_sheet()))));
        hush(&mut app);
        let shown = |app: &OpenCADStudio| app.tabs[app.active_tab].current_path.clone();

        // A file is being read, opened as OPEN opens it ...
        drop(app.update_root(Message::OpenRecent(earlier.clone())));
        let began = app.opening.as_ref().map(|opening| opening.id);
        let began = began.expect("the open began");
        // ... the panel is asked for its drawing ...
        let asked = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(0)));
        // ... and the read that began first is over before the panel's
        // begins.
        let _ = app.update_root(read_as(began, earlier.clone(), sheet_for_the_database()));
        assert_eq!(shown(&app), Some(earlier.clone()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert!(
            app.pid_backup_tags.asked.is_some(),
            "the ask is for an open that is still to come"
        );

        let ran = run(&mut app, asked, vec![sheet_for_the_database()]);

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 0
            }
        );
        assert_eq!(shown(&app), Some(drawing.clone()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_DATABASE));
        assert_eq!(said_of_tags(&app), [taken(3, "A-101.pid", 1)]);
        assert!(app.pid_backup_tags.asked.is_none());
    }

    /// A backup with no database the panel reads is asked for no tags: a
    /// drawing of it opens with the names of its file, nothing is read and
    /// nothing is said of tags.
    #[test]
    fn a_backup_without_a_database_is_asked_for_no_tags() {
        let scratch = Scratch::new("no-dump");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let mut index = backup_of(&[&drawing]);
        Arc::get_mut(&mut index)
            .expect("the index is not shared yet")
            .files
            .clear();
        assert!(index.dump_unreadable().is_some());
        load(&mut app, Arc::clone(&index));
        hush(&mut app);

        let ran = open_from_the_panel(&mut app, 0, sheet_for_the_database());

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 0
            }
        );
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(drawing.as_path()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        assert_eq!(format!("{:?}", index.dump), "DumpCell(not read)");
        let tags = &app.pid_backup_tags;
        assert!(tags.asked.is_none() && tags.reading.is_none());
        assert!(tags.read.is_none() && tags.waiting.is_empty());
    }

    /// The panel's ask is its last one. An ask that came to nothing is
    /// forgotten when the panel asks next, though it asks for a drawing of
    /// a backup that has no database: that drawing is not taken for the one
    /// that was asked for before, whose file it is.
    #[test]
    fn an_ask_that_came_to_nothing_is_forgotten_at_the_next() {
        let scratch = Scratch::new("stale");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let earlier = load(&mut app, backup_of(&[&drawing]));
        app.pid_backup_tags.read = Some((earlier, Ok(Arc::new(tags_for_that_sheet()))));
        // The panel asks for the drawing, and the open does not start.
        drop(app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(0))));
        assert!(app.pid_backup_tags.asked.is_some());
        // The same folder comes into the panel as a backup with no database.
        let mut bare = backup_of(&[&drawing]);
        Arc::get_mut(&mut bare)
            .expect("the index is not shared yet")
            .files
            .clear();
        load(&mut app, bare);
        hush(&mut app);

        let ran = open_from_the_panel(&mut app, 0, sheet_for_the_database());

        assert_eq!(
            ran,
            Ran {
                opened: vec![drawing.clone()],
                reads: 0
            }
        );
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(drawing.as_path()));
        assert_eq!(tags_said(&app), tagged(&BY_THE_FILE));
        assert_eq!(said_of_tags(&app), NOTHING_SAID);
        assert!(app.pid_backup_tags.asked.is_none());
    }

    /// A symbol that was drawn in this program takes no tag from the
    /// database, whatever item its record names: it is no item of the
    /// backup's.
    #[test]
    fn a_symbol_drawn_in_this_program_takes_no_tag() {
        let scratch = Scratch::new("drawn-here");
        let drawing = scratch.drawing("A-101.pid");
        let mut app = app();
        let generation = load(&mut app, backup_of(&[&drawing]));
        app.pid_backup_tags.read = Some((generation, Ok(Arc::new(tags_for_that_sheet()))));
        // The second valve of the sheet, the one the database alone names,
        // carries the record of what is drawn here.
        let mut sheet = sheet_for_the_database();
        let second: Handle = sheet
            .model_space_entities()
            .filter(|entity| matches!(entity, EntityType::Insert(_)))
            .nth(1)
            .map(|valve| valve.common().handle)
            .expect("the second valve");
        crate::pid::persist::PidAuthor::new("VALVE", "V-9").store(&mut sheet, second);
        hush(&mut app);

        let ran = open_from_the_panel(&mut app, 0, sheet);

        assert_eq!(ran.opened, [drawing.as_path()]);
        let mut said = BY_THE_DATABASE;
        said[1] = ["", "", ""];
        assert_eq!(tags_said(&app), tagged(&said));
        assert_eq!(said_of_tags(&app), [taken(2, "A-101.pid", 1)]);
    }

    /// The id the next open will go by is the id the next open goes by:
    /// what the panel's ask is told from any other open by.
    #[test]
    fn the_id_to_come_is_the_id_the_next_open_goes_by() {
        let mut app = app();
        let mut last = app.next_open_id();
        for _ in 0..3 {
            let to_come = app.open_id_to_come();
            assert_ne!(to_come, last, "an open has an id of its own");
            assert_eq!(app.open_id_to_come(), to_come, "asking starts no open");
            assert_eq!(app.next_open_id(), to_come);
            last = to_come;
        }
        // Zero is no open's id: the count wraps past it.
        app.open_job_serial = u64::MAX;
        assert_eq!(app.open_id_to_come(), 1);
        assert_eq!(app.next_open_id(), 1);
    }

    // ── The plant backup ─────────────────────────────────────────────────

    /// What the symbols of a drawing say of their tags, counted.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Counted {
        /// Block references that say they are symbols.
        symbols: usize,
        /// Of those, the ones that name an item.
        of_an_item: usize,
        /// The ones that carry a tag.
        tagged: usize,
        /// The tags by where they are from.
        by_the_file: usize,
        by_the_database: usize,
        by_neither: usize,
        /// The ones a name of the drawing file is kept beside the tag of.
        named_otherwise: usize,
    }

    fn counted(app: &OpenCADStudio) -> Counted {
        let mut counted = Counted::default();
        for symbol in symbols(app, app.active_tab) {
            let stated = |key| crate::io::pid_identity::stated(symbol, key);
            if stated("role") != Some("symbol") {
                continue;
            }
            counted.symbols += 1;
            counted.of_an_item += usize::from(stated("item").is_some());
            if stated("label").is_some() {
                counted.tagged += 1;
                match stated("tag_source") {
                    Some("file") => counted.by_the_file += 1,
                    Some("database") => counted.by_the_database += 1,
                    _ => counted.by_neither += 1,
                }
            }
            counted.named_otherwise += usize::from(stated("label_in_file").is_some());
        }
        counted
    }

    /// The sample sheet opened from the panel of the plant backup it came
    /// out of carries the tags of the backup's database (SPEC section 6,
    /// item 2): 354 symbols, every tag marked as the database's and none
    /// with another name kept from the file. Opened from its file alone it
    /// carries the 231 names the file has. Prints what was counted and how
    /// long it took. Soft-skips when the backup is not on this machine.
    #[test]
    #[ignore = "reads the 74 MB plant dump; run with --ignored"]
    fn the_sample_opened_from_its_backup_carries_the_tags_of_the_database() {
        let backup = Path::new(r"D:\work\plant-code\cad\pid-test-data");
        if !backup.join("Export.dmp").is_file() {
            eprintln!("skipped: no plant backup at {}", backup.display());
            return;
        }
        let index = Arc::new(build_index(backup, None).expect("the backup indexes"));
        let named = |n: usize| {
            index.drawings[n]
                .pid_path
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
        };
        let sample = (0..index.drawings.len())
            .find(|n| named(*n).is_some_and(|name| name.starts_with("L M井场")))
            .expect("the sample is a drawing of the backup");
        let file = index.drawings[sample].pid_path.clone().expect("its file");
        let name = named(sample).expect("its name");

        // From its file alone: the names the file has.
        let mut alone = app();
        let open = Task::done(Message::OpenRecent(file.clone()));
        let ran = drive(&mut alone, open, Opens::FromTheFile);
        assert_eq!(
            ran,
            Ran {
                opened: vec![file.clone()],
                reads: 0
            }
        );
        let shown = &alone.tabs[alone.active_tab];
        assert_eq!(shown.current_path.as_deref(), Some(file.as_path()));
        let by_the_file = counted(&alone);
        eprintln!("{name} from its file: {by_the_file:?}");

        // From the panel: the first drawing of the backup has the database
        // read, and is shown before the tags arrive.
        let mut app = app();
        load(&mut app, Arc::clone(&index));
        hush(&mut app);
        let errors = app.command_line.error_revision;
        let clock = std::time::Instant::now();
        let open = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(sample)));
        let ran = drive(&mut app, open, Opens::FromTheFile);
        let took = clock.elapsed();
        assert_eq!(
            ran,
            Ran {
                opened: vec![file.clone()],
                reads: 1
            }
        );
        let front = &app.tabs[app.active_tab];
        assert_eq!(front.current_path.as_deref(), Some(file.as_path()));
        let by_the_database = counted(&app);
        let said = said_of_tags(&app);
        eprintln!(
            "{name} from the panel in {:.2} s with the database read: {by_the_database:?}; {said:?}",
            took.as_secs_f64()
        );
        assert!(!front.dirty, "taking the tags is no edit");
        assert!(front.history.undo_stack.is_empty());
        assert_eq!(app.command_line.error_revision, errors);

        // The same tags put on the drawing that was opened from its file:
        // how long the symbols take to take them.
        let tags = match &app.pid_backup_tags.read {
            Some((_, Ok(tags))) => Arc::clone(tags),
            other => panic!("the tags were read: {other:?}"),
        };
        let i = alone.active_tab;
        let clock = std::time::Instant::now();
        let fill = fill_database_tags(&mut alone.tabs[i].scene.document, &tags, |_| false);
        let filled_in = clock.elapsed();
        let again = fill_database_tags(&mut alone.tabs[i].scene.document, &tags, |_| false);
        eprintln!(
            "{} tag(s) in the database; put on the sheet in {:.2} ms: {fill:?}; again: {again:?}",
            tags.len(),
            filled_in.as_secs_f64() * 1000.0
        );

        // The next drawing of the backup takes them as it opens.
        let next = (0..index.drawings.len())
            .find(|n| *n != sample && named(*n).is_some())
            .expect("the backup has another drawing");
        hush(&mut app);
        let open = app.on_pid_backup(PidBackupMsg::Activate(NodeId::Drawing(next)));
        let ran = drive(&mut app, open, Opens::FromTheFile);
        assert_eq!(ran.reads, 0, "the database is read once");
        let said_of_the_next = said_of_tags(&app);
        eprintln!(
            "{} from the panel: {:?}; {said_of_the_next:?}",
            named(next).unwrap_or_default(),
            counted(&app)
        );

        assert_eq!(
            by_the_file,
            Counted {
                symbols: 592,
                of_an_item: 592,
                tagged: 231,
                by_the_file: 231,
                by_the_database: 0,
                by_neither: 0,
                named_otherwise: 0,
            }
        );
        assert_eq!(
            by_the_database,
            Counted {
                symbols: 592,
                of_an_item: 592,
                tagged: 354,
                by_the_file: 0,
                by_the_database: 354,
                by_neither: 0,
                named_otherwise: 0,
            }
        );
        assert_eq!(said, [taken(354, &name, 0)]);
        assert_eq!(
            fill,
            DatabaseTagFill {
                tagged: 354,
                differing: 0,
                changed: 354,
            }
        );
        assert_eq!(
            again,
            DatabaseTagFill {
                tagged: 354,
                differing: 0,
                changed: 0,
            }
        );
        assert_eq!(counted(&alone), by_the_database);
        assert_eq!(said_of_the_next.len(), 1, "{said_of_the_next:?}");
        assert_eq!(said_of_the_next[0].0, EntryKind::Info);
    }
}

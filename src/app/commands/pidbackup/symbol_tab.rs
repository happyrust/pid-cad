//! A symbol file of a backup, shown as a drawing in a tab of its own (plan
//! 2026-11-21-pid-sym-preview, SP-3).
//!
//! The backup panel draws a `.sym` as a thumbnail in its details. A double
//! click on the file's row, or the button under the thumbnail, shows what
//! the thumbnail draws as a drawing: in a new tab called what the symbol is
//! called, in the symbol's own coordinates -- millimetres about its origin,
//! not turned, not scaled -- with the whole of it in view.
//!
//! The file is read again for it, off the UI thread, the way it is read for
//! the thumbnail. A file that read draws nothing of opens no tab, and the
//! command line says why in the words the details say it in.
//!
//! The tab is read-only the way a `.pid` is. Nothing is written back to the
//! file: the tab has no file of its own, so Save on it is Save As, under
//! the symbol's name. Looking round it is no edit: a zoom or a pan does not
//! mark it modified, and a tab nobody touched closes without a question. An
//! edit marks it as it marks any drawing.
//!
//! One file is shown in one tab: asked for again, it brings its tab to the
//! front. A file is a file of the disk or a member of an archive, so the
//! same symbol in a backup's archive and in its restored folder is two. A
//! tab that is closed is forgotten, and one saved under a name is a drawing
//! like any other from then on: the symbol opens anew beside it.

use document::EntityType;
use iced::Task;

use crate::app::tab_document::DocumentTab;
use crate::app::{Message, OpenCADStudio};
use crate::io::pid_backup_browse::symbol_file::{
    read_drawing_in_background, SymbolDrawing, SymbolSource,
};
use crate::ui::window::pid_backup_browser::{self, PidBackupMsg};

/// What a tab keeps of the symbol file it was added to show.
#[derive(Debug)]
pub(in crate::app) struct SymbolTab {
    /// The file the symbol was read from: one tab is told from another by
    /// it.
    source: SymbolSource,
    /// Set while the view waits to be fitted to its canvas. A tab added
    /// beside the start page has no canvas to take the size of, and is
    /// fitted by the proportions a tab starts with. This is the camera's
    /// count of its moves as that fit left it: a view that has moved since
    /// is one somebody chose, and is left alone.
    refit: Option<u64>,
}

impl DocumentTab {
    /// The symbol file this tab shows, until the tab is saved under a name
    /// of its own: from then on it is a drawing like any other.
    pub(in crate::app) fn shown_symbol(&self) -> Option<&SymbolTab> {
        self.pid_symbol
            .as_ref()
            .filter(|_| self.current_path.is_none())
    }
}

impl OpenCADStudio {
    /// The backup panel asked for a symbol file as a drawing. A tab that
    /// shows the file comes to the front. Otherwise the file is read, off
    /// the UI thread, and [`Self::show_the_symbol_read`] hears what it
    /// held.
    pub(super) fn open_symbol_from_the_backup(
        &mut self,
        source: SymbolSource,
        name: String,
    ) -> Task<Message> {
        if let Some(shown) = self.tab_showing_symbol(&source) {
            return Task::done(Message::TabSwitch(shown));
        }
        if self.no_tab_for_a_symbol(&name) {
            return Task::none();
        }
        Task::perform(read_drawing_in_background(source.clone()), move |result| {
            Message::PidBackup(PidBackupMsg::SymbolRead {
                source: source.clone(),
                name: name.clone(),
                result: result.map(Box::new),
            })
        })
    }

    /// A symbol file was read to be shown as a drawing: show what the read
    /// draws in a tab of its own, or say why there is nothing to show.
    pub(super) fn show_the_symbol_read(
        &mut self,
        source: SymbolSource,
        name: String,
        result: Result<Box<SymbolDrawing>, String>,
    ) -> Task<Message> {
        // A file asked for twice before it was read once is read twice: the
        // second read finds the tab the first one made.
        if let Some(shown) = self.tab_showing_symbol(&source) {
            return Task::done(Message::TabSwitch(shown));
        }
        if self.no_tab_for_a_symbol(&name) {
            return Task::none();
        }
        let drawing = match result {
            Ok(drawing) => *drawing,
            Err(error) => {
                self.say_the_symbol_is_not_opened(&name, &error);
                return Task::none();
            }
        };
        // What the details say of the file, under its thumbnail or in the
        // place of one.
        let notes = pid_backup_browser::symbol_notes(&drawing.file);
        if !drawing.draws() {
            self.say_the_symbol_is_not_opened(&name, &notes.join(" "));
            return Task::none();
        }
        let (held, switch) = self.show_symbol_in_a_new_tab(source, &name, drawing.entities);
        self.command_line.push_output(
            crate::tf!(
                "PIDBACKUPOPEN: symbol \"{}\" opened ({} entities)",
                name,
                held
            )
            .as_ref(),
        );
        // A file read short of its end is drawn as far as it was read: the
        // command line says so as the details do, last, where it is seen.
        for note in notes {
            self.command_line
                .push_warning(crate::tf!("PIDBACKUPOPEN: {}", note).as_ref());
        }
        switch
    }

    /// The tab that shows the symbol file `source`, when one does.
    fn tab_showing_symbol(&self, source: &SymbolSource) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            tab.shown_symbol()
                .is_some_and(|shown| shown.source.is_same_file(source))
        })
    }

    /// Whether there is no tab to be had for a symbol, the command line
    /// being told so. With SDI on the program shows one drawing, and one
    /// that is opened takes the place of the one in front: looking at a
    /// symbol is no reason to close the drawing it is looked at from.
    fn no_tab_for_a_symbol(&mut self, name: &str) -> bool {
        if !self.sdi {
            return false;
        }
        let why = crate::t!("SDI allows one drawing, and a symbol takes a tab of its own");
        self.say_the_symbol_is_not_opened(name, why.as_ref());
        true
    }

    fn say_the_symbol_is_not_opened(&mut self, name: &str, why: &str) {
        self.command_line.push_error(
            crate::tf!("PIDBACKUPOPEN: symbol \"{}\" is not opened: {}", name, why).as_ref(),
        );
    }

    /// Put the entities read of a symbol in a new tab, in front, the whole
    /// of them in view. How many of them the tab holds, and what the switch
    /// to it started.
    ///
    /// The steps are those of a tab a drawing is opened in, less what a
    /// file takes: there is none to remember, to lock or to save to.
    fn show_symbol_in_a_new_tab(
        &mut self,
        source: SymbolSource,
        name: &str,
        entities: Vec<EntityType>,
    ) -> (usize, Task<Message>) {
        // The canvas the tab in front is shown on. The new tab takes its
        // place there, and knows no size of its own until it is first laid
        // out. The start page has no canvas.
        let front = self.tabs[self.active_tab]
            .scene
            .selection
            .borrow()
            .view
            .vp_size;
        let canvas = (front.0 >= 1.0 && front.1 >= 1.0).then_some(front);

        let (mut document, held) = crate::io::pid::symbol_file_document(entities);
        crate::app::style_ops::ensure_standard_styles(&mut document);
        let mut tab = DocumentTab::new_drawing(0);
        tab.tab_title = name.to_owned();
        tab.active_layer = document.header.current_layer_name.clone();
        tab.scene.document = document;
        tab.scene.sync_annotation_scale_from_header();
        tab.scene.rebuild_derived_caches();
        tab.scene.set_current_layout("Model".into());
        if let Some((width, height)) = canvas {
            tab.scene.selection.borrow_mut().view.vp_size = (width, height);
            tab.scene.sync_tiles_from_panes(width, height);
        }
        tab.scene.fit_all();
        // The fit is the program's, not a view somebody chose: the document
        // takes it here, so the tick finds nothing to write and nothing to
        // mark. A drawing just opened is not modified.
        tab.scene.sync_camera_to_document();
        tab.last_synced_camera_gen = tab.scene.camera_generation;
        tab.dirty = false;
        tab.pid_symbol = Some(SymbolTab {
            source,
            refit: canvas.is_none().then_some(tab.scene.camera_generation),
        });
        // The Layer manager and the ribbon list the layers the tab's panel
        // has read: the import's own, here.
        let viewports = tab.scene.viewport_list();
        tab.layers
            .sync_with_viewports(&tab.scene.document.layers, viewports);

        self.tabs.push(tab);
        let index = self.tabs.len() - 1;
        self.sync_pid_view(index);
        self.apply_display_defaults(index);
        // As a new drawing does, the tab takes the Ortho and the running
        // OSNAP that are on: its header states neither, and the switch
        // adopts what the header of the tab it goes to states.
        self.stamp_header_sysvars(index);
        // The program's own switch leaves the tab in front the way any
        // switch leaves one, and brings the ribbon, the grid and the snap
        // over to the new one. It is refused while a recovery prompt is up:
        // the tab is there all the same, behind.
        let switch = self.update(Message::TabSwitch(index));
        if self.active_tab == index {
            self.refresh_properties();
        }
        (held, switch)
    }

    /// A symbol opened beside the start page was fitted without a canvas to
    /// fit it to. Once its tab has been laid out the canvas has a size, and
    /// the fit is made again -- unless the view has moved in between, which
    /// is somebody's doing and stays. Looked at after every message. The
    /// fit is the program's, and no edit.
    pub(in crate::app) fn refit_the_symbol_opened_without_a_canvas(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let Some(fitted) = tab.pid_symbol.as_ref().and_then(|symbol| symbol.refit) else {
            return;
        };
        let (width, height) = tab.scene.selection.borrow().view.vp_size;
        if width < 1.0 || height < 1.0 {
            return;
        }
        if let Some(symbol) = tab.pid_symbol.as_mut() {
            symbol.refit = None;
        }
        if tab.scene.camera_generation != fitted {
            return;
        }
        tab.scene.sync_tiles_from_panes(width, height);
        tab.scene.fit_all();
        tab.scene.sync_camera_to_document();
        tab.last_synced_camera_gen = tab.scene.camera_generation;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::*;
    use crate::app::pid_read_only_view_tests::{
        asks_to_save_on_closing, moved, pose, same, tick, wheel, zoom_extents, Move,
    };
    use crate::io::pid_backup_browse::symbol_file::{made_up, Sketch};
    use crate::io::pid_backup_browse::{build_index_in, RefNode};
    use crate::io::pid_sym_census::CFB_MAGIC;
    use crate::ui::command_line::EntryKind;
    use crate::ui::window::pid_backup_browser::{detail_buttons, symbol_opens, Effect, NodeId};

    // ── A drawing in front, and symbols read for a tab ──────────────────────

    /// The canvas the drawing in front is shown on: wider than high, as
    /// with the panel floating.
    const WIDE: (f32, f32) = (1400.0, 760.0);
    /// A canvas narrower than high, as with the panel docked beside it.
    const NARROW: (f32, f32) = (420.0, 760.0);

    /// The program with a new drawing in front, shown on `canvas`.
    fn with_a_drawing_on(canvas: (f32, f32)) -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        assert!(!app.tabs[i].is_start, "a drawing is in front");
        app.tabs[i].scene.selection.borrow_mut().view.vp_size = canvas;
        app
    }

    /// The program with a new drawing in front, on the wide canvas.
    fn with_a_drawing() -> OpenCADStudio {
        with_a_drawing_on(WIDE)
    }

    /// Where a symbol file would lie. Nothing lies there: a test hands over
    /// what was read of the file.
    fn file(name: &str) -> SymbolSource {
        SymbolSource::File(
            std::env::temp_dir()
                .join("ocs-pid-symbol-tab-no-such-folder")
                .join(name),
        )
    }

    /// What a read of a file that holds the made-up valve gives, and the
    /// sketch its thumbnail is drawn from.
    fn gate_valve() -> (SymbolDrawing, Sketch) {
        let drawing = made_up::drawing(&made_up::valve());
        let sketch = Sketch::clone(&drawing.file.body.as_ref().expect("a body").sketch);
        (drawing, sketch)
    }

    /// Run `task` as the runtime runs one. No window is waited for.
    fn run(app: &mut OpenCADStudio, task: Task<Message>) {
        app.drive_headless_task(task)
            .expect("nothing here waits for a window");
    }

    /// Tell the program what a read of `source` came to, as the read tells
    /// it, and run what that starts.
    fn read(
        app: &mut OpenCADStudio,
        source: &SymbolSource,
        name: &str,
        result: Result<SymbolDrawing, String>,
    ) {
        let task = app.update(Message::PidBackup(PidBackupMsg::SymbolRead {
            source: source.clone(),
            name: name.to_owned(),
            result: result.map(Box::new),
        }));
        run(app, task);
    }

    /// What asking for `source` as a drawing starts, as the panel asks: the
    /// switch to the tab that shows it, the read of the file, or nothing.
    /// It is not run here.
    fn asked(app: &mut OpenCADStudio, source: &SymbolSource, name: &str) -> Task<Message> {
        app.run_pid_backup_effect(Effect::OpenSymbol {
            source: source.clone(),
            name: name.to_owned(),
        })
    }

    /// Ask for `source` as a drawing, as the panel asks, and run what that
    /// starts: the read of the file, when one is started.
    fn ask(app: &mut OpenCADStudio, source: &SymbolSource, name: &str) {
        let task = asked(app, source, name);
        run(app, task);
    }

    /// The program with the made-up valve shown in a tab of its own, in
    /// front of a new drawing. The clock has gone on, and nothing is
    /// modified by having been opened.
    fn shown() -> OpenCADStudio {
        let mut app = with_a_drawing();
        let gate = file("Gate Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        let i = app.active_tab;
        assert!(app.tabs[i].shown_symbol().is_some());
        tick(&mut app);
        tick(&mut app);
        assert!(!app.tabs[i].dirty, "a symbol just opened is not modified");
        assert_eq!(
            app.tabs[i].last_synced_camera_gen, app.tabs[i].scene.camera_generation,
            "the document's view is in step with the camera"
        );
        app
    }

    /// What the command line was told last, which was of `kind`.
    fn told(app: &OpenCADStudio, kind: EntryKind) -> &str {
        let last = app
            .command_line
            .history
            .last()
            .expect("the command line was told something");
        assert_eq!(last.kind, kind, "{}", last.text);
        &last.text
    }

    /// What the command line refused last, without the mark it puts before
    /// an error: its last line is that error.
    fn refused(app: &OpenCADStudio) -> &str {
        let line = told(app, EntryKind::Error);
        let said = app
            .command_line
            .last_error
            .as_deref()
            .expect("an error was told");
        assert!(line.ends_with(said), "{line}");
        said
    }

    /// The line that says a symbol was opened.
    fn opened(name: &str, entities: usize) -> String {
        crate::tf!(
            "PIDBACKUPOPEN: symbol \"{}\" opened ({} entities)",
            name,
            entities
        )
        .into_owned()
    }

    /// The line that says a symbol was not opened, and why.
    fn not_opened(name: &str, why: &str) -> String {
        crate::tf!("PIDBACKUPOPEN: symbol \"{}\" is not opened: {}", name, why).into_owned()
    }

    /// Whether the plan view of the tab in front takes in the whole of the
    /// rectangle `sketch` covers. A sketch of nothing is taken in.
    fn takes_in(app: &OpenCADStudio, sketch: &Sketch) -> bool {
        let Some((min_x, min_y, max_x, max_y)) = sketch.bounds() else {
            return true;
        };
        let ([x, y], [width, height], _) = app.tabs[app.active_tab].scene.view_window();
        x - width / 2.0 <= min_x
            && max_x <= x + width / 2.0
            && y - height / 2.0 <= min_y
            && max_y <= y + height / 2.0
    }

    /// How much of the plan view of the tab in front the rectangle `sketch`
    /// covers takes up, across and up: the whole of it is in view, or this
    /// panics.
    fn share_of_the_view(app: &OpenCADStudio, sketch: &Sketch) -> (f64, f64) {
        let (min_x, min_y, max_x, max_y) = sketch.bounds().expect("something is drawn");
        let window = app.tabs[app.active_tab].scene.view_window();
        assert!(
            takes_in(app, sketch),
            "({min_x}, {min_y})-({max_x}, {max_y}) is not all inside {window:?}"
        );
        let (_, [width, height], _) = window;
        ((max_x - min_x) / width, (max_y - min_y) / height)
    }

    /// Whether the document of the tab in front holds the view the tab is
    /// shown at: the view a file saved from the tab opens at.
    fn document_holds_the_view(app: &OpenCADStudio) -> bool {
        let document = &app.tabs[app.active_tab].scene.document;
        let held = document
            .vports
            .iter()
            .find(|view| view.name.eq_ignore_ascii_case("*Active"));
        held.is_some_and(|view| {
            let at = [
                view.view_target.x,
                view.view_target.y,
                view.view_height / 2.0,
            ];
            same(at, pose(app))
        })
    }

    // ── The tests ──────────────────────────────────────────────────────────────

    /// What was read of a symbol is put in a new tab, in front: called what
    /// the symbol is called, with no file of its own, holding the entities
    /// that were read and not modified.
    #[test]
    fn what_was_read_of_a_symbol_is_shown_in_a_tab_of_its_own() {
        let mut app = with_a_drawing();
        let tabs = app.tabs.len();
        let (valve, sketch) = gate_valve();
        let count = valve.entities.len();
        read(&mut app, &file("Gate Valve.sym"), "Gate Valve", Ok(valve));

        assert_eq!(app.tabs.len(), tabs + 1);
        let i = app.active_tab;
        assert_eq!(i, tabs, "the new tab is in front");
        let tab = &app.tabs[i];
        assert_eq!(tab.tab_display_name(), "Gate Valve");
        assert_eq!(tab.current_path, None);
        assert!(tab.shown_symbol().is_some());
        // In model space, the entities that were read: what the thumbnail
        // draws, where it draws it.
        assert_eq!(tab.scene.current_layout, "Model");
        let held = &tab.scene.document;
        assert_eq!(held.model_space_entities().count(), count);
        assert_eq!(Sketch::of(held.model_space_entities()), sketch);
        // Its fill is among what the scene draws.
        let fills = held
            .model_space_entities()
            .filter(|entity| matches!(entity, EntityType::Hatch(_)))
            .count();
        assert!(fills > 0, "the valve has a fill");
        assert_eq!(tab.scene.hatches.len(), fills);
        // On the import's layers, which the tab's panel lists.
        for layer in ["PID-SYMBOL", "PID-FILL"] {
            assert!(
                tab.layers.layers.iter().any(|listed| listed.name == layer),
                "{layer} is listed"
            );
        }
        // Nothing of it is selected: the properties panel shows what it
        // shows of a drawing.
        assert!(!tab.properties.sections.is_empty());
        // A drawing just opened is not modified, and the document holds
        // the view it is shown at.
        assert!(!tab.dirty);
        assert_eq!(tab.last_synced_camera_gen, tab.scene.camera_generation);
        assert!(document_holds_the_view(&app));
        // The command line says what was opened.
        assert_eq!(told(&app, EntryKind::Output), opened("Gate Valve", count));
    }

    /// The whole symbol is in view, and takes up most of it one way: on a
    /// canvas wider than high, and on one narrower than high, as the panel
    /// docked beside the drawing leaves it.
    #[test]
    fn the_whole_symbol_is_in_view_whatever_the_shape_of_the_canvas() {
        for canvas in [WIDE, NARROW] {
            let mut app = with_a_drawing_on(canvas);
            let front = app.active_tab;
            let (valve, sketch) = gate_valve();
            read(&mut app, &file("Gate Valve.sym"), "Gate Valve", Ok(valve));
            let i = app.active_tab;
            assert_ne!(i, front);
            // The tab took the place of the one in front, on its canvas,
            // and is owed no fit.
            assert_eq!(app.tabs[i].scene.selection.borrow().view.vp_size, canvas);
            assert!(app.tabs[i]
                .shown_symbol()
                .is_some_and(|symbol| symbol.refit.is_none()));
            let (across, up) = share_of_the_view(&app, &sketch);
            assert!(
                across.max(up) >= 0.5,
                "{canvas:?}: the symbol takes up {across} of the view across and {up} up"
            );
        }
    }

    /// Beside the start page there is no canvas to take the size of. The
    /// symbol is fitted when its tab has one: by the next message after the
    /// tab was laid out. The fit is no edit.
    #[test]
    fn a_symbol_opened_beside_the_start_page_is_fitted_once_its_canvas_has_a_size() {
        let mut app = OpenCADStudio::new_for_test();
        assert!(
            app.tabs[app.active_tab].is_start,
            "the start page is in front"
        );
        let (valve, sketch) = gate_valve();
        read(&mut app, &file("Gate Valve.sym"), "Gate Valve", Ok(valve));
        let i = app.active_tab;
        assert!(!app.tabs[i].is_start, "the symbol is in front");
        let owed = |app: &OpenCADStudio| {
            app.tabs[i]
                .shown_symbol()
                .is_some_and(|symbol| symbol.refit.is_some())
        };
        assert!(owed(&app), "fitted without a canvas, the fit is owed");
        assert!(!app.tabs[i].dirty);

        // A message before the tab is laid out changes nothing.
        let fitted = app.tabs[i].scene.camera_generation;
        tick(&mut app);
        assert!(owed(&app));
        assert_eq!(app.tabs[i].scene.camera_generation, fitted);

        // The tab is laid out, beside a docked panel: the next message
        // fits the symbol to the canvas it has.
        app.tabs[i].scene.selection.borrow_mut().view.vp_size = NARROW;
        tick(&mut app);
        assert!(!owed(&app));
        assert!(app.tabs[i].scene.camera_generation > fitted, "fitted anew");
        let (across, up) = share_of_the_view(&app, &sketch);
        assert!(across.max(up) >= 0.5, "{across} across, {up} up");
        // The document holds the view, and nothing is modified: not now,
        // and not when the clock has gone on.
        assert_eq!(
            app.tabs[i].last_synced_camera_gen,
            app.tabs[i].scene.camera_generation
        );
        assert!(document_holds_the_view(&app));
        tick(&mut app);
        tick(&mut app);
        assert!(!app.tabs[i].dirty);
        assert!(!asks_to_save_on_closing(&mut app));
    }

    /// A view that was moved before the canvas had a size is one somebody
    /// chose: the fit that was owed is not made over it.
    #[test]
    fn a_view_moved_before_the_canvas_had_a_size_is_left_alone() {
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start);
        let gate = file("Gate Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        let i = app.active_tab;

        moved(&mut app, zoom_extents);
        let chosen = pose(&app);
        let moves = app.tabs[i].scene.camera_generation;
        app.tabs[i].scene.selection.borrow_mut().view.vp_size = NARROW;
        tick(&mut app);
        assert!(app.tabs[i]
            .shown_symbol()
            .is_some_and(|symbol| symbol.refit.is_none()));
        assert_eq!(app.tabs[i].scene.camera_generation, moves);
        assert!(same(pose(&app), chosen), "the view is where it was put");
        assert!(!app.tabs[i].dirty);
    }

    /// One file is shown in one tab. Asked for again it comes to the
    /// front; another file takes another tab; a tab that was closed is
    /// forgotten, and its file opens anew.
    #[test]
    fn one_symbol_file_is_shown_in_one_tab() {
        let mut app = with_a_drawing();
        let drawing = app.active_tab;
        let tabs = app.tabs.len();
        let gate = file("Gate Valve.sym");
        let globe = file("Globe Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        let shown = app.active_tab;
        assert_eq!((app.tabs.len(), shown), (tabs + 1, tabs));

        // From behind the drawing, the panel asks for the file again: its
        // tab comes to the front, and nothing is said.
        let _ = app.update(Message::TabSwitch(drawing));
        assert_eq!(app.active_tab, drawing);
        let said = app.command_line.history.len();
        ask(&mut app, &gate, "Gate Valve");
        assert_eq!((app.tabs.len(), app.active_tab), (tabs + 1, shown));
        assert_eq!(app.command_line.history.len(), said, "nothing was said");

        // Two reads of the one file, the second over after the first made
        // its tab: one tab.
        let _ = app.update(Message::TabSwitch(drawing));
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        assert_eq!((app.tabs.len(), app.active_tab), (tabs + 1, shown));
        assert_eq!(app.command_line.history.len(), said, "nothing was opened");

        // Another file, and the member of an archive that is called the
        // same, are other symbols: a tab each.
        read(&mut app, &globe, "Globe Valve", Ok(gate_valve().0));
        assert_eq!((app.tabs.len(), app.active_tab), (tabs + 2, tabs + 1));
        let member = SymbolSource::Member {
            archive: PathBuf::from("no-such-archive.zip"),
            member: 0,
            name: "Gate Valve.sym".to_string(),
        };
        read(&mut app, &member, "Gate Valve", Ok(gate_valve().0));
        assert_eq!((app.tabs.len(), app.active_tab), (tabs + 3, tabs + 2));
        // The member asked for again, at another place in its archive.
        let again = SymbolSource::Member {
            archive: PathBuf::from("no-such-archive.zip"),
            member: 7,
            name: "Gate Valve.sym".to_string(),
        };
        let _ = app.update(Message::TabSwitch(drawing));
        ask(&mut app, &again, "Gate Valve");
        assert_eq!((app.tabs.len(), app.active_tab), (tabs + 3, tabs + 2));

        // The first tab is closed, without a question: nobody touched it.
        let _ = app.update(Message::TabSwitch(shown));
        assert!(!asks_to_save_on_closing(&mut app));
        assert_eq!(app.tabs.len(), tabs + 2);
        // Its file is shown in a new one.
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        assert_eq!(app.tabs.len(), tabs + 3);
        let i = app.active_tab;
        assert_eq!(app.tabs[i].tab_display_name(), "Gate Valve");
        assert!(app.tabs[i]
            .shown_symbol()
            .is_some_and(|symbol| symbol.source == gate));
    }

    /// Looking round a symbol is no edit, whichever way the view is moved
    /// and whatever tab is looked at in between: it is not marked modified,
    /// and closes without a question.
    #[test]
    fn looking_round_a_symbol_is_no_edit() {
        let moves: [(&str, Move); 2] = [("the wheel", wheel), ("ZOOM EXTENTS", zoom_extents)];
        for (how, by) in moves {
            let mut app = shown();
            let i = app.active_tab;
            moved(&mut app, by);
            assert!(!app.tabs[i].dirty, "{how}: looking round is not an edit");

            // Away to the drawing behind it, and back.
            let _ = app.update(Message::TabSwitch(i - 1));
            assert_eq!(app.active_tab, i - 1);
            tick(&mut app);
            let _ = app.update(Message::TabSwitch(i));
            assert_eq!(app.active_tab, i);
            tick(&mut app);
            tick(&mut app);
            assert!(
                !app.tabs[i].dirty,
                "{how}: nor is leaving it and coming back"
            );
            assert!(
                !asks_to_save_on_closing(&mut app),
                "{how}: nothing to save, nothing to ask"
            );
        }
    }

    /// An edit marks the tab as it marks any drawing, and it stays marked
    /// whatever the view does afterwards.
    #[test]
    fn an_edit_of_a_symbol_is_an_edit() {
        let mut app = shown();
        let i = app.active_tab;
        let drawn = app.automation_op(r#"{"op":"run","cmd":"LINE 0,0 100,100"}"#);
        assert_eq!(drawn["ok"], true, "{drawn}");
        assert!(app.tabs[i].dirty, "a line drawn is an edit");

        moved(&mut app, zoom_extents);
        assert!(app.tabs[i].dirty, "the edit is still to save");
        assert!(
            asks_to_save_on_closing(&mut app),
            "there is an edit to save"
        );
    }

    /// The tab has no file to write to: Save on it is Save As, and the
    /// name that is offered is the symbol's.
    #[test]
    fn save_on_a_symbol_is_save_as_under_its_name() {
        let mut app = shown();
        let i = app.active_tab;
        assert_eq!(app.direct_save_path(i), None);
        let _ = app.open_save_dialog_window(i);
        let (extension, _) = crate::io::parse_save_format(&app.save_dialog_format);
        assert_eq!(app.save_dialog_filename, format!("Gate Valve.{extension}"));
    }

    /// Saved under a name the tab is a drawing like any other: its view is
    /// saved with its file, and the symbol opens anew beside it.
    #[test]
    fn a_symbol_saved_under_a_name_is_a_drawing_from_then_on() {
        let mut app = shown();
        let i = app.active_tab;
        let tabs = app.tabs.len();
        // The name a Save As leaves the tab with. Nothing is written here.
        app.tabs[i].current_path = Some(std::env::temp_dir().join("ocs-sp3-gate-valve.dwg"));
        assert!(app.tabs[i].shown_symbol().is_none());
        moved(&mut app, wheel);
        assert!(app.tabs[i].dirty, "the view is saved with the file");

        let gate = file("Gate Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        assert_eq!(app.tabs.len(), tabs + 1, "the symbol opens anew");
        assert_ne!(app.active_tab, i);
    }

    /// A tab whose drawing is replaced -- by the `new` of a script, here --
    /// shows the symbol no longer: the symbol opens anew beside it.
    #[test]
    fn a_tab_given_another_drawing_shows_the_symbol_no_longer() {
        let mut app = shown();
        let i = app.active_tab;
        let tabs = app.tabs.len();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        assert_eq!((app.tabs.len(), app.active_tab), (tabs, i));
        assert!(app.tabs[i].shown_symbol().is_none());

        let gate = file("Gate Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        assert_eq!(app.tabs.len(), tabs + 1, "the symbol opens anew");
        assert_ne!(app.active_tab, i);
    }

    /// A file that is shown is not read again. What the ask started is
    /// kept, and the tab is saved under a name before it runs: the tab
    /// shows the symbol no longer, so a read, had one been started, would
    /// be heard -- no file lies there, and the command line would say so.
    /// What was started is the switch to the tab, and nothing else.
    #[test]
    fn a_file_that_is_shown_is_not_read_again() {
        let mut app = shown();
        let shown = app.active_tab;
        let gate = file("Gate Valve.sym");
        let _ = app.update(Message::TabSwitch(shown - 1));
        assert_eq!(app.active_tab, shown - 1);

        let answer = asked(&mut app, &gate, "Gate Valve");
        app.tabs[shown].current_path = Some(std::env::temp_dir().join("ocs-sp3-gate-valve.dwg"));
        let said = app.command_line.history.len();
        run(&mut app, answer);
        assert_eq!(app.active_tab, shown, "the tab came to the front");
        assert_eq!(app.command_line.history.len(), said, "nothing was read");
    }

    /// The tab is set up as a new drawing's is: Ortho stays as it is set,
    /// and the canvas has the background the program is set to.
    #[test]
    fn a_symbol_is_opened_as_a_new_drawing_is() {
        let mut app = with_a_drawing();
        app.ortho_mode = true;
        let background = [0.2, 0.3, 0.4, 1.0];
        app.default_bg_color = Some(background);
        let gate = file("Gate Valve.sym");
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        let i = app.active_tab;
        assert!(app.tabs[i].shown_symbol().is_some());
        assert!(app.ortho_mode, "Ortho is still on");
        assert_eq!(app.tabs[i].bg_color, Some(background));
    }

    /// What draws nothing opens no tab, and the command line says why in
    /// the words the details say it in.
    #[test]
    fn what_draws_nothing_opens_no_tab_and_the_command_line_says_why() {
        let mut app = with_a_drawing();
        let (tabs, front) = (app.tabs.len(), app.active_tab);
        // A body of placeholders, a body that holds nothing, and a file
        // whose deciding stream was read short.
        for geometry in [
            made_up::placeholder(),
            made_up::empty(),
            made_up::valve_short(&["PSMcluster0"], &[]),
        ] {
            let nothing = made_up::drawing(&geometry);
            let why = pid_backup_browser::symbol_notes(&nothing.file);
            assert!(!why.is_empty(), "{:?}", nothing.file.reading);
            read(&mut app, &file("Nothing.sym"), "Nothing", Ok(nothing));
            assert_eq!((app.tabs.len(), app.active_tab), (tabs, front));
            assert_eq!(refused(&app), not_opened("Nothing", &why.join(" ")));
        }
        // A read that failed: what the read says.
        let failed = "cannot read Nothing.sym: it went away".to_string();
        read(
            &mut app,
            &file("Nothing.sym"),
            "Nothing",
            Err(failed.clone()),
        );
        assert_eq!((app.tabs.len(), app.active_tab), (tabs, front));
        assert_eq!(refused(&app), not_opened("Nothing", &failed));
    }

    /// A file read short of its end is shown as far as it was read, and the
    /// command line says so after it has said what was opened.
    #[test]
    fn a_symbol_read_in_part_is_shown_with_what_the_details_say_of_it() {
        let mut app = with_a_drawing();
        let tabs = app.tabs.len();
        let part = made_up::drawing(&made_up::valve_short(&[], &[made_up::NORMAL]));
        let why = pid_backup_browser::symbol_notes(&part.file);
        assert_eq!(why.len(), 1, "{why:?}");
        let count = part.entities.len();
        let said = app.command_line.history.len();
        read(&mut app, &file("Gate Valve.sym"), "Gate Valve", Ok(part));

        assert_eq!(app.tabs.len(), tabs + 1);
        let lines = &app.command_line.history[said..];
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].kind, EntryKind::Output);
        assert_eq!(lines[0].text, opened("Gate Valve", count));
        assert_eq!(lines[1].kind, EntryKind::Warning);
        assert!(lines[1].text.contains(&why[0]), "{}", lines[1].text);
    }

    /// The panel's ask is answered by a read of the file itself. A file
    /// that is no symbol file, one that opens as one and holds nothing, and
    /// one that is not there open no tab; the command line says of each
    /// what a read of it says.
    #[test]
    fn a_file_that_does_not_read_as_a_symbol_opens_no_tab() {
        let root = std::env::temp_dir().join(format!("ocs-pid-symbol-tab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a scratch folder");
        let mut hollow = CFB_MAGIC.to_vec();
        hollow.extend([0u8; 256]);
        let files: [(&str, &[u8]); 2] = [("Plain", b"PLAIN-ASCII-NOT-CFB"), ("Hollow", &hollow)];

        let mut app = with_a_drawing();
        let (tabs, front) = (app.tabs.len(), app.active_tab);
        for (name, bytes) in files {
            let path = root.join(format!("{name}.sym"));
            std::fs::write(&path, bytes).expect("a file");
            let source = SymbolSource::File(path);
            let drawing = source.read_drawing().expect("the file is there");
            assert!(!drawing.draws(), "{name}");
            let why = pid_backup_browser::symbol_notes(&drawing.file);
            assert!(!why.is_empty(), "{name}");

            ask(&mut app, &source, name);
            assert_eq!((app.tabs.len(), app.active_tab), (tabs, front), "{name}");
            assert_eq!(refused(&app), not_opened(name, &why.join(" ")));
        }
        let gone = SymbolSource::File(root.join("Gone.sym"));
        let why = gone.read_drawing().expect_err("no file is there");
        ask(&mut app, &gone, "Gone");
        assert_eq!((app.tabs.len(), app.active_tab), (tabs, front));
        assert_eq!(refused(&app), not_opened("Gone", &why));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// With SDI on the program shows one drawing: a symbol is not opened,
    /// when it is asked for and when a read of it comes back, and the
    /// command line says why.
    #[test]
    fn with_sdi_on_a_symbol_is_not_opened() {
        let mut app = with_a_drawing();
        let (tabs, front) = (app.tabs.len(), app.active_tab);
        app.sdi = true;
        let why = crate::t!("SDI allows one drawing, and a symbol takes a tab of its own");
        let gate = file("Gate Valve.sym");

        // The ask is refused at once, and starts no read: nothing more is
        // said when what it started has run.
        let answer = asked(&mut app, &gate, "Gate Valve");
        assert_eq!(refused(&app), not_opened("Gate Valve", why.as_ref()));
        let said = app.command_line.history.len();
        run(&mut app, answer);
        assert_eq!((app.tabs.len(), app.active_tab), (tabs, front));
        assert_eq!(app.command_line.history.len(), said, "nothing was read");
        // A read that comes back with SDI on -- one started before SDI was
        // set -- is refused as well.
        read(&mut app, &gate, "Gate Valve", Ok(gate_valve().0));
        assert_eq!((app.tabs.len(), app.active_tab), (tabs, front));
        assert_eq!(app.command_line.history.len(), said + 1);
        assert_eq!(refused(&app), not_opened("Gate Valve", why.as_ref()));
    }

    // ── Every symbol file of a real backup ────────────────────────────────────

    /// The files of the `Ref` tree under `node`, `dirs` being the way to it.
    fn ref_files(node: &RefNode, dirs: &mut Vec<usize>, found: &mut Vec<NodeId>) {
        for at in 0..node.files.len() {
            found.push(NodeId::RefFile(dirs.clone(), at));
        }
        for (at, dir) in node.dirs.iter().enumerate() {
            dirs.push(at);
            ref_files(dir, dirs, found);
            dirs.pop();
        }
    }

    /// The SHA-256 of the file at `path`.
    fn digest(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let mut from = std::fs::File::open(path).expect("what was read from is still there");
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 16];
        loop {
            let count = std::io::Read::read(&mut from, &mut buffer).expect("it reads");
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        format!("{:x}", hasher.finalize())
    }

    /// A symbol file of a backup, as the panel lists it.
    struct Listed {
        /// Where it is listed: the backup folder, an archive, the `Ref`
        /// tree.
        place: String,
        /// What it is listed as there.
        listed: String,
        node: NodeId,
        source: SymbolSource,
        /// What the symbol is called.
        name: String,
    }

    /// What asking for the symbol files of one place came to.
    #[derive(Default)]
    struct Asked {
        files: usize,
        /// The details offer the button: the file has a thumbnail.
        offered: usize,
        opened: usize,
        /// Opened, with a word of what was not read of it.
        in_part: usize,
        /// The entities the drawings that were opened hold between them.
        entities: usize,
        widest: f64,
        tallest: f64,
        slowest: Option<(std::time::Duration, String)>,
        not_opened: Vec<String>,
    }

    /// Every symbol file of a real backup asked for as a drawing, from the
    /// panel: its row is selected, which reads it for its thumbnail, and
    /// then double-clicked. What has a thumbnail opens, in a tab called
    /// what the symbol is called, holding the entities a read of the file
    /// gives and drawing what the thumbnail draws, the whole of it in view;
    /// it is not modified by that, and closes without a question. What has
    /// none opens no tab, and the command line says why in the details'
    /// words. The first symbol of each place is asked for by its button
    /// too, pressed as a script presses it: the tab it has comes to the
    /// front, and once that is closed the button opens it anew. Nothing is
    /// written to what was read from: the files and the archives have the
    /// digests they had. The totals are printed.
    ///
    /// The backup is the one `every_symbol_of_a_real_backup_is_previewed`
    /// reads: the folder or the zip `OCS_PID_SYM_PREVIEW_BACKUP` names, or
    /// the plant package beside the repository, of whose archive of symbols
    /// 643 files open and the one with nothing in it does not.
    ///
    /// ```text
    /// cargo test --lib every_symbol_of_a_real_backup_is_opened -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "reads a real backup: ../pid-test-data, or the one OCS_PID_SYM_PREVIEW_BACKUP names"]
    fn every_symbol_of_a_real_backup_is_opened_from_the_panel() {
        let named = std::env::var_os("OCS_PID_SYM_PREVIEW_BACKUP").map(PathBuf::from);
        let plant = named.is_none();
        let Some(backup) = named.or_else(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()?
                .join("pid-test-data");
            root.join("Export.dmp").is_file().then_some(root)
        }) else {
            println!("skipped: there is no backup to read");
            return;
        };
        // Whatever has to be unpacked goes where the test clears it away,
        // and not where the program keeps what it restored.
        let cache =
            std::env::temp_dir().join(format!("ocs-pid-symbol-tab-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        std::fs::create_dir_all(&cache).expect("a scratch folder");
        let index =
            Arc::new(build_index_in(&backup, None, Some(&cache)).expect("the backup indexes"));

        // The symbol files, in the three places the panel lists them: the
        // backup folder, the archives, and the `Ref` tree.
        let mut nodes: Vec<(String, String, NodeId)> = Vec::new();
        for (at, entry) in index.files.iter().enumerate() {
            nodes.push((
                "backup folder".to_string(),
                entry.name.clone(),
                NodeId::File(at),
            ));
            for (member, name) in entry.members.iter().enumerate() {
                nodes.push((
                    entry.name.clone(),
                    name.clone(),
                    NodeId::FileMember(at, member),
                ));
            }
        }
        let mut in_ref = Vec::new();
        ref_files(&index.ref_tree, &mut Vec::new(), &mut in_ref);
        for node in in_ref {
            let NodeId::RefFile(dirs, at) = &node else {
                unreachable!()
            };
            let name = index
                .ref_file_path(dirs, *at)
                .and_then(|path| {
                    path.strip_prefix(&index.ref_root)
                        .ok()
                        .map(|path| path.display().to_string())
                })
                .unwrap_or_default();
            nodes.push(("Ref tree".to_string(), name, node));
        }
        let symbols: Vec<Listed> = nodes
            .into_iter()
            .filter_map(|(place, listed, node)| {
                let (source, name) = symbol_opens(&index, &node)?;
                Some(Listed {
                    place,
                    listed,
                    node,
                    source,
                    name,
                })
            })
            .collect();

        // What the symbols are read from: the files that lie on disk, and
        // the archives that hold the others.
        let mut read_from: BTreeMap<PathBuf, String> = BTreeMap::new();
        for symbol in &symbols {
            let (path, called) = match &symbol.source {
                SymbolSource::File(path) => {
                    (path.clone(), format!("{}: {}", symbol.place, symbol.listed))
                }
                SymbolSource::Member { archive, .. } => (archive.clone(), symbol.place.clone()),
            };
            read_from.entry(path).or_insert(called);
        }
        let before: Vec<String> = read_from.keys().map(|path| digest(path)).collect();

        // The program with a drawing in front and the backup in its panel,
        // which docks beside the drawing.
        let mut app = with_a_drawing();
        let drawing = app.active_tab;
        let loaded = PidBackupMsg::Loaded(Ok(Arc::clone(&index)));
        let task = app.update(Message::PidBackup(loaded));
        run(&mut app, task);
        assert!(
            app.pid_backup_browser.index().is_some(),
            "the backup is in the panel"
        );
        let tabs = app.tabs.len();
        let open = crate::t!("Open symbol").into_owned();
        let press = format!("PIDBACKUPOPEN Press {open}");

        let started = std::time::Instant::now();
        let mut places: BTreeMap<String, Asked> = BTreeMap::new();
        let mut wrong: Vec<String> = Vec::new();
        let mut canvas = None;
        for symbol in &symbols {
            let Listed {
                place,
                listed,
                node,
                source,
                name,
            } = symbol;
            let which = format!("{place}: {listed}");
            if app.active_tab != drawing {
                let _ = app.update(Message::TabSwitch(drawing));
            }
            assert_eq!(
                (app.tabs.len(), app.active_tab),
                (tabs, drawing),
                "{which}: the drawing is in front"
            );

            // The row is selected, which reads the file for its details:
            // with a thumbnail comes the button.
            let task = app.update(Message::PidBackup(PidBackupMsg::Select(node.clone())));
            run(&mut app, task);
            let offered = detail_buttons(&app.pid_backup_browser, node)
                .iter()
                .any(|(label, to)| *label == open && to == node);
            // What a read of the file gives, to hold the tab against.
            let on_disk = source.read_drawing();

            // The row is double-clicked. The command line keeps its newest
            // lines only, so it is emptied first: what it holds afterwards
            // is what it said of this symbol.
            app.command_line.clear_history();
            let asked = std::time::Instant::now();
            let task = app.update(Message::PidBackup(PidBackupMsg::Activate(node.clone())));
            run(&mut app, task);
            let took = asked.elapsed();
            let has_a_tab = app.tabs.len() == tabs + 1;

            let tally = places.entry(place.clone()).or_default();
            tally.files += 1;
            tally.offered += usize::from(offered);
            if tally.slowest.as_ref().is_none_or(|(most, _)| took > *most) {
                tally.slowest = Some((took, listed.clone()));
            }
            if has_a_tab != offered {
                wrong.push(format!(
                    "{which}: the details {} the button, and the double click opened {}",
                    if offered { "offer" } else { "do not offer" },
                    if has_a_tab { "a tab" } else { "none" }
                ));
            }
            if !has_a_tab {
                assert_eq!(app.tabs.len(), tabs, "{which}: no tab came or went");
                // The command line says why, in the details' words.
                let why = match &on_disk {
                    Ok(got) => pid_backup_browser::symbol_notes(&got.file).join(" "),
                    Err(error) => error.clone(),
                };
                let said_why = not_opened(name, &why);
                let last = app.command_line.history.last();
                let marked = last.is_some_and(|line| {
                    line.kind == EntryKind::Error && line.text.ends_with(&said_why)
                });
                if !marked || app.command_line.last_error.as_deref() != Some(said_why.as_str()) {
                    let last = last.map(|line| (line.kind.clone(), line.text.clone()));
                    wrong.push(match &on_disk {
                        Ok(_) => format!("{which}: not opened; the command line says {last:?}"),
                        Err(_) => format!("{which}: not opened, and not for what a read says"),
                    });
                }
                tally.not_opened.push(match &on_disk {
                    Ok(_) => format!("{listed}: {why}"),
                    Err(_) => format!("{listed}: it does not read"),
                });
                continue;
            }

            // The tab is the new one, in front: called what the symbol is
            // called, with no file of its own.
            let i = tabs;
            if app.active_tab != i {
                wrong.push(format!("{which}: the tab it opened is not in front"));
                let _ = app.update(Message::TabSwitch(i));
            }
            let (count, drawn) = {
                let tab = &app.tabs[i];
                let called = tab.tab_display_name();
                if called != *name {
                    wrong.push(format!("{which}: its tab is called {called:?}"));
                }
                if tab.current_path.is_some() || tab.shown_symbol().is_none() {
                    wrong.push(format!("{which}: its tab is not a symbol's"));
                }
                if canvas.is_none() {
                    canvas = Some(tab.scene.selection.borrow().view.vp_size);
                }
                let held = &tab.scene.document;
                (
                    held.model_space_entities().count(),
                    Sketch::of(held.model_space_entities()),
                )
            };
            tally.opened += 1;
            tally.entities += count;
            if let Some((min_x, min_y, max_x, max_y)) = drawn.bounds() {
                tally.widest = tally.widest.max(max_x - min_x);
                tally.tallest = tally.tallest.max(max_y - min_y);
            }

            // It holds what a read of the file gives, and draws what the
            // thumbnail draws, over the rectangle the details measure.
            let notes = match &on_disk {
                Ok(got) => {
                    if count != got.entities.len() {
                        wrong.push(format!(
                            "{which}: its tab holds {count} entities, and {} were read",
                            got.entities.len()
                        ));
                    }
                    let thumbnail = got.file.body.as_ref().map(|body| &*body.sketch);
                    if thumbnail != Some(&drawn) {
                        wrong.push(format!("{which}: its tab draws other than its thumbnail"));
                    }
                    if thumbnail.and_then(Sketch::bounds) != drawn.bounds() {
                        wrong.push(format!("{which}: its tab covers other than its thumbnail"));
                    }
                    pid_backup_browser::symbol_notes(&got.file).len()
                }
                Err(_) => {
                    wrong.push(format!("{which}: opened, and a read of it fails"));
                    0
                }
            };

            // The command line says what was opened, and after that what
            // the details say of how the file reads, when they say it.
            let lines = &app.command_line.history;
            let first = lines
                .first()
                .map(|line| (line.kind.clone(), line.text.clone()));
            if first != Some((EntryKind::Output, opened(name, count))) {
                wrong.push(format!(
                    "{which}: opened, and the command line says {first:?}"
                ));
            }
            let reminders = lines
                .iter()
                .filter(|line| line.kind == EntryKind::Warning)
                .count();
            if reminders != notes {
                wrong.push(format!(
                    "{which}: the details say {notes} thing(s) of how it reads, the command line {reminders}"
                ));
            }
            tally.in_part += usize::from(reminders > 0);

            // The whole of it is in view; and with the clock gone on,
            // nothing is modified by having been opened.
            if !takes_in(&app, &drawn) {
                wrong.push(format!("{which}: not the whole of it is in view"));
            }
            tick(&mut app);
            tick(&mut app);
            if app.tabs[i].dirty {
                wrong.push(format!("{which}: marked modified by being opened"));
                app.tabs[i].dirty = false;
            }

            // The first of its place is asked for by its button as well,
            // pressed the way a script presses it. From behind the drawing
            // its tab comes to the front; closed, it is opened anew.
            if tally.opened == 1 {
                let _ = app.update(Message::TabSwitch(drawing));
                let task = app.dispatch_command(&press);
                run(&mut app, task);
                if (app.tabs.len(), app.active_tab) != (tabs + 1, i) {
                    wrong.push(format!(
                        "{which}: its button did not bring its tab to the front"
                    ));
                    let _ = app.update(Message::TabSwitch(i));
                }
                assert!(
                    !asks_to_save_on_closing(&mut app),
                    "{which}: closing it asks about saving"
                );
                let task = app.dispatch_command(&press);
                run(&mut app, task);
                if app.tabs.len() != tabs + 1
                    || app.active_tab != i
                    || app.tabs[i].tab_display_name() != *name
                {
                    wrong.push(format!("{which}: closed, its button did not open it anew"));
                }
                tick(&mut app);
                tick(&mut app);
            }

            // Closed without a question: nobody touched it.
            if app.tabs.len() == tabs + 1 {
                if app.active_tab != i {
                    let _ = app.update(Message::TabSwitch(i));
                }
                assert!(
                    !asks_to_save_on_closing(&mut app),
                    "{which}: closing it asks about saving"
                );
            }
        }
        let took = started.elapsed();

        // Nothing was written to what was read from.
        let after: Vec<String> = read_from.keys().map(|path| digest(path)).collect();
        for ((called, before), after) in read_from.values().zip(&before).zip(&after) {
            if before != after {
                wrong.push(format!("{called}: its bytes are not what they were"));
            }
        }

        let mut files = 0;
        for (place, tally) in &places {
            files += tally.files;
            println!(
                "{place}: {} symbol file(s): {} opened in a tab, {} of them read in part; {} not opened; the details offer the button of {}",
                tally.files,
                tally.opened,
                tally.in_part,
                tally.not_opened.len(),
                tally.offered
            );
            println!(
                "  the drawings opened hold {} entities; the widest {:.2} mm, the tallest {:.2} mm",
                tally.entities, tally.widest, tally.tallest
            );
            if let Some((most, name)) = &tally.slowest {
                println!(
                    "  the slowest to answer a double click: {:.1} ms ({name})",
                    most.as_secs_f64() * 1000.0
                );
            }
            for line in &tally.not_opened {
                println!("  not opened: {line}");
            }
        }
        if let Some((width, height)) = canvas {
            println!("the canvas the symbols were fitted to: {width} by {height}");
        }
        println!(
            "{files} symbol file(s) asked for from the panel in {:.1} s; the {} file(s) and archive(s) they were read from digested before and after",
            took.as_secs_f64(),
            read_from.len()
        );
        for line in &wrong {
            println!("wrong: {line}");
        }
        drop(app);
        let _ = std::fs::remove_dir_all(&cache);

        assert!(files > 0, "the backup holds symbol files");
        assert!(
            wrong.is_empty(),
            "{} thing(s) are not as they should be: they are printed above",
            wrong.len()
        );
        if plant {
            let archive = places
                .get("RefData~4~681.zip")
                .expect("the plant package's archive of symbols");
            assert_eq!(
                (archive.files, archive.opened, archive.not_opened.len()),
                (644, 643, 1)
            );
        }
    }
}

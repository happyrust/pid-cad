//! PIDLEGEND — mark up the P&ID symbols a sheet places.
//!
//! Recognition and the legend geometry live in `io::pid_legend`; this is the
//! command-line front: run it, print the per-class summary, put the coloured
//! rectangles and labels into the drawing on `PID-LEGEND-<CLASS>` layers (and
//! the traced pipe runs on `PID-PIPE-<line number>` ones), and take them out
//! again.
//!
//! ```text
//! PIDLEGEND            prompt with the verbs below (Enter = ON)
//! PIDLEGEND ON         recognise, then draw (replacing an earlier legend)
//! PIDLEGEND OFF        remove the legend entities (layers stay, empty)
//! PIDLEGEND REPORT     recognise and print the summary, draw nothing
//! PIDLEGEND LIST       toggle the docked legend list panel (recognising
//!                      first when this sheet has not been read yet)
//! PIDLINE              prompt for a line number or family (Enter = resolve
//!                      the picked stroke instead)
//! PIDLINE <code>       select the whole line by number or family
//!                      (PIDLINE 100-FW, PIDLINE FS-31001)
//! PIDTAG               prompt for a tag (Enter = the picked symbol's)
//! PIDTAG <tag>         select the symbol(s) carrying the tag with their tag
//!                      lettering, frame them in red, zoom to them
//! ```
//!
//! ON and REPORT stash the recognition on the tab; the legend list panel
//! (`ui::window::pid_legend_list`) renders it: its symbol rows select and
//! frame the symbol as PIDTAG does, its pipe rows select the line. ON also
//! opens the panel — the list is the index of what was just drawn. The
//! index is of the drawing it was read from: once the drawing moves on (an
//! edit, an erase, an undo — any geometry epoch bump other than the legend's
//! own entities going in or out) the panel says it is out of date, and
//! PIDLINE, PIDTAG, a row of the panel and LIST read the sheet again before
//! they use it (`refresh_pid_legend`).
//!
//! PIDTAG answers "what does this tag stand for on the sheet?": the group
//! the SVG export writes as `<g tagName="…">` — the symbol's own entities
//! and the lettering its tag was read from — is selected, drawn over in red
//! with a red frame round it (`Scene::set_pid_group_highlight`, the
//! debugging view of the group's extent), and zoomed to. The highlight goes
//! with the selection: pick anything else, or Esc, and it is gone.
//!
//! PIDLINE answers "what else is this pipe?": from a picked pipe stroke (or
//! a typed code) it resolves the line's *family* by the sheets' coding rule
//! — the service and sequence number name one physical line however its
//! size or class changes (`200-FS-31001-A2` and `150-FS-31001-A2` are one
//! family `FS-31001`) — then selects every stroke of every run the family
//! covers and zooms to their whole extent. Clicking a pipe row in the
//! legend list panel does the same.
//!
//! Bare `PIDLEGEND` only prompts, like bare `PURGE`: the command driver
//! dispatches the first word of any inline line on its own before the whole
//! line, so a bare verb that drew would draw once more on every `PIDLEGEND
//! OFF`. Rules come from `assets/pid-legend.json`, or the file
//! `OCS_PID_LEGEND_RULES` names. A script of one line, `PIDLEGEND ON`, run
//! with `--script`, marks up a new sheet as it opens.

use super::*;
use crate::io::pid_legend::{self, Rules};
use acadrust::types::Color;
use std::collections::BTreeSet;

impl OpenCADStudio {
    pub(super) fn dispatch_pidlegend(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if words.next() != Some("PIDLEGEND") {
            return None;
        }
        let Some(option) = words.next().map(str::to_uppercase) else {
            use crate::command::KeywordCommand;
            let c = KeywordCommand::new(
                "PIDLEGEND",
                "PIDLEGEND  [ON / OFF / REPORT / LIST] <ON>:",
                vec![
                    ("ON", "ON", None),
                    ("OFF", "OFF", None),
                    ("REPORT", "REPORT", None),
                    ("LIST", "LIST", None),
                ],
            )
            .with_default("ON");
            self.command_line.push_info(&c.prompt());
            self.tabs[i].active_cmd = Some(Box::new(c));
            return Some(self.finish_dispatch(cmd));
        };
        match option.as_str() {
            "OFF" => {
                let handles = pid_legend::legend_handles(&self.tabs[i].scene.document);
                if handles.is_empty() {
                    self.command_line
                        .push_info("PIDLEGEND: no legend entities to remove.");
                } else {
                    self.push_undo_snapshot(i, "PIDLEGEND");
                    let was = self.tabs[i].scene.geometry_epoch;
                    self.tabs[i].scene.erase_entities(&handles);
                    self.tabs[i].dirty = true;
                    // Taking the legend out changes nothing recognition
                    // reads (it skips the legend layers): the index stays.
                    self.tabs[i].pid_legend_survives(was);
                    self.command_line.push_output(&format!(
                        "PIDLEGEND: removed {} legend entities.",
                        handles.len()
                    ));
                }
            }
            "REPORT" => {
                let rules = Rules::load();
                let recognition = pid_legend::recognise(&self.tabs[i].scene.document, &rules);
                for line in pid_legend::report(&recognition) {
                    self.command_line.push_output(&line);
                }
                self.tabs[i].set_pid_legend(recognition);
            }
            "LIST" => {
                if self.show_pid_legend_list {
                    self.show_pid_legend_list = false;
                } else {
                    if self.refresh_pid_legend(i) {
                        self.command_line.push_output(&format!(
                            "PIDLEGEND: {} symbols listed. PIDLEGEND ON draws them.",
                            self.tabs[i].pid_legend().map_or(0, |r| r.symbols.len())
                        ));
                    }
                    self.open_pid_legend_panel();
                }
            }
            "ON" => {
                let rules = Rules::load();
                let recognition = pid_legend::recognise(&self.tabs[i].scene.document, &rules);
                if recognition.symbols.is_empty() {
                    self.command_line.push_info(
                        "PIDLEGEND: nothing recognised -- no known block or circle symbol on this sheet. PIDLEGEND REPORT lists what was seen.",
                    );
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "PIDLEGEND");
                // Running it twice must not stack two legends.
                let stale = pid_legend::legend_handles(&self.tabs[i].scene.document);
                if !stale.is_empty() {
                    self.tabs[i].scene.erase_entities(&stale);
                }
                let layers = pid_legend::legend_layers(&recognition);
                let names: Vec<String> = layers.keys().cloned().collect();
                for (name, [r, g, b]) in &layers {
                    self.tabs[i].scene.ensure_layer(name);
                    if let Some(layer) = self.tabs[i].scene.document.layers.get_mut(name) {
                        layer.color = Color::Rgb {
                            r: *r,
                            g: *g,
                            b: *b,
                        };
                    }
                }
                self.tabs[i].scene.invalidate_layer_dependencies(&names);
                let entities = pid_legend::legend_entities(&recognition, &rules);
                let drawn = entities.len();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.tabs[i].scene.add_entities(entities);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    for entity in entities {
                        self.tabs[i].scene.add_entity(entity);
                    }
                }
                self.tabs[i].dirty = true;
                for line in pid_legend::report(&recognition) {
                    self.command_line.push_output(&line);
                }
                let pipe_layers = layers
                    .keys()
                    .filter(|name| name.starts_with(pid_legend::PIPE_LAYER_PREFIX))
                    .count();
                self.command_line.push_output(&format!(
                    "PIDLEGEND: drew {drawn} entities on {} layers ({}*{}). PIDLEGEND OFF removes them.",
                    layers.len(),
                    pid_legend::LAYER_PREFIX,
                    if pipe_layers > 0 {
                        format!(", {pipe_layers} of them {}*", pid_legend::PIPE_LAYER_PREFIX)
                    } else {
                        String::new()
                    }
                ));
                // Stamped after the legend went in: drawing it is not a
                // change to the sheet it indexes.
                self.tabs[i].set_pid_legend(recognition);
                self.open_pid_legend_panel();
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDLEGEND: unknown option \"{other}\" -- use ON, OFF, REPORT or LIST."
                ));
            }
        }
        Some(Task::none())
    }

    /// Show the legend list panel, docking it on the right the first time.
    fn open_pid_legend_panel(&mut self) {
        use crate::ui::dock::PanelId;
        if self.dock.location(PanelId::PidLegend).is_none() {
            self.dock
                .dock(PanelId::PidLegend, crate::app::config::DockSide::Right, 0);
        }
        self.show_pid_legend_list = true;
    }

    /// Read the sheet when it has not been read yet, or has changed since it
    /// was (`DocumentTab::pid_legend_is_stale`): the index's row boxes and
    /// the runs' stroke handles are only good for the drawing they came
    /// from. Cheap enough to do in place -- under 0.2 s on the CPECC sheets
    /// -- and says so on the command line when it is a re-read. True when it
    /// read.
    pub(in crate::app) fn refresh_pid_legend(&mut self, i: usize) -> bool {
        let stale = self.tabs[i].pid_legend_is_stale();
        if self.tabs[i].pid_legend.is_some() && !stale {
            return false;
        }
        let rules = Rules::load();
        let recognition = pid_legend::recognise(&self.tabs[i].scene.document, &rules);
        if stale {
            self.command_line.push_info(&format!(
                "PIDLEGEND: the sheet changed since it was read -- read again, {} symbols.",
                recognition.symbols.len()
            ));
        }
        self.tabs[i].set_pid_legend(recognition);
        true
    }

    /// Hand the active sheet's tagged symbols to a plot job as groups, for a
    /// backend that can name a group of elements -- the SVG export wraps each
    /// in `<g tagName="…">`. The sheet is read first when it has not been, or
    /// has changed since (`pid_legend_is_stale`), and what was read is kept
    /// as the tab's index, as LIST would keep it; quietly, since a plot is
    /// not the place for the panel's "read again" line. A sheet with no
    /// tagged symbol leaves the job as it was.
    pub(in crate::app) fn attach_pid_groups(&mut self, job: &mut crate::app::update::file::PlotJob) {
        let i = self.active_tab;
        if self.tabs[i].pid_legend.is_none() || self.tabs[i].pid_legend_is_stale() {
            let rules = Rules::load();
            let recognition = pid_legend::recognise(&self.tabs[i].scene.document, &rules);
            self.tabs[i].set_pid_legend(recognition);
        }
        if let Some(recognition) = self.tabs[i].pid_legend() {
            job.assets.groups = pid_legend::plot_groups(recognition);
        }
    }

    pub(super) fn dispatch_pidline(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        if words.next() != Some("PIDLINE") {
            return None;
        }
        let code = words.next().map(str::to_uppercase);
        if code.is_none() && cmd.trim_end() == cmd {
            // Bare verb: prompt for the code, like bare PDMODE. Acting here
            // instead would act twice on every inline `PIDLINE <code>` (the
            // driver dispatches the first word on its own first). Enter at
            // the prompt dispatches `PIDLINE ` -- the picked-stroke path.
            use crate::command::ValuePromptCommand;
            let c = ValuePromptCommand::new(
                "PIDLINE",
                "PIDLINE  line number or family <picked stroke>:",
            );
            self.command_line.push_info(&c.prompt());
            self.tabs[i].active_cmd = Some(Box::new(c));
            return Some(self.finish_dispatch(cmd));
        }
        // The index must exist, and be of this drawing, before a line can be
        // looked up in it: a fresh sheet is recognised here, like PIDLEGEND
        // LIST does, and a sheet that changed since it was read is read
        // again -- the runs' stroke handles are what gets selected.
        self.refresh_pid_legend(i);
        // What to select: the families the argument or the picked strokes
        // name, plus any picked run no line number reaches.
        let mut families: BTreeSet<String> = BTreeSet::new();
        let mut picked_runs: Vec<usize> = Vec::new();
        {
            let rec = self.tabs[i].pid_legend().expect("recognised above");
            match &code {
                Some(code) => {
                    let by_family = rec.pipes.by_family();
                    if by_family.contains_key(code) {
                        families.insert(code.clone());
                    } else {
                        // A number this sheet does not letter (another size
                        // of a line it does) is not in the index; the rules
                        // say what family it belongs to.
                        let family = Rules::load().pipes.line_family(code);
                        if by_family.contains_key(&family) {
                            families.insert(family);
                        } else {
                            self.command_line.push_error(&format!(
                                "PIDLINE: no line \"{code}\" on this sheet -- PIDLEGEND LIST shows what it carries."
                            ));
                            return Some(Task::none());
                        }
                    }
                }
                None => {
                    for handle in self.tabs[i].scene.selected_handles_in_order() {
                        // A picked legend overlay names its line on its layer.
                        if let Some(entity) = self.tabs[i].scene.document.get_entity(handle) {
                            if let Some(line) = entity
                                .common()
                                .layer
                                .strip_prefix(pid_legend::PIPE_LAYER_PREFIX)
                            {
                                if line != "NONE" {
                                    families.insert(rec.pipes.family_of(line).to_string());
                                }
                                continue;
                            }
                        }
                        // A picked sheet stroke names the runs made of it.
                        for (ri, run) in rec.pipes.runs.iter().enumerate() {
                            if run
                                .handles
                                .binary_search_by_key(&handle.value(), |h| h.value())
                                .is_err()
                            {
                                continue;
                            }
                            if run.lines.is_empty() {
                                picked_runs.push(ri);
                            } else {
                                for line in &run.lines {
                                    families.insert(rec.pipes.family_of(line).to_string());
                                }
                            }
                        }
                    }
                    if families.is_empty() && picked_runs.is_empty() {
                        self.command_line.push_info(
                            "PIDLINE: pick a stroke of the pipe first, or name it -- PIDLINE 100-FW.",
                        );
                        return Some(Task::none());
                    }
                }
            }
        }
        match self.pid_line_select(i, &families, &picked_runs) {
            Some(receipt) => self.command_line.push_output(&receipt),
            None => self.command_line.push_info("PIDLINE: nothing to select."),
        }
        Some(Task::none())
    }

    /// Select every stroke of the runs `families` cover -- plus
    /// `picked_runs`, for picked strokes no line number reaches -- zoom to
    /// their whole extent, and word the command-line receipt. `None` when
    /// nothing matches. Also serves the legend list panel's pipe rows, via
    /// `Message::PidLegendPickFamily`; a sheet that changed since the panel
    /// was filled is read again first, so the handles selected are of the
    /// drawing as it is (a family the change took away then matches nothing).
    pub(in crate::app) fn pid_line_select(
        &mut self,
        i: usize,
        families: &BTreeSet<String>,
        picked_runs: &[usize],
    ) -> Option<String> {
        self.refresh_pid_legend(i);
        let (handles, bbox, upm, receipt) = {
            let rec = self.tabs[i].pid_legend()?;
            let runs = &rec.pipes.runs;
            let mut picked: BTreeSet<usize> = picked_runs
                .iter()
                .copied()
                .filter(|&r| r < runs.len())
                .collect();
            let mut lines: BTreeSet<&str> = BTreeSet::new();
            for (ri, run) in runs.iter().enumerate() {
                for line in &run.lines {
                    if families.contains(rec.pipes.family_of(line)) {
                        picked.insert(ri);
                        lines.insert(line);
                    }
                }
            }
            if picked.is_empty() {
                return None;
            }
            let mut handles: rustc_hash::FxHashSet<acadrust::Handle> =
                rustc_hash::FxHashSet::default();
            let mut bbox: Option<(f64, f64, f64, f64)> = None;
            let mut length_mm = 0.0;
            for &ri in &picked {
                let run = &runs[ri];
                handles.extend(run.handles.iter().copied());
                length_mm += run.length_mm;
                if let Some(r) = run.bbox() {
                    bbox = Some(match bbox {
                        None => r,
                        Some(a) => (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3)),
                    });
                }
            }
            let what = if families.is_empty() {
                "unnumbered pipe".to_string()
            } else {
                families.iter().cloned().collect::<Vec<_>>().join(" + ")
            };
            let spelt = if lines.len() > families.len() {
                format!(" ({})", lines.iter().copied().collect::<Vec<_>>().join(", "))
            } else {
                String::new()
            };
            let receipt = format!(
                "PIDLINE: {what}{spelt} -- {} runs, {:.0} mm, {} entities selected.",
                picked.len(),
                length_mm,
                handles.len(),
            );
            (handles, bbox, rec.units_per_mm, receipt)
        };
        self.tabs[i].scene.replace_selection(handles);
        self.refresh_properties();
        if let Some(bbox) = bbox {
            let (min, max) = crate::ui::window::pid_legend_list::jump_rect(bbox, upm);
            self.tabs[i].scene.remember_current_view();
            self.tabs[i].scene.zoom_to_window(
                glam::Vec3::new(min.0 as f32, min.1 as f32, 0.0),
                glam::Vec3::new(max.0 as f32, max.1 as f32, 0.0),
            );
        }
        Some(receipt)
    }

    /// PIDTAG: jump to a symbol group by its tag.
    ///
    /// ```text
    /// PIDTAG              prompt for a tag (Enter = the picked symbol's)
    /// PIDTAG <tag>        select the symbol(s) tagged so and their tag
    ///                     lettering, frame them in red, zoom to them
    /// ```
    ///
    /// The group is what the SVG export wraps in one `<g tagName>`: the
    /// symbol's own entities plus the lettering its tag was read from. Two
    /// symbols may carry one tag (a motorised valve and its `XV` bubble) and
    /// are then both selected, each in its own frame. With nothing named, the
    /// picked entities say which symbols: a stroke of the valve, or its tag.
    pub(super) fn dispatch_pidtag(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let rest = cmd.strip_prefix("PIDTAG")?;
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return None;
        }
        let arg = rest.trim();
        if arg.is_empty() && cmd.trim_end() == cmd {
            // Bare verb: prompt for the tag, like bare PIDLINE. Enter at the
            // prompt dispatches `PIDTAG ` -- the picked-symbol path.
            use crate::command::ValuePromptCommand;
            let c = ValuePromptCommand::new("PIDTAG", "PIDTAG  tag <picked symbol>:");
            self.command_line.push_info(&c.prompt());
            self.tabs[i].active_cmd = Some(Box::new(c));
            return Some(self.finish_dispatch(cmd));
        }
        let receipt = if arg.is_empty() {
            let picked: Vec<u64> = self.tabs[i]
                .scene
                .selected_handles_in_order()
                .iter()
                .map(|h| h.value())
                .collect();
            if picked.is_empty() {
                self.command_line.push_info(
                    "PIDTAG: pick a symbol (or its tag lettering) first, or name the tag -- PIDTAG BUV-3201.",
                );
                return Some(Task::none());
            }
            self.pid_group_select(i, |s| symbol_has_any(s, &picked))
        } else {
            let wanted = arg.to_uppercase();
            let found = self.pid_group_select(i, |s| {
                s.tag.as_deref().is_some_and(|t| t.to_uppercase() == wanted)
            });
            if found.is_none() {
                self.command_line.push_error(&format!(
                    "PIDTAG: no symbol tagged \"{arg}\" on this sheet -- PIDLEGEND LIST shows the tags."
                ));
                return Some(Task::none());
            }
            found
        };
        match receipt {
            Some(receipt) => self.command_line.push_output(&receipt),
            None => self
                .command_line
                .push_info("PIDTAG: the picked entities belong to no recognised symbol."),
        }
        Some(Task::none())
    }

    /// Select the entities of every recognised symbol `pick` admits -- each
    /// symbol's own entities and the lettering of its tag, what the SVG export
    /// wraps in one `<g tagName>` -- draw them in red with a frame round each
    /// (`Scene::set_pid_group_highlight`), zoom to their whole extent, and
    /// word the receipt. `None` when nothing matches or nothing of it draws.
    /// Serves PIDTAG and the legend list's symbol rows; a sheet that changed
    /// since it was read is read again first, and `pick` sees the fresh
    /// recognition -- which is why the callers pick by tag or by handle, not
    /// by row.
    pub(in crate::app) fn pid_group_select(
        &mut self,
        i: usize,
        pick: impl Fn(&pid_legend::Recognized) -> bool,
    ) -> Option<String> {
        self.refresh_pid_legend(i);
        let (groups, names, upm) = {
            let rec = self.tabs[i].pid_legend()?;
            let mut groups: Vec<Vec<acadrust::Handle>> = Vec::new();
            let mut names: Vec<String> = Vec::new();
            for symbol in rec.symbols.iter().filter(|s| pick(s)) {
                let handles: Vec<acadrust::Handle> = symbol
                    .handles
                    .iter()
                    .chain(&symbol.tag_handles)
                    .copied()
                    .filter(|h| !h.is_null())
                    .collect();
                if handles.is_empty() {
                    continue;
                }
                groups.push(handles);
                names.push(match &symbol.tag {
                    Some(tag) => format!("{} {tag}", symbol.label),
                    None => format!("{} (无位号)", symbol.label),
                });
            }
            (groups, names, rec.units_per_mm)
        };
        if groups.is_empty() {
            return None;
        }
        let selected: rustc_hash::FxHashSet<acadrust::Handle> =
            groups.iter().flatten().copied().collect();
        let count = selected.len();
        self.tabs[i].scene.replace_selection(selected);
        self.refresh_properties();
        // A millimetre of paper round each group, so the frame stands off
        // the symbol's own strokes.
        let frame = self.tabs[i].scene.set_pid_group_highlight(&groups, upm);
        let what = names.join(" + ");
        let Some(bbox) = frame else {
            return Some(format!(
                "PIDTAG: {what} -- {count} entities selected, none of them draws anything to frame."
            ));
        };
        let (min, max) = crate::ui::window::pid_legend_list::jump_rect(bbox, upm);
        self.tabs[i].scene.remember_current_view();
        self.tabs[i].scene.zoom_to_window(
            glam::Vec3::new(min.0 as f32, min.1 as f32, 0.0),
            glam::Vec3::new(max.0 as f32, max.1 as f32, 0.0),
        );
        Some(format!(
            "PIDTAG: {what} -- {} group(s), {count} entities selected and framed in red.",
            groups.len()
        ))
    }
}

/// Whether any of `handles` (handle values) is one of `symbol`'s entities or
/// of its tag lettering.
fn symbol_has_any(symbol: &pid_legend::Recognized, handles: &[u64]) -> bool {
    symbol
        .handles
        .iter()
        .chain(&symbol.tag_handles)
        .any(|h| handles.contains(&h.value()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_legend;

    /// A real CPECC sheet beside the checkout, when it is there and not
    /// locked by an editor; otherwise skipped with a line on stderr -- unless
    /// `OCS_PID_SHEETS_REQUIRED=1`, which makes the skip a failure so a green
    /// run cannot hide it (same switch as `tests/pid_legend.rs`).
    fn open_sheet(app: &mut OpenCADStudio, name: &str) -> bool {
        let skip = |why: String| {
            if std::env::var_os("OCS_PID_SHEETS_REQUIRED").is_some_and(|v| v == "1") {
                panic!("{name}: {why} -- and OCS_PID_SHEETS_REQUIRED=1 does not skip");
            }
            eprintln!("skipping {name}: {why}");
            false
        };
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("0版重新处理dxf-12张")
            .join(name);
        if !path.is_file() {
            return skip("not present".to_string());
        }
        let request = serde_json::json!({ "op": "open", "path": path.to_string_lossy() });
        let reply = app.automation_op(&request.to_string());
        if reply["ok"].as_bool() != Some(true) {
            return skip(reply.to_string());
        }
        true
    }

    fn legend_count(app: &OpenCADStudio) -> usize {
        pid_legend::legend_handles(&app.tabs[app.active_tab].scene.document).len()
    }

    /// Both verbs complete on the command line: they dispatch here without a
    /// `CadCommand` module of their own, so `commands/mod.rs` lists them by
    /// hand -- PIDLINE was left out when it arrived.
    #[test]
    fn pidlegend_and_pidline_are_registered_for_autocomplete() {
        let names = crate::command::all_registered_command_names();
        for verb in ["PIDLEGEND", "PIDLINE", "PIDTAG"] {
            assert!(names.contains(&verb), "{verb} missing from the registry");
        }
    }

    /// A sheet of one symbol: a butterfly valve block (`$VALVE$00000316`, a
    /// 6 x 3 mm bow-tie of four lines) with `BUV-3101` lettered above it, a
    /// run of pipe through it and a frame round everything. Returns the
    /// handles of the block reference, the tag lettering and the pipe. The
    /// frame is also the sheet's extent -- lettering does not count towards
    /// it, and a plot fitted to the body alone would leave the tag off the
    /// page.
    fn valve_with_tag(
        app: &mut OpenCADStudio,
    ) -> (acadrust::Handle, acadrust::Handle, acadrust::Handle) {
        use acadrust::entities::{EntityType, Line, Text};
        use acadrust::types::{Transform, Vector3};

        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let line = |a: (f64, f64), b: (f64, f64)| {
            let mut line = Line::new();
            line.start = Vector3::new(a.0, a.1, 0.0);
            line.end = Vector3::new(b.0, b.1, 0.0);
            EntityType::Line(line)
        };
        let body: Vec<acadrust::Handle> = [
            ((0.0, 0.0), (0.0, 3.0)),
            ((0.0, 3.0), (6.0, 0.0)),
            ((6.0, 0.0), (6.0, 3.0)),
            ((6.0, 3.0), (0.0, 0.0)),
        ]
        .into_iter()
        .map(|(a, b)| app.tabs[i].scene.add_entity(line(a, b)))
        .collect();
        let identity = Transform::identity();
        let insert = app.tabs[i]
            .scene
            .create_block_from_entities(&body, "$VALVE$00000316", &identity, &identity)
            .unwrap();
        let tag = app.tabs[i].scene.add_entity(EntityType::Text(
            Text::with_value("BUV-3101", Vector3::new(0.0, 4.0, 0.0)).with_height(2.5),
        ));
        let pipe = app.tabs[i]
            .scene
            .add_entity(line((-20.0, 1.5), (30.0, 1.5)));
        for (a, b) in [
            ((-30.0, -10.0), (40.0, -10.0)),
            ((40.0, -10.0), (40.0, 20.0)),
            ((40.0, 20.0), (-30.0, 20.0)),
            ((-30.0, 20.0), (-30.0, -10.0)),
        ] {
            app.tabs[i].scene.add_entity(line(a, b));
        }
        (insert, tag, pipe)
    }

    fn selected(app: &OpenCADStudio) -> Vec<acadrust::Handle> {
        let mut handles = app.tabs[app.active_tab].scene.selected_handles_in_order();
        handles.sort_by_key(|h| h.value());
        handles
    }

    /// The command line's latest entry.
    fn last_line(app: &OpenCADStudio) -> String {
        app.command_line
            .history
            .last()
            .map(|entry| entry.text.clone())
            .unwrap_or_default()
    }

    /// PIDTAG <tag>: the symbol's block reference and its tag lettering are
    /// selected, drawn over in red with a red frame round them, and zoomed
    /// to; the pipe through the valve is none of it. The highlight goes with
    /// the selection, and with the geometry it was built from.
    #[test]
    fn pidtag_selects_frames_and_zooms_to_the_tagged_group() {
        use crate::scene::{PID_GROUP_FRAME, PID_GROUP_HIGHLIGHT};

        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        let mut expected = vec![insert, tag];
        expected.sort_by_key(|h| h.value());
        let camera_before = app.tabs[i].scene.camera_generation;

        let _ = app.run_command_line("PIDTAG buv-3101");
        assert_eq!(
            selected(&app),
            expected,
            "the block and its tag, case aside"
        );
        let receipt = last_line(&app);
        assert!(
            receipt.contains("蝶阀 BUV-3101") && receipt.contains("2 entities"),
            "{receipt}"
        );
        assert!(
            app.tabs[i].scene.camera_generation > camera_before,
            "the view moved to the group"
        );
        let highlight = app.tabs[i].scene.pid_group_highlight_wires().to_vec();
        assert!(
            highlight.len() >= 2,
            "the group's wires and a frame, got {}",
            highlight.len()
        );
        assert!(highlight.iter().all(|w| w.color == PID_GROUP_HIGHLIGHT));
        assert!(highlight
            .iter()
            .all(|w| w.text_verts.iter().all(|v| v.color == PID_GROUP_HIGHLIGHT)));
        assert!(
            highlight.iter().all(|w| w.render_instance.is_none()),
            "the block's strokes draw as plain preview wires, not by the instanced path"
        );
        let frames: Vec<&crate::scene::WireModel> = highlight
            .iter()
            .filter(|w| w.name == PID_GROUP_FRAME)
            .collect();
        assert_eq!(frames.len(), 1, "one frame for one group");
        // The frame is a closed rectangle a millimetre outside the valve body
        // (0..6 x 0..3) and its tag (lettered from y = 4, 2.5 mm high; the
        // glyph quads carry a little SDF padding past the ink, so the left
        // edge sits a hair further out than the body alone would put it).
        let frame = frames[0];
        assert_eq!(frame.points.len(), 5);
        let xs: Vec<f32> = frame.points.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = frame.points.iter().map(|p| p[1]).collect();
        let (x0, x1) = (
            xs.iter().cloned().fold(f32::MAX, f32::min),
            xs.iter().cloned().fold(f32::MIN, f32::max),
        );
        let (y0, y1) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(x0 > -2.0 && x0 <= -1.0 + 1e-3, "left {x0}");
        assert!((y0 - -1.0).abs() < 0.05, "bottom {y0}");
        assert!(
            x1 > 6.9 && x1 < 40.0,
            "right {x1} clears the body and stops at the tag"
        );
        assert!(y1 > 6.4 && y1 < 9.0, "top {y1} covers the tag's height");
        assert!(
            !highlight
                .iter()
                .any(|w| w.name.ends_with(&pipe.value().to_string())),
            "the pipe is not the symbol's"
        );

        // The selection overlay draws x-ray over the red copies, so it is the
        // overlay that has to go red while the group is the selection.
        assert_eq!(app.tabs[i].scene.selection_tint(), PID_GROUP_HIGHLIGHT);

        // Any other selection takes the highlight with it, and the overlay
        // is the user's selection colour again.
        app.tabs[i].scene.deselect_all();
        assert!(app.tabs[i].scene.pid_group_highlight_wires().is_empty());
        assert_ne!(app.tabs[i].scene.selection_tint(), PID_GROUP_HIGHLIGHT);

        // Built again, it does not survive the geometry moving on: the copies
        // would show the symbol where it was.
        let _ = app.run_command_line("PIDTAG BUV-3101");
        assert!(!app.tabs[i].scene.pid_group_highlight_wires().is_empty());
        app.tabs[i].scene.bump_geometry();
        assert!(app.tabs[i].scene.pid_group_highlight_wires().is_empty());
    }

    /// On a real sheet the frame closes round the block reference's expansion
    /// and the tag's glyphs, and nothing else: a block's wires carry their
    /// world coordinates as they stand, whatever `render_instance` says. The
    /// sheet letters its tags at z = 100 while the valve sits on the plane:
    /// the frame goes up to the lettering, and the zoom keeps the drawing's
    /// depth so the lettering stays in front of the near plane -- sized to
    /// the flat zoom window, as it was, the tag vanished the moment the view
    /// arrived at it.
    ///
    /// Ignored by default for the same reason as the whole-sheet plot below:
    /// sizing the depth to the drawing lays the whole sheet out once, and
    /// that grows the process-wide glyph atlas for seconds under the other
    /// plot tests, which then run out of relayouts and refuse their pages.
    #[test]
    #[ignore = "lays a real sheet out; run alone with --ignored, with the sheets beside the checkout"]
    fn on_a_real_sheet_the_frame_closes_round_the_valve_and_its_tag() {
        use crate::scene::PID_GROUP_FRAME;
        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf") {
            return;
        }
        let i = app.active_tab;
        let _ = app.run_command_line("PIDTAG BUV-3201");
        let highlight = app.tabs[i].scene.pid_group_highlight_wires();
        let frame = highlight
            .iter()
            .find(|w| w.name == PID_GROUP_FRAME)
            .expect("a frame");
        let xs: Vec<f32> = frame.points.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = frame.points.iter().map(|p| p[1]).collect();
        let width = xs.iter().cloned().fold(f32::MIN, f32::max)
            - xs.iter().cloned().fold(f32::MAX, f32::min);
        let height = ys.iter().cloned().fold(f32::MIN, f32::max)
            - ys.iter().cloned().fold(f32::MAX, f32::min);
        // The valve is 3 mm across and its tag 10.7 mm long, 100 drawing
        // units to the millimetre, a millimetre of frame each side.
        assert!(width > 1000.0 && width < 1600.0, "frame width {width}");
        assert!(height > 500.0 && height < 900.0, "frame height {height}");
        assert!(
            frame.points.iter().all(|p| (p[2] - 100.0).abs() < 1e-3),
            "the frame sits at the tag's z = 100, got {:?}",
            frame.points[0]
        );
        assert_eq!(selected(&app).len(), 2, "the block reference and the tag");
        let camera = app.tabs[i].scene.camera.borrow();
        let (min, max) = camera
            .fitted_model_bounds()
            .expect("the depth is sized to the drawing");
        assert!(
            min.z <= 0.0 && max.z >= 100.0,
            "the depth box {min:?}..{max:?} must hold the valve and its tag"
        );
    }

    /// PIDTAG with nothing named goes by the picked entities -- a stroke of
    /// the valve, or its tag -- and PIDTAG with a tag nobody carries says so
    /// and leaves the selection alone.
    #[test]
    fn pidtag_reads_the_picked_symbol_and_refuses_an_unknown_tag() {
        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        let mut expected = vec![insert, tag];
        expected.sort_by_key(|h| h.value());

        // Picked the tag lettering only: the whole group follows.
        app.tabs[i].scene.select_entity(tag, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(selected(&app), expected);

        // Picked the pipe: no symbol is made of it.
        app.tabs[i].scene.select_entity(pipe, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(selected(&app), vec![pipe], "the pick stays as it was");
        assert!(app.tabs[i].scene.pid_group_highlight_wires().is_empty());

        // An unknown tag.
        let _ = app.run_command_line("PIDTAG XV-9999");
        assert_eq!(selected(&app), vec![pipe]);
        let last = last_line(&app);
        assert!(last.contains("no symbol tagged \"XV-9999\""), "{last}");
    }

    /// A symbol row of the legend list does what PIDTAG does, and names the
    /// symbol by its handles so a click still lands after the sheet was
    /// re-read.
    #[test]
    fn a_legend_list_symbol_row_selects_and_frames_the_group() {
        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, _pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        let _ = app.update(Message::PidLegendPickSymbol(vec![insert.value()]));
        let mut expected = vec![insert, tag];
        expected.sort_by_key(|h| h.value());
        assert_eq!(selected(&app), expected);
        assert!(!app.tabs[i].scene.pid_group_highlight_wires().is_empty());
        assert!(
            app.tabs[i].pid_legend().is_some(),
            "the click read the sheet"
        );
        // A handle of nothing recognised selects nothing and says so.
        let _ = app.update(Message::PidLegendPickSymbol(vec![u64::MAX]));
        assert_eq!(selected(&app), expected, "the selection is left as it was");
        let last = last_line(&app);
        assert!(last.contains("no longer on the sheet"), "{last}");
    }

    #[test]
    fn pidlegend_on_draws_off_removes_and_on_twice_does_not_stack() {
        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf") {
            return;
        }
        let i = app.active_tab;
        let before = app.tabs[i].scene.document.entities().count();
        let rules = Rules::load();
        let recognition = pid_legend::recognise(&app.tabs[i].scene.document, &rules);
        assert_eq!(recognition.symbols.len(), 118);
        // A box and a label per symbol, then the pipe runs with their rings.
        let expected = pid_legend::legend_entities(&recognition, &rules).len();
        assert!(expected > 118 * 2, "{expected}");

        let _ = app.run_command_line("PIDLEGEND ON");
        assert_eq!(legend_count(&app), expected);
        // ON stashes the recognition for the legend list panel and opens it.
        assert_eq!(
            app.tabs[i].pid_legend().map(|r| r.symbols.len()),
            Some(118),
            "recognition stored on the tab"
        );
        assert!(
            !app.tabs[i].pid_legend_is_stale(),
            "drawing the legend does not date the index"
        );
        assert!(app.show_pid_legend_list, "ON opens the legend list");
        assert!(
            app.dock
                .location(crate::ui::dock::PanelId::PidLegend)
                .is_some(),
            "panel got a dock slot"
        );
        let layer = app.tabs[i]
            .scene
            .document
            .layers
            .get("PID-LEGEND-BUTTERFLY")
            .expect("class layer created");
        assert_eq!(
            layer.color,
            Color::Rgb {
                r: 255,
                g: 140,
                b: 0
            }
        );
        assert!(
            app.tabs[i]
                .scene
                .document
                .layers
                .get("PID-PIPE-100-FW")
                .is_some(),
            "a layer per line number"
        );
        assert!(app.tabs[i].dirty);

        let _ = app.run_command_line("PIDLEGEND ON");
        assert_eq!(legend_count(&app), expected, "a second ON replaces");
        assert!(!app.tabs[i].pid_legend_is_stale(), "nor does replacing it");

        let _ = app.run_command_line("PIDLEGEND OFF");
        assert_eq!(legend_count(&app), 0);
        assert_eq!(app.tabs[i].scene.document.entities().count(), before);
        assert!(
            app.tabs[i].pid_legend.is_some(),
            "the list stays browsable after OFF"
        );
        assert!(
            !app.tabs[i].pid_legend_is_stale(),
            "taking the legend out does not date the index either"
        );
    }

    /// The `<g tagName="…">` element for `tag`, open tag to its own close --
    /// the paint runs inside are groups too, so the close is found by nesting.
    #[cfg(not(target_arch = "wasm32"))]
    fn group_element<'a>(svg: &'a str, tag: &str) -> &'a str {
        let open = svg
            .find(&format!("<g tagName=\"{tag}\">"))
            .unwrap_or_else(|| panic!("no group named {tag:?}"));
        let (mut depth, mut at) = (0usize, open);
        loop {
            let rest = &svg[at..];
            match (rest.find("<g"), rest.find("</g>")) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    at += o + 2;
                }
                (_, Some(c)) => {
                    depth -= 1;
                    at += c + 4;
                    if depth == 0 {
                        return &svg[open..at];
                    }
                }
                _ => panic!("the group {tag:?} never closes"),
            }
        }
    }

    /// An SVG plot carries the sheet's tagged symbols as groups: a butterfly
    /// valve block with `BUV-3101` lettered beside it comes out as one
    /// `<g tagName="BUV-3101">` holding the block's strokes and the tag's
    /// glyphs and nothing else -- the pipe through it stays outside -- and a
    /// plot job carries no groups until the SVG entry point asks.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_svg_plot_groups_a_tagged_symbol_with_its_tag() {
        use crate::app::update::file::PlotRequest;
        use crate::io::svg_export::{svg_job_to_string, SvgOptions};

        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;

        app.select_layout_headless("Model").unwrap();
        app.set_headless_model_page("A4", true, true, None, None, 1.0)
            .unwrap();
        let mut job = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        assert!(
            job.assets.groups.is_empty(),
            "a plot job carries no groups by itself"
        );
        app.attach_pid_groups(&mut job);
        assert!(
            app.tabs[i].pid_legend().is_some(),
            "the plot read the sheet and kept the index"
        );
        let mut members = vec![insert.value().to_string(), tag.value().to_string()];
        members.sort();
        assert_eq!(
            job.assets.groups,
            [crate::io::plot_emit::PlotGroup {
                tag: "BUV-3101".into(),
                members,
            }]
        );

        let (svg, report) =
            svg_job_to_string(&job.pages, None, &job.assets, &SvgOptions::default()).unwrap();
        assert_eq!(report.groups, 1);
        let group = group_element(&svg, "BUV-3101");
        let inside = group.matches("<path").count();
        // The pipe and the frame's four sides are all that is drawn outside
        // the group (the clip path in `<defs>` is not ink).
        let drawn = &svg[svg.find("</defs>").unwrap()..];
        let total = drawn.matches("<path").count();
        assert!(
            inside >= 4 + 8,
            "four body strokes and eight glyphs, got {inside} of {total} drawn:\n{group}"
        );
        assert_eq!(total, inside + 5, "{svg}");
        let _ = pipe;
    }

    /// The whole of a real sheet: every tagged symbol recognised on FF02-06
    /// draws as a group, and a block symbol's group holds its strokes and
    /// its tag's glyphs. Ignored by default: laying out a whole sheet's text
    /// grows the glyph atlas for seconds, which the plot tests running beside
    /// it can only re-lay out so many times (R1).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "plots a real sheet; run alone with --ignored, with the sheets beside the checkout"]
    fn a_real_sheet_plots_every_tagged_symbol_as_a_group() {
        use crate::app::update::file::PlotRequest;
        use crate::io::svg_export::{svg_job_to_string, SvgOptions};

        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf") {
            return;
        }
        app.select_layout_headless("Model").unwrap();
        app.set_headless_model_page("A1", true, true, None, None, 1.0)
            .unwrap();
        let mut job = app.resolve_plot_job(&PlotRequest::current_view()).unwrap();
        app.attach_pid_groups(&mut job);
        let i = app.active_tab;
        let recognition = app.tabs[i].pid_legend().expect("the plot read the sheet");
        let tagged: Vec<&pid_legend::Recognized> = recognition
            .symbols
            .iter()
            .filter(|s| s.tag.is_some())
            .collect();
        assert_eq!(job.assets.groups.len(), tagged.len());
        assert!(
            tagged.len() >= 60,
            "{} tagged symbols on FF02-06",
            tagged.len()
        );

        let (svg, report) =
            svg_job_to_string(&job.pages, None, &job.assets, &SvgOptions::default())
                .expect("the sheet plots");
        assert_eq!(
            report.groups,
            tagged.len(),
            "every tagged symbol draws as a group"
        );
        assert_eq!(svg.matches("<g tagName=\"").count(), tagged.len());
        let valve = tagged
            .iter()
            .find(|s| s.source.starts_with('$') && !s.tag_handles.is_empty())
            .expect("a block symbol with a tag lettered beside it");
        let group = group_element(&svg, valve.tag.as_deref().unwrap());
        assert!(
            group.matches("<path").count() >= 2,
            "{}",
            &group[..group.len().min(400)]
        );
    }

    /// The index is of the drawing it was read from. Once the drawing moves
    /// on -- here a line is added -- the panel is told it is out of date, and
    /// PIDLINE, a pipe row of the panel and LIST each read the sheet again
    /// before they use it (C4 / D24). An empty drawing serves: what is under
    /// test is the epoch, not the recognition.
    #[test]
    fn a_changed_sheet_dates_the_index_and_it_is_read_again_before_use() {
        use acadrust::entities::{EntityType, Line};
        use acadrust::types::Vector3;
        let add_line = |app: &mut OpenCADStudio, i: usize, x: f64| {
            let mut line = Line::new();
            line.start = Vector3::new(x, 0.0, 0.0);
            line.end = Vector3::new(x + 10.0, 0.0, 0.0);
            app.tabs[i].scene.add_entity(EntityType::Line(line));
        };
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        add_line(&mut app, i, 0.0);

        let _ = app.run_command_line("PIDLEGEND LIST");
        let read_at = app.tabs[i].pid_legend.as_ref().expect("LIST reads").epoch;
        assert_eq!(read_at, app.tabs[i].scene.geometry_epoch);
        assert!(!app.tabs[i].pid_legend_is_stale());

        // An edit dates it.
        add_line(&mut app, i, 20.0);
        assert!(
            app.tabs[i].pid_legend_is_stale(),
            "an added entity dates the index"
        );

        // PIDLINE reads again before it looks the code up; the code is not on
        // this sheet, the index is current again all the same.
        let _ = app.run_command_line("PIDLINE 100-FW");
        assert!(
            !app.tabs[i].pid_legend_is_stale(),
            "PIDLINE read the sheet again"
        );
        let read_at_2 = app.tabs[i].pid_legend.as_ref().unwrap().epoch;
        assert!(read_at_2 > read_at);

        // A pipe row picked in the panel goes the same way.
        add_line(&mut app, i, 40.0);
        assert!(app.tabs[i].pid_legend_is_stale());
        let families: BTreeSet<String> = std::iter::once("FW-1".to_string()).collect();
        assert!(app.pid_line_select(i, &families, &[]).is_none());
        assert!(
            !app.tabs[i].pid_legend_is_stale(),
            "the panel's pick read the sheet again"
        );

        // And LIST, opening the panel on a dated index, reads too; a current
        // index it leaves alone.
        assert!(app.show_pid_legend_list, "LIST opened the panel");
        let _ = app.run_command_line("PIDLEGEND LIST");
        assert!(!app.show_pid_legend_list, "second LIST hid it");
        add_line(&mut app, i, 60.0);
        assert!(app.tabs[i].pid_legend_is_stale());
        let _ = app.run_command_line("PIDLEGEND LIST");
        assert!(app.show_pid_legend_list);
        assert!(
            !app.tabs[i].pid_legend_is_stale(),
            "LIST read the sheet again"
        );
        let read_at_3 = app.tabs[i].pid_legend.as_ref().unwrap().epoch;
        let _ = app.run_command_line("PIDLINE 100-FW");
        assert_eq!(
            app.tabs[i].pid_legend.as_ref().unwrap().epoch,
            read_at_3,
            "a current index is not read again"
        );
    }

    /// LIST recognises without drawing and toggles the docked panel.
    #[test]
    fn pidlegend_list_fills_the_panel_without_drawing() {
        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf") {
            return;
        }
        let i = app.active_tab;
        assert!(app.tabs[i].pid_legend.is_none());
        assert!(!app.show_pid_legend_list);

        let _ = app.run_command_line("PIDLEGEND LIST");
        let recognition = app.tabs[i].pid_legend().expect("recognition stored");
        assert_eq!(recognition.symbols.len(), 118);
        assert!(
            !recognition.pipes.runs.is_empty(),
            "pipe runs listed for the line-number section"
        );
        assert!(app.show_pid_legend_list, "first LIST shows the panel");
        assert_eq!(legend_count(&app), 0, "LIST draws nothing");

        let _ = app.run_command_line("PIDLEGEND LIST");
        assert!(!app.show_pid_legend_list, "second LIST hides the panel");
        assert!(
            app.tabs[i].pid_legend.is_some(),
            "hiding keeps the recognition"
        );
    }

    /// PIDLINE resolves a line (typed or picked) to its whole family and
    /// selects every sheet stroke the family's runs are made of.
    #[test]
    fn pidline_selects_the_whole_line_from_a_code_or_a_picked_stroke() {
        use std::collections::BTreeSet;
        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf") {
            return;
        }
        let i = app.active_tab;

        // By code. PIDLINE recognises on first use, no PIDLEGEND run needed.
        let _ = app.run_command_line("PIDLINE 100-FW");
        let rec = app.tabs[i]
            .pid_legend()
            .expect("PIDLINE recognises on first use")
            .clone();
        assert!(
            rec.pipes.runs.iter().all(|r| !r.handles.is_empty()),
            "every run knows the strokes it is made of"
        );
        let by_line = rec.pipes.by_line();
        let runs = by_line.get("100-FW").expect("the sheet letters 100-FW");
        let expected: BTreeSet<_> = runs
            .iter()
            .flat_map(|r| r.handles.iter().copied())
            .collect();
        assert!(!expected.is_empty());
        let selected: BTreeSet<_> = app.tabs[i].scene.selected.iter().copied().collect();
        assert_eq!(selected, expected, "the code selects the line's strokes");

        // From a picked stroke of a run lettered 100-FW alone: the same set.
        // Bare PIDLINE prompts; Enter at the prompt resolves the pick.
        let seed = runs
            .iter()
            .find(|r| r.lines == ["100-FW"])
            .expect("a run lettered 100-FW alone");
        let mut one = rustc_hash::FxHashSet::default();
        one.insert(seed.handles[0]);
        app.tabs[i].scene.replace_selection(one);
        let _ = app.run_command_line("PIDLINE");
        assert!(app.tabs[i].active_cmd.is_some(), "bare PIDLINE prompts");
        let _ = app.feed_command(crate::command::StepInput::Enter);
        let selected: BTreeSet<_> = app.tabs[i].scene.selected.iter().copied().collect();
        assert_eq!(selected, expected, "one pick grows to the whole line");

        // An unknown code selects nothing and keeps the selection.
        let _ = app.run_command_line("PIDLINE 999-ZZ");
        let selected_after: BTreeSet<_> =
            app.tabs[i].scene.selected.iter().copied().collect();
        assert_eq!(selected_after, expected, "an unknown code changes nothing");
    }

    #[test]
    fn pidlegend_marks_up_an_exploded_sheet_with_shape_colours() {
        let mut app = OpenCADStudio::new_for_test();
        if !open_sheet(&mut app, "DWG-0100SP02-05 发油泵棚(二)工艺自控流程图.dxf") {
            return;
        }
        let i = app.active_tab;
        let recognition =
            pid_legend::recognise(&app.tabs[i].scene.document, &pid_legend::Rules::load());
        let _ = app.run_command_line("PIDLEGEND ON");
        let doc = &app.tabs[i].scene.document;
        assert!(legend_count(&app) > 200, "{}", legend_count(&app));
        assert!(doc.layers.get("PID-LEGEND-EVALVE").is_some());
        // Unnamed shapes carry their own colour; everything else is ByLayer.
        // (As the dictionary fills up a sheet may have none left; then there
        // is nothing on the shape layer either.)
        let (own, by_layer): (Vec<_>, Vec<_>) = doc
            .model_space_entities()
            .filter(|e| pid_legend::is_legend_layer(&e.common().layer))
            .partition(|e| e.common().layer == pid_legend::SHAPE_LAYER);
        if recognition.unknown_shapes.is_empty() {
            assert!(own.is_empty());
        } else {
            assert!(doc.layers.get(pid_legend::SHAPE_LAYER).is_some());
            assert!(!own.is_empty() && own.iter().all(|e| e.common().color != Color::ByLayer));
        }
        assert!(by_layer.iter().all(|e| e.common().color == Color::ByLayer));
    }
}

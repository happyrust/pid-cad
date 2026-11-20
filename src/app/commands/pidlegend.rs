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
//! PIDLEGEND ON         recognise, publish entity XDATA, then draw (replacing
//!                      an earlier legend)
//! PIDLEGEND OFF        remove the legend entities; published XDATA stays
//! PIDLEGEND PURGE      remove both legend entities and recognition XDATA
//! PIDLEGEND REPORT     recognise and print the summary, draw nothing
//! PIDLEGEND LIST       toggle the docked legend list panel (recognising
//!                      first when this sheet has not been read yet)
//! PIDLEGEND EXPORT p   write the current recognition as JSON or CSV,
//!                      selected by `p`'s extension
//! PIDLINE              prompt for a line number or family (Enter = resolve
//!                      the picked stroke instead)
//! PIDLINE <code>       select the whole line by number or family
//!                      (PIDLINE 100-FW, PIDLINE FS-31001)
//! PIDTAG               prompt for a tag (Enter = the picked symbol's)
//! PIDTAG <tag>         select the symbol(s) carrying the tag with their tag
//!                      lettering, frame them in red, zoom to them
//! PIDGROUP             prompt with the verbs below (Enter = GROUP)
//! PIDGROUP GROUP       group the selection as a symbol, tag read from its
//!                      own lettering (GROUP does the same, and asks a name)
//! PIDGROUP TAG <tag>   set the group's tag by hand; AUTO reads it again;
//!                      OFF dissolves the group (= UNGROUP)
//! ```
//!
//! A group a person makes is a P&ID symbol when its description carries
//! `tagName=` (`io::pid_legend::GroupTag`): GROUP and PIDGROUP read the tag
//! from the group's own lettering by the recognition's rule and write it in;
//! a tag set by hand is `tagSource=manual`. The group helpers here
//! (`make_pid_group`, `dissolve_pid_groups`) are what GROUP and UNGROUP run
//! too, so a drawn legend is redrawn in the same undo step as the group
//! change.
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
//! A symbol a `.pid` import placed is no recognised symbol -- its block
//! reference says what it is, and recognition leaves it out -- but PIDTAG
//! finds it all the same, by the tag its record states
//! (`io::pid_identity`): its group is its block reference with the
//! lettering of its labels (`imported_groups`). Its tag is the plant's, and
//! `PIDTAG <old> <new>` does not rename it.
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
use crate::app::history::PendingObjectDelta;
use crate::io::pid_legend::{self, GroupTag, Rules, TagSource};
use document::types::Color;
use std::collections::BTreeSet;

fn export_path_argument(cmd: &str, option: &str) -> Option<std::path::PathBuf> {
    let tail = cmd
        .trim_start()
        .get("PIDLEGEND".len()..)?
        .trim_start()
        .get(option.len()..)?
        .trim();
    if tail.is_empty() {
        return None;
    }
    let unquoted = if tail.len() >= 2
        && ((tail.starts_with('"') && tail.ends_with('"'))
            || (tail.starts_with('\'') && tail.ends_with('\'')))
    {
        &tail[1..tail.len() - 1]
    } else {
        tail
    };
    Some(std::path::PathBuf::from(unquoted))
}

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
                "PIDLEGEND  [ON / OFF / PURGE / REPORT / LIST / EXPORT] <ON>:",
                vec![
                    ("ON", "ON", None),
                    ("OFF", "OFF", None),
                    ("PURGE", "PURGE", None),
                    ("REPORT", "REPORT", None),
                    ("LIST", "LIST", None),
                    ("EXPORT", "EXPORT", None),
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
            "PURGE" => {
                let legend = pid_legend::legend_handles(&self.tabs[i].scene.document);
                let metadata = pid_legend::attached_handles(&self.tabs[i].scene.document);
                if legend.is_empty() && metadata.is_empty() {
                    self.command_line
                        .push_info("PIDLEGEND: no legend entities or recognition XDATA to remove.");
                } else {
                    self.push_undo_snapshot(i, "PIDLEGEND PURGE");
                    let was = self.tabs[i].scene.geometry_epoch;
                    if !legend.is_empty() {
                        self.tabs[i].scene.erase_entities(&legend);
                    }
                    let purged = pid_legend::purge(&mut self.tabs[i].scene.document);
                    self.tabs[i].dirty = true;
                    // Neither the overlay nor its published copy is input to
                    // recognition, so the in-memory list remains current.
                    self.tabs[i].pid_legend_survives(was);
                    self.refresh_properties();
                    self.command_line.push_output(&format!(
                        "PIDLEGEND: removed {} legend entities and {purged} recognition XDATA record(s).",
                        legend.len()
                    ));
                }
            }
            "EXPORT" => {
                let Some(path) = export_path_argument(cmd, &option) else {
                    if cmd.trim_end() != cmd {
                        self.command_line
                            .push_error("Usage: PIDLEGEND EXPORT <file.json|file.csv>");
                        return Some(Task::none());
                    }
                    use crate::command::ValuePromptCommand;
                    let prompt = "PIDLEGEND EXPORT  file path (.json or .csv):";
                    let command = ValuePromptCommand::new("PIDLEGEND EXPORT", prompt);
                    self.command_line.push_info(&command.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(command));
                    return Some(self.finish_dispatch(cmd));
                };
                match self.export_pid_legend(i, &path) {
                    Ok(receipt) => {
                        let recognition = self.tabs[i].pid_legend().expect("export recognised");
                        self.command_line.push_output(&format!(
                            "PIDLEGEND: exported {} symbols and {} pipe runs to {} ({}, {} bytes).",
                            recognition.symbols.len(),
                            recognition.pipes.runs.len(),
                            path.display(),
                            receipt.format.name(),
                            receipt.bytes
                        ));
                    }
                    Err(error) => self
                        .command_line
                        .push_error(&format!("PIDLEGEND EXPORT: {error}")),
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
                // The stored tag of a hand-made group whose lettering has
                // moved on is refreshed here, where the sheet is being marked
                // up anyway (D35): the recognition reads the lettering as it
                // stands either way.
                self.refresh_auto_group_tags(i, &rules);
                let recognition = pid_legend::recognise(&self.tabs[i].scene.document, &rules);
                if recognition.symbols.is_empty() {
                    // An earlier ON may have published a now-deleted symbol.
                    // Clear its two published forms in one undo step instead
                    // of leaving stale boxes or XDATA on the sheet.
                    let legend = pid_legend::legend_handles(&self.tabs[i].scene.document);
                    let metadata = pid_legend::attached_handles(&self.tabs[i].scene.document);
                    if !legend.is_empty() || !metadata.is_empty() {
                        self.push_undo_snapshot(i, "PIDLEGEND");
                        if !legend.is_empty() {
                            self.tabs[i].scene.erase_entities(&legend);
                        }
                        pid_legend::attach(&mut self.tabs[i].scene.document, &recognition);
                        self.tabs[i].dirty = true;
                    }
                    self.tabs[i].set_pid_legend(recognition);
                    self.refresh_properties();
                    self.command_line.push_info(
                        "PIDLEGEND: nothing recognised -- no known block or circle symbol on this sheet. PIDLEGEND REPORT lists what was seen.",
                    );
                    return Some(Task::none());
                }
                self.push_undo_snapshot(i, "PIDLEGEND");
                for line in pid_legend::report(&recognition) {
                    self.command_line.push_output(&line);
                }
                let (drawn, layers, pipe_layers) = self.place_pid_legend(i, &rules, recognition);
                self.command_line.push_output(&format!(
                    "PIDLEGEND: drew {drawn} entities on {layers} layers ({}*{}). PIDLEGEND OFF removes them.",
                    pid_legend::LAYER_PREFIX,
                    if pipe_layers > 0 {
                        format!(", {pipe_layers} of them {}*", pid_legend::PIPE_LAYER_PREFIX)
                    } else {
                        String::new()
                    }
                ));
                self.open_pid_legend_panel();
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDLEGEND: unknown option \"{other}\" -- use ON, OFF, PURGE, REPORT, LIST or EXPORT."
                ));
            }
        }
        Some(Task::none())
    }

    /// Recognise the current sheet when necessary and write its structured
    /// JSON/CSV representation. Export never draws the overlay or publishes
    /// XDATA; it is valid before ON and after OFF.
    pub(in crate::app) fn export_pid_legend(
        &mut self,
        i: usize,
        path: &std::path::Path,
    ) -> Result<pid_legend::ExportReceipt, String> {
        self.refresh_pid_legend(i);
        let source = self.tabs[i]
            .current_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| self.tabs[i].tab_title.clone());
        let recognition = self.tabs[i]
            .pid_legend()
            .ok_or_else(|| "recognition is unavailable".to_string())?;

        #[cfg(not(target_arch = "wasm32"))]
        {
            pid_legend::write_export(path, &source, recognition)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let format = pid_legend::ExportFormat::for_path(path)?;
            let body = pid_legend::render_export(path, &source, recognition)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_else(|| match format {
                    pid_legend::ExportFormat::Json => "pid-legend.json",
                    pid_legend::ExportFormat::Csv => "pid-legend.csv",
                });
            crate::sys::download_bytes(name, body.as_bytes());
            Ok(pid_legend::ExportReceipt {
                format,
                bytes: body.len(),
            })
        }
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

    /// Put `recognition`'s legend into the drawing, replacing the legend
    /// already there (running it twice must not stack two), and keep the
    /// recognition as the tab's index. What PIDLEGEND ON does once it has
    /// decided to draw; also how the legend is redrawn after a group command
    /// (D38). The caller has opened the undo step. Returns the entities
    /// drawn, the layers used and how many of those are pipe layers.
    fn place_pid_legend(
        &mut self,
        i: usize,
        rules: &Rules,
        recognition: pid_legend::Recognition,
    ) -> (usize, usize, usize) {
        // The overlay and entity metadata are one published view of the same
        // recognition and therefore one undo step. `attach` also drops stale
        // legend-owned records left on entities no longer recognised.
        pid_legend::attach(&mut self.tabs[i].scene.document, &recognition);
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
        let entities = pid_legend::legend_entities(&recognition, rules);
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
        let pipe_layers = layers
            .keys()
            .filter(|name| name.starts_with(pid_legend::PIPE_LAYER_PREFIX))
            .count();
        // Stamped after the legend went in: drawing it is not a change to
        // the sheet it indexes.
        self.tabs[i].set_pid_legend(recognition);
        self.refresh_properties();
        (drawn, layers.len(), pipe_layers)
    }

    /// When a legend is drawn on the sheet, read the sheet again and redraw
    /// it, inside the undo step the caller opened -- so a group made or
    /// dissolved shows up in the coloured boxes at once (D38). A sheet with
    /// no legend drawn is left alone: PIDLEGEND ON is the user's call. True
    /// when it redrew.
    fn redraw_pid_legend_if_drawn(&mut self, i: usize, rules: &Rules) -> bool {
        if pid_legend::legend_handles(&self.tabs[i].scene.document).is_empty() {
            return false;
        }
        let recognition = pid_legend::recognise(&self.tabs[i].scene.document, rules);
        let (drawn, ..) = self.place_pid_legend(i, rules, recognition);
        self.command_line
            .push_info(&format!("PIDLEGEND: legend redrawn, {drawn} entities."));
        true
    }

    /// Keep recognition XDATA current across GROUP / UNGROUP / PIDGROUP even
    /// after `PIDLEGEND OFF` hid the overlay. `published` is captured before
    /// the edit, because dissolving the last manual group may make the new
    /// recognition empty and `attach` still has stale records to remove.
    fn refresh_pid_publication_after_group_change(
        &mut self,
        i: usize,
        rules: &Rules,
        published: bool,
    ) {
        if self.redraw_pid_legend_if_drawn(i, rules) || !published {
            return;
        }
        let recognition = pid_legend::recognise(&self.tabs[i].scene.document, rules);
        pid_legend::attach(&mut self.tabs[i].scene.document, &recognition);
        self.tabs[i].set_pid_legend(recognition);
        self.refresh_properties();
    }

    /// Bring the stored tag of every `auto` group up to what its lettering
    /// reads as now (D35): the description is a copy for whoever reads the
    /// file, the lettering is the truth. Returns how many changed. No undo
    /// of its own; callers run it inside their own step.
    pub(in crate::app) fn refresh_auto_group_tags(&mut self, i: usize, rules: &Rules) -> usize {
        let scene = &self.tabs[i].scene;
        let refreshed: Vec<(document::Handle, GroupTag)> = scene
            .tagged_groups()
            .filter(|(_, tag)| tag.source == TagSource::Auto)
            .filter_map(|(group, tag)| {
                let read = pid_legend::derive_group_tag(&scene.document, group, rules)
                    .map(|read| read.value);
                (read != tag.name).then(|| (group.handle, GroupTag::auto(read)))
            })
            .collect();
        let count = refreshed.len();
        for (handle, tag) in refreshed {
            self.tabs[i].scene.set_group_tag(handle, Some(&tag));
        }
        if count > 0 {
            self.tabs[i].dirty = true;
        }
        count
    }

    /// Whether the sheet has a legend drawn or recognition XDATA published:
    /// then a group command's undo step has to cover entities as well as group
    /// objects, so it takes the full snapshot PIDLEGEND ON takes rather than
    /// the group-object delta.
    fn open_group_undo(&mut self, i: usize, label: &str) -> Option<PendingObjectDelta> {
        if pid_legend::legend_handles(&self.tabs[i].scene.document).is_empty()
            && pid_legend::attached_handles(&self.tabs[i].scene.document).is_empty()
        {
            Some(self.begin_group_undo(i, label))
        } else {
            self.push_undo_snapshot(i, label);
            None
        }
    }

    fn close_group_undo(&mut self, i: usize, pending: Option<PendingObjectDelta>) {
        if let Some(pending) = pending {
            self.commit_group_undo(i, pending);
        }
    }

    /// GROUP and PIDGROUP, once the members are known: make the group, read
    /// its tag from its own lettering by the recognition's rule and write it
    /// into the description when there is one (a group that letters nothing
    /// readable stays a plain group, D31), redraw the legend when one is
    /// drawn -- one undo step. `tag` names the tag by hand instead of
    /// reading it (`PIDGROUP TAG <tag>` on an ungrouped selection). Returns
    /// the group's handle and the receipt; `None` when nothing groupable was
    /// picked.
    pub(in crate::app) fn make_pid_group(
        &mut self,
        i: usize,
        label: &str,
        name: String,
        mut handles: Vec<document::Handle>,
        tag: Option<&str>,
    ) -> Option<(document::Handle, String)> {
        handles.retain(|handle| !self.tabs[i].scene.is_layer_locked(*handle));
        if handles.is_empty() {
            return None;
        }
        let rules = Rules::load();
        let published = !pid_legend::attached_handles(&self.tabs[i].scene.document).is_empty();
        let pending = self.open_group_undo(i, label);
        let group = self.tabs[i].scene.create_group(name.clone(), handles);
        self.tabs[i].dirty = true;
        let created = crate::tf!("Group \"{}\" created.", name);
        let tail = match tag {
            Some(tag) => self.set_pid_group_tag(i, group, Some(tag), &rules),
            None => self.read_pid_group_tag(i, group, &rules),
        };
        self.refresh_pid_publication_after_group_change(i, &rules, published);
        self.close_group_undo(i, pending);
        Some((group, format!("{created} {tail}")))
    }

    /// Read a new group's tag from its lettering and write it in; the tail of
    /// the GROUP receipt says what was read, or why nothing was.
    fn read_pid_group_tag(&mut self, i: usize, group: document::Handle, rules: &Rules) -> String {
        match self.tabs[i].scene.derive_group_tag(group, rules) {
            Some(read) => {
                let class = pid_legend::class_for_tag(&read.value, rules);
                self.tabs[i]
                    .scene
                    .set_group_tag(group, Some(&GroupTag::auto(Some(read.value.clone()))));
                match class {
                    Some(class) => format!("tagName {} ({}).", read.value, class.label),
                    None => format!("tagName {}.", read.value),
                }
            }
            None => {
                let texts = self.group_lettering_count(i, group);
                format!(
                    "No tagName: the group letters nothing that reads as a tag ({texts} text{}). Set one in Properties or with PIDGROUP TAG <tag>.",
                    if texts == 1 { "" } else { "s" }
                )
            }
        }
    }

    /// How many pieces of lettering (TEXT / MTEXT) a group has among its
    /// members -- for the receipt that says why no tag was read.
    fn group_lettering_count(&self, i: usize, group: document::Handle) -> usize {
        use document::objects::ObjectType;
        let doc = &self.tabs[i].scene.document;
        match doc.objects.get(&group) {
            Some(ObjectType::Group(g)) => g
                .entities
                .iter()
                .filter_map(|h| doc.get_entity(*h))
                .filter(|e| matches!(e, document::EntityType::Text(_) | document::EntityType::MText(_)))
                .count(),
            _ => 0,
        }
    }

    /// Set a group's tag by hand (`Some`), or hand it back to its lettering
    /// (`None`). A hand-set value that is what the lettering reads anyway, or
    /// an empty one, is `auto` too: `manual` is only for a value the
    /// lettering would not give (D35). Returns the tail of the receipt.
    fn set_pid_group_tag(
        &mut self,
        i: usize,
        group: document::Handle,
        value: Option<&str>,
        rules: &Rules,
    ) -> String {
        let tag = self.pid_group_tag_for_value(i, group, value, rules);
        self.tabs[i].scene.set_group_tag(group, Some(&tag));
        match (&tag.name, tag.source) {
            (Some(name), TagSource::Manual) => format!("tagName {name} (set by hand)."),
            (Some(name), TagSource::Auto) => format!("tagName {name} (read from the lettering)."),
            (None, _) => "No tagName: the group letters nothing that reads as a tag.".to_string(),
        }
    }

    fn pid_group_tag_for_value(
        &self,
        i: usize,
        group: document::Handle,
        value: Option<&str>,
        rules: &Rules,
    ) -> GroupTag {
        let read = self.tabs[i]
            .scene
            .derive_group_tag(group, rules)
            .map(|read| read.value);
        let value = value.map(str::trim).filter(|v| !v.is_empty());
        match value {
            Some(v) if read.as_deref() != Some(v) => GroupTag::manual(v),
            _ => GroupTag::auto(read.clone()),
        }
    }

    /// UNGROUP and PIDGROUP OFF: dissolve every group containing one of
    /// `handles`, redrawing the legend when one is drawn (D38) -- one undo
    /// step. Returns how many groups went.
    pub(in crate::app) fn dissolve_pid_groups(
        &mut self,
        i: usize,
        label: &str,
        handles: &[document::Handle],
    ) -> usize {
        let handles: Vec<document::Handle> = handles
            .iter()
            .copied()
            .filter(|handle| !self.tabs[i].scene.is_layer_locked(*handle))
            .collect();
        if handles.is_empty() {
            return 0;
        }
        let published = !pid_legend::attached_handles(&self.tabs[i].scene.document).is_empty();
        let pending = self.open_group_undo(i, label);
        let count = self.tabs[i].scene.delete_groups_containing(&handles);
        self.tabs[i].dirty = true;
        if count > 0 {
            // Removing a marked group hands its members back to automatic
            // recognition even though no entity geometry changed.
            self.tabs[i].scene.bump_geometry();
            let rules = Rules::load();
            self.refresh_pid_publication_after_group_change(i, &rules, published);
        }
        self.close_group_undo(i, pending);
        count
    }

    /// PIDGROUP: a group as a P&ID symbol.
    ///
    /// ```text
    /// PIDGROUP              prompt with the verbs below (Enter = GROUP)
    /// PIDGROUP GROUP        group the selection; the tag is read from the
    ///                       group's own lettering (no name prompt: *A<n>)
    /// PIDGROUP TAG <tag>    set the tag of the selection's group(s) by hand
    ///                       (an ungrouped selection is grouped first); the
    ///                       value the lettering reads anyway is `auto`
    /// PIDGROUP AUTO         read the tag from the lettering again
    /// PIDGROUP OFF          dissolve the selection's group(s) (= UNGROUP)
    /// ```
    ///
    /// GROUP and UNGROUP do the same reading and dissolving; PIDGROUP is the
    /// scriptable front that skips the name prompt and carries the by-hand
    /// verbs. Bare `PIDGROUP` only prompts, like bare PIDLEGEND: the driver
    /// dispatches the first word of an inline line on its own first.
    pub(super) fn dispatch_pidgroup(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let rest = cmd.strip_prefix("PIDGROUP")?;
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return None;
        }
        let mut words = rest.split_whitespace();
        let Some(verb) = words.next().map(str::to_uppercase) else {
            use crate::command::KeywordCommand;
            let c = KeywordCommand::new(
                "PIDGROUP",
                "PIDGROUP  [GROUP / TAG / AUTO / OFF] <GROUP>:",
                vec![
                    ("GROUP", "GROUP", None),
                    ("TAG", "TAG", Some("PIDGROUP  tag:")),
                    ("AUTO", "AUTO", None),
                    ("OFF", "OFF", None),
                ],
            )
            .with_default("GROUP");
            self.command_line.push_info(&c.prompt());
            self.tabs[i].active_cmd = Some(Box::new(c));
            return Some(self.finish_dispatch(cmd));
        };
        let selected: Vec<document::Handle> = self.tabs[i].scene.selected_handles_in_order();
        match verb.as_str() {
            "GROUP" => {
                if selected.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let c = SelectObjectsCommand::plain("PIDGROUP", "PIDGROUP GROUP");
                    self.command_line.push_info(&c.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(c));
                    return Some(self.finish_dispatch(cmd));
                }
                let name = crate::app::helpers::next_group_auto_name(&self.tabs[i].scene);
                match self.make_pid_group(i, "PIDGROUP", name, selected, None) {
                    Some((_, receipt)) => self.command_line.push_info(&receipt),
                    None => self
                        .command_line
                        .push_info("PIDGROUP: nothing groupable selected (locked layers?)."),
                }
            }
            "TAG" => {
                let value = rest.trim_start()["TAG".len()..].trim();
                if value.is_empty() {
                    self.command_line.push_error(
                        "PIDGROUP TAG needs the tag -- PIDGROUP TAG BUV-3101. PIDGROUP AUTO reads it from the lettering again.",
                    );
                    return Some(Task::none());
                }
                if selected.is_empty() {
                    self.command_line.push_info(
                        "PIDGROUP: pick the group's members first, then PIDGROUP TAG <tag>.",
                    );
                    return Some(Task::none());
                }
                let groups = self.groups_of(i, &selected);
                if groups.is_empty() {
                    // An ungrouped selection: group it with the tag given.
                    let name = crate::app::helpers::next_group_auto_name(&self.tabs[i].scene);
                    match self.make_pid_group(i, "PIDGROUP", name, selected, Some(value)) {
                        Some((_, receipt)) => self.command_line.push_info(&receipt),
                        None => self
                            .command_line
                            .push_info("PIDGROUP: nothing groupable selected (locked layers?)."),
                    }
                } else {
                    self.retag_pid_groups(i, &groups, Some(value));
                }
            }
            "AUTO" => {
                if selected.is_empty() {
                    self.command_line
                        .push_info("PIDGROUP: pick the group's members first, then PIDGROUP AUTO.");
                    return Some(Task::none());
                }
                let groups = self.groups_of(i, &selected);
                if groups.is_empty() {
                    self.command_line
                        .push_info("PIDGROUP: the picked entities are in no group.");
                    return Some(Task::none());
                }
                self.retag_pid_groups(i, &groups, None);
            }
            "OFF" => {
                if selected.is_empty() {
                    use crate::modules::draw::select::SelectObjectsCommand;
                    let c = SelectObjectsCommand::plain("PIDGROUP", "PIDGROUP OFF");
                    self.command_line.push_info(&c.prompt());
                    self.tabs[i].active_cmd = Some(Box::new(c));
                    return Some(self.finish_dispatch(cmd));
                }
                let count = self.dissolve_pid_groups(i, "PIDGROUP", &selected);
                if count > 0 {
                    self.command_line
                        .push_info(crate::tf!("{} group(s) dissolved.", count).as_ref());
                } else {
                    self.command_line
                        .push_info(crate::t!("No groups found for selected objects.").as_ref());
                }
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDGROUP: unknown option \"{other}\" -- use GROUP, TAG <tag>, AUTO or OFF."
                ));
            }
        }
        Some(Task::none())
    }

    /// The groups containing any of `handles`, each once, in first-seen order.
    fn groups_of(&self, i: usize, handles: &[document::Handle]) -> Vec<document::Handle> {
        let mut groups: Vec<document::Handle> = Vec::new();
        for handle in handles {
            for group in self.tabs[i].scene.groups_containing(*handle) {
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
        }
        groups
    }

    /// Friendly default for the ordinary GROUP prompt. Once the sheet has a
    /// current P&ID index, a selected set that reads one tag uses that tag as
    /// its group name when the tag occurs on exactly one recognised symbol
    /// and no existing group already owns the name. The description remains
    /// the data; this is only a readable dictionary key.
    pub(in crate::app) fn suggested_pid_group_name(
        &self,
        i: usize,
        handles: &[document::Handle],
    ) -> String {
        let fallback = crate::app::helpers::next_group_auto_name(&self.tabs[i].scene);
        if self.tabs[i].pid_legend_is_stale() {
            return fallback;
        }
        let Some(recognition) = self.tabs[i].pid_legend() else {
            return fallback;
        };
        let rules = Rules::load();
        let Some(read) =
            pid_legend::derive_handles_tag(&self.tabs[i].scene.document, handles, &rules)
        else {
            return fallback;
        };
        let candidate = read.value.trim();
        if recognition
            .symbols
            .iter()
            .filter(|symbol| {
                symbol
                    .tag
                    .as_deref()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case(candidate))
            })
            .count()
            != 1
        {
            return fallback;
        }
        let document = &self.tabs[i].scene.document;
        let in_objects = self.tabs[i]
            .scene
            .groups()
            .any(|group| group.name.eq_ignore_ascii_case(candidate));
        let in_dictionary = document
            .objects
            .get(&document.header.acad_group_dict_handle)
            .and_then(|object| match object {
                document::objects::ObjectType::Dictionary(dictionary) => Some(dictionary),
                _ => None,
            })
            .is_some_and(|dictionary| {
                dictionary
                    .entries
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(candidate))
            });
        if in_objects || in_dictionary {
            fallback
        } else {
            candidate.to_string()
        }
    }

    /// Apply the Properties panel's `pid_tag` value to every marked group
    /// touched by the selected handles. Empty, or the value the lettering
    /// already reads, means `auto`; any other value means `manual`.
    pub(in crate::app) fn set_pid_group_tag_property(
        &mut self,
        i: usize,
        handles: &[document::Handle],
        value: &str,
    ) -> usize {
        if value == crate::app::VARIES_LABEL {
            return 0;
        }
        let rules = Rules::load();
        let mut updates = Vec::new();
        for group in self.groups_of(i, handles) {
            let Some(current) = self.tabs[i].scene.group_tag(group) else {
                continue;
            };
            let desired = self.pid_group_tag_for_value(i, group, Some(value), &rules);
            if desired != current {
                updates.push((group, desired));
            }
        }
        if updates.is_empty() {
            return 0;
        }
        let published = !pid_legend::attached_handles(&self.tabs[i].scene.document).is_empty();
        let pending = self.open_group_undo(i, "PIDGROUP TAG");
        for (group, tag) in &updates {
            self.tabs[i].scene.set_group_tag(*group, Some(tag));
        }
        self.tabs[i].dirty = true;
        self.refresh_pid_publication_after_group_change(i, &rules, published);
        self.close_group_undo(i, pending);
        updates.len()
    }

    /// PIDGROUP TAG / AUTO on existing groups: set (`Some`) or hand back
    /// (`None`) the tag of each, redraw the legend when drawn, one undo step,
    /// one receipt line per group.
    fn retag_pid_groups(&mut self, i: usize, groups: &[document::Handle], value: Option<&str>) {
        let rules = Rules::load();
        let published = !pid_legend::attached_handles(&self.tabs[i].scene.document).is_empty();
        let pending = self.open_group_undo(i, "PIDGROUP");
        let mut receipts = Vec::new();
        for group in groups {
            let name = match self.tabs[i].scene.document.objects.get(group) {
                Some(document::objects::ObjectType::Group(g)) => g.name.clone(),
                _ => continue,
            };
            let tail = self.set_pid_group_tag(i, *group, value, &rules);
            receipts.push(format!("Group \"{name}\": {tail}"));
        }
        self.tabs[i].dirty = true;
        self.refresh_pid_publication_after_group_change(i, &rules, published);
        self.close_group_undo(i, pending);
        for receipt in receipts {
            self.command_line.push_info(&receipt);
        }
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
    pub(in crate::app) fn attach_pid_groups(
        &mut self,
        job: &mut crate::app::update::file::PlotJob,
    ) {
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
            let mut handles: rustc_hash::FxHashSet<document::Handle> =
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
                format!(
                    " ({})",
                    lines.iter().copied().collect::<Vec<_>>().join(", ")
                )
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
    ///
    /// A symbol a `.pid` import placed is found by the tag its record
    /// states, or picked by its block reference or by the lettering of one
    /// of its labels; its group is the reference with the lettering of all
    /// its labels, and the receipt then counts symbols and labels.
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
        // `PIDTAG <old> <new>` re-tags: the collaboration contract states a tag
        // change as exactly this line (`VAL-CLI-009`), and it is the only
        // command-line spelling of "set this symbol's tag" that exists. Two
        // words make it a write; one word stays the jump-to-tag it always was.
        let words: Vec<&str> = arg.split_whitespace().collect();
        if words.len() == 2 {
            return Some(self.retag_by_tag(i, words[0], words[1]));
        }
        if words.len() > 2 {
            self.command_line.push_error(&format!(
                "PIDTAG: 改位号只接受两个参数（旧位号 新位号），实得 \"{arg}\"。"
            ));
            return Some(Task::none());
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
            self.pid_groups_select(
                i,
                |s| symbol_has_any(s, &picked),
                Some(Asked::Picked(&picked)),
            )
        } else {
            let wanted = arg.to_uppercase();
            let found = self.pid_groups_select(
                i,
                |s| s.tag.as_deref().is_some_and(|t| t.to_uppercase() == wanted),
                Some(Asked::Tag(&wanted)),
            );
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

    /// `PIDTAG <old> <new>`: rename every symbol carrying `old` to `new`.
    ///
    /// The rename is the same write the properties panel's `pid_tag` row makes
    /// (`PidAuthor.tag` + `tag_source = manual`, then the uniqueness pass and
    /// the label refresh), so the two entry points cannot drift. It is one undo
    /// step, which is also what makes it one broadcast op
    /// (`push_undo_entry` → `broadcast_history_delta`).
    ///
    /// A symbol a `.pid` import placed carries the tag the plant's data
    /// gives it and is not renamed here (SPEC-pid-symbol-identity §4.7 and
    /// §8). Where such symbols alone carry `old` the rename is refused,
    /// coded `tag_imported`; where objects drawn here carry it too, those
    /// are renamed and the receipt says the imported ones were not.
    fn retag_by_tag(&mut self, i: usize, old: &str, new: &str) -> Task<Message> {
        use crate::pid::persist::PidAuthor;
        let old_upper = old.to_uppercase();
        let targets: Vec<document::Handle> = self.tabs[i]
            .scene
            .document
            .entities()
            .filter_map(|entity| {
                let handle = entity.common().handle;
                let author = PidAuthor::load(&self.tabs[i].scene.document, handle)?;
                (author.tag.to_uppercase() == old_upper).then_some(handle)
            })
            .collect();
        let imported = imported_groups(&self.tabs[i].scene.document, &Asked::Tag(&old_upper)).len();
        if targets.is_empty() {
            if imported > 0 {
                self.command_line.push_error_coded(
                    "tag_imported",
                    &format!(
                        "PIDTAG: 本图上带位号 \"{old}\" 的是 {imported} 个导入的图例，导入的图例不能在这里改位号，未做改动。"
                    ),
                );
                return Task::none();
            }
            self.command_line.push_error_coded(
                "tag_absent",
                &format!("PIDTAG: 本图上没有位号 \"{old}\" 的符号，未做改动。"),
            );
            return Task::none();
        }
        self.push_undo_snapshot(i, "PIDTAG");
        {
            let document = &mut self.tabs[i].scene.document;
            for handle in &targets {
                let Some(mut author) = PidAuthor::load(document, *handle) else {
                    continue;
                };
                author.tag = new.to_owned();
                author.tag_source = crate::pid::tag_engine::TAG_SOURCE_MANUAL.to_owned();
                author.store(document, *handle);
            }
        }
        self.refresh_pid_tags(i, &targets);
        self.tabs[i].dirty = true;
        let kept = if imported > 0 {
            format!("；另有 {imported} 个导入的图例带这个位号，导入的图例不能在这里改位号，没有改")
        } else {
            String::new()
        };
        self.command_line.push_output(&format!(
            "PIDTAG: 位号 {old} → {new}，{} 个符号已改{kept}。",
            targets.len()
        ));
        Task::none()
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
        self.pid_groups_select(i, pick, None)
    }

    /// What [`Self::pid_group_select`] does, and where `asked` is given, of
    /// the symbols a `.pid` import placed that it admits as well
    /// ([`imported_groups`]): the group of such a symbol is its block
    /// reference with the lettering of its labels. With one of those among
    /// the groups the receipt counts symbols and labels -- the labels being
    /// every group's lettering, a recognised symbol's tag included --
    /// instead of groups and entities; without one it reads as it did.
    fn pid_groups_select(
        &mut self,
        i: usize,
        pick: impl Fn(&pid_legend::Recognized) -> bool,
        asked: Option<Asked<'_>>,
    ) -> Option<String> {
        self.refresh_pid_legend(i);
        let (groups, names, upm, labels) = {
            let rec = self.tabs[i].pid_legend()?;
            let mut groups: Vec<Vec<document::Handle>> = Vec::new();
            let mut names: Vec<String> = Vec::new();
            let mut lettering: rustc_hash::FxHashSet<document::Handle> =
                rustc_hash::FxHashSet::default();
            for symbol in rec.symbols.iter().filter(|s| pick(s)) {
                let handles: Vec<document::Handle> = symbol
                    .handles
                    .iter()
                    .chain(&symbol.tag_handles)
                    .copied()
                    .filter(|h| !h.is_null())
                    .collect();
                if handles.is_empty() {
                    continue;
                }
                lettering.extend(symbol.tag_handles.iter().filter(|h| !h.is_null()));
                groups.push(handles);
                names.push(match &symbol.tag {
                    Some(tag) => format!("{} {tag}", symbol.label),
                    None => format!("{} (无位号)", symbol.label),
                });
            }
            let imported = match &asked {
                Some(asked) => imported_groups(&self.tabs[i].scene.document, asked),
                None => Vec::new(),
            };
            let counted = !imported.is_empty();
            for group in imported {
                lettering.extend(&group.labels);
                let mut handles = vec![group.reference];
                handles.extend(group.labels);
                groups.push(handles);
                names.push(group.name);
            }
            (
                groups,
                names,
                rec.units_per_mm,
                counted.then_some(lettering.len()),
            )
        };
        if groups.is_empty() {
            return None;
        }
        let selected: rustc_hash::FxHashSet<document::Handle> =
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
        Some(match labels {
            Some(labels) => format!(
                "PIDTAG: {what} -- {} symbol(s), {labels} label(s) selected and framed in red.",
                groups.len()
            ),
            None => format!(
                "PIDTAG: {what} -- {} group(s), {count} entities selected and framed in red.",
                groups.len()
            ),
        })
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

/// What PIDTAG asks for among the symbols a `.pid` import placed.
enum Asked<'a> {
    /// Those that carry this tag: upper-cased, the way tags are compared.
    Tag(&'a str),
    /// Those the picked entities are of, by handle value: a symbol's block
    /// reference, or the lettering of one of its labels.
    Picked(&'a [u64]),
}

/// A symbol a `.pid` import placed, as PIDTAG selects it.
struct ImportedGroup {
    /// The symbol's block reference.
    reference: document::Handle,
    /// The lettering of its labels, in the order the sheet files it.
    labels: Vec<document::Handle>,
    /// What the receipt calls the symbol: the name of its symbol -- what
    /// kind of thing it is, where it states no name -- and its tag.
    name: String,
}

/// The symbols a `.pid` import placed in model space that `asked` admits,
/// in the order the sheet files them, each with the lettering of its
/// labels.
///
/// A symbol is a block reference whose `PID_SEMANTICS` record says it is
/// one, and its labels are the texts that name the number of its graphic
/// as a symbol's (`crate::io::pid_identity`): what the P&ID editor's
/// inspector shows of the two. An object drawn in this program
/// (`PID_AUTHOR`) is none of these symbols whatever else its record
/// states, as it is none to the inspector. A copy of a symbol states its
/// original's number, so the two share their labels, and the lettering of
/// one of those picks the first of the two the sheet files -- the symbol
/// the Properties panel names as the label's
/// (`pid_identity::label_owner`), and no symbol where that one was drawn
/// here.
///
/// The sheet is gone through once, however much is asked for: the picked
/// entities may be the whole drawing.
fn imported_groups(document: &document::Document, asked: &Asked<'_>) -> Vec<ImportedGroup> {
    use crate::io::pid_identity::{self, LabelOn};
    use crate::pid::persist::PidAuthor;
    use rustc_hash::{FxHashMap, FxHashSet};

    let picked: FxHashSet<u64> = match asked {
        Asked::Picked(handles) => handles.iter().copied().collect(),
        Asked::Tag(_) => FxHashSet::default(),
    };
    let mut symbols = Vec::new();
    // The lettering of the labels, under the number of the graphic each
    // label hangs on; and the numbers the picked lettering names.
    let mut lettering: FxHashMap<&str, Vec<document::Handle>> = FxHashMap::default();
    let mut named: FxHashSet<&str> = FxHashSet::default();
    for entity in document.model_space_entities() {
        let handle = entity.common().handle;
        if let Some(symbol) = pid_identity::symbol_identity(entity) {
            symbols.push((handle, symbol, PidAuthor::from_entity(entity).is_some()));
            continue;
        }
        if pid_identity::lettering(entity).is_none() {
            continue;
        }
        let Some(pairing) = pid_identity::label_pairing(entity) else {
            continue;
        };
        if pairing.on != LabelOn::Symbol {
            continue;
        }
        lettering.entry(pairing.graphic).or_default().push(handle);
        if picked.contains(&handle.value()) {
            named.insert(pairing.graphic);
        }
    }

    let mut numbered: FxHashSet<&str> = FxHashSet::default();
    symbols
        .into_iter()
        .filter(|(handle, symbol, authored)| {
            // Whether this is the first symbol the sheet files under the
            // number of its graphic, drawn here or not: the one a label of
            // that number is a label of.
            let first = symbol
                .graphic
                .is_some_and(|graphic| numbered.insert(graphic));
            if *authored {
                return false;
            }
            match asked {
                Asked::Tag(wanted) => symbol.tag.is_some_and(|tag| tag.to_uppercase() == *wanted),
                Asked::Picked(_) => {
                    picked.contains(&handle.value())
                        || (first
                            && symbol
                                .graphic
                                .is_some_and(|graphic| named.contains(graphic)))
                }
            }
        })
        .map(|(reference, symbol, _)| {
            let what = symbol.symbol.or_else(|| symbol.kind()).unwrap_or("图例");
            ImportedGroup {
                reference,
                labels: symbol
                    .graphic
                    .and_then(|graphic| lettering.get(graphic))
                    .cloned()
                    .unwrap_or_default(),
                name: match symbol.tag {
                    Some(tag) => format!("{what} {tag}"),
                    None => format!("{what} (无位号)"),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::pid_symbol_block_tests::{lettered, opened_with, reference, restate, VALVE};
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
        for verb in ["PIDLEGEND", "PIDLINE", "PIDTAG", "PIDGROUP"] {
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
    fn valve_with_tag(app: &mut OpenCADStudio) -> (document::Handle, document::Handle, document::Handle) {
        use document::entities::{EntityType, Line, Text};
        use document::types::{Transform, Vector3};

        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let line = |a: (f64, f64), b: (f64, f64)| {
            let mut line = Line::new();
            line.start = Vector3::new(a.0, a.1, 0.0);
            line.end = Vector3::new(b.0, b.1, 0.0);
            EntityType::Line(line)
        };
        let body: Vec<document::Handle> = [
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

    fn selected(app: &OpenCADStudio) -> Vec<document::Handle> {
        let mut handles = app.tabs[app.active_tab].scene.selected_handles_in_order();
        handles.sort_by_key(|h| h.value());
        handles
    }

    fn pid_xdata_strings(app: &OpenCADStudio, handle: document::Handle) -> Vec<String> {
        app.tabs[app.active_tab]
            .scene
            .document
            .get_entity(handle)
            .and_then(|entity| {
                entity
                    .common()
                    .extended_data
                    .get_record(crate::io::PID_SEMANTICS_XDATA_APP)
            })
            .map(|record| {
                record
                    .values
                    .iter()
                    .filter_map(|value| match value {
                        document::xdata::XDataValue::String(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A bow-tie valve of four loose lines at `x`, with `tag` lettered above
    /// it when given. Returns the strokes and the tag's handle.
    fn loose_valve(
        app: &mut OpenCADStudio,
        x: f64,
        tag: Option<&str>,
    ) -> (Vec<document::Handle>, Option<document::Handle>) {
        use document::entities::{EntityType, Line, Text};
        use document::types::Vector3;
        let i = app.active_tab;
        let strokes: Vec<document::Handle> = [
            ((0.0, 0.0), (0.0, 3.0)),
            ((0.0, 3.0), (6.0, 0.0)),
            ((6.0, 0.0), (6.0, 3.0)),
            ((6.0, 3.0), (0.0, 0.0)),
        ]
        .into_iter()
        .map(|(a, b)| {
            let mut line = Line::new();
            line.start = Vector3::new(x + a.0, a.1, 0.0);
            line.end = Vector3::new(x + b.0, b.1, 0.0);
            app.tabs[i].scene.add_entity(EntityType::Line(line))
        })
        .collect();
        let tag = tag.map(|value| {
            app.tabs[i].scene.add_entity(EntityType::Text(
                Text::with_value(value, Vector3::new(x, 4.0, 0.0)).with_height(2.5),
            ))
        });
        (strokes, tag)
    }

    fn select_all_of(app: &mut OpenCADStudio, handles: &[document::Handle]) {
        let i = app.active_tab;
        app.tabs[i]
            .scene
            .replace_selection(handles.iter().copied().collect());
    }

    /// The groups of the active drawing, by handle, in handle order.
    fn groups(app: &OpenCADStudio) -> Vec<document::Handle> {
        let mut out: Vec<document::Handle> = app.tabs[app.active_tab]
            .scene
            .groups()
            .map(|g| g.handle)
            .collect();
        out.sort_by_key(|h| h.value());
        out
    }

    fn description_of(app: &OpenCADStudio, group: document::Handle) -> String {
        app.tabs[app.active_tab]
            .scene
            .groups()
            .find(|g| g.handle == group)
            .map(|g| g.description.clone())
            .unwrap_or_else(|| panic!("no group {group:?}"))
    }

    fn property_value(app: &OpenCADStudio, field: &str) -> Option<String> {
        use crate::scene::model::object::PropValue;
        app.tabs[app.active_tab]
            .properties
            .sections
            .iter()
            .flat_map(|section| &section.props)
            .find(|property| property.field == field)
            .and_then(|property| match &property.value {
                PropValue::EditText(value)
                | PropValue::PlainText(value)
                | PropValue::ReadOnly(value) => Some(value.clone()),
                PropValue::ReadOnlyWithTooltip { value, .. } => Some(value.clone()),
                _ => None,
            })
    }

    /// Every line the command line has shown, newest last.
    fn history(app: &OpenCADStudio) -> Vec<String> {
        app.command_line
            .history
            .iter()
            .map(|entry| entry.text.clone())
            .collect()
    }

    /// GROUP reads the new group's tag from its own lettering and writes it
    /// into the description; a group that letters nothing stays a plain
    /// group and the receipt says so. Both undo and redo as one step each.
    #[test]
    fn group_reads_its_tag_from_the_lettering_into_the_description() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let (strokes, tag) = loose_valve(&mut app, 0.0, Some("BUV-3101"));
        let (bare, _) = loose_valve(&mut app, 40.0, None);

        let mut members = strokes.clone();
        members.push(tag.unwrap());
        select_all_of(&mut app, &members);
        let _ = app.run_command_line("GROUP");
        assert!(
            app.tabs[app.active_tab].active_cmd.is_some(),
            "GROUP asks a name"
        );
        let _ = app.feed_command(crate::command::StepInput::Enter);
        let tagged = groups(&app);
        assert_eq!(tagged.len(), 1);
        assert_eq!(
            description_of(&app, tagged[0]),
            "tagName=BUV-3101;tagSource=auto"
        );
        // The first half of the receipt is the localised GROUP line.
        let receipt = last_line(&app);
        assert!(
            receipt.contains("*A1") && receipt.contains("tagName BUV-3101 (蝶阀)."),
            "{receipt}"
        );

        select_all_of(&mut app, &bare);
        let _ = app.run_command_line("GROUP");
        let _ = app.feed_command(crate::command::StepInput::Enter);
        let all = groups(&app);
        assert_eq!(all.len(), 2);
        let plain = *all.iter().find(|g| **g != tagged[0]).unwrap();
        assert_eq!(description_of(&app, plain), "", "no lettering, no marker");
        let receipt = last_line(&app);
        assert!(
            receipt.contains("No tagName") && receipt.contains("0 texts"),
            "{receipt}"
        );
        assert_eq!(app.tabs[app.active_tab].scene.tagged_groups().count(), 1);

        // One undo step per GROUP, description and all.
        let _ = app.update(Message::Undo);
        assert_eq!(groups(&app), tagged);
        let _ = app.update(Message::Undo);
        assert!(groups(&app).is_empty());
        let _ = app.update(Message::Redo);
        assert_eq!(groups(&app), tagged);
        assert_eq!(
            description_of(&app, tagged[0]),
            "tagName=BUV-3101;tagSource=auto",
            "redo brings the tag back with the group"
        );
    }

    /// With a current P&ID index, GROUP offers the selected symbol's unique
    /// tag as its readable dictionary name. A stale index or an occupied name
    /// falls back to the ordinary anonymous sequence.
    #[test]
    fn group_uses_a_unique_indexed_tag_as_its_default_name() {
        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, _pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        let members = [insert, tag];
        select_all_of(&mut app, &members);
        let _ = app.run_command_line("PIDLEGEND REPORT");
        assert_eq!(app.suggested_pid_group_name(i, &members), "BUV-3101");

        app.tabs[i].scene.bump_geometry();
        assert_eq!(
            app.suggested_pid_group_name(i, &members),
            "*A1",
            "a stale index is never used for naming"
        );
        let _ = app.run_command_line("PIDLEGEND REPORT");
        select_all_of(&mut app, &members);
        let _ = app.run_command_line("GROUP");
        let prompt = app.tabs[i]
            .active_cmd
            .as_ref()
            .map(|command| command.prompt())
            .unwrap_or_default();
        assert!(prompt.contains("BUV-3101"), "{prompt}");
        let _ = app.feed_command(crate::command::StepInput::Enter);
        let group = app.tabs[i].scene.groups().next().expect("group created");
        assert_eq!(group.name, "BUV-3101");
        assert_eq!(group.description, "tagName=BUV-3101;tagSource=auto");

        let _ = app.run_command_line("PIDLEGEND REPORT");
        assert_eq!(
            app.suggested_pid_group_name(i, &members),
            "*A1",
            "an existing dictionary name is not reused"
        );
    }

    /// PIDGROUP groups without asking a name, sets a tag by hand (`manual`),
    /// hands it back to the lettering (`auto`), and treats a hand-set value
    /// the lettering reads anyway as `auto`; on an ungrouped selection TAG
    /// groups first; OFF dissolves. Bare PIDGROUP only prompts.
    #[test]
    fn pidgroup_groups_tags_by_hand_reads_again_and_dissolves() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let (strokes, tag) = loose_valve(&mut app, 0.0, Some("BUV-3101"));
        let (bare, _) = loose_valve(&mut app, 40.0, None);

        let mut members = strokes.clone();
        members.push(tag.unwrap());
        select_all_of(&mut app, &members);
        let _ = app.run_command_line("PIDGROUP GROUP");
        assert!(app.tabs[i].active_cmd.is_none(), "no name prompt");
        let first = groups(&app);
        assert_eq!(first.len(), 1);
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=BUV-3101;tagSource=auto"
        );
        assert!(last_line(&app).contains("tagName BUV-3101 (蝶阀)"));

        // By hand, through one member.
        select_all_of(&mut app, &[tag.unwrap()]);
        let _ = app.run_command_line("PIDGROUP TAG XV-0001");
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=XV-0001;tagSource=manual"
        );
        assert!(
            last_line(&app).contains("set by hand"),
            "{}",
            last_line(&app)
        );
        // The value the lettering reads anyway is not a hand-set one.
        let _ = app.run_command_line("PIDGROUP TAG BUV-3101");
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=BUV-3101;tagSource=auto"
        );
        // Back to the lettering from a hand-set value.
        let _ = app.run_command_line("PIDGROUP TAG XV-0002");
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=XV-0002;tagSource=manual"
        );
        let _ = app.run_command_line("PIDGROUP AUTO");
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=BUV-3101;tagSource=auto"
        );
        assert!(last_line(&app).contains("read from the lettering"));
        // Undo the last two by-hand steps: one step each.
        let _ = app.update(Message::Undo);
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=XV-0002;tagSource=manual"
        );
        let _ = app.update(Message::Redo);
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=BUV-3101;tagSource=auto"
        );

        // TAG without a value: typed inline, the verb prompt asks for the tag
        // and Enter cancels; dispatched whole, it is refused. The group is
        // left alone either way.
        let _ = app.run_command_line("PIDGROUP TAG");
        assert!(
            app.tabs[i].active_cmd.is_none(),
            "Enter at the tag prompt cancelled"
        );
        let _ = app.dispatch_command("PIDGROUP TAG");
        assert!(
            last_line(&app).contains("needs the tag"),
            "{}",
            last_line(&app)
        );
        assert_eq!(
            description_of(&app, first[0]),
            "tagName=BUV-3101;tagSource=auto"
        );

        // An ungrouped selection tagged by hand is grouped first.
        select_all_of(&mut app, &bare);
        let _ = app.run_command_line("PIDGROUP TAG GV0301");
        let all = groups(&app);
        assert_eq!(all.len(), 2);
        let second = *all.iter().find(|g| **g != first[0]).unwrap();
        assert_eq!(
            description_of(&app, second),
            "tagName=GV0301;tagSource=manual"
        );
        assert!(
            last_line(&app).contains("set by hand"),
            "{}",
            last_line(&app)
        );
        // Nothing to read from: AUTO leaves it marked, tagless.
        select_all_of(&mut app, &bare[..1]);
        let _ = app.run_command_line("PIDGROUP AUTO");
        assert_eq!(description_of(&app, second), "tagName=;tagSource=auto");

        // OFF dissolves the picked member's group only (the receipt is the
        // localised UNGROUP line).
        let _ = app.run_command_line("PIDGROUP OFF");
        assert_eq!(groups(&app), first);
        assert!(app.tabs[i].active_cmd.is_none());

        // Bare PIDGROUP prompts; Enter with nothing selected goes on to a
        // selection prompt rather than making an empty group.
        app.tabs[i].scene.deselect_all();
        let _ = app.run_command_line("PIDGROUP");
        assert!(app.tabs[i].active_cmd.is_some(), "bare PIDGROUP prompts");
        let _ = app.feed_command(crate::command::StepInput::Enter);
        assert!(
            app.tabs[i].active_cmd.is_some(),
            "GROUP with nothing selected asks for objects"
        );
        let _ = app.feed_command(crate::command::StepInput::Escape);
        assert_eq!(groups(&app), first, "nothing was made");
    }

    /// Selecting any member exposes the group's P&ID identity in Properties.
    /// Editing `pid_tag` uses the same auto/manual rule as PIDGROUP, is one
    /// undo step, and dates the recognition index in both directions.
    #[test]
    fn properties_edits_a_group_tag_and_undo_restores_it() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let (strokes, tag) = loose_valve(&mut app, 0.0, Some("BUV-3101"));
        let tag = tag.unwrap();
        let mut members = strokes;
        members.push(tag);
        select_all_of(&mut app, &members);
        let _ = app.run_command_line("PIDGROUP GROUP");
        let group = groups(&app)[0];

        let _ = app.run_command_line("PIDLEGEND LIST");
        assert!(!app.tabs[i].pid_legend_is_stale());
        select_all_of(&mut app, &[tag]);
        app.refresh_properties();
        assert_eq!(property_value(&app, "pid_tag").as_deref(), Some("BUV-3101"));
        assert_eq!(
            property_value(&app, "pid_tag_source").as_deref(),
            Some(crate::t!("Automatic").as_ref())
        );
        assert_eq!(property_value(&app, "pid_group").as_deref(), Some("*A1"));
        assert!(
            property_value(&app, "pid_group_class")
                .is_some_and(|value| value.contains("butterfly")),
            "{:?}",
            property_value(&app, "pid_group_class")
        );

        let _ = app.update(Message::PropGeomInput {
            field: "pid_tag",
            value: "XV-0001".to_string(),
        });
        let _ = app.update(Message::PropGeomCommit("pid_tag"));
        assert_eq!(
            description_of(&app, group),
            "tagName=XV-0001;tagSource=manual"
        );
        assert!(app.tabs[i].pid_legend_is_stale());
        assert_eq!(
            property_value(&app, "pid_tag_source").as_deref(),
            Some(crate::t!("Manual").as_ref())
        );
        let _ = app.run_command_line("PIDTAG XV-0001");
        assert_eq!(selected(&app), {
            let mut expected = members.clone();
            expected.sort_by_key(|handle| handle.value());
            expected
        });

        // Clearing the field hands the tag back to the lettering.
        select_all_of(&mut app, &[tag]);
        app.refresh_properties();
        let _ = app.update(Message::PropGeomInput {
            field: "pid_tag",
            value: String::new(),
        });
        let _ = app.update(Message::PropGeomCommit("pid_tag"));
        assert_eq!(
            description_of(&app, group),
            "tagName=BUV-3101;tagSource=auto"
        );
        let _ = app.run_command_line("PIDTAG BUV-3101");
        assert!(!app.tabs[i].pid_legend_is_stale());

        let _ = app.update(Message::Undo);
        assert_eq!(
            description_of(&app, group),
            "tagName=XV-0001;tagSource=manual"
        );
        assert!(
            app.tabs[i].pid_legend_is_stale(),
            "object-only group undo dates the P&ID index"
        );
        let _ = app.run_command_line("PIDTAG XV-0001");
        assert_eq!(selected(&app).len(), members.len());
        let _ = app.update(Message::Redo);
        assert_eq!(
            description_of(&app, group),
            "tagName=BUV-3101;tagSource=auto"
        );

        let (second_strokes, second_tag) = loose_valve(&mut app, 40.0, Some("BUV-3102"));
        let second_tag = second_tag.unwrap();
        let mut second_members = second_strokes;
        second_members.push(second_tag);
        select_all_of(&mut app, &second_members);
        let _ = app.run_command_line("PIDGROUP GROUP");
        let second_group = *groups(&app)
            .iter()
            .find(|candidate| **candidate != group)
            .unwrap();
        select_all_of(&mut app, &[tag, second_tag]);
        app.refresh_properties();
        assert_eq!(
            property_value(&app, "pid_tag").as_deref(),
            Some(crate::app::VARIES_LABEL)
        );
        assert_eq!(
            property_value(&app, "pid_group").as_deref(),
            Some(crate::app::VARIES_LABEL)
        );
        assert!(
            property_value(&app, "pid_group_class")
                .is_some_and(|value| value.contains("butterfly")),
            "the common type remains visible across two groups"
        );
        let _ = app.update(Message::PropGeomInput {
            field: "pid_tag",
            value: "XV-9000".to_string(),
        });
        let _ = app.update(Message::PropGeomCommit("pid_tag"));
        assert_eq!(
            description_of(&app, group),
            "tagName=XV-9000;tagSource=manual"
        );
        assert_eq!(
            description_of(&app, second_group),
            "tagName=XV-9000;tagSource=manual"
        );
    }

    /// A copy of a tagged group carries the tag (the description is copied
    /// with the group, in the drawing and through the clipboard); when the
    /// copy's lettering is then changed, PIDLEGEND ON brings the stored
    /// `auto` tag up to date (D35). A drawn legend is redrawn when a group
    /// is made or dissolved, in the same undo step (D38).
    #[test]
    fn a_copied_group_keeps_its_tag_and_the_legend_follows_group_changes() {
        use crate::command::EntityTransform;
        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, _pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        select_all_of(&mut app, &[insert, tag]);
        let _ = app.run_command_line("PIDGROUP GROUP");
        let original = groups(&app);
        assert_eq!(original.len(), 1);
        assert_eq!(
            description_of(&app, original[0]),
            "tagName=BUV-3101;tagSource=auto"
        );

        // An in-drawing copy of the whole group: a second group, same tag.
        let copied = app.tabs[i].scene.copy_entities(
            &[insert, tag],
            &EntityTransform::Translate(glam::DVec3::new(60.0, 0.0, 0.0)),
        );
        assert_eq!(copied.len(), 2);
        let all = groups(&app);
        assert_eq!(all.len(), 2, "the copy is grouped too");
        let copy = *all.iter().find(|g| **g != original[0]).unwrap();
        assert_eq!(
            description_of(&app, copy),
            "tagName=BUV-3101;tagSource=auto"
        );

        // Letter the copy differently; the stored tag lags until the sheet is
        // marked up again, then follows.
        let copied_tag = *copied
            .iter()
            .find(|h| {
                matches!(
                    app.tabs[i].scene.document.get_entity(**h),
                    Some(document::EntityType::Text(_))
                )
            })
            .expect("the copied tag");
        if let Some(document::EntityType::Text(text)) =
            app.tabs[i].scene.document.get_entity_mut(copied_tag)
        {
            text.value = "BUV-3102".to_string();
        }
        assert_eq!(
            description_of(&app, copy),
            "tagName=BUV-3101;tagSource=auto"
        );
        let _ = app.run_command_line("PIDLEGEND ON");
        assert_eq!(
            description_of(&app, copy),
            "tagName=BUV-3102;tagSource=auto"
        );
        assert_eq!(
            description_of(&app, original[0]),
            "tagName=BUV-3101;tagSource=auto",
            "the original is as it was"
        );
        let drawn = legend_count(&app);
        assert!(drawn > 0, "PIDLEGEND ON drew the legend");

        // With a legend drawn, dissolving and making a group redraws it in
        // the same step: one undo takes both back.
        let before = history(&app).len();
        select_all_of(&mut app, &[copied_tag]);
        let _ = app.run_command_line("UNGROUP");
        assert_eq!(groups(&app), original);
        let lines = history(&app)[before..].join("\n");
        assert!(lines.contains("legend redrawn"), "{lines}");
        assert_eq!(
            legend_count(&app),
            drawn,
            "the same sheet draws the same legend"
        );
        let _ = app.update(Message::Undo);
        assert_eq!(groups(&app).len(), 2, "one undo brings the group back");
        assert_eq!(legend_count(&app), drawn, "and the legend is whole");
        let _ = app.update(Message::Redo);
        assert_eq!(groups(&app), original);

        // Through the clipboard into another drawing: the tag travels.
        select_all_of(&mut app, &[insert, tag]);
        let _ = app.run_command_line("COPYCLIP");
        assert_eq!(
            app.clipboard.len(),
            2,
            "the block and its tag are on the clipboard"
        );
        app.automation_op(r#"{"op":"new"}"#);
        assert!(groups(&app).is_empty());
        let _ = app.run_command_line("PASTECLIP 0,0");
        let pasted = groups(&app);
        assert_eq!(pasted.len(), 1, "the pasted group came along");
        assert_eq!(
            description_of(&app, pasted[0]),
            "tagName=BUV-3101;tagSource=auto"
        );
    }

    /// The command line's latest entry.
    fn last_line(app: &OpenCADStudio) -> String {
        app.command_line
            .history
            .last()
            .map(|entry| entry.text.clone())
            .unwrap_or_default()
    }

    /// `PIDTAG <old> <new>` renames the tag. The collaboration contract states
    /// a tag change with exactly this line (`VAL-CLI-009`), so it must be a
    /// real write: the stored tag, its source, its label, and one undo step.
    #[test]
    fn a_two_word_pidtag_renames_the_tag_in_one_undo_step() {
        use crate::pid::persist::PidAuthor;

        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let handle = {
            let document = &mut app.tabs[i].scene.document;
            let handle = document
                .add_entity(document::EntityType::Insert(document::entities::Insert::new(
                    "PID_SYM_equipment_vessel",
                    document::types::Vector3::ZERO,
                )))
                .expect("entity");
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1001".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, handle);
            handle
        };

        let undone_before = app.tabs[i].history.undo_stack.len();
        let _ = app.run_command_line("PIDTAG P-1001 P-1009");
        app.finish_all_pending_history();

        let author = PidAuthor::load(&app.tabs[i].scene.document, handle).expect("author");
        assert_eq!(author.tag, "P-1009");
        assert_eq!(author.tag_source, "manual");
        assert!(
            app.tabs[i].history.undo_stack.len() > undone_before,
            "改位号必须是一个可撤销步"
        );
        assert!(last_line(&app).contains("P-1009"), "{}", last_line(&app));

        let _ = app.automation_op(r#"{"op":"undo"}"#);
        let back = PidAuthor::load(&app.tabs[i].scene.document, handle).expect("author");
        assert_eq!((back.tag.as_str(), back.tag_source.as_str()), ("P-1001", "auto"));
    }

    /// A tag nobody carries is refused with a code, and nothing is written.
    #[test]
    fn a_rename_of_an_absent_tag_is_refused_by_code() {
        use crate::pid::persist::PidAuthor;

        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let handle = {
            let document = &mut app.tabs[i].scene.document;
            let handle = document
                .add_entity(document::EntityType::Insert(document::entities::Insert::new(
                    "PID_SYM_equipment_vessel",
                    document::types::Vector3::ZERO,
                )))
                .expect("entity");
            PidAuthor {
                type_code: "EQUIPMENT".to_owned(),
                tag: "P-1001".to_owned(),
                tag_source: "auto".to_owned(),
                tag_status: "ok".to_owned(),
                ..PidAuthor::default()
            }
            .store(document, handle);
            handle
        };
        let _ = app.run_command_line("PIDTAG P-7777 P-1009");
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("tag_absent")
        );
        let author = PidAuthor::load(&app.tabs[i].scene.document, handle).expect("author");
        assert_eq!(author.tag, "P-1001");
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

    // ── PIDTAG, of the symbols a `.pid` import placed ──────────────────────

    /// The program with the sheet of a tagged valve and its labels open, as
    /// a `.pid` import leaves it
    /// (`io::pid::sheet_of_a_tagged_valve_and_its_labels`). In millimetres:
    /// a valve the file names `GV-101` at (100, 100), with four labels; the
    /// same valve at (170, 100), with no name and one label; a run of line
    /// and its label; the label of what is no graphic of the sheet; and a
    /// text that is no label's.
    fn imported_sheet() -> OpenCADStudio {
        opened_with(crate::io::pid::sheet_of_a_tagged_valve_and_its_labels())
    }

    /// The drawing in front.
    fn sheet(app: &OpenCADStudio) -> &document::Document {
        &app.tabs[app.active_tab].scene.document
    }

    /// The valve of that sheet the file names `GV-101`, and the lettering
    /// of its four labels in the order the sheet files it.
    fn tagged_valve(app: &OpenCADStudio) -> (document::Handle, [document::Handle; 4]) {
        (
            reference(app, VALVE, (100.0, 100.0)),
            ["GV-101", "DN50", "PN16", "CS"].map(|lettering| lettered(app, lettering)),
        )
    }

    /// The valve of that sheet the file gives no name, and the lettering of
    /// its one label.
    fn untagged_valve(app: &OpenCADStudio) -> (document::Handle, document::Handle) {
        (reference(app, VALVE, (170.0, 100.0)), lettered(app, "DN80"))
    }

    /// The run of line of that sheet: the one entity of it that is neither
    /// a block reference nor a text.
    fn run_of_line(app: &OpenCADStudio) -> document::Handle {
        let found: Vec<document::Handle> = sheet(app)
            .model_space_entities()
            .filter(|entity| {
                !matches!(
                    entity,
                    document::EntityType::Insert(_) | document::EntityType::Text(_)
                )
            })
            .map(|entity| entity.common().handle)
            .collect();
        assert_eq!(found.len(), 1, "one run of line");
        found[0]
    }

    /// Handles in the order `selected` gives them.
    fn in_order(handles: impl IntoIterator<Item = document::Handle>) -> Vec<document::Handle> {
        let mut handles: Vec<document::Handle> = handles.into_iter().collect();
        handles.sort_by_key(|h| h.value());
        handles
    }

    /// The red frames PIDTAG drew, each by its extent -- left, bottom,
    /// right, top -- from left to right.
    fn frames(app: &OpenCADStudio) -> Vec<[f32; 4]> {
        let mut frames: Vec<[f32; 4]> = app.tabs[app.active_tab]
            .scene
            .pid_group_highlight_wires()
            .iter()
            .filter(|wire| wire.name == crate::scene::PID_GROUP_FRAME)
            .map(|wire| {
                let along = |axis: usize| wire.points.iter().map(move |point| point[axis]);
                [
                    along(0).fold(f32::MAX, f32::min),
                    along(1).fold(f32::MAX, f32::min),
                    along(0).fold(f32::MIN, f32::max),
                    along(1).fold(f32::MIN, f32::max),
                ]
            })
            .collect();
        frames.sort_by(|a, b| a[0].total_cmp(&b[0]));
        frames
    }

    /// What the command line last refused, in the words it was refused in.
    fn refused(app: &OpenCADStudio) -> &str {
        app.command_line.last_error.as_deref().unwrap_or_default()
    }

    /// An object drawn in this program that carries `tag`: a block
    /// reference with a `PID_AUTHOR` record, as PIDINSERT leaves one.
    fn authored(app: &mut OpenCADStudio, tag: &str) -> document::Handle {
        use crate::pid::persist::PidAuthor;

        let i = app.active_tab;
        let document = &mut app.tabs[i].scene.document;
        let handle = document
            .add_entity(document::EntityType::Insert(
                document::entities::Insert::new(
                    "PID_SYM_equipment_vessel",
                    document::types::Vector3::ZERO,
                ),
            ))
            .expect("entity");
        PidAuthor {
            type_code: "EQUIPMENT".to_owned(),
            tag: tag.to_owned(),
            tag_source: "auto".to_owned(),
            tag_status: "ok".to_owned(),
            ..PidAuthor::default()
        }
        .store(document, handle);
        handle
    }

    /// PIDTAG <tag> finds a symbol a `.pid` import placed by the tag its
    /// record states (plan 2026-11-20-pid-symbol-identity, SI-5; SPEC
    /// §4.7): the symbol's block reference and the lettering of all its
    /// labels are selected, framed in red and zoomed to, and the receipt
    /// counts the symbols and the labels. The other valve and its label,
    /// the run of line and its label are none of it.
    #[test]
    fn pidtag_finds_an_imported_symbol_by_its_tag_with_all_its_labels() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, labels) = tagged_valve(&app);
        let camera_before = app.tabs[i].scene.camera_generation;

        let _ = app.run_command_line("PIDTAG gv-101");
        assert_eq!(
            selected(&app),
            in_order(labels.into_iter().chain([valve])),
            "the valve and its four labels, case aside"
        );
        assert_eq!(
            last_line(&app),
            "PIDTAG: Ball Valve GV-101 -- 1 symbol(s), 4 label(s) selected and framed in red."
        );
        assert!(
            app.tabs[i].scene.camera_generation > camera_before,
            "the view moved to the group"
        );
        // One frame, a millimetre outside the valve (80..120 x 90..110) and
        // its labels: lettered from x = 85, the topmost standing on y = 150
        // and 2.5 mm high (the glyph quads carry a little padding past the
        // ink).
        let frames = frames(&app);
        assert_eq!(frames.len(), 1, "one frame for one symbol: {frames:?}");
        let [left, bottom, right, top] = frames[0];
        assert!((left - 79.0).abs() < 0.05, "left {left}");
        assert!((right - 121.0).abs() < 0.05, "right {right}");
        assert!(bottom > 88.9 && bottom < 89.3, "bottom {bottom}");
        assert!(top > 153.4 && top < 155.5, "top {top}");
        assert_eq!(
            app.tabs[i].scene.selection_tint(),
            crate::scene::PID_GROUP_HIGHLIGHT,
            "the selection is drawn in the group's red"
        );
    }

    /// Several imported symbols that carry one tag are all selected, each
    /// with its labels and in a frame of its own.
    #[test]
    fn pidtag_selects_every_imported_symbol_that_carries_the_tag() {
        let mut app = imported_sheet();
        let (valve, labels) = tagged_valve(&app);
        let (other, its_label) = untagged_valve(&app);
        restate(
            &mut app,
            other,
            &[("label", "GV-101"), ("tag_source", "file")],
        );

        let _ = app.run_command_line("PIDTAG GV-101");
        assert_eq!(
            selected(&app),
            in_order(labels.into_iter().chain([valve, other, its_label]))
        );
        assert_eq!(
            last_line(&app),
            "PIDTAG: Ball Valve GV-101 + Ball Valve GV-101 -- 2 symbol(s), 5 label(s) selected and framed in red."
        );
        let drawn = frames(&app);
        assert_eq!(drawn.len(), 2, "a frame for each symbol: {drawn:?}");
        // The second valve is 150..190 x 90..110, and its label is lettered
        // from (160, 120).
        let [left, bottom, right, top] = drawn[1];
        assert!((left - 149.0).abs() < 0.05, "left {left}");
        assert!((right - 191.0).abs() < 0.05, "right {right}");
        assert!(bottom > 88.9 && bottom < 89.3, "bottom {bottom}");
        assert!(top > 123.4 && top < 125.5, "top {top}");

        // The second valve made a copy of the first, the two have the
        // first's labels: each of those is selected, and counted, once.
        restate(&mut app, other, &[("graphic", "612")]);
        let _ = app.run_command_line("PIDTAG GV-101");
        assert_eq!(
            selected(&app),
            in_order(labels.into_iter().chain([valve, other]))
        );
        assert_eq!(
            last_line(&app),
            "PIDTAG: Ball Valve GV-101 + Ball Valve GV-101 -- 2 symbol(s), 4 label(s) selected and framed in red."
        );
        assert_eq!(frames(&app).len(), 2, "a frame for each symbol");
    }

    /// A tag is compared case aside on both sides, as it was: the tag the
    /// record of an imported symbol states in small letters is found by its
    /// capitals, and the receipt spells it as the record does.
    #[test]
    fn pidtag_compares_the_tag_of_an_imported_symbol_case_aside() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let (other, its_label) = untagged_valve(&app);
        restate(&mut app, other, &[("label", "gv-102a")]);

        for typed in ["GV-102A", "gv-102a", "Gv-102A"] {
            app.tabs[i].scene.deselect_all();
            let _ = app.run_command_line(&format!("PIDTAG {typed}"));
            assert_eq!(selected(&app), in_order([other, its_label]), "{typed}");
            assert_eq!(
                last_line(&app),
                "PIDTAG: Ball Valve gv-102a -- 1 symbol(s), 1 label(s) selected and framed in red.",
                "{typed}"
            );
        }
    }

    /// A tag no symbol carries is refused in the words it was, on a sheet of
    /// imported symbols as on any other, and the selection is left alone.
    /// What a label reads is no tag unless its symbol's record states it,
    /// the label of a run of line is no symbol's tag, and a tag is matched
    /// whole.
    #[test]
    fn pidtag_refuses_a_tag_no_imported_symbol_carries_as_it_did() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let note = lettered(&app, "NOTE 4");
        app.tabs[i].scene.select_entity(note, true);

        for absent in [
            "XV-9999",
            "DN50",
            "DN80",
            "P-1001-50-CS",
            "GV-10",
            "GV-1011",
        ] {
            let _ = app.run_command_line(&format!("PIDTAG {absent}"));
            assert_eq!(
                refused(&app),
                format!(
                    "PIDTAG: no symbol tagged \"{absent}\" on this sheet -- PIDLEGEND LIST shows the tags."
                )
            );
            assert_eq!(selected(&app), vec![note], "{absent}: the pick stays");
            assert!(
                app.tabs[i].scene.pid_group_highlight_wires().is_empty(),
                "{absent}: nothing is framed"
            );
        }
    }

    /// PIDTAG with nothing named goes by the picked entities on a sheet of
    /// imported symbols too: the block reference of a symbol, or the
    /// lettering of any one of its labels, selects the symbol with all its
    /// labels -- a symbol that has no tag as well. A label of one symbol
    /// picked together with the block reference of another selects both.
    #[test]
    fn pidtag_reads_a_picked_imported_symbol_or_any_one_of_its_labels() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, labels) = tagged_valve(&app);
        let (other, its_label) = untagged_valve(&app);
        let tagged = in_order(labels.into_iter().chain([valve]));
        let untagged = in_order([other, its_label]);
        let of_the_tagged =
            "PIDTAG: Ball Valve GV-101 -- 1 symbol(s), 4 label(s) selected and framed in red.";
        let of_the_untagged =
            "PIDTAG: Ball Valve (无位号) -- 1 symbol(s), 1 label(s) selected and framed in red.";

        for (picked, group, receipt) in [
            (valve, &tagged, of_the_tagged),
            // Filed before its valve.
            (labels[0], &tagged, of_the_tagged),
            (labels[2], &tagged, of_the_tagged),
            (other, &untagged, of_the_untagged),
            (its_label, &untagged, of_the_untagged),
        ] {
            app.tabs[i].scene.select_entity(picked, true);
            let _ = app.run_command_line("PIDTAG ");
            assert_eq!(selected(&app), *group, "{picked:?} picked");
            assert_eq!(last_line(&app), receipt, "{picked:?} picked");
            assert_eq!(frames(&app).len(), 1, "{picked:?} picked");
        }

        app.tabs[i].scene.select_entity(labels[3], true);
        app.tabs[i].scene.select_entity(other, false);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(
            selected(&app),
            in_order(tagged.iter().chain(&untagged).copied())
        );
        assert_eq!(
            last_line(&app),
            "PIDTAG: Ball Valve GV-101 + Ball Valve (无位号) -- 2 symbol(s), 5 label(s) selected and framed in red."
        );
        assert_eq!(frames(&app).len(), 2, "a frame for each symbol");
    }

    /// What is picked and is no imported symbol's makes no group, and PIDTAG
    /// says so in the words it did and leaves the pick as it was: the label
    /// of a run of line, the run itself, the label of what is no graphic of
    /// the sheet, a text that is no label's -- and the label of a symbol
    /// that has been erased, until an undo gives the symbol back. With
    /// nothing picked it asks for a pick, as it did.
    #[test]
    fn pidtag_makes_no_group_of_a_pick_that_is_no_imported_symbols() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let no_group = "ⓘ PIDTAG: the picked entities belong to no recognised symbol.";

        for picked in [
            lettered(&app, "P-1001-50-CS"),
            run_of_line(&app),
            lettered(&app, "DWG-0201"),
            lettered(&app, "NOTE 4"),
        ] {
            app.tabs[i].scene.select_entity(picked, true);
            let _ = app.run_command_line("PIDTAG ");
            assert_eq!(selected(&app), vec![picked], "the pick stays as it was");
            assert_eq!(last_line(&app), no_group, "{picked:?} picked");
            assert!(
                app.tabs[i].scene.pid_group_highlight_wires().is_empty(),
                "{picked:?} picked: nothing is framed"
            );
        }

        let (other, its_label) = untagged_valve(&app);
        app.tabs[i].scene.select_entity(other, true);
        let _ = app.run_command_line("ERASE");
        assert!(
            !sheet(&app)
                .model_space_entities()
                .any(|entity| entity.common().handle == other),
            "the valve is erased"
        );
        app.tabs[i].scene.select_entity(its_label, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(selected(&app), vec![its_label], "the pick stays as it was");
        assert_eq!(last_line(&app), no_group, "the label of an erased symbol");

        let _ = app.automation_op(r#"{"op":"undo"}"#);
        app.tabs[i].scene.select_entity(its_label, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(
            selected(&app),
            in_order([other, its_label]),
            "the valve is back"
        );

        app.tabs[i].scene.deselect_all();
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(
            last_line(&app),
            "ⓘ PIDTAG: pick a symbol (or its tag lettering) first, or name the tag -- PIDTAG BUV-3201."
        );
        assert!(selected(&app).is_empty(), "nothing is selected");
    }

    /// A row of the legend list is a recognised symbol, and a click on one
    /// goes by the recognised symbols alone, as it did: handed the handle of
    /// an imported symbol it selects nothing and says what it said.
    #[test]
    fn a_legend_list_row_goes_by_the_recognised_symbols_alone() {
        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, _) = tagged_valve(&app);
        let note = lettered(&app, "NOTE 4");
        app.tabs[i].scene.select_entity(note, true);

        let _ = app.update(Message::PidLegendPickSymbol(vec![valve.value()]));
        assert_eq!(selected(&app), vec![note], "the pick stays as it was");
        assert_eq!(
            last_line(&app),
            "ⓘ PIDTAG: that symbol is no longer on the sheet -- PIDLEGEND LIST re-reads it."
        );
    }

    /// `PIDTAG <old> <new>` does not rename a symbol a `.pid` import placed
    /// (SPEC §4.7, §8): its tag is the plant's. Where imported symbols
    /// alone carry the old tag the rename is refused, by code and in words
    /// that say how many they are, and nothing is written: no record, no
    /// undo step, nothing to save. A tag nobody carries is absent, as it
    /// was.
    #[test]
    fn a_rename_is_refused_where_imported_symbols_alone_carry_the_tag() {
        use crate::pid::persist::PidAuthor;

        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, _) = tagged_valve(&app);
        let said = pid_xdata_strings(&app, valve);
        assert!(said.contains(&"label=GV-101".to_owned()), "{said:?}");
        app.tabs[i].dirty = false;
        let undo_before = app.tabs[i].history.undo_stack.len();

        let _ = app.run_command_line("PIDTAG gv-101 GV-109");
        app.finish_all_pending_history();
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("tag_imported")
        );
        assert_eq!(
            refused(&app),
            "PIDTAG: 本图上带位号 \"gv-101\" 的是 1 个导入的图例，导入的图例不能在这里改位号，未做改动。"
        );
        assert_eq!(
            pid_xdata_strings(&app, valve),
            said,
            "the record says what it said"
        );
        assert!(
            PidAuthor::load(sheet(&app), valve).is_none(),
            "and the valve was not made an object drawn here"
        );
        assert_eq!(
            app.tabs[i].history.undo_stack.len(),
            undo_before,
            "no undo step"
        );
        assert!(!app.tabs[i].dirty, "nothing to save");

        let (other, _) = untagged_valve(&app);
        restate(&mut app, other, &[("label", "GV-101")]);
        let _ = app.run_command_line("PIDTAG GV-101 GV-109");
        assert_eq!(
            refused(&app),
            "PIDTAG: 本图上带位号 \"GV-101\" 的是 2 个导入的图例，导入的图例不能在这里改位号，未做改动。"
        );

        let _ = app.run_command_line("PIDTAG GV-109 GV-110");
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("tag_absent")
        );
        assert_eq!(
            refused(&app),
            "PIDTAG: 本图上没有位号 \"GV-109\" 的符号，未做改动。"
        );
    }

    /// Where objects drawn here carry the tag as well, those are renamed as
    /// they were, in one undo step, and the receipt says the imported
    /// symbols were left: their records say what they said. With no
    /// imported symbol carrying the tag the receipt reads as it did.
    #[test]
    fn a_rename_takes_the_objects_drawn_here_and_leaves_the_imported_symbols() {
        use crate::pid::persist::PidAuthor;

        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, _) = tagged_valve(&app);
        let said = pid_xdata_strings(&app, valve);
        let drawn_here = authored(&mut app, "GV-101");
        let another = authored(&mut app, "P-1001");
        let undo_before = app.tabs[i].history.undo_stack.len();

        let _ = app.run_command_line("PIDTAG GV-101 GV-109");
        app.finish_all_pending_history();
        let author = PidAuthor::load(sheet(&app), drawn_here).expect("author");
        assert_eq!(
            (author.tag.as_str(), author.tag_source.as_str()),
            ("GV-109", "manual")
        );
        assert_eq!(
            last_line(&app),
            "PIDTAG: 位号 GV-101 → GV-109，1 个符号已改；另有 1 个导入的图例带这个位号，导入的图例不能在这里改位号，没有改。"
        );
        assert_eq!(
            pid_xdata_strings(&app, valve),
            said,
            "the imported valve says what it said"
        );
        assert_eq!(
            app.tabs[i].history.undo_stack.len(),
            undo_before + 1,
            "one undo step"
        );

        let _ = app.run_command_line("PIDTAG P-1001 P-1009");
        app.finish_all_pending_history();
        let author = PidAuthor::load(sheet(&app), another).expect("author");
        assert_eq!(author.tag, "P-1009");
        assert_eq!(
            last_line(&app),
            "PIDTAG: 位号 P-1001 → P-1009，1 个符号已改。"
        );
    }

    /// What a pick of one entity makes PIDTAG select of the symbols a
    /// `.pid` import placed, said the long way: by the functions the
    /// Properties panel and the inspector go by (`io::pid_identity`), one
    /// reading of the sheet after another. It is the picked symbol, or the
    /// symbol the label of the picked lettering hangs on, with the texts
    /// that are the lettering of its labels -- unless that symbol was drawn
    /// here.
    fn the_long_way(
        document: &document::Document,
        picked: document::Handle,
    ) -> Vec<(document::Handle, Vec<document::Handle>)> {
        use crate::io::pid_identity::{self, LabelOwner};
        use crate::pid::persist::PidAuthor;

        let entity = document
            .model_space_entities()
            .find(|entity| entity.common().handle == picked)
            .expect("what is picked is in model space");
        let symbol = if pid_identity::symbol_identity(entity).is_some() {
            Some(entity)
        } else if pid_identity::lettering(entity).is_some() {
            match pid_identity::label_owner(document, entity) {
                Some(LabelOwner::Symbol(owner)) => Some(owner),
                _ => None,
            }
        } else {
            None
        };
        let Some(symbol) = symbol.filter(|symbol| PidAuthor::from_entity(symbol).is_none()) else {
            return Vec::new();
        };
        let labels = pid_identity::symbol_identity(symbol)
            .and_then(|said| said.graphic)
            .map(|graphic| {
                pid_identity::labels_of(document, graphic)
                    .into_iter()
                    .filter(|label| pid_identity::lettering(label).is_some())
                    .map(|label| label.common().handle)
                    .collect()
            })
            .unwrap_or_default();
        vec![(symbol.common().handle, labels)]
    }

    /// What `imported_groups` makes of what is asked: the block reference
    /// and the labels of each group.
    fn made_of(
        document: &document::Document,
        asked: &Asked<'_>,
    ) -> Vec<(document::Handle, Vec<document::Handle>)> {
        imported_groups(document, asked)
            .into_iter()
            .map(|group| (group.reference, group.labels))
            .collect()
    }

    /// Every entity of the sheet in front, picked alone, makes the group
    /// the long way makes of it; the number of those that make one.
    fn picks_that_make_a_group(app: &OpenCADStudio) -> usize {
        let document = sheet(app);
        document
            .model_space_entities()
            .map(|entity| entity.common().handle)
            .filter(|picked| {
                let made = made_of(document, &Asked::Picked(&[picked.value()]));
                assert_eq!(made, the_long_way(document, *picked), "{picked:?} picked");
                !made.is_empty()
            })
            .count()
    }

    /// The groups PIDTAG makes of the symbols a `.pid` import placed are the
    /// ones the Properties panel and the inspector lead to, entity by
    /// entity: on the built sheets as imported, and with what an import
    /// does not leave but a drawing can come to hold -- an entity that is
    /// no text and states what a label states, which is no label; a text
    /// of several lines that does, which is one; a copy of a symbol, which
    /// has its original's labels while their lettering picks the original;
    /// and an object drawn here that states what an imported symbol states,
    /// which is none and, filed first under a number, leaves the labels of
    /// that number with no imported symbol to pick.
    #[test]
    fn imported_groups_are_what_the_identity_functions_make_of_each_pick() {
        use crate::pid::persist::PidAuthor;
        use document::entities::{EntityType, MText};

        let none: Vec<(document::Handle, Vec<document::Handle>)> = Vec::new();
        let of = |app: &OpenCADStudio, picked: document::Handle| {
            made_of(sheet(app), &Asked::Picked(&[picked.value()]))
        };

        // Three valves, and a label on each of two of them.
        let less = opened_with(crate::io::pid::sheet_of_symbols_the_file_says_less_of());
        assert_eq!(picks_that_make_a_group(&less), 5, "symbols said less of");

        let mut app = imported_sheet();
        let i = app.active_tab;
        let (valve, labels) = tagged_valve(&app);
        let (other, its_label) = untagged_valve(&app);
        let pipe = run_of_line(&app);
        // The two valves, the four labels of the one and the label of the
        // other.
        assert_eq!(picks_that_make_a_group(&app), 7, "as imported");
        assert_eq!(of(&app, valve), [(valve, labels.to_vec())]);
        assert_eq!(of(&app, labels[2]), [(valve, labels.to_vec())]);
        assert_eq!(of(&app, its_label), [(other, vec![its_label])]);

        // The label of the run of line states the first valve's number: it
        // hangs on a run of line all the same.
        let on_the_line = lettered(&app, "P-1001-50-CS");
        restate(&mut app, on_the_line, &[("label_of", "612")]);
        assert_eq!(picks_that_make_a_group(&app), 7, "a line's label");
        assert_eq!(of(&app, valve), [(valve, labels.to_vec())]);
        assert_eq!(of(&app, on_the_line), none);

        // The run of line itself states what a label of that valve states.
        restate(
            &mut app,
            pipe,
            &[("label_of", "612"), ("label_on", "symbol")],
        );
        assert_eq!(picks_that_make_a_group(&app), 7, "a run of line as a label");
        assert_eq!(of(&app, valve), [(valve, labels.to_vec())]);
        assert_eq!(of(&app, pipe), none);

        // A text of several lines made a label of the second valve.
        let several = app.tabs[i]
            .scene
            .add_entity(EntityType::MText(Box::new(MText {
                value: "DN80\\PPN16".to_owned(),
                ..MText::default()
            })));
        restate(
            &mut app,
            several,
            &[("label_of", "613"), ("label_on", "symbol")],
        );
        assert_eq!(picks_that_make_a_group(&app), 8, "a text of several lines");
        assert_eq!(of(&app, several), [(other, vec![its_label, several])]);

        // The second valve as a copy of the first: the labels of its own
        // number are left with no symbol.
        restate(&mut app, other, &[("graphic", "612")]);
        assert_eq!(picks_that_make_a_group(&app), 6, "a copy");
        assert_eq!(of(&app, other), [(other, labels.to_vec())]);
        assert_eq!(of(&app, labels[1]), [(valve, labels.to_vec())]);
        assert_eq!(of(&app, its_label), none);
        assert_eq!(
            made_of(sheet(&app), &Asked::Picked(&[valve.value(), other.value()])),
            [(valve, labels.to_vec()), (other, labels.to_vec())],
            "picked together, each has the labels"
        );

        // The first valve as an object drawn here: the copy is picked by
        // itself alone.
        PidAuthor::new("VALVE", "GV-101").store(&mut app.tabs[i].scene.document, valve);
        assert_eq!(picks_that_make_a_group(&app), 1, "an object drawn here");
        assert_eq!(of(&app, valve), none);
        assert_eq!(of(&app, labels[1]), none);
        assert_eq!(of(&app, other), [(other, labels.to_vec())]);
        assert_eq!(made_of(sheet(&app), &Asked::Tag("GV-101")), none);
    }

    /// What the receipt calls an imported symbol: the name of its symbol
    /// and its tag; what kind of thing it is where it states no symbol's
    /// name -- the class its item is published as, or else the kind
    /// SmartPlant holds the item as --; and a word for a symbol where it
    /// states neither. A symbol with no tag says so.
    #[test]
    fn the_receipt_names_an_imported_symbol_by_its_symbol_and_its_tag() {
        let mut app = opened_with(crate::io::pid::sheet_of_symbols_the_file_says_less_of());
        let named = |app: &OpenCADStudio| -> Vec<String> {
            let every: Vec<u64> = sheet(app)
                .model_space_entities()
                .map(|entity| entity.common().handle.value())
                .collect();
            imported_groups(sheet(app), &Asked::Picked(&every))
                .into_iter()
                .map(|group| group.name)
                .collect()
        };
        assert_eq!(
            named(&app),
            [
                "Ball Valve (无位号)",
                "Ball Valve (无位号)",
                "Ball Valve ZZ-9"
            ]
        );

        for at in [(100.0, 100.0), (170.0, 100.0), (100.0, 160.0)] {
            let valve = reference(&app, VALVE, at);
            restate(&mut app, valve, &[("symbol", "")]);
        }
        assert_eq!(
            named(&app),
            ["OPC (无位号)", "图例 (无位号)", "PIDBranchPoint ZZ-9"]
        );
    }

    /// The sample drawing of the plant test data beside this repository --
    /// the sheet of the well sites L and M, 592 symbols among some six
    /// thousand entities -- as a `.pid` import leaves it with nothing
    /// published beside it; or `None`, and a line saying so, where the
    /// drawing is not there.
    fn sample_sheet() -> Option<document::Document> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("pid-test-data")
            .join("backup-plant-data")
            .join("00")
            .join("00")
            .join("L M井场 注采阀组工艺及自控流程图.pid");
        if !path.is_file() {
            eprintln!("skipped: no sample drawing at {}", path.display());
            return None;
        }
        let import = crate::io::pid::load_pid(&path).expect("the sample imports");
        Some(import.document)
    }

    /// On the sample drawing, what PIDTAG makes of the symbols is what the
    /// functions the Properties panel goes by make of them, one reading of
    /// the sheet after another: everything picked at once is every symbol
    /// with the texts that are its labels; every tag finds the symbols that
    /// carry it; and the lettering of every label picks the symbol the
    /// label hangs on, or nothing where it hangs on a run of line or on
    /// anything else. How many there are of each is pinned beside it, and
    /// legend recognition takes none of those symbols for its own. Skipped,
    /// with a line saying so, where the plant test data is not beside the
    /// repository.
    #[test]
    fn on_the_sample_drawing_every_tag_and_every_label_finds_its_symbol() {
        use crate::io::pid_identity::{self, LabelOwner};
        use std::collections::BTreeMap;
        use std::time::Instant;

        let Some(document) = sample_sheet() else {
            return;
        };
        let every: Vec<u64> = document
            .model_space_entities()
            .map(|entity| entity.common().handle.value())
            .collect();
        let started = Instant::now();
        let all = made_of(&document, &Asked::Picked(&every));
        let at_once = started.elapsed();

        let symbols: Vec<&document::EntityType> = document
            .model_space_entities()
            .filter(|entity| pid_identity::symbol_identity(entity).is_some())
            .collect();
        let long_way: Vec<(document::Handle, Vec<document::Handle>)> = symbols
            .iter()
            .flat_map(|symbol| the_long_way(&document, symbol.common().handle))
            .collect();
        assert_eq!(all.len(), symbols.len(), "a group of every symbol");
        assert_eq!(all, long_way, "everything picked at once");

        // The symbols that carry each tag, by their place among the symbols.
        let mut carrying: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (place, symbol) in symbols.iter().enumerate() {
            if let Some(tag) = pid_identity::symbol_identity(symbol).and_then(|said| said.tag) {
                carrying.entry(tag.to_uppercase()).or_default().push(place);
            }
        }
        let started = Instant::now();
        for (tag, places) in &carrying {
            let carried_by: Vec<_> = places.iter().map(|place| all[*place].clone()).collect();
            assert_eq!(made_of(&document, &Asked::Tag(tag)), carried_by, "{tag}");
        }
        let by_tag = started.elapsed();

        let place: rustc_hash::FxHashMap<document::Handle, usize> = all
            .iter()
            .enumerate()
            .map(|(place, (reference, _))| (*reference, place))
            .collect();
        let (mut of_symbols, mut of_none, mut of_lines, mut of_others) = (0, 0, 0, 0);
        let started = Instant::now();
        for entity in document.model_space_entities() {
            if pid_identity::lettering(entity).is_none() {
                continue;
            }
            let hangs_on = match pid_identity::label_owner(&document, entity) {
                Some(LabelOwner::Symbol(symbol)) => {
                    of_symbols += 1;
                    vec![all[place[&symbol.common().handle]].clone()]
                }
                Some(LabelOwner::Missing) => {
                    of_none += 1;
                    Vec::new()
                }
                Some(LabelOwner::Line) => {
                    of_lines += 1;
                    Vec::new()
                }
                Some(LabelOwner::Other) => {
                    of_others += 1;
                    Vec::new()
                }
                None => continue,
            };
            let picked = entity.common().handle;
            assert_eq!(
                made_of(&document, &Asked::Picked(&[picked.value()])),
                hangs_on,
                "{picked:?} picked"
            );
        }
        let one_by_one = started.elapsed();

        let started = Instant::now();
        let recognition = pid_legend::recognise(&document, &Rules::load());
        let recognised = started.elapsed();
        assert!(
            recognition
                .symbols
                .iter()
                .flat_map(|symbol| &symbol.handles)
                .all(|handle| !place.contains_key(handle)),
            "legend recognition takes no imported symbol for its own"
        );
        eprintln!(
            "sample: {} entities in model space; everything picked at once {at_once:?}; \
             {} tags asked for one by one {by_tag:?}; {} labels picked one by one \
             {one_by_one:?}; legend recognition {recognised:?}, {} symbols recognised",
            every.len(),
            carrying.len(),
            of_symbols + of_none + of_lines + of_others,
            recognition.symbols.len()
        );

        assert_eq!(
            [
                ("symbols", all.len()),
                (
                    "symbols that carry a tag",
                    carrying.values().map(Vec::len).sum()
                ),
                ("tags", carrying.len()),
                (
                    "the most symbols of one tag",
                    carrying.values().map(Vec::len).max().unwrap_or(0)
                ),
                ("labels of symbols", of_symbols),
                (
                    "labels in the groups",
                    all.iter().map(|(_, labels)| labels.len()).sum()
                ),
                ("labels of a symbol that is not on the sheet", of_none),
                ("labels of runs of line", of_lines),
                ("labels of anything else", of_others),
                (
                    "symbols with no label",
                    all.iter().filter(|(_, labels)| labels.is_empty()).count()
                ),
                (
                    "the most labels of one symbol",
                    all.iter()
                        .map(|(_, labels)| labels.len())
                        .max()
                        .unwrap_or(0)
                ),
            ],
            [
                ("symbols", 592),
                ("symbols that carry a tag", 231),
                ("tags", 229),
                ("the most symbols of one tag", 2),
                ("labels of symbols", 803),
                ("labels in the groups", 803),
                ("labels of a symbol that is not on the sheet", 0),
                ("labels of runs of line", 151),
                ("labels of anything else", 4),
                ("symbols with no label", 209),
                ("the most labels of one symbol", 4),
            ]
        );
    }

    /// On a sheet that has both, PIDTAG selects the recognised symbols and
    /// the imported ones that carry the tag, and the receipt counts symbols
    /// and labels -- the tag lettering of a recognised symbol among the
    /// labels. Picked alone, the recognised one reads as it did, in groups
    /// and entities, and the imported one counts itself and its label.
    #[test]
    fn pidtag_selects_recognised_and_imported_symbols_side_by_side() {
        use document::entities::{EntityType, Line, Text};
        use document::types::{Transform, Vector3};

        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, _pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        // A block of four lines, 6 x 3 mm at (20, 5), made to state what a
        // symbol a `.pid` import placed states -- the tag of the recognised
        // valve with it -- and a text made the lettering of a label of it.
        let strokes: Vec<document::Handle> = [
            ((20.0, 5.0), (26.0, 5.0)),
            ((26.0, 5.0), (26.0, 8.0)),
            ((26.0, 8.0), (20.0, 8.0)),
            ((20.0, 8.0), (20.0, 5.0)),
        ]
        .into_iter()
        .map(|((x0, y0), (x1, y1))| {
            let mut line = Line::new();
            line.start = Vector3::new(x0, y0, 0.0);
            line.end = Vector3::new(x1, y1, 0.0);
            app.tabs[i].scene.add_entity(EntityType::Line(line))
        })
        .collect();
        let identity = Transform::identity();
        let imported = app.tabs[i]
            .scene
            .create_block_from_entities(&strokes, "IMPORTED-900", &identity, &identity)
            .unwrap();
        restate(
            &mut app,
            imported,
            &[
                ("role", "symbol"),
                ("graphic", "900"),
                ("symbol", "Gate Valve"),
                ("label", "BUV-3101"),
                ("tag_source", "file"),
            ],
        );
        let label = app.tabs[i].scene.add_entity(EntityType::Text(
            Text::with_value("DN80", Vector3::new(20.0, 9.0, 0.0)).with_height(2.5),
        ));
        restate(
            &mut app,
            label,
            &[("label_of", "900"), ("label_on", "symbol")],
        );

        let _ = app.run_command_line("PIDTAG BUV-3101");
        assert_eq!(selected(&app), in_order([insert, tag, imported, label]));
        assert_eq!(
            last_line(&app),
            "PIDTAG: 蝶阀 BUV-3101 + Gate Valve BUV-3101 -- 2 symbol(s), 2 label(s) selected and framed in red."
        );
        assert_eq!(frames(&app).len(), 2, "a frame for each");

        app.tabs[i].scene.select_entity(tag, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(selected(&app), in_order([insert, tag]));
        assert_eq!(
            last_line(&app),
            "PIDTAG: 蝶阀 BUV-3101 -- 1 group(s), 2 entities selected and framed in red."
        );

        app.tabs[i].scene.select_entity(label, true);
        let _ = app.run_command_line("PIDTAG ");
        assert_eq!(selected(&app), in_order([imported, label]));
        assert_eq!(
            last_line(&app),
            "PIDTAG: Gate Valve BUV-3101 -- 1 symbol(s), 1 label(s) selected and framed in red."
        );
    }

    #[test]
    fn a_pid_exception_row_selects_and_zooms_to_its_source() {
        use document::entities::{EntityType, Text};
        use document::types::Vector3;

        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        let source = app.tabs[i]
            .scene
            .add_entity(EntityType::Text(Text::with_value(
                "BUV-9999",
                Vector3::new(20.0, 30.0, 0.0),
            )));
        assert!(!app.pid_legend_exceptions_open);
        let _ = app.update(Message::PidLegendExceptionsToggle);
        assert!(app.pid_legend_exceptions_open);
        let camera = app.tabs[i].scene.camera_generation;
        let _ = app.update(Message::PidLegendPickException {
            handles: vec![source.value()],
            min: (16.0, 26.0),
            max: (24.0, 34.0),
        });
        assert_eq!(selected(&app), vec![source]);
        assert!(app.tabs[i].scene.camera_generation > camera);
    }

    #[test]
    fn pidlegend_on_publishes_xdata_off_keeps_it_and_purge_removes_everything() {
        use document::entities::{EntityType, Text};
        use document::types::Vector3;

        let mut app = OpenCADStudio::new_for_test();
        let (insert, tag, pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;
        app.tabs[i]
            .scene
            .document
            .get_entity_mut(pipe)
            .unwrap()
            .common_mut()
            .layer = "PIPE-PROCESS".to_string();
        app.tabs[i].scene.add_entity(EntityType::Text(
            Text::with_value("100-FW", Vector3::new(-10.0, 3.0, 0.0)).with_height(2.5),
        ));
        let before = app.tabs[i].scene.document.entities().count();

        let _ = app.run_command_line("PIDLEGEND REPORT");
        assert!(
            pid_legend::attached_handles(&app.tabs[i].scene.document).is_empty(),
            "REPORT reads but does not publish"
        );
        let _ = app.run_command_line("PIDLEGEND LIST");
        assert!(
            pid_legend::attached_handles(&app.tabs[i].scene.document).is_empty(),
            "LIST reads but does not publish"
        );

        let _ = app.run_command_line("PIDLEGEND ON");
        assert!(legend_count(&app) > 0);
        assert!(
            app.tabs[i]
                .scene
                .document
                .app_ids
                .contains(crate::io::PID_SEMANTICS_XDATA_APP),
            "ON registers the APPID so metadata survives a save"
        );
        let symbol = app.tabs[i]
            .pid_legend()
            .unwrap()
            .symbols
            .iter()
            .find(|symbol| symbol.handles.contains(&insert))
            .expect("butterfly valve");
        assert_eq!(symbol.tag.as_deref(), Some("BUV-3101"));
        let mut expected_symbol = vec![
            format!("class={}", symbol.label),
            "label=BUV-3101".to_string(),
        ];
        if !symbol.lines.is_empty() {
            expected_symbol.push(format!("lines={}", symbol.lines.join(",")));
        }
        expected_symbol.push("resolved=legend:block".to_string());
        assert_eq!(pid_xdata_strings(&app, insert), expected_symbol);
        assert_eq!(
            pid_xdata_strings(&app, pipe),
            ["class=PIDPipeline", "label=100-FW", "resolved=legend:pipe",]
        );
        assert!(
            pid_xdata_strings(&app, tag).is_empty(),
            "external tag lettering is not the symbol body"
        );

        let section = crate::scene::cache::properties::pid_semantics_section(
            app.tabs[i].scene.document.get_entity(insert).unwrap(),
        )
        .expect("published identity appears in Properties");
        assert!(section.props.iter().any(|property| {
            property.field == "pid_resolved"
                && property.value
                    == crate::scene::model::object::PropValue::ReadOnly(
                        crate::t!("Legend recognition").into_owned(),
                    )
        }));

        let _ = app.run_command_line("PIDLEGEND OFF");
        assert_eq!(legend_count(&app), 0);
        assert!(
            !pid_legend::attached_handles(&app.tabs[i].scene.document).is_empty(),
            "OFF only hides the overlay"
        );

        let _ = app.run_command_line("PIDLEGEND ON");
        assert!(legend_count(&app) > 0);
        let _ = app.run_command_line("PIDLEGEND PURGE");
        assert_eq!(legend_count(&app), 0);
        assert_eq!(app.tabs[i].scene.document.entities().count(), before);
        assert!(
            pid_legend::attached_handles(&app.tabs[i].scene.document).is_empty(),
            "PURGE clears recognition XDATA"
        );
        assert!(
            app.tabs[i].pid_legend.is_some() && !app.tabs[i].pid_legend_is_stale(),
            "the in-memory list remains usable after its published copies go"
        );
        let _ = app.update(Message::Undo);
        assert!(legend_count(&app) > 0);
        assert!(!pid_xdata_strings(&app, insert).is_empty());
        let _ = app.update(Message::Redo);
        assert_eq!(legend_count(&app), 0);
        assert!(pid_xdata_strings(&app, insert).is_empty());

        // If the overlay is hidden, dissolving a manual group still replaces
        // its group-owned metadata rather than leaving a stale
        // `resolved=legend:group:…` record behind.
        app.tabs[i].scene.deselect_all();
        app.tabs[i].scene.select_entity(insert, false);
        app.tabs[i].scene.select_entity(tag, false);
        let _ = app.run_command_line("PIDGROUP GROUP");
        let _ = app.run_command_line("PIDLEGEND ON");
        assert!(pid_xdata_strings(&app, insert)
            .iter()
            .any(|value| value.starts_with("resolved=legend:group:")));
        let _ = app.run_command_line("PIDLEGEND OFF");
        app.tabs[i].scene.deselect_all();
        app.tabs[i].scene.select_entity(insert, false);
        let _ = app.run_command_line("PIDGROUP OFF");
        assert!(!pid_xdata_strings(&app, insert)
            .iter()
            .any(|value| value.starts_with("resolved=legend:group:")));
    }

    #[test]
    fn pidlegend_on_clears_stale_publication_when_the_last_symbol_is_gone() {
        let mut app = OpenCADStudio::new_for_test();
        let (insert, _tag, _pipe) = valve_with_tag(&mut app);
        let i = app.active_tab;

        let _ = app.run_command_line("PIDLEGEND ON");
        assert!(legend_count(&app) > 0);
        assert!(!pid_xdata_strings(&app, insert).is_empty());

        let document::EntityType::Insert(block) =
            app.tabs[i].scene.document.get_entity_mut(insert).unwrap()
        else {
            panic!("valve helper returned a block reference");
        };
        block.block_name = "NOT-A-PID-SYMBOL".to_string();
        app.tabs[i].scene.bump_geometry();

        let _ = app.run_command_line("PIDLEGEND ON");
        assert_eq!(legend_count(&app), 0);
        assert!(pid_xdata_strings(&app, insert).is_empty());
        assert_eq!(
            app.tabs[i]
                .pid_legend()
                .map(|recognition| recognition.symbols.len()),
            Some(0)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pidlegend_export_writes_shared_json_and_csv_before_on_and_after_off() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = valve_with_tag(&mut app);
        let i = app.active_tab;
        let dir = std::env::temp_dir().join(format!("ocs pid export {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let json_path = dir.join("legend result.json");
        let _ = app.run_command_line(&format!("PIDLEGEND EXPORT \"{}\"", json_path.display()));
        let source = app.tabs[i].tab_title.clone();
        let expected_json = pid_legend::to_json_pretty(
            &source,
            app.tabs[i].pid_legend().expect("EXPORT recognises"),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&json_path).unwrap(), expected_json);
        assert_eq!(legend_count(&app), 0, "EXPORT does not draw");
        assert!(
            pid_legend::attached_handles(&app.tabs[i].scene.document).is_empty(),
            "EXPORT does not publish XDATA"
        );

        let _ = app.run_command_line("PIDLEGEND ON");
        let _ = app.run_command_line("PIDLEGEND OFF");
        let published = pid_legend::attached_handles(&app.tabs[i].scene.document);
        let csv_path = dir.join("legend after off.csv");
        let _ = app.run_command_line(&format!("PIDLEGEND EXPORT {}", csv_path.display()));
        assert_eq!(
            std::fs::read_to_string(&csv_path).unwrap(),
            pid_legend::to_csv(app.tabs[i].pid_legend().unwrap())
        );
        assert_eq!(
            pid_legend::attached_handles(&app.tabs[i].scene.document),
            published,
            "EXPORT after OFF leaves published metadata as it was"
        );

        let error_revision = app.command_line.error_revision;
        let _ = app.run_command_line("PIDLEGEND EXPORT");
        assert!(
            app.tabs[i].active_cmd.is_some(),
            "EXPORT prompts for a path"
        );
        let _ = app.feed_command(crate::command::StepInput::Enter);
        assert!(app.tabs[i].active_cmd.is_none());
        assert!(app.command_line.error_revision > error_revision);
        let error_revision = app.command_line.error_revision;
        let _ = app.run_command_line(&format!(
            "PIDLEGEND EXPORT {}",
            dir.join("legend.txt").display()
        ));
        assert!(app.command_line.error_revision > error_revision);
        assert!(!dir.join("legend.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
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
        let buv = app.tabs[i]
            .pid_legend()
            .unwrap()
            .symbols
            .iter()
            .find(|symbol| symbol.tag.as_deref() == Some("BUV-3201"))
            .expect("BUV-3201 on FF02-06");
        assert_eq!(buv.label, "蝶阀");
        assert!(
            buv.lines.iter().any(|line| line == "100-FW"),
            "{:?}",
            buv.lines
        );
        let body = *buv.handles.first().expect("BUV-3201 body handle");
        let section = crate::scene::cache::properties::pid_semantics_section(
            app.tabs[i].scene.document.get_entity(body).unwrap(),
        )
        .expect("BUV-3201 has published identity");
        for (field, expected) in [
            ("pid_class", "蝶阀"),
            ("pid_label", "BUV-3201"),
            ("pid_lines", "100-FW"),
        ] {
            assert!(
                section.props.iter().any(|property| {
                    property.field == field
                        && property.value
                            == crate::scene::model::object::PropValue::ReadOnly(
                                expected.to_string(),
                            )
                }),
                "{field}"
            );
        }
        assert!(section.props.iter().any(|property| {
            property.field == "pid_resolved"
                && property.value
                    == crate::scene::model::object::PropValue::ReadOnly(
                        crate::t!("Legend recognition").into_owned(),
                    )
        }));
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
        use document::entities::{EntityType, Line};
        use document::types::Vector3;
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
        let selected_after: BTreeSet<_> = app.tabs[i].scene.selected.iter().copied().collect();
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

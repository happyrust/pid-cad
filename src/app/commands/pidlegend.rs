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
//! ```
//!
//! ON and REPORT stash the recognition on the tab; the legend list panel
//! (`ui::window::pid_legend_list`) renders it and its rows zoom to symbols
//! and pipe lines. ON also opens the panel — the list is the index of what
//! was just drawn.
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
                    self.tabs[i].scene.erase_entities(&handles);
                    self.tabs[i].dirty = true;
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
                self.tabs[i].pid_legend = Some(recognition);
            }
            "LIST" => {
                if self.show_pid_legend_list {
                    self.show_pid_legend_list = false;
                } else {
                    if self.tabs[i].pid_legend.is_none() {
                        let rules = Rules::load();
                        let recognition =
                            pid_legend::recognise(&self.tabs[i].scene.document, &rules);
                        self.command_line.push_output(&format!(
                            "PIDLEGEND: {} symbols listed. PIDLEGEND ON draws them.",
                            recognition.symbols.len()
                        ));
                        self.tabs[i].pid_legend = Some(recognition);
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
                self.tabs[i].pid_legend = Some(recognition);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_legend;

    /// A real CPECC sheet beside the checkout, when it is there and not
    /// locked by an editor.
    fn open_sheet(app: &mut OpenCADStudio, name: &str) -> bool {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("0版重新处理dxf-12张")
            .join(name);
        if !path.is_file() {
            eprintln!("skipping: {name} not present");
            return false;
        }
        let request = serde_json::json!({ "op": "open", "path": path.to_string_lossy() });
        let reply = app.automation_op(&request.to_string());
        if reply["ok"].as_bool() != Some(true) {
            eprintln!("skipping: {reply}");
            return false;
        }
        true
    }

    fn legend_count(app: &OpenCADStudio) -> usize {
        pid_legend::legend_handles(&app.tabs[app.active_tab].scene.document).len()
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
            app.tabs[i].pid_legend.as_ref().map(|r| r.symbols.len()),
            Some(118),
            "recognition stored on the tab"
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

        let _ = app.run_command_line("PIDLEGEND OFF");
        assert_eq!(legend_count(&app), 0);
        assert_eq!(app.tabs[i].scene.document.entities().count(), before);
        assert!(
            app.tabs[i].pid_legend.is_some(),
            "the list stays browsable after OFF"
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
        let recognition = app.tabs[i].pid_legend.as_ref().expect("recognition stored");
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

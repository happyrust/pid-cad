//! PIDLEGEND — mark up the P&ID symbols a sheet places.
//!
//! Recognition and the legend geometry live in `io::pid_legend`; this is the
//! command-line front: run it, print the per-class summary, put the coloured
//! rectangles and labels into the drawing on `PID-LEGEND-<CLASS>` layers, and
//! take them out again.
//!
//! ```text
//! PIDLEGEND            prompt with the three verbs below (Enter = ON)
//! PIDLEGEND ON         recognise, then draw (replacing an earlier legend)
//! PIDLEGEND OFF        remove the legend entities (layers stay, empty)
//! PIDLEGEND REPORT     recognise and print the summary, draw nothing
//! ```
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
                "PIDLEGEND  [ON / OFF / REPORT] <ON>:",
                vec![
                    ("ON", "ON", None),
                    ("OFF", "OFF", None),
                    ("REPORT", "REPORT", None),
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
                self.command_line.push_output(&format!(
                    "PIDLEGEND: drew {drawn} entities on {} layers ({}*). PIDLEGEND OFF removes them.",
                    layers.len(),
                    pid_legend::LAYER_PREFIX
                ));
            }
            other => {
                self.command_line.push_error(&format!(
                    "PIDLEGEND: unknown option \"{other}\" -- use ON, OFF or REPORT."
                ));
            }
        }
        Some(Task::none())
    }
}

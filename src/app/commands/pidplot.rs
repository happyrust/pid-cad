//! PIDPLOT -- plot the active `.pid` drawing to PDF with pid-parse's own
//! plotter (`io::pid_pdf`).
//!
//! ```text
//! PIDPLOT              prompt for the PDF file (Enter = <name>.pidplot.pdf
//!                      next to the drawing)
//! PIDPLOT <file.pdf>   write the plot there; a relative path is taken from
//!                      the drawing's folder
//! ```
//!
//! The plot is made from the `.pid` on disk -- the sheet as SmartPlant prints
//! it, border template and library symbol bodies included -- not from the
//! drawing as edited here. It runs on a worker thread and reports on the
//! command line when the file is written. Bare `PIDPLOT` only prompts, like
//! bare PIDTAG: the driver dispatches the first word of an inline line on its
//! own first.

use super::*;

fn unquoted(arg: &str) -> &str {
    let quoted = arg.len() >= 2
        && ((arg.starts_with('"') && arg.ends_with('"'))
            || (arg.starts_with('\'') && arg.ends_with('\'')));
    if quoted {
        &arg[1..arg.len() - 1]
    } else {
        arg
    }
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidplot(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let rest = cmd.strip_prefix("PIDPLOT")?;
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return None;
        }
        let arg = unquoted(rest.trim());
        if arg.is_empty() && cmd.trim_end() == cmd {
            use crate::command::FreeTextValuePromptCommand;
            let c = FreeTextValuePromptCommand::new(
                "PIDPLOT",
                "PIDPLOT  PDF file <next to the drawing>:",
            );
            self.command_line.push_info(&c.prompt());
            self.tabs[i].active_cmd = Some(Box::new(c));
            return Some(self.finish_dispatch(cmd));
        }
        Some(self.start_pid_pdf(i, arg))
    }

    #[cfg(target_arch = "wasm32")]
    fn start_pid_pdf(&mut self, _i: usize, _arg: &str) -> Task<Message> {
        self.command_line
            .push_error("PIDPLOT: plotting a .pid to PDF is not available in the web build.");
        Task::none()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_pid_pdf(&mut self, i: usize, arg: &str) -> Task<Message> {
        let Some(drawing) = self.pid_source(i) else {
            self.command_line
                .push_error("PIDPLOT: the active drawing was not opened from a .pid file.");
            return Task::none();
        };
        let target = if arg.is_empty() {
            crate::io::pid_pdf::default_target(&drawing)
        } else {
            let mut target = std::path::PathBuf::from(arg);
            if target.is_relative() {
                if let Some(folder) = drawing.parent() {
                    target = folder.join(target);
                }
            }
            if target.extension().is_none() {
                target.set_extension("pdf");
            }
            target
        };
        self.plot_pid_pdf(drawing, target)
    }

    /// The `.pid` file tab `i` was opened from, if it was.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn pid_source(&self, i: usize) -> Option<std::path::PathBuf> {
        self.tabs[i]
            .current_path
            .clone()
            .filter(|path| crate::io::pid_pdf::is_pid(path))
    }

    /// EXPORT / EXPORTPDF / Plot to a PDF file, for a drawing opened from a
    /// `.pid`: pid-parse's plotter makes the file, as PIDPLOT does, so the
    /// export is the sheet SmartPlant prints. An SVG target, or any other
    /// drawing, stays with the plot pipeline (`None`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn export_pid_pdf(
        &mut self,
        i: usize,
        target: &std::path::Path,
    ) -> Option<Task<Message>> {
        let drawing = self.pid_source(i)?;
        if !target
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            return None;
        }
        self.command_line.push_info(
            "PDF export of a .pid: pid-parse's plotter draws the sheet; the plot dialog's paper, scale and plot style settings do not apply.",
        );
        Some(self.plot_pid_pdf(drawing, target.to_path_buf()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn plot_pid_pdf(
        &mut self,
        drawing: std::path::PathBuf,
        target: std::path::PathBuf,
    ) -> Task<Message> {
        if target == drawing {
            self.command_line
                .push_error("PIDPLOT: the PDF cannot replace the drawing itself.");
            return Task::none();
        }
        self.command_line.push_output(&format!(
            "PIDPLOT: plotting {} (the .pid on disk; edits made here are not in it) ...",
            drawing.display()
        ));
        Task::perform(
            crate::io::pid_pdf::plot_pdf_in_background(drawing, target),
            Message::PidPdfFinished,
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::app) fn on_pid_pdf_finished(
        &mut self,
        result: Result<crate::io::pid_pdf::PidPdfReceipt, String>,
    ) {
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                self.command_line.push_error(&format!("PIDPLOT: {error}"));
                return;
            }
        };
        let page = receipt
            .page_mm
            .map_or_else(String::new, |[w, h]| format!(", page {w:.0} x {h:.0} mm"));
        self.command_line.push_output(&format!(
            "PIDPLOT: wrote {} ({} bytes, {} primitives{page}).",
            receipt.path.display(),
            receipt.bytes,
            receipt.primitives,
        ));
        match &receipt.template {
            Some(template) => self.command_line.push_output(&format!(
                "PIDPLOT: border template {}.",
                template.display()
            )),
            None => self.command_line.push_info(&format!(
                "PIDPLOT: no border template of this page size under {}.",
                receipt.corpus.display()
            )),
        }
        self.command_line.push_output(&format!(
            "PIDPLOT: {} of {} symbol files found in the library.",
            receipt.symbols_found, receipt.symbols_referenced
        ));
        if !receipt.missing_symbols.is_empty() {
            self.command_line.push_warning(&format!(
                "PIDPLOT: not in the library (boxed on the page): {}",
                receipt.missing_symbols.join(", ")
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unquoted;

    #[test]
    fn a_quoted_path_loses_its_quotes() {
        assert_eq!(unquoted("\"D:/out/a b.pdf\""), "D:/out/a b.pdf");
        assert_eq!(unquoted("'a.pdf'"), "a.pdf");
        assert_eq!(unquoted("a.pdf"), "a.pdf");
        assert_eq!(unquoted("\""), "\"");
    }
}

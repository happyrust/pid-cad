//! PIDPLOT's engine: a `.pid` drawing plotted to PDF by pid-parse's own
//! plotter (`pid-plot`).
//!
//! This is the sheet the way SmartPlant prints it -- the border template the
//! frame record links to, the library bodies of the placed symbols, and the
//! typefaces the style table names, subset into the file -- made from the
//! `.pid` on disk. The imported drawing and the PLOT pipeline only
//! approximate that page, and edits made since the file was opened are not
//! in it.

use std::path::{Path, PathBuf};

/// How many folders above the drawing are searched for the project's `Ref`.
const CORPUS_SEARCH_DEPTH: usize = 5;

/// What one plot wrote.
#[derive(Debug, Clone)]
pub struct PidPdfReceipt {
    pub path: PathBuf,
    pub bytes: usize,
    pub primitives: usize,
    /// Page size the PDF carries, in millimetres.
    pub page_mm: Option<[f64; 2]>,
    /// Border template the furniture came from, when the sheet has one.
    pub template: Option<PathBuf>,
    pub symbols_found: usize,
    pub symbols_referenced: usize,
    /// Symbol files the drawing names and no library root holds.
    pub missing_symbols: Vec<String>,
    /// Where the symbol library, border templates and fonts were looked up.
    pub corpus: PathBuf,
}

pub fn is_pid(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pid"))
}

/// Where the plot goes when PIDPLOT is given no file: next to the drawing,
/// under a name of its own. `<name>.pdf` there is usually SmartPlant's
/// reference print of the same sheet, which must not be overwritten.
pub fn default_target(drawing: &Path) -> PathBuf {
    let stem = drawing
        .file_stem()
        .map_or_else(|| "drawing".into(), |stem| stem.to_string_lossy());
    drawing.with_file_name(format!("{stem}.pidplot.pdf"))
}

/// The folder pid-plot reads reference data from for `drawing`: the nearest
/// one, from the drawing's own folder up, that holds a `Ref` folder (where a
/// SmartPlant project keeps `Symbols` and `Template Files`, next to its
/// drawings), else the drawing's folder. pid-plot looks for symbols in
/// `<corpus>/Ref`, templates in `<corpus>/Ref/Template Files` and fonts in
/// `<corpus>/fonts`; `PID_SYMBOL_LIBRARY` and `PID_PLOT_FONTS` still win.
pub fn corpus_for(drawing: &Path) -> PathBuf {
    let own = drawing.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    let mut dir = Some(own.as_path());
    for _ in 0..CORPUS_SEARCH_DEPTH {
        let Some(at) = dir else { break };
        if at.join("Ref").is_dir() {
            return at.to_path_buf();
        }
        dir = at.parent();
    }
    own
}

fn font_dirs(corpus: &Path) -> Vec<PathBuf> {
    let mut dirs = ::pid_plot::fonts::font_dirs(Some(corpus));
    // Last resort: a corpus without its own `fonts` folder still finds Arial,
    // Arial Narrow and FangSong where Windows installs them.
    if let Some(windows) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windows).join("Fonts"));
    }
    dirs
}

/// Lay out `drawing`, write its PDF to `target` and say what went in.
pub fn plot_pdf(drawing: &Path, target: &Path) -> Result<PidPdfReceipt, String> {
    let corpus = corpus_for(drawing);
    let model =
        ::pid_plot::model::lay_out_drawing(drawing, &corpus).map_err(|error| error.to_string())?;
    let prepared = ::pid_plot::fonts::prepare_fonts(&[&model], &font_dirs(&corpus))?;
    let fonts: Vec<::pid_plot::write::PlotFont<'_>> = prepared
        .iter()
        .map(|face| ::pid_plot::write::PlotFont {
            key: &face.key,
            bytes: &face.bytes,
        })
        .collect();
    let pdf = ::pid_plot::write::write_plot(&model, &fonts).map_err(|error| error.to_string())?;
    if let Some(parent) = target.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(target, &pdf)
        .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
    Ok(PidPdfReceipt {
        path: target.to_path_buf(),
        bytes: pdf.len(),
        primitives: model.primitives.len(),
        page_mm: model.resolved_page_mm(),
        template: model
            .template
            .as_ref()
            .map(|template| PathBuf::from(&template.path)),
        symbols_found: model.symbols_found,
        symbols_referenced: model.symbols_referenced,
        missing_symbols: model.missing_symbols.clone(),
        corpus,
    })
}

/// [`plot_pdf`] on a worker thread: parsing the sheet and its template and
/// subsetting CJK faces takes seconds, which the UI thread must not wait out.
pub async fn plot_pdf_in_background(
    drawing: PathBuf,
    target: PathBuf,
) -> Result<PidPdfReceipt, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-pid-plot".into())
        // The same stack the app's own threads get (RUST_MIN_STACK in
        // .cargo/config.toml): the record walk recurses through nested
        // symbol and template structures.
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let _ = sender.send(plot_pdf(&drawing, &target));
        })
        .map_err(|error| format!("cannot start the plot: {error}"))?;
    receiver
        .await
        .unwrap_or_else(|_| Err("the plot stopped before it finished".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_target_leaves_the_reference_print_alone() {
        let drawing = Path::new("D:/plant/Drawings/DWG-0201.pid");
        assert_eq!(
            default_target(drawing),
            Path::new("D:/plant/Drawings/DWG-0201.pidplot.pdf")
        );
    }

    #[test]
    fn the_corpus_is_the_nearest_folder_holding_ref() {
        let root = std::env::temp_dir().join(format!("ocs-pid-pdf-corpus-{}", std::process::id()));
        let drawings = root.join("Drawings").join("Unit 1");
        std::fs::create_dir_all(&drawings).unwrap();
        std::fs::create_dir_all(root.join("Ref").join("Symbols")).unwrap();
        let drawing = drawings.join("sheet.pid");
        assert_eq!(corpus_for(&drawing), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_ref_the_corpus_is_the_drawing_folder() {
        let root = std::env::temp_dir().join(format!("ocs-pid-pdf-plain-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let drawing = root.join("sheet.pid");
        assert_eq!(corpus_for(&drawing), root);
        let _ = std::fs::remove_dir_all(&root);
    }
}

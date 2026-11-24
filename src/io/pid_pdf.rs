//! PIDPLOT's engine: a `.pid` drawing plotted to PDF by pid-parse's own
//! plotter (`pid-plot`).
//!
//! This is the sheet the way SmartPlant prints it -- the border template the
//! frame record links to, the library bodies of the placed symbols, and the
//! typefaces the style table names, subset into the file -- made from the
//! `.pid` on disk. The imported drawing and the PLOT pipeline only
//! approximate that page, and edits made since the file was opened are not
//! in it.
//!
//! Reference data comes from the drawing's own project first (the nearest
//! folder above it holding `Ref`). A project that lacks the border templates
//! -- a SmartPlant backup keeps them inside a `RefData~*` archive, not under
//! `Ref/Template Files` -- takes them, and fonts it does not carry, from the
//! reference folder ([`reference_root`]).
//!
//! A drawing whose project carries its own templates is plotted the way
//! pid-parse's command-line plotter plots it against that project: the same
//! layout call, the page titled with the drawing's file name, and
//! `PID_PLOT_CJK_FONT` adding a face as it does there. With the same faces in
//! reach the two files are the same bytes; three plant sheets are held to
//! that by `pid_pdf_real_sheets_are_the_bytes_pid_plot_writes`. Faces are
//! looked for in more folders here ([`font_folders`]) than `pid_plot` looks
//! in, so a sheet it refuses for want of a face may still be written here.

use std::io::Read;
use std::path::{Path, PathBuf};

/// How many folders above the drawing are searched for the project's `Ref`.
const CORPUS_SEARCH_DEPTH: usize = 5;

/// Names the reference folder; see [`reference_root`].
const REFERENCE_ENV: &str = "PID_PLOT_CORPUS";

/// Names one more face file to embed, whole and under no name, as `pid_plot`
/// reads it: it letters only a stretch that no single face pid-plot borrows
/// from draws whole.
const CJK_FONT_ENV: &str = "PID_PLOT_CJK_FONT";

/// The reference folder when [`REFERENCE_ENV`] is not set: the plant test
/// data on the machine this was set up for (symbols under `Ref/Symbols`,
/// templates in its `RefData~*` archives, plot faces under `fonts`).
const DEFAULT_REFERENCE_ROOT: &str = r"D:\work\plant-code\cad\pid-test-data";

/// Largest template file read out of an archive; real ones are under 200 KB.
const TEMPLATE_BYTES_MAX: u64 = 16 << 20;

/// Counts the layouts of the template cache, so a folder an earlier build
/// filled is not read again: up to layout 1 the templates were written under
/// rank-prefixed names (`1 CPE-SQ-A1.igr`), which no sheet's link names.
const TEMPLATE_CACHE_LAYOUT: u32 = 2;

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
    /// The folder border templates were looked up under.
    pub corpus: PathBuf,
    /// Where reference data came from beyond the drawing's own project.
    pub notes: Vec<String>,
    /// pid-plot's account of the page.
    pub ledger: PageLedger,
}

/// pid-plot's account of one page, summed over the drawing and its border
/// template: the line `pid_plot` prints as `ledger entities ... on_page ...`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageLedger {
    /// Entities the layout visited.
    pub entities: u64,
    /// Of those, the ones that reached the page.
    pub on_page: u64,
    /// What was left out on purpose, counted over every layer of the account.
    pub intentional: u64,
    /// What was left out in a way that may be a fault, counted the same way.
    pub suspect: u64,
    /// What nothing accounts for; a page that balances has none.
    pub unattributed: u64,
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

/// The drawing's project: the nearest folder, from the drawing's own folder
/// up, that holds a `Ref` folder (where a SmartPlant project keeps `Symbols`
/// and `Template Files`, next to its drawings), else the drawing's folder.
/// pid-plot looks for symbols in `<corpus>/Ref`, templates in
/// `<corpus>/Ref/Template Files` and fonts in `<corpus>/fonts`;
/// `PID_SYMBOL_LIBRARY` and `PID_PLOT_FONTS` still win.
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

/// The reference folder: `PID_PLOT_CORPUS`, else [`DEFAULT_REFERENCE_ROOT`],
/// whichever exists.
pub fn reference_root() -> Option<PathBuf> {
    std::env::var_os(REFERENCE_ENV)
        .map(PathBuf::from)
        .filter(|root| root.is_dir())
        .or_else(|| Some(PathBuf::from(DEFAULT_REFERENCE_ROOT)).filter(|root| root.is_dir()))
}

/// Where one plot takes its reference data from.
struct Sources {
    /// The folder pid-plot reads border templates under.
    corpus: PathBuf,
    /// Symbol roots, when they are stated rather than left to pid-plot.
    symbol_roots: Option<Vec<PathBuf>>,
    fonts: Vec<PathBuf>,
    notes: Vec<String>,
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// The folders faces are looked for in, in order. `own` is pid-plot's list
/// for the project -- `PID_PLOT_FONTS`, the project's `fonts`, the folder its
/// build names -- and stays whole and first, so a face `pid_plot` finds is
/// the file found here. Then the caller's folders, the reference folder's
/// `fonts`, and last the folder Windows installs Arial, Arial Narrow and
/// FangSong in. SimSun is not among those: Windows ships it as a collection
/// (`simsun.ttc`), which pid-plot does not embed.
fn font_folders(
    own: Vec<PathBuf>,
    extra: &[PathBuf],
    reference: Option<&Path>,
    windows: Option<&Path>,
) -> Vec<PathBuf> {
    let mut fonts = own;
    for dir in extra {
        push_unique(&mut fonts, dir.clone());
    }
    if let Some(root) = reference {
        push_unique(&mut fonts, root.join("fonts"));
    }
    if let Some(windows) = windows {
        push_unique(&mut fonts, windows.join("Fonts"));
    }
    fonts
}

/// The sources of a drawing plotted where it lies: its project is found from
/// its folder ([`corpus_for`]).
fn sources_for(drawing: &Path) -> Sources {
    sources_in(drawing, corpus_for(drawing), &[])
}

/// The sources of a drawing whose project the caller states, with the font
/// folders the caller adds ([`font_folders`]).
fn sources_in(drawing: &Path, project: PathBuf, extra_fonts: &[PathBuf]) -> Sources {
    let reference = reference_root();
    let windows = std::env::var_os("WINDIR").map(PathBuf::from);
    let fonts = font_folders(
        ::pid_plot::fonts::font_dirs(Some(&project)),
        extra_fonts,
        reference.as_deref(),
        windows.as_deref(),
    );
    let reference = match reference {
        Some(root) if !has_templates(&project) => root,
        _ => {
            return Sources {
                corpus: project,
                symbol_roots: None,
                fonts,
                notes: Vec::new(),
            };
        }
    };
    let mut notes = Vec::new();
    let corpus = if has_templates(&reference) {
        notes.push(format!("border templates from {}", reference.display()));
        reference.clone()
    } else {
        match archived_templates(&reference) {
            Ok(Some((cache, archive))) => {
                notes.push(format!(
                    "border templates read out of {} into {}",
                    archive.display(),
                    cache.display()
                ));
                cache
            }
            Ok(None) => {
                notes.push(format!(
                    "no border templates under {} or in its RefData archives",
                    reference.display()
                ));
                project.clone()
            }
            Err(error) => {
                notes.push(format!("border templates: {error}"));
                project.clone()
            }
        }
    };
    let roots = symbol_roots(drawing, &reference);
    notes.push(format!(
        "symbol library: {}",
        roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    ));
    Sources {
        corpus,
        symbol_roots: Some(roots),
        fonts,
        notes,
    }
}

fn holds_templates(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("igr"))
        })
    })
}

/// Whether pid-plot finds a border template under `corpus`: the same four
/// folders it searches (`<corpus>/Ref/Template Files`, `<corpus>/Ref`, and
/// both under the corpus's parent).
fn has_templates(corpus: &Path) -> bool {
    let parent = corpus.parent();
    [
        Some(corpus.join("Ref").join("Template Files")),
        Some(corpus.join("Ref")),
        parent.map(|parent| parent.join("Ref").join("Template Files")),
        parent.map(|parent| parent.join("Ref")),
    ]
    .into_iter()
    .flatten()
    .any(|dir| holds_templates(&dir))
}

fn push_symbol_dirs(roots: &mut Vec<PathBuf>, holder: &Path) {
    let Ok(entries) = std::fs::read_dir(holder) else {
        return;
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.file_name().to_string_lossy().to_lowercase().contains("symbol"))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    for root in found {
        push_unique(roots, root);
    }
}

/// Symbol roots once the reference folder joins in: `PID_SYMBOL_LIBRARY`
/// alone when it is set (pid-plot's own rule), else the symbol folders from
/// the drawing up -- in each folder and in a `Ref` beside it, as the importer
/// finds them -- then the reference folder's. An earlier root wins a symbol
/// both hold, so the drawing's own project decides what it looks like.
fn symbol_roots(drawing: &Path, reference: &Path) -> Vec<PathBuf> {
    if let Some(list) = std::env::var_os("PID_SYMBOL_LIBRARY") {
        let roots: Vec<PathBuf> = std::env::split_paths(&list)
            .filter(|root| root.is_dir())
            .collect();
        if !roots.is_empty() {
            return roots;
        }
    }
    let mut roots = Vec::new();
    let mut dir = drawing.parent();
    for _ in 0..CORPUS_SEARCH_DEPTH {
        let Some(at) = dir else { break };
        push_symbol_dirs(&mut roots, at);
        push_symbol_dirs(&mut roots, &at.join("Ref"));
        dir = at.parent();
    }
    push_symbol_dirs(&mut roots, &reference.join("Ref"));
    roots
}

/// The `RefData~<schema>~<id>` files of a SmartPlant backup, path-sorted.
fn refdata_archives(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut archives: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("RefData~"))
        })
        .collect();
    archives.sort();
    archives
}

/// The `.igr` files of one archive, by file name. `None` when the file is
/// not a zip (a `RefData~` payload can also be a compound file or text).
fn templates_in(archive: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    let mut file = std::fs::File::open(archive).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"PK\x03\x04" {
        return None;
    }
    let file = std::fs::File::open(archive).ok()?;
    let mut entries = zip::ZipArchive::new(file).ok()?;
    let mut templates = Vec::new();
    for index in 0..entries.len() {
        let Ok(entry) = entries.by_index(index) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        let Some(base) = name
            .rsplit('/')
            .next()
            .filter(|base| base.to_ascii_lowercase().ends_with(".igr"))
            .map(str::to_string)
        else {
            continue;
        };
        let mut bytes = Vec::new();
        if entry.take(TEMPLATE_BYTES_MAX).read_to_end(&mut bytes).is_ok() {
            templates.push((base, bytes));
        }
    }
    Some(templates)
}

/// Read the border templates of a SmartPlant backup out of its `RefData~*`
/// archives into a cache folder laid out as a corpus
/// (`<cache>/Ref/Template Files`), and answer that folder with the archive
/// they came from. The first archive holding any `.igr` is used; the cache is
/// keyed by its path, size and time, so a later plot reuses it.
///
/// A template keeps the name the archive gives it. pid-plot lets a sheet whose
/// frame links a template (`CPE-SQ~2.IGR`) wear only a file whose name begins
/// with what the link spells (`CPE-SQ`), and picks among those by extent.
fn archived_templates(root: &Path) -> Result<Option<(PathBuf, PathBuf)>, String> {
    for archive in refdata_archives(root) {
        let Some(templates) = templates_in(&archive) else {
            continue;
        };
        if templates.is_empty() {
            continue;
        }
        let cache = template_cache(&archive);
        let dir = cache.join("Ref").join("Template Files");
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        for (name, bytes) in &templates {
            let path = dir.join(name);
            if !path.is_file() {
                std::fs::write(&path, bytes)
                    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            }
        }
        return Ok(Some((cache, archive)));
    }
    Ok(None)
}

fn template_cache(archive: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    TEMPLATE_CACHE_LAYOUT.hash(&mut hasher);
    archive.hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(archive) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            modified.hash(&mut hasher);
        }
    }
    std::env::temp_dir()
        .join("ocs-pid-plot-templates")
        .join(format!("{:016x}", hasher.finish()))
}

/// Lay out `drawing`, write its PDF to `target` and say what went in. The
/// drawing's project is found from where it lies ([`corpus_for`]).
pub fn plot_pdf(drawing: &Path, target: &Path) -> Result<PidPdfReceipt, String> {
    plot_from(drawing, target, sources_for(drawing))
}

/// Where a drawing belongs, stated by a caller that knows, as
/// [`plot_pdf_against`] takes it: the folder holding its `Ref`, and further
/// folders to look for faces in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub folder: PathBuf,
    pub fonts: Vec<PathBuf>,
}

/// [`plot_pdf`] for a caller that knows where the drawing belongs, as one
/// plotting every sheet of a backup does: `project` is the folder holding
/// its `Ref` -- the folder the backup was restored to -- and `fonts` are
/// further folders to look for faces in ([`font_folders`]). A project
/// without border templates takes them from the reference folder as it does
/// for [`plot_pdf`], and the receipt's notes say so.
pub fn plot_pdf_against(
    drawing: &Path,
    target: &Path,
    project: &Path,
    fonts: &[PathBuf],
) -> Result<PidPdfReceipt, String> {
    let sources = sources_in(drawing, project.to_path_buf(), fonts);
    plot_from(drawing, target, sources)
}

fn plot_from(drawing: &Path, target: &Path, sources: Sources) -> Result<PidPdfReceipt, String> {
    let model = match &sources.symbol_roots {
        None => ::pid_plot::model::lay_out_drawing(drawing, &sources.corpus),
        Some(roots) => {
            let labels: Vec<String> = roots.iter().map(|root| root.display().to_string()).collect();
            ::pid_plot::model::lay_out_with_symbol_roots(
                drawing,
                &sources.corpus,
                roots.clone(),
                // No `RefData~*` archives: the symbol library is the folders
                // `symbol_roots` found.
                Vec::new(),
                &labels,
            )
        }
    }
    .map_err(|error| error.to_string())?;
    // `pid_plot` reads the variable the same way: a value that is not
    // Unicode names no file.
    let extra_face = std::env::var(CJK_FONT_ENV).ok().map(PathBuf::from);
    let written = write_page(&model, &sources.fonts, extra_face.as_deref(), drawing)?;
    let ledger = ::pid_plot::ledger::ExportLedger::assemble(&model, &written.ledger).summary();
    let pdf = written.pdf;
    if let Some(parent) = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
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
        corpus: sources.corpus,
        notes: sources.notes,
        ledger: PageLedger {
            entities: ledger.entities,
            on_page: ledger.on_page,
            intentional: ledger.intentional,
            suspect: ledger.suspect,
            unattributed: ledger.unattributed,
        },
    })
}

/// The page of a laid-out drawing as `pid_plot` writes it: set in the faces
/// its labels name, with `extra_face` embedded whole beside them when one is
/// given, and titled with the drawing's file name.
fn write_page(
    model: &::pid_plot::model::PaperModel,
    folders: &[PathBuf],
    extra_face: Option<&Path>,
    drawing: &Path,
) -> Result<::pid_plot::write::WrittenPlot, String> {
    let mut prepared = ::pid_plot::fonts::prepare_fonts(&[model], folders)?;
    if let Some(path) = extra_face {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        prepared.push(::pid_plot::fonts::PreparedFont {
            key: String::new(),
            bytes,
            subset: false,
        });
    }
    let fonts: Vec<::pid_plot::write::PlotFont<'_>> = prepared
        .iter()
        .map(|face| ::pid_plot::write::PlotFont {
            key: &face.key,
            bytes: &face.bytes,
            subset: face.subset,
        })
        .collect();
    let title = drawing.file_stem().map(|stem| stem.to_string_lossy());
    ::pid_plot::write::write_plot_titled(model, &fonts, title.as_deref())
        .map_err(|error| error.to_string())
}

/// [`plot_pdf`] on a worker thread: parsing the sheet and its template and
/// subsetting CJK faces takes seconds, which the UI thread must not wait out.
/// With a `project` stated it is [`plot_pdf_against`] that runs there.
pub async fn plot_pdf_in_background(
    drawing: PathBuf,
    target: PathBuf,
    project: Option<Project>,
) -> Result<PidPdfReceipt, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-pid-plot".into())
        // The same stack the app's own threads get (RUST_MIN_STACK in
        // .cargo/config.toml): the record walk recurses through nested
        // symbol and template structures.
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let plotted = match &project {
                Some(Project { folder, fonts }) => {
                    plot_pdf_against(&drawing, &target, folder, fonts)
                }
                None => plot_pdf(&drawing, &target),
            };
            let _ = sender.send(plotted);
        })
        .map_err(|error| format!("cannot start the plot: {error}"))?;
    receiver
        .await
        .unwrap_or_else(|_| Err("the plot stopped before it finished".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ocs-pid-pdf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in members {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

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
        let root = scratch("corpus");
        let drawings = root.join("Drawings").join("Unit 1");
        std::fs::create_dir_all(&drawings).unwrap();
        std::fs::create_dir_all(root.join("Ref").join("Symbols")).unwrap();
        let drawing = drawings.join("sheet.pid");
        assert_eq!(corpus_for(&drawing), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn without_ref_the_corpus_is_the_drawing_folder() {
        let root = scratch("plain");
        let drawing = root.join("sheet.pid");
        assert_eq!(corpus_for(&drawing), root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn templates_count_only_where_pid_plot_looks() {
        let root = scratch("templates");
        let project = root.join("plant");
        std::fs::create_dir_all(project.join("Ref").join("Symbols")).unwrap();
        assert!(!has_templates(&project));
        let files = project.join("Ref").join("Template Files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("CPE-SQ-A1.igr"), b"").unwrap();
        assert!(has_templates(&project));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_with_its_own_templates_keeps_pid_plots_lookup() {
        let root = scratch("own");
        let files = root.join("Ref").join("Template Files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("CPE-SQ-A1.igr"), b"").unwrap();
        let sources = sources_for(&root.join("sheet.pid"));
        assert_eq!(sources.corpus, root);
        assert!(sources.symbol_roots.is_none());
        assert!(sources.notes.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn symbol_roots_run_from_the_drawing_up_then_the_reference() {
        if std::env::var_os("PID_SYMBOL_LIBRARY").is_some() {
            return;
        }
        let root = scratch("roots");
        let project = root.join("acceptance");
        let own = project.join("Ref").join("Symbols");
        let reference = root.join("reference");
        let shared = reference.join("Ref").join("Symbols");
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        let roots = symbol_roots(&project.join("sheet.pid"), &reference);
        // The walk continues above the scratch folder, where the machine may
        // hold folders of its own; only the two ends are fixed.
        assert_eq!(roots.first(), Some(&own));
        assert_eq!(roots.last(), Some(&shared));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_zip_refdata_payloads_are_searched_for_templates() {
        let root = scratch("refdata");
        std::fs::write(root.join("RefData~4~680"), b"\"Begin Rules\"").unwrap();
        std::fs::write(root.join("Manifest.txt"), b"").unwrap();
        assert_eq!(refdata_archives(&root), vec![root.join("RefData~4~680")]);
        assert!(templates_in(&root.join("RefData~4~680")).is_none());
        assert_eq!(archived_templates(&root), Ok(None));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_template_read_out_of_an_archive_keeps_its_own_name() {
        let root = scratch("archived");
        let archive = root.join("RefData~4~682.zip");
        let members: &[(&str, &[u8])] = &[
            ("Template Files/CPE-SQ-A1.igr", b"frame"),
            ("Template Files/A1-W - New.igr", b"other"),
            ("Template Files/Readme.txt", b"no template"),
        ];
        write_zip(&archive, members);
        let _ = std::fs::remove_dir_all(template_cache(&archive));
        let (cache, from) = archived_templates(&root).unwrap().unwrap();
        assert_eq!(from, archive);
        let dir = cache.join("Ref").join("Template Files");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        // pid-plot gives a sheet whose frame links `CPE-SQ~2.IGR` only a file
        // whose name begins with `CPE-SQ`: under any other name the template
        // is one no sheet asks for.
        assert_eq!(names, ["A1-W - New.igr", "CPE-SQ-A1.igr"]);
        assert_eq!(std::fs::read(dir.join("CPE-SQ-A1.igr")).unwrap(), b"frame");
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cache_is_not_the_folder_an_earlier_layout_filled() {
        use std::hash::{Hash, Hasher};
        let root = scratch("layout");
        let archive = root.join("RefData~4~682.zip");
        let members: &[(&str, &[u8])] = &[("Template Files/CPE-SQ-A1.igr", b"frame")];
        write_zip(&archive, members);
        // The key as it was while templates were written under ranked names:
        // the archive's path, size and time, and nothing else. A folder under
        // that key may still hold `1 CPE-SQ-A1.igr`, and a sheet that names no
        // template is given the file of its size whose name sorts first.
        let mut hasher = std::hash::DefaultHasher::new();
        archive.hash(&mut hasher);
        let meta = std::fs::metadata(&archive).unwrap();
        meta.len().hash(&mut hasher);
        meta.modified().unwrap().hash(&mut hasher);
        let earlier = std::env::temp_dir()
            .join("ocs-pid-plot-templates")
            .join(format!("{:016x}", hasher.finish()));
        assert_ne!(template_cache(&archive), earlier);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn faces_are_looked_for_in_pid_plots_folders_then_the_callers_then_the_fallbacks() {
        // pid-plot's own list is taken as it is, a repeat included.
        let own = vec![
            PathBuf::from("named"),
            PathBuf::from("project/fonts"),
            PathBuf::from("named"),
        ];
        // The caller's second folder is one pid-plot searches already.
        let extra = [
            PathBuf::from("package/fonts"),
            PathBuf::from("project/fonts"),
        ];
        let folders = font_folders(
            own,
            &extra,
            Some(Path::new("reference")),
            Some(Path::new("windows")),
        );
        assert_eq!(
            folders,
            [
                PathBuf::from("named"),
                PathBuf::from("project/fonts"),
                PathBuf::from("named"),
                PathBuf::from("package/fonts"),
                Path::new("reference").join("fonts"),
                Path::new("windows").join("Fonts"),
            ]
        );
    }

    #[test]
    fn without_a_reference_or_windows_only_the_stated_folders_are_searched() {
        let own = vec![PathBuf::from("project/fonts")];
        let extra = [PathBuf::from("package/fonts")];
        assert_eq!(
            font_folders(own, &extra, None, None),
            [
                PathBuf::from("project/fonts"),
                PathBuf::from("package/fonts")
            ]
        );
    }

    #[test]
    fn a_stated_project_is_the_corpus_wherever_the_drawing_lies() {
        let root = scratch("stated");
        let project = root.join("restored");
        let files = project.join("Ref").join("Template Files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("CPE-SQ-A1.igr"), b"").unwrap();
        let package_fonts = [root.join("package").join("fonts")];
        // Not under the project: found from its own folder, the drawing's
        // corpus would be that folder.
        let drawing = root.join("elsewhere").join("sheet.pid");
        let sources = sources_in(&drawing, project.clone(), &package_fonts);
        assert_eq!(sources.corpus, project);
        assert!(sources.symbol_roots.is_none());
        assert!(sources.notes.is_empty());
        // pid-plot's own folders, the project's among them, then the caller's.
        let own = ::pid_plot::fonts::font_dirs(Some(&project));
        assert!(own.contains(&project.join("fonts")));
        assert_eq!(sources.fonts[..own.len()], own[..]);
        assert_eq!(sources.fonts[own.len()], package_fonts[0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A sheet with nothing on it: a page that is written without a drawing
    /// and without a face.
    fn blank_page() -> ::pid_plot::model::PaperModel {
        ::pid_plot::model::PaperModel {
            library: Vec::new(),
            page_mm: Some([420.0, 297.0]),
            frame_mm: None,
            template: None,
            primitives: Vec::new(),
            off_sheet_primitives: 0,
            skipped_endpoint_pairs: 0,
            missing_symbols: Vec::new(),
            library_gaps: Vec::new(),
            missing_templates: Vec::new(),
            missing_boxes: Vec::new(),
            symbols_found: 0,
            symbols_referenced: 0,
            ledger: ::pid_plot::ledger::LayoutLedger::default(),
        }
    }

    /// The document title `pdf` carries, as text.
    fn title_of(pdf: Vec<u8>) -> Option<String> {
        let pdf = hayro::hayro_syntax::Pdf::new(std::sync::Arc::new(pdf)).unwrap();
        let title = pdf.metadata().title.clone()?;
        Some(match title.strip_prefix(&[0xFE, 0xFF]) {
            Some(utf16) => {
                let (pairs, odd) = utf16.as_chunks::<2>();
                assert!(odd.is_empty(), "half a UTF-16 unit in the title");
                let units: Vec<u16> = pairs.iter().copied().map(u16::from_be_bytes).collect();
                String::from_utf16(&units).unwrap()
            }
            None => String::from_utf8(title).unwrap(),
        })
    }

    #[test]
    fn the_page_is_titled_with_the_drawings_file_name() {
        for name in ["D04", "EF井场 注采阀组工艺及自控流程图"] {
            let drawing = Path::new("plant").join(format!("{name}.pid"));
            let written = write_page(&blank_page(), &[], None, &drawing).unwrap();
            assert_eq!(title_of(written.pdf).as_deref(), Some(name));
        }
    }

    #[test]
    fn an_extra_face_that_cannot_be_read_stops_the_page() {
        let root = scratch("face");
        let face = root.join("absent.ttf");
        let error =
            write_page(&blank_page(), &[], Some(&face), Path::new("sheet.pid")).unwrap_err();
        assert!(error.starts_with("cannot read "), "{error}");
        assert!(error.contains("absent.ttf"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What `variable` names, else `default`: where the real-sample
    /// comparison finds its inputs on the machine it was set up on.
    fn real_input(variable: &str, default: impl FnOnce() -> PathBuf) -> PathBuf {
        std::env::var_os(variable).map_or_else(default, PathBuf::from)
    }

    /// `pid_plot` built from the commit `Cargo.toml` pins pid-plot to, where
    /// `scripts/sppid/build-pid-tools.ps1` leaves it.
    fn pinned_pid_plot() -> PathBuf {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let text = std::fs::read_to_string(manifest).unwrap();
        let rev = text
            .lines()
            .find(|line| line.starts_with("pid-plot "))
            .and_then(|line| line.split("rev = \"").nth(1))
            .and_then(|rest| rest.split('"').next())
            .expect("Cargo.toml pins pid-plot to a rev");
        PathBuf::from(r"D:\scratch\ocs-sppid\tools")
            .join(&rev[..7])
            .join("pid_plot.exe")
    }

    /// `ledger` the way `pid_plot` prints a page's account.
    fn account_line(ledger: PageLedger) -> String {
        format!(
            "ledger entities {} on_page {} intentional {} suspect {} unattributed {}",
            ledger.entities,
            ledger.on_page,
            ledger.intentional,
            ledger.suspect,
            ledger.unattributed
        )
    }

    /// Three sheets of the plant backup -- its second smallest, median and
    /// largest `.pid` -- plotted here twice, where the sheet lies
    /// ([`plot_pdf`]) and, from a copy lying outside it, against the restored
    /// folder as a stated project ([`plot_pdf_against`]), and by `pid_plot`
    /// of the pinned commit: the same restored drawing, against the corpus
    /// this module chose, with the font folders this module searches handed
    /// over as `PID_PLOT_FONTS`. The three pages must be one file, byte for
    /// byte, and the account both receipts carry must be the line `pid_plot`
    /// prints.
    ///
    /// The copy is then plotted on the worker thread, as PIDPLOT plots
    /// ([`plot_pdf_in_background`]): with the restored folder stated it must
    /// be that file once more, and with nothing stated the plot [`plot_pdf`]
    /// makes of the copy, whose corpus is found from where the copy lies and
    /// is not the restored folder.
    ///
    /// `OCS_PID_PDF_REAL_BACKUP` names another backup,
    /// `OCS_PID_PDF_REAL_PLOTTER` another `pid_plot`, `OCS_PID_PDF_REAL_OUT` a
    /// folder to keep both sets of pages in. A backup or plotter that is not
    /// there fails the test: a comparison that did not run must not read as
    /// one that passed.
    #[test]
    #[ignore = "plots real sheets and runs pid_plot beside them; run alone with --ignored --nocapture"]
    fn pid_pdf_real_sheets_are_the_bytes_pid_plot_writes() {
        const SHEETS: [&str; 3] = [
            "D04",
            "集注站 低压气及轻烃处理单元 轻烃分离器二工艺及自控流程图",
            "EF井场 注采阀组工艺及自控流程图",
        ];
        let backup = real_input("OCS_PID_PDF_REAL_BACKUP", || DEFAULT_REFERENCE_ROOT.into());
        let plotter = real_input("OCS_PID_PDF_REAL_PLOTTER", pinned_pid_plot);
        assert!(backup.is_dir(), "no backup at {}", backup.display());
        assert!(
            plotter.is_file(),
            "no pid_plot at {}: scripts/sppid/build-pid-tools.ps1 builds it",
            plotter.display()
        );
        let kept = std::env::var_os("OCS_PID_PDF_REAL_OUT").map(PathBuf::from);
        let root = kept.clone().unwrap_or_else(|| scratch("real"));
        let restored =
            crate::io::pid_backup::restore(&backup, Some(&root.join("restored"))).unwrap();
        let mut unlike = Vec::new();
        for name in SHEETS {
            let drawing = restored
                .drawings
                .iter()
                .find(|path| path.file_stem().is_some_and(|stem| stem == name))
                .unwrap_or_else(|| panic!("the backup holds no {name}.pid"));
            let ours = root.join("ours").join(format!("{name}.pdf"));
            let stated = root.join("stated").join(format!("{name}.pdf"));
            let theirs = root.join("pid_plot").join(format!("{name}.pdf"));
            let receipt = plot_pdf(drawing, &ours).unwrap();
            // A copy under a folder that holds no `Ref`: found from where it
            // lies, its corpus would not be the restored folder.
            let apart = root.join("apart").join(format!("{name}.pid"));
            std::fs::create_dir_all(root.join("apart")).unwrap();
            std::fs::copy(drawing, &apart).unwrap();
            let whole = plot_pdf_against(&apart, &stated, &restored.root, &[]).unwrap();
            // The copy on the worker thread, with the project and without.
            let alike = |one: &PidPdfReceipt, other: &PidPdfReceipt| {
                one.ledger == other.ledger && one.corpus == other.corpus && one.notes == other.notes
            };
            let page = |kind: &str| root.join(kind).join(format!("{name}.pdf"));
            let read = |kind: &str| std::fs::read(page(kind)).ok();
            let project = Project {
                folder: restored.root.clone(),
                fonts: Vec::new(),
            };
            let handed = iced::futures::executor::block_on(plot_pdf_in_background(
                apart.clone(),
                page("handed"),
                Some(project),
            ));
            let handed = handed.is_ok_and(|handed| alike(&handed, &whole))
                && read("handed").is_some_and(|bytes| read("stated") == Some(bytes));
            let alone = plot_pdf(&apart, &page("alone"));
            let left = iced::futures::executor::block_on(plot_pdf_in_background(
                apart.clone(),
                page("left"),
                None,
            ));
            let found = match &left {
                Ok(left) => left.corpus.display().to_string(),
                Err(why) => format!("none ({why})"),
            };
            let left = match (&left, &alone) {
                (Ok(left), Ok(alone)) => {
                    alike(left, alone)
                        && left.corpus != whole.corpus
                        && read("left").is_some_and(|bytes| read("alone") == Some(bytes))
                }
                (Err(left), Err(alone)) => left == alone,
                _ => false,
            };
            let fonts = std::env::join_paths(sources_for(drawing).fonts).unwrap();
            let run = std::process::Command::new(&plotter)
                .arg(drawing)
                .arg(&theirs)
                .arg(&receipt.corpus)
                .env("PID_PLOT_FONTS", &fonts)
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&run.stderr);
            assert!(run.status.success(), "pid_plot {name}: {said}");
            let told = String::from_utf8_lossy(&run.stdout);
            let account = account_line(receipt.ledger);
            let accounted = told.lines().any(|line| line == account);
            let agreed = whole.ledger == receipt.ledger
                && whole.corpus == receipt.corpus
                && whole.notes == receipt.notes;
            let ours = std::fs::read(&ours).unwrap();
            let stated = std::fs::read(&stated).unwrap();
            let theirs = std::fs::read(&theirs).unwrap();
            let verdict = match ours.iter().zip(&theirs).position(|(a, b)| a != b) {
                Some(at) => format!("first differ at byte {at}"),
                None if ours.len() == theirs.len() => "the same bytes".to_string(),
                None => "one is the start of the other".to_string(),
            };
            eprintln!(
                "{name}: here {} bytes, pid_plot {} bytes, {verdict}; corpus {}; notes {:?}",
                ours.len(),
                theirs.len(),
                receipt.corpus.display(),
                receipt.notes
            );
            eprintln!(
                "{name}: as a stated project {} bytes, same file {}, same receipt {agreed}; \
                 {account}, printed by pid_plot {accounted}",
                stated.len(),
                stated == theirs
            );
            eprintln!(
                "{name}: on the worker thread, the project stated: the stated plot {handed}; \
                 nothing stated: corpus {found}, the plot found from where the copy lies {left}"
            );
            if ours != theirs || stated != theirs || !agreed || !accounted || !handed || !left {
                unlike.push(name);
            }
        }
        if kept.is_none() {
            let _ = std::fs::remove_dir_all(&root);
        }
        assert!(
            unlike.is_empty(),
            "not the bytes pid_plot writes: {unlike:?}"
        );
    }

    /// The folder the vendor's sheets lie in when `OCS_PID_PDF_VENDOR` names
    /// none: where they are on the machine the plan was written on.
    const VENDOR_FOLDER: &str = r"D:\work\cad\pid-test-data\验收数据";

    /// The sheets of the vendor's folder -- every `<name>.pid` with the
    /// vendor's own print `<name>.pdf` beside it -- each plotted where it
    /// lies, as PIDPLOT plots it ([`plot_pdf`]), into one folder as
    /// `<name>.pdf`, with a `report.json` of what went into each page.
    /// `scripts/sppid/pdf-compare.ps1` then has `pid_plot_compare` read each
    /// page against the vendor's print, and holds the readings to the ones
    /// pid-plot recorded for its own pages (SPEC-sppid-pdf §6, criterion 8).
    ///
    /// `OCS_PID_PDF_VENDOR` names another folder, `OCS_PID_PDF_VENDOR_OUT` a
    /// folder to keep the pages and the report in. A folder that is not
    /// there or holds no such sheet fails the test, and so does a sheet that
    /// does not plot: pages that were not made must not read as compared.
    #[test]
    #[ignore = "plots the vendor's sheets for pid_plot_compare; run alone with --ignored --nocapture"]
    fn the_vendors_sheets_are_plotted_where_they_lie() {
        let folder = real_input("OCS_PID_PDF_VENDOR", || VENDOR_FOLDER.into());
        assert!(folder.is_dir(), "no folder at {}", folder.display());
        let kept = std::env::var_os("OCS_PID_PDF_VENDOR_OUT").map(PathBuf::from);
        let out = kept.clone().unwrap_or_else(|| scratch("vendor"));
        // What an earlier run left must not pass for this one's.
        let report = out.join("report.json");
        assert!(!report.exists(), "{} is there already", report.display());
        std::fs::create_dir_all(&out).unwrap();
        let mut sheets: Vec<PathBuf> = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_pid(path) && path.with_extension("pdf").is_file())
            .collect();
        sheets.sort();
        assert!(
            !sheets.is_empty(),
            "{} holds no sheet with a print beside it",
            folder.display()
        );
        let mut pages = Vec::new();
        let mut unplotted = Vec::new();
        for sheet in &sheets {
            let name = sheet.file_stem().unwrap().to_string_lossy().into_owned();
            let pdf = format!("{name}.pdf");
            let clock = std::time::Instant::now();
            let plotted = plot_pdf(sheet, &out.join(&pdf));
            let seconds = clock.elapsed().as_secs_f64();
            let page = match plotted {
                Ok(receipt) => {
                    eprintln!(
                        "{name}: {} bytes in {seconds:.1} s; corpus {}; notes {:?}",
                        receipt.bytes,
                        receipt.corpus.display(),
                        receipt.notes
                    );
                    let template = receipt.template.as_ref();
                    serde_json::json!({
                        "name": name,
                        "pdf": pdf,
                        "bytes": receipt.bytes,
                        "primitives": receipt.primitives,
                        "page_mm": receipt.page_mm,
                        "template": template.map(|template| template.display().to_string()),
                        "corpus": receipt.corpus.display().to_string(),
                        "symbols_found": receipt.symbols_found,
                        "symbols_referenced": receipt.symbols_referenced,
                        "missing_symbols": receipt.missing_symbols,
                        "notes": receipt.notes,
                        "ledger": account_line(receipt.ledger),
                        "seconds": seconds,
                    })
                }
                Err(why) => {
                    eprintln!("{name}: not plotted: {why}");
                    unplotted.push(name.clone());
                    serde_json::json!({ "name": name, "error": why, "seconds": seconds })
                }
            };
            pages.push(page);
        }
        let told = serde_json::json!({
            "folder": folder.display().to_string(),
            "debug_assertions": cfg!(debug_assertions),
            "sheets": pages,
        });
        let text = serde_json::to_string_pretty(&told).unwrap();
        std::fs::write(&report, text + "\n").unwrap();
        if kept.is_none() {
            let _ = std::fs::remove_dir_all(&out);
        }
        assert!(unplotted.is_empty(), "not plotted: {unplotted:?}");
    }
}

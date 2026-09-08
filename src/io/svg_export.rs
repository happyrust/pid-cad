// SVG export — writes a plotted page as SVG.
//
// What goes on the page is decided by `io::plot_emit::emit_plot_content`, the
// same traversal the PDF exporter runs (grouping, depth order, CTB, pens,
// dashes, hatch segments, glyph geometry). This module owns only the SVG half:
// `SvgSink`, which turns that operation stream into elements and presentation
// attributes, and `write_svg_page`, which wraps it in a document. There is no
// second copy of the plot traversal here, and this module never re-decides
// what the emitter already decided — it translates.
//
// Coordinates (D1/D2 of docs/plans/2026-09-07-dxf-to-svg-export.md). The
// emitter speaks PDF points with the origin bottom-left and Y up; SVG has Y
// down. The page therefore carries two nested transforms:
//
//   <g transform="matrix(1/k 0 0 -1/k 0 H)">   flip + points → mm  (F)
//     <g transform="matrix(a b c d e f)">      the plot CTM        (M)
//
// where k is `plot_emit::GEOMETRY_MM_TO_PT`. F is one ordinary affine
// transform: `translate(0,H) scale(1,-1)` and a per-point `H − y` are the same
// map, and mirroring reverses every ring's direction without changing which
// points non-zero or even-odd call inside. Glyphs are already mapped into
// world coordinates by the emitter, so they ride the same F and must not be
// flipped again. Two groups instead of one composed matrix because the seam
// between "page" and "drawing" is where a reader looks first.
//
// The document is mm at the root and unitless inside (D2): `width="297mm"`
// with `viewBox="0 0 297 210"`, so one user unit is one millimetre and
// `stroke-width="0.25"` is a quarter of a millimetre of paper — while
// `"0.25mm"` would be a CSS length converted *before* the viewBox scale and
// would not be. Pen widths and dash lengths stay in the emitter's points
// because they are written inside F, which scales them along with the
// geometry, exactly as the PDF CTM does.
//
// Not supported here: the plot stamp. It is device text in Helvetica
// (`PlotOp::BuiltinText`), not glyph geometry, and an SVG that names a font it
// does not carry renders differently on every machine — so `stamp: true` is
// refused rather than approximated.

use crate::io::plot_emit::{
    emit_plot_content, FillRule, LineCap, LineJoin, PlotAssets, PlotBlend, PlotOp, PlotPage,
    PlotPoint, PlotSink, GEOMETRY_MM_TO_PT,
};
use crate::io::plot_style::PlotStyleTable;
use crate::io::plot_types::PdfPageInput;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// PDF points → sheet mm. The reciprocal of the factor the emitter's geometry
/// went through, so a coordinate that started life as `mm * k` comes back to
/// the millimetre it was.
const PT_TO_MM: f64 = 1.0 / GEOMETRY_MM_TO_PT as f64;

/// Paper error budget for a serialised coordinate, in mm per axis (D2).
const COORD_BUDGET_MM: f64 = 0.001;

/// The stroke defaults the page group carries, so only the CTB's overrides
/// have to be written on individual elements. SVG's own defaults are butt /
/// miter / 4, which are not the plot's.
const DEFAULT_CAP: LineCap = LineCap::Round;
const DEFAULT_JOIN: LineJoin = LineJoin::Round;
/// PDF's default miter limit is 10 and SVG's is 4; say 10 out loud.
const MITER_LIMIT: &str = "10";

#[derive(Debug)]
pub enum SvgError {
    /// The page asks for something SVG cannot carry faithfully.
    Unsupported(&'static str),
    /// A coordinate, length or page dimension that cannot be serialised.
    Invalid(String),
    /// The page's text is incomplete and the caller asked not to be handed a
    /// drawing like that (R1).
    MissingGlyphs {
        count: usize,
        atlas_unavailable: bool,
        /// The wire and quad index of the first one lost.
        first: Option<(String, usize)>,
    },
    Io(std::io::Error),
    /// Which page of a multi-page job failed.
    Page {
        page: usize,
        source: Box<SvgError>,
    },
    /// A page could not be written after earlier pages had already been
    /// published. A file each is not a transaction (D3), so say what is on
    /// disk rather than pretend the job did not happen.
    #[cfg(not(target_arch = "wasm32"))]
    Partial {
        published: Vec<std::path::PathBuf>,
        failed: std::path::PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SvgError::Unsupported(what) => write!(f, "SVG export does not support {what}"),
            SvgError::Invalid(what) => write!(f, "cannot write SVG: {what}"),
            SvgError::MissingGlyphs {
                count,
                atlas_unavailable,
                first,
            } => {
                if *atlas_unavailable {
                    return write!(
                        f,
                        "the glyph atlas could not be read, so the page would \
                         have carried no text at all"
                    );
                }
                write!(f, "{count} glyph(s) are missing from the page")?;
                if let Some((wire, quad)) = first {
                    write!(f, ", first on '{wire}' at quad {quad}")?;
                }
                Ok(())
            }
            SvgError::Io(error) => write!(f, "cannot write SVG: {error}"),
            SvgError::Page { page, source } => write!(f, "page {page}: {source}"),
            #[cfg(not(target_arch = "wasm32"))]
            SvgError::Partial {
                published,
                failed,
                reason,
            } => {
                write!(f, "cannot write {}: {reason}", failed.display())?;
                if published.is_empty() {
                    write!(f, " (nothing was written)")
                } else {
                    write!(
                        f,
                        " ({} already written: {})",
                        published.len(),
                        published
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
        }
    }
}

impl std::error::Error for SvgError {}

impl From<std::io::Error> for SvgError {
    fn from(error: std::io::Error) -> Self {
        SvgError::Io(error)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SvgOptions {
    /// Decimal places for path coordinates. `None` derives them from the plot
    /// scale so a serialised point is within [`COORD_BUDGET_MM`] of the point
    /// the emitter produced; a larger scale magnifies the rounding and needs
    /// more digits.
    pub decimals: Option<usize>,
    /// What to do about text the glyph atlas could not supply (R1).
    pub missing_glyphs: MissingGlyphs,
}

/// A drawing whose text is quietly incomplete looks fine and is not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MissingGlyphs {
    /// Refuse the page and name the first quad that was lost.
    #[default]
    Refuse,
    /// Write the page anyway and count them in the report — for a caller who
    /// would rather have the drawing and the warning than nothing.
    Report,
}

/// What the writer had to do, for the caller to report or refuse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SvgReport {
    /// Drawing elements written (paths and the page background).
    pub elements: usize,
    /// Clip paths defined.
    pub clip_paths: usize,
    /// The file uses `mix-blend-mode: multiply` and needs a consumer that
    /// supports CSS blending — `merge_lines` output is wrong without it.
    pub needs_mix_blend_mode: bool,
    /// Glyph / fill meshes written as one outline (R5: no seams).
    pub mesh_outlines: usize,
    /// Meshes whose triangles did not form a clean manifold and were written
    /// as one path of triangle sub-paths instead.
    pub mesh_fallbacks: usize,
    /// Glyph quads the atlas had no geometry for. Non-zero only when the
    /// caller asked for [`MissingGlyphs::Report`]; otherwise the page is
    /// refused instead.
    pub missing_glyphs: usize,
}

/// Write one plotted page as an SVG document.
pub fn write_svg_page<W: std::io::Write>(
    page: &PlotPage<'_>,
    assets: &PlotAssets,
    options: &SvgOptions,
    out: &mut W,
) -> Result<SvgReport, SvgError> {
    let mut sink = SvgSink::new(page, options)?;
    let plot = emit_plot_content(page, assets, &mut sink)?;
    // Before a byte reaches the caller: the sink cannot see text that was
    // never drawn, so this is the only place that can refuse it (R1).
    if options.missing_glyphs == MissingGlyphs::Refuse && plot.text_is_incomplete() {
        return Err(SvgError::MissingGlyphs {
            count: plot.missing_glyphs,
            atlas_unavailable: plot.atlas_unavailable,
            first: plot.first_missing.map(|m| (m.wire, m.quad)),
        });
    }
    sink.report.missing_glyphs = plot.missing_glyphs;
    sink.finish(out)
}

/// `write_svg_page` into a `String`, for callers that want the document in
/// memory (the web build's download, tests).
pub fn svg_page_to_string(
    page: &PlotPage<'_>,
    assets: &PlotAssets,
    options: &SvgOptions,
) -> Result<(String, SvgReport), SvgError> {
    let mut bytes = Vec::new();
    let report = write_svg_page(page, assets, options, &mut bytes)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| SvgError::Invalid("the document is not valid UTF-8".into()))?;
    Ok((text, report))
}

// ── In memory (the web build) ─────────────────────────────────────────────

/// A plot job as one document in memory, for a caller with no filesystem to
/// write to: the web build hands the result to the browser as a download.
///
/// This wraps `write_svg_page`, the writer the files go through — it is not a
/// second one — with the same job-wide `plot_style` fallback (page-level CTB
/// still wins, `as_plot_page` decides that) and the same options, so the bytes
/// a browser downloads are the bytes `export_svg_pages` would have put on disk
/// for the same page. A test on the native side pins that.
///
/// One page only. SVG has no pages and a download has no numbering, so a
/// longer job is refused rather than quietly cut down to its first page. The
/// GUI plots the active view today, which is one page; what a multi-page web
/// plot should be — N downloads, or a zip — is decided when the GUI's layout
/// selection reaches SVG (§6 Q1 of docs/plans/2026-09-08-svg-export-next-steps.md).
pub fn svg_job_to_string(
    pages: &[PdfPageInput],
    plot_style: Option<&PlotStyleTable>,
    assets: &PlotAssets,
    options: &SvgOptions,
) -> Result<(String, SvgReport), SvgError> {
    let page = match pages {
        [page] => page,
        [] => return Err(SvgError::Invalid("no pages were selected".into())),
        more => {
            return Err(SvgError::Invalid(format!(
                "a download is one page and this job has {}; plot one layout at a time",
                more.len()
            )))
        }
    };
    svg_page_to_string(&page.as_plot_page(plot_style), assets, options)
}

// ── Files (D3) ────────────────────────────────────────────────────────────

/// Where one page went, and what it needed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct SvgPageOutcome {
    pub path: std::path::PathBuf,
    pub report: SvgReport,
    pub bytes: usize,
}

/// The result of writing a plot job.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct SvgBatch {
    pub pages: Vec<SvgPageOutcome>,
    /// Nothing was written: the pages were rendered and the paths resolved,
    /// but the files were not published.
    pub dry_run: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Default)]
pub struct SvgWriteOptions {
    pub svg: SvgOptions,
    /// Replace files that are already there. Off by default: a plot that
    /// silently overwrote last week's issue would be a bad afternoon.
    pub force: bool,
    /// Render and resolve the paths, write nothing. Unsupported options still
    /// fail — the point of the rehearsal is to find out before the files move.
    pub dry_run: bool,
}

/// SVG has no pages, so a multi-page job is a file each (D3).
///
/// One page keeps the name it was given. Several are numbered
/// `stem-001.svg`, `stem-002.svg`, … in request order — all of them, including
/// the first, so a script can enumerate them without special-casing.
#[cfg(not(target_arch = "wasm32"))]
pub fn page_paths(base: &Path, pages: usize) -> Vec<std::path::PathBuf> {
    if pages <= 1 {
        return vec![base.to_path_buf()];
    }
    let parent = base.parent().unwrap_or_else(|| Path::new(""));
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plot".into());
    let extension = base
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "svg".into());
    (1..=pages)
        .map(|n| parent.join(format!("{stem}-{n:03}.{extension}")))
        .collect()
}

/// Write a plot job: one SVG per page.
///
/// Every target path is resolved and checked before anything is written — for
/// clashes inside the batch (including two that differ only in case, which is
/// one file on Windows) and, unless `force`, for files that already exist.
/// Each page is then written to a temporary file next to its target and
/// renamed onto it, so a reader never sees half a document.
///
/// **A file each is not a transaction.** If page three cannot be written,
/// pages one and two stay on disk and the error names them; nothing is rolled
/// back and nothing left over from an earlier, longer run is deleted.
#[cfg(not(target_arch = "wasm32"))]
pub fn export_svg_pages(
    pages: &[PdfPageInput],
    plot_style: Option<&PlotStyleTable>,
    assets: &PlotAssets,
    base: &Path,
    options: &SvgWriteOptions,
) -> Result<SvgBatch, SvgError> {
    if pages.is_empty() {
        return Err(SvgError::Invalid("no pages were selected".into()));
    }
    let paths = page_paths(base, pages.len());

    if let Some((first, second)) = first_case_insensitive_clash(&paths) {
        return Err(SvgError::Invalid(format!(
            "pages {} and {} would both write {} (paths that differ only in \
             case are the same file on Windows)",
            first + 1,
            second + 1,
            paths[second].display()
        )));
    }
    if !options.force {
        let taken: Vec<String> = paths
            .iter()
            .filter(|p| p.exists())
            .map(|p| p.display().to_string())
            .collect();
        if !taken.is_empty() {
            return Err(SvgError::Invalid(format!(
                "these files already exist: {}",
                taken.join(", ")
            )));
        }
    }

    // `.svgz` is gzip, and only when the caller asked for it by name: a
    // consumer that is handed gzip bytes in a file called `.svg` has no way to
    // know, and half of them will not sniff for it.
    let compress = base
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svgz"));

    // Render every page before publishing any of them, so a page that cannot
    // be written at all — a stamp, a bad number — stops the job with nothing
    // on disk rather than half a set.
    let mut documents = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let mut bytes = Vec::new();
        let report = write_svg_page(
            &page.as_plot_page(plot_style),
            assets,
            &options.svg,
            &mut bytes,
        )
        .map_err(|error| {
            if pages.len() > 1 {
                SvgError::Page {
                    page: index + 1,
                    source: Box::new(error),
                }
            } else {
                error
            }
        })?;
        if compress {
            bytes = gzip(&bytes)?;
        }
        documents.push((bytes, report));
    }

    let mut outcomes = Vec::with_capacity(pages.len());
    for (path, (bytes, report)) in paths.iter().zip(documents) {
        if !options.dry_run {
            if let Err(error) = publish(path, &bytes) {
                return Err(SvgError::Partial {
                    published: outcomes
                        .iter()
                        .map(|o: &SvgPageOutcome| o.path.clone())
                        .collect(),
                    failed: path.clone(),
                    reason: error.to_string(),
                });
            }
        }
        outcomes.push(SvgPageOutcome {
            path: path.clone(),
            report,
            bytes: bytes.len(),
        });
    }
    Ok(SvgBatch {
        pages: outcomes,
        dry_run: options.dry_run,
    })
}

/// Show a parented SVG save-file dialog and return the chosen path.
///
/// Parented for the same reason the PDF one is: a parentless save dialog is
/// refused on some Wayland desktops (#537).
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_svg_path_owned(
    stem: String,
    parent: &dyn iced::window::Window,
) -> Option<std::path::PathBuf> {
    let path = crate::sys::blocking_file_dialog()
        .set_parent(parent)
        .set_title("Export as SVG")
        .set_file_name(format!("{stem}.svg"))
        .add_filter("SVG Files", &["svg"])
        .add_filter("All Files", &["*"])
        .save_file()?;
    crate::config::remember_dialog_dir(&path);
    Some(path)
}

/// The first pair of paths that name the same file on a case-insensitive
/// filesystem. Numbering cannot produce one today; a naming scheme that takes
/// the layout's name — the obvious next request — can.
#[cfg(not(target_arch = "wasm32"))]
pub fn first_case_insensitive_clash(paths: &[std::path::PathBuf]) -> Option<(usize, usize)> {
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        if let Some(first) = seen.insert(path.to_string_lossy().to_lowercase(), index) {
            return Some((first, index));
        }
    }
    None
}

/// The document, gzipped, for a `.svgz` target. A plotted sheet is mostly
/// digits and repeated tags, so this is worth about a third of the file.
#[cfg(not(target_arch = "wasm32"))]
fn gzip(bytes: &[u8]) -> Result<Vec<u8>, SvgError> {
    let mut encoder = flate2::write::GzEncoder::new(
        Vec::with_capacity(bytes.len() / 3),
        flate2::Compression::default(),
    );
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

/// Write `bytes` to a temporary file beside `path`, then rename it on.
#[cfg(not(target_arch = "wasm32"))]
fn publish(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = parent.join(format!(".{name}.part"));
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // Windows will not rename onto an existing file.
    let _ = std::fs::remove_file(path);
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

// ── PlotSink → SVG ────────────────────────────────────────────────────────

#[derive(Clone)]
struct State {
    stroke: [f32; 3],
    fill: [f32; 3],
    width_pt: f32,
    cap: LineCap,
    join: LineJoin,
    dash: Vec<i64>,
    dash_phase: i64,
    blend: PlotBlend,
    /// Groups opened since the enclosing `Save`, to close on `Restore`.
    open_groups: usize,
}

/// Turns the emitter's operation stream into SVG elements.
///
/// The body is built in memory rather than streamed: `<clipPath>`s are
/// discovered while walking the page but have to appear in `<defs>` before the
/// content that references them, and a forward reference is not something
/// every consumer resolves. Page-sized drawings are megabytes of text, so this
/// is a real cost and a candidate for the P3 size work (two passes, or defs
/// last for consumers known to cope).
struct SvgSink {
    body: String,
    defs: String,
    paper_w: f32,
    paper_h: f32,
    decimals: usize,
    state: State,
    stack: Vec<State>,
    /// The paint a run of elements shares, written once on a wrapping group
    /// instead of on every leaf. `None` = no run is open.
    style_run: Option<String>,
    report: SvgReport,
}

impl SvgSink {
    fn new(page: &PlotPage<'_>, options: &SvgOptions) -> Result<Self, SvgError> {
        let (w, h) = (page.paper_w, page.paper_h);
        if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
            return Err(SvgError::Invalid(format!("page size {w} × {h} mm")));
        }
        if !page.scale.is_finite() || page.scale <= 0.0 {
            return Err(SvgError::Invalid(format!("plot scale {}", page.scale)));
        }
        if let Some((cx, cy, cw, ch)) = page.clip {
            if ![cx, cy, cw, ch].iter().all(|v| v.is_finite()) || cw <= 0.0 || ch <= 0.0 {
                return Err(SvgError::Invalid(format!(
                    "clip rectangle {cx},{cy} {cw}×{ch} mm"
                )));
            }
        }
        Ok(Self {
            body: String::new(),
            defs: String::new(),
            paper_w: w,
            paper_h: h,
            decimals: options
                .decimals
                .unwrap_or_else(|| auto_decimals(page.scale as f64)),
            state: State {
                stroke: [0.0; 3],
                fill: [0.0; 3],
                width_pt: 1.0,
                cap: DEFAULT_CAP,
                join: DEFAULT_JOIN,
                dash: Vec::new(),
                dash_phase: 0,
                blend: PlotBlend::Normal,
                open_groups: 0,
            },
            stack: Vec::new(),
            style_run: None,
            report: SvgReport::default(),
        })
    }

    /// Assemble the document: header, the clip definitions collected while
    /// walking the page, the page groups, the body.
    fn finish<W: std::io::Write>(mut self, out: &mut W) -> Result<SvgReport, SvgError> {
        self.end_style_run();
        // The emitter balances its own Save/Restore; close anything left over
        // rather than write a malformed document.
        let dangling =
            self.state.open_groups + self.stack.iter().map(|s| s.open_groups).sum::<usize>();
        for _ in 0..dangling {
            self.body.push_str("</g>");
        }

        let (w, h) = (self.paper_w, self.paper_h);
        let mut head = String::with_capacity(512);
        head.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        head.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"");
        num(&mut head, w as f64, 4);
        head.push_str("mm\" height=\"");
        num(&mut head, h as f64, 4);
        head.push_str("mm\" viewBox=\"0 0 ");
        num(&mut head, w as f64, 4);
        head.push(' ');
        num(&mut head, h as f64, 4);
        head.push_str("\" preserveAspectRatio=\"xMidYMid meet\">\n");

        // Ink outside the sheet is clipped away, as the PDF media box does,
        // so an embedded page cannot paint over its neighbours.
        head.push_str("<defs><clipPath id=\"page-clip\" clipPathUnits=\"userSpaceOnUse\"><rect x=\"0\" y=\"0\" width=\"");
        num(&mut head, w as f64, 4);
        head.push_str("\" height=\"");
        num(&mut head, h as f64, 4);
        head.push_str("\"/></clipPath>");
        head.push_str(&self.defs);
        head.push_str("</defs>\n");

        head.push_str("<g clip-path=\"url(#page-clip)\">\n<g");
        if self.report.needs_mix_blend_mode {
            // Multiply must see the page's white background and the ink under
            // it as its backdrop, and nothing outside this document.
            head.push_str(" style=\"isolation:isolate\"");
        }
        head.push_str(" stroke-linecap=\"round\" stroke-linejoin=\"round\" stroke-miterlimit=\"");
        head.push_str(MITER_LIMIT);
        head.push_str("\" transform=\"matrix(");
        num(&mut head, PT_TO_MM, 12);
        head.push(',');
        head.push_str("0,0,");
        num(&mut head, -PT_TO_MM, 12);
        head.push_str(",0,");
        num(&mut head, h as f64, 4);
        head.push_str(")\">\n");

        out.write_all(head.as_bytes())?;
        out.write_all(self.body.as_bytes())?;
        out.write_all(b"\n</g>\n</g>\n</svg>\n")?;
        Ok(self.report)
    }

    /// The paint the next element will use, as SVG attributes.
    ///
    /// These are all inherited properties, so a run of elements that share
    /// them carries them once on a wrapping group; only what a leaf has to
    /// contradict (`fill="none"` on a stroke, `stroke="none"` on a fill) goes
    /// on the leaf. Attributes are a tenth of a plotted sheet's bytes and the
    /// emitter changes paint far less often than it draws, so the run is
    /// usually long.
    fn paint_attributes(&self) -> Result<String, SvgError> {
        finite(self.state.width_pt, "stroke width")?;
        let mut out = String::with_capacity(64);
        out.push_str(" stroke=\"");
        color(&mut out, self.state.stroke);
        out.push_str("\" fill=\"");
        color(&mut out, self.state.fill);
        out.push_str("\" stroke-width=\"");
        num(&mut out, self.state.width_pt as f64, self.decimals);
        out.push('"');
        if self.state.cap != DEFAULT_CAP {
            out.push_str(" stroke-linecap=\"");
            out.push_str(match self.state.cap {
                LineCap::Butt => "butt",
                LineCap::Round => "round",
                LineCap::Square => "square",
            });
            out.push('"');
        }
        if self.state.join != DEFAULT_JOIN {
            out.push_str(" stroke-linejoin=\"");
            out.push_str(match self.state.join {
                LineJoin::Miter => "miter",
                LineJoin::Round => "round",
                LineJoin::Bevel => "bevel",
            });
            out.push('"');
        }
        if !self.state.dash.is_empty() {
            out.push_str(" stroke-dasharray=\"");
            for (i, length) in self.state.dash.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                let _ = write!(out, "{length}");
            }
            out.push('"');
            if self.state.dash_phase != 0 {
                let _ = write!(out, " stroke-dashoffset=\"{}\"", self.state.dash_phase);
            }
        } else {
            // The run this replaces may have left a dash pattern behind.
            out.push_str(" stroke-dasharray=\"none\"");
        }
        Ok(out)
    }

    /// Open the group the next element belongs to, if the paint changed.
    fn begin_style_run(&mut self) -> Result<(), SvgError> {
        let paint = self.paint_attributes()?;
        if self.style_run.as_deref() == Some(paint.as_str()) {
            return Ok(());
        }
        self.end_style_run();
        self.body.push_str("<g");
        self.body.push_str(&paint);
        self.body.push_str(">\n");
        self.style_run = Some(paint);
        Ok(())
    }

    /// Close the paint group, before anything that changes the structure —
    /// a save, a transform, a clip — so a run never straddles one.
    fn end_style_run(&mut self) {
        if self.style_run.take().is_some() {
            self.body.push_str("</g>\n");
        }
    }

    /// `mix-blend-mode` does not inherit, so it goes on the leaf that draws —
    /// and as a `style=`, because librsvg only reads the CSS property, not a
    /// same-named XML attribute.
    fn blend_attribute(&mut self) {
        if self.state.blend == PlotBlend::Multiply {
            self.body.push_str(" style=\"mix-blend-mode:multiply\"");
            self.report.needs_mix_blend_mode = true;
        }
    }

    /// `M x,y L x,y x,y …`, one sub-path per ring. `close` appends `Z`.
    fn path_data(&mut self, rings: &[&[PlotPoint]], close: bool) -> Result<(), SvgError> {
        self.body.push_str(" d=\"");
        let decimals = self.decimals;
        for ring in rings {
            let Some((first, rest)) = ring.split_first() else {
                continue;
            };
            self.body.push('M');
            point(&mut self.body, *first, decimals)?;
            if !rest.is_empty() {
                self.body.push('L');
                for (i, p) in rest.iter().enumerate() {
                    if i > 0 {
                        self.body.push(' ');
                    }
                    point(&mut self.body, *p, decimals)?;
                }
            }
            if close {
                self.body.push('Z');
            }
        }
        self.body.push('"');
        Ok(())
    }
}

impl PlotSink for SvgSink {
    type Error = SvgError;

    fn emit(&mut self, op: PlotOp) -> Result<(), Self::Error> {
        match op {
            PlotOp::Save => {
                self.end_style_run();
                self.stack.push(self.state.clone());
                self.state.open_groups = 0;
            }
            PlotOp::Restore => {
                self.end_style_run();
                for _ in 0..self.state.open_groups {
                    self.body.push_str("</g>");
                }
                self.state = self
                    .stack
                    .pop()
                    .ok_or(SvgError::Invalid("unbalanced graphics state".into()))?;
            }
            PlotOp::Concat(m) => {
                self.end_style_run();
                for v in m {
                    finite(v, "transform")?;
                }
                self.body.push_str("<g transform=\"matrix(");
                for (i, v) in m.iter().enumerate() {
                    if i > 0 {
                        self.body.push(',');
                    }
                    // Linear coefficients decide where everything lands; give
                    // them their own precision instead of the coordinates'.
                    num(&mut self.body, *v as f64, 9);
                }
                self.body.push_str(")\">");
                self.state.open_groups += 1;
            }
            PlotOp::Blend(blend) => self.state.blend = blend,
            PlotOp::LineCap(cap) => self.state.cap = cap,
            PlotOp::LineJoin(join) => self.state.join = join,
            PlotOp::StrokeColor(c) => self.state.stroke = c,
            PlotOp::FillColor(c) => self.state.fill = c,
            PlotOp::StrokeWidthPt(w) => self.state.width_pt = w,
            PlotOp::Dash { lengths, phase } => {
                self.state.dash = lengths;
                self.state.dash_phase = phase;
            }
            PlotOp::Clip { rings, rule } => {
                self.end_style_run();
                let id = format!("clip-{}", self.report.clip_paths + 1);
                self.report.clip_paths += 1;
                let mut data = String::new();
                std::mem::swap(&mut self.body, &mut data);
                let refs: Vec<&[PlotPoint]> = rings.iter().map(|r| r.as_slice()).collect();
                self.path_data(&refs, true)?;
                std::mem::swap(&mut self.body, &mut data);
                let _ = write!(
                    self.defs,
                    "<clipPath id=\"{id}\" clipPathUnits=\"userSpaceOnUse\"><path{}{data}/></clipPath>",
                    if rule == FillRule::EvenOdd {
                        " clip-rule=\"evenodd\""
                    } else {
                        ""
                    }
                );
                let _ = write!(self.body, "<g clip-path=\"url(#{id})\">");
                self.state.open_groups += 1;
            }
            PlotOp::FillRect {
                x,
                y,
                width,
                height,
            } => {
                for v in [x, y, width, height] {
                    finite(v, "rectangle")?;
                }
                self.begin_style_run()?;
                self.body.push_str("<rect stroke=\"none\" x=\"");
                let d = self.decimals;
                num(&mut self.body, x as f64, d);
                self.body.push_str("\" y=\"");
                num(&mut self.body, y as f64, d);
                self.body.push_str("\" width=\"");
                num(&mut self.body, width as f64, d);
                self.body.push_str("\" height=\"");
                num(&mut self.body, height as f64, d);
                self.body.push('"');
                self.blend_attribute();
                self.body.push_str("/>\n");
                self.report.elements += 1;
            }
            PlotOp::Stroke { points, closed } => {
                self.begin_style_run()?;
                self.body.push_str("<path fill=\"none\"");
                self.blend_attribute();
                self.path_data(&[points.as_slice()], closed)?;
                self.body.push_str("/>\n");
                self.report.elements += 1;
            }
            PlotOp::Fill { rings, rule } => {
                self.begin_style_run()?;
                self.body.push_str("<path stroke=\"none\"");
                if rule == FillRule::EvenOdd {
                    self.body.push_str(" fill-rule=\"evenodd\"");
                }
                self.blend_attribute();
                let refs: Vec<&[PlotPoint]> = rings.iter().map(|r| r.as_slice()).collect();
                self.path_data(&refs, true)?;
                self.body.push_str("/>\n");
                self.report.elements += 1;
            }
            PlotOp::FillMesh { tris } => {
                // One shape, one path: filling the triangles separately leaves
                // a lighter seam along every shared edge wherever the renderer
                // anti-aliases per shape (R5). Preferred form is the outline
                // with the interior edges cancelled out; a mesh that is not a
                // clean manifold still goes out as a single path, whose
                // sub-paths are rasterised together, rather than as N shapes.
                let outline = mesh_outline(&tris);
                match &outline {
                    Some(_) => self.report.mesh_outlines += 1,
                    None => self.report.mesh_fallbacks += 1,
                }
                let rings: Vec<&[PlotPoint]> = match &outline {
                    Some(rings) => rings.iter().map(|r| r.as_slice()).collect(),
                    None => tris.iter().map(|t| t.as_slice()).collect(),
                };
                if rings.is_empty() {
                    return Ok(());
                }
                self.begin_style_run()?;
                self.body.push_str("<path stroke=\"none\"");
                self.blend_attribute();
                self.path_data(&rings, true)?;
                self.body.push_str("/>\n");
                self.report.elements += 1;
            }
            PlotOp::BuiltinText { .. } => {
                return Err(SvgError::Unsupported(
                    "the plot stamp: it is device text in a built-in font, not geometry",
                ))
            }
        }
        Ok(())
    }
}

// ── Mesh outlines (R5) ────────────────────────────────────────────────────

/// Bit pattern of a point, so shared mesh vertices match exactly.
///
/// The triangles of one glyph all come from the same affine map applied to the
/// same glyph-space vertices, so two triangles that share a corner carry
/// identical f32s — no epsilon welding, which at UTM-scale sheet coordinates
/// would either miss real joins or swallow small counters.
fn vertex_key(p: PlotPoint) -> u64 {
    ((p.x.to_bits() as u64) << 32) | p.y.to_bits() as u64
}

/// The boundary rings of a triangle mesh, or `None` if the triangles do not
/// form a clean manifold (a shared edge traversed twice the same way, a vertex
/// where several boundary edges meet, a T-junction, a degenerate triangle).
///
/// Interior edges appear once in each direction and cancel; what is left is
/// the outline plus one ring per hole, each keeping the winding the mesh gave
/// it, so a non-zero fill reproduces the counters of a glyph like "o".
fn mesh_outline(tris: &[[PlotPoint; 3]]) -> Option<Vec<Vec<PlotPoint>>> {
    if tris.is_empty() {
        return None;
    }
    let mut vertices: HashMap<u64, PlotPoint> = HashMap::new();
    let mut edges: HashSet<(u64, u64)> = HashSet::new();
    for tri in tris {
        let keys = [vertex_key(tri[0]), vertex_key(tri[1]), vertex_key(tri[2])];
        if keys[0] == keys[1] || keys[1] == keys[2] || keys[2] == keys[0] {
            return None;
        }
        for (i, &key) in keys.iter().enumerate() {
            vertices.insert(key, tri[i]);
            let edge = (key, keys[(i + 1) % 3]);
            if !edges.remove(&(edge.1, edge.0)) && !edges.insert(edge) {
                return None;
            }
        }
    }
    if edges.is_empty() {
        return None;
    }
    // Exactly one boundary edge may leave each boundary vertex; more than one
    // is a pinch point and the walk below would have to guess.
    let mut next: HashMap<u64, u64> = HashMap::with_capacity(edges.len());
    for &(from, to) in &edges {
        if next.insert(from, to).is_some() {
            return None;
        }
    }
    // Ring order follows first appearance in the mesh, so the same glyph
    // always serialises the same way.
    let mut starts: Vec<u64> = Vec::with_capacity(next.len());
    let mut seen: HashSet<u64> = HashSet::with_capacity(next.len());
    for tri in tris {
        for corner in tri {
            let key = vertex_key(*corner);
            if next.contains_key(&key) && seen.insert(key) {
                starts.push(key);
            }
        }
    }
    let mut unvisited: HashSet<u64> = next.keys().copied().collect();
    let mut rings = Vec::new();
    for start in starts {
        if !unvisited.contains(&start) {
            continue;
        }
        let mut ring = Vec::new();
        let mut at = start;
        loop {
            if !unvisited.remove(&at) {
                return None;
            }
            ring.push(*vertices.get(&at)?);
            at = *next.get(&at)?;
            if at == start {
                break;
            }
        }
        if ring.len() < 3 {
            return None;
        }
        rings.push(ring);
    }
    unvisited.is_empty().then_some(rings)
}

// ── Serialisation ─────────────────────────────────────────────────────────

/// Decimals that keep a serialised coordinate within the paper error budget.
///
/// A coordinate is written in the pre-CTM space, so its rounding error reaches
/// the sheet multiplied by the plot scale and by points → mm: the written
/// error `½·10⁻ᵈ` must satisfy `½·10⁻ᵈ · s · PT_TO_MM ≤ budget`.
///
/// Nine tenths of a plotted sheet's bytes are coordinates, so the budget is
/// what decides this and not a round number: at 1:1 it asks for three
/// decimals, and a digit per coordinate is about a tenth of the file. A scale
/// above 1 magnifies the rounding and buys more digits back.
fn auto_decimals(scale: f64) -> usize {
    let factor = scale.max(1.0) * PT_TO_MM;
    let needed = (factor / (2.0 * COORD_BUDGET_MM)).log10().ceil();
    (needed.max(1.0) as usize).min(9)
}

/// A number with at most `decimals` places, trailing zeros and a trailing
/// point removed, `-0` normalised to `0`, and always a `.` for the decimal
/// point (Rust's float formatting is locale-independent, which is the point).
fn num(out: &mut String, value: f64, decimals: usize) {
    let start = out.len();
    let _ = write!(out, "{value:.decimals$}");
    let text = &out[start..];
    let trimmed = if text.contains('.') {
        let t = text.trim_end_matches('0');
        t.strip_suffix('.').unwrap_or(t)
    } else {
        text
    };
    let len = if trimmed == "-0" || trimmed.is_empty() {
        out.truncate(start);
        out.push('0');
        return;
    } else {
        trimmed.len()
    };
    out.truncate(start + len);
}

fn point(out: &mut String, p: PlotPoint, decimals: usize) -> Result<(), SvgError> {
    finite(p.x, "coordinate")?;
    finite(p.y, "coordinate")?;
    num(out, p.x as f64, decimals);
    out.push(',');
    num(out, p.y as f64, decimals);
    Ok(())
}

fn finite(value: f32, what: &str) -> Result<(), SvgError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(SvgError::Invalid(format!("non-finite {what} ({value})")))
    }
}

/// `#rrggbb`. The emitter's colours are already the plotted ones — screening
/// and the `transparency` option are pre-mixed against white upstream, so
/// there is no alpha to carry and none is written.
fn color(out: &mut String, rgb: [f32; 3]) {
    let [r, g, b] = channels(rgb);
    let _ = write!(out, "#{r:02x}{g:02x}{b:02x}");
}

fn channels([r, g, b]: [f32; 3]) -> [u8; 3] {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [channel(r), channel(g), channel(b)]
}

#[cfg(test)]
mod tests;

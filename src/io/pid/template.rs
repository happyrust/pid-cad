// The border template: a P&ID's frame, title block and margin strips are an
// OLE-linked `.igr` drawing of their own, not sheet records -- `page.rs` only
// ever drew the page rectangle because the link went nowhere. This module
// resolves the link to a local `.igr`, parses it through the same `pid-parse`
// pipeline the drawing itself went through, and overlays its drafting on
// `PID-FRAME`.
//
// Which file a link means is `pid-plot`'s rule (its `model.rs`:
// `linked_template_prefix`, `load_border_template`), so the canvas and the
// plotted PDF wear the same template. A link is an authoring-machine path,
// usually in 8.3 short form (`...\TEMPLA~1\CPE-SQ~2.IGR`): its last component,
// cut at the `~` and upper-cased, is a prefix of the file it names. Only
// files whose name starts with a prefix the sheet spells are candidates, the
// page chooses among them, and a sheet that names a template never wears one
// it does not name -- when no file answers, no border is the honest answer.

use super::*;

/// Environment override listing directories holding the linked sheet
/// templates (`.igr`), `;`-separated like PATH -- the analogue of
/// [`SYMBOL_LIBRARY_ENV`] for the border rather than the symbols. Without it
/// the importer looks for the template next to the drawing the same way
/// [`discover_symbol_library`] looks for the symbol share: a SmartPlant
/// project keeps drawings and reference data under one root.
const TEMPLATE_LIBRARY_ENV: &str = "PID_TEMPLATE_LIBRARY";

/// Deepest the basename search recurses below a template root. `Template
/// Files` holds its `.igr` flat; the bound is for a root that turns out to
/// be a whole reference share.
const TEMPLATE_SEARCH_DEPTH: usize = 6;

/// How far a frame the template drew may differ from the drawing's stated
/// page and still be that page's frame -- the one the synthetic border would
/// double: millimetres, generous enough for the float noise between two
/// frame records of the same sheet size.
const PAGE_MATCH_TOLERANCE_MM: f64 = 2.0;

/// How far the strokes of a template the sheet names may sit from the
/// sheet's page, millimetres on the worse axis (`pid-plot`'s
/// `TEMPLATE_NAMED_TOLERANCE_MM`). The name is the sheet's own statement of
/// which template it wears, so a frame left a few millimetres larger than
/// the template's drafting is slack, not another size: the plant backup's
/// `N井场` states 843.46 x 596.71 against an 841 x 594 template. Two
/// neighbouring A-series sheets differ by 123 mm or more on that axis.
const NAMED_TEMPLATE_TOLERANCE_MM: f64 = 10.0;

/// Extents closer than this rank as the same extent (`pid-plot`'s
/// `TEMPLATE_EXTENT_QUANTUM_MM`): two templates authored for one sheet size
/// differ in the last bits of their `f64` metres, and which is nearer the
/// page's own noise is an accident the file name settles instead.
const TEMPLATE_RANK_QUANTUM_MM: f64 = 1e-3;

/// The five ISO A-series sheets, long side first, millimetres: what a sheet
/// that states no page is measured against.
const STANDARD_SHEETS_MM: [(f64, f64); 5] = [
    (297.0, 210.0),
    (420.0, 297.0),
    (594.0, 420.0),
    (841.0, 594.0),
    (1189.0, 841.0),
];

/// How far past a standard sheet's edge strokes may reach and still be
/// called that sheet, millimetres.
const SHEET_FIT_SLACK_MM: f64 = 5.0;

/// What the linked border template added to the drawing, for the summary
/// and for [`draw_page_border`]'s don't-double-the-frame check.
#[derive(Debug)]
pub(super) struct TemplateOverlay {
    /// The `.igr` file the overlay drew from.
    pub(super) file: PathBuf,
    /// Entities handed to the document.
    pub(super) drawn: usize,
    /// Template records that were not drawn: the ones the parser saw but
    /// has no decoder for, or in a shape the decoder for that type refuses,
    /// and the symbol placements no body answered for (the border wears no
    /// marker dot; see [`overlay_template`]). Counted beside the drawing's
    /// own `missing`, never folded into it.
    pub(super) missing: usize,
    /// The template drew a frame covering the drawing's stated page, so the
    /// synthetic border rectangle would double it.
    pub(super) page_frame_covered: bool,
}

/// Overlay the sheet's linked border template onto `doc`, if a file its
/// links name can be found, fits the page and parses.
///
/// `None` is a silent-sheet answer only when the file names no template at
/// all; a name no file on disk answers, or answers only in another size, is
/// said in the log. The drawing imports identically with or without the
/// overlay -- the template is the frame around the drawing, never a
/// prerequisite.
pub(super) fn overlay_linked_template(
    parsed: &pid_parse::PidDocument,
    geometry: &NormalizedPidGeometry,
    unit: &ImportUnit,
    drawing: &Path,
    doc: &mut Document,
    paper_style_scale: f64,
) -> Option<TemplateOverlay> {
    let links = linked_templates(parsed);
    if linked_template_prefixes(&links).is_empty() {
        return None;
    }
    // One link arrives as two strings, with its path and bare; the log names
    // it once.
    let mut named: Vec<&str> = Vec::new();
    for link in &links {
        let name = template_basename(link);
        if !named.contains(&name) {
            named.push(name);
        }
    }
    let named = named.join(", ");
    let Some(page_mm) = sheet_page_mm(geometry, unit) else {
        log::warn!(
            "{}: names border template(s) {named}, but states no page and its strokes fit no standard sheet to choose one by; the sheet imports without its title block and margin strips",
            drawing.display()
        );
        return None;
    };
    let page = format!(
        "{:.2} x {:.2} mm page{}",
        page_mm.0,
        page_mm.1,
        if geometry.page_dimensions_mm.is_some() {
            ""
        } else {
            " (not stated: the standard sheet its strokes fit)"
        }
    );
    // Only the files the links name are read, and none of them touches the
    // document until one is picked: a candidate that loses leaves no
    // linetype or text style behind.
    let mut templates: Vec<ReadTemplate> = named_template_files(&links, &template_roots(drawing))
        .into_iter()
        .filter_map(|(file, stated)| read_template(&file, stated))
        .collect();
    let candidates: Vec<TemplateCandidate> =
        templates.iter().map(ReadTemplate::candidate).collect();
    let found = candidates
        .iter()
        .map(|candidate| match candidate.stroked_mm {
            Some((width, height)) => {
                format!("{} ({width:.2} x {height:.2} mm)", candidate.file_name)
            }
            None => format!("{} (no strokes)", candidate.file_name),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let Some(picked) = pick_linked_template(&candidates, &links, page_mm) else {
        let why = if candidates.is_empty() {
            "no file under the template folders has such a name".to_string()
        } else {
            format!("none of the files so named fits its {page}: {found}")
        };
        log::warn!(
            "{}: names border template(s) {named}, but {why}; the sheet imports without its title block and margin strips",
            drawing.display()
        );
        return None;
    };
    let template = templates.swap_remove(picked);
    log::info!(
        "{}: border template {} for the link(s) {named}, on its {page}; files so named: {found}",
        drawing.display(),
        template.file.display()
    );
    overlay_template(&template, geometry, doc, paper_style_scale)
}

/// The distinct template targets the drawing's `JSite` OLE streams name, as
/// the raw link strings (`C:\...\CPE-SQ-A1.igr` and a bare
/// `CPE-SQ-A1.igr` arrive as separate strings of one link). A sheet links
/// its template's exported `.igr`, the template's own drawing (`.pid`), or
/// both; either names the template ([`linked_template_prefix`]).
pub(super) fn linked_templates(parsed: &pid_parse::PidDocument) -> Vec<String> {
    let mut links: Vec<String> = parsed
        .jsites
        .iter()
        .flat_map(|site| site.ole_links.iter())
        .filter(|link| linked_template_prefix(link).is_some())
        .map(|link| link.trim().to_string())
        .collect();
    links.sort();
    links.dedup();
    links
}

/// The file name a link ends in -- `C:\PID_SQProject\...\Template Files\
/// CPE-SQ-A1.igr` and `CPE-SQ-A1.igr` both answer `CPE-SQ-A1.igr`. Split on
/// either separator so the path reads the same whatever the linking machine
/// ran on.
pub(super) fn template_basename(link: &str) -> &str {
    link.trim()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .trim()
}

/// The stem a link spells, as one a template's file name can begin with:
/// the last component of the path, cut at the `~`, upper-cased.
///
/// The link is an authoring-machine path, usually through Windows' 8.3 short
/// name -- `C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~2.IGR` is the file this
/// machine has as `CPE-SQ-A1.igr`, and the six characters before the `~` are
/// all of the name that survived (`CPE-SQ`). A link authored where long names
/// were kept spells the whole stem (`CPECCHB-SQ-A2`). Either way the stem is
/// a prefix of the file it names, and `CPE-SQ` is no prefix of
/// `CPECCHB-SQ-A1`: that tells two template families of one sheet size
/// apart, which no measurement can.
///
/// The link may be to the template's drawing (`CPE-SQ~2.PID`) rather than
/// its exported `.igr`; both spell the same stem. Any other target -- a
/// symbol, a spreadsheet -- names no template.
pub(super) fn linked_template_prefix(link: &str) -> Option<String> {
    let (stem, extension) = template_basename(link).rsplit_once('.')?;
    if !extension.eq_ignore_ascii_case("igr") && !extension.eq_ignore_ascii_case("pid") {
        return None;
    }
    let prefix = stem.split('~').next().unwrap_or(stem);
    (!prefix.is_empty()).then(|| prefix.to_ascii_uppercase())
}

/// Every template stem the sheet's links spell, each once, in the links'
/// order. A sheet may wear any template it names.
pub(super) fn linked_template_prefixes(links: &[String]) -> Vec<String> {
    let mut prefixes: Vec<String> = Vec::new();
    for prefix in links.iter().filter_map(|link| linked_template_prefix(link)) {
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
    prefixes
}

/// Every whole file stem the links spell, upper-cased: the `.igr` links
/// whose name Windows did not cut. `CPE-SQ~2.IGR` is not a whole name;
/// `XIONGANA2.igr` is.
fn linked_template_stems(links: &[String]) -> Vec<String> {
    let mut stems: Vec<String> = Vec::new();
    for link in links {
        let Some((stem, extension)) = template_basename(link).rsplit_once('.') else {
            continue;
        };
        if !extension.eq_ignore_ascii_case("igr") || stem.is_empty() || stem.contains('~') {
            continue;
        }
        let stem = stem.to_ascii_uppercase();
        if !stems.contains(&stem) {
            stems.push(stem);
        }
    }
    stems
}

/// The stem of a template file's name, upper-cased the way the links' stems
/// are.
fn template_stem(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .to_ascii_uppercase()
}

/// Whether a file of this name is one the prefixes name: its stem starts
/// with one of them. This decides eligibility, not just order.
fn template_named_by(file_name: &str, prefixes: &[String]) -> bool {
    let stem = template_stem(file_name);
    prefixes
        .iter()
        .any(|prefix| stem.starts_with(prefix.as_str()))
}

/// What the pick needs to know of one template file found on this machine.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TemplateCandidate {
    /// The file's name on disk, extension included.
    pub(super) file_name: String,
    /// The file is the very path a link states, opened on this machine.
    pub(super) stated: bool,
    /// Width and height its strokes span, millimetres; `None` for a file
    /// that draws no stroke at all.
    pub(super) stroked_mm: Option<(f64, f64)>,
}

/// Which of `candidates` a sheet that links `links` wears on a page of
/// `page_mm`: the index of the pick, or `None` for no border.
///
/// Only a file the links name is eligible -- its stem starts with a prefix
/// one of them spells ([`linked_template_prefix`]) -- so a sheet that names
/// nothing, or whose names no candidate answers, wears nothing: another
/// family's template of the right size would letter another company into
/// the title block. Among the eligible:
///
/// - a file whose whole stem a link spells, or the file a link's own path
///   opens, is the sheet's statement of the very file it wears. It is taken
///   without measuring it and ahead of every other candidate;
/// - any other must have strokes to measure, within
///   [`NAMED_TEMPLATE_TOLERANCE_MM`] of the page on the worse axis;
/// - the nearer extent wins, then the file name, then the earlier
///   candidate, so the pick is the same one twice.
pub(super) fn pick_linked_template(
    candidates: &[TemplateCandidate],
    links: &[String],
    page_mm: (f64, f64),
) -> Option<usize> {
    let prefixes = linked_template_prefixes(links);
    let stems = linked_template_stems(links);
    let mut best: Option<((bool, f64, String), usize)> = None;
    for (index, candidate) in candidates.iter().enumerate() {
        let spelled = candidate.stated || stems.contains(&template_stem(&candidate.file_name));
        if !spelled && !template_named_by(&candidate.file_name, &prefixes) {
            continue;
        }
        let error = candidate.stroked_mm.map_or(0.0, |(width, height)| {
            (width - page_mm.0).abs().max((height - page_mm.1).abs())
        });
        if !spelled && (candidate.stroked_mm.is_none() || error > NAMED_TEMPLATE_TOLERANCE_MM) {
            continue;
        }
        let rank = (
            !spelled,
            (error / TEMPLATE_RANK_QUANTUM_MM).round(),
            candidate.file_name.to_ascii_lowercase(),
        );
        if best
            .as_ref()
            .is_some_and(|(best_rank, _)| *best_rank <= rank)
        {
            continue;
        }
        best = Some((rank, index));
    }
    best.map(|(_, index)| index)
}

/// Where the template for `link` actually is: the path the link states when
/// it exists on this machine, else a basename search under the template
/// roots.
pub(super) fn resolve_template_path(link: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let stated = Path::new(link.trim());
    if stated.is_file() {
        return Some(stated.to_path_buf());
    }
    let name = template_basename(link);
    if name.is_empty() {
        return None;
    }
    for root in roots {
        if let Some(found) = find_named(root, name, 0) {
            return Some(found);
        }
    }
    None
}

/// The directories a linked template can live under: [`TEMPLATE_LIBRARY_ENV`]
/// first, then the `*Template*` trees beside the drawing, found the way
/// [`discover_symbol_library`] finds the symbol share -- walking up, and
/// checking `Ref` at each level since that is where SmartPlant parks
/// reference data.
fn template_roots(drawing: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(list) = std::env::var_os(TEMPLATE_LIBRARY_ENV) {
        roots.extend(std::env::split_paths(&list).filter(|root| root.is_dir()));
    }
    let mut dir = drawing.parent();
    for _ in 0..SYMBOL_SEARCH_DEPTH {
        let Some(at) = dir else { break };
        for holder in [at.to_path_buf(), at.join("Ref")] {
            for candidate in template_dirs_in(&holder) {
                if !roots.contains(&candidate) {
                    roots.push(candidate);
                }
            }
        }
        dir = at.parent();
    }
    roots
}

/// The directories directly inside `dir` whose name says they hold
/// templates. Matched on the word the way [`symbol_roots_in`] matches
/// "symbol": a local copy of the share is rarely named exactly `Template
/// Files`.
fn template_dirs_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains("template")
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// The file named `name` under `dir`, depth-bounded. Files are answered
/// before subdirectories are descended, so a root holding the template flat
/// costs one listing.
fn find_named(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > TEMPLATE_SEARCH_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        {
            return Some(path);
        }
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs.sort();
    subdirs
        .into_iter()
        .find_map(|subdir| find_named(&subdir, name, depth + 1))
}

/// Every `.igr` under `dir`, depth-bounded like [`find_named`], appended to
/// `out` in sorted order. `out` doubles as the dedup set, so a file already
/// tried under one root is not walked again under an overlapping one.
fn collect_igr(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > TEMPLATE_SEARCH_DEPTH || out.len() >= 64 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("igr"))
            && !out.contains(&path)
        {
            out.push(path);
        } else if path.is_dir() {
            subdirs.push(path);
        }
    }
    subdirs.sort();
    for subdir in subdirs {
        collect_igr(&subdir, depth + 1, out);
    }
}

/// The name `file` has on disk, extension included.
fn file_name_of(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The template files the links could mean, in search order, each with
/// whether it is the very path a link states. Only files the links name are
/// listed -- [`pick_linked_template`] takes no other, so no other is worth
/// parsing:
///
/// 1. the path an `.igr` link states, when it opens on this machine, else
///    the file of that name under the template roots;
/// 2. every `.igr` under the roots whose name starts with a stem the links
///    spell. This is what answers a link stored in 8.3 short form:
///    `CPE-SQ~2.IGR` is no file on this disk, but `CPE-SQ-A0.igr`,
///    `CPE-SQ-A1.igr` and `CPE-SQ-A2.igr` all start with what it kept.
fn named_template_files(links: &[String], roots: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    let prefixes = linked_template_prefixes(links);
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    for link in links {
        let is_igr = template_basename(link)
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("igr"));
        if !is_igr {
            continue;
        }
        if let Some(file) = resolve_template_path(link, roots) {
            if !files.iter().any(|(seen, _)| *seen == file) {
                files.push((file, Path::new(link.trim()).is_file()));
            }
        }
    }
    let mut listed: Vec<PathBuf> = Vec::new();
    for root in roots {
        collect_igr(root, 0, &mut listed);
    }
    for file in listed {
        if template_named_by(&file_name_of(&file), &prefixes)
            && !files.iter().any(|(seen, _)| *seen == file)
        {
            files.push((file, false));
        }
    }
    files
}

/// The smallest standard sheet strokes spanning `extent_mm` fit, either way
/// up. A drawing is laid out from the sheet's own origin, so the box its
/// strokes span is also the corner they are drawn up to.
fn standard_sheet_for(extent_mm: (f64, f64)) -> Option<(f64, f64)> {
    STANDARD_SHEETS_MM
        .iter()
        .flat_map(|&(long, short)| [(long, short), (short, long)])
        .find(|page| {
            extent_mm.0 <= page.0 + SHEET_FIT_SLACK_MM && extent_mm.1 <= page.1 + SHEET_FIT_SLACK_MM
        })
}

/// Width and height the strokes of `geometry` span, millimetres, over the
/// entities `keep` admits; `None` when it strokes nothing.
///
/// Only what a border is drawn with counts -- lines, polylines, arcs and
/// circles (`pid-plot`'s `ink_extent_mm`). A template's frame is its own
/// outermost stroke, and lettering that hangs a millimetre past it must not
/// move the sheet it is matched against.
fn stroked_extent_mm(
    geometry: &NormalizedPidGeometry,
    mm_per_unit: f64,
    keep: impl Fn(&pid_parse::PidGraphicEntity) -> bool,
) -> Option<(f64, f64)> {
    let mut min = (f64::MAX, f64::MAX);
    let mut max = (f64::MIN, f64::MIN);
    let mut seen = false;
    let mut take = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
            seen = true;
        }
    };
    for entity in geometry.entities.iter().filter(|entity| keep(entity)) {
        match &entity.kind {
            PidGraphicKind::Line { start, end } => {
                take(start.x, start.y);
                take(end.x, end.y);
            }
            PidGraphicKind::Polyline { points, .. } => {
                for point in points {
                    take(point.x, point.y);
                }
            }
            PidGraphicKind::Arc { center, radius, .. }
            | PidGraphicKind::Circle { center, radius } => {
                take(center.x - radius, center.y - radius);
                take(center.x + radius, center.y + radius);
            }
            PidGraphicKind::Point { .. }
            | PidGraphicKind::Text { .. }
            | PidGraphicKind::SymbolInstance { .. }
            | PidGraphicKind::Annotation { .. }
            | PidGraphicKind::Unknown { .. } => {}
        }
    }
    seen.then_some(((max.0 - min.0) * mm_per_unit, (max.1 - min.1) * mm_per_unit))
}

/// Whether SmartPlant prints `entity`: not from a sheet layer the drawing's
/// view filter switches off, and not the `ConsistencyChecks` review overlay
/// (`pid-plot`'s `suppressed_from_plot`, negated).
fn printed(entity: &pid_parse::PidGraphicEntity) -> bool {
    match &entity.source_layer {
        Some(layer) => {
            layer.displayed != Some(false) && layer.name.as_deref() != Some("ConsistencyChecks")
        }
        None => true,
    }
}

/// The page a sheet that names a template is matched on: the one it states,
/// else the standard sheet its printed strokes fit. `Q井场` is the second
/// kind -- it states no page and its strokes span 841 x 594. Strokes on a
/// switched-off layer reach no paper, so they may not push the sheet a size
/// up and hand it a border of the wrong size.
fn sheet_page_mm(geometry: &NormalizedPidGeometry, unit: &ImportUnit) -> Option<(f64, f64)> {
    geometry.page_dimensions_mm.or_else(|| {
        stroked_extent_mm(geometry, unit.mm_per_unit(), printed).and_then(standard_sheet_for)
    })
}

/// A template file read far enough to be measured, and to be drawn if it is
/// the one picked.
pub(super) struct ReadTemplate {
    file: PathBuf,
    /// The file is the very path a link states, opened on this machine.
    stated: bool,
    parsed: pid_parse::PidDocument,
    geometry: NormalizedPidGeometry,
    unit: ImportUnit,
    /// Width and height its strokes span, millimetres.
    stroked_mm: Option<(f64, f64)>,
}

impl ReadTemplate {
    /// What [`pick_linked_template`] judges this file on.
    pub(super) fn candidate(&self) -> TemplateCandidate {
        TemplateCandidate {
            file_name: file_name_of(&self.file),
            stated: self.stated,
            stroked_mm: self.stroked_mm,
        }
    }
}

/// Parse `igr` and measure its strokes. Nothing is drawn and no document is
/// touched: that is [`overlay_template`], for the one file picked.
///
/// The template is a drawing of the same family: `PidParser` reads its CFB
/// tree and `build_normalized_geometry` projects its records the same way,
/// so it is measured in its own unit like any sheet.
pub(super) fn read_template(igr: &Path, stated: bool) -> Option<ReadTemplate> {
    let parsed = match PidParser::with_options(ParseOptions::geometry()).parse_file(igr) {
        Ok(parsed) => parsed,
        Err(error) => {
            log::warn!("{}: linked template did not parse: {error}", igr.display());
            return None;
        }
    };
    let geometry = build_normalized_geometry(&parsed);
    let unit = ImportUnit::read(&geometry, igr);
    let stroked_mm = stroked_extent_mm(&geometry, unit.mm_per_unit(), |_| true);
    Some(ReadTemplate {
        file: igr.to_path_buf(),
        stated,
        parsed,
        geometry,
        unit,
        stroked_mm,
    })
}

/// Draw `template`'s decoded geometry onto `PID-FRAME`.
///
/// The whole styling apparatus of a sheet -- its own `StyleCluster` line
/// widths and colours, its character-style lettering -- applies to the
/// template unchanged. The entity loop below is the drawing's loop minus the
/// parts a border template does not have: sheet-layer filing (the drawing's
/// view filter belongs to the drawing), and framing bounds (the page already
/// frames the view).
///
/// A template is a drawing, so a symbol it places draws like any other, the
/// way `pid-plot` draws it: [`build_entities`] is handed the body the
/// template itself caches for the placement and the symbol library found
/// from the template's own folder, as it would be were the `.igr` opened as
/// a sheet. The plant backup's templates keep drafting in placements -- the
/// wave in the `比例` cell of the `CPE-SQ` title blocks, the sign-off column
/// of `CPECCHB-SQ-A2.igr`, the whole frame and title block of
/// `XIONGANA2.igr` -- and a loop that skipped them left it off the sheet.
///
/// Two things are still left out:
///
/// - A label placement whose cached body is only its anchor cross, which is
///   [`build_entities`]' rule for every sheet ([`is_label_symbol`],
///   [`is_anchor_cross_only`]). What such a label shows is lettering the
///   sheet states in text records of its own, so drawing the body would add
///   a cross and nothing else. This is also why the overlay cannot letter
///   the block twice: the name and number a sheet fills in are that sheet's
///   records, drawn by the sheet's loop, and none of them is the template's
///   to draw.
/// - What the importer adds beside a sheet's placement: the symbol's name
///   ([`LAYER_SYMBOL_LABEL`]) and, where no body answers, the marker dot. A
///   title block wears neither, so a placement no body answers for draws
///   nothing and is counted in [`TemplateOverlay::missing`] instead.
pub(super) fn overlay_template(
    template: &ReadTemplate,
    geometry: &NormalizedPidGeometry,
    doc: &mut Document,
    paper_style_scale: f64,
) -> Option<TemplateOverlay> {
    let igr = template.file.as_path();
    let tpl_geometry = &template.geometry;
    let styles = resolve_styles(&template.parsed, tpl_geometry, igr, doc, paper_style_scale);
    let projection = Projection::for_geometry(tpl_geometry, &template.unit);
    // The template's own tally and its own library, looked for from where
    // the template lies: neither its placements nor its lookups belong in
    // the drawing's report. A template that places no symbol looks for none.
    let mut symbol_bodies = SymbolBodies::default();
    let places_symbols = tpl_geometry.entities.iter().any(|entity| {
        entity.confidence == PidGeometryConfidence::Decoded
            && matches!(entity.kind, PidGraphicKind::SymbolInstance { .. })
    });
    let mut library = if places_symbols {
        discover_symbol_library(igr)
    } else {
        None
    };
    let mut decoded = 0usize;
    let mut drawn = 0usize;
    let mut page_frame_covered = false;
    let template_name = igr
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| igr.display().to_string());
    let mut seen_fill_rings: HashSet<u64> = HashSet::new();
    for entity in &tpl_geometry.entities {
        if entity.confidence != PidGeometryConfidence::Decoded {
            continue;
        }
        let fill = fill_for(&styles.fills, entity);
        if let Some(fill) = fill {
            if let Some(key) = fill_dedupe_key(&entity.kind, fill) {
                if !seen_fill_rings.insert(key) {
                    continue;
                }
            }
        }
        let symbology = style_for(&styles.styles, entity);
        let style_name = style_name_for(&styles.style_names, entity, symbology);
        // The body the template itself caches for a placement, read the way
        // the drawing's loop reads the drawing's.
        let cached_body = match &entity.kind {
            PidGraphicKind::SymbolInstance {
                definition: Some(definition),
                ..
            } => tpl_geometry.symbol_definition(*definition),
            _ => None,
        };
        let is_placement = matches!(entity.kind, PidGraphicKind::SymbolInstance { .. });
        let placed_from = match &entity.kind {
            PidGraphicKind::SymbolInstance { symbol_path, .. } => symbol_path.as_deref(),
            _ => None,
        };
        let tally_before = symbol_bodies;
        let built = match fill {
            Some(fill) => build_fill(&entity.kind, fill, projection),
            // `LAYER_GEOMETRY` is the working layer so `apply_symbology`
            // paints the template's own width and colour; the slot below
            // ends up `PID-FRAME`, which says whose drafting it is.
            None => build_entities(
                &entity.kind,
                BodySources {
                    cached: cached_body,
                    library: library.as_mut(),
                },
                projection,
                symbology,
                LAYER_GEOMETRY,
                &mut symbol_bodies,
                &styles.dash_linetypes,
                paper_style_scale,
                &styles.font_styles,
            ),
        };
        let built = if is_placement {
            let outcome = placement_outcome(tally_before, symbol_bodies);
            log::debug!(
                "{}: template placement {:?}: {outcome}",
                igr.display(),
                placed_from.unwrap_or_default()
            );
            template_symbol_body(built, symbol_bodies.markers > tally_before.markers)
        } else {
            built
        };
        if built.is_empty() {
            continue;
        }
        decoded += 1;
        let template_symbol = placed_from.and_then(symbol_name);
        let text_style = height_for(&styles.text_heights, entity);
        let height_mm = text_style.map(|h| projection.mm(h.height_m));
        let text_rgb = text_style.and_then(pid_parse::style_link::ResolvedTextHeight::rgb);
        let text_alignment = text_style.and_then(|style| style.alignment);
        // The record's own trailer byte beats the paragraph the same way the
        // sheet's labels do: the template letters its title-block captions at
        // the hot point it states (`0x10` bottom-centre up the left column).
        let anchor_code = match &entity.kind {
            PidGraphicKind::Text { anchor_code, .. } => Some(*anchor_code),
            _ => None,
        };
        let font_style = text_style
            .and_then(|style| style.font_name.as_deref())
            .and_then(|font| styles.font_styles.get(font))
            .map(String::as_str);
        let built = stack_text_lines(
            built,
            height_mm.unwrap_or(TEXT_HEIGHT_MM),
            text_style.and_then(|style| style.line_spacing),
        );
        for mut one in built {
            if let Some(style) = symbology {
                apply_symbology(&mut one, style, &styles.dash_linetypes, paper_style_scale);
            }
            if let Some(mm) = height_mm {
                apply_text_height(&mut one, mm);
            }
            if let Some(rgb) = text_rgb {
                apply_text_colour(&mut one, rgb);
            }
            match anchor_code {
                Some(code) => apply_text_anchor(&mut one, code),
                None => {
                    if let Some(alignment) = text_alignment {
                        apply_text_alignment(&mut one, alignment);
                    }
                }
            }
            if let Some(name) = font_style {
                apply_text_style(&mut one, name);
            }
            // No `sheet_layer=`: the drawing's view filter would read the
            // template's own layer name as one of the drawing's, and a same-
            // named layer the drawing switches off would take the template's
            // drafting down with it. The name is provenance, not a switch --
            // it rides the second record as `template_layer=` instead.
            attach_pid_metadata(
                &mut one,
                None,
                None,
                role_of_layer(LAYER_FRAME),
                style_name,
                None,
            );
            attach_template_provenance(
                &mut one,
                &template_name,
                entity
                    .source_layer
                    .as_ref()
                    .and_then(|layer| layer.name.as_deref()),
                template_symbol.as_deref(),
            );
            if let Some((width, height)) = geometry.page_dimensions_mm {
                if let Some((min_x, min_y, max_x, max_y)) = drawn_extent(&one) {
                    if (max_x - min_x - width).abs() <= PAGE_MATCH_TOLERANCE_MM
                        && (max_y - min_y - height).abs() <= PAGE_MATCH_TOLERANCE_MM
                    {
                        page_frame_covered = true;
                    }
                }
            }
            one.common_mut().layer = LAYER_FRAME.to_string();
            let _ = doc.add_entity(one);
            drawn += 1;
        }
    }
    // A placement no body answered for is a template record that was not
    // drawn, the same as one no decoder reads.
    let missing: usize = tpl_geometry
        .dropped_graphic_records
        .iter()
        .map(|dropped| dropped.count)
        .chain(
            tpl_geometry
                .refused_graphic_records
                .iter()
                .map(|refused| refused.count),
        )
        .sum::<usize>()
        + symbol_bodies.markers;
    log::info!(
        "{}: border template drew {drawn} entities from {decoded} decoded records ({missing} template record(s) not drawn)",
        igr.display()
    );
    if places_symbols {
        let library_state = match library.as_ref() {
            Some(library) => format!(
                "{} of {} symbol(s) looked up are not in the library at {:?}",
                library.missing().len(),
                library.lookups(),
                library.roots()
            ),
            None => "no symbol library found from the template's folder".to_string(),
        };
        log::info!(
            "{}: template placements: {} drew the body the template caches, {} a library body, {} label anchor(s) left to the sheet's own lettering, {} with no body (not drawn); {library_state}",
            igr.display(),
            symbol_bodies.cache,
            symbol_bodies.library,
            symbol_bodies.label_anchors,
            symbol_bodies.markers
        );
    }
    (drawn > 0).then_some(TemplateOverlay {
        file: igr.to_path_buf(),
        drawn,
        missing,
        page_frame_covered,
    })
}

/// What [`build_entities`] did with one placement, read off the tally it
/// keeps: each placement it is handed moves exactly one of the four counts.
fn placement_outcome(before: SymbolBodies, after: SymbolBodies) -> &'static str {
    if after.markers > before.markers {
        "no body, not drawn"
    } else if after.label_anchors > before.label_anchors {
        "a label's anchor cross, left to the sheet's own lettering"
    } else if after.cache > before.cache {
        "drawn from the body the template caches"
    } else if after.library > before.library {
        "drawn from the symbol library"
    } else {
        "nothing built"
    }
}

/// What a template's placement leaves on the sheet out of what
/// [`build_entities`] built for it: the body alone. The symbol's name is
/// lettering the importer adds beside a sheet's placement, and the marker
/// dot stands for a body that is missing; a title block wears neither, so
/// `no_body` -- the marker is what was built -- leaves nothing at all.
fn template_symbol_body(built: Vec<EntityType>, no_body: bool) -> Vec<EntityType> {
    if no_body {
        return Vec::new();
    }
    built
        .into_iter()
        .filter(|entity| entity.common().layer != LAYER_SYMBOL_LABEL)
        .collect()
}

/// Which `.igr` an entity came from and which layer the template filed it
/// on, beside the `role=frame` it already carries:
/// `template=CPE-SQ-A1.igr` under the same XDATA application, so a saved
/// drawing still says whose drafting the title block is. An entity one of
/// the template's symbol placements drew also names the symbol,
/// `template_symbol=hqz`, which is what tells the drafting a template keeps
/// in a placement from the drafting it states in records of its own.
fn attach_template_provenance(
    entity: &mut EntityType,
    template_name: &str,
    template_layer: Option<&str>,
    template_symbol: Option<&str>,
) {
    use document::xdata::{ExtendedDataRecord, XDataValue};
    let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
    record.add_value(XDataValue::String(format!("template={template_name}")));
    if let Some(layer) = template_layer {
        record.add_value(XDataValue::String(format!("template_layer={layer}")));
    }
    if let Some(symbol) = template_symbol {
        record.add_value(XDataValue::String(format!("template_symbol={symbol}")));
    }
    entity.common_mut().extended_data.add_record(record);
}

#[cfg(test)]
mod tests {
    use super::*;

    const A0: (f64, f64) = (1189.0, 841.0);
    const A1: (f64, f64) = (841.0, 594.0);
    const A2: (f64, f64) = (594.0, 420.0);

    /// The link most of the plant backup's sheets store: the authoring
    /// machine's 8.3 short path to `...\Template Files\CPE-SQ-A1.igr`.
    const SHORT_CPE_SQ: &str = r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~2.IGR";

    fn links(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn candidate(file_name: &str, stroked_mm: Option<(f64, f64)>) -> TemplateCandidate {
        TemplateCandidate {
            file_name: file_name.to_string(),
            stated: false,
            stroked_mm,
        }
    }

    fn point(x: f64, y: f64) -> PidPoint {
        PidPoint { x, y }
    }

    fn line(start: (f64, f64), end: (f64, f64)) -> PidGraphicKind {
        PidGraphicKind::Line {
            start: point(start.0, start.1),
            end: point(end.0, end.1),
        }
    }

    fn lettering(at: (f64, f64)) -> PidGraphicKind {
        PidGraphicKind::Text {
            insertion: point(at.0, at.1),
            value: "TITLE".to_string(),
            height: 0.0035,
            rotation: 0.0,
            anchor_code: 0,
            text_box_width: None,
        }
    }

    /// One sheet record of `kind`, filed on the sheet layer `layer` names
    /// with the display bit it carries.
    fn entity(
        kind: PidGraphicKind,
        layer: Option<(&str, Option<bool>)>,
    ) -> pid_parse::PidGraphicEntity {
        pid_parse::PidGraphicEntity {
            id: "e".to_string(),
            drawing_id: None,
            graphic_oid: None,
            source_layer: layer.map(|(name, displayed)| PidSourceLayer {
                oid: 1,
                name: Some(name.to_string()),
                storage_path: String::new(),
                displayed,
            }),
            kind,
            coordinate_context: pid_parse::PidCoordinateContext::default(),
            source: pid_parse::PidGraphicProvenance {
                stream_path: None,
                byte_range: None,
                record_id: None,
                record_kind: None,
                field_x: None,
                note: None,
            },
            confidence: PidGeometryConfidence::Decoded,
        }
    }

    /// A short name: Windows kept the characters before the `~`, and those
    /// are all the link says.
    #[test]
    fn pid_template_prefix_of_a_short_name_stops_at_the_tilde() {
        assert_eq!(
            linked_template_prefix("CPE-SQ~2.IGR").as_deref(),
            Some("CPE-SQ")
        );
        assert_eq!(
            linked_template_prefix("CPECCH~3.IGR").as_deref(),
            Some("CPECCH")
        );
    }

    /// A long name spells the whole stem.
    #[test]
    fn pid_template_prefix_of_a_long_name_is_the_whole_stem() {
        assert_eq!(
            linked_template_prefix("CPE-SQ-A1.igr").as_deref(),
            Some("CPE-SQ-A1")
        );
        assert_eq!(
            linked_template_prefix("A2-W - New.igr").as_deref(),
            Some("A2-W - NEW")
        );
    }

    /// Only the last component is the file name -- the directories of a
    /// short path carry `~` of their own -- whichever separator the linking
    /// machine wrote.
    #[test]
    fn pid_template_prefix_reads_only_the_last_component_of_a_path() {
        assert_eq!(
            linked_template_prefix(SHORT_CPE_SQ).as_deref(),
            Some("CPE-SQ")
        );
        assert_eq!(
            linked_template_prefix(r"\\WIN-SPID\share\Ref\Template Files\CPECCHB-SQ-A2.igr")
                .as_deref(),
            Some("CPECCHB-SQ-A2")
        );
        assert_eq!(
            linked_template_prefix("D:/plant/Ref/Template Files/CPE-SQ-A0.igr").as_deref(),
            Some("CPE-SQ-A0")
        );
    }

    /// The prefix is upper-cased, so a link and a file that differ only in
    /// case still meet.
    #[test]
    fn pid_template_prefix_is_upper_cased_whatever_the_link_spells() {
        assert_eq!(
            linked_template_prefix("cpe-sq-a1.IGR").as_deref(),
            Some("CPE-SQ-A1")
        );
        assert_eq!(
            linked_template_prefix("Cpe-Sq~2.Igr").as_deref(),
            Some("CPE-SQ")
        );
    }

    /// A link to the template's drawing (`.pid`) names the same template as
    /// one to its exported `.igr`; any other target names no template.
    #[test]
    fn pid_template_prefix_needs_a_template_extension() {
        assert_eq!(
            linked_template_prefix(r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~2.PID").as_deref(),
            Some("CPE-SQ")
        );
        assert_eq!(linked_template_prefix("Gate Valve.sym"), None);
        assert_eq!(linked_template_prefix("Book1.xls"), None);
        assert_eq!(linked_template_prefix("CPE-SQ~2"), None);
        assert_eq!(linked_template_prefix("~1.igr"), None);
        assert_eq!(linked_template_prefix(""), None);
    }

    /// Several links of one sheet name each template once, in the links'
    /// order.
    #[test]
    fn pid_template_prefixes_are_distinct_and_keep_the_links_order() {
        assert_eq!(
            linked_template_prefixes(&links(&[
                SHORT_CPE_SQ,
                "CPE-SQ-A1.igr",
                r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~2.PID",
                "Gate Valve.sym",
            ])),
            links(&["CPE-SQ", "CPE-SQ-A1"])
        );
        assert!(linked_template_prefixes(&links(&["Gate Valve.sym"])).is_empty());
    }

    /// The links read off the drawing are the template targets of every
    /// `JSite`: the exported `.igr` and the template's own `.pid`.
    #[test]
    fn pid_template_links_are_the_igr_and_pid_links_of_every_jsite() {
        let site = |ole_links: &[&str]| pid_parse::JSite {
            name: "JSite1".to_string(),
            ole_links: links(ole_links),
            ..Default::default()
        };
        let mut parsed = pid_parse::PidDocument::default();
        parsed.jsites.push(site(&[
            r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~3.PID",
            "Sheet1",
        ]));
        parsed
            .jsites
            .push(site(&["CPE-SQ-A0.igr ", "Gate Valve.sym", "CPE-SQ-A0.igr"]));
        assert_eq!(
            linked_templates(&parsed),
            links(&[
                r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~3.PID",
                "CPE-SQ-A0.igr"
            ])
        );
    }

    /// The template the link names is the one picked -- not a stranger of
    /// the same size that happens to be listed first.
    #[test]
    fn pid_template_pick_takes_the_template_the_link_names() {
        let candidates = [
            candidate("A1-W - New.igr", Some(A1)),
            candidate("CPECCHB-SQ-A1.igr", Some(A1)),
            candidate("CPE-SQ-A1.igr", Some(A1)),
        ];
        assert_eq!(
            pick_linked_template(&candidates, &links(&[SHORT_CPE_SQ]), A1),
            Some(2)
        );
        // `CPECCH` is no prefix of `CPE-SQ-A1`, nor `CPE-SQ` of
        // `CPECCHB-SQ-A1`: the two families never answer for each other.
        let other = links(&[r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPECCH~3.IGR"]);
        assert_eq!(pick_linked_template(&candidates, &other, A1), Some(1));
    }

    /// One prefix answers for a whole family (`CPE-SQ-A0`, `-A1`, `-A2`);
    /// among them the page decides.
    #[test]
    fn pid_template_pick_chooses_among_one_prefix_by_the_page() {
        let candidates = [
            candidate("CPE-SQ-A0.igr", Some(A0)),
            candidate("CPE-SQ-A1.igr", Some(A1)),
            candidate("CPE-SQ-A2.igr", Some(A2)),
        ];
        let named = links(&[SHORT_CPE_SQ]);
        assert_eq!(pick_linked_template(&candidates, &named, A0), Some(0));
        assert_eq!(pick_linked_template(&candidates, &named, A1), Some(1));
        assert_eq!(pick_linked_template(&candidates, &named, A2), Some(2));
        assert_eq!(
            pick_linked_template(&candidates, &named, (843.09, 596.09)),
            Some(1),
            "a page left a little larger than the template's drafting still wears it"
        );
    }

    /// A sheet that names a template never wears one it does not name: with
    /// no file answering the name, or none of its size, there is no border.
    #[test]
    fn pid_template_pick_never_substitutes_a_template_the_link_does_not_name() {
        let named = links(&[SHORT_CPE_SQ]);
        let strangers = [
            candidate("A1-W - New.igr", Some(A1)),
            candidate("CPECCHB-SQ-A1.igr", Some(A1)),
        ];
        assert_eq!(pick_linked_template(&strangers, &named, A1), None);
        let other_sizes = [
            candidate("A1-W - New.igr", Some(A1)),
            candidate("CPE-SQ-A0.igr", Some(A0)),
            candidate("CPE-SQ-A2.igr", Some(A2)),
        ];
        assert_eq!(pick_linked_template(&other_sizes, &named, A1), None);
        assert_eq!(pick_linked_template(&[], &named, A1), None);
    }

    /// A sheet that names no template gets none, as before.
    #[test]
    fn pid_template_pick_is_nothing_for_a_sheet_that_names_no_template() {
        let candidates = [
            candidate("A1-W - New.igr", Some(A1)),
            candidate("CPE-SQ-A1.igr", Some(A1)),
        ];
        assert_eq!(pick_linked_template(&candidates, &[], A1), None);
        assert_eq!(
            pick_linked_template(&candidates, &links(&["Gate Valve.sym", "Book1.xls"]), A1),
            None
        );
    }

    /// A link to the template's drawing is a name like any other: the four
    /// plant sheets that link only `...~N.PID` wear the template it spells.
    #[test]
    fn pid_template_pick_reads_a_link_to_the_templates_drawing_as_a_name() {
        let candidates = [
            candidate("A0-W - New.igr", Some(A0)),
            candidate("CPE-SQ-A0.igr", Some(A0)),
        ];
        let named = links(&[r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPE-SQ~3.PID"]);
        assert_eq!(
            pick_linked_template(&candidates, &named, (1189.31, 841.31)),
            Some(1)
        );
    }

    /// A link that spells a whole file name is the sheet's own statement of
    /// the file it wears: taken without measuring it, ahead of any template
    /// that only shares the prefix. `XIONGANA2.igr` is why -- a title-block
    /// fragment with no stroke at all, which no size rule could accept.
    #[test]
    fn pid_template_pick_takes_a_wholly_spelled_name_without_measuring_it() {
        let fragment = [
            candidate("A2-W - New.igr", Some(A2)),
            candidate("XIONGANA2.igr", None),
        ];
        let spelled = links(&[r"\\WIN-SPID\QSMCQTAZ13\PLANT\REF\Template Files\XIONGANA2.igr"]);
        assert_eq!(pick_linked_template(&fragment, &spelled, A2), Some(1));
        let family = [
            candidate("CPECCHBA2-old.igr", Some(A2)),
            candidate("cpecchba2.IGR", Some((617.0, 437.0))),
        ];
        assert_eq!(
            pick_linked_template(&family, &links(&["CPECCHBA2.igr"]), A2),
            Some(1),
            "the whole name outranks a better-fitting file that only starts with it"
        );
        // A short name is not a whole name: the file it could mean is still
        // measured, and one with nothing to measure is not taken on trust.
        assert_eq!(
            pick_linked_template(
                &[candidate("CPE-SQ-A1.igr", None)],
                &links(&[SHORT_CPE_SQ]),
                A1
            ),
            None
        );
        // The file the link's own path opens on this machine is that file.
        let stated = [TemplateCandidate {
            stated: true,
            ..candidate("CPE-SQ~2.IGR", Some(A0))
        }];
        assert_eq!(
            pick_linked_template(&stated, &links(&[SHORT_CPE_SQ]), A1),
            Some(0)
        );
    }

    /// A named template is allowed ten millimetres on the worse axis: the
    /// frame of `N井场` states 843.46 x 596.71 against an 841 x 594 template.
    #[test]
    fn pid_template_pick_allows_a_named_template_ten_millimetres() {
        let candidates = [candidate("CPECCHB-SQ-A1.igr", Some(A1))];
        let named = links(&[r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPECCH~3.PID"]);
        assert_eq!(
            pick_linked_template(&candidates, &named, (843.46, 596.71)),
            Some(0)
        );
        assert_eq!(
            pick_linked_template(&candidates, &named, (851.0, 594.0)),
            Some(0)
        );
        assert_eq!(
            pick_linked_template(&candidates, &named, (851.5, 594.0)),
            None
        );
        assert_eq!(
            pick_linked_template(&candidates, &named, (841.0, 583.5)),
            None,
            "the worse axis decides"
        );
    }

    /// Two templates of one family and one size differ only in the last
    /// bits of their extents; which is nearer the page's own noise is an
    /// accident, so the name decides and the pick is the same one twice.
    #[test]
    fn pid_template_pick_is_the_same_one_twice_when_two_templates_measure_alike() {
        let first = candidate("CPE-SQ-A1-b.igr", Some((841.0000000003785, 594.0)));
        let second = candidate("CPE-SQ-A1-a.igr", Some((841.0000000000001, 594.0)));
        let named = links(&[SHORT_CPE_SQ]);
        let page = (841.0000500894253, 594.0);
        assert_eq!(
            pick_linked_template(&[first.clone(), second.clone()], &named, page),
            Some(1)
        );
        assert_eq!(
            pick_linked_template(&[second, first], &named, page),
            Some(0)
        );
        let nearer = [
            candidate("CPE-SQ-A1-a.igr", Some((842.5, 594.0))),
            candidate("CPE-SQ-A1-z.igr", Some(A1)),
        ];
        assert_eq!(
            pick_linked_template(&nearer, &named, A1),
            Some(1),
            "a difference that can be measured decides before the name"
        );
    }

    /// Only the files a link names are read at all, wherever they sit under
    /// the template roots; the one a link's own path opens says so.
    #[test]
    fn pid_template_files_are_only_the_ones_a_link_names() {
        // No "template" in this name: a folder so named directly under the
        // temp directory is a template root for every drawing a backup
        // unpacks there.
        let temp = std::env::temp_dir().join(format!(
            "ocs-pid-named-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let dir = temp.join("Ref").join("Template Files");
        std::fs::create_dir_all(&dir).expect("temp dirs");
        for name in [
            "A1-W - New.igr",
            "CPE-SQ-A0.igr",
            "cpe-sq-a1.IGR",
            "CPECCHB-SQ-A1.igr",
            "CPE-SQ-notes.txt",
        ] {
            std::fs::write(dir.join(name), b"stub").expect("write");
        }
        let roots = vec![temp.clone()];
        let found = |links: &[String]| -> Vec<(String, bool)> {
            let mut found: Vec<(String, bool)> = named_template_files(links, &roots)
                .into_iter()
                .map(|(file, stated)| {
                    let name = file.file_name().unwrap_or_default();
                    (name.to_string_lossy().into_owned(), stated)
                })
                .collect();
            found.sort();
            found
        };
        let short = found(&links(&[SHORT_CPE_SQ]));
        let drawing = found(&links(&[r"C:\PID_SQ~1\SQPlant\Ref\TEMPLA~1\CPECCH~3.PID"]));
        let own = dir.join("CPECCHB-SQ-A1.igr").to_string_lossy().into_owned();
        let stated = found(&[own]);
        let unanswered = found(&links(&["TransAcad.igr"]));
        // Before anything is asserted, so that a failure leaves no file behind.
        let _ = std::fs::remove_dir_all(&temp);
        assert_eq!(
            short,
            vec![
                ("CPE-SQ-A0.igr".to_string(), false),
                ("cpe-sq-a1.IGR".to_string(), false)
            ]
        );
        assert_eq!(drawing, vec![("CPECCHB-SQ-A1.igr".to_string(), false)]);
        assert_eq!(stated, vec![("CPECCHB-SQ-A1.igr".to_string(), true)]);
        assert!(
            unanswered.is_empty(),
            "a name no file answers has no candidates"
        );
    }

    /// A sheet's size is read off the standard sheet its strokes fit, either
    /// way up, with a few millimetres of slack past the edge.
    #[test]
    fn pid_template_standard_sheet_is_the_smallest_one_the_strokes_fit() {
        assert_eq!(standard_sheet_for(A1), Some(A1));
        assert_eq!(standard_sheet_for((838.2, 590.9)), Some(A1));
        assert_eq!(standard_sheet_for((845.9, 598.9)), Some(A1));
        assert_eq!(standard_sheet_for((846.1, 594.0)), Some(A0));
        assert_eq!(standard_sheet_for((594.0, 841.0)), Some((594.0, 841.0)));
        assert_eq!(standard_sheet_for((100.0, 80.0)), Some((297.0, 210.0)));
        assert_eq!(standard_sheet_for((1300.0, 900.0)), None);
    }

    /// Strokes are what a border is drawn with: lines, polylines, arcs and
    /// circles are measured, lettering and markers are not.
    #[test]
    fn pid_template_strokes_are_lines_polylines_arcs_and_circles() {
        let geometry = NormalizedPidGeometry {
            entities: vec![
                entity(
                    PidGraphicKind::Polyline {
                        points: vec![
                            point(0.010, 0.020),
                            point(0.200, 0.020),
                            point(0.200, 0.100),
                        ],
                        closed: false,
                    },
                    None,
                ),
                entity(
                    PidGraphicKind::Circle {
                        center: point(0.300, 0.050),
                        radius: 0.010,
                    },
                    None,
                ),
                entity(
                    PidGraphicKind::Arc {
                        center: point(0.050, 0.200),
                        radius: 0.020,
                        start_angle: 0.0,
                        end_angle: 1.0,
                        clockwise: false,
                    },
                    None,
                ),
                entity(
                    PidGraphicKind::Point {
                        position: point(5.0, 5.0),
                    },
                    None,
                ),
                entity(lettering((9.0, 9.0)), None),
            ],
            ..Default::default()
        };
        let (width, height) =
            stroked_extent_mm(&geometry, MM_PER_METRE, |_| true).expect("strokes to measure");
        assert!((width - 300.0).abs() < 1e-9, "width {width}");
        assert!((height - 200.0).abs() < 1e-9, "height {height}");
        assert_eq!(stroked_extent_mm(&geometry, MM_PER_METRE, |_| false), None);
    }

    /// The page a named sheet is matched on: the one it states, else the
    /// standard sheet the strokes SmartPlant prints fit -- strokes on a
    /// switched-off layer or the review overlay do not push it a size up.
    #[test]
    fn pid_template_page_of_a_sheet_that_states_none_is_the_sheet_its_printed_strokes_fit() {
        let geometry = NormalizedPidGeometry {
            entities: vec![
                entity(
                    line((0.0, 0.0), (0.841, 0.0)),
                    Some(("Default", Some(true))),
                ),
                entity(line((0.0, 0.0), (0.0, 0.594)), None),
                entity(lettering((1.5, 0.9)), None),
                entity(
                    line((0.0, 0.0), (1.189, 0.841)),
                    Some(("HiddenObjects", Some(false))),
                ),
                entity(
                    line((0.0, 0.0), (1.189, 0.841)),
                    Some(("ConsistencyChecks", Some(true))),
                ),
            ],
            ..Default::default()
        };
        let unit = ImportUnit::AssumedMetre;
        assert_eq!(sheet_page_mm(&geometry, &unit), Some(A1));
        let stated = NormalizedPidGeometry {
            page_dimensions_mm: Some((843.46, 596.71)),
            ..geometry
        };
        assert_eq!(sheet_page_mm(&stated, &unit), Some((843.46, 596.71)));
        assert_eq!(
            sheet_page_mm(&NormalizedPidGeometry::default(), &unit),
            None
        );
    }

    /// Every `key=value` string an entity carries in its XDATA, whichever
    /// record it rides.
    fn xdata_strings(entity: &EntityType) -> Vec<String> {
        entity
            .common()
            .extended_data
            .records()
            .iter()
            .flat_map(|record| record.values.iter())
            .filter_map(|value| match value {
                document::xdata::XDataValue::String(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// A template as [`read_template`] hands it over, built in memory: the
    /// records and cached symbol bodies of `geometry`, read in metres.
    fn template_of(file: &str, geometry: NormalizedPidGeometry) -> ReadTemplate {
        ReadTemplate {
            file: PathBuf::from(file),
            stated: false,
            parsed: pid_parse::PidDocument::default(),
            geometry,
            unit: ImportUnit::AssumedMetre,
            stroked_mm: None,
        }
    }

    /// Overlay `template` onto `doc` as the border of an A2 sheet.
    fn overlay_on_a2(template: &ReadTemplate, doc: &mut Document) -> Option<TemplateOverlay> {
        let sheet = NormalizedPidGeometry {
            page_dimensions_mm: Some(A2),
            ..Default::default()
        };
        overlay_template(template, &sheet, doc, 1.0)
    }

    /// A placement of the symbol filed at `path`, unrotated and unscaled.
    fn placement(
        at: (f64, f64),
        path: &str,
        definition: Option<pid_parse::PidSymbolDefinitionRef>,
    ) -> PidGraphicKind {
        PidGraphicKind::SymbolInstance {
            insertion: point(at.0, at.1),
            symbol_path: Some(path.to_string()),
            rotation: 0.0,
            scale: [1.0, 1.0],
            definition,
        }
    }

    /// A black, solid stroke style of the given width.
    fn pen(width_mm: f64) -> Option<PrimitiveStyle> {
        Some(PrimitiveStyle {
            rgb: [0, 0, 0],
            width_mm,
            dash_mm: Vec::new(),
            paper: false,
        })
    }

    /// A cached symbol body whose `Default` layer (oid 8) the file displays
    /// and whose `Label` layer (oid 9) it switches off, the way the caches
    /// of the corpus do. Each stroke names the layer it sits on and the
    /// style its own storage states for it.
    fn cached_body(
        reference: pid_parse::PidSymbolDefinitionRef,
        strokes: Vec<(SymbolPrimitive, u32, Option<PrimitiveStyle>)>,
    ) -> PidSymbolDefinition {
        let layer = |oid: u32, name: &str, displayed: bool| pid_parse::PidSymbolSheetLayer {
            oid,
            name: Some(name.to_string()),
            displayed: Some(displayed),
        };
        let mut primitives = Vec::new();
        let mut primitive_layers = Vec::new();
        let mut primitive_styles = Vec::new();
        for (primitive, on, style) in strokes {
            primitives.push(primitive);
            primitive_layers.push(on);
            primitive_styles.push(style);
        }
        PidSymbolDefinition {
            reference,
            layers: vec![8, 9],
            sheet_layers: vec![layer(8, "Default", true), layer(9, "Label", false)],
            primitives,
            primitive_layers,
            primitive_styles,
            primitive_letterings: Vec::new(),
            dimensions: Vec::new(),
            variables: Vec::new(),
            template: None,
        }
    }

    /// A template is a drawing, so a symbol it places draws like any other:
    /// the sign-off column of `CPECCHB-SQ-A2.igr` is three placements whose
    /// bodies the template caches, and `XIONGANA2.igr` is one placement
    /// around its whole frame and title block. The body lands where the
    /// placement puts it, on `PID-FRAME`, naming the template and the
    /// symbol it came from -- and a border carries neither the symbol's
    /// name beside it nor a marker dot.
    #[test]
    fn pid_template_draws_the_symbols_the_template_places() {
        let sign_off = pid_parse::PidSymbolDefinitionRef { site: 16, sheet: 3 };
        let stroke = |end: (f64, f64)| {
            let start = (0.0, 0.0);
            (SymbolPrimitive::Line { start, end }, 8, pen(0.25))
        };
        let caption = |text: &str, at: (f64, f64), layer: u32| {
            let text = text.to_string();
            (SymbolPrimitive::Text { text, at }, layer, None)
        };
        let shown = Some(("Default", Some(true)));
        let template = template_of(
            "SIGN-A2.igr",
            NormalizedPidGeometry {
                entities: vec![
                    entity(line((0.0, 0.0), (0.594, 0.0)), shown),
                    entity(placement((0.010, 0.020), "hqz.sym", Some(sign_off)), shown),
                ],
                symbol_definitions: vec![cached_body(
                    sign_off,
                    vec![
                        stroke((0.008, 0.0)),
                        stroke((0.0, 0.030)),
                        caption("会 签", (0.002, 0.010), 8),
                        caption("Label", (0.002, 0.020), 9),
                    ],
                )],
                ..Default::default()
            },
        );
        let mut doc = Document::new();
        let overlay = overlay_on_a2(&template, &mut doc).expect("the template draws");
        assert_eq!(
            overlay.drawn, 4,
            "the template's own line, two strokes and one caption"
        );
        assert_eq!(overlay.missing, 0);

        let drawn: Vec<&EntityType> = doc.entities().collect();
        assert_eq!(drawn.len(), 4);
        assert!(
            drawn
                .iter()
                .all(|entity| entity.common().layer == LAYER_FRAME),
            "everything the template draws files on PID-FRAME"
        );
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        let has_line = |x0: f64, y0: f64, x1: f64, y1: f64| {
            drawn.iter().any(|entity| {
                matches!(entity, EntityType::Line(_))
                    && drawn_extent(entity).is_some_and(|(min_x, min_y, max_x, max_y)| {
                        near(min_x, x0) && near(min_y, y0) && near(max_x, x1) && near(max_y, y1)
                    })
            })
        };
        assert!(
            has_line(10.0, 20.0, 18.0, 20.0),
            "the body's horizontal stroke, at the placement"
        );
        assert!(
            has_line(10.0, 20.0, 10.0, 50.0),
            "the body's vertical stroke, at the placement"
        );
        let lettered: Vec<(&str, f64, f64)> = drawn
            .iter()
            .filter_map(|entity| match entity {
                EntityType::Text(text) => Some((
                    text.value.as_str(),
                    text.insertion_point.x,
                    text.insertion_point.y,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            lettered.len(),
            1,
            "the caption alone: not the lettering on the switched-off layer, not the symbol's name: {lettered:?}"
        );
        let (value, x, y) = lettered[0];
        assert_eq!(value, "会 签");
        assert!(
            near(x, 12.0) && near(y, 30.0),
            "lettered at the placement: {x}, {y}"
        );
        assert!(
            !drawn
                .iter()
                .any(|entity| matches!(entity, EntityType::Circle(_))),
            "no marker dot"
        );
        let says =
            |entity: &EntityType, pair: &str| xdata_strings(entity).iter().any(|said| said == pair);
        assert!(
            drawn
                .iter()
                .all(|entity| says(entity, "template=SIGN-A2.igr") && says(entity, "role=frame")),
            "every entity is the frame's and names the template"
        );
        assert_eq!(
            drawn
                .iter()
                .filter(|entity| says(entity, "template_symbol=hqz"))
                .count(),
            3,
            "what the placement drew names the symbol; the template's own line does not"
        );
    }

    /// A sheet's symbols are block references, and the import's summary
    /// counts them and the definitions they share (plan
    /// 2026-11-19-pid-symbol-block, SB-5). The symbols its border template
    /// places are neither: they stay the border's loose drafting on
    /// `PID-FRAME`, add no reference and no definition, and are not among
    /// the two counts -- even where the template places the very symbol the
    /// sheet does. Here the sheet places a valve twice and a gauge once,
    /// and its template the valve.
    #[test]
    fn pid_template_placements_are_not_among_the_symbol_blocks_the_summary_counts() {
        let valve = pid_parse::PidSymbolDefinitionRef { site: 40, sheet: 7 };
        let gauge = pid_parse::PidSymbolDefinitionRef { site: 41, sheet: 2 };
        let stroke = |end: (f64, f64)| {
            let start = (0.0, 0.0);
            (SymbolPrimitive::Line { start, end }, 8, pen(0.25))
        };
        let valve_body = || cached_body(valve, vec![stroke((0.004, 0.0)), stroke((0.0, 0.003))]);
        let shown = Some(("Default", Some(true)));
        let sheet = NormalizedPidGeometry {
            entities: vec![
                entity(
                    placement((0.100, 0.200), "Ball Valve.sym", Some(valve)),
                    shown,
                ),
                entity(
                    placement((0.300, 0.200), "Ball Valve.sym", Some(valve)),
                    shown,
                ),
                entity(placement((0.200, 0.300), "Gauge.sym", Some(gauge)), shown),
            ],
            symbol_definitions: vec![
                valve_body(),
                cached_body(gauge, vec![stroke((0.002, 0.002))]),
            ],
            page_dimensions_mm: Some(A2),
            ..Default::default()
        };
        let template = template_of(
            "VALVE-A2.igr",
            NormalizedPidGeometry {
                entities: vec![
                    entity(line((0.0, 0.0), (0.594, 0.0)), shown),
                    entity(
                        placement((0.500, 0.020), "Ball Valve.sym", Some(valve)),
                        shown,
                    ),
                ],
                symbol_definitions: vec![valve_body()],
                ..Default::default()
            },
        );

        // The steps `load_pid_with` takes once the file is parsed.
        let parsed = pid_parse::PidDocument::default();
        let file = Path::new("sheet.pid");
        let mut doc = prepare_document(&parsed);
        let tables = resolve_styles(&parsed, &sheet, file, &mut doc, 1.0);
        let built = build_document_entities(
            &sheet,
            &tables,
            &SheetItems::default(),
            &mut None,
            &ImportUnit::AssumedMetre,
            &mut doc,
            1.0,
        );
        let overlay =
            overlay_template(&template, &sheet, &mut doc, 1.0).expect("the template draws");
        assert_eq!(
            overlay.drawn, 3,
            "the template's own line and the two strokes of its valve"
        );
        let summary = finish(
            file,
            &sheet,
            None,
            &tables,
            ImportUnit::AssumedMetre,
            built,
            Some(&overlay),
            &mut doc,
        );

        let references: Vec<&str> = doc
            .model_space_entities()
            .filter_map(|entity| match entity {
                EntityType::Insert(insert) => Some(insert.block_name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            references,
            ["SPPID_Ball Valve", "SPPID_Ball Valve", "SPPID_Gauge"],
            "the sheet's three placements and none of the template's"
        );
        let of_the_templates_valve: Vec<&EntityType> = doc
            .model_space_entities()
            .filter(|entity| {
                xdata_strings(entity)
                    .iter()
                    .any(|said| said == "template_symbol=Ball Valve")
            })
            .collect();
        assert_eq!(of_the_templates_valve.len(), 2, "its two strokes");
        assert!(
            of_the_templates_valve.iter().all(|entity| {
                matches!(entity, EntityType::Line(_)) && entity.common().layer == LAYER_FRAME
            }),
            "loose lines on PID-FRAME, not a reference"
        );
        assert_eq!(summary.border_template.as_deref(), Some("VALVE-A2.igr"));
        assert_eq!(
            (summary.drawn, summary.template_entities),
            (8, 3),
            "the sheet's own entities, and the template's beside them"
        );

        let count = |key: &str| {
            summary
                .counts()
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| *value)
        };
        assert_eq!(
            (count("symbol_blocks"), count("symbol_definitions")),
            (Some(3), Some(2)),
            "the sheet's three references to its two definitions, and nothing of the template's"
        );
    }

    /// What a title block's label shows is the sheet's own text record; the
    /// template only places the label, and the body it caches for it is the
    /// hairline cross the label is anchored by, its lettering on a layer
    /// the file switches off. Such a placement adds nothing to the border
    /// -- no cross, no second copy of the lettering, no marker dot, no name
    /// -- and is not a record the overlay failed to draw.
    #[test]
    fn pid_template_label_placement_of_only_its_anchor_cross_draws_nothing() {
        let label = pid_parse::PidSymbolDefinitionRef {
            site: 329,
            sheet: 327,
        };
        let hairline = |start: (f64, f64), end: (f64, f64)| {
            (SymbolPrimitive::Line { start, end }, 8, pen(0.01))
        };
        let number = SymbolPrimitive::Text {
            text: "DWG-0201".to_string(),
            at: (0.0, 0.0),
        };
        let shown = Some(("Default", Some(true)));
        let template = template_of(
            "TITLE-A2.igr",
            NormalizedPidGeometry {
                entities: vec![
                    entity(lettering((0.500, 0.030)), shown),
                    entity(
                        placement(
                            (0.540, 0.030),
                            r"\\SERVER\Ref\Symbols\Design\Annotation\Labels\Item Note & Label.sym",
                            Some(label),
                        ),
                        shown,
                    ),
                ],
                symbol_definitions: vec![cached_body(
                    label,
                    vec![
                        hairline((-0.00127, 0.0), (0.00127, 0.0)),
                        hairline((0.0, -0.00127), (0.0, 0.00127)),
                        (number, 9, None),
                    ],
                )],
                ..Default::default()
            },
        );
        let mut doc = Document::new();
        // The drawing number as the sheet itself letters it, on the document
        // before the border is overlaid.
        let mut own = Text::new();
        own.value = "DWG-0201".to_string();
        own.insertion_point = Vector3::new(540.0, 30.0, 0.0);
        own.common.layer = LAYER_TEXT.to_string();
        let _ = doc.add_entity(EntityType::Text(own));

        let overlay = overlay_on_a2(&template, &mut doc).expect("the caption draws");
        assert_eq!(
            overlay.drawn, 1,
            "the template's own caption and nothing of the label"
        );
        assert_eq!(
            overlay.missing, 0,
            "a label left out is not a record that failed to draw"
        );
        let lettered: Vec<&str> = doc
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Text(text) => Some(text.value.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lettered, ["DWG-0201", "TITLE"], "each lettered once");
        assert!(
            !doc.entities()
                .any(|entity| matches!(entity, EntityType::Line(_) | EntityType::Circle(_))),
            "neither the anchor cross nor a marker dot"
        );
    }

    /// A placement neither the template's cache nor a library has a body
    /// for draws nothing on the border -- the marker dot is the drawing's
    /// way of saying a symbol belongs here, and a title block does not wear
    /// one -- but it is counted: it is a record of the template the overlay
    /// did not draw.
    #[test]
    fn pid_template_placement_without_a_body_is_counted_and_draws_no_marker() {
        let template = template_of(
            "LOGO-A2.igr",
            NormalizedPidGeometry {
                entities: vec![
                    entity(line((0.0, 0.0), (0.594, 0.0)), None),
                    entity(placement((0.300, 0.200), "logo.sym", None), None),
                ],
                ..Default::default()
            },
        );
        let mut doc = Document::new();
        let overlay = overlay_on_a2(&template, &mut doc).expect("the frame line draws");
        assert_eq!(overlay.drawn, 1, "the frame line alone");
        assert_eq!(
            overlay.missing, 1,
            "the placement is a template record that was not drawn"
        );
        assert_eq!(doc.entities().count(), 1);
        assert!(
            !doc.entities()
                .any(|entity| matches!(entity, EntityType::Circle(_) | EntityType::Text(_))),
            "neither a marker dot nor the symbol's name"
        );
    }
}

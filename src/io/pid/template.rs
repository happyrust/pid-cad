// The border template: a P&ID's frame, title block and margin strips are an
// OLE-linked `.igr` drawing of their own, not sheet records -- `page.rs` only
// ever drew the page rectangle because the link went nowhere. This module
// resolves the link to a local `.igr`, parses it through the same `pid-parse`
// pipeline the drawing itself went through, and overlays its drafting on
// `PID-FRAME`.

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

/// How far the template's own page frame may differ from the drawing's
/// before the overlay is refused: millimetres, generous enough for the
/// float noise between two frame records of the same sheet size, tight
/// enough that an A0 template never lands on an A1 drawing.
const PAGE_MATCH_TOLERANCE_MM: f64 = 2.0;

/// What the linked border template added to the drawing, for the summary
/// and for [`draw_page_border`]'s don't-double-the-frame check.
#[derive(Debug)]
pub(super) struct TemplateOverlay {
    /// The `.igr` file the overlay drew from.
    pub(super) file: PathBuf,
    /// Entities handed to the document.
    pub(super) drawn: usize,
    /// Template records the parser saw but could not draw: no decoder, or a
    /// shape the decoder for that type refuses. Counted beside the
    /// drawing's own `missing`, never folded into it.
    pub(super) missing: usize,
    /// The template drew a frame covering the drawing's stated page, so the
    /// synthetic border rectangle would double it.
    pub(super) page_frame_covered: bool,
}

/// Overlay the sheet's linked border template onto `doc`, if its `.igr` can
/// be found and parsed.
///
/// `None` is a silent-sheet answer only when the file names no template at
/// all; a link that resolves to nothing on disk is said in the log. The
/// drawing imports identically with or without the overlay -- the template
/// is the frame around the drawing, never a prerequisite.
pub(super) fn overlay_linked_template(
    parsed: &pid_parse::PidDocument,
    geometry: &NormalizedPidGeometry,
    drawing: &Path,
    doc: &mut Document,
    paper_style_scale: f64,
) -> Option<TemplateOverlay> {
    let links = linked_templates(parsed);
    if links.is_empty() {
        return None;
    }
    let roots = template_roots(drawing);
    // Candidates in trust order: the files the links name first, then every
    // `.igr` under the template roots. The second list is what rescues an
    // OLE link stored in 8.3 short-path form (`CPE-SQ~2.IGR` names no file
    // on disk, and the long name is not recoverable from it); when the
    // basename cannot be trusted the page frame does the choosing, since an
    // A0 template never covers an A1 drawing.
    let mut candidates: Vec<PathBuf> = Vec::new();
    for link in &links {
        if let Some(candidate) = resolve_template_path(link, &roots) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    for root in &roots {
        collect_igr(root, 0, &mut candidates);
    }
    for candidate in &candidates {
        if let Some(overlay) = overlay_from(candidate, geometry, doc, paper_style_scale) {
            return Some(overlay);
        }
    }
    log::warn!(
        "{}: linked border template(s) {} resolved to nothing drawable; the sheet imports without its title block and margin strips",
        drawing.display(),
        links
            .iter()
            .map(|link| template_basename(link))
            .collect::<Vec<_>>()
            .join(", ")
    );
    None
}

/// The distinct `.igr` targets the drawing's `JSite` OLE streams name, as
/// the raw link strings (`C:\...\CPE-SQ-A1.igr` and a bare
/// `CPE-SQ-A1.igr` arrive as separate strings of one link).
pub(super) fn linked_templates(parsed: &pid_parse::PidDocument) -> Vec<String> {
    let mut links: Vec<String> = parsed
        .jsites
        .iter()
        .flat_map(|site| site.ole_links.iter())
        .filter(|link| link.trim_end().to_lowercase().ends_with(".igr"))
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

/// Parse `igr` and, when its page agrees with the drawing's, draw its
/// decoded geometry onto `PID-FRAME`.
///
/// The template is a drawing of the same family: `PidParser` reads its CFB
/// tree and `build_normalized_geometry` projects its records the same way,
/// so the whole styling apparatus -- its own `StyleCluster` line widths and
/// colours, its character-style lettering -- applies unchanged. The entity
/// loop below is the drawing's loop minus the parts a border template does
/// not have: symbol placements (a template's `JSymbol` is the label the
/// sheet letters into the block, not a body to draw), sheet-layer filing
/// (the drawing's view filter belongs to the drawing), and framing bounds
/// (the page already frames the view).
pub(super) fn overlay_from(
    igr: &Path,
    geometry: &NormalizedPidGeometry,
    doc: &mut Document,
    paper_style_scale: f64,
) -> Option<TemplateOverlay> {
    let parsed = match PidParser::with_options(ParseOptions::geometry()).parse_file(igr) {
        Ok(parsed) => parsed,
        Err(error) => {
            log::warn!("{}: linked template did not parse: {error}", igr.display());
            return None;
        }
    };
    let tpl_geometry = build_normalized_geometry(&parsed);
    let styles = resolve_styles(&parsed, &tpl_geometry, igr, doc, paper_style_scale);
    let unit = ImportUnit::read(&tpl_geometry, igr);
    let projection = Projection::for_geometry(&tpl_geometry, &unit);
    let mut symbol_bodies = SymbolBodies::default();
    let mut decoded = 0usize;
    let mut page_frame_covered = false;
    // Built entities stage here until the page check passes -- a template
    // that states no page of its own is judged on what it drew, and the
    // drawing must not keep a wrong-size overlay while that is measured.
    let mut staged: Vec<EntityType> = Vec::new();
    let template_name = igr
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| igr.display().to_string());
    let mut seen_fill_rings: HashSet<u64> = HashSet::new();
    for entity in &tpl_geometry.entities {
        if entity.confidence != PidGeometryConfidence::Decoded {
            continue;
        }
        // A placement in a template is the label the sheet letters into the
        // block; there is no body for this importer to draw, so the record
        // stays a record rather than becoming a stray marker dot.
        if matches!(entity.kind, PidGraphicKind::SymbolInstance { .. }) {
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
        let built = match fill {
            Some(fill) => build_fill(&entity.kind, fill, projection),
            // `LAYER_GEOMETRY` is the working layer so `apply_symbology`
            // paints the template's own width and colour; the slot below
            // ends up `PID-FRAME`, which says whose drafting it is.
            None => build_entities(
                &entity.kind,
                BodySources {
                    cached: None,
                    library: None,
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
        if built.is_empty() {
            continue;
        }
        decoded += 1;
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
            staged.push(one);
        }
    }
    if !same_page(&tpl_geometry, &staged, geometry) {
        log::warn!(
            "{}: linked template's page {:?} is not the drawing's {:?}; not overlaid",
            igr.display(),
            template_page_mm(&tpl_geometry, &staged),
            geometry.page_dimensions_mm
        );
        return None;
    }
    let drawn = staged.len();
    for one in staged {
        let _ = doc.add_entity(one);
    }
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
        .sum();
    log::info!(
        "{}: border template drew {drawn} entities from {decoded} decoded records ({missing} template record(s) not drawn)",
        igr.display()
    );
    (drawn > 0).then_some(TemplateOverlay {
        file: igr.to_path_buf(),
        drawn,
        missing,
        page_frame_covered,
    })
}

/// Whether the template's page is the drawing's stated page. Only a page
/// both sides state can disagree; a side that says nothing cannot refuse
/// the match. A border template carries no frame record of its own -- it
/// *is* the frame -- so its page reads off what it drew: the union of the
/// staged entities' extents is the outer border it would have framed.
pub(super) fn same_page(
    template: &NormalizedPidGeometry,
    staged: &[EntityType],
    drawing: &NormalizedPidGeometry,
) -> bool {
    match (
        template_page_mm(template, staged),
        drawing.page_dimensions_mm,
    ) {
        (Some((tw, th)), Some((dw, dh))) => {
            (tw - dw).abs() <= PAGE_MATCH_TOLERANCE_MM && (th - dh).abs() <= PAGE_MATCH_TOLERANCE_MM
        }
        _ => true,
    }
}

/// The page the template claims: stated when it can state one, else the
/// extent of its own drafting.
fn template_page_mm(template: &NormalizedPidGeometry, staged: &[EntityType]) -> Option<(f64, f64)> {
    template.page_dimensions_mm.or_else(|| {
        staged
            .iter()
            .filter_map(drawn_extent)
            .reduce(union_extent)
            .map(|(min_x, min_y, max_x, max_y)| (max_x - min_x, max_y - min_y))
    })
}

/// Which `.igr` an entity came from and which layer the template filed it
/// on, beside the `role=frame` it already carries:
/// `template=CPE-SQ-A1.igr` under the same XDATA application, so a saved
/// drawing still says whose drafting the title block is.
fn attach_template_provenance(
    entity: &mut EntityType,
    template_name: &str,
    template_layer: Option<&str>,
) {
    use codec::xdata::{ExtendedDataRecord, XDataValue};
    let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
    record.add_value(XDataValue::String(format!("template={template_name}")));
    if let Some(layer) = template_layer {
        record.add_value(XDataValue::String(format!("template_layer={layer}")));
    }
    entity.common_mut().extended_data.add_record(record);
}

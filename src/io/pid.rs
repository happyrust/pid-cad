// SmartPlant / Smart P&ID `.pid` import.
//
// `pid-parse` decodes the CFB container's Sheet streams into a normalized
// geometry projection; this module maps the source-backed part of that
// projection onto acadrust entities so a `.pid` opens like any other drawing.
//
// `Decoded` entities are imported as drawing geometry. One kind of
// `Inferred` evidence comes in as well, on its own hidden layer, because its
// coordinates share the decoded geometry's space: the endpoint pairs whose
// two ends both land on the sheet. The rest stays out, and inferred `Point`
// is the part of "the rest" worth naming: it is the largest inferred
// category on every fixture, so dropping it looks like a loss. It is not. Of
// the 321 across the four fixtures, none is drawing content -- 261 come from
// a sliding window over raw bytes rather than from a record, and the ones
// that do land on the sheet either sit at the origin or duplicate a
// placement the drawing already carries. Measured in
// `pid-parse/docs/analysis/2026-08-04-inferred-points-negative-note.md`;
// `igPoint2d` is the only point family the format has, and all of it
// decodes. `ProbeOnly` evidence has no position at all.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use acadrust::entities::hatch::{BoundaryEdge, BoundaryPath, BoundaryPathFlags, LineEdge};
use acadrust::entities::{
    Circle, Hatch, Line, LwPolyline, Point, Text, TextHorizontalAlignment,
};
use acadrust::tables::linetype::{LineType, LineTypeElement};
use acadrust::types::{Color, LineWeight, Vector2, Vector3};
use acadrust::{CadDocument, EntityType, TableEntry};
use pid_parse::style_link::{
    DashPattern, LineStyleIndex, ResolvedFill, ResolvedLineStyle, TextAlignment,
};
use pid_parse::symbol_library::{SymbolLibrary, SymbolPrimitive};
use pid_parse::{
    build_normalized_geometry, NormalizedPidGeometry, PidDrawingUnits, PidGeometryConfidence,
    PidGraphicKind, PidParser, PidPoint, PidSemanticHit, PidSemanticIndex,
};

// Millimetres in a metre, which is the unit a `.pid`'s decoded coordinates
// come in. It is also the assumption a drawing whose unit `pid-parse` could
// not state falls back on: the decoded extents of every fixture land in 0..1
// and match the drawing's own ISO template once multiplied by 1000.
const MM_PER_METRE: f64 = 1000.0;

// SmartPlant carries text height in the style record, which is not decoded
// yet; 2.5mm is the ISO 3098 body-text size a P&ID annotation normally uses.
const TEXT_HEIGHT_MM: f64 = 2.5;

// What a symbol's template text reads as where the drawing is expected to
// supply the value. See `carries_a_label`.
const SYMBOL_TEXT_PLACEHOLDER: &str = "NULL";

// A symbol's body lives in an external `.sym` library the drawing references
// over UNC. With the library on hand the body is drawn; without it, or for a
// symbol the local copy lacks, the placement falls back to this marker so it
// is still visible as something rather than silently absent.
const SYMBOL_MARKER_RADIUS_MM: f64 = 1.5;

// Environment override for the reference-data shares holding the `Design`,
// `Piping`, `Equipment` ... symbol trees, `;` separated like PATH. Without it
// the importer looks for the library next to the drawing.
const SYMBOL_LIBRARY_ENV: &str = "PID_SYMBOL_LIBRARY";

// How far above the drawing to look for that share. A SmartPlant project puts
// the drawing at `Plant\Drawings\Area\x.pid` and the symbols at `Plant\Ref\
// Symbols`, so the walk has to clear the deepest drawing subfolder in use.
const SYMBOL_SEARCH_DEPTH: usize = 5;

// The library file name says what the marker stands for ("Flanged Nozzle",
// "Gauge Hatch"), which is the readable half of a symbol until the body can be
// drawn. Smaller than body text so a label never reads as an annotation, and
// on its own layer because a dense sheet carries dozens of them.
const SYMBOL_LABEL_HEIGHT_MM: f64 = 2.0;
const SYMBOL_LABEL_GAP_MM: f64 = 0.8;

// An annotation record carries an anchor and an orientation but no shape, so
// it is drawn as a stub leaving the anchor along that orientation: the end at
// the anchor is the position, the direction is the decoded angle. A cross
// would hide the angle, since the ones observed are all multiples of 90.
// Nothing reaches this while the anchor read stands retracted; see
// `build_inferred`.
const ANNOTATION_TICK_MM: f64 = 3.0;

// Shortest connectivity link worth drawing, and the same bound used to tell an
// endpoint that decoded as the origin from one that genuinely sits in the
// sheet's bottom-left corner. A P&ID's own line work is millimetres apart at
// the closest, so a tenth of one separates "two places" from "one place
// twice".
const CONNECTIVITY_MIN_MM: f64 = 0.1;

// How far past the page edge a coordinate can sit and still be part of the
// drawing. Wide enough for a symbol whose insertion point a misparse nudged
// off the border, narrow enough to reject the metres-off strays that framing
// and the connectivity filter exist to keep out.
const SHEET_MARGIN_MM: f64 = 100.0;

// XDATA application name carrying an entity's published P&ID identity
// (`class=…`, `label=…`, `oid=…`, `resolved=…` string pairs). Written only
// when a `<stem>_Data.xml` sits beside the drawing; the properties panel
// shows a "P&ID" group for entities that carry it and no group otherwise.
pub(crate) use super::PID_SEMANTICS_XDATA_APP;

// Angles cross this module unchanged, because both sides already agree on
// radians: `pid-parse` states them that way, and so does the in-memory
// drawing model -- `src/io/mod.rs`'s `fix_dxf_dimension_rotations` exists
// precisely to convert the DXF reader's degrees into radians on load, and
// `entities::text` adds `PI` to `Text::rotation` for upside-down text.
//
// Six assignments here used to call `to_degrees()` on the way in. It was
// invisible while every angle was zero: `pid-parse` hard-coded text rotation
// to `0.0`, and `0.0f64.to_degrees()` is still `0.0`. Decoding the real
// rotation made it visible -- a quarter turn arrived as 90 *radians*, which
// renders at 116 degrees. The symbol library's arcs and labels were wrong
// the whole time.
//
// The assignments below point back here, so the next person tempted to write
// `.to_degrees()` in this file has something to read first.

const LAYER_GEOMETRY: &str = "PID-GEOMETRY";
const LAYER_TEXT: &str = "PID-TEXT";
const LAYER_SYMBOL: &str = "PID-SYMBOL";
const LAYER_SYMBOL_LABEL: &str = "PID-SYMBOL-LABEL";
const LAYER_POINT: &str = "PID-POINT";
const LAYER_ANNOTATION: &str = "PID-ANNOTATION";
const LAYER_CONNECTIVITY: &str = "PID-CONNECTIVITY";
const LAYER_FILL: &str = "PID-FILL";
const LAYER_FRAME: &str = "PID-FRAME";

/// What the import wants the reader to know, sized for one command-line line:
/// how much of the file became drawing, how much did not, and whether the
/// style tables were readable at all. The detail behind each number stays in
/// the log (see [`report_import`]); this is the headline.
#[derive(Clone, Copy)]
pub struct ImportSummary {
    /// Entities handed to the document.
    pub drawn: usize,
    /// Decoded source records those entities came from.
    pub decoded: usize,
    /// Source records the parser saw but could not place: no decoder, or a
    /// shape the decoder for that type refuses.
    pub missing: usize,
    /// A style table failed to read outright, so line work, lettering or
    /// fills are on their fallbacks rather than the drawing's own statement.
    pub style_tables_failed: bool,
}

/// Mailbox carrying each import's summary out of the io layer, keyed by the
/// drawing's path.
///
/// `acadrust::ReadOutcome` is a foreign type with no room for extra freight,
/// so the summary rides beside it: written when the import finishes, taken by
/// the open-completion handler that installs the tab. Keying by path keeps
/// concurrent imports (tests run them in parallel) out of each other's slots;
/// an entry an abandoned open leaves behind is overwritten by the next import
/// of that path rather than shown against the wrong drawing.
static IMPORT_SUMMARIES: std::sync::Mutex<std::collections::BTreeMap<PathBuf, ImportSummary>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// The summary the last [`load_pid`] of `path` left behind, draining it.
pub fn take_import_summary(path: &Path) -> Option<ImportSummary> {
    IMPORT_SUMMARIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(path)
}

/// Parse a `.pid` file and project its decoded Sheet geometry into a document.
pub fn load_pid(path: &Path) -> Result<CadDocument, String> {
    let parsed = PidParser::new()
        .parse_file(path)
        .map_err(|error| error.to_string())?;
    let geometry = build_normalized_geometry(&parsed);

    let mut doc = CadDocument::new();
    crate::io::linetypes::populate_document(&mut doc);
    // A DWG carries `$LWDISPLAY` in its header; a `.pid` has no header to read
    // it from, so a fresh document's `false` would stand. Every line here comes
    // in at a width read off the drawing's own style table (see
    // `apply_symbology`), and with the flag off the wire shader collapses all
    // of them to a hairline -- a 0.13mm instrument line and a 0.7mm process
    // header would look identical on screen, which is the thing reading that
    // table exists to fix.
    doc.header.lineweight_display = true;
    // Colours separate the kinds at a glance: a P&ID is mostly line work, and
    // an all-white import makes lettering, symbol bodies and the decode's own
    // loose ends indistinguishable from the piping.
    for (layer, colour, visible) in [
        (LAYER_GEOMETRY, Color::WHITE, true),
        // The sheet's own border. It is part of the drawing rather than
        // evidence about it, so it opens visible and in the same colour as
        // the line work it encloses.
        (LAYER_FRAME, Color::WHITE, true),
        (LAYER_TEXT, Color::GREEN, true),
        // Filled areas. Each fill carries its own decoded colour, so the layer
        // colour is only the default for a fill that states none; white keeps
        // those consistent with the line work they belong to.
        (LAYER_FILL, Color::WHITE, true),
        (LAYER_SYMBOL, Color::CYAN, true),
        // "Flanged Nozzle with blind" is wider than the equipment it names, so
        // on a sheet with 58 placements the labels bury the drawing. They ship
        // switched off: the answer is in the file, one layer toggle away.
        (LAYER_SYMBOL_LABEL, Color::GRAY, false),
        (LAYER_POINT, Color::MAGENTA, true),
        // There is no `PID-UNRESOLVED`. It held the `GLine2d` unit lines,
        // which turned out not to be records at all: each was the top two
        // bytes of an `igSmartFrame2d`'s page ratio, matched by a decoder
        // that scanned rather than walked the record chain. `pid-parse`
        // emits none now -- see its
        // `docs/analysis/2026-08-10-gline2d-is-the-iso-page-ratio-not-a-record.md`.
        // Unlike `PID-ANNOTATION`, whose records really are in the file with
        // an unread anchor, there is nothing left for an empty layer to
        // stand for.
        // Empty since the `JStyleOverride` anchor read was retracted -- see
        // `build_inferred`. Still declared, and still hidden: the records are
        // in the file, and a layer that is present and empty says so where a
        // missing one would not.
        (LAYER_ANNOTATION, Color::YELLOW, false),
        (LAYER_CONNECTIVITY, Color::BLUE, false),
    ] {
        ensure_layer(&mut doc, layer, colour, visible);
    }

    let mut library = discover_symbol_library(path);

    // Line width and colour are the drawing's own, read from its style table:
    // each geometry record names a style id, and that record carries a width
    // in metres and a Win32 COLORREF. See `pid-parse`'s `style_link` module
    // and `docs/analysis/2026-08-05-geometry-index-is-the-style-link.md`.
    // Until this landed every line came in at the layer's white default, so a
    // 0.13mm instrument line and a 0.7mm process header looked identical.
    // A file whose style table will not read keeps that old behaviour rather
    // than failing the import -- but says so, in the log and in the summary,
    // because an all-default sheet is otherwise indistinguishable from a
    // drawing that genuinely states no styles.
    let mut style_tables_failed = false;
    let styles = pid_parse::style_link::line_styles_for_file(path).unwrap_or_else(|error| {
        style_tables_failed = true;
        log::warn!(
            "{}: the line style table did not read; line work keeps the layer defaults: {error}",
            path.display()
        );
        Default::default()
    });
    // Character height comes from the same table, one hop further along: a
    // text record names a paragraph style, and the height is on the character
    // style that paragraph style names. Most of a P&ID's lettering turns out
    // to be 1/8 inch, so `TEXT_HEIGHT_MM` was reading a quarter too small.
    // Records whose height does not resolve keep that fallback.
    let text_heights =
        pid_parse::style_link::text_heights_for_file(path).unwrap_or_else(|error| {
            style_tables_failed = true;
            log::warn!(
                "{}: the text style table did not read; lettering keeps the {TEXT_HEIGHT_MM}mm fallback: {error}",
                path.display()
            );
            Default::default()
        });
    // Which areas the drawing fills. `pid-parse` resolves an `igBoundary2d`
    // ring through its `JStyleOverride` to a `JStyleSimpleFill`; the fill's
    // own colour is not decoded, so a filled ring is drawn in its layer's
    // colour. On the reference corpus these are the solid flow arrowheads on
    // the pipelines -- 5 on DWG-0202 and 10 on the gongyi drawing, all of
    // which used to import as hollow triangles.
    let fills = pid_parse::style_link::fill_styles_for_file(path).unwrap_or_else(|error| {
        style_tables_failed = true;
        log::warn!(
            "{}: the fill style table did not read; boundary rings import as outlines: {error}",
            path.display()
        );
        Default::default()
    });
    // A line's dash pattern comes from the same style table, one reference
    // further along: a JStyleSimpleLine names a JStyleSimpleDashType, and
    // style_link hands the decoded segments back. Pool the distinct patterns
    // into named document linetypes now, so `apply_symbology` can name each
    // dashed line to one and the renderer dashes it like any other linetype.
    // See pid-parse's `docs/analysis/2026-08-07-jstyle-simple-dash-type-linetype.md`.
    let dash_linetypes = register_dash_linetypes(&mut doc, &styles);
    // Same pooling for the typefaces the character styles name, so a label can
    // reference a document text style the way any other text entity does.
    let font_styles = register_text_styles(&mut doc, &text_heights);
    // The published semantic model, when the drawing ships one: SmartPlant
    // publishes `<stem>_Data.xml` beside the `.pid`, and pid-parse joins its
    // GraphicOIDs onto the decoded records (two-hop rule, see pid-parse's
    // `docs/analysis/2026-08-07-graphic-oid-is-the-semantic-join.md`). A
    // drawing without one imports exactly as before -- the XML is an
    // enrichment, never a prerequisite.
    let semantics = PidSemanticIndex::load_beside(path, &parsed);
    // The DWG writer skips XDATA whose application is not in the APPID
    // table, so without this registration the identities would survive the
    // session and silently vanish on save (see `set_entity_xdata`).
    if semantics.is_some() && !doc.app_ids.contains(PID_SEMANTICS_XDATA_APP) {
        let mut app = acadrust::tables::AppId::new(PID_SEMANTICS_XDATA_APP);
        app.handle = doc.allocate_handle();
        let _ = doc.app_ids.add(app);
    }

    let page_mm = geometry.page_dimensions_mm;
    let projection = Projection::for_geometry(&geometry, path);
    let mut bounds = Bounds::new(projection);
    let mut decoded = 0usize;
    let mut drawn = 0usize;
    let mut lettering_on_fallback = 0usize;
    for entity in &geometry.entities {
        // A boundary ring is the one kind whose style decides its shape rather
        // than its colour: filled, it is an area; unfilled, it is an outline
        // the member lines already drew. See `build_fill`.
        let fill = fill_for(&fills, entity);
        let built = match entity.confidence {
            PidGeometryConfidence::Decoded => match fill {
                Some(fill) => build_fill(&entity.kind, fill, projection),
                None => build_entities(&entity.kind, library.as_mut(), projection),
            },
            PidGeometryConfidence::Inferred => build_inferred(&entity.kind, projection),
            PidGeometryConfidence::ProbeOnly => Vec::new(),
        };
        if built.is_empty() {
            continue;
        }
        // Only the decoded geometry decides where the camera goes. An
        // inferred item is evidence about the drawing rather than the
        // drawing, and it is the kind that strays: see `accumulate_bounds`.
        if entity.confidence == PidGeometryConfidence::Decoded {
            decoded += 1;
            accumulate_bounds(&entity.kind, &mut bounds);
        }
        drawn += built.len();
        let symbology = style_for(&styles, entity);
        let text_style = height_for(&text_heights, entity);
        let height_mm = text_style.map(|h| projection.mm(h.height_m));
        // The same character style states the colour, so it comes off the
        // join already made rather than a second one.
        let text_rgb = text_style.and_then(pid_parse::style_link::ResolvedTextHeight::rgb);
        // Alignment rides the same join but comes off the paragraph rather
        // than the character style, since it belongs to the run.
        let text_alignment = text_style.and_then(|style| style.alignment);
        // And the typeface, which was registered as a document text style
        // above, so what lands on the entity is that style's name.
        let font_style = text_style
            .and_then(|style| style.font_name.as_deref())
            .and_then(|font| font_styles.get(font))
            .map(String::as_str);
        if height_mm.is_none() && matches!(entity.kind, PidGraphicKind::Text { .. }) {
            lettering_on_fallback += 1;
        }
        let semantic_hit = semantics.as_ref().and_then(|index| {
            entity
                .graphic_oid
                .and_then(|graphic_oid| index.resolve(graphic_oid))
        });
        for mut one in built {
            if let Some(style) = symbology {
                apply_symbology(&mut one, style, &dash_linetypes);
            }
            if let Some(mm) = height_mm {
                apply_text_height(&mut one, mm);
            }
            if let Some(rgb) = text_rgb {
                apply_text_colour(&mut one, rgb);
            }
            if let Some(alignment) = text_alignment {
                apply_text_alignment(&mut one, alignment);
            }
            if let Some(name) = font_style {
                apply_text_style(&mut one, name);
            }
            if let Some(hit) = &semantic_hit {
                attach_semantics(&mut one, hit);
            }
            let _ = doc.add_entity(one);
        }
    }

    if decoded == 0 {
        return Err(format!(
            "No decoded geometry in {}: pid-parse produced {} evidence item(s), none of them source-backed",
            path.display(),
            geometry.entities.len()
        ));
    }

    report_import(
        path,
        &geometry,
        library.as_ref(),
        drawn,
        lettering_on_fallback,
    );
    // The headline the open-completion handler shows on the command line;
    // the counts agree with `report_import`'s log lines by construction.
    let missing: usize = geometry
        .dropped_graphic_records
        .iter()
        .map(|dropped| dropped.count)
        .chain(
            geometry
                .refused_graphic_records
                .iter()
                .map(|refused| refused.count),
        )
        .sum();
    IMPORT_SUMMARIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            path.to_path_buf(),
            ImportSummary {
                drawn,
                decoded,
                missing,
                style_tables_failed,
            },
        );
    draw_page_border(&mut doc, page_mm);
    frame_drawing(&mut doc, &bounds, page_mm);
    doc.source_path = Some(path.to_string_lossy().into_owned());
    Ok(doc)
}

/// Draw the sheet the drawing states it is on.
///
/// A P&ID's border is an OLE object linked into the sheet rather than line
/// work, so it decodes as a page size and nothing else: `pid-parse` hands
/// over the extent, and until this is drawn the content hangs in the middle
/// of an empty background with no edge to read it against.
///
/// Only the rectangle is drawn. The title block, the revision table and the
/// grid divisions inside a real border are the template's own drafting, and
/// synthesising them would be inventing content the file does not carry.
fn draw_page_border(doc: &mut CadDocument, page_mm: Option<(f64, f64)>) {
    let Some((width, height)) = page_mm else {
        return;
    };
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return;
    }
    let mut border = LwPolyline::from_points(vec![
        Vector2::new(0.0, 0.0),
        Vector2::new(width, 0.0),
        Vector2::new(width, height),
        Vector2::new(0.0, height),
    ]);
    border.is_closed = true;
    border.common.layer = LAYER_FRAME.to_string();
    let _ = doc.add_entity(EntityType::LwPolyline(border));
}

/// How a drawing's source coordinates become millimetres on its sheet.
///
/// Both halves used to be constants written into this file. `pid-parse` now
/// states the unit its decoded coordinates are in and the page they sit on,
/// so the conversion and the on-sheet test are read off the drawing instead
/// of assumed about it.
#[derive(Clone, Copy)]
struct Projection {
    mm_per_unit: f64,
    band: SheetBand,
}

impl Projection {
    fn for_geometry(geometry: &NormalizedPidGeometry, path: &Path) -> Self {
        Self {
            mm_per_unit: mm_per_source_unit(geometry, path),
            band: SheetBand::for_page(geometry.page_dimensions_mm),
        }
    }

    /// A source coordinate in millimetres.
    fn mm(self, value: f64) -> f64 {
        value * self.mm_per_unit
    }

    /// A source point in millimetres, flat on the sheet.
    fn point(self, point: &PidPoint) -> Vector3 {
        Vector3::new(self.mm(point.x), self.mm(point.y), 0.0)
    }
}

/// The millimetres a source unit is worth, as the parser states it.
///
/// `pid-parse` puts the unit on each entity's coordinate context rather than
/// on the document, and only decoded records carry one, so those are what is
/// read. They agree with each other by construction, because the unit comes
/// from the sheet's single page frame. A drawing whose frame the parser could
/// not decode says nothing about units and falls back to the metre, which is
/// what every fixture measured before the parser could say so -- and the log
/// then records having assumed it.
fn mm_per_source_unit(geometry: &NormalizedPidGeometry, path: &Path) -> f64 {
    let stated = geometry
        .entities
        .iter()
        .filter(|entity| entity.confidence == PidGeometryConfidence::Decoded)
        .find_map(|entity| millimetres_in(&entity.coordinate_context.units));
    stated.unwrap_or_else(|| {
        log::info!(
            "{}: no decoded coordinate unit; assuming the metre, {MM_PER_METRE}mm per source unit",
            path.display()
        );
        MM_PER_METRE
    })
}

/// Millimetres in one of the units `pid-parse` can state.
///
/// The unit arrives as a label rather than a factor. The metre is the only
/// one a decoded page frame produces; the millimetre is here because it is
/// the identity and costs nothing to be right about. Any other label is a
/// unit this importer has not been shown, and guessing at its factor would be
/// worse than falling back and saying so.
fn millimetres_in(units: &PidDrawingUnits) -> Option<f64> {
    let PidDrawingUnits::Known { unit } = units else {
        return None;
    };
    match unit.trim().to_ascii_lowercase().as_str() {
        "m" => Some(MM_PER_METRE),
        "mm" => Some(1.0),
        _ => None,
    }
}

/// The band a converted coordinate has to fall in to count as part of the
/// drawing.
///
/// Framing and the connectivity filter both have to tell a coordinate on the
/// sheet from one a misparse threw kilometres away. Where the drawing states
/// its page that is the page plus a margin; without one it falls back to a
/// window wide enough for an oversized custom sheet, since the largest ISO
/// size, A0, is 1189 x 841mm.
#[derive(Clone, Copy)]
struct SheetBand {
    min: f64,
    max: f64,
}

impl SheetBand {
    fn for_page(page_mm: Option<(f64, f64)>) -> Self {
        match page_mm {
            Some((width, height)) if width.is_finite() && height.is_finite() => Self {
                min: -SHEET_MARGIN_MM,
                max: width.max(height) + SHEET_MARGIN_MM,
            },
            _ => Self {
                min: -SHEET_MARGIN_MM,
                max: 2000.0,
            },
        }
    }

    /// Whether a converted coordinate could plausibly sit on the sheet.
    ///
    /// Only framing and connectivity are filtered; the entity itself is still
    /// imported, so zooming out still finds it.
    fn holds(self, value: f64) -> bool {
        value.is_finite() && (self.min..=self.max).contains(&value)
    }
}

/// Say what the import could not draw, in the log rather than on the sheet.
///
/// The gaps a reader hits in practice are silent otherwise: evidence the
/// parser could not decode looks like a sparse drawing, an unreachable symbol
/// library looks like a sheet full of dots, and lettering the drawing gave no
/// usable height for looks like lettering the drawing sized at 2.5mm. All
/// three are recoverable -- the second by pointing [`SYMBOL_LIBRARY_ENV`] at a
/// local copy -- but only if the import says so.
fn report_import(
    path: &Path,
    geometry: &pid_parse::NormalizedPidGeometry,
    library: Option<&SymbolLibrary>,
    drawn: usize,
    lettering_on_fallback: usize,
) {
    for warning in &geometry.warnings {
        log::debug!("{}: {warning}", path.display());
    }

    // Content the vendor's own graphic predicate says should draw, which
    // pid-parse has no decoder for, is a named warning rather than a debug
    // line: an igDimension / igBalloon / igLeader class silently vanishing
    // is exactly the failure a reader cannot notice on their own. Kept
    // separate from the inferred-or-probe-only aggregate below so the drop
    // is called by name (Phase 38 S2).
    for dropped in &geometry.dropped_graphic_records {
        let class_name = dropped
            .rad_class_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        log::warn!(
            "{}: {} record(s) of graphic type 0x{:04X}{} in {} have no decoder; that content is missing from the drawing",
            path.display(),
            dropped.count,
            dropped.type_code,
            class_name,
            dropped.stream_path
        );
    }

    // The other way content goes missing, and on the reference corpus the
    // larger one: a record whose type pid-parse does decode, in a shape its
    // decoder refuses. Refusing beats guessing -- a decoder that read an
    // unknown framing would draw fiction -- but the reader still has a
    // drawing with strokes missing, so it is named the same way. Worded
    // apart from the no-decoder case because the two ask for different work.
    for refused in &geometry.refused_graphic_records {
        let class_name = refused
            .rad_class_name
            .as_deref()
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        log::warn!(
            "{}: {} record(s) of graphic type 0x{:04X}{} in {} are a shape pid-parse's decoder for that type refuses; that content is missing from the drawing",
            path.display(),
            refused.count,
            refused.type_code,
            class_name,
            refused.stream_path
        );
    }

    // The headline number a thin-looking sheet is read against: how much of
    // the file reached the drawing, and how much the parser saw but could not
    // place. Counting the evidence rather than the entities keeps it
    // comparable with `pid-parse`'s own coverage reporting, so the two agree
    // on how much of the file is understood.
    let mut placed = 0usize;
    let mut undrawn = 0usize;
    for entity in &geometry.entities {
        match entity.confidence {
            PidGeometryConfidence::Decoded => placed += 1,
            PidGeometryConfidence::Inferred | PidGeometryConfidence::ProbeOnly => undrawn += 1,
        }
    }
    log::info!(
        "{}: drew {drawn} entit(ies) from {placed} decoded record(s); {undrawn} further evidence item(s) are inferred or probe-only and are hidden or dropped",
        path.display()
    );

    // Lettering the drawing states no usable height for. The style chain
    // itself resolves -- across the reference corpus all 184 text records
    // reach a character style -- but 25 of them reach one storing 0.254mm
    // (0.01"), which no drawing letters at, so `style_link` refuses it and
    // this importer keeps `TEXT_HEIGHT_MM`. Measured in `pid-parse`'s
    // `docs/analysis/2026-08-10-text-height-residue-is-one-sentinel-not-version-2.md`.
    // Silent, this reads as lettering the drawing sized at 2.5mm.
    if lettering_on_fallback > 0 {
        log::warn!(
            "{}: {lettering_on_fallback} text record(s) state no usable character height; they are lettered at the {TEXT_HEIGHT_MM}mm ISO 3098 fallback rather than a height read off the drawing",
            path.display()
        );
    }

    let Some(library) = library else {
        log::warn!(
            "{}: no symbol library found; every symbol is drawn as a marker. Set {SYMBOL_LIBRARY_ENV} to a local copy of the project's reference-data Symbols share.",
            path.display()
        );
        return;
    };
    let missing = library.missing();
    if missing.is_empty() {
        return;
    }
    log::warn!(
        "{}: {} of {} symbol(s) are not in the library at {:?}; they are drawn as markers. First missing: {}",
        path.display(),
        missing.len(),
        library.lookups(),
        library.roots(),
        missing.iter().take(3).copied().collect::<Vec<_>>().join(", ")
    );
}

/// Write an entity's published P&ID identity into its XDATA, under
/// [`PID_SEMANTICS_XDATA_APP`] as self-describing `key=value` strings.
///
/// The values come from the drawing's own published `_Data.xml`, joined by
/// `pid-parse`'s semantic index: `class` is the owning object's XML element
/// name (`PIDPipeline`, `PIDProcessVessel`, …), `label` its `ItemTag` /
/// `Name`, `oid` the published `GraphicOID`, and `resolved` says which hop
/// found it (`direct`, or `dependency:<aggregate oid>`). No new layer is
/// involved: identity is data about an entity, not a place to put one.
fn attach_semantics(entity: &mut EntityType, hit: &PidSemanticHit<'_>) {
    use acadrust::xdata::{ExtendedDataRecord, XDataValue};

    fn push_pair(record: &mut ExtendedDataRecord, key: &str, value: &str) {
        if !value.is_empty() {
            record.add_value(XDataValue::String(format!("{key}={value}")));
        }
    }

    let object = hit.object();
    let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
    push_pair(&mut record, "class", &object.class);
    if let Some(label) = object.label() {
        push_pair(&mut record, "label", label);
    }
    push_pair(&mut record, "oid", &object.graphic_oid.to_string());
    match hit {
        PidSemanticHit::Direct(_) => push_pair(&mut record, "resolved", "direct"),
        PidSemanticHit::ViaDependency { dependency_oid, .. } => {
            push_pair(
                &mut record,
                "resolved",
                &format!("dependency:{dependency_oid}"),
            );
        }
    }
    entity.common_mut().extended_data.add_record(record);
}

/// The style table's entry for one normalized entity, if it has one.
///
/// The join is `(stream path, graphic oid)`, which is what `pid-parse` keys
/// the table on. Only the three families that carry a style reference are in
/// it, so text, symbols and every kind of evidence simply miss.
fn style_for<'a>(
    styles: &'a LineStyleIndex,
    entity: &pid_parse::PidGraphicEntity,
) -> Option<&'a ResolvedLineStyle> {
    let stream = entity.source.stream_path.as_deref()?;
    let oid = entity.graphic_oid?;
    styles.get(&(stream.to_string(), oid))
}

/// The fill the drawing states for one boundary ring, if it states one. Same
/// `(stream path, graphic oid)` join as [`style_for`].
fn fill_for<'a>(
    fills: &'a pid_parse::style_link::FillIndex,
    entity: &pid_parse::PidGraphicEntity,
) -> Option<&'a pid_parse::style_link::ResolvedFill> {
    let stream = entity.source.stream_path.as_deref()?;
    let oid = entity.graphic_oid?;
    fills.get(&(stream.to_string(), oid))
}

/// Draw a filled area rather than its outline.
///
/// `pid-parse` emits an `igBoundary2d` as a closed polyline whose segments
/// re-list the member `igLine2d` records that already drew the outline. So
/// stroking it again would only thicken what is there; what the member lines
/// cannot say is that the ring is *filled*. This turns the ring into a solid
/// `HATCH`, which is the entity the renderer already fills.
///
/// The fill takes the colour `JStyleSimpleFill` states at payload +30, a
/// Win32 `COLORREF` decoded the same way as a line's. On the reference corpus
/// the flow arrowheads read `#0000FF`, so they now come in blue rather than
/// the layer's white. A fill that states no colour -- a hatch, or the "unset"
/// sentinel every document's template fill carries -- keeps the layer default,
/// which is what "no stated colour" asks for anyway.
fn build_fill(
    kind: &PidGraphicKind,
    fill: &ResolvedFill,
    projection: Projection,
) -> Vec<EntityType> {
    let PidGraphicKind::Polyline { points, .. } = kind else {
        return Vec::new();
    };
    if points.len() < 3 {
        return Vec::new();
    }
    let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    for (from, to) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        path.edges.push(BoundaryEdge::Line(LineEdge {
            start: Vector2::new(projection.mm(from.x), projection.mm(from.y)),
            end: Vector2::new(projection.mm(to.x), projection.mm(to.y)),
        }));
    }
    // `Hatch::new` is already a solid; the ring only has to be handed over.
    let mut hatch = Hatch::new();
    hatch.paths.push(path);
    hatch.common.layer = LAYER_FILL.to_string();
    if let Some([r, g, b]) = fill.rgb() {
        hatch.common.color = Color::from_rgb(r, g, b);
    }
    vec![EntityType::Hatch(hatch)]
}

/// The character height the drawing states for one text record, if it has
/// one. Same `(stream path, graphic oid)` join as [`style_for`].
fn height_for<'a>(
    heights: &'a pid_parse::style_link::TextHeightIndex,
    entity: &pid_parse::PidGraphicEntity,
) -> Option<&'a pid_parse::style_link::ResolvedTextHeight> {
    let stream = entity.source.stream_path.as_deref()?;
    let oid = entity.graphic_oid?;
    heights.get(&(stream.to_string(), oid))
}

/// Letter a text entity at the height its style states, in millimetres.
///
/// Only the sheet's own lettering. A symbol's internal text is drawn from the
/// `.sym` library, whose records carry no height of their own, so it keeps
/// the scaled fallback rather than borrowing a height from the label beside
/// it.
fn apply_text_height(entity: &mut EntityType, height_mm: f64) {
    if !(height_mm.is_finite() && height_mm > 0.0) {
        return;
    }
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.height = height_mm;
        }
    }
}

/// Letter a text entity in the colour its character style states.
///
/// Scoped the way [`apply_text_height`] is: only the sheet's own lettering on
/// [`LAYER_TEXT`]. A symbol's internal text comes from the `.sym` library and
/// states no colour of its own, and the diagnostic layers' colours *are* the
/// diagnosis -- the same reason [`apply_symbology`] refuses to paint them.
///
/// Most of a P&ID letters in black, which on the editor's dark background the
/// renderer flips to white (`scene::view::render::adapt_to_bg`) exactly as it
/// already does for the drawing's black line work. So this is visible as
/// colour where the drawing states one, and as no change at all where it
/// states the black that most lettering uses.
fn apply_text_colour(entity: &mut EntityType, [r, g, b]: [u8; 3]) {
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.common.color = Color::from_rgb(r, g, b);
        }
    }
}

/// Letter a text entity from the side its paragraph style states.
///
/// Half the labels a P&ID reaches are centred or right-aligned, and until this
/// they all rendered from the left -- each of those runs sitting half a label
/// away from where the drawing puts it.
///
/// The insertion point does double duty in a TEXT entity: it is the run origin
/// only while the alignment is left-on-baseline, and otherwise the origin is
/// `alignment_point` instead. Setting the alignment alone would therefore move
/// the run to whatever `alignment_point` happened to hold -- which is nothing
/// -- so [`sync_text_alignment_point`] seeds it from the insertion point in
/// the same breath. The failure mode that guards against is not subtle: the
/// label lands at the origin rather than half a word off.
///
/// Scoped like [`apply_text_height`] and [`apply_text_colour`]: the sheet's
/// own lettering only. A symbol's internal text is placed by the `.sym`
/// library, which states its own alignment.
fn apply_text_alignment(entity: &mut EntityType, alignment: TextAlignment) {
    use crate::entities::text::sync_text_alignment_point;

    if let EntityType::Text(text) = entity {
        if text.common.layer != LAYER_TEXT {
            return;
        }
        // The two enums agree on 0/1/2 by coincidence of both following the
        // DXF convention, but they are different types, so the mapping is
        // written out rather than cast.
        text.horizontal_alignment = match alignment {
            TextAlignment::Left => TextHorizontalAlignment::Left,
            TextAlignment::Center => TextHorizontalAlignment::Center,
            TextAlignment::Right => TextHorizontalAlignment::Right,
        };
        sync_text_alignment_point(text);
    }
}

/// Letter a text entity in the typeface its character style names.
///
/// The entity names a document text style rather than carrying the typeface
/// itself, which is how every other text entity in the application works --
/// see [`register_text_styles`] for what those styles hold.
///
/// Scoped like [`apply_text_height`], [`apply_text_colour`] and
/// [`apply_text_alignment`]: the sheet's own lettering only. A symbol's
/// internal text is drawn from the `.sym` library and names no typeface.
fn apply_text_style(entity: &mut EntityType, style_name: &str) {
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.style = style_name.to_string();
        }
    }
}

/// Give an entity the width and colour its source record asks for.
///
/// Only the two layers carrying the drawing's own line work are painted.
/// `PID-CONNECTIVITY` is a diagnostic whose layer colour *is* the diagnosis,
/// and repainting it in the drawing's palette would hide the thing it exists
/// to show.
fn apply_symbology(
    entity: &mut EntityType,
    style: &ResolvedLineStyle,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) {
    let common = entity.common_mut();
    if common.layer != LAYER_GEOMETRY && common.layer != LAYER_POINT {
        return;
    }
    let [r, g, b] = style.symbology.rgb();
    common.color = Color::from_rgb(r, g, b);
    // DXF stores line weight in hundredths of a millimetre. The source values
    // are mostly on the ISO 128 ladder; the 0.10mm point ticks are not, and
    // are carried across as measured rather than snapped, because snapping
    // would report a width the drawing does not ask for.
    let hundredths = (style.symbology.width_mm() * 100.0).round();
    if (0.0..=211.0).contains(&hundredths) {
        common.line_weight = LineWeight::Value(hundredths as i16);
    }
    // The dashed linetype the line draws with, resolved through the same table
    // as every other linetype so the renderer dashes it with no special case.
    // A solid line names none and keeps the layer's Continuous default.
    if let Some(dash) = style.dash.as_ref() {
        if let Some(name) = dash_linetypes.get(&dash_key(dash)) {
            common.linetype = name.clone();
        }
    }
}

/// A dedup key for a dash pattern: its segment magnitudes, in micrometres.
///
/// Two patterns that differ only in sign render identically — see
/// [`build_dash_linetype`] — so the key is built from magnitudes, and the
/// micrometre rounding folds together patterns that agree to within a
/// nanometre of float noise.
fn dash_key(dash: &DashPattern) -> Vec<i64> {
    dash.segments_mm()
        .iter()
        .map(|mm| (mm.abs() * 1_000.0).round() as i64)
        .collect()
}

/// Register one document linetype per distinct decoded dash pattern, returning
/// the map from a pattern's [`dash_key`] to the linetype name it was given.
///
/// A `.pid` carries only a handful of distinct patterns, so pooling them into
/// named entries in the same table [`crate::io::linetypes::populate_document`]
/// fills lets [`apply_symbology`] name a line to one and the renderer dash it
/// through its ordinary `resolve_pattern` path — no differently from a linetype
/// a DWG shipped. The names are assigned over a `BTreeMap`, so they are stable
/// for a given file.
fn register_dash_linetypes(
    doc: &mut CadDocument,
    styles: &LineStyleIndex,
) -> HashMap<Vec<i64>, String> {
    let mut names: HashMap<Vec<i64>, String> = HashMap::new();
    for style in styles.values() {
        let Some(dash) = style.dash.as_ref() else {
            continue;
        };
        let key = dash_key(dash);
        if key.is_empty() || names.contains_key(&key) {
            continue;
        }
        let name = format!("PID-DASH-{}", names.len() + 1);
        if !doc.line_types.contains(&name) {
            let mut lt = build_dash_linetype(&name, dash);
            lt.set_handle(doc.allocate_handle());
            let _ = doc.line_types.add(lt);
        }
        names.insert(key, name);
    }
    names
}

/// Register one document text style per distinct typeface the drawing's
/// character styles name, returning the map from typeface to the style name it
/// was given.
///
/// Pooled the way [`register_dash_linetypes`] pools patterns, and for the same
/// reason: a `.pid` names a handful of typefaces, and once each is a named
/// `TextStyle` the lettering resolves through
/// [`crate::entities::text_support::resolve_text_style`] like a style any DWG
/// shipped. That resolver prefers `true_type_font` and looks the name up among
/// the installed system fonts, which is where the vendor's name goes, verbatim.
///
/// **`height` stays 0.** A `TextStyle` height is a *fixed* height that
/// overrides the entity's own, and every one of these entities has already
/// been given the height its character style states.
///
/// A typeface the corpus cannot match -- twelve of the 381 corpus names are
/// damaged on the vendor's side, and none of them is reached by text today --
/// gets a style like any other. `resolve_text_style` finds no such font and
/// falls back, which is the intended outcome: better than writing a
/// reconstructed name nobody measured.
///
/// Names are assigned over a `BTreeSet`, so they are stable for a given file.
fn register_text_styles(
    doc: &mut CadDocument,
    text_heights: &pid_parse::style_link::TextHeightIndex,
) -> HashMap<String, String> {
    let fonts: BTreeSet<&str> = text_heights
        .values()
        .filter_map(|style| style.font_name.as_deref())
        .collect();
    let mut names = HashMap::new();
    for font in fonts {
        let base = text_style_name(font);
        let mut name = base.clone();
        let mut suffix = 2;
        // The table matches case-insensitively, so asking it is also what
        // keeps two typefaces that sanitise alike from colliding.
        while doc.text_styles.contains(&name) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut style = acadrust::tables::TextStyle::new(&name);
        style.true_type_font = font.to_string();
        style.set_handle(doc.allocate_handle());
        if doc.text_styles.add(style).is_ok() {
            names.insert(font.to_string(), name);
        }
    }
    names
}

/// Turn a typeface name into a symbol-table name for the style that carries it.
///
/// The name only has to be stable, unique and readable, since the typeface
/// itself travels in `true_type_font` -- so anything that is not a letter,
/// digit or underscore becomes a hyphen. That includes the space in
/// `Arial Narrow`: modern DXF permits one in a symbol name and R12 did not,
/// and `PID-Arial-Narrow` reads the same either way. CJK names come through as
/// themselves, since they are alphanumeric.
fn text_style_name(font: &str) -> String {
    let mut body = String::new();
    for ch in font.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            body.push(ch);
        } else if !body.is_empty() && !body.ends_with('-') {
            body.push('-');
        }
    }
    // Well short of the 255-character symbol-name limit. A typeface name
    // longer than this is damage rather than a name, and the caller's
    // uniqueness loop numbers apart any two that truncate alike.
    let body: String = body.chars().take(64).collect();
    let body = body.trim_end_matches('-');
    if body.is_empty() {
        return "PID-FONT".to_string();
    }
    format!("PID-{body}")
}

/// Build a document linetype from a decoded dash pattern.
///
/// The segment lengths are the drawing's own, in millimetres — the unit the
/// geometry is projected into — so a 3.5 mm dash is 3.5 mm on the sheet. The
/// elements are laid out the way every AutoCAD "A"-type linetype is: drawn and
/// gap alternate, the first is drawn, and a zero-length element is a dot.
///
/// **The format's own sign is not the dash/gap flag.** It does not read
/// consistently as one across the corpus (see pid-parse's
/// `docs/analysis/2026-08-07-jstyle-simple-dash-type-linetype.md` §3.1), so
/// only the magnitudes carry over and the alternation is the linetype
/// convention rather than the file's. That is the one interpretive step
/// between the decode and the screen.
fn build_dash_linetype(name: &str, dash: &DashPattern) -> LineType {
    let mut lt = LineType::new(name);
    lt.description = format!("P&ID dash pattern ({} segments)", dash.len());
    let mut pattern_length = 0.0;
    for (i, mm) in dash.segments_mm().iter().enumerate() {
        let len = mm.abs();
        pattern_length += len;
        let element = if len < 1e-9 {
            LineTypeElement::dot()
        } else if i % 2 == 0 {
            LineTypeElement::dash(len)
        } else {
            LineTypeElement::space(len)
        };
        lt.add_element(element);
    }
    lt.pattern_length = pattern_length;
    lt
}

fn build_entities(
    kind: &PidGraphicKind,
    library: Option<&mut SymbolLibrary>,
    projection: Projection,
) -> Vec<EntityType> {
    match kind {
        PidGraphicKind::Line { start, end } => {
            let mut line = Line::from_points(projection.point(start), projection.point(end));
            line.common.layer = LAYER_GEOMETRY.to_string();
            vec![EntityType::Line(line)]
        }
        PidGraphicKind::Polyline { points, closed } => {
            if points.len() < 2 {
                return Vec::new();
            }
            let vertices: Vec<Vector2> = points
                .iter()
                .map(|p| Vector2::new(projection.mm(p.x), projection.mm(p.y)))
                .collect();
            let mut polyline = LwPolyline::from_points(vertices);
            polyline.is_closed = *closed;
            polyline.common.layer = LAYER_GEOMETRY.to_string();
            vec![EntityType::LwPolyline(polyline)]
        }
        PidGraphicKind::Circle { center, radius } => {
            let mut circle = Circle::new();
            circle.center = projection.point(center);
            circle.radius = projection.mm(*radius);
            circle.common.layer = LAYER_GEOMETRY.to_string();
            vec![EntityType::Circle(circle)]
        }
        PidGraphicKind::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let mut arc = acadrust::entities::Arc::new();
            arc.center = projection.point(center);
            arc.radius = projection.mm(*radius);
            // Radians on both sides -- see the angle-unit note by the layer constants.
            arc.start_angle = *start_angle;
            arc.end_angle = *end_angle;
            arc.common.layer = LAYER_GEOMETRY.to_string();
            vec![EntityType::Arc(arc)]
        }
        PidGraphicKind::Text {
            insertion,
            value,
            height,
            rotation,
        } => {
            if value.trim().is_empty() {
                return Vec::new();
            }
            let mut text = Text::new();
            text.value = value.clone();
            text.insertion_point = projection.point(insertion);
            text.height = if *height > 0.0 {
                projection.mm(*height)
            } else {
                TEXT_HEIGHT_MM
            };
            // Radians on both sides -- see the angle-unit note by the layer constants.
            text.rotation = *rotation;
            text.common.layer = LAYER_TEXT.to_string();
            vec![EntityType::Text(text)]
        }
        PidGraphicKind::SymbolInstance {
            insertion,
            symbol_path,
            rotation,
            scale,
        } => {
            let placement = Placement {
                insertion,
                rotation: *rotation,
                scale: *scale,
                projection,
            };
            let body = library
                .zip(symbol_path.as_deref())
                .and_then(|(library, path)| library.resolve(path))
                .filter(|body| !body.primitives.is_empty())
                .map(|body| {
                    body.primitives
                        .iter()
                        .filter_map(|primitive| place_primitive(primitive, &placement))
                        .collect::<Vec<_>>()
                });

            // Only fall back to the marker when the body is genuinely
            // unavailable. A symbol that resolved to real geometry should not
            // also carry a dot -- that reads as a second object.
            let mut built = match body {
                Some(entities) if !entities.is_empty() => entities,
                _ => {
                    let mut marker = Circle::new();
                    marker.center = projection.point(insertion);
                    marker.radius = SYMBOL_MARKER_RADIUS_MM;
                    marker.common.layer = LAYER_SYMBOL.to_string();
                    vec![EntityType::Circle(marker)]
                }
            };

            if let Some(name) = symbol_path.as_deref().and_then(symbol_name) {
                let mut label = Text::new();
                label.value = name;
                label.height = SYMBOL_LABEL_HEIGHT_MM;
                // Beside the marker, not on it, and horizontal whatever the
                // placement angle is -- a rotated label is the harder read.
                label.insertion_point = Vector3::new(
                    projection.mm(insertion.x) + SYMBOL_MARKER_RADIUS_MM + SYMBOL_LABEL_GAP_MM,
                    projection.mm(insertion.y) - SYMBOL_LABEL_HEIGHT_MM / 2.0,
                    0.0,
                );
                label.common.layer = LAYER_SYMBOL_LABEL.to_string();
                built.push(EntityType::Text(label));
            }
            built
        }
        PidGraphicKind::Point { position } => {
            let mut point = Point::new();
            point.location = projection.point(position);
            point.common.layer = LAYER_POINT.to_string();
            vec![EntityType::Point(point)]
        }
        PidGraphicKind::Annotation { .. } | PidGraphicKind::Unknown { .. } => Vec::new(),
    }
}

/// Draw the inferred kinds that have a usable position.
///
/// The `Annotation` arm is unreachable as it stands, and kept for when it is
/// not. It drew a stub at the anchor a `JStyleOverride` record (PSM `0x0030`)
/// was read to carry in payload `+0..15`, on the strength of those sixteen
/// bytes holding two normalized f64 across every fixture. `style.dll`'s own
/// version-3 serialiser reads them as four independent u32 instead, so the
/// anchor was a coincidence rather than a position, and `pid-parse` emits the
/// family as `ProbeOnly` now -- measured in that crate's
/// `docs/analysis/2026-08-04-jstyleoverride-native-reader-settles-it.md`. The
/// kind stays in the parser's vocabulary, so if the real anchor is ever
/// located these stubs come back with it.
///
/// An endpoint pair is the drawing's connectivity graph -- which object joins
/// which -- rather than drafting geometry, and only part of it is expressed in
/// sheet coordinates: of `DWG-0201`'s 49 pairs, 35 have both ends on the
/// sheet, 9 mix a normalized end with a raw one, and 5 are raw at both ends.
/// The mixed and raw ones would draw a segment kilometres long, so only a pair
/// that passes [`on_sheet_pair`] is kept.
fn build_inferred(kind: &PidGraphicKind, projection: Projection) -> Vec<EntityType> {
    match kind {
        PidGraphicKind::Annotation {
            anchor,
            rotation_angle,
            ..
        } => {
            let (sin, cos) = rotation_angle.sin_cos();
            let start = projection.point(anchor);
            let mut tick = Line::from_points(
                start,
                Vector3::new(
                    start.x + ANNOTATION_TICK_MM * cos,
                    start.y + ANNOTATION_TICK_MM * sin,
                    0.0,
                ),
            );
            tick.common.layer = LAYER_ANNOTATION.to_string();
            vec![EntityType::Line(tick)]
        }
        PidGraphicKind::Line { start, end } if on_sheet_pair(start, end, projection) => {
            let mut link = Line::from_points(projection.point(start), projection.point(end));
            link.common.layer = LAYER_CONNECTIVITY.to_string();
            vec![EntityType::Line(link)]
        }
        _ => Vec::new(),
    }
}

/// Whether an inferred endpoint pair describes a link between two places on
/// the sheet.
///
/// Both ends have to be in sheet coordinates, and the pair has to go
/// somewhere: an unresolved end decodes as the origin, and a link whose ends
/// coincide -- 2 of `DWG-0201`'s 35 on-sheet pairs -- carries no direction to
/// draw.
fn on_sheet_pair(start: &PidPoint, end: &PidPoint, projection: Projection) -> bool {
    let (start_x, start_y) = (projection.mm(start.x), projection.mm(start.y));
    let (end_x, end_y) = (projection.mm(end.x), projection.mm(end.y));
    [start_x, start_y, end_x, end_y]
        .iter()
        .all(|value| projection.band.holds(*value))
        && (start_x.hypot(start_y) > CONNECTIVITY_MIN_MM)
        && (end_x.hypot(end_y) > CONNECTIVITY_MIN_MM)
        && ((end_x - start_x).hypot(end_y - start_y) > CONNECTIVITY_MIN_MM)
}

/// State the opening view, the way a DWG does.
///
/// `Scene::restore_saved_camera` reads the `*Active` VPORT and only falls back
/// to `fit_all` when there is none. Neither default is usable here: a fresh
/// `CadDocument` ships an `*Active` entry parked at the origin with a 10-unit
/// height, and `fit_all` fits every wire including the off-sheet strays that
/// [`SheetBand`] keeps out of the framing box. So the importer states the view
/// itself, over the filtered bounds.
///
/// A drawing that states its page opens on that whole sheet, the way it does
/// in `SmartPlant`: framing on the content alone leaves an A2 whose drafting
/// stops short of the border opening zoomed past its own title block. The
/// page is where the border is drawn, so the view and the border agree by
/// construction.
fn frame_drawing(doc: &mut CadDocument, bounds: &Bounds, page_mm: Option<(f64, f64)>) {
    if bounds.is_empty() {
        return;
    }

    doc.header.model_space_extents_min = Vector3::new(bounds.min_x, bounds.min_y, 0.0);
    doc.header.model_space_extents_max = Vector3::new(bounds.max_x, bounds.max_y, 0.0);

    let (min_x, min_y, max_x, max_y) = match page_mm {
        Some((page_width, page_height)) => (0.0, 0.0, page_width, page_height),
        None => (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y),
    };
    let width = max_x - min_x;
    let height = max_y - min_y;

    let mut vport = acadrust::tables::VPort::new("*Active");
    vport.lower_left = Vector2::new(0.0, 0.0);
    vport.upper_right = Vector2::new(1.0, 1.0);
    vport.view_direction = Vector3::new(0.0, 0.0, 1.0);
    vport.view_target = Vector3::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0, 0.0);
    vport.view_center = Vector2::ZERO;
    // `view_height` alone decides the zoom, so a landscape sheet in a window
    // narrower than 4:3 would spill sideways; widen it to cover that case.
    vport.view_height = (height.max(width * 0.75) * 1.05).max(1.0);
    vport.handle = doc.allocate_handle();
    // `add` refuses a duplicate name, and the default entry above is one.
    doc.vports.add_or_replace(vport);
}

struct Bounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    projection: Projection,
}

impl Bounds {
    fn new(projection: Projection) -> Self {
        Self {
            min_x: f64::MAX,
            min_y: f64::MAX,
            max_x: f64::MIN,
            max_y: f64::MIN,
            projection,
        }
    }

    fn add(&mut self, point: &PidPoint) {
        let (x, y) = (self.projection.mm(point.x), self.projection.mm(point.y));
        if !self.projection.band.holds(x) || !self.projection.band.holds(y) {
            return;
        }
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }
}

fn accumulate_bounds(kind: &PidGraphicKind, bounds: &mut Bounds) {
    match kind {
        PidGraphicKind::Line { start, end } => {
            bounds.add(start);
            bounds.add(end);
        }
        PidGraphicKind::Polyline { points, .. } => {
            for point in points {
                bounds.add(point);
            }
        }
        PidGraphicKind::Circle { center, .. } | PidGraphicKind::Arc { center, .. } => {
            bounds.add(center);
        }
        PidGraphicKind::Text { insertion, .. }
        | PidGraphicKind::SymbolInstance { insertion, .. } => bounds.add(insertion),
        PidGraphicKind::Point { position } => bounds.add(position),
        // Never reached: both kinds only ever arrive inferred or probe-only,
        // and the caller frames on decoded geometry alone.
        PidGraphicKind::Annotation { .. } | PidGraphicKind::Unknown { .. } => {}
    }
}

/// Where a symbol placement puts its library body on the sheet.
struct Placement<'a> {
    insertion: &'a PidPoint,
    rotation: f64,
    scale: [f64; 2],
    projection: Projection,
}

impl Placement<'_> {
    /// Map a point out of the symbol's own space onto the sheet.
    ///
    /// `pid-parse` splits the placement's 2x2 matrix into an angle and a
    /// scale, carrying a reflection as a negative y scale instead of folding
    /// it into the angle. Recomposing gives rows `sx * (cos, sin)` and
    /// `sy * (-sin, cos)`, which reproduces the original matrix for the
    /// rotate / scale / mirror placements a P&ID uses. Symbol bodies are in
    /// the same source unit as the drawing, so the conversion to mm happens
    /// once, at the end.
    fn apply(&self, x: f64, y: f64) -> Vector3 {
        let (sin, cos) = self.rotation.sin_cos();
        let [scale_x, scale_y] = self.scale;
        Vector3::new(
            self.projection
                .mm(self.insertion.x + x * scale_x * cos - y * scale_y * sin),
            self.projection
                .mm(self.insertion.y + x * scale_x * sin + y * scale_y * cos),
            0.0,
        )
    }

    /// Scale a radius. A placement with different x and y scales would turn a
    /// circle into an ellipse; P&ID placements mirror and turn but do not
    /// stretch one axis alone, so the mean is exact in practice and degrades
    /// gently if that ever stops being true.
    fn scale_radius(&self, radius: f64) -> f64 {
        self.projection
            .mm(radius * (self.scale[0].abs() + self.scale[1].abs()) / 2.0)
    }

    /// Whether the placement flips handedness, which reverses the direction
    /// an arc sweeps.
    fn mirrored(&self) -> bool {
        self.scale[1] < 0.0
    }
}

/// Draw one primitive of a symbol body at its placement.
fn place_primitive(primitive: &SymbolPrimitive, at: &Placement<'_>) -> Option<EntityType> {
    match primitive {
        SymbolPrimitive::Line { start, end } => {
            let mut line = Line::from_points(at.apply(start.0, start.1), at.apply(end.0, end.1));
            line.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::Line(line))
        }
        SymbolPrimitive::Circle { center, radius } => {
            let radius = at.scale_radius(*radius);
            if !radius.is_finite() || radius <= 0.0 {
                return None;
            }
            let mut circle = Circle::new();
            circle.center = at.apply(center.0, center.1);
            circle.radius = radius;
            circle.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::Circle(circle))
        }
        SymbolPrimitive::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let radius = at.scale_radius(*radius);
            if !radius.is_finite() || radius <= 0.0 {
                return None;
            }
            // An arc always runs counter-clockwise from start to end, so a
            // mirrored placement has to swap the ends as well as reflect the
            // angles -- otherwise the arc is drawn as its own complement.
            let (start_angle, end_angle) = if at.mirrored() {
                (at.rotation - end_angle, at.rotation - start_angle)
            } else {
                (at.rotation + start_angle, at.rotation + end_angle)
            };
            let mut arc = acadrust::entities::Arc::new();
            arc.center = at.apply(center.0, center.1);
            arc.radius = radius;
            // Radians on both sides -- see the angle-unit note by the layer
            // constants. The sum above already proves the unit: `at.rotation`
            // is what `Placement::apply` feeds to `sin_cos`, so the library's
            // angles have to be radians for the addition to mean anything.
            arc.start_angle = start_angle;
            arc.end_angle = end_angle;
            arc.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::Arc(arc))
        }
        SymbolPrimitive::Polyline { vertices } => {
            if vertices.len() < 2 {
                return None;
            }
            let points: Vec<Vector2> = vertices
                .iter()
                .map(|(x, y)| {
                    let placed = at.apply(*x, *y);
                    Vector2::new(placed.x, placed.y)
                })
                .collect();
            let mut polyline = LwPolyline::from_points(points);
            polyline.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::LwPolyline(polyline))
        }
        SymbolPrimitive::Text { text, at: origin } => {
            let value = text.trim();
            if !carries_a_label(value) {
                return None;
            }
            let mut label = Text::new();
            label.value = value.to_string();
            label.insertion_point = at.apply(origin.0, origin.1);
            // The record holds no height, so this is the same ISO 3098
            // fallback the sheet's own text gets, scaled with the placement
            // so a half-size symbol does not carry full-size lettering.
            label.height = at.scale_radius(TEXT_HEIGHT_MM / at.projection.mm_per_unit);
            // Radians on both sides -- see the angle-unit note by the layer constants.
            label.rotation = at.rotation;
            label.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::Text(label))
        }
    }
}

/// Whether a symbol's own text run says anything once its unfilled template
/// fields are discounted.
///
/// The library is a set of templates: a run reads `NULL` wherever the drawing
/// is expected to supply a value, and 328 of the 1043 runs in the reference
/// library are nothing but that. Those are not lettering anyone drew, and
/// putting them on the sheet would print `NULL` across every equipment table.
/// A run that still has a word in it after the placeholders are discounted --
/// `HH=NULL`, `设备位号` -- is real lettering and is drawn as it stands,
/// placeholder included, because guessing at the missing value would be worse
/// than showing that it is missing.
fn carries_a_label(text: &str) -> bool {
    text.replace(SYMBOL_TEXT_PLACEHOLDER, "")
        .chars()
        .any(char::is_alphanumeric)
}

/// Find the `SmartPlant` reference-data symbol libraries for a drawing.
///
/// A `.pid` names its symbols by UNC path into the project's reference share,
/// which is normally unreachable from wherever the file is being read. The
/// override says where local copies live; failing that, a SmartPlant project
/// keeps drawings and reference data under one root (`Plant\Drawings\...` next
/// to `Plant\Ref\Symbols\...`), so walking up from the drawing finds it.
///
/// Every root found is kept, searched in the order listed. One drawing can
/// cite several shares, and a machine usually holds a partial copy of each
/// rather than one merged tree, so stopping at the first root leaves whatever
/// it does not cover undrawn. `SymbolLibrary` only settles for a root that
/// has nothing to draw once the others have been tried, so a root that turns
/// out to be a two-file sample costs a miss rather than a wrong symbol.
fn discover_symbol_library(drawing: &Path) -> Option<SymbolLibrary> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(list) = std::env::var_os(SYMBOL_LIBRARY_ENV) {
        roots.extend(std::env::split_paths(&list).filter(|root| root.is_dir()));
    }
    let mut dir = drawing.parent();
    for _ in 0..SYMBOL_SEARCH_DEPTH {
        let Some(at) = dir else { break };
        // `Ref` is the level SmartPlant parks reference data at, and it is
        // the one intermediate name the walk would otherwise step straight
        // past on its way up.
        for holder in [at.to_path_buf(), at.join("Ref")] {
            for candidate in symbol_roots_in(&holder) {
                if !roots.contains(&candidate) {
                    roots.push(candidate);
                }
            }
        }
        dir = at.parent();
    }
    (!roots.is_empty()).then(|| SymbolLibrary::with_roots(roots))
}

/// The symbol roots directly inside `dir`.
///
/// A local copy of a reference share is rarely named exactly `Symbols`: it
/// arrives as `symbols-full`, `Symbols_2024`, `plant-symbols`. Matching on
/// the word rather than the whole name finds those, and costs one directory
/// listing per level -- the library itself still decides whether a root
/// actually holds the symbol being placed.
fn symbol_roots_in(dir: &Path) -> Vec<PathBuf> {
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
                .contains("symbol")
        })
        .map(|entry| entry.path())
        .collect();
    // `read_dir` order is filesystem order; searching in a stated order keeps
    // which copy of a symbol wins from depending on how the disk is laid out.
    found.sort();
    found
}

/// The symbol's name, which is the file name of the `.sym` it is placed from.
///
/// `pid-parse` resolves the placement to a library path off the drawing's
/// `JSite` layer, and those are UNC paths into a SmartPlant reference share
/// (`\\WIN-SPID\...\Piping\Valves\Angle\2-Way Angle Globe Valve.sym`), so the
/// leaf is the name the drafter picked the symbol by.
fn symbol_name(path: &str) -> Option<String> {
    let file = path.rsplit(['\\', '/']).next()?.trim();
    let stem = match file.rfind('.') {
        Some(dot) if file[dot..].eq_ignore_ascii_case(".sym") => &file[..dot],
        _ => file,
    };
    let stem = stem.trim();
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn ensure_layer(doc: &mut CadDocument, name: &str, colour: Color, visible: bool) {
    if doc.layers.contains(name) {
        return;
    }
    let mut layer = acadrust::tables::Layer::new(name);
    layer.handle = doc.allocate_handle();
    layer.color = colour;
    layer.flags.off = !visible;
    let _ = doc.layers.add(layer);
}

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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use acadrust::entities::hatch::{BoundaryEdge, BoundaryPath, BoundaryPathFlags, LineEdge};
use acadrust::entities::{Circle, Hatch, Line, LwPolyline, Point, Text, TextHorizontalAlignment};
use acadrust::tables::linetype::{LineType, LineTypeElement};
use acadrust::types::{Color, LineWeight, Vector2, Vector3};
use acadrust::{CadDocument, EntityType, TableEntry};
use pid_parse::style_link::{
    DashPattern, LineStyleIndex, MarkerStatus, PointMarker, ResolvedFill, ResolvedLineStyle,
    StyleNameIndex, TextAlignment,
};
use pid_parse::symbol_library::{PrimitiveStyle, SymbolLibrary, SymbolPrimitive};
use pid_parse::{
    build_normalized_geometry, NormalizedPidGeometry, ParseOptions, PidDrawingUnits,
    PidGeometryConfidence, PidGraphicKind, PidParser, PidPoint, PidSemanticHit, PidSemanticIndex,
    PidSourceLayer, PidSymbolDefinition,
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

// XDATA application name carrying an entity's P&ID identity: the importer's
// own reading (`role=…`, `style=…`, see `role_of_layer`), the authored sheet
// layer (`sheet_layer=…`, `sheet_layer_oid=…`), plus the published semantic
// pairs when `_Data.xml` sits beside the drawing. Every drawn entity carries
// at least a role; authored layer identity is present without `_Data.xml`.
pub(crate) use super::PID_SEMANTICS_XDATA_APP;
// The name criterion for "SmartPlant hides this sheet layer" is the fallback
// behind [`sheet_layer_is_hidden`]: it speaks only for a layer whose display
// bit `pid-parse` could not read.
use super::pid_view_filter::{is_hidden_sheet_layer, PidViewFilter, PidViewSummary};

mod build;
mod metadata;
mod page;
mod styles;
mod summary;
mod symbols;
mod text;

use self::build::*;
use self::metadata::*;
use self::page::*;
use self::styles::*;
use self::summary::*;
pub use self::summary::{ImportSummary, ImportUnit, SUMMARY_PROPERTY_PREFIX};
use self::symbols::*;
use self::text::*;

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
// A point's mark is a review status, not decoration: the drawing's style
// librarian names the four point symbols `psOk`, `psWarning`, `psError` and
// `psApproved`, and `psOk`'s glyph is deliberately blank. Splitting the drawn
// ones onto their own layers is what makes "show me everything SmartPlant
// flagged" one toggle instead of a hunt.
const LAYER_POINT_WARNING: &str = "PID-POINT-WARNING";
const LAYER_POINT_ERROR: &str = "PID-POINT-ERROR";
const LAYER_POINT_APPROVED: &str = "PID-POINT-APPROVED";
const LAYER_ANNOTATION: &str = "PID-ANNOTATION";
const LAYER_CONNECTIVITY: &str = "PID-CONNECTIVITY";
const LAYER_FILL: &str = "PID-FILL";
const LAYER_FRAME: &str = "PID-FRAME";
/// Where the import files the entities of the sheet layers it starts switched
/// off, with the layer itself off. `pid_view_filter` releases it when such a
/// sheet layer is switched back on.
pub(crate) const LAYER_HIDDEN: &str = "PID-HIDDEN";

// The review-status vocabulary, which is not a discipline. These eight names
// say what state an item is in, and the `PID-POINT-*` layers already carry
// that; letting them through `discipline_layer` would file line work under a
// vocabulary it does not belong to.
const REVIEW_STATUS_STYLE_NAMES: [&str; 8] = [
    "lsApproved",
    "lsError",
    "lsOk",
    "lsWarning",
    "psApproved",
    "psError",
    "psOk",
    "psWarning",
];
// Names that say how a line is drawn, not what it is. A discipline layer is
// for the classification width and colour cannot express; these three are that
// appearance, so filing them separately splits `PID-GEOMETRY` along a line the
// drawing already draws.
//
// `Normal` is the clearest of the three: the style librarian files it under
// five different families -- simple line, simple fill, hatch fill, text
// character, text paragraph -- so it is each family's default name rather than
// any one role. `As Drawn` and `Dashed` are line styles, but they name the
// stroke: `As Drawn` covers two different widths in this corpus, and `Dashed`
// is the dash pattern the linetype already carries.
const APPEARANCE_STYLE_NAMES: [&str; 3] = ["As Drawn", "Dashed", "Normal"];
// Line work keyed, while the document is built, by the drawing's own name
// for the style it draws with -- `PID-STYLE-PRIMARY-PIPING-NEW`,
// `PID-STYLE-NOZZLE-NEW`. A working key only: the entity's layer slot ends up
// holding its authored sheet layer (see `load_pid`), and the style name
// itself is written as `style=` XDATA. Until plan 2026-09-21 these were also
// output layers, under the taxonomy layer mode that plan retired along with
// its environment switch; a `PID-STYLE-*` key that reaches the slot with no
// authored layer to give way to falls back to `PID-GEOMETRY`.
// The prefix is what keeps the key from colliding with the other working
// layers: a project style library is free to call a style `Text` or `Point`.
const LAYER_DISCIPLINE_PREFIX: &str = "PID-STYLE-";

// What an imported entity's layer slot holds (plan 2026-09-07, D4 / D5 / L3;
// plan 2026-09-21).
//
// A DXF entity has one layer, and a `.pid` entity has two things that want
// it: the importer's own classification (line work, lettering, symbol,
// review point …) and the sheet layer the drawing itself filed it under
// (`Default`, `Labels`, `ConsistencyChecks` …). The slot holds the authored
// sheet layer, by name and verbatim -- no prefix, the same name across
// storages merged into one layer (the storage-local oid is still in XDATA),
// each layer on or off as the drawing's own view filter set draws it. The
// classification is written into the entity's `PID_SEMANTICS` XDATA (`role=`,
// `style=`), which is where a consumer that wants it reads it. Entities the
// importer makes itself (the page border, connectivity links, a symbol's own
// label, a style cluster's glyph strokes) were never on a sheet layer and
// keep their `PID-*` layer; a hidden sheet layer is simply off, and
// `PID-HIDDEN` is only for an entity hidden under a layer with no name to be
// off under.
//
// Until plan 2026-09-21 the slot could instead hold the classification
// itself (`PID-GEOMETRY` / `PID-TEXT` / `PID-SYMBOL` / `PID-POINT-*` /
// `PID-STYLE-*` / `PID-HIDDEN`): the taxonomy layer mode, the default of plan
// 2026-09-07 L3, selected by an environment switch. Measured on the corpus
// the two readings drew the same picture -- every entity carries its own
// colour and width -- and differed only in the layer table, so the taxonomy
// mode and the switch were retired; the working layers above stay as the
// classification's keys while the document is built.

// Which body a symbol placement draws when the drawing caches one and the
// library holds one too is not a choice any more (plan 2026-09-19, P-D1 /
// P-D3; the library-first way back, an environment switch kept for one
// round, was retired by plan 2026-09-20-retire-the-library-first-symbol-source).
//
// A `.pid` caches, for every symbol it places, the body SmartPlant actually
// put on the sheet -- the flavour its `JFlavorManager` picked, resized where
// the symbol is parametric -- and the reference share's `.sym` holds the
// template that body was made from, every `Sheet*` of the file merged. On
// the corpus all 109 placements have a cached body and the library has 97;
// of those 97 pairs only 26 draw stroke for stroke the same, and in none of
// the other 71 is the library the one that matches the sheet: it draws a
// second sheet or another revision, a different symbol under the same name
// (工艺's 35 `Remarks` are a 27mm cloud in the library and a 1.3mm mark on
// the sheet), or the template of a parametric body the drawing resized. So
// the drawing's own cached body draws, without the strokes on the symbol's
// switched-off internal layers (P-D2: the heat tracing, the jacket, the
// `NULL` placeholder lettering, a parametric body's construction lines --
// what SmartPlant does not put on the screen); the library only for a
// placement the drawing caches nothing drawable for; the marker dot when
// neither has a body. Measured in pid-parse's
// `docs/analysis/2026-09-07-placement-tail-names-the-cached-definition.md`
// and the 2026-09-19 plan's corpus table.

/// The importer's own layers, with the colour and initial visibility each
/// opens with. The layer table is the drawing's own, so one of these is
/// declared only when an entity that has no authored sheet layer -- one the
/// importer made itself, or one whose layer the file gave no name for --
/// lands on it (see [`ensure_taxonomy_layer`]). Until plan 2026-09-21 the
/// retired taxonomy layer mode declared every one up front.
fn taxonomy_layers() -> [(&'static str, Color, bool); 13] {
    // Colours separate the kinds at a glance: a P&ID is mostly line work, and
    // an all-white import makes lettering, symbol bodies and the decode's own
    // loose ends indistinguishable from the piping.
    [
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
        // The review statuses. Each mark keeps the colour its own style
        // states, so these layer colours only ever show on a mark that states
        // none — but they are the status colours the drawings themselves use,
        // so a defaulted mark still reads correctly. `PID-POINT-ERROR` is
        // empty across this whole corpus, which is the point of declaring it:
        // every drawing that defines the state defines it and nothing is in
        // it, and a present empty layer says that where a missing one would
        // not. There is no `PID-POINT-OK` because a passing item's glyph is
        // blank by construction and there is nothing to put on it.
        (LAYER_POINT_WARNING, Color::BLUE, true),
        (LAYER_POINT_ERROR, Color::RED, true),
        (LAYER_POINT_APPROVED, Color::GREEN, true),
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
        // Content authored on a hidden/invisible sheet layer remains present
        // and inspectable, but follows the source visibility on first open.
        (LAYER_HIDDEN, Color::GRAY, false),
    ]
}

/// Declare `layer` with its colour and visibility if it is one of the
/// importer's own ([`taxonomy_layers`]) and not yet in the table. Anything
/// else is left alone: an authored sheet layer is declared where its state is
/// known, and a `PID-STYLE-*` working key never reaches the slot (see
/// `load_pid`).
fn ensure_taxonomy_layer(doc: &mut CadDocument, layer: &str) {
    if doc.layers.contains(layer) {
        return;
    }
    if let Some((_, colour, visible)) = taxonomy_layers().iter().find(|(name, _, _)| *name == layer)
    {
        ensure_layer(doc, layer, *colour, *visible);
    }
}

/// The importer's reading of what an entity is, as the `role=` XDATA value:
/// `geometry`, `text`, `symbol`, `symbol-label`, `point-ok` / `-warning` /
/// `-error` / `-approved`, `annotation`, `connectivity`, `fill`, `frame`.
///
/// This is the classification the `PID-*` layer names have carried in the
/// layer slot until now, moved to where a change of layer policy cannot lose
/// it: the plan (`docs/plans/2026-09-07-jdim-driving-dimensions-and-layer-panel.md`,
/// D8 / L2) wants the slot free to hold the authored sheet layer instead, and
/// the role has to survive that. It is derived from the synthetic layer an
/// entity was built on, so it must be read *before* the hidden-layer override
/// moves the entity to `PID-HIDDEN` -- which is the one synthetic layer with
/// no role of its own, because it says where the drawing hid something rather
/// than what it is.
///
/// Not `class=`: that key already holds the published object's XML element
/// name in the same record (see [`attach_pid_metadata`]), and legend
/// recognition writes it too.
fn role_of_layer(layer: &str) -> Option<&'static str> {
    Some(match layer {
        LAYER_GEOMETRY => "geometry",
        LAYER_TEXT => "text",
        LAYER_SYMBOL => "symbol",
        LAYER_SYMBOL_LABEL => "symbol-label",
        LAYER_POINT => "point-ok",
        LAYER_POINT_WARNING => "point-warning",
        LAYER_POINT_ERROR => "point-error",
        LAYER_POINT_APPROVED => "point-approved",
        LAYER_ANNOTATION => "annotation",
        LAYER_CONNECTIVITY => "connectivity",
        LAYER_FILL => "fill",
        LAYER_FRAME => "frame",
        discipline if discipline.starts_with(LAYER_DISCIPLINE_PREFIX) => "geometry",
        _ => return None,
    })
}

/// Whether the drawing draws nothing of the sheet layer an entity is filed
/// under. The answer is the file's own: the display bit of the sheet's
/// `Top ViewFilterSet`, which `pid-parse` reads onto every entity's source
/// layer (plan 2026-09-07, L1). Only a layer whose bit the parser could not
/// read falls back to the three names SmartPlant conventionally hides
/// under -- the criterion this importer used on its own until the bit was
/// decoded, and which the corpus's sheets agree with except for `Invisible`,
/// a definition-cache layer the file displays.
///
/// This one function decides both which entities move to `PID-HIDDEN` and
/// which sheet layers the view filter starts with switched off.
fn sheet_layer_is_hidden(layer: &PidSourceLayer) -> bool {
    match layer.displayed {
        Some(displayed) => !displayed,
        None => layer.name.as_deref().is_some_and(is_hidden_sheet_layer),
    }
}

/// What [`load_pid`] hands back: the drawing, and the summary of how it was
/// read.
#[derive(Debug)]
pub struct PidImport {
    /// The imported drawing.
    pub document: CadDocument,
    /// The headline of the import: what was drawn, what was not, where the
    /// bodies and the unit came from.
    pub summary: ImportSummary,
}

/// Parse a `.pid` file and project its decoded Sheet geometry into a document,
/// filing each entity under the sheet layer the drawing itself filed it under
/// (plan 2026-09-07 L3; the only reading since plan 2026-09-21, which retired
/// the layer-mode environment switch and the taxonomy layers it could select).
/// Hands back the document and the import's [`ImportSummary`] together.
///
/// Four steps, each a function below (plan 2026-09-21-load-pid-returns-its-
/// summary, Q-D8): [`prepare_document`] readies the document and its layer
/// table, [`resolve_styles`] reads the style indexes and registers what they
/// need in the document, [`build_document_entities`] files every drawn
/// entity, and [`finish`] reports, frames and summarises. The order of what
/// each does to the document -- every layer, linetype, entity and record it
/// adds -- is the order handles are issued in, and so the order a saved
/// drawing comes out in.
pub fn load_pid(path: &Path) -> Result<PidImport, String> {
    // The Geometry profile: every pass whose output reaches this document
    // -- the record families, the cached symbol bodies and their stroke
    // styles, the sheet layers and their display state, the style tables,
    // the endpoint links behind `PID-CONNECTIVITY` -- and none of the
    // probes and derived views only `pid_inspect` reads (pid-parse plan
    // 2026-09-21-a-geometry-parse-profile). On the corpus the drawn
    // entities are identical to a Full parse's; `pid_probe` and
    // `pid_plot_dump` stay on Full because they look at the probes.
    let parsed = PidParser::with_options(ParseOptions::geometry())
        .parse_file(path)
        .map_err(|error| error.to_string())?;
    let geometry = build_normalized_geometry(&parsed);

    let mut doc = prepare_document(&parsed);
    let mut library = discover_symbol_library(path);
    let styles = resolve_styles(&parsed, &geometry, path, &mut doc);
    let unit = ImportUnit::read(&geometry, path);
    let built = build_document_entities(&geometry, &styles, &mut library, &unit, &mut doc);
    if built.decoded == 0 {
        return Err(format!(
            "No decoded geometry in {}: pid-parse produced {} evidence item(s), none of them source-backed",
            path.display(),
            geometry.entities.len()
        ));
    }
    let summary = finish(
        path,
        &geometry,
        library.as_ref(),
        &styles,
        unit,
        built,
        &mut doc,
    );
    doc.source_path = Some(path.to_string_lossy().into_owned());
    Ok(PidImport {
        document: doc,
        summary,
    })
}

/// The document an import fills: a fresh one with the standard linetypes,
/// lineweights shown, and the drawing's own layer table declared -- every
/// sheet layer of the document storage, on or off as the file draws it.
fn prepare_document(parsed: &pid_parse::PidDocument) -> CadDocument {
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
    // The layer table is the drawing's own: every sheet layer of the document
    // storage, on or off as its view filter set draws it, whether or not
    // anything drawn lands on it -- a present-and-off `HeatTrace` says what
    // SmartPlant's layer list says. A name two layer objects share is one
    // layer here, off only when the file draws neither. Nested-storage names
    // and the `PID-*` layers the importer's own entities keep are declared as
    // they are met; the page border is always drawn, so its layer is declared
    // now.
    let mut hidden_by_name: BTreeMap<&str, bool> = BTreeMap::new();
    for layer in parsed.sheet_layers.get("/").into_iter().flatten() {
        let hidden = match layer.displayed {
            Some(displayed) => !displayed,
            None => is_hidden_sheet_layer(&layer.name),
        };
        hidden_by_name
            .entry(layer.name.as_str())
            .and_modify(|all_hidden| *all_hidden &= hidden)
            .or_insert(hidden);
    }
    for (name, hidden) in hidden_by_name {
        ensure_layer(&mut doc, name, Color::WHITE, !hidden);
    }
    ensure_taxonomy_layer(&mut doc, LAYER_FRAME);
    doc
}

/// Close the import: log the report, draw the page border, frame the camera,
/// store and apply the drawing's own view filter, and state the headline.
fn finish(
    path: &Path,
    geometry: &NormalizedPidGeometry,
    library: Option<&SymbolLibrary>,
    styles: &Styles,
    unit: ImportUnit,
    built: Built,
    doc: &mut CadDocument,
) -> ImportSummary {
    let Built {
        drawn,
        decoded,
        lettering_on_fallback,
        symbol_bodies,
        parametric_placements,
        sheet_layer_distribution,
        sheet_layers_off,
        bounds,
    } = built;
    report_import(
        path,
        geometry,
        library,
        drawn,
        lettering_on_fallback,
        symbol_bodies,
        &sheet_layer_distribution,
    );
    let page_mm = geometry.page_dimensions_mm;
    draw_page_border(doc, page_mm);
    frame_drawing(doc, &bounds, page_mm);
    // The drawing's own view filter: the sheet layers the file switches off
    // start switched off here too, and the entities on them go dark by their
    // own `invisible` bit as well as by sitting on `PID-HIDDEN`. Stored in the
    // document, so what the user later switches on or off rides every save
    // with the bits.
    let filter = PidViewFilter::with_layers_off(sheet_layers_off.iter().map(String::as_str));
    filter.store(doc);
    filter.apply(doc);
    // The headline the open-completion handler shows on the command line;
    // the counts agree with `report_import`'s log lines by construction, and
    // the sheet-layer line with what the Layer Manager lists.
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
    let view = PidViewSummary::of(doc);
    ImportSummary {
        drawn,
        decoded,
        missing,
        style_tables_failed: styles.style_tables_failed,
        sheet_layers: sheet_layer_distribution.len(),
        layered_entities: sheet_layer_distribution.values().sum(),
        unresolved_sheet_layers: sheet_layer_distribution
            .keys()
            .filter(|(_, _, name)| name.is_none())
            .count(),
        sheet_layer_names: view.layers.len(),
        sheet_layers_off: view.layers.iter().filter(|row| !row.on).count(),
        // The templates and their named dimensions are counted over the
        // cache rather than over what was drawn: no placement draws a
        // template, so the entity loop never meets them.
        driving_dimensions: geometry
            .symbol_definitions
            .iter()
            .flat_map(|body| body.dimensions.iter())
            .filter(|dimension| dimension.name.is_some())
            .count(),
        template_bodies: geometry
            .symbol_definitions
            .iter()
            .filter(|body| !body.dimensions.is_empty())
            .count(),
        parametric_placements,
        cache_bodies: symbol_bodies.cache,
        library_bodies: symbol_bodies.library,
        hidden_strokes_skipped: symbol_bodies.hidden_strokes_skipped,
        lettering_flattened: styles.lettering_flattened,
        symbol_library: library
            .map(|library| library.roots().to_vec())
            .unwrap_or_default(),
        unit,
    }
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

#[cfg(test)]
mod tests;

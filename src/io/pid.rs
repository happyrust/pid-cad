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
    /// Distinct authored layers referenced by drawn entities, keyed by
    /// storage-local oid rather than merged by display name.
    pub sheet_layers: usize,
    /// Drawn CAD entities carrying an authored sheet-layer reference.
    pub layered_entities: usize,
    /// Distinct referenced layer ids whose authored name did not resolve.
    pub unresolved_sheet_layers: usize,
    /// Distinct authored sheet layer *names* drawn entities state -- the rows
    /// of the Layer Manager's sheet-layer view. Fewer than `sheet_layers`
    /// when the same name is several layer objects across storages.
    pub sheet_layer_names: usize,
    /// How many of those names the import starts switched off in the
    /// drawing's view filter.
    pub sheet_layers_off: usize,
    /// Named driving dimensions over every symbol body the drawing caches --
    /// all of them on library templates no placement draws (pid-parse
    /// `docs/analysis/2026-09-18-the-parametric-chain-closes-on-the-template-
    /// not-the-instance.md`). A dimension without a name is not counted: the
    /// panel cannot show it.
    pub driving_dimensions: usize,
    /// Cached bodies carrying at least one driving dimension: the templates.
    pub template_bodies: usize,
    /// Placements whose cached body names a template with named dimensions,
    /// so whose entities carry `driving=` -- what the panel can show library
    /// defaults for. Zero on a drawing without parametric symbols, in which
    /// case the headline says nothing about dimensions at all.
    pub parametric_placements: usize,
    /// Symbol placements drawn from the body the drawing caches for them --
    /// every placement the drawing has a drawable body for. Logged, not
    /// shown: the shape being right is what the user sees.
    pub cache_bodies: usize,
    /// Symbol placements drawn from the library's `.sym`: those the drawing
    /// caches nothing drawable for.
    pub library_bodies: usize,
    /// Strokes of cached bodies left undrawn because the file switches the
    /// symbol-internal layer they sit on off (P-D2), summed over the
    /// placements whose cache was consulted.
    pub hidden_strokes_skipped: usize,
}

/// How the symbol placements of one import were drawn, tallied as
/// [`build_entities`] draws them; the last three numbers of
/// [`ImportSummary`] and one line of [`report_import`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SymbolBodies {
    /// Placements drawn from the drawing's own definition cache.
    cache: usize,
    /// Placements drawn from the library.
    library: usize,
    /// Placements neither body reached, drawn as the marker dot.
    markers: usize,
    /// Cached strokes left out for sitting on a switched-off symbol layer.
    hidden_strokes_skipped: usize,
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

/// Parse a `.pid` file and project its decoded Sheet geometry into a document,
/// filing each entity under the sheet layer the drawing itself filed it under
/// (plan 2026-09-07 L3; the only reading since plan 2026-09-21, which retired
/// the layer-mode environment switch and the taxonomy layers it could select).
pub fn load_pid(path: &Path) -> Result<CadDocument, String> {
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
    //
    // The table was read with the document: `parsed.style_tables` holds each
    // storage's `StyleCluster`, and the five indexes below join the sheets'
    // decoded records to it without opening the file again (pid-parse plan
    // 2026-09-21-style-link-reads-the-parsed-document). So "failed" here
    // means what the flag always meant to say -- the document's own table is
    // missing or did not walk -- rather than "the file would not open a
    // second time", and the indexes themselves cannot fail.
    let style_tables_failed = parsed
        .style_tables
        .get("/")
        .is_none_or(pid_parse::style_link::DocumentStyleTable::is_empty);
    if style_tables_failed {
        log::warn!(
            "{}: the style table did not read; line work keeps the layer defaults, lettering keeps the {TEXT_HEIGHT_MM}mm fallback, and boundary rings import as outlines",
            path.display()
        );
    }
    let styles = pid_parse::style_link::line_styles_for_document(&parsed);
    // What the drawing calls each of those styles. The same table carries it,
    // one field further: every `StyleCluster` opens with a style librarian
    // holding the authored name of every style the project library gave the
    // document -- `Primary Piping - New`, `Nozzle - New`, `Off-Line
    // Instrument`. It is a classification width and colour cannot express (a
    // nozzle and its equipment are drawn identically), so it becomes a layer
    // rather than being dropped. See `discipline_layer`.
    //
    // Only names on a line style reach line work, which is narrower than it
    // looks: the librarian also names fills, text and dash patterns, and it
    // states the family of each. `Electric Signal` reads like a discipline and
    // is a dash pattern, so nothing is ever filed under it.
    //
    // A drawing whose librarian names nothing is not a failed import: the line
    // work stays keyed to `PID-GEOMETRY` exactly as it did before this landed,
    // and the names' absence is itself the reading.
    let style_names = pid_parse::style_link::style_names_for_document(&parsed);
    // Which project standards file those names came from. It bounds them: a
    // name means the same thing across two drawings only as far as they were
    // drawn against the same library, and on the reference corpus the two
    // drawings sharing a `.SPP` are exactly the two whose vocabularies agree.
    // Logged rather than drawn -- it is provenance for the layer names above.
    let libraries = pid_parse::style_link::style_libraries_for_document(&parsed);
    let sources: std::collections::BTreeSet<&str> =
        libraries.values().map(String::as_str).collect();
    for source in sources {
        log::info!("{}: styles were read from {source}", path.display());
    }
    // Character height comes from the same table, one hop further along: a
    // text record names a paragraph style, and the height is on the character
    // style that paragraph style names. Most of a P&ID's lettering turns out
    // to be 1/8 inch, so `TEXT_HEIGHT_MM` was reading a quarter too small.
    // Records whose height does not resolve keep that fallback.
    let text_heights = pid_parse::style_link::text_heights_for_document(&parsed);
    // Which areas the drawing fills. `pid-parse` resolves an `igBoundary2d`
    // ring through its `JStyleOverride` to a `JStyleSimpleFill`; the fill's
    // own colour is not decoded, so a filled ring is drawn in its layer's
    // colour. On the reference corpus these are the solid flow arrowheads on
    // the pipelines -- 5 on DWG-0202 and 10 on the gongyi drawing, all of
    // which used to import as hollow triangles.
    let fills = pid_parse::style_link::fill_styles_for_document(&parsed);
    // A line's dash pattern comes from the same style table, one reference
    // further along: a JStyleSimpleLine names a JStyleSimpleDashType, and
    // style_link hands the decoded segments back. Pool the distinct patterns
    // into named document linetypes now, so `apply_symbology` can name each
    // dashed line to one and the renderer dashes it like any other linetype.
    // See pid-parse's `docs/analysis/2026-08-07-jstyle-simple-dash-type-linetype.md`.
    // The bodies the drawing caches state a dash of their own per stroke
    // (plan 2026-09-20, P-E8), pooled into the same table so a symbol's
    // internal dash draws through the same path.
    let dash_linetypes = register_dash_linetypes(&mut doc, &styles, &geometry.symbol_definitions);
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
    // session and silently vanish on save (see `set_entity_xdata`). Every
    // drawn entity states its `role=`, so the registration is unconditional.
    if !doc.app_ids.contains(PID_SEMANTICS_XDATA_APP) {
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
    let mut symbol_bodies = SymbolBodies::default();
    // Placements whose entities carry `driving=`; see `ImportSummary`.
    let mut parametric_placements = 0usize;
    let mut sheet_layer_distribution: BTreeMap<(String, u32, Option<String>), usize> =
        BTreeMap::new();
    // The sheet layers the drawing draws nothing of, by name: what the view
    // filter starts with switched off. Filled as the entities are filed.
    let mut sheet_layers_off: BTreeSet<String> = BTreeSet::new();
    for entity in &geometry.entities {
        // A boundary ring is the one kind whose style decides its shape rather
        // than its colour: filled, it is an area; unfilled, it is an outline
        // the member lines already drew. See `build_fill`.
        let fill = fill_for(&fills, entity);
        // Resolved before building: a point's style decides whether it draws
        // the slash mark SmartPlant shows for a class-coloured point.
        let symbology = style_for(&styles, entity);
        // What the drawing calls the style this record draws with, and the
        // working key its line work is built under. A record whose style the
        // drawing does not name keeps `PID-GEOMETRY`, and that absence is
        // itself a reading: the librarian lists what came from the project
        // library, so an unnamed style is one drawn in this file. The name
        // goes into XDATA as `style=` whether or not it becomes a key -- an
        // appearance name such as `Normal` is still what the drawing said.
        let style_name = style_name_for(&style_names, entity, symbology);
        let line_work = style_name.and_then(discipline_layer);
        let line_work = line_work.as_deref().unwrap_or(LAYER_GEOMETRY);
        // The body the drawing itself caches for a placement: what it draws
        // by default, and what the library stands in for when the file
        // carries none. See `build_entities`.
        let cached_body = match &entity.kind {
            PidGraphicKind::SymbolInstance {
                definition: Some(definition),
                ..
            } => geometry.symbol_definition(*definition),
            _ => None,
        };
        let built = match entity.confidence {
            PidGeometryConfidence::Decoded => match fill {
                Some(fill) => build_fill(&entity.kind, fill, projection),
                None => build_entities(
                    &entity.kind,
                    BodySources {
                        cached: cached_body,
                        library: library.as_mut(),
                    },
                    projection,
                    symbology,
                    line_work,
                    &mut symbol_bodies,
                    &dash_linetypes,
                ),
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
            accumulate_bounds(&entity.kind, &built, &mut bounds);
        }
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
        // A label that carries a line break is several lines of lettering, and
        // a TEXT entity is one. Split it here, before anything is styled, so
        // each line arrives as an entity of its own and then picks up the
        // height, colour, alignment and typeface below exactly as a one-line
        // label does.
        let built = stack_text_lines(
            built,
            height_mm.unwrap_or(TEXT_HEIGHT_MM),
            text_style.and_then(|style| style.line_spacing),
        );
        drawn += built.len();
        if let Some(layer) = &entity.source_layer {
            *sheet_layer_distribution
                .entry((layer.storage_path.clone(), layer.oid, layer.name.clone()))
                .or_default() += built.len();
        }
        let semantic_hit = semantics.as_ref().and_then(|index| {
            entity
                .graphic_oid
                .and_then(|graphic_oid| index.resolve(graphic_oid))
        });
        // A placement's size on this sheet and its template's library
        // defaults, the same on every entity it drew. See
        // `PlacementMeasures`.
        let measures = PlacementMeasures::of(&entity.kind, &geometry, projection, &built);
        if measures.as_ref().is_some_and(|m| m.driving.is_some()) {
            parametric_placements += 1;
        }
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
            // The role is read off the working layer the entity was built
            // on, before the slot is filled below: an authored sheet layer
            // says where the drawing filed the entity, and `PID-HIDDEN` where
            // it hid one -- neither says what it is.
            let role = role_of_layer(&one.common().layer);
            attach_pid_metadata(
                &mut one,
                semantic_hit.as_ref(),
                entity.source_layer.as_ref(),
                role,
                style_name,
                measures.as_ref(),
            );
            let source_layer = entity.source_layer.as_ref();
            let hidden = source_layer.is_some_and(sheet_layer_is_hidden);
            let authored_name = source_layer.and_then(|layer| layer.name.as_deref());
            match authored_name {
                // The slot holds the authored layer, verbatim. A layer of a
                // nested storage, or one the document storage did not list,
                // is declared here with the state the file gives it; one
                // already declared keeps the state it opened with.
                //
                // A symbol's own label is the exception: it is lettering the
                // importer adds beside a placement, not content the drawing
                // has on that sheet layer, and it ships switched off for a
                // reason (`LAYER_SYMBOL_LABEL`). It keeps its layer, and its
                // `sheet_layer=` still says which placement it belongs to.
                Some(name) if role != Some("symbol-label") => {
                    one.common_mut().layer = name.to_string();
                    ensure_layer(&mut doc, name, Color::WHITE, !hidden);
                }
                // The label of a placement on a hidden sheet layer is not
                // moved either: its layer is already off, and the view filter
                // darkens it by its `sheet_layer=` like the body.
                Some(_) => {
                    ensure_taxonomy_layer(&mut doc, &one.common().layer.clone());
                }
                // No authored layer, or one whose name did not resolve: the
                // entity keeps the importer's own layer it was built on,
                // declared on demand -- except that a `PID-STYLE-*` working
                // key is not an output layer any more (plan 2026-09-21) and
                // gives way to `PID-GEOMETRY`; the style it spelt is in
                // `style=`. The one thing left to say here is "hidden" for a
                // layer with no name to be off under, and `PID-HIDDEN` says
                // it.
                None => {
                    if hidden {
                        one.common_mut().layer = LAYER_HIDDEN.to_string();
                    } else if one.common().layer.starts_with(LAYER_DISCIPLINE_PREFIX) {
                        one.common_mut().layer = LAYER_GEOMETRY.to_string();
                    }
                    ensure_taxonomy_layer(&mut doc, &one.common().layer.clone());
                }
            }
            if hidden {
                if let Some(name) = authored_name {
                    sheet_layers_off.insert(name.to_string());
                }
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
        symbol_bodies,
        &sheet_layer_distribution,
    );
    draw_page_border(&mut doc, page_mm);
    frame_drawing(&mut doc, &bounds, page_mm);
    // The drawing's own view filter: the sheet layers the file switches off
    // start switched off here too, and the entities on them go dark by their
    // own `invisible` bit as well as by sitting on `PID-HIDDEN`. Stored in the
    // document, so what the user later switches on or off rides every save
    // with the bits.
    let filter = PidViewFilter::with_layers_off(sheet_layers_off.iter().map(String::as_str));
    filter.store(&mut doc);
    filter.apply(&mut doc);
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
    let view = PidViewSummary::of(&doc);
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
                sheet_layers: sheet_layer_distribution.len(),
                layered_entities: sheet_layer_distribution.values().sum(),
                unresolved_sheet_layers: sheet_layer_distribution
                    .keys()
                    .filter(|(_, _, name)| name.is_none())
                    .count(),
                sheet_layer_names: view.layers.len(),
                sheet_layers_off: view.layers.iter().filter(|row| !row.on).count(),
                // The templates and their named dimensions are counted over
                // the cache rather than over what was drawn: no placement
                // draws a template, so the entity loop never meets them.
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
            },
        );
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
    let mut border = EntityType::LwPolyline(border);
    // Drawn by the importer, so no authored layer and no published identity;
    // it still says what it is, like every other entity of the import.
    attach_pid_metadata(
        &mut border,
        None,
        None,
        role_of_layer(LAYER_FRAME),
        None,
        None,
    );
    let _ = doc.add_entity(border);
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
    symbol_bodies: SymbolBodies,
    sheet_layer_distribution: &BTreeMap<(String, u32, Option<String>), usize>,
) {
    // Where the symbol bodies came from, and how many cached strokes the
    // file itself switches off were left out (plan 2026-09-19, P-D2).
    // Information rather than a warning: by default the cached body is the
    // one SmartPlant placed, and the strokes left out are ones its screen
    // does not show either.
    if symbol_bodies.cache + symbol_bodies.library > 0 {
        log::info!(
            "{}: {} symbol placement(s) drew the body the drawing caches for them and {} the library's; {} cached stroke(s) on switched-off symbol layers were left undrawn",
            path.display(),
            symbol_bodies.cache,
            symbol_bodies.library,
            symbol_bodies.hidden_strokes_skipped
        );
    }
    for warning in &geometry.warnings {
        log::debug!("{}: {warning}", path.display());
    }

    for ((storage, oid, name), count) in sheet_layer_distribution {
        match name {
            Some(name) => log::info!(
                "{}: authored sheet layer storage={} oid={} name={:?} drawn_entities={}",
                path.display(), storage, oid, name, count
            ),
            None => log::warn!(
                "{}: authored sheet layer storage={} oid={} has no decoded name; {} entity/entities keep their synthetic PID layer",
                path.display(), storage, oid, count
            ),
        }
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

    // A marker dot is a placement neither body reached: the drawing caches
    // nothing drawable for it and the library has no `.sym` for it (or was
    // not found). On the corpus every placement has a cached body, so this
    // is the line that says why a dot appeared when one does.
    if symbol_bodies.markers > 0 {
        log::warn!(
            "{}: {} symbol placement(s) drew as a marker dot, the drawing caching no drawable body for them and the library having none{}",
            path.display(),
            symbol_bodies.markers,
            if library.is_none() {
                format!(
                    ". Set {SYMBOL_LIBRARY_ENV} to a local copy of the project's reference-data Symbols share"
                )
            } else {
                String::new()
            }
        );
    }
    // The library is the stand-in for a placement the drawing caches no body
    // for, so its absence is information: the bodies on screen are the
    // drawing's own.
    let Some(library) = library else {
        log::info!(
            "{}: no symbol library found; placements draw the body the drawing caches for them, and a marker where it caches none",
            path.display()
        );
        return;
    };
    let missing = library.missing();
    if missing.is_empty() {
        return;
    }
    log::info!(
        "{}: {} of {} symbol(s) looked up are not in the library at {:?}; the drawing's own cached body stands in where it has one. First missing: {}",
        path.display(),
        missing.len(),
        library.lookups(),
        library.roots(),
        missing
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Write an entity's published P&ID identity into its XDATA, under
/// [`PID_SEMANTICS_XDATA_APP`] as self-describing `key=value` strings.
///
/// Four sources, four vocabularies, one record. The authored layer pair
/// (`sheet_layer`, `sheet_layer_oid`) is what the drawing files the entity
/// under, carried whenever the record declares one. `role` and `style` are
/// the importer's reading: `role` is what the entity is (see
/// [`role_of_layer`]), `style` the name the drawing gives the line style it
/// draws with -- the name `discipline_layer` derives a `PID-STYLE-*` layer
/// from, stated even when that name is an appearance and earns no layer.
/// `extent` and `driving` are a symbol placement's measures, the same on
/// every entity the placement drew (see [`PlacementMeasures`]). The
/// published pairs come from the drawing's own `_Data.xml`, joined by
/// `pid-parse`'s semantic index: `class` is the owning object's XML element
/// name (`PIDPipeline`, `PIDProcessVessel`, …), `label` its `ItemTag` /
/// `Name`, `oid` the published `GraphicOID`, and `resolved` says which hop
/// found it (`direct`, or `dependency:<aggregate oid>`).
///
/// `class` and `role` are deliberately two keys: the first is the file's
/// business object, the second this importer's classification, and legend
/// recognition (`pid_legend::xdata`) writes `class` too. Records it owns are
/// marked `resolved=legend:*`; nothing written here carries that prefix, so
/// `PIDLEGEND PURGE` leaves import identity alone.
fn attach_pid_metadata(
    entity: &mut EntityType,
    hit: Option<&PidSemanticHit<'_>>,
    source_layer: Option<&pid_parse::PidSourceLayer>,
    role: Option<&str>,
    style: Option<&str>,
    measures: Option<&PlacementMeasures>,
) {
    use acadrust::xdata::{ExtendedDataRecord, XDataValue};

    fn push_pair(record: &mut ExtendedDataRecord, key: &str, value: &str) {
        if !value.is_empty() {
            record.add_value(XDataValue::String(format!("{key}={value}")));
        }
    }

    let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
    if let Some(layer) = source_layer {
        if let Some(name) = layer.name.as_deref() {
            push_pair(&mut record, "sheet_layer", name);
        }
        push_pair(&mut record, "sheet_layer_oid", &layer.oid.to_string());
    }
    if let Some(role) = role {
        push_pair(&mut record, "role", role);
    }
    if let Some(style) = style {
        push_pair(&mut record, "style", style);
    }
    if let Some(measures) = measures {
        if let Some(extent) = measures.extent.as_deref() {
            push_pair(&mut record, "extent", extent);
        }
        if let Some(driving) = measures.driving.as_deref() {
            push_pair(&mut record, "driving", driving);
        }
        if let Some(instance) = measures.instance.as_deref() {
            push_pair(&mut record, "instance", instance);
        }
    }
    if let Some(hit) = hit {
        let object = hit.object();
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
    }
    if !record.values.is_empty() {
        entity.common_mut().extended_data.add_record(record);
    }
}

/// What a symbol placement says about its size, written into the
/// `PID_SEMANTICS` record of every entity the placement drew -- body strokes
/// and the name lettered beside them alike -- so the properties panel can
/// answer for any of them (plan `2026-09-18-driving-dimensions-reach-the-
/// panel-as-library-defaults`, K2).
///
/// The two are different facts and the panel shows them as two rows. The
/// extent is what this drawing actually draws the placement at; the driving
/// dimensions are what the symbol library's template was authored with. They
/// agree only for an instance nobody resized: `DWG-0201`'s Parametric
/// Manifold is placed 172.21 x 71.18 mm while its template's `Left` / `Right`
/// / `Top` say 228.6 x 40.64, and a placed instance carries no dimension
/// values of its own (pid-parse `docs/analysis/2026-09-18-the-parametric-
/// chain-closes-on-the-template-not-the-instance.md`). So the second row is
/// captioned as the library default, and never as the instance's size.
struct PlacementMeasures {
    /// `<W>x<H>`, millimetres to two places: the rectangle the placement's
    /// body covers on the sheet, rotation, scale and mirror applied. Measured
    /// on the body the drawing itself caches for the placement -- the
    /// instance as SmartPlant last drew it -- over the strokes on the layers
    /// the file displays, so it is the size of the very strokes on screen
    /// (plan 2026-09-19, P-D7: ` Line2` reads `25.40x0.00` once its
    /// construction tick on a switched-off layer is left out). A placement
    /// the drawing carries no drawable body for is measured on what was
    /// drawn instead, which is the library body or the marker dot. Every
    /// placement writes it (K-D3).
    extent: Option<String>,
    /// `<name>:<mm>;<name>:<mm>;…`, two places, in the template's on-disk
    /// order: the named driving dimensions of the library template the
    /// cached body names (`PidSymbolDefinition::template`). `None` for a
    /// placement whose body names no template -- every symbol that is not
    /// parametric, and a parametric one pid-parse could not pair -- so the
    /// panel shows no default it cannot vouch for. A dimension without a name
    /// (a derived one, or one no relation writes) is left out.
    driving: Option<String>,
    /// `<name>:<mm>;<name>:<mm>;…`, two places, in the symbol's variable
    /// order: the parameters this drawing's instance was actually placed
    /// with, read off the instance storage's `JFlavorHolder`
    /// (`PidSymbolVariable::instance_value_m`; pid-parse
    /// `docs/analysis/2026-09-20-jflavorholder-carries-the-placed-instances-parameters.md`).
    /// DWG-0201's Manifold reads `Left:57.91;Right:114.30;Top:35.59` here
    /// against `Top:20.32;Left:114.30;Right:114.30` in [`Self::driving`]. An
    /// unstretched instance repeats the defaults. `None` for a placement
    /// whose body carries no variables with an instance value.
    instance: Option<String>,
}

impl PlacementMeasures {
    /// The measures of one placement, or `None` for any other kind of record.
    fn of(
        kind: &PidGraphicKind,
        geometry: &NormalizedPidGeometry,
        projection: Projection,
        drawn: &[EntityType],
    ) -> Option<Self> {
        let PidGraphicKind::SymbolInstance {
            insertion,
            rotation,
            scale,
            definition,
            ..
        } = kind
        else {
            return None;
        };
        let placement = Placement {
            insertion,
            rotation: *rotation,
            scale: *scale,
            projection,
        };
        let cached = definition
            .and_then(|reference| geometry.symbol_definition(reference))
            .filter(|body| !body.primitives.is_empty());
        let extent = cached
            .and_then(|body| {
                body.visible_primitives()
                    .filter_map(|primitive| shape_primitive(primitive, &placement))
                    .filter_map(|entity| stroke_extent(&entity))
                    .reduce(union_extent)
            })
            .or_else(|| drawn.iter().filter_map(stroke_extent).reduce(union_extent))
            .map(|(min_x, min_y, max_x, max_y)| {
                format!("{:.2}x{:.2}", max_x - min_x, max_y - min_y)
            });
        let driving = cached
            .and_then(|body| body.template)
            .and_then(|template| geometry.symbol_definition(template))
            .map(|template| {
                template
                    .dimensions
                    .iter()
                    .filter_map(|dimension| {
                        dimension
                            .name
                            .as_deref()
                            .map(|name| format!("{name}:{:.2}", projection.mm(dimension.value_m)))
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .filter(|driving| !driving.is_empty());
        let instance = cached
            .map(|body| {
                body.variables
                    .iter()
                    .filter_map(|variable| {
                        variable.instance_value_m.map(|value_m| {
                            format!("{}:{:.2}", variable.name, projection.mm(value_m))
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .filter(|instance| !instance.is_empty());
        Some(Self {
            extent,
            driving,
            instance,
        })
    }
}

/// The rectangle one stroke of a body covers, in millimetres, as
/// `(min_x, min_y, max_x, max_y)`; `None` for lettering and for anything
/// that is not a stroke.
///
/// Unlike [`drawn_extent`], which hangs labels and frames views and may take
/// an arc as its whole circle, this is a number a user reads off the panel,
/// so an arc contributes exactly the sweep it draws: its two ends and
/// whichever quadrant points the sweep passes.
fn stroke_extent(entity: &EntityType) -> Option<(f64, f64, f64, f64)> {
    match entity {
        EntityType::Line(_) | EntityType::Circle(_) | EntityType::LwPolyline(_) => {
            drawn_extent(entity)
        }
        EntityType::Arc(arc) => Some(arc_extent(
            (arc.center.x, arc.center.y),
            arc.radius,
            arc.start_angle,
            arc.end_angle,
        )),
        _ => None,
    }
}

/// The rectangle an arc sweeping counter-clockwise from `start_angle` to
/// `end_angle` (radians) covers. Equal angles are read as the whole circle,
/// which is what a DXF arc with coincident ends draws.
fn arc_extent(
    center: (f64, f64),
    radius: f64,
    start_angle: f64,
    end_angle: f64,
) -> (f64, f64, f64, f64) {
    use std::f64::consts::{FRAC_PI_2, TAU};
    let start = start_angle.rem_euclid(TAU);
    let mut sweep = (end_angle - start_angle).rem_euclid(TAU);
    if sweep == 0.0 {
        sweep = TAU;
    }
    let end = start + sweep;
    let mut angles = vec![start, end];
    let mut quadrant = (start / FRAC_PI_2).ceil() * FRAC_PI_2;
    while quadrant <= end + 1e-12 {
        angles.push(quadrant);
        quadrant += FRAC_PI_2;
    }
    angles
        .into_iter()
        .map(|angle| {
            let (sin, cos) = angle.sin_cos();
            let (x, y) = (center.0 + radius * cos, center.1 + radius * sin);
            (x, y, x, y)
        })
        .reduce(union_extent)
        .expect("two ends at least")
}

/// The style table's entry for one normalized entity, if it has one.
///
/// The join is `(stream path, graphic oid)`, which is what `pid-parse` keys
/// the table on. Four families carry a style reference and are in it — lines,
/// points, linestrings, and symbol placements (whose one style covers the
/// placed body) — so text and every kind of evidence simply miss.
fn style_for<'a>(
    styles: &'a LineStyleIndex,
    entity: &pid_parse::PidGraphicEntity,
) -> Option<&'a ResolvedLineStyle> {
    let stream = entity.source.stream_path.as_deref()?;
    let oid = entity.graphic_oid?;
    styles.get(&(stream.to_string(), oid))
}

/// What the drawing calls the style one entity draws with, when it names it.
///
/// A second join, one hop past [`style_for`]: that one gives the record its
/// style, this one asks the drawing what it calls that style. The key is
/// `(stream path, style id)` rather than the graphic oid, because a name
/// belongs to the style — one `Primary Piping - New` covers thirty-nine
/// records across this corpus. The name is written to XDATA as `style=` and,
/// through [`discipline_layer`], keys the line work while the document is
/// built.
fn style_name_for<'a>(
    names: &'a StyleNameIndex,
    entity: &pid_parse::PidGraphicEntity,
    style: Option<&ResolvedLineStyle>,
) -> Option<&'a str> {
    let stream = entity.source.stream_path.as_deref()?;
    let style_id = style?.style_id;
    names
        .get(&(stream.to_string(), style_id))
        .map(String::as_str)
}

/// The working key for one authored style name, or `None` when the name is
/// not a discipline. A `PID-STYLE-*` layer name while the document is built;
/// the slot ends up holding the authored sheet layer (see `load_pid`), and
/// until plan 2026-09-21 retired the taxonomy layer mode this was also the
/// output layer.
///
/// # Why the drawing's own name and not a taxonomy of ours
///
/// Because the name carries information the rest of the import cannot
/// recover. Width and colour do not separate these: `0.350mm #800000` is both
/// `Nozzle - New` and the `Equipment - New` the nozzle sits on, and `0.350mm
/// #808000` is three different roles of piping. Mapping those onto a fixed
/// set of disciplines would mean this importer deciding which bucket a name
/// belongs in, and the file already decided — see `pid-parse`'s
/// `docs/pid-format-guide.md` §6.1. So the key is the name, and a drawing
/// whose project library uses a vocabulary nobody here has seen is keyed by
/// it with no code change.
///
/// Two sets are excluded, both because they are already carried elsewhere.
///
/// The review statuses are the four states a point's mark shows, and
/// `PID-POINT-WARNING` and its siblings hold them. Every point in the corpus
/// resolves to one, so this is also what keeps point marks out of the
/// discipline keys.
///
/// The appearance names say how a line is drawn rather than what it is, and
/// `PID-GEOMETRY` plus the linetype already say that. Excluding them is the
/// one place this function does decide something the file did not spell out,
/// so the evidence is in `APPEARANCE_STYLE_NAMES` next to the list.
///
/// Names that share every alphanumeric character land on one key —
/// `As Drawn` and `as-drawn` would merge. Nothing in the corpus does, and
/// merging two spellings of one name is a better failure than emitting a
/// layer name `DXF` cannot round-trip.
fn discipline_layer(name: &str) -> Option<String> {
    if REVIEW_STATUS_STYLE_NAMES.contains(&name) || APPEARANCE_STYLE_NAMES.contains(&name) {
        return None;
    }
    let mut layer = String::from(LAYER_DISCIPLINE_PREFIX);
    let mut gap = false;
    for character in name.chars() {
        if character.is_alphanumeric() {
            if gap && layer.len() > LAYER_DISCIPLINE_PREFIX.len() {
                layer.push('-');
            }
            gap = false;
            layer.extend(character.to_uppercase());
        } else {
            gap = true;
        }
    }
    (layer.len() > LAYER_DISCIPLINE_PREFIX.len()).then_some(layer)
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

/// The code points SmartPlant may end a line of a label on.
///
/// The corpus only ever uses `U+000D`, a bare carriage return with no line
/// feed — measured over all 235 `igTextBox` records in `pid-parse`'s
/// `examples/probe_text_multiline_census`. The rest are here because splitting
/// on a break this list is missing is the failure that looks like nothing:
/// the label renders with its lines run together and no error anywhere.
const TEXT_LINE_BREAKS: [char; 7] = [
    '\u{000A}', '\u{000D}', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
];

/// Turn a label that carries line breaks into one text entity per line,
/// stacked down the page at the pitch its paragraph style asks for.
///
/// Four labels in the reference corpus have a second line, and until this they
/// imported as a single TEXT whose `U+000D` is a code point no stroke font has
/// a glyph for: the lines ran together on one baseline with a blank gap where
/// each break should have been. A DXF TEXT entity is single-line by
/// definition, so there is nowhere to put a line break except in more entities.
///
/// **`line_spacing` is the reason this takes a parameter rather than assuming
/// single spacing.** `JStyleTextPara +66` states the multiple, and in this
/// corpus the two labels with a second line both state `1.5` while all 228
/// one-line labels state `1.0` — so the field and the line breaks agree about
/// which labels they concern. Measured in pid-parse's
/// `docs/analysis/2026-08-22-four-labels-have-a-second-line.md`. A label whose
/// paragraph states nothing usable stacks at single spacing, which is what a
/// consumer with no stated pitch has to assume anyway.
///
/// Scoped like [`apply_text_height`] and its siblings: only the sheet's own
/// lettering on [`LAYER_TEXT`]. A symbol's internal text is placed by the
/// `.sym` library, which carries no paragraph style and no breaks.
///
/// Lines are offset perpendicular to the baseline rather than straight down,
/// so a label rotated to read up the page stacks across the page — the corpus
/// letters at 0, 90 and 180 degrees, and a vertical label stacked downwards
/// would overprint itself.
fn stack_text_lines(
    built: Vec<EntityType>,
    height_mm: f64,
    line_spacing: Option<f64>,
) -> Vec<EntityType> {
    if !built.iter().any(is_multi_line_label) {
        return built;
    }
    let pitch = height_mm * line_spacing.unwrap_or(1.0);
    let mut out = Vec::with_capacity(built.len());
    for one in built {
        if !is_multi_line_label(&one) {
            out.push(one);
            continue;
        }
        let EntityType::Text(text) = one else {
            unreachable!("is_multi_line_label admits only a Text");
        };
        // Down the page as the reader sees it: the baseline runs along
        // `rotation`, so the next line sits one pitch along its right normal.
        let (sin, cos) = text.rotation.sin_cos();
        let step = Vector3::new(pitch * sin, -pitch * cos, 0.0);
        for (index, line) in text
            .value
            .replace("\r\n", "\n")
            .split(TEXT_LINE_BREAKS)
            .enumerate()
        {
            // A blank line still occupies its slot -- the offset comes from
            // the index -- but it is not worth an entity, which is the same
            // call `build_entities` makes for an empty label.
            if line.trim().is_empty() {
                continue;
            }
            let mut one_line = text.clone();
            one_line.value = line.to_string();
            #[allow(clippy::cast_precision_loss)]
            let offset = index as f64;
            one_line.insertion_point = Vector3::new(
                text.insertion_point.x + step.x * offset,
                text.insertion_point.y + step.y * offset,
                text.insertion_point.z,
            );
            // `apply_text_alignment` re-seeds the alignment point from the
            // insertion point afterwards, so a stale one from the clone would
            // be overwritten -- but only when the paragraph states an
            // alignment. Clear it here so a label that states none cannot
            // letter every line from the first line's anchor.
            one_line.alignment_point = None;
            out.push(EntityType::Text(one_line));
        }
    }
    out
}

/// Whether this is a sheet label with more than one line in it.
fn is_multi_line_label(entity: &EntityType) -> bool {
    let EntityType::Text(text) = entity else {
        return false;
    };
    text.common.layer == LAYER_TEXT && text.value.contains(TEXT_LINE_BREAKS)
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
/// The drawing's own line work is painted: `PID-GEOMETRY` and the
/// `PID-STYLE-*` discipline layers its named line work files under, every
/// `PID-POINT` layer — the bare one and the three review statuses, which is
/// why the test is a prefix — and the placed symbol bodies (`PID-SYMBOL`),
/// whose placement
/// record names one style for the whole body — `igSymbol2d +25` — that wins
/// over the per-stroke styles the body states (see [`paint_symbol_stroke`],
/// which ran first and is overwritten here exactly when the placement names a
/// style). Colour and width are what it overwrites; the linetype only when
/// the placement's style names a dash of its own, so a stroke the body dashes
/// stays dashed under a solid placement style (plan 2026-09-20, P-E1).
/// `PID-CONNECTIVITY` is a diagnostic whose layer colour *is* the
/// diagnosis, and repainting it in the drawing's palette would hide the thing
/// it exists to show.
fn apply_symbology(
    entity: &mut EntityType,
    style: &ResolvedLineStyle,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) {
    // A placed symbol's lettering follows the placement's colour the same
    // way its line work does. This is measured, not assumed: LG and LT
    // author their bubble letters `#FF0000` in their own `.sym` character
    // styles, the placements name `#008000`, and the screenshot letters
    // them green. Size and typeface stay the `.sym`'s own — the placement
    // names a line style, which carries no lettering metrics — and a line
    // weight on a text entity would mean nothing, so colour is all that is
    // painted here.
    if let EntityType::Text(_) = entity {
        let common = entity.common_mut();
        if common.layer == LAYER_SYMBOL {
            let [r, g, b] = style.symbology.rgb();
            common.color = Color::from_rgb(r, g, b);
        }
        return;
    }
    let common = entity.common_mut();
    let painted = common.layer == LAYER_GEOMETRY
        || common.layer.starts_with(LAYER_DISCIPLINE_PREFIX)
        || common.layer == LAYER_SYMBOL
        || common.layer.starts_with(LAYER_POINT);
    if !painted {
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
        if let Some(name) = dash_linetypes.get(&dash_key(&dash.segments_mm())) {
            common.linetype = name.clone();
        }
    }
}

/// A dedup key for a dash pattern stated as segment lengths in millimetres:
/// its segment magnitudes, in micrometres.
///
/// Two patterns that differ only in sign render identically — see
/// [`build_dash_linetype`] — so the key is built from magnitudes, and the
/// micrometre rounding folds together patterns that agree to within a
/// nanometre of float noise. Both sources of a dash speak this vocabulary:
/// the drawing's line styles through [`DashPattern::segments_mm`], a cached
/// body's strokes through [`PrimitiveStyle::dash_mm`].
fn dash_key(segments_mm: &[f64]) -> Vec<i64> {
    segments_mm
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
/// a DWG shipped. The drawing's own line styles are pooled first, over a
/// `BTreeMap`, then the dash each stroke of the bodies the drawing caches
/// states for itself (plan 2026-09-20, P-E8), in the cache's order -- so the
/// names are stable for a given file, and the ones the sheet's line work was
/// given do not move when a body's dash joins the pool. A library body's dash
/// is not registered here: which `.sym` a placement falls back to is only
/// known as it is drawn, so its dash draws exactly when the pool already holds
/// the pattern (see [`paint_symbol_stroke`]).
fn register_dash_linetypes(
    doc: &mut CadDocument,
    styles: &LineStyleIndex,
    cached_bodies: &[PidSymbolDefinition],
) -> HashMap<Vec<i64>, String> {
    let mut names: HashMap<Vec<i64>, String> = HashMap::new();
    let drawing = styles
        .values()
        .filter_map(|style| style.dash.as_ref())
        .map(DashPattern::segments_mm);
    let cached = cached_bodies
        .iter()
        .flat_map(|body| body.primitive_styles.iter().flatten())
        .filter(|style| !style.dash_mm.is_empty())
        .map(|style| style.dash_mm.clone());
    for segments_mm in drawing.chain(cached) {
        let key = dash_key(&segments_mm);
        if key.is_empty() || names.contains_key(&key) {
            continue;
        }
        let name = format!("PID-DASH-{}", names.len() + 1);
        if !doc.line_types.contains(&name) {
            let mut lt = build_dash_linetype(&name, &segments_mm);
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

/// Build a document linetype from a decoded dash pattern, stated as its
/// segment lengths in millimetres.
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
fn build_dash_linetype(name: &str, segments_mm: &[f64]) -> LineType {
    let mut lt = LineType::new(name);
    lt.description = format!("P&ID dash pattern ({} segments)", segments_mm.len());
    let mut pattern_length = 0.0;
    for (i, mm) in segments_mm.iter().enumerate() {
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

/// `line_work` is the layer the sheet's own lines, arcs and rings go on:
/// `PID-GEOMETRY`, or the discipline layer when the drawing names the style
/// they draw with (see [`discipline_layer`]). It is decided by the caller and
/// passed in rather than patched afterwards, so there is one place that
/// answers "which layer is this line on".
///
/// `dash_linetypes` is the pool [`register_dash_linetypes`] filled, for the
/// dash a symbol body's own stroke style names (see [`paint_symbol_stroke`]).
fn build_entities(
    kind: &PidGraphicKind,
    bodies: BodySources<'_>,
    projection: Projection,
    symbology: Option<&ResolvedLineStyle>,
    line_work: &str,
    symbol_bodies: &mut SymbolBodies,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) -> Vec<EntityType> {
    match kind {
        PidGraphicKind::Line { start, end } => {
            let mut line = Line::from_points(projection.point(start), projection.point(end));
            line.common.layer = line_work.to_string();
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
            polyline.common.layer = line_work.to_string();
            vec![EntityType::LwPolyline(polyline)]
        }
        PidGraphicKind::Circle { center, radius } => {
            let mut circle = Circle::new();
            circle.center = projection.point(center);
            circle.radius = projection.mm(*radius);
            circle.common.layer = line_work.to_string();
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
            // Radians on both sides -- see the angle-unit note by the layer
            // constants. The file's arc runs clockwise from start to end and
            // a DXF arc counter-clockwise, so the ends swap (see
            // `shape_primitive`); no sheet of the corpus carries an arc of
            // its own, so this arm follows the record's convention rather
            // than a measured case.
            arc.start_angle = *end_angle;
            arc.end_angle = *start_angle;
            arc.common.layer = line_work.to_string();
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
            ..
        } => {
            let placement = Placement {
                insertion,
                rotation: *rotation,
                scale: *scale,
                projection,
            };
            // Two bodies can answer for a placement, and the drawing's own
            // is asked first (plan 2026-09-19, P-D1 / P-D3): a `.pid` caches
            // every symbol it places inside itself, keyed by the placement's
            // own definition reference, and that copy is the flavour
            // SmartPlant placed and the instance as it was resized -- where
            // the library `.sym` is the template, every sheet of it merged.
            // The library stands in where the drawing caches nothing
            // drawable. Either body arrives painted in its own per-stroke
            // styles, and only the dash of that coat survives:
            // `apply_symbology` repaints the colour and width in the
            // placement's style afterwards, so the two routes end up in the
            // same colour and width (plan 2026-09-20, P-E1). Measured in
            // pid-parse's `docs/analysis/
            // 2026-09-07-placement-tail-names-the-cached-definition.md`.
            let symbol_path = symbol_path.as_deref();
            let BodySources { cached, library } = bodies;
            let body = cached_body_entities(cached, &placement, symbol_bodies, dash_linetypes)
                .or_else(|| {
                    library_body_entities(
                        library,
                        symbol_path,
                        &placement,
                        symbol_bodies,
                        dash_linetypes,
                    )
                });

            // Only fall back to the marker when the body is genuinely
            // unavailable. A symbol that resolved to real geometry should not
            // also carry a dot -- that reads as a second object.
            let mut built = match body {
                Some(entities) => entities,
                None => {
                    symbol_bodies.markers += 1;
                    let mut marker = Circle::new();
                    marker.center = projection.point(insertion);
                    marker.radius = SYMBOL_MARKER_RADIUS_MM;
                    marker.common.layer = LAYER_SYMBOL.to_string();
                    vec![EntityType::Circle(marker)]
                }
            };

            if let Some(name) = symbol_path.and_then(symbol_name) {
                let mut label = Text::new();
                label.value = name;
                label.height = SYMBOL_LABEL_HEIGHT_MM;
                label.insertion_point = symbol_label_anchor(
                    &built,
                    (projection.mm(insertion.x), projection.mm(insertion.y)),
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
            let mut built = vec![EntityType::Point(point)];
            // Whether a point shows a mark, what shape that mark is, and what
            // it means are all stated by the file. Its line style may name a
            // `JStyleLineTerminator`, which names a `JStylePointSymbol`,
            // which owns its glyph as a group of line records and is named by
            // the style librarian. A symbol whose lines are all zero-length is
            // `psOk` -- an item that passed has nothing to draw -- and 53 of
            // DWG-0201's 75 points are exactly that.
            //
            // Drawn at the size the group states. SmartPlant's own screen
            // draws it larger, but so are that screen's line weights, so the
            // factor is a view-wide scale on style-declared sizes rather than
            // anything the file says about this glyph. See pid-parse
            // `docs/analysis/
            // 2026-08-25-a-point-draws-the-symbol-its-terminator-names.md`.
            let marker = symbology
                .and_then(|style| style.marker)
                .filter(PointMarker::draws);
            // A mark whose status the librarian does not name stays on
            // `PID-POINT`: filing it under a status would be this importer
            // deciding one, which is the drawing's job.
            let mark_layer = match marker.and_then(|marker| marker.status) {
                Some(MarkerStatus::Warning) => LAYER_POINT_WARNING,
                Some(MarkerStatus::Error) => LAYER_POINT_ERROR,
                Some(MarkerStatus::Approved) => LAYER_POINT_APPROVED,
                Some(MarkerStatus::Ok) | None => LAYER_POINT,
            };
            let origin = projection.point(position);
            let offset = |(x, y): (f64, f64)| {
                Vector3::new(
                    origin.x + projection.mm(x),
                    origin.y + projection.mm(y),
                    0.0,
                )
            };
            for stroke in marker.iter().flat_map(PointMarker::strokes) {
                if stroke.is_degenerate() {
                    continue;
                }
                let mut segment = Line::from_points(offset(stroke.start), offset(stroke.end));
                segment.common.layer = mark_layer.to_string();
                built.push(EntityType::Line(segment));
            }
            built
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
        self.add_mm(self.projection.mm(point.x), self.projection.mm(point.y));
    }

    /// Take in a point that is already on the sheet in millimetres.
    fn add_mm(&mut self, x: f64, y: f64) {
        if !self.projection.band.holds(x) || !self.projection.band.holds(y) {
            return;
        }
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    /// Take in the rectangle a set of drawn entities covers.
    fn add_drawn(&mut self, entities: &[EntityType]) {
        let Some((min_x, min_y, max_x, max_y)) = drawn_bounds(entities) else {
            return;
        };
        self.add_mm(min_x, min_y);
        self.add_mm(max_x, max_y);
    }

    fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }
}

fn accumulate_bounds(kind: &PidGraphicKind, built: &[EntityType], bounds: &mut Bounds) {
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
        PidGraphicKind::Text { insertion, .. } => bounds.add(insertion),
        // A placement is framed on what it drew rather than on its insertion
        // point. The two agree only for a symbol whose library body is drawn
        // around its own origin; for the third of the library that is not,
        // the anchor is up to 200mm from any of the symbol's own line work,
        // and framing on it opened the drawing zoomed out over sheet nothing
        // reaches. `DWG-0202`'s `ext_min` used to read x = -25.64 on the
        // strength of six anchors whose symbols all draw inside the border.
        PidGraphicKind::SymbolInstance { .. } => bounds.add_drawn(built),
        PidGraphicKind::Point { position } => bounds.add(position),
        // Never reached: both kinds only ever arrive inferred or probe-only,
        // and the caller frames on decoded geometry alone.
        PidGraphicKind::Annotation { .. } | PidGraphicKind::Unknown { .. } => {}
    }
}

/// The rectangle a drawn entity covers, in millimetres, as
/// `(min_x, min_y, max_x, max_y)`.
///
/// An arc is taken as its whole circle, which over-reports a sweep that does
/// not reach every quadrant. Both callers want somewhere to hang a label and
/// a view to frame; neither is harmed by a millimetre of slack, and the
/// quadrant walk that would remove it is not worth carrying. Lettering
/// contributes its insertion point alone, since how far a run reaches needs
/// the font the renderer will pick.
fn drawn_extent(entity: &EntityType) -> Option<(f64, f64, f64, f64)> {
    let around = |x: f64, y: f64, radius: f64| (x - radius, y - radius, x + radius, y + radius);
    match entity {
        EntityType::Line(line) => Some((
            line.start.x.min(line.end.x),
            line.start.y.min(line.end.y),
            line.start.x.max(line.end.x),
            line.start.y.max(line.end.y),
        )),
        EntityType::Circle(circle) => Some(around(circle.center.x, circle.center.y, circle.radius)),
        EntityType::Arc(arc) => Some(around(arc.center.x, arc.center.y, arc.radius)),
        EntityType::LwPolyline(polyline) => polyline
            .vertices
            .iter()
            .map(|vertex| {
                (
                    vertex.location.x,
                    vertex.location.y,
                    vertex.location.x,
                    vertex.location.y,
                )
            })
            .reduce(union_extent),
        EntityType::Text(text) => Some((
            text.insertion_point.x,
            text.insertion_point.y,
            text.insertion_point.x,
            text.insertion_point.y,
        )),
        _ => None,
    }
}

/// Where a placement's name is lettered: clear of the right edge of what the
/// placement drew, level with its middle, and horizontal whatever the
/// placement angle is -- a rotated label is the harder read.
///
/// It reads off the drawn body rather than off `insertion_mm`, because those
/// two are the same point only for a symbol whose library body is drawn
/// around its own origin, and a third of the reference library is not: 211 of
/// its 613 readable `.sym` are authored a hundred millimetres or more away,
/// `Design` and `Equipment` almost entirely so. Naming those from the anchor
/// put the name off the sheet while the symbol itself sat well inside it --
/// `docs/analysis/2026-08-24-two-texts-outside-the-frame.md`.
///
/// The marker fallback keeps the position it always had: a dot of
/// `SYMBOL_MARKER_RADIUS_MM` centred on the insertion point reaches exactly
/// that far right of it and is level with it, so the arithmetic below comes
/// out at the `+ radius + gap` the old formula spelled out. `insertion_mm` is
/// only reached when a placement drew nothing at all, which the marker exists
/// to prevent.
fn symbol_label_anchor(drawn: &[EntityType], insertion_mm: (f64, f64)) -> Vector3 {
    let (_, min_y, max_x, max_y) = drawn_bounds(drawn).unwrap_or((
        insertion_mm.0,
        insertion_mm.1,
        insertion_mm.0,
        insertion_mm.1,
    ));
    Vector3::new(
        max_x + SYMBOL_LABEL_GAP_MM,
        (min_y + max_y) / 2.0 - SYMBOL_LABEL_HEIGHT_MM / 2.0,
        0.0,
    )
}

/// The rectangle a set of drawn entities covers, or `None` where none of them
/// has an extent this module can state.
fn drawn_bounds(entities: &[EntityType]) -> Option<(f64, f64, f64, f64)> {
    entities
        .iter()
        .filter_map(drawn_extent)
        .reduce(union_extent)
}

fn union_extent(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

/// Where a symbol placement's body can come from: the body the drawing
/// caches for this placement, asked first, and the library it falls back
/// to. Handed over per entity, since the cached body is the placement's own.
struct BodySources<'a> {
    /// The drawing's own definition for the placement, when it has one.
    cached: Option<&'a PidSymbolDefinition>,
    /// The library found beside the drawing, when one was.
    library: Option<&'a mut SymbolLibrary>,
}

/// The body the drawing caches for a placement, drawn at it: the strokes on
/// the layers the file displays ([`PidSymbolDefinition::visible_strokes`]),
/// each in the style its own storage states for it (the coat
/// [`apply_symbology`] paints over), or `None` when the drawing caches
/// nothing for the placement or nothing of it draws. Tallies the body and
/// the strokes left out for sitting on a layer the file switches off (plan
/// 2026-09-19, P-D2) -- a count the placement earns whether or not what
/// remains is enough to draw, since those strokes are not drawn either way.
/// The same selection feeds [`PlacementMeasures`], so what is on screen and
/// what the panel says is the size of are one set of strokes.
fn cached_body_entities(
    body: Option<&PidSymbolDefinition>,
    at: &Placement<'_>,
    symbol_bodies: &mut SymbolBodies,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) -> Option<Vec<EntityType>> {
    let body = body?;
    let strokes: Vec<(&SymbolPrimitive, Option<&PrimitiveStyle>)> =
        body.visible_strokes().collect();
    symbol_bodies.hidden_strokes_skipped += body.primitives.len() - strokes.len();
    let entities: Vec<EntityType> = strokes
        .into_iter()
        .filter_map(|(primitive, style)| place_primitive(primitive, style, at, dash_linetypes))
        .collect();
    if entities.is_empty() {
        return None;
    }
    symbol_bodies.cache += 1;
    Some(entities)
}

/// The library's body for a placement, drawn at it in the `.sym`'s own
/// stroke styles (the coat [`apply_symbology`] paints over), or `None` when
/// no library was found, the library has no `.sym` for the path, or the body
/// draws nothing.
fn library_body_entities(
    library: Option<&mut SymbolLibrary>,
    symbol_path: Option<&str>,
    at: &Placement<'_>,
    symbol_bodies: &mut SymbolBodies,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) -> Option<Vec<EntityType>> {
    let body = library
        .zip(symbol_path)
        .and_then(|(library, path)| library.resolve(path))?;
    let entities: Vec<EntityType> = body
        .primitives
        .iter()
        .filter_map(|styled| {
            place_primitive(&styled.primitive, styled.style.as_ref(), at, dash_linetypes)
        })
        .collect();
    if entities.is_empty() {
        return None;
    }
    symbol_bodies.library += 1;
    Some(entities)
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

/// Draw one primitive of a symbol body at its placement, in the colour,
/// width and dash the symbol states for it -- the one route both bodies
/// take, the drawing's cached one and the library's.
fn place_primitive(
    primitive: &SymbolPrimitive,
    style: Option<&PrimitiveStyle>,
    at: &Placement<'_>,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) -> Option<EntityType> {
    let mut entity = shape_primitive(primitive, at)?;
    paint_symbol_stroke(&mut entity, style, dash_linetypes);
    Some(entity)
}

/// Give a symbol's stroke the colour, width and dash its own body states --
/// the `.sym`'s style table for a library body, the storage's own
/// `StyleCluster` for a cached one.
///
/// This is the undercoat, not the final one. A placement record *does*
/// name a style — `igSymbol2d +25`, a slot this route once believed absent —
/// and where it resolves, [`apply_symbology`] repaints the whole body over
/// what is painted here: DWG-0201's vessel is authored black in
/// `Parametric Manifold.sym` and SmartPlant screens it in the placement's
/// `#800000`. What this coat still decides is the body of a placement whose
/// style does not resolve, and the strokes' dash either way (plan
/// 2026-09-20, P-E1): the placement styles of the corpus are all solid, and
/// the dashed circle and legs of an off-page connector are dashed by the
/// symbol's own style alone. A symbol whose own style index names no line
/// style keeps `ByLayer`, which is what it drew as before this.
///
/// The dash is named to the `PID-DASH-<n>` linetype
/// [`register_dash_linetypes`] pooled for its pattern. A pattern the pool
/// does not hold -- only a library body can state one, since the cached
/// bodies were pooled up front -- draws solid, as it did before, and says
/// so in the log.
fn paint_symbol_stroke(
    entity: &mut EntityType,
    style: Option<&PrimitiveStyle>,
    dash_linetypes: &HashMap<Vec<i64>, String>,
) {
    let Some(style) = style else {
        return;
    };
    let common = entity.common_mut();
    let [r, g, b] = style.rgb;
    common.color = Color::from_rgb(r, g, b);
    // Same ladder and the same guard as the drawing's own line work: DXF
    // stores hundredths of a millimetre, and a width outside the range the
    // format can state is left at the layer default rather than clamped into
    // a width the symbol did not ask for.
    let hundredths = (style.width_mm * 100.0).round();
    if (0.0..=211.0).contains(&hundredths) {
        common.line_weight = LineWeight::Value(hundredths as i16);
    }
    if style.dash_mm.is_empty() {
        return;
    }
    match dash_linetypes.get(&dash_key(&style.dash_mm)) {
        Some(name) => common.linetype = name.clone(),
        None => log::debug!(
            "a symbol stroke names a dash pattern {:?} mm the drawing did not pool; drawing it solid",
            style.dash_mm
        ),
    }
}

/// Where one primitive of a symbol body lands at its placement.
fn shape_primitive(primitive: &SymbolPrimitive, at: &Placement<'_>) -> Option<EntityType> {
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
            // The file's arc runs **clockwise** from its start angle to its
            // end angle (pid-parse `docs/analysis/2026-09-19-igarc2d-sweeps-
            // clockwise-from-start-to-end.md`: the Manifold's caps bulge out
            // of its shell, to where its construction axes end, only read
            // that way), and a DXF arc always runs counter-clockwise from
            // start to end -- so the same arc is drawn from the file's end
            // angle to its start angle. A mirrored placement reverses the
            // sense once more and reflects the angles, which lands it back
            // on the file's own order. Either way round the other, and the
            // arc is drawn as its own complement.
            let (start_angle, end_angle) = if at.mirrored() {
                (at.rotation - start_angle, at.rotation - end_angle)
            } else {
                (at.rotation + end_angle, at.rotation + start_angle)
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
        SymbolPrimitive::Polyline {
            vertices,
            is_closed,
        } => {
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
            polyline.is_closed = *is_closed;
            polyline.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::LwPolyline(polyline))
        }
        SymbolPrimitive::BSpline {
            poles,
            weights,
            knots,
        } => {
            // Sampled in the symbol's own space and placed point by point, so
            // the placement's rotation, scale and mirror fall out of the same
            // `apply` a polyline's vertices go through. The segment count is
            // pid-parse's own, so the curve looks the same here as it does
            // when a sheet carries one directly.
            let points: Vec<Vector2> = pid_parse::bspline::sample(
                poles,
                weights,
                knots,
                pid_parse::bspline::SEGMENTS_PER_SPAN,
            )
            .into_iter()
            .map(|(x, y)| {
                let placed = at.apply(x, y);
                Vector2::new(placed.x, placed.y)
            })
            .collect();
            if points.len() < 2 {
                return None;
            }
            let mut curve = LwPolyline::from_points(points);
            curve.common.layer = LAYER_SYMBOL.to_string();
            Some(EntityType::LwPolyline(curve))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's display bit decides, and the name only speaks when the
    /// file said nothing (plan 2026-09-07, L1): a layer named `Invisible`
    /// the sheet displays is drawn, a layer named `Default` the sheet hides
    /// is hidden, and a layer without a bit falls back to the three names.
    #[test]
    fn the_display_bit_outranks_the_name_and_the_name_is_only_the_fallback() {
        let layer = |name: &str, displayed: Option<bool>| PidSourceLayer {
            oid: 8,
            name: Some(name.to_string()),
            storage_path: "/".to_string(),
            displayed,
        };
        assert!(!sheet_layer_is_hidden(&layer("Invisible", Some(true))));
        assert!(sheet_layer_is_hidden(&layer("Default", Some(false))));
        assert!(sheet_layer_is_hidden(&layer("HiddenObjects", Some(false))));
        assert!(sheet_layer_is_hidden(&layer("Hidden", None)));
        assert!(sheet_layer_is_hidden(&layer("Invisible", None)));
        assert!(!sheet_layer_is_hidden(&layer("Labels", None)));
        assert!(!sheet_layer_is_hidden(&PidSourceLayer {
            oid: 8,
            name: None,
            storage_path: "/".to_string(),
            displayed: None,
        }));
    }

    /// The extent a user reads off the panel follows the arc's sweep rather
    /// than its whole circle (plan 2026-09-18, K2): a quarter reaches one
    /// corner, a half one side, a sweep across the x axis takes in the point
    /// at angle zero, and coincident ends are the full circle.
    #[test]
    fn an_arcs_extent_is_the_sweep_it_draws_and_not_its_circle() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        let close = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
            (a.0 - b.0).abs() < 1e-9
                && (a.1 - b.1).abs() < 1e-9
                && (a.2 - b.2).abs() < 1e-9
                && (a.3 - b.3).abs() < 1e-9
        };
        let quarter = arc_extent((0.0, 0.0), 1.0, 0.0, FRAC_PI_2);
        assert!(close(quarter, (0.0, 0.0, 1.0, 1.0)), "{quarter:?}");
        let half = arc_extent((0.0, 0.0), 1.0, FRAC_PI_2, 3.0 * FRAC_PI_2);
        assert!(close(half, (-1.0, -1.0, 0.0, 1.0)), "{half:?}");
        let across_zero = arc_extent((0.0, 0.0), 1.0, 7.0 * FRAC_PI_4, FRAC_PI_4);
        let c = FRAC_PI_4.cos();
        assert!(close(across_zero, (c, -c, 1.0, c)), "{across_zero:?}");
        let whole = arc_extent((2.0, 3.0), 1.0, PI, PI);
        assert!(close(whole, (1.0, 2.0, 3.0, 4.0)), "{whole:?}");
        // The Manifold's end caps: two semicircles of r 35.59 facing outward
        // reach exactly the body's top and bottom and one end each.
        let mut arc = acadrust::entities::Arc::new();
        arc.center = Vector3::new(10.0, 10.0, 0.0);
        arc.radius = 35.59;
        arc.start_angle = FRAC_PI_2;
        arc.end_angle = 3.0 * FRAC_PI_2;
        let cap = stroke_extent(&EntityType::Arc(arc)).expect("an arc is a stroke");
        assert!(
            close(cap, (10.0 - 35.59, 10.0 - 35.59, 10.0, 10.0 + 35.59)),
            "{cap:?}"
        );
        let mut label = Text::new();
        label.value = "Parametric Manifold".to_string();
        assert!(stroke_extent(&EntityType::Text(label)).is_none());
    }

    fn marker_at(x: f64, y: f64) -> EntityType {
        let mut marker = Circle::new();
        marker.center = Vector3::new(x, y, 0.0);
        marker.radius = SYMBOL_MARKER_RADIUS_MM;
        EntityType::Circle(marker)
    }

    fn line(start: (f64, f64), end: (f64, f64)) -> EntityType {
        EntityType::Line(Line::from_points(
            Vector3::new(start.0, start.1, 0.0),
            Vector3::new(end.0, end.1, 0.0),
        ))
    }

    /// The marker fallback keeps the position the old anchor-relative formula
    /// gave it, to the last decimal: `insertion + radius + gap` across, and
    /// half a cap-height down from the insertion point.
    #[test]
    fn a_marker_is_named_where_the_insertion_point_formula_put_it() {
        let insertion = (40.0, 25.0);
        let anchor = symbol_label_anchor(&[marker_at(insertion.0, insertion.1)], insertion);
        assert_eq!(
            anchor.x,
            insertion.0 + SYMBOL_MARKER_RADIUS_MM + SYMBOL_LABEL_GAP_MM
        );
        assert_eq!(anchor.y, insertion.1 - SYMBOL_LABEL_HEIGHT_MM / 2.0);
    }

    /// A symbol whose library body is drawn away from its own origin is named
    /// beside the body. Reverting to the insertion point puts the name a
    /// hundred millimetres away from anything the placement drew, which on
    /// `DWG-0202` was off the edge of the sheet -- see
    /// `docs/analysis/2026-08-24-two-texts-outside-the-frame.md`.
    #[test]
    fn a_body_drawn_away_from_its_anchor_is_named_beside_the_body() {
        // `ElecTraceLine.sym` to scale: the placement is anchored at x = -25.6
        // and the body it draws sits 104mm right and 155mm up from there.
        let insertion = (-25.6, 349.9);
        let body = [
            line((78.2, 504.1), (82.1, 505.6)),
            line((82.1, 504.1), (78.2, 505.6)),
        ];
        let anchor = symbol_label_anchor(&body, insertion);

        assert_eq!(anchor.x, 82.1 + SYMBOL_LABEL_GAP_MM);
        assert_eq!(
            anchor.y,
            (504.1 + 505.6) / 2.0 - SYMBOL_LABEL_HEIGHT_MM / 2.0
        );
        assert!(
            anchor.x - insertion.0 > 100.0,
            "the name follows the body, not the anchor it was placed from"
        );
    }

    /// An arc contributes its whole circle and lettering contributes only the
    /// point it starts from, so the extent a label and the opening view are
    /// built on covers the line work either way.
    #[test]
    fn an_extent_covers_an_arc_and_stops_at_a_text_insertion_point() {
        let mut arc = acadrust::entities::Arc::new();
        arc.center = Vector3::new(10.0, 20.0, 0.0);
        arc.radius = 2.5;
        arc.start_angle = 0.0;
        arc.end_angle = std::f64::consts::FRAC_PI_2;
        assert_eq!(
            drawn_extent(&EntityType::Arc(arc)),
            Some((7.5, 17.5, 12.5, 22.5))
        );

        let mut text = Text::new();
        text.value = "pump".to_string();
        text.height = 2.5;
        text.insertion_point = Vector3::new(3.0, 4.0, 0.0);
        assert_eq!(
            drawn_extent(&EntityType::Text(text)),
            Some((3.0, 4.0, 3.0, 4.0))
        );
    }

    /// A cached body draws its strokes on the layers the file displays and
    /// none of the others (plan 2026-09-19, P-D2), and the placement is
    /// measured over the same strokes. A stroke on a layer the file says
    /// nothing about is drawn, and one with no layer entry at all (a body
    /// from before the field existed) too.
    #[test]
    fn a_cached_body_draws_only_the_strokes_on_its_displayed_layers() {
        use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};
        let line = |x: f64| SymbolPrimitive::Line {
            start: (x, 0.0),
            end: (x, 0.01),
        };
        let body = PidSymbolDefinition {
            reference: PidSymbolDefinitionRef {
                site: 396,
                sheet: 113,
            },
            layers: vec![8, 9, 10],
            sheet_layers: vec![
                PidSymbolSheetLayer {
                    oid: 8,
                    name: Some("Default".to_string()),
                    displayed: Some(true),
                },
                PidSymbolSheetLayer {
                    oid: 9,
                    name: Some("Construction".to_string()),
                    displayed: Some(false),
                },
                PidSymbolSheetLayer {
                    oid: 10,
                    name: None,
                    displayed: None,
                },
            ],
            primitives: vec![line(0.0), line(0.1), line(0.2), line(0.3), line(0.4)],
            primitive_layers: vec![8, 9, 10, 9],
            primitive_styles: Vec::new(),
            dimensions: Vec::new(),
            variables: Vec::new(),
            template: None,
        };
        let x_of = |stroke: &SymbolPrimitive| match stroke {
            SymbolPrimitive::Line { start, .. } => start.0,
            other => panic!("{other:?}"),
        };
        let drawn: Vec<f64> = body.visible_strokes().map(|(p, _)| x_of(p)).collect();
        assert_eq!(
            drawn,
            vec![0.0, 0.2, 0.4],
            "Default on, Construction off, an unlisted layer and a stroke with no layer entry drawn"
        );
        let measured: Vec<f64> = body.visible_primitives().map(x_of).collect();
        assert_eq!(measured, drawn, "the panel measures the strokes on screen");

        let insertion = PidPoint { x: 0.0, y: 0.0 };
        let at = Placement {
            insertion: &insertion,
            rotation: 0.0,
            scale: [1.0, 1.0],
            projection: Projection {
                mm_per_unit: 1000.0,
                band: SheetBand::for_page(None),
            },
        };
        let mut tally = SymbolBodies::default();
        let entities = cached_body_entities(Some(&body), &at, &mut tally, &HashMap::new())
            .expect("three strokes draw");
        assert_eq!(entities.len(), 3);
        assert_eq!(
            (tally.cache, tally.hidden_strokes_skipped),
            (1, 2),
            "one body drawn, its two Construction strokes counted as skipped"
        );
    }

    /// A cached stroke's dash is its own storage's, and its colour and width
    /// are its placement's (plan 2026-09-20, P-E1 / P-E8): an off-page
    /// connector's dashed circle, authored `#00FEA0` 0.50 with a 3.5 / 1.75
    /// dash, is repainted olive 0.35 by the solid placement style and stays
    /// dashed; its solid leg stays solid; a stroke with no style of its own
    /// takes only the placement's paint. The body's pattern joins the pool
    /// after the sheet's own, whose name does not move. A placement style
    /// that names a dash of its own wins over the stroke's.
    #[test]
    fn a_cached_strokes_dash_is_its_own_and_its_colour_its_placements() {
        use pid_parse::style_link::{LineSymbology, StyleHop};
        use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};

        let own = |dash_mm: Vec<f64>| PrimitiveStyle {
            rgb: [0x00, 0xFE, 0xA0],
            width_mm: 0.5,
            dash_mm,
        };
        let body = PidSymbolDefinition {
            reference: PidSymbolDefinitionRef {
                site: 7559,
                sheet: 155,
            },
            layers: vec![8],
            sheet_layers: vec![PidSymbolSheetLayer {
                oid: 8,
                name: Some("Default".to_string()),
                displayed: Some(true),
            }],
            primitives: vec![
                SymbolPrimitive::Circle {
                    center: (0.0, 0.0),
                    radius: 0.005,
                },
                SymbolPrimitive::Line {
                    start: (0.0, 0.0),
                    end: (0.01, 0.0),
                },
                SymbolPrimitive::Line {
                    start: (0.0, 0.0),
                    end: (0.0, 0.01),
                },
            ],
            primitive_layers: vec![8, 8, 8],
            primitive_styles: vec![Some(own(vec![3.5, 1.75])), Some(own(Vec::new())), None],
            dimensions: Vec::new(),
            variables: Vec::new(),
            template: None,
        };
        // The placement's style: olive 0.35, `COLORREF` `0x00BBGGRR`.
        let placement_style = |dash: Option<DashPattern>| ResolvedLineStyle {
            style_id: 25,
            symbology: LineSymbology {
                width_m: 0.000_35,
                colour: 0x0000_8080,
            },
            dash,
            marker: None,
            hop: StyleHop::Direct,
        };
        let sheet_dash = || DashPattern::from_segments_m(&[0.001, 0.001]);

        let mut styles = LineStyleIndex::new();
        styles.insert(("/".to_string(), 1), placement_style(sheet_dash()));
        let mut doc = CadDocument::new();
        let pool = register_dash_linetypes(&mut doc, &styles, std::slice::from_ref(&body));
        assert_eq!(
            pool.get(&vec![1000, 1000]).map(String::as_str),
            Some("PID-DASH-1"),
            "the sheet's own pattern is pooled first and keeps its name"
        );
        assert_eq!(
            pool.get(&vec![3500, 1750]).map(String::as_str),
            Some("PID-DASH-2"),
            "the cached body's pattern joins the pool after it"
        );
        assert!(doc.line_types.contains("PID-DASH-2"));

        let insertion = PidPoint { x: 0.0, y: 0.0 };
        let at = Placement {
            insertion: &insertion,
            rotation: 0.0,
            scale: [1.0, 1.0],
            projection: Projection {
                mm_per_unit: 1000.0,
                band: SheetBand::for_page(None),
            },
        };
        let mut tally = SymbolBodies::default();
        let mut drawn =
            cached_body_entities(Some(&body), &at, &mut tally, &pool).expect("three strokes draw");
        assert_eq!(drawn.len(), 3);
        let coat = |entity: &EntityType| {
            let common = entity.common();
            (common.color, common.line_weight, common.linetype.clone())
        };
        let fresh =
            Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)).common;
        let as_built = fresh.linetype.clone();
        let olive = Color::from_rgb(0x80, 0x80, 0x00);
        assert_eq!(
            coat(&drawn[0]),
            (
                Color::from_rgb(0x00, 0xFE, 0xA0),
                LineWeight::Value(50),
                "PID-DASH-2".to_string()
            ),
            "the undercoat is the body's own colour, width and dash"
        );
        assert_eq!(
            coat(&drawn[1]).2,
            as_built,
            "a solid stroke names no linetype"
        );
        assert_eq!(
            coat(&drawn[2]),
            (fresh.color, fresh.line_weight, as_built.clone()),
            "a stroke with no style of its own is left as built"
        );

        for entity in &mut drawn {
            apply_symbology(entity, &placement_style(None), &pool);
        }
        assert_eq!(
            coat(&drawn[0]),
            (olive, LineWeight::Value(35), "PID-DASH-2".to_string()),
            "a solid placement style repaints colour and width and leaves the dash"
        );
        assert_eq!(
            coat(&drawn[1]),
            (olive, LineWeight::Value(35), as_built.clone())
        );
        assert_eq!(coat(&drawn[2]), (olive, LineWeight::Value(35), as_built));

        for entity in &mut drawn {
            apply_symbology(entity, &placement_style(sheet_dash()), &pool);
        }
        assert!(
            drawn
                .iter()
                .all(|entity| entity.common().linetype == "PID-DASH-1"),
            "a placement style that names a dash of its own wins over the stroke's: {:?}",
            drawn.iter().map(coat).collect::<Vec<_>>()
        );
    }

    /// A symbol's lettering takes its placement's colour and nothing else.
    /// Measured on DWG-0201's level-gauge bubbles: `LG-Magnetic Float
    /// Gauge.sym` and `LT-Magnetostrictive Level Gauge.sym` author their
    /// letters in a `#FF0000` character style, the placements name the
    /// instrument class colour `#008000`, and SmartPlant's screenshot
    /// letters them green. The rule used to be pinned on the library-first
    /// import, the one that still lettered the symbol layer; that import is
    /// retired (plan 2026-09-20-retire-the-library-first-symbol-source), and
    /// the branch is still a real path -- a library body standing in for a
    /// placement the drawing caches nothing drawable for letters through it
    /// -- so it is pinned here: a text on `PID-SYMBOL` is repainted and
    /// takes no line weight, and a text on the sheet's own layer is left
    /// alone, its colour being its character style's.
    #[test]
    fn a_symbols_lettering_takes_its_placements_colour_and_nothing_else() {
        use pid_parse::style_link::{LineSymbology, StyleHop};
        let green = ResolvedLineStyle {
            style_id: 61,
            symbology: LineSymbology {
                width_m: 0.000_18,
                colour: 0x0000_8000,
            },
            dash: None,
            marker: None,
            hop: StyleHop::Direct,
        };
        let letters = |layer: &str| {
            let mut text = Text::new();
            text.value = "LGM".to_string();
            text.height = 2.5;
            text.common.layer = layer.to_string();
            text.common.color = Color::from_rgb(0xFF, 0x00, 0x00);
            EntityType::Text(text)
        };
        let no_dashes = HashMap::new();

        let mut on_symbol = letters(LAYER_SYMBOL);
        apply_symbology(&mut on_symbol, &green, &no_dashes);
        assert_eq!(on_symbol.common().color, Color::from_rgb(0x00, 0x80, 0x00));
        assert_eq!(
            on_symbol.common().line_weight,
            Text::new().common.line_weight,
            "no line weight lands on lettering"
        );
        let EntityType::Text(text) = &on_symbol else {
            panic!("still a text");
        };
        assert_eq!((text.value.as_str(), text.height), ("LGM", 2.5));

        let mut on_sheet = letters(LAYER_TEXT);
        apply_symbology(&mut on_sheet, &green, &no_dashes);
        assert_eq!(
            on_sheet.common().color,
            Color::from_rgb(0xFF, 0x00, 0x00),
            "the sheet's own lettering is coloured by its character style, not here"
        );
    }

    /// A body's arc runs clockwise from its start angle to its end angle,
    /// and lands on the sheet as the DXF arc that covers the same points:
    /// the Manifold's left cap, `270° -> 90°` about a centre on the shell's
    /// left edge, is drawn as the counter-clockwise arc `90° -> 270°`, whose
    /// midpoint bulges left out of the shell -- and a mirrored placement,
    /// which reverses the sense again, keeps the bulge on the outside.
    #[test]
    fn a_bodys_arc_is_drawn_as_the_counter_clockwise_arc_over_the_same_points() {
        use std::f64::consts::{FRAC_PI_2, PI};
        let insertion = PidPoint { x: 0.0, y: 0.0 };
        let placement = |scale_y: f64| Placement {
            insertion: &insertion,
            rotation: 0.0,
            scale: [1.0, scale_y],
            projection: Projection {
                mm_per_unit: 1000.0,
                band: SheetBand::for_page(None),
            },
        };
        let left_cap = SymbolPrimitive::Arc {
            center: (0.04118, 0.08890),
            radius: 0.03559,
            start_angle: 3.0 * FRAC_PI_2,
            end_angle: FRAC_PI_2,
        };
        let midpoint = |arc: &acadrust::entities::Arc| {
            let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
            let angle = arc.start_angle + sweep / 2.0;
            (
                arc.center.x + arc.radius * angle.cos(),
                arc.center.y + arc.radius * angle.sin(),
            )
        };

        let Some(EntityType::Arc(upright)) = shape_primitive(&left_cap, &placement(1.0)) else {
            panic!("the cap did not build");
        };
        assert!((upright.start_angle - FRAC_PI_2).abs() < 1e-12);
        assert!((upright.end_angle - 3.0 * FRAC_PI_2).abs() < 1e-12);
        let (x, y) = midpoint(&upright);
        assert!(
            (x - (41.18 - 35.59)).abs() < 1e-6 && (y - 88.90).abs() < 1e-6,
            "the cap's midpoint is the shell's left edge less the radius: ({x}, {y})"
        );
        let cap = stroke_extent(&EntityType::Arc(upright)).expect("an arc is a stroke");
        assert!(
            (cap.0 - (41.18 - 35.59)).abs() < 1e-6 && (cap.2 - 41.18).abs() < 1e-6,
            "the cap reaches from the shell's edge out to its apex: {cap:?}"
        );

        // Mirrored about x: the same cap on a body flipped upside down still
        // bulges left, with its ends swapped top for bottom.
        let Some(EntityType::Arc(mirrored)) = shape_primitive(&left_cap, &placement(-1.0)) else {
            panic!("the mirrored cap did not build");
        };
        let (x, y) = midpoint(&mirrored);
        assert!(
            (x - (41.18 - 35.59)).abs() < 1e-6 && (y + 88.90).abs() < 1e-6,
            "the mirrored cap still bulges out of the shell: ({x}, {y})"
        );
    }

    #[test]
    fn symbol_polyline_closure_reaches_the_cad_entity() {
        let insertion = PidPoint { x: 0.0, y: 0.0 };
        let placement = Placement {
            insertion: &insertion,
            rotation: 0.0,
            scale: [1.0, 1.0],
            projection: Projection {
                mm_per_unit: 1000.0,
                band: SheetBand::for_page(None),
            },
        };
        let primitive = SymbolPrimitive::Polyline {
            vertices: vec![(0.0, 0.0), (0.01, 0.0), (0.01, 0.01)],
            is_closed: true,
        };
        let Some(EntityType::LwPolyline(polyline)) = shape_primitive(&primitive, &placement) else {
            panic!("symbol polyline did not build");
        };
        assert!(polyline.is_closed);
    }
}

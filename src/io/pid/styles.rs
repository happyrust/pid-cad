// Line work and fills: the style indexes one parse yields, and how a record's style becomes an entity's colour, width and linetype.

use super::*;

/// What [`resolve_styles`] reads off the parsed document for the entity
/// loop: the style indexes, the document linetypes and text styles
/// registered for the dash patterns and typefaces they name, and the
/// published semantic model when the drawing ships one.
pub(super) struct Styles {
    /// The document's own style table is missing or did not walk, so line
    /// work, lettering and fills are on their fallbacks. See
    /// [`ImportSummary::style_tables_failed`].
    pub(super) style_tables_failed: bool,
    /// Width and colour per line style.
    pub(super) styles: LineStyleIndex,
    /// What the drawing calls each style.
    pub(super) style_names: StyleNameIndex,
    /// Character height, colour, alignment and typeface per text record: the
    /// record's own run lettering over its paragraph default.
    pub(super) text_heights: pid_parse::style_link::TextHeightIndex,
    /// Text records whose runs disagree and were lettered in the widest one.
    /// See [`ImportSummary::lettering_flattened`].
    pub(super) lettering_flattened: usize,
    /// Which boundary rings the drawing fills.
    pub(super) fills: pid_parse::style_link::FillIndex,
    /// The document linetype registered for each pooled dash pattern.
    pub(super) dash_linetypes: HashMap<Vec<i64>, String>,
    /// The document text style registered for each typeface.
    pub(super) font_styles: HashMap<String, String>,
    /// The semantic model published beside the drawing, if any.
    pub(super) semantics: Option<PidSemanticIndex>,
}

/// Read the style indexes off the parsed document and register in `doc`
/// what drawing from them needs: the pooled dash linetypes, the text styles,
/// and the APPID the entities' XDATA is filed under.
pub(super) fn resolve_styles(
    parsed: &pid_parse::PidDocument,
    geometry: &NormalizedPidGeometry,
    path: &Path,
    doc: &mut CadDocument,
) -> Styles {
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
    let styles = pid_parse::style_link::line_styles_for_document(parsed);
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
    let style_names = pid_parse::style_link::style_names_for_document(parsed);
    // Which project standards file those names came from. It bounds them: a
    // name means the same thing across two drawings only as far as they were
    // drawn against the same library, and on the reference corpus the two
    // drawings sharing a `.SPP` are exactly the two whose vocabularies agree.
    // Logged rather than drawn -- it is provenance for the layer names above.
    let libraries = pid_parse::style_link::style_libraries_for_document(parsed);
    let sources: std::collections::BTreeSet<&str> =
        libraries.values().map(String::as_str).collect();
    for source in sources {
        log::info!("{}: styles were read from {source}", path.display());
    }
    // Lettering comes from the same table, by two routes. A text record names
    // a paragraph style whose character style is the paragraph's *default*;
    // the record's own character-style runs override it, and where a record
    // has runs they cover every character. So height, colour and typeface
    // come from the run and alignment and line spacing from the paragraph,
    // which only it states (plan 2026-09-24, N-D2; pid-parse's
    // `docs/analysis/2026-08-22-run-beats-paragraph-default.md`). Until then
    // the default was all that was read, and 106 of the corpus's 155 labels
    // lettered in the wrong size or face -- 7 pt line numbers drawn at 9 pt,
    // Arial Narrow drawn as Braggadocio. Records neither route resolves keep
    // the `TEXT_HEIGHT_MM` fallback.
    let text_styles = pid_parse::style_link::text_styles_for_document(parsed);
    let text_heights: pid_parse::style_link::TextHeightIndex = text_styles
        .iter()
        .filter_map(|(key, style)| style.effective().map(|lettering| (key.clone(), lettering)))
        .collect();
    // Labels lettered in more than one character style -- a line number's
    // segments in one and its separators in another, a superscript `3` in
    // `m^3`. A TEXT entity carries one style, so each is drawn in the run that
    // covers most of its characters, and the flattening is said rather than
    // made quietly.
    let lettering_flattened = text_styles
        .values()
        .filter(|style| {
            matches!(
                style.runs,
                pid_parse::style_link::TextRunStatus::Flattened { .. }
            )
        })
        .count();
    if lettering_flattened > 0 {
        log::info!(
            "{}: {lettering_flattened} text record(s) carry character-style runs in more than one lettering; each is lettered in the run covering most of its characters",
            path.display()
        );
    }
    // Which areas the drawing fills. `pid-parse` resolves an `igBoundary2d`
    // ring through its `JStyleOverride` to a `JStyleSimpleFill`; the fill's
    // own colour is not decoded, so a filled ring is drawn in its layer's
    // colour. On the reference corpus these are the solid flow arrowheads on
    // the pipelines -- 5 on DWG-0202 and 10 on the gongyi drawing, all of
    // which used to import as hollow triangles.
    let fills = pid_parse::style_link::fill_styles_for_document(parsed);
    // A line's dash pattern comes from the same style table, one reference
    // further along: a JStyleSimpleLine names a JStyleSimpleDashType, and
    // style_link hands the decoded segments back. Pool the distinct patterns
    // into named document linetypes now, so `apply_symbology` can name each
    // dashed line to one and the renderer dashes it like any other linetype.
    // See pid-parse's `docs/analysis/2026-08-07-jstyle-simple-dash-type-linetype.md`.
    // The bodies the drawing caches state a dash of their own per stroke
    // (plan 2026-09-20, P-E8), pooled into the same table so a symbol's
    // internal dash draws through the same path.
    let dash_linetypes = register_dash_linetypes(doc, &styles, &geometry.symbol_definitions);
    // Same pooling for the typefaces the character styles name, so a label can
    // reference a document text style the way any other text entity does.
    let font_styles = register_text_styles(doc, &text_heights, path);
    // The published semantic model, when the drawing ships one: SmartPlant
    // publishes `<stem>_Data.xml` beside the `.pid`, and pid-parse joins its
    // GraphicOIDs onto the decoded records (two-hop rule, see pid-parse's
    // `docs/analysis/2026-08-07-graphic-oid-is-the-semantic-join.md`). A
    // drawing without one imports exactly as before -- the XML is an
    // enrichment, never a prerequisite.
    let semantics = PidSemanticIndex::load_beside(path, parsed);
    // The DWG writer skips XDATA whose application is not in the APPID
    // table, so without this registration the identities would survive the
    // session and silently vanish on save (see `set_entity_xdata`). Every
    // drawn entity states its `role=`, so the registration is unconditional.
    if !doc.app_ids.contains(PID_SEMANTICS_XDATA_APP) {
        let mut app = acadrust::tables::AppId::new(PID_SEMANTICS_XDATA_APP);
        app.handle = doc.allocate_handle();
        let _ = doc.app_ids.add(app);
    }
    Styles {
        style_tables_failed,
        styles,
        style_names,
        text_heights,
        lettering_flattened,
        fills,
        dash_linetypes,
        font_styles,
        semantics,
    }
}

/// The style table's entry for one normalized entity, if it has one.
///
/// The join is `(stream path, graphic oid)`, which is what `pid-parse` keys
/// the table on. Four families carry a style reference and are in it — lines,
/// points, linestrings, and symbol placements (whose one style covers the
/// placed body) — so text and every kind of evidence simply miss.
pub(super) fn style_for<'a>(
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
pub(super) fn style_name_for<'a>(
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
pub(super) fn discipline_layer(name: &str) -> Option<String> {
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
pub(super) fn fill_for<'a>(
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
pub(super) fn build_fill(
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
pub(super) fn apply_symbology(
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
pub(super) fn dash_key(segments_mm: &[f64]) -> Vec<i64> {
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
pub(super) fn register_dash_linetypes(
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
pub(super) fn build_dash_linetype(name: &str, segments_mm: &[f64]) -> LineType {
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

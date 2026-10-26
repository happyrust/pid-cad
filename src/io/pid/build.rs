// The entity loop: each normalized record of the drawing becomes the entities it draws.

use super::*;

/// What [`build_document_entities`] tallied while filing the drawing's
/// entities: the counts the report and the summary state, and the extent
/// the camera is framed on.
pub(super) struct Built {
    /// Entities handed to the document.
    pub(super) drawn: usize,
    /// Decoded source records those entities came from.
    pub(super) decoded: usize,
    /// Text records whose height did not resolve and kept the fallback.
    pub(super) lettering_on_fallback: usize,
    /// Where the symbol placements' bodies came from.
    pub(super) symbol_bodies: SymbolBodies,
    /// Placements whose entities carry `driving=`; see [`ImportSummary`].
    pub(super) parametric_placements: usize,
    /// Drawn entities per authored sheet layer, keyed by storage path, oid
    /// and name.
    pub(super) sheet_layer_distribution: BTreeMap<(String, u32, Option<String>), usize>,
    /// The sheet layers the drawing draws nothing of, by name: what the view
    /// filter starts with switched off.
    pub(super) sheet_layers_off: BTreeSet<String>,
    /// The extent of the decoded geometry, for framing.
    pub(super) bounds: Bounds,
}

/// Build every entity of the drawing and file it into `doc` under the layer
/// the drawing filed it under, styled from `lookups` and drawn from the
/// bodies the drawing caches or `library` stands in with.
pub(super) fn build_document_entities(
    geometry: &NormalizedPidGeometry,
    lookups: &Styles,
    library: &mut Option<SymbolLibrary>,
    unit: &ImportUnit,
    doc: &mut Document,
    paper_style_scale: f64,
) -> Built {
    let Styles {
        styles,
        style_names,
        text_heights,
        fills,
        dash_linetypes,
        font_styles,
        semantics,
        ..
    } = lookups;
    let projection = Projection::for_geometry(geometry, unit);
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
    // Rings the file stores twice -- identical fill, identical vertices --
    // draw once, the way the reference plots draw them.
    let mut seen_fill_rings: HashSet<u64> = HashSet::new();
    for entity in &geometry.entities {
        // A boundary ring is the one kind whose style decides its shape rather
        // than its colour: filled, it is an area; unfilled, it is an outline
        // the member lines already drew. See `build_fill`.
        let fill = fill_for(fills, entity);
        if let Some(fill) = fill {
            if let Some(key) = fill_dedupe_key(&entity.kind, fill) {
                if !seen_fill_rings.insert(key) {
                    continue;
                }
            }
        }
        // Resolved before building: a point's style decides whether it draws
        // the slash mark SmartPlant shows for a class-coloured point.
        let symbology = style_for(styles, entity);
        // What the drawing calls the style this record draws with, and the
        // working key its line work is built under. A record whose style the
        // drawing does not name keeps `PID-GEOMETRY`, and that absence is
        // itself a reading: the librarian lists what came from the project
        // library, so an unnamed style is one drawn in this file. The name
        // goes into XDATA as `style=` whether or not it becomes a key -- an
        // appearance name such as `Normal` is still what the drawing said.
        let style_name = style_name_for(style_names, entity, symbology);
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
                    dash_linetypes,
                    paper_style_scale,
                    font_styles,
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
        let text_style = height_for(text_heights, entity);
        let height_mm = text_style.map(|h| projection.mm(h.height_m));
        // The same character style states the colour, so it comes off the
        // join already made rather than a second one.
        let text_rgb = text_style.and_then(pid_parse::style_link::ResolvedTextHeight::rgb);
        // Alignment rides the same join but comes off the paragraph rather
        // than the character style, since it belongs to the run.
        let text_alignment = text_style.and_then(|style| style.alignment);
        // ...and the record's own trailer byte beats both: it is the anchor
        // the label was placed at, where the paragraph is only the default
        // it overrides. See `apply_text_anchor`.
        let anchor_code = match &entity.kind {
            PidGraphicKind::Text { anchor_code, .. } => Some(*anchor_code),
            _ => None,
        };
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
        let measures = PlacementMeasures::of(&entity.kind, geometry, projection, &built);
        if measures.as_ref().is_some_and(|m| m.driving.is_some()) {
            parametric_placements += 1;
        }
        for mut one in built {
            if let Some(style) = symbology {
                apply_symbology(&mut one, style, dash_linetypes, paper_style_scale);
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
                    // `ConsistencyChecks` is the vendor's review-overlay
                    // layer: displayed on screen, never plotted — the
                    // reference prints show no mark at any junction.
                    ensure_layer(
                        doc,
                        name,
                        Color::WHITE,
                        !hidden,
                        name != "ConsistencyChecks",
                    );
                }
                // The label of a placement on a hidden sheet layer is not
                // moved either: its layer is already off, and the view filter
                // darkens it by its `sheet_layer=` like the body.
                Some(_) => {
                    ensure_taxonomy_layer(doc, &one.common().layer.clone());
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
                    ensure_taxonomy_layer(doc, &one.common().layer.clone());
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
    Built {
        drawn,
        decoded,
        lettering_on_fallback,
        symbol_bodies,
        parametric_placements,
        sheet_layer_distribution,
        sheet_layers_off,
        bounds,
    }
}

/// `line_work` is the layer the sheet's own lines, arcs and rings go on:
/// `PID-GEOMETRY`, or the discipline layer when the drawing names the style
/// they draw with (see [`discipline_layer`]). It is decided by the caller and
/// passed in rather than patched afterwards, so there is one place that
/// answers "which layer is this line on".
///
/// `dash_linetypes` is the pool [`register_dash_linetypes`] filled, for the
/// dash a symbol body's own stroke style names (see [`paint_symbol_stroke`]).
#[allow(clippy::too_many_arguments)]
pub(super) fn build_entities(
    kind: &PidGraphicKind,
    bodies: BodySources<'_>,
    projection: Projection,
    symbology: Option<&ResolvedLineStyle>,
    line_work: &str,
    symbol_bodies: &mut SymbolBodies,
    dash_linetypes: &HashMap<Vec<i64>, String>,
    paper_style_scale: f64,
    font_styles: &HashMap<String, String>,
) -> Vec<EntityType> {
    match kind {
        PidGraphicKind::Line { start, end } => {
            let mut line = Line::from_points(projection.point(start), projection.point(end));
            line.common.layer = line_work.to_string();
            let mut built = vec![EntityType::Line(line)];
            if let Some(dash) = symbology.and_then(|style| style.complex_dash) {
                stamp_complex_dash(
                    &[
                        Vector2::new(projection.mm(start.x), projection.mm(start.y)),
                        Vector2::new(projection.mm(end.x), projection.mm(end.y)),
                    ],
                    false,
                    &dash,
                    projection,
                    line_work,
                    paper_style_scale,
                    &mut built,
                );
            }
            built
        }
        PidGraphicKind::Polyline { points, closed } => {
            if points.len() < 2 {
                return Vec::new();
            }
            let vertices: Vec<Vector2> = points
                .iter()
                .map(|p| Vector2::new(projection.mm(p.x), projection.mm(p.y)))
                .collect();
            let mut polyline = LwPolyline::from_points(vertices.clone());
            polyline.is_closed = *closed;
            polyline.common.layer = line_work.to_string();
            let mut built = vec![EntityType::LwPolyline(polyline)];
            if let Some(dash) = symbology.and_then(|style| style.complex_dash) {
                stamp_complex_dash(
                    &vertices,
                    *closed,
                    &dash,
                    projection,
                    line_work,
                    paper_style_scale,
                    &mut built,
                );
            }
            built
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
            clockwise: _,
        } => {
            let mut arc = codec::entities::Arc::new();
            arc.center = projection.point(center);
            arc.radius = projection.mm(*radius);
            // Radians on both sides -- see the angle-unit note by the layer
            // constants. The record draws its minor arc: the linked border
            // templates letter their logo with ~20-130deg sweeps and the
            // sheet's own valve details read the same way -- the other
            // reading is always the complement, which is what the old
            // clockwise swap produced as near-full rings.
            arc.start_angle = *start_angle;
            arc.end_angle = *end_angle;
            let sweep = (arc.end_angle - arc.start_angle).rem_euclid(std::f64::consts::TAU);
            if sweep > std::f64::consts::PI + 1e-9 {
                std::mem::swap(&mut arc.start_angle, &mut arc.end_angle);
            }
            arc.common.layer = line_work.to_string();
            vec![EntityType::Arc(arc)]
        }
        PidGraphicKind::Text {
            insertion,
            value,
            height,
            rotation,
            ..
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
            // The authored insertion is the top of the text's line box, not
            // its baseline: measured over the corpus, the record's anchor
            // sits within a millimetre of the reference glyphs' ascender
            // line, so left-on-baseline lettering floated one line box
            // (~2-5 mm) above where the drawing puts the run. Top anchors
            // the ink top at the insertion instead, which lands the run
            // where it was authored (still inside the line box, ~1 mm high).
            // Like `apply_text_alignment`, the anchor moves to
            // `alignment_point` whenever the pair is not left-on-baseline,
            // so it is seeded from the insertion in the same breath.
            text.vertical_alignment = TextVerticalAlignment::Top;
            text.alignment_point = Some(text.insertion_point);
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
            let insertion_mm = (projection.mm(insertion.x), projection.mm(insertion.y));

            // A label placement whose cached body is nothing but the cross
            // it is anchored by draws nothing at all: not the cross, not the
            // library's catalogue preview in its place, not the marker dot
            // (plan 2026-10-10-a-label-placement-keeps-its-anchor-cross-off-
            // the-sheet). The file displays the cross; SmartPlant's screen
            // does not, while it does draw the one other hairline body of
            // the corpus (` Line2.sym`), so both words are read: the path
            // that files the symbol as a label and the width that makes the
            // body a construction mark. See `is_label_symbol` and
            // `is_anchor_cross_only` for the evidence. The two half-matches
            // are drawn as the file states and said so in the log, since a
            // later corpus may show which of the two words SmartPlant goes
            // by.
            let is_label = symbol_path.is_some_and(is_label_symbol);
            let anchor_only = cached.is_some_and(is_anchor_cross_only);
            let (mut built, name_anchor) = if is_label && anchor_only {
                symbol_bodies.label_anchors += 1;
                // The name is still lettered beside where the cross would
                // have been -- next to the label's own lettering -- rather
                // than at the insertion point a hundred millimetres away.
                let cross: Vec<EntityType> = cached
                    .into_iter()
                    .flat_map(PidSymbolDefinition::visible_primitives)
                    .filter_map(|primitive| {
                        shape_primitive(primitive, None, &placement, font_styles)
                    })
                    .collect();
                (Vec::new(), symbol_label_anchor(&cross, insertion_mm))
            } else {
                if is_label && cached.is_some() {
                    log::info!(
                        "label placement {:?} caches a body of more than its anchor cross; drawn as the file states",
                        symbol_path.unwrap_or_default()
                    );
                } else if anchor_only {
                    log::info!(
                        "placement {:?} outside the label folders caches only hairline lines; drawn as the file states",
                        symbol_path.unwrap_or_default()
                    );
                }
                let body = cached_body_entities(
                    cached,
                    &placement,
                    symbol_bodies,
                    dash_linetypes,
                    paper_style_scale,
                    font_styles,
                )
                .or_else(|| {
                    library_body_entities(
                        library,
                        symbol_path,
                        &placement,
                        symbol_bodies,
                        dash_linetypes,
                        paper_style_scale,
                        font_styles,
                    )
                });

                // Only fall back to the marker when the body is genuinely
                // unavailable. A symbol that resolved to real geometry should
                // not also carry a dot -- that reads as a second object.
                let built = match body {
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
                let name_anchor = symbol_label_anchor(&built, insertion_mm);
                (built, name_anchor)
            };

            if let Some(name) = symbol_path.and_then(symbol_name) {
                let mut label = Text::new();
                label.value = name;
                label.height = SYMBOL_LABEL_HEIGHT_MM;
                label.insertion_point = name_anchor;
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
            // Drawn at the size the group states, times `paper_style_scale`
            // (1 unless the caller asked otherwise). SmartPlant's own screen
            // draws the glyph larger by a view-wide scale on style-declared
            // sizes; that scale is not in the file, so the default import
            // leaves the glyph at the stated size. See pid-parse
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
                point_marker_offset(origin, (x, y), projection, paper_style_scale)
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

/// One end of a point-marker stroke, in sheet millimetres.
///
/// The glyph's coordinates are metres relative to the point. `paper_style_scale`
/// multiplies that offset and nothing else: the point's own location stays
/// where the file put it.
pub(super) fn point_marker_offset(
    origin: Vector3,
    xy: (f64, f64),
    projection: Projection,
    paper_style_scale: f64,
) -> Vector3 {
    Vector3::new(
        origin.x + projection.mm(xy.0 * paper_style_scale),
        origin.y + projection.mm(xy.1 * paper_style_scale),
        0.0,
    )
}

/// The tick marks a complex dash stamps along a line, drawn as separate
/// short `Line` entities on the line's own layer.
///
/// The layout is the one the reference plots evidence on the W drawing's
/// `Hydraulic` pipes (see [`pid_parse::style_link::ComplexDash`]): each
/// straight segment is partitioned into an integral number of cells of the
/// dash's declared advance — `round(len / period)` of them, so the cells
/// stretch to fit — and the glyph stamps once at every cell midpoint, in the
/// segment's local frame (+x along the segment, +y the left normal). A
/// segment shorter than half a cell stamps nothing.
///
/// `paper_style_scale` multiplies the glyph and the period together, the
/// same treatment it gives the declared segment lengths of a plain dash.
fn stamp_complex_dash(
    vertices_mm: &[Vector2],
    closed: bool,
    dash: &ComplexDash,
    projection: Projection,
    line_work: &str,
    paper_style_scale: f64,
    out: &mut Vec<EntityType>,
) {
    let period_mm = dash.period_mm() * paper_style_scale;
    if !dash.draws() || period_mm <= 0.0 {
        return;
    }
    // The glyph's coordinates are in the file's own units — the same
    // conversion the line's vertices took — times the display multiple,
    // the treatment `point_marker_offset` gives a point symbol's strokes.
    let glyph_mm = |xy: (f64, f64)| {
        (
            projection.mm(xy.0 * paper_style_scale),
            projection.mm(xy.1 * paper_style_scale),
        )
    };
    let mut stamp_segment = |a: Vector2, b: Vector2| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = dx.hypot(dy);
        let cells = (len / period_mm).round() as usize;
        // The glyph's frame is the segment's undirected angle: on the W
        // drawing's backward-running pipes the mark draws exactly as on the
        // forward ones, so a direction pointing "left" is flipped before the
        // frame is built. Stamp positions still walk a->b along the run.
        let (t_x, t_y) = (dx / len, dy / len);
        let (dir_x, dir_y) = if t_x < 0.0 || (t_x == 0.0 && t_y < 0.0) {
            (-t_x, -t_y)
        } else {
            (t_x, t_y)
        };
        let (norm_x, norm_y) = (-dir_y, dir_x);
        // A segment too short for two fitted cells still draws one mark —
        // at its end vertex, measured on the W stubs (3.8-3.9mm stubs all
        // stamp at the run's terminal vertex, not at a cell midpoint).
        let count = cells.max(1);
        let cell = len / count as f64;
        for k in 0..count {
            let t = if cells <= 1 {
                len
            } else {
                (k as f64 + 0.5) * cell
            };
            let (cx, cy) = (a.x + t_x * t, a.y + t_y * t);
            for stroke in dash.strokes() {
                if stroke.is_degenerate() {
                    continue;
                }
                let (sx, sy) = glyph_mm(stroke.start);
                let (ex, ey) = glyph_mm(stroke.end);
                let mut tick = Line::from_points(
                    Vector3::new(
                        cx + sx * dir_x + sy * norm_x,
                        cy + sx * dir_y + sy * norm_y,
                        0.0,
                    ),
                    Vector3::new(
                        cx + ex * dir_x + ey * norm_x,
                        cy + ex * dir_y + ey * norm_y,
                        0.0,
                    ),
                );
                tick.common.layer = line_work.to_string();
                out.push(EntityType::Line(tick));
            }
        }
    };
    for pair in vertices_mm.windows(2) {
        stamp_segment(pair[0], pair[1]);
    }
    if closed && vertices_mm.len() > 2 {
        stamp_segment(*vertices_mm.last().unwrap(), vertices_mm[0]);
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
pub(super) fn build_inferred(kind: &PidGraphicKind, projection: Projection) -> Vec<EntityType> {
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
pub(super) fn on_sheet_pair(start: &PidPoint, end: &PidPoint, projection: Projection) -> bool {
    let (start_x, start_y) = (projection.mm(start.x), projection.mm(start.y));
    let (end_x, end_y) = (projection.mm(end.x), projection.mm(end.y));
    [start_x, start_y, end_x, end_y]
        .iter()
        .all(|value| projection.band.holds(*value))
        && (start_x.hypot(start_y) > CONNECTIVITY_MIN_MM)
        && (end_x.hypot(end_y) > CONNECTIVITY_MIN_MM)
        && ((end_x - start_x).hypot(end_y - start_y) > CONNECTIVITY_MIN_MM)
}

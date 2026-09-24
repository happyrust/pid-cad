// Symbol placements: the body the drawing caches, the library body behind it, and the strokes they draw.

use super::*;

/// How the symbol placements of one import were drawn, tallied as
/// [`build_entities`] draws them; the last three numbers of
/// [`ImportSummary`] and one line of [`report_import`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SymbolBodies {
    /// Placements drawn from the drawing's own definition cache.
    pub(super) cache: usize,
    /// Placements drawn from the library.
    pub(super) library: usize,
    /// Placements neither body reached, drawn as the marker dot.
    pub(super) markers: usize,
    /// Cached strokes left out for sitting on a switched-off symbol layer.
    pub(super) hidden_strokes_skipped: usize,
}

/// Where a symbol placement's body can come from: the body the drawing
/// caches for this placement, asked first, and the library it falls back
/// to. Handed over per entity, since the cached body is the placement's own.
pub(super) struct BodySources<'a> {
    /// The drawing's own definition for the placement, when it has one.
    pub(super) cached: Option<&'a PidSymbolDefinition>,
    /// The library found beside the drawing, when one was.
    pub(super) library: Option<&'a mut SymbolLibrary>,
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
pub(super) fn cached_body_entities(
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
pub(super) fn library_body_entities(
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
pub(super) struct Placement<'a> {
    pub(super) insertion: &'a PidPoint,
    pub(super) rotation: f64,
    pub(super) scale: [f64; 2],
    pub(super) projection: Projection,
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
    pub(super) fn apply(&self, x: f64, y: f64) -> Vector3 {
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
    pub(super) fn scale_radius(&self, radius: f64) -> f64 {
        self.projection
            .mm(radius * (self.scale[0].abs() + self.scale[1].abs()) / 2.0)
    }

    /// Whether the placement flips handedness, which reverses the direction
    /// an arc sweeps.
    pub(super) fn mirrored(&self) -> bool {
        self.scale[1] < 0.0
    }
}

/// Draw one primitive of a symbol body at its placement, in the colour,
/// width and dash the symbol states for it -- the one route both bodies
/// take, the drawing's cached one and the library's.
pub(super) fn place_primitive(
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
pub(super) fn paint_symbol_stroke(
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
pub(super) fn shape_primitive(
    primitive: &SymbolPrimitive,
    at: &Placement<'_>,
) -> Option<EntityType> {
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
pub(super) fn carries_a_label(text: &str) -> bool {
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
pub(super) fn discover_symbol_library(drawing: &Path) -> Option<SymbolLibrary> {
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
pub(super) fn symbol_roots_in(dir: &Path) -> Vec<PathBuf> {
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
pub(super) fn symbol_name(path: &str) -> Option<String> {
    let file = path.rsplit(['\\', '/']).next()?.trim();
    let stem = match file.rfind('.') {
        Some(dot) if file[dot..].eq_ignore_ascii_case(".sym") => &file[..dot],
        _ => file,
    };
    let stem = stem.trim();
    (!stem.is_empty()).then(|| stem.to_owned())
}

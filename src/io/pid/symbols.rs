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
    /// Cached strokes left out for sitting on a switched-off symbol layer,
    /// over the placements whose body was drawn.
    pub(super) hidden_strokes_skipped: usize,
    /// Label placements whose cached body is nothing but the hairline cross
    /// the label is anchored by, left undrawn as SmartPlant's screen leaves
    /// them -- see [`is_label_symbol`] and [`is_anchor_cross_only`]. Not
    /// among [`Self::cache`], and their switched-off lettering not among
    /// [`Self::hidden_strokes_skipped`]: the body was not consulted to draw.
    pub(super) label_anchors: usize,
}

/// Narrower than any pen ISO 128 names -- the thinnest is 0.13 mm -- so a
/// width under it is a hairline: a construction mark, not drafting.
///
/// The corpus states exactly one such width, 0.01 mm, and only in symbol
/// bodies: the anchor crosses of the label symbols (every one of them) and
/// the line and tick of ` Line2.sym`, whose line SmartPlant's screen does
/// draw. The threshold therefore says nothing on its own; it is one of the
/// two words [`is_anchor_cross_only`] and [`is_label_symbol`] read together.
pub(super) const UNDER_ISO_WIDTH_MM: f64 = 0.1;

/// Label symbols filed outside a `Labels` folder.
///
/// SmartPlant's reference data keeps its labels under `Labels` folders --
/// `Design\Annotation\Labels\`, `Piping\Labels - Piping Segments\`,
/// `Equipment\Labels - Equipment\` -- and that folder is what
/// [`is_label_symbol`] reads. The one exception on this plant is its
/// drawing-description label, filed as `Design\Drawing Description.sym`
/// beside the sheet templates: its `.sym` and its cached bodies are cut from
/// the same template as `Item Note & Label.sym` (the hairline cross on the
/// displayed `Default` layer, the lettering and an underline on the
/// switched-off `Label` layer), so it is named here rather than drawn as a
/// cross. Matched on the file name, case-insensitively.
pub(super) const LABEL_SYMBOLS_FILED_ELSEWHERE: [&str; 1] = ["Drawing Description.sym"];

/// Whether a placement's library path names a label symbol.
///
/// A directory segment of the path called `Labels`, or starting `Labels - `
/// (`Labels - Piping Segments`, `Labels - Equipment`, `Labels - General
/// Instrument`), is SmartPlant's own filing of label symbols; a file on
/// [`LABEL_SYMBOLS_FILED_ELSEWHERE`] is one filed outside them. Directory
/// segments only: a symbol whose file name happens to carry the word is not
/// a label. The path is the one word the file has on the matter -- the
/// placement record's sub-type and the `JSite`'s properties say nothing a
/// label placement does not share with a valve's.
///
/// What a label placement draws on SmartPlant's screen is its lettering,
/// and that lettering is the sheet's own text records (DWG-0201's 「说 明」
/// is a `/Sheet6` text on the `Labels` layer); the symbol body the drawing
/// caches for it is the cross the label is placed by, which the screen
/// leaves out -- see [`is_anchor_cross_only`].
pub(super) fn is_label_symbol(path: &str) -> bool {
    let segments: Vec<&str> = path
        .split(['\\', '/'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    let Some((file, directories)) = segments.split_last() else {
        return false;
    };
    directories
        .iter()
        .any(|directory| *directory == "Labels" || directory.starts_with("Labels - "))
        || LABEL_SYMBOLS_FILED_ELSEWHERE
            .iter()
            .any(|name| file.eq_ignore_ascii_case(name))
}

/// Whether a cached body's displayed strokes are nothing but hairline lines
/// -- the cross a label is anchored by, and no drafting.
///
/// Every label placement of the corpus caches such a body (plan
/// 2026-10-10-a-label-placement-keeps-its-anchor-cross-off-the-sheet, §1):
/// two 0.01 mm lines 2.54 mm across centred on the symbol's origin
/// (`Item Note & Label`, `Drawing Description`), or three making a 1.27 mm
/// cross with a tick (`Remarks`), on the body's displayed `Default` layer,
/// with the template lettering (`NULL`, 「说 明」) and any underline on the
/// `Label` layer the file switches off. The strokes name the one style of
/// their cache that states a hairline width and the by-layer colour
/// `0x80000000` -- the only by-layer colour among the corpus's 69 overrides
/// -- and SmartPlant's screenshot of DWG-0201 shows none of the four at
/// 2.12 px/mm, where a 2.54 mm cross would be five pixels across. The
/// drawing's `.sym` keeps the cross in the first layer manager's `Default`
/// and its catalogue preview (a boxed `XXX`, two leader strokes) on a second
/// manager's sheet the drawing never caches, so the library body is no
/// stand-in either: a label placement whose body is only its cross draws
/// nothing -- no body, no library fallback, no marker dot.
///
/// A body with no displayed stroke is not a cross (it is a placement with
/// nothing to draw, which the marker exists for), and a stroke with no style
/// of its own cannot be called a hairline. The width is read alone because
/// the colour is not visible here: pid-parse resolves the by-layer word to
/// the base's colour, so a stroke's `rgb` cannot say which it was.
pub(super) fn is_anchor_cross_only(body: &PidSymbolDefinition) -> bool {
    let mut strokes = body.visible_strokes().peekable();
    strokes.peek().is_some()
        && strokes.all(|(primitive, style)| {
            matches!(primitive, SymbolPrimitive::Line { .. })
                && style.is_some_and(|style| style.width_mm < UNDER_ISO_WIDTH_MM)
        })
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
    paper_style_scale: f64,
    font_styles: &HashMap<String, String>,
) -> Option<Vec<EntityType>> {
    let body = body?;
    let strokes: Vec<(
        &SymbolPrimitive,
        Option<&PrimitiveStyle>,
        Option<&SymbolLettering>,
    )> = body.visible_styled().collect();
    symbol_bodies.hidden_strokes_skipped += body.primitives.len() - strokes.len();
    let entities: Vec<EntityType> = strokes
        .into_iter()
        .flat_map(|(primitive, style, lettering)| {
            place_primitive(
                primitive,
                style,
                lettering,
                at,
                dash_linetypes,
                paper_style_scale,
                font_styles,
            )
        })
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
    paper_style_scale: f64,
    font_styles: &HashMap<String, String>,
) -> Option<Vec<EntityType>> {
    let body = library
        .zip(symbol_path)
        .and_then(|(library, path)| library.resolve(path))?;
    let strokes = body.displayed_primitives();
    symbol_bodies.hidden_strokes_skipped += body.primitives.len() - strokes.len();
    let entities: Vec<EntityType> = strokes
        .into_iter()
        .flat_map(|styled| {
            place_primitive(
                &styled.primitive,
                styled.style.as_ref(),
                styled.lettering.as_ref(),
                at,
                dash_linetypes,
                paper_style_scale,
                font_styles,
            )
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
    lettering: Option<&SymbolLettering>,
    at: &Placement<'_>,
    dash_linetypes: &HashMap<Vec<i64>, String>,
    paper_style_scale: f64,
    font_styles: &HashMap<String, String>,
) -> Vec<EntityType> {
    let mut entities = shape_primitives(primitive, lettering, at, font_styles);
    for entity in &mut entities {
        paint_symbol_stroke(entity, style, dash_linetypes, paper_style_scale);
    }
    entities
}

/// The entities one primitive of a symbol body draws at its placement —
/// usually one; a boundary draws a solid hatch per member shape (see
/// [`shape_boundary`]).
pub(super) fn shape_primitives(
    primitive: &SymbolPrimitive,
    lettering: Option<&SymbolLettering>,
    at: &Placement<'_>,
    font_styles: &HashMap<String, String>,
) -> Vec<EntityType> {
    if let SymbolPrimitive::Boundary { members, fill } = primitive {
        return shape_boundary(members, *fill, at).unwrap_or_default();
    }
    shape_primitive(primitive, lettering, at, font_styles)
        .into_iter()
        .collect()
}

/// A boundary's member shapes as one solid hatch each — SmartPlant fills
/// every disc of a plug valve's concentric circles rather than an even-odd
/// compound of them, so each member becomes its own `EXTERNAL`-pathed
/// hatch. Members that are not self-closing (lines) group into one ring's
/// hatch, the same way the chain reads.
fn shape_boundary(
    members: &[SymbolPrimitive],
    fill: Option<pid_parse::symbol_library::PrimitiveFill>,
    at: &Placement<'_>,
) -> Option<Vec<EntityType>> {
    let mut hatches: Vec<EntityType> = Vec::new();
    let mut lines = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    for member in members {
        match member {
            SymbolPrimitive::Circle { center, radius } => {
                let radius = at.scale_radius(*radius);
                if !radius.is_finite() || radius <= 0.0 {
                    continue;
                }
                let placed = at.apply(center.0, center.1);
                let mut hatch = Hatch::new();
                let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
                path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
                    center: Vector2::new(placed.x, placed.y),
                    radius,
                    start_angle: 0.0,
                    end_angle: std::f64::consts::TAU,
                    counter_clockwise: !at.mirrored(),
                }));
                hatch.paths.push(path);
                hatch.common.layer = LAYER_FILL.to_string();
                if let Some([r, g, b]) = fill.and_then(|fill| fill.rgb) {
                    hatch.common.color = foreground_color(r, g, b);
                }
                hatches.push(EntityType::Hatch(Box::new(hatch)));
            }
            SymbolPrimitive::Line { start, end } => {
                let a = at.apply(start.0, start.1);
                let b = at.apply(end.0, end.1);
                lines.edges.push(BoundaryEdge::Line(LineEdge {
                    start: Vector2::new(a.x, a.y),
                    end: Vector2::new(b.x, b.y),
                }));
            }
            _ => {}
        }
    }
    if !lines.edges.is_empty() {
        let mut hatch = Hatch::new();
        hatch.paths.push(lines);
        hatch.common.layer = LAYER_FILL.to_string();
        if let Some([r, g, b]) = fill.and_then(|fill| fill.rgb) {
            hatch.common.color = foreground_color(r, g, b);
        }
        hatches.push(EntityType::Hatch(Box::new(hatch)));
    }
    (!hatches.is_empty()).then_some(hatches)
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
    paper_style_scale: f64,
) {
    let Some(style) = style else {
        return;
    };
    let common = entity.common_mut();
    let [r, g, b] = style.rgb;
    common.color = foreground_color(r, g, b);
    // Same ladder and the same guard as the drawing's own line work: DXF
    // stores hundredths of a millimetre, and a width outside the range the
    // format can state is left at the layer default rather than clamped into
    // a width the symbol did not ask for. The declared width runs through
    // the same three-pen table [`plot_pen_mm`] as the sheet line work.
    let hundredths = (plot_pen_mm(style.width_mm * paper_style_scale) * 100.0).round();
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

/// Letter a symbol's own text the way its record states: the hot point its
/// trailer's anchor byte decodes to, the colour its character style names,
/// and the typeface its style resolved to when the drawing pooled it.
///
/// The anchor decode is the same nibble scheme [`apply_text_anchor`] reads
/// off the sheet's records — `(code >> 4) % 3` horizontal, `(code & 0x0F) %
/// 3` vertical — applied directly since the `apply_text_*` helpers are
/// scoped to the sheet's own [`LAYER_TEXT`]. The colour is the undercoat
/// only: where the placement names a style of its own, `apply_symbology`
/// repaints it over this afterwards, which is measured behaviour -- LG and
/// LT author their bubble letters `#FF0000` here and the placement letters
/// them `#008000`.
fn letter_label(
    label: &mut Text,
    lettering: &SymbolLettering,
    font_styles: &HashMap<String, String>,
) {
    use crate::entities::text::sync_text_alignment_point;

    label.horizontal_alignment = match (lettering.anchor_code >> 4) % 3 {
        1 => TextHorizontalAlignment::Center,
        2 => TextHorizontalAlignment::Right,
        _ => TextHorizontalAlignment::Left,
    };
    label.vertical_alignment = match (lettering.anchor_code & 0x0F) % 3 {
        0 => TextVerticalAlignment::Bottom,
        1 => TextVerticalAlignment::Top,
        _ => TextVerticalAlignment::Middle,
    };
    sync_text_alignment_point(label);
    if let Some([r, g, b]) = lettering.rgb {
        label.common.color = foreground_color(r, g, b);
    }
    if let Some(name) = lettering
        .font_name
        .as_deref()
        .and_then(|font| font_styles.get(font))
    {
        label.style = name.clone();
    }
}

/// Where one primitive of a symbol body lands at its placement.
pub(super) fn shape_primitive(
    primitive: &SymbolPrimitive,
    lettering: Option<&SymbolLettering>,
    at: &Placement<'_>,
    font_styles: &HashMap<String, String>,
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
            clockwise: _,
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
            let mut arc = codec::entities::Arc::new();
            arc.center = at.apply(center.0, center.1);
            arc.radius = radius;
            // Radians on both sides -- see the angle-unit note by the layer
            // constants. The sum above already proves the unit: `at.rotation`
            // is what `Placement::apply` feeds to `sin_cos`, so the library's
            // angles have to be radians for the addition to mean anything.
            arc.start_angle = start_angle;
            arc.end_angle = end_angle;
            // Every measured `igArc2d` draws its minor arc: the CV valve's
            // diaphragm reads start<end in the file yet the reference draws
            // a 15deg cap, not the 345deg complement the clockwise reading
            // builds. Only an exact half-turn needs the clockwise rule --
            // the Manifold caps sit there, and it still wins on ties. The
            // comparison needs epsilon room: `Parametric V Drum`'s caps read
            // `pi -> 0` where the file's pi is one ulp above `f64::consts::PI`,
            // and an exact `>` test flips that half-turn into its complement.
            let sweep = (arc.end_angle - arc.start_angle).rem_euclid(std::f64::consts::TAU);
            if sweep > std::f64::consts::PI + 1e-9 {
                std::mem::swap(&mut arc.start_angle, &mut arc.end_angle);
            }
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
        SymbolPrimitive::Boundary { members, fill } => {
            // One entity per caller that asks for it; the full one-hatch-
            // per-member fan-out lives in `shape_boundary` and comes out
            // through `shape_primitives`.
            shape_boundary(members, *fill, at).and_then(|mut hatches| {
                if hatches.is_empty() {
                    None
                } else {
                    Some(hatches.remove(0))
                }
            })
        }
        SymbolPrimitive::Text { text, at: origin } => {
            let value = text.trim();
            if !carries_a_label(value) {
                return None;
            }
            let mut label = Text::new();
            label.value = value.to_string();
            label.insertion_point = at.apply(origin.0, origin.1);
            // The height the record states wins -- a `.sym` can scale its
            // direction pair to the letter height the way the linked
            // templates do -- then the character style's, then the same ISO
            // 3098 fallback the sheet's own text gets, scaled with the
            // placement so a half-size symbol does not carry full-size
            // lettering.
            label.height = lettering
                .and_then(|lettering| {
                    (lettering.letter_height_m > 0.0)
                        .then_some(lettering.letter_height_m)
                        .or(lettering.height_m)
                })
                .map(|height_m| at.scale_radius(height_m))
                .unwrap_or_else(|| at.scale_radius(TEXT_HEIGHT_MM / at.projection.mm_per_unit));
            // Radians on both sides -- see the angle-unit note by the layer
            // constants. The record's own direction composes with the
            // placement's rotation, and mirrors with it the way an arc's
            // angles do. A mirrored placement turns lettering backwards,
            // and SmartPlant keeps it readable: the emitted rotation is
            // normalized into [-pi/2, pi/2) -- the ESD interlock's label
            // sits at a pi-rotation mirror and reads upright in the
            // reference -- and the anchor flips the way the dropped mirror
            // would have flipped it.
            let own_rotation = lettering.map_or(0.0, |lettering| lettering.rotation_rad);
            let mut h_flip = false;
            let mut v_flip = false;
            label.rotation = if at.mirrored() {
                let raw = at.rotation - own_rotation;
                let emit = (raw + std::f64::consts::FRAC_PI_2).rem_euclid(std::f64::consts::PI)
                    - std::f64::consts::FRAC_PI_2;
                let delta = emit - at.rotation;
                h_flip = delta.cos() < -0.5;
                v_flip = delta.cos() > 0.5;
                emit
            } else {
                at.rotation + own_rotation
            };
            label.common.layer = LAYER_SYMBOL.to_string();
            if let Some(lettering) = lettering {
                letter_label(&mut label, lettering, font_styles);
                if h_flip {
                    label.horizontal_alignment = match label.horizontal_alignment {
                        TextHorizontalAlignment::Left => TextHorizontalAlignment::Right,
                        TextHorizontalAlignment::Right => TextHorizontalAlignment::Left,
                        other => other,
                    };
                }
                if v_flip {
                    label.vertical_alignment = match label.vertical_alignment {
                        TextVerticalAlignment::Top => TextVerticalAlignment::Bottom,
                        TextVerticalAlignment::Bottom => TextVerticalAlignment::Top,
                        other => other,
                    };
                }
                if h_flip || v_flip {
                    crate::entities::text::sync_text_alignment_point(&mut label);
                }
            }
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

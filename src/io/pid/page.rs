// The page: its border, the projection from sheet units to millimetres, and the extents that frame the view.

use super::*;

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
pub(super) fn draw_page_border(doc: &mut CadDocument, page_mm: Option<(f64, f64)>) {
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
pub(super) struct Projection {
    pub(super) mm_per_unit: f64,
    pub(super) band: SheetBand,
}

impl Projection {
    pub(super) fn for_geometry(geometry: &NormalizedPidGeometry, unit: &ImportUnit) -> Self {
        Self {
            mm_per_unit: unit.mm_per_unit(),
            band: SheetBand::for_page(geometry.page_dimensions_mm),
        }
    }

    /// A source coordinate in millimetres.
    pub(super) fn mm(self, value: f64) -> f64 {
        value * self.mm_per_unit
    }

    /// A source point in millimetres, flat on the sheet.
    pub(super) fn point(self, point: &PidPoint) -> Vector3 {
        Vector3::new(self.mm(point.x), self.mm(point.y), 0.0)
    }
}

/// Millimetres in one of the units `pid-parse` can state.
///
/// The unit arrives as a label rather than a factor. The metre is the only
/// one a decoded page frame produces; the millimetre is here because it is
/// the identity and costs nothing to be right about. Any other label is a
/// unit this importer has not been shown, and guessing at its factor would be
/// worse than falling back and saying so.
pub(super) fn millimetres_in(units: &PidDrawingUnits) -> Option<f64> {
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
pub(super) struct SheetBand {
    pub(super) min: f64,
    pub(super) max: f64,
}

impl SheetBand {
    pub(super) fn for_page(page_mm: Option<(f64, f64)>) -> Self {
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
    pub(super) fn holds(self, value: f64) -> bool {
        value.is_finite() && (self.min..=self.max).contains(&value)
    }
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
pub(super) fn frame_drawing(doc: &mut CadDocument, bounds: &Bounds, page_mm: Option<(f64, f64)>) {
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

pub(super) struct Bounds {
    pub(super) min_x: f64,
    pub(super) min_y: f64,
    pub(super) max_x: f64,
    pub(super) max_y: f64,
    pub(super) projection: Projection,
}

impl Bounds {
    pub(super) fn new(projection: Projection) -> Self {
        Self {
            min_x: f64::MAX,
            min_y: f64::MAX,
            max_x: f64::MIN,
            max_y: f64::MIN,
            projection,
        }
    }

    pub(super) fn add(&mut self, point: &PidPoint) {
        self.add_mm(self.projection.mm(point.x), self.projection.mm(point.y));
    }

    /// Take in a point that is already on the sheet in millimetres.
    pub(super) fn add_mm(&mut self, x: f64, y: f64) {
        if !self.projection.band.holds(x) || !self.projection.band.holds(y) {
            return;
        }
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    /// Take in the rectangle a set of drawn entities covers.
    pub(super) fn add_drawn(&mut self, entities: &[EntityType]) {
        let Some((min_x, min_y, max_x, max_y)) = drawn_bounds(entities) else {
            return;
        };
        self.add_mm(min_x, min_y);
        self.add_mm(max_x, max_y);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }
}

pub(super) fn accumulate_bounds(kind: &PidGraphicKind, built: &[EntityType], bounds: &mut Bounds) {
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
pub(super) fn drawn_extent(entity: &EntityType) -> Option<(f64, f64, f64, f64)> {
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
pub(super) fn symbol_label_anchor(drawn: &[EntityType], insertion_mm: (f64, f64)) -> Vector3 {
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
pub(super) fn drawn_bounds(entities: &[EntityType]) -> Option<(f64, f64, f64, f64)> {
    entities
        .iter()
        .filter_map(drawn_extent)
        .reduce(union_extent)
}

pub(super) fn union_extent(
    a: (f64, f64, f64, f64),
    b: (f64, f64, f64, f64),
) -> (f64, f64, f64, f64) {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

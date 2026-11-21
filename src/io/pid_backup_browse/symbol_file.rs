//! Reading a symbol file of a backup, to show it by itself.
//!
//! A backup carries the plant's symbol library: hundreds of `.sym` files
//! under `Ref\Symbols`, and the same files in one of its archives. This
//! module reads one of them -- from the `Ref` tree, out of an archive, or
//! from the backup folder -- and answers what the browser shows of it
//! (issue #116): how the file reads, what is drawn of it, and what it
//! holds.
//!
//! * How a file reads is one of the four states the symbol census counts
//!   ([`crate::io::pid_sym_census`], issue #114), by the census's own rules,
//!   so the two agree file by file.
//! * What is drawn of it is what the import draws of its body: the
//!   primitives [`SymbolGeometry::displayed`] picks, each as the entities a
//!   placement of the symbol puts on a sheet
//!   ([`crate::io::pid::symbol_file_primitive_entities`]), flattened to a
//!   [`Sketch`] in millimetres. A primitive the import draws nothing for is
//!   not drawn here either, and is counted.
//! * At most [`SYMBOL_LIMIT`] bytes are read: a longer file is no symbol.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::f64::consts::TAU;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use document::entities::hatch::BoundaryEdge;
use document::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use document::types::Color;
use document::EntityType;
use pid_parse::symbol_library::{
    read_symbol_geometry_from_bytes, StyledPrimitive, SymbolGeometry, SymbolPrimitive,
};

/// Why nothing is drawn of a symbol that was read short, as the conversion
/// library says it.
pub use pid_parse::symbol_library::SymbolNotDrawn;

use super::text_file::with_zip_member;
use crate::io::pid::symbol_file_primitive_entities;
use crate::io::pid_sym_census::CFB_MAGIC;

/// How much of a file is read as a symbol. The largest symbol of the plants
/// seen so far is 104 KB.
pub const SYMBOL_LIMIT: u64 = 16 << 20;

/// How many chords a full circle is drawn with.
const CIRCLE_CHORDS: usize = 64;

/// The kinds of primitive a symbol body is made of, in the order they are
/// listed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Line,
    Circle,
    Arc,
    Polyline,
    Spline,
    Fill,
    Text,
}

impl Kind {
    /// Every kind, in the order they are listed in.
    pub const ALL: [Kind; 7] = [
        Kind::Line,
        Kind::Circle,
        Kind::Arc,
        Kind::Polyline,
        Kind::Spline,
        Kind::Fill,
        Kind::Text,
    ];

    /// The kind of `primitive`.
    pub fn of(primitive: &SymbolPrimitive) -> Self {
        match primitive {
            SymbolPrimitive::Line { .. } => Kind::Line,
            SymbolPrimitive::Circle { .. } => Kind::Circle,
            SymbolPrimitive::Arc { .. } => Kind::Arc,
            SymbolPrimitive::Polyline { .. } => Kind::Polyline,
            SymbolPrimitive::BSpline { .. } => Kind::Spline,
            SymbolPrimitive::Boundary { .. } => Kind::Fill,
            SymbolPrimitive::Text { .. } => Kind::Text,
        }
    }
}

/// How many primitives there are of each kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts([usize; Kind::ALL.len()]);

impl Counts {
    fn add(&mut self, kind: Kind) {
        self.0[kind as usize] += 1;
    }

    /// How many there are of `kind`.
    pub fn of(&self, kind: Kind) -> usize {
        self.0[kind as usize]
    }

    /// How many there are in all.
    pub fn total(&self) -> usize {
        self.0.iter().sum()
    }

    /// The kinds there is at least one of, each with how many.
    pub fn listed(&self) -> impl Iterator<Item = (Kind, usize)> + '_ {
        Kind::ALL
            .into_iter()
            .map(|kind| (kind, self.of(kind)))
            .filter(|(_, count)| *count > 0)
    }
}

/// How a symbol file reads: the four states of the census.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Read whole, and at least one primitive of it is displayed.
    Drawable,
    /// Read whole, and it holds no primitive to display.
    EmptyBody,
    /// A stream that decides what is drawn was read short. `Some` is why
    /// nothing is drawn of the file; `None` is a file part of which is
    /// drawn.
    Short(Option<SymbolNotDrawn>),
    /// Not read as a symbol.
    Failed(Failure),
}

impl Reading {
    /// The state the census gives a file that reads so.
    #[cfg(test)]
    pub(crate) fn census_status(&self) -> crate::io::pid_sym_census::SymDecodeStatus {
        use crate::io::pid_sym_census::SymDecodeStatus;
        match self {
            Reading::Drawable => SymDecodeStatus::SuccessDrawable,
            Reading::EmptyBody => SymDecodeStatus::SuccessEmptyBody,
            Reading::Short(_) => SymDecodeStatus::PartialNotDrawn,
            Reading::Failed(_) => SymDecodeStatus::Failed,
        }
    }
}

/// Why a file was not read as a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// It is longer than [`SYMBOL_LIMIT`] and was not read; its length.
    TooLarge(u64),
    /// It does not open the way an OLE compound file does; its length.
    NotCompound(u64),
    /// The conversion library could not decode it, in its words.
    Decode(String),
}

/// What was read of a symbol file.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolFile {
    /// How the file reads.
    pub reading: Reading,
    /// What it holds, when it decodes.
    pub body: Option<Body>,
}

/// What a symbol file that decodes holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// What the import draws of it.
    pub sketch: Arc<Sketch>,
    /// The primitives displayed, by kind.
    pub displayed: Counts,
    /// Of those, the ones the import draws nothing for.
    pub not_imported: Counts,
    /// The layers of the displayed primitives -- `None` for the primitives
    /// that name no layer -- each with how many it holds.
    pub layers_shown: Vec<(Option<String>, usize)>,
    /// The layers of the primitives that are on a sheet that is drawn and
    /// are not displayed themselves, each with how many it holds.
    pub layers_off: Vec<(Option<String>, usize)>,
    /// The layers of the primitives on the sheets that are not drawn, each
    /// with how many it holds.
    pub on_sheets_not_drawn: Vec<(Option<String>, usize)>,
    /// How many sheets the file has: the ones a primitive was read from and
    /// the ones a view set governs.
    pub sheets: usize,
    /// How many records the file has retired: copies its own reader skips.
    pub retired: usize,
    /// The chain streams whose framing does not hold, each in the
    /// conversion library's words.
    pub faults: Vec<String>,
}

/// The colour something of a sketch is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// The colour the surface draws its foreground in: what black and
    /// white are on a sheet, and what a stroke that states no colour gets.
    Foreground,
    /// The colour the file states.
    Rgb([u8; 3]),
}

/// A run of straight segments.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub points: Vec<(f64, f64)>,
    /// Whether the last point joins the first.
    pub closed: bool,
    pub ink: Ink,
}

/// A filled polygon.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub points: Vec<(f64, f64)>,
    pub ink: Ink,
}

/// Which end of a label its anchor is at, along its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Across {
    Left,
    Center,
    Right,
}

/// Which line of a label its anchor is on, going up it: the one its
/// letters stand on, the one their descenders reach down to, the middle of
/// its capitals, or their top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upright {
    Baseline,
    Bottom,
    Middle,
    Top,
}

/// A line of lettering.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    /// The point the label hangs from.
    pub at: (f64, f64),
    /// The height of its capitals.
    pub height: f64,
    /// Its turn in radians, counter-clockwise.
    pub rotation: f64,
    pub across: Across,
    pub upright: Upright,
    pub ink: Ink,
}

/// What is drawn of a symbol, flattened for a thumbnail: millimetres about
/// the symbol's origin, y up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sketch {
    pub strokes: Vec<Stroke>,
    pub fills: Vec<Fill>,
    pub labels: Vec<Label>,
}

impl Sketch {
    /// Whether there is nothing to draw.
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.fills.is_empty() && self.labels.is_empty()
    }

    /// The rectangle the sketch covers, as `(min_x, min_y, max_x, max_y)`:
    /// its strokes and its fills, and of its lettering the anchors alone --
    /// how far a line of it reaches takes the font it is drawn in.
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let strokes = self.strokes.iter().flat_map(|stroke| &stroke.points);
        let fills = self.fills.iter().flat_map(|fill| &fill.points);
        let labels = self.labels.iter().map(|label| &label.at);
        strokes
            .chain(fills)
            .chain(labels)
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .fold(None, |bounds, &(x, y)| {
                Some(match bounds {
                    None => (x, y, x, y),
                    Some((min_x, min_y, max_x, max_y)) => {
                        (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                    }
                })
            })
    }

    /// Add what `entity`, one the import drew, shows.
    fn add(&mut self, entity: &EntityType) {
        let ink = match entity.common().color {
            Color::Rgb { r, g, b } => Ink::Rgb([r, g, b]),
            _ => Ink::Foreground,
        };
        match entity {
            EntityType::Line(line) => self.strokes.push(Stroke {
                points: vec![(line.start.x, line.start.y), (line.end.x, line.end.y)],
                closed: false,
                ink,
            }),
            EntityType::Circle(circle) => self.strokes.push(Stroke {
                points: circle_points((circle.center.x, circle.center.y), circle.radius),
                closed: true,
                ink,
            }),
            EntityType::Arc(arc) => {
                // The model keeps an arc counter-clockwise from its start
                // angle to its end angle, in radians. Two equal angles are
                // a whole turn.
                let center = (arc.center.x, arc.center.y);
                let sweep = (arc.end_angle - arc.start_angle).rem_euclid(TAU);
                let whole = sweep < 1.0e-9;
                self.strokes.push(Stroke {
                    points: if whole {
                        circle_points(center, arc.radius)
                    } else {
                        arc_points(center, arc.radius, arc.start_angle, sweep)
                    },
                    closed: whole,
                    ink,
                });
            }
            EntityType::LwPolyline(polyline) => self.strokes.push(Stroke {
                points: polyline
                    .vertices
                    .iter()
                    .map(|vertex| (vertex.location.x, vertex.location.y))
                    .collect(),
                closed: polyline.is_closed,
                ink,
            }),
            EntityType::Text(text) => {
                // A label that is not left on its baseline hangs from its
                // alignment point.
                let anchor = text.alignment_point.unwrap_or(text.insertion_point);
                self.labels.push(Label {
                    text: text.value.clone(),
                    at: (anchor.x, anchor.y),
                    height: text.height,
                    rotation: text.rotation,
                    across: match text.horizontal_alignment {
                        TextHorizontalAlignment::Center | TextHorizontalAlignment::Middle => {
                            Across::Center
                        }
                        TextHorizontalAlignment::Right => Across::Right,
                        _ => Across::Left,
                    },
                    upright: match text.vertical_alignment {
                        TextVerticalAlignment::Baseline => Upright::Baseline,
                        TextVerticalAlignment::Bottom => Upright::Bottom,
                        TextVerticalAlignment::Middle => Upright::Middle,
                        TextVerticalAlignment::Top => Upright::Top,
                    },
                    ink,
                });
            }
            EntityType::Hatch(hatch) => {
                for path in &hatch.paths {
                    let mut points: Vec<(f64, f64)> = Vec::new();
                    for edge in &path.edges {
                        match edge {
                            BoundaryEdge::Line(line) => {
                                let start = (line.start.x, line.start.y);
                                if points.last() != Some(&start) {
                                    points.push(start);
                                }
                                points.push((line.end.x, line.end.y));
                            }
                            BoundaryEdge::CircularArc(arc) => {
                                let center = (arc.center.x, arc.center.y);
                                let turn = arc.end_angle - arc.start_angle;
                                let sweep = if arc.counter_clockwise {
                                    turn.rem_euclid(TAU)
                                } else {
                                    -(-turn).rem_euclid(TAU)
                                };
                                if sweep.abs() < 1.0e-9 {
                                    points.extend(circle_points(center, arc.radius));
                                } else {
                                    points.extend(arc_points(
                                        center,
                                        arc.radius,
                                        arc.start_angle,
                                        sweep,
                                    ));
                                }
                            }
                            // The import fills with neither.
                            _ => {}
                        }
                    }
                    // A ring that ends where it began states that point
                    // once.
                    if points.len() > 1 && points.first() == points.last() {
                        points.pop();
                    }
                    if points.len() >= 3 {
                        self.fills.push(Fill { points, ink });
                    }
                }
            }
            _ => {}
        }
    }
}

/// The corners of the polygon a circle is drawn as, counter-clockwise from
/// its rightmost point. The first is not stated again at the end.
fn circle_points(center: (f64, f64), radius: f64) -> Vec<(f64, f64)> {
    (0..CIRCLE_CHORDS)
        .map(|step| {
            let angle = TAU * step as f64 / CIRCLE_CHORDS as f64;
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            )
        })
        .collect()
}

/// The points an arc is drawn through: from the angle `start` through
/// `sweep` radians, counter-clockwise when `sweep` is positive, in chords
/// no longer than a circle's.
fn arc_points(center: (f64, f64), radius: f64, start: f64, sweep: f64) -> Vec<(f64, f64)> {
    let chords =
        ((sweep.abs() / TAU * CIRCLE_CHORDS as f64).ceil() as usize).clamp(1, CIRCLE_CHORDS);
    (0..=chords)
        .map(|step| {
            let angle = start + sweep * step as f64 / chords as f64;
            (
                center.0 + radius * angle.cos(),
                center.1 + radius * angle.sin(),
            )
        })
        .collect()
}

/// What the browser shows of a symbol body the conversion library read.
pub fn digest(geometry: &SymbolGeometry) -> SymbolFile {
    let picked = geometry.displayed();
    let reading = match &picked {
        Err(why) => Reading::Short(Some(*why)),
        Ok(_) if !geometry.short.is_empty() => Reading::Short(None),
        Ok(shown) if shown.is_empty() => Reading::EmptyBody,
        Ok(_) => Reading::Drawable,
    };
    let shown: Vec<&StyledPrimitive> = picked.unwrap_or_default();
    // `displayed` answers with references into the body, so which of its
    // primitives are displayed is which of them it points at.
    let is_shown: HashSet<*const StyledPrimitive> = shown
        .iter()
        .map(|styled| std::ptr::from_ref::<StyledPrimitive>(styled))
        .collect();
    let drawn_sheets: BTreeSet<u32> = shown.iter().map(|styled| styled.sheet_ref).collect();

    let mut sketch = Sketch::default();
    let mut displayed = Counts::default();
    let mut not_imported = Counts::default();
    let mut layers_shown: BTreeMap<Option<String>, usize> = BTreeMap::new();
    for styled in &shown {
        let kind = Kind::of(&styled.primitive);
        displayed.add(kind);
        *layers_shown
            .entry(styled.sheet_layer_name.clone())
            .or_default() += 1;
        let entities = symbol_file_primitive_entities(styled);
        if entities.is_empty() {
            not_imported.add(kind);
        }
        for entity in &entities {
            sketch.add(entity);
        }
    }

    let mut layers_off: BTreeMap<Option<String>, usize> = BTreeMap::new();
    let mut on_sheets_not_drawn: BTreeMap<Option<String>, usize> = BTreeMap::new();
    for styled in &geometry.primitives {
        if is_shown.contains(&std::ptr::from_ref(styled)) {
            continue;
        }
        let layers = if drawn_sheets.contains(&styled.sheet_ref) {
            &mut layers_off
        } else {
            &mut on_sheets_not_drawn
        };
        *layers.entry(styled.sheet_layer_name.clone()).or_default() += 1;
    }

    let sheets: BTreeSet<u32> = geometry
        .primitives
        .iter()
        .map(|styled| styled.sheet_ref)
        .chain(geometry.sheet_views.iter().map(|view| view.sheet_ref))
        .collect();
    SymbolFile {
        reading,
        body: Some(Body {
            sketch: Arc::new(sketch),
            displayed,
            not_imported,
            layers_shown: layers_shown.into_iter().collect(),
            layers_off: layers_off.into_iter().collect(),
            on_sheets_not_drawn: on_sheets_not_drawn.into_iter().collect(),
            sheets: sheets.len(),
            retired: geometry.retired_records.values().sum(),
            faults: geometry
                .chain_faults
                .iter()
                .map(pid_parse::StreamChainFault::describe_framing)
                .collect(),
        }),
    }
}

/// A file that was not read as a symbol.
fn failed(why: Failure) -> SymbolFile {
    SymbolFile {
        reading: Reading::Failed(why),
        body: None,
    }
}

/// Read `bytes`, all of a symbol file.
pub fn read_symbol_bytes(bytes: &[u8]) -> SymbolFile {
    if !bytes.starts_with(&CFB_MAGIC) {
        return failed(Failure::NotCompound(bytes.len() as u64));
    }
    match read_symbol_geometry_from_bytes(bytes) {
        Ok(geometry) => digest(&geometry),
        Err(error) => failed(Failure::Decode(error.to_string())),
    }
}

/// Read what `reader` gives of a file `total` bytes long as a symbol.
fn read(reader: &mut dyn Read, total: u64) -> std::io::Result<SymbolFile> {
    if total > SYMBOL_LIMIT {
        return Ok(failed(Failure::TooLarge(total)));
    }
    // One byte more than the limit is asked for: an archive can state a
    // member shorter than it is.
    let mut bytes = Vec::new();
    reader.take(SYMBOL_LIMIT + 1).read_to_end(&mut bytes)?;
    let read = bytes.len() as u64;
    Ok(if read > SYMBOL_LIMIT {
        failed(Failure::TooLarge(read.max(total)))
    } else {
        read_symbol_bytes(&bytes)
    })
}

/// Read the symbol file at `path`.
pub fn read_symbol_file(path: &Path) -> Result<SymbolFile, String> {
    let fail = |error: std::io::Error| {
        crate::tf!("cannot read {}: {}", path.display(), error).into_owned()
    };
    let total = std::fs::metadata(path).map_err(fail)?.len();
    let mut file = std::fs::File::open(path).map_err(fail)?;
    read(&mut file, total).map_err(fail)
}

/// Read member `member` of the zip at `archive` as a symbol. `name` is the
/// member's name as the index lists it; the member is found the way
/// [`super::text_file::read_zip_member`] finds one.
pub fn read_symbol_member(archive: &Path, member: usize, name: &str) -> Result<SymbolFile, String> {
    with_zip_member(archive, member, name, read)
}

/// Symbol bodies made up for the tests, here and in the browser's.
#[cfg(test)]
pub(crate) mod made_up {
    use pid_parse::symbol_library::{
        PrimitiveFill, PrimitiveStyle, StyledPrimitive, SymbolGeometry, SymbolLettering,
        SymbolPrimitive, SymbolSheetView, SymbolShortRead,
    };

    /// The sheet the made-up bodies draw, and one they do not.
    pub(crate) const NORMAL: u32 = 6;
    pub(crate) const ALTERNATE: u32 = 7;

    /// A primitive on the sheet `sheet`, on the layer `layer` when one is
    /// named: the layer numbered 0 is `Default`, any other is numbered 1.
    pub(crate) fn on(
        sheet: u32,
        layer: Option<&str>,
        primitive: SymbolPrimitive,
    ) -> StyledPrimitive {
        StyledPrimitive {
            primitive,
            style: None,
            lettering: None,
            sheet_layer_ref: u32::from(layer.is_some()),
            sheet_layer_name: layer.map(str::to_string),
            sheet_layer_number: match layer {
                Some("Default") => 0,
                Some(_) => 1,
                None => u32::MAX,
            },
            sheet_ref: sheet,
        }
    }

    /// The view set of a body's normal view: `Default` on, `Label` off.
    pub(crate) fn normal_view(sheet: u32) -> SymbolSheetView {
        SymbolSheetView {
            sheet_ref: sheet,
            display: vec![0b0000_0001],
            layers: vec![(0, "Default".to_string()), (1, "Label".to_string())],
        }
    }

    pub(crate) fn line(from: (f64, f64), to: (f64, f64)) -> SymbolPrimitive {
        SymbolPrimitive::Line {
            start: from,
            end: to,
        }
    }

    pub(crate) fn text(value: &str, at: (f64, f64)) -> SymbolPrimitive {
        SymbolPrimitive::Text {
            text: value.to_string(),
            at,
        }
    }

    /// Lettering 2.5 mm high, hung from the middle of its line, turned a
    /// quarter.
    pub(crate) fn lettering() -> SymbolLettering {
        SymbolLettering {
            // High nibble 1: centred; low nibble 2: the middle.
            anchor_code: 0x12,
            rotation_rad: std::f64::consts::FRAC_PI_2,
            letter_height_m: 0.0025,
            height_m: None,
            height_mode: None,
            width_permille: None,
            rgb: Some([255, 0, 0]),
            alignment: None,
            line_spacing: None,
            font_name: None,
            run_style_id: None,
            spans: Vec::new(),
        }
    }

    /// A body with one primitive of every kind on its normal view, a label
    /// that is only a placeholder, a stroke on a layer the view keeps off
    /// and two strokes on an alternate sheet. Coordinates are metres.
    pub(crate) fn valve() -> SymbolGeometry {
        let mut blue = on(NORMAL, Some("Default"), line((0.0, 0.0), (0.010, 0.0)));
        blue.style = Some(PrimitiveStyle {
            rgb: [0, 0, 255],
            width_mm: 0.25,
            dash_mm: vec![2.0, -1.0],
            paper: false,
        });
        let mut tag = on(NORMAL, Some("Default"), text("PG", (0.005, 0.002)));
        tag.lettering = Some(lettering());
        SymbolGeometry {
            primitives: vec![
                blue,
                on(
                    NORMAL,
                    Some("Default"),
                    SymbolPrimitive::Circle {
                        center: (0.005, 0.0),
                        radius: 0.002,
                    },
                ),
                on(
                    NORMAL,
                    Some("Default"),
                    SymbolPrimitive::Arc {
                        center: (0.0, 0.0),
                        radius: 0.004,
                        start_angle: 0.0,
                        end_angle: std::f64::consts::FRAC_PI_2,
                        clockwise: false,
                    },
                ),
                on(
                    NORMAL,
                    None,
                    SymbolPrimitive::Polyline {
                        vertices: vec![(0.0, 0.0), (0.010, 0.004), (0.0, 0.004)],
                        is_closed: true,
                    },
                ),
                on(
                    NORMAL,
                    None,
                    SymbolPrimitive::BSpline {
                        poles: vec![(0.0, 0.0), (0.005, 0.006), (0.010, 0.0)],
                        weights: Vec::new(),
                        knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    },
                ),
                on(
                    NORMAL,
                    Some("Default"),
                    SymbolPrimitive::Boundary {
                        members: vec![SymbolPrimitive::Circle {
                            center: (0.005, 0.0),
                            radius: 0.001,
                        }],
                        fill: Some(PrimitiveFill {
                            rgb: Some([0, 128, 0]),
                            type_code: 0x002A,
                        }),
                    },
                ),
                tag,
                on(NORMAL, Some("Default"), text("NULL", (0.0, 0.0))),
                on(NORMAL, Some("Label"), line((0.0, 0.0), (0.0, 0.5))),
                on(ALTERNATE, Some("Default"), line((0.0, 0.0), (1.0, 1.0))),
                on(ALTERNATE, Some("Default"), line((1.0, 1.0), (2.0, 2.0))),
            ],
            retired_records: [(0x0018, 2), (0x0059, 1)].into_iter().collect(),
            sheet_views: vec![normal_view(NORMAL)],
            ..SymbolGeometry::default()
        }
    }

    /// A read that came up short of the clusters and of the sheets named.
    pub(crate) fn short(clusters: &[&str], sheets: &[u32]) -> SymbolShortRead {
        SymbolShortRead {
            clusters: clusters.iter().map(|path| (*path).to_string()).collect(),
            sheets: sheets.to_vec(),
            sheets_after: Vec::new(),
        }
    }

    /// [`valve`], read short of the clusters and of the sheets named.
    pub(crate) fn valve_short(clusters: &[&str], sheets: &[u32]) -> SymbolGeometry {
        SymbolGeometry {
            short: short(clusters, sheets),
            ..valve()
        }
    }

    /// A body that holds nothing.
    pub(crate) fn empty() -> SymbolGeometry {
        SymbolGeometry::default()
    }

    /// A body whose one primitive is a label that is only a placeholder.
    pub(crate) fn placeholder() -> SymbolGeometry {
        SymbolGeometry {
            primitives: vec![on(NORMAL, None, text("NULL", (0.0, 0.0)))],
            ..SymbolGeometry::default()
        }
    }

    /// The first word of the stream [`faulty`] says does not open the way
    /// a chain stream does.
    pub(crate) const WRONG_MAGIC: u32 = 0x1234_5678;

    /// [`valve`], one of whose streams does not open the way a chain
    /// stream does.
    pub(crate) fn faulty() -> SymbolGeometry {
        SymbolGeometry {
            chain_faults: vec![pid_parse::StreamChainFault {
                stream_path: "Sheet6".to_string(),
                stream_len: 64,
                wrong_magic: Some(WRONG_MAGIC),
                stop: None,
                unread_bytes: 0,
                records_read: 0,
                live_records_read: 0,
                stated_records: None,
            }],
            ..valve()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::made_up::*;
    use super::*;
    use crate::io::pid_sym_census::{inspect_sym_bytes, SymDecodeStatus};
    use std::io::Write;

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1.0e-9 && (a.1 - b.1).abs() < 1.0e-9
    }

    /// Whether `points` are `expected`, one for one.
    fn run_near(points: &[(f64, f64)], expected: &[(f64, f64)]) -> bool {
        points.len() == expected.len()
            && points
                .iter()
                .zip(expected)
                .all(|(point, expected)| near(*point, *expected))
    }

    #[test]
    fn a_body_read_whole_is_drawn_and_counted() {
        let file = digest(&valve());
        assert_eq!(file.reading, Reading::Drawable);
        let body = file.body.expect("a body");

        // Eight primitives are displayed, one of every kind and a second
        // text; the import draws nothing for the placeholder.
        let listed: Vec<(Kind, usize)> = body.displayed.listed().collect();
        assert_eq!(
            listed,
            [
                (Kind::Line, 1),
                (Kind::Circle, 1),
                (Kind::Arc, 1),
                (Kind::Polyline, 1),
                (Kind::Spline, 1),
                (Kind::Fill, 1),
                (Kind::Text, 2),
            ]
        );
        assert_eq!(body.displayed.total(), 8);
        assert_eq!(
            body.not_imported.listed().collect::<Vec<_>>(),
            [(Kind::Text, 1)]
        );

        // Where they are, and where the rest of the file is.
        assert_eq!(
            body.layers_shown,
            [(None, 2), (Some("Default".to_string()), 6)]
        );
        assert_eq!(body.layers_off, [(Some("Label".to_string()), 1)]);
        assert_eq!(body.on_sheets_not_drawn, [(Some("Default".to_string()), 2)]);
        assert_eq!((body.sheets, body.retired), (2, 3));
        assert!(body.faults.is_empty());

        // What is drawn, in millimetres: five strokes, a fill, a label.
        let sketch = &body.sketch;
        assert_eq!(
            (
                sketch.strokes.len(),
                sketch.fills.len(),
                sketch.labels.len()
            ),
            (5, 1, 1)
        );
        let stroke = &sketch.strokes[0];
        assert!(
            run_near(&stroke.points, &[(0.0, 0.0), (10.0, 0.0)]),
            "{stroke:?}"
        );
        // The colour the file states; the dash it states is not drawn.
        assert_eq!((stroke.closed, stroke.ink), (false, Ink::Rgb([0, 0, 255])));
        let circle = &sketch.strokes[1];
        assert_eq!(
            (circle.points.len(), circle.closed, circle.ink),
            (CIRCLE_CHORDS, true, Ink::Foreground)
        );
        assert!(near(circle.points[0], (7.0, 0.0)));
        assert!(near(circle.points[CIRCLE_CHORDS / 4], (5.0, 2.0)));
        // A quarter turn, counter-clockwise from the x axis.
        let arc = &sketch.strokes[2];
        assert_eq!(
            (arc.points.len(), arc.closed),
            (CIRCLE_CHORDS / 4 + 1, false)
        );
        assert!(near(arc.points[0], (4.0, 0.0)));
        assert!(near(arc.points[CIRCLE_CHORDS / 4], (0.0, 4.0)));
        let run = &sketch.strokes[3];
        assert!(
            run_near(&run.points, &[(0.0, 0.0), (10.0, 4.0), (0.0, 4.0)]),
            "{run:?}"
        );
        assert!(run.closed);
        // The curve runs from its first pole to its last, under the middle
        // one.
        let curve = &sketch.strokes[4];
        assert!(curve.points.len() > 2 && !curve.closed);
        assert!(near(curve.points[0], (0.0, 0.0)));
        assert!(near(curve.points[curve.points.len() - 1], (10.0, 0.0)));
        assert!(curve.points.iter().all(|(_, y)| (-1.0e-9..6.0).contains(y)));

        let fill = &sketch.fills[0];
        assert_eq!(
            (fill.points.len(), fill.ink),
            (CIRCLE_CHORDS, Ink::Rgb([0, 128, 0]))
        );
        assert!(near(fill.points[0], (6.0, 0.0)));

        let label = &sketch.labels[0];
        assert_eq!(label.text, "PG");
        assert!(near(label.at, (5.0, 2.0)));
        assert!((label.height - 2.5).abs() < 1.0e-9);
        assert!((label.rotation - std::f64::consts::FRAC_PI_2).abs() < 1.0e-9);
        assert_eq!(
            (label.across, label.upright, label.ink),
            (Across::Center, Upright::Middle, Ink::Rgb([255, 0, 0]))
        );

        // From the left end of the line and the foot of the circle to the
        // right end of the line and the top of the run: the curve stays
        // under its middle pole, and the label adds its anchor alone.
        let (min_x, min_y, max_x, max_y) = sketch.bounds().expect("bounds");
        assert!(near((min_x, min_y), (0.0, -2.0)), "{min_x} {min_y}");
        assert!(near((max_x, max_y), (10.0, 4.0)), "{max_x} {max_y}");
    }

    #[test]
    fn a_label_hangs_from_the_line_its_record_states() {
        // The anchor byte of a label's record says which end of its line
        // the point it hangs from is at, and which line of it that point
        // is on. A label with no record of its own stands on its baseline
        // and starts at the point.
        let lettered = |code, value: &str| {
            let mut label = on(NORMAL, None, text(value, (0.0, 0.0)));
            label.lettering = Some(pid_parse::symbol_library::SymbolLettering {
                anchor_code: code,
                rotation_rad: 0.0,
                ..lettering()
            });
            label
        };
        let geometry = SymbolGeometry {
            primitives: vec![
                on(NORMAL, None, text("A", (0.0, 0.0))),
                lettered(0x00, "B"),
                lettered(0x21, "C"),
                lettered(0x12, "D"),
            ],
            ..SymbolGeometry::default()
        };
        let body = digest(&geometry).body.expect("a body");
        let hung: Vec<(&str, Across, Upright)> = body
            .sketch
            .labels
            .iter()
            .map(|label| (label.text.as_str(), label.across, label.upright))
            .collect();
        assert_eq!(
            hung,
            [
                ("A", Across::Left, Upright::Baseline),
                ("B", Across::Left, Upright::Bottom),
                ("C", Across::Right, Upright::Top),
                ("D", Across::Center, Upright::Middle),
            ]
        );
    }

    #[test]
    fn a_fill_is_the_shapes_the_import_fills() {
        // The import fills every circle of a boundary as a shape of its
        // own, and the lines of it together as one ring.
        let geometry = SymbolGeometry {
            primitives: vec![on(
                NORMAL,
                None,
                SymbolPrimitive::Boundary {
                    members: vec![
                        line((0.0, 0.0), (0.004, 0.0)),
                        line((0.004, 0.0), (0.002, 0.003)),
                        line((0.002, 0.003), (0.0, 0.0)),
                        SymbolPrimitive::Circle {
                            center: (0.010, 0.0),
                            radius: 0.001,
                        },
                    ],
                    fill: None,
                },
            )],
            ..SymbolGeometry::default()
        };
        let file = digest(&geometry);
        assert_eq!(file.reading, Reading::Drawable);
        let body = file.body.expect("a body");
        assert_eq!(
            body.displayed.listed().collect::<Vec<_>>(),
            [(Kind::Fill, 1)]
        );
        assert_eq!(body.not_imported.total(), 0);
        let sketch = &body.sketch;
        assert!(sketch.strokes.is_empty() && sketch.labels.is_empty());
        assert_eq!(sketch.fills.len(), 2);
        let disc = &sketch.fills[0];
        assert_eq!(
            (disc.points.len(), disc.ink),
            (CIRCLE_CHORDS, Ink::Foreground)
        );
        assert!(near(disc.points[0], (11.0, 0.0)));
        // The ring ends where it began, and states that corner once.
        let ring = &sketch.fills[1];
        assert!(
            run_near(&ring.points, &[(0.0, 0.0), (4.0, 0.0), (2.0, 3.0)]),
            "{ring:?}"
        );
        assert_eq!(ring.ink, Ink::Foreground);

        // A boundary with neither a circle nor a line is filled by
        // nothing, and is counted as what the import does not draw.
        let geometry = SymbolGeometry {
            primitives: vec![on(
                NORMAL,
                None,
                SymbolPrimitive::Boundary {
                    members: vec![SymbolPrimitive::Arc {
                        center: (0.0, 0.0),
                        radius: 0.004,
                        start_angle: 0.0,
                        end_angle: std::f64::consts::FRAC_PI_2,
                        clockwise: false,
                    }],
                    fill: None,
                },
            )],
            ..SymbolGeometry::default()
        };
        let file = digest(&geometry);
        assert_eq!(file.reading, Reading::Drawable);
        let body = file.body.expect("a body");
        assert_eq!(
            body.not_imported.listed().collect::<Vec<_>>(),
            [(Kind::Fill, 1)]
        );
        assert!(body.sketch.is_empty());
    }

    #[test]
    fn a_body_with_nothing_in_it_is_empty() {
        let file = digest(&empty());
        assert_eq!(file.reading, Reading::EmptyBody);
        let body = file.body.expect("a body");
        assert!(body.sketch.is_empty());
        assert_eq!(body.sketch.bounds(), None);
        assert_eq!((body.displayed.total(), body.not_imported.total()), (0, 0));
        assert!(body.layers_shown.is_empty() && body.layers_off.is_empty());
        assert!(body.on_sheets_not_drawn.is_empty());
        assert_eq!((body.sheets, body.retired), (0, 0));

        // A sheet a view set governs is one of the file's sheets, with
        // nothing read from it.
        let governed = SymbolGeometry {
            sheet_views: vec![normal_view(NORMAL)],
            ..SymbolGeometry::default()
        };
        let file = digest(&governed);
        assert_eq!(file.reading, Reading::EmptyBody);
        assert_eq!(file.body.expect("a body").sheets, 1);
    }

    #[test]
    fn a_body_of_placeholders_reads_drawable_and_draws_nothing() {
        // The census counts it drawable: a primitive is displayed. The
        // import draws nothing for a label that is only `NULL`.
        let file = digest(&placeholder());
        assert_eq!(file.reading, Reading::Drawable);
        let body = file.body.expect("a body");
        assert!(body.sketch.is_empty());
        assert_eq!((body.displayed.total(), body.not_imported.total()), (1, 1));
    }

    #[test]
    fn a_file_read_short_says_why_nothing_is_drawn_or_draws_its_part() {
        // The cluster that says which sheet and which layers draw.
        let geometry = valve_short(&["PSMcluster0"], &[]);
        let file = digest(&geometry);
        assert_eq!(
            file.reading,
            Reading::Short(Some(SymbolNotDrawn::ClusterShort))
        );
        let body = file.body.expect("a body");
        assert!(body.sketch.is_empty());
        assert_eq!(body.displayed.total(), 0);
        // No sheet is drawn, so every primitive is on one that is not: the
        // layers that draw, the one that is off and the alternate sheet
        // alike.
        assert_eq!(
            body.on_sheets_not_drawn,
            [
                (None, 2),
                (Some("Default".to_string()), 8),
                (Some("Label".to_string()), 1)
            ]
        );
        assert!(body.layers_off.is_empty());
        assert_eq!(geometry.primitives.len(), 11);

        // The normal view, read short: what was read of it is drawn.
        let file = digest(&valve_short(&[], &[NORMAL]));
        assert_eq!(file.reading, Reading::Short(None));
        let body = file.body.expect("a body");
        assert_eq!(body.displayed.total(), 8);
        assert!(!body.sketch.is_empty());

        // A stream whose framing does not hold is said in the conversion
        // library's words.
        let body = digest(&faulty()).body.expect("a body");
        assert_eq!(body.faults.len(), 1);
        assert!(
            body.faults[0].contains(&format!("0x{WRONG_MAGIC:08X}")),
            "{:?}",
            body.faults
        );

        // The normal view, read short, and nothing read of it shows.
        let geometry = SymbolGeometry {
            primitives: vec![
                on(NORMAL, Some("Label"), line((0.0, 0.0), (0.0, 0.5))),
                on(ALTERNATE, Some("Default"), line((0.0, 0.0), (1.0, 1.0))),
            ],
            sheet_views: vec![normal_view(NORMAL)],
            short: short(&[], &[NORMAL]),
            ..SymbolGeometry::default()
        };
        assert_eq!(
            digest(&geometry).reading,
            Reading::Short(Some(SymbolNotDrawn::NothingToShow))
        );

        // No normal view, and a sheet in the running read short.
        let geometry = SymbolGeometry {
            primitives: vec![on(ALTERNATE, None, line((0.0, 0.0), (1.0, 1.0)))],
            short: short(&[], &[ALTERNATE]),
            ..SymbolGeometry::default()
        };
        assert_eq!(
            digest(&geometry).reading,
            Reading::Short(Some(SymbolNotDrawn::PickAmongShortSheets))
        );
    }

    #[test]
    fn a_file_that_is_no_symbol_fails_the_way_the_census_counts_it() {
        let mut hollow = CFB_MAGIC.to_vec();
        hollow.extend([0u8; 256]);
        for (bytes, not_compound) in [
            (&b""[..], true),
            (&b"PLAIN-ASCII-NOT-CFB"[..], true),
            (&CFB_MAGIC[..7], true),
            // It opens the way a compound file does and holds none.
            (&hollow[..], false),
        ] {
            let file = read_symbol_bytes(bytes);
            assert_eq!(file.body, None, "{bytes:?}");
            match &file.reading {
                Reading::Failed(Failure::NotCompound(length)) => {
                    assert!(not_compound, "{bytes:?}");
                    assert_eq!(*length, bytes.len() as u64);
                }
                Reading::Failed(Failure::Decode(why)) => {
                    assert!(!not_compound, "{bytes:?}");
                    assert!(!why.is_empty());
                }
                other => panic!("{bytes:?} read as {other:?}"),
            }
            let census = inspect_sym_bytes("test", "a.sym", bytes, None);
            assert_eq!(census.status, SymDecodeStatus::Failed, "{bytes:?}");
            assert_eq!(file.reading.census_status(), census.status, "{bytes:?}");
        }
    }

    #[test]
    fn a_symbol_is_read_from_a_file_and_from_an_archive_alike() {
        let root = std::env::temp_dir().join(format!("ocs-pid-symbol-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let plain = b"PLAIN-ASCII-NOT-CFB";
        let on_disk = root.join("plain.sym");
        std::fs::write(&on_disk, plain).unwrap();
        let archive = root.join("symbols.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.add_directory("Piping/", options).unwrap();
        writer.start_file("Piping/plain.sym", options).unwrap();
        writer.write_all(plain).unwrap();
        writer.finish().unwrap();

        let expected = failed(Failure::NotCompound(plain.len() as u64));
        assert_eq!(read_symbol_file(&on_disk), Ok(expected.clone()));
        assert_eq!(
            read_symbol_member(&archive, 1, "Piping/plain.sym"),
            Ok(expected.clone())
        );
        // A position that holds another member: found by its name.
        assert_eq!(
            read_symbol_member(&archive, 0, "Piping/plain.sym"),
            Ok(expected)
        );

        // What is not there says so, with where it was looked for.
        let missing = read_symbol_file(&root.join("gone.sym")).unwrap_err();
        assert!(missing.contains("gone.sym"), "{missing}");
        let missing = read_symbol_member(&archive, 0, "Piping/gone.sym").unwrap_err();
        assert!(
            missing.contains("Piping/gone.sym") && missing.contains("symbols.zip"),
            "{missing}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_longer_than_the_limit_is_not_read() {
        // A reader that would give more than the limit is never asked for
        // it: the length the file states is enough.
        struct Endless;
        impl Read for Endless {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                panic!("a file stated too long is not read");
            }
        }
        assert_eq!(
            read(&mut Endless, SYMBOL_LIMIT + 1).unwrap(),
            failed(Failure::TooLarge(SYMBOL_LIMIT + 1))
        );
        // One at the limit is read.
        assert_eq!(
            read(&mut &b"short"[..], SYMBOL_LIMIT).unwrap(),
            failed(Failure::NotCompound(5))
        );
        // An archive can state a member shorter than it is: what is read
        // of it stops a byte past the limit, and that byte says the file is
        // too long.
        let mut understated = std::io::repeat(0).take(SYMBOL_LIMIT + 5);
        assert_eq!(
            read(&mut understated, 10).unwrap(),
            failed(Failure::TooLarge(SYMBOL_LIMIT + 1))
        );
        assert_eq!(understated.limit(), 4, "one byte past the limit is read");
    }
}

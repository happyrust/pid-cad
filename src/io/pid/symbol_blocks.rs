// Symbol placements as blocks: each placement is one block reference, and
// placements that draw the same thing share one definition.

use std::fmt::Write as _;

use document::entities::{Block, BlockEnd, Insert};
use document::tables::BlockRecord;
use document::Handle;

use super::*;

/// What the name of every block definition made here starts with; the
/// symbol's own name follows.
pub(super) const SYMBOL_BLOCK_PREFIX: &str = "SPPID_";

/// The definition of a placement that names no symbol.
const UNNAMED_SYMBOL_BLOCK: &str = "SPPID_SYMBOL";

/// The longest name before a variant's number is added: a table name holds
/// 255 characters, and this leaves the number its room.
const BASE_NAME_CHARS: usize = 240;

/// The decimal places two drawings of a symbol have to agree to: a millionth
/// of a drawing unit, far below anything a sheet states and far above what a
/// subtraction leaves in the last bits.
const FINGERPRINT_DECIMALS: usize = 6;

/// The application whose record on a hatch states the origin its pattern
/// starts from.
const PATTERN_ORIGIN_APP: &str = "ACAD";

/// The block definitions an import has made for its symbol placements, kept
/// so that a placement which draws what an earlier one drew refers to that
/// definition instead of making another.
#[derive(Default)]
pub(super) struct SymbolBlocks {
    /// Per symbol name, each distinct drawing of it met so far -- its
    /// fingerprint and the block that holds it -- in the order met.
    drawings: HashMap<String, Vec<(String, String)>>,
    /// Definitions made.
    pub(super) definitions: usize,
    /// References handed out.
    pub(super) references: usize,
}

impl SymbolBlocks {
    /// File one placement of `symbol` as a block. `members` are the entities
    /// it draws, in drawing coordinates and already on the layers and in the
    /// styles they are to keep; `origin` is the placement's insertion point.
    ///
    /// The members go into a definition, moved so that `origin` is its base
    /// point -- the definition of an earlier placement when that one drew
    /// the same thing, a new one otherwise. What comes back is the reference
    /// that draws them where they were. It is not in the document yet: the
    /// layer and the record it carries are the caller's to give.
    pub(super) fn place(
        &mut self,
        doc: &mut Document,
        symbol: Option<&str>,
        origin: Vector3,
        mut members: Vec<EntityType>,
    ) -> Insert {
        for member in &mut members {
            to_block_space(member, origin);
        }
        let symbol = symbol.map(str::trim).filter(|name| !name.is_empty());
        let print = fingerprint(&members);
        let drawings = self
            .drawings
            .entry(symbol.unwrap_or_default().to_string())
            .or_default();
        let known = drawings
            .iter()
            .find(|(seen, _)| *seen == print)
            .map(|(_, block)| block.clone());
        let block = match known {
            Some(block) => block,
            None => {
                let block = fresh_block_name(doc, symbol, drawings.len() + 1);
                define_block(doc, &block, members);
                drawings.push((print, block.clone()));
                self.definitions += 1;
                block
            }
        };
        self.references += 1;
        Insert::new(block, origin)
    }
}

/// Move `entity` from drawing coordinates into those of a block whose
/// reference sits at `origin`, unrotated and unscaled.
///
/// Moving a hatch also writes down the origin its pattern starts from --
/// for one that stated none, the move itself, which is a different point
/// for every placement. A fill without pattern lines has nothing that
/// starts anywhere, so it is left saying what it said: the same fill drawn
/// in two places is the same member.
fn to_block_space(entity: &mut EntityType, origin: Vector3) {
    let said = match &*entity {
        EntityType::Hatch(fill) if fill.pattern.lines.is_empty() => Some(
            fill.common
                .extended_data
                .get_record(PATTERN_ORIGIN_APP)
                .cloned(),
        ),
        _ => None,
    };
    entity.as_entity_mut().translate(-origin);
    if let Some(said) = said {
        let records = &mut entity.common_mut().extended_data;
        match said {
            Some(record) => records.upsert_record(record),
            None => {
                let _ = records.remove_record(PATTERN_ORIGIN_APP);
            }
        }
    }
}

/// What two placements have to agree on to share a definition: the `Debug`
/// text of every member in block coordinates -- its kind, geometry, layer,
/// colour, weight, linetype, lettering and record, all of it -- with each
/// decimal rounded to [`FINGERPRINT_DECIMALS`] places.
///
/// Equal fingerprints are equal drawings to within that rounding. Anything
/// the text differs in makes a definition of its own, which costs file size
/// and nothing else.
fn fingerprint(members: &[EntityType]) -> String {
    let mut text = String::new();
    for member in members {
        let _ = writeln!(text, "{member:?}");
    }
    round_decimals(&text)
}

/// `text` with every decimal number outside a quoted string rewritten to
/// [`FINGERPRINT_DECIMALS`] places.
///
/// `Debug` writes a float with a point or an exponent and an integer with
/// neither, so integers -- handles, counts, colour indexes -- pass through
/// as they are, and so do the digits of a type's name (`Vector3`). Whatever
/// stands between quotes passes through too: that is where a text's value,
/// a layer's name and a record's strings are.
fn round_decimals(text: &str) -> String {
    let bytes = text.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let digits_end = |mut at: usize| {
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        at
    };
    let mut out = String::with_capacity(text.len());
    // Where the stretch not yet copied to `out` starts.
    let mut pending = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    if bytes[at] == b'\\' {
                        at += 1;
                    }
                    at += 1;
                }
                at += 1;
            }
            // A digit that follows a letter is part of a name, not a number.
            b'0'..=b'9' if at == 0 || !is_word(bytes[at - 1]) => {
                let start = if at > 0 && bytes[at - 1] == b'-' {
                    at - 1
                } else {
                    at
                };
                let mut end = digits_end(at);
                let mut decimal = false;
                if bytes.get(end) == Some(&b'.')
                    && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)
                {
                    end = digits_end(end + 1);
                    decimal = true;
                }
                if bytes.get(end) == Some(&b'e') {
                    let exponent = if bytes.get(end + 1) == Some(&b'-') {
                        end + 2
                    } else {
                        end + 1
                    };
                    if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                        end = digits_end(exponent);
                        decimal = true;
                    }
                }
                let ends_clean = !bytes.get(end).is_some_and(|byte| is_word(*byte));
                let value = (decimal && ends_clean)
                    .then(|| text[start..end].parse::<f64>().ok())
                    .flatten();
                if let Some(value) = value {
                    out.push_str(&text[pending..start]);
                    let from = out.len();
                    let _ = write!(out, "{:.*}", FINGERPRINT_DECIMALS, value);
                    // What rounds to nothing from below is zero, not minus zero.
                    if out[from..].starts_with('-')
                        && out[from..]
                            .bytes()
                            .all(|byte| matches!(byte, b'-' | b'0' | b'.'))
                    {
                        out.remove(from);
                    }
                    pending = end;
                }
                at = end;
            }
            _ => at += 1,
        }
    }
    out.push_str(&text[pending..]);
    out
}

/// A name the document does not hold yet for drawing number `nth` (from 1)
/// of `symbol`: `SPPID_<name>` for the first, `SPPID_<name>_2`, `_3`, ...
/// after it, counting on past any name already taken. Characters a block
/// name cannot hold become `_`; a placement that names no symbol is
/// `SPPID_SYMBOL`.
fn fresh_block_name(doc: &Document, symbol: Option<&str>, nth: usize) -> String {
    let base = match symbol {
        Some(name) => crate::scene::sanitized_table_name(
            &format!("{SYMBOL_BLOCK_PREFIX}{name}"),
            UNNAMED_SYMBOL_BLOCK,
        ),
        None => UNNAMED_SYMBOL_BLOCK.to_string(),
    };
    let base: String = base.chars().take(BASE_NAME_CHARS).collect();
    let base = base.trim_end();
    let mut number = nth;
    loop {
        let candidate = if number == 1 {
            base.to_string()
        } else {
            format!("{base}_{number}")
        };
        if !doc.block_records.contains(&candidate) {
            return candidate;
        }
        number += 1;
    }
}

/// Add the definition `name` to the document: its record, the BLOCK that
/// opens it, `members` owned by the record, and the ENDBLK that closes it --
/// the order `Scene::create_block_with_options` files a drawn block in, and
/// so the order the handles are issued in.
fn define_block(doc: &mut Document, name: &str, members: Vec<EntityType>) {
    let record_handle = doc.allocate_handle();
    let block_handle = doc.allocate_handle();
    let mut record = BlockRecord::new(name);
    record.handle = record_handle;
    record.block_entity_handle = block_handle;
    // `name` was free in the table a moment ago, so this cannot clash.
    let _ = doc.block_records.add(record);

    let mut block = Block::new(name, Vector3::ZERO);
    block.common.handle = block_handle;
    block.common.owner_handle = record_handle;
    let _ = doc.add_entity(EntityType::Block(block));

    for mut member in members {
        let common = member.common_mut();
        common.handle = Handle::NULL;
        common.owner_handle = record_handle;
        let _ = doc.add_entity(member);
    }

    let end_handle = doc.allocate_handle();
    let mut end = BlockEnd::new();
    end.common.handle = end_handle;
    end.common.owner_handle = record_handle;
    let _ = doc.add_entity(EntityType::BlockEnd(end));
    if let Some(record) = doc.block_records.get_mut(name) {
        record.block_end_handle = end_handle;
    }
}

#[cfg(test)]
mod tests {
    use document::entities::hatch::{HatchPattern, HatchPatternLine};
    use document::entities::Spline;

    use super::*;

    fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> EntityType {
        EntityType::Line(Line::from_points(
            Vector3::new(x1, y1, 0.0),
            Vector3::new(x2, y2, 0.0),
        ))
    }

    fn label(x: f64, y: f64, value: &str) -> EntityType {
        let mut text = Text::new();
        text.value = value.to_string();
        text.insertion_point = Vector3::new(x, y, 0.0);
        text.height = 2.0;
        EntityType::Text(text)
    }

    /// A small symbol body drawn around `at`: two strokes, a ring and the
    /// symbol's name, the way a placement's entities reach `place`.
    fn valve(at: (f64, f64)) -> Vec<EntityType> {
        let (x, y) = at;
        let mut ring = Circle::new();
        ring.center = Vector3::new(x + 0.3, y + 0.7, 0.0);
        ring.radius = 1.1;
        vec![
            line(x - 2.1, y - 1.3, x + 2.1, y + 1.3),
            line(x - 2.1, y + 1.3, x + 2.1, y - 1.3),
            EntityType::Circle(ring),
            label(x - 2.1, y + 2.9, "Gate Valve"),
        ]
    }

    /// A solid fill the way the importer draws one -- a ring handed to
    /// `Hatch::new` -- around `at`.
    fn fill(at: (f64, f64)) -> Hatch {
        let (x, y) = at;
        let corners = [(x - 1.0, y - 1.0), (x + 1.0, y - 1.0), (x, y + 1.5)];
        let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
        for (from, to) in corners.iter().zip(corners.iter().cycle().skip(1)) {
            path.edges.push(BoundaryEdge::Line(LineEdge {
                start: Vector2::new(from.0, from.1),
                end: Vector2::new(to.0, to.1),
            }));
        }
        let mut hatch = Hatch::new();
        hatch.paths.push(path);
        hatch
    }

    /// The fingerprint `members` have once they are in the block space of a
    /// placement at `origin`.
    fn print_at(origin: (f64, f64), mut members: Vec<EntityType>) -> String {
        for member in &mut members {
            to_block_space(member, Vector3::new(origin.0, origin.1, 0.0));
        }
        fingerprint(&members)
    }

    #[track_caller]
    fn assert_at(actual: (f64, f64), expected: (f64, f64), what: &str) {
        assert!(
            (actual.0 - expected.0).abs() <= 1e-9 && (actual.1 - expected.1).abs() <= 1e-9,
            "{what}: {actual:?}, expected {expected:?}"
        );
    }

    /// Every kind a symbol body is drawn with moves by the insertion point
    /// and by nothing else: what was at `p` is at `p - origin`, and a radius,
    /// an angle, a height or a bulge is what it was.
    #[test]
    fn a_member_moves_into_block_space_by_the_insertion_point_alone() {
        let origin = Vector3::new(120.5, -40.25, 0.0);

        let mut entity = line(121.5, -38.25, 130.5, -40.25);
        to_block_space(&mut entity, origin);
        let EntityType::Line(moved) = &entity else {
            panic!("a line stays a line");
        };
        assert_at((moved.start.x, moved.start.y), (1.0, 2.0), "line start");
        assert_at((moved.end.x, moved.end.y), (10.0, 0.0), "line end");

        let mut circle = Circle::new();
        circle.center = Vector3::new(125.5, -35.25, 0.0);
        circle.radius = 3.5;
        let mut entity = EntityType::Circle(circle);
        to_block_space(&mut entity, origin);
        let EntityType::Circle(moved) = &entity else {
            panic!("a circle stays a circle");
        };
        assert_at(
            (moved.center.x, moved.center.y),
            (5.0, 5.0),
            "circle centre",
        );
        assert_eq!(moved.radius, 3.5);

        let mut arc = document::entities::Arc::new();
        arc.center = Vector3::new(120.5, -40.25, 0.0);
        arc.radius = 2.0;
        arc.start_angle = 0.25;
        arc.end_angle = 1.75;
        let mut entity = EntityType::Arc(arc);
        to_block_space(&mut entity, origin);
        let EntityType::Arc(moved) = &entity else {
            panic!("an arc stays an arc");
        };
        assert_at((moved.center.x, moved.center.y), (0.0, 0.0), "arc centre");
        assert_eq!(
            (moved.radius, moved.start_angle, moved.end_angle),
            (2.0, 0.25, 1.75)
        );

        let mut polyline = LwPolyline::from_points(vec![
            Vector2::new(120.5, -40.25),
            Vector2::new(124.5, -40.25),
            Vector2::new(124.5, -37.25),
        ]);
        polyline.vertices[1].bulge = 0.5;
        polyline.is_closed = true;
        let mut entity = EntityType::LwPolyline(polyline);
        to_block_space(&mut entity, origin);
        let EntityType::LwPolyline(moved) = &entity else {
            panic!("a polyline stays a polyline");
        };
        let corners: Vec<(f64, f64)> = moved
            .vertices
            .iter()
            .map(|vertex| (vertex.location.x, vertex.location.y))
            .collect();
        assert_eq!(corners.len(), 3);
        assert_at(corners[0], (0.0, 0.0), "polyline vertex 1");
        assert_at(corners[1], (4.0, 0.0), "polyline vertex 2");
        assert_at(corners[2], (4.0, 3.0), "polyline vertex 3");
        assert_eq!(moved.vertices[1].bulge, 0.5);
        assert!(moved.is_closed);

        let mut spline = Spline::from_control_points(
            3,
            vec![
                Vector3::new(120.5, -40.25, 0.0),
                Vector3::new(121.5, -39.25, 0.0),
                Vector3::new(122.5, -40.25, 0.0),
                Vector3::new(123.5, -39.25, 0.0),
            ],
        );
        spline.fit_points = vec![Vector3::new(122.0, -39.75, 0.0)];
        let knots = spline.knots.clone();
        let mut entity = EntityType::Spline(spline);
        to_block_space(&mut entity, origin);
        let EntityType::Spline(moved) = &entity else {
            panic!("a spline stays a spline");
        };
        let control: Vec<(f64, f64)> = moved.control_points.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(control.len(), 4);
        assert_at(control[0], (0.0, 0.0), "control point 1");
        assert_at(control[1], (1.0, 1.0), "control point 2");
        assert_at(control[2], (2.0, 0.0), "control point 3");
        assert_at(control[3], (3.0, 1.0), "control point 4");
        assert_at(
            (moved.fit_points[0].x, moved.fit_points[0].y),
            (1.5, 0.5),
            "fit point",
        );
        assert_eq!(moved.knots, knots);

        let mut text = Text::new();
        text.value = "FV-101".to_string();
        text.insertion_point = Vector3::new(126.5, -30.25, 0.0);
        text.alignment_point = Some(Vector3::new(127.5, -29.25, 0.0));
        text.height = 2.0;
        text.rotation = 0.3;
        let mut entity = EntityType::Text(text);
        to_block_space(&mut entity, origin);
        let EntityType::Text(moved) = &entity else {
            panic!("a text stays a text");
        };
        assert_at(
            (moved.insertion_point.x, moved.insertion_point.y),
            (6.0, 10.0),
            "text insertion",
        );
        let aligned = moved.alignment_point.expect("the alignment point is kept");
        assert_at((aligned.x, aligned.y), (7.0, 11.0), "text alignment point");
        assert_eq!(
            (moved.value.as_str(), moved.height, moved.rotation),
            ("FV-101", 2.0, 0.3)
        );

        let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
        path.edges.push(BoundaryEdge::Line(LineEdge {
            start: Vector2::new(120.5, -40.25),
            end: Vector2::new(125.5, -38.25),
        }));
        path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
            center: Vector2::new(122.5, -38.25),
            radius: 1.5,
            start_angle: 0.0,
            end_angle: std::f64::consts::TAU,
            counter_clockwise: true,
        }));
        let mut hatch = Hatch::new();
        hatch.paths.push(path);
        let mut entity = EntityType::Hatch(Box::new(hatch));
        to_block_space(&mut entity, origin);
        let EntityType::Hatch(moved) = &entity else {
            panic!("a hatch stays a hatch");
        };
        let BoundaryEdge::Line(edge) = &moved.paths[0].edges[0] else {
            panic!("the first edge is the line");
        };
        assert_at((edge.start.x, edge.start.y), (0.0, 0.0), "edge start");
        assert_at((edge.end.x, edge.end.y), (5.0, 2.0), "edge end");
        let BoundaryEdge::CircularArc(edge) = &moved.paths[0].edges[1] else {
            panic!("the second edge is the arc");
        };
        assert_at((edge.center.x, edge.center.y), (2.0, 2.0), "edge centre");
        assert_eq!(edge.radius, 1.5);
    }

    /// A fill with no pattern lines comes out of the move saying what it
    /// said going in: nothing when it stated no pattern origin, and the
    /// record it carried when it did. The origin a move writes on a hatch is
    /// the move, so left on, one symbol placed twice would be two drawings;
    /// without it they are one.
    #[test]
    fn a_solid_fill_is_the_same_member_wherever_its_placement_stands() {
        let origin = Vector3::new(120.5, -40.25, 0.0);

        let mut member = EntityType::Hatch(Box::new(fill((120.5, -40.25))));
        to_block_space(&mut member, origin);
        let EntityType::Hatch(moved) = &member else {
            panic!("a hatch stays a hatch");
        };
        let BoundaryEdge::Line(edge) = &moved.paths[0].edges[0] else {
            panic!("the first edge is a line");
        };
        assert_at((edge.start.x, edge.start.y), (-1.0, -1.0), "edge start");
        assert_eq!(moved.stored_pattern_origin(), None);
        assert!(
            moved.common.extended_data.is_empty(),
            "the move leaves no record on the fill: {:?}",
            moved.common.extended_data
        );

        let mut stated = fill((120.5, -40.25));
        assert!(stated.record_pattern_origin(Vector2::new(3.0, 4.0)));
        let said = stated.common.extended_data.clone();
        let mut member = EntityType::Hatch(Box::new(stated));
        to_block_space(&mut member, origin);
        assert_eq!(
            member.common().extended_data,
            said,
            "what a fill did state is left as it stated it"
        );

        let with_fill = |at: (f64, f64)| {
            let mut members = valve(at);
            members.push(EntityType::Hatch(Box::new(fill(at))));
            members
        };
        assert_eq!(
            print_at((10.1, 20.7), with_fill((10.1, 20.7))),
            print_at((310.3, 95.9), with_fill((310.3, 95.9)))
        );
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let first = blocks.place(
            &mut doc,
            Some("Ball Valve"),
            Vector3::new(10.1, 20.7, 0.0),
            with_fill((10.1, 20.7)),
        );
        let second = blocks.place(
            &mut doc,
            Some("Ball Valve"),
            Vector3::new(310.3, 95.9, 0.0),
            with_fill((310.3, 95.9)),
        );
        assert_eq!(second.block_name, first.block_name);
        assert_eq!((blocks.definitions, blocks.references), (1, 2));
    }

    /// A hatch that does have pattern lines keeps what the move does to it:
    /// the lines go with the area and the origin they start from is written
    /// down, so the pattern falls where it fell.
    #[test]
    fn a_patterned_fill_keeps_the_origin_its_lines_start_from() {
        let mut hatch = fill((120.5, -40.25));
        hatch.is_solid = false;
        hatch.pattern = HatchPattern::new("LINES");
        hatch.pattern.add_line(HatchPatternLine {
            angle: 0.0,
            base_point: Vector2::new(121.5, -39.25),
            offset: Vector2::new(0.0, 0.5),
            dash_lengths: Vec::new(),
        });
        let mut member = EntityType::Hatch(Box::new(hatch));
        to_block_space(&mut member, Vector3::new(120.5, -40.25, 0.0));
        let EntityType::Hatch(moved) = &member else {
            panic!("a hatch stays a hatch");
        };
        let base = moved.pattern.lines[0].base_point;
        assert_at((base.x, base.y), (1.0, 1.0), "the line's base point");
        let origin = moved
            .stored_pattern_origin()
            .expect("the origin the lines start from is stated");
        assert_at((origin.x, origin.y), (-120.5, 40.25), "pattern origin");
    }

    /// The same members give the same fingerprint, a placement elsewhere on
    /// the sheet gives it too, and so does one whose coordinates differ by
    /// less than the rounding -- but not one that differs by more.
    #[test]
    fn a_fingerprint_sees_the_drawing_and_not_where_it_stands_or_the_last_bits() {
        let here = print_at((10.1, 20.7), valve((10.1, 20.7)));
        assert_eq!(here, print_at((10.1, 20.7), valve((10.1, 20.7))));
        assert_eq!(here, print_at((310.3, 95.9), valve((310.3, 95.9))));

        let nudged = |by: f64| {
            let mut members = valve((10.1, 20.7));
            let EntityType::Line(first) = &mut members[0] else {
                panic!("the first member is a line");
            };
            first.end.x += by;
            print_at((10.1, 20.7), members)
        };
        assert_eq!(here, nudged(1e-8), "a hundredth of the rounding is noise");
        assert_ne!(here, nudged(1e-5), "ten times the rounding is a difference");
    }

    /// Everything a member is drawn with is in the fingerprint: change the
    /// colour, the weight, the linetype, the layer, the lettering or its
    /// height of one member and the drawing is another drawing.
    #[test]
    fn a_fingerprint_tells_apart_every_property_a_member_is_drawn_with() {
        let plain = print_at((10.0, 20.0), valve((10.0, 20.0)));
        type Change<'a> = &'a dyn Fn(&mut Vec<EntityType>);
        let changed = |change: Change<'_>| {
            let mut members = valve((10.0, 20.0));
            change(&mut members);
            print_at((10.0, 20.0), members)
        };
        let cases: [(&str, Change<'_>); 6] = [
            ("colour", &|members: &mut Vec<EntityType>| {
                members[0].common_mut().color = Color::from_rgb(255, 0, 0);
            }),
            ("line weight", &|members: &mut Vec<EntityType>| {
                members[0].common_mut().line_weight = LineWeight::Value(35);
            }),
            ("linetype", &|members: &mut Vec<EntityType>| {
                members[0].common_mut().linetype = "DASHED".to_string();
            }),
            ("layer", &|members: &mut Vec<EntityType>| {
                members[0].common_mut().layer = "PID-FILL".to_string();
            }),
            ("lettering", &|members: &mut Vec<EntityType>| {
                let EntityType::Text(text) = &mut members[3] else {
                    panic!("the last member is the name");
                };
                text.value = "Globe Valve".to_string();
            }),
            ("text height", &|members: &mut Vec<EntityType>| {
                let EntityType::Text(text) = &mut members[3] else {
                    panic!("the last member is the name");
                };
                text.height = 2.5;
            }),
        ];
        for (what, change) in cases {
            assert_ne!(plain, changed(change), "{what} is part of the drawing");
        }
    }

    /// Rounding touches decimals and nothing else: not an integer, not the
    /// digits of a type's name, and not a digit of anything quoted -- two
    /// texts that read `1.5` and `1.50000001` are two texts.
    #[test]
    fn rounding_rewrites_decimals_and_leaves_integers_names_and_quoted_text() {
        assert_eq!(
            round_decimals("Vector3 { x: 1.23456789, y: -0.25, z: 0.0 }"),
            "Vector3 { x: 1.234568, y: -0.250000, z: 0.000000 }"
        );
        assert_eq!(
            round_decimals("a: 1.7763568394002505e-15, b: -1e-7, c: 1.5e-5, d: 1e16"),
            "a: 0.000000, b: 0.000000, c: 0.000015, d: 10000000000000000.000000"
        );
        assert_eq!(
            round_decimals("x: -0.0, y: -0.0000004"),
            "x: 0.000000, y: 0.000000"
        );
        assert_eq!(
            round_decimals("Handle(255), Index(7), R2018, Face3D, 0..10, [1, 2, 3]"),
            "Handle(255), Index(7), R2018, Face3D, 0..10, [1, 2, 3]"
        );
        assert_eq!(
            round_decimals(r#"value: "DN 1.23456789 \"2.5\" \\", height: 2.5"#),
            r#"value: "DN 1.23456789 \"2.5\" \\", height: 2.500000"#
        );
        assert_eq!(round_decimals(""), "");

        let with_text = |value: &str| {
            let mut members = valve((10.0, 20.0));
            members.push(label(10.0, 15.0, value));
            print_at((10.0, 20.0), members)
        };
        assert_ne!(with_text("1.5"), with_text("1.50000001"));
    }

    /// The first drawing of a symbol is named after it, the next ones are
    /// numbered from 2, and a placement that names no symbol goes by
    /// `SPPID_SYMBOL`.
    #[test]
    fn a_block_is_named_after_its_symbol_and_other_drawings_of_it_are_numbered() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let origin = Vector3::new(10.0, 10.0, 0.0);
        let mut name_of = |symbol: Option<&str>, length: f64| {
            blocks
                .place(
                    &mut doc,
                    symbol,
                    origin,
                    vec![line(10.0, 10.0, 10.0 + length, 10.0)],
                )
                .block_name
        };
        assert_eq!(name_of(Some("Gate Valve"), 2.0), "SPPID_Gate Valve");
        assert_eq!(name_of(Some("Gate Valve"), 3.0), "SPPID_Gate Valve_2");
        assert_eq!(name_of(Some("Gate Valve"), 4.0), "SPPID_Gate Valve_3");
        assert_eq!(name_of(Some("Gate Valve"), 3.0), "SPPID_Gate Valve_2");
        assert_eq!(name_of(None, 2.0), "SPPID_SYMBOL");
        assert_eq!(name_of(None, 3.0), "SPPID_SYMBOL_2");
        assert_eq!(name_of(Some("  "), 2.0), "SPPID_SYMBOL");
        assert_eq!(name_of(Some("  Check Valve "), 2.0), "SPPID_Check Valve");
    }

    /// A name the document already holds -- in any case -- is not taken a
    /// second time, whoever holds it: a block that was there before, or
    /// another symbol whose name comes out the same once the characters a
    /// block name cannot hold are replaced. The name stays within what a
    /// table name may be.
    #[test]
    fn a_block_name_is_made_fit_for_the_table_and_never_taken_twice() {
        let mut doc = Document::new();
        doc.block_records
            .add(BlockRecord::new("sppid_check valve"))
            .expect("a free name");
        let mut blocks = SymbolBlocks::default();
        let origin = Vector3::new(10.0, 10.0, 0.0);
        let mut name_of = |symbol: &str, length: f64| {
            blocks
                .place(
                    &mut doc,
                    Some(symbol),
                    origin,
                    vec![line(10.0, 10.0, 10.0 + length, 10.0)],
                )
                .block_name
        };
        assert_eq!(name_of("Check Valve", 2.0), "SPPID_Check Valve_2");
        assert_eq!(name_of("Check Valve", 3.0), "SPPID_Check Valve_3");

        assert_eq!(name_of("Valve <2\"> a/b", 2.0), "SPPID_Valve _2__ a_b");
        assert_eq!(name_of("Valve [2'] a:b", 2.0), "SPPID_Valve [2'] a_b");
        assert_eq!(name_of("Valve [2'] a?b", 2.0), "SPPID_Valve [2'] a_b_2");

        let long = "N".repeat(300);
        let first = name_of(&long, 2.0);
        let second = name_of(&long, 3.0);
        assert_eq!(first, format!("SPPID_{}", "N".repeat(BASE_NAME_CHARS - 6)));
        assert_eq!(second, format!("{first}_2"));
        assert!(second.chars().count() <= 255);

        let held = doc
            .block_records
            .get("sppid_check valve")
            .expect("still there");
        assert_eq!(held.name, "sppid_check valve");
        assert!(
            held.entity_handles.is_empty(),
            "a block that was there is left alone"
        );
    }

    /// Two placements that draw the same thing refer to one definition, and
    /// the members are filed once. The same strokes under another symbol's
    /// name are that symbol's own block.
    #[test]
    fn placements_that_draw_the_same_thing_share_one_definition() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let records = doc.block_records.len();
        let entities = doc.entities().count();

        let first = blocks.place(
            &mut doc,
            Some("Gate Valve"),
            Vector3::new(10.1, 20.7, 0.0),
            valve((10.1, 20.7)),
        );
        let second = blocks.place(
            &mut doc,
            Some("Gate Valve"),
            Vector3::new(310.3, 95.9, 0.0),
            valve((310.3, 95.9)),
        );
        assert_eq!(first.block_name, "SPPID_Gate Valve");
        assert_eq!(second.block_name, first.block_name);
        assert_eq!(doc.block_records.len(), records + 1);
        assert_eq!(doc.entities_in_block(&first.block_name).count(), 4);
        assert_eq!(
            doc.entities().count(),
            entities + 4,
            "the second placement added nothing"
        );
        assert_eq!((blocks.definitions, blocks.references), (1, 2));

        let other = blocks.place(
            &mut doc,
            Some("Globe Valve"),
            Vector3::new(50.0, 50.0, 0.0),
            valve((50.0, 50.0)),
        );
        assert_eq!(other.block_name, "SPPID_Globe Valve");
        assert_eq!((blocks.definitions, blocks.references), (2, 3));
    }

    /// A placement that draws something else -- here the same body with one
    /// stroke longer -- gets a definition of its own under the next number.
    #[test]
    fn a_placement_that_draws_something_else_gets_its_own_definition() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let records = doc.block_records.len();
        let origin = Vector3::new(10.0, 20.0, 0.0);

        let first = blocks.place(&mut doc, Some("Gate Valve"), origin, valve((10.0, 20.0)));
        let mut longer = valve((10.0, 20.0));
        let EntityType::Line(stroke) = &mut longer[0] else {
            panic!("the first member is a line");
        };
        stroke.end.x += 1.0;
        let second = blocks.place(&mut doc, Some("Gate Valve"), origin, longer);

        assert_eq!(first.block_name, "SPPID_Gate Valve");
        assert_eq!(second.block_name, "SPPID_Gate Valve_2");
        assert_eq!(doc.block_records.len(), records + 2);
        assert_eq!(doc.entities_in_block(&first.block_name).count(), 4);
        assert_eq!(doc.entities_in_block(&second.block_name).count(), 4);
        assert_eq!((blocks.definitions, blocks.references), (2, 2));
    }

    /// A definition is a record, the BLOCK the record names, the members
    /// owned by the record and the ENDBLK the record names, with handles in
    /// that order -- and none of it is in model space.
    #[test]
    fn a_definition_is_a_record_its_block_its_members_and_its_endblk_in_that_order() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let in_model_space = doc.model_space_entities().count();
        let reference = blocks.place(
            &mut doc,
            Some("Gate Valve"),
            Vector3::new(50.0, 60.0, 0.0),
            valve((50.0, 60.0)),
        );

        let record = doc
            .block_records
            .get(&reference.block_name)
            .expect("the record is in the table");
        assert!(!record.handle.is_null());
        let Some(EntityType::Block(block)) = doc.get_entity(record.block_entity_handle) else {
            panic!("the record names its BLOCK");
        };
        assert_eq!(block.name, reference.block_name);
        assert_eq!(block.common.owner_handle, record.handle);
        assert_eq!(block.base_point, Vector3::ZERO);
        let Some(EntityType::BlockEnd(end)) = doc.get_entity(record.block_end_handle) else {
            panic!("the record names its ENDBLK");
        };
        assert_eq!(end.common.owner_handle, record.handle);

        assert_eq!(record.entity_handles.len(), 4);
        let kinds: Vec<&str> = record
            .entity_handles
            .iter()
            .map(|handle| {
                let member = doc
                    .get_entity(*handle)
                    .expect("a member is in the document");
                assert_eq!(member.common().owner_handle, record.handle);
                assert!(
                    record.block_entity_handle < *handle && *handle < record.block_end_handle,
                    "BLOCK {:?}, member {handle:?}, ENDBLK {:?}",
                    record.block_entity_handle,
                    record.block_end_handle
                );
                member.as_entity().entity_type()
            })
            .collect();
        assert_eq!(kinds, ["LINE", "LINE", "CIRCLE", "TEXT"]);
        assert_eq!(doc.entities_in_block(&reference.block_name).count(), 4);
        assert_eq!(
            doc.model_space_entities().count(),
            in_model_space,
            "nothing of a definition is in model space"
        );
    }

    /// The reference stands at the insertion point, unrotated and unscaled,
    /// goes into model space when it is added, and draws each member where
    /// the placement drew it: member plus insertion is the original.
    #[test]
    fn the_reference_draws_the_members_where_the_placement_drew_them() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let in_model_space = doc.model_space_entities().count();
        let origin = Vector3::new(310.3, 95.9, 0.0);
        let drawn = valve((310.3, 95.9));
        let reference = blocks.place(&mut doc, Some("Gate Valve"), origin, drawn.clone());

        assert_eq!(reference.insert_point, origin);
        assert_eq!(
            (
                reference.x_scale(),
                reference.y_scale(),
                reference.z_scale(),
                reference.rotation
            ),
            (1.0, 1.0, 1.0, 0.0)
        );
        let name = reference.block_name.clone();
        let handle = doc
            .add_entity(EntityType::Insert(reference))
            .expect("the reference is added");
        let last: Option<Handle> = doc
            .model_space_entities()
            .last()
            .map(|entity| entity.common().handle);
        assert_eq!(last, Some(handle), "the reference is in model space");
        assert_eq!(doc.model_space_entities().count(), in_model_space + 1);

        let members: Vec<&EntityType> = doc.entities_in_block(&name).collect();
        assert_eq!(members.len(), drawn.len());
        let (EntityType::Line(member), EntityType::Line(original)) = (members[0], &drawn[0]) else {
            panic!("the first member is the first stroke");
        };
        assert_at(
            (member.start.x + origin.x, member.start.y + origin.y),
            (original.start.x, original.start.y),
            "stroke start",
        );
        assert_at(
            (member.end.x + origin.x, member.end.y + origin.y),
            (original.end.x, original.end.y),
            "stroke end",
        );
        let (EntityType::Circle(member), EntityType::Circle(original)) = (members[2], &drawn[2])
        else {
            panic!("the third member is the ring");
        };
        assert_at(
            (member.center.x + origin.x, member.center.y + origin.y),
            (original.center.x, original.center.y),
            "ring centre",
        );
        assert_eq!(member.radius, original.radius);
    }

    /// Written as a DWG and read back, the definitions, their members and
    /// the references are all there.
    #[test]
    fn symbol_blocks_survive_a_dwg_round_trip() {
        let mut doc = Document::new();
        let mut blocks = SymbolBlocks::default();
        let placements = [
            ("Gate Valve", (10.5, 20.5), 0.0),
            ("Gate Valve", (300.25, 90.75), 0.0),
            ("Gate Valve", (40.0, 60.0), 1.0),
            ("Globe Valve", (80.0, 20.0), 0.0),
        ];
        for (symbol, at, longer) in placements {
            let mut members = valve(at);
            let EntityType::Line(stroke) = &mut members[0] else {
                panic!("the first member is a line");
            };
            stroke.end.x += longer;
            let reference = blocks.place(
                &mut doc,
                Some(symbol),
                Vector3::new(at.0, at.1, 0.0),
                members,
            );
            doc.add_entity(EntityType::Insert(reference))
                .expect("the reference is added");
        }
        assert_eq!((blocks.definitions, blocks.references), (3, 4));

        let dir = std::env::temp_dir().join("ocs_pid_symbol_blocks_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("round_trip_{}.dwg", std::process::id()));
        document::DwgWriter::write_to_file(&path, &doc).unwrap();
        let mut reader = document::DwgReader::from_file(&path).unwrap();
        let reopened = reader.read().unwrap();
        std::fs::remove_file(&path).ok();

        for name in [
            "SPPID_Gate Valve",
            "SPPID_Gate Valve_2",
            "SPPID_Globe Valve",
        ] {
            let kinds: Vec<&str> = reopened
                .entities_in_block(name)
                .map(|member| member.as_entity().entity_type())
                .collect();
            assert_eq!(kinds, ["LINE", "LINE", "CIRCLE", "TEXT"], "{name}");
        }
        let references: Vec<(String, (f64, f64))> = reopened
            .model_space_entities()
            .filter_map(|entity| match entity {
                EntityType::Insert(insert) => Some((
                    insert.block_name.clone(),
                    (insert.insert_point.x, insert.insert_point.y),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            references,
            [
                ("SPPID_Gate Valve".to_string(), (10.5, 20.5)),
                ("SPPID_Gate Valve".to_string(), (300.25, 90.75)),
                ("SPPID_Gate Valve_2".to_string(), (40.0, 60.0)),
                ("SPPID_Globe Valve".to_string(), (80.0, 20.0)),
            ]
        );
        assert_eq!(reopened.model_space_entities().count(), 4);
    }
}

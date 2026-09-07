//! `io::pid_legend` against a sheet built in memory and against the real CPECC
//! sheets when they are on this machine.
//!
//! The real sheets live beside the OCS checkout
//! (`../0版重新处理dxf-12张/`); those tests soft-skip when the folder is not
//! there, the in-memory one always runs.

use std::collections::BTreeSet;
use std::path::PathBuf;

use acadrust::entities::{Block, BlockEnd, Circle, Insert, Line, Point, Text};
use acadrust::tables::BlockRecord;
use acadrust::types::{Color, Vector3};
use acadrust::{CadDocument, EntityType, Handle};
use OpenCADStudio::io;
use OpenCADStudio::io::pid_legend::{self, Rules};
use OpenCADStudio::io::pid_pipes;

// ── in-memory sheet ─────────────────────────────────────────────────────

/// Define `name` as a block whose body is `members` (block-local, base at the
/// origin), the way the scene's own block definer does it.
fn define_block(doc: &mut CadDocument, name: &str, members: Vec<EntityType>) {
    let next = doc.next_handle();
    let br_handle = Handle::new(next);
    let block_handle = Handle::new(next + 1);
    let end_handle = Handle::new(next + 2);
    let mut record = BlockRecord::new(name);
    record.handle = br_handle;
    record.block_entity_handle = block_handle;
    record.block_end_handle = end_handle;
    doc.block_records.add(record).unwrap();
    let mut block = Block::new(name, Vector3::ZERO);
    block.common.handle = block_handle;
    block.common.owner_handle = br_handle;
    doc.add_entity(EntityType::Block(block)).unwrap();
    let mut end = BlockEnd::new();
    end.common.handle = end_handle;
    end.common.owner_handle = br_handle;
    doc.add_entity(EntityType::BlockEnd(end)).unwrap();
    for mut member in members {
        member.common_mut().owner_handle = br_handle;
        doc.add_entity(member).unwrap();
    }
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> EntityType {
    EntityType::Line(Line::from_points(
        Vector3::new(x0, y0, 0.0),
        Vector3::new(x1, y1, 0.0),
    ))
}

fn point(x: f64, y: f64) -> EntityType {
    let mut p = Point::new();
    p.location = Vector3::new(x, y, 0.0);
    EntityType::Point(p)
}

fn text(value: &str, x: f64, y: f64) -> EntityType {
    EntityType::Text(Text::with_value(value, Vector3::new(x, y, 0.0)).with_height(2.0))
}

fn circle(x: f64, y: f64, r: f64) -> EntityType {
    let mut c = Circle::new();
    c.center = Vector3::new(x, y, 0.0);
    c.radius = r;
    EntityType::Circle(c)
}

fn insert(name: &str, x: f64, y: f64, layer: &str) -> EntityType {
    let mut i = Insert::new(name, Vector3::new(x, y, 0.0));
    i.common.layer = layer.to_string();
    EntityType::Insert(i)
}

/// A sheet in paper millimetres: two butterfly valves 9 mm apart with their
/// two BUV tags between them (the ambiguity one-to-one pairing must
/// resolve), a motorised valve with an XV bubble, a panel-mounted HS bubble
/// in its square, a tank, and a block id the rules have never met.
fn synthetic_sheet() -> CadDocument {
    let mut doc = CadDocument::new();
    define_block(
        &mut doc,
        "$VALVE$00000316",
        vec![
            line(-1.25, -0.72, 1.25, -0.72),
            line(1.25, -0.72, 1.25, 0.72),
            line(1.25, 0.72, -1.25, 0.72),
            line(-1.25, 0.72, -1.25, -0.72),
            line(-1.25, -0.72, 1.25, 0.72),
            point(-1.25, 0.0),
            point(1.25, 0.0),
        ],
    );
    define_block(
        &mut doc,
        "$EVALVE$00000018",
        vec![
            line(-1.25, -0.6, 1.25, 0.6),
            line(-1.25, 0.6, 1.25, -0.6),
            line(0.0, 0.0, 0.0, 2.2),
            point(-1.25, 0.0),
            point(1.25, 0.0),
        ],
    );
    define_block(
        &mut doc,
        "$VALVE$00000999",
        vec![line(-1.0, -0.5, 1.0, 0.5), line(-1.0, 0.5, 1.0, -0.5)],
    );

    // Two butterfly valves on a vertical run, tags lettered between them.
    doc.add_entity(insert("$VALVE$00000316", 10.0, 10.0, "VALVE_消防"))
        .unwrap();
    doc.add_entity(insert("$VALVE$00000316", 10.0, 19.0, "VALVE_消防"))
        .unwrap();
    doc.add_entity(text("BUV-3101", 12.0, 13.0)).unwrap();
    doc.add_entity(text("BUV-3102", 12.0, 16.0)).unwrap();
    // Sizes lettered near a valve must not be taken for tags.
    doc.add_entity(text("1/2\"NPT", 8.0, 9.0)).unwrap();

    // Motorised valve, its XV bubble 6 mm above it.
    doc.add_entity(insert("$EVALVE$00000018", 30.0, 4.0, "EVALVE_消防"))
        .unwrap();
    doc.add_entity(circle(30.0, 10.0, 3.85)).unwrap();
    doc.add_entity(text("XV", 30.0, 11.0)).unwrap();
    doc.add_entity(text("3101", 30.0, 9.0)).unwrap();

    // Panel-mounted HS bubble: circle in a square of loose lines.
    doc.add_entity(circle(50.0, 10.0, 3.7)).unwrap();
    doc.add_entity(text("HS", 50.0, 11.0)).unwrap();
    doc.add_entity(text("3101", 50.0, 9.0)).unwrap();
    doc.add_entity(line(46.3, 6.3, 53.7, 6.3)).unwrap();
    doc.add_entity(line(53.7, 6.3, 53.7, 13.7)).unwrap();
    doc.add_entity(line(53.7, 13.7, 46.3, 13.7)).unwrap();
    doc.add_entity(line(46.3, 13.7, 46.3, 6.3)).unwrap();

    // A tank with its equipment number inside.
    doc.add_entity(circle(90.0, 40.0, 13.4)).unwrap();
    doc.add_entity(text("TD-0201", 90.0, 41.0)).unwrap();
    doc.add_entity(text("5000m", 90.0, 38.0)).unwrap();

    // A block id the rules do not know, on a valve layer.
    doc.add_entity(insert("$VALVE$00000999", 70.0, 20.0, "VALVE_雨水"))
        .unwrap();

    // Sheet frame so the extent is a real A3 in paper millimetres.
    doc.add_entity(line(0.0, 0.0, 420.0, 0.0)).unwrap();
    doc.add_entity(line(0.0, 0.0, 0.0, 297.0)).unwrap();
    doc
}

fn tags_of<'a>(recognition: &'a pid_legend::Recognition, class: &str) -> Vec<&'a str> {
    let mut tags: Vec<&str> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == class)
        .filter_map(|s| s.tag.as_deref())
        .collect();
    tags.sort_unstable();
    tags
}

fn count(recognition: &pid_legend::Recognition, class: &str) -> usize {
    recognition
        .symbols
        .iter()
        .filter(|s| s.class == class)
        .count()
}

#[test]
fn synthetic_sheet_is_recognised_and_tagged_one_to_one() {
    let doc = synthetic_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);

    assert_eq!(recognition.units_per_mm, 1.0, "an A3 in paper mm");
    assert_eq!(count(&recognition, "butterfly"), 2);
    assert_eq!(tags_of(&recognition, "butterfly"), ["BUV-3101", "BUV-3102"]);
    // The lower valve is nearer BUV-3101, the upper one nearer BUV-3102.
    let lower = recognition
        .symbols
        .iter()
        .find(|s| s.class == "butterfly" && s.at.1 == 10.0)
        .unwrap();
    assert_eq!(lower.tag.as_deref(), Some("BUV-3101"));
    assert!(lower.tag_distance_mm.unwrap() < 4.0);
    // Its box is the placed body plus nothing else: 2.5 x 1.44 mm about (10, 10).
    assert!((lower.bbox.0 - 8.75).abs() < 0.01 && (lower.bbox.2 - 11.25).abs() < 0.01);
    assert!((lower.bbox.1 - 9.28).abs() < 0.01 && (lower.bbox.3 - 10.72).abs() < 0.01);

    assert_eq!(count(&recognition, "bubble"), 1);
    assert_eq!(tags_of(&recognition, "bubble"), ["XV-3101"]);
    assert_eq!(count(&recognition, "evalve"), 1);
    assert_eq!(tags_of(&recognition, "evalve"), ["XV-3101"]);
    assert_eq!(count(&recognition, "bubble-panel"), 1);
    assert_eq!(tags_of(&recognition, "bubble-panel"), ["HS-3101"]);
    assert_eq!(count(&recognition, "tank"), 1);
    assert_eq!(tags_of(&recognition, "tank"), ["TD-0201"]);

    // The unknown id is still boxed, classified by its layer, and reported.
    assert_eq!(count(&recognition, "valve-unknown"), 1);
    let unknown = recognition
        .symbols
        .iter()
        .find(|s| s.class == "valve-unknown")
        .unwrap();
    assert!(!unknown.known);
    assert_eq!(unknown.color, [255, 255, 255]);
    assert_eq!(
        recognition
            .unknown_blocks
            .get("$VALVE$00000999 on VALVE_雨水"),
        Some(&1)
    );
    assert!(
        recognition.orphan_tags.is_empty(),
        "{:?}",
        recognition.orphan_tags
    );
    assert_eq!(recognition.symbols.len(), 7);

    let lines = pid_legend::report(&recognition);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("蝶阀 (butterfly) x2  tagged 2/2")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("UNKNOWN BLOCK $VALVE$00000999")),
        "{lines:?}"
    );
}

#[test]
fn legend_entities_go_on_coloured_class_layers_and_come_off_again() {
    let mut doc = synthetic_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    let before = doc.entity_count();

    let added = pid_legend::apply(&mut doc, &recognition, &rules);
    assert_eq!(
        added,
        recognition.symbols.len() * 2,
        "a rectangle and a label each"
    );
    assert_eq!(doc.entity_count(), before + added);

    let butterfly_layer = doc
        .layers
        .get("PID-LEGEND-BUTTERFLY")
        .expect("class layer created");
    assert_eq!(
        butterfly_layer.color,
        Color::Rgb {
            r: 255,
            g: 140,
            b: 0
        },
        "layer colour is the class colour"
    );
    assert!(doc.layers.get("PID-LEGEND-VALVE-UNKNOWN").is_some());

    let legend: Vec<&EntityType> = doc
        .model_space_entities()
        .filter(|e| pid_legend::is_legend_layer(&e.common().layer))
        .collect();
    assert_eq!(legend.len(), added);
    let rectangles = legend
        .iter()
        .filter(|e| matches!(e, EntityType::LwPolyline(p) if p.is_closed && p.vertices.len() == 4))
        .count();
    assert_eq!(rectangles, recognition.symbols.len());
    let labels: BTreeSet<String> = legend
        .iter()
        .filter_map(|e| match e {
            EntityType::Text(t) => Some(t.value.clone()),
            _ => None,
        })
        .collect();
    assert!(labels.contains("蝶阀 BUV-3101"), "{labels:?}");
    assert!(labels.contains("电动阀 XV-3101"), "{labels:?}");
    assert!(labels.contains("盘装仪表 HS-3101"), "{labels:?}");
    assert!(labels.contains("储罐 TD-0201"), "{labels:?}");
    assert!(labels.contains("未知阀 $VALVE$00000999"), "{labels:?}");
    // Legend entities draw ByLayer, so recolouring the layer recolours them.
    assert!(legend.iter().all(|e| e.common().color == Color::ByLayer));

    let removed = pid_legend::clear(&mut doc);
    assert_eq!(removed, added);
    assert_eq!(doc.entity_count(), before);
    assert!(pid_legend::legend_handles(&doc).is_empty());
}

// ── piped block sheet ───────────────────────────────────────────────────

fn insert_turned(name: &str, x: f64, y: f64, layer: &str, rotation: f64) -> EntityType {
    let mut i = Insert::new(name, Vector3::new(x, y, 0.0));
    i.common.layer = layer.to_string();
    i.rotation = rotation;
    EntityType::Insert(i)
}

/// A header on `PIPE-消防` from an open end at x = 10 to a sheet connector at
/// x = 60, lettered `100-FW`; a vent stub off it at x = 20 ending in the air;
/// at x = 30 and x = 45 a branch down through a butterfly valve (turned a
/// quarter, so its connection points face up and down) to an S point --
/// BUV-3101's branch lettered `80-FW` below the valve, BUV-3102's lettered
/// nowhere.
fn piped_block_sheet() -> CadDocument {
    let mut doc = CadDocument::new();
    define_block(
        &mut doc,
        "$VALVE$00000316",
        vec![
            line(-1.25, -0.72, 1.25, -0.72),
            line(1.25, -0.72, 1.25, 0.72),
            line(1.25, 0.72, -1.25, 0.72),
            line(-1.25, 0.72, -1.25, -0.72),
            line(-1.25, -0.72, 1.25, 0.72),
            point(-1.25, 0.0),
            point(1.25, 0.0),
        ],
    );
    define_block(
        &mut doc,
        "$TwtSys$00000132",
        vec![line(0.0, 0.0, 1.0, 1.0), line(0.0, 0.0, 1.0, -1.0)],
    );
    doc.add_entity(line(0.0, 0.0, 420.0, 0.0)).unwrap();
    doc.add_entity(line(0.0, 0.0, 0.0, 297.0)).unwrap();

    let pipe = |doc: &mut CadDocument, x0: f64, y0: f64, x1: f64, y1: f64| {
        doc.add_entity(layered(line(x0, y0, x1, y1), "PIPE-消防"))
            .unwrap();
    };
    pipe(&mut doc, 10.0, 50.0, 30.0, 50.0);
    pipe(&mut doc, 30.0, 50.0, 45.0, 50.0);
    pipe(&mut doc, 45.0, 50.0, 60.0, 50.0);
    pipe(&mut doc, 20.0, 50.0, 20.0, 53.0);
    doc.add_entity(insert("$TwtSys$00000132", 60.0, 50.0, "EQUIP_消防"))
        .unwrap();
    doc.add_entity(text("100-FW", 37.0, 51.5)).unwrap();
    for (x, tag) in [(30.0, "BUV-3101"), (45.0, "BUV-3102")] {
        pipe(&mut doc, x, 50.0, x, 41.25);
        doc.add_entity(insert_turned(
            "$VALVE$00000316",
            x,
            40.0,
            "VALVE_消防",
            std::f64::consts::FRAC_PI_2,
        ))
        .unwrap();
        doc.add_entity(text(tag, x + 2.0, 40.5)).unwrap();
        pipe(&mut doc, x, 38.75, x, 30.0);
        doc.add_entity(circle(x, 27.9, 2.1)).unwrap();
        doc.add_entity(text("S", x, 27.9)).unwrap();
    }
    doc.add_entity(text("80-FW", 31.5, 34.0)).unwrap();
    doc
}

/// Pipe strokes join into runs cut at the valves' connection points, at tees
/// and at open ends; a line number goes to the run it is lettered along and
/// on to the runs that continue it straight through a tee or through a valve,
/// not to a branch; each symbol knows the lines at its connection points.
#[test]
fn pipe_runs_join_symbols_and_carry_the_line_number_lettered_along_them() {
    let doc = piped_block_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    assert_eq!(recognition.units_per_mm, 1.0);
    let pipes = &recognition.pipes;
    assert_eq!(pipes.segments, 8);
    // Header in four pieces (open end, vent tee, two branch tees, connector),
    // the vent, two pieces per branch.
    assert_eq!(pipes.runs.len(), 9, "{:?}", pipes.runs);
    assert_eq!((pipes.connected_ports, pipes.ports), (7, 7));
    assert_eq!(pipes.open_ends, 2);

    let symbol = |tag: &str| {
        recognition
            .symbols
            .iter()
            .position(|s| s.tag.as_deref() == Some(tag))
            .unwrap_or_else(|| panic!("no symbol tagged {tag}"))
    };
    let (v1, v2) = (symbol("BUV-3101"), symbol("BUV-3102"));
    let connector = recognition
        .symbols
        .iter()
        .position(|s| s.source == "$TwtSys$00000132")
        .expect("the sheet connector");
    let run_between = |a: pid_pipes::End, b: pid_pipes::End| {
        pipes
            .runs
            .iter()
            .find(|r| r.ends == [a, b] || r.ends == [b, a])
            .unwrap_or_else(|| panic!("no run {a:?} -- {b:?} in {:?}", pipes.runs))
    };
    use pid_pipes::End::{Open, Symbol, Tee};
    // The header: lettered on its middle piece, the same line straight through
    // both tees to the open end and to the connector.
    let middle = run_between(Tee, Tee);
    assert_eq!(middle.numbers, ["100-FW"]);
    assert!((middle.length_mm - 15.0).abs() < 0.01, "{middle:?}");
    let last = run_between(Tee, Symbol(connector));
    assert!(
        last.numbers.is_empty() && last.lines == ["100-FW"],
        "{last:?}"
    );
    assert_eq!(recognition.symbols[connector].lines, ["100-FW"]);
    let mut header_open: Vec<&pid_pipes::Run> = pipes
        .runs
        .iter()
        .filter(|r| r.ends.contains(&Open))
        .collect();
    header_open.sort_by(|a, b| a.length_mm.total_cmp(&b.length_mm));
    assert_eq!(header_open.len(), 2);
    assert!(
        header_open[0].lines.is_empty() && (header_open[0].length_mm - 3.0).abs() < 0.01,
        "the vent stub is a branch and carries no line: {:?}",
        header_open[0]
    );
    assert_eq!(header_open[1].lines, ["100-FW"], "{:?}", header_open[1]);
    // BUV-3101: 80-FW lettered below the valve, carried up through the valve
    // to the piece above it -- but not on to the header, which it branches
    // off.
    let below = run_between(Symbol(v1), Symbol(symbol_s(&recognition, 30.0)));
    assert_eq!(below.numbers, ["80-FW"]);
    let above = run_between(Tee, Symbol(v1));
    assert!(
        above.numbers.is_empty() && above.lines == ["80-FW"],
        "{above:?}"
    );
    assert_eq!(recognition.symbols[v1].lines, ["80-FW"]);
    assert_eq!(
        recognition.symbols[symbol_s(&recognition, 30.0)].lines,
        ["80-FW"]
    );
    // BUV-3102: nothing lettered on its branch, so no line -- the header's
    // number does not turn down a branch.
    let above2 = run_between(Tee, Symbol(v2));
    assert!(above2.lines.is_empty(), "{above2:?}");
    assert!(recognition.symbols[v2].lines.is_empty());
    assert!(recognition.symbols[symbol_s(&recognition, 45.0)]
        .lines
        .is_empty());
}

/// The runs are drawn in with the legend: a polyline per run on a layer per
/// line number in a colour of its own, the runs on no line in grey, a ring
/// at every open end -- and `clear` takes them off with the rectangles.
#[test]
fn pipe_runs_are_drawn_on_a_layer_per_line_number_and_come_off_again() {
    let mut doc = piped_block_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    let before = doc.entity_count();
    let added = pid_legend::apply(&mut doc, &recognition, &rules);
    // A rectangle and a label per symbol, a polyline per run (none carries
    // two lines here), a ring at each of the two open ends.
    assert_eq!(added, recognition.symbols.len() * 2 + 9 + 2);

    let on = |doc: &CadDocument, layer: &str| -> Vec<EntityType> {
        doc.model_space_entities()
            .filter(|e| e.common().layer == layer)
            .cloned()
            .collect()
    };
    let open_polylines = |es: &[EntityType]| {
        es.iter()
            .filter(|e| matches!(e, EntityType::LwPolyline(p) if !p.is_closed))
            .count()
    };
    fn rings(es: &[EntityType]) -> Vec<&Circle> {
        es.iter()
            .filter_map(|e| match e {
                EntityType::Circle(c) => Some(c),
                _ => None,
            })
            .collect()
    }
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;

    // 100-FW: the header's four pieces, ringed where it starts in the air
    // at x = 10; the middle piece runs tee to tee, x = 30 to x = 45.
    let header = on(&doc, "PID-PIPE-100-FW");
    assert_eq!(open_polylines(&header), 4, "{header:?}");
    let header_rings = rings(&header);
    assert_eq!(header_rings.len(), 1);
    assert!(
        near(header_rings[0].center.x, 10.0)
            && near(header_rings[0].center.y, 50.0)
            && near(header_rings[0].radius, 0.6),
        "{:?}",
        header_rings[0]
    );
    assert!(header.iter().any(|e| match e {
        EntityType::LwPolyline(p) => {
            let mut xs: Vec<f64> = p.vertices.iter().map(|v| v.location.x).collect();
            xs.sort_by(f64::total_cmp);
            xs.len() == 2 && near(xs[0], 30.0) && near(xs[1], 45.0)
        }
        _ => false,
    }));
    // 80-FW: the two pieces either side of BUV-3101, no open end.
    let branch = on(&doc, "PID-PIPE-80-FW");
    assert_eq!(open_polylines(&branch), 2, "{branch:?}");
    assert!(rings(&branch).is_empty());
    // No line: the vent stub, ringed where it stops at (20, 53), and both
    // pieces of BUV-3102's unlettered branch; grey.
    let none = on(&doc, pid_legend::PIPE_NONE_LAYER);
    assert_eq!(open_polylines(&none), 3, "{none:?}");
    let none_rings = rings(&none);
    assert_eq!(none_rings.len(), 1);
    assert!(near(none_rings[0].center.x, 20.0) && near(none_rings[0].center.y, 53.0));
    let colour = |name: &str| {
        doc.layers
            .get(name)
            .unwrap_or_else(|| panic!("{name}"))
            .color
    };
    assert_eq!(
        colour(pid_legend::PIPE_NONE_LAYER),
        Color::Rgb {
            r: 128,
            g: 128,
            b: 128
        }
    );
    assert_ne!(colour("PID-PIPE-100-FW"), colour("PID-PIPE-80-FW"));
    assert_ne!(
        colour("PID-PIPE-100-FW"),
        colour(pid_legend::PIPE_NONE_LAYER)
    );
    // Every run and ring is there, all ByLayer.
    let pipe_entities: Vec<&EntityType> = doc
        .model_space_entities()
        .filter(|e| e.common().layer.starts_with(pid_legend::PIPE_LAYER_PREFIX))
        .collect();
    assert_eq!(pipe_entities.len(), 11);
    assert!(pipe_entities
        .iter()
        .all(|e| e.common().color == Color::ByLayer));

    assert_eq!(pid_legend::clear(&mut doc), added);
    assert_eq!(doc.entity_count(), before);
    assert!(doc
        .model_space_entities()
        .all(|e| !pid_legend::is_legend_layer(&e.common().layer)));
}

/// The S point (circle) at x on the piped block sheet.
fn symbol_s(recognition: &pid_legend::Recognition, x: f64) -> usize {
    recognition
        .symbols
        .iter()
        .position(|s| s.source.starts_with("circle") && (s.at.0 - x).abs() < 0.01)
        .unwrap_or_else(|| panic!("no S point at x = {x}"))
}

// ── exploded sheet ──────────────────────────────────────────────────────

fn layered(mut entity: EntityType, layer: &str) -> EntityType {
    entity.common_mut().layer = layer.to_string();
    entity
}

/// A bowtie valve of four touching lines, 2.4 x 1.2 mm about (x, y), turned
/// by `turn`.
fn bowtie(doc: &mut CadDocument, x: f64, y: f64, turn: fn((f64, f64)) -> (f64, f64)) {
    let p = |dx: f64, dy: f64| {
        let (tx, ty) = turn((dx, dy));
        (x + tx, y + ty)
    };
    for (a, b) in [
        (p(-1.2, -0.6), p(-1.2, 0.6)),
        (p(-1.2, 0.6), p(1.2, -0.6)),
        (p(1.2, -0.6), p(1.2, 0.6)),
        (p(1.2, 0.6), p(-1.2, -0.6)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
}

/// A ball valve the way the loading-island sheets draw one: two uprights
/// 2.8 mm apart, a 0.4 mm ball in the middle, and four half-diagonals from
/// the corners to the ball's rim. Seven strokes about (x, y), turned by
/// `turn`.
fn ball_valve(doc: &mut CadDocument, x: f64, y: f64, turn: fn((f64, f64)) -> (f64, f64)) {
    let p = |dx: f64, dy: f64| {
        let (tx, ty) = turn((dx, dy));
        (x + tx, y + ty)
    };
    // A corner is 1.565 mm from the centre; the rim point on the way to it
    // is 0.4 mm out along the same direction.
    let (rx, ry) = (0.4 * 1.4 / 1.565, 0.4 * 0.7 / 1.565);
    for (a, b) in [
        (p(-1.4, -0.7), p(-1.4, 0.7)),
        (p(1.4, -0.7), p(1.4, 0.7)),
        (p(-1.4, -0.7), p(-rx, -ry)),
        (p(-1.4, 0.7), p(-rx, ry)),
        (p(1.4, -0.7), p(rx, -ry)),
        (p(1.4, 0.7), p(rx, ry)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
    doc.add_entity(layered(circle(x, y, 0.4), "DEVICE"))
        .unwrap();
}

/// A triangle of three touching lines, 2 mm, about (x, y).
fn triangle(doc: &mut CadDocument, x: f64, y: f64) {
    for (a, b) in [
        ((x - 1.0, y - 1.0), (x + 1.0, y - 1.0)),
        ((x + 1.0, y - 1.0), (x, y + 1.0)),
        ((x, y + 1.0), (x - 1.0, y - 1.0)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
}

/// A sheet in paper millimetres whose symbols are loose strokes, the way the
/// loading-island sheets are drawn: ball valves lettered BV, a check valve,
/// an XV bubble and a motor mark, a pump with its number, a shape the rules
/// do not name repeated three times, and one that occurs once.
fn exploded_sheet() -> CadDocument {
    let mut doc = CadDocument::new();
    // Frame, so the extent is an A3 in paper mm; "A" is a skipped layer.
    doc.add_entity(layered(line(0.0, 0.0, 420.0, 0.0), "A"))
        .unwrap();
    doc.add_entity(layered(line(0.0, 0.0, 0.0, 297.0), "A"))
        .unwrap();

    // Two ball valves on a pipe run, 9 mm apart, tags lettered beside them;
    // the pipe stubs between are straight axis runs and must not chain them.
    bowtie(&mut doc, 20.0, 20.0, |(x, y)| (x, y));
    bowtie(&mut doc, 29.0, 20.0, |(x, y)| (-y, x));
    doc.add_entity(layered(line(21.2, 20.0, 27.8, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(line(10.0, 20.0, 18.8, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(text("BV0301", 18.0, 22.0), "DEVICE"))
        .unwrap();
    doc.add_entity(layered(text("BV0302", 27.0, 23.0), "DEVICE"))
        .unwrap();
    // A check valve: the same bowtie, named by its tag alone.
    bowtie(&mut doc, 60.0, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(text("CV0301", 58.0, 22.5), "DEVICE"))
        .unwrap();

    // An XV bubble (small, as this family draws them) and a motor mark.
    doc.add_entity(circle(80.0, 30.0, 2.45)).unwrap();
    doc.add_entity(text("XV", 80.0, 30.8)).unwrap();
    doc.add_entity(text("0301", 80.0, 29.0)).unwrap();
    doc.add_entity(circle(80.0, 24.0, 1.0)).unwrap();
    doc.add_entity(text("M", 80.0, 24.0)).unwrap();

    // A pump, its number lettered below.
    doc.add_entity(circle(120.0, 60.0, 5.2)).unwrap();
    doc.add_entity(layered(text("P-0301", 118.0, 50.0), "DEVICE"))
        .unwrap();

    // Three of a shape nobody has named, one of them mirrored.
    triangle(&mut doc, 150.0, 100.0);
    triangle(&mut doc, 170.0, 100.0);
    for (a, b) in [
        ((189.0, 101.0), (191.0, 101.0)),
        ((191.0, 101.0), (190.0, 99.0)),
        ((190.0, 99.0), (189.0, 101.0)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
    // And a one-off: a lone pentagon-ish zigzag.
    for (a, b) in [
        ((200.0, 200.0), (201.0, 202.0)),
        ((201.0, 202.0), (202.5, 200.3)),
        ((202.5, 200.3), (203.0, 202.0)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
    doc
}

#[test]
fn exploded_sheet_symbols_are_named_by_their_tags_or_boxed_by_shape() {
    let doc = exploded_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    assert_eq!(recognition.units_per_mm, 1.0);

    // Tags name the strokes beside them, one to one.
    assert_eq!(
        count(&recognition, "ball-valve"),
        2,
        "{:?}",
        recognition.symbols
    );
    assert_eq!(tags_of(&recognition, "ball-valve"), ["BV0301", "BV0302"]);
    let left = recognition
        .symbols
        .iter()
        .find(|s| s.class == "ball-valve" && s.at.0 < 25.0)
        .unwrap();
    assert_eq!(left.tag.as_deref(), Some("BV0301"));
    assert!((left.bbox.0 - 18.8).abs() < 0.01 && (left.bbox.2 - 21.2).abs() < 0.01);
    assert_eq!(count(&recognition, "check-valve"), 1);
    assert_eq!(tags_of(&recognition, "check-valve"), ["CV0301"]);

    // Small circles are told apart by their lettering.
    assert_eq!(tags_of(&recognition, "bubble"), ["XV-0301"]);
    assert_eq!(count(&recognition, "motor"), 1);
    assert_eq!(tags_of(&recognition, "pump"), ["P-0301"]);

    // The repeated unnamed shape is boxed under one id, the one-off is not.
    let shapes: Vec<&pid_legend::Recognized> = recognition
        .symbols
        .iter()
        .filter(|s| pid_legend::is_shape_class(&s.class))
        .collect();
    assert_eq!(shapes.len(), 3, "{shapes:?}");
    let ids: BTreeSet<&str> = shapes.iter().map(|s| s.class.as_str()).collect();
    assert_eq!(ids.len(), 1, "the mirrored one has the same id: {ids:?}");
    assert!(shapes
        .iter()
        .all(|s| !s.known && s.color != [255, 255, 255]));
    assert_eq!(recognition.unknown_shapes.len(), 1);
    assert_eq!(recognition.unknown_shapes[0].count, 3);
    assert_eq!(recognition.unknown_shapes[0].strokes, 3);
    assert!(
        recognition.orphan_tags.is_empty(),
        "{:?}",
        recognition.orphan_tags
    );

    let lines = pid_legend::report(&recognition);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("UNKNOWN SHAPE") && l.contains("x3")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("球阀 (ball-valve) x2  tagged 2/2")),
        "{lines:?}"
    );
}

#[test]
fn the_shape_dictionary_names_or_drops_a_shape_by_its_id() {
    let doc = exploded_sheet();
    let mut rules = Rules::builtin();
    let first = pid_legend::recognise(&doc, &rules);
    let id = first.unknown_shapes[0].id.clone();

    // Named: the three triangles become a class of their own, on their layer.
    let shapes = rules.shapes.as_mut().unwrap();
    shapes.dictionary.insert(
        id.clone(),
        serde_json::from_str(r#"{"class":"vent-hood","label":"放空罩","color":[1,2,3]}"#).unwrap(),
    );
    let named = pid_legend::recognise(&doc, &rules);
    assert_eq!(count(&named, "vent-hood"), 3);
    assert!(named.unknown_shapes.is_empty());
    assert!(named
        .symbols
        .iter()
        .all(|s| !pid_legend::is_shape_class(&s.class)));

    // Dropped: class "ignore" means it is not a symbol at all.
    rules.shapes.as_mut().unwrap().dictionary.insert(
        id,
        serde_json::from_str(r#"{"class":"ignore","label":"-","color":[0,0,0]}"#).unwrap(),
    );
    let dropped = pid_legend::recognise(&doc, &rules);
    assert_eq!(count(&dropped, "vent-hood"), 0);
    assert!(dropped.unknown_shapes.is_empty());
    assert_eq!(dropped.symbols.len(), first.symbols.len() - 3);
}

/// A sheet whose symbols touch the pipe they sit on: a check valve and a
/// ball valve joined by a 1.5 mm piece of pipe, three ball valves each with a
/// different length of pipe stub left touching them, and a valve with a stem
/// up to a two-stroke actuator, on a pipe run.
fn piped_sheet() -> CadDocument {
    let mut doc = CadDocument::new();
    doc.add_entity(layered(line(0.0, 0.0, 420.0, 0.0), "A"))
        .unwrap();
    doc.add_entity(layered(line(0.0, 0.0, 0.0, 297.0), "A"))
        .unwrap();

    // Pipe run, check valve, 1.5 mm of pipe -- drawn twice over, as the
    // sheets do -- ball valve, 1.5 mm stub, pipe run.
    doc.add_entity(layered(line(10.0, 20.0, 17.8, 20.0), "0"))
        .unwrap();
    bowtie(&mut doc, 19.0, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(20.2, 20.0, 21.7, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(line(21.7, 20.0, 20.2, 20.0), "1"))
        .unwrap();
    ball_valve(&mut doc, 23.1, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(24.5, 20.0, 26.0, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(line(26.0, 20.0, 40.0, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(text("CV0301", 17.5, 22.3), "DEVICE"))
        .unwrap();
    doc.add_entity(layered(text("BV0301", 23.5, 22.5), "DEVICE"))
        .unwrap();

    // The same ball valve three times, untagged, with 1.0, 2.2 and 0.8 mm of
    // pipe left touching it -- one of them turned a quarter.
    doc.add_entity(layered(line(50.0, 50.0, 57.6, 50.0), "0"))
        .unwrap();
    doc.add_entity(layered(line(57.6, 50.0, 58.6, 50.0), "0"))
        .unwrap();
    ball_valve(&mut doc, 60.0, 50.0, |(x, y)| (x, y));
    ball_valve(&mut doc, 80.0, 50.0, |(x, y)| (-y, x));
    doc.add_entity(layered(line(80.0, 51.4, 80.0, 53.6), "0"))
        .unwrap();
    ball_valve(&mut doc, 100.0, 50.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(101.4, 50.0, 102.2, 50.0), "0"))
        .unwrap();
    // ... this one with a half-diagonal drawn twice.
    doc.add_entity(layered(line(98.6, 49.3, 99.6422, 49.8211), "DEVICE"))
        .unwrap();

    // A valve on a pipe run with a 2 mm stem up from its centre to a
    // three-line actuator: one symbol, the stem is not pipe.
    doc.add_entity(layered(line(50.0, 80.0, 58.8, 80.0), "0"))
        .unwrap();
    doc.add_entity(layered(line(61.2, 80.0, 70.0, 80.0), "0"))
        .unwrap();
    bowtie(&mut doc, 60.0, 80.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(60.0, 80.0, 60.0, 82.0), "DEVICE"))
        .unwrap();
    for (a, b) in [
        ((59.3, 82.0), (60.7, 82.0)),
        ((60.7, 82.0), (60.0, 83.2)),
        ((60.0, 83.2), (59.3, 82.0)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "DEVICE"))
            .unwrap();
    }
    doc.add_entity(layered(text("GV0301", 62.5, 82.5), "DEVICE"))
        .unwrap();
    doc
}

fn shape_id(symbol: &pid_legend::Recognized) -> &str {
    symbol
        .source
        .strip_prefix("shape ")
        .and_then(|s| s.split(' ').next())
        .unwrap_or_else(|| panic!("not an exploded shape: {}", symbol.source))
}

#[test]
fn pipe_is_cut_away_from_exploded_symbols_and_a_stem_is_not() {
    let doc = piped_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    assert_eq!(recognition.units_per_mm, 1.0);

    // Two symbols joined by a piece of pipe are two symbols, each its own tag.
    let check = recognition
        .symbols
        .iter()
        .find(|s| s.class == "check-valve")
        .expect("check valve");
    assert_eq!(check.tag.as_deref(), Some("CV0301"));
    assert!(
        (check.bbox.0 - 17.8).abs() < 0.01 && (check.bbox.2 - 20.2).abs() < 0.01,
        "the pipe piece is not part of the check valve: {:?}",
        check.bbox
    );
    let ball = recognition
        .symbols
        .iter()
        .find(|s| s.class == "ball-valve" && s.tag.is_some())
        .expect("the ball valve beside BV0301");
    assert_eq!(ball.tag.as_deref(), Some("BV0301"));
    assert!(
        (ball.bbox.0 - 21.7).abs() < 0.01 && (ball.bbox.2 - 24.5).abs() < 0.01,
        "neither the pipe piece nor the stub is part of the ball valve: {:?}",
        ball.bbox
    );
    assert!(ball.source.contains("(7 strokes)"), "{}", ball.source);

    // Whatever pipe was drawn touching it, and however it is turned, a ball
    // valve has the id of the tagged one -- which, drawn to the loading-island
    // sheets' measure, is the dictionary's ball valve, so the three untagged
    // ones are named too rather than boxed as an unknown shape.
    assert_eq!(
        count(&recognition, "ball-valve"),
        4,
        "{:?}",
        recognition.symbols
    );
    assert!(
        recognition.unknown_shapes.is_empty(),
        "{:?}",
        recognition.unknown_shapes
    );
    let untagged: Vec<&pid_legend::Recognized> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == "ball-valve" && s.tag.is_none())
        .collect();
    assert_eq!(untagged.len(), 3);
    for s in untagged {
        assert_eq!(shape_id(s), shape_id(ball), "{}", s.source);
        assert!(s.known);
        let (w, h) = (s.bbox.2 - s.bbox.0, s.bbox.3 - s.bbox.1);
        assert!(
            (w.max(h) - 2.8).abs() < 0.01 && (w.min(h) - 1.4).abs() < 0.01,
            "boxed without its stub: {:?}",
            s.bbox
        );
    }

    // The stem to the actuator stays: the pipe runs through the valve the
    // other way, so the vertical stroke is part of the symbol.
    let gate = recognition
        .symbols
        .iter()
        .find(|s| s.class == "gate")
        .expect("the actuated valve is one component, claimed by GV0301");
    assert_eq!(gate.tag.as_deref(), Some("GV0301"));
    assert!(gate.source.contains("(8 strokes)"), "{}", gate.source);
    assert!((gate.bbox.3 - 83.2).abs() < 0.01, "{:?}", gate.bbox);
    assert!(
        recognition.orphan_tags.is_empty(),
        "{:?}",
        recognition.orphan_tags
    );
}

/// Two bowtie valves joined by 2 mm of pipe, GV0301 and GV0302, with a small
/// arrow -- another symbol, four strokes of its own -- drawn under the second
/// valve, its shaft ending 0.07 mm short of the valve's lower edge, the way
/// SP02-05 draws a stem arrow beside the lower gate of each relief branch.
/// Only pipe, or a stub trimmed off the component itself, entering a part
/// from the side keeps the pipe between two symbols from being cut; the
/// arrow's shaft is neither, so the two valves come apart and each takes
/// its tag.
#[test]
fn another_symbols_stroke_at_the_edge_does_not_hold_two_valves_together() {
    let mut doc = CadDocument::new();
    doc.add_entity(layered(line(0.0, 0.0, 420.0, 0.0), "A"))
        .unwrap();
    doc.add_entity(layered(line(0.0, 0.0, 0.0, 297.0), "A"))
        .unwrap();
    doc.add_entity(layered(line(10.0, 20.0, 18.8, 20.0), "0"))
        .unwrap();
    bowtie(&mut doc, 20.0, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(21.2, 20.0, 23.2, 20.0), "0"))
        .unwrap();
    bowtie(&mut doc, 24.4, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(25.6, 20.0, 40.0, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(text("GV0301", 18.0, 22.3), "DEVICE"))
        .unwrap();
    doc.add_entity(layered(text("GV0302", 25.0, 22.3), "DEVICE"))
        .unwrap();
    for (a, b) in [
        ((24.4, 18.0), (24.4, 19.33)),
        ((24.4, 18.0), (24.2, 18.6)),
        ((24.4, 18.0), (24.6, 18.6)),
        ((24.2, 18.6), (24.6, 18.6)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "0"))
            .unwrap();
    }

    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    let mut gates: Vec<&pid_legend::Recognized> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == "gate")
        .collect();
    gates.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    assert_eq!(
        gates.iter().map(|s| s.tag.as_deref()).collect::<Vec<_>>(),
        [Some("GV0301"), Some("GV0302")],
        "{:?}",
        recognition.symbols
    );
    for (s, x0, x1) in [(gates[0], 18.8, 21.2), (gates[1], 23.2, 25.6)] {
        assert!(
            (s.bbox.0 - x0).abs() < 0.01
                && (s.bbox.2 - x1).abs() < 0.01
                && (s.bbox.1 - 19.4).abs() < 0.01,
            "each valve boxed alone, without the pipe or the arrow: {:?}",
            s.bbox
        );
        assert!(s.source.contains("(4 strokes)"), "{}", s.source);
    }
    assert!(
        recognition.orphan_tags.is_empty(),
        "{:?}",
        recognition.orphan_tags
    );
}

/// A flame arrester the way the loading-island sheets draw one, to their
/// measure: a frame of two 3.18 mm uprights 1.81 mm apart and two 2.72 mm
/// bars across the top and bottom, sticking out 0.45 mm each side, with two
/// 1.81 mm lines through the middle -- every stroke of the frame longer than
/// a pipe stub -- on a vertical pipe, 3.6 mm of stub from each bar to the
/// run.
fn flame_arrester(doc: &mut CadDocument, x: f64, y: f64) {
    for (a, b) in [
        ((x - 0.905, y - 1.59), (x - 0.905, y + 1.59)),
        ((x + 0.905, y - 1.59), (x + 0.905, y + 1.59)),
        ((x - 1.36, y + 1.59), (x + 1.36, y + 1.59)),
        ((x - 1.36, y - 1.59), (x + 1.36, y - 1.59)),
        ((x - 0.905, y + 0.2), (x + 0.905, y + 0.2)),
        ((x - 0.905, y - 0.2), (x + 0.905, y - 0.2)),
        ((x, y + 1.59), (x, y + 5.2)),
        ((x, y - 1.59), (x, y - 5.2)),
        ((x, y + 5.2), (x, y + 30.0)),
        ((x, y - 5.2), (x, y - 30.0)),
    ] {
        doc.add_entity(layered(line(a.0, a.1, b.0, b.1), "0"))
            .unwrap();
    }
}

/// A sheet with a flame arrester lettered FA0301 on a riser and another,
/// unlettered, on a second riser; and two ball valves 8 mm apart joined by
/// 5.6 mm of pipe with a stray FA0302 lettered over the pipe.
fn framed_sheet() -> CadDocument {
    let mut doc = CadDocument::new();
    doc.add_entity(layered(line(0.0, 0.0, 420.0, 0.0), "A"))
        .unwrap();
    doc.add_entity(layered(line(0.0, 0.0, 0.0, 297.0), "A"))
        .unwrap();

    flame_arrester(&mut doc, 40.0, 100.0);
    doc.add_entity(layered(text("FA0301", 34.0, 101.0), "DEVICE"))
        .unwrap();
    flame_arrester(&mut doc, 120.0, 200.0);

    doc.add_entity(layered(line(50.0, 20.0, 58.8, 20.0), "0"))
        .unwrap();
    bowtie(&mut doc, 60.0, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(61.2, 20.0, 66.8, 20.0), "0"))
        .unwrap();
    bowtie(&mut doc, 68.0, 20.0, |(x, y)| (x, y));
    doc.add_entity(layered(line(69.2, 20.0, 80.0, 20.0), "0"))
        .unwrap();
    doc.add_entity(layered(text("BV0301", 58.0, 22.3), "DEVICE"))
        .unwrap();
    doc.add_entity(layered(text("BV0302", 66.0, 22.3), "DEVICE"))
        .unwrap();
    doc.add_entity(layered(text("FA0302", 64.0, 23.0), "DEVICE"))
        .unwrap();
    doc
}

/// The pipe rule takes every stroke of the flame arrester's frame, so the
/// first pass has nothing for FA0301; the second pass brings back the
/// pipe-length strokes that other strokes hold at two or more points -- the
/// frame, but not the stubs, held only at the frame end -- and the tag names
/// the frame. The unlettered frame is the dictionary's flame arrester
/// (`2bbe9e32`, the loading-island sheets' seven) and is named by it. The
/// pipe between the two ball valves is held only by symbols and stays pipe:
/// the stray FA0302 gets nothing.
#[test]
fn a_symbol_drawn_in_pipe_length_strokes_is_found_for_its_tag() {
    let doc = framed_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    assert_eq!(recognition.units_per_mm, 1.0);

    let arrester = recognition
        .symbols
        .iter()
        .find(|s| s.class == "flame-arrester" && s.tag.is_some())
        .expect("the lettered frame is a flame arrester");
    assert_eq!(arrester.tag.as_deref(), Some("FA0301"));
    assert!(
        arrester.source.contains("(6 strokes, second pass)"),
        "{}",
        arrester.source
    );
    let (x0, y0, x1, y1) = arrester.bbox;
    assert!(
        (x0 - 38.64).abs() < 0.01
            && (x1 - 41.36).abs() < 0.01
            && (y0 - 98.41).abs() < 0.01
            && (y1 - 101.59).abs() < 0.01,
        "the frame without its stubs: {:?}",
        arrester.bbox
    );
    assert_eq!(shape_id(arrester), "2bbe9e32", "{}", arrester.source);
    assert_eq!(count(&recognition, "flame-arrester"), 2);
    let unlettered = recognition
        .symbols
        .iter()
        .find(|s| s.class == "flame-arrester" && s.tag.is_none())
        .expect("the unlettered frame is named by the dictionary");
    assert!(unlettered.known);
    assert_eq!(shape_id(unlettered), "2bbe9e32", "{}", unlettered.source);
    assert!(
        unlettered.source.contains("second pass") && (unlettered.at.0 - 120.0).abs() < 0.01,
        "{} at {:?}",
        unlettered.source,
        unlettered.at
    );

    let mut balls: Vec<&pid_legend::Recognized> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == "ball-valve")
        .collect();
    balls.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    assert_eq!(
        balls.iter().map(|s| s.tag.as_deref()).collect::<Vec<_>>(),
        [Some("BV0301"), Some("BV0302")]
    );
    for (s, x0, x1) in [(balls[0], 58.8, 61.2), (balls[1], 66.8, 69.2)] {
        assert!(
            (s.bbox.0 - x0).abs() < 0.01 && (s.bbox.2 - x1).abs() < 0.01,
            "the pipe between the valves is not part of either: {:?}",
            s.bbox
        );
    }
    assert_eq!(
        recognition.orphan_tags.get("阻火器").map(Vec::as_slice),
        Some(&["FA0302".to_string()][..]),
        "{:?}",
        recognition.orphan_tags
    );
    assert_eq!(
        recognition.orphan_tags.len(),
        1,
        "{:?}",
        recognition.orphan_tags
    );
}

#[test]
fn unnamed_shapes_draw_in_their_own_colour_on_one_layer() {
    let mut doc = exploded_sheet();
    let rules = Rules::builtin();
    let recognition = pid_legend::recognise(&doc, &rules);
    let added = pid_legend::apply(&mut doc, &recognition, &rules);
    assert_eq!(added, recognition.symbols.len() * 2);
    let shape_entities: Vec<&EntityType> = doc
        .model_space_entities()
        .filter(|e| e.common().layer == pid_legend::SHAPE_LAYER)
        .collect();
    assert_eq!(shape_entities.len(), 6);
    assert!(shape_entities
        .iter()
        .all(|e| matches!(e.common().color, Color::Rgb { .. })));
    let valve_entities = doc
        .model_space_entities()
        .filter(|e| e.common().layer == "PID-LEGEND-BALL-VALVE")
        .count();
    assert_eq!(valve_entities, 4);
    assert_eq!(pid_legend::clear(&mut doc), added);
}

#[test]
fn a_broken_rules_override_falls_back_to_the_builtin_rules() {
    let dir = std::env::temp_dir().join(format!("ocs-pid-legend-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{ not json").unwrap();
    assert!(Rules::from_path(&bad).is_err());
    let good = dir.join("good.json");
    std::fs::write(
        &good,
        r#"{"blocks": {"X": {"class": "x", "label": "X", "color": [1,2,3]}}}"#,
    )
    .unwrap();
    let rules = Rules::from_path(&good).unwrap();
    assert_eq!(
        rules.radius_mm, 15.0,
        "defaults fill what the file leaves out"
    );
    assert_eq!(rules.blocks["X"].class, "x");
    assert!(!rules.blocks["X"].tag.inner);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── real sheets ─────────────────────────────────────────────────────────

fn sheet(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("0版重新处理dxf-12张")
        .join(name);
    path.is_file().then_some(path)
}

fn load_sheet(name: &str) -> Option<CadDocument> {
    let path = sheet(name)?;
    match io::load_file(&path) {
        Ok(doc) => Some(doc),
        Err(error) => {
            // The editor holding the file open locks byte ranges; that is not
            // a recognition failure.
            eprintln!("skipping {name}: {error}");
            None
        }
    }
}

struct Expected {
    file: &'static str,
    butterfly: usize,
    evalve: usize,
    tanks: usize,
    field_bubbles: usize,
    panel_bubbles: usize,
    total: usize,
    /// The pipe: strokes, runs, connection points a pipe meets of those the
    /// symbols offer, open ends.
    pipe_strokes: usize,
    runs: usize,
    connected_ports: (usize, usize),
    open_ends: usize,
    /// Every motorised valve sits in this line ...
    evalve_line: &'static str,
    /// ... and these butterfly valves in this one; the other butterfly valves
    /// are on branches the sheet letters nowhere and carry no line.
    butterfly_line: (&'static [&'static str], &'static str),
}

const SHEETS: &[Expected] = &[
    Expected {
        file: "DWG-0100FF02-06 罐组II消防冷却水流程图.dxf",
        butterfly: 16,
        evalve: 8,
        tanks: 8,
        field_bubbles: 16,
        panel_bubbles: 8,
        total: 118,
        pipe_strokes: 124,
        runs: 118,
        connected_ports: (112, 167),
        open_ends: 16,
        evalve_line: "150-FW",
        butterfly_line: (
            &[
                "BUV-3201", "BUV-3202", "BUV-3203", "BUV-3204", "BUV-3205", "BUV-3206", "BUV-3207",
                "BUV-3208", "BUV-3209", "BUV-3210", "BUV-3211", "BUV-3212", "BUV-3213", "BUV-3214",
                "BUV-3215", "BUV-3216",
            ],
            "100-FW",
        ),
    },
    Expected {
        file: "DWG-0100FF02-05 罐组I泡沫混合液流程图.dxf",
        butterfly: 24,
        evalve: 11,
        tanks: 6,
        field_bubbles: 22,
        panel_bubbles: 11,
        total: 145,
        pipe_strokes: 218,
        runs: 174,
        connected_ports: (150, 201),
        open_ends: 12,
        evalve_line: "",
        butterfly_line: (
            &[
                "BUV-3113", "BUV-3114", "BUV-3115", "BUV-3116", "BUV-3125", "BUV-3126", "BUV-3127",
                "BUV-3128",
            ],
            "80-FS",
        ),
    },
];

#[test]
fn cpecc_sheets_every_symbol_is_known_and_every_valve_has_its_own_tag() {
    let rules = Rules::builtin();
    let mut checked = 0;
    for expected in SHEETS {
        let Some(doc) = load_sheet(expected.file) else {
            continue;
        };
        let recognition = pid_legend::recognise(&doc, &rules);
        let name = expected.file;
        assert_eq!(
            recognition.units_per_mm, 100.0,
            "{name}: 1:100 model-space sheet"
        );
        assert!(
            recognition.unknown_blocks.is_empty(),
            "{name}: unknown blocks {:?}",
            recognition.unknown_blocks
        );
        assert_eq!(recognition.symbols.len(), expected.total, "{name}: symbols");

        let butterfly = tags_of(&recognition, "butterfly");
        assert_eq!(
            count(&recognition, "butterfly"),
            expected.butterfly,
            "{name}"
        );
        assert_eq!(
            butterfly.len(),
            expected.butterfly,
            "{name}: every butterfly valve tagged"
        );
        let distinct: BTreeSet<&str> = butterfly.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            butterfly.len(),
            "{name}: no BUV tag used twice"
        );
        assert!(
            butterfly
                .iter()
                .all(|t| pid_legend::shape_matches("BUV-9999", t)),
            "{name}: {butterfly:?}"
        );
        assert!(
            !recognition.orphan_tags.contains_key("蝶阀"),
            "{name}: BUV tags nobody claimed: {:?}",
            recognition.orphan_tags.get("蝶阀")
        );

        let evalve = tags_of(&recognition, "evalve");
        assert_eq!(count(&recognition, "evalve"), expected.evalve, "{name}");
        assert_eq!(
            evalve.len(),
            expected.evalve,
            "{name}: every motorised valve has its XV bubble"
        );
        let distinct: BTreeSet<&str> = evalve.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            evalve.len(),
            "{name}: no XV bubble used twice"
        );
        assert!(
            evalve.iter().all(|t| t.starts_with("XV-")),
            "{name}: {evalve:?}"
        );

        assert_eq!(count(&recognition, "tank"), expected.tanks, "{name}");
        assert_eq!(
            tags_of(&recognition, "tank").len(),
            expected.tanks,
            "{name}: every tank numbered"
        );
        assert_eq!(
            count(&recognition, "bubble"),
            expected.field_bubbles,
            "{name}"
        );
        assert_eq!(
            count(&recognition, "bubble-panel"),
            expected.panel_bubbles,
            "{name}"
        );
        assert_eq!(
            tags_of(&recognition, "bubble").len() + tags_of(&recognition, "bubble-panel").len(),
            expected.field_bubbles + expected.panel_bubbles,
            "{name}: every bubble reads its own lettering"
        );

        // Every box sits on the sheet.
        for s in &recognition.symbols {
            assert!(
                s.bbox.0 >= -100.0
                    && s.bbox.2 <= 43000.0
                    && s.bbox.1 >= -100.0
                    && s.bbox.3 <= 30000.0,
                "{name}: {} boxed off the sheet at {:?}",
                s.source,
                s.bbox
            );
        }

        // The pipe: every PIPE-* stroke is in a run, the runs end at the
        // valves' connection points (0.00 mm off on these sheets), at S / K
        // circles and at the sheet connectors, and each valve knows the line
        // number lettered along the pipe it sits in.
        let pipes = &recognition.pipes;
        assert_eq!(
            pipes.segments, expected.pipe_strokes,
            "{name}: pipe strokes"
        );
        assert_eq!(pipes.runs.len(), expected.runs, "{name}: runs");
        assert_eq!(
            (pipes.connected_ports, pipes.ports),
            expected.connected_ports,
            "{name}: connection points a pipe end meets"
        );
        assert_eq!(pipes.open_ends, expected.open_ends, "{name}: open ends");
        assert!(
            pipes.runs.iter().map(|r| r.segments).sum::<usize>() >= expected.pipe_strokes,
            "{name}: every stroke is in a run (a tee only adds strokes)"
        );
        if !expected.evalve_line.is_empty() {
            for s in recognition
                .symbols
                .iter()
                .filter(|s| s.class == "evalve" && !s.source.starts_with("circle"))
            {
                assert_eq!(
                    s.lines,
                    [expected.evalve_line.to_string()],
                    "{name}: {:?} sits in {}",
                    s.tag,
                    expected.evalve_line
                );
            }
        }
        let (lettered, line) = expected.butterfly_line;
        for s in recognition
            .symbols
            .iter()
            .filter(|s| s.class == "butterfly")
        {
            let tag = s.tag.as_deref().unwrap_or("");
            if lettered.contains(&tag) {
                assert_eq!(s.lines, [line.to_string()], "{name}: {tag} sits in {line}");
            } else {
                assert!(
                    s.lines.is_empty(),
                    "{name}: {tag} is on an unlettered branch: {:?}",
                    s.lines
                );
            }
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("cpecc sheets not present; skipped");
    }
}

/// The loading-island sheets: symbols are loose strokes, bubbles are small.
#[test]
fn cpecc_exploded_sheets_read_every_bubble_and_name_the_tagged_valves() {
    let rules = Rules::builtin();
    let mut checked = 0;
    // SP02-07's flame arresters and SP02-05's flow indicators are drawn in
    // pipe-length strokes and are found for their tags by the second pass
    // (`second_pass` names the classes all of whose symbols must be); SP02-07's
    // five flow indicators are ordinary first-pass claims.
    // SP02-05's ten relief branches are each two gates and a relief valve
    // drawn touching; all ten come apart (five used to be held together by
    // the stem arrow beside the lower gate), so no symbol there carries more
    // than one tag.
    for (
        file,
        bubbles,
        ball_valves,
        check_valves,
        evalves,
        gates,
        relief_valves,
        reducers,
        arresters,
        indicators,
        second_pass,
    ) in [
        (
            "DWG-0100SP02-07 汽车装卸岛(二)工艺自控流程图.dxf",
            43 + 9,
            25,
            10,
            4,
            10,
            0,
            0,
            2,
            5,
            &["flame-arrester"][..],
        ),
        (
            "DWG-0100SP02-05 发油泵棚(二)工艺自控流程图.dxf",
            80 + 38,
            0,
            5,
            27,
            20,
            10,
            5,
            0,
            5,
            &["flow-indicator"][..],
        ),
    ] {
        let Some(doc) = load_sheet(file) else {
            continue;
        };
        let recognition = pid_legend::recognise(&doc, &rules);
        assert_eq!(recognition.units_per_mm, 1.0, "{file}: paper mm");
        assert!(
            recognition.unknown_blocks.is_empty(),
            "{file}: {:?}",
            recognition.unknown_blocks
        );
        let all_bubbles = count(&recognition, "bubble") + count(&recognition, "bubble-panel");
        assert_eq!(all_bubbles, bubbles, "{file}: bubbles");
        assert_eq!(
            tags_of(&recognition, "bubble").len() + tags_of(&recognition, "bubble-panel").len(),
            bubbles,
            "{file}: every bubble reads its lettering"
        );
        for (class, expected) in [
            ("ball-valve", ball_valves),
            ("check-valve", check_valves),
            ("evalve", evalves),
            ("gate", gates),
            ("relief-valve", relief_valves),
            ("flame-arrester", arresters),
            ("flow-indicator", indicators),
        ] {
            assert_eq!(count(&recognition, class), expected, "{file}: {class}");
            assert_eq!(
                tags_of(&recognition, class).len(),
                expected,
                "{file}: every {class} tagged"
            );
        }
        let joined: Vec<&str> = recognition
            .symbols
            .iter()
            .filter_map(|s| s.tag.as_deref())
            .filter(|t| t.contains(" + "))
            .collect();
        assert!(
            joined.is_empty(),
            "{file}: symbols still joined: {joined:?}"
        );
        let gate_tags = tags_of(&recognition, "gate");
        let distinct: BTreeSet<&str> = gate_tags.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            gate_tags.len(),
            "{file}: no GV tag used twice"
        );
        for s in &recognition.symbols {
            if second_pass.contains(&s.class.as_str()) {
                assert!(s.source.contains("second pass"), "{file}: {}", s.source);
                let (w, h) = (s.bbox.2 - s.bbox.0, s.bbox.3 - s.bbox.1);
                assert!(
                    (2.0..6.0).contains(&w) && (2.0..6.0).contains(&h),
                    "{file}: {} boxed without pipe: {w:.2} x {h:.2}",
                    s.source
                );
            }
        }
        for label in ["阻火器", "流量指示"] {
            assert!(
                !recognition.orphan_tags.contains_key(label),
                "{file}: {label} tags nobody claimed: {:?}",
                recognition.orphan_tags.get(label)
            );
        }
        let ball_tags = tags_of(&recognition, "ball-valve");
        let distinct: BTreeSet<&str> = ball_tags.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            ball_tags.len(),
            "{file}: no BV tag used twice"
        );
        // Dictionary-named shapes: every FV control valve reads its FV bubble.
        let control = tags_of(&recognition, "control-valve");
        assert_eq!(
            control.len(),
            count(&recognition, "control-valve"),
            "{file}"
        );
        assert!(
            control.iter().all(|t| t.starts_with("FV-")),
            "{file}: {control:?}"
        );
        assert!(
            !recognition.orphan_tags.contains_key("球阀"),
            "{file}: BV tags nobody claimed: {:?}",
            recognition.orphan_tags.get("球阀")
        );
        // The eccentric reducer on each pump suction carries no tag and is
        // named by the dictionary; with it named, neither sheet has a shape
        // left unnamed.
        assert_eq!(count(&recognition, "eccentric-reducer"), reducers, "{file}");
        assert!(
            recognition.unknown_shapes.is_empty(),
            "{file}: {:?}",
            recognition.unknown_shapes
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("cpecc SP02 sheets not present; skipped");
    }
}

/// SP02-10 draws its valves touching the pipe and each other: a ball valve
/// against a strainer, a check valve against a ball valve, a coupling
/// against a ball valve. Each comes apart into its two symbols, and the
/// ball valve is one dictionary id however much pipe touched it.
#[test]
fn cpecc_sp02_10_joined_symbols_come_apart_and_are_named() {
    let rules = Rules::builtin();
    let Some(doc) = load_sheet("DWG-0100SP02-10 汽车装卸岛(五)工艺自控流程图.dxf")
    else {
        eprintln!("cpecc SP02-10 not present; skipped");
        return;
    };
    let recognition = pid_legend::recognise(&doc, &rules);
    for (class, expected, tagged) in [
        ("ball-valve", 24, 2),
        ("check-valve", 8, 2),
        // Seven gates in a row on the 0326/0327 risers, each carrying its
        // own tag (see below), plus the four elsewhere.
        ("gate", 11, 7),
        ("y-strainer", 6, 0),
        ("quick-coupling", 4, 0),
        ("normally-open-valve", 2, 0),
        ("additive-tank", 2, 0),
        ("globe-valve", 2, 0),
        ("pipe-cap", 2, 0),
        ("flame-arrester", 2, 0),
        ("breather-valve", 2, 0),
        ("vent-outlet", 2, 0),
        // The two flow indicators on the 0326 / 0327 risers: a 5 mm D-shaped
        // body of pipe-length strokes, found for FI0326 / FI0327 by the
        // second pass.
        ("flow-indicator", 2, 2),
    ] {
        assert_eq!(count(&recognition, class), expected, "{class}");
        assert_eq!(tags_of(&recognition, class).len(), tagged, "{class} tagged");
    }
    let mut indicators: Vec<&str> = tags_of(&recognition, "flow-indicator");
    indicators.sort_unstable();
    assert_eq!(indicators, ["FI0326", "FI0327"]);
    assert!(
        !recognition.orphan_tags.contains_key("流量指示"),
        "{:?}",
        recognition.orphan_tags.get("流量指示")
    );
    // The dictionary's ball valve, 22 times, none carrying a stub: all one
    // size. (The two BV-tagged ones are a larger drawing of their own.)
    let small: Vec<&pid_legend::Recognized> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == "ball-valve" && shape_id(s) == "5311cc3f")
        .collect();
    assert_eq!(small.len(), 22);
    for s in small {
        let (w, h) = (s.bbox.2 - s.bbox.0, s.bbox.3 - s.bbox.1);
        assert!(
            (w.max(h) - 2.8).abs() < 0.1 && (w.min(h) - 1.4).abs() < 0.1,
            "{}: {w:.2} x {h:.2} mm",
            s.source
        );
    }
    // Everything the sheet repeats is now named or ignored.
    assert!(
        recognition.unknown_shapes.is_empty(),
        "{:?}",
        recognition.unknown_shapes
    );
    // The row of seven gates on the 80-LG92/LD20-0326A..0327D risers: each
    // tag sits 6 mm left of its own valve and 3 mm right of the neighbour's,
    // so nearest-first paired the whole row one valve to the left, orphaned
    // GV0326A and left the seventh gate unnamed. The one-to-one pairing puts
    // every tag on its own valve, left to right.
    let mut row: Vec<(f64, &str)> = recognition
        .symbols
        .iter()
        .filter(|s| s.class == "gate" && (s.at.1 / recognition.units_per_mm - 150.1).abs() < 0.5)
        .map(|s| {
            (
                s.at.0 / recognition.units_per_mm,
                s.tag.as_deref().unwrap_or("-"),
            )
        })
        .collect();
    row.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        row.iter().map(|(_, tag)| *tag).collect::<Vec<_>>(),
        ["GV0326A", "GV0326B", "GV0326C", "GV0327A", "GV0327B", "GV0327C", "GV0327D"],
        "{row:?}"
    );
    assert!(
        !recognition.orphan_tags.contains_key("闸阀"),
        "{:?}",
        recognition.orphan_tags.get("闸阀")
    );
    // QK-0302 / QK-0303 are the additive skids' equipment numbers (they sit in
    // the equipment table too), not quick-coupling tags: no class claims them
    // and none reports them.
    assert!(
        !recognition
            .orphan_tags
            .keys()
            .any(|label| label.contains("快速接头") || label.contains("橇块")),
        "{:?}",
        recognition.orphan_tags
    );
}

/// With the dictionary as named on 2026-09-07, four of the loading-island
/// sheets have nothing left unnamed: every repeated shape is a symbol class
/// or an ignore.
#[test]
fn cpecc_loading_island_sheets_have_no_unnamed_shape_left() {
    let rules = Rules::builtin();
    let mut checked = 0;
    // SP02-08 has four globe valves: the fourth used to be taken for a gate by
    // GV0311A, whose own gate stands 6 mm to the right of it; a tag no longer
    // pulls a shape the dictionary names as another class.
    // Each sheet's flame arresters (one per loading lane's vapour return) are
    // drawn in pipe-length strokes: the second pass finds them for their FA
    // tags, one each, all the same shape.
    for (file, globe_valves, small_check_valves, arresters) in [
        ("DWG-0100SP02-06 汽车装卸岛(一)工艺自控流程图.dxf", 11, 3, 1),
        ("DWG-0100SP02-07 汽车装卸岛(二)工艺自控流程图.dxf", 4, 0, 2),
        ("DWG-0100SP02-08 汽车装卸岛(三)工艺自控流程图.dxf", 4, 0, 2),
        ("DWG-0100SP02-09 汽车装卸岛(四)工艺自控流程图.dxf", 4, 0, 2),
    ] {
        let Some(doc) = load_sheet(file) else {
            continue;
        };
        let recognition = pid_legend::recognise(&doc, &rules);
        assert!(
            recognition.unknown_shapes.is_empty(),
            "{file}: {:?}",
            recognition.unknown_shapes
        );
        assert_eq!(count(&recognition, "globe-valve"), globe_valves, "{file}");
        let found: Vec<&pid_legend::Recognized> = recognition
            .symbols
            .iter()
            .filter(|s| s.class == "flame-arrester")
            .collect();
        assert_eq!(found.len(), arresters, "{file}");
        for s in &found {
            assert!(
                s.tag.as_deref().is_some_and(|t| t.starts_with("FA03")),
                "{file}: {:?}",
                s.tag
            );
            assert_eq!(shape_id(s), "2bbe9e32", "{file}: {}", s.source);
        }
        assert!(
            !recognition.orphan_tags.contains_key("阻火器"),
            "{file}: {:?}",
            recognition.orphan_tags.get("阻火器")
        );
        // The small check valves beside the pumps carry no tag; the tagged
        // ones are the tag-class claims.
        let untagged_checks = recognition
            .symbols
            .iter()
            .filter(|s| s.class == "check-valve" && s.tag.is_none())
            .count();
        assert_eq!(untagged_checks, small_check_valves, "{file}");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("cpecc SP02 sheets not present; skipped");
    }
}

#[test]
fn cpecc_sheet_legend_round_trips_through_apply_and_clear() {
    let rules = Rules::builtin();
    let Some(mut doc) = load_sheet(SHEETS[0].file) else {
        eprintln!("cpecc sheet not present; skipped");
        return;
    };
    let recognition = pid_legend::recognise(&doc, &rules);
    let before = doc.entity_count();
    let added = pid_legend::apply(&mut doc, &recognition, &rules);
    // A rectangle and a label per symbol; a polyline per run on the layer of
    // each line it carries, a ring at each open end.
    let pipes = &recognition.pipes;
    let drawn_runs: usize = pipes
        .runs
        .iter()
        .map(|r| {
            let layers = r.lines.len().max(1);
            let open = r
                .ends
                .iter()
                .filter(|e| **e == pid_pipes::End::Open)
                .count();
            layers * (1 + open)
        })
        .sum();
    assert_eq!(
        pid_legend::pipe_entities(&recognition, &rules).len(),
        drawn_runs
    );
    assert_eq!(added, recognition.symbols.len() * 2 + drawn_runs);
    assert_eq!(pid_legend::legend_handles(&doc).len(), added);
    for layer in [
        "PID-PIPE-100-FW",
        "PID-PIPE-150-FW",
        pid_legend::PIPE_NONE_LAYER,
    ] {
        assert!(doc.layers.get(layer).is_some(), "{layer}");
    }
    let rings = doc
        .model_space_entities()
        .filter(|e| {
            matches!(e, EntityType::Circle(_))
                && e.common().layer.starts_with(pid_legend::PIPE_LAYER_PREFIX)
        })
        .count();
    assert!(
        rings >= pipes.open_ends,
        "{rings} rings for {} open ends",
        pipes.open_ends
    );
    // Labels are 1.3 mm on a 1:100 sheet: 130 drawing units.
    let label_height = doc
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Text(t) if pid_legend::is_legend_layer(&t.common.layer) => Some(t.height),
            _ => None,
        })
        .unwrap();
    assert!((label_height - 130.0).abs() < 1e-6, "{label_height}");
    assert_eq!(pid_legend::clear(&mut doc), added);
    assert_eq!(doc.entity_count(), before);
}

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
    },
    Expected {
        file: "DWG-0100FF02-05 罐组I泡沫混合液流程图.dxf",
        butterfly: 24,
        evalve: 11,
        tanks: 6,
        field_bubbles: 22,
        panel_bubbles: 11,
        total: 145,
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
        checked += 1;
    }
    if checked == 0 {
        eprintln!("cpecc sheets not present; skipped");
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
    assert_eq!(added, recognition.symbols.len() * 2);
    assert_eq!(pid_legend::legend_handles(&doc).len(), added);
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

//! `.pid` import regression cover.
//!
//! The fixtures live in the sibling `pid-parse` checkout rather than in this
//! repository -- they are real SmartPlant drawings, and the parser they
//! exercise is developed against them there. Every test soft-skips when that
//! checkout is absent, the way the ACadSharp sample tests do, so a clone of
//! this repository alone still runs green.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use acadrust::{CadDocument, EntityType};

// An A2 sheet is 594 x 420mm. The decoded content of both fixtures sits
// inside that, so any converted coordinate an order of magnitude past it is a
// stray that reached the drawing rather than the diagnostic layers.
const SHEET_LIMIT_MM: f64 = 2000.0;

fn fixture(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pid-parse")
        .join("test-file")
        .join(name);
    path.is_file().then_some(path)
}

fn import(name: &str) -> Option<CadDocument> {
    let path = fixture(name)?;
    Some(
        OpenCADStudio::io::load_file(&path)
            .unwrap_or_else(|error| panic!("load {}: {error}", path.display())),
    )
}

fn layer_of(entity: &EntityType) -> &str {
    entity.common().layer.as_str()
}

fn on_layer<'a>(doc: &'a CadDocument, layer: &'a str) -> impl Iterator<Item = &'a EntityType> {
    doc.entities().filter(move |e| layer_of(e) == layer)
}

fn is_hidden(doc: &CadDocument, layer: &str) -> bool {
    doc.layers
        .get(layer)
        .unwrap_or_else(|| panic!("{layer} is not in the layer table"))
        .flags
        .off
}

/// Line work comes in at the width and colour the drawing asks for.
///
/// Until `pid-parse` could resolve a geometry record to its style, every line
/// arrived at the layer's white default, so a 0.13mm instrument line and a
/// 0.7mm process header were indistinguishable. The import now reads both off
/// the drawing's own style table, and this pins the result: a regression
/// would show up as line work back on `ByLayer`, which is invisible in a
/// count of entities.
#[test]
fn line_work_carries_the_width_and_colour_the_drawing_states() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut palette: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut unstyled = 0usize;
    for layer in ["PID-GEOMETRY", "PID-POINT"] {
        for entity in on_layer(&doc, layer) {
            let common = entity.common();
            match (common.color, common.line_weight) {
                (
                    acadrust::types::Color::Rgb { r, g, b },
                    acadrust::types::LineWeight::Value(w),
                ) => {
                    *palette
                        .entry(format!("{w:>3} #{r:02X}{g:02X}{b:02X}"))
                        .or_default() += 1;
                }
                _ => unstyled += 1,
            }
        }
    }

    assert_eq!(
        unstyled, 0,
        "every entity on the drawing layers should carry a resolved style, got palette {palette:?}"
    );
    // Widths are hundredths of a millimetre, so 70 is the 0.7mm process
    // header and 10 the 0.1mm point tick. Olive #808000 on the heavy lines
    // and green #008000 on the thin ones is this drawing's own palette.
    let expected: std::collections::BTreeMap<String, usize> = [
        (" 10 #000000", 53),
        (" 10 #0000FF", 11),
        (" 18 #008000", 4),
        (" 35 #000000", 43),
        (" 35 #FE0060", 3),
        (" 70 #808000", 24),
    ]
    .iter()
    .map(|(key, count)| ((*key).to_string(), *count))
    .collect();
    assert_eq!(palette, expected);
}

/// Dashed line work comes in as a named linetype the renderer can dash.
///
/// `style.dll` stores a line's dash as a `JStyleSimpleDashType` reference, and
/// until `pid-parse` decoded `0x002F` every line drew solid. The import now
/// pools each distinct decoded pattern into a `PID-DASH-<n>` document linetype
/// and names the line to it, so the ordinary dash shader draws it. A
/// regression shows up as line work back on `Continuous`, which is invisible
/// in an entity count.
#[test]
fn dashed_line_work_carries_a_linetype_matching_the_decoded_pattern() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    // Every PID-DASH linetype an entity names, and how many entities name it.
    let mut used: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for layer in ["PID-GEOMETRY", "PID-POINT"] {
        for entity in on_layer(&doc, layer) {
            let name = entity.common().linetype.as_str();
            if name.starts_with("PID-DASH-") {
                *used.entry(name.to_string()).or_default() += 1;
            }
        }
    }

    assert!(
        !used.is_empty(),
        "0x002F is decoded, so some of DWG-0201's line work must draw dashed"
    );

    // The two patterns DWG-0201 uses, as segment magnitudes in micrometres: a
    // 3.5 / 1.75mm metric dash and a 0.075" / 0.05" imperial one.
    let metric = vec![3500_i64, 1750];
    let imperial = vec![1905_i64, 1270];
    let mut seen: std::collections::BTreeSet<Vec<i64>> = std::collections::BTreeSet::new();

    for name in used.keys() {
        let lt = doc
            .line_types
            .get(name.as_str())
            .unwrap_or_else(|| panic!("{name} is named by an entity but absent from the table"));
        assert!(
            !lt.elements.is_empty(),
            "{name} is dashed, so it must carry pattern elements"
        );
        // "A"-type layout: the first element is drawn -- a dash, or a dot at
        // length zero -- never a bare gap, so element 0's length is >= 0.
        assert!(
            lt.elements[0].length >= 0.0,
            "{name} starts on a gap; the A-type layout starts drawn"
        );
        let magnitudes: Vec<i64> = lt
            .elements
            .iter()
            .map(|e| (e.length.abs() * 1000.0).round() as i64)
            .collect();
        seen.insert(magnitudes);
    }

    assert!(
        seen.contains(&metric),
        "the 3.5/1.75mm dash is DWG-0201's commonest pattern; got {seen:?}"
    );
    assert!(
        seen.iter().all(|m| *m == metric || *m == imperial),
        "an unexpected dash pattern reached the drawing: {seen:?}"
    );
}

/// Lettering comes in at the height the drawing's character style states.
///
/// It used to be a flat ISO 2.5mm for every label, because the height was not
/// reachable. Most of a P&ID's lettering turns out to be 1/8 inch, so that
/// default was a quarter too small across the sheet.
#[test]
fn lettering_carries_the_height_the_drawing_states() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut heights: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entity in on_layer(&doc, "PID-TEXT") {
        if let EntityType::Text(text) = entity {
            *heights.entry(format!("{:.3}", text.height)).or_default() += 1;
        }
    }

    // 3.175 is 1/8 inch, 1.500 and 3.500 are ISO 3098 sizes, and 2.464 is a
    // real stated height that lands on no drafting step at all. 2.500 is the
    // one bucket that also happens to be the fallback -- but it is mostly not
    // one: this drawing states 2.5mm ten times, and exactly one of its text
    // records reaches a character style whose height is refused, so eight of
    // the nine here are the drawing's own. Measured in `pid-parse`'s
    // `docs/analysis/2026-08-10-text-height-residue-is-one-sentinel-not-version-2.md`.
    let expected: std::collections::BTreeMap<String, usize> = [
        ("1.500", 2),
        ("2.464", 3),
        ("2.500", 9),
        ("3.175", 21),
        ("3.500", 2),
    ]
    .iter()
    .map(|(key, count)| ((*key).to_string(), *count))
    .collect();
    assert_eq!(heights, expected);
}

/// Every layer the importer names exists, and the ones carrying evidence
/// rather than drawing ship switched off.
#[test]
fn import_declares_its_layers_and_hides_the_evidence_ones() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    for visible in [
        "PID-GEOMETRY",
        "PID-FRAME",
        "PID-TEXT",
        "PID-FILL",
        "PID-SYMBOL",
        "PID-POINT",
    ] {
        assert!(!is_hidden(&doc, visible), "{visible} must open visible");
    }
    for hidden in ["PID-SYMBOL-LABEL", "PID-ANNOTATION", "PID-CONNECTIVITY"] {
        assert!(is_hidden(&doc, hidden), "{hidden} must open hidden");
    }
}

/// `PID-ANNOTATION` is declared, hidden, and empty.
///
/// It used to carry one stub per `JStyleOverride` record (PSM `0x0030`),
/// placed at an anchor read from payload `+0..15` as two f64. `style.dll`'s
/// own version-3 serialiser reads those same sixteen bytes as four
/// independent u32, so the anchor was never a coordinate; `pid-parse` emits
/// the family as `ProbeOnly` evidence now, and probe evidence carries no
/// position to draw. Settled in pid-parse's
/// `docs/analysis/2026-08-04-jstyleoverride-native-reader-settles-it.md`.
///
/// The layer keeps its declaration rather than going away with its contents.
/// The records are still in the file and still reach the importer, so an
/// empty layer states a decode gap that a missing one would hide -- and if
/// the anchor's real offset is ever found, the stubs come back here.
#[test]
fn the_annotation_layer_is_declared_but_draws_nothing() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        assert!(
            doc.layers.get("PID-ANNOTATION").is_some(),
            "{name}: PID-ANNOTATION stays declared even while it is empty"
        );
        assert!(
            is_hidden(&doc, "PID-ANNOTATION"),
            "{name}: PID-ANNOTATION must open hidden"
        );
        assert_eq!(
            on_layer(&doc, "PID-ANNOTATION").count(),
            0,
            "{name}: the JStyleOverride anchor read is retracted, so nothing may reach PID-ANNOTATION"
        );
    }
}

/// No line spans the sheet, and the layer that used to hold the ones that
/// did is gone.
///
/// DWG-0201 used to import two 1000mm rules straight across a 594mm sheet.
/// They were parked on a hidden `PID-UNRESOLVED` layer and blamed on a
/// `GLine2d` whose parameter range had not decoded. The parameter range had
/// decoded fine; the records were not records. Each was the top two bytes of
/// an `igSmartFrame2d`'s `1/√2` page ratio, 160 bytes inside that record,
/// matched by a decoder that scanned every byte offset. `pid-parse` now
/// requires chain membership and emits none, so both the lines and the layer
/// they needed are gone -- see that repo's
/// `docs/analysis/2026-08-10-gline2d-is-the-iso-page-ratio-not-a-record.md`.
#[test]
fn no_line_spans_the_sheet_and_the_diagnostic_layer_is_gone() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    assert!(
        doc.layers.get("PID-UNRESOLVED").is_none(),
        "PID-UNRESOLVED is back; nothing in the file needs it"
    );

    for entity in doc.entities() {
        let EntityType::Line(line) = entity else {
            continue;
        };
        let width = (line.end.x - line.start.x).abs();
        assert!(
            width < 900.0,
            "a {width:.0}mm line reached the drawing on layer {:?}; a scan artifact is back",
            layer_of(entity)
        );
    }
}

/// An area the drawing fills comes in filled.
///
/// `igBoundary2d` used to emit nothing, on the grounds that its segments
/// re-list the member `igLine2d` records that already draw the outline. True,
/// and it left the drawing's flow arrowheads hollow: every one of the corpus's
/// boundaries resolves through a `JStyleOverride` to a `JStyleSimpleFill`, and
/// the member lines have no way to say so. They now import as solid hatches --
/// five on DWG-0202, ten on the gongyi drawing -- in their layer's colour,
/// because `JStyleSimpleFill`'s own payload is still undecoded. Measured in
/// `pid-parse`'s `docs/analysis/2026-08-10-fill-has-a-consumer-after-all.md`.
#[test]
fn filled_areas_come_in_as_solid_hatches() {
    for (name, expected) in [("DWG-0202GP06-01.pid", 5), ("D06.pid", 0)] {
        let Some(doc) = import(name) else {
            continue;
        };
        let hatches: Vec<_> = on_layer(&doc, "PID-FILL").collect();
        assert_eq!(
            hatches.len(),
            expected,
            "{name}: expected {expected} filled area(s) on PID-FILL"
        );
        for entity in &hatches {
            let EntityType::Hatch(hatch) = entity else {
                panic!("{name}: PID-FILL must carry hatches, got {entity:?}");
            };
            assert!(hatch.is_solid, "{name}: the decoded fill is a solid one");
            let edges: usize = hatch.paths.iter().map(|path| path.edges.len()).sum();
            assert!(
                edges >= 3,
                "{name}: a filled area needs a closed ring, got {edges} edge(s)"
            );
        }
    }
}

/// Endpoint pairs are the drawing's connectivity graph. Only the ones whose
/// two ends both land on the sheet are drawn, on their own hidden layer.
#[test]
fn connectivity_links_stay_on_the_sheet() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let links: Vec<_> = on_layer(&doc, "PID-CONNECTIVITY").collect();
    assert!(
        !links.is_empty(),
        "DWG-0201 has 35 on-sheet endpoint pairs; none were imported"
    );
    for entity in links {
        let EntityType::Line(line) = entity else {
            panic!("PID-CONNECTIVITY carries lines only, found {entity:?}");
        };
        for value in [line.start.x, line.start.y, line.end.x, line.end.y] {
            assert!(
                value.abs() < SHEET_LIMIT_MM,
                "connectivity link reaches {value:.0}mm, which is off the sheet"
            );
        }
        assert!(
            line.start.distance(&line.end) > 0.0,
            "a zero-length link carries no direction to draw"
        );
    }
}

/// The opening view is stated by the importer rather than left to the
/// document default, and it is stated over the sheet.
#[test]
fn import_frames_the_drawing_on_its_sheet() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let extents_min = doc.header.model_space_extents_min;
    let extents_max = doc.header.model_space_extents_max;
    assert!(
        extents_max.x > extents_min.x && extents_max.y > extents_min.y,
        "model-space extents are empty: {extents_min:?}..{extents_max:?}"
    );
    assert!(
        extents_max.x < SHEET_LIMIT_MM && extents_max.y < SHEET_LIMIT_MM,
        "extents {extents_max:?} were framed on an off-sheet stray"
    );

    let vport = doc
        .vports
        .get("*Active")
        .expect("importer states the opening viewport");
    // The default `CadDocument` entry is parked at the origin with a 10-unit
    // height; anything sheet-sized means the importer replaced it.
    assert!(
        vport.view_height > 100.0,
        "*Active still has the default {}-unit height",
        vport.view_height
    );
}

/// The sheet's border is drawn, because a `.pid` carries it as an OLE object
/// linked into the drawing rather than as line work: without this the content
/// hangs in an empty background with no edge to read it against.
///
/// Only the rectangle is drawn. Its corners are the page `pid-parse` decoded
/// from the drawing's own `igSmartFrame2d` record, so the border and the
/// opening view agree by construction.
#[test]
fn the_sheet_border_is_drawn_at_the_page_the_drawing_states() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let borders: Vec<_> = on_layer(&doc, "PID-FRAME").collect();
    assert_eq!(borders.len(), 1, "one sheet carries one border");
    let EntityType::LwPolyline(border) = borders[0] else {
        panic!("the border is a polyline, found {:?}", borders[0]);
    };
    assert!(
        border.is_closed,
        "an open border does not read as a sheet edge"
    );
    assert_eq!(border.vertices.len(), 4, "a sheet is a rectangle");

    let xs: Vec<f64> = border.vertices.iter().map(|v| v.location.x).collect();
    let ys: Vec<f64> = border.vertices.iter().map(|v| v.location.y).collect();
    let min_x = xs.iter().copied().fold(f64::MAX, f64::min);
    let min_y = ys.iter().copied().fold(f64::MAX, f64::min);
    let width = xs.iter().copied().fold(f64::MIN, f64::max) - min_x;
    let height = ys.iter().copied().fold(f64::MIN, f64::max) - min_y;

    assert!(
        min_x.abs() < 1.0e-9 && min_y.abs() < 1.0e-9,
        "the page starts at the origin, this one at ({min_x}, {min_y})"
    );
    // DWG-0201 is an A2 whose own frame measures 594.3 x 420.3mm.
    assert!(
        (width - 594.3).abs() < 0.1 && (height - 420.3).abs() < 0.1,
        "border is {width:.1} x {height:.1}mm, the drawing states 594.3 x 420.3"
    );

    for entity in on_layer(&doc, "PID-GEOMETRY") {
        for point in geometry_extremes(entity) {
            assert!(
                point.0 > -SHEET_MARGIN_MM && point.0 < width + SHEET_MARGIN_MM,
                "drawing content at x={} is not on the {width:.1}mm sheet",
                point.0
            );
        }
    }
}

/// How far outside the border a drawn coordinate may still sit. A symbol
/// whose insertion point a misparse nudged past the edge is still part of the
/// drawing; a coordinate a page-width away is not.
const SHEET_MARGIN_MM: f64 = 100.0;

fn geometry_extremes(entity: &EntityType) -> Vec<(f64, f64)> {
    match entity {
        EntityType::Line(line) => vec![(line.start.x, line.start.y), (line.end.x, line.end.y)],
        EntityType::LwPolyline(polyline) => polyline
            .vertices
            .iter()
            .map(|v| (v.location.x, v.location.y))
            .collect(),
        EntityType::Circle(circle) => vec![(circle.center.x, circle.center.y)],
        _ => Vec::new(),
    }
}

/// With the published `_Data.xml` beside the drawing, imported entities
/// carry their published identity in XDATA: class, tag / line number, the
/// published GraphicOID, and which hop of pid-parse's two-hop join found
/// them. The properties panel renders these as the read-only "P&ID" group.
#[test]
fn published_semantics_land_on_entities_when_the_xml_sits_beside() {
    let Some(doc) = import("export-test/publish-data/DWG-0202GP06-01/DWG-0202GP06-01.pid") else {
        return;
    };

    let mut tagged = 0usize;
    let mut classes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut labelled = 0usize;
    let mut via_dependency = 0usize;
    for entity in doc.entities() {
        let Some(record) = entity.common().extended_data.get_record("PID_SEMANTICS") else {
            continue;
        };
        tagged += 1;
        for value in &record.values {
            let acadrust::xdata::XDataValue::String(text) = value else {
                continue;
            };
            if let Some(class) = text.strip_prefix("class=") {
                classes.insert(class.to_string());
            }
            if text.starts_with("label=") {
                labelled += 1;
            }
            if text.starts_with("resolved=dependency:") {
                via_dependency += 1;
            }
        }
    }

    assert!(
        tagged > 0,
        "the publish pair ships a _Data.xml; some drawn entities must carry PID_SEMANTICS"
    );
    assert!(
        labelled > 0,
        "at least one published object carries an ItemTag / Name to show"
    );
    assert!(
        via_dependency > 0,
        "the S1 aggregates resolve their line work one dependency hop out; none arrived"
    );
    assert!(
        !classes.is_empty(),
        "every PID_SEMANTICS record carries its owning class"
    );
}

/// Without a `_Data.xml` beside the drawing nothing changes: no entity
/// carries the semantics record, so the properties panel never shows the
/// "P&ID" group. The XML is an enrichment, never a prerequisite.
#[test]
fn a_drawing_without_published_xml_carries_no_semantics() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        assert!(
            doc.entities().all(|e| e
                .common()
                .extended_data
                .get_record("PID_SEMANTICS")
                .is_none()),
            "{name}: no _Data.xml sits beside this fixture, so no entity may carry PID_SEMANTICS"
        );
    }
}

/// Both fixtures import, and the drawing lands on the layers that open
/// visible rather than only on the diagnostic ones.
#[test]
fn fixtures_import_with_visible_drawing_content() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        let visible = ["PID-GEOMETRY", "PID-TEXT", "PID-SYMBOL", "PID-POINT"]
            .iter()
            .map(|layer| on_layer(&doc, layer).count())
            .sum::<usize>();
        assert!(visible > 0, "{name}: nothing reached a visible layer");
        assert_eq!(
            doc.source_path.as_deref().map(|p| p.ends_with(name)),
            Some(true),
            "{name}: import did not record where it came from"
        );
    }
}

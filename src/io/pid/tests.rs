use super::*;

/// A summary stored into a document reads back field for field --
/// counts, the failed flag, several library roots, and both unit
/// readings -- and `take` leaves nothing behind for a second reader: the
/// document is the one it was before `store`, allocator included, which
/// is what keeps a `.pid` saved after its open byte-identical to one
/// saved before the summary rode this way.
#[test]
fn the_summary_round_trips_through_its_properties_and_take_removes_them() {
    let mut summary = ImportSummary::empty();
    summary.drawn = 335;
    summary.decoded = 310;
    summary.missing = 12;
    summary.style_tables_failed = true;
    summary.sheet_layers = 7;
    summary.layered_entities = 300;
    summary.unresolved_sheet_layers = 1;
    summary.sheet_layer_names = 4;
    summary.sheet_layers_off = 1;
    summary.driving_dimensions = 4;
    summary.template_bodies = 2;
    summary.parametric_placements = 2;
    summary.cache_bodies = 20;
    summary.library_bodies = 3;
    summary.hidden_strokes_skipped = 31;
    summary.lettering_flattened = 10;
    summary.symbol_library = vec![
        PathBuf::from(r"\\server\Plant\Ref\Symbols"),
        PathBuf::from("D:/sym with spaces/=and=equals"),
    ];
    summary.unit = ImportUnit::Stated {
        unit: "m".to_string(),
        mm_per_unit: MM_PER_METRE,
    };

    let mut doc = CadDocument::new();
    doc.summary_info
        .custom_properties
        .push(("Client".to_string(), "kept".to_string()));
    let before = doc.clone();
    assert!(
        ImportSummary::load(&doc).is_none(),
        "a document with only its own properties carries none"
    );
    summary.store(&mut doc);
    assert_eq!(ImportSummary::load(&doc).as_ref(), Some(&summary));
    assert_ne!(doc, before, "stored, it is in the document");
    assert_eq!(ImportSummary::take(&mut doc), Some(summary.clone()));
    assert!(
        ImportSummary::take(&mut doc).is_none(),
        "take drains the properties"
    );
    assert_eq!(
        doc, before,
        "taken, the document is exactly what it was -- other properties kept, no handle spent"
    );

    // Storing again replaces rather than appends, and the assumed unit
    // reads back as itself.
    summary.unit = ImportUnit::AssumedMetre;
    summary.symbol_library.clear();
    summary.store(&mut doc);
    summary.store(&mut doc);
    let read = ImportSummary::load(&doc).expect("stored");
    assert_eq!(read, summary);
    assert!(read.unit.is_assumed());
    assert_eq!(read.unit.mm_per_unit(), MM_PER_METRE);
    assert_eq!(
        doc.summary_info
            .custom_properties
            .iter()
            .filter(|(tag, _)| tag.starts_with(SUMMARY_PROPERTY_PREFIX))
            .count(),
        summary.counts().len() + 2,
        "one property per field: the counts, the failed flag, the unit"
    );
}

/// The unit is read off the first decoded record that states one this
/// importer knows; a drawing whose records state none, or an unknown
/// label, is scaled as metres and says so.
#[test]
fn the_unit_is_read_from_a_decoded_record_or_assumed_to_be_the_metre() {
    let entity =
        |confidence: PidGeometryConfidence, units: PidDrawingUnits| pid_parse::PidGraphicEntity {
            id: "e".to_string(),
            drawing_id: None,
            graphic_oid: None,
            source_layer: None,
            kind: PidGraphicKind::Point {
                position: PidPoint { x: 0.0, y: 0.0 },
            },
            coordinate_context: pid_parse::PidCoordinateContext {
                units,
                ..Default::default()
            },
            source: pid_parse::PidGraphicProvenance {
                stream_path: None,
                byte_range: None,
                record_id: None,
                record_kind: None,
                field_x: None,
                note: None,
            },
            confidence,
        };
    let known = |unit: &str| PidDrawingUnits::Known {
        unit: unit.to_string(),
    };
    let unknown = || PidDrawingUnits::Unknown {
        diagnostic: "no frame".to_string(),
    };
    let path = Path::new("unit-test.pid");
    let geometry = |entities: Vec<pid_parse::PidGraphicEntity>| NormalizedPidGeometry {
        entities,
        ..Default::default()
    };

    // A stated metre, even after an inferred record with no unit.
    let stated = ImportUnit::read(
        &geometry(vec![
            entity(PidGeometryConfidence::Inferred, unknown()),
            entity(PidGeometryConfidence::Decoded, known("m")),
        ]),
        path,
    );
    assert_eq!(
        stated,
        ImportUnit::Stated {
            unit: "m".to_string(),
            mm_per_unit: MM_PER_METRE
        }
    );
    assert!(!stated.is_assumed());
    // Millimetres are the identity.
    assert_eq!(
        ImportUnit::read(
            &geometry(vec![entity(PidGeometryConfidence::Decoded, known("mm"))]),
            path
        )
        .mm_per_unit(),
        1.0
    );
    // Nothing decoded states a unit: assumed, and a metre's worth.
    let assumed = ImportUnit::read(
        &geometry(vec![entity(PidGeometryConfidence::Decoded, unknown())]),
        path,
    );
    assert_eq!(assumed, ImportUnit::AssumedMetre);
    assert_eq!(assumed.mm_per_unit(), MM_PER_METRE);
    // A label this importer has not been shown is not guessed at.
    assert!(ImportUnit::read(
        &geometry(vec![entity(
            PidGeometryConfidence::Decoded,
            known("furlong")
        )]),
        path
    )
    .is_assumed());
    // Only decoded records are asked; a probe's unit is not a reading.
    assert!(ImportUnit::read(
        &geometry(vec![entity(PidGeometryConfidence::ProbeOnly, known("mm"))]),
        path
    )
    .is_assumed());
}

/// The file's display bit decides, and the name only speaks when the
/// file said nothing (plan 2026-09-07, L1): a layer named `Invisible`
/// the sheet displays is drawn, a layer named `Default` the sheet hides
/// is hidden, and a layer without a bit falls back to the three names.
#[test]
fn the_display_bit_outranks_the_name_and_the_name_is_only_the_fallback() {
    let layer = |name: &str, displayed: Option<bool>| PidSourceLayer {
        oid: 8,
        name: Some(name.to_string()),
        storage_path: "/".to_string(),
        displayed,
    };
    assert!(!sheet_layer_is_hidden(&layer("Invisible", Some(true))));
    assert!(sheet_layer_is_hidden(&layer("Default", Some(false))));
    assert!(sheet_layer_is_hidden(&layer("HiddenObjects", Some(false))));
    assert!(sheet_layer_is_hidden(&layer("Hidden", None)));
    assert!(sheet_layer_is_hidden(&layer("Invisible", None)));
    assert!(!sheet_layer_is_hidden(&layer("Labels", None)));
    assert!(!sheet_layer_is_hidden(&PidSourceLayer {
        oid: 8,
        name: None,
        storage_path: "/".to_string(),
        displayed: None,
    }));
}

/// The extent a user reads off the panel follows the arc's sweep rather
/// than its whole circle (plan 2026-09-18, K2): a quarter reaches one
/// corner, a half one side, a sweep across the x axis takes in the point
/// at angle zero, and coincident ends are the full circle.
#[test]
fn an_arcs_extent_is_the_sweep_it_draws_and_not_its_circle() {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let close = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        (a.0 - b.0).abs() < 1e-9
            && (a.1 - b.1).abs() < 1e-9
            && (a.2 - b.2).abs() < 1e-9
            && (a.3 - b.3).abs() < 1e-9
    };
    let quarter = arc_extent((0.0, 0.0), 1.0, 0.0, FRAC_PI_2);
    assert!(close(quarter, (0.0, 0.0, 1.0, 1.0)), "{quarter:?}");
    let half = arc_extent((0.0, 0.0), 1.0, FRAC_PI_2, 3.0 * FRAC_PI_2);
    assert!(close(half, (-1.0, -1.0, 0.0, 1.0)), "{half:?}");
    let across_zero = arc_extent((0.0, 0.0), 1.0, 7.0 * FRAC_PI_4, FRAC_PI_4);
    let c = FRAC_PI_4.cos();
    assert!(close(across_zero, (c, -c, 1.0, c)), "{across_zero:?}");
    let whole = arc_extent((2.0, 3.0), 1.0, PI, PI);
    assert!(close(whole, (1.0, 2.0, 3.0, 4.0)), "{whole:?}");
    // The Manifold's end caps: two semicircles of r 35.59 facing outward
    // reach exactly the body's top and bottom and one end each.
    let mut arc = acadrust::entities::Arc::new();
    arc.center = Vector3::new(10.0, 10.0, 0.0);
    arc.radius = 35.59;
    arc.start_angle = FRAC_PI_2;
    arc.end_angle = 3.0 * FRAC_PI_2;
    let cap = stroke_extent(&EntityType::Arc(arc)).expect("an arc is a stroke");
    assert!(
        close(cap, (10.0 - 35.59, 10.0 - 35.59, 10.0, 10.0 + 35.59)),
        "{cap:?}"
    );
    let mut label = Text::new();
    label.value = "Parametric Manifold".to_string();
    assert!(stroke_extent(&EntityType::Text(label)).is_none());
}

fn marker_at(x: f64, y: f64) -> EntityType {
    let mut marker = Circle::new();
    marker.center = Vector3::new(x, y, 0.0);
    marker.radius = SYMBOL_MARKER_RADIUS_MM;
    EntityType::Circle(marker)
}

fn line(start: (f64, f64), end: (f64, f64)) -> EntityType {
    EntityType::Line(Line::from_points(
        Vector3::new(start.0, start.1, 0.0),
        Vector3::new(end.0, end.1, 0.0),
    ))
}

/// The marker fallback keeps the position the old anchor-relative formula
/// gave it, to the last decimal: `insertion + radius + gap` across, and
/// half a cap-height down from the insertion point.
#[test]
fn a_marker_is_named_where_the_insertion_point_formula_put_it() {
    let insertion = (40.0, 25.0);
    let anchor = symbol_label_anchor(&[marker_at(insertion.0, insertion.1)], insertion);
    assert_eq!(
        anchor.x,
        insertion.0 + SYMBOL_MARKER_RADIUS_MM + SYMBOL_LABEL_GAP_MM
    );
    assert_eq!(anchor.y, insertion.1 - SYMBOL_LABEL_HEIGHT_MM / 2.0);
}

/// A symbol whose library body is drawn away from its own origin is named
/// beside the body. Reverting to the insertion point puts the name a
/// hundred millimetres away from anything the placement drew, which on
/// `DWG-0202` was off the edge of the sheet -- see
/// `docs/analysis/2026-08-24-two-texts-outside-the-frame.md`.
#[test]
fn a_body_drawn_away_from_its_anchor_is_named_beside_the_body() {
    // `ElecTraceLine.sym` to scale: the placement is anchored at x = -25.6
    // and the body it draws sits 104mm right and 155mm up from there.
    let insertion = (-25.6, 349.9);
    let body = [
        line((78.2, 504.1), (82.1, 505.6)),
        line((82.1, 504.1), (78.2, 505.6)),
    ];
    let anchor = symbol_label_anchor(&body, insertion);

    assert_eq!(anchor.x, 82.1 + SYMBOL_LABEL_GAP_MM);
    assert_eq!(
        anchor.y,
        (504.1 + 505.6) / 2.0 - SYMBOL_LABEL_HEIGHT_MM / 2.0
    );
    assert!(
        anchor.x - insertion.0 > 100.0,
        "the name follows the body, not the anchor it was placed from"
    );
}

/// An arc contributes its whole circle and lettering contributes only the
/// point it starts from, so the extent a label and the opening view are
/// built on covers the line work either way.
#[test]
fn an_extent_covers_an_arc_and_stops_at_a_text_insertion_point() {
    let mut arc = acadrust::entities::Arc::new();
    arc.center = Vector3::new(10.0, 20.0, 0.0);
    arc.radius = 2.5;
    arc.start_angle = 0.0;
    arc.end_angle = std::f64::consts::FRAC_PI_2;
    assert_eq!(
        drawn_extent(&EntityType::Arc(arc)),
        Some((7.5, 17.5, 12.5, 22.5))
    );

    let mut text = Text::new();
    text.value = "pump".to_string();
    text.height = 2.5;
    text.insertion_point = Vector3::new(3.0, 4.0, 0.0);
    assert_eq!(
        drawn_extent(&EntityType::Text(text)),
        Some((3.0, 4.0, 3.0, 4.0))
    );
}

/// A cached body draws its strokes on the layers the file displays and
/// none of the others (plan 2026-09-19, P-D2), and the placement is
/// measured over the same strokes. A stroke on a layer the file says
/// nothing about is drawn, and one with no layer entry at all (a body
/// from before the field existed) too.
#[test]
fn a_cached_body_draws_only_the_strokes_on_its_displayed_layers() {
    use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};
    let line = |x: f64| SymbolPrimitive::Line {
        start: (x, 0.0),
        end: (x, 0.01),
    };
    let body = PidSymbolDefinition {
        reference: PidSymbolDefinitionRef {
            site: 396,
            sheet: 113,
        },
        layers: vec![8, 9, 10],
        sheet_layers: vec![
            PidSymbolSheetLayer {
                oid: 8,
                name: Some("Default".to_string()),
                displayed: Some(true),
            },
            PidSymbolSheetLayer {
                oid: 9,
                name: Some("Construction".to_string()),
                displayed: Some(false),
            },
            PidSymbolSheetLayer {
                oid: 10,
                name: None,
                displayed: None,
            },
        ],
        primitives: vec![line(0.0), line(0.1), line(0.2), line(0.3), line(0.4)],
        primitive_layers: vec![8, 9, 10, 9],
        primitive_styles: Vec::new(),
        dimensions: Vec::new(),
        variables: Vec::new(),
        template: None,
    };
    let x_of = |stroke: &SymbolPrimitive| match stroke {
        SymbolPrimitive::Line { start, .. } => start.0,
        other => panic!("{other:?}"),
    };
    let drawn: Vec<f64> = body.visible_strokes().map(|(p, _)| x_of(p)).collect();
    assert_eq!(
        drawn,
        vec![0.0, 0.2, 0.4],
        "Default on, Construction off, an unlisted layer and a stroke with no layer entry drawn"
    );
    let measured: Vec<f64> = body.visible_primitives().map(x_of).collect();
    assert_eq!(measured, drawn, "the panel measures the strokes on screen");

    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let at = Placement {
        insertion: &insertion,
        rotation: 0.0,
        scale: [1.0, 1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let mut tally = SymbolBodies::default();
    let entities = cached_body_entities(Some(&body), &at, &mut tally, &HashMap::new())
        .expect("three strokes draw");
    assert_eq!(entities.len(), 3);
    assert_eq!(
        (tally.cache, tally.hidden_strokes_skipped),
        (1, 2),
        "one body drawn, its two Construction strokes counted as skipped"
    );
}

/// A cached stroke's dash is its own storage's, and its colour and width
/// are its placement's (plan 2026-09-20, P-E1 / P-E8): an off-page
/// connector's dashed circle, authored `#00FEA0` 0.50 with a 3.5 / 1.75
/// dash, is repainted olive 0.35 by the solid placement style and stays
/// dashed; its solid leg stays solid; a stroke with no style of its own
/// takes only the placement's paint. The body's pattern joins the pool
/// after the sheet's own, whose name does not move. A placement style
/// that names a dash of its own wins over the stroke's.
#[test]
fn a_cached_strokes_dash_is_its_own_and_its_colour_its_placements() {
    use pid_parse::style_link::{LineSymbology, StyleHop};
    use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};

    let own = |dash_mm: Vec<f64>| PrimitiveStyle {
        rgb: [0x00, 0xFE, 0xA0],
        width_mm: 0.5,
        dash_mm,
    };
    let body = PidSymbolDefinition {
        reference: PidSymbolDefinitionRef {
            site: 7559,
            sheet: 155,
        },
        layers: vec![8],
        sheet_layers: vec![PidSymbolSheetLayer {
            oid: 8,
            name: Some("Default".to_string()),
            displayed: Some(true),
        }],
        primitives: vec![
            SymbolPrimitive::Circle {
                center: (0.0, 0.0),
                radius: 0.005,
            },
            SymbolPrimitive::Line {
                start: (0.0, 0.0),
                end: (0.01, 0.0),
            },
            SymbolPrimitive::Line {
                start: (0.0, 0.0),
                end: (0.0, 0.01),
            },
        ],
        primitive_layers: vec![8, 8, 8],
        primitive_styles: vec![Some(own(vec![3.5, 1.75])), Some(own(Vec::new())), None],
        dimensions: Vec::new(),
        variables: Vec::new(),
        template: None,
    };
    // The placement's style: olive 0.35, `COLORREF` `0x00BBGGRR`.
    let placement_style = |dash: Option<DashPattern>| ResolvedLineStyle {
        style_id: 25,
        symbology: LineSymbology {
            width_m: 0.000_35,
            colour: 0x0000_8080,
        },
        dash,
        marker: None,
        hop: StyleHop::Direct,
    };
    let sheet_dash = || DashPattern::from_segments_m(&[0.001, 0.001]);

    let mut styles = LineStyleIndex::new();
    styles.insert(("/".to_string(), 1), placement_style(sheet_dash()));
    let mut doc = CadDocument::new();
    let pool = register_dash_linetypes(&mut doc, &styles, std::slice::from_ref(&body));
    assert_eq!(
        pool.get(&vec![1000, 1000]).map(String::as_str),
        Some("PID-DASH-1"),
        "the sheet's own pattern is pooled first and keeps its name"
    );
    assert_eq!(
        pool.get(&vec![3500, 1750]).map(String::as_str),
        Some("PID-DASH-2"),
        "the cached body's pattern joins the pool after it"
    );
    assert!(doc.line_types.contains("PID-DASH-2"));

    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let at = Placement {
        insertion: &insertion,
        rotation: 0.0,
        scale: [1.0, 1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let mut tally = SymbolBodies::default();
    let mut drawn =
        cached_body_entities(Some(&body), &at, &mut tally, &pool).expect("three strokes draw");
    assert_eq!(drawn.len(), 3);
    let coat = |entity: &EntityType| {
        let common = entity.common();
        (common.color, common.line_weight, common.linetype.clone())
    };
    let fresh = Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)).common;
    let as_built = fresh.linetype.clone();
    let olive = Color::from_rgb(0x80, 0x80, 0x00);
    assert_eq!(
        coat(&drawn[0]),
        (
            Color::from_rgb(0x00, 0xFE, 0xA0),
            LineWeight::Value(50),
            "PID-DASH-2".to_string()
        ),
        "the undercoat is the body's own colour, width and dash"
    );
    assert_eq!(
        coat(&drawn[1]).2,
        as_built,
        "a solid stroke names no linetype"
    );
    assert_eq!(
        coat(&drawn[2]),
        (fresh.color, fresh.line_weight, as_built.clone()),
        "a stroke with no style of its own is left as built"
    );

    for entity in &mut drawn {
        apply_symbology(entity, &placement_style(None), &pool);
    }
    assert_eq!(
        coat(&drawn[0]),
        (olive, LineWeight::Value(35), "PID-DASH-2".to_string()),
        "a solid placement style repaints colour and width and leaves the dash"
    );
    assert_eq!(
        coat(&drawn[1]),
        (olive, LineWeight::Value(35), as_built.clone())
    );
    assert_eq!(coat(&drawn[2]), (olive, LineWeight::Value(35), as_built));

    for entity in &mut drawn {
        apply_symbology(entity, &placement_style(sheet_dash()), &pool);
    }
    assert!(
        drawn
            .iter()
            .all(|entity| entity.common().linetype == "PID-DASH-1"),
        "a placement style that names a dash of its own wins over the stroke's: {:?}",
        drawn.iter().map(coat).collect::<Vec<_>>()
    );
}

/// A symbol's lettering takes its placement's colour and nothing else.
/// Measured on DWG-0201's level-gauge bubbles: `LG-Magnetic Float
/// Gauge.sym` and `LT-Magnetostrictive Level Gauge.sym` author their
/// letters in a `#FF0000` character style, the placements name the
/// instrument class colour `#008000`, and SmartPlant's screenshot
/// letters them green. The rule used to be pinned on the library-first
/// import, the one that still lettered the symbol layer; that import is
/// retired (plan 2026-09-20-retire-the-library-first-symbol-source), and
/// the branch is still a real path -- a library body standing in for a
/// placement the drawing caches nothing drawable for letters through it
/// -- so it is pinned here: a text on `PID-SYMBOL` is repainted and
/// takes no line weight, and a text on the sheet's own layer is left
/// alone, its colour being its character style's.
#[test]
fn a_symbols_lettering_takes_its_placements_colour_and_nothing_else() {
    use pid_parse::style_link::{LineSymbology, StyleHop};
    let green = ResolvedLineStyle {
        style_id: 61,
        symbology: LineSymbology {
            width_m: 0.000_18,
            colour: 0x0000_8000,
        },
        dash: None,
        marker: None,
        hop: StyleHop::Direct,
    };
    let letters = |layer: &str| {
        let mut text = Text::new();
        text.value = "LGM".to_string();
        text.height = 2.5;
        text.common.layer = layer.to_string();
        text.common.color = Color::from_rgb(0xFF, 0x00, 0x00);
        EntityType::Text(text)
    };
    let no_dashes = HashMap::new();

    let mut on_symbol = letters(LAYER_SYMBOL);
    apply_symbology(&mut on_symbol, &green, &no_dashes);
    assert_eq!(on_symbol.common().color, Color::from_rgb(0x00, 0x80, 0x00));
    assert_eq!(
        on_symbol.common().line_weight,
        Text::new().common.line_weight,
        "no line weight lands on lettering"
    );
    let EntityType::Text(text) = &on_symbol else {
        panic!("still a text");
    };
    assert_eq!((text.value.as_str(), text.height), ("LGM", 2.5));

    let mut on_sheet = letters(LAYER_TEXT);
    apply_symbology(&mut on_sheet, &green, &no_dashes);
    assert_eq!(
        on_sheet.common().color,
        Color::from_rgb(0xFF, 0x00, 0x00),
        "the sheet's own lettering is coloured by its character style, not here"
    );
}

/// A body's arc runs clockwise from its start angle to its end angle,
/// and lands on the sheet as the DXF arc that covers the same points:
/// the Manifold's left cap, `270° -> 90°` about a centre on the shell's
/// left edge, is drawn as the counter-clockwise arc `90° -> 270°`, whose
/// midpoint bulges left out of the shell -- and a mirrored placement,
/// which reverses the sense again, keeps the bulge on the outside.
#[test]
fn a_bodys_arc_is_drawn_as_the_counter_clockwise_arc_over_the_same_points() {
    use std::f64::consts::{FRAC_PI_2, PI};
    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let placement = |scale_y: f64| Placement {
        insertion: &insertion,
        rotation: 0.0,
        scale: [1.0, scale_y],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let left_cap = SymbolPrimitive::Arc {
        center: (0.04118, 0.08890),
        radius: 0.03559,
        start_angle: 3.0 * FRAC_PI_2,
        end_angle: FRAC_PI_2,
    };
    let midpoint = |arc: &acadrust::entities::Arc| {
        let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
        let angle = arc.start_angle + sweep / 2.0;
        (
            arc.center.x + arc.radius * angle.cos(),
            arc.center.y + arc.radius * angle.sin(),
        )
    };

    let Some(EntityType::Arc(upright)) = shape_primitive(&left_cap, &placement(1.0)) else {
        panic!("the cap did not build");
    };
    assert!((upright.start_angle - FRAC_PI_2).abs() < 1e-12);
    assert!((upright.end_angle - 3.0 * FRAC_PI_2).abs() < 1e-12);
    let (x, y) = midpoint(&upright);
    assert!(
        (x - (41.18 - 35.59)).abs() < 1e-6 && (y - 88.90).abs() < 1e-6,
        "the cap's midpoint is the shell's left edge less the radius: ({x}, {y})"
    );
    let cap = stroke_extent(&EntityType::Arc(upright)).expect("an arc is a stroke");
    assert!(
        (cap.0 - (41.18 - 35.59)).abs() < 1e-6 && (cap.2 - 41.18).abs() < 1e-6,
        "the cap reaches from the shell's edge out to its apex: {cap:?}"
    );

    // Mirrored about x: the same cap on a body flipped upside down still
    // bulges left, with its ends swapped top for bottom.
    let Some(EntityType::Arc(mirrored)) = shape_primitive(&left_cap, &placement(-1.0)) else {
        panic!("the mirrored cap did not build");
    };
    let (x, y) = midpoint(&mirrored);
    assert!(
        (x - (41.18 - 35.59)).abs() < 1e-6 && (y + 88.90).abs() < 1e-6,
        "the mirrored cap still bulges out of the shell: ({x}, {y})"
    );
}

#[test]
fn symbol_polyline_closure_reaches_the_cad_entity() {
    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let placement = Placement {
        insertion: &insertion,
        rotation: 0.0,
        scale: [1.0, 1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let primitive = SymbolPrimitive::Polyline {
        vertices: vec![(0.0, 0.0), (0.01, 0.0), (0.01, 0.01)],
        is_closed: true,
    };
    let Some(EntityType::LwPolyline(polyline)) = shape_primitive(&primitive, &placement) else {
        panic!("symbol polyline did not build");
    };
    assert!(polyline.is_closed);
}

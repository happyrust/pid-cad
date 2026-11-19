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
    summary.label_anchors_undrawn = 4;
    summary.lettering_flattened = 10;
    summary.symbol_library = vec![
        PathBuf::from(r"\\server\Plant\Ref\Symbols"),
        PathBuf::from("D:/sym with spaces/=and=equals"),
    ];
    summary.unit = ImportUnit::Stated {
        unit: "m".to_string(),
        mm_per_unit: MM_PER_METRE,
    };

    let mut doc = Document::new();
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
    let mut arc = document::entities::Arc::new();
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
    let mut arc = document::entities::Arc::new();
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
        primitive_letterings: Vec::new(),
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
    let entities = cached_body_entities(
        Some(&body),
        &at,
        &mut tally,
        &HashMap::new(),
        1.0,
        &HashMap::new(),
    )
    .expect("three strokes draw");
    assert_eq!(entities.len(), 3);
    assert_eq!(
        (tally.cache, tally.hidden_strokes_skipped),
        (1, 2),
        "one body drawn, its two Construction strokes counted as skipped"
    );
}

/// A label symbol is one SmartPlant files under a `Labels` folder -- plain
/// or `Labels - <class>` -- or the one this plant files beside its sheet
/// templates, matched by file name; the folder is read off the directory
/// segments, so a file whose name carries the word is not one, nor a
/// folder that only starts with it, and a bare `Labels` with no file is
/// nothing at all (plan 2026-10-10, label anchors).
#[test]
fn a_label_symbol_is_one_filed_under_a_labels_folder_or_named_as_filed_elsewhere() {
    for path in [
        r"\\WIN-SPID\qsmcqtaz13\Plant\Ref\Symbols\Design\Annotation\Labels\Item Note & Label.sym",
        r"\\SPID\XaLNG\P&ID Reference Data\Symbols\Piping\Labels - Piping Segments\Remarks.sym",
        r"\\WIN-SPID\x\Plant\Ref\Symbols\Equipment\Labels - Equipment\Descriptions\Vessel Description.sym",
        r"\\WIN-SPID\qsmcqtaz13\Plant\Ref\Symbols\Design\Drawing Description.sym",
        "Design/Annotation/Labels/Item Note & Label line1.5.sym",
        r"Design\DRAWING DESCRIPTION.SYM",
        "Drawing Description.sym",
    ] {
        assert!(is_label_symbol(path), "{path} is a label symbol");
    }
    for path in [
        r"\\WIN-SPID\qsmcqtaz13\Plant\Ref\Symbols\Piping\Piping OPC's\Off-Drawing.sym",
        r"\\WIN-SPID\qsmcqtaz13\Plant\Ref\Symbols\Equipment\Other Equipment\Parametric Manifold.sym",
        r"\\WIN-SPID\x\Plant\Ref\Symbols\Piping\Valves\Labels Valve.sym",
        r"\\WIN-SPID\x\Plant\Ref\Symbols\Piping\Labelsmith\Valve.sym",
        r"\\WIN-SPID\x\Plant\Ref\Symbols\Design\ Line2.sym",
        "Labels",
        "",
    ] {
        assert!(!is_label_symbol(path), "{path:?} is not a label symbol");
    }
}

/// A body is nothing but its anchor cross when every stroke the file
/// displays is a line narrower than any ISO pen: the two 0.01 mm lines of
/// an `Item Note & Label` body are, the same cross at 0.13 mm is not, a
/// cross with a hidden `NULL` beside it still is (the lettering is on a
/// switched-off layer), a stroke with no style of its own is not a
/// hairline, and a body with nothing displayed is not a cross (plan
/// 2026-10-10, label anchors).
#[test]
fn a_body_is_an_anchor_cross_when_its_displayed_strokes_are_all_hairline_lines() {
    use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};
    let hairline = |width_mm: f64| {
        Some(PrimitiveStyle {
            rgb: [0, 0, 0],
            width_mm,
            dash_mm: Vec::new(),
            paper: false,
        })
    };
    let cross = vec![
        SymbolPrimitive::Line {
            start: (0.10541, 0.13970),
            end: (0.10795, 0.13970),
        },
        SymbolPrimitive::Line {
            start: (0.10668, 0.14097),
            end: (0.10668, 0.13843),
        },
    ];
    let body = |primitives: Vec<SymbolPrimitive>,
                layers: Vec<u32>,
                styles: Vec<Option<PrimitiveStyle>>| PidSymbolDefinition {
        reference: PidSymbolDefinitionRef {
            site: 329,
            sheet: 327,
        },
        layers: vec![334, 332],
        sheet_layers: vec![
            PidSymbolSheetLayer {
                oid: 334,
                name: Some("Default".to_string()),
                displayed: Some(true),
            },
            PidSymbolSheetLayer {
                oid: 332,
                name: Some("Label".to_string()),
                displayed: Some(false),
            },
        ],
        primitives,
        primitive_layers: layers,
        primitive_styles: styles,
        primitive_letterings: Vec::new(),
        dimensions: Vec::new(),
        variables: Vec::new(),
        template: None,
    };

    assert!(
        is_anchor_cross_only(&body(
            cross.clone(),
            vec![334, 334],
            vec![hairline(0.01), hairline(0.01)]
        )),
        "two 0.01 mm lines on the displayed layer"
    );
    let mut with_lettering = cross.clone();
    with_lettering.push(SymbolPrimitive::Text {
        text: "NULL".to_string(),
        at: (0.10668, 0.13970),
    });
    assert!(
        is_anchor_cross_only(&body(
            with_lettering,
            vec![334, 334, 332],
            vec![hairline(0.01), hairline(0.01), None]
        )),
        "the template lettering sits on the switched-off Label layer"
    );
    assert!(
        !is_anchor_cross_only(&body(
            cross.clone(),
            vec![334, 334],
            vec![hairline(0.01), hairline(0.13)]
        )),
        "a 0.13 mm line is drafting"
    );
    assert!(
        !is_anchor_cross_only(&body(
            cross.clone(),
            vec![334, 334],
            vec![hairline(0.01), None]
        )),
        "a stroke with no style of its own cannot be called a hairline"
    );
    assert!(
        !is_anchor_cross_only(&body(
            cross.clone(),
            vec![332, 332],
            vec![hairline(0.01), hairline(0.01)]
        )),
        "a body with nothing displayed is not a cross"
    );
    assert!(
        !is_anchor_cross_only(&body(
            vec![SymbolPrimitive::Circle {
                center: (0.0, 0.0),
                radius: 0.001,
            }],
            vec![334],
            vec![hairline(0.01)]
        )),
        "a hairline circle is not the cross"
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
        paper: false,
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
        primitive_letterings: Vec::new(),
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
            paper: false,
        },
        dash,
        marker: None,
        complex_dash: None,
        hop: StyleHop::Direct,
    };
    let sheet_dash = || DashPattern::from_segments_m(&[0.001, 0.001]);

    let mut styles = LineStyleIndex::new();
    styles.insert(("/".to_string(), 1), placement_style(sheet_dash()));
    let mut doc = Document::new();
    let pool = register_dash_linetypes(&mut doc, &styles, std::slice::from_ref(&body), 1.0);
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
    let mut drawn = cached_body_entities(Some(&body), &at, &mut tally, &pool, 1.0, &HashMap::new())
        .expect("three strokes draw");
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
            LineWeight::Value(34),
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
        apply_symbology(entity, &placement_style(None), &pool, 1.0);
    }
    assert_eq!(
        coat(&drawn[0]),
        (olive, LineWeight::Value(34), "PID-DASH-2".to_string()),
        "a solid placement style repaints colour and width and leaves the dash"
    );
    assert_eq!(
        coat(&drawn[1]),
        (olive, LineWeight::Value(34), as_built.clone())
    );
    assert_eq!(coat(&drawn[2]), (olive, LineWeight::Value(34), as_built));

    for entity in &mut drawn {
        apply_symbology(entity, &placement_style(sheet_dash()), &pool, 1.0);
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
            paper: false,
        },
        dash: None,
        marker: None,
        complex_dash: None,
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
    apply_symbology(&mut on_symbol, &green, &no_dashes, 1.0);
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
    apply_symbology(&mut on_sheet, &green, &no_dashes, 1.0);
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
        clockwise: false,
    };
    let midpoint = |arc: &document::entities::Arc| {
        let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
        let angle = arc.start_angle + sweep / 2.0;
        (
            arc.center.x + arc.radius * angle.cos(),
            arc.center.y + arc.radius * angle.sin(),
        )
    };

    let Some(EntityType::Arc(upright)) =
        shape_primitive(&left_cap, None, &placement(1.0), &HashMap::new())
    else {
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
    let Some(EntityType::Arc(mirrored)) =
        shape_primitive(&left_cap, None, &placement(-1.0), &HashMap::new())
    else {
        panic!("the mirrored cap did not build");
    };
    let (x, y) = midpoint(&mirrored);
    assert!(
        (x - (41.18 - 35.59)).abs() < 1e-6 && (y + 88.90).abs() < 1e-6,
        "the mirrored cap still bulges out of the shell: ({x}, {y})"
    );
}

/// A half-turn cap whose file angles are one ulp past `f64::consts::PI`
/// is still a half-turn: `Parametric V Drum`'s left head reads `π -> 0`
/// with `π` stored as `3.141592653589794`, and a bare `sweep > PI`
/// comparison flipped it into its complement (the head caved into the
/// shell). With the epsilon, a placement rotated a quarter turn draws
/// the cap bulging left.
#[test]
fn a_half_turn_cap_within_one_ulp_of_pi_keeps_its_clockwise_half() {
    use std::f64::consts::PI;
    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let placement = Placement {
        insertion: &insertion,
        rotation: 1.571,
        scale: [1.0, 1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let head = SymbolPrimitive::Arc {
        center: (0.1016, 0.22959),
        radius: 0.02691,
        start_angle: 3.141592653589794,
        end_angle: 0.0,
        clockwise: false,
    };
    let Some(EntityType::Arc(arc)) = shape_primitive(&head, None, &placement, &HashMap::new())
    else {
        panic!("the head did not build");
    };
    let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
    let mid = arc.start_angle + sweep / 2.0;
    let mid_x = arc.center.x + arc.radius * mid.cos();
    assert!(
        mid_x < arc.center.x - 1e-6,
        "the rotated head must bulge left of its chord: mid_x {mid_x} vs chord x {}",
        arc.center.x
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
    let Some(EntityType::LwPolyline(polyline)) =
        shape_primitive(&primitive, None, &placement, &HashMap::new())
    else {
        panic!("symbol polyline did not build");
    };
    assert!(polyline.is_closed);
}

/// A `.sym`'s own lettering: the record's anchor byte picks the hot point,
/// the style's height and typeface letter it, and the placement's rotation
/// and scale carry through. `0x15` is centre-on-middle, `0x01` left-on-top —
/// the two anchors the library's lettering is written at.
#[test]
fn symbol_text_letters_at_its_own_style_and_anchor() {
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
    let primitive = SymbolPrimitive::Text {
        text: "I".to_string(),
        at: (0.001, 0.004),
    };
    let lettering = SymbolLettering {
        anchor_code: 0x15,
        rotation_rad: 0.0,
        letter_height_m: 0.0,
        height_m: Some(0.003_175),
        height_mode: None,
        width_permille: None,
        rgb: Some([0, 0, 0]),
        alignment: Some(TextAlignment::Left),
        line_spacing: Some(1.0),
        run_style_id: Default::default(),
        spans: Default::default(),
        font_name: Some("Arial".to_string()),
    };
    let font_styles: HashMap<String, String> = [("Arial".to_string(), "PID-Arial".to_string())]
        .into_iter()
        .collect();
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, Some(&lettering), &placement, &font_styles)
    else {
        panic!("symbol text did not build");
    };
    assert!(
        (label.height - 3.175).abs() < 1e-9,
        "the character style's 3.175 mm: {}",
        label.height
    );
    assert_eq!(label.horizontal_alignment, TextHorizontalAlignment::Center);
    assert_eq!(label.vertical_alignment, TextVerticalAlignment::Middle);
    assert_eq!(
        label.alignment_point,
        Some(label.insertion_point),
        "the anchor moved the origin to alignment_point"
    );
    assert_eq!(label.style, "PID-Arial");

    let left_top = SymbolLettering {
        anchor_code: 0x01,
        ..lettering.clone()
    };
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, Some(&left_top), &placement, &font_styles)
    else {
        panic!("symbol text did not build");
    };
    assert_eq!(label.horizontal_alignment, TextHorizontalAlignment::Left);
    assert_eq!(label.vertical_alignment, TextVerticalAlignment::Top);

    // A record that scales its direction pair to the letter height states
    // the size itself, and it beats the style's: `letter_height_m` is what
    // the record letters at.
    let self_sized = SymbolLettering {
        letter_height_m: 0.002,
        height_m: Some(0.003_175),
        ..lettering.clone()
    };
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, Some(&self_sized), &placement, &font_styles)
    else {
        panic!("symbol text did not build");
    };
    assert!(
        (label.height - 2.0).abs() < 1e-9,
        "the direction pair's own 2 mm: {}",
        label.height
    );

    // No lettering at all keeps the scaled ISO fallback the record used
    // to draw with.
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, None, &placement, &HashMap::new())
    else {
        panic!("symbol text did not build");
    };
    assert!(
        (label.height - 2.5).abs() < 1e-9,
        "the 2.5 mm fallback: {}",
        label.height
    );
}

/// A mirrored placement still prints readable lettering: SmartPlant
/// normalizes the glyph rotation into `[-pi/2, pi/2)` and flips the anchor
/// the way the dropped mirror would have flipped it. The ESD interlock sits
/// at a pi-rotation mirror and its label reads upright in the reference; a
/// quarter-turn mirror keeps right-reading vertical text.
#[test]
fn mirrored_placement_lettering_stays_readable() {
    let insertion = PidPoint { x: 0.0, y: 0.0 };
    let lettering = SymbolLettering {
        anchor_code: 0x01,
        rotation_rad: 0.0,
        letter_height_m: 0.0,
        height_m: Some(0.003_175),
        height_mode: None,
        width_permille: None,
        rgb: Some([0, 0, 0]),
        alignment: Some(TextAlignment::Left),
        line_spacing: Some(1.0),
        run_style_id: Default::default(),
        spans: Default::default(),
        font_name: None,
    };
    let primitive = SymbolPrimitive::Text {
        text: "ESD".to_string(),
        at: (0.0, 0.0),
    };

    // A pi-rotation mirror (scale y negated): the label emits upright, and
    // its left anchor flips right — the mirror the rotation absorbed.
    let pi_mirror = Placement {
        insertion: &insertion,
        rotation: std::f64::consts::PI,
        scale: [1.0, -1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, Some(&lettering), &pi_mirror, &HashMap::new())
    else {
        panic!("mirrored symbol text did not build");
    };
    assert!(
        label.rotation.abs() < 1e-9,
        "the pi mirror emits upright lettering: {}",
        label.rotation
    );
    assert_eq!(label.horizontal_alignment, TextHorizontalAlignment::Right);
    assert_eq!(label.vertical_alignment, TextVerticalAlignment::Top);

    // A quarter-turn mirror keeps the right-reading -pi/2 direction and
    // flips the vertical anchor instead.
    let quarter_mirror = Placement {
        insertion: &insertion,
        rotation: -std::f64::consts::FRAC_PI_2,
        scale: [1.0, -1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let Some(EntityType::Text(label)) = shape_primitive(
        &primitive,
        Some(&lettering),
        &quarter_mirror,
        &HashMap::new(),
    ) else {
        panic!("mirrored symbol text did not build");
    };
    assert!(
        (label.rotation + std::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "the quarter-turn mirror keeps -pi/2: {}",
        label.rotation
    );
    assert_eq!(label.vertical_alignment, TextVerticalAlignment::Bottom);

    // The same rotation without the mirror is left alone: a pi-rotated
    // placement letters upside down the way the file placed it.
    let unmirrored = Placement {
        insertion: &insertion,
        rotation: std::f64::consts::PI,
        scale: [1.0, 1.0],
        projection: Projection {
            mm_per_unit: 1000.0,
            band: SheetBand::for_page(None),
        },
    };
    let Some(EntityType::Text(label)) =
        shape_primitive(&primitive, Some(&lettering), &unmirrored, &HashMap::new())
    else {
        panic!("symbol text did not build");
    };
    assert!(
        (label.rotation - std::f64::consts::PI).abs() < 1e-9,
        "unmirrored lettering keeps the placement rotation: {}",
        label.rotation
    );
    assert_eq!(label.horizontal_alignment, TextHorizontalAlignment::Left);
    assert_eq!(label.vertical_alignment, TextVerticalAlignment::Top);
}

/// A plug valve's boundary holds concentric circle members, and SmartPlant
/// prints every ring as its own solid disc — so the boundary builds one
/// hatch per circle rather than one compound path the renderer would read
/// as a ring with holes.
#[test]
fn concentric_boundary_circles_build_a_solid_hatch_each() {
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
    let circle = |radius| SymbolPrimitive::Circle {
        center: (0.0, 0.0),
        radius,
    };
    let primitive = SymbolPrimitive::Boundary {
        members: vec![circle(0.00034), circle(0.00065), circle(0.00095)],
        fill: Some(pid_parse::symbol_library::PrimitiveFill {
            rgb: Some([0, 0, 255]),
            type_code: 0x002A,
        }),
    };
    let entities = shape_primitives(&primitive, None, &placement, &HashMap::new());
    assert_eq!(entities.len(), 3, "one hatch per circle member");
    let mut radii: Vec<f64> = entities
        .iter()
        .map(|entity| {
            let EntityType::Hatch(hatch) = entity else {
                panic!("a circle member builds a hatch");
            };
            assert_eq!(hatch.paths.len(), 1, "the disc is a single path");
            let Some(BoundaryEdge::CircularArc(arc)) = hatch.paths[0].edges.first() else {
                panic!("the circle member's path is a circular arc");
            };
            arc.radius
        })
        .collect();
    radii.sort_by(f64::total_cmp);
    let expected = [0.34, 0.65, 0.95];
    for (got, want) in radii.iter().zip(expected) {
        assert!((got - want).abs() < 1e-6, "radius {want}: {got}");
    }
}

/// `paper_style_scale` multiplies style-declared dash lengths, line weights,
/// and point-marker stroke offsets, and leaves lettering at the height the
/// file states. Scale 1 is the unscaled reading; 5/3 is the screen multiple
/// measured on the reference sheet.
#[test]
fn paper_style_scale_multiplies_declared_sizes_and_leaves_lettering() {
    use pid_parse::style_link::{LineSymbology, StyleHop};

    let scale = 5.0 / 3.0;
    assert_eq!(effective_paper_style_scale(scale), scale);
    assert_eq!(effective_paper_style_scale(1.0), 1.0);
    assert_eq!(effective_paper_style_scale(0.0), 1.0);
    assert_eq!(effective_paper_style_scale(-1.0), 1.0);
    assert_eq!(effective_paper_style_scale(f64::NAN), 1.0);
    assert_eq!(effective_paper_style_scale(f64::INFINITY), 1.0);

    let style = |width_mm: f64| ResolvedLineStyle {
        style_id: 1,
        symbology: LineSymbology {
            width_m: width_mm / 1000.0,
            colour: 0,
            paper: false,
        },
        dash: None,
        marker: None,
        complex_dash: None,
        hop: StyleHop::Direct,
    };
    let weight = |width_mm: f64, k: f64| {
        let mut line = Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0));
        line.common.layer = LAYER_GEOMETRY.to_string();
        let mut entity = EntityType::Line(line);
        apply_symbology(&mut entity, &style(width_mm), &HashMap::new(), k);
        entity.common().line_weight
    };
    // Declared widths land on the plotter's three pens — 0.169/0.339/0.678mm
    // (17/34/68 hundredths), thin to 0.30mm and medium to 0.55mm — so 0.35mm
    // is the medium pen at 1:1 and heavy once the scale carries it past
    // 0.55mm.
    for (mm, at_one, at_scale) in [
        (0.35, 34_i16, 68),
        (0.13, 17, 17),
        (0.18, 17, 17),
        (0.70, 68, 68),
    ] {
        assert_eq!(
            weight(mm, 1.0),
            LineWeight::Value(at_one),
            "{mm} mm at scale 1"
        );
        assert_eq!(
            weight(mm, scale),
            LineWeight::Value(at_scale),
            "{mm} mm at 5/3"
        );
    }

    let magnitudes = |segments: &[f64], k: f64| {
        build_dash_linetype("PID-DASH", segments, k)
            .elements
            .iter()
            .map(|element| (element.length.abs() * 1000.0).round() as i64)
            .collect::<Vec<_>>()
    };
    assert_eq!(magnitudes(&[3.5, 1.75], 1.0), vec![3500, 1750]);
    assert_eq!(magnitudes(&[3.5, 1.75], scale), vec![5833, 2917]);
    assert_eq!(magnitudes(&[1.905, 1.27], 1.0), vec![1905, 1270]);
    assert_eq!(magnitudes(&[1.905, 1.27], scale), vec![3175, 2117]);

    // The pool is keyed by the unscaled pattern, so the name does not move
    // when the stored segment lengths do.
    let dashed = |id: u32, segments_m: &[f64]| ResolvedLineStyle {
        style_id: id,
        symbology: LineSymbology {
            width_m: 0.000_35,
            colour: 0,
            paper: false,
        },
        dash: DashPattern::from_segments_m(segments_m),
        marker: None,
        complex_dash: None,
        hop: StyleHop::Direct,
    };
    let mut styles = LineStyleIndex::new();
    styles.insert(("/".to_string(), 1), dashed(1, &[0.0035, 0.00175]));
    styles.insert(("/".to_string(), 2), dashed(2, &[0.001_905, 0.001_27]));
    let mut doc = Document::new();
    let pool = register_dash_linetypes(&mut doc, &styles, &[], scale);
    assert_eq!(
        pool.get(&vec![3500, 1750]).map(String::as_str),
        Some("PID-DASH-1"),
        "the 3.5/1.75 key is the unscaled pattern"
    );
    assert_eq!(
        pool.get(&vec![1905, 1270]).map(String::as_str),
        Some("PID-DASH-2"),
        "the 1.905/1.27 key is the unscaled pattern"
    );
    let stored = |name: &str| {
        doc.line_types
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing"))
            .elements
            .iter()
            .map(|element| (element.length.abs() * 1000.0).round() as i64)
            .collect::<Vec<_>>()
    };
    assert_eq!(stored("PID-DASH-1"), vec![5833, 2917]);
    assert_eq!(stored("PID-DASH-2"), vec![3175, 2117]);

    let mut stroke = Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0));
    stroke.common.layer = LAYER_SYMBOL.to_string();
    let mut entity = EntityType::Line(stroke);
    paint_symbol_stroke(
        &mut entity,
        Some(&PrimitiveStyle {
            rgb: [0, 0, 0],
            width_mm: 0.35,
            dash_mm: vec![3.5, 1.75],
            paper: false,
        }),
        &pool,
        scale,
    );
    assert_eq!(entity.common().line_weight, LineWeight::Value(68));
    assert_eq!(entity.common().linetype, "PID-DASH-1");

    let mut text = Text::new();
    text.value = "LIA".to_string();
    text.height = 2.5;
    text.common.layer = LAYER_SYMBOL.to_string();
    let mut lettering = EntityType::Text(text);
    apply_symbology(&mut lettering, &style(0.18), &HashMap::new(), scale);
    let EntityType::Text(text) = &lettering else {
        panic!("lettering is still text");
    };
    assert_eq!(text.height, 2.5, "text height is not multiplied");
    assert_eq!(
        lettering.common().line_weight,
        Text::new().common.line_weight,
        "lettering takes no line weight"
    );

    let projection = Projection {
        mm_per_unit: 1000.0,
        band: SheetBand::for_page(None),
    };
    let origin = Vector3::new(10.0, 20.0, 0.0);
    let length = |k: f64, end: (f64, f64)| {
        let at = point_marker_offset(origin, end, projection, k);
        (at.x - origin.x).hypot(at.y - origin.y)
    };
    // (0, 0) -> (3, 6) mm, the long stroke of the warning slash.
    let long = length(1.0, (0.003, 0.006));
    assert!((long - 6.708_203_932_499_369).abs() < 1e-9, "{long}");
    assert!((length(scale, (0.003, 0.006)) - long * scale).abs() < 1e-9);
    // (-1, -2) -> (-0.7, -1) mm, the stub.
    let stub_at = |k: f64, xy: (f64, f64)| point_marker_offset(origin, xy, projection, k);
    let stub = stub_at(1.0, (-0.0007, -0.001));
    let stub_from = stub_at(1.0, (-0.001, -0.002));
    let stub_len = (stub.x - stub_from.x).hypot(stub.y - stub_from.y);
    let stub_k = stub_at(scale, (-0.0007, -0.001));
    let stub_from_k = stub_at(scale, (-0.001, -0.002));
    let stub_len_k = (stub_k.x - stub_from_k.x).hypot(stub_k.y - stub_from_k.y);
    assert!((stub_len_k - stub_len * scale).abs() < 1e-9);
}

/// Authored paper ink becomes the contrast colour: `RGB(0,0,0)` stored as
/// ACI 7 reads white on the dark canvas (`adapt_to_bg`) and black on the
/// plotted sheet, where the true colour it used to be stayed invisible in
/// the former. `RGB(255,255,255)` is the same foreground pen -- SmartPlant
/// prints the `.igr`'s white-stated frame drafting black. Every other
/// stated colour is the true colour it is.
#[test]
fn authored_black_is_the_paper_foreground_colour() {
    assert_eq!(foreground_color(0, 0, 0), Color::Index(7));
    assert_eq!(
        foreground_color(0xFF, 0x00, 0x00),
        Color::from_rgb(0xFF, 0x00, 0x00)
    );
    assert_eq!(
        foreground_color(0xFF, 0xFF, 0xFF),
        Color::Index(7),
        "stated white is the same foreground pen: the linked .igr letters its \
         frame drafting white and SmartPlant's own PDFs print it black"
    );

    let mut text = Text::new();
    text.value = "TITLE".to_string();
    text.common.layer = LAYER_TEXT.to_string();
    let mut lettering = EntityType::Text(text);
    apply_text_colour(&mut lettering, [0, 0, 0]);
    assert_eq!(
        lettering.common().color,
        Color::Index(7),
        "the sheet's black lettering is the contrast colour"
    );
    let mut text = Text::new();
    text.value = "LGM".to_string();
    text.common.layer = LAYER_SYMBOL.to_string();
    text.common.color = Color::from_rgb(0xFF, 0x00, 0x00);
    let mut symbol_lettering = EntityType::Text(text);
    apply_text_colour(&mut symbol_lettering, [0, 0, 0]);
    assert_eq!(
        symbol_lettering.common().color,
        Color::from_rgb(0xFF, 0x00, 0x00),
        "a symbol's internal lettering keeps what the .sym authored"
    );
}

/// An OLE link names its target several ways -- the absolute path the
/// linking machine saw, a relative tail, a bare name; all of them answer
/// the file name the local copy is found by.
#[test]
fn a_template_link_is_named_by_its_basename() {
    assert_eq!(
        template_basename(r"C:\PID_SQProject\SQPlant\Ref\Template Files\CPE-SQ-A1.igr"),
        "CPE-SQ-A1.igr"
    );
    assert_eq!(
        template_basename(r"\\WIN-SPID\share\Ref\Template Files\CPECCHBA2.igr"),
        "CPECCHBA2.igr"
    );
    assert_eq!(
        template_basename("Ref\\Template Files\\XA-A1.igr"),
        "XA-A1.igr"
    );
    assert_eq!(template_basename("CPE-SQ-A0.igr"), "CPE-SQ-A0.igr");
    assert_eq!(template_basename("  XA-A1.igr  "), "XA-A1.igr");
}

/// `.igr` links come off every `JSite`'s OLE stream: whatever the site's
/// symbol is for, its `\x01Ole` strings are scanned for template targets.
#[test]
fn linked_templates_are_the_igr_links_of_every_jsite() {
    let mut parsed = pid_parse::PidDocument::default();
    let site = |ole_links: Vec<String>| pid_parse::JSite {
        name: "JSite1".to_string(),
        ole_links,
        ..Default::default()
    };
    assert!(linked_templates(&parsed).is_empty());
    parsed.jsites.push(site(vec![
        r"C:\Plant\Ref\Template Files\A1.igr".to_string(),
        "A1.igr".to_string(),
        "Sheet1".to_string(),
    ]));
    parsed.jsites.push(site(vec![
        r"\\share\Ref\Template Files\A0.IGR".to_string(),
        "not-a-template.xls".to_string(),
    ]));
    assert_eq!(
        linked_templates(&parsed),
        vec![
            "A1.igr".to_string(),
            r"C:\Plant\Ref\Template Files\A1.igr".to_string(),
            r"\\share\Ref\Template Files\A0.IGR".to_string()
        ],
        "deduplicated, sorted, only .igr"
    );
}

/// The stated path wins when it exists on this machine; the basename search
/// under the roots is the fallback for a link authored elsewhere.
#[test]
fn a_link_resolves_directly_or_by_basename_under_a_root() {
    let temp = std::env::temp_dir().join(format!(
        "pid-template-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let nested = temp.join("Ref").join("Template Files");
    std::fs::create_dir_all(&nested).expect("temp dirs");
    let local = nested.join("Local-A1.igr");
    std::fs::write(&local, b"stub").expect("write");

    let roots = vec![temp.clone()];
    assert_eq!(
        resolve_template_path(&local.to_string_lossy(), &roots).as_deref(),
        Some(local.as_path()),
        "a path that exists resolves to itself"
    );
    assert_eq!(
        resolve_template_path(r"C:\elsewhere\Ref\Template Files\Local-A1.igr", &roots).as_deref(),
        Some(local.as_path()),
        "a foreign absolute path resolves by basename"
    );
    assert_eq!(
        resolve_template_path(r"C:\elsewhere\Ref\Template Files\Missing.igr", &roots),
        None
    );
    assert!(
        resolve_template_path("", &roots).is_none(),
        "an empty link has no basename"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

/// The linked `.igr` parses through the same pipeline as the drawing and
/// overlays onto `PID-FRAME`: the fixture template draws its closed page
/// frame (so the synthetic border stands down), its title-block and
/// margin-strip lines, and reports the records it cannot draw beside the
/// drawing's own `missing` rather than in it.
#[test]
fn a_linked_igr_overlays_its_drafting_on_pid_frame() {
    let igr = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|dir| {
            dir.join("pid-parse")
                .join("test-file")
                .join("Ref")
                .join("Template Files")
                .join("CPE-SQ-A1.igr")
        })
        .filter(|path| path.is_file());
    let Some(igr) = igr else {
        return; // fixture absent: soft-skip like the corpus tests do
    };
    let mut doc = Document::new();
    // The drawing's own normalized geometry is only consulted for its page:
    // a frame of the template that covers it stands the synthetic border
    // down. Which template a sheet wears is `pick_linked_template`'s call,
    // made on the strokes measured here.
    let drawing = NormalizedPidGeometry {
        page_dimensions_mm: Some((841.0, 594.0)),
        ..Default::default()
    };
    let template = read_template(&igr, false).expect("the fixture parses");
    let (width, height) = template
        .candidate()
        .stroked_mm
        .expect("the fixture has strokes to measure");
    assert!(
        (width - 841.0).abs() < 0.01 && (height - 594.0).abs() < 0.01,
        "the A1 template's strokes span the A1 sheet, not {width} x {height}"
    );
    let overlay = overlay_template(&template, &drawing, &mut doc, 1.0).expect("the fixture draws");
    assert!(overlay.drawn > 50, "a title block is more than a few lines");
    assert!(
        overlay.page_frame_covered,
        "the template draws the page frame the synthetic border doubles"
    );
    let frames: Vec<_> = doc
        .entities()
        .filter(|entity| entity.common().layer == LAYER_FRAME)
        .collect();
    assert_eq!(
        frames.len(),
        overlay.drawn,
        "every template entity files on PID-FRAME"
    );
    // The page rectangle the synthetic border would have drawn is among the
    // template's own drafting -- however the `.igr` files it: a closed
    // polyline or the four lines of the frame.
    let spans_page = frames
        .iter()
        .filter_map(|entity| page::drawn_extent(entity))
        .any(|(min_x, min_y, max_x, max_y)| {
            (max_x - min_x - 841.0).abs() < 2.0 && (max_y - min_y - 594.0).abs() < 2.0
        });
    assert!(spans_page, "the frame covers the A1 page");
    // Provenance: every overlay entity says which template drew it.
    let tagged = frames
        .iter()
        .filter(|entity| {
            entity
                .common()
                .extended_data
                .records()
                .iter()
                .flat_map(|record| record.values.iter())
                .any(|value| {
                    matches!(value, document::xdata::XDataValue::String(s) if s == "template=CPE-SQ-A1.igr")
                })
        })
        .count();
    assert_eq!(
        tagged, overlay.drawn,
        "every template entity names the .igr it came from"
    );
}

/// The importer finds a drawing's symbols by walking up from the drawing to
/// a `Ref` folder. A backup as SmartPlant leaves it has none: its symbol
/// library is a numbered volume. Restored, the volume is back under `Ref`
/// beside the drawings, and the walk finds it.
#[test]
fn a_restored_backup_has_the_symbol_library_its_drawings_look_for() {
    use std::io::Write;
    // The name says nothing of symbols, and the restores lie two folders
    // down: the walk goes five levels up from a drawing and takes every
    // folder with "symbol" in its name for a library, the temp folder's too.
    let root = std::env::temp_dir().join(format!("ocs-pid-restored-refdata-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let deep = root.join("a").join("b");
    let backup = deep.join("backup");
    std::fs::create_dir_all(&backup).unwrap();
    let archive = |name: &str, members: &[(&str, &[u8])]| {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(backup.join(name)).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (member, bytes) in members {
            writer.start_file(*member, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    };
    std::fs::write(
        backup.join("Manifest.txt"),
        "Name<<|>>SQPlant\r\n\
         File<<|>>2<<|>>1<<|>>711<<|>>1<<|>>\\\\host\\P\\Plant<<|>>1<<|>>PlantData~2~711.zip\r\n\
         File<<|>>4<<|>>1<<|>>681<<|>>1<<|>>\\\\host\\P\\Ref\\Symbols<<|>>1<<|>>RefData~4~681.zip\r\n",
    )
    .unwrap();
    archive("PlantData~2~711.zip", &[("00/00/A.pid", b"a")]);
    archive("RefData~4~681.zip", &[("Piping/Valves/gate.sym", b"gate")]);

    // The library roots the walk finds under `folder` (the environment may
    // name others, which are not this test's business).
    let found_under = |drawing: &Path, folder: &Path| -> Vec<PathBuf> {
        super::symbols::discover_symbol_library(drawing)
            .map(|library| {
                library
                    .roots()
                    .iter()
                    .filter(|root| root.starts_with(folder))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    let target = deep.join("out");
    let restored = crate::io::pid_backup::restore(&backup, Some(&target)).unwrap();
    let drawing = restored.drawings.first().expect("the drawing is restored");
    assert_eq!(found_under(drawing, &root), [target.join("Ref").join("Symbols")]);
    assert!(target.join("Ref/Symbols/Piping/Valves/gate.sym").is_file());

    // The same backup with its volume taken away: nothing to find.
    std::fs::remove_file(backup.join("RefData~4~681.zip")).unwrap();
    let bare = deep.join("bare");
    let restored = crate::io::pid_backup::restore(&backup, Some(&bare)).unwrap();
    let drawing = restored.drawings.first().expect("the drawing is restored");
    assert!(found_under(drawing, &root).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// A symbol placement is filed as one block reference (plan
/// 2026-11-19-pid-symbol-block, SB-2): what the placement says is on the
/// reference, what it draws is the block's definition, and every other
/// record of the sheet stays the loose entity it was.
pub(super) mod placements_as_blocks {
    use crate::io::pid_view_filter::{switch_role, switch_sheet_layer, PidViewRow, Switched};
    use document::entities::Insert;
    use pid_parse::style_link::{LineSymbology, StyleHop};
    use pid_parse::symbol_library::PrimitiveFill;
    use pid_parse::{PidSymbolDefinitionRef, PidSymbolSheetLayer};

    use super::super::*;

    /// The stream the records of these sheets are read from: half of the
    /// key the style tables are joined on.
    const STREAM: &str = "/Sheet6";
    const VALVE: &str = r"\\SRV\Ref\Symbols\Piping\Valves\Ball Valve.sym";
    const NOTE: &str = r"\\SRV\Ref\Symbols\Design\Annotation\Labels\Item Note.sym";
    const BODY: PidSymbolDefinitionRef = PidSymbolDefinitionRef { site: 40, sheet: 7 };
    const CROSS: PidSymbolDefinitionRef = PidSymbolDefinitionRef { site: 41, sheet: 9 };

    /// The sheet layer a record is filed on -- the name the drawing resolves
    /// for it, if any, and whether the file displays it -- or `None` for a
    /// record that states no layer.
    type OnLayer = Option<(Option<&'static str>, bool)>;

    fn point(x: f64, y: f64) -> PidPoint {
        PidPoint { x, y }
    }

    /// One decoded record of the sheet: graphic oid `oid`, sheet layer 12.
    fn record(oid: u32, kind: PidGraphicKind, layer: OnLayer) -> pid_parse::PidGraphicEntity {
        pid_parse::PidGraphicEntity {
            id: format!("e{oid}"),
            drawing_id: None,
            graphic_oid: Some(oid),
            source_layer: layer.map(|(name, displayed)| PidSourceLayer {
                oid: 12,
                name: name.map(str::to_string),
                storage_path: "/".to_string(),
                displayed: Some(displayed),
            }),
            kind,
            coordinate_context: pid_parse::PidCoordinateContext::default(),
            source: pid_parse::PidGraphicProvenance {
                stream_path: Some(STREAM.to_string()),
                byte_range: None,
                record_id: None,
                record_kind: None,
                field_x: None,
                note: None,
            },
            confidence: PidGeometryConfidence::Decoded,
        }
    }

    /// A placement of the symbol filed at `path`, unscaled.
    fn placement(
        at: (f64, f64),
        path: Option<&str>,
        definition: Option<PidSymbolDefinitionRef>,
        rotation: f64,
    ) -> PidGraphicKind {
        PidGraphicKind::SymbolInstance {
            insertion: point(at.0, at.1),
            symbol_path: path.map(str::to_string),
            rotation,
            scale: [1.0, 1.0],
            definition,
        }
    }

    /// A black, solid stroke style of the given width.
    fn pen(width_mm: f64) -> Option<PrimitiveStyle> {
        Some(PrimitiveStyle {
            rgb: [0, 0, 0],
            width_mm,
            dash_mm: Vec::new(),
            paper: false,
        })
    }

    /// A cached symbol body on one layer the file displays, each stroke in
    /// the style its own storage states for it.
    fn body(
        reference: PidSymbolDefinitionRef,
        strokes: Vec<(SymbolPrimitive, Option<PrimitiveStyle>)>,
    ) -> PidSymbolDefinition {
        let (primitives, primitive_styles): (Vec<_>, Vec<_>) = strokes.into_iter().unzip();
        PidSymbolDefinition {
            reference,
            layers: vec![8],
            sheet_layers: vec![PidSymbolSheetLayer {
                oid: 8,
                name: Some("Default".to_string()),
                displayed: Some(true),
            }],
            primitive_layers: vec![8; primitives.len()],
            primitives,
            primitive_styles,
            primitive_letterings: Vec::new(),
            dimensions: Vec::new(),
            variables: Vec::new(),
            template: None,
        }
    }

    /// A valve that draws one of everything a body can: two strokes, a
    /// filled disc and a pair of letters. Placed, it is those four entities
    /// and the symbol's name beside them.
    fn valve_body() -> PidSymbolDefinition {
        body(
            BODY,
            vec![
                (
                    SymbolPrimitive::Line {
                        start: (0.0, 0.0),
                        end: (0.004, 0.0),
                    },
                    pen(0.25),
                ),
                (
                    SymbolPrimitive::Line {
                        start: (0.004, 0.0),
                        end: (0.004, 0.003),
                    },
                    pen(0.25),
                ),
                (
                    SymbolPrimitive::Boundary {
                        members: vec![SymbolPrimitive::Circle {
                            center: (0.002, 0.001),
                            radius: 0.001,
                        }],
                        fill: Some(PrimitiveFill {
                            rgb: Some([0xFF, 0x00, 0x00]),
                            type_code: 0x002A,
                        }),
                    },
                    None,
                ),
                (
                    SymbolPrimitive::Text {
                        text: "FO".to_string(),
                        at: (0.001, 0.002),
                    },
                    None,
                ),
            ],
        )
    }

    /// The body of a label symbol: nothing but the hairline cross it is
    /// anchored by, so the placement draws its name and no more.
    fn cross_body() -> PidSymbolDefinition {
        body(
            CROSS,
            vec![
                (
                    SymbolPrimitive::Line {
                        start: (-0.00127, 0.0),
                        end: (0.00127, 0.0),
                    },
                    pen(0.01),
                ),
                (
                    SymbolPrimitive::Line {
                        start: (0.0, 0.00127),
                        end: (0.0, -0.00127),
                    },
                    pen(0.01),
                ),
            ],
        )
    }

    /// Style tables that resolve nothing.
    fn styles() -> Styles {
        Styles {
            style_tables_failed: false,
            styles: LineStyleIndex::new(),
            style_names: StyleNameIndex::new(),
            text_heights: Default::default(),
            lettering_flattened: 0,
            fills: Default::default(),
            dash_linetypes: HashMap::new(),
            font_styles: HashMap::new(),
            semantics: None,
        }
    }

    /// `tables` with record `oid` drawing in a maroon 0.35 mm line style
    /// the drawing calls `name`.
    fn styled(mut tables: Styles, oid: u32, name: &str) -> Styles {
        tables.styles.insert(
            (STREAM.to_string(), oid),
            ResolvedLineStyle {
                style_id: 25,
                symbology: LineSymbology {
                    width_m: 0.000_35,
                    colour: 0x0000_0080,
                    paper: false,
                },
                dash: None,
                marker: None,
                complex_dash: None,
                hop: StyleHop::Direct,
            },
        );
        tables
            .style_names
            .insert((STREAM.to_string(), 25), name.to_string());
        tables
    }

    fn sheet(
        entities: Vec<pid_parse::PidGraphicEntity>,
        bodies: Vec<PidSymbolDefinition>,
    ) -> NormalizedPidGeometry {
        NormalizedPidGeometry {
            entities,
            symbol_definitions: bodies,
            ..Default::default()
        }
    }

    /// Run the entity loop over `geometry` into a document prepared the way
    /// an import prepares one.
    fn import(geometry: &NormalizedPidGeometry, tables: &Styles) -> (Document, Built) {
        let mut doc = prepare_document(&pid_parse::PidDocument::default());
        let built = build_document_entities(
            geometry,
            tables,
            &mut None,
            &ImportUnit::AssumedMetre,
            &mut doc,
            1.0,
        );
        (doc, built)
    }

    /// `geometry` imported by every step an import takes once the file is
    /// parsed, `finish` included: the layers and the view filter are the
    /// ones an open leaves. `file` is the name the import is logged under.
    fn finished(file: &str, geometry: &NormalizedPidGeometry) -> Document {
        let tables = styles();
        let (mut doc, built) = import(geometry, &tables);
        let _ = finish(
            Path::new(file),
            geometry,
            None,
            &tables,
            ImportUnit::AssumedMetre,
            built,
            None,
            &mut doc,
        );
        doc
    }

    /// The sheet the program-level tests of a symbol as one object open
    /// (`crate::app::pid_symbol_block_tests`, SB-3): built by the steps an
    /// import takes once the file is parsed, `finish` included, so its
    /// layers and its view filter are the ones an open leaves. In
    /// millimetres of the drawing:
    ///
    /// - a valve at (100, 100) and the same valve at (170, 100): a stub from
    ///   20 to 10 left of the insertion point, a ring of radius 10 around
    ///   it, a stub from 10 to 20 right of it;
    /// - a label symbol at (170, 140), which draws its name and no more;
    /// - a pipe from (90, 80) to (180, 80), which belongs to no symbol.
    pub(crate) fn sheet_of_two_valves_a_note_and_a_pipe() -> Document {
        let stub = |from: f64, to: f64| SymbolPrimitive::Line {
            start: (from, 0.0),
            end: (to, 0.0),
        };
        let ring = SymbolPrimitive::Circle {
            center: (0.0, 0.0),
            radius: 0.01,
        };
        let valve = body(
            BODY,
            vec![
                (stub(-0.02, -0.01), pen(0.25)),
                (ring, pen(0.25)),
                (stub(0.01, 0.02), pen(0.25)),
            ],
        );
        let on_process = Some((Some("Process"), true));
        let geometry = sheet(
            vec![
                record(
                    701,
                    placement((0.1, 0.1), Some(VALVE), Some(BODY), 0.0),
                    on_process,
                ),
                record(
                    702,
                    placement((0.17, 0.1), Some(VALVE), Some(BODY), 0.0),
                    on_process,
                ),
                record(
                    703,
                    placement((0.17, 0.14), Some(NOTE), Some(CROSS), 0.0),
                    on_process,
                ),
                record(
                    704,
                    PidGraphicKind::Line {
                        start: point(0.09, 0.08),
                        end: point(0.18, 0.08),
                    },
                    on_process,
                ),
            ],
            vec![valve, cross_body()],
        );
        finished("two-valves.pid", &geometry)
    }

    /// The sheet the program-level tests of what is drawn of a symbol open
    /// (`crate::app::pid_symbol_view_tests`, SB-4), built like the one
    /// above. One valve is placed four times, in millimetres of the
    /// drawing:
    ///
    /// - at (100, 100) on `Process` and at (170, 100) on `Instrument`, two
    ///   layers the file displays;
    /// - at (100, 150) on `Demolition`, a layer the file hides;
    /// - at (170, 150) on a hidden layer the file gives no name;
    ///
    /// and a pipe runs from (90, 80) to (180, 80) on `Process`. The valve
    /// is a stub from 20 to 10 left of the insertion point, in no style of
    /// its own; a ring of radius 10 around the point, in black; a stub from
    /// 10 to 20 right of it, in green; and a red disc of radius 5 inside
    /// the ring.
    pub(crate) fn sheet_of_valves_on_shown_and_hidden_layers() -> Document {
        let stub = |from: f64, to: f64| SymbolPrimitive::Line {
            start: (from, 0.0),
            end: (to, 0.0),
        };
        let green = Some(PrimitiveStyle {
            rgb: [0x00, 0x80, 0x00],
            width_mm: 0.25,
            dash_mm: Vec::new(),
            paper: false,
        });
        let valve = body(
            BODY,
            vec![
                (stub(-0.02, -0.01), None),
                (
                    SymbolPrimitive::Circle {
                        center: (0.0, 0.0),
                        radius: 0.01,
                    },
                    pen(0.25),
                ),
                (stub(0.01, 0.02), green),
                (
                    SymbolPrimitive::Boundary {
                        members: vec![SymbolPrimitive::Circle {
                            center: (0.0, 0.0),
                            radius: 0.005,
                        }],
                        fill: Some(PrimitiveFill {
                            rgb: Some([0xFF, 0x00, 0x00]),
                            type_code: 0x002A,
                        }),
                    },
                    None,
                ),
            ],
        );
        let place = |oid: u32, at: (f64, f64), layer: OnLayer| {
            record(oid, placement(at, Some(VALVE), Some(BODY), 0.0), layer)
        };
        let geometry = sheet(
            vec![
                place(801, (0.1, 0.1), Some((Some("Process"), true))),
                place(802, (0.17, 0.1), Some((Some("Instrument"), true))),
                place(803, (0.1, 0.15), Some((Some("Demolition"), false))),
                place(804, (0.17, 0.15), Some((None, false))),
                record(
                    805,
                    PidGraphicKind::Line {
                        start: point(0.09, 0.08),
                        end: point(0.18, 0.08),
                    },
                    Some((Some("Process"), true)),
                ),
            ],
            vec![valve],
        );
        finished("valves-on-layers.pid", &geometry)
    }

    /// How many block records a document holds before anything is imported.
    fn blocks_to_start_with() -> usize {
        prepare_document(&pid_parse::PidDocument::default())
            .block_records
            .len()
    }

    /// The `key=value` strings of an entity's `PID_SEMANTICS` record, in the
    /// order they were written.
    fn says(entity: &EntityType) -> Vec<String> {
        entity
            .common()
            .extended_data
            .get_record(PID_SEMANTICS_XDATA_APP)
            .into_iter()
            .flat_map(|record| record.values.iter())
            .filter_map(|value| match value {
                document::xdata::XDataValue::String(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The block references in model space, in the order they were filed.
    fn references(doc: &Document) -> Vec<&EntityType> {
        doc.model_space_entities()
            .filter(|entity| matches!(entity, EntityType::Insert(_)))
            .collect()
    }

    fn as_insert(entity: &EntityType) -> &Insert {
        match entity {
            EntityType::Insert(insert) => insert,
            other => panic!("not a block reference: {other:?}"),
        }
    }

    /// Everything in model space that is not a block reference.
    fn loose(doc: &Document) -> Vec<&EntityType> {
        doc.model_space_entities()
            .filter(|entity| !matches!(entity, EntityType::Insert(_)))
            .collect()
    }

    fn members<'a>(doc: &'a Document, block: &str) -> Vec<&'a EntityType> {
        doc.entities_in_block(block).collect()
    }

    /// Which entities in model space are dark, in the order they were filed.
    fn dark(doc: &Document) -> Vec<bool> {
        doc.model_space_entities()
            .map(|entity| entity.common().invisible)
            .collect()
    }

    /// Which members of a block are dark, in the order they were filed.
    fn dark_inside(doc: &Document, block: &str) -> Vec<bool> {
        members(doc, block)
            .iter()
            .map(|member| member.common().invisible)
            .collect()
    }

    /// The rows of one axis of the sheet-layer view: name, count, switch.
    fn rows(rows: &[PidViewRow]) -> Vec<(String, usize, bool)> {
        rows.iter()
            .map(|row| (row.name.clone(), row.entities, row.on))
            .collect()
    }

    /// What a placement drew before it was filed as a block: the entities
    /// of `record`, in drawing coordinates, in the placement's paint.
    fn drawn_loose(
        record: &pid_parse::PidGraphicEntity,
        geometry: &NormalizedPidGeometry,
        tables: &Styles,
    ) -> Vec<EntityType> {
        let cached = match &record.kind {
            PidGraphicKind::SymbolInstance {
                definition: Some(definition),
                ..
            } => geometry.symbol_definition(*definition),
            _ => None,
        };
        let symbology = style_for(&tables.styles, record);
        let mut drawn = build_entities(
            &record.kind,
            BodySources {
                cached,
                library: None,
            },
            Projection::for_geometry(geometry, &ImportUnit::AssumedMetre),
            symbology,
            LAYER_GEOMETRY,
            &mut SymbolBodies::default(),
            &tables.dash_linetypes,
            1.0,
            &tables.font_styles,
        );
        if let Some(style) = symbology {
            for entity in &mut drawn {
                apply_symbology(entity, style, &tables.dash_linetypes, 1.0);
            }
        }
        drawn
    }

    /// An entity as its `Debug` text, without what filing it adds: the
    /// handle, the owner and the `PID_SEMANTICS` record.
    fn portrait(entity: &EntityType) -> String {
        let mut bare = entity.clone();
        let common = bare.common_mut();
        common.handle = document::Handle::NULL;
        common.owner_handle = document::Handle::NULL;
        let _ = common.extended_data.remove_record(PID_SEMANTICS_XDATA_APP);
        format!("{bare:?}")
    }

    /// The portrait of `drawn`, an entity in drawing coordinates, as the
    /// member on `layer` of a block whose base point is `origin`: moved
    /// there and saying no more than it said -- the pattern origin a move
    /// writes on a hatch is not something the placement drew.
    fn as_member(drawn: &EntityType, origin: Vector3, layer: &str) -> String {
        let mut member = drawn.clone();
        member.as_entity_mut().translate(-origin);
        let common = member.common_mut();
        common.extended_data = drawn.common().extended_data.clone();
        common.layer = layer.to_string();
        portrait(&member)
    }

    fn assert_at(actual: Vector3, expected: (f64, f64), what: &str) {
        assert!(
            (actual.x - expected.0).abs() < 1e-9
                && (actual.y - expected.1).abs() < 1e-9
                && actual.z == 0.0,
            "{what}: {actual:?} is not {expected:?}"
        );
    }

    /// One placement record becomes one block reference in model space and
    /// one definition in the document, and nothing it drew is left loose --
    /// not a stroke, not the fill, not the name.
    #[test]
    fn a_placement_is_one_reference_to_one_definition_and_leaves_nothing_loose() {
        let geometry = sheet(
            vec![record(
                612,
                placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0),
                Some((Some("Process"), true)),
            )],
            vec![valve_body()],
        );
        let (doc, built) = import(&geometry, &styles());

        let placed = references(&doc);
        assert_eq!(placed.len(), 1, "one placement, one block reference");
        assert_eq!(as_insert(placed[0]).block_name, "SPPID_Ball Valve");
        assert_eq!(
            doc.block_records.len(),
            blocks_to_start_with() + 1,
            "and one definition"
        );
        assert!(
            loose(&doc).is_empty(),
            "nothing the placement drew is left loose: {:?}",
            loose(&doc)
                .iter()
                .map(|entity| says(entity))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            members(&doc, "SPPID_Ball Valve").len(),
            5,
            "two strokes, the disc, the letters and the name are in the block"
        );
        assert_eq!((built.symbol_blocks, built.symbol_definitions), (1, 1));
    }

    /// The reference stands at the placement's insertion point, unrotated
    /// and unscaled, and carries the record the placement's strokes used to
    /// carry -- sheet layer, role, style, measures -- plus the symbol's
    /// name as the drawing spells it, where the block's name can only hold
    /// what a table name allows.
    #[test]
    fn the_reference_stands_at_the_insertion_point_and_says_what_the_placement_said() {
        let path = r#"\\SRV\Ref\Symbols\Piping\Valves\Valve <2"> a:b.sym"#;
        let geometry = sheet(
            vec![record(
                612,
                placement((0.1, 0.2), Some(path), Some(BODY), 0.0),
                Some((Some("Process"), true)),
            )],
            vec![valve_body()],
        );
        let tables = styled(styles(), 612, "Equipment - New");
        let (doc, _) = import(&geometry, &tables);

        let placed = references(&doc);
        assert_eq!(placed.len(), 1);
        let insert = as_insert(placed[0]);
        assert_eq!(
            insert.block_name, "SPPID_Valve _2__ a_b",
            "the block is named after the symbol, less what a block name cannot hold"
        );
        assert_at(
            insert.insert_point,
            (100.0, 200.0),
            "the insertion point, in millimetres",
        );
        assert_eq!(insert.rotation, 0.0);
        assert_eq!(
            (insert.x_scale(), insert.y_scale(), insert.z_scale()),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(
            says(placed[0]),
            [
                "sheet_layer=Process",
                "sheet_layer_oid=12",
                "role=symbol",
                "style=Equipment - New",
                "extent=4.00x3.00",
                r#"symbol=Valve <2"> a:b"#,
            ],
            "the placement's record, and the symbol's name as the drawing spells it"
        );
    }

    /// What `_Data.xml` publishes about a placement is the reference's to
    /// state: the members say what they are and nothing else.
    #[test]
    fn the_published_identity_is_on_the_reference_and_not_in_the_block() {
        let xml = r#"<Container>
  <PIDInlineComponent><IObject UID="VALVE" ItemTag="GV-101"/></PIDInlineComponent>
  <PIDRepresentation><IObject UID="REP"/><IDrawingRepresentation GraphicOID="612"/></PIDRepresentation>
  <Rel><IRel UID1="VALVE" UID2="REP" DefUID="DwgRepresentationComposition"/></Rel>
</Container>"#;
        let mut tables = styles();
        tables.semantics = Some(PidSemanticIndex::from_xml(
            xml,
            &pid_parse::PidDocument::default(),
        ));
        let geometry = sheet(
            vec![record(
                612,
                placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0),
                Some((Some("Process"), true)),
            )],
            vec![valve_body()],
        );
        let (doc, _) = import(&geometry, &tables);

        let placed = references(&doc);
        assert_eq!(placed.len(), 1);
        assert_eq!(
            says(placed[0]),
            [
                "sheet_layer=Process",
                "sheet_layer_oid=12",
                "role=symbol",
                "extent=4.00x3.00",
                "class=PIDInlineComponent",
                "label=GV-101",
                "oid=612",
                "resolved=direct",
                "symbol=Ball Valve",
            ]
        );
        let inside: Vec<Vec<String>> = members(&doc, "SPPID_Ball Valve")
            .iter()
            .map(|member| says(member))
            .collect();
        assert_eq!(
            inside,
            [
                vec!["role=symbol"],
                vec!["role=symbol"],
                vec!["role=fill"],
                vec!["role=symbol"],
                vec!["role=symbol-label"],
            ],
            "a member says what it is and nothing else"
        );
    }

    /// The members are the entities the placement drew, moved so that the
    /// insertion point is the block's base point: the same shapes in the
    /// same paint, the name among them, each saying its role alone.
    #[test]
    fn the_members_are_what_the_placement_drew_about_its_insertion_point() {
        let geometry = sheet(
            vec![record(
                612,
                placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0),
                Some((Some("Process"), true)),
            )],
            vec![valve_body()],
        );
        let tables = styled(styles(), 612, "Equipment - New");
        let (doc, _) = import(&geometry, &tables);

        let inside = members(&doc, "SPPID_Ball Valve");
        let drew = drawn_loose(&geometry.entities[0], &geometry, &tables);
        assert_eq!(
            inside.len(),
            drew.len(),
            "every entity the placement draws is in the block"
        );
        let origin = Vector3::new(100.0, 200.0, 0.0);
        let layers = ["0", "0", "0", "0", LAYER_SYMBOL_LABEL];
        let roles = [
            "role=symbol",
            "role=symbol",
            "role=fill",
            "role=symbol",
            "role=symbol-label",
        ];
        for (index, (member, before)) in inside.iter().zip(&drew).enumerate() {
            assert_eq!(
                says(member),
                [roles[index]],
                "member {index} says its role and nothing else"
            );
            assert_eq!(
                portrait(member),
                as_member(before, origin, layers[index]),
                "member {index} is what the placement drew, about the insertion point"
            );
        }

        // The same, by hand, for one member of each kind.
        let maroon = Color::from_rgb(0x80, 0x00, 0x00);
        match inside[0] {
            EntityType::Line(stroke) => {
                assert_at(
                    stroke.start,
                    (0.0, 0.0),
                    "the first stroke leaves the base point",
                );
                assert_at(stroke.end, (4.0, 0.0), "and runs its four millimetres");
                assert_eq!(
                    (stroke.common.color, stroke.common.line_weight),
                    (maroon, LineWeight::Value(34)),
                    "in the placement's colour and pen"
                );
            }
            other => panic!("the first member is the first stroke: {other:?}"),
        }
        match inside[2] {
            EntityType::Hatch(disc) => {
                match disc.paths.first().and_then(|path| path.edges.first()) {
                    Some(BoundaryEdge::CircularArc(edge)) => assert!(
                        (edge.center.x - 2.0).abs() < 1e-9
                            && (edge.center.y - 1.0).abs() < 1e-9
                            && (edge.radius - 1.0).abs() < 1e-9,
                        "the disc, about the base point: {edge:?}"
                    ),
                    other => panic!("the fill is a disc: {other:?}"),
                }
                assert_eq!(
                    disc.common.color,
                    Color::from_rgb(0xFF, 0x00, 0x00),
                    "the fill keeps the colour its own style states"
                );
                assert_eq!(
                    disc.stored_pattern_origin(),
                    None,
                    "a solid fill states no pattern origin, wherever it stands"
                );
            }
            other => panic!("the third member is the fill: {other:?}"),
        }
        match inside[3] {
            EntityType::Text(letters) => {
                assert_eq!(letters.value, "FO");
                assert_at(letters.insertion_point, (1.0, 2.0), "the body's letters");
                assert_eq!(letters.common.color, maroon, "in the placement's colour");
            }
            other => panic!("the fourth member is the body's lettering: {other:?}"),
        }
        match inside[4] {
            EntityType::Text(name) => {
                assert_eq!(name.value, "Ball Valve");
                assert_at(
                    name.insertion_point,
                    (4.8, 0.5),
                    "the name, beside the body",
                );
                assert_eq!(name.height, SYMBOL_LABEL_HEIGHT_MM);
                assert_eq!(name.common.layer, LAYER_SYMBOL_LABEL);
            }
            other => panic!("the last member is the symbol's name: {other:?}"),
        }
    }

    /// Import one placement filed on `layer` -- the valve, or the label
    /// that draws only its name -- and check which layer the reference is
    /// on, whether that layer is on, and which layers the members are on.
    fn check_layers(
        what: &str,
        label_only: bool,
        layer: OnLayer,
        reference_layer: (&str, bool),
        member_layers: &[&str],
    ) -> Built {
        let (path, definition) = if label_only {
            (NOTE, CROSS)
        } else {
            (VALVE, BODY)
        };
        let geometry = sheet(
            vec![record(
                612,
                placement((0.1, 0.2), Some(path), Some(definition), 0.0),
                layer,
            )],
            vec![valve_body(), cross_body()],
        );
        let (doc, built) = import(&geometry, &styles());

        let placed = references(&doc);
        assert_eq!(placed.len(), 1, "{what}: one reference");
        let (on_layer, layer_on) = reference_layer;
        assert_eq!(
            placed[0].common().layer,
            on_layer,
            "{what}: the reference's layer"
        );
        assert!(
            says(placed[0]).iter().any(|pair| pair == "role=symbol"),
            "{what}: a reference is a symbol whatever its block draws: {:?}",
            says(placed[0])
        );
        let declared = doc
            .layers
            .get(on_layer)
            .unwrap_or_else(|| panic!("{what}: {on_layer} is declared"));
        assert_eq!(
            !declared.flags.off, layer_on,
            "{what}: whether {on_layer} is on"
        );
        let inside: Vec<&str> = members(&doc, &as_insert(placed[0]).block_name)
            .iter()
            .map(|member| member.common().layer.as_str())
            .collect();
        assert_eq!(inside, member_layers, "{what}: the members' layers");
        assert!(loose(&doc).is_empty(), "{what}: nothing is left loose");
        built
    }

    /// The reference is filed where the placement's strokes used to be: on
    /// the sheet layer the drawing names, on `PID-SYMBOL` when it names
    /// none, on `PID-HIDDEN` when it hides one it does not name. A member
    /// that would have been on that same layer floats on layer 0, so it
    /// shows, hides and takes its colour with the reference; a member that
    /// kept a layer of the importer's own -- the name, and a fill when the
    /// drawing names no layer -- keeps it inside the block. A placement
    /// that draws nothing but its name is filed with the names.
    #[test]
    fn the_reference_takes_the_placements_layer_and_what_shared_it_floats_on_layer_0() {
        let named = ["0", "0", "0", "0", LAYER_SYMBOL_LABEL];
        let shown = check_layers(
            "a named layer the file displays",
            false,
            Some((Some("Process"), true)),
            ("Process", true),
            &named,
        );
        assert!(shown.sheet_layers_off.is_empty());
        let hidden = check_layers(
            "a named layer the file hides",
            false,
            Some((Some("Process"), false)),
            ("Process", false),
            &named,
        );
        assert_eq!(
            hidden.sheet_layers_off,
            BTreeSet::from(["Process".to_string()]),
            "the view filter still starts with the layer off"
        );
        let unnamed = ["0", "0", LAYER_FILL, "0", LAYER_SYMBOL_LABEL];
        check_layers(
            "a displayed layer with no name",
            false,
            Some((None, true)),
            (LAYER_SYMBOL, true),
            &unnamed,
        );
        check_layers(
            "no layer at all",
            false,
            None,
            (LAYER_SYMBOL, true),
            &unnamed,
        );
        check_layers(
            "a hidden layer with no name",
            false,
            Some((None, false)),
            (LAYER_HIDDEN, false),
            &["0"; 5],
        );

        let name_alone = ["0"];
        check_layers(
            "a label on a named layer the file displays",
            true,
            Some((Some("Labels"), true)),
            (LAYER_SYMBOL_LABEL, false),
            &name_alone,
        );
        check_layers(
            "a label on a named layer the file hides",
            true,
            Some((Some("Labels"), false)),
            (LAYER_SYMBOL_LABEL, false),
            &name_alone,
        );
        check_layers(
            "a label on a displayed layer with no name",
            true,
            Some((None, true)),
            (LAYER_SYMBOL_LABEL, false),
            &name_alone,
        );
        check_layers(
            "a label on no layer at all",
            true,
            None,
            (LAYER_SYMBOL_LABEL, false),
            &name_alone,
        );
        check_layers(
            "a label on a hidden layer with no name",
            true,
            Some((None, false)),
            (LAYER_HIDDEN, false),
            &name_alone,
        );
    }

    /// Two placements that draw the same thing in different places refer to
    /// one definition. A placement turned a quarter draws something else,
    /// so it gets a definition of its own, numbered after the first -- and
    /// the next one turned the same way shares that. The turn is in the
    /// definition; every reference is unrotated.
    #[test]
    fn placements_that_draw_the_same_share_a_definition_and_a_turned_one_gets_its_own() {
        let on = Some((Some("Process"), true));
        let quarter = std::f64::consts::FRAC_PI_2;
        let geometry = sheet(
            vec![
                record(601, placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0), on),
                record(602, placement((0.3, 0.1), Some(VALVE), Some(BODY), 0.0), on),
                record(
                    603,
                    placement((0.2, 0.3), Some(VALVE), Some(BODY), quarter),
                    on,
                ),
                record(
                    604,
                    placement((0.4, 0.3), Some(VALVE), Some(BODY), quarter),
                    on,
                ),
            ],
            vec![valve_body()],
        );
        let (doc, built) = import(&geometry, &styles());

        let placed = references(&doc);
        let names: Vec<&str> = placed
            .iter()
            .map(|reference| as_insert(reference).block_name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "SPPID_Ball Valve",
                "SPPID_Ball Valve",
                "SPPID_Ball Valve_2",
                "SPPID_Ball Valve_2",
            ]
        );
        assert_eq!(doc.block_records.len(), blocks_to_start_with() + 2);
        assert_eq!((built.symbol_blocks, built.symbol_definitions), (4, 2));
        let stands = [
            (100.0, 200.0),
            (300.0, 100.0),
            (200.0, 300.0),
            (400.0, 300.0),
        ];
        for (reference, at) in placed.iter().zip(stands) {
            let insert = as_insert(reference);
            assert_at(
                insert.insert_point,
                at,
                "each reference at its own placement",
            );
            assert_eq!(
                insert.rotation, 0.0,
                "the turn is drawn in the definition, not put on the reference"
            );
        }

        let (again, _) = import(&geometry, &styles());
        let names_again: Vec<String> = references(&again)
            .iter()
            .map(|reference| as_insert(reference).block_name.clone())
            .collect();
        assert_eq!(
            names_again, names,
            "the same drawing imports to the same names"
        );
    }

    /// A placement neither body reaches is drawn as its marker dot, and the
    /// dot is a block like any body: the dot on the base point and the name
    /// beside it. One that names no symbol has no name to state or letter,
    /// and all of those share the one definition.
    #[test]
    fn a_placement_with_no_body_is_a_block_of_its_marker_dot() {
        let geometry = sheet(
            vec![
                record(701, placement((0.1, 0.2), Some(VALVE), None, 0.0), None),
                record(702, placement((0.3, 0.2), None, None, 0.0), None),
                record(703, placement((0.5, 0.2), None, None, 0.0), None),
            ],
            Vec::new(),
        );
        let (doc, built) = import(&geometry, &styles());

        let placed = references(&doc);
        let names: Vec<&str> = placed
            .iter()
            .map(|reference| as_insert(reference).block_name.as_str())
            .collect();
        assert_eq!(names, ["SPPID_Ball Valve", "SPPID_SYMBOL", "SPPID_SYMBOL"]);
        assert!(placed
            .iter()
            .all(|reference| reference.common().layer == LAYER_SYMBOL));
        assert_eq!(
            says(placed[0]),
            ["role=symbol", "extent=3.00x3.00", "symbol=Ball Valve"]
        );
        assert_eq!(
            says(placed[1]),
            ["role=symbol", "extent=3.00x3.00"],
            "a placement that names no symbol has no name to state"
        );

        let named = members(&doc, "SPPID_Ball Valve");
        assert_eq!(named.len(), 2, "the dot and the name");
        match named[0] {
            EntityType::Circle(dot) => {
                assert_at(dot.center, (0.0, 0.0), "the dot sits on the base point");
                assert_eq!(dot.radius, SYMBOL_MARKER_RADIUS_MM);
                assert_eq!(dot.common.layer, "0");
            }
            other => panic!("the first member is the dot: {other:?}"),
        }
        match named[1] {
            EntityType::Text(name) => {
                assert_eq!(name.value, "Ball Valve");
                assert_at(
                    name.insertion_point,
                    (
                        SYMBOL_MARKER_RADIUS_MM + SYMBOL_LABEL_GAP_MM,
                        -SYMBOL_LABEL_HEIGHT_MM / 2.0,
                    ),
                    "the name, beside the dot",
                );
                assert_eq!(name.common.layer, LAYER_SYMBOL_LABEL);
            }
            other => panic!("the second member is the name: {other:?}"),
        }
        assert_eq!(
            members(&doc, "SPPID_SYMBOL").len(),
            1,
            "a nameless placement is the dot alone"
        );
        assert!(loose(&doc).is_empty());
        assert_eq!(built.symbol_bodies.markers, 3);
        assert_eq!((built.symbol_blocks, built.symbol_definitions), (3, 2));
    }

    /// A label placement whose body is only its anchor cross draws its name
    /// and nothing else, and that name is its block. The reference is filed
    /// with the names, on the layer that opens switched off, and still says
    /// which sheet layer the placement is on and that it is a symbol.
    #[test]
    fn a_label_that_draws_only_its_name_is_a_block_of_that_name() {
        let geometry = sheet(
            vec![record(
                801,
                placement((0.1, 0.2), Some(NOTE), Some(CROSS), 0.0),
                Some((Some("Labels"), true)),
            )],
            vec![cross_body()],
        );
        let (doc, built) = import(&geometry, &styles());

        let placed = references(&doc);
        assert_eq!(placed.len(), 1, "the label is a block reference");
        assert_eq!(as_insert(placed[0]).block_name, "SPPID_Item Note");
        assert_eq!(placed[0].common().layer, LAYER_SYMBOL_LABEL);
        assert!(
            doc.layers
                .get(LAYER_SYMBOL_LABEL)
                .is_some_and(|layer| layer.flags.off),
            "and the names' layer opens switched off"
        );
        assert_eq!(
            says(placed[0]),
            [
                "sheet_layer=Labels",
                "sheet_layer_oid=12",
                "role=symbol",
                "extent=2.54x2.54",
                "symbol=Item Note",
            ]
        );
        let inside = members(&doc, "SPPID_Item Note");
        assert_eq!(inside.len(), 1, "the name alone");
        match inside[0] {
            EntityType::Text(name) => {
                assert_eq!(name.value, "Item Note");
                assert_at(
                    name.insertion_point,
                    (2.07, -1.0),
                    "beside where the cross would have been",
                );
                assert_eq!(name.common.layer, "0");
            }
            other => panic!("the member is the name: {other:?}"),
        }
        assert_eq!(says(inside[0]), ["role=symbol-label"]);
        assert!(loose(&doc).is_empty());
        assert_eq!((built.symbol_bodies.label_anchors, built.drawn), (1, 1));
    }

    /// A record that is not a placement -- a line, a run of text, a filled
    /// ring, a point -- is the loose entity it always was, on the layer it
    /// was on, carrying its whole record.
    #[test]
    fn a_record_that_is_no_placement_stays_loose_with_its_whole_record() {
        let on = Some((Some("Process"), true));
        let geometry = sheet(
            vec![
                record(
                    901,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.0),
                        end: point(0.05, 0.0),
                    },
                    on,
                ),
                record(
                    902,
                    PidGraphicKind::Text {
                        insertion: point(0.01, 0.03),
                        value: "V-101".to_string(),
                        height: 0.0025,
                        rotation: 0.0,
                        anchor_code: 0,
                        text_box_width: None,
                    },
                    on,
                ),
                record(
                    903,
                    PidGraphicKind::Polyline {
                        points: vec![point(0.01, 0.01), point(0.02, 0.01), point(0.02, 0.02)],
                        closed: true,
                    },
                    on,
                ),
                record(
                    904,
                    PidGraphicKind::Point {
                        position: point(0.03, 0.03),
                    },
                    None,
                ),
                record(905, placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0), on),
            ],
            vec![valve_body()],
        );
        let mut tables = styled(styles(), 901, "Primary Piping - New");
        tables.fills.insert(
            (STREAM.to_string(), 903),
            ResolvedFill {
                style_id: 9,
                type_code: 0x002A,
                colour: Some(0x00FF_0000),
                hop: StyleHop::Direct,
            },
        );
        let (doc, _) = import(&geometry, &tables);

        assert_eq!(references(&doc).len(), 1, "the placement alone is a block");
        let stays = loose(&doc);
        let kinds: Vec<&str> = stays
            .iter()
            .map(|entity| match entity {
                EntityType::Line(_) => "line",
                EntityType::Text(_) => "text",
                EntityType::Hatch(_) => "fill",
                EntityType::Point(_) => "point",
                _ => "something else",
            })
            .collect();
        assert_eq!(kinds, ["line", "text", "fill", "point"]);
        let filed: Vec<(&str, Vec<String>)> = stays
            .iter()
            .map(|entity| (entity.common().layer.as_str(), says(entity)))
            .collect();
        let on_process = |role: &str| {
            vec![
                "sheet_layer=Process".to_string(),
                "sheet_layer_oid=12".to_string(),
                format!("role={role}"),
            ]
        };
        let mut line_says = on_process("geometry");
        line_says.push("style=Primary Piping - New".to_string());
        assert_eq!(
            filed,
            [
                ("Process", line_says),
                ("Process", on_process("text")),
                ("Process", on_process("fill")),
                (LAYER_POINT, vec!["role=point-ok".to_string()]),
            ]
        );
    }

    /// What the import tallies does not change: a placement's entities
    /// still count as drawn and towards their sheet layer, and the camera
    /// is still framed on what they cover -- the name included.
    #[test]
    fn the_tallies_count_what_the_placements_draw_as_before() {
        let on = Some((Some("Process"), true));
        let geometry = sheet(
            vec![
                record(
                    1001,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.0),
                        end: point(0.05, 0.0),
                    },
                    on,
                ),
                record(
                    1002,
                    placement((0.1, 0.2), Some(VALVE), Some(BODY), 0.0),
                    on,
                ),
                record(
                    1003,
                    placement((0.3, 0.1), Some(VALVE), Some(BODY), 0.0),
                    on,
                ),
            ],
            vec![valve_body()],
        );
        let (doc, built) = import(&geometry, &styles());

        assert_eq!(
            built.drawn, 11,
            "a line and two placements of five entities each"
        );
        assert_eq!(built.decoded, 3);
        assert_eq!(
            built.sheet_layer_distribution,
            BTreeMap::from([(("/".to_string(), 12, Some("Process".to_string())), 11)])
        );
        assert_eq!(built.symbol_bodies.cache, 2);
        let framed = [
            (built.bounds.min_x, 0.0),
            (built.bounds.min_y, 0.0),
            (built.bounds.max_x, 304.8),
            (built.bounds.max_y, 203.0),
        ];
        assert!(
            framed
                .iter()
                .all(|(actual, expected)| (actual - expected).abs() < 1e-9),
            "the extent runs from the line's start to the second name and the first body's top: {framed:?}"
        );
        assert_eq!(
            doc.model_space_entities().count(),
            3,
            "the line and two references"
        );
        assert_eq!((built.symbol_blocks, built.symbol_definitions), (2, 1));
    }

    /// The blocks the valve and the label symbol are filed as.
    const VALVE_BLOCK: &str = "SPPID_Ball Valve";
    const NOTE_BLOCK: &str = "SPPID_Item Note";

    /// A placement of the valve at (`x`, 0.2) metres, filed on `layer`.
    fn valve_at(oid: u32, x: f64, layer: OnLayer) -> pid_parse::PidGraphicEntity {
        record(
            oid,
            placement((x, 0.2), Some(VALVE), Some(BODY), 0.0),
            layer,
        )
    }

    /// A sheet whose layers the file displays: the valve twice on `Process`
    /// and once on `Instrument`, a label symbol on `Process` and a pipe on
    /// `Process`. In model space that is four references and the pipe, in
    /// that order. The three valves draw one definition -- two strokes, the
    /// disc, the letters, the name -- and the label one of its own, which
    /// holds its name.
    fn valves_a_note_and_a_pipe() -> Document {
        let process = Some((Some("Process"), true));
        let geometry = sheet(
            vec![
                valve_at(1101, 0.1, process),
                valve_at(1102, 0.3, process),
                valve_at(1103, 0.5, Some((Some("Instrument"), true))),
                record(
                    1104,
                    placement((0.3, 0.3), Some(NOTE), Some(CROSS), 0.0),
                    process,
                ),
                record(
                    1105,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.0),
                        end: point(0.05, 0.0),
                    },
                    process,
                ),
            ],
            vec![valve_body(), cross_body()],
        );
        let (doc, _) = import(&geometry, &styles());
        doc
    }

    /// A sheet layer switched off darkens the references the drawing files
    /// on it, and the pipe, and nothing inside a block: the valve on the
    /// other layer draws the same definition. Switched on again, they are
    /// back.
    #[test]
    fn a_sheet_layer_switched_off_darkens_its_references_and_no_member() {
        let mut doc = valves_a_note_and_a_pipe();
        assert_eq!(dark(&doc), [false; 5]);

        let switched = switch_sheet_layer(&mut doc, "Process", false);
        assert_eq!(
            switched,
            Switched {
                entities: 4,
                released_layer: None
            },
            "two valves, the label and the pipe"
        );
        assert_eq!(
            dark(&doc),
            [true, true, false, true, true],
            "the valve on Instrument stays lit"
        );
        assert_eq!(dark_inside(&doc, VALVE_BLOCK), [false; 5]);
        assert_eq!(dark_inside(&doc, NOTE_BLOCK), [false]);

        let switched = switch_sheet_layer(&mut doc, "Process", true);
        assert_eq!(
            switched,
            Switched {
                entities: 4,
                released_layer: None
            }
        );
        assert_eq!(dark(&doc), [false; 5]);
    }

    /// A role switched off darkens what states it, inside the blocks too:
    /// `symbol` every reference and the strokes and letters of the bodies,
    /// `fill` the disc and nothing else, `symbol-label` the names and
    /// nothing else.
    #[test]
    fn a_role_switched_off_darkens_the_members_that_state_it() {
        let mut doc = valves_a_note_and_a_pipe();

        let switched = switch_role(&mut doc, "symbol", false);
        assert_eq!(
            switched.entities, 7,
            "four references, two strokes and the letters"
        );
        assert_eq!(
            dark(&doc),
            [true, true, true, true, false],
            "the pipe is no symbol"
        );
        assert_eq!(
            dark_inside(&doc, VALVE_BLOCK),
            [true, true, false, true, false],
            "the disc and the name are not"
        );
        assert_eq!(dark_inside(&doc, NOTE_BLOCK), [false]);
        assert_eq!(switch_role(&mut doc, "symbol", true).entities, 7);

        let switched = switch_role(&mut doc, "fill", false);
        assert_eq!(switched.entities, 1);
        assert_eq!(dark(&doc), [false; 5]);
        assert_eq!(
            dark_inside(&doc, VALVE_BLOCK),
            [false, false, true, false, false]
        );
        assert_eq!(dark_inside(&doc, NOTE_BLOCK), [false]);
        assert_eq!(switch_role(&mut doc, "fill", true).entities, 1);

        let switched = switch_role(&mut doc, "symbol-label", false);
        assert_eq!(switched.entities, 2, "the valve's name and the label's");
        assert_eq!(dark(&doc), [false; 5]);
        assert_eq!(
            dark_inside(&doc, VALVE_BLOCK),
            [false, false, false, false, true]
        );
        assert_eq!(dark_inside(&doc, NOTE_BLOCK), [true]);
    }

    /// The sheet-layer view counts a symbol once on its sheet layer, by its
    /// reference, whatever its block draws. Along the roles it counts the
    /// reference as a symbol and each member of a definition under what it
    /// is, once however many references draw it -- so the rows of the
    /// symbols, of their names and of their fills are all there.
    #[test]
    fn the_sheet_layer_view_counts_a_symbol_once_and_its_members_by_role() {
        let summary = PidViewSummary::of(&valves_a_note_and_a_pipe());
        assert_eq!(
            rows(&summary.layers),
            [
                ("Instrument".to_string(), 1, true),
                ("Process".to_string(), 4, true),
            ],
            "a valve; two valves, the label and the pipe"
        );
        assert_eq!(
            rows(&summary.roles),
            [
                ("geometry".to_string(), 1, true),
                ("symbol".to_string(), 7, true),
                ("symbol-label".to_string(), 2, true),
                ("fill".to_string(), 1, true),
            ],
            "symbol: four references and, in the valve's definition, two strokes and the letters"
        );
    }

    /// A sheet layer the file hides starts switched off: the reference on
    /// it is dark, on a layer of the table that is off. Switching the sheet
    /// layer on lights the reference and turns that layer on with it, as it
    /// does for a loose entity, and switching it off again leaves the table
    /// alone. A placement on a hidden layer the file gives no name is under
    /// `PID-HIDDEN` and states no sheet layer: no row of the view is its
    /// switch.
    #[test]
    fn a_hidden_sheet_layer_switched_on_lights_its_reference_and_turns_its_layer_on() {
        let geometry = sheet(
            vec![
                valve_at(1201, 0.1, Some((Some("Demolition"), false))),
                valve_at(1202, 0.3, Some((Some("Process"), true))),
                valve_at(1203, 0.5, Some((None, false))),
            ],
            vec![valve_body()],
        );
        let mut doc = finished("hidden-layers.pid", &geometry);
        let filed: Vec<String> = references(&doc)
            .iter()
            .map(|reference| reference.common().layer.clone())
            .collect();
        assert_eq!(filed, ["Demolition", "Process", LAYER_HIDDEN]);
        let lit = |doc: &Document| -> Vec<bool> {
            references(doc)
                .iter()
                .map(|reference| !reference.common().invisible)
                .collect()
        };
        let is_on = |doc: &Document, layer: &str| !doc.layers.get(layer).unwrap().flags.off;
        assert_eq!(lit(&doc), [false, true, true]);
        assert!(!is_on(&doc, "Demolition"));
        assert!(!is_on(&doc, LAYER_HIDDEN));
        assert_eq!(
            rows(&PidViewSummary::of(&doc).layers),
            [
                ("Demolition".to_string(), 1, false),
                ("Process".to_string(), 1, true),
            ]
        );

        let switched = switch_sheet_layer(&mut doc, "Demolition", true);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_layer: Some("Demolition".to_string())
            }
        );
        assert_eq!(lit(&doc), [true; 3]);
        assert!(is_on(&doc, "Demolition"));
        assert!(
            !is_on(&doc, LAYER_HIDDEN),
            "PID-HIDDEN holds nothing of Demolition"
        );

        let switched = switch_sheet_layer(&mut doc, "Demolition", false);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_layer: None
            }
        );
        assert_eq!(lit(&doc), [false, true, true]);
        assert!(
            is_on(&doc, "Demolition"),
            "switching off leaves the layer table alone"
        );
    }
}

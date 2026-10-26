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
    let mut arc = codec::entities::Arc::new();
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
    let mut arc = codec::entities::Arc::new();
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
    };
    let midpoint = |arc: &codec::entities::Arc| {
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
        rgb: Some([0, 0, 0]),
        alignment: Some(TextAlignment::Left),
        line_spacing: Some(1.0),
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
        rgb: Some([0, 0, 0]),
        alignment: Some(TextAlignment::Left),
        line_spacing: Some(1.0),
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

/// The overlay takes the template only when its page is the drawing's: a
/// stated pair must agree within the tolerance; a side that states nothing
/// cannot refuse the match. A template stating no page of its own is
/// measured on the frame it drew.
#[test]
fn a_template_is_overlaid_only_on_the_page_it_is_for() {
    let geometry = |page: Option<(f64, f64)>| NormalizedPidGeometry {
        page_dimensions_mm: page,
        ..Default::default()
    };
    let a1_frame: Vec<EntityType> = vec![line((0.0, 0.0), (841.0, 594.0))];
    let a0_frame: Vec<EntityType> = vec![line((0.0, 0.0), (1189.0, 841.0))];
    assert!(same_page(
        &geometry(Some((841.0, 594.0))),
        &[],
        &geometry(Some((841.0, 594.0)))
    ));
    assert!(
        same_page(
            &geometry(Some((841.2, 594.1))),
            &[],
            &geometry(Some((841.0, 594.0))),
        ),
        "float noise between the two frame records is the same page"
    );
    assert!(
        !same_page(
            &geometry(Some((1189.0, 841.0))),
            &[],
            &geometry(Some((841.0, 594.0))),
        ),
        "an A0 template never lands on an A1 drawing"
    );
    assert!(same_page(
        &geometry(None),
        &[],
        &geometry(Some((841.0, 594.0)))
    ));
    assert!(same_page(
        &geometry(Some((841.0, 594.0))),
        &[],
        &geometry(None)
    ));
    // The igr states no page of its own, so its drawn frame is the claim:
    // an A1-sized overlay passes on A1, an A0-sized one is refused.
    assert!(
        same_page(&geometry(None), &a1_frame, &geometry(Some((841.0, 594.0)))),
        "a page-less template is measured by the frame it drew"
    );
    assert!(
        !same_page(&geometry(None), &a0_frame, &geometry(Some((841.0, 594.0)))),
        "the page-less A0 template may not land on the A1 drawing"
    );
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
    // the template's must agree with it.
    let drawing = NormalizedPidGeometry {
        page_dimensions_mm: Some((841.0, 594.0)),
        ..Default::default()
    };
    let overlay = overlay_from(&igr, &drawing, &mut doc, 1.0).expect("the fixture draws");
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
                    matches!(value, codec::xdata::XDataValue::String(s) if s == "template=CPE-SQ-A1.igr")
                })
        })
        .count();
    assert_eq!(
        tagged, overlay.drawn,
        "every template entity names the .igr it came from"
    );
}

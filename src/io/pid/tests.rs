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
    summary.symbol_blocks = 27;
    summary.symbol_definitions = 19;
    summary.symbol_items = 25;
    summary.symbol_tags = 11;
    summary.symbol_labels = 30;
    summary.other_labels = 9;
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

/// The four things that can stand between a drawing and the border template
/// it links, each with the file names the links end in.
fn templates_not_drawn() -> [TemplateNotDrawn; 4] {
    let of = |why: TemplateGap| TemplateNotDrawn {
        named: "CPE-SQ~2.IGR, CPE-SQ~3.PID".to_string(),
        why,
    };
    [
        of(TemplateGap::NoFile),
        of(TemplateGap::Undrawable {
            files: "CPE-SQ-A1.igr, CPE-SQ-A0.igr".to_string(),
        }),
        of(TemplateGap::NoneFits {
            page: "841.00 x 594.00 mm".to_string(),
            found: "CPE-SQ-A0.igr (1189.00 x 841.00 mm), CPE-SQ-A2.igr (no strokes)".to_string(),
        }),
        of(TemplateGap::NoPage),
    ]
}

/// The values the line for `not_drawn` has to name in whatever language.
fn values_said_of(not_drawn: &TemplateNotDrawn) -> Vec<&str> {
    let mut values = vec![not_drawn.named.as_str()];
    match &not_drawn.why {
        TemplateGap::NoFile => values.push("PID_TEMPLATE_LIBRARY"),
        TemplateGap::Undrawable { files } => values.push(files),
        TemplateGap::NoneFits { page, found } => values.extend([page.as_str(), found.as_str()]),
        TemplateGap::NoPage => {}
    }
    values
}

/// Why a linked border template was not drawn rides the document like the
/// counts do: each of the four reasons reads back as it was stored, and
/// `take` leaves the document as it was. A summary with nothing to say of a
/// template stores no property for one.
#[test]
fn the_summary_carries_why_a_linked_template_was_not_drawn() {
    let mut doc = Document::new();
    let before = doc.clone();
    for not_drawn in templates_not_drawn() {
        let mut summary = ImportSummary::empty();
        summary.drawn = 12;
        summary.template_not_drawn = Some(not_drawn.clone());
        summary.store(&mut doc);
        assert_eq!(
            ImportSummary::load(&doc).and_then(|read| read.template_not_drawn),
            Some(not_drawn),
            "stored and read back"
        );
        assert_eq!(ImportSummary::take(&mut doc), Some(summary));
        assert_eq!(doc, before, "taken, the document is exactly what it was");
    }

    // Storing a summary with nothing to say of a template replaces what an
    // earlier one said, and adds no property of its own.
    let mut said = ImportSummary::empty();
    said.template_not_drawn = templates_not_drawn().into_iter().next();
    said.store(&mut doc);
    let silent = ImportSummary::empty();
    silent.store(&mut doc);
    assert_eq!(ImportSummary::load(&doc), Some(silent.clone()));
    assert_eq!(
        doc.summary_info
            .custom_properties
            .iter()
            .filter(|(tag, _)| tag.starts_with(SUMMARY_PROPERTY_PREFIX))
            .count(),
        silent.counts().len() + 2,
        "one property per field: the counts, the failed flag, the unit"
    );

    // A reason this build does not know reads as none said, not as another.
    said.store(&mut doc);
    for (tag, value) in &mut doc.summary_info.custom_properties {
        if tag.ends_with(".template_gap") {
            *value = "from-a-later-build".to_string();
        }
    }
    assert_eq!(
        ImportSummary::load(&doc).and_then(|read| read.template_not_drawn),
        None
    );
}

/// The line the command line gets for a template not drawn: which template
/// the drawing links, why it is not on the sheet, and what the sheet lacks
/// for it. Where no file was found it also names the variable that says
/// where the templates are.
#[test]
fn the_line_for_a_template_not_drawn_names_the_template_and_the_reason() {
    let expected = [
        "P&ID import: the drawing links the border template(s) CPE-SQ~2.IGR, CPE-SQ~3.PID, but no .igr file of that name was found; the template's frame and title block are not drawn. Set PID_TEMPLATE_LIBRARY to the folder that holds the templates.",
        "P&ID import: the drawing links the border template(s) CPE-SQ~2.IGR, CPE-SQ~3.PID, but nothing could be drawn from the file(s) found for it: CPE-SQ-A1.igr, CPE-SQ-A0.igr; the template's frame and title block are not drawn.",
        "P&ID import: the drawing links the border template(s) CPE-SQ~2.IGR, CPE-SQ~3.PID, but none of the files of that name fits its 841.00 x 594.00 mm page: CPE-SQ-A0.igr (1189.00 x 841.00 mm), CPE-SQ-A2.igr (no strokes); the template's frame and title block are not drawn.",
        "P&ID import: the drawing links the border template(s) CPE-SQ~2.IGR, CPE-SQ~3.PID, but it states no page size and its lines fit no standard sheet, so there is no size to choose a file by; the template's frame and title block are not drawn.",
    ];
    for (not_drawn, line) in templates_not_drawn().iter().zip(expected) {
        assert_eq!(not_drawn.headline(), line, "{not_drawn:?}");
    }
}

/// In every language of the interface the line still names the template
/// and what was found of it -- no catalog drops a value or leaves a
/// placeholder -- and a Chinese interface is told in Chinese. The language
/// is one loader for the whole process, so this runs in a child process of
/// the test binary.
#[test]
fn the_line_for_a_template_not_drawn_speaks_the_ui_language() {
    const CHILD: &str = "OCS_PID_TEMPLATE_LINE_CHILD";
    const NAME: &str = "io::pid::tests::the_line_for_a_template_not_drawn_speaks_the_ui_language";
    if std::env::var_os(CHILD).is_none() {
        let program = std::env::current_exe().expect("the test binary has a path");
        let output = std::process::Command::new(program)
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .expect("the test binary must start");
        assert!(
            output.status.success(),
            "the child run failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let cases = templates_not_drawn();
    for language in crate::i18n::Language::ALL {
        if language == crate::i18n::Language::System {
            continue;
        }
        crate::i18n::set_language(language).expect("Fluent resources must parse");
        for not_drawn in &cases {
            let line = not_drawn.headline();
            for value in values_said_of(not_drawn) {
                assert!(line.contains(value), "{language:?} drops {value}: {line}");
            }
            assert!(!line.contains("__ocs_fmt_"), "{language:?}: {line}");
        }
    }
    crate::i18n::set_language(crate::i18n::Language::ZhCn)
        .expect("Chinese Fluent resources must parse");
    let expected = [
        "P&ID 导入：图纸链接的图框模板 CPE-SQ~2.IGR, CPE-SQ~3.PID 未找到（没有这个名字的 .igr 文件），模板里的图框和标题栏未绘制。可将环境变量 PID_TEMPLATE_LIBRARY 指向存放模板的文件夹。",
        "P&ID 导入：图纸链接的图框模板 CPE-SQ~2.IGR, CPE-SQ~3.PID 找到了文件，但画不出内容：CPE-SQ-A1.igr, CPE-SQ-A0.igr；模板里的图框和标题栏未绘制。",
        "P&ID 导入：图纸链接的图框模板 CPE-SQ~2.IGR, CPE-SQ~3.PID 没有尺寸相符的文件：页面为 841.00 x 594.00 mm，找到的是 CPE-SQ-A0.igr (1189.00 x 841.00 mm), CPE-SQ-A2.igr (no strokes)；模板里的图框和标题栏未绘制。",
        "P&ID 导入：图纸链接了图框模板 CPE-SQ~2.IGR, CPE-SQ~3.PID，但未声明页面尺寸，线条也对不上标准图幅，无法按尺寸选定文件；模板里的图框和标题栏未绘制。",
    ];
    for (not_drawn, line) in cases.iter().zip(expected) {
        assert_eq!(not_drawn.headline(), line, "{not_drawn:?}");
    }
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

/// A body's arc whose byte says clockwise runs clockwise from its start
/// angle to its end angle, and lands on the sheet as the DXF arc that
/// covers the same points:
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
        clockwise: true,
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
/// clockwise, with `π` stored as `3.141592653589794`, and a placement
/// rotated a quarter turn draws the cap bulging left of its chord rather
/// than caved into the shell.
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
        clockwise: true,
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

/// A body's arc is read by its record's last byte, the direction (pid-parse
/// `docs/analysis/2026-10-23-the-arc-trailing-byte-is-the-direction.md`).
/// `FT-Ultrasonic Flowmeter.sym`'s wave is two semicircles over the same
/// angles, `90° -> 270°`, one clockwise and one counter-clockwise: the first
/// passes 0° and the second 180°, so the wave reads as an S. A mirrored
/// placement reflects both and keeps them on opposite sides.
#[test]
fn pid_symbol_arc_semicircles_over_the_same_angles_take_the_halves_their_bytes_name() {
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
    let wave = |center_y: f64, clockwise: bool| SymbolPrimitive::Arc {
        center: (0.0, center_y),
        radius: 0.00127,
        start_angle: FRAC_PI_2,
        end_angle: 3.0 * FRAC_PI_2,
        clockwise,
    };
    let midpoint = |primitive: &SymbolPrimitive, scale_y: f64| {
        let Some(EntityType::Arc(arc)) =
            shape_primitive(primitive, None, &placement(scale_y), &HashMap::new())
        else {
            panic!("the wave did not build");
        };
        let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
        assert!((sweep - PI).abs() < 1e-9, "a semicircle stays one: {sweep}");
        let angle = arc.start_angle + sweep / 2.0;
        (
            arc.center.x + arc.radius * angle.cos(),
            arc.center.y + arc.radius * angle.sin(),
        )
    };
    let close =
        |(x, y): (f64, f64), (ex, ey): (f64, f64)| (x - ex).abs() < 1e-6 && (y - ey).abs() < 1e-6;
    let upper = wave(0.00127, true);
    let lower = wave(-0.00127, false);

    let at = midpoint(&upper, 1.0);
    assert!(
        close(at, (1.27, 1.27)),
        "the clockwise half passes 0°: {at:?}"
    );
    let at = midpoint(&lower, 1.0);
    assert!(
        close(at, (-1.27, -1.27)),
        "the counter-clockwise half passes 180°: {at:?}"
    );

    let at = midpoint(&upper, -1.0);
    assert!(
        close(at, (1.27, -1.27)),
        "mirrored, the clockwise half still passes 0°: {at:?}"
    );
    let at = midpoint(&lower, -1.0);
    assert!(
        close(at, (-1.27, 1.27)),
        "mirrored, the counter-clockwise half still passes 180°: {at:?}"
    );
}

/// An arc past a half-turn keeps its long side. `Local Vent`'s cap runs
/// counter-clockwise `88.18° -> 273.44°`, 185.26°; folded to the short side
/// it became the 174.74° on the other side of its chord, and the gooseneck
/// bent down where the plot bends it up.
#[test]
fn pid_symbol_arc_past_a_half_turn_keeps_its_long_side() {
    use std::f64::consts::PI;
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
    let cap = SymbolPrimitive::Arc {
        center: (0.00119, -0.00254),
        radius: 0.00254,
        start_angle: 88.18_f64.to_radians(),
        end_angle: 273.44_f64.to_radians(),
        clockwise: false,
    };
    let Some(EntityType::Arc(arc)) = shape_primitive(&cap, None, &placement, &HashMap::new())
    else {
        panic!("the cap did not build");
    };
    let sweep = (arc.end_angle - arc.start_angle).rem_euclid(2.0 * PI);
    assert!(
        (sweep.to_degrees() - 185.26).abs() < 1e-9,
        "the cap keeps its 185.26°: {}",
        sweep.to_degrees()
    );
    let mid = arc.start_angle + sweep / 2.0;
    let mid_x = arc.center.x + arc.radius * mid.cos();
    assert!(
        mid_x < arc.center.x - 2.5,
        "the cap bulges left of its centre: mid_x {mid_x} vs centre x {}",
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
    let overlay = overlay_template(&template, &drawing, &mut doc, 1.0, FramePlacement::NONE)
        .expect("the fixture draws");
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

/// The view the importer states for a drawing that draws over the rectangle
/// `drawn` spans, on `page` where it names one: its centre, its height, and
/// the width it states it takes in (`view_height * aspect_ratio`).
fn opening_view(page: Option<(f64, f64)>, drawn: [(f64, f64); 2]) -> ((f64, f64), f64, f64) {
    let mut bounds = Bounds::new(Projection {
        mm_per_unit: 1.0,
        band: SheetBand::for_page(page),
    });
    for (x, y) in drawn {
        bounds.add_mm(x, y);
    }
    let mut doc = Document::new();
    frame_drawing(&mut doc, &bounds, page, (0.0, 0.0));
    let view = doc
        .vports
        .iter()
        .find(|vport| vport.name == "*Active")
        .expect("the importer states the opening view");
    (
        (view.view_target.x, view.view_target.y),
        view.view_height,
        view.view_height * view.aspect_ratio,
    )
}

fn assert_same_view(found: ((f64, f64), f64, f64), wanted: ((f64, f64), f64, f64)) {
    let ((x, y), height, width) = found;
    let ((wanted_x, wanted_y), wanted_height, wanted_width) = wanted;
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
    assert!(
        close(x, wanted_x)
            && close(y, wanted_y)
            && close(height, wanted_height)
            && close(width, wanted_width),
        "found {found:?}, wanted {wanted:?}"
    );
}

/// A landscape sheet -- `D07`'s A2, 594.328 x 420.566 -- opens centred on
/// the sheet and as high as three quarters of its width with a twentieth
/// over, 468.03 as measured, and the view states the width it has to take
/// in: the sheet's with a twentieth over. Restored at its height alone, it
/// showed 81.8% of the sheet's width on the canvas the backup panel leaves
/// (`readings.md` §8).
#[test]
fn a_landscape_sheet_opens_on_a_view_that_states_its_width() {
    let (width, height) = (594.328, 420.566);
    assert_same_view(
        opening_view(Some((width, height)), [(30.0, 40.0), (560.0, 400.0)]),
        (
            (width / 2.0, height / 2.0),
            width * 0.75 * 1.05,
            width * 1.05,
        ),
    );
}

/// A portrait sheet opens as high as the sheet with a twentieth over, and
/// states the sheet's width with a twentieth over: a canvas no narrower
/// than the sheet takes that in.
#[test]
fn a_portrait_sheet_opens_on_a_view_that_states_its_width() {
    let (width, height) = (420.0, 594.0);
    assert_same_view(
        opening_view(Some((width, height)), [(20.0, 30.0), (400.0, 560.0)]),
        ((width / 2.0, height / 2.0), height * 1.05, width * 1.05),
    );
}

/// A drawing that names no sheet opens on what it draws, and states the
/// width of what it draws with a twentieth over. What it draws here is
/// narrower than 4:3, so the height is its own and not three quarters of
/// its width.
#[test]
fn a_drawing_without_a_sheet_opens_on_a_view_that_states_the_width_it_draws_over() {
    assert_same_view(
        opening_view(None, [(10.0, 20.0), (310.0, 270.0)]),
        ((160.0, 145.0), 250.0 * 1.05, 300.0 * 1.05),
    );
}

/// The page sits where the drawing's frame record puts its corner: the
/// border drawn where no template frame covers the page, and the view the
/// drawing opens on, both start there, and keep the page's size. `N井场`
/// states its page from (-1.04, -1.34) mm.
#[test]
fn pid_template_frame_corner_moves_the_page_border_and_the_opening_view() {
    let page = (843.46, 596.71);
    let corner = (-1.043573, -1.338395);
    let mut doc = Document::new();
    draw_page_border(&mut doc, Some(page), corner);
    let border: Vec<(f64, f64)> = doc
        .entities()
        .find_map(|entity| match entity {
            EntityType::LwPolyline(polyline) => Some(
                polyline
                    .vertices
                    .iter()
                    .map(|vertex| (vertex.location.x, vertex.location.y))
                    .collect(),
            ),
            _ => None,
        })
        .expect("the border is drawn");
    let (x, y) = corner;
    assert_eq!(
        border,
        [
            (x, y),
            (x + page.0, y),
            (x + page.0, y + page.1),
            (x, y + page.1)
        ]
    );

    let mut bounds = Bounds::new(Projection {
        mm_per_unit: 1.0,
        band: SheetBand::for_page(Some(page)),
    });
    bounds.add_mm(30.0, 40.0);
    bounds.add_mm(800.0, 560.0);
    let mut doc = Document::new();
    frame_drawing(&mut doc, &bounds, Some(page), corner);
    let view = doc
        .vports
        .iter()
        .find(|vport| vport.name == "*Active")
        .expect("the importer states the opening view");
    let at_origin = opening_view(Some(page), [(30.0, 40.0), (800.0, 560.0)]);
    assert_same_view(
        (
            (view.view_target.x, view.view_target.y),
            view.view_height,
            view.view_height * view.aspect_ratio,
        ),
        (
            (x + page.0 / 2.0, y + page.1 / 2.0),
            at_origin.1,
            at_origin.2,
        ),
    );
}

/// A character style states its character width in thousandths of the
/// face's own (`width_permille`), and only a narrowing or a widening becomes
/// a width factor: `0` -- the face's own width, what nearly every style of a
/// drawing's own lettering states -- reads like `1000` and like no statement
/// at all, the way `pid-plot` reads it. Like the height, the width goes on
/// the sheet's own lettering only.
#[test]
fn pid_text_width_only_a_stated_width_becomes_a_width_factor() {
    assert_eq!(text_width_factor(None), None);
    assert_eq!(text_width_factor(Some(0)), None);
    assert_eq!(text_width_factor(Some(1000)), None);
    assert_eq!(text_width_factor(Some(800)), Some(0.8));
    assert_eq!(text_width_factor(Some(700)), Some(0.7));

    let lettered_on = |layer: &str| {
        let mut text = Text::new();
        text.common.layer = layer.to_string();
        let mut entity = EntityType::Text(text);
        apply_text_width(&mut entity, 0.8);
        match entity {
            EntityType::Text(text) => text.width_factor,
            _ => unreachable!("built as text"),
        }
    };
    assert_eq!(lettered_on(LAYER_TEXT), 0.8);
    assert_eq!(
        lettered_on(LAYER_SYMBOL),
        Text::new().width_factor,
        "a symbol's lettering takes its width from its own record"
    );
}

/// A symbol's own lettering letters at the width its record's style states:
/// the `M` and `S` of the motor and solenoid operators in the vendor's
/// drawings state `800`.
#[test]
fn pid_text_width_symbol_lettering_letters_at_its_styles_width() {
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
        text: "M".to_string(),
        at: (0.0, 0.0),
    };
    let width_of = |width_permille: Option<u16>| {
        let lettering = SymbolLettering {
            anchor_code: 0x15,
            rotation_rad: 0.0,
            letter_height_m: 0.0,
            height_m: Some(0.002_54),
            height_mode: None,
            width_permille,
            rgb: Some([0, 0, 0]),
            alignment: Some(TextAlignment::Center),
            line_spacing: Some(1.0),
            run_style_id: Default::default(),
            spans: Default::default(),
            font_name: Some("Arial".to_string()),
        };
        let Some(EntityType::Text(label)) =
            shape_primitive(&primitive, Some(&lettering), &placement, &HashMap::new())
        else {
            panic!("symbol text did not build");
        };
        label.width_factor
    };
    assert_eq!(width_of(Some(800)), 0.8);
    for width_permille in [None, Some(0), Some(1000)] {
        assert_eq!(
            width_of(width_permille),
            Text::new().width_factor,
            "{width_permille:?} keeps the face's own width"
        );
    }
}

/// The `CPE-SQ` title block letters at the width its styles state: every
/// label of the template states `800`, and the vendor's plots narrow them to
/// match. At the face's own width the company name ran over the title
/// block's right-hand line.
#[test]
fn pid_text_width_template_lettering_letters_at_its_styles_width() {
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
    let drawing = NormalizedPidGeometry {
        page_dimensions_mm: Some((841.0, 594.0)),
        ..Default::default()
    };
    let template = read_template(&igr, false).expect("the fixture parses");
    overlay_template(&template, &drawing, &mut doc, 1.0, FramePlacement::NONE)
        .expect("the fixture draws");
    let widths: Vec<(String, f64)> = doc
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Text(text) => Some((text.value.clone(), text.width_factor)),
            _ => None,
        })
        .collect();
    assert!(widths.len() >= 30, "a title block's labels: {widths:?}");
    let off: Vec<_> = widths
        .iter()
        .filter(|(_, width)| (width - 0.8).abs() > 1e-12)
        .collect();
    assert!(off.is_empty(), "labels not at the stated 0.8: {off:?}");
}

/// A drawing's own lettering letters at the width its style states, and only
/// where its style states one: in the export fixture `A01`, `项目经理` states
/// `800` and `文件号` beside it in the title block states `0`.
#[test]
fn pid_text_width_sheet_lettering_letters_at_its_styles_width() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|dir| {
            dir.join("pid-parse")
                .join("test-file")
                .join("export-test")
                .join("publish-data")
                .join("A01")
                .join("A01.pid")
        })
        .filter(|path| path.is_file());
    let Some(path) = path else {
        return; // fixture absent: soft-skip like the corpus tests do
    };
    let import = load_pid(&path).expect("the fixture imports");
    // The drawing's own lettering, not a linked template's that may letter
    // the same words.
    let widths_of = |value: &str| -> Vec<f64> {
        import
            .document
            .entities()
            .filter(|entity| {
                !entity
                    .common()
                    .extended_data
                    .records()
                    .iter()
                    .flat_map(|record| record.values.iter())
                    .any(|datum| {
                        matches!(datum, document::xdata::XDataValue::String(s) if s.starts_with("template="))
                    })
            })
            .filter_map(|entity| match entity {
                EntityType::Text(text) if text.value == value => Some(text.width_factor),
                _ => None,
            })
            .collect()
    };
    assert_eq!(widths_of("项目经理"), vec![0.8]);
    assert_eq!(widths_of("文件号"), vec![Text::new().width_factor]);
}

/// Narrowing a label keeps the hot point its record states. The canvas steps
/// back from the anchor by the run's advance measured at the width factor,
/// so a right-aligned label keeps its right end and a centred one its middle.
#[test]
fn pid_text_width_keeps_a_right_or_centred_labels_anchor() {
    use crate::entities::text::text_run_placement_at_scale;
    use crate::entities::text_support::text_local_bounds;

    let doc = Document::new();
    // `0x20` is right-on-bottom and `0x10` centre-on-bottom; beside each, the
    // share of the run's advance that lies left of the anchor.
    for (code, left_of_anchor) in [(0x20_u8, 1.0), (0x10, 0.5)] {
        let laid_out = |factor: Option<f64>| {
            let mut text = Text::new();
            text.value = "华北油田苏桥储气库群".to_string();
            text.height = 4.0;
            text.insertion_point.x = 700.0;
            text.insertion_point.y = 30.0;
            text.common.layer = LAYER_TEXT.to_string();
            let mut entity = EntityType::Text(text);
            apply_text_anchor(&mut entity, code, None);
            if let Some(factor) = factor {
                apply_text_width(&mut entity, factor);
            }
            let EntityType::Text(text) = entity else {
                unreachable!("built as text");
            };
            let placement = text_run_placement_at_scale(&text, &doc, 1.0);
            let advance = text_local_bounds(
                &placement.font,
                &text.value,
                placement.height,
                placement.width_factor,
                placement.oblique_angle,
            )
            .expect("the label lays out")
            .advance;
            let advance = f64::from(advance);
            (placement.origin[0] + left_of_anchor * advance, advance)
        };
        let (anchor, advance) = laid_out(None);
        let (narrow_anchor, narrow_advance) = laid_out(text_width_factor(Some(800)));
        assert!((anchor - 700.0).abs() < 1e-3, "{code:#04x}: {anchor}");
        assert!(
            (narrow_anchor - 700.0).abs() < 1e-3,
            "{code:#04x} narrowed: {narrow_anchor}"
        );
        assert!(
            (narrow_advance / advance - 0.8).abs() < 1e-4,
            "{code:#04x}: {narrow_advance} against {advance}"
        );
    }
}

/// A sheet label placed the way the importer places it -- the anchor code,
/// then the box when the record states one -- and laid out the way the canvas
/// lays it out: the entity, the run's origin and its advance along the
/// baseline, in drawing millimetres.
fn boxed_label(
    value: &str,
    insertion: (f64, f64),
    rotation: f64,
    code: u8,
    box_width_mm: Option<f64>,
    paragraph: Option<TextAlignment>,
) -> (Text, [f64; 2], f64) {
    use crate::entities::text::text_run_placement_at_scale;
    use crate::entities::text_support::text_local_bounds;

    let mut text = Text::new();
    text.value = value.to_string();
    text.height = 2.469;
    text.insertion_point.x = insertion.0;
    text.insertion_point.y = insertion.1;
    text.rotation = rotation;
    text.common.layer = LAYER_TEXT.to_string();
    let mut entity = EntityType::Text(text);
    apply_text_anchor(&mut entity, code, None);
    apply_text_box(&mut entity, code, box_width_mm, paragraph);
    let EntityType::Text(text) = entity else {
        unreachable!("built as text");
    };
    let placement = text_run_placement_at_scale(&text, &Document::new(), 1.0);
    let advance = text_local_bounds(
        &placement.font,
        &text.value,
        placement.height,
        placement.width_factor,
        placement.oblique_angle,
    )
    .expect("the label lays out")
    .advance;
    let origin = [placement.origin[0], placement.origin[1]];
    (text, origin, f64::from(advance))
}

/// A boxed record whose paragraph is left-aligned starts at its box's left
/// edge. The connectors' drawing numbers are the case: a 45.307 mm box with
/// the insertion at its middle (`0x48`), around a number some 28 mm long.
#[test]
fn pid_text_box_starts_a_left_aligned_run_at_the_box_left_edge() {
    let label = |width| {
        boxed_label(
            "DWG-0100集05-37",
            (106.956, 505.482),
            0.0,
            0x48,
            width,
            Some(TextAlignment::Left),
        )
    };
    let (text, origin, advance) = label(Some(45.307));
    let (_, unboxed, _) = label(None);
    assert!(advance < 40.0, "a box wider than its text: {advance}");
    assert_eq!(text.horizontal_alignment, TextHorizontalAlignment::Left);
    assert!(
        (origin[0] - (106.956 - 45.307 / 2.0)).abs() < 1e-6,
        "starts at the box's left edge: {origin:?}"
    );
    assert!(
        (origin[1] - unboxed[1]).abs() < 1e-9,
        "keeps the line the anchor code puts it on: {origin:?} against {unboxed:?}"
    );
}

/// A right-aligned paragraph hangs the run from the box's right edge: the
/// three such numbers of the reference drawings end 0.05 mm inside it.
#[test]
fn pid_text_box_ends_a_right_aligned_run_at_the_box_right_edge() {
    let (text, origin, advance) = boxed_label(
        "DWG-0100集05-42",
        (689.867, 300.0),
        0.0,
        0x48,
        Some(45.307),
        Some(TextAlignment::Right),
    );
    assert!(advance < 40.0, "a box wider than its text: {advance}");
    assert_eq!(text.horizontal_alignment, TextHorizontalAlignment::Right);
    assert!(
        (origin[0] + advance - (689.867 + 45.307 / 2.0)).abs() < 1e-3,
        "ends at the box's right edge: {origin:?} + {advance}"
    );
}

/// A centred paragraph letters the run at the box's middle, and the anchor
/// code still says where the box is: a left anchor (`0x05`) puts the
/// insertion on the box's left edge, so the run centres half a box further
/// along.
#[test]
fn pid_text_box_centres_a_centre_aligned_run_in_the_box() {
    let (text, origin, advance) = boxed_label(
        "323",
        (63.922, 335.647),
        0.0,
        0x05,
        Some(20.0),
        Some(TextAlignment::Center),
    );
    assert!(advance < 20.0, "a box wider than its text: {advance}");
    assert_eq!(text.horizontal_alignment, TextHorizontalAlignment::Center);
    assert!(
        (origin[0] + advance / 2.0 - (63.922 + 10.0)).abs() < 1e-3,
        "centred in the box: {origin:?} + {advance} / 2"
    );
}

/// The box lies along the baseline: a record lettered up the page moves up
/// it, and stays where it was across it.
#[test]
fn pid_text_box_moves_a_rotated_run_along_its_baseline() {
    let label = |width| {
        boxed_label(
            "DWG-0100集05-37",
            (74.005, 283.540),
            std::f64::consts::FRAC_PI_2,
            0x48,
            width,
            Some(TextAlignment::Left),
        )
    };
    let (_, origin, _) = label(Some(42.009));
    let (_, unboxed, _) = label(None);
    assert!(
        (origin[1] - (283.540 - 42.009 / 2.0)).abs() < 1e-6,
        "starts at the box's lower edge: {origin:?}"
    );
    // The canvas turns a run in `f32`: the centred one steps back half its
    // advance along a cosine of about -4e-8, some 7e-7 mm across.
    assert!(
        (origin[0] - unboxed[0]).abs() < 1e-3,
        "stays across the baseline: {origin:?} against {unboxed:?}"
    );
}

/// A record that states no box letters at its insertion by its anchor code,
/// whatever its paragraph says.
#[test]
fn pid_text_box_absent_leaves_the_anchor_code_in_charge() {
    let mut anchored = Text::new();
    anchored.value = "PG-1001".to_string();
    anchored.height = 2.469;
    anchored.insertion_point.x = 100.0;
    anchored.insertion_point.y = 50.0;
    anchored.common.layer = LAYER_TEXT.to_string();
    let mut anchored = EntityType::Text(anchored);
    apply_text_anchor(&mut anchored, 0x15, None);
    let EntityType::Text(anchored) = anchored else {
        unreachable!("built as text");
    };
    for paragraph in [
        None,
        Some(TextAlignment::Left),
        Some(TextAlignment::Center),
        Some(TextAlignment::Right),
    ] {
        let (text, _, _) = boxed_label("PG-1001", (100.0, 50.0), 0.0, 0x15, None, paragraph);
        assert_eq!(
            (
                text.horizontal_alignment,
                text.vertical_alignment,
                text.insertion_point,
                text.alignment_point,
            ),
            (
                anchored.horizontal_alignment,
                anchored.vertical_alignment,
                anchored.insertion_point,
                anchored.alignment_point,
            ),
            "{paragraph:?}"
        );
    }
}

/// Read from a drawing: `DWG-0100集05-38` is lettered twice on the corpus's
/// `一级闪蒸分离器B`, by records that state a 45.307 mm box around
/// insertions at x = 106.956 and 693.814 mm and a left-aligned paragraph. Both
/// start at their box's left edge.
#[test]
fn pid_text_box_sheet_numbers_start_at_their_box_left_edge() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|dir| {
            dir.join("pid-parse")
                .join("test-file")
                .join("集注站 凝析油闪蒸单元 一级闪蒸分离器B工艺及自控流程图.pid")
        })
        .filter(|path| path.is_file());
    let Some(path) = path else {
        return; // fixture absent: soft-skip like the corpus tests do
    };
    let import = load_pid(&path).expect("the fixture imports");
    let mut starts: Vec<(TextHorizontalAlignment, f64)> = import
        .document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Text(text) if text.value == "DWG-0100集05-38" => Some((
                text.horizontal_alignment,
                text.alignment_point.unwrap_or(text.insertion_point).x,
            )),
            _ => None,
        })
        .collect();
    starts.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(starts.len(), 2, "{starts:?}");
    for ((alignment, x), insertion) in starts.iter().zip([106.956, 693.814]) {
        assert_eq!(*alignment, TextHorizontalAlignment::Left, "{starts:?}");
        assert!((x - (insertion - 45.307 / 2.0)).abs() < 0.01, "{starts:?}");
    }
}

/// A sheet record's lettering as its styles state it: 7 pt in `font`, the
/// paragraph's lines spaced by `leading`, the height measured the way
/// `height_mode` says.
fn line_lettering(
    font: &str,
    leading: Option<pid_parse::style_link::LineLeading>,
    height_mode: Option<u8>,
) -> pid_parse::style_link::ResolvedTextHeight {
    pid_parse::style_link::ResolvedTextHeight {
        style_id: 0,
        height_m: 0.002_469,
        height_mode,
        width_permille: None,
        colour: None,
        alignment: None,
        line_spacing: None,
        leading,
        font_name: Some(font.to_string()),
    }
}

/// Where the canvas starts the baseline of a 2.469 mm sheet record inserted
/// at (100, 50) and placed by its anchor code, in drawing millimetres.
fn line_origin(
    code: u8,
    rotation: f64,
    lettering: Option<&pid_parse::style_link::ResolvedTextHeight>,
) -> [f64; 2] {
    use crate::entities::text::text_run_placement_at_scale;

    let mut text = Text::new();
    text.value = "PG-1001".to_string();
    text.height = 2.469;
    text.insertion_point.x = 100.0;
    text.insertion_point.y = 50.0;
    text.rotation = rotation;
    text.common.layer = LAYER_TEXT.to_string();
    let mut entity = EntityType::Text(text);
    apply_text_anchor(&mut entity, code, lettering);
    let EntityType::Text(text) = entity else {
        unreachable!("built as text");
    };
    let placement = text_run_placement_at_scale(&text, &Document::new(), 1.0);
    [placement.origin[0], placement.origin[1]]
}

/// The low nibble of a sheet record's anchor code names the line its
/// insertion point sits on, the way `pid-plot` reads it: `0` the baseline,
/// `1` and `3` the face's ascender line, `2` the top of the body. Arial
/// Narrow's ascender is 58 px of a 72 px body whose capitals take 44.
#[test]
fn pid_text_line_low_nibble_names_the_line_the_insertion_sits_on() {
    use pid_parse::style_link::LineLeading;

    let narrow = line_lettering("Arial Narrow", Some(LineLeading::FractionTile), Some(0x0D));
    for (code, above) in [
        (0x10_u8, 0.0),
        (0x00, 0.0),
        (0x01, 58.0 / 44.0),
        (0x21, 58.0 / 44.0),
        (0x03, 58.0 / 44.0),
        (0x12, 1.0),
    ] {
        let origin = line_origin(code, 0.0, Some(&narrow));
        assert!(
            (origin[1] - (50.0 - above * 2.469)).abs() < 1e-6,
            "{code:#04x}: baseline at {}",
            origin[1]
        );
    }
}

/// Any other low nibble centres the run on the middle of the body, halfway
/// between the face's ascender and descender lines, so where that is depends
/// on the face: half a cap height up in Arial Narrow, 0.539 of one in 仿宋
/// and 0.478 in Arial. A face outside `pid-plot`'s table, and a record with
/// no style at all, are measured as Arial, the way `pid-plot` measures them.
#[test]
fn pid_text_line_middle_sits_where_the_face_puts_it() {
    use pid_parse::style_link::LineLeading;

    for (font, middle) in [
        (Some("Arial Narrow"), 22.0 / 44.0),
        (Some("Arial"), 22.0 / 46.0),
        (Some("宋体"), 26.0 / 49.2),
        (Some("SimSun"), 26.0 / 49.2),
        (Some("仿宋"), 26.0 / 48.2),
        (Some("Braggadocio"), 22.0 / 46.0),
        (None, 22.0 / 46.0),
    ] {
        let lettering =
            font.map(|font| line_lettering(font, Some(LineLeading::FractionTile), Some(0x0D)));
        for code in [0x05_u8, 0x15, 0x25, 0x18, 0x38, 0x48, 0x58] {
            let origin = line_origin(code, 0.0, lettering.as_ref());
            assert!(
                (origin[1] - (50.0 - middle * 2.469)).abs() < 1e-6,
                "{font:?} {code:#04x}: baseline at {}",
                origin[1]
            );
        }
    }
}

/// The device tags lettered in Arial Narrow under a paragraph whose lines are
/// spaced by the face's ascent hang from half its ascender, 29 px against the
/// 44 its capitals take, rather than from the middle of the body: `pid-plot`
/// measured them 0.656 of a body down. Only an `8` does: a `5` under the same
/// paragraph keeps the middle, and so does an `8` under any other spacing.
#[test]
fn pid_text_line_ascent_spaced_tags_hang_from_half_the_ascender() {
    use pid_parse::style_link::LineLeading;

    let ascent_spaced = line_lettering(
        "Arial Narrow",
        Some(LineLeading::FractionAscender),
        Some(0x0D),
    );
    let tile_spaced = line_lettering("Arial Narrow", Some(LineLeading::FractionTile), Some(0x0D));
    for (code, lettering, above) in [
        (0x48_u8, &ascent_spaced, 29.0 / 44.0),
        (0x08, &ascent_spaced, 29.0 / 44.0),
        (0x45, &ascent_spaced, 0.5),
        (0x48, &tile_spaced, 0.5),
    ] {
        let origin = line_origin(code, 0.0, Some(lettering));
        assert!(
            (origin[1] - (50.0 - above * 2.469)).abs() < 1e-6,
            "{code:#04x} {:?}: baseline at {}",
            lettering.leading,
            origin[1]
        );
    }
}

/// A character style that measures its height to the face's ascent rather
/// than its capitals puts the ascender line one height up and the rest in
/// proportion: the top of the body at 44/58 of it and the middle at half
/// that. No sheet or template record of the corpus states it.
#[test]
fn pid_text_line_ascent_measured_height_scales_by_the_ascent() {
    let ascent_measured = line_lettering("Arial Narrow", None, Some(0x0C));
    for (code, above) in [
        (0x10_u8, 0.0),
        (0x01, 1.0),
        (0x12, 44.0 / 58.0),
        (0x15, 22.0 / 58.0),
    ] {
        let origin = line_origin(code, 0.0, Some(&ascent_measured));
        assert!(
            (origin[1] - (50.0 - above * 2.469)).abs() < 1e-6,
            "{code:#04x}: baseline at {}",
            origin[1]
        );
    }
}

/// The baseline drops across the run rather than down the page: a record
/// lettered up the page has its baseline to the right of the insertion.
#[test]
fn pid_text_line_rotated_record_drops_across_its_baseline() {
    use pid_parse::style_link::LineLeading;

    let narrow = line_lettering("Arial Narrow", Some(LineLeading::FractionTile), Some(0x0D));
    let origin = line_origin(0x01, std::f64::consts::FRAC_PI_2, Some(&narrow));
    assert!(
        (origin[0] - (100.0 + 58.0 / 44.0 * 2.469)).abs() < 1e-6,
        "{origin:?}"
    );
    assert!((origin[1] - 50.0).abs() < 1e-6, "{origin:?}");
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
    /// an import prepares one. The file states no plant item for any of its
    /// graphics; see [`import_stating`].
    fn import(geometry: &NormalizedPidGeometry, tables: &Styles) -> (Document, Built) {
        import_stating(geometry, tables, &SheetItems::default())
    }

    /// [`import`], with `items` as what the file states its graphics draw.
    fn import_stating(
        geometry: &NormalizedPidGeometry,
        tables: &Styles,
        items: &SheetItems,
    ) -> (Document, Built) {
        import_as_stated(
            geometry,
            tables,
            Stated {
                items,
                labels: &SheetLabels::default(),
            },
        )
    }

    /// [`import`], with `labels` as what the file hangs its labels on.
    fn import_labelled(
        geometry: &NormalizedPidGeometry,
        tables: &Styles,
        labels: &SheetLabels,
    ) -> (Document, Built) {
        import_as_stated(
            geometry,
            tables,
            Stated {
                items: &SheetItems::default(),
                labels,
            },
        )
    }

    /// [`import`], with `stated` as what the file says its graphics are.
    fn import_as_stated(
        geometry: &NormalizedPidGeometry,
        tables: &Styles,
        stated: Stated<'_>,
    ) -> (Document, Built) {
        let mut doc = prepare_document(&pid_parse::PidDocument::default());
        let built = build_document_entities(
            geometry,
            tables,
            stated,
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
        summarised(file, geometry).0
    }

    /// [`finished`], beside the summary `finish` hands back: what the import
    /// says of what it drew.
    fn summarised(file: &str, geometry: &NormalizedPidGeometry) -> (Document, ImportSummary) {
        summarised_as_stated(
            file,
            geometry,
            Stated {
                items: &SheetItems::default(),
                labels: &SheetLabels::default(),
            },
        )
    }

    /// [`summarised`], with `stated` as what the file says its graphics are.
    fn summarised_as_stated(
        file: &str,
        geometry: &NormalizedPidGeometry,
        stated: Stated<'_>,
    ) -> (Document, ImportSummary) {
        let tables = styles();
        let (mut doc, built) = import_as_stated(geometry, &tables, stated);
        let summary = finish(
            Path::new(file),
            geometry,
            None,
            &tables,
            ImportUnit::AssumedMetre,
            built,
            &LinkedTemplate::Unnamed,
            FramePlacement::NONE,
            &mut doc,
        );
        (doc, summary)
    }

    /// The valve the sheets of the program-level tests place, large enough
    /// to click on: a stub from 20 to 10 millimetres left of the insertion
    /// point, a ring of radius 10 around it, a stub from 10 to 20 right of
    /// it.
    fn ringed_valve() -> PidSymbolDefinition {
        let stub = |from: f64, to: f64| SymbolPrimitive::Line {
            start: (from, 0.0),
            end: (to, 0.0),
        };
        let ring = SymbolPrimitive::Circle {
            center: (0.0, 0.0),
            radius: 0.01,
        };
        body(
            BODY,
            vec![
                (stub(-0.02, -0.01), pen(0.25)),
                (ring, pen(0.25)),
                (stub(0.01, 0.02), pen(0.25)),
            ],
        )
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
            vec![ringed_valve(), cross_body()],
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

    /// A text of the two sheets below, on the displayed layer `Process`: 2.5
    /// millimetres high, standing on `at`, the bottom left of its lettering.
    fn lettering_at(oid: u32, at: (f64, f64), value: &str) -> pid_parse::PidGraphicEntity {
        record(
            oid,
            PidGraphicKind::Text {
                insertion: point(at.0, at.1),
                value: value.to_string(),
                height: 0.0025,
                rotation: 0.0,
                anchor_code: 0,
                text_box_width: None,
            },
            Some((Some("Process"), true)),
        )
    }

    /// The valve of those sheets placed at `at`, on the same layer.
    fn valve_placed(oid: u32, at: (f64, f64)) -> pid_parse::PidGraphicEntity {
        record(
            oid,
            placement(at, Some(VALVE), Some(BODY), 0.0),
            Some((Some("Process"), true)),
        )
    }

    /// The sheet the program-level test of what a symbol and its labels say
    /// of each other opens (`crate::app::pid_symbol_block_tests`; plan
    /// 2026-11-20-pid-symbol-identity, SI-3). It is built like the two
    /// above, with what a drawing file states of its graphics beside them:
    /// the items they draw, and what its labels hang on. In millimetres of
    /// the drawing:
    ///
    /// - a valve at (100, 100), a piping component the file names `GV-101`,
    ///   and four labels of it: `GV-101` at (85, 150), `DN50` at (85, 140),
    ///   `PN16` at (85, 130) and `CS` at (85, 120);
    /// - the same valve at (170, 100), a piping component the file gives no
    ///   name, and one label of it: `DN80` at (160, 120);
    /// - a run of line from (90, 80) to (180, 80), and its label
    ///   `P-1001-50-CS` at (120, 75);
    /// - `DWG-0201` at (60, 60), the label of what is no graphic of the
    ///   sheet, and `NOTE 4` at (60, 170), which is no label's.
    ///
    /// A text is 2.5 high and stands on its point, the bottom left of its
    /// lettering. The first label is filed before the valve it hangs on,
    /// the others after it.
    pub(crate) fn sheet_of_a_tagged_valve_and_its_labels() -> Document {
        let geometry = sheet(
            vec![
                lettering_at(801, (0.085, 0.15), "GV-101"),
                valve_placed(612, (0.1, 0.1)),
                lettering_at(802, (0.085, 0.14), "DN50"),
                lettering_at(803, (0.085, 0.13), "PN16"),
                lettering_at(804, (0.085, 0.12), "CS"),
                valve_placed(613, (0.17, 0.1)),
                lettering_at(805, (0.16, 0.12), "DN80"),
                record(
                    704,
                    PidGraphicKind::Polyline {
                        points: vec![point(0.09, 0.08), point(0.18, 0.08)],
                        closed: false,
                    },
                    Some((Some("Process"), true)),
                ),
                lettering_at(811, (0.12, 0.075), "P-1001-50-CS"),
                lettering_at(812, (0.06, 0.06), "DWG-0201"),
                lettering_at(813, (0.06, 0.17), "NOTE 4"),
            ],
            vec![ringed_valve()],
        );
        let items = SheetItems::stating([
            (
                612,
                plant_item(
                    "4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("GV-101"),
                ),
            ),
            (
                613,
                plant_item(
                    "7A3C5E9B1D2F4A6C8E0B2D4F6A8C0E13",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    None,
                ),
            ),
        ]);
        // Each text names the record of its label, and each label the
        // graphic it hangs on; `NOTE 4` names a record that is no label.
        let labels = labels_of(
            &[
                (801, 901),
                (802, 902),
                (803, 903),
                (804, 904),
                (805, 905),
                (811, 910),
                (812, 911),
                (813, 6),
            ],
            &[
                (901, 612),
                (902, 612),
                (903, 612),
                (904, 612),
                (905, 613),
                (910, 704),
                (911, 24_578),
            ],
            &[704],
        );
        summarised_as_stated(
            "tagged-valve.pid",
            &geometry,
            Stated {
                items: &items,
                labels: &labels,
            },
        )
        .0
    }

    /// The sheet the test of what the P&ID editor's inspector shows of a
    /// symbol the drawing file says less of opens
    /// (`crate::app::pid_shell::pid_editor_screen`; plan
    /// 2026-11-20-pid-symbol-identity, SI-4). It is built like the one
    /// above, of the same valve. In millimetres of the drawing:
    ///
    /// - a valve at (100, 100), an off-page connector by its item, which the
    ///   file publishes as no class and gives no name, and one label of it:
    ///   `TO UNIT 2` at (85, 120);
    /// - a valve at (170, 100) the file states no item for, and one label
    ///   of it: `SPARE` at (160, 120);
    /// - a valve at (100, 160) that no label hangs on, an item named `ZZ-9`
    ///   and published as `PIDBranchPoint`: a class the items of a drawing
    ///   file are not published as, but those of a `_Data.xml` are.
    pub(crate) fn sheet_of_symbols_the_file_says_less_of() -> Document {
        let geometry = sheet(
            vec![
                valve_placed(621, (0.1, 0.1)),
                lettering_at(821, (0.085, 0.12), "TO UNIT 2"),
                valve_placed(622, (0.17, 0.1)),
                lettering_at(822, (0.16, 0.12), "SPARE"),
                valve_placed(623, (0.1, 0.16)),
            ],
            vec![ringed_valve()],
        );
        let items = SheetItems::stating([
            (
                621,
                plant_item("9C0D2E4F6A8B1C3D5E7F9A0B2C4D6E81", "OPC", None, None),
            ),
            (
                623,
                plant_item(
                    "1B3D5F7A9C0E2A4C6E8B0D2F4A6C8E05",
                    "OPC",
                    Some("PIDBranchPoint"),
                    Some("ZZ-9"),
                ),
            ),
        ]);
        let labels = labels_of(&[(821, 921), (822, 922)], &[(921, 621), (922, 622)], &[]);
        summarised_as_stated(
            "symbols-said-less-of.pid",
            &geometry,
            Stated {
                items: &items,
                labels: &labels,
            },
        )
        .0
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
                "graphic=612",
            ],
            "the placement's record, the symbol's name as the drawing spells it, and the placement's own number in the file"
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
                "graphic=612",
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
            [
                "role=symbol",
                "extent=3.00x3.00",
                "symbol=Ball Valve",
                "graphic=701",
            ]
        );
        assert_eq!(
            says(placed[1]),
            ["role=symbol", "extent=3.00x3.00", "graphic=702"],
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
                "graphic=801",
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

    /// The import's summary counts the symbols the way they are filed:
    /// `symbol_blocks` is the references and `symbol_definitions` the
    /// definitions they refer to. A valve placed twice as it is filed and
    /// once turned a quarter is three references to two definitions --
    /// neither the one symbol the three name nor the three placements is
    /// the number of definitions. The eighteen counts the summary kept
    /// before are what they were: a block's members count as drawn and
    /// towards their sheet layer, the reference does not. Stored into the
    /// document for the open to pick up, both counts ride under their own
    /// keys and read back.
    #[test]
    fn the_summary_counts_the_references_and_the_definitions_they_refer_to() {
        let on = Some((Some("Process"), true));
        let valve = |oid: u32, at: (f64, f64), rotation: f64| {
            record(oid, placement(at, Some(VALVE), Some(BODY), rotation), on)
        };
        let geometry = sheet(
            vec![
                valve(1201, (0.1, 0.2), 0.0),
                valve(1202, (0.3, 0.1), 0.0),
                valve(1203, (0.2, 0.3), std::f64::consts::FRAC_PI_2),
                record(
                    1204,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.0),
                        end: point(0.05, 0.0),
                    },
                    on,
                ),
            ],
            vec![valve_body()],
        );
        let (mut doc, summary) = summarised("three-valves.pid", &geometry);

        let counts = summary.counts();
        assert_eq!(
            counts[..18],
            [
                ("drawn", 16),
                ("decoded", 4),
                ("missing", 0),
                ("sheet_layers", 1),
                ("layered_entities", 16),
                ("unresolved_sheet_layers", 0),
                ("sheet_layer_names", 1),
                ("sheet_layers_off", 0),
                ("driving_dimensions", 0),
                ("template_bodies", 0),
                ("parametric_placements", 0),
                ("cache_bodies", 3),
                ("library_bodies", 0),
                ("hidden_strokes_skipped", 0),
                ("label_anchors_undrawn", 0),
                ("lettering_flattened", 0),
                ("template_entities", 0),
                ("template_missing", 0),
            ],
            "what the summary counted before: a pipe and three placements of five entities each, on one sheet layer"
        );
        let names: Vec<&str> = references(&doc)
            .into_iter()
            .map(|reference| as_insert(reference).block_name.as_str())
            .collect();
        assert_eq!(
            names,
            ["SPPID_Ball Valve", "SPPID_Ball Valve", "SPPID_Ball Valve_2"],
            "what the two new counts count: the references, and the definitions they name"
        );
        assert_eq!(
            doc.block_records.len(),
            blocks_to_start_with() + 2,
            "and no definition beside those two"
        );
        assert_eq!(
            counts[18..20],
            [("symbol_blocks", 3), ("symbol_definitions", 2)],
            "three references to two definitions"
        );
        assert_eq!((summary.symbol_blocks, summary.symbol_definitions), (3, 2));

        summary.store(&mut doc);
        let stored = |key: &str| {
            let tag = format!("{SUMMARY_PROPERTY_PREFIX}{key}");
            doc.summary_info
                .custom_properties
                .iter()
                .find(|(name, _)| *name == tag)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            (
                stored("symbol_blocks").as_deref(),
                stored("symbol_definitions").as_deref()
            ),
            (Some("3"), Some("2")),
            "each rides the document under its own key"
        );
        assert_eq!(ImportSummary::take(&mut doc), Some(summary));
    }

    /// A plant item as a drawing file states one.
    fn plant_item(
        uid: &str,
        item_type: &str,
        class: Option<&'static str>,
        name: Option<&str>,
    ) -> SheetItem {
        SheetItem {
            uid: uid.to_string(),
            item_type: item_type.to_string(),
            class,
            name: name.map(str::to_string),
        }
    }

    /// A valve placed at (`x`, 0.2) metres on a displayed layer `Process`.
    fn valve_on_process(oid: u32, x: f64) -> pid_parse::PidGraphicEntity {
        record(
            oid,
            placement((x, 0.2), Some(VALVE), Some(BODY), 0.0),
            Some((Some("Process"), true)),
        )
    }

    /// A reference states what the drawing file says its graphic draws
    /// (plan 2026-11-20-pid-symbol-identity): the graphic's own number, the
    /// item and its kind, the class the item is published as -- marked as
    /// the file's own reading, `resolved=item` -- and the item's name as the
    /// tag, marked as the file's, `tag_source=file`. `class`, `label` and
    /// `resolved` stand where a published `_Data.xml` writes them, before
    /// the symbol's name; the keys only the file gives follow it. Nothing is
    /// added inside the block.
    #[test]
    fn the_reference_states_the_item_its_graphic_draws() {
        let geometry = sheet(vec![valve_on_process(612, 0.1)], vec![valve_body()]);
        let tables = styled(styles(), 612, "Equipment - New");
        let items = SheetItems::stating([(
            612,
            plant_item(
                "4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                "PipingComp",
                Some("PIDPipingComponent"),
                Some("GV-101"),
            ),
        )]);
        let (doc, built) = import_stating(&geometry, &tables, &items);

        let placed = references(&doc);
        assert_eq!(placed.len(), 1);
        assert_eq!(
            says(placed[0]),
            [
                "sheet_layer=Process",
                "sheet_layer_oid=12",
                "role=symbol",
                "style=Equipment - New",
                "extent=4.00x3.00",
                "class=PIDPipingComponent",
                "label=GV-101",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
                "item=4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                "item_type=PipingComp",
                "tag_source=file",
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
        assert_eq!((built.symbol_items, built.symbol_tags), (1, 1));
    }

    /// What the file does not decide is not stated. An item the file
    /// publishes no class for -- an off-page connector -- gets no `class`
    /// and no `resolved`; an item the file does not name gets no `label`
    /// and no `tag_source`; a graphic the file states no item for says its
    /// number alone; and a placement whose record has no number says
    /// nothing new.
    #[test]
    fn a_reference_states_only_what_the_file_decides() {
        let mut unnumbered = valve_on_process(615, 0.4);
        unnumbered.graphic_oid = None;
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                valve_on_process(613, 0.2),
                valve_on_process(614, 0.3),
                unnumbered,
            ],
            vec![valve_body()],
        );
        let items = SheetItems::stating([
            (612, plant_item("CONNECTOR", "OPC", None, None)),
            (
                613,
                plant_item("FITTING", "PipingComp", Some("PIDPipingComponent"), None),
            ),
        ]);
        let (doc, built) = import_stating(&geometry, &styles(), &items);

        let said: Vec<Vec<String>> = references(&doc).into_iter().map(says).collect();
        assert_eq!(
            said,
            [
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "symbol=Ball Valve",
                    "graphic=612",
                    "item=CONNECTOR",
                    "item_type=OPC",
                ],
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "class=PIDPipingComponent",
                    "resolved=item",
                    "symbol=Ball Valve",
                    "graphic=613",
                    "item=FITTING",
                    "item_type=PipingComp",
                ],
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "symbol=Ball Valve",
                    "graphic=614",
                ],
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "symbol=Ball Valve",
                ],
            ]
        );
        assert_eq!(
            (built.symbol_items, built.symbol_tags),
            (2, 0),
            "two of the four state an item, and none of them a tag"
        );
    }

    /// Beside a published `_Data.xml`, what it published stands: its class,
    /// its tag, its oid and how it was resolved are the values they were,
    /// and a tag it gave is not marked with a source. The file adds what
    /// the XML left out: the graphic, the item and its kind on every
    /// reference; its own name where the XML gives the object none, marked
    /// `tag_source=file`; its own class where the XML publishes nothing for
    /// the graphic, marked `resolved=item`.
    #[test]
    fn what_data_xml_published_stands_and_the_file_adds_what_it_left_out() {
        let xml = r#"<Container>
  <PIDInlineComponent><IObject UID="VALVE" ItemTag="GV-101"/></PIDInlineComponent>
  <PIDRepresentation><IObject UID="REP1"/><IDrawingRepresentation GraphicOID="612"/></PIDRepresentation>
  <Rel><IRel UID1="VALVE" UID2="REP1" DefUID="DwgRepresentationComposition"/></Rel>
  <PIDInstrument><IObject UID="GAUGE"/></PIDInstrument>
  <PIDRepresentation><IObject UID="REP2"/><IDrawingRepresentation GraphicOID="613"/></PIDRepresentation>
  <Rel><IRel UID1="GAUGE" UID2="REP2" DefUID="DwgRepresentationComposition"/></Rel>
</Container>"#;
        let mut tables = styles();
        tables.semantics = Some(PidSemanticIndex::from_xml(
            xml,
            &pid_parse::PidDocument::default(),
        ));
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                valve_on_process(613, 0.2),
                valve_on_process(614, 0.3),
            ],
            vec![valve_body()],
        );
        let items = SheetItems::stating([
            (
                612,
                plant_item(
                    "VALVE-ITEM",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("GV-101 AS THE FILE HAS IT"),
                ),
            ),
            (
                613,
                plant_item(
                    "GAUGE-ITEM",
                    "Instrument",
                    Some("PIDInstrument"),
                    Some("PG-7"),
                ),
            ),
            (
                614,
                plant_item("NOZZLE-ITEM", "Nozzle", Some("PIDNozzle"), Some("N1")),
            ),
        ]);
        let (doc, built) = import_stating(&geometry, &tables, &items);

        let said: Vec<Vec<String>> = references(&doc).into_iter().map(says).collect();
        assert_eq!(
            said,
            [
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "class=PIDInlineComponent",
                    "label=GV-101",
                    "oid=612",
                    "resolved=direct",
                    "symbol=Ball Valve",
                    "graphic=612",
                    "item=VALVE-ITEM",
                    "item_type=PipingComp",
                ],
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "class=PIDInstrument",
                    "label=PG-7",
                    "oid=613",
                    "resolved=direct",
                    "symbol=Ball Valve",
                    "graphic=613",
                    "item=GAUGE-ITEM",
                    "item_type=Instrument",
                    "tag_source=file",
                ],
                vec![
                    "sheet_layer=Process",
                    "sheet_layer_oid=12",
                    "role=symbol",
                    "extent=4.00x3.00",
                    "class=PIDNozzle",
                    "label=N1",
                    "resolved=item",
                    "symbol=Ball Valve",
                    "graphic=614",
                    "item=NOZZLE-ITEM",
                    "item_type=Nozzle",
                    "tag_source=file",
                ],
            ]
        );
        assert_eq!((built.symbol_items, built.symbol_tags), (3, 3));
    }

    /// The summary counts the references the file states an item for and
    /// the references that carry a tag, after the counts it kept before.
    /// Both ride the document under their own keys and read back.
    #[test]
    fn the_summary_counts_the_references_that_state_an_item_and_a_tag() {
        let geometry = sheet(
            vec![
                valve_on_process(1301, 0.1),
                valve_on_process(1302, 0.2),
                valve_on_process(1303, 0.3),
            ],
            vec![valve_body()],
        );
        let items = SheetItems::stating([
            (
                1301,
                plant_item(
                    "A",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("GV-101"),
                ),
            ),
            (
                1302,
                plant_item("B", "PipingComp", Some("PIDPipingComponent"), None),
            ),
        ]);
        let tables = styles();
        let (mut doc, built) = import_stating(&geometry, &tables, &items);
        let summary = finish(
            Path::new("three-valves.pid"),
            &geometry,
            None,
            &tables,
            ImportUnit::AssumedMetre,
            built,
            &LinkedTemplate::Unnamed,
            FramePlacement::NONE,
            &mut doc,
        );

        assert_eq!(
            summary.counts()[18..],
            [
                ("symbol_blocks", 3),
                ("symbol_definitions", 1),
                ("symbol_items", 2),
                ("symbol_tags", 1),
                ("symbol_labels", 0),
                ("other_labels", 0),
            ],
            "three references: two state an item, one of them a tag; the sheet has no label"
        );
        assert_eq!((summary.symbol_items, summary.symbol_tags), (2, 1));

        summary.store(&mut doc);
        let stored = |key: &str| {
            let tag = format!("{SUMMARY_PROPERTY_PREFIX}{key}");
            doc.summary_info
                .custom_properties
                .iter()
                .find(|(name, _)| *name == tag)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            (
                stored("symbol_items").as_deref(),
                stored("symbol_tags").as_deref()
            ),
            (Some("2"), Some("1")),
            "each rides the document under its own key"
        );
        assert_eq!(ImportSummary::take(&mut doc), Some(summary));
    }

    /// A key written after its record was made takes its place in the
    /// record's order, not the end, so a record reads the same whichever
    /// way it was arrived at. A key the record already holds has its value
    /// replaced where it stands; an empty value writes nothing; keys the
    /// order does not know go last, in the order they come; what is not a
    /// `key=value` string is left where it is. The record keeps its place
    /// among the entity's records, and an entity that has none gets one.
    #[test]
    fn a_key_stamped_later_takes_its_place_in_the_record() {
        use document::xdata::{ExtendedDataRecord, XDataValue};

        let a_line = || {
            EntityType::Line(Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
            ))
        };
        let mut entity = a_line();
        attach_pid_metadata(
            &mut entity,
            None,
            None,
            Some("symbol"),
            Some("Equipment - New"),
            None,
        );
        attach_symbol_name(&mut entity, "Ball Valve");
        let mut other = ExtendedDataRecord::new("SOMEONE_ELSE");
        other.add_value(XDataValue::String("label=theirs".to_string()));
        entity.common_mut().extended_data.add_record(other.clone());
        assert_eq!(
            says(&entity),
            ["role=symbol", "style=Equipment - New", "symbol=Ball Valve"]
        );

        stamp_pid_pairs(
            &mut entity,
            &[
                ("graphic", "612"),
                ("item", "ITEM"),
                ("item_type", "PipingComp"),
            ],
        );
        stamp_pid_pairs(
            &mut entity,
            &[
                ("label", "GV-101"),
                ("tag_source", "file"),
                ("class", "PIDPipingComponent"),
                ("resolved", "item"),
            ],
        );
        assert_eq!(
            says(&entity),
            [
                "role=symbol",
                "style=Equipment - New",
                "class=PIDPipingComponent",
                "label=GV-101",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
                "item=ITEM",
                "item_type=PipingComp",
                "tag_source=file",
            ],
            "each new key after the last key that comes no later than it"
        );

        // What the plant database later says of the same item: its tag in
        // place of the file's, and the file's name kept beside it.
        stamp_pid_pairs(
            &mut entity,
            &[
                ("label", "GV-101A"),
                ("tag_source", "database"),
                ("label_in_file", "GV-101"),
                ("oid", ""),
                ("made_up", "1"),
                ("another", "2"),
            ],
        );
        assert_eq!(
            says(&entity),
            [
                "role=symbol",
                "style=Equipment - New",
                "class=PIDPipingComponent",
                "label=GV-101A",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
                "item=ITEM",
                "item_type=PipingComp",
                "tag_source=database",
                "label_in_file=GV-101",
                "made_up=1",
                "another=2",
            ],
            "a held key is replaced where it stands, an empty value is not written"
        );
        // A value is read by its whole key: `label` is not `label_in_file`,
        // nor is the start of a key a key.
        assert_eq!(
            [
                pid_value(&entity, "label"),
                pid_value(&entity, "label_in_file"),
                pid_value(&entity, "item"),
                pid_value(&entity, "item_type"),
                pid_value(&entity, "label_in"),
                pid_value(&entity, "oid"),
            ],
            [
                Some("GV-101A"),
                Some("GV-101"),
                Some("ITEM"),
                Some("PipingComp"),
                None,
                None,
            ]
        );
        let records: Vec<&str> = entity
            .common()
            .extended_data
            .records()
            .iter()
            .map(|record| record.application_name.as_str())
            .collect();
        assert_eq!(
            records,
            [PID_SEMANTICS_XDATA_APP, "SOMEONE_ELSE"],
            "the record stays where it was among the entity's records"
        );
        assert_eq!(
            entity.common().extended_data.get_record("SOMEONE_ELSE"),
            Some(&other),
            "and another application's record is not read as this one"
        );

        // A value that is not a `key=value` string stays where it is, and
        // keys still find their places around it.
        let mut mixed = a_line();
        let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
        record.add_value(XDataValue::String("role=symbol".to_string()));
        record.add_value(XDataValue::Integer16(7));
        record.add_value(XDataValue::String("symbol=Ball Valve".to_string()));
        mixed.common_mut().extended_data.add_record(record);
        stamp_pid_pairs(&mut mixed, &[("graphic", "9"), ("label", "GV-9")]);
        let values = &mixed
            .common()
            .extended_data
            .get_record(PID_SEMANTICS_XDATA_APP)
            .expect("the record")
            .values;
        assert_eq!(
            values,
            &[
                XDataValue::String("role=symbol".to_string()),
                XDataValue::String("label=GV-9".to_string()),
                XDataValue::Integer16(7),
                XDataValue::String("symbol=Ball Valve".to_string()),
                XDataValue::String("graphic=9".to_string()),
            ]
        );

        let mut bare = a_line();
        stamp_pid_pairs(&mut bare, &[("label", "")]);
        assert!(
            bare.common().extended_data.is_empty(),
            "nothing to write makes no record"
        );
        stamp_pid_pairs(&mut bare, &[("graphic", "7")]);
        assert_eq!(says(&bare), ["graphic=7"]);
    }

    /// A key taken out of a record leaves the rest of the record as it was:
    /// the other keys in their order, what is no `key=value` string, and the
    /// record's place among the entity's records. Every statement of the
    /// key goes. A key is its whole name; one the record does not state is
    /// not taken, and an entity with no record gets none.
    #[test]
    fn a_key_taken_out_leaves_the_rest_of_its_record() {
        use document::xdata::{ExtendedDataRecord, XDataValue};

        let a_line = || {
            EntityType::Line(Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
            ))
        };
        let string = |value: &str| XDataValue::String(value.to_string());
        let mut entity = a_line();
        let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
        record.add_value(string("role=symbol"));
        record.add_value(string("label=GV-101A"));
        record.add_value(XDataValue::Integer16(7));
        record.add_value(string("tag_source=database"));
        record.add_value(string("label_in_file=GV-101"));
        record.add_value(string("label_in_file=GV-1"));
        let mut other = ExtendedDataRecord::new("SOMEONE_ELSE");
        other.add_value(string("label_in_file=theirs"));
        let whole = record.values.clone();
        entity.common_mut().extended_data.add_record(record);
        entity.common_mut().extended_data.add_record(other.clone());
        let held = |entity: &EntityType| {
            entity
                .common()
                .extended_data
                .get_record(PID_SEMANTICS_XDATA_APP)
                .expect("the record")
                .values
                .clone()
        };

        // A key is its whole name. `label_in` is none of the record's,
        // though two of them start with it; `label_in_file_too` is none,
        // though it starts with one; nor is `oid`. Nothing is taken.
        clear_pid_key(&mut entity, "label_in");
        clear_pid_key(&mut entity, "label_in_file_too");
        clear_pid_key(&mut entity, "oid");
        assert_eq!(held(&entity), whole);

        clear_pid_key(&mut entity, "label_in_file");
        let left = [
            string("role=symbol"),
            string("label=GV-101A"),
            XDataValue::Integer16(7),
            string("tag_source=database"),
        ];
        assert_eq!(held(&entity), left, "both statements of the key are gone");
        let records: Vec<&str> = entity
            .common()
            .extended_data
            .records()
            .iter()
            .map(|record| record.application_name.as_str())
            .collect();
        assert_eq!(records, [PID_SEMANTICS_XDATA_APP, "SOMEONE_ELSE"]);
        assert_eq!(
            entity.common().extended_data.get_record("SOMEONE_ELSE"),
            Some(&other),
            "another application's record is not this one"
        );

        let mut bare = a_line();
        clear_pid_key(&mut bare, "label");
        assert!(
            bare.common().extended_data.is_empty(),
            "nothing to take makes no record"
        );
    }

    /// The tags a plant database gives, by the ids of their items spelt as
    /// the dump folds them: no braces, no dashes, capitals.
    fn tags_of(tags: &[(&str, &str)]) -> HashMap<String, String> {
        tags.iter()
            .map(|(item, tag)| (item.to_string(), tag.to_string()))
            .collect()
    }

    /// Nothing of a sheet was drawn in this program.
    fn nothing_drawn_here(_: &EntityType) -> bool {
        false
    }

    /// What a valve of the sheets below says: the four lines every one of
    /// them starts with, then `rest`.
    fn valve_saying(rest: &[&str]) -> Vec<String> {
        [
            "sheet_layer=Process",
            "sheet_layer_oid=12",
            "role=symbol",
            "extent=4.00x3.00",
        ]
        .iter()
        .chain(rest)
        .map(|line| line.to_string())
        .collect()
    }

    /// What every block reference of model space says, in the order they
    /// were filed.
    fn said_by_the_references(doc: &Document) -> Vec<Vec<String>> {
        references(doc).into_iter().map(says).collect()
    }

    /// The sheet the tags of a plant database are put on, as an import of
    /// the file alone leaves it: six valves on a displayed layer `Process`,
    /// in the order they are filed --
    ///
    /// - 612, a piping component the file names `GV-101`;
    /// - 613, a piping component the file gives no name;
    /// - 614, an instrument the file names `PIA-0112101B`, its id spelt
    ///   with braces, dashes and small letters;
    /// - 615, a nozzle the file names `N1`;
    /// - 616, a graphic the file states no item for;
    /// - 617, a piping component the file names `K-1`.
    pub(crate) fn sheet_for_the_database() -> Document {
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.05),
                valve_on_process(613, 0.1),
                valve_on_process(614, 0.15),
                valve_on_process(615, 0.2),
                valve_on_process(616, 0.25),
                valve_on_process(617, 0.3),
            ],
            vec![valve_body()],
        );
        let items = SheetItems::stating([
            (
                612,
                plant_item(
                    "4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("GV-101"),
                ),
            ),
            (
                613,
                plant_item(
                    "7A3C5E9B1D2F4A6C8E0B2D4F6A8C0E13",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    None,
                ),
            ),
            (
                614,
                plant_item(
                    "{0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}",
                    "Instrument",
                    Some("PIDInstrument"),
                    Some("PIA-0112101B"),
                ),
            ),
            (
                615,
                plant_item(
                    "AAAA0000BBBB1111CCCC2222DDDD3333",
                    "Nozzle",
                    Some("PIDNozzle"),
                    Some("N1"),
                ),
            ),
            (
                617,
                plant_item(
                    "EEEE4444FFFF5555AAAA6666BBBB7777",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("K-1"),
                ),
            ),
        ]);
        let tables = styles();
        import_stating(&geometry, &tables, &items).0
    }

    /// The tags a database gives three items of that sheet: the first the
    /// name the file has, the second where the file has none, the third
    /// another than the file's, with blanks around it. It gives a fourth
    /// item a blank tag, and tags an item that is on another sheet.
    pub(crate) fn tags_for_that_sheet() -> HashMap<String, String> {
        tags_of(&[
            ("4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21", "GV-101"),
            ("7A3C5E9B1D2F4A6C8E0B2D4F6A8C0E13", "BV1301102B"),
            ("0B1C2D3E4F5061728394A5B6C7D8E9F0", "  PIA-old_0112101A "),
            ("EEEE4444FFFF5555AAAA6666BBBB7777", "   "),
            ("99990000999900009999000099990000", "ON-ANOTHER-SHEET"),
        ])
    }

    /// The tag the plant database gives a symbol's item is the symbol's tag
    /// (plan 2026-11-20-pid-symbol-identity, SPEC sections 4.2 and 4.4),
    /// marked `tag_source=database`: in place of the file's own name where
    /// the file has one, and where a tag stands in a record where it has
    /// none. The file's name stays beside it, as `label_in_file`, only
    /// where it is another name. An item's id is matched as the dump folds
    /// it, and a tag comes without the blanks around it. What the database
    /// has no tag for -- an item it does not hold, an item whose tag is
    /// blank, a graphic of no item -- says what it said.
    #[test]
    fn the_database_tag_of_an_item_is_its_symbols_tag() {
        let mut doc = sheet_for_the_database();
        let before = said_by_the_references(&doc);
        let loose_before: Vec<Vec<String>> = loose(&doc).into_iter().map(says).collect();

        let fill = fill_database_tags(&mut doc, &tags_for_that_sheet(), nothing_drawn_here);

        assert_eq!(
            fill,
            DatabaseTagFill {
                tagged: 3,
                differing: 1,
                changed: 3,
            }
        );
        let said = said_by_the_references(&doc);
        assert_eq!(
            said[0],
            valve_saying(&[
                "class=PIDPipingComponent",
                "label=GV-101",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=612",
                "item=4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
                "item_type=PipingComp",
                "tag_source=database",
            ]),
            "the name the file has: the tag is the database's, and nothing is kept beside it"
        );
        assert_eq!(
            said[1],
            valve_saying(&[
                "class=PIDPipingComponent",
                "label=BV1301102B",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=613",
                "item=7A3C5E9B1D2F4A6C8E0B2D4F6A8C0E13",
                "item_type=PipingComp",
                "tag_source=database",
            ]),
            "no name in the file: the tag stands where a tag stands"
        );
        assert_eq!(
            said[2],
            valve_saying(&[
                "class=PIDInstrument",
                "label=PIA-old_0112101A",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=614",
                "item={0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}",
                "item_type=Instrument",
                "tag_source=database",
                "label_in_file=PIA-0112101B",
            ]),
            "another name: the file's is kept beside the database's"
        );
        assert_eq!(
            &said[3..],
            &before[3..],
            "what the database has no tag for says what it said"
        );
        assert_eq!(
            &before[3..],
            [
                valve_saying(&[
                    "class=PIDNozzle",
                    "label=N1",
                    "resolved=item",
                    "symbol=Ball Valve",
                    "graphic=615",
                    "item=AAAA0000BBBB1111CCCC2222DDDD3333",
                    "item_type=Nozzle",
                    "tag_source=file",
                ]),
                valve_saying(&["symbol=Ball Valve", "graphic=616"]),
                valve_saying(&[
                    "class=PIDPipingComponent",
                    "label=K-1",
                    "resolved=item",
                    "symbol=Ball Valve",
                    "graphic=617",
                    "item=EEEE4444FFFF5555AAAA6666BBBB7777",
                    "item_type=PipingComp",
                    "tag_source=file",
                ]),
            ]
        );
        let loose_after: Vec<Vec<String>> = loose(&doc).into_iter().map(says).collect();
        assert_eq!(loose_after, loose_before, "nothing else was written");
    }

    /// Putting the same tags on a sheet again writes nothing (SPEC section
    /// 4.4): no record is touched, and every record says what it said. A
    /// later table is followed all the same: where the database's tag has
    /// become the file's own name, the name kept beside it goes.
    #[test]
    fn the_same_tags_put_on_again_write_nothing() {
        let mut doc = sheet_for_the_database();
        let tags = tags_for_that_sheet();
        let first = fill_database_tags(&mut doc, &tags, nothing_drawn_here);
        let said = said_by_the_references(&doc);

        let second = fill_database_tags(&mut doc, &tags, nothing_drawn_here);

        assert_eq!((first.tagged, first.differing, first.changed), (3, 1, 3));
        assert_eq!(
            second,
            DatabaseTagFill {
                tagged: 3,
                differing: 1,
                changed: 0,
            },
            "the sheet said all of it already"
        );
        assert_eq!(said_by_the_references(&doc), said);

        let later = tags_of(&[("0B1C2D3E4F5061728394A5B6C7D8E9F0", "PIA-0112101B")]);
        let third = fill_database_tags(&mut doc, &later, nothing_drawn_here);
        assert_eq!(
            third,
            DatabaseTagFill {
                tagged: 1,
                differing: 0,
                changed: 1,
            }
        );
        let after = said_by_the_references(&doc);
        assert_eq!(
            after[2],
            valve_saying(&[
                "class=PIDInstrument",
                "label=PIA-0112101B",
                "resolved=item",
                "symbol=Ball Valve",
                "graphic=614",
                "item={0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}",
                "item_type=Instrument",
                "tag_source=database",
            ]),
            "the database's tag is the file's name now: nothing is kept beside it"
        );
        assert_eq!(
            [&after[..2], &after[3..]],
            [&said[..2], &said[3..]],
            "the others are as the first table left them"
        );
    }

    /// Only a symbol of the file takes a tag from the database: a block
    /// reference that says `role=symbol` and was not drawn in this program.
    /// A reference `drawn_here` answers for, a reference that says another
    /// role and an entity that is no block reference are left as they are,
    /// whatever item they name.
    #[test]
    fn only_a_symbol_of_the_file_takes_the_databases_tag() {
        let mut doc = sheet_for_the_database();
        let valves: Vec<document::Handle> = references(&doc)
            .iter()
            .map(|valve| valve.common().handle)
            .collect();
        // The second valve says it is a fill, not a symbol.
        stamp_pid_pairs(
            doc.get_entity_mut(valves[1]).expect("the second valve"),
            &[("role", "fill")],
        );
        // A stroke says of an item what the first valve says of its own.
        let mut stroke = EntityType::Line(Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
        ));
        stamp_pid_pairs(
            &mut stroke,
            &[
                ("role", "symbol"),
                ("label", "GV-101"),
                ("item", "4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21"),
                ("tag_source", "file"),
            ],
        );
        let stroke = doc.add_entity(stroke).expect("the stroke is added");
        let says_of = |doc: &Document, handle: document::Handle| {
            says(doc.get_entity(handle).expect("the entity is in the sheet"))
        };
        let before: Vec<Vec<String>> = valves.iter().map(|valve| says_of(&doc, *valve)).collect();
        let stroke_before = says_of(&doc, stroke);
        let tags = tags_of(&[
            ("4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21", "GV-101-DB"),
            ("7A3C5E9B1D2F4A6C8E0B2D4F6A8C0E13", "BV1301102B"),
            ("0B1C2D3E4F5061728394A5B6C7D8E9F0", "PIA-old_0112101A"),
        ]);

        // The first valve was drawn in this program, the caller says.
        let first = valves[0];
        let fill = fill_database_tags(&mut doc, &tags, |entity| entity.common().handle == first);

        assert_eq!(
            fill,
            DatabaseTagFill {
                tagged: 1,
                differing: 1,
                changed: 1,
            },
            "the third valve alone"
        );
        let after: Vec<Vec<String>> = valves.iter().map(|valve| says_of(&doc, *valve)).collect();
        assert_eq!(after[0], before[0], "drawn in this program");
        assert_eq!(after[1], before[1], "no symbol by its role");
        assert_eq!(says_of(&doc, stroke), stroke_before, "no block reference");
        assert_ne!(after[2], before[2]);
        assert_eq!(&after[3..], &before[3..]);
    }

    /// On a drawing with a `_Data.xml` beside it the database's tag still
    /// wins (SPEC section 3, and the first level of section 4.2): the tag
    /// the XML published is replaced and marked `tag_source=database`, and
    /// what else the XML published -- the class, the oid, how it was
    /// resolved -- stands. No `label_in_file` is written beside it: such a
    /// record does not say what the file called the item. A tag the file
    /// itself gave on that drawing is treated as on any other.
    #[test]
    fn the_databases_tag_replaces_the_one_data_xml_published() {
        let xml = r#"<Container>
  <PIDInlineComponent><IObject UID="VALVE" ItemTag="GV-101"/></PIDInlineComponent>
  <PIDRepresentation><IObject UID="REP1"/><IDrawingRepresentation GraphicOID="612"/></PIDRepresentation>
  <Rel><IRel UID1="VALVE" UID2="REP1" DefUID="DwgRepresentationComposition"/></Rel>
  <PIDInstrument><IObject UID="GAUGE"/></PIDInstrument>
  <PIDRepresentation><IObject UID="REP2"/><IDrawingRepresentation GraphicOID="613"/></PIDRepresentation>
  <Rel><IRel UID1="GAUGE" UID2="REP2" DefUID="DwgRepresentationComposition"/></Rel>
</Container>"#;
        let mut tables = styles();
        tables.semantics = Some(PidSemanticIndex::from_xml(
            xml,
            &pid_parse::PidDocument::default(),
        ));
        let geometry = sheet(
            vec![valve_on_process(612, 0.1), valve_on_process(613, 0.2)],
            vec![valve_body()],
        );
        let items = SheetItems::stating([
            (
                612,
                plant_item(
                    "VALVE-ITEM",
                    "PipingComp",
                    Some("PIDPipingComponent"),
                    Some("GV-101 AS THE FILE HAS IT"),
                ),
            ),
            (
                613,
                plant_item(
                    "GAUGE-ITEM",
                    "Instrument",
                    Some("PIDInstrument"),
                    Some("PG-7"),
                ),
            ),
        ]);
        let (mut doc, _) = import_stating(&geometry, &tables, &items);
        let tags = tags_of(&[("VALVEITEM", "GV-101-A"), ("GAUGEITEM", "PG-7A")]);

        let fill = fill_database_tags(&mut doc, &tags, nothing_drawn_here);

        assert_eq!(
            fill,
            DatabaseTagFill {
                tagged: 2,
                differing: 1,
                changed: 2,
            }
        );
        assert_eq!(
            said_by_the_references(&doc),
            [
                valve_saying(&[
                    "class=PIDInlineComponent",
                    "label=GV-101-A",
                    "oid=612",
                    "resolved=direct",
                    "symbol=Ball Valve",
                    "graphic=612",
                    "item=VALVE-ITEM",
                    "item_type=PipingComp",
                    "tag_source=database",
                ]),
                valve_saying(&[
                    "class=PIDInstrument",
                    "label=PG-7A",
                    "oid=613",
                    "resolved=direct",
                    "symbol=Ball Valve",
                    "graphic=613",
                    "item=GAUGE-ITEM",
                    "item_type=Instrument",
                    "tag_source=database",
                    "label_in_file=PG-7",
                ]),
            ]
        );
    }

    /// Text record `oid`: a run of lettering `value` at (`x`, 0.25) metres
    /// on a displayed layer `Process`.
    fn text_on_process(oid: u32, x: f64, value: &str) -> pid_parse::PidGraphicEntity {
        record(
            oid,
            PidGraphicKind::Text {
                insertion: point(x, 0.25),
                value: value.to_string(),
                height: 0.0025,
                rotation: 0.0,
                anchor_code: 0,
                text_box_width: None,
            },
            Some((Some("Process"), true)),
        )
    }

    /// What the file hangs the labels of a sheet stored in [`STREAM`] on:
    /// its `texts`, each with the record it names as its parent; its
    /// `labels`, each label record with its first member; and its `runs` of
    /// line.
    fn labels_of(texts: &[(u32, u32)], labels: &[(u32, u32)], runs: &[u32]) -> SheetLabels {
        let mut table = SheetLabels::default();
        table.add_stream(
            STREAM,
            texts.iter().copied(),
            labels.iter().copied(),
            runs.iter().copied(),
        );
        table
    }

    /// The loose texts of `doc` in the order they were filed, each as its
    /// lettering and what its record says.
    fn texts_say(doc: &Document) -> Vec<(String, Vec<String>)> {
        loose(doc)
            .into_iter()
            .filter_map(|entity| match entity {
                EntityType::Text(text) => Some((text.value.clone(), says(entity))),
                _ => None,
            })
            .collect()
    }

    /// What a loose text on `Process` says, and `more` after it.
    fn text_says(value: &str, more: &[&str]) -> (String, Vec<String>) {
        let said = ["sheet_layer=Process", "sheet_layer_oid=12", "role=text"]
            .iter()
            .chain(more)
            .map(|pair| (*pair).to_string())
            .collect();
        (value.to_string(), said)
    }

    /// A loose text that is a label's states what its label hangs on (plan
    /// 2026-11-20-pid-symbol-identity, SPEC section 4.3): the number of the
    /// graphic and, the graphic being a symbol of the sheet, that it is a
    /// symbol. That symbol's reference states the same number as its
    /// `graphic`, which is how the two are found from each other. The texts
    /// of one label state the same, one filed before its symbol no less
    /// than one filed after it; a text that is no label's states nothing of
    /// the kind. The references and the members of their block say what
    /// they said.
    #[test]
    fn a_label_text_states_the_symbol_its_label_hangs_on() {
        let geometry = sheet(
            vec![
                text_on_process(801, 0.1, "GV-101"),
                valve_on_process(612, 0.1),
                text_on_process(802, 0.1, "DN50"),
                valve_on_process(613, 0.3),
                text_on_process(803, 0.3, "NOTE 4"),
            ],
            vec![valve_body()],
        );
        let labels = labels_of(&[(801, 900), (802, 900), (803, 6)], &[(900, 612)], &[]);
        let (doc, built) = import_labelled(&geometry, &styles(), &labels);

        assert_eq!(
            texts_say(&doc),
            [
                text_says("GV-101", &["label_of=612", "label_on=symbol"]),
                text_says("DN50", &["label_of=612", "label_on=symbol"]),
                text_says("NOTE 4", &[]),
            ]
        );
        let placed: Vec<Vec<String>> = references(&doc).into_iter().map(says).collect();
        let valve = |graphic: &str| {
            vec![
                "sheet_layer=Process".to_string(),
                "sheet_layer_oid=12".to_string(),
                "role=symbol".to_string(),
                "extent=4.00x3.00".to_string(),
                "symbol=Ball Valve".to_string(),
                format!("graphic={graphic}"),
            ]
        };
        assert_eq!(
            placed,
            [valve("612"), valve("613")],
            "a reference states its own number and nothing of its labels"
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
            ]
        );
        assert_eq!((built.symbol_labels, built.other_labels), (2, 0));
    }

    /// A label that hangs on a run of line says `line`, and one that hangs
    /// on anything else says `other`: a row of the title block's
    /// attributes, which is no graphic of the sheet, or a single line,
    /// which is no run. Either way the text states the number the file
    /// names, and what is hung on says nothing of it.
    #[test]
    fn a_label_on_a_run_of_line_says_line_and_one_on_anything_else_other() {
        let on = Some((Some("Process"), true));
        let geometry = sheet(
            vec![
                record(
                    701,
                    PidGraphicKind::Polyline {
                        points: vec![point(0.0, 0.1), point(0.2, 0.1), point(0.2, 0.2)],
                        closed: false,
                    },
                    on,
                ),
                record(
                    702,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.3),
                        end: point(0.2, 0.3),
                    },
                    on,
                ),
                text_on_process(811, 0.05, "P-1001-50-CS"),
                text_on_process(812, 0.1, "DWG-0201"),
                text_on_process(813, 0.15, "SEE NOTE 2"),
            ],
            Vec::new(),
        );
        let labels = labels_of(
            &[(811, 910), (812, 911), (813, 912)],
            &[(910, 701), (911, 24_578), (912, 702)],
            &[701],
        );
        let (doc, built) = import_labelled(&geometry, &styles(), &labels);

        assert_eq!(
            texts_say(&doc),
            [
                text_says("P-1001-50-CS", &["label_of=701", "label_on=line"]),
                text_says("DWG-0201", &["label_of=24578", "label_on=other"]),
                text_says("SEE NOTE 2", &["label_of=702", "label_on=other"]),
            ]
        );
        let line_work: Vec<Vec<String>> = loose(&doc)
            .into_iter()
            .filter(|entity| !matches!(entity, EntityType::Text(_)))
            .map(says)
            .collect();
        let geometry_on_process = vec![
            "sheet_layer=Process".to_string(),
            "sheet_layer_oid=12".to_string(),
            "role=geometry".to_string(),
        ];
        assert_eq!(
            line_work,
            [geometry_on_process.clone(), geometry_on_process],
            "the run and the line say what they said"
        );
        assert_eq!((built.symbol_labels, built.other_labels), (0, 3));
    }

    /// A label hangs on a symbol only where that symbol was filed as a
    /// reference: `label_on=symbol` promises a reference whose `graphic` is
    /// the text's `label_of`. A placement that draws nothing -- a label
    /// symbol whose body is only its anchor cross and whose file has no
    /// name to letter -- is no reference, and a text hung on it says
    /// `other`. So does a text hung on the number of a placement that lies
    /// in another stream: a record's number is its stream's own.
    #[test]
    fn a_label_hangs_on_a_symbol_only_where_the_symbol_was_filed_as_a_reference() {
        const NAMELESS: &str = r"\\SRV\Ref\Symbols\Design\Annotation\Labels\.sym";
        let mut elsewhere = valve_on_process(630, 0.5);
        elsewhere.source.stream_path = Some("/Sheet7".to_string());
        let geometry = sheet(
            vec![
                record(
                    620,
                    placement((0.3, 0.3), Some(NAMELESS), Some(CROSS), 0.0),
                    Some((Some("Process"), true)),
                ),
                text_on_process(821, 0.3, "ON WHAT DRAWS NOTHING"),
                elsewhere,
                text_on_process(822, 0.5, "ON A NUMBER OF ANOTHER STREAM"),
                valve_on_process(612, 0.1),
                text_on_process(823, 0.1, "GV-101"),
            ],
            vec![valve_body(), cross_body()],
        );
        let labels = labels_of(
            &[(821, 920), (822, 921), (823, 922)],
            &[(920, 620), (921, 630), (922, 612)],
            &[],
        );
        let (doc, built) = import_labelled(&geometry, &styles(), &labels);

        let graphics: Vec<Option<&str>> = references(&doc)
            .into_iter()
            .map(|reference| pid_value(reference, "graphic"))
            .collect();
        assert_eq!(
            graphics,
            [Some("630"), Some("612")],
            "the placement that draws nothing is no reference"
        );
        assert_eq!(
            texts_say(&doc),
            [
                text_says("ON WHAT DRAWS NOTHING", &["label_of=620", "label_on=other"]),
                text_says(
                    "ON A NUMBER OF ANOTHER STREAM",
                    &["label_of=630", "label_on=other"]
                ),
                text_says("GV-101", &["label_of=612", "label_on=symbol"]),
            ]
        );
        assert_eq!((built.symbol_labels, built.other_labels), (1, 2));
    }

    /// Only a text record is a label's text. A record of another kind
    /// states no pairing even where the table has a text of its number: a
    /// line stays a line that says what it said, and a placement's
    /// reference states its own graphic and no more.
    #[test]
    fn a_record_that_is_no_text_states_no_pairing() {
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                record(
                    702,
                    PidGraphicKind::Line {
                        start: point(0.0, 0.3),
                        end: point(0.2, 0.3),
                    },
                    Some((Some("Process"), true)),
                ),
            ],
            vec![valve_body()],
        );
        let labels = labels_of(&[(702, 900), (612, 900)], &[(900, 612)], &[]);
        let (doc, built) = import_labelled(&geometry, &styles(), &labels);

        let paired: Vec<Vec<String>> = doc
            .model_space_entities()
            .map(says)
            .filter(|said| said.iter().any(|pair| pair.starts_with("label_o")))
            .collect();
        assert!(
            paired.is_empty(),
            "records that state a pairing: {paired:?}"
        );
        assert_eq!((built.symbol_labels, built.other_labels), (0, 0));
    }

    /// The file pairs text records, and the import states the pairing on
    /// what each record draws. A label that reads on two lines is two text
    /// entities, and each states it; a record whose lettering is blank
    /// draws nothing, so nothing states it. The counts are of text
    /// entities.
    #[test]
    fn every_line_a_label_draws_states_the_pairing_and_a_blank_one_draws_nothing() {
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                text_on_process(831, 0.1, "PT\r0112101"),
                text_on_process(832, 0.1, "  "),
                text_on_process(833, 0.1, ""),
            ],
            vec![valve_body()],
        );
        let labels = labels_of(&[(831, 930), (832, 930), (833, 930)], &[(930, 612)], &[]);
        let (doc, built) = import_labelled(&geometry, &styles(), &labels);

        assert_eq!(
            texts_say(&doc),
            [
                text_says("PT", &["label_of=612", "label_on=symbol"]),
                text_says("0112101", &["label_of=612", "label_on=symbol"]),
            ],
            "two lines of one record, and nothing of the two blank ones"
        );
        assert_eq!((built.symbol_labels, built.other_labels), (2, 0));
    }

    /// What a published `_Data.xml` says of a label's text stands where it
    /// stood, and the pairing follows it: the two keys come after every key
    /// a record had before them.
    #[test]
    fn the_pairing_follows_what_data_xml_published_for_the_text() {
        let xml = r#"<Container>
  <PIDInlineComponent><IObject UID="VALVE" ItemTag="GV-101"/></PIDInlineComponent>
  <PIDRepresentation><IObject UID="REP1"/><IDrawingRepresentation GraphicOID="801"/></PIDRepresentation>
  <Rel><IRel UID1="VALVE" UID2="REP1" DefUID="DwgRepresentationComposition"/></Rel>
</Container>"#;
        let mut tables = styles();
        tables.semantics = Some(PidSemanticIndex::from_xml(
            xml,
            &pid_parse::PidDocument::default(),
        ));
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                text_on_process(801, 0.1, "GV-101"),
            ],
            vec![valve_body()],
        );
        let labels = labels_of(&[(801, 900)], &[(900, 612)], &[]);
        let (doc, _) = import_labelled(&geometry, &tables, &labels);

        assert_eq!(
            texts_say(&doc),
            [text_says(
                "GV-101",
                &[
                    "class=PIDInlineComponent",
                    "label=GV-101",
                    "oid=801",
                    "resolved=direct",
                    "label_of=612",
                    "label_on=symbol",
                ]
            )]
        );
    }

    /// The two keys of the pairing have their place in a record's order:
    /// after the keys a symbol's identity brings and before any key the
    /// order does not know, `label_of` ahead of `label_on` whichever is
    /// written first.
    #[test]
    fn the_keys_of_the_pairing_have_their_place_in_the_record() {
        let mut text = EntityType::Line(Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
        ));
        stamp_pid_pairs(&mut text, &[("role", "text"), ("made_up", "1")]);
        stamp_pid_pairs(&mut text, &[("label_on", "symbol")]);
        stamp_pid_pairs(&mut text, &[("label_of", "612"), ("label_in_file", "X")]);
        assert_eq!(
            says(&text),
            [
                "role=text",
                "label_in_file=X",
                "label_of=612",
                "label_on=symbol",
                "made_up=1",
            ]
        );

        // Stated again, the pairing replaces what was stated: a record
        // holds it once.
        attach_label_owner(&mut text, 701, LabelOn::Line);
        assert_eq!(
            says(&text),
            [
                "role=text",
                "label_in_file=X",
                "label_of=701",
                "label_on=line",
                "made_up=1",
            ]
        );
        assert_eq!(
            [
                LabelOn::Symbol.as_str(),
                LabelOn::Line.as_str(),
                LabelOn::Other.as_str(),
            ],
            ["symbol", "line", "other"]
        );
    }

    /// The summary counts the label texts by what they hang on, after the
    /// counts it kept before: those on a symbol of the sheet, and those on
    /// a line or on anything else together. A text that is no label's is in
    /// neither. Both ride the document under their own keys and read back.
    #[test]
    fn the_summary_counts_the_label_texts_by_what_they_hang_on() {
        let geometry = sheet(
            vec![
                valve_on_process(612, 0.1),
                record(
                    701,
                    PidGraphicKind::Polyline {
                        points: vec![point(0.0, 0.1), point(0.2, 0.1)],
                        closed: false,
                    },
                    Some((Some("Process"), true)),
                ),
                text_on_process(841, 0.1, "GV-101"),
                text_on_process(842, 0.1, "DN50"),
                text_on_process(843, 0.1, "FO"),
                text_on_process(844, 0.2, "P-1001-50-CS"),
                text_on_process(845, 0.3, "DWG-0201"),
                text_on_process(846, 0.4, "NOTE 4"),
            ],
            vec![valve_body()],
        );
        let labels = labels_of(
            &[(841, 940), (842, 940), (843, 941), (844, 942), (845, 943)],
            &[(940, 612), (941, 612), (942, 701), (943, 24_578)],
            &[701],
        );
        let tables = styles();
        let (mut doc, built) = import_labelled(&geometry, &tables, &labels);
        let summary = finish(
            Path::new("labels.pid"),
            &geometry,
            None,
            &tables,
            ImportUnit::AssumedMetre,
            built,
            &LinkedTemplate::Unnamed,
            FramePlacement::NONE,
            &mut doc,
        );

        assert_eq!(
            summary.counts()[18..],
            [
                ("symbol_blocks", 1),
                ("symbol_definitions", 1),
                ("symbol_items", 0),
                ("symbol_tags", 0),
                ("symbol_labels", 3),
                ("other_labels", 2),
            ],
            "six texts: three on the valve, one on the run, one on the title block, one no label's"
        );
        assert_eq!((summary.symbol_labels, summary.other_labels), (3, 2));

        summary.store(&mut doc);
        let stored = |key: &str| {
            let tag = format!("{SUMMARY_PROPERTY_PREFIX}{key}");
            doc.summary_info
                .custom_properties
                .iter()
                .find(|(name, _)| *name == tag)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            (
                stored("symbol_labels").as_deref(),
                stored("other_labels").as_deref()
            ),
            (Some("3"), Some("2")),
            "each rides the document under its own key"
        );
        assert_eq!(ImportSummary::take(&mut doc), Some(summary));
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

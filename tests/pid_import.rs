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
    // The 22 blue entities are the eleven class-coloured points twice over:
    // the point record itself and the slash mark drawn on it -- see
    // `a_class_coloured_point_is_marked_with_the_slash_smartplant_shows`.
    let expected: std::collections::BTreeMap<String, usize> = [
        (" 10 #000000", 53),
        (" 10 #0000FF", 22),
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

/// Those widths are switched on for display, not only recorded.
///
/// The wire shader reads `$LWDISPLAY` from the header and collapses every
/// line to a hairline while it is off, and a `.pid` has no header of its own
/// to turn it on -- a fresh `CadDocument` ships it off. So the width test
/// above passed on a sheet that drew every line identically, which is the
/// state reading the style table was meant to end. The importer states the
/// flag itself; this pins that, because nothing in an entity count or a
/// symbology assertion can see it.
#[test]
fn the_widths_the_drawing_states_are_switched_on_for_display() {
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
            doc.header.lineweight_display,
            "{name}: LWDISPLAY must open on, or the decoded widths draw as hairlines"
        );
    }
}

/// A `.pid` is never a write target: the deepest save routine refuses the
/// destination before anything touches the disk.
///
/// Everything above it — QSAVE on a `.pid` tab, the save-before-close prompt —
/// reroutes to Save As first (`direct_save_path`), so this gate is the safety
/// net for whatever slips past the UI: a Save As where the user types `.pid`
/// back in, WBLOCK to a typed name, automation. Needs no fixture: the gate
/// must hold for any document aimed at any `.pid` path.
#[test]
fn saving_refuses_a_pid_destination() {
    let doc = acadrust::CadDocument::new();
    for case in ["pid", "PID"] {
        let target =
            std::env::temp_dir().join(format!("ocs-save-gate-{}.{case}", std::process::id()));
        let error =
            OpenCADStudio::io::save(&doc, &target).expect_err("a .pid destination must be refused");
        assert!(
            error.contains("read-only"),
            "the refusal should say why: {error}"
        );
        assert!(
            !target.exists(),
            "nothing may be written at the refused destination"
        );
    }
}

/// The import leaves its command-line headline behind, keyed by path.
///
/// The open-completion handler takes it and shows the reader one line: how
/// much of the file became drawing, how much did not, and whether the style
/// tables read. A headline that disagreed with the document would be worse
/// than none, so the numbers are checked against the import itself rather
/// than pinned: everything the summary counts as drawn went through the
/// entity loop, and the only entity added outside it is the sheet border.
#[test]
fn the_import_leaves_a_summary_the_app_can_show() {
    let Some(path) = fixture("DWG-0201GP06-01.pid") else {
        return;
    };
    let doc = OpenCADStudio::io::load_file(&path).expect("the fixture imports");
    let summary = OpenCADStudio::io::pid::take_import_summary(&path)
        .expect("an import leaves its summary behind");
    assert_eq!(
        summary.drawn + 1,
        doc.entities().count(),
        "drawn plus the page border is everything in the document"
    );
    assert!(summary.decoded > 0, "the fixture has decoded records");
    assert!(
        !summary.style_tables_failed,
        "the fixture's style tables read; the palette test depends on it"
    );
    assert!(
        OpenCADStudio::io::pid::take_import_summary(&path).is_none(),
        "taking the summary drains it"
    );
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
    //
    // The table grew twice as `pid-parse` learned the `igTextBox` family.
    // Retiring its fixed-68 overhead took the 1/8 inch bucket from 21 to 28;
    // then reading the record's three native sub-types took it to 30 and put
    // two sizes here that this drawing had never shown -- 6.350 is a quarter
    // inch, the heading size. All of it is lettering that was simply missing
    // from the sheet before. See that crate's
    // `docs/analysis/2026-08-12-igtextbox-overhead-is-a-floor-not-a-constant.md`
    // and `docs/analysis/2026-08-13-igtextbox-has-three-shapes.md`.
    let expected: std::collections::BTreeMap<String, usize> = [
        ("1.500", 2),
        ("1.524", 1),
        ("2.464", 3),
        ("2.500", 9),
        ("3.175", 30),
        ("3.500", 2),
        ("6.350", 1),
    ]
    .iter()
    .map(|(key, count)| ((*key).to_string(), *count))
    .collect();
    assert_eq!(heights, expected);
}

/// Rotated lettering arrives in the unit the drawing model uses: radians.
///
/// The importer used to call `to_degrees()` on its way in, which was
/// invisible while `pid-parse` hard-coded text rotation to zero -- and became
/// a quarter turn rendered at 116 degrees the moment real rotations decoded.
/// The model's contract is radians throughout: `io::fix_dxf_dimension_rotations`
/// exists to convert the DXF reader's degrees on load, and `entities::text`
/// adds `PI` for upside-down text. This pins the vertical labels to the value
/// that contract asks for; under the old conversion they read 90.0 and fail.
#[test]
fn rotated_lettering_is_stored_in_radians() {
    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };

    let mut buckets: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entity in on_layer(&doc, "PID-TEXT") {
        if let EntityType::Text(text) = entity {
            // Degrees would land on 0 / 90 / 180; radians land on 0 / 1.571 /
            // 3.142. Rounding to three places keeps float noise out.
            buckets
                .entry(format!("{:.3}", text.rotation))
                .and_modify(|n| *n += 1)
                .or_insert(1);
        }
    }

    let quarter = format!("{:.3}", std::f64::consts::FRAC_PI_2);
    let half = format!("{:.3}", std::f64::consts::PI);
    assert_eq!(
        buckets.get(&quarter).copied().unwrap_or(0),
        13,
        "this sheet letters 13 labels up its vertical pipe runs, got {buckets:?}"
    );
    assert_eq!(
        buckets.get(&half).copied().unwrap_or(0),
        1,
        "and one upside down, got {buckets:?}"
    );
    assert!(
        !buckets.contains_key("90.000") && !buckets.contains_key("180.000"),
        "a degree value here means the importer converted on the way in: {buckets:?}"
    );
}

/// A label with a second line comes in as a second line.
///
/// Four `igTextBox` records in the reference corpus carry a `U+000D` in their
/// text — a title block and a note, each published twice. A DXF TEXT entity is
/// single-line, so until this they imported as one run holding a code point no
/// stroke font draws: the title block's four lines lettered end to end across
/// the sheet with a blank gap between them.
///
/// The pitch is not this importer's invention. `JStyleTextPara +66` states a
/// line spacing multiple, and the measurement that justified reading it is
/// that the two labels here state `1.5` while all 228 single-line labels in
/// the corpus state `1.0` — the field varies exactly where the line breaks do.
/// See pid-parse's `docs/analysis/2026-08-22-four-labels-have-a-second-line.md`.
///
/// This pins both halves: that no break survives into an entity, and that the
/// lines land one stated pitch apart rather than at a spacing of ours.
///
/// **The pitch is `height × spacing`, and the height arrives over pid-parse's
/// two-hop join** (`igTextBox` → `JStyleTextPara` → `JStyleTextChar`). A
/// shape-2 record also carries a run naming a different character style, and
/// where the two disagree the run's height is the non-nominal one —
/// unsettled as of 2026-08-22. The multiple is safe from that (spacing is a
/// paragraph property), but if the run ever wins for height, this assertion's
/// expected value moves with it and has to be re-measured rather than
/// re-derived. The test reads the height off the entity for that reason: only
/// the multiple is written down here.
#[test]
fn a_multi_line_label_stacks_at_the_spacing_its_paragraph_states() {
    /// The title block, in the order the record spells it.
    const TITLE: [&str; 4] = ["安3集气站", "排污单元", "污油池", "管道及仪表流程图"];
    /// What `+66` reads on both of this drawing's multi-line labels.
    const STATED_SPACING: f64 = 1.5;

    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };

    let line_of = |value: &str| -> &acadrust::entities::Text {
        on_layer(&doc, "PID-TEXT")
            .find_map(|entity| match entity {
                EntityType::Text(text) if text.value == value => Some(text),
                _ => None,
            })
            .unwrap_or_else(|| panic!("`{value}` did not reach PID-TEXT as a line of its own"))
    };

    let lines: Vec<&acadrust::entities::Text> = TITLE.iter().map(|value| line_of(value)).collect();
    let height = lines[0].height;
    assert!(height > 0.0, "the title block letters at a real height");
    let expected = height * STATED_SPACING;
    for pair in lines.windows(2) {
        let (from, to) = (pair[0].insertion_point, pair[1].insertion_point);
        // Measured as a distance rather than a drop in Y, because the offset
        // follows the baseline's normal and this label need not be horizontal.
        let step = (to.x - from.x).hypot(to.y - from.y);
        assert!(
            (step - expected).abs() < 1e-6,
            "`{}` to `{}` is {step}mm, not the stated {expected}mm",
            pair[0].value,
            pair[1].value
        );
        assert!(
            (pair[1].height - height).abs() < 1e-9,
            "every line of one label letters at one height"
        );
    }

    // The two-line note splits too, so this is not a special case for titles.
    let note = line_of("2、阻火器电伴热带从根部缠至地面以上2m。");
    assert!(note.height > 0.0);
}

/// No label reaches an entity still holding a line break.
///
/// A break that survives is not a visible error — a stroke font has no glyph
/// for `U+000D`, so it renders as a gap and the label simply reads wrong. This
/// is the assertion that makes that loud, across every fixture rather than
/// only the one that has multi-line labels today.
#[test]
fn no_label_keeps_a_line_break_it_cannot_draw() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        for entity in on_layer(&doc, "PID-TEXT") {
            let EntityType::Text(text) = entity else {
                continue;
            };
            assert!(
                !text.value.chars().any(char::is_control),
                "{name}: `{}` reached the drawing with a control character in it",
                text.value.escape_debug()
            );
        }
    }
}

/// Lettering comes in the colour the drawing's character style states.
///
/// It used to be the `PID-TEXT` layer's green for every label, an invention of
/// this importer: nothing read a text colour, so nothing could state one. The
/// character style does, at `JStyleTextChar +34`, and the same two-hop join
/// that already fetched the height carries it. Nearly all of a P&ID letters in
/// black, which the renderer flips to white on the dark background exactly as
/// it does the drawing's black line work -- so the visible change is that
/// lettering stops being green, plus the handful of labels the drawing colours
/// deliberately. A regression puts them back on `ByLayer`, which no entity
/// count would show.
#[test]
fn lettering_carries_the_colour_the_drawing_states() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut palette: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entity in on_layer(&doc, "PID-TEXT") {
        let key = match entity.common().color {
            acadrust::types::Color::Rgb { r, g, b } => format!("#{r:02X}{g:02X}{b:02X}"),
            _ => "ByLayer".to_string(),
        };
        *palette.entry(key).or_default() += 1;
    }

    // 47 of this sheet's 48 labels reach a character style and letter in the
    // black it states. The one left on `ByLayer` is the same record that
    // keeps the 2.5mm height fallback -- its style states the unexplained
    // 0.254mm that `style_link` refuses, so the whole resolution returns
    // nothing and neither the height nor the colour is applied. Colour and
    // height fall back together because they ride one join.
    let expected: std::collections::BTreeMap<String, usize> = [("#000000", 47), ("ByLayer", 1)]
        .iter()
        .map(|(key, count)| ((*key).to_string(), *count))
        .collect();
    assert_eq!(palette, expected);
}

/// Lettering starts from the side the drawing's paragraph style states, and
/// carries the alignment point that makes it mean anything.
///
/// Every label used to render left-aligned because nothing read an alignment.
/// `JStyleTextPara +35` states one, with Intergraph's own values, and across
/// the corpus 49% of the styles text reaches are centred or right -- each of
/// those runs sitting half a label from where the drawing puts it.
///
/// The second assertion is the one that matters more. A TEXT entity's
/// insertion point is the run origin *only* while the alignment is
/// left-on-baseline; otherwise the origin is `alignment_point`. Setting the
/// alignment without seeding that point does not nudge a label, it drops it at
/// the origin -- so this pins that every non-left label has one, and that the
/// left ones do not (a stray point there would be equally wrong).
#[test]
fn lettering_starts_from_the_side_the_drawing_states() {
    use acadrust::entities::TextHorizontalAlignment as HA;

    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut sides: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut misplaced = Vec::new();
    for entity in on_layer(&doc, "PID-TEXT") {
        let acadrust::EntityType::Text(text) = entity else {
            continue;
        };
        let key = match text.horizontal_alignment {
            HA::Left => "left",
            HA::Center => "center",
            HA::Right => "right",
            other => {
                misplaced.push(format!("unexpected alignment {other:?}"));
                continue;
            }
        };
        *sides.entry(key.to_string()).or_default() += 1;
        let needs_point = text.horizontal_alignment != HA::Left;
        if needs_point != text.alignment_point.is_some() {
            misplaced.push(format!(
                "{:?} label {:?} has alignment_point {:?}",
                text.horizontal_alignment, text.value, text.alignment_point
            ));
        }
    }

    assert!(
        misplaced.is_empty(),
        "every non-left label needs an alignment point and no left one may carry one: {misplaced:?}"
    );
    // This sheet letters 20 of its 48 labels from somewhere other than the
    // left -- every one of which used to render half a label off.
    //
    // `left` is the one bucket that cannot be read as a measurement: a label
    // whose style resolution fails states no alignment, keeps the entity
    // default, and lands here indistinguishable from a stated left. This
    // sheet has exactly one such record -- the same one that keeps the height
    // and colour fallbacks, since all three ride one join -- so 28 means 27
    // stated plus 1 defaulted.
    let expected: std::collections::BTreeMap<String, usize> =
        [("center", 18), ("left", 28), ("right", 2)]
            .iter()
            .map(|(key, count)| ((*key).to_string(), *count))
            .collect();
    assert_eq!(sides, expected);
}

/// Lettering names the typeface the drawing's character style states.
///
/// Every label used to render in the application's default face, because
/// nothing read a font name. `JStyleTextChar` ends with one -- `+68` count,
/// `+70` UTF-16 body -- and the importer pools the distinct ones into document
/// text styles so a label references a style the way any other text entity in
/// the application does.
///
/// The `height == 0` assertion is the one worth keeping. A `TextStyle` height
/// is a *fixed* height that overrides the entity's, so a non-zero value here
/// would silently undo the per-entity heights that
/// `lettering_carries_the_height_the_drawing_states` pins -- and that test
/// would keep passing, because it reads the entity rather than the style.
#[test]
fn lettering_names_the_typeface_the_drawing_states() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let registered: std::collections::BTreeMap<String, String> = doc
        .text_styles
        .iter()
        .filter(|style| style.name.starts_with("PID-"))
        .map(|style| {
            assert_eq!(
                style.height, 0.0,
                "{} states a fixed height, which would override every entity's own",
                style.name
            );
            (style.name.clone(), style.true_type_font.clone())
        })
        .collect();
    // Five typefaces on this sheet. The style name is sanitised for the symbol
    // table -- the space in "Arial Narrow" becomes a hyphen -- while the
    // typeface itself travels verbatim in `true_type_font`, which is what the
    // renderer matches against the installed fonts.
    let expected_styles: std::collections::BTreeMap<String, String> = [
        ("PID-Arial", "Arial"),
        ("PID-Arial-Narrow", "Arial Narrow"),
        ("PID-SimSun-ExtB", "SimSun-ExtB"),
        ("PID-仿宋", "仿宋"),
        ("PID-宋体", "宋体"),
    ]
    .iter()
    .map(|(name, font)| ((*name).to_string(), (*font).to_string()))
    .collect();
    assert_eq!(registered, expected_styles);

    let mut used: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entity in on_layer(&doc, "PID-TEXT") {
        if let EntityType::Text(text) = entity {
            *used.entry(text.style.clone()).or_default() += 1;
        }
    }
    // Three of the five are referenced. The other two are reached only by
    // records whose lettering lands on `PID-SYMBOL-LABEL`, which this importer
    // deliberately leaves alone -- the same scope every other text property
    // respects -- so their styles are registered and unused, the way a DWG
    // carries any style nothing currently names. `Standard` is the one label
    // whose style resolution fails outright, keeping the fallback height,
    // colour and alignment along with the default face.
    let expected_used: std::collections::BTreeMap<String, usize> = [
        ("PID-Arial", 21),
        ("PID-Arial-Narrow", 8),
        ("PID-宋体", 18),
        ("Standard", 1),
    ]
    .iter()
    .map(|(name, count)| ((*name).to_string(), *count))
    .collect();
    assert_eq!(used, expected_used);

    // The scope itself, pinned: lettering that is not the sheet's own keeps
    // the document default. A symbol's text is placed by the `.sym` library.
    let strayed: Vec<&str> = ["PID-SYMBOL-LABEL", "PID-SYMBOL"]
        .iter()
        .flat_map(|layer| on_layer(&doc, layer))
        .filter_map(|entity| match entity {
            EntityType::Text(text) if text.style != "Standard" => Some(text.style.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        strayed.is_empty(),
        "the importer restyled lettering it does not own: {strayed:?}"
    );
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

/// An area the drawing fills comes in filled, and in the colour it states.
///
/// `igBoundary2d` used to emit nothing, on the grounds that its segments
/// re-list the member `igLine2d` records that already draw the outline. True,
/// and it left the drawing's flow arrowheads hollow: every one of the corpus's
/// boundaries resolves through a `JStyleOverride` to a `JStyleSimpleFill`, and
/// the member lines have no way to say so. They now import as solid hatches --
/// five on DWG-0202, ten on the gongyi drawing -- in the blue `#0000FF` the
/// fill states at payload +30, decoded like a line's `COLORREF`. Measured in
/// `pid-parse`'s `docs/analysis/2026-08-10-fill-colour-is-002a-plus-30.md`.
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
            // The arrowheads state blue, and the decode carries it onto the
            // hatch rather than leaving it the layer's white default.
            assert_eq!(
                hatch.common.color,
                acadrust::types::Color::Rgb { r: 0, g: 0, b: 255 },
                "{name}: the decoded fill colour is the blue the drawing states"
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

/// The rectangle an entity covers, or `None` for a kind with no extent worth
/// stating here. Written out again rather than shared with the importer, so a
/// change to the importer's own idea of an extent cannot quietly move the
/// yardstick these tests measure against.
fn drawn_box(entity: &EntityType) -> Option<(f64, f64, f64, f64)> {
    let around = |x: f64, y: f64, r: f64| (x - r, y - r, x + r, y + r);
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
            .map(|v| (v.location.x, v.location.y, v.location.x, v.location.y))
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
        EntityType::Text(text) => Some((
            text.insertion_point.x,
            text.insertion_point.y,
            text.insertion_point.x,
            text.insertion_point.y,
        )),
        EntityType::Point(point) => Some((
            point.location.x,
            point.location.y,
            point.location.x,
            point.location.y,
        )),
        _ => None,
    }
}

fn distance_to_box(point: (f64, f64), area: (f64, f64, f64, f64)) -> f64 {
    let dx = (area.0 - point.0).max(point.0 - area.2).max(0.0);
    let dy = (area.1 - point.1).max(point.1 - area.3).max(0.0);
    dx.hypot(dy)
}

/// How far a symbol's name may sit from the nearest single stroke of drawing.
///
/// The importer puts the name 0.8mm clear of the right edge of the whole
/// body, level with the body's middle, so on a compact symbol it lands
/// against the line work. On a wide sparse one it need not: `Wastewater Pit`
/// is 114mm across and the stroke that reaches furthest right is a short stub
/// well above the middle, which measures 13.5mm -- the worst case across the
/// four fixtures. The bound is set clear of that and still an order of
/// magnitude under the 148mm the defect this guards produced.
const LABEL_REACH_MM: f64 = 20.0;

/// A symbol's name is lettered beside the symbol, not beside its anchor.
///
/// `PID-SYMBOL-LABEL` names each placement after its `.sym`. The name used to
/// be hung 2.3mm right of the placement's insertion point, which is only
/// beside the symbol when the library body is drawn around its own origin --
/// and 211 of the reference library's 613 readable `.sym` are not, `Design`
/// and `Equipment` almost entirely so. On DWG-0202 that put six
/// `ElecTraceLine` names and four `Item Note & Label` names 100 to 200mm from
/// their own line work, five of them off the left edge of the sheet and one
/// below the bottom of it, while every one of those symbols drew inside the
/// border. Anchoring on the drawn body instead is what this pins; the marker
/// fallback is unaffected, since a 1.5mm dot centred on the insertion point
/// reaches exactly as far right as the old formula assumed.
///
/// Measured in `docs/analysis/2026-08-24-two-texts-outside-the-frame.md`.
#[test]
fn a_symbol_name_is_lettered_beside_the_symbol_it_names() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        let bodies: Vec<(f64, f64, f64, f64)> =
            on_layer(&doc, "PID-SYMBOL").filter_map(drawn_box).collect();
        let labels: Vec<&EntityType> = on_layer(&doc, "PID-SYMBOL-LABEL").collect();
        assert!(
            !labels.is_empty() && !bodies.is_empty(),
            "{name}: every placement is named and drawn, got {} names over {} bodies",
            labels.len(),
            bodies.len()
        );

        let mut stranded: Vec<String> = Vec::new();
        for label in labels {
            let EntityType::Text(text) = label else {
                panic!("{name}: PID-SYMBOL-LABEL carries lettering only, found {label:?}");
            };
            let at = (text.insertion_point.x, text.insertion_point.y);
            let reach = bodies
                .iter()
                .map(|body| distance_to_box(at, *body))
                .fold(f64::MAX, f64::min);
            if reach > LABEL_REACH_MM {
                stranded.push(format!(
                    "{:?} at ({:.1}, {:.1}) is {reach:.1}mm from the nearest symbol",
                    text.value, at.0, at.1
                ));
            }
        }
        assert!(
            stranded.is_empty(),
            "{name}: {} symbol names were lettered away from their symbol: {stranded:#?}",
            stranded.len()
        );
    }
}

/// Every straight run an entity draws, as endpoint pairs.
fn segments_of(entity: &EntityType) -> Vec<((f64, f64), (f64, f64))> {
    match entity {
        EntityType::Line(line) => vec![((line.start.x, line.start.y), (line.end.x, line.end.y))],
        EntityType::LwPolyline(polyline) => polyline
            .vertices
            .windows(2)
            .map(|pair| {
                (
                    (pair[0].location.x, pair[0].location.y),
                    (pair[1].location.x, pair[1].location.y),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn distance_to_segment(point: (f64, f64), segment: ((f64, f64), (f64, f64))) -> f64 {
    let ((x1, y1), (x2, y2)) = segment;
    let (dx, dy) = (x2 - x1, y2 - y1);
    let length_squared = dx * dx + dy * dy;
    let along = if length_squared == 0.0 {
        0.0
    } else {
        (((point.0 - x1) * dx + (point.1 - y1) * dy) / length_squared).clamp(0.0, 1.0)
    };
    (point.0 - (x1 + along * dx)).hypot(point.1 - (y1 + along * dy))
}

/// How far a trace symbol's name may sit from the pipe run it marks.
///
/// The worst of DWG-0202's six placements measures 2.55mm. The reading this
/// guards against puts the same six 31 to 74mm out, so the bound sits clear
/// of the first by more than double and under the second by five times.
const TRACE_REACH_MM: f64 = 6.0;

/// A symbol authored away from its own origin still lands on the line work it
/// marks.
///
/// 211 of the reference library's 613 readable `.sym` draw their body 100 to
/// 200mm from the file's own origin -- `ElecTraceLine` puts its ten strokes at
/// (103.2..107.7, 154.1..155.6)mm. That left a question the analysis note
/// carried unresolved for two rounds: does a placement record's insertion
/// point mean *put the body here*, in which case some origin has to be
/// subtracted and all 211 of those symbols are being drawn a hand's width off,
/// or does it mean *add the library coordinates to this*, which is what the
/// importer does.
///
/// Electric trace runs along a pipe, so its symbol has to sit on one, and
/// `PID-GEOMETRY` carries that pipe from the drawing's own records with no
/// help from the symbol library. Under what the importer does, all six of
/// DWG-0202's placements land on it. Subtract an origin instead and the same
/// measurement reads 31 to 74mm, with two of the six off the left edge of the
/// sheet -- so this is the assertion that goes red first if anyone starts.
///
/// The name is measured rather than the strokes because the document does not
/// say which stroke came from which `.sym`; the name is anchored on the drawn
/// body's bounding box, which
/// `a_symbol_name_is_lettered_beside_the_symbol_it_names` pins separately.
///
/// Measured in `docs/analysis/2026-08-24-two-texts-outside-the-frame.md`.
#[test]
fn a_symbol_authored_away_from_its_origin_lands_on_the_line_work_it_marks() {
    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };

    let drawing: Vec<((f64, f64), (f64, f64))> = on_layer(&doc, "PID-GEOMETRY")
        .flat_map(segments_of)
        .collect();
    assert!(
        !drawing.is_empty(),
        "the drawing's own line work is what this measures against"
    );

    let mut reach: Vec<(f64, f64, f64)> = Vec::new();
    for entity in on_layer(&doc, "PID-SYMBOL-LABEL") {
        let EntityType::Text(text) = entity else {
            continue;
        };
        if text.value != "ElecTraceLine" {
            continue;
        }
        let at = (text.insertion_point.x, text.insertion_point.y);
        let nearest = drawing
            .iter()
            .map(|segment| distance_to_segment(at, *segment))
            .fold(f64::MAX, f64::min);
        reach.push((at.0, at.1, nearest));
    }

    assert_eq!(
        reach.len(),
        6,
        "DWG-0202 places six ElecTraceLine symbols, found {reach:#?}"
    );
    let adrift: Vec<&(f64, f64, f64)> = reach
        .iter()
        .filter(|(_, _, nearest)| *nearest > TRACE_REACH_MM)
        .collect();
    assert!(
        adrift.is_empty(),
        "trace symbols marking no pipe: {adrift:#?} (all six of {reach:#?})"
    );
}

/// A symbol body draws in the one style its placement names, which is the
/// drawing's item-class colouring — not in the styles its `.sym` states
/// stroke by stroke, and not in the layer default.
///
/// The placement record names a style after all: `igSymbol2d +25`, resolved
/// in the root document's `StyleCluster`. On DWG-0201 the twenty placements
/// resolve to four class colours — equipment `#800000`, piping `#808000`,
/// instruments `#008000`, annotation black — and a SmartPlant screenshot of
/// this drawing shows exactly that: the vessel maroon though its `.sym` is
/// authored black, `Off-Unit` olive though its `.sym` authors cyan strokes.
/// The `.sym`'s own symbology remains the fallback coat for a placement whose
/// style does not resolve (none on this corpus), and the internal lettering
/// keeps its text styles either way.
#[test]
fn a_symbol_body_draws_in_the_style_its_placement_names() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut palette: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut lettering = 0usize;
    for entity in on_layer(&doc, "PID-SYMBOL") {
        let common = entity.common();
        match (common.color, common.line_weight) {
            (acadrust::types::Color::Rgb { r, g, b }, acadrust::types::LineWeight::Value(w)) => {
                *palette
                    .entry(format!("{w:>3} #{r:02X}{g:02X}{b:02X}"))
                    .or_default() += 1;
            }
            _ => lettering += 1,
        }
    }

    // The three internal texts take the placement's colour but no line
    // weight -- a weight means nothing on lettering -- so they sit outside
    // the (colour, weight) palette; their colours are pinned by
    // `a_symbols_lettering_follows_its_placement_colour_not_its_syms`.
    assert_eq!(
        lettering, 3,
        "expected exactly the three internal texts outside the line-work \
         palette, got palette {palette:?}"
    );
    let expected: std::collections::BTreeMap<String, usize> = [
        // 3 instrument placements: LG / LT gauges and the DCS access box.
        (" 18 #008000", 11),
        // 5 annotation placements: Drawing Description, Item Note & Label
        // x3, Line2.
        (" 35 #000000", 28),
        // 7 equipment placements: the vessel, three Flanged Nozzles, one
        // with blind, Manway-Large, Gauge Hatch.
        (" 35 #800000", 29),
        // 5 piping placements: Cap, jinchuzhan2, Ball Valve Type 2, flame
        // arrester breather valve, Off-Unit -- whose `.sym`-cyan strokes
        // are among these, olive on screen and olive here.
        (" 35 #808000", 52),
    ]
    .iter()
    .map(|(key, count)| ((*key).to_string(), *count))
    .collect();
    assert_eq!(
        palette, expected,
        "PID-SYMBOL line work should carry the four class colours the \
         placements name"
    );
}

/// The vessel draws in the maroon its placement states, not the black its
/// `.sym` is authored in.
///
/// This is the discriminating case for where a body's colour comes from.
/// `Parametric Manifold.sym` styles every vessel stroke `#000000` 0.35mm;
/// the placement (oid 326) names style id 75, `#800000` 0.35mm; SmartPlant's
/// screen shows maroon. Reverting either half — the `+25` read in `pid-parse`
/// or `apply_symbology`'s `PID-SYMBOL` gate here — turns these strokes black
/// and this test red.
#[test]
fn the_vessel_draws_in_its_placements_maroon_not_its_syms_black() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    // The vessel shell: two 188mm runs at y = 223.33 and y = 182.69.
    let mut shell = 0usize;
    for entity in on_layer(&doc, "PID-SYMBOL") {
        let EntityType::Line(line) = entity else {
            continue;
        };
        let length = line.start.distance(&line.end);
        if !(187.0..190.0).contains(&length) {
            continue;
        }
        shell += 1;
        assert_eq!(
            line.common.color,
            acadrust::types::Color::Rgb { r: 128, g: 0, b: 0 },
            "a vessel shell stroke should draw #800000: {line:?}"
        );
        assert_eq!(
            line.common.line_weight,
            acadrust::types::LineWeight::Value(35),
            "a vessel shell stroke should weigh 0.35mm: {line:?}"
        );
    }
    assert_eq!(shell, 2, "DWG-0201's vessel has two 188mm shell runs");
}

/// A point that carries a class colour is marked with the slash SmartPlant
/// shows; a black-styled point draws nothing, which is also what its screen
/// shows.
///
/// DWG-0201 places 75 decoded points. Eleven resolve to `#0000FF` -- one on
/// each riser top and one at the vessel inlet -- and the screenshot shows a
/// short blue slash at exactly those eleven spots and nowhere else: not at
/// the 53 black junction points, not at the 11 black riser feet, though the
/// records and style chains are byte-identical apart from the colour. The
/// slash is measured off that screenshot: 15.24mm (0.6 inch) at 62 degrees,
/// centred on the point. Reverting the tick build in `build_entities` leaves
/// `PID-POINT` with no line work and this test red.
#[test]
fn a_class_coloured_point_is_marked_with_the_slash_smartplant_shows() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    // The eleven class-coloured points, from the decoded igPoint2d records.
    let marked = [
        (81.83, 260.51),
        (95.43, 261.35),
        (108.12, 260.35),
        (120.91, 258.83),
        (133.22, 261.35),
        (146.23, 260.35),
        (158.95, 260.35),
        (171.54, 260.35),
        (184.13, 260.35),
        (196.85, 260.35),
        (294.11, 224.66),
    ];

    let mut ticks = 0usize;
    let mut points = 0usize;
    for entity in on_layer(&doc, "PID-POINT") {
        match entity {
            EntityType::Point(_) => points += 1,
            EntityType::Line(line) => {
                ticks += 1;
                let length = line.start.distance(&line.end);
                assert!(
                    (length - 15.24).abs() < 0.01,
                    "a point slash is 0.6 inch long, got {length:.3}: {line:?}"
                );
                let angle = (line.end.y - line.start.y)
                    .atan2(line.end.x - line.start.x)
                    .to_degrees()
                    .rem_euclid(180.0);
                assert!(
                    (angle - 62.0).abs() < 0.1,
                    "a point slash leans 62 degrees, got {angle:.2}: {line:?}"
                );
                let mid = (
                    (line.start.x + line.end.x) / 2.0,
                    (line.start.y + line.end.y) / 2.0,
                );
                assert!(
                    marked
                        .iter()
                        .any(|(x, y)| (mid.0 - x).hypot(mid.1 - y) < 0.05),
                    "a slash centres on one of the class-coloured points, got {mid:?}"
                );
                assert_eq!(
                    line.common.color,
                    acadrust::types::Color::Rgb { r: 0, g: 0, b: 255 },
                    "DWG-0201's marked points are trace-blue: {line:?}"
                );
            }
            other => panic!("PID-POINT carries points and slashes only, found {other:?}"),
        }
    }
    assert_eq!(points, 75, "every decoded point still lands on the layer");
    assert_eq!(
        ticks, 11,
        "exactly the class-coloured points are marked -- the 64 black ones \
         draw nothing, same as SmartPlant's screen"
    );
}

/// A symbol's lettering follows its placement's colour, not the colour its
/// `.sym` character style states.
///
/// The discriminating pair is the level-gauge bubbles: `LG-Magnetic Float
/// Gauge.sym` and `LT-Magnetostrictive Level Gauge.sym` both author their
/// bubble letters in a `#FF0000` character style, their placements name the
/// instrument class colour `#008000`, and the screenshot letters them green.
/// `Drawing Description`'s `说 明` letters black on a black-styled placement,
/// which agrees with either reading and is pinned as the control. Reverting
/// the lettering branch in `apply_symbology` leaves these texts `ByLayer`
/// and this test red.
#[test]
fn a_symbols_lettering_follows_its_placement_colour_not_its_syms() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    let mut seen: std::collections::BTreeMap<String, acadrust::types::Color> =
        std::collections::BTreeMap::new();
    for entity in on_layer(&doc, "PID-SYMBOL") {
        if let EntityType::Text(text) = entity {
            seen.insert(text.value.clone(), text.common.color);
        }
    }

    let expected: std::collections::BTreeMap<String, acadrust::types::Color> = [
        ("LGM", acadrust::types::Color::Rgb { r: 0, g: 128, b: 0 }),
        ("LTM", acadrust::types::Color::Rgb { r: 0, g: 128, b: 0 }),
        ("说 明", acadrust::types::Color::Rgb { r: 0, g: 0, b: 0 }),
    ]
    .iter()
    .map(|(value, colour)| ((*value).to_string(), *colour))
    .collect();
    assert_eq!(
        seen, expected,
        "a symbol's lettering takes the placement's colour -- the LG/LT \
         letters are authored #FF0000 in their own .sym and screen green"
    );
}

/// The opening view is framed on what the drawing draws.
///
/// Framing used to take a symbol placement's insertion point, which for the
/// third of the library authored away from its own origin is a point no line
/// work reaches. DWG-0202's `ext_min` read x = -25.64mm on the strength of
/// six such anchors, so the drawing opened zoomed out over 25mm of sheet that
/// holds nothing -- while its border starts at x = 0. This pins the extents
/// inside the geometry that exists, which is the property the anchor broke.
#[test]
fn the_opening_view_is_framed_on_geometry_that_exists() {
    // A hatch states its area through boundary edges this test does not walk,
    // and lettering reaches past its insertion point by however wide the
    // renderer sets it. Both are inside the drawing rather than off it, so a
    // millimetre of slack covers the difference without covering the defect,
    // which was 25mm wide.
    const SLACK_MM: f64 = 1.0;

    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        let Some(drawn) = doc
            .entities()
            .filter_map(drawn_box)
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))
        else {
            panic!("{name}: nothing was drawn");
        };

        let min = doc.header.model_space_extents_min;
        let max = doc.header.model_space_extents_max;
        assert!(
            min.x >= drawn.0 - SLACK_MM
                && min.y >= drawn.1 - SLACK_MM
                && max.x <= drawn.2 + SLACK_MM
                && max.y <= drawn.3 + SLACK_MM,
            "{name}: framed on ({:.2}, {:.2})..({:.2}, {:.2}), \
             but the drawing only reaches ({:.2}, {:.2})..({:.2}, {:.2})",
            min.x,
            min.y,
            max.x,
            max.y,
            drawn.0,
            drawn.1,
            drawn.2,
            drawn.3
        );
    }
}

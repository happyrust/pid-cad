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
use OpenCADStudio::io::pid::{load_pid_with_layer_mode, PidLayerMode, LAYER_MODE_ENV};
use OpenCADStudio::io::pid_view_filter::{
    switch_role, switch_sheet_layer, PidViewFilter, PidViewSummary,
};

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

/// The fixture as the application opens it: under the layer mode
/// `OCS_PID_LAYER_MODE` selects, the taxonomy unless whoever runs the suite
/// set `sheet`. A test that reaches its entities by `role=` passes either way,
/// which is how the suite runs green in both modes (plan 2026-09-07, L3).
fn import(name: &str) -> Option<CadDocument> {
    let path = fixture(name)?;
    Some(
        OpenCADStudio::io::load_file(&path)
            .unwrap_or_else(|error| panic!("load {}: {error}", path.display())),
    )
}

/// The fixture under the taxonomy layer mode whatever the environment says:
/// what a test about the `PID-*` layers themselves -- which is declared, which
/// opens off, what `PID-HIDDEN` holds -- imports with.
fn import_in_taxonomy_mode(name: &str) -> Option<CadDocument> {
    let path = fixture(name)?;
    Some(
        load_pid_with_layer_mode(&path, PidLayerMode::Taxonomy)
            .unwrap_or_else(|error| panic!("load {} in taxonomy mode: {error}", path.display())),
    )
}

fn layer_of(entity: &EntityType) -> &str {
    entity.common().layer.as_str()
}

fn on_layer<'a>(doc: &'a CadDocument, layer: &'a str) -> impl Iterator<Item = &'a EntityType> {
    doc.entities().filter(move |e| layer_of(e) == layer)
}

/// Prefix of the layers named line work files under, one per authored style
/// name the drawing's project library states.
const DISCIPLINE_PREFIX: &str = "PID-STYLE-";

/// Whether a layer carries the sheet's own line work under the taxonomy
/// layer mode.
///
/// The family, not one member: a record whose style the drawing names is on a
/// `PID-STYLE-*` layer and one it does not name is on `PID-GEOMETRY`. Same
/// shape as `PID-POINT*`, and for the same reason -- a test that means "the
/// drawing's line work" would otherwise quietly narrow to the unnamed part of
/// it as soon as the disciplines landed.
fn is_line_work(layer: &str) -> bool {
    layer == "PID-GEOMETRY" || layer.starts_with(DISCIPLINE_PREFIX)
}

/// The sheet's own line work, by the `role=` the importer wrote rather than
/// the layer it filed the entity on: the same set as [`is_line_work`] under
/// the taxonomy layer mode, and still the line work when the slot holds the
/// authored sheet layer instead (plan 2026-09-07, L3).
fn on_line_work(doc: &CadDocument) -> impl Iterator<Item = &EntityType> {
    of_role(doc, "geometry")
}

/// The sheet's own lettering -- `role=text`, which a label the importer made
/// for a symbol (`symbol-label`) is not. Independent of the layer slot, so a
/// hidden text is included whether it sits on `PID-HIDDEN` or on its own
/// authored layer.
fn on_sheet_text(doc: &CadDocument) -> impl Iterator<Item = &EntityType> {
    of_role(doc, "text").filter(|entity| matches!(entity, EntityType::Text(_)))
}

/// Placement-time values are separate sheet text records, not the `NULL`
/// placeholders embedded in the reusable symbol body. Both halves must meet
/// on screen: the placed symbol supplies its bubble/frame while the sheet
/// supplies the assigned tag, sequence and notes.
#[test]
fn placement_assignments_render_as_sheet_text_without_null_placeholders() {
    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };

    let values: std::collections::BTreeSet<String> = on_sheet_text(&doc)
        .filter_map(|entity| match entity {
            EntityType::Text(text) => Some(text.value.trim().to_string()),
            _ => None,
        })
        .collect();
    for expected in [
        "LIA",
        "LIT",
        "060201",
        "RD060201",
        "1、污油池上放空管线高度应大于5m。",
        "2、阻火器电伴热带从根部缠至地面以上2m。",
    ] {
        assert!(
            values.contains(expected),
            "assigned value {expected:?} did not reach PID-TEXT; values={values:?}"
        );
    }
    assert!(
        doc.entities().all(|entity| !matches!(entity, EntityType::Text(text) if text.value.contains("NULL"))),
        "a symbol-library NULL template leaked into visible drawing text"
    );
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
    // The drawing's own line work and every point's mark, by role rather
    // than by layer: a named line files under its discipline, a mark under
    // its review status, a hidden stroke under `PID-HIDDEN` or its authored
    // layer, and the style has to survive every one of those moves.
    let drawing = doc.entities().filter(|entity| {
        let role = pid_value(entity, "role");
        role.as_deref() == Some("geometry")
            || role
                .as_deref()
                .is_some_and(|role| role.starts_with("point-"))
    });
    for entity in drawing {
        let common = entity.common();
        match (common.color, common.line_weight) {
            (acadrust::types::Color::Rgb { r, g, b }, acadrust::types::LineWeight::Value(w)) => {
                *palette
                    .entry(format!("{w:>3} #{r:02X}{g:02X}{b:02X}"))
                    .or_default() += 1;
            }
            _ => unstyled += 1,
        }
    }

    assert_eq!(
        unstyled, 0,
        "every entity on the drawing layers should carry a resolved style, got palette {palette:?}"
    );
    // Widths are hundredths of a millimetre, so 70 is the 0.7mm process
    // header and 10 the 0.1mm point symbol. Olive #808000 on the heavy lines
    // and green #008000 on the thin ones is this drawing's own palette.
    // The 33 blue entities are the eleven marked points three times over:
    // the point record itself plus the two strokes of the glyph its
    // terminator names -- see `a_point_draws_the_symbol_its_terminator_names`.
    let expected: std::collections::BTreeMap<String, usize> = [
        (" 10 #000000", 53),
        (" 10 #0000FF", 33),
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
        summary.sheet_layers > 0,
        "authored sheet layers are summarized"
    );
    assert!(
        summary.layered_entities > 0,
        "drawn entities retain authored layers"
    );
    assert!(
        !summary.style_tables_failed,
        "the fixture's style tables read; the palette test depends on it"
    );
    // The line the Layer Manager's sheet-layer view is summarised by (plan
    // 2026-09-07, L2 step 4): the layers as the reader sees them -- by name,
    // which is fewer than by storage-local id -- and how many start off.
    assert_eq!(
        summary.sheet_layer_names, 4,
        "ConsistencyChecks / Default / HiddenObjects / Labels"
    );
    assert!(summary.sheet_layer_names <= summary.sheet_layers);
    assert_eq!(summary.sheet_layers_off, 1, "HiddenObjects");
    assert_eq!(
        summary.sheet_layer_names,
        PidViewSummary::of(&doc).layers.len()
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
    let dashable = doc.entities().filter(|entity| {
        matches!(
            pid_value(entity, "role").as_deref(),
            Some("geometry" | "point-ok")
        )
    });
    for entity in dashable {
        let name = entity.common().linetype.as_str();
        if name.starts_with("PID-DASH-") {
            *used.entry(name.to_string()).or_default() += 1;
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
    for entity in on_sheet_text(&doc) {
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
    for entity in on_sheet_text(&doc) {
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
        on_sheet_text(&doc)
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
        for entity in on_sheet_text(&doc) {
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
    for entity in on_sheet_text(&doc) {
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
    for entity in on_sheet_text(&doc) {
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
    for entity in on_sheet_text(&doc) {
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
    let strayed: Vec<&str> = ["symbol-label", "symbol"]
        .iter()
        .flat_map(|role| of_role(&doc, role))
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
/// rather than drawing ship switched off. A statement about the taxonomy's
/// layer table, so it imports under that mode.
#[test]
fn import_declares_its_layers_and_hides_the_evidence_ones() {
    let Some(doc) = import_in_taxonomy_mode("DWG-0201GP06-01.pid") else {
        return;
    };

    for visible in [
        "PID-GEOMETRY",
        "PID-FRAME",
        "PID-TEXT",
        "PID-FILL",
        "PID-SYMBOL",
        "PID-POINT",
        // The review statuses are drawing content the source shows, so they
        // open visible even though `PID-POINT-ERROR` is empty on every
        // fixture -- see
        // `a_points_mark_files_under_the_review_status_the_drawing_names`.
        "PID-POINT-WARNING",
        "PID-POINT-ERROR",
        "PID-POINT-APPROVED",
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
/// the anchor's real offset is ever found, the stubs come back here. The
/// declaration is the taxonomy's, so that mode is imported; that nothing
/// takes the role holds in either.
#[test]
fn the_annotation_layer_is_declared_but_draws_nothing() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import_in_taxonomy_mode(name) else {
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
        let Some(doc) = import(name) else {
            continue;
        };
        assert_eq!(
            of_role(&doc, "annotation").count(),
            0,
            "{name}: nothing may take role=annotation in either layer mode"
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
        let hatches: Vec<_> = of_role(&doc, "fill").collect();
        assert_eq!(
            hatches.len(),
            expected,
            "{name}: expected {expected} filled area(s) with role=fill"
        );
        for entity in &hatches {
            let EntityType::Hatch(hatch) = entity else {
                panic!("{name}: role=fill must be a hatch, got {entity:?}");
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

    for entity in on_line_work(&doc) {
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
    let mut labels: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
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
            if let Some(label) = text.strip_prefix("label=") {
                labels.insert(label.to_string());
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
        !labels.is_empty(),
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
    assert!(
        classes.contains("PIDControlSystemFunction")
            && labels.contains("LIA-060201")
            && labels.contains("LIT-060201"),
        "DrawingItems must bind representations to business objects, not the PIDDrawing container; classes={classes:?} labels={labels:?}"
    );
}

/// An authored sheet layer is P&ID metadata even when the optional published
/// semantic XML is absent. Its storage-local oid and decoded name therefore
/// ride the same durable XDATA record without inventing a semantic class.
#[test]
fn a_drawing_without_published_xml_still_carries_authored_sheet_layers() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import(name) else {
            continue;
        };
        // Every entity carries a record now (`role=` at least), so the
        // authored-layer records are the ones that mention a sheet layer at
        // all -- by oid, which is always written, or by the name the oid
        // resolved to.
        let records: Vec<_> = doc
            .entities()
            .filter_map(|entity| entity.common().extended_data.get_record("PID_SEMANTICS"))
            .filter(|record| {
                record.values.iter().any(|value| {
                    matches!(value, acadrust::xdata::XDataValue::String(text) if text.starts_with("sheet_layer"))
                })
            })
            .collect();
        assert!(
            !records.is_empty(),
            "{name}: no authored layer XDATA reached the drawing"
        );
        assert!(records.iter().all(|record| record.values.iter().any(|value| {
            matches!(value, acadrust::xdata::XDataValue::String(text) if text.starts_with("sheet_layer_oid="))
        })), "{name}: an authored-layer XDATA record lost its oid");
        assert!(records.iter().any(|record| record.values.iter().any(|value| {
            matches!(value, acadrust::xdata::XDataValue::String(text) if text.starts_with("sheet_layer="))
        })), "{name}: no authored layer name resolved");
    }
}

/// Under the taxonomy layer mode the entities of a hidden sheet layer sit on
/// `PID-HIDDEN`, off, and keep the sheet layer they came from in XDATA
/// through DWG and DXF. (The sheet layer mode's counterpart is
/// `sheet_mode_layer_names_survive_dwg_and_dxf`.)
#[test]
fn hidden_authored_layers_open_on_pid_hidden_and_metadata_survives_dwg_and_dxf() {
    let Some(doc) = import_in_taxonomy_mode("DWG-0201GP06-01.pid") else {
        return;
    };
    assert!(is_hidden(&doc, "PID-HIDDEN"), "PID-HIDDEN must default off");
    assert_eq!(on_layer(&doc, "PID-HIDDEN").count(), 11);
    for entity in on_layer(&doc, "PID-HIDDEN") {
        let record = entity
            .common()
            .extended_data
            .get_record("PID_SEMANTICS")
            .expect("hidden source entity keeps PID metadata");
        assert!(record.values.iter().any(|value| {
            matches!(value, acadrust::xdata::XDataValue::String(text) if text == "sheet_layer=HiddenObjects")
        }));
    }

    for ext in ["dwg", "dxf"] {
        let bytes = OpenCADStudio::io::save_to_bytes(&doc, ext, doc.version)
            .unwrap_or_else(|error| panic!("save {ext}: {error}"));
        let reopened =
            OpenCADStudio::io::load_bytes(&format!("sheet-layer-roundtrip.{ext}"), bytes)
                .unwrap_or_else(|error| panic!("reopen {ext}: {error}"));
        assert!(reopened.entities().any(|entity| {
            entity
                .common()
                .extended_data
                .get_record("PID_SEMANTICS")
                .is_some_and(|record| record.values.iter().any(|value| {
                    matches!(value, acadrust::xdata::XDataValue::String(text) if text == "sheet_layer=HiddenObjects")
                }))
        }), "authored sheet-layer XDATA was lost across {ext} round-trip");
        assert_eq!(
            pid_records_with(&reopened, "role").count(),
            pid_records_with(&doc, "role").count(),
            "{ext} round-trip changed how many entities state their role"
        );
        assert!(
            reopened
                .entities()
                .any(|entity| pid_value(entity, "role").as_deref() == Some("frame")),
            "{ext} round-trip lost the page border's role=frame"
        );
    }
}

/// The entities filed under one authored sheet layer, by the `sheet_layer=`
/// the importer wrote.
fn on_sheet_layer<'a>(
    doc: &'a CadDocument,
    layer: &'a str,
) -> impl Iterator<Item = &'a EntityType> {
    doc.entities()
        .filter(move |entity| pid_value(entity, "sheet_layer").as_deref() == Some(layer))
}

fn of_role<'a>(doc: &'a CadDocument, role: &'a str) -> impl Iterator<Item = &'a EntityType> {
    doc.entities()
        .filter(move |entity| pid_value(entity, "role").as_deref() == Some(role))
}

fn dark(doc: &CadDocument) -> usize {
    doc.entities()
        .filter(|entity| entity.common().invisible)
        .count()
}

/// The import leaves the drawing a view filter of its own (plan 2026-09-07,
/// L2 step 2): the sheet layers SmartPlant hides -- `Hidden` / `HiddenObjects`
/// / `Invisible`, by name until L1 reads the file's display state -- start
/// switched off, and every entity on them carries `invisible`, the same
/// reading `PID-HIDDEN` gives but on the entity itself. Nothing else is dark,
/// and no role starts switched off.
#[test]
fn the_import_switches_the_hidden_sheet_layers_off_in_a_stored_view_filter() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };
    let filter = PidViewFilter::load(&doc).expect("the import stores a PID_VIEW_FILTER record");
    assert_eq!(filter.layers_off().collect::<Vec<_>>(), ["HiddenObjects"]);
    assert_eq!(filter.roles_off().count(), 0);
    assert!(!filter.layer_is_on("HiddenObjects"));
    assert!(filter.layer_is_on("Labels"));

    let hidden: Vec<_> = on_sheet_layer(&doc, "HiddenObjects").collect();
    assert_eq!(hidden.len(), 11);
    assert!(
        hidden.iter().all(|entity| entity.common().invisible),
        "an entity on a switched-off sheet layer draws"
    );
    assert_eq!(
        dark(&doc),
        hidden.len(),
        "something outside HiddenObjects is dark"
    );
}

/// The switched-off set is the file's own statement, not a guess from names
/// (plan 2026-09-07, L1): `pid-parse` reads each sheet layer's display bit
/// from the sheet's `Top ViewFilterSet`, and the import starts a layer off
/// exactly when the file draws none of it. On the corpus that is
/// `HiddenObjects` alone -- the same answer the name criterion gave -- so
/// this pins the *source* of the answer: for every drawn entity, the layer
/// it was filed under and the bit its record carries agree with the stored
/// filter. Which layer that is depends on the layer mode -- `PID-HIDDEN`
/// under the taxonomy, the sheet layer's own name under the sheet mode -- and
/// either way it opens off; no entity with a displayed layer is on
/// `PID-HIDDEN`.
#[test]
fn the_import_takes_the_switched_off_layers_from_the_file_not_from_their_names() {
    for fixture_name in ["DWG-0201GP06-01.pid", "DWG-0202GP06-01.pid", "D06.pid"] {
        let Some(path) = fixture(fixture_name) else {
            return;
        };
        let parsed = pid_parse::PidParser::new()
            .parse_file(&path)
            .unwrap_or_else(|error| panic!("{fixture_name}: pid-parse: {error}"));
        let geometry = pid_parse::build_normalized_geometry(&parsed);
        let mut stated_off = std::collections::BTreeSet::new();
        let mut stated_on = std::collections::BTreeSet::new();
        for entity in &geometry.entities {
            let Some(layer) = &entity.source_layer else {
                continue;
            };
            let (Some(name), Some(displayed)) = (layer.name.as_deref(), layer.displayed) else {
                continue;
            };
            if displayed {
                stated_on.insert(name.to_string());
            } else {
                stated_off.insert(name.to_string());
            }
        }
        assert!(
            !stated_off.is_empty() && !stated_on.is_empty(),
            "{fixture_name}: the file states both switched-off and displayed layers"
        );
        assert!(
            stated_off.is_disjoint(&stated_on),
            "{fixture_name}: a name both off and on: {:?}",
            stated_off.intersection(&stated_on).collect::<Vec<_>>()
        );

        let doc = OpenCADStudio::io::load_file(&path)
            .unwrap_or_else(|error| panic!("{fixture_name}: import: {error}"));
        let filter = PidViewFilter::load(&doc).expect("the import stores a filter");
        assert_eq!(
            filter
                .layers_off()
                .collect::<std::collections::BTreeSet<_>>(),
            stated_off
                .iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            "{fixture_name}: the filter starts with the layers the file switches off"
        );
        for entity in doc.entities() {
            let Some(name) = pid_value(entity, "sheet_layer") else {
                continue;
            };
            if stated_off.contains(&name) {
                let layer = layer_of(entity);
                let is_label = pid_value(entity, "role").as_deref() == Some("symbol-label");
                assert!(
                    layer == "PID-HIDDEN"
                        || layer == name
                        || (is_label && layer == "PID-SYMBOL-LABEL"),
                    "{fixture_name}: {name} is filed on {layer}, which neither mode does"
                );
                assert!(
                    is_hidden(&doc, layer),
                    "{fixture_name}: {name} is filed on {layer}, which opens on"
                );
                assert!(entity.common().invisible, "{fixture_name}: {name} draws");
            } else if stated_on.contains(&name) {
                assert_ne!(layer_of(entity), "PID-HIDDEN", "{fixture_name}: {name}");
                assert!(!entity.common().invisible, "{fixture_name}: {name} is dark");
            }
        }
    }
}

/// The acceptance the plan names: switch `Labels` off in the record and
/// 0202's labels go dark -- its 46 text entities, and with them the 46 pieces
/// of line work and 5 fills the sheet files under the same layer -- while
/// `Default` keeps drawing; switch it back on and every one of them returns.
/// The record follows the state, and an all-on filter leaves no record
/// behind to resurrect anything later.
#[test]
fn switching_a_sheet_layer_off_hides_its_entities_and_on_brings_them_back() {
    let Some(mut doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };
    let mut filter = PidViewFilter::load(&doc).expect("0202 hides HiddenObjects too");
    assert_eq!(filter.layers_off().collect::<Vec<_>>(), ["HiddenObjects"]);
    let hidden_objects = on_sheet_layer(&doc, "HiddenObjects").count();
    assert_eq!(hidden_objects, 16);
    assert_eq!(dark(&doc), hidden_objects);
    let labels = on_sheet_layer(&doc, "Labels").count();
    assert_eq!(labels, 97, "46 text + 46 line work + 5 fills");
    let label_text = on_sheet_layer(&doc, "Labels")
        .filter(|entity| matches!(entity, EntityType::Text(_)))
        .count();
    assert_eq!(label_text, 46);

    filter.set_layer("Labels", false);
    filter.store(&mut doc);
    assert_eq!(
        filter.apply(&mut doc),
        labels,
        "every Labels entity changes"
    );
    assert!(
        on_sheet_layer(&doc, "Labels").all(|entity| entity.common().invisible),
        "a label still draws with its layer off"
    );
    assert!(
        on_sheet_layer(&doc, "Default").all(|entity| !entity.common().invisible),
        "Default went dark with Labels"
    );
    assert_eq!(dark(&doc), labels + hidden_objects);
    assert_eq!(
        PidViewFilter::load(&doc).as_ref(),
        Some(&filter),
        "the record does not read back what was set"
    );

    filter.set_layer("Labels", true);
    filter.store(&mut doc);
    assert_eq!(filter.apply(&mut doc), labels);
    assert!(on_sheet_layer(&doc, "Labels").all(|entity| !entity.common().invisible));
    assert_eq!(dark(&doc), hidden_objects);

    filter.set_layer("HiddenObjects", true);
    filter.store(&mut doc);
    assert_eq!(filter.apply(&mut doc), hidden_objects);
    assert_eq!(dark(&doc), 0);
    assert!(filter.is_empty());
    assert!(
        PidViewFilter::load(&doc).is_none(),
        "an all-on filter must not leave a record behind"
    );
}

/// Roles are the second axis. `text` off darkens every text entity whatever
/// sheet layer it sits on -- and only those: a symbol's label is
/// `symbol-label`, not `text`. The entities the importer drew itself -- the
/// frame, the connectivity links -- have no sheet layer to answer to and are
/// governed by role alone: no `sheet_layer=` counts as on (L2 step 2).
#[test]
fn a_role_switch_reaches_every_sheet_layer_and_is_all_a_layerless_entity_answers_to() {
    let Some(mut doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };
    let mut filter = PidViewFilter::load(&doc).unwrap_or_default();
    let hidden_objects = on_sheet_layer(&doc, "HiddenObjects").count();
    let text = of_role(&doc, "text").count();
    assert_eq!(text, 48, "47 on Labels and 1 on HiddenObjects");
    let text_already_dark = of_role(&doc, "text")
        .filter(|entity| entity.common().invisible)
        .count();
    assert_eq!(
        text_already_dark, 1,
        "the HiddenObjects text is dark already"
    );

    filter.set_role("text", false);
    filter.store(&mut doc);
    assert_eq!(filter.apply(&mut doc), text - text_already_dark);
    assert!(of_role(&doc, "text").all(|entity| entity.common().invisible));
    assert!(
        of_role(&doc, "symbol-label").all(|entity| !entity.common().invisible),
        "a symbol label is not `text` and must keep drawing"
    );
    assert_eq!(dark(&doc), hidden_objects + text - text_already_dark);

    let frame: Vec<_> = of_role(&doc, "frame").collect();
    assert_eq!(frame.len(), 1);
    assert!(
        pid_value(frame[0], "sheet_layer").is_none(),
        "the frame is the importer's own"
    );
    assert!(!frame[0].common().invisible);
    let links = of_role(&doc, "connectivity").count();
    assert_eq!(links, 25);
    assert!(of_role(&doc, "connectivity").all(|entity| pid_value(entity, "sheet_layer").is_none()));

    filter.set_role("frame", false);
    filter.set_role("connectivity", false);
    filter.store(&mut doc);
    assert_eq!(filter.apply(&mut doc), 1 + links);
    assert!(of_role(&doc, "frame").all(|entity| entity.common().invisible));
    assert!(of_role(&doc, "connectivity").all(|entity| entity.common().invisible));
    assert_eq!(
        filter.roles_off().collect::<Vec<_>>(),
        ["connectivity", "frame", "text"]
    );

    filter.set_role("text", true);
    filter.set_role("frame", true);
    filter.set_role("connectivity", true);
    filter.store(&mut doc);
    assert_eq!(filter.apply(&mut doc), text - text_already_dark + 1 + links);
    assert_eq!(dark(&doc), hidden_objects);
}

/// The filter and the bits it set travel together: after a DWG or a DXF save
/// the reopened drawing reads the same record, and the same entities -- and
/// only those -- are dark.
#[test]
fn the_view_filter_and_its_invisible_bits_survive_dwg_and_dxf() {
    let Some(mut doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };
    let mut filter = PidViewFilter::load(&doc).expect("0202 stores a filter");
    filter.set_layer("Labels", false);
    filter.set_role("point-warning", false);
    filter.store(&mut doc);
    filter.apply(&mut doc);
    let expected_dark = dark(&doc);
    assert_eq!(
        expected_dark,
        97 + 16 + 10,
        "Labels + HiddenObjects + point-warning"
    );

    for ext in ["dwg", "dxf"] {
        let bytes = OpenCADStudio::io::save_to_bytes(&doc, ext, doc.version)
            .unwrap_or_else(|error| panic!("save {ext}: {error}"));
        let reopened = OpenCADStudio::io::load_bytes(&format!("view-filter.{ext}"), bytes)
            .unwrap_or_else(|error| panic!("reopen {ext}: {error}"));
        assert_eq!(
            PidViewFilter::load(&reopened).as_ref(),
            Some(&filter),
            "{ext}: the PID_VIEW_FILTER record did not survive"
        );
        assert_eq!(
            dark(&reopened),
            expected_dark,
            "{ext}: the invisible bits changed"
        );
        for entity in reopened
            .entities()
            .filter(|entity| entity.common().invisible)
        {
            let layer = pid_value(entity, "sheet_layer");
            let role = pid_value(entity, "role");
            assert!(
                matches!(layer.as_deref(), Some("Labels" | "HiddenObjects"))
                    || role.as_deref() == Some("point-warning"),
                "{ext}: a dark entity the filter does not name: layer={layer:?} role={role:?}"
            );
        }
    }
}

/// What the layer manager's sheet-layer view lists (plan 2026-09-07, L2 step
/// 3): one row per authored sheet layer name with the entities filed under
/// it and the filter's answer for it, and one row per role in vocabulary
/// order. The names are the ones SmartPlant gave the layers, and the counts
/// are what the drawing has on each of them, so the view can be checked
/// against the filter tests above. All four fixtures draw on the same four
/// of SmartPlant's layers -- `ConsistencyChecks` / `Default` /
/// `HiddenObjects` / `Labels`; the others the template declares (`HeatTrace`
/// among them) carry nothing drawn, so they do not list.
#[test]
fn the_layer_manager_summary_lists_sheet_layers_and_roles_with_their_counts() {
    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };
    let summary = PidViewSummary::of(&doc);
    assert!(!summary.is_empty());

    let names: Vec<&str> = summary.layers.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["ConsistencyChecks", "Default", "HiddenObjects", "Labels"],
        "sheet layers are listed by name"
    );
    let row = |name: &str| {
        summary
            .layers
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("no row for {name}"))
    };
    assert_eq!(
        row("Labels").entities,
        97,
        "46 text + 46 line work + 5 fills"
    );
    assert!(row("Labels").on);
    assert_eq!(row("HiddenObjects").entities, 16);
    assert!(
        !row("HiddenObjects").on,
        "the import switches HiddenObjects off"
    );
    for row in &summary.layers {
        assert_eq!(
            row.entities,
            on_sheet_layer(&doc, &row.name).count(),
            "{}: the count is not the entities stating that layer",
            row.name
        );
    }

    let roles: Vec<&str> = summary.roles.iter().map(|row| row.name.as_str()).collect();
    let rank = |role: &str| ROLES.iter().position(|known| *known == role).unwrap();
    assert!(
        roles.windows(2).all(|pair| rank(pair[0]) < rank(pair[1])),
        "roles are listed in vocabulary order: {roles:?}"
    );
    for row in &summary.roles {
        assert!(row.on, "no role starts switched off");
        assert_eq!(
            row.entities,
            of_role(&doc, &row.name).count(),
            "{}: the count is not the entities stating that role",
            row.name
        );
    }
    assert_eq!(
        summary.layers.iter().map(|row| row.entities).sum::<usize>(),
        doc.entities()
            .filter(|entity| pid_value(entity, "sheet_layer").is_some())
            .count()
    );
}

/// The switch behind each row of that view: a sheet layer goes dark with one
/// call and the record follows (the plan's acceptance names `HeatTrace`; the
/// fixtures draw nothing on it, so `ConsistencyChecks` -- 0202's 37 check
/// marks -- stands in); and switching a hidden sheet layer back on shows its
/// entities -- which means turning `PID-HIDDEN` on as well, since the import
/// files them there with that layer off. The layer is the importer's own
/// hiding device and the entity bit now carries the same reading, so the
/// switch is allowed to release it. That is the taxonomy layer mode's
/// arrangement, so it is imported here; the sheet mode's is pinned by
/// `sheet_mode_switching_a_hidden_sheet_layer_on_releases_its_own_layer`.
#[test]
fn switching_a_row_reaches_the_document_and_releases_the_hidden_layer_when_needed() {
    let Some(mut doc) = import_in_taxonomy_mode("DWG-0202GP06-01.pid") else {
        return;
    };
    let checks = on_sheet_layer(&doc, "ConsistencyChecks").count();
    assert_eq!(checks, 37);
    let hidden_objects = on_sheet_layer(&doc, "HiddenObjects").count();
    assert_eq!(hidden_objects, 16);
    assert!(
        doc.layers
            .get("PID-HIDDEN")
            .is_some_and(|layer| layer.flags.off),
        "PID-HIDDEN opens switched off"
    );
    assert!(on_sheet_layer(&doc, "HiddenObjects").all(|entity| layer_of(entity) == "PID-HIDDEN"));

    let switched = switch_sheet_layer(&mut doc, "ConsistencyChecks", false);
    assert_eq!(switched.entities, checks);
    assert!(switched.released_layer.is_none());
    assert!(on_sheet_layer(&doc, "ConsistencyChecks").all(|entity| entity.common().invisible));
    assert_eq!(dark(&doc), checks + hidden_objects);
    let filter = PidViewFilter::load(&doc).expect("the record follows the switch");
    assert_eq!(
        filter.layers_off().collect::<Vec<_>>(),
        ["ConsistencyChecks", "HiddenObjects"]
    );
    assert!(
        !PidViewSummary::of(&doc)
            .layers
            .iter()
            .find(|row| row.name == "ConsistencyChecks")
            .unwrap()
            .on
    );

    let switched = switch_sheet_layer(&mut doc, "HiddenObjects", true);
    assert_eq!(switched.entities, hidden_objects);
    assert!(
        switched.released_layer.as_deref() == Some("PID-HIDDEN"),
        "the hidden layer must be turned on for the entities to show"
    );
    assert!(doc
        .layers
        .get("PID-HIDDEN")
        .is_some_and(|layer| !layer.flags.off));
    assert!(on_sheet_layer(&doc, "HiddenObjects").all(|entity| !entity.common().invisible));
    assert_eq!(dark(&doc), checks);

    let switched = switch_sheet_layer(&mut doc, "HiddenObjects", false);
    assert_eq!(switched.entities, hidden_objects);
    assert!(
        switched.released_layer.is_none(),
        "switching off never touches the layer table"
    );
    assert!(
        doc.layers
            .get("PID-HIDDEN")
            .is_some_and(|layer| !layer.flags.off),
        "the released layer stays released; the bits do the hiding now"
    );

    let text = of_role(&doc, "text").count();
    let text_dark = of_role(&doc, "text")
        .filter(|entity| entity.common().invisible)
        .count();
    let switched = switch_role(&mut doc, "text", false);
    assert_eq!(switched.entities, text - text_dark);
    assert!(of_role(&doc, "text").all(|entity| entity.common().invisible));
    let summary = PidViewSummary::of(&doc);
    assert!(
        !summary
            .roles
            .iter()
            .find(|row| row.name == "text")
            .unwrap()
            .on
    );
    assert!(summary
        .roles
        .iter()
        .filter(|row| row.name != "text")
        .all(|row| row.on));
}

/// The `key=value` pairs of an entity's `PID_SEMANTICS` record, in the order
/// the importer wrote them; empty when the entity carries none.
fn pid_pairs(entity: &EntityType) -> Vec<(String, String)> {
    entity
        .common()
        .extended_data
        .get_record("PID_SEMANTICS")
        .map(|record| {
            record
                .values
                .iter()
                .filter_map(|value| match value {
                    acadrust::xdata::XDataValue::String(text) => text
                        .split_once('=')
                        .map(|(key, val)| (key.to_string(), val.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The first value written under `key`, if any.
fn pid_value(entity: &EntityType, key: &str) -> Option<String> {
    pid_pairs(entity)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn pid_records_with<'a>(
    doc: &'a CadDocument,
    key: &'a str,
) -> impl Iterator<Item = &'a EntityType> {
    doc.entities()
        .filter(move |entity| pid_value(entity, key).is_some())
}

/// The vocabulary of `role=`: the importer's own reading of what an entity
/// is. Written out here rather than shared with the importer, so a new role
/// has to be added to the test on purpose.
const ROLES: [&str; 12] = [
    "geometry",
    "text",
    "symbol",
    "symbol-label",
    "point-ok",
    "point-warning",
    "point-error",
    "point-approved",
    "annotation",
    "connectivity",
    "fill",
    "frame",
];

/// The role an entity's synthetic layer stands for today. The test's own
/// table, not the importer's: the whole point of `role=` is that the layer
/// slot may later hold the authored sheet layer instead (plan 2026-09-07 L3),
/// and this is the yardstick that says the XDATA kept the reading.
fn role_of_layer(layer: &str) -> Option<&'static str> {
    Some(match layer {
        "PID-GEOMETRY" => "geometry",
        "PID-TEXT" => "text",
        "PID-SYMBOL" => "symbol",
        "PID-SYMBOL-LABEL" => "symbol-label",
        "PID-POINT" => "point-ok",
        "PID-POINT-WARNING" => "point-warning",
        "PID-POINT-ERROR" => "point-error",
        "PID-POINT-APPROVED" => "point-approved",
        "PID-ANNOTATION" => "annotation",
        "PID-CONNECTIVITY" => "connectivity",
        "PID-FILL" => "fill",
        "PID-FRAME" => "frame",
        other if other.starts_with(DISCIPLINE_PREFIX) => "geometry",
        _ => return None,
    })
}

/// Every entity the import draws says what it is, in XDATA, independently of
/// the layer it happens to be filed on: `role=` is the classification the
/// `PID-*` taxonomy has carried in the layer slot until now, moved to where a
/// change of layer policy cannot lose it (plan 2026-09-07, D8 / L2). On an
/// entity that stays on its synthetic layer the two agree; an entity moved to
/// `PID-HIDDEN` keeps the role of the layer it would otherwise be on. The
/// agreement is with the taxonomy's layers, so that mode is imported; under
/// the sheet mode the slot holds the authored layer and
/// `sheet_mode_files_every_entity_under_its_authored_layer_and_declares_the_drawings_layers`
/// checks the roles instead.
#[test]
fn every_imported_entity_states_its_role_and_the_role_matches_its_layer() {
    for name in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import_in_taxonomy_mode(name) else {
            continue;
        };
        let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for entity in doc.entities() {
            let layer = layer_of(entity);
            let role = pid_value(entity, "role")
                .unwrap_or_else(|| panic!("{name}: an entity on {layer} states no role="));
            assert!(
                ROLES.contains(&role.as_str()),
                "{name}: role={role} on {layer} is not in the vocabulary"
            );
            if layer == "PID-HIDDEN" {
                assert_ne!(
                    role, "frame",
                    "{name}: the page border is drawn by the importer and cannot be on an authored hidden layer"
                );
            } else {
                assert_eq!(
                    role_of_layer(layer),
                    Some(role.as_str()),
                    "{name}: role={role} disagrees with the layer {layer} the entity is filed on"
                );
            }
            *seen.entry(role).or_default() += 1;
        }
        for expected in ["geometry", "text", "symbol", "frame"] {
            assert!(
                seen.contains_key(expected),
                "{name}: no entity took role={expected}; seen={seen:?}"
            );
        }
        assert_eq!(
            seen.get("frame"),
            Some(&1),
            "{name}: exactly one entity is the page border"
        );
    }
}

/// `role=` and `class=` are two keys with two vocabularies in the same
/// record: `class` is the published object's XML element name
/// (`PIDPipeline`, `PIDProcessVessel`, …) and `role` is the importer's
/// reading. Neither may leak into the other, and the import never writes a
/// record legend recognition would claim as its own (`resolved=legend:*`).
#[test]
fn role_and_class_are_separate_keys_with_disjoint_vocabularies() {
    let Some(doc) = import("export-test/publish-data/DWG-0202GP06-01/DWG-0202GP06-01.pid") else {
        return;
    };
    const KEYS: [&str; 8] = [
        "sheet_layer",
        "sheet_layer_oid",
        "role",
        "style",
        "class",
        "label",
        "oid",
        "resolved",
    ];
    let mut both = 0usize;
    for entity in doc.entities() {
        let pairs = pid_pairs(entity);
        for (key, value) in &pairs {
            assert!(
                KEYS.contains(&key.as_str()),
                "the import wrote an unknown PID_SEMANTICS key {key}={value}"
            );
            if key == "resolved" {
                assert!(
                    !value.starts_with("legend:"),
                    "the import wrote a legend-owned record; PIDLEGEND PURGE would delete it"
                );
            }
        }
        let role = pid_value(entity, "role");
        let class = pid_value(entity, "class");
        if let (Some(role), Some(class)) = (role, class) {
            both += 1;
            assert!(ROLES.contains(&role.as_str()), "role={role} is not a role");
            assert!(
                !ROLES.contains(&class.as_str()) && class.starts_with("PID"),
                "class={class} reads like a role, not a published element name"
            );
        }
    }
    assert!(
        both > 0,
        "no entity carries both a published class= and an import role="
    );
}

/// Line work that is filed under a discipline also states, as `style=`, the
/// authored style name the discipline layer was derived from -- the name is
/// the fact, the layer is one spelling of it (D8 / L2). The spelling is the
/// taxonomy's, so that mode is imported.
#[test]
fn named_line_work_states_the_style_its_discipline_layer_is_derived_from() {
    let Some(doc) = import_in_taxonomy_mode("DWG-0201GP06-01.pid") else {
        return;
    };
    let mut styles: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for entity in doc
        .entities()
        .filter(|entity| layer_of(entity).starts_with(DISCIPLINE_PREFIX))
    {
        let style = pid_value(entity, "style").unwrap_or_else(|| {
            panic!(
                "an entity on {} names no style= to derive that layer from",
                layer_of(entity)
            )
        });
        // The layer is the style name's alphanumeric words, upper-cased and
        // joined with '-', behind the prefix.
        let mut expected = String::from(DISCIPLINE_PREFIX);
        let mut gap = false;
        for character in style.chars() {
            if character.is_alphanumeric() {
                if gap && expected.len() > DISCIPLINE_PREFIX.len() {
                    expected.push('-');
                }
                gap = false;
                expected.extend(character.to_uppercase());
            } else {
                gap = true;
            }
        }
        assert_eq!(
            layer_of(entity),
            expected,
            "style={style} and its layer disagree"
        );
        styles.insert(style);
    }
    assert!(
        styles.contains("Primary Piping - New"),
        "DWG-0201's 24 primary piping records must name their style; styles={styles:?}"
    );
    // An appearance name stays on PID-GEOMETRY but is still stated: `style=`
    // records what the drawing calls it, the layer decision is separate.
    assert!(
        on_layer(&doc, "PID-GEOMETRY").any(|entity| matches!(
            pid_value(entity, "style").as_deref(),
            Some("Normal" | "As Drawn" | "Dashed")
        )),
        "an appearance-named record lost its style= on PID-GEOMETRY"
    );
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
        let visible = on_line_work(&doc).count()
            + ["text", "symbol", "point-ok"]
                .iter()
                .map(|role| of_role(&doc, role).count())
                .sum::<usize>();
        assert!(visible > 0, "{name}: nothing reached a drawing role");
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
            of_role(&doc, "symbol").filter_map(drawn_box).collect();
        let labels: Vec<&EntityType> = of_role(&doc, "symbol-label").collect();
        assert!(
            !labels.is_empty() && !bodies.is_empty(),
            "{name}: every placement is named and drawn, got {} names over {} bodies",
            labels.len(),
            bodies.len()
        );

        let mut stranded: Vec<String> = Vec::new();
        for label in labels {
            let EntityType::Text(text) = label else {
                panic!("{name}: role=symbol-label is lettering only, found {label:?}");
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

/// A drawing opened away from any symbol library still draws its symbols.
///
/// A `.pid` carries a copy of every symbol definition it places -- the same
/// records the `.sym` holds, in the same symbol-local coordinates -- and each
/// placement names its copy. Until this landed a drawing without the
/// reference share drew every placement as a 1.5mm marker dot; now it draws
/// the body the file itself carries: the PT transmitter's two balloon
/// circles at their radii, no marker left, and every circle a radius the
/// library body also draws.
///
/// The two routes are not asserted equal, because they are not: the library
/// reader merges every `Sheet*` stream of a `.sym`, and `Ball Valve Type 1`
/// carries a second sheet (a 1.59mm circle and six lines) that the drawing's
/// own copy of the placed body does not include. The cache is the flavour
/// SmartPlant placed; which of the two the screen should prefer is recorded
/// as an open question in pid-parse's
/// `docs/analysis/2026-09-07-placement-tail-names-the-cached-definition.md`.
///
/// The library is found by walking up from the drawing, so the fixture is
/// copied into a fresh temp directory to take it away. Skips when
/// `PID_SYMBOL_LIBRARY` is set, since that would hand the library back.
#[test]
fn a_placement_without_a_library_body_draws_the_body_the_drawing_carries() {
    if std::env::var_os("PID_SYMBOL_LIBRARY").is_some() {
        eprintln!("skipping: PID_SYMBOL_LIBRARY is set, so no import is library-less");
        return;
    }
    let Some(source) = fixture("D06.pid") else {
        return;
    };
    let Some(with_library) = import("D06.pid") else {
        return;
    };

    let dir = std::env::temp_dir().join(format!(
        "ocs-pid-embedded-body-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let copy = dir.join("D06.pid");
    std::fs::copy(&source, &copy).expect("copy fixture");
    let without_library =
        OpenCADStudio::io::load_file(&copy).unwrap_or_else(|error| panic!("load copy: {error}"));
    let _ = std::fs::remove_dir_all(&dir);

    let radii = |doc: &CadDocument| -> Vec<f64> {
        let mut radii: Vec<f64> = of_role(doc, "symbol")
            .filter_map(|entity| match entity {
                EntityType::Circle(circle) => Some((circle.radius * 100.0).round() / 100.0),
                _ => None,
            })
            .collect();
        radii.sort_by(f64::total_cmp);
        radii
    };
    let library_radii = radii(&with_library);
    let embedded_radii = radii(&without_library);
    assert!(
        !embedded_radii.contains(&1.5),
        "a placement still fell back to the 1.5mm marker: {embedded_radii:?}"
    );
    for balloon in [6.35, 7.57] {
        assert!(
            embedded_radii.contains(&balloon),
            "the PT transmitter's {balloon}mm balloon is missing from the embedded body: {embedded_radii:?}"
        );
    }
    assert!(
        embedded_radii
            .iter()
            .all(|radius| library_radii.contains(radius)),
        "an embedded body drew a circle no library body draws: embedded {embedded_radii:?}, library {library_radii:?}"
    );
    // Six placements, each a real body of several strokes rather than one
    // marker: the count is well past six and in the library's neighbourhood.
    let embedded = of_role(&without_library, "symbol").count();
    let library = of_role(&with_library, "symbol").count();
    assert!(
        embedded >= 30 && embedded <= library,
        "the embedded bodies drew {embedded} entities against the library's {library}"
    );
}

/// The fixture imported from a fresh temp directory, where the walk up from
/// the drawing finds no symbol library. `None` when the fixture is absent.
fn import_without_library(name: &str) -> Option<CadDocument> {
    let source = fixture(name)?;
    let dir = std::env::temp_dir().join(format!(
        "ocs-pid-no-library-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let copy = dir.join(name);
    std::fs::copy(&source, &copy).expect("copy fixture");
    let doc =
        OpenCADStudio::io::load_file(&copy).unwrap_or_else(|error| panic!("load copy: {error}"));
    let _ = std::fs::remove_dir_all(&dir);
    Some(doc)
}

/// Whether an open run turns the same way at every vertex: what a sampled
/// convex curve does and a hand-drawn outline does not.
fn bends_one_way(points: &[(f64, f64)]) -> bool {
    let turns: Vec<f64> = points
        .windows(3)
        .map(|w| {
            let (ax, ay) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let (bx, by) = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            ax * by - ay * bx
        })
        .collect();
    !turns.is_empty()
        && (turns.iter().all(|turn| *turn > 0.0) || turns.iter().all(|turn| *turn < 0.0))
}

/// The curved lip of `arrester breather valve(RD)` is a B-spline record, and
/// it reaches the drawing by both routes: the `.sym` reader carries the curve
/// the library body holds, and the drawing's own cached copy of that body
/// holds the same curve. Either way it draws as one open polyline of
/// seventeen vertices -- two knot spans of eight segments, plus the end --
/// bending one way for its whole length and spanning about a millimetre and
/// a half, and the two routes put every vertex in the same place. DWG-0202
/// places the valve once, so there is one such run on `PID-SYMBOL` and no
/// other.
///
/// Before the two readers carried the record, both routes stepped over it
/// and the valve drew without its lip. Skips when `PID_SYMBOL_LIBRARY` is
/// set, since the library-less import would not be.
#[test]
fn a_symbols_bspline_lip_reaches_the_drawing_from_either_body() {
    if std::env::var_os("PID_SYMBOL_LIBRARY").is_some() {
        eprintln!("skipping: PID_SYMBOL_LIBRARY is set, so no import is library-less");
        return;
    }
    let Some(with_library) = import("DWG-0202GP06-01.pid") else {
        return;
    };
    let Some(without_library) = import_without_library("DWG-0202GP06-01.pid") else {
        return;
    };

    // The lip at scale one is 1.04 x 1.59mm; a run this small, this smooth
    // and this finely divided is the sampled curve and nothing else on the
    // symbol layer.
    const LIP_REACH_MM: f64 = 3.0;
    let lips = |doc: &CadDocument| -> Vec<Vec<(f64, f64)>> {
        of_role(doc, "symbol")
            .filter_map(|entity| match entity {
                EntityType::LwPolyline(polyline)
                    if !polyline.is_closed && polyline.vertices.len() == 17 =>
                {
                    Some(
                        polyline
                            .vertices
                            .iter()
                            .map(|v| (v.location.x, v.location.y))
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .filter(|points| {
                let (mut min_x, mut min_y, mut max_x, mut max_y) =
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for (x, y) in points {
                    min_x = min_x.min(*x);
                    min_y = min_y.min(*y);
                    max_x = max_x.max(*x);
                    max_y = max_y.max(*y);
                }
                max_x - min_x < LIP_REACH_MM && max_y - min_y < LIP_REACH_MM
            })
            .filter(|points| bends_one_way(points))
            .collect()
    };
    let library_lips = lips(&with_library);
    let embedded_lips = lips(&without_library);
    assert_eq!(
        library_lips.len(),
        1,
        "the library body draws the valve's lip once: {library_lips:?}"
    );
    assert_eq!(
        embedded_lips.len(),
        1,
        "the drawing's own body draws the valve's lip once: {embedded_lips:?}"
    );
    for (index, (from_library, from_cache)) in
        library_lips[0].iter().zip(&embedded_lips[0]).enumerate()
    {
        assert!(
            (from_library.0 - from_cache.0).abs() < 1e-6
                && (from_library.1 - from_cache.1).abs() < 1e-6,
            "vertex {index}: library {from_library:?} vs cache {from_cache:?}"
        );
    }
    // A run of about the lip's size, not a degenerate one.
    let reach = library_lips[0]
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum::<f64>();
    assert!(
        (1.5..3.0).contains(&reach),
        "the lip's sampled length is {reach:.3}mm; the curve is about 2mm long"
    );
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
/// Electric trace runs along a pipe, so its symbol has to sit on one, and the
/// line-work layers carry that pipe from the drawing's own records with no
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

    let drawing: Vec<((f64, f64), (f64, f64))> = on_line_work(&doc).flat_map(segments_of).collect();
    assert!(
        !drawing.is_empty(),
        "the drawing's own line work is what this measures against"
    );

    let mut reach: Vec<(f64, f64, f64)> = Vec::new();
    for entity in of_role(&doc, "symbol-label") {
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
    for entity in of_role(&doc, "symbol") {
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
        "a symbol body's line work should carry the four class colours the \
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
    for entity in of_role(&doc, "symbol") {
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

/// A point draws the symbol its line terminator names, at the origin and the
/// size the file states.
///
/// DWG-0201 places 75 decoded points and the screenshot marks exactly eleven
/// of them -- one per riser top plus the vessel inlet -- and nothing at the
/// other 64. The file says why: those eleven reach a `JStylePointSymbol`
/// whose group holds two real `igLine2d`, while the 53 junction points reach
/// one whose lines are zero-length and the 11 riser feet name no terminator
/// at all. So each mark is **two** strokes, a 6.708mm one from the point plus
/// a 1.044mm stub below-left of it, both read out of the group rather than
/// measured off a screen.
///
/// The screen draws them larger, but it draws that drawing's line weights
/// wide of their stated widths too, so the factor is a view-wide scale on
/// style-declared sizes rather than anything the file says about this glyph.
/// It is deliberately not applied here; see pid-parse
/// `docs/analysis/2026-08-25-a-point-draws-the-symbol-its-terminator-names.md`.
///
/// Reverting the marker build in `build_entities` leaves the point layers with
/// no line work; gating on colour instead of on the glyph puts one stroke on
/// each marked point instead of two.
#[test]
fn a_point_draws_the_symbol_its_terminator_names() {
    let Some(doc) = import("DWG-0201GP06-01.pid") else {
        return;
    };

    // The eleven marked points, from the decoded igPoint2d records.
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

    let mut points = 0usize;
    for entity in of_role(&doc, "point-ok") {
        match entity {
            EntityType::Point(_) => points += 1,
            other => panic!(
                "the marks now take their review status as their role, so the \
                 bare point role carries points only, found {other:?}"
            ),
        }
    }
    assert_eq!(points, 75, "every decoded point still takes the point role");

    let mut long_strokes = 0usize;
    let mut stubs = 0usize;
    for entity in of_role(&doc, "point-warning") {
        let EntityType::Line(line) = entity else {
            panic!("a status role is glyph strokes only, found {entity:?}");
        };
        let length = line.start.distance(&line.end);
        // The glyph's own two strokes: (0,0)->(3,6) and (-1,-2)->(-0.7,-1),
        // in millimetres.
        if (length - 6.708_204).abs() < 0.001 {
            long_strokes += 1;
        } else if (length - 1.044_175).abs() < 0.001 {
            stubs += 1;
        } else {
            panic!("a point mark is one of the glyph's two strokes, got {length:.4}mm");
        }

        // The glyph's origin is the point, so every stroke sits within the
        // glyph's own reach of one of the eleven.
        let near = marked
            .iter()
            .any(|(x, y)| (line.start.x - x).hypot(line.start.y - y) < 7.0);
        assert!(near, "a stroke belongs to a marked point, got {line:?}");
        assert_eq!(
            line.common.color,
            acadrust::types::Color::Rgb { r: 0, g: 0, b: 255 },
            "DWG-0201's marked points are trace-blue: {line:?}"
        );
    }
    assert_eq!(
        (long_strokes, stubs),
        (11, 11),
        "each of the eleven marked points draws both of its glyph's strokes, \
         and the 64 blank-symbol points draw none"
    );
}

/// A point's mark is a review status, and it lands on the layer for that
/// status.
///
/// The drawing's style librarian names the four point symbols `psOk`,
/// `psWarning`, `psError` and `psApproved`, each paired with a like-named line
/// style — so the mark is one of four review states, not decoration and not a
/// per-discipline tick. Two consequences are pinned here:
///
/// * `psOk` puts nothing on any status layer. Its glyph is two zero-length
///   lines because an item that passed has nothing to draw, so all 53 of
///   DWG-0201's junction points and all 23 of the gongyi drawing's are silent.
/// * `psError` puts nothing on one either. Both drawings that define the state
///   define it fully and no point in either is in it, which is why the layer
///   ships declared and empty rather than not at all.
///
/// The gongyi drawing is the discriminating fixture: ten of its marks are
/// `psApproved` check marks and exactly one is a `psWarning` slash, so an
/// importer that filed every mark under one status, or picked the status off
/// the glyph's shape or colour, would split them wrong.
#[test]
fn a_points_mark_files_under_the_review_status_the_drawing_names() {
    for (fixture, warning, approved) in [
        ("DWG-0201GP06-01.pid", 22usize, 0usize),
        ("DWG-0202GP06-01.pid", 10, 0),
        ("工艺管道及仪表流程-1.pid", 2, 20),
    ] {
        let Some(doc) = import(fixture) else {
            continue;
        };
        // Two strokes per mark, so these are stroke counts. By role, which
        // under the taxonomy layer mode is the `PID-POINT-*` layer the mark
        // files on and under the sheet mode is the only place the status is.
        assert_eq!(
            of_role(&doc, "point-warning").count(),
            warning,
            "{fixture}: psWarning strokes"
        );
        assert_eq!(
            of_role(&doc, "point-approved").count(),
            approved,
            "{fixture}: psApproved strokes"
        );
        assert_eq!(
            of_role(&doc, "point-error").count(),
            0,
            "{fixture}: nothing in this corpus is in the error state"
        );
        assert!(
            of_role(&doc, "point-ok").all(|e| matches!(e, EntityType::Point(_))),
            "{fixture}: every mark whose status the drawing names leaves the bare point role"
        );
    }
}

/// Named line work files under the drawing's own word for what it is.
///
/// Every `StyleCluster` opens with a style librarian holding the authored name
/// of each style the project library gave the document, and those names are a
/// classification the import cannot recover any other way: `0.350mm #800000`
/// is both `Nozzle - New` and the `Equipment - New` the nozzle sits on, and
/// `0.350mm #808000` is three separate roles of piping. So the layer is the
/// name.
///
/// Three things are pinned, because they fail differently:
///
/// * the census is exact, so a change that starts filing lines under the wrong
///   name shows up even though the entity total does not move;
/// * `PID-GEOMETRY` keeps a share of the line work rather than emptying. Those
///   are the records whose style the librarian does not name, and their
///   absence from it is the reading -- the librarian lists what came from the
///   project library, so an unnamed style is one drawn in this file. The
///   gongyi drawing is the fixture that would notice: 182 of its lines are on
///   one unnamed style;
/// * no review status becomes a discipline. All 107 `lsOk` and 18 `lsWarning`
///   records in the corpus are points, whose marks belong on `PID-POINT-*`,
///   and letting that vocabulary through would file them twice.
///
/// Nor does an appearance name. `Normal`, `As Drawn` and `Dashed` say how a
/// line is drawn, not what it is — the librarian files `Normal` under five
/// different families — so they stay on `PID-GEOMETRY` with the unnamed work.
#[test]
fn named_line_work_files_under_the_discipline_the_drawing_names() {
    // DWG-0201's 63 lines and linestrings, whole. Points and symbol bodies are
    // not here: a point's style names a review status, and a placement's body
    // stays on `PID-SYMBOL`. `PID-GEOMETRY` holds three records on this
    // drawing's own unnamed style plus the 32 whose name is an appearance.
    let expected: std::collections::BTreeMap<String, usize> = [
        // Ten otherwise-geometry strokes are authored on HiddenObjects and
        // intentionally move to the default-off PID-HIDDEN layer.
        ("PID-GEOMETRY", 25usize),
        ("PID-STYLE-CONNECT-TO-PROCESS", 3),
        ("PID-STYLE-ELECTRIC", 1),
        ("PID-STYLE-PRIMARY-PIPING-NEW", 24),
    ]
    .iter()
    .map(|(layer, count)| ((*layer).to_string(), *count))
    .collect();

    // The whole test is about where the taxonomy files the line work, so it
    // imports under that mode whatever the environment says.
    if let Some(doc) = import_in_taxonomy_mode("DWG-0201GP06-01.pid") {
        // By layer, deliberately: the ten strokes moved to `PID-HIDDEN` are
        // outside the census the way they are outside these layers.
        let mut census: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for entity in doc.entities().filter(|e| is_line_work(layer_of(e))) {
            *census.entry(layer_of(entity).to_string()).or_default() += 1;
        }
        assert_eq!(census, expected);
        for layer in census.keys() {
            assert!(
                !is_hidden(&doc, layer),
                "{layer} carries drawing content and must open visible"
            );
        }
    }

    for fixture in [
        "DWG-0201GP06-01.pid",
        "DWG-0202GP06-01.pid",
        "D06.pid",
        "工艺管道及仪表流程-1.pid",
    ] {
        let Some(doc) = import_in_taxonomy_mode(fixture) else {
            continue;
        };
        let statuses: Vec<&str> = doc
            .entities()
            .map(layer_of)
            .filter(|layer| layer.starts_with("PID-STYLE-LS") || layer.starts_with("PID-STYLE-PS"))
            .collect();
        assert!(
            statuses.is_empty(),
            "{fixture}: a review status became a discipline layer: {statuses:?}"
        );
        let appearances: Vec<&str> = doc
            .entities()
            .map(layer_of)
            .filter(|layer| {
                matches!(
                    *layer,
                    "PID-STYLE-NORMAL" | "PID-STYLE-AS-DRAWN" | "PID-STYLE-DASHED"
                )
            })
            .collect();
        assert!(
            appearances.is_empty(),
            "{fixture}: an appearance name became a discipline layer: {appearances:?}"
        );
    }

    let Some(doc) = import_in_taxonomy_mode("工艺管道及仪表流程-1.pid") else {
        return;
    };
    assert!(
        on_layer(&doc, "PID-GEOMETRY").count() >= 182,
        "the 182 lines on this drawing's own unnamed style must stay on \
         PID-GEOMETRY: being unnamed is what says they did not come from the \
         project style library"
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
    for entity in of_role(&doc, "symbol") {
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

// ── Layer mode (plan 2026-09-07, D4 / D5 / L3) ──────────────────────────────

/// The same import with the layer slot holding the authored sheet layer,
/// stated rather than read from `OCS_PID_LAYER_MODE` so both modes can run
/// in one process.
fn import_in_sheet_mode(name: &str) -> Option<CadDocument> {
    let path = fixture(name)?;
    Some(
        load_pid_with_layer_mode(&path, PidLayerMode::Sheet)
            .unwrap_or_else(|error| panic!("load {} in sheet mode: {error}", path.display())),
    )
}

/// What `pid-parse` says the document storage's sheet layers are, by name,
/// with whether the file draws each -- the yardstick for the layer table the
/// sheet mode opens with. A name two layer objects share is hidden only when
/// neither is drawn, which is how the importer merges them.
fn authored_layers(name: &str) -> Option<std::collections::BTreeMap<String, bool>> {
    let path = fixture(name)?;
    let parsed = pid_parse::PidParser::new()
        .parse_file(&path)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    let mut hidden: std::collections::BTreeMap<String, bool> = std::collections::BTreeMap::new();
    for layer in parsed.sheet_layers.get("/").into_iter().flatten() {
        let this_hidden = match layer.displayed {
            Some(displayed) => !displayed,
            None => OpenCADStudio::io::pid_view_filter::is_hidden_sheet_layer(&layer.name),
        };
        hidden
            .entry(layer.name.clone())
            .and_modify(|all| *all &= this_hidden)
            .or_insert(this_hidden);
    }
    Some(hidden)
}

/// `OCS_PID_LAYER_MODE` names two modes and defaults to the taxonomy (D5:
/// the option can be switched, the default is not flipped). An unknown value
/// is the default too, not a failed open.
#[test]
fn the_layer_mode_defaults_to_taxonomy_and_names_its_two_modes() {
    assert_eq!(PidLayerMode::default(), PidLayerMode::Taxonomy);
    assert_eq!(PidLayerMode::parse(""), Some(PidLayerMode::Taxonomy));
    assert_eq!(
        PidLayerMode::parse("taxonomy"),
        Some(PidLayerMode::Taxonomy)
    );
    assert_eq!(PidLayerMode::parse("sheet"), Some(PidLayerMode::Sheet));
    assert_eq!(PidLayerMode::parse(" Sheet "), Some(PidLayerMode::Sheet));
    assert_eq!(PidLayerMode::parse("layers"), None);
    assert_eq!(LAYER_MODE_ENV, "OCS_PID_LAYER_MODE");
    // The import every other test in this file exercises is whatever the
    // environment selects, and that is the default mode unless whoever runs
    // the suite has switched it -- the suite is meant to be run both ways.
    let Some(doc) = import("DWG-0202GP06-01.pid") else {
        return;
    };
    let mode = PidLayerMode::from_env();
    match std::env::var(LAYER_MODE_ENV) {
        Ok(value) if PidLayerMode::parse(&value).is_some() => {
            assert_eq!(Some(mode), PidLayerMode::parse(&value));
        }
        _ => assert_eq!(
            mode,
            PidLayerMode::Taxonomy,
            "unset, empty or unknown reads as the default"
        ),
    }
    match mode {
        PidLayerMode::Taxonomy => {
            assert!(doc.layers.contains("PID-GEOMETRY"));
            assert!(doc.layers.contains("PID-HIDDEN"));
            assert!(!doc.layers.contains("Default"));
        }
        PidLayerMode::Sheet => {
            assert!(doc.layers.contains("Default"));
            assert!(!doc.layers.contains("PID-HIDDEN"));
        }
    }
}

/// Under `OCS_PID_LAYER_MODE=sheet` the layer slot holds the sheet layer the
/// drawing itself filed the entity under, verbatim and with no prefix; the
/// layer table is the drawing's own list -- every sheet layer of the document
/// storage, on or off as the file draws it, drawn on or not -- plus only the
/// `PID-*` layers the importer's own entities keep. No `PID-STYLE-*` layer is
/// generated and nothing sits on `PID-HIDDEN`: a hidden sheet layer is simply
/// off, and its entities dark by their bit as well.
#[test]
fn sheet_mode_files_every_entity_under_its_authored_layer_and_declares_the_drawings_layers() {
    for name in ["DWG-0202GP06-01.pid", "DWG-0201GP06-01.pid", "D06.pid"] {
        let (Some(doc), Some(authored)) = (import_in_sheet_mode(name), authored_layers(name))
        else {
            continue;
        };
        assert!(
            !authored.is_empty(),
            "{name}: the document storage names its layers"
        );

        // The table: the drawing's own names, each in the file's state.
        for (layer, hidden) in &authored {
            let table = doc
                .layers
                .get(layer)
                .unwrap_or_else(|| panic!("{name}: sheet layer {layer:?} is not in the table"));
            assert_eq!(
                table.flags.off,
                *hidden,
                "{name}: sheet layer {layer:?} opens {} but the file draws {}",
                if table.flags.off { "off" } else { "on" },
                if *hidden { "none of it" } else { "it" }
            );
        }
        for layer in doc.layers.iter() {
            let layer_name = layer.name.as_str();
            let is_pid = layer_name.starts_with("PID-");
            let is_authored = authored.contains_key(layer_name)
                || doc
                    .entities()
                    .any(|entity| pid_value(entity, "sheet_layer").as_deref() == Some(layer_name));
            assert!(
                is_pid || is_authored || layer_name == "0",
                "{name}: layer {layer_name:?} is neither a sheet layer nor the importer's own"
            );
            assert!(
                !layer_name.starts_with(DISCIPLINE_PREFIX),
                "{name}: sheet mode generated a discipline layer {layer_name:?}"
            );
            if is_pid {
                assert!(
                    doc.entities().any(|entity| layer_of(entity) == layer_name),
                    "{name}: {layer_name:?} is declared but the importer put nothing on it"
                );
            }
        }
        assert!(
            !doc.layers.contains("PID-HIDDEN"),
            "{name}: every hidden sheet layer of this drawing has a name to be off under"
        );

        // The slot: the authored layer when there is one, the taxonomy layer
        // when the importer made the entity itself.
        let mut layered = 0usize;
        for entity in doc.entities() {
            let role = pid_value(entity, "role").unwrap_or_else(|| {
                panic!("{name}: an entity without a role on {:?}", layer_of(entity))
            });
            match pid_value(entity, "sheet_layer") {
                // A symbol's own label carries its placement's sheet layer
                // but is the importer's lettering, and keeps the layer that
                // ships it switched off.
                Some(_) if role == "symbol-label" => {
                    assert_eq!(layer_of(entity), "PID-SYMBOL-LABEL", "{name}: a symbol label");
                }
                Some(sheet_layer) => {
                    layered += 1;
                    assert_eq!(
                        layer_of(entity),
                        sheet_layer,
                        "{name}: an entity of sheet layer {sheet_layer:?} is filed on {:?}",
                        layer_of(entity)
                    );
                    assert!(
                        ROLES.contains(&role.as_str()),
                        "{name}: role {role:?} is not in the vocabulary"
                    );
                }
                None => assert_eq!(
                    role_of_layer(layer_of(entity)).map(str::to_string),
                    Some(role.clone()),
                    "{name}: an importer-made entity keeps its taxonomy layer: {:?} vs role {role:?}",
                    layer_of(entity)
                ),
            }
        }
        assert!(layered > 0, "{name}: nothing carried a sheet layer");

        // Hidden means off and dark, without `PID-HIDDEN`.
        let filter = PidViewFilter::load(&doc).expect("the import stores its filter");
        for (layer, hidden) in &authored {
            if !*hidden {
                continue;
            }
            let on_it: Vec<&EntityType> = on_sheet_layer(&doc, layer).collect();
            if on_it.is_empty() {
                continue;
            }
            assert!(
                !filter.layer_is_on(layer),
                "{name}: {layer:?} is off in the filter"
            );
            assert!(
                on_it.iter().all(|entity| entity.common().invisible),
                "{name}: entities of the hidden sheet layer {layer:?} are dark"
            );
        }
    }
}

/// The two modes disagree about the layer slot and nothing else: same
/// entities, same roles, same sheet layers in XDATA, same view filter, same
/// dark count, and the layer manager's sheet-layer view reads the same. The
/// slot is the only thing a consumer that reads layer names sees, which is
/// why it is an option; everything the import knows is in XDATA either way.
#[test]
fn the_two_layer_modes_agree_on_everything_but_the_slot() {
    let (Some(taxonomy), Some(sheet)) = (
        import_in_taxonomy_mode("DWG-0202GP06-01.pid"),
        import_in_sheet_mode("DWG-0202GP06-01.pid"),
    ) else {
        return;
    };
    assert_eq!(taxonomy.entities().count(), sheet.entities().count());
    let keys = |doc: &CadDocument| -> Vec<(Option<String>, Option<String>, Option<String>)> {
        let mut keys: Vec<_> = doc
            .entities()
            .map(|entity| {
                (
                    pid_value(entity, "role"),
                    pid_value(entity, "sheet_layer"),
                    pid_value(entity, "style"),
                )
            })
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&taxonomy), keys(&sheet));
    assert_eq!(dark(&taxonomy), dark(&sheet));
    assert_eq!(PidViewFilter::load(&taxonomy), PidViewFilter::load(&sheet));
    assert_eq!(PidViewSummary::of(&taxonomy), PidViewSummary::of(&sheet));
    // The slot itself differs: the taxonomy spells the disciplines as layers,
    // the sheet mode leaves them to `style=`.
    assert!(
        taxonomy
            .layers
            .iter()
            .any(|layer| layer.name.starts_with(DISCIPLINE_PREFIX)),
        "the taxonomy files named line work under discipline layers"
    );
    assert!(
        !sheet
            .layers
            .iter()
            .any(|layer| layer.name.starts_with(DISCIPLINE_PREFIX)),
        "the sheet mode generates no discipline layer"
    );
}

/// Switching a hidden sheet layer on under the sheet mode releases the sheet
/// layer's own layer -- what `PID-HIDDEN` is to the taxonomy mode -- and
/// nothing else; switching off leaves the table alone.
#[test]
fn sheet_mode_switching_a_hidden_sheet_layer_on_releases_its_own_layer() {
    let Some(mut doc) = import_in_sheet_mode("DWG-0202GP06-01.pid") else {
        return;
    };
    let hidden_objects = on_sheet_layer(&doc, "HiddenObjects").count();
    assert_eq!(hidden_objects, 16);
    assert!(is_hidden(&doc, "HiddenObjects"));
    assert!(on_sheet_layer(&doc, "HiddenObjects").all(|entity| layer_of(entity) == "HiddenObjects"));

    let switched = switch_sheet_layer(&mut doc, "HiddenObjects", true);
    assert_eq!(switched.entities, hidden_objects);
    assert_eq!(switched.released_layer.as_deref(), Some("HiddenObjects"));
    assert!(!is_hidden(&doc, "HiddenObjects"));
    assert!(on_sheet_layer(&doc, "HiddenObjects").all(|entity| !entity.common().invisible));

    let switched = switch_sheet_layer(&mut doc, "HiddenObjects", false);
    assert_eq!(switched.entities, hidden_objects);
    assert!(switched.released_layer.is_none());
    assert!(
        !is_hidden(&doc, "HiddenObjects"),
        "off never touches the table"
    );
    assert!(on_sheet_layer(&doc, "HiddenObjects").all(|entity| entity.common().invisible));

    // A layer the user switched off in the table is not this switch's to
    // reopen when some other sheet layer is switched on.
    doc.layers.get_mut("Default").expect("Default").flags.off = true;
    let switched = switch_sheet_layer(&mut doc, "Labels", false);
    let switched_back = switch_sheet_layer(&mut doc, "Labels", true);
    assert!(switched.released_layer.is_none() && switched_back.released_layer.is_none());
    assert!(is_hidden(&doc, "Default"));
}

/// The authored layer names are what a third-party viewer sees after Save As:
/// the slot survives DWG and DXF as a layer name, with the table's on/off state.
#[test]
fn sheet_mode_layer_names_survive_dwg_and_dxf() {
    let Some(doc) = import_in_sheet_mode("DWG-0202GP06-01.pid") else {
        return;
    };
    let expected: std::collections::BTreeMap<String, (bool, usize)> = doc
        .layers
        .iter()
        .map(|layer| {
            (
                layer.name.clone(),
                (
                    layer.flags.off,
                    doc.entities().filter(|e| layer_of(e) == layer.name).count(),
                ),
            )
        })
        .collect();
    for ext in ["dwg", "dxf"] {
        let bytes = OpenCADStudio::io::save_to_bytes(&doc, ext, doc.version)
            .unwrap_or_else(|error| panic!("save {ext}: {error}"));
        let reopened = OpenCADStudio::io::load_bytes(&format!("sheet-mode.{ext}"), bytes)
            .unwrap_or_else(|error| panic!("reopen {ext}: {error}"));
        for (layer, (off, count)) in &expected {
            let table = reopened
                .layers
                .get(layer)
                .unwrap_or_else(|| panic!("{ext}: layer {layer:?} did not survive"));
            assert_eq!(table.flags.off, *off, "{ext}: {layer:?} changed state");
            assert_eq!(
                reopened.entities().filter(|e| layer_of(e) == layer).count(),
                *count,
                "{ext}: {layer:?} lost or gained entities"
            );
        }
    }
}

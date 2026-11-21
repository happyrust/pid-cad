// Lettering: height, colour, alignment, line breaks and the text styles the drawing's typefaces need.

use super::*;

/// The character height the drawing states for one text record, if it has
/// one. Same `(stream path, graphic oid)` join as [`style_for`].
pub(super) fn height_for<'a>(
    heights: &'a pid_parse::style_link::TextHeightIndex,
    entity: &pid_parse::PidGraphicEntity,
) -> Option<&'a pid_parse::style_link::ResolvedTextHeight> {
    let stream = entity.source.stream_path.as_deref()?;
    let oid = entity.graphic_oid?;
    heights.get(&(stream.to_string(), oid))
}

/// Letter a text entity at the height its style states, in millimetres.
///
/// Only the sheet's own lettering. A symbol's internal text is drawn from the
/// `.sym` library, whose records carry no height of their own, so it keeps
/// the scaled fallback rather than borrowing a height from the label beside
/// it.
pub(super) fn apply_text_height(entity: &mut EntityType, height_mm: f64) {
    if !(height_mm.is_finite() && height_mm > 0.0) {
        return;
    }
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.height = height_mm;
        }
    }
}

/// The width factor a character style's stated character width letters at.
///
/// `width_permille` is thousandths of the face's own width. The `CPE-SQ`
/// templates letter their whole title block at `800`, and the vendor's plots
/// narrow it to match; lettered at the face's own width, the company name
/// runs over the title block's right-hand line. `0` is the face's own width
/// -- what nearly every style of the drawings' own lettering states -- and
/// `pid-plot` reads it as no statement, the same as `1000`: neither gives a
/// factor, and the entity keeps the width it was built with.
///
/// `pid-plot` rounds the fraction the way the Windows print path does, which
/// letters `800` at 0.742..0.797 depending on the height. The canvas has no
/// device font to round against, so it letters at the stated fraction.
pub(super) fn text_width_factor(width_permille: Option<u16>) -> Option<f64> {
    width_permille
        .filter(|width| *width > 0 && *width != 1000)
        .map(|width| f64::from(width) / 1000.0)
}

/// Letter a text entity at the width its character style states, as
/// [`text_width_factor`] reads it.
///
/// The anchor does not move: the canvas steps back from the hot point by the
/// run's advance measured at the width factor, so a right-aligned label
/// narrows towards its right end and a centred one towards its middle.
///
/// Scoped like [`apply_text_height`]: the sheet's own lettering on
/// [`LAYER_TEXT`]. A symbol's lettering takes its width in `letter_label`,
/// from its own record.
pub(super) fn apply_text_width(entity: &mut EntityType, factor: f64) {
    if !(factor.is_finite() && factor > 0.0) {
        return;
    }
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.width_factor = factor;
        }
    }
}

/// Letter a text entity in the colour its character style states.
///
/// Scoped the way [`apply_text_height`] is: only the sheet's own lettering on
/// [`LAYER_TEXT`]. A symbol's internal text comes from the `.sym` library and
/// states no colour of its own, and the diagnostic layers' colours *are* the
/// diagnosis -- the same reason [`apply_symbology`] refuses to paint them.
///
/// Most of a P&ID letters in black, which [`foreground_color`] stores as
/// ACI 7 -- the contrast colour the renderer flips to white on the dark
/// canvas and the plotter prints black on paper, so the letter is readable
/// both ways. Any other stated colour lands as the true colour it is.
pub(super) fn apply_text_colour(entity: &mut EntityType, [r, g, b]: [u8; 3]) {
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.common.color = foreground_color(r, g, b);
        }
    }
}

/// Letter a text entity from the side its paragraph style states.
///
/// Half the labels a P&ID reaches are centred or right-aligned, and until this
/// they all rendered from the left -- each of those runs sitting half a label
/// away from where the drawing puts it.
///
/// The insertion point does double duty in a TEXT entity: it is the run origin
/// only while the alignment is left-on-baseline, and otherwise the origin is
/// `alignment_point` instead. Setting the alignment alone would therefore move
/// the run to whatever `alignment_point` happened to hold -- which is nothing
/// -- so [`sync_text_alignment_point`] seeds it from the insertion point in
/// the same breath. The failure mode that guards against is not subtle: the
/// label lands at the origin rather than half a word off.
///
/// Scoped like [`apply_text_height`] and [`apply_text_colour`]: the sheet's
/// own lettering only. A symbol's internal text is placed by the `.sym`
/// library, which states its own alignment.
pub(super) fn apply_text_alignment(entity: &mut EntityType, alignment: TextAlignment) {
    use crate::entities::text::sync_text_alignment_point;

    if let EntityType::Text(text) = entity {
        if text.common.layer != LAYER_TEXT {
            return;
        }
        // The two enums agree on 0/1/2 by coincidence of both following the
        // DXF convention, but they are different types, so the mapping is
        // written out rather than cast.
        text.horizontal_alignment = match alignment {
            TextAlignment::Left => TextHorizontalAlignment::Left,
            TextAlignment::Center => TextHorizontalAlignment::Center,
            TextAlignment::Right => TextHorizontalAlignment::Right,
        };
        sync_text_alignment_point(text);
    }
}

/// Letter a text entity at the hot point its own record states.
///
/// Every `igTextBox` ends its placement tail with a 4-byte trailer whose last
/// byte is the label's own justification. `(code >> 4) % 3` picks the
/// horizontal point of the text box the insertion sits on — `0` left,
/// `1` centre, `2` right. It is the record's authored anchor and it overrides
/// the paragraph style's declared alignment: `L=0.8MPa ` and `1/2"` share a
/// `Center` paragraph yet letter left and centred respectively, and the
/// `+3`-shifted high nibbles (`0x38`/`0x48`/`0x58`) mark labels attached to a
/// parent symbol while folding into the same classes.
///
/// The low nibble names the line of the face the insertion sits on
/// ([`PidTextLine`]), and how far that line is above the baseline depends on
/// the face and on what its character height measures ([`pid_text_drop`]).
/// None of those lines is one the canvas aligns to -- its bottom is the
/// descender line, its middle half the height, and it has no ascender line --
/// so the run is lettered from its baseline and both points move down to it.
/// Measured against the vendor's plots of the eleven reference drawings in
/// `readings.md` §25.
///
/// Scoped like [`apply_text_alignment`]: the sheet's own lettering only.
pub(super) fn apply_text_anchor(
    entity: &mut EntityType,
    code: u8,
    lettering: Option<&pid_parse::style_link::ResolvedTextHeight>,
) {
    use crate::entities::text::sync_text_alignment_point;

    if let EntityType::Text(text) = entity {
        if text.common.layer != LAYER_TEXT {
            return;
        }
        text.horizontal_alignment = match (code >> 4) % 3 {
            1 => TextHorizontalAlignment::Center,
            2 => TextHorizontalAlignment::Right,
            _ => TextHorizontalAlignment::Left,
        };
        let line = PidTextLine::of(code, lettering.and_then(|style| style.leading));
        let below = text.height
            * pid_text_drop(
                line,
                lettering.and_then(|style| style.height_mode),
                lettering.and_then(|style| style.font_name.as_deref()),
            );
        let (sin, cos) = text.rotation.sin_cos();
        let (dx, dy) = (sin * below, -cos * below);
        text.insertion_point.x += dx;
        text.insertion_point.y += dy;
        if let Some(point) = text.alignment_point.as_mut() {
            point.x += dx;
            point.y += dy;
        }
        text.vertical_alignment = TextVerticalAlignment::Baseline;
        sync_text_alignment_point(text);
    }
}

/// The line of the face a sheet record's insertion point sits on, as the low
/// nibble of its anchor code names it: `pid-plot`'s `TextLine` for the
/// sheet's records (`model.rs`, `TextLine::from_paragraph`). It measured them
/// on the vendor's plot of drawing I: the baseline at the insertion for `0`,
/// and 1.25 to 1.38 bodies below it for `1`, 0.99 to 1.02 for `2`, 0.47 to
/// 0.50 for `5` and 0.49 to 0.69 for `8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidTextLine {
    /// The baseline: `0`.
    Bottom,
    /// The middle of the body: every nibble not named here.
    Middle,
    /// The top of the body, one character height up: `2`.
    Top,
    /// The face's ascender line: `1` and `3`.
    Ascender,
    /// Half the ascender: `8` under a paragraph whose lines are spaced by the
    /// face's ascent, which only the 10 pt Arial Narrow device tags are.
    HalfAscender,
}

impl PidTextLine {
    fn of(code: u8, leading: Option<pid_parse::style_link::LineLeading>) -> Self {
        use pid_parse::style_link::LineLeading;

        match (code & 0x0F, leading) {
            (8, Some(LineLeading::FractionAscender)) => Self::HalfAscender,
            (0, _) => Self::Bottom,
            (1 | 3, _) => Self::Ascender,
            (2, _) => Self::Top,
            _ => Self::Middle,
        }
    }
}

/// How far below the insertion point a sheet record's baseline sits, in
/// character heights: `pid-plot`'s `ScaleHeights::drop` (`write.rs`) on its
/// table of the faces the drawings letter in.
///
/// The table gives each face's cap height and ascent in a 72 px body as
/// Windows renders it (`REFERENCE_CAP_PX`); the descent is the rest of the
/// body. A character height is the cap height unless the style's height mode
/// says it is the ascent. The middle of the body is halfway between the
/// ascender and descender lines -- half a cap height up in Arial Narrow,
/// 0.539 of one in 仿宋 -- or half the cap height when the height is the
/// ascent. A face outside the table is measured as Arial, the way `pid-plot`
/// measures a typeface it does not know.
fn pid_text_drop(line: PidTextLine, height_mode: Option<u8>, font_name: Option<&str>) -> f64 {
    const SIMSUN: (f64, f64) = (49.2, 62.0);
    const FANGSONG: (f64, f64) = (48.2, 62.0);
    const ARIAL: (f64, f64) = (46.0, 58.0);
    const ARIAL_NARROW: (f64, f64) = (44.0, 58.0);

    let (cap, ascent) = match font_name {
        Some("宋体" | "新宋体" | "ËÎÌå") => SIMSUN,
        Some("仿宋" | "仿宋_GB2312") => FANGSONG,
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "simsun" | "nsimsun" => SIMSUN,
            "fangsong" => FANGSONG,
            "arial narrow" => ARIAL_NARROW,
            _ => ARIAL,
        },
        None => ARIAL,
    };
    let by_ascent = height_mode == Some(pid_parse::style_link::TEXT_HEIGHT_MODE_ASCENT);
    let descent = 72.0 - ascent;
    let above = match line {
        PidTextLine::Bottom => 0.0,
        PidTextLine::Middle if by_ascent => cap / 2.0,
        PidTextLine::Middle => (ascent - descent) / 2.0,
        PidTextLine::Top => cap,
        PidTextLine::Ascender => ascent,
        PidTextLine::HalfAscender => ascent / 2.0,
    };
    above / if by_ascent { ascent } else { cap }
}

/// Letter a record that states a text box from the edge of the box its
/// paragraph names.
///
/// A boxed `igTextBox` is laid out in its box, not on its insertion point: the
/// anchor code says where inside the box the insertion sits -- left, middle or
/// right -- which recovers the box, and the paragraph's alignment says which
/// edge of it the run hangs from, the left one when the paragraph states
/// none. The vertical stays where [`apply_text_anchor`] put it. The
/// connectors' drawing numbers are the measured case: a 45.3 mm box centred
/// on the insertion, and the vendor's plots start a left-aligned number at the
/// box's left edge and end a right-aligned one at its right edge, where
/// centring the run on the insertion put it about 9 mm off (`pid-parse`'s
/// `docs/analysis/2026-10-19-igtextbox-box-is-where-the-text-starts.md` and
/// `docs/analysis/2026-10-20-a-field-hangs-from-its-box-edge.md`).
///
/// Call it after [`apply_text_anchor`] with the same code. Scoped like it: the
/// sheet's own lettering only.
pub(super) fn apply_text_box(
    entity: &mut EntityType,
    code: u8,
    box_width_mm: Option<f64>,
    paragraph: Option<TextAlignment>,
) {
    use crate::entities::text::sync_text_alignment_point;

    let Some(width) = box_width_mm.filter(|width| width.is_finite() && *width > 0.0) else {
        return;
    };
    if let EntityType::Text(text) = entity {
        if text.common.layer != LAYER_TEXT {
            return;
        }
        let insertion_share = match (code >> 4) % 3 {
            1 => 0.5,
            2 => 1.0,
            _ => 0.0,
        };
        let (alignment, edge_share) = match paragraph.unwrap_or(TextAlignment::Left) {
            TextAlignment::Left => (TextHorizontalAlignment::Left, 0.0),
            TextAlignment::Center => (TextHorizontalAlignment::Center, 0.5),
            TextAlignment::Right => (TextHorizontalAlignment::Right, 1.0),
        };
        let back = width * (insertion_share - edge_share);
        let (sin, cos) = text.rotation.sin_cos();
        let (dx, dy) = (cos * back, sin * back);
        // Past the anchor the two points are the same hot point, and the
        // canvas letters from `alignment_point`: both move, or the run stays.
        text.insertion_point.x -= dx;
        text.insertion_point.y -= dy;
        if let Some(point) = text.alignment_point.as_mut() {
            point.x -= dx;
            point.y -= dy;
        }
        text.horizontal_alignment = alignment;
        sync_text_alignment_point(text);
    }
}

/// The code points SmartPlant may end a line of a label on.
///
/// The corpus only ever uses `U+000D`, a bare carriage return with no line
/// feed — measured over all 235 `igTextBox` records in `pid-parse`'s
/// `examples/probe_text_multiline_census`. The rest are here because splitting
/// on a break this list is missing is the failure that looks like nothing:
/// the label renders with its lines run together and no error anywhere.
pub(super) const TEXT_LINE_BREAKS: [char; 7] = [
    '\u{000A}', '\u{000D}', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
];

/// Turn a label that carries line breaks into one text entity per line,
/// stacked down the page at the pitch its paragraph style asks for.
///
/// Four labels in the reference corpus have a second line, and until this they
/// imported as a single TEXT whose `U+000D` is a code point no stroke font has
/// a glyph for: the lines ran together on one baseline with a blank gap where
/// each break should have been. A DXF TEXT entity is single-line by
/// definition, so there is nowhere to put a line break except in more entities.
///
/// **`line_spacing` is the reason this takes a parameter rather than assuming
/// single spacing.** `JStyleTextPara +66` states the multiple, and in this
/// corpus the two labels with a second line both state `1.5` while all 228
/// one-line labels state `1.0` — so the field and the line breaks agree about
/// which labels they concern. Measured in pid-parse's
/// `docs/analysis/2026-08-22-four-labels-have-a-second-line.md`. A label whose
/// paragraph states nothing usable stacks at single spacing, which is what a
/// consumer with no stated pitch has to assume anyway.
///
/// Scoped like [`apply_text_height`] and its siblings: only the sheet's own
/// lettering on [`LAYER_TEXT`]. A symbol's internal text is placed by the
/// `.sym` library, which carries no paragraph style and no breaks.
///
/// Lines are offset perpendicular to the baseline rather than straight down,
/// so a label rotated to read up the page stacks across the page — the corpus
/// letters at 0, 90 and 180 degrees, and a vertical label stacked downwards
/// would overprint itself.
pub(super) fn stack_text_lines(
    built: Vec<EntityType>,
    height_mm: f64,
    line_spacing: Option<f64>,
) -> Vec<EntityType> {
    if !built.iter().any(is_multi_line_label) {
        return built;
    }
    let pitch = height_mm * line_spacing.unwrap_or(1.0);
    let mut out = Vec::with_capacity(built.len());
    for one in built {
        if !is_multi_line_label(&one) {
            out.push(one);
            continue;
        }
        let EntityType::Text(text) = one else {
            unreachable!("is_multi_line_label admits only a Text");
        };
        // Down the page as the reader sees it: the baseline runs along
        // `rotation`, so the next line sits one pitch along its right normal.
        let (sin, cos) = text.rotation.sin_cos();
        let step = Vector3::new(pitch * sin, -pitch * cos, 0.0);
        for (index, line) in text
            .value
            .replace("\r\n", "\n")
            .split(TEXT_LINE_BREAKS)
            .enumerate()
        {
            // A blank line still occupies its slot -- the offset comes from
            // the index -- but it is not worth an entity, which is the same
            // call `build_entities` makes for an empty label.
            if line.trim().is_empty() {
                continue;
            }
            let mut one_line = text.clone();
            one_line.value = line.to_string();
            #[allow(clippy::cast_precision_loss)]
            let offset = index as f64;
            one_line.insertion_point = Vector3::new(
                text.insertion_point.x + step.x * offset,
                text.insertion_point.y + step.y * offset,
                text.insertion_point.z,
            );
            // `apply_text_alignment` re-seeds the alignment point from the
            // insertion point afterwards, so a stale one from the clone would
            // be overwritten -- but only when the paragraph states an
            // alignment. Clear it here so a label that states none cannot
            // letter every line from the first line's anchor.
            one_line.alignment_point = None;
            out.push(EntityType::Text(one_line));
        }
    }
    out
}

/// Whether this is a sheet label with more than one line in it.
pub(super) fn is_multi_line_label(entity: &EntityType) -> bool {
    let EntityType::Text(text) = entity else {
        return false;
    };
    text.common.layer == LAYER_TEXT && text.value.contains(TEXT_LINE_BREAKS)
}

/// Letter a text entity in the typeface its character style names.
///
/// The entity names a document text style rather than carrying the typeface
/// itself, which is how every other text entity in the application works --
/// see [`register_text_styles`] for what those styles hold.
///
/// Scoped like [`apply_text_height`], [`apply_text_colour`] and
/// [`apply_text_alignment`]: the sheet's own lettering only. A symbol's
/// internal text is drawn from the `.sym` library and names no typeface.
pub(super) fn apply_text_style(entity: &mut EntityType, style_name: &str) {
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.style = style_name.to_string();
        }
    }
}

/// Register one document text style per distinct typeface the drawing's
/// character styles name -- plus `symbol_fonts`, the faces the symbol bodies'
/// own lettering names (the cached bodies' `primitive_letterings`; a library
/// body resolves its lettering at placement, so a face only it uses still
/// letters in the default style) -- returning the map from typeface to the
/// style name it was given.
///
/// Pooled the way [`register_dash_linetypes`] pools patterns, and for the same
/// reason: a `.pid` names a handful of typefaces, and once each is a named
/// `TextStyle` the lettering resolves through
/// [`crate::entities::text_support::resolve_text_style`] like a style any DWG
/// shipped. That resolver prefers `true_type_font` and looks the name up among
/// the installed system fonts, which is where the vendor's name goes, verbatim.
///
/// **`height` stays 0.** A `TextStyle` height is a *fixed* height that
/// overrides the entity's own, and every one of these entities has already
/// been given the height its character style states.
///
/// A typeface the corpus cannot match -- twelve of the 381 corpus names are
/// damaged on the vendor's side, and none of them is reached by text today --
/// gets a style like any other. `resolve_text_style` finds no such font and
/// falls back, which is the intended outcome: better than writing a
/// reconstructed name nobody measured.
///
/// **`font_file` names the typeface's file** (`ARIALN.TTF`) when it is
/// installed. `true_type_font` lives only in memory -- the DWG / DXF writers
/// keep a `STYLE` record's font file and drop the family -- so a style carrying
/// the family alone saves as `txt` and reopens as a stroke font, in this editor
/// and in every other reader. A typeface not installed here keeps `txt` and is
/// named in the log, rather than a guessed file name (plan
/// 2026-09-24-pid-text-styles-keep-their-truetype-face-when-saved).
///
/// Names are assigned over a `BTreeSet`, so they are stable for a given file.
pub(super) fn register_text_styles(
    doc: &mut Document,
    text_heights: &pid_parse::style_link::TextHeightIndex,
    symbol_fonts: impl Iterator<Item = String>,
    path: &Path,
) -> HashMap<String, String> {
    let fonts: BTreeSet<String> = text_heights
        .values()
        .filter_map(|style| style.font_name.clone())
        .chain(symbol_fonts)
        .collect();
    let mut names = HashMap::new();
    for font in &fonts {
        let base = text_style_name(font);
        let mut name = base.clone();
        let mut suffix = 2;
        // The table matches case-insensitively, so asking it is also what
        // keeps two typefaces that sanitise alike from colliding.
        while doc.text_styles.contains(&name) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut style = document::tables::TextStyle::new(&name);
        style.true_type_font = font.to_string();
        match crate::scene::text::sysfont::face_file_name(font) {
            Some(file) => style.font_file = file,
            None => log::info!(
                "{}: typeface {font:?} is not installed here; its text style keeps the txt font file, so a saved copy letters in txt",
                path.display()
            ),
        }
        style.set_handle(doc.allocate_handle());
        if doc.text_styles.add(style).is_ok() {
            names.insert(font.clone(), name);
        }
    }
    names
}

/// Turn a typeface name into a symbol-table name for the style that carries it.
///
/// The name only has to be stable, unique and readable, since the typeface
/// itself travels in `true_type_font` -- so anything that is not a letter,
/// digit or underscore becomes a hyphen. That includes the space in
/// `Arial Narrow`: modern DXF permits one in a symbol name and R12 did not,
/// and `PID-Arial-Narrow` reads the same either way. CJK names come through as
/// themselves, since they are alphanumeric.
pub(super) fn text_style_name(font: &str) -> String {
    let mut body = String::new();
    for ch in font.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            body.push(ch);
        } else if !body.is_empty() && !body.ends_with('-') {
            body.push('-');
        }
    }
    // Well short of the 255-character symbol-name limit. A typeface name
    // longer than this is damage rather than a name, and the caller's
    // uniqueness loop numbers apart any two that truncate alike.
    let body: String = body.chars().take(64).collect();
    let body = body.trim_end_matches('-');
    if body.is_empty() {
        return "PID-FONT".to_string();
    }
    format!("PID-{body}")
}

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

/// Letter a text entity in the colour its character style states.
///
/// Scoped the way [`apply_text_height`] is: only the sheet's own lettering on
/// [`LAYER_TEXT`]. A symbol's internal text comes from the `.sym` library and
/// states no colour of its own, and the diagnostic layers' colours *are* the
/// diagnosis -- the same reason [`apply_symbology`] refuses to paint them.
///
/// Most of a P&ID letters in black, which on the editor's dark background the
/// renderer flips to white (`scene::view::render::adapt_to_bg`) exactly as it
/// already does for the drawing's black line work. So this is visible as
/// colour where the drawing states one, and as no change at all where it
/// states the black that most lettering uses.
pub(super) fn apply_text_colour(entity: &mut EntityType, [r, g, b]: [u8; 3]) {
    if let EntityType::Text(text) = entity {
        if text.common.layer == LAYER_TEXT {
            text.common.color = Color::from_rgb(r, g, b);
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
/// character styles name, returning the map from typeface to the style name it
/// was given.
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
/// Names are assigned over a `BTreeSet`, so they are stable for a given file.
pub(super) fn register_text_styles(
    doc: &mut CadDocument,
    text_heights: &pid_parse::style_link::TextHeightIndex,
) -> HashMap<String, String> {
    let fonts: BTreeSet<&str> = text_heights
        .values()
        .filter_map(|style| style.font_name.as_deref())
        .collect();
    let mut names = HashMap::new();
    for font in fonts {
        let base = text_style_name(font);
        let mut name = base.clone();
        let mut suffix = 2;
        // The table matches case-insensitively, so asking it is also what
        // keeps two typefaces that sanitise alike from colliding.
        while doc.text_styles.contains(&name) {
            name = format!("{base}-{suffix}");
            suffix += 1;
        }
        let mut style = acadrust::tables::TextStyle::new(&name);
        style.true_type_font = font.to_string();
        style.set_handle(doc.allocate_handle());
        if doc.text_styles.add(style).is_ok() {
            names.insert(font.to_string(), name);
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

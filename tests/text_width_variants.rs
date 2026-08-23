//! A width variant has to letter at its own widths, not at its family's.
//!
//! `fontdb` keys a face by its *typographic* family, and that name
//! deliberately merges a family's width variants: Windows' Arial Narrow
//! reports family `Arial` and is told apart from Arial only by
//! `stretch = Condensed`. Every lookup in the text stack used to carry the
//! family name on its own, so a label whose style named `Arial Narrow` was
//! lettered with Arial's glyphs — the right letters, about 1.2x too wide.
//! Found by measuring the screen, in
//! `docs/analysis/2026-08-22-screen-vs-assertion-crosscheck.md`.
//!
//! This measures the same thing the eye did, through the same entry the
//! viewport uses (`lff::tessellate_text_ex`), so a regression anywhere along
//! `Face::resolve` → `sysfont` → cosmic-text turns it red instead of waiting
//! for someone to measure another screenshot.
//!
//! Which fonts count as width variants is worked out here from the font files
//! rather than asked of `sysfont`: if the recovery under test stopped finding
//! Arial Narrow, asking it would leave this with nothing to check and it would
//! pass in silence. The check is written against whatever the machine has
//! installed rather than Arial Narrow specifically, and says out loud when it
//! skipped and why.

use std::collections::BTreeMap;

use fontdb::{Stretch, Style, Weight};

use OpenCADStudio::scene::text::font_face::Face;
use OpenCADStudio::scene::text::{lff, sysfont, ttf_glyph};

/// Upright Latin only: every text font covers it, so no character in it
/// reaches for a fallback face whose widths would drown out the one being
/// measured.
const SAMPLE: &str = "HXOnoe0123";

/// Cap height the run is tessellated at. Every ratio below is taken against
/// the run's own measured ink height, so this only sets the resolution.
const HEIGHT: f32 = 100.0;

/// A face installed under a name the family index cannot express, read
/// straight off the font files: `(typed name, family fontdb files it under,
/// stretch)`.
///
/// Deliberately independent of `sysfont`'s own recovery — this is the oracle
/// the recovery is checked against.
fn installed_width_variants() -> Vec<(String, String, Stretch)> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut found: BTreeMap<String, (String, String, Stretch)> = BTreeMap::new();
    for face in db.faces() {
        if face.stretch == Stretch::Normal
            || face.weight != Weight::NORMAL
            || face.style != Style::Normal
        {
            continue;
        }
        let Some(filed_under) = face.families.first().map(|(name, _)| name.clone()) else {
            continue;
        };
        // OpenType name ID 1 is the family Windows shows in a font menu;
        // fontdb prefers name ID 16, which is where the merge happens.
        let legacy = db.with_face_data(face.id, |data, index| {
            let parsed = ttf_parser::Face::parse(data, index).ok()?;
            parsed
                .names()
                .into_iter()
                .filter(|name| name.name_id == ttf_parser::name_id::FAMILY && name.is_unicode())
                .find_map(|name| name.to_string())
        });
        let Some(Some(legacy)) = legacy else { continue };
        let legacy = legacy.trim().to_string();
        if legacy.is_empty() || legacy.eq_ignore_ascii_case(&filed_under) {
            continue;
        }
        found
            .entry(legacy.to_lowercase())
            .or_insert((legacy, filed_under, face.stretch));
    }
    found.into_values().collect()
}

/// Ink bounding box of a tessellated run, as `(width, height)`.
fn ink(font: &str, text: &str) -> Option<(f32, f32)> {
    let (strokes, _fill) = lff::tessellate_text_ex([0.0, 0.0], HEIGHT, 0.0, 1.0, 0.0, font, text);
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in strokes.iter().flatten() {
        min[0] = min[0].min(point[0]);
        min[1] = min[1].min(point[1]);
        max[0] = max[0].max(point[0]);
        max[1] = max[1].max(point[1]);
    }
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    (width.is_finite() && height > 0.0).then_some((width, height))
}

/// Whether the face draws every character of the sample itself. One that has
/// to borrow a glyph from a fallback font would be measured partly at that
/// font's widths, which proves nothing either way.
fn covers_sample(font: &str) -> bool {
    SAMPLE
        .chars()
        .all(|ch| ttf_glyph::glyph(font, ch).is_some())
}

#[test]
fn a_width_variant_letters_at_its_own_widths() {
    let variants = installed_width_variants();
    if variants.is_empty() {
        eprintln!(
            "SKIPPED: no width-variant font installed — this needs a family whose condensed \
             or expanded face fontdb files under the regular family (Arial Narrow is the \
             usual one on Windows). Nothing to measure."
        );
        return;
    }

    let mut measured = 0usize;
    for (name, base_family, stretch) in &variants {
        // Resolution is checked before anything is allowed to skip: a name
        // that resolves to nothing, or to the family's regular face, *is* the
        // regression, and a guard that reads that as "can't compare" would
        // turn the whole check into a silent pass.
        assert!(
            matches!(Face::resolve(name), Face::Ttf { .. }),
            "{name} is installed but Face::resolve sent it to a stroke font"
        );
        let resolved = sysfont::face_attributes(name)
            .unwrap_or_else(|| panic!("{name} is installed but resolves to no face"));
        assert_eq!(
            resolved.stretch, *stretch,
            "{name} resolved to a {:?} face; the installed one is {stretch:?}",
            resolved.stretch
        );

        if !covers_sample(name) || !covers_sample(base_family) {
            eprintln!(
                "skipping {name}: it or {base_family} does not cover {SAMPLE:?}, so the two \
                 runs would not be comparing the same glyphs"
            );
            continue;
        }

        let Some((variant_w, variant_h)) = ink(name, SAMPLE) else {
            eprintln!("skipping {name}: tessellated to nothing");
            continue;
        };
        let Some((base_w, base_h)) = ink(base_family, SAMPLE) else {
            eprintln!("skipping {name}: {base_family} tessellated to nothing");
            continue;
        };

        let variant_ratio = variant_w / variant_h;
        let base_ratio = base_w / base_h;
        eprintln!(
            "{name} ({stretch:?}) w/h {variant_ratio:.3} vs {base_family} w/h {base_ratio:.3}"
        );

        // Equal ratios are the regression's signature: the variant's name
        // resolved to its family's regular face, so both runs drew the same
        // glyphs at the same widths.
        assert!(
            (variant_ratio - base_ratio).abs() > 1.0e-3,
            "{name} lettered exactly as wide as {base_family} (w/h {variant_ratio:.3}); \
             its {stretch:?} face was not used"
        );
        if *stretch < Stretch::Normal {
            assert!(
                variant_ratio < base_ratio,
                "{name} is {stretch:?}, so it must letter narrower than {base_family} — \
                 got w/h {variant_ratio:.3} vs {base_ratio:.3}"
            );
        } else {
            assert!(
                variant_ratio > base_ratio,
                "{name} is {stretch:?}, so it must letter wider than {base_family} — \
                 got w/h {variant_ratio:.3} vs {base_ratio:.3}"
            );
        }
        measured += 1;
    }

    if measured == 0 {
        eprintln!(
            "SKIPPED: {} width variant(s) installed, but none could be compared against \
             their family over {SAMPLE:?}",
            variants.len()
        );
    }
}

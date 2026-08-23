// System TrueType/OpenType font discovery.
//
// Wraps a `fontdb` database loaded with the user's installed system fonts so
// the rest of the app can (a) list available font families for the text-style
// picker and (b) borrow a face's raw bytes to extract glyph outlines (see the
// TTF glyph engine). LFF stroke fonts stay separate — this is purely the
// TrueType side of the renderer.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Everything a font matcher needs to land on one specific face: the family
/// fontdb indexes it under, plus the axis values that pick that face out of
/// the family.
///
/// The two have to travel together because fontdb keys a face by its
/// *typographic* family (OpenType name ID 16), and that name deliberately
/// merges a family's width variants: Windows' Arial Narrow reports family
/// `Arial` and is told apart from Arial only by `stretch = Condensed`. Hand a
/// matcher the family name on its own — cosmic-text, or fontdb's own `Query`
/// — and Arial Narrow silently letters in Arial, at Arial's widths.
///
/// An ordinary family answers with the CSS defaults, so passing the whole
/// thing through is a no-op for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRequest {
    pub family: String,
    pub stretch: fontdb::Stretch,
    pub weight: fontdb::Weight,
    pub style: fontdb::Style,
}

/// A face whose own family name the family index cannot express, put back in
/// front of the name a user types for it.
struct Recovered {
    /// [`fold_name`] of `name`, so a drawing's spelling can meet the font's.
    key: String,
    /// The legacy family (name ID 1), e.g. `Arial Narrow`.
    name: String,
    id: fontdb::ID,
}

struct SysFonts {
    db: fontdb::Database,
    /// Sorted, de-duplicated family names for the picker.
    families: Vec<String>,
    /// Faces fontdb merged into a wider family, reachable again by name.
    recovered: Vec<Recovered>,
}

static FONTS: OnceLock<SysFonts> = OnceLock::new();

/// Case- and separator-insensitive key for a font name: `Arial Narrow`,
/// `arial narrow` and `ArialNarrow` all fold to the same thing.
fn fold_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The face's legacy family (name ID 1) when it differs from the typographic
/// family fontdb filed it under — i.e. the name that got lost in the merge.
fn legacy_family(db: &fontdb::Database, face: &fontdb::FaceInfo) -> Option<String> {
    let filed_under = face.families.first().map(|(name, _)| name.as_str())?;
    let legacy = db.with_face_data(face.id, |data, index| {
        let parsed = ttf_parser::Face::parse(data, index).ok()?;
        parsed
            .names()
            .into_iter()
            .filter(|name| name.name_id == ttf_parser::name_id::FAMILY && name.is_unicode())
            .find_map(|name| name.to_string())
    })??;
    let legacy = legacy.trim().to_string();
    (!legacy.is_empty() && !legacy.eq_ignore_ascii_case(filed_under)).then_some(legacy)
}

/// Recover the faces the family index cannot express.
///
/// Only a face whose stretch is not `Normal` can be hidden this way — every
/// other axis (weight, slant) is something fontdb's own query already selects
/// within a family. Where several faces share one recovered name (Arial
/// Narrow ships regular, bold, italic and bold-italic, all four filed under
/// `Arial`), the upright regular one is what the bare name means.
///
/// The name table is only re-read for those faces, which is a handful on a
/// typical machine — the family scan itself stays fontdb's.
fn recover_hidden_faces(db: &fontdb::Database) -> Vec<Recovered> {
    let mut best: HashMap<String, (Recovered, bool)> = HashMap::new();
    for face in db.faces() {
        if face.stretch == fontdb::Stretch::Normal {
            continue;
        }
        let Some(name) = legacy_family(db, face) else {
            continue;
        };
        let regular = face.weight == fontdb::Weight::NORMAL && face.style == fontdb::Style::Normal;
        let key = fold_name(&name);
        if matches!(best.get(&key), Some((_, held_by_regular)) if *held_by_regular || !regular) {
            continue;
        }
        best.insert(
            key.clone(),
            (
                Recovered {
                    key,
                    name,
                    id: face.id,
                },
                regular,
            ),
        );
    }
    let mut recovered: Vec<Recovered> = best.into_values().map(|(entry, _)| entry).collect();
    recovered.sort_by(|a, b| a.key.cmp(&b.key));
    recovered
}

fn fonts() -> &'static SysFonts {
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        let recovered = recover_hidden_faces(&db);

        let mut families: Vec<String> = db
            .faces()
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .chain(recovered.iter().map(|entry| entry.name.clone()))
            .collect();
        families.sort_by_key(|n| n.to_lowercase());
        families.dedup();

        SysFonts {
            db,
            families,
            recovered,
        }
    })
}

/// All installed system font families, sorted case-insensitively, de-duped.
///
/// Includes the width variants fontdb merged into a wider family, so a name a
/// drawing can legitimately state is one the picker can offer.
pub fn families() -> &'static [String] {
    &fonts().families
}

/// Resolve a requested family name to the canonical installed system family name (with exact case).
///
/// Memoised process-wide: `resolve_font` calls this once per word on the MTEXT
/// measure hot path for inline-`\f` TTF runs, and `Face::resolve` re-runs it
/// immediately after — the underlying fontdb query plus linear family scans are
/// not free. The cache keys on the raw request string; results are stable for
/// the process lifetime (the font DB is loaded once via `OnceLock`).
pub fn canonical_family_name(family: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap().get(family) {
        return hit.clone();
    }
    let resolved = canonical_family_name_uncached(family);
    cache
        .lock()
        .unwrap()
        .insert(family.to_string(), resolved.clone());
    resolved
}

fn canonical_family_name_uncached(family: &str) -> Option<String> {
    let db = &fonts().db;
    
    // 1. Try exact match first
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        ..Default::default()
    };
    if db.query(&query).is_some() {
        if let Some(canonical) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
            return Some(canonical.clone());
        }
        return Some(family.to_string());
    }
    
    // 2. Try case-insensitive match on the families we have
    if let Some(matched) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
        return Some(matched.clone());
    }
    
    // 3. Match common prefixes / variations
    let family_lower = family.to_lowercase();
    let alias = match family_lower.as_str() {
        "arialn" => Some("Arial Narrow"),
        "gothic" => Some("Century Gothic"),
        "times" => Some("Times New Roman"),
        "cour" => Some("Courier New"),
        _ => None,
    };
    
    if let Some(alias_name) = alias {
        if let Some(matched) = fonts().families.iter().find(|&f| f.eq_ignore_ascii_case(alias_name)) {
            return Some(matched.clone());
        }
    }
    
    // 4. Try matching prefix/subset case-insensitively. Require at least 3
    //    chars so a 1–2 letter request can't grab an arbitrary family by the
    //    first iteration order. Iterating sorted keeps the pick deterministic.
    if family_lower.len() >= 3 {
        let mut candidates: Vec<&String> = fonts()
            .families
            .iter()
            .filter(|&f| {
                let f_low = f.to_lowercase();
                f_low.starts_with(&family_lower) || family_lower.starts_with(&f_low)
            })
            .collect();
        candidates.sort();
        if let Some(matched) = candidates.first() {
            return Some((*matched).clone());
        }
    }

    None
}

/// Resolve a family name to a concrete face id (regular weight/style).
///
/// The family index answers first, so an ordinary name resolves exactly as it
/// always did. A [`Recovered`] name reaches nothing there — fontdb filed that
/// face under a wider family — so it is looked up by name afterwards.
fn face_id(family: &str) -> Option<fontdb::ID> {
    let sys = fonts();
    let canonical = canonical_family_name(family)?;
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(&canonical)],
        ..Default::default()
    };
    if let Some(id) = sys.db.query(&query) {
        return Some(id);
    }
    let (canonical_key, requested_key) = (fold_name(&canonical), fold_name(family));
    sys.recovered
        .iter()
        .find(|entry| entry.key == canonical_key || entry.key == requested_key)
        .map(|entry| entry.id)
}

/// Everything another font matcher needs to land on the face `name` means.
///
/// Hand the family name on its own to cosmic-text or to `fontdb::Query` and a
/// width variant collapses to its family's regular face; the stretch here is
/// what keeps that from happening. An ordinary family yields the CSS
/// defaults, so passing the whole thing through is a no-op for it.
pub fn face_attributes(name: &str) -> Option<FaceRequest> {
    let sys = fonts();
    let face = sys.db.face(face_id(name)?)?;
    Some(FaceRequest {
        family: face.families.first().map(|(name, _)| name.clone())?,
        stretch: face.stretch,
        weight: face.weight,
        style: face.style,
    })
}

/// Borrow the raw face bytes for `family` and run `f` over them. The byte slice
/// is only valid inside the closure, so callers extract everything they need
/// (e.g. flattened glyph outlines) before returning. `index` is the face index
/// within a TrueType collection. Returns `None` if the family is unknown.
pub fn with_face_data<T>(family: &str, f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
    let id = face_id(family)?;
    fonts().db.with_face_data(id, f)
}

/// Whether `family` matches an installed system font (case-insensitive via
/// fontdb's own matching).
pub fn has_family(family: &str) -> bool {
    face_id(family).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recovered name must land on the face it names, not on the regular
    /// face of the family fontdb filed that face under. Drop the `recovered`
    /// arm of `face_id` and this asks the family index alone, which for
    /// `Arial Narrow` answers Arial.
    #[test]
    fn a_recovered_name_resolves_to_its_own_face() {
        let recovered = &fonts().recovered;
        if recovered.is_empty() {
            eprintln!(
                "SKIPPED: no width-variant font installed (a family whose condensed or \
                 expanded face fontdb files under the regular family, e.g. Arial Narrow); \
                 nothing for this check to resolve"
            );
            return;
        }
        for entry in recovered {
            let got = face_attributes(&entry.name)
                .unwrap_or_else(|| panic!("{} is installed but resolves to no face", entry.name));
            assert_ne!(
                got.stretch,
                fontdb::Stretch::Normal,
                "{} resolved to a normal-width face; its own face is a width variant",
                entry.name
            );
            assert!(
                families().iter().any(|name| name == &entry.name),
                "{} is resolvable, so the picker has to be able to offer it",
                entry.name
            );
        }
    }

    /// Recovering the hidden names must not shadow the family index: every
    /// name that index can answer still resolves to the face it answered
    /// with. (A family can legitimately be condensed throughout — Agency FB
    /// is — so what is pinned here is agreement with fontdb, not `Normal`.)
    #[test]
    fn an_ordinary_family_still_resolves_through_the_family_index() {
        let sys = fonts();
        let mut checked = 0usize;
        for name in families() {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(name)],
                ..Default::default()
            };
            let Some(expected) = sys.db.query(&query) else {
                // A recovered name reaches nothing here; that is the point.
                continue;
            };
            assert_eq!(
                face_id(name),
                Some(expected),
                "{name} is in the family index but resolved elsewhere"
            );
            checked += 1;
        }
        if checked == 0 {
            eprintln!("SKIPPED: no system fonts installed");
        }
    }

    #[test]
    fn folding_ignores_case_and_separators() {
        assert_eq!(fold_name("Arial Narrow"), fold_name("arialnarrow"));
        assert_eq!(fold_name("Arial-Narrow"), fold_name("ARIAL NARROW"));
        assert_ne!(fold_name("Arial Narrow"), fold_name("Arial"));
    }
}


// System TrueType/OpenType font discovery.
//
// Wraps a `fontdb` database loaded with the user's installed system fonts so
// the rest of the app can (a) list available font families for the text-style
// picker and (b) borrow a face's raw bytes to extract glyph outlines (see the
// TTF glyph engine). LFF stroke fonts stay separate — this is purely the
// TrueType side of the renderer.
//
// The web build has no installed fonts to scan (`load_system_fonts` finds
// nothing on wasm), so the database is instead populated at runtime with the
// Noto subsets `web_font` fetches (#141): each completed fetch is registered
// via [`register_web_font`], and a static alias table maps the common
// office/CAD families (Arial, Cambria, …) onto that Noto set so styles naming
// them resolve to filled TTF glyphs instead of falling back to LFF strokes.

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
    db: Mutex<fontdb::Database>,
    /// Sorted, de-duplicated family names for the picker and name resolution.
    families: Mutex<Vec<String>>,
    /// Faces fontdb merged into a wider family, reachable again by name.
    recovered: Vec<Recovered>,
    /// Installed face file name (lower-case) → the family that face holds.
    /// A drawing references a TrueType style by *file* name, so this is what
    /// makes such a style resolve — see [`family_for_reference`]. Files a host
    /// registers at run time join it ([`register_font_file`]).
    by_file: Mutex<HashMap<String, String>>,
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
        // No system fonts to scan on the web — the database is filled from the
        // fetched Noto subsets instead (see `register_web_font`).
        #[cfg(not(target_arch = "wasm32"))]
        let db = {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            // TrueType files dropped into the Fonts folder resolve like
            // installed ones, as the folder's SHX fonts already do (#925).
            if let Some(dir) = crate::io::font_repo::fonts_dir() {
                db.load_fonts_dir(dir);
            }
            db
        };
        #[cfg(target_arch = "wasm32")]
        let db = fontdb::Database::new();

        let mut by_file: HashMap<String, String> = HashMap::new();
        for face in db.faces() {
            let Some((family, _)) = face.families.first() else {
                continue;
            };
            // Remember every face file: a DWG style names the file, and the
            // family inside it is usually a different string. A bold or italic
            // file (`GOTHICB.TTF`) names that face of its family.
            if let fontdb::Source::File(path) = &face.source {
                if let Some(name) = path.file_name() {
                    by_file
                        .entry(name.to_string_lossy().to_ascii_lowercase())
                        .or_insert_with(|| face_family(face, family));
                }
            }
        }
        let recovered = recover_hidden_faces(&db);
        let families = picker_names(&db, &recovered);
        SysFonts {
            db: Mutex::new(db),
            families: Mutex::new(families),
            recovered,
            by_file: Mutex::new(by_file),
        }
    })
}

/// `family` naming `face` itself: with the [`STYLE_SEP`] style of a bold or
/// italic face.
fn face_family(face: &fontdb::FaceInfo, family: &str) -> String {
    let b = if face.weight.0 >= fontdb::Weight::SEMIBOLD.0 { "b" } else { "" };
    let i = if face.style != fontdb::Style::Normal { "i" } else { "" };
    if b.is_empty() && i.is_empty() {
        family.to_string()
    } else {
        format!("{family}{STYLE_SEP}{b}{i}")
    }
}

/// Sorted, de-duplicated family names of every face currently in `db`.
fn family_names(db: &fontdb::Database) -> Vec<String> {
    let mut families: Vec<String> = db
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    families.sort_by_key(|n| n.to_lowercase());
    families.dedup();
    families
}

/// The picker's family list: every family name fontdb knows plus the recovered
/// width-variant names — `family_names` alone files "Arial Narrow" under
/// "Arial", so rebuilding the list without these drops the recovered names the
/// next time a font registers.
fn picker_names(db: &fontdb::Database, recovered: &[Recovered]) -> Vec<String> {
    let mut families = family_names(db);
    families.extend(recovered.iter().map(|entry| entry.name.clone()));
    families.sort_by_key(|n| n.to_lowercase());
    families.dedup();
    families
}

/// All known system font families, sorted case-insensitively, de-duped. On the
/// web this is the fetched Noto set, so it grows as fetches complete.
pub fn families() -> Vec<String> {
    fonts().families.lock().unwrap().clone()
}

/// Sans families aliased to the Noto stand-in on the web.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
static SANS_ALIASES: &[&str] = &[
    "Arial",
    "Calibri",
    "DejaVu Sans",
    "Helvetica",
    "Liberation Sans",
    "Segoe UI",
    "Tahoma",
    "Verdana",
];

/// Serif families aliased on the web; they prefer Noto Serif when such a
/// subset is ever fetched and otherwise degrade to Noto Sans.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
static SERIF_ALIASES: &[&str] = &[
    "Book Antiqua",
    "Cambria",
    "Constantia",
    "DejaVu Serif",
    "Garamond",
    "Georgia",
    "Liberation Serif",
    "Palatino Linotype",
    "Times New Roman",
];

/// Preferred stand-in families for an alias request (lowercase), in order.
/// `None` when the name is not a known alias. Pure table lookup, so it is unit
/// tested on every platform even though only the web resolution path uses it.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn alias_targets(family_lower: &str) -> Option<&'static [&'static str]> {
    const SANS: &[&str] = &["Noto Sans"];
    const SERIF: &[&str] = &["Noto Serif", "Noto Sans"];
    if SANS_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(family_lower))
    {
        Some(SANS)
    } else if SERIF_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(family_lower))
    {
        Some(SERIF)
    } else {
        None
    }
}

/// First alias stand-in that `known` reports as available. `known` receives
/// candidate family names; on the live path it scans the registered families.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn alias_resolution(family_lower: &str, known: impl Fn(&str) -> bool) -> Option<&'static str> {
    alias_targets(family_lower)?
        .iter()
        .copied()
        .find(|t| known(t))
}

/// Append the alias names to a family list when the sans stand-in is present
/// (every fetched subset carries the Latin face, so one successful fetch makes
/// all aliases resolvable). Pure, so the picker behaviour is unit testable.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn merge_web_aliases(families: &mut Vec<String>) {
    if !families.iter().any(|f| f.eq_ignore_ascii_case("Noto Sans")) {
        return;
    }
    families.extend(
        SANS_ALIASES
            .iter()
            .chain(SERIF_ALIASES.iter())
            .map(|name| name.to_string()),
    );
    families.sort_by_key(|n| n.to_lowercase());
    families.dedup();
}

/// Families offered by the text-style picker's "System fonts" list: installed
/// fonts natively; on the web the fetched Noto families plus the common
/// office/CAD family names that stand in via them.
pub fn picker_families() -> Vec<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let mut families = families();
        merge_web_aliases(&mut families);
        families
    }
    #[cfg(not(target_arch = "wasm32"))]
    families()
}

fn canonical_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve a requested family name to the canonical installed system family name (with exact case).
///
/// Memoised: `resolve_font` calls this once per word on the MTEXT
/// measure hot path for inline-`\f` TTF runs, and `Face::resolve` re-runs it
/// immediately after — the underlying fontdb query plus linear family scans are
/// not free. The cache keys on the raw request string. Results are stable for
/// the process lifetime on desktop; on the web the cache is cleared whenever a
/// fetched Noto subset joins the database, because earlier `None` resolutions
/// (font still in flight) must not stick.
pub fn canonical_family_name(family: &str) -> Option<String> {
    // A styled name keeps its style on the canonical family.
    if let Some((base, style)) = family.split_once(STYLE_SEP) {
        return canonical_family_name(base).map(|c| format!("{c}{STYLE_SEP}{style}"));
    }
    if let Some(hit) = canonical_cache().lock().unwrap().get(family) {
        return hit.clone();
    }
    let resolved = canonical_family_name_uncached(family);
    canonical_cache()
        .lock()
        .unwrap()
        .insert(family.to_string(), resolved.clone());
    resolved
}

fn canonical_family_name_uncached(family: &str) -> Option<String> {
    // Snapshot so the families lock is never held across the db lock below
    // (`register_web_font` takes them in the opposite order, scoped).
    let families = fonts().families.lock().unwrap().clone();

    // 1. Try exact match first
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        ..Default::default()
    };
    if fonts().db.lock().unwrap().query(&query).is_some() {
        if let Some(canonical) = families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
            return Some(canonical.clone());
        }
        return Some(family.to_string());
    }

    // 2. Try case-insensitive match on the families we have
    if let Some(matched) = families.iter().find(|&f| f.eq_ignore_ascii_case(family)) {
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
        if let Some(matched) = families
            .iter()
            .find(|&f| f.eq_ignore_ascii_case(alias_name))
        {
            return Some(matched.clone());
        }
    }

    // 4. Web: the common office/CAD families the client cannot have installed
    //    stand in via the fetched Noto set (#141). Runs before the fuzzy prefix
    //    match so "Arial" can never drift onto an arbitrary Noto name.
    #[cfg(target_arch = "wasm32")]
    if let Some(target) = alias_resolution(&family_lower, |name| {
        families.iter().any(|f| f.eq_ignore_ascii_case(name))
    }) {
        if let Some(matched) = families.iter().find(|&f| f.eq_ignore_ascii_case(target)) {
            return Some(matched.clone());
        }
    }

    // 5. Try matching prefix/subset case-insensitively. Require at least 3
    //    chars so a 1–2 letter request can't grab an arbitrary family by the
    //    first iteration order. Iterating sorted keeps the pick deterministic.
    if family_lower.len() >= 3 {
        let mut candidates: Vec<&String> = families
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

/// Separator between a family and the face of it a [`styled_family`] name
/// asks for; a control character no family name contains.
pub const STYLE_SEP: char = '\u{2}';

/// A [`styled_family`] name split into the family and whether it asks for
/// the bold and the italic face. A plain family asks for neither.
pub fn split_style(name: &str) -> (&str, bool, bool) {
    match name.split_once(STYLE_SEP) {
        Some((family, style)) => (family, style.contains('b'), style.contains('i')),
        None => (name, false, false),
    }
}

/// The family name that draws `family`'s own bold and/or italic face — a
/// separate face, often a separate file (`GOTHICB.TTF`) — wherever a family
/// name goes (glyph outlines, shaping, PDF embedding). Only what the
/// installed faces really have is asked for; `None` when they have neither,
/// so the caller keeps the plain family.
pub fn styled_family(family: &str, bold: bool, italic: bool) -> Option<String> {
    let (base, was_bold, was_italic) = split_style(family);
    let (bold, italic) = (bold || was_bold, italic || was_italic);
    if !bold && !italic {
        return None;
    }
    let canonical = canonical_family_name(base)?;
    let db = fonts().db.lock().unwrap();
    let face = db.face(db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(&canonical)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
        ..Default::default()
    })?)?;
    let bold = bold && face.weight.0 >= fontdb::Weight::SEMIBOLD.0;
    let italic = italic && face.style != fontdb::Style::Normal;
    (bold || italic).then(|| {
        let b = if bold { "b" } else { "" };
        let i = if italic { "i" } else { "" };
        format!("{canonical}{STYLE_SEP}{b}{i}")
    })
}

/// Resolve a family name to a concrete face id (regular weight/style, or
/// the face a [`styled_family`] name asks for).
fn face_id(family: &str) -> Option<fontdb::ID> {
    face_id_weighted(family, fontdb::Weight::NORMAL)
}

/// Resolve a family name to a concrete face id at the requested weight
/// (e.g. bold runs). Falls back through fontdb's fuzzy matching, so a
/// family without a bold face still resolves — to its regular face.
fn face_id_weighted(family: &str, weight: fontdb::Weight) -> Option<fontdb::ID> {
    let (family, bold, italic) = split_style(family);
    let canonical = canonical_family_name(family)?;
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(&canonical)],
        weight: if bold { fontdb::Weight::BOLD } else { weight },
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
        ..Default::default()
    };
    let sys = fonts();
    if let Some(id) = sys.db.lock().unwrap().query(&query) {
        return Some(id);
    }
    // A width variant is filed under the wider family, so the index above
    // misses the name a drawing actually states (`Arial Narrow`).
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
    let id = face_id(name)?;
    let db = fonts().db.lock().unwrap();
    let face = db.face(id)?;
    Some(FaceRequest {
        family: face.families.first().map(|(name, _)| name.clone())?,
        stretch: face.stretch,
        weight: face.weight,
        style: face.style,
    })
}

/// The file name (lower case) of the installed face of exactly `family` —
/// no stand-in family — in the bold and italic asked for, or `None`.
pub fn face_file(family: &str, bold: bool, italic: bool) -> Option<String> {
    let db = fonts().db.lock().unwrap();
    let id = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
        ..Default::default()
    })?;
    match &db.face(id)?.source {
        fontdb::Source::File(path) => Some(path.file_name()?.to_string_lossy().to_ascii_lowercase()),
        _ => None,
    }
}

/// Borrow the raw face bytes for `family` and run `f` over them. The byte slice
/// is only valid inside the closure, so callers extract everything they need
/// (e.g. flattened glyph outlines) before returning. `index` is the face index
/// within a TrueType collection. Returns `None` if the family is unknown.
pub fn with_face_data<T>(family: &str, f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
    let id = face_id(family)?;
    fonts().db.lock().unwrap().with_face_data(id, f)
}

/// Borrow the raw face bytes for `family` at `weight` (see
/// [`face_id_weighted`]). The PDF exporter uses this to embed the bold face
/// for bold runs instead of faux-stroking the regular one.
pub fn with_face_data_weighted<T>(
    family: &str,
    weight: fontdb::Weight,
    f: impl FnOnce(&[u8], u32) -> T,
) -> Option<T> {
    let id = face_id_weighted(family, weight)?;
    fonts().db.lock().unwrap().with_face_data(id, f)
}

/// The family of the installed face whose file is named `name` — a bare file
/// name or any path to one, matched case-insensitively.
pub(crate) fn family_for_file(name: &str) -> Option<String> {
    let file = name.rsplit(['/', '\\']).next()?.trim();
    if file.is_empty() {
        return None;
    }
    fonts()
        .by_file
        .lock()
        .unwrap()
        .get(&file.to_ascii_lowercase())
        .cloned()
}

/// The installed family a drawing's font reference names, or `None` when
/// nothing installed answers to it.
///
/// A DWG text style stores the TrueType font *file* (`GOST2304_TypeA_italic.ttf`,
/// sometimes with a directory), which is what the style resolves by; the family name
/// lives inside the file and is usually not the same string. Inline `\f` codes
/// carry either form, so both are tried.
pub fn family_for_reference(reference: &str) -> Option<String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    // The file name is exact; the family lookup ends in a prefix/subset match
    // that a longer file name loses to (`ISOCPEUI.TTF` would take `ISOCP`), so
    // an installed face file answers first.
    family_for_file(reference).or_else(|| canonical_family_name(reference))
}

/// Whether `family` matches an installed system font (case-insensitive via
/// fontdb's own matching).
pub fn has_family(family: &str) -> bool {
    face_id(family).is_some()
}

/// The file name a face was loaded from, when it was loaded from a file.
fn source_file_name(source: &fontdb::Source) -> Option<String> {
    let path = match source {
        fontdb::Source::File(path) => path,
        fontdb::Source::SharedFile(path, _) => path,
        fontdb::Source::Binary(_) => return None,
    };
    Some(path.file_name()?.to_string_lossy().into_owned())
}

/// The file the face `name` resolves to lives in, as a DXF / DWG `STYLE`
/// record's font file names a TrueType font (`ARIALN.TTF` for Arial Narrow).
/// `None` when `name` matches no installed face.
pub fn face_file_name(name: &str) -> Option<String> {
    let id = face_id(name)?;
    let db = fonts().db.lock().unwrap();
    source_file_name(&db.face(id)?.source)
}

/// The family a font file holds, for a `STYLE` record that names its font by
/// file (`arialn.ttf`) rather than by family — the name a user would type for
/// it, so a width variant answers with its own name (`Arial Narrow`) rather
/// than the wider family fontdb filed it under.
pub fn family_of_file(file: &str) -> Option<String> {
    let sys = fonts();
    let wanted = file.rsplit(['/', '\\']).next()?.to_lowercase();
    let db = sys.db.lock().unwrap();
    let face = db
        .faces()
        .filter(|face| {
            source_file_name(&face.source).is_some_and(|name| name.to_lowercase() == wanted)
        })
        .min_by_key(|face| face.index)?;
    if face.stretch != fontdb::Stretch::Normal {
        if let Some(entry) = sys.recovered.iter().find(|entry| entry.id == face.id) {
            return Some(entry.name.clone());
        }
        if let Some(legacy) = legacy_family(&db, face) {
            return Some(legacy);
        }
    }
    face.families.first().map(|(name, _)| name.clone())
}

/// Web: add a fetched font's bytes (a Noto subset, see `web_font`) to the
/// database so family-name resolution starts finding it. Fetches land after
/// the first text layout pass, so the family list is rebuilt and the
/// canonical-name memo cleared — negative resolutions from while the font was
/// in flight must not stick. Callers invalidate the glyph caches separately
/// (`ttf_glyph::clear_fallback_cache`).
///
/// Faces may repeat across subsets (every file carries the Latin face, and the
/// CJK collections repeat it per language); `family_names` de-duplicates the
/// list, and fontdb picks one of the identical Latin faces per query.
#[cfg(target_arch = "wasm32")]
pub fn register_web_font(bytes: &[u8]) {
    let sys = fonts();
    let names = {
        let mut db = sys.db.lock().unwrap();
        db.load_font_data(bytes.to_vec());
        picker_names(&db, &sys.recovered)
        // db lock released before the families lock is taken — the resolution
        // path takes them in the opposite order.
    };
    *sys.families.lock().unwrap() = names;
    canonical_cache().lock().unwrap().clear();
}

/// Add a font file a host supplies at run time (the `font_register` control
/// op): its faces join the database and `file_name` resolves to the family of
/// the first face, so a text style naming that file finds it. Returns that
/// family and the number of faces the file added, or `None` when the bytes
/// hold no face fontdb can read. Like [`register_web_font`], the family list
/// is rebuilt and the canonical-name memo cleared; callers invalidate the glyph
/// caches and re-layout text.
pub fn register_font_file(file_name: &str, bytes: &[u8]) -> Option<(String, usize)> {
    let file = file_name.rsplit(['/', '\\']).next()?.trim();
    if file.is_empty() {
        return None;
    }
    let sys = fonts();
    let (family, faces, names) = {
        let mut db = sys.db.lock().unwrap();
        let before: std::collections::HashSet<fontdb::ID> = db.faces().map(|f| f.id).collect();
        db.load_font_data(bytes.to_vec());
        let added: Vec<String> = db
            .faces()
            .filter(|face| !before.contains(&face.id))
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .collect();
        let family = added.first()?.clone();
        (family, added.len(), picker_names(&db, &sys.recovered))
    };
    sys.by_file
        .lock()
        .unwrap()
        .insert(file.to_ascii_lowercase(), family.clone());
    *sys.families.lock().unwrap() = names;
    canonical_cache().lock().unwrap().clear();
    Some((family, faces))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recovered name must land on the face it names, not on the regular
    /// face of the family fontdb filed that face under.
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
    /// with.
    #[test]
    fn an_ordinary_family_still_resolves_through_the_family_index() {
        let sys = fonts();
        let mut checked = 0usize;
        for name in families() {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(&name)],
                ..Default::default()
            };
            let Some(expected) = sys.db.lock().unwrap().query(&query) else {
                continue;
            };
            assert_eq!(
                face_id(&name),
                Some(expected),
                "{name} is in the family index but resolved elsewhere"
            );
            checked += 1;
        }
        if checked == 0 {
            eprintln!("SKIPPED: no system fonts installed");
        }
    }

    /// A face's file leads back to the name that asked for it, width variants
    /// included: the file is what a saved `STYLE` record keeps.
    #[test]
    fn a_face_file_leads_back_to_its_family() {
        let mut checked = 0usize;
        for name in ["Arial", "Arial Narrow", "Times New Roman"] {
            let Some(file) = face_file_name(name) else {
                continue;
            };
            let family = family_of_file(&file)
                .unwrap_or_else(|| panic!("{name} lives in {file}, which resolves to no family"));
            assert_eq!(
                fold_name(&family),
                fold_name(&canonical_family_name(name).unwrap_or_else(|| name.to_string())),
                "{name} -> {file} -> {family}"
            );
            assert_eq!(
                family_of_file(&file.to_uppercase()),
                Some(family.clone()),
                "the file name matches case-insensitively"
            );
            checked += 1;
        }
        if checked == 0 {
            eprintln!("SKIPPED: none of the probe families is installed");
        }
        assert_eq!(family_of_file("no-such-font-file.ttf"), None);
    }

    #[test]
    fn folding_ignores_case_and_separators() {
        assert_eq!(fold_name("Arial Narrow"), fold_name("arialnarrow"));
        assert_eq!(fold_name("Arial-Narrow"), fold_name("ARIAL NARROW"));
        assert_ne!(fold_name("Arial Narrow"), fold_name("Arial"));
    }

    /// A host-registered file resolves by its file name — any case, bare or
    /// with a path — to the family inside it; bytes without a face are refused
    /// and leave nothing behind.
    #[test]
    fn registered_font_file_resolves_by_file_name() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/web/fonts/greek.ttf"))
            .expect("bundled web font");
        let (family, faces) =
            register_font_file("C:\\Fonts\\HostRegistered-Test.ttf", &bytes).expect("a face");
        assert!(faces >= 1);
        assert!(!family.is_empty());
        assert_eq!(
            family_for_reference("hostregistered-test.TTF").as_deref(),
            Some(family.as_str())
        );
        assert_eq!(
            family_for_reference("fonts/HostRegistered-Test.ttf").as_deref(),
            Some(family.as_str())
        );
        assert!(has_family(&family));

        assert_eq!(register_font_file("Broken-Test.ttf", b"not a font"), None);
        assert_eq!(family_for_reference("Broken-Test.ttf"), None);
        assert_eq!(register_font_file("", &bytes), None);
    }

    /// A DWG stores a style's TrueType font as a file name, so every installed
    /// face has to be reachable by the name of its file — case-insensitively,
    /// and whether it comes as a bare name or as a path.
    #[test]
    fn face_files_resolve_to_their_family() {
        let found = fonts()
            .db
            .lock()
            .unwrap()
            .faces()
            .find_map(|face| match &face.source {
                fontdb::Source::File(path) => {
                    let file = path.file_name()?.to_string_lossy().into_owned();
                    let family = face.families.first().map(|(name, _)| name.clone())?;
                    Some((file, family))
                }
                _ => None,
            });
        let Some((file, family)) = found else {
            eprintln!("no installed face files on this host; skipping");
            return;
        };
        // A bold / italic file names that face of the family.
        let family_of = |name: &str| family_for_file(name).map(|f| split_style(&f).0.to_string());
        let family_of_reference =
            |name: &str| family_for_reference(name).map(|f| split_style(&f).0.to_string());

        assert_eq!(family_of(&file).as_deref(), Some(family.as_str()));
        assert_eq!(
            family_of(&file.to_ascii_uppercase()).as_deref(),
            Some(family.as_str())
        );
        assert_eq!(
            family_of_reference(&format!("/somewhere/else/{file}")).as_deref(),
            Some(family.as_str())
        );
        assert_eq!(family_for_file("no-such-face-9f3.ttf"), None);
        assert_eq!(family_for_reference("   "), None);
    }

    #[test]
    fn sans_aliases_map_to_noto_sans() {
        for name in [
            "Arial",
            "Helvetica",
            "Verdana",
            "Tahoma",
            "Calibri",
            "Segoe UI",
            "Liberation Sans",
            "DejaVu Sans",
        ] {
            assert_eq!(
                alias_targets(&name.to_lowercase()),
                Some(&["Noto Sans"][..]),
                "{name} must alias to Noto Sans"
            );
        }
    }

    #[test]
    fn serif_aliases_prefer_serif_with_sans_fallback() {
        for name in [
            "Cambria",
            "Times New Roman",
            "Georgia",
            "Liberation Serif",
            "DejaVu Serif",
        ] {
            assert_eq!(
                alias_targets(&name.to_lowercase()),
                Some(&["Noto Serif", "Noto Sans"][..]),
                "{name} must alias to Noto Serif first, Noto Sans second"
            );
        }
    }

    #[test]
    fn alias_resolution_is_case_insensitive_and_needs_a_target() {
        // Only the sans stand-in available (today's web fetch set): a serif
        // alias degrades to Noto Sans rather than resolving to nothing.
        assert_eq!(
            alias_resolution("arial", |f| f == "Noto Sans"),
            Some("Noto Sans")
        );
        assert_eq!(
            alias_resolution("ARIAL", |f| f == "Noto Sans"),
            Some("Noto Sans")
        );
        assert_eq!(
            alias_resolution("cambria", |f| f == "Noto Sans"),
            Some("Noto Sans")
        );
        // Serif preferred when a serif subset is present.
        assert_eq!(
            alias_resolution("cambria", |f| f == "Noto Serif"),
            Some("Noto Serif")
        );
        // Nothing registered yet — fetch still in flight.
        assert_eq!(alias_resolution("arial", |_| false), None);
        // Unknown / stroke-font names are never aliased.
        assert_eq!(alias_resolution("txt", |_| true), None);
        assert_eq!(alias_resolution("isocp", |_| true), None);
        assert_eq!(alias_resolution("monospace", |_| true), None);
    }

    #[test]
    fn picker_aliases_only_once_noto_arrives() {
        // Before any fetch lands the picker must not offer unresolvable names.
        let mut empty: Vec<String> = Vec::new();
        merge_web_aliases(&mut empty);
        assert!(empty.is_empty());

        // With the Noto set present the aliases join, sorted and de-duplicated.
        let mut with_noto = vec!["Noto Sans CJK SC".to_string(), "Noto Sans".to_string()];
        merge_web_aliases(&mut with_noto);
        assert_eq!(
            with_noto,
            vec![
                "Arial".to_string(),
                "Book Antiqua".to_string(),
                "Calibri".to_string(),
                "Cambria".to_string(),
                "Constantia".to_string(),
                "DejaVu Sans".to_string(),
                "DejaVu Serif".to_string(),
                "Garamond".to_string(),
                "Georgia".to_string(),
                "Helvetica".to_string(),
                "Liberation Sans".to_string(),
                "Liberation Serif".to_string(),
                "Noto Sans".to_string(),
                "Noto Sans CJK SC".to_string(),
                "Palatino Linotype".to_string(),
                "Segoe UI".to_string(),
                "Tahoma".to_string(),
                "Times New Roman".to_string(),
                "Verdana".to_string(),
            ]
        );

        // A registry that already carries an alias name (case differs) dedupes.
        let mut dup = vec!["noto sans".to_string()];
        merge_web_aliases(&mut dup);
        assert_eq!(
            dup.iter()
                .filter(|f| f.eq_ignore_ascii_case("Noto Sans"))
                .count(),
            1
        );
    }
}

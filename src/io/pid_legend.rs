//! P&ID legend recognition: which symbols a sheet places, what each one is,
//! which tag belongs to it -- and the coloured rectangles that show the answer
//! in the drawing.
//!
//! The sheets this was written against (CPECC fire-water / foam / drainage
//! P&IDs exported by TWT) encode a symbol as a block named `$<class>$<id>`,
//! so recognition is first a dictionary lookup on the block name, with the
//! layer prefix (`VALVE_`, `EVALVE_`, ...) as the fallback for an id the
//! dictionary has not met. Symbols that are not blocks -- instrument bubbles,
//! tanks, spray points -- are loose circles, told apart by radius and read by
//! the lettering inside them. Nothing here is specific to that family beyond
//! the rules file: [`Rules`] is data (`assets/pid-legend.json`), and a new
//! block id is a new line in it, not new code.
//!
//! Tags are paired one-to-one: every symbol class names the shape of tag it
//! takes (`BUV-9999` for a butterfly valve, the nearest `XV` bubble for a
//! motorised valve), candidates within `radius_mm` are ranked by distance
//! across the whole sheet, and the closest free pair wins. That is what
//! resolves two valves 9 mm apart both having two `BUV-` tags within reach:
//! each gets the one it is nearer to, and a tag is never handed out twice.
//!
//! The result is drawn as real entities -- a closed polyline rectangle and a
//! one-line label per symbol -- on one layer per class, `PID-LEGEND-<CLASS>`,
//! whose colour is the class colour. Layers give on/off and recolouring for
//! free, and the marked-up sheet survives a DXF export. [`clear`] removes
//! them again.

use std::collections::BTreeMap;
use std::path::Path;

use acadrust::entities::{LwPolyline, Text};
use acadrust::types::{Color, Vector2, Vector3};
use acadrust::{CadDocument, EntityType, Handle};
use serde::Deserialize;

/// Every legend layer starts with this; the rest is the class in upper case.
pub const LAYER_PREFIX: &str = "PID-LEGEND-";

/// Name of the environment variable that points at a rules file overriding
/// the compiled-in `assets/pid-legend.json`.
pub const RULES_ENV: &str = "OCS_PID_LEGEND_RULES";

const DEFAULT_RULES_JSON: &str = include_str!("../../assets/pid-legend.json");

/// A model-space extent wider than this is a sheet drawn at 1:100 in
/// hundredths of a millimetre; anything narrower is paper millimetres.
const MODEL_SPACE_SHEET_UNITS: f64 = 5000.0;

/// A block reference whose definition draws nothing (or nothing measurable)
/// still gets a box this many millimetres either side of its insertion point.
const EMPTY_BODY_HALF_MM: f64 = 1.0;

/// Lettering counts as "inside" a circle up to this multiple of its radius:
/// a bubble's two lines of text are justified on the centre but the lower
/// one's anchor sits a little outside a tight circle.
const INNER_TEXT_RADIUS: f64 = 1.15;

/// A motorised valve's bubble may sit a little further away than a tag
/// lettered beside a valve body.
const BUBBLE_RADIUS_FACTOR: f64 = 1.5;

// ── Rules ────────────────────────────────────────────────────────────────

/// How a symbol class finds its tag. Absent = the class carries no tag.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TagRule {
    /// Tag shape the surrounding lettering must have: `9` digit, `?` capital
    /// letter, `*` anything from here on, other characters literal.
    pub shape: Option<String>,
    /// Read the tag from the lettering inside the symbol (circles): a line of
    /// capitals and a line of digits compose as `XV-3201`.
    pub inner: bool,
    /// Take the tag of the nearest recognised bubble whose function letters
    /// are these (`XV` for a motorised valve).
    pub bubble: Option<String>,
}

impl TagRule {
    fn wants_tag(&self) -> bool {
        self.shape.is_some() || self.inner || self.bubble.is_some()
    }
}

/// What a block name means.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRule {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    #[serde(default)]
    pub tag: TagRule,
}

/// A fallback for block ids the dictionary has not met: the layer they are
/// placed on says what family they belong to.
#[derive(Debug, Clone, Deserialize)]
pub struct LayerPrefixRule {
    pub prefix: String,
    pub class: String,
    pub label: String,
}

/// A loose circle whose radius (paper mm) falls in `radius_mm` is this class.
#[derive(Debug, Clone, Deserialize)]
pub struct CircleRule {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    pub radius_mm: [f64; 2],
    #[serde(default)]
    pub tag: TagRule,
}

/// A bubble drawn inside a square of loose lines is panel-mounted rather than
/// field-mounted; this promotes it to its own class.
#[derive(Debug, Clone, Deserialize)]
pub struct PanelBubbleRule {
    /// Circle class this applies to.
    pub from: String,
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    /// Loose lines of length ≈ 2r with both ends within 1.6r of the centre.
    pub min_lines: usize,
}

/// The whole rule set. Distances are paper millimetres.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Lettering within this distance of a symbol is a tag candidate.
    pub radius_mm: f64,
    /// Height of the label written above each rectangle.
    pub label_mm: f64,
    /// Margin between a symbol's box and its rectangle.
    pub pad_mm: f64,
    /// Blocks that are not symbols (title block, logo, ...).
    pub ignore_blocks: Vec<String>,
    pub blocks: BTreeMap<String, BlockRule>,
    pub layer_prefixes: Vec<LayerPrefixRule>,
    pub circles: Vec<CircleRule>,
    pub panel_bubble: Option<PanelBubbleRule>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            radius_mm: 15.0,
            label_mm: 1.3,
            pad_mm: 0.5,
            ignore_blocks: Vec::new(),
            blocks: BTreeMap::new(),
            layer_prefixes: Vec::new(),
            circles: Vec::new(),
            panel_bubble: None,
        }
    }
}

impl Rules {
    /// The rules compiled in from `assets/pid-legend.json`.
    pub fn builtin() -> Rules {
        serde_json::from_str(DEFAULT_RULES_JSON)
            .expect("assets/pid-legend.json is checked in and must parse")
    }

    /// Rules read from a JSON file.
    pub fn from_path(path: &Path) -> Result<Rules, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The rules to use: the file named by [`RULES_ENV`] when it is set and
    /// parses, else the compiled-in default. A broken override is reported
    /// and ignored rather than silently replaced, so the sheet still gets
    /// marked up.
    pub fn load() -> Rules {
        match std::env::var_os(RULES_ENV) {
            Some(path) => match Rules::from_path(Path::new(&path)) {
                Ok(rules) => rules,
                Err(error) => {
                    log::warn!("{RULES_ENV}: {error}; using the built-in rules");
                    Rules::builtin()
                }
            },
            None => Rules::builtin(),
        }
    }

    fn circle_rule(&self, radius_mm: f64) -> Option<&CircleRule> {
        self.circles
            .iter()
            .find(|c| c.radius_mm[0] <= radius_mm && radius_mm <= c.radius_mm[1])
    }

    fn layer_fallback(&self, layer: &str) -> Option<&LayerPrefixRule> {
        self.layer_prefixes
            .iter()
            .find(|r| layer.starts_with(&r.prefix))
    }
}

/// Whether `value` has the shape `shape` describes (see [`TagRule::shape`]).
pub fn shape_matches(shape: &str, value: &str) -> bool {
    let mut chars = value.chars();
    for p in shape.chars() {
        match p {
            '*' => return true,
            '9' => {
                if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
                    return false;
                }
            }
            '?' => {
                if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
                    return false;
                }
            }
            literal => {
                if chars.next() != Some(literal) {
                    return false;
                }
            }
        }
    }
    chars.next().is_none()
}

// ── Recognition ──────────────────────────────────────────────────────────

/// One recognised symbol.
#[derive(Debug, Clone)]
pub struct Recognized {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    /// Insertion point (block) or centre (circle), drawing units.
    pub at: (f64, f64),
    /// World-space box of what the symbol draws, drawing units.
    pub bbox: (f64, f64, f64, f64),
    /// Block name, or `circle r=<mm>` for a loose circle.
    pub source: String,
    /// `false` for a block the dictionary does not know (classified by layer).
    pub known: bool,
    /// Lettering found inside a circle, top line first.
    pub inner_text: Vec<String>,
    pub tag: Option<String>,
    /// Distance from the symbol to its tag, paper mm (0 for an inner tag).
    pub tag_distance_mm: Option<f64>,
    /// Whether this class expects a tag at all.
    pub wants_tag: bool,
}

/// What [`recognise`] found on a sheet.
#[derive(Debug, Clone, Default)]
pub struct Recognition {
    pub units_per_mm: f64,
    pub symbols: Vec<Recognized>,
    /// `<block name> on <layer>` -> placements, for ids the rules lack.
    pub unknown_blocks: BTreeMap<String, usize>,
    /// Class label -> lettering that matched the class's tag shape but no
    /// symbol claimed.
    pub orphan_tags: BTreeMap<String, Vec<String>>,
    /// Pieces of model-space lettering considered.
    pub lettering: usize,
}

impl Recognition {
    /// Symbols per (class, label), most frequent first.
    pub fn by_class(&self) -> Vec<((String, String), Vec<&Recognized>)> {
        let mut groups: BTreeMap<(String, String), Vec<&Recognized>> = BTreeMap::new();
        for s in &self.symbols {
            groups
                .entry((s.class.clone(), s.label.clone()))
                .or_default()
                .push(s);
        }
        let mut out: Vec<_> = groups.into_iter().collect();
        out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        out
    }
}

struct Lettering {
    at: (f64, f64),
    value: String,
}

fn sane(value: f64) -> bool {
    value.is_finite() && value.abs() < 1.0e12
}

fn grow(bbox: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
    if !(sane(x) && sane(y)) {
        return;
    }
    *bbox = Some(match *bbox {
        None => (x, y, x, y),
        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
    });
}

/// Entities whose bounding box says nothing about what a block draws.
fn skip_for_box(entity: &EntityType) -> bool {
    matches!(
        entity,
        // acadrust boxes an insert at its insertion point (see dxf_pid_probe).
        EntityType::Insert(_)
            | EntityType::Block(_)
            | EntityType::BlockEnd(_)
            | EntityType::Seqend(_)
            | EntityType::AttributeDefinition(_)
            | EntityType::Text(_)
            | EntityType::MText(_)
    )
}

/// Where a block-local point lands for a placement.
fn place(
    local: (f64, f64),
    base: (f64, f64),
    scale: (f64, f64),
    rotation: f64,
    at: (f64, f64),
) -> (f64, f64) {
    let x = (local.0 - base.0) * scale.0;
    let y = (local.1 - base.1) * scale.1;
    let (sin, cos) = rotation.sin_cos();
    (x * cos - y * sin + at.0, x * sin + y * cos + at.1)
}

fn lettering_of(doc: &CadDocument) -> Vec<Lettering> {
    let mut out = Vec::new();
    for entity in doc.model_space_entities() {
        match entity {
            EntityType::Text(text) => {
                let value = text.value.trim();
                if value.is_empty() {
                    continue;
                }
                let anchor = text.alignment_point.unwrap_or(text.insertion_point);
                out.push(Lettering {
                    at: (anchor.x, anchor.y),
                    value: value.to_string(),
                });
            }
            EntityType::MText(mtext) => {
                let value = mtext.value.replace("\\P", " ");
                let value = value.trim();
                if value.is_empty() {
                    continue;
                }
                out.push(Lettering {
                    at: (mtext.insertion_point.x, mtext.insertion_point.y),
                    value: value.to_string(),
                });
            }
            _ => {}
        }
    }
    out
}

/// The model-space extent over drawn entities, inserts left out (their box
/// is just the insertion point, which for a title block is nowhere near what
/// it draws).
fn drawn_extent(doc: &CadDocument) -> Option<(f64, f64, f64, f64)> {
    let mut bbox = None;
    for entity in doc.model_space_entities() {
        if skip_for_box(entity) {
            continue;
        }
        let bb = entity.as_entity().bounding_box();
        if bb.min.x == 0.0 && bb.min.y == 0.0 && bb.max.x == 0.0 && bb.max.y == 0.0 {
            continue;
        }
        grow(&mut bbox, bb.min.x, bb.min.y);
        grow(&mut bbox, bb.max.x, bb.max.y);
    }
    bbox
}

/// Drawing units per paper millimetre, from the sheet's drawn extent.
pub fn guess_units_per_mm(doc: &CadDocument) -> f64 {
    let width = drawn_extent(doc).map_or(0.0, |(x0, _, x1, _)| x1 - x0);
    if width > MODEL_SPACE_SHEET_UNITS {
        100.0
    } else {
        1.0
    }
}

/// Compose a circle's inner lettering into a tag: a line of capitals and a
/// line of digits (optionally suffixed, `0320A`) become `HS-0320A`; anything
/// else is joined as read.
fn inner_tag(inner: &[String]) -> Option<String> {
    let letters = inner
        .iter()
        .find(|v| (1..=5).contains(&v.len()) && v.chars().all(|c| c.is_ascii_uppercase()));
    let digits = inner.iter().find(|v| {
        let body = v.trim_end_matches(|c: char| c.is_ascii_uppercase());
        (3..=5).contains(&body.len())
            && body.chars().all(|c| c.is_ascii_digit())
            && v.len() - body.len() <= 1
    });
    match (letters, digits) {
        (Some(l), Some(d)) => Some(format!("{l}-{d}")),
        _ if !inner.is_empty() => Some(inner.join(" ")),
        _ => None,
    }
}

/// Recognise the symbols on `doc` with `rules`, detecting the sheet's units.
pub fn recognise(doc: &CadDocument, rules: &Rules) -> Recognition {
    recognise_with_units(doc, rules, guess_units_per_mm(doc))
}

/// [`recognise`] with the drawing-units-per-millimetre given.
pub fn recognise_with_units(doc: &CadDocument, rules: &Rules, units_per_mm: f64) -> Recognition {
    let upm = units_per_mm;
    let radius = rules.radius_mm * upm;
    let lettering = lettering_of(doc);
    let mut symbols: Vec<Recognized> = Vec::new();
    // The tag rule of each symbol, in `symbols` order, for the pairing pass.
    let mut tag_rules: Vec<TagRule> = Vec::new();
    let mut unknown_blocks: BTreeMap<String, usize> = BTreeMap::new();

    // ── block symbols
    for entity in doc.model_space_entities() {
        let EntityType::Insert(insert) = entity else {
            continue;
        };
        let name = insert.block_name.as_str();
        if rules.ignore_blocks.iter().any(|n| n == name) {
            continue;
        }
        let Some(record) = doc.block_records.get(name) else {
            continue;
        };
        let base = (record.base_point.x, record.base_point.y);
        let at = (insert.insert_point.x, insert.insert_point.y);
        let scale = (insert.x_scale(), insert.y_scale());
        let mut bbox = None;
        for member in doc.entities_in_block(name) {
            if skip_for_box(member) {
                continue;
            }
            let bb = member.as_entity().bounding_box();
            for corner in [
                (bb.min.x, bb.min.y),
                (bb.min.x, bb.max.y),
                (bb.max.x, bb.min.y),
                (bb.max.x, bb.max.y),
            ] {
                let (x, y) = place(corner, base, scale, insert.rotation, at);
                grow(&mut bbox, x, y);
            }
        }
        let half = EMPTY_BODY_HALF_MM * upm;
        let bbox = bbox.unwrap_or((at.0 - half, at.1 - half, at.0 + half, at.1 + half));
        let (class, label, color, tag, known) = match rules.blocks.get(name) {
            Some(rule) => (
                rule.class.clone(),
                rule.label.clone(),
                rule.color,
                rule.tag.clone(),
                true,
            ),
            None => {
                let layer = insert.common.layer.as_str();
                *unknown_blocks
                    .entry(format!("{name} on {layer}"))
                    .or_default() += 1;
                let (class, family) = match rules.layer_fallback(layer) {
                    Some(fallback) => (fallback.class.clone(), fallback.label.clone()),
                    None => ("unknown".to_string(), "未知块".to_string()),
                };
                (
                    class,
                    format!("{family} {name}"),
                    [255, 255, 255],
                    TagRule::default(),
                    false,
                )
            }
        };
        symbols.push(Recognized {
            class,
            label,
            color,
            at,
            bbox,
            source: name.to_string(),
            known,
            inner_text: Vec::new(),
            wants_tag: tag.wants_tag(),
            tag: None,
            tag_distance_mm: None,
        });
        tag_rules.push(tag);
    }

    // ── loose circles
    let lines: Vec<((f64, f64), (f64, f64))> = doc
        .model_space_entities()
        .filter_map(|e| match e {
            EntityType::Line(l) => Some(((l.start.x, l.start.y), (l.end.x, l.end.y))),
            _ => None,
        })
        .collect();
    for entity in doc.model_space_entities() {
        let EntityType::Circle(circle) = entity else {
            continue;
        };
        let (cx, cy, r) = (circle.center.x, circle.center.y, circle.radius);
        let r_mm = r / upm;
        let Some(rule) = rules.circle_rule(r_mm) else {
            continue;
        };
        let mut inner: Vec<(f64, f64, &str)> = lettering
            .iter()
            .filter(|l| (l.at.0 - cx).hypot(l.at.1 - cy) <= r * INNER_TEXT_RADIUS)
            .map(|l| (l.at.1, l.at.0, l.value.as_str()))
            .collect();
        // Top line first, then left to right.
        inner.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.total_cmp(&b.1)));
        let inner: Vec<String> = inner.into_iter().map(|(_, _, v)| v.to_string()).collect();

        let (class, label, color) = match &rules.panel_bubble {
            Some(panel) if panel.from == rule.class => {
                let framed = lines
                    .iter()
                    .filter(|(a, b)| {
                        let near = |p: &(f64, f64)| (p.0 - cx).hypot(p.1 - cy) <= 1.6 * r;
                        let len = (a.0 - b.0).hypot(a.1 - b.1);
                        near(a) && near(b) && (len - 2.0 * r).abs() < 0.5 * r
                    })
                    .count();
                if framed >= panel.min_lines {
                    (panel.class.clone(), panel.label.clone(), panel.color)
                } else {
                    (rule.class.clone(), rule.label.clone(), rule.color)
                }
            }
            _ => (rule.class.clone(), rule.label.clone(), rule.color),
        };
        symbols.push(Recognized {
            class,
            label,
            color,
            at: (cx, cy),
            bbox: (cx - r, cy - r, cx + r, cy + r),
            source: format!("circle r={r_mm:.2}mm"),
            known: true,
            inner_text: inner,
            wants_tag: rule.tag.wants_tag(),
            tag: None,
            tag_distance_mm: None,
        });
        tag_rules.push(rule.tag.clone());
    }

    // ── tags read from inside the symbol
    for (symbol, rule) in symbols.iter_mut().zip(&tag_rules) {
        if !rule.inner {
            continue;
        }
        let tag = match &rule.shape {
            Some(shape) => symbol
                .inner_text
                .iter()
                .find(|v| shape_matches(shape, v))
                .cloned(),
            None => inner_tag(&symbol.inner_text),
        };
        if tag.is_some() {
            symbol.tag = tag;
            symbol.tag_distance_mm = Some(0.0);
        }
    }

    // ── tags paired from outside: nearest free pair first, one-to-one
    enum Key {
        Text(usize),
        Symbol(usize),
    }
    let mut pairs: Vec<(f64, usize, Key)> = Vec::new();
    for (i, (symbol, rule)) in symbols.iter().zip(&tag_rules).enumerate() {
        if rule.inner || symbol.tag.is_some() {
            continue;
        }
        if let Some(letters) = &rule.bubble {
            let prefix = format!("{letters}-");
            for (j, other) in symbols.iter().enumerate() {
                if other.class.starts_with("bubble")
                    && other.tag.as_ref().is_some_and(|t| t.starts_with(&prefix))
                {
                    let d = (other.at.0 - symbol.at.0).hypot(other.at.1 - symbol.at.1);
                    if d <= radius * BUBBLE_RADIUS_FACTOR {
                        pairs.push((d, i, Key::Symbol(j)));
                    }
                }
            }
        } else if let Some(shape) = &rule.shape {
            for (k, l) in lettering.iter().enumerate() {
                if shape_matches(shape, &l.value) {
                    let d = (l.at.0 - symbol.at.0).hypot(l.at.1 - symbol.at.1);
                    if d <= radius {
                        pairs.push((d, i, Key::Text(k)));
                    }
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut taken_text = vec![false; lettering.len()];
    let mut taken_symbol = vec![false; symbols.len()];
    for (d, i, key) in pairs {
        if symbols[i].tag.is_some() {
            continue;
        }
        let tag = match key {
            Key::Text(k) => {
                if taken_text[k] {
                    continue;
                }
                taken_text[k] = true;
                lettering[k].value.clone()
            }
            Key::Symbol(j) => {
                if taken_symbol[j] {
                    continue;
                }
                taken_symbol[j] = true;
                symbols[j].tag.clone().unwrap_or_default()
            }
        };
        symbols[i].tag = Some(tag);
        symbols[i].tag_distance_mm = Some(d / upm);
    }

    // ── lettering that looks like a tag but found no symbol
    let mut orphan_tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, l) in lettering.iter().enumerate() {
        if taken_text[k] {
            continue;
        }
        for rule in rules.blocks.values() {
            if let Some(shape) = &rule.tag.shape {
                if shape_matches(shape, &l.value) {
                    orphan_tags
                        .entry(rule.label.clone())
                        .or_default()
                        .push(l.value.clone());
                }
            }
        }
    }
    for values in orphan_tags.values_mut() {
        values.sort();
    }

    Recognition {
        units_per_mm: upm,
        symbols,
        unknown_blocks,
        orphan_tags,
        lettering: lettering.len(),
    }
}

// ── Report ───────────────────────────────────────────────────────────────

/// A per-class summary, one line per class plus the unknown-block and
/// orphan-tag lines, for the command line or a terminal.
pub fn report(recognition: &Recognition) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} symbols recognised, {} pieces of lettering, {} units/mm",
        recognition.symbols.len(),
        recognition.lettering,
        recognition.units_per_mm
    ));
    for ((class, label), items) in recognition.by_class() {
        let mut line = format!("  {label} ({class}) x{}", items.len());
        if items[0].wants_tag {
            let tagged: Vec<&&Recognized> = items.iter().filter(|s| s.tag.is_some()).collect();
            line.push_str(&format!("  tagged {}/{}", tagged.len(), items.len()));
            let mut distances: Vec<f64> = tagged.iter().filter_map(|s| s.tag_distance_mm).collect();
            distances.retain(|d| *d > 0.0);
            if let (Some(min), Some(max)) = (
                distances.iter().copied().reduce(f64::min),
                distances.iter().copied().reduce(f64::max),
            ) {
                line.push_str(&format!("  at {min:.1}..{max:.1} mm"));
            }
            let mut tags: Vec<&str> = tagged.iter().filter_map(|s| s.tag.as_deref()).collect();
            tags.sort_unstable();
            if !tags.is_empty() {
                line.push_str(&format!(": {}", tags.join(", ")));
            }
            let untagged: Vec<String> = items
                .iter()
                .filter(|s| s.tag.is_none())
                .map(|s| format!("({:.0}, {:.0})", s.at.0, s.at.1))
                .collect();
            if !untagged.is_empty() {
                line.push_str(&format!("  UNTAGGED at {}", untagged.join(" ")));
            }
        }
        lines.push(line);
    }
    for (what, n) in &recognition.unknown_blocks {
        lines.push(format!(
            "  UNKNOWN BLOCK {what} x{n} -- add it to the rules"
        ));
    }
    for (label, tags) in &recognition.orphan_tags {
        lines.push(format!(
            "  ORPHAN {label} tags (no symbol claimed them): {}",
            tags.join(", ")
        ));
    }
    lines
}

// ── Legend entities ──────────────────────────────────────────────────────

/// The layer a class's rectangles and labels go on.
pub fn layer_for_class(class: &str) -> String {
    format!("{LAYER_PREFIX}{}", class.to_ascii_uppercase())
}

/// Whether `layer` is one [`legend_entities`] writes to.
pub fn is_legend_layer(layer: &str) -> bool {
    layer.starts_with(LAYER_PREFIX)
}

/// Layer name -> colour for every class in `recognition`.
pub fn legend_layers(recognition: &Recognition) -> BTreeMap<String, [u8; 3]> {
    recognition
        .symbols
        .iter()
        .map(|s| (layer_for_class(&s.class), s.color))
        .collect()
}

/// A closed rectangle and a one-line label per recognised symbol, each on
/// its class layer with colour ByLayer. The label reads `<label> <tag>`, or
/// just the label when the symbol carries no tag.
pub fn legend_entities(recognition: &Recognition, rules: &Rules) -> Vec<EntityType> {
    let upm = recognition.units_per_mm;
    let pad = rules.pad_mm * upm;
    let height = rules.label_mm * upm;
    let mut out = Vec::with_capacity(recognition.symbols.len() * 2);
    for symbol in &recognition.symbols {
        let layer = layer_for_class(&symbol.class);
        let (x0, y0, x1, y1) = symbol.bbox;
        let (x0, y0, x1, y1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
        let mut rectangle = LwPolyline::from_points(vec![
            Vector2::new(x0, y0),
            Vector2::new(x1, y0),
            Vector2::new(x1, y1),
            Vector2::new(x0, y1),
        ]);
        rectangle.is_closed = true;
        rectangle.common.layer = layer.clone();
        out.push(EntityType::LwPolyline(rectangle));

        let text = match &symbol.tag {
            Some(tag) => format!("{} {tag}", symbol.label),
            None => symbol.label.clone(),
        };
        let mut label =
            Text::with_value(text, Vector3::new(x0, y1 + 0.3 * height, 0.0)).with_height(height);
        label.common.layer = layer;
        out.push(EntityType::Text(label));
    }
    out
}

fn ensure_layer(doc: &mut CadDocument, name: &str, color: [u8; 3]) {
    let rgb = Color::Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
    };
    if let Some(layer) = doc.layers.get_mut(name) {
        layer.color = rgb;
        return;
    }
    let mut layer = acadrust::tables::Layer::new(name);
    layer.handle = doc.allocate_handle();
    layer.color = rgb;
    let _ = doc.layers.add(layer);
}

/// Draw the legend into `doc` (headless path): create the class layers in
/// their colours and add the rectangles and labels to model space. Returns
/// how many entities were added.
pub fn apply(doc: &mut CadDocument, recognition: &Recognition, rules: &Rules) -> usize {
    for (layer, color) in legend_layers(recognition) {
        ensure_layer(doc, &layer, color);
    }
    let mut added = 0;
    for entity in legend_entities(recognition, rules) {
        if doc.add_entity(entity).is_ok() {
            added += 1;
        }
    }
    added
}

/// Handles of every entity on a legend layer.
pub fn legend_handles(doc: &CadDocument) -> Vec<Handle> {
    doc.entities()
        .filter(|e| is_legend_layer(&e.common().layer))
        .map(|e| e.common().handle)
        .collect()
}

/// Remove everything [`apply`] added. Returns how many entities went. The
/// layers stay: empty, they purge like any other.
pub fn clear(doc: &mut CadDocument) -> usize {
    let handles = legend_handles(doc);
    handles
        .into_iter()
        .filter(|h| doc.remove_entity(*h).is_some())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_parse_and_name_the_twt_family() {
        let rules = Rules::builtin();
        assert!(rules.blocks.contains_key("$VALVE$00000316"));
        assert_eq!(rules.blocks["$VALVE$00000316"].class, "butterfly");
        assert_eq!(
            rules.blocks["$VALVE$00000316"].tag.shape.as_deref(),
            Some("BUV-9999")
        );
        assert_eq!(
            rules.blocks["$EVALVE$00000018"].tag.bubble.as_deref(),
            Some("XV")
        );
        assert!(rules.circle_rule(3.85).is_some_and(|c| c.class == "bubble"));
        assert!(rules.circle_rule(13.4).is_some_and(|c| c.class == "tank"));
        assert!(rules.circle_rule(2.1).is_some_and(|c| c.class == "s-point"));
        assert!(rules.circle_rule(1.12).is_none());
    }

    #[test]
    fn tag_shapes() {
        assert!(shape_matches("BUV-9999", "BUV-3101"));
        assert!(!shape_matches("BUV-9999", "BUV-310"));
        assert!(!shape_matches("BUV-9999", "BUV-31011"));
        assert!(!shape_matches("BUV-9999", "XV-3101"));
        assert!(shape_matches("T?-9999", "TG-0101"));
        assert!(shape_matches("T?-9999", "TD-0208"));
        assert!(!shape_matches("T?-9999", "T-0208"));
        assert!(shape_matches("DWG-*", "DWG-0100FF02-04"));
        assert!(!shape_matches("DWG-*", "接 DWG-0100FF02-04"));
    }

    #[test]
    fn inner_lettering_composes_a_bubble_tag() {
        assert_eq!(
            inner_tag(&["XV".to_string(), "3201".to_string()]),
            Some("XV-3201".to_string())
        );
        assert_eq!(
            inner_tag(&["HS".to_string(), "0320A".to_string()]),
            Some("HS-0320A".to_string())
        );
        assert_eq!(inner_tag(&["S".to_string()]), Some("S".to_string()));
        assert_eq!(inner_tag(&[]), None);
    }

    #[test]
    fn legend_layer_names() {
        assert_eq!(layer_for_class("butterfly"), "PID-LEGEND-BUTTERFLY");
        assert!(is_legend_layer("PID-LEGEND-TANK"));
        assert!(!is_legend_layer("VALVE_消防"));
    }
}

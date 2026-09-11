//! P&ID legend recognition: which symbols a sheet places, what each one is,
//! which tag belongs to it -- and the coloured rectangles that show the answer
//! in the drawing.
//!
//! The sheets this was written against (CPECC fire-water / foam / drainage
//! P&IDs exported by TWT) encode a symbol as a block named `$<class>$<id>`,
//! so recognition is first a dictionary lookup on the block name, with the
//! layer prefix (`VALVE_`, `EVALVE_`, ...) as the fallback for an id the
//! dictionary has not met. Symbols that are not blocks -- instrument bubbles,
//! tanks, spray points -- are loose circles, told apart by radius and by the
//! lettering inside them. Nothing here is specific to that family beyond
//! the rules file: [`Rules`] is data (`assets/pid-legend.json`), and a new
//! block id is a new line in it, not new code.
//!
//! A second family (the loading-island sheets) has its symbols exploded into
//! loose strokes. Those are recovered as connected components of short
//! geometry, each reduced to a shape id that is the same for every placement
//! of the same symbol at any rotation or mirror. A component is named by the
//! tag lettered beside it (`BV0301A` says ball valve), or by the shape
//! dictionary in the rules, and otherwise boxed in a colour of its own with
//! the id in the report, so a person can name it with one line of JSON.
//!
//! Tags are paired one-to-one: every symbol class names the shape of tag it
//! takes (`BUV-9999` for a butterfly valve, an `XV` bubble for a motorised
//! valve), the candidates within `radius_mm` are gathered across the whole
//! sheet, and of them as many pairs are made as the candidates allow and, of
//! such pairings, the one with the least total distance ([`match_pairs`]).
//! That is what resolves two valves 9 mm apart both having two `BUV-` tags
//! within reach -- each gets the one it is nearer to, a tag is never handed
//! out twice -- and also a column of loading arms whose tags are each
//! lettered under their own arm and so nearer the middle of the next one:
//! nearest-first would shift the whole column by one, leaving the top arm
//! unnamed and the bottom tag an orphan.
//!
//! Lettering shorter than `tag_min_chars` (four characters unless the rules
//! say otherwise) is never a tag, whichever way it would be read: the lone
//! `S` or `K` inside a spray point says what the symbol is, not which one it
//! is, and a symbol lettered so is not one that lost its tag.
//!
//! The result is drawn as real entities -- a closed polyline rectangle and a
//! one-line label per symbol -- on one layer per class, `PID-LEGEND-<CLASS>`,
//! whose colour is the class colour. Layers give on/off and recolouring for
//! free, and the marked-up sheet survives a DXF export. [`clear`] removes
//! them again.
//!
//! On the block family the pipe is traced too ([`pid_pipes`]): the `PIPE-*`
//! strokes joined into runs between the symbols' connection points (the
//! `POINT`s of a valve block, the insertion point of a block without any --
//! or the far end of its stem when its rule says `"port": "stem-end"` --
//! the rim of an S / K circle), each run carrying the line number lettered
//! along it, so a symbol knows the lines it sits in. The runs are drawn in
//! with the legend: a polyline along each on `PID-PIPE-<line number>`, one
//! layer per line in a colour of its own, the runs on no numbered line in
//! grey on `PID-PIPE-NONE`, and a small ring wherever a run ends in the air.

mod blocks;
mod exploded;
mod groups;
mod legend;
mod manual_groups;
mod pairing;
mod report;
mod rules;
mod tags;

pub use blocks::guess_units_per_mm;
pub use exploded::hash_color;
pub use groups::plot_groups;
pub use legend::{
    apply, clear, is_legend_layer, is_shape_class, layer_for_class, legend_entities,
    legend_handles, legend_layers, pipe_entities, pipe_layer, pipe_layers,
};
pub use manual_groups::{group_details, GroupDetails};
pub use report::report;
pub use rules::{
    shape_matches, BlockRule, CircleRule, LayerPrefixRule, ManualGroupRule, OrphanRules,
    PanelBubbleRule, PortRule, Rules, ShapeRules, TagClassRule, TagRule, IGNORE_CLASS,
};
pub use tags::{
    class_for_tag, derive_group_tag, derive_handles_tag, tag_from_lettering, GroupTag, TagClass,
    TagHow, TagRead, TagSource, TAG_NAME_KEY, TAG_SOURCE_KEY,
};

use std::collections::{BTreeMap, HashSet};

use acadrust::{CadDocument, EntityType, Handle};

use super::pid_pipes::{self, End, Pipes, Port};
use blocks::{lettering_of, placed_block};
use exploded::{exploded_symbols, ExplodedExclusions};
use manual_groups::recognise_manual_groups;
use pairing::match_pairs;

/// Every legend layer starts with this; the rest is the class in upper case.
pub const LAYER_PREFIX: &str = "PID-LEGEND-";

/// Layer that unnamed exploded shapes go on; each shape keeps its own colour
/// there (there is no class to give a layer to).
pub const SHAPE_LAYER: &str = "PID-LEGEND-SHAPE";

/// Every pipe layer starts with this; the rest is the line number the runs
/// on it carry (`PID-PIPE-100-FW`), or `NONE`.
pub const PIPE_LAYER_PREFIX: &str = "PID-PIPE-";

/// Layer of the runs on no numbered line.
pub const PIPE_NONE_LAYER: &str = "PID-PIPE-NONE";

/// Colour of [`PIPE_NONE_LAYER`]: grey.
pub const PIPE_NONE_COLOR: [u8; 3] = [128, 128, 128];

/// Class prefix of an exploded shape the dictionary does not name.
pub const SHAPE_CLASS_PREFIX: &str = "shape-";

/// Name of the environment variable that points at a rules file overriding
/// the compiled-in `assets/pid-legend.json`.
pub const RULES_ENV: &str = "OCS_PID_LEGEND_RULES";

const DEFAULT_RULES_JSON: &str = include_str!("../../../assets/pid-legend.json");

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

/// Grid cell, paper mm, for finding touching strokes.
const COMPONENT_CELL_MM: f64 = 5.0;

/// How many unnamed shapes the report lists before saying "and N more".
const REPORTED_SHAPES: usize = 20;

// ── Recognition ──────────────────────────────────────────────────────────

/// The explicit GROUP that supplied a symbol, when a person overrode the
/// automatic passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupOrigin {
    pub name: String,
    pub tag_source: TagSource,
}

/// One recognised symbol.
#[derive(Debug, Clone)]
pub struct Recognized {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    /// Centre of the symbol's box, drawing units.
    pub at: (f64, f64),
    /// World-space box of what the symbol draws, drawing units.
    pub bbox: (f64, f64, f64, f64),
    /// Block name, `circle r=<mm>` for a loose circle, `shape <id>` for an
    /// exploded symbol, or `group <name>` for a manual override.
    pub source: String,
    /// The marked DXF GROUP that owns this symbol, if any.
    pub group: Option<GroupOrigin>,
    /// `false` for a block the dictionary does not know (classified by
    /// layer) and for a shape nothing names.
    pub known: bool,
    /// Lettering found inside a circle, top line first.
    pub inner_text: Vec<String>,
    pub tag: Option<String>,
    /// Distance from the symbol to its tag, paper mm (0 for an inner tag).
    pub tag_distance_mm: Option<f64>,
    /// Whether this class expects a tag at all.
    pub wants_tag: bool,
    /// Whether the report says where this symbol stands when it got no tag
    /// (`TagRule::report_untagged`).
    pub report_untagged: bool,
    /// Line numbers of the pipe runs at the symbol's connection points,
    /// distinct, sorted. Empty when no pipe reaches it or none is numbered.
    pub lines: Vec<String>,
    /// The entities the symbol is drawn with: the block reference; the
    /// circle and the lettering inside it; the strokes of an exploded shape.
    pub handles: Vec<Handle>,
    /// The lettering the tag was read from, when it stands beside the symbol
    /// rather than inside it (an exploded assembly lists every tag it took).
    /// Empty for an inner tag -- its lettering is in `handles` -- and for a
    /// tag taken over from a bubble, which is a symbol of its own.
    pub tag_handles: Vec<Handle>,
}

/// Lettering that names several tags at once -- `XV-0407A～0409A`,
/// `XV-0407D/0407E`, `LA-0308～0313` -- an interlock or equipment table's
/// way of writing a group, not a tag a symbol could carry. Its members are
/// checked against the symbols' tags: an annotation whose members are all on
/// a symbol is a free cross-check of the sheet; a member no symbol carries is
/// reported as an orphan in its own right.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeAnnotation {
    pub value: String,
    /// The tags it names, in order.
    pub members: Vec<String>,
    /// Those of `members` no symbol carries.
    pub missing: Vec<String>,
}

/// An exploded shape the sheet repeats but nothing names: what the report
/// shows so a person can put it in the dictionary.
#[derive(Debug, Clone)]
pub struct UnknownShape {
    pub id: String,
    pub count: usize,
    /// Width and height of one placement, paper mm.
    pub size_mm: (f64, f64),
    /// Strokes in one placement.
    pub strokes: usize,
    /// Where one placement sits, drawing units.
    pub example_at: (f64, f64),
    /// Lettering near the placements, most frequent first.
    pub nearby: Vec<(String, usize)>,
}

/// What [`recognise`] found on a sheet.
#[derive(Debug, Clone, Default)]
pub struct Recognition {
    pub units_per_mm: f64,
    pub symbols: Vec<Recognized>,
    /// `<block name> on <layer>` -> placements, for ids the rules lack.
    pub unknown_blocks: BTreeMap<String, usize>,
    /// Repeated exploded shapes nothing names, most frequent first.
    pub unknown_shapes: Vec<UnknownShape>,
    /// Class label -> lettering that matched the class's tag shape but no
    /// symbol claimed -- and no range annotation or duplicate accounts for.
    /// A range annotation's missing members are listed here too.
    pub orphan_tags: BTreeMap<String, Vec<String>>,
    /// Class label -> unclaimed lettering that names a group of tags, with
    /// the members checked against the symbols.
    pub range_annotations: BTreeMap<String, Vec<RangeAnnotation>>,
    /// Class label -> unclaimed lettering whose value a symbol already
    /// carries as its tag (a table row), distinct. Empty when the rules say
    /// `orphans.claimed_elsewhere`, which keeps them among the orphans.
    pub duplicate_tags: BTreeMap<String, Vec<String>>,
    /// Pieces of model-space lettering considered.
    pub lettering: usize,
    /// The pipe, joined into runs between the symbols (block family).
    pub pipes: Pipes,
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
    handle: Handle,
    /// The anchor the drawing stores: the alignment point, else the
    /// insertion point. Tags are measured from here and not from the middle
    /// of the lettering: the CPECC sheets start a valve's tag beside the
    /// valve and let it run on towards the next one, so the insertion point
    /// is the end that sits by the valve.
    at: (f64, f64),
    value: String,
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
    let mut taken_text = vec![false; lettering.len()];
    let manual = recognise_manual_groups(doc, rules, upm, &lettering, &mut taken_text);
    let excluded_handles = manual.excluded_handles;
    let mut used_circles = manual.circle_handles;
    let mut symbols = manual.symbols;
    // The tag rule of each symbol, in `symbols` order, for the pairing pass.
    let mut tag_rules = vec![TagRule::default(); symbols.len()];
    let mut unknown_blocks: BTreeMap<String, usize> = BTreeMap::new();
    // Where pipe can meet each symbol, by index into `symbols`.
    let mut ports = manual.ports;

    // ── block symbols
    for entity in doc.model_space_entities() {
        let EntityType::Insert(insert) = entity else {
            continue;
        };
        if excluded_handles.contains(&insert.common.handle) {
            continue;
        }
        let name = insert.block_name.as_str();
        if rules.ignore_blocks.iter().any(|n| n == name) {
            continue;
        }
        let port_rule = rules.blocks.get(name).and_then(|rule| rule.port);
        let Some(placed) = placed_block(doc, insert, upm, port_rule) else {
            continue;
        };
        for port in placed.ports {
            ports.push((symbols.len(), port));
        }
        // Tags are measured from the body, not the insertion point: a block
        // whose base point is far from what it draws (the title block, the
        // loading arm) is inserted nowhere near its body.
        let at = placed.at;
        let bbox = placed.bbox;
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
            group: None,
            known,
            inner_text: Vec::new(),
            wants_tag: tag.wants_tag(),
            report_untagged: tag.reports_untagged(),
            lines: Vec::new(),
            tag: None,
            tag_distance_mm: None,
            handles: vec![insert.common.handle],
            tag_handles: Vec::new(),
        });
        tag_rules.push(tag);
    }

    // ── loose circles
    // Straight strokes, for the square a panel-mounted bubble sits in: lines,
    // and the sides of polylines (one family draws the square as two of them).
    let mut lines: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for entity in doc.model_space_entities() {
        if excluded_handles.contains(&entity.common().handle) {
            continue;
        }
        match entity {
            EntityType::Line(l) => lines.push(((l.start.x, l.start.y), (l.end.x, l.end.y))),
            EntityType::LwPolyline(p) => {
                let pts: Vec<(f64, f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect();
                lines.extend(pts.windows(2).map(|w| (w[0], w[1])));
                if p.is_closed && pts.len() > 2 {
                    lines.push((pts[pts.len() - 1], pts[0]));
                }
            }
            _ => {}
        }
    }
    for entity in doc.model_space_entities() {
        let EntityType::Circle(circle) = entity else {
            continue;
        };
        if excluded_handles.contains(&circle.common.handle) {
            continue;
        }
        let (cx, cy, r) = (circle.center.x, circle.center.y, circle.radius);
        let r_mm = r / upm;
        let mut inner: Vec<&Lettering> = lettering
            .iter()
            .enumerate()
            .filter(|(k, l)| {
                !taken_text[*k] && (l.at.0 - cx).hypot(l.at.1 - cy) <= r * INNER_TEXT_RADIUS
            })
            .map(|(_, l)| l)
            .collect();
        // Top line first, then left to right.
        inner.sort_by(|a, b| {
            b.at.1
                .total_cmp(&a.at.1)
                .then_with(|| a.at.0.total_cmp(&b.at.0))
        });
        let mut handles = vec![circle.common.handle];
        handles.extend(inner.iter().map(|l| l.handle));
        let inner: Vec<String> = inner.into_iter().map(|l| l.value.clone()).collect();
        let Some(rule) = rules.circle_rule(r_mm, &inner) else {
            continue;
        };
        used_circles.insert(circle.common.handle);

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
        // Pipe meets a circle symbol (an S / K point) on its rim.
        ports.push((
            symbols.len(),
            Port::Rim {
                centre: (cx, cy),
                r,
            },
        ));
        symbols.push(Recognized {
            class,
            label,
            color,
            at: (cx, cy),
            bbox: (cx - r, cy - r, cx + r, cy + r),
            source: format!("circle r={r_mm:.2}mm"),
            group: None,
            known: true,
            inner_text: inner,
            wants_tag: rule.tag.wants_tag(),
            report_untagged: rule.tag.reports_untagged(),
            lines: Vec::new(),
            tag: None,
            tag_distance_mm: None,
            handles,
            tag_handles: Vec::new(),
        });
        tag_rules.push(rule.tag.clone());
    }

    // ── exploded symbols: connected components of loose strokes
    let mut unknown_shapes = Vec::new();
    if let Some(shape_rules) = &rules.shapes {
        if shape_rules.applies(doc) {
            let found = exploded_symbols(
                doc,
                upm,
                rules,
                shape_rules,
                &ExplodedExclusions {
                    circles: &used_circles,
                    handles: &excluded_handles,
                },
                &lettering,
                &mut taken_text,
            );
            for (symbol, tag) in found.symbols {
                symbols.push(symbol);
                tag_rules.push(tag);
            }
            unknown_shapes = found.unknown;
        }
    }

    // ── tags read from inside the symbol
    // The one tag rule (`tags::tag_from_lettering`): a rule with a shape
    // takes the inner line of that shape and nothing else; without one the
    // lines compose as a bubble's, or read as they stand.
    for (symbol, rule) in symbols.iter_mut().zip(&tag_rules) {
        if !rule.inner {
            continue;
        }
        let inner: Vec<&str> = symbol.inner_text.iter().map(String::as_str).collect();
        let read = match &rule.shape {
            Some(shape) => tag_from_lettering(&inner, &[shape.as_str()])
                .filter(|read| read.how == TagHow::Shape),
            None => tag_from_lettering(&inner, &[]),
        };
        match read {
            Some(read) if rules.accepts_tag(&read.value) => {
                symbol.tag = Some(read.value);
                symbol.tag_distance_mm = Some(0.0);
            }
            // Lettered, not tagged: what is inside is too short to be a tag
            // (`S`, `K`), and an inner rule has nowhere else to look, so the
            // symbol is not one that lost its tag.
            Some(_) => {
                symbol.wants_tag = false;
                symbol.report_untagged = false;
            }
            None => {}
        }
    }

    // ── tags paired from outside, one-to-one: as many pairs as the
    // candidates allow and, of those pairings, the shortest (`match_pairs`,
    // as the exploded family). Nearest-first is not enough here either: the
    // loading-island sheets letter a loading arm's tag under its body, and a
    // column of arms 9.5 mm apart has every tag nearer the middle of the arm
    // *below* the one it names -- the whole column shifted by one, the top
    // arm unnamed and the bottom tag an orphan, though pairing them all is
    // possible. What a symbol takes is a piece of lettering or, for a
    // `bubble` rule, a recognised bubble; both are keyed on one index space
    // so that neither is handed out twice.
    let bubble_key = |j: usize| lettering.len() + j;
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new(); // (units, key, symbol)
    for (i, (symbol, rule)) in symbols.iter().zip(&tag_rules).enumerate() {
        if rule.inner || symbol.tag.is_some() {
            continue;
        }
        let reach = rule.radius_mm.map_or(radius, |r| r * upm);
        if let Some(letters) = &rule.bubble {
            let prefix = format!("{letters}-");
            for (j, other) in symbols.iter().enumerate() {
                if other.class.starts_with("bubble")
                    && other.tag.as_ref().is_some_and(|t| t.starts_with(&prefix))
                {
                    let d = (other.at.0 - symbol.at.0).hypot(other.at.1 - symbol.at.1);
                    if d <= reach * BUBBLE_RADIUS_FACTOR {
                        candidates.push((d, bubble_key(j), i));
                    }
                }
            }
        } else if let Some(shape) = &rule.shape {
            for (k, l) in lettering.iter().enumerate() {
                if !taken_text[k] && rules.accepts_tag(&l.value) && shape_matches(shape, &l.value) {
                    let d = (l.at.0 - symbol.at.0).hypot(l.at.1 - symbol.at.1);
                    if d <= reach {
                        candidates.push((d, k, i));
                    }
                }
            }
        }
    }
    for e in match_pairs(&candidates) {
        let (d, key, i) = candidates[e];
        let tag = if key < lettering.len() {
            taken_text[key] = true;
            symbols[i].tag_handles = vec![lettering[key].handle];
            lettering[key].value.clone()
        } else {
            symbols[key - lettering.len()]
                .tag
                .clone()
                .unwrap_or_default()
        };
        symbols[i].tag = Some(tag);
        symbols[i].tag_distance_mm = Some(d / upm);
    }

    // ── lettering that looks like a tag but found no symbol
    // Sorted three ways: a range annotation (`XV-0407A～0409A`) is checked
    // member by member and only a member no symbol carries is an orphan; a
    // value some symbol already carries is a duplicate (a table row), not a
    // symbol that lost its tag; the rest are orphans proper.
    let mut orphan_tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut range_annotations: BTreeMap<String, Vec<RangeAnnotation>> = BTreeMap::new();
    let mut duplicate_tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let orphan_shapes: Vec<(&str, &str)> = rules
        .blocks
        .values()
        .filter(|r| r.tag.report_orphans)
        .filter_map(|r| r.tag.shape.as_deref().map(|s| (s, r.label.as_str())))
        .chain(
            rules
                .tag_classes
                .iter()
                .filter(|r| r.report_orphans)
                .map(|r| (r.shape.as_str(), r.label.as_str())),
        )
        .collect();
    let carried: HashSet<&str> = symbols.iter().filter_map(|s| s.tag.as_deref()).collect();
    for (k, l) in lettering.iter().enumerate() {
        if taken_text[k] || !rules.accepts_tag(&l.value) {
            continue;
        }
        let expanded = expand_range(&l.value);
        for (shape, label) in &orphan_shapes {
            // A range is of a class when every tag it names has the class's
            // shape -- `BUV-3101/3102` is not itself shaped `BUV-9999`.
            let range = expanded
                .as_ref()
                .filter(|members| members.iter().all(|m| shape_matches(shape, m)));
            if range.is_none() && !shape_matches(shape, &l.value) {
                continue;
            }
            if let Some(members) = range {
                let missing: Vec<String> = members
                    .iter()
                    .filter(|m| !carried.contains(m.as_str()))
                    .cloned()
                    .collect();
                if !missing.is_empty() {
                    orphan_tags
                        .entry(label.to_string())
                        .or_default()
                        .extend(missing.iter().cloned());
                }
                range_annotations
                    .entry(label.to_string())
                    .or_default()
                    .push(RangeAnnotation {
                        value: l.value.clone(),
                        members: members.clone(),
                        missing,
                    });
            } else if carried.contains(l.value.as_str()) && !rules.orphans.claimed_elsewhere {
                duplicate_tags
                    .entry(label.to_string())
                    .or_default()
                    .push(l.value.clone());
            } else {
                orphan_tags
                    .entry(label.to_string())
                    .or_default()
                    .push(l.value.clone());
            }
        }
    }
    for values in orphan_tags.values_mut().chain(duplicate_tags.values_mut()) {
        values.sort();
        values.dedup();
    }
    for annotations in range_annotations.values_mut() {
        annotations.sort_by(|a, b| a.value.cmp(&b.value));
        annotations.dedup_by(|a, b| a.value == b.value);
    }

    // ── the pipe: runs between the symbols' connection points, numbered
    let letter_points: Vec<((f64, f64), &str)> =
        lettering.iter().map(|l| (l.at, l.value.as_str())).collect();
    let pipes = pid_pipes::trace(doc, upm, &rules.pipes, &ports, &letter_points);
    for run in &pipes.runs {
        for end in run.ends {
            if let End::Symbol(i) = end {
                symbols[i].lines.extend(run.lines.iter().cloned());
            }
        }
    }
    for symbol in &mut symbols {
        symbol.lines.sort();
        symbol.lines.dedup();
    }

    Recognition {
        units_per_mm: upm,
        symbols,
        unknown_blocks,
        unknown_shapes,
        orphan_tags,
        range_annotations,
        duplicate_tags,
        lettering: lettering.len(),
        pipes,
    }
}

/// A tag split into the part before its number, the number, and what
/// follows it: `XV-0407A` -> `("XV-", "0407", "A")`, `0409A` -> `("", "0409",
/// "A")`, `LA-0303` -> `("LA-", "0303", "")`. `None` when there is no number
/// or more than one capital letter follows it.
fn split_tag(value: &str) -> Option<(&str, &str, &str)> {
    let start = value.find(|c: char| c.is_ascii_digit())?;
    let digits = value[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(value.len(), |n| start + n);
    let (prefix, number, suffix) = (&value[..start], &value[start..digits], &value[digits..]);
    let suffix_ok =
        suffix.is_empty() || (suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_uppercase());
    suffix_ok.then_some((prefix, number, suffix))
}

/// The most members a range annotation may name; a wider span is a typo or
/// not a range at all.
const RANGE_MEMBERS_MAX: usize = 50;

/// The tags a range annotation names, or `None` when `value` is not one.
///
/// Two spellings occur on the sheets: an enumeration, `XV-0407D/0407E` --
/// each part after the first inherits the first's prefix -- and a span,
/// `XV-0407A～0409A` (also `~`), which runs over the numbers when the
/// suffixes agree (`0407A`, `0408A`, `0409A`) and over the suffix letters
/// when the numbers agree (`XV-0410A～0410H`). Anything else -- a size like
/// `1/2"`, a span over both number and letter, a part with a different
/// prefix -- is not a range.
pub fn expand_range(value: &str) -> Option<Vec<String>> {
    if value.contains(char::is_whitespace) || value.contains('"') {
        return None;
    }
    let separator = ['～', '~', '/'].into_iter().find(|s| value.contains(*s))?;
    let parts: Vec<&str> = value.split(separator).collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (prefix, first_number, first_suffix) = split_tag(parts[0])?;
    if prefix.is_empty() {
        return None;
    }
    let mut ends = vec![(first_number, first_suffix)];
    for part in &parts[1..] {
        let (p, number, suffix) = split_tag(part)?;
        if !p.is_empty() && p != prefix {
            return None;
        }
        ends.push((number, suffix));
    }
    if separator == '/' {
        return Some(
            ends.iter()
                .map(|(number, suffix)| format!("{prefix}{number}{suffix}"))
                .collect(),
        );
    }
    if ends.len() != 2 {
        return None;
    }
    let ((n1, s1), (n2, s2)) = (ends[0], ends[1]);
    if n1 != n2 && s1 == s2 {
        let (a, b) = (n1.parse::<usize>().ok()?, n2.parse::<usize>().ok()?);
        if b < a || b - a + 1 > RANGE_MEMBERS_MAX {
            return None;
        }
        let width = n1.len();
        return Some(
            (a..=b)
                .map(|n| format!("{prefix}{n:0width$}{s1}"))
                .collect(),
        );
    }
    if n1 == n2 && s1 != s2 && s1.len() == 1 && s2.len() == 1 {
        let (a, b) = (s1.as_bytes()[0], s2.as_bytes()[0]);
        if b < a {
            return None;
        }
        return Some(
            (a..=b)
                .map(|c| format!("{prefix}{n1}{}", c as char))
                .collect(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// The two spellings of a group of tags come apart into their members;
    /// what is not a group stays whole.
    #[test]
    fn a_range_annotation_expands_to_the_tags_it_names() {
        assert_eq!(
            expand_range("XV-0407A～0409A"),
            Some(words(&["XV-0407A", "XV-0408A", "XV-0409A"]))
        );
        assert_eq!(
            expand_range("XV-0410A~0410D"),
            Some(words(&["XV-0410A", "XV-0410B", "XV-0410C", "XV-0410D"]))
        );
        assert_eq!(
            expand_range("XV-0407D/0407E"),
            Some(words(&["XV-0407D", "XV-0407E"]))
        );
        assert_eq!(
            expand_range("LA-0303～0307"),
            Some(words(&[
                "LA-0303", "LA-0304", "LA-0305", "LA-0306", "LA-0307"
            ]))
        );
        assert_eq!(
            expand_range("LA-0326～LA-0327"),
            Some(words(&["LA-0326", "LA-0327"])),
            "a repeated prefix is allowed"
        );
        // Not ranges: a plain tag, a size, a span over number and letter at
        // once, a backwards span, a different prefix on the far end.
        assert_eq!(expand_range("XV-0407A"), None);
        assert_eq!(expand_range("PR-0301A 1/2\""), None);
        assert_eq!(expand_range("XV-0407A～0409B"), None);
        assert_eq!(expand_range("LA-0307～0303"), None);
        assert_eq!(expand_range("XV-0407A～HS-0409A"), None);
        assert_eq!(
            expand_range("LA-0001～0999"),
            None,
            "too wide to be a group"
        );
    }
}

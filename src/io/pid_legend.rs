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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use acadrust::entities::{Circle, LwPolyline, Text};
use acadrust::types::{Color, Vector2, Vector3};
use acadrust::{CadDocument, EntityType, Handle};
use serde::Deserialize;

use super::pid_pipes::{self, End, PipeRules, Pipes, Port};

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

/// Grid cell, paper mm, for finding touching strokes.
const COMPONENT_CELL_MM: f64 = 5.0;

/// How many unnamed shapes the report lists before saying "and N more".
const REPORTED_SHAPES: usize = 20;

// ── Rules ────────────────────────────────────────────────────────────────

/// How a symbol class finds its tag. Absent = the class carries no tag.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct TagRule {
    /// Tag shape the surrounding lettering must have: `9` digit, `?` capital
    /// letter, `*` anything from here on (a leading `*` = anything before),
    /// other characters literal.
    pub shape: Option<String>,
    /// Read the tag from the lettering inside the symbol (circles): a line of
    /// capitals and a line of digits compose as `XV-3201`.
    pub inner: bool,
    /// Take the tag of the nearest recognised bubble whose function letters
    /// are these (`XV` for a motorised valve).
    pub bubble: Option<String>,
    /// Search radius for this class, paper mm; absent = `Rules::radius_mm`.
    pub radius_mm: Option<f64>,
    /// List lettering of this shape that no symbol claimed. Off for shapes
    /// that also occur where no symbol is expected (a sheet's own number in
    /// the title block has the shape of a sheet reference).
    pub report_orphans: bool,
}

impl Default for TagRule {
    fn default() -> Self {
        TagRule {
            shape: None,
            inner: false,
            bubble: None,
            radius_mm: None,
            report_orphans: true,
        }
    }
}

impl TagRule {
    fn wants_tag(&self) -> bool {
        self.shape.is_some() || self.inner || self.bubble.is_some()
    }
}

/// Where pipe joins a block that carries no connection `POINT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PortRule {
    /// Its insertion point: the sheet connector, the foam interface.
    Insertion,
    /// The far end of the block along its stem -- the line drawn out of the
    /// insertion point -- where the block's geometry stops: the vent stub,
    /// whose pipe is drawn over the stem right through to the closed end of
    /// the cup, 13 mm from the insertion point.
    StemEnd,
}

/// What a block name (or a shape id) means.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRule {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    #[serde(default)]
    pub tag: TagRule,
    /// Where pipe joins the block when it has no `POINT`; absent = its
    /// insertion point.
    #[serde(default)]
    pub port: Option<PortRule>,
}

/// A fallback for block ids the dictionary has not met: the layer they are
/// placed on says what family they belong to.
#[derive(Debug, Clone, Deserialize)]
pub struct LayerPrefixRule {
    pub prefix: String,
    pub class: String,
    pub label: String,
}

/// A loose circle whose radius (paper mm) falls in `radius_mm`, and whose
/// inner lettering passes `inner` / `inner_is`, is this class. Rules are
/// tried in order; the first that fits wins.
#[derive(Debug, Clone, Deserialize)]
pub struct CircleRule {
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    pub radius_mm: [f64; 2],
    /// What the lettering inside must be: `"tag"` -- a line of capitals and
    /// a number (`HS` / `0320A`); `"empty"` -- none at all. Absent: anything.
    #[serde(default)]
    pub inner: Option<String>,
    /// Or: the lettering inside, joined by spaces, is one of these (`S`, `K`).
    #[serde(default)]
    pub inner_is: Vec<String>,
    #[serde(default)]
    pub tag: TagRule,
}

impl CircleRule {
    fn inner_matches(&self, inner: &[String]) -> bool {
        if !self.inner_is.is_empty() {
            let joined = inner.join(" ");
            return self.inner_is.contains(&joined);
        }
        match self.inner.as_deref() {
            Some("tag") => compose_tag(inner).is_some(),
            Some("empty") => inner.is_empty(),
            _ => true,
        }
    }
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

/// Lettering of this shape names the exploded symbol drawn beside it: the
/// nearest free component within `radius_mm` becomes `class` and takes the
/// lettering as its tag.
#[derive(Debug, Clone, Deserialize)]
pub struct TagClassRule {
    pub shape: String,
    pub class: String,
    pub label: String,
    pub color: [u8; 3],
    /// A component must be at least this wide and this tall (mm) to be
    /// claimed; keeps a flange tick from taking the valve's tag.
    #[serde(default = "default_min_side_mm")]
    pub min_side_mm: f64,
    /// Search radius, paper mm; absent = `Rules::radius_mm`. Zero = the
    /// shape is an equipment number this pass cannot pair (a skid's QK
    /// number names an outline too big to be a component): claims nothing.
    #[serde(default)]
    pub radius_mm: Option<f64>,
    /// List lettering of this shape that no component claimed.
    #[serde(default = "default_true")]
    pub report_orphans: bool,
}

fn default_min_side_mm() -> f64 {
    0.8
}

fn default_true() -> bool {
    true
}

/// Connected-component clustering of loose geometry, for sheets whose
/// symbols are exploded strokes rather than blocks.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ShapeRules {
    /// Cluster only sheets that draw on one of these layers (the exploded
    /// family's layers); empty = every sheet.
    pub layers_any: Vec<String>,
    /// Layers left out of clustering (frame, title block, dimensions).
    pub skip_layers: Vec<String>,
    /// A stroke with a segment longer than this is pipe, not symbol.
    pub max_stroke_mm: f64,
    /// A straight horizontal or vertical stroke longer than this is a pipe
    /// stub or an instrument leader: left out, so that it does not chain the
    /// symbols on a manifold into one component. 0 turns this off.
    pub pipe_stub_mm: f64,
    /// A loose circle larger than this is equipment, not a symbol part.
    pub max_circle_mm: f64,
    /// Endpoints closer than this touch.
    pub touch_mm: f64,
    /// Components whose longest side is under this are noise ...
    pub min_box_mm: f64,
    /// ... and over this are equipment outlines.
    pub max_box_mm: f64,
    /// An unnamed shape thinner than this in either direction is a tick or a
    /// collinear pair of strokes, not a symbol; it is not boxed.
    pub min_side_mm: f64,
    /// Coordinates are rounded to this before a shape is hashed.
    pub quantum_mm: f64,
    /// A shape the dictionary lacks is boxed only when the sheet repeats it
    /// at least this often.
    pub min_count: usize,
    /// A component at least this long is an assembly that may hold several
    /// symbols: a tag left over after the one-to-one pass may join one that
    /// is already claimed, and the box then lists every tag. 0 turns this off.
    pub assembly_mm: f64,
    /// Take the pipe out of a component: drop the stubs of pipe left touching
    /// a symbol, and cut at a piece of pipe joining two symbols (a valve and
    /// the strainer beside it) when each side has at least this many strokes.
    /// A symbol then has one id whatever pipe was drawn against it. 0 leaves
    /// components as they connect.
    pub split_min_strokes: usize,
    /// Second pass for the tags left over: a symbol drawn in pipe-length
    /// strokes (a flame arrester's frame, a flow indicator's body) falls to
    /// `pipe_stub_mm` whole. The strokes not yet part of any symbol are
    /// clustered again together with the pipe-length axis runs (over
    /// `pipe_stub_mm`, under `max_stroke_mm`) that are held at two or more
    /// points by other such strokes -- a frame's side or a T's bar is, a pipe
    /// leading off is held at one end only -- and a component of that pass
    /// holding at least this many of those runs may be claimed by a tag no
    /// component of the first pass took. 0 turns the second pass off.
    pub recover_min_runs: usize,
    /// Shape id -> what it is. Ids come from the report (`UNKNOWN SHAPE`).
    /// Class [`IGNORE_CLASS`] drops the shape: neither boxed nor reported
    /// (the square a panel bubble sits in is not a symbol of its own).
    pub dictionary: BTreeMap<String, BlockRule>,
}

/// Dictionary class that means "not a symbol, leave it alone".
pub const IGNORE_CLASS: &str = "ignore";

impl Default for ShapeRules {
    fn default() -> Self {
        ShapeRules {
            layers_any: Vec::new(),
            skip_layers: Vec::new(),
            max_stroke_mm: 14.0,
            pipe_stub_mm: 0.0,
            max_circle_mm: 8.0,
            touch_mm: 0.05,
            min_box_mm: 0.6,
            max_box_mm: 30.0,
            min_side_mm: 0.3,
            quantum_mm: 0.1,
            min_count: 2,
            assembly_mm: 0.0,
            split_min_strokes: 0,
            recover_min_runs: 0,
            dictionary: BTreeMap::new(),
        }
    }
}

impl ShapeRules {
    /// Whether this sheet is one to cluster.
    fn applies(&self, doc: &CadDocument) -> bool {
        self.layers_any.is_empty()
            || doc
                .model_space_entities()
                .any(|e| self.layers_any.iter().any(|l| *l == e.common().layer))
    }
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
    /// Lettering shapes that name the exploded symbol beside them.
    pub tag_classes: Vec<TagClassRule>,
    pub shapes: Option<ShapeRules>,
    /// The pipe pass; no layer prefixes = off.
    pub pipes: PipeRules,
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
            tag_classes: Vec::new(),
            shapes: None,
            pipes: PipeRules::default(),
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

    /// The first circle rule that fits a circle of `radius_mm` with `inner`
    /// lettered inside it.
    pub fn circle_rule(&self, radius_mm: f64, inner: &[String]) -> Option<&CircleRule> {
        self.circles.iter().find(|c| {
            c.radius_mm[0] <= radius_mm && radius_mm <= c.radius_mm[1] && c.inner_matches(inner)
        })
    }

    fn layer_fallback(&self, layer: &str) -> Option<&LayerPrefixRule> {
        self.layer_prefixes
            .iter()
            .find(|r| layer.starts_with(&r.prefix))
    }
}

/// Whether `value` has the shape `shape` describes (see [`TagRule::shape`]).
pub fn shape_matches(shape: &str, value: &str) -> bool {
    if let Some(rest) = shape.strip_prefix('*') {
        if rest.is_empty() {
            return true;
        }
        return value
            .char_indices()
            .any(|(i, _)| shape_matches(rest, &value[i..]));
    }
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
    /// Centre of the symbol's box, drawing units.
    pub at: (f64, f64),
    /// World-space box of what the symbol draws, drawing units.
    pub bbox: (f64, f64, f64, f64),
    /// Block name, `circle r=<mm>` for a loose circle, `shape <id>` for an
    /// exploded symbol.
    pub source: String,
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
    /// Line numbers of the pipe runs at the symbol's connection points,
    /// distinct, sorted. Empty when no pipe reaches it or none is numbered.
    pub lines: Vec<String>,
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
    /// symbol claimed.
    pub orphan_tags: BTreeMap<String, Vec<String>>,
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
    /// The anchor the drawing stores: the alignment point, else the
    /// insertion point. Tags are measured from here and not from the middle
    /// of the lettering: the CPECC sheets start a valve's tag beside the
    /// valve and let it run on towards the next one, so the insertion point
    /// is the end that sits by the valve.
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

/// A block's line, block coordinates.
type Segment = ((f64, f64), (f64, f64));

/// The far end of a block along its stem, block coordinates
/// ([`PortRule::StemEnd`]). The stem is the block's line that starts at the
/// base point (the one nearest it, if none quite does -- within a twentieth
/// of the block's reach); the far end is the point on the stem's axis where
/// the block's geometry (`corners`, its members' boxes) stops. None when the
/// block has no line at its base.
fn stem_end(lines: &[Segment], corners: &[(f64, f64)], base: (f64, f64)) -> Option<(f64, f64)> {
    let dist = |p: (f64, f64)| (p.0 - base.0).hypot(p.1 - base.1);
    let reach = corners.iter().map(|&c| dist(c)).fold(0.0, f64::max);
    let (near, far) = lines
        .iter()
        .map(|&(a, b)| if dist(a) <= dist(b) { (a, b) } else { (b, a) })
        .min_by(|x, y| dist(x.0).total_cmp(&dist(y.0)))?;
    if reach <= 0.0 || dist(near) > reach / 20.0 {
        return None;
    }
    let length = (far.0 - near.0).hypot(far.1 - near.1);
    if length <= 0.0 {
        return None;
    }
    let direction = ((far.0 - near.0) / length, (far.1 - near.1) / length);
    let along = corners
        .iter()
        .map(|c| (c.0 - base.0) * direction.0 + (c.1 - base.1) * direction.1)
        .fold(f64::NEG_INFINITY, f64::max);
    if !along.is_finite() {
        return None;
    }
    Some((base.0 + direction.0 * along, base.1 + direction.1 * along))
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

/// A circle's inner lettering as a tag, when it has the two parts of one: a
/// line of capitals and a line of digits (optionally suffixed, `0320A`)
/// become `HS-0320A`.
fn compose_tag(inner: &[String]) -> Option<String> {
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
        _ => None,
    }
}

/// [`compose_tag`], or the inner lettering joined as read when it is not a
/// tag (`S`, `K`, `M`).
fn inner_tag(inner: &[String]) -> Option<String> {
    compose_tag(inner).or_else(|| (!inner.is_empty()).then(|| inner.join(" ")))
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
    let mut symbols: Vec<Recognized> = Vec::new();
    // The tag rule of each symbol, in `symbols` order, for the pairing pass.
    let mut tag_rules: Vec<TagRule> = Vec::new();
    let mut unknown_blocks: BTreeMap<String, usize> = BTreeMap::new();
    // Where pipe can meet each symbol, by index into `symbols`.
    let mut ports: Vec<(usize, Port)> = Vec::new();

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
        // The block's connection points, placed; a block without any joins
        // pipe at its insertion point (the sheet connector, the foam
        // interface) or, when its rule says so, at the far end of its stem
        // (the vent stub). The stem rule needs the block's own lines and
        // extent, in block coordinates.
        let port_rule = rules.blocks.get(name).and_then(|r| r.port);
        let mut connections = 0;
        let mut local_lines: Vec<Segment> = Vec::new();
        let mut local_corners: Vec<(f64, f64)> = Vec::new();
        for member in doc.entities_in_block(name) {
            if let EntityType::Point(point) = member {
                let p = place(
                    (point.location.x, point.location.y),
                    base,
                    scale,
                    insert.rotation,
                    at,
                );
                ports.push((symbols.len(), Port::At(p)));
                connections += 1;
            }
            if skip_for_box(member) {
                continue;
            }
            if port_rule == Some(PortRule::StemEnd) {
                if let EntityType::Line(l) = member {
                    local_lines.push(((l.start.x, l.start.y), (l.end.x, l.end.y)));
                }
            }
            let bb = member.as_entity().bounding_box();
            for corner in [
                (bb.min.x, bb.min.y),
                (bb.min.x, bb.max.y),
                (bb.max.x, bb.min.y),
                (bb.max.x, bb.max.y),
            ] {
                if port_rule == Some(PortRule::StemEnd) {
                    local_corners.push(corner);
                }
                let (x, y) = place(corner, base, scale, insert.rotation, at);
                grow(&mut bbox, x, y);
            }
        }
        let half = EMPTY_BODY_HALF_MM * upm;
        let bbox = bbox.unwrap_or((at.0 - half, at.1 - half, at.0 + half, at.1 + half));
        if connections == 0 {
            let port = match port_rule {
                Some(PortRule::StemEnd) => stem_end(&local_lines, &local_corners, base)
                    .map(|p| place(p, base, scale, insert.rotation, at))
                    .unwrap_or(at),
                Some(PortRule::Insertion) | None => at,
            };
            ports.push((symbols.len(), Port::At(port)));
        }
        // Tags are measured from the body, not the insertion point: a block
        // whose base point is far from what it draws (the title block, the
        // loading arm) is inserted nowhere near its body.
        let at = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
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
            lines: Vec::new(),
            tag: None,
            tag_distance_mm: None,
        });
        tag_rules.push(tag);
    }

    // ── loose circles
    // Straight strokes, for the square a panel-mounted bubble sits in: lines,
    // and the sides of polylines (one family draws the square as two of them).
    let mut lines: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for entity in doc.model_space_entities() {
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
    let mut used_circles: HashSet<Handle> = HashSet::new();
    for entity in doc.model_space_entities() {
        let EntityType::Circle(circle) = entity else {
            continue;
        };
        let (cx, cy, r) = (circle.center.x, circle.center.y, circle.radius);
        let r_mm = r / upm;
        let mut inner: Vec<(f64, f64, &str)> = lettering
            .iter()
            .filter(|l| (l.at.0 - cx).hypot(l.at.1 - cy) <= r * INNER_TEXT_RADIUS)
            .map(|l| (l.at.1, l.at.0, l.value.as_str()))
            .collect();
        // Top line first, then left to right.
        inner.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.total_cmp(&b.1)));
        let inner: Vec<String> = inner.into_iter().map(|(_, _, v)| v.to_string()).collect();
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
            known: true,
            inner_text: inner,
            wants_tag: rule.tag.wants_tag(),
            lines: Vec::new(),
            tag: None,
            tag_distance_mm: None,
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
                &used_circles,
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
        let reach = rule.radius_mm.map_or(radius, |r| r * upm);
        if let Some(letters) = &rule.bubble {
            let prefix = format!("{letters}-");
            for (j, other) in symbols.iter().enumerate() {
                if other.class.starts_with("bubble")
                    && other.tag.as_ref().is_some_and(|t| t.starts_with(&prefix))
                {
                    let d = (other.at.0 - symbol.at.0).hypot(other.at.1 - symbol.at.1);
                    if d <= reach * BUBBLE_RADIUS_FACTOR {
                        pairs.push((d, i, Key::Symbol(j)));
                    }
                }
            }
        } else if let Some(shape) = &rule.shape {
            for (k, l) in lettering.iter().enumerate() {
                if !taken_text[k] && shape_matches(shape, &l.value) {
                    let d = (l.at.0 - symbol.at.0).hypot(l.at.1 - symbol.at.1);
                    if d <= reach {
                        pairs.push((d, i, Key::Text(k)));
                    }
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
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
    for (k, l) in lettering.iter().enumerate() {
        if taken_text[k] {
            continue;
        }
        for (shape, label) in &orphan_shapes {
            if shape_matches(shape, &l.value) {
                orphan_tags
                    .entry(label.to_string())
                    .or_default()
                    .push(l.value.clone());
            }
        }
    }
    for values in orphan_tags.values_mut() {
        values.sort();
        values.dedup();
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
        lettering: lettering.len(),
        pipes,
    }
}

// ── Exploded symbols ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrimKind {
    Line,
    Poly,
    Circle,
    Arc,
    Ellipse,
    Solid,
}

impl PrimKind {
    fn letter(self) -> char {
        match self {
            PrimKind::Circle => 'C',
            PrimKind::Arc => 'A',
            PrimKind::Ellipse => 'E',
            PrimKind::Line | PrimKind::Poly | PrimKind::Solid => 'L',
        }
    }
}

/// One loose stroke, paper millimetres.
#[derive(Clone)]
struct Prim {
    kind: PrimKind,
    /// Polyline points; a circle's centre only.
    pts: Vec<(f64, f64)>,
    /// Radius of a circle / arc / ellipse (major), else 0.
    r: f64,
    bbox: (f64, f64, f64, f64),
}

impl Prim {
    fn new(kind: PrimKind, pts: Vec<(f64, f64)>, r: f64) -> Option<Prim> {
        let mut bbox = None;
        for &(x, y) in &pts {
            grow(&mut bbox, x, y);
        }
        let mut bbox = bbox?;
        if kind == PrimKind::Circle {
            bbox = (bbox.0 - r, bbox.1 - r, bbox.2 + r, bbox.3 + r);
        }
        Some(Prim { kind, pts, r, bbox })
    }

    fn longest_segment(&self) -> f64 {
        self.pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .fold(0.0, f64::max)
    }

    /// This stroke as a single straight run along one axis, if it is one.
    fn axis_run(&self) -> Option<AxisRun> {
        if !matches!(self.kind, PrimKind::Line | PrimKind::Poly) || self.pts.len() != 2 {
            return None;
        }
        AxisRun::of(self.pts[0], self.pts[1])
    }
}

type Point = (f64, f64);
type BBox = (f64, f64, f64, f64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    fn other(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }
}

/// A straight stroke along one axis: where it sits across that axis (`at`)
/// and how far it runs along it (`lo..hi`), paper mm.
#[derive(Clone, Copy)]
struct AxisRun {
    axis: Axis,
    at: f64,
    lo: f64,
    hi: f64,
}

/// A stroke runs along an axis when it strays from it by no more than this
/// (or a thousandth of its length): a pipe snapped a hair off square is
/// still a pipe.
const AXIS_SLACK_MM: f64 = 0.05;

impl AxisRun {
    fn of(a: Point, b: Point) -> Option<AxisRun> {
        let (dx, dy) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
        let slack = AXIS_SLACK_MM.max(1e-3 * dx.max(dy));
        if dx > dy && dy <= slack {
            Some(AxisRun {
                axis: Axis::Horizontal,
                at: (a.1 + b.1) / 2.0,
                lo: a.0.min(b.0),
                hi: a.0.max(b.0),
            })
        } else if dy > dx && dx <= slack {
            Some(AxisRun {
                axis: Axis::Vertical,
                at: (a.0 + b.0) / 2.0,
                lo: a.1.min(b.1),
                hi: a.1.max(b.1),
            })
        } else {
            None
        }
    }

    fn len(&self) -> f64 {
        self.hi - self.lo
    }

    /// Whether this run sits strictly within a box's width across the axis:
    /// through the body, not along one of its edges.
    fn through(&self, bbox: BBox, eps: f64) -> bool {
        let (_, (lo, hi)) = extents(bbox, self.axis);
        self.at > lo + eps && self.at < hi - eps
    }
}

/// The extent of `bbox` along `axis`, and across it.
fn extents(bbox: BBox, axis: Axis) -> ((f64, f64), (f64, f64)) {
    match axis {
        Axis::Horizontal => ((bbox.0, bbox.2), (bbox.1, bbox.3)),
        Axis::Vertical => ((bbox.1, bbox.3), (bbox.0, bbox.2)),
    }
}

/// An axis-aligned straight stroke of the sheet, whether or not it is short
/// enough to be part of a symbol; `prim` indexes it in the loose strokes
/// when it is, `pipe` in the pipe-length strokes kept for the second pass.
#[derive(Clone, Copy)]
struct SheetRun {
    run: AxisRun,
    prim: Option<usize>,
    pipe: Option<usize>,
}

/// What [`loose_prims`] takes from a sheet.
struct Loose {
    /// Strokes short enough to be part of a symbol.
    prims: Vec<Prim>,
    /// Straight axis-aligned strokes past the pipe stub length but not past
    /// the longest a symbol stroke can be: pipe, unless the second pass finds
    /// them framed into a symbol.
    pipe_prims: Vec<Prim>,
    /// Every straight axis-aligned stroke, pipe runs included.
    runs: Vec<SheetRun>,
    /// Circles already recognised as symbols, `(centre, radius)`.
    circles: Vec<(Point, f64)>,
}

fn arc_points(c: (f64, f64), r: f64, start: f64, end: f64, n: usize) -> Vec<(f64, f64)> {
    let tau = std::f64::consts::TAU;
    let mut span = (end - start).rem_euclid(tau);
    if span == 0.0 {
        span = tau;
    }
    (0..=n)
        .map(|k| {
            let a = start + span * k as f64 / n as f64;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// The loose strokes of a sheet that can be part of an exploded symbol, with
/// the pipe runs and recognised circles that say where the pipe goes.
fn loose_prims(
    doc: &CadDocument,
    upm: f64,
    rules: &ShapeRules,
    used_circles: &HashSet<Handle>,
) -> Loose {
    let mm = |v: f64| v / upm;
    // Strokes short enough for a symbol, with their axis run if straight;
    // pipe runs go straight to `out.runs`.
    let mut kept: Vec<(Prim, Option<AxisRun>)> = Vec::new();
    let mut pipes: Vec<(Prim, AxisRun)> = Vec::new();
    let mut out = Loose {
        prims: Vec::new(),
        pipe_prims: Vec::new(),
        runs: Vec::new(),
        circles: Vec::new(),
    };
    for entity in doc.model_space_entities() {
        let layer = entity.common().layer.as_str();
        if is_legend_layer(layer) || rules.skip_layers.iter().any(|l| l == layer) {
            continue;
        }
        let prim = match entity {
            EntityType::Line(l) => Prim::new(
                PrimKind::Line,
                vec![(mm(l.start.x), mm(l.start.y)), (mm(l.end.x), mm(l.end.y))],
                0.0,
            ),
            EntityType::LwPolyline(p) => {
                let mut pts: Vec<(f64, f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (mm(v.location.x), mm(v.location.y)))
                    .collect();
                if p.is_closed && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                Prim::new(PrimKind::Poly, pts, 0.0)
            }
            EntityType::Polyline2D(p) => {
                let mut pts: Vec<(f64, f64)> = p
                    .vertices
                    .iter()
                    .map(|v| (mm(v.location.x), mm(v.location.y)))
                    .collect();
                if p.flags.is_closed() && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                Prim::new(PrimKind::Poly, pts, 0.0)
            }
            EntityType::Circle(c) => {
                let r = mm(c.radius);
                let centre = (mm(c.center.x), mm(c.center.y));
                if used_circles.contains(&c.common.handle) {
                    if sane(centre.0) && sane(centre.1) && sane(r) {
                        out.circles.push((centre, r));
                    }
                    continue;
                }
                if r > rules.max_circle_mm {
                    continue;
                }
                Prim::new(PrimKind::Circle, vec![centre], r)
            }
            EntityType::Arc(a) => {
                let r = mm(a.radius);
                Prim::new(
                    PrimKind::Arc,
                    arc_points(
                        (mm(a.center.x), mm(a.center.y)),
                        r,
                        a.start_angle,
                        a.end_angle,
                        6,
                    ),
                    r,
                )
            }
            EntityType::Ellipse(e) => {
                let (cx, cy) = (mm(e.center.x), mm(e.center.y));
                let (ax, ay) = (mm(e.major_axis.x), mm(e.major_axis.y));
                let ratio = e.minor_axis_ratio;
                let pts = (0..=12)
                    .map(|k| {
                        let t = std::f64::consts::TAU * k as f64 / 12.0;
                        (
                            cx + ax * t.cos() - ay * ratio * t.sin(),
                            cy + ay * t.cos() + ax * ratio * t.sin(),
                        )
                    })
                    .collect();
                Prim::new(PrimKind::Ellipse, pts, ax.hypot(ay))
            }
            EntityType::Solid(s) => {
                let p = |v: Vector3| (mm(v.x), mm(v.y));
                let corners = [
                    p(s.first_corner),
                    p(s.second_corner),
                    p(s.fourth_corner),
                    p(s.third_corner),
                    p(s.first_corner),
                ];
                Prim::new(PrimKind::Solid, corners.to_vec(), 0.0)
            }
            _ => continue,
        };
        let Some(prim) = prim else {
            continue;
        };
        if !(sane(prim.bbox.0) && sane(prim.bbox.1) && sane(prim.bbox.2) && sane(prim.bbox.3)) {
            continue;
        }
        let run = prim.axis_run();
        // Too long for a symbol stroke, or a straight axis run past the pipe
        // stub length: pipe (or an instrument leader), not symbol. A run of
        // pipe stub length that a symbol stroke could still be is kept aside
        // for the second pass.
        let too_long =
            prim.kind != PrimKind::Circle && prim.longest_segment() > rules.max_stroke_mm;
        let stub = rules.pipe_stub_mm > 0.0 && run.is_some_and(|r| r.len() > rules.pipe_stub_mm);
        match run {
            Some(run) if too_long => out.runs.push(SheetRun {
                run,
                prim: None,
                pipe: None,
            }),
            Some(run) if stub => pipes.push((prim, run)),
            _ if too_long => {}
            _ => kept.push((prim, run)),
        }
    }
    let (prims, runs): (Vec<Prim>, Vec<Option<AxisRun>>) = kept.into_iter().unzip();
    let (prims, index) = dedupe(prims, rules.touch_mm);
    for (old, run) in runs.into_iter().enumerate() {
        if let (Some(run), Some(prim)) = (run, index[old]) {
            out.runs.push(SheetRun {
                run,
                prim: Some(prim),
                pipe: None,
            });
        }
    }
    out.prims = prims;
    let (pipe_prims, runs): (Vec<Prim>, Vec<AxisRun>) = pipes.into_iter().unzip();
    let (pipe_prims, index) = dedupe(pipe_prims, rules.touch_mm);
    for (old, run) in runs.into_iter().enumerate() {
        if let Some(pipe) = index[old] {
            out.runs.push(SheetRun {
                run,
                prim: None,
                pipe: Some(pipe),
            });
        }
    }
    out.pipe_prims = pipe_prims;
    out
}

/// Whether two strokes are the same drawing: same kind of mark, same points
/// (either way round, within `eps`), same radius.
fn same_stroke(a: &Prim, b: &Prim, eps: f64) -> bool {
    if a.kind.letter() != b.kind.letter() || a.pts.len() != b.pts.len() || (a.r - b.r).abs() > eps {
        return false;
    }
    let close = |p: Point, q: Point| (p.0 - q.0).abs() <= eps && (p.1 - q.1).abs() <= eps;
    a.pts.iter().zip(&b.pts).all(|(&p, &q)| close(p, q))
        || a.pts
            .iter()
            .zip(b.pts.iter().rev())
            .all(|(&p, &q)| close(p, q))
}

/// Drop strokes drawn twice over: a line on top of an identical polyline, a
/// diagonal repeated. A symbol with one of its lines doubled then has the id
/// of the symbol drawn once, and a doubled piece of pipe between two valves
/// is one piece, so taking it out does part them. Returns the strokes kept
/// and, per input stroke, its new index (`None` = dropped).
fn dedupe(prims: Vec<Prim>, eps: f64) -> (Vec<Prim>, Vec<Option<usize>>) {
    let mut order: Vec<usize> = (0..prims.len()).collect();
    order.sort_by(|&a, &b| prims[a].bbox.0.total_cmp(&prims[b].bbox.0));
    let mut dropped = vec![false; prims.len()];
    for (n, &i) in order.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        for &j in &order[n + 1..] {
            if prims[j].bbox.0 - prims[i].bbox.0 > eps {
                break;
            }
            if !dropped[j] && same_stroke(&prims[i], &prims[j], eps) {
                dropped[j] = true;
            }
        }
    }
    let mut index = vec![None; prims.len()];
    let mut out = Vec::with_capacity(prims.len());
    for (i, prim) in prims.into_iter().enumerate() {
        if !dropped[i] {
            index[i] = Some(out.len());
            out.push(prim);
        }
    }
    (out, index)
}

fn point_segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return (p.0 - a.0).hypot(p.1 - a.1);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Whether segments `a`–`b` and `c`–`d` cross (proper intersection).
fn segments_cross(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let orient = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (o1, o2) = (orient(a, b, c), orient(a, b, d));
    let (o3, o4) = (orient(c, d, a), orient(c, d, b));
    o1 * o2 < 0.0 && o3 * o4 < 0.0
}

/// Whether an end of one stroke lies on the other (either way round), two
/// strokes cross (the X of a valve drawn as two lines), or two circles meet.
fn touches(a: &Prim, b: &Prim, eps: f64) -> bool {
    for (p, q) in [(a, b), (b, a)] {
        for &e in &p.pts {
            if q.kind == PrimKind::Circle {
                let d = (e.0 - q.pts[0].0).hypot(e.1 - q.pts[0].1);
                if (d - q.r).abs() <= eps || d <= eps {
                    return true;
                }
            } else if q
                .pts
                .windows(2)
                .any(|w| point_segment_distance(e, w[0], w[1]) <= eps)
            {
                return true;
            }
        }
    }
    if a.kind == PrimKind::Circle && b.kind == PrimKind::Circle {
        let d = (a.pts[0].0 - b.pts[0].0).hypot(a.pts[0].1 - b.pts[0].1);
        if d <= eps || (d - (a.r + b.r)).abs() <= eps || (d - (a.r - b.r).abs()).abs() <= eps {
            return true;
        }
    }
    if a.kind != PrimKind::Circle && b.kind != PrimKind::Circle {
        for s in a.pts.windows(2) {
            for t in b.pts.windows(2) {
                if segments_cross(s[0], s[1], t[0], t[1]) {
                    return true;
                }
            }
        }
    }
    false
}

/// Groups of strokes that touch, transitively.
fn components(prims: &[Prim], eps: f64) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..prims.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in prims.iter().enumerate() {
        let (x0, y0, x1, y1) = p.bbox;
        let cell = |v: f64| (v / COMPONENT_CELL_MM).floor() as i64;
        for gx in cell(x0)..=cell(x1) {
            for gy in cell(y0)..=cell(y1) {
                grid.entry((gx, gy)).or_default().push(i);
            }
        }
    }
    let mut checked: HashSet<(usize, usize)> = HashSet::new();
    for items in grid.values() {
        for (n, &i) in items.iter().enumerate() {
            for &j in &items[n + 1..] {
                let key = (i.min(j), i.max(j));
                if !checked.insert(key) {
                    continue;
                }
                let (bi, bj) = (prims[i].bbox, prims[j].bbox);
                if bi.0 > bj.2 + eps || bj.0 > bi.2 + eps || bi.1 > bj.3 + eps || bj.1 > bi.3 + eps
                {
                    continue;
                }
                if touches(&prims[i], &prims[j], eps) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..prims.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    groups.into_values().collect()
}

/// Box of a set of strokes.
fn group_bbox(prims: &[Prim], idxs: &[usize]) -> Option<BBox> {
    let mut bbox = None;
    for &i in idxs {
        let b = prims[i].bbox;
        grow(&mut bbox, b.0, b.1);
        grow(&mut bbox, b.2, b.3);
    }
    bbox
}

/// Connected parts of `idxs` when `without` is taken out, by the touch
/// relation among the remaining strokes.
fn parts_without(prims: &[Prim], idxs: &[usize], without: usize, eps: f64) -> Vec<Vec<usize>> {
    let rest: Vec<usize> = idxs.iter().copied().filter(|&i| i != without).collect();
    let mut seen = vec![false; rest.len()];
    let mut parts = Vec::new();
    for start in 0..rest.len() {
        if seen[start] {
            continue;
        }
        let mut part = vec![rest[start]];
        seen[start] = true;
        let mut queue = vec![start];
        while let Some(a) = queue.pop() {
            for b in 0..rest.len() {
                if !seen[b] && touches(&prims[rest[a]], &prims[rest[b]], eps) {
                    seen[b] = true;
                    part.push(rest[b]);
                    queue.push(b);
                }
            }
        }
        parts.push(part);
    }
    parts
}

/// A part cut off a component must be at least this wide and this tall to
/// count as a symbol rather than a bit of pipe or a tick.
const MIN_PART_SIDE_MM: f64 = 0.5;

/// A run reaching within this of a box's edge enters it.
const REACH_MM: f64 = 0.2;

/// A stub whose free end lies within this of a recognised circle's rim is a
/// stem to that circle (an actuator mark, a drain point), not pipe.
const STEM_END_MM: f64 = 0.1;

/// Whether point `p` lies on stroke `q` (within `eps`).
fn point_on_prim(p: Point, q: &Prim, eps: f64) -> bool {
    if q.kind == PrimKind::Circle {
        let d = (p.0 - q.pts[0].0).hypot(p.1 - q.pts[0].1);
        return (d - q.r).abs() <= eps;
    }
    q.pts
        .windows(2)
        .any(|w| point_segment_distance(p, w[0], w[1]) <= eps)
}

/// Whether a straight run along `axis` that is not one of the part's own
/// strokes (`own`) enters the part from outside: it lies within the part's
/// width across the axis and reaches either of its edges along the axis, or
/// passes straight through. That is a pipe (or an instrument leader) drawn
/// into the symbol. `runs` are the runs that can do so -- see
/// [`runs_for_group`].
fn threaded(
    runs: &[SheetRun],
    own: impl Fn(usize) -> bool,
    bbox: BBox,
    axis: Axis,
    eps: f64,
) -> bool {
    let ((lo, hi), _) = extents(bbox, axis);
    runs.iter().any(|r| {
        r.run.axis == axis
            && r.prim.is_none_or(|i| !own(i))
            && r.run.through(bbox, eps)
            && (((r.run.hi - lo).abs() <= REACH_MM && r.run.lo < lo)
                || ((r.run.lo - hi).abs() <= REACH_MM && r.run.hi > hi)
                || (r.run.lo <= lo + REACH_MM && r.run.hi >= hi - REACH_MM))
    })
}

/// The runs that can enter a component's parts from outside and so keep a
/// bridge from being cut: the pipe runs (no stroke of the pass) and the
/// component's own strokes -- the stubs trimmed off it are pipe too. Another
/// symbol's stroke is neither: the stem arrow drawn 0.07 mm off a gate valve
/// has a 0.2 mm base that would otherwise "enter" the valve and hold the
/// three valves of SP02-05's relief branch together. Empty when the
/// component is too small for [`split_at_bridges`] to cut at all.
fn runs_for_group(runs: &[SheetRun], group: &[usize], min_strokes: usize) -> Vec<SheetRun> {
    if min_strokes == 0 || group.len() < 2 * min_strokes + 1 {
        return Vec::new();
    }
    runs.iter()
        .filter(|r| r.prim.is_none_or(|i| group.contains(&i)))
        .copied()
        .collect()
}

/// Cut a component where a piece of pipe joins two symbols: a straight
/// axis-aligned stroke whose removal leaves at least two parts of
/// `min_strokes` strokes and some width and height, running through the
/// body of each part rather than along an edge -- unless the pipe enters one
/// of those parts the other way, in which case the stroke is a stem or a
/// branch and belongs to the symbol. The pipe piece is dropped and each part
/// is cut again the same way. `runs` as [`runs_for_group`] gives them.
fn split_at_bridges(
    prims: &[Prim],
    runs: &[SheetRun],
    idxs: Vec<usize>,
    min_strokes: usize,
    eps: f64,
) -> Vec<Vec<usize>> {
    if min_strokes == 0 || idxs.len() < 2 * min_strokes + 1 {
        return vec![idxs];
    }
    for &candidate in &idxs {
        let Some(bridge) = prims[candidate].axis_run() else {
            continue;
        };
        let parts = parts_without(prims, &idxs, candidate, eps);
        let substantial: Vec<(&Vec<usize>, BBox)> = parts
            .iter()
            .filter_map(|part| {
                let bbox = group_bbox(prims, part)?;
                let side = (bbox.2 - bbox.0).min(bbox.3 - bbox.1);
                (part.len() >= min_strokes && side >= MIN_PART_SIDE_MM).then_some((part, bbox))
            })
            .collect();
        if substantial.len() < 2 || !substantial.iter().all(|(_, b)| bridge.through(*b, eps)) {
            continue;
        }
        let crossed = substantial.iter().any(|(part, bbox)| {
            threaded(runs, |i| part.contains(&i), *bbox, bridge.axis.other(), eps)
        });
        if crossed {
            continue;
        }
        return parts
            .into_iter()
            .flat_map(|part| split_at_bridges(prims, runs, part, min_strokes, eps))
            .collect();
    }
    vec![idxs]
}

/// Drop the pipe left touching a symbol: a straight axis-aligned stroke
/// attached to the rest at one end only, whose free end sticks out past the
/// rest along its own axis while it lies within the rest's width -- the
/// pipe entering the symbol, or turning a corner at its edge -- and whose
/// free end touches no recognised circle (a stem to an actuator mark or a
/// drain point looks the same and is part of the symbol). A tick at the
/// symbol's edge ends where the body ends and so does not stick out. Done
/// until nothing changes, so a stub drawn in two pieces goes too. A symbol
/// so has one id whatever length of pipe was drawn against it.
fn trim_pipe_stubs(
    prims: &[Prim],
    mut idxs: Vec<usize>,
    circles: &[(Point, f64)],
    eps: f64,
) -> Vec<usize> {
    loop {
        if idxs.len() < 2 {
            return idxs;
        }
        let stub = idxs.iter().position(|&i| {
            let p = &prims[i];
            let Some(run) = p.axis_run() else {
                return false;
            };
            let attached = |e: Point| {
                idxs.iter()
                    .any(|&j| j != i && point_on_prim(e, &prims[j], eps))
            };
            let free = match (attached(p.pts[0]), attached(p.pts[1])) {
                (true, false) => p.pts[1],
                (false, true) => p.pts[0],
                _ => return false,
            };
            let rest: Vec<usize> = idxs.iter().copied().filter(|&j| j != i).collect();
            let Some(core) = group_bbox(prims, &rest) else {
                return false;
            };
            let ((lo, hi), (across_lo, across_hi)) = extents(core, run.axis);
            let along = match run.axis {
                Axis::Horizontal => free.0,
                Axis::Vertical => free.1,
            };
            let sticks_out = along < lo - eps || along > hi + eps;
            let within = run.at >= across_lo - eps && run.at <= across_hi + eps;
            let to_a_circle = circles
                .iter()
                .any(|&(c, r)| ((free.0 - c.0).hypot(free.1 - c.1) - r).abs() <= STEM_END_MM);
            sticks_out && within && !to_a_circle
        });
        match stub {
            Some(at) => {
                idxs.remove(at);
            }
            None => return idxs,
        }
    }
}

/// The eight symmetries of the square.
const SYMMETRIES: [fn(Point) -> Point; 8] = [
    |(x, y)| (x, y),
    |(x, y)| (-y, x),
    |(x, y)| (-x, -y),
    |(x, y)| (y, -x),
    |(x, y)| (-x, y),
    |(x, y)| (y, x),
    |(x, y)| (x, -y),
    |(x, y)| (-y, -x),
];

/// A string that is the same for every placement of a shape at any rotation
/// by 90° or mirror: strokes about the box centre, rounded to `q`, under the
/// symmetry that sorts lowest.
fn signature(prims: &[Prim], idxs: &[usize], centre: (f64, f64), q: f64) -> String {
    // (stroke kind, points about the centre, radius in quanta)
    let tokens: Vec<(char, Vec<Point>, i64)> = idxs
        .iter()
        .map(|&i| {
            let p = &prims[i];
            let rel = |pt: Point| (pt.0 - centre.0, pt.1 - centre.1);
            let pts = match p.kind {
                PrimKind::Circle => vec![rel(p.pts[0])],
                PrimKind::Arc | PrimKind::Ellipse => vec![
                    rel(p.pts[0]),
                    rel(p.pts[p.pts.len() - 1]),
                    rel(p.pts[p.pts.len() / 2]),
                ],
                _ => p.pts.iter().map(|&pt| rel(pt)).collect(),
            };
            (p.kind.letter(), pts, (p.r / q).round() as i64)
        })
        .collect();
    let mut best: Option<String> = None;
    for sym in SYMMETRIES {
        let mut rows: Vec<String> = tokens
            .iter()
            .map(|(kind, pts, r)| {
                let mut q_pts: Vec<(i64, i64)> = pts
                    .iter()
                    .map(|&pt| {
                        let (x, y) = sym(pt);
                        ((x / q).round() as i64, (y / q).round() as i64)
                    })
                    .collect();
                // A stroke reads the same from either end.
                let reversed: Vec<(i64, i64)> = q_pts.iter().rev().copied().collect();
                if reversed < q_pts {
                    q_pts = reversed;
                }
                format!("{kind}{r}:{q_pts:?}")
            })
            .collect();
        rows.sort();
        let s = rows.join("|");
        if best.as_ref().is_none_or(|b| s < *b) {
            best = Some(s);
        }
    }
    best.unwrap_or_default()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A saturated, mid-light colour that differs between shape ids.
pub fn hash_color(hash: u64) -> [u8; 3] {
    let hue = (hash % 360) as f64;
    let (s, l): (f64, f64) = (0.85, 0.55);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b)]
}

struct Component {
    /// Box in paper mm.
    bbox: (f64, f64, f64, f64),
    centre: (f64, f64),
    id: String,
    hash: u64,
    /// The strokes, as indices into the pass's stroke list.
    idxs: Vec<usize>,
}

impl Component {
    /// A component of `idxs` with its box, centre and shape id, unless its
    /// box is too small for a symbol or too big for one.
    fn of(prims: &[Prim], idxs: Vec<usize>, rules: &ShapeRules) -> Option<Component> {
        let bbox = group_bbox(prims, &idxs)?;
        let longest = (bbox.2 - bbox.0).max(bbox.3 - bbox.1);
        if longest < rules.min_box_mm || longest > rules.max_box_mm {
            return None;
        }
        let centre = ((bbox.0 + bbox.2) / 2.0, (bbox.1 + bbox.3) / 2.0);
        let sig = signature(prims, &idxs, centre, rules.quantum_mm);
        let hash = fnv1a(sig.as_bytes());
        Some(Component {
            bbox,
            centre,
            id: format!("{:08x}", (hash >> 32) as u32 ^ hash as u32),
            hash,
            idxs,
        })
    }

    fn strokes(&self) -> usize {
        self.idxs.len()
    }

    fn size(&self) -> (f64, f64) {
        (self.bbox.2 - self.bbox.0, self.bbox.3 - self.bbox.1)
    }
}

struct Exploded {
    symbols: Vec<(Recognized, TagRule)>,
    unknown: Vec<UnknownShape>,
}

/// One-to-one pairing of texts with components over the candidate
/// `(distance, text, component)` edges: as many pairs as the edges allow
/// and, of those pairings, the least total distance. Returns the indices of
/// the chosen edges.
///
/// Successive shortest augmenting paths on the unit-capacity flow network
/// source -> texts -> components -> sink; each augmentation adds one pair at
/// the least extra distance, so after the last one the pairing is both
/// maximum and cheapest. The graph is small (a sheet's tags, each within
/// reach of a few components), so Bellman-Ford with a queue is plenty.
fn match_pairs(edges: &[(f64, usize, usize)]) -> Vec<usize> {
    struct Arc {
        to: usize,
        cap: u8,
        cost: f64,
        rev: usize,
    }
    const SOURCE: usize = 0;
    const SINK: usize = 1;
    let mut text_node: BTreeMap<usize, usize> = BTreeMap::new();
    let mut comp_node: BTreeMap<usize, usize> = BTreeMap::new();
    for &(_, k, ci) in edges {
        text_node.entry(k).or_insert(0);
        comp_node.entry(ci).or_insert(0);
    }
    let mut n = 2;
    for node in text_node.values_mut().chain(comp_node.values_mut()) {
        *node = n;
        n += 1;
    }
    let mut adj: Vec<Vec<Arc>> = (0..n).map(|_| Vec::new()).collect();
    let add = |adj: &mut Vec<Vec<Arc>>, from: usize, to: usize, cost: f64| -> usize {
        let (fi, ti) = (adj[from].len(), adj[to].len());
        adj[from].push(Arc {
            to,
            cap: 1,
            cost,
            rev: ti,
        });
        adj[to].push(Arc {
            to: from,
            cap: 0,
            cost: -cost,
            rev: fi,
        });
        fi
    };
    for &t in text_node.values() {
        add(&mut adj, SOURCE, t, 0.0);
    }
    for &c in comp_node.values() {
        add(&mut adj, c, SINK, 0.0);
    }
    let arcs: Vec<(usize, usize)> = edges
        .iter()
        .map(|&(d, k, ci)| {
            let t = text_node[&k];
            (t, add(&mut adj, t, comp_node[&ci], d))
        })
        .collect();
    loop {
        let mut dist = vec![f64::INFINITY; n];
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
        let mut queued = vec![false; n];
        let mut queue = std::collections::VecDeque::from([SOURCE]);
        dist[SOURCE] = 0.0;
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            for (i, arc) in adj[u].iter().enumerate() {
                if arc.cap > 0 && dist[u] + arc.cost < dist[arc.to] - 1e-9 {
                    dist[arc.to] = dist[u] + arc.cost;
                    prev[arc.to] = Some((u, i));
                    if !queued[arc.to] {
                        queued[arc.to] = true;
                        queue.push_back(arc.to);
                    }
                }
            }
        }
        if !dist[SINK].is_finite() {
            break;
        }
        let mut v = SINK;
        while let Some((u, i)) = prev[v] {
            let rev = adj[u][i].rev;
            adj[u][i].cap -= 1;
            adj[v][rev].cap += 1;
            v = u;
        }
    }
    arcs.iter()
        .enumerate()
        .filter(|(_, &(t, i))| adj[t][i].cap == 0)
        .map(|(e, _)| e)
        .collect()
}

/// Components of loose geometry, named by the tag beside them or by the
/// shape dictionary, or boxed as repeated unknowns.
fn exploded_symbols(
    doc: &CadDocument,
    upm: f64,
    rules: &Rules,
    shape_rules: &ShapeRules,
    used_circles: &HashSet<Handle>,
    lettering: &[Lettering],
    taken_text: &mut [bool],
) -> Exploded {
    let loose = loose_prims(doc, upm, shape_rules, used_circles);
    let prims = loose.prims.as_slice();
    let mut comps: Vec<Component> = Vec::new();
    let eps = shape_rules.touch_mm;
    let min_strokes = shape_rules.split_min_strokes;
    // Take the pipe out: stubs off each component, then a cut wherever a
    // piece of pipe joins two symbols, then the stubs that cut left behind.
    let trim = |idxs: Vec<usize>| {
        if min_strokes > 0 {
            trim_pipe_stubs(prims, idxs, &loose.circles, eps)
        } else {
            idxs
        }
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for group in components(prims, eps) {
        let runs = runs_for_group(&loose.runs, &group, min_strokes);
        for part in split_at_bridges(prims, &runs, trim(group), min_strokes, eps) {
            groups.push(trim(part));
        }
    }
    for idxs in groups {
        if idxs.is_empty() || (idxs.len() < 2 && prims[idxs[0]].kind != PrimKind::Circle) {
            continue;
        }
        if let Some(comp) = Component::of(prims, idxs, shape_rules) {
            comps.push(comp);
        }
    }

    // ── a tag lettered beside a component names it
    let mut claims: Vec<(f64, usize, usize, usize)> = Vec::new(); // (mm, text, comp, rule)
    for (ri, rule) in rules.tag_classes.iter().enumerate() {
        let reach = rule.radius_mm.unwrap_or(rules.radius_mm);
        if reach <= 0.0 {
            continue;
        }
        for (k, l) in lettering.iter().enumerate() {
            if taken_text[k] || !shape_matches(&rule.shape, &l.value) {
                continue;
            }
            let at = (l.at.0 / upm, l.at.1 / upm);
            for (ci, c) in comps.iter().enumerate() {
                let (w, h) = c.size();
                if w < rule.min_side_mm || h < rule.min_side_mm {
                    continue;
                }
                let d = (c.centre.0 - at.0).hypot(c.centre.1 - at.1);
                if d <= reach {
                    claims.push((d, k, ci, ri));
                }
            }
        }
    }
    claims.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut text_used = vec![false; lettering.len()];
    let mut comp_used = vec![false; comps.len()];
    // Nearest free pair first, to learn which classes have a symbol on this
    // sheet at all. One none of whose tags finds one (the flame arresters,
    // drawn in pipe-length strokes and filtered out) sits out the pairing
    // below: with no symbol of its own to be had, it could only take another
    // class's.
    let mut found: HashSet<&str> = HashSet::new();
    for &(_, k, ci, ri) in &claims {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        found.insert(rules.tag_classes[ri].class.as_str());
    }
    // Then pair for real, keeping as many tags matched as the candidates
    // allow and, of such pairings, the shortest. A row of valves lettered at
    // one offset (SP02-10's GV0326A..GV0327D: 7 mm apart, each tag 6 mm left
    // of its own valve and 3 mm right of the neighbour's) defeats
    // nearest-first: the whole row shifts by one, the end tag is orphaned and
    // the end valve goes unnamed, though pairing all seven is possible. A
    // shape the dictionary names as some other class is left out here, so
    // that being matched at all never pulls a tag onto it.
    let eligible: Vec<usize> = (0..claims.len())
        .filter(|&e| {
            let (_, _, ci, ri) = claims[e];
            let rule = &rules.tag_classes[ri];
            found.contains(rule.class.as_str())
                && !shape_rules
                    .dictionary
                    .get(&comps[ci].id)
                    .is_some_and(|named| named.class != rule.class)
        })
        .collect();
    let pairs: Vec<(f64, usize, usize)> = eligible
        .iter()
        .map(|&e| (claims[e].0, claims[e].1, claims[e].2))
        .collect();
    let mut chosen: Vec<usize> = match_pairs(&pairs)
        .into_iter()
        .map(|p| eligible[p])
        .collect();
    // What that could not place takes the nearest free component as before:
    // a tag whose only candidate the dictionary names otherwise (SP02-05's
    // PSV0407A, lettered beside a valve drawn like the bleed valves) still
    // names it -- the tag is the stronger evidence.
    text_used.fill(false);
    comp_used.fill(false);
    for &e in &chosen {
        text_used[claims[e].1] = true;
        comp_used[claims[e].2] = true;
    }
    for (e, &(_, k, ci, _)) in claims.iter().enumerate() {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        chosen.push(e);
    }
    // comp -> (rule, text, mm) of its claim, then any further tags.
    let mut claimed: HashMap<usize, (usize, usize, f64, Vec<usize>)> = HashMap::new();
    for e in chosen {
        let (d, k, ci, ri) = claims[e];
        taken_text[k] = true;
        claimed.insert(ci, (ri, k, d, Vec::new()));
    }
    // Tags still free may join an assembly: a long component already claimed
    // (valves on one branch drawn touching), which then lists them all. The
    // tag has to sit beside the assembly's box, not merely within reach of
    // its centre, or the next branch's tags would drift in.
    if shape_rules.assembly_mm > 0.0 {
        let mut joins: Vec<(f64, usize, usize)> = Vec::new();
        for &(_, k, ci, _) in &claims {
            if taken_text[k] || !claimed.contains_key(&ci) {
                continue;
            }
            let c = &comps[ci];
            let longest = (c.bbox.2 - c.bbox.0).max(c.bbox.3 - c.bbox.1);
            if longest < shape_rules.assembly_mm {
                continue;
            }
            let at = (lettering[k].at.0 / upm, lettering[k].at.1 / upm);
            let dx = (c.bbox.0 - at.0).max(0.0).max(at.0 - c.bbox.2);
            let dy = (c.bbox.1 - at.1).max(0.0).max(at.1 - c.bbox.3);
            let gap = dx.hypot(dy);
            if gap <= shape_rules.assembly_mm {
                joins.push((gap, k, ci));
            }
        }
        joins.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, k, ci) in joins {
            if taken_text[k] {
                continue;
            }
            if let Some(entry) = claimed.get_mut(&ci) {
                taken_text[k] = true;
                entry.3.push(k);
            }
        }
    }

    // ── the rest: dictionary, or repeated unknowns
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (ci, c) in comps.iter().enumerate() {
        let (w, h) = c.size();
        if !claimed.contains_key(&ci)
            && !shape_rules.dictionary.contains_key(&c.id)
            && w.min(h) >= shape_rules.min_side_mm
        {
            *counts.entry(c.id.as_str()).or_default() += 1;
        }
    }
    let mut summaries: BTreeMap<&str, UnknownShape> = BTreeMap::new();
    let mut nearby: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
    let mut symbols = Vec::new();
    let scale = |b: (f64, f64, f64, f64)| (b.0 * upm, b.1 * upm, b.2 * upm, b.3 * upm);
    // Strokes of the components that became symbols (named, ignored or
    // boxed): the second pass leaves them alone.
    let mut in_symbol = vec![false; prims.len()];
    for (ci, c) in comps.iter().enumerate() {
        let at = (c.centre.0 * upm, c.centre.1 * upm);
        let source = format!("shape {} ({} strokes)", c.id, c.strokes());
        let repeated = counts.get(c.id.as_str()).copied().unwrap_or(0) >= shape_rules.min_count;
        if claimed.contains_key(&ci) || shape_rules.dictionary.contains_key(&c.id) || repeated {
            for &i in &c.idxs {
                in_symbol[i] = true;
            }
        }
        if let Some((ri, k, d, more)) = claimed.get(&ci) {
            let rule = &rules.tag_classes[*ri];
            let mut tag = lettering[*k].value.clone();
            for &m in more {
                tag.push_str(" + ");
                tag.push_str(&lettering[m].value);
            }
            symbols.push((
                Recognized {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: true,
                    inner_text: Vec::new(),
                    tag: Some(tag),
                    tag_distance_mm: Some(*d),
                    wants_tag: true,
                    lines: Vec::new(),
                },
                TagRule::default(),
            ));
        } else if let Some(rule) = shape_rules.dictionary.get(&c.id) {
            if rule.class == IGNORE_CLASS {
                continue;
            }
            symbols.push((
                Recognized {
                    class: rule.class.clone(),
                    label: rule.label.clone(),
                    color: rule.color,
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: true,
                    inner_text: Vec::new(),
                    tag: None,
                    tag_distance_mm: None,
                    wants_tag: rule.tag.wants_tag(),
                    lines: Vec::new(),
                },
                rule.tag.clone(),
            ));
        } else if repeated {
            let (w, h) = c.size();
            symbols.push((
                Recognized {
                    class: format!("{SHAPE_CLASS_PREFIX}{}", c.id),
                    label: format!("图形 {}", c.id),
                    color: hash_color(c.hash),
                    at,
                    bbox: scale(c.bbox),
                    source,
                    known: false,
                    inner_text: Vec::new(),
                    tag: None,
                    tag_distance_mm: None,
                    wants_tag: false,
                    lines: Vec::new(),
                },
                TagRule::default(),
            ));
            let summary = summaries
                .entry(c.id.as_str())
                .or_insert_with(|| UnknownShape {
                    id: c.id.clone(),
                    count: 0,
                    size_mm: (w, h),
                    strokes: c.strokes(),
                    example_at: at,
                    nearby: Vec::new(),
                });
            summary.count += 1;
            let reach = w.max(h) * 0.75 + 4.0;
            let near = nearby.entry(c.id.as_str()).or_default();
            for l in lettering {
                let (lx, ly) = (l.at.0 / upm, l.at.1 / upm);
                if (lx - c.centre.0).abs() <= reach && (ly - c.centre.1).abs() <= reach {
                    *near.entry(l.value.as_str()).or_default() += 1;
                }
            }
        }
    }
    let mut unknown: Vec<UnknownShape> = summaries
        .into_iter()
        .map(|(id, mut summary)| {
            let mut near: Vec<(String, usize)> = nearby
                .remove(id)
                .unwrap_or_default()
                .into_iter()
                .map(|(v, n)| (v.to_string(), n))
                .collect();
            near.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            near.truncate(4);
            summary.nearby = near;
            summary
        })
        .collect();
    unknown.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));

    // ── second pass: the tags still free, over the strokes the pipe rule took
    if shape_rules.recover_min_runs > 0 && shape_rules.pipe_stub_mm > 0.0 {
        for (comp, claim) in recovered_symbols(
            &loose,
            &in_symbol,
            rules,
            shape_rules,
            lettering,
            upm,
            taken_text,
        ) {
            let at = (comp.centre.0 * upm, comp.centre.1 * upm);
            let source = format!(
                "shape {} ({} strokes, second pass)",
                comp.id,
                comp.strokes()
            );
            if let Some((ri, k, d)) = claim {
                let rule = &rules.tag_classes[ri];
                symbols.push((
                    Recognized {
                        class: rule.class.clone(),
                        label: rule.label.clone(),
                        color: rule.color,
                        at,
                        bbox: scale(comp.bbox),
                        source,
                        known: true,
                        inner_text: Vec::new(),
                        tag: Some(lettering[k].value.clone()),
                        tag_distance_mm: Some(d),
                        wants_tag: true,
                        lines: Vec::new(),
                    },
                    TagRule::default(),
                ));
            } else if let Some(rule) = shape_rules.dictionary.get(&comp.id) {
                // Named by the dictionary, as a first-pass component would be;
                // one of these that no tag took is still the symbol it is.
                if rule.class == IGNORE_CLASS {
                    continue;
                }
                symbols.push((
                    Recognized {
                        class: rule.class.clone(),
                        label: rule.label.clone(),
                        color: rule.color,
                        at,
                        bbox: scale(comp.bbox),
                        source,
                        known: true,
                        inner_text: Vec::new(),
                        tag: None,
                        tag_distance_mm: None,
                        wants_tag: rule.tag.wants_tag(),
                        lines: Vec::new(),
                    },
                    rule.tag.clone(),
                ));
            }
        }
    }
    Exploded { symbols, unknown }
}

/// A tag's claim on a component: `(tag rule, text, mm)`.
type Claim = (usize, usize, f64);

/// The candidate components of the second pass, each with the claim a tag
/// no first-pass component took makes on it, or none. A candidate nobody
/// claims is the caller's to name by the dictionary.
///
/// A symbol drawn in pipe-length strokes -- the flame arrester's frame of
/// 2.7 and 3.2 mm lines, the flow indicator's 3.7 x 4.9 mm body -- loses all
/// of them to `pipe_stub_mm` and leaves nothing for its tag to claim. Here
/// the strokes not yet part of a symbol are clustered again together with
/// those pipe-length runs (`Loose::pipe_prims`) that are *framed*: held at
/// two or more points by other strokes of this pool, not counting a run
/// along the same line. A frame's side has the two neighbouring sides on
/// it, a T's bar has its stem and a side; a pipe leading off the symbol is
/// held at one end, by the symbol, and the pipe between two valves is held
/// by the valves, which are symbols already and so not in the pool. The
/// pipe is then taken out of each component as in the first pass, and a
/// component of at least three strokes, `recover_min_runs` of them such
/// runs, is a candidate. Tags pair with candidates as in the first pass:
/// most pairs, then shortest -- a candidate the dictionary names as another
/// class left out of that -- then nearest free for the rest.
fn recovered_symbols(
    loose: &Loose,
    in_symbol: &[bool],
    rules: &Rules,
    shape_rules: &ShapeRules,
    lettering: &[Lettering],
    upm: f64,
    taken_text: &mut [bool],
) -> Vec<(Component, Option<Claim>)> {
    let eps = shape_rules.touch_mm;
    let free: Vec<usize> = (0..loose.prims.len()).filter(|&i| !in_symbol[i]).collect();
    if loose.pipe_prims.is_empty() {
        return Vec::new();
    }
    // The pool: the free strokes first, then every pipe-length run.
    let mut pool: Vec<Prim> = free.iter().map(|&i| loose.prims[i].clone()).collect();
    pool.extend(loose.pipe_prims.iter().cloned());
    let n_free = free.len();
    let collinear = |a: &Prim, b: &Prim| match (a.axis_run(), b.axis_run()) {
        (Some(ra), Some(rb)) => ra.axis == rb.axis && (ra.at - rb.at).abs() <= eps,
        _ => false,
    };
    let framed: Vec<bool> = (0..pool.len())
        .map(|i| {
            if i < n_free {
                return true;
            }
            let (run, bi) = (&pool[i], pool[i].bbox);
            let held = pool
                .iter()
                .enumerate()
                .filter(|&(j, other)| {
                    let bj = other.bbox;
                    j != i
                        && !(bi.0 > bj.2 + eps
                            || bj.0 > bi.2 + eps
                            || bi.1 > bj.3 + eps
                            || bj.1 > bi.3 + eps)
                        && !collinear(run, other)
                        && touches(run, other, eps)
                })
                .count();
            held >= 2
        })
        .collect();
    // Renumber to the strokes of the pass: the free strokes (all of them, so
    // they keep the indices 0..n_free), then the framed runs.
    let mut prims: Vec<Prim> = Vec::new();
    let mut of_prim: HashMap<usize, usize> = HashMap::new(); // Loose::prims index -> pass index
    let mut of_pipe: HashMap<usize, usize> = HashMap::new(); // Loose::pipe_prims index -> pass index
    for (p, prim) in pool.into_iter().enumerate() {
        if !framed[p] {
            continue;
        }
        if p < n_free {
            of_prim.insert(free[p], prims.len());
        } else {
            of_pipe.insert(p - n_free, prims.len());
        }
        prims.push(prim);
    }
    if prims.len() == n_free {
        return Vec::new();
    }
    // The sheet's runs in this pass's numbering: a symbol's stroke is gone
    // (it can no more thread a component here than in the first pass), a
    // pipe-length run is a stroke of the pass when framed and pipe when not.
    let runs: Vec<SheetRun> = loose
        .runs
        .iter()
        .filter_map(|r| {
            let prim = match (r.prim, r.pipe) {
                (Some(i), _) => Some(*of_prim.get(&i)?),
                (None, Some(p)) => of_pipe.get(&p).copied(),
                (None, None) => None,
            };
            Some(SheetRun {
                run: r.run,
                prim,
                pipe: None,
            })
        })
        .collect();
    let min_strokes = shape_rules.split_min_strokes;
    let trim = |idxs: Vec<usize>| {
        if min_strokes > 0 {
            trim_pipe_stubs(&prims, idxs, &loose.circles, eps)
        } else {
            idxs
        }
    };
    let mut comps: Vec<Component> = Vec::new();
    for group in components(&prims, eps) {
        let runs = runs_for_group(&runs, &group, min_strokes);
        for part in split_at_bridges(&prims, &runs, trim(group), min_strokes, eps) {
            let idxs = trim(part);
            let held_runs = idxs.iter().filter(|&&i| i >= n_free).count();
            if idxs.len() < 3 || held_runs < shape_rules.recover_min_runs {
                continue;
            }
            if let Some(comp) = Component::of(&prims, idxs, shape_rules) {
                comps.push(comp);
            }
        }
    }
    // Pair as the first pass does.
    let mut claims: Vec<(f64, usize, usize, usize)> = Vec::new(); // (mm, text, comp, rule)
    for (ri, rule) in rules.tag_classes.iter().enumerate() {
        let reach = rule.radius_mm.unwrap_or(rules.radius_mm);
        if reach <= 0.0 {
            continue;
        }
        for (k, l) in lettering.iter().enumerate() {
            if taken_text[k] || !shape_matches(&rule.shape, &l.value) {
                continue;
            }
            let at = (l.at.0 / upm, l.at.1 / upm);
            for (ci, c) in comps.iter().enumerate() {
                let (w, h) = c.size();
                if w < rule.min_side_mm || h < rule.min_side_mm {
                    continue;
                }
                let d = (c.centre.0 - at.0).hypot(c.centre.1 - at.1);
                if d <= reach {
                    claims.push((d, k, ci, ri));
                }
            }
        }
    }
    claims.sort_by(|a, b| a.0.total_cmp(&b.0));
    let eligible: Vec<usize> = (0..claims.len())
        .filter(|&e| {
            let (_, _, ci, ri) = claims[e];
            !shape_rules
                .dictionary
                .get(&comps[ci].id)
                .is_some_and(|named| named.class != rules.tag_classes[ri].class)
        })
        .collect();
    let pairs: Vec<(f64, usize, usize)> = eligible
        .iter()
        .map(|&e| (claims[e].0, claims[e].1, claims[e].2))
        .collect();
    let mut chosen: Vec<usize> = match_pairs(&pairs)
        .into_iter()
        .map(|p| eligible[p])
        .collect();
    let mut text_used = vec![false; lettering.len()];
    let mut comp_used = vec![false; comps.len()];
    for &e in &chosen {
        text_used[claims[e].1] = true;
        comp_used[claims[e].2] = true;
    }
    for (e, &(_, k, ci, _)) in claims.iter().enumerate() {
        if text_used[k] || comp_used[ci] {
            continue;
        }
        text_used[k] = true;
        comp_used[ci] = true;
        chosen.push(e);
    }
    let mut taken_comp: Vec<Option<Claim>> = vec![None; comps.len()];
    for e in chosen {
        let (d, k, ci, ri) = claims[e];
        taken_text[k] = true;
        taken_comp[ci] = Some((ri, k, d));
    }
    comps.into_iter().zip(taken_comp).collect()
}

// ── Report ───────────────────────────────────────────────────────────────

/// A per-class summary, one line per class plus the unknown-block,
/// unknown-shape and orphan-tag lines, for the command line or a terminal.
pub fn report(recognition: &Recognition) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "{} symbols recognised, {} pieces of lettering, {} units/mm",
        recognition.symbols.len(),
        recognition.lettering,
        recognition.units_per_mm
    ));
    for ((class, label), items) in recognition.by_class() {
        if is_shape_class(&class) {
            // Summarised below, one line per shape id.
            continue;
        }
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
    for shape in recognition.unknown_shapes.iter().take(REPORTED_SHAPES) {
        let near: Vec<String> = shape
            .nearby
            .iter()
            .map(|(v, n)| {
                if *n > 1 {
                    format!("{v} x{n}")
                } else {
                    v.clone()
                }
            })
            .collect();
        lines.push(format!(
            "  UNKNOWN SHAPE {} x{}  {:.1}x{:.1} mm  {} strokes  e.g. at ({:.0}, {:.0}){} -- name it under shapes.dictionary",
            shape.id,
            shape.count,
            shape.size_mm.0,
            shape.size_mm.1,
            shape.strokes,
            shape.example_at.0,
            shape.example_at.1,
            if near.is_empty() {
                String::new()
            } else {
                format!("  near: {}", near.join(", "))
            }
        ));
    }
    if recognition.unknown_shapes.len() > REPORTED_SHAPES {
        lines.push(format!(
            "  ... and {} more unknown shapes",
            recognition.unknown_shapes.len() - REPORTED_SHAPES
        ));
    }
    for (label, tags) in &recognition.orphan_tags {
        lines.push(format!(
            "  ORPHAN {label} tags (no symbol claimed them): {}",
            tags.join(", ")
        ));
    }
    let pipes = &recognition.pipes;
    if pipes.segments > 0 {
        let lettered = pipes.runs.iter().filter(|r| !r.numbers.is_empty()).count();
        let on_a_line = pipes.runs.iter().filter(|r| !r.lines.is_empty()).count();
        let ambiguous = pipes.runs.iter().filter(|r| r.lines.len() > 1).count();
        lines.push(format!(
            "  PIPE {} strokes -> {} runs: {lettered} lettered, {on_a_line} on a line ({ambiguous} on two), {} on none; {}/{} connection points on a pipe, {} open ends",
            pipes.segments,
            pipes.runs.len(),
            pipes.runs.len() - on_a_line,
            pipes.connected_ports,
            pipes.ports,
            pipes.open_ends
        ));
        for (line, runs) in pipes.by_line() {
            let length: f64 = runs.iter().map(|r| r.length_mm).sum();
            let mut on: Vec<String> = pipes
                .symbols_on(line)
                .into_iter()
                .map(|i| {
                    let s = &recognition.symbols[i];
                    s.tag
                        .clone()
                        .unwrap_or_else(|| format!("{} ({:.0}, {:.0})", s.label, s.at.0, s.at.1))
                })
                .collect();
            on.sort();
            on.dedup();
            lines.push(format!(
                "  LINE {line}: {} runs, {length:.0} mm, symbols: {}",
                runs.len(),
                if on.is_empty() {
                    "-".to_string()
                } else {
                    on.join(", ")
                }
            ));
        }
        let off_line: Vec<&pid_pipes::Run> =
            pipes.runs.iter().filter(|r| r.lines.is_empty()).collect();
        if !off_line.is_empty() {
            let length: f64 = off_line.iter().map(|r| r.length_mm).sum();
            lines.push(format!(
                "  LINE (none): {} runs, {length:.0} mm on no numbered line",
                off_line.len()
            ));
        }
    }
    lines
}

// ── Legend entities ──────────────────────────────────────────────────────

/// Whether `class` is an exploded shape nothing has named yet.
pub fn is_shape_class(class: &str) -> bool {
    class.starts_with(SHAPE_CLASS_PREFIX)
}

/// The layer a class's rectangles and labels go on.
pub fn layer_for_class(class: &str) -> String {
    if is_shape_class(class) {
        return SHAPE_LAYER.to_string();
    }
    format!("{LAYER_PREFIX}{}", class.to_ascii_uppercase())
}

/// The layer the runs carrying `line` are drawn on.
pub fn pipe_layer(line: &str) -> String {
    format!("{PIPE_LAYER_PREFIX}{line}")
}

/// Whether `layer` is one [`legend_entities`] writes to: a class layer or a
/// pipe layer.
pub fn is_legend_layer(layer: &str) -> bool {
    layer.starts_with(LAYER_PREFIX) || layer.starts_with(PIPE_LAYER_PREFIX)
}

/// Layer name -> colour for every class in `recognition` and every line
/// number its pipe carries. Unnamed shapes share [`SHAPE_LAYER`], white;
/// each draws in its own colour.
pub fn legend_layers(recognition: &Recognition) -> BTreeMap<String, [u8; 3]> {
    let mut layers: BTreeMap<String, [u8; 3]> = recognition
        .symbols
        .iter()
        .map(|s| {
            if is_shape_class(&s.class) {
                (SHAPE_LAYER.to_string(), [255, 255, 255])
            } else {
                (layer_for_class(&s.class), s.color)
            }
        })
        .collect();
    layers.extend(pipe_layers(recognition));
    layers
}

/// The layers a run is drawn on: one per line number it carries, so each
/// line's layer shows the whole line; [`PIPE_NONE_LAYER`] for a run on none.
fn run_layers(run: &pid_pipes::Run) -> Vec<String> {
    if run.lines.is_empty() {
        vec![PIPE_NONE_LAYER.to_string()]
    } else {
        run.lines.iter().map(|line| pipe_layer(line)).collect()
    }
}

/// Layer name -> colour for the pipe runs: a colour of its own per line
/// number, derived from the number so the same line is the same colour on
/// every sheet; grey for the runs on no numbered line.
pub fn pipe_layers(recognition: &Recognition) -> BTreeMap<String, [u8; 3]> {
    let mut out = BTreeMap::new();
    for run in &recognition.pipes.runs {
        if run.lines.is_empty() {
            out.insert(PIPE_NONE_LAYER.to_string(), PIPE_NONE_COLOR);
        }
        for line in &run.lines {
            out.insert(pipe_layer(line), hash_color(fnv1a(line.as_bytes())));
        }
    }
    out
}

/// The pipe runs drawn in: a polyline along each run on the layer of every
/// line number it carries -- a run two lines both reach is drawn on both, so
/// either layer alone shows its whole line -- or on [`PIPE_NONE_LAYER`] when
/// it carries none, and a ring of `pipes.open_end_mm` wherever a run ends in
/// the air. All ByLayer.
pub fn pipe_entities(recognition: &Recognition, rules: &Rules) -> Vec<EntityType> {
    let radius = rules.pipes.open_end_mm * recognition.units_per_mm;
    let mut out = Vec::new();
    for run in &recognition.pipes.runs {
        let Some((&first, &last)) = run.path.first().zip(run.path.last()) else {
            continue;
        };
        for layer in run_layers(run) {
            let mut polyline = LwPolyline::from_points(
                run.path.iter().map(|&(x, y)| Vector2::new(x, y)).collect(),
            );
            polyline.common.layer = layer.clone();
            polyline.common.color = Color::ByLayer;
            out.push(EntityType::LwPolyline(polyline));
            for (end, at) in [(run.ends[0], first), (run.ends[1], last)] {
                if end != End::Open {
                    continue;
                }
                let mut ring = Circle::new();
                ring.center = Vector3::new(at.0, at.1, 0.0);
                ring.radius = radius;
                ring.common.layer = layer.clone();
                ring.common.color = Color::ByLayer;
                out.push(EntityType::Circle(ring));
            }
        }
    }
    out
}

/// A closed rectangle and a one-line label per recognised symbol, each on
/// its class layer with colour ByLayer (an unnamed shape carries its own
/// colour), then the pipe runs ([`pipe_entities`]). The label reads
/// `<label> <tag>`, or just the label when the symbol carries no tag.
pub fn legend_entities(recognition: &Recognition, rules: &Rules) -> Vec<EntityType> {
    let upm = recognition.units_per_mm;
    let pad = rules.pad_mm * upm;
    let height = rules.label_mm * upm;
    let mut out = Vec::with_capacity(recognition.symbols.len() * 2);
    for symbol in &recognition.symbols {
        let layer = layer_for_class(&symbol.class);
        let color = if is_shape_class(&symbol.class) {
            Color::Rgb {
                r: symbol.color[0],
                g: symbol.color[1],
                b: symbol.color[2],
            }
        } else {
            Color::ByLayer
        };
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
        rectangle.common.color = color;
        out.push(EntityType::LwPolyline(rectangle));

        let text = match &symbol.tag {
            Some(tag) => format!("{} {tag}", symbol.label),
            None => symbol.label.clone(),
        };
        let mut label =
            Text::with_value(text, Vector3::new(x0, y1 + 0.3 * height, 0.0)).with_height(height);
        label.common.layer = layer;
        label.common.color = color;
        out.push(EntityType::Text(label));
    }
    out.extend(pipe_entities(recognition, rules));
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

/// Draw the legend into `doc` (headless path): create the class and pipe
/// layers in their colours and add the rectangles, labels and runs to model
/// space. Returns how many entities were added.
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

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

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
        let bubble = words(&["XV", "3201"]);
        assert!(rules
            .circle_rule(3.85, &bubble)
            .is_some_and(|c| c.class == "bubble"));
        assert!(rules
            .circle_rule(13.4, &words(&["TD-0201", "5000m"]))
            .is_some_and(|c| c.class == "tank"));
        assert!(rules
            .circle_rule(2.1, &words(&["S"]))
            .is_some_and(|c| c.class == "s-point"));
        assert!(rules.circle_rule(1.12, &[]).is_none());
        assert!(rules.shapes.is_some(), "exploded family is configured");
        assert!(!rules.tag_classes.is_empty());
        assert_eq!(
            rules.blocks["$Standard$00000144"].port,
            Some(PortRule::StemEnd),
            "the vent stub joins pipe at the far end of its stem"
        );
        assert_eq!(rules.blocks["$TwtSys$00000132"].port, None);
    }

    #[test]
    fn a_stem_ends_where_the_block_stops_along_it() {
        // The vent stub as TWT draws it: a stem from the base point along -x
        // and a cup at its end, closed 245 units past the stem's own end.
        let lines: [Segment; 4] = [
            ((-2430.8, 0.0), (0.0, 0.0)),
            ((-2675.7, 252.7), (-2675.7, -252.7)),
            ((-2675.7, -252.7), (-2430.8, -252.7)),
            ((-2675.7, 252.7), (-2430.8, 252.7)),
        ];
        let corners: Vec<(f64, f64)> = lines
            .iter()
            .flat_map(|&(a, b)| {
                let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
                let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
                [(x0, y0), (x0, y1), (x1, y0), (x1, y1)]
            })
            .collect();
        let end = stem_end(&lines, &corners, (0.0, 0.0)).unwrap();
        assert!(
            (end.0 + 2675.7).abs() < 1e-9 && end.1.abs() < 1e-9,
            "{end:?}"
        );
        // A block whose lines are all far from its base has no stem.
        assert_eq!(stem_end(&lines[1..], &corners, (0.0, 0.0)), None);
        // A stem pointing the other way, base off the origin.
        let up: [Segment; 2] = [((10.0, 5.0), (10.0, 20.0)), ((8.0, 20.0), (12.0, 22.0))];
        let corners = [(10.0, 5.0), (10.0, 20.0), (8.0, 20.0), (12.0, 22.0)];
        let end = stem_end(&up, &corners, (10.0, 5.0)).unwrap();
        assert!(
            (end.0 - 10.0).abs() < 1e-9 && (end.1 - 22.0).abs() < 1e-9,
            "{end:?}"
        );
    }

    #[test]
    fn small_circles_are_told_apart_by_what_is_written_inside() {
        let rules = Rules::builtin();
        // The loading-island sheets letter their bubbles in circles of
        // 2.16 .. 3.5 mm; a 2.1 mm circle with a lone S is a spray point.
        assert!(rules
            .circle_rule(2.16, &words(&["HS", "0320B"]))
            .is_some_and(|c| c.class == "bubble"));
        assert!(rules
            .circle_rule(2.25, &words(&["LIA", "0302A"]))
            .is_some_and(|c| c.class == "bubble"));
        assert!(rules
            .circle_rule(2.1, &words(&["K"]))
            .is_some_and(|c| c.class == "s-point"));
        assert!(rules
            .circle_rule(1.0, &words(&["M"]))
            .is_some_and(|c| c.class == "motor"));
        assert!(rules
            .circle_rule(1.75, &words(&["HS", "0302A"]))
            .is_some_and(|c| c.class == "bubble"));
        assert!(rules
            .circle_rule(0.93, &words(&["D"]))
            .is_some_and(|c| c.class == "drain-point"));
        assert!(rules
            .circle_rule(5.19, &[])
            .is_some_and(|c| c.class == "pump"));
        // An unlettered 3 mm circle is a stroke of something, not a bubble.
        assert!(rules.circle_rule(3.0, &[]).is_none());
        // Nor is a circle with a lone actuator letter a bubble.
        assert!(rules.circle_rule(2.94, &words(&["E", "H"])).is_none());
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
        assert!(shape_matches("*DWG-*", "接 DWG-0100FF02-04"));
        assert!(shape_matches("*DWG-*", "DWG-0100FF02-04"));
        assert!(!shape_matches("*DWG-*", "接图"));
        assert!(shape_matches("BV9999*", "BV0301A"));
        assert!(shape_matches("BV9999*", "BV0301"));
        assert!(!shape_matches("BV9999*", "BV030"));
        assert!(!shape_matches("CV9999*", "CVV0319"));
        assert!(shape_matches("P-9999*", "P-0302A"));
    }

    #[test]
    fn inner_lettering_composes_a_bubble_tag() {
        assert_eq!(
            inner_tag(&words(&["XV", "3201"])),
            Some("XV-3201".to_string())
        );
        assert_eq!(
            inner_tag(&words(&["HS", "0320A"])),
            Some("HS-0320A".to_string())
        );
        assert_eq!(
            inner_tag(&words(&["FQRC", "0301"])),
            Some("FQRC-0301".to_string())
        );
        assert_eq!(inner_tag(&words(&["S"])), Some("S".to_string()));
        assert_eq!(inner_tag(&[]), None);
        assert_eq!(compose_tag(&words(&["S"])), None);
        assert_eq!(compose_tag(&words(&["E", "H"])), None);
    }

    #[test]
    fn legend_layer_names() {
        assert_eq!(layer_for_class("butterfly"), "PID-LEGEND-BUTTERFLY");
        assert_eq!(layer_for_class("shape-1a2b3c4d"), SHAPE_LAYER);
        assert_eq!(pipe_layer("100-FW"), "PID-PIPE-100-FW");
        assert!(is_legend_layer("PID-LEGEND-TANK"));
        assert!(is_legend_layer(SHAPE_LAYER));
        assert!(is_legend_layer("PID-PIPE-200-FS-31001-A2"));
        assert!(is_legend_layer(PIPE_NONE_LAYER));
        assert!(!is_legend_layer("VALVE_消防"));
        assert!(!is_legend_layer("PIPE-消防"));
    }

    #[test]
    fn pipe_runs_draw_on_a_layer_per_line_and_ring_their_open_ends() {
        use super::pid_pipes::{End, Run};
        let run = |lines: &[&str], ends: [End; 2], path: &[(f64, f64)]| Run {
            numbers: Vec::new(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
            segments: path.len() - 1,
            length_mm: 0.0,
            ends,
            path: path.to_vec(),
            handles: Vec::new(),
        };
        let recognition = Recognition {
            units_per_mm: 100.0,
            pipes: Pipes {
                runs: vec![
                    run(
                        &["100-FW"],
                        [End::Open, End::Tee],
                        &[(0.0, 0.0), (500.0, 0.0)],
                    ),
                    run(&[], [End::Tee, End::Open], &[(500.0, 0.0), (500.0, 300.0)]),
                    run(
                        &["100-FW", "80-FW"],
                        [End::Tee, End::Symbol(0)],
                        &[(500.0, 0.0), (900.0, 0.0), (900.0, -200.0)],
                    ),
                ],
                ..Pipes::default()
            },
            ..Recognition::default()
        };
        let layers = pipe_layers(&recognition);
        assert_eq!(
            layers.keys().collect::<Vec<_>>(),
            ["PID-PIPE-100-FW", "PID-PIPE-80-FW", PIPE_NONE_LAYER]
        );
        assert_eq!(layers[PIPE_NONE_LAYER], PIPE_NONE_COLOR);
        assert_ne!(layers["PID-PIPE-100-FW"], layers["PID-PIPE-80-FW"]);
        assert_eq!(
            layers["PID-PIPE-100-FW"],
            hash_color(fnv1a(b"100-FW")),
            "a line's colour comes from its number, the same on every sheet"
        );

        let rules = Rules::default();
        let entities = pipe_entities(&recognition, &rules);
        let on = |layer: &str| -> Vec<&EntityType> {
            entities
                .iter()
                .filter(|e| e.common().layer == layer)
                .collect()
        };
        // The header piece and the two-line piece; one ring at the open end.
        let header = on("PID-PIPE-100-FW");
        assert_eq!(header.len(), 3, "{header:?}");
        assert!(
            matches!(header[0], EntityType::LwPolyline(p) if p.vertices.len() == 2 && !p.is_closed)
        );
        assert!(matches!(header[1], EntityType::Circle(c)
            if c.center.x == 0.0 && c.center.y == 0.0 && (c.radius - 60.0).abs() < 1e-9));
        assert!(matches!(header[2], EntityType::LwPolyline(p) if p.vertices.len() == 3));
        // The two-line piece is on the second line's layer too, whole.
        let branch = on("PID-PIPE-80-FW");
        assert_eq!(branch.len(), 1);
        assert!(matches!(branch[0], EntityType::LwPolyline(p) if p.vertices.len() == 3));
        // The unnumbered stub, grey, ringed where it stops.
        let none = on(PIPE_NONE_LAYER);
        assert_eq!(none.len(), 2);
        assert!(matches!(none[1], EntityType::Circle(c) if c.center.y == 300.0));
        assert_eq!(entities.len(), 6);
        assert!(entities.iter().all(|e| e.common().color == Color::ByLayer));
        assert!(entities.iter().all(|e| is_legend_layer(&e.common().layer)));
    }

    fn bowtie(x: f64, y: f64, turn: fn((f64, f64)) -> (f64, f64)) -> Vec<Prim> {
        // Two triangles tip to tip, 2.4 x 1.2 mm, placed with `turn`.
        let p = |dx: f64, dy: f64| {
            let (tx, ty) = turn((dx, dy));
            (x + tx, y + ty)
        };
        vec![
            Prim::new(PrimKind::Line, vec![p(-1.2, -0.6), p(-1.2, 0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(-1.2, 0.6), p(1.2, -0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(1.2, -0.6), p(1.2, 0.6)], 0.0).unwrap(),
            Prim::new(PrimKind::Line, vec![p(1.2, 0.6), p(-1.2, -0.6)], 0.0).unwrap(),
        ]
    }

    #[test]
    fn a_shape_has_the_same_id_however_it_is_turned_or_mirrored() {
        let upright = bowtie(10.0, 10.0, |(x, y)| (x, y));
        let quarter = bowtie(50.0, 80.0, |(x, y)| (-y, x));
        let mirrored = bowtie(90.0, 20.0, |(x, y)| (-x, y));
        let other = bowtie(90.0, 20.0, |(x, y)| (x * 1.5, y));
        let id = |prims: &[Prim]| {
            let idxs: Vec<usize> = (0..prims.len()).collect();
            let mut bbox = None;
            for p in prims {
                grow(&mut bbox, p.bbox.0, p.bbox.1);
                grow(&mut bbox, p.bbox.2, p.bbox.3);
            }
            let b = bbox.unwrap();
            signature(prims, &idxs, ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0), 0.1)
        };
        assert_eq!(id(&upright), id(&quarter));
        assert_eq!(id(&upright), id(&mirrored));
        assert_ne!(id(&upright), id(&other), "a wider valve is another shape");
    }

    #[test]
    fn a_stroke_a_hair_off_square_is_still_an_axis_run() {
        let run = AxisRun::of((275.47, 241.07), (277.35, 241.03)).expect("0.04 mm off over 1.9 mm");
        assert_eq!(run.axis, Axis::Horizontal);
        assert!((run.len() - 1.88).abs() < 1e-9);
        assert!((run.at - 241.05).abs() < 1e-9);
        assert!(
            AxisRun::of((0.0, 0.0), (1.0, 0.3)).is_none(),
            "a slope is not a run"
        );
        assert!(
            AxisRun::of((1.0, 1.0), (1.0, 1.0)).is_none(),
            "nor is a point"
        );
        let v = AxisRun::of((5.0, 9.0), (5.02, 1.0)).unwrap();
        assert_eq!(v.axis, Axis::Vertical);
        assert!((v.at - 5.01).abs() < 1e-9 && v.lo == 1.0 && v.hi == 9.0);
        // Through a box's body, not along its edge.
        assert!(v.through((4.0, 0.0, 6.0, 10.0), 0.05));
        assert!(!v.through((5.0, 0.0, 6.0, 10.0), 0.05));
    }

    #[test]
    fn touching_strokes_form_one_component_and_a_gap_breaks_it() {
        let mut prims = bowtie(0.0, 0.0, |(x, y)| (x, y));
        prims.extend(bowtie(20.0, 0.0, |(x, y)| (x, y)));
        // A leader that ends on the first valve's right edge joins it.
        prims.push(Prim::new(PrimKind::Line, vec![(1.2, 0.0), (1.2, 4.0)], 0.0).unwrap());
        let groups = components(&prims, 0.05);
        let mut sizes: Vec<usize> = groups.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [4, 5]);
    }

    #[test]
    fn hash_colours_are_bright() {
        for h in [0u64, 1, 12345, u64::MAX, 0xdead_beef] {
            let [r, g, b] = hash_color(h);
            assert!(r.max(g).max(b) >= 200, "{h}: {r} {g} {b}");
        }
        assert_ne!(hash_color(1), hash_color(100));
    }
}

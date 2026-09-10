//! The rules: what a block, a circle, a layer prefix or a piece of lettering
//! means, read from `assets/pid-legend.json` or the file `OCS_PID_LEGEND_RULES`
//! names, and the tag-shape language (`shape_matches`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::blocks::compose_tag;
use super::pid_pipes::PipeRules;
use super::*;

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
    /// Take the tag of a recognised bubble whose function letters are these
    /// (`XV` for a motorised valve), paired one-to-one like lettering.
    pub bubble: Option<String>,
    /// Search radius for this class, paper mm; absent = `Rules::radius_mm`.
    pub radius_mm: Option<f64>,
    /// List lettering of this shape that no symbol claimed. Off for shapes
    /// that also occur where no symbol is expected (a sheet's own number in
    /// the title block has the shape of a sheet reference).
    pub report_orphans: bool,
    /// List where the symbols of this class that got no tag stand. Absent =
    /// same as `report_orphans`: a class whose tags are not worth chasing
    /// (the flow arrows, which only carry a sheet number at the sheet's edge)
    /// is not worth a line of coordinates either. Set it to keep the
    /// coordinates for a class that turns orphans off for another reason
    /// (the sheet connector, whose orphan would be the sheet's own number).
    pub report_untagged: Option<bool>,
}

impl Default for TagRule {
    fn default() -> Self {
        TagRule {
            shape: None,
            inner: false,
            bubble: None,
            radius_mm: None,
            report_orphans: true,
            report_untagged: None,
        }
    }
}

impl TagRule {
    pub(super) fn wants_tag(&self) -> bool {
        self.shape.is_some() || self.inner || self.bubble.is_some()
    }

    pub(super) fn reports_untagged(&self) -> bool {
        self.report_untagged.unwrap_or(self.report_orphans)
    }
}

/// How lettering that has the shape of a tag but that no symbol claimed is
/// sorted, `orphans` in the rules JSON.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct OrphanRules {
    /// Report lettering whose value a symbol already carries as its tag as
    /// an orphan too. Off (the default), such lettering -- an equipment
    /// table's row, an interlock table's cell -- is listed as a duplicate
    /// instead: not lost, not mistaken for a symbol that lost its tag.
    pub claimed_elsewhere: bool,
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
    pub(super) fn applies(&self, doc: &CadDocument) -> bool {
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
    /// Lettering shorter than this many characters is never a tag, whatever
    /// shape or inner rule would let it through: the lone `S` or `K` inside
    /// a spray point says what the symbol is, not which one it is.
    pub tag_min_chars: usize,
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
    /// How unclaimed tag-shaped lettering is sorted.
    pub orphans: OrphanRules,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            radius_mm: 15.0,
            tag_min_chars: 4,
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
            orphans: OrphanRules::default(),
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

    pub(super) fn layer_fallback(&self, layer: &str) -> Option<&LayerPrefixRule> {
        self.layer_prefixes
            .iter()
            .find(|r| layer.starts_with(&r.prefix))
    }

    /// Whether `value` is long enough to be a tag at all: `tag_min_chars`
    /// characters, surrounding whitespace not counted.
    pub fn accepts_tag(&self, value: &str) -> bool {
        value.trim().chars().count() >= self.tag_min_chars
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
        // The loading arm both ways the sheets draw it: the `11111` block with
        // its tag up to 25 mm from the body, and SP02-10's loose strokes,
        // whose bracket has one pipe-length stroke the second pass brings
        // back -- so one framed run is enough for a second-pass candidate.
        assert_eq!(rules.blocks["11111"].tag.shape.as_deref(), Some("LA-9999*"));
        assert_eq!(rules.blocks["11111"].tag.radius_mm, Some(25.0));
        let shapes = rules.shapes.as_ref().unwrap();
        assert_eq!(shapes.recover_min_runs, 1);
        let arm = &shapes.dictionary["b6f54271"];
        assert_eq!(arm.class, "loading-arm");
        assert_eq!(arm.tag.shape.as_deref(), Some("LA-9999*"));
        assert_eq!(
            rules.blocks["$Standard$00000144"].port,
            Some(PortRule::StemEnd),
            "the vent stub joins pipe at the far end of its stem"
        );
        assert_eq!(rules.blocks["$TwtSys$00000132"].port, None);
        // The sheet connector turns orphans off (the sheet's own number has
        // the shape) but keeps the coordinates of the connectors without one.
        let connector = &rules.blocks["$TwtSys$00000132"].tag;
        assert!(!connector.report_orphans && connector.reports_untagged());
        assert!(
            !rules.orphans.claimed_elsewhere,
            "a table row a symbol already carries is a duplicate, not an orphan"
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

    /// Four characters make a tag; the lone letter inside a spray point does
    /// not, and the limit counts characters, not bytes.
    #[test]
    fn a_tag_is_at_least_four_characters() {
        let rules = Rules::builtin();
        assert_eq!(rules.tag_min_chars, 4);
        for tag in ["XV-3201", "BUV-3101", "P-01", "接图 DWG-0100SP02-03"] {
            assert!(rules.accepts_tag(tag), "{tag}");
        }
        for not in ["S", "K", " S ", "V-1", "S点", ""] {
            assert!(!rules.accepts_tag(not), "{not:?}");
        }
        let lenient = Rules {
            tag_min_chars: 1,
            ..Rules::builtin()
        };
        assert!(lenient.accepts_tag("S"));
        assert!(!lenient.accepts_tag(" "));
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
}

//! SPPID-oriented symbol, multi-format tag pairing, pipe-segment/line-label
//! extraction, and three-tier confidence scoring (`L3-2` / `L3-7` / `#125`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use document::entities::Insert;
use document::{Document, EntityType};
use serde::{Deserialize, Serialize};

use super::class_map::{BlockResolution, LegendClassMap};
use crate::io::pid_legend::guess_units_per_mm;
use crate::io::pid_publish::format_nominal_diameter_attr;

/// The 10 SPPID taggable symbol classes evaluated by `scripts/sppid/legend_vs_xml.py`.
pub const TEN_SYMBOL_CLASSES: [&str; 10] = [
    "PIDProcessVessel",
    "PIDMechanicalEquipment",
    "PIDProcessEquipment",
    "PIDHeatTransferEquipment",
    "PIDInstrument",
    "PIDInlineInstrument",
    "PIDControlSystemFunction",
    "PIDPipingComponent",
    "PIDNozzle",
    "PIDBranchPoint",
];

const NON_FUNC_WORDS: &[&str] = &[
    "M", "S", "K", "D", "NO", "NC", "LO", "LC", "CSO", "CSC", "FO", "FC", "STOP", "OPEN", "CLOSE",
    "LOCAL", "REMOTE", "ON", "OFF", "AUTO", "MAN", "RESET", "MIN", "MAX", "TYP",
];

/// Configurable three-tier confidence thresholds (`L3-7`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceThresholds {
    pub high_min: f64,
    pub medium_min: f64,
}

impl Default for ConfidenceThresholds {
    fn default() -> Self {
        Self {
            high_min: 0.90,
            medium_min: 0.60,
        }
    }
}

impl ConfidenceThresholds {
    pub fn classify(self, score: f64) -> ConfidenceTier {
        if score >= self.high_min {
            ConfidenceTier::High
        } else if score >= self.medium_min {
            ConfidenceTier::Medium
        } else {
            ConfidenceTier::Low
        }
    }
}

/// Three-tier confidence bucket (`high >= 0.90`, `medium 0.60..0.89`, `low < 0.60`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
}

/// Sheet-level confidence and candidate conservation counts (`L3-7`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfidenceSummary {
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub ignored_blocks: usize,
    pub unmapped_blocks: usize,
}

impl ConfidenceSummary {
    pub fn total_symbols(&self) -> usize {
        self.high + self.medium + self.low
    }
}

/// One recognized SPPID symbol (block-backed or loose-geometry equipment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertedSymbol {
    pub sppid_class: String,
    pub variant_id: Option<String>,
    pub symbol_path: Option<String>,
    pub source_block: String,
    pub layer: String,
    pub at: (f64, f64),
    pub bbox: (f64, f64, f64, f64),
    pub ports: Vec<(f64, f64)>,
    pub tag: Option<String>,
    pub description: Option<String>,
    pub to_from_text: Option<String>,
    pub lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_diameter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_handle: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_handles: Vec<u64>,
    pub confidence: f64,
    pub tier: ConfidenceTier,
    pub known: bool,
}

/// Title-block metadata extracted from the lower-right frame (`L3-6` / `#132`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleBlockInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Line-number label detected on the sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipeLabelHit {
    pub raw_text: String,
    pub at: (f64, f64),
    pub layer: String,
}

/// Raw pipe or signal line segment extracted from the sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipeSegmentHit {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_handle: Option<u64>,
}

/// Range or slash-expanded multi-tag annotation (`XV-0407A/0407B`, `BUV-3101～3103`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeTagHit {
    pub raw_text: String,
    pub members: Vec<String>,
}

/// Full recognition output for one DWG/DXF sheet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SheetRecognition {
    pub units_per_mm: f64,
    pub symbols: Vec<ConvertedSymbol>,
    pub pipe_labels: Vec<PipeLabelHit>,
    pub pipe_segments: Vec<PipeSegmentHit>,
    pub orphan_tags: Vec<String>,
    pub range_tags: Vec<RangeTagHit>,
    #[serde(default)]
    pub title_block: TitleBlockInfo,
    pub confidence_summary: ConfidenceSummary,
}

impl SheetRecognition {
    /// Serialize to the JSON format consumed by `scripts/sppid/legend_vs_xml.py`
    /// and `scripts/sppid/legend_baseline.py`.
    pub fn to_legend_json(
        &self,
        file_name: &str,
        only_ten_symbol_classes: bool,
    ) -> serde_json::Value {
        let allowed: BTreeSet<&str> = TEN_SYMBOL_CLASSES.iter().copied().collect();
        let sym_vals: Vec<serde_json::Value> = self
            .symbols
            .iter()
            .filter(|s| {
                if !only_ten_symbol_classes {
                    true
                } else {
                    allowed.contains(s.sppid_class.as_str())
                }
            })
            .map(|s| {
                let tag_str = s.tag.clone();
                let label_str = tag_str.clone().unwrap_or_default();
                serde_json::json!({
                    "class": s.sppid_class,
                    "variant_id": s.variant_id,
                    "label": label_str,
                    "tag": tag_str,
                    "description": s.description,
                    "to_from_text": s.to_from_text,
                    "known": s.known,
                    "source": s.source_block,
                    "confidence": s.confidence,
                    "confidence_tier": s.tier,
                    "at": [s.at.0, s.at.1],
                    "bbox": [s.bbox.0, s.bbox.1, s.bbox.2, s.bbox.3],
                    "lines": s.lines,
                })
            })
            .collect();

        serde_json::json!({
            "file": file_name,
            "units_per_mm": self.units_per_mm,
            "symbols": sym_vals,
            "orphan_tags": self.orphan_tags,
            "unknown_blocks": {},
            "unknown_shapes": [],
            "confidence_summary": self.confidence_summary,
            "pipes": {
                "labels": self.pipe_labels.len(),
                "segments": self.pipe_segments.len(),
            }
        })
    }
}

#[derive(Debug, Clone)]
struct SheetText {
    value: String,
    layer: String,
    anchor: (f64, f64),
    center: (f64, f64),
    handle: u64,
}

fn sane(v: f64) -> bool {
    v.is_finite() && v.abs() < 1.0e12
}

fn transform_point(
    local: (f64, f64),
    base: (f64, f64),
    scale: (f64, f64),
    rotation: f64,
    ins_pt: (f64, f64),
) -> (f64, f64) {
    let x = (local.0 - base.0) * scale.0;
    let y = (local.1 - base.1) * scale.1;
    let (s, c) = rotation.sin_cos();
    (x * c - y * s + ins_pt.0, x * s + y * c + ins_pt.1)
}

fn grow_bbox(bbox: &mut Option<(f64, f64, f64, f64)>, p: (f64, f64)) {
    if !(sane(p.0) && sane(p.1)) {
        return;
    }
    *bbox = Some(match *bbox {
        None => (p.0, p.1, p.0, p.1),
        Some((x0, y0, x1, y1)) => (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1)),
    });
}

struct PlacedGeometry {
    at: (f64, f64),
    bbox: (f64, f64, f64, f64),
    ports: Vec<(f64, f64)>,
}

fn placed_insert_geometry(doc: &Document, ins: &Insert, upm: f64) -> PlacedGeometry {
    let insertion = (ins.insert_point.x, ins.insert_point.y);
    let Some(record) = doc.block_records.get(&ins.block_name) else {
        let h = upm;
        return PlacedGeometry {
            at: insertion,
            bbox: (
                insertion.0 - h,
                insertion.1 - h,
                insertion.0 + h,
                insertion.1 + h,
            ),
            ports: vec![insertion],
        };
    };
    let base = (record.base_point.x, record.base_point.y);
    let scale = (ins.x_scale(), ins.y_scale());
    let mut bbox = None;
    let mut ports = Vec::new();
    for member in doc.entities_in_block(&ins.block_name) {
        if let EntityType::Point(pt) = member {
            ports.push(transform_point(
                (pt.location.x, pt.location.y),
                base,
                scale,
                ins.rotation,
                insertion,
            ));
            continue;
        }
        if matches!(
            member,
            EntityType::Insert(_)
                | EntityType::Block(_)
                | EntityType::BlockEnd(_)
                | EntityType::Seqend(_)
                | EntityType::AttributeDefinition(_)
                | EntityType::Text(_)
                | EntityType::MText(_)
        ) {
            continue;
        }
        let bb = member.as_entity().bounding_box();
        for corner in [
            (bb.min.x, bb.min.y),
            (bb.min.x, bb.max.y),
            (bb.max.x, bb.min.y),
            (bb.max.x, bb.max.y),
        ] {
            let wp = transform_point(corner, base, scale, ins.rotation, insertion);
            grow_bbox(&mut bbox, wp);
        }
    }
    let h = upm;
    let bbox = bbox.unwrap_or((
        insertion.0 - h,
        insertion.1 - h,
        insertion.0 + h,
        insertion.1 + h,
    ));
    let at = ((bbox.0 + bbox.2) * 0.5, (bbox.1 + bbox.3) * 0.5);
    if ports.is_empty() {
        let span = (bbox.2 - bbox.0).max(bbox.3 - bbox.1);
        if span > 0.5 * upm {
            ports.push((bbox.0, at.1));
            ports.push((bbox.2, at.1));
            ports.push((at.0, bbox.1));
            ports.push((at.0, bbox.3));
        }
        ports.push(insertion);
    }
    PlacedGeometry { at, bbox, ports }
}

fn point_to_bbox_dist(p: (f64, f64), bbox: (f64, f64, f64, f64)) -> f64 {
    let dx = if p.0 < bbox.0 {
        bbox.0 - p.0
    } else if p.0 > bbox.2 {
        p.0 - bbox.2
    } else {
        0.0
    };
    let dy = if p.1 < bbox.1 {
        bbox.1 - p.1
    } else if p.1 > bbox.3 {
        p.1 - bbox.3
    } else {
        0.0
    };
    dx.hypot(dy)
}

/// Min-cost max-flow bipartite matching over `(cost, left_idx, right_idx)` candidate edges.
/// Returns the selected indices into `edges`.
fn min_cost_bipartite_match(edges: &[(f64, usize, usize)]) -> Vec<usize> {
    if edges.is_empty() {
        return Vec::new();
    }
    struct Arc {
        to: usize,
        cap: u8,
        cost: f64,
        rev: usize,
    }
    const SOURCE: usize = 0;
    const SINK: usize = 1;
    let mut left_nodes: BTreeMap<usize, usize> = BTreeMap::new();
    let mut right_nodes: BTreeMap<usize, usize> = BTreeMap::new();
    for &(_, l, r) in edges {
        left_nodes.entry(l).or_insert(0);
        right_nodes.entry(r).or_insert(0);
    }
    let mut n = 2;
    for slot in left_nodes.values_mut().chain(right_nodes.values_mut()) {
        *slot = n;
        n += 1;
    }
    let mut adj: Vec<Vec<Arc>> = (0..n).map(|_| Vec::new()).collect();
    let add_edge = |adj: &mut Vec<Vec<Arc>>, u: usize, v: usize, cost: f64| -> usize {
        let (iu, iv) = (adj[u].len(), adj[v].len());
        adj[u].push(Arc {
            to: v,
            cap: 1,
            cost,
            rev: iv,
        });
        adj[v].push(Arc {
            to: u,
            cap: 0,
            cost: -cost,
            rev: iu,
        });
        iu
    };
    for &ln in left_nodes.values() {
        add_edge(&mut adj, SOURCE, ln, 0.0);
    }
    for &rn in right_nodes.values() {
        add_edge(&mut adj, rn, SINK, 0.0);
    }
    let recorded: Vec<(usize, usize)> = edges
        .iter()
        .map(|&(cost, l, r)| {
            let u = left_nodes[&l];
            let v = right_nodes[&r];
            (u, add_edge(&mut adj, u, v, cost))
        })
        .collect();

    loop {
        let mut dist = vec![f64::INFINITY; n];
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
        let mut in_q = vec![false; n];
        let mut q = VecDeque::from([SOURCE]);
        dist[SOURCE] = 0.0;
        while let Some(u) = q.pop_front() {
            in_q[u] = false;
            for (ei, arc) in adj[u].iter().enumerate() {
                if arc.cap > 0 && dist[u] + arc.cost < dist[arc.to] - 1e-9 {
                    dist[arc.to] = dist[u] + arc.cost;
                    prev[arc.to] = Some((u, ei));
                    if !in_q[arc.to] {
                        in_q[arc.to] = true;
                        q.push_back(arc.to);
                    }
                }
            }
        }
        if !dist[SINK].is_finite() {
            break;
        }
        let mut cur = SINK;
        while let Some((u, ei)) = prev[cur] {
            let rev = adj[u][ei].rev;
            adj[u][ei].cap -= 1;
            adj[cur][rev].cap += 1;
            cur = u;
        }
    }

    recorded
        .into_iter()
        .enumerate()
        .filter(|(_, (u, ei))| adj[*u][*ei].cap == 0)
        .map(|(idx, _)| idx)
        .collect()
}

/// Whether `text` is a SPPID pipe line number (e.g. `25-CI-0201505-B9-EH40`,
/// `100-PG-020104-F9-`, `6"-PG-749-B5-P40`).
pub fn is_sppid_line_label(text: &str) -> bool {
    let s = text.trim().trim_end_matches('-');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 4 {
        return false;
    }
    let p0 = parts[0].trim();
    let has_size = !p0.is_empty()
        && p0.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && p0.chars().all(|c| {
            c.is_ascii_digit() || matches!(c, '"' | '/' | '.' | 'm' | 'M' | 'i' | 'n' | ' ')
        });
    if !has_size {
        return false;
    }
    let fluid = parts[1].trim();
    let seq = parts[2].trim();
    let spec = parts[3].trim();
    !fluid.is_empty()
        && fluid.len() <= 6
        && fluid
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && seq.len() >= 2
        && seq
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        && !spec.is_empty()
        && spec.len() <= 8
        && spec.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Expand slash or wave range tags (`XV-0407A/0407B`, `P-02015A/B`, `XV-0407～0409`).
pub fn expand_range_or_slash_tag(text: &str) -> Option<Vec<String>> {
    let s = text.trim();
    if s.starts_with("DWG-") || s.contains(' ') || is_sppid_line_label(s) {
        return None;
    }
    if let Some((left, right)) = s.split_once('/') {
        let left = left.trim();
        let right = right.trim();
        if left.len() < 4 || right.is_empty() || !left.chars().any(|c| c.is_ascii_digit()) {
            return None;
        }
        if !left
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
            || !right
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return None;
        }
        if right.len() < left.len() {
            let prefix_len = left.len() - right.len();
            let second = format!("{}{}", &left[..prefix_len], right);
            return Some(vec![left.to_string(), second]);
        }
    }
    for sep in ['～', '~'] {
        if let Some((left, right)) = s.split_once(sep) {
            let left = left.trim();
            let right = right.trim();
            if left.len() < 4 || right.is_empty() {
                return None;
            }
            let l_digits_start = left
                .rfind(|c: char| !c.is_ascii_alphanumeric())
                .map(|i| i + 1)
                .unwrap_or(0);
            let head = &left[..l_digits_start];
            let l_tail = &left[l_digits_start..];
            let l_num_str: String = l_tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            let l_sfx: String = l_tail.chars().skip(l_num_str.len()).collect();
            let r_num_str: String = right.chars().take_while(|c| c.is_ascii_digit()).collect();
            let r_sfx: String = right.chars().skip(r_num_str.len()).collect();
            if !l_num_str.is_empty() && l_num_str.len() == r_num_str.len() && l_sfx == r_sfx {
                if let (Ok(a), Ok(b)) = (l_num_str.parse::<u32>(), r_num_str.parse::<u32>()) {
                    if b > a && (b - a) <= 20 {
                        let width = l_num_str.len();
                        let members = (a..=b)
                            .map(|v| format!("{head}{v:0width$}{l_sfx}"))
                            .collect();
                        return Some(members);
                    }
                }
            }
        }
    }
    None
}

fn is_bubble_top_code(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || NON_FUNC_WORDS.contains(&t) {
        return false;
    }
    if t.len() <= 6 && t.chars().all(|c| c.is_ascii_uppercase()) {
        return true;
    }
    t.len() == 4 && t.starts_with('3') && t.chars().all(|c| c.is_ascii_digit())
}

fn is_bubble_bottom_seq(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 || t.len() > 10 {
        return false;
    }
    if !t.bytes().next().is_some_and(|b| b.is_ascii_digit()) {
        return false;
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits < 2 {
        return false;
    }
    let rest = &t[digits..];
    rest.len() <= 2 && rest.chars().all(|c| c.is_ascii_uppercase())
}

fn parse_single_line_bubble_tag(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    let (head, tail) = t.split_once('-')?;
    if is_bubble_top_code(head) && is_bubble_bottom_seq(tail) {
        Some((head.to_string(), format!("{head}-{tail}")))
    } else {
        None
    }
}

fn parse_equipment_tag(s: &str) -> Option<(&str, String)> {
    let t = s.trim();
    if t.starts_with("DWG-") || is_sppid_line_label(t) {
        return None;
    }
    let (pfx, seq) = t.split_once('-')?;
    if pfx.is_empty() || pfx.len() > 4 || !pfx.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    if NON_FUNC_WORDS.contains(&pfx) {
        return None;
    }
    if seq.len() < 3
        || seq.len() > 10
        || !seq.bytes().next().is_some_and(|b| b.is_ascii_digit())
        || !seq
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    Some((pfx, format!("{pfx}-{seq}")))
}

fn equipment_class_for_prefix(pfx: &str) -> (&'static str, &'static str) {
    match pfx {
        "V" | "T" | "R" | "C" | "D" | "F" | "S" | "TK" | "SEP" => {
            ("PIDProcessVessel", "PIDProcessVessel#01")
        }
        "P" | "K" | "B" | "CP" | "COMP" | "BL" | "MX" => {
            ("PIDMechanicalEquipment", "PIDMechanicalEquipment#01")
        }
        "E" | "H" | "AC" | "HE" | "HTR" => {
            ("PIDHeatTransferEquipment", "PIDHeatTransferEquipment#01")
        }
        _ => ("PIDProcessEquipment", "PIDProcessEquipment#01"),
    }
}

fn parse_valve_tag(s: &str) -> Option<(String, String)> {
    let t = s.trim();
    if t.starts_with("DWG-") || t.contains('=') || t.contains('"') || is_sppid_line_label(t) {
        return None;
    }
    if let Some((pfx, seq)) = t.split_once(' ') {
        let pfx = pfx.trim();
        let seq = seq.trim();
        if (1..=6).contains(&pfx.len())
            && pfx.chars().all(|c| c.is_ascii_uppercase())
            && !NON_FUNC_WORDS.contains(&pfx)
            && is_bubble_bottom_seq(seq)
        {
            return Some((pfx.to_string(), format!("{pfx}{seq}")));
        }
        return None;
    }
    if let Some((pfx, seq)) = t.split_once('-') {
        if (1..=6).contains(&pfx.len())
            && pfx.chars().all(|c| c.is_ascii_uppercase())
            && !NON_FUNC_WORDS.contains(&pfx)
            && is_bubble_bottom_seq(seq)
        {
            return Some((pfx.to_string(), format!("{pfx}-{seq}")));
        }
        return None;
    }
    let alpha_len = t.chars().take_while(|c| c.is_ascii_uppercase()).count();
    if !(1..=6).contains(&alpha_len) {
        return None;
    }
    let pfx = &t[..alpha_len];
    if matches!(pfx, "DN" | "PN" | "CL" | "EL") || NON_FUNC_WORDS.contains(&pfx) {
        return None;
    }
    let seq = &t[alpha_len..];
    if is_bubble_bottom_seq(seq) {
        Some((pfx.to_string(), format!("{pfx}{seq}")))
    } else {
        None
    }
}

fn is_naturally_untagged_symbol(sppid_class: &str, block_name: &str) -> bool {
    if matches!(
        sppid_class,
        "PIDNozzle" | "PIDNote" | "PIDPipingBranchPoint" | "PIDSignalBranchPoint"
    ) {
        return true;
    }
    let up = block_name.to_ascii_uppercase();
    [
        "FLANGE",
        "REDUCER",
        "SPECTACLE",
        "BLIND",
        "CAP",
        "PLUG",
        "CHECK VALVE",
        "ACTUATOR",
        "SOLENOID",
        "INTERLOCK",
        "SIGNAL",
        "HOSE",
        "COUPLING",
        "TRAP",
        "STRAINER",
        "SILENCER",
    ]
    .iter()
    .any(|kw| up.contains(kw))
}

/// Recognize SPPID symbols, tags, pipe segments/labels, and three-tier confidence on `doc`.
pub fn recognise_sppid_sheet(
    doc: &Document,
    map: &LegendClassMap,
    thresholds: ConfidenceThresholds,
) -> SheetRecognition {
    let upm = guess_units_per_mm(doc).max(1e-6);
    let ignore_layers: BTreeSet<String> = map
        .rules()
        .ignore_layers
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .collect();

    let mut symbols: Vec<ConvertedSymbol> = Vec::new();
    let mut summary = ConfidenceSummary::default();
    let mut texts: Vec<SheetText> = Vec::new();
    let mut all_texts: Vec<SheetText> = Vec::new();
    let mut pipe_segments: Vec<PipeSegmentHit> = Vec::new();
    let mut layer10_boxes: Vec<(f64, f64, f64, f64)> = Vec::new();
    let mut sheet_bounds: Option<(f64, f64, f64, f64)> = None;

    let mut include_pt = |p: (f64, f64)| {
        if !p.0.is_finite() || !p.1.is_finite() {
            return;
        }
        sheet_bounds = Some(match sheet_bounds {
            Some((x0, y0, x1, y1)) => (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1)),
            None => (p.0, p.1, p.0, p.1),
        });
    };

    for entity in doc.model_space_entities() {
        let ent_handle = entity.common().handle.value();
        let layer = entity.common().layer.trim();
        let layer_upper = layer.to_ascii_uppercase();

        match entity {
            EntityType::Insert(ins) => {
                let bname = ins.block_name.as_str();
                match map.resolve_block(bname, layer) {
                    BlockResolution::Ignored { .. } => {
                        summary.ignored_blocks += 1;
                    }
                    BlockResolution::Mapped(m) => {
                        let placed = placed_insert_geometry(doc, ins, upm);
                        symbols.push(ConvertedSymbol {
                            sppid_class: m.class,
                            variant_id: Some(m.variant_id),
                            symbol_path: m.symbol_path,
                            source_block: bname.to_string(),
                            layer: layer.to_string(),
                            at: placed.at,
                            bbox: placed.bbox,
                            ports: placed.ports,
                            tag: None,
                            description: None,
                            to_from_text: None,
                            lines: Vec::new(),
                            nominal_diameter: None,
                            entity_handle: Some(ent_handle),
                            tag_handles: Vec::new(),
                            confidence: 0.0,
                            tier: ConfidenceTier::Medium,
                            known: true,
                        });
                    }
                    BlockResolution::Unmapped { .. } => {
                        summary.unmapped_blocks += 1;
                        let placed = placed_insert_geometry(doc, ins, upm);
                        symbols.push(ConvertedSymbol {
                            sppid_class: "Unmapped".to_string(),
                            variant_id: None,
                            symbol_path: None,
                            source_block: bname.to_string(),
                            layer: layer.to_string(),
                            at: placed.at,
                            bbox: placed.bbox,
                            ports: placed.ports,
                            tag: None,
                            description: None,
                            to_from_text: None,
                            lines: Vec::new(),
                            nominal_diameter: None,
                            entity_handle: Some(ent_handle),
                            tag_handles: Vec::new(),
                            confidence: 0.35,
                            tier: thresholds.classify(0.35),
                            known: false,
                        });
                    }
                }
            }
            EntityType::Text(t) => {
                let val = t.value.trim();
                if !val.is_empty() {
                    let x0 = t.insertion_point.x;
                    let y0 = t.insertion_point.y;
                    let h = t.height.abs().max(2.0 * upm);
                    let width_est = (val.chars().count() as f64) * h * 0.65;
                    let center = match t.alignment_point {
                        Some(ap) if (ap.x - x0).hypot(ap.y - y0) > 1e-6 => (ap.x, ap.y + h * 0.4),
                        _ => (x0 + width_est * 0.5, y0 + h * 0.4),
                    };
                    include_pt((x0, y0));
                    include_pt(center);
                    let st = SheetText {
                        value: val.to_string(),
                        layer: layer.to_string(),
                        anchor: (x0, y0),
                        center,
                        handle: ent_handle,
                    };
                    if !ignore_layers.contains(&layer_upper) {
                        texts.push(SheetText {
                            value: st.value.clone(),
                            layer: st.layer.clone(),
                            anchor: st.anchor,
                            center: st.center,
                            handle: ent_handle,
                        });
                    }
                    all_texts.push(st);
                }
            }
            EntityType::MText(mt) => {
                let cleaned = mt.value.replace("\\P", " ");
                let val = cleaned.trim();
                if !val.is_empty() {
                    let p = (mt.insertion_point.x, mt.insertion_point.y);
                    include_pt(p);
                    let st = SheetText {
                        value: val.to_string(),
                        layer: layer.to_string(),
                        anchor: p,
                        center: p,
                        handle: ent_handle,
                    };
                    if !ignore_layers.contains(&layer_upper) {
                        texts.push(SheetText {
                            value: st.value.clone(),
                            layer: st.layer.clone(),
                            anchor: st.anchor,
                            center: st.center,
                            handle: ent_handle,
                        });
                    }
                    all_texts.push(st);
                }
            }
            EntityType::Line(l) => {
                let a = (l.start.x, l.start.y);
                let b = (l.end.x, l.end.y);
                include_pt(a);
                include_pt(b);
                if (a.0 - b.0).hypot(a.1 - b.1) > 1e-6 {
                    if layer == "10" {
                        layer10_boxes.push((
                            a.0.min(b.0),
                            a.1.min(b.1),
                            a.0.max(b.0),
                            a.1.max(b.1),
                        ));
                    }
                    if !ignore_layers.contains(&layer_upper) {
                        pipe_segments.push(PipeSegmentHit {
                            start: a,
                            end: b,
                            layer: layer.to_string(),
                            entity_handle: Some(ent_handle),
                        });
                    }
                }
            }
            EntityType::LwPolyline(pl) => {
                let pts: Vec<(f64, f64)> = pl
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect();
                for &pt in &pts {
                    include_pt(pt);
                }
                for w in pts.windows(2) {
                    if (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1) > 1e-6 {
                        if layer == "10" {
                            layer10_boxes.push((
                                w[0].0.min(w[1].0),
                                w[0].1.min(w[1].1),
                                w[0].0.max(w[1].0),
                                w[0].1.max(w[1].1),
                            ));
                        }
                        if !ignore_layers.contains(&layer_upper) {
                            pipe_segments.push(PipeSegmentHit {
                                start: w[0],
                                end: w[1],
                                layer: layer.to_string(),
                                entity_handle: Some(ent_handle),
                            });
                        }
                    }
                }
                if pl.is_closed && pts.len() > 2 {
                    let a = pts[pts.len() - 1];
                    let b = pts[0];
                    if layer == "10" {
                        layer10_boxes.push((
                            a.0.min(b.0),
                            a.1.min(b.1),
                            a.0.max(b.0),
                            a.1.max(b.1),
                        ));
                    }
                    if !ignore_layers.contains(&layer_upper) {
                        pipe_segments.push(PipeSegmentHit {
                            start: a,
                            end: b,
                            layer: layer.to_string(),
                            entity_handle: Some(ent_handle),
                        });
                    }
                }
            }
            EntityType::Polyline2D(pl) => {
                let pts: Vec<(f64, f64)> = pl
                    .vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y))
                    .collect();
                for &pt in &pts {
                    include_pt(pt);
                }
                for w in pts.windows(2) {
                    if (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1) > 1e-6 {
                        if layer == "10" {
                            layer10_boxes.push((
                                w[0].0.min(w[1].0),
                                w[0].1.min(w[1].1),
                                w[0].0.max(w[1].0),
                                w[0].1.max(w[1].1),
                            ));
                        }
                        if !ignore_layers.contains(&layer_upper) {
                            pipe_segments.push(PipeSegmentHit {
                                start: w[0],
                                end: w[1],
                                layer: layer.to_string(),
                                entity_handle: Some(ent_handle),
                            });
                        }
                    }
                }
                if pl.flags.is_closed() && pts.len() > 2 {
                    let a = pts[pts.len() - 1];
                    let b = pts[0];
                    if layer == "10" {
                        layer10_boxes.push((
                            a.0.min(b.0),
                            a.1.min(b.1),
                            a.0.max(b.0),
                            a.1.max(b.1),
                        ));
                    }
                    if !ignore_layers.contains(&layer_upper) {
                        pipe_segments.push(PipeSegmentHit {
                            start: a,
                            end: b,
                            layer: layer.to_string(),
                            entity_handle: Some(ent_handle),
                        });
                    }
                }
            }
            EntityType::Circle(c) if layer == "10" => {
                let r = c.radius.abs();
                layer10_boxes.push((
                    c.center.x - r,
                    c.center.y - r,
                    c.center.x + r,
                    c.center.y + r,
                ));
            }
            EntityType::Arc(a) if layer == "10" => {
                let r = a.radius.abs();
                layer10_boxes.push((
                    a.center.x - r,
                    a.center.y - r,
                    a.center.x + r,
                    a.center.y + r,
                ));
            }
            _ => {}
        }
    }

    let mut claimed_text = vec![false; texts.len()];
    let mut pipe_labels = Vec::new();
    let mut range_tags = Vec::new();

    // Pre-pass: claim pipe line numbers and range/slash annotations.
    for (ti, t) in texts.iter().enumerate() {
        if is_sppid_line_label(&t.value) {
            claimed_text[ti] = true;
            pipe_labels.push(PipeLabelHit {
                raw_text: t.value.clone(),
                at: t.center,
                layer: t.layer.clone(),
            });
            continue;
        }
        if let Some(members) = expand_range_or_slash_tag(&t.value) {
            claimed_text[ti] = true;
            range_tags.push(RangeTagHit {
                raw_text: t.value.clone(),
                members,
            });
        }
    }

    // Pass 1: Compose two-line instrument/control/inline bubble tags and pair with symbols.
    #[derive(Debug)]
    struct BubbleCand {
        func: String,
        tag: String,
        at: (f64, f64),
        text_indices: Vec<usize>,
    }

    let mut bubble_cands: Vec<BubbleCand> = Vec::new();

    // 1a: Two-line (top function code + bottom sequence code) composition.
    let top_indices: Vec<usize> = texts
        .iter()
        .enumerate()
        .filter(|(i, t)| !claimed_text[*i] && is_bubble_top_code(&t.value))
        .map(|(i, _)| i)
        .collect();
    let bot_indices: Vec<usize> = texts
        .iter()
        .enumerate()
        .filter(|(i, t)| !claimed_text[*i] && is_bubble_bottom_seq(&t.value))
        .map(|(i, _)| i)
        .collect();

    let mut tb_edges: Vec<(f64, usize, usize)> = Vec::new();
    for &ti in &top_indices {
        for &bi in &bot_indices {
            if ti == bi {
                continue;
            }
            let dy_mm = (texts[ti].center.1 - texts[bi].center.1) / upm;
            let dx_mm = (texts[ti].center.0 - texts[bi].center.0).abs() / upm;
            let dx_anchor_mm = (texts[ti].anchor.0 - texts[bi].anchor.0).abs() / upm;
            let eff_dx = dx_mm.min(dx_anchor_mm);
            if (1.2..=11.5).contains(&dy_mm) && eff_dx <= 10.5 {
                let mut cost = eff_dx.hypot(dy_mm - 5.2);
                if texts[ti].layer != texts[bi].layer {
                    cost += 4.0;
                }
                tb_edges.push((cost, ti, bi));
            }
        }
    }
    for chosen in min_cost_bipartite_match(&tb_edges) {
        let (_, ti, bi) = tb_edges[chosen];
        claimed_text[ti] = true;
        claimed_text[bi] = true;
        let func = texts[ti].value.trim().to_string();
        let seq = texts[bi].value.trim();
        let mid = (
            (texts[ti].center.0 + texts[bi].center.0) * 0.5,
            (texts[ti].center.1 + texts[bi].center.1) * 0.5,
        );
        bubble_cands.push(BubbleCand {
            tag: format!("{func}-{seq}"),
            func,
            at: mid,
            text_indices: vec![ti, bi],
        });
    }

    // 1b: Single-line hyphenated instrument tags on instrument layer ("12") or non-equipment layer.
    for (ti, t) in texts.iter().enumerate() {
        if claimed_text[ti] || t.layer == "10" {
            continue;
        }
        if let Some((func, full_tag)) = parse_single_line_bubble_tag(&t.value) {
            if t.layer == "12" {
                claimed_text[ti] = true;
                bubble_cands.push(BubbleCand {
                    func,
                    tag: full_tag,
                    at: t.center,
                    text_indices: vec![ti],
                });
            }
        }
    }

    // 1c: Pair bubble candidates 1-to-1 with PIDInstrument / PIDControlSystemFunction / PIDInlineInstrument.
    let inst_syms: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            matches!(
                s.sppid_class.as_str(),
                "PIDInstrument" | "PIDControlSystemFunction" | "PIDInlineInstrument"
            )
        })
        .map(|(idx, _)| idx)
        .collect();

    let mut inst_edges: Vec<(f64, usize, usize)> = Vec::new();
    for (ci, cand) in bubble_cands.iter().enumerate() {
        for &si in &inst_syms {
            let sym = &symbols[si];
            let d_bbox_mm = point_to_bbox_dist(cand.at, sym.bbox) / upm;
            let d_center_mm = (cand.at.0 - sym.at.0).hypot(cand.at.1 - sym.at.1) / upm;
            let max_r = if sym.sppid_class == "PIDInlineInstrument" {
                28.0
            } else {
                18.0
            };
            if d_bbox_mm <= max_r {
                let mut cost = d_bbox_mm * 0.7 + d_center_mm * 0.3;
                let blk_up = sym.source_block.to_ascii_uppercase();
                let pfx_dash = format!("{}-", cand.func);
                let pfx_space = format!("{} ", cand.func);
                if blk_up.starts_with(&pfx_dash) || blk_up.starts_with(&pfx_space) {
                    cost -= 12.0;
                }
                if d_center_mm <= 4.5
                    && matches!(
                        sym.sppid_class.as_str(),
                        "PIDInstrument" | "PIDControlSystemFunction"
                    )
                {
                    cost -= 6.0;
                }
                inst_edges.push((cost, ci, si));
            }
        }
    }

    let mut paired_cands = vec![false; bubble_cands.len()];
    for chosen in min_cost_bipartite_match(&inst_edges) {
        let (_, ci, si) = inst_edges[chosen];
        paired_cands[ci] = true;
        symbols[si].tag = Some(bubble_cands[ci].tag.clone());
        symbols[si].tag_handles = bubble_cands[ci]
            .text_indices
            .iter()
            .map(|&ti| texts[ti].handle)
            .collect();
    }
    let mut orphan_tags: Vec<String> = Vec::new();
    for (ci, cand) in bubble_cands.iter().enumerate() {
        if !paired_cands[ci] {
            // Release single-line non-12 text if unused, keep true instrument layer orphans.
            if cand.text_indices.len() == 2 || texts[cand.text_indices[0]].layer == "12" {
                orphan_tags.push(cand.tag.clone());
            } else {
                for &ti in &cand.text_indices {
                    claimed_text[ti] = false;
                }
            }
        }
    }

    // Pass 2: Equipment tags & loose-geometry vessel/equipment recovery on layer "10".
    let equip_syms: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.tag.is_none()
                && matches!(
                    s.sppid_class.as_str(),
                    "PIDProcessVessel"
                        | "PIDMechanicalEquipment"
                        | "PIDProcessEquipment"
                        | "PIDHeatTransferEquipment"
                )
        })
        .map(|(idx, _)| idx)
        .collect();

    let equip_texts: Vec<(usize, String, String)> = texts
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| {
            if claimed_text[ti] || t.layer == "12" {
                return None;
            }
            let (pfx, norm_tag) = parse_equipment_tag(&t.value)?;
            Some((ti, pfx.to_string(), norm_tag))
        })
        .collect();

    let mut eq_edges: Vec<(f64, usize, usize)> = Vec::new();
    for (eti, (ti, pfx, _)) in equip_texts.iter().enumerate() {
        let (expected_cls, _) = equipment_class_for_prefix(pfx);
        for &si in &equip_syms {
            let sym = &symbols[si];
            let d_mm = point_to_bbox_dist(texts[*ti].center, sym.bbox)
                .min(point_to_bbox_dist(texts[*ti].anchor, sym.bbox))
                / upm;
            if d_mm <= 65.0 {
                let mut cost = d_mm;
                if sym.sppid_class == expected_cls {
                    cost -= 18.0;
                }
                if texts[*ti].layer == "10" && sym.layer == "10" {
                    cost -= 6.0;
                }
                eq_edges.push((cost, eti, si));
            }
        }
    }
    let mut paired_eq_text = vec![false; equip_texts.len()];
    for chosen in min_cost_bipartite_match(&eq_edges) {
        let (_, eti, si) = eq_edges[chosen];
        let (ti, _, ref norm_tag) = equip_texts[eti];
        paired_eq_text[eti] = true;
        claimed_text[ti] = true;
        symbols[si].tag = Some(norm_tag.clone());
        symbols[si].tag_handles = vec![texts[ti].handle];
    }

    // Recover loose-geometry vessels / equipment on layer "10" that have no INSERT block.
    for (eti, (ti, pfx, norm_tag)) in equip_texts.into_iter().enumerate() {
        if paired_eq_text[eti] {
            continue;
        }
        if texts[ti].layer == "10" {
            claimed_text[ti] = true;
            let (cls, vid) = equipment_class_for_prefix(&pfx);
            let p = texts[ti].center;
            let r = 8.0 * upm;
            let base_bbox = (p.0 - r, p.1 - r, p.0 + r, p.1 + r);
            let shell_bbox = expand_bbox_over_layer10(
                base_bbox,
                &layer10_boxes,
                36.0 * upm,
                6.0 * upm,
                380.0 * upm,
            );
            symbols.push(ConvertedSymbol {
                sppid_class: cls.to_string(),
                variant_id: Some(vid.to_string()),
                symbol_path: None,
                source_block: "loose-equipment".to_string(),
                layer: texts[ti].layer.clone(),
                at: p,
                bbox: shell_bbox,
                ports: vec![p],
                tag: Some(norm_tag),
                description: None,
                to_from_text: None,
                lines: Vec::new(),
                nominal_diameter: None,
                entity_handle: Some(texts[ti].handle),
                tag_handles: vec![texts[ti].handle],
                confidence: 0.85,
                tier: thresholds.classify(0.85),
                known: true,
            });
        }
    }

    // Also absorb touching layer-"10" nozzle-neck stubs onto INSERT-backed equipment.
    for sym in &mut symbols {
        if sym.source_block != "loose-equipment"
            && matches!(
                sym.sppid_class.as_str(),
                "PIDProcessVessel"
                    | "PIDMechanicalEquipment"
                    | "PIDProcessEquipment"
                    | "PIDHeatTransferEquipment"
            )
        {
            sym.bbox = expand_bbox_over_layer10(
                sym.bbox,
                &layer10_boxes,
                5.0 * upm,
                4.5 * upm,
                320.0 * upm,
            );
        }
    }

    // Pass 3: Valve & inline component tags (`BV0201902C`, `BV 0201501B`, `PSV-0201`, etc.).
    let valve_syms: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.tag.is_none()
                && matches!(
                    s.sppid_class.as_str(),
                    "PIDPipingComponent" | "PIDInlineInstrument"
                )
        })
        .map(|(idx, _)| idx)
        .collect();

    let valve_texts: Vec<(usize, String, String)> = texts
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| {
            if claimed_text[ti] {
                return None;
            }
            let (pfx, norm_tag) = parse_valve_tag(&t.value)?;
            Some((ti, pfx, norm_tag))
        })
        .collect();

    let mut v_edges: Vec<(f64, usize, usize)> = Vec::new();
    for (vti, (ti, pfx, _)) in valve_texts.iter().enumerate() {
        for &si in &valve_syms {
            let sym = &symbols[si];
            let d_mm = point_to_bbox_dist(texts[*ti].anchor, sym.bbox)
                .min(point_to_bbox_dist(texts[*ti].center, sym.bbox))
                / upm;
            if d_mm <= 24.0 {
                let mut cost = d_mm;
                let blk_up = sym.source_block.to_ascii_uppercase();
                if (pfx == "BV" && blk_up.contains("BALL VALVE"))
                    || (pfx == "GLV" && blk_up.contains("GLOBE VALVE"))
                    || (pfx == "CV" && blk_up.contains("CHECK VALVE"))
                    || (pfx == "GV" && blk_up.contains("GATE VALVE"))
                    || (pfx == "BFV" && blk_up.contains("BUTTERFLY"))
                    || blk_up.starts_with(&format!("{pfx}-"))
                    || blk_up.starts_with(&format!("{pfx} "))
                {
                    cost -= 6.0;
                }
                v_edges.push((cost, vti, si));
            }
        }
    }
    for chosen in min_cost_bipartite_match(&v_edges) {
        let (_, vti, si) = v_edges[chosen];
        let (ti, _, ref norm_tag) = valve_texts[vti];
        claimed_text[ti] = true;
        symbols[si].tag = Some(norm_tag.clone());
        symbols[si].tag_handles = vec![texts[ti].handle];
    }

    // Pass 3b: Nozzle nominal diameter callout pairing (`DN50`, `2"`, `1-1/2"`, etc., `L3-5`).
    let noz_syms: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.sppid_class == "PIDNozzle")
        .map(|(idx, _)| idx)
        .collect();
    if !noz_syms.is_empty() {
        let noz_size_texts: Vec<(usize, String)> = texts
            .iter()
            .enumerate()
            .filter_map(|(ti, t)| {
                if claimed_text[ti] {
                    return None;
                }
                parse_nozzle_size_callout(&t.value).map(|nd| (ti, nd))
            })
            .collect();
        let mut nd_edges: Vec<(f64, usize, usize)> = Vec::new();
        for (nti, (ti, _)) in noz_size_texts.iter().enumerate() {
            for &si in &noz_syms {
                let d_mm = point_to_bbox_dist(texts[*ti].anchor, symbols[si].bbox)
                    .min(point_to_bbox_dist(texts[*ti].center, symbols[si].bbox))
                    / upm;
                if d_mm <= 14.0 {
                    nd_edges.push((d_mm, nti, si));
                }
            }
        }
        for chosen in min_cost_bipartite_match(&nd_edges) {
            let (_, nti, si) = nd_edges[chosen];
            let (ti, ref nd) = noz_size_texts[nti];
            claimed_text[ti] = true;
            symbols[si].nominal_diameter = Some(nd.clone());
        }
    }

    // Pass 4: Off-page connectors (`PIDBranchPoint`: `DWG-*` -> `to_from_text`, prose -> `description`).
    let opc_syms: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.sppid_class == "PIDBranchPoint")
        .map(|(idx, _)| idx)
        .collect();

    if !opc_syms.is_empty() {
        let dwg_texts: Vec<usize> = texts
            .iter()
            .enumerate()
            .filter(|(ti, t)| !claimed_text[*ti] && t.value.trim().starts_with("DWG-"))
            .map(|(ti, _)| ti)
            .collect();
        let mut dwg_edges: Vec<(f64, usize, usize)> = Vec::new();
        for &ti in &dwg_texts {
            for &si in &opc_syms {
                let d_mm = point_to_bbox_dist(texts[ti].anchor, symbols[si].bbox)
                    .min(point_to_bbox_dist(texts[ti].center, symbols[si].bbox))
                    / upm;
                if d_mm <= 40.0 {
                    dwg_edges.push((d_mm, ti, si));
                }
            }
        }
        for chosen in min_cost_bipartite_match(&dwg_edges) {
            let (_, ti, si) = dwg_edges[chosen];
            claimed_text[ti] = true;
            symbols[si].to_from_text = Some(texts[ti].value.clone());
        }

        let prose_texts: Vec<usize> = texts
            .iter()
            .enumerate()
            .filter(|(ti, t)| {
                if claimed_text[*ti] {
                    return false;
                }
                let v = t.value.trim();
                v.chars().count() >= 3
                    && (!v.is_ascii()
                        || v.contains("FROM ")
                        || v.contains("TO ")
                        || v.contains("UNIT "))
            })
            .map(|(ti, _)| ti)
            .collect();
        let mut prose_edges: Vec<(f64, usize, usize)> = Vec::new();
        for &ti in &prose_texts {
            for &si in &opc_syms {
                let d_mm = point_to_bbox_dist(texts[ti].anchor, symbols[si].bbox)
                    .min(point_to_bbox_dist(texts[ti].center, symbols[si].bbox))
                    / upm;
                if d_mm <= 40.0 {
                    prose_edges.push((d_mm, ti, si));
                }
            }
        }
        for chosen in min_cost_bipartite_match(&prose_edges) {
            let (_, ti, si) = prose_edges[chosen];
            claimed_text[ti] = true;
            symbols[si].description = Some(texts[ti].value.clone());
        }
    }

    // Assign final three-tier confidence scores and tally conserved counts.
    for sym in &mut symbols {
        let score = if !sym.known {
            0.35
        } else if sym.source_block == "loose-equipment" {
            0.85
        } else if sym.tag.is_some() || sym.to_from_text.is_some() {
            0.96
        } else if is_naturally_untagged_symbol(&sym.sppid_class, &sym.source_block) {
            0.92
        } else {
            0.75
        };
        sym.confidence = score;
        sym.tier = thresholds.classify(score);
        match sym.tier {
            ConfidenceTier::High => summary.high += 1,
            ConfidenceTier::Medium => summary.medium += 1,
            ConfidenceTier::Low => summary.low += 1,
        }
    }

    if !symbols.iter().any(|s| s.known) {
        if let Some(fused) = fuse_legacy_autocad_recognition(doc, map, thresholds, upm) {
            symbols = fused.symbols;
            summary = fused.summary;
            if let Some((segs, labels)) = fused.pipes {
                pipe_segments = segs;
                pipe_labels = labels;
            }
        }
    }

    let title_block = extract_title_block(&all_texts, sheet_bounds, upm);

    SheetRecognition {
        units_per_mm: upm,
        symbols,
        pipe_labels,
        pipe_segments,
        orphan_tags,
        range_tags,
        title_block,
        confidence_summary: summary,
    }
}

struct LegacyFusion {
    symbols: Vec<ConvertedSymbol>,
    summary: ConfidenceSummary,
    pipes: Option<(Vec<PipeSegmentHit>, Vec<PipeLabelHit>)>,
}

fn fuse_legacy_autocad_recognition(
    doc: &Document,
    map: &LegendClassMap,
    thresholds: ConfidenceThresholds,
    upm: f64,
) -> Option<LegacyFusion> {
    let rules = crate::io::pid_legend::Rules::load();
    let legacy = crate::io::pid_legend::recognise_with_units(doc, &rules, upm);

    let mut fused_syms = Vec::new();
    let mut fused_sum = ConfidenceSummary {
        unmapped_blocks: legacy.unknown_blocks.values().sum::<usize>()
            + legacy.unknown_shapes.iter().map(|u| u.count).sum::<usize>(),
        ..Default::default()
    };
    let mut mapped_business = 0usize;

    for ls in &legacy.symbols {
        if map.is_legacy_ignored(&ls.class) {
            fused_sum.ignored_blocks += 1;
            continue;
        }
        if !ls.known || ls.class.starts_with("shape-") || ls.class == "unknown" {
            continue;
        }
        let Some(ov) = map.resolve_legacy_class(&ls.class) else {
            fused_sum.unmapped_blocks += 1;
            let score = 0.35;
            let tier = thresholds.classify(score);
            match tier {
                ConfidenceTier::High => fused_sum.high += 1,
                ConfidenceTier::Medium => fused_sum.medium += 1,
                ConfidenceTier::Low => fused_sum.low += 1,
            }
            fused_syms.push(ConvertedSymbol {
                sppid_class: "Unmapped".to_string(),
                variant_id: None,
                symbol_path: None,
                source_block: ls.source.clone(),
                layer: ls.class.clone(),
                at: ls.at,
                bbox: ls.bbox,
                ports: vec![ls.at],
                tag: ls.tag.clone(),
                description: None,
                to_from_text: None,
                lines: ls.lines.clone(),
                nominal_diameter: None,
                entity_handle: ls.handles.first().map(|h| h.value()),
                tag_handles: ls.tag_handles.iter().map(|h| h.value()).collect(),
                confidence: score,
                tier,
                known: false,
            });
            continue;
        };

        mapped_business += 1;
        let ports = vec![
            (ls.bbox.0, ls.at.1),
            (ls.bbox.2, ls.at.1),
            (ls.at.0, ls.bbox.1),
            (ls.at.0, ls.bbox.3),
            ls.at,
        ];
        let (tag, to_from_text, description) = if ov.class == "PIDBranchPoint" {
            match ls.tag.as_deref() {
                Some(t) if t.contains("DWG-") => (None, Some(t.to_string()), None),
                Some(t) => (None, None, Some(t.to_string())),
                None => (None, None, None),
            }
        } else {
            (ls.tag.clone(), None, None)
        };

        let tag_handles: Vec<u64> = if !ls.tag_handles.is_empty() {
            ls.tag_handles.iter().map(|h| h.value()).collect()
        } else if ls.handles.len() > 1 && ls.source.starts_with("circle") {
            ls.handles[1..].iter().map(|h| h.value()).collect()
        } else {
            Vec::new()
        };

        let score = if tag.is_some() || to_from_text.is_some() || !ls.wants_tag {
            0.95
        } else {
            0.78
        };
        let tier = thresholds.classify(score);
        match tier {
            ConfidenceTier::High => fused_sum.high += 1,
            ConfidenceTier::Medium => fused_sum.medium += 1,
            ConfidenceTier::Low => fused_sum.low += 1,
        }

        fused_syms.push(ConvertedSymbol {
            sppid_class: ov.class.clone(),
            variant_id: Some(ov.variant_id.clone()),
            symbol_path: ov.symbol_path.clone(),
            source_block: ls.source.clone(),
            layer: ls.class.clone(),
            at: ls.at,
            bbox: ls.bbox,
            ports,
            tag,
            description,
            to_from_text,
            lines: ls.lines.clone(),
            nominal_diameter: None,
            entity_handle: ls.handles.first().map(|h| h.value()),
            tag_handles,
            confidence: score,
            tier,
            known: true,
        });
    }

    if mapped_business == 0 {
        return None;
    }

    let pipes = if !legacy.pipes.runs.is_empty() {
        let mut new_segs = Vec::new();
        let mut new_labels = Vec::new();
        for run in &legacy.pipes.runs {
            let first_h = run.handles.first().map(|h| h.value());
            for w in run.path.windows(2) {
                if (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1) > 1e-6 {
                    new_segs.push(PipeSegmentHit {
                        start: w[0],
                        end: w[1],
                        layer: "PIPE".to_string(),
                        entity_handle: first_h,
                    });
                }
            }
            if let Some(lbl) = run.lines.first().or_else(|| run.numbers.first()) {
                let mid_idx = run.path.len() / 2;
                let anchor = if run.path.len() >= 2 && mid_idx > 0 {
                    let a = run.path[mid_idx - 1];
                    let b = run.path[mid_idx];
                    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
                } else {
                    run.path.first().copied().unwrap_or((0.0, 0.0))
                };
                new_labels.push(PipeLabelHit {
                    raw_text: lbl.clone(),
                    at: anchor,
                    layer: "PIPE".to_string(),
                });
            }
        }
        Some((new_segs, new_labels))
    } else {
        None
    };

    Some(LegacyFusion {
        symbols: fused_syms,
        summary: fused_sum,
        pipes,
    })
}

fn rect_gap(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let dx = if a.2 < b.0 {
        b.0 - a.2
    } else if b.2 < a.0 {
        a.0 - b.2
    } else {
        0.0
    };
    let dy = if a.3 < b.1 {
        b.1 - a.3
    } else if b.3 < a.1 {
        a.1 - b.3
    } else {
        0.0
    };
    dx.hypot(dy)
}

fn expand_bbox_over_layer10(
    seed: (f64, f64, f64, f64),
    boxes: &[(f64, f64, f64, f64)],
    seed_tol: f64,
    link_tol: f64,
    max_span: f64,
) -> (f64, f64, f64, f64) {
    if boxes.is_empty() {
        return seed;
    }
    let mut cur = seed;
    let mut used = vec![false; boxes.len()];
    let mut queue: VecDeque<usize> = VecDeque::new();

    for (i, &b) in boxes.iter().enumerate() {
        if (b.2 - b.0) > max_span || (b.3 - b.1) > max_span {
            continue;
        }
        if rect_gap(seed, b) <= seed_tol {
            let nx0 = cur.0.min(b.0);
            let ny0 = cur.1.min(b.1);
            let nx1 = cur.2.max(b.2);
            let ny1 = cur.3.max(b.3);
            if (nx1 - nx0) <= max_span && (ny1 - ny0) <= max_span {
                cur = (nx0, ny0, nx1, ny1);
                used[i] = true;
                queue.push_back(i);
            }
        }
    }

    while let Some(u) = queue.pop_front() {
        let ub = boxes[u];
        for (i, &b) in boxes.iter().enumerate() {
            if used[i] || (b.2 - b.0) > max_span || (b.3 - b.1) > max_span {
                continue;
            }
            if rect_gap(ub, b) <= link_tol {
                let nx0 = cur.0.min(b.0);
                let ny0 = cur.1.min(b.1);
                let nx1 = cur.2.max(b.2);
                let ny1 = cur.3.max(b.3);
                if (nx1 - nx0) <= max_span && (ny1 - ny0) <= max_span {
                    cur = (nx0, ny0, nx1, ny1);
                    used[i] = true;
                    queue.push_back(i);
                }
            }
        }
    }
    cur
}

fn parse_nozzle_size_callout(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    if upper.starts_with("DN")
        || upper.ends_with("MM")
        || upper.ends_with("IN")
        || s.ends_with('"')
        || s.ends_with("''")
    {
        return format_nominal_diameter_attr(s);
    }
    if matches!(
        s,
        "15" | "20"
            | "25"
            | "32"
            | "40"
            | "50"
            | "65"
            | "80"
            | "100"
            | "125"
            | "150"
            | "200"
            | "250"
            | "300"
            | "350"
            | "400"
            | "450"
            | "500"
            | "600"
    ) {
        return format_nominal_diameter_attr(s);
    }
    None
}

fn extract_title_block(
    all_texts: &[SheetText],
    sheet_bounds: Option<(f64, f64, f64, f64)>,
    upm: f64,
) -> TitleBlockInfo {
    let Some((min_x, min_y, max_x, max_y)) = sheet_bounds else {
        return TitleBlockInfo::default();
    };
    let w = (max_x - min_x).max(1.0);
    let h = (max_y - min_y).max(1.0);

    let tb_cands: Vec<&SheetText> = all_texts
        .iter()
        .filter(|t| {
            let lu = t.layer.to_ascii_uppercase();
            let in_corner = (t.anchor.0 - min_x) / w >= 0.55 && (t.anchor.1 - min_y) / h <= 0.32;
            let on_tb_layer = (lu == "LABELS" || lu.contains("TITLE") || lu.contains("BORDER"))
                && (t.anchor.1 - min_y) / h <= 0.36
                && (t.anchor.0 - min_x) / w >= 0.45;
            in_corner || on_tb_layer
        })
        .collect();

    if tb_cands.is_empty() {
        return TitleBlockInfo::default();
    }

    // 1. Drawing number: prefer `DWG-*`, else drawing-code token in lower-right corner.
    let mut drawing_number: Option<String> = None;
    for t in &tb_cands {
        let v: String = t
            .value
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if v.starts_with("DWG-") && v.chars().count() >= 6 {
            drawing_number = Some(v);
            break;
        }
    }
    if drawing_number.is_none() {
        for t in &tb_cands {
            let v: String = t
                .value
                .trim()
                .trim_end_matches('-')
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let char_n = v.chars().count();
            if (6..=28).contains(&char_n)
                && !is_sppid_line_label(&t.value)
                && v.chars().any(|c| c.is_ascii_digit())
                && !v.ends_with("流程图")
                && !v.ends_with("系统图")
                && (v.contains('-') || (v.is_ascii() && char_n >= 7))
                && v.chars().all(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, '-' | '_' | '.' | '/')
                        || ('\u{4e00}'..='\u{9fff}').contains(&c)
                })
            {
                drawing_number = Some(v);
                break;
            }
        }
    }

    // 2. Drawing title: prefer text ending in `流程图` / `系统图` / `P&ID` / `DIAGRAM`, optionally paired with unit prefix line.
    let is_title_noise = |s: &str| -> bool {
        matches!(
            s,
            "图号"
                | "图名"
                | "版次"
                | "比例"
                | "日期"
                | "设计"
                | "校核"
                | "审核"
                | "审定"
                | "批准"
        )
    };
    let mut primary_title: Option<&SheetText> = None;
    for t in &tb_cands {
        let v = t.value.trim();
        let vu = v.to_ascii_uppercase();
        if (v.ends_with("流程图")
            || v.ends_with("系统图")
            || vu.contains("P&ID")
            || vu.contains("FLOW DIAGRAM"))
            && primary_title.is_none_or(|prev| v.chars().count() > prev.value.chars().count())
        {
            primary_title = Some(t);
        }
    }
    if primary_title.is_none() {
        for t in &tb_cands {
            let v = t.value.trim();
            if !v.is_ascii()
                && v.chars().count() >= 6
                && !is_title_noise(v)
                && t.layer.eq_ignore_ascii_case("Labels")
                && primary_title.is_none_or(|prev| v.chars().count() > prev.value.chars().count())
            {
                primary_title = Some(t);
            }
        }
    }

    let drawing_title = primary_title.map(|pt| {
        let base = pt.value.trim();
        if base.contains(' ') {
            return base.to_string();
        }
        // Check for a stacked station/unit line in the same title-block area (`单元` / `站`)
        let mut prefix_cand: Option<&SheetText> = None;
        for t in &tb_cands {
            if std::ptr::eq(*t, pt) {
                continue;
            }
            let v = t.value.trim();
            if !v.is_ascii()
                && v.chars().count() >= 4
                && !is_title_noise(v)
                && (v.contains("单元") || v.contains('站'))
                && !base.contains(v)
                && (t.anchor.1 - pt.anchor.1).abs() <= 26.0 * upm
                && (t.anchor.0 - pt.anchor.0).abs() <= 80.0 * upm
                && prefix_cand.is_none_or(|p| v.chars().count() > p.value.chars().count())
            {
                prefix_cand = Some(t);
            }
        }
        match prefix_cand {
            Some(pfx) => format!("{} {base}", pfx.value.trim()),
            None => base.to_string(),
        }
    });

    // 3. Revision: look for `版次` / `版` / `REV` label and nearest short revision token.
    let mut revision: Option<String> = None;
    for lbl in &tb_cands {
        let lv = lbl.value.trim();
        let lvu = lv.to_ascii_uppercase();
        if lv == "版次" || lv == "版" || lvu == "REV" || lvu == "REV." {
            let mut best_rev: Option<(f64, &str)> = None;
            for cand in &tb_cands {
                if std::ptr::eq(*cand, *lbl) {
                    continue;
                }
                let cv = cand.value.trim();
                if (1..=4).contains(&cv.len())
                    && cv.is_ascii()
                    && cv.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
                {
                    let dx = (cand.anchor.0 - lbl.anchor.0) / upm;
                    let dy = (cand.anchor.1 - lbl.anchor.1).abs() / upm;
                    if (-5.0..=40.0).contains(&dx) && dy <= 16.0 {
                        let d = dx.abs().hypot(dy);
                        if best_rev.is_none_or(|(bd, _)| d < bd) {
                            best_rev = Some((d, cv));
                        }
                    }
                }
            }
            if let Some((_, rv)) = best_rev {
                revision = Some(rv.to_string());
                break;
            }
        }
    }

    TitleBlockInfo {
        drawing_number,
        drawing_title,
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_convert::default_legend_class_map;
    use document::entities::{Block, BlockEnd, Insert, Line, Point, Text};
    use document::tables::BlockRecord;
    use document::types::Vector3;
    use document::{EntityType, Handle};

    fn define_block(doc: &mut Document, name: &str, members: Vec<EntityType>) {
        if doc.block_records.get(name).is_some() {
            return;
        }
        let next = doc.next_handle();
        let br_handle = Handle::new(next);
        let block_handle = Handle::new(next + 1);
        let end_handle = Handle::new(next + 2);
        let mut record = BlockRecord::new(name);
        record.handle = br_handle;
        record.block_entity_handle = block_handle;
        record.block_end_handle = end_handle;
        doc.block_records.add(record).unwrap();
        let mut block = Block::new(name, Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = br_handle;
        doc.add_entity(EntityType::Block(block)).unwrap();
        let mut end = BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = br_handle;
        doc.add_entity(EntityType::BlockEnd(end)).unwrap();
        for mut m in members {
            m.common_mut().owner_handle = br_handle;
            doc.add_entity(m).unwrap();
        }
    }

    fn box_lines(w: f64, h: f64) -> Vec<EntityType> {
        let mut p1 = Point::new();
        p1.location = Vector3::new(-w / 2.0, 0.0, 0.0);
        let mut p2 = Point::new();
        p2.location = Vector3::new(w / 2.0, 0.0, 0.0);
        vec![
            EntityType::Line(Line::from_points(
                Vector3::new(-w / 2.0, -h / 2.0, 0.0),
                Vector3::new(w / 2.0, h / 2.0, 0.0),
            )),
            EntityType::Point(p1),
            EntityType::Point(p2),
        ]
    }

    fn add_ins(doc: &mut Document, block: &str, x: f64, y: f64, layer: &str) {
        let mut ins = Insert::new(block, Vector3::new(x, y, 0.0));
        ins.common.layer = layer.to_string();
        doc.add_entity(EntityType::Insert(ins)).unwrap();
    }

    fn add_txt(doc: &mut Document, val: &str, x: f64, y: f64, layer: &str) {
        let mut t = Text::with_value(val, Vector3::new(x, y, 0.0)).with_height(2.5);
        t.common.layer = layer.to_string();
        doc.add_entity(EntityType::Text(t)).unwrap();
    }

    #[test]
    fn composes_two_line_bubbles_for_instruments_dcs_and_inline_valves() {
        let mut doc = Document::new();
        define_block(&mut doc, "PG-PRESSURE GAUGE", box_lines(8.0, 8.0));
        define_block(
            &mut doc,
            "DCS FUNC ACCESS IN PRIME LOC",
            box_lines(10.0, 10.0),
        );
        define_block(
            &mut doc,
            "MOV-GLV-ELECTRICALLY OPERATED VALVE WITH FLANGE 02",
            box_lines(10.0, 10.0),
        );

        add_ins(&mut doc, "PG-PRESSURE GAUGE", 100.0, 200.0, "12");
        add_txt(&mut doc, "PG", 98.0, 201.4, "12");
        add_txt(&mut doc, "0201501A", 94.0, 196.1, "12");

        add_ins(&mut doc, "DCS FUNC ACCESS IN PRIME LOC", 200.0, 200.0, "12");
        add_txt(&mut doc, "PIA", 198.0, 201.4, "12");
        add_txt(&mut doc, "0201301D", 194.0, 196.1, "12");

        add_ins(
            &mut doc,
            "MOV-GLV-ELECTRICALLY OPERATED VALVE WITH FLANGE 02",
            300.0,
            200.0,
            "12",
        );
        add_txt(&mut doc, "MOV", 306.0, 212.0, "12");
        add_txt(&mut doc, "0201101C", 303.0, 206.8, "12");

        let rec = recognise_sppid_sheet(
            &doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        assert_eq!(rec.symbols.len(), 3);
        assert_eq!(rec.symbols[0].sppid_class, "PIDInstrument");
        assert_eq!(rec.symbols[0].tag.as_deref(), Some("PG-0201501A"));
        assert_eq!(rec.symbols[1].sppid_class, "PIDControlSystemFunction");
        assert_eq!(rec.symbols[1].tag.as_deref(), Some("PIA-0201301D"));
        assert_eq!(rec.symbols[2].sppid_class, "PIDInlineInstrument");
        assert_eq!(rec.symbols[2].tag.as_deref(), Some("MOV-0201101C"));
    }

    #[test]
    fn normalizes_spaced_7digit_valve_tags_and_resolves_bipartite_nearest() {
        let mut doc = Document::new();
        define_block(&mut doc, "BALL VALVE TYPE 2", box_lines(6.0, 4.0));
        define_block(&mut doc, "GLOBE VALVE TYPE 2", box_lines(6.0, 4.0));

        add_ins(&mut doc, "BALL VALVE TYPE 2", 100.0, 100.0, "11");
        add_ins(&mut doc, "GLOBE VALVE TYPE 2", 112.0, 100.0, "11");

        add_txt(&mut doc, "4\"", 99.0, 95.0, "11");
        add_txt(&mut doc, "NO", 111.0, 105.0, "11");

        add_txt(&mut doc, "BV 0201501B", 94.0, 103.5, "11");
        add_txt(&mut doc, "GLV 0201501A", 115.0, 103.5, "11");

        let rec = recognise_sppid_sheet(
            &doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        assert_eq!(rec.symbols.len(), 2);
        assert_eq!(rec.symbols[0].tag.as_deref(), Some("BV0201501B"));
        assert_eq!(rec.symbols[1].tag.as_deref(), Some("GLV0201501A"));
    }

    #[test]
    fn pairs_equipment_recovers_loose_vessels_expands_ranges_and_conserves_confidence_counts() {
        let mut doc = Document::new();
        define_block(&mut doc, "METERING PUMP", box_lines(14.0, 14.0));
        define_block(&mut doc, "_OFF-UNIT SQ", box_lines(12.0, 8.0));
        define_block(&mut doc, "REMARKS2", box_lines(20.0, 10.0));
        define_block(&mut doc, "UNKNOWN_PLANT_SYM", box_lines(6.0, 6.0));

        add_ins(&mut doc, "METERING PUMP", 150.0, 300.0, "10");
        add_txt(&mut doc, "P-02015A", 150.0, 315.0, "10");

        add_txt(&mut doc, "V-02019", 60.0, 320.0, "10");

        add_ins(&mut doc, "_OFF-UNIT SQ", 950.0, 200.0, "11");
        add_txt(&mut doc, "DWG-0001-05", 935.0, 199.0, "11");
        add_txt(&mut doc, "FROM UNIT 100", 932.0, 204.0, "11");

        add_ins(&mut doc, "REMARKS2", 400.0, 50.0, "Default");
        add_ins(&mut doc, "UNKNOWN_PLANT_SYM", 500.0, 300.0, "11");
        add_txt(&mut doc, "XV-0407A/0407B", 500.0, 420.0, "11");
        add_txt(&mut doc, "25-CI-0201505-B9-EH40", 200.0, 300.0, "11");

        let rec = recognise_sppid_sheet(
            &doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        assert_eq!(
            rec.confidence_summary.total_symbols(),
            rec.symbols.len(),
            "three-tier confidence buckets must sum to total recognized symbols"
        );
        assert_eq!(rec.confidence_summary.ignored_blocks, 1);
        assert_eq!(rec.confidence_summary.unmapped_blocks, 1);
        assert_eq!(rec.confidence_summary.low, 1);

        let pump = rec
            .symbols
            .iter()
            .find(|s| s.sppid_class == "PIDMechanicalEquipment")
            .expect("pump");
        assert_eq!(pump.tag.as_deref(), Some("P-02015A"));
        assert_eq!(pump.tier, ConfidenceTier::High);

        let vessel = rec
            .symbols
            .iter()
            .find(|s| s.sppid_class == "PIDProcessVessel")
            .expect("loose vessel recovered");
        assert_eq!(vessel.tag.as_deref(), Some("V-02019"));
        assert_eq!(vessel.tier, ConfidenceTier::Medium);

        let opc = rec
            .symbols
            .iter()
            .find(|s| s.sppid_class == "PIDBranchPoint")
            .expect("offpage connector");
        assert_eq!(opc.to_from_text.as_deref(), Some("DWG-0001-05"));
        assert_eq!(opc.description.as_deref(), Some("FROM UNIT 100"));

        assert_eq!(rec.range_tags.len(), 1);
        assert_eq!(
            rec.range_tags[0].members,
            vec!["XV-0407A".to_string(), "XV-0407B".to_string()]
        );
        assert_eq!(rec.pipe_labels.len(), 1);
        assert_eq!(rec.pipe_labels[0].raw_text, "25-CI-0201505-B9-EH40");
    }

    #[test]
    fn expands_loose_vessel_shell_bbox_extracts_nozzle_size_and_reads_title_block() {
        let mut doc = Document::new();
        define_block(&mut doc, "FLANGED NOZZLE WITH BLIND", box_lines(4.0, 4.0));

        // Sheet border + lower-right title block on `Labels`
        let mut border = Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1189.0, 841.0, 0.0),
        );
        border.common.layer = "Default".to_string();
        doc.add_entity(EntityType::Line(border)).unwrap();
        add_txt(&mut doc, "DWG-0202GP06-01", 1120.0, 24.0, "Labels");
        add_txt(
            &mut doc,
            "分离单元 生产分离器工艺及自控流程图",
            1080.0,
            38.0,
            "Labels",
        );
        add_txt(&mut doc, "版次", 1045.0, 24.0, "Labels");
        add_txt(&mut doc, "B", 1055.0, 24.0, "Labels");

        // Loose-geometry vessel shell on layer "10": x in 200..260, y in 300..440
        for (p1, p2) in [
            ((200.0, 300.0), (260.0, 300.0)),
            ((260.0, 300.0), (260.0, 440.0)),
            ((260.0, 440.0), (200.0, 440.0)),
            ((200.0, 440.0), (200.0, 300.0)),
        ] {
            let mut ln =
                Line::from_points(Vector3::new(p1.0, p1.1, 0.0), Vector3::new(p2.0, p2.1, 0.0));
            ln.common.layer = "10".to_string();
            doc.add_entity(EntityType::Line(ln)).unwrap();
        }
        add_txt(&mut doc, "V-02021", 225.0, 370.0, "10");

        // Nozzle mounted on outer shell at x=262, y=410 (65 mm away from text center!) with DN50 callout
        add_ins(&mut doc, "FLANGED NOZZLE WITH BLIND", 262.0, 410.0, "10");
        add_txt(&mut doc, "DN50", 266.0, 414.0, "10");

        let rec = recognise_sppid_sheet(
            &doc,
            default_legend_class_map(),
            ConfidenceThresholds::default(),
        );
        assert_eq!(
            rec.title_block.drawing_number.as_deref(),
            Some("DWG-0202GP06-01")
        );
        assert_eq!(
            rec.title_block.drawing_title.as_deref(),
            Some("分离单元 生产分离器工艺及自控流程图")
        );
        assert_eq!(rec.title_block.revision.as_deref(), Some("B"));

        let vessel = rec
            .symbols
            .iter()
            .find(|s| s.sppid_class == "PIDProcessVessel")
            .expect("loose vessel");
        assert!(
            vessel.bbox.0 <= 200.5 && vessel.bbox.2 >= 259.5 && vessel.bbox.3 >= 439.5,
            "loose vessel bbox should encompass layer-10 shell: {:?}",
            vessel.bbox
        );

        let nozzle = rec
            .symbols
            .iter()
            .find(|s| s.sppid_class == "PIDNozzle")
            .expect("nozzle");
        assert_eq!(nozzle.nominal_diameter.as_deref(), Some("50 mm"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "exports 90-DXF recognition JSON cache for legend_baseline.py evaluation"]
    fn real_90_dxf_export_recognition_cache() {
        let Ok(corpus_dir) = std::env::var("OCS_PID_CORPUS_DIR") else {
            println!("未设置语料目录（OCS_PID_CORPUS_DIR），跳过 90 图识别缓存生成。");
            return;
        };
        let Ok(manifest_path) = std::env::var("OCS_PID_MANIFEST_PATH") else {
            println!("未设置清单路径（OCS_PID_MANIFEST_PATH），跳过 90 图识别缓存生成。");
            return;
        };
        let Ok(cache_dir) = std::env::var("OCS_PID_REC_CACHE_DIR") else {
            println!("未设置输出目录（OCS_PID_REC_CACHE_DIR），跳过 90 图识别缓存生成。");
            return;
        };
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let dxf_sub = manifest["sections"]["EXPORT_DXF"]["subdir"]
            .as_str()
            .unwrap();
        let mut dxf_base = std::path::PathBuf::from(&corpus_dir);
        for seg in dxf_sub.split('/') {
            dxf_base.push(seg);
        }
        let cache_root = std::path::PathBuf::from(&cache_dir);
        std::fs::create_dir_all(&cache_root).unwrap();

        let map = default_legend_class_map();
        let thresholds = ConfidenceThresholds::default();
        let drawings = manifest["correspondence"]["drawings"].as_array().unwrap();
        let mut eval_idx = 0usize;
        let mut high = 0usize;
        let mut medium = 0usize;
        let mut low = 0usize;
        for row in drawings {
            if !row["triple_complete"].as_bool().unwrap_or(false) {
                continue;
            }
            let Some(rel_dxf) = row["dxf_path"].as_str() else {
                continue;
            };
            if row["dxf_bytes"].as_u64().unwrap_or(0) == 0 {
                continue;
            }
            let mut dxf_p = dxf_base.clone();
            for seg in rel_dxf.split('/') {
                dxf_p.push(seg);
            }
            let doc = crate::io::load_file(&dxf_p).expect("load DXF");
            let rec = recognise_sppid_sheet(&doc, map, thresholds);
            high += rec.confidence_summary.high;
            medium += rec.confidence_summary.medium;
            low += rec.confidence_summary.low;
            let json_val = rec.to_legend_json(&format!("{eval_idx:04}.dxf"), true);
            std::fs::write(
                cache_root.join(format!("{eval_idx:04}.json")),
                serde_json::to_string_pretty(&json_val).unwrap(),
            )
            .unwrap();
            eval_idx += 1;
        }
        println!(
            "exported_90_cache={} confidence_high={} confidence_medium={} confidence_low={}",
            eval_idx, high, medium, low
        );
        assert_eq!(eval_idx, 90);
    }
}

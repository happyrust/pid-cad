//! Externalized DWG/DXF block-to-SPPID class & variant mapping (`L3-1` / `#124`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use document::{Document, EntityType};
use serde::{Deserialize, Serialize};

use crate::io::pid_publish::classes::{default_class_map, ClassMapSpec};

const DEFAULT_LEGEND_MAP_JSON: &str = include_str!("../../../assets/pid-sppid-legend-map.json");

/// Optional environment variable pointing to an external override JSON file.
pub const LEGEND_MAP_ENV: &str = "OCS_PID_SPPID_LEGEND_MAP";

/// Explicit override entry in `assets/pid-sppid-legend-map.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockOverrideRule {
    pub class: String,
    pub variant_id: String,
    #[serde(default)]
    pub symbol_path: Option<String>,
}

/// Externalized normalization, filtering, and alias rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegendMapRules {
    pub schema_version: u32,
    #[serde(default)]
    pub ignore_layers: Vec<String>,
    #[serde(default)]
    pub ignore_block_exact: Vec<String>,
    #[serde(default)]
    pub ignore_block_prefixes: Vec<String>,
    #[serde(default)]
    pub ignore_block_suffixes: Vec<String>,
    #[serde(default)]
    pub strip_suffixes: Vec<String>,
    #[serde(default)]
    pub strip_leading_chars: String,
    #[serde(default = "default_true")]
    pub strip_trailing_digits_fallback: bool,
    #[serde(default)]
    pub stem_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub block_overrides: BTreeMap<String, BlockOverrideRule>,
    #[serde(default)]
    pub layer_class_hints: BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// Why a block placement is excluded from business symbol recognition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreReason {
    Layer(String),
    BlockRule(String),
}

/// How a block name matched the SPPID class/variant catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockMatchKind {
    Override,
    ExactStem,
    NormalizedStem,
    AliasStem,
}

/// Resolved SPPID class, variant ID, and `.sym` catalog path for a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappedBlock {
    pub class: String,
    pub variant_id: String,
    pub symbol_path: Option<String>,
    pub normalized_stem: String,
    pub match_kind: BlockMatchKind,
}

/// Outcome of resolving one `(block_name, layer)` placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BlockResolution {
    Ignored {
        reason: IgnoreReason,
    },
    Mapped(MappedBlock),
    Unmapped {
        normalized_stem: String,
        layer_hint: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct StemEntry {
    class: String,
    variant_id: String,
    symbol_path: String,
}

/// Compiled block-to-SPPID class & variant resolver.
#[derive(Debug, Clone)]
pub struct LegendClassMap {
    rules: LegendMapRules,
    ignore_layers_upper: BTreeSet<String>,
    ignore_exact_upper: BTreeSet<String>,
    ignore_prefixes_upper: Vec<String>,
    ignore_suffixes_upper: Vec<String>,
    strip_suffixes_upper: Vec<String>,
    aliases_upper: BTreeMap<String, String>,
    overrides_upper: BTreeMap<String, BlockOverrideRule>,
    stem_index: BTreeMap<String, StemEntry>,
}

/// Aggregate block-mapping coverage across one or more drawings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockAuditSummary {
    pub drawings: usize,
    pub total_inserts: usize,
    pub ignored_inserts: usize,
    pub business_inserts: usize,
    pub mapped_inserts: usize,
    pub unmapped_inserts: usize,
    pub by_sppid_class: BTreeMap<String, usize>,
    pub unmapped_blocks: BTreeMap<String, usize>,
}

impl BlockAuditSummary {
    pub fn coverage_ratio(&self) -> f64 {
        if self.business_inserts == 0 {
            1.0
        } else {
            self.mapped_inserts as f64 / self.business_inserts as f64
        }
    }

    pub fn merge(&mut self, other: &BlockAuditSummary) {
        self.drawings += other.drawings;
        self.total_inserts += other.total_inserts;
        self.ignored_inserts += other.ignored_inserts;
        self.business_inserts += other.business_inserts;
        self.mapped_inserts += other.mapped_inserts;
        self.unmapped_inserts += other.unmapped_inserts;
        for (k, v) in &other.by_sppid_class {
            *self.by_sppid_class.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &other.unmapped_blocks {
            *self.unmapped_blocks.entry(k.clone()).or_default() += v;
        }
    }
}

static DEFAULT_MAP: OnceLock<LegendClassMap> = OnceLock::new();

/// Default compiled `LegendClassMap` (reads `OCS_PID_SPPID_LEGEND_MAP` if set,
/// otherwise embedded `assets/pid-sppid-legend-map.json`).
pub fn default_legend_class_map() -> &'static LegendClassMap {
    DEFAULT_MAP.get_or_init(|| {
        if let Ok(path) = std::env::var(LEGEND_MAP_ENV) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                if let Ok(loaded) = LegendClassMap::from_path(Path::new(trimmed)) {
                    return loaded;
                }
            }
        }
        LegendClassMap::from_json_str(DEFAULT_LEGEND_MAP_JSON, default_class_map())
            .expect("embedded assets/pid-sppid-legend-map.json must parse")
    })
}

fn sym_stem_upper(sym_path: &str) -> String {
    let file_part = sym_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(sym_path)
        .trim();
    let stem = file_part
        .strip_suffix(".sym")
        .or_else(|| file_part.strip_suffix(".SYM"))
        .unwrap_or(file_part);
    stem.trim().to_ascii_uppercase()
}

impl LegendClassMap {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_json_str(&text, default_class_map())
    }

    pub fn from_json_str(json_str: &str, catalog: &ClassMapSpec) -> Result<Self, String> {
        let rules: LegendMapRules =
            serde_json::from_str(json_str).map_err(|e| format!("parse legend map JSON: {e}"))?;
        Ok(Self::from_rules(rules, catalog))
    }

    pub fn from_rules(rules: LegendMapRules, catalog: &ClassMapSpec) -> Self {
        let ignore_layers_upper = rules
            .ignore_layers
            .iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        let ignore_exact_upper = rules
            .ignore_block_exact
            .iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        let ignore_prefixes_upper = rules
            .ignore_block_prefixes
            .iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        let ignore_suffixes_upper = rules
            .ignore_block_suffixes
            .iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        let strip_suffixes_upper = rules
            .strip_suffixes
            .iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        let aliases_upper = rules
            .stem_aliases
            .iter()
            .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_ascii_uppercase()))
            .collect();
        let overrides_upper = rules
            .block_overrides
            .iter()
            .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.clone()))
            .collect();

        let mut sorted_variants: Vec<_> = catalog.variants.iter().collect();
        sorted_variants.sort_by(|a, b| {
            b.instance_count
                .cmp(&a.instance_count)
                .then_with(|| a.variant_id.cmp(&b.variant_id))
        });
        let mut stem_index = BTreeMap::new();
        for var in sorted_variants {
            for sp in &var.symbol_paths {
                let stem = sym_stem_upper(sp);
                if !stem.is_empty() {
                    stem_index.entry(stem).or_insert_with(|| StemEntry {
                        class: var.class.clone(),
                        variant_id: var.variant_id.clone(),
                        symbol_path: sp.clone(),
                    });
                }
            }
        }

        Self {
            rules,
            ignore_layers_upper,
            ignore_exact_upper,
            ignore_prefixes_upper,
            ignore_suffixes_upper,
            strip_suffixes_upper,
            aliases_upper,
            overrides_upper,
            stem_index,
        }
    }

    pub fn rules(&self) -> &LegendMapRules {
        &self.rules
    }

    /// Resolve a placed DXF/DWG block name and its layer to SPPID `(class, variant_id, symbol_path)`.
    pub fn resolve_block(&self, block_name: &str, layer: &str) -> BlockResolution {
        let layer_trimmed = layer.trim();
        let layer_upper = layer_trimmed.to_ascii_uppercase();
        if self.ignore_layers_upper.contains(&layer_upper) {
            return BlockResolution::Ignored {
                reason: IgnoreReason::Layer(layer_trimmed.to_string()),
            };
        }

        let raw_trimmed = block_name.trim();
        let raw_upper = raw_trimmed.to_ascii_uppercase();
        if self.ignore_exact_upper.contains(&raw_upper) {
            return BlockResolution::Ignored {
                reason: IgnoreReason::BlockRule(format!("exact:{raw_upper}")),
            };
        }
        for pfx in &self.ignore_prefixes_upper {
            if !pfx.is_empty() && raw_upper.starts_with(pfx.as_str()) {
                return BlockResolution::Ignored {
                    reason: IgnoreReason::BlockRule(format!("prefix:{pfx}")),
                };
            }
        }
        for sfx in &self.ignore_suffixes_upper {
            if !sfx.is_empty() && raw_upper.ends_with(sfx.as_str()) {
                return BlockResolution::Ignored {
                    reason: IgnoreReason::BlockRule(format!("suffix:{sfx}")),
                };
            }
        }

        if let Some(ov) = self.overrides_upper.get(&raw_upper) {
            return BlockResolution::Mapped(MappedBlock {
                class: ov.class.clone(),
                variant_id: ov.variant_id.clone(),
                symbol_path: ov.symbol_path.clone(),
                normalized_stem: raw_upper,
                match_kind: BlockMatchKind::Override,
            });
        }

        if let Some(hit) = self.stem_index.get(&raw_upper) {
            return BlockResolution::Mapped(MappedBlock {
                class: hit.class.clone(),
                variant_id: hit.variant_id.clone(),
                symbol_path: Some(hit.symbol_path.clone()),
                normalized_stem: raw_upper,
                match_kind: BlockMatchKind::ExactStem,
            });
        }

        if let Some(alias_target) = self.aliases_upper.get(&raw_upper) {
            if let Some(hit) = self.stem_index.get(alias_target) {
                return BlockResolution::Mapped(MappedBlock {
                    class: hit.class.clone(),
                    variant_id: hit.variant_id.clone(),
                    symbol_path: Some(hit.symbol_path.clone()),
                    normalized_stem: alias_target.clone(),
                    match_kind: BlockMatchKind::AliasStem,
                });
            }
        }

        let mut norm = raw_upper.clone();
        for sfx in &self.strip_suffixes_upper {
            if !sfx.is_empty() {
                if let Some(stripped) = norm.strip_suffix(sfx.as_str()) {
                    norm = stripped.trim().to_string();
                }
            }
        }
        if !self.rules.strip_leading_chars.is_empty() {
            let lead_chars: Vec<char> = self.rules.strip_leading_chars.chars().collect();
            norm = norm
                .trim_start_matches(|c: char| lead_chars.contains(&c))
                .trim()
                .to_string();
        }

        if norm != raw_upper {
            if let Some(hit) = self.stem_index.get(&norm) {
                return BlockResolution::Mapped(MappedBlock {
                    class: hit.class.clone(),
                    variant_id: hit.variant_id.clone(),
                    symbol_path: Some(hit.symbol_path.clone()),
                    normalized_stem: norm,
                    match_kind: BlockMatchKind::NormalizedStem,
                });
            }
            if let Some(alias_target) = self.aliases_upper.get(&norm) {
                if let Some(hit) = self.stem_index.get(alias_target) {
                    return BlockResolution::Mapped(MappedBlock {
                        class: hit.class.clone(),
                        variant_id: hit.variant_id.clone(),
                        symbol_path: Some(hit.symbol_path.clone()),
                        normalized_stem: alias_target.clone(),
                        match_kind: BlockMatchKind::AliasStem,
                    });
                }
            }
        }

        if self.rules.strip_trailing_digits_fallback {
            let no_digits = norm
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .trim()
                .to_string();
            if !no_digits.is_empty() && no_digits != norm {
                if let Some(hit) = self.stem_index.get(&no_digits) {
                    return BlockResolution::Mapped(MappedBlock {
                        class: hit.class.clone(),
                        variant_id: hit.variant_id.clone(),
                        symbol_path: Some(hit.symbol_path.clone()),
                        normalized_stem: no_digits,
                        match_kind: BlockMatchKind::NormalizedStem,
                    });
                }
                if let Some(alias_target) = self.aliases_upper.get(&no_digits) {
                    if let Some(hit) = self.stem_index.get(alias_target) {
                        return BlockResolution::Mapped(MappedBlock {
                            class: hit.class.clone(),
                            variant_id: hit.variant_id.clone(),
                            symbol_path: Some(hit.symbol_path.clone()),
                            normalized_stem: alias_target.clone(),
                            match_kind: BlockMatchKind::AliasStem,
                        });
                    }
                }
            }
        }

        let layer_hint = self
            .rules
            .layer_class_hints
            .get(layer_trimmed)
            .or_else(|| self.rules.layer_class_hints.get(&layer_upper))
            .cloned();
        BlockResolution::Unmapped {
            normalized_stem: norm,
            layer_hint,
        }
    }
}

/// Audit all model-space `INSERT` entities in `doc` against `map`.
pub fn audit_document_blocks(doc: &Document, map: &LegendClassMap) -> BlockAuditSummary {
    let mut summary = BlockAuditSummary {
        drawings: 1,
        ..Default::default()
    };
    for entity in doc.model_space_entities() {
        let EntityType::Insert(ins) = entity else {
            continue;
        };
        summary.total_inserts += 1;
        let bname = ins.block_name.as_str();
        let layer = ins.common.layer.as_str();
        match map.resolve_block(bname, layer) {
            BlockResolution::Ignored { .. } => {
                summary.ignored_inserts += 1;
            }
            BlockResolution::Mapped(m) => {
                summary.business_inserts += 1;
                summary.mapped_inserts += 1;
                *summary.by_sppid_class.entry(m.class).or_default() += 1;
            }
            BlockResolution::Unmapped { .. } => {
                summary.business_inserts += 1;
                summary.unmapped_inserts += 1;
                let key = format!("{bname} on {layer}");
                *summary.unmapped_blocks.entry(key).or_default() += 1;
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_normalized_mirrored_and_aliased_sppid_blocks() {
        let map = default_legend_class_map();

        match map.resolve_block("BALL VALVE TYPE 2", "11") {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDPipingComponent");
                assert_eq!(m.match_kind, BlockMatchKind::ExactStem);
                assert!(m
                    .symbol_path
                    .as_deref()
                    .unwrap_or("")
                    .ends_with("Ball Valve Type 2.sym"));
            }
            other => panic!("expected Mapped for BALL VALVE TYPE 2, got {other:?}"),
        }

        match map.resolve_block("_OFF-UNIT SQ", "11") {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDBranchPoint");
                assert_eq!(m.match_kind, BlockMatchKind::NormalizedStem);
            }
            other => panic!("expected Mapped for _OFF-UNIT SQ, got {other:?}"),
        }

        match map.resolve_block("PT-PRESSURE TRANSMITTER1", "12") {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDInstrument");
                assert_eq!(m.match_kind, BlockMatchKind::NormalizedStem);
            }
            other => panic!("expected Mapped for PT-PRESSURE TRANSMITTER1, got {other:?}"),
        }

        match map.resolve_block(
            "PV-PRESSURE REGULATING VALVE WITH FLANGE__MIRROREDTEXT",
            "12",
        ) {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDInlineInstrument");
                assert_eq!(m.match_kind, BlockMatchKind::NormalizedStem);
            }
            other => panic!("expected Mapped for mirrored inline instrument, got {other:?}"),
        }

        match map.resolve_block("DCS_", "12") {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDControlSystemFunction");
                assert_eq!(m.match_kind, BlockMatchKind::AliasStem);
            }
            other => panic!("expected Mapped for DCS_ alias, got {other:?}"),
        }
    }

    #[test]
    fn filters_ignored_layers_and_remark_or_border_blocks_and_honors_overrides() {
        let mut rules = default_legend_class_map().rules().clone();
        rules.block_overrides.insert(
            "CUSTOM SKID BLOCK".to_string(),
            BlockOverrideRule {
                class: "PIDProcessEquipment".to_string(),
                variant_id: "PIDProcessEquipment#01".to_string(),
                symbol_path: None,
            },
        );
        let map = LegendClassMap::from_rules(rules, default_class_map());

        assert!(matches!(
            map.resolve_block("ANYTHING", "DrawingBorder"),
            BlockResolution::Ignored {
                reason: IgnoreReason::Layer(_)
            }
        ));
        assert!(matches!(
            map.resolve_block("REMARKS2", "11"),
            BlockResolution::Ignored {
                reason: IgnoreReason::BlockRule(_)
            }
        ));
        assert!(matches!(
            map.resolve_block("FRAME_A1_DWG", "10"),
            BlockResolution::Ignored {
                reason: IgnoreReason::BlockRule(_)
            }
        ));
        match map.resolve_block("CUSTOM SKID BLOCK", "10") {
            BlockResolution::Mapped(m) => {
                assert_eq!(m.class, "PIDProcessEquipment");
                assert_eq!(m.variant_id, "PIDProcessEquipment#01");
                assert_eq!(m.match_kind, BlockMatchKind::Override);
            }
            other => panic!("expected Override match, got {other:?}"),
        }
        match map.resolve_block("TOTALLY_UNKNOWN_BLOCK_XYZ", "11") {
            BlockResolution::Unmapped { layer_hint, .. } => {
                assert_eq!(layer_hint.as_deref(), Some("PIDPipingComponent"));
            }
            other => panic!("expected Unmapped, got {other:?}"),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "scans all non-empty DXFs in OCS_PID_DXF_DIR and verifies >= 99% block mapping coverage"]
    fn real_90_dxf_batch_block_mapping_audit() {
        let Ok(dxf_dir) = std::env::var("OCS_PID_DXF_DIR") else {
            println!("未设置 DXF 目录（OCS_PID_DXF_DIR），跳过全批图块映射审计。");
            return;
        };
        let root = std::path::PathBuf::from(dxf_dir);
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        fn collect_dxfs(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    collect_dxfs(&p, out);
                } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dxf"))
                    && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
                {
                    out.push(p);
                }
            }
        }
        collect_dxfs(&root, &mut files);
        let map = default_legend_class_map();
        let mut total = BlockAuditSummary::default();
        for f in &files {
            let doc = crate::io::load_file(f).expect("load non-empty DXF");
            let one = audit_document_blocks(&doc, map);
            total.merge(&one);
        }
        if let Ok(out_json) = std::env::var("OCS_PID_CLASS_MAP_AUDIT_OUT") {
            let text = serde_json::to_string_pretty(&total).unwrap();
            std::fs::write(out_json, text).unwrap();
        }
        println!(
            "evaluated_dxfs={} total_inserts={} ignored={} business={} mapped={} unmapped={} coverage={:.6}",
            total.drawings,
            total.total_inserts,
            total.ignored_inserts,
            total.business_inserts,
            total.mapped_inserts,
            total.unmapped_inserts,
            total.coverage_ratio()
        );
        assert!(total.drawings >= 90);
        assert!(
            total.coverage_ratio() >= 0.99,
            "expected >= 99% coverage, got {:.6}, unmapped={:?}",
            total.coverage_ratio(),
            total.unmapped_blocks
        );
    }
}

//! Audit `.sym` symbol references across `.pid` drawings against a symbol
//! library directory (`L1-3` / `#113`), using `pid-parse`'s native reader.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pid_parse::symbol_library::library_relative_path;
use pid_parse::{
    build_normalized_geometry, NormalizedPidGeometry, ParseOptions, PidGeometryConfidence,
    PidGraphicKind, PidParser,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachedBodyCoverage {
    /// Every placement of this symbol across scanned drawings carries a non-empty cached body.
    AllCached,
    /// Some placements carry a cached body and some do not.
    PartiallyCached,
    /// No placement carries a cached body.
    NoneCached,
}

impl CachedBodyCoverage {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::AllCached => "有",
            Self::PartiallyCached => "部分有",
            Self::NoneCached => "无",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolReferenceRow {
    /// Normalized library-relative path (`\`-separated, preserving first-seen casing).
    pub relative_path: String,
    /// Leaf `.sym` filename.
    pub symbol_name: String,
    /// Total placement count across all scanned drawings.
    pub placement_count: usize,
    /// Number of distinct drawings referencing this symbol.
    pub drawing_count: usize,
    /// Number of placements carrying a non-empty cached body inside the `.pid`.
    pub cached_placement_count: usize,
    /// Whether the symbol exists in the scanned `SYMBOLS` library.
    pub in_library: bool,
    /// Summary of cached body availability.
    pub cached_body: CachedBodyCoverage,
    /// Where to source the symbol when missing or uncached.
    pub remediation: String,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolMissingSummary {
    pub drawings_scanned: usize,
    pub drawings_failed: usize,
    pub library_symbol_count: usize,
    pub total_placements: usize,
    pub distinct_referenced_symbols: usize,
    pub in_library_symbols: usize,
    pub missing_from_library_symbols: usize,
    pub missing_placements: usize,
    pub missing_but_all_cached_symbols: usize,
    pub missing_without_cache_symbols: usize,
    pub library_scope_note: Option<String>,
    pub rows: Vec<SymbolReferenceRow>,
}

pub fn normalize_sym_rel_key(raw_path: &str) -> String {
    display_sym_rel_path(raw_path).to_ascii_lowercase()
}

pub fn display_sym_rel_path(raw_path: &str) -> String {
    if let Some(sub) = library_relative_path(raw_path) {
        return sub
            .trim()
            .trim_start_matches(['\\', '/'])
            .replace('/', "\\");
    }
    let trimmed = raw_path.trim().replace('/', "\\");
    if trimmed.contains(':') || trimmed.starts_with("\\\\") {
        let leaf = trimmed.rsplit('\\').next().unwrap_or(trimmed.as_str());
        format!("<非标准库路径>\\{leaf}")
    } else {
        trimmed.trim_start_matches('\\').to_string()
    }
}

pub fn collect_library_keys(symbols_root: &Path) -> Result<BTreeSet<String>, String> {
    if !symbols_root.is_dir() {
        return Err(format!(
            "symbols library directory does not exist: {}",
            symbols_root.display()
        ));
    }
    let mut keys = BTreeSet::new();
    let mut stack = vec![symbols_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("cannot read '{}': {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sym"))
            {
                if let Ok(rel) = path.strip_prefix(symbols_root) {
                    keys.insert(normalize_sym_rel_key(&rel.to_string_lossy()));
                }
            }
        }
    }
    Ok(keys)
}

#[derive(Default)]
struct SymbolAccum {
    display_rel: String,
    placement_count: usize,
    drawings: BTreeSet<usize>,
    cached_count: usize,
}

fn tally_geometry(
    drawing_idx: usize,
    geometry: &NormalizedPidGeometry,
    accum: &mut BTreeMap<String, SymbolAccum>,
    total_placements: &mut usize,
) {
    for entity in &geometry.entities {
        if entity.confidence != PidGeometryConfidence::Decoded {
            continue;
        }
        let PidGraphicKind::SymbolInstance {
            symbol_path: Some(raw_path),
            definition,
            ..
        } = &entity.kind
        else {
            continue;
        };
        let key = normalize_sym_rel_key(raw_path);
        if key.is_empty() {
            continue;
        }
        *total_placements += 1;
        let has_cache = definition
            .and_then(|def| geometry.symbol_definition(def))
            .is_some_and(|body| !body.primitives.is_empty());

        let slot = accum.entry(key).or_insert_with(|| SymbolAccum {
            display_rel: display_sym_rel_path(raw_path),
            ..Default::default()
        });
        slot.placement_count += 1;
        slot.drawings.insert(drawing_idx);
        if has_cache {
            slot.cached_count += 1;
        }
    }
}

fn finalize_summary(
    drawings_scanned: usize,
    drawings_failed: usize,
    accum: BTreeMap<String, SymbolAccum>,
    total_placements: usize,
    library_keys: &BTreeSet<String>,
    library_scope_note: Option<String>,
) -> SymbolMissingSummary {
    let mut rows = Vec::with_capacity(accum.len());
    let mut in_library_symbols = 0usize;
    let mut missing_from_library_symbols = 0usize;
    let mut missing_placements = 0usize;
    let mut missing_but_all_cached_symbols = 0usize;
    let mut missing_without_cache_symbols = 0usize;

    for (key, acc) in accum {
        let in_library = library_keys.contains(&key);
        let cached_body = if acc.cached_count == acc.placement_count {
            CachedBodyCoverage::AllCached
        } else if acc.cached_count == 0 {
            CachedBodyCoverage::NoneCached
        } else {
            CachedBodyCoverage::PartiallyCached
        };

        let remediation = if in_library {
            "已在符号库中".to_string()
        } else if cached_body == CachedBodyCoverage::AllCached {
            "图纸自带内嵌缓存本体，显示不受影响；编辑新放符号时从源厂 SPPID 备份包 RefData~4~681.zip 或 Ref\\Symbols 补齐".to_string()
        } else {
            "无完整内嵌缓存本体，须从源厂 SPPID 备份包 RefData~4~681.zip 或工厂 Ref\\Symbols 目录补齐".to_string()
        };

        if in_library {
            in_library_symbols += 1;
        } else {
            missing_from_library_symbols += 1;
            missing_placements += acc.placement_count;
            if cached_body == CachedBodyCoverage::AllCached {
                missing_but_all_cached_symbols += 1;
            } else {
                missing_without_cache_symbols += 1;
            }
        }

        let symbol_name = acc
            .display_rel
            .rsplit('\\')
            .next()
            .unwrap_or(acc.display_rel.as_str())
            .to_string();

        rows.push(SymbolReferenceRow {
            relative_path: acc.display_rel,
            symbol_name,
            placement_count: acc.placement_count,
            drawing_count: acc.drawings.len(),
            cached_placement_count: acc.cached_count,
            in_library,
            cached_body,
            remediation,
        });
    }

    // Sort missing symbols first (highest placement count first), then in-library symbols.
    rows.sort_by(|a, b| {
        a.in_library
            .cmp(&b.in_library)
            .then_with(|| b.placement_count.cmp(&a.placement_count))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    SymbolMissingSummary {
        drawings_scanned,
        drawings_failed,
        library_symbol_count: library_keys.len(),
        total_placements,
        distinct_referenced_symbols: rows.len(),
        in_library_symbols,
        missing_from_library_symbols,
        missing_placements,
        missing_but_all_cached_symbols,
        missing_without_cache_symbols,
        library_scope_note,
        rows,
    }
}

pub fn audit_geometries_against_library(
    geometries: &[NormalizedPidGeometry],
    library_keys: &BTreeSet<String>,
    library_scope_note: Option<String>,
) -> SymbolMissingSummary {
    let mut accum = BTreeMap::new();
    let mut total_placements = 0usize;
    for (idx, geom) in geometries.iter().enumerate() {
        tally_geometry(idx, geom, &mut accum, &mut total_placements);
    }
    finalize_summary(
        geometries.len(),
        0,
        accum,
        total_placements,
        library_keys,
        library_scope_note,
    )
}

pub fn audit_pid_files_against_library(
    pid_files: &[PathBuf],
    symbols_root: &Path,
    library_scope_note: Option<String>,
) -> Result<SymbolMissingSummary, String> {
    let library_keys = collect_library_keys(symbols_root)?;
    let parser = PidParser::with_options(ParseOptions::geometry());
    let mut accum = BTreeMap::new();
    let mut total_placements = 0usize;
    let mut scanned = 0usize;
    let mut failed = 0usize;

    for (idx, path) in pid_files.iter().enumerate() {
        match parser.parse_file(path) {
            Ok(parsed) => {
                scanned += 1;
                let geom = build_normalized_geometry(&parsed);
                tally_geometry(idx, &geom, &mut accum, &mut total_placements);
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    Ok(finalize_summary(
        scanned,
        failed,
        accum,
        total_placements,
        &library_keys,
        library_scope_note,
    ))
}

fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

pub fn render_csv(summary: &SymbolMissingSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "symbol_name,relative_path,placement_count,drawing_count,in_library,has_cached_body,cached_placements,remediation"
    );
    for row in &summary.rows {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{}",
            csv_escape(&row.symbol_name),
            csv_escape(&row.relative_path),
            row.placement_count,
            row.drawing_count,
            if row.in_library {
                "在库"
            } else {
                "不在库"
            },
            row.cached_body.label_zh(),
            row.cached_placement_count,
            csv_escape(&row.remediation),
        );
    }
    out
}

pub fn render_markdown(summary: &SymbolMissingSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 符号引用与缺失清单（#113）\n");
    if let Some(note) = &summary.library_scope_note {
        let _ = writeln!(out, "> 说明：{note}\n");
    }
    let _ = writeln!(
        out,
        "- 扫描图纸数：`{}`（解析失败：`{}`）",
        summary.drawings_scanned, summary.drawings_failed
    );
    let _ = writeln!(
        out,
        "- 对比符号库文件数：`{}`",
        summary.library_symbol_count
    );
    let _ = writeln!(out, "- 符号放置总次数：`{}`", summary.total_placements);
    let _ = writeln!(
        out,
        "- 引用去重符号种类：`{}`（在库：`{}`，不在库：`{}`）",
        summary.distinct_referenced_symbols,
        summary.in_library_symbols,
        summary.missing_from_library_symbols
    );
    let _ = writeln!(
        out,
        "- 不在库符号中：**有缓存本体 `{}` 种**，**无完整缓存本体 `{}` 种**（涉及放置 `{}` 次）\n",
        summary.missing_but_all_cached_symbols,
        summary.missing_without_cache_symbols,
        summary.missing_placements
    );
    let _ = writeln!(
        out,
        "| 符号名 | 符号相对路径 | 引用次数 | 涉及图纸数 | 在库/不在库 | 有无缓存本体 | 补齐来源 |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---|---|---|");
    for row in &summary.rows {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} | {} | {}（{}/{}） | {} |",
            row.symbol_name,
            row.relative_path,
            row.placement_count,
            row.drawing_count,
            if row.in_library {
                "在库"
            } else {
                "不在库"
            },
            row.cached_body.label_zh(),
            row.cached_placement_count,
            row.placement_count,
            row.remediation,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pid_parse::symbol_library::{PrimitiveStyle, SymbolPrimitive};
    use pid_parse::{
        PidGraphicEntity, PidPoint, PidSymbolDefinition, PidSymbolDefinitionRef,
        PidSymbolSheetLayer,
    };

    fn make_cached_def(site: u32, has_strokes: bool) -> PidSymbolDefinition {
        let reference = PidSymbolDefinitionRef { site, sheet: 1 };
        let primitives = if has_strokes {
            vec![SymbolPrimitive::Line {
                start: (0.0, 0.0),
                end: (1.0, 0.0),
            }]
        } else {
            Vec::new()
        };
        let primitive_layers = if has_strokes { vec![8] } else { Vec::new() };
        let primitive_styles = if has_strokes {
            vec![Some(PrimitiveStyle {
                rgb: [0, 0, 0],
                width_mm: 0.25,
                dash_mm: Vec::new(),
                paper: false,
            })]
        } else {
            Vec::new()
        };
        PidSymbolDefinition {
            reference,
            layers: vec![8],
            sheet_layers: vec![PidSymbolSheetLayer {
                oid: 8,
                name: Some("Default".to_string()),
                displayed: Some(true),
            }],
            primitives,
            primitive_layers,
            primitive_styles,
            primitive_letterings: Vec::new(),
            dimensions: Vec::new(),
            variables: Vec::new(),
            template: None,
        }
    }

    fn sym_entity(path: &str, def: Option<PidSymbolDefinitionRef>) -> PidGraphicEntity {
        PidGraphicEntity {
            id: "1".to_string(),
            drawing_id: None,
            graphic_oid: Some(10),
            source_layer: None,
            kind: PidGraphicKind::SymbolInstance {
                insertion: PidPoint { x: 0.0, y: 0.0 },
                symbol_path: Some(path.to_string()),
                rotation: 0.0,
                scale: [1.0, 1.0],
                definition: def,
            },
            coordinate_context: Default::default(),
            source: pid_parse::PidGraphicProvenance {
                stream_path: None,
                byte_range: None,
                record_id: None,
                record_kind: None,
                field_x: None,
                note: None,
            },
            confidence: PidGeometryConfidence::Decoded,
        }
    }

    fn make_geom(
        entities: Vec<PidGraphicEntity>,
        symbol_definitions: Vec<PidSymbolDefinition>,
    ) -> NormalizedPidGeometry {
        NormalizedPidGeometry {
            entities,
            symbol_definitions,
            ..Default::default()
        }
    }

    #[test]
    fn aggregates_references_and_classifies_cache_and_remediation() {
        let g1 = make_geom(
            vec![
                sym_entity(
                    r"\\HOST\Share\Ref\Symbols\Piping\Valves\Gate Valve.sym",
                    Some(PidSymbolDefinitionRef { site: 1, sheet: 1 }),
                ),
                sym_entity(
                    r"\\HOST\Share\Ref\Symbols\Piping\Specialty\Pulsation Damper.sym",
                    Some(PidSymbolDefinitionRef { site: 2, sheet: 1 }),
                ),
            ],
            vec![make_cached_def(1, true), make_cached_def(2, true)],
        );
        let g2 = make_geom(
            vec![
                sym_entity(
                    r"\\OTHER\Plant\Symbols\piping\valves\gate valve.sym",
                    Some(PidSymbolDefinitionRef { site: 3, sheet: 1 }),
                ),
                sym_entity(r"\\OTHER\Plant\Symbols\Custom\NoCache.sym", None),
            ],
            vec![make_cached_def(3, true)],
        );

        let mut lib = BTreeSet::new();
        lib.insert(normalize_sym_rel_key(r"Piping\Valves\Gate Valve.sym"));

        let summary = audit_geometries_against_library(
            &[g1, g2],
            &lib,
            Some("符号库覆盖不全：测试子集库".to_string()),
        );

        assert_eq!(summary.drawings_scanned, 2);
        assert_eq!(summary.total_placements, 4);
        assert_eq!(summary.distinct_referenced_symbols, 3);
        assert_eq!(summary.in_library_symbols, 1);
        assert_eq!(summary.missing_from_library_symbols, 2);
        assert_eq!(summary.missing_but_all_cached_symbols, 1);
        assert_eq!(summary.missing_without_cache_symbols, 1);

        let gate = summary
            .rows
            .iter()
            .find(|r| r.symbol_name.eq_ignore_ascii_case("Gate Valve.sym"))
            .unwrap();
        assert!(gate.in_library);
        assert_eq!(gate.placement_count, 2);
        assert_eq!(gate.drawing_count, 2);
        assert_eq!(gate.cached_body, CachedBodyCoverage::AllCached);

        let damper = summary
            .rows
            .iter()
            .find(|r| r.symbol_name.eq_ignore_ascii_case("Pulsation Damper.sym"))
            .unwrap();
        assert!(!damper.in_library);
        assert_eq!(damper.cached_body, CachedBodyCoverage::AllCached);
        assert_eq!(damper.cached_body.label_zh(), "有");

        let nocache = summary
            .rows
            .iter()
            .find(|r| r.symbol_name.eq_ignore_ascii_case("NoCache.sym"))
            .unwrap();
        assert!(!nocache.in_library);
        assert_eq!(nocache.cached_body, CachedBodyCoverage::NoneCached);
        assert_eq!(nocache.cached_body.label_zh(), "无");
        assert!(nocache.remediation.contains("补齐"));

        let md = render_markdown(&summary);
        assert!(md.contains("符号库覆盖不全：测试子集库"));
        assert!(md.contains("Pulsation Damper.sym"));
        let csv = render_csv(&summary);
        assert!(csv.contains("NoCache.sym"));
    }

    #[test]
    #[ignore = "scans external .pid corpora and writes missing symbol tables to OCS_PID_EVIDENCE_DIR"]
    fn real_corpus_symbol_missing_inventory() {
        let Ok(pid_dir) = std::env::var("OCS_PID_SYM_AUDIT_PIDS") else {
            println!("未设置语料目录（OCS_PID_SYM_AUDIT_PIDS），跳过符号缺失清单扫描。");
            return;
        };
        let Ok(sym_dir) = std::env::var("OCS_PID_SYM_AUDIT_LIB") else {
            println!("未设置符号库目录（OCS_PID_SYM_AUDIT_LIB），跳过符号缺失清单扫描。");
            return;
        };
        let note = std::env::var("OCS_PID_SYM_AUDIT_NOTE")
            .ok()
            .filter(|s| !s.is_empty());
        let out_prefix = std::env::var("OCS_PID_SYM_AUDIT_OUT").unwrap_or_default();

        let mut pid_files = Vec::new();
        let mut stack = vec![PathBuf::from(&pid_dir)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pid")) {
                    pid_files.push(p);
                }
            }
        }
        pid_files.sort();
        assert!(!pid_files.is_empty(), "no .pid files found in {pid_dir}");

        let summary =
            audit_pid_files_against_library(&pid_files, Path::new(&sym_dir), note).unwrap();
        println!(
            "drawings={} failed={} lib_syms={} placements={} distinct_ref={} in_lib={} missing_syms={} missing_placements={} missing_all_cached={} missing_no_cache={}",
            summary.drawings_scanned,
            summary.drawings_failed,
            summary.library_symbol_count,
            summary.total_placements,
            summary.distinct_referenced_symbols,
            summary.in_library_symbols,
            summary.missing_from_library_symbols,
            summary.missing_placements,
            summary.missing_but_all_cached_symbols,
            summary.missing_without_cache_symbols,
        );
        if !out_prefix.is_empty() {
            let csv_path = PathBuf::from(format!("{out_prefix}.csv"));
            let md_path = PathBuf::from(format!("{out_prefix}.md"));
            if let Some(parent) = csv_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&csv_path, render_csv(&summary)).unwrap();
            std::fs::write(&md_path, render_markdown(&summary)).unwrap();
        }
        assert!(summary.distinct_referenced_symbols > 0);
    }
}

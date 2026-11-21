//! Full decoding census of SmartPlant `.sym` symbol libraries (`L1-6` / `#114`),
//! built on `pid_parse::symbol_library::read_symbol_geometry_from_bytes`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pid_parse::symbol_library::read_symbol_geometry_from_bytes;

pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymDecodeStatus {
    /// Decoded cleanly with >= 1 displayed primitive.
    SuccessDrawable,
    /// Valid `.sym` CFB with 0 displayed primitives (e.g. blank / hidden-layer-only template).
    SuccessEmptyBody,
    /// Decoded partially (short chain stream or `SymbolNotDrawn`).
    PartialNotDrawn,
    /// Failed to parse as `.sym` CFB.
    Failed,
}

impl SymDecodeStatus {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::SuccessDrawable => "成功（可画）",
            Self::SuccessEmptyBody => "成功（空本体）",
            Self::PartialNotDrawn => "部分成功（不完整未画）",
            Self::Failed => "失败",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymFailureCategory {
    None,
    VersionDiff,
    Encrypted,
    NonStandardFile,
    ShortStream,
    Other,
}

impl SymFailureCategory {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::None => "—",
            Self::VersionDiff => "版本差异",
            Self::Encrypted => "加密或受保护",
            Self::NonStandardFile => "非标准文件（非OLE复合文档或截断）",
            Self::ShortStream => "记录链截断",
            Self::Other => "其他解析错误",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymCensusRow {
    pub source_label: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub status: SymDecodeStatus,
    pub category: SymFailureCategory,
    pub displayed_primitives: usize,
    pub total_primitives: usize,
    pub reason: String,
    pub cache_fallback_note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymCensusSummary {
    pub source_label: String,
    pub total_files: usize,
    pub success_drawable: usize,
    pub success_empty_body: usize,
    pub partial_not_drawn: usize,
    pub failed: usize,
    pub rows: Vec<SymCensusRow>,
}

fn classify_error_text(msg: &str) -> SymFailureCategory {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("encrypt") || lower.contains("password") || lower.contains("protect") {
        SymFailureCategory::Encrypted
    } else if lower.contains("version") {
        SymFailureCategory::VersionDiff
    } else if lower.contains("cfb") || lower.contains("compound") || lower.contains("magic") {
        SymFailureCategory::NonStandardFile
    } else {
        SymFailureCategory::Other
    }
}

pub fn inspect_sym_bytes(
    source_label: &str,
    relative_path: &str,
    bytes: &[u8],
    cached_keys: Option<&BTreeSet<String>>,
) -> SymCensusRow {
    let norm_key = relative_path
        .trim()
        .trim_start_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase();
    let cache_fallback_note = match cached_keys {
        Some(set) if set.contains(&norm_key) => "有缓存本体".to_string(),
        Some(_) => "无（样本图未引用或无缓存）".to_string(),
        None => "图纸引用时优先读图纸内嵌缓存本体".to_string(),
    };

    if bytes.len() < CFB_MAGIC.len() || bytes[..CFB_MAGIC.len()] != CFB_MAGIC {
        return SymCensusRow {
            source_label: source_label.to_string(),
            relative_path: relative_path.to_string(),
            size_bytes: bytes.len() as u64,
            status: SymDecodeStatus::Failed,
            category: SymFailureCategory::NonStandardFile,
            displayed_primitives: 0,
            total_primitives: 0,
            reason: format!("header is not OLE CFB ({} bytes)", bytes.len()),
            cache_fallback_note,
        };
    }

    match read_symbol_geometry_from_bytes(bytes) {
        Err(err) => {
            let msg = err.to_string();
            SymCensusRow {
                source_label: source_label.to_string(),
                relative_path: relative_path.to_string(),
                size_bytes: bytes.len() as u64,
                status: SymDecodeStatus::Failed,
                category: classify_error_text(&msg),
                displayed_primitives: 0,
                total_primitives: 0,
                reason: msg,
                cache_fallback_note,
            }
        }
        Ok(geom) => {
            let total_primitives = geom.primitives.len();
            match geom.displayed() {
                Err(not_drawn) => SymCensusRow {
                    source_label: source_label.to_string(),
                    relative_path: relative_path.to_string(),
                    size_bytes: bytes.len() as u64,
                    status: SymDecodeStatus::PartialNotDrawn,
                    category: SymFailureCategory::ShortStream,
                    displayed_primitives: 0,
                    total_primitives,
                    reason: not_drawn.describe().to_string(),
                    cache_fallback_note,
                },
                Ok(displayed) => {
                    let disp_count = displayed.len();
                    if !geom.short.is_empty() {
                        SymCensusRow {
                            source_label: source_label.to_string(),
                            relative_path: relative_path.to_string(),
                            size_bytes: bytes.len() as u64,
                            status: SymDecodeStatus::PartialNotDrawn,
                            category: SymFailureCategory::ShortStream,
                            displayed_primitives: disp_count,
                            total_primitives,
                            reason: "partial short stream read".to_string(),
                            cache_fallback_note,
                        }
                    } else if disp_count == 0 {
                        SymCensusRow {
                            source_label: source_label.to_string(),
                            relative_path: relative_path.to_string(),
                            size_bytes: bytes.len() as u64,
                            status: SymDecodeStatus::SuccessEmptyBody,
                            category: SymFailureCategory::None,
                            displayed_primitives: 0,
                            total_primitives,
                            reason: if total_primitives == 0 {
                                "空本体（0 图元）".to_string()
                            } else {
                                format!("仅含关闭图层图元（总图元 {total_primitives}）")
                            },
                            cache_fallback_note,
                        }
                    } else {
                        SymCensusRow {
                            source_label: source_label.to_string(),
                            relative_path: relative_path.to_string(),
                            size_bytes: bytes.len() as u64,
                            status: SymDecodeStatus::SuccessDrawable,
                            category: SymFailureCategory::None,
                            displayed_primitives: disp_count,
                            total_primitives,
                            reason: String::new(),
                            cache_fallback_note,
                        }
                    }
                }
            }
        }
    }
}

pub fn census_symbol_directory(
    source_label: &str,
    root: &Path,
    cached_keys: Option<&BTreeSet<String>>,
) -> Result<SymCensusSummary, String> {
    if !root.is_dir() {
        return Err(format!(
            "symbol directory does not exist: {}",
            root.display()
        ));
    }
    let mut sym_files: Vec<(String, PathBuf)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("cannot read '{}': {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("sym"))
            {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('/', "\\");
                sym_files.push((rel, p));
            }
        }
    }
    sym_files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut summary = SymCensusSummary {
        source_label: source_label.to_string(),
        total_files: sym_files.len(),
        ..Default::default()
    };

    for (rel, path) in sym_files {
        let bytes =
            std::fs::read(&path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        let row = inspect_sym_bytes(source_label, &rel, &bytes, cached_keys);
        match row.status {
            SymDecodeStatus::SuccessDrawable => summary.success_drawable += 1,
            SymDecodeStatus::SuccessEmptyBody => summary.success_empty_body += 1,
            SymDecodeStatus::PartialNotDrawn => summary.partial_not_drawn += 1,
            SymDecodeStatus::Failed => summary.failed += 1,
        }
        summary.rows.push(row);
    }

    Ok(summary)
}

fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

pub fn render_census_csv(summary: &SymCensusSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "source,relative_path,size_bytes,status,category,displayed_primitives,total_primitives,cache_fallback,reason"
    );
    for r in &summary.rows {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{}",
            csv_escape(&r.source_label),
            csv_escape(&r.relative_path),
            r.size_bytes,
            csv_escape(r.status.label_zh()),
            csv_escape(r.category.label_zh()),
            r.displayed_primitives,
            r.total_primitives,
            csv_escape(&r.cache_fallback_note),
            csv_escape(&r.reason),
        );
    }
    out
}

pub fn render_census_markdown(summary: &SymCensusSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# `.sym` 符号库解码状态普查（来源：`{}`，#114）\n",
        summary.source_label
    );
    let _ = writeln!(out, "- 总文件数：`{}`", summary.total_files);
    let _ = writeln!(out, "- 成功（可画）：`{}`", summary.success_drawable);
    let _ = writeln!(out, "- 成功（空本体）：`{}`", summary.success_empty_body);
    let _ = writeln!(out, "- 部分成功（未画）：`{}`", summary.partial_not_drawn);
    let _ = writeln!(out, "- 失败：`{}`\n", summary.failed);
    let _ = writeln!(
        out,
        "| 来源 | 相对路径 | 大小(B) | 解码结果 | 失败/异常分类 | 可见/总图元 | 缓存本体替代 | 原因说明 |"
    );
    let _ = writeln!(out, "|---|---|---:|---|---|---:|---|---|");
    for r in &summary.rows {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} | {} | {}/{} | {} | {} |",
            r.source_label,
            r.relative_path,
            r.size_bytes,
            r.status.label_zh(),
            r.category.label_zh(),
            r.displayed_primitives,
            r.total_primitives,
            r.cache_fallback_note,
            if r.reason.is_empty() {
                "—"
            } else {
                &r.reason
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_non_standard_files_and_runs_deterministically() {
        let dir = std::env::temp_dir().join(format!("ocs-sym-census-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Piping")).unwrap();
        std::fs::write(
            dir.join("Piping").join("not_cfb.sym"),
            b"PLAIN-ASCII-NOT-CFB",
        )
        .unwrap();
        std::fs::write(dir.join("Piping").join("empty.sym"), b"").unwrap();

        let mut cached = BTreeSet::new();
        cached.insert("piping\\not_cfb.sym".to_string());

        let run1 = census_symbol_directory("synth", &dir, Some(&cached)).unwrap();
        let run2 = census_symbol_directory("synth", &dir, Some(&cached)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(run1, run2, "census must be 100% deterministic");
        assert_eq!(run1.total_files, 2);
        assert_eq!(run1.failed, 2);
        assert!(run1
            .rows
            .iter()
            .all(|r| r.category == SymFailureCategory::NonStandardFile));
        let hit = run1
            .rows
            .iter()
            .find(|r| r.relative_path == "Piping\\not_cfb.sym")
            .unwrap();
        assert_eq!(hit.cache_fallback_note, "有缓存本体");

        let csv = render_census_csv(&run1);
        assert!(csv.contains("Piping\\not_cfb.sym"));
    }

    #[test]
    #[ignore = "runs full .sym decoding census on external symbol directories"]
    fn real_corpus_sym_census() {
        let Ok(raw_sources) = std::env::var("OCS_PID_SYM_CENSUS_SOURCES") else {
            println!("未设置语料目录（OCS_PID_SYM_CENSUS_SOURCES），跳过 .sym 普查。");
            return;
        };
        let out_dir = std::env::var("OCS_PID_SYM_CENSUS_OUT").unwrap_or_default();
        for spec in raw_sources
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let Some((label, path_str)) = spec.split_once('=') else {
                panic!("expected LABEL=PATH in OCS_PID_SYM_CENSUS_SOURCES, got {spec}");
            };
            let root = Path::new(path_str);
            let s1 = census_symbol_directory(label, root, None).unwrap();
            let s2 = census_symbol_directory(label, root, None).unwrap();
            assert_eq!(s1, s2, "two consecutive runs on {label} must match");
            println!(
                "source={label} total={} drawable={} empty={} partial={} failed={}",
                s1.total_files,
                s1.success_drawable,
                s1.success_empty_body,
                s1.partial_not_drawn,
                s1.failed
            );
            if !out_dir.is_empty() {
                let out_p = PathBuf::from(&out_dir);
                std::fs::create_dir_all(&out_p).unwrap();
                std::fs::write(
                    out_p.join(format!("{label}-census.csv")),
                    render_census_csv(&s1),
                )
                .unwrap();
                std::fs::write(
                    out_p.join(format!("{label}-census.md")),
                    render_census_markdown(&s1),
                )
                .unwrap();
            }
        }
    }
}

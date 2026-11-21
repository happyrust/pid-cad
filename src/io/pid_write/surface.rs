//! CFB stream write-surface inventory and disposition classification (`L5-1` / `#128`).

use std::collections::BTreeMap;

use pid_parse::inspect::coverage::coverage_report;
use pid_parse::{CoverageNodeKind, PidPackage};
use serde::{Deserialize, Serialize};

/// How the `.pid` generator treats each top-level CFB stream or storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteDisposition {
    /// Record-chain stream (`0x6C90F544`) where new geometry, text, symbol,
    /// or attribute records are appended and `JSheetLayer` counts updated.
    AppendRecords,
    /// Persistent-ID / incoming-reference table (`/PSMspacemap/*`) kept in
    /// sync when new objects or attribute rows are added.
    SyncIndex,
    /// Embedded symbol cache storage (`/JSite*/*`) and `/JSitesList` updated
    /// when placing symbols not yet cached in the seed.
    CloneSymbolSite,
    /// OLE property-set stream (`SummaryInformation` / `DocumentSummaryInformation`)
    /// updated with `_SPID` (`DocUID`), `_PlantName`, and `_PIDTemplate`.
    RewriteSummary,
    /// Known SPPID structural/style/version stream preserved byte-for-byte from seed.
    PassthroughPreserve,
    /// Unrecognized stream preserved byte-for-byte via `PidPackage` passthrough.
    UnknownPreserve,
}

impl WriteDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppendRecords => "append_records",
            Self::SyncIndex => "sync_index",
            Self::CloneSymbolSite => "clone_symbol_site",
            Self::RewriteSummary => "rewrite_summary",
            Self::PassthroughPreserve => "passthrough_preserve",
            Self::UnknownPreserve => "unknown_preserve",
        }
    }
}

/// Write-surface classification of one top-level stream or storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteSurfaceEntry {
    pub top_level_name: String,
    pub is_storage: bool,
    pub stream_size_bytes: u64,
    pub coverage_status: String,
    pub disposition: WriteDisposition,
    pub rationale: String,
}

/// Complete CFB write-surface inventory for a `.pid` package.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteSurfaceReport {
    pub total_raw_streams: usize,
    pub total_bytes: u64,
    pub entries: Vec<WriteSurfaceEntry>,
    pub by_disposition: BTreeMap<String, usize>,
}

fn classify_disposition(top_name: &str) -> (WriteDisposition, &'static str) {
    if top_name.starts_with("Sheet") {
        (
            WriteDisposition::AppendRecords,
            "0x6C90F544 geometry/text/symbol record chain + 0x0081 JSheetLayer object counts (Q2/Q9)",
        )
    } else if top_name == "PSMcluster0" || top_name == "Unclustered Dynamic Attributes" {
        (
            WriteDisposition::AppendRecords,
            "0x6C90F544 object & 0x0089 FreeFormAttrSet / 0x00FA DependencyObject chain (Q3/Q9)",
        )
    } else if top_name == "PSMspacemap" {
        (
            WriteDisposition::SyncIndex,
            "'tseg' persistent-ID allocator (m_iNext) and incoming reference slots 190/181/182 (Q9)",
        )
    } else if top_name.starts_with("JSite") {
        (
            WriteDisposition::CloneSymbolSite,
            "Embedded symbol definition cache storage / 'OLEM' JSitesList registry (Q9/Q12)",
        )
    } else if top_name == "\x05SummaryInformation" || top_name == "\x05DocumentSummaryInformation" {
        (
            WriteDisposition::RewriteSummary,
            "OLE property set carrying standard summary + Section 2 _SPID/_PlantName/_PIDTemplate (Q10)",
        )
    } else if matches!(
        top_name,
        "StyleCluster"
            | "PSMroots"
            | "PSMclustertable"
            | "PSMsegmenttable"
            | "DocVersion2"
            | "DocVersion3"
            | "AppObject"
            | "JTaggedTxtStgList"
            | "Dynamic Attributes Metadata"
            | "TaggedTxtData"
    ) {
        (
            WriteDisposition::PassthroughPreserve,
            "Seed container/style/root/version stream preserved verbatim unless segment table grows (Q2/Q11)",
        )
    } else {
        (
            WriteDisposition::UnknownPreserve,
            "Unrecognized stream preserved byte-for-byte by PidWriter passthrough (Q1)",
        )
    }
}

/// Classify every top-level stream and storage in `pkg` for `.pid` generation (`L5-1`).
pub fn inspect_write_surface(pkg: &PidPackage) -> WriteSurfaceReport {
    let cov = coverage_report(&pkg.parsed);
    let mut cov_by_name: BTreeMap<String, (bool, String)> = BTreeMap::new();
    for ce in cov.entries {
        let is_stg = matches!(ce.kind, CoverageNodeKind::TopLevelStorage);
        cov_by_name.insert(ce.name, (is_stg, format!("{:?}", ce.status)));
    }

    let mut sizes_by_top: BTreeMap<String, (bool, u64)> = BTreeMap::new();
    let mut total_bytes = 0u64;
    for raw in pkg.streams.values() {
        let sz = raw.data.len() as u64;
        total_bytes = total_bytes.saturating_add(sz);
        let trimmed = raw.path.trim_start_matches('/');
        let (head, is_stg) = match trimmed.find('/') {
            Some(idx) => (&trimmed[..idx], true),
            None => (trimmed, false),
        };
        if !head.is_empty() {
            let entry = sizes_by_top
                .entry(head.to_string())
                .or_insert((is_stg, 0u64));
            entry.0 |= is_stg;
            entry.1 = entry.1.saturating_add(sz);
        }
    }

    let mut entries = Vec::with_capacity(sizes_by_top.len());
    let mut by_disposition: BTreeMap<String, usize> = BTreeMap::new();

    for (top_name, (raw_is_stg, stream_size_bytes)) in sizes_by_top {
        let (cov_is_stg, coverage_status) = cov_by_name
            .remove(&top_name)
            .unwrap_or((raw_is_stg, "RawPassthrough".to_string()));
        let (disposition, rationale) = classify_disposition(&top_name);
        *by_disposition
            .entry(disposition.as_str().to_string())
            .or_default() += 1;
        entries.push(WriteSurfaceEntry {
            top_level_name: top_name,
            is_storage: raw_is_stg || cov_is_stg,
            stream_size_bytes,
            coverage_status,
            disposition,
            rationale: rationale.to_string(),
        });
    }

    WriteSurfaceReport {
        total_raw_streams: pkg.streams.len(),
        total_bytes,
        entries,
        by_disposition,
    }
}

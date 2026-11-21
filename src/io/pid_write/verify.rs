//! Stream-level and CLSID-level round-trip verification (`a`-level gate, `L5-3` / `#128`).

use pid_parse::{diff_packages, PidPackage, PidWriter, WritePlan};
use serde::{Deserialize, Serialize};

/// Result of round-tripping a `PidPackage` through `PidWriter` and re-parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedRoundtripReport {
    pub stream_count: usize,
    pub only_in_source: Vec<String>,
    pub only_in_written: Vec<String>,
    pub modified_streams: Vec<String>,
    pub root_clsid_match: bool,
    pub storage_clsid_diffs: usize,
    pub state_bits_diffs: usize,
    pub reparsed_sheets: usize,
}

impl SeedRoundtripReport {
    /// True when every stream's raw bytes, root CLSID, storage CLSIDs, and
    /// state bits match 100% between source and round-tripped package.
    pub fn is_lossless_passthrough(&self) -> bool {
        self.only_in_source.is_empty()
            && self.only_in_written.is_empty()
            && self.modified_streams.is_empty()
            && self.root_clsid_match
            && self.storage_clsid_diffs == 0
            && self.state_bits_diffs == 0
    }
}

/// Write `seed` through `PidWriter::write_to_bytes(&seed, &WritePlan::default())`,
/// re-parse with `PidPackage::from_bytes`, and verify lossless CFB stream/CLSID equality.
pub fn verify_passthrough_roundtrip(seed: &PidPackage) -> Result<SeedRoundtripReport, String> {
    let bytes = PidWriter::write_to_bytes(seed, &WritePlan::default())
        .map_err(|e| format!("PidWriter::write_to_bytes failed: {e}"))?;
    let reparsed =
        PidPackage::from_bytes(&bytes).map_err(|e| format!("reparse written CFB failed: {e}"))?;
    let diff = diff_packages(seed, &reparsed);

    if seed.parsed.sheet_streams.len() != reparsed.parsed.sheet_streams.len() {
        return Err(format!(
            "sheet count mismatch across passthrough round-trip: {} vs {}",
            seed.parsed.sheet_streams.len(),
            reparsed.parsed.sheet_streams.len()
        ));
    }

    Ok(SeedRoundtripReport {
        stream_count: reparsed.streams.len(),
        only_in_source: diff.only_in_a,
        only_in_written: diff.only_in_b,
        modified_streams: diff.modified.into_iter().map(|m| m.path).collect(),
        root_clsid_match: diff.root_clsid_match,
        storage_clsid_diffs: diff.storage_clsid_diffs.len(),
        state_bits_diffs: diff.state_bits_diffs.len(),
        reparsed_sheets: reparsed.parsed.sheet_streams.len(),
    })
}

/// Repackage a `.pid` CFB container through `PidWriter` and verify 100% lossless
/// stream & storage-CLSID preservation (`V2-01` / `LX-4` / `#144`).
pub fn repack_pid_bytes_lossless(raw_pid: &[u8]) -> Result<(Vec<u8>, SeedRoundtripReport), String> {
    let pkg = PidPackage::from_bytes(raw_pid)
        .map_err(|e| format!("parse source .pid bytes failed: {e}"))?;
    let repacked = PidWriter::write_to_bytes(&pkg, &WritePlan::default())
        .map_err(|e| format!("PidWriter::write_to_bytes failed: {e}"))?;
    let rt = verify_passthrough_roundtrip(&pkg)?;
    if !rt.is_lossless_passthrough() {
        return Err(format!("CFB repack was not stream-lossless: {rt:?}"));
    }
    Ok((repacked, rt))
}

/// Modify only `_PlantName` in `DocumentSummaryInformation` Section 2 while preserving
/// every drawing/record/index stream untouched (`LX-4 E2` / `#144`).
pub fn stamp_pid_summary_probe(raw_pid: &[u8], plant_override: &str) -> Result<Vec<u8>, String> {
    let mut pkg = PidPackage::from_bytes(raw_pid)
        .map_err(|e| format!("parse source .pid bytes failed: {e}"))?;
    let mut updates = std::collections::BTreeMap::new();
    updates.insert("_PlantName".to_string(), plant_override.trim().to_string());
    super::seed::set_doc_summary_user_properties(&mut pkg, &updates)?;
    PidWriter::write_to_bytes(&pkg, &WritePlan::default())
        .map_err(|e| format!("write stamped E2 .pid bytes failed: {e}"))
}

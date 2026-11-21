//! Seed `.pid` loading (file, memory, or `RefData~*.zip` template archive)
//! and `DocumentSummaryInformation` Section 2 user-property stamping (`L5-3` / `#128`).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use pid_parse::{PidPackage, SummaryPropertyValue};

const DOC_SUMMARY_STREAM_PATH: &str = "/\x05DocumentSummaryInformation";

/// `{D5CDD502-2E9C-101B-9397-08002B2CF9AE}` (Section 1 standard DocumentSummary).
const FMTID_DOC_SUMMARY_SEC1: [u8; 16] = [
    0x02, 0xD5, 0xCD, 0xD5, 0x9C, 0x2E, 0x1B, 0x10, 0x93, 0x97, 0x08, 0x00, 0x2B, 0x2C, 0xF9, 0xAE,
];

/// `{D5CDD505-2E9C-101B-9397-08002B2CF9AE}` (Section 2 user-defined dictionary).
const FMTID_DOC_SUMMARY_SEC2: [u8; 16] = [
    0x05, 0xD5, 0xCD, 0xD5, 0x9C, 0x2E, 0x1B, 0x10, 0x93, 0x97, 0x08, 0x00, 0x2B, 0x2C, 0xF9, 0xAE,
];

fn u16_le(data: &[u8], at: usize) -> Option<u16> {
    let b = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_le(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

/// Load a seed `.pid` package from disk.
pub fn load_seed_from_path(path: &Path) -> Result<PidPackage, String> {
    PidPackage::from_path(path).map_err(|e| format!("parse seed {}: {e}", path.display()))
}

/// Load a seed `.pid` package from in-memory bytes.
pub fn load_seed_from_bytes(bytes: &[u8]) -> Result<PidPackage, String> {
    PidPackage::from_bytes(bytes).map_err(|e| format!("parse seed bytes: {e}"))
}

/// List `(entry_name, uncompressed_bytes)` for all `.pid` seed templates inside a
/// reference-data archive (e.g. `RefData~4~682.zip`).
pub fn list_seeds_in_refdata_zip(zip_path: &Path) -> Result<Vec<(String, u64)>, String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("open {}: {e}", zip_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("read zip {}: {e}", zip_path.display()))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry #{i}: {e}"))?;
        let name = entry.name().to_string();
        if Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pid"))
        {
            out.push((name, entry.size()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Load one named `.pid` template (e.g. `"A1-W-New.pid"`) from a `RefData~*.zip` archive.
pub fn load_seed_from_refdata_zip(zip_path: &Path, entry_name: &str) -> Result<PidPackage, String> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| format!("open {}: {e}", zip_path.display()))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("read zip {}: {e}", zip_path.display()))?;
    let mut target_idx = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip entry #{i}: {e}"))?;
        if entry.name().eq_ignore_ascii_case(entry_name) {
            target_idx = Some(i);
            break;
        }
    }
    let idx = target_idx.ok_or_else(|| {
        format!(
            "seed template '{entry_name}' not found in {}",
            zip_path.display()
        )
    })?;
    let mut entry = archive
        .by_index(idx)
        .map_err(|e| format!("open zip entry '{entry_name}': {e}"))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut buf)
        .map_err(|e| format!("read zip entry '{entry_name}': {e}"))?;
    load_seed_from_bytes(&buf)
}

fn minimal_section_1_bytes() -> Vec<u8> {
    // 16B header+table (`size=24`, `num_props=1`, `PROPID 1 @ 16`) + 8B `VT_I2 CodePage=1200`
    let mut sec1 = Vec::with_capacity(24);
    sec1.extend_from_slice(&24u32.to_le_bytes());
    sec1.extend_from_slice(&1u32.to_le_bytes());
    sec1.extend_from_slice(&1u32.to_le_bytes());
    sec1.extend_from_slice(&16u32.to_le_bytes());
    sec1.extend_from_slice(&0x0002u32.to_le_bytes());
    sec1.extend_from_slice(&1200u32.to_le_bytes());
    sec1
}

fn extract_existing_header_and_sec1(raw: &[u8]) -> Option<([u8; 24], Vec<u8>)> {
    if raw.len() < 48 || u16_le(raw, 0)? != 0xFFFE {
        return None;
    }
    let num_sec = u32_le(raw, 24)? as usize;
    if num_sec == 0 {
        return None;
    }
    let sec1_off = u32_le(raw, 44)? as usize;
    let sec1_size = u32_le(raw, sec1_off)? as usize;
    let sec1_end = sec1_off.checked_add(sec1_size)?;
    if sec1_size < 8 || sec1_end > raw.len() {
        return None;
    }
    let mut prefix24 = [0u8; 24];
    prefix24.copy_from_slice(&raw[..24]);
    let mut sec1 = raw[sec1_off..sec1_end].to_vec();
    pad4(&mut sec1);
    let padded_len = sec1.len() as u32;
    sec1[0..4].copy_from_slice(&padded_len.to_le_bytes());
    Some((prefix24, sec1))
}

fn encode_user_prop_value(val: &SummaryPropertyValue) -> Vec<u8> {
    let mut buf = Vec::new();
    match val {
        SummaryPropertyValue::Lpstr(s) if s.is_ascii() => {
            buf.extend_from_slice(&0x001Eu32.to_le_bytes());
            let len_incl_nul = (s.len() + 1) as u32;
            buf.extend_from_slice(&len_incl_nul.to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
            pad4(&mut buf);
        }
        SummaryPropertyValue::Lpstr(s) | SummaryPropertyValue::Lpwstr(s) => {
            buf.extend_from_slice(&0x001Fu32.to_le_bytes());
            let mut units: Vec<u16> = s.encode_utf16().collect();
            units.push(0);
            buf.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {
                buf.extend_from_slice(&u.to_le_bytes());
            }
            pad4(&mut buf);
        }
        SummaryPropertyValue::I4(v) => {
            buf.extend_from_slice(&0x0003u32.to_le_bytes());
            buf.extend_from_slice(&v.to_le_bytes());
        }
        SummaryPropertyValue::Bool(b) => {
            buf.extend_from_slice(&0x000Bu32.to_le_bytes());
            let word: u32 = if *b { 0xFFFF } else { 0 };
            buf.extend_from_slice(&word.to_le_bytes());
        }
        SummaryPropertyValue::Filetime(ft) => {
            buf.extend_from_slice(&0x0040u32.to_le_bytes());
            buf.extend_from_slice(&(*ft as u32).to_le_bytes());
            buf.extend_from_slice(&((*ft >> 32) as u32).to_le_bytes());
        }
        SummaryPropertyValue::Raw { vt, bytes } => {
            buf.extend_from_slice(&u32::from(*vt).to_le_bytes());
            buf.extend_from_slice(bytes);
            pad4(&mut buf);
        }
    }
    buf
}

fn encode_section_2(props: &BTreeMap<String, SummaryPropertyValue>) -> Result<Vec<u8>, String> {
    for k in props.keys() {
        if k.is_empty() || !k.is_ascii() || k.contains('\0') {
            return Err(format!(
                "DocumentSummary Section 2 key must be non-empty ASCII without NUL, got {k:?}"
            ));
        }
    }

    // PROPID 0: Dictionary (tight LPSTR entries per [MS-OLEPS] §2.16)
    let mut dict_body = Vec::new();
    dict_body.extend_from_slice(&(props.len() as u32).to_le_bytes());
    for (idx, key) in props.keys().enumerate() {
        let prop_id = (idx as u32) + 2;
        dict_body.extend_from_slice(&prop_id.to_le_bytes());
        let name_len = (key.len() + 1) as u32;
        dict_body.extend_from_slice(&name_len.to_le_bytes());
        dict_body.extend_from_slice(key.as_bytes());
        dict_body.push(0);
    }
    pad4(&mut dict_body);

    // PROPID 1: CodePage = 1252 (standard ANSI dictionary codepage; string values use VT_LPSTR/VT_LPWSTR)
    let mut cp_body = Vec::with_capacity(8);
    cp_body.extend_from_slice(&0x0002u32.to_le_bytes());
    cp_body.extend_from_slice(&1252u32.to_le_bytes());

    let mut encoded_vals: Vec<(u32, Vec<u8>)> = Vec::with_capacity(props.len() + 2);
    encoded_vals.push((0, dict_body));
    encoded_vals.push((1, cp_body));
    for (idx, val) in props.values().enumerate() {
        let prop_id = (idx as u32) + 2;
        encoded_vals.push((prop_id, encode_user_prop_value(val)));
    }

    let prop_count = encoded_vals.len();
    let header_and_table_len = 8 + prop_count * 8;
    let payload_total: usize = encoded_vals.iter().map(|(_, b)| b.len()).sum();
    let sec_size = header_and_table_len + payload_total;

    let mut sec2 = Vec::with_capacity(sec_size);
    sec2.extend_from_slice(&(sec_size as u32).to_le_bytes());
    sec2.extend_from_slice(&(prop_count as u32).to_le_bytes());

    let mut cur_off = header_and_table_len;
    for (pid, body) in &encoded_vals {
        sec2.extend_from_slice(&pid.to_le_bytes());
        sec2.extend_from_slice(&(cur_off as u32).to_le_bytes());
        cur_off += body.len();
    }
    for (_, body) in encoded_vals {
        sec2.extend_from_slice(&body);
    }
    Ok(sec2)
}

/// Update or insert user-defined string properties in Section 2
/// (`{D5CDD505-2E9C-101B-9397-08002B2CF9AE}`) of `/\x05DocumentSummaryInformation`,
/// preserving Section 1 byte-for-byte when present.
pub fn set_doc_summary_user_properties(
    pkg: &mut PidPackage,
    updates: &BTreeMap<String, String>,
) -> Result<(), String> {
    let mut merged: BTreeMap<String, SummaryPropertyValue> = pkg
        .parsed
        .summary
        .as_ref()
        .map(|s| s.user_properties.clone())
        .unwrap_or_default();

    for (k, v) in updates {
        let new_val = if v.is_ascii() {
            SummaryPropertyValue::Lpstr(v.clone())
        } else {
            SummaryPropertyValue::Lpwstr(v.clone())
        };
        merged.insert(k.clone(), new_val);
    }

    let (prefix24, sec1_bytes) = pkg
        .get_stream(DOC_SUMMARY_STREAM_PATH)
        .and_then(|s| extract_existing_header_and_sec1(&s.data))
        .unwrap_or_else(|| {
            let mut p = [0u8; 24];
            p[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
            (p, minimal_section_1_bytes())
        });

    let sec2_bytes = encode_section_2(&merged)?;
    let sec1_off = 28u32 + 40u32; // 28B header + 2 × 20B section directory entries = 68
    let sec2_off = sec1_off + (sec1_bytes.len() as u32);

    let mut out = Vec::with_capacity((sec2_off as usize) + sec2_bytes.len());
    out.extend_from_slice(&prefix24);
    out.extend_from_slice(&2u32.to_le_bytes()); // NumPropertySets = 2
    out.extend_from_slice(&FMTID_DOC_SUMMARY_SEC1);
    out.extend_from_slice(&sec1_off.to_le_bytes());
    out.extend_from_slice(&FMTID_DOC_SUMMARY_SEC2);
    out.extend_from_slice(&sec2_off.to_le_bytes());
    out.extend_from_slice(&sec1_bytes);
    out.extend_from_slice(&sec2_bytes);

    pkg.replace_stream(DOC_SUMMARY_STREAM_PATH, out);
    Ok(())
}

/// Stamp a seed package with drawing identity (`_SPID = doc_uid`, `_PlantName = plant`,
/// optional `_PIDTemplate`) in `DocumentSummaryInformation` Section 2 (`Q10`).
pub fn stamp_seed_drawing_identity(
    pkg: &mut PidPackage,
    doc_uid: &str,
    plant: &str,
    template_name: Option<&str>,
) -> Result<(), String> {
    let mut updates = BTreeMap::new();
    updates.insert("_SPID".to_string(), doc_uid.trim().to_string());
    updates.insert("_PlantName".to_string(), plant.trim().to_string());
    if let Some(tpl) = template_name {
        if !tpl.trim().is_empty() {
            updates.insert("_PIDTemplate".to_string(), tpl.trim().to_string());
        }
    }
    set_doc_summary_user_properties(pkg, &updates)
}

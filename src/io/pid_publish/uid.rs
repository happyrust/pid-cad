//! Unified UID and `GraphicOID` allocator shared between publish XML export
//! and `.pid` generation (`L4-3` / `#123`).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::io::pid_publish_check::default_publish_spec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaEnvelopeUids {
    pub doc_version_uid: String,
    pub doc_revision_uid: String,
    pub file_uid: String,
    pub rel_versioned_doc_uid: String,
    pub rel_revised_doc_uid: String,
    pub rel_file_comp_uid: String,
}

#[derive(Debug, Clone)]
pub struct UidAllocator {
    plant: String,
    drawing_key: String,
    next_graphic_oid: u32,
    used_uids: BTreeSet<String>,
    used_graphic_oids: BTreeSet<u32>,
}

fn hex32_from_hash(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update(b"\x1f");
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for b in &digest[..16] {
        let _ = write!(out, "{b:02X}");
    }
    out
}

fn aaad_guid_from_hash(domain: &str, parts: &[&str]) -> String {
    let hex = hex32_from_hash(domain, parts);
    format!(
        "AAAD{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub fn is_valid_hex32(uid: &str) -> bool {
    uid.len() == 32
        && uid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

impl UidAllocator {
    pub fn new(plant: impl Into<String>, drawing_key: impl Into<String>) -> Self {
        Self {
            plant: plant.into(),
            drawing_key: drawing_key.into(),
            next_graphic_oid: 100,
            used_uids: BTreeSet::new(),
            used_graphic_oids: BTreeSet::new(),
        }
    }

    /// Allocate or preserve a drawing-unique 32-char uppercase hex primary UID.
    pub fn alloc_primary_uid(&mut self, preferred_uid: Option<&str>, stable_key: &str) -> String {
        if let Some(pref) = preferred_uid {
            if is_valid_hex32(pref) && self.used_uids.insert(pref.to_string()) {
                return pref.to_string();
            }
        }
        let mut salt = 0u32;
        loop {
            let key_buf;
            let key_ref = if salt == 0 {
                stable_key
            } else {
                key_buf = format!("{stable_key}#{salt}");
                key_buf.as_str()
            };
            let candidate = hex32_from_hash(
                "ocs/pid-publish/object",
                &[&self.plant, &self.drawing_key, key_ref],
            );
            if self.used_uids.insert(candidate.clone()) {
                return candidate;
            }
            salt = salt.saturating_add(1);
        }
    }

    /// Allocate or preserve a storage-unique `GraphicOID` (`< 0x2000` in segment 0
    /// first, advancing monotonically without collisions).
    pub fn alloc_graphic_oid(&mut self, preferred: Option<u32>) -> u32 {
        if let Some(oid) = preferred {
            if oid > 0 && self.used_graphic_oids.insert(oid) {
                if oid >= self.next_graphic_oid {
                    self.next_graphic_oid = oid.saturating_add(1);
                }
                return oid;
            }
        }
        while !self.used_graphic_oids.insert(self.next_graphic_oid) {
            self.next_graphic_oid = self.next_graphic_oid.saturating_add(1);
        }
        let assigned = self.next_graphic_oid;
        self.next_graphic_oid = self.next_graphic_oid.saturating_add(1);
        assigned
    }
}

pub fn port_uid(owner_uid: &str, port_index: u32) -> Result<String, String> {
    if !is_valid_hex32(owner_uid) {
        return Err(format!(
            "owner UID must be 32-char uppercase hex, got '{owner_uid}'"
        ));
    }
    if port_index == 0 {
        return Err("port_index must be >= 1".to_string());
    }
    Ok(format!("{owner_uid}.{port_index}"))
}

pub fn branch_point_uid(owner_uid: &str) -> Result<String, String> {
    if !is_valid_hex32(owner_uid) {
        return Err(format!(
            "owner UID must be 32-char uppercase hex, got '{owner_uid}'"
        ));
    }
    Ok(format!("{owner_uid}.BPT"))
}

pub fn process_point_uid(connector_uid: &str) -> Result<String, String> {
    if !is_valid_hex32(connector_uid) {
        return Err(format!(
            "connector UID must be 32-char uppercase hex, got '{connector_uid}'"
        ));
    }
    Ok(format!("{connector_uid}.PPT"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pipeline_uid(plant: &str, pipeline_name: &str) -> String {
    pid_parse::publish::pipeline::pipeline_uid(plant, pipeline_name)
}

#[cfg(target_arch = "wasm32")]
pub fn pipeline_uid(plant: &str, pipeline_name: &str) -> String {
    hex32_from_hash("pid-parse/pipeline", &[plant, pipeline_name])
}

pub fn relation_uid(def_uid: &str, uid1: &str, uid2: &str) -> Result<String, String> {
    let spec = default_publish_spec();
    let rel = spec
        .data_relations
        .get(def_uid)
        .ok_or_else(|| format!("unknown data relation DefUID '{def_uid}'"))?;
    Ok(format!("{}-{uid1}-{uid2}", rel.prefix))
}

pub fn derive_meta_envelope_uids(plant: &str, doc_uid: &str) -> MetaEnvelopeUids {
    MetaEnvelopeUids {
        doc_version_uid: hex32_from_hash("ocs/pid-publish/meta-dv", &[plant, doc_uid]),
        doc_revision_uid: hex32_from_hash("ocs/pid-publish/meta-dr", &[plant, doc_uid]),
        file_uid: aaad_guid_from_hash("ocs/pid-publish/meta-file", &[plant, doc_uid]),
        rel_versioned_doc_uid: hex32_from_hash("ocs/pid-publish/meta-rel-vd", &[plant, doc_uid]),
        rel_revised_doc_uid: hex32_from_hash("ocs/pid-publish/meta-rel-rd", &[plant, doc_uid]),
        rel_file_comp_uid: aaad_guid_from_hash("ocs/pid-publish/meta-rel-fc", &[plant, doc_uid]),
    }
}

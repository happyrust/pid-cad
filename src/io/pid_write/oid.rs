//! Persistent-ID (`GraphicOID`) allocator and sheet-layer target resolver
//! initialized from a seed `PidPackage` (`L5-5` / `#129`).

use std::collections::BTreeSet;

use pid_parse::parsers::cluster_header::decode_psm_cluster0_body_records;
use pid_parse::parsers::sheet_records::{parse_live_psm_header, SheetChain};
use pid_parse::PidPackage;

/// Primary top-level sheet stream path and resolved `JSheetLayer` OIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetLayerTargets {
    pub sheet_stream_path: String,
    pub default_layer_oid: u32,
    pub labels_layer_oid: u32,
}

/// Storage-scoped persistent-ID pool avoiding collisions with all OIDs already
/// present in a seed `.pid` package.
#[derive(Debug, Clone)]
pub struct SeedOidPool {
    next_oid: u32,
    used_oids: BTreeSet<u32>,
    targets: SheetLayerTargets,
}

fn collect_chain_oids(data: &[u8], out: &mut BTreeSet<u32>) {
    for at in SheetChain::walk(data).starts {
        if let Some(hdr) = parse_live_psm_header(data, at) {
            if let Some(b) = data.get(hdr.body_start..hdr.body_start + 4) {
                let oid = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if oid > 0 {
                    out.insert(oid);
                }
            }
        }
    }
}

fn parse_segment_num_from_spacemap_path(path: &str) -> Option<u32> {
    let rest = path.strip_prefix("/PSMspacemap/0x")?;
    let raw = u32::from_str_radix(rest, 16).ok()?;
    Some(raw >> 13)
}

impl SeedOidPool {
    /// Inspect `seed` to collect all existing top-level OIDs, `PSMspacemap`
    /// watermarks, `JSite<id>` numbers, and `Default` / `Labels` layer OIDs.
    pub fn from_seed(seed: &PidPackage) -> Self {
        let mut used_oids: BTreeSet<u32> = BTreeSet::new();
        let mut watermark: u32 = 32;

        // 1. Existing PSMroots ids
        if let Some(ref roots) = seed.parsed.psm_roots {
            for r in &roots.entries {
                if r.id > 0 {
                    used_oids.insert(r.id);
                    watermark = watermark.max(r.id.saturating_add(1));
                }
            }
        }

        // 2. Top-level PSMspacemap segments (`/PSMspacemap/0x...`)
        for (sm_path, sm) in &seed.parsed.psm_space_maps {
            if let Some(seg) = parse_segment_num_from_spacemap_path(sm_path) {
                let free_set: BTreeSet<u16> = sm.free_list.iter().copied().collect();
                for idx in 1..sm.next_free_index {
                    if !free_set.contains(&idx) {
                        let oid = (seg << 13) | u32::from(idx);
                        used_oids.insert(oid);
                    }
                }
                let next_in_seg = (seg << 13) | u32::from(sm.next_free_index);
                watermark = watermark.max(next_in_seg);
                for entry in &sm.entries {
                    let oid = (seg << 13) | u32::from(entry.index);
                    used_oids.insert(oid);
                }
            }
        }

        // 3. Top-level JSite<id> storages
        for js in &seed.parsed.jsites {
            if let Some(num_str) = js.name.strip_prefix("JSite") {
                if let Ok(site_id) = num_str.parse::<u32>() {
                    if site_id > 0 {
                        used_oids.insert(site_id);
                        watermark = watermark.max(site_id.saturating_add(1));
                    }
                }
            }
        }

        // 4. Top-level record chains (`/Sheet*`, `/Unclustered Dynamic Attributes`, `/PSMcluster0`)
        let mut sheet_stream_path: Option<String> = None;
        for (path, raw) in &seed.streams {
            let trimmed = path.trim_start_matches('/');
            if trimmed.contains('/') {
                continue;
            }
            if trimmed.starts_with("Sheet") {
                if sheet_stream_path.is_none() {
                    sheet_stream_path = Some(path.clone());
                }
                collect_chain_oids(&raw.data, &mut used_oids);
            } else if trimmed == "Unclustered Dynamic Attributes" {
                collect_chain_oids(&raw.data, &mut used_oids);
            } else if trimmed == "PSMcluster0" {
                for rec in decode_psm_cluster0_body_records(&raw.data) {
                    if rec.raw_payload.len() >= 4 {
                        let oid = u32::from_le_bytes([
                            rec.raw_payload[0],
                            rec.raw_payload[1],
                            rec.raw_payload[2],
                            rec.raw_payload[3],
                        ]);
                        if oid > 0 {
                            used_oids.insert(oid);
                        }
                    }
                }
            }
        }

        if let Some(&max_used) = used_oids.iter().max() {
            watermark = watermark.max(max_used.saturating_add(1));
        }

        // 5. Resolve Default and Labels layer OIDs on root `/`
        let mut default_layer_oid = 8u32;
        let mut labels_layer_oid = 12u32;
        if let Some(root_layers) = seed.parsed.sheet_layers.get("/") {
            if let Some(dl) = root_layers.iter().find(|l| l.name == "Default") {
                default_layer_oid = dl.oid;
            } else if let Some(first) = root_layers.first() {
                default_layer_oid = first.oid;
            }
            if let Some(ll) = root_layers
                .iter()
                .find(|l| l.name == "Labels" || l.name == "Label")
            {
                labels_layer_oid = ll.oid;
            } else {
                labels_layer_oid = default_layer_oid;
            }
        }

        Self {
            next_oid: watermark.max(100),
            used_oids,
            targets: SheetLayerTargets {
                sheet_stream_path: sheet_stream_path.unwrap_or_else(|| "/Sheet6".to_string()),
                default_layer_oid,
                labels_layer_oid,
            },
        }
    }

    pub fn targets(&self) -> &SheetLayerTargets {
        &self.targets
    }

    pub fn is_used(&self, oid: u32) -> bool {
        self.used_oids.contains(&oid)
    }

    pub fn used_oids(&self) -> &BTreeSet<u32> {
        &self.used_oids
    }

    /// Reserve a preferred OID if free, or allocate the next monotonically
    /// increasing unused OID (`>= 1`, skipping segment-0 index 0 boundaries `(seg << 13)`).
    pub fn alloc_oid(&mut self, preferred: Option<u32>) -> u32 {
        if let Some(p) = preferred {
            if p > 0 && (p & 0x1FFF) != 0 && self.used_oids.insert(p) {
                if p >= self.next_oid {
                    self.next_oid = p.saturating_add(1);
                }
                return p;
            }
        }
        loop {
            if self.next_oid > 0
                && (self.next_oid & 0x1FFF) != 0
                && self.used_oids.insert(self.next_oid)
            {
                let out = self.next_oid;
                self.next_oid = self.next_oid.saturating_add(1);
                return out;
            }
            self.next_oid = self.next_oid.saturating_add(1);
        }
    }

    pub fn max_allocated_in_segment(&self, segment: u32) -> Option<u16> {
        self.used_oids
            .iter()
            .copied()
            .filter(|&id| (id >> 13) == segment)
            .map(|id| (id & 0x1FFF) as u16)
            .max()
    }
}

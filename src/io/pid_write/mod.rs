//! SmartPlant `.pid` seed-plus-increment CFB writer (`L5` / `#128`..`#130`).

pub mod index;
pub mod oid;
pub mod records;
pub mod seed;
pub mod surface;
pub mod verify;

pub use index::{
    apply_increments_to_seed, verify_package_index_consistency, AddedAttributeSpec, AddedLineSpec,
    AddedPolylineSpec, AddedSymbolSpec, AddedTextSpec, IndexCheckReport, PidWriteIncrements,
    PidWriteStats,
};
pub use oid::{SeedOidPool, SheetLayerTargets};
pub use records::{
    encode_freeform_attr_row_record, encode_igline2d_record, encode_iglinestring2d_record,
    encode_igsymbol2d_record, encode_igtextbox_record, mm_to_sheet_m,
};
pub use seed::{
    list_seeds_in_refdata_zip, load_seed_from_bytes, load_seed_from_path,
    load_seed_from_refdata_zip, set_doc_summary_user_properties, stamp_seed_drawing_identity,
};
pub use surface::{inspect_write_surface, WriteDisposition, WriteSurfaceEntry, WriteSurfaceReport};
pub use verify::{verify_passthrough_roundtrip, SeedRoundtripReport};

#[cfg(test)]
mod tests {
    use super::*;
    use pid_parse::{
        PidDocument, PidPackage, PidWriter, RawStream, SummaryPropertyValue, WritePlan,
    };
    use std::collections::BTreeMap;

    fn synthetic_seed_package() -> PidPackage {
        let mut streams = BTreeMap::new();
        let insert_raw = |streams: &mut BTreeMap<String, RawStream>, path: &str, data: Vec<u8>| {
            streams.insert(
                path.to_string(),
                RawStream {
                    path: path.to_string(),
                    data,
                    modified: false,
                },
            );
        };

        // Empty `0x6C90F544` record chains for Sheet6, PSMcluster0, StyleCluster
        let mut empty_chain = 0x6C90_F544u32.to_le_bytes().to_vec();
        empty_chain.extend_from_slice(&0u32.to_le_bytes());

        insert_raw(&mut streams, "/Sheet6", empty_chain.clone());
        insert_raw(&mut streams, "/PSMcluster0", empty_chain.clone());
        insert_raw(
            &mut streams,
            "/Unclustered Dynamic Attributes",
            empty_chain.clone(),
        );
        insert_raw(&mut streams, "/StyleCluster", empty_chain);

        // Minimal `PSMroots` (`'root'` + terminator)
        let mut roots = 0x746F_6F72u32.to_le_bytes().to_vec();
        roots.extend_from_slice(&[0u8; 8]);
        insert_raw(&mut streams, "/PSMroots", roots);

        // Minimal `PSMspacemap/0x00000000` (`'tseg'` + 0 entries + next_idx=10 + 0 free)
        let mut tseg = 0x6765_7374u32.to_le_bytes().to_vec();
        tseg.extend_from_slice(&0u16.to_le_bytes());
        tseg.extend_from_slice(&4u16.to_le_bytes());
        tseg.extend_from_slice(&10u16.to_le_bytes());
        tseg.extend_from_slice(&0u16.to_le_bytes());
        insert_raw(&mut streams, "/PSMspacemap/0x00000000", tseg);

        // 0-byte `/TaggedTxtData/Revision` matching real production `.pid` files (Q11)
        insert_raw(&mut streams, "/TaggedTxtData/Revision", Vec::new());

        let raw_pkg = PidPackage::new(None, streams, PidDocument::default());
        let bytes = PidWriter::write_to_bytes(&raw_pkg, &WritePlan::default()).unwrap();
        PidPackage::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn classifies_cfb_streams_into_expected_write_dispositions() {
        let pkg = synthetic_seed_package();
        let report = inspect_write_surface(&pkg);
        assert_eq!(report.total_raw_streams, 7);

        let by_name: BTreeMap<&str, WriteDisposition> = report
            .entries
            .iter()
            .map(|e| (e.top_level_name.as_str(), e.disposition))
            .collect();
        assert_eq!(
            by_name.get("Sheet6"),
            Some(&WriteDisposition::AppendRecords)
        );
        assert_eq!(
            by_name.get("PSMcluster0"),
            Some(&WriteDisposition::AppendRecords)
        );
        assert_eq!(
            by_name.get("PSMspacemap"),
            Some(&WriteDisposition::SyncIndex)
        );
        assert_eq!(
            by_name.get("StyleCluster"),
            Some(&WriteDisposition::PassthroughPreserve)
        );
        assert_eq!(
            by_name.get("TaggedTxtData"),
            Some(&WriteDisposition::PassthroughPreserve)
        );
    }

    #[test]
    fn passthrough_roundtrip_preserves_every_stream_and_clsid() {
        let pkg = synthetic_seed_package();
        let rt = verify_passthrough_roundtrip(&pkg).expect("passthrough roundtrip");
        assert!(rt.is_lossless_passthrough(), "unexpected diff: {rt:?}");
        assert_eq!(rt.stream_count, 7);
    }

    #[test]
    fn stamps_spid_and_plant_into_doc_summary_section2_and_reparses_cleanly() {
        let mut pkg = synthetic_seed_package();
        let doc_uid = "0123456789ABCDEF0123456789ABCDEF";
        stamp_seed_drawing_identity(&mut pkg, doc_uid, "P01", Some("A1-W-New.igr"))
            .expect("stamp section 2");

        let out_bytes = PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap();
        let reparsed = load_seed_from_bytes(&out_bytes).expect("reparse stamped seed");
        let summary = reparsed.parsed.summary.as_ref().expect("summary decoded");

        fn str_val(v: &SummaryPropertyValue) -> Option<&str> {
            match v {
                SummaryPropertyValue::Lpstr(s) | SummaryPropertyValue::Lpwstr(s) => {
                    Some(s.as_str())
                }
                _ => None,
            }
        }
        assert_eq!(
            summary.user_properties.get("_SPID").and_then(str_val),
            Some(doc_uid)
        );
        assert_eq!(
            summary.user_properties.get("_PlantName").and_then(str_val),
            Some("P01")
        );
        assert_eq!(
            summary
                .user_properties
                .get("_PIDTemplate")
                .and_then(str_val),
            Some("A1-W-New.igr")
        );
    }

    #[test]
    #[ignore = "verifies all 10 blank seed .pid templates in RefData~4~682.zip and production write surface"]
    fn real_refdata_10_blank_seeds_roundtrip_and_surface_audit() {
        let Ok(zip_path) = std::env::var("OCS_PID_SEED_ZIP") else {
            println!("未设置种子压缩包（OCS_PID_SEED_ZIP），跳过真实空白种子往返验证。");
            return;
        };
        let zp = std::path::Path::new(&zip_path);
        let seeds = list_seeds_in_refdata_zip(zp).expect("list seeds");
        assert!(!seeds.is_empty());

        let mut audit_rows = Vec::new();
        for (name, bytes_len) in &seeds {
            let mut pkg = load_seed_from_refdata_zip(zp, name).expect("load seed from zip");
            let surf = inspect_write_surface(&pkg);
            let rt = verify_passthrough_roundtrip(&pkg).expect("roundtrip");
            assert!(
                rt.is_lossless_passthrough(),
                "seed {name} failed passthrough roundtrip: {rt:?}"
            );

            // Also verify stamping _SPID on the real vendor seed preserves all other streams
            stamp_seed_drawing_identity(
                &mut pkg,
                "FEDCBA9876543210FEDCBA9876543210",
                "P01",
                Some("A1-W-New.igr"),
            )
            .expect("stamp real seed");
            let stamped_bytes = PidWriter::write_to_bytes(&pkg, &WritePlan::default()).unwrap();
            let reparsed = load_seed_from_bytes(&stamped_bytes).unwrap();
            let spid_ok = reparsed
                .parsed
                .summary
                .as_ref()
                .and_then(|s| s.user_properties.get("_SPID"))
                .is_some_and(|v| {
                    matches!(
                        v,
                        SummaryPropertyValue::Lpstr(t) | SummaryPropertyValue::Lpwstr(t)
                            if t == "FEDCBA9876543210FEDCBA9876543210"
                    )
                });
            assert!(spid_ok, "seed {name} failed _SPID re-read");

            audit_rows.push(serde_json::json!({
                "seed": name,
                "file_bytes": bytes_len,
                "stream_count": rt.stream_count,
                "lossless_passthrough": rt.is_lossless_passthrough(),
                "by_disposition": surf.by_disposition,
            }));
        }

        if let Ok(out_file) = std::env::var("OCS_PID_SEED_AUDIT_OUT") {
            std::fs::write(out_file, serde_json::to_string_pretty(&audit_rows).unwrap()).unwrap();
        }
        println!("verified_blank_seeds={}", audit_rows.len());
    }
}

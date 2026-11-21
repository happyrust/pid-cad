//! DWG/DXF-to-Smart-P&ID conversion and SPPID class/tag/publish bridging (`L3` + `L4`).
//!
//! Kept in its own module tree (`src/io/pid_convert/`) so recognition mapping,
//! tag pairing, and publish conversion evolve independently without colliding
//! with `src/io/pid/` or `src/io/pid_legend/`.

pub mod bom;
pub mod class_map;
pub mod drc;
pub mod export;
pub mod external_props;
pub mod opc;
pub mod promote;
pub mod recognize;

pub use drc::{audit_sheet_drc, DrcFinding, DrcKind, DrcSeverity, DrcSummary, SheetDrcReport};

#[cfg(not(target_arch = "wasm32"))]
pub use bom::{extract_bom_from_batch_dir, write_bom_schedule_file};
pub use bom::{
    extract_bom_from_documents, format_bom_delimited, format_bom_json, ingest_data_xml_into_bom,
    BomScheduleReport, BomScheduleRow, BomScheduleSummary, BomSheetInput, BOM_CSV_HEADERS,
};

pub use opc::{
    analyze_open_sheets_opc, OpcConnectorItem, OpcPeerRef, OpcSheetInput, OpcStatus,
    WorkspaceOpcReport,
};

#[cfg(not(target_arch = "wasm32"))]
pub use promote::save_author_document_to_dual;
pub use promote::{
    accept_author_handles, deduplicate_author_tags, expand_author_member_handles,
    filter_author_handles, has_native_pid_semantics, promote_semantics_to_author,
    promote_sheet_to_author, reject_author_handles, resolve_owning_author_handles,
    sppid_class_to_author_type, summarize_author_review, sync_author_after_edit,
    sync_paired_canvas_tag_texts, AuthorReviewStats, AuthorSyncReport, PromoteReport,
};

pub use class_map::{
    audit_document_blocks, default_legend_class_map, BlockAuditSummary, BlockMatchKind,
    BlockOverrideRule, BlockResolution, IgnoreReason, LegendClassMap, LegendMapRules, MappedBlock,
};
#[cfg(not(target_arch = "wasm32"))]
pub use export::{
    convert_document_to_dual, convert_path_to_dir, export_author_document_to_dir,
    export_document_to_dir, save_cad_file_as_binary_dwg, verify_batch_dir_strict,
    verify_dual_bundle, verify_published_xml_pair, DualConvertBundle,
};
pub use export::{
    convert_document_to_publish, pair_cross_sheet_connectors, BatchConvertSummary, ConvertOptions,
    ConvertSheetReport, CrossSheetOpcPair, CrossSheetOpcReport, DeadEndAttribution,
    DualVerifyReport, OffpageConnectorEntry, PipeNetworkCensus,
};
#[cfg(not(target_arch = "wasm32"))]
pub use external_props::load_external_property_table;
pub use external_props::{
    apply_external_properties_to_drawing, normalize_lookup_key, parse_external_property_delimited,
    parse_external_property_json, ExternalItemRecord, ExternalPropertyTable, ExternalPropsReport,
};
pub use recognize::{
    recognise_sppid_sheet, ConfidenceSummary, ConfidenceThresholds, ConfidenceTier,
    ConvertedSymbol, PipeLabelHit, PipeSegmentHit, RangeTagHit, SheetRecognition, TitleBlockInfo,
};

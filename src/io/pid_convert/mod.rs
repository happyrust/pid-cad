//! DWG/DXF-to-Smart-P&ID conversion and SPPID class/tag/publish bridging (`L3` + `L4`).
//!
//! Kept in its own module tree (`src/io/pid_convert/`) so recognition mapping,
//! tag pairing, and publish conversion evolve independently without colliding
//! with `src/io/pid/` or `src/io/pid_legend/`.

pub mod bom;
pub mod class_map;
pub mod diff;
pub mod drc;
pub mod export;
pub mod external_props;
pub mod find;
pub mod isolate;
pub mod loops;
pub mod opc;
pub mod promote;
pub mod recognize;
pub mod spec_check;
pub mod trace;

pub use diff::{
    diff_pid_documents, format_pid_diff_delimited, DiffFieldChange, DiffKind, PidDiffItem,
    PidDiffReport, PidDiffSummary, DIFF_CSV_HEADERS,
};
pub use loops::{
    analyze_pid_loops, format_pid_loops_delimited, loop_matches_query, parse_isa_instrument_tag,
    IsaLoopRole, LoopStatus, ParsedIsaTag, PidInstrumentLoop, PidLoopMember, PidLoopReport,
    PidLoopSummary, LOOP_CSV_HEADERS,
};
#[cfg(not(target_arch = "wasm32"))]
pub use spec_check::write_pid_spec_file;
pub use spec_check::{
    audit_pid_spec_consistency, format_pid_spec_delimited, PidSpecFinding, PidSpecReport,
    PidSpecSummary, SpecFindingKind, SPEC_CSV_HEADERS,
};

#[cfg(not(target_arch = "wasm32"))]
pub use loops::{extract_loops_from_batch_dir, write_pid_loops_file};

#[cfg(not(target_arch = "wasm32"))]
pub use isolate::write_pid_isolation_file;
pub use isolate::{
    format_pid_isolation_delimited, solve_pid_isolation_by_query, solve_pid_isolation_from_handles,
    IsolationHazardKind, PidIsolationHazard, PidIsolationReport, PidIsolationSummary,
    PidIsolationValve, ISOLATION_CSV_HEADERS,
};

#[cfg(not(target_arch = "wasm32"))]
pub use diff::{diff_pid_files, write_pid_diff_file};

pub use trace::{
    trace_shortest_path_by_query, trace_subnet_by_query, trace_subnet_from_handles, TraceMode,
    TraceNodeKind, TraceReport, TraceStep,
};

pub use find::{match_query_rank, search_pid_entities, PidFindHit, PidFindKind};

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

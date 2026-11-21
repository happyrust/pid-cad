//! Unified intermediate export model for SmartPlant publish XML (`L4-1` / `#123`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmappedItem {
    pub source_id: String,
    pub raw_class: String,
    pub tag: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportItem {
    /// Already-allocated UID (`hex32`, `.N`, `.BPT`, or `.PPT`).
    pub uid: String,
    /// SPPID publish class name (one of the 20 non-representation classes).
    pub class: String,
    /// `IObject/@Name` when applicable.
    pub name: Option<String>,
    /// `IObject/@Description` when applicable.
    pub description: Option<String>,
    /// Optional explicit variant id (e.g. `"PIDPipingComponent#01"`).
    pub variant_id: Option<String>,
    /// Optional normalized library-relative `.sym` path used to pick best variant.
    pub symbol_path: Option<String>,
    /// GraphicOIDs on the sheet representing this item (each becomes a `PIDRepresentation`
    /// plus `DRC` and `DRI` relations).
    pub graphic_oids: Vec<u32>,
    /// Explicit `(interface, attribute) -> value` overrides.
    pub attributes: BTreeMap<(String, String), String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRelation {
    pub def_uid: String,
    pub uid1: String,
    pub uid2: String,
    pub order_value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExportDrawing {
    pub plant: String,
    pub doc_uid: String,
    pub doc_name: String,
    pub doc_title: String,
    pub doc_version_date: String,
    pub meta_file_path: String,
    pub drawing_attributes: BTreeMap<(String, String), String>,
    pub items: Vec<ExportItem>,
    pub relations: Vec<ExportRelation>,
    pub unmapped: Vec<UnmappedItem>,
}

impl ExportDrawing {
    pub fn new(
        plant: impl Into<String>,
        doc_uid: impl Into<String>,
        doc_name: impl Into<String>,
    ) -> Self {
        let name_str = doc_name.into();
        Self {
            plant: plant.into(),
            doc_uid: doc_uid.into(),
            doc_title: name_str.clone(),
            doc_name: name_str,
            doc_version_date: "2026/11/20".to_string(),
            meta_file_path: String::new(),
            drawing_attributes: BTreeMap::new(),
            items: Vec::new(),
            relations: Vec::new(),
            unmapped: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportBundle {
    pub data_xml: Vec<u8>,
    pub meta_xml: Vec<u8>,
    pub coverage_json: String,
}

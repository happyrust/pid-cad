//! Class-variant lookup and per-class interface/attribute template builder
//! (`L4-2` / `#123`), backed by `assets/pid-publish-spec.json` and
//! `assets/pid-class-map.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;

use super::model::ExportItem;
use crate::io::pid_publish_check::{default_publish_spec, ClassSpec};

const EMBEDDED_CLASS_MAP_JSON: &str = include_str!("../../../assets/pid-class-map.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogVariant {
    pub variant_id: String,
    pub class: String,
    pub instance_count: usize,
    pub interfaces: Vec<String>,
    pub codelist_values: BTreeMap<String, String>,
    pub graphic_kinds: Vec<String>,
    pub symbol_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassMapSpec {
    pub schema_version: u32,
    pub total_variants: usize,
    pub variants: Vec<CatalogVariant>,
}

pub fn default_class_map() -> &'static ClassMapSpec {
    static MAP: OnceLock<ClassMapSpec> = OnceLock::new();
    MAP.get_or_init(|| {
        serde_json::from_str(EMBEDDED_CLASS_MAP_JSON)
            .expect("embedded assets/pid-class-map.json must be valid")
    })
}

pub fn resolve_variant<'a>(
    map: &'a ClassMapSpec,
    class: &str,
    variant_id: Option<&str>,
    symbol_path: Option<&str>,
) -> Option<&'a CatalogVariant> {
    if let Some(vid) = variant_id {
        if let Some(hit) = map
            .variants
            .iter()
            .find(|v| v.class == class && v.variant_id == vid)
        {
            return Some(hit);
        }
    }
    if let Some(sym) = symbol_path {
        let norm = sym
            .trim()
            .trim_start_matches(['\\', '/'])
            .replace('/', "\\");
        // `variants` are already sorted by descending `instance_count` per class,
        // so the first variant whose `symbol_paths` matches is the most frequent one.
        if let Some(hit) = map.variants.iter().find(|v| {
            v.class == class && v.symbol_paths.iter().any(|p| p.eq_ignore_ascii_case(&norm))
        }) {
            return Some(hit);
        }
    }
    map.variants.iter().find(|v| v.class == class)
}

fn default_attr_constant(class: &str, iface: &str, attr: &str) -> &'static str {
    match (class, iface, attr) {
        ("PIDDrawing", "IDocument", "DocCategory") => "P&ID Documents",
        ("PIDDrawing", "IDocument", "DocType") => "P&ID",
        (_, "IDrawingItem", "IsTypical") => "False",
        (_, "IPIDTypical", "IsTypical") => "False",
        (_, "IConnector", "RepresentationsAreAllZeroLength") => "False",
        (_, "IInlineComponent", "IsFlowDirectional") => "False",
        (_, "IExpandableThing", "IsExpanded") => "False",
        _ => "",
    }
}

pub type OrderedInterfaces = Vec<(String, Vec<(String, String)>)>;

pub fn materialize_class_interfaces(
    class: &str,
    cls_spec: &ClassSpec,
    uid: &str,
    name: Option<&str>,
    description: Option<&str>,
    variant: Option<&CatalogVariant>,
    overrides: &BTreeMap<(String, String), String>,
) -> OrderedInterfaces {
    let allowed_ifaces: BTreeSet<&str> = cls_spec
        .allowed_interfaces
        .iter()
        .map(String::as_str)
        .collect();
    let mut chosen_ifaces: BTreeSet<String> =
        cls_spec.required_interfaces.iter().cloned().collect();
    if let Some(v) = variant {
        for iface in &v.interfaces {
            if allowed_ifaces.contains(iface.as_str()) {
                chosen_ifaces.insert(iface.clone());
            }
        }
    }
    for (iface, _attr) in overrides.keys() {
        if allowed_ifaces.contains(iface.as_str()) {
            chosen_ifaces.insert(iface.clone());
        }
    }
    chosen_ifaces.insert("IObject".to_string());

    // Order interfaces with `IObject` strictly first, followed by alphabetical interfaces.
    let mut iface_order = vec!["IObject".to_string()];
    for iface in chosen_ifaces {
        if iface != "IObject" {
            iface_order.push(iface);
        }
    }

    let mut out = Vec::with_capacity(iface_order.len());
    for iface in iface_order {
        let attr_spec = cls_spec.interface_attributes.get(&iface);
        let allowed_attrs: BTreeSet<&str> = attr_spec
            .map(|s| s.allowed.iter().map(String::as_str).collect())
            .unwrap_or_default();

        let mut attrs_map: BTreeMap<String, String> = BTreeMap::new();
        if let Some(s) = attr_spec {
            for req_a in &s.required {
                attrs_map.insert(
                    req_a.clone(),
                    default_attr_constant(class, &iface, req_a).to_string(),
                );
            }
        }

        if let Some(v) = variant {
            let prefix = format!("{iface}.");
            for (k, val) in &v.codelist_values {
                if let Some(attr_name) = k.strip_prefix(&prefix) {
                    if allowed_attrs.contains(attr_name) {
                        attrs_map.insert(attr_name.to_string(), val.clone());
                    }
                }
            }
        }

        if iface == "IObject" {
            attrs_map.insert("UID".to_string(), uid.to_string());
            if let Some(n) = name {
                if allowed_attrs.contains("Name") {
                    attrs_map.insert("Name".to_string(), n.to_string());
                }
            }
            if let Some(d) = description {
                if allowed_attrs.contains("Description") {
                    attrs_map.insert("Description".to_string(), d.to_string());
                }
            }
        }

        for ((ov_iface, ov_attr), ov_val) in overrides {
            if ov_iface == &iface && allowed_attrs.contains(ov_attr.as_str()) {
                attrs_map.insert(ov_attr.clone(), ov_val.clone());
            }
        }

        // Put `UID`, `Name`, `Description` in canonical leading order on `IObject`.
        let mut attr_vec = Vec::with_capacity(attrs_map.len());
        if iface == "IObject" {
            for lead in ["UID", "Name", "Description"] {
                if let Some(val) = attrs_map.remove(lead) {
                    attr_vec.push((lead.to_string(), val));
                }
            }
        }
        for (k, v) in attrs_map {
            attr_vec.push((k, v));
        }

        out.push((iface, attr_vec));
    }
    out
}

pub fn materialize_export_item(item: &ExportItem) -> Result<OrderedInterfaces, String> {
    let spec = default_publish_spec();
    let cls_spec = spec
        .data_classes
        .get(&item.class)
        .ok_or_else(|| format!("unknown SPPID publish class '{}'", item.class))?;
    let variant = resolve_variant(
        default_class_map(),
        &item.class,
        item.variant_id.as_deref(),
        item.symbol_path.as_deref(),
    );
    Ok(materialize_class_interfaces(
        &item.class,
        cls_spec,
        &item.uid,
        item.name.as_deref(),
        item.description.as_deref(),
        variant,
        &item.attributes,
    ))
}

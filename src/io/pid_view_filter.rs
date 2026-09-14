//! The document-level P&ID view filter: which authored sheet layers and which
//! import roles are switched off, and the `invisible` bit that carries the
//! answer onto every entity.
//!
//! A `.pid` import files each entity on a synthetic `PID-*` layer and writes
//! the two things that layer cannot hold at once into its `PID_SEMANTICS`
//! XDATA: the sheet layer the drawing itself puts the entity on
//! (`sheet_layer=`) and what the importer read it as (`role=`). This filter is
//! the on/off state along both of those axes, kept in one place in the
//! drawing -- an XRecord named [`XRECORD_KEY`] in the model-space block
//! record's extension dictionary -- and applied as
//!
//! ```text
//! invisible = !(sheet_layer_on && role_on)
//! ```
//!
//! over every entity that states a sheet layer or a role. An entity without a
//! `sheet_layer=` -- the page border, the connectivity links, a symbol's own
//! label, the glyph strokes a style cluster contributes -- was never on any
//! sheet layer, so it counts as on there and answers to its role alone.
//! Entities carrying neither key are not the filter's to touch: legend
//! recognition's records on a DXF drawing, or a user's own `invisible`.
//!
//! Only the switched-off names are stored; everything is on until said
//! otherwise, and an all-on filter stores nothing rather than leave a record
//! that would say the same as no record. Both the record and the bits ride
//! DWG and DXF saves on their own, so a reopened drawing needs no re-apply.
//!
//! The starting point is the one reading the importer already had: the sheet
//! layers SmartPlant hides by name (`Hidden` / `HiddenObjects` / `Invisible`)
//! start switched off -- see [`is_hidden_sheet_layer`], the single function to
//! change once the file's own display state is decoded (plan 2026-09-07, L1).

use std::collections::BTreeSet;

use acadrust::objects::{XRecordEntry, XRecordValue};
use acadrust::xdata::XDataValue;
use acadrust::{CadDocument, EntityType, Handle};

use super::PID_SEMANTICS_XDATA_APP;

/// The XRecord key the filter is stored under, in the model-space block
/// record's extension dictionary.
pub const XRECORD_KEY: &str = "PID_VIEW_FILTER";

const LAYER_OFF: &str = "layer_off=";
const ROLE_OFF: &str = "role_off=";

/// Whether an authored sheet layer name means "SmartPlant does not draw this":
/// `Hidden`, `HiddenObjects` or `Invisible`, trimmed, case-folded and with
/// inner whitespace ignored, so `Hidden Objects` and `HIDDENOBJECTS` both
/// count. This name criterion is the filter's starting point and the reason
/// an import moves such entities to `PID-HIDDEN`; when the view filter set's
/// display state is decoded it is this one function that changes.
pub fn is_hidden_sheet_layer(name: &str) -> bool {
    let normalized: String = name
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "hidden" | "hiddenobjects" | "invisible"
    )
}

/// The on/off state of a drawing's sheet layers and import roles. Everything
/// not named here is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PidViewFilter {
    layers_off: BTreeSet<String>,
    roles_off: BTreeSet<String>,
}

impl PidViewFilter {
    /// The filter the importer starts a drawing with: every authored sheet
    /// layer named on an entity that [`is_hidden_sheet_layer`] is off, no
    /// role is.
    pub fn initial(doc: &CadDocument) -> Self {
        let mut filter = Self::default();
        for entity in doc.entities() {
            let Some(keys) = pid_keys(entity) else {
                continue;
            };
            if let Some(layer) = keys.sheet_layer {
                if is_hidden_sheet_layer(layer) {
                    filter.layers_off.insert(layer.to_string());
                }
            }
        }
        filter
    }

    /// The filter the drawing stores, or `None` when it carries no record --
    /// which reads as everything on.
    pub fn load(doc: &CadDocument) -> Option<Self> {
        let record = doc.xrecord(owner(doc), XRECORD_KEY)?;
        let mut filter = Self::default();
        for entry in &record.entries {
            let XRecordValue::String(text) = &entry.value else {
                continue;
            };
            if let Some(layer) = text.strip_prefix(LAYER_OFF) {
                filter.layers_off.insert(layer.to_string());
            } else if let Some(role) = text.strip_prefix(ROLE_OFF) {
                filter.roles_off.insert(role.to_string());
            }
        }
        Some(filter)
    }

    /// Write the filter into the drawing, replacing whatever record was
    /// there. An empty filter removes the record instead of storing "all on".
    pub fn store(&self, doc: &mut CadDocument) {
        let owner = owner(doc);
        if self.is_empty() {
            remove_xrecord(doc, owner);
            return;
        }
        doc.ensure_xrecord(owner, XRECORD_KEY);
        if let Some(record) = doc.xrecord_mut(owner, XRECORD_KEY) {
            record.entries.clear();
            for layer in &self.layers_off {
                record
                    .entries
                    .push(XRecordEntry::string(1, format!("{LAYER_OFF}{layer}")));
            }
            for role in &self.roles_off {
                record
                    .entries
                    .push(XRecordEntry::string(1, format!("{ROLE_OFF}{role}")));
            }
        }
    }

    /// Set every entity's `invisible` bit from the filter, and return how
    /// many entities changed. Only entities stating a sheet layer or a role
    /// are touched.
    pub fn apply(&self, doc: &mut CadDocument) -> usize {
        let mut changed = 0;
        for entity in doc.entities_mut() {
            let Some(keys) = pid_keys(entity) else {
                continue;
            };
            if keys.sheet_layer.is_none() && keys.role.is_none() {
                continue;
            }
            let layer_on = keys.sheet_layer.is_none_or(|layer| self.layer_is_on(layer));
            let role_on = keys.role.is_none_or(|role| self.role_is_on(role));
            let invisible = !(layer_on && role_on);
            let common = entity.common_mut();
            if common.invisible != invisible {
                common.invisible = invisible;
                changed += 1;
            }
        }
        changed
    }

    pub fn layer_is_on(&self, layer: &str) -> bool {
        !self.layers_off.contains(layer)
    }

    pub fn role_is_on(&self, role: &str) -> bool {
        !self.roles_off.contains(role)
    }

    pub fn set_layer(&mut self, layer: &str, on: bool) {
        if on {
            self.layers_off.remove(layer);
        } else {
            self.layers_off.insert(layer.to_string());
        }
    }

    pub fn set_role(&mut self, role: &str, on: bool) {
        if on {
            self.roles_off.remove(role);
        } else {
            self.roles_off.insert(role.to_string());
        }
    }

    /// The sheet layers switched off, sorted.
    pub fn layers_off(&self) -> impl Iterator<Item = &str> {
        self.layers_off.iter().map(String::as_str)
    }

    /// The roles switched off, sorted.
    pub fn roles_off(&self) -> impl Iterator<Item = &str> {
        self.roles_off.iter().map(String::as_str)
    }

    /// Nothing is switched off.
    pub fn is_empty(&self) -> bool {
        self.layers_off.is_empty() && self.roles_off.is_empty()
    }
}

/// The two keys of an entity's `PID_SEMANTICS` record the filter reads.
struct PidKeys<'a> {
    sheet_layer: Option<&'a str>,
    role: Option<&'a str>,
}

/// `None` when the entity carries no `PID_SEMANTICS` record at all.
fn pid_keys(entity: &EntityType) -> Option<PidKeys<'_>> {
    let record = entity
        .common()
        .extended_data
        .get_record(PID_SEMANTICS_XDATA_APP)?;
    let mut keys = PidKeys {
        sheet_layer: None,
        role: None,
    };
    for value in &record.values {
        let XDataValue::String(text) = value else {
            continue;
        };
        if let Some(layer) = text.strip_prefix("sheet_layer=") {
            keys.sheet_layer.get_or_insert(layer);
        } else if let Some(role) = text.strip_prefix("role=") {
            keys.role.get_or_insert(role);
        }
    }
    Some(keys)
}

/// The record's owner: the model-space block record, the one object every
/// drawing has exactly one of.
fn owner(doc: &CadDocument) -> Handle {
    doc.header.model_space_block_handle
}

/// Drop the filter's XRecord from `owner`'s extension dictionary, if present.
/// The dictionary itself stays: other records may live there, and an empty
/// one is harmless bookkeeping.
fn remove_xrecord(doc: &mut CadDocument, owner: Handle) {
    let Some(dictionary_handle) = doc.extension_dictionary_handle(owner) else {
        return;
    };
    let Some(acadrust::objects::ObjectType::Dictionary(dictionary)) =
        doc.objects.get_mut(&dictionary_handle)
    else {
        return;
    };
    let Some(index) = dictionary
        .entries
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(XRECORD_KEY))
    else {
        return;
    };
    let (_, record_handle) = dictionary.entries.remove(index);
    doc.objects.remove(&record_handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use acadrust::entities::Line;
    use acadrust::types::Vector3;
    use acadrust::xdata::ExtendedDataRecord;

    #[test]
    fn hidden_sheet_layer_names_are_trimmed_case_folded_and_space_insensitive() {
        for name in [
            "Hidden",
            " hidden ",
            "HIDDENOBJECTS",
            "Hidden Objects",
            "Hidden\t  Objects",
            "Invisible",
        ] {
            assert!(is_hidden_sheet_layer(name), "{name:?}");
        }
        assert!(!is_hidden_sheet_layer("Visible"));
        assert!(!is_hidden_sheet_layer("Labels"));
    }

    fn line_with(pairs: &[&str]) -> EntityType {
        let mut line = EntityType::Line(Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
        ));
        if !pairs.is_empty() {
            let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
            for pair in pairs {
                record.add_value(XDataValue::String(pair.to_string()));
            }
            line.common_mut().extended_data.add_record(record);
        }
        line
    }

    fn document_with(entities: Vec<EntityType>) -> CadDocument {
        let mut doc = CadDocument::new();
        for entity in entities {
            let _ = doc.add_entity(entity);
        }
        doc
    }

    #[test]
    fn a_stored_filter_reads_back_and_an_empty_one_leaves_no_record() {
        let mut doc = document_with(Vec::new());
        assert_eq!(PidViewFilter::load(&doc), None);

        let mut filter = PidViewFilter::default();
        filter.set_layer("Labels", false);
        filter.set_layer("Hidden Objects", false);
        filter.set_role("text", false);
        filter.store(&mut doc);
        assert_eq!(PidViewFilter::load(&doc).as_ref(), Some(&filter));
        assert_eq!(
            filter.layers_off().collect::<Vec<_>>(),
            ["Hidden Objects", "Labels"]
        );

        filter.set_layer("Labels", true);
        filter.store(&mut doc);
        assert_eq!(
            PidViewFilter::load(&doc).as_ref(),
            Some(&filter),
            "a re-store replaces the record rather than appending to it"
        );

        filter.set_layer("Hidden Objects", true);
        filter.set_role("text", true);
        assert!(filter.is_empty());
        filter.store(&mut doc);
        assert_eq!(PidViewFilter::load(&doc), None);
        assert!(doc.xrecord(owner(&doc), XRECORD_KEY).is_none());
    }

    #[test]
    fn apply_darkens_by_layer_or_role_and_leaves_foreign_records_alone() {
        let mut doc = document_with(vec![
            line_with(&["sheet_layer=Labels", "role=text"]),
            line_with(&["sheet_layer=Default", "role=text"]),
            line_with(&["sheet_layer=Default", "role=geometry"]),
            line_with(&["role=frame"]),
            line_with(&["class=蝶阀", "resolved=legend:block"]),
            line_with(&[]),
        ]);
        let dark = |doc: &CadDocument| -> Vec<bool> {
            doc.entities().map(|e| e.common().invisible).collect()
        };

        let mut filter = PidViewFilter::default();
        assert_eq!(filter.apply(&mut doc), 0);
        assert_eq!(dark(&doc), [false; 6]);

        filter.set_layer("Labels", false);
        assert_eq!(filter.apply(&mut doc), 1);
        assert_eq!(dark(&doc), [true, false, false, false, false, false]);

        filter.set_role("text", false);
        assert_eq!(
            filter.apply(&mut doc),
            1,
            "the Labels text was dark already"
        );
        assert_eq!(dark(&doc), [true, true, false, false, false, false]);

        filter.set_role("frame", false);
        assert_eq!(filter.apply(&mut doc), 1);
        assert_eq!(
            dark(&doc),
            [true, true, false, true, false, false],
            "a layerless entity answers to its role"
        );

        // A foreign `invisible` is neither set nor cleared.
        doc.entities_mut().nth(4).unwrap().common_mut().invisible = true;
        let all_on = PidViewFilter::default();
        assert_eq!(all_on.apply(&mut doc), 3);
        assert_eq!(dark(&doc), [false, false, false, false, true, false]);
    }

    #[test]
    fn the_initial_filter_switches_off_only_the_hidden_sheet_layers_present() {
        let doc = document_with(vec![
            line_with(&["sheet_layer=Labels", "role=text"]),
            line_with(&["sheet_layer=HiddenObjects", "role=geometry"]),
            line_with(&["sheet_layer=Hidden Objects", "role=geometry"]),
            line_with(&["role=frame"]),
        ]);
        let filter = PidViewFilter::initial(&doc);
        assert_eq!(
            filter.layers_off().collect::<Vec<_>>(),
            ["Hidden Objects", "HiddenObjects"],
            "each spelling the drawing uses is its own switch"
        );
        assert_eq!(filter.roles_off().count(), 0);
        assert!(PidViewFilter::initial(&document_with(Vec::new())).is_empty());
    }
}

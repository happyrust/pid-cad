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
//!
//! The layer manager's sheet-layer view is a reading of the same two axes:
//! [`PidViewSummary`] lists what the drawing's entities state along each,
//! with the filter's answer for every name, and [`switch_sheet_layer`] /
//! [`switch_role`] are the one call behind each of its switches.

use std::collections::{BTreeMap, BTreeSet};

use acadrust::objects::{XRecordEntry, XRecordValue};
use acadrust::xdata::XDataValue;
use acadrust::{CadDocument, EntityType, Handle};

use super::pid::LAYER_HIDDEN;
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

/// The import's role vocabulary, in the order the layer manager lists it:
/// what the sheet draws first, then the review marks, then what the importer
/// adds around the drawing. A role outside it -- there is none today -- would
/// list after these, alphabetically.
pub const ROLE_ORDER: [&str; 12] = [
    "geometry",
    "text",
    "symbol",
    "symbol-label",
    "point-ok",
    "point-warning",
    "point-error",
    "point-approved",
    "annotation",
    "connectivity",
    "fill",
    "frame",
];

/// One row of the layer manager's sheet-layer view: an authored sheet layer
/// or an import role, how many entities state it, and whether the filter has
/// it on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidViewRow {
    pub name: String,
    pub entities: usize,
    pub on: bool,
}

/// What a drawing's entities state along the filter's two axes, with the
/// filter's answer for each name.
///
/// Sheet layers are one row per distinct name, sorted. SmartPlant keeps one
/// layer object per view filter set and per storage, so the same name can be
/// several objects in the file; the filter switches names, the importer writes
/// names, and a reader thinks in names, so that is what is listed. Roles are
/// in [`ROLE_ORDER`]. Empty for a drawing without a `.pid` import, which is
/// how the layer manager knows not to offer the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PidViewSummary {
    pub layers: Vec<PidViewRow>,
    pub roles: Vec<PidViewRow>,
}

impl PidViewSummary {
    /// Read the summary off the drawing: its entities' `sheet_layer=` and
    /// `role=` keys, and the filter it stores (none stored reads as all on).
    pub fn of(doc: &CadDocument) -> Self {
        let filter = PidViewFilter::load(doc).unwrap_or_default();
        let mut layers: BTreeMap<&str, usize> = BTreeMap::new();
        let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
        for entity in doc.entities() {
            let Some(keys) = pid_keys(entity) else {
                continue;
            };
            if let Some(layer) = keys.sheet_layer {
                *layers.entry(layer).or_default() += 1;
            }
            if let Some(role) = keys.role {
                *roles.entry(role).or_default() += 1;
            }
        }
        let mut roles: Vec<PidViewRow> = roles
            .into_iter()
            .map(|(name, entities)| PidViewRow {
                on: filter.role_is_on(name),
                name: name.to_string(),
                entities,
            })
            .collect();
        // Stable, so roles outside the vocabulary keep their alphabetical order
        // after the known ones.
        roles.sort_by_key(|row| role_rank(&row.name));
        Self {
            layers: layers
                .into_iter()
                .map(|(name, entities)| PidViewRow {
                    on: filter.layer_is_on(name),
                    name: name.to_string(),
                    entities,
                })
                .collect(),
            roles,
        }
    }

    /// No entity states a sheet layer or a role: nothing to list.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.roles.is_empty()
    }
}

fn role_rank(role: &str) -> usize {
    ROLE_ORDER
        .iter()
        .position(|known| *known == role)
        .unwrap_or(ROLE_ORDER.len())
}

/// What one switch did to the drawing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Switched {
    /// Entities whose `invisible` bit changed.
    pub entities: usize,
    /// The `PID-HIDDEN` layer was turned on so the entities the import filed
    /// there could show. Only switching a sheet layer on ever does this.
    pub released_hidden_layer: bool,
}

/// Switch one authored sheet layer on or off: the stored record follows and
/// every entity's bit is set from the result.
///
/// Switching a layer *on* may have to release `PID-HIDDEN` as well: the import
/// files the entities of the sheet layers it starts off on that layer, with the
/// layer itself off, so their bit alone would not show them. That layer is the
/// importer's own hiding device and the bit now carries the same reading, so
/// the switch turns it on when an entity it just lit sits there. Switching off
/// never touches the layer table; the bits do the hiding.
pub fn switch_sheet_layer(doc: &mut CadDocument, layer: &str, on: bool) -> Switched {
    let mut filter = PidViewFilter::load(doc).unwrap_or_default();
    filter.set_layer(layer, on);
    let mut switched = Switched {
        entities: store_and_apply(doc, &filter),
        released_hidden_layer: false,
    };
    if on {
        switched.released_hidden_layer = release_hidden_layer(doc);
    }
    switched
}

/// Switch one import role on or off; see [`switch_sheet_layer`]. A role
/// switch lights nothing the import hid, so the layer table is never touched.
pub fn switch_role(doc: &mut CadDocument, role: &str, on: bool) -> Switched {
    let mut filter = PidViewFilter::load(doc).unwrap_or_default();
    filter.set_role(role, on);
    Switched {
        entities: store_and_apply(doc, &filter),
        released_hidden_layer: false,
    }
}

fn store_and_apply(doc: &mut CadDocument, filter: &PidViewFilter) -> usize {
    filter.store(doc);
    filter.apply(doc)
}

/// Turn `PID-HIDDEN` on when it is off and holds an entity whose bit says it
/// should draw. Returns whether the layer changed.
fn release_hidden_layer(doc: &mut CadDocument) -> bool {
    let holds_a_lit_entity = doc
        .entities()
        .any(|entity| entity.common().layer == LAYER_HIDDEN && !entity.common().invisible);
    if !holds_a_lit_entity {
        return false;
    }
    match doc.layers.get_mut(LAYER_HIDDEN) {
        Some(hidden) if hidden.flags.off => {
            hidden.flags.off = false;
            true
        }
        _ => false,
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

    #[test]
    fn the_summary_counts_by_name_orders_roles_by_vocabulary_and_reads_the_filter() {
        let mut doc = document_with(vec![
            line_with(&["sheet_layer=Labels", "role=text"]),
            line_with(&["sheet_layer=Labels", "role=geometry"]),
            line_with(&["sheet_layer=Default", "role=geometry"]),
            line_with(&["sheet_layer=HiddenObjects", "role=geometry"]),
            line_with(&["role=frame"]),
            line_with(&["role=connectivity"]),
            line_with(&["role=zzz-unknown"]),
            line_with(&["class=蝶阀", "resolved=legend:block"]),
            line_with(&[]),
        ]);
        assert!(PidViewSummary::of(&document_with(Vec::new())).is_empty());

        let summary = PidViewSummary::of(&doc);
        let rows = |rows: &[PidViewRow]| -> Vec<(String, usize, bool)> {
            rows.iter()
                .map(|row| (row.name.clone(), row.entities, row.on))
                .collect()
        };
        assert_eq!(
            rows(&summary.layers),
            [
                ("Default".to_string(), 1, true),
                ("HiddenObjects".to_string(), 1, true),
                ("Labels".to_string(), 2, true),
            ],
            "no record stored reads as everything on"
        );
        assert_eq!(
            rows(&summary.roles),
            [
                ("geometry".to_string(), 3, true),
                ("text".to_string(), 1, true),
                ("connectivity".to_string(), 1, true),
                ("frame".to_string(), 1, true),
                ("zzz-unknown".to_string(), 1, true),
            ]
        );

        let mut filter = PidViewFilter::default();
        filter.set_layer("HiddenObjects", false);
        filter.set_role("frame", false);
        filter.store(&mut doc);
        let summary = PidViewSummary::of(&doc);
        assert!(
            !summary
                .layers
                .iter()
                .find(|row| row.name == "HiddenObjects")
                .unwrap()
                .on
        );
        assert!(
            summary
                .layers
                .iter()
                .find(|row| row.name == "Labels")
                .unwrap()
                .on
        );
        assert!(
            !summary
                .roles
                .iter()
                .find(|row| row.name == "frame")
                .unwrap()
                .on
        );
    }

    #[test]
    fn a_switch_stores_applies_and_releases_the_hidden_layer_only_when_lighting_it() {
        let mut doc = document_with(vec![
            line_with(&["sheet_layer=Labels", "role=text"]),
            line_with(&["sheet_layer=HiddenObjects", "role=geometry"]),
            line_with(&["role=frame"]),
        ]);
        let mut hidden = acadrust::tables::layer::Layer::new(LAYER_HIDDEN);
        hidden.flags.off = true;
        doc.layers
            .add(hidden)
            .expect("a fresh document has no PID-HIDDEN");
        doc.entities_mut().nth(1).unwrap().common_mut().layer = LAYER_HIDDEN.to_string();
        let initial = PidViewFilter::initial(&doc);
        initial.store(&mut doc);
        initial.apply(&mut doc);
        let dark = |doc: &CadDocument| -> Vec<bool> {
            doc.entities().map(|e| e.common().invisible).collect()
        };
        assert_eq!(dark(&doc), [false, true, false]);

        let switched = switch_sheet_layer(&mut doc, "Labels", false);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_hidden_layer: false
            }
        );
        assert_eq!(dark(&doc), [true, true, false]);
        assert_eq!(
            PidViewFilter::load(&doc)
                .unwrap()
                .layers_off()
                .collect::<Vec<_>>(),
            ["HiddenObjects", "Labels"]
        );
        assert!(doc.layers.get(LAYER_HIDDEN).unwrap().flags.off);

        let switched = switch_sheet_layer(&mut doc, "Labels", true);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_hidden_layer: false
            },
            "nothing lit sits on PID-HIDDEN, so the layer stays as it was"
        );
        assert!(doc.layers.get(LAYER_HIDDEN).unwrap().flags.off);

        let switched = switch_sheet_layer(&mut doc, "HiddenObjects", true);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_hidden_layer: true
            }
        );
        assert_eq!(dark(&doc), [false, false, false]);
        assert!(!doc.layers.get(LAYER_HIDDEN).unwrap().flags.off);
        assert!(
            PidViewFilter::load(&doc).is_none(),
            "all on leaves no record"
        );

        let switched = switch_sheet_layer(&mut doc, "HiddenObjects", false);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_hidden_layer: false
            }
        );
        assert!(
            !doc.layers.get(LAYER_HIDDEN).unwrap().flags.off,
            "off never touches the table"
        );

        let switched = switch_role(&mut doc, "frame", false);
        assert_eq!(
            switched,
            Switched {
                entities: 1,
                released_hidden_layer: false
            }
        );
        assert_eq!(dark(&doc), [false, true, true]);
        let switched = switch_role(&mut doc, "frame", false);
        assert_eq!(
            switched.entities, 0,
            "switching what is already off changes nothing"
        );
    }
}

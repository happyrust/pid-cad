// The plant items a drawing states for its own graphics: what each placed symbol stands for, read off the file with nothing published beside it.

use pid_parse::publish::pid_load::{
    load_publish_from_pid, PidLoadError, PidPublishItem, PidPublishRepresentation,
};

use super::*;

/// One plant item as the drawing file states it: what a placed symbol
/// stands for.
///
/// SmartPlant keeps a symbol's type and tag in its database, not on the
/// graphic, and the drawing file carries a copy of the item behind each
/// graphic: one `P&IDAttributes` row an item, and the graphic that draws
/// it. `pid-parse` reads that copy (`load_publish_from_pid`, the reader the
/// backup panel's item tree uses). Measured on the 53 sheets of the plant
/// test data, it names the same item as the database for every placement
/// the database has one for -- 9,218 of 9,225 (plan
/// 2026-11-20-pid-symbol-identity, SPEC section 2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SheetItem {
    /// The item's id (`ModelID`), spelt as the file spells it.
    pub(super) uid: String,
    /// What kind of item SmartPlant holds it as: `PipingComp`, `Instrument`,
    /// `Vessel`, ...
    pub(super) item_type: String,
    /// The class the item is published as -- `PIDPipingComponent`,
    /// `PIDInstrument`, ... -- where the file decides one. An off-page
    /// connector has none.
    pub(super) class: Option<&'static str>,
    /// The name the file gives the item (`IObject.Name`), where it gives
    /// one; never blank. For most items that have one it is the tag; but
    /// the file names fewer items than the plant database tags, and a few
    /// differently (SPEC section 2.2), so a tag taken from here says so
    /// (`tag_source=file`).
    pub(super) name: Option<String>,
}

impl SheetItem {
    /// The item as its row of the file states it. Its name is the
    /// `IObject.Name` attribute less the blanks around it, and a blank name
    /// is no name.
    fn of(item: &PidPublishItem) -> Self {
        let name = item
            .attributes
            .iter()
            .find(|attribute| attribute.interface == "IObject" && attribute.name == "Name")
            .map(|attribute| attribute.value.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        Self {
            uid: item.uid.clone(),
            item_type: item.model_item_type.clone(),
            class: item.class,
            name,
        }
    }
}

/// The plant items of one sheet, by the graphic that draws each.
#[derive(Debug, Default)]
pub(super) struct SheetItems {
    by_graphic: HashMap<u32, SheetItem>,
}

impl SheetItems {
    /// The items the `.pid` at `path` states for its graphics. A file the
    /// reader gives nothing for -- one that states no drawing number, one
    /// that will not open a second time -- is still a drawing: it imports
    /// with no item on any symbol, and the log says why.
    pub(super) fn read(path: &Path) -> Self {
        match load_publish_from_pid(path) {
            Ok(drawing) => Self::of(&drawing.items, &drawing.representations),
            Err(error) => {
                let why = match &error {
                    PidLoadError::NoDrawing => "it states no drawing number".to_string(),
                    PidLoadError::Io(error) => error.to_string(),
                };
                log::info!(
                    "{}: the drawing's own plant items were not read ({why}); its symbols state no type and no tag",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// The table of what the file states: each representation's graphic
    /// against the item it draws. A representation that names no item -- a
    /// branch point drawn on a run -- or an item the file has no row for is
    /// left out. The first representation of a graphic stands.
    pub(super) fn of(
        items: &[PidPublishItem],
        representations: &[PidPublishRepresentation],
    ) -> Self {
        let stated: HashMap<&str, &PidPublishItem> =
            items.iter().map(|item| (item.uid.as_str(), item)).collect();
        let mut by_graphic = HashMap::new();
        for representation in representations {
            let Some(item) = representation
                .item_uid
                .as_deref()
                .and_then(|uid| stated.get(uid))
            else {
                continue;
            };
            by_graphic
                .entry(representation.graphic_oid)
                .or_insert_with(|| SheetItem::of(item));
        }
        Self { by_graphic }
    }

    /// The item the graphic `graphic_oid` draws, where the file states one.
    pub(super) fn item_of(&self, graphic_oid: u32) -> Option<&SheetItem> {
        self.by_graphic.get(&graphic_oid)
    }

    /// How many graphics the table names an item for.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.by_graphic.len()
    }

    /// A table that states each item of `stated` for the graphic beside it:
    /// what the tests of the entity loop hand it for a file's reading.
    #[cfg(test)]
    pub(super) fn stating(stated: impl IntoIterator<Item = (u32, SheetItem)>) -> Self {
        Self {
            by_graphic: stated.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pid_parse::publish::pid_load::{PidPublishAttribute, PidValueSource};

    use super::*;

    fn attribute(interface: &'static str, name: &'static str, value: &str) -> PidPublishAttribute {
        PidPublishAttribute {
            interface,
            name,
            value: value.to_string(),
            source: PidValueSource::ClassConstant,
        }
    }

    fn item(
        uid: &str,
        item_type: &str,
        class: Option<&'static str>,
        attributes: Vec<PidPublishAttribute>,
    ) -> PidPublishItem {
        PidPublishItem {
            uid: uid.to_string(),
            model_item_type: item_type.to_string(),
            class,
            attributes,
        }
    }

    fn drawn_by(uid: &str, item_uid: Option<&str>, graphic_oid: u32) -> PidPublishRepresentation {
        PidPublishRepresentation {
            uid: uid.to_string(),
            item_uid: item_uid.map(str::to_string),
            graphic_oid,
        }
    }

    /// A graphic is joined to the item its representation names: the item's
    /// id and type as the file spells them, its published class where the
    /// file decides one, and its `IObject.Name` -- trimmed, and none where
    /// it is blank or another interface's `Name`. A representation that
    /// names no item, or one the file has no row for, joins nothing; of two
    /// representations of one graphic the first stands.
    #[test]
    fn a_graphic_is_joined_to_the_item_its_representation_names() {
        let items = [
            item(
                "VALVE-1",
                "PipingComp",
                Some("PIDPipingComponent"),
                vec![
                    attribute("IPipingComponent", "Name", "not the item's name"),
                    attribute("IObject", "Description", "a gate valve"),
                    attribute("IObject", "Name", "  GV-101 "),
                ],
            ),
            item(
                "OPC-1",
                "OPC",
                None,
                vec![attribute("IObject", "Name", "   ")],
            ),
            item("GAUGE-1", "Instrument", Some("PIDInstrument"), Vec::new()),
        ];
        let representations = [
            drawn_by("R1", Some("VALVE-1"), 612),
            drawn_by("R2", Some("OPC-1"), 613),
            drawn_by("R3", Some("GAUGE-1"), 614),
            drawn_by("R4", None, 615),
            drawn_by("R5", Some("NO-SUCH-ITEM"), 616),
            drawn_by("R6", Some("GAUGE-1"), 612),
            drawn_by("R7", Some("VALVE-1"), 617),
        ];
        let table = SheetItems::of(&items, &representations);

        assert_eq!(
            table.item_of(612),
            Some(&SheetItem {
                uid: "VALVE-1".to_string(),
                item_type: "PipingComp".to_string(),
                class: Some("PIDPipingComponent"),
                name: Some("GV-101".to_string()),
            }),
            "the first representation of the graphic, its name trimmed"
        );
        assert_eq!(
            table.item_of(613),
            Some(&SheetItem {
                uid: "OPC-1".to_string(),
                item_type: "OPC".to_string(),
                class: None,
                name: None,
            }),
            "no class, and a blank name is no name"
        );
        assert_eq!(
            table
                .item_of(614)
                .map(|item| (item.uid.as_str(), item.name.as_deref())),
            Some(("GAUGE-1", None)),
            "an item without the attribute has no name"
        );
        assert_eq!(table.item_of(615), None, "a representation of no item");
        assert_eq!(table.item_of(616), None, "an item the file has no row for");
        assert_eq!(
            table.item_of(617).map(|item| item.uid.as_str()),
            Some("VALVE-1"),
            "an item drawn by two graphics answers for both"
        );
        assert_eq!(
            table.item_of(999),
            None,
            "a graphic the file says nothing of"
        );
        assert_eq!(table.len(), 4);
    }

    /// A file the item reader gives nothing for yields an empty table, not
    /// a failure: the drawing still imports.
    #[test]
    fn a_file_the_reader_gives_nothing_for_yields_no_items() {
        let nowhere = std::env::temp_dir().join("ocs-no-such-drawing-7f3a91.pid");
        assert!(!nowhere.exists());
        let table = SheetItems::read(&nowhere);
        assert_eq!(table.len(), 0);
        assert_eq!(table.item_of(612), None);
    }
}

//! What an imported `.pid` says its symbols and their labels are, read back
//! off the drawing (plan 2026-11-20-pid-symbol-identity, SPEC sections 3 and
//! 4.5 to 4.7).
//!
//! A `.pid` import states on each symbol's block reference what the symbol
//! is -- the number of its graphic, the plant item it draws, its tag and
//! where the tag came from -- and on each loose text the graphic its label
//! hangs on (`crate::io::pid`). Those are `key=value` strings of the
//! entity's `PID_SEMANTICS` record, and a save as DWG or DXF keeps them.
//! This module reads them back, and answers the two questions that take the
//! sheet around an entity: which texts label a symbol, and which symbol a
//! text labels.
//!
//! It reads and changes nothing, and it is compiled for every target: a
//! drawing saved from an import states these whether or not the `.pid`
//! reader is there to import another.
//!
//! A symbol and its labels find each other by the number of the symbol's
//! graphic alone. A copy of a symbol states the number its original states,
//! so the two share their labels; and a number is one graphic's only within
//! the sheet of the file it was read from. Nothing here makes it more.

use document::xdata::XDataValue;
use document::{Document, EntityType};

use super::PID_SEMANTICS_XDATA_APP;

/// The `key=value` strings of an entity's `PID_SEMANTICS` record, in the
/// order the record states them. What the record holds that is no such
/// string is passed over.
fn pairs(entity: &EntityType) -> impl Iterator<Item = (&str, &str)> {
    entity
        .common()
        .extended_data
        .get_record(PID_SEMANTICS_XDATA_APP)
        .into_iter()
        .flat_map(|record| record.values.iter())
        .filter_map(|value| match value {
            XDataValue::String(text) => text.split_once('='),
            _ => None,
        })
}

/// The value an entity's `PID_SEMANTICS` record states for `key`: the first,
/// where the record states the key more than once.
pub fn stated<'a>(entity: &'a EntityType, key: &str) -> Option<&'a str> {
    pairs(entity)
        .find(|(stated, _)| *stated == key)
        .map(|(_, value)| value)
}

/// What an entity states for `key`, an empty value being nothing stated.
fn state<'a>(entity: &'a EntityType, key: &str) -> Option<&'a str> {
    stated(entity, key).filter(|value| !value.is_empty())
}

/// Whether an entity is the block reference of an imported symbol.
fn is_symbol(entity: &EntityType) -> bool {
    matches!(entity, EntityType::Insert(_)) && stated(entity, "role") == Some("symbol")
}

/// Whether an entity is the block reference of an imported symbol whose
/// graphic is numbered `graphic`.
fn is_numbered(entity: &EntityType, graphic: &str) -> bool {
    is_symbol(entity) && state(entity, "graphic") == Some(graphic)
}

/// Where a symbol's tag came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSource<'a> {
    /// The drawing file's own name for the item (`tag_source=file`).
    File,
    /// The plant database of a backup (`tag_source=database`).
    Database,
    /// The `_Data.xml` published beside the drawing: a tag the record marks
    /// with no `tag_source`.
    Published,
    /// A source this version does not know, as the record spells it.
    Other(&'a str),
}

/// What the block reference of an imported symbol says the symbol is. A
/// field is `None` where the record does not state it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SymbolIdentity<'a> {
    /// The number of the symbol's graphic in the drawing file (`graphic`):
    /// what a label names its symbol by.
    pub graphic: Option<&'a str>,
    /// The plant item the graphic draws, as the file numbers it (`item`).
    pub item: Option<&'a str>,
    /// The kind SmartPlant holds the item as, such as `PipingComp`
    /// (`item_type`).
    pub item_type: Option<&'a str>,
    /// The class the item is published as, such as `PIDPipingComponent`
    /// (`class`).
    pub class: Option<&'a str>,
    /// The symbol's name, as the drawing spells it (`symbol`).
    pub symbol: Option<&'a str>,
    /// The item's tag (`label`).
    pub tag: Option<&'a str>,
    /// Where the tag came from; `None` where there is no tag.
    pub tag_source: Option<TagSource<'a>>,
    /// The drawing file's own name for the item, where the tag is another's
    /// and differs from it (`label_in_file`).
    pub name_in_file: Option<&'a str>,
}

impl<'a> SymbolIdentity<'a> {
    /// What kind of thing the symbol is: the class its item is published
    /// as, or else the kind SmartPlant holds the item as.
    pub fn kind(&self) -> Option<&'a str> {
        self.class.or(self.item_type)
    }
}

/// What the block reference of an imported symbol says the symbol is, or
/// `None` for anything else: an entity that is no block reference, or one
/// whose record does not say `role=symbol`. An empty value states nothing.
pub fn symbol_identity(entity: &EntityType) -> Option<SymbolIdentity<'_>> {
    if !is_symbol(entity) {
        return None;
    }
    let tag = state(entity, "label");
    let tag_source = tag.map(|_| match state(entity, "tag_source") {
        None => TagSource::Published,
        Some("file") => TagSource::File,
        Some("database") => TagSource::Database,
        Some(other) => TagSource::Other(other),
    });
    Some(SymbolIdentity {
        graphic: state(entity, "graphic"),
        item: state(entity, "item"),
        item_type: state(entity, "item_type"),
        class: state(entity, "class"),
        symbol: state(entity, "symbol"),
        tag,
        tag_source,
        name_in_file: state(entity, "label_in_file"),
    })
}

/// What a label hangs on, as its text states it (`label_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelOn {
    /// A symbol of the sheet, whose block reference states the number as
    /// its `graphic`.
    Symbol,
    /// A run of line.
    Line,
    /// Anything else, a value this version does not know included.
    Other,
}

/// What a text states of the label it is the lettering of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelPairing<'a> {
    /// The number of the graphic the label hangs on (`label_of`).
    pub graphic: &'a str,
    /// What that graphic is (`label_on`).
    pub on: LabelOn,
}

/// What an entity states of the label it is the lettering of, or `None`
/// where it does not state both the graphic and what the graphic is.
pub fn label_pairing(entity: &EntityType) -> Option<LabelPairing<'_>> {
    let graphic = state(entity, "label_of")?;
    let on = match stated(entity, "label_on")? {
        "symbol" => LabelOn::Symbol,
        "line" => LabelOn::Line,
        _ => LabelOn::Other,
    };
    Some(LabelPairing { graphic, on })
}

/// What a text reads on the sheet, or `None` for what is no text.
pub fn lettering(entity: &EntityType) -> Option<&str> {
    match entity {
        EntityType::Text(text) => Some(text.value.as_str()),
        EntityType::MText(text) => Some(text.value.as_str()),
        _ => None,
    }
}

/// The entities in model space that are the lettering of a label of the
/// symbol numbered `graphic`, in the order the sheet files them.
pub fn labels_of<'a>(document: &'a Document, graphic: &str) -> Vec<&'a EntityType> {
    document
        .model_space_entities()
        .filter(|entity| {
            label_pairing(entity)
                .is_some_and(|pairing| pairing.on == LabelOn::Symbol && pairing.graphic == graphic)
        })
        .collect()
}

/// The block references of symbols in model space that state `graphic` as
/// the number of their graphic, in the order the sheet files them: more
/// than one where a symbol was copied.
pub fn symbols_numbered<'a>(document: &'a Document, graphic: &str) -> Vec<&'a EntityType> {
    document
        .model_space_entities()
        .filter(|entity| is_numbered(entity, graphic))
        .collect()
}

/// What the label of a text hangs on, looked up on the sheet.
#[derive(Debug, Clone, Copy)]
pub enum LabelOwner<'a> {
    /// A symbol of the sheet: its block reference, the first the sheet
    /// files where several state the number.
    Symbol(&'a EntityType),
    /// A symbol, by what the text states, that is not on the sheet: no
    /// block reference in model space states the number.
    Missing,
    /// A run of line.
    Line,
    /// Anything else.
    Other,
}

/// What the label `text` is the lettering of hangs on, or `None` where
/// `text` states no pairing.
pub fn label_owner<'a>(document: &'a Document, text: &EntityType) -> Option<LabelOwner<'a>> {
    let pairing = label_pairing(text)?;
    Some(match pairing.on {
        LabelOn::Symbol => document
            .model_space_entities()
            .find(|entity| is_numbered(entity, pairing.graphic))
            .map_or(LabelOwner::Missing, LabelOwner::Symbol),
        LabelOn::Line => LabelOwner::Line,
        LabelOn::Other => LabelOwner::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::entities::{Insert, Line, MText, Text};
    use document::tables::BlockRecord;
    use document::types::Vector3;
    use document::xdata::ExtendedDataRecord;
    use document::Handle;

    /// `entity` with `pairs` as its `PID_SEMANTICS` record -- with no record
    /// at all when there are none.
    fn saying(mut entity: EntityType, pairs: &[&str]) -> EntityType {
        if !pairs.is_empty() {
            let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
            for pair in pairs {
                record.add_value(XDataValue::String(pair.to_string()));
            }
            entity.common_mut().extended_data.add_record(record);
        }
        entity
    }

    /// A block reference that says `pairs`.
    fn reference(pairs: &[&str]) -> EntityType {
        saying(
            EntityType::Insert(Insert::new("SPPID_Ball Valve", Vector3::new(0.0, 0.0, 0.0))),
            pairs,
        )
    }

    /// A text that reads `value` and says `pairs`.
    fn text(value: &str, pairs: &[&str]) -> EntityType {
        let mut text = Text::new();
        text.value = value.to_string();
        saying(EntityType::Text(text), pairs)
    }

    /// A stroke that says `pairs`.
    fn line(pairs: &[&str]) -> EntityType {
        saying(
            EntityType::Line(Line::from_points(
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
            )),
            pairs,
        )
    }

    /// A sheet with `entities` in model space, and their handles in the
    /// order they were filed.
    fn sheet(entities: Vec<EntityType>) -> (Document, Vec<Handle>) {
        let mut doc = Document::new();
        let filed = entities
            .into_iter()
            .map(|entity| {
                doc.add_entity(entity)
                    .expect("model space takes the entity")
            })
            .collect();
        (doc, filed)
    }

    /// File `members` as the definition of the block `name`: a record of the
    /// block table owns them, where model space owns a loose entity.
    fn define(doc: &mut Document, name: &str, members: Vec<EntityType>) {
        let owner = doc.allocate_handle();
        let mut record = BlockRecord::new(name);
        record.handle = owner;
        doc.block_records
            .add(record)
            .expect("no block of that name yet");
        for mut member in members {
            member.common_mut().owner_handle = owner;
            doc.add_entity(member).expect("the block takes the member");
        }
    }

    fn handles(entities: &[&EntityType]) -> Vec<Handle> {
        entities
            .iter()
            .map(|entity| entity.common().handle)
            .collect()
    }

    /// A key is read whole -- `label` is not `label_of` -- and the first
    /// value stands where a record states a key twice. What a record holds
    /// that is no `key=value` string is passed over, and an entity with no
    /// record states nothing.
    #[test]
    fn a_key_reads_back_whole_and_first() {
        let said = line(&["label_of=612", "label=GV-101", "label=GV-102", "note=a=b"]);
        assert_eq!(stated(&said, "label"), Some("GV-101"));
        assert_eq!(stated(&said, "label_of"), Some("612"));
        assert_eq!(stated(&said, "label_o"), None);
        assert_eq!(stated(&said, "note"), Some("a=b"));
        assert_eq!(stated(&line(&[]), "label"), None);

        let mut mixed = EntityType::Insert(Insert::new("SPPID_Pump", Vector3::new(0.0, 0.0, 0.0)));
        let mut record = ExtendedDataRecord::new(PID_SEMANTICS_XDATA_APP);
        record.add_value(XDataValue::String("role=symbol".to_string()));
        record.add_value(XDataValue::Integer16(7));
        record.add_value(XDataValue::String("no pair".to_string()));
        record.add_value(XDataValue::String("graphic=9".to_string()));
        mixed.common_mut().extended_data.add_record(record);
        assert_eq!(stated(&mixed, "graphic"), Some("9"));
        assert_eq!(
            symbol_identity(&mixed),
            Some(SymbolIdentity {
                graphic: Some("9"),
                ..SymbolIdentity::default()
            })
        );
    }

    /// What an import states on a symbol's block reference (SPEC section 3)
    /// reads back field by field: a reference the drawing file names its
    /// item on, one the plant database tagged over the file's own name, and
    /// one the file states no item for, which says its number and its
    /// symbol's name and nothing else.
    #[test]
    fn a_symbol_reference_reads_back_what_it_states() {
        let valve = reference(&[
            "sheet_layer=Process",
            "sheet_layer_oid=12",
            "role=symbol",
            "style=Equipment - New",
            "extent=4.00x3.00",
            "class=PIDPipingComponent",
            "label=GV-101",
            "resolved=item",
            "symbol=Ball Valve",
            "graphic=612",
            "item=4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21",
            "item_type=PipingComp",
            "tag_source=file",
        ]);
        assert_eq!(
            symbol_identity(&valve),
            Some(SymbolIdentity {
                graphic: Some("612"),
                item: Some("4E1F0B7C9A2D4C6E8F1A3B5D7C9E0F21"),
                item_type: Some("PipingComp"),
                class: Some("PIDPipingComponent"),
                symbol: Some("Ball Valve"),
                tag: Some("GV-101"),
                tag_source: Some(TagSource::File),
                name_in_file: None,
            })
        );

        let pump = reference(&[
            "role=symbol",
            "label=P-1001A",
            "symbol=Pump",
            "graphic=9",
            "item=AB12",
            "item_type=Equipment",
            "tag_source=database",
            "label_in_file=P-1001",
        ]);
        assert_eq!(
            symbol_identity(&pump),
            Some(SymbolIdentity {
                graphic: Some("9"),
                item: Some("AB12"),
                item_type: Some("Equipment"),
                class: None,
                symbol: Some("Pump"),
                tag: Some("P-1001A"),
                tag_source: Some(TagSource::Database),
                name_in_file: Some("P-1001"),
            })
        );

        let bare = reference(&["role=symbol", "symbol=Ball Valve", "graphic=613"]);
        assert_eq!(
            symbol_identity(&bare),
            Some(SymbolIdentity {
                graphic: Some("613"),
                symbol: Some("Ball Valve"),
                ..SymbolIdentity::default()
            })
        );
    }

    /// The kind of a symbol is the class its item is published as, or else
    /// the kind SmartPlant holds the item as; a symbol that states neither
    /// has none.
    #[test]
    fn the_kind_of_a_symbol_is_its_class_or_else_its_item_type() {
        let both = reference(&[
            "role=symbol",
            "class=PIDPipingComponent",
            "item_type=PipingComp",
        ]);
        let unpublished = reference(&["role=symbol", "item_type=OPC"]);
        let neither = reference(&["role=symbol", "graphic=612"]);
        let kind = |entity: &EntityType| -> Option<String> {
            symbol_identity(entity)
                .expect("a symbol")
                .kind()
                .map(str::to_string)
        };
        assert_eq!(kind(&both).as_deref(), Some("PIDPipingComponent"));
        assert_eq!(kind(&unpublished).as_deref(), Some("OPC"));
        assert_eq!(kind(&neither), None);
    }

    /// A tag says where it came from (SPEC section 4.2): the drawing file,
    /// the plant database, or -- marked with nothing -- the published data
    /// beside the drawing. A mark this version does not know reads back as
    /// it is spelt. Where there is no tag there is no source to speak of,
    /// and an empty tag is no tag.
    #[test]
    fn a_tag_says_where_it_came_from() {
        let source = |pairs: &[&str]| -> String {
            let symbol = reference(pairs);
            format!(
                "{:?}",
                symbol_identity(&symbol).expect("a symbol").tag_source
            )
        };
        assert_eq!(
            source(&["role=symbol", "label=GV-101", "tag_source=file"]),
            "Some(File)"
        );
        assert_eq!(
            source(&["role=symbol", "label=GV-101", "tag_source=database"]),
            "Some(Database)"
        );
        assert_eq!(source(&["role=symbol", "label=GV-101"]), "Some(Published)");
        assert_eq!(
            source(&["role=symbol", "label=GV-101", "tag_source=manual"]),
            "Some(Other(\"manual\"))"
        );
        assert_eq!(source(&["role=symbol", "tag_source=file"]), "None");
        assert_eq!(
            source(&["role=symbol", "label=", "tag_source=file"]),
            "None"
        );
    }

    /// Only the block reference of an imported symbol is one: a reference
    /// that says nothing, one legend recognition tagged, one that plays
    /// another role, and a loose stroke that says `symbol` -- what an import
    /// drew before symbols were filed as blocks -- are not.
    #[test]
    fn only_a_block_reference_that_says_symbol_is_a_symbol() {
        assert!(symbol_identity(&reference(&["role=symbol"])).is_some());
        assert!(symbol_identity(&reference(&[])).is_none());
        assert!(symbol_identity(&reference(&[
            "class=PIDPipingComponent",
            "label=GV-101",
            "resolved=legend:block",
        ]))
        .is_none());
        assert!(symbol_identity(&reference(&["role=geometry", "graphic=612"])).is_none());
        assert!(symbol_identity(&line(&["role=symbol", "symbol=Ball Valve"])).is_none());
    }

    /// A text reads back what its label hangs on (SPEC section 3): the
    /// number of the graphic, and what the graphic is. A value of
    /// `label_on` this version does not know is anything else; a text that
    /// states only one of the two keys, or names no graphic, states no
    /// pairing.
    #[test]
    fn a_label_text_reads_back_what_its_label_hangs_on() {
        let pairing = |pairs: &[&str]| -> Option<(String, LabelOn)> {
            let lettered = text("GV-101", pairs);
            label_pairing(&lettered).map(|pairing| (pairing.graphic.to_string(), pairing.on))
        };
        let hung = |graphic: &str, on: LabelOn| Some((graphic.to_string(), on));
        assert_eq!(
            pairing(&["role=text", "label_of=612", "label_on=symbol"]),
            hung("612", LabelOn::Symbol)
        );
        assert_eq!(
            pairing(&["role=text", "label_of=704", "label_on=line"]),
            hung("704", LabelOn::Line)
        );
        assert_eq!(
            pairing(&["role=text", "label_of=6", "label_on=other"]),
            hung("6", LabelOn::Other)
        );
        assert_eq!(
            pairing(&["role=text", "label_of=6", "label_on=somewhere-new"]),
            hung("6", LabelOn::Other)
        );
        assert_eq!(pairing(&["role=text"]), None);
        assert_eq!(pairing(&["role=text", "label_of=612"]), None);
        assert_eq!(pairing(&["role=text", "label_on=symbol"]), None);
        assert_eq!(
            pairing(&["role=text", "label_of=", "label_on=symbol"]),
            None
        );
        assert_eq!(pairing(&[]), None);
    }

    /// The lettering of a text is what it reads, whichever kind of text it
    /// is; a stroke has none.
    #[test]
    fn the_lettering_of_a_text_is_what_it_reads() {
        assert_eq!(lettering(&text("GV-101", &[])), Some("GV-101"));
        let note = MText {
            value: "DN50".to_string(),
            ..MText::default()
        };
        assert_eq!(lettering(&EntityType::MText(Box::new(note))), Some("DN50"));
        assert_eq!(lettering(&line(&[])), None);
    }

    /// The labels of a symbol are the texts in model space that name its
    /// graphic as a symbol's, in the order the sheet files them -- one filed
    /// before the symbol no less than one filed after it. A text that names
    /// the number as a line's is another graphic's label, a text inside a
    /// block's definition is not on the sheet, and a number is compared
    /// whole.
    #[test]
    fn the_labels_of_a_symbol_are_the_texts_that_name_its_graphic() {
        let (mut doc, filed) = sheet(vec![
            text("GV-101", &["role=text", "label_of=612", "label_on=symbol"]),
            reference(&["role=symbol", "symbol=Ball Valve", "graphic=612"]),
            text("DN50", &["role=text", "label_of=612", "label_on=symbol"]),
            reference(&["role=symbol", "symbol=Ball Valve", "graphic=613"]),
            text("FV-2", &["role=text", "label_of=613", "label_on=symbol"]),
            text("L-1001", &["role=text", "label_of=612", "label_on=line"]),
            text("NOTE 4", &["role=text"]),
        ]);
        define(
            &mut doc,
            "SPPID_Note",
            vec![text(
                "INSIDE",
                &["role=text", "label_of=612", "label_on=symbol"],
            )],
        );

        assert_eq!(handles(&labels_of(&doc, "612")), [filed[0], filed[2]]);
        assert_eq!(handles(&labels_of(&doc, "613")), [filed[4]]);
        assert!(labels_of(&doc, "614").is_empty());
        assert!(labels_of(&doc, "61").is_empty());
    }

    /// A label finds what it hangs on: the block reference that states its
    /// number -- the first the sheet files, where a copy states it too --
    /// or, where no reference states it any more, that the symbol is
    /// missing. A label on a line or on anything else is looked up no
    /// further, and a text that is no label's has no owner. A stroke that
    /// states a number is no symbol to hang a label on.
    #[test]
    fn a_label_finds_the_symbol_it_hangs_on() {
        let (doc, filed) = sheet(vec![
            text("GV-101", &["role=text", "label_of=612", "label_on=symbol"]),
            reference(&["role=symbol", "symbol=Ball Valve", "graphic=612"]),
            reference(&["role=symbol", "symbol=Ball Valve", "graphic=612"]),
            text("ERASED", &["role=text", "label_of=700", "label_on=symbol"]),
            text("L-1001", &["role=text", "label_of=612", "label_on=line"]),
            text("TITLE", &["role=text", "label_of=6", "label_on=other"]),
            text("NOTE 4", &["role=text"]),
            line(&["role=geometry", "graphic=700"]),
        ]);
        let owner = |at: usize| -> String {
            let lettered = doc.get_entity(filed[at]).expect("filed on the sheet");
            match label_owner(&doc, lettered) {
                Some(LabelOwner::Symbol(reference)) => {
                    let handle = reference.common().handle;
                    let position = filed.iter().position(|at| *at == handle);
                    format!("the symbol filed at {position:?}")
                }
                Some(LabelOwner::Missing) => "a missing symbol".to_string(),
                Some(LabelOwner::Line) => "a line".to_string(),
                Some(LabelOwner::Other) => "another object".to_string(),
                None => "no label".to_string(),
            }
        };
        assert_eq!(owner(0), "the symbol filed at Some(1)");
        assert_eq!(owner(3), "a missing symbol");
        assert_eq!(owner(4), "a line");
        assert_eq!(owner(5), "another object");
        assert_eq!(owner(6), "no label");

        assert_eq!(
            handles(&symbols_numbered(&doc, "612")),
            [filed[1], filed[2]]
        );
        assert!(symbols_numbered(&doc, "700").is_empty());
        assert!(symbols_numbered(&doc, "61").is_empty());
    }
}

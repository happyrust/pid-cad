//! Line weights on the DWG save copy.
//!
//! A DWG states a line weight as a place in a fixed list of widths, so a
//! width that is not on the list has no place to be stated at and the codec
//! writes `LineWeight::Default` for it: read back, the stroke takes the
//! drawing's default width, not the one it was drawn with. A DXF states the
//! number itself and keeps any width.
//!
//! The Smart P&ID importer states its three pens as 0.17, 0.34 and 0.68 mm,
//! none of them on the list, so every stroke of a `.pid` saved as DWG lost
//! its pen. On the DWG save copy -- never on the drawing being edited, and
//! never for a DXF -- each such width moves to the nearest one on the list.

use std::cmp::Reverse;

use document::entities::EntityType;
use document::{Document, Handle, LineWeight};

/// The widths a DWG can state, in hundredths of a millimetre, narrowest
/// first: the list the pinned codec's `LineWeight::to_dwg_index` looks a
/// width up in. The codec keeps its list to itself; the test
/// `the_list_is_the_codecs` holds this copy to it.
const STANDARD_WIDTHS: [i16; 24] = [
    0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200,
    211,
];

/// The width a DWG states in place of `width`: the nearest on the list, the
/// wider of two equally near. `None` when `width` is on the list already, or
/// lies outside what the list spans: such a width stays as it is and is
/// written as the default -- the importer, too, leaves a pen the format
/// cannot state unstated rather than clamp it into a width nobody asked for.
fn nearest_standard(width: i16) -> Option<i16> {
    let narrowest = STANDARD_WIDTHS[0];
    let widest = STANDARD_WIDTHS[STANDARD_WIDTHS.len() - 1];
    if !(narrowest..=widest).contains(&width) || STANDARD_WIDTHS.contains(&width) {
        return None;
    }
    STANDARD_WIDTHS
        .iter()
        .copied()
        .min_by_key(|&standard| ((standard - width).abs(), Reverse(standard)))
}

/// [`nearest_standard`] for a line weight as an entity or a layer states it:
/// only a width given as a number can be off the list.
fn restated(weight: LineWeight) -> Option<LineWeight> {
    match weight {
        LineWeight::Value(width) => nearest_standard(width).map(LineWeight::Value),
        LineWeight::ByLayer | LineWeight::ByBlock | LineWeight::Default => None,
    }
}

/// Whether `entity` states a width a DWG cannot: its own, or that of an
/// attribute it carries -- the DWG writer states every attribute of a block
/// reference with the attribute's own width.
fn states_a_width_off_the_list(entity: &EntityType) -> bool {
    if restated(entity.common().line_weight).is_some() {
        return true;
    }
    match entity {
        EntityType::Insert(reference) => reference
            .attributes
            .iter()
            .any(|attribute| restated(attribute.common.line_weight).is_some()),
        _ => false,
    }
}

/// Whether [`snap_dwg_line_weights`] would change `doc`. A caller that only
/// borrows the drawing asks first, and copies it just when there is a width
/// to restate.
pub fn has_unstorable_dwg_line_weights(doc: &Document) -> bool {
    nearest_standard(doc.header.current_line_weight).is_some()
        || doc
            .layers
            .iter()
            .any(|layer| restated(layer.line_weight).is_some())
        || doc.entities().any(states_a_width_off_the_list)
}

/// Restate every width of `doc` that a DWG cannot state at the nearest one it
/// can: the header's current width, the layers', and the entities' -- those
/// inside the block definitions and the attributes of block references among
/// them. For the DWG save copy only: a DXF keeps the width as drawn, and so
/// does the drawing being edited. Returns whether anything changed.
pub fn snap_dwg_line_weights(doc: &mut Document) -> bool {
    let mut changed = false;
    if let Some(width) = nearest_standard(doc.header.current_line_weight) {
        doc.header.current_line_weight = width;
        changed = true;
    }
    for layer in doc.layers.iter_mut() {
        if let Some(weight) = restated(layer.line_weight) {
            layer.line_weight = weight;
            changed = true;
        }
    }
    // One entity at a time, by handle: `entities_mut` would drop the record
    // every entity was read with -- and with it whatever of a DWG the model
    // does not hold -- for the sake of the few that change.
    let handles: Vec<Handle> = doc
        .entities()
        .filter(|entity| states_a_width_off_the_list(entity))
        .map(|entity| entity.common().handle)
        .collect();
    for handle in handles {
        let Some(entity) = doc.get_entity_mut(handle) else {
            continue;
        };
        let common = entity.common_mut();
        if let Some(weight) = restated(common.line_weight) {
            common.line_weight = weight;
        }
        if let EntityType::Insert(reference) = entity {
            for attribute in &mut reference.attributes {
                if let Some(weight) = restated(attribute.common.line_weight) {
                    attribute.common.line_weight = weight;
                }
            }
        }
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Cursor;

    use document::entities::{AttributeEntity, Block, BlockEnd, Insert, Line};
    use document::tables::{BlockRecord, Layer};
    use document::types::{DxfVersion, Vector3};
    use document::{DwgReader, DxfReader};

    use super::*;
    use crate::serialize_document;

    /// The layer that states a width of its own.
    const PEN_LAYER: &str = "PENS";

    /// The definition the strokes at 34 and 68 are members of.
    const SYMBOL: &str = "SYMBOL";

    /// A vertical stroke at `x` stating `weight`. The tests tell the strokes
    /// apart by where they stand.
    fn stroke(x: f64, weight: LineWeight) -> EntityType {
        let mut line = Line::from_points(Vector3::new(x, 0.0, 0.0), Vector3::new(x, 10.0, 0.0));
        line.common.line_weight = weight;
        EntityType::Line(line)
    }

    /// File the definition `name` the way a drawn block is filed: its record,
    /// the BLOCK that opens it, `members` owned by the record, the ENDBLK
    /// that closes it.
    fn define_block(doc: &mut Document, name: &str, members: Vec<EntityType>) {
        let record_handle = doc.allocate_handle();
        let block_handle = doc.allocate_handle();
        let mut record = BlockRecord::new(name);
        record.handle = record_handle;
        record.block_entity_handle = block_handle;
        doc.block_records.add(record).unwrap();

        let mut block = Block::new(name, Vector3::ZERO);
        block.common.handle = block_handle;
        block.common.owner_handle = record_handle;
        doc.add_entity(EntityType::Block(block)).unwrap();

        for mut member in members {
            member.common_mut().owner_handle = record_handle;
            doc.add_entity(member).unwrap();
        }

        let end_handle = doc.allocate_handle();
        let mut end = BlockEnd::new();
        end.common.handle = end_handle;
        end.common.owner_handle = record_handle;
        doc.add_entity(EntityType::BlockEnd(end)).unwrap();
        doc.block_records.get_mut(name).unwrap().block_end_handle = end_handle;
    }

    /// A drawing that states three pens -- fine, medium and bold, in
    /// hundredths of a millimetre -- in every place a DWG states a width: the
    /// fine one on a stroke in model space and on an attribute of a block
    /// reference, the medium one on a stroke inside the block's definition
    /// and as the header's current width, the bold one on a second stroke of
    /// the definition and on a layer. Beside them, what a DWG states as it
    /// stands whatever the pens: a standard width and the three widths taken
    /// by reference.
    fn drawing_with(pens: [i16; 3]) -> Document {
        let [fine, medium, bold] = pens.map(LineWeight::Value);
        let mut doc = Document::new();
        doc.version = DxfVersion::AC1032;
        let mut layer = Layer::new(PEN_LAYER);
        layer.handle = doc.allocate_handle();
        layer.line_weight = bold;
        doc.layers.add(layer).unwrap();
        doc.header.current_line_weight = pens[1];
        for (x, weight) in [
            (1.0, LineWeight::ByLayer),
            (2.0, LineWeight::ByBlock),
            (3.0, LineWeight::Default),
            (17.0, fine),
            (25.0, LineWeight::Value(25)),
        ] {
            doc.add_entity(stroke(x, weight)).unwrap();
        }
        let members = vec![stroke(34.0, medium), stroke(68.0, bold)];
        define_block(&mut doc, SYMBOL, members);
        let mut attribute = AttributeEntity::new("TAG".to_string(), "P-101".to_string());
        attribute.common.line_weight = fine;
        let mut reference = Insert::new(SYMBOL, Vector3::ZERO);
        reference.attributes.push(attribute);
        doc.add_entity(EntityType::Insert(reference)).unwrap();
        doc
    }

    /// The drawing with the pens of a Smart P&ID import: 0.17, 0.34 and
    /// 0.68 mm, none of them a width a DWG can state.
    fn drawing() -> Document {
        drawing_with([17, 34, 68])
    }

    /// The widths a drawing states, by where it states them.
    #[derive(Debug, PartialEq)]
    struct Stated {
        /// The strokes' widths by the x each stands at, those of model space
        /// and those of the definitions alike.
        strokes: BTreeMap<i64, LineWeight>,
        /// The width of the attribute on the reference.
        attribute: Option<LineWeight>,
        /// The width of [`PEN_LAYER`].
        layer: Option<LineWeight>,
        /// The header's current width.
        current: i16,
    }

    fn stated(doc: &Document) -> Stated {
        let mut strokes = BTreeMap::new();
        let mut attribute = None;
        for entity in doc.entities() {
            match entity {
                EntityType::Line(line) => {
                    strokes.insert(line.start.x.round() as i64, line.common.line_weight);
                }
                EntityType::Insert(reference) => {
                    attribute = reference
                        .attributes
                        .first()
                        .map(|attribute| attribute.common.line_weight);
                }
                _ => {}
            }
        }
        Stated {
            strokes,
            attribute,
            layer: doc.layers.get(PEN_LAYER).map(|layer| layer.line_weight),
            current: doc.header.current_line_weight,
        }
    }

    /// What [`drawing_with`] states, given the same pens.
    fn with_pens(pens: [i16; 3]) -> Stated {
        let [fine, medium, bold] = pens.map(LineWeight::Value);
        Stated {
            strokes: BTreeMap::from([
                (1, LineWeight::ByLayer),
                (2, LineWeight::ByBlock),
                (3, LineWeight::Default),
                (17, fine),
                (25, LineWeight::Value(25)),
                (34, medium),
                (68, bold),
            ]),
            attribute: Some(fine),
            layer: Some(bold),
            current: pens[1],
        }
    }

    /// [`drawing`] as drawn.
    fn as_drawn() -> Stated {
        with_pens([17, 34, 68])
    }

    /// [`drawing`] as a DWG states it: each of the three pens at the nearest
    /// standard width, everything else as drawn.
    fn as_a_dwg_states_it() -> Stated {
        with_pens([18, 35, 70])
    }

    /// `doc` saved as a DWG, and read back.
    fn through_a_dwg(doc: &Document) -> Document {
        let bytes = serialize_document(doc, "dwg").unwrap();
        DwgReader::from_stream(Cursor::new(bytes)).read().unwrap()
    }

    /// State `weight` on the stroke that stands at `x`.
    fn restate_stroke(doc: &mut Document, x: f64, weight: LineWeight) {
        let handle = doc
            .entities()
            .find_map(|entity| match entity {
                EntityType::Line(line) if line.start.x == x => Some(line.common.handle),
                _ => None,
            })
            .expect("a stroke at x");
        doc.get_entity_mut(handle).unwrap().common_mut().line_weight = weight;
    }

    /// State `weight` on the attribute of the block reference.
    fn restate_attribute(doc: &mut Document, weight: LineWeight) {
        let handle = doc
            .entities()
            .find_map(|entity| match entity {
                EntityType::Insert(reference) => Some(reference.common.handle),
                _ => None,
            })
            .expect("a block reference");
        let Some(EntityType::Insert(reference)) = doc.get_entity_mut(handle) else {
            panic!("the block reference is gone");
        };
        reference.attributes[0].common.line_weight = weight;
    }

    #[test]
    fn the_drawing_states_the_widths_the_tests_take_it_to() {
        assert_eq!(stated(&drawing()), as_drawn());
    }

    #[test]
    fn each_pen_of_the_importer_moves_to_the_standard_width_next_to_it() {
        assert_eq!(nearest_standard(17), Some(18));
        assert_eq!(nearest_standard(34), Some(35));
        assert_eq!(nearest_standard(68), Some(70));
    }

    #[test]
    fn a_standard_width_stays() {
        for width in STANDARD_WIDTHS {
            assert_eq!(nearest_standard(width), None, "{width}");
        }
    }

    #[test]
    fn a_width_goes_to_a_standard_one_no_other_is_nearer_than() {
        for width in 0..=211_i16 {
            let stated = nearest_standard(width).unwrap_or(width);
            assert!(STANDARD_WIDTHS.contains(&stated), "{width} -> {stated}");
            let off = (stated - width).abs();
            for standard in STANDARD_WIDTHS {
                assert!(
                    (standard - width).abs() >= off,
                    "{width} -> {stated}, yet {standard} is nearer"
                );
            }
        }
    }

    #[test]
    fn a_width_halfway_between_two_standard_ones_takes_the_wider() {
        let off = |width: i16| STANDARD_WIDTHS.map(|standard| (standard - width).abs());
        let halfway: Vec<i16> = (0..=211)
            .filter(|&width| {
                let nearest = off(width).into_iter().min();
                off(width)
                    .into_iter()
                    .filter(|&off| Some(off) == nearest)
                    .count()
                    == 2
            })
            .collect();
        assert_eq!(
            halfway,
            [7, 11, 14, 19, 45, 65, 75, 85, 95, 103, 113, 130, 149, 179]
        );
        for width in halfway {
            let wider = STANDARD_WIDTHS
                .into_iter()
                .find(|&standard| standard > width);
            assert_eq!(nearest_standard(width), wider, "{width}");
        }
    }

    #[test]
    fn a_width_outside_what_the_list_spans_stays() {
        for width in [i16::MIN, -4, -3, -2, -1, 212, 300, i16::MAX] {
            assert_eq!(nearest_standard(width), None, "{width}");
        }
    }

    #[test]
    fn a_width_taken_by_reference_stays() {
        for weight in [
            LineWeight::ByLayer,
            LineWeight::ByBlock,
            LineWeight::Default,
        ] {
            assert_eq!(restated(weight), None, "{weight:?}");
        }
        assert_eq!(restated(LineWeight::Value(17)), Some(LineWeight::Value(18)));
        assert_eq!(restated(LineWeight::Value(18)), None);
    }

    /// The codec keeps its list private; what it does with a width tells
    /// what is on it. A width is on the codec's list exactly when it is on
    /// this module's: the codec then gives it a place that reads back as the
    /// same width, and gives every other width the place of the default.
    #[test]
    fn the_list_is_the_codecs() {
        assert!(STANDARD_WIDTHS.windows(2).all(|pair| pair[0] < pair[1]));
        let default = LineWeight::Default.to_dwg_index();
        for width in 0..=i16::MAX {
            let weight = LineWeight::Value(width);
            let place = weight.to_dwg_index();
            if STANDARD_WIDTHS.contains(&width) {
                assert_ne!(place, default, "{width} has no place in a DWG");
                assert_eq!(LineWeight::from_dwg_index(place), weight, "{width}");
            } else {
                assert_eq!(place, default, "{width} has a place in a DWG");
            }
        }
    }

    #[test]
    fn snapping_restates_a_width_in_every_place_it_is_stated() {
        let mut doc = drawing();
        assert!(has_unstorable_dwg_line_weights(&doc));
        assert!(snap_dwg_line_weights(&mut doc));
        assert_eq!(stated(&doc), as_a_dwg_states_it());
        assert!(!has_unstorable_dwg_line_weights(&doc));

        let restated_once = doc.clone();
        assert!(!snap_dwg_line_weights(&mut doc));
        assert!(doc == restated_once, "a second pass changed the drawing");
    }

    #[test]
    fn a_drawing_of_standard_widths_is_left_as_it_is() {
        let mut doc = drawing_with([18, 35, 70]);
        let as_drawn = doc.clone();
        assert!(!has_unstorable_dwg_line_weights(&doc));
        assert!(!snap_dwg_line_weights(&mut doc));
        assert!(doc == as_drawn, "the pass changed the drawing");
    }

    /// A caller that copies the drawing only when asked to must be asked for
    /// a width off the list wherever it is the only one.
    #[test]
    fn a_width_off_the_list_is_noticed_wherever_it_alone_is_stated() {
        type Place = (&'static str, fn(&mut Document));
        let places: [Place; 5] = [
            ("the header", |doc| doc.header.current_line_weight = 34),
            ("a layer", |doc| {
                doc.layers.get_mut(PEN_LAYER).unwrap().line_weight = LineWeight::Value(68);
            }),
            ("a stroke in model space", |doc| {
                restate_stroke(doc, 17.0, LineWeight::Value(17));
            }),
            ("a stroke in a definition", |doc| {
                restate_stroke(doc, 34.0, LineWeight::Value(34));
            }),
            ("an attribute", |doc| {
                restate_attribute(doc, LineWeight::Value(17));
            }),
        ];
        for (place, state) in places {
            let mut doc = drawing_with([18, 35, 70]);
            state(&mut doc);
            assert!(has_unstorable_dwg_line_weights(&doc), "{place}");
            assert!(snap_dwg_line_weights(&mut doc), "{place}");
            assert_eq!(stated(&doc), as_a_dwg_states_it(), "{place}");
        }
    }

    /// Only the entities that change are written anew: every other keeps the
    /// record it was read with, which holds whatever of a DWG the model does
    /// not.
    #[test]
    fn snapping_keeps_the_record_of_every_entity_it_does_not_change() {
        let mut doc = through_a_dwg(&drawing_with([18, 35, 70]));
        let with_a_record = |doc: &Document| {
            doc.entities()
                .filter(|entity| entity.common().raw_record.is_some())
                .count()
        };
        let read = with_a_record(&doc);
        assert!(read > 1, "entities read with a record: {read}");

        restate_stroke(&mut doc, 17.0, LineWeight::Value(17));
        assert_eq!(with_a_record(&doc), read - 1, "the stroke restated by hand");
        assert!(snap_dwg_line_weights(&mut doc));
        assert_eq!(stated(&doc), as_a_dwg_states_it());
        assert_eq!(with_a_record(&doc), read - 1, "records kept by the pass");
    }

    #[test]
    fn a_dwg_states_the_nearest_standard_width_for_a_width_it_cannot_state() {
        let doc = drawing();
        assert_eq!(stated(&through_a_dwg(&doc)), as_a_dwg_states_it());
        assert_eq!(stated(&doc), as_drawn(), "the drawing stays as drawn");
    }

    /// A drawing opened from a DWG and then given a width off the list is
    /// saved like a drawn one: what was left alone goes out as the records it
    /// was read with, the restated stroke at the nearest standard width.
    #[test]
    fn a_drawing_opened_from_a_dwg_is_saved_at_the_nearest_standard_width() {
        let mut doc = through_a_dwg(&drawing_with([18, 35, 70]));
        restate_stroke(&mut doc, 34.0, LineWeight::Value(34));
        assert_eq!(stated(&through_a_dwg(&doc)), as_a_dwg_states_it());
    }

    /// A DXF states the number itself: the strokes, the attribute and the
    /// layer keep the width they were drawn with. The header's current width
    /// is left out: the DXF writer states it only when it is on a list of the
    /// writer's own, and BYLAYER otherwise -- 34 reads back as -1.
    #[test]
    fn a_dxf_states_the_widths_as_drawn() {
        let doc = drawing();
        let bytes = serialize_document(&doc, "dxf").unwrap();
        let reopened = DxfReader::from_reader(Cursor::new(bytes))
            .unwrap()
            .read()
            .unwrap();
        let read_back = Stated {
            current: doc.header.current_line_weight,
            ..stated(&reopened)
        };
        assert_eq!(read_back, as_drawn());
    }
}

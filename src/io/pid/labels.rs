// What the labels of a drawing hang on: each text of a label against the graphic the drawing file hangs that label on, read off records the parser already holds.

use super::*;

/// What a label hangs on, as its text states it under `label_on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LabelOn {
    /// A symbol of the sheet: the block reference whose `graphic` is the
    /// text's `label_of`.
    Symbol,
    /// A run of line, which is how the drawing draws a pipe run.
    Line,
    /// Anything else. On the plant test data that is a row of attributes in
    /// the title block.
    Other,
}

impl LabelOn {
    /// The value written under `label_on`.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Symbol => "symbol",
            Self::Line => "line",
            Self::Other => "other",
        }
    }
}

/// What each label text of a drawing hangs on.
///
/// SmartPlant draws a label as text of its own beside what it labels, and
/// the drawing file keeps the pairing in a label record (PSM `0x00FA`, the
/// parser's `DependencyObject`): each text names its label record as its
/// parent, and the record lists its members, the first of which is the
/// graphic the label hangs on -- a symbol placement, a run of line, or in
/// the title block a row of attributes (plan
/// 2026-11-20-pid-symbol-identity, SPEC section 2.3). The parser decodes
/// both kinds of record for the import, so the table costs no second
/// reading of the file.
///
/// A record's number is its stream's own, so a label is read against the
/// records of the stream it lies in.
#[derive(Debug, Default)]
pub(super) struct SheetLabels {
    by_stream: HashMap<String, StreamLabels>,
}

/// The labels of one sheet stream.
#[derive(Debug, Default)]
struct StreamLabels {
    /// The graphic each label text hangs on, by the text's own number.
    hung_on: HashMap<u32, u32>,
    /// The numbers of the stream's runs of line (`igLineString2d`). On the
    /// plant test data every label that hangs on line work hangs on one of
    /// these; none hangs on a single line (`igLine2d`).
    runs: HashSet<u32>,
}

impl SheetLabels {
    /// The labels of the drawing `parsed` was read from: of every sheet
    /// stream the parser decoded records of.
    pub(super) fn read(parsed: &pid_parse::PidDocument) -> Self {
        let mut labels = Self::default();
        for sheet in &parsed.sheet_streams {
            let Some(records) = sheet.geometry.as_ref() else {
                continue;
            };
            labels.add_stream(
                &sheet.path,
                records
                    .decoded_igtextboxes
                    .iter()
                    .map(|text| (text.oid, text.parent_ref)),
                records
                    .decoded_dependency_objects
                    .iter()
                    .filter_map(|label| {
                        let first = first_member(
                            label.group_kind_word,
                            label.sub_type_word,
                            &label.raw_reference_payload,
                        )?;
                        Some((label.oid, first))
                    }),
                records.decoded_iglinestrings.iter().map(|run| run.oid),
            );
        }
        labels
    }

    /// Add the labels of the stream stored at `path`: its `texts`, each
    /// with the record it names as its parent; its `labels`, each label
    /// record with its first member; and its `runs` of line. A text whose
    /// parent is none of `labels` hangs on nothing.
    pub(super) fn add_stream(
        &mut self,
        path: &str,
        texts: impl IntoIterator<Item = (u32, u32)>,
        labels: impl IntoIterator<Item = (u32, u32)>,
        runs: impl IntoIterator<Item = u32>,
    ) {
        let first_of: HashMap<u32, u32> = labels.into_iter().collect();
        let stream = self.by_stream.entry(path.to_string()).or_default();
        stream.hung_on.extend(
            texts
                .into_iter()
                .filter_map(|(text, parent)| Some((text, *first_of.get(&parent)?))),
        );
        stream.runs.extend(runs);
    }

    /// The graphic the label of text `text` of the stream at `stream` hangs
    /// on, where the text is a label's.
    pub(super) fn hung_on(&self, stream: &str, text: u32) -> Option<u32> {
        self.by_stream.get(stream)?.hung_on.get(&text).copied()
    }

    /// Whether graphic `graphic` of the stream at `stream` is a run of
    /// line.
    pub(super) fn is_run(&self, stream: &str, graphic: u32) -> bool {
        self.by_stream
            .get(stream)
            .is_some_and(|labels| labels.runs.contains(&graphic))
    }
}

/// The number of the first member of a label record, or `None` for a record
/// that lists none.
///
/// A label record's content lists its members from offset 16, six bytes
/// each: the member's number, then a word that is 1. How many there are
/// stands before them, at offset 14. The parser hands the record over the
/// way it reads every `DependencyObject` -- that count as
/// `group_kind_word`, the two bytes after it as `sub_type_word`, the rest
/// as a raw tail -- so the first number comes in two halves:
/// `sub_type_word` is its low half, and the first two bytes of the tail are
/// its high half.
pub(super) fn first_member(members: u16, low_half: u16, tail: &[u8]) -> Option<u32> {
    if members == 0 {
        return None;
    }
    let high_half = u16::from_le_bytes([*tail.first()?, *tail.get(1)?]);
    Some(u32::from(low_half) | (u32::from(high_half) << 16))
}

#[cfg(test)]
mod tests {
    use pid_parse::{
        DecodedDependencyObjectRecord, DecodedIgLineString2dRecord, DecodedIgTextBoxRecord,
        SheetGeometry, SheetStream,
    };

    use super::*;

    /// The first member of a label record is put together from the two
    /// fields the parser hands it over in: the low half of its number and,
    /// low byte first, the high half at the head of the tail. A record that
    /// lists no member has none, and neither has one whose tail is too
    /// short to hold the high half.
    #[test]
    fn the_first_member_of_a_label_record_is_read_across_the_fields_that_hold_it() {
        // Label record 3845 of the sample sheet: two members, 41377 and
        // 5984, each followed by its word of 1.
        let tail = [0, 0, 1, 0, 96, 23, 0, 0, 1, 0, 1, 0, 251, 255];
        assert_eq!(first_member(2, 41_377, &tail), Some(41_377));
        assert_eq!(
            first_member(1, 0x9A7C, &[0x02, 0x01, 1, 0]),
            Some(0x0102_9A7C),
            "a number past 65,535 has its high half in the tail, low byte first"
        );
        assert_eq!(
            first_member(1, 7, &[0, 0]),
            Some(7),
            "the two bytes of the high half are all the tail it needs"
        );
        assert_eq!(
            first_member(0, 41_377, &tail),
            None,
            "a record that lists no member"
        );
        assert_eq!(
            first_member(2, 41_377, &[7]),
            None,
            "a tail too short for the high half"
        );
        assert_eq!(first_member(2, 41_377, &[]), None);
    }

    /// A text hangs on the first member of the label record it names as its
    /// parent, and the texts of one label hang on the same graphic. A text
    /// whose parent is no label record hangs on nothing, and a number is
    /// read within its own stream: the same number in another stream is
    /// another record.
    #[test]
    fn a_text_hangs_on_the_first_member_of_the_label_record_it_names() {
        let mut labels = SheetLabels::default();
        labels.add_stream(
            "/Sheet6",
            [
                (40_273, 3_845),
                (40_241, 3_845),
                (51_631, 52_520),
                (700, 6),
                (701, 999),
            ],
            [(3_845, 41_377), (52_520, 52_304), (82, 6_121)],
            [52_304, 77],
        );
        labels.add_stream("/Sheet7", [(40_273, 12)], [(12, 500)], [41_377]);

        assert_eq!(labels.hung_on("/Sheet6", 40_273), Some(41_377));
        assert_eq!(
            labels.hung_on("/Sheet6", 40_241),
            Some(41_377),
            "the second text of the same label"
        );
        assert_eq!(labels.hung_on("/Sheet6", 51_631), Some(52_304));
        assert_eq!(
            [
                labels.hung_on("/Sheet6", 700),
                labels.hung_on("/Sheet6", 701)
            ],
            [None, None],
            "a parent that is no label record"
        );
        assert_eq!(
            [
                labels.hung_on("/Sheet6", 3_845),
                labels.hung_on("/Sheet6", 41_377),
                labels.hung_on("/Sheet6", 9),
            ],
            [None, None, None],
            "a label record, the graphic it hangs on, a number of nothing: none is a label's text"
        );
        assert_eq!(
            labels.hung_on("/Sheet7", 40_273),
            Some(500),
            "the same number in another stream is another text"
        );
        assert_eq!(labels.hung_on("/Sheet7", 51_631), None);
        assert_eq!(
            labels.hung_on("/Sheet8", 40_273),
            None,
            "a stream the table has nothing of"
        );

        assert!(labels.is_run("/Sheet6", 52_304));
        assert!(labels.is_run("/Sheet6", 77));
        assert!(!labels.is_run("/Sheet6", 41_377), "a run of another stream");
        assert!(labels.is_run("/Sheet7", 41_377));
        assert!(!labels.is_run("/Sheet7", 77));
        assert!(!labels.is_run("/Sheet8", 77));
    }

    /// A text box as the parser decodes one: number `oid`, parent `parent`.
    fn text_box(oid: u32, parent: u32) -> DecodedIgTextBoxRecord {
        DecodedIgTextBoxRecord {
            byte_start: 0,
            byte_end: 0,
            type_code: 0x004D,
            type_flags: 0,
            bytes_to_follow: 0,
            oid,
            parent_ref: parent,
            sheet_layer_ref: 0,
            sub_type_word: 0,
            index: 0,
            text_length: 2,
            text: "PT".to_string(),
            trailing_double_1: 0.0,
            trailing_double_2: 0.0,
            text_sub_type: 0,
            trailing_double_3: 0.0,
            rotation_rad: 0.0,
            letter_height_m: 0.0,
            anchor_code: 0,
            text_box_width_m: 0.0,
            text_box_height_m: 0.0,
            runs: Vec::new(),
        }
    }

    /// A label record as the parser decodes one: number `oid`, `members`
    /// members, the low half of the first in `sub_type_word` and the rest
    /// of the list in `tail`.
    fn label_record(
        oid: u32,
        members: u16,
        low_half: u16,
        tail: &[u8],
    ) -> DecodedDependencyObjectRecord {
        DecodedDependencyObjectRecord {
            byte_start: 0,
            byte_end: 0,
            type_code: 0x00FA,
            type_flags: 0,
            bytes_to_follow: 0,
            oid,
            parent_ref: 6,
            group_kind_word: members,
            sub_type_word: low_half,
            raw_reference_payload: tail.to_vec(),
        }
    }

    /// A run of line as the parser decodes one: number `oid`.
    fn run(oid: u32) -> DecodedIgLineString2dRecord {
        DecodedIgLineString2dRecord {
            byte_start: 0,
            byte_end: 0,
            type_code: 0x0084,
            type_flags: 0,
            bytes_to_follow: 0,
            oid,
            parent_ref: 0,
            sheet_layer_ref: 0,
            sub_type_word: 0,
            index: 0,
            form: 0,
            scope: 0,
            vertex_xs: vec![0.0, 0.1],
            vertex_ys: vec![0.0, 0.0],
        }
    }

    /// A sheet stream stored at `path`, holding `records`.
    fn stream(path: &str, records: Option<SheetGeometry>) -> SheetStream {
        SheetStream {
            name: path.trim_start_matches('/').to_string(),
            path: path.to_string(),
            size: 0,
            extracted_texts: Vec::new(),
            magic_u32_le: None,
            magic_tag: None,
            header: None,
            attribute_records: Vec::new(),
            probe_summary: None,
            geometry: records,
            endpoint_records: Vec::new(),
            endpoint_decode_error: None,
        }
    }

    /// The table is read off the records the parser decoded: a text box's
    /// number and parent, a label record's number and first member, a run's
    /// number -- each within the stream that holds it. A label record that
    /// lists no member pairs nothing, and a stream the parser decoded no
    /// records of adds nothing.
    #[test]
    fn the_labels_are_read_off_the_records_the_parser_decoded() {
        let mut parsed = pid_parse::PidDocument::default();
        assert_eq!(
            SheetLabels::read(&parsed).hung_on("/Sheet6", 40_273),
            None,
            "a document without a sheet has no label"
        );
        parsed.sheet_streams = vec![
            stream(
                "/Sheet6",
                Some(SheetGeometry {
                    decoded_igtextboxes: vec![
                        text_box(40_273, 3_845),
                        text_box(51_631, 52_520),
                        text_box(60_001, 4_000),
                        text_box(60_002, 6),
                    ],
                    decoded_dependency_objects: vec![
                        label_record(3_845, 2, 41_377, &[0, 0, 1, 0, 96, 23, 0, 0, 1, 0]),
                        label_record(52_520, 2, 52_304, &[1, 0, 1, 0, 231, 200, 0, 0, 1, 0]),
                        label_record(4_000, 0, 41_377, &[0, 0, 1, 0]),
                    ],
                    decoded_iglinestrings: vec![run(118_000), run(77)],
                    ..Default::default()
                }),
            ),
            stream("/Sheet7", None),
            stream(
                "/Sheet8",
                Some(SheetGeometry {
                    decoded_igtextboxes: vec![text_box(40_273, 9)],
                    decoded_dependency_objects: vec![label_record(9, 1, 77, &[0, 0, 1, 0])],
                    ..Default::default()
                }),
            ),
        ];
        let labels = SheetLabels::read(&parsed);

        assert_eq!(labels.hung_on("/Sheet6", 40_273), Some(41_377));
        assert_eq!(
            labels.hung_on("/Sheet6", 51_631),
            Some(117_840),
            "52304 and a high half of 1"
        );
        assert_eq!(
            labels.hung_on("/Sheet6", 60_001),
            None,
            "its label record lists no member"
        );
        assert_eq!(
            labels.hung_on("/Sheet6", 60_002),
            None,
            "its parent is no label record"
        );
        assert_eq!(
            labels.hung_on("/Sheet8", 40_273),
            Some(77),
            "each stream is read for itself"
        );
        assert_eq!(labels.hung_on("/Sheet7", 40_273), None);
        assert_eq!(
            [
                labels.is_run("/Sheet6", 118_000),
                labels.is_run("/Sheet6", 77),
                labels.is_run("/Sheet6", 41_377),
                labels.is_run("/Sheet8", 77),
            ],
            [true, true, false, false],
            "the runs of /Sheet6, and none of /Sheet8"
        );
    }
}

//! What the import makes of every drawing of the plant test backup, in sum
//! (plan 2026-11-20-pid-symbol-identity, SI-8; SPEC section 6, items 3 to
//! 5).
//!
//! One test, run only when asked for: it imports the 53 drawings of the
//! backup and reads the backup's 74 MB dump for the tags of its database.
//! What the import makes of one of those drawings, the sample, is pinned by
//! tests that run by default (`tests/pid_import.rs`, `sample_sheet`).
//!
//! The drawings are read where the backup's index finds them, with nothing
//! published beside them. Where the backup is not on the machine, or
//! `PID_SYMBOL_LIBRARY` names libraries of its own, the test says so and
//! passes -- which proves nothing: the counts are held only on a machine
//! that has the backup.
//!
//! The counts are of what the drawings hold once imported, not of the
//! records of their files. The files pair 15,616 text records with what
//! they label, 13,161 of them with a symbol; 126 of those are blank and
//! draw no text, so the texts are 15,490 and 13,035. Nineteen symbols have
//! none but blank ones, so the symbols a text hangs on are 6,091 where the
//! records make them 6,110 (the plan's todo.md, SI-7).

use std::time::Instant;

use super::*;

/// The drawings the backup lists.
const DRAWINGS: usize = 53;

/// The one of them the import refuses: it holds no graphics at all.
const EMPTY: &str = "D03.pid";

/// What the symbols and the texts of the other 52 say, read from the files
/// alone ([`Counted::lines`]). Counted on the drawings on 2026-11-21, not
/// derived; the kinds are the rows of SPEC section 2.2, less one.
///
/// The counts are those of the conversion library `Cargo.toml` pins,
/// pid-parse `a85fa10`. It gives no representation to a symbol that stands
/// for a pipe run -- the gap drawn where two lines cross, of which the
/// plant has three -- so those three name no item and state no class. Under
/// `96ee00c`, which the SPEC was measured with, they did: 9218 of an item,
/// 8742 of a class, and a row `PipeRun PIDPipingConnector 3`.
const FROM_THE_FILES: &str = "\
symbols 9225, 0 of them sharing a graphic
of an item 9215
of a class 8739
tagged 2957: file 2957
named otherwise in the file 0
labels 15490: line 2303, other 152, symbol 13035
labels of a symbol the sheet has not 0
labelled symbols 6091
texts of no label, the frame's aside 95
EquipmentOther PIDProcessEquipment 225
Exchanger PIDHeatTransferEquipment 12
Instrument PIDControlSystemFunction 1104
Instrument PIDInlineInstrument 1025
Instrument PIDInstrument 1576
ItemNote PIDNote 793
Mechanical PIDMechanicalEquipment 91
Nozzle PIDNozzle 372
OPC (no class) 476
PipingComp PIDPipingComponent 3473
Vessel PIDProcessVessel 68";

/// What their symbols say of their tags once the tags of the backup's
/// database are put on them ([`Tagged::lines`]). Counted likewise, and under
/// the same library: the three gap symbols, naming no item, are given no
/// tag. Under `96ee00c` the database's tags reached them too: 5027 tagged,
/// 5006 of them by the database, 2070 in the database alone, 4198 in
/// neither.
const WITH_THE_DATABASE: &str = "\
tagged 5024: database 5003, file 21
named otherwise in the file 32
the database's tag put on 5003, 32 of them named otherwise in the file
in both and the same 2904, in both and not the same 32, in the database alone 2067, \
in the file alone 21, in neither 4201, otherwise 0";

/// Where the plant backup lies -- the plant test data, a folder beside this
/// repository -- or `None`, and a line saying why, where it cannot be read
/// the way the counts were taken.
fn backup() -> Option<PathBuf> {
    if std::env::var_os(SYMBOL_LIBRARY_ENV).is_some() {
        eprintln!("skipped: {SYMBOL_LIBRARY_ENV} is set; the counts were taken without it");
        return None;
    }
    let backup = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pid-test-data");
    if !backup.join("Export.dmp").is_file() {
        eprintln!("skipped: no plant backup at {}", backup.display());
        return None;
    }
    Some(backup)
}

/// What an entity states for `key`, an empty value being nothing stated.
fn stated<'a>(entity: &'a EntityType, key: &str) -> Option<&'a str> {
    pid_value(entity, key).filter(|value| !value.is_empty())
}

/// The symbols of a drawing: the block references of model space that say
/// `role=symbol`.
fn symbols(doc: &Document) -> impl Iterator<Item = &EntityType> {
    doc.model_space_entities().filter(|entity| {
        matches!(entity, EntityType::Insert(_)) && stated(entity, "role") == Some("symbol")
    })
}

/// `counts` as `name count, name count`, the names in the order of the
/// alphabet.
fn listed(counts: &BTreeMap<String, usize>) -> String {
    let counts: Vec<String> = counts
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    counts.join(", ")
}

/// What the symbols and the texts of drawings say, counted.
#[derive(Debug, Default)]
struct Counted {
    /// The symbols.
    symbols: usize,
    /// Of those, the ones that state the graphic another symbol of their
    /// drawing states.
    sharing_a_graphic: usize,
    /// The ones that name an item, and the ones that state a class.
    of_an_item: usize,
    of_a_class: usize,
    /// The ones that carry a tag, by where the tag is from.
    tagged_by: BTreeMap<String, usize>,
    /// The ones a name of the drawing file is kept beside the tag of.
    named_otherwise: usize,
    /// The ones that state what kind of item they stand for, by that kind
    /// and the class stated with it.
    kinds: BTreeMap<String, usize>,
    /// The entities that say they are a label's, by what the label hangs
    /// on.
    labels_on: BTreeMap<String, usize>,
    /// Of the ones that hang on a symbol, those whose symbol is none of
    /// their drawing's.
    astray: usize,
    /// The symbols at least one of them hangs on.
    labelled: usize,
    /// The texts that are no label's, the ones of the frame aside.
    loose_texts: usize,
}

impl Counted {
    /// Add what `doc` holds.
    fn add(&mut self, doc: &Document) {
        let mut graphics = HashSet::new();
        for symbol in symbols(doc) {
            self.symbols += 1;
            if let Some(graphic) = stated(symbol, "graphic") {
                self.sharing_a_graphic += usize::from(!graphics.insert(graphic));
            }
            self.of_an_item += usize::from(stated(symbol, "item").is_some());
            self.of_a_class += usize::from(stated(symbol, "class").is_some());
            if let Some(kind) = stated(symbol, "item_type") {
                let class = stated(symbol, "class").unwrap_or("(no class)");
                *self.kinds.entry(format!("{kind} {class}")).or_default() += 1;
            }
            if stated(symbol, "label").is_some() {
                let by = stated(symbol, "tag_source").unwrap_or("(not said)");
                *self.tagged_by.entry(by.to_string()).or_default() += 1;
            }
            self.named_otherwise += usize::from(stated(symbol, "label_in_file").is_some());
        }

        let mut labelled = HashSet::new();
        for entity in doc.model_space_entities() {
            let Some(of) = stated(entity, "label_of") else {
                let loose = matches!(entity, EntityType::Text(_))
                    && stated(entity, "role") != Some("frame");
                self.loose_texts += usize::from(loose);
                continue;
            };
            let on = stated(entity, "label_on").unwrap_or("(not said)");
            *self.labels_on.entry(on.to_string()).or_default() += 1;
            if on == "symbol" {
                if graphics.contains(of) {
                    labelled.insert(of);
                } else {
                    self.astray += 1;
                }
            }
        }
        self.labelled += labelled.len();
    }

    /// The counts as lines, a count to a line and the kinds last.
    fn lines(&self) -> String {
        let mut lines = vec![
            format!(
                "symbols {}, {} of them sharing a graphic",
                self.symbols, self.sharing_a_graphic
            ),
            format!("of an item {}", self.of_an_item),
            format!("of a class {}", self.of_a_class),
            format!(
                "tagged {}: {}",
                self.tagged_by.values().sum::<usize>(),
                listed(&self.tagged_by)
            ),
            format!("named otherwise in the file {}", self.named_otherwise),
            format!(
                "labels {}: {}",
                self.labels_on.values().sum::<usize>(),
                listed(&self.labels_on)
            ),
            format!("labels of a symbol the sheet has not {}", self.astray),
            format!("labelled symbols {}", self.labelled),
            format!("texts of no label, the frame's aside {}", self.loose_texts),
        ];
        lines.extend(
            self.kinds
                .iter()
                .map(|(kind, count)| format!("{kind} {count}")),
        );
        lines.join("\n")
    }
}

/// What the symbols of drawings say of their tags once the tags of the
/// database are put on them, counted.
#[derive(Debug, Default)]
struct Tagged {
    /// The symbols that carry a tag, by where the tag is from.
    tagged_by: BTreeMap<String, usize>,
    /// The ones a name of the drawing file is kept beside the tag of.
    named_otherwise: usize,
    /// What putting the tags on answered: the symbols the database tags,
    /// and those of them the file names otherwise.
    put_on: usize,
    differing: usize,
    /// The symbols by what the file and the database call them: both the
    /// same, both and not the same, the one alone, the other alone,
    /// neither -- and whatever is none of these.
    same: usize,
    not_the_same: usize,
    database_alone: usize,
    file_alone: usize,
    neither: usize,
    otherwise: usize,
}

impl Tagged {
    /// Add what the symbols of `doc` say now that `fill` is what putting
    /// the tags on them answered; `named` is what the file called each of
    /// them before, by its handle.
    fn add(&mut self, doc: &Document, named: &HashMap<u64, String>, fill: DatabaseTagFill) {
        self.put_on += fill.tagged;
        self.differing += fill.differing;
        for symbol in symbols(doc) {
            let kept = stated(symbol, "label_in_file");
            self.named_otherwise += usize::from(kept.is_some());
            let before = named
                .get(&symbol.common().handle.value())
                .map(String::as_str);
            let Some(label) = stated(symbol, "label") else {
                if kept.is_none() && before.is_none() {
                    self.neither += 1;
                } else {
                    self.otherwise += 1;
                }
                continue;
            };
            let by = stated(symbol, "tag_source").unwrap_or("(not said)");
            *self.tagged_by.entry(by.to_string()).or_default() += 1;
            match (by, kept, before) {
                ("database", None, Some(before)) if before == label => self.same += 1,
                ("database", Some(kept), Some(before)) if kept == before && kept != label => {
                    self.not_the_same += 1;
                }
                ("database", None, None) => self.database_alone += 1,
                ("file", None, Some(before)) if before == label => self.file_alone += 1,
                _ => self.otherwise += 1,
            }
        }
    }

    /// The counts as lines.
    fn lines(&self) -> String {
        [
            format!(
                "tagged {}: {}",
                self.tagged_by.values().sum::<usize>(),
                listed(&self.tagged_by)
            ),
            format!("named otherwise in the file {}", self.named_otherwise),
            format!(
                "the database's tag put on {}, {} of them named otherwise in the file",
                self.put_on, self.differing
            ),
            format!(
                "in both and the same {}, in both and not the same {}, in the database alone {}, \
                 in the file alone {}, in neither {}, otherwise {}",
                self.same,
                self.not_the_same,
                self.database_alone,
                self.file_alone,
                self.neither,
                self.otherwise
            ),
        ]
        .join("\n")
    }
}

/// The drawings of the plant backup, imported one by one, come to the
/// counts taken of them (SPEC section 6, items 3 to 5): one of the 53 holds
/// no graphics and is refused for that; the symbols of the other 52 state
/// what the files say of them, their texts what they are labels of; and
/// with the tags of the backup's database put on them, the symbols carry
/// those.
#[test]
#[ignore = "imports the 53 drawings of the plant backup and reads its 74 MB dump; run with --ignored --nocapture"]
fn the_drawings_of_the_plant_backup_come_to_the_counts_taken_of_them() {
    let Some(backup) = backup() else {
        return;
    };
    let index =
        crate::io::pid_backup_browse::build_index(&backup, None).expect("the backup indexes");
    let tags = index
        .dump()
        .expect("an Oracle dump reads")
        .item_tags()
        .expect("the item tags");
    assert_eq!(index.drawings.len(), DRAWINGS);

    let started = Instant::now();
    let mut from_the_files = Counted::default();
    let mut with_the_database = Tagged::default();
    let mut refused = Vec::new();
    for drawing in &index.drawings {
        let path = drawing
            .pid_path
            .as_deref()
            .unwrap_or_else(|| panic!("{} has its file", drawing.name));
        let file = path
            .file_name()
            .expect("a file has a name")
            .to_string_lossy()
            .into_owned();
        let mut doc = match load_pid(path) {
            Ok(import) => import.document,
            Err(why) => {
                println!("{file}: refused: {why}");
                refused.push((file, why));
                continue;
            }
        };
        let (symbols_before, labels_before) = (
            from_the_files.symbols,
            from_the_files.labels_on.values().sum::<usize>(),
        );
        from_the_files.add(&doc);
        let named: HashMap<u64, String> = symbols(&doc)
            .filter_map(|symbol| {
                let name = stated(symbol, "label")?.to_string();
                Some((symbol.common().handle.value(), name))
            })
            .collect();
        let fill = fill_database_tags(&mut doc, &tags, |_| false);
        with_the_database.add(&doc, &named, fill);
        println!(
            "{file}: {} symbol(s), {} named by the file, {} label text(s); the database: {fill:?}",
            from_the_files.symbols - symbols_before,
            named.len(),
            from_the_files.labels_on.values().sum::<usize>() - labels_before,
        );
    }
    println!(
        "{} drawing(s), {} refused, in {:.1} s; {} tag(s) in the database",
        index.drawings.len(),
        refused.len(),
        started.elapsed().as_secs_f64(),
        tags.len()
    );
    println!("from the files:\n{}", from_the_files.lines());
    println!("with the database:\n{}", with_the_database.lines());

    let file = EMPTY.to_string();
    let nothing_to_draw = crate::tf!(
        "{} holds nothing to draw: the parser found no graphics in it",
        file
    )
    .into_owned();
    assert_eq!(refused, vec![(EMPTY.to_string(), nothing_to_draw)]);
    assert_eq!(from_the_files.lines(), FROM_THE_FILES);
    assert_eq!(with_the_database.lines(), WITH_THE_DATABASE);
}

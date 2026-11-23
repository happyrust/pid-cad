//! The backup's database dump, read whole, and the views of it the reader
//! gives.
//!
//! Indexing a backup reads two tables of its `Export.dmp` -- the drawings
//! and the plant groups -- and lets the dump go again. What else the pinned
//! reader knows of the database is read here, the first time the panel asks
//! for it ([`BackupIndex::dump`]), and kept for as long as the index is: the
//! tables of thirteen names. For a plant of 53 drawings (a dump of 74 MB)
//! that is 1.7 seconds and 123 MiB in a debug build, 147 MiB once the views
//! are worked out (`the_dumps_of_the_test_backups_are_read_and_measured`).
//!
//! The reader takes the tables all or none, and it refuses a table it
//! cannot follow to the letter. A dump it refuses is kept as its bytes, and
//! the views that can do without the refused table are read from those, one
//! by one ([`Dump::partial`]).
//!
//! The reader has no "rows of table X": it has typed projections, each of
//! the columns one publishing rule needs. Those are the views:
//!
//! - of the whole database: the pipe runs with the name their pipeline goes
//!   by, the piping and signal ports, the nozzle and note relations, the
//!   code lists ([`Dump::pipe_runs`] and its neighbours);
//! - of one drawing: its ports, which end of which run they connect, its
//!   relations, its off-page connectors and its pipelines
//!   ([`Dump::of_drawing`]).
//!
//! A SQL Server dump is not read: [`BackupIndex::dump`] says so.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pid_parse::publish::oracle_exp::{
    backup_uid_key, composition_relations, connectivity_ends_for_drawing, nominal_spelling,
    opc_points_for_drawing, ports_for_parents, read_port_tables, ExpTables, PortTables, StoredCell,
};
use pid_parse::publish::pid_load::{
    PidPublishDrawing, PidPublishItem, PidPublishStats, PipelineSynthesis,
};
use pid_parse::publish::pipeline::{attach_pipelines, pipeline_name};
use pid_parse::publish::placement::DumpColumn;

use super::{BackupIndex, DrawingItems, FileKind};

/// The class of a published piping connector: what pipelines are made of.
const PIPING_CONNECTOR: &str = "PIDPipingConnector";

/// The code list a pipe run's fluid code (`OPERFLUIDCODE`) is an index of.
const FLUID_CODES: i64 = 19;

/// How far code list numbers are looked for. The reader answers for one
/// list at a time and cannot say which lists there are; the plant backup's
/// highest is 182.
const CODE_LISTS_PROBED: i64 = 1000;

/// The dump of a backup in its index: read when first asked for, by
/// whichever thread asks, and shared by the copies of the index.
#[derive(Clone, Default)]
pub struct DumpCell(Arc<OnceLock<Result<Arc<Dump>, String>>>);

impl std::fmt::Debug for DumpCell {
    /// Never the tables: they are megabytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0.get() {
            None => "DumpCell(not read)",
            Some(Ok(_)) => "DumpCell(read)",
            Some(Err(_)) => "DumpCell(unreadable)",
        })
    }
}

impl BackupIndex {
    /// Why the backup's database cannot be read, without reading it: it has
    /// no dump, or the dump is not an Oracle export.
    pub fn dump_unreadable(&self) -> Option<String> {
        let Some(file) = self
            .files
            .iter()
            .find(|file| file.kind == FileKind::ExportDump && !file.missing)
        else {
            return Some(crate::t!("This backup has no database dump.").into_owned());
        };
        file.version_line.is_none().then(|| {
            crate::t!("The dump is not an Oracle export; the panel does not read its database.")
                .into_owned()
        })
    }

    /// The backup's dump with its tables read. The first call reads it --
    /// seconds, on the caller's thread, so not the UI's -- and every later
    /// one has it.
    pub fn dump(&self) -> Result<Arc<Dump>, String> {
        self.dump
            .0
            .get_or_init(|| {
                if let Some(why) = self.dump_unreadable() {
                    return Err(why);
                }
                let file = self
                    .files
                    .iter()
                    .find(|file| file.kind == FileKind::ExportDump && !file.missing)
                    .expect("a readable dump is a file of the backup");
                // A test tells by this count which thread a dump is read on.
                #[cfg(test)]
                tests::DUMPS_READ_HERE.with(|read| read.set(read.get() + 1));
                Dump::read(&self.backup_root.join(&file.name)).map(Arc::new)
            })
            .clone()
    }

    /// The dump, when it has been read already.
    pub fn dump_if_read(&self) -> Option<Arc<Dump>> {
        self.dump.0.get().and_then(|read| read.clone().ok())
    }
}

/// The thread the item tags of a backup are read on.
const TAGS_THREAD: &str = "ocs-pid-backup-tags";

/// [`Dump::item_tags`] of a backup's dump, on a worker thread: the first to
/// ask for the dump reads it, and that is seconds the UI thread must not
/// wait out.
pub async fn item_tags_in_background(
    index: Arc<BackupIndex>,
) -> Result<Arc<HashMap<String, String>>, String> {
    on_tags_thread(move || index.dump().and_then(|dump| dump.item_tags())).await
}

/// `work` on a thread of its own, under the name of [`TAGS_THREAD`]. A
/// thread that is gone before it answers is a read that failed.
async fn on_tags_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name(TAGS_THREAD.into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|error| {
            crate::tf!("cannot start reading the backup data: {}", error).into_owned()
        })?;
    receiver.await.unwrap_or_else(|_| {
        Err(crate::t!("the backup data read stopped before it finished").into_owned())
    })
}

/// The column of the dump the tags of the plant items are stored in.
fn item_tag_column() -> DumpColumn {
    DumpColumn {
        table: "T_PlantItem".to_string(),
        column: "ItemTag".to_string(),
    }
}

/// The tags of that column's cells, by the ids the cells come under: the
/// text of a cell without the blanks around it. A cell that is all blanks
/// is no tag, nor is one that is no text.
fn tags_of_cells(cells: HashMap<String, StoredCell>) -> HashMap<String, String> {
    cells
        .into_iter()
        .filter_map(|(id, cell)| match cell {
            StoredCell::Text(tag) => {
                let tag = tag.trim();
                (!tag.is_empty()).then(|| (id, tag.to_string()))
            }
            _ => None,
        })
        .collect()
}

/// What was read of a dump.
enum Tables {
    /// Every table the reader knows. Boxed: the tables are many times the
    /// size of the other variant.
    Whole(Box<ExpTables>),
    /// The reader refused one of them, and with it the read of them all.
    /// The views that do not need that table are read from the dump's
    /// bytes, each on its own; the others say why they are not there.
    Partial { dump: Vec<u8>, why: String },
}

/// An Oracle dump as the reader holds it: the tables it knows.
pub struct Dump {
    tables: Tables,
    /// The size of the dump's file.
    pub bytes: u64,
    /// How long reading the file and its tables took.
    pub read_in: Duration,
    ports: OnceLock<Result<PortTables, String>>,
    relations: OnceLock<Result<Vec<Relation>, String>>,
    fluids: OnceLock<Result<HashMap<i64, String>, String>>,
    item_tags: OnceLock<Result<Arc<HashMap<String, String>>, String>>,
}

/// One `T_PIPERUN` row as a pipeline is named from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeRun {
    /// `SP_ID`.
    pub id: String,
    /// The name of the pipeline the run belongs to; empty for a run with no
    /// tag sequence number, which is no pipeline.
    pub pipeline: String,
    /// `NOMINALDIAMETER`, spelled (`25 mm`, `1 in`); `#` and the code when
    /// the code is none the reader spells.
    pub nominal: String,
    /// `OPERFLUIDCODE` as code list 19 spells it; `#` and the code when the
    /// list has no such index.
    pub fluid: String,
    /// `PIPINGMATERIALSCLASS`.
    pub materials_class: String,
    /// `TAGSEQUENCENO`.
    pub tag_sequence: String,
    /// `FLUIDSYSTEM`, the code as stored.
    pub fluid_system: String,
    /// `INSULATIONSPEC`.
    pub insulation: String,
}

/// One port row: a point of a plant item where a run connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    /// The item's UID, a dot, and the point's number.
    pub uid: String,
    /// `SP_PLANTITEMID`: the item the port is on.
    pub item_uid: String,
    /// What the tree calls that item, when a drawing's items are at hand.
    pub item: Option<String>,
    /// `PIDPipingPort` or `PIDSignalPort`.
    pub class: &'static str,
    /// `PIPINGPOINTNUMBER` / `SIGNALPOINTNUMBER`.
    pub number: i64,
    /// `FLOWDIRECTION`, the code as stored; empty for a signal port and for
    /// a row that has none.
    pub flow: String,
    /// `NOMINALDIAMETER`, spelled as in [`PipeRun::nominal`]; empty likewise.
    pub nominal: String,
}

/// A nozzle of an equipment, or a note of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// `EquipmentComponentComposition` or `PBSItemNotes`.
    pub definition: &'static str,
    /// The nozzle or the note.
    pub from_uid: String,
    /// The equipment, or the item the note is about.
    pub to_uid: String,
    /// What the tree calls the two, when a drawing's items are at hand.
    pub from: Option<String>,
    pub to: Option<String>,
}

/// One entry of a code list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeEntry {
    /// `CODELIST_NUMBER`.
    pub list: i64,
    /// `CODELIST_INDEX`.
    pub index: i64,
    /// `CODELIST_SHORT_TEXT`.
    pub text: String,
}

/// The code lists the reader could spell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeLists {
    /// The entries with a short text, by list and index.
    pub entries: Vec<CodeEntry>,
    /// How many lists they are of.
    pub lists: usize,
    /// How many rows the table has: entries with no short text, and lists
    /// numbered past [`Self::probed_to`], are rows that are no entry here.
    pub rows: usize,
    /// The highest list number looked for.
    pub probed_to: i64,
}

/// Which end of which run a port, a branch point or an off-page connector
/// is connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// The port, branch point or off-page connector.
    pub symbol_uid: String,
    /// The pipe run or signal run.
    pub run_uid: String,
    /// What the tree calls that run, when it is one of the drawing's items.
    pub run: Option<String>,
    /// Connected at the run's start, at its end.
    pub on_start: bool,
    pub on_end: bool,
    /// `PipingEnd1Conn`, `SignalTapOrFitting` and so on.
    pub definition: &'static str,
}

/// An off-page connector drawn on a drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opc {
    /// The lesser id of the pair: the UID it is published under.
    pub uid: String,
    /// The representation drawn on this drawing.
    pub representation_uid: String,
    /// `OPCTAG`.
    pub name: String,
    /// The description of its model item.
    pub description: String,
    /// `TOFROMTEXT`.
    pub to_from_text: String,
}

/// A pipeline: the piping connectors of a drawing that share a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// Made from the plant's name and the pipeline's; the database has no
    /// row for a pipeline.
    pub uid: String,
    /// `{diameter}-{fluid}-{sequence}-{class}[-{insulation}]`.
    pub name: String,
    /// What the tree calls its connectors, and how many they are.
    pub connectors: Vec<String>,
}

/// The pipelines of a drawing, and the connectors that are in none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pipelines {
    pub rows: Vec<Pipeline>,
    /// Piping connectors of the drawing, in a pipeline or not.
    pub connectors: usize,
    /// Connectors with no tag sequence number, with a fluid code the list
    /// does not spell, and with no row among the pipe runs.
    pub without_sequence: usize,
    pub fluid_code_missing: usize,
    pub not_in_piperun: usize,
}

/// What the database knows of one drawing. Each view failed or not on its
/// own: a table the reader cannot follow costs that view only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingDump {
    pub ports: Result<Vec<Port>, String>,
    pub connections: Result<Vec<Connection>, String>,
    pub relations: Result<Vec<Relation>, String>,
    pub opc: Result<Vec<Opc>, String>,
    pub pipelines: Result<Pipelines, String>,
}

/// A code as text: nothing for none, its spelling when it has one, `#` and
/// the number when it has not.
fn code_text(code: Option<i64>, spell: impl Fn(i64) -> Option<String>) -> String {
    match code {
        None => String::new(),
        Some(code) => spell(code).unwrap_or_else(|| format!("#{code}")),
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Dump {
    /// Read the dump at `path`: the whole file, then every table the reader
    /// knows. A dump with a table the reader refuses is kept as its bytes,
    /// for the views that can do without that table.
    pub fn read(path: &Path) -> Result<Dump, String> {
        let started = Instant::now();
        let dump = std::fs::read(path)
            .map_err(|error| crate::tf!("cannot read {}: {}", path.display(), error))?;
        let bytes = dump.len() as u64;
        let tables = match ExpTables::read(&dump) {
            Ok(tables) => Tables::Whole(Box::new(tables)),
            Err(error) => Tables::Partial {
                dump,
                why: error.to_string(),
            },
        };
        Ok(Dump {
            tables,
            bytes,
            read_in: started.elapsed(),
            ports: OnceLock::new(),
            relations: OnceLock::new(),
            fluids: OnceLock::new(),
            item_tags: OnceLock::new(),
        })
    }

    /// Why the tables were not read whole, when they were not: the views
    /// that need the table the reader refused are missing.
    pub fn partial(&self) -> Option<&str> {
        match &self.tables {
            Tables::Whole(_) => None,
            Tables::Partial { why, .. } => Some(why),
        }
    }

    /// The tables, for a view that needs them all read.
    fn whole(&self) -> Result<&ExpTables, String> {
        match &self.tables {
            Tables::Whole(tables) => Ok(tables),
            Tables::Partial { why, .. } => Err(why.clone()),
        }
    }

    fn ports(&self) -> Result<&PortTables, String> {
        self.ports
            .get_or_init(|| match &self.tables {
                Tables::Whole(tables) => tables.port_tables().map_err(text),
                Tables::Partial { dump, .. } => read_port_tables(dump).map_err(text),
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn fluids(&self) -> Result<&HashMap<i64, String>, String> {
        self.fluids
            .get_or_init(|| self.whole()?.codelist_short_text(FLUID_CODES).map_err(text))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The tag the database gives each plant item that has one
    /// (`T_PlantItem.ItemTag`), by the item's id as [`backup_uid_key`]
    /// spells it. Worked out when first asked for, and kept.
    ///
    /// From the tables read whole only: a dump the reader could not follow
    /// answers with why it could not, not with the tags of the rows it did
    /// follow, and so does a dump that holds no such column.
    pub fn item_tags(&self) -> Result<Arc<HashMap<String, String>>, String> {
        self.item_tags
            .get_or_init(|| {
                let column = item_tag_column();
                let cells = self
                    .whole()?
                    .stored_column(&column)
                    .map_err(text)?
                    .ok_or_else(|| {
                        crate::tf!("the dump holds no {} column", column).into_owned()
                    })?;
                Ok(Arc::new(tags_of_cells(cells)))
            })
            .clone()
    }

    /// The pipe runs, each with the name its pipeline goes by.
    pub fn pipe_runs(&self) -> Result<Vec<PipeRun>, String> {
        let fluids = self.fluids()?;
        let runs = self.whole()?.pipe_runs().map_err(text)?;
        Ok(runs
            .iter()
            .map(|run| {
                let spelled = run.fluid_code.and_then(|code| fluids.get(&code));
                let pipeline = if run.tag_sequence.is_empty() {
                    String::new()
                } else {
                    // As the reader names a pipeline: no code keeps an
                    // empty slot, a code the list does not spell loses it.
                    let slot = match (run.fluid_code, spelled) {
                        (None, _) => Some(""),
                        (Some(_), Some(text)) => Some(text.as_str()),
                        (Some(_), None) => None,
                    };
                    pipeline_name(run, slot)
                };
                PipeRun {
                    id: run.id.clone(),
                    pipeline,
                    nominal: code_text(run.nominal_diameter, nominal_spelling),
                    fluid: code_text(run.fluid_code, |code| fluids.get(&code).cloned()),
                    materials_class: run.materials_class.clone(),
                    tag_sequence: run.tag_sequence.clone(),
                    fluid_system: code_text(run.fluid_system, |code| Some(code.to_string())),
                    insulation: run.insulation_spec.clone(),
                }
            })
            .collect())
    }

    /// The piping ports of every item, and how many rows the table has.
    pub fn piping_ports(&self) -> Result<(Vec<Port>, usize), String> {
        let tables = self.ports()?;
        Ok((
            tables.piping_points.iter().map(piping_port).collect(),
            tables.piping_row_count,
        ))
    }

    /// The signal ports of every item, and how many rows the table has.
    pub fn signal_ports(&self) -> Result<(Vec<Port>, usize), String> {
        let tables = self.ports()?;
        Ok((
            tables.signal_points.iter().map(signal_port).collect(),
            tables.signal_row_count,
        ))
    }

    /// The nozzles of the equipment and the notes of the items.
    pub fn relations(&self) -> Result<&[Relation], String> {
        self.relations
            .get_or_init(|| {
                let read = match &self.tables {
                    Tables::Whole(tables) => tables.composition_relations(),
                    Tables::Partial { dump, .. } => composition_relations(dump),
                };
                read.map(|relations| {
                    relations
                        .into_iter()
                        .map(|relation| Relation {
                            definition: relation.def_uid,
                            from_uid: relation.uid1,
                            to_uid: relation.uid2,
                            from: None,
                            to: None,
                        })
                        .collect()
                })
                .map_err(text)
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    /// The code lists, as far as the reader spells them.
    pub fn code_lists(&self) -> Result<CodeLists, String> {
        let tables = self.whole()?;
        let mut found = CodeLists {
            rows: tables.codelist_row_count(),
            probed_to: CODE_LISTS_PROBED,
            ..CodeLists::default()
        };
        for list in 1..=CODE_LISTS_PROBED {
            let texts = tables.codelist_short_text(list).map_err(text)?;
            if texts.is_empty() {
                continue;
            }
            found.lists += 1;
            let mut entries: Vec<(i64, String)> = texts.into_iter().collect();
            entries.sort();
            found
                .entries
                .extend(
                    entries
                        .into_iter()
                        .map(|(index, text)| CodeEntry { list, index, text }),
                );
        }
        Ok(found)
    }

    /// What the database knows of the drawing `items` were read from, in a
    /// plant called `plant`.
    pub fn of_drawing(&self, items: &DrawingItems, plant: &str) -> DrawingDump {
        // What the tree calls an item, by its UID as the database folds it.
        let labels: HashMap<String, &str> = items
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .map(|item| (backup_uid_key(&item.uid), item.label.as_str()))
            .collect();
        let label = |uid: &str| {
            labels
                .get(&backup_uid_key(uid))
                .map(|label| (*label).to_string())
        };

        // Ports hang on the items the drawing publishes with a class.
        let ports = self.ports().map(|tables| {
            let parents: HashSet<String> = items
                .groups
                .iter()
                .flat_map(|group| &group.items)
                .filter(|item| item.class.is_some())
                .map(|item| item.uid.clone())
                .collect();
            let found = ports_for_parents(tables, &parents);
            found
                .piping
                .iter()
                .map(piping_port)
                .chain(found.signal.iter().map(signal_port))
                .map(|port| Port {
                    item: label(&port.item_uid),
                    ..port
                })
                .collect()
        });

        let connections = match &self.tables {
            Tables::Whole(tables) => tables.connectivity_ends(&items.drawing_uid),
            Tables::Partial { dump, .. } => connectivity_ends_for_drawing(dump, &items.drawing_uid),
        };
        let connections = connections
            .map(|ends| {
                ends.into_iter()
                    .map(|end| Connection {
                        run: label(&end.run_uid),
                        symbol_uid: end.symbol_uid,
                        run_uid: end.run_uid,
                        on_start: end.on_start,
                        on_end: end.on_end,
                        definition: end.def_uid,
                    })
                    .collect()
            })
            .map_err(text);

        // A relation is the drawing's when either end is one of its items.
        let relations = self.relations().map(|relations| {
            relations
                .iter()
                .filter(|relation| {
                    labels.contains_key(&backup_uid_key(&relation.from_uid))
                        || labels.contains_key(&backup_uid_key(&relation.to_uid))
                })
                .map(|relation| Relation {
                    from: label(&relation.from_uid),
                    to: label(&relation.to_uid),
                    ..relation.clone()
                })
                .collect()
        });

        let opc = match &self.tables {
            Tables::Whole(tables) => tables.opc_points(&items.drawing_uid),
            Tables::Partial { dump, .. } => opc_points_for_drawing(dump, &items.drawing_uid),
        };
        let opc = opc
            .map(|points| {
                points
                    .into_iter()
                    .map(|point| Opc {
                        uid: point.uid,
                        representation_uid: point.representation_uid,
                        name: point.name,
                        description: point.description,
                        to_from_text: point.to_from_text,
                    })
                    .collect()
            })
            .map_err(text);

        DrawingDump {
            ports,
            connections,
            relations,
            opc,
            pipelines: self.pipelines(items, plant, &label),
        }
    }

    /// The pipelines of a drawing, named as the reader names them: it works
    /// on a drawing as it was read, of which the piping connectors are all
    /// it looks at.
    fn pipelines(
        &self,
        items: &DrawingItems,
        plant: &str,
        label: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Pipelines, String> {
        let mut connectors: Vec<&str> = items
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .filter(|item| item.class.as_deref() == Some(PIPING_CONNECTOR))
            .map(|item| item.uid.as_str())
            .collect();
        // The order the reader has them in: by UID.
        connectors.sort_unstable();
        let mut drawing = PidPublishDrawing {
            drawing_uid: items.drawing_uid.clone(),
            drawing_name: items.drawing_name.clone(),
            drawing_attributes: Vec::new(),
            items: connectors
                .iter()
                .map(|uid| PidPublishItem {
                    uid: (*uid).to_string(),
                    model_item_type: "PipeRun".to_string(),
                    class: Some(PIPING_CONNECTOR),
                    attributes: Vec::new(),
                })
                .collect(),
            process_points: Vec::new(),
            branch_points: Vec::new(),
            ports: Vec::new(),
            connectivity_ends: Vec::new(),
            exp_relations: Vec::new(),
            opc_points: Vec::new(),
            exp: None,
            pipelines: Vec::new(),
            pipelines_attempted: false,
            pipeline_synthesis: PipelineSynthesis::default(),
            representations: Vec::new(),
            stats: PidPublishStats::default(),
        };
        attach_pipelines(&mut drawing, self.whole()?, plant, None).map_err(text)?;
        let synthesis = drawing.pipeline_synthesis;
        Ok(Pipelines {
            rows: drawing
                .pipelines
                .into_iter()
                .map(|pipeline| Pipeline {
                    uid: pipeline.uid,
                    name: pipeline.name,
                    connectors: pipeline
                        .connector_uids
                        .iter()
                        .map(|uid| label(uid).unwrap_or_else(|| uid.clone()))
                        .collect(),
                })
                .collect(),
            connectors: connectors.len(),
            without_sequence: synthesis.without_sequence,
            fluid_code_missing: synthesis.fluid_code_missing,
            not_in_piperun: synthesis.not_in_piperun,
        })
    }
}

fn piping_port(point: &pid_parse::publish::oracle_exp::PipingPoint) -> Port {
    Port {
        uid: point.uid(),
        item_uid: point.plant_item_id.clone(),
        item: None,
        class: "PIDPipingPort",
        number: point.number,
        flow: code_text(point.flow_direction, |code| Some(code.to_string())),
        nominal: code_text(point.nominal_diameter, nominal_spelling),
    }
}

fn signal_port(point: &pid_parse::publish::oracle_exp::SignalPoint) -> Port {
    Port {
        uid: point.uid(),
        item_uid: point.plant_item_id.clone(),
        item: None,
        class: "PIDSignalPort",
        number: point.number,
        flow: String::new(),
        nominal: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pid_backup_browse::{build_index, load_items, BackupFile};

    thread_local! {
        /// How many dumps this thread has set out to read
        /// ([`BackupIndex::dump`] counts them).
        pub(super) static DUMPS_READ_HERE: std::cell::Cell<usize> =
            const { std::cell::Cell::new(0) };
    }

    #[test]
    fn a_code_is_its_spelling_or_its_number() {
        assert_eq!(code_text(None, nominal_spelling), "");
        assert_eq!(code_text(Some(25), nominal_spelling), "25 mm");
        assert_eq!(code_text(Some(5032), nominal_spelling), "1 in");
        // A code off the reader's grid is shown as the code it is.
        assert_eq!(code_text(Some(5001), nominal_spelling), "#5001");
        assert_eq!(code_text(Some(7), |_| None), "#7");
    }

    fn dump_file(version_line: Option<&str>) -> BackupFile {
        BackupFile {
            name: "Export.dmp".to_string(),
            kind: FileKind::ExportDump,
            version_line: version_line.map(str::to_string),
            ..BackupFile::default()
        }
    }

    #[test]
    fn a_backup_without_an_oracle_dump_says_why_its_database_is_not_read() {
        let none = BackupIndex::default();
        assert_eq!(
            none.dump_unreadable(),
            Some(crate::t!("This backup has no database dump.").into_owned())
        );
        assert_eq!(none.dump().err(), none.dump_unreadable());
        // A dump the export named and did not write is no dump.
        let missing = BackupIndex {
            files: vec![BackupFile {
                missing: true,
                ..dump_file(Some("EXPORT:V12.01.00"))
            }],
            ..BackupIndex::default()
        };
        assert_eq!(missing.dump_unreadable(), none.dump_unreadable());
        // A SQL Server dump is there, and is not read.
        let sql_server = BackupIndex {
            files: vec![dump_file(None)],
            ..BackupIndex::default()
        };
        let why =
            crate::t!("The dump is not an Oracle export; the panel does not read its database.")
                .into_owned();
        assert_eq!(sql_server.dump_unreadable(), Some(why.clone()));
        assert_eq!(sql_server.dump().err(), Some(why));
        assert!(sql_server.dump_if_read().is_none());
        // An Oracle dump can be read; whether it reads is the read's to say.
        let oracle = BackupIndex {
            files: vec![dump_file(Some("EXPORT:V12.01.00"))],
            backup_root: std::env::temp_dir().join("ocs-no-such-backup"),
            ..BackupIndex::default()
        };
        assert_eq!(oracle.dump_unreadable(), None);
        let failed = oracle.dump().err().expect("there is no such file");
        assert!(failed.contains("Export.dmp"), "{failed}");
        // The failure is the answer from then on, and a copy has it too.
        assert_eq!(oracle.clone().dump().err(), Some(failed));
        assert_eq!(format!("{:?}", oracle.dump), "DumpCell(unreadable)");
    }

    /// A tag is the text of its cell without the blanks around it; a cell
    /// that is all blanks is no tag, nor is a cell that is no text.
    #[test]
    fn a_tag_is_the_text_of_its_cell() {
        let cells: HashMap<String, StoredCell> = [
            ("A", StoredCell::Text("GV-101".to_string())),
            ("B", StoredCell::Text("  PIA-0112101B \t".to_string())),
            ("C", StoredCell::Text("   ".to_string())),
            ("D", StoredCell::Integer(7)),
            ("E", StoredCell::Text("P 101 A".to_string())),
        ]
        .into_iter()
        .map(|(id, cell)| (id.to_string(), cell))
        .collect();
        let mut tags: Vec<(String, String)> = tags_of_cells(cells).into_iter().collect();
        tags.sort();
        assert_eq!(
            tags,
            [
                ("A".to_string(), "GV-101".to_string()),
                ("B".to_string(), "PIA-0112101B".to_string()),
                ("E".to_string(), "P 101 A".to_string()),
            ]
        );
    }

    /// A dump the reader could not follow gives no tags: it says why it was
    /// not followed, and does not give the tags of the tables that were
    /// read (plan 2026-11-20-pid-symbol-identity, SPEC section 4.4).
    #[test]
    fn a_dump_not_read_whole_gives_no_item_tags() {
        let why = "the reader could not follow T_REPRESENTATION".to_string();
        let dump = Dump {
            tables: Tables::Partial {
                dump: Vec::new(),
                why: why.clone(),
            },
            bytes: 0,
            read_in: Duration::ZERO,
            ports: OnceLock::new(),
            relations: OnceLock::new(),
            fluids: OnceLock::new(),
            item_tags: OnceLock::new(),
        };
        assert_eq!(dump.partial(), Some(why.as_str()));
        assert_eq!(dump.item_tags(), Err(why.clone()));
        // The answer is kept: asked again, it is the same.
        assert_eq!(dump.item_tags(), Err(why));
    }

    /// The tags are read on a thread of their own, under its own name, not
    /// on the thread that asks (SPEC section 4.4: the UI's). A thread that
    /// is gone before it answers is a read that failed. A dump that cannot
    /// be opened answers with why, and it is that thread it was read on:
    /// the thread that asked for the tags read no dump.
    #[test]
    fn the_item_tags_are_read_on_a_thread_of_their_own() {
        use iced::futures::executor::block_on;

        let named = || {
            let thread = std::thread::current();
            thread.name().unwrap_or_default().to_string()
        };
        let on = block_on(on_tags_thread(move || Ok(named())));
        assert_eq!(on, Ok("ocs-pid-backup-tags".to_string()));
        assert_ne!(named(), "ocs-pid-backup-tags");

        let gone: Result<(), String> =
            block_on(on_tags_thread(|| panic!("the reading thread died")));
        assert_eq!(
            gone,
            Err(crate::t!("the backup data read stopped before it finished").into_owned())
        );

        let read_here = || DUMPS_READ_HERE.with(std::cell::Cell::get);
        let unread = || {
            Arc::new(BackupIndex {
                files: vec![dump_file(Some("EXPORT:V12.01.00"))],
                backup_root: std::env::temp_dir().join("ocs-no-such-backup"),
                ..BackupIndex::default()
            })
        };
        let before = read_here();
        let index = unread();
        assert_eq!(format!("{:?}", index.dump), "DumpCell(not read)");
        let failed = block_on(item_tags_in_background(Arc::clone(&index)))
            .expect_err("there is no such file");
        assert!(failed.contains("Export.dmp"), "{failed}");
        assert_eq!(format!("{:?}", index.dump), "DumpCell(unreadable)");
        assert_eq!(read_here(), before, "read, and not on this thread");
        // A dump this thread reads is counted here.
        assert!(unread().dump().is_err());
        assert_eq!(read_here(), before + 1);
    }

    /// The plant database of the test backup gives 8,145 of its items a tag
    /// (plan 2026-11-20-pid-symbol-identity, SPEC section 2.2). Prints what
    /// the column holds and how long the tags take. Soft-skips when the
    /// backup is not on this machine.
    #[test]
    #[ignore = "reads the 74 MB plant dump; run with --ignored"]
    fn the_plant_database_gives_its_items_their_tags() {
        let backup = Path::new(r"D:\work\plant-code\cad\pid-test-data");
        if !backup.join("Export.dmp").is_file() {
            eprintln!("skipped: no plant backup at {}", backup.display());
            return;
        }
        let index = build_index(backup, None).expect("the backup indexes");
        let dump = index.dump().expect("an Oracle dump reads");
        assert_eq!(dump.partial(), None, "the tables are read whole");
        let started = Instant::now();
        let tags = dump.item_tags().expect("the item tags");
        let took = started.elapsed();
        // A second ask is the same table, at once.
        assert!(Arc::ptr_eq(&tags, &dump.item_tags().unwrap()));

        // What the column itself holds.
        let cells = dump
            .whole()
            .unwrap()
            .stored_column(&item_tag_column())
            .expect("the column reads")
            .expect("the dump holds the column");
        let texts: Vec<&str> = cells
            .values()
            .filter_map(|cell| match cell {
                StoredCell::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let blank = texts.iter().filter(|text| text.trim().is_empty()).count();
        let padded = texts
            .iter()
            .filter(|text| !text.trim().is_empty() && text.trim() != **text)
            .count();
        eprintln!(
            "T_PlantItem.ItemTag: {} cell(s) stored, {} of them text, {} blank, {} with blanks around the tag; {} tag(s) worked out in {:.1} ms",
            cells.len(),
            texts.len(),
            blank,
            padded,
            tags.len(),
            took.as_secs_f64() * 1000.0
        );
        assert_eq!(tags.len(), 8145);
        assert_eq!(tags.len(), texts.len() - blank);
        assert!(tags.iter().all(|(id, tag)| {
            *id == backup_uid_key(id) && !tag.is_empty() && tag.trim() == tag.as_str()
        }));
    }

    /// The private bytes of this process, in MiB.
    #[cfg(windows)]
    fn private_mib() -> f64 {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        let mut counters = Counters::default();
        let size = std::mem::size_of::<Counters>() as u32;
        counters.cb = size;
        // SAFETY: the struct is the documented PROCESS_MEMORY_COUNTERS, and
        // the handle is the pseudo handle of this process.
        let read = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
        if read == 0 {
            return 0.0;
        }
        counters.pagefile_usage as f64 / (1024.0 * 1024.0)
    }

    #[cfg(not(windows))]
    fn private_mib() -> f64 {
        0.0
    }

    /// How long the dumps of the test backups take to read, what they cost
    /// in memory, and what the views of them hold. Prints; asserts only
    /// what any Oracle dump of a plant must give.
    #[test]
    #[ignore = "reads real SmartPlant backups; run alone with --ignored --nocapture"]
    fn the_dumps_of_the_test_backups_are_read_and_measured() {
        let fixtures = Path::new(r"D:\work\plant-code\cad\pid-parse\test-file\backup-test");
        for backup in [
            Path::new(r"D:\work\plant-code\cad\pid-test-data").to_path_buf(),
            fixtures.join("DWG-0202GP06-01_p"),
        ] {
            if !backup.join("Export.dmp").is_file() {
                eprintln!("skipped: no dump in {}", backup.display());
                continue;
            }
            let index = build_index(&backup, None).expect("the backup indexes");
            let before = private_mib();
            let started = Instant::now();
            let dump = index.dump().expect("an Oracle dump reads");
            let read = started.elapsed();
            let after = private_mib();
            eprintln!(
                "{}: {} bytes read in {:.2} s (the reader says {:.2} s); private memory {:.0} -> {:.0} MiB (+{:.0})",
                backup.display(),
                dump.bytes,
                read.as_secs_f64(),
                dump.read_in.as_secs_f64(),
                before,
                after,
                after - before
            );
            // A second ask is the same dump, at once.
            let started = Instant::now();
            assert!(Arc::ptr_eq(&dump, &index.dump().unwrap()));
            assert!(started.elapsed() < Duration::from_millis(50));

            match dump.partial() {
                None => eprintln!("  every table the reader knows was read"),
                Some(why) => eprintln!("  not read whole: {why}"),
            }
            // A view with its rows and its time, or why it is not there.
            fn shown<T>(
                what: &str,
                started: Instant,
                view: Result<T, String>,
                rows: impl Fn(&T) -> usize,
            ) -> Option<T> {
                let took = started.elapsed().as_secs_f64() * 1000.0;
                match view {
                    Ok(view) => {
                        eprintln!("  {what}: {} row(s) in {took:.0} ms", rows(&view));
                        Some(view)
                    }
                    Err(why) => {
                        eprintln!("  {what}: not there ({why})");
                        None
                    }
                }
            }
            let whole = dump.partial().is_none();
            let started = Instant::now();
            let runs = shown("pipe runs", started, dump.pipe_runs(), Vec::len);
            if let Some(runs) = &runs {
                let named = runs.iter().filter(|run| !run.pipeline.is_empty()).count();
                let unspelled = runs
                    .iter()
                    .filter(|run| run.nominal.starts_with('#') || run.fluid.starts_with('#'))
                    .count();
                eprintln!("    {named} belong to a pipeline; {unspelled} carry a code that is not spelled");
            }
            assert_eq!(runs.is_some(), whole);
            let started = Instant::now();
            let piping = shown(
                "piping ports",
                started,
                dump.piping_ports(),
                |(ports, _)| ports.len(),
            );
            let started = Instant::now();
            let signal = shown(
                "signal ports",
                started,
                dump.signal_ports(),
                |(ports, _)| ports.len(),
            );
            if let (Some((_, piping_rows)), Some((_, signal_rows))) = (&piping, &signal) {
                eprintln!("    of {piping_rows} and {signal_rows} table row(s)");
            }
            // The ports are read whether the rest is or not.
            assert!(piping.is_some_and(|(ports, _)| !ports.is_empty()));
            let started = Instant::now();
            let relations = shown(
                "relations",
                started,
                dump.relations().map(<[Relation]>::to_vec),
                Vec::len,
            );
            if let Some(relations) = &relations {
                let nozzles = relations
                    .iter()
                    .filter(|relation| relation.definition == "EquipmentComponentComposition")
                    .count();
                eprintln!(
                    "    {nozzles} nozzle(s), {} note(s)",
                    relations.len() - nozzles
                );
            }
            let started = Instant::now();
            let lists = shown("code lists", started, dump.code_lists(), |lists| {
                lists.entries.len()
            });
            if let Some(lists) = &lists {
                eprintln!(
                    "    {} list(s), the highest {}; the table has {} row(s)",
                    lists.lists,
                    lists.entries.last().map_or(0, |entry| entry.list),
                    lists.rows
                );
                assert!(lists.entries.len() <= lists.rows);
                // The probe reaches well past the highest list there is.
                assert!(lists
                    .entries
                    .last()
                    .is_some_and(|entry| entry.list * 2 < CODE_LISTS_PROBED));
            }
            assert_eq!(lists.is_some(), whole);
            eprintln!(
                "  private memory with the views worked out: {:.0} MiB",
                private_mib()
            );

            // Every drawing: what the database knows of it.
            let plant = index
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.name.clone())
                .unwrap_or_default();
            // Rows of each view over the drawings, and drawings it failed for.
            let mut totals = [0usize; 5];
            let mut failed = [0usize; 5];
            let mut first_failure: [Option<String>; 5] = Default::default();
            let mut slowest = (Duration::ZERO, String::new());
            let mut read_drawings = 0;
            let started_all = Instant::now();
            for drawing in &index.drawings {
                let Some(path) = &drawing.pid_path else {
                    continue;
                };
                let Ok(items) = load_items(path) else {
                    continue;
                };
                read_drawings += 1;
                let started = Instant::now();
                let view = dump.of_drawing(&items, &plant);
                let took = started.elapsed();
                if took > slowest.0 {
                    slowest = (took, drawing.name.clone());
                }
                let counts = [
                    view.ports.map(|rows| rows.len()),
                    view.connections.map(|rows| rows.len()),
                    view.relations.map(|rows| rows.len()),
                    view.opc.map(|rows| rows.len()),
                    view.pipelines.map(|pipelines| pipelines.rows.len()),
                ];
                for (at, count) in counts.into_iter().enumerate() {
                    match count {
                        Ok(count) => totals[at] += count,
                        Err(why) => {
                            failed[at] += 1;
                            first_failure[at].get_or_insert(why);
                        }
                    }
                }
            }
            eprintln!(
                "  {read_drawings} drawing(s) in {:.1} s with their files read: {} port(s), {} connection(s), {} relation(s), {} off-page connector(s), {} pipeline(s); slowest view {} ({:.0} ms)",
                started_all.elapsed().as_secs_f64(),
                totals[0],
                totals[1],
                totals[2],
                totals[3],
                totals[4],
                slowest.1,
                slowest.0.as_secs_f64() * 1000.0
            );
            for (what, (failed, why)) in [
                "ports",
                "connections",
                "relations",
                "off-page connectors",
                "pipelines",
            ]
            .iter()
            .zip(failed.iter().zip(&first_failure))
            {
                if let Some(why) = why {
                    eprintln!("    {what}: not there for {failed} drawing(s) ({why})");
                }
            }
            assert!(totals[0] > 0);
            if whole {
                assert_eq!(failed, [0; 5]);
                assert!(totals[1] > 0);
            }
        }
    }
}

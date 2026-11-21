//! What a SmartPlant P&ID backup holds, in one index the browser panel draws.
//!
//! [`crate::io::pid_backup::restore`] makes a backup browsable; this module
//! then reads it once: the `Manifest.txt` summary, the backup files (with the
//! member lists of the archives), the plant-group tree, the `T_DRAWING` rows
//! matched to the `.pid` files the restore produced, and the `Ref` reference
//! data tree.
//!
//! The database side is optional. A backup whose `Export.dmp` is absent, is a
//! SQL Server MTF stream, or cannot be read still lists its drawings -- by the
//! folder they were unpacked into -- and says so in [`BackupIndex::warnings`].
//! A `T_DRAWING` row whose file is not on disk is kept, with
//! [`DrawingEntry::pid_path`] empty, so the panel can show it as missing rather
//! than hide a drawing the plant knows about. A `.pid` the table does not
//! mention is listed too, under [`UNREGISTERED_GROUP`], so nothing the backup
//! carries is lost from the tree.
//!
//! Matching a row to a file is [`match_drawings`]: a pure function over the
//! rows and the `.pid` paths, testable without an Oracle dump.
//!
//! What a drawing holds -- its model items and their published attributes,
//! and what its file states beside them ([`drawing_data`]) -- is
//! [`load_items`], read on demand when the panel first expands a drawing.

pub mod datamap;
pub mod diagnostics;
pub mod drawing_data;
pub mod exp_views;
pub mod export_log;
pub mod labels;
pub mod loose_files;
pub mod manifest;
pub mod symbol_file;
pub mod text_file;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pid_parse::backup::{
    scan_backup_folder, BackupFolderScan, ExportDumpFormat, ExportDumpScan, ZipListing,
};
use pid_parse::publish::oracle_exp::{
    backup_path_key, plant_group_path, read_backup_drawings, read_plant_groups, BackupDrawing,
};
use pid_parse::publish::pid_load::{
    load_publish_from_pid, PidLoadError, PidPublishDrawing, PidPublishItem,
};

/// Group the drawings the table does not mention are filed under.
pub const UNREGISTERED_GROUP: &str = "Unregistered";

/// Group a drawing with no folder or plant group is filed under.
pub const UNGROUPED: &str = "(ungrouped)";

/// How deep the `Ref` tree is walked before a runaway directory is cut off.
const REF_SEARCH_DEPTH: usize = 16;

/// The manifest's schema number for reference data (`RefData~4~…`).
const REFDATA_SCHEMA: u32 = 4;

/// One backup, indexed for the browser panel.
#[derive(Debug, Clone, Default)]
pub struct BackupIndex {
    /// The zip or folder the user picked.
    pub source: PathBuf,
    /// The folder the restore produced (a loose-drawing folder source is its
    /// own restore folder).
    pub restored: PathBuf,
    /// The folder holding `Manifest.txt` / `Export.dmp` / `PlantData~*`.
    pub backup_root: PathBuf,
    /// `true` when the backup is a folder used where it lies: never written.
    pub read_only: bool,
    /// The `Manifest.txt` singleton fields the panel summarises.
    pub manifest: Option<ManifestSummary>,
    /// `Manifest.txt`, `Export.dmp`, every `RefData~*` and `PlantData~*`.
    pub files: Vec<BackupFile>,
    /// The plant-group tree; every drawing index appears under one group.
    pub plant_groups: Vec<GroupNode>,
    /// One entry per `T_DRAWING` row plus every unmatched `.pid`.
    pub drawings: Vec<DrawingEntry>,
    /// The `Ref` reference-data tree.
    pub ref_tree: RefNode,
    /// Everything the panel should tell the user went wrong or was skipped.
    pub warnings: Vec<String>,
    /// All of `Manifest.txt`, sorted for the panel, its credentials hidden.
    pub manifest_model: Option<manifest::ManifestModel>,
    /// The folder [`BackupIndex::ref_tree`] was read from.
    pub ref_root: PathBuf,
    /// The tables of the database the dump holds, with their row counts
    /// when the backup carries its export log.
    pub table_catalog: Option<export_log::TableCatalog>,
    /// The dump's own tables, read the first time something asks for them
    /// ([`BackupIndex::dump`]).
    pub dump: exp_views::DumpCell,
    /// How far a check of every drawing has come, while one runs
    /// ([`diagnostics::check_all`]).
    pub check_progress: diagnostics::CheckProgress,
}

impl BackupIndex {
    /// Where file `file` of the `Ref` directory at `dirs` -- the child
    /// indices from the tree's root -- is on disk.
    pub fn ref_file_path(&self, dirs: &[usize], file: usize) -> Option<PathBuf> {
        let mut node = &self.ref_tree;
        let mut path = self.ref_root.clone();
        for at in dirs {
            node = node.dirs.get(*at)?;
            path.push(&node.name);
        }
        path.push(&node.files.get(file)?.name);
        Some(path)
    }
}

/// The `Manifest.txt` fields a backup always carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSummary {
    pub name: Option<String>,
    pub version: Option<String>,
    pub date_created: Option<String>,
    pub backup_type: Option<String>,
    pub spid: Option<String>,
}

/// One file in the backup root, with its format and archive members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupFile {
    /// File name as it appears on disk.
    pub name: String,
    /// What the file is: `Manifest`, `Export.dmp`, `RefData`, `PlantData`,
    /// or a file the export left beside them.
    pub kind: FileKind,
    /// Human-readable format, e.g. `Oracle exp (EXPORT:V12.01.00)`.
    pub format: String,
    /// Length in bytes.
    pub size: u64,
    /// Member names when the file is a zip; empty otherwise.
    pub members: Vec<String>,
    /// The `EXPORT:V…` line for an Oracle `exp` dump.
    pub version_line: Option<String>,
    /// `true` for an archive `Manifest.txt` names but the backup does not
    /// carry: the export logged the file as failed and wrote nothing.
    pub missing: bool,
    /// The manifest's `File` line for this file: what it was on the plant's
    /// server before the backup stored it under a number.
    pub record: Option<manifest::FileRecord>,
}

/// Which of the backup's file families a [`BackupFile`] belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileKind {
    #[default]
    Manifest,
    ExportDump,
    RefData,
    PlantData,
    /// A file of the backup folder that is none of the above: the dump's
    /// log, the plant's configuration (see [`loose_files`]).
    Other,
}

/// One node of the plant-group (or, degraded, folder) tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupNode {
    /// Group name alone, e.g. `00`.
    pub name: String,
    /// Full path from the root, e.g. `SQPlant / 00 / 00`.
    pub path: String,
    /// Nested groups.
    pub children: Vec<GroupNode>,
    /// Indices into [`BackupIndex::drawings`] filed directly here.
    pub drawings: Vec<usize>,
}

/// One drawing of the backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingEntry {
    /// The name the table (or the file) gives the drawing.
    pub name: String,
    /// The restored `.pid`, or `None` when the row's file is missing.
    pub pid_path: Option<PathBuf>,
    /// `plant_group_path` of its group, or its folder when degraded.
    pub group_path: String,
    /// Every `T_DRAWING` column, empty values included.
    pub meta: Vec<(String, String)>,
}

impl DrawingEntry {
    /// Whether the drawing has a file to open.
    pub fn is_missing(&self) -> bool {
        self.pid_path.is_none()
    }
}

/// The `Ref` reference-data tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefNode {
    /// Directory or (for the root) `Ref` name.
    pub name: String,
    /// Subdirectories, name-sorted.
    pub dirs: Vec<RefNode>,
    /// Files directly in this directory, name-sorted.
    pub files: Vec<RefFile>,
}

impl RefNode {
    /// Every file in this subtree, depth first.
    pub fn file_count(&self) -> usize {
        self.files.len() + self.dirs.iter().map(RefNode::file_count).sum::<usize>()
    }
}

/// One file in the `Ref` tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefFile {
    pub name: String,
    pub size: u64,
}

/// A `T_DRAWING` row reduced to what matching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingRow {
    pub name: String,
    /// [`backup_path_key`] of the row's `PATH`.
    pub path_key: String,
    /// `plant_group_path` of the row's group.
    pub group_path: String,
    /// Every `T_DRAWING` column, empty values included.
    pub meta: Vec<(String, String)>,
}

/// [`match_drawings`]'s outcome: the rows in order, and the `.pid` files no
/// row claimed.
#[derive(Debug, Clone, Default)]
pub struct MatchedDrawings {
    pub entries: Vec<DrawingEntry>,
    pub unmatched: Vec<PathBuf>,
}

/// Build the index of `source` -- a backup zip or folder -- restoring it into
/// `target` (default [`crate::io::pid_backup::restore_cache`]) when it has to
/// be unpacked.
pub fn build_index(source: &Path, target: Option<&Path>) -> Result<BackupIndex, String> {
    build_index_in(source, target, None)
}

/// [`build_index`] with the restore cache in `cache_root` instead of the
/// default folder.
pub fn build_index_in(
    source: &Path,
    target: Option<&Path>,
    cache_root: Option<&Path>,
) -> Result<BackupIndex, String> {
    let restored = crate::io::pid_backup::restore_in(source, target, cache_root)?;
    let read_only = source.is_dir();
    let backup_root = if read_only && is_backup_folder(source) {
        source.to_path_buf()
    } else {
        find_backup_root(&restored.root)
    };
    // Nothing of a backup, here or below, and no drawing: there is nothing
    // to list, and what was unpacked to find that out is not kept.
    if !is_backup_folder(&backup_root) && restored.drawings.is_empty() {
        discard_restore(source, target.is_some(), &restored);
        return Err(not_a_backup(source));
    }
    let scan = scan_backup_folder(&backup_root)
        .map_err(|error| crate::tf!("cannot scan {}: {}", backup_root.display(), error))?;

    let mut warnings = Vec::new();
    let mut files = describe_files(&backup_root, &scan, &mut warnings);
    let manifest = scan.manifest.as_ref().map(manifest_summary);
    let manifest_model = scan.manifest.as_ref().map(manifest::manifest_model);
    if let Some(model) = &manifest_model {
        attach_file_records(&mut files, model);
    }
    // What the export left beside its numbered files.
    let loose = loose_files::loose_files(&backup_root, &files);
    let at = loose_files::insert_at(&files);
    files.splice(at..at, loose);
    // The tables of the database: the manifest's names, the log's counts.
    let export_log = read_export_log(&backup_root);
    let table_catalog = export_log::table_catalog(
        manifest_model
            .as_ref()
            .map_or(&[][..], |model| model.tables.as_slice()),
        export_log.as_ref(),
    );

    let mut drawings = Vec::new();
    let mut group_paths = Vec::new();
    match dump_drawings(&backup_root, &scan, &mut warnings) {
        Some(rows) => {
            let (entries, unmatched) =
                assemble_drawings(&rows, &restored.drawings, &backup_root, &restored.root);
            group_paths.extend(entries.iter().map(|entry| entry.group_path.clone()));
            drawings.extend(entries);
            if !unmatched.is_empty() {
                warnings.push(
                    crate::tf!(
                        "{} .pid file(s) are not in the drawing table; they are listed under \"{}\"",
                        unmatched.len(),
                        crate::t!(UNREGISTERED_GROUP)
                    )
                    .into_owned(),
                );
                for entry in &unmatched {
                    group_paths.push(entry.group_path.clone());
                }
                drawings.extend(unmatched);
            }
        }
        None => {
            let entries = folder_entries(&restored.drawings, &backup_root, &restored.root);
            group_paths.extend(entries.iter().map(|entry| entry.group_path.clone()));
            drawings.extend(entries);
        }
    }
    let mut plant_groups = build_group_tree(&group_paths);
    sort_group_drawings(&mut plant_groups, &drawings);
    // The restore's own `Ref` is the whole of it: the backup's reference
    // data volumes unpacked, and what a `Ref` folder beside the backup adds.
    // A folder used where it lies has only what lies beside it.
    let restored_ref = restored.root.join(crate::io::pid_backup::REFERENCE_DIR);
    let ref_root = if restored.extracted && restored_ref.is_dir() {
        restored_ref
    } else {
        backup_root.join(crate::io::pid_backup::REFERENCE_DIR)
    };
    let ref_tree = read_ref_tree(&ref_root);

    Ok(BackupIndex {
        source: source.to_path_buf(),
        restored: restored.root,
        backup_root,
        read_only,
        manifest,
        files,
        plant_groups,
        drawings,
        ref_tree,
        warnings,
        manifest_model,
        ref_root,
        table_catalog,
        dump: exp_views::DumpCell::default(),
        check_progress: diagnostics::CheckProgress::default(),
    })
}

/// `Export.log` of the backup folder, when it is there and reads as text.
fn read_export_log(backup_root: &Path) -> Option<export_log::ExportLog> {
    let path = backup_root.join("Export.log");
    if !path.is_file() {
        return None;
    }
    match text_file::read_text_file(&path) {
        Ok(text_file::Readable::Text(file)) => Some(export_log::parse_export_log(
            file.lines().iter().map(String::as_str),
        )),
        _ => None,
    }
}

/// Tell each backup file what the manifest says it is. A single file stored
/// under a number (`RefData~4~703` is `exportlayer.xlsx`) takes its format
/// from its real name, and has no members: what a zip listing finds inside
/// an `.xlsx` are the parts of one document, not files of the backup.
fn attach_file_records(files: &mut [BackupFile], model: &manifest::ManifestModel) {
    for record in &model.files {
        let name = record
            .stored_name()
            .map(str::to_string)
            .unwrap_or_else(|| record.expected_name());
        let Some(file) = files
            .iter_mut()
            .find(|file| file.name.eq_ignore_ascii_case(&name))
        else {
            continue;
        };
        if record.backed_up && record.kind == manifest::FileRecordKind::SingleFile {
            if let Some(format) = manifest::format_of(&record.original) {
                file.format = format;
            }
            file.members.clear();
        }
        file.record = Some(record.clone());
    }
}

/// Whether `dir` looks like the folder of a backup: it holds the manifest, the
/// dump, or the drawing archives.
pub fn is_backup_folder(dir: &Path) -> bool {
    if dir.join("Manifest.txt").is_file() || dir.join("Export.dmp").is_file() {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("PlantData~")
        })
    })
}

/// The layer of a restore folder that holds the backup itself: the folder
/// itself when it is one, else the first folder (two levels down) that is.
pub fn find_backup_root(root: &Path) -> PathBuf {
    if is_backup_folder(root) {
        return root.to_path_buf();
    }
    let mut level: Vec<PathBuf> = vec![root.to_path_buf()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut children: Vec<PathBuf> = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect();
            children.sort();
            for child in children {
                if is_backup_folder(&child) {
                    return child;
                }
                next.push(child);
            }
        }
        level = next;
    }
    root.to_path_buf()
}

/// What a source with nothing to list is refused with: it holds nothing of a
/// backup ([`is_backup_folder`], in it or two levels down) and no drawing.
fn not_a_backup(source: &Path) -> String {
    crate::tf!(
        "{} is not a SmartPlant P&ID backup: no Manifest.txt, Export.dmp or PlantData~ archive in it, and no .pid drawing",
        source.display()
    )
    .into_owned()
}

/// Take away the restore of a source that was refused, so that nothing of
/// what is no backup stays unpacked. Only a copy that was unpacked apart
/// from the source goes: a folder used where it lies is the source itself.
/// In the cache the restore goes whether this call made it or an earlier one
/// left it there -- the cache is this module's own. In a folder the user
/// `named` for the restore only what this call made goes: that folder was
/// new or empty before, and it goes whole, as when a restore fails half way.
/// A restore that was kept there from an earlier call is theirs, and stays.
fn discard_restore(source: &Path, named: bool, restored: &crate::io::pid_backup::Restored) {
    if !restored.extracted || (named && restored.reused) {
        return;
    }
    // Never the source, nor a folder the source lies in. Paths that cannot
    // be resolved are not told apart, and then nothing is removed.
    let apart = match (
        std::fs::canonicalize(source),
        std::fs::canonicalize(&restored.root),
    ) {
        (Ok(source), Ok(root)) => !source.starts_with(root),
        _ => false,
    };
    if !apart {
        return;
    }
    if let Err(error) = std::fs::remove_dir_all(&restored.root) {
        log::warn!(
            "{}: the restore of a source that is no backup could not be removed: {error}",
            restored.root.display()
        );
    }
}

fn manifest_summary(manifest: &pid_parse::backup::Manifest) -> ManifestSummary {
    let field = |key: &str| manifest.first_field(key).map(str::to_string);
    ManifestSummary {
        name: field("Name"),
        version: field("Version"),
        date_created: field("DateCreated"),
        backup_type: field("BackupType"),
        spid: field("Spid"),
    }
}

/// The backup files, in the panel's order: manifest, dump, then the archives.
fn describe_files(
    backup_root: &Path,
    scan: &BackupFolderScan,
    warnings: &mut Vec<String>,
) -> Vec<BackupFile> {
    let mut files = Vec::new();
    if scan.manifest.is_some() {
        let path = backup_root.join("Manifest.txt");
        files.push(BackupFile {
            name: "Manifest.txt".to_string(),
            kind: FileKind::Manifest,
            format: "Manifest".to_string(),
            size: file_size(&path),
            members: Vec::new(),
            version_line: None,
            missing: false,
            ..BackupFile::default()
        });
    }
    if let Some(export) = &scan.export {
        files.push(export_file(export));
    }
    for entry in &scan.refdata {
        let (format, members) = zip_parts(&entry.zip);
        files.push(BackupFile {
            name: entry.entry.file_name.clone(),
            kind: FileKind::RefData,
            format,
            size: entry.entry.size,
            members,
            version_line: None,
            missing: false,
            ..BackupFile::default()
        });
    }
    for plant in &scan.plant_data {
        let (format, members) = zip_parts(&plant.zip);
        files.push(BackupFile {
            name: plant.file_name.clone(),
            kind: FileKind::PlantData,
            format,
            size: plant.size,
            members,
            version_line: None,
            missing: false,
            ..BackupFile::default()
        });
    }
    if let Some(manifest) = &scan.manifest {
        let present: Vec<String> = scan
            .refdata
            .iter()
            .map(|entry| entry.entry.file_name.clone())
            .collect();
        files.extend(declared_refdata(manifest, &present, warnings));
    }
    for problem in &scan.problems {
        warnings.push(
            crate::tf!("{}: {}", problem.path.display(), problem.detail).into_owned(),
        );
    }
    files
}

/// The `RefData` files the manifest names but the backup does not carry.
///
/// A plant export logs a file it could not read as `File<<|>>4<<|>>…<<|>>
/// Failed to access file.` and writes no archive for it, so the file list
/// would silently lose a reference file the plant has. Those rows are
/// listed as [`BackupFile::missing`] with the reason in the warnings.
fn declared_refdata(
    manifest: &pid_parse::backup::Manifest,
    present: &[String],
    warnings: &mut Vec<String>,
) -> Vec<BackupFile> {
    let mut missing = Vec::new();
    for line in manifest.all("File") {
        let Some(schema) = line.fields.first().and_then(|field| field.parse::<u32>().ok()) else {
            continue;
        };
        // `4` is the reference-data schema in every backup observed; the
        // drawings ride in schema `2` (`PlantData~2~…`).
        if schema != REFDATA_SCHEMA {
            continue;
        }
        let Some(id) = line.fields.get(2).and_then(|field| field.parse::<u32>().ok()) else {
            continue;
        };
        let declared = line.fields.get(6).map(String::as_str).unwrap_or_default();
        let name = if declared.starts_with("RefData~") {
            declared.to_string()
        } else {
            format!("RefData~{schema}~{id}")
        };
        let carried = present.iter().any(|file| {
            file == &name
                || file
                    .strip_suffix(".zip")
                    .is_some_and(|stem| stem == name)
        });
        if carried {
            continue;
        }
        warnings.push(
            crate::tf!(
                "{} is listed in Manifest.txt but the backup does not carry it ({})",
                name,
                declared
            )
            .into_owned(),
        );
        missing.push(BackupFile {
            name,
            kind: FileKind::RefData,
            format: String::new(),
            size: 0,
            members: Vec::new(),
            version_line: None,
            missing: true,
            ..BackupFile::default()
        });
    }
    missing
}

fn export_file(export: &ExportDumpScan) -> BackupFile {
    let (format, version_line) = match &export.format {
        ExportDumpFormat::OracleExp { version_line } => (
            format!("Oracle exp ({version_line})"),
            Some(version_line.clone()),
        ),
        ExportDumpFormat::SqlServerMtf(_) => ("SQL Server MTF".to_string(), None),
        ExportDumpFormat::Unrecognized { detail } => {
            (format!("Unrecognized ({detail})"), None)
        }
    };
    BackupFile {
        name: "Export.dmp".to_string(),
        kind: FileKind::ExportDump,
        format,
        size: export.size,
        members: Vec::new(),
        version_line,
        missing: false,
        ..BackupFile::default()
    }
}

fn zip_parts(zip: &ZipListing) -> (String, Vec<String>) {
    match zip {
        ZipListing::Listed(entries) => (
            "Zip".to_string(),
            entries.iter().map(|entry| entry.name.clone()).collect(),
        ),
        ZipListing::NotAZip => (String::new(), Vec::new()),
        ZipListing::Unreadable => ("Zip (unreadable)".to_string(), Vec::new()),
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// The `T_DRAWING` rows and plant groups, when the dump can supply them.
///
/// A dump that is absent, is a SQL Server stream, or cannot be read pushes one
/// warning and returns `None`, which sends the caller to the folder listing.
fn dump_drawings(
    backup_root: &Path,
    scan: &BackupFolderScan,
    warnings: &mut Vec<String>,
) -> Option<Vec<DrawingRow>> {
    let Some(export) = &scan.export else {
        warnings.push(
            crate::t!("no Export.dmp in the backup folder; drawings are listed by folder")
                .into_owned(),
        );
        return None;
    };
    match &export.format {
        ExportDumpFormat::SqlServerMtf(_) => {
            warnings.push(
                crate::t!(
                    "Export.dmp is a SQL Server MTF dump; its database is not read, so drawings are listed by folder"
                )
                .into_owned(),
            );
            return None;
        }
        ExportDumpFormat::Unrecognized { detail } => {
            warnings.push(
                crate::tf!(
                    "Export.dmp is not a recognizable Oracle exp dump ({}); drawings are listed by folder",
                    detail
                )
                .into_owned(),
            );
            return None;
        }
        ExportDumpFormat::OracleExp { .. } => {}
    }
    let bytes = match std::fs::read(&export.path) {
        Ok(bytes) => bytes,
        Err(error) => {
            warnings.push(
                crate::tf!(
                    "cannot read {}: {}; drawings are listed by folder",
                    export.path.display(),
                    error
                )
                .into_owned(),
            );
            return None;
        }
    };
    let rows = match read_backup_drawings(&bytes) {
        Ok(rows) => rows,
        Err(error) => {
            warnings.push(
                crate::tf!(
                    "cannot read the drawing table from {}: {}; drawings are listed by folder",
                    backup_root.join("Export.dmp").display(),
                    format!("{error:?}")
                )
                .into_owned(),
            );
            return None;
        }
    };
    let groups = read_plant_groups(&bytes).unwrap_or_else(|error| {
        warnings.push(
            crate::tf!(
                "cannot read the plant groups from Export.dmp: {}; group paths are left empty",
                format!("{error:?}")
            )
            .into_owned(),
        );
        Vec::new()
    });
    let rows = rows
        .into_iter()
        .map(|row| DrawingRow {
            group_path: plant_group_path(&groups, &row.plant_group_id),
            meta: drawing_meta(&row),
            name: row.name.clone(),
            path_key: row.path_key.clone(),
        })
        .collect();
    Some(rows)
}

/// Every `T_DRAWING` column as a label/value pair, empty values included.
pub fn drawing_meta(row: &BackupDrawing) -> Vec<(String, String)> {
    let number = |value: Option<i64>| value.map(|value| value.to_string()).unwrap_or_default();
    vec![
        ("SP_ID".to_string(), row.id.clone()),
        ("NAME".to_string(), row.name.clone()),
        ("PATH".to_string(), row.path.clone()),
        ("DRAWINGNUMBER".to_string(), row.drawing_number.clone()),
        ("TITLE".to_string(), row.title.clone()),
        ("DESCRIPTION".to_string(), row.description.clone()),
        ("REVISION".to_string(), row.revision.clone()),
        ("VERSION".to_string(), row.version.clone()),
        ("DOCUMENTTYPE".to_string(), number(row.document_type)),
        ("DOCUMENTCATEGORY".to_string(), number(row.document_category)),
        ("ITEMSTATUS".to_string(), number(row.item_status)),
        ("TEMPLATE".to_string(), row.template.clone()),
        ("DESIGNPHASE".to_string(), row.design_phase.clone()),
        ("PROJECTNUMBER".to_string(), row.project_number.clone()),
        ("DATECREATED".to_string(), row.date_created.clone()),
        ("DRAWINGDATE".to_string(), row.drawing_date.clone()),
        ("DATEOFDRAWING".to_string(), row.date_of_drawing.clone()),
        ("LASTPUBLISHTIME".to_string(), row.last_publish_time.clone()),
        ("LASTARCHIVETIME".to_string(), row.last_archive_time.clone()),
        ("SP_PLANTGROUPID".to_string(), row.plant_group_id.clone()),
    ]
}

/// Pair each row with the restored `.pid` it names.
///
/// A row matches by [`backup_path_key`] of the file's path relative to `base`
/// (the table's `PATH` is `\00\00\name.pid`, the unpacked file is
/// `00/00/name.pid`). A row whose key does not match falls back to a unique
/// file-name match, which covers a plant that stores its `PATH` with a prefix
/// the backup does not carry. A row that matches nothing keeps its metadata
/// with an empty [`DrawingEntry::pid_path`].
pub fn match_drawings(rows: &[DrawingRow], pids: &[PathBuf], base: &Path) -> MatchedDrawings {
    match_drawings_under(rows, pids, &[base])
}

/// [`match_drawings`] with several candidate bases: a `.pid` is keyed relative
/// to the first base it lies under. A backup folder source keeps its drawings
/// in the restore folder, so the backup root and the restore folder are both
/// offered.
pub fn match_drawings_under(
    rows: &[DrawingRow],
    pids: &[PathBuf],
    bases: &[&Path],
) -> MatchedDrawings {
    let mut by_key: Vec<(String, usize)> = Vec::with_capacity(pids.len());
    let mut by_name: Vec<(String, usize)> = Vec::with_capacity(pids.len());
    for (index, pid) in pids.iter().enumerate() {
        let base = bases
            .iter()
            .find(|base| pid.starts_with(base))
            .copied()
            .unwrap_or_else(|| bases.first().copied().unwrap_or(Path::new("")));
        by_key.push((backup_path_key(&relative_key(pid, base)), index));
        by_name.push((file_name_lower(pid), index));
    }
    let mut used = vec![false; pids.len()];
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let mut found = None;
        if !row.path_key.is_empty() {
            found = by_key
                .iter()
                .find(|(key, index)| key == &row.path_key && !used[*index])
                .map(|(_, index)| *index);
        }
        if found.is_none() {
            let wanted = file_name_of_key(&row.path_key).to_lowercase();
            found = by_name
                .iter()
                .find(|(name, index)| !used[*index] && name == &wanted)
                .map(|(_, index)| *index);
        }
        if let Some(index) = found {
            used[index] = true;
        }
        entries.push(DrawingEntry {
            name: row.name.clone(),
            pid_path: found.map(|index| pids[index].clone()),
            group_path: filed_under(&row.group_path),
            meta: row.meta.clone(),
        });
    }
    let unmatched = pids
        .iter()
        .enumerate()
        .filter(|(index, _)| !used[*index])
        .map(|(_, pid)| pid.clone())
        .collect();
    MatchedDrawings { entries, unmatched }
}

/// [`match_drawings`] plus the entries the unmatched `.pid` files become: each
/// is listed under [`UNREGISTERED_GROUP`], so a drawing the table does not
/// know about still appears in the tree.
pub fn assemble_drawings(
    rows: &[DrawingRow],
    pids: &[PathBuf],
    base: &Path,
    restored: &Path,
) -> (Vec<DrawingEntry>, Vec<DrawingEntry>) {
    let matched = match_drawings_under(rows, pids, &[base, restored]);
    let unregistered = matched
        .unmatched
        .iter()
        .map(|pid| DrawingEntry {
            name: file_stem(pid),
            pid_path: Some(pid.clone()),
            group_path: UNREGISTERED_GROUP.to_string(),
            meta: vec![
                ("PATH".to_string(), relative_key(pid, base)),
                ("FILE".to_string(), pid.display().to_string()),
            ],
        })
        .collect();
    (matched.entries, unregistered)
}

/// The drawings of a backup with no usable drawing table, filed by folder.
pub fn folder_entries(pids: &[PathBuf], backup_root: &Path, restored: &Path) -> Vec<DrawingEntry> {
    pids.iter()
        .map(|pid| {
            let base = if pid.starts_with(backup_root) {
                backup_root
            } else {
                restored
            };
            let folder = folder_key(pid, base);
            DrawingEntry {
                name: file_stem(pid),
                pid_path: Some(pid.clone()),
                group_path: filed_under(&folder),
                meta: vec![
                    ("PATH".to_string(), relative_key(pid, base)),
                    ("FILE".to_string(), pid.display().to_string()),
                ],
            }
        })
        .collect()
}

/// `path` relative to `base`, slash-folded; the whole path when it is outside.
fn relative_key(path: &Path, base: &Path) -> String {
    let relative = path.strip_prefix(base).unwrap_or(path);
    backup_path_key(&relative.to_string_lossy())
}

/// The folder part of `path` relative to `base`, slash-folded.
fn folder_key(path: &Path, base: &Path) -> String {
    let key = relative_key(path, base);
    key.rsplit_once('/')
        .map(|(folder, _)| folder.to_string())
        .unwrap_or_default()
}

fn file_name_of_key(key: &str) -> String {
    key.rsplit_once('/')
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| key.to_string())
}

fn file_name_lower(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// What one drawing holds: its model items, grouped by `ModelItemType`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrawingItems {
    /// The drawing UID (`DrawingNo`) the file states.
    pub drawing_uid: String,
    /// The drawing name: the file stem.
    pub drawing_name: String,
    /// Groups ordered by type name; items inside a group ordered as the
    /// tree shows them (see [`labels::label_items`]).
    pub groups: Vec<ItemGroup>,
    /// How many items the groups hold together.
    pub item_count: usize,
    /// What the file holds beside the items: connection points, the graphic
    /// each item is drawn as, and how much of the file was read.
    pub data: drawing_data::DrawingData,
}

/// One `ModelItemType` of a drawing and the items of that type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemGroup {
    /// `ModelItemType`, e.g. `Instrument`.
    pub item_type: String,
    /// The items of this type: the named ones by name, then the rest by UID.
    pub items: Vec<DrawingItem>,
}

/// One model item of a drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingItem {
    /// The model item UID (`ModelID`).
    pub uid: String,
    /// What the tree calls the item: the name it publishes (`MM-222`), or
    /// its type and number when it publishes none (`PipeRun #17`).
    pub label: String,
    /// `ModelItemType`, repeated here so an item reads on its own.
    pub item_type: String,
    /// The published class, when the file decides it.
    pub class: Option<String>,
    /// Published attributes, in the order the file gives them.
    pub attributes: Vec<ItemAttribute>,
}

/// One published attribute of a model item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemAttribute {
    /// The publish interface, e.g. `INamedInstrument`.
    pub interface: String,
    /// The attribute, e.g. `MeasuredVariable`.
    pub name: String,
    /// The value as the file states it.
    pub value: String,
}

/// Read the model items of the `.pid` at `path`, grouped by type.
///
/// The panel asks for this the first time a drawing is expanded: a drawing's
/// attributes are megabytes of compound file the tree must not wait on.
pub fn load_items(path: &Path) -> Result<DrawingItems, String> {
    let drawing = load_publish_from_pid(path).map_err(|error| read_failure(path, &error))?;
    Ok(group_items(drawing))
}

/// The name the file at `path` goes by in a message: its file name. The
/// folder of a backup's drawing is the restore cache's -- a key and all --
/// and tells nobody which drawing it is; the details of the drawing state
/// the whole path for whoever wants it.
pub(crate) fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `text` with the whole path of `path`, wherever a reader quoted it, cut
/// down to the file's name.
pub(crate) fn name_the_file_only(text: &str, path: &Path) -> String {
    let whole = path.display().to_string();
    let name = file_label(path);
    if whole.is_empty() || whole == name {
        text.to_string()
    } else {
        text.replace(&whole, &name)
    }
}

/// Why the `.pid` at `path` gave no items, as the panel says it.
pub(crate) fn read_failure(path: &Path, error: &PidLoadError) -> String {
    let reason = match error {
        // The reader's one reason of its own, in the application's words.
        PidLoadError::NoDrawing => crate::t!(
            "it states no drawing number (no P&IDAttributes row carries a DrawingNo)"
        )
        .into_owned(),
        // What went wrong with the file itself.
        PidLoadError::Io(error) => name_the_file_only(&error.to_string(), path),
    };
    crate::tf!("cannot read {}: {}", file_label(path), reason).into_owned()
}

/// [`load_items`] on a worker thread, as [`crate::io::pid_backup::restore`]
/// does its unpacking.
pub async fn load_items_in_background(path: PathBuf) -> Result<DrawingItems, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-pid-backup-items".into())
        .spawn(move || {
            let _ = sender.send(load_items(&path));
        })
        .map_err(|error| {
            crate::tf!("cannot start reading the drawing: {}", error).into_owned()
        })?;
    receiver
        .await
        .unwrap_or_else(|_| {
            Err(crate::t!("the drawing read stopped before it finished").into_owned())
        })
}

/// [`build_index`] on a worker thread: restoring a backup and reading its dump
/// are tens of megabytes the UI thread must not wait out.
pub async fn build_index_in_background(
    source: PathBuf,
    target: Option<PathBuf>,
    cache_root: Option<PathBuf>,
) -> Result<BackupIndex, String> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-pid-backup-index".into())
        .spawn(move || {
            let _ = sender.send(build_index_in(
                &source,
                target.as_deref(),
                cache_root.as_deref(),
            ));
        })
        .map_err(index_start_error)?;
    receiver.await.unwrap_or_else(|_| Err(index_stopped_error()))
}

/// The message a refused index thread reports, in the UI language.
fn index_start_error(error: std::io::Error) -> String {
    crate::tf!("cannot start indexing the backup: {}", error).into_owned()
}

/// The message a dropped index channel reports, in the UI language: the
/// worker thread went away without sending, e.g. because it panicked.
fn index_stopped_error() -> String {
    crate::t!("the backup index stopped before it finished").into_owned()
}

/// Group a loaded drawing's items by `ModelItemType`: type-name sorted
/// between the groups, and inside each ordered and labelled for the tree.
pub fn group_items(drawing: PidPublishDrawing) -> DrawingItems {
    let PidPublishDrawing {
        drawing_uid,
        drawing_name,
        items,
        process_points,
        branch_points,
        representations,
        stats,
        ..
    } = drawing;
    let (groups, item_count) = group_published_items(items);
    // The rest of the file names items by UID; the groups know what the
    // tree calls them.
    let data = drawing_data::drawing_data(
        &process_points,
        &branch_points,
        &representations,
        stats,
        &groups,
    );
    DrawingItems {
        drawing_uid,
        drawing_name,
        groups,
        item_count,
        data,
    }
}

/// The grouping [`group_items`] does, over the items alone.
pub fn group_published_items(items: Vec<PidPublishItem>) -> (Vec<ItemGroup>, usize) {
    let mut by_type: BTreeMap<String, Vec<DrawingItem>> = BTreeMap::new();
    let mut item_count = 0;
    for item in items {
        item_count += 1;
        by_type
            .entry(item.model_item_type.clone())
            .or_default()
            .push(DrawingItem {
                uid: item.uid,
                // Set with the group's order, once the whole group is known.
                label: String::new(),
                item_type: item.model_item_type,
                class: item.class.map(str::to_string),
                attributes: item
                    .attributes
                    .into_iter()
                    .map(|attribute| ItemAttribute {
                        interface: attribute.interface.to_string(),
                        name: attribute.name.to_string(),
                        value: attribute.value,
                    })
                    .collect(),
            });
    }
    let groups = by_type
        .into_iter()
        .map(|(item_type, mut items)| {
            labels::label_items(&item_type, &mut items);
            ItemGroup { item_type, items }
        })
        .collect();
    (groups, item_count)
}

/// The path of the group the tree files a drawing under, spelled as the
/// group spells its own ([`GroupNode::path`]): the segments of the drawing
/// table's `SQPlant \ 00 \ 00`, or of a folder key `00/00`, joined by
/// ` / `; [`UNGROUPED`] when there are none. A fixed label
/// ([`UNREGISTERED_GROUP`]) is one segment and stays itself.
pub fn filed_under(path: &str) -> String {
    let parts = split_group_path(path);
    if parts.is_empty() {
        UNGROUPED.to_string()
    } else {
        parts.join(" / ")
    }
}

/// Put the drawings of every group in name order, as a person sorts names
/// ([`labels::natural_cmp`]): the order of the drawing table's rows, or of
/// the files' paths, is nobody's. Drawings of one name keep their order.
pub fn sort_group_drawings(groups: &mut [GroupNode], drawings: &[DrawingEntry]) {
    let name = |at: &usize| drawings.get(*at).map_or("", |entry| entry.name.as_str());
    for group in groups {
        group
            .drawings
            .sort_by(|left, right| labels::natural_cmp(name(left), name(right)).then(left.cmp(right)));
        sort_group_drawings(&mut group.children, drawings);
    }
}

/// Fold the group paths into a tree; a path's segments nest under each other.
pub fn build_group_tree(paths: &[String]) -> Vec<GroupNode> {
    let mut roots: Vec<GroupNode> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let parts = split_group_path(path);
        if parts.is_empty() {
            insert_group(&mut roots, &[UNGROUPED.to_string()], "", index);
        } else {
            insert_group(&mut roots, &parts, "", index);
        }
    }
    roots
}

/// Split a `plant_group_path` (`SQPlant \ 00 \ 00`) or a folder key
/// (`00/00`) into its segments.
fn split_group_path(path: &str) -> Vec<String> {
    path.replace(" \\ ", "/")
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn insert_group(nodes: &mut Vec<GroupNode>, parts: &[String], prefix: &str, index: usize) {
    let Some((head, rest)) = parts.split_first() else {
        return;
    };
    let path = if prefix.is_empty() {
        head.clone()
    } else {
        format!("{prefix} / {head}")
    };
    let at = match nodes.iter().position(|node| node.name == *head) {
        Some(at) => at,
        None => {
            nodes.push(GroupNode {
                name: head.clone(),
                path,
                children: Vec::new(),
                drawings: Vec::new(),
            });
            nodes.len() - 1
        }
    };
    if rest.is_empty() {
        nodes[at].drawings.push(index);
        return;
    }
    let child_prefix = nodes[at].path.clone();
    insert_group(&mut nodes[at].children, rest, &child_prefix, index);
}

/// The `Ref` reference-data tree under `dir`; empty when the folder is absent.
pub fn read_ref_tree(dir: &Path) -> RefNode {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Ref".to_string());
    let mut node = RefNode {
        name,
        dirs: Vec::new(),
        files: Vec::new(),
    };
    fill_ref_tree(dir, &mut node, REF_SEARCH_DEPTH);
    node
}

fn fill_ref_tree(dir: &Path, node: &mut RefNode, depth: usize) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_dir() {
            let mut child = RefNode {
                name,
                dirs: Vec::new(),
                files: Vec::new(),
            };
            fill_ref_tree(&entry.path(), &mut child, depth - 1);
            node.dirs.push(child);
        } else if kind.is_file() {
            node.files.push(RefFile {
                name,
                size: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            });
        }
    }
    node.dirs.sort_by(|left, right| left.name.cmp(&right.name));
    node.files.sort_by(|left, right| left.name.cmp(&right.name));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ocs-pid-backup-browse-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in members {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }

    /// A backup folder with a manifest, a drawing archive of two folders, and
    /// a `Ref` tree -- and no `Export.dmp`.
    ///
    /// The `Ref` tree is nested three levels deep on purpose: a read-only
    /// check that only lists the top level of the source would miss a file
    /// written under `Ref/Symbols/Valves` or `Ref/Sheets`.
    fn synthetic_backup(root: &Path) -> PathBuf {
        let backup = root.join("backup");
        std::fs::create_dir_all(backup.join("Ref").join("Symbols").join("Valves")).unwrap();
        std::fs::create_dir_all(backup.join("Ref").join("Sheets")).unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "BackupType<<|>>2\nVersion<<|>>7.02\nDateCreated<<|>>07/19/2026 20:13:24\nName<<|>>SQPlant\nSpid<<|>>F8D466206C55478D9BB9482FA7769A46\n",
        )
        .unwrap();
        std::fs::write(backup.join("Ref").join("Symbols").join("valve.sym"), b"s").unwrap();
        std::fs::write(
            backup
                .join("Ref")
                .join("Symbols")
                .join("Valves")
                .join("ball.sym"),
            b"b",
        )
        .unwrap();
        std::fs::write(backup.join("Ref").join("Sheets").join("legend.xml"), b"l").unwrap();
        std::fs::write(backup.join("Ref").join("readme.txt"), b"r").unwrap();
        write_zip(
            &backup.join("PlantData~2~711.zip"),
            &[
                ("00/00/A.pid", b"a"),
                ("00/00/B.pid", b"b"),
                ("01/01/C.pid", b"c"),
            ],
        );
        write_zip(
            &backup.join("RefData~4~681.zip"),
            &[("Symbols/valve.sym", b"s"), ("Symbols/pump.sym", b"p")],
        );
        backup
    }

    /// Everything under `path`, depth first: `relative/path:size` per entry,
    /// with `-1` for a directory. Relative paths are `/`-separated so the
    /// listing does not depend on the platform separator, and directories are
    /// listed so a folder the index created shows up as well.
    fn manifest_of(path: &Path) -> Vec<String> {
        fn walk(dir: &Path, prefix: &str, listed: &mut Vec<String>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .collect();
            entries.sort();
            for entry in entries {
                let name = entry.file_name().unwrap().to_string_lossy().into_owned();
                let relative = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    listed.push(format!("{relative}:-1"));
                    walk(&entry, &relative, listed);
                } else {
                    listed.push(format!("{relative}:{}", meta.len()));
                }
            }
        }
        let mut listed = Vec::new();
        walk(path, "", &mut listed);
        listed.sort();
        listed
    }

    #[test]
    fn a_backup_without_a_dump_lists_its_files_groups_and_ref_tree() {
        let root = scratch("nodes");
        let backup = synthetic_backup(&root);
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();

        let manifest = index.manifest.as_ref().unwrap();
        assert_eq!(manifest.name.as_deref(), Some("SQPlant"));
        assert_eq!(manifest.version.as_deref(), Some("7.02"));
        assert_eq!(
            manifest.date_created.as_deref(),
            Some("07/19/2026 20:13:24")
        );
        assert_eq!(manifest.backup_type.as_deref(), Some("2"));
        assert_eq!(
            manifest.spid.as_deref(),
            Some("F8D466206C55478D9BB9482FA7769A46")
        );

        let names: Vec<&str> = index.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Manifest.txt", "RefData~4~681.zip", "PlantData~2~711.zip"]
        );
        let refdata = index
            .files
            .iter()
            .find(|file| file.kind == FileKind::RefData)
            .unwrap();
        assert_eq!(refdata.format, "Zip");
        assert_eq!(
            refdata.members,
            vec!["Symbols/valve.sym", "Symbols/pump.sym"]
        );
        let plant = index
            .files
            .iter()
            .find(|file| file.kind == FileKind::PlantData)
            .unwrap();
        assert_eq!(plant.format, "Zip");
        assert_eq!(
            plant.members,
            vec!["00/00/A.pid", "00/00/B.pid", "01/01/C.pid"]
        );
        assert!(plant.size > 0);
        let manifest_file = index
            .files
            .iter()
            .find(|file| file.kind == FileKind::Manifest)
            .unwrap();
        assert_eq!(manifest_file.size, file_size(&backup.join("Manifest.txt")));

        assert_eq!(index.drawings.len(), 3);
        assert!(index
            .drawings
            .iter()
            .all(|drawing| drawing.pid_path.is_some()));
        assert_eq!(index.ref_tree.file_count(), 4);
        let symbols = index
            .ref_tree
            .dirs
            .iter()
            .find(|dir| dir.name == "Symbols")
            .unwrap();
        assert_eq!(
            symbols
                .files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["valve.sym"]
        );
        // Sub-directories are walked, not just the first level.
        assert_eq!(
            symbols
                .dirs
                .iter()
                .map(|dir| dir.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Valves"]
        );
        assert_eq!(symbols.dirs[0].files[0].name, "ball.sym");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_backup_without_a_dump_groups_its_drawings_by_folder() {
        let root = scratch("degraded");
        let backup = synthetic_backup(&root);
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();

        assert!(
            index
                .warnings
                .iter()
                .any(|warning| warning.contains("no Export.dmp")),
            "{:?}",
            index.warnings
        );
        let groups: Vec<&str> = index
            .plant_groups
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(groups, vec!["00", "01"]);
        assert_eq!(index.plant_groups[0].children[0].name, "00");
        assert_eq!(index.plant_groups[0].children[0].drawings.len(), 2);
        assert_eq!(index.plant_groups[1].children[0].drawings.len(), 1);
        let mut names: Vec<&str> = index.plant_groups[0].children[0]
            .drawings
            .iter()
            .map(|at| index.drawings[*at].name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["A", "B"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_source_is_left_untouched() {
        let root = scratch("readonly");
        let backup = synthetic_backup(&root);
        let before = manifest_of(&backup);
        // The listing really reaches into the sub-folders: a top-level-only
        // manifest would still be equal before and after a write below it.
        assert!(
            before.contains(&"Ref/Symbols/Valves/ball.sym:1".to_string()),
            "{before:?}"
        );
        assert!(before.contains(&"Ref/Symbols:-1".to_string()), "{before:?}");
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();

        assert!(index.read_only);
        assert_eq!(index.backup_root, backup);
        assert_eq!(manifest_of(&backup), before);
        assert!(!backup.join("00").exists(), "the source is never written");
        assert!(!backup.join(".ocs-restored").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_zip_source_finds_the_backup_root_below_the_restore_folder() {
        let root = scratch("zipped");
        let inner = root.join("PlantData~2~711.zip");
        write_zip(&inner, &[("00/00/A.pid", b"a")]);
        let inner_bytes = std::fs::read(&inner).unwrap();
        let outer = root.join("backup.zip");
        write_zip(
            &outer,
            &[
                (
                    "Backup/Manifest.txt",
                    b"Name<<|>>SQPlant\nVersion<<|>>7.02\n",
                ),
                ("Backup/PlantData~2~711.zip", &inner_bytes),
                ("Backup/Ref/Symbols/valve.sym", b"s"),
            ],
        );
        let target = root.join("out");
        let index = build_index(&outer, Some(&target)).unwrap();

        assert_eq!(index.restored, target);
        assert_eq!(index.backup_root, target.join("Backup"));
        assert!(!index.read_only);
        assert_eq!(
            index.manifest.as_ref().unwrap().name.as_deref(),
            Some("SQPlant")
        );
        assert_eq!(index.drawings.len(), 1);
        let drawing = &index.drawings[0];
        assert_eq!(drawing.name, "A");
        assert!(drawing.pid_path.as_ref().unwrap().is_file());
        assert_eq!(index.ref_tree.file_count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_database_row_without_a_file_is_marked_missing() {
        let pids = vec![
            PathBuf::from("/restore/00/00/A.pid"),
            PathBuf::from("/restore/00/00/B.pid"),
        ];
        let rows = vec![
            DrawingRow {
                name: "A".to_string(),
                path_key: "00/00/A.pid".to_string(),
                group_path: "SQPlant / 00 / 00".to_string(),
                meta: vec![("SP_ID".to_string(), "1".to_string())],
            },
            DrawingRow {
                name: "Gone".to_string(),
                path_key: "00/00/Gone.pid".to_string(),
                group_path: "SQPlant / 00 / 00".to_string(),
                meta: vec![("SP_ID".to_string(), "2".to_string())],
            },
        ];
        let matched = match_drawings(&rows, &pids, Path::new("/restore"));
        assert_eq!(matched.entries.len(), 2);
        assert_eq!(
            matched.entries[0].pid_path.as_deref(),
            Some(Path::new("/restore/00/00/A.pid"))
        );
        assert!(matched.entries[1].is_missing());
        assert_eq!(matched.entries[1].meta.len(), 1);
        // B matched nothing either, so it is offered back as unregistered.
        assert_eq!(matched.unmatched, vec![PathBuf::from("/restore/00/00/B.pid")]);
    }

    #[test]
    fn a_row_falls_back_to_its_file_name() {
        let pids = vec![PathBuf::from("/restore/00/00/Drawing.pid")];
        let rows = vec![DrawingRow {
            name: "Drawing".to_string(),
            path_key: "test/U01/Drawing.pid".to_string(),
            group_path: String::new(),
            meta: Vec::new(),
        }];
        let matched = match_drawings(&rows, &pids, Path::new("/restore"));
        assert_eq!(
            matched.entries[0].pid_path.as_deref(),
            Some(Path::new("/restore/00/00/Drawing.pid"))
        );
        assert!(matched.unmatched.is_empty());
    }

    #[test]
    fn an_unregistered_drawing_is_still_listed() {
        let pids = vec![
            PathBuf::from("/restore/00/00/A.pid"),
            PathBuf::from("/restore/00/00/Loose.pid"),
        ];
        let rows = vec![DrawingRow {
            name: "A".to_string(),
            path_key: "00/00/A.pid".to_string(),
            group_path: "SQPlant \\ 00 \\ 00".to_string(),
            meta: Vec::new(),
        }];
        let (entries, unregistered) =
            assemble_drawings(&rows, &pids, Path::new("/restore"), Path::new("/restore"));
        assert_eq!(entries.len(), 1);
        // The table spells a group's path with backslashes; the drawing
        // states it the way the tree spells the group's own.
        assert_eq!(entries[0].group_path, "SQPlant / 00 / 00");
        assert_eq!(unregistered.len(), 1);
        assert_eq!(unregistered[0].name, "Loose");
        assert_eq!(unregistered[0].group_path, UNREGISTERED_GROUP);
        assert_eq!(
            unregistered[0].pid_path.as_deref(),
            Some(Path::new("/restore/00/00/Loose.pid"))
        );
        // The unregistered drawing is grouped under its own node.
        let tree = build_group_tree(&[UNREGISTERED_GROUP.to_string()]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].drawings, vec![0]);
    }

    #[test]
    fn a_backup_lists_a_drawing_the_table_does_not_know() {
        let root = scratch("unregistered-index");
        let backup = synthetic_backup(&root);
        // A loose `.pid` in the backup folder is not in any archive, so a
        // folder source's restore leaves it where it is; a `.pid` added to
        // the restore folder is what an unregistered drawing looks like.
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();
        std::fs::write(index.restored.join("00").join("00").join("Extra.pid"), b"x").unwrap();
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();
        assert_eq!(index.drawings.len(), 4);
        assert!(index
            .drawings
            .iter()
            .any(|drawing| drawing.name == "Extra" && drawing.pid_path.is_some()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_of_loose_drawings_is_indexed_in_place() {
        let root = scratch("loose");
        let backup = root.join("loose");
        std::fs::create_dir_all(backup.join("00").join("00")).unwrap();
        std::fs::write(backup.join("00").join("00").join("A.pid"), b"a").unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "Name<<|>>SQPlant\nVersion<<|>>7.02\n",
        )
        .unwrap();
        let before = manifest_of(&backup);
        // The drawing sits two folders down; only a recursive listing can
        // prove the in-place source was not touched at that depth.
        assert!(before.contains(&"00/00/A.pid:1".to_string()), "{before:?}");
        let index = build_index(&backup, None).unwrap();

        assert!(index.read_only);
        assert_eq!(index.restored, backup);
        assert_eq!(index.backup_root, backup);
        assert_eq!(manifest_of(&backup), before);
        assert_eq!(index.drawings.len(), 1);
        assert_eq!(index.drawings[0].name, "A");
        assert_eq!(
            index.drawings[0].pid_path.as_deref(),
            Some(backup.join("00").join("00").join("A.pid").as_path())
        );
        assert_eq!(index.plant_groups[0].name, "00");
        assert_eq!(index.plant_groups[0].children[0].drawings, vec![0]);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── A source with nothing to list ────────────────────────────────────

    /// What a source that holds nothing of a backup and no drawing is refused
    /// with, as an English interface says it.
    fn not_a_backup_in_english(source: &Path) -> String {
        format!(
            "{} is not a SmartPlant P&ID backup: no Manifest.txt, Export.dmp or PlantData~ \
             archive in it, and no .pid drawing",
            source.display()
        )
    }

    /// The refusal that `result` is. An index fails the test, which then says
    /// what was listed.
    fn refusal(result: Result<BackupIndex, String>) -> String {
        match result {
            Ok(index) => panic!(
                "{} was listed: {} drawing(s), warnings {:?}",
                index.source.display(),
                index.drawings.len(),
                index.warnings
            ),
            Err(reason) => reason,
        }
    }

    /// What the folder `cache` holds, as [`manifest_of`] lists it; nothing
    /// when the folder was never made.
    fn kept_in(cache: &Path) -> Vec<String> {
        if cache.is_dir() {
            manifest_of(cache)
        } else {
            Vec::new()
        }
    }

    /// A folder with nothing of a backup in it -- no manifest, no dump, no
    /// drawing archive -- and no drawing is refused: one sentence that names
    /// the folder and says it is no backup, and no index. The folder is the
    /// user's, and holds afterwards what it held; nothing was unpacked for
    /// it, into the cache or into the folder a command names for the
    /// restore. A folder with nothing at all in it is refused the same way.
    #[test]
    fn a_folder_with_nothing_of_a_backup_is_refused_and_left_as_it_was() {
        let root = scratch("refused-folder");
        let folder = root.join("holiday");
        std::fs::create_dir_all(folder.join("album").join("day 1")).unwrap();
        std::fs::write(folder.join("notes.txt"), b"nothing of a backup").unwrap();
        std::fs::write(folder.join("album").join("day 1").join("beach.jpg"), b"jpg").unwrap();
        let before = manifest_of(&folder);
        assert!(
            before
                .iter()
                .any(|entry| entry == "album/day 1/beach.jpg:3"),
            "{before:?}"
        );
        let cache = root.join("cache");

        let reason = refusal(build_index_in(&folder, None, Some(&cache)));
        assert_eq!(reason, not_a_backup_in_english(&folder));
        assert_eq!(manifest_of(&folder), before);
        assert_eq!(kept_in(&cache), Vec::<String>::new());

        // With a folder named for the restore: refused as well, and that
        // folder is not made.
        let named = root.join("out");
        let reason = refusal(build_index_in(&folder, Some(&named), Some(&cache)));
        assert_eq!(reason, not_a_backup_in_english(&folder));
        assert_eq!(manifest_of(&folder), before);
        assert!(!named.exists());

        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let reason = refusal(build_index_in(&empty, None, Some(&cache)));
        assert_eq!(reason, not_a_backup_in_english(&empty));
        assert!(empty.is_dir());
        assert_eq!(kept_in(&cache), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A zip with nothing of a backup in it and no drawing is refused with
    /// the same sentence, and nothing of it stays in the cache: what was
    /// unpacked to find that out is taken away again. The zip is as it was.
    /// Asked a second time it is refused a second time -- no restore was
    /// left to reuse -- and the restore of another source in the same cache
    /// is there throughout.
    #[test]
    fn a_zip_with_nothing_of_a_backup_is_refused_and_nothing_of_it_stays_in_the_cache() {
        let root = scratch("refused-zip");
        let zip = root.join("holiday.zip");
        write_zip(
            &zip,
            &[
                ("notes.txt", b"nothing of a backup"),
                ("album/day 1/beach.jpg", b"jpg"),
            ],
        );
        let bytes = std::fs::read(&zip).unwrap();
        let cache = root.join("cache");
        // A backup, restored into the same cache before.
        let plant = root.join("plant.zip");
        write_zip(
            &plant,
            &[("Manifest.txt", b"Name<<|>>SQPlant\n"), ("00/A.pid", b"a")],
        );
        let listed = build_index_in(&plant, None, Some(&cache)).unwrap();
        assert_eq!(listed.drawings.len(), 1);
        let held = kept_in(&cache);
        assert!(
            held.iter().any(|entry| entry.ends_with("/00/A.pid:1")),
            "{held:?}"
        );

        for _ in 0..2 {
            let reason = refusal(build_index_in(&zip, None, Some(&cache)));
            assert_eq!(reason, not_a_backup_in_english(&zip));
            assert_eq!(kept_in(&cache), held);
            assert_eq!(std::fs::read(&zip).unwrap(), bytes);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A restore of such a zip that an earlier version of the program left in
    /// the cache -- it listed the zip as a backup of no drawing -- goes with
    /// the refusal: the cache is the program's own, and the restore of what
    /// is no backup is of no use in it.
    #[test]
    fn an_earlier_restore_of_a_refused_zip_is_taken_out_of_the_cache() {
        let root = scratch("refused-earlier");
        let zip = root.join("holiday.zip");
        write_zip(&zip, &[("notes.txt", b"nothing of a backup")]);
        let cache = root.join("cache");
        // As the earlier version left it: unpacked, finished, and reused when
        // the zip is opened again.
        let earlier = crate::io::pid_backup::restore_in(&zip, None, Some(&cache)).unwrap();
        assert!(
            earlier.root.starts_with(&cache),
            "{}",
            earlier.root.display()
        );
        assert!(earlier.root.join("notes.txt").is_file());
        let again = crate::io::pid_backup::restore_in(&zip, None, Some(&cache)).unwrap();
        assert!(again.reused);

        let reason = refusal(build_index_in(&zip, None, Some(&cache)));
        assert_eq!(reason, not_a_backup_in_english(&zip));
        assert_eq!(kept_in(&cache), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// With a folder named for the restore, the refusal takes away what this
    /// call unpacked there and nothing else. The folder was new or empty
    /// before the call -- a restore is made into no other -- so it goes
    /// whole, as it does when a restore fails half way. A finished restore
    /// of the same zip that was kept there since an earlier version is not
    /// this call's work: the zip is refused, and that folder stays as it is.
    #[test]
    fn a_refused_zip_leaves_no_restore_in_a_folder_named_for_it_but_an_earlier_one() {
        let root = scratch("refused-named");
        let zip = root.join("holiday.zip");
        write_zip(&zip, &[("notes.txt", b"nothing of a backup")]);
        let cache = root.join("cache");
        let named = root.join("out");

        for made_before in [false, true] {
            if made_before {
                std::fs::create_dir_all(&named).unwrap();
            }
            let reason = refusal(build_index_in(&zip, Some(&named), Some(&cache)));
            assert_eq!(reason, not_a_backup_in_english(&zip));
            assert!(!named.exists(), "made before: {made_before}");
        }
        assert_eq!(kept_in(&cache), Vec::<String>::new());

        let earlier = crate::io::pid_backup::restore_in(&zip, Some(&named), Some(&cache)).unwrap();
        assert_eq!(earlier.root, named);
        let held = manifest_of(&named);
        assert!(held.iter().any(|entry| entry == "notes.txt:19"), "{held:?}");
        let reason = refusal(build_index_in(&zip, Some(&named), Some(&cache)));
        assert_eq!(reason, not_a_backup_in_english(&zip));
        assert_eq!(manifest_of(&named), held);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Drawings alone are something to list. A folder that holds `.pid`
    /// files and nothing of a backup is browsed where it lies, as it was
    /// before there was a refusal, and a zip of drawings is unpacked and
    /// listed. Neither has a manifest.
    #[test]
    fn drawings_with_nothing_of_a_backup_around_them_are_listed_as_before() {
        let root = scratch("drawings-alone");
        let folder = root.join("drawings");
        std::fs::create_dir_all(folder.join("unit 1")).unwrap();
        std::fs::write(folder.join("A.pid"), b"a").unwrap();
        std::fs::write(folder.join("unit 1").join("B.pid"), b"b").unwrap();
        std::fs::write(folder.join("notes.txt"), b"not a drawing").unwrap();
        let before = manifest_of(&folder);
        let cache = root.join("cache");
        let names = |index: &BackupIndex| -> Vec<String> {
            let mut names: Vec<String> = index
                .drawings
                .iter()
                .map(|drawing| drawing.name.clone())
                .collect();
            names.sort();
            names
        };

        let index = build_index_in(&folder, None, Some(&cache)).unwrap();
        assert!(index.read_only);
        assert_eq!(index.restored, folder);
        assert_eq!(index.backup_root, folder);
        assert!(index.manifest.is_none());
        assert_eq!(names(&index), ["A", "B"]);
        assert_eq!(manifest_of(&folder), before);
        assert_eq!(kept_in(&cache), Vec::<String>::new());

        let zip = root.join("drawings.zip");
        write_zip(&zip, &[("A.pid", b"a"), ("unit 1/B.pid", b"b")]);
        let index = build_index_in(&zip, None, Some(&cache)).unwrap();
        assert!(!index.read_only);
        assert!(
            index.restored.starts_with(&cache),
            "{}",
            index.restored.display()
        );
        assert!(index.manifest.is_none());
        assert_eq!(names(&index), ["A", "B"]);
        for drawing in &index.drawings {
            let file = drawing
                .pid_path
                .as_ref()
                .expect("a listed drawing has a file");
            assert!(
                file.starts_with(&index.restored) && file.is_file(),
                "{}",
                file.display()
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder with a backup one or two levels below it is no stranger
    /// either: the backup in it is found, as before.
    #[test]
    fn a_folder_with_a_backup_two_levels_below_it_is_listed_as_before() {
        let root = scratch("backup-below");
        let deliveries = root.join("deliveries");
        let backup = synthetic_backup(&deliveries.join("2026"));
        let index = build_index_in(&deliveries, None, Some(&root.join("cache"))).unwrap();
        assert!(index.read_only);
        assert_eq!(index.backup_root, backup);
        assert_eq!(
            index
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.name.as_deref()),
            Some("SQPlant")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What makes a source a backup is what [`is_backup_folder`] asks for,
    /// not its drawings: a backup that holds no drawing file is listed, with
    /// no drawing, as before -- as a zip, as a zip that wraps the backup in
    /// a folder, and as a folder.
    #[test]
    fn a_backup_without_a_drawing_file_is_listed_as_before() {
        let root = scratch("backup-no-drawing");
        let cache = root.join("cache");
        let plant = |index: &BackupIndex| {
            index
                .manifest
                .as_ref()
                .and_then(|manifest| manifest.name.clone())
        };

        let zip = root.join("plant.zip");
        write_zip(&zip, &[("Manifest.txt", b"Name<<|>>SQPlant\n")]);
        let index = build_index_in(&zip, None, Some(&cache)).unwrap();
        assert!(index.drawings.is_empty());
        assert_eq!(index.backup_root, index.restored);
        assert_eq!(plant(&index).as_deref(), Some("SQPlant"));

        let wrapped = root.join("wrapped.zip");
        write_zip(&wrapped, &[("plant/Manifest.txt", b"Name<<|>>SQPlant\n")]);
        let index = build_index_in(&wrapped, None, Some(&cache)).unwrap();
        assert!(index.drawings.is_empty());
        assert_eq!(index.backup_root, index.restored.join("plant"));
        assert_eq!(plant(&index).as_deref(), Some("SQPlant"));

        let folder = root.join("plant");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Manifest.txt"), b"Name<<|>>SQPlant\n").unwrap();
        let index = build_index_in(&folder, None, Some(&cache)).unwrap();
        assert!(index.read_only);
        assert!(index.drawings.is_empty());
        assert_eq!(index.backup_root, folder);
        assert_eq!(plant(&index).as_deref(), Some("SQPlant"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The refusal speaks the UI language, as the other failures of the index
    /// do: a Chinese interface is told in Chinese, and in every language of
    /// the interface the sentence names the source. The language is one
    /// loader for the whole process, so this runs in a child process of the
    /// test binary, as `the_index_failures_speak_the_ui_language` does.
    #[test]
    fn the_refusal_of_what_is_no_backup_speaks_the_ui_language() {
        const CHILD: &str = "OCS_PID_BACKUP_NOT_A_BACKUP_CHILD";
        const NAME: &str =
            "io::pid_backup_browse::tests::the_refusal_of_what_is_no_backup_speaks_the_ui_language";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(
                std::env::current_exe().expect("the test binary has a path"),
            )
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .expect("the test binary must start");
            assert!(
                output.status.success(),
                "the child run failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let root = scratch("refused-chinese");
        let folder = root.join("holiday");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("notes.txt"), b"nothing of a backup").unwrap();
        let cache = root.join("cache");
        let named = folder.display().to_string();
        for language in crate::i18n::Language::ALL {
            if language == crate::i18n::Language::System {
                continue;
            }
            crate::i18n::set_language(language).expect("Fluent resources must parse");
            let reason = refusal(build_index_in(&folder, None, Some(&cache)));
            assert!(reason.contains(&named), "{language:?}: {reason}");
            assert!(!reason.contains("__ocs_fmt_"), "{language:?}: {reason}");
        }
        crate::i18n::set_language(crate::i18n::Language::ZhCn)
            .expect("Chinese Fluent resources must parse");
        let reason = refusal(build_index_in(&folder, None, Some(&cache)));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            reason,
            format!(
                "{} 不是 SmartPlant P&ID 备份：里面没有 Manifest.txt、Export.dmp 或 PlantData~ \
                 压缩包，也没有 .pid 图纸",
                folder.display()
            )
        );
    }

    /// What goes with a refusal, case by case. A restore that was unpacked
    /// into the cache goes, whichever call made it. One unpacked into a
    /// folder named for it goes when this call made it and stays when it was
    /// found there. A folder that was not unpacked into stays. And a folder
    /// that is the source, or that the source lies in, is not removed
    /// whatever the restore says of itself; nor is anything removed for a
    /// source that cannot be found, which nothing can be told apart from.
    #[test]
    fn only_a_restore_that_was_unpacked_apart_from_its_source_is_discarded() {
        let root = scratch("discarded");
        let restore = |name: &str, extracted: bool, reused: bool| {
            let folder = root.join(name);
            std::fs::create_dir_all(folder.join("inner")).unwrap();
            std::fs::write(folder.join("inner").join("kept.txt"), b"k").unwrap();
            crate::io::pid_backup::Restored {
                root: folder,
                extracted,
                reused,
                drawings: Vec::new(),
            }
        };
        let kept = |restored: &crate::io::pid_backup::Restored| {
            restored.root.join("inner").join("kept.txt").is_file()
        };
        let zip = root.join("source.zip");
        std::fs::write(&zip, b"zip").unwrap();

        for reused in [false, true] {
            let in_cache = restore("in-cache", true, reused);
            discard_restore(&zip, false, &in_cache);
            assert!(!in_cache.root.exists(), "reused: {reused}");
        }
        let named_now = restore("named-now", true, false);
        discard_restore(&zip, true, &named_now);
        assert!(!named_now.root.exists());
        let named_before = restore("named-before", true, true);
        discard_restore(&zip, true, &named_before);
        assert!(kept(&named_before));

        let not_unpacked = restore("not-unpacked", false, false);
        discard_restore(&zip, false, &not_unpacked);
        assert!(kept(&not_unpacked));
        // A restore that claims to be unpacked, in the folder the source is
        // or lies in.
        let is_source = restore("is-the-source", true, false);
        discard_restore(&is_source.root, false, &is_source);
        assert!(kept(&is_source));
        let holds_source = restore("holds-the-source", true, false);
        discard_restore(
            &holds_source.root.join("inner").join("kept.txt"),
            false,
            &holds_source,
        );
        assert!(kept(&holds_source));
        let unresolved = restore("unresolved", true, false);
        discard_restore(&root.join("gone.zip"), false, &unresolved);
        assert!(kept(&unresolved));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_row_matches_a_drawing_unpacked_beside_the_backup_folder() {
        // A folder source: the backup root is the source folder, the drawings
        // land in the restore folder, and the row's PATH is relative to that.
        let pids = vec![PathBuf::from("/restore/00/00/A.pid")];
        let rows = vec![DrawingRow {
            name: "A".to_string(),
            path_key: "00/00/A.pid".to_string(),
            group_path: "SQPlant \\ 00 \\ 00".to_string(),
            meta: Vec::new(),
        }];
        let matched = match_drawings_under(
            &rows,
            &pids,
            &[Path::new("/backup"), Path::new("/restore")],
        );
        assert_eq!(
            matched.entries[0].pid_path.as_deref(),
            Some(Path::new("/restore/00/00/A.pid"))
        );
        assert!(matched.unmatched.is_empty());
    }

    /// The drawings of a group were listed in the order of the drawing
    /// table (or of their paths), which is nobody's order; and a drawing
    /// spelled its group's path another way than the group did.
    #[test]
    fn a_groups_drawings_are_in_name_order_and_state_the_groups_own_path() {
        let root = scratch("group-order");
        let backup = root.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("Manifest.txt"), "Name<<|>>SQPlant\n").unwrap();
        write_zip(
            &backup.join("PlantData~2~711.zip"),
            &[
                ("00/00/b.pid", b"b"),
                ("00/00/A10.pid", b"a"),
                ("00/00/a9.pid", b"a"),
                ("00/01/Z.pid", b"z"),
            ],
        );
        let index = build_index(&backup, Some(&root.join("out"))).unwrap();
        assert_eq!(index.drawings.len(), 4);
        // No drawing table here: the drawings are filed by folder.
        let group = &index.plant_groups[0].children[0];
        assert_eq!(group.path, "00 / 00");
        let names: Vec<&str> = group
            .drawings
            .iter()
            .map(|at| index.drawings[*at].name.as_str())
            .collect();
        // As a person sorts names: case aside, 9 before 10.
        assert_eq!(names, ["a9", "A10", "b"]);
        fn check(node: &GroupNode, index: &BackupIndex) {
            for at in &node.drawings {
                assert_eq!(index.drawings[*at].group_path, node.path);
            }
            for child in &node.children {
                check(child, index);
            }
        }
        for node in &index.plant_groups {
            check(node, &index);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn group_paths_nest_under_their_parents() {
        let tree = build_group_tree(&[
            "SQPlant \\ 00 \\ 00".to_string(),
            "SQPlant \\ 00 \\ 01".to_string(),
            "SQPlant \\ 01 \\ 00".to_string(),
            String::new(),
        ]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "SQPlant");
        assert_eq!(tree[0].path, "SQPlant");
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].path, "SQPlant / 00");
        assert_eq!(tree[0].children[0].children[0].path, "SQPlant / 00 / 00");
        assert_eq!(tree[0].children[0].children[0].drawings, vec![0]);
        assert_eq!(tree[0].children[0].children[1].drawings, vec![1]);
        assert_eq!(tree[1].name, UNGROUPED);
        assert_eq!(tree[1].drawings, vec![3]);
    }

    fn published_item(uid: &str, item_type: &str, class: Option<&'static str>) -> PidPublishItem {
        PidPublishItem {
            uid: uid.to_string(),
            model_item_type: item_type.to_string(),
            class,
            attributes: vec![pid_parse::publish::pid_load::PidPublishAttribute {
                interface: "INamedInstrument",
                name: "MeasuredVariable",
                value: "T".to_string(),
                source: pid_parse::publish::pid_load::PidValueSource::NameRule("test"),
            }],
        }
    }

    #[test]
    fn items_are_grouped_by_model_item_type() {
        // Fed in descending UID order, so an unsorted group would come out
        // reversed: only the group's own sort can yield ascending UIDs.
        let (groups, count) = group_published_items(vec![
            published_item("UID-E", "Pipe", None),
            published_item("UID-D", "Instrument", None),
            published_item("UID-C", "Instrument", Some("PIDInstrument")),
            published_item("UID-B", "Pipe", None),
            published_item("UID-A", "Instrument", None),
        ]);

        assert_eq!(count, 5);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].item_type, "Instrument");
        assert_eq!(
            groups[0]
                .items
                .iter()
                .map(|item| item.uid.as_str())
                .collect::<Vec<_>>(),
            vec!["UID-A", "UID-C", "UID-D"]
        );
        assert_eq!(groups[0].items[0].class, None);
        assert_eq!(groups[0].items[1].class.as_deref(), Some("PIDInstrument"));
        // None of them publishes a name: each is its type and its number.
        assert_eq!(
            groups[0]
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Instrument #1", "Instrument #2", "Instrument #3"]
        );
        assert_eq!(groups[1].item_type, "Pipe");
        assert_eq!(
            groups[1]
                .items
                .iter()
                .map(|item| item.uid.as_str())
                .collect::<Vec<_>>(),
            vec!["UID-B", "UID-E"]
        );
        // Every item carries its type and its attribute triple.
        let first = &groups[0].items[0];
        assert_eq!(first.item_type, "Instrument");
        assert_eq!(first.attributes.len(), 1);
        assert_eq!(first.attributes[0].interface, "INamedInstrument");
        assert_eq!(first.attributes[0].name, "MeasuredVariable");
        assert_eq!(first.attributes[0].value, "T");
        // The group counts add up to the item count.
        assert_eq!(
            groups.iter().map(|group| group.items.len()).sum::<usize>(),
            count
        );
    }

    #[test]
    fn a_declared_refdata_the_backup_lacks_is_listed_as_missing() {
        let root = scratch("declared-refdata");
        let backup = root.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "Name<<|>>SQPlant\n\
             File<<|>>4<<|>>1<<|>>681<<|>>20681<<|>>\\\\host\\Ref\\Symbols<<|>>1<<|>>RefData~4~681.zip\n\
             File<<|>>4<<|>>2<<|>>824<<|>>20824<<|>>\\\\host\\Ref\\Jacket.XML<<|>>2<<|>>Failed to access file.\n",
        )
        .unwrap();
        write_zip(&backup.join("RefData~4~681.zip"), &[("Symbols/valve.sym", b"s")]);
        let index = build_index(&backup, None).unwrap();

        let refdata: Vec<(&str, bool)> = index
            .files
            .iter()
            .filter(|file| file.kind == FileKind::RefData)
            .map(|file| (file.name.as_str(), file.missing))
            .collect();
        assert_eq!(
            refdata,
            vec![("RefData~4~681.zip", false), ("RefData~4~824", true)]
        );
        assert!(index
            .warnings
            .iter()
            .any(|warning| warning.contains("RefData~4~824")), "{:?}", index.warnings);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `RefData~4~703` is a spreadsheet: one file under another name, and a
    /// zip only in the sense every `.xlsx` is one. The manifest says so, and
    /// the index believes it rather than the bytes.
    #[test]
    fn a_file_stored_under_a_number_is_named_and_typed_by_the_manifest() {
        let root = scratch("file-records");
        let backup = root.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "Name<<|>>SQPlant\n\
             DBPwds<<|>>Zz#####a-made-up-obfuscated-value#####\n\
             File<<|>>4<<|>>1<<|>>681<<|>>20681<<|>>\\\\host\\P\\Ref\\Symbols<<|>>1<<|>>RefData~4~681.zip\n\
             FileSize<<|>>4<<|>>681<<|>>1<<|>>1<<|>>40960\n\
             File<<|>>4<<|>>2<<|>>703<<|>>20703<<|>>\\\\host\\P\\Ref\\exportlayer.xlsx<<|>>1<<|>>RefData~4~703\n\
             File<<|>>4<<|>>2<<|>>824<<|>>20824<<|>>\\\\host\\P\\Ref\\Jacket.XML<<|>>2<<|>>Failed to access file.\n",
        )
        .unwrap();
        write_zip(&backup.join("RefData~4~681.zip"), &[("Symbols/valve.sym", b"s")]);
        write_zip(
            &backup.join("RefData~4~703"),
            &[("[Content_Types].xml", b"x"), ("xl/workbook.xml", b"y")],
        );
        let index = build_index(&backup, None).unwrap();
        let file = |name: &str| {
            index
                .files
                .iter()
                .find(|file| file.name == name)
                .unwrap_or_else(|| panic!("{name} is listed"))
        };

        let sheet = file("RefData~4~703");
        assert_eq!(sheet.format, "XLSX");
        assert!(sheet.members.is_empty(), "the parts of a document are not members");
        assert_eq!(
            sheet.record.as_ref().map(|record| record.original.as_str()),
            Some("Ref\\exportlayer.xlsx")
        );
        // A folder is the archive it looks like.
        let symbols = file("RefData~4~681.zip");
        assert_eq!(symbols.format, "Zip");
        assert_eq!(symbols.members, ["Symbols/valve.sym"]);
        let record = symbols.record.as_ref().expect("the manifest names it");
        assert_eq!(record.original, "Ref\\Symbols");
        assert_eq!(record.bytes, Some(40960));
        // The file the backup failed to read is known by what it was, too.
        let missing = file("RefData~4~824");
        assert!(missing.missing);
        assert_eq!(
            missing.record.as_ref().map(|record| record.original.as_str()),
            Some("Ref\\Jacket.XML")
        );
        // The whole manifest is in the index, minus its credentials.
        let model = index.manifest_model.as_ref().expect("the manifest's model");
        assert_eq!(model.files.len(), 3);
        assert_eq!(model.hidden, 1);
        assert!(!format!("{index:?}").contains("a-made-up-obfuscated-value"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A backup as SmartPlant leaves it has no `Ref` folder: its reference
    /// data is in numbered volumes. The index's reference tree is those
    /// volumes, unpacked where the manifest says they came from.
    #[test]
    fn a_backup_without_a_ref_folder_lists_its_volumes_as_reference_data() {
        let root = scratch("volumes");
        let backup = root.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(
            backup.join("Manifest.txt"),
            "Name<<|>>SQPlant\r\n\
             File<<|>>2<<|>>1<<|>>711<<|>>1<<|>>\\\\host\\P\\Plant<<|>>1<<|>>PlantData~2~711.zip\r\n\
             File<<|>>4<<|>>1<<|>>681<<|>>1<<|>>\\\\host\\P\\Ref\\Symbols<<|>>1<<|>>RefData~4~681.zip\r\n\
             File<<|>>4<<|>>1<<|>>682<<|>>1<<|>>\\\\host\\P\\Ref\\Template Files<<|>>1<<|>>RefData~4~682.zip\r\n\
             File<<|>>4<<|>>2<<|>>680<<|>>1<<|>>\\\\host\\P\\Ref\\rules.rul<<|>>1<<|>>RefData~4~680\r\n",
        )
        .unwrap();
        write_zip(&backup.join("PlantData~2~711.zip"), &[("00/00/A.pid", b"a")]);
        write_zip(
            &backup.join("RefData~4~681.zip"),
            &[("Piping/gate.sym", b"gate"), ("CatalogIndex.xml", b"<c/>")],
        );
        write_zip(
            &backup.join("RefData~4~682.zip"),
            &[("A1.igr", b"border"), ("A1.pid", b"template drawing")],
        );
        std::fs::write(backup.join("RefData~4~680"), b"rules").unwrap();
        let target = root.join("out");
        let index = build_index(&backup, Some(&target)).unwrap();

        assert_eq!(index.ref_root, target.join("Ref"));
        let names = |nodes: &[RefNode]| -> Vec<String> {
            let mut names: Vec<String> = nodes.iter().map(|node| node.name.clone()).collect();
            names.sort();
            names
        };
        assert_eq!(names(&index.ref_tree.dirs), ["Symbols", "Template Files"]);
        assert_eq!(
            index
                .ref_tree
                .files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            ["rules.rul"]
        );
        assert_eq!(index.ref_tree.file_count(), 5);
        // A reference file is where the tree says it is.
        let rules = index.ref_file_path(&[], 0).expect("rules.rul has a path");
        assert_eq!(std::fs::read(&rules).unwrap(), b"rules");
        // The template drawing is reference data, not a drawing of the plant.
        assert_eq!(
            index
                .drawings
                .iter()
                .map(|drawing| drawing.name.as_str())
                .collect::<Vec<_>>(),
            ["A"]
        );
        // The backup folder itself is only read.
        assert!(!backup.join("Ref").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_drawing_without_items_groups_nothing() {
        let (groups, count) = group_published_items(Vec::new());
        assert!(groups.is_empty());
        assert_eq!(count, 0);
    }

    /// Both failure paths of [`build_index_in_background`] speak the UI
    /// language: the catalog already carries `pid-backup.cannot-start-indexing`
    /// and `pid-backup.index-stopped`, and a hard-coded English string here
    /// reaches the command line through `PIDBACKUPOPEN: {}` and the panel's
    /// idle error line, leaving a Chinese UI half English.
    ///
    /// The thread-start failure itself cannot be provoked on demand -- it
    /// needs `spawn` to fail, which happens only when the OS refuses a thread
    /// -- so this pins the mapping the failing call uses, not the `spawn` call.
    ///
    /// The UI language is one process-wide loader, so switching it here would
    /// leak into every test running beside this one (it turned a warning
    /// assertion in this very module Chinese). The assertions therefore run in
    /// a child process of this test binary, filtered to this test alone.
    #[test]
    fn the_index_failures_speak_the_ui_language() {
        const CHILD: &str = "OCS_PID_BACKUP_INDEX_ERRORS_CHILD";
        const NAME: &str = "io::pid_backup_browse::tests::the_index_failures_speak_the_ui_language";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(
                std::env::current_exe().expect("the test binary has a path"),
            )
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .output()
            .expect("the test binary must start");
            assert!(
                output.status.success(),
                "the child run failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        crate::i18n::set_language(crate::i18n::Language::ZhCn)
            .expect("Chinese Fluent resources must parse");
        let start = index_start_error(std::io::Error::other("boom"));
        assert_eq!(start, "无法开始建立备份索引：boom");
        assert_ne!(start, "cannot start indexing the backup: boom");
        assert_eq!(index_stopped_error(), "备份索引在完成前停止");
        assert_ne!(
            index_stopped_error(),
            "the backup index stopped before it finished"
        );
    }

    /// The two messages above only reach the user if `build_index_in_background`
    /// really routes through them; a later edit that inlines `format!` again
    /// would leave the catalog keys orphaned exactly as they were before.
    #[test]
    fn the_index_failures_route_through_the_catalog() {
        let source = include_str!("pid_backup_browse.rs");
        let body = source
            .split("pub async fn build_index_in_background")
            .nth(1)
            .expect("the background indexer is still declared");
        let body = &body[..body.find("\n}\n").expect("the function still ends at column 0")];
        assert!(body.contains("map_err(index_start_error)"), "{body}");
        assert!(body.contains("index_stopped_error()"), "{body}");
        assert!(
            !body.contains("cannot start indexing the backup"),
            "the start failure went back to a hard-coded string: {body}"
        );
        assert!(
            !body.contains("the backup index stopped before it finished"),
            "the stop failure went back to a hard-coded string: {body}"
        );
    }

    /// The panel hands both background loaders to `Task::perform`; this pins
    /// the signatures that call site depends on.
    #[test]
    fn both_background_loaders_fit_a_task() {
        let source = PathBuf::from("backup");
        let _: iced::Task<Result<BackupIndex, String>> = iced::Task::perform(
            build_index_in_background(source.clone(), None, None),
            std::convert::identity,
        );
        let _: iced::Task<Result<DrawingItems, String>> =
            iced::Task::perform(load_items_in_background(source), std::convert::identity);
    }

    /// The plant test backup, indexed and read: 53 drawings, an Oracle dump,
    /// 11 RefData files, 645 Ref files, and the first drawing's items.
    /// Soft-skips when the backup is not on this machine.
    #[test]
    #[ignore = "reads the 74 MB plant dump; run with --ignored"]
    fn the_plant_test_backup_indexes_and_reads_items() {
        let backup = Path::new(r"D:\work\plant-code\cad\pid-test-data");
        if !backup.join("Export.dmp").is_file() {
            eprintln!("skipped: no plant backup at {}", backup.display());
            return;
        }
        let started = std::time::Instant::now();
        let index = build_index(backup, None).unwrap();
        let index_elapsed = started.elapsed();

        assert!(index.read_only);
        assert_eq!(index.backup_root, backup);
        assert_eq!(
            index.manifest.as_ref().and_then(|manifest| manifest.name.as_deref()),
            Some("SQPlant")
        );
        let export = index
            .files
            .iter()
            .find(|file| file.kind == FileKind::ExportDump)
            .expect("the backup has an Export.dmp");
        assert!(
            export.format.starts_with("Oracle exp"),
            "{}",
            export.format
        );
        assert!(export.version_line.is_some(), "{}", export.format);
        let refdata: Vec<&BackupFile> = index
            .files
            .iter()
            .filter(|file| file.kind == FileKind::RefData)
            .collect();
        // Ten archives on disk plus the one `RefData~4~824` the manifest
        // declares but the export failed to write.
        assert_eq!(refdata.len(), 11);
        // Exactly one declared archive is missing, and it is the one the
        // export failed to write: `RefData~4~824`.
        let missing: Vec<&str> = refdata
            .iter()
            .filter(|file| file.missing)
            .map(|file| file.name.as_str())
            .collect();
        assert_eq!(
            missing,
            vec!["RefData~4~824"],
            "{:?}",
            refdata
                .iter()
                .map(|file| (&file.name, file.missing))
                .collect::<Vec<_>>()
        );
        assert_eq!(index.ref_tree.file_count(), 645);
        assert_eq!(index.warnings.len(), 1, "{:?}", index.warnings);
        assert!(index.warnings[0].contains("RefData~4~824"), "{:?}", index.warnings);

        assert_eq!(index.drawings.len(), 53);
        for drawing in &index.drawings {
            assert!(!drawing.meta.is_empty(), "{} has no metadata", drawing.name);
            assert!(
                !drawing.group_path.is_empty(),
                "{} has no plant group",
                drawing.name
            );
            let path = drawing
                .pid_path
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no file", drawing.name));
            assert!(path.is_file(), "{} is missing", path.display());
            // The row's PATH and the restored file line up without the
            // file-name fallback: `\00\00\name.pid` against `00/00/name.pid`.
            let row_key = drawing
                .meta
                .iter()
                .find(|(label, _)| label == "PATH")
                .map(|(_, value)| backup_path_key(value))
                .expect("every row lists its PATH");
            let file_key = backup_path_key(
                &path
                    .strip_prefix(&index.restored)
                    .unwrap_or(path)
                    .to_string_lossy(),
            );
            assert_eq!(row_key, file_key, "{} is matched by another rule", drawing.name);
        }

        let first = index.drawings[0]
            .pid_path
            .clone()
            .expect("the first drawing has a file");
        let started = std::time::Instant::now();
        let items = load_items(&first).unwrap();
        let items_elapsed = started.elapsed();
        eprintln!(
            "index: {} drawings, {} warnings in {index_elapsed:?}; load_items {} items in {items_elapsed:?}",
            index.drawings.len(),
            index.warnings.len(),
            items.item_count
        );
        assert!(items.item_count > 0, "{}", first.display());
        assert_eq!(
            items
                .groups
                .iter()
                .map(|group| group.items.len())
                .sum::<usize>(),
            items.item_count
        );
        assert!(items
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .all(|item| !item.uid.is_empty() && !item.item_type.is_empty()));
        // Not every item type publishes attributes: a run or a pipe point
        // carries none, while the instruments beside it carry several.
        assert!(
            items
                .groups
                .iter()
                .flat_map(|group| &group.items)
                .any(|item| !item.attributes.is_empty()),
            "no item of {} has an attribute",
            first.display()
        );
    }
}

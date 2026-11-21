//! The whole of an opened backup package as publish files, and an account of
//! what became of each drawing (SPEC-sppid-export §5).
//!
//! What the files hold is `pid-parse`'s to decide: the publish is its
//! whole-package entry, the one `pid_publish_xml` runs, and nothing it
//! writes is touched here. This module's part lies before and after.
//! Before: an export that cannot be done is refused with nothing written,
//! and the folder is the user's, so one that holds anything is not written
//! into. After: the index the publish leaves among its files,
//! `_Index.json`, is read for what became of each drawing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pid_parse::publish::backup_package::{
    publish_backup_package, BackupPublishError, BackupPublishReport, PackageChainFault,
    PackageDrawingShortfalls, PackageDuplicateUids,
};
use serde_json::Value;

use crate::io::pid_backup_pdf::{full, lies_in, panic_text, resolved};

/// The plant name the publish files state when none is asked for, as
/// `pid_publish_xml` has it.
pub const DEFAULT_PLANT: &str = "P01";

/// The file that makes a folder a backup package.
const MANIFEST_NAME: &str = "Manifest.txt";

/// What the index calls the database export of a SQL Server package.
const SQL_SERVER: &str = "SqlServerMtf";

/// The thread a background export runs on.
const THREAD_NAME: &str = "ocs-pid-backup-export";

/// The stack of that thread: what the export to PDF asks for.
const STACK_BYTES: usize = 64 * 1024 * 1024;

/// How many of the things in the way an error names; past that it counts.
const ENTRIES_NAMED: usize = 5;

/// How many of a kind the summary lists; past that it counts.
const LISTED: usize = 20;

/// What to export, where to, and under which plant name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    /// The folder that holds the backup's own files (`Manifest.txt`, ...),
    /// not the one the backup is restored to. The index states it as it is
    /// spelled here.
    pub package_dir: PathBuf,
    /// The folder the publish files go to: empty, or not there yet.
    pub out_dir: PathBuf,
    /// The plant name every `_Data.xml` and `_Meta.xml` states.
    pub plant: String,
    /// A folder of `<drawing>_Data.xml` files to compare what is written
    /// with; `None` to compare with nothing.
    pub reference_dir: Option<PathBuf>,
}

/// A drawing the index has something to say of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingNote {
    /// The drawing's name, as the index has it.
    pub name: String,
    /// What the index says of it: why it was not written, or what to know
    /// of the files that were.
    pub notes: Vec<String>,
}

/// What comparing the written drawings with a reference folder came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSummary {
    /// How many drawings are the same as their reference.
    pub clean: usize,
    /// How many differ from it.
    pub dirty: usize,
    /// How many have a reference that is another revision of the drawing.
    pub revision_mismatch: usize,
    /// How many have no reference.
    pub unpaired: usize,
    /// The lines `pid_publish_xml` prints of the comparison: a sentence for
    /// every drawing, each finding of one that differs after its own.
    pub lines: Vec<String>,
}

/// What an export came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOutcome {
    /// How many drawings were written, all three files of each.
    pub written: usize,
    /// `pid-parse`'s count of what it could not do. Not a number of
    /// drawings: an archive of the package that cannot be opened counts
    /// one, however many drawings it holds.
    pub failed: usize,
    /// How many `.pid` files the package holds.
    pub pid_entries: usize,
    /// How many of them are border templates.
    pub templates: usize,
    /// The ones that are no template and that no drawing of the database
    /// is matched to.
    pub unmatched: Vec<String>,
    /// How many written drawings state no `DrawingNo` and take their number
    /// from the database.
    pub without_drawing_no: usize,
    /// The drawings that were not written, each with the reason.
    pub unwritten: Vec<DrawingNote>,
    /// The drawings that were written with something to know of them.
    pub noted: Vec<DrawingNote>,
    /// What the index lists as problems, sentence by sentence.
    pub problems: Vec<String>,
    /// The ones of `problems` that `unwritten` and `noted` do not say: a
    /// sentence that is a note of a drawing listed there, under one of the
    /// names the index has for the drawing, is left out.
    pub other_problems: Vec<String>,
    /// The sentences `pid_publish_xml` prints as warnings, in its order.
    pub warnings: Vec<String>,
    /// The comparison with the reference folder; `None` when none was given.
    pub gate: Option<GateSummary>,
    /// What the package's database export is: `OracleExp`, `SqlServerMtf`,
    /// or `PidOnly` when the package holds none.
    pub export_kind: String,
    /// How many files the export wrote: the ones the index lists, and the
    /// index.
    pub files: usize,
    /// Where the index is.
    pub index_path: PathBuf,
    /// How long the export took.
    pub elapsed: Duration,
}

impl ExportOutcome {
    /// Whether the package is a SQL Server one: what the export of such a
    /// package writes has not been verified, and the summary says so.
    pub fn sql_server(&self) -> bool {
        self.export_kind == SQL_SERVER
    }
}

/// Why an export did not happen, or did not come to its end. A drawing that
/// cannot be written is none of these: the index says why, and the export
/// goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// No backup package is open. The command's to raise: an export that
    /// got this far was given one.
    NoPackage,
    /// The folder holds no `Manifest.txt`: loose drawings, not a package.
    NotABackup(PathBuf),
    /// `folder` holds something already: `count` things, `names` the first
    /// few. Nothing was written.
    TargetNotEmpty {
        folder: PathBuf,
        names: Vec<String>,
        count: usize,
    },
    /// The folder cannot be used: none was given, it is a file, it cannot
    /// be made or read, or it lies in the package.
    TargetInvalid(String),
    /// The reference folder given is not a folder.
    ReferenceInvalid(String),
    /// The last export has not finished. The command's to raise.
    Busy,
    /// `pid-parse` does not publish this package: its database export is of
    /// no kind it reads.
    Unsupported(String),
    /// The export as a whole could not be done: the package could not be
    /// read, the files could not be written, the index cannot be used, or
    /// the thread the export ran on is gone.
    Failed(String),
}

impl ExportError {
    /// The code scripts and tests tell the errors apart by
    /// (SPEC-sppid-export §3.3).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoPackage => "SPPID_EXPORT_NO_PACKAGE",
            Self::NotABackup(_) => "SPPID_EXPORT_NOT_A_BACKUP",
            Self::TargetNotEmpty { .. } => "SPPID_EXPORT_TARGET_NOT_EMPTY",
            Self::TargetInvalid(_) => "SPPID_EXPORT_TARGET_INVALID",
            Self::ReferenceInvalid(_) => "SPPID_EXPORT_REFERENCE_INVALID",
            Self::Busy => "SPPID_EXPORT_BUSY",
            Self::Unsupported(_) => "SPPID_EXPORT_UNSUPPORTED",
            Self::Failed(_) => "SPPID_EXPORT_FAILED",
        }
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPackage => f.write_str("no backup package is open"),
            Self::NotABackup(folder) => {
                let folder = folder.display();
                write!(
                    f,
                    "{folder} holds no {MANIFEST_NAME}, so it is not a backup package"
                )
            }
            Self::TargetNotEmpty {
                folder,
                names,
                count,
            } => {
                let folder = folder.display();
                let listed = names.join(", ");
                write!(f, "{folder} is not empty: it holds {listed}")?;
                if *count > names.len() {
                    write!(f, " and {} more", count - names.len())?;
                }
                f.write_str("; nothing was written")
            }
            Self::TargetInvalid(why)
            | Self::ReferenceInvalid(why)
            | Self::Unsupported(why)
            | Self::Failed(why) => f.write_str(why),
            Self::Busy => {
                f.write_str("the last export of the package to publish files has not finished")
            }
        }
    }
}

impl std::error::Error for ExportError {}

/// Why the export cannot write to `out_dir` -- the request's, in [`full`]
/// -- when it cannot. Nothing is made to find out.
fn unusable(out_dir: &Path, request: &ExportRequest) -> Option<String> {
    if request.out_dir.as_os_str().is_empty() {
        return Some("no folder to export to was given".to_string());
    }
    // What the folder would be made in decides whether it can be made.
    let existing = out_dir
        .ancestors()
        .find(|path| path.symlink_metadata().is_ok());
    if let Some(existing) = existing.filter(|path| !path.is_dir()) {
        let folder = out_dir.display();
        return Some(if existing == out_dir {
            format!("{folder} is a file, not a folder")
        } else {
            format!("{folder} cannot be made: {} is a file", existing.display())
        });
    }
    // The export writes nothing into the backup (SPEC-sppid-export §1).
    if !lies_in(&resolved(out_dir), &resolved(&request.package_dir)) {
        return None;
    }
    let (folder, package) = (out_dir.display(), request.package_dir.display());
    Some(format!(
        "{folder} lies in the backup package ({package}); export to a folder outside of it"
    ))
}

/// What `folder` holds, when it is there and holds anything: the names of
/// the first few things in it, and how many there are. The error is why
/// that cannot be told.
fn held(folder: &Path) -> Result<Option<(Vec<String>, usize)>, String> {
    let unread = |error: std::io::Error| format!("{} cannot be read: {error}", folder.display());
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unread(error)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let name = entry.map_err(unread)?.file_name();
        names.push(name.to_string_lossy().into_owned());
    }
    names.sort();
    let count = names.len();
    names.truncate(ENTRIES_NAMED);
    Ok((count > 0).then_some((names, count)))
}

/// Why `reference`, which is no folder, cannot be compared with.
fn no_reference(reference: &Path) -> String {
    let folder = reference.display();
    if reference.as_os_str().is_empty() {
        "no reference folder was named".to_string()
    } else if reference.symlink_metadata().is_ok() {
        format!("the reference folder {folder} is a file, not a folder")
    } else {
        format!("the reference folder {folder} does not exist")
    }
}

/// Whether the export of `request` can start: the package is one, the
/// folder can be used and holds nothing yet, and the reference folder,
/// when one is given, is a folder (SPEC-sppid-export §3.3). Only looks:
/// nothing is made or written.
pub fn check(request: &ExportRequest) -> Result<(), ExportError> {
    if !request.package_dir.join(MANIFEST_NAME).is_file() {
        return Err(ExportError::NotABackup(request.package_dir.clone()));
    }
    let folder = full(&request.out_dir);
    if let Some(why) = unusable(&folder, request) {
        return Err(ExportError::TargetInvalid(why));
    }
    if let Some((names, count)) = held(&folder).map_err(ExportError::TargetInvalid)? {
        return Err(ExportError::TargetNotEmpty {
            folder,
            names,
            count,
        });
    }
    match request.reference_dir.as_deref() {
        Some(reference) if !reference.is_dir() => {
            Err(ExportError::ReferenceInvalid(no_reference(reference)))
        }
        _ => Ok(()),
    }
}

/// Why the export cannot write to `out_dir` when the package is restored to
/// `restored`, when it cannot: the folder lies in the restore, which is the
/// restore cache's to clear, not the user's. A request names the package
/// and not where it is restored to, so [`check`] cannot tell; the command,
/// which knows both, asks this first (SPEC-sppid-export §11). Nothing is
/// made to find out.
pub fn in_the_restore(out_dir: &Path, restored: &Path) -> Option<String> {
    if !lies_in(&resolved(out_dir), &resolved(restored)) {
        return None;
    }
    let (folder, restore) = (out_dir.display(), restored.display());
    Some(format!(
        "{folder} lies in the folder the backup is restored to ({restore}); export to a \
         folder outside of it"
    ))
}

/// Publish the package into the request's folder and say what became of
/// its drawings. A drawing that cannot be written is in the outcome with
/// the reason, and the others are written all the same; what [`check`]
/// refuses is refused here too, before anything is made.
pub fn export(request: &ExportRequest) -> Result<ExportOutcome, ExportError> {
    export_with(request, |request, out_dir| {
        let reference = request.reference_dir.as_deref();
        publish_backup_package(&request.package_dir, out_dir, &request.plant, reference)
    })
}

/// [`export`] with the publish given: `publish(request, out_dir)` writes
/// the publish files of the request's package into `out_dir`, the request's
/// folder in [`full`] and made by then, and reports what it wrote.
fn export_with<P>(request: &ExportRequest, publish: P) -> Result<ExportOutcome, ExportError>
where
    P: FnOnce(&ExportRequest, &Path) -> Result<BackupPublishReport, BackupPublishError>,
{
    let started = Instant::now();
    check(request)?;
    let out_dir = full(&request.out_dir);
    let made = make(&out_dir)?;
    let outcome =
        published(request, &out_dir, publish).and_then(|report| accounted(report, started));
    if outcome.is_err() {
        unmake(&made);
    }
    outcome
}

/// Make `out_dir`, and say which folders that took, the innermost first.
fn make(out_dir: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let absent = |path: &&Path| {
        let found = path.symlink_metadata();
        found.is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    };
    let made = out_dir.ancestors().take_while(absent);
    let made: Vec<PathBuf> = made.map(Path::to_path_buf).collect();
    match std::fs::create_dir_all(out_dir) {
        Ok(()) => Ok(made),
        Err(error) => {
            unmake(&made);
            let folder = out_dir.display();
            Err(ExportError::TargetInvalid(format!(
                "{folder} cannot be made: {error}"
            )))
        }
    }
}

/// Take away again the folders -- the innermost first -- that were made for
/// an export that came to nothing. Only an empty folder goes: one that
/// holds anything stays, and with it every folder it lies in.
fn unmake(made: &[PathBuf]) {
    for folder in made {
        let _ = std::fs::remove_dir(folder);
    }
}

/// What `publish` came to, in this module's errors. A publish that panics
/// costs the export, not the program.
fn published<P>(
    request: &ExportRequest,
    out_dir: &Path,
    publish: P,
) -> Result<BackupPublishReport, ExportError>
where
    P: FnOnce(&ExportRequest, &Path) -> Result<BackupPublishReport, BackupPublishError>,
{
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| publish(request, out_dir)));
    match run {
        Ok(Ok(report)) => Ok(report),
        Ok(Err(BackupPublishError::Usage(why))) => Err(ExportError::Unsupported(why)),
        Ok(Err(BackupPublishError::Io(why))) => Err(ExportError::Failed(why)),
        Err(panic) => {
            let said = panic_text(panic.as_ref());
            Err(ExportError::Failed(format!("the export panicked: {said}")))
        }
    }
}

/// The outcome of the publish `report` is of, by the index it wrote. An
/// index that cannot be read or used fails the export, files written or
/// not: it is what accounts for every drawing (SPEC-sppid-export §3.2).
fn accounted(report: BackupPublishReport, started: Instant) -> Result<ExportOutcome, ExportError> {
    let index_path = report.index_path.clone();
    let failed = |why: String| {
        let index = index_path.display();
        ExportError::Failed(format!("the export ran, but its index {index} {why}"))
    };
    let index = std::fs::read(&index_path)
        .map_err(|error| failed(format!("could not be read: {error}")))?;
    outcome_of(report, &index, started.elapsed())
        .map_err(|why| failed(format!("cannot be used: {why}")))
}

/// The outcome of the publish `report` is of, whose index reads `index`:
/// what the report counts, and what the index says of each drawing
/// (SPEC-sppid-export §3.2). The error is why the index cannot be used: it
/// is not JSON, or one of the fields read is not there or not of its kind.
/// No field is made up for.
fn outcome_of(
    report: BackupPublishReport,
    index: &[u8],
    elapsed: Duration,
) -> Result<ExportOutcome, String> {
    let index: Value =
        serde_json::from_slice(index).map_err(|error| format!("it is not JSON: {error}"))?;
    let export_kind = text_of(&index, "export")?;
    let pid_entries = count_of(&index, "pid_entries")?;
    let templates = count_of(&index, "template_pid_entries")?;
    let unmatched = sentences_of(&index, "unmatched_pid_entries")?;
    let problems = sentences_of(&index, "problems")?;
    let (mut unwritten, mut noted) = (Vec::new(), Vec::new());
    let mut files = 0;
    // The sentences that say a note of a drawing under the drawing's name.
    let mut said = HashSet::new();
    for (at, drawing) in list_of(&index, "drawings")?.iter().enumerate() {
        let (note, listed, same) = drawing_of(drawing)
            .map_err(|why| format!("of drawing {} of its \"drawings\", {why}", at + 1))?;
        files += listed;
        said.extend(same);
        if listed == 0 {
            unwritten.push(note);
        } else if !note.notes.is_empty() {
            noted.push(note);
        }
    }
    let other_problems = problems.iter().filter(|problem| !said.contains(*problem));
    let other_problems = other_problems.cloned().collect();
    let warnings = warnings_of(&report);
    let gate = report.gate.map(|gate| GateSummary {
        clean: gate.clean,
        dirty: gate.dirty,
        revision_mismatch: gate.revision_mismatch,
        unpaired: gate.unpaired,
        lines: gate.lines,
    });
    Ok(ExportOutcome {
        written: report.written,
        failed: report.failed,
        pid_entries,
        templates,
        unmatched,
        without_drawing_no: report.shells,
        unwritten,
        noted,
        problems,
        other_problems,
        warnings,
        gate,
        export_kind,
        files: files + 1,
        index_path: report.index_path,
        elapsed,
    })
}

/// The sentences `pid_publish_xml` prints as warnings of a package, in its
/// order: the record chains that do not hold, what the database line wrote
/// in less than full, the UIDs written more than once.
fn warnings_of(report: &BackupPublishReport) -> Vec<String> {
    let chains = report.chain_faults.iter().map(PackageChainFault::describe);
    let short = report.shortfalls.iter();
    let short = short.flat_map(PackageDrawingShortfalls::describe);
    let again = report.duplicate_uids.iter();
    let again = again.map(PackageDuplicateUids::describe);
    chains.chain(short).chain(again).collect()
}

/// One entry of the index's `drawings`: the drawing with what the index
/// says of it, how many files it lists of it, and the sentences that say
/// the same of it among the index's `problems`.
fn drawing_of(drawing: &Value) -> Result<(DrawingNote, usize, Vec<String>), String> {
    let note = DrawingNote {
        name: text_of(drawing, "name")?,
        notes: sentences_of(drawing, "problems")?,
    };
    let files = list_of(drawing, "files")?.len();
    // The names the index's `problems` say a drawing's note under: the
    // drawing's, its entry in the package, its name with its UID.
    let stated = |field: &str| drawing.get(field).and_then(Value::as_str);
    let entry = stated("zip_file").zip(stated("zip_entry"));
    let entry = entry.map(|(file, entry)| format!("{file} {entry}"));
    let uid = stated("doc_uid").map(|uid| format!("{} {uid}", note.name));
    let names = [Some(note.name.clone()), entry, uid];
    let same = names.into_iter().flatten().flat_map(|name| {
        let notes = note.notes.iter();
        notes.map(move |text| format!("{name}: {text}"))
    });
    let same = same.collect();
    Ok((note, files, same))
}

/// The field `name` of `of`.
fn field_of<'a>(of: &'a Value, name: &str) -> Result<&'a Value, String> {
    of.get(name)
        .ok_or_else(|| format!("\"{name}\" is not there"))
}

fn text_of(of: &Value, name: &str) -> Result<String, String> {
    let text = field_of(of, name)?.as_str().map(str::to_string);
    text.ok_or_else(|| format!("\"{name}\" is not a text"))
}

fn count_of(of: &Value, name: &str) -> Result<usize, String> {
    let count = field_of(of, name)?.as_u64();
    let count = count.and_then(|count| usize::try_from(count).ok());
    count.ok_or_else(|| format!("\"{name}\" is not a count"))
}

fn list_of<'a>(of: &'a Value, name: &str) -> Result<&'a Vec<Value>, String> {
    let list = field_of(of, name)?.as_array();
    list.ok_or_else(|| format!("\"{name}\" is not a list"))
}

fn sentences_of(of: &Value, name: &str) -> Result<Vec<String>, String> {
    let texts = list_of(of, name)?.iter();
    let texts: Option<Vec<String>> = texts
        .map(|text| text.as_str().map(str::to_string))
        .collect();
    texts.ok_or_else(|| format!("\"{name}\" holds what is not a text"))
}

/// [`export`] on a worker thread: a plant's package takes a quarter of a
/// minute, which the UI thread must not wait out.
pub async fn export_in_background(request: ExportRequest) -> Result<ExportOutcome, ExportError> {
    in_background(move || export(&request)).await
}

/// Run `job` -- an export -- on a thread of its own and wait for what it
/// comes to.
async fn in_background<J>(job: J) -> Result<ExportOutcome, ExportError>
where
    J: FnOnce() -> Result<ExportOutcome, ExportError> + Send + 'static,
{
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name(THREAD_NAME.to_string())
        .stack_size(STACK_BYTES)
        .spawn(move || {
            let _ = sender.send(job());
        })
        .map_err(|error| {
            ExportError::Failed(format!("the export could not be started: {error}"))
        })?;
    let stopped = || ExportError::Failed("the export stopped before it finished".to_string());
    receiver.await.unwrap_or_else(|_| Err(stopped()))
}

/// What the command line says of a finished export, each line without the
/// command's name: how many drawings and files were written and how long
/// it took; the failure count and what became of the `.pid` files; how
/// many drawings state no `DrawingNo`, when any; where the index is; what
/// the comparison with the reference came to, when there was one; that
/// what a SQL Server package's export writes has not been verified. Then
/// four lists, each the first twenty of its kind under how many there are:
/// the drawings that were not written, each with the reason; the ones
/// written with notes; the other problems the index lists; the warnings
/// the index does not list.
pub fn summary_lines(outcome: &ExportOutcome) -> Vec<String> {
    let seconds = outcome.elapsed.as_secs_f64();
    let (written, files, failed) = (outcome.written, outcome.files, outcome.failed);
    let (pids, templates) = (outcome.pid_entries, outcome.templates);
    let unmatched = outcome.unmatched.len();
    let matched = pids.saturating_sub(templates + unmatched);
    let mut lines = vec![
        format!("wrote {written} drawing(s), {files} file(s), in {seconds:.1} s"),
        format!(
            "failure count {failed}; {pids} .pid file(s): {matched} matched to a drawing, \
             {templates} border template(s), {unmatched} unmatched"
        ),
    ];
    let unnumbered = outcome.without_drawing_no;
    if unnumbered > 0 {
        lines.push(format!(
            "{unnumbered} drawing(s) have no DrawingNo: their number is the database's"
        ));
    }
    lines.push(format!("index: {}", outcome.index_path.display()));
    if let Some(gate) = &outcome.gate {
        let (same, differ) = (gate.clean, gate.dirty);
        let (revised, alone) = (gate.revision_mismatch, gate.unpaired);
        let compared = same + differ + revised + alone;
        lines.push(format!(
            "compared with the reference: {compared} drawing(s), {same} the same, {differ} with \
             findings, {revised} of another revision, {alone} without a reference"
        ));
    }
    if outcome.sql_server() {
        let unverified = "a SQL Server package: what its export writes has not been verified";
        lines.push(unverified.to_string());
    }
    let by_name = |drawing: &DrawingNote| match drawing.notes.as_slice() {
        [] => format!("{}: no reason given", drawing.name),
        notes => format!("{}: {}", drawing.name, notes.join("; ")),
    };
    let in_index = ", see the index";
    let unwritten = outcome.unwritten.iter().map(by_name);
    list(
        &mut lines,
        "drawing(s) could not be written",
        unwritten,
        in_index,
    );
    let noted = outcome.noted.iter().map(by_name);
    list(
        &mut lines,
        "drawing(s) were written with notes",
        noted,
        in_index,
    );
    let others = outcome.other_problems.iter().cloned();
    list(
        &mut lines,
        "other problem(s) listed in the index",
        others,
        in_index,
    );
    let unlisted = outcome.warnings.iter();
    let unlisted = unlisted.filter(|warning| !outcome.problems.contains(warning));
    list(
        &mut lines,
        "warning(s) the index does not list",
        unlisted.cloned(),
        "",
    );
    lines
}

/// Add to `lines` how many `sentences` there are of `what`, and the first
/// [`LISTED`] of them; where there are more, how many more, and
/// `elsewhere` they are to be found. Nothing where there is none.
fn list<S>(lines: &mut Vec<String>, what: &str, sentences: S, elsewhere: &str)
where
    S: Iterator<Item = String>,
{
    let sentences: Vec<String> = sentences.collect();
    let count = sentences.len();
    if count == 0 {
        return;
    }
    lines.push(format!("{count} {what}:"));
    let first = sentences.iter().take(LISTED);
    lines.extend(first.map(|sentence| format!("  {sentence}")));
    if count > LISTED {
        lines.push(format!("  ... and {} more{elsewhere}", count - LISTED));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;
    use pid_parse::model::StreamChainFault;
    use pid_parse::publish::backup_package::PackageGateReport;
    use pid_parse::publish::model::DatabaseShortfalls;
    use pid_parse::publish::pid_load::PublishChainFault;
    use std::io::Write;

    /// The index a publish ends with, among the files it wrote.
    const INDEX_NAME: &str = "_Index.json";

    /// What a drawing whose document states one UID twice is said of.
    const AGAIN: &str = "1 UID written more than once: 9F ×2";

    /// What the summary says of a SQL Server package.
    const UNVERIFIED: &str = "a SQL Server package: what its export writes has not been verified";

    /// What a folder holds, by path from it: a file with its bytes, a
    /// folder with none.
    type Tree = Vec<(String, Option<Vec<u8>>)>;

    /// An empty folder of one test's own.
    fn scratch(name: &str) -> PathBuf {
        let name = format!("ocs-backup-export-{name}-{}", std::process::id());
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A request to export a made-up package under `root` -- a manifest and
    /// nothing else, in `root/package` -- to `root/out`.
    fn request(root: &Path) -> ExportRequest {
        let package_dir = root.join("package");
        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::write(package_dir.join(MANIFEST_NAME), "a manifest\n").unwrap();
        ExportRequest {
            package_dir,
            out_dir: root.join("out"),
            plant: DEFAULT_PLANT.to_string(),
            reference_dir: None,
        }
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

    fn tree(folder: &Path) -> Tree {
        fn walk(folder: &Path, from: &Path, found: &mut Tree) {
            for entry in std::fs::read_dir(folder).unwrap() {
                let path = entry.unwrap().path();
                let name = path.strip_prefix(from).unwrap().to_string_lossy();
                if path.is_dir() {
                    found.push((name.into_owned(), None));
                    walk(&path, from, found);
                } else {
                    found.push((name.into_owned(), Some(std::fs::read(&path).unwrap())));
                }
            }
        }
        let mut found = Tree::new();
        walk(folder, folder, &mut found);
        found.sort();
        found
    }

    /// What refuses `request`: the same from [`check`] and from the export,
    /// which does not get as far as the publish. Nothing under `root` is
    /// made, written or taken away by either.
    fn refusal(root: &Path, request: &ExportRequest) -> ExportError {
        let before = tree(root);
        let checked = check(request).expect_err("the check lets the request through");
        let mut reached = false;
        let exported = export_with(request, |_, _| {
            reached = true;
            Err(BackupPublishError::Io(
                "the publish was reached".to_string(),
            ))
        });
        assert_eq!(exported, Err(checked.clone()), "the export, then the check");
        assert!(!reached, "the publish was reached");
        assert_eq!(tree(root), before, "what {} holds changed", root.display());
        checked
    }

    /// The report of a publish into `out_dir` that has nothing to count.
    fn report(out_dir: &Path) -> BackupPublishReport {
        BackupPublishReport {
            failed: 0,
            written: 0,
            index_path: out_dir.join(INDEX_NAME),
            pid_entries: 0,
            shells: 0,
            gate: None,
            chain_faults: Vec::new(),
            shortfalls: Vec::new(),
            duplicate_uids: Vec::new(),
        }
    }

    fn texts<const N: usize>(texts: [&str; N]) -> Vec<String> {
        texts.map(str::to_string).to_vec()
    }

    fn note(name: &str, notes: &[&str]) -> DrawingNote {
        DrawingNote {
            name: name.to_string(),
            notes: notes.iter().map(|note| (*note).to_string()).collect(),
        }
    }

    /// `drawing` as one whose document states a UID twice.
    fn stated_again(drawing: &str) -> PackageDuplicateUids {
        PackageDuplicateUids {
            drawing_uid: format!("uid of {drawing}"),
            drawing_name: drawing.to_string(),
            uids: vec![("9F".to_string(), 2)],
        }
    }

    /// `drawing` as one the database line wrote with a placeholder.
    fn written_short(drawing: &str) -> PackageDrawingShortfalls {
        let mut shortfalls = DatabaseShortfalls::default();
        shortfalls
            .placeholder_objects
            .insert("Exchanger".to_string(), 1);
        PackageDrawingShortfalls {
            drawing_uid: format!("uid of {drawing}"),
            drawing_name: drawing.to_string(),
            shortfalls,
        }
    }

    /// `entry` of the package as a drawing with a stream whose header
    /// states one record more than the stream holds.
    fn chain_short(entry: &str) -> PackageChainFault {
        let fault = StreamChainFault {
            stream_path: "/Sheet6".to_string(),
            stream_len: 4096,
            wrong_magic: None,
            stop: None,
            unread_bytes: 0,
            records_read: 12,
            live_records_read: 10,
            stated_records: Some(11),
        };
        PackageChainFault {
            zip_file: "PlantData~2~1.zip".to_string(),
            zip_entry: entry.to_string(),
            fault: PublishChainFault {
                fault,
                publishable_records_read: 10,
            },
        }
    }

    /// The outcome of the plant package's export, as SPEC-sppid-export
    /// §5.2 has it.
    fn plain() -> ExportOutcome {
        ExportOutcome {
            written: 53,
            failed: 0,
            pid_entries: 53,
            templates: 0,
            unmatched: Vec::new(),
            without_drawing_no: 2,
            unwritten: Vec::new(),
            noted: Vec::new(),
            problems: Vec::new(),
            other_problems: Vec::new(),
            warnings: Vec::new(),
            gate: None,
            export_kind: "OracleExp".to_string(),
            files: 160,
            index_path: Path::new("out").join(INDEX_NAME),
            elapsed: Duration::from_millis(15_234),
        }
    }

    /// An index of seven `.pid` files. A-101 is written. B-202, D-404 and
    /// E-505 are not: the index's `problems` say the reason of the first
    /// under its entry in the package, of the second in other words, of
    /// the third under its name and UID. C-303 is written with two notes,
    /// one of them also among the `problems` under its name. The other two
    /// files are a template and one that is neither.
    const INDEX: &str = r#"{
  "package": "D:\\plant",
  "export": "OracleExp",
  "pid_entries": 7,
  "failed": 4,
  "template_pid_entries": 1,
  "unmatched_pid_entries": ["PlantData~2~1.zip:Loose/L-900.pid"],
  "drawings": [
    {"doc_uid": "AA01", "name": "A-101", "zip_file": "PlantData~2~1.zip",
     "zip_entry": "Unit/A-101.pid",
     "files": ["A-101_Data.xml", "A-101_Meta.xml", "A-101_Coverage.json"], "problems": []},
    {"doc_uid": "", "name": "B-202", "zip_file": "PlantData~2~1.zip",
     "zip_entry": "Unit/B-202.pid", "files": [], "problems": ["not a compound file"]},
    {"doc_uid": "CC03", "name": "C-303", "zip_file": "PlantData~2~1.zip",
     "zip_entry": "Unit/C-303.pid",
     "files": ["C-303_Data.xml", "C-303_Meta.xml", "C-303_Coverage.json"],
     "problems": ["1 UID written more than once: 9F ×2", "a column was not read: T_X"]},
    {"name": "D-404", "files": [], "problems": ["read: too large", "left out"]},
    {"doc_uid": "EE05", "name": "E-505", "zip_file": "", "zip_entry": "", "files": [],
     "problems": ["no such table"]}
  ],
  "problems": [
    "D:\\plant\\PlantData~2~9.zip: cannot be opened",
    "PlantData~2~1.zip Unit/B-202.pid: not a compound file",
    "PlantData~2~1.zip Unit/C-303.pid: stream Sheet6 stops short",
    "C-303: 1 UID written more than once: 9F ×2",
    "PlantData~2~1.zip Unit/D-404.pid: too large",
    "E-505 EE05: no such table"
  ]
}"#;

    #[test]
    fn a_folder_without_a_manifest_is_not_exported() {
        let root = scratch("no-manifest");
        let mut request = request(&root);
        let package = request.package_dir.clone();
        std::fs::remove_file(package.join(MANIFEST_NAME)).unwrap();
        std::fs::write(package.join("A-101.pid"), "a loose drawing").unwrap();
        let error = refusal(&root, &request);
        assert_eq!(error, ExportError::NotABackup(package.clone()));
        assert_eq!(error.code(), "SPPID_EXPORT_NOT_A_BACKUP");
        let said = format!(
            "{} holds no Manifest.txt, so it is not a backup package",
            package.display()
        );
        assert_eq!(error.to_string(), said);

        // A folder of that name is no manifest, and neither is a manifest
        // in a folder below.
        std::fs::create_dir(package.join(MANIFEST_NAME)).unwrap();
        assert_eq!(refusal(&root, &request).code(), "SPPID_EXPORT_NOT_A_BACKUP");
        std::fs::remove_dir(package.join(MANIFEST_NAME)).unwrap();
        let below = package.join("below");
        std::fs::create_dir(&below).unwrap();
        std::fs::write(below.join(MANIFEST_NAME), "a manifest\n").unwrap();
        assert_eq!(refusal(&root, &request).code(), "SPPID_EXPORT_NOT_A_BACKUP");

        // The folder the manifest is in is let through.
        request.package_dir = below;
        assert_eq!(check(&request), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_holds_anything_is_not_written_into() {
        let root = scratch("not-empty");
        let request = request(&root);
        let out = request.out_dir.clone();

        // An empty folder in it is something in it.
        std::fs::create_dir_all(out.join("kept")).unwrap();
        let error = refusal(&root, &request);
        let held = ExportError::TargetNotEmpty {
            folder: out.clone(),
            names: texts(["kept"]),
            count: 1,
        };
        assert_eq!(error, held);
        assert_eq!(error.code(), "SPPID_EXPORT_TARGET_NOT_EMPTY");
        let said = format!(
            "{} is not empty: it holds kept; nothing was written",
            out.display()
        );
        assert_eq!(error.to_string(), said);

        // What an earlier export left, and more than the error names: the
        // first five by name, and the count.
        let left = [
            "_Index.json",
            "e.txt",
            "A-101_Data.xml",
            "d.txt",
            "c.txt",
            "b.txt",
        ];
        for name in left {
            std::fs::write(out.join(name), format!("left as {name}")).unwrap();
        }
        let error = refusal(&root, &request);
        let held = ExportError::TargetNotEmpty {
            folder: out.clone(),
            names: texts(["A-101_Data.xml", "_Index.json", "b.txt", "c.txt", "d.txt"]),
            count: 7,
        };
        assert_eq!(error, held);
        let said = format!(
            "{} is not empty: it holds A-101_Data.xml, _Index.json, b.txt, c.txt, d.txt and 2 \
             more; nothing was written",
            out.display()
        );
        assert_eq!(error.to_string(), said);

        // However the folder is spelled, the error names it in full.
        let mut spelled = request.clone();
        spelled.out_dir = root.join("elsewhere").join("..").join("out");
        assert_eq!(refusal(&root, &spelled), held);

        // There, and empty: the export may write into it.
        std::fs::remove_dir_all(&out).unwrap();
        std::fs::create_dir(&out).unwrap();
        assert_eq!(check(&request), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_that_is_no_folder_to_write_into_is_refused() {
        let root = scratch("invalid");
        let mut request = request(&root);
        let code = "SPPID_EXPORT_TARGET_INVALID";

        // None given.
        request.out_dir = PathBuf::new();
        let error = refusal(&root, &request);
        let none = "no folder to export to was given";
        assert_eq!(error, ExportError::TargetInvalid(none.to_string()));
        assert_eq!(error.code(), code);

        // A file, and a folder that would have to be made in a file.
        let file = root.join("a file");
        std::fs::write(&file, "kept").unwrap();
        request.out_dir.clone_from(&file);
        let error = refusal(&root, &request);
        let said = format!("{} is a file, not a folder", file.display());
        assert_eq!(error, ExportError::TargetInvalid(said));
        request.out_dir = file.join("below").join("out");
        let error = refusal(&root, &request);
        let said = format!(
            "{} cannot be made: {} is a file",
            request.out_dir.display(),
            file.display()
        );
        assert_eq!(error, ExportError::TargetInvalid(said));

        // A name that cannot be asked what it holds.
        request.out_dir = root.join("a\0b");
        let error = refusal(&root, &request);
        assert_eq!(error.code(), code, "{error}");
        let said = error.to_string();
        assert!(said.contains(" cannot be read: "), "{said}");

        // Beside the package, under a name that begins with the package's:
        // another folder. The check that lets it through makes nothing.
        request.out_dir = root.join("package-out").join("deeper");
        let before = tree(&root);
        assert_eq!(check(&request), Ok(()));
        assert_eq!(tree(&root), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_in_the_package_is_not_exported_to() {
        let root = scratch("inside");
        let mut request = request(&root);
        let package = request.package_dir.clone();
        // The package itself, a folder in it that is not there yet, one
        // deeper, and one spelled another way.
        let mut inside = vec![
            package.clone(),
            package.join("out"),
            package.join("deep").join("out"),
            root.join("elsewhere")
                .join("..")
                .join("package")
                .join("out"),
        ];
        if cfg!(windows) {
            // Windows takes names that differ in letter case for one, and
            // has a second spelling for a path.
            inside.push(root.join("PACKAGE").join("Out"));
            let plain = package.join("out").display().to_string();
            if !plain.starts_with(r"\\") {
                inside.push(PathBuf::from(format!(r"\\?\{plain}")));
            }
        }
        for out_dir in inside {
            request.out_dir = out_dir;
            let error = refusal(&root, &request);
            let said = error.to_string();
            assert_eq!(error.code(), "SPPID_EXPORT_TARGET_INVALID", "{said}");
            let why = format!(
                " lies in the backup package ({}); export to a folder outside of it",
                package.display()
            );
            assert!(said.ends_with(&why), "{said}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_in_the_restore_is_one_the_command_does_not_export_to() {
        let root = scratch("restore");
        let restored = root.join("restored");
        std::fs::create_dir_all(&restored).unwrap();
        // The restore itself, a folder in it that is not there yet, one
        // deeper, and one spelled another way.
        let mut inside = vec![
            restored.clone(),
            restored.join("out"),
            restored.join("deep").join("out"),
            root.join("elsewhere")
                .join("..")
                .join("restored")
                .join("out"),
        ];
        if cfg!(windows) {
            inside.push(root.join("RESTORED").join("Out"));
        }
        for out_dir in inside {
            let why = format!(
                "{} lies in the folder the backup is restored to ({}); export to a folder \
                 outside of it",
                out_dir.display(),
                restored.display()
            );
            let said = in_the_restore(&out_dir, &restored);
            assert_eq!(said, Some(why), "{}", out_dir.display());
        }
        // Beside it, above it, or with no restore to speak of: nothing.
        for out_dir in [root.join("restored out"), root.join("out"), root.clone()] {
            let said = in_the_restore(&out_dir, &restored);
            assert_eq!(said, None, "{}", out_dir.display());
        }
        assert_eq!(in_the_restore(Path::new("out"), Path::new("")), None);
        let made = std::fs::read_dir(&restored).unwrap().count();
        assert_eq!(made, 0, "nothing is made");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_refused_and_nothing_is_left() {
        let root = scratch("unmade");
        let mut request = request(&root);
        // No folder has a name this long. That shows when it is made, and
        // not before: the folder it would lie in is not there either.
        request.out_dir = root.join("made").join("x".repeat(300));
        let before = tree(&root);
        let exported = export_with(&request, |_, _| panic!("the publish was reached"));
        let error = exported.expect_err("a folder of that name was made");
        let said = error.to_string();
        assert_eq!(error.code(), "SPPID_EXPORT_TARGET_INVALID", "{said}");
        let start = format!("{} cannot be made: ", request.out_dir.display());
        assert!(said.starts_with(&start), "{said}");
        // The folder made on the way is gone again.
        assert_eq!(tree(&root), before);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_reference_that_is_no_folder_is_refused() {
        let root = scratch("reference");
        let mut request = request(&root);
        let code = "SPPID_EXPORT_REFERENCE_INVALID";

        let missing = root.join("no such folder");
        request.reference_dir = Some(missing.clone());
        let error = refusal(&root, &request);
        let said = format!("the reference folder {} does not exist", missing.display());
        assert_eq!(error, ExportError::ReferenceInvalid(said));
        assert_eq!(error.code(), code);

        let file = root.join("a file");
        std::fs::write(&file, "kept").unwrap();
        request.reference_dir = Some(file.clone());
        let error = refusal(&root, &request);
        let said = format!(
            "the reference folder {} is a file, not a folder",
            file.display()
        );
        assert_eq!(error, ExportError::ReferenceInvalid(said));

        request.reference_dir = Some(PathBuf::new());
        let error = refusal(&root, &request);
        let unnamed = "no reference folder was named";
        assert_eq!(error, ExportError::ReferenceInvalid(unnamed.to_string()));

        // A folder is let through, whatever it holds.
        request.reference_dir = Some(root.clone());
        assert_eq!(check(&request), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn of_the_reasons_to_refuse_the_first_is_given() {
        let root = scratch("first");
        let mut request = request(&root);
        let manifest = request.package_dir.join(MANIFEST_NAME);
        std::fs::remove_file(&manifest).unwrap();
        std::fs::create_dir(&request.out_dir).unwrap();
        std::fs::write(request.out_dir.join("kept"), "kept").unwrap();
        request.reference_dir = Some(root.join("no such folder"));

        // No package, a folder with something in it, no reference: the
        // package first, then the folder.
        assert_eq!(refusal(&root, &request).code(), "SPPID_EXPORT_NOT_A_BACKUP");
        std::fs::write(&manifest, "a manifest\n").unwrap();
        let code = refusal(&root, &request).code();
        assert_eq!(code, "SPPID_EXPORT_TARGET_NOT_EMPTY");
        // A folder that cannot be used before a reference that cannot.
        request.out_dir = request.out_dir.join("kept");
        assert_eq!(
            refusal(&root, &request).code(),
            "SPPID_EXPORT_TARGET_INVALID"
        );
        request.out_dir = root.join("other");
        let code = refusal(&root, &request).code();
        assert_eq!(code, "SPPID_EXPORT_REFERENCE_INVALID");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_index_says_what_became_of_each_drawing() {
        let out = Path::new("out");
        let counted = || BackupPublishReport {
            failed: 4,
            written: 2,
            // Not the index's: the index's is the one reported.
            pid_entries: 99,
            shells: 1,
            ..report(out)
        };
        let elapsed = Duration::from_secs(3);
        let outcome = outcome_of(counted(), INDEX.as_bytes(), elapsed).expect("the index");
        let expected = ExportOutcome {
            written: 2,
            failed: 4,
            pid_entries: 7,
            templates: 1,
            unmatched: texts(["PlantData~2~1.zip:Loose/L-900.pid"]),
            without_drawing_no: 1,
            unwritten: vec![
                note("B-202", &["not a compound file"]),
                note("D-404", &["read: too large", "left out"]),
                note("E-505", &["no such table"]),
            ],
            noted: vec![note("C-303", &[AGAIN, "a column was not read: T_X"])],
            problems: texts([
                "D:\\plant\\PlantData~2~9.zip: cannot be opened",
                "PlantData~2~1.zip Unit/B-202.pid: not a compound file",
                "PlantData~2~1.zip Unit/C-303.pid: stream Sheet6 stops short",
                "C-303: 1 UID written more than once: 9F ×2",
                "PlantData~2~1.zip Unit/D-404.pid: too large",
                "E-505 EE05: no such table",
            ]),
            // What is said of B-202, C-303 and E-505 above is not said
            // again; what is said of D-404 in other words is.
            other_problems: texts([
                "D:\\plant\\PlantData~2~9.zip: cannot be opened",
                "PlantData~2~1.zip Unit/C-303.pid: stream Sheet6 stops short",
                "PlantData~2~1.zip Unit/D-404.pid: too large",
            ]),
            warnings: Vec::new(),
            gate: None,
            export_kind: "OracleExp".to_string(),
            files: 7,
            index_path: out.join(INDEX_NAME),
            elapsed,
        };
        assert_eq!(outcome, expected);
        assert!(!outcome.sql_server());
        let lines = summary_lines(&outcome);
        assert!(!lines.iter().any(|line| line == UNVERIFIED));

        // The same of a SQL Server package: it is one, and said to be.
        let index = INDEX.replace("\"OracleExp\"", "\"SqlServerMtf\"");
        let outcome = outcome_of(counted(), index.as_bytes(), elapsed).expect("the index");
        assert!(outcome.sql_server());
        let lines = summary_lines(&outcome);
        assert!(lines.iter().any(|line| line == UNVERIFIED));
        let expected = ExportOutcome {
            export_kind: "SqlServerMtf".to_string(),
            ..expected
        };
        assert_eq!(outcome, expected);

        // The index of a package with nothing in it.
        let empty = r#"{"export": "PidOnly", "pid_entries": 0, "template_pid_entries": 0,
            "unmatched_pid_entries": [], "drawings": [], "problems": []}"#;
        let outcome = outcome_of(report(out), empty.as_bytes(), elapsed).expect("the index");
        assert_eq!(outcome.export_kind, "PidOnly");
        assert_eq!((outcome.pid_entries, outcome.files), (0, 1));
        assert!(outcome.unwritten.is_empty() && outcome.noted.is_empty());
        assert!(outcome.problems.is_empty() && outcome.other_problems.is_empty());
    }

    #[test]
    fn the_export_gives_the_publish_the_folder_and_reads_its_index() {
        let root = scratch("wiring");
        let mut request = request(&root);
        request.plant = "SQPlant".to_string();
        request.reference_dir = Some(root.clone());
        // Not there yet, two folders deep, and not spelled plainly.
        request.out_dir = root.join("elsewhere").join("..").join("deep").join("out");
        let out = root.join("deep").join("out");
        let verdicts = texts([
            "A-101: clean (objects 1, values 2, rels 3)",
            "C-303: unpaired",
        ]);

        let took = Duration::from_millis(20);

        let mut given = None;
        let outcome = export_with(&request, |request, out_dir| {
            given = Some((request.clone(), out_dir.to_path_buf(), tree(out_dir)));
            std::thread::sleep(took);
            std::fs::write(out_dir.join(INDEX_NAME), INDEX).unwrap();
            Ok(BackupPublishReport {
                failed: 4,
                written: 2,
                shells: 1,
                // No two of the counts the same, to tell them apart.
                gate: Some(PackageGateReport {
                    lines: verdicts.clone(),
                    clean: 1,
                    dirty: 2,
                    unpaired: 3,
                    revision_mismatch: 4,
                }),
                chain_faults: vec![chain_short("Unit/C-303.pid")],
                shortfalls: vec![written_short("C-303")],
                duplicate_uids: vec![stated_again("C-303")],
                ..report(out_dir)
            })
        });
        let outcome = outcome.expect("the export");

        // The publish is given the request as it is, and the folder in
        // full, made and empty.
        assert_eq!(given, Some((request.clone(), out.clone(), Tree::new())));
        // The time the export took is the publish's as well.
        assert!(outcome.elapsed >= took, "{:?}", outcome.elapsed);
        assert_eq!((outcome.written, outcome.failed), (2, 4));
        assert_eq!(outcome.without_drawing_no, 1);
        // The warnings as `pid_publish_xml` prints them: the chain that
        // does not hold, then what is written in less than full, then what
        // is stated again.
        let mut warnings = vec![chain_short("Unit/C-303.pid").describe()];
        warnings.extend(written_short("C-303").describe());
        warnings.push(format!("C-303: {AGAIN}"));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(outcome.warnings, warnings);
        let gate = GateSummary {
            clean: 1,
            dirty: 2,
            revision_mismatch: 4,
            unpaired: 3,
            lines: verdicts,
        };
        assert_eq!(outcome.gate, Some(gate));
        assert_eq!(outcome.index_path, out.join(INDEX_NAME));
        // What the index says is in the outcome as well.
        assert_eq!((outcome.pid_entries, outcome.files), (7, 7));
        assert_eq!(outcome.unwritten.len(), 3, "{:?}", outcome.unwritten);
        // Nothing but what the publish wrote is in the folder.
        let index = (INDEX_NAME.to_string(), Some(INDEX.as_bytes().to_vec()));
        assert_eq!(tree(&out), [index]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_index_that_cannot_be_used_fails_the_export() {
        let root = scratch("index");
        let request = request(&root);
        let out = request.out_dir.clone();
        let index_path = out.join(INDEX_NAME);
        // What the publish left as its index, and what the export says of
        // it after "the export ran, but its index <path> ".
        let whole = INDEX.to_string();
        let files_of_b = "\"zip_entry\": \"Unit/B-202.pid\", \"files\": [],";
        let cases = [
            (None, "could not be read: "),
            (
                Some("{ not JSON".to_string()),
                "cannot be used: it is not JSON: ",
            ),
            (
                Some("[]".to_string()),
                "cannot be used: \"export\" is not there",
            ),
            (
                Some(whole.replace("\"OracleExp\"", "7")),
                "cannot be used: \"export\" is not a text",
            ),
            (
                Some(whole.replace("\"pid_entries\": 7", "\"pid_entries\": -7")),
                "cannot be used: \"pid_entries\" is not a count",
            ),
            (
                Some(whole.replace("\"template_pid_entries\": 1,", "")),
                "cannot be used: \"template_pid_entries\" is not there",
            ),
            (
                Some(whole.replace("[\"PlantData~2~1.zip:Loose/L-900.pid\"]", "[1]")),
                "cannot be used: \"unmatched_pid_entries\" holds what is not a text",
            ),
            (
                Some(whole.replace("[\"PlantData~2~1.zip:Loose/L-900.pid\"]", "{}")),
                "cannot be used: \"unmatched_pid_entries\" is not a list",
            ),
            (
                Some(whole.replace("\"drawings\": [", "\"sheets\": [")),
                "cannot be used: \"drawings\" is not there",
            ),
            (
                Some(whole.replace(files_of_b, "\"zip_entry\": \"Unit/B-202.pid\",")),
                "cannot be used: of drawing 2 of its \"drawings\", \"files\" is not there",
            ),
            (
                Some(whole.replace("{\"name\": \"D-404\"", "{\"name\": null")),
                "cannot be used: of drawing 4 of its \"drawings\", \"name\" is not a text",
            ),
            (
                Some(whole.replace("\"left out\"]", "{}]")),
                "cannot be used: of drawing 4 of its \"drawings\", \"problems\" holds what is \
                 not a text",
            ),
            (
                Some(whole.replace("\"E-505 EE05: no such table\"", "5")),
                "cannot be used: \"problems\" holds what is not a text",
            ),
        ];
        for (index, why) in cases {
            let _ = std::fs::remove_dir_all(&out);
            assert_ne!(index.as_deref(), Some(INDEX), "{why}");
            let left = index.clone();
            let exported = export_with(&request, |_, out_dir| {
                std::fs::write(out_dir.join("A-101_Data.xml"), "a drawing").unwrap();
                if let Some(index) = &left {
                    std::fs::write(out_dir.join(INDEX_NAME), index).unwrap();
                }
                Ok(report(out_dir))
            });
            let error = exported.expect_err(why);
            let said = error.to_string();
            assert_eq!(error.code(), "SPPID_EXPORT_FAILED", "{said}");
            let start = format!(
                "the export ran, but its index {} {why}",
                index_path.display()
            );
            assert!(said.starts_with(&start), "{said}\n{start}");
            // What the publish wrote stays as it wrote it.
            let mut kept = vec![("A-101_Data.xml".to_string(), Some(b"a drawing".to_vec()))];
            if let Some(index) = index {
                kept.push((INDEX_NAME.to_string(), Some(index.into_bytes())));
            }
            assert_eq!(tree(&out), kept, "{why}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_publish_that_fails_fails_the_export_and_leaves_no_folder() {
        let root = scratch("errors");
        let mut request = request(&root);
        request.out_dir = root.join("deep").join("deeper").join("out");
        let before = tree(&root);

        // What `pid-parse` does not publish, and what it could not read.
        let unread = "Export.dmp is not an Oracle exp dump: no TAPE descriptor";
        let exported = export_with(&request, |_, _| {
            Err(BackupPublishError::Usage(unread.to_string()))
        });
        let error = exported.expect_err("a usage error");
        assert_eq!(error, ExportError::Unsupported(unread.to_string()));
        assert_eq!(error.code(), "SPPID_EXPORT_UNSUPPORTED");
        assert_eq!(tree(&root), before, "the folders made for it are gone");

        let denied = "read Export.dmp: access is denied";
        let exported = export_with(&request, |_, _| {
            Err(BackupPublishError::Io(denied.to_string()))
        });
        let error = exported.expect_err("a read error");
        assert_eq!(error, ExportError::Failed(denied.to_string()));
        assert_eq!(error.code(), "SPPID_EXPORT_FAILED");
        assert_eq!(tree(&root), before);

        // A publish that panics is a failed export.
        let exported = export_with(&request, |_, _| panic!("no such table"));
        let error = exported.expect_err("a panic");
        let said = "the export panicked: no such table";
        assert_eq!(error, ExportError::Failed(said.to_string()));
        assert_eq!(tree(&root), before);

        // A folder that was there stays, empty as it was.
        std::fs::create_dir_all(&request.out_dir).unwrap();
        let there = tree(&root);
        let exported = export_with(&request, |_, _| {
            Err(BackupPublishError::Io(denied.to_string()))
        });
        assert_eq!(exported, Err(ExportError::Failed(denied.to_string())));
        assert_eq!(tree(&root), there);
        std::fs::remove_dir_all(root.join("deep")).unwrap();

        // What a publish wrote before it failed stays, and so do the
        // folders it lies in.
        let full_disk = "write _Index.json: the disk is full";
        let exported = export_with(&request, |_, out_dir| {
            std::fs::write(out_dir.join("A-101_Data.xml"), "a drawing").unwrap();
            Err(BackupPublishError::Io(full_disk.to_string()))
        });
        assert_eq!(exported, Err(ExportError::Failed(full_disk.to_string())));
        let written = request.out_dir.join("A-101_Data.xml");
        assert_eq!(std::fs::read(written).unwrap(), b"a drawing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_error_has_its_code_and_says_why_without_it() {
        let why = || "why".to_string();
        let errors = [
            (ExportError::NoPackage, "SPPID_EXPORT_NO_PACKAGE"),
            (
                ExportError::NotABackup(PathBuf::from("loose")),
                "SPPID_EXPORT_NOT_A_BACKUP",
            ),
            (
                ExportError::TargetNotEmpty {
                    folder: PathBuf::from("out"),
                    names: texts(["kept"]),
                    count: 1,
                },
                "SPPID_EXPORT_TARGET_NOT_EMPTY",
            ),
            (
                ExportError::TargetInvalid(why()),
                "SPPID_EXPORT_TARGET_INVALID",
            ),
            (
                ExportError::ReferenceInvalid(why()),
                "SPPID_EXPORT_REFERENCE_INVALID",
            ),
            (ExportError::Busy, "SPPID_EXPORT_BUSY"),
            (ExportError::Unsupported(why()), "SPPID_EXPORT_UNSUPPORTED"),
            (ExportError::Failed(why()), "SPPID_EXPORT_FAILED"),
        ];
        let mut codes = Vec::new();
        for (error, code) in errors {
            assert_eq!(error.code(), code);
            let said = error.to_string();
            assert!(
                !said.is_empty() && !said.contains("SPPID"),
                "{code}: {said}"
            );
            codes.push(code);
        }
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 8, "{codes:?}");
        let said = ExportError::NoPackage.to_string();
        assert_eq!(said, "no backup package is open");
        let said = ExportError::Busy.to_string();
        let busy = "the last export of the package to publish files has not finished";
        assert_eq!(said, busy);
        for error in [
            ExportError::TargetInvalid(why()),
            ExportError::ReferenceInvalid(why()),
            ExportError::Unsupported(why()),
            ExportError::Failed(why()),
        ] {
            assert_eq!(error.to_string(), "why");
        }
    }

    #[test]
    fn the_summary_of_an_export_is_its_counts_and_where_its_index_is() {
        let plain = plain();
        let index = format!("index: {}", plain.index_path.display());
        let lines = [
            "wrote 53 drawing(s), 160 file(s), in 15.2 s",
            "failure count 0; 53 .pid file(s): 53 matched to a drawing, 0 border template(s), 0 \
             unmatched",
            "2 drawing(s) have no DrawingNo: their number is the database's",
            index.as_str(),
        ];
        assert_eq!(summary_lines(&plain), lines);

        // A package with no database export is said nothing more of, and
        // drawings that all state their number are not spoken of.
        let loose = ExportOutcome {
            export_kind: "PidOnly".to_string(),
            without_drawing_no: 0,
            ..plain.clone()
        };
        assert_eq!(summary_lines(&loose), [lines[0], lines[1], lines[3]]);

        // A SQL Server package, most of whose files are templates, compared
        // with a reference.
        let compared = ExportOutcome {
            written: 1,
            pid_entries: 12,
            templates: 11,
            without_drawing_no: 0,
            files: 4,
            export_kind: "SqlServerMtf".to_string(),
            gate: Some(GateSummary {
                clean: 7,
                dirty: 1,
                revision_mismatch: 4,
                unpaired: 41,
                lines: texts(["not in the summary"]),
            }),
            ..plain
        };
        let lines = [
            "wrote 1 drawing(s), 4 file(s), in 15.2 s",
            "failure count 0; 12 .pid file(s): 1 matched to a drawing, 11 border template(s), 0 \
             unmatched",
            index.as_str(),
            "compared with the reference: 53 drawing(s), 7 the same, 1 with findings, 4 of \
             another revision, 41 without a reference",
            UNVERIFIED,
        ];
        assert_eq!(summary_lines(&compared), lines);
    }

    #[test]
    fn the_summary_lists_what_is_not_written_and_what_is_wrong() {
        let unwritten = (1..=25).map(|at| note(&format!("D-{at:02}"), &["not a compound file"]));
        let troubled = ExportOutcome {
            written: 27,
            failed: 26,
            unmatched: texts(["a loose file"]),
            unwritten: unwritten.collect(),
            noted: vec![
                note("C-303", &[AGAIN, "a column was not read"]),
                note("C-304", &[AGAIN]),
            ],
            problems: texts(["PlantData~2~9.zip: cannot be opened", "C-303: in the index"]),
            other_problems: texts(["PlantData~2~9.zip: cannot be opened"]),
            warnings: texts(["C-303: in the index", "said nowhere else"]),
            files: 82,
            ..plain()
        };
        let mut expected = texts([
            "wrote 27 drawing(s), 82 file(s), in 15.2 s",
            "failure count 26; 53 .pid file(s): 52 matched to a drawing, 0 border template(s), 1 \
             unmatched",
            "2 drawing(s) have no DrawingNo: their number is the database's",
        ]);
        expected.push(format!("index: {}", troubled.index_path.display()));
        expected.push("25 drawing(s) could not be written:".to_string());
        expected.extend((1..=20).map(|at| format!("  D-{at:02}: not a compound file")));
        expected.extend(texts([
            "  ... and 5 more, see the index",
            "2 drawing(s) were written with notes:",
            "  C-303: 1 UID written more than once: 9F ×2; a column was not read",
            "  C-304: 1 UID written more than once: 9F ×2",
            "1 other problem(s) listed in the index:",
            "  PlantData~2~9.zip: cannot be opened",
            "1 warning(s) the index does not list:",
            "  said nowhere else",
        ]));
        assert_eq!(summary_lines(&troubled), expected);
    }

    #[test]
    fn a_list_of_the_summary_ends_at_twenty_of_its_kind() {
        // As many as are listed: all of them, and no count of more. A
        // drawing with two reasons, and one with none.
        let unwritten = (1..=18).map(|at| note(&format!("D-{at:02}"), &["too large"]));
        let mut unwritten: Vec<DrawingNote> = unwritten.collect();
        unwritten.push(note("E-19", &["too large", "left out"]));
        unwritten.push(note("E-20", &[]));
        let noted = (1..=21).map(|at| note(&format!("N-{at:02}"), &["a note"]));
        let listed = ExportOutcome {
            // Counts that do not add up are not made to.
            pid_entries: 1,
            templates: 1,
            unmatched: texts(["a loose file"]),
            without_drawing_no: 0,
            unwritten,
            noted: noted.collect(),
            other_problems: (1..=22).map(|at| format!("problem {at}")).collect(),
            warnings: (1..=23).map(|at| format!("warning {at}")).collect(),
            ..plain()
        };
        let mut expected = texts([
            "wrote 53 drawing(s), 160 file(s), in 15.2 s",
            "failure count 0; 1 .pid file(s): 0 matched to a drawing, 1 border template(s), 1 \
             unmatched",
        ]);
        expected.push(format!("index: {}", listed.index_path.display()));
        expected.push("20 drawing(s) could not be written:".to_string());
        expected.extend((1..=18).map(|at| format!("  D-{at:02}: too large")));
        expected.extend(texts([
            "  E-19: too large; left out",
            "  E-20: no reason given",
            "21 drawing(s) were written with notes:",
        ]));
        expected.extend((1..=20).map(|at| format!("  N-{at:02}: a note")));
        expected.extend(texts([
            "  ... and 1 more, see the index",
            "22 other problem(s) listed in the index:",
        ]));
        expected.extend((1..=20).map(|at| format!("  problem {at}")));
        expected.extend(texts([
            "  ... and 2 more, see the index",
            "23 warning(s) the index does not list:",
        ]));
        expected.extend((1..=20).map(|at| format!("  warning {at}")));
        expected.push("  ... and 3 more".to_string());
        assert_eq!(summary_lines(&listed), expected);
    }

    #[test]
    fn a_made_up_package_goes_through_the_publish_of_pid_parse() {
        let root = scratch("publish");
        let mut request = request(&root);
        let package = request.package_dir.clone();
        // A package with no database export and one drawing, which is none.
        let drawings = package.join("PlantData~2~1.zip");
        write_zip(&drawings, &[("Unit 1/A-101.pid", b"not a drawing")]);
        let before = tree(&package);

        let outcome = export(&request).expect("the export");
        let out = root.join("out");
        assert_eq!(outcome.export_kind, "PidOnly");
        assert_eq!((outcome.written, outcome.failed), (0, 1));
        assert_eq!((outcome.pid_entries, outcome.templates), (1, 0));
        assert_eq!(outcome.unmatched.len(), 1, "{:?}", outcome.unmatched);
        assert_eq!(outcome.without_drawing_no, 0);
        assert_eq!(outcome.unwritten.len(), 1, "{:?}", outcome.unwritten);
        let unwritten = &outcome.unwritten[0];
        assert_eq!(unwritten.name, "A-101");
        assert_eq!(unwritten.notes.len(), 1, "{unwritten:?}");
        let why = &unwritten.notes[0];
        assert!(outcome.noted.is_empty(), "{:?}", outcome.noted);
        // The index says why under the drawing's entry in the package, and
        // that is not a problem beside the drawing.
        let problem = format!("PlantData~2~1.zip Unit 1/A-101.pid: {why}");
        assert_eq!(outcome.problems, [problem]);
        assert_eq!(outcome.other_problems, [""; 0]);
        assert_eq!(outcome.warnings, [""; 0]);
        assert_eq!(outcome.gate, None);
        assert_eq!(outcome.files, 1);
        assert_eq!(outcome.index_path, out.join(INDEX_NAME));
        // The index is all that is written, it states the package as the
        // request spells it, and the package is as it was.
        let written = tree(&out);
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(written[0].0, INDEX_NAME);
        let index: Value = serde_json::from_slice(written[0].1.as_ref().unwrap()).unwrap();
        assert_eq!(index["package"], package.display().to_string());
        assert_eq!(tree(&package), before);
        let lines = summary_lines(&outcome);
        let listed = [
            "1 drawing(s) could not be written:".to_string(),
            format!("  A-101: {why}"),
        ];
        assert_eq!(lines[3..], listed, "{lines:?}");

        // The same compared with a reference that holds nothing.
        request.out_dir = root.join("compared");
        request.reference_dir = Some(root.join("compared with"));
        std::fs::create_dir(root.join("compared with")).unwrap();
        let outcome = export(&request).expect("the export with a reference");
        let gate = outcome.gate.expect("the comparison");
        let counts = (
            gate.clean,
            gate.dirty,
            gate.revision_mismatch,
            gate.unpaired,
        );
        assert_eq!(counts, (0, 0, 0, 0), "{gate:?}");

        // A database export of no kind `pid-parse` reads: nothing is
        // published, and no folder is left.
        std::fs::write(package.join("Export.dmp"), vec![b'x'; 600]).unwrap();
        request.out_dir = root.join("unread");
        let error = export(&request).expect_err("a dump that is none");
        assert_eq!(error.code(), "SPPID_EXPORT_UNSUPPORTED", "{error}");
        assert!(!root.join("unread").exists(), "the folder is left");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_export_in_the_background_comes_to_what_the_export_comes_to() {
        let root = scratch("background");
        let request = request(&root);
        let there = block_on(export_in_background(request.clone()));
        let there = there.expect("the export in the background");
        assert_eq!(there.index_path, root.join("out").join(INDEX_NAME));
        assert!(there.index_path.is_file(), "no index");
        let again = ExportRequest {
            out_dir: root.join("again"),
            ..request.clone()
        };
        let here = export(&again).expect("the export");
        let same = ExportOutcome {
            index_path: here.index_path.clone(),
            elapsed: here.elapsed,
            ..there
        };
        assert_eq!(same, here);
        assert_eq!(here.export_kind, "PidOnly");
        assert_eq!((here.written, here.failed, here.files), (0, 0, 1));

        // What is refused is refused there too.
        let refused = block_on(export_in_background(request));
        let code = refused.expect_err("into the folder just written").code();
        assert_eq!(code, "SPPID_EXPORT_TARGET_NOT_EMPTY");

        // The export runs on a thread of its own, under its own name.
        let named = || {
            let thread = std::thread::current();
            thread.name().unwrap_or_default().to_string()
        };
        let on = block_on(in_background(move || Err(ExportError::Failed(named()))));
        let own = "ocs-pid-backup-export";
        assert_eq!(on, Err(ExportError::Failed(own.to_string())));
        assert_ne!(named(), own);

        // A thread that is gone before it answers is a failed export.
        let gone = block_on(in_background(|| panic!("the export thread died")));
        let stopped = "the export stopped before it finished";
        assert_eq!(gone, Err(ExportError::Failed(stopped.to_string())));
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! PIDBACKUPEXPORT -- the open backup package as publish files
//! (`io::pid_backup_export`, SPEC-sppid-export §5.2).
//!
//! ```text
//! PIDBACKUPEXPORT [Plant=<name>] [Ref=<folder>] <folder>
//!                        publish every drawing of the package PIDBACKUPOPEN
//!                        holds into <folder>, with the plant name <name>
//!                        (P01 when none is given), and compare what is
//!                        written with the files of the Ref folder when one
//!                        is given; the folder is the rest of the line, so
//!                        it may hold spaces without quotes
//! PIDBACKUPEXPORT        say where that stands: no export yet, for how long
//!                        the one that runs has run, or what the last one
//!                        came to
//! ```
//!
//! `Plant=` and `Ref=` come before the folder, in either order, and a value
//! that holds spaces is quoted. A folder that is not given in full, the Ref
//! one too, is taken from the folder the backup lies in, as PIDBACKUPPDF
//! takes its folder; the line that says the export has started names them
//! in full. The folder the backup is restored to is not exported to: it is
//! the restore cache's to clear, not the user's.
//!
//! The export runs on a worker thread and the command returns once it has
//! started. What it came to is said when it ends, and again by the bare
//! verb. One export runs at a time; an export to PDF may run beside it, each
//! with a state of its own. While it runs the restore cache is not cleared
//! (`PIDBACKUPOPEN Cache=Clear`): the package it reads may lie there.
//!
//! An error is said with the code a script tells it by, in the line and as
//! the command line's error code (`SPPID_EXPORT_...`, SPEC-sppid-export
//! §3.3). A line the command cannot read is told how it is written, with no
//! code: no export was asked for. The lines are English, as PIDBACKUPPDF's
//! are.

use super::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::io::pid_backup_browse::BackupIndex;
#[cfg(not(target_arch = "wasm32"))]
use crate::io::pid_backup_export::{self, ExportError, ExportOutcome, ExportRequest};
#[cfg(not(target_arch = "wasm32"))]
use crate::ui::command_line::CommandLine;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// What an export comes to, once it has ended.
#[cfg(not(target_arch = "wasm32"))]
type Exported = iced::futures::future::BoxFuture<'static, Result<ExportOutcome, ExportError>>;

/// The export the command starts, given what to export.
#[cfg(not(target_arch = "wasm32"))]
type Export = Box<dyn Fn(ExportRequest) -> Exported + Send + Sync>;

/// The command's share of the application's state. Of the application, not
/// of a tab: the export is of the package, whatever drawing is in front.
/// Beside PIDBACKUPPDF's, not in it: the two exports may run at once.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::app) struct BackupExport {
    /// The export that runs; `None` while none does.
    running: Option<Running>,
    /// What the last export that ran came to; `None` before the first. One
    /// that was refused before it started is not among them.
    last: Option<Result<ExportOutcome, ExportError>>,
    /// `backup_root` of the package that produced `last`.
    last_backup_root: Option<PathBuf>,
    /// The export itself: [`pid_backup_export::export_in_background`], or
    /// what a test puts in its place.
    export: Export,
}

/// An export on its way.
#[cfg(not(target_arch = "wasm32"))]
struct Running {
    /// What it was asked, the folders in full.
    request: ExportRequest,
    /// When it started.
    started: Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for BackupExport {
    fn default() -> Self {
        Self {
            running: None,
            last: None,
            last_backup_root: None,
            export: Box::new(|request| Box::pin(pid_backup_export::export_in_background(request))),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl BackupExport {
    /// Whether a publish export is currently running on a worker thread.
    pub(in crate::app) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Active export request and how long it has run at `now`.
    pub(in crate::app) fn running_status(
        &self,
        now: Instant,
    ) -> Option<(&ExportRequest, std::time::Duration)> {
        let running = self.running.as_ref()?;
        Some((
            &running.request,
            now.saturating_duration_since(running.started),
        ))
    }

    /// Structured outcome of the most recent export that ran to completion.
    pub(in crate::app) fn last_outcome(&self) -> Option<&Result<ExportOutcome, ExportError>> {
        self.last.as_ref()
    }

    /// Structured outcome only when it belongs to `backup_root`.
    pub(in crate::app) fn last_outcome_for(
        &self,
        backup_root: &std::path::Path,
    ) -> Option<&Result<ExportOutcome, ExportError>> {
        (self.last_backup_root.as_deref() == Some(backup_root))
            .then_some(self.last.as_ref())
            .flatten()
    }
}

/// What the line asks for: the plant name and the reference folder when it
/// gives them, and the folder to export to, each as it is typed.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Asked<'a> {
    plant: Option<&'a str>,
    reference: Option<&'a str>,
    folder: &'a str,
}

/// Read `rest`, what follows the verb: the options before the folder, in
/// either order, and then the folder, the rest of the line without the
/// quotes around it. The error says what is wrong with the line.
#[cfg(not(target_arch = "wasm32"))]
fn asked(rest: &str) -> Result<Asked<'_>, String> {
    let mut asked = Asked::default();
    let mut rest = rest;
    while let Some((key, value, after)) = option(rest)? {
        let (given, takes) = if key.eq_ignore_ascii_case("Plant") {
            (&mut asked.plant, "a plant name")
        } else if key.eq_ignore_ascii_case("Ref") {
            (&mut asked.reference, "a folder")
        } else {
            return Err(format!(
                "{key}= is no option: the options are Plant= and Ref="
            ));
        };
        if given.is_some() {
            return Err(format!("{key}= is given twice"));
        }
        if value.is_empty() {
            return Err(format!("{key}= takes {takes}"));
        }
        *given = Some(value);
        rest = after.trim_start();
    }
    asked.folder = super::pidbackup::unquoted(rest);
    Ok(asked)
}

/// The option `rest` starts with: its key, its value without the quotes
/// around it, and what follows. `None` when `rest` starts with no
/// `<word>=`, so that a folder may hold `=`; the error is a quote that is
/// not closed.
#[cfg(not(target_arch = "wasm32"))]
fn option(rest: &str) -> Result<Option<(&str, &str, &str)>, String> {
    let Some((key, after)) = rest.split_once('=') else {
        return Ok(None);
    };
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphabetic()) {
        return Ok(None);
    }
    let known = key.eq_ignore_ascii_case("Plant") || key.eq_ignore_ascii_case("Ref");
    let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''));
    let Some(quote) = quote else {
        let (value, tail) = after.split_once(char::is_whitespace).unwrap_or((after, ""));
        if !known && tail.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some((key, value, tail)));
    };
    match after[quote.len_utf8()..].split_once(quote) {
        Some((value, tail)) => Ok(Some((key, value, tail))),
        None => Err(format!("the quote after {key}= is not closed")),
    }
}

/// What the open package and the line give an export: the package's own
/// folder, the folders asked for in full, and the plant name, P01 when the
/// line names none.
#[cfg(not(target_arch = "wasm32"))]
fn request_for(
    index: &BackupIndex,
    out_dir: PathBuf,
    plant: &str,
    reference_dir: Option<PathBuf>,
) -> ExportRequest {
    let in_full = |folder: PathBuf| super::pidbackuppdf::resolve_folder_for(index, folder);
    ExportRequest {
        package_dir: index.backup_root.clone(),
        out_dir: in_full(out_dir),
        plant: plant.to_string(),
        reference_dir: reference_dir.map(in_full),
    }
}

/// The line that says the export of `request` has started.
#[cfg(not(target_arch = "wasm32"))]
fn starting(request: &ExportRequest) -> String {
    let (package, out_dir) = (request.package_dir.display(), request.out_dir.display());
    let compared = match &request.reference_dir {
        Some(reference) => format!(", compared with {}", reference.display()),
        None => String::new(),
    };
    let plant = &request.plant;
    format!("PIDBACKUPEXPORT: exporting {package} to {out_dir} (plant {plant}{compared}) ...")
}

/// What the bare verb says before any export has run.
#[cfg(not(target_arch = "wasm32"))]
const NO_EXPORT_YET: &str = "PIDBACKUPEXPORT: no export yet. PIDBACKUPEXPORT [Plant=<name>] \
     [Ref=<folder>] <folder> writes the open backup package as publish files into <folder>.";

/// Say `error` with its code: in the line, and as the command line's own.
#[cfg(not(target_arch = "wasm32"))]
fn say_error(command_line: &mut CommandLine, error: &ExportError) {
    let code = error.code();
    command_line.push_error_coded(code, &format!("PIDBACKUPEXPORT: {code}: {error}"));
}

/// Say what an export came to: its summary, or why it did not get there.
#[cfg(not(target_arch = "wasm32"))]
fn say_result(command_line: &mut CommandLine, result: &Result<ExportOutcome, ExportError>) {
    match result {
        Ok(outcome) => {
            for line in pid_backup_export::summary_lines(outcome) {
                command_line.push_output(&format!("PIDBACKUPEXPORT: {line}"));
            }
        }
        Err(error) => say_error(command_line, error),
    }
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidbackupexport(
        &mut self,
        cmd: &str,
        _i: usize,
    ) -> Option<Task<Message>> {
        let verb = "PIDBACKUPEXPORT";
        let head = cmd.get(..verb.len())?;
        if !head.eq_ignore_ascii_case(verb) {
            return None;
        }
        let rest = &cmd[verb.len()..];
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return None;
        }
        Some(self.pid_backup_export_command(rest.trim()))
    }

    #[cfg(target_arch = "wasm32")]
    fn pid_backup_export_command(&mut self, _rest: &str) -> Task<Message> {
        self.command_line.push_error(
            "PIDBACKUPEXPORT: exporting a backup package to publish files is not available in \
             the web build.",
        );
        Task::none()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl OpenCADStudio {
    /// `rest` is what follows the verb: nothing asks where the export
    /// stands, anything else is an export to start.
    fn pid_backup_export_command(&mut self, rest: &str) -> Task<Message> {
        if rest.is_empty() {
            self.say_where_the_pid_backup_export_stands();
            return Task::none();
        }
        let started = asked(rest).map(|asked| self.start_pid_backup_export(&asked));
        match started {
            Ok(Ok(export)) => export,
            Ok(Err(error)) => {
                say_error(&mut self.command_line, &error);
                Task::none()
            }
            Err(unread) => {
                let said = format!("PIDBACKUPEXPORT: {unread}");
                self.command_line.push_error(&said);
                Task::none()
            }
        }
    }

    /// Start the export the line asks for and say so. One that cannot start
    /// -- another runs, no package is open, a folder will not do -- makes
    /// nothing and is no export to speak of later.
    fn start_pid_backup_export(&mut self, asked: &Asked<'_>) -> Result<Task<Message>, ExportError> {
        let plant = asked
            .plant
            .unwrap_or(pid_backup_export::DEFAULT_PLANT)
            .to_string();
        self.start_pid_backup_export_ui(
            PathBuf::from(asked.folder),
            plant,
            asked.reference.map(PathBuf::from),
        )
    }

    /// The bare verb: for how long the export that runs has run, what the
    /// last one came to, or that there has been none.
    fn say_where_the_pid_backup_export_stands(&mut self) {
        if let Some((request, elapsed)) = self.pid_backup_export.running_status(Instant::now()) {
            let seconds = elapsed.as_secs();
            self.command_line.push_output(&format!(
                "PIDBACKUPEXPORT: exporting to {} (plant {}): {seconds} s so far",
                request.out_dir.display(),
                request.plant
            ));
        } else if let Some(last) = self.pid_backup_export.last_outcome() {
            say_result(&mut self.command_line, last);
        } else {
            self.command_line.push_output(NO_EXPORT_YET);
        }
    }

    /// The export ended: say what it came to, and keep it for the bare verb.
    pub(in crate::app) fn on_pid_backup_export_finished(
        &mut self,
        result: Result<ExportOutcome, ExportError>,
    ) {
        let prev_root = self
            .pid_backup_export
            .running
            .take()
            .map(|r| r.request.package_dir)
            .or_else(|| {
                self.pid_backup_browser
                    .index()
                    .map(|idx| idx.backup_root.clone())
            });
        self.pid_backup_export.last_backup_root = prev_root;
        say_result(&mut self.command_line, &result);
        self.pid_backup_export.last = Some(result);
    }

    /// Whether the restore cache has to stay as it is: the package the
    /// export reads may lie in it. Says so when it has to, to the one who
    /// asked for the clearing.
    pub(super) fn pid_backup_export_keeps_the_cache(&mut self) -> bool {
        if !self.pid_backup_export.is_running() {
            return false;
        }
        let code = ExportError::Busy.code();
        self.command_line.push_error_coded(
            code,
            &format!(
                "PIDBACKUPOPEN: {code}: the restore cache is not cleared while PIDBACKUPEXPORT \
                 exports a package"
            ),
        );
        true
    }

    /// Validate a publish-export request from the UI without writing files.
    pub(in crate::app) fn check_pid_backup_export_ui(
        &self,
        out_dir: PathBuf,
        plant: &str,
        reference_dir: Option<PathBuf>,
    ) -> Result<ExportRequest, ExportError> {
        if self.pid_backup_export.is_running() {
            return Err(ExportError::Busy);
        }
        let index = self
            .pid_backup_browser
            .index()
            .ok_or(ExportError::NoPackage)?;
        if out_dir.as_os_str().is_empty() {
            let why = "no folder to export to was given".to_string();
            return Err(ExportError::TargetInvalid(why));
        }
        if plant.trim().is_empty() {
            let why = "no plant name was given".to_string();
            return Err(ExportError::TargetInvalid(why));
        }
        let request = request_for(index, out_dir, plant, reference_dir);
        if let Some(why) = pid_backup_export::in_the_restore(&request.out_dir, &index.restored) {
            return Err(ExportError::TargetInvalid(why));
        }
        pid_backup_export::check(&request)?;
        Ok(request)
    }

    /// Start a publish export from the UI, sharing state and guards with
    /// `PIDBACKUPEXPORT`.
    pub(in crate::app) fn start_pid_backup_export_ui(
        &mut self,
        out_dir: PathBuf,
        plant: String,
        reference_dir: Option<PathBuf>,
    ) -> Result<Task<Message>, ExportError> {
        let request = self.check_pid_backup_export_ui(out_dir, &plant, reference_dir)?;
        self.command_line.push_output(&starting(&request));
        self.pid_backup_export.running = Some(Running {
            request: request.clone(),
            started: Instant::now(),
        });
        let export = (self.pid_backup_export.export)(request);
        Ok(Task::perform(export, Message::PidBackupExportFinished))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::app::pid_shell::PidScreen;
    use crate::io::pid_backup_browse::DrawingEntry;
    use crate::io::pid_backup_export::{DrawingNote, GateSummary};
    use crate::io::pid_backup_pdf::{PdfError, PdfOutcome};
    use crate::ui::command_line::EntryKind;
    use crate::ui::window::pid_backup_browser::{self, CacheMsg, LoadState, NodeId, PidBackupMsg};
    use iced::futures::channel::oneshot;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    // ── A folder, a package, an application ──────────────────────────────────

    /// A folder of one test's own under the temp folder. It goes when the
    /// test is over, however the test ended.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let name = format!("ocs-pidbackupexport-{name}-{}", std::process::id());
            let root = std::env::temp_dir().join(name);
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a scratch folder");
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The names of what `folder` holds, sorted; none when it is not there.
    fn names(folder: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The index of a made-up package under `root`, as the panel holds one:
    /// the backup's own folder, with the manifest that makes it a package,
    /// and the folder it is restored to, with the files of two drawings.
    fn package(root: &Path) -> Arc<BackupIndex> {
        let backup_root = root.join("package");
        let restored = root.join("restored");
        let filed = restored.join("00").join("00");
        std::fs::create_dir_all(&backup_root).expect("the backup's folder");
        let manifest = "BackupType<<|>>2\nVersion<<|>>7.02\nName<<|>>SQPlant\n";
        std::fs::write(backup_root.join("Manifest.txt"), manifest).expect("the manifest");
        std::fs::create_dir_all(&filed).expect("the restore folder");
        let drawing = |name: &str| {
            let pid_path = filed.join(format!("{name}.pid"));
            std::fs::write(&pid_path, name).expect("a drawing's file");
            DrawingEntry {
                name: name.to_string(),
                pid_path: Some(pid_path),
                group_path: "SQPlant / 00".to_string(),
                meta: Vec::new(),
            }
        };
        let drawings = vec![drawing("A-101"), drawing("B-202")];
        Arc::new(BackupIndex {
            source: backup_root.clone(),
            restored,
            backup_root,
            drawings,
            ..BackupIndex::default()
        })
    }

    /// An application in the classic window, with a drawing open.
    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    /// Have `app` keep what it restores, and so what it clears, in the
    /// scratch folder: the default cache is not a test's to touch. The
    /// folder it will use.
    fn restores_into(app: &mut OpenCADStudio, scratch: &Scratch) -> PathBuf {
        let cache = scratch.join("restores");
        pid_backup_browser::set_cache_folder(&mut app.pid_backup_browser, Some(cache.clone()));
        cache
    }

    /// Give `app` the made-up [`package`] under `scratch` as the backup its
    /// panel holds.
    fn open_package(app: &mut OpenCADStudio, scratch: &Scratch) {
        restores_into(app, scratch);
        let _ = app.on_pid_backup(PidBackupMsg::Loaded(Ok(package(scratch.path()))));
    }

    /// A zip of a package as small as one gets: the manifest, and one
    /// drawing.
    fn tiny_backup(path: &Path, drawing: &str) {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("Manifest.txt", options).unwrap();
        writer.write_all(b"Name<<|>>SQPlant\n").unwrap();
        writer
            .start_file(format!("00/00/{drawing}.pid"), options)
            .unwrap();
        writer.write_all(drawing.as_bytes()).unwrap();
        writer.finish().unwrap();
    }

    /// A backup folder as small as one gets: the manifest, and three
    /// drawings in the archive a backup files its drawings in. A drawing's
    /// file holds its own path, which no publish takes for a drawing.
    fn backup_folder(scratch: &Scratch) -> PathBuf {
        use std::io::Write;
        let backup = scratch.join("backup");
        std::fs::create_dir_all(&backup).expect("the backup folder");
        let manifest = "BackupType<<|>>2\nVersion<<|>>7.02\nName<<|>>SQPlant\n";
        std::fs::write(backup.join("Manifest.txt"), manifest).expect("the manifest");
        let archive = backup.join("PlantData~2~711.zip");
        let archive = std::fs::File::create(archive).expect("the archive");
        let mut writer = zip::ZipWriter::new(archive);
        let options = zip::write::SimpleFileOptions::default();
        for file in ["00/00/A-101.pid", "00/00/B-202.pid", "01/01/C-303.pid"] {
            writer.start_file(file, options).expect("a drawing");
            writer.write_all(file.as_bytes()).expect("its bytes");
        }
        writer.finish().expect("the archive is written");
        backup
    }

    /// The application as a launch without `--classic` leaves it once the
    /// startup prompts are answered: in the P&ID module, on its editor.
    fn in_the_module() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        app.default_assoc_prompted = true;
        app.donation_prompt_version = env!("OCS_APP_VERSION").to_owned();
        let launch = crate::cli::GuiConfig::default();
        assert!(
            !launch.classic,
            "a launch is in the module unless it says otherwise"
        );
        let first = app.open_first_screen(&launch);
        app.drive_headless_task(first)
            .expect("the first screen opens without a window");
        let _ = app.update(Message::WindowResized(1600.0, 1000.0));
        app
    }

    // ── Exports that wait ────────────────────────────────────────────────────

    /// One call of the stand-in.
    struct Call {
        request: ExportRequest,
        /// Says what the call comes to; gone once it has been said.
        end: Option<oneshot::Sender<Result<ExportOutcome, ExportError>>>,
    }

    /// What stands in for the export: it keeps what the command asked of
    /// it, makes nothing, and each call waits until the test says what it
    /// came to.
    struct StandIn(Arc<Mutex<Vec<Call>>>);

    impl StandIn {
        /// Put a stand-in in the place of `app`'s export.
        fn put_into(app: &mut OpenCADStudio) -> Self {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let kept = Arc::clone(&calls);
            app.pid_backup_export.export = Box::new(move |request| {
                let (end, ended) = oneshot::channel();
                kept.lock().unwrap().push(Call {
                    request,
                    end: Some(end),
                });
                Box::pin(async move {
                    let let_go = || ExportError::Failed("the stand-in was let go of".to_string());
                    ended.await.unwrap_or_else(|_| Err(let_go()))
                })
            });
            Self(calls)
        }

        /// What the command asked of the export, call by call.
        fn requests(&self) -> Vec<ExportRequest> {
            let calls = self.0.lock().unwrap();
            calls.iter().map(|call| call.request.clone()).collect()
        }

        /// Have the latest call come to `result`.
        fn end(&self, result: Result<ExportOutcome, ExportError>) {
            let mut calls = self.0.lock().unwrap();
            let latest = calls.last_mut().expect("an export was started");
            let end = latest.end.take().expect("the export still waits");
            end.send(result).expect("the command waits for its export");
        }
    }

    /// What says what an export to PDF came to.
    type PdfEnd = oneshot::Sender<Result<PdfOutcome, PdfError>>;

    /// What stands in for the export to PDF: it makes nothing, and each
    /// call waits until the test has it fail.
    struct PdfStandIn(Arc<Mutex<Vec<Option<PdfEnd>>>>);

    impl PdfStandIn {
        /// Put a stand-in in the place of `app`'s export to PDF.
        fn put_into(app: &mut OpenCADStudio) -> Self {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let kept = Arc::clone(&calls);
            app.pid_backup_pdf.export = Box::new(move |_request, _progress| {
                let (end, ended) = oneshot::channel();
                kept.lock().unwrap().push(Some(end));
                Box::pin(async move {
                    let let_go = || PdfError::Failed("the stand-in was let go of".to_string());
                    ended.await.unwrap_or_else(|_| Err(let_go()))
                })
            });
            Self(calls)
        }

        /// How many exports to PDF were started.
        fn calls(&self) -> usize {
            self.0.lock().unwrap().len()
        }

        /// Have the latest call fail, `why` being the reason.
        fn fail(&self, why: &str) {
            let mut calls = self.0.lock().unwrap();
            let latest = calls.last_mut().expect("an export to PDF was started");
            let end = latest.take().expect("the export to PDF still waits");
            let failed = Err(PdfError::Failed(why.to_string()));
            end.send(failed).expect("the command waits for its export");
        }
    }

    // ── The command line ─────────────────────────────────────────────────────

    /// One thing the command line said.
    #[derive(Debug, Clone, PartialEq)]
    enum Said {
        /// A plain line.
        Line(String),
        /// An error, without the mark every error starts with.
        Error(String),
        /// Anything else but the echo of a command: a note, a warning.
        Other(String),
    }

    fn line(text: impl Into<String>) -> Said {
        Said::Line(text.into())
    }

    fn error(text: impl Into<String>) -> Said {
        Said::Error(text.into())
    }

    /// What the command line holds, the echo of the commands left out.
    fn said(app: &OpenCADStudio) -> Vec<Said> {
        let entries = app.command_line.history.iter();
        entries
            .filter_map(|entry| {
                let text = entry.text.clone();
                match entry.kind {
                    EntryKind::Command => None,
                    EntryKind::Output => Some(Said::Line(text)),
                    EntryKind::Error => {
                        let unmarked = text
                            .split_once(": ")
                            .map_or(text.as_str(), |(_, said)| said);
                        Some(Said::Error(unmarked.to_string()))
                    }
                    _ => Some(Said::Other(text)),
                }
            })
            .collect()
    }

    /// The code of the error the command line said last, as a script reads
    /// it.
    fn code(app: &OpenCADStudio) -> Option<&str> {
        app.command_line.last_error_code.as_deref()
    }

    /// Send `typed` as the command line and a script send it. What it starts
    /// is given back, not run: an export has not ended when its command has.
    fn start(app: &mut OpenCADStudio, typed: &str) -> Task<Message> {
        app.command_line.history.clear();
        app.command_line.last_error_code = None;
        app.run_command_line(typed)
    }

    /// Send `typed` and let what it starts run to its end: what the command
    /// line said.
    fn say(app: &mut OpenCADStudio, typed: &str) -> Vec<Said> {
        let task = start(app, typed);
        app.drive_headless_task(task)
            .expect("the command needs no window");
        said(app)
    }

    /// Send a line that is to start nothing -- the bare verb, an export
    /// that is to be refused -- and what the command line said. What it
    /// started after all is not run: an export in the stand-in's place
    /// would wait for an end the test is not going to give it.
    fn ask(app: &mut OpenCADStudio, typed: &str) -> Vec<Said> {
        let _ = start(app, typed);
        said(app)
    }

    /// Run `task`, which waits for an export the test has had end, to its
    /// end: what the command line said of it.
    fn finish(app: &mut OpenCADStudio, task: Task<Message>) -> Vec<Said> {
        app.command_line.history.clear();
        app.command_line.last_error_code = None;
        app.drive_headless_task(task)
            .expect("an export ends without a window");
        said(app)
    }

    /// Let the export `task` waits for come to `result`, and run the task to
    /// its end: what the command line said of it.
    fn end(
        app: &mut OpenCADStudio,
        stand_in: &StandIn,
        task: Task<Message>,
        result: Result<ExportOutcome, ExportError>,
    ) -> Vec<Said> {
        stand_in.end(result);
        finish(app, task)
    }

    /// Set the start of the export that runs `ago` seconds back.
    fn started_ago(app: &mut OpenCADStudio, ago: u64) {
        let running = app.pid_backup_export.running.as_mut();
        let running = running.expect("an export runs");
        running.started = Instant::now() - Duration::from_secs(ago);
    }

    // ── What is typed, what an export comes to, what is said of it ──────────

    const NONE_YET: &str = "PIDBACKUPEXPORT: no export yet. PIDBACKUPEXPORT [Plant=<name>] [Ref=<folder>] <folder> writes the open backup package as publish files into <folder>.";
    const NO_PACKAGE: &str = "PIDBACKUPEXPORT: SPPID_EXPORT_NO_PACKAGE: no backup package is open";
    const BUSY: &str = "PIDBACKUPEXPORT: SPPID_EXPORT_BUSY: the last export of the package to publish files has not finished";
    const CACHE_KEPT: &str = "PIDBACKUPOPEN: SPPID_EXPORT_BUSY: the restore cache is not cleared while PIDBACKUPEXPORT exports a package";

    /// The line that asks for an export into `out`, as it is typed.
    fn export_line(out: &Path) -> String {
        format!("PIDBACKUPEXPORT {}", out.display())
    }

    /// The line that opens the backup `source`, as it is typed.
    fn open_line(source: &Path) -> String {
        format!("PIDBACKUPOPEN \"{}\"", source.display())
    }

    /// What an export of `package` into `out` starts with.
    fn exporting(package: &Path, out: &Path, plant: &str, reference: Option<&Path>) -> Said {
        let compared = reference.map_or(String::new(), |reference| {
            format!(", compared with {}", reference.display())
        });
        let (package, out) = (package.display(), out.display());
        line(format!(
            "PIDBACKUPEXPORT: exporting {package} to {out} (plant {plant}{compared}) ..."
        ))
    }

    /// What the bare verb says while an export into `out` has run for
    /// `seconds`.
    fn running(out: &Path, plant: &str, seconds: u64) -> Said {
        line(format!(
            "PIDBACKUPEXPORT: exporting to {} (plant {plant}): {seconds} s so far",
            out.display()
        ))
    }

    /// What an export into `out` comes to: two drawings written, one of them
    /// without a DrawingNo, and a third `.pid` no drawing of the database is
    /// matched to.
    fn outcome(out: &Path) -> ExportOutcome {
        ExportOutcome {
            written: 2,
            failed: 0,
            pid_entries: 3,
            templates: 0,
            unmatched: vec!["PlantData~2~711.zip 01/01/C-303.pid".to_string()],
            without_drawing_no: 1,
            unwritten: Vec::new(),
            noted: Vec::new(),
            problems: Vec::new(),
            other_problems: Vec::new(),
            warnings: Vec::new(),
            gate: None,
            export_kind: "OracleExp".to_string(),
            files: 7,
            index_path: out.join("_Index.json"),
            elapsed: Duration::from_millis(15_200),
        }
    }

    /// What the command line says of [`outcome`].
    fn summary(out: &Path) -> [Said; 4] {
        let index = out.join("_Index.json");
        [
            line("PIDBACKUPEXPORT: wrote 2 drawing(s), 7 file(s), in 15.2 s"),
            line(
                "PIDBACKUPEXPORT: failure count 0; 3 .pid file(s): 2 matched to a drawing, \
                 0 border template(s), 1 unmatched",
            ),
            line("PIDBACKUPEXPORT: 1 drawing(s) have no DrawingNo: their number is the database's"),
            line(format!("PIDBACKUPEXPORT: index: {}", index.display())),
        ]
    }

    /// What the export of a SQL Server package into `out` comes to,
    /// compared with a reference: one drawing written, which differs from
    /// its reference, and one that could not be.
    fn compared(out: &Path) -> ExportOutcome {
        let gate = GateSummary {
            clean: 0,
            dirty: 1,
            revision_mismatch: 0,
            unpaired: 0,
            lines: Vec::new(),
        };
        let unwritten = DrawingNote {
            name: "B-202".to_string(),
            notes: vec!["its database record is gone".to_string()],
        };
        ExportOutcome {
            written: 1,
            failed: 1,
            unwritten: vec![unwritten],
            gate: Some(gate),
            export_kind: "SqlServerMtf".to_string(),
            files: 4,
            ..outcome(out)
        }
    }

    /// What the command line says of [`compared`].
    fn compared_summary(out: &Path) -> [Said; 8] {
        let index = out.join("_Index.json");
        [
            line("PIDBACKUPEXPORT: wrote 1 drawing(s), 4 file(s), in 15.2 s"),
            line(
                "PIDBACKUPEXPORT: failure count 1; 3 .pid file(s): 2 matched to a drawing, \
                 0 border template(s), 1 unmatched",
            ),
            line("PIDBACKUPEXPORT: 1 drawing(s) have no DrawingNo: their number is the database's"),
            line(format!("PIDBACKUPEXPORT: index: {}", index.display())),
            line(
                "PIDBACKUPEXPORT: compared with the reference: 1 drawing(s), 0 the same, \
                 1 with findings, 0 of another revision, 0 without a reference",
            ),
            line(
                "PIDBACKUPEXPORT: a SQL Server package: what its export writes has not been \
                 verified",
            ),
            line("PIDBACKUPEXPORT: 1 drawing(s) could not be written:"),
            line("PIDBACKUPEXPORT:   B-202: its database record is gone"),
        ]
    }

    /// What an export must leave as it was: how many tabs there are, which
    /// is in front, and of that one whether it is modified, how much can be
    /// undone and redone in it, and how much it holds.
    fn front(app: &OpenCADStudio) -> (usize, usize, bool, usize, usize, usize) {
        let tab = &app.tabs[app.active_tab];
        (
            app.tabs.len(),
            app.active_tab,
            tab.dirty,
            tab.history.undo_stack.len(),
            tab.history.redo_stack.len(),
            tab.scene.document.entities().count(),
        )
    }

    // ── The tests ────────────────────────────────────────────────────────────

    #[test]
    fn without_an_open_package_or_while_one_loads_the_export_is_refused() {
        let scratch = Scratch::new("no-package");
        let out = scratch.join("out");
        let mut app = app();
        // The export is the application's own here: what is refused never
        // gets as far as an export, whichever it is.
        assert_eq!(say(&mut app, &export_line(&out)), [error(NO_PACKAGE)]);
        assert_eq!(code(&app), Some("SPPID_EXPORT_NO_PACKAGE"));
        let optioned = format!("PIDBACKUPEXPORT Plant=P02 Ref=refs {}", out.display());
        assert_eq!(say(&mut app, &optioned), [error(NO_PACKAGE)]);
        // What was refused is no export: there has been none yet.
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [line(NONE_YET)]);
        // A script on the automation channel is told the code.
        let run = serde_json::json!({ "op": "run", "cmd": export_line(&out) });
        let reply = app.automation_op(&run.to_string());
        assert_eq!(reply["code"], "SPPID_EXPORT_NO_PACKAGE", "{reply}");

        // A package was open and another is on its way: the panel holds
        // neither while it loads.
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let next = scratch.join("next.zip");
        let _ = app.dispatch_command(&format!("PIDBACKUPOPEN {}", next.display()));
        let loading = &app.pid_backup_browser.load;
        assert!(matches!(loading, LoadState::Loading(_)));
        assert_eq!(ask(&mut app, &export_line(&out)), [error(NO_PACKAGE)]);
        assert_eq!(code(&app), Some("SPPID_EXPORT_NO_PACKAGE"));
        assert!(stand_in.requests().is_empty());
        assert!(!out.exists(), "nothing is made");
    }

    #[test]
    fn the_options_and_the_folder_are_read_from_the_line_and_the_export_is_asked_for_once() {
        let scratch = Scratch::new("line");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let package = scratch.join("package");
        let out = scratch.join("publish out");
        let reference = scratch.join("the ref");
        std::fs::create_dir_all(&reference).expect("the reference folder");
        let (o, r) = (out.display(), reference.display());
        // As it is typed, and the plant name and reference folder the export
        // is asked for. The folder: with its space and no quotes, in double
        // quotes, in single ones, by a path from the folder the backup lies
        // in. The options: either, both in either order, quoted or not, the
        // reference folder by a path from the backup's folder too.
        let typed = [
            (format!("PIDBACKUPEXPORT {o}"), "P01", None),
            (format!("PIDBACKUPEXPORT \"{o}\""), "P01", None),
            (format!("PIDBACKUPEXPORT   '{o}'  "), "P01", None),
            ("PIDBACKUPEXPORT publish out".to_string(), "P01", None),
            ("pidbackupexport publish out".to_string(), "P01", None),
            (format!("PIDBACKUPEXPORT Plant=P02 {o}"), "P02", None),
            (
                format!("PIDBACKUPEXPORT Ref=\"{r}\" Plant='Plant 7' {o}"),
                "Plant 7",
                Some(reference.as_path()),
            ),
            (
                format!("PIDBACKUPEXPORT plant=P03  REF=\"the ref\" \"{o}\""),
                "P03",
                Some(reference.as_path()),
            ),
        ];
        for (round, (typed, plant, reference)) in typed.iter().enumerate() {
            let export = start(&mut app, typed);
            // One line, the export's own: the verb was not run by itself
            // first, which would have said where the last export stands.
            let started = exporting(&package, &out, plant, *reference);
            assert_eq!(said(&app), [started], "{typed}");
            let requests = stand_in.requests();
            assert_eq!(requests.len(), round + 1, "{typed}");
            let asked = ExportRequest {
                package_dir: package.clone(),
                out_dir: out.clone(),
                plant: plant.to_string(),
                reference_dir: reference.map(Path::to_path_buf),
            };
            assert_eq!(requests[round], asked, "{typed}");
            end(&mut app, &stand_in, export, Ok(outcome(&out)));
        }
        // The stand-in makes nothing, and neither does the command.
        assert_eq!(names(scratch.path()), ["package", "restored", "the ref"]);

        // A bare relative folder whose name holds `=` (and is not followed by
        // another folder) is taken as the output folder rather than rejected
        // as an unknown option.
        let eq_out = scratch.join("out=1");
        let export = start(&mut app, "pidbackupexport Plant=P04 out=1");
        let started = exporting(&package, &eq_out, "P04", None);
        assert_eq!(said(&app), [started]);
        assert_eq!(stand_in.requests().last().unwrap().out_dir, eq_out);
        end(&mut app, &stand_in, export, Ok(outcome(&eq_out)));
    }

    #[test]
    fn a_line_the_command_cannot_read_says_how_it_is_written_and_starts_nothing() {
        let scratch = Scratch::new("unread");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let o = scratch.join("out").display().to_string();
        let refused = [
            (
                format!("PIDBACKUPEXPORT Plnt=P02 {o}"),
                "Plnt= is no option: the options are Plant= and Ref=",
            ),
            (
                format!("PIDBACKUPEXPORT Plant= {o}"),
                "Plant= takes a plant name",
            ),
            (format!("PIDBACKUPEXPORT Ref='' {o}"), "Ref= takes a folder"),
            (
                format!("PIDBACKUPEXPORT Plant=P02 plant=P03 {o}"),
                "plant= is given twice",
            ),
            (
                format!("PIDBACKUPEXPORT Ref=\"the ref {o}"),
                "the quote after Ref= is not closed",
            ),
        ];
        for (typed, why) in refused {
            let answer = ask(&mut app, &typed);
            assert_eq!(
                answer,
                [error(format!("PIDBACKUPEXPORT: {why}"))],
                "{typed}"
            );
            // No export was asked for, so there is no code of one.
            assert_eq!(code(&app), None, "{typed}");
        }
        // Options and no folder: the folder is what is missing.
        let answer = ask(&mut app, "PIDBACKUPEXPORT Plant=P02");
        let missing =
            "PIDBACKUPEXPORT: SPPID_EXPORT_TARGET_INVALID: no folder to export to was given";
        assert_eq!(answer, [error(missing)]);
        assert_eq!(code(&app), Some("SPPID_EXPORT_TARGET_INVALID"));
        assert!(stand_in.requests().is_empty());
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [line(NONE_YET)]);
        assert_eq!(names(scratch.path()), ["package", "restored"]);
    }

    #[test]
    fn a_folder_in_the_restore_is_not_exported_to() {
        let scratch = Scratch::new("restore");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let restored = scratch.join("restored");
        let held = names(&restored);
        // In it, the restore itself, and by a path that only ends up there.
        let package = scratch.join("package");
        let inside = [
            restored.join("publish"),
            restored.clone(),
            package.join("..").join("restored").join("00").join("out"),
        ];
        for out in inside {
            let answer = ask(&mut app, &export_line(&out));
            let asked = std::path::absolute(&out).expect("the folder in full");
            let why = format!(
                "PIDBACKUPEXPORT: SPPID_EXPORT_TARGET_INVALID: {} lies in the folder the backup \
                 is restored to ({}); export to a folder outside of it",
                asked.display(),
                restored.display()
            );
            assert_eq!(answer, [error(why)], "{}", out.display());
            assert_eq!(code(&app), Some("SPPID_EXPORT_TARGET_INVALID"));
        }
        assert!(stand_in.requests().is_empty());
        assert_eq!(names(&restored), held, "nothing is made");
        // Beside it is the user's.
        let beside = scratch.join("restored publish");
        let export = start(&mut app, &export_line(&beside));
        assert_eq!(said(&app), [exporting(&package, &beside, "P01", None)]);
        end(&mut app, &stand_in, export, Ok(outcome(&beside)));
    }

    #[test]
    fn while_the_export_runs_the_bare_verb_says_for_how_long_and_no_other_starts() {
        let scratch = Scratch::new("running");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let package = scratch.join("package");
        let (out, other) = (scratch.join("out"), scratch.join("other"));
        let export = start(
            &mut app,
            &format!("PIDBACKUPEXPORT Plant=P02 {}", out.display()),
        );
        // The command is back and has said so; the export has not ended.
        assert_eq!(said(&app), [exporting(&package, &out, "P02", None)]);
        assert_eq!(stand_in.requests().len(), 1);
        // The bare verb says where to and for how long, in whole seconds.
        started_ago(&mut app, 0);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [running(&out, "P02", 0)]);
        started_ago(&mut app, 75);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [running(&out, "P02", 75)]);
        // Another export is refused, whatever it asks for.
        let others = [
            export_line(&other),
            format!("PIDBACKUPEXPORT Plant=P09 {}", other.display()),
            export_line(&out),
        ];
        for typed in &others {
            assert_eq!(ask(&mut app, typed), [error(BUSY)], "{typed}");
            assert_eq!(code(&app), Some("SPPID_EXPORT_BUSY"), "{typed}");
        }
        assert_eq!(stand_in.requests().len(), 1);
        // It ends: what it came to is said, and the next one starts.
        let answer = end(&mut app, &stand_in, export, Ok(outcome(&out)));
        assert_eq!(answer, summary(&out));
        let next = start(&mut app, &export_line(&other));
        assert_eq!(said(&app), [exporting(&package, &other, "P01", None)]);
        assert_eq!(stand_in.requests().len(), 2);
        end(&mut app, &stand_in, next, Ok(outcome(&other)));
        assert!(!out.exists() && !other.exists(), "nothing is made");
    }

    #[test]
    fn while_the_export_runs_the_restore_cache_is_not_cleared() {
        let scratch = Scratch::new("cache");
        let (first, second) = (scratch.join("first.zip"), scratch.join("second.zip"));
        tiny_backup(&first, "A-101");
        tiny_backup(&second, "B-202");
        let mut app = app();
        let cache = restores_into(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let restored = |app: &OpenCADStudio| {
            let index = app.pid_backup_browser.index().expect("the backup is open");
            index.restored.clone()
        };

        // The export of one backup runs, which is read from its restore,
        // and another backup is opened: from here on a clearing would
        // remove what the export reads.
        say(&mut app, &open_line(&first));
        let read = restored(&app);
        let drawing = read.join("00").join("00").join("A-101.pid");
        assert_eq!(read.parent(), Some(cache.as_path()));
        assert!(drawing.is_file());
        let out = scratch.join("out");
        let export = start(&mut app, &export_line(&out));
        let package = stand_in.requests()[0].package_dir.clone();
        assert!(package.starts_with(&read), "{}", package.display());
        say(&mut app, &open_line(&second));
        let kept = restored(&app);
        assert_ne!(kept, read);
        // The backup's own row has the button that clears the others.
        let task = app.on_pid_backup(PidBackupMsg::Select(NodeId::Root));
        app.drive_headless_task(task)
            .expect("measuring needs no window");
        let button = crate::t!("Clear the other restores").into_owned();

        // Asked for on the command line, pressed by its label, clicked.
        let typed = [
            "PIDBACKUPOPEN Cache=Clear".to_string(),
            format!("PIDBACKUPOPEN Press {button}"),
        ];
        for typed in &typed {
            let answer = say(&mut app, typed);
            assert_eq!(answer.last(), Some(&error(CACHE_KEPT)), "{typed}");
            assert_eq!(code(&app), Some("SPPID_EXPORT_BUSY"), "{typed}");
            assert!(drawing.is_file(), "{typed}");
        }
        app.command_line.history.clear();
        let task = app.on_pid_backup(PidBackupMsg::Cache(CacheMsg::Clear));
        app.drive_headless_task(task)
            .expect("a refusal needs no window");
        assert_eq!(said(&app), [error(CACHE_KEPT)]);
        assert!(drawing.is_file());

        // The export has ended: a clearing clears again.
        end(&mut app, &stand_in, export, Ok(outcome(&out)));
        let answer = say(&mut app, "PIDBACKUPOPEN Cache=Clear");
        assert!(!read.exists(), "{answer:?}");
        assert!(kept.join("00").join("00").join("B-202.pid").is_file());
        let refused = |said: &Said| matches!(said, Said::Error(_));
        assert!(!answer.iter().any(refused), "{answer:?}");
    }

    #[test]
    fn an_export_that_fails_says_its_code_and_why_and_the_next_one_starts() {
        let scratch = Scratch::new("failed");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let package = scratch.join("package");
        let out = scratch.join("out");
        let unread = "the package could not be read: PlantData~2~711.zip is no archive";
        let unsupported = "the database export Export.dmp is of no kind pid-parse reads";
        let failures = [
            (
                ExportError::Failed(unread.to_string()),
                "SPPID_EXPORT_FAILED",
                unread,
            ),
            (
                ExportError::Unsupported(unsupported.to_string()),
                "SPPID_EXPORT_UNSUPPORTED",
                unsupported,
            ),
        ];
        for (failure, its_code, why) in failures {
            let export = start(&mut app, &export_line(&out));
            let answer = end(&mut app, &stand_in, export, Err(failure));
            let failed = [error(format!("PIDBACKUPEXPORT: {its_code}: {why}"))];
            assert_eq!(answer, failed);
            assert_eq!(code(&app), Some(its_code));
            // That is what the last export came to, and the bare verb says so.
            assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), failed);
            assert_eq!(code(&app), Some(its_code));
        }

        // The application goes on: a line is drawn, and the next export
        // starts, into the same folder, and ends well.
        let held = front(&app).5;
        say(&mut app, "LINE 0,0 10,0");
        assert_eq!(front(&app).5, held + 1);
        let export = start(&mut app, &export_line(&out));
        assert_eq!(said(&app), [exporting(&package, &out, "P01", None)]);
        assert_eq!(stand_in.requests().len(), 3);
        let answer = end(&mut app, &stand_in, export, Ok(outcome(&out)));
        assert_eq!(answer, summary(&out));
        assert_eq!(code(&app), None);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), summary(&out));
    }

    #[test]
    fn when_the_export_ends_its_summary_is_said_and_the_bare_verb_says_it_again() {
        let scratch = Scratch::new("summary");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [line(NONE_YET)]);

        // No reference, an Oracle package: the summary of the engine, line
        // by line after the command's name, and no more.
        let out = scratch.join("out");
        let export = start(&mut app, &export_line(&out));
        let answer = end(&mut app, &stand_in, export, Ok(outcome(&out)));
        let engine = pid_backup_export::summary_lines(&outcome(&out));
        let named: Vec<Said> = engine
            .iter()
            .map(|said| line(format!("PIDBACKUPEXPORT: {said}")))
            .collect();
        assert_eq!(answer, named);
        assert_eq!(answer, summary(&out));
        assert_eq!(code(&app), None);
        // Asked again, it is said again, as often as it is asked.
        for _ in 0..2 {
            assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), summary(&out));
        }

        // Compared with a reference, of a SQL Server package: a line more
        // for each, and the drawing that could not be written.
        let reference = scratch.join("ref");
        std::fs::create_dir_all(&reference).expect("the reference folder");
        let other = scratch.join("compared");
        let (r, o) = (reference.display(), other.display());
        let export = start(&mut app, &format!("PIDBACKUPEXPORT Ref=\"{r}\" {o}"));
        assert_eq!(
            stand_in.requests()[1].reference_dir,
            Some(reference.clone())
        );
        let answer = end(&mut app, &stand_in, export, Ok(compared(&other)));
        assert_eq!(answer, compared_summary(&other));
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), compared_summary(&other));

        // An export that is refused is none to speak of: the bare verb says
        // what the last one that ran came to.
        let full = scratch.join("package");
        let answer = ask(&mut app, &export_line(&full));
        assert_eq!(
            code(&app),
            Some("SPPID_EXPORT_TARGET_INVALID"),
            "{answer:?}"
        );
        assert_eq!(stand_in.requests().len(), 2);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), compared_summary(&other));
    }

    #[test]
    fn the_drawing_in_front_is_left_as_it_was() {
        let scratch = Scratch::new("front");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        // A drawing nothing was done to, then one with a line that can be
        // undone: neither is the export's to change.
        for (round, drawn) in [false, true].into_iter().enumerate() {
            if drawn {
                say(&mut app, "LINE 0,0 10,0");
            }
            let before = front(&app);
            let out = scratch.join(&format!("out {round}"));
            let export = start(&mut app, &export_line(&out));
            assert_eq!(front(&app), before, "started, round {round}");
            ask(&mut app, "PIDBACKUPEXPORT");
            assert_eq!(front(&app), before, "asked while it runs, round {round}");
            end(&mut app, &stand_in, export, Ok(outcome(&out)));
            assert_eq!(front(&app), before, "ended, round {round}");
            ask(&mut app, "PIDBACKUPEXPORT");
            assert_eq!(front(&app), before, "asked after it, round {round}");
        }
        // The line was a change, so there was something to lose.
        let (_, _, modified, undone, _, held) = front(&app);
        assert!(modified && undone > 0 && held > 0, "{:?}", front(&app));
        assert_eq!(stand_in.requests().len(), 2);
    }

    #[test]
    fn an_export_to_pdf_and_one_to_publish_files_run_at_once_either_way_round() {
        let scratch = Scratch::new("beside");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let pdf = PdfStandIn::put_into(&mut app);
        let package = scratch.join("package");
        for round in 0..2 {
            let publish = scratch.join(&format!("publish {round}"));
            let pdfs = scratch.join(&format!("pdf {round}"));
            let pdf_line = format!("PIDBACKUPPDF {}", pdfs.display());
            let pdf_started = line(format!(
                "PIDBACKUPPDF: exporting 2 drawing(s) to {} ...",
                pdfs.display()
            ));
            let pdf_running = line(format!(
                "PIDBACKUPPDF: exporting to {}: 0 of 2 drawing(s) done",
                pdfs.display()
            ));
            let pdf_failed = error(format!("PIDBACKUPPDF: SPPID_PDF_FAILED: round {round}"));
            let publish_started = exporting(&package, &publish, "P01", None);

            // The first round starts the PDFs first, the second the publish
            // files; each starts while the other runs.
            let (to_pdf, to_publish) = if round == 0 {
                let to_pdf = start(&mut app, &pdf_line);
                assert_eq!(said(&app), std::slice::from_ref(&pdf_started));
                let to_publish = start(&mut app, &export_line(&publish));
                assert_eq!(said(&app), std::slice::from_ref(&publish_started));
                (to_pdf, to_publish)
            } else {
                let to_publish = start(&mut app, &export_line(&publish));
                assert_eq!(said(&app), std::slice::from_ref(&publish_started));
                let to_pdf = start(&mut app, &pdf_line);
                assert_eq!(said(&app), std::slice::from_ref(&pdf_started));
                (to_pdf, to_publish)
            };
            assert_eq!(pdf.calls(), round + 1);
            assert_eq!(stand_in.requests().len(), round + 1);
            // Each says its own state.
            assert_eq!(
                ask(&mut app, "PIDBACKUPPDF"),
                std::slice::from_ref(&pdf_running)
            );
            started_ago(&mut app, 3);
            assert_eq!(
                ask(&mut app, "PIDBACKUPEXPORT"),
                [running(&publish, "P01", 3)]
            );

            // The first round ends the publish files first, the second the
            // PDFs; the other runs on.
            if round == 0 {
                let answer = end(&mut app, &stand_in, to_publish, Ok(outcome(&publish)));
                assert_eq!(answer, summary(&publish));
                assert_eq!(
                    ask(&mut app, "PIDBACKUPPDF"),
                    std::slice::from_ref(&pdf_running)
                );
                pdf.fail(&format!("round {round}"));
                assert_eq!(finish(&mut app, to_pdf), std::slice::from_ref(&pdf_failed));
            } else {
                pdf.fail(&format!("round {round}"));
                assert_eq!(finish(&mut app, to_pdf), std::slice::from_ref(&pdf_failed));
                started_ago(&mut app, 4);
                let still = [running(&publish, "P01", 4)];
                assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), still);
                let answer = end(&mut app, &stand_in, to_publish, Ok(outcome(&publish)));
                assert_eq!(answer, summary(&publish));
            }
            // And each says again what it came to.
            assert_eq!(ask(&mut app, "PIDBACKUPPDF"), [pdf_failed]);
            assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), summary(&publish));
        }
    }

    #[test]
    fn the_command_is_listed_for_autocomplete_and_runs_with_no_drawing_open() {
        let listed = crate::command::all_registered_command_names();
        assert!(listed.contains(&"PIDBACKUPEXPORT"));
        // The start page: the package is the application's, not a drawing's.
        let scratch = Scratch::new("start");
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start);
        assert_eq!(ask(&mut app, "PIDBACKUPEXPORT"), [line(NONE_YET)]);
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);
        let (package, out) = (scratch.join("package"), scratch.join("out"));
        let export = start(&mut app, &export_line(&out));
        assert_eq!(said(&app), [exporting(&package, &out, "P01", None)]);
        let answer = end(&mut app, &stand_in, export, Ok(outcome(&out)));
        assert_eq!(answer, summary(&out));
        assert!(app.tabs[app.active_tab].is_start);
    }

    #[test]
    fn a_longer_word_is_not_this_command() {
        let mut app = app();
        // It is answered as any word that is no command is, the word aside:
        // not as an export into a folder called `S`.
        let mut answered = |word: &str| match say(&mut app, word).as_slice() {
            [Said::Error(text)] => text.replace(word, "<word>"),
            other => panic!("{word}: {other:?}"),
        };
        let no_command = answered("PIDBACKUPNOSUCH");
        assert_eq!(answered("PIDBACKUPEXPORTS"), no_command);
    }

    #[test]
    fn in_the_module_the_export_itself_publishes_the_backup_the_panel_holds() {
        let scratch = Scratch::new("module");
        let mut app = in_the_module();
        assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
        restores_into(&mut app, &scratch);
        let backup = backup_folder(&scratch);
        say(&mut app, &open_line(&backup));
        let index = app.pid_backup_browser.index().expect("the backup is open");
        assert_eq!(index.drawings.len(), 3);
        let package = index.backup_root.clone();
        let tabs = app.tabs.len();

        // No stand-in: the export is the one the application has.
        let out = scratch.join("publish out");
        let answer = say(
            &mut app,
            &format!("PIDBACKUPEXPORT Plant=P02 {}", out.display()),
        );

        assert_eq!(answer.len(), 8, "{answer:?}");
        let text = |at: usize| match &answer[at] {
            Said::Line(text) => text.as_str(),
            other => panic!("{other:?} is no plain line"),
        };
        assert_eq!(answer[0], exporting(&package, &out, "P02", None));
        // Not one of the made-up files is a drawing, and each is named.
        let wrote = "PIDBACKUPEXPORT: wrote 0 drawing(s), 1 file(s), in ";
        assert!(text(1).starts_with(wrote), "{answer:?}");
        let counted = "PIDBACKUPEXPORT: failure count 3; 3 .pid file(s): 0 matched to a \
                       drawing, 0 border template(s), 3 unmatched";
        assert_eq!(text(2), counted);
        let index = out.join("_Index.json");
        assert_eq!(
            text(3),
            format!("PIDBACKUPEXPORT: index: {}", index.display())
        );
        assert_eq!(
            text(4),
            "PIDBACKUPEXPORT: 3 drawing(s) could not be written:"
        );
        let mut unwritten: Vec<&str> = (5..8)
            .map(|at| text(at).trim_start_matches("PIDBACKUPEXPORT:   "))
            .map(|named| named.split_once(": ").map_or(named, |(name, _)| name))
            .collect();
        unwritten.sort_unstable();
        assert_eq!(unwritten, ["A-101", "B-202", "C-303"], "{answer:?}");
        // The index is all the folder holds, and the backup is as it was.
        assert_eq!(names(&out), ["_Index.json"]);
        assert_eq!(names(&backup), ["Manifest.txt", "PlantData~2~711.zip"]);
        // The module is where it was.
        assert_eq!(app.pid_shell.current(), Some(PidScreen::Editor));
        assert_eq!(app.tabs.len(), tabs);
        // And the bare verb says again what the export came to.
        assert_eq!(say(&mut app, "PIDBACKUPEXPORT"), answer[1..]);
    }

    // ── A real backup ────────────────────────────────────────────────────────

    /// The backup the real export takes when `OCS_PID_BACKUP_EXPORT_REAL`
    /// names none: the plant sample, where it lies on the machine the plan
    /// was written on.
    const REAL_BACKUP: &str = r"D:\work\plant-code\cad\pid-test-data";

    /// The backup the broken copy is made of when
    /// `OCS_PID_BACKUP_EXPORT_BROKEN` names none: the small sample.
    const SMALL_BACKUP: &str =
        r"D:\work\plant-code\cad\pid-parse\test-file\backup-test\DWG-0202GP06-01_p";

    /// What the broken copy holds in the place of a drawing's `.pid`.
    const NO_DRAWING: &[u8] = b"no SmartPlant drawing";

    /// What a folder holds: every file under it by its path from there,
    /// `/`-separated, with its length and when it was last written.
    type Listed = BTreeMap<String, (u64, Option<SystemTime>)>;

    /// What `root` holds. A file holds itself, under its own name.
    fn listed(root: &Path) -> Listed {
        let mut files = Listed::new();
        let mut left = vec![root.to_path_buf()];
        while let Some(at) = left.pop() {
            let metadata =
                std::fs::metadata(&at).unwrap_or_else(|error| panic!("{}: {error}", at.display()));
            if metadata.is_dir() {
                let entries = std::fs::read_dir(&at)
                    .unwrap_or_else(|error| panic!("{}: {error}", at.display()));
                left.extend(entries.map(|entry| entry.expect("an entry of the folder").path()));
                continue;
            }
            let inside = at.strip_prefix(root).ok();
            let inside = inside.filter(|inside| !inside.as_os_str().is_empty());
            let name = inside.or_else(|| at.file_name().map(Path::new));
            let name = name.unwrap_or(&at).to_string_lossy().replace('\\', "/");
            files.insert(name, (metadata.len(), metadata.modified().ok()));
        }
        files
    }

    /// The names two listings do not agree on: a file only one of them
    /// holds, or one whose length or time is not the same in both.
    fn unlike(one: &Listed, other: &Listed) -> Vec<String> {
        let names: BTreeSet<&String> = one.keys().chain(other.keys()).collect();
        let differs = |name: &&String| one.get(*name) != other.get(*name);
        names.into_iter().filter(differs).cloned().collect()
    }

    /// [`listed`] summed up for the report: how many files, how many bytes.
    fn counted(files: &Listed) -> serde_json::Value {
        let bytes: u64 = files.values().map(|(length, _)| length).sum();
        serde_json::json!({ "files": files.len(), "bytes": bytes })
    }

    /// The folders of `index` an export must leave alone, each once, with
    /// what each is to the backup.
    fn watched(index: &BackupIndex) -> Vec<(&'static str, PathBuf)> {
        let mut watched: Vec<(&'static str, PathBuf)> = Vec::new();
        let folders = [
            ("source", &index.source),
            ("package", &index.backup_root),
            ("restored", &index.restored),
        ];
        for (what, folder) in folders {
            if watched.iter().all(|(_, seen)| seen != folder) {
                watched.push((what, folder.clone()));
            }
        }
        watched
    }

    /// What became of the `watched` folders, by what they held `before` the
    /// export and `after` it: for the report, and the sentences of what
    /// changed.
    fn kept(
        watched: &[(&str, PathBuf)],
        before: &[Listed],
        after: &[Listed],
    ) -> (Vec<serde_json::Value>, Vec<String>) {
        let (mut kept, mut changed) = (Vec::new(), Vec::new());
        for ((what, folder), (before, after)) in watched.iter().zip(before.iter().zip(after)) {
            let differing = unlike(before, after);
            if !differing.is_empty() {
                let folder = folder.display();
                changed.push(format!(
                    "the export changed the {what} ({folder}): {differing:?}"
                ));
            }
            kept.push(serde_json::json!({
                "what": what,
                "folder": folder.display().to_string(),
                "before": counted(before),
                "after": counted(after),
                "changed": differing,
            }));
        }
        (kept, changed)
    }

    /// Copy the folder `from`, and all it holds, to `to`.
    fn copy_folder(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap_or_else(|error| panic!("{}: {error}", to.display()));
        let entries =
            std::fs::read_dir(from).unwrap_or_else(|error| panic!("{}: {error}", from.display()));
        for entry in entries {
            let entry = entry.expect("an entry of the folder");
            let (path, into) = (entry.path(), to.join(entry.file_name()));
            if path.is_dir() {
                copy_folder(&path, &into);
            } else {
                std::fs::copy(&path, &into)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            }
        }
    }

    /// Put [`NO_DRAWING`] in the place of the first `.pid` of the package's
    /// archives, by the archive's name and then by its own: the archive's
    /// name and the `.pid`'s, as the index states them.
    fn break_a_drawing(package: &Path) -> (String, String) {
        use std::io::Write;
        let entries = std::fs::read_dir(package)
            .unwrap_or_else(|error| panic!("{}: {error}", package.display()));
        let mut archives: Vec<PathBuf> = entries
            .map(|entry| entry.expect("an entry of the folder").path())
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with("PlantData") && name.to_ascii_lowercase().ends_with(".zip")
            })
            .collect();
        archives.sort();
        for archive in archives {
            let file = std::fs::File::open(&archive)
                .unwrap_or_else(|error| panic!("{}: {error}", archive.display()));
            let mut zip = zip::ZipArchive::new(file)
                .unwrap_or_else(|error| panic!("{}: {error}", archive.display()));
            let mut drawings: Vec<String> = zip
                .file_names()
                .filter(|name| name.to_ascii_lowercase().ends_with(".pid"))
                .map(str::to_string)
                .collect();
            drawings.sort();
            let Some(drawing) = drawings.into_iter().next() else {
                continue;
            };
            let broken = archive.with_extension("broken");
            let written = std::fs::File::create(&broken).expect("the broken archive");
            let mut writer = zip::ZipWriter::new(written);
            for at in 0..zip.len() {
                let file = zip.by_index_raw(at).expect("an entry of the archive");
                if file.name() == drawing {
                    let options = zip::write::SimpleFileOptions::default();
                    writer
                        .start_file(drawing.as_str(), options)
                        .expect("the drawing that is none");
                    writer.write_all(NO_DRAWING).expect("its bytes");
                } else {
                    writer.raw_copy_file(file).expect("an entry as it was");
                }
            }
            writer.finish().expect("the broken archive is written");
            drop(zip);
            std::fs::rename(&broken, &archive).expect("the broken archive in its place");
            let name = archive.file_name().unwrap_or_default();
            return (name.to_string_lossy().into_owned(), drawing);
        }
        panic!("no archive of {} holds a .pid", package.display());
    }

    /// One export of the backup an application holds, asked for as it is
    /// typed, and what the command line said of it on the way.
    struct RealExport {
        /// The line that asked for it.
        typed: String,
        /// What the command line said when the command returned.
        started: Vec<Said>,
        /// Whether the export ran when the command returned.
        running: bool,
        /// Whether the index was there by then.
        index_by_then: bool,
        /// What the bare verb said then.
        status: Vec<Said>,
        /// What the command line said when the export ended.
        ended: Vec<Said>,
        /// What the bare verb said after that.
        again: Vec<Said>,
        /// From the command to the end, in seconds.
        seconds: f64,
        /// What the export came to.
        result: Option<Result<ExportOutcome, ExportError>>,
    }

    /// Export the backup `app` holds into `folder` through the command, with
    /// the plant name and the reference folder when they are given, and wait
    /// for the end.
    fn export_for_real(
        app: &mut OpenCADStudio,
        folder: &Path,
        plant: Option<&str>,
        reference: Option<&Path>,
    ) -> RealExport {
        let mut typed = "PIDBACKUPEXPORT".to_string();
        if let Some(plant) = plant {
            typed += &format!(" Plant=\"{plant}\"");
        }
        if let Some(reference) = reference {
            typed += &format!(" Ref=\"{}\"", reference.display());
        }
        typed += &format!(" \"{}\"", folder.display());
        let clock = Instant::now();
        let task = start(app, &typed);
        let started = said(app);
        let running = app.pid_backup_export.running.is_some();
        let index_by_then = folder.join("_Index.json").exists();
        let status = ask(app, "PIDBACKUPEXPORT");
        let ended = finish(app, task);
        let seconds = clock.elapsed().as_secs_f64();
        let again = say(app, "PIDBACKUPEXPORT");
        RealExport {
            typed,
            started,
            running,
            index_by_then,
            status,
            ended,
            again,
            seconds,
            result: app.pid_backup_export.last.clone(),
        }
    }

    /// What the command line said, as text, for the report.
    fn texts(said: &[Said]) -> Vec<String> {
        let text = |said: &Said| match said {
            Said::Line(text) => text.clone(),
            Said::Error(text) => format!("error: {text}"),
            Said::Other(text) => format!("other: {text}"),
        };
        said.iter().map(text).collect()
    }

    /// The index an export wrote into `folder`, and the names of the files
    /// it lists, sorted, its own among them; why not, when it cannot be
    /// read.
    fn index_in(folder: &Path) -> Result<(serde_json::Value, Vec<String>), String> {
        let path = folder.join("_Index.json");
        let unread = |why: String| format!("{}: {why}", path.display());
        let text = std::fs::read_to_string(&path).map_err(|error| unread(error.to_string()))?;
        let index: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| unread(error.to_string()))?;
        let drawings = index.get("drawings").and_then(serde_json::Value::as_array);
        let files = drawings
            .into_iter()
            .flatten()
            .filter_map(|drawing| drawing.get("files")?.as_array())
            .flatten();
        let mut names: Vec<String> = files
            .filter_map(|file| file.as_str().map(str::to_string))
            .collect();
        names.push("_Index.json".to_string());
        names.sort();
        Ok((index, names))
    }

    /// What of `export`, of `package` into `folder` under the plant name
    /// `plant`, is not as SPEC-sppid-export §6 has it: the command returned
    /// with the export running and nothing of it written yet, and the bare
    /// verb said so (criterion 8); when the export ended the command line
    /// said its own summary, and the bare verb said it again (criterion 2);
    /// the folder holds the files the index lists and the index, and
    /// nothing else (criterion 1).
    fn real_unmet(
        export: &RealExport,
        package: &Path,
        folder: &Path,
        plant: &str,
        reference: Option<&Path>,
    ) -> Vec<String> {
        let mut unmet = Vec::new();
        let owed = [exporting(package, folder, plant, reference)];
        if export.started != owed {
            let started = &export.started;
            unmet.push(format!("the command said {started:?}, not {owed:?}"));
        }
        if !export.running {
            unmet.push("the export did not run when the command returned".to_string());
        }
        if export.index_by_then {
            unmet.push("the index was there when the command returned".to_string());
        }
        let so_far = format!(
            "PIDBACKUPEXPORT: exporting to {} (plant {plant}): ",
            folder.display()
        );
        let says_so = |said: &Said| matches!(said, Said::Line(text) if text.starts_with(&so_far) && text.ends_with(" s so far"));
        if !matches!(export.status.as_slice(), [said] if says_so(said)) {
            let status = &export.status;
            unmet.push(format!(
                "while the export ran the bare verb said {status:?}"
            ));
        }
        let outcome = match &export.result {
            Some(Ok(outcome)) => outcome,
            other => {
                let ended = &export.ended;
                unmet.push(format!(
                    "the export came to {other:?}; the command line said {ended:?}"
                ));
                return unmet;
            }
        };
        let summary: Vec<Said> = pid_backup_export::summary_lines(outcome)
            .iter()
            .map(|text| line(format!("PIDBACKUPEXPORT: {text}")))
            .collect();
        if export.ended != summary {
            let ended = &export.ended;
            unmet.push(format!(
                "when the export ended the command line said {ended:?}, not {summary:?}"
            ));
        }
        if export.again != export.ended {
            let again = &export.again;
            unmet.push(format!("after the export the bare verb said {again:?}"));
        }
        match index_in(folder) {
            Ok((_, owed)) => {
                let there = names(folder);
                if there != owed {
                    unmet.push(format!(
                        "the folder holds {there:?}, the index lists {owed:?}"
                    ));
                }
                if outcome.files != owed.len() {
                    let (files, listed) = (outcome.files, owed.len());
                    unmet.push(format!(
                        "the export says {files} files, the index lists {listed}"
                    ));
                }
            }
            Err(why) => unmet.push(why),
        }
        unmet
    }

    /// What `export`, into `folder`, came to and what was said of it, for
    /// the report.
    fn export_report(export: &RealExport, folder: &Path) -> serde_json::Value {
        let notes = |notes: &[DrawingNote]| -> Vec<serde_json::Value> {
            let note =
                |note: &DrawingNote| serde_json::json!({ "name": note.name, "notes": note.notes });
            notes.iter().map(note).collect()
        };
        let (outcome, error) = match &export.result {
            Some(Ok(outcome)) => {
                let gate = outcome.gate.as_ref().map(|gate| {
                    serde_json::json!({
                        "clean": gate.clean,
                        "dirty": gate.dirty,
                        "revision_mismatch": gate.revision_mismatch,
                        "unpaired": gate.unpaired,
                        "lines": gate.lines,
                    })
                });
                let outcome = serde_json::json!({
                    "written": outcome.written,
                    "failed": outcome.failed,
                    "pid_entries": outcome.pid_entries,
                    "templates": outcome.templates,
                    "unmatched": outcome.unmatched,
                    "without_drawing_no": outcome.without_drawing_no,
                    "unwritten": notes(&outcome.unwritten),
                    "noted": notes(&outcome.noted),
                    "problems": outcome.problems,
                    "other_problems": outcome.other_problems,
                    "warnings": outcome.warnings,
                    "gate": gate,
                    "export_kind": outcome.export_kind,
                    "sql_server": outcome.sql_server(),
                    "files": outcome.files,
                    "index_path": outcome.index_path.display().to_string(),
                    "elapsed_seconds": outcome.elapsed.as_secs_f64(),
                });
                (outcome, serde_json::Value::Null)
            }
            Some(Err(error)) => {
                let error = serde_json::json!({ "code": error.code(), "text": error.to_string() });
                (serde_json::Value::Null, error)
            }
            None => (serde_json::Value::Null, serde_json::Value::Null),
        };
        serde_json::json!({
            "folder": folder.display().to_string(),
            "typed": export.typed,
            "started": texts(&export.started),
            "running": export.running,
            "index_by_then": export.index_by_then,
            "status": texts(&export.status),
            "ended": texts(&export.ended),
            "again": texts(&export.again),
            "seconds": export.seconds,
            "outcome": outcome,
            "error": error,
            "files": names(folder),
        })
    }

    /// Open `source` in the P&ID module of a new application, keeping what
    /// it restores under `out`: the application, the index of the backup,
    /// what the command line said and how long the opening took.
    fn opened_for_real(
        source: &Path,
        out: &Path,
    ) -> (OpenCADStudio, Arc<BackupIndex>, Vec<Said>, f64) {
        let mut app = in_the_module();
        let cache = Some(out.join("restores"));
        pid_backup_browser::set_cache_folder(&mut app.pid_backup_browser, cache);
        let clock = Instant::now();
        let answer = say(&mut app, &open_line(source));
        let seconds = clock.elapsed().as_secs_f64();
        let Some(index) = app.pid_backup_browser.index().map(Arc::clone) else {
            panic!("{} did not open: {answer:?}", source.display());
        };
        (app, index, answer, seconds)
    }

    /// Write `report` into `out` as `report.json`.
    fn write_report(out: &Path, report: &serde_json::Value) {
        let text = serde_json::to_string_pretty(report).expect("the report as JSON");
        std::fs::write(out.join("report.json"), text + "\n").expect("the report is written");
    }

    /// A real backup through the commands: `PIDBACKUPOPEN` opens it in the
    /// P&ID module, `PIDBACKUPEXPORT` exports it into an empty folder, under
    /// the plant name and with the reference folder the caller names. Then,
    /// of SPEC-sppid-export §6:
    ///
    /// - the command returned with the export running and nothing of it
    ///   written yet, and the bare verb said so (criterion 8);
    /// - when the export ended the command line said its own summary, and
    ///   the bare verb said it again (criterion 2);
    /// - the folder holds the files the index lists and the index, and
    ///   nothing else: no file left half written (criterion 1);
    /// - what the backup was opened from, the backup's own folder and the
    ///   folder it is restored to hold the same files, of the same lengths
    ///   and times, after the export as before it (criterion 11).
    ///
    /// What was measured goes to `report.json` before anything is asserted.
    /// `scripts/sppid/export-compare.ps1` runs this test on every sample
    /// backup, has `pid_publish_xml` export the same package into a folder
    /// beside it, and compares the two folders and what each said.
    ///
    /// `OCS_PID_BACKUP_EXPORT_REAL` names the backup, a folder or a zip;
    /// `OCS_PID_BACKUP_EXPORT_REAL_OUT` a folder to keep the restore, the
    /// export and the report in; `OCS_PID_BACKUP_EXPORT_REAL_PLANT` the
    /// plant name and `OCS_PID_BACKUP_EXPORT_REAL_REF` the reference folder,
    /// none when they are not set. A backup that is not there fails the
    /// test: an export that did not run must not read as one that passed.
    #[test]
    #[ignore = "exports a real SmartPlant backup; run alone with --ignored --nocapture"]
    fn a_real_backup_is_exported_through_the_command_and_stays_as_it_was() {
        let named = |variable: &str| std::env::var_os(variable).map(PathBuf::from);
        let source = named("OCS_PID_BACKUP_EXPORT_REAL").unwrap_or_else(|| REAL_BACKUP.into());
        assert!(source.exists(), "no backup at {}", source.display());
        let plant = std::env::var("OCS_PID_BACKUP_EXPORT_REAL_PLANT").ok();
        let reference = named("OCS_PID_BACKUP_EXPORT_REAL_REF");
        let scratch = Scratch::new("real");
        let out = named("OCS_PID_BACKUP_EXPORT_REAL_OUT");
        let out = out.unwrap_or_else(|| scratch.path().to_path_buf());
        // What an earlier run left must not pass for this one's.
        for left in ["restores", "export", "report.json"] {
            let left = out.join(left);
            assert!(!left.exists(), "{} is there already", left.display());
        }
        std::fs::create_dir_all(&out).expect("the folder of the run");

        let (mut app, index, answer, open_seconds) = opened_for_real(&source, &out);
        let watched = watched(&index);
        let before: Vec<Listed> = watched.iter().map(|(_, folder)| listed(folder)).collect();
        let folder = out.join("export");
        let export = export_for_real(&mut app, &folder, plant.as_deref(), reference.as_deref());
        let after: Vec<Listed> = watched.iter().map(|(_, folder)| listed(folder)).collect();

        let plant = plant.as_deref().unwrap_or(pid_backup_export::DEFAULT_PLANT);
        let package = &index.backup_root;
        let mut unmet = real_unmet(&export, package, &folder, plant, reference.as_deref());
        let (kept, changed) = kept(&watched, &before, &after);
        unmet.extend(changed);
        let report = serde_json::json!({
            "source": index.source.display().to_string(),
            "package": package.display().to_string(),
            "restored": index.restored.display().to_string(),
            "used_in_place": index.read_only,
            "drawings": index.drawings.len(),
            "debug_assertions": cfg!(debug_assertions),
            "plant": plant,
            "reference": reference.as_ref().map(|reference| reference.display().to_string()),
            "opened": texts(&answer),
            "open_seconds": open_seconds,
            "export": export_report(&export, &folder),
            "kept": kept,
            "unmet": unmet,
        });
        write_report(&out, &report);
        eprintln!(
            "{}: {} drawings; exported in {:.1} s; opened in {open_seconds:.1} s; report in {}",
            source.display(),
            index.drawings.len(),
            export.seconds,
            out.display()
        );
        assert!(
            unmet.is_empty(),
            "{} thing(s) are not as they should be:\n{}",
            unmet.len(),
            unmet.join("\n")
        );
    }

    /// A copy of a real backup with one drawing's `.pid` made into no
    /// drawing, through the commands as [the test of a real
    /// backup](a_real_backup_is_exported_through_the_command_and_stays_as_it_was)
    /// goes, and that test's checks. Then, of SPEC-sppid-export §6
    /// (criterion 6): the index lists no file of that drawing, the export
    /// names it among the ones it could not write and gives the reason, it
    /// writes every other drawing, and its failure count is not 0.
    ///
    /// `OCS_PID_BACKUP_EXPORT_BROKEN` names the backup to copy, a folder;
    /// `OCS_PID_BACKUP_EXPORT_BROKEN_OUT` a folder to keep the copy, the
    /// restore, the export and the report in. The `.pid` that is made into
    /// no drawing is the first of the package's archives, by the archive's
    /// name and then by its own.
    #[test]
    #[ignore = "exports a broken copy of a real SmartPlant backup; run alone with --ignored --nocapture"]
    fn a_real_backup_with_a_broken_drawing_names_it_and_writes_the_others() {
        let named = |variable: &str| std::env::var_os(variable).map(PathBuf::from);
        let source = named("OCS_PID_BACKUP_EXPORT_BROKEN").unwrap_or_else(|| SMALL_BACKUP.into());
        assert!(source.is_dir(), "no backup folder at {}", source.display());
        let scratch = Scratch::new("broken");
        let out = named("OCS_PID_BACKUP_EXPORT_BROKEN_OUT");
        let out = out.unwrap_or_else(|| scratch.path().to_path_buf());
        for left in ["package", "restores", "export", "report.json"] {
            let left = out.join(left);
            assert!(!left.exists(), "{} is there already", left.display());
        }
        let copy = out.join("package");
        copy_folder(&source, &copy);
        let (archive, entry) = break_a_drawing(&copy);

        let (mut app, index, answer, open_seconds) = opened_for_real(&copy, &out);
        let watched = watched(&index);
        let before: Vec<Listed> = watched.iter().map(|(_, folder)| listed(folder)).collect();
        let folder = out.join("export");
        let export = export_for_real(&mut app, &folder, None, None);
        let after: Vec<Listed> = watched.iter().map(|(_, folder)| listed(folder)).collect();

        let plant = pid_backup_export::DEFAULT_PLANT;
        let package = &index.backup_root;
        let mut unmet = real_unmet(&export, package, &folder, plant, None);
        let (kept, changed) = kept(&watched, &before, &after);
        unmet.extend(changed);

        // The drawing that is none, as the index has it, and the others.
        let entries = index_in(&folder).map_or_else(
            |why| {
                unmet.push(why);
                serde_json::Value::Null
            },
            |(index, _)| index,
        );
        let drawings = entries
            .get("drawings")
            .and_then(serde_json::Value::as_array);
        let text = |drawing: &serde_json::Value, key: &str| {
            let text = drawing.get(key).and_then(serde_json::Value::as_str);
            text.unwrap_or_default().to_string()
        };
        let files = |drawing: &serde_json::Value| {
            let files = drawing.get("files").and_then(serde_json::Value::as_array);
            files.map_or(0, Vec::len)
        };
        let (hit, others): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) =
            drawings.into_iter().flatten().partition(|drawing| {
                text(drawing, "zip_file") == archive && text(drawing, "zip_entry") == entry
            });
        let name = match hit.as_slice() {
            [drawing] => {
                let name = text(drawing, "name");
                if files(drawing) != 0 {
                    unmet.push(format!(
                        "the index lists files of {name}, whose .pid is none"
                    ));
                }
                Some(name)
            }
            hit => {
                let count = hit.len();
                unmet.push(format!(
                    "the index has {count} drawing(s) of {archive} {entry}"
                ));
                None
            }
        };
        let unwritten: Vec<String> = others
            .iter()
            .filter(|drawing| files(drawing) == 0)
            .map(|drawing| text(drawing, "name"))
            .collect();
        if !unwritten.is_empty() {
            unmet.push(format!(
                "drawings whose .pid is whole were not written: {unwritten:?}"
            ));
        }
        if let Some(Ok(outcome)) = &export.result {
            let named = |note: &&DrawingNote| Some(&note.name) == name.as_ref();
            let notes: Vec<&DrawingNote> = outcome.unwritten.iter().filter(named).collect();
            if outcome.unwritten.len() != 1
                || !matches!(notes.as_slice(), [note] if !note.notes.is_empty())
            {
                let said = &outcome.unwritten;
                unmet.push(format!(
                    "the export names {said:?} as not written, not {name:?} with the reason"
                ));
            }
            if outcome.failed == 0 {
                unmet.push("the failure count is 0".to_string());
            }
            if outcome.written != others.len() {
                let (written, whole) = (outcome.written, others.len());
                unmet.push(format!("{written} drawing(s) written of {whole} whole"));
            }
        }
        let broken = serde_json::json!({
            "copied_from": source.display().to_string(),
            "archive": archive,
            "entry": entry,
            "drawing": name,
            "others": others.len(),
        });
        let report = serde_json::json!({
            "source": index.source.display().to_string(),
            "package": package.display().to_string(),
            "restored": index.restored.display().to_string(),
            "used_in_place": index.read_only,
            "drawings": index.drawings.len(),
            "debug_assertions": cfg!(debug_assertions),
            "plant": plant,
            "reference": serde_json::Value::Null,
            "opened": texts(&answer),
            "open_seconds": open_seconds,
            "broken": broken,
            "export": export_report(&export, &folder),
            "kept": kept,
            "unmet": unmet,
        });
        write_report(&out, &report);
        eprintln!(
            "{}: {entry} of {archive} made into no drawing; exported in {:.1} s; report in {}",
            copy.display(),
            export.seconds,
            out.display()
        );
        assert!(
            unmet.is_empty(),
            "{} thing(s) are not as they should be:\n{}",
            unmet.len(),
            unmet.join("\n")
        );
    }

    #[test]
    fn ui2_ui_export_shares_running_and_structured_outcome_with_command() {
        let _one = crate::headless_ui_lock();
        let scratch = Scratch::new("ui2-publish-shared");
        let mut app = in_the_module();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);

        assert!(!app.pid_backup_export.is_running());
        assert!(app.pid_backup_export.last_outcome().is_none());
        assert!(!app.pid_backup_wants_second_tick());

        let out_dir = scratch.join("out-ui");
        let ref_dir = scratch.join("ref");
        std::fs::create_dir_all(&ref_dir).expect("ref dir");
        let task = app
            .start_pid_backup_export_ui(out_dir.clone(), "P99".into(), Some(ref_dir.clone()))
            .expect("UI export starts");

        assert!(app.pid_backup_export.is_running());
        assert!(app.pid_backup_wants_second_tick());
        let (req, _elapsed) = app
            .pid_backup_export
            .running_status(Instant::now())
            .expect("running status available");
        assert_eq!(req.out_dir, out_dir);
        assert_eq!(req.plant, "P99");
        assert_eq!(req.reference_dir.as_deref(), Some(ref_dir.as_path()));

        let _ = app.dispatch_command("PIDBACKUPEXPORT");
        let last_line = &app.command_line.history.last().expect("status line").text;
        assert!(last_line.contains("so far"), "got: {last_line}");

        let _ = app.dispatch_command("PIDBACKUPOPEN Cache=Clear");
        assert_eq!(
            app.command_line.last_error_code.as_deref(),
            Some("SPPID_EXPORT_BUSY")
        );

        let expected = compared(&out_dir);
        stand_in.end(Ok(expected.clone()));
        app.drive_headless_task(task).expect("export task finishes");

        assert!(!app.pid_backup_export.is_running());
        assert!(!app.pid_backup_wants_second_tick());
        let stored = app
            .pid_backup_export
            .last_outcome()
            .expect("structured outcome kept")
            .as_ref()
            .expect("export succeeded");
        assert_eq!(stored.written, expected.written);
        assert_eq!(stored.index_path, expected.index_path);
        assert_eq!(
            stored.gate.as_ref().map(|g| g.dirty),
            Some(1),
            "gate summary kept structurally"
        );
        assert!(stored.sql_server());
    }

    #[test]
    fn ui2_ui_export_preflight_rejects_bad_targets_without_clobbering_last() {
        let scratch = Scratch::new("ui2-publish-preflight");
        let mut app = app();
        open_package(&mut app, &scratch);
        let stand_in = StandIn::put_into(&mut app);

        let first_out = scratch.join("out-ok");
        let task = app
            .start_pid_backup_export_ui(first_out.clone(), "P01".into(), None)
            .expect("first export starts");
        stand_in.end(Ok(outcome(&first_out)));
        app.drive_headless_task(task).expect("first export ends");
        assert!(matches!(app.pid_backup_export.last_outcome(), Some(Ok(_))));

        let non_empty = scratch.join("non-empty");
        std::fs::create_dir_all(&non_empty).unwrap();
        std::fs::write(non_empty.join("existing.txt"), "x").unwrap();
        assert!(matches!(
            app.check_pid_backup_export_ui(non_empty.clone(), "P01", None),
            Err(ExportError::TargetNotEmpty { .. })
        ));
        assert!(matches!(
            app.start_pid_backup_export_ui(non_empty, "P01".into(), None),
            Err(ExportError::TargetNotEmpty { .. })
        ));

        let in_restore = scratch.join("restored").join("sub");
        assert!(matches!(
            app.check_pid_backup_export_ui(in_restore, "P01", None),
            Err(ExportError::TargetInvalid(_))
        ));

        let missing_ref = scratch.join("no-such-ref");
        assert!(matches!(
            app.check_pid_backup_export_ui(scratch.join("out-2"), "P01", Some(missing_ref)),
            Err(ExportError::ReferenceInvalid(_))
        ));

        assert!(matches!(
            app.check_pid_backup_export_ui(scratch.join("out-3"), "   ", None),
            Err(ExportError::TargetInvalid(_))
        ));

        let preserved = app
            .pid_backup_export
            .last_outcome()
            .expect("previous outcome still stored")
            .as_ref()
            .expect("previous outcome still Ok");
        assert_eq!(preserved.index_path, first_out.join("_Index.json"));
    }
}

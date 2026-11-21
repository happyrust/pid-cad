//! PIDBOM：材料清单（`pid::bom`，`SPEC-pid-bom.md` §4、§5.2）。
//!
//! ```text
//! PIDBOM                    算当前图的清单：各种类几行、管长合计、跳过了几个对象
//! PIDBOM PROJECT            按项目算：项目目录里的全部图
//! PIDBOM PBS <节点号>       按节点算：项目文件里归在这个节点及其下级的图
//! PIDBOM SNAPSHOT <序号>    按快照算：当前图已存的第几份快照（PIDSNAPSHOT 存的）
//! PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>]
//! PIDBOM SNAPSHOT <序号> EXPORT <文件>
//!                           算了写成文件，扩展名定格式（.csv、.xlsx、.json）；CSV、XLSX
//!                           另在同目录写副表 <文件名>-pipe-summary.<扩展名>
//! ```
//!
//! 清单都从 [`OpenCADStudio::pidbom_compute`] 算：取范围内各图的摘要，交给 `bom::build`。
//! 当前图现做摘要，含没保存的修改；项目、节点范围用 `commands::pidindex` 的项目扫描（默认的
//! 连接容差，与 `PIDINDEX` 共用摘要缓存），要从磁盘读的图在后台读，读完接着算。按节点算时
//! 节点号要在主数据的 PBS 表里（`PBS_NODE_UNKNOWN`）。范围内有图读不出来就不出清单
//! （`BOM_SCOPE_INCOMPLETE`，§9 B4），逐张说原因；范围外读不出来的不管。有重复的（类型编码，
//! 位号）也不出（`BOM_DUPLICATE_TAG`），逐个说各在哪张图的哪里。
//!
//! 按快照算（`PIDBOM SNAPSHOT`）用快照里存的摘要，快照目录同 `PIDSNAPSHOT`，读时校验：图没存过
//! 没有快照（`INDEX_NO_PROJECT_DIR`），序号不在 `SNAPSHOT_NOT_FOUND`，坏了 `SNAPSHOT_CORRUPT`。
//! 只算这一张图；清单的 `source` 是 `snapshot`，`snapshot_seq` 是序号。
//!
//! 导出：文件不是完整路径时从当前图所在的目录算起，没存过的图从当前目录算起；扩展名不分大小写，
//! 别的扩展名报 `BOM_FORMAT_UNSUPPORTED`，这时还没算清单；不拿清单盖项目文件。目标文件已存在
//! 时覆盖。清单算不出来、转表出错时一个文件也不写。JSON 是 §3.2 的对象，带 `ok`，缩进 2 格，
//! 副表在里面。
//!
//! 自动化操作（§5.3）同命令：`pid_bom` 照 `scope`（`drawing`、`project`、`pbs`）算，应答是清单带
//! `ok`；`pid_bom_export` 照 `path` 的扩展名写文件，结果是写到哪里、什么格式、几行、几个字节、
//! 副表写到哪里。要从磁盘读图时先应答 `scanning`，调用方稍后再发；导出在登记写操作以前等。
//! 带 `snapshot`（快照序号，只和 `drawing` 一起）同 `PIDBOM SNAPSHOT`，不读别的图，不用等。
//!
//! 几个命令、操作都不改图，只读会话里照常能用：导出写的是本机文件，不是图纸（§5.4）。

use super::piddrc::{pbs_known, PBS_NODE_UNKNOWN};
use super::pidindex::{point, Scanned, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::app::annotation_data::write_extraction_file;
use crate::pid::bom::{
    self, Bom, BomError, BomOptions, BomScope, BomSource, ByKind, Quantity, ScopeKind, SkipReason,
    Skipped,
};
use crate::pid::master_data::{self, Resolved, PROJECT_FILE};
use crate::pid::project_index::{
    summarize, DrawingSummary, IndexDrawing, IndexError, ObjectKind, DEFAULT_TOLERANCE,
};
use crate::pid::snapshot::{store, Snapshot, SnapshotError};
use serde::Serialize;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 导出文件的扩展名不是 `.csv`、`.xlsx`、`.json`（§3.5）。
pub(in crate::app) const BOM_FORMAT_UNSUPPORTED: &str = "BOM_FORMAT_UNSUPPORTED";

/// 范围内有图读不出来，不出清单（§3.5、§9 B4）。
pub(in crate::app) const BOM_SCOPE_INCOMPLETE: &str = "BOM_SCOPE_INCOMPLETE";

/// 按 PBS 节点算而没给节点号（§3.5）。
pub(in crate::app) const SCOPE_NOT_SELECTED: &str = "SCOPE_NOT_SELECTED";

/// `pid_bom_export` 写盘出错：请求和图纸都没错，是这台机器的文件系统报的错。
pub(in crate::app) const BOM_IO_ERROR: &str = "BOM_IO_ERROR";

/// 副表的文件名：主表的文件名（去掉扩展名）后面加上它，扩展名照旧（§5.2）。
const PIPE_SUMMARY_SUFFIX: &str = "-pipe-summary";

/// 跳过的对象、重复的键、读不出来的图，命令行上各最多逐个列出这么多。
const LISTED: usize = 10;

const USAGE: &str =
    "PIDBOM: 写成 PIDBOM、PIDBOM PROJECT、PIDBOM PBS <节点号> 或 PIDBOM SNAPSHOT <序号>；导出写成 PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>] 或 PIDBOM SNAPSHOT <序号> EXPORT <文件>。";

const EXPORT_USAGE: &str =
    "PIDBOM EXPORT: 写成 PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>]，按快照导出写成 PIDBOM SNAPSHOT <序号> EXPORT <文件>；文件名里有空格就加双引号。";

const SNAPSHOT_USAGE: &str =
    "PIDBOM SNAPSHOT: 写成 PIDBOM SNAPSHOT <序号> [EXPORT <文件>]，序号见 PIDSNAPSHOT LIST；文件名里有空格就加双引号。";

/// 要扫项目的范围：全项目，或一个 PBS 节点及其下级。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Range {
    Project,
    Pbs(String),
}

/// 扫完项目接着算的：范围，读出来的主数据（按节点算要找节点的下级），要写的文件。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BomJob {
    range: Range,
    master: Resolved,
    export: Option<Export>,
}

/// 清单范围里的图从哪来。
pub(super) enum Sheets<'a> {
    /// 第 `i` 个标签页的图，现做摘要。
    Current(usize),
    /// 第 `i` 个标签页的图已存的第 `seq` 份快照，用快照里的摘要。
    Snapshot(usize, u64),
    /// 一次项目扫描的结果，取 `range` 里的图；按节点取时节点及其下级查 `master`。
    Scanned {
        scanned: &'a Scanned,
        range: &'a Range,
        master: &'a Resolved,
    },
}

/// 算不出清单的原因。
#[derive(Debug)]
pub(super) enum BomFailure {
    /// 定当前图的图号时读项目目录、项目文件出错。
    Index(IndexError),
    /// 范围内这几张图读不出来（`BOM_SCOPE_INCOMPLETE`）。
    Incomplete(Vec<IndexDrawing>),
    /// `bom::build` 报的错（`BOM_DUPLICATE_TAG`）。
    Bom(BomError),
    /// 按快照算而图没存过，没有快照目录（`INDEX_NO_PROJECT_DIR`）。
    NoProjectDir,
    /// 读快照的错：序号不在（`SNAPSHOT_NOT_FOUND`）、坏了（`SNAPSHOT_CORRUPT`）等（快照规格 §3.8）。
    Snapshot(SnapshotError),
}

impl BomFailure {
    /// 错误码（§3.5）。
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Index(error) => error.code(),
            Self::Incomplete(_) => BOM_SCOPE_INCOMPLETE,
            Self::Bom(error) => error.code(),
            Self::NoProjectDir => INDEX_NO_PROJECT_DIR,
            Self::Snapshot(error) => error.code(),
        }
    }
}

impl fmt::Display for BomFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(error) => write!(f, "{error}"),
            Self::Incomplete(drawings) => {
                let names: Vec<&str> = drawings
                    .iter()
                    .map(|drawing| drawing.drawing_id.as_str())
                    .collect();
                write!(
                    f,
                    "{BOM_SCOPE_INCOMPLETE}: 范围内这些图读不出来：{}；清单不产出",
                    names.join("、")
                )
            }
            Self::Bom(error) => write!(f, "{error}"),
            Self::NoProjectDir => write!(
                f,
                "{INDEX_NO_PROJECT_DIR}: 当前图没保存过，没有带序号的快照；存盘、存过快照后再按快照算"
            ),
            Self::Snapshot(error) => write!(f, "{error}"),
        }
    }
}

/// 要写的文件和它的格式。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Export {
    path: PathBuf,
    format: Format,
}

/// 导出的格式，由文件扩展名定（§3.4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Csv,
    Xlsx,
    Json,
}

impl Format {
    /// `path` 的扩展名对应的格式，不分大小写；别的扩展名、没有扩展名是 `None`。
    fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        [Self::Csv, Self::Xlsx, Self::Json]
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
            Self::Json => "json",
        }
    }

    /// 说给人听的名字。
    fn name(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Xlsx => "XLSX",
            Self::Json => "JSON",
        }
    }
}

/// 写好的：主表的文件、格式、几行（不算列名）、文件几个字节，副表的文件和几行（JSON 没有）。
#[derive(Debug)]
struct Written {
    path: PathBuf,
    format: Format,
    rows: usize,
    bytes: u64,
    pipe_summary: Option<(PathBuf, usize)>,
}

/// 写不成文件的原因。
#[derive(Debug)]
enum ExportError {
    /// 选的列不认识（`BOM_COLUMN_UNKNOWN`），一个文件也没写。
    Columns(BomError),
    /// 写盘出错。
    Io { path: PathBuf, reason: String },
}

/// `PIDBOM EXPORT` 写的 JSON：§3.2 的清单，带上应答外面那层的 `ok`。
#[derive(Serialize)]
struct Exported<'a> {
    ok: bool,
    #[serde(flatten)]
    bom: &'a Bom,
}

/// `pid_bom_export` 的请求：范围、要写的文件、主表选的列（`None` 是默认九列）。
struct ExportAsked {
    scope: Asked,
    export: Export,
    columns: Option<Vec<String>>,
}

/// `pid_bom`、`pid_bom_export` 请求的范围（§5.3）。
enum Asked {
    /// 当前图，现做摘要。
    Drawing,
    /// 当前图已存的第几份快照（`snapshot`）。
    Snapshot(u64),
    /// 按项目、按节点算，要扫项目。
    Range(Range),
}

/// 范围的写法（关键字不分大小写）：什么都不写是当前图，`PROJECT`，`PBS <节点号>`。
enum Scoped {
    Drawing,
    Range(Range),
    /// `PBS` 后面没写节点号（`SCOPE_NOT_SELECTED`）。
    NoNode,
    /// 写的不是上面几种。
    Wrong,
}

impl Scoped {
    fn of(text: &str) -> Self {
        let (word, rest) = first_word(text);
        if word.is_empty() {
            return Self::Drawing;
        }
        if word.eq_ignore_ascii_case("PROJECT") && rest.is_empty() {
            return Self::Range(Range::Project);
        }
        if !word.eq_ignore_ascii_case("PBS") {
            return Self::Wrong;
        }
        match first_word(rest) {
            ("", _) => Self::NoNode,
            (node, "") => Self::Range(Range::Pbs(node.to_owned())),
            _ => Self::Wrong,
        }
    }
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidbom(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (word, rest) = first_word(cmd);
        if !word.eq_ignore_ascii_case("PIDBOM") {
            return None;
        }
        let (option, after) = first_word(rest);
        if option.eq_ignore_ascii_case("SNAPSHOT") {
            self.pidbom_snapshot_run(i, after);
            return Some(Task::none());
        }
        let (export, scope) = if option.eq_ignore_ascii_case("EXPORT") {
            let Some((name, scope)) = file_word(after) else {
                self.command_line.push_error(EXPORT_USAGE);
                return Some(Task::none());
            };
            let Some(export) = self.pidbom_target(i, "PIDBOM EXPORT", name) else {
                return Some(Task::none());
            };
            (Some(export), scope)
        } else {
            (None, rest)
        };
        let task = match Scoped::of(scope) {
            Scoped::Drawing => {
                let outcome = self.pidbom_compute(Sheets::Current(i));
                self.pidbom_done(verb_of(None, export.is_some()), outcome, export.as_ref());
                Task::none()
            }
            Scoped::Range(range) => self.pidbom_start(i, range, export),
            Scoped::NoNode => {
                let (verb, written) = if export.is_some() {
                    ("PIDBOM EXPORT", "PIDBOM EXPORT <文件> PBS <节点号>")
                } else {
                    ("PIDBOM PBS", "PIDBOM PBS <节点号>")
                };
                let said =
                    format!("{verb}: {SCOPE_NOT_SELECTED}: 按节点算要给节点号，写成 {written}。");
                self.command_line
                    .push_error_coded(SCOPE_NOT_SELECTED, &said);
                Task::none()
            }
            Scoped::Wrong => {
                let usage = if export.is_some() {
                    EXPORT_USAGE
                } else {
                    USAGE
                };
                self.command_line.push_error(usage);
                Task::none()
            }
        };
        Some(task)
    }

    /// 导出要写的文件（[`export_target`]），从第 `i` 个标签页的图所在目录算起。扩展名不支持
    /// （`BOM_FORMAT_UNSUPPORTED`）、是项目文件时在命令行上说（`verb` 打头），得 `None`。
    fn pidbom_target(&mut self, i: usize, verb: &str, name: &str) -> Option<Export> {
        let dir = crate::io::drawing_dir(&self.tabs[i].scene.document);
        let (code, why) = match export_target(dir, name) {
            Ok(export) => return Some(export),
            Err(refused) => refused,
        };
        let said = format!("{verb}: {why}");
        match code {
            Some(code) => self.command_line.push_error_coded(code, &said),
            None => self.command_line.push_error(&said),
        }
        None
    }

    /// `PIDBOM SNAPSHOT <序号> [EXPORT <文件>]`（§5.2）：按第 `i` 个标签页的图已存的这份快照算，
    /// 说清单，要导出的同 `PIDBOM EXPORT` 写成文件（先认文件再读快照）。写错了说怎么写。
    fn pidbom_snapshot_run(&mut self, i: usize, text: &str) {
        let verb = "PIDBOM SNAPSHOT";
        let Some((seq, name)) = snapshot_words(text) else {
            self.command_line.push_error(SNAPSHOT_USAGE);
            return;
        };
        let export = match name {
            None => None,
            Some(name) => {
                let Some(export) = self.pidbom_target(i, verb, name) else {
                    return;
                };
                Some(export)
            }
        };
        let outcome = self.pidbom_compute(Sheets::Snapshot(i, seq));
        self.pidbom_done(verb, outcome, export.as_ref());
    }

    /// 按项目、按节点算（导出）：定项目目录、读主数据，按节点算时认节点号，再扫项目，扫完
    /// 接着算（[`Self::pidbom_scanned`]）。
    fn pidbom_start(&mut self, i: usize, range: Range, export: Option<Export>) -> Task<Message> {
        let verb = verb_of(Some(&range), export.is_some());
        let Some(project_dir) =
            crate::io::drawing_dir(&self.tabs[i].scene.document).map(Path::to_path_buf)
        else {
            self.command_line.push_error_coded(
                INDEX_NO_PROJECT_DIR,
                &format!(
                    "{verb}: {INDEX_NO_PROJECT_DIR}: 当前图没保存过，不属于哪个项目；存盘后再算。"
                ),
            );
            return Task::none();
        };
        let master = master_data::resolve(&self.tabs[i].scene.document, Some(&project_dir));
        if let Range::Pbs(node) = &range {
            if let Err(why) = pbs_known(&master, node) {
                self.command_line.push_error_coded(
                    PBS_NODE_UNKNOWN,
                    &format!("{verb}: {PBS_NODE_UNKNOWN}: {why}。"),
                );
                return Task::none();
            }
        }
        self.pidindex_bom(
            i,
            BomJob {
                range,
                master,
                export,
            },
        )
    }

    /// 扫完项目接着算（`commands::pidindex`）：在扫好的项目上算 `job` 的范围，说出来，要导出
    /// 的写成文件。
    pub(super) fn pidbom_scanned(&mut self, scanned: &Scanned, job: BomJob) {
        let BomJob {
            range,
            master,
            export,
        } = job;
        let sheets = Sheets::Scanned {
            scanned,
            range: &range,
            master: &master,
        };
        let outcome = self.pidbom_compute(sheets);
        self.pidbom_done(
            verb_of(Some(&range), export.is_some()),
            outcome,
            export.as_ref(),
        );
    }

    /// 算清单（§4）：取范围内各图的摘要，交给 `bom::build`。命令、自动化操作，以后的面板和
    /// 发布包都从这里算。当前图现做摘要（默认的连接容差）；快照用里面存的摘要
    /// （[`Self::pidbom_snapshot`]），清单的 `source` 是 `snapshot`；项目、节点范围取扫好的项目里在
    /// 范围内的图（[`in_scope`]）。得到清单和算进去的图数。不碰命令行。
    pub(super) fn pidbom_compute(&self, sheets: Sheets<'_>) -> Result<(Bom, usize), BomFailure> {
        let (summaries, scope, source) = match sheets {
            Sheets::Current(i) => {
                let meta = self.drawing_meta(i).map_err(BomFailure::Index)?;
                let scope = BomScope::drawing(&meta.drawing_id);
                let document = &self.tabs[i].scene.document;
                let summary = summarize(document, &self.pid_symbols, meta, DEFAULT_TOLERANCE);
                (Cow::Owned(vec![summary]), scope, BomSource::Live)
            }
            Sheets::Snapshot(i, seq) => {
                let snapshot = self.pidbom_snapshot(i, seq)?;
                let scope = BomScope::drawing(&snapshot.drawing_id);
                let summaries = Cow::Owned(vec![snapshot.summary]);
                (summaries, scope, BomSource::Snapshot(seq))
            }
            Sheets::Scanned {
                scanned,
                range,
                master,
            } => {
                let (summaries, scope) = in_scope(scanned, range, master)?;
                (summaries, scope, BomSource::Live)
            }
        };
        let options = BomOptions {
            scope,
            source,
            generated_at: unix_now(),
            symbol_labels: bom::symbol_labels(&self.pid_symbols),
        };
        let bom = bom::build(&summaries, &options).map_err(BomFailure::Bom)?;
        Ok((bom, summaries.len()))
    }

    /// 第 `i` 个标签页的图已存的第 `seq` 份快照，读时校验；快照目录同 `PIDSNAPSHOT`（快照规格
    /// §3.1）。图没存过时没有快照目录，是 [`BomFailure::NoProjectDir`]。
    fn pidbom_snapshot(&self, i: usize, seq: u64) -> Result<Snapshot, BomFailure> {
        let document = &self.tabs[i].scene.document;
        let drawing_dir = crate::io::drawing_dir(document).ok_or(BomFailure::NoProjectDir)?;
        let meta = self.drawing_meta(i).map_err(BomFailure::Index)?;
        let dir = store::dir_for(drawing_dir, &meta.drawing_id);
        store::load(&dir, seq).map_err(BomFailure::Snapshot)
    }

    /// 算完了：说清单（[`Self::pidbom_say`]），要导出的写成文件；算不出来的连同错误码说为什么
    /// （[`Self::pidbom_refuse`]），不写文件。
    fn pidbom_done(
        &mut self,
        verb: &str,
        outcome: Result<(Bom, usize), BomFailure>,
        export: Option<&Export>,
    ) {
        let (bom, drawings) = match outcome {
            Ok(computed) => computed,
            Err(failure) => {
                self.pidbom_refuse(verb, &failure);
                return;
            }
        };
        self.pidbom_say(verb, &bom, drawings);
        let Some(export) = export else {
            return;
        };
        match write_bom(&bom, export, None) {
            Ok(written) => self.command_line.push_output(&written_said(verb, &written)),
            Err(ExportError::Columns(error)) => {
                let said = format!("{verb}: {error}");
                self.command_line.push_error_coded(error.code(), &said);
            }
            Err(ExportError::Io { path, reason }) => {
                let said = format!("{verb}: 写不了 {}：{reason}", path.display());
                self.command_line.push_error(&said);
            }
        }
    }

    /// 说一份清单：范围、几行（按种类）、管长合计、跳过几个对象；逐个列出跳过的，逐条说提示。
    fn pidbom_say(&mut self, verb: &str, bom: &Bom, drawings: usize) {
        let length: f64 = bom
            .rows
            .iter()
            .map(|row| match row.quantity {
                Quantity::Length(metres) => metres,
                Quantity::Count(_) => 0.0,
            })
            .sum();
        self.command_line.push_output(&format!(
            "{verb}: {}{} 行{}，管长合计 {length:.2} m，跳过 {} 个对象。",
            scope_said(bom, drawings),
            bom.totals.rows,
            kinds_said(&bom.totals.by_kind),
            bom.skipped.len()
        ));
        let skipped: Vec<String> = bom.skipped.iter().map(skipped_said).collect();
        self.pidbom_list(&skipped, "跳过的对象");
        for notice in &bom.notices {
            let said = format!("{verb}: {}: {}", notice.code, notice.message);
            self.command_line.push_info(&said);
        }
    }

    /// 算不出清单：连同错误码说为什么，再逐个列出重复的键各在哪张图的哪里、读不出来的图和原因。
    fn pidbom_refuse(&mut self, verb: &str, failure: &BomFailure) {
        self.command_line
            .push_error_coded(failure.code(), &format!("{verb}: {failure}"));
        match failure {
            BomFailure::Bom(BomError::DuplicateTag(duplicates)) => {
                let lines: Vec<String> = duplicates
                    .iter()
                    .map(|duplicate| {
                        let places: Vec<String> = duplicate
                            .places
                            .iter()
                            .map(|place| {
                                format!("{} {}", drawing_said(&place.drawing_id), point(place.at))
                            })
                            .collect();
                        format!(
                            "  {} {}：{}",
                            duplicate.type_code,
                            duplicate.tag,
                            places.join("、")
                        )
                    })
                    .collect();
                self.pidbom_list(&lines, "重复的键");
            }
            BomFailure::Incomplete(drawings) => {
                let lines: Vec<String> = drawings
                    .iter()
                    .map(|drawing| {
                        let file = drawing
                            .file
                            .as_deref()
                            .map_or_else(String::new, |file| format!("（{file}）"));
                        let why = drawing.error.as_deref().unwrap_or_default();
                        format!("  {}{file}：{why}", drawing.drawing_id)
                    })
                    .collect();
                self.pidbom_list(&lines, "读不出来的图");
            }
            BomFailure::Index(_)
            | BomFailure::Bom(_)
            | BomFailure::NoProjectDir
            | BomFailure::Snapshot(_) => {}
        }
    }

    /// 逐行列出 `lines`，最多 [`LISTED`] 行，多的说还有几个 `what` 没列出。
    fn pidbom_list(&mut self, lines: &[String], what: &str) {
        for line in lines.iter().take(LISTED) {
            self.command_line.push_output(line);
        }
        if lines.len() > LISTED {
            let more = lines.len() - LISTED;
            self.command_line
                .push_output(&format!("  还有 {more} 个{what}没列出。"));
        }
    }
}

/// 自动化操作（§5.3）：应答和失败的写法同别的控制面操作，不碰命令行。
impl OpenCADStudio {
    /// 控制面的 `pid_bom`：照 `scope` 算，应答是清单（§3.2）带 `ok`。当前图同 `PIDBOM`，带
    /// `snapshot` 同 `PIDBOM SNAPSHOT`，按项目、按节点同 `PIDBOM PROJECT`、`PIDBOM PBS`；要读的图
    /// 缓存里没有时应答 `scanning` 并交出后台读的任务（[`Self::op_scan`]），调用方稍后再发。算不
    /// 出来时带上重复的键或读不出来的图（[`failure_answer`]）。
    pub(in crate::app) fn control_pid_bom(&mut self, req: &Value) -> (Value, Task<Message>) {
        if self.tabs[self.active_tab].is_start {
            return no_task(refuse("no_document", "Create or open a drawing first"));
        }
        let asked = match scope_asked(req) {
            Ok(asked) => asked,
            Err(refusal) => return no_task(refusal),
        };
        match self.op_bom(asked, true) {
            Ok(bom) => no_task(json!(Exported {
                ok: true,
                bom: &bom
            })),
            Err(answered) => answered,
        }
    }

    /// `pid_bom_export` 进写操作的流程以前先看（§5.3）：没有图、请求写错了、按项目按节点算时
    /// 当前图没存过或节点号不认识（[`Self::op_bom_master`]）、要等扫描，就在这里应答，要扫描的
    /// 交出后台读的任务；能算了是 `None`。当前图、按快照算的不等，读快照的错在写操作里应答。
    pub(in crate::app) fn control_pid_bom_export_waits(
        &mut self,
        req: &Value,
    ) -> Option<(Value, Task<Message>)> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            let refusal = refuse("no_document", "Create or open a drawing first");
            return Some(no_task(refusal));
        }
        let dir = crate::io::drawing_dir(&self.tabs[i].scene.document);
        let range = match export_asked(req, dir) {
            Ok(ExportAsked {
                scope: Asked::Range(range),
                ..
            }) => range,
            Ok(_) => return None,
            Err(refusal) => return Some(no_task(refusal)),
        };
        if let Err(refusal) = self.op_bom_master(i, &range) {
            return Some(no_task(refusal));
        }
        self.op_scan(i, DEFAULT_TOLERANCE, true).err()
    }

    /// 控制面的 `pid_bom_export`，在写操作的流程里（§5.3）：同 `PIDBOM EXPORT`，照 `path` 的
    /// 扩展名写文件，`columns` 选主表的列；结果是写到哪里、什么格式、主表几行、主表文件几个
    /// 字节、副表写到哪里（JSON 没有副表文件，是 `null`）。扫描前面
    /// [`Self::control_pid_bom_export_waits`] 等过了，这里不读盘。写不了是 `BOM_IO_ERROR`。
    pub(in crate::app) fn control_pid_bom_export(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        let dir = crate::io::drawing_dir(&self.tabs[self.active_tab].scene.document);
        let ExportAsked {
            scope,
            export,
            columns,
        } = export_asked(req, dir)?;
        let bom = self.op_bom(scope, false).map_err(|(answer, _)| answer)?;
        let written =
            write_bom(&bom, &export, columns.as_deref()).map_err(|error| match error {
                ExportError::Columns(error) => refuse(error.code(), error.to_string()),
                ExportError::Io { path, reason } => {
                    refuse(BOM_IO_ERROR, format!("写不了 {}：{reason}", path.display()))
                }
            })?;
        let pipe_summary_path = written
            .pipe_summary
            .map(|(path, _)| path.display().to_string());
        self.set_control_result(json!({
            "path": written.path.display().to_string(),
            "format": written.format.extension(),
            "rows": written.rows,
            "bytes": written.bytes,
            "pipe_summary_path": pipe_summary_path,
        }));
        Ok(Task::none())
    }

    /// 两个操作算 `asked` 的清单：当前图现做，快照读出来算；按项目、按节点算先认范围
    /// （[`Self::op_bom_master`]），再扫项目（`read` 同 [`Self::op_scan`]），在扫好的项目上算。
    /// 算不出来是失败应答（[`failure_answer`]）。
    fn op_bom(&mut self, asked: Asked, read: bool) -> Result<Bom, (Value, Task<Message>)> {
        let i = self.active_tab;
        let outcome = match asked {
            Asked::Drawing => self.pidbom_compute(Sheets::Current(i)),
            Asked::Snapshot(seq) => self.pidbom_compute(Sheets::Snapshot(i, seq)),
            Asked::Range(range) => {
                let master = self.op_bom_master(i, &range).map_err(no_task)?;
                let (_, scanned) = self.op_scan(i, DEFAULT_TOLERANCE, read)?;
                let sheets = Sheets::Scanned {
                    scanned: &scanned,
                    range: &range,
                    master: &master,
                };
                self.pidbom_compute(sheets)
            }
        };
        outcome
            .map(|(bom, _)| bom)
            .map_err(|failure| no_task(failure_answer(&failure)))
    }

    /// 按项目、按节点算以前先看的，都在扫项目之前：当前图要存过盘（`INDEX_NO_PROJECT_DIR`），
    /// 按节点算时节点号要在主数据的 PBS 表里（`PBS_NODE_UNKNOWN`）。得到读出来的主数据。
    fn op_bom_master(&self, i: usize, range: &Range) -> Result<Resolved, Value> {
        let document = &self.tabs[i].scene.document;
        let Some(project_dir) = crate::io::drawing_dir(document) else {
            let why = "当前图没保存过，不属于哪个项目；存盘后再算";
            return Err(refuse(INDEX_NO_PROJECT_DIR, why));
        };
        let master = master_data::resolve(document, Some(project_dir));
        if let Range::Pbs(node) = range {
            pbs_known(&master, node).map_err(|why| refuse(PBS_NODE_UNKNOWN, why))?;
        }
        Ok(master)
    }
}

/// 扫好的项目里在范围内的图（§3.5）：全项目是读得出来的全部图，按节点是项目文件里归在这个
/// 节点及其下级的图，连同清单的范围。范围内有图读不出来是 `BOM_SCOPE_INCOMPLETE`。
fn in_scope<'a>(
    scanned: &'a Scanned,
    range: &Range,
    master: &Resolved,
) -> Result<(Cow<'a, [DrawingSummary]>, BomScope), BomFailure> {
    let nodes: Option<HashSet<&str>> = match (range, master) {
        (Range::Project, _) => None,
        (Range::Pbs(node), Resolved::Configured { data, .. }) => {
            Some(data.pbs_subtree(node).into_iter().collect())
        }
        (Range::Pbs(_), _) => Some(HashSet::new()),
    };
    let inside = |pbs: Option<&str>| match &nodes {
        None => true,
        Some(nodes) => pbs.is_some_and(|pbs| nodes.contains(pbs)),
    };
    let unreadable: Vec<IndexDrawing> = scanned
        .index
        .drawings
        .iter()
        .filter(|drawing| drawing.error.is_some() && inside(drawing.pbs.as_deref()))
        .cloned()
        .collect();
    if !unreadable.is_empty() {
        return Err(BomFailure::Incomplete(unreadable));
    }
    Ok(match range {
        Range::Project => (
            Cow::Borrowed(scanned.summaries.as_slice()),
            BomScope::project(),
        ),
        Range::Pbs(node) => {
            let picked = scanned
                .summaries
                .iter()
                .filter(|summary| inside(summary.pbs.as_deref()))
                .cloned()
                .collect();
            (Cow::Owned(picked), BomScope::pbs(node))
        }
    })
}

/// 说话打头的命令：导出的是 `PIDBOM EXPORT`，其余按范围。
fn verb_of(range: Option<&Range>, export: bool) -> &'static str {
    match (export, range) {
        (true, _) => "PIDBOM EXPORT",
        (false, None) => "PIDBOM",
        (false, Some(Range::Project)) => "PIDBOM PROJECT",
        (false, Some(Range::Pbs(_))) => "PIDBOM PBS",
    }
}

/// 清单算的是什么：「当前图，」「图 PID-1101，」「图 PID-1101 的第 1 份快照，」「全项目 2 张图，」
/// 「PBS 节点 U-100 1 张图，」。
fn scope_said(bom: &Bom, drawings: usize) -> String {
    let scope = &bom.scope;
    match scope.kind {
        ScopeKind::Drawing => {
            let drawing = drawing_said(scope.drawing_id.as_deref().unwrap_or_default());
            match bom.snapshot_seq {
                Some(seq) => format!("{drawing} 的第 {seq} 份快照，"),
                None => format!("{drawing}，"),
            }
        }
        ScopeKind::Pbs => {
            let node = scope.pbs.as_deref().unwrap_or_default();
            format!("PBS 节点 {node} {drawings} 张图，")
        }
        ScopeKind::Project => format!("全项目 {drawings} 张图，"),
    }
}

/// 一张图怎么称呼：「图 PID-1101」；没存过的图图号是空的，叫「当前图」。
fn drawing_said(drawing_id: &str) -> String {
    if drawing_id.is_empty() {
        "当前图".to_owned()
    } else {
        format!("图 {drawing_id}")
    }
}

/// 各种类几行，按行序的种类、没有的不写：「（管线 1，设备 2）」；一行也没有是空的。
fn kinds_said(by_kind: &ByKind) -> String {
    let kinds = [
        ("管线", by_kind.pipe_line),
        ("设备", by_kind.equipment),
        ("阀门", by_kind.valve),
        ("仪表", by_kind.instrument),
        ("管件", by_kind.fitting),
        ("其他", by_kind.other),
    ];
    let said: Vec<String> = kinds
        .iter()
        .filter(|(_, rows)| *rows > 0)
        .map(|(kind, rows)| format!("{kind} {rows}"))
        .collect();
    if said.is_empty() {
        String::new()
    } else {
        format!("（{}）", said.join("，"))
    }
}

/// 跳过的一个对象：种类、类型编码、在哪张图的哪里、句柄，和为什么跳过。
fn skipped_said(skipped: &Skipped) -> String {
    let reason = match skipped.reason {
        SkipReason::EmptyTag => "位号为空",
    };
    format!(
        "  跳过 {} {}：{} {}，句柄 {}，{reason}",
        kind_said(skipped.kind),
        skipped.type_code,
        drawing_said(&skipped.drawing_id),
        point(skipped.at),
        skipped.handle
    )
}

/// 种类的叫法（§3.2 的 `kind`）。
fn kind_said(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Equipment => "设备",
        ObjectKind::Valve => "阀门",
        ObjectKind::Instrument => "仪表",
        ObjectKind::Fitting => "管件",
        ObjectKind::Break => "打断符",
        ObjectKind::PipeLine => "管线",
        ObjectKind::PipeRun => "管路",
        ObjectKind::SignalLine => "信号线",
        ObjectKind::Other => "其他",
    }
}

/// 写好了：主表几行写成什么文件，副表几行写在哪。
fn written_said(verb: &str, written: &Written) -> String {
    let side = written
        .pipe_summary
        .as_ref()
        .map_or_else(String::new, |(path, rows)| {
            format!("，管道汇总（{rows} 行）写成 {}", path.display())
        });
    format!(
        "{verb}: 已把清单（{} 行）写成 {} 文件 {}{side}。",
        written.rows,
        written.format.name(),
        written.path.display()
    )
}

/// 把清单写成 `export` 的文件（§3.4）：主表按 `columns` 选列（`None` 是默认九列），CSV、
/// XLSX 另在同目录写副表（[`pipe_summary_path`]）；JSON 是整份清单带 `ok`，副表在里面。要写的
/// 都转好了再写，选列出错时一个文件也不写。目标文件已存在时覆盖。
fn write_bom(
    bom: &Bom,
    export: &Export,
    columns: Option<&[String]>,
) -> Result<Written, ExportError> {
    let path = &export.path;
    let pipe_summary = match export.format {
        Format::Json => {
            let text = serde_json::to_string_pretty(&Exported { ok: true, bom })
                .map_err(|error| io_error(path, &error))?;
            write_file(path, format!("{text}\n").as_bytes())?;
            None
        }
        Format::Csv | Format::Xlsx => {
            let table = bom::rows(bom, columns).map_err(ExportError::Columns)?;
            let summary = bom::pipe_summary_rows(bom);
            let side = pipe_summary_path(path);
            if export.format == Format::Csv {
                write_file(path, &bom::csv_bytes(&table))?;
                write_file(&side, &bom::csv_bytes(&summary))?;
            } else {
                write_sheet(path, &table)?;
                write_sheet(&side, &summary)?;
            }
            Some((side, bom.pipe_summary.len()))
        }
    };
    let bytes = std::fs::metadata(path)
        .map_err(|error| io_error(path, &error))?
        .len();
    Ok(Written {
        path: path.clone(),
        format: export.format,
        rows: bom.rows.len(),
        bytes,
        pipe_summary,
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    std::fs::write(path, bytes).map_err(|error| io_error(path, &error))
}

fn write_sheet(path: &Path, rows: &[Vec<String>]) -> Result<(), ExportError> {
    write_extraction_file(path, rows).map_err(|reason| ExportError::Io {
        path: path.to_path_buf(),
        reason,
    })
}

fn io_error(path: &Path, error: &dyn fmt::Display) -> ExportError {
    ExportError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// 副表的文件：同目录的 `<文件名>-pipe-summary.<扩展名>`，扩展名照主表的写法。
fn pipe_summary_path(path: &Path) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push(PIPE_SUMMARY_SUFFIX);
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    path.with_file_name(name)
}

/// 要写的文件（§5.2）：不是完整路径时从 `dir`（图所在的目录）算起，没有 `dir`（图没存过）时
/// 从当前目录算起。扩展名不支持、是项目文件时是 `Err`：错误码（`BOM_FORMAT_UNSUPPORTED`；项目
/// 文件没有码）和原因。
fn export_target(dir: Option<&Path>, name: &str) -> Result<Export, (Option<&'static str>, String)> {
    let path = match dir {
        Some(dir) if Path::new(name).is_relative() => dir.join(name),
        _ => PathBuf::from(name),
    };
    let Some(format) = Format::of(&path) else {
        let why = format!(
            "{BOM_FORMAT_UNSUPPORTED}: 只写 .csv、.xlsx、.json 文件，{} 不是。",
            path.display()
        );
        return Err((Some(BOM_FORMAT_UNSUPPORTED), why));
    };
    if path
        .file_name()
        .is_some_and(|file| file.eq_ignore_ascii_case(PROJECT_FILE))
    {
        let why = format!("{} 是项目文件，不拿清单盖掉它。", path.display());
        return Err((None, why));
    }
    Ok(Export { path, format })
}

/// 读 `pid_bom`、`pid_bom_export` 请求里的范围（§5.3）。`scope` 不分大小写，默认 `drawing`；
/// `scope: pbs` 时 `pbs` 必给，没给或是空的报 `SCOPE_NOT_SELECTED`；别的范围不收 `pbs`。按快照算
/// 给 `snapshot`：快照序号，写 JSON 的非负整数，只和 `drawing` 一起（快照是一张图的）；序号在
/// 不在，读的时候才知道。写得不对是 `invalid_request`。
fn scope_asked(req: &Value) -> Result<Asked, Value> {
    let wrong = || refuse("invalid_request", "snapshot 写快照的序号：非负整数");
    let snapshot = match &req["snapshot"] {
        Value::Null => None,
        written => Some(written.as_u64().ok_or_else(wrong)?),
    };
    let scope = match &req["scope"] {
        Value::Null => "drawing".to_owned(),
        Value::String(scope) => scope.trim().to_ascii_lowercase(),
        _ => {
            return Err(refuse(
                "invalid_request",
                "scope 写 drawing、project 或 pbs",
            ))
        }
    };
    let pbs = match &req["pbs"] {
        Value::Null => None,
        Value::String(node) => Some(node.trim()).filter(|node| !node.is_empty()),
        _ => return Err(refuse("invalid_request", "pbs 写 PBS 节点号")),
    };
    match (scope.as_str(), pbs, snapshot) {
        ("drawing", None, None) => Ok(Asked::Drawing),
        ("drawing", None, Some(seq)) => Ok(Asked::Snapshot(seq)),
        ("project", None, None) => Ok(Asked::Range(Range::Project)),
        ("pbs", Some(node), None) => Ok(Asked::Range(Range::Pbs(node.to_owned()))),
        ("project" | "pbs", _, Some(_)) => Err(refuse(
            "invalid_request",
            "snapshot 只和 scope: drawing 一起给：快照是一张图的",
        )),
        ("pbs", None, None) => Err(refuse(
            SCOPE_NOT_SELECTED,
            "scope 是 pbs 时要给 pbs（PBS 节点号）",
        )),
        ("drawing" | "project", Some(_), _) => {
            Err(refuse("invalid_request", "pbs 只和 scope: pbs 一起给"))
        }
        (other, _, _) => Err(refuse(
            "invalid_request",
            format!("scope 写 drawing、project 或 pbs，不认识 {other}"),
        )),
    }
}

/// `pid_bom_export` 的请求（§5.3）：范围同 [`scope_asked`]（含 `snapshot`）；`path` 必给，文件同
/// [`export_target`]，项目文件是 `invalid_request`；`columns` 写列名的数组，不给是默认九列。
fn export_asked(req: &Value, dir: Option<&Path>) -> Result<ExportAsked, Value> {
    let scope = scope_asked(req)?;
    let name = match &req["path"] {
        Value::String(name) if !name.trim().is_empty() => name,
        _ => return Err(refuse("invalid_request", "要给 path：写到哪个文件")),
    };
    let wrong = || refuse("invalid_request", "columns 写列名的数组");
    let columns = match &req["columns"] {
        Value::Null => None,
        Value::Array(names) => Some(
            names
                .iter()
                .map(|name| name.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
                .ok_or_else(wrong)?,
        ),
        _ => return Err(wrong()),
    };
    let export = export_target(dir, name)
        .map_err(|(code, why)| refuse(code.unwrap_or("invalid_request"), why))?;
    Ok(ExportAsked {
        scope,
        export,
        columns,
    })
}

/// 算不出清单时的失败应答（§3.5）：错误码和原因；重复的键带上 `duplicates`（各在哪张图的
/// 哪里），读不出来的图带上 `drawings`（文件和原因）。
fn failure_answer(failure: &BomFailure) -> Value {
    let mut answer = refuse(failure.code(), failure.to_string());
    match failure {
        BomFailure::Bom(BomError::DuplicateTag(duplicates)) => {
            answer["duplicates"] = json!(duplicates);
        }
        BomFailure::Incomplete(drawings) => answer["drawings"] = json!(drawings),
        BomFailure::Index(_)
        | BomFailure::Bom(_)
        | BomFailure::NoProjectDir
        | BomFailure::Snapshot(_) => {}
    }
    answer
}

/// 控制面的失败应答。
fn refuse(code: &str, error: impl Into<String>) -> Value {
    json!({ "ok": false, "status": "failed", "code": code, "error": error.into() })
}

/// 不带后台任务的应答。
fn no_task(answer: Value) -> (Value, Task<Message>) {
    (answer, Task::none())
}

/// `text` 的第一个词，和它后面剩下的部分。
fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}

/// `EXPORT` 后面的文件名（有空格时加双引号，引号里的原样取）和剩下的部分。没写文件名、
/// 引号没配对是 `None`。
fn file_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let (name, rest) = match text.strip_prefix('"') {
        Some(quoted) => {
            let end = quoted.find('"')?;
            (&quoted[..end], quoted[end + 1..].trim())
        }
        None => first_word(text),
    };
    (!name.is_empty()).then_some((name, rest))
}

/// `PIDBOM SNAPSHOT` 后面的写法：`<序号> [EXPORT <文件>]`，序号全是数字，文件名同 [`file_word`]。
/// 得到序号和要写的文件名；写得不对是 `None`。
fn snapshot_words(text: &str) -> Option<(u64, Option<&str>)> {
    let (seq, rest) = first_word(text);
    if seq.is_empty() || !seq.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seq = seq.parse().ok()?;
    let (option, after) = first_word(rest);
    if option.is_empty() {
        return Some((seq, None));
    }
    if !option.eq_ignore_ascii_case("EXPORT") {
        return None;
    }
    match file_word(after) {
        Some((name, "")) => Some((seq, Some(name))),
        _ => None,
    }
}

/// 现在的 Unix 秒，清单的 `generated_at`（跟检查结果的 `checked_at` 一样算）。
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::bom::PIPE_LINE_ROW;
    use crate::pid::master_data::{MasterData, PbsNode};
    use crate::pid::persist::PID_AUTHOR_APP;
    use crate::pid::project_index::{SummaryMeta, SummaryObject, SummarySource};
    use crate::pid::snapshot::{store, SNAPSHOT_CORRUPT, SNAPSHOT_NOT_FOUND};
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 测试用的临时目录，用完删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：同一纳秒里建的两个靠计数分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "ocs-pidbom-{}-{}-{nanos}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("临时目录");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 一行命令说了什么，报了什么错：`error` 是那行错，`code` 是它的错误码，没报错都是
    /// `None`。
    struct Ran {
        said: String,
        error: Option<String>,
        code: Option<String>,
    }

    /// 像在命令行上敲进去那样跑一行，它在后台起的任务也一直跑到完。
    fn run(app: &mut OpenCADStudio, line: &str) -> Ran {
        app.command_line.history.clear();
        app.command_line.last_error = None;
        app.command_line.last_error_code = None;
        let task = app.run_command_line(line);
        app.drive_headless_task(task).expect("命令的任务");
        Ran {
            said: app
                .command_line
                .history
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            error: app.command_line.last_error.clone(),
            code: app.command_line.last_error_code.clone(),
        }
    }

    fn undo_steps(app: &OpenCADStudio) -> usize {
        app.tabs[app.active_tab].history.undo_stack.len()
    }

    /// 协议 1 的一条控制请求，发给当前的图。
    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        req["request_id"] = json!(format!("pidbom-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
        app.automation_op(&req.to_string())
    }

    /// 起 `cmd`，依次喂点，最后回车。
    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
        let started = request(app, json!({"op": "run", "cmd": cmd}));
        assert_eq!(started["ok"], true, "{cmd}：{started}");
        for [x, y] in points {
            let point =
                json!({"op": "input", "kind": "point", "point": [x, y, 0.0], "space": "wcs"});
            let fed = request(app, point);
            assert_eq!(fed["ok"], true, "{cmd} 喂点：{fed}");
        }
        request(app, json!({"op": "input", "kind": "enter"}));
    }

    /// 把当前图存成 `path`。
    fn save(app: &OpenCADStudio, path: &Path) {
        let bytes = crate::io::save_to_bytes(
            app.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("存 DWG");
        fs::write(path, bytes).expect("写文件");
    }

    /// 当前图上种类是 `kind` 的对象，按摘要里的先后。
    fn objects_of(app: &OpenCADStudio, kind: ObjectKind) -> Vec<SummaryObject> {
        let meta = SummaryMeta {
            drawing_id: String::new(),
            file: None,
            pbs: None,
            source: SummarySource::Open,
            fingerprint: None,
        };
        let document = app.active_document_for_test();
        let summary = summarize(document, &app.pid_symbols, meta, DEFAULT_TOLERANCE);
        let objects = summary.objects.into_iter();
        objects.filter(|object| object.kind == kind).collect()
    }

    /// 用命令建的一张图（§7）：一台泵左边一个不在线上的阀门，一台容器；管线 L-1001 自己一根
    /// 线，管路 L-1001-01 从泵的出口到容器的入口，上面一个阀门、一个不单独编号的管件；两台
    /// 仪表，前一台手填位号 PT-1001，后一台的位号是空的（仪表没有 `仪表功能` 属性编不出位号，
    /// 这里照样清一次）。管线的管道等级是 A1A，管路的公称直径是 50。
    fn plant(app: &mut OpenCADStudio) {
        draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(app, "PIDINSERT gate_valve", &[[-28.0, 0.0]]);
        draw(app, "PIDINSERT vessel", &[[200.0, 12.0]]);
        draw(app, "PIDPIPE", &[[200.0, 100.0], [300.0, 100.0]]);
        draw(app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
        draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
        draw(app, "PIDFITTING transmitter", &[[40.0, 0.0]]);
        draw(app, "PIDINSERT transmitter", &[[58.0, 80.0]]);
        draw(app, "PIDINSERT transmitter", &[[120.0, 80.0]]);
        for line in [
            "PIDSETPROP L-1001 管道等级 A1A",
            "PIDSETPROP L-1001-01 公称直径 50",
        ] {
            let ran = run(app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        let instruments = objects_of(app, ObjectKind::Instrument);
        assert_eq!(instruments.len(), 2, "{instruments:#?}");
        let tagged = ["tag=PT-1001", "tag_source=manual", "tag_status=ok"];
        let marks = [
            (&instruments[0], &tagged[..]),
            (&instruments[1], &["tag="][..]),
        ];
        for (instrument, values) in marks {
            let data: Vec<Value> = values
                .iter()
                .map(|value| json!({"code": 1000, "value": value}))
                .collect();
            let handles = [&instrument.handle];
            let set =
                json!({"op": "xdata_set", "handles": handles, "app": PID_AUTHOR_APP, "data": data});
            let done = request(app, set);
            assert_eq!(done["ok"], true, "{done}");
        }
    }

    /// 当前图画成项目里的一张小图：一台泵，一根管线（位号自动编，两张图上编出来的一样）。
    fn sheet(app: &mut OpenCADStudio) {
        draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
    }

    /// 临时目录里两张图，各一台泵、一根管线 L-1001。PID-1102 只在磁盘上，给了 `other_pump`
    /// 就把它的泵改成这个位号，不给两张图上的泵同位号；当前图是 PID-1101，存过盘、打开着。
    fn project(scratch: &Scratch, other_pump: Option<&str>) -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        sheet(&mut app);
        if let Some(tag) = other_pump {
            let pumps = objects_of(&app, ObjectKind::Equipment);
            let ran = run(&mut app, &format!("PIDTAG {} {tag}", pumps[0].tag));
            assert_eq!(ran.error, None, "{}", ran.said);
        }
        save(&app, &scratch.path("PID-1102.dwg"));
        let _ = app.automation_op(r#"{"op":"new"}"#);
        sheet(&mut app);
        let current = scratch.path("PID-1101.dwg");
        save(&app, &current);
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(current.to_string_lossy().into_owned());
        app
    }

    /// 项目文件：主数据的 PBS 表是 U-100（下有 U-110）和 U-200；PID-1101 归 U-110，
    /// PID-1102 归 U-200。
    fn write_project_file(scratch: &Scratch) {
        let node = |id: &str, parent: Option<&str>| PbsNode {
            id: id.to_owned(),
            label: id.to_owned(),
            parent: parent.map(str::to_owned),
        };
        let data = MasterData {
            version: 1,
            revision: 1,
            updated_at: 0,
            legend_standard: None,
            nominal_diameters: Vec::new(),
            pipe_classes: Vec::new(),
            insulation_classes: Vec::new(),
            fluid_codes: Vec::new(),
            pbs: vec![
                node("U-100", None),
                node("U-110", Some("U-100")),
                node("U-200", None),
            ],
        };
        let file = json!({
            "version": 1,
            "master_data": data.to_json(),
            "drawings": [
                { "file": "PID-1101.dwg", "drawing_id": "PID-1101", "pbs": "U-110" },
                { "file": "PID-1102.dwg", "drawing_id": "PID-1102", "pbs": "U-200" }
            ]
        });
        fs::write(scratch.path(PROJECT_FILE), file.to_string()).expect("项目文件");
    }

    /// 读回导出的 JSON：带 `ok`，其余是一份清单。
    fn read_back(path: &Path) -> Bom {
        let text = fs::read_to_string(path).expect("读回 JSON");
        let value: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["ok"], true, "{text}");
        serde_json::from_value(value).expect("一份清单")
    }

    /// 读回 XLSX 第一个工作表的格子，都按文本。
    fn sheet_rows(path: &Path) -> Vec<Vec<String>> {
        use calamine::{open_workbook_auto_from_rs, Reader};
        let bytes = fs::read(path).expect("XLSX 文件");
        let mut workbook =
            open_workbook_auto_from_rs(std::io::Cursor::new(bytes)).expect("读回 XLSX");
        let sheet = workbook.sheet_names().first().cloned().expect("工作表");
        let cells = workbook.worksheet_range(&sheet).expect("格子");
        let rows = cells.rows();
        rows.map(|row| row.iter().map(ToString::to_string).collect())
            .collect()
    }

    /// `PIDBOM`（§5.2）：各种类几行、管长合计、跳过几个对象与入口函数算的一样，种类按行序说；
    /// 逐个列出跳过的，逐条说提示。算清单不改图：前后图的「已修改」、撤销栈、编辑修订号都不变。
    #[test]
    fn pidbom_says_the_rows_by_kind_the_pipe_length_and_the_skipped() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let i = app.active_tab;
        let dirty = app.tabs[i].dirty;
        let steps = undo_steps(&app);
        let revision = app.tabs[i].edit_revision;

        let ran = run(&mut app, "PIDBOM");
        assert_eq!(ran.error, None, "{}", ran.said);
        let (bom, drawings) = app.pidbom_compute(Sheets::Current(i)).expect("清单");
        assert_eq!(drawings, 1);
        let by_kind = &bom.totals.by_kind;
        let counts = (
            by_kind.pipe_line,
            by_kind.equipment,
            by_kind.valve,
            by_kind.instrument,
            by_kind.fitting,
            by_kind.other,
        );
        assert_eq!(counts, (1, 2, 2, 1, 1, 0), "{:#?}", bom.rows);
        assert_eq!(bom.skipped.len(), 1, "{:#?}", bom.skipped);
        let skipped = &bom.skipped[0];
        assert_eq!(skipped.kind, ObjectKind::Instrument);
        let length: f64 = bom
            .rows
            .iter()
            .filter_map(|row| match row.quantity {
                Quantity::Length(metres) => Some(metres),
                Quantity::Count(_) => None,
            })
            .sum();
        assert!(length > 0.0, "{:#?}", bom.rows);
        let said = format!(
            "PIDBOM: 当前图，7 行（管线 1，设备 2，阀门 2，仪表 1，管件 1），管长合计 {length:.2} m，跳过 1 个对象。"
        );
        assert!(ran.said.contains(&said), "{}", ran.said);
        let place = format!("  跳过 仪表 {}：当前图 (", skipped.type_code);
        let handle = format!("，句柄 {}，位号为空", skipped.handle);
        assert!(ran.said.contains(&place), "{}", ran.said);
        assert!(ran.said.contains(&handle), "{}", ran.said);
        let codes: Vec<&str> = bom
            .notices
            .iter()
            .map(|notice| notice.code.as_str())
            .collect();
        assert!(codes.contains(&bom::LENGTH_FROM_DRAWING), "{codes:?}");
        for notice in &bom.notices {
            let said = format!("PIDBOM: {}: {}", notice.code, notice.message);
            assert!(ran.said.contains(&said), "{}", ran.said);
        }
        assert_eq!(app.tabs[i].dirty, dirty);
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(app.tabs[i].edit_revision, revision);
    }

    /// 项目范围（§6 第 7 条）：行数是两张图的对象归并后的——两张图上同一根管线 L-1001 合成
    /// 一行、两张图都列上，两台泵位号不同各一行，3 行而不是 4 行。按节点算只算项目文件里归在
    /// 节点及其下级的图。
    #[test]
    fn pidbom_project_merges_the_drawings_and_pbs_takes_the_drawings_under_the_node() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));

        let ran = run(&mut app, "PIDBOM");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = "PIDBOM: 图 PID-1101，2 行（管线 1，设备 1），";
        assert!(ran.said.contains(said), "{}", ran.said);

        let ran = run(&mut app, "PIDBOM PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(
            ran.said.contains("PIDBOM: 要从磁盘读 1 个文件"),
            "{}",
            ran.said
        );
        let said = "PIDBOM PROJECT: 全项目 2 张图，3 行（管线 1，设备 2），";
        assert!(ran.said.contains(said), "{}", ran.said);

        let ran = run(&mut app, "PIDBOM EXPORT project.json PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        let bom = read_back(&scratch.path("project.json"));
        assert_eq!(bom.scope, BomScope::project());
        let line = bom.rows.iter().find(|row| row.type_code == PIPE_LINE_ROW);
        let line = line.expect("管线行");
        assert_eq!(line.instances, 2, "{line:#?}");
        assert_eq!(line.drawings, ["PID-1101", "PID-1102"]);
        let pumps: Vec<(&str, &[String])> = bom
            .rows
            .iter()
            .filter(|row| row.kind == ObjectKind::Equipment)
            .map(|row| (row.tag.as_str(), row.drawings.as_slice()))
            .collect();
        assert_eq!(pumps.len(), 2, "{pumps:?}");
        assert_eq!(pumps[1], ("P-2001", &["PID-1102".to_owned()][..]));

        write_project_file(&scratch);
        for (node, drawing) in [("U-100", "PID-1101"), ("U-200", "PID-1102")] {
            let line = format!("pidbom pbs {node}");
            let ran = run(&mut app, &line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            let said = format!("PIDBOM PBS: PBS 节点 {node} 1 张图，2 行（管线 1，设备 1），");
            assert!(ran.said.contains(&said), "{line}: {}", ran.said);

            let file = format!("{node}.json");
            let ran = run(&mut app, &format!("PIDBOM EXPORT {file} PBS {node}"));
            assert_eq!(ran.error, None, "{}", ran.said);
            let bom = read_back(&scratch.path(&file));
            assert_eq!(bom.scope, BomScope::pbs(node));
            assert!(
                bom.rows.iter().all(|row| row.drawings == [drawing]),
                "{:#?}",
                bom.rows
            );
        }
    }

    /// 三种格式（§3.4、§6 第 8 条）：CSV 与 `csv_bytes` 逐字节相同、以 BOM 头开始，副表另一个
    /// 文件；XLSX 用 calamine 读回，主表、副表的格子与转出的表相同；JSON 带 `ok`、缩进 2 格，
    /// 解析回同一份清单，副表在里面、不另写文件。目标文件已存在时覆盖。选列照 `bom::rows`；
    /// 有不认识的列一个文件也不写。
    #[test]
    fn pidbom_export_writes_the_three_formats_and_they_read_back_as_the_bom() {
        let scratch = Scratch::new();
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let current = scratch.path("PID-1101.dwg");
        save(&app, &current);
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(current.to_string_lossy().into_owned());
        let (bom, _) = app.pidbom_compute(Sheets::Current(i)).expect("清单");
        assert_eq!(bom.pipe_summary.len(), 2, "{:#?}", bom.pipe_summary);
        let table = bom::rows(&bom, None).expect("主表");
        let summary = bom::pipe_summary_rows(&bom);

        let main = scratch.path("bom.csv");
        let side = scratch.path("bom-pipe-summary.csv");
        fs::write(&main, "旧的").expect("旧文件");
        let ran = run(&mut app, "PIDBOM EXPORT bom.csv");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(
            ran.said.contains("PIDBOM EXPORT: 图 PID-1101，7 行"),
            "{}",
            ran.said
        );
        let said = format!(
            "PIDBOM EXPORT: 已把清单（7 行）写成 CSV 文件 {}，管道汇总（2 行）写成 {}。",
            main.display(),
            side.display()
        );
        assert!(ran.said.contains(&said), "{}", ran.said);
        let bytes = fs::read(&main).expect("CSV");
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(bytes, bom::csv_bytes(&table));
        assert_eq!(fs::read(&side).expect("副表 CSV"), bom::csv_bytes(&summary));

        let ran = run(&mut app, r#"PIDBOM EXPORT "bom out.XLSX""#);
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(sheet_rows(&scratch.path("bom out.XLSX")), table);
        assert_eq!(
            sheet_rows(&scratch.path("bom out-pipe-summary.XLSX")),
            summary
        );

        let ran = run(&mut app, "PIDBOM EXPORT bom.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let text = fs::read_to_string(scratch.path("bom.json")).expect("JSON");
        assert!(
            text.starts_with("{\n  \"ok\": true,\n  \"version\": 1,\n"),
            "{text}"
        );
        assert!(text.ends_with("}\n"), "{text}");
        let read = read_back(&scratch.path("bom.json"));
        let expected = Bom {
            generated_at: read.generated_at,
            ..bom.clone()
        };
        assert_eq!(read, expected);
        assert!(!scratch.path("bom-pipe-summary.json").exists());

        let columns = ["位号".to_owned(), "数量".to_owned()];
        let picked = Export {
            path: scratch.path("picked.csv"),
            format: Format::Csv,
        };
        let written = write_bom(&bom, &picked, Some(&columns)).expect("选列导出");
        assert_eq!(written.rows, 7);
        let expected = bom::rows(&bom, Some(&columns)).expect("选列");
        assert_eq!(
            fs::read(&picked.path).expect("选列 CSV"),
            bom::csv_bytes(&expected)
        );
        let unknown = ["重量".to_owned()];
        let refused = Export {
            path: scratch.path("refused.xlsx"),
            format: Format::Xlsx,
        };
        let error = write_bom(&bom, &refused, Some(&unknown));
        let columns_unknown =
            matches!(error, Err(ExportError::Columns(BomError::ColumnUnknown(_))));
        assert!(columns_unknown, "{error:?}");
        assert!(!refused.path.exists());
        assert!(!scratch.path("refused-pipe-summary.xlsx").exists());
    }

    /// 有错就不写文件（§3.4、§3.5）：两张图上同类型同位号的泵报 `BOM_DUPLICATE_TAG`，逐个说
    /// 各在哪张图的哪里，三种格式都不写；扩展名不支持报 `BOM_FORMAT_UNSUPPORTED`，这时还没算
    /// 清单；不拿清单盖项目文件。
    #[test]
    fn pidbom_export_writes_nothing_when_it_cannot() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, None);
        let pumps = objects_of(&app, ObjectKind::Equipment);
        let key = format!("  {} {}：图 PID-1101 (", pumps[0].type_code, pumps[0].tag);
        for line in [
            "PIDBOM PROJECT",
            "PIDBOM EXPORT dup.csv PROJECT",
            "PIDBOM EXPORT dup.xlsx PROJECT",
            "PIDBOM EXPORT dup.json PROJECT",
        ] {
            let ran = run(&mut app, line);
            let code = ran.code.as_deref();
            assert_eq!(code, Some(bom::BOM_DUPLICATE_TAG), "{line}: {}", ran.said);
            assert!(ran.said.contains(&key), "{line}: {}", ran.said);
            assert!(ran.said.contains("、图 PID-1102 ("), "{line}: {}", ran.said);
        }
        for name in [
            "dup.csv",
            "dup-pipe-summary.csv",
            "dup.xlsx",
            "dup-pipe-summary.xlsx",
            "dup.json",
        ] {
            assert!(!scratch.path(name).exists(), "{name}");
        }
        for (line, name) in [
            ("PIDBOM EXPORT bom.txt", "bom.txt"),
            ("PIDBOM EXPORT bom", "bom"),
            ("PIDBOM EXPORT bom.pdf PROJECT", "bom.pdf"),
        ] {
            let ran = run(&mut app, line);
            let code = ran.code.as_deref();
            assert_eq!(code, Some(BOM_FORMAT_UNSUPPORTED), "{line}: {}", ran.said);
            assert!(!ran.said.contains("图 PID-1101，"), "{line}: {}", ran.said);
            assert!(!scratch.path(name).exists(), "{name}");
        }
        let ran = run(&mut app, &format!("PIDBOM EXPORT {PROJECT_FILE}"));
        let error = ran.error.unwrap_or_default();
        assert!(error.contains("是项目文件"), "{error}");
        assert!(!scratch.path(PROJECT_FILE).exists());
    }

    /// 范围出错时说为什么（§3.5）：按节点算不给节点号是 `SCOPE_NOT_SELECTED`；没存过的图
    /// 不属于哪个项目（`INDEX_NO_PROJECT_DIR`）；主数据里没有的节点是 `PBS_NODE_UNKNOWN`；
    /// 范围内有图读不出来是 `BOM_SCOPE_INCOMPLETE`，说是哪张，不出清单也不写文件——范围外
    /// 读不出来的不管。
    #[test]
    fn pidbom_says_why_it_cannot_take_the_scope() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for line in ["PIDBOM PBS", "pidbom pbs", "PIDBOM EXPORT bom.csv PBS"] {
            let ran = run(&mut app, line);
            let code = ran.code.as_deref();
            assert_eq!(code, Some(SCOPE_NOT_SELECTED), "{line}: {}", ran.said);
        }
        for line in [
            "PIDBOM PROJECT",
            "PIDBOM PBS U-100",
            "PIDBOM EXPORT bom.csv PROJECT",
        ] {
            let ran = run(&mut app, line);
            let code = ran.code.as_deref();
            assert_eq!(code, Some(INDEX_NO_PROJECT_DIR), "{line}: {}", ran.said);
        }

        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));
        let ran = run(&mut app, "PIDBOM PBS U-100");
        assert_eq!(ran.code.as_deref(), Some(PBS_NODE_UNKNOWN), "{}", ran.said);
        assert!(ran.said.contains("没有配置主数据"), "{}", ran.said);
        write_project_file(&scratch);
        let ran = run(&mut app, "PIDBOM EXPORT bom.csv PBS U-999");
        assert_eq!(ran.code.as_deref(), Some(PBS_NODE_UNKNOWN), "{}", ran.said);
        assert!(!scratch.path("bom.csv").exists());

        fs::write(scratch.path("PID-1103.dwg"), "不是图纸").expect("坏文件");
        for line in ["PIDBOM PROJECT", "PIDBOM EXPORT all.json PROJECT"] {
            let ran = run(&mut app, line);
            let code = ran.code.as_deref();
            assert_eq!(code, Some(BOM_SCOPE_INCOMPLETE), "{line}: {}", ran.said);
            assert!(
                ran.said.contains("  PID-1103（PID-1103.dwg）："),
                "{line}: {}",
                ran.said
            );
        }
        assert!(!scratch.path("all.json").exists());
        let ran = run(&mut app, "PIDBOM PBS U-100");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = "PIDBOM PBS: PBS 节点 U-100 1 张图，";
        assert!(ran.said.contains(said), "{}", ran.said);
    }

    /// 只读会话（§5.4）：几种写法都不拒，照常算、照常导出（写的是本机文件，不是图纸），
    /// 撤销栈不变。
    #[test]
    fn pidbom_runs_and_exports_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));
        write_project_file(&scratch);
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for (line, said) in [
            ("PIDBOM", "PIDBOM: 图 PID-1101，"),
            ("PIDBOM PROJECT", "PIDBOM PROJECT: 全项目 2 张图，"),
            ("pidbom pbs U-100", "PIDBOM PBS: PBS 节点 U-100 1 张图，"),
            ("PIDBOM EXPORT bom.csv", "PIDBOM EXPORT: 已把清单"),
            ("PIDBOM EXPORT bom.xlsx PROJECT", "PIDBOM EXPORT: 已把清单"),
            (
                "PIDBOM EXPORT bom.json PBS U-200",
                "PIDBOM EXPORT: 已把清单",
            ),
        ] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert!(ran.said.contains(said), "{line}: {}", ran.said);
        }
        assert_eq!(undo_steps(&app), steps);
        for name in [
            "bom.csv",
            "bom-pipe-summary.csv",
            "bom.xlsx",
            "bom-pipe-summary.xlsx",
            "bom.json",
        ] {
            assert!(scratch.path(name).exists(), "{name}");
        }
    }

    /// 登记了；写错了说怎么写，不带错误码（不是 §3.5 的那几种错）。
    #[test]
    fn pidbom_is_registered_and_says_how_it_is_written() {
        assert!(crate::command::all_registered_command_names().contains(&"PIDBOM"));
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for line in [
            "PIDBOM FROB",
            "PIDBOM PROJECT NOW",
            "pidbom pbs U-100 U-200",
            "PIDBOM EXPORT",
            r#"PIDBOM EXPORT "bom.csv"#,
            "PIDBOM EXPORT bom.csv FROB",
            "PIDBOM EXPORT bom.csv PROJECT NOW",
            "PIDBOM EXPORT bom.csv PBS U-100 U-200",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code, None, "{line}: {}", ran.said);
            let error = ran.error.unwrap_or_default();
            assert!(error.starts_with("PIDBOM"), "{line}: {error}");
            assert!(error.contains("写成"), "{line}: {error}");
        }
    }

    /// 命令行上的 `PIDBOM PROJECT` 整行是一条命令：只按项目算一次，不先拿裸动词算一遍当前图。
    #[test]
    fn pidbom_project_from_the_command_line_runs_once() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));
        let ran = run(&mut app, "PIDBOM PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = "PIDBOM PROJECT: 全项目 2 张图，";
        assert_eq!(ran.said.matches(said).count(), 1, "{}", ran.said);
        assert!(!ran.said.contains("PIDBOM: 图 PID-1101"), "{}", ran.said);
    }

    /// 发 `ask`，应答 `scanning` 就再发：测试里后台读随应答跑完，再发就是结果。
    fn settled(app: &mut OpenCADStudio, ask: &Value) -> Value {
        for _ in 0..5 {
            let answer = request(app, ask.clone());
            if answer["status"] != "scanning" {
                return answer;
            }
        }
        panic!("{ask}：一直在扫");
    }

    /// 控制面的 `pid_bom`（§5.3）：应答是 §3.2 的整份清单带 `ok`，与入口函数在同一张图上算的
    /// 一样；`scope` 不分大小写，不写是当前图。是查询：不带 `request_id` 也能发，不改图，不留
    /// 撤销步。能力清单里有 `pid_bom`。
    #[test]
    fn pid_bom_op_answers_the_bom_the_command_computes() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let i = app.active_tab;
        let steps = undo_steps(&app);
        let dirty = app.tabs[i].dirty;
        let revision = app.tabs[i].edit_revision;
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        assert!(
            state["capabilities"]
                .as_array()
                .is_some_and(|names| names.contains(&json!("pid_bom"))),
            "{state}"
        );

        let answer = app.automation_op(r#"{"protocol":1,"op":"pid_bom"}"#);
        assert_eq!(answer["ok"], true, "{answer}");
        let (bom, _) = app.pidbom_compute(Sheets::Current(i)).expect("清单");
        assert_eq!(bom.totals.rows, 7, "{bom:#?}");
        let mut answered: Bom = serde_json::from_value(answer.clone()).expect("一份清单");
        answered.generated_at = bom.generated_at;
        assert_eq!(answered, bom);
        let scoped = request(&mut app, json!({"op": "pid_bom", "scope": "DRAWING"}));
        assert_eq!(scoped["rows"], answer["rows"], "{scoped}");
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(app.tabs[i].dirty, dirty);
        assert_eq!(app.tabs[i].edit_revision, revision);
    }

    /// 按项目、按节点（§5.3）：要从磁盘读别的图时先应答 `scanning`（读了几个、共几个），扫着时
    /// 再发还是 `scanning`、不再起一次扫描；读完同样的请求再发才是清单。全项目 3 行；按节点只
    /// 算归在节点及其下级的图。
    #[test]
    fn pid_bom_op_answers_scanning_then_the_project_and_the_node() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));
        write_project_file(&scratch);
        let ask = json!({"protocol": 1, "op": "pid_bom", "scope": "Project"});
        let (answer, task) = app.control_request(ask.clone());
        assert_eq!(
            answer,
            json!({"ok": true, "status": "scanning", "done": 0, "total": 1})
        );
        let (again, none) = app.control_request(ask.clone());
        assert_eq!(again, answer);
        assert!(
            iced_runtime::task::into_stream(none).is_none(),
            "正在扫又起了一次"
        );
        app.drive_headless_task(task).expect("后台读");
        let (answer, _) = app.control_request(ask);
        assert_eq!(answer["ok"], true, "{answer}");
        let scope = json!({"kind": "project", "drawing_id": null, "pbs": null});
        assert_eq!(answer["scope"], scope, "{answer}");
        assert_eq!(answer["totals"]["rows"], 3, "{answer}");

        let ask = json!({"op": "pid_bom", "scope": "pbs", "pbs": "U-100"});
        let node = request(&mut app, ask);
        assert_eq!(node["ok"], true, "{node}");
        let scope = json!({"kind": "pbs", "drawing_id": null, "pbs": "U-100"});
        assert_eq!(node["scope"], scope, "{node}");
        assert_eq!(node["totals"]["rows"], 2, "{node}");
    }

    /// `pid_bom` 算不出清单时（§3.5）：请求写错是 `invalid_request`（范围不认识、不按节点算却
    /// 给了节点号）；按节点算没给节点号 `SCOPE_NOT_SELECTED`；没存过的图要按项目、按节点、
    /// 按快照算 `INDEX_NO_PROJECT_DIR`；节点号不认识 `PBS_NODE_UNKNOWN`；
    /// 重复位号 `BOM_DUPLICATE_TAG`，带 `duplicates`；范围内有图读不出来
    /// `BOM_SCOPE_INCOMPLETE`，带 `drawings`，范围外的不管。
    #[test]
    fn pid_bom_op_says_why_it_cannot() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for (ask, code) in [
            (json!({"scope": "everything"}), "invalid_request"),
            (json!({"scope": 1}), "invalid_request"),
            (json!({"pbs": "U-100"}), "invalid_request"),
            (
                json!({"scope": "project", "pbs": "U-100"}),
                "invalid_request",
            ),
            (json!({"snapshot": 3}), INDEX_NO_PROJECT_DIR),
            (json!({"scope": "pbs"}), SCOPE_NOT_SELECTED),
            (json!({"scope": "pbs", "pbs": " "}), SCOPE_NOT_SELECTED),
            (json!({"scope": "project"}), INDEX_NO_PROJECT_DIR),
            (
                json!({"scope": "pbs", "pbs": "U-100"}),
                INDEX_NO_PROJECT_DIR,
            ),
        ] {
            let mut ask = ask;
            ask["op"] = json!("pid_bom");
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }

        let scratch = Scratch::new();
        let mut app = project(&scratch, None);
        write_project_file(&scratch);
        let ask = json!({"op": "pid_bom", "scope": "pbs", "pbs": "U-999"});
        let answer = request(&mut app, ask);
        assert_eq!(answer["code"], PBS_NODE_UNKNOWN, "{answer}");
        let answer = settled(&mut app, &json!({"op": "pid_bom", "scope": "project"}));
        assert_eq!(answer["ok"], false, "{answer}");
        assert_eq!(answer["code"], bom::BOM_DUPLICATE_TAG, "{answer}");
        let pumps = objects_of(&app, ObjectKind::Equipment);
        let duplicates = answer["duplicates"].as_array().expect("duplicates");
        assert_eq!(duplicates.len(), 1, "{answer}");
        assert_eq!(duplicates[0]["tag"], pumps[0].tag.as_str(), "{answer}");
        let places: Vec<&str> = duplicates[0]["places"]
            .as_array()
            .expect("places")
            .iter()
            .filter_map(|place| place["drawing_id"].as_str())
            .collect();
        assert_eq!(places, ["PID-1101", "PID-1102"], "{answer}");

        fs::write(scratch.path("PID-1103.dwg"), "不是图纸").expect("坏文件");
        let answer = settled(&mut app, &json!({"op": "pid_bom", "scope": "project"}));
        assert_eq!(answer["code"], BOM_SCOPE_INCOMPLETE, "{answer}");
        let drawings = answer["drawings"].as_array().expect("drawings");
        assert_eq!(drawings.len(), 1, "{answer}");
        assert_eq!(drawings[0]["drawing_id"], "PID-1103", "{answer}");
        assert!(drawings[0]["error"].is_string(), "{answer}");
        let ask = json!({"op": "pid_bom", "scope": "pbs", "pbs": "U-100"});
        let answer = settled(&mut app, &ask);
        assert_eq!(answer["ok"], true, "{answer}");
    }

    /// `pid_bom_export`（§5.3）：按扩展名写文件，应答的 `result` 是写到哪里、什么格式、主表
    /// 几行、文件几个字节、副表写到哪里（JSON 没有副表文件，是 `null`）；不是完整路径时从当前
    /// 图所在的目录算起；`columns` 选列。写不成的一个文件也不写：列名不认识
    /// `BOM_COLUMN_UNKNOWN`、扩展名不支持 `BOM_FORMAT_UNSUPPORTED`，没给路径、参数类型不对、
    /// 是项目文件 `invalid_request`，目录不在 `BOM_IO_ERROR`。不改图，不留撤销步。
    #[test]
    fn pid_bom_export_op_writes_the_file_and_answers_what_it_wrote() {
        let scratch = Scratch::new();
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let current = scratch.path("PID-1101.dwg");
        save(&app, &current);
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(current.to_string_lossy().into_owned());
        let steps = undo_steps(&app);
        let dirty = app.tabs[i].dirty;
        let (bom, _) = app.pidbom_compute(Sheets::Current(i)).expect("清单");

        let main = scratch.path("bom.csv");
        let side = scratch.path("bom-pipe-summary.csv");
        let ask = json!({"op": "pid_bom_export", "path": main.to_string_lossy()});
        let wrote = request(&mut app, ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        let bytes = fs::read(&main).expect("CSV");
        assert_eq!(bytes, bom::csv_bytes(&bom::rows(&bom, None).expect("主表")));
        let result = &wrote["result"];
        assert_eq!(result["path"], main.display().to_string(), "{wrote}");
        assert_eq!(result["format"], "csv", "{wrote}");
        assert_eq!(result["rows"], bom.rows.len(), "{wrote}");
        assert_eq!(result["bytes"], bytes.len(), "{wrote}");
        let side_said = side.display().to_string();
        assert_eq!(result["pipe_summary_path"], side_said, "{wrote}");
        assert!(side.exists());

        let wrote = request(
            &mut app,
            json!({"op": "pid_bom_export", "path": "bom.json"}),
        );
        assert_eq!(wrote["ok"], true, "{wrote}");
        let path = scratch.path("bom.json");
        let result = &wrote["result"];
        assert_eq!(result["path"], path.display().to_string(), "{wrote}");
        assert_eq!(result["format"], "json", "{wrote}");
        assert_eq!(result["pipe_summary_path"], Value::Null, "{wrote}");
        let size = fs::metadata(&path).expect("JSON").len();
        assert_eq!(result["bytes"], size, "{wrote}");
        let mut back = read_back(&path);
        back.generated_at = bom.generated_at;
        assert_eq!(back, bom);

        let columns = ["位号", "数量"];
        let ask = json!({"op": "pid_bom_export", "path": "cols.XLSX", "columns": columns});
        let wrote = request(&mut app, ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"]["format"], "xlsx", "{wrote}");
        let picked: Vec<String> = columns.map(str::to_owned).to_vec();
        let table = bom::rows(&bom, Some(picked.as_slice())).expect("选列");
        assert_eq!(sheet_rows(&scratch.path("cols.XLSX")), table);

        for (ask, code, name) in [
            (
                json!({"path": "bad.csv", "columns": ["位号", "颜色"]}),
                bom::BOM_COLUMN_UNKNOWN,
                "bad.csv",
            ),
            (
                json!({"path": "bom.txt"}),
                BOM_FORMAT_UNSUPPORTED,
                "bom.txt",
            ),
            (json!({}), "invalid_request", ""),
            (json!({"path": 7}), "invalid_request", ""),
            (json!({"path": " "}), "invalid_request", ""),
            (
                json!({"path": "one.csv", "columns": "位号"}),
                "invalid_request",
                "one.csv",
            ),
            (
                json!({"path": "two.csv", "columns": [1]}),
                "invalid_request",
                "two.csv",
            ),
            (
                json!({"path": PROJECT_FILE}),
                "invalid_request",
                PROJECT_FILE,
            ),
            (json!({"path": "nowhere/bom.csv"}), BOM_IO_ERROR, "nowhere"),
        ] {
            let mut ask = ask;
            ask["op"] = json!("pid_bom_export");
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
            if !name.is_empty() {
                assert!(!scratch.path(name).exists(), "{name}");
            }
        }
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(app.tabs[i].dirty, dirty);
    }

    /// 按项目导出要先扫：扫着时应答 `scanning`，不写文件，同样的请求读完再发才写；按节点导出
    /// 只算节点下的图。只读会话里也能导出（§5.4）：写的是本机文件，不是图纸。
    #[test]
    fn pid_bom_export_op_waits_for_the_scan_and_runs_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = project(&scratch, Some("P-2001"));
        write_project_file(&scratch);
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        let path = scratch.path("all.xlsx");
        let ask = json!({"op": "pid_bom_export", "path": "all.xlsx", "scope": "PROJECT"});
        let answer = request(&mut app, ask.clone());
        assert_eq!(
            answer,
            json!({"ok": true, "status": "scanning", "done": 0, "total": 1})
        );
        assert!(!path.exists());
        let wrote = request(&mut app, ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"]["rows"], 3, "{wrote}");
        assert_eq!(sheet_rows(&path).len(), 4);

        let ask =
            json!({"op": "pid_bom_export", "path": "node.csv", "scope": "pbs", "pbs": "U-100"});
        let wrote = settled(&mut app, &ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"]["rows"], 2, "{wrote}");
        assert!(scratch.path("node.csv").exists());
        assert_eq!(undo_steps(&app), steps);
    }

    /// 临时目录里的 PID-1101.dwg：画好 [`plant`]、存过盘、打开着。
    fn saved_plant(scratch: &Scratch) -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let current = scratch.path("PID-1101.dwg");
        save(&app, &current);
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(current.to_string_lossy().into_owned());
        app
    }

    /// 存第 1 份快照，再改图：空白处再插一台泵（位号自动编）。得到存快照时现算的清单，改成从
    /// 第 1 份快照算的样子（`source`、`snapshot_seq`）：按这份快照算出的应当就是它。
    fn snapshot_then_edit(app: &mut OpenCADStudio) -> Bom {
        let i = app.active_tab;
        let (mut before, _) = app
            .pidbom_compute(Sheets::Current(i))
            .expect("改图前的清单");
        let ran = run(app, "PIDSNAPSHOT SAVE 改图前");
        assert_eq!(ran.error, None, "{}", ran.said);
        draw(app, "PIDINSERT pump", &[[0.0, -100.0]]);
        before.source = bom::SourceKind::Snapshot;
        before.snapshot_seq = Some(1);
        before
    }

    /// PID-1101 的快照目录里放一份坏的第 2 份：抄第 1 份，校验和改掉。
    fn corrupt_second(scratch: &Scratch) {
        let dir = store::dir_for(&scratch.0, "PID-1101");
        let bytes = fs::read(dir.join("0001.json")).expect("第 1 份");
        let mut tampered: Value = serde_json::from_slice(&bytes).expect("快照是 JSON");
        tampered["seq"] = json!(2);
        tampered["body_sha256"] = json!("0".repeat(64));
        fs::write(dir.join("0002.json"), tampered.to_string()).expect("写一份坏的");
    }

    /// `PIDBOM SNAPSHOT`（§5.2）：存一份快照后改图，按快照算的是改图之前的清单（`source` 是
    /// `snapshot`、`snapshot_seq` 是 1），`PIDBOM` 算的是改图之后的。关键字不分大小写；导出同
    /// `PIDBOM EXPORT`，写的也是快照的清单。
    #[test]
    fn pidbom_snapshot_counts_the_drawing_as_it_was_when_the_snapshot_was_saved() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let expected = snapshot_then_edit(&mut app);
        assert_eq!(expected.totals.rows, 7, "{expected:#?}");

        let ran = run(&mut app, "PIDBOM SNAPSHOT 1");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said =
            "PIDBOM SNAPSHOT: 图 PID-1101 的第 1 份快照，7 行（管线 1，设备 2，阀门 2，仪表 1，管件 1），";
        assert!(ran.said.contains(said), "{}", ran.said);
        let ran = run(&mut app, "PIDBOM");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = "PIDBOM: 图 PID-1101，8 行（管线 1，设备 3，阀门 2，仪表 1，管件 1），";
        assert!(ran.said.contains(said), "{}", ran.said);

        let ran = run(&mut app, r#"pidbom snapshot 1 export "快照 清单.json""#);
        assert_eq!(ran.error, None, "{}", ran.said);
        let mut back = read_back(&scratch.path("快照 清单.json"));
        back.generated_at = expected.generated_at;
        assert_eq!(back, expected);
        let ran = run(&mut app, "PIDBOM SNAPSHOT 1 EXPORT 快照清单.csv");
        assert_eq!(ran.error, None, "{}", ran.said);
        let bytes = fs::read(scratch.path("快照清单.csv")).expect("CSV");
        let table = bom::rows(&expected, None).expect("主表");
        assert_eq!(bytes, bom::csv_bytes(&table));
    }

    /// `PIDBOM SNAPSHOT` 算不出来时（§3.5）：图没存过、没有快照目录 `INDEX_NO_PROJECT_DIR`；
    /// 序号不在 `SNAPSHOT_NOT_FOUND`、快照坏了 `SNAPSHOT_CORRUPT`，要导出的都不写。写错了说
    /// 怎么写（连同按快照算的写法），不带错误码。
    #[test]
    fn pidbom_snapshot_says_why_it_cannot() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let ran = run(&mut app, "PIDBOM SNAPSHOT 1");
        let code = ran.code.as_deref();
        assert_eq!(code, Some(INDEX_NO_PROJECT_DIR), "{}", ran.said);
        for line in [
            "PIDBOM SNAPSHOT",
            "PIDBOM SNAPSHOT first",
            "PIDBOM SNAPSHOT -1",
            "PIDBOM SNAPSHOT 1 PROJECT",
            "PIDBOM SNAPSHOT 1 PBS U-100",
            "PIDBOM SNAPSHOT 1 EXPORT",
            "PIDBOM SNAPSHOT 1 EXPORT bom.csv PROJECT",
            "PIDBOM EXPORT bom.csv SNAPSHOT 1",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code, None, "{line}: {}", ran.said);
            let error = ran.error.unwrap_or_default();
            assert!(error.starts_with("PIDBOM"), "{line}: {error}");
            assert!(error.contains("PIDBOM SNAPSHOT <序号>"), "{line}: {error}");
        }

        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        snapshot_then_edit(&mut app);
        corrupt_second(&scratch);
        for (line, code, name) in [
            ("PIDBOM SNAPSHOT 9", SNAPSHOT_NOT_FOUND, ""),
            ("PIDBOM SNAPSHOT 0", SNAPSHOT_NOT_FOUND, ""),
            (
                "PIDBOM SNAPSHOT 9 EXPORT nine.csv",
                SNAPSHOT_NOT_FOUND,
                "nine.csv",
            ),
            ("PIDBOM SNAPSHOT 2", SNAPSHOT_CORRUPT, ""),
            (
                "PIDBOM SNAPSHOT 2 EXPORT two.json",
                SNAPSHOT_CORRUPT,
                "two.json",
            ),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code.as_deref(), Some(code), "{line}: {}", ran.said);
            let error = ran.error.unwrap_or_default();
            let said = format!("PIDBOM SNAPSHOT: {code}");
            assert!(error.starts_with(&said), "{line}: {error}");
            if !name.is_empty() {
                assert!(!scratch.path(name).exists(), "{line}");
            }
        }
    }

    /// 控制面带 `snapshot`（§5.3）：`pid_bom` 应答的是那份快照的清单，同 `PIDBOM SNAPSHOT`；
    /// 不带的还是改图之后的。`pid_bom_export` 写的也是快照的清单。序号不在
    /// `SNAPSHOT_NOT_FOUND`、快照坏了 `SNAPSHOT_CORRUPT`，都不写文件；`snapshot` 不是非负整数、
    /// 和按项目按节点算一起给是 `invalid_request`。
    #[test]
    fn pid_bom_ops_with_snapshot_answer_the_bom_of_that_snapshot() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let expected = snapshot_then_edit(&mut app);
        let answer = request(&mut app, json!({"op": "pid_bom", "snapshot": 1}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["source"], "snapshot", "{answer}");
        assert_eq!(answer["snapshot_seq"], 1, "{answer}");
        let mut answered: Bom = serde_json::from_value(answer.clone()).expect("一份清单");
        answered.generated_at = expected.generated_at;
        assert_eq!(answered, expected);
        let ask = json!({"op": "pid_bom", "scope": "Drawing", "snapshot": 1});
        let scoped = request(&mut app, ask);
        assert_eq!(scoped["rows"], answer["rows"], "{scoped}");
        let live = request(&mut app, json!({"op": "pid_bom"}));
        assert_eq!(live["source"], "live", "{live}");
        assert_eq!(live["snapshot_seq"], Value::Null, "{live}");
        assert_eq!(live["totals"]["rows"], 8, "{live}");

        let ask = json!({"op": "pid_bom_export", "path": "snap.json", "snapshot": 1});
        let wrote = request(&mut app, ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"]["rows"], 7, "{wrote}");
        let mut back = read_back(&scratch.path("snap.json"));
        back.generated_at = expected.generated_at;
        assert_eq!(back, expected);

        corrupt_second(&scratch);
        for (ask, code, name) in [
            (json!({"snapshot": 9}), SNAPSHOT_NOT_FOUND, ""),
            (json!({"snapshot": 2}), SNAPSHOT_CORRUPT, ""),
            (json!({"snapshot": "1"}), "invalid_request", ""),
            (json!({"snapshot": -1}), "invalid_request", ""),
            (json!({"snapshot": 1.5}), "invalid_request", ""),
            (
                json!({"scope": "project", "snapshot": 1}),
                "invalid_request",
                "",
            ),
            (
                json!({"scope": "pbs", "pbs": "U-100", "snapshot": 1}),
                "invalid_request",
                "",
            ),
            (
                json!({"path": "nine.csv", "snapshot": 9}),
                SNAPSHOT_NOT_FOUND,
                "nine.csv",
            ),
            (
                json!({"path": "two.csv", "snapshot": 2}),
                SNAPSHOT_CORRUPT,
                "two.csv",
            ),
            (
                json!({"path": "one.csv", "snapshot": "1"}),
                "invalid_request",
                "one.csv",
            ),
            (
                json!({"path": "all.csv", "scope": "project", "snapshot": 1}),
                "invalid_request",
                "all.csv",
            ),
        ] {
            let mut ask = ask;
            let op = if name.is_empty() {
                "pid_bom"
            } else {
                "pid_bom_export"
            };
            ask["op"] = json!(op);
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
            if !name.is_empty() {
                assert!(!scratch.path(name).exists(), "{ask}");
            }
        }
    }
}

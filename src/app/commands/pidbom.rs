//! PIDBOM：材料清单（`pid::bom`，`SPEC-pid-bom.md` §4、§5.2）。
//!
//! ```text
//! PIDBOM                    算当前图的清单：各种类几行、管长合计、跳过了几个对象
//! PIDBOM PROJECT            按项目算：项目目录里的全部图
//! PIDBOM PBS <节点号>       按节点算：项目文件里归在这个节点及其下级的图
//! PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>]
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
//! 导出：文件不是完整路径时从当前图所在的目录算起，没存过的图从当前目录算起；扩展名不分大小写，
//! 别的扩展名报 `BOM_FORMAT_UNSUPPORTED`，这时还没算清单；不拿清单盖项目文件。目标文件已存在
//! 时覆盖。清单算不出来、转表出错时一个文件也不写。JSON 是 §3.2 的对象，带 `ok`，缩进 2 格，
//! 副表在里面。
//!
//! 几个命令都不改图，只读会话里照常能用：导出写的是本机文件，不是图纸（§5.4）。

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
use serde::Serialize;
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

/// 副表的文件名：主表的文件名（去掉扩展名）后面加上它，扩展名照旧（§5.2）。
const PIPE_SUMMARY_SUFFIX: &str = "-pipe-summary";

/// 跳过的对象、重复的键、读不出来的图，命令行上各最多逐个列出这么多。
const LISTED: usize = 10;

const USAGE: &str =
    "PIDBOM: 写成 PIDBOM、PIDBOM PROJECT 或 PIDBOM PBS <节点号>；导出写成 PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>]。";

const EXPORT_USAGE: &str =
    "PIDBOM EXPORT: 写成 PIDBOM EXPORT <文件> [PROJECT | PBS <节点号>]；文件名里有空格就加双引号。";

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
}

impl BomFailure {
    /// 错误码（§3.5）。
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Index(error) => error.code(),
            Self::Incomplete(_) => BOM_SCOPE_INCOMPLETE,
            Self::Bom(error) => error.code(),
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

/// 写好的：主表的文件、格式、几行（不算列名），副表的文件和几行（JSON 没有）。
#[derive(Debug)]
struct Written {
    path: PathBuf,
    format: Format,
    rows: usize,
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
        let (export, scope) = if option.eq_ignore_ascii_case("EXPORT") {
            let Some((name, scope)) = file_word(after) else {
                self.command_line.push_error(EXPORT_USAGE);
                return Some(Task::none());
            };
            let Some(export) = self.pidbom_target(i, name) else {
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

    /// `PIDBOM EXPORT` 要写的文件：不是完整路径时从第 `i` 个标签页的图所在目录算起，没存过
    /// 的图从当前目录算起。扩展名不支持（`BOM_FORMAT_UNSUPPORTED`）、是项目文件时在命令行上
    /// 说，得 `None`。
    fn pidbom_target(&mut self, i: usize, name: &str) -> Option<Export> {
        let path = match crate::io::drawing_dir(&self.tabs[i].scene.document) {
            Some(dir) if Path::new(name).is_relative() => dir.join(name),
            _ => PathBuf::from(name),
        };
        let Some(format) = Format::of(&path) else {
            self.command_line.push_error_coded(
                BOM_FORMAT_UNSUPPORTED,
                &format!(
                    "PIDBOM EXPORT: {BOM_FORMAT_UNSUPPORTED}: 只写 .csv、.xlsx、.json 文件，{} 不是。",
                    path.display()
                ),
            );
            return None;
        };
        if path
            .file_name()
            .is_some_and(|file| file.eq_ignore_ascii_case(PROJECT_FILE))
        {
            self.command_line.push_error(&format!(
                "PIDBOM EXPORT: {} 是项目文件，不拿清单盖掉它。",
                path.display()
            ));
            return None;
        }
        Some(Export { path, format })
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
    /// 发布包都从这里算。当前图现做摘要（默认的连接容差）；项目、节点范围取扫好的项目里在
    /// 范围内的图（[`in_scope`]）。得到清单和算进去的图数。不碰命令行。
    pub(super) fn pidbom_compute(&self, sheets: Sheets<'_>) -> Result<(Bom, usize), BomFailure> {
        let (summaries, scope) = match sheets {
            Sheets::Current(i) => {
                let meta = self.drawing_meta(i).map_err(BomFailure::Index)?;
                let scope = BomScope::drawing(&meta.drawing_id);
                let document = &self.tabs[i].scene.document;
                let summary = summarize(document, &self.pid_symbols, meta, DEFAULT_TOLERANCE);
                (Cow::Owned(vec![summary]), scope)
            }
            Sheets::Scanned {
                scanned,
                range,
                master,
            } => in_scope(scanned, range, master)?,
        };
        let options = BomOptions {
            scope,
            source: BomSource::Live,
            generated_at: unix_now(),
            symbol_labels: bom::symbol_labels(&self.pid_symbols),
        };
        let bom = bom::build(&summaries, &options).map_err(BomFailure::Bom)?;
        Ok((bom, summaries.len()))
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
            scope_said(&bom.scope, drawings),
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
            BomFailure::Index(_) | BomFailure::Bom(_) => {}
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

/// 清单算的是什么：「当前图，」「图 PID-1101，」「全项目 2 张图，」「PBS 节点 U-100 1 张图，」。
fn scope_said(scope: &BomScope, drawings: usize) -> String {
    match scope.kind {
        ScopeKind::Drawing => {
            let drawing_id = scope.drawing_id.as_deref().unwrap_or_default();
            format!("{}，", drawing_said(drawing_id))
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
    Ok(Written {
        path: path.clone(),
        format: export.format,
        rows: bom.rows.len(),
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
}

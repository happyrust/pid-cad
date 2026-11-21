//! PIDSNAPSHOT、PIDDIFF：快照与比对（`pid::snapshot`，`SPEC-pid-snapshot.md` §5.2）。
//!
//! ```text
//! PIDSNAPSHOT [SAVE [<原因>]]       给当前图存一份带序号的快照，说序号和文件
//! PIDSNAPSHOT LIST                  列出当前图的快照：序号、时间、生成人、对象数、原因
//! PIDSNAPSHOT EXPORT <序号> <文件>  把一份原样复制出去
//! PIDSNAPSHOT FILE <文件> [<原因>]  不编序号，把当前图的快照直接写成这个文件
//! PIDDIFF <甲> [<乙>]               比对，说三组计数和前几条明细；不写乙就是与当前图比
//! PIDDIFF <甲> <乙> EXPORT <文件>   把比对结果（§3.7）写成 JSON 文件
//! ```
//!
//! 快照都从 [`OpenCADStudio::pidsnapshot_capture`] 做：当前图现做摘要（默认的连接容差，含没
//! 保存的修改）和连通图，交给 `snapshot::capture`；不必先存盘，`dirty` 记标签页的「已修改」。
//! 生成人同规则检查的豁免人：连着协同时是协同的用户号，否则是操作系统的账户名。带序号的
//! 快照放在当前图所在目录的 `pid-snapshots/<图号>/`（`snapshot::store`），图号同 `PIDINDEX`；
//! 图没存过就没有这个目录，报 `INDEX_NO_PROJECT_DIR`，`FILE` 形式照常能用（§3.1）。
//!
//! 原因是 `SAVE`、`FILE <文件>` 后面剩下的整段，整段加了双引号的去掉引号。甲、乙写序号（全是
//! 数字）、快照文件或 `LIVE`（当前图，现做一份），关键字不分大小写。文件名有空格时加双引号，
//! 不是完整路径的从当前图所在的目录算起，图没存过从当前目录算起。两边是同一份（同一个
//! 序号、同一个文件、都是当前图）报 `DIFF_SAME_SNAPSHOT`，这时什么都不读；序号和它自己的文件、
//! 导出的副本不认作同一份。结果的 `ref` 照写的填：序号、文件路径照原样，当前图是 `live`。
//!
//! 快照文件只新建：`EXPORT`、`FILE` 的目标已存在报 `SNAPSHOT_IMMUTABLE`。比对结果的 JSON 带
//! `"ok": true`，缩进 2 格、末尾一个换行，目标已存在时覆盖，不拿它盖项目文件。
//!
//! 两个命令都不改图，只读会话里照常能用：写的是本机文件，不是图纸（§5.4）。
//!
//! 自动化操作（§5.3）同这两个命令：读的 `pid_snapshots`（列表）、`pid_snapshot`（第几份的全文）、
//! `pid_diff`（`b` 不给是当前图，`move_tolerance` 交给比对）在控制面的读操作里；写的
//! `pid_snapshot_save`（给了 `path` 就是 `FILE` 形式）、`pid_snapshot_export` 走写操作的流程，结果
//! 放在 `result` 里。错误码同命令。

use super::pidindex::{number, point, same_file, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::pid::master_data::PROJECT_FILE;
use crate::pid::project_index::{graph, summarize, IndexError, DEFAULT_TOLERANCE};
use crate::pid::snapshot::store::{self, Saved, SnapshotInfo, SnapshotStatus};
use crate::pid::snapshot::{
    capture, diff, Change, Diff, DiffObject, EdgeChange, Snapshot, SnapshotError, SnapshotMeta,
    DEFAULT_MOVE_TOLERANCE,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 比对的两份是同一份（§3.8）。
pub(in crate::app) const DIFF_SAME_SNAPSHOT: &str = "DIFF_SAME_SNAPSHOT";

/// 比对的明细，命令行上最多逐个列出这么多个对象。
const LISTED: usize = 10;

const USAGE: &str =
    "PIDSNAPSHOT: 写成 PIDSNAPSHOT [SAVE [<原因>]]、PIDSNAPSHOT LIST、PIDSNAPSHOT EXPORT <序号> <文件> 或 PIDSNAPSHOT FILE <文件> [<原因>]；文件名里有空格就加双引号。";

const DIFF_USAGE: &str =
    "PIDDIFF: 写成 PIDDIFF <甲> [<乙>] 或 PIDDIFF <甲> <乙> EXPORT <文件>；甲、乙写序号、快照文件或 LIVE（当前图），不写乙就是与当前图比；文件名里有空格就加双引号。";

/// 比对的一边（§5.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Side {
    /// 当前图的第几份快照。
    Seq(u64),
    /// 快照文件：照写的，和从哪算起以后的路径。
    File { written: String, path: PathBuf },
    /// 当前图，现做一份。
    Live,
}

impl Side {
    /// 结果里的 `ref`（§3.7）。
    fn reference(&self) -> String {
        match self {
            Self::Seq(seq) => seq.to_string(),
            Self::File { written, .. } => written.clone(),
            Self::Live => "live".to_owned(),
        }
    }

    /// 命令行上怎么称呼这一份。
    fn said(&self) -> String {
        match self {
            Self::Seq(seq) => format!("第 {seq} 份"),
            Self::File { written, .. } => written.clone(),
            Self::Live => "当前图".to_owned(),
        }
    }

    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Seq(a), Self::Seq(b)) => a == b,
            (Self::Live, Self::Live) => true,
            (Self::File { path: a, .. }, Self::File { path: b, .. }) => {
                same_file(a) == same_file(b)
            }
            _ => false,
        }
    }
}

/// 快照命令做不成的原因。
#[derive(Debug)]
enum Failure {
    /// 图没存过，没有放带序号快照的目录（§3.1）。
    NoProjectDir,
    /// 定当前图的图号时读项目目录出错。
    Index(IndexError),
    /// 做快照、存取快照的错（§3.8）。
    Snapshot(SnapshotError),
    /// 比对的两边是同一份；带上是哪一份。
    Same(String),
    /// 比对结果没写成：说原因，不带错误码（同 `PIDBOM EXPORT`）。
    Write(String),
}

impl Failure {
    fn code(&self) -> Option<&'static str> {
        match self {
            Self::NoProjectDir => Some(INDEX_NO_PROJECT_DIR),
            Self::Index(error) => Some(error.code()),
            Self::Snapshot(error) => Some(error.code()),
            Self::Same(_) => Some(DIFF_SAME_SNAPSHOT),
            Self::Write(_) => None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProjectDir => write!(
                f,
                "{INDEX_NO_PROJECT_DIR}: 当前图没保存过，没有放带序号快照的目录；存盘后再用，或者用 PIDSNAPSHOT FILE <文件> 写成文件"
            ),
            Self::Index(error) => write!(f, "{error}"),
            Self::Snapshot(error) => write!(f, "{error}"),
            Self::Same(which) => write!(
                f,
                "{DIFF_SAME_SNAPSHOT}: 两边是同一份（{which}），比对要两份不同的"
            ),
            Self::Write(detail) => f.write_str(detail),
        }
    }
}

impl From<IndexError> for Failure {
    fn from(error: IndexError) -> Self {
        Self::Index(error)
    }
}

impl From<SnapshotError> for Failure {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

/// 带上应答那一层的 `ok`：`PIDDIFF … EXPORT` 写的 JSON（§3.7 的比对结果），自动化操作应答的
/// 快照全文和比对结果。
#[derive(Serialize)]
struct Answered<'a, T> {
    ok: bool,
    #[serde(flatten)]
    body: &'a T,
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidsnapshot(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (word, rest) = first_word(cmd);
        if !word.eq_ignore_ascii_case("PIDSNAPSHOT") {
            return None;
        }
        let (option, after) = first_word(rest);
        if option.is_empty() || option.eq_ignore_ascii_case("SAVE") {
            self.pidsnapshot_save(i, reason_of(after));
        } else if option.eq_ignore_ascii_case("LIST") && after.is_empty() {
            self.pidsnapshot_list(i);
        } else if option.eq_ignore_ascii_case("EXPORT") {
            match words(after).as_deref() {
                Some(&[seq, file]) if !file.is_empty() => match seq_of(seq) {
                    Some(seq) => self.pidsnapshot_export(i, seq, file),
                    None => self.command_line.push_error(USAGE),
                },
                _ => self.command_line.push_error(USAGE),
            }
        } else if option.eq_ignore_ascii_case("FILE") {
            match file_word(after) {
                Some((name, reason)) => self.pidsnapshot_file(i, name, reason_of(reason)),
                None => self.command_line.push_error(USAGE),
            }
        } else {
            self.command_line.push_error(USAGE);
        }
        Some(Task::none())
    }

    pub(super) fn dispatch_piddiff(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (word, rest) = first_word(cmd);
        if !word.eq_ignore_ascii_case("PIDDIFF") {
            return None;
        }
        match diff_asked(rest) {
            Some((a, b, to)) => self.piddiff(i, a, b, to),
            None => self.command_line.push_error(DIFF_USAGE),
        }
        Some(Task::none())
    }

    /// 给第 `i` 个标签页的图做一份快照（§4），序号空着。命令、自动化操作和以后的面板都从
    /// 这里做。只读图，不碰撤销栈和「已修改」。
    fn pidsnapshot_capture(&self, i: usize, reason: &str) -> Result<Snapshot, Failure> {
        let meta = self.drawing_meta(i)?;
        let document = &self.tabs[i].scene.document;
        let summary = summarize(document, &self.pid_symbols, meta, DEFAULT_TOLERANCE);
        let connections = graph(&summary);
        let meta = SnapshotMeta {
            created_at: unix_now(),
            created_by: self.waiver_by(),
            reason: reason.to_owned(),
            generator: format!("OpenCADStudio {}", env!("CARGO_PKG_VERSION")),
            dirty: self.tabs[i].dirty,
        };
        Ok(capture(summary, connections, meta)?)
    }

    fn pidsnapshot_drawing_dir(&self, i: usize) -> Result<PathBuf, Failure> {
        let document = &self.tabs[i].scene.document;
        let dir = crate::io::drawing_dir(document).ok_or(Failure::NoProjectDir)?;
        Ok(dir.to_path_buf())
    }

    /// 第 `i` 个标签页的图的快照目录（§3.1）和图号。
    fn pidsnapshot_dir(&self, i: usize) -> Result<(PathBuf, String), Failure> {
        let drawing_dir = self.pidsnapshot_drawing_dir(i)?;
        let drawing_id = self.drawing_meta(i)?.drawing_id;
        Ok((store::dir_for(&drawing_dir, &drawing_id), drawing_id))
    }

    /// 写的文件名：不是完整路径时从第 `i` 个标签页的图所在的目录算起，图没存过从当前目录
    /// 算起。
    fn pidsnapshot_path(&self, i: usize, name: &str) -> PathBuf {
        match crate::io::drawing_dir(&self.tabs[i].scene.document) {
            Some(dir) if Path::new(name).is_relative() => dir.join(name),
            _ => PathBuf::from(name),
        }
    }

    /// 存一份快照：不给文件时带序号存进快照目录，先认目录再做快照，图没存过时不做；给了文件就
    /// 不编序号写成它。一并交出 `about` 从这份快照里取的东西。
    fn pidsnapshot_store<T>(
        &self,
        i: usize,
        file: Option<&Path>,
        reason: &str,
        about: impl FnOnce(&Snapshot) -> T,
    ) -> Result<(Saved, T), Failure> {
        let Some(path) = file else {
            let drawing_dir = self.pidsnapshot_drawing_dir(i)?;
            let snapshot = self.pidsnapshot_capture(i, reason)?;
            let dir = store::dir_for(&drawing_dir, &snapshot.drawing_id);
            let about = about(&snapshot);
            return Ok((store::save(&dir, snapshot)?, about));
        };
        let snapshot = self.pidsnapshot_capture(i, reason)?;
        Ok((store::save_to_file(path, &snapshot)?, about(&snapshot)))
    }

    /// 第 `i` 个标签页的图的快照列表，连同快照目录和图号。
    fn pidsnapshot_listed(
        &self,
        i: usize,
    ) -> Result<(Vec<SnapshotInfo>, PathBuf, String), Failure> {
        let (dir, drawing_id) = self.pidsnapshot_dir(i)?;
        Ok((store::list(&dir)?, dir, drawing_id))
    }

    /// 把第 `seq` 份快照原样复制成 `to`，得到字节数。
    fn pidsnapshot_copy(&self, i: usize, seq: u64, to: &Path) -> Result<u64, Failure> {
        let (dir, _) = self.pidsnapshot_dir(i)?;
        Ok(store::export(&dir, seq, to)?)
    }

    /// `PIDSNAPSHOT [SAVE [<原因>]]`：存一份带序号的快照。
    fn pidsnapshot_save(&mut self, i: usize, reason: &str) {
        match self.pidsnapshot_store(i, None, reason, snapshot_said) {
            Ok((saved, said)) => {
                let seq = saved.seq.unwrap_or_default();
                let path = saved.path.display();
                let said = format!("PIDSNAPSHOT: 已存第 {seq} 份快照（{said}），文件 {path}。");
                self.command_line.push_output(&said);
            }
            Err(failure) => self.pidsnapshot_refuse("PIDSNAPSHOT", &failure),
        }
    }

    /// `PIDSNAPSHOT LIST`：先说有几份、在哪个目录，再一份一行。
    fn pidsnapshot_list(&mut self, i: usize) {
        let (infos, dir, drawing_id) = match self.pidsnapshot_listed(i) {
            Ok(listed) => listed,
            Err(failure) => return self.pidsnapshot_refuse("PIDSNAPSHOT LIST", &failure),
        };
        let dir = dir.display();
        if infos.is_empty() {
            let said = format!("PIDSNAPSHOT LIST: 图 {drawing_id} 还没有快照（{dir}）。");
            self.command_line.push_output(&said);
            return;
        }
        let count = infos.len();
        let said = format!("PIDSNAPSHOT LIST: 图 {drawing_id} 有 {count} 份快照，在 {dir}：");
        self.command_line.push_output(&said);
        for info in &infos {
            self.command_line.push_output(&info_said(info));
        }
    }

    /// `PIDSNAPSHOT EXPORT <序号> <文件>`：把一份原样复制出去。
    fn pidsnapshot_export(&mut self, i: usize, seq: u64, name: &str) {
        let to = self.pidsnapshot_path(i, name);
        match self.pidsnapshot_copy(i, seq, &to) {
            Ok(bytes) => {
                let to = to.display();
                let said = format!(
                    "PIDSNAPSHOT EXPORT: 已把第 {seq} 份快照原样复制成 {to}（{bytes} 字节）。"
                );
                self.command_line.push_output(&said);
            }
            Err(failure) => self.pidsnapshot_refuse("PIDSNAPSHOT EXPORT", &failure),
        }
    }

    /// `PIDSNAPSHOT FILE <文件> [<原因>]`：不编序号，把当前图的快照写成这个文件。
    fn pidsnapshot_file(&mut self, i: usize, name: &str, reason: &str) {
        let path = self.pidsnapshot_path(i, name);
        match self.pidsnapshot_store(i, Some(&path), reason, snapshot_said) {
            Ok((saved, said)) => {
                let path = saved.path.display();
                let said =
                    format!("PIDSNAPSHOT FILE: 已把当前图的快照写成 {path}（{said}，不编序号）。");
                self.command_line.push_output(&said);
            }
            Err(failure) => self.pidsnapshot_refuse("PIDSNAPSHOT FILE", &failure),
        }
    }

    /// `PIDDIFF <甲> [<乙>] [EXPORT <文件>]`：比对，说三组计数和前几条明细；给了文件就把结果写成
    /// JSON，写不成只说为什么。
    fn piddiff(&mut self, i: usize, a: &str, b: &str, to: Option<&str>) {
        let verb = if to.is_some() {
            "PIDDIFF EXPORT"
        } else {
            "PIDDIFF"
        };
        let (a, b) = (self.piddiff_side(i, a), self.piddiff_side(i, b));
        let compared = self
            .piddiff_compare(i, &a, &b, DEFAULT_MOVE_TOLERANCE)
            .and_then(|result| {
                let written = match to {
                    Some(name) => Some(self.piddiff_write(i, name, &result)?),
                    None => None,
                };
                Ok((result, written))
            });
        let (result, written) = match compared {
            Ok(compared) => compared,
            Err(failure) => return self.pidsnapshot_refuse(verb, &failure),
        };
        for line in diff_said(verb, &a, &b, &result) {
            self.command_line.push_output(&line);
        }
        for notice in &result.notices {
            let said = format!("{verb}: {}: {}", notice.code, notice.message);
            self.command_line.push_info(&said);
        }
        if let Some((path, bytes)) = written {
            let path = path.display();
            let said = format!("{verb}: 已把比对结果写成 {path}（{bytes} 字节）。");
            self.command_line.push_output(&said);
        }
    }

    /// 甲、乙的一种写法认成哪一份。
    fn piddiff_side(&self, i: usize, written: &str) -> Side {
        if written.eq_ignore_ascii_case("LIVE") {
            Side::Live
        } else if let Some(seq) = seq_of(written) {
            Side::Seq(seq)
        } else {
            let path = self.pidsnapshot_path(i, written);
            Side::File {
                written: written.to_owned(),
                path,
            }
        }
    }

    /// 比对甲、乙（§3.5–§3.7），位置差不超过 `tolerance` 的不算改了，`ref` 照写的填。
    fn piddiff_compare(
        &self,
        i: usize,
        a: &Side,
        b: &Side,
        tolerance: f64,
    ) -> Result<Diff, Failure> {
        if a.same_as(b) {
            return Err(Failure::Same(a.said()));
        }
        let old = self.piddiff_load(i, a)?;
        let new = self.piddiff_load(i, b)?;
        let mut result = diff(&old, &new, tolerance);
        result.a.reference = a.reference();
        result.b.reference = b.reference();
        Ok(result)
    }

    fn piddiff_load(&self, i: usize, side: &Side) -> Result<Snapshot, Failure> {
        match side {
            Side::Seq(seq) => Ok(store::load(&self.pidsnapshot_dir(i)?.0, *seq)?),
            Side::File { path, .. } => Ok(store::load_file(path)?),
            Side::Live => self.pidsnapshot_capture(i, ""),
        }
    }

    /// 把比对结果写成 `name`，说写到哪里、几个字节。
    fn piddiff_write(
        &self,
        i: usize,
        name: &str,
        result: &Diff,
    ) -> Result<(PathBuf, usize), Failure> {
        let path = self.pidsnapshot_path(i, name);
        if path
            .file_name()
            .is_some_and(|file| file.eq_ignore_ascii_case(PROJECT_FILE))
        {
            let shown = path.display();
            return Err(Failure::Write(format!(
                "{shown} 是项目文件，不拿比对结果盖它"
            )));
        }
        let exported = Answered {
            ok: true,
            body: result,
        };
        let text = serde_json::to_string_pretty(&exported)
            .map_err(|error| Failure::Write(format!("比对结果写不成 JSON：{error}")))?;
        let text = format!("{text}\n");
        std::fs::write(&path, &text).map_err(|error| {
            let shown = path.display();
            Failure::Write(format!("写不了 {shown}：{error}"))
        })?;
        Ok((path, text.len()))
    }

    fn pidsnapshot_refuse(&mut self, verb: &str, failure: &Failure) {
        let said = format!("{verb}: {failure}");
        match failure.code() {
            Some(code) => self.command_line.push_error_coded(code, &said),
            None => self.command_line.push_error(&said),
        }
    }
}

/// 自动化操作（§5.3）：应答和失败的写法同别的控制面操作，不碰命令行。
impl OpenCADStudio {
    /// 控制面的 `pid_snapshots`：当前图的快照列表，带图号和快照目录。每份同 `PIDSNAPSHOT LIST`
    /// 一行说的，另带文件名和整个文件的 SHA-256；文件不在了 `status` 是 `missing`。
    pub(in crate::app) fn control_pid_snapshots(&self) -> Value {
        let listed = self.pidsnapshot_op_tab().and_then(|i| {
            self.pidsnapshot_listed(i)
                .map_err(|failure| failure_answer(&failure))
        });
        match listed {
            Ok((infos, dir, drawing_id)) => json!({
                "ok": true,
                "drawing_id": drawing_id,
                "dir": dir.display().to_string(),
                "snapshots": infos,
            }),
            Err(refusal) => refusal,
        }
    }

    /// 控制面的 `pid_snapshot`：第 `seq` 份快照的全文（§3.2）带 `ok`；读的时候照常校验。
    pub(in crate::app) fn control_pid_snapshot(&self, req: &Value) -> Value {
        let loaded = self.pidsnapshot_op_tab().and_then(|i| {
            let seq = seq_asked(req)?;
            self.piddiff_load(i, &Side::Seq(seq))
                .map_err(|failure| failure_answer(&failure))
        });
        match loaded {
            Ok(snapshot) => json!(Answered {
                ok: true,
                body: &snapshot
            }),
            Err(refusal) => refusal,
        }
    }

    /// 控制面的 `pid_diff`：同 `PIDDIFF`，比对 `a`、`b`（不给是当前图），位置差不超过
    /// `move_tolerance` 的不算改了；应答是比对结果（§3.7）带 `ok`。
    pub(in crate::app) fn control_pid_diff(&self, req: &Value) -> Value {
        let compared = self.pidsnapshot_op_tab().and_then(|i| {
            let (a, b, tolerance) = diff_request(req)?;
            let (a, b) = (self.piddiff_side(i, &a), self.piddiff_side(i, &b));
            self.piddiff_compare(i, &a, &b, tolerance)
                .map_err(|failure| failure_answer(&failure))
        });
        match compared {
            Ok(result) => json!(Answered {
                ok: true,
                body: &result
            }),
            Err(refusal) => refusal,
        }
    }

    /// 控制面的 `pid_snapshot_save`，在写操作的流程里：同 `PIDSNAPSHOT SAVE`，给了 `path` 同
    /// `PIDSNAPSHOT FILE`。结果是序号（写成文件的是 `null`）、写到哪里、对象数、字节数和整个
    /// 文件的 SHA-256。不改图。
    pub(in crate::app) fn control_pid_snapshot_save(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        let i = self.pidsnapshot_op_tab()?;
        let reason = reason_asked(req)?;
        let file = path_asked(req)?.map(|name| self.pidsnapshot_path(i, name));
        let (saved, objects) = self
            .pidsnapshot_store(i, file.as_deref(), reason, |snapshot| {
                snapshot.counts.objects
            })
            .map_err(|failure| failure_answer(&failure))?;
        self.set_control_result(json!({
            "seq": saved.seq,
            "path": saved.path.display().to_string(),
            "objects": objects,
            "bytes": saved.bytes,
            "sha256": saved.sha256,
        }));
        Ok(Task::none())
    }

    /// 控制面的 `pid_snapshot_export`，在写操作的流程里：同 `PIDSNAPSHOT EXPORT`，把第 `seq` 份
    /// 原样复制成 `path`；结果是写到哪里、几个字节。目标已在报 `SNAPSHOT_IMMUTABLE`。
    pub(in crate::app) fn control_pid_snapshot_export(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        let i = self.pidsnapshot_op_tab()?;
        let seq = seq_asked(req)?;
        let name = path_asked(req)?
            .ok_or_else(|| refuse("invalid_request", "要给 path：复制到哪个文件"))?;
        let to = self.pidsnapshot_path(i, name);
        let bytes = self
            .pidsnapshot_copy(i, seq, &to)
            .map_err(|failure| failure_answer(&failure))?;
        self.set_control_result(json!({
            "path": to.display().to_string(),
            "bytes": bytes,
        }));
        Ok(Task::none())
    }

    /// 自动化操作对着的标签页：当前的；开始页上没有图。
    fn pidsnapshot_op_tab(&self) -> Result<usize, Value> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Err(refuse("no_document", "Create or open a drawing first"));
        }
        Ok(i)
    }
}

/// 自动化操作做不成的应答：错误码和说法同命令；图没存过时说给 `path` 写成文件。
fn failure_answer(failure: &Failure) -> Value {
    let error = match failure {
        Failure::NoProjectDir => format!(
            "{INDEX_NO_PROJECT_DIR}: 当前图没保存过，没有放带序号快照的目录；存盘后再用，或者给 pid_snapshot_save 一个 path 写成文件"
        ),
        _ => failure.to_string(),
    };
    refuse(failure.code().unwrap_or("invalid_request"), error)
}

/// 控制面的失败应答。
fn refuse(code: &str, error: impl Into<String>) -> Value {
    json!({ "ok": false, "status": "failed", "code": code, "error": error.into() })
}

/// `seq`：第几份快照，写整数。
fn seq_asked(req: &Value) -> Result<u64, Value> {
    req["seq"]
        .as_u64()
        .ok_or_else(|| refuse("invalid_request", "要给 seq：第几份快照，写整数"))
}

/// `path`：写到哪个文件，不给是 `None`；给了要是非空的文字。
fn path_asked(req: &Value) -> Result<Option<&str>, Value> {
    match &req["path"] {
        Value::Null => Ok(None),
        Value::String(name) if !name.trim().is_empty() => Ok(Some(name)),
        _ => Err(refuse("invalid_request", "path 写文件名")),
    }
}

/// `reason`：原因，去掉首尾空白；不给是空的。
fn reason_asked(req: &Value) -> Result<&str, Value> {
    match &req["reason"] {
        Value::Null => Ok(""),
        Value::String(reason) => Ok(reason.trim()),
        _ => Err(refuse("invalid_request", "reason 写文字")),
    }
}

/// `pid_diff` 的请求：`a` 必给，`b` 不给是当前图，都换成 `PIDDIFF` 的写法；`move_tolerance`
/// 不给是 [`DEFAULT_MOVE_TOLERANCE`]，给了要是不小于 0 的数。
fn diff_request(req: &Value) -> Result<(String, String, f64), Value> {
    let a = side_asked(&req["a"], "a")?
        .ok_or_else(|| refuse("invalid_request", "要给 a：比对的旧的一边"))?;
    let b = side_asked(&req["b"], "b")?.unwrap_or_else(|| "live".to_owned());
    let tolerance = match &req["move_tolerance"] {
        Value::Null => DEFAULT_MOVE_TOLERANCE,
        value => value
            .as_f64()
            .filter(|tolerance| *tolerance >= 0.0)
            .ok_or_else(|| refuse("invalid_request", "move_tolerance 写不小于 0 的数"))?,
    };
    Ok((a, b, tolerance))
}

/// 比对的一边：整数是序号，文字同 `PIDDIFF` 的写法（序号、快照文件或 `live`），去掉首尾空白；
/// 不给是 `None`。
fn side_asked(value: &Value, key: &str) -> Result<Option<String>, Value> {
    match value {
        Value::Null => Ok(None),
        Value::Number(seq) if seq.is_u64() => Ok(Some(seq.to_string())),
        Value::String(text) if !text.trim().is_empty() => Ok(Some(text.trim().to_owned())),
        _ => Err(refuse(
            "invalid_request",
            format!("{key} 写序号、快照文件或 live"),
        )),
    }
}

/// `PIDDIFF` 后面的写法：甲、乙（不写是 `LIVE`）、要写的文件。写得不对是 `None`。
fn diff_asked(text: &str) -> Option<(&str, &str, Option<&str>)> {
    let export = |word: &str| word.eq_ignore_ascii_case("EXPORT");
    let asked = match *words(text)?.as_slice() {
        [a] => (a, "LIVE", None),
        [a, b] => (a, b, None),
        [a, b, word, file] if export(word) => (a, b, Some(file)),
        _ => return None,
    };
    let (a, b, file) = asked;
    let fine = !a.is_empty() && !b.is_empty() && !export(a) && !export(b) && file != Some("");
    fine.then_some(asked)
}

/// 一份快照是哪张图的、多少个对象、有没有没保存的修改。
fn snapshot_said(snapshot: &Snapshot) -> String {
    let drawing = if snapshot.drawing_id.is_empty() {
        "没存过的图".to_owned()
    } else {
        format!("图 {}", snapshot.drawing_id)
    };
    let dirty = if snapshot.dirty {
        "，含没保存的修改"
    } else {
        ""
    };
    format!("{drawing}，{} 个对象{dirty}", snapshot.counts.objects)
}

/// 列表里的一份：序号、UTC 时间、生成人、对象数、原因；文件不在了照说。
fn info_said(info: &SnapshotInfo) -> String {
    let reason = if info.reason.is_empty() {
        String::new()
    } else {
        format!("  原因：{}", info.reason)
    };
    let missing = if info.status == SnapshotStatus::Missing {
        "  （文件不在了）"
    } else {
        ""
    };
    format!(
        "  #{}  {}  {}  {} 个对象{reason}{missing}",
        info.seq,
        utc_said(info.created_at),
        info.created_by,
        info.objects
    )
}

/// 比对结果的说法：一行三组计数（没有差别时说无差异），再按新增、删除、修改逐个列出对象，
/// 最多 [`LISTED`] 个。
fn diff_said(verb: &str, a: &Side, b: &Side, result: &Diff) -> Vec<String> {
    let sides = format!("{} → {}", a.said(), b.said());
    if result.identical {
        return vec![format!("{verb}: {sides}：无差异。")];
    }
    let summary = &result.summary;
    let by = &summary.by_what;
    let mut said = vec![format!(
        "{verb}: {sides}：新增 {}，删除 {}，修改 {}（属性 {}，位置 {}，长度 {}，管件 {}，连接 {}，对端 {}），\
         边多 {} 条、少 {} 条。",
        summary.added,
        summary.removed,
        summary.modified,
        by.property,
        by.position,
        by.length,
        by.fittings,
        by.connection,
        by.peer,
        summary.edges_added,
        summary.edges_removed
    )];
    let added = result.added.iter().map(|object| ("新增", object));
    let objects = added
        .chain(result.removed.iter().map(|object| ("删除", object)))
        .chain(result.modified.iter().map(|object| ("修改", object)));
    said.extend(
        objects
            .take(LISTED)
            .map(|(group, object)| object_said(group, object)),
    );
    let total = result.added.len() + result.removed.len() + result.modified.len();
    if total > LISTED {
        said.push(format!("  还有 {} 个对象没列出。", total - LISTED));
    }
    said
}

/// 明细里的一个对象：组、种类、类型编码、位号、位置；修改的接着说改了什么。
fn object_said(group: &str, object: &DiffObject) -> String {
    let tag = if object.tag.is_empty() {
        "（无位号）"
    } else {
        object.tag.as_str()
    };
    let at = point(object.at);
    let said = format!("  {group} {} {} {tag} {at}", object.kind, object.type_code);
    if object.changes.is_empty() {
        return said;
    }
    let changes: Vec<String> = object.changes.iter().map(change_said).collect();
    format!("{said}：{}", changes.join("；"))
}

/// 一处不同（§3.6）怎么说。
fn change_said(change: &Change) -> String {
    match change {
        Change::Property { field, old, new } => {
            format!("属性 {field} {} → {}", value_said(old), value_said(new))
        }
        Change::Position { old, new } => {
            format!("位置 {} → {}", place_said(old), place_said(new))
        }
        Change::Length { old, new } => format!("长度 {} → {}", number(*old), number(*new)),
        Change::Fittings { symbol, old, new } => format!("管件 {symbol} {old} → {new} 个"),
        Change::Connection { change, with, port } => {
            let what = match change {
                EdgeChange::Added => "新增",
                EdgeChange::Removed => "删除",
            };
            let port = port
                .as_deref()
                .map(|port| format!("（本端 {port}）"))
                .unwrap_or_default();
            format!("连接 {what} {with}{port}")
        }
        Change::Peer { field, old, new } => {
            format!("对端 {field} {} → {}", value_said(old), value_said(new))
        }
    }
}

/// 一边的值：没有和空的都说「（空）」。
fn value_said(value: &Option<String>) -> &str {
    match value.as_deref() {
        Some(text) if !text.is_empty() => text,
        _ => "（空）",
    }
}

/// 位置：两个数是点，四个数是包围盒的两个角。
fn place_said(numbers: &[f64]) -> String {
    match *numbers {
        [x, y] => point([x, y]),
        [x1, y1, x2, y2] => format!("{}–{}", point([x1, y1]), point([x2, y2])),
        _ => format!("{numbers:?}"),
    }
}

/// 原因：去掉首尾空白；整段加了双引号的去掉引号。
fn reason_of(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(text)
}

/// 全是数字的写法是序号。
fn seq_of(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}

/// 开头的文件名和后面剩下的；文件名可以加双引号。没有文件名、引号没配对是 `None`。
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

/// 按空白分开的几个词；加了双引号的一段算一个词，去掉引号。引号没配对是 `None`。
fn words(text: &str) -> Option<Vec<&str>> {
    let mut found = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (word, after) = match rest.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"')?;
                (&quoted[..end], &quoted[end + 1..])
            }
            None => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            }
        };
        found.push(word);
        rest = after.trim_start();
    }
    Some(found)
}

/// 现在的 Unix 秒；1970 年以前的算 0。
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Unix 秒写成 UTC 的日期和时刻，到分，标明 UTC（§7.4：标准库拿不到本机时区）。
fn utc_said(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or_default();
    let minutes = seconds % 86_400 / 60;
    // 1970-01-01 起的天数换成公历日期（Howard Hinnant 的 civil_from_days）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (hour, minute) = (minutes / 60, minutes % 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::project_index::{ObjectKind, SummaryMeta, SummaryObject, SummarySource};
    use crate::pid::snapshot::{
        MAX_REASON_CHARS, SNAPSHOT_CORRUPT, SNAPSHOT_IMMUTABLE, SNAPSHOT_NOT_FOUND,
        SNAPSHOT_REASON_TOO_LONG,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 不在线上的阀门插在哪里：离泵、容器、管路都远，删掉它不牵动连接。
    const LOOSE_VALVE: [f64; 2] = [-100.0, 60.0];
    /// 改图时新插的仪表插在哪里，也不碰别的。
    const NEW_INSTRUMENT: [f64; 2] = [60.0, 80.0];

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
                "ocs-pidsnapshot-{}-{}-{nanos}",
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
        req["request_id"] = json!(format!(
            "pidsnapshot-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
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

    /// 当前图上种类是 `kind`、定位点在 `at` 的那一个对象。
    fn object_at(app: &OpenCADStudio, kind: ObjectKind, at: [f64; 2]) -> SummaryObject {
        let meta = SummaryMeta {
            drawing_id: String::new(),
            file: None,
            pbs: None,
            source: SummarySource::Open,
            fingerprint: None,
        };
        let document = app.active_document_for_test();
        let summary = summarize(document, &app.pid_symbols, meta, DEFAULT_TOLERANCE);
        let mut found = summary.objects.into_iter().filter(|object| {
            object.kind == kind
                && (object.at[0] - at[0]).abs() < 1e-6
                && (object.at[1] - at[1]).abs() < 1e-6
        });
        let object = found.next().unwrap_or_else(|| panic!("{kind:?} 在 {at:?}"));
        assert!(found.next().is_none(), "{kind:?} 在 {at:?} 不止一个");
        object
    }

    /// 用命令建的一张图（§7）：一台泵，左上方一个不在线上的阀门；一台容器；管路从泵的出口到
    /// 容器的入口。
    fn plant(app: &mut OpenCADStudio) {
        draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(app, "PIDINSERT gate_valve", &[LOOSE_VALVE]);
        draw(app, "PIDINSERT vessel", &[[200.0, 12.0]]);
        draw(app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
        app.finish_all_pending_history();
    }

    /// 画好 [`plant`]、存成临时目录里的 PID-1101.dwg 的当前图。
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

    /// PID-1101 的快照目录。
    fn snapshots(scratch: &Scratch) -> PathBuf {
        store::dir_for(&scratch.0, "PID-1101")
    }

    /// 改图（§7）：泵的设计温度填成 80，删掉不在线上的阀门，空白处新插一台仪表。
    fn edit(app: &mut OpenCADStudio) {
        let pump = object_at(app, ObjectKind::Equipment, [0.0, 0.0]);
        let ran = run(app, &format!("PIDSETPROP {} 设计温度 80", pump.tag));
        assert_eq!(ran.error, None, "{}", ran.said);
        let valve = object_at(app, ObjectKind::Valve, LOOSE_VALVE);
        let selected = request(app, json!({"op": "select", "handles": [valve.handle]}));
        assert_eq!(selected["ok"], true, "{selected}");
        let erased = request(app, json!({"op": "run", "cmd": "ERASE"}));
        assert_eq!(erased["ok"], true, "{erased}");
        draw(app, "PIDINSERT transmitter", &[NEW_INSTRUMENT]);
        app.finish_all_pending_history();
    }

    /// 说的话里以 `start` 开头的那一行。
    fn line_of<'a>(said: &'a str, start: &str) -> &'a str {
        said.lines()
            .find(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("没有以「{start}」开头的一行：\n{said}"))
    }

    /// 验收一（§7）：用命令建图、存盘，`PIDSNAPSHOT SAVE 送审前` 存出第 1 份；改图（改泵的
    /// 一个属性、删一个阀门、插一台仪表）后 `PIDDIFF 1` 说三组计数，逐条说明细。
    #[test]
    fn pidsnapshot_save_then_piddiff_counts_and_lists_what_changed() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT SAVE 送审前");
        assert_eq!(ran.error, None, "{}", ran.said);
        let file = snapshots(&scratch).join("0001.json");
        let said = line_of(&ran.said, "PIDSNAPSHOT: ");
        assert!(
            said.starts_with("PIDSNAPSHOT: 已存第 1 份快照（图 PID-1101，"),
            "{said}"
        );
        assert!(said.contains(&file.display().to_string()), "{said}");
        let saved = store::load(&snapshots(&scratch), 1).expect("第 1 份");
        assert_eq!(saved.seq, Some(1));
        assert_eq!(saved.reason, "送审前");
        assert_eq!(saved.drawing_id, "PID-1101");

        let pump = object_at(&app, ObjectKind::Equipment, [0.0, 0.0]);
        let valve = object_at(&app, ObjectKind::Valve, LOOSE_VALVE);
        edit(&mut app);
        let instrument = object_at(&app, ObjectKind::Instrument, NEW_INSTRUMENT);
        let ran = run(&mut app, "PIDDIFF 1");
        assert_eq!(ran.error, None, "{}", ran.said);
        let counts = "PIDDIFF: 第 1 份 → 当前图：新增 1，删除 1，修改 1（属性 1，位置 0，长度 0，\
                      管件 0，连接 0，对端 0），边多 0 条、少 0 条。";
        assert_eq!(line_of(&ran.said, "PIDDIFF: "), counts, "{}", ran.said);
        let added = line_of(&ran.said, "  新增 ");
        let wanted = format!("  新增 仪表 {} ", instrument.type_code);
        assert!(added.starts_with(&wanted), "{added}");
        assert!(added.ends_with("(60, 80)"), "{added}");
        let removed = line_of(&ran.said, "  删除 ");
        let wanted = format!("  删除 阀门 {} ", valve.type_code);
        assert!(removed.starts_with(&wanted), "{removed}");
        assert!(removed.ends_with("(-100, 60)"), "{removed}");
        let modified = line_of(&ran.said, "  修改 ");
        let wanted = format!("  修改 设备 {} {} (0, 0)：", pump.type_code, pump.tag);
        assert!(modified.starts_with(&wanted), "{modified}");
        assert!(modified.contains("属性 设计温度 （空） → 80"), "{modified}");
        let details = ran.said.lines().filter(|line| line.starts_with("  "));
        assert_eq!(details.count(), 3, "三组各一条明细：\n{}", ran.said);
    }

    /// 验收一：从命令行敲 `PIDSNAPSHOT SAVE <原因>` 只存出一份；光敲 `PIDSNAPSHOT`、小写的
    /// 也一样，序号接着编。原因是 `SAVE` 后面的整段，整段加了双引号的去掉引号。
    #[test]
    fn pidsnapshot_from_the_command_line_saves_one_snapshot_each_time() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        for (line, count) in [
            ("PIDSNAPSHOT SAVE 送审前", 1),
            ("PIDSNAPSHOT", 2),
            ("pidsnapshot save 第三次 改完", 3),
            ("PIDSNAPSHOT SAVE \"送审 前\"", 4),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(
                ran.said.matches("已存第").count(),
                1,
                "{line}: {}",
                ran.said
            );
            let listed = store::list(&snapshots(&scratch)).expect("快照列表");
            assert_eq!(listed.len(), count, "{line}: {listed:#?}");
        }
        let third = store::load(&snapshots(&scratch), 3).expect("第 3 份");
        assert_eq!(third.reason, "第三次 改完", "原因是 SAVE 后面的整段");
        assert_eq!(
            store::load(&snapshots(&scratch), 2)
                .expect("第 2 份")
                .reason,
            ""
        );
        let fourth = store::load(&snapshots(&scratch), 4).expect("第 4 份");
        assert_eq!(fourth.reason, "送审 前", "整段加了双引号的去掉引号");
    }

    /// 验收二：做快照、列快照、复制、比对、导出比对结果，前后图的「已修改」、撤销栈、编辑
    /// 修订号都不变。快照的 `dirty` 照「已修改」记，生成人同豁免人，生成程序是程序名和版本。
    #[test]
    fn snapshots_and_comparisons_leave_the_drawing_its_dirty_mark_and_undo_steps_alone() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let i = app.active_tab;
        let dirty = app.tabs[i].dirty;
        let steps = undo_steps(&app);
        let revision = app.tabs[i].edit_revision;
        for line in [
            "PIDSNAPSHOT SAVE 送审前",
            "PIDSNAPSHOT LIST",
            "PIDSNAPSHOT EXPORT 1 copy.json",
            "PIDSNAPSHOT FILE spare.json 备份",
            "PIDDIFF 1",
            "PIDDIFF 1 spare.json EXPORT diff.json",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(app.tabs[i].dirty, dirty, "{line}");
            assert_eq!(undo_steps(&app), steps, "{line}");
            assert_eq!(app.tabs[i].edit_revision, revision, "{line}");
        }
        let first = store::load(&snapshots(&scratch), 1).expect("第 1 份");
        assert_eq!(first.dirty, dirty);
        assert_eq!(first.created_by, app.waiver_by());
        let generator = format!("OpenCADStudio {}", env!("CARGO_PKG_VERSION"));
        assert_eq!(first.generator, generator);

        app.tabs[i].dirty = !dirty;
        let ran = run(&mut app, "PIDSNAPSHOT");
        assert_eq!(ran.error, None, "{}", ran.said);
        let second = store::load(&snapshots(&scratch), 2).expect("第 2 份");
        assert_eq!(second.dirty, !dirty, "dirty 跟着「已修改」走");
        assert_eq!(app.tabs[i].dirty, !dirty);
    }

    /// 验收二：没存过的图没有带序号的快照，`PIDSNAPSHOT`、`LIST`、`EXPORT` 和按序号比对都报
    /// `INDEX_NO_PROJECT_DIR`；`PIDSNAPSHOT FILE <文件>` 照常写出一份（`seq` 是空的，图号是
    /// 空的），拿它与当前图比是无差异。
    #[test]
    fn an_unsaved_drawing_has_no_numbered_snapshots_but_the_file_form_still_writes_one() {
        let scratch = Scratch::new();
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        for line in [
            "PIDSNAPSHOT",
            "PIDSNAPSHOT SAVE 送审前",
            "PIDSNAPSHOT LIST",
            "PIDSNAPSHOT EXPORT 1 copy.json",
            "PIDDIFF 1",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(INDEX_NO_PROJECT_DIR),
                "{line}: {}",
                ran.said
            );
        }
        let file = scratch.path("草稿 快照.json");
        let ran = run(
            &mut app,
            &format!("PIDSNAPSHOT FILE \"{}\" 先存一份", file.display()),
        );
        assert_eq!(ran.error, None, "{}", ran.said);
        let written = store::load_file(&file).expect("写出的快照");
        assert_eq!(written.seq, None);
        assert_eq!(written.drawing_id, "");
        assert_eq!(written.reason, "先存一份");
        let ran = run(&mut app, &format!("PIDDIFF \"{}\"", file.display()));
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDDIFF: ");
        assert!(said.ends_with(" → 当前图：无差异。"), "{}", ran.said);
    }

    /// 验收二：只读会话里两个命令的各种写法都不拦，照常存、列、复制、比对、导出（写的是本机
    /// 文件，不是图纸，§5.4）。
    #[test]
    fn pidsnapshot_and_piddiff_run_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        app.collab.readonly = true;
        for line in [
            "PIDSNAPSHOT",
            "PIDSNAPSHOT SAVE 送审前",
            "PIDSNAPSHOT LIST",
            "PIDSNAPSHOT EXPORT 1 copy.json",
            "PIDSNAPSHOT FILE spare.json",
            "PIDDIFF 1",
            "PIDDIFF 1 2",
            "PIDDIFF 1 LIVE EXPORT diff.json",
        ] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        for name in ["copy.json", "spare.json", "diff.json"] {
            assert!(scratch.path(name).is_file(), "{name}");
        }
        assert_eq!(
            store::list(&snapshots(&scratch)).expect("快照列表").len(),
            2
        );
    }

    /// 验收三：比对的两边是同一份报 `DIFF_SAME_SNAPSHOT`（同一个序号、都是当前图、同一个
    /// 文件）；序号、文件不在报 `SNAPSHOT_NOT_FOUND`，复制不在的序号也是，什么都不写。序号与
    /// 它自己的文件不算同一份，照常比。
    #[test]
    fn piddiff_refuses_the_same_snapshot_twice_and_a_snapshot_that_is_not_there() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        for line in [
            "PIDSNAPSHOT SAVE 一",
            "PIDSNAPSHOT SAVE 二",
            "PIDSNAPSHOT FILE spare.json",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        let spare = scratch.path("spare.json");
        let spare_again = format!("PIDDIFF spare.json \"{}\"", spare.display());
        let same = [
            "PIDDIFF 2 2",
            "PIDDIFF LIVE",
            "PIDDIFF live LIVE",
            spare_again.as_str(),
        ];
        for line in same {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(DIFF_SAME_SNAPSHOT),
                "{line}: {}",
                ran.said
            );
        }
        for line in [
            "PIDDIFF 3",
            "PIDDIFF 1 9",
            "PIDDIFF missing.json",
            "PIDSNAPSHOT EXPORT 9 out.json",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(SNAPSHOT_NOT_FOUND),
                "{line}: {}",
                ran.said
            );
        }
        assert!(!scratch.path("out.json").exists());
        let ran = run(&mut app, "PIDDIFF 1 2");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDDIFF: ");
        assert_eq!(said, "PIDDIFF: 第 1 份 → 第 2 份：无差异。", "{}", ran.said);
        let own = snapshots(&scratch).join("0001.json");
        let ran = run(&mut app, &format!("PIDDIFF 1 \"{}\"", own.display()));
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDDIFF: ");
        assert!(said.ends_with("：无差异。"), "{}", ran.said);
    }

    /// 验收三：`PIDDIFF 1 LIVE EXPORT <文件>` 写出的 JSON 就是 §3.7 的比对结果：带 `ok`，
    /// 键的先后照 §3.7，缩进 2 格、末尾一个换行，`ref` 照写的填；其余与 `diff` 算的一样。
    /// 文件已在时覆盖；文件名是项目文件的不写，不带错误码。
    #[test]
    fn piddiff_export_writes_the_comparison_result_as_json() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT SAVE 送审前");
        assert_eq!(ran.error, None, "{}", ran.said);
        edit(&mut app);
        fs::write(scratch.path("diff.json"), "旧的").expect("占位");
        let ran = run(&mut app, "PIDDIFF 1 LIVE EXPORT diff.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let path = scratch.path("diff.json");
        assert!(
            ran.said.contains(&path.display().to_string()),
            "{}",
            ran.said
        );
        let text = fs::read_to_string(&path).expect("比对结果");
        assert!(
            text.starts_with("{\n  \"ok\": true,\n  \"version\": 1,\n"),
            "{text}"
        );
        assert!(text.ends_with("}\n") && !text.ends_with("\n\n"), "{text}");
        let value: Value = serde_json::from_str(&text).expect("JSON");
        let keys: Vec<&str> = value
            .as_object()
            .expect("对象")
            .keys()
            .map(String::as_str)
            .collect();
        let order = [
            "ok",
            "version",
            "a",
            "b",
            "identical",
            "summary",
            "added",
            "removed",
            "modified",
            "edges",
            "notices",
        ];
        assert_eq!(keys, order);
        assert_eq!(value["a"]["ref"], "1");
        assert_eq!(value["a"]["seq"], 1);
        assert_eq!(value["b"]["ref"], "live");
        assert_eq!(value["b"]["seq"], Value::Null);
        assert_eq!(value["summary"]["新增"], 1);
        assert_eq!(value["summary"]["删除"], 1);
        assert_eq!(value["summary"]["修改"], 1);

        let before = store::load(&snapshots(&scratch), 1).expect("第 1 份");
        let live = app.pidsnapshot_capture(app.active_tab, "").expect("当前图");
        let mut expected = diff(&before, &live, DEFAULT_MOVE_TOLERANCE);
        expected.a.reference = "1".to_owned();
        expected.b.reference = "live".to_owned();
        expected.b.created_at = value["b"]["created_at"].as_u64().expect("做的时间");
        let mut got = value.clone();
        got.as_object_mut().expect("对象").remove("ok");
        assert_eq!(got, serde_json::to_value(&expected).expect("比对结果"));

        let ran = run(&mut app, &format!("PIDDIFF 1 LIVE EXPORT {PROJECT_FILE}"));
        assert_eq!(ran.code, None, "{}", ran.said);
        let error = ran.error.unwrap_or_default();
        assert!(error.contains("是项目文件"), "{error}");
        assert!(!scratch.path(PROJECT_FILE).exists());
    }

    /// `PIDSNAPSHOT EXPORT` 把一份原样复制出去（不是完整路径的放在图所在的目录），目标已在
    /// 报 `SNAPSHOT_IMMUTABLE`、不盖；`FILE` 的目标已在也是。原因太长报
    /// `SNAPSHOT_REASON_TOO_LONG`，不存。
    #[test]
    fn pidsnapshot_export_copies_byte_for_byte_and_nothing_is_overwritten() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT SAVE 送审前");
        assert_eq!(ran.error, None, "{}", ran.said);
        let ran = run(&mut app, "PIDSNAPSHOT EXPORT 1 送审前.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let copy = scratch.path("送审前.json");
        let original = fs::read(snapshots(&scratch).join("0001.json")).expect("第 1 份");
        assert_eq!(fs::read(&copy).expect("复制的"), original);
        for line in [
            "PIDSNAPSHOT EXPORT 1 送审前.json",
            "PIDSNAPSHOT FILE 送审前.json",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(SNAPSHOT_IMMUTABLE),
                "{line}: {}",
                ran.said
            );
            assert_eq!(fs::read(&copy).expect("复制的"), original, "{line}");
        }
        let long = "字".repeat(MAX_REASON_CHARS + 1);
        let ran = run(&mut app, &format!("PIDSNAPSHOT SAVE {long}"));
        assert_eq!(
            ran.code.as_deref(),
            Some(SNAPSHOT_REASON_TOO_LONG),
            "{}",
            ran.said
        );
        assert_eq!(
            store::list(&snapshots(&scratch)).expect("快照列表").len(),
            1
        );
    }

    /// `PIDSNAPSHOT LIST` 先说有几份，再一份一行：序号、UTC 时间、生成人、对象数、原因；还
    /// 没有快照时照说；文件不在了的标出来。
    #[test]
    fn pidsnapshot_list_says_each_snapshot_with_its_time_in_utc() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT LIST");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDSNAPSHOT LIST: ");
        assert!(
            said.starts_with("PIDSNAPSHOT LIST: 图 PID-1101 还没有快照"),
            "{said}"
        );
        for line in ["PIDSNAPSHOT SAVE 送审前", "PIDSNAPSHOT"] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        let ran = run(&mut app, "PIDSNAPSHOT LIST");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDSNAPSHOT LIST: ");
        assert!(
            said.starts_with("PIDSNAPSHOT LIST: 图 PID-1101 有 2 份快照"),
            "{said}"
        );
        let rows: Vec<&str> = ran
            .said
            .lines()
            .filter(|line| line.starts_with("  #"))
            .collect();
        assert_eq!(rows.len(), 2, "{}", ran.said);
        let listed = store::list(&snapshots(&scratch)).expect("快照列表");
        for (line, info) in rows.iter().zip(&listed) {
            let wanted = format!(
                "  #{}  {}  {}  {} 个对象",
                info.seq,
                utc_said(info.created_at),
                info.created_by,
                info.objects
            );
            assert!(line.starts_with(&wanted), "{line}\n{wanted}");
        }
        assert!(rows[0].ends_with("  原因：送审前"), "{}", rows[0]);
        assert!(!rows[1].contains("原因"), "{}", rows[1]);

        fs::remove_file(snapshots(&scratch).join("0002.json")).expect("只读的文件也删得掉");
        let ran = run(&mut app, "PIDSNAPSHOT LIST");
        assert_eq!(ran.error, None, "{}", ran.said);
        let rows: Vec<&str> = ran
            .said
            .lines()
            .filter(|line| line.starts_with("  #"))
            .collect();
        assert_eq!(rows.len(), 2, "{}", ran.said);
        assert!(!rows[0].contains("文件不在了"), "{}", rows[0]);
        assert!(rows[1].starts_with("  #2  "), "{}", rows[1]);
        assert!(rows[1].ends_with("  （文件不在了）"), "{}", rows[1]);
    }

    /// 比对的明细最多逐个列出 10 个对象，多的说还有几个。
    #[test]
    fn piddiff_lists_at_most_ten_objects_and_says_how_many_more() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT");
        assert_eq!(ran.error, None, "{}", ran.said);
        for n in 0..12 {
            draw(
                &mut app,
                "PIDINSERT transmitter",
                &[[f64::from(n) * 40.0, -200.0]],
            );
        }
        let ran = run(&mut app, "PIDDIFF 1");
        assert_eq!(ran.error, None, "{}", ran.said);
        let said = line_of(&ran.said, "PIDDIFF: ");
        assert!(said.contains("：新增 12，删除 0，修改 0（"), "{said}");
        let listed = ran.said.lines().filter(|line| line.starts_with("  新增 "));
        assert_eq!(listed.count(), 10, "{}", ran.said);
        let more = ran.said.lines().filter(|line| line.starts_with("  还有 "));
        assert_eq!(
            more.collect::<Vec<_>>(),
            ["  还有 2 个对象没列出。"],
            "{}",
            ran.said
        );
    }

    /// 两个命令都登记了；写错的说怎么写、不带错误码。
    #[test]
    fn pidsnapshot_and_piddiff_are_registered_and_say_how_they_are_written() {
        let names = crate::command::all_registered_command_names();
        assert!(names.contains(&"PIDSNAPSHOT"), "PIDSNAPSHOT 没登记");
        assert!(names.contains(&"PIDDIFF"), "PIDDIFF 没登记");
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        for line in [
            "PIDSNAPSHOT 送审前",
            "PIDSNAPSHOT LIST 1",
            "PIDSNAPSHOT EXPORT",
            "PIDSNAPSHOT EXPORT 1",
            "PIDSNAPSHOT EXPORT first out.json",
            "PIDSNAPSHOT EXPORT 1 out.json more",
            "PIDSNAPSHOT FILE",
            "PIDDIFF",
            "PIDDIFF 1 2 3",
            "PIDDIFF 1 EXPORT diff.json",
            "PIDDIFF 1 2 EXPORT",
            "PIDDIFF 1 EXPORT",
            "PIDDIFF \"1",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code, None, "{line}: {}", ran.said);
            let error = ran.error.unwrap_or_default();
            let verb = line.split_whitespace().next().unwrap_or_default();
            assert!(
                error.starts_with(verb) && error.contains("写成"),
                "{line}: {error}"
            );
        }
    }

    /// 列表里的时间按 UTC 写到分，标明 UTC（§7.4）。
    #[test]
    fn utc_said_writes_unix_seconds_as_a_utc_date_and_minute() {
        assert_eq!(utc_said(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_said(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc_said(1_795_272_374), "2026-11-21 14:46 UTC");
    }

    /// 写操作做成了，它的结果（`result`）。
    fn result_of(answer: &Value) -> &Value {
        assert_eq!(answer["ok"], true, "{answer}");
        &answer["result"]
    }

    /// `asked` 没做成，报的是 `code`。
    fn assert_refused(asked: &Value, answer: &Value, code: &str) {
        assert_eq!(answer["ok"], false, "{asked}: {answer}");
        assert_eq!(answer["code"], code, "{asked}: {answer}");
    }

    /// 自动化操作（§5.3）：`pid_snapshot_save` 同 `PIDSNAPSHOT SAVE`，结果是序号、文件、对象数、
    /// 字节数和整个文件的 SHA-256，与列表里记的一致；带 `path` 就是 `FILE` 形式，不编序号（`seq`
    /// 是 `null`），不是完整路径的从图所在的目录算起。图的「已修改」和撤销栈都不变。
    #[test]
    fn pid_snapshot_save_answers_the_seq_and_the_file_it_wrote() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let i = app.active_tab;
        let (dirty, steps) = (app.tabs[i].dirty, undo_steps(&app));
        let answer = request(
            &mut app,
            json!({"op": "pid_snapshot_save", "reason": "送审前"}),
        );
        let saved = result_of(&answer);
        let file = snapshots(&scratch).join("0001.json");
        let listed = store::list(&snapshots(&scratch)).expect("快照列表");
        assert_eq!(saved["seq"], 1, "{answer}");
        assert_eq!(saved["path"], file.display().to_string(), "{answer}");
        assert_eq!(saved["objects"], listed[0].objects, "{answer}");
        let bytes = fs::metadata(&file).expect("快照文件").len();
        assert_eq!(saved["bytes"], bytes, "{answer}");
        assert_eq!(saved["sha256"], listed[0].sha256, "{answer}");
        let first = store::load(&snapshots(&scratch), 1).expect("第 1 份");
        assert_eq!(first.reason, "送审前");
        let again = request(&mut app, json!({"op": "pid_snapshot_save"}));
        assert_eq!(result_of(&again)["seq"], 2, "{again}");

        let answer = request(
            &mut app,
            json!({"op": "pid_snapshot_save", "path": "spare.json", "reason": "备份"}),
        );
        let saved = result_of(&answer);
        let spare = scratch.path("spare.json");
        assert_eq!(saved["seq"], Value::Null, "{answer}");
        assert_eq!(saved["path"], spare.display().to_string(), "{answer}");
        let written = store::load_file(&spare).expect("FILE 形式的快照");
        assert_eq!((written.seq, written.reason.as_str()), (None, "备份"));
        assert_eq!(saved["objects"], written.counts.objects, "{answer}");
        let listed = store::list(&snapshots(&scratch)).expect("快照列表");
        assert_eq!(listed.len(), 2);
        assert_eq!(app.tabs[i].dirty, dirty);
        assert_eq!(undo_steps(&app), steps);
    }

    /// `pid_snapshots` 是当前图的快照列表：图号、快照目录，每份的序号、时间、生成人、原因、对象数、
    /// 字节数和 `status`（文件不在了是 `missing`），另带文件名和整个文件的 SHA-256。
    #[test]
    fn pid_snapshots_lists_each_snapshot_and_marks_the_missing_ones() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let answer = request(&mut app, json!({"op": "pid_snapshots"}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["drawing_id"], "PID-1101", "{answer}");
        let dir = snapshots(&scratch).display().to_string();
        assert_eq!(answer["dir"], dir, "{answer}");
        assert_eq!(answer["snapshots"], json!([]), "{answer}");
        for reason in ["一", "二"] {
            let saved = request(
                &mut app,
                json!({"op": "pid_snapshot_save", "reason": reason}),
            );
            assert_eq!(saved["ok"], true, "{saved}");
        }
        fs::remove_file(snapshots(&scratch).join("0002.json")).expect("只读的文件也删得掉");
        let answer = request(&mut app, json!({"op": "pid_snapshots"}));
        assert_eq!(answer["ok"], true, "{answer}");
        let listed = store::list(&snapshots(&scratch)).expect("快照列表");
        assert_eq!(answer["snapshots"], json!(listed), "{answer}");
        let first = &answer["snapshots"][0];
        for key in [
            "seq",
            "created_at",
            "created_by",
            "reason",
            "objects",
            "bytes",
            "status",
            "file",
            "sha256",
        ] {
            assert!(!first[key].is_null(), "{key}: {first}");
        }
        assert_eq!(first["created_by"], app.waiver_by(), "{first}");
        assert_eq!(first["reason"], "一", "{first}");
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(answer["snapshots"][1]["status"], "missing", "{answer}");
    }

    /// `pid_snapshot` 应答第几份快照的全文（§3.2），`ok` 在最前；读的时候照常校验：序号不在报
    /// `SNAPSHOT_NOT_FOUND`，与校验和对不上报 `SNAPSHOT_CORRUPT`；`seq` 不是整数是
    /// `invalid_request`。
    #[test]
    fn pid_snapshot_answers_the_whole_snapshot_and_checks_it_on_the_way() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let saved = request(
            &mut app,
            json!({"op": "pid_snapshot_save", "reason": "送审前"}),
        );
        assert_eq!(saved["ok"], true, "{saved}");
        let answer = request(&mut app, json!({"op": "pid_snapshot", "seq": 1}));
        assert_eq!(answer["ok"], true, "{answer}");
        let bytes = fs::read(snapshots(&scratch).join("0001.json")).expect("第 1 份");
        let file: Value = serde_json::from_slice(&bytes).expect("快照是 JSON");
        let mut whole = answer.clone();
        whole.as_object_mut().expect("应答是对象").remove("ok");
        assert_eq!(whole, file);
        let keys: Vec<&str> = answer
            .as_object()
            .expect("应答是对象")
            .keys()
            .take(3)
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["ok", "version", "kind"]);

        let mut tampered = file;
        tampered["seq"] = json!(2);
        tampered["body_sha256"] = json!("0".repeat(64));
        let bad = snapshots(&scratch).join("0002.json");
        fs::write(bad, tampered.to_string()).expect("写一份坏的");
        for (asked, code) in [
            (json!({"op": "pid_snapshot", "seq": 9}), SNAPSHOT_NOT_FOUND),
            (json!({"op": "pid_snapshot", "seq": 2}), SNAPSHOT_CORRUPT),
            (json!({"op": "pid_snapshot"}), "invalid_request"),
            (json!({"op": "pid_snapshot", "seq": "1"}), "invalid_request"),
            (json!({"op": "pid_snapshot", "seq": -1}), "invalid_request"),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_refused(&asked, &answer, code);
        }
    }

    /// `pid_snapshot_export` 同 `PIDSNAPSHOT EXPORT`：原样复制，结果是写到哪里、几个字节；目标已在
    /// 报 `SNAPSHOT_IMMUTABLE`，序号不在报 `SNAPSHOT_NOT_FOUND`，都不写；`seq`、`path` 都要给。
    #[test]
    fn pid_snapshot_export_copies_byte_for_byte_and_never_overwrites() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        let saved = request(&mut app, json!({"op": "pid_snapshot_save"}));
        assert_eq!(saved["ok"], true, "{saved}");
        let answer = request(
            &mut app,
            json!({"op": "pid_snapshot_export", "seq": 1, "path": "copy.json"}),
        );
        let copied = result_of(&answer);
        let copy = scratch.path("copy.json");
        let original = fs::read(snapshots(&scratch).join("0001.json")).expect("第 1 份");
        assert_eq!(copied["path"], copy.display().to_string(), "{answer}");
        assert_eq!(copied["bytes"], original.len(), "{answer}");
        assert_eq!(fs::read(&copy).expect("复制出来的"), original);
        for (asked, code) in [
            (
                json!({"op": "pid_snapshot_export", "seq": 1, "path": "copy.json"}),
                SNAPSHOT_IMMUTABLE,
            ),
            (
                json!({"op": "pid_snapshot_export", "seq": 9, "path": "nine.json"}),
                SNAPSHOT_NOT_FOUND,
            ),
            (
                json!({"op": "pid_snapshot_export", "seq": 1}),
                "invalid_request",
            ),
            (
                json!({"op": "pid_snapshot_export", "path": "other.json"}),
                "invalid_request",
            ),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_refused(&asked, &answer, code);
        }
        assert_eq!(fs::read(&copy).expect("复制出来的"), original);
        for name in ["nine.json", "other.json"] {
            assert!(!scratch.path(name).exists(), "{name}");
        }
    }

    /// `pid_diff` 同 `PIDDIFF`：`a`、`b` 写序号（整数或全是数字的文字）、快照文件或 `live`，`b`
    /// 默认 `live`，`ref` 照写的填；`move_tolerance` 交给比对（§3.6），不给是 0.001。两边是同一份报
    /// `DIFF_SAME_SNAPSHOT`；写错了是 `invalid_request`。
    #[test]
    fn pid_diff_compares_with_the_drawing_by_default_and_passes_move_tolerance_on() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        // 挑有位号的来挪（§3.5）：位号空的挪了是删一个、增一个。这台泵不在线上，挪了不牵动连接。
        let spare_pump = [60.0, -80.0];
        draw(&mut app, "PIDINSERT pump", &[spare_pump]);
        app.finish_all_pending_history();
        let saved = request(&mut app, json!({"op": "pid_snapshot_save"}));
        assert_eq!(saved["ok"], true, "{saved}");
        edit(&mut app);
        let counts = |answer: &Value| {
            let summary = &answer["summary"];
            [&summary["新增"], &summary["删除"], &summary["修改"]].map(Value::clone)
        };
        let answer = request(&mut app, json!({"op": "pid_diff", "a": 1}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["a"]["ref"], "1", "{answer}");
        assert_eq!(answer["a"]["seq"], 1, "{answer}");
        assert_eq!(answer["b"]["ref"], "live", "{answer}");
        assert_eq!(answer["b"]["seq"], Value::Null, "{answer}");
        assert_eq!(answer["identical"], false, "{answer}");
        assert_eq!(counts(&answer), [json!(1), json!(1), json!(1)], "{answer}");
        let saved = request(&mut app, json!({"op": "pid_snapshot_save"}));
        assert_eq!(saved["ok"], true, "{saved}");
        let answer = request(&mut app, json!({"op": "pid_diff", "a": "1", "b": "2"}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["a"]["ref"], "1", "{answer}");
        assert_eq!(answer["b"]["ref"], "2", "{answer}");
        assert_eq!(counts(&answer), [json!(1), json!(1), json!(1)], "{answer}");
        let answer = request(&mut app, json!({"op": "pid_diff", "a": 2, "b": "LIVE"}));
        assert_eq!(answer["identical"], true, "{answer}");

        let pump = object_at(&app, ObjectKind::Equipment, spare_pump);
        assert!(!pump.tag.is_empty(), "挪的泵要有位号，挪了才认得是同一个");
        let moved = request(
            &mut app,
            json!({"op": "entities_transform", "handles": [pump.handle], "vector": [0.5, 0.0, 0.0]}),
        );
        assert_eq!(moved["ok"], true, "{moved}");
        for (tolerance, identical) in [(None, false), (Some(0.1), false), (Some(1.0), true)] {
            let mut asked = json!({"op": "pid_diff", "a": 2});
            if let Some(tolerance) = tolerance {
                asked["move_tolerance"] = json!(tolerance);
            }
            let answer = request(&mut app, asked.clone());
            assert_eq!(answer["ok"], true, "{asked}: {answer}");
            assert_eq!(answer["identical"], identical, "{asked}: {answer}");
            if !identical {
                let moved = &answer["summary"]["by_what"]["位置"];
                assert_eq!(*moved, 1, "{asked}: {answer}");
            }
        }

        let saved = request(
            &mut app,
            json!({"op": "pid_snapshot_save", "path": "spare.json"}),
        );
        assert_eq!(saved["ok"], true, "{saved}");
        let spare = scratch.path("spare.json").display().to_string();
        for (asked, code) in [
            (
                json!({"op": "pid_diff", "a": 2, "b": "2"}),
                DIFF_SAME_SNAPSHOT,
            ),
            (json!({"op": "pid_diff", "a": "live"}), DIFF_SAME_SNAPSHOT),
            (
                json!({"op": "pid_diff", "a": "spare.json", "b": spare}),
                DIFF_SAME_SNAPSHOT,
            ),
            (json!({"op": "pid_diff", "a": 9}), SNAPSHOT_NOT_FOUND),
            (
                json!({"op": "pid_diff", "a": "none.json"}),
                SNAPSHOT_NOT_FOUND,
            ),
            (json!({"op": "pid_diff"}), "invalid_request"),
            (json!({"op": "pid_diff", "a": ""}), "invalid_request"),
            (
                json!({"op": "pid_diff", "a": 1, "b": true}),
                "invalid_request",
            ),
            (
                json!({"op": "pid_diff", "a": 1, "move_tolerance": -1}),
                "invalid_request",
            ),
            (
                json!({"op": "pid_diff", "a": 1, "move_tolerance": "1"}),
                "invalid_request",
            ),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_refused(&asked, &answer, code);
        }
    }

    /// 只读会话里五个操作照常能用（写的是本机文件，不是图纸，§5.4）；自动化停了以后三个读的照常
    /// 能用，存快照同别的写操作被拒。
    #[test]
    fn pid_snapshot_ops_work_in_a_readonly_session_and_the_reads_with_automation_stopped() {
        let scratch = Scratch::new();
        let mut app = saved_plant(&scratch);
        app.collab.readonly = true;
        for asked in [
            json!({"op": "pid_snapshot_save", "reason": "只读"}),
            json!({"op": "pid_snapshot_save", "path": "spare.json"}),
            json!({"op": "pid_snapshots"}),
            json!({"op": "pid_snapshot", "seq": 1}),
            json!({"op": "pid_snapshot_export", "seq": 1, "path": "copy.json"}),
            json!({"op": "pid_diff", "a": 1}),
            json!({"op": "pid_diff", "a": "spare.json", "b": 1}),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_eq!(answer["ok"], true, "{asked}: {answer}");
        }
        for name in ["spare.json", "copy.json"] {
            assert!(scratch.path(name).is_file(), "{name}");
        }
        let stopped = request(&mut app, json!({"op": "stop"}));
        assert_eq!(stopped["ok"], true, "{stopped}");
        for asked in [
            json!({"op": "pid_snapshots"}),
            json!({"op": "pid_snapshot", "seq": 1}),
            json!({"op": "pid_diff", "a": 1}),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_eq!(answer["ok"], true, "{asked}: {answer}");
        }
        let asked = json!({"op": "pid_snapshot_save"});
        let answer = request(&mut app, asked.clone());
        assert_refused(&asked, &answer, "disabled");
    }

    /// 没保存过的图没有放带序号快照的目录，要用目录的操作都报 `INDEX_NO_PROJECT_DIR`，说可以给
    /// `path`；`pid_snapshot_save` 带 `path` 照常写成文件，拿它和当前图比也行。
    #[test]
    fn pid_snapshot_ops_on_an_unsaved_drawing_need_a_path() {
        let scratch = Scratch::new();
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        plant(&mut app);
        let copy = scratch.path("copy.json").display().to_string();
        for asked in [
            json!({"op": "pid_snapshot_save"}),
            json!({"op": "pid_snapshots"}),
            json!({"op": "pid_snapshot", "seq": 1}),
            json!({"op": "pid_snapshot_export", "seq": 1, "path": copy}),
            json!({"op": "pid_diff", "a": 1}),
        ] {
            let answer = request(&mut app, asked.clone());
            assert_refused(&asked, &answer, INDEX_NO_PROJECT_DIR);
            let error = answer["error"].as_str().unwrap_or_default();
            assert!(
                error.contains("没保存过") && error.contains("path"),
                "{asked}: {answer}"
            );
        }
        assert!(!scratch.path("copy.json").exists());
        let file = scratch.path("unsaved.json").display().to_string();
        let answer = request(&mut app, json!({"op": "pid_snapshot_save", "path": file}));
        let saved = result_of(&answer);
        assert_eq!(saved["seq"], Value::Null, "{answer}");
        assert_eq!(saved["path"], file, "{answer}");
        let answer = request(&mut app, json!({"op": "pid_diff", "a": file}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["identical"], true, "{answer}");
    }

    /// 状态里的能力清单有 `pid_snapshot`。
    #[test]
    fn pid_snapshot_is_among_the_capabilities() {
        let mut app = OpenCADStudio::new_for_test();
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let capabilities = state["capabilities"].as_array().expect("能力清单");
        assert!(capabilities.contains(&json!("pid_snapshot")), "{state}");
    }
}

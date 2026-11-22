//! PIDPUBLISH：把当前图发布成带修订号的包（`SPEC-pid-publish.md` §4、§5.2）。
//!
//! ```text
//! PIDPUBLISH [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]
//!                        发布当前图：前提和门禁都过了就存快照、出包
//! PIDPUBLISH CHECK [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]
//!                        只看能不能发布：前提和四项门禁，不写任何东西
//! PIDPUBLISH LIST [<目标目录>]
//!                        这张图发布到哪一版了，和已发布的各版
//! PIDPUBLISH VERIFY <包目录>
//!                        校验一个包
//! ```
//!
//! 目标目录不写时是图纸所在目录下的 `pid-publish`，相对路径从图纸所在目录算，有空格的加双引号；
//! 修订号不写时是目标目录里这张图已有的最大修订号加一（§3.1）。紧跟在 `PIDPUBLISH` 后面、没加
//! 引号的 `CHECK`、`LIST`、`VERIFY` 是子命令，加了引号是目录名。
//!
//! 前提按 §3.4 的先后查，先不满足的那条连同错误码说出来，不往下做：图保存过、没有没保存的修改
//! （`PUBLISH_UNSAVED`）；磁盘上的文件就是本程序上次读写的那一份——现取的指纹与标签页记的相同，
//! 标签页没记到指纹也不算（`PUBLISH_FILE_CHANGED`）；图里有同步记录时的三条协同前提（离线规格
//! §5.7，`COLLAB_UNSYNCED`、`COLLAB_DIVERGED`、`COLLAB_OFFLINE`，`ALLOW_OFFLINE` 只放过最后
//! 一条）；当前图在项目的图纸里（`INDEX_NO_PROJECT_DIR`）；目标目录能用
//! （`PUBLISH_TARGET_INVALID`：只看路径，不建也不试写）、修订号合法
//! （`PUBLISH_REVISION_INVALID`）、这一版的包还没有（`PUBLISH_TARGET_EXISTS`）。只读会话的
//! 发布由命令闸门拒（`COLLAB_READONLY`，`collab::apply`）；`CHECK` 照查，最后说这个会话发不了。
//!
//! 前提都满足后取规则和主数据，扫项目（`commands::pidindex`：要读盘就在后台读，读完接着做），
//! 扫完再过一遍前提——读盘时图可能又改过——然后在扫好的项目上算门禁（[`publish_gates`]）：当前
//! 图的摘要就是这次扫描做的那一份，只做一次；现做一次检查，跨图，触发时机「发布前」，不用也不
//! 更新 `PIDCHECK` 记着的结果；数重复位号；四项门禁。说出四项的计数、前 10 条没豁免的错误、
//! 重复位号的各处和读不出来的图，最后说能不能发布（不能时错误码 `PUBLISH_BLOCKED`）。
//!
//! 发布而门禁都过了，接着出包（§4）：用门禁查的那份摘要存一份快照，原因「发布前自动快照（修订
//! N）」；在目标目录里建临时目录，先清掉以前崩溃留下的；用 `plot` 操作的出图出 PDF 和 SVG，不给
//! 出图日期字段盖时间；从刚存的快照算材料清单；写包，自检后改名落位。说包在哪里、几个文件、多大，
//! 和各步用时。存快照以后哪一步失败，都删掉临时目录和这次建的目标目录，报 `PUBLISH_FAILED` 并说
//! 是哪一步（`step`），快照留着。发布不改图：「已修改」、撤销栈、磁盘上的源文件都不变。
//!
//! `LIST` 不扫项目，只读目标目录里这张图的包（[`publications`]）：先说发布到哪一版了（§4 末尾的
//! 表），再新的在前一行一个包，与图纸文件一致的标「当前」。`VERIFY` 校验一个包（§3.5），相对路径从
//! 图纸所在目录算。两者都不写任何东西，只读会话里也能用。

use super::pidbom::Sheets;
use super::piddrc::{self, Check};
use super::pidindex::{point, same_file, Scanned, INDEX_NO_PROJECT_DIR};
use super::pidsnapshot::utc_said;
use super::*;
use crate::pid::collab_state;
use crate::pid::drc::{CheckReport, CheckResult, RuleSet, Severity};
use crate::pid::master_data::{self, Resolved};
use crate::pid::project_index::{self, graph, Duplicate};
use crate::pid::publish_pkg::{
    self, CollabInput, CollabRecord, CollabVerdict, Gates, MasterDataState, PackageInput,
    ProjectRef, PublishError, PUBLISH_BLOCKED, PUBLISH_FAILED, PUBLISH_TARGET_EXISTS,
};
use crate::pid::snapshot::{capture, store, Snapshot, SnapshotMeta};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 图没保存过，或者有没保存的修改（§3.4，HTTP 409）。
pub(in crate::app) const PUBLISH_UNSAVED: &str = "PUBLISH_UNSAVED";
/// 磁盘上的文件不是本程序上次读写的那一份，或者标签页没记到它的指纹（§3.4，HTTP 409）。
pub(in crate::app) const PUBLISH_FILE_CHANGED: &str = "PUBLISH_FILE_CHANGED";
/// 目标目录建不了、写不了（§3.6，HTTP 400）。
pub(in crate::app) const PUBLISH_TARGET_INVALID: &str = "PUBLISH_TARGET_INVALID";
/// 指定的修订号不是正整数，或者不大于已有的最大修订号（§3.6，HTTP 400）。
pub(in crate::app) const PUBLISH_REVISION_INVALID: &str = "PUBLISH_REVISION_INVALID";
/// 扫出来的项目里没有当前图：项目没读成（项目索引规格的码，HTTP 500）。
const INDEX_IO_ERROR: &str = "INDEX_IO_ERROR";

/// 不写目标目录时用的：图纸所在目录下的这个子目录（§3.1）。
const DEFAULT_TARGET: &str = "pid-publish";
/// 命令行上列出的没豁免的错误条数。
const LISTED: usize = 10;
/// 写错时说的写法。
const USAGE: &str =
    "写成 PIDPUBLISH [CHECK] [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]，目录有空格就加双引号。";
const LIST_USAGE: &str = "写成 PIDPUBLISH LIST [<目标目录>]，目录有空格就加双引号。";
const VERIFY_USAGE: &str = "写成 PIDPUBLISH VERIFY <包目录>，目录有空格就加双引号。";
/// 出包用的临时目录的名字的开头（§4）。以 `.` 开头：数修订号、列包都不看它；清崩溃留下的临时
/// 目录也只认这个开头。
const STAGING_PREFIX: &str = ".publishing-";

/// 命令行上写的：目标目录和修订号都是原样（合不合法在前提里查），带没带 `ALLOW_OFFLINE`。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Asked {
    dir: Option<String>,
    revision: Option<String>,
    allow_offline: bool,
}

/// 前提都满足时，要发到哪里：源文件、项目目录、图号、目标目录、这一版的修订号，和协同前提的
/// 结论。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    /// 标签页上的图存在磁盘上的那个文件，原样拷进包里。
    source: PathBuf,
    project_dir: PathBuf,
    drawing_id: String,
    dir: PathBuf,
    revision: u32,
    collab: CollabVerdict,
}

/// 要发到哪里（§3.1、§3.6，[`destination`]）：项目目录、图号、目标目录和这一版的修订号。
struct Destination {
    project_dir: PathBuf,
    drawing_id: String,
    dir: PathBuf,
    revision: u32,
}

/// 扫完项目接着做的发布或检查（`commands::pidindex` 的扫描排着的一件）：哪张图（标签页的
/// `id`，扫的时候它可能被关掉）、命令行上写的，和起头时取的规则、主数据（`SPEC-pid-drc.md`
/// §4.1 的先后，摘要的连接容差照这套规则）。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PublishJob {
    only_check: bool,
    tab: u64,
    asked: Asked,
    rules: RuleSet,
    master: Resolved,
}

impl PublishJob {
    /// 说话时打头的命令。
    pub(super) fn verb(&self) -> &'static str {
        verb_of(self.only_check)
    }

    /// 做摘要用的连接容差：规则包的。
    pub(super) fn tolerance(&self) -> f64 {
        self.rules.connect_tolerance
    }
}

/// 门禁这一段算出来的：现做的检查报告、重复位号和四项门禁。
#[derive(Debug)]
pub(super) struct Gated {
    pub(super) report: CheckReport,
    pub(super) duplicates: Vec<Duplicate>,
    pub(super) gates: Gates,
}

/// 出包要用的（§4）：第几个标签页、扫好的项目、前提给的目标、主数据、门禁这一段的结果、这次发布
/// 的时间（Unix 秒），和出图设置（§5.3 `pid_publish` 的 `plot`；命令行上发布是 `null`，用默认的）。
struct Packing<'a> {
    i: usize,
    scanned: &'a Scanned,
    target: &'a Target,
    master: &'a Resolved,
    gated: &'a Gated,
    now: u64,
    plot: &'a Value,
}

/// 出包成了：包的目录、里面几个文件、共几个字节，和发布前的快照的序号。
struct Published {
    dir: PathBuf,
    files: usize,
    bytes: u64,
    seq: u64,
}

/// 出包没成：错在哪里，和留下了什么。
struct Unpublished {
    failed: Failed,
    kept: String,
}

/// `pid_publish` 发得了时（[`OpenCADStudio::pidpublish_ready`]）：出图设置、前提给的目标和规则包。
struct Ready {
    plot: Value,
    target: Target,
    rules: RuleSet,
}

/// 出包没成：错误码，`PUBLISH_FAILED` 时是哪一步（§3.6 的 `step`），和为什么。
struct Failed {
    code: &'static str,
    step: Option<&'static str>,
    why: String,
}

impl Failed {
    /// 出包的 `step` 这一步失败了（`PUBLISH_FAILED`）。
    fn at(step: &'static str, why: impl Into<String>) -> Self {
        Self {
            code: PUBLISH_FAILED,
            step: Some(step),
            why: why.into(),
        }
    }

    /// 说给人听：`PUBLISH_FAILED` 先说是在哪一步失败的。
    fn said(&self) -> String {
        match self.step {
            Some(step) => format!(
                "出包在「{}」这一步失败（step {step}）：{}",
                step_said(step),
                self.why
            ),
            None => self.why.clone(),
        }
    }
}

impl From<PublishError> for Failed {
    fn from(error: PublishError) -> Self {
        Self {
            code: error.code(),
            step: error.step(),
            why: why_of(&error),
        }
    }
}

/// 出包各步的用时，按做的先后。
#[derive(Default)]
struct Timings(Vec<(&'static str, Duration)>);

impl Timings {
    /// 记下 `step` 这一步：从 `started` 到现在。
    fn add(&mut self, step: &'static str, started: Instant) {
        self.0.push((step, started.elapsed()));
    }

    /// 说给人听：「快照 3 ms，出图 120 ms，……，共 150 ms」。
    fn said(&self) -> String {
        let total: Duration = self.0.iter().map(|(_, took)| *took).sum();
        let steps: Vec<String> = self
            .0
            .iter()
            .map(|(step, took)| format!("{step} {} ms", took.as_millis()))
            .collect();
        format!("{}，共 {} ms", steps.join("，"), total.as_millis())
    }
}

/// 出包用的临时目录（§4），建在目标目录里；`created` 是为它新建的目标目录和上级，从里往外，
/// 丢弃时一起删掉。
struct Staging {
    path: PathBuf,
    created: Vec<PathBuf>,
}

impl Staging {
    /// 在目标目录 `dir` 里给包 `package` 建临时目录：名字以 [`STAGING_PREFIX`] 开头，后缀是现在的
    /// 纳秒数和进程号。目标目录还没有就建出来，先清掉以前崩溃留下的临时目录（[`clear_leftovers`]）。
    /// 建不了时说为什么，这次建的目标目录删掉。
    fn prepare(dir: &Path, package: &str) -> Result<Self, String> {
        let created: Vec<PathBuf> = dir
            .ancestors()
            .take_while(|at| !at.as_os_str().is_empty() && fs::symlink_metadata(at).is_err())
            .map(Path::to_path_buf)
            .collect();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!("{STAGING_PREFIX}{package}-{nanos}-{}", std::process::id());
        let staging = Self {
            path: dir.join(name),
            created,
        };
        let made = fs::create_dir_all(dir).and_then(|()| {
            clear_leftovers(dir);
            fs::create_dir(&staging.path)
        });
        match made {
            Ok(()) => Ok(staging),
            Err(error) => {
                staging.remove_created();
                Err(format!("在 {} 里建不了临时目录：{error}", dir.display()))
            }
        }
    }

    /// 删掉临时目录，再删这次建的目标目录。临时目录删不掉时说为什么。
    fn discard(self) -> Result<(), String> {
        let removed = fs::remove_dir_all(&self.path).map_err(|error| {
            format!(
                "临时目录 {} 没删掉（{error}），下次往这里发布时清掉",
                self.path.display()
            )
        });
        self.remove_created();
        removed
    }

    /// 这次建的目标目录从里往外删：只删空的，里面有东西的留着。
    fn remove_created(&self) {
        for dir in &self.created {
            let _ = fs::remove_dir(dir);
        }
    }
}

/// 这张图发布到哪一版了（§4 末尾的表）。不记在图里，由目标目录里的包推出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PublishState {
    /// 目标目录里没有这张图的包。
    Unpublished,
    /// 修订号最大的包的源文件就是磁盘上现在这一份，图也没有没保存的修改。
    Published(u32),
    /// 其余：修订号最大的那一版之后图改过，或者有没保存的修改。
    ChangedSince(u32),
}

impl fmt::Display for PublishState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpublished => f.write_str("未发布"),
            Self::Published(revision) => write!(f, "已发布，修订 {revision}"),
            Self::ChangedSince(revision) => write!(f, "修订 {revision} 之后有改动"),
        }
    }
}

/// 目标目录里这张图的一个包，和它的发布清单里记的时间、发布人。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Publication {
    pub(super) package_id: String,
    pub(super) revision: u32,
    /// 出包的时间，Unix 秒；清单读不出来时为 `None`。
    pub(super) published_at: Option<i64>,
    pub(super) published_by: Option<String>,
    /// 与磁盘上现在的图纸文件一致（图也没有没保存的修改）的包里，修订号最大的那一个。
    pub(super) current: bool,
}

/// 一张图的发布记录（§5.3 `pid_publications`）：目标目录、发布到哪一版了，和各包，新的在前。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Publications {
    pub(super) drawing_id: String,
    pub(super) target_dir: PathBuf,
    pub(super) state: PublishState,
    pub(super) packages: Vec<Publication>,
}

impl OpenCADStudio {
    pub(super) fn dispatch_pidpublish(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (word, rest) = first_word(cmd);
        if !word.eq_ignore_ascii_case("PIDPUBLISH") {
            return None;
        }
        let (option, after) = first_word(rest);
        if option.eq_ignore_ascii_case("LIST") {
            self.pidpublish_list(i, after);
            return Some(Task::none());
        }
        if option.eq_ignore_ascii_case("VERIFY") {
            self.pidpublish_verify(i, after);
            return Some(Task::none());
        }
        let only_check = option.eq_ignore_ascii_case("CHECK");
        let verb = verb_of(only_check);
        match asked(if only_check { after } else { rest }) {
            Ok(asked) => Some(self.pidpublish_start(i, only_check, asked)),
            Err(why) => {
                self.command_line
                    .push_error(&format!("{verb}: {why}；{USAGE}"));
                Some(Task::none())
            }
        }
    }

    /// 起一次发布或检查（§4）：过前提，取规则和主数据，扫第 `i` 个标签页的图所在的项目，扫完
    /// 接着算门禁（[`Self::pidpublish_scanned`]）。要从磁盘读的图在后台读，这里先交差。
    fn pidpublish_start(&mut self, i: usize, only_check: bool, asked: Asked) -> Task<Message> {
        let verb = verb_of(only_check);
        let target = match self.pidpublish_preconditions(i, &asked) {
            Ok(target) => target,
            Err((code, why)) => {
                self.pidpublish_refuse(verb, code, &why);
                return Task::none();
            }
        };
        let rules = match self.pidcheck_load_rules(i) {
            Ok(rules) => rules,
            Err((code, why)) => {
                self.pidpublish_refuse(verb, code, &why);
                return Task::none();
            }
        };
        let master = master_data::resolve(&self.tabs[i].scene.document, Some(&target.project_dir));
        let job = PublishJob {
            only_check,
            tab: self.tabs[i].id,
            asked,
            rules,
            master,
        };
        self.pidindex_publish(i, job)
    }

    /// 扫完项目接着做（§4）：前提再过一遍，在扫好的项目上算门禁（[`publish_gates`]），说出来；是发布
    /// 而门禁都过了，接着出包（[`Self::pidpublish_package`]）。
    pub(super) fn pidpublish_scanned(&mut self, scanned: &Scanned, job: PublishJob) {
        let verb = job.verb();
        let Some(i) = self.tabs.iter().position(|tab| tab.id == job.tab) else {
            self.command_line
                .push_error(&format!("{verb}: 要发布的图已经关掉了。"));
            return;
        };
        let target = match self.pidpublish_preconditions(i, &job.asked) {
            Ok(target) => target,
            Err((code, why)) => {
                self.pidpublish_refuse(verb, code, &why);
                return;
            }
        };
        let gated = match publish_gates(scanned, &target.drawing_id, job.rules, &job.master) {
            Ok(gated) => gated,
            Err(why) => {
                self.command_line.push_error(&format!("{verb}: {why}。"));
                return;
            }
        };
        self.pidpublish_say(verb, scanned, &target, &gated);
        if self.pidpublish_verdict(verb, job.only_check, &gated.gates) {
            self.pidpublish_package(&Packing {
                i,
                scanned,
                target: &target,
                master: &job.master,
                gated: &gated,
                now: unix_now(),
                plot: &Value::Null,
            });
        }
    }

    /// 发布的前提（§3.4，离线规格 §5.7），按先后查第 `i` 个标签页的图：先不满足的那条是错误码
    /// 和为什么；都满足时是要发到哪里（[`destination`]）。只读会话不在这里查（命令闸门和
    /// `pid_publish` 自己拒发布，`CHECK` 照查）。不写任何东西。
    fn pidpublish_preconditions(
        &self,
        i: usize,
        asked: &Asked,
    ) -> Result<Target, (&'static str, String)> {
        let tab = &self.tabs[i];
        let Some(path) = tab.current_path.as_deref() else {
            return Err((PUBLISH_UNSAVED, "图还没保存过；存盘后再发布".to_owned()));
        };
        if tab.dirty {
            return Err((PUBLISH_UNSAVED, "图有没保存的修改；存盘后再发布".to_owned()));
        }
        if !self.pidpublish_file_unchanged(i, path) {
            return Err((
                PUBLISH_FILE_CHANGED,
                format!(
                    "磁盘上的 {} 不是本程序上次读写的那一份（被别的程序改过，或者没记下它的样子）；\
                     重新打开或者再存一次后发布",
                    path.display()
                ),
            ));
        }
        let collab = CollabInput {
            allow_offline: asked.allow_offline,
            ..self.publish_collab_input(i)
        };
        let verdict = publish_pkg::collab_preconditions(collab);
        if let CollabVerdict::Refused(code) = verdict {
            return Err((code, collab_said(code, collab.queued)));
        }
        let Destination {
            project_dir,
            drawing_id,
            dir,
            revision,
        } = destination(path, asked)?;
        Ok(Target {
            source: path.to_path_buf(),
            project_dir,
            drawing_id,
            dir,
            revision,
            collab: verdict,
        })
    }

    /// 磁盘上的 `path` 是不是第 `i` 个标签页上次读写的那一份：现取的指纹与标签页记的相同。标签页
    /// 没记到指纹、指纹取不到都不算。
    #[cfg(not(target_arch = "wasm32"))]
    fn pidpublish_file_unchanged(&self, i: usize, path: &Path) -> bool {
        self.tabs[i]
            .disk_fingerprint
            .as_ref()
            .is_some_and(|recorded| {
                crate::io::edit_lock::FileFingerprint::capture(path)
                    .is_ok_and(|now| now == *recorded)
            })
    }

    /// 网页版没有本机文件，认不出来。
    #[cfg(target_arch = "wasm32")]
    fn pidpublish_file_unchanged(&self, _i: usize, _path: &Path) -> bool {
        false
    }

    /// 前提没过：连同错误码说出来。
    fn pidpublish_refuse(&mut self, verb: &str, code: &str, why: &str) {
        self.command_line
            .push_error_coded(code, &format!("{verb}: {code}: {why}。"));
    }

    /// 说门禁这一段的结果（§5.2）：发到哪里、第几版，四项门禁的计数，前 10 条没豁免的错误，重复
    /// 位号的各处，读不出来的图。
    fn pidpublish_say(&mut self, verb: &str, scanned: &Scanned, target: &Target, gated: &Gated) {
        self.command_line.push_output(&format!(
            "{verb}: 图 {} 发布为第 {} 版（{}），放在 {}。",
            target.drawing_id,
            target.revision,
            publish_pkg::package_name(&target.drawing_id, target.revision),
            target.dir.display()
        ));
        if target.collab == (CollabVerdict::Pass { reconciled: false }) {
            self.command_line.push_info(&format!(
                "{verb}: 没连着服务器，按 ALLOW_OFFLINE 照现在的样子发布，清单记为没对过账。"
            ));
        }
        let gates = &gated.gates;
        let waived = if gates.waived > 0 {
            format!("，已豁免 {} 条", gates.waived)
        } else {
            String::new()
        };
        self.command_line.push_output(&format!(
            "{verb}: 检查（发布前）错误 {} 条，警告 {} 条，提示 {} 条{waived}；重复位号 {} 个；主数据{}；\
             项目里的图{}。",
            gates.drc_errors,
            gates.drc_warnings,
            gates.drc_hints,
            gates.duplicate_tags,
            master_said(gates.master_data),
            if gates.index_complete {
                "都读出来了"
            } else {
                "有读不出来的"
            }
        ));
        let errors: Vec<&CheckResult> = gated
            .report
            .results
            .iter()
            .filter(|result| !result.waived && result.severity == Severity::Error)
            .collect();
        for result in errors.iter().take(LISTED) {
            self.command_line
                .push_output(&format!("  {}", piddrc::listed(result, false)));
        }
        if errors.len() > LISTED {
            self.command_line
                .push_output(&format!("  还有 {} 条错误没列出。", errors.len() - LISTED));
        }
        for duplicate in &gated.duplicates {
            let places: Vec<String> = duplicate
                .places
                .iter()
                .map(|place| format!("{} {}", place.drawing_id, point(place.at)))
                .collect();
            self.command_line.push_output(&format!(
                "  重复位号 {} {}：{}",
                duplicate.type_code,
                duplicate.tag,
                places.join("、")
            ));
        }
        for drawing in &scanned.index.drawings {
            if let Some(error) = &drawing.error {
                self.command_line.push_output(&format!(
                    "  读不出来的图 {} {}：{error}",
                    drawing.drawing_id,
                    drawing.file.as_deref().unwrap_or_default()
                ));
            }
        }
    }

    /// 最后一句（§5.2）：门禁没过是 `PUBLISH_BLOCKED`，说没过的是哪几项；过了而会话只读，是只读的
    /// 错误码；都过了，`CHECK` 说可以发布。是发布、门禁都过了、会话也不只读时什么都不说，得到
    /// `true`：接着出包。
    fn pidpublish_verdict(&mut self, verb: &str, only_check: bool, gates: &Gates) -> bool {
        let readonly = self
            .readonly_refuses_command("PIDPUBLISH")
            .then(|| self.collab.refusal().0);
        if !gates.passed() {
            if let Some(code) = &readonly {
                self.command_line.push_info(&format!(
                    "{verb}: 这个会话只读（{code}），门禁都过了也发布不了。"
                ));
            }
            let end = if only_check {
                "不能发布"
            } else {
                "没有发布"
            };
            self.command_line.push_error_coded(
                PUBLISH_BLOCKED,
                &format!(
                    "{verb}: {PUBLISH_BLOCKED}: {}，{end}。",
                    blocked_said(gates)
                ),
            );
            false
        } else if let Some(code) = readonly {
            self.command_line.push_error_coded(
                &code,
                &format!("{verb}: {code}: 前提和门禁都过了，但这个会话只读，不能发布。"),
            );
            false
        } else if only_check {
            self.command_line
                .push_output(&format!("{verb}: 前提和门禁都过了，可以发布。"));
            false
        } else {
            true
        }
    }

    /// 出包（§4，[`Self::pidpublish_pack`]）并说出来：包在哪里、几个文件、多大、发布前的快照是第
    /// 几份和各步用时；没成时连同错误码和是哪一步说为什么、留下了什么。发布前的快照出包没成也留着，
    /// 成没成「校验与交付」面板都重读这张图的快照列表。
    fn pidpublish_package(&mut self, packing: &Packing<'_>) {
        let verb = verb_of(false);
        let mut timings = Timings::default();
        match self.pidpublish_pack(packing, &mut timings) {
            Ok(published) => {
                self.command_line.push_output(&format!(
                    "{verb}: 已发布第 {} 版：{}，{} 个文件，共 {} 字节；发布前的快照是第 {} 份。",
                    packing.target.revision,
                    published.dir.display(),
                    published.files,
                    published.bytes,
                    published.seq
                ));
                self.command_line
                    .push_output(&format!("{verb}: 用时：{}。", timings.said()));
            }
            Err(Unpublished { failed, kept }) => {
                self.pidpublish_failed(verb, &timings, &failed, &kept);
            }
        }
        self.pid_verify_snapshots_saved(packing.i);
    }

    /// 出包（§4）：存发布前的快照，在目标目录里建临时目录，出图、算材料清单、写包、自检后落位
    /// （[`Self::pidpublish_build`]），各步的用时记进 `timings`。存了快照以后哪一步失败，都删掉临时
    /// 目录和这次建的目标目录，快照留着。不改图，不碰命令行。
    fn pidpublish_pack(
        &mut self,
        packing: &Packing<'_>,
        timings: &mut Timings,
    ) -> Result<Published, Unpublished> {
        let target = packing.target;
        let started = Instant::now();
        let snapshot = self.pidpublish_snapshot(packing);
        timings.add("快照", started);
        let snapshot = snapshot.map_err(|failed| Unpublished {
            failed,
            kept: "什么都没写".to_owned(),
        })?;
        let seq = snapshot.seq.unwrap_or_default();
        let kept = format!("发布前的快照第 {seq} 份留着");
        let package = publish_pkg::package_name(&target.drawing_id, target.revision);
        let staging = Staging::prepare(&target.dir, &package).map_err(|why| Unpublished {
            failed: Failed {
                code: PUBLISH_TARGET_INVALID,
                step: None,
                why,
            },
            kept: kept.clone(),
        })?;
        match self.pidpublish_build(packing, &snapshot, &staging.path, timings) {
            Ok(dir) => {
                let (files, bytes) = package_size(&dir);
                Ok(Published {
                    dir,
                    files,
                    bytes,
                    seq,
                })
            }
            Err(failed) => {
                let kept = match staging.discard() {
                    Ok(()) => format!("临时目录已删，{kept}"),
                    Err(why) => format!("{why}；{kept}"),
                };
                Err(Unpublished { failed, kept })
            }
        }
    }

    /// 存发布前的快照（§4）：用这次扫描做的当前图摘要——门禁查的就是这一份——原因写「发布前
    /// 自动快照（修订 N）」，存进 `PIDSNAPSHOT` 用的快照目录，再按序号读回来：包里放的就是存下的那一份。
    fn pidpublish_snapshot(&self, packing: &Packing<'_>) -> Result<Snapshot, Failed> {
        let failed = |why: String| Failed::at("snapshot", why);
        let target = packing.target;
        let summary = packing
            .scanned
            .summaries
            .iter()
            .find(|summary| summary.drawing_id == target.drawing_id)
            .ok_or_else(|| failed(format!("扫出来的项目里没有图 {}", target.drawing_id)))?;
        let tab = &self.tabs[packing.i];
        let drawing_dir = crate::io::drawing_dir(&tab.scene.document)
            .ok_or_else(|| failed("图没有所在的目录".to_owned()))?;
        let meta = SnapshotMeta {
            created_at: packing.now,
            created_by: self.waiver_by(),
            reason: format!("发布前自动快照（修订 {}）", target.revision),
            generator: generator(),
            dirty: tab.dirty,
        };
        let snapshot = capture(summary.clone(), graph(summary), meta)
            .map_err(|error| failed(error.to_string()))?;
        let dir = store::dir_for(drawing_dir, &snapshot.drawing_id);
        unless_broken("snapshot", || {
            let saved = store::save(&dir, snapshot).map_err(|error| failed(error.to_string()))?;
            let seq = saved
                .seq
                .ok_or_else(|| failed("存下的快照没有序号".to_owned()))?;
            store::load(&dir, seq).map_err(|error| failed(error.to_string()))
        })
    }

    /// 出包的中段（§4）：在临时目录 `staging` 里出 PDF、SVG 两张图，从快照 `snapshot` 算材料清单，
    /// 写包，再自检、改名落位成目标目录里的 `<图号>-r<修订号>`。各步的用时记进 `timings`。得到包的
    /// 目录。
    fn pidpublish_build(
        &mut self,
        packing: &Packing<'_>,
        snapshot: &Snapshot,
        staging: &Path,
        timings: &mut Timings,
    ) -> Result<PathBuf, Failed> {
        let (i, target) = (packing.i, packing.target);
        let started = Instant::now();
        let plotted = unless_broken("plot", || {
            self.pidpublish_plot(i, &target.drawing_id, staging, packing.plot)
                .map_err(|why| Failed::at("plot", why))
        });
        timings.add("出图", started);
        plotted?;

        let started = Instant::now();
        let seq = snapshot.seq.unwrap_or_default();
        let bom = unless_broken("bom", || {
            self.pidbom_compute(Sheets::Snapshot(i, seq))
                .map(|(bom, _)| bom)
                .map_err(|failure| Failed::at("bom", failure.to_string()))
        });
        timings.add("材料清单", started);
        let bom = bom?;

        let started = Instant::now();
        let project = project_ref(&target.project_dir);
        let collab = collab_record(&self.tabs[i].scene.document, target.collab);
        let published_by = self.waiver_by();
        let generator = generator();
        let input = PackageInput {
            drawing_id: &target.drawing_id,
            revision: target.revision,
            project: project.as_ref(),
            published_at: i64::try_from(packing.now).unwrap_or(i64::MAX),
            published_by: &published_by,
            generator: &generator,
            source: &target.source,
            snapshot,
            bom: &bom,
            index: &packing.scanned.index,
            master_data: match packing.master {
                Resolved::Configured { data, .. } => Some(data),
                _ => None,
            },
            report: &packing.gated.report,
            gates: &packing.gated.gates,
            collab: collab.as_ref(),
        };
        let written = unless_broken("write", || {
            publish_pkg::write_package(staging, &input).map_err(Failed::from)
        });
        timings.add("写包", started);
        let manifest = written?;
        spoil_if_broken(staging);

        let started = Instant::now();
        let dir = target.dir.join(&manifest.package_id);
        let committed = unless_broken("commit", || {
            publish_pkg::commit(staging, &dir).map_err(Failed::from)
        });
        timings.add("自检落位", started);
        committed.map(|()| dir)
    }

    /// 出图（§3.1）：把第 `i` 个标签页的图出成 PDF 和 SVG，按 [`publish_pkg::plot_files`] 的名字写进
    /// `staging`。走 `plot` 操作的出图，设置是 `plot`（字段同 `plot` 操作；`null` 是模型空间、图形范围、
    /// 布满图纸），但不给出图日期字段盖时间：发布不改图。出图只认当前标签页，出的时候临时把
    /// 这一页当成当前的，出完换回来。
    #[cfg(not(target_arch = "wasm32"))]
    fn pidpublish_plot(
        &mut self,
        i: usize,
        drawing_id: &str,
        staging: &Path,
        plot: &Value,
    ) -> Result<(), String> {
        let active = std::mem::replace(&mut self.active_tab, i);
        let mut plotted = Ok(());
        for name in publish_pkg::plot_files(drawing_id) {
            let mut request = plot.clone();
            request["path"] = json!(staging.join(&name).to_string_lossy());
            if let Err(error) = self.plot_current(&request) {
                plotted = Err(format!(
                    "{name} 出不来：{}（{}）",
                    error["error"].as_str().unwrap_or_default(),
                    error["code"].as_str().unwrap_or_default()
                ));
                break;
            }
        }
        self.active_tab = active;
        plotted
    }

    /// 网页版不能出图到本机文件。
    #[cfg(target_arch = "wasm32")]
    fn pidpublish_plot(
        &mut self,
        _i: usize,
        _drawing_id: &str,
        _staging: &Path,
        _plot: &Value,
    ) -> Result<(), String> {
        Err("网页版不能出图到本机文件".to_owned())
    }

    /// 出包没成：先说做了的几步各用了多久，再连同错误码说为什么——`PUBLISH_FAILED` 带是哪一步——
    /// 和留下了什么（`kept`）。
    fn pidpublish_failed(&mut self, verb: &str, timings: &Timings, failed: &Failed, kept: &str) {
        self.command_line
            .push_output(&format!("{verb}: 用时：{}。", timings.said()));
        let code = failed.code;
        let what = failed.said();
        self.command_line
            .push_error_coded(code, &format!("{verb}: {code}: {what}；{kept}。"));
    }

    /// `PIDPUBLISH LIST [<目标目录>]`：说这张图发布到哪一版了，再新的在前一行一个包。不扫项目，
    /// 不写任何东西。
    fn pidpublish_list(&mut self, i: usize, rest: &str) {
        let verb = "PIDPUBLISH LIST";
        let dir = match only_folder(rest) {
            Ok(dir) => dir,
            Err(why) => {
                self.command_line
                    .push_error(&format!("{verb}: {why}；{LIST_USAGE}"));
                return;
            }
        };
        let listed = match self.pidpublish_listed(i, dir) {
            Ok(listed) => listed,
            Err((code, why)) => {
                self.pidpublish_refuse(verb, code, &why);
                return;
            }
        };
        let target = listed.target_dir.display();
        if listed.packages.is_empty() {
            self.command_line.push_output(&format!(
                "{verb}: 图 {}：{}（{target} 里没有这张图的包）。",
                listed.drawing_id, listed.state
            ));
            return;
        }
        self.command_line.push_output(&format!(
            "{verb}: 图 {}：{}；{target} 里有 {} 个包，新的在前：",
            listed.drawing_id,
            listed.state,
            listed.packages.len()
        ));
        for package in &listed.packages {
            self.command_line.push_output(&publication_said(package));
        }
    }

    /// 第 `i` 个标签页的图的发布记录（[`publications`]）。目标目录 `dir` 与发布的算法相同：从图纸
    /// 所在目录算，不写是 `pid-publish`。图没保存过、不在项目的图纸里时是错误码和为什么。
    pub(super) fn pidpublish_listed(
        &self,
        i: usize,
        dir: Option<&str>,
    ) -> Result<Publications, (&'static str, String)> {
        let tab = &self.tabs[i];
        let Some(path) = tab.current_path.as_deref() else {
            return Err((PUBLISH_UNSAVED, "图还没保存过，没有发布记录".to_owned()));
        };
        let (_, drawing_id, target_dir) = placed(path, dir)?;
        Ok(publications(&target_dir, &drawing_id, path, tab.dirty))
    }

    /// `PIDPUBLISH VERIFY <包目录>`：校验一个包（§3.5），相对路径从图纸所在目录算，图没保存过时从
    /// 当前目录算。不写任何东西。
    fn pidpublish_verify(&mut self, i: usize, rest: &str) {
        let verb = "PIDPUBLISH VERIFY";
        let name = match only_folder(rest) {
            Ok(Some(name)) => name,
            Ok(None) => {
                self.command_line
                    .push_error(&format!("{verb}: 要写包目录；{VERIFY_USAGE}"));
                return;
            }
            Err(why) => {
                self.command_line
                    .push_error(&format!("{verb}: {why}；{VERIFY_USAGE}"));
                return;
            }
        };
        let dir = self.pidpublish_package_dir(i, name);
        match publish_pkg::verify_package(&dir) {
            Ok(report) => {
                self.command_line.push_output(&format!(
                    "{verb}: {} 完好：{}，第 {} 版，共 {} 个文件，清单列的都在，字节数和校验和都对。",
                    dir.display(),
                    report.package_id,
                    report.revision,
                    report.files
                ));
                if !report.extra.is_empty() {
                    self.command_line.push_output(&format!(
                        "  清单没列的文件（不算失败）：{}",
                        report.extra.join("、")
                    ));
                }
            }
            Err(error) => {
                self.command_line
                    .push_error_coded(error.code(), &format!("{verb}: {error}。"));
            }
        }
    }

    /// 要校验的包目录 `name`：相对路径从第 `i` 个标签页的图纸所在目录算，图没保存过时从当前目录算。
    fn pidpublish_package_dir(&self, i: usize, name: &str) -> PathBuf {
        match crate::io::drawing_dir(&self.tabs[i].scene.document) {
            Some(drawing_dir) if Path::new(name).is_relative() => drawing_dir.join(name),
            _ => PathBuf::from(name),
        }
    }
}

/// 自动化操作（§5.3）：应答和失败的写法同别的控制面操作，不碰命令行。
impl OpenCADStudio {
    /// 控制面的 `pid_publish_check`：同 `PIDPUBLISH CHECK`，但前提逐条照实答，不在先不满足的那条
    /// 停下（[`Self::pidpublish_each`]），再答四项门禁、目标目录和这次会用的修订号；前提和门禁都过了
    /// `can_publish` 才为真。图没保存过时门禁、目标目录、修订号都是 `null`；`dir`、`rev` 写的目标
    /// 目录用不了、修订号不对、这一版已经有了是失败应答。要读的图缓存里没有时应答 `scanning`
    /// 并交出后台读的任务（[`Self::op_scan`]），调用方稍后再发。是查询：不改图，不写东西。
    pub(in crate::app) fn control_pid_publish_check(
        &mut self,
        req: &Value,
    ) -> (Value, Task<Message>) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return no_task(refuse("no_document", "Create or open a drawing first"));
        }
        let asked = match asked_of(req) {
            Ok(asked) => asked,
            Err(refusal) => return no_task(refusal),
        };
        let (preconditions, held) = self.pidpublish_each(i, &asked);
        let Some(path) = self.tabs[i].current_path.clone() else {
            return no_task(json!({
                "ok": true,
                "can_publish": false,
                "preconditions": preconditions,
                "gates": null,
                "target_dir": null,
                "next_revision": null,
            }));
        };
        let destination = match destination(&path, &asked) {
            Ok(destination) => destination,
            Err((code, why)) => return no_task(refuse(code, why)),
        };
        let rules = match self.pidcheck_load_rules(i) {
            Ok(rules) => rules,
            Err((code, why)) => return no_task(refuse(code, why)),
        };
        let document = &self.tabs[i].scene.document;
        let master = master_data::resolve(document, Some(&destination.project_dir));
        let (_, scanned) = match self.op_scan(i, rules.connect_tolerance, true) {
            Ok(scanned) => scanned,
            Err(answered) => return answered,
        };
        let gated = match publish_gates(&scanned, &destination.drawing_id, rules, &master) {
            Ok(gated) => gated,
            Err(why) => return no_task(refuse(INDEX_IO_ERROR, why)),
        };
        no_task(json!({
            "ok": true,
            "can_publish": held && gated.gates.passed(),
            "preconditions": preconditions,
            "gates": gated.gates,
            "target_dir": destination.dir.display().to_string(),
            "next_revision": destination.revision,
        }))
    }

    /// `pid_publish_check` 的七条前提（§3.4，离线规格 §5.7），各自照实答：存过、没有没保存的修改、
    /// 磁盘上的文件对得上、会话可写，和带同步记录的图队列空、没有不一致、连着并补齐了（没有同步
    /// 记录时这三条都为真）；再答它们放不放行发布——协同的照 [`publish_pkg::collab_preconditions`]，
    /// `allow_offline` 放过没连着。
    fn pidpublish_each(&self, i: usize, asked: &Asked) -> (Value, bool) {
        let tab = &self.tabs[i];
        let saved = tab.current_path.as_deref();
        let unchanged = saved.is_some_and(|path| self.pidpublish_file_unchanged(i, path));
        let writable = !self.readonly_refuses_command("PIDPUBLISH");
        let collab = CollabInput {
            allow_offline: asked.allow_offline,
            ..self.publish_collab_input(i)
        };
        let unrecorded = !collab.has_record;
        let verdict = publish_pkg::collab_preconditions(collab);
        let preconditions = json!({
            "saved": saved.is_some(),
            "clean": !tab.dirty,
            "file_unchanged": unchanged,
            "writable_session": writable,
            "collab_synced": unrecorded || collab.queued == 0,
            "collab_consistent": unrecorded || !collab.diverged,
            "collab_online": unrecorded || collab.caught_up,
        });
        let held = saved.is_some()
            && !tab.dirty
            && unchanged
            && writable
            && !matches!(verdict, CollabVerdict::Refused(_));
        (preconditions, held)
    }

    /// `pid_publish` 进写操作的流程以前先看（§5.3）：没有图、请求写错了、会话只读、前提没过、
    /// 取不到规则包（[`Self::pidpublish_ready`]），就在这里应答；要读的图缓存里没有时应答 `scanning`
    /// 并交出后台读的任务。能发布了是 `None`。
    pub(in crate::app) fn control_pid_publish_waits(
        &mut self,
        req: &Value,
    ) -> Option<(Value, Task<Message>)> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            let refusal = refuse("no_document", "Create or open a drawing first");
            return Some(no_task(refusal));
        }
        let tolerance = match self.pidpublish_ready(i, req) {
            Ok(ready) => ready.rules.connect_tolerance,
            Err(refusal) => return Some(no_task(refusal)),
        };
        self.op_scan(i, tolerance, true).err()
    }

    /// 控制面的 `pid_publish`，在写操作的流程里（§5.3）：同 `PIDPUBLISH`，`dir`、`rev`、
    /// `allow_offline` 是命令行上的目标目录、`REV`、`ALLOW_OFFLINE`，`plot` 交给出图。扫描前面
    /// [`Self::control_pid_publish_waits`] 等过了，这里不读盘。门禁没过是 `PUBLISH_BLOCKED`，带四项
    /// 门禁、重复位号的各处和检查报告；出包没成是它的错误码，`PUBLISH_FAILED` 带是哪一步（`step`）。
    /// 结果是包名、包的目录、修订号、发布前快照的序号、文件数、字节数和门禁。不改图。
    pub(in crate::app) fn control_pid_publish(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        let i = self.active_tab;
        let Ready {
            plot,
            target,
            rules,
        } = self.pidpublish_ready(i, req)?;
        let document = &self.tabs[i].scene.document;
        let master = master_data::resolve(document, Some(&target.project_dir));
        let (_, scanned) = self
            .op_scan(i, rules.connect_tolerance, false)
            .map_err(|(answer, _)| answer)?;
        let gated = publish_gates(&scanned, &target.drawing_id, rules, &master)
            .map_err(|why| refuse(INDEX_IO_ERROR, why))?;
        if !gated.gates.passed() {
            let why = format!("{}，没有发布", blocked_said(&gated.gates));
            let mut refusal = refuse(PUBLISH_BLOCKED, why);
            refusal["gates"] = json!(gated.gates);
            refusal["duplicates"] = json!(gated.duplicates);
            refusal["report"] = json!(gated.report);
            return Err(refusal);
        }
        let packing = Packing {
            i,
            scanned: &scanned,
            target: &target,
            master: &master,
            gated: &gated,
            now: unix_now(),
            plot: &plot,
        };
        let mut timings = Timings::default();
        let published = self.pidpublish_pack(&packing, &mut timings).map_err(
            |Unpublished { failed, kept }| {
                let mut refusal = refuse(failed.code, format!("{}；{kept}", failed.said()));
                if let Some(step) = failed.step {
                    refusal["step"] = json!(step);
                }
                refusal
            },
        )?;
        self.set_control_result(json!({
            "package_id": publish_pkg::package_name(&target.drawing_id, target.revision),
            "path": published.dir.display().to_string(),
            "revision": target.revision,
            "snapshot_seq": published.seq,
            "files": published.files,
            "bytes": published.bytes,
            "gates": gated.gates,
        }));
        Ok(Task::none())
    }

    /// `pid_publish` 发得了吗（§5.3）：请求写对了（[`asked_of`]、[`plot_of`]），会话可写，前提都过了
    /// （[`Self::pidpublish_preconditions`]），规则包取得到。只读会话拒发布：同命令闸门记下这次尝试，
    /// 应答带拒绝的级别。不写任何东西。
    fn pidpublish_ready(&mut self, i: usize, req: &Value) -> Result<Ready, Value> {
        let asked = asked_of(req)?;
        let plot = plot_of(req)?;
        if self.readonly_refuses_command("PIDPUBLISH") {
            let (code, level) = self.collab.refusal();
            self.collab.report_attempt("", "pid_publish");
            let mut refusal = refuse(&code, "协同：当前会话只读，拒绝发布");
            refusal["level"] = json!(level);
            return Err(refusal);
        }
        let target = self
            .pidpublish_preconditions(i, &asked)
            .map_err(|(code, why)| refuse(code, why))?;
        let rules = self
            .pidcheck_load_rules(i)
            .map_err(|(code, why)| refuse(code, why))?;
        Ok(Ready {
            plot,
            target,
            rules,
        })
    }

    /// 控制面的 `pid_publications`：同 `PIDPUBLISH LIST`，`dir` 是目标目录——图号、目标目录、这张图
    /// 发布到哪一版了（`state`，同命令行上说的）和各包，新的在前，各带包的目录（目标目录和包名
    /// 拼出）。是查询：不扫项目，不写东西。
    pub(in crate::app) fn control_pid_publications(&self, req: &Value) -> Value {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return refuse("no_document", "Create or open a drawing first");
        }
        let dir = match dir_of(req) {
            Ok(dir) => dir,
            Err(refusal) => return refusal,
        };
        let listed = match self.pidpublish_listed(i, dir.as_deref()) {
            Ok(listed) => listed,
            Err((code, why)) => return refuse(code, why),
        };
        let packages: Vec<Value> = listed
            .packages
            .iter()
            .map(|package| {
                let path = listed.target_dir.join(&package.package_id);
                json!({
                    "package_id": package.package_id,
                    "revision": package.revision,
                    "path": path.display().to_string(),
                    "published_at": package.published_at,
                    "published_by": package.published_by,
                    "current": package.current,
                })
            })
            .collect();
        json!({
            "ok": true,
            "drawing_id": listed.drawing_id,
            "target_dir": listed.target_dir.display().to_string(),
            "state": listed.state.to_string(),
            "packages": packages,
        })
    }

    /// 控制面的 `pid_publish_verify`：同 `PIDPUBLISH VERIFY`，`path` 是包目录（[`Self::pidpublish_package_dir`]）。
    /// 完好时应答是 §3.5 的报告带 `ok`；缺文件、对不上清单是它的错误码，也带报告；目录不在、
    /// 没有清单是 `PACKAGE_NOT_FOUND`。是查询：不写东西。
    pub(in crate::app) fn control_pid_publish_verify(&self, req: &Value) -> Value {
        let Some(name) = req["path"].as_str().filter(|name| !name.is_empty()) else {
            return refuse("invalid_request", "path 要写成不空的文本：包目录");
        };
        let dir = self.pidpublish_package_dir(self.active_tab, name);
        match publish_pkg::verify_package(&dir) {
            Ok(report) => with_report(json!({ "ok": true }), &report),
            Err(error) => {
                let refusal = refuse(error.code(), why_of(&error));
                match &error {
                    PublishError::Incomplete(report) | PublishError::Corrupt(report) => {
                        with_report(refusal, report)
                    }
                    _ => refusal,
                }
            }
        }
    }
}

/// 门禁这一段（§3.4、§4）：在扫好的项目 `scanned` 上现做一次检查——只报图号 `drawing_id` 那张图
/// 上的，跨图，触发时机「发布前」，用规则 `rules` 和主数据 `master`——数这张图的重复位号，再算
/// 四项门禁。这张图的摘要就是扫描时做的那一份。扫出来的摘要里没有这张图时说为什么。不碰命令行
/// 和程序状态。
pub(super) fn publish_gates(
    scanned: &Scanned,
    drawing_id: &str,
    rules: RuleSet,
    master: &Resolved,
) -> Result<Gated, String> {
    let current = scanned
        .summaries
        .iter()
        .find(|summary| summary.drawing_id == drawing_id)
        .ok_or_else(|| format!("扫出来的项目里没有图 {drawing_id}"))?;
    let duplicates = publish_pkg::duplicate_tags(current, &scanned.index, &scanned.summaries);
    let check = Check::before_publish(drawing_id.to_owned(), rules, master.clone());
    let (report, _) = piddrc::check_project(scanned, check);
    let gates = publish_pkg::gates(&report, duplicates.len(), master, scanned.index.complete);
    Ok(Gated {
        report,
        duplicates,
        gates,
    })
}

fn verb_of(only_check: bool) -> &'static str {
    if only_check {
        "PIDPUBLISH CHECK"
    } else {
        "PIDPUBLISH"
    }
}

/// 子命令词后面写的：一个目标目录、`REV <修订号>`、`ALLOW_OFFLINE`，先后不论。`REV`、
/// `ALLOW_OFFLINE` 不分大小写，加了引号就是目录名。写错时是错在哪里。
fn asked(text: &str) -> Result<Asked, String> {
    let mut asked = Asked::default();
    let mut words = words(text).ok_or("双引号没配对")?.into_iter();
    while let Some((word, quoted)) = words.next() {
        if !quoted && word.eq_ignore_ascii_case("REV") {
            let Some((revision, _)) = words.next() else {
                return Err("REV 后面要写修订号".to_owned());
            };
            if asked.revision.replace(revision.to_owned()).is_some() {
                return Err("REV 只能写一次".to_owned());
            }
        } else if !quoted && word.eq_ignore_ascii_case("ALLOW_OFFLINE") {
            if std::mem::replace(&mut asked.allow_offline, true) {
                return Err("ALLOW_OFFLINE 只能写一次".to_owned());
            }
        } else if word.is_empty() {
            return Err("目标目录不能是空的".to_owned());
        } else if asked.dir.replace(word.to_owned()).is_some() {
            return Err("目标目录只能写一个".to_owned());
        }
    }
    Ok(asked)
}

/// 按空白分开的词，各带它加没加双引号；加了双引号的一段算一个词，去掉引号。引号没配对是
/// `None`。
fn words(text: &str) -> Option<Vec<(&str, bool)>> {
    let mut found = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (word, quoted, after) = match rest.strip_prefix('"') {
            Some(inner) => {
                let end = inner.find('"')?;
                (&inner[..end], true, &inner[end + 1..])
            }
            None => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], false, &rest[end..])
            }
        };
        found.push((word, quoted));
        rest = after.trim_start();
    }
    Some(found)
}

fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}

/// 存在 `path` 的图在项目目录 `project_dir` 的图纸里的图号。按文件认：不在里面——项目只认这个
/// 目录里的 `.dwg`——是 `INDEX_NO_PROJECT_DIR`；列图出错是它的错误码。
fn listed_id(project_dir: &Path, path: &Path) -> Result<String, (&'static str, String)> {
    let drawings = project_index::list_drawings(project_dir)
        .map_err(|error| (error.code(), error.to_string()))?;
    let current = same_file(path);
    drawings
        .into_iter()
        .find(|drawing| same_file(&drawing.path) == current)
        .map(|drawing| drawing.drawing_id)
        .ok_or_else(|| {
            (
                INDEX_NO_PROJECT_DIR,
                "当前图不在项目的图纸里（项目只认目录里的 .dwg），发布要跨图检查".to_owned(),
            )
        })
}

/// 存在 `path` 的图的项目目录（图纸所在目录）、图号（[`listed_id`]）和目标目录：命令行上写的
/// `dir` 从项目目录算，不写是 `pid-publish`（§3.1）。
fn placed(
    path: &Path,
    dir: Option<&str>,
) -> Result<(PathBuf, String, PathBuf), (&'static str, String)> {
    let Some(project_dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) else {
        return Err((INDEX_NO_PROJECT_DIR, "当前图不属于哪个项目目录".to_owned()));
    };
    let drawing_id = listed_id(project_dir, path)?;
    let target = dir.map_or_else(
        || project_dir.join(DEFAULT_TARGET),
        |dir| project_dir.join(dir),
    );
    Ok((project_dir.to_path_buf(), drawing_id, target))
}

/// 要发到哪里（§3.1、§3.6）：存在 `path` 的图的项目目录和图号（[`placed`]），目标目录——要能用
/// （[`target_usable`]）——和这一版的修订号：写了的要对（[`revision_of`]），不写是下一版；这一版的
/// 名字被占了是 `PUBLISH_TARGET_EXISTS`。不写任何东西。
fn destination(path: &Path, asked: &Asked) -> Result<Destination, (&'static str, String)> {
    let (project_dir, drawing_id, dir) = placed(path, asked.dir.as_deref())?;
    target_usable(&dir).map_err(|why| (PUBLISH_TARGET_INVALID, why))?;
    let next = publish_pkg::next_revision(&dir, &drawing_id);
    let revision = match asked.revision.as_deref() {
        Some(text) => revision_of(text, next)?,
        None => next,
    };
    let package = dir.join(publish_pkg::package_name(&drawing_id, revision));
    if package.symlink_metadata().is_ok() {
        return Err((
            PUBLISH_TARGET_EXISTS,
            format!(
                "{} 已经有了，发布不覆盖已有的东西；换一个修订号或者目标目录",
                package.display()
            ),
        ));
    }
    Ok(Destination {
        project_dir,
        drawing_id,
        dir,
        revision,
    })
}

/// 请求里的目标目录 `dir`（§5.3）：不写是 `None`，写了要是不空的文本。
fn dir_of(req: &Value) -> Result<Option<String>, Value> {
    match &req["dir"] {
        Value::Null => Ok(None),
        Value::String(dir) if !dir.is_empty() => Ok(Some(dir.clone())),
        _ => Err(refuse("invalid_request", "dir 要写成不空的文本：目标目录")),
    }
}

/// 请求里写的（§5.3）：目标目录（[`dir_of`]），修订号 `rev`——整数原样交给前提查，不是整数也是
/// `PUBLISH_REVISION_INVALID`——和 `allow_offline`（布尔）。写错时是失败应答。
fn asked_of(req: &Value) -> Result<Asked, Value> {
    let dir = dir_of(req)?;
    let revision = match &req["rev"] {
        Value::Null => None,
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        other => {
            return Err(refuse(
                PUBLISH_REVISION_INVALID,
                format!("修订号 {other} 不是正整数"),
            ))
        }
    };
    let allow_offline = match &req["allow_offline"] {
        Value::Null => false,
        Value::Bool(allow) => *allow,
        _ => {
            return Err(refuse(
                "invalid_request",
                "allow_offline 要写成 true 或 false",
            ))
        }
    };
    Ok(Asked {
        dir,
        revision,
        allow_offline,
    })
}

/// `pid_publish` 的出图设置 `plot`（§5.3）：字段同 `plot` 操作，PDF 和 SVG 都用它，不写是默认的
/// （`null`）。文件名由包定，不能写 `path`；一次只出一个布局，`layout` 不能是 `all`。写错时是失败
/// 应答。
fn plot_of(req: &Value) -> Result<Value, Value> {
    let plot = &req["plot"];
    let Some(fields) = plot.as_object() else {
        return if plot.is_null() {
            Ok(Value::Null)
        } else {
            Err(refuse(
                "invalid_request",
                "plot 要写成对象，字段同 plot 操作",
            ))
        };
    };
    if fields.contains_key("path") {
        return Err(refuse(
            "invalid_request",
            "plot 里不写 path：PDF 和 SVG 的文件名由包定",
        ));
    }
    let every = fields
        .get("layout")
        .and_then(Value::as_str)
        .is_some_and(|layout| layout.eq_ignore_ascii_case("all"));
    if every {
        return Err(refuse(
            "invalid_request",
            "plot 的 layout 不能是 all：一次只出一个布局",
        ));
    }
    Ok(plot.clone())
}

/// 控制面的失败应答。
fn refuse(code: &str, error: impl Into<String>) -> Value {
    json!({ "ok": false, "status": "failed", "code": code, "error": error.into() })
}

/// 不带后台任务的应答。
fn no_task(answer: Value) -> (Value, Task<Message>) {
    (answer, Task::none())
}

/// 发布错误说的为什么，去掉打头的错误码。
fn why_of(error: &PublishError) -> String {
    let code = error.code();
    let said = error.to_string();
    said.strip_prefix(code)
        .and_then(|rest| rest.strip_prefix(": "))
        .unwrap_or(&said)
        .to_owned()
}

/// 校验报告（§3.5）的各字段，再加上 `answer` 的：应答外面的 `ok`，失败时的码和为什么。
fn with_report(answer: Value, report: &publish_pkg::VerifyReport) -> Value {
    let mut merged = json!(report);
    if let (Some(fields), Value::Object(outer)) = (merged.as_object_mut(), answer) {
        fields.extend(outer);
    }
    merged
}

/// 目标目录 `target_dir` 里图号 `drawing_id` 的包，新的在前，和发布到哪一版了（§4 末尾的表）：
/// 修订号最大的包，清单里源文件的校验和 `source.sha256` 与图纸文件 `source` 现在的相同、`dirty`
/// （有没保存的修改）为假时是已发布，否则是之后有改动。清单读不出来的包照列，算不一致。不写任何
/// 东西。
pub(super) fn publications(
    target_dir: &Path,
    drawing_id: &str,
    source: &Path,
    dirty: bool,
) -> Publications {
    let checksum = if dirty {
        None
    } else {
        fs::read(source)
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
    };
    let mut packages: Vec<Publication> = Vec::new();
    for package in publish_pkg::list_packages(target_dir, drawing_id)
        .into_iter()
        .rev()
    {
        let manifest = manifest_of(&package.path);
        let same = checksum.as_deref().is_some_and(|checksum| {
            manifest
                .pointer("/source/sha256")
                .and_then(Value::as_str)
                .is_some_and(|listed| listed.eq_ignore_ascii_case(checksum))
        });
        let current = same && !packages.iter().any(|newer| newer.current);
        packages.push(Publication {
            package_id: package.package_id,
            revision: package.revision,
            published_at: manifest.get("published_at").and_then(Value::as_i64),
            published_by: manifest
                .get("published_by")
                .and_then(Value::as_str)
                .map(str::to_owned),
            current,
        });
    }
    let state = match packages.first() {
        None => PublishState::Unpublished,
        Some(latest) if latest.current => PublishState::Published(latest.revision),
        Some(latest) => PublishState::ChangedSince(latest.revision),
    };
    Publications {
        drawing_id: drawing_id.to_owned(),
        target_dir: target_dir.to_path_buf(),
        state,
        packages,
    }
}

/// 包目录 `dir` 里的发布清单；读不出来、不是 JSON 时是 `null`。
fn manifest_of(dir: &Path) -> Value {
    let Ok(bytes) = fs::read(dir.join(publish_pkg::MANIFEST_FILE)) else {
        return Value::Null;
    };
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(text).unwrap_or(Value::Null)
}

/// 发布记录的一行：包名、出包的时间（UTC）、发布人，与图纸文件一致的标「当前」。
fn publication_said(package: &Publication) -> String {
    let Some(at) = package.published_at else {
        return format!("  {}  （发布清单读不出来）", package.package_id);
    };
    let at = utc_said(u64::try_from(at).unwrap_or_default());
    let by = package.published_by.as_deref().unwrap_or_default();
    let current = if package.current { "  当前" } else { "" };
    format!("  {}  {at}  {by}{current}", package.package_id)
}

/// `LIST`、`VERIFY` 后面写的：至多一个目录，有空格的加双引号。写错时说错在哪里。
fn only_folder(text: &str) -> Result<Option<&str>, &'static str> {
    match words(text).ok_or("双引号没配对")?.as_slice() {
        [] => Ok(None),
        [("", _)] => Err("目录不能是空的"),
        [(word, _)] => Ok(Some(*word)),
        _ => Err("只能写一个目录"),
    }
}

/// 目标目录 `dir` 能不能用（§3.6 `PUBLISH_TARGET_INVALID`），只看路径，不建也不试写：已经有的
/// 要是目录；还没有的，往上找到的第一个已有的要是目录，发布时从那里建下来。不能用时说为什么。
fn target_usable(dir: &Path) -> Result<(), String> {
    let mut at = dir;
    loop {
        match fs::metadata(at) {
            Ok(meta) if meta.is_dir() => return Ok(()),
            Ok(_) if at == dir => return Err(format!("{} 不是目录", dir.display())),
            Ok(_) => {
                return Err(format!(
                    "{} 不是目录，{} 建不了",
                    at.display(),
                    dir.display()
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match at.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                    Some(parent) => at = parent,
                    None => return Err(format!("{} 建不了：往上没有已有的目录", dir.display())),
                }
            }
            Err(error) => return Err(format!("{} 看不了：{error}", at.display())),
        }
    }
}

/// `REV` 后面写的修订号（§3.1）：正整数，并且不小于下一版 `next`，也就是大于已有的最大修订号。
fn revision_of(text: &str, next: u32) -> Result<u32, (&'static str, String)> {
    let Some(revision) = text.parse::<u32>().ok().filter(|revision| *revision > 0) else {
        return Err((
            PUBLISH_REVISION_INVALID,
            format!("修订号 {text} 不是正整数"),
        ));
    };
    if revision < next {
        return Err((
            PUBLISH_REVISION_INVALID,
            format!(
                "修订号 {revision} 不大于已有的最大修订号 {}；不写 REV 就是第 {next} 版",
                next - 1
            ),
        ));
    }
    Ok(revision)
}

/// 协同前提没过时说的为什么（离线规格 §5.7），`queued` 是队列里的条数。
fn collab_said(code: &str, queued: usize) -> String {
    match code {
        publish_pkg::COLLAB_UNSYNCED => format!(
            "图里有同步记录，队列里还有 {queued} 条修改没交给服务器；连上服务器交完再发布"
        ),
        publish_pkg::COLLAB_DIVERGED => {
            "图里有同步记录，它已知与服务器上的不一致，或者这次连接对不上；PIDCOLLAB UNSYNCED 看是哪些"
                .to_owned()
        }
        _ => "图里有同步记录，现在没连着服务器或者还没补齐；连上补齐后再发布，\
              或者加 ALLOW_OFFLINE 照现在的样子发布"
            .to_owned(),
    }
}

/// 门禁没过的是哪几项，说给人听：「2 条错误、1 个重复位号」。
fn blocked_said(gates: &Gates) -> String {
    let mut parts = Vec::new();
    if gates.drc_errors > 0 {
        parts.push(format!("{} 条错误", gates.drc_errors));
    }
    if gates.duplicate_tags > 0 {
        parts.push(format!("{} 个重复位号", gates.duplicate_tags));
    }
    if gates.master_data == MasterDataState::Unreadable {
        parts.push("主数据读不出来".to_owned());
    }
    if !gates.index_complete {
        parts.push("项目里有读不出来的图".to_owned());
    }
    parts.join("、")
}

fn master_said(state: MasterDataState) -> &'static str {
    match state {
        MasterDataState::Configured => "已配置",
        MasterDataState::NotConfigured => "没配置（不拦）",
        MasterDataState::Unreadable => "读不出来",
    }
}

/// 清掉目标目录 `dir` 里以前崩溃留下的临时目录（§4）：名字以 [`STAGING_PREFIX`] 开头的目录，符号
/// 链接不跟。删不掉的留着，下次再清。正在往这里发布的另一个程序的临时目录也认不出来，会一并删掉。
fn clear_leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let leftover = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        if leftover && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// 清单的 `project`：项目目录 `project_dir` 里项目文件的 `project.id`、`project.name`（主数据规格
/// §3.3），缺的一项是空的；没有项目文件、文件里没有 `project`，或者读不出来时没有。
fn project_ref(project_dir: &Path) -> Option<ProjectRef> {
    let path = project_dir.join(master_data::PROJECT_FILE);
    let file = master_data::project_file_on_disk(&path).ok()??;
    let project = file.get("project")?.as_object()?;
    let text = |key: &str| {
        project
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(ProjectRef {
        id: text("id"),
        name: text("name"),
    })
}

/// 清单的 `collab`（离线规格 §5.7）：协同前提放行了的，是图 `document` 里的同步记录，带放行时
/// 是不是对过账；图里没有同步记录时没有。
fn collab_record(document: &document::Document, verdict: CollabVerdict) -> Option<CollabRecord> {
    let CollabVerdict::Pass { reconciled } = verdict else {
        return None;
    };
    let state = collab_state::load_state(document)?;
    Some(CollabRecord {
        drawing_id: state.drawing_id,
        repo_id: state.repo_id,
        revision: state.revision,
        reconciled,
    })
}

/// 快照和清单里记的出处：程序名和版本。
fn generator() -> String {
    format!("OpenCADStudio {}", env!("CARGO_PKG_VERSION"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 包目录 `dir` 里的文件数和总字节数。
fn package_size(dir: &Path) -> (usize, u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| fs::metadata(entry.path()).ok())
        .filter(fs::Metadata::is_file)
        .fold((0, 0), |(files, bytes), meta| {
            (files + 1, bytes + meta.len())
        })
}

/// 出包的一步（§3.6 的 `step`）说给人听的名字。
fn step_said(step: &str) -> &'static str {
    match step {
        "snapshot" => "存快照",
        "plot" => "出图",
        "bom" => "算材料清单",
        "write" => "写包",
        "verify" => "自检",
        "commit" => "落位",
        _ => "出包",
    }
}

/// 测试钩子要出包的 `step` 这一步失败时就当它失败了，否则做 `work`。
fn unless_broken<T>(
    step: &'static str,
    work: impl FnOnce() -> Result<T, Failed>,
) -> Result<T, Failed> {
    if broken(step) {
        return Err(Failed::at(step, "测试让这一步失败"));
    }
    work()
}

#[cfg(test)]
thread_local! {
    /// 测试钩子（验收第 3 条）：设成出包的一步（`snapshot`、`plot`、`bom`、`write`、`verify`、
    /// `commit`），走到那一步就失败；`verify` 是把临时目录里的材料清单改坏，让落位前的自检真的不过。
    static BREAK_AT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn broken(step: &str) -> bool {
    BREAK_AT.with(|at| at.get() == Some(step))
}

/// 不是测试时没有钩子，哪一步都不故意失败。
#[cfg(not(test))]
fn broken(_step: &str) -> bool {
    false
}

/// 测试钩子要自检失败时，给临时目录 `staging` 里的材料清单末尾多写一个换行，字节数和校验和就
/// 对不上清单了。
#[cfg(test)]
fn spoil_if_broken(staging: &Path) {
    if broken("verify") {
        let path = staging.join("bom.csv");
        let mut bytes = fs::read(&path).expect("临时目录里有材料清单");
        bytes.push(b'\n');
        fs::write(&path, bytes).expect("改得了材料清单");
    }
}

#[cfg(not(test))]
fn spoil_if_broken(_staging: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::collab::FileClaim;
    use crate::pid::collab_state::Diverged;
    use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
    use crate::pid::project_index::{
        build_index, summarize, synthetic_drawing, SummaryMeta, SummarySource,
    };
    use crate::pid::publish_pkg::{
        COLLAB_DIVERGED, COLLAB_OFFLINE, COLLAB_UNSYNCED, PACKAGE_CORRUPT, PACKAGE_NOT_FOUND,
    };
    use crate::pid::symbols::SymbolLibrary;
    use crate::pid::{FileCollabState, QueuedOp};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 测试用的临时目录，就是项目目录；用完删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：同一纳秒里建的两个靠计数分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "ocs-pidpublish-{}-{}-{nanos}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("临时目录");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// 目录里现有的东西，连同子目录里的，各是相对路径和大小，排好序。
        fn listing(&self) -> Vec<(String, u64)> {
            fn walk(dir: &Path, root: &Path, found: &mut Vec<(String, u64)>) {
                for entry in fs::read_dir(dir).expect("读目录").flatten() {
                    let path = entry.path();
                    let name = path.strip_prefix(root).expect("在目录里");
                    // Windows 上 `DirEntry::metadata` 取的是目录项里记的大小，开着的文件（编辑租约的
                    // 锁文件）要过一阵才更新；按路径取才是现在的大小。
                    let len = fs::metadata(&path).map_or(0, |meta| meta.len());
                    found.push((name.to_string_lossy().into_owned(), len));
                    if path.is_dir() {
                        walk(&path, root, found);
                    }
                }
            }
            let mut found = Vec::new();
            walk(&self.0, &self.0, &mut found);
            found.sort();
            found
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

    fn forget_said(app: &mut OpenCADStudio) {
        app.command_line.history.clear();
        app.command_line.last_error = None;
        app.command_line.last_error_code = None;
    }

    fn ran(app: &OpenCADStudio) -> Ran {
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

    /// 像在命令行上敲进去那样跑一行，它在后台起的任务也一直跑到完。
    fn run(app: &mut OpenCADStudio, line: &str) -> Ran {
        forget_said(app);
        let task = app.run_command_line(line);
        app.drive_headless_task(task).expect("命令的任务");
        ran(app)
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
            "pidpublish-{}",
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
        app.finish_all_pending_history();
    }

    /// 用存盘操作把当前图存成 `path`：标签页记下路径和指纹，不再是「已修改」。
    fn save(app: &mut OpenCADStudio, path: &Path) {
        let saved = request(app, json!({"op": "save", "path": path, "overwrite": true}));
        assert_eq!(saved["ok"], true, "{saved}");
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.current_path.as_deref(), Some(path));
        assert!(!tab.dirty, "存完还是已修改");
        assert!(tab.disk_fingerprint.is_some(), "存完没记下指纹");
    }

    /// 一台泵的图，存盘前先在图上做 `before_save`（写同步记录之类），再用存盘操作存成项目目录里的
    /// PID-1101.dwg。
    fn sheet_with(
        scratch: &Scratch,
        before_save: impl FnOnce(&mut document::Document),
    ) -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        let i = app.active_tab;
        before_save(&mut app.tabs[i].scene.document);
        save(&mut app, &scratch.path("PID-1101.dwg"));
        app
    }

    fn sheet(scratch: &Scratch) -> OpenCADStudio {
        sheet_with(scratch, |_| {})
    }

    /// 项目里另一张没打开的图 PID-1102：一台泵，位号与 [`sheet`] 的那台相同，直接写成文件。
    fn other_sheet(scratch: &Scratch) {
        let mut other = OpenCADStudio::new_for_test();
        let _ = other.automation_op(r#"{"op":"new"}"#);
        draw(&mut other, "PIDINSERT pump", &[[40.0, 60.0]]);
        let bytes = crate::io::save_to_bytes(
            other.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("存 DWG");
        fs::write(scratch.path("PID-1102.dwg"), bytes).expect("写文件");
    }

    /// 项目规则包：错误级的内置规则只留 `keep` 一条，别的都停用；有错也存得了盘（`warn`）。
    fn only_rule(scratch: &Scratch, keep: &str) {
        let off: Vec<Value> = builtin_rules::builtin()
            .rules
            .iter()
            .filter(|rule| rule.severity == Severity::Error && rule.id != keep)
            .map(|rule| json!({ "id": rule.id, "enabled": false }))
            .collect();
        let pack = json!({
            "version": 1,
            "pack": format!("只留 {keep}"),
            "rule_set_version": "P1",
            "save_gate": "warn",
            "rules": off,
        });
        fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("写项目包");
    }

    /// 说的话里以 `start` 开头的那一行。
    fn line_of<'a>(said: &'a str, start: &str) -> &'a str {
        said.lines()
            .find(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("没有以 {start} 开头的一行：\n{said}"))
    }

    /// `text` 里 `key` 后面紧跟的数。
    fn number_after(text: &str, key: &str) -> usize {
        let start = text
            .find(key)
            .unwrap_or_else(|| panic!("{key} 不在 {text} 里"))
            + key.len();
        let digits: String = text[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits
            .parse()
            .unwrap_or_else(|_| panic!("{key} 后面不是数：{text}"))
    }

    /// 目录里的名字，排好序；目录不在时是空的。
    fn names(dir: &Path) -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    }

    /// 存下的 PID-1101 的快照文件，不算目录清单。
    fn snapshots(scratch: &Scratch) -> Vec<String> {
        names(&store::dir_for(&scratch.0, "PID-1101"))
            .into_iter()
            .filter(|name| name != store::INDEX_FILE)
            .collect()
    }

    fn json_at(path: &Path) -> Value {
        let bytes = fs::read(path).unwrap_or_else(|error| panic!("读 {}：{error}", path.display()));
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{} 不是 JSON：{error}", path.display()))
    }

    /// 图现在的样子：「已修改」、撤销栈有几步、磁盘上的文件。
    fn drawing_state(app: &OpenCADStudio, path: &Path) -> (bool, usize, Vec<u8>) {
        let tab = &app.tabs[app.active_tab];
        let bytes = fs::read(path).expect("读图");
        (tab.dirty, tab.history.undo_stack.len(), bytes)
    }

    /// 先查一遍（`PIDCHECK`），把规则 `rule` 报的错误都豁免掉，再存成 `path`。
    fn waive_all(app: &mut OpenCADStudio, rule: &str, path: &Path) {
        run(app, "PIDCHECK");
        let i = app.active_tab;
        let kept = app.tabs[i].pid_check.as_ref().expect("PIDCHECK 记下了结果");
        let mut keys: Vec<String> = Vec::new();
        for result in &kept.report.results {
            let error = result.rule_id == rule && result.severity == Severity::Error;
            if error && !keys.contains(&result.object_key) {
                keys.push(result.object_key.clone());
            }
        }
        assert!(!keys.is_empty(), "没有 {rule} 的错误");
        for key in keys {
            let line = format!("PIDCHECK WAIVE {rule} \"{key}\" 名称发布后补");
            let ran = run(app, &line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        save(app, path);
    }

    /// 让出包的 `step` 这一步失败着跑一行。
    fn run_broken(app: &mut OpenCADStudio, step: &'static str, line: &str) -> Ran {
        BREAK_AT.with(|at| at.set(Some(step)));
        let ran = run(app, line);
        BREAK_AT.with(|at| at.set(None));
        ran
    }

    #[test]
    fn pidpublish_is_registered_and_says_how_it_is_written() {
        assert!(crate::command::all_registered_command_names().contains(&"PIDPUBLISH"));
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for line in [
            "PIDPUBLISH REV",
            "PIDPUBLISH CHECK REV",
            "PIDPUBLISH out more",
            "pidpublish check REV 2 REV 3",
            "PIDPUBLISH ALLOW_OFFLINE allow_offline",
            "PIDPUBLISH \"out",
            "PIDPUBLISH CHECK \"\"",
            "PIDPUBLISH LIST a b",
            "pidpublish list \"a",
            "PIDPUBLISH VERIFY",
            "pidpublish verify a b",
            "PIDPUBLISH VERIFY \"\"",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code, None, "{line}: {}", ran.said);
            assert!(
                ran.error
                    .is_some_and(|error| error.starts_with("PIDPUBLISH")),
                "{line}: {}",
                ran.said
            );
        }
        let ran = run(&mut app, "PIDPUBLISH LIST");
        assert_eq!(ran.code.as_deref(), Some(PUBLISH_UNSAVED), "{}", ran.said);
    }

    /// 目标目录、`REV`、`ALLOW_OFFLINE` 先后不论；加了引号的是目录名，可以有空格。
    #[test]
    fn pidpublish_reads_the_folder_the_revision_and_allow_offline() {
        assert_eq!(asked(""), Ok(Asked::default()));
        assert_eq!(
            asked(r#"rev 3 "D:/发布 区" allow_offline"#),
            Ok(Asked {
                dir: Some("D:/发布 区".to_owned()),
                revision: Some("3".to_owned()),
                allow_offline: true,
            })
        );
        assert_eq!(
            asked(r#"REV x "REV""#),
            Ok(Asked {
                dir: Some("REV".to_owned()),
                revision: Some("x".to_owned()),
                allow_offline: false,
            })
        );
        for wrong in [
            "REV",
            "a b",
            "REV 1 REV 2",
            "ALLOW_OFFLINE ALLOW_OFFLINE",
            "\"a",
            "\"\"",
        ] {
            assert!(asked(wrong).is_err(), "{wrong}");
        }
    }

    /// 验收第 1 条前两种：没保存过、有没保存的修改是 `PUBLISH_UNSAVED`；标签页没记到指纹、文件被别的
    /// 程序改过是 `PUBLISH_FILE_CHANGED`。检查和发布都一样，都不写任何东西。
    #[test]
    fn pidpublish_refuses_an_unsaved_drawing_and_a_file_changed_on_disk() {
        let scratch = Scratch::new();
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        let refused = |app: &mut OpenCADStudio, code: &str| {
            let before = scratch.listing();
            for line in ["PIDPUBLISH CHECK", "PIDPUBLISH"] {
                let ran = run(app, line);
                assert_eq!(ran.code.as_deref(), Some(code), "{line}: {}", ran.said);
                assert!(!ran.said.contains("检查（发布前）"), "{line}: {}", ran.said);
                assert_eq!(scratch.listing(), before, "{line} 写了东西");
            }
        };
        refused(&mut app, PUBLISH_UNSAVED);

        let current = scratch.path("PID-1101.dwg");
        save(&mut app, &current);
        draw(&mut app, "PIDINSERT pump", &[[100.0, 0.0]]);
        assert!(app.tabs[app.active_tab].dirty);
        refused(&mut app, PUBLISH_UNSAVED);

        save(&mut app, &current);
        let i = app.active_tab;
        let recorded = app.tabs[i].disk_fingerprint.take();
        refused(&mut app, PUBLISH_FILE_CHANGED);
        app.tabs[i].disk_fingerprint = recorded;

        // Windows 不让别人写租着的图：先放掉租约，再像别的程序那样改文件。
        let _ = app.tabs[i].edit_lease.take();
        let mut bytes = fs::read(&current).expect("读文件");
        bytes.extend_from_slice(b"changed elsewhere");
        fs::write(&current, bytes).expect("别的程序改了文件");
        refused(&mut app, PUBLISH_FILE_CHANGED);
    }

    /// 验收第 1 条第三种：只读会话里发布被拒（`COLLAB_READONLY`），什么也不写；`PIDPUBLISH CHECK`
    /// 照常查到门禁，门禁都过了也说这个会话发布不了。
    #[test]
    fn pidpublish_is_refused_in_a_readonly_session_and_its_check_is_not() {
        let scratch = Scratch::new();
        // DR-001 只管管线，这张图上没有，所以没有错误。
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        app.collab.readonly = true;
        let before = scratch.listing();
        for line in [
            "PIDPUBLISH",
            "pidpublish REV 2",
            "PIDPUBLISH \"out dir\" ALLOW_OFFLINE",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some("COLLAB_READONLY"),
                "{line}: {}",
                ran.said
            );
            assert!(!ran.said.contains("检查（发布前）"), "{line}: {}", ran.said);
        }
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        let counts = line_of(&ran.said, "PIDPUBLISH CHECK: 检查（发布前）错误 ");
        assert_eq!(number_after(counts, "错误 "), 0, "{}", ran.said);
        assert_eq!(ran.code.as_deref(), Some("COLLAB_READONLY"), "{}", ran.said);
        assert!(ran.said.contains("只读"), "{}", ran.said);
        assert_eq!(scratch.listing(), before, "只读会话里写了东西");
    }

    /// 验收第 2 条：一张有错误的图（泵没填名称，位号另一张图上也有），检查和发布都报出四项门禁的
    /// 计数和前几条错误、重复位号的各处，`PUBLISH_BLOCKED`；项目目录里没有多出任何文件或目录——
    /// 目标目录、快照都没有。
    #[test]
    fn pidpublish_blocks_a_drawing_with_errors_and_writes_nothing() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        other_sheet(&scratch);
        let before = scratch.listing();
        for line in ["PIDPUBLISH CHECK", "PIDPUBLISH"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(PUBLISH_BLOCKED),
                "{line}: {}",
                ran.said
            );
            let target = line_of(&ran.said, &format!("{line}: 图 PID-1101 发布为第 1 版"));
            assert!(target.contains("PID-1101-r001"), "{target}");
            let counts = line_of(&ran.said, &format!("{line}: 检查（发布前）错误 "));
            let errors = number_after(counts, "错误 ");
            assert!(errors > 0, "{counts}");
            assert_eq!(number_after(counts, "重复位号 "), 1, "{counts}");
            assert!(counts.contains("项目里的图都读出来了"), "{counts}");
            let listed = ran
                .said
                .lines()
                .filter(|said| said.starts_with("  错误 DR-"));
            assert_eq!(listed.count(), errors.min(LISTED), "{}", ran.said);
            assert!(ran.said.contains("  错误 DR-010 "), "{}", ran.said);
            assert!(ran.said.contains("  错误 DR-102 "), "{}", ran.said);
            let duplicate = line_of(&ran.said, "  重复位号 ");
            assert!(
                duplicate.contains("PID-1101") && duplicate.contains("PID-1102"),
                "{duplicate}"
            );
            let verdict = ran.error.expect("拦下了");
            assert!(
                verdict.contains(&format!("{errors} 条错误、1 个重复位号")),
                "{verdict}"
            );
            assert_eq!(scratch.listing(), before, "{line} 写了东西");
        }
    }

    /// 验收第 2 条后半：检查是现做的。空图存盘后 `PIDCHECK` 留下一份「没有错误」的结果，这时发布
    /// 检查放行；再放一台没填名称的泵（`DR-010`）、存盘，标签页上还是那份旧结果，发布检查仍然拦下，
    /// 旧结果也不动。
    #[test]
    fn pidpublish_check_does_the_check_afresh() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-010");
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let current = scratch.path("PID-1101.dwg");
        save(&mut app, &current);
        let ran = run(&mut app, "PIDCHECK");
        assert!(ran.said.contains("错误 0 条"), "{}", ran.said);
        let i = app.active_tab;
        let kept = app.tabs[i].pid_check.take().expect("PIDCHECK 记下了结果");
        assert_eq!(kept.report.summary.errors, 0);
        let serial = kept.serial;

        let ran = run(&mut app, "PIDPUBLISH CHECK");
        assert_eq!(ran.code, None, "{}", ran.said);
        assert!(ran.said.contains("可以发布"), "{}", ran.said);

        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        save(&mut app, &current);
        app.tabs[i].pid_check = Some(kept);
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        assert_eq!(ran.code.as_deref(), Some(PUBLISH_BLOCKED), "{}", ran.said);
        assert!(ran.said.contains("  错误 DR-010 "), "{}", ran.said);
        let kept = app.tabs[i].pid_check.as_ref().expect("旧结果还在");
        assert_eq!((kept.serial, kept.report.summary.errors), (serial, 0));
    }

    /// 扫完再过一遍前提：后台读盘的时候图改了，就照前提说（`PUBLISH_UNSAVED`），不算门禁。
    #[test]
    fn pidpublish_looks_at_the_preconditions_again_after_the_scan() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        other_sheet(&scratch);
        forget_said(&mut app);
        let task = app.run_command_line("PIDPUBLISH CHECK");
        let i = app.active_tab;
        app.tabs[i].dirty = true;
        app.drive_headless_task(task).expect("命令的任务");
        let ran = ran(&app);
        assert!(
            ran.said.contains("PIDPUBLISH CHECK: 要从磁盘读 1 个文件"),
            "{}",
            ran.said
        );
        assert_eq!(ran.code.as_deref(), Some(PUBLISH_UNSAVED), "{}", ran.said);
        assert!(!ran.said.contains("检查（发布前）"), "{}", ran.said);
    }

    /// 目标目录和修订号（§3.1、§3.6）：修订号不是正整数、不大于已有的最大修订号是
    /// `PUBLISH_REVISION_INVALID`；目标是文件、在文件下面是 `PUBLISH_TARGET_INVALID`；这一版的名字已经
    /// 有人占了是 `PUBLISH_TARGET_EXISTS`。不写修订号就是下一版，还没有的目标目录不建。都不写任何东西。
    #[test]
    fn pidpublish_check_refuses_a_bad_revision_or_target() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let published = scratch.path(DEFAULT_TARGET);
        fs::create_dir_all(published.join("PID-1101-r002")).expect("建旧包");
        fs::write(scratch.path("a file.txt"), b"not a folder").expect("写文件");
        let before = scratch.listing();
        for (line, code) in [
            ("PIDPUBLISH CHECK REV 0", PUBLISH_REVISION_INVALID),
            ("PIDPUBLISH CHECK REV -1", PUBLISH_REVISION_INVALID),
            ("PIDPUBLISH CHECK REV 二", PUBLISH_REVISION_INVALID),
            ("PIDPUBLISH CHECK REV 2", PUBLISH_REVISION_INVALID),
            ("PIDPUBLISH CHECK \"a file.txt\"", PUBLISH_TARGET_INVALID),
            (
                "PIDPUBLISH CHECK \"a file.txt/sub\" REV 1",
                PUBLISH_TARGET_INVALID,
            ),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code.as_deref(), Some(code), "{line}: {}", ran.said);
        }
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        line_of(
            &ran.said,
            "PIDPUBLISH CHECK: 图 PID-1101 发布为第 3 版（PID-1101-r003）",
        );
        let ran = run(&mut app, "PIDPUBLISH CHECK new/sub REV 1");
        let target = line_of(&ran.said, "PIDPUBLISH CHECK: 图 PID-1101 发布为第 1 版");
        assert!(target.contains("new"), "{target}");
        assert_eq!(scratch.listing(), before, "检查写了东西");

        fs::write(published.join("PID-1101-r003"), b"in the way").expect("占位");
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        assert_eq!(
            ran.code.as_deref(),
            Some(PUBLISH_TARGET_EXISTS),
            "{}",
            ran.said
        );
        let ran = run(&mut app, "PIDPUBLISH CHECK REV 7");
        line_of(
            &ran.said,
            "PIDPUBLISH CHECK: 图 PID-1101 发布为第 7 版（PID-1101-r007）",
        );
    }

    /// 验收第 4 条：带同步记录的图，队列不空、标了不一致、没连着各报对应的码，`ALLOW_OFFLINE` 只放过
    /// 没连着；没有同步记录的图不受影响。
    #[test]
    fn pidpublish_check_answers_the_collab_preconditions_of_a_drawing_with_a_sync_record() {
        let record = FileCollabState {
            drawing_id: "PID-1101".to_owned(),
            repo_id: "repo-1".to_owned(),
            revision: 3,
        };
        let queued = QueuedOp {
            op_id: "op-1".to_owned(),
            payload: json!({ "changes": [] }),
            base_revision: 3,
            on: true,
            label: "改位号".to_owned(),
            user_id: Some("u-1".to_owned()),
            queued_at: Some(1_795_000_000),
            inverse: None,
        };
        let apart = Diverged {
            code: collab_state::REVISION_GAP.to_owned(),
            message: "缺了修订 4".to_owned(),
            at: 1_795_000_000,
        };
        let passed_on = |ran: &Ran| {
            let code = ran.code.as_deref().unwrap_or_default();
            assert!(!code.starts_with("COLLAB_"), "{}", ran.said);
            line_of(&ran.said, "PIDPUBLISH CHECK: 检查（发布前）错误 ");
        };

        let scratch = Scratch::new();
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
            collab_state::store_queue(document, std::slice::from_ref(&queued));
        });
        for line in ["PIDPUBLISH CHECK", "PIDPUBLISH CHECK ALLOW_OFFLINE"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(COLLAB_UNSYNCED),
                "{line}: {}",
                ran.said
            );
            assert!(ran.said.contains("队列里还有 1 条"), "{line}: {}", ran.said);
        }

        let scratch = Scratch::new();
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state_diverged(document, Some(&record), Some(&apart));
        });
        for line in ["PIDPUBLISH CHECK", "PIDPUBLISH CHECK ALLOW_OFFLINE"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(COLLAB_DIVERGED),
                "{line}: {}",
                ran.said
            );
        }

        let scratch = Scratch::new();
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
        });
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        assert_eq!(ran.code.as_deref(), Some(COLLAB_OFFLINE), "{}", ran.said);
        assert!(!ran.said.contains("检查（发布前）"), "{}", ran.said);
        let ran = run(&mut app, "PIDPUBLISH CHECK ALLOW_OFFLINE");
        passed_on(&ran);
        assert!(ran.said.contains("按 ALLOW_OFFLINE"), "{}", ran.said);

        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let ran = run(&mut app, "PIDPUBLISH CHECK");
        passed_on(&ran);
        assert!(!ran.said.contains("ALLOW_OFFLINE"), "{}", ran.said);
    }

    /// PUB-4 验收第 1 条：一张干净的图（没有错误，变送器没分配回路是警告）发布成十一个文件的包：自检过得了，
    /// 图纸文件与源文件逐字节相同，`bom.json` 是从清单记的那份快照算的，包里的快照就是快照目录里存下的那份，
    /// 警告和提示都在 `drc-report.json` 里；清单的 `project` 取自项目文件，没有同步记录时 `collab` 是 `null`；
    /// 包里的文件都是只读的。说出包在哪里、几个文件、各步用时；图的「已修改」、撤销栈、磁盘上的文件都不变。
    #[test]
    fn pidpublish_writes_a_package_of_eleven_files_that_verifies() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let project = json!({
            "version": 1,
            "project": { "id": "CDU-2026", "name": "常压蒸馏改造" },
        });
        fs::write(scratch.path(master_data::PROJECT_FILE), project.to_string())
            .expect("写项目文件");
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDINSERT transmitter", &[[60.0, 80.0]]);
        let current = scratch.path("PID-1101.dwg");
        save(&mut app, &current);
        let before = drawing_state(&app, &current);

        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.error, None, "{}", ran.said);
        let package = scratch.path(DEFAULT_TARGET).join("PID-1101-r001");
        let done = line_of(&ran.said, "PIDPUBLISH: 已发布第 1 版：");
        assert!(done.contains(&package.display().to_string()), "{done}");
        assert!(done.contains("11 个文件"), "{done}");
        let took = line_of(&ran.said, "PIDPUBLISH: 用时：");
        for step in ["快照", "出图", "材料清单", "写包", "自检落位"] {
            assert!(took.contains(step), "{took}");
        }
        assert_eq!(drawing_state(&app, &current), before, "发布动了图");

        let files = names(&package);
        assert_eq!(files.len(), 11, "{files:?}");
        let report = publish_pkg::verify_package(&package).expect("自检过得了");
        let problems = (
            report.missing.len(),
            report.mismatched.len(),
            report.extra.len(),
        );
        assert_eq!((report.files, problems), (11, (0, 0, 0)), "{report:?}");
        for name in &files {
            let meta = fs::metadata(package.join(name)).expect("包里的文件");
            assert!(meta.permissions().readonly(), "{name} 不是只读的");
        }
        assert_eq!(
            fs::read(package.join("PID-1101.dwg")).expect("包里的图"),
            before.2
        );
        let pdf = fs::read(package.join("PID-1101.pdf")).expect("PDF");
        assert!(pdf.starts_with(b"%PDF"), "不是 PDF");
        let svg = fs::read_to_string(package.join("PID-1101.svg")).expect("SVG");
        assert!(svg.contains("<svg"), "不是 SVG");

        let manifest = json_at(&package.join("manifest.json"));
        let bill = json_at(&package.join("bom.json"));
        assert_eq!(manifest["snapshot_seq"], 1, "{manifest}");
        assert_eq!(bill["snapshot_seq"], manifest["snapshot_seq"]);
        assert_eq!(bill["source"], "snapshot");
        assert_eq!(
            manifest["project"],
            json!({ "id": "CDU-2026", "name": "常压蒸馏改造" })
        );
        assert_eq!(manifest["collab"], Value::Null);
        assert_eq!(snapshots(&scratch), ["0001.json"]);
        let stored = store::dir_for(&scratch.0, "PID-1101").join("0001.json");
        let packed = fs::read(package.join("snapshot.json")).expect("包里的快照");
        assert_eq!(packed, fs::read(&stored).expect("存下的快照"));
        assert_eq!(json_at(&stored)["reason"], "发布前自动快照（修订 1）");

        let checked = json_at(&package.join("drc-report.json"));
        let results = checked["results"].as_array().expect("检查结果");
        let open = |severity: &str| {
            results
                .iter()
                .filter(|result| result["severity"] == severity && result["waived"] != true)
                .count()
        };
        let gates = &manifest["gates"];
        assert_eq!(gates["drc_errors"], 0, "{gates}");
        assert!(open("警告") > 0, "{checked}");
        assert_eq!(gates["drc_warnings"], open("警告"), "{gates}");
        assert_eq!(gates["drc_hints"], open("提示"), "{gates}");
    }

    /// PUB-4 验收第 2 条：有没豁免的错误时 `PUBLISH_BLOCKED`，项目目录里不多任何文件、目录，也没有快照；把错误
    /// 豁免掉、存盘后发布成了，豁免的那条在 `drc-report.json` 里。再发布一次是第 2 版，两个包都在；`REV` 不大于
    /// 已有的最大修订号是 `PUBLISH_REVISION_INVALID`，什么也不写。
    #[test]
    fn pidpublish_blocks_until_the_errors_are_waived_and_numbers_each_publication() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-010");
        let mut app = sheet(&scratch);
        let current = scratch.path("PID-1101.dwg");
        let before = scratch.listing();
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.code.as_deref(), Some(PUBLISH_BLOCKED), "{}", ran.said);
        assert_eq!(scratch.listing(), before, "拦下了还写了东西");

        waive_all(&mut app, "DR-010", &current);
        let published = scratch.path(DEFAULT_TARGET);
        for revision in 1..=2 {
            let ran = run(&mut app, "PIDPUBLISH");
            assert_eq!(ran.error, None, "第 {revision} 次：{}", ran.said);
            line_of(&ran.said, &format!("PIDPUBLISH: 已发布第 {revision} 版："));
        }
        assert_eq!(names(&published), ["PID-1101-r001", "PID-1101-r002"]);
        for package in ["PID-1101-r001", "PID-1101-r002"] {
            publish_pkg::verify_package(&published.join(package)).expect("自检过得了");
        }
        let latest = published.join("PID-1101-r002");
        let checked = json_at(&latest.join("drc-report.json"));
        let waived = checked["results"]
            .as_array()
            .expect("检查结果")
            .iter()
            .filter(|result| result["rule_id"] == "DR-010" && result["waived"] == true)
            .count();
        assert!(waived > 0, "{checked}");
        let manifest = json_at(&latest.join("manifest.json"));
        assert_eq!(manifest["gates"]["waived"], waived, "{manifest}");
        assert_eq!(manifest["snapshot_seq"], 2, "{manifest}");
        assert_eq!(snapshots(&scratch), ["0001.json", "0002.json"]);

        let before = scratch.listing();
        for line in ["PIDPUBLISH REV 2", "pidpublish rev 1"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(PUBLISH_REVISION_INVALID),
                "{line}: {}",
                ran.said
            );
        }
        assert_eq!(scratch.listing(), before, "修订号不对还写了东西");
    }

    /// PUB-4 验收第 3 条：出包中途哪一步失败（测试钩子让它失败；自检那一步是真的改坏了临时目录里的文件）都是
    /// `PUBLISH_FAILED`，说是哪一步；包和临时目录都不留——这次建的目标目录也删了；快照存下了的留着；图的
    /// 「已修改」、撤销栈、磁盘上的文件都不变。之后不带钩子发布是第 1 版，顺手清掉以前崩溃留下的临时目录，
    /// 别的以点开头的东西不动。
    #[test]
    fn pidpublish_that_fails_midway_leaves_no_package_and_does_not_touch_the_drawing() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let current = scratch.path("PID-1101.dwg");
        let published = scratch.path(DEFAULT_TARGET);
        let before = drawing_state(&app, &current);
        let mut saved = 0;
        for step in ["snapshot", "plot", "bom", "write", "verify", "commit"] {
            let ran = run_broken(&mut app, step, "PIDPUBLISH");
            assert_eq!(
                ran.code.as_deref(),
                Some(PUBLISH_FAILED),
                "{step}: {}",
                ran.said
            );
            let error = ran.error.expect("说了失败");
            assert!(error.contains(&format!("（step {step}）")), "{error}");
            if step != "snapshot" {
                saved += 1;
                assert!(error.contains("临时目录已删"), "{error}");
            }
            assert_eq!(snapshots(&scratch).len(), saved, "{step}: 快照");
            assert!(
                !published.exists(),
                "{step}: 留下了 {:?}",
                names(&published)
            );
            assert_eq!(drawing_state(&app, &current), before, "{step}: 图动了");
        }

        let leftover = published.join(".publishing-PID-1101-r001-1-1");
        fs::create_dir_all(&leftover).expect("崩溃留下的临时目录");
        fs::write(leftover.join("PID-1101.pdf"), b"half").expect("写了一半的出图");
        fs::create_dir_all(published.join(".notes")).expect("别人的目录");
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(names(&published), [".notes", "PID-1101-r001"]);
        assert_eq!(drawing_state(&app, &current), before, "发布动了图");
    }

    /// PUB-4 验收第 3 条末尾：从命令行敲 `PIDPUBLISH <目录>` 只发布一次，只发到那个目录。
    #[test]
    fn pidpublish_to_a_folder_from_the_command_line_publishes_once_there() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let ran = run(&mut app, "PIDPUBLISH \"out dir\"");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(
            ran.said.matches("PIDPUBLISH: 已发布").count(),
            1,
            "{}",
            ran.said
        );
        assert_eq!(names(&scratch.path("out dir")), ["PID-1101-r001"]);
        assert!(!scratch.path(DEFAULT_TARGET).exists(), "发到了默认目录");
        assert_eq!(snapshots(&scratch), ["0001.json"]);
    }

    /// PNL-8：「校验与交付」面板读过这张图的快照列表，发布以后重读：发布前的快照列进来。
    #[test]
    fn pidpublish_has_the_pid_verify_panel_read_its_snapshots_again() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let ran = run(&mut app, "PIDVERIFY DELIVER");
        assert_eq!(ran.error, None, "{}", ran.said);
        let listed = |app: &OpenCADStudio| -> Vec<u64> {
            let rows = &app.pid_verify_view.deliver.rows;
            rows.iter().map(|row| row.info.seq).collect()
        };
        assert_eq!(listed(&app), Vec::<u64>::new());
        let ran = run(&mut app, "PIDPUBLISH \"out dir\"");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(listed(&app), [1]);
    }

    /// PUB-4 验收第 4 条：带同步记录的图，队列不空、标了不一致、没连着时发布各报对应的码，什么也不写——连
    /// 快照也没有；没连着而带 `ALLOW_OFFLINE` 发布成了，清单的 `collab` 是图里的同步记录、`reconciled` 为假；
    /// 连着并且补齐了再发布，`reconciled` 为真。没有同步记录时 `collab` 是 `null`，见验收第 1 条的测试。
    #[test]
    fn pidpublish_with_a_sync_record_publishes_only_once_the_collab_preconditions_pass() {
        let record = FileCollabState {
            drawing_id: "PID-1101".to_owned(),
            repo_id: "repo-1".to_owned(),
            revision: 3,
        };
        let queued = QueuedOp {
            op_id: "op-1".to_owned(),
            payload: json!({ "changes": [] }),
            base_revision: 3,
            on: true,
            label: "改位号".to_owned(),
            user_id: Some("u-1".to_owned()),
            queued_at: Some(1_795_000_000),
            inverse: None,
        };
        let apart = Diverged {
            code: collab_state::REVISION_GAP.to_owned(),
            message: "缺了修订 4".to_owned(),
            at: 1_795_000_000,
        };
        let refused = |app: &mut OpenCADStudio, scratch: &Scratch, code: &str| {
            let before = scratch.listing();
            for line in ["PIDPUBLISH", "PIDPUBLISH ALLOW_OFFLINE"] {
                let ran = run(app, line);
                assert_eq!(ran.code.as_deref(), Some(code), "{line}: {}", ran.said);
                assert_eq!(scratch.listing(), before, "{line} 写了东西");
            }
        };

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
            collab_state::store_queue(document, std::slice::from_ref(&queued));
        });
        refused(&mut app, &scratch, COLLAB_UNSYNCED);

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state_diverged(document, Some(&record), Some(&apart));
        });
        refused(&mut app, &scratch, COLLAB_DIVERGED);

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
        });
        let before = scratch.listing();
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.code.as_deref(), Some(COLLAB_OFFLINE), "{}", ran.said);
        assert_eq!(scratch.listing(), before, "没连着还写了东西");
        let ran = run(&mut app, "PIDPUBLISH ALLOW_OFFLINE");
        assert_eq!(ran.error, None, "{}", ran.said);
        let published = scratch.path(DEFAULT_TARGET);
        let manifest = json_at(&published.join("PID-1101-r001").join("manifest.json"));
        let offline = json!({
            "drawing_id": "PID-1101",
            "repo_id": "repo-1",
            "revision": 3,
            "reconciled": false,
        });
        assert_eq!(manifest["collab"], offline, "{manifest}");

        let i = app.active_tab;
        app.collab.document = Some(app.tabs[i].id);
        app.collab.session_id = Some("s-1".to_owned());
        app.collab.connected = true;
        let claim = FileClaim {
            drawing_id: record.drawing_id.clone(),
            repo_id: record.repo_id.clone(),
            revision: record.revision,
        };
        assert!(app.collab.seed_file_state(&claim, Vec::new(), Vec::new()));
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.error, None, "{}", ran.said);
        let manifest = json_at(&published.join("PID-1101-r002").join("manifest.json"));
        assert_eq!(manifest["collab"]["reconciled"], true, "{manifest}");
    }

    /// PUB-5 验收第 1 条：没发布过是「未发布」；发布后「已发布，修订 1」，那个包标当前；改了图
    /// 没存、存了都是「修订 1 之后有改动」；再发布两次是「已发布，修订 3」，新的在前，两版源文件
    /// 相同时只有新的那版标当前。`LIST` 什么都不写。
    #[test]
    fn pidpublish_list_says_which_revision_the_drawing_was_published_to() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let current = scratch.path("PID-1101.dwg");
        let listed = |app: &mut OpenCADStudio, line: &str| {
            let before = scratch.listing();
            let ran = run(app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(scratch.listing(), before, "{line} 写了东西");
            ran.said
        };
        let rows = |said: &str| -> Vec<String> {
            said.lines()
                .filter(|line| line.starts_with("  PID-1101-r"))
                .map(str::to_owned)
                .collect()
        };

        let said = listed(&mut app, "PIDPUBLISH LIST");
        let head = line_of(&said, "PIDPUBLISH LIST: 图 PID-1101：");
        assert!(head.contains("未发布"), "{said}");
        assert!(rows(&said).is_empty(), "{said}");

        assert_eq!(run(&mut app, "PIDPUBLISH").error, None);
        let said = listed(&mut app, "PIDPUBLISH LIST");
        let head = line_of(&said, "PIDPUBLISH LIST: 图 PID-1101：");
        assert!(head.contains("已发布，修订 1；"), "{said}");
        let found = rows(&said);
        assert_eq!(found.len(), 1, "{said}");
        assert!(found[0].starts_with("  PID-1101-r001  "), "{said}");
        assert!(
            found[0].contains(" UTC  ") && found[0].ends_with("当前"),
            "{said}"
        );

        draw(&mut app, "PIDINSERT pump", &[[100.0, 0.0]]);
        assert!(app.tabs[app.active_tab].dirty);
        let said = listed(&mut app, "PIDPUBLISH LIST");
        assert!(said.contains("修订 1 之后有改动；"), "{said}");
        assert!(!rows(&said)[0].ends_with("当前"), "{said}");
        save(&mut app, &current);
        let said = listed(&mut app, "PIDPUBLISH LIST");
        assert!(said.contains("修订 1 之后有改动；"), "{said}");
        assert!(!rows(&said)[0].ends_with("当前"), "{said}");

        for _ in 0..2 {
            let ran = run(&mut app, "PIDPUBLISH");
            assert_eq!(ran.error, None, "{}", ran.said);
        }
        let said = listed(&mut app, "pidpublish list");
        assert!(said.contains("已发布，修订 3；"), "{said}");
        let found = rows(&said);
        assert_eq!(found.len(), 3, "{said}");
        for (row, (package, current)) in found.iter().zip([
            ("PID-1101-r003", true),
            ("PID-1101-r002", false),
            ("PID-1101-r001", false),
        ]) {
            assert!(row.starts_with(&format!("  {package}  ")), "{said}");
            assert_eq!(row.ends_with("当前"), current, "{said}");
        }

        let i = app.active_tab;
        let publications = app.pidpublish_listed(i, None).expect("发布记录");
        assert_eq!(publications.state, PublishState::Published(3));
        assert_eq!(publications.target_dir, scratch.path("pid-publish"));
        let revisions: Vec<u32> = publications
            .packages
            .iter()
            .map(|package| package.revision)
            .collect();
        assert_eq!(revisions, [3, 2, 1]);
        assert!(publications
            .packages
            .iter()
            .all(|package| package.published_at.is_some() && package.published_by.is_some()));

        let said = listed(&mut app, "PIDPUBLISH LIST \"out dir\"");
        assert!(said.contains("未发布"), "{said}");
        assert!(!scratch.path("out dir").exists());
    }

    /// PUB-5 验收第 2 条：完好的包通过（相对路径从图纸所在目录算，也可以写完整路径）；目录不在是
    /// `PACKAGE_NOT_FOUND`；动过一个字节是 `PACKAGE_CORRUPT`。都在只读会话里做，`LIST` 也照用。
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn pidpublish_verify_checks_a_package_even_in_a_readonly_session() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.error, None, "{}", ran.said);
        let package = scratch.path("pid-publish").join("PID-1101-r001");

        app.collab.readonly = true;
        let ran = run(&mut app, "PIDPUBLISH LIST");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(ran.said.contains("已发布，修订 1"), "{}", ran.said);
        let full = format!("PIDPUBLISH VERIFY \"{}\"", package.display());
        for line in ["PIDPUBLISH VERIFY pid-publish/PID-1101-r001", full.as_str()] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            let said = line_of(&ran.said, "PIDPUBLISH VERIFY: ");
            assert!(said.contains("完好：PID-1101-r001"), "{line}: {said}");
            assert!(said.contains("共 11 个文件"), "{line}: {said}");
        }
        let ran = run(&mut app, "pidpublish verify pid-publish/PID-1101-r002");
        assert_eq!(ran.code.as_deref(), Some(PACKAGE_NOT_FOUND), "{}", ran.said);

        let bom = package.join("bom.csv");
        let mut permissions = fs::metadata(&bom).expect("材料清单").permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&bom, permissions).expect("去掉只读");
        let mut bytes = fs::read(&bom).expect("读材料清单");
        bytes[0] ^= 1;
        fs::write(&bom, bytes).expect("改一个字节");
        let ran = run(&mut app, "PIDPUBLISH VERIFY pid-publish/PID-1101-r001");
        assert_eq!(ran.code.as_deref(), Some(PACKAGE_CORRUPT), "{}", ran.said);
        assert!(
            ran.error.is_some_and(|error| error.contains("bom.csv")),
            "{}",
            ran.said
        );
    }

    /// 发 `ask`，应答 `scanning` 就再发：测试里后台读随应答跑完，再发就是结论。
    fn settled(app: &mut OpenCADStudio, ask: &Value) -> Value {
        for _ in 0..5 {
            let answer = request(app, ask.clone());
            if answer["status"] != "scanning" {
                return answer;
            }
        }
        panic!("{ask}：一直在扫");
    }

    /// `pid_publish_check` 的七条前提里只有 `failing` 这几条为假。
    fn only_failing(answer: &Value, failing: &[&str]) {
        let preconditions = answer["preconditions"].as_object().expect("前提逐条的结论");
        assert_eq!(preconditions.len(), 7, "{answer}");
        for (key, holds) in preconditions {
            let expected = !failing.contains(&key.as_str());
            assert_eq!(holds, &json!(expected), "{key}: {answer}");
        }
    }

    /// PUB-6 验收第 1 条：`pid_publish_check` 答前提逐条的结论、四项门禁、目标目录和这次会用的修订号，前提和
    /// 门禁都过了 `can_publish` 才为真；是查询，不改图、不写东西。图有没保存的修改、磁盘上的文件对不上、会话
    /// 只读时各是那一条为假，门禁照算；没保存过的图门禁、目标目录、修订号都是 `null`。`dir`、`rev` 同命令的
    /// 目标目录和 `REV`，写错是 `invalid_request` 或 §3.6 的码。能力清单里有 `pid_publish`。
    #[test]
    fn pid_publish_check_op_answers_each_precondition_the_gates_and_the_target() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let current = scratch.path("PID-1101.dwg");
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        assert!(
            state["capabilities"]
                .as_array()
                .is_some_and(|names| names.contains(&json!("pid_publish"))),
            "{state}"
        );
        let before = scratch.listing();
        let drawing = drawing_state(&app, &current);
        let checked = |app: &mut OpenCADStudio| request(app, json!({"op": "pid_publish_check"}));

        let answer = checked(&mut app);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["can_publish"], true, "{answer}");
        only_failing(&answer, &[]);
        let gates = &answer["gates"];
        assert_eq!(gates["drc_errors"], 0, "{answer}");
        assert_eq!(gates["duplicate_tags"], 0, "{answer}");
        assert_eq!(gates["master_data"], "not_configured", "{answer}");
        assert_eq!(gates["index_complete"], true, "{answer}");
        assert_eq!(gates["result"], publish_pkg::PASSED, "{answer}");
        let target = scratch.path(DEFAULT_TARGET).display().to_string();
        assert_eq!(answer["target_dir"], target, "{answer}");
        assert_eq!(answer["next_revision"], 1, "{answer}");
        assert_eq!(scratch.listing(), before, "检查写了东西");
        assert_eq!(drawing_state(&app, &current), drawing, "检查动了图");

        let ask = json!({"op": "pid_publish_check", "dir": "out dir", "rev": 4});
        let answer = request(&mut app, ask);
        let target = scratch.path("out dir").display().to_string();
        assert_eq!(answer["target_dir"], target, "{answer}");
        assert_eq!(answer["next_revision"], 4, "{answer}");

        let i = app.active_tab;
        let failing = |app: &mut OpenCADStudio, which: &str| {
            let answer = checked(app);
            assert_eq!(answer["ok"], true, "{which}: {answer}");
            assert_eq!(answer["can_publish"], false, "{which}: {answer}");
            only_failing(&answer, &[which]);
            let result = &answer["gates"]["result"];
            assert_eq!(result, publish_pkg::PASSED, "{which}: {answer}");
        };
        app.tabs[i].dirty = true;
        failing(&mut app, "clean");
        app.tabs[i].dirty = false;
        let recorded = app.tabs[i].disk_fingerprint.take();
        failing(&mut app, "file_unchanged");
        app.tabs[i].disk_fingerprint = recorded;
        app.collab.readonly = true;
        failing(&mut app, "writable_session");
        app.collab.readonly = false;

        fs::write(scratch.path("a file.txt"), b"not a folder").expect("写文件");
        for (ask, code) in [
            (json!({"dir": "a file.txt"}), PUBLISH_TARGET_INVALID),
            (json!({"rev": 0}), PUBLISH_REVISION_INVALID),
            (json!({"rev": -1}), PUBLISH_REVISION_INVALID),
            (json!({"rev": "2"}), PUBLISH_REVISION_INVALID),
            (json!({"dir": ""}), "invalid_request"),
            (json!({"dir": 7}), "invalid_request"),
            (json!({"allow_offline": "yes"}), "invalid_request"),
        ] {
            let mut ask = ask;
            ask["op"] = json!("pid_publish_check");
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }

        let mut unsaved = OpenCADStudio::new_for_test();
        let _ = unsaved.automation_op(r#"{"op":"new"}"#);
        let answer = checked(&mut unsaved);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["can_publish"], false, "{answer}");
        assert_eq!(answer["preconditions"]["saved"], false, "{answer}");
        assert_eq!(answer["preconditions"]["file_unchanged"], false, "{answer}");
        for key in ["gates", "target_dir", "next_revision"] {
            assert_eq!(answer[key], Value::Null, "{key}: {answer}");
        }
    }

    /// 项目里还有没打开的图时 `pid_publish_check` 先应答 `scanning`（读了几个、共几个），读完再发才是结论：
    /// 泵没填名称、位号另一张图上也有，门禁拦下（`result` 是 `PUBLISH_BLOCKED`），前提都过了 `can_publish` 也为假。
    #[test]
    fn pid_publish_check_op_answers_scanning_then_the_blocked_gates() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        other_sheet(&scratch);
        let ask = json!({"protocol": 1, "op": "pid_publish_check"});
        let (answer, task) = app.control_request(ask.clone());
        assert_eq!(
            answer,
            json!({"ok": true, "status": "scanning", "done": 0, "total": 1})
        );
        app.drive_headless_task(task).expect("后台读");
        let (answer, _) = app.control_request(ask);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["can_publish"], false, "{answer}");
        only_failing(&answer, &[]);
        let gates = &answer["gates"];
        let errors = gates["drc_errors"].as_u64().unwrap_or_default();
        assert!(errors > 0, "{answer}");
        assert_eq!(gates["duplicate_tags"], 1, "{answer}");
        assert_eq!(gates["result"], PUBLISH_BLOCKED, "{answer}");
    }

    /// PUB-6 验收第 3 条：带同步记录的图，`preconditions` 里队列空、没有不一致、连着并补齐三条（`collab_synced`、
    /// `collab_consistent`、`collab_online`）各自照实答，不在先不满足的那条停下；有一条不满足 `can_publish` 就为假，
    /// `allow_offline` 只放过没连着，`collab_online` 照实为假。连着并补齐了三条都为真。
    #[test]
    fn pid_publish_check_op_answers_the_collab_preconditions_one_by_one() {
        let record = FileCollabState {
            drawing_id: "PID-1101".to_owned(),
            repo_id: "repo-1".to_owned(),
            revision: 3,
        };
        let queued = QueuedOp {
            op_id: "op-1".to_owned(),
            payload: json!({ "changes": [] }),
            base_revision: 3,
            on: true,
            label: "改位号".to_owned(),
            user_id: Some("u-1".to_owned()),
            queued_at: Some(1_795_000_000),
            inverse: None,
        };
        let apart = Diverged {
            code: collab_state::REVISION_GAP.to_owned(),
            message: "缺了修订 4".to_owned(),
            at: 1_795_000_000,
        };
        let checked = |app: &mut OpenCADStudio, allow_offline: bool| {
            let ask = json!({"op": "pid_publish_check", "allow_offline": allow_offline});
            request(app, ask)
        };

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
            collab_state::store_queue(document, std::slice::from_ref(&queued));
        });
        for allow_offline in [false, true] {
            let answer = checked(&mut app, allow_offline);
            only_failing(&answer, &["collab_synced", "collab_online"]);
            assert_eq!(answer["can_publish"], false, "{answer}");
        }

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state_diverged(document, Some(&record), Some(&apart));
        });
        for allow_offline in [false, true] {
            let answer = checked(&mut app, allow_offline);
            only_failing(&answer, &["collab_consistent", "collab_online"]);
            assert_eq!(answer["can_publish"], false, "{answer}");
        }

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
        });
        for allow_offline in [false, true] {
            let answer = checked(&mut app, allow_offline);
            only_failing(&answer, &["collab_online"]);
            assert_eq!(answer["can_publish"], allow_offline, "{answer}");
        }
        let i = app.active_tab;
        app.collab.document = Some(app.tabs[i].id);
        app.collab.session_id = Some("s-1".to_owned());
        app.collab.connected = true;
        let claim = FileClaim {
            drawing_id: record.drawing_id.clone(),
            repo_id: record.repo_id.clone(),
            revision: record.revision,
        };
        assert!(app.collab.seed_file_state(&claim, Vec::new(), Vec::new()));
        let answer = checked(&mut app, false);
        only_failing(&answer, &[]);
        assert_eq!(answer["can_publish"], true, "{answer}");
    }

    /// PUB-6 验收第 1 条：`pid_publish` 同 `PIDPUBLISH`，结果是包名、包的目录、修订号、发布前快照的序号、文件数、
    /// 字节数和门禁；包校验得过，图不动。`rev`、`dir` 同命令的 `REV` 和目标目录。经 REST 是 201，见 `rest` 的测试。
    #[test]
    fn pid_publish_op_publishes_a_package_and_answers_what_it_wrote() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let current = scratch.path("PID-1101.dwg");
        let before = drawing_state(&app, &current);

        let wrote = settled(&mut app, &json!({"op": "pid_publish"}));
        assert_eq!(wrote["ok"], true, "{wrote}");
        let package = scratch.path(DEFAULT_TARGET).join("PID-1101-r001");
        let result = &wrote["result"];
        assert_eq!(result["package_id"], "PID-1101-r001", "{wrote}");
        assert_eq!(result["path"], package.display().to_string(), "{wrote}");
        assert_eq!(result["revision"], 1, "{wrote}");
        assert_eq!(result["snapshot_seq"], 1, "{wrote}");
        assert_eq!(result["files"], 11, "{wrote}");
        let bytes: u64 = names(&package)
            .iter()
            .map(|name| fs::metadata(package.join(name)).expect("包里的文件").len())
            .sum();
        assert_eq!(result["bytes"], bytes, "{wrote}");
        let manifest = json_at(&package.join("manifest.json"));
        assert_eq!(result["gates"], manifest["gates"], "{wrote}");
        assert_eq!(result["gates"]["result"], publish_pkg::PASSED, "{wrote}");
        publish_pkg::verify_package(&package).expect("自检过得了");
        assert_eq!(drawing_state(&app, &current), before, "发布动了图");

        let ask = json!({"op": "pid_publish", "rev": 5, "dir": "out dir"});
        let wrote = settled(&mut app, &ask);
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"]["package_id"], "PID-1101-r005", "{wrote}");
        assert_eq!(wrote["result"]["snapshot_seq"], 2, "{wrote}");
        assert_eq!(names(&scratch.path("out dir")), ["PID-1101-r005"]);
    }

    /// PUB-6 验收第 1 条：`plot` 交给出图，字段同 `plot` 操作，PDF 和 SVG 都用它——A3 竖放、横放出来的 SVG 各是
    /// 297×420、420×297 毫米；纸张不认识时出包在出图这一步失败（`PUBLISH_FAILED`，`step` 是 `plot`），不留包。
    /// `plot` 不是对象、带 `path`、`layout` 是 `all` 都是 `invalid_request`，什么也不写。
    #[test]
    fn pid_publish_op_hands_plot_to_both_plots() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let published = scratch.path(DEFAULT_TARGET);
        for (package, orientation, size) in [
            (
                "PID-1101-r001",
                "Portrait",
                "width=\"297mm\" height=\"420mm\"",
            ),
            (
                "PID-1101-r002",
                "Landscape",
                "width=\"420mm\" height=\"297mm\"",
            ),
        ] {
            let plot = json!({"paper": "A3", "orientation": orientation});
            let wrote = settled(&mut app, &json!({"op": "pid_publish", "plot": plot}));
            assert_eq!(wrote["ok"], true, "{orientation}: {wrote}");
            let svg =
                fs::read_to_string(published.join(package).join("PID-1101.svg")).expect("SVG");
            let head = svg.lines().take(3).collect::<Vec<_>>().join("\n");
            assert!(svg.contains(size), "{orientation}: {head}");
        }

        let ask = json!({"op": "pid_publish", "plot": {"paper": "No Such Paper"}});
        let failed = settled(&mut app, &ask);
        assert_eq!(failed["ok"], false, "{failed}");
        assert_eq!(failed["code"], PUBLISH_FAILED, "{failed}");
        assert_eq!(failed["step"], "plot", "{failed}");
        let error = failed["error"].as_str().unwrap_or_default();
        assert!(error.contains("invalid_paper"), "{failed}");
        assert_eq!(names(&published), ["PID-1101-r001", "PID-1101-r002"]);

        let before = scratch.listing();
        for plot in [
            json!("A3"),
            json!({"path": "elsewhere.pdf"}),
            json!({"layout": "ALL"}),
        ] {
            let refused = request(&mut app, json!({"op": "pid_publish", "plot": plot}));
            assert_eq!(refused["code"], "invalid_request", "{plot}: {refused}");
        }
        assert_eq!(scratch.listing(), before, "plot 写错了还写了东西");
    }

    /// PUB-6 验收第 2 条：门禁没过的 `pid_publish` 是 `PUBLISH_BLOCKED`，带四项门禁、重复位号的各处和检查报告；
    /// 项目里还有没打开的图时先应答 `scanning`。什么也不写——快照也没有。
    #[test]
    fn pid_publish_op_answers_blocked_with_the_gates_and_the_duplicates() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        other_sheet(&scratch);
        let before = scratch.listing();
        let first = request(&mut app, json!({"op": "pid_publish"}));
        assert_eq!(first["status"], "scanning", "{first}");
        assert_eq!(first["total"], 1, "{first}");

        let blocked = settled(&mut app, &json!({"op": "pid_publish"}));
        assert_eq!(blocked["ok"], false, "{blocked}");
        assert_eq!(blocked["code"], PUBLISH_BLOCKED, "{blocked}");
        let gates = &blocked["gates"];
        let errors = gates["drc_errors"].as_u64().unwrap_or_default();
        assert!(errors > 0, "{blocked}");
        assert_eq!(gates["duplicate_tags"], 1, "{blocked}");
        assert_eq!(gates["result"], PUBLISH_BLOCKED, "{blocked}");
        let duplicates = blocked["duplicates"].as_array().expect("重复位号");
        assert_eq!(duplicates.len(), 1, "{blocked}");
        let mut drawings: Vec<&str> = duplicates[0]["places"]
            .as_array()
            .expect("各处")
            .iter()
            .filter_map(|place| place["drawing_id"].as_str())
            .collect();
        drawings.sort_unstable();
        assert_eq!(drawings, ["PID-1101", "PID-1102"], "{blocked}");
        let results = blocked["report"]["results"].as_array().expect("检查报告");
        assert!(!results.is_empty(), "{blocked}");
        let error = blocked["error"].as_str().unwrap_or_default();
        assert!(
            error.contains(&format!("{errors} 条错误、1 个重复位号")),
            "{blocked}"
        );
        assert_eq!(scratch.listing(), before, "拦下了还写了东西");
    }

    /// PUB-6 验收第 2 条：只读会话里 `pid_publish` 是 `COLLAB_READONLY`，带拒绝的级别；没保存的修改是
    /// `PUBLISH_UNSAVED`，修订号不大于已有的或不是正整数是 `PUBLISH_REVISION_INVALID`，这一版的名字被占了是
    /// `PUBLISH_TARGET_EXISTS`；请求写错是 `invalid_request`。都不写任何东西。
    #[test]
    fn pid_publish_op_is_refused_by_a_readonly_session_and_by_the_preconditions() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let i = app.active_tab;
        let published = scratch.path(DEFAULT_TARGET);
        fs::create_dir_all(published.join("PID-1101-r002")).expect("建旧包");
        fs::write(published.join("PID-1101-r003"), b"in the way").expect("占位");
        let before = scratch.listing();

        app.collab.readonly = true;
        let refused = request(&mut app, json!({"op": "pid_publish", "rev": 4}));
        assert_eq!(refused["code"], "COLLAB_READONLY", "{refused}");
        assert_eq!(refused["level"], "state", "{refused}");
        app.collab.readonly = false;
        for (ask, code) in [
            (json!({}), PUBLISH_TARGET_EXISTS),
            (json!({"rev": 2}), PUBLISH_REVISION_INVALID),
            (json!({"rev": 1.5}), PUBLISH_REVISION_INVALID),
            (json!({"dir": ""}), "invalid_request"),
            (json!({"allow_offline": 1}), "invalid_request"),
        ] {
            let mut ask = ask;
            ask["op"] = json!("pid_publish");
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }
        app.tabs[i].dirty = true;
        let refused = request(&mut app, json!({"op": "pid_publish", "rev": 4}));
        assert_eq!(refused["code"], PUBLISH_UNSAVED, "{refused}");
        assert_eq!(scratch.listing(), before, "被拒了还写了东西");
    }

    /// PUB-6 验收第 3 条：带同步记录的图，队列不空、标了不一致、没连着时 `pid_publish` 各是 `COLLAB_UNSYNCED`、
    /// `COLLAB_DIVERGED`、`COLLAB_OFFLINE`，带 `allow_offline: true` 也只放过没连着，被拒时什么也不写；没连着而
    /// 带 `allow_offline: true` 发布成了，清单的 `collab` 记 `reconciled` 为假。
    #[test]
    fn pid_publish_op_publishes_offline_only_with_allow_offline() {
        let record = FileCollabState {
            drawing_id: "PID-1101".to_owned(),
            repo_id: "repo-1".to_owned(),
            revision: 3,
        };
        let queued = QueuedOp {
            op_id: "op-1".to_owned(),
            payload: json!({ "changes": [] }),
            base_revision: 3,
            on: true,
            label: "改位号".to_owned(),
            user_id: Some("u-1".to_owned()),
            queued_at: Some(1_795_000_000),
            inverse: None,
        };
        let apart = Diverged {
            code: collab_state::REVISION_GAP.to_owned(),
            message: "缺了修订 4".to_owned(),
            at: 1_795_000_000,
        };
        let refused = |app: &mut OpenCADStudio, scratch: &Scratch, code: &str| {
            let before = scratch.listing();
            for allow_offline in [false, true] {
                let ask = json!({"op": "pid_publish", "allow_offline": allow_offline});
                let answer = request(app, ask);
                assert_eq!(answer["code"], code, "{allow_offline}: {answer}");
                assert_eq!(scratch.listing(), before, "{allow_offline}: 写了东西");
            }
        };

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
            collab_state::store_queue(document, std::slice::from_ref(&queued));
        });
        refused(&mut app, &scratch, COLLAB_UNSYNCED);

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state_diverged(document, Some(&record), Some(&apart));
        });
        refused(&mut app, &scratch, COLLAB_DIVERGED);

        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet_with(&scratch, |document| {
            collab_state::store_state(document, Some(&record));
        });
        let before = scratch.listing();
        let answer = request(&mut app, json!({"op": "pid_publish"}));
        assert_eq!(answer["code"], COLLAB_OFFLINE, "{answer}");
        assert_eq!(scratch.listing(), before, "没连着还写了东西");
        let wrote = settled(
            &mut app,
            &json!({"op": "pid_publish", "allow_offline": true}),
        );
        assert_eq!(wrote["ok"], true, "{wrote}");
        let package = scratch.path(DEFAULT_TARGET).join("PID-1101-r001");
        let manifest = json_at(&package.join("manifest.json"));
        let offline = json!({
            "drawing_id": "PID-1101",
            "repo_id": "repo-1",
            "revision": 3,
            "reconciled": false,
        });
        assert_eq!(manifest["collab"], offline, "{manifest}");
    }

    /// PUB-6 验收第 2 条：出包中途失败的 `pid_publish` 是 `PUBLISH_FAILED`，`step` 是哪一步；不留包和临时目录，
    /// 存下了的快照留着。
    #[test]
    fn pid_publish_op_that_fails_midway_answers_the_step() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let published = scratch.path(DEFAULT_TARGET);
        for (step, saved) in [("snapshot", 0), ("bom", 1), ("commit", 2)] {
            BREAK_AT.with(|at| at.set(Some(step)));
            let failed = request(&mut app, json!({"op": "pid_publish"}));
            BREAK_AT.with(|at| at.set(None));
            assert_eq!(failed["ok"], false, "{step}: {failed}");
            assert_eq!(failed["code"], PUBLISH_FAILED, "{step}: {failed}");
            assert_eq!(failed["step"], step, "{step}: {failed}");
            assert_eq!(snapshots(&scratch).len(), saved, "{step}: 快照");
            assert!(
                !published.exists(),
                "{step}: 留下了 {:?}",
                names(&published)
            );
        }
    }

    /// PUB-6 验收第 1 条：`pid_publications` 同 `PIDPUBLISH LIST`——目标目录、发布到哪一版了（`state`）和各包，新的
    /// 在前，每个带包的目录；`pid_publish_verify` 同 `PIDPUBLISH VERIFY`——完好的包答 §3.5 的报告，目录不在是
    /// `PACKAGE_NOT_FOUND`，动过一个字节是 `PACKAGE_CORRUPT` 并带报告，不给 `path` 是 `invalid_request`。两个都是
    /// 查询：只读会话里、自动化停了也照用，`pid_publish` 这时是 `disabled`。图没保存过时 `pid_publications` 是
    /// `PUBLISH_UNSAVED`。
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn pid_publish_ops_list_the_packages_and_verify_one_as_queries() {
        let scratch = Scratch::new();
        only_rule(&scratch, "DR-001");
        let mut app = sheet(&scratch);
        let listed = request(&mut app, json!({"op": "pid_publications"}));
        assert_eq!(listed["ok"], true, "{listed}");
        assert_eq!(listed["state"], "未发布", "{listed}");
        assert_eq!(listed["packages"], json!([]), "{listed}");
        for _ in 0..2 {
            let wrote = settled(&mut app, &json!({"op": "pid_publish"}));
            assert_eq!(wrote["ok"], true, "{wrote}");
        }

        app.collab.readonly = true;
        let stopped = request(&mut app, json!({"op": "stop"}));
        assert_eq!(stopped["ok"], true, "{stopped}");
        let refused = request(&mut app, json!({"op": "pid_publish"}));
        assert_eq!(refused["code"], "disabled", "{refused}");
        let listed = request(&mut app, json!({"op": "pid_publications"}));
        assert_eq!(listed["ok"], true, "{listed}");
        let published = scratch.path(DEFAULT_TARGET);
        assert_eq!(listed["drawing_id"], "PID-1101", "{listed}");
        assert_eq!(
            listed["target_dir"],
            published.display().to_string(),
            "{listed}"
        );
        assert_eq!(listed["state"], "已发布，修订 2", "{listed}");
        let packages = listed["packages"].as_array().expect("各包");
        let ids: Vec<&Value> = packages
            .iter()
            .map(|package| &package["package_id"])
            .collect();
        assert_eq!(ids, ["PID-1101-r002", "PID-1101-r001"], "{listed}");
        let newest = &packages[0];
        assert_eq!(newest["revision"], 2, "{listed}");
        let path = published.join("PID-1101-r002");
        assert_eq!(newest["path"], path.display().to_string(), "{listed}");
        assert_eq!(newest["current"], true, "{listed}");
        assert!(newest["published_at"].is_i64(), "{listed}");
        assert!(newest["published_by"].is_string(), "{listed}");
        assert_eq!(packages[1]["current"], false, "{listed}");

        let ask = json!({"op": "pid_publish_verify", "path": "pid-publish/PID-1101-r002"});
        let verified = request(&mut app, ask);
        let report = json!({
            "ok": true,
            "package_id": "PID-1101-r002",
            "revision": 2,
            "files": 11,
            "missing": [],
            "mismatched": [],
            "extra": [],
        });
        assert_eq!(verified, report);
        for (ask, code) in [
            (
                json!({"path": "pid-publish/PID-1101-r009"}),
                PACKAGE_NOT_FOUND,
            ),
            (json!({}), "invalid_request"),
            (json!({"path": 7}), "invalid_request"),
        ] {
            let mut ask = ask;
            ask["op"] = json!("pid_publish_verify");
            let answer = request(&mut app, ask.clone());
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }

        let bom = path.join("bom.csv");
        let mut permissions = fs::metadata(&bom).expect("材料清单").permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&bom, permissions).expect("去掉只读");
        let mut bytes = fs::read(&bom).expect("读材料清单");
        bytes[0] ^= 1;
        fs::write(&bom, bytes).expect("改一个字节");
        let full = path.display().to_string();
        let corrupt = request(&mut app, json!({"op": "pid_publish_verify", "path": full}));
        assert_eq!(corrupt["code"], PACKAGE_CORRUPT, "{corrupt}");
        assert_eq!(corrupt["mismatched"], json!(["bom.csv"]), "{corrupt}");
        assert_eq!(corrupt["package_id"], "PID-1101-r002", "{corrupt}");

        let mut unsaved = OpenCADStudio::new_for_test();
        let _ = unsaved.automation_op(r#"{"op":"new"}"#);
        let listed = request(&mut unsaved, json!({"op": "pid_publications"}));
        assert_eq!(listed["code"], PUBLISH_UNSAVED, "{listed}");
    }

    /// 契约规模：2,000 个对象的合成图（`IX-4`）。
    const SYNTHETIC_OBJECTS: usize = 2000;
    /// 采样次数：取中位数。
    const RUNS: usize = 3;

    /// 验收第 3 条：2,000 个对象的图，门禁部分（不含扫项目）不超过 1 秒。量的是 [`publish_gates`]：
    /// 现做检查（内置规则、没配主数据）、数重复位号、算门禁；做摘要、建索引是扫描的事，另列。
    #[test]
    #[ignore]
    fn publish_gates_2000_objects_within_one_second() {
        const BUDGET: Duration = Duration::from_secs(1);
        const DRAWING: &str = "PID-2000";
        let doc = synthetic_drawing(SYNTHETIC_OBJECTS);
        let library = SymbolLibrary::builtin();
        let rules = builtin_rules::builtin();
        let ms = |duration: Duration| duration.as_secs_f64() * 1e3;
        let mut samples = Vec::with_capacity(RUNS);
        for run in 1..=RUNS {
            let started = Instant::now();
            let meta = SummaryMeta {
                drawing_id: DRAWING.to_owned(),
                file: Some("PID-2000.dwg".to_owned()),
                pbs: None,
                source: SummarySource::Open,
                fingerprint: None,
            };
            let summary = summarize(&doc, &library, meta, rules.connect_tolerance);
            let objects = summary.objects.len();
            let index = build_index(Path::new("synthetic"), std::slice::from_ref(&summary), &[])
                .expect("建得出索引");
            let scanned = Scanned {
                index,
                summaries: vec![summary],
                waivers: vec![(DRAWING.to_owned(), Vec::new())],
            };
            let prepared = started.elapsed();
            let gated = publish_gates(&scanned, DRAWING, rules.clone(), &Resolved::NotConfigured)
                .expect("算得出门禁");
            let elapsed = started.elapsed() - prepared;
            println!(
                "PUB-3 gates run {run}/{RUNS}: {:.2} ms (summary + index {:.2} ms before it), \
                 objects {objects}, errors {}, warnings {}, hints {}, duplicates {}",
                ms(elapsed),
                ms(prepared),
                gated.gates.drc_errors,
                gated.gates.drc_warnings,
                gated.gates.drc_hints,
                gated.gates.duplicate_tags
            );
            assert_eq!(objects, SYNTHETIC_OBJECTS, "run {run}");
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[samples.len() / 2];
        println!(
            "PUB-3 gates median {:.2} ms, budget {} ms",
            ms(median),
            BUDGET.as_millis()
        );
        assert!(
            median <= BUDGET,
            "PUB-3 门禁超预算：中位数 {:.2} ms",
            ms(median)
        );
    }
}

#[cfg(test)]
#[path = "pidpublish_acceptance.rs"]
mod acceptance;

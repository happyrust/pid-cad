//! PIDPUBLISH：把当前图发布成带修订号的包（`SPEC-pid-publish.md` §4、§5.2）。
//!
//! ```text
//! PIDPUBLISH [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]
//!                        发布当前图（出包还没接上：过完门禁就停，不写任何东西）
//! PIDPUBLISH CHECK [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]
//!                        只看能不能发布：前提和四项门禁，不写任何东西
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

use super::piddrc::{self, Check};
use super::pidindex::{point, same_file, Scanned, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::pid::drc::{CheckReport, CheckResult, RuleSet, Severity};
use crate::pid::master_data::{self, Resolved};
use crate::pid::project_index::{self, Duplicate};
use crate::pid::publish_pkg::{
    self, CollabInput, CollabVerdict, Gates, MasterDataState, PUBLISH_BLOCKED,
    PUBLISH_TARGET_EXISTS,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 图没保存过，或者有没保存的修改（§3.4，HTTP 409）。
pub(in crate::app) const PUBLISH_UNSAVED: &str = "PUBLISH_UNSAVED";
/// 磁盘上的文件不是本程序上次读写的那一份，或者标签页没记到它的指纹（§3.4，HTTP 409）。
pub(in crate::app) const PUBLISH_FILE_CHANGED: &str = "PUBLISH_FILE_CHANGED";
/// 目标目录建不了、写不了（§3.6，HTTP 400）。
pub(in crate::app) const PUBLISH_TARGET_INVALID: &str = "PUBLISH_TARGET_INVALID";
/// 指定的修订号不是正整数，或者不大于已有的最大修订号（§3.6，HTTP 400）。
pub(in crate::app) const PUBLISH_REVISION_INVALID: &str = "PUBLISH_REVISION_INVALID";

/// 不写目标目录时用的：图纸所在目录下的这个子目录（§3.1）。
const DEFAULT_TARGET: &str = "pid-publish";
/// 命令行上列出的没豁免的错误条数。
const LISTED: usize = 10;
/// 写错时说的写法。
const USAGE: &str =
    "写成 PIDPUBLISH [CHECK] [<目标目录>] [REV <修订号>] [ALLOW_OFFLINE]，目录有空格就加双引号。";

/// 命令行上写的：目标目录和修订号都是原样（合不合法在前提里查），带没带 `ALLOW_OFFLINE`。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Asked {
    dir: Option<String>,
    revision: Option<String>,
    allow_offline: bool,
}

/// 前提都满足时，要发到哪里：项目目录、图号、目标目录、这一版的修订号，和协同前提的结论。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    project_dir: PathBuf,
    drawing_id: String,
    dir: PathBuf,
    revision: u32,
    collab: CollabVerdict,
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

impl OpenCADStudio {
    pub(super) fn dispatch_pidpublish(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (word, rest) = first_word(cmd);
        if !word.eq_ignore_ascii_case("PIDPUBLISH") {
            return None;
        }
        let (option, after) = first_word(rest);
        if option.eq_ignore_ascii_case("LIST") || option.eq_ignore_ascii_case("VERIFY") {
            let option = option.to_ascii_uppercase();
            self.command_line
                .push_error(&format!("PIDPUBLISH {option}: 还没有这一项。"));
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

    /// 扫完项目接着做（§4）：前提再过一遍，在扫好的项目上算门禁（[`publish_gates`]），说出来。
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
        self.pidpublish_verdict(verb, job.only_check, &gated.gates);
    }

    /// 发布的前提（§3.4，离线规格 §5.7），按先后查第 `i` 个标签页的图：先不满足的那条是错误码
    /// 和为什么；都满足时是要发到哪里。只读会话不在这里查（命令闸门拒发布，`CHECK` 照查）。
    /// 不写任何东西。
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
        let Some(project_dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) else {
            return Err((INDEX_NO_PROJECT_DIR, "当前图不属于哪个项目目录".to_owned()));
        };
        let drawing_id = listed_id(project_dir, path)?;
        let dir = asked.dir.as_deref().map_or_else(
            || project_dir.join(DEFAULT_TARGET),
            |dir| project_dir.join(dir),
        );
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
        Ok(Target {
            project_dir: project_dir.to_path_buf(),
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
    /// 错误码；都过了，`CHECK` 说可以发布，发布本身因为出包还没接上，说没有发布。
    fn pidpublish_verdict(&mut self, verb: &str, only_check: bool, gates: &Gates) {
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
        } else if let Some(code) = readonly {
            self.command_line.push_error_coded(
                &code,
                &format!("{verb}: {code}: 前提和门禁都过了，但这个会话只读，不能发布。"),
            );
        } else if only_check {
            self.command_line
                .push_output(&format!("{verb}: 前提和门禁都过了，可以发布。"));
        } else {
            self.command_line.push_error(&format!(
                "{verb}: 前提和门禁都过了，但出包还没接上，没有发布。"
            ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::collab_state::{self, Diverged};
    use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
    use crate::pid::project_index::{
        build_index, summarize, synthetic_drawing, SummaryMeta, SummarySource,
    };
    use crate::pid::publish_pkg::{COLLAB_DIVERGED, COLLAB_OFFLINE, COLLAB_UNSYNCED};
    use crate::pid::symbols::SymbolLibrary;
    use crate::pid::{FileCollabState, QueuedOp};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
            "PIDPUBLISH LIST",
            "pidpublish verify out/PID-1101-r001",
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

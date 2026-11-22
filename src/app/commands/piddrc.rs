//! PIDCHECK：按规则检查 P&ID 图纸（`SPEC-pid-drc.md` §4、§5.2）。
//!
//! ```text
//! PIDCHECK                 查当前图：一句汇总，再列前 10 条结果
//! PIDCHECK PROJECT         查当前图所在项目目录里的全部图，项目级规则也跑
//! PIDCHECK PBS <节点号>    查项目文件里归在这个 PBS 节点及其下级的图，项目级规则也跑
//! PIDCHECK RULES           列出生效的规则：号、名称、严重度、启停、来源
//! PIDCHECK EXPORT <文件>   把最近一次的检查结果写成 JSON 文件（§3.6）
//! PIDCHECK ENABLE <规则号> 在项目包里启用一条规则，没有项目包就新建一个
//! PIDCHECK DISABLE <规则号>
//!                          在项目包里停用一条规则，没有项目包就新建一个
//! PIDCHECK WAIVE <规则号> <对象键> <理由>
//!                          豁免当前图最近一次检查结果里的一条，理由取到行尾
//! PIDCHECK UNWAIVE <规则号> <对象键>
//!                          去掉当前图上的一条豁免
//! ```
//!
//! 一次检查照 §4.1 走：取规则——内置包，当前图所在目录有项目包就叠上；项目包有错就连同错误码
//! 报出来、不查，不退回去只用内置包。再取主数据、做摘要（连接容差取规则包的），范围要项目
//! 索引就扫项目，然后检查。查当前图用保存门禁的同一个函数（`pid::drc::check_drawing`），项目级
//! 规则不跑。查项目、查节点借 `PIDINDEX` 的扫描（`commands::pidindex`）：要从磁盘读的图在后台读，
//! 读完接着查，`PIDINDEX CANCEL` 可以取消；磁盘上那些图里记着的豁免读图时一起读出来。当前图
//! 没保存过就没有项目目录（`INDEX_NO_PROJECT_DIR`）；节点号要在主数据的 PBS 表里，不在、或者
//! 主数据没配好，是 `PBS_NODE_UNKNOWN`。
//!
//! 结果只在内存里留最近一份（§4.1），不写进图：查当前图的放在那个标签页上
//! （`DocumentTab::pid_check`），查项目、查节点的按项目目录放在程序里（[`PidChecks`]）。
//! `EXPORT` 写当前标签页上的那份和它项目目录的那份里较新的一份。
//!
//! `ENABLE`、`DISABLE` 改写图所在目录的项目包（§3.2）：只动这条规则的 `enabled`，别的原样留着；
//! 改好的先照读规则包的规矩校验，再写临时文件、改名盖过去，下一次检查起生效。`WAIVE`、`UNWAIVE`
//! 改图里记的豁免（§3.7），各是一个撤销步；只能豁免当前图最近一次检查结果里落在这张图上的条目。
//! 豁免改了以后——撤销、重做也算——留着的、查过这张图的结果就地改
//! （[`OpenCADStudio::pidcheck_follow_waivers`]），不重新检查。
//!
//! 检查、看规则、导出不改图，只读会话里都能用；四个写的子命令只读会话里不让用，由
//! `collab::apply` 的 `readonly_refuses_command` 按子命令词拒绝。
//!
//! 自动化操作（§5.3）是这些子命令的接口，走同样的逻辑：`pid_check` 按 `scope` 查，应答就是检查
//! 结果；查项目、查节点、跨图（`cross_drawing`）借 `PIDINDEX` 操作的扫描，要从磁盘读图时先应答
//! `scanning`，调用方稍后再发一次。`pid_check_rules` 列生效的规则，`pid_check_rules_set` 整个换掉
//! 项目包，`pid_check_waive`、`pid_check_unwaive` 同 `WAIVE`、`UNWAIVE`。操作不经过命令行，三个写的
//! 操作自己按对应的子命令词查只读会话。

use super::pidindex::{index_refusal, same_file, Scanned, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::io::linetypes::writer::write_atomic;
use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
use crate::pid::drc::{
    self, CheckInput, CheckReport, CheckResult, RuleSet, Scope, ScopeKind, Severity, Summary,
    Trigger, Waiver,
};
use crate::pid::master_data::{self, Resolved, PROJECT_FILE};
use crate::pid::project_index::{self, DrawingSummary};
use crate::pid::type_lib::TypeLib;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 按节点检查时节点号不在主数据的 PBS 表里（§3.8）。
pub(in crate::app) const PBS_NODE_UNKNOWN: &str = "PBS_NODE_UNKNOWN";

/// 启停、豁免时指的规则号不在生效的规则里（§3.8）。
pub(in crate::app) const RULE_NOT_FOUND: &str = "RULE_NOT_FOUND";

/// 要豁免的（规则号, 对象键）不在当前图最近一次检查的结果里，或者要取消的豁免图里没有
/// （§3.8）。
pub(in crate::app) const WAIVER_TARGET_NOT_FOUND: &str = "WAIVER_TARGET_NOT_FOUND";

/// 换项目包时写不了文件（`pid_check_rules_set`；`SPEC-pid-drc.md` §11.14 补的码）。
const RULESET_IO_ERROR: &str = "RULESET_IO_ERROR";

/// 命令行上列出的结果条数（§5.2）。
const LISTED: usize = 10;

/// 本期没有对象、不会出结果的内置规则（决定 D8），`PIDCHECK RULES` 里标出来。
const NO_OBJECTS_YET: [&str; 3] = ["DR-016", "DR-020", "DR-109"];

/// 一次检查的结果，和它是程序里第几次检查：两份结果比新旧用它。
pub(in crate::app) struct Checked {
    pub(in crate::app) serial: u64,
    pub(in crate::app) report: CheckReport,
    /// 查了哪些图（图号）。
    drawings: Vec<String>,
    /// `stale_waivers` 里的每条是哪张图上的（`None` 是认不出来的）：图上的豁免改了以后，
    /// `stale_waivers` 照它重算（[`Checked::follow`]）。
    stale: Vec<(Option<String>, Waiver)>,
    /// 检查时打开着的图各自的改动计数（标签页编号，`edit_revision`）：校验与交付面板拿它对照现在的，
    /// 看结果过期没有（`SPEC-pid-verify-deliver-panel.md` §3.1）。
    revisions: Vec<(u64, u64)>,
}

impl Checked {
    /// 编号 `tab` 的标签页上的图检查时的改动计数；检查之后才打开的图从打开时的 0 算起。
    pub(in crate::app) fn revision_at_check(&self, tab: u64) -> u64 {
        self.revisions
            .iter()
            .find(|(id, _)| *id == tab)
            .map_or(0, |&(_, revision)| revision)
    }

    /// 豁免把编号 `tab` 的标签页上的图从改动计数 `before` 改到了 `after`，这份结果已就地跟上
    /// （§3.7）：原先跟得上这张图的，记的计数跟到 `after`，面板不把它算过期（面板规格 §4.4）。
    /// 原先就跟不上的照旧过期。
    fn restamp(&mut self, tab: u64, before: u64, after: u64) {
        if self.revision_at_check(tab) != before {
            return;
        }
        match self.revisions.iter_mut().find(|(id, _)| *id == tab) {
            Some((_, revision)) => *revision = after,
            None => self.revisions.push((tab, after)),
        }
    }
    /// 图 `drawing` 上记的豁免改成 `now` 以后就地改这份结果（§3.7），不重新检查：这张图上的结果
    /// 按（规则号, 对象键）重挂豁免，各严重度和已豁免的条数、`blocked` 跟着重算；`stale_waivers`
    /// 里这张图的只留 `now` 里还有的。没查过这张图的结果不动。
    fn follow(&mut self, drawing: &str, now: &[Waiver]) {
        if !self.drawings.iter().any(|id| id == drawing) {
            return;
        }
        let report = &mut self.report;
        for result in report
            .results
            .iter_mut()
            .filter(|result| result.object.drawing_id == drawing)
        {
            result.waiver = now
                .iter()
                .find(|waiver| {
                    waiver.rule_id == result.rule_id && waiver.object_key == result.object_key
                })
                .cloned();
            result.waived = result.waiver.is_some();
        }
        let mut summary = Summary {
            errors: 0,
            warnings: 0,
            infos: 0,
            waived: 0,
            ..report.summary
        };
        for result in &report.results {
            let count = match (result.waived, result.severity) {
                (true, _) => &mut summary.waived,
                (false, Severity::Error) => &mut summary.errors,
                (false, Severity::Warning) => &mut summary.warnings,
                (false, Severity::Info) => &mut summary.infos,
            };
            *count += 1;
        }
        report.summary = summary;
        report.blocked = summary.errors > 0;
        // 一模一样的两条豁免各算各的：`now` 里还有几条，就留几条。
        let mut left: Vec<&Waiver> = now.iter().collect();
        report.stale_waivers = self
            .stale
            .iter()
            .filter(|(on, waiver)| {
                on.as_deref() != Some(drawing)
                    || left
                        .iter()
                        .position(|still| *still == waiver)
                        .map(|at| left.remove(at))
                        .is_some()
            })
            .map(|(_, waiver)| waiver.clone())
            .collect();
    }
}

/// 查项目、查 PBS 节点的结果，按项目目录各留最近一份（§4.1）；查当前图的在标签页上。检查的
/// 序号也从这里发，两处的结果才比得出新旧。
#[derive(Default)]
pub(in crate::app) struct PidChecks {
    serial: u64,
    projects: HashMap<PathBuf, Checked>,
}

impl PidChecks {
    /// 给新的一份结果编上序号。`drawings` 是查了的图，`recorded` 是交给检查器的豁免（图号和图里
    /// 记的），`revisions` 是这时打开着的图各自的改动计数（[`OpenCADStudio::pidcheck_revisions`]）。
    fn checked(
        &mut self,
        report: CheckReport,
        drawings: Vec<String>,
        recorded: &[(String, Vec<Waiver>)],
        revisions: Vec<(u64, u64)>,
    ) -> Checked {
        self.serial += 1;
        Checked {
            serial: self.serial,
            stale: stale_by_drawing(&report, &drawings, recorded),
            drawings,
            report,
            revisions,
        }
    }

    /// 项目目录 `dir` 留着的查项目、查节点的结果。
    pub(in crate::app) fn of_project(&self, dir: &Path) -> Option<&Checked> {
        self.projects.get(&same_file(dir))
    }

    /// 程序里最近一次检查的序号，还没查过是 0：变了就是又查了一次，不管留在哪里。
    pub(in crate::app) fn last_serial(&self) -> u64 {
        self.serial
    }
}

/// 检查时 `stale_waivers` 里的每条是哪张图上的。检查器按 `recorded` 里图的先后、图里记的先后，
/// 列出查了的图上对不上结果的豁免（`SPEC-pid-drc.md` §11.9 第 6 条）；照同样的先后认回去，
/// 认不出来的记 `None`。
fn stale_by_drawing(
    report: &CheckReport,
    drawings: &[String],
    recorded: &[(String, Vec<Waiver>)],
) -> Vec<(Option<String>, Waiver)> {
    let mut stale = report.stale_waivers.iter().peekable();
    let mut found = Vec::new();
    for (drawing, waivers) in recorded
        .iter()
        .filter(|(drawing, _)| drawings.contains(drawing))
    {
        for waiver in waivers {
            let answered = report.results.iter().any(|result| {
                result.object.drawing_id == *drawing
                    && result.rule_id == waiver.rule_id
                    && result.object_key == waiver.object_key
            });
            if !answered && stale.peek() == Some(&waiver) {
                found.push((Some(drawing.clone()), waiver.clone()));
                stale.next();
            }
        }
    }
    found.extend(stale.map(|waiver| (None, waiver.clone())));
    found
}

/// 扫好项目要做的检查（`commands::pidindex` 的扫描排着的一件，或者操作扫好了当场查）：查哪些图，
/// 用的规则和主数据，按哪个触发时机查。规则和主数据在起头时取（§4.1 的先后），摘要的连接容差照
/// 这套规则。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Check {
    range: Range,
    pub(super) rules: RuleSet,
    master: Resolved,
    /// `PIDCHECK`、`pid_check` 是「按需」，发布前现做的那次是「发布前」。
    trigger: Trigger,
}

impl Check {
    /// 发布前现做的检查（`SPEC-pid-publish.md` §4）：只报要发布的那张图（图号 `drawing_id`）上的，
    /// 跨图查，触发时机「发布前」。
    pub(super) fn before_publish(drawing_id: String, rules: RuleSet, master: Resolved) -> Self {
        Self {
            range: Range::Drawing(drawing_id),
            rules,
            master,
            trigger: Trigger::BeforePublish,
        }
    }
}

/// 扫项目的检查报哪些图上的结果（§4.2），项目级规则都跑。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Range {
    /// 全项目。
    Project,
    /// 这个 PBS 节点及其下级。
    Pbs(String),
    /// 只报这张图（图号）上的：`pid_check` 的 `cross_drawing`。
    Drawing(String),
}

/// `PIDCHECK EXPORT` 写的、`pid_check` 应答的：§3.6 的检查结果，带上应答外面那层的 `ok`。
#[derive(Serialize)]
struct Exported<'a> {
    ok: bool,
    #[serde(flatten)]
    report: &'a CheckReport,
}

impl OpenCADStudio {
    pub(super) fn dispatch_piddrc(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let (verb, rest) = first_word(cmd);
        if !verb.eq_ignore_ascii_case("PIDCHECK") {
            return None;
        }
        let (option, rest) = first_word(rest);
        let option = option.to_ascii_uppercase();
        match option.as_str() {
            "" => self.pidcheck_drawing(i),
            "PROJECT" | "RULES" if !rest.is_empty() => {
                self.command_line
                    .push_error(&format!("PIDCHECK {option}: 后面不用再写东西，多了 {rest}。"));
            }
            "PROJECT" => return Some(self.pidcheck_start(i, None)),
            "PBS" => match first_word(rest) {
                (node, "") if !node.is_empty() => {
                    return Some(self.pidcheck_start(i, Some(node.to_owned())));
                }
                _ => self
                    .command_line
                    .push_error("PIDCHECK PBS: 写成 PIDCHECK PBS <节点号>。"),
            },
            "RULES" => self.pidcheck_rules(i),
            "EXPORT" => self.pidcheck_export(i, rest),
            "ENABLE" | "DISABLE" => match first_word(rest) {
                (rule, "") if !rule.is_empty() => {
                    self.pidcheck_switch(i, &rule.to_ascii_uppercase(), option == "ENABLE");
                }
                _ => self
                    .command_line
                    .push_error(&format!("PIDCHECK {option}: 写成 PIDCHECK {option} <规则号>。")),
            },
            "WAIVE" => match waive_words(rest) {
                Some((rule, key, reason)) => self.pidcheck_waive(i, &rule, key, reason),
                None => self.command_line.push_error(
                    "PIDCHECK WAIVE: 写成 PIDCHECK WAIVE <规则号> <对象键> <理由>；对象键里有空格就加双引号。",
                ),
            },
            "UNWAIVE" => match waive_words(rest) {
                Some((rule, key, "")) => self.pidcheck_unwaive(i, &rule, key),
                _ => self.command_line.push_error(
                    "PIDCHECK UNWAIVE: 写成 PIDCHECK UNWAIVE <规则号> <对象键>；对象键里有空格就加双引号。",
                ),
            },
            _ => self.command_line.push_error(&format!(
                "PIDCHECK: 不认识 {option}；写 PROJECT、PBS <节点号>、RULES、EXPORT <文件>、ENABLE <规则号>、DISABLE <规则号>、WAIVE <规则号> <对象键> <理由> 或 UNWAIVE <规则号> <对象键>，或者只写 PIDCHECK。"
            )),
        }
        Some(Task::none())
    }

    /// `PIDCHECK`：查第 `i` 个标签页的图（§4.2 的 `drawing`），项目级规则不跑。说一句汇总、列前
    /// 10 条，结果记在这个标签页上。
    fn pidcheck_drawing(&mut self, i: usize) {
        let Some(rules) = self.pidcheck_rule_set(i, "PIDCHECK") else {
            return;
        };
        let checked = self.pidcheck_this_drawing(i, &rules);
        self.pidcheck_say("PIDCHECK", &checked.report, None);
        self.tabs[i].pid_check = Some(checked);
    }

    /// 只查第 `i` 个标签页的图，用保存门禁的同一个函数（[`drc::check_drawing`]），项目级规则不跑。
    /// 不碰命令行，结果由调用的记下。
    pub(super) fn pidcheck_this_drawing(&mut self, i: usize, rules: &RuleSet) -> Checked {
        let revisions = self.pidcheck_revisions();
        let document = &self.tabs[i].scene.document;
        let path = document.source_path.as_deref().map(Path::new);
        let report = drc::check_drawing(document, path, rules, Trigger::OnDemand);
        let drawing = report.scope.drawing_id.clone().unwrap_or_default();
        let recorded = [(drawing.clone(), drc::waivers(document))];
        self.pid_checks
            .checked(report, vec![drawing], &recorded, revisions)
    }

    /// 这时打开着的图各自的改动计数（标签页编号，`edit_revision`），跟着检查结果记下。
    fn pidcheck_revisions(&self) -> Vec<(u64, u64)> {
        self.tabs
            .iter()
            .filter(|tab| !tab.is_start)
            .map(|tab| (tab.id, tab.edit_revision))
            .collect()
    }

    /// 生效的规则（[`Self::pidcheck_load_rules`]）。读不出来就连同错误码说出来，得 `None`。
    fn pidcheck_rule_set(&mut self, i: usize, verb: &str) -> Option<RuleSet> {
        match self.pidcheck_load_rules(i) {
            Ok(rules) => Some(rules),
            Err((code, why)) => {
                self.command_line
                    .push_error_coded(code, &format!("{verb}: {why}"));
                None
            }
        }
    }

    /// 生效的规则：内置包，第 `i` 个标签页的图所在目录有项目包就叠上（§3.2）。读不出来时是错误码
    /// 和「哪个包有错：错在哪」，不退回去只用内置包（§4.1）。
    pub(super) fn pidcheck_load_rules(&self, i: usize) -> Result<RuleSet, (&'static str, String)> {
        let dir = crate::io::drawing_dir(&self.tabs[i].scene.document);
        builtin_rules::load(dir).map_err(|error| {
            let pack = match dir {
                Some(dir) => format!("项目包 {}", dir.join(PROJECT_PACK_FILE).display()),
                None => "内置规则包".to_owned(),
            };
            (error.code(), format!("{pack} 有错：{error}"))
        })
    }

    /// `PIDCHECK PROJECT`、`PIDCHECK PBS <节点号>`（§4.2）：取规则和主数据，查节点号在不在，再扫
    /// 第 `i` 个标签页的图所在的项目目录，扫完接着查（[`Self::pidcheck_scanned`]）。要从磁盘读的图
    /// 在后台读，这里先交差。
    fn pidcheck_start(&mut self, i: usize, pbs: Option<String>) -> Task<Message> {
        let verb = if pbs.is_some() {
            "PIDCHECK PBS"
        } else {
            "PIDCHECK PROJECT"
        };
        match self.pidcheck_prepare(i, pbs) {
            Ok(check) => self.pidindex_check(i, check),
            Err((code, why)) => {
                self.command_line
                    .push_error_coded(code, &format!("{verb}: {why}"));
                Task::none()
            }
        }
    }

    /// 查项目、查节点在扫项目之前要备齐的（§4.1）：第 `i` 个标签页的图得有项目目录，取生效的
    /// 规则和主数据，`pbs` 是节点号时它得在主数据里。不碰命令行：不成时是错误码和为什么，
    /// 命令和校验与交付面板各自说。
    pub(super) fn pidcheck_prepare(
        &self,
        i: usize,
        pbs: Option<String>,
    ) -> Result<Check, (&'static str, String)> {
        let range = pbs.map_or(Range::Project, Range::Pbs);
        let Some(project_dir) = crate::io::drawing_dir(&self.tabs[i].scene.document) else {
            return Err((
                INDEX_NO_PROJECT_DIR,
                format!("{INDEX_NO_PROJECT_DIR}: 当前图没保存过，不属于哪个项目；存盘后再查。"),
            ));
        };
        let rules = self.pidcheck_load_rules(i)?;
        let master = master_data::resolve(&self.tabs[i].scene.document, Some(project_dir));
        if let Range::Pbs(node) = &range {
            pbs_known(&master, node)
                .map_err(|why| (PBS_NODE_UNKNOWN, format!("{PBS_NODE_UNKNOWN}: {why}。")))?;
        }
        Ok(Check {
            range,
            rules,
            master,
            trigger: Trigger::OnDemand,
        })
    }

    /// 扫完项目接着查（[`check_project`]），说一句汇总、列前 10 条，结果按项目目录记下。
    pub(super) fn pidcheck_scanned(&mut self, project_dir: &Path, scanned: &Scanned, check: Check) {
        let verb = verb_of(&check.range);
        let (report, drawings) = check_project(scanned, check);
        self.pidcheck_say(verb, &report, Some(drawings.len()));
        let revisions = self.pidcheck_revisions();
        let checked = self
            .pid_checks
            .checked(report, drawings, &scanned.waivers, revisions);
        self.pid_checks
            .projects
            .insert(same_file(project_dir), checked);
    }

    /// 说一次检查的结果（§5.2）：一句汇总，再按结果的先后列前 10 条没豁免的，提示逐条说。
    /// 查项目、查节点时 `drawings` 是查了几张图，每条前面带图号。
    fn pidcheck_say(&mut self, verb: &str, report: &CheckReport, drawings: Option<usize>) {
        let counts = &report.summary;
        let sheets = drawings.map_or_else(String::new, |count| format!("{count} 张图，"));
        let waived = if counts.waived > 0 {
            format!("，已豁免 {} 条", counts.waived)
        } else {
            String::new()
        };
        let skipped = if counts.rules_skipped > 0 {
            format!("，{} 条没跑", counts.rules_skipped)
        } else {
            String::new()
        };
        self.command_line.push_output(&format!(
            "{verb}: {sheets}错误 {} 条，警告 {} 条，提示 {} 条{waived}；查了 {} 个对象，跑了 {} 条规则{skipped}。",
            counts.errors, counts.warnings, counts.infos, counts.objects, counts.rules_run
        ));
        let open: Vec<&CheckResult> = report
            .results
            .iter()
            .filter(|result| !result.waived)
            .collect();
        for result in open.iter().take(LISTED) {
            self.command_line
                .push_output(&format!("  {}", listed(result, drawings.is_some())));
        }
        if open.len() > LISTED {
            self.command_line.push_output(&format!(
                "  还有 {} 条没列出；PIDCHECK EXPORT <文件> 可以导出全部。",
                open.len() - LISTED
            ));
        }
        for notice in &report.notices {
            self.command_line
                .push_info(&format!("{verb}: {}: {}", notice.code, notice.message));
        }
        if !report.stale_waivers.is_empty() {
            self.command_line.push_info(&format!(
                "{verb}: 图里有 {} 条豁免这次没对上结果。",
                report.stale_waivers.len()
            ));
        }
    }

    /// `PIDCHECK RULES`：生效的规则（§3.2），一条一行：号、名称、严重度、启停、来源；本期没有
    /// 对象的标出来（决定 D8）。项目包有错就把错报出来（§5.2）。
    fn pidcheck_rules(&mut self, i: usize) {
        let Some(rules) = self.pidcheck_rule_set(i, "PIDCHECK RULES") else {
            return;
        };
        let enabled = rules
            .rules
            .iter()
            .filter(|rule| rules.is_enabled(rule))
            .count();
        let pack = rules.project_pack.as_ref().map_or_else(
            || "只用内置包".to_owned(),
            |path| format!("内置包叠项目包 {}", path.display()),
        );
        self.command_line.push_output(&format!(
            "PIDCHECK RULES: 规则版本 {}，{pack}；{} 条规则，启用 {enabled} 条。",
            rules.version,
            rules.rules.len()
        ));
        for rule in &rules.rules {
            let state = if !rule.enabled {
                "停用".to_owned()
            } else if rules.is_enabled(rule) {
                "启用".to_owned()
            } else {
                format!("停用（规则集 {} 停用）", rule.set)
            };
            let silent = if NO_OBJECTS_YET.contains(&rule.id.as_str()) {
                "，本期没有对象"
            } else {
                ""
            };
            self.command_line.push_output(&format!(
                "  {} {}：{}，{state}，{}{silent}",
                rule.id,
                rule.name,
                rule.severity.name(),
                rule.source.name()
            ));
        }
    }

    /// `PIDCHECK EXPORT <文件>`：把最近一次的检查结果写成文件（[`Self::pidcheck_write_export`]），
    /// 说一句写到了哪里。
    fn pidcheck_export(&mut self, i: usize, rest: &str) {
        let name = unquoted(rest);
        if name.is_empty() {
            self.command_line
                .push_error("PIDCHECK EXPORT: 写成 PIDCHECK EXPORT <文件>。");
            return;
        }
        match self.pidcheck_write_export(i, name) {
            Ok(said) => self
                .command_line
                .push_output(&format!("PIDCHECK EXPORT: {said}")),
            Err(why) => self
                .command_line
                .push_error(&format!("PIDCHECK EXPORT: {why}")),
        }
    }

    /// 把第 `i` 个标签页最近一次的检查结果（[`Self::pidcheck_latest`]）写到文件 `name`：§3.6 的
    /// JSON，带 `ok`。文件名不是完整路径时从图纸所在目录算起，没存过的图从当前目录算起。只写
    /// `.json`，不写项目文件；已有的覆盖。不碰命令行：成了是写了什么、写到哪里，不成是为什么，
    /// 命令和校验与交付面板各自说。
    pub(super) fn pidcheck_write_export(&self, i: usize, name: &str) -> Result<String, String> {
        let path = match crate::io::drawing_dir(&self.tabs[i].scene.document) {
            Some(dir) if Path::new(name).is_relative() => dir.join(name),
            _ => PathBuf::from(name),
        };
        let json = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        if !json {
            return Err(format!("只写 .json 文件，{} 不是。", path.display()));
        }
        if path
            .file_name()
            .is_some_and(|file| file.eq_ignore_ascii_case(PROJECT_FILE))
        {
            return Err(format!(
                "{} 是项目文件，不拿检查结果盖掉它。",
                path.display()
            ));
        }
        let Some(checked) = self.pidcheck_latest(i) else {
            return Err(
                "还没有检查结果；先用 PIDCHECK、PIDCHECK PROJECT 或 PIDCHECK PBS <节点号> 查一次。"
                    .to_owned(),
            );
        };
        let report = &checked.report;
        let counts = report.summary;
        let scope = scope_said(&report.scope);
        serde_json::to_string_pretty(&Exported { ok: true, report })
            .map_err(|error| error.to_string())
            .and_then(|text| std::fs::write(&path, text + "\n").map_err(|error| error.to_string()))
            .map_err(|error| format!("写不了 {}：{error}", path.display()))?;
        Ok(format!(
            "已把{scope}的检查结果（错误 {} 条，警告 {} 条，提示 {} 条）写到 {}。",
            counts.errors,
            counts.warnings,
            counts.infos,
            path.display()
        ))
    }

    /// `PIDCHECK ENABLE <规则号>`、`PIDCHECK DISABLE <规则号>`（§3.2、§5.2）：在第 `i` 个标签页的图
    /// 所在目录的项目包里启停一条规则（[`switched_pack`]）。改好的先照读规则包的规矩校验，再写临时
    /// 文件、改名盖过去；本来就是那样的不写。下一次检查起生效，留着的结果不改。
    fn pidcheck_switch(&mut self, i: usize, rule_id: &str, enable: bool) {
        let (verb, state) = if enable {
            ("PIDCHECK ENABLE", "启用")
        } else {
            ("PIDCHECK DISABLE", "停用")
        };
        let Some(dir) = crate::io::drawing_dir(&self.tabs[i].scene.document).map(Path::to_path_buf)
        else {
            self.command_line.push_error_coded(
                INDEX_NO_PROJECT_DIR,
                &format!(
                    "{verb}: {INDEX_NO_PROJECT_DIR}: 当前图没保存过，没有放项目包的目录；存盘后再改。"
                ),
            );
            return;
        };
        let Some(rules) = self.pidcheck_rule_set(i, verb) else {
            return;
        };
        let Some(rule) = rules.rule(rule_id) else {
            self.command_line.push_error_coded(
                RULE_NOT_FOUND,
                &format!(
                    "{verb}: {RULE_NOT_FOUND}: 生效的规则里没有 {rule_id}；PIDCHECK RULES 可以列出全部。"
                ),
            );
            return;
        };
        let set_off = enable
            && rules
                .sets
                .iter()
                .any(|set| set.name == rule.set && !set.enabled);
        let set_note = if set_off {
            format!("；它所在的规则集 {} 停用着，检查时仍不跑", rule.set)
        } else {
            String::new()
        };
        if rule.enabled == enable {
            self.command_line.push_info(&format!(
                "{verb}: {rule_id} 本来就{state}着，项目包没改{set_note}。"
            ));
            return;
        }
        let path = dir.join(PROJECT_PACK_FILE);
        let pack = match switched_pack(&path, rule_id, enable) {
            Ok(pack) => pack,
            Err(why) => {
                self.command_line.push_error(&format!("{verb}: {why}"));
                return;
            }
        };
        if let Err(error) = builtin_rules::validate_project_pack(&pack) {
            self.command_line.push_error_coded(
                error.code(),
                &format!("{verb}: 改好的项目包校验不过，没有写：{error}"),
            );
            return;
        }
        if let Err(error) = write_atomic(&path, &format!("{pack:#}\n")) {
            self.command_line
                .push_error(&format!("{verb}: 写不了 {}：{error}", path.display()));
            return;
        }
        self.command_line.push_output(&format!(
            "{verb}: 已{state} {rule_id} {}（项目包 {}）；下一次检查起生效{set_note}。",
            rule.name,
            path.display()
        ));
    }

    /// `PIDCHECK WAIVE <规则号> <对象键> <理由>`（§3.7）：豁免第 `i` 个标签页的图最近一次检查结果
    /// 里的一条（[`Self::pidcheck_add_waiver`]）。
    fn pidcheck_waive(&mut self, i: usize, rule_id: &str, key: &str, reason: &str) {
        const VERB: &str = "PIDCHECK WAIVE";
        let Some(rules) = self.pidcheck_rule_set(i, VERB) else {
            return;
        };
        let replaced = match self.pidcheck_add_waiver(i, &rules, rule_id, key, unquoted(reason)) {
            Ok(replaced) => replaced,
            Err((code, said)) => {
                self.command_line
                    .push_error_coded(code, &format!("{VERB}: {said}"));
                return;
            }
        };
        let counts = self.pidcheck_counts(i, &self.pidcheck_drawing_id(i));
        let done = if replaced {
            format!("{rule_id} 对 {key} 原来就豁免着，理由换成了这次写的")
        } else {
            format!("已豁免 {rule_id} 对 {key} 的结果")
        };
        self.command_line
            .push_output(&format!("{VERB}: {done}；{counts}。"));
    }

    /// 豁免第 `i` 个标签页的图最近一次检查结果里的一条（§3.7），命令和操作共用。规则号要在生效的
    /// 规则 `rules` 里；（规则号, 对象键）要在查过这张图的最近一份结果里、落在这张图上；理由照
    /// [`Waiver::new`] 查。写进图是一个撤销步，这张图上同一（规则号, 对象键）已有豁免就换成这条；
    /// 写完就地改留着的结果，不重新检查。成了得有没有换掉原来的；不成什么也不写，得错误码和命令行
    /// 上命令词后面说的话。
    pub(super) fn pidcheck_add_waiver(
        &mut self,
        i: usize,
        rules: &RuleSet,
        rule_id: &str,
        key: &str,
        reason: &str,
    ) -> Result<bool, (&'static str, String)> {
        if rules.rule(rule_id).is_none() {
            return Err((
                RULE_NOT_FOUND,
                format!("{RULE_NOT_FOUND}: 生效的规则里没有 {rule_id}。"),
            ));
        }
        let drawing = self.pidcheck_drawing_id(i);
        let target = match self.pidcheck_covering(i, &drawing) {
            None => Err("当前图还没有检查结果，先用 PIDCHECK 查一次".to_owned()),
            Some(checked) => checked
                .report
                .results
                .iter()
                .any(|result| {
                    result.object.drawing_id == drawing
                        && result.rule_id == rule_id
                        && result.object_key == key
                })
                .then_some(())
                .ok_or_else(|| {
                    format!(
                        "当前图最近一次检查的结果里，这张图上没有 {rule_id} 对 {key} 的一条；只能豁免里面有的"
                    )
                }),
        };
        if let Err(why) = target {
            return Err((
                WAIVER_TARGET_NOT_FOUND,
                format!("{WAIVER_TARGET_NOT_FOUND}: {why}。"),
            ));
        }
        let by = self.waiver_by();
        let waiver = Waiver::new(rule_id, key, reason, &by, unix_now())
            .map_err(|error| (error.code(), error.to_string()))?;
        let mut replaced = false;
        let mut waivers = Vec::new();
        for old in drc::waivers(&self.tabs[i].scene.document) {
            if old.rule_id != rule_id || old.object_key != key {
                waivers.push(old);
            } else if !replaced {
                waivers.push(waiver.clone());
                replaced = true;
            }
        }
        if !replaced {
            waivers.push(waiver);
        }
        self.pidcheck_write_waivers(i, &waivers);
        Ok(replaced)
    }

    /// `PIDCHECK UNWAIVE <规则号> <对象键>`（§3.7）：去掉第 `i` 个标签页的图上这条豁免
    /// （[`Self::pidcheck_remove_waiver`]）。
    fn pidcheck_unwaive(&mut self, i: usize, rule_id: &str, key: &str) {
        const VERB: &str = "PIDCHECK UNWAIVE";
        if let Err((code, said)) = self.pidcheck_remove_waiver(i, rule_id, key) {
            self.command_line
                .push_error_coded(code, &format!("{VERB}: {said}"));
            return;
        }
        let counts = self.pidcheck_counts(i, &self.pidcheck_drawing_id(i));
        self.command_line.push_output(&format!(
            "{VERB}: 已取消 {rule_id} 对 {key} 的豁免；{counts}。"
        ));
    }

    /// 去掉第 `i` 个标签页的图上的一条豁免（§3.7），命令和操作共用：一个撤销步，写完就地改留着的
    /// 结果。对不上结果的老豁免、规则包里已经没有的规则的豁免也去得掉，所以不查规则号、不要检查
    /// 结果；图里没有这条是 `WAIVER_TARGET_NOT_FOUND`，连同命令行上命令词后面说的话。
    pub(super) fn pidcheck_remove_waiver(
        &mut self,
        i: usize,
        rule_id: &str,
        key: &str,
    ) -> Result<(), (&'static str, String)> {
        let mut waivers = drc::waivers(&self.tabs[i].scene.document);
        let before = waivers.len();
        waivers.retain(|waiver| waiver.rule_id != rule_id || waiver.object_key != key);
        if waivers.len() == before {
            return Err((
                WAIVER_TARGET_NOT_FOUND,
                format!("{WAIVER_TARGET_NOT_FOUND}: 当前图上没有 {rule_id} 对 {key} 的豁免。"),
            ));
        }
        self.pidcheck_write_waivers(i, &waivers);
        Ok(())
    }

    /// 把第 `i` 个标签页的图上的豁免整个换成 `waivers`：先记撤销快照（§3.7），写完就地改留着的结果。
    /// 这一步当场收口：改动计数要到收口时才加一，结果记的计数跟到那以后，原先没过期的就还不算
    /// 过期（[`Checked::restamp`]）。
    fn pidcheck_write_waivers(&mut self, i: usize, waivers: &[Waiver]) {
        let before = self.tabs[i].edit_revision;
        self.push_undo_snapshot(i, "PIDCHECK");
        drc::set_waivers(&mut self.tabs[i].scene.document, waivers);
        self.tabs[i].dirty = true;
        self.finish_pending_history(i);
        self.pidcheck_follow_waivers(i);
        let (tab, after) = (self.tabs[i].id, self.tabs[i].edit_revision);
        let project = crate::io::drawing_dir(&self.tabs[i].scene.document).map(same_file);
        if let Some(checked) = self.tabs[i].pid_check.as_mut() {
            checked.restamp(tab, before, after);
        }
        if let Some(checked) = project.and_then(|dir| self.pid_checks.projects.get_mut(&dir)) {
            checked.restamp(tab, before, after);
        }
    }

    /// 第 `i` 个标签页的图上的豁免改了以后（豁免、取消豁免、撤销、重做），就地改留着的、查过这张图
    /// 的结果（§3.7）：标签页上的，和它项目目录的。不重新检查；没有留着的结果时什么也不做。
    pub(in crate::app) fn pidcheck_follow_waivers(&mut self, i: usize) {
        let project = crate::io::drawing_dir(&self.tabs[i].scene.document).map(same_file);
        let has_project = project
            .as_ref()
            .is_some_and(|dir| self.pid_checks.projects.contains_key(dir));
        if self.tabs[i].pid_check.is_none() && !has_project {
            return;
        }
        let drawing = self.pidcheck_drawing_id(i);
        let now = drc::waivers(&self.tabs[i].scene.document);
        if let Some(checked) = self.tabs[i].pid_check.as_mut() {
            checked.follow(&drawing, &now);
        }
        if let Some(checked) = project.and_then(|dir| self.pid_checks.projects.get_mut(&dir)) {
            checked.follow(&drawing, &now);
        }
    }

    /// 第 `i` 个标签页最近一次的检查结果（`SPEC-pid-verify-deliver-panel.md` §3.1）：标签页上查这张
    /// 图的，和它项目目录查项目、查节点的，两份都有取较新的。
    pub(super) fn pidcheck_latest(&self, i: usize) -> Option<&Checked> {
        self.pidcheck_kept(i).max_by_key(|checked| checked.serial)
    }

    /// 第 `i` 个标签页留着的、查过图 `drawing` 的最近一份结果：只能豁免这一份里的条目（§3.7）。
    fn pidcheck_covering(&self, i: usize, drawing: &str) -> Option<&Checked> {
        self.pidcheck_kept(i)
            .filter(|checked| checked.drawings.iter().any(|id| id == drawing))
            .max_by_key(|checked| checked.serial)
    }

    /// 第 `i` 个标签页留着的结果：标签页上的，和它项目目录的。
    fn pidcheck_kept(&self, i: usize) -> impl Iterator<Item = &Checked> + '_ {
        let tab = self.tabs[i].pid_check.as_ref();
        let project = crate::io::drawing_dir(&self.tabs[i].scene.document)
            .and_then(|dir| self.pid_checks.projects.get(&same_file(dir)));
        tab.into_iter().chain(project)
    }

    /// 第 `i` 个标签页的图在检查结果里的图号（[`drc::drawing_of`]）。
    pub(super) fn pidcheck_drawing_id(&self, i: usize) -> String {
        let path = self.tabs[i]
            .scene
            .document
            .source_path
            .as_deref()
            .map(Path::new);
        drc::drawing_of(path).0
    }

    /// 豁免改了以后说给人听的：查过图 `drawing` 的最近一份结果现在的条数。
    fn pidcheck_counts(&self, i: usize, drawing: &str) -> String {
        let Some(checked) = self.pidcheck_covering(i, drawing) else {
            return "当前图没有留着的检查结果，下一次检查起算上".to_owned();
        };
        let report = &checked.report;
        let counts = report.summary;
        format!(
            "{}的结果现在是错误 {} 条，警告 {} 条，提示 {} 条，已豁免 {} 条（没有重新检查）",
            scope_said(&report.scope),
            counts.errors,
            counts.warnings,
            counts.infos,
            counts.waived
        )
    }

    /// 豁免记在谁名下（§3.7）：连着协同时是协同的用户 ID，否则是操作系统的账户名。快照的生成人
    /// （`commands::pidsnapshot`）也这样取。
    pub(super) fn waiver_by(&self) -> String {
        let user = self.collab.user_id.trim();
        if self.collab.connected && !user.is_empty() {
            return user.to_owned();
        }
        ["USERNAME", "USER", "LOGNAME"]
            .into_iter()
            .find_map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_default()
    }
}

/// 自动化操作（§5.3）：应答和失败的写法同别的控制面操作，不碰命令行。
impl OpenCADStudio {
    /// 控制面的 `pid_check`：照 `scope` 查，应答是检查结果（§3.6）带 `ok`；`include_waived` 为假时
    /// `results` 里不列已豁免的，计数照旧。只查当前图同 `PIDCHECK`，查项目、查节点同
    /// `PIDCHECK PROJECT`、`PIDCHECK PBS`；查当前图带 `cross_drawing` 时扫项目，项目级规则也跑，
    /// 只报这张图上的。要扫项目时，要读的图缓存里没有就应答 `scanning` 并交出后台读的任务
    /// （[`Self::op_scan`]），调用方稍后再发。结果和命令的一样留着：查当前图的（跨图也算）在
    /// 标签页上，查项目、查节点的按项目目录。
    pub(in crate::app) fn control_pid_check(&mut self, req: &Value) -> (Value, Task<Message>) {
        match self.op_check(req) {
            Ok(answer) => (answer, Task::none()),
            Err(answered) => answered,
        }
    }

    fn op_check(&mut self, req: &Value) -> Result<Value, (Value, Task<Message>)> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            let refusal = refuse("no_document", "Create or open a drawing first");
            return Err(no_task(refusal));
        }
        let (asked, include_waived) = check_request(req).map_err(no_task)?;
        let range = match asked {
            Asked::Drawing => {
                let rules = self
                    .pidcheck_load_rules(i)
                    .map_err(|(code, why)| no_task(refuse(code, why)))?;
                let checked = self.pidcheck_this_drawing(i, &rules);
                let answer = answer_of(&checked.report, include_waived);
                self.tabs[i].pid_check = Some(checked);
                return Ok(answer);
            }
            Asked::CrossDrawing => None,
            Asked::Pbs(node) => Some(Range::Pbs(node)),
            Asked::Project => Some(Range::Project),
        };
        let Some(project_dir) =
            crate::io::drawing_dir(&self.tabs[i].scene.document).map(Path::to_path_buf)
        else {
            let refusal = refuse(
                INDEX_NO_PROJECT_DIR,
                "当前图没保存过，不属于哪个项目；存盘后再查",
            );
            return Err(no_task(refusal));
        };
        let range = match range {
            Some(range) => range,
            None => Range::Drawing(self.pidcheck_listed_id(i, &project_dir).map_err(no_task)?),
        };
        let rules = self
            .pidcheck_load_rules(i)
            .map_err(|(code, why)| no_task(refuse(code, why)))?;
        let master = master_data::resolve(&self.tabs[i].scene.document, Some(&project_dir));
        if let Range::Pbs(node) = &range {
            pbs_known(&master, node).map_err(|why| no_task(refuse(PBS_NODE_UNKNOWN, why)))?;
        }
        let (_, scanned) = self.op_scan(i, rules.connect_tolerance, true)?;
        let on_tab = matches!(range, Range::Drawing(_));
        let (report, drawings) = check_project(
            &scanned,
            Check {
                range,
                rules,
                master,
                trigger: Trigger::OnDemand,
            },
        );
        let answer = answer_of(&report, include_waived);
        let revisions = self.pidcheck_revisions();
        let checked = self
            .pid_checks
            .checked(report, drawings, &scanned.waivers, revisions);
        if on_tab {
            self.tabs[i].pid_check = Some(checked);
        } else {
            self.pid_checks
                .projects
                .insert(same_file(&project_dir), checked);
        }
        Ok(answer)
    }

    /// 第 `i` 个标签页的图在项目目录 `project_dir` 的图纸里的图号（跨图检查要它）。按文件认：
    /// 不在里面——项目只认这个目录里的 `.dwg`——是 `INDEX_NO_PROJECT_DIR`，同名的别的文件顶替不了。
    fn pidcheck_listed_id(&self, i: usize, project_dir: &Path) -> Result<String, Value> {
        let drawings =
            project_index::list_drawings(project_dir).map_err(|error| index_refusal(&error))?;
        let current = self.tabs[i]
            .scene
            .document
            .source_path
            .as_deref()
            .map(|path| same_file(Path::new(path)));
        drawings
            .into_iter()
            .find(|drawing| current.as_ref() == Some(&same_file(&drawing.path)))
            .map(|drawing| drawing.drawing_id)
            .ok_or_else(|| {
                refuse(
                    INDEX_NO_PROJECT_DIR,
                    "当前图不在项目的图纸里（项目只认目录里的 .dwg），查不了跨图；不带 cross_drawing 只查这张图",
                )
            })
    }

    /// 控制面的 `pid_check_rules`：生效的规则——版本、摘要值、叠上的项目包（没有是 `null`）、保存
    /// 门禁、规则集和规则（照规则包文件的写法，[`builtin_rules::listing`]）。项目包有错时应答它的
    /// 错误码，同 `PIDCHECK RULES`。
    pub(in crate::app) fn control_pid_check_rules(&self) -> Value {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return refuse("no_document", "Create or open a drawing first");
        }
        let rules = match self.pidcheck_load_rules(i) {
            Ok(rules) => rules,
            Err((code, why)) => return refuse(code, why),
        };
        let (sets, list) = builtin_rules::listing(&rules);
        json!({
            "ok": true,
            "rule_set_version": rules.version,
            "rule_set_digest": builtin_rules::rule_set_digest(&rules),
            "project_pack": rules.project_pack.as_ref().map(|path| path.display().to_string()),
            "save_gate": rules.save_gate.name(),
            "sets": sets,
            "rules": list,
        })
    }

    /// 控制面的 `pid_check_rules_set`：整个换掉当前图所在目录的项目包。`pack` 先照读规则包的规矩
    /// 校验（[`builtin_rules::validate_project_pack`]），全过才写：写临时文件、改名盖过去，下一次
    /// 检查起生效，留着的结果不动。结果是换上以后生效规则的版本和摘要值。只读会话里同
    /// `PIDCHECK ENABLE` 被拒（§5.4）。
    pub(in crate::app) fn control_pid_check_rules_set(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Err(refuse("no_document", "Create or open a drawing first"));
        }
        self.pidcheck_writable("PIDCHECK ENABLE", "pid_check_rules_set")?;
        let Some(dir) = crate::io::drawing_dir(&self.tabs[i].scene.document).map(Path::to_path_buf)
        else {
            return Err(refuse(
                INDEX_NO_PROJECT_DIR,
                "当前图没保存过，没有放项目包的目录；存盘后再换",
            ));
        };
        let pack = &req["pack"];
        if pack.is_null() {
            return Err(refuse(
                "invalid_request",
                "pid_check_rules_set 要给 pack：整个项目包",
            ));
        }
        builtin_rules::validate_project_pack(pack)
            .map_err(|error| refuse(error.code(), format!("项目包没有写：{error}")))?;
        let path = dir.join(PROJECT_PACK_FILE);
        write_atomic(&path, &format!("{pack:#}\n")).map_err(|error| {
            refuse(
                RULESET_IO_ERROR,
                format!("写不了 {}：{error}", path.display()),
            )
        })?;
        let rules = builtin_rules::load(Some(&dir))
            .map_err(|error| refuse(error.code(), error.to_string()))?;
        self.set_control_result(json!({
            "rule_set_version": rules.version,
            "rule_set_digest": builtin_rules::rule_set_digest(&rules),
        }));
        Ok(Task::none())
    }

    /// 控制面的 `pid_check_waive`：同 `PIDCHECK WAIVE`（[`Self::pidcheck_add_waiver`]），规则号不分
    /// 大小写，理由原样交给 [`Waiver::new`] 查。结果是图上现在有几条豁免。只读会话里同命令被拒
    /// （§5.4）。
    pub(in crate::app) fn control_pid_check_waive(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        const OP: &str = "pid_check_waive";
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Err(refuse("no_document", "Create or open a drawing first"));
        }
        self.pidcheck_writable("PIDCHECK WAIVE", OP)?;
        let rule_id = required_text(req, "rule_id", OP)?.to_ascii_uppercase();
        let key = required_text(req, "object_key", OP)?;
        let Some(reason) = req["reason"].as_str() else {
            return Err(refuse(
                "invalid_request",
                format!("{OP} 要给 reason：豁免的理由"),
            ));
        };
        let rules = self
            .pidcheck_load_rules(i)
            .map_err(|(code, why)| refuse(code, why))?;
        self.pidcheck_add_waiver(i, &rules, &rule_id, &key, reason)
            .map_err(|(code, said)| refuse(code, said))?;
        self.pidcheck_waivers_now(i);
        Ok(Task::none())
    }

    /// 控制面的 `pid_check_unwaive`：同 `PIDCHECK UNWAIVE`（[`Self::pidcheck_remove_waiver`]），规则号
    /// 不分大小写。结果是图上现在有几条豁免。只读会话里同命令被拒（§5.4）。
    pub(in crate::app) fn control_pid_check_unwaive(
        &mut self,
        req: &Value,
    ) -> Result<Task<Message>, Value> {
        const OP: &str = "pid_check_unwaive";
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Err(refuse("no_document", "Create or open a drawing first"));
        }
        self.pidcheck_writable("PIDCHECK UNWAIVE", OP)?;
        let rule_id = required_text(req, "rule_id", OP)?.to_ascii_uppercase();
        let key = required_text(req, "object_key", OP)?;
        self.pidcheck_remove_waiver(i, &rule_id, &key)
            .map_err(|(code, said)| refuse(code, said))?;
        self.pidcheck_waivers_now(i);
        Ok(Task::none())
    }

    /// 豁免改完以后操作的结果：第 `i` 个标签页的图上现在有几条豁免。
    fn pidcheck_waivers_now(&mut self, i: usize) {
        let waivers = drc::waivers(&self.tabs[i].scene.document).len();
        self.set_control_result(json!({ "waivers": waivers }));
    }

    /// 只读会话里拒绝写的操作 `op`（§5.4），同命令 `command` 被拒：码和级别同别的被拒的写，
    /// 也报给服务器。
    fn pidcheck_writable(&mut self, command: &str, op: &str) -> Result<(), Value> {
        if !self.readonly_refuses_command(command) {
            return Ok(());
        }
        let (code, level) = self.collab.refusal();
        self.collab.report_attempt("", op);
        let mut refusal = refuse(&code, "协同：当前会话只读，拒绝改规则包和豁免");
        refusal["level"] = json!(level);
        Err(refusal)
    }
}

/// 在扫好的项目上查（§4.1、§4.2），得到结果和查了的图（图号）。查全项目时范围是读得出来的
/// 全部图；查节点时是项目文件里归在这个节点及其下级的图；跨图查一张图时是这张图。范围外的
/// 作对端（`others`），项目级规则都跑。不碰命令行和程序状态。
pub(super) fn check_project(scanned: &Scanned, check: Check) -> (CheckReport, Vec<String>) {
    let Check {
        range,
        rules,
        master,
        trigger,
    } = check;
    let nodes: HashSet<&str> = match (&range, &master) {
        (Range::Pbs(node), Resolved::Configured { data, .. }) => {
            data.pbs_subtree(node).into_iter().collect()
        }
        _ => HashSet::new(),
    };
    let (drawings, others): (Vec<DrawingSummary>, Vec<DrawingSummary>) = scanned
        .summaries
        .iter()
        .cloned()
        .partition(|summary| match &range {
            Range::Project => true,
            Range::Pbs(_) => summary
                .pbs
                .as_deref()
                .is_some_and(|pbs| nodes.contains(pbs)),
            Range::Drawing(id) => summary.drawing_id == *id,
        });
    let types = TypeLib::embedded();
    let (kind, drawing_id, pbs) = match range {
        Range::Project => (ScopeKind::Project, None, None),
        Range::Pbs(node) => (ScopeKind::Pbs, None, Some(node)),
        Range::Drawing(id) => (ScopeKind::Drawing, Some(id), None),
    };
    let report = drc::check(&CheckInput {
        rules: &rules,
        master: &master,
        types: &types,
        drawings: &drawings,
        index: Some(&scanned.index),
        others: &others,
        waivers: &scanned.waivers,
        trigger,
        scope: Scope {
            kind,
            drawing_id,
            pbs,
            cross_drawing: true,
        },
    });
    let checked = drawings
        .into_iter()
        .map(|summary| summary.drawing_id)
        .collect();
    (report, checked)
}

/// 扫完项目接着查时说话打头的命令。
fn verb_of(range: &Range) -> &'static str {
    match range {
        Range::Project => "PIDCHECK PROJECT",
        Range::Pbs(_) => "PIDCHECK PBS",
        Range::Drawing(_) => "PIDCHECK",
    }
}

/// 节点号 `node` 在不在主数据的 PBS 表里（§3.8 的 `PBS_NODE_UNKNOWN`）；不在时说为什么。
pub(super) fn pbs_known(master: &Resolved, node: &str) -> Result<(), String> {
    match master {
        Resolved::Configured { data, .. } if data.pbs_subtree(node).is_empty() => {
            Err(format!("主数据的 PBS 表里没有节点 {node}"))
        }
        Resolved::Configured { .. } => Ok(()),
        Resolved::Unreadable { error, .. } => {
            Err(format!("主数据读不出来，认不出节点 {node}：{error}"))
        }
        Resolved::NotConfigured => Err(format!("没有配置主数据，认不出节点 {node}")),
    }
}

/// `pid_check` 要查的（§5.3 的 `scope`、`pbs`、`cross_drawing`）。
enum Asked {
    /// 只查当前图，项目级规则不跑。
    Drawing,
    /// 查当前图，扫项目、项目级规则也跑，只报这张图上的。
    CrossDrawing,
    /// 查这个 PBS 节点及其下级。
    Pbs(String),
    /// 查全项目。
    Project,
}

/// 读 `pid_check` 的请求：要查的，和列不列已豁免的结果（`include_waived`，默认列）。`scope` 不分
/// 大小写，默认 `drawing`；`pbs` 只和 `scope: pbs` 一起给，这时必给；`cross_drawing` 只对查当前图
/// 有用。写得不对是 `invalid_request`。
fn check_request(req: &Value) -> Result<(Asked, bool), Value> {
    let scope = match &req["scope"] {
        Value::Null => "drawing".to_owned(),
        Value::String(scope) => scope.trim().to_ascii_lowercase(),
        _ => {
            return Err(refuse(
                "invalid_request",
                "scope 写 drawing、pbs 或 project",
            ))
        }
    };
    let pbs = match &req["pbs"] {
        Value::Null => None,
        Value::String(node) if !node.trim().is_empty() => Some(node.trim().to_owned()),
        _ => return Err(refuse("invalid_request", "pbs 写不空的 PBS 节点号")),
    };
    let cross_drawing = flag(req, "cross_drawing", false)?;
    let include_waived = flag(req, "include_waived", true)?;
    let asked = match (scope.as_str(), pbs) {
        ("pbs", Some(node)) => Asked::Pbs(node),
        ("pbs", None) => {
            return Err(refuse(
                "invalid_request",
                "scope 是 pbs 时要给 pbs（PBS 节点号）",
            ))
        }
        ("drawing" | "project", Some(_)) => {
            return Err(refuse("invalid_request", "pbs 只和 scope: pbs 一起给"))
        }
        ("drawing", None) if cross_drawing => Asked::CrossDrawing,
        ("drawing", None) => Asked::Drawing,
        ("project", None) => Asked::Project,
        (other, _) => {
            return Err(refuse(
                "invalid_request",
                format!("scope 写 drawing、pbs 或 project，不认识 {other}"),
            ))
        }
    };
    Ok((asked, include_waived))
}

/// 请求里可以不给的一项真假：没给、给的是 `null` 时是 `default`；不是真假报 `invalid_request`。
fn flag(req: &Value, key: &str, default: bool) -> Result<bool, Value> {
    match &req[key] {
        Value::Null => Ok(default),
        Value::Bool(value) => Ok(*value),
        _ => Err(refuse("invalid_request", format!("{key} 写 true 或 false"))),
    }
}

/// 请求里必给的一项文本，去掉两头的空白：没给、不是文本或者是空的报 `invalid_request`。
fn required_text(req: &Value, key: &str, op: &str) -> Result<String, Value> {
    match &req[key] {
        Value::String(text) if !text.trim().is_empty() => Ok(text.trim().to_owned()),
        _ => Err(refuse(
            "invalid_request",
            format!("{op} 要给 {key}：不空的文本"),
        )),
    }
}

/// `pid_check` 的应答：§3.6 的检查结果带 `ok`；`include_waived` 为假时 `results` 里不列已豁免的，
/// 计数照旧。
fn answer_of(report: &CheckReport, include_waived: bool) -> Value {
    let mut answer = json!(Exported { ok: true, report });
    if !include_waived {
        if let Some(results) = answer["results"].as_array_mut() {
            results.retain(|result| result["waived"] != true);
        }
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

/// 列在命令行上的一条结果：严重度、规则号、（查项目时）图号、消息，括号里是对象键——豁免
/// 时用它。
pub(super) fn listed(result: &CheckResult, with_drawing: bool) -> String {
    let drawing = &result.object.drawing_id;
    let at = if with_drawing && !drawing.is_empty() {
        format!("{drawing} ")
    } else {
        String::new()
    };
    format!(
        "{} {} {at}{}（{}）",
        result.severity.name(),
        result.rule_id,
        result.message,
        result.object_key
    )
}

/// 结果查的是什么，说给人听：「图 PID-1101」「全项目」「PBS 节点 U-100」。
fn scope_said(scope: &Scope) -> String {
    match scope.kind {
        ScopeKind::Drawing => match scope.drawing_id.as_deref() {
            Some(id) if !id.is_empty() => format!("图 {id} "),
            _ => "当前图".to_owned(),
        },
        ScopeKind::Pbs => format!("PBS 节点 {} ", scope.pbs.as_deref().unwrap_or_default()),
        ScopeKind::Project => "全项目".to_owned(),
    }
}

/// `text` 的第一个词，和它后面剩下的部分。
fn first_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}

/// 去掉首尾空白和成对的双引号。
fn unquoted(name: &str) -> &str {
    let name = name.trim();
    name.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(name)
}

/// `WAIVE`、`UNWAIVE` 后面的规则号（转成大写）、对象键（有空格时加双引号，引号里的原样取）和剩下
/// 的部分。规则号、对象键没写全，或者引号没配对，是 `None`。
fn waive_words(text: &str) -> Option<(String, &str, &str)> {
    let (rule, rest) = first_word(text);
    let (key, rest) = match rest.strip_prefix('"') {
        Some(quoted) => {
            let end = quoted.find('"')?;
            (&quoted[..end], quoted[end + 1..].trim())
        }
        None => first_word(rest),
    };
    (!rule.is_empty() && !key.is_empty()).then(|| (rule.to_ascii_uppercase(), key, rest))
}

/// 项目包 `path` 改成启停 `rule_id` 以后的样子（§3.2）：`rules` 里有这条就只改它的 `enabled`，
/// 没有就在末尾加一条只带 `id`、`enabled` 的；没有项目包就新建一个只有这一条的。别的原样留着。
fn switched_pack(path: &Path, rule_id: &str, enable: bool) -> Result<Value, String> {
    let mut pack = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|error| format!("项目包 {} 不是合法的 JSON：{error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({
            "version": builtin_rules::PACK_VERSION,
            "pack": "项目规则包",
            "rule_set_version": "1.0.0",
            "rules": [],
        }),
        Err(error) => return Err(format!("读不出项目包 {}：{error}", path.display())),
    };
    let rules = pack
        .as_object_mut()
        .map(|top| top.entry("rules").or_insert_with(|| json!([])))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("项目包 {} 的 rules 不是数组", path.display()))?;
    match rules
        .iter_mut()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(rule_id))
    {
        Some(entry) => entry["enabled"] = json!(enable),
        None => rules.push(json!({ "id": rule_id, "enabled": enable })),
    }
    Ok(pack)
}

/// 现在的 Unix 秒，跟检查结果的 `checked_at` 一样算。
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
    use crate::pid::master_data::{MasterData, PbsNode};
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
                "ocs-piddrc-{}-{}-{nanos}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("临时目录");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// 程序里这个项目目录最近一次查项目、查节点的结果。
        fn report<'a>(&self, app: &'a OpenCADStudio) -> &'a CheckReport {
            let checked = app.pid_checks.projects.get(&same_file(&self.0));
            &checked.expect("按项目目录记着").report
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
        req["request_id"] = json!(format!("piddrc-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
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

    /// 当前图画成项目里的一张小图：`pumps` 台泵（位号自动编，两张图上编出来的一样），一根管线。
    fn sheet(app: &mut OpenCADStudio, pumps: usize) {
        for spot in &[[0.0, 0.0], [0.0, -60.0]][..pumps] {
            draw(app, "PIDINSERT pump", &[*spot]);
        }
        draw(app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
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

    /// 临时目录里两张图：PID-1101 上一台泵，PID-1102 上两台，头一台的位号两张图上相同。当前图
    /// 是 PID-1101，存过盘、打开着；PID-1102 只在磁盘上。
    fn project(scratch: &Scratch) -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        sheet(&mut app, 2);
        save(&app, &scratch.path("PID-1102.dwg"));
        let _ = app.automation_op(r#"{"op":"new"}"#);
        sheet(&mut app, 1);
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

    /// 用命令建一张有若干缺陷的图（§6），`PIDCHECK` 的汇总数和逐条结果与检查器直接查的一样：
    /// 列前 10 条没豁免的，多的说还有几条；项目级规则没跑要说。结果记在标签页上；检查前后
    /// 图的「已修改」、撤销栈、编辑修订号都不变。
    #[test]
    fn pidcheck_says_the_counts_and_the_first_ten_results() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for x in [0.0, 80.0, 160.0] {
            draw(&mut app, "PIDINSERT pump", &[[x, 0.0]]);
        }
        draw(&mut app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 200.0], [100.0, 200.0]]);
        let i = app.active_tab;
        let dirty = app.tabs[i].dirty;
        let steps = undo_steps(&app);
        let revision = app.tabs[i].edit_revision;

        let ran = run(&mut app, "PIDCHECK");
        assert_eq!(ran.error, None, "{}", ran.said);

        let document = &app.tabs[i].scene.document;
        let path = document.source_path.as_deref().map(Path::new);
        let rules = builtin_rules::builtin();
        let expected = drc::check_drawing(document, path, &rules, Trigger::OnDemand);
        let counts = expected.summary;
        let open: Vec<&CheckResult> = expected
            .results
            .iter()
            .filter(|result| !result.waived)
            .collect();
        assert!(
            open.len() > LISTED,
            "要多于 {LISTED} 条才查得到只列前几条：{}",
            open.len()
        );
        let said = format!(
            "PIDCHECK: 错误 {} 条，警告 {} 条，提示 {} 条",
            counts.errors, counts.warnings, counts.infos
        );
        assert!(ran.said.contains(&said), "{}", ran.said);
        let said = format!(
            "查了 {} 个对象，跑了 {} 条规则",
            counts.objects, counts.rules_run
        );
        assert!(ran.said.contains(&said), "{}", ran.said);
        for result in open.iter().take(LISTED) {
            let line = listed(result, false);
            assert!(ran.said.contains(&line), "没列出 {line}：{}", ran.said);
        }
        for result in open.iter().skip(LISTED) {
            let line = listed(result, false);
            assert!(!ran.said.contains(&line), "多列了 {line}：{}", ran.said);
        }
        let more = format!("还有 {} 条没列出", open.len() - LISTED);
        assert!(ran.said.contains(&more), "{}", ran.said);
        assert!(
            ran.said
                .contains("PIDCHECK: PROJECT_RULES_SKIPPED: 只检查了当前图"),
            "{}",
            ran.said
        );

        let kept = app.tabs[i].pid_check.as_ref().expect("结果记在标签页上");
        assert_eq!(kept.report.summary, counts);
        assert_eq!(kept.report.results, expected.results);
        assert_eq!(kept.report.scope.kind, ScopeKind::Drawing);
        assert_eq!(app.tabs[i].dirty, dirty);
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(app.tabs[i].edit_revision, revision);
    }

    /// `PIDCHECK PROJECT` 查出另一张图造成的跨图重号（`DR-102`），两张图上各一条；结果按项目
    /// 目录记在程序里，范围是 `project`。只查当前图的 `PIDCHECK` 不跑这条。
    #[test]
    fn pidcheck_project_finds_a_tag_repeated_on_another_drawing() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);

        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(
            ran.said.contains("PIDCHECK PROJECT: 2 张图，"),
            "{}",
            ran.said
        );
        let report = scratch.report(&app);
        assert_eq!(report.scope.kind, ScopeKind::Project);
        assert!(report.scope.cross_drawing);
        let mut repeated: Vec<&str> = report
            .results
            .iter()
            .filter(|result| result.rule_id == "DR-102")
            .map(|result| result.object.drawing_id.as_str())
            .collect();
        repeated.sort_unstable();
        assert_eq!(repeated, ["PID-1101", "PID-1102"], "{:#?}", report.results);

        let ran = run(&mut app, "PIDCHECK");
        assert_eq!(ran.error, None, "{}", ran.said);
        let i = app.active_tab;
        let kept = app.tabs[i].pid_check.as_ref().expect("当前图的结果");
        assert!(
            kept.report
                .results
                .iter()
                .all(|result| result.rule_id != "DR-102"),
            "{}",
            ran.said
        );
        assert!(ran.said.contains("PROJECT_RULES_SKIPPED"), "{}", ran.said);
    }

    /// `PIDCHECK PBS <节点号>` 只查项目文件里归在这个节点及其下级的图（§4.2），项目级规则照跑：
    /// 范围外那张图上的同位号照样算重号，结果只落在范围内的图上。
    #[test]
    fn pidcheck_pbs_checks_the_drawings_under_the_node() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        write_project_file(&scratch);

        for (line, drawing) in [
            ("PIDCHECK PBS U-100", "PID-1101"),
            ("pidcheck pbs U-200", "PID-1102"),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert!(
                ran.said.contains("PIDCHECK PBS: 1 张图，"),
                "{line}: {}",
                ran.said
            );
            let report = scratch.report(&app);
            assert_eq!(report.scope.kind, ScopeKind::Pbs, "{line}");
            assert!(
                report
                    .results
                    .iter()
                    .all(|result| result.object.drawing_id == drawing),
                "{line}: {:#?}",
                report.results
            );
            assert!(
                report
                    .results
                    .iter()
                    .any(|result| result.rule_id == "DR-102"),
                "{line}: {:#?}",
                report.results
            );
        }
        let report = scratch.report(&app);
        assert_eq!(report.scope.pbs.as_deref(), Some("U-200"));
    }

    /// 查不了时说为什么，连同错误码：当前图没保存过（`INDEX_NO_PROJECT_DIR`）；主数据没配置、
    /// 节点号不在 PBS 表里（`PBS_NODE_UNKNOWN`）；项目包有错时哪个范围都不查，`PIDCHECK RULES` 把错
    /// 报出来。
    #[test]
    fn pidcheck_says_why_it_did_not_check() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = None;
        for line in ["PIDCHECK PROJECT", "PIDCHECK PBS U-100"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(INDEX_NO_PROJECT_DIR),
                "{line}: {}",
                ran.said
            );
        }

        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ran = run(&mut app, "PIDCHECK PBS U-100");
        assert_eq!(ran.code.as_deref(), Some(PBS_NODE_UNKNOWN), "{}", ran.said);
        assert!(
            ran.error
                .is_some_and(|error| error.contains("没有配置主数据")),
            "{}",
            ran.said
        );
        write_project_file(&scratch);
        let ran = run(&mut app, "PIDCHECK PBS U-999");
        assert_eq!(ran.code.as_deref(), Some(PBS_NODE_UNKNOWN), "{}", ran.said);
        assert!(app.pid_checks.projects.is_empty());

        fs::write(scratch.path(PROJECT_PACK_FILE), "{ 不是 JSON").expect("项目包");
        for line in [
            "PIDCHECK",
            "PIDCHECK PROJECT",
            "PIDCHECK PBS U-100",
            "PIDCHECK RULES",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some("RULESET_SCHEMA_ERROR"),
                "{line}: {}",
                ran.said
            );
            assert!(
                ran.error
                    .is_some_and(|error| error.contains(PROJECT_PACK_FILE)),
                "{line}: {}",
                ran.said
            );
        }
        let i = app.active_tab;
        assert!(app.tabs[i].pid_check.is_none());
        assert!(app.pid_checks.projects.is_empty());
    }

    /// 磁盘上那张图里记着的豁免照样套上（§3.7）：读图时一起读出来的。规则包换了连接容差，
    /// 没变的文件也重读一遍（摘要按规则包的容差做，§4.1），容差没再变就用缓存；`PIDINDEX`
    /// 照旧用默认容差。
    #[test]
    fn pidcheck_project_reads_waivers_from_disk_and_summarizes_at_the_pack_tolerance() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert!(ran.said.contains("要从磁盘读 1 个文件"), "{}", ran.said);
        let key = scratch
            .report(&app)
            .results
            .iter()
            .find(|result| result.rule_id == "DR-102" && result.object.drawing_id == "PID-1102")
            .map(|result| result.object_key.clone())
            .expect("PID-1102 上的重号");

        let file = scratch.path("PID-1102.dwg");
        let mut drawing = crate::io::load_file(&file).expect("读 PID-1102");
        let reason = "同一台泵画在两张图上";
        let waiver = Waiver::new("DR-102", &key, reason, "tester", 1_794_800_000).expect("豁免");
        drc::set_waivers(&mut drawing, &[waiver]);
        let bytes = crate::io::save_to_bytes(&drawing, "dwg", document::DxfVersion::AC1032)
            .expect("存 DWG");
        fs::write(&file, bytes).expect("写文件");

        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert!(ran.said.contains("要从磁盘读 1 个文件"), "{}", ran.said);
        let report = scratch.report(&app);
        let waived: Vec<&str> = report
            .results
            .iter()
            .filter(|result| result.waived)
            .map(|result| result.object.drawing_id.as_str())
            .collect();
        assert_eq!(waived, ["PID-1102"], "{:#?}", report.results);
        assert_eq!(report.summary.waived, 1);
        assert!(
            report
                .results
                .iter()
                .any(|result| result.rule_id == "DR-102"
                    && result.object.drawing_id == "PID-1101"
                    && !result.waived),
            "{:#?}",
            report.results
        );
        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert!(!ran.said.contains("要从磁盘读"), "{}", ran.said);

        let pack = json!({ "version": 1, "rule_set_version": "P1", "connect_tolerance": 0.5 });
        fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("项目包");
        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(
            ran.said.contains("要从磁盘读 1 个文件"),
            "容差换了要重读：{}",
            ran.said
        );
        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert!(!ran.said.contains("要从磁盘读"), "{}", ran.said);
        let ran = run(&mut app, "PIDINDEX");
        assert!(
            ran.said.contains("要从磁盘读 1 个文件"),
            "PIDINDEX 用默认容差：{}",
            ran.said
        );
    }

    /// `PIDCHECK RULES` 一条一行列出生效的规则：号、名称、严重度、启停、来源；本期没有对象的
    /// 三条标出来。项目包停用的规则写「停用」。
    #[test]
    fn pidcheck_rules_lists_the_rules_in_effect() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let builtin = builtin_rules::builtin();
        let ran = run(&mut app, "PIDCHECK RULES");
        assert_eq!(ran.error, None, "{}", ran.said);
        let enabled = builtin
            .rules
            .iter()
            .filter(|rule| builtin.is_enabled(rule))
            .count();
        let said = format!("{} 条规则，启用 {enabled} 条", builtin.rules.len());
        assert!(ran.said.contains(&said), "{}", ran.said);
        assert!(ran.said.contains("只用内置包"), "{}", ran.said);
        for rule in &builtin.rules {
            let head = format!("{} {}：{}，", rule.id, rule.name, rule.severity.name());
            let line = ran.said.lines().find(|line| line.contains(&head));
            assert!(line.is_some(), "没列出 {head}：{}", ran.said);
            let silent = line.is_some_and(|line| line.ends_with("本期没有对象"));
            assert_eq!(silent, NO_OBJECTS_YET.contains(&rule.id.as_str()), "{head}");
        }

        let pack = json!({
            "version": 1,
            "rule_set_version": "P1",
            "rules": [{ "id": "DR-014", "enabled": false }]
        });
        fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("项目包");
        let ran = run(&mut app, "PIDCHECK RULES");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(ran.said.contains("内置包叠项目包"), "{}", ran.said);
        let rule = builtin.rule("DR-014").expect("DR-014");
        let head = format!("DR-014 {}：{}，停用，", rule.name, rule.severity.name());
        assert!(ran.said.contains(&head), "{}", ran.said);
    }

    /// `PIDCHECK EXPORT <文件>` 写出的 JSON 就是最近一次的结果（§3.6，带 `ok`）：查当前图的和
    /// 查项目的哪个新写哪个。还没查过、不是 .json、是项目文件时不写。
    #[test]
    fn pidcheck_export_writes_the_latest_result() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let out = scratch.path("drc.json");
        let ran = run(&mut app, "PIDCHECK EXPORT drc.json");
        assert!(
            ran.error
                .is_some_and(|error| error.contains("还没有检查结果")),
            "{}",
            ran.said
        );
        assert!(!out.exists());

        let exported = |app: &mut OpenCADStudio| -> Value {
            let ran = run(app, "PIDCHECK EXPORT drc.json");
            assert_eq!(ran.error, None, "{}", ran.said);
            let text = fs::read_to_string(&out).expect("读导出的文件");
            serde_json::from_str(&text).expect("JSON")
        };
        let with_ok = |report: &CheckReport| -> Value {
            let text = serde_json::to_string(report).expect("JSON");
            let mut value: Value = serde_json::from_str(&text).expect("JSON");
            value["ok"] = json!(true);
            value
        };

        run(&mut app, "PIDCHECK");
        let value = exported(&mut app);
        let i = app.active_tab;
        let kept = app.tabs[i].pid_check.as_ref().expect("当前图的结果");
        assert_eq!(value, with_ok(&kept.report));
        assert_eq!(value["scope"]["kind"], "drawing");

        run(&mut app, "PIDCHECK PROJECT");
        let value = exported(&mut app);
        assert_eq!(value, with_ok(scratch.report(&app)));
        assert_eq!(value["scope"]["kind"], "project");

        run(&mut app, "PIDCHECK");
        assert_eq!(exported(&mut app)["scope"]["kind"], "drawing");

        let ran = run(&mut app, "PIDCHECK EXPORT drc.txt");
        assert!(
            ran.error.is_some_and(|error| error.contains("只写 .json")),
            "{}",
            ran.said
        );
        let ran = run(&mut app, &format!("PIDCHECK EXPORT {PROJECT_FILE}"));
        assert!(
            ran.error.is_some_and(|error| error.contains("是项目文件")),
            "{}",
            ran.said
        );
        assert!(!scratch.path(PROJECT_FILE).exists());
        assert!(!scratch.path("drc.txt").exists());
    }

    /// 只读会话里五个命令都能用，不留撤销步（§5.2）。
    #[test]
    fn pidcheck_runs_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        write_project_file(&scratch);
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for (line, said) in [
            ("PIDCHECK", "PIDCHECK: 错误"),
            ("PIDCHECK PROJECT", "PIDCHECK PROJECT: 2 张图"),
            ("pidcheck pbs U-100", "PIDCHECK PBS: 1 张图"),
            ("PIDCHECK RULES", "PIDCHECK RULES: 规则版本"),
            ("PIDCHECK EXPORT drc.json", "PIDCHECK EXPORT: 已把"),
        ] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert!(ran.said.contains(said), "{line}: {}", ran.said);
        }
        assert_eq!(undo_steps(&app), steps);
    }

    /// 命令行上的 `PIDCHECK PROJECT` 整行是一条命令，只查一次：不先拿裸动词查一遍当前图。
    #[test]
    fn pidcheck_project_from_the_command_line_checks_once() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ran = run(&mut app, "PIDCHECK PROJECT");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(
            ran.said.matches("PIDCHECK PROJECT: 2 张图").count(),
            1,
            "{}",
            ran.said
        );
        assert!(
            !ran.said.contains("PIDCHECK: 错误"),
            "裸的 PIDCHECK 先跑了：{}",
            ran.said
        );
        let i = app.active_tab;
        assert!(app.tabs[i].pid_check.is_none(), "裸的 PIDCHECK 先跑了");
    }

    fn undo(app: &mut OpenCADStudio) {
        let _ = app.automation_op(r#"{"op":"undo"}"#);
    }

    fn redo(app: &mut OpenCADStudio) {
        let _ = app.automation_op(r#"{"op":"redo"}"#);
    }

    /// 当前标签页上留着的检查结果。
    fn kept(app: &OpenCADStudio) -> &Checked {
        app.tabs[app.active_tab]
            .pid_check
            .as_ref()
            .expect("当前图的结果")
    }

    /// 当前图里记着的豁免。
    fn recorded(app: &OpenCADStudio) -> Vec<Waiver> {
        drc::waivers(&app.tabs[app.active_tab].scene.document)
    }

    /// `report` 里规则 `rule_id` 在图 `drawing` 上的头一条结果的对象键。
    fn key_of(report: &CheckReport, rule_id: &str, drawing: &str) -> String {
        report
            .results
            .iter()
            .find(|result| result.rule_id == rule_id && result.object.drawing_id == drawing)
            .map(|result| result.object_key.clone())
            .unwrap_or_else(|| panic!("{drawing} 上没有 {rule_id}：{:#?}", report.results))
    }

    /// 查一遍当前图，结果里有没有规则 `rule_id`。
    fn fires(app: &mut OpenCADStudio, rule_id: &str) -> bool {
        let ran = run(app, "PIDCHECK");
        assert_eq!(ran.error, None, "{}", ran.said);
        kept(app)
            .report
            .results
            .iter()
            .any(|result| result.rule_id == rule_id)
    }

    /// `PIDCHECK DISABLE <规则号>` 在项目包里停用这条规则（没有项目包就新建一个，只写这一条），
    /// 之后检查结果里没有它；`ENABLE` 后恢复（§6 第 5 条，命令层换成真图上会命中的 `DR-014`）。
    /// 规则号小写也认。规则号不存在是 `RULE_NOT_FOUND`，项目包不动。
    #[test]
    fn pidcheck_disable_and_enable_switch_a_rule_in_the_project_pack() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let pack = scratch.path(PROJECT_PACK_FILE);
        assert!(fires(&mut app, "DR-014"), "管线两头都没接上");
        assert!(!pack.exists());

        let ran = run(&mut app, "PIDCHECK DISABLE DR-014");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(ran.said.contains("已停用 DR-014"), "{}", ran.said);
        let text = fs::read_to_string(&pack).expect("新建的项目包");
        let written: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            written["rules"],
            json!([{ "id": "DR-014", "enabled": false }])
        );
        assert_eq!(builtin_rules::validate_project_pack(&written), Ok(()));
        assert!(!fires(&mut app, "DR-014"), "停用以后没有它");

        let ran = run(&mut app, "pidcheck enable dr-014");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert!(ran.said.contains("已启用 DR-014"), "{}", ran.said);
        assert!(fires(&mut app, "DR-014"), "启用以后恢复");

        let text = fs::read_to_string(&pack).expect("项目包");
        for line in ["PIDCHECK DISABLE DR-999", "PIDCHECK ENABLE XX-001"] {
            let ran = run(&mut app, line);
            assert_eq!(
                ran.code.as_deref(),
                Some(RULE_NOT_FOUND),
                "{line}: {}",
                ran.said
            );
        }
        assert_eq!(fs::read_to_string(&pack).expect("项目包"), text);
    }

    /// 启停只改项目包里这条规则的 `enabled`，别的原样留着：项目包自己的规则改它那一项，
    /// 内置规则没有条目就加一条只带 `id`、`enabled` 的（§3.2）。本来就是那样时不写文件；
    /// 规则集停用着要提醒。项目包有错、图没保存过时什么都不写，说为什么。
    #[test]
    fn pidcheck_enable_and_disable_change_only_that_rule_in_the_pack() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let pack = scratch.path(PROJECT_PACK_FILE);
        let original = json!({
            "version": 1,
            "pack": "项目规则",
            "rule_set_version": "P1",
            "connect_tolerance": 0.001,
            "sets": [{ "name": "工程扩展", "enabled": false }],
            "rules": [
                { "id": "DR-114", "enabled": true, "params": { "允许图层": ["0"] } },
                {
                    "id": "DR-201", "name": "项目自己的规则", "set": "基础规则",
                    "category": "设计类", "target": "管线", "severity": "提示",
                    "expr": "管道等级 = NULL", "scope": "项目", "source": "新建"
                }
            ]
        });
        fs::write(&pack, original.to_string()).expect("项目包");
        let read = || -> Value {
            let text = fs::read_to_string(&pack).expect("项目包");
            serde_json::from_str(&text).expect("JSON")
        };

        for line in ["PIDCHECK DISABLE DR-201", "PIDCHECK DISABLE DR-014"] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        let mut expected = original.clone();
        expected["rules"][1]["enabled"] = json!(false);
        if let Some(rules) = expected["rules"].as_array_mut() {
            rules.push(json!({ "id": "DR-014", "enabled": false }));
        }
        assert_eq!(read(), expected);

        let text = fs::read_to_string(&pack).expect("项目包");
        for (line, said) in [
            ("PIDCHECK ENABLE DR-114", "本来就启用着"),
            ("PIDCHECK DISABLE DR-014", "本来就停用着"),
            ("PIDCHECK ENABLE DR-101", "规则集 工程扩展 停用着"),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert!(ran.said.contains(said), "{line}: {}", ran.said);
        }
        assert_eq!(
            fs::read_to_string(&pack).expect("项目包"),
            text,
            "本来就是那样时不写"
        );

        fs::write(&pack, "{ 不是 JSON").expect("项目包");
        let ran = run(&mut app, "PIDCHECK ENABLE DR-014");
        assert_eq!(
            ran.code.as_deref(),
            Some("RULESET_SCHEMA_ERROR"),
            "{}",
            ran.said
        );
        assert_eq!(fs::read_to_string(&pack).expect("项目包"), "{ 不是 JSON");

        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = None;
        let ran = run(&mut app, "PIDCHECK DISABLE DR-014");
        assert_eq!(
            ran.code.as_deref(),
            Some(INDEX_NO_PROJECT_DIR),
            "{}",
            ran.said
        );
    }

    /// `PIDCHECK WAIVE <规则号> <对象键> <理由>` 豁免当前图最近一次结果里的一条（§3.7）：写进图、
    /// 一个撤销步，留着的结果就地改——那条的 `waived`、`waiver`，计数和 `blocked` 立即变，不重新
    /// 检查（序号不变）。对象键可以加双引号，理由去掉首尾空白。把错误逐条豁免完，`blocked` 变假。
    /// 存盘再打开豁免还在；撤销一步，最后那条豁免从图里没了，结果也跟着退回；重做又回来。
    #[test]
    fn pidcheck_waive_changes_the_kept_result_at_once_and_stays_with_the_drawing() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ran = run(&mut app, "PIDCHECK");
        assert_eq!(ran.error, None, "{}", ran.said);
        let serial = kept(&app).serial;
        let results = kept(&app).report.results.clone();
        let before = kept(&app).report.summary;
        assert!(kept(&app).report.blocked);
        let mut errors: Vec<(String, String)> = Vec::new();
        for result in &results {
            let pair = (result.rule_id.clone(), result.object_key.clone());
            if result.severity == drc::Severity::Error && !errors.contains(&pair) {
                errors.push(pair);
            }
        }
        assert!(
            errors.iter().any(|(rule, _)| rule == "DR-014"),
            "{results:#?}"
        );
        let counted = |done: &[(String, String)]| -> (usize, usize) {
            let waived = |result: &&CheckResult| {
                done.contains(&(result.rule_id.clone(), result.object_key.clone()))
            };
            let open = results
                .iter()
                .filter(|result| result.severity == drc::Severity::Error)
                .filter(|result| !waived(result))
                .count();
            (open, before.waived + results.iter().filter(waived).count())
        };
        let i = app.active_tab;
        app.tabs[i].dirty = false;
        let steps = undo_steps(&app);

        for (n, (rule, key)) in errors.iter().enumerate() {
            let line = format!("PIDCHECK WAIVE {rule} \"{key}\"  放空管，端部通大气 ");
            let ran = run(&mut app, &line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(kept(&app).serial, serial, "不重新检查");
            let report = &kept(&app).report;
            let (open, waived) = counted(&errors[..=n]);
            assert_eq!(
                (report.summary.errors, report.summary.waived),
                (open, waived),
                "{line}"
            );
            assert_eq!(report.blocked, open > 0, "{line}");
        }
        assert!(!kept(&app).report.blocked);
        assert_eq!(undo_steps(&app), steps + errors.len());
        assert!(app.tabs[i].dirty);
        let result = kept(&app)
            .report
            .results
            .iter()
            .find(|result| result.rule_id == "DR-014")
            .expect("DR-014");
        assert!(result.waived);
        let waiver = result.waiver.as_ref().expect("挂着豁免");
        assert_eq!(waiver.reason, "放空管，端部通大气");
        assert!(waiver.at > 1_700_000_000, "{}", waiver.at);
        let written = recorded(&app);
        let pairs: Vec<(String, String)> = written
            .iter()
            .map(|waiver| (waiver.rule_id.clone(), waiver.object_key.clone()))
            .collect();
        assert_eq!(pairs, errors);

        let path = scratch.path("PID-1101.dwg");
        save(&app, &path);
        let reopened = crate::io::load_file(&path).expect("再打开");
        assert_eq!(drc::waivers(&reopened), written, "存盘再打开豁免还在");

        undo(&mut app);
        assert_eq!(recorded(&app), &written[..written.len() - 1]);
        let (open, waived) = counted(&errors[..errors.len() - 1]);
        let report = &kept(&app).report;
        assert_eq!(
            (report.summary.errors, report.summary.waived),
            (open, waived)
        );
        assert!(report.blocked, "撤销一步，结果也退回");
        redo(&mut app);
        assert_eq!(recorded(&app), written);
        assert!(!kept(&app).report.blocked);
    }

    /// 豁免不了时说为什么，连同错误码，图和撤销栈都不动：还没检查过、要豁免的不在最近一次的
    /// 结果里（`WAIVER_TARGET_NOT_FOUND`）；规则号不存在（`RULE_NOT_FOUND`）；理由是空的、超过 200 字
    /// （`WAIVER_REASON_REQUIRED`、`WAIVER_REASON_TOO_LONG`）。正好 200 字可以。
    #[test]
    fn pidcheck_waive_says_why_it_cannot() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ran = run(&mut app, "PIDCHECK WAIVE DR-014 PIPE_LINE|L-1 放空管");
        assert_eq!(
            ran.code.as_deref(),
            Some(WAIVER_TARGET_NOT_FOUND),
            "{}",
            ran.said
        );
        run(&mut app, "PIDCHECK");
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let steps = undo_steps(&app);
        let long = "长".repeat(drc::WAIVER_REASON_LIMIT + 1);
        for (line, code) in [
            (format!("PIDCHECK WAIVE DR-999 {key} 理由"), RULE_NOT_FOUND),
            (
                format!("PIDCHECK WAIVE DR-014 {key}-X 理由"),
                WAIVER_TARGET_NOT_FOUND,
            ),
            (
                format!("PIDCHECK WAIVE DR-102 {key} 理由"),
                WAIVER_TARGET_NOT_FOUND,
            ),
            (
                format!("PIDCHECK WAIVE DR-014 {key}"),
                "WAIVER_REASON_REQUIRED",
            ),
            (
                format!("PIDCHECK WAIVE DR-014 {key} \"  \""),
                "WAIVER_REASON_REQUIRED",
            ),
            (
                format!("PIDCHECK WAIVE DR-014 {key} {long}"),
                "WAIVER_REASON_TOO_LONG",
            ),
        ] {
            let ran = run(&mut app, &line);
            assert_eq!(ran.code.as_deref(), Some(code), "{line}: {}", ran.said);
        }
        assert_eq!(undo_steps(&app), steps);
        assert!(recorded(&app).is_empty());

        let exactly = "长".repeat(drc::WAIVER_REASON_LIMIT);
        let ran = run(&mut app, &format!("PIDCHECK WAIVE DR-014 {key} {exactly}"));
        assert_eq!(ran.error, None, "正好 200 字可以：{}", ran.said);
        assert_eq!(recorded(&app).len(), 1);
    }

    /// 查过项目以后豁免当前图上的一条：最近一次查到这张图的是查项目的那份，它里面当前图那条
    /// 就地改，另一张图上对象键相同的那条不动；标签页上查当前图的结果也跟着改。只在另一张图
    /// 上的结果不能在这张图上豁免（`WAIVER_TARGET_NOT_FOUND`）。
    #[test]
    fn pidcheck_waive_after_a_project_check_marks_this_drawing_only() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        run(&mut app, "PIDCHECK");
        run(&mut app, "PIDCHECK PROJECT");
        let report = scratch.report(&app);
        let repeated = key_of(report, "DR-102", "PID-1101");
        assert_eq!(repeated, key_of(report, "DR-102", "PID-1102"));
        let elsewhere = report
            .results
            .iter()
            .find(|result| {
                result.object.drawing_id == "PID-1102"
                    && !report.results.iter().any(|other| {
                        other.object.drawing_id == "PID-1101"
                            && other.rule_id == result.rule_id
                            && other.object_key == result.object_key
                    })
            })
            .map(|result| format!("{} {}", result.rule_id, result.object_key))
            .expect("只在 PID-1102 上的一条");
        let is_error = |rule_id: &str| {
            report
                .results
                .iter()
                .any(|result| result.rule_id == rule_id && result.severity == drc::Severity::Error)
        };
        let errors = report.summary.errors - usize::from(is_error("DR-102"));
        let pipe = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let tab_errors = kept(&app).report.summary.errors;

        let ran = run(
            &mut app,
            &format!("PIDCHECK WAIVE DR-102 {repeated} 同一台泵画在两张图上"),
        );
        assert_eq!(ran.error, None, "{}", ran.said);
        let report = scratch.report(&app);
        let waived: Vec<&str> = report
            .results
            .iter()
            .filter(|result| result.waived)
            .map(|result| result.object.drawing_id.as_str())
            .collect();
        assert_eq!(waived, ["PID-1101"], "{:#?}", report.results);
        assert_eq!(report.summary.errors, errors);
        assert_eq!(kept(&app).report.summary.errors, tab_errors);

        let ran = run(&mut app, &format!("PIDCHECK WAIVE DR-014 {pipe} 放空管"));
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(kept(&app).report.summary.errors, tab_errors - 1);
        assert!(
            scratch
                .report(&app)
                .results
                .iter()
                .any(|result| result.rule_id == "DR-014"
                    && result.object.drawing_id == "PID-1101"
                    && result.waived),
            "{:#?}",
            scratch.report(&app).results
        );

        let ran = run(&mut app, &format!("PIDCHECK WAIVE {elsewhere} 理由"));
        assert_eq!(
            ran.code.as_deref(),
            Some(WAIVER_TARGET_NOT_FOUND),
            "{}",
            ran.said
        );
        assert_eq!(recorded(&app).len(), 2);
    }

    /// `PIDCHECK UNWAIVE <规则号> <对象键>` 取消当前图上的豁免：图里去掉、一个撤销步，结果就地
    /// 退回。对不上结果的老豁免（`stale_waivers` 里的，包括规则包里已经没有的规则的）也能取消，
    /// 从 `stale_waivers` 里去掉，撤销一步又回来。图里没有这条豁免是 `WAIVER_TARGET_NOT_FOUND`。
    #[test]
    fn pidcheck_unwaive_takes_a_waiver_away() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let i = app.active_tab;
        let stale = [
            Waiver::new(
                "DR-014",
                "PIPE_LINE|L-0000",
                "管线早删了",
                "tester",
                1_794_800_000,
            )
            .expect("豁免"),
            Waiver::new(
                "AB-123",
                "PUMP|P-0000",
                "规则早删了",
                "tester",
                1_794_800_000,
            )
            .expect("豁免"),
        ];
        drc::set_waivers(&mut app.tabs[i].scene.document, &stale);
        run(&mut app, "PIDCHECK");
        assert_eq!(kept(&app).report.stale_waivers, stale);
        let before = kept(&app).report.summary;
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let steps = undo_steps(&app);

        run(&mut app, &format!("PIDCHECK WAIVE DR-014 {key} 放空管"));
        assert_ne!(kept(&app).report.summary, before);
        let ran = run(&mut app, &format!("PIDCHECK UNWAIVE DR-014 {key}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(recorded(&app), stale);
        assert_eq!(kept(&app).report.summary, before);
        assert!(kept(&app)
            .report
            .results
            .iter()
            .all(|result| !result.waived));
        assert_eq!(undo_steps(&app), steps + 2);

        let ran = run(&mut app, "pidcheck unwaive AB-123 PUMP|P-0000");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(recorded(&app), &stale[..1]);
        assert_eq!(kept(&app).report.stale_waivers, &stale[..1]);
        undo(&mut app);
        assert_eq!(recorded(&app), stale);
        assert_eq!(kept(&app).report.stale_waivers, stale);

        let ran = run(&mut app, &format!("PIDCHECK UNWAIVE DR-014 {key}"));
        assert_eq!(
            ran.code.as_deref(),
            Some(WAIVER_TARGET_NOT_FOUND),
            "{}",
            ran.said
        );
        assert_eq!(undo_steps(&app), steps + 2);
    }

    /// 只读会话里启停规则、豁免、取消豁免都被拒：项目包和图里的豁免都不动，不留撤销步；
    /// `PIDCHECK` 照常（§5.2、§6 第 11 条）。
    #[test]
    fn pidcheck_writes_are_refused_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        run(&mut app, "PIDCHECK");
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for line in [
            "PIDCHECK ENABLE DR-014".to_owned(),
            "pidcheck disable DR-014".to_owned(),
            format!("PIDCHECK WAIVE DR-014 {key} 放空管"),
            format!("PIDCHECK UNWAIVE DR-014 {key}"),
        ] {
            assert!(app.readonly_refuses_command(&line), "{line}");
            let ran = run(&mut app, &line);
            assert!(ran.error.is_some(), "{line}: {}", ran.said);
        }
        assert_eq!(undo_steps(&app), steps);
        assert!(recorded(&app).is_empty());
        assert!(!scratch.path(PROJECT_PACK_FILE).exists());
        for line in ["PIDCHECK", "PIDCHECK RULES"] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
    }

    #[test]
    fn pidcheck_is_registered_and_says_how_it_is_written() {
        assert!(crate::command::all_registered_command_names().contains(&"PIDCHECK"));
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for line in [
            "PIDCHECK FROB",
            "PIDCHECK PROJECT NOW",
            "pidcheck rules all",
            "PIDCHECK PBS",
            "PIDCHECK PBS U-100 U-200",
            "PIDCHECK EXPORT",
            "PIDCHECK ENABLE",
            "PIDCHECK DISABLE DR-014 DR-015",
            "PIDCHECK WAIVE",
            "PIDCHECK WAIVE DR-014",
            "PIDCHECK UNWAIVE DR-014",
            "PIDCHECK UNWAIVE DR-014 PUMP|P-1 多了",
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.code, None, "{line}: {}", ran.said);
            assert!(
                ran.error.is_some_and(|error| error.starts_with("PIDCHECK")),
                "{line}: {}",
                ran.said
            );
        }
    }

    /// 发一条 `pid_check`；应答 `scanning` 就用同样的请求再发（§5.3），直到有结果。
    fn checked(app: &mut OpenCADStudio, req: &Value) -> Value {
        for _ in 0..5 {
            let answer = request(app, req.clone());
            if answer["status"] != "scanning" {
                return answer;
            }
        }
        panic!("一直应答 scanning：{req}");
    }

    /// 应答里的 `results`。
    fn results_of(answer: &Value) -> Vec<&Value> {
        answer["results"].as_array().into_iter().flatten().collect()
    }

    /// 应答里规则 `rule_id` 的结果落在哪些图上，图号排好序。
    fn drawings_hit(answer: &Value, rule_id: &str) -> Vec<String> {
        let mut drawings: Vec<String> = results_of(answer)
            .into_iter()
            .filter(|result| result["rule_id"] == rule_id)
            .filter_map(|result| result["object"]["drawing_id"].as_str())
            .map(str::to_owned)
            .collect();
        drawings.sort_unstable();
        drawings
    }

    /// 读得进来的项目包：停用 `DR-014`，加一条项目自己的规则 `DR-201`，保存门禁 `block`。
    fn project_pack() -> Value {
        json!({
            "version": 1,
            "pack": "项目规则",
            "rule_set_version": "P2",
            "save_gate": "block",
            "rules": [
                { "id": "DR-014", "enabled": false },
                {
                    "id": "DR-201", "name": "项目自己的规则", "set": "基础规则",
                    "category": "设计类", "target": "管线", "severity": "提示",
                    "expr": "管道等级 = NULL", "scope": "项目", "source": "新建"
                }
            ]
        })
    }

    /// 三个校验不过的项目包和它们的错误码（§3.8）：改内置规则的严重度、新规则用内置号段、
    /// 表达式解析不过。
    fn broken_packs() -> [(Value, &'static str); 3] {
        let mut readonly = project_pack();
        readonly["rules"][0]["severity"] = json!("提示");
        let mut reserved = project_pack();
        reserved["rules"][1]["id"] = json!("DR-199");
        let mut unparsed = project_pack();
        unparsed["rules"][1]["expr"] = json!("管道等级 =");
        [
            (readonly, "RULE_BUILTIN_READONLY"),
            (reserved, "RULE_ID_RESERVED"),
            (unparsed, drc::DSL_PARSE_ERROR),
        ]
    }

    /// `pid_check`（§5.3）的应答就是检查结果（§3.6）带上 `ok`，三种范围各一例。不写 `scope` 查
    /// 当前图，和保存门禁用的函数查的一样，结果记在标签页上；`project` 要从磁盘读另一张图，先应答
    /// `scanning`（读了几个、共几个），同样的请求再发才是结果，有跨图重号，按项目目录记下；`pbs`
    /// 只报节点下那张图上的。是查询：不带 `request_id` 也能发，不留撤销步。能力清单里有 `pid_check`。
    #[test]
    fn pid_check_op_answers_the_check_result_for_each_scope() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        write_project_file(&scratch);
        let steps = undo_steps(&app);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        assert!(
            state["capabilities"]
                .as_array()
                .is_some_and(|names| names.contains(&json!("pid_check"))),
            "{state}"
        );

        let answer = app.automation_op(r#"{"protocol":1,"op":"pid_check"}"#);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["version"], drc::REPORT_VERSION, "{answer}");
        assert_eq!(answer["trigger"], "按需", "{answer}");
        assert_eq!(
            answer["scope"],
            json!({"kind": "drawing", "drawing_id": "PID-1101", "pbs": null, "cross_drawing": false}),
            "{answer}"
        );
        let rules = builtin_rules::load(Some(&scratch.0)).expect("规则");
        let path = scratch.path("PID-1101.dwg");
        let direct = drc::check_drawing(
            app.active_document_for_test(),
            Some(&path),
            &rules,
            Trigger::OnDemand,
        );
        assert_eq!(answer["rule_set_digest"], direct.rule_set_digest.as_str());
        assert_eq!(answer["summary"], json!(direct.summary), "{answer}");
        assert_eq!(answer["results"], json!(direct.results), "{answer}");
        assert!(drawings_hit(&answer, "DR-102").is_empty(), "{answer}");
        assert_eq!(kept(&app).report.summary, direct.summary);
        assert_eq!(kept(&app).drawings, ["PID-1101"]);

        let ask = json!({"op": "pid_check", "scope": "project"});
        let answer = request(&mut app, ask.clone());
        assert_eq!(
            answer,
            json!({"ok": true, "status": "scanning", "done": 0, "total": 1})
        );
        let answer = request(&mut app, ask);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["scope"],
            json!({"kind": "project", "drawing_id": null, "pbs": null, "cross_drawing": true}),
            "{answer}"
        );
        assert_eq!(drawings_hit(&answer, "DR-102"), ["PID-1101", "PID-1102"]);
        assert_eq!(answer["summary"], json!(scratch.report(&app).summary));

        let answer = request(
            &mut app,
            json!({"op": "pid_check", "scope": "pbs", "pbs": "U-100"}),
        );
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["scope"],
            json!({"kind": "pbs", "drawing_id": null, "pbs": "U-100", "cross_drawing": true}),
            "{answer}"
        );
        assert_eq!(drawings_hit(&answer, "DR-102"), ["PID-1101"]);
        assert!(
            results_of(&answer)
                .iter()
                .all(|result| result["object"]["drawing_id"] == "PID-1101"),
            "{answer}"
        );
        assert_eq!(scratch.report(&app).scope.pbs.as_deref(), Some("U-100"));
        assert_eq!(undo_steps(&app), steps);
    }

    /// 编辑器里不等后台：扫着的时候再发 `pid_check` 还是 `scanning`，也不再起一次扫描；读完再发
    /// 才是结果（§5.3）。
    #[test]
    fn pid_check_op_answers_scanning_until_the_project_is_read() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ask = json!({"protocol": 1, "op": "pid_check", "scope": "project"});
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
        assert_eq!(drawings_hit(&answer, "DR-102"), ["PID-1101", "PID-1102"]);
    }

    /// `scope: drawing` 带 `cross_drawing: true`（§4.2）：扫项目、项目级规则也跑，只报当前图上的
    /// 对象；结果记在标签页上，可以照常豁免。当前图不在项目的图纸里（项目只认目录里的 `.dwg`）
    /// 时查不了跨图，同名的 `.dwg` 也不能顶替它：`INDEX_NO_PROJECT_DIR`。
    #[test]
    fn pid_check_op_cross_drawing_runs_the_project_rules_for_this_drawing_only() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let ask = json!({"op": "pid_check", "cross_drawing": true});
        let answer = request(&mut app, ask.clone());
        assert_eq!(answer["status"], "scanning", "{answer}");
        let answer = checked(&mut app, &ask);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["scope"],
            json!({"kind": "drawing", "drawing_id": "PID-1101", "pbs": null, "cross_drawing": true}),
            "{answer}"
        );
        assert_eq!(drawings_hit(&answer, "DR-102"), ["PID-1101"]);
        assert!(
            results_of(&answer)
                .iter()
                .all(|result| result["object"]["drawing_id"] == "PID-1101"),
            "{answer}"
        );
        assert!(kept(&app).report.scope.cross_drawing);
        assert_eq!(kept(&app).drawings, ["PID-1101"]);
        assert!(
            app.pid_checks.projects.is_empty(),
            "查当前图的不按项目目录记"
        );

        let key = key_of(&kept(&app).report, "DR-102", "PID-1101");
        let wrote = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "DR-102", "object_key": key, "reason": "同一台泵画在两张图上"}),
        );
        assert_eq!(wrote["result"], json!({"waivers": 1}), "{wrote}");

        let i = app.active_tab;
        let dxf = scratch.path("PID-1101.dxf");
        app.tabs[i].scene.document.source_path = Some(dxf.to_string_lossy().into_owned());
        let answer = request(&mut app, ask);
        assert_eq!(answer["code"], INDEX_NO_PROJECT_DIR, "{answer}");
    }

    /// `include_waived: false` 只是不列已豁免的结果（§5.3），计数照旧；默认列出来，带 `waived`
    /// 和豁免的理由。
    #[test]
    fn pid_check_op_can_leave_out_the_waived_results() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let answer = request(&mut app, json!({"op": "pid_check"}));
        assert_eq!(answer["ok"], true, "{answer}");
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let wrote = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "DR-014", "object_key": key, "reason": "放空管"}),
        );
        assert_eq!(wrote["result"], json!({"waivers": 1}), "{wrote}");

        let all = request(&mut app, json!({"op": "pid_check"}));
        let listed = results_of(&all);
        let waived: Vec<&Value> = listed
            .iter()
            .copied()
            .filter(|result| result["waived"] == true)
            .collect();
        assert!(!waived.is_empty(), "{all}");
        assert!(
            waived.iter().all(|result| result["rule_id"] == "DR-014"
                && result["object_key"] == key.as_str()
                && result["waiver"]["reason"] == "放空管"),
            "{all}"
        );

        let open = request(
            &mut app,
            json!({"op": "pid_check", "include_waived": false}),
        );
        assert_eq!(open["ok"], true, "{open}");
        assert_eq!(open["summary"], all["summary"]);
        assert_eq!(open["summary"]["已豁免"], waived.len(), "{open}");
        let left = results_of(&open);
        assert!(
            left.iter().all(|result| result["waived"] == false),
            "{open}"
        );
        assert_eq!(left.len() + waived.len(), listed.len());
    }

    /// `pid_check` 查不了时说为什么，连同错误码：参数不对是 `invalid_request`；图没保存过，查项目、
    /// 查节点、跨图都是 `INDEX_NO_PROJECT_DIR`（只查当前图照样行）；没配置主数据、节点号不在 PBS 表里
    /// 是 `PBS_NODE_UNKNOWN`；项目包有错时是那条错误的码（§3.8），哪个范围都不查。
    #[test]
    fn pid_check_op_says_why_it_cannot_check() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        for (ask, code) in [
            (json!({"scope": "sheet"}), "invalid_request"),
            (json!({"scope": 1}), "invalid_request"),
            (json!({"scope": "pbs"}), "invalid_request"),
            (json!({"scope": "pbs", "pbs": " "}), "invalid_request"),
            (
                json!({"scope": "project", "pbs": "U-100"}),
                "invalid_request",
            ),
            (json!({"cross_drawing": "yes"}), "invalid_request"),
            (json!({"include_waived": 0}), "invalid_request"),
            (json!({"scope": "project"}), INDEX_NO_PROJECT_DIR),
            (
                json!({"scope": "PBS", "pbs": "U-100"}),
                INDEX_NO_PROJECT_DIR,
            ),
            (json!({"cross_drawing": true}), INDEX_NO_PROJECT_DIR),
        ] {
            let mut req = ask.clone();
            req["op"] = json!("pid_check");
            let answer = request(&mut app, req);
            assert_eq!(answer["ok"], false, "{ask}: {answer}");
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }
        let answer = request(&mut app, json!({"op": "pid_check"}));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["scope"]["drawing_id"], "", "{answer}");

        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let pbs = json!({"op": "pid_check", "scope": "pbs", "pbs": "U-100"});
        let answer = request(&mut app, pbs.clone());
        assert_eq!(answer["code"], PBS_NODE_UNKNOWN, "没配置主数据：{answer}");
        write_project_file(&scratch);
        let answer = request(
            &mut app,
            json!({"op": "pid_check", "scope": "pbs", "pbs": "U-999"}),
        );
        assert_eq!(answer["code"], PBS_NODE_UNKNOWN, "{answer}");

        for (pack, code) in broken_packs() {
            fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("项目包");
            for ask in [
                json!({"op": "pid_check"}),
                json!({"op": "pid_check", "scope": "project"}),
                pbs.clone(),
                json!({"op": "pid_check", "cross_drawing": true}),
            ] {
                let answer = request(&mut app, ask.clone());
                assert_eq!(answer["code"], code, "{ask}: {answer}");
            }
        }
    }

    /// `pid_check_rules`（§5.3）：生效规则的版本、摘要值、项目包的位置、保存门禁、规则集和规则，
    /// 每条规则照规则包文件的写法（§3.1）。没有项目包时就是内置包；有就叠上，版本是
    /// `<内置版本>+<项目包版本>`；项目包有错时是它的错误码。只读会话里也能看。
    #[test]
    fn pid_check_rules_op_lists_the_rules_in_effect() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let builtin = builtin_rules::builtin();
        let answer = app.automation_op(r#"{"protocol":1,"op":"pid_check_rules"}"#);
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(
            answer["rule_set_version"],
            builtin.version.as_str(),
            "{answer}"
        );
        assert_eq!(
            answer["rule_set_digest"],
            builtin_rules::rule_set_digest(&builtin).as_str()
        );
        assert_eq!(answer["project_pack"], Value::Null, "{answer}");
        assert_eq!(answer["save_gate"], builtin.save_gate.name(), "{answer}");
        assert_eq!(
            answer["sets"],
            json!([
                { "name": "基础规则", "enabled": true },
                { "name": "工程扩展", "enabled": true }
            ])
        );
        let rules = answer["rules"].as_array().expect("rules");
        assert_eq!(rules.len(), builtin.rules.len());
        let first = &rules[0];
        for (key, value) in [
            ("id", json!("DR-001")),
            ("set", json!("基础规则")),
            ("category", json!("设计类")),
            ("target", json!("管线")),
            ("severity", json!("错误")),
            ("expr", json!("管道等级 = NULL")),
            ("scope", json!("项目")),
            ("scope_ref", Value::Null),
            ("enabled", json!(true)),
            ("source", json!("内置")),
            ("params", json!({})),
        ] {
            assert_eq!(first[key], value, "{key}: {first}");
        }
        assert_eq!(
            first.as_object().map(serde_json::Map::len),
            Some(14),
            "{first}"
        );

        let pack_path = scratch.path(PROJECT_PACK_FILE);
        fs::write(&pack_path, project_pack().to_string()).expect("项目包");
        app.collab.readonly = true;
        let answer = request(&mut app, json!({"op": "pid_check_rules"}));
        assert_eq!(answer["ok"], true, "只读会话里也能看：{answer}");
        let loaded = builtin_rules::load(Some(&scratch.0)).expect("规则");
        assert_eq!(
            answer["rule_set_version"],
            format!("{}+P2", builtin.version),
            "{answer}"
        );
        assert_eq!(
            answer["rule_set_digest"],
            builtin_rules::rule_set_digest(&loaded).as_str()
        );
        assert_eq!(
            answer["project_pack"],
            pack_path.display().to_string(),
            "{answer}"
        );
        assert_eq!(answer["save_gate"], "block", "{answer}");
        let rules = answer["rules"].as_array().expect("rules");
        assert_eq!(rules.len(), builtin.rules.len() + 1);
        let rule = |id: &str| rules.iter().find(|rule| rule["id"] == id).cloned();
        assert_eq!(
            rule("DR-014").map(|rule| rule["enabled"].clone()),
            Some(json!(false))
        );
        assert_eq!(
            rule("DR-201").map(|rule| rule["source"].clone()),
            Some(json!("新建"))
        );

        for (pack, code) in broken_packs() {
            fs::write(&pack_path, pack.to_string()).expect("项目包");
            let answer = request(&mut app, json!({"op": "pid_check_rules"}));
            assert_eq!(answer["code"], code, "{answer}");
        }
    }

    /// `pid_check_rules_set`（§5.3）整个换掉项目包：先照读规则包的规矩校验，全过才写，应答新的规则
    /// 版本和摘要值，下一次检查起生效，不改图。校验不过是那条错误的码（§3.8），文件不动；没给
    /// `pack` 是 `invalid_request`；图没保存过没有放项目包的目录，是 `INDEX_NO_PROJECT_DIR`。
    #[test]
    fn pid_check_rules_set_op_replaces_the_pack_only_when_it_is_valid() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let pack_path = scratch.path(PROJECT_PACK_FILE);
        let steps = undo_steps(&app);
        let pack = project_pack();
        let answer = request(&mut app, json!({"op": "pid_check_rules_set", "pack": pack}));
        assert_eq!(answer["ok"], true, "{answer}");
        let loaded = builtin_rules::load(Some(&scratch.0)).expect("规则");
        assert_eq!(
            loaded.version,
            format!("{}+P2", builtin_rules::builtin().version)
        );
        assert_eq!(
            answer["result"],
            json!({
                "rule_set_version": loaded.version,
                "rule_set_digest": builtin_rules::rule_set_digest(&loaded),
            }),
            "{answer}"
        );
        let written: Value =
            serde_json::from_str(&fs::read_to_string(&pack_path).expect("项目包")).expect("JSON");
        assert_eq!(written, pack);
        assert_eq!(undo_steps(&app), steps);
        assert!(!fires(&mut app, "DR-014"), "下一次检查起生效");

        let text = fs::read_to_string(&pack_path).expect("项目包");
        let mut repeated = project_pack();
        let copy = repeated["rules"][1].clone();
        if let Some(rules) = repeated["rules"].as_array_mut() {
            rules.push(copy);
        }
        let [first, second, third] = broken_packs();
        for (pack, code) in [
            first,
            second,
            third,
            (repeated, "RULE_ID_DUPLICATED"),
            (json!("不是对象"), "RULESET_SCHEMA_ERROR"),
        ] {
            let answer = request(&mut app, json!({"op": "pid_check_rules_set", "pack": pack}));
            assert_eq!(answer["code"], code, "{answer}");
        }
        let answer = request(&mut app, json!({"op": "pid_check_rules_set"}));
        assert_eq!(answer["code"], "invalid_request", "{answer}");
        assert_eq!(fs::read_to_string(&pack_path).expect("项目包"), text);

        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = None;
        let answer = request(
            &mut app,
            json!({"op": "pid_check_rules_set", "pack": project_pack()}),
        );
        assert_eq!(answer["code"], INDEX_NO_PROJECT_DIR, "{answer}");
    }

    /// `pid_check_waive`、`pid_check_unwaive`（§5.3）与命令一样改图里的豁免（§3.7）：各是一个撤销步，
    /// 留着的结果就地改，应答是图上现在有几条豁免。规则号小写也认，理由去掉首尾空白；同一条
    /// 再豁免是换理由。规则号不在生效的规则里是 `RULE_NOT_FOUND`；还没查过、（规则号, 对象键）不在
    /// 最近一次结果里、要取消的豁免图里没有是 `WAIVER_TARGET_NOT_FOUND`；理由空着、太长各有各的码；
    /// 缺参数是 `invalid_request`。这些都什么也不写。
    #[test]
    fn pid_check_waive_and_unwaive_ops_change_the_waivers_on_the_drawing() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let early = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "DR-014", "object_key": "x", "reason": "r"}),
        );
        assert_eq!(early["code"], WAIVER_TARGET_NOT_FOUND, "还没查过：{early}");
        let answer = request(&mut app, json!({"op": "pid_check"}));
        assert_eq!(answer["ok"], true, "{answer}");
        let serial = kept(&app).serial;
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let steps = undo_steps(&app);

        let wrote = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "dr-014", "object_key": key, "reason": " 放空管 "}),
        );
        assert_eq!(wrote["ok"], true, "{wrote}");
        assert_eq!(wrote["result"], json!({"waivers": 1}), "{wrote}");
        assert_eq!(undo_steps(&app), steps + 1);
        let now = recorded(&app);
        assert_eq!(
            (
                now[0].rule_id.as_str(),
                now[0].object_key.as_str(),
                now[0].reason.as_str()
            ),
            ("DR-014", key.as_str(), "放空管")
        );
        assert_eq!(kept(&app).serial, serial, "不重新检查");
        assert!(kept(&app)
            .report
            .results
            .iter()
            .any(|result| result.rule_id == "DR-014" && result.object_key == key && result.waived));

        let again = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "DR-014", "object_key": key, "reason": "端部通大气"}),
        );
        assert_eq!(again["result"], json!({"waivers": 1}), "{again}");
        assert_eq!(recorded(&app)[0].reason, "端部通大气");

        let steps = undo_steps(&app);
        for (ask, code) in [
            (
                json!({"rule_id": "DR-999", "object_key": key, "reason": "r"}),
                RULE_NOT_FOUND,
            ),
            (
                json!({"rule_id": "DR-014", "object_key": "没有这个", "reason": "r"}),
                WAIVER_TARGET_NOT_FOUND,
            ),
            (
                json!({"rule_id": "DR-014", "object_key": key, "reason": "  "}),
                "WAIVER_REASON_REQUIRED",
            ),
            (
                json!({"rule_id": "DR-014", "object_key": key, "reason": "长".repeat(201)}),
                "WAIVER_REASON_TOO_LONG",
            ),
            (
                json!({"rule_id": "DR-014", "object_key": key}),
                "invalid_request",
            ),
            (json!({"object_key": key, "reason": "r"}), "invalid_request"),
            (
                json!({"rule_id": 14, "object_key": key, "reason": "r"}),
                "invalid_request",
            ),
        ] {
            let mut req = ask.clone();
            req["op"] = json!("pid_check_waive");
            let answer = request(&mut app, req);
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(recorded(&app)[0].reason, "端部通大气");

        let removed = request(
            &mut app,
            json!({"op": "pid_check_unwaive", "rule_id": "DR-014", "object_key": key}),
        );
        assert_eq!(removed["result"], json!({"waivers": 0}), "{removed}");
        assert!(recorded(&app).is_empty());
        assert_eq!(undo_steps(&app), steps + 1);
        assert!(kept(&app)
            .report
            .results
            .iter()
            .all(|result| !result.waived));
        for (ask, code) in [
            (
                json!({"rule_id": "DR-014", "object_key": key}),
                WAIVER_TARGET_NOT_FOUND,
            ),
            (json!({"rule_id": "DR-014"}), "invalid_request"),
        ] {
            let mut req = ask.clone();
            req["op"] = json!("pid_check_unwaive");
            let answer = request(&mut app, req);
            assert_eq!(answer["code"], code, "{ask}: {answer}");
        }
        assert_eq!(undo_steps(&app), steps + 1);
    }

    /// 只读会话里三个写操作都被拒（§5.4）：码同别的被拒的写，也报给服务器；项目包和图里的
    /// 豁免都不动，不留撤销步。检查和看规则照常。
    #[test]
    fn pid_check_writes_are_refused_in_a_readonly_session() {
        let scratch = Scratch::new();
        let mut app = project(&scratch);
        let answer = request(&mut app, json!({"op": "pid_check"}));
        assert_eq!(answer["ok"], true, "{answer}");
        let key = key_of(&kept(&app).report, "DR-014", "PID-1101");
        let wrote = request(
            &mut app,
            json!({"op": "pid_check_waive", "rule_id": "DR-014", "object_key": key, "reason": "放空管"}),
        );
        assert_eq!(wrote["ok"], true, "{wrote}");
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for (op, ask) in [
            ("pid_check_rules_set", json!({"pack": project_pack()})),
            (
                "pid_check_waive",
                json!({"rule_id": "DR-014", "object_key": key, "reason": "换个理由"}),
            ),
            (
                "pid_check_unwaive",
                json!({"rule_id": "DR-014", "object_key": key}),
            ),
        ] {
            let mut req = ask;
            req["op"] = json!(op);
            let refused = request(&mut app, req);
            assert_eq!(refused["ok"], false, "{op}: {refused}");
            assert_eq!(refused["code"], "COLLAB_READONLY", "{op}: {refused}");
            assert_eq!(refused["level"], "state", "{op}: {refused}");
            assert!(
                app.collab.attempts.iter().any(|(_, text)| text == op),
                "{op}: {:?}",
                app.collab.attempts
            );
        }
        assert_eq!(undo_steps(&app), steps);
        assert_eq!(recorded(&app).len(), 1);
        assert_eq!(recorded(&app)[0].reason, "放空管");
        assert!(!scratch.path(PROJECT_PACK_FILE).exists());
        for op in ["pid_check", "pid_check_rules"] {
            let answer = request(&mut app, json!({ "op": op }));
            assert_eq!(answer["ok"], true, "{op}: {answer}");
        }
    }
}

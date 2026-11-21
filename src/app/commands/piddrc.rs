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

use super::pidindex::{same_file, Scanned, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::io::linetypes::writer::write_atomic;
use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
use crate::pid::drc::{
    self, CheckInput, CheckReport, CheckResult, RuleSet, Scope, ScopeKind, Severity, Summary,
    Trigger, Waiver,
};
use crate::pid::master_data::{self, Resolved, PROJECT_FILE};
use crate::pid::project_index::DrawingSummary;
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
}

impl Checked {
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
    /// 记的）。
    fn checked(
        &mut self,
        report: CheckReport,
        drawings: Vec<String>,
        recorded: &[(String, Vec<Waiver>)],
    ) -> Checked {
        self.serial += 1;
        Checked {
            serial: self.serial,
            stale: stale_by_drawing(&report, &drawings, recorded),
            drawings,
            report,
        }
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

/// 扫完项目接着查的（`commands::pidindex` 的扫描排着的一件）：查哪些图，用的规则和主数据。
/// 规则和主数据在命令起头时取（§4.1 的先后），摘要的连接容差照这套规则。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Check {
    /// `None` 是全项目，否则是这个 PBS 节点及其下级。
    pbs: Option<String>,
    pub(super) rules: RuleSet,
    master: Resolved,
}

/// `PIDCHECK EXPORT` 写的：§3.6 的检查结果，带上应答外面那层的 `ok`。
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
        let document = &self.tabs[i].scene.document;
        let path = document.source_path.as_deref().map(Path::new);
        let report = drc::check_drawing(document, path, &rules, Trigger::OnDemand);
        let drawing = report.scope.drawing_id.clone().unwrap_or_default();
        let recorded = [(drawing.clone(), drc::waivers(document))];
        self.pidcheck_say("PIDCHECK", &report, None);
        self.tabs[i].pid_check = Some(self.pid_checks.checked(report, vec![drawing], &recorded));
    }

    /// 生效的规则：内置包，第 `i` 个标签页的图所在目录有项目包就叠上（§3.2）。读不出来就连同
    /// 错误码说出来，得 `None`：不退回去只用内置包（§4.1）。
    fn pidcheck_rule_set(&mut self, i: usize, verb: &str) -> Option<RuleSet> {
        let dir = crate::io::drawing_dir(&self.tabs[i].scene.document);
        match builtin_rules::load(dir) {
            Ok(rules) => Some(rules),
            Err(error) => {
                let pack = match dir {
                    Some(dir) => format!("项目包 {}", dir.join(PROJECT_PACK_FILE).display()),
                    None => "内置规则包".to_owned(),
                };
                self.command_line
                    .push_error_coded(error.code(), &format!("{verb}: {pack} 有错：{error}"));
                None
            }
        }
    }

    /// `PIDCHECK PROJECT`、`PIDCHECK PBS <节点号>`（§4.2）：取规则和主数据，查节点号在不在，再扫
    /// 第 `i` 个标签页的图所在的项目目录，扫完接着查（[`Self::pidcheck_scanned`]）。要从磁盘读的图
    /// 在后台读，这里先交差。
    fn pidcheck_start(&mut self, i: usize, pbs: Option<String>) -> Task<Message> {
        let verb = verb_of(pbs.as_deref());
        let Some(project_dir) =
            crate::io::drawing_dir(&self.tabs[i].scene.document).map(Path::to_path_buf)
        else {
            self.command_line.push_error_coded(
                INDEX_NO_PROJECT_DIR,
                &format!(
                    "{verb}: {INDEX_NO_PROJECT_DIR}: 当前图没保存过，不属于哪个项目；存盘后再查。"
                ),
            );
            return Task::none();
        };
        let Some(rules) = self.pidcheck_rule_set(i, verb) else {
            return Task::none();
        };
        let master = master_data::resolve(&self.tabs[i].scene.document, Some(&project_dir));
        if let Some(node) = pbs.as_deref() {
            if let Err(why) = pbs_known(&master, node) {
                self.command_line.push_error_coded(
                    PBS_NODE_UNKNOWN,
                    &format!("{verb}: {PBS_NODE_UNKNOWN}: {why}。"),
                );
                return Task::none();
            }
        }
        self.pidindex_check(i, Check { pbs, rules, master })
    }

    /// 扫完项目接着查（[`check_project`]），说一句汇总、列前 10 条，结果按项目目录记下。
    pub(super) fn pidcheck_scanned(&mut self, project_dir: &Path, scanned: &Scanned, check: Check) {
        let verb = verb_of(check.pbs.as_deref());
        let (report, drawings) = check_project(scanned, check);
        self.pidcheck_say(verb, &report, Some(drawings.len()));
        let checked = self.pid_checks.checked(report, drawings, &scanned.waivers);
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

    /// `PIDCHECK EXPORT <文件>`：把最近一次的检查结果（[`Self::pidcheck_latest`]）写成
    /// §3.6 的 JSON，带 `ok`。文件名不是完整路径时从图纸所在目录算起，没存过的图从当前目录
    /// 算起。只写 `.json`，不写项目文件；已有的覆盖。
    fn pidcheck_export(&mut self, i: usize, rest: &str) {
        let name = unquoted(rest);
        if name.is_empty() {
            self.command_line
                .push_error("PIDCHECK EXPORT: 写成 PIDCHECK EXPORT <文件>。");
            return;
        }
        let path = match crate::io::drawing_dir(&self.tabs[i].scene.document) {
            Some(dir) if Path::new(name).is_relative() => dir.join(name),
            _ => PathBuf::from(name),
        };
        let json = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        if !json {
            self.command_line.push_error(&format!(
                "PIDCHECK EXPORT: 只写 .json 文件，{} 不是。",
                path.display()
            ));
            return;
        }
        if path
            .file_name()
            .is_some_and(|file| file.eq_ignore_ascii_case(PROJECT_FILE))
        {
            self.command_line.push_error(&format!(
                "PIDCHECK EXPORT: {} 是项目文件，不拿检查结果盖掉它。",
                path.display()
            ));
            return;
        }
        let Some(checked) = self.pidcheck_latest(i) else {
            self.command_line.push_error(
                "PIDCHECK EXPORT: 还没有检查结果；先用 PIDCHECK、PIDCHECK PROJECT 或 PIDCHECK PBS <节点号> 查一次。",
            );
            return;
        };
        let report = &checked.report;
        let counts = report.summary;
        let scope = scope_said(&report.scope);
        let written = serde_json::to_string_pretty(&Exported { ok: true, report })
            .map_err(|error| error.to_string())
            .and_then(|text| std::fs::write(&path, text + "\n").map_err(|error| error.to_string()));
        match written {
            Ok(()) => self.command_line.push_output(&format!(
                "PIDCHECK EXPORT: 已把{scope}的检查结果（错误 {} 条，警告 {} 条，提示 {} 条）写到 {}。",
                counts.errors,
                counts.warnings,
                counts.infos,
                path.display()
            )),
            Err(error) => self.command_line.push_error(&format!(
                "PIDCHECK EXPORT: 写不了 {}：{error}",
                path.display()
            )),
        }
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
    /// 里的一条。规则号要在生效的规则里；（规则号, 对象键）要在查过这张图的最近一份结果里、落在
    /// 这张图上；理由照 [`Waiver::new`] 查。写进图是一个撤销步，这张图上同一（规则号, 对象键）已有
    /// 豁免就换成这条。写完就地改留着的结果，不重新检查。
    fn pidcheck_waive(&mut self, i: usize, rule_id: &str, key: &str, reason: &str) {
        const VERB: &str = "PIDCHECK WAIVE";
        let Some(rules) = self.pidcheck_rule_set(i, VERB) else {
            return;
        };
        if rules.rule(rule_id).is_none() {
            self.command_line.push_error_coded(
                RULE_NOT_FOUND,
                &format!("{VERB}: {RULE_NOT_FOUND}: 生效的规则里没有 {rule_id}。"),
            );
            return;
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
            self.command_line.push_error_coded(
                WAIVER_TARGET_NOT_FOUND,
                &format!("{VERB}: {WAIVER_TARGET_NOT_FOUND}: {why}。"),
            );
            return;
        }
        let by = self.waiver_by();
        let waiver = match Waiver::new(rule_id, key, unquoted(reason), &by, unix_now()) {
            Ok(waiver) => waiver,
            Err(error) => {
                self.command_line
                    .push_error_coded(error.code(), &format!("{VERB}: {error}"));
                return;
            }
        };
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
        let counts = self.pidcheck_counts(i, &drawing);
        let done = if replaced {
            format!("{rule_id} 对 {key} 原来就豁免着，理由换成了这次写的")
        } else {
            format!("已豁免 {rule_id} 对 {key} 的结果")
        };
        self.command_line
            .push_output(&format!("{VERB}: {done}；{counts}。"));
    }

    /// `PIDCHECK UNWAIVE <规则号> <对象键>`（§3.7）：去掉第 `i` 个标签页的图上这条豁免，一个撤销步。
    /// 对不上结果的老豁免、规则包里已经没有的规则的豁免也去得掉，所以不查规则号、不要检查结果；
    /// 图里没有这条是 `WAIVER_TARGET_NOT_FOUND`。写完就地改留着的结果。
    fn pidcheck_unwaive(&mut self, i: usize, rule_id: &str, key: &str) {
        const VERB: &str = "PIDCHECK UNWAIVE";
        let mut waivers = drc::waivers(&self.tabs[i].scene.document);
        let before = waivers.len();
        waivers.retain(|waiver| waiver.rule_id != rule_id || waiver.object_key != key);
        if waivers.len() == before {
            self.command_line.push_error_coded(
                WAIVER_TARGET_NOT_FOUND,
                &format!(
                    "{VERB}: {WAIVER_TARGET_NOT_FOUND}: 当前图上没有 {rule_id} 对 {key} 的豁免。"
                ),
            );
            return;
        }
        self.pidcheck_write_waivers(i, &waivers);
        let counts = self.pidcheck_counts(i, &self.pidcheck_drawing_id(i));
        self.command_line.push_output(&format!(
            "{VERB}: 已取消 {rule_id} 对 {key} 的豁免；{counts}。"
        ));
    }

    /// 把第 `i` 个标签页的图上的豁免整个换成 `waivers`：先记撤销快照（§3.7），写完就地改留着的结果。
    fn pidcheck_write_waivers(&mut self, i: usize, waivers: &[Waiver]) {
        self.push_undo_snapshot(i, "PIDCHECK");
        drc::set_waivers(&mut self.tabs[i].scene.document, waivers);
        self.tabs[i].dirty = true;
        self.pidcheck_follow_waivers(i);
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
    fn pidcheck_latest(&self, i: usize) -> Option<&Checked> {
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
    fn pidcheck_drawing_id(&self, i: usize) -> String {
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

    /// 豁免记在谁名下（§3.7）：连着协同时是协同的用户 ID，否则是操作系统的账户名。
    fn waiver_by(&self) -> String {
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

/// 在扫好的项目上查（§4.1、§4.2），得到结果和查了的图（图号）。查全项目时范围是读得出来的
/// 全部图；查节点时是项目文件里归在这个节点及其下级的图，其余的作对端（`others`）。项目级
/// 规则都跑。不碰命令行和程序状态。
fn check_project(scanned: &Scanned, check: Check) -> (CheckReport, Vec<String>) {
    let Check { pbs, rules, master } = check;
    let nodes: Option<HashSet<&str>> = pbs.as_deref().map(|node| match &master {
        Resolved::Configured { data, .. } => data.pbs_subtree(node).into_iter().collect(),
        Resolved::Unreadable { .. } | Resolved::NotConfigured => HashSet::new(),
    });
    let (drawings, others): (Vec<DrawingSummary>, Vec<DrawingSummary>) = scanned
        .summaries
        .iter()
        .cloned()
        .partition(|summary| match &nodes {
            None => true,
            Some(nodes) => summary
                .pbs
                .as_deref()
                .is_some_and(|pbs| nodes.contains(pbs)),
        });
    let types = TypeLib::embedded();
    let kind = if pbs.is_some() {
        ScopeKind::Pbs
    } else {
        ScopeKind::Project
    };
    let report = drc::check(&CheckInput {
        rules: &rules,
        master: &master,
        types: &types,
        drawings: &drawings,
        index: Some(&scanned.index),
        others: &others,
        waivers: &scanned.waivers,
        trigger: Trigger::OnDemand,
        scope: Scope {
            kind,
            drawing_id: None,
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

/// 查项目、查节点时说话打头的命令。
fn verb_of(pbs: Option<&str>) -> &'static str {
    if pbs.is_some() {
        "PIDCHECK PBS"
    } else {
        "PIDCHECK PROJECT"
    }
}

/// 节点号 `node` 在不在主数据的 PBS 表里（§3.8 的 `PBS_NODE_UNKNOWN`）；不在时说为什么。
fn pbs_known(master: &Resolved, node: &str) -> Result<(), String> {
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

/// 列在命令行上的一条结果：严重度、规则号、（查项目时）图号、消息，括号里是对象键——豁免
/// 时用它。
fn listed(result: &CheckResult, with_drawing: bool) -> String {
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
}

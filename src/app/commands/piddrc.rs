//! PIDCHECK：按规则检查 P&ID 图纸（`SPEC-pid-drc.md` §4、§5.2）。
//!
//! ```text
//! PIDCHECK                 查当前图：一句汇总，再列前 10 条结果
//! PIDCHECK PROJECT         查当前图所在项目目录里的全部图，项目级规则也跑
//! PIDCHECK PBS <节点号>    查项目文件里归在这个 PBS 节点及其下级的图，项目级规则也跑
//! PIDCHECK RULES           列出生效的规则：号、名称、严重度、启停、来源
//! PIDCHECK EXPORT <文件>   把最近一次的检查结果写成 JSON 文件（§3.6）
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
//! `EXPORT` 写当前标签页上的那份和它项目目录的那份里较新的一份。检查不改图，几个子命令在只读
//! 会话里都能用。

use super::pidindex::{same_file, Scanned, INDEX_NO_PROJECT_DIR};
use super::*;
use crate::pid::drc::builtin_rules::{self, PROJECT_PACK_FILE};
use crate::pid::drc::{
    self, CheckInput, CheckReport, CheckResult, RuleSet, Scope, ScopeKind, Trigger,
};
use crate::pid::master_data::{self, Resolved, PROJECT_FILE};
use crate::pid::project_index::DrawingSummary;
use crate::pid::type_lib::TypeLib;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 按节点检查时节点号不在主数据的 PBS 表里（§3.8）。
pub(in crate::app) const PBS_NODE_UNKNOWN: &str = "PBS_NODE_UNKNOWN";

/// 命令行上列出的结果条数（§5.2）。
const LISTED: usize = 10;

/// 本期没有对象、不会出结果的内置规则（决定 D8），`PIDCHECK RULES` 里标出来。
const NO_OBJECTS_YET: [&str; 3] = ["DR-016", "DR-020", "DR-109"];

/// 一次检查的结果，和它是程序里第几次检查：两份结果比新旧用它。
pub(in crate::app) struct Checked {
    pub(in crate::app) serial: u64,
    pub(in crate::app) report: CheckReport,
}

/// 查项目、查 PBS 节点的结果，按项目目录各留最近一份（§4.1）；查当前图的在标签页上。检查的
/// 序号也从这里发，两处的结果才比得出新旧。
#[derive(Default)]
pub(in crate::app) struct PidChecks {
    serial: u64,
    projects: HashMap<PathBuf, Checked>,
}

impl PidChecks {
    /// 给新的一份结果编上序号。
    fn checked(&mut self, report: CheckReport) -> Checked {
        self.serial += 1;
        Checked {
            serial: self.serial,
            report,
        }
    }
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
            _ => self.command_line.push_error(&format!(
                "PIDCHECK: 不认识 {option}；写 PROJECT、PBS <节点号>、RULES 或 EXPORT <文件>，或者只写 PIDCHECK。"
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
        self.pidcheck_say("PIDCHECK", &report, None);
        self.tabs[i].pid_check = Some(self.pid_checks.checked(report));
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
        self.pidcheck_say(verb, &report, Some(drawings));
        let checked = self.pid_checks.checked(report);
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

    /// 第 `i` 个标签页最近一次的检查结果（`SPEC-pid-verify-deliver-panel.md` §3.1）：标签页上查这张
    /// 图的，和它项目目录查项目、查节点的，两份都有取较新的。
    fn pidcheck_latest(&self, i: usize) -> Option<&Checked> {
        let tab = self.tabs[i].pid_check.as_ref();
        let project = crate::io::drawing_dir(&self.tabs[i].scene.document)
            .and_then(|dir| self.pid_checks.projects.get(&same_file(dir)));
        tab.into_iter()
            .chain(project)
            .max_by_key(|checked| checked.serial)
    }
}

/// 在扫好的项目上查（§4.1、§4.2），得到结果和查了几张图。查全项目时范围是读得出来的全部图；
/// 查节点时是项目文件里归在这个节点及其下级的图，其余的作对端（`others`）。项目级规则都跑。
/// 不碰命令行和程序状态。
fn check_project(scanned: &Scanned, check: Check) -> (CheckReport, usize) {
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
    (report, drawings.len())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::drc::Waiver;
    use crate::pid::master_data::{MasterData, PbsNode};
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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

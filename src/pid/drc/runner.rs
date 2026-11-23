//! 检查器（`SPEC-pid-drc.md` §4.1、§5.1）：对每条启用的规则，从摘要里选出作用对象，逐个
//! 求值，为真即命中，产出 §3.6 的检查结果。只吃摘要、索引、主数据和规则，不碰文件和图纸。

use std::collections::HashSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::pid::master_data;
use crate::pid::project_index::{DrawingSummary, LinkStatus, ProjectIndex};
use crate::pid::type_lib::TypeLib;

use super::builtin_rules::rule_set_digest;
use super::dsl::{Expr, Operand, Path};
use super::facts::{
    reads_index, reads_master, subjects, Context, Project, Sheet, Subject, SubjectFacts, LOCATED,
    MISSING_COUNT,
};
use super::model::{
    Category, CheckReport, CheckResult, Notice, Rule, RuleScope, RuleSet, Scope, ScopeKind,
    Severity, Summary, Trigger, Waiver, REPORT_VERSION,
};

/// 消息和修复建议最多这么多字（§3.1）。
const TEXT_LIMIT: usize = 200;

/// 一次检查要的东西（§5.1）。
pub struct CheckInput<'a> {
    pub rules: &'a RuleSet,
    pub master: &'a master_data::Resolved,
    /// 按类型编码选对象时认下级类型。
    pub types: &'a TypeLib,
    /// 范围内的图。
    pub drawings: &'a [DrawingSummary],
    /// 有它才跑项目级规则。
    pub index: Option<&'a ProjectIndex>,
    /// 索引涉及的其余图，给 `对端线` 用。
    pub others: &'a [DrawingSummary],
    /// 图号 → 该图的豁免；不在 `drawings` 里的图不看。
    pub waivers: &'a [(String, Vec<Waiver>)],
    pub trigger: Trigger,
    /// 结果里的 `cross_drawing` 不照这里写的，按有没有 `index` 填。
    pub scope: Scope,
}

/// 按规则检查范围内的图（§4.1）。纯函数：只读输入，不碰文件和图纸。
pub fn check(input: &CheckInput) -> CheckReport {
    let started = Instant::now();
    let sheets: Vec<Sheet<'_>> = input.drawings.iter().map(Sheet::new).collect();
    let elsewhere: Vec<Sheet<'_>> = if input.index.is_some() {
        input.others.iter().map(Sheet::new).collect()
    } else {
        Vec::new()
    };
    let context = Context {
        rules: input.rules,
        master: input.master,
        types: input.types,
        project: input
            .index
            .map(|index| Project::new(index, &sheets, &elsewhere)),
    };
    let mut summary = Summary {
        objects: input
            .drawings
            .iter()
            .map(|drawing| drawing.objects.len())
            .sum(),
        ..Summary::default()
    };
    let configured = matches!(input.master, master_data::Resolved::Configured { .. });
    let mut without_master = 0;
    let mut without_index = 0;
    let mut unknown = Vec::new();
    let mut results = Vec::new();
    let mut ran = HashSet::new();
    for rule in &input.rules.rules {
        if !input.rules.is_enabled(rule) {
            continue;
        }
        if rule.category != Category::Design {
            summary.rules_skipped += 1;
            continue;
        }
        if !configured && uses_master(rule) {
            summary.rules_skipped += 1;
            without_master += 1;
            continue;
        }
        if input.index.is_none() && uses_index(rule) {
            summary.rules_skipped += 1;
            without_index += 1;
            continue;
        }
        summary.rules_run += 1;
        ran.insert(rule.id.as_str());
        results.extend(hits(rule, &sheets, &context));
        unknown.extend(scope_notice(rule, input));
    }
    results.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
            .then_with(|| a.object.drawing_id.cmp(&b.object.drawing_id))
            .then_with(|| a.object_key.cmp(&b.object_key))
    });
    let stale_waivers = apply_waivers(&mut results, input, &ran);
    for result in &results {
        if result.waived {
            summary.waived += 1;
            continue;
        }
        match result.severity {
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Info => summary.infos += 1,
        }
    }
    let mut notices: Vec<Notice> = master_notice(input.master, without_master)
        .into_iter()
        .chain(project_notice(input.scope.kind, without_index))
        .collect();
    if let Some(index) = input.index {
        notices.extend(index_notices(index, input.drawings));
    }
    notices.extend(unknown);
    CheckReport {
        version: REPORT_VERSION,
        rule_set_version: input.rules.version.clone(),
        rule_set_digest: rule_set_digest(input.rules),
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            }),
        trigger: input.trigger,
        scope: Scope {
            cross_drawing: input.index.is_some(),
            ..input.scope.clone()
        },
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        summary,
        blocked: summary.errors > 0,
        results,
        notices,
        stale_waivers,
    }
}

/// 把图里记着的豁免挂到对得上的结果上（§3.7），返回这次对不上的。图号、规则号、对象键都相同才算
/// 对上；一条豁免可以对上几条结果，一条结果挂先对上的那条。规则停用了或没跑的，它的豁免对不对得上
/// 下不了结论，不算对不上；规则包里已经没有的规则、跑了但作用域不含这张图的规则，它的豁免算。
fn apply_waivers(
    results: &mut [CheckResult],
    input: &CheckInput,
    ran: &HashSet<&str>,
) -> Vec<Waiver> {
    let mut stale = Vec::new();
    for (drawing_id, waivers) in input.waivers {
        if !input
            .drawings
            .iter()
            .any(|drawing| drawing.drawing_id == *drawing_id)
        {
            continue;
        }
        for waiver in waivers {
            let mut matched = false;
            for result in results.iter_mut().filter(|result| {
                result.object.drawing_id == *drawing_id
                    && result.rule_id == waiver.rule_id
                    && result.object_key == waiver.object_key
            }) {
                matched = true;
                if !result.waived {
                    result.waived = true;
                    result.waiver = Some(waiver.clone());
                }
            }
            let known = input
                .rules
                .rules
                .iter()
                .any(|rule| rule.id == waiver.rule_id);
            if !matched && (ran.contains(waiver.rule_id.as_str()) || !known) {
                stale.push(waiver.clone());
            }
        }
    }
    stale
}

/// 规则用不用主数据的表（[`reads_master`]）。主数据没配置或读不出来时这些规则下不了结论，不跑
/// （主数据规格 §3.5）；只用 `项目.图例标准` 的照跑。
fn uses_master(rule: &Rule) -> bool {
    let mut found = false;
    visit_paths(&rule.tree, &mut |path| {
        found |= reads_master(&path.segments)
    });
    found
}

/// 主数据没配置或读不出来、`skipped` 条用到主数据的规则因此没跑时，报告的 `notices[]` 里说一句。
fn master_notice(master: &master_data::Resolved, skipped: usize) -> Option<Notice> {
    if skipped == 0 {
        return None;
    }
    let (code, state) = match master {
        master_data::Resolved::Configured { .. } => return None,
        master_data::Resolved::NotConfigured => {
            ("MASTERDATA_NOT_CONFIGURED", "没有配置".to_owned())
        }
        master_data::Resolved::Unreadable { error, .. } => {
            ("MASTERDATA_UNREADABLE", format!("读不出来（{error}）"))
        }
    };
    Some(Notice {
        code: code.to_owned(),
        message: format!("项目主数据{state}，用到主数据的 {skipped} 条规则没有执行"),
    })
}

/// 规则是不是项目级的（[`reads_index`]）：没有项目索引时这些规则下不了结论，不跑（§3.4）。
fn uses_index(rule: &Rule) -> bool {
    let kind = rule.target.kind();
    let mut found = false;
    visit_paths(&rule.tree, &mut |path| {
        found |= reads_index(kind, &path.segments)
    });
    found
}

/// 没有项目索引、`skipped` 条项目级规则因此没跑时，`notices[]` 里说一句（§3.6）；只查当前图时
/// 说只查了当前图。
fn project_notice(kind: ScopeKind, skipped: usize) -> Option<Notice> {
    if skipped == 0 {
        return None;
    }
    let reason = if kind == ScopeKind::Drawing {
        "只检查了当前图"
    } else {
        "没有项目索引"
    };
    Some(Notice {
        code: "PROJECT_RULES_SKIPPED".to_owned(),
        message: format!("{reason}，{skipped} 条项目级规则没有执行"),
    })
}

/// 带着项目索引查时，跨图的结论可能不全的两种情形各说一句（§3.6）：项目里有图读不出来，查的图上
/// 有页间连接符分不出对端是哪一个。
fn index_notices(index: &ProjectIndex, drawings: &[DrawingSummary]) -> Vec<Notice> {
    let unreadable = index
        .drawings
        .iter()
        .filter(|drawing| drawing.error.is_some())
        .count();
    let ambiguous = index
        .unresolved
        .iter()
        .filter(|item| item.status == LinkStatus::Ambiguous)
        .filter(|item| {
            drawings
                .iter()
                .any(|drawing| drawing.drawing_id == item.drawing_id)
        })
        .count();
    let mut notices = Vec::new();
    if unreadable > 0 {
        notices.push(Notice {
            code: "INDEX_INCOMPLETE".to_owned(),
            message: format!("项目里有 {unreadable} 张图读不出来，跨图的结论不完整"),
        });
    }
    if ambiguous > 0 {
        notices.push(Notice {
            code: "CONNECTOR_AMBIGUOUS".to_owned(),
            message: format!("有 {ambiguous} 个页间连接符分不出对端是哪一个，跨图的结论不完整"),
        });
    }
    notices
}

/// 规则的作用域（§3.1 `scope_ref`）指的 PBS 节点、图纸或类型不在项目里时，`notices[]` 里说一句。
/// 主数据没配好时不知道有哪些节点，没有项目索引时不知道有哪些图，这时不说。
fn scope_notice(rule: &Rule, input: &CheckInput) -> Option<Notice> {
    let message = match &rule.scope {
        RuleScope::Project => return None,
        RuleScope::Pbs(node) => {
            let master_data::Resolved::Configured { data, .. } = input.master else {
                return None;
            };
            if !data.pbs_subtree(node).is_empty() {
                return None;
            }
            format!(
                "规则 {} 的作用域是 PBS 节点 {node}，项目主数据里没有这个节点",
                rule.id
            )
        }
        RuleScope::Drawing(id) => {
            let index = input.index?;
            if index
                .drawings
                .iter()
                .any(|drawing| drawing.drawing_id == *id)
            {
                return None;
            }
            format!("规则 {} 的作用域是图纸 {id}，项目里没有这张图", rule.id)
        }
        RuleScope::Type(code) => {
            if !input.types.ancestors(code).is_empty() {
                return None;
            }
            format!(
                "规则 {} 的作用域是类型 {code}，类型库里没有这个类型",
                rule.id
            )
        }
    };
    Some(Notice {
        code: "RULE_SCOPE_UNKNOWN".to_owned(),
        message,
    })
}

/// 规则的作用域管不管这张图：PBS 节点管挂在它和它下级节点上的图，主数据没配好时只认挂在它
/// 自己上的；图纸只管那一张；项目和类型不挑图。
fn covers_drawing(
    scope: &RuleScope,
    drawing: &DrawingSummary,
    master: &master_data::Resolved,
) -> bool {
    match scope {
        RuleScope::Project | RuleScope::Type(_) => true,
        RuleScope::Drawing(id) => drawing.drawing_id == *id,
        RuleScope::Pbs(node) => {
            let Some(pbs) = drawing.pbs.as_deref() else {
                return false;
            };
            match master {
                master_data::Resolved::Configured { data, .. } => {
                    data.pbs_subtree(node).contains(&pbs)
                }
                master_data::Resolved::NotConfigured | master_data::Resolved::Unreadable { .. } => {
                    pbs == node.as_str()
                }
            }
        }
    }
}

/// 规则的作用域管不管这个对象：类型管这个类型和它下级类型的对象，没有类型编码的（仪表回路、
/// 图纸）不管；别的作用域不挑对象。
fn covers_subject(scope: &RuleScope, subject: Subject<'_>, types: &TypeLib) -> bool {
    let RuleScope::Type(code) = scope else {
        return true;
    };
    subject
        .type_code()
        .is_some_and(|own| own == code.as_str() || types.ancestors(own).contains(code))
}

/// 一条规则在范围内各图上的命中：表达式为真才算，不知道不算；规则的作用域管不到的图和对象不查。
fn hits(rule: &Rule, sheets: &[Sheet<'_>], context: &Context<'_>) -> Vec<CheckResult> {
    let fields = fields(rule);
    let located = located(rule);
    let missing = rule.target.kind().has_fact(MISSING_COUNT)
        && fields.iter().any(|name| name == MISSING_COUNT);
    let mut found = Vec::new();
    for sheet in sheets {
        if !covers_drawing(&rule.scope, sheet.drawing(), context.master) {
            continue;
        }
        for subject in subjects(sheet, &rule.target, context.types) {
            if !covers_subject(&rule.scope, subject, context.types) {
                continue;
            }
            let facts = SubjectFacts::new(subject, rule, sheet, context);
            if !rule.tree.eval(&facts).is_true() {
                continue;
            }
            let message = match &rule.message {
                Some(template) => render(template, &facts),
                None => limit(format!("{}：{}", rule.name, facts.location())),
            };
            let fix = rule
                .fix
                .as_deref()
                .map(|template| render(template, &facts))
                .unwrap_or_default();
            found.push(CheckResult {
                rule_id: rule.id.clone(),
                rule_name: rule.name.clone(),
                severity: rule.severity,
                object: facts.object_ref(),
                object_key: facts.object_key(),
                message,
                fix,
                points: facts.points(&located),
                fields: if missing {
                    with_missing(&fields, &facts)
                } else {
                    fields.clone()
                },
                waived: false,
                waiver: None,
            });
        }
    }
    found
}

/// 把模板里的 `{名字}` 换成对象上的值，再截到 200 字。没有配对 `}` 的 `{` 照原样留着。
fn render(template: &str, facts: &SubjectFacts) -> String {
    let mut text = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        text.push_str(&rest[..open]);
        text.push_str(&facts.placeholder(&rest[open + 1..open + close]));
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);
    limit(text)
}

fn limit(mut text: String) -> String {
    if let Some((at, _)) = text.char_indices().nth(TEXT_LIMIT) {
        text.truncate(at);
    }
    text
}

/// 结果的 `fields`（§3.6）：表达式里出现的属性名，用到位号的再加 `位号`；派生事实、沿关系取的
/// 和根下的不算。`必填项缺失数` 先记下这个名字，出结果时换成缺的属性名（[`with_missing`]）。
/// 按出现的先后，不重复。
fn fields(rule: &Rule) -> Vec<String> {
    let kind = rule.target.kind();
    let mut names: Vec<String> = Vec::new();
    visit_paths(&rule.tree, &mut |path| {
        let name = match path.segments.as_slice() {
            [first, ..] if first == "位号" => first,
            [name] if name == MISSING_COUNT || !kind.has_fact(name) => name,
            _ => return,
        };
        if !names.contains(name) {
            names.push(name.clone());
        }
    });
    names
}

/// 结果的 `points` 要看的派生事实：表达式里单独用到、是作用对象的派生事实的 [`LOCATED`]，
/// 按出现的先后，不重复。沿关系取的不算。
fn located(rule: &Rule) -> Vec<&'static str> {
    let kind = rule.target.kind();
    let mut names: Vec<&'static str> = Vec::new();
    visit_paths(&rule.tree, &mut |path| {
        let found = match path.segments.as_slice() {
            [name] => LOCATED.into_iter().find(|located| located == name),
            _ => None,
        };
        if let Some(found) = found.filter(|found| kind.has_fact(found) && !names.contains(found)) {
            names.push(found);
        }
    });
    names
}

/// 一条结果的 `fields`：`必填项缺失数` 换成这个对象缺的那些属性名，不重复。
fn with_missing(fields: &[String], facts: &SubjectFacts) -> Vec<String> {
    let mut names = Vec::new();
    for name in fields {
        let expanded = if name == MISSING_COUNT {
            facts.missing_fields()
        } else {
            vec![name.clone()]
        };
        for name in expanded {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// 表达式里的每个路径，按出现的先后。
fn visit_paths(expr: &Expr, visit: &mut impl FnMut(&Path)) {
    match expr {
        Expr::Or(terms) | Expr::And(terms) => {
            for term in terms {
                visit_paths(term, visit);
            }
        }
        Expr::Not(inner) => visit_paths(inner, visit),
        Expr::Compare(comparison) => {
            visit(&comparison.left);
            visit_operand(&comparison.right, visit);
        }
    }
}

fn visit_operand(operand: &Operand, visit: &mut impl FnMut(&Path)) {
    match operand {
        Operand::Path(path) => visit(path),
        Operand::List(items) => {
            for item in items {
                visit_operand(item, visit);
            }
        }
        Operand::Value(_) | Operand::Pattern(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use serde_json::{json, Value};

    use super::*;
    use crate::app::OpenCADStudio;
    use crate::pid::drc::builtin_rules::{builtin, rule_set_digest};
    use crate::pid::drc::dsl::parse;
    use crate::pid::drc::facts::ANNOTATION_TYPE;
    use crate::pid::drc::model::{
        Category, CheckResult, Notice, ObjectRef, RuleScope, ScopeKind, Severity, Source, Target,
    };
    use crate::pid::project_index::{
        build_index, summarize, synthetic_drawing, FailedDrawing, Joint, ObjectKind, PeerKind,
        SignalEnd, SummaryConnector, SummaryMeta, SummaryObject, SummaryPort, SummarySegment,
        SummarySource, DEFAULT_TOLERANCE, SUMMARY_VERSION,
    };
    use crate::pid::symbols::{PortDir, SymbolLibrary};
    use crate::pid::tag_engine::TAG_STATUS_DUPLICATED;
    use crate::pid::topology::TYPE_SIGNAL_LINE;

    /// 本期不出结果的内置规则（规格 §3.4）：管口没有位号，`位号前缀` 不知道；符号库里没有带方向
    /// 的阀门，`流向相反` 不知道。它们命中的例子是要查的那种情形，本期也不命中。
    const SILENT: [&str; 2] = ["DR-020", "DR-109"];

    /// 命中的例子出两条的规则：两个对象互相违反（两段管段互相重叠）。
    const MUTUAL: [&str; 1] = ["DR-110"];

    fn object(
        kind: ObjectKind,
        type_code: &str,
        label: &str,
        tag: &str,
        props: &[(&str, &str)],
    ) -> SummaryObject {
        SummaryObject {
            kind,
            type_code: type_code.to_owned(),
            type_label: label.to_owned(),
            tag: tag.to_owned(),
            own_tag: kind.has_own_tag(),
            tag_source: "rule".to_owned(),
            tag_status: "ok".to_owned(),
            symbol: None,
            symbol_known: false,
            props: props
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            overrides: Vec::new(),
            owner: None,
            layer: "PID".to_owned(),
            at: [10.0, 20.0],
            bbox: [0.0, 0.0, 20.0, 40.0],
            handle: "2D4".to_owned(),
        }
    }

    fn line(tag: &str, props: &[(&str, &str)]) -> SummaryObject {
        object(ObjectKind::PipeLine, "PIPE_LINE", "管线", tag, props)
    }

    fn pump(tag: &str) -> SummaryObject {
        object(ObjectKind::Equipment, "PUMP", "泵", tag, &[])
    }

    fn instrument(tag: &str, props: &[(&str, &str)]) -> SummaryObject {
        object(ObjectKind::Instrument, "INSTRUMENT", "仪表", tag, props)
    }

    fn valve(tag: &str, props: &[(&str, &str)]) -> SummaryObject {
        object(ObjectKind::Valve, "VALVE_GATE", "闸阀", tag, props)
    }

    fn annotation(props: &[(&str, &str)]) -> SummaryObject {
        object(ObjectKind::Other, ANNOTATION_TYPE, "标注", "", props)
    }

    fn tank(tag: &str) -> SummaryObject {
        object(ObjectKind::Equipment, "TANK", "储罐", tag, &[])
    }

    fn pipe_run(tag: &str, line: &str, props: &[(&str, &str)]) -> SummaryObject {
        SummaryObject {
            owner: Some(line.to_owned()),
            ..object(ObjectKind::PipeRun, "PIPE_RUN", "管路", tag, props)
        }
    }

    fn signal(tag: &str, props: &[(&str, &str)]) -> SummaryObject {
        object(
            ObjectKind::SignalLine,
            TYPE_SIGNAL_LINE,
            "信号线",
            tag,
            props,
        )
    }

    /// 管路 `run` 上的对象：属主是这条管路。
    fn on_run(run: &str, base: SummaryObject) -> SummaryObject {
        SummaryObject {
            owner: Some(run.to_owned()),
            ..base
        }
    }

    /// 管路 `run` 上一个不单独编号的管件，记着管路的位号。
    fn fitting(run: &str, props: &[(&str, &str)]) -> SummaryObject {
        on_run(
            run,
            object(ObjectKind::Fitting, "FITTING", "管件", run, props),
        )
    }

    /// 放了符号块 `block` 的对象；`known` 是当前符号库里有没有这个块。
    fn with_symbol(base: SummaryObject, block: &str, known: bool) -> SummaryObject {
        SummaryObject {
            symbol: Some(block.to_owned()),
            symbol_known: known,
            ..base
        }
    }

    /// 设备 `type_code` `tag` 的 `count` 个管口。
    fn nozzles(type_code: &str, tag: &str, count: usize) -> Vec<SummaryPort> {
        (1..=count)
            .map(|n| SummaryPort {
                owner_kind: ObjectKind::Equipment,
                type_code: type_code.to_owned(),
                tag: tag.to_owned(),
                port: format!("N{n}"),
                direction: None,
                at: [0.0, 0.0],
                joined: false,
            })
            .collect()
    }

    /// `owner` 的符号的端口 `name`，在原点，不分流向。
    fn port_of(owner: &SummaryObject, name: &str, joined: bool) -> SummaryPort {
        SummaryPort {
            owner_kind: owner.kind,
            type_code: owner.type_code.clone(),
            tag: owner.tag.clone(),
            port: name.to_owned(),
            direction: None,
            at: [0.0, 0.0],
            joined,
        }
    }

    /// 接在 `owner` 的符号的端口 `name` 上。
    fn on_port(owner: &SummaryObject, name: &str) -> Option<Joint> {
        Some(Joint::Port {
            type_code: owner.type_code.clone(),
            tag: owner.tag.clone(),
            port: name.to_owned(),
        })
    }

    /// 信号线 `tag` 在 `at` 的一端，接着 `joint`。
    fn signal_end(tag: &str, at: [f64; 2], joint: Option<Joint>) -> SignalEnd {
        SignalEnd {
            tag: tag.to_owned(),
            at,
            joint,
        }
    }

    /// 管线 `line` 上从 `a` 到 `b` 的一段，两端都没接上；`run` 为空是管线自己的线。
    fn segment(line: &str, run: Option<&str>, a: [f64; 2], b: [f64; 2]) -> SummarySegment {
        SummarySegment {
            line: line.to_owned(),
            run: run.map(str::to_owned),
            a,
            b,
            length: (b[0] - a[0]).hypot(b[1] - a[1]),
            joint_a: None,
            joint_b: None,
            handle: "2E0".to_owned(),
        }
    }

    /// 接在管线 `L-1001` 的管路 `run` 的线段上（`run` 为空是管线自己的线）；`tee` 是接在中间。
    fn joined(run: Option<&str>, tee: bool) -> Option<Joint> {
        Some(Joint::Segment {
            line: "L-1001".to_owned(),
            run: run.map(str::to_owned),
            tee,
        })
    }

    /// 有这些对象和线段的图 `PID-1101`。
    fn piping(objects: Vec<SummaryObject>, segments: Vec<SummarySegment>) -> DrawingSummary {
        DrawingSummary {
            segments,
            ..drawing("PID-1101", objects)
        }
    }

    fn drawing(id: &str, objects: Vec<SummaryObject>) -> DrawingSummary {
        DrawingSummary {
            version: SUMMARY_VERSION,
            drawing_id: id.to_owned(),
            file: None,
            source: SummarySource::Open,
            fingerprint: None,
            pbs: None,
            length_unit: "mm".to_owned(),
            objects,
            segments: Vec::new(),
            signal_ends: Vec::new(),
            connectors: Vec::new(),
            ports: Vec::new(),
        }
    }

    /// 图 `id` 上的对象和页间连接符。
    fn with_connectors(
        id: &str,
        objects: Vec<SummaryObject>,
        connectors: Vec<SummaryConnector>,
    ) -> DrawingSummary {
        DrawingSummary {
            connectors,
            ..drawing(id, objects)
        }
    }

    /// 线 `line` 上的页间连接符，`kind` 如 `pipe_out`。像刚放上去的那样：对端什么都没记，
    /// `upstream` 是本侧线号。
    fn connector(kind: &str, line: &str, handle: &str) -> SummaryConnector {
        SummaryConnector {
            kind: kind.to_owned(),
            line: line.to_owned(),
            at: [300.0, 100.0],
            radius: 3.0,
            peer_kind: PeerKind::None,
            peer_drawing: None,
            upstream: Some(line.to_owned()),
            downstream: None,
            handle: handle.to_owned(),
        }
    }

    /// 协同服务器配过对的连接符：对端在图 `peer` 上，流出侧线号 `upstream`、流入侧 `downstream`。
    fn served(
        base: SummaryConnector,
        peer: &str,
        upstream: &str,
        downstream: &str,
    ) -> SummaryConnector {
        SummaryConnector {
            peer_kind: PeerKind::Server,
            peer_drawing: Some(peer.to_owned()),
            upstream: Some(upstream.to_owned()),
            downstream: Some(downstream.to_owned()),
            ..base
        }
    }

    fn run(rules: &RuleSet, drawings: &[DrawingSummary]) -> CheckReport {
        run_with(rules, &master_data::Resolved::NotConfigured, drawings)
    }

    /// 只查当前图，没有项目索引。
    fn run_with(
        rules: &RuleSet,
        master: &master_data::Resolved,
        drawings: &[DrawingSummary],
    ) -> CheckReport {
        inspect(rules, master, drawings, None, &[], ScopeKind::Drawing)
    }

    /// 当前图带 `cross_drawing` 查：项目索引由 `drawings` 和 `others` 一起建，查的是 `drawings`。
    fn run_across(
        rules: &RuleSet,
        master: &master_data::Resolved,
        drawings: &[DrawingSummary],
        others: &[DrawingSummary],
    ) -> CheckReport {
        let index = index_of(&[drawings, others].concat(), &[]);
        inspect(
            rules,
            master,
            drawings,
            Some(&index),
            others,
            ScopeKind::Drawing,
        )
    }

    /// 由这些图和读不出来的 `failed` 建的项目索引。
    fn index_of(drawings: &[DrawingSummary], failed: &[FailedDrawing]) -> ProjectIndex {
        build_index(std::path::Path::new("项目"), drawings, failed).expect("建得出项目索引")
    }

    /// 在范围 `kind` 里查 `drawings`；给了 `index` 就是带 `cross_drawing` 查，`others` 是索引里的
    /// 别的图。
    fn inspect(
        rules: &RuleSet,
        master: &master_data::Resolved,
        drawings: &[DrawingSummary],
        index: Option<&ProjectIndex>,
        others: &[DrawingSummary],
        kind: ScopeKind,
    ) -> CheckReport {
        let types = TypeLib::embedded();
        check(&CheckInput {
            rules,
            master,
            types: &types,
            drawings,
            index,
            others,
            waivers: &[],
            trigger: Trigger::OnDemand,
            scope: Scope {
                kind,
                drawing_id: drawings
                    .first()
                    .filter(|_| kind == ScopeKind::Drawing)
                    .map(|drawing| drawing.drawing_id.clone()),
                pbs: None,
                cross_drawing: index.is_some(),
            },
        })
    }

    /// 一条豁免，理由、人和时间都是固定的。
    fn waiver(rule_id: &str, object_key: &str) -> Waiver {
        Waiver {
            rule_id: rule_id.to_owned(),
            object_key: object_key.to_owned(),
            reason: "已知，按设计如此".to_owned(),
            by: "zhangsan".to_owned(),
            at: 1_794_800_000,
        }
    }

    /// 只查当前图，图里记着豁免 `waivers`（图号 → 那张图的豁免）。
    fn run_waived(
        rules: &RuleSet,
        drawings: &[DrawingSummary],
        waivers: &[(String, Vec<Waiver>)],
    ) -> CheckReport {
        let types = TypeLib::embedded();
        check(&CheckInput {
            rules,
            master: &master_data::Resolved::NotConfigured,
            types: &types,
            drawings,
            index: None,
            others: &[],
            waivers,
            trigger: Trigger::OnDemand,
            scope: Scope {
                kind: ScopeKind::Drawing,
                drawing_id: drawings.first().map(|drawing| drawing.drawing_id.clone()),
                pbs: None,
                cross_drawing: false,
            },
        })
    }

    /// 管道等级、绝热等级、流体介质表的一行。
    fn entry(name: &str, attrs: &[(&str, &str)]) -> master_data::Entry {
        master_data::Entry {
            name: name.to_owned(),
            attrs: attrs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// 配好了的项目主数据：图例标准是 `legend`；公称直径 DN50、DN80，DN65 标了隐藏；管道等级
    /// `A1`（碳钢，最大公称压力 16）、`B2`（不锈钢，最大公称压力 40）；绝热等级 `H1`（厚度 50）；
    /// 流体介质 `工艺水`（液）、`蒸汽`（气）、`氯气`（气）。
    fn master(legend: Option<&str>) -> master_data::Resolved {
        let diameter = |dn: u32, hidden: bool| master_data::Diameter {
            dn,
            label: format!("DN{dn}"),
            outer_diameter: None,
            hidden,
        };
        master_data::Resolved::Configured {
            data: master_data::MasterData {
                version: master_data::PROJECT_FILE_VERSION,
                revision: 1,
                updated_at: 0,
                legend_standard: legend.map(str::to_owned),
                nominal_diameters: vec![
                    diameter(50, false),
                    diameter(65, true),
                    diameter(80, false),
                ],
                pipe_classes: vec![
                    entry("A1", &[("材质", "碳钢"), ("最大公称压力", "16")]),
                    entry("B2", &[("材质", "不锈钢"), ("最大公称压力", "40")]),
                ],
                insulation_classes: vec![entry("H1", &[("厚度", "50")])],
                fluid_codes: vec![
                    entry("工艺水", &[("相态", "液")]),
                    entry("蒸汽", &[("相态", "气")]),
                    entry("氯气", &[("相态", "气")]),
                ],
                pbs: Vec::new(),
            },
            source: master_data::Source::Drawing,
        }
    }

    /// 改过几张表的 [`master`]，图例标准没写。
    fn master_with(change: impl FnOnce(&mut master_data::MasterData)) -> master_data::Resolved {
        let master_data::Resolved::Configured { mut data, source } = master(None) else {
            unreachable!("master 给的是配好了的主数据");
        };
        change(&mut data);
        master_data::Resolved::Configured { data, source }
    }

    fn hits(report: &CheckReport, id: &str) -> usize {
        report
            .results
            .iter()
            .filter(|result| result.rule_id == id)
            .count()
    }

    /// 内置规则全部启用，含默认停用的 `DR-114`，给它的参数是只许 `PID` 图层。
    fn all_enabled() -> RuleSet {
        let mut rules = builtin();
        for rule in &mut rules.rules {
            rule.enabled = true;
            if rule.id == "DR-114" {
                rule.params.insert("允许图层".to_owned(), json!(["PID"]));
            }
        }
        rules
    }

    fn custom(id: &str, target: &str, expr: &str, message: Option<&str>) -> Rule {
        let target = Target::parse(target);
        Rule {
            id: id.to_owned(),
            name: "自定规则".to_owned(),
            set: "基础规则".to_owned(),
            category: Category::Design,
            tree: parse(expr, &target).expect("表达式能解析"),
            target,
            severity: Severity::Info,
            expr: expr.to_owned(),
            message: message.map(str::to_owned),
            fix: None,
            scope: RuleScope::Project,
            enabled: true,
            source: Source::New,
            params: BTreeMap::new(),
        }
    }

    /// 只有这几条规则的规则集；规则集的启停照内置包。
    fn only(rules: Vec<Rule>) -> RuleSet {
        RuleSet { rules, ..builtin() }
    }

    /// 一条内置规则命中的例子和不命中的例子，各是一张图；项目级规则的例子在 [`across`] 里。
    /// `SILENT` 里的规则，命中的例子是它要查的那种情形。
    fn example(id: &str) -> Option<(DrawingSummary, DrawingSummary)> {
        let (hit, miss) = match id {
            "DR-001" => (
                vec![line("L-1001", &[])],
                vec![line("L-1001", &[("管道等级", "A1")])],
            ),
            "DR-002" => (
                vec![line("L-1001", &[])],
                vec![line("L-1001", &[("公称直径", "50")])],
            ),
            "DR-003" => (vec![pump(" ")], vec![pump("P-101")]),
            "DR-004" => (
                vec![SummaryObject {
                    tag_status: TAG_STATUS_DUPLICATED.to_owned(),
                    ..pump("P-101")
                }],
                vec![pump("P-101"), pump("P-102")],
            ),
            "DR-005" => (
                vec![instrument("FT-101", &[])],
                vec![instrument("FT-101", &[("仪表回路", "F-101")])],
            ),
            "DR-006" => {
                let fresh = connector("pipe_out", "L-1001", "3A0");
                let with = |plug| with_connectors("PID-1101", Vec::new(), vec![plug]);
                return Some((
                    with(fresh.clone()),
                    with(served(fresh, "PID-1102", "L-1001", "L-2001")),
                ));
            }
            "DR-007" => {
                let with = |rating| {
                    vec![
                        line("L-1001", &[("管道等级", "A1")]),
                        pipe_run("L-1001-01", "L-1001", &[]),
                        on_run("L-1001-01", valve("V-101", &[("压力等级", rating)])),
                    ]
                };
                (with("40"), with("16"))
            }
            "DR-008" => {
                let with = |joined| DrawingSummary {
                    ports: vec![port_of(&pump("P-101"), "N1", joined)],
                    ..drawing("PID-1101", vec![pump("P-101")])
                };
                return Some((with(false), with(true)));
            }
            "DR-009" => (Vec::new(), Vec::new()),
            "DR-010" => (
                vec![pump("P-101")],
                vec![object(
                    ObjectKind::Equipment,
                    "PUMP",
                    "泵",
                    "P-101",
                    &[("名称", "进料泵")],
                )],
            ),
            "DR-011" => (
                vec![pipe_run("L-1001", "L-1001", &[])],
                vec![pipe_run("L-1001-01", "L-1001", &[])],
            ),
            "DR-012" => {
                let from = [0.0, 0.0];
                let with = |to| piping(Vec::new(), vec![segment("L-1001", None, from, to)]);
                return Some((with(from), with([10.0, 0.0])));
            }
            "DR-013" => (
                vec![instrument("FT-101", &[("仪表回路", "F-101")])],
                vec![
                    instrument("FT-101", &[("仪表回路", "F-101")]),
                    instrument("FIC-101", &[("仪表回路", "F-101")]),
                ],
            ),
            "DR-014" => {
                let with = |end: Option<Joint>| {
                    let ends = SummarySegment {
                        joint_a: end.clone(),
                        joint_b: end,
                        ..segment("L-1001", None, [0.0, 0.0], [10.0, 0.0])
                    };
                    piping(vec![line("L-1001", &[])], vec![ends])
                };
                return Some((with(None), with(Some(Joint::Break))));
            }
            "DR-015" => {
                let transmitter = instrument("FT-101", &[]);
                let controller = instrument("FIC-101", &[]);
                let with = |far: Option<Joint>| {
                    let joined = far.is_some();
                    DrawingSummary {
                        signal_ends: vec![
                            signal_end("S-1", [0.0, 0.0], on_port(&transmitter, "out1")),
                            signal_end("S-1", [10.0, 0.0], far),
                        ],
                        ports: vec![
                            port_of(&transmitter, "out1", true),
                            port_of(&controller, "in1", joined),
                        ],
                        ..drawing(
                            "PID-1101",
                            vec![signal("S-1", &[]), transmitter.clone(), controller.clone()],
                        )
                    }
                };
                return Some((with(None), with(on_port(&controller, "in1"))));
            }
            "DR-016" => (
                vec![annotation(&[("文字", "  ")])],
                vec![annotation(&[("文字", "去火炬")])],
            ),
            "DR-017" => (
                vec![valve("V-101", &[("开度", "-5")])],
                vec![valve("V-101", &[("开度", "100")])],
            ),
            "DR-018" => (vec![line("X-1001", &[])], vec![line("L-1001", &[])]),
            "DR-019" => (
                vec![line(&format!("L-{}", "1".repeat(31)), &[])],
                vec![line("L-1001", &[])],
            ),
            "DR-020" => {
                let with = |ports| DrawingSummary {
                    ports,
                    ..drawing("PID-1101", vec![pump("P-101")])
                };
                return Some((with(nozzles("PUMP", "P-101", 2)), with(Vec::new())));
            }
            "DR-021" => (
                vec![line("L-1001", &[("流体介质", "氯气"), ("管道等级", "A1")])],
                vec![line("L-1001", &[("流体介质", "氯气"), ("管道等级", "B2")])],
            ),
            "DR-022" => (
                vec![line("L-1001", &[("管道等级", "C3")])],
                vec![line("L-1001", &[("管道等级", "A1")])],
            ),
            "DR-023" => {
                let with = |count| DrawingSummary {
                    ports: nozzles("PUMP", "P-101", count),
                    ..drawing("PID-1101", vec![pump("P-101")])
                };
                return Some((with(1), with(2)));
            }
            "DR-024" => (
                vec![with_symbol(pump("P-101"), "PID_SYM_equipment_pump", false)],
                vec![with_symbol(pump("P-101"), "PID_SYM_equipment_pump", true)],
            ),
            "DR-025" => (
                vec![line("L-1001", &[("流体介质", "蒸汽")])],
                vec![line("L-1001", &[("流体介质", "蒸汽"), ("绝热等级", "H1")])],
            ),
            "DR-101" => (vec![valve("", &[])], vec![valve("V-101", &[])]),
            "DR-103" => (
                vec![line("L-1001", &[("公称直径", "DN100")])],
                vec![line("L-1001", &[("公称直径", "DN50")])],
            ),
            "DR-104" => (
                vec![line("L-1001", &[("流体介质", "氨")])],
                vec![line("L-1001", &[("流体介质", "工艺水")])],
            ),
            "DR-105" => {
                let with = |size| {
                    let first = segment("L-1001", Some("L-1001-01"), [0.0, 0.0], [10.0, 0.0]);
                    let second = segment("L-1001", Some("L-1001-02"), [10.0, 0.0], [20.0, 0.0]);
                    piping(
                        vec![
                            pipe_run("L-1001-01", "L-1001", &[("公称直径", "50")]),
                            pipe_run("L-1001-02", "L-1001", &[("公称直径", size)]),
                        ],
                        vec![
                            SummarySegment {
                                joint_b: joined(Some("L-1001-02"), false),
                                ..first
                            },
                            second,
                        ],
                    )
                };
                return Some((with("80"), with("50")));
            }
            "DR-106" => {
                let with = |size| {
                    vec![
                        pipe_run("L-1001-01", "L-1001", &[("公称直径", "50")]),
                        on_run("L-1001-01", valve("V-101", &[("公称直径", size)])),
                    ]
                };
                (with("80"), with("50"))
            }
            "DR-107" => {
                let with = |class| {
                    vec![
                        line("L-1001", &[("管道等级", "A1")]),
                        pipe_run("L-1001-01", "L-1001", &[]),
                        on_run("L-1001-01", valve("V-101", &[("管道等级", class)])),
                    ]
                };
                (with("B2"), with("A1"))
            }
            "DR-108" => {
                let equipment = pump("P-101");
                let transmitter = instrument("FT-101", &[]);
                let with = |to: &SummaryObject| DrawingSummary {
                    signal_ends: vec![signal_end("S-1", [0.0, 0.0], on_port(to, "N1"))],
                    ports: vec![
                        port_of(&equipment, "N1", true),
                        port_of(&transmitter, "N1", true),
                    ],
                    ..drawing(
                        "PID-1101",
                        vec![signal("S-1", &[]), equipment.clone(), transmitter.clone()],
                    )
                };
                return Some((with(&equipment), with(&transmitter)));
            }
            "DR-109" => {
                let check = on_run(
                    "L-1001-01",
                    with_symbol(
                        object(ObjectKind::Valve, "VALVE_CHECK", "止回阀", "V-101", &[]),
                        "PID_SYM_valve_check_valve",
                        false,
                    ),
                );
                let with = |inlet, outlet| {
                    let upstream = SummarySegment {
                        joint_b: on_port(&check, inlet),
                        ..segment("L-1001", Some("L-1001-01"), [0.0, 0.0], [10.0, 0.0])
                    };
                    let downstream = SummarySegment {
                        joint_a: on_port(&check, outlet),
                        ..segment("L-1001", Some("L-1001-01"), [20.0, 0.0], [30.0, 0.0])
                    };
                    DrawingSummary {
                        ports: vec![
                            SummaryPort {
                                direction: Some(PortDir::In),
                                ..port_of(&check, "in1", true)
                            },
                            SummaryPort {
                                direction: Some(PortDir::Out),
                                ..port_of(&check, "out1", true)
                            },
                        ],
                        ..piping(
                            vec![pipe_run("L-1001-01", "L-1001", &[]), check.clone()],
                            vec![upstream, downstream],
                        )
                    }
                };
                return Some((with("out1", "in1"), with("in1", "out1")));
            }
            "DR-110" => {
                let with = |from: f64| {
                    piping(
                        Vec::new(),
                        vec![
                            segment("L-1001", None, [0.0, 0.0], [10.0, 0.0]),
                            segment("L-1001", None, [from, 0.0], [from + 10.0, 0.0]),
                        ],
                    )
                };
                return Some((with(5.0), with(10.0)));
            }
            "DR-111" => {
                let gate = with_symbol(valve("V-101", &[]), "PID_SYM_valve_gate_valve", true);
                let with = |joined| DrawingSummary {
                    ports: vec![port_of(&gate, "in1", true), port_of(&gate, "out1", joined)],
                    ..drawing("PID-1101", vec![gate.clone()])
                };
                return Some((with(false), with(true)));
            }
            "DR-113" => {
                let transmitter = with_symbol(
                    instrument("FT-101", &[("仪表回路", "F-101")]),
                    "PID_SYM_instrument_transmitter",
                    true,
                );
                let controller = with_symbol(
                    instrument("FIC-101", &[("仪表回路", "F-101")]),
                    "PID_SYM_instrument_controller",
                    true,
                );
                (vec![transmitter.clone()], vec![transmitter, controller])
            }
            "DR-114" => (
                vec![SummaryObject {
                    layer: "0".to_owned(),
                    ..line("L-1001", &[])
                }],
                vec![line("L-1001", &[])],
            ),
            "DR-115" => {
                let with = |class| {
                    vec![
                        line("L-1001", &[("管道等级", "A1")]),
                        pipe_run("L-1001-01", "L-1001", &[("管道等级", class)]),
                    ]
                };
                (with("B2"), with("A1"))
            }
            _ => return None,
        };
        Some((drawing("PID-1101", hit), drawing("PID-1101", miss)))
    }

    /// 项目级规则的例子：命中的和不命中的各是一个项目的几张图，查第一张，其余的只进项目索引。
    /// `PID-1101` 上流出的 `L-1001` 和 `PID-1102` 上流入的 `L-2001` 由协同服务器配过对。
    fn across(id: &str) -> Option<(Vec<DrawingSummary>, Vec<DrawingSummary>)> {
        let out = served(
            connector("pipe_out", "L-1001", "3A0"),
            "PID-1102",
            "L-1001",
            "L-2001",
        );
        let back = served(
            connector("pipe_in", "L-2001", "4B0"),
            "PID-1101",
            "L-1001",
            "L-2001",
        );
        let pair = match id {
            "DR-102" => {
                let with = |there| {
                    vec![
                        drawing("PID-1101", vec![pump("P-1001")]),
                        drawing("PID-1102", vec![pump(there)]),
                    ]
                };
                (with("P-1001"), with("P-1002"))
            }
            "DR-112" => {
                let pipe = |tag, class| line(tag, &[("管道等级", class), ("公称直径", "50")]);
                let with = |class| {
                    vec![
                        with_connectors("PID-1101", vec![pipe("L-1001", "A1")], vec![out.clone()]),
                        with_connectors(
                            "PID-1102",
                            vec![pipe("L-2001", class)],
                            vec![back.clone()],
                        ),
                    ]
                };
                (with("B2"), with("A1"))
            }
            "DR-116" => {
                let with = |there| {
                    vec![
                        with_connectors("PID-1101", Vec::new(), vec![out.clone()]),
                        with_connectors("PID-1102", Vec::new(), there),
                    ]
                };
                (with(Vec::new()), with(vec![back.clone()]))
            }
            _ => return None,
        };
        Some(pair)
    }

    /// 表驱动：每条内置规则在它的例子图上，命中的例子出一条（`MUTUAL` 里的出两条，`SILENT` 里的
    /// 本期不出）、不命中的不出。41 条都有例子：项目级的见 [`across`]，其余的见 [`example`]。例子
    /// 带着由例子里的图建的项目索引、在配好了的主数据（[`master`]）下跑；`DR-009` 命中的那次，
    /// 主数据里没写图例标准。
    #[test]
    fn every_builtin_rule_has_a_hit_and_a_miss() {
        let rules = all_enabled();
        let (configured, unset) = (master(Some("GB/T 6567")), master(None));
        let ids: Vec<&str> = rules.rules.iter().map(|rule| rule.id.as_str()).collect();
        assert_eq!(ids.len(), 41);
        for id in SILENT {
            assert!(ids.contains(&id), "名单里的 {id} 不是内置规则");
        }
        for id in ids {
            let (hit, miss) = across(id)
                .or_else(|| example(id).map(|(hit, miss)| (vec![hit], vec![miss])))
                .unwrap_or_else(|| panic!("{id} 没有例子"));
            let expected = if SILENT.contains(&id) {
                0
            } else if MUTUAL.contains(&id) {
                2
            } else {
                1
            };
            let legend = if id == "DR-009" { &unset } else { &configured };
            let report = run_across(&rules, legend, &hit[..1], &hit[1..]);
            assert_eq!(hits(&report, id), expected, "{id} 应命中 {expected} 条");
            let report = run_across(&rules, &configured, &miss[..1], &miss[1..]);
            assert_eq!(hits(&report, id), 0, "{id} 不应命中");
        }
    }

    /// 用到 `位号.跨图重复数`、`配对状态`、`对端线` 的是项目级规则，经关系走到的也算：没有项目
    /// 索引时不跑，计入 `rules_skipped`，`notices` 里说有几条没跑，报告的 `cross_drawing` 为假；
    /// 有索引时照跑。`所在线`、别的对象上叫 `配对状态` 的属性不算。
    #[test]
    fn project_rules_are_skipped_without_the_project_index() {
        let configured = master(Some("GB/T 6567"));
        let (project, _) = across("DR-102").expect("DR-102");
        let report = run_with(&all_enabled(), &configured, &project[..1]);
        for id in ["DR-102", "DR-112", "DR-116"] {
            assert_eq!(hits(&report, id), 0, "{id}");
        }
        assert_eq!(
            (report.summary.rules_run, report.summary.rules_skipped),
            (38, 3)
        );
        assert_eq!(
            report.notices,
            [Notice {
                code: "PROJECT_RULES_SKIPPED".to_owned(),
                message: "只检查了当前图，3 条项目级规则没有执行".to_owned(),
            }]
        );
        let report = run_across(&all_enabled(), &configured, &project[..1], &project[1..]);
        assert_eq!(hits(&report, "DR-102"), 1);
        assert_eq!(
            (report.summary.rules_run, report.summary.rules_skipped),
            (41, 0)
        );
        assert!(report.notices.is_empty());
        assert!(report.scope.cross_drawing);

        let rules = only(vec![
            custom("XY-001", "阀门", "所属管线.位号.跨图重复数 > 0", None),
            custom("XY-002", "页间连接符", "配对状态 = NULL", None),
            custom("XY-003", "页间连接符", "对端线.管道等级 = NULL", None),
            custom("XY-004", "页间连接符", "所在线.管道等级 = NULL", None),
            custom("XY-005", "管线", "配对状态 = NULL", None),
        ]);
        let types = TypeLib::embedded();
        for (kind, message) in [
            (ScopeKind::Drawing, "只检查了当前图，3 条项目级规则没有执行"),
            (ScopeKind::Project, "没有项目索引，3 条项目级规则没有执行"),
        ] {
            let report = check(&CheckInput {
                rules: &rules,
                master: &configured,
                types: &types,
                drawings: &project,
                index: None,
                others: &[],
                waivers: &[],
                trigger: Trigger::OnDemand,
                scope: Scope {
                    kind,
                    drawing_id: None,
                    pbs: None,
                    cross_drawing: true,
                },
            });
            assert_eq!(
                (report.summary.rules_run, report.summary.rules_skipped),
                (2, 3),
                "{kind:?}"
            );
            assert_eq!(report.notices.len(), 1, "{kind:?}");
            assert_eq!(report.notices[0].message, message);
            assert!(!report.scope.cross_drawing);
        }
    }

    /// 当前图带 `cross_drawing` 查时，项目级规则看整个项目的索引，只报当前图上的对象；查整个项目
    /// 时各图上的都报。`位号.跨图重复数` 数同类型同位号的对象还在几张别的图上：同一张图上重了的
    /// 不算，管线这样本来就跨图的不算。项目里有图读不出来时 `notices` 给 `INDEX_INCOMPLETE`。
    #[test]
    fn a_drawing_checked_across_the_project_reports_only_its_own_objects() {
        let unset = master_data::Resolved::NotConfigured;
        let rules = only(vec![builtin().rule("DR-102").expect("DR-102").clone()]);
        let (project, _) = across("DR-102").expect("DR-102");
        let places = |report: &CheckReport| -> Vec<String> {
            report
                .results
                .iter()
                .map(|result| format!("{} {}", result.object.drawing_id, result.object_key))
                .collect()
        };
        let report = run_across(&rules, &unset, &project[..1], &project[1..]);
        assert_eq!(places(&report), ["PID-1101 PUMP|P-1001"]);
        assert_eq!(
            report.results[0].message,
            "泵 的位号 P-1001 在别的图纸上也有"
        );
        let index = index_of(&project, &[]);
        let report = inspect(
            &rules,
            &unset,
            &project,
            Some(&index),
            &[],
            ScopeKind::Project,
        );
        assert_eq!(
            places(&report),
            ["PID-1101 PUMP|P-1001", "PID-1102 PUMP|P-1001"]
        );
        assert_eq!(report.scope.kind, ScopeKind::Project);
        assert!(report.scope.cross_drawing && report.notices.is_empty());

        let drawings = [
            drawing(
                "PID-1101",
                vec![
                    pump("P-1001"),
                    pump("P-1001"),
                    pump("P-1002"),
                    line("L-1001", &[]),
                ],
            ),
            drawing("PID-1102", vec![pump("P-1001"), line("L-1001", &[])]),
            drawing("PID-1103", vec![pump("P-1001"), pump("P-1003")]),
        ];
        let counted = |count: usize| -> Vec<String> {
            let expr = format!("位号.跨图重复数 = {count}");
            let rules = only(vec![custom("XY-001", "对象实例", &expr, None)]);
            run_across(&rules, &unset, &drawings[..1], &drawings[1..])
                .results
                .into_iter()
                .map(|result| result.object_key)
                .collect()
        };
        assert_eq!(counted(2), ["PUMP|P-1001", "PUMP|P-1001"]);
        assert_eq!(counted(0), ["PIPE_LINE|L-1001", "PUMP|P-1002"]);

        let failed = [FailedDrawing {
            drawing_id: "PID-1109".to_owned(),
            file: "PID-1109.dwg".to_owned(),
            pbs: None,
            error: "文件损坏".to_owned(),
        }];
        let index = index_of(&project, &failed);
        let report = inspect(
            &rules,
            &unset,
            &project[..1],
            Some(&index),
            &project[1..],
            ScopeKind::Drawing,
        );
        assert_eq!(hits(&report, "DR-102"), 1);
        assert_eq!(
            report.notices,
            [Notice {
                code: "INDEX_INCOMPLETE".to_owned(),
                message: "项目里有 1 张图读不出来，跨图的结论不完整".to_owned(),
            }]
        );
    }

    /// 页间连接符的事实：`连接符种类` 是摘要里记的种类；`对端图纸` 是记着的对端图，同进程配的对
    /// 是「本机配对」；`对端位号` 在两侧线号都记着时取对面那侧的；`配对状态` 照项目索引。
    /// `所在线` 是本图上这个线号的管线或信号线，`对端线` 是配成了对的那个连接符所在的线。结果里
    /// 连接符的属主是它所在的线。有连接符分不出对端时 `notices` 给 `CONNECTOR_AMBIGUOUS`。
    #[test]
    fn connectors_read_their_peer_from_the_drawing_and_the_index() {
        let at = |x: f64, base: SummaryConnector| SummaryConnector {
            at: [x, 0.0],
            ..base
        };
        let local = |base: SummaryConnector, upstream: &str, downstream: &str| SummaryConnector {
            peer_kind: PeerKind::Local,
            upstream: Some(upstream.to_owned()),
            downstream: Some(downstream.to_owned()),
            ..base
        };
        let targeted = SummaryConnector {
            peer_kind: PeerKind::Target,
            peer_drawing: Some("PID-1103".to_owned()),
            upstream: Some("L-3001".to_owned()),
            downstream: Some("L-1001".to_owned()),
            ..connector("pipe_in", "L-1001", "A2")
        };
        let here = with_connectors(
            "PID-1101",
            vec![line("L-1001", &[("管道等级", "A1")]), signal("S-101", &[])],
            vec![
                at(
                    1.0,
                    served(
                        connector("pipe_out", "L-1001", "A1"),
                        "PID-1102",
                        "L-1001",
                        "L-2001",
                    ),
                ),
                at(2.0, targeted),
                at(
                    3.0,
                    local(connector("signal_out", "S-101", "A3"), "S-101", "S-201"),
                ),
                at(
                    4.0,
                    served(
                        connector("pipe_out", "L-1001", "A4"),
                        "PID-1109",
                        "L-1001",
                        "L-9001",
                    ),
                ),
                at(
                    5.0,
                    served(
                        connector("pipe_out", "L-1009", "A5"),
                        "PID-1102",
                        "L-1009",
                        "L-2009",
                    ),
                ),
                at(6.0, connector("pipe_out", "L-1001", "A6")),
            ],
        );
        let there = [
            with_connectors(
                "PID-1102",
                vec![line("L-2001", &[("管道等级", "B2")]), signal("S-201", &[])],
                vec![
                    served(
                        connector("pipe_in", "L-2001", "B1"),
                        "PID-1101",
                        "L-1001",
                        "L-2001",
                    ),
                    local(connector("signal_in", "S-201", "B2"), "S-101", "S-201"),
                ],
            ),
            with_connectors(
                "PID-1103",
                vec![signal("S-201", &[])],
                vec![local(
                    connector("signal_in", "S-201", "C1"),
                    "S-101",
                    "S-201",
                )],
            ),
        ];
        let checked = |expr: &str, message: Option<&str>| {
            let rules = only(vec![custom("XY-001", "页间连接符", expr, message)]);
            run_across(
                &rules,
                &master_data::Resolved::NotConfigured,
                std::slice::from_ref(&here),
                &there,
            )
        };
        let report = checked(
            "连接符种类 != NULL",
            Some("{所属}|{连接符种类}|{对端图纸}|{对端位号}|{配对状态}"),
        );
        let messages: Vec<&str> = report
            .results
            .iter()
            .map(|result| result.message.as_str())
            .collect();
        assert_eq!(
            messages,
            [
                "L-1001|pipe_out|PID-1102|L-2001|paired",
                "L-1001|pipe_in|PID-1103|L-3001|unpaired",
                "L-1001|pipe_out|PID-1109|L-9001|target_missing",
                "L-1001|pipe_out|||unpaired",
                "L-1009|pipe_out|PID-1102|L-2009|one_sided",
                "S-101|signal_out|本机配对|S-201|ambiguous",
            ]
        );
        assert_eq!(
            report.results[0].object,
            ObjectRef {
                kind: "页间连接符".to_owned(),
                type_code: "PIPE_LINE".to_owned(),
                tag: None,
                drawing_id: "PID-1101".to_owned(),
                owner: Some("L-1001".to_owned()),
                port: None,
                at: [1.0, 0.0],
                handle: Some("A1".to_owned()),
            }
        );
        assert_eq!(report.results[0].object_key, "PIPE_LINE|L-1001|1.000,0.000");
        assert_eq!(
            report.notices,
            [Notice {
                code: "CONNECTOR_AMBIGUOUS".to_owned(),
                message: "有 1 个页间连接符分不出对端是哪一个，跨图的结论不完整".to_owned(),
            }]
        );

        let keys = |expr: &str| -> Vec<String> {
            checked(expr, None)
                .results
                .into_iter()
                .map(|result| result.object_key)
                .collect()
        };
        assert_eq!(
            keys("所在线.管道等级 = \"A1\""),
            [
                "PIPE_LINE|L-1001|1.000,0.000",
                "PIPE_LINE|L-1001|2.000,0.000",
                "PIPE_LINE|L-1001|4.000,0.000",
                "PIPE_LINE|L-1001|6.000,0.000",
            ]
        );
        assert_eq!(
            keys("所在线.位号 = \"S-101\""),
            ["SIGNAL_LINE|S-101|3.000,0.000"]
        );
        assert_eq!(
            keys("对端线.管道等级 = \"B2\""),
            ["PIPE_LINE|L-1001|1.000,0.000"]
        );
        assert_eq!(
            keys("对端线.位号 = NULL"),
            [
                "PIPE_LINE|L-1001|2.000,0.000",
                "PIPE_LINE|L-1001|4.000,0.000",
                "PIPE_LINE|L-1001|6.000,0.000",
                "PIPE_LINE|L-1009|5.000,0.000",
                "SIGNAL_LINE|S-101|3.000,0.000",
            ]
        );
    }

    /// 规则的作用域：`项目` 处处查；`PBS 节点` 只查挂在这个节点及其下级上的图，主数据没配好时
    /// 只认挂在这个节点自己上的；`图纸` 只查这张图；`类型` 只查这个类型及其下级的对象，图纸和
    /// 仪表回路没有类型，不查。作用域指的东西不在项目里时 `notices` 给 `RULE_SCOPE_UNKNOWN`；主数据没
    /// 配好时不知道有哪些节点，没有项目索引时不知道有哪些图，就不说。
    #[test]
    fn rule_scopes_limit_where_a_rule_applies() {
        let node = |id: &str, parent: Option<&str>| master_data::PbsNode {
            id: id.to_owned(),
            label: id.to_owned(),
            parent: parent.map(str::to_owned),
        };
        let configured = master_with(|data| {
            data.pbs = vec![
                node("1001", None),
                node("1001-01", Some("1001")),
                node("2001", None),
            ];
        });
        let placed = |id: &str, pbs: &str, objects| DrawingSummary {
            pbs: Some(pbs.to_owned()),
            ..drawing(id, objects)
        };
        let drawings = [
            placed("PID-1101", "1001-01", vec![pump("P-101"), tank("T-101")]),
            placed("PID-1102", "2001", vec![pump("P-201")]),
            placed("PID-1103", "1001", vec![pump("P-301")]),
        ];
        let scoped = |id: &str, scope| Rule {
            scope,
            ..custom(id, "设备", "位号 != NULL", None)
        };
        let rules = only(vec![
            scoped("XY-001", RuleScope::Project),
            scoped("XY-002", RuleScope::Pbs("1001".to_owned())),
            scoped("XY-003", RuleScope::Drawing("PID-1102".to_owned())),
            scoped("XY-004", RuleScope::Type("VESSEL".to_owned())),
            Rule {
                scope: RuleScope::Type("VESSEL".to_owned()),
                ..custom("XY-005", "图纸", "图号 != NULL", None)
            },
            scoped("XY-006", RuleScope::Pbs("9999".to_owned())),
            scoped("XY-007", RuleScope::Drawing("PID-9999".to_owned())),
            scoped("XY-008", RuleScope::Type("NOPE".to_owned())),
        ]);
        let found = |report: &CheckReport| -> Vec<String> {
            report
                .results
                .iter()
                .map(|result| format!("{} {}", result.rule_id, result.object_key))
                .collect()
        };
        let unknown = |message: &str| Notice {
            code: "RULE_SCOPE_UNKNOWN".to_owned(),
            message: message.to_owned(),
        };
        let index = index_of(&drawings, &[]);
        let report = inspect(
            &rules,
            &configured,
            &drawings,
            Some(&index),
            &[],
            ScopeKind::Project,
        );
        assert_eq!(
            found(&report),
            [
                "XY-001 PUMP|P-101",
                "XY-001 TANK|T-101",
                "XY-001 PUMP|P-201",
                "XY-001 PUMP|P-301",
                "XY-002 PUMP|P-101",
                "XY-002 TANK|T-101",
                "XY-002 PUMP|P-301",
                "XY-003 PUMP|P-201",
                "XY-004 TANK|T-101",
            ]
        );
        assert_eq!(
            report.notices,
            [
                unknown("规则 XY-006 的作用域是 PBS 节点 9999，项目主数据里没有这个节点"),
                unknown("规则 XY-007 的作用域是图纸 PID-9999，项目里没有这张图"),
                unknown("规则 XY-008 的作用域是类型 NOPE，类型库里没有这个类型"),
            ]
        );

        let report = inspect(
            &rules,
            &master_data::Resolved::NotConfigured,
            &drawings,
            None,
            &[],
            ScopeKind::Project,
        );
        assert_eq!(
            found(&report),
            [
                "XY-001 PUMP|P-101",
                "XY-001 TANK|T-101",
                "XY-001 PUMP|P-201",
                "XY-001 PUMP|P-301",
                "XY-002 PUMP|P-301",
                "XY-003 PUMP|P-201",
                "XY-004 TANK|T-101",
            ]
        );
        assert_eq!(
            report.notices,
            [unknown(
                "规则 XY-008 的作用域是类型 NOPE，类型库里没有这个类型"
            )]
        );
    }

    /// 豁免按规则号和对象键对上同一张图上的结果（§3.7），对象键的四种写法各一例：有自己编号的
    /// 对象、管口、仪表回路和图纸、其余对象（这里是页间连接符）。豁免了的结果还在，`waived` 为真、
    /// 带着那条豁免，不计入三个严重度，另计 `已豁免`；`blocked` 只看没豁免的错误。
    #[test]
    fn waivers_match_results_on_the_rule_and_the_object_key() {
        let rules = only(vec![
            Rule {
                severity: Severity::Error,
                ..custom("XY-001", "设备", "位号 = \"P-101\"", None)
            },
            Rule {
                severity: Severity::Error,
                ..custom("XY-002", "管口", "连接状态 = \"未连接\"", None)
            },
            custom("XY-003", "仪表回路", "回路号 = \"F-101\"", None),
            custom("XY-004", "图纸", "图号 = \"PID-1101\"", None),
            Rule {
                severity: Severity::Warning,
                ..custom("XY-005", "页间连接符", "连接符种类 = \"pipe_out\"", None)
            },
        ]);
        let drawings = [DrawingSummary {
            ports: nozzles("PUMP", "P-101", 1),
            ..with_connectors(
                "PID-1101",
                vec![
                    pump("P-101"),
                    instrument("FT-101", &[("仪表回路", "F-101")]),
                ],
                vec![connector("pipe_out", "L-1001", "3A0")],
            )
        }];
        let keys = [
            ("XY-001", "PUMP|P-101"),
            ("XY-002", "PUMP|P-101#N1"),
            ("XY-005", "PIPE_LINE|L-1001|300.000,100.000"),
            ("XY-003", "仪表回路|F-101"),
            ("XY-004", "图纸|PID-1101"),
        ];
        let counts = |report: &CheckReport| {
            let summary = report.summary;
            (
                summary.errors,
                summary.warnings,
                summary.infos,
                summary.waived,
                report.blocked,
            )
        };
        let plain = run_waived(&rules, &drawings, &[]);
        let found: Vec<(&str, &str)> = plain
            .results
            .iter()
            .map(|result| (result.rule_id.as_str(), result.object_key.as_str()))
            .collect();
        assert_eq!(found, keys);
        assert_eq!(counts(&plain), (2, 1, 2, 0, true));

        let every: Vec<Waiver> = keys.iter().map(|(rule, key)| waiver(rule, key)).collect();
        let report = run_waived(&rules, &drawings, &[("PID-1101".to_owned(), every.clone())]);
        let attached: Vec<Option<&Waiver>> = report
            .results
            .iter()
            .map(|result| result.waiver.as_ref())
            .collect();
        assert_eq!(attached, every.iter().map(Some).collect::<Vec<_>>());
        assert!(report.results.iter().all(|result| result.waived));
        assert_eq!(counts(&report), (0, 0, 0, 5, false));
        assert!(report.stale_waivers.is_empty());

        let first = [("PID-1101".to_owned(), vec![waiver("XY-001", "PUMP|P-101")])];
        let report = run_waived(&rules, &drawings, &first);
        assert_eq!(counts(&report), (1, 1, 2, 1, true));
        let value = serde_json::to_value(&report).expect("能写成 JSON");
        assert_eq!(value["summary"]["已豁免"], 1);
        assert_eq!(value["results"][0]["waived"], json!(true));
        assert_eq!(
            value["results"][0]["waiver"],
            json!({ "reason": "已知，按设计如此", "by": "zhangsan", "at": 1_794_800_000 })
        );
        assert_eq!(value["results"][1]["waived"], json!(false));
        assert_eq!(value["results"][1]["waiver"], Value::Null);
        let both = [(
            "PID-1101".to_owned(),
            vec![
                waiver("XY-001", "PUMP|P-101"),
                waiver("XY-002", "PUMP|P-101#N1"),
            ],
        )];
        assert_eq!(
            counts(&run_waived(&rules, &drawings, &both)),
            (0, 1, 2, 2, false)
        );

        let near = [
            (
                "PID-1101".to_owned(),
                vec![
                    waiver("XY-003", "PUMP|P-101"),
                    waiver("XY-001", "PUMP|P-102"),
                    waiver("XY-001", "pump|P-101"),
                ],
            ),
            ("PID-1102".to_owned(), vec![waiver("XY-001", "PUMP|P-101")]),
        ];
        let report = run_waived(&rules, &drawings, &near);
        assert_eq!(counts(&report), (2, 1, 2, 0, true));
        assert_eq!(report.stale_waivers, near[0].1);
    }

    /// 图里记着、这次却没有结果对得上的豁免进 `stale_waivers`（§3.7）：对象改了位号、问题改掉了、
    /// 规则包里没有这条规则。停用的规则、没有项目索引而没跑的项目级规则下不了结论，它们的豁免不算。
    #[test]
    fn waivers_without_a_result_this_time_are_stale() {
        let rules = only(vec![
            Rule {
                severity: Severity::Error,
                ..custom("XY-001", "设备", "设备类别 = \"泵\"", None)
            },
            Rule {
                enabled: false,
                ..custom("XY-002", "设备", "设备类别 = \"泵\"", None)
            },
            custom("XY-003", "设备", "位号.跨图重复数 > 0", None),
        ]);
        let kept = waiver("XY-001", "PUMP|P-101");
        let gone = waiver("XY-099", "PUMP|P-101");
        let waivers = [(
            "PID-1101".to_owned(),
            vec![
                kept.clone(),
                waiver("XY-002", "PUMP|P-101"),
                waiver("XY-003", "PUMP|P-101"),
                gone.clone(),
            ],
        )];
        let check_with = |objects| run_waived(&rules, &[drawing("PID-1101", objects)], &waivers);

        let report = check_with(vec![pump("P-101")]);
        assert_eq!(
            (report.summary.errors, report.summary.waived, report.blocked),
            (0, 1, false)
        );
        assert_eq!(report.stale_waivers, std::slice::from_ref(&gone));

        let renamed = check_with(vec![pump("P-102")]);
        assert_eq!(
            (
                renamed.summary.errors,
                renamed.summary.waived,
                renamed.blocked
            ),
            (1, 0, true)
        );
        assert_eq!(renamed.results[0].object_key, "PUMP|P-102");
        assert!(!renamed.results[0].waived);
        assert_eq!(renamed.stale_waivers, [kept.clone(), gone.clone()]);
        let value = serde_json::to_value(&renamed).expect("能写成 JSON");
        assert_eq!(
            value["stale_waivers"][0],
            json!({
                "rule_id": "XY-001", "object_key": "PUMP|P-101",
                "reason": "已知，按设计如此", "by": "zhangsan", "at": 1_794_800_000
            })
        );

        let fixed = check_with(vec![tank("T-101")]);
        assert!(fixed.results.is_empty());
        assert_eq!(fixed.stale_waivers, [kept, gone]);
    }

    /// 第一批纯属性规则都有正反例；需求的例子：管线设计温度 260、流体介质工艺水、没有绝热等级，
    /// `DR-025` 命中一条。
    #[test]
    fn the_requirement_example_hits_dr_025_once() {
        for id in [
            "DR-001", "DR-002", "DR-003", "DR-005", "DR-016", "DR-017", "DR-025", "DR-101",
        ] {
            assert!(example(id).is_some(), "{id} 要有正反例");
        }
        let report = run(
            &builtin(),
            &[drawing(
                "PID-1101",
                vec![line(
                    "L-1003",
                    &[("设计温度", "260"), ("流体介质", "工艺水")],
                )],
            )],
        );
        let found: Vec<&CheckResult> = report
            .results
            .iter()
            .filter(|result| result.rule_id == "DR-025")
            .collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert_eq!(
            found[0].message,
            "管线 L-1003 是高温或蒸汽管线，没有绝热等级"
        );
        assert_eq!(
            found[0].fix,
            "填写绝热等级：PIDSETPROP L-1003 绝热等级 <值>"
        );
    }

    /// §3.6 的例子：一条结果的对象引用、对象键、消息和修复建议。
    #[test]
    fn a_result_carries_its_object_key_message_and_fix() {
        let pipe = SummaryObject {
            at: [0.0, 20.0],
            ..line("L-1003", &[])
        };
        let nameless = SummaryObject {
            at: [12.3456, -7.0],
            ..pump("")
        };
        let report = run(&builtin(), &[drawing("PID-1101", vec![pipe, nameless])]);

        let result = report
            .results
            .iter()
            .find(|result| result.rule_id == "DR-001")
            .expect("DR-001 命中");
        assert_eq!(result.rule_name, "管线缺管道等级");
        assert_eq!(result.severity, Severity::Error);
        assert_eq!(
            result.object,
            ObjectRef {
                kind: "管线".to_owned(),
                type_code: "PIPE_LINE".to_owned(),
                tag: Some("L-1003".to_owned()),
                drawing_id: "PID-1101".to_owned(),
                owner: None,
                port: None,
                at: [0.0, 20.0],
                handle: Some("2D4".to_owned()),
            }
        );
        assert_eq!(result.object_key, "PIPE_LINE|L-1003");
        assert_eq!(result.message, "管线 L-1003 没有管道等级");
        assert_eq!(
            result.fix,
            "给管线填写管道等级：PIDSETPROP L-1003 管道等级 <值>"
        );
        assert!(result.points.is_empty());
        assert_eq!(result.fields, ["管道等级"]);
        assert!(!result.waived);
        assert_eq!(result.waiver, None);

        let result = report
            .results
            .iter()
            .find(|result| result.rule_id == "DR-003")
            .expect("DR-003 命中");
        assert_eq!(result.object.tag, None);
        assert_eq!(result.object_key, "PUMP||12.346,-7.000");
        assert_eq!(result.message, "设备（泵）没有位号");
        assert_eq!(result.fix, "选中这台设备，在属性表里填写位号");
    }

    /// 模板里的四个固定名字、事实名和属性名都能填，没有值填空串；没写模板用「名称：位号或
    /// 位置」；超过 200 字截断。没有自己编号的对象键用所属和坐标。
    #[test]
    fn templates_fill_in_names_and_fall_back_to_the_rule_name() {
        let fitting = SummaryObject {
            owner: Some("L-1001-01".to_owned()),
            ..object(
                ObjectKind::Fitting,
                "FITTING",
                "管件",
                "L-1001-01",
                &[("材质", " 碳钢 ")],
            )
        };
        let long = "字".repeat(250);
        let rules = only(vec![
            custom(
                "XY-001",
                "管件",
                "材质 != NULL",
                Some("{种类} {位号} 在 {图号}，属于 {所属}；{材质}{类型}{没有这个}。{位号"),
            ),
            custom("XY-002", "管件", "材质 != NULL", None),
            custom("XY-003", "PUMP", "材质 = NULL", None),
            custom("XY-004", "管件", "材质 != NULL", Some(&long)),
        ]);
        let report = run(&rules, &[drawing("PID-1101", vec![fitting, pump("")])]);
        let message = |id: &str| {
            report
                .results
                .iter()
                .find(|result| result.rule_id == id)
                .map(|result| (result.message.clone(), result.object_key.clone()))
                .unwrap_or_else(|| panic!("{id} 应命中"))
        };
        assert_eq!(
            message("XY-001"),
            (
                "管件 L-1001-01 在 PID-1101，属于 L-1001-01；碳钢管件。{位号".to_owned(),
                "FITTING|L-1001-01|10.000,20.000".to_owned()
            )
        );
        assert_eq!(message("XY-002").0, "自定规则：L-1001-01");
        assert_eq!(message("XY-003").0, "自定规则：10.000,20.000");
        assert_eq!(message("XY-004").0, "字".repeat(200));
        assert!(report.results.iter().all(|result| result.fix.is_empty()));
    }

    /// `fields`：单属性、多属性、用到位号、沿关系取的不算；派生事实不算；按出现的先后、去重。
    #[test]
    fn fields_follow_the_expression() {
        let rules = builtin();
        let of = |id: &str| fields(rules.rule(id).expect("内置规则"));
        assert_eq!(of("DR-001"), ["管道等级"]);
        assert_eq!(of("DR-025"), ["设计温度", "流体介质", "绝热等级"]);
        assert_eq!(of("DR-003"), ["位号"]);
        assert_eq!(of("DR-019"), ["位号"]);
        assert_eq!(of("DR-101"), ["位号"]);
        assert_eq!(of("DR-106"), ["公称直径"]);
        assert_eq!(of("DR-021"), ["流体介质"]);
        assert!(of("DR-014").is_empty());
        let rule = custom(
            "XY-001",
            "阀门",
            "开度 > 下限 OR (所属管线.管道等级 = 开度 AND NOT 位号.长度 > 3) OR 介质 IN (#水, 备用介质)",
            None,
        );
        assert_eq!(fields(&rule), ["开度", "下限", "位号", "介质", "备用介质"]);
    }

    /// 停用 `DR-016` 后结果里没有它、其余照常，重新启用后恢复；停用整个规则集同理。停用的
    /// 不算进 `rules_skipped`。在配好了的主数据下跑，启用的规则都执行。
    #[test]
    fn disabling_a_rule_or_its_set_drops_only_its_results() {
        let drawings = [drawing(
            "PID-1101",
            vec![annotation(&[]), line("L-1001", &[]), valve("", &[])],
        )];
        let rules = builtin();
        let configured = master(Some("GB/T 6567"));
        let checked = |rules: &RuleSet| run_with(rules, &configured, &drawings);
        let all = checked(&rules);
        assert_eq!(hits(&all, "DR-016"), 1);
        assert!(all.results.len() > 1);
        let keep = |prefix: &str| -> Vec<CheckResult> {
            all.results
                .iter()
                .filter(|result| !result.rule_id.starts_with(prefix))
                .cloned()
                .collect()
        };

        let mut changed = rules.clone();
        let at = changed
            .rules
            .iter()
            .position(|rule| rule.id == "DR-016")
            .expect("DR-016");
        changed.rules[at].enabled = false;
        let report = checked(&changed);
        assert_eq!(report.results, keep("DR-016"));
        assert_eq!(report.summary.rules_run, all.summary.rules_run - 1);
        assert_eq!(report.summary.rules_skipped, all.summary.rules_skipped);
        changed.rules[at].enabled = true;
        assert_eq!(checked(&changed).results, all.results);

        let at = changed
            .sets
            .iter()
            .position(|set| set.name == "基础规则")
            .expect("基础规则");
        changed.sets[at].enabled = false;
        let report = checked(&changed);
        assert_eq!(report.results, keep("DR-0"));
        assert_eq!(report.summary.rules_run, all.summary.rules_run - 25);
        changed.sets[at].enabled = true;
        assert_eq!(checked(&changed).results, all.results);
    }

    /// 同一份输入检查两次，规则版本、摘要值和结果相同。
    #[test]
    fn checking_twice_gives_the_same_version_digest_and_results() {
        let rules = builtin();
        let drawings = [drawing(
            "PID-1101",
            vec![
                line("L-1001", &[]),
                pump(""),
                valve("V-101", &[("开度", "120")]),
            ],
        )];
        let first = run(&rules, &drawings);
        let second = run(&rules, &drawings);
        assert!(!first.results.is_empty());
        assert_eq!(first.rule_set_version, "1.0.0");
        assert_eq!(first.rule_set_digest, rule_set_digest(&rules));
        assert_eq!(
            (first.rule_set_version, first.rule_set_digest, first.results),
            (
                second.rule_set_version,
                second.rule_set_digest,
                second.results
            )
        );
    }

    /// 结果按严重度、规则号、图号、对象键排；三个严重度各自计数，有错误就 `blocked`。没配主数据、
    /// 没有项目索引，用到主数据的五条和三条项目级规则不跑，计入 `rules_skipped`（和 §3.6 的例子一样）。
    #[test]
    fn results_are_sorted_and_counted() {
        let drawings = [
            drawing(
                "PID-1102",
                vec![
                    valve("V-201", &[("开度", "120")]),
                    pump(""),
                    line("L-2001", &[("公称直径", "50")]),
                ],
            ),
            drawing(
                "PID-1101",
                vec![
                    annotation(&[]),
                    line("L-1009", &[("公称直径", "50")]),
                    line("L-1001", &[("公称直径", "50")]),
                ],
            ),
        ];
        let report = run(&builtin(), &drawings);
        let order: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object.drawing_id.as_str(),
                    result.object_key.as_str(),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("DR-001", "PID-1101", "PIPE_LINE|L-1001"),
                ("DR-001", "PID-1101", "PIPE_LINE|L-1009"),
                ("DR-001", "PID-1102", "PIPE_LINE|L-2001"),
                ("DR-003", "PID-1102", "PUMP||10.000,20.000"),
                ("DR-009", "PID-1101", "图纸|PID-1101"),
                ("DR-009", "PID-1102", "图纸|PID-1102"),
                ("DR-010", "PID-1101", "PIPE_LINE|L-1001"),
                ("DR-010", "PID-1101", "PIPE_LINE|L-1009"),
                ("DR-010", "PID-1102", "PIPE_LINE|L-2001"),
                ("DR-010", "PID-1102", "PUMP||10.000,20.000"),
                ("DR-017", "PID-1102", "VALVE_GATE|V-201"),
                ("DR-023", "PID-1102", "PUMP||10.000,20.000"),
                ("DR-016", "PID-1101", "ANNOTATION||10.000,20.000"),
            ]
        );
        assert_eq!(
            report.summary,
            Summary {
                errors: 10,
                warnings: 2,
                infos: 1,
                waived: 0,
                objects: 6,
                rules_run: 32,
                rules_skipped: 8,
            }
        );
        assert!(report.blocked);
        let codes: Vec<&str> = report
            .notices
            .iter()
            .map(|notice| notice.code.as_str())
            .collect();
        assert_eq!(
            codes,
            ["MASTERDATA_NOT_CONFIGURED", "PROJECT_RULES_SKIPPED"]
        );
        assert!(report.stale_waivers.is_empty());

        let report = run_with(
            &builtin(),
            &master(Some("GB/T 6567")),
            &[drawing("PID-1101", vec![annotation(&[])])],
        );
        assert_eq!(
            (report.summary.errors, report.summary.infos, report.blocked),
            (0, 1, false)
        );
    }

    /// 作用对象写类型编码时含它的下级类型，不含兄弟类型。
    #[test]
    fn a_type_code_target_takes_its_subtypes() {
        let equipment = |code: &str, label: &str, tag: &str| {
            object(ObjectKind::Equipment, code, label, tag, &[])
        };
        let drawings = [drawing(
            "PID-1101",
            vec![
                equipment("VESSEL", "容器", "V-1"),
                equipment("TANK", "储罐", "T-1"),
                equipment("PUMP", "泵", "P-1"),
            ],
        )];
        let keys = |target: &str| -> Vec<String> {
            let rules = only(vec![custom("XY-001", target, "材质 = NULL", None)]);
            run(&rules, &drawings)
                .results
                .into_iter()
                .map(|result| result.object_key)
                .collect()
        };
        assert_eq!(keys("VESSEL"), ["TANK|T-1", "VESSEL|V-1"]);
        assert_eq!(keys("TANK"), ["TANK|T-1"]);
        assert_eq!(keys("EQUIPMENT"), ["PUMP|P-1", "TANK|T-1", "VESSEL|V-1"]);
        assert_eq!(keys("设备"), ["PUMP|P-1", "TANK|T-1", "VESSEL|V-1"]);
    }

    /// 配置类规则不跑，计入 `rules_skipped`；停用的规则两边都不算。
    #[test]
    fn configuration_rules_are_skipped_and_disabled_rules_are_not_counted() {
        let configuration = Rule {
            category: Category::Configuration,
            ..custom("XY-001", "管线", "管道等级 = NULL", None)
        };
        let disabled = Rule {
            enabled: false,
            ..custom("XY-002", "管线", "管道等级 = NULL", None)
        };
        let rules = only(vec![
            configuration,
            disabled,
            custom("XY-003", "管线", "管道等级 = NULL", None),
        ]);
        let report = run(&rules, &[drawing("PID-1101", vec![line("L-1001", &[])])]);
        assert_eq!(
            (report.summary.rules_run, report.summary.rules_skipped),
            (1, 1)
        );
        let ids: Vec<&str> = report
            .results
            .iter()
            .map(|result| result.rule_id.as_str())
            .collect();
        assert_eq!(ids, ["XY-003"]);
    }

    /// 检查结果写成 JSON 是 §3.6 的样子，字段顺序也照 §3.6。
    #[test]
    fn the_report_serializes_to_the_shape_of_section_3_6() {
        let pipe = SummaryObject {
            at: [0.0, 20.0],
            ..line("L-1003", &[("公称直径", "50"), ("流体介质", "工艺水")])
        };
        let report = run_with(
            &builtin(),
            &master(Some("GB/T 6567")),
            &[drawing("PID-1101", vec![pipe])],
        );
        let value = serde_json::to_value(&report).expect("能写成 JSON");
        assert_eq!(value["version"], 1);
        assert_eq!(value["rule_set_version"], "1.0.0");
        assert_eq!(value["trigger"], "按需");
        assert_eq!(
            value["scope"],
            json!({ "kind": "drawing", "drawing_id": "PID-1101", "pbs": null, "cross_drawing": false })
        );
        assert_eq!(
            value["summary"],
            json!({ "错误": 1, "警告": 0, "提示": 0, "已豁免": 0, "objects": 1, "rules_run": 37, "rules_skipped": 3 })
        );
        assert_eq!(value["blocked"], json!(true));
        assert_eq!(
            value["results"],
            json!([{
                "rule_id": "DR-001", "rule_name": "管线缺管道等级", "severity": "错误",
                "object": { "kind": "管线", "type_code": "PIPE_LINE", "tag": "L-1003", "drawing_id": "PID-1101",
                            "owner": null, "port": null, "at": [0.0, 20.0], "handle": "2D4" },
                "object_key": "PIPE_LINE|L-1003",
                "message": "管线 L-1003 没有管道等级",
                "fix": "给管线填写管道等级：PIDSETPROP L-1003 管道等级 <值>",
                "points": [], "fields": ["管道等级"],
                "waived": false, "waiver": null
            }])
        );
        assert_eq!(
            value["notices"],
            json!([{ "code": "PROJECT_RULES_SKIPPED", "message": "只检查了当前图，3 条项目级规则没有执行" }])
        );
        assert_eq!(value["stale_waivers"], json!([]));

        let text = serde_json::to_string(&report).expect("能写成 JSON");
        let keys = [
            "version",
            "rule_set_version",
            "rule_set_digest",
            "checked_at",
            "trigger",
            "scope",
            "elapsed_ms",
            "summary",
            "blocked",
            "results",
            "notices",
            "stale_waivers",
        ];
        let at: Vec<usize> = keys
            .iter()
            .map(|key| text.find(&format!("\"{key}\":")).expect("有这个键"))
            .collect();
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    }

    /// `位号.同类型重复数` 只数同一张图上类型编码和位号（去掉首尾空白）都相同、有自己编号的
    /// 对象；`位号合规` 按这个类型的位号规则判，类型没有位号规则或对象没有自己编号的不知道；
    /// `位号.长度` 数去掉首尾空白后的字数。
    #[test]
    fn tag_facts_follow_the_type_and_its_tag_rule() {
        let fitting = |tag: &str| object(ObjectKind::Fitting, "FITTING", "管件", tag, &[]);
        let run_of = |tag: &str| {
            object(
                ObjectKind::PipeRun,
                "PIPE_RUN",
                "管路",
                tag,
                &[("管线号", "L-1001")],
            )
        };
        let long = "管".repeat(33);
        let drawings = [drawing(
            "PID-1101",
            vec![
                pump("P-101"),
                pump(" P-101 "),
                tank("P-101"),
                fitting("X 1"),
                fitting("X 1"),
                valve("阀门一号", &[]),
                instrument("TI-101", &[("仪表功能", "PI")]),
                instrument("FIC-101", &[]),
                run_of("L-1001-01"),
                run_of("L-1001"),
                line(&format!(" L-{} ", "1".repeat(30)), &[]),
                line(&long, &[]),
            ],
        )];
        let report = run(&builtin(), &drawings);
        let found = |id: &str| -> Vec<(String, String)> {
            report
                .results
                .iter()
                .filter(|result| result.rule_id == id)
                .map(|result| (result.object_key.clone(), result.message.clone()))
                .collect()
        };
        let duplicated = (
            "PUMP|P-101".to_owned(),
            "泵 的位号 P-101 在本图重复".to_owned(),
        );
        assert_eq!(found("DR-004"), [duplicated.clone(), duplicated]);
        assert_eq!(
            found("DR-018"),
            [
                (
                    "INSTRUMENT|TI-101".to_owned(),
                    "仪表 TI-101 的位号不符合这个类型的位号规则".to_owned(),
                ),
                (
                    format!("PIPE_LINE|{long}"),
                    format!("管线 {long} 的位号不符合这个类型的位号规则"),
                ),
                (
                    "PIPE_RUN|L-1001".to_owned(),
                    "管路 L-1001 的位号不符合这个类型的位号规则".to_owned(),
                ),
            ]
        );
        assert_eq!(
            found("DR-019"),
            [(
                format!("PIPE_LINE|{long}"),
                format!("位号 {long} 超过 32 个字符"),
            )]
        );
    }

    /// `必填项缺失数` 数 `required_fields` 里这个类型和它的上级类型列的属性里空着的；消息和
    /// `fields` 列出缺的属性名，上级类型的在前。
    #[test]
    fn required_fields_include_the_parent_types() {
        let mut rules = builtin();
        rules.required_fields.insert(
            "TANK".to_owned(),
            vec!["容积".to_owned(), "名称".to_owned()],
        );
        let equipment = |code: &str, label: &str, tag: &str, props: &[(&str, &str)]| {
            object(ObjectKind::Equipment, code, label, tag, props)
        };
        let drawings = [drawing(
            "PID-1101",
            vec![
                tank("T-101"),
                equipment(
                    "TANK",
                    "储罐",
                    "T-102",
                    &[("名称", "原料罐"), ("容积", " ")],
                ),
                equipment("VESSEL", "容器", "V-101", &[("名称", "分离器")]),
                line("L-1001", &[("流体介质", "工艺水")]),
                valve("V-201", &[]),
            ],
        )];
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, Vec<&str>)> = report
            .results
            .iter()
            .filter(|result| result.rule_id == "DR-010")
            .map(|result| {
                (
                    result.object_key.as_str(),
                    result.message.as_str(),
                    result.fields.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "TANK|T-101",
                    "储罐 T-101 有 2 个必填属性没填：名称、容积",
                    vec!["名称", "容积"],
                ),
                (
                    "TANK|T-102",
                    "储罐 T-102 有 1 个必填属性没填：容积",
                    vec!["容积"],
                ),
            ]
        );
    }

    /// `管口数` 数图上属于这台设备（类型编码和位号都对上）的端口；储罐不要求两个管口；
    /// 同类型同位号的设备不止一台时分不清，不知道。
    #[test]
    fn equipment_counts_its_own_nozzles() {
        let mut ports = nozzles("PUMP", "P-101", 1);
        ports.extend(nozzles("PUMP", "P-102", 2));
        ports.extend(nozzles("VESSEL", "P-101", 3));
        ports.push(SummaryPort {
            owner_kind: ObjectKind::Instrument,
            ..nozzles("PUMP", "P-101", 1).remove(0)
        });
        let drawings = [DrawingSummary {
            ports,
            ..drawing(
                "PID-1101",
                vec![
                    pump("P-101"),
                    pump("P-102"),
                    tank("T-101"),
                    pump("P-103"),
                    pump("P-103"),
                    pump(""),
                    pump(" "),
                ],
            )
        }];
        let report = run(&builtin(), &drawings);
        let found: Vec<(&str, &str)> = report
            .results
            .iter()
            .filter(|result| result.rule_id == "DR-023")
            .map(|result| (result.object_key.as_str(), result.message.as_str()))
            .collect();
        assert_eq!(found, [("PUMP|P-101", "设备 P-101 只有 1 个管口")]);
    }

    /// 仪表回路是 `仪表回路` 属性相同（去掉首尾空白）的一组仪表；控制器、测量元件按
    /// `loop_roles` 认符号，规则包没写的角色数不出来。结果的对象键是「仪表回路|回路号」。
    #[test]
    fn instrument_loops_group_instruments_by_loop_number() {
        let member = |tag: &str, number: &str, symbol: &str| {
            with_symbol(
                instrument(tag, &[("仪表回路", number)]),
                &format!("PID_SYM_instrument_{symbol}"),
                true,
            )
        };
        let drawings = [drawing(
            "PID-1101",
            vec![
                SummaryObject {
                    at: [5.0, 6.0],
                    ..member("TI-201", "T-201", "indicator")
                },
                member("FT-101", " F-101 ", "transmitter"),
                member("FIC-101", "F-101", "controller"),
                instrument("PI-301", &[]),
                valve("V-101", &[("仪表回路", "T-201")]),
            ],
        )];
        let report = run(&builtin(), &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .filter(|result| ["DR-013", "DR-113"].contains(&result.rule_id.as_str()))
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "DR-113",
                    "仪表回路|T-201",
                    "仪表回路 T-201 缺控制器或测量元件"
                ),
                ("DR-013", "仪表回路|T-201", "仪表回路 T-201 只有 1 个仪表"),
            ]
        );
        let result = report
            .results
            .iter()
            .find(|result| result.rule_id == "DR-013")
            .expect("DR-013 命中");
        assert_eq!(
            result.object,
            ObjectRef {
                kind: "仪表回路".to_owned(),
                type_code: String::new(),
                tag: Some("T-201".to_owned()),
                drawing_id: "PID-1101".to_owned(),
                owner: None,
                port: None,
                at: [5.0, 6.0],
                handle: None,
            }
        );
        assert!(result.fields.is_empty());

        let mut rules = builtin();
        rules.loop_roles.remove("控制器");
        assert_eq!(hits(&run(&rules, &drawings), "DR-113"), 0);
    }

    /// 图纸是整张图：`图例标准` 取项目主数据的，没配置、没写都是空；`图号`、`PBS`、`对象数`
    /// 取摘要的。根 `图纸.图号`、`图纸.PBS`、`参数.<名>` 哪种对象都能用；参数是空表时不知道。
    #[test]
    fn drawings_and_roots_read_the_summary_master_data_and_params() {
        let drawings = [DrawingSummary {
            pbs: Some("A-100".to_owned()),
            ..drawing("PID-1101", vec![line("L-1001", &[]), pump("P-101")])
        }];
        let legend = only(vec![builtin().rule("DR-009").expect("DR-009").clone()]);
        let report = run(&legend, &drawings);
        assert_eq!(report.results.len(), 1);
        let result = &report.results[0];
        assert_eq!(
            result.object,
            ObjectRef {
                kind: "图纸".to_owned(),
                type_code: String::new(),
                tag: None,
                drawing_id: "PID-1101".to_owned(),
                owner: None,
                port: None,
                at: [0.0, 0.0],
                handle: None,
            }
        );
        assert_eq!(result.object_key, "图纸|PID-1101");
        assert_eq!(result.message, "图纸 PID-1101 没有设图例标准");
        let unset = run_with(&legend, &master(None), &drawings);
        assert_eq!(hits(&unset, "DR-009"), 1);
        let report = run_with(&legend, &master(Some("GB/T 6567")), &drawings);
        assert_eq!(hits(&report, "DR-009"), 0);

        let mut limit = custom("XY-004", "管线", "位号.长度 > 参数.上限", None);
        limit.params.insert("上限".to_owned(), json!(5));
        let mut switch = custom("XY-006", "管线", "参数.开关 = TRUE", None);
        switch.params.insert("开关".to_owned(), json!(true));
        let rules = only(vec![
            custom(
                "XY-001",
                "图纸",
                "图号 = \"PID-1101\" AND PBS = \"A-100\" AND 对象数 = 2",
                Some("{图号} {类型}"),
            ),
            custom(
                "XY-002",
                "管线",
                "图纸.图号 = \"PID-1101\" AND 图纸.PBS = \"A-100\"",
                None,
            ),
            custom("XY-003", "图纸", "PBS = NULL", None),
            limit,
            custom("XY-005", "管线", "参数.没有 = NULL", None),
            switch,
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str)> = report
            .results
            .iter()
            .map(|result| (result.rule_id.as_str(), result.object_key.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("XY-001", "图纸|PID-1101"),
                ("XY-002", "PIPE_LINE|L-1001"),
                ("XY-004", "PIPE_LINE|L-1001"),
                ("XY-005", "PIPE_LINE|L-1001"),
                ("XY-006", "PIPE_LINE|L-1001"),
            ]
        );
        assert_eq!(report.results[0].message, "PID-1101 图纸");

        let mut layers = builtin();
        for rule in &mut layers.rules {
            if rule.id == "DR-114" {
                rule.enabled = true;
            }
        }
        assert_eq!(hits(&run(&layers, &drawings), "DR-114"), 0);
    }

    /// 主数据没配置或读不出来：用到主数据的规则（内置的 `DR-007`、`021`、`022`、`103`、`104`，
    /// 和经主数据的表或关系取值的自定规则）不跑，计入 `rules_skipped`，`notices` 里说明有几条
    /// 没跑；`DR-009` 照常命中，只用 `项目.图例标准` 的照常跑。停用的不算；没有规则因此没跑时
    /// 不给说明。
    #[test]
    fn rules_that_need_master_data_are_skipped_without_it() {
        let drawings = [drawing(
            "PID-1101",
            vec![
                line(
                    "L-1001",
                    &[
                        ("管道等级", "C3"),
                        ("公称直径", "DN100"),
                        ("流体介质", "氨"),
                    ],
                ),
                line("L-1002", &[("管道等级", "A1"), ("流体介质", "氯气")]),
                pipe_run("L-1002-01", "L-1002", &[]),
                on_run("L-1002-01", valve("V-101", &[("压力等级", "40")])),
            ],
        )];
        let needs_master = ["DR-007", "DR-021", "DR-022", "DR-103", "DR-104"];
        let report = run_across(&builtin(), &master(Some("GB/T 6567")), &drawings, &[]);
        for id in needs_master {
            assert_eq!(hits(&report, id), 1, "{id}");
        }
        assert_eq!(
            (report.summary.rules_run, report.summary.rules_skipped),
            (40, 0)
        );
        assert!(report.notices.is_empty());

        let unreadable = master_data::Resolved::Unreadable {
            source: master_data::Source::Drawing,
            error: master_data::MasterDataError::RequiredFieldMissing {
                at: "pipe_classes[0].name".to_owned(),
            },
        };
        for (resolved, code, message) in [
            (
                master_data::Resolved::NotConfigured,
                "MASTERDATA_NOT_CONFIGURED",
                "项目主数据没有配置，用到主数据的 5 条规则没有执行",
            ),
            (
                unreadable,
                "MASTERDATA_UNREADABLE",
                "项目主数据读不出来（REQUIRED_FIELD_MISSING: pipe_classes[0].name 不能为空），用到主数据的 5 条规则没有执行",
            ),
        ] {
            let report = run_across(&builtin(), &resolved, &drawings, &[]);
            for id in needs_master {
                assert_eq!(hits(&report, id), 0, "{id}");
            }
            assert_eq!(hits(&report, "DR-009"), 1);
            assert_eq!(
                (report.summary.rules_run, report.summary.rules_skipped),
                (35, 5)
            );
            assert_eq!(
                report.notices,
                [Notice {
                    code: code.to_owned(),
                    message: message.to_owned(),
                }]
            );
        }

        let rules = only(vec![
            custom("XY-001", "管线", "公称直径 < 管道等级.公称直径下限", None),
            custom("XY-002", "阀门", "所属管线.流体介质.相态 = #气", None),
            custom("XY-003", "管线", "项目.图例标准 = NULL", None),
            Rule {
                enabled: false,
                ..custom("XY-004", "管线", "管道等级 NOT IN (项目.管道等级表)", None)
            },
        ]);
        let report = run(&rules, &drawings);
        assert_eq!(
            (report.summary.rules_run, report.summary.rules_skipped),
            (1, 2)
        );
        assert_eq!(hits(&report, "XY-003"), 2);
        assert_eq!(
            report.notices[0].message,
            "项目主数据没有配置，用到主数据的 2 条规则没有执行"
        );
        let plain = only(vec![custom("XY-005", "管线", "管道等级 = NULL", None)]);
        let report = run(&plain, &drawings);
        assert_eq!(report.summary.rules_skipped, 0);
        assert!(report.notices.is_empty());
    }

    /// 根 `项目.…` 读项目主数据：名称表去掉首尾空白后逐字比，区分大小写，不当数读；公称直径表
    /// 按主数据规格 §3.5 读成数再比，隐藏的、读不出数的不在表里。配好了而表是空的，取值都不在
    /// 表里。`项目.图例标准` 是主数据的图例标准，没配置时是 `NULL`。
    #[test]
    fn project_roots_read_the_master_data_tables() {
        let rules = only(vec![
            custom("XY-001", "管线", "管道等级 NOT IN (项目.管道等级表)", None),
            custom("XY-002", "管线", "公称直径 NOT IN (项目.公称直径表)", None),
            custom("XY-003", "管线", "绝热等级 NOT IN (项目.绝热等级表)", None),
            custom("XY-004", "管线", "流体介质 NOT IN (项目.流体介质表)", None),
        ]);
        let found = |resolved: &master_data::Resolved, props: &[(&str, &str)]| -> Vec<String> {
            let drawings = [drawing("PID-1101", vec![line("L-1001", props)])];
            run_with(&rules, resolved, &drawings)
                .results
                .into_iter()
                .map(|result| result.rule_id)
                .collect()
        };
        let listed = [
            ("管道等级", " A1 "),
            ("公称直径", "dn 50"),
            ("绝热等级", "H1"),
            ("流体介质", "工艺水"),
        ];
        let all = ["XY-001", "XY-002", "XY-003", "XY-004"];
        let configured = master(Some("GB/T 6567"));
        assert!(found(&configured, &listed).is_empty());
        assert!(found(&configured, &[("公称直径", "80")]).is_empty());
        let unlisted = [
            ("管道等级", "a1"),
            ("公称直径", "DN65"),
            ("绝热等级", "h1"),
            ("流体介质", "水"),
        ];
        assert_eq!(found(&configured, &unlisted), all);
        assert_eq!(found(&configured, &[("公称直径", "五十")]), ["XY-002"]);
        let numbered = master_with(|data| data.insulation_classes.push(entry("1", &[])));
        assert!(found(&numbered, &[("绝热等级", "1")]).is_empty());
        assert_eq!(found(&numbered, &[("绝热等级", "01")]), ["XY-003"]);
        let empty = master_with(|data| {
            data.nominal_diameters.clear();
            data.pipe_classes.clear();
            data.insulation_classes.clear();
            data.fluid_codes.clear();
        });
        assert_eq!(found(&empty, &listed), all);

        let legend = only(vec![
            custom("XY-005", "管线", "项目.图例标准 = \"GB/T 6567\"", None),
            custom("XY-006", "管线", "项目.图例标准 = NULL", None),
        ]);
        let drawings = [drawing("PID-1101", vec![line("L-1001", &[])])];
        let ids = |resolved: &master_data::Resolved| -> Vec<String> {
            run_with(&legend, resolved, &drawings)
                .results
                .into_iter()
                .map(|result| result.rule_id)
                .collect()
        };
        assert_eq!(ids(&configured), ["XY-005"]);
        assert_eq!(ids(&master(None)), ["XY-006"]);
        assert_eq!(ids(&master_data::Resolved::NotConfigured), ["XY-006"]);
    }

    /// `管道等级.<扩展属性>`、`绝热等级.<…>`、`流体介质.<…>`：按对象的同名属性（去掉首尾空白）在
    /// 主数据里找名称逐字相同的那一行，取它的扩展属性；能接在 `所属管线` 等关系后面。属性没填、
    /// 主数据里没有这一行、这一行没有这个扩展属性时是 `NULL`。
    #[test]
    fn master_data_relations_read_the_entry_attributes() {
        let rules = only(vec![
            custom("XY-001", "管线", "管道等级.材质 = #碳钢", None),
            custom("XY-002", "管线", "绝热等级.厚度 > 40", None),
            custom("XY-003", "管线", "流体介质.相态 = #气", None),
            custom(
                "XY-004",
                "阀门",
                "所属管线.管道等级.最大公称压力 = 16",
                None,
            ),
            custom("XY-005", "管线", "管道等级.材质 = NULL", None),
            custom("XY-006", "管线", "管道等级.口径 = NULL", None),
        ]);
        let found = |props: &[(&str, &str)]| -> Vec<String> {
            let objects = vec![
                line("L-1001", props),
                pipe_run("L-1001-01", "L-1001", &[]),
                on_run("L-1001-01", valve("V-101", &[])),
            ];
            run_with(&rules, &master(None), &[drawing("PID-1101", objects)])
                .results
                .into_iter()
                .map(|result| format!("{} {}", result.rule_id, result.object_key))
                .collect()
        };
        assert_eq!(
            found(&[
                ("管道等级", " A1 "),
                ("绝热等级", "H1"),
                ("流体介质", "蒸汽")
            ]),
            [
                "XY-001 PIPE_LINE|L-1001",
                "XY-002 PIPE_LINE|L-1001",
                "XY-003 PIPE_LINE|L-1001",
                "XY-004 VALVE_GATE|V-101",
                "XY-006 PIPE_LINE|L-1001",
            ]
        );
        assert_eq!(
            found(&[("管道等级", "B2"), ("流体介质", "工艺水")]),
            ["XY-006 PIPE_LINE|L-1001"]
        );
        for class in ["a1", "C3", " "] {
            assert_eq!(
                found(&[("管道等级", class)]),
                ["XY-005 PIPE_LINE|L-1001", "XY-006 PIPE_LINE|L-1001"],
                "{class}"
            );
        }
    }

    /// 管线、管路名下的线段：长度、段数、条数、没接上的端点（接到打断符的不算）。
    /// `DR-014` 一条管线出一条结果，`points` 列出三个悬空端点；`DR-011` 看管路位号的后缀。
    #[test]
    fn pipe_lines_and_runs_add_up_their_segments_and_loose_ends() {
        let own = SummarySegment {
            joint_b: Some(Joint::Break),
            ..segment("L-1001", None, [0.0, 0.0], [10.0, 0.0])
        };
        let first = SummarySegment {
            joint_a: joined(None, false),
            ..segment("L-1001", Some("L-1001-01"), [10.0, 0.0], [20.0, 0.0])
        };
        let branch = SummarySegment {
            joint_a: joined(None, true),
            ..segment("L-1001", Some("L-1001-01"), [5.0, 0.0], [5.0, 10.0])
        };
        let elsewhere = segment("L-3001", None, [50.0, 0.0], [60.0, 0.0]);
        let drawings = [piping(
            vec![
                line("L-1001", &[]),
                pipe_run("L-1001-01", "L-1001", &[]),
                pipe_run("L-1001-02", "L-1001", &[]),
                pipe_run("L-1001-", "L-1001", &[]),
                pipe_run("L-100103", "L-1001", &[]),
                SummaryObject {
                    owner: None,
                    ..pipe_run("L-1001-04", "L-1001", &[])
                },
            ],
            vec![own, first, branch, elsewhere],
        )];
        let rules = only(vec![
            builtin().rule("DR-011").expect("DR-011").clone(),
            builtin().rule("DR-014").expect("DR-014").clone(),
            custom(
                "XY-001",
                "管线",
                "长度 >= 0",
                Some("{位号} 长 {长度}，{管路数} 条管路，{端口未连接数} 个端点没接上"),
            ),
            custom(
                "XY-002",
                "管路",
                "管段数 >= 0",
                Some("{位号} 后缀 {后缀}，{管段数} 段，长 {长度}，{端口未连接数} 个端点没接上"),
            ),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("DR-011", "PIPE_RUN|L-1001-", "管路 L-1001- 的位号没有后缀"),
                (
                    "DR-011",
                    "PIPE_RUN|L-1001-04",
                    "管路 L-1001-04 的位号没有后缀"
                ),
                (
                    "DR-011",
                    "PIPE_RUN|L-100103",
                    "管路 L-100103 的位号没有后缀"
                ),
                (
                    "DR-014",
                    "PIPE_LINE|L-1001",
                    "管线 L-1001 有 3 个端点没接上"
                ),
                (
                    "XY-001",
                    "PIPE_LINE|L-1001",
                    "L-1001 长 30，4 条管路，3 个端点没接上",
                ),
                (
                    "XY-002",
                    "PIPE_RUN|L-1001-",
                    "L-1001- 后缀 ，0 段，长 0，0 个端点没接上",
                ),
                (
                    "XY-002",
                    "PIPE_RUN|L-1001-01",
                    "L-1001-01 后缀 01，2 段，长 20，2 个端点没接上",
                ),
                (
                    "XY-002",
                    "PIPE_RUN|L-1001-02",
                    "L-1001-02 后缀 02，0 段，长 0，0 个端点没接上",
                ),
                (
                    "XY-002",
                    "PIPE_RUN|L-1001-04",
                    "L-1001-04 后缀 ，0 段，长 0，0 个端点没接上",
                ),
                (
                    "XY-002",
                    "PIPE_RUN|L-100103",
                    "L-100103 后缀 ，0 段，长 0，0 个端点没接上",
                ),
            ]
        );
        let loose = &report.results[3];
        assert_eq!(loose.points, [[0.0, 0.0], [20.0, 0.0], [5.0, 10.0]]);
        assert!(loose.fields.is_empty());
        assert!(report.results[4].points.is_empty());
    }

    /// 管段：两端距离（不超过容差的算 0）、所属、重叠的其他线段。共线且重叠超过容差才算，
    /// 首尾相接、平行错开的不算。结果指到管段：没有位号，靠所属和起点定位。
    #[test]
    fn segments_have_a_length_an_owner_and_overlaps() {
        let drawings = [piping(
            vec![line("L-1001", &[])],
            vec![
                segment("L-1001", None, [0.0, 0.0], [10.0, 0.0]),
                segment("L-1001", None, [15.0, 0.0], [5.0, 0.0]),
                segment("L-1001", None, [0.0, 1.0], [10.0, 1.0]),
                segment("L-1001", None, [10.0, 0.0], [20.0, 0.0]),
                segment(
                    "L-1001",
                    Some("L-1001-01"),
                    [30.0, 0.0],
                    [30.0, 0.000_000_5],
                ),
            ],
        )];
        let rules = only(vec![
            builtin().rule("DR-012").expect("DR-012").clone(),
            builtin().rule("DR-110").expect("DR-110").clone(),
            custom(
                "XY-001",
                "管段",
                "长度 >= 0",
                Some(
                    "{所属} {类型} 长 {长度}，{端口未连接数} 个端点没接上，与 {重叠管段数} 段重叠",
                ),
            ),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "DR-012",
                    "PIPE_SEGMENT|L-1001-01|30.000,0.000",
                    "L-1001-01 上有一段管段长度为 0",
                ),
                (
                    "DR-110",
                    "PIPE_LINE|L-1001|0.000,0.000",
                    "L-1001 上有管段与别的管段重叠",
                ),
                (
                    "DR-110",
                    "PIPE_LINE|L-1001|10.000,0.000",
                    "L-1001 上有管段与别的管段重叠",
                ),
                (
                    "DR-110",
                    "PIPE_LINE|L-1001|15.000,0.000",
                    "L-1001 上有管段与别的管段重叠",
                ),
                (
                    "XY-001",
                    "PIPE_LINE|L-1001|0.000,0.000",
                    "L-1001 管段 长 10，2 个端点没接上，与 1 段重叠",
                ),
                (
                    "XY-001",
                    "PIPE_LINE|L-1001|0.000,1.000",
                    "L-1001 管段 长 10，2 个端点没接上，与 0 段重叠",
                ),
                (
                    "XY-001",
                    "PIPE_LINE|L-1001|10.000,0.000",
                    "L-1001 管段 长 10，2 个端点没接上，与 1 段重叠",
                ),
                (
                    "XY-001",
                    "PIPE_LINE|L-1001|15.000,0.000",
                    "L-1001 管段 长 10，2 个端点没接上，与 2 段重叠",
                ),
                (
                    "XY-001",
                    "PIPE_SEGMENT|L-1001-01|30.000,0.000",
                    "L-1001-01 管段 长 0，2 个端点没接上，与 0 段重叠",
                ),
            ]
        );
        assert_eq!(
            report.results[0].object,
            ObjectRef {
                kind: "管段".to_owned(),
                type_code: "PIPE_SEGMENT".to_owned(),
                tag: None,
                drawing_id: "PID-1101".to_owned(),
                owner: Some("L-1001-01".to_owned()),
                port: None,
                at: [30.0, 0.0],
                handle: Some("2E0".to_owned()),
            }
        );
    }

    /// `直连异径管路数`：这条管路的线段端点直接接在别的管路的线段端点上、公称直径不同。
    /// 接在中间、隔着管件、写法不同但直径相等、没填直径的都不算。`points` 是相接的地方。
    #[test]
    fn runs_of_different_sizes_meeting_end_to_end_need_a_reducer() {
        let piece = |run, a, b| segment("L-1001", Some(run), a, b);
        let fitting = || {
            Some(Joint::Fitting {
                type_code: "FITTING".to_owned(),
                tag: "L-1001-05".to_owned(),
                port: "连接点1".to_owned(),
            })
        };
        let drawings = [piping(
            vec![
                pipe_run("L-1001-01", "L-1001", &[("公称直径", "50")]),
                pipe_run("L-1001-02", "L-1001", &[("公称直径", "80")]),
                pipe_run("L-1001-03", "L-1001", &[("公称直径", "DN80")]),
                pipe_run("L-1001-04", "L-1001", &[("公称直径", "100")]),
                pipe_run("L-1001-05", "L-1001", &[]),
                pipe_run("L-1001-06", "L-1001", &[("公称直径", "25")]),
            ],
            vec![
                SummarySegment {
                    joint_b: joined(Some("L-1001-02"), false),
                    ..piece("L-1001-01", [0.0, 0.0], [10.0, 0.0])
                },
                SummarySegment {
                    joint_a: joined(Some("L-1001-01"), false),
                    joint_b: joined(Some("L-1001-03"), false),
                    ..piece("L-1001-02", [10.0, 0.0], [20.0, 0.0])
                },
                SummarySegment {
                    joint_a: joined(Some("L-1001-02"), false),
                    joint_b: joined(Some("L-1001-05"), false),
                    ..piece("L-1001-03", [20.0, 0.0], [30.0, 0.0])
                },
                SummarySegment {
                    joint_a: joined(Some("L-1001-03"), true),
                    ..piece("L-1001-04", [25.0, 0.0], [25.0, 10.0])
                },
                SummarySegment {
                    joint_a: joined(Some("L-1001-03"), false),
                    joint_b: fitting(),
                    ..piece("L-1001-05", [30.0, 0.0], [40.0, 0.0])
                },
                SummarySegment {
                    joint_a: fitting(),
                    ..piece("L-1001-06", [40.0, 0.0], [50.0, 0.0])
                },
            ],
        )];
        let rules = only(vec![builtin().rule("DR-105").expect("DR-105").clone()]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, Vec<[f64; 2]>)> = report
            .results
            .iter()
            .map(|result| (result.object_key.as_str(), result.points.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("PIPE_RUN|L-1001-01", vec![[10.0, 0.0]]),
                ("PIPE_RUN|L-1001-02", vec![[10.0, 0.0]]),
            ]
        );
        assert_eq!(
            report.results[0].message,
            "管路 L-1001-01 与公称直径不同的管路直接相接"
        );
    }

    /// `未标记覆盖数`：五个管级属性里，管路的值与所属管线不同、又没有覆盖标记的个数；
    /// 没填的当空，去掉首尾空白再比；所属管线不在图上时不知道。
    #[test]
    fn run_properties_that_differ_from_the_line_need_an_override_mark() {
        let same = [
            ("管道等级", "A1"),
            ("公称直径", "50"),
            ("流体介质", "工艺水"),
        ];
        let changed = [
            ("管道等级", "B2"),
            ("公称直径", "50"),
            ("流体介质", "工艺水"),
        ];
        let drawings = [piping(
            vec![
                line("L-1001", &same),
                pipe_run("L-1001-01", "L-1001", &changed),
                SummaryObject {
                    overrides: vec!["管道等级".to_owned()],
                    ..pipe_run("L-1001-02", "L-1001", &changed)
                },
                pipe_run(
                    "L-1001-03",
                    "L-1001",
                    &[
                        ("管道等级", "A1"),
                        ("公称直径", " "),
                        ("绝热等级", "H1"),
                        ("流体介质", "工艺水"),
                    ],
                ),
                pipe_run(
                    "L-1001-04",
                    "L-1001",
                    &[
                        ("管道等级", " A1 "),
                        ("公称直径", "50"),
                        ("流体介质", "工艺水"),
                        ("设计温度", "80"),
                    ],
                ),
                pipe_run("L-2001-01", "L-2001", &changed),
            ],
            Vec::new(),
        )];
        let rules = only(vec![builtin().rule("DR-115").expect("DR-115").clone()]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str)> = report
            .results
            .iter()
            .map(|result| (result.object_key.as_str(), result.message.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "PIPE_RUN|L-1001-01",
                    "管路 L-1001-01 有 1 个管级属性与所属管线不同，却没有覆盖标记",
                ),
                (
                    "PIPE_RUN|L-1001-03",
                    "管路 L-1001-03 有 2 个管级属性与所属管线不同，却没有覆盖标记",
                ),
            ]
        );
    }

    /// `所属管路`、`所属管线`：管段到它的管路和管线，管路到它的管线，线上的阀门经所属管路
    /// 到管线；关系可以连用；图上找不到时不知道。
    #[test]
    fn owner_relations_lead_to_the_run_and_the_line() {
        let drawings = [piping(
            vec![
                line("L-1001", &[("管道等级", "A1")]),
                pipe_run("L-1001-01", "L-1001", &[("公称直径", "50")]),
                SummaryObject {
                    owner: Some("L-1001-01".to_owned()),
                    ..valve("V-101", &[("公称直径", "80")])
                },
                SummaryObject {
                    owner: Some("L-1009-01".to_owned()),
                    ..valve("V-102", &[("公称直径", "80")])
                },
            ],
            vec![
                segment("L-1001", None, [0.0, 0.0], [10.0, 0.0]),
                segment("L-1001", Some("L-1001-01"), [10.0, 0.0], [20.0, 0.0]),
            ],
        )];
        let rules = only(vec![
            custom("XY-001", "管段", "所属管路.公称直径 = 50", None),
            custom("XY-002", "管段", "所属管线.管道等级 = \"A1\"", None),
            custom("XY-003", "管路", "所属管线.管道等级 = \"A1\"", None),
            custom("XY-004", "阀门", "公称直径 != 所属管路.公称直径", None),
            custom("XY-005", "阀门", "所属管线.管道等级 = \"A1\"", None),
            custom(
                "XY-006",
                "管段",
                "所属管路.所属管线.管道等级 = \"A1\"",
                None,
            ),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str)> = report
            .results
            .iter()
            .map(|result| (result.rule_id.as_str(), result.object_key.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("XY-001", "PIPE_SEGMENT|L-1001-01|10.000,0.000"),
                ("XY-002", "PIPE_LINE|L-1001|0.000,0.000"),
                ("XY-002", "PIPE_SEGMENT|L-1001-01|10.000,0.000"),
                ("XY-003", "PIPE_RUN|L-1001-01"),
                ("XY-004", "VALVE_GATE|V-101"),
                ("XY-005", "VALVE_GATE|V-101"),
                ("XY-006", "PIPE_SEGMENT|L-1001-01|10.000,0.000"),
            ]
        );
    }

    /// 管口是设备符号的端口：名称、方向、连接状态取摘要的端口，位号前缀本期不知道，`DR-020` 不出
    /// 结果。关系 `所属设备` 走到类型编码和位号都对上的那台设备；同类型同位号的不止一台、图上没有
    /// 这台设备时走不到。结果指到管口：没有位号，靠设备位号和端口名定位，句柄是设备的。
    #[test]
    fn nozzles_are_the_ports_of_equipment() {
        let feed = pump("P-101");
        let ports = vec![
            SummaryPort {
                direction: Some(PortDir::In),
                at: [-14.0, 0.0],
                ..port_of(&feed, "in1", false)
            },
            SummaryPort {
                direction: Some(PortDir::Out),
                at: [14.0, 0.0],
                ..port_of(&feed, "out1", true)
            },
            SummaryPort {
                direction: Some(PortDir::Bidir),
                ..port_of(&pump("P-103"), "N1", false)
            },
            port_of(&pump("P-109"), "N1", true),
            port_of(&instrument("FT-101", &[]), "in1", false),
        ];
        let drawings = [DrawingSummary {
            ports,
            ..drawing(
                "PID-1101",
                vec![
                    feed.clone(),
                    pump("P-103"),
                    pump("P-103"),
                    instrument("FT-101", &[]),
                ],
            )
        }];
        let rules = only(vec![
            builtin().rule("DR-008").expect("DR-008").clone(),
            builtin().rule("DR-020").expect("DR-020").clone(),
            custom(
                "XY-001",
                "管口",
                "位号前缀 = NULL",
                Some("{类型} {所属}#{名称}：{方向}，{连接状态}"),
            ),
            custom("XY-002", "管口", "所属设备.位号 = \"P-101\"", None),
            custom("XY-003", "管口", "所属设备.位号 = NULL", None),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("DR-008", "PUMP|P-101#in1", "P-101 的管口 in1 没有接上"),
                ("DR-008", "PUMP|P-103#N1", "P-103 的管口 N1 没有接上"),
                ("XY-001", "PUMP|P-101#in1", "管口 P-101#in1：流入，未连接"),
                ("XY-001", "PUMP|P-101#out1", "管口 P-101#out1：流出，已连接"),
                ("XY-001", "PUMP|P-103#N1", "管口 P-103#N1：双向，未连接"),
                ("XY-001", "PUMP|P-109#N1", "管口 P-109#N1：，已连接"),
                ("XY-002", "PUMP|P-101#in1", "自定规则：P-101#in1"),
                ("XY-002", "PUMP|P-101#out1", "自定规则：P-101#out1"),
                ("XY-003", "PUMP|P-103#N1", "自定规则：P-103#N1"),
                ("XY-003", "PUMP|P-109#N1", "自定规则：P-109#N1"),
            ]
        );
        assert_eq!(
            report.results[0].object,
            ObjectRef {
                kind: "管口".to_owned(),
                type_code: "PUMP".to_owned(),
                tag: None,
                drawing_id: "PID-1101".to_owned(),
                owner: Some("P-101".to_owned()),
                port: Some("in1".to_owned()),
                at: [-14.0, 0.0],
                handle: Some("2D4".to_owned()),
            }
        );
        assert_eq!(report.results[1].object.handle, None);
    }

    /// 信号线的起点仪表、终点仪表：属性里有就用属性；都没有时看两端接在哪个仪表的端口上，只接着
    /// 一个时终点仪表不知道。端口未连接数数两端里什么都没接的（接到打断符的不算），`points` 是这些
    /// 端点；接到设备管口数数接在设备端口上的。图上同位号的信号线不止一根时看不了端点，都不知道。
    #[test]
    fn signal_lines_find_their_instruments_from_props_or_ends() {
        let transmitter = instrument("FT-101", &[]);
        let controller = instrument("FIC-101", &[]);
        let equipment = pump("P-101");
        let drawings = [DrawingSummary {
            signal_ends: vec![
                signal_end("S-1", [0.0, 0.0], None),
                signal_end("S-1", [10.0, 0.0], None),
                signal_end("S-2", [0.0, 10.0], on_port(&transmitter, "out1")),
                signal_end("S-2", [10.0, 10.0], on_port(&controller, "in1")),
                signal_end("S-3", [0.0, 20.0], None),
                signal_end("S-3", [10.0, 20.0], on_port(&controller, "in1")),
                signal_end("S-4", [0.0, 30.0], on_port(&equipment, "out1")),
                signal_end("S-4", [10.0, 30.0], Some(Joint::Break)),
                signal_end("S-5", [0.0, 40.0], None),
                signal_end("S-5", [10.0, 40.0], None),
                signal_end("S-6", [0.0, 50.0], None),
            ],
            ports: vec![
                port_of(&transmitter, "out1", true),
                port_of(&controller, "in1", true),
                port_of(&equipment, "out1", true),
            ],
            ..drawing(
                "PID-1101",
                vec![
                    signal("S-1", &[("起点仪表", "FT-101"), ("终点仪表", "FIC-101")]),
                    signal("S-2", &[]),
                    signal("S-3", &[]),
                    signal("S-4", &[]),
                    signal("S-5", &[("起点仪表", "FT-109")]),
                    signal("S-6", &[]),
                    signal("S-6", &[]),
                    transmitter.clone(),
                    controller.clone(),
                    equipment.clone(),
                ],
            )
        }];
        let rules = only(vec![
            builtin().rule("DR-015").expect("DR-015").clone(),
            builtin().rule("DR-108").expect("DR-108").clone(),
            custom(
                "XY-001",
                "信号线",
                "端口未连接数 >= 0",
                Some("{位号}：{起点仪表}→{终点仪表}，悬空 {端口未连接数}，接设备 {接到设备管口数}"),
            ),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "DR-015",
                    "SIGNAL_LINE|S-3",
                    "信号线 S-3 没有两端都接到仪表上"
                ),
                (
                    "DR-015",
                    "SIGNAL_LINE|S-4",
                    "信号线 S-4 没有两端都接到仪表上"
                ),
                (
                    "DR-015",
                    "SIGNAL_LINE|S-5",
                    "信号线 S-5 没有两端都接到仪表上"
                ),
                (
                    "DR-015",
                    "SIGNAL_LINE|S-6",
                    "信号线 S-6 没有两端都接到仪表上"
                ),
                (
                    "DR-015",
                    "SIGNAL_LINE|S-6",
                    "信号线 S-6 没有两端都接到仪表上"
                ),
                ("DR-108", "SIGNAL_LINE|S-4", "信号线 S-4 接到了设备管口上"),
                (
                    "XY-001",
                    "SIGNAL_LINE|S-1",
                    "S-1：FT-101→FIC-101，悬空 2，接设备 0"
                ),
                (
                    "XY-001",
                    "SIGNAL_LINE|S-2",
                    "S-2：FT-101→FIC-101，悬空 0，接设备 0"
                ),
                (
                    "XY-001",
                    "SIGNAL_LINE|S-3",
                    "S-3：FIC-101→，悬空 1，接设备 0"
                ),
                ("XY-001", "SIGNAL_LINE|S-4", "S-4：→，悬空 0，接设备 1"),
                (
                    "XY-001",
                    "SIGNAL_LINE|S-5",
                    "S-5：FT-109→，悬空 2，接设备 0"
                ),
            ]
        );
        let points: Vec<Vec<[f64; 2]>> = report
            .results
            .iter()
            .filter(|result| result.rule_id == "XY-001")
            .map(|result| result.points.clone())
            .collect();
        assert_eq!(
            points,
            [
                vec![[0.0, 0.0], [10.0, 0.0]],
                vec![],
                vec![[0.0, 20.0]],
                vec![],
                vec![[0.0, 40.0], [10.0, 40.0]],
            ]
        );
    }

    /// `未连接端口数` 数这个符号的端口里没接上的，按种类、类型编码和位号认端口；图上同种同类型
    /// 同位号的符号不止一个（同一条管路上两个同类型的管件）时分不清，不带符号的也不知道。
    /// `DR-111` 只查阀门和管件。
    #[test]
    fn symbols_count_their_unconnected_ports() {
        let gate = with_symbol(valve("V-101", &[]), "PID_SYM_valve_gate_valve", true);
        let transmitter = with_symbol(
            fitting("L-1001-01", &[]),
            "PID_SYM_instrument_transmitter",
            true,
        );
        let feed = with_symbol(pump("P-101"), "PID_SYM_equipment_pump", true);
        let mut ports = vec![
            port_of(&gate, "in1", true),
            port_of(&gate, "out1", false),
            port_of(&feed, "in1", false),
            port_of(&feed, "out1", false),
            SummaryPort {
                owner_kind: ObjectKind::Equipment,
                ..port_of(&gate, "in1", false)
            },
        ];
        for _ in 0..2 {
            ports.push(port_of(&transmitter, "in1", false));
            ports.push(port_of(&transmitter, "out1", true));
        }
        let drawings = [DrawingSummary {
            ports,
            ..drawing(
                "PID-1101",
                vec![
                    gate.clone(),
                    transmitter.clone(),
                    transmitter.clone(),
                    feed.clone(),
                    valve("V-102", &[]),
                ],
            )
        }];
        let rules = only(vec![
            builtin().rule("DR-111").expect("DR-111").clone(),
            custom(
                "XY-001",
                "对象实例",
                "未连接端口数 >= 0",
                Some("{类型} {位号}：{未连接端口数}"),
            ),
            custom("XY-002", "对象实例", "未连接端口数 = NULL", None),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("DR-111", "VALVE_GATE|V-101", "闸阀 V-101 有 1 个端口没接上"),
                ("XY-001", "PUMP|P-101", "泵 P-101：2"),
                ("XY-001", "VALVE_GATE|V-101", "闸阀 V-101：1"),
                (
                    "XY-002",
                    "FITTING|L-1001-01|10.000,20.000",
                    "自定规则：L-1001-01",
                ),
                (
                    "XY-002",
                    "FITTING|L-1001-01|10.000,20.000",
                    "自定规则：L-1001-01",
                ),
                ("XY-002", "VALVE_GATE|V-102", "自定规则：V-102"),
            ]
        );
    }

    /// 线上的阀门、管件经所属管路、所属管线取属性：阀门的公称直径与所在管路不同、阀门或管件的
    /// 管道等级与所在管线不同才命中；阀门没填直径、不在线上的不命中。
    #[test]
    fn valves_and_fittings_follow_their_run_and_line() {
        let drawings = [drawing(
            "PID-1101",
            vec![
                line("L-1001", &[("管道等级", "A1")]),
                pipe_run("L-1001-01", "L-1001", &[("公称直径", "50")]),
                on_run(
                    "L-1001-01",
                    valve("V-101", &[("公称直径", "80"), ("管道等级", "A1")]),
                ),
                on_run("L-1001-01", valve("V-102", &[("管道等级", "B2")])),
                on_run(
                    "L-1001-01",
                    valve("V-103", &[("公称直径", "50"), ("管道等级", "A1")]),
                ),
                fitting("L-1001-01", &[("管道等级", "B2")]),
                valve("V-104", &[("公称直径", "80"), ("管道等级", "B2")]),
            ],
        )];
        let rules = only(vec![
            builtin().rule("DR-106").expect("DR-106").clone(),
            builtin().rule("DR-107").expect("DR-107").clone(),
        ]);
        let report = run(&rules, &drawings);
        let found: Vec<(&str, &str, &str)> = report
            .results
            .iter()
            .map(|result| {
                (
                    result.rule_id.as_str(),
                    result.object_key.as_str(),
                    result.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "DR-106",
                    "VALVE_GATE|V-101",
                    "阀门 V-101 的公称直径 80 与所在管路不一致",
                ),
                (
                    "DR-107",
                    "FITTING|L-1001-01|10.000,20.000",
                    "管件 L-1001-01 的管道等级 B2 与所在管线不一致",
                ),
                (
                    "DR-107",
                    "VALVE_GATE|V-102",
                    "闸阀 V-102 的管道等级 B2 与所在管线不一致",
                ),
            ]
        );
    }

    /// 协议 1 的一条控制请求，发给当前的图。
    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        req["request_id"] = json!(format!("drc-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
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

    /// 用命令建的图上 `DR-020`、`DR-109` 不出结果：管口没有位号，库里的阀门不带方向。同一张图上
    /// `DR-008` 照样查出没接上的管口，阀门也在摘要里。
    #[test]
    fn nozzle_prefixes_and_valve_flow_say_nothing_on_a_drawing_built_by_commands() {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDINSERT vessel", &[[200.0, 12.0]]);
        draw(&mut app, "PIDPIPE", &[[200.0, 100.0], [300.0, 100.0]]);
        draw(&mut app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
        draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
        let summary = summarize(
            app.active_document_for_test(),
            &app.pid_symbols,
            SummaryMeta {
                drawing_id: "PID-1101".to_owned(),
                file: None,
                pbs: None,
                source: SummarySource::Open,
                fingerprint: None,
            },
            DEFAULT_TOLERANCE,
        );
        assert!(summary
            .objects
            .iter()
            .any(|object| object.kind == ObjectKind::Valve));
        let report = run(&builtin(), &[summary]);
        assert!(hits(&report, "DR-008") > 0, "{:#?}", report.results);
        assert_eq!(hits(&report, "DR-020"), 0);
        assert_eq!(hits(&report, "DR-109"), 0);
    }

    /// 契约规模：2,000 个对象的合成图（`IX-4`）。
    const SYNTHETIC_OBJECTS: usize = 2000;
    /// 预算：单图全量检查（含做摘要）不超过 500 ms（成功标准第 12 条）。
    const BUDGET: Duration = Duration::from_millis(500);
    /// 采样次数：取中位数。
    const RUNS: usize = 3;

    /// 全量：主数据配好、带项目索引，启用的规则都跑、一条不跳。计时的是做摘要加检查；项目索引
    /// 平时由项目扫描建好留着，这里事先建好，不计时。
    #[test]
    #[ignore]
    fn drc_check_2000_objects_within_500_ms() {
        let doc = synthetic_drawing(SYNTHETIC_OBJECTS);
        let library = SymbolLibrary::builtin();
        let meta = || SummaryMeta {
            drawing_id: "PID-1101".to_owned(),
            file: None,
            pbs: None,
            source: SummarySource::Open,
            fingerprint: None,
        };
        let index = index_of(&[summarize(&doc, &library, meta(), DEFAULT_TOLERANCE)], &[]);
        let rules = builtin();
        let configured = master(None);
        let types = TypeLib::embedded();
        let mut samples = Vec::with_capacity(RUNS);
        for run in 1..=RUNS {
            let started = Instant::now();
            let summary = summarize(&doc, &library, meta(), DEFAULT_TOLERANCE);
            let summarized = started.elapsed();
            let report = check(&CheckInput {
                rules: &rules,
                master: &configured,
                types: &types,
                drawings: std::slice::from_ref(&summary),
                index: Some(&index),
                others: &[],
                waivers: &[],
                trigger: Trigger::OnDemand,
                scope: Scope {
                    kind: ScopeKind::Drawing,
                    drawing_id: Some(summary.drawing_id.clone()),
                    pbs: None,
                    cross_drawing: true,
                },
            });
            let elapsed = started.elapsed();
            println!(
                "DRC-10 run {run}/{RUNS}: {:.2} ms (summary {:.2} ms), objects {}, rules run {}, results {}",
                elapsed.as_secs_f64() * 1e3,
                summarized.as_secs_f64() * 1e3,
                report.summary.objects,
                report.summary.rules_run,
                report.results.len()
            );
            assert_eq!(report.summary.objects, SYNTHETIC_OBJECTS, "run {run}");
            assert_eq!(report.summary.rules_skipped, 0, "run {run}");
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[RUNS / 2];
        let shown: Vec<String> = samples
            .iter()
            .map(|sample| format!("{:.2}", sample.as_secs_f64() * 1e3))
            .collect();
        println!(
            "DRC-10 median {:.2} ms of {shown:?}, budget {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
        assert!(
            median <= BUDGET,
            "DRC-10 超预算：中位数 {:.2} ms（{shown:?}），预算 {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
    }
}

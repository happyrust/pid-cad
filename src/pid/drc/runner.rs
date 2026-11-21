//! 检查器（`SPEC-pid-drc.md` §4.1、§5.1）：对每条启用的规则，从摘要里选出作用对象，逐个
//! 求值，为真即命中，产出 §3.6 的检查结果。只吃摘要、索引、主数据和规则，不碰文件和图纸。

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::pid::master_data;
use crate::pid::project_index::{DrawingSummary, ProjectIndex};
use crate::pid::type_lib::TypeLib;

use super::builtin_rules::rule_set_digest;
use super::dsl::{Expr, Operand, Path};
use super::facts::{subjects, SubjectFacts};
use super::model::{
    Category, CheckReport, CheckResult, Kind, Rule, RuleSet, Scope, Severity, Summary, Trigger,
    Waiver, REPORT_VERSION,
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
    /// 图号 → 该图的豁免。
    pub waivers: &'a [(String, Vec<Waiver>)],
    pub trigger: Trigger,
    pub scope: Scope,
}

/// 按规则检查范围内的图（§4.1）。纯函数：只读输入，不碰文件和图纸。
pub fn check(input: &CheckInput) -> CheckReport {
    let started = Instant::now();
    let mut summary = Summary {
        objects: input
            .drawings
            .iter()
            .map(|drawing| drawing.objects.len())
            .sum(),
        ..Summary::default()
    };
    let mut results = Vec::new();
    for rule in &input.rules.rules {
        if !input.rules.is_enabled(rule) {
            continue;
        }
        if rule.category != Category::Design {
            summary.rules_skipped += 1;
            continue;
        }
        summary.rules_run += 1;
        results.extend(hits(rule, input));
    }
    results.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
            .then_with(|| a.object.drawing_id.cmp(&b.object.drawing_id))
            .then_with(|| a.object_key.cmp(&b.object_key))
    });
    for result in &results {
        match result.severity {
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Info => summary.infos += 1,
        }
    }
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
        scope: input.scope.clone(),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        summary,
        blocked: summary.errors > 0,
        results,
        notices: Vec::new(),
        stale_waivers: Vec::new(),
    }
}

/// 一条规则在范围内各图上的命中：表达式为真才算，不知道不算。
fn hits(rule: &Rule, input: &CheckInput) -> Vec<CheckResult> {
    let kind = rule.target.kind();
    let fields = fields(rule);
    let mut found = Vec::new();
    for drawing in input.drawings {
        for subject in subjects(drawing, &rule.target, input.types) {
            let facts = SubjectFacts::new(subject, kind, drawing);
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
                points: Vec::new(),
                fields: fields.clone(),
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
/// 和根下的不算。按出现的先后，不重复。
fn fields(rule: &Rule) -> Vec<String> {
    let mut names = Vec::new();
    collect_expr(&rule.tree, rule.target.kind(), &mut names);
    names
}

fn collect_expr(expr: &Expr, kind: Kind, names: &mut Vec<String>) {
    match expr {
        Expr::Or(terms) | Expr::And(terms) => {
            for term in terms {
                collect_expr(term, kind, names);
            }
        }
        Expr::Not(inner) => collect_expr(inner, kind, names),
        Expr::Compare(comparison) => {
            collect_path(&comparison.left, kind, names);
            collect_operand(&comparison.right, kind, names);
        }
    }
}

fn collect_operand(operand: &Operand, kind: Kind, names: &mut Vec<String>) {
    match operand {
        Operand::Path(path) => collect_path(path, kind, names),
        Operand::List(items) => {
            for item in items {
                collect_operand(item, kind, names);
            }
        }
        Operand::Value(_) | Operand::Pattern(_) => {}
    }
}

fn collect_path(path: &Path, kind: Kind, names: &mut Vec<String>) {
    let name = match path.segments.as_slice() {
        [first, ..] if first == "位号" => first,
        [name] if !kind.has_fact(name) => name,
        _ => return,
    };
    if !names.contains(name) {
        names.push(name.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::pid::drc::builtin_rules::{builtin, rule_set_digest};
    use crate::pid::drc::dsl::parse;
    use crate::pid::drc::facts::ANNOTATION_TYPE;
    use crate::pid::drc::model::{
        Category, CheckResult, ObjectRef, RuleScope, ScopeKind, Severity, Source, Target,
    };
    use crate::pid::project_index::{ObjectKind, SummaryObject, SummarySource, SUMMARY_VERSION};

    /// 还没有正反例的内置规则：用到的事实还没接上，只能不命中或只能命中。到 DRC-8 要清空。
    const UNCOVERED: [&str; 33] = [
        "DR-004", "DR-006", "DR-007", "DR-008", "DR-009", "DR-010", "DR-011", "DR-012", "DR-013",
        "DR-014", "DR-015", "DR-018", "DR-019", "DR-020", "DR-021", "DR-022", "DR-023", "DR-024",
        "DR-102", "DR-103", "DR-104", "DR-105", "DR-106", "DR-107", "DR-108", "DR-109", "DR-110",
        "DR-111", "DR-112", "DR-113", "DR-114", "DR-115", "DR-116",
    ];

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

    fn run(rules: &RuleSet, drawings: &[DrawingSummary]) -> CheckReport {
        let master = master_data::Resolved::NotConfigured;
        let types = TypeLib::embedded();
        check(&CheckInput {
            rules,
            master: &master,
            types: &types,
            drawings,
            index: None,
            others: &[],
            waivers: &[],
            trigger: Trigger::OnDemand,
            scope: Scope {
                kind: ScopeKind::Drawing,
                drawing_id: drawings.first().map(|drawing| drawing.drawing_id.clone()),
                pbs: None,
                cross_drawing: false,
            },
        })
    }

    fn hits(report: &CheckReport, id: &str) -> usize {
        report
            .results
            .iter()
            .filter(|result| result.rule_id == id)
            .count()
    }

    /// 内置规则全部启用，含默认停用的 `DR-114`。
    fn all_enabled() -> RuleSet {
        let mut rules = builtin();
        for rule in &mut rules.rules {
            rule.enabled = true;
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

    /// 一条内置规则命中的例子和不命中的例子；还没有例子的在 `UNCOVERED` 里。
    fn example(id: &str) -> Option<(Vec<SummaryObject>, Vec<SummaryObject>)> {
        let pair = match id {
            "DR-001" => (
                vec![line("L-1001", &[])],
                vec![line("L-1001", &[("管道等级", "A1")])],
            ),
            "DR-002" => (
                vec![line("L-1001", &[])],
                vec![line("L-1001", &[("公称直径", "50")])],
            ),
            "DR-003" => (vec![pump(" ")], vec![pump("P-101")]),
            "DR-005" => (
                vec![instrument("FT-101", &[])],
                vec![instrument("FT-101", &[("仪表回路", "F-101")])],
            ),
            "DR-016" => (
                vec![annotation(&[("文字", "  ")])],
                vec![annotation(&[("文字", "去火炬")])],
            ),
            "DR-017" => (
                vec![valve("V-101", &[("开度", "-5")])],
                vec![valve("V-101", &[("开度", "100")])],
            ),
            "DR-025" => (
                vec![line("L-1001", &[("流体介质", "蒸汽")])],
                vec![line("L-1001", &[("流体介质", "蒸汽"), ("绝热等级", "H1")])],
            ),
            "DR-101" => (vec![valve("", &[])], vec![valve("V-101", &[])]),
            _ => return None,
        };
        Some(pair)
    }

    /// 表驱动：每条内置规则在一张只有例子对象的图上，命中的例子出一条、不命中的不出。
    #[test]
    fn every_builtin_rule_has_a_hit_and_a_miss() {
        let rules = all_enabled();
        let ids: Vec<&str> = rules.rules.iter().map(|rule| rule.id.as_str()).collect();
        for id in UNCOVERED {
            assert!(ids.contains(&id), "未覆盖名单里的 {id} 不是内置规则");
        }
        for id in ids {
            let Some((hit, miss)) = example(id) else {
                assert!(UNCOVERED.contains(&id), "{id} 没有例子，也不在未覆盖名单里");
                continue;
            };
            assert!(
                !UNCOVERED.contains(&id),
                "{id} 有了例子，要从未覆盖名单里拿掉"
            );
            let report = run(&rules, &[drawing("PID-1101", hit)]);
            assert_eq!(hits(&report, id), 1, "{id} 应命中一条");
            let report = run(&rules, &[drawing("PID-1101", miss)]);
            assert_eq!(hits(&report, id), 0, "{id} 不应命中");
        }
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
    /// 不算进 `rules_skipped`。
    #[test]
    fn disabling_a_rule_or_its_set_drops_only_its_results() {
        let drawings = [drawing(
            "PID-1101",
            vec![annotation(&[]), line("L-1001", &[]), valve("", &[])],
        )];
        let rules = builtin();
        let all = run(&rules, &drawings);
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
        let report = run(&changed, &drawings);
        assert_eq!(report.results, keep("DR-016"));
        assert_eq!(report.summary.rules_run, all.summary.rules_run - 1);
        assert_eq!(report.summary.rules_skipped, all.summary.rules_skipped);
        changed.rules[at].enabled = true;
        assert_eq!(run(&changed, &drawings).results, all.results);

        let at = changed
            .sets
            .iter()
            .position(|set| set.name == "基础规则")
            .expect("基础规则");
        changed.sets[at].enabled = false;
        let report = run(&changed, &drawings);
        assert_eq!(report.results, keep("DR-0"));
        assert_eq!(report.summary.rules_run, all.summary.rules_run - 25);
        changed.sets[at].enabled = true;
        assert_eq!(run(&changed, &drawings).results, all.results);
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

    /// 结果按严重度、规则号、图号、对象键排；三个严重度各自计数，有错误就 `blocked`。
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
                ("DR-017", "PID-1102", "VALVE_GATE|V-201"),
                ("DR-016", "PID-1101", "ANNOTATION||10.000,20.000"),
            ]
        );
        assert_eq!(
            report.summary,
            Summary {
                errors: 4,
                warnings: 1,
                infos: 1,
                waived: 0,
                objects: 6,
                rules_run: 40,
                rules_skipped: 0,
            }
        );
        assert!(report.blocked);
        assert!(report.notices.is_empty());
        assert!(report.stale_waivers.is_empty());

        let report = run(&builtin(), &[drawing("PID-1101", vec![annotation(&[])])]);
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
            ..line("L-1003", &[("公称直径", "50")])
        };
        let report = run(&builtin(), &[drawing("PID-1101", vec![pipe])]);
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
            json!({ "错误": 1, "警告": 0, "提示": 0, "已豁免": 0, "objects": 1, "rules_run": 40, "rules_skipped": 0 })
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
        assert_eq!(value["notices"], json!([]));
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
}

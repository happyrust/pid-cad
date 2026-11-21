//! §3.4 的事实：从图纸摘要里选出规则的作用对象，在对象上取表达式里的名字。
//!
//! 现在接了对象自己的属性；派生事实 `位号`、`位号来源`、`独立编号`、`种类`、`类型编码`、`类型`、
//! `图层`、`位号合规`、`位号.长度`、`位号.同类型重复数`、`必填项缺失数`，设备的 `管口数`、
//! `设备类别`，符号实例的 `符号.状态`；仪表回路和图纸自己的事实；根 `图纸.图号`、`图纸.PBS`、
//! `参数.<名>`。管线、管路、管段、管口、信号线、页间连接符、主数据和跨图的事实还取不到值。
//! 作用对象从摘要的 `objects[]` 里选；仪表回路是按 `仪表回路` 属性分组的仪表，图纸是整张图。

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::pid::master_data::Resolved;
use crate::pid::project_index::{DrawingSummary, ObjectKind, SummaryObject};
use crate::pid::symbols::BLOCK_PREFIX;
use crate::pid::tag_engine::{resolve_rule, TAG_STATUS_DUPLICATED};
use crate::pid::tag_rule;
use crate::pid::type_lib::TypeLib;

use super::dsl::{Fact, FactList, Facts, Matching, Path};
use super::model::{Kind, ObjectRef, Rule, RuleSet, Target};

/// 标注的类型编码。本期图里没有标注；摘要里这个类型的对象当标注检查。
pub const ANNOTATION_TYPE: &str = "ANNOTATION";

/// 缺的必填属性数。结果的 `fields` 里换成缺的那些属性名（§3.6）。
pub const MISSING_COUNT: &str = "必填项缺失数";

/// 消息模板里填缺的必填属性名的占位符，名字之间用顿号隔开。
const MISSING_FIELDS: &str = "缺失字段";

/// 仪表分组用的属性，也是仪表回路的种类名。
const LOOP: &str = "仪表回路";

/// 图纸的种类名，也是根 `图纸`。
const DRAWING: &str = "图纸";

/// 检查时各张图共用的：规则包、主数据、类型库。
pub struct Context<'a> {
    pub rules: &'a RuleSet,
    pub master: &'a Resolved,
    pub types: &'a TypeLib,
}

/// 一张图，连同检查前先数好的：同类型同位号的对象数、设备数、设备的管口数、仪表回路。
pub struct Sheet<'a> {
    drawing: &'a DrawingSummary,
    /// （类型编码，位号）→ 有自己编号的对象数。位号去掉首尾空白，空的不数。
    tags: HashMap<(&'a str, &'a str), usize>,
    /// （类型编码，位号）→ 设备数。位号去掉首尾空白，空的也数。
    equipment: HashMap<(&'a str, &'a str), usize>,
    /// （类型编码，位号）→ 属主是设备的端口数。
    nozzles: HashMap<(&'a str, &'a str), usize>,
    loops: Vec<Loop<'a>>,
}

impl<'a> Sheet<'a> {
    pub fn new(drawing: &'a DrawingSummary) -> Self {
        let mut tags = HashMap::new();
        let mut equipment = HashMap::new();
        let mut loops: Vec<Loop<'a>> = Vec::new();
        let mut loop_at: HashMap<&'a str, usize> = HashMap::new();
        for object in &drawing.objects {
            let key = (object.type_code.as_str(), object.tag.trim());
            if object.own_tag && !key.1.is_empty() {
                *tags.entry(key).or_insert(0) += 1;
            }
            if object.kind == ObjectKind::Equipment {
                *equipment.entry(key).or_insert(0) += 1;
            }
            let number = object.props.get(LOOP).map_or("", |number| number.trim());
            if object.kind == ObjectKind::Instrument && !number.is_empty() {
                let at = *loop_at.entry(number).or_insert_with(|| {
                    loops.push(Loop {
                        number,
                        members: Vec::new(),
                    });
                    loops.len() - 1
                });
                loops[at].members.push(object);
            }
        }
        let mut nozzles = HashMap::new();
        for port in &drawing.ports {
            if port.owner_kind == ObjectKind::Equipment {
                *nozzles
                    .entry((port.type_code.as_str(), port.tag.trim()))
                    .or_insert(0) += 1;
            }
        }
        Self {
            drawing,
            tags,
            equipment,
            nozzles,
            loops,
        }
    }
}

/// 一个仪表回路：`仪表回路` 属性相同（去掉首尾空白）的一组仪表，按摘要里的顺序。
#[derive(Debug)]
pub struct Loop<'a> {
    number: &'a str,
    members: Vec<&'a SummaryObject>,
}

/// 一张图上规则的一个作用对象。
#[derive(Debug, Clone, Copy)]
pub enum Subject<'a> {
    /// 摘要 `objects[]` 的一项。
    Object(&'a SummaryObject),
    Loop(&'a Loop<'a>),
    /// 整张图。
    Drawing,
}

/// `sheet` 上作用对象是 `target` 的那些，按摘要里的顺序；仪表回路按第一个仪表出现的
/// 先后。写类型编码的含它的下级类型，下级按 `types` 认。
pub fn subjects<'a>(sheet: &'a Sheet<'a>, target: &Target, types: &TypeLib) -> Vec<Subject<'a>> {
    match target {
        Target::Kind(Kind::Loop) => sheet.loops.iter().map(Subject::Loop).collect(),
        Target::Kind(Kind::Drawing) => vec![Subject::Drawing],
        _ => sheet
            .drawing
            .objects
            .iter()
            .filter(|object| picks(target, object, types))
            .map(Subject::Object)
            .collect(),
    }
}

fn picks(target: &Target, object: &SummaryObject, types: &TypeLib) -> bool {
    let kind = match target {
        Target::Kind(kind) => *kind,
        Target::TypeCode(code) => {
            return object.type_code == *code || types.ancestors(&object.type_code).contains(code);
        }
    };
    match kind {
        Kind::Object => true,
        Kind::Equipment => object.kind == ObjectKind::Equipment,
        Kind::Valve => object.kind == ObjectKind::Valve,
        Kind::Fitting => object.kind == ObjectKind::Fitting,
        Kind::Instrument => object.kind == ObjectKind::Instrument,
        Kind::PipeLine => object.kind == ObjectKind::PipeLine,
        Kind::PipeRun => object.kind == ObjectKind::PipeRun,
        Kind::SignalLine => object.kind == ObjectKind::SignalLine,
        Kind::Symbol => object.symbol.is_some(),
        Kind::Annotation => object.type_code == ANNOTATION_TYPE,
        Kind::Segment | Kind::Nozzle | Kind::Connector | Kind::Loop | Kind::Drawing => false,
    }
}

/// 一个作用对象上的事实。单段的名字先看是不是 `kind` 这种对象的派生事实，不是就当
/// 属性名；`kind` 是解析表达式时认名字用的那种对象。`参数.<名>` 取规则的 `params`。
pub struct SubjectFacts<'a> {
    subject: Subject<'a>,
    kind: Kind,
    params: &'a BTreeMap<String, Value>,
    sheet: &'a Sheet<'a>,
    context: &'a Context<'a>,
}

impl<'a> SubjectFacts<'a> {
    pub fn new(
        subject: Subject<'a>,
        rule: &'a Rule,
        sheet: &'a Sheet<'a>,
        context: &'a Context<'a>,
    ) -> Self {
        Self {
            subject,
            kind: rule.target.kind(),
            params: &rule.params,
            sheet,
            context,
        }
    }

    /// 结果里的对象引用（§3.6）。仪表回路和图纸没有类型编码和句柄；回路的 `tag` 是回路号，
    /// 位置取回路里第一个仪表的。
    pub fn object_ref(&self) -> ObjectRef {
        let drawing_id = self.sheet.drawing.drawing_id.clone();
        match self.subject {
            Subject::Object(object) => ObjectRef {
                kind: kind_name(object.kind).to_owned(),
                type_code: object.type_code.clone(),
                tag: text(&object.tag),
                drawing_id,
                owner: object.owner.clone(),
                port: None,
                at: object.at,
                handle: Some(object.handle.clone()),
            },
            Subject::Loop(group) => ObjectRef {
                kind: LOOP.to_owned(),
                type_code: String::new(),
                tag: Some(group.number.to_owned()),
                drawing_id,
                owner: None,
                port: None,
                at: group.members.first().map_or([0.0, 0.0], |member| member.at),
                handle: None,
            },
            Subject::Drawing => ObjectRef {
                kind: DRAWING.to_owned(),
                type_code: String::new(),
                tag: None,
                drawing_id,
                owner: None,
                port: None,
                at: [0.0, 0.0],
                handle: None,
            },
        }
    }

    /// 豁免用的对象键（§3.7）：有自己编号的是「类型编码|位号」，没有位号或没有自己
    /// 编号的是「类型编码|所属|x,y」；仪表回路是「仪表回路|回路号」，图纸是「图纸|图号」。
    pub fn object_key(&self) -> String {
        match self.subject {
            Subject::Object(object) => match text(&object.tag) {
                Some(tag) if object.own_tag => format!("{}|{tag}", object.type_code),
                _ => format!(
                    "{}|{}|{}",
                    object.type_code,
                    object.owner.as_deref().unwrap_or_default(),
                    point(object.at)
                ),
            },
            Subject::Loop(group) => format!("{LOOP}|{}", group.number),
            Subject::Drawing => format!("{DRAWING}|{}", self.sheet.drawing.drawing_id),
        }
    }

    /// 没写消息模板时消息里的「位号或位置」：仪表回路是回路号，图纸是图号。
    pub fn location(&self) -> String {
        match self.subject {
            Subject::Object(object) => text(&object.tag).unwrap_or_else(|| point(object.at)),
            Subject::Loop(group) => group.number.to_owned(),
            Subject::Drawing => self.sheet.drawing.drawing_id.clone(),
        }
    }

    /// 消息模板里 `{name}` 填的值（§3.1）；没有值是空串。仪表回路的 `{位号}` 是回路号，
    /// 仪表回路和图纸的 `{类型}` 是种类名；`{缺失字段}` 是缺的必填属性名。
    pub fn placeholder(&self, name: &str) -> String {
        let value = match (name, self.subject) {
            ("位号", Subject::Object(object)) => Some(object.tag.clone()),
            ("位号", Subject::Loop(group)) => Some(group.number.to_owned()),
            ("类型", Subject::Object(object)) => Some(object.type_label.clone()),
            ("类型", Subject::Loop(_)) => Some(LOOP.to_owned()),
            ("类型", Subject::Drawing) => Some(DRAWING.to_owned()),
            ("所属", Subject::Object(object)) => object.owner.clone(),
            ("位号" | "所属", _) => None,
            ("图号", _) => Some(self.sheet.drawing.drawing_id.clone()),
            (MISSING_FIELDS, _) => Some(self.missing_fields().join("、")),
            _ => match self.fact(&Path {
                segments: vec![name.to_owned()],
            }) {
                Fact::Value(value) => value,
                Fact::List(list) => list.map(|list| list.items.join("、")),
            },
        };
        value
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    }

    /// 这个对象缺的必填属性名：`required_fields` 里它的类型和上级类型列的、值为空的，从根类型
    /// 往下排，不重复。类型库里没有的类型只看它自己。不是对象的没有。
    pub fn missing_fields(&self) -> Vec<String> {
        let Subject::Object(object) = self.subject else {
            return Vec::new();
        };
        let mut chain = self.context.types.ancestors(&object.type_code);
        if chain.is_empty() {
            chain.push(object.type_code.clone());
        }
        let required = &self.context.rules.required_fields;
        let mut missing: Vec<String> = Vec::new();
        for code in chain.iter().rev() {
            for name in required.get(code).into_iter().flatten() {
                let filled = object
                    .props
                    .get(name)
                    .is_some_and(|value| !value.trim().is_empty());
                if !filled && !missing.contains(name) {
                    missing.push(name.clone());
                }
            }
        }
        missing
    }

    fn value(&self, name: &str) -> Option<String> {
        if !self.kind.has_fact(name) {
            return match self.subject {
                Subject::Object(object) => object.props.get(name).cloned(),
                Subject::Loop(_) | Subject::Drawing => None,
            };
        }
        match self.subject {
            Subject::Object(object) => self.object_fact(object, name),
            Subject::Loop(group) => loop_fact(group, name, self.context.rules),
            Subject::Drawing => self.drawing_fact(name),
        }
    }

    fn object_fact(&self, object: &SummaryObject, name: &str) -> Option<String> {
        match name {
            "位号" => Some(object.tag.clone()),
            "位号来源" => Some(object.tag_source.clone()),
            "独立编号" => Some(truth(object.own_tag)),
            "种类" => Some(kind_name(object.kind).to_owned()),
            "类型编码" => Some(object.type_code.clone()),
            "类型" | "设备类别" => Some(object.type_label.clone()),
            "图层" => Some(object.layer.clone()),
            "位号合规" => self.tag_fits(object).map(truth),
            MISSING_COUNT => Some(self.missing_fields().len().to_string()),
            "管口数" => self.nozzles(object).map(|count| count.to_string()),
            _ => None,
        }
    }

    /// `位号合规`：按位号引擎给这个类型用的那条位号规则判。没有自己编号、位号为空、类型没有
    /// 位号规则时不知道。
    fn tag_fits(&self, object: &SummaryObject) -> Option<bool> {
        let tag = own_tag(object)?;
        let rule = resolve_rule(self.context.types, &object.type_code).ok()?;
        tag_rule::matches(&rule, tag, &object.props)
    }

    /// `管口数`：图上属于这台设备的端口数，按类型编码和位号认。同类型同位号的设备不止
    /// 一台时分不清端口是谁的，不知道。
    fn nozzles(&self, object: &SummaryObject) -> Option<usize> {
        let key = (object.type_code.as_str(), object.tag.trim());
        if self.sheet.equipment.get(&key).copied().unwrap_or(0) > 1 {
            return None;
        }
        Some(self.sheet.nozzles.get(&key).copied().unwrap_or(0))
    }

    fn drawing_fact(&self, name: &str) -> Option<String> {
        let drawing = self.sheet.drawing;
        match name {
            "图号" => Some(drawing.drawing_id.clone()),
            "PBS" => drawing.pbs.clone(),
            "对象数" => Some(drawing.objects.len().to_string()),
            "图例标准" => match self.context.master {
                Resolved::Configured { data, .. } => data.legend_standard.clone(),
                Resolved::Unreadable { .. } | Resolved::NotConfigured => None,
            },
            _ => None,
        }
    }

    /// `位号.长度`、`位号.同类型重复数`。没有自己编号或位号为空时不知道；位号引擎标了重复的
    /// 至少算 2。
    fn tag_fact(&self, name: &str) -> Option<String> {
        let Subject::Object(object) = self.subject else {
            return None;
        };
        let tag = own_tag(object)?;
        match name {
            "长度" => Some(tag.chars().count().to_string()),
            "同类型重复数" => {
                let count = self
                    .sheet
                    .tags
                    .get(&(object.type_code.as_str(), tag))
                    .copied()
                    .unwrap_or(0);
                let count = if object.tag_status == TAG_STATUS_DUPLICATED {
                    count.max(2)
                } else {
                    count
                };
                Some(count.to_string())
            }
            _ => None,
        }
    }

    /// `符号.状态`：当前符号库里有这个块是 `有效`，没有是 `缺失`；不带符号的不知道。
    fn symbol_state(&self) -> Option<String> {
        let Subject::Object(object) = self.subject else {
            return None;
        };
        object.symbol.as_ref()?;
        let known = object.symbol_known;
        Some(if known { "有效" } else { "缺失" }.to_owned())
    }
}

impl Facts for SubjectFacts<'_> {
    fn fact(&self, path: &Path) -> Fact {
        match path.segments.as_slice() {
            [name] => Fact::Value(self.value(name)),
            [root, name] if root == "参数" => param(self.params.get(name)),
            [root, name] if root == DRAWING && (name == "图号" || name == "PBS") => {
                Fact::Value(self.drawing_fact(name))
            }
            [first, name] if first == "位号" => Fact::Value(self.tag_fact(name)),
            [first, name] if first == "符号" && name == "状态" => {
                Fact::Value(self.symbol_state())
            }
            _ => Fact::Value(None),
        }
    }
}

/// 仪表回路自己的事实。
fn loop_fact(group: &Loop, name: &str, rules: &RuleSet) -> Option<String> {
    match name {
        "回路号" => Some(group.number.to_owned()),
        "仪表数" => Some(group.members.len().to_string()),
        "控制器数" => role_count(group, "控制器", rules),
        "测量元件数" => role_count(group, "测量元件", rules),
        _ => None,
    }
}

/// 回路里符号在 `loop_roles` 给这个角色列的符号里的仪表数；规则包没写这个角色时不知道。
fn role_count(group: &Loop, role: &str, rules: &RuleSet) -> Option<String> {
    let symbols = rules.loop_roles.get(role)?;
    let count = group
        .members
        .iter()
        .filter(|member| {
            member
                .symbol
                .as_deref()
                .and_then(symbol_id)
                .is_some_and(|id| symbols.iter().any(|symbol| symbol == id))
        })
        .count();
    Some(count.to_string())
}

/// 块名 `PID_SYM_<类别>_<id>` 里的符号 id。符号库的类别名不含下划线，取第一个下划线后面的。
fn symbol_id(block: &str) -> Option<&str> {
    block
        .strip_prefix(BLOCK_PREFIX)?
        .split_once('_')
        .map(|(_, id)| id)
}

/// `参数.<名>`：文本、数、真假是一个值；数组是一张表，跟 `=` 一样比。没有这个参数、写成
/// `null` 或对象、数组里没有非空的项时取不到。
fn param(value: Option<&Value>) -> Fact {
    match value {
        Some(Value::Array(items)) => {
            let items: Vec<String> = items
                .iter()
                .filter_map(scalar)
                .filter(|item| !item.trim().is_empty())
                .collect();
            Fact::List((!items.is_empty()).then_some(FactList {
                items,
                matching: Matching::Equal,
            }))
        }
        value => Fact::Value(value.and_then(scalar)),
    }
}

/// 参数里的一个值写成文本：真假是 `TRUE`、`FALSE`；`null`、数组和对象不算。
fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(truth(*value)),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// 对象自己的位号：有自己编号、去掉首尾空白后不空的。
fn own_tag(object: &SummaryObject) -> Option<&str> {
    let tag = object.tag.trim();
    (object.own_tag && !tag.is_empty()).then_some(tag)
}

/// 摘要里的种类名（项目索引规格 §3.1 归类表的 `kind`）。
fn kind_name(kind: ObjectKind) -> &'static str {
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

/// 去掉首尾空白；剩下空的是 `None`。
fn text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 对象键和消息里的坐标：保留 3 位小数。
fn point(at: [f64; 2]) -> String {
    format!("{:.3},{:.3}", at[0], at[1])
}

/// 派生事实里的真假写成 `TRUE`、`FALSE` 两个文本（规格 §11.1 第 10 条）。
fn truth(value: bool) -> String {
    if value { "TRUE" } else { "FALSE" }.to_owned()
}

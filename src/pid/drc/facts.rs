//! §3.4 的事实：从图纸摘要里选出规则的作用对象，在对象上取表达式里的名字。
//!
//! 现在接了对象自己的属性和最基本的派生事实（`位号`、`位号来源`、`独立编号`、`种类`、
//! `类型编码`、`类型`、`图层`）；别的派生事实、关系和根还取不到值。作用对象现在只从摘要的
//! `objects[]` 里选。

use crate::pid::project_index::{DrawingSummary, ObjectKind, SummaryObject};
use crate::pid::type_lib::TypeLib;

use super::dsl::{Fact, Facts, Path};
use super::model::{Kind, ObjectRef, Target};

/// 标注的类型编码。本期图里没有标注；摘要里这个类型的对象当标注检查。
pub const ANNOTATION_TYPE: &str = "ANNOTATION";

/// 一张图上规则的一个作用对象。
#[derive(Debug, Clone, Copy)]
pub enum Subject<'a> {
    /// 摘要 `objects[]` 的一项。
    Object(&'a SummaryObject),
}

/// `drawing` 上作用对象是 `target` 的那些对象，按摘要里的顺序。写类型编码的含它的下级
/// 类型，下级按 `types` 认。
pub fn subjects<'a>(
    drawing: &'a DrawingSummary,
    target: &Target,
    types: &TypeLib,
) -> Vec<Subject<'a>> {
    drawing
        .objects
        .iter()
        .filter(|object| picks(target, object, types))
        .map(Subject::Object)
        .collect()
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
/// 属性名；`kind` 是解析表达式时认名字用的那种对象。
pub struct SubjectFacts<'a> {
    subject: Subject<'a>,
    kind: Kind,
    drawing: &'a DrawingSummary,
}

impl<'a> SubjectFacts<'a> {
    pub fn new(subject: Subject<'a>, kind: Kind, drawing: &'a DrawingSummary) -> Self {
        Self {
            subject,
            kind,
            drawing,
        }
    }

    /// 结果里的对象引用（§3.6）。
    pub fn object_ref(&self) -> ObjectRef {
        let Subject::Object(object) = self.subject;
        ObjectRef {
            kind: kind_name(object.kind).to_owned(),
            type_code: object.type_code.clone(),
            tag: text(&object.tag),
            drawing_id: self.drawing.drawing_id.clone(),
            owner: object.owner.clone(),
            port: None,
            at: object.at,
            handle: Some(object.handle.clone()),
        }
    }

    /// 豁免用的对象键（§3.7）：有自己编号的是「类型编码|位号」，没有位号或没有自己
    /// 编号的是「类型编码|所属|x,y」。
    pub fn object_key(&self) -> String {
        let Subject::Object(object) = self.subject;
        match text(&object.tag) {
            Some(tag) if object.own_tag => format!("{}|{tag}", object.type_code),
            _ => format!(
                "{}|{}|{}",
                object.type_code,
                object.owner.as_deref().unwrap_or_default(),
                point(object.at)
            ),
        }
    }

    /// 没写消息模板时消息里的「位号或位置」。
    pub fn location(&self) -> String {
        let Subject::Object(object) = self.subject;
        text(&object.tag).unwrap_or_else(|| point(object.at))
    }

    /// 消息模板里 `{name}` 填的值（§3.1）；没有值是空串。
    pub fn placeholder(&self, name: &str) -> String {
        let Subject::Object(object) = self.subject;
        let value = match name {
            "位号" => Some(object.tag.clone()),
            "类型" => Some(object.type_label.clone()),
            "图号" => Some(self.drawing.drawing_id.clone()),
            "所属" => object.owner.clone(),
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

    fn value(&self, name: &str) -> Option<String> {
        let Subject::Object(object) = self.subject;
        if !self.kind.has_fact(name) {
            return object.props.get(name).cloned();
        }
        match name {
            "位号" => Some(object.tag.clone()),
            "位号来源" => Some(object.tag_source.clone()),
            "独立编号" => Some(truth(object.own_tag)),
            "种类" => Some(kind_name(object.kind).to_owned()),
            "类型编码" => Some(object.type_code.clone()),
            "类型" => Some(object.type_label.clone()),
            "图层" => Some(object.layer.clone()),
            _ => None,
        }
    }
}

impl Facts for SubjectFacts<'_> {
    fn fact(&self, path: &Path) -> Fact {
        match path.segments.as_slice() {
            [name] => Fact::Value(self.value(name)),
            _ => Fact::Value(None),
        }
    }
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

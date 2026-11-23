//! §3.4 的事实：从图纸摘要里选出规则的作用对象，在对象上取表达式里的名字。
//!
//! 现在接了对象自己的属性；派生事实 `位号`、`位号来源`、`独立编号`、`种类`、`类型编码`、`类型`、
//! `图层`、`位号合规`、`位号.长度`、`位号.同类型重复数`、`必填项缺失数`、`未连接端口数`，设备的
//! `管口数`、`设备类别`，符号实例的 `符号.状态`；管线、管路、管段的 `端口未连接数`、`长度`、
//! `管路数`、`管段数`、`后缀`、`重叠管段数`、`直连异径管路数`、`未标记覆盖数`，关系 `所属管路`、
//! `所属管线`；管口的 `名称`、`方向`、`连接状态` 和关系 `所属设备`；信号线的 `起点仪表`、
//! `终点仪表`、`端口未连接数`、`接到设备管口数`；仪表回路和图纸自己的事实；到主数据里找的关系
//! `管道等级`、`绝热等级`、`流体介质`；根 `项目` 下的主数据表和图例标准、`图纸.图号`、
//! `图纸.PBS`、`参数.<名>`；页间连接符的 `连接符种类`、`对端图纸`、`对端位号`、`配对状态` 和关系
//! `所在线`、`对端线`，对象的 `位号.跨图重复数`。管口的 `位号前缀`、阀门的 `流向相反` 还取不到值。
//! 作用对象从摘要的 `objects[]` 里选；管段是摘要的 `segments[]`，管口是摘要 `ports[]` 里属主是
//! 设备的，页间连接符是摘要的 `connectors[]`，仪表回路是按 `仪表回路` 属性分组的仪表，图纸是
//! 整张图。`配对状态`、`对端线`、`位号.跨图重复数` 要项目索引（[`Project`]），没有时不知道。

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::pid::master_data::{parse_dn, MasterData, Resolved, Table};
use crate::pid::project_index::{
    DrawingSummary, Joint, LinkEnd, LinkStatus, ObjectKind, PeerKind, ProjectIndex, SignalEnd,
    SummaryConnector, SummaryObject, SummaryPort, SummarySegment, DEFAULT_TOLERANCE,
};
use crate::pid::propagate::LINE_FIELDS;
use crate::pid::symbols::{PortDir, BLOCK_PREFIX};
use crate::pid::tag_engine::{resolve_rule, TAG_STATUS_DUPLICATED};
use crate::pid::tag_rule;
use crate::pid::topology::{ConnectorKind, TYPE_PIPE_LINE, TYPE_PIPE_SEGMENT};
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

/// 管段的种类名。
const SEGMENT: &str = "管段";

/// 管口的种类名。
const NOZZLE: &str = "管口";

/// 页间连接符的种类名。
const CONNECTOR: &str = "页间连接符";

/// 同一进程里配的对（`peer_kind` 是 `local`）的 `对端图纸`。
const LOCAL_PEER: &str = "本机配对";

/// 根 `项目`：项目主数据的表和图例标准。
const PROJECT: &str = "项目";

/// 图例标准：根 `项目` 下的名字，也是图纸自己的事实。
const LEGEND: &str = "图例标准";

/// 按对象的同名属性到主数据里找那一行的关系（§3.4），和各自找的表。
const MASTER_RELATIONS: [(&str, Table); 3] = [
    ("管道等级", Table::PipeClass),
    ("绝热等级", Table::InsulationClass),
    ("流体介质", Table::FluidCode),
];

/// 结果的 `points` 要列出位置的派生事实（§3.6）：没接上的端点、与异径管路相接的地方。
pub const LOCATED: [&str; 2] = ["端口未连接数", "直连异径管路数"];

/// 检查时各张图共用的：规则包、主数据、类型库，带着项目索引查时还有项目（[`Project`]）。
pub struct Context<'a> {
    pub rules: &'a RuleSet,
    pub master: &'a Resolved,
    pub types: &'a TypeLib,
    pub project: Option<Project<'a>>,
}

/// 跨图的事实要的（§3.4）：按图号找得到的图（查的和只用来找对端的），项目索引里连接符的配对和
/// 跨图重复的位号。
pub struct Project<'a> {
    sheets: HashMap<&'a str, &'a Sheet<'a>>,
    /// （图号，句柄）→ 这个连接符的配对。
    pairing: HashMap<(&'a str, &'a str), Pairing<'a>>,
    /// （类型编码，位号）→ 这个位号的对象落在的图号，不重复。位号照索引里的，不去空白。
    duplicates: HashMap<(&'a str, &'a str), Vec<&'a str>>,
}

/// 一个连接符在项目索引里的配对。
#[derive(Clone, Copy)]
enum Pairing<'a> {
    /// 配成了对，对端是这一个。
    Paired(&'a LinkEnd),
    Unresolved(LinkStatus),
}

impl<'a> Project<'a> {
    /// 由项目索引 `index` 和找得到的图建：`checked` 是查的图，`others` 是只用来找对端的图。
    pub fn new(index: &'a ProjectIndex, checked: &'a [Sheet<'a>], others: &'a [Sheet<'a>]) -> Self {
        let sheets = checked
            .iter()
            .chain(others)
            .map(|sheet| (sheet.drawing.drawing_id.as_str(), sheet))
            .collect();
        let mut pairing = HashMap::new();
        for link in &index.links {
            for (end, other) in [(&link.from, &link.to), (&link.to, &link.from)] {
                pairing.insert(
                    (end.drawing_id.as_str(), end.handle.as_str()),
                    Pairing::Paired(other),
                );
            }
        }
        for item in &index.unresolved {
            pairing.insert(
                (item.drawing_id.as_str(), item.handle.as_str()),
                Pairing::Unresolved(item.status),
            );
        }
        let mut duplicates = HashMap::new();
        for duplicate in &index.duplicates {
            let mut drawings: Vec<&str> = duplicate
                .places
                .iter()
                .map(|place| place.drawing_id.as_str())
                .collect();
            drawings.sort_unstable();
            drawings.dedup();
            duplicates.insert(
                (duplicate.type_code.as_str(), duplicate.tag.as_str()),
                drawings,
            );
        }
        Self {
            sheets,
            pairing,
            duplicates,
        }
    }

    /// 图 `drawing_id` 上句柄是 `handle` 的连接符的配对；索引里没有它时是 `None`。
    fn pairing(&self, drawing_id: &'a str, handle: &'a str) -> Option<Pairing<'a>> {
        self.pairing.get(&(drawing_id, handle)).copied()
    }

    /// 和 `object` 类型编码、位号都相同的对象还落在几张别的图上（`drawing_id` 以外）。
    fn elsewhere(&self, object: &'a SummaryObject, drawing_id: &str) -> usize {
        self.duplicates
            .get(&(object.type_code.as_str(), object.tag.as_str()))
            .map_or(0, |drawings| {
                drawings.iter().filter(|id| **id != drawing_id).count()
            })
    }
}

/// 一张图，连同检查前先数好的：同类型同位号的对象数、设备、符号实例的端口和个数、仪表回路，
/// 按位号找的管线、管路和信号线，管线的管路数，管线和管路名下的线段，信号线的两端。
pub struct Sheet<'a> {
    drawing: &'a DrawingSummary,
    /// （类型编码，位号）→ 有自己编号的对象数。位号去掉首尾空白，空的不数。
    tags: HashMap<(&'a str, &'a str), usize>,
    /// （类型编码，位号）→ 这个位号的设备。位号去掉首尾空白，空的也收。
    equipment: HashMap<(&'a str, &'a str), Vec<&'a SummaryObject>>,
    /// （属主种类，类型编码，位号）→ 属主的端口，按摘要里的顺序。位号去掉首尾空白，空的也收。
    ports: HashMap<(ObjectKind, &'a str, &'a str), Vec<&'a SummaryPort>>,
    /// （种类，类型编码，位号）→ 带符号的对象数。位号去掉首尾空白，空的也数。
    symbols: HashMap<(ObjectKind, &'a str, &'a str), usize>,
    loops: Vec<Loop<'a>>,
    /// 管线位号 → 这个位号的管线。位号去掉首尾空白，空的不收。
    lines: HashMap<&'a str, Vec<&'a SummaryObject>>,
    /// 管路位号 → 这个位号的管路。位号去掉首尾空白，空的不收。
    runs: HashMap<&'a str, Vec<&'a SummaryObject>>,
    /// 信号线位号 → 这个位号的信号线。位号去掉首尾空白，空的也收。
    signals: HashMap<&'a str, Vec<&'a SummaryObject>>,
    /// 管线位号 → 属主是它的管路数。
    line_runs: HashMap<&'a str, usize>,
    /// 管线位号 → 它的线段，它的管路的也算。
    line_segments: HashMap<&'a str, Vec<&'a SummarySegment>>,
    /// 管路位号 → 它的线段。
    run_segments: HashMap<&'a str, Vec<&'a SummarySegment>>,
    /// 信号线位号 → 它的两端，按摘要里的顺序。
    signal_ends: HashMap<&'a str, Vec<&'a SignalEnd>>,
}

impl<'a> Sheet<'a> {
    pub fn new(drawing: &'a DrawingSummary) -> Self {
        let mut tags = HashMap::new();
        let mut equipment: HashMap<(&'a str, &'a str), Vec<&'a SummaryObject>> = HashMap::new();
        let mut symbols = HashMap::new();
        let mut loops: Vec<Loop<'a>> = Vec::new();
        let mut loop_at: HashMap<&'a str, usize> = HashMap::new();
        let mut lines: HashMap<&'a str, Vec<&'a SummaryObject>> = HashMap::new();
        let mut runs: HashMap<&'a str, Vec<&'a SummaryObject>> = HashMap::new();
        let mut signals: HashMap<&'a str, Vec<&'a SummaryObject>> = HashMap::new();
        let mut line_runs = HashMap::new();
        for object in &drawing.objects {
            let key = (object.type_code.as_str(), object.tag.trim());
            if object.own_tag && !key.1.is_empty() {
                *tags.entry(key).or_insert(0) += 1;
            }
            if object.symbol.is_some() {
                *symbols.entry((object.kind, key.0, key.1)).or_insert(0) += 1;
            }
            match object.kind {
                ObjectKind::Equipment => equipment.entry(key).or_default().push(object),
                ObjectKind::PipeLine if !key.1.is_empty() => {
                    lines.entry(key.1).or_default().push(object);
                }
                ObjectKind::PipeRun if !key.1.is_empty() => {
                    runs.entry(key.1).or_default().push(object);
                }
                ObjectKind::SignalLine => signals.entry(key.1).or_default().push(object),
                _ => {}
            }
            let line = owner(object).filter(|_| object.kind == ObjectKind::PipeRun);
            if let Some(line) = line {
                *line_runs.entry(line).or_insert(0) += 1;
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
        let mut ports: HashMap<(ObjectKind, &'a str, &'a str), Vec<&'a SummaryPort>> =
            HashMap::new();
        for port in &drawing.ports {
            ports
                .entry((port.owner_kind, port.type_code.as_str(), port.tag.trim()))
                .or_default()
                .push(port);
        }
        let mut line_segments: HashMap<&'a str, Vec<&'a SummarySegment>> = HashMap::new();
        let mut run_segments: HashMap<&'a str, Vec<&'a SummarySegment>> = HashMap::new();
        for segment in &drawing.segments {
            line_segments
                .entry(segment.line.trim())
                .or_default()
                .push(segment);
            if let Some(run) = &segment.run {
                run_segments.entry(run.trim()).or_default().push(segment);
            }
        }
        let mut signal_ends: HashMap<&'a str, Vec<&'a SignalEnd>> = HashMap::new();
        for end in &drawing.signal_ends {
            signal_ends.entry(end.tag.trim()).or_default().push(end);
        }
        Self {
            drawing,
            tags,
            equipment,
            ports,
            symbols,
            loops,
            lines,
            runs,
            signals,
            line_runs,
            line_segments,
            run_segments,
            signal_ends,
        }
    }

    /// 位号是 `tag` 的管线；图上没有或不止一条时是 `None`。
    fn line(&self, tag: &str) -> Option<&'a SummaryObject> {
        single(self.lines.get(tag.trim()))
    }

    /// 位号是 `tag` 的管路；图上没有或不止一条时是 `None`。
    fn run(&self, tag: &str) -> Option<&'a SummaryObject> {
        single(self.runs.get(tag.trim()))
    }

    /// 位号是 `tag` 的信号线；图上没有或不止一条时是 `None`。
    fn signal(&self, tag: &str) -> Option<&'a SummaryObject> {
        single(self.signals.get(tag.trim()))
    }

    /// 这张图的摘要。
    pub fn drawing(&self) -> &'a DrawingSummary {
        self.drawing
    }

    /// 类型编码是 `type_code`、位号是 `tag` 的设备；图上没有或不止一台时是 `None`。
    fn equipment(&self, type_code: &'a str, tag: &'a str) -> Option<&'a SummaryObject> {
        single(self.equipment.get(&(type_code, tag.trim())))
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
    /// 摘要 `segments[]` 的一项。
    Segment(&'a SummarySegment),
    /// 摘要 `ports[]` 里属主是设备的一项：一个管口。
    Port(&'a SummaryPort),
    /// 摘要 `connectors[]` 的一项：一个页间连接符。
    Connector(&'a SummaryConnector),
    Loop(&'a Loop<'a>),
    /// 整张图。
    Drawing,
}

impl Subject<'_> {
    /// 作用对象的类型编码；仪表回路和图纸没有。
    pub fn type_code(&self) -> Option<&str> {
        match self {
            Self::Object(object) => Some(&object.type_code),
            Self::Segment(segment) => Some(segment_type(segment)),
            Self::Port(port) => Some(&port.type_code),
            Self::Connector(plug) => Some(connector_type(plug)),
            Self::Loop(_) | Self::Drawing => None,
        }
    }
}

/// `sheet` 上作用对象是 `target` 的那些，按摘要里的顺序；仪表回路按第一个仪表出现的
/// 先后；管口是属主为设备的端口。写类型编码的含它的下级类型，下级按 `types` 认。
pub fn subjects<'a>(sheet: &'a Sheet<'a>, target: &Target, types: &TypeLib) -> Vec<Subject<'a>> {
    match target {
        Target::Kind(Kind::Segment) => sheet
            .drawing
            .segments
            .iter()
            .map(Subject::Segment)
            .collect(),
        Target::Kind(Kind::Nozzle) => sheet
            .drawing
            .ports
            .iter()
            .filter(|port| port.owner_kind == ObjectKind::Equipment)
            .map(Subject::Port)
            .collect(),
        Target::Kind(Kind::Connector) => sheet
            .drawing
            .connectors
            .iter()
            .map(Subject::Connector)
            .collect(),
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

    /// 结果里的对象引用（§3.6）。管段没有位号，属主是它的管路或管线，位置是起点。管口没有位号，
    /// 类型编码和属主是设备的，句柄是那台设备的，图上没有或不止一台时没有句柄。页间连接符没有
    /// 位号，类型编码是它那一族线的，属主是它所在的线。仪表回路和图纸没有类型编码和句柄；回路的
    /// `tag` 是回路号，位置取回路里第一个仪表的。
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
            Subject::Segment(segment) => ObjectRef {
                kind: SEGMENT.to_owned(),
                type_code: segment_type(segment).to_owned(),
                tag: None,
                drawing_id,
                owner: text(segment_owner(segment)),
                port: None,
                at: segment.a,
                handle: Some(segment.handle.clone()),
            },
            Subject::Port(port) => ObjectRef {
                kind: NOZZLE.to_owned(),
                type_code: port.type_code.clone(),
                tag: None,
                drawing_id,
                owner: text(&port.tag),
                port: Some(port.port.clone()),
                at: port.at,
                handle: self
                    .sheet
                    .equipment(&port.type_code, &port.tag)
                    .map(|equipment| equipment.handle.clone()),
            },
            Subject::Connector(plug) => ObjectRef {
                kind: CONNECTOR.to_owned(),
                type_code: connector_type(plug).to_owned(),
                tag: None,
                drawing_id,
                owner: text(&plug.line),
                port: None,
                at: plug.at,
                handle: Some(plug.handle.clone()),
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
    /// 编号的是「类型编码|所属|x,y」；管段是「类型编码|所属|起点」，管口是「设备类型编码|设备位号#
    /// 端口名」，页间连接符是「类型编码|所在线|x,y」，仪表回路是「仪表回路|回路号」，图纸是
    /// 「图纸|图号」。
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
            Subject::Segment(segment) => format!(
                "{}|{}|{}",
                segment_type(segment),
                segment_owner(segment),
                point(segment.a)
            ),
            Subject::Port(port) => format!("{}|{}#{}", port.type_code, port.tag.trim(), port.port),
            Subject::Connector(plug) => format!(
                "{}|{}|{}",
                connector_type(plug),
                plug.line.trim(),
                point(plug.at)
            ),
            Subject::Loop(group) => format!("{LOOP}|{}", group.number),
            Subject::Drawing => format!("{DRAWING}|{}", self.sheet.drawing.drawing_id),
        }
    }

    /// 没写消息模板时消息里的「位号或位置」：管段是起点，管口是「设备位号#端口名」，页间连接符是
    /// 它的位置，仪表回路是回路号，图纸是图号。
    pub fn location(&self) -> String {
        match self.subject {
            Subject::Object(object) => text(&object.tag).unwrap_or_else(|| point(object.at)),
            Subject::Segment(segment) => point(segment.a),
            Subject::Connector(plug) => point(plug.at),
            Subject::Port(port) => format!("{}#{}", port.tag.trim(), port.port),
            Subject::Loop(group) => group.number.to_owned(),
            Subject::Drawing => self.sheet.drawing.drawing_id.clone(),
        }
    }

    /// 消息模板里 `{name}` 填的值（§3.1）；没有值是空串。仪表回路的 `{位号}` 是回路号，
    /// 管段、管口、页间连接符、仪表回路和图纸的 `{类型}` 是种类名，管段的 `{所属}` 是它的管路或
    /// 管线，管口的是设备位号，页间连接符的是它所在的线；`{缺失字段}` 是缺的必填属性名。
    pub fn placeholder(&self, name: &str) -> String {
        let value = match (name, self.subject) {
            ("位号", Subject::Object(object)) => Some(object.tag.clone()),
            ("位号", Subject::Loop(group)) => Some(group.number.to_owned()),
            ("类型", Subject::Object(object)) => Some(object.type_label.clone()),
            ("类型", Subject::Segment(_)) => Some(SEGMENT.to_owned()),
            ("类型", Subject::Port(_)) => Some(NOZZLE.to_owned()),
            ("类型", Subject::Connector(_)) => Some(CONNECTOR.to_owned()),
            ("类型", Subject::Loop(_)) => Some(LOOP.to_owned()),
            ("类型", Subject::Drawing) => Some(DRAWING.to_owned()),
            ("所属", Subject::Object(object)) => object.owner.clone(),
            ("所属", Subject::Segment(segment)) => Some(segment_owner(segment).to_owned()),
            ("所属", Subject::Port(port)) => Some(port.tag.clone()),
            ("所属", Subject::Connector(plug)) => Some(plug.line.clone()),
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

    /// 结果的 `points`（§3.6）：`names` 里的派生事实指到的位置，按名字、再按线段的先后，不重复。
    /// `端口未连接数` 是没接上的端点，`直连异径管路数` 是与异径管路相接的地方。
    pub fn points(&self, names: &[&str]) -> Vec<[f64; 2]> {
        let mut points: Vec<[f64; 2]> = Vec::new();
        for name in names {
            let found = match (*name, self.subject) {
                ("端口未连接数", _) => self.loose_ends(),
                ("直连异径管路数", Subject::Object(object)) => {
                    self.reducers(object).map(|(_, at)| at)
                }
                _ => None,
            };
            for at in found.into_iter().flatten() {
                if !points.contains(&at) {
                    points.push(at);
                }
            }
        }
        points
    }

    fn value(&self, name: &str) -> Option<String> {
        if !self.kind.has_fact(name) {
            return match self.subject {
                Subject::Object(object) => object.props.get(name).cloned(),
                Subject::Segment(_)
                | Subject::Port(_)
                | Subject::Connector(_)
                | Subject::Loop(_)
                | Subject::Drawing => None,
            };
        }
        match self.subject {
            Subject::Object(object) => self.object_fact(object, name),
            Subject::Segment(segment) => self.segment_fact(segment, name),
            Subject::Port(port) => nozzle_fact(port, name),
            Subject::Connector(plug) => self.connector_fact(plug, name),
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
            "端口未连接数" => self.loose_ends().map(|ends| ends.len().to_string()),
            "长度" => self
                .own_segments(object)
                .map(|segments| total(segments).to_string()),
            "管路数" => self.pipe_runs(object).map(|count| count.to_string()),
            "管段数" => self
                .own_segments(object)
                .map(|segments| segments.len().to_string()),
            "后缀" => suffix(object).map(str::to_owned),
            "直连异径管路数" => self.reducers(object).map(|(count, _)| count.to_string()),
            "未标记覆盖数" => self.unmarked(object).map(|count| count.to_string()),
            "未连接端口数" => self.loose_ports(object).map(|count| count.to_string()),
            "起点仪表" => self.instruments(object).and_then(|[from, _]| from),
            "终点仪表" => self.instruments(object).and_then(|[_, to]| to),
            "接到设备管口数" => self.equipment_ends(object).map(|count| count.to_string()),
            _ => None,
        }
    }

    /// 管段自己的事实。摘要里的线段不记图层，`图层` 不知道。
    fn segment_fact(&self, segment: &SummarySegment, name: &str) -> Option<String> {
        match name {
            "长度" => Some(length(segment).to_string()),
            "端口未连接数" => self.loose_ends().map(|ends| ends.len().to_string()),
            "重叠管段数" => Some(self.overlaps(segment).to_string()),
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
        let (type_code, tag) = (object.type_code.as_str(), object.tag.trim());
        let sheet = self.sheet;
        if sheet.equipment.get(&(type_code, tag)).map_or(0, Vec::len) > 1 {
            return None;
        }
        let ports = sheet.ports.get(&(ObjectKind::Equipment, type_code, tag));
        Some(ports.map_or(0, Vec::len))
    }

    /// `未连接端口数`：这个符号的端口里没接上的个数，按种类、类型编码和位号认端口。不带符号的、
    /// 图上同种同类型同位号的符号不止一个时分不清端口是谁的，不知道。
    fn loose_ports(&self, object: &SummaryObject) -> Option<usize> {
        object.symbol.as_ref()?;
        let key = (object.kind, object.type_code.as_str(), object.tag.trim());
        if self.sheet.symbols.get(&key).copied().unwrap_or(0) > 1 {
            return None;
        }
        let ports = self.sheet.ports.get(&key).into_iter().flatten();
        Some(ports.filter(|port| !port.joined).count())
    }

    /// 信号线的两端，按摘要里的顺序。图上同位号的信号线不止一根时分不清是谁的，不知道。
    fn signal_ends(&self, line: &SummaryObject) -> Option<&'a [&'a SignalEnd]> {
        if line.kind != ObjectKind::SignalLine {
            return None;
        }
        let tag = line.tag.trim();
        if self.sheet.signals.get(tag).map_or(0, Vec::len) != 1 {
            return None;
        }
        let ends = self.sheet.signal_ends.get(tag);
        Some(ends.map(Vec::as_slice).unwrap_or_default())
    }

    /// `起点仪表`、`终点仪表`：属性里填了任一个就都按属性；都没填时看两端接在哪个仪表的端口上，
    /// 按两端的先后，只接着一个时终点仪表不知道。
    fn instruments(&self, line: &SummaryObject) -> Option<[Option<String>; 2]> {
        if line.kind != ObjectKind::SignalLine {
            return None;
        }
        let from = text(prop(line, "起点仪表"));
        let to = text(prop(line, "终点仪表"));
        if from.is_some() || to.is_some() {
            return Some([from, to]);
        }
        let mut found = self.signal_ends(line)?.iter().filter_map(|end| {
            let joint = end.joint.as_ref()?;
            self.port_owner(joint, ObjectKind::Instrument)
        });
        Some([
            found.next().map(str::to_owned),
            found.next().map(str::to_owned),
        ])
    }

    /// `接到设备管口数`：两端里接在设备端口上的个数。
    fn equipment_ends(&self, line: &SummaryObject) -> Option<usize> {
        let count = self
            .signal_ends(line)?
            .iter()
            .filter_map(|end| end.joint.as_ref())
            .filter(|joint| self.port_owner(joint, ObjectKind::Equipment).is_some())
            .count();
        Some(count)
    }

    /// 接点是 `kind` 这种对象的端口时那个对象的位号：图上要有属主种类、类型编码、位号和端口名都
    /// 对上的端口。
    fn port_owner<'j>(&self, joint: &'j Joint, kind: ObjectKind) -> Option<&'j str> {
        let Joint::Port {
            type_code,
            tag,
            port,
        } = joint
        else {
            return None;
        };
        let tag = tag.trim();
        let ports = self.sheet.ports.get(&(kind, type_code.as_str(), tag))?;
        ports.iter().any(|found| found.port == *port).then_some(tag)
    }

    /// 管线、管路的位号：去掉首尾空白不空、图上同种同位号的只有它一条。不然分不清名下的线段
    /// 和管路，不知道。
    fn pipe_tag<'o>(&self, object: &'o SummaryObject) -> Option<&'o str> {
        let pipes = match object.kind {
            ObjectKind::PipeLine => &self.sheet.lines,
            ObjectKind::PipeRun => &self.sheet.runs,
            _ => return None,
        };
        let tag = object.tag.trim();
        (pipes.get(tag).map_or(0, Vec::len) == 1).then_some(tag)
    }

    /// 管线、管路名下的线段：摘要里记在这个管线、管路位号下的；管线的含它的管路的。
    fn own_segments(&self, object: &SummaryObject) -> Option<&'a [&'a SummarySegment]> {
        let tag = self.pipe_tag(object)?;
        let segments = match object.kind {
            ObjectKind::PipeLine => &self.sheet.line_segments,
            _ => &self.sheet.run_segments,
        };
        Some(segments.get(tag).map(Vec::as_slice).unwrap_or_default())
    }

    /// `管路数`：属主是这条管线的管路数。
    fn pipe_runs(&self, object: &SummaryObject) -> Option<usize> {
        if object.kind != ObjectKind::PipeLine {
            return None;
        }
        let tag = self.pipe_tag(object)?;
        Some(self.sheet.line_runs.get(tag).copied().unwrap_or(0))
    }

    /// 没接上的端点：管段是它两端里的，管线、管路是名下线段两端里的，信号线是它两端里的。什么都
    /// 没接才算，接到打断符的不算。
    fn loose_ends(&self) -> Option<Vec<[f64; 2]>> {
        match self.subject {
            Subject::Segment(segment) => Some(open_ends(segment).collect()),
            Subject::Object(object) if object.kind == ObjectKind::SignalLine => {
                let ends = self.signal_ends(object)?;
                Some(
                    ends.iter()
                        .filter(|end| end.joint.is_none())
                        .map(|end| end.at)
                        .collect(),
                )
            }
            Subject::Object(object) => {
                let segments = self.own_segments(object)?;
                Some(segments.iter().copied().flat_map(open_ends).collect())
            }
            Subject::Port(_) | Subject::Connector(_) | Subject::Loop(_) | Subject::Drawing => None,
        }
    }

    /// `重叠管段数`：图上与这段共线、重叠长度超过容差的其他线段数。
    fn overlaps(&self, segment: &SummarySegment) -> usize {
        let Some(along) = Along::new(segment) else {
            return 0;
        };
        self.sheet
            .drawing
            .segments
            .iter()
            .filter(|other| {
                !std::ptr::eq(*other, segment) && along.overlap(other) > DEFAULT_TOLERANCE
            })
            .count()
    }

    /// `直连异径管路数` 和相接的地方：这条管路的线段端点直接接在别的管路的线段端点上（不是接在
    /// 中间，也没隔着管件），两条管路的公称直径不同。按这条管路线段两端记的接点认；对面的管路
    /// 图上没有或不止一条、任一方没填直径的不算。
    fn reducers(&self, run: &SummaryObject) -> Option<(usize, Vec<[f64; 2]>)> {
        if run.kind != ObjectKind::PipeRun {
            return None;
        }
        let segments = self.own_segments(run)?;
        let tag = run.tag.trim();
        let mut others: Vec<&str> = Vec::new();
        let mut points: Vec<[f64; 2]> = Vec::new();
        for (joint, at) in segments.iter().copied().flat_map(ends) {
            let Some(other) = joint.and_then(direct_run) else {
                continue;
            };
            let neighbour = self.sheet.run(other).filter(|_| other != tag);
            if !neighbour.is_some_and(|neighbour| differs(diameter(run), diameter(neighbour))) {
                continue;
            }
            if !others.contains(&other) {
                others.push(other);
            }
            if !points.contains(&at) {
                points.push(at);
            }
        }
        Some((others.len(), points))
    }

    /// `未标记覆盖数`：五个管级属性里，管路的值与所属管线的不同、又不在覆盖标记里的个数。值去掉
    /// 首尾空白再比，没填当空。所属管线图上没有或不止一条时不知道。
    fn unmarked(&self, run: &SummaryObject) -> Option<usize> {
        if run.kind != ObjectKind::PipeRun {
            return None;
        }
        let line = self.sheet.line(owner(run)?)?;
        let count = LINE_FIELDS
            .into_iter()
            .filter(|&name| prop(run, name) != prop(line, name))
            .filter(|&name| !run.overrides.iter().any(|mark| mark == name))
            .count();
        Some(count)
    }

    /// 关系 `所属管路`、`所属管线`、`所属设备`、`所在线`、`对端线` 走到的对象上的事实。走不到时
    /// 是 `None`；`对端线` 要项目索引里配成了对，对端那张图也在手上。
    fn related(&self, relation: &str) -> Option<SubjectFacts<'a>> {
        let mut sheet = self.sheet;
        let (kind, found) = match (relation, self.subject) {
            ("所属管路", _) => (Kind::PipeRun, self.sheet.run(self.run_tag()?)?),
            ("所属管线", _) => (Kind::PipeLine, self.sheet.line(self.line_tag()?)?),
            ("所属设备", Subject::Port(port)) => (
                Kind::Equipment,
                self.sheet.equipment(&port.type_code, &port.tag)?,
            ),
            ("所在线", Subject::Connector(plug)) => carrier(sheet, plug, &plug.line)?,
            ("对端线", Subject::Connector(plug)) => {
                let project = self.context.project.as_ref()?;
                let drawing_id = self.sheet.drawing.drawing_id.as_str();
                let Pairing::Paired(end) = project.pairing(drawing_id, &plug.handle)? else {
                    return None;
                };
                sheet = project.sheets.get(end.drawing_id.as_str()).copied()?;
                carrier(sheet, plug, &end.line)?
            }
            _ => return None,
        };
        Some(SubjectFacts {
            subject: Subject::Object(found),
            kind,
            sheet,
            ..*self
        })
    }

    /// 所属管路的位号：管段记的管路；线上的阀门、管件、打断符等是属主。管线、管路自己没有。
    fn run_tag(&self) -> Option<&'a str> {
        match self.subject {
            Subject::Segment(segment) => segment.run.as_deref(),
            Subject::Object(object) => match object.kind {
                ObjectKind::PipeLine | ObjectKind::PipeRun => None,
                _ => owner(object),
            },
            Subject::Port(_) | Subject::Connector(_) | Subject::Loop(_) | Subject::Drawing => None,
        }
    }

    /// 所属管线的位号：管段记的管线；管路是属主；别的对象经所属管路找。管线自己没有。
    fn line_tag(&self) -> Option<&'a str> {
        match self.subject {
            Subject::Segment(segment) => Some(segment.line.as_str()),
            Subject::Object(object) => match object.kind {
                ObjectKind::PipeLine => None,
                ObjectKind::PipeRun => owner(object),
                _ => owner(self.sheet.run(self.run_tag()?)?),
            },
            Subject::Port(_) | Subject::Connector(_) | Subject::Loop(_) | Subject::Drawing => None,
        }
    }

    /// 页间连接符自己的事实（§3.4）：`连接符种类` 是摘要里记的种类；`对端图纸` 是记着的对端图，
    /// 同进程配的对是「本机配对」，没记的是 `NULL`；`对端位号` 在两侧线号都记着时才有，流出的
    /// 取 `downstream`、流入的取 `upstream`；`配对状态` 照项目索引，没有索引或索引里没有它时不知道。
    fn connector_fact(&self, plug: &'a SummaryConnector, name: &str) -> Option<String> {
        match name {
            "连接符种类" => text(&plug.kind),
            "对端图纸" => match plug.peer_kind {
                PeerKind::Target | PeerKind::Server => plug.peer_drawing.as_deref().and_then(text),
                PeerKind::Local => Some(LOCAL_PEER.to_owned()),
                PeerKind::None => None,
            },
            "对端位号" => {
                let upstream = plug.upstream.as_deref()?;
                let downstream = plug.downstream.as_deref()?;
                let inflow = ConnectorKind::parse(&plug.kind)?.is_inflow();
                text(if inflow { upstream } else { downstream })
            }
            "配对状态" => {
                let project = self.context.project.as_ref()?;
                let drawing_id = self.sheet.drawing.drawing_id.as_str();
                let status = match project.pairing(drawing_id, &plug.handle)? {
                    Pairing::Paired(_) => "paired",
                    Pairing::Unresolved(status) => status_name(status),
                };
                Some(status.to_owned())
            }
            _ => None,
        }
    }

    fn drawing_fact(&self, name: &str) -> Option<String> {
        let drawing = self.sheet.drawing;
        match name {
            "图号" => Some(drawing.drawing_id.clone()),
            "PBS" => drawing.pbs.clone(),
            "对象数" => Some(drawing.objects.len().to_string()),
            LEGEND => self.master()?.legend_standard.clone(),
            _ => None,
        }
    }

    /// 配好了的项目主数据；没配置或读不出来是 `None`。
    fn master(&self) -> Option<&MasterData> {
        match self.context.master {
            Resolved::Configured { data, .. } => Some(data),
            Resolved::Unreadable { .. } | Resolved::NotConfigured => None,
        }
    }

    /// 根 `项目.<名>`（主数据规格 §3.5）：`图例标准` 是一个值，四张表见 [`master_list`]。主数据
    /// 没配置或读不出来时都取不到。
    fn project(&self, name: &str) -> Fact {
        let master = self.master();
        if name == LEGEND {
            return Fact::Value(master.and_then(|data| data.legend_standard.clone()));
        }
        Fact::List(master.and_then(|data| master_list(data, name)))
    }

    /// `管道等级.<名>`、`绝热等级.<名>`、`流体介质.<名>`：按对象的同名属性在主数据里找名称逐字
    /// 相同的那一行，取它的扩展属性 `<名>`。不是这三个关系、属性没填、主数据没配好、表里没有
    /// 这一行或这一行没有这个扩展属性时取不到。
    fn master_attr(&self, relation: &str, name: &str) -> Option<String> {
        let table = master_table(relation)?;
        let Subject::Object(object) = self.subject else {
            return None;
        };
        let value = prop(object, relation);
        if value.is_empty() {
            return None;
        }
        let entry = self.master()?.entry(table, value)?;
        entry.attr(name).map(str::to_owned)
    }

    /// `位号.长度`、`位号.同类型重复数`、`位号.跨图重复数`。没有自己编号或位号为空时不知道；位号
    /// 引擎标了重复的，同类型重复数至少算 2。跨图重复数数项目索引里类型编码、位号都相同的对象还
    /// 落在几张别的图上，没有项目索引时不知道。
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
            "跨图重复数" => {
                let project = self.context.project.as_ref()?;
                let count = project.elsewhere(object, &self.sheet.drawing.drawing_id);
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
            [first, rest @ ..]
                if matches!(
                    first.as_str(),
                    "所属管路" | "所属管线" | "所属设备" | "所在线" | "对端线"
                ) =>
            {
                self.related(first).map_or(Fact::Value(None), |facts| {
                    facts.fact(&Path {
                        segments: rest.to_vec(),
                    })
                })
            }
            [root, name] if root == PROJECT => self.project(name),
            [relation, name] => Fact::Value(self.master_attr(relation, name)),
            _ => Fact::Value(None),
        }
    }
}

/// 路径要不要读主数据的表（主数据规格 §3.5）：根 `项目` 下的四张表，或经 `管道等级`、`绝热等级`、
/// `流体介质` 到主数据里取扩展属性。`项目.图例标准` 不算：主数据没配置时它是 `NULL`，照样能下结论。
pub fn reads_master(segments: &[String]) -> bool {
    match segments {
        [root, name] if root == PROJECT => name != LEGEND,
        [relations @ .., _] => relations
            .iter()
            .any(|relation| master_table(relation).is_some()),
        [] => false,
    }
}

/// 路径要不要项目索引（§3.4）：页间连接符的 `配对状态`、经 `对端线` 走到别的图上、
/// `位号.跨图重复数`。`kind` 是规则的作用对象；用到这些的规则是项目级规则。
pub fn reads_index(kind: Kind, segments: &[String]) -> bool {
    let connector = kind == Kind::Connector;
    match segments {
        [name] => connector && name == "配对状态",
        [first, ..] if connector && first == "对端线" => true,
        _ => segments
            .windows(2)
            .any(|pair| pair[0] == "位号" && pair[1] == "跨图重复数"),
    }
}

/// 关系 `relation` 到主数据里找的表；不是按主数据找的关系是 `None`。
fn master_table(relation: &str) -> Option<Table> {
    MASTER_RELATIONS
        .iter()
        .find(|(name, _)| *name == relation)
        .map(|&(_, table)| table)
}

/// 主数据的一张表（根 `项目` 下的名字），连同表里的值怎么比：`公称直径表` 是没隐藏的公称直径，
/// 按公称直径比；另三张是名称，逐字比。配好了而表是空的，取得到一张空表。不是这四张表是 `None`。
fn master_list(data: &MasterData, name: &str) -> Option<FactList> {
    let table = match name {
        "公称直径表" => {
            let items = data
                .nominal_diameters
                .iter()
                .filter(|diameter| !diameter.hidden)
                .map(|diameter| diameter.dn.to_string())
                .collect();
            return Some(FactList {
                items,
                matching: Matching::NominalDiameter,
            });
        }
        "管道等级表" => Table::PipeClass,
        "绝热等级表" => Table::InsulationClass,
        "流体介质表" => Table::FluidCode,
        _ => return None,
    };
    let items = data.names(table).into_iter().map(str::to_owned).collect();
    Some(FactList {
        items,
        matching: Matching::Exact,
    })
}

/// 管口自己的事实。管口没有位号，`位号前缀` 本期不知道（§3.4）。
fn nozzle_fact(port: &SummaryPort, name: &str) -> Option<String> {
    match name {
        "名称" => Some(port.port.clone()),
        "方向" => port.direction.map(direction_name).map(str::to_owned),
        "连接状态" => Some(joined_name(port.joined).to_owned()),
        _ => None,
    }
}

/// 端口流向写成文本。
fn direction_name(direction: PortDir) -> &'static str {
    match direction {
        PortDir::In => "流入",
        PortDir::Out => "流出",
        PortDir::Bidir => "双向",
    }
}

/// 端口接没接上写成文本。
fn joined_name(joined: bool) -> &'static str {
    if joined {
        "已连接"
    } else {
        "未连接"
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

/// 对象的属主，去掉首尾空白；没有或空的是 `None`。
fn owner(object: &SummaryObject) -> Option<&str> {
    let owner = object.owner.as_deref()?.trim();
    (!owner.is_empty()).then_some(owner)
}

/// 只有一个时取那一个。
fn single<'a>(found: Option<&Vec<&'a SummaryObject>>) -> Option<&'a SummaryObject> {
    match found?.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// 属性值去掉首尾空白；没有这个属性是空串。
fn prop<'o>(object: &'o SummaryObject, name: &str) -> &'o str {
    object.props.get(name).map_or("", |value| value.trim())
}

/// `后缀`：管路位号去掉「所属管线位号 + `-`」剩下的部分；不是这个形状的不知道。
fn suffix(run: &SummaryObject) -> Option<&str> {
    if run.kind != ObjectKind::PipeRun {
        return None;
    }
    run.tag.trim().strip_prefix(owner(run)?)?.strip_prefix('-')
}

/// 管路的公称直径，去掉首尾空白；没填的是 `None`。
fn diameter(run: &SummaryObject) -> Option<&str> {
    Some(prop(run, "公称直径")).filter(|value| !value.is_empty())
}

/// 两个公称直径不同：都读得出数时比数（`DN80` 与 `80` 相同），不然比文本。任一方没填不算不同。
fn differs(one: Option<&str>, other: Option<&str>) -> bool {
    let (Some(one), Some(other)) = (one, other) else {
        return false;
    };
    match (parse_dn(one), parse_dn(other)) {
        (Some(a), Some(b)) => a != b,
        _ => one != other,
    }
}

/// 端点直接接着的管路的位号：接在别的线段的端点上（不是接在中间）、那段线是管路的。
fn direct_run(joint: &Joint) -> Option<&str> {
    match joint {
        Joint::Segment {
            run: Some(run),
            tee: false,
            ..
        } => Some(run.trim()),
        _ => None,
    }
}

/// 管段的类型编码：管路的线是管段，管线自己的线是管线。
fn segment_type(segment: &SummarySegment) -> &'static str {
    if segment.run.is_some() {
        TYPE_PIPE_SEGMENT
    } else {
        TYPE_PIPE_LINE
    }
}

/// 管段的所属：管路的线是管路位号，管线自己的线是管线位号。去掉首尾空白。
fn segment_owner(segment: &SummarySegment) -> &str {
    segment.run.as_deref().unwrap_or(&segment.line).trim()
}

/// 线段两端：接点和坐标。
fn ends(segment: &SummarySegment) -> [(Option<&Joint>, [f64; 2]); 2] {
    [
        (segment.joint_a.as_ref(), segment.a),
        (segment.joint_b.as_ref(), segment.b),
    ]
}

/// 线段两端里没接上的：什么都没接。接到打断符的有接点，不算悬空。
fn open_ends(segment: &SummarySegment) -> impl Iterator<Item = [f64; 2]> + '_ {
    ends(segment)
        .into_iter()
        .filter(|(joint, _)| joint.is_none())
        .map(|(_, at)| at)
}

/// 线段长度：两端距离，不超过容差的算 0。
fn length(segment: &SummarySegment) -> f64 {
    let length = (segment.b[0] - segment.a[0]).hypot(segment.b[1] - segment.a[1]);
    if length > DEFAULT_TOLERANCE {
        length
    } else {
        0.0
    }
}

/// 线段长度之和。从 0 加起：浮点数的 `sum` 对空表得 `-0`，写成文本是 `-0`。
fn total(segments: &[&SummarySegment]) -> f64 {
    segments
        .iter()
        .fold(0.0, |sum, segment| sum + length(segment))
}

/// 一段线所在的直线：从这段的起点量，量到终点是这段的长度。
struct Along {
    origin: [f64; 2],
    direction: [f64; 2],
    length: f64,
}

impl Along {
    /// 长度为 0 的线段定不出方向，是 `None`。
    fn new(segment: &SummarySegment) -> Option<Self> {
        let span = length(segment);
        if span <= 0.0 {
            return None;
        }
        Some(Self {
            origin: segment.a,
            direction: [
                (segment.b[0] - segment.a[0]) / span,
                (segment.b[1] - segment.a[1]) / span,
            ],
            length: span,
        })
    }

    /// 与 `other` 重叠的长度：`other` 两端离这条直线都不超过容差时，是两段在直线上的公共部分；
    /// 不然是 0。
    fn overlap(&self, other: &SummarySegment) -> f64 {
        let (from, off_from) = self.project(other.a);
        let (to, off_to) = self.project(other.b);
        if off_from > DEFAULT_TOLERANCE || off_to > DEFAULT_TOLERANCE {
            return 0.0;
        }
        (from.max(to).min(self.length) - from.min(to).max(0.0)).max(0.0)
    }

    /// 点在直线上的位置和离开直线的距离。
    fn project(&self, at: [f64; 2]) -> (f64, f64) {
        let dx = at[0] - self.origin[0];
        let dy = at[1] - self.origin[1];
        let along = dx * self.direction[0] + dy * self.direction[1];
        let off = (dx * self.direction[1] - dy * self.direction[0]).abs();
        (along, off)
    }
}

/// 页间连接符的类型编码：管道连接符是管线的，信号连接符是信号线的；种类认不出时是摘要里记的
/// 种类。
fn connector_type(plug: &SummaryConnector) -> &str {
    match ConnectorKind::parse(&plug.kind) {
        Some(kind) => kind.type_code(),
        None => &plug.kind,
    }
}

/// 连接符 `plug` 那一族里位号是 `tag` 的线：管道连接符找管线，信号连接符找信号线。种类认不出、
/// 图上没有或不止一条时是 `None`。
fn carrier<'a>(
    sheet: &Sheet<'a>,
    plug: &SummaryConnector,
    tag: &str,
) -> Option<(Kind, &'a SummaryObject)> {
    match ConnectorKind::parse(&plug.kind)? {
        ConnectorKind::PipeIn | ConnectorKind::PipeOut => Some((Kind::PipeLine, sheet.line(tag)?)),
        ConnectorKind::SignalIn | ConnectorKind::SignalOut => {
            Some((Kind::SignalLine, sheet.signal(tag)?))
        }
    }
}

/// 配对状态在项目索引里的写法（§3.6）。
fn status_name(status: LinkStatus) -> &'static str {
    match status {
        LinkStatus::OneSided => "one_sided",
        LinkStatus::TargetMissing => "target_missing",
        LinkStatus::Unpaired => "unpaired",
        LinkStatus::Ambiguous => "ambiguous",
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

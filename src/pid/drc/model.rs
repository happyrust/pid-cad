//! 规则检查用到的数据（`SPEC-pid-drc.md` §3）。现在是规则的作用对象（§3.1 的 `target`）
//! 和 §3.4 的名字表：每种对象有哪些派生事实，哪些关系能接 `.` 往下取，以及哪种
//! 对象都能用的根；还有规则包（§3.1）读出来的规则、规则集和读不进来的原因，以及
//! 检查结果（§3.6）。

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Serialize, Serializer};
use serde_json::Value;

use super::dsl::{DslError, Expr};

/// 对象种类（§3.4 的表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 对象实例：摘要 `objects[]` 的每一项。
    Object,
    Equipment,
    Valve,
    Fitting,
    Instrument,
    PipeLine,
    PipeRun,
    /// 管段：摘要 `segments[]` 的每一项（含管线自己的线）。
    Segment,
    /// 管口：设备符号的每个端口。
    Nozzle,
    SignalLine,
    /// 页间连接符：摘要 `connectors[]` 的每一项。
    Connector,
    /// 仪表回路：`仪表回路` 属性值相同的一组仪表。
    Loop,
    Drawing,
    /// 符号实例：带符号的对象。
    Symbol,
    /// 标注：本期没有这种对象。
    Annotation,
}

/// 一个关系后面还能接什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// 只能再接一段，是这几个名字之一（`位号.长度`、`项目.管道等级表`）。
    OneOf(&'static [&'static str]),
    /// 只能再接一段，名字不限（`参数.<名>`、`管道等级.<扩展属性>`）。
    AnyName,
    /// 取到另一个对象，后面按那种对象接着取（`所属管线.管道等级.材质`）。
    Object(Kind),
}

/// 名字表里的一个关系：多段路径的第一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relation {
    pub name: &'static str,
    pub next: Next,
}

/// 对象实例的派生事实（§3.4 表的第一行），各类对象都有。
const OBJECT_FACTS: &[&str] = &[
    "位号",
    "位号合规",
    "位号来源",
    "独立编号",
    "种类",
    "类型编码",
    "类型",
    "图层",
    "必填项缺失数",
    "未连接端口数",
];

/// 对象实例的关系（§3.4 表的第一行），各类对象都有。后三个按对象的同名属性在主数据里
/// 找到那一行，后面接那一行的扩展属性。
const OBJECT_RELATIONS: &[Relation] = &[
    Relation {
        name: "位号",
        next: Next::OneOf(&["长度", "同类型重复数", "跨图重复数"]),
    },
    Relation {
        name: "所属管路",
        next: Next::Object(Kind::PipeRun),
    },
    Relation {
        name: "所属管线",
        next: Next::Object(Kind::PipeLine),
    },
    Relation {
        name: "管道等级",
        next: Next::AnyName,
    },
    Relation {
        name: "绝热等级",
        next: Next::AnyName,
    },
    Relation {
        name: "流体介质",
        next: Next::AnyName,
    },
];

const SEGMENT_RELATIONS: &[Relation] = &[
    Relation {
        name: "所属管路",
        next: Next::Object(Kind::PipeRun),
    },
    Relation {
        name: "所属管线",
        next: Next::Object(Kind::PipeLine),
    },
];

const NOZZLE_RELATIONS: &[Relation] = &[Relation {
    name: "所属设备",
    next: Next::Object(Kind::Equipment),
}];

/// 页间连接符所在的线和对端的线是管线或信号线，两者的关系相同，都按管线认。
const CONNECTOR_RELATIONS: &[Relation] = &[
    Relation {
        name: "所在线",
        next: Next::Object(Kind::PipeLine),
    },
    Relation {
        name: "对端线",
        next: Next::Object(Kind::PipeLine),
    },
];

const SYMBOL_RELATIONS: &[Relation] = &[Relation {
    name: "符号",
    next: Next::OneOf(&["状态"]),
}];

/// 哪种对象都能用的根（§3.4 表后），只能写在路径开头。
pub const ROOTS: &[Relation] = &[
    Relation {
        name: "项目",
        next: Next::OneOf(&[
            "管道等级表",
            "公称直径表",
            "绝热等级表",
            "流体介质表",
            "图例标准",
        ]),
    },
    Relation {
        name: "图纸",
        next: Next::OneOf(&["图号", "PBS"]),
    },
    Relation {
        name: "参数",
        next: Next::AnyName,
    },
];

/// 根 `name` 后面能接什么；不是根是 `None`。
pub fn root(name: &str) -> Option<Next> {
    find(ROOTS, name)
}

fn find(relations: &[Relation], name: &str) -> Option<Next> {
    relations
        .iter()
        .find(|relation| relation.name == name)
        .map(|relation| relation.next)
}

impl Kind {
    /// §3.4 表里的顺序。
    pub const ALL: [Kind; 15] = [
        Kind::Object,
        Kind::Equipment,
        Kind::Valve,
        Kind::Fitting,
        Kind::Instrument,
        Kind::PipeLine,
        Kind::PipeRun,
        Kind::Segment,
        Kind::Nozzle,
        Kind::SignalLine,
        Kind::Connector,
        Kind::Loop,
        Kind::Drawing,
        Kind::Symbol,
        Kind::Annotation,
    ];

    /// 规则里写的种类名。
    pub fn name(self) -> &'static str {
        match self {
            Kind::Object => "对象实例",
            Kind::Equipment => "设备",
            Kind::Valve => "阀门",
            Kind::Fitting => "管件",
            Kind::Instrument => "仪表",
            Kind::PipeLine => "管线",
            Kind::PipeRun => "管路",
            Kind::Segment => "管段",
            Kind::Nozzle => "管口",
            Kind::SignalLine => "信号线",
            Kind::Connector => "页间连接符",
            Kind::Loop => "仪表回路",
            Kind::Drawing => "图纸",
            Kind::Symbol => "符号实例",
            Kind::Annotation => "标注",
        }
    }

    /// 按种类名找种类。
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// 对象实例的一种：有对象实例的派生事实和关系。
    fn is_object(self) -> bool {
        matches!(
            self,
            Kind::Object
                | Kind::Equipment
                | Kind::Valve
                | Kind::Fitting
                | Kind::Instrument
                | Kind::PipeLine
                | Kind::PipeRun
                | Kind::SignalLine
                | Kind::Symbol
        )
    }

    /// 这种对象自己的派生事实，不含对象实例的。
    fn own_facts(self) -> &'static [&'static str] {
        match self {
            Kind::Equipment => &["管口数", "设备类别"],
            Kind::Valve => &["流向相反"],
            Kind::PipeLine => &["端口未连接数", "长度", "管路数"],
            Kind::PipeRun => &[
                "后缀",
                "端口未连接数",
                "长度",
                "管段数",
                "直连异径管路数",
                "未标记覆盖数",
            ],
            Kind::Segment => &["长度", "端口未连接数", "重叠管段数", "图层"],
            Kind::Nozzle => &["名称", "方向", "连接状态", "位号前缀"],
            Kind::SignalLine => &["起点仪表", "终点仪表", "端口未连接数", "接到设备管口数"],
            Kind::Connector => &["连接符种类", "对端图纸", "对端位号", "配对状态"],
            Kind::Loop => &["回路号", "仪表数", "控制器数", "测量元件数"],
            Kind::Drawing => &["图号", "图例标准", "PBS", "对象数"],
            Kind::Object | Kind::Fitting | Kind::Instrument | Kind::Symbol | Kind::Annotation => {
                &[]
            }
        }
    }

    /// 这种对象自己的关系，不含对象实例的。
    fn own_relations(self) -> &'static [Relation] {
        match self {
            Kind::Segment => SEGMENT_RELATIONS,
            Kind::Nozzle => NOZZLE_RELATIONS,
            Kind::Connector => CONNECTOR_RELATIONS,
            Kind::Symbol => SYMBOL_RELATIONS,
            Kind::Object
            | Kind::Equipment
            | Kind::Valve
            | Kind::Fitting
            | Kind::Instrument
            | Kind::PipeLine
            | Kind::PipeRun
            | Kind::SignalLine
            | Kind::Loop
            | Kind::Drawing
            | Kind::Annotation => &[],
        }
    }

    fn object_facts(self) -> &'static [&'static str] {
        if self.is_object() {
            OBJECT_FACTS
        } else {
            &[]
        }
    }

    fn object_relations(self) -> &'static [Relation] {
        if self.is_object() {
            OBJECT_RELATIONS
        } else {
            &[]
        }
    }

    /// 这种对象的派生事实：属性之外可以直接写的名字。
    pub fn facts(self) -> Vec<&'static str> {
        self.object_facts()
            .iter()
            .chain(self.own_facts())
            .copied()
            .collect()
    }

    /// `name` 是不是这种对象的派生事实。
    pub fn has_fact(self, name: &str) -> bool {
        self.object_facts().contains(&name) || self.own_facts().contains(&name)
    }

    /// 这种对象的关系，不含根。
    pub fn relations(self) -> Vec<Relation> {
        self.object_relations()
            .iter()
            .chain(self.own_relations())
            .copied()
            .collect()
    }

    /// 关系 `name` 后面能接什么；这种对象没有这个关系是 `None`。
    pub fn relation(self, name: &str) -> Option<Next> {
        find(self.object_relations(), name).or_else(|| find(self.own_relations(), name))
    }
}

/// 规则的作用对象（§3.1 的 `target`）：§3.4 的一种对象，或一个类型编码（含它的下级类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Kind(Kind),
    TypeCode(String),
}

impl Target {
    /// `target` 字段的写法：是种类名就是那种对象，否则当类型编码。
    pub fn parse(text: &str) -> Self {
        Kind::from_name(text).map_or_else(|| Self::TypeCode(text.to_owned()), Self::Kind)
    }

    /// 解析表达式时按哪种对象认关系名。类型编码按符号实例认：解析时不知道这个类型的
    /// 对象是设备还是阀门，只知道它们都带符号。
    pub fn kind(&self) -> Kind {
        match self {
            Self::Kind(kind) => *kind,
            Self::TypeCode(_) => Kind::Symbol,
        }
    }

    /// `target` 字段的写法：种类名，或类型编码。
    pub fn name(&self) -> &str {
        match self {
            Self::Kind(kind) => kind.name(),
            Self::TypeCode(code) => code,
        }
    }
}

// ── 规则包 ─────────────────────────────────────────────────────────────────────

/// 严重度（§3.1）：只有三级，结果按这个顺序排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum Severity {
    #[serde(rename = "错误")]
    Error,
    #[serde(rename = "警告")]
    Warning,
    #[serde(rename = "提示")]
    Info,
}

impl Severity {
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Info];

    /// 规则包和检查结果里的写法。
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "错误",
            Self::Warning => "警告",
            Self::Info => "提示",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|severity| severity.name() == name)
    }
}

/// 规则类别：本期只求值设计类，配置类可以写进文件但不跑（§8）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Design,
    Configuration,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Self::Design => "设计类",
            Self::Configuration => "配置类",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Design, Self::Configuration]
            .into_iter()
            .find(|category| category.name() == name)
    }
}

/// 规则从哪来：内置包里的，或项目包新加的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Builtin,
    /// 复制一条内置规则再改。
    Modified,
    New,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Self::Builtin => "内置",
            Self::Modified => "复制修改",
            Self::New => "新建",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Builtin, Self::Modified, Self::New]
            .into_iter()
            .find(|source| source.name() == name)
    }
}

/// 规则在哪里生效（§3.1 的 `scope` 和 `scope_ref`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleScope {
    /// 处处生效。
    Project,
    /// 这个 PBS 节点及其下级的图纸。
    Pbs(String),
    /// 这张图（图号）。
    Drawing(String),
    /// 这个类型及其下级类型的对象（类型编码）。
    Type(String),
}

impl RuleScope {
    /// `scope` 的写法。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Project => "项目",
            Self::Pbs(_) => "PBS 节点",
            Self::Drawing(_) => "图纸",
            Self::Type(_) => "类型",
        }
    }

    /// `scope_ref`：作用域是项目时没有。
    pub fn reference(&self) -> Option<&str> {
        match self {
            Self::Project => None,
            Self::Pbs(reference) | Self::Drawing(reference) | Self::Type(reference) => {
                Some(reference)
            }
        }
    }
}

/// 保存门禁的三档（§4.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveGate {
    /// 保存时不检查。
    Off,
    /// 检查后只提示。
    Warn,
    /// 有错误就拒绝保存。
    Block,
}

impl SaveGate {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Block => "block",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Off, Self::Warn, Self::Block]
            .into_iter()
            .find(|gate| gate.name() == name)
    }
}

/// 一条规则（§3.1）。表达式在读规则包时就按作用对象解析好了。
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    /// 所在的规则集。
    pub set: String,
    pub category: Category,
    pub target: Target,
    pub severity: Severity,
    /// 表达式原文。
    pub expr: String,
    /// 解析好的表达式。
    pub tree: Expr,
    /// 消息模板；没写时用「<名称>：<位号或位置>」。
    pub message: Option<String>,
    /// 修复建议的模板。
    pub fix: Option<String>,
    pub scope: RuleScope,
    /// 这条规则自己的启停；所在的规则集停用时它也不跑，见 [`RuleSet::is_enabled`]。
    pub enabled: bool,
    pub source: Source,
    /// 表达式里的 `参数.<名>` 取这里的值。
    pub params: BTreeMap<String, Value>,
}

/// 规则集（规则包里 `sets[]` 的一项）：可以整组启停的一组规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleGroup {
    pub name: String,
    pub enabled: bool,
}

/// 生效的整套规则：内置包，有项目包时叠上项目包（§3.2）。
#[derive(Debug, Clone, PartialEq)]
pub struct RuleSet {
    /// 规则版本：内置包的 `rule_set_version`；有项目包时是 `<内置版本>+<项目包版本>`。
    pub version: String,
    /// 这套规则是为哪套图例标准写的。本期只记录，不校验。
    pub legend_standard: Option<String>,
    pub save_gate: SaveGate,
    /// 判断两点重合的容差，图纸单位。
    pub connect_tolerance: f64,
    /// 类型编码 → 必填的属性名；下级类型也要填。
    pub required_fields: BTreeMap<String, Vec<String>>,
    /// 仪表回路里的角色 → 符号 id，只有 `DR-113` 用。
    pub loop_roles: BTreeMap<String, Vec<String>>,
    pub sets: Vec<RuleGroup>,
    /// 内置规则在前，项目包新加的按文件里的顺序接在后面。
    pub rules: Vec<Rule>,
    /// 叠上来的项目包文件；只用内置包时是 `None`。
    pub project_pack: Option<PathBuf>,
}

impl RuleSet {
    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    /// 这条规则跑不跑：它自己启用，所在的规则集也启用。
    pub fn is_enabled(&self, rule: &Rule) -> bool {
        rule.enabled
            && self
                .sets
                .iter()
                .any(|set| set.name == rule.set && set.enabled)
    }
}

// ── 检查结果 ──────────────────────────────────────────────────────────────────────

/// 检查结果格式的版本（§3.6 的 `version`）。
pub const REPORT_VERSION: u32 = 1;

/// 什么时候查的（§4.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Trigger {
    #[serde(rename = "按需")]
    OnDemand,
    #[serde(rename = "保存前")]
    BeforeSave,
    #[serde(rename = "发布前")]
    BeforePublish,
}

/// 查哪些图（§4.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Drawing,
    Pbs,
    Project,
}

/// 检查范围（§3.6 的 `scope`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scope {
    pub kind: ScopeKind,
    pub drawing_id: Option<String>,
    pub pbs: Option<String>,
    /// 项目级规则跑没跑。
    pub cross_drawing: bool,
}

/// 一条豁免（§3.7）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Waiver {
    pub rule_id: String,
    pub object_key: String,
    pub reason: String,
    pub by: String,
    /// Unix 秒。
    pub at: i64,
}

/// 一条结果指向的对象（§3.6 的 `results[].object`）：类型编码、位号、图号三样定位一个
/// 对象，没有位号的靠 `owner`、`port`、`at` 补足；`handle` 只用来在图上定位。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObjectRef {
    /// 摘要里的种类名，如 `管线`。
    pub kind: String,
    pub type_code: String,
    pub tag: Option<String>,
    pub drawing_id: String,
    pub owner: Option<String>,
    pub port: Option<String>,
    pub at: [f64; 2],
    pub handle: Option<String>,
}

/// 一条检查结果（§3.6 的 `results[]`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    pub rule_id: String,
    pub rule_name: String,
    pub severity: Severity,
    pub object: ObjectRef,
    /// 豁免用的对象键（§3.7）。
    pub object_key: String,
    pub message: String,
    pub fix: String,
    /// 一条结果涉及多个位置时逐个列出。
    pub points: Vec<[f64; 2]>,
    /// 涉及对象自己的哪些属性，给属性表高亮用。
    pub fields: Vec<String>,
    pub waived: bool,
    /// 结果里只写豁免的 `reason`、`by`、`at`。
    #[serde(serialize_with = "waiver_note")]
    pub waiver: Option<Waiver>,
}

/// 不是违规、但用户该知道的事（§3.6 的 `notices[]`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub code: String,
    pub message: String,
}

/// 计数（§3.6 的 `summary`）。三个严重度只数没被豁免的结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    #[serde(rename = "错误")]
    pub errors: usize,
    #[serde(rename = "警告")]
    pub warnings: usize,
    #[serde(rename = "提示")]
    pub infos: usize,
    #[serde(rename = "已豁免")]
    pub waived: usize,
    /// 查过的图里摘要 `objects[]` 的总数。
    pub objects: usize,
    pub rules_run: usize,
    /// 启用了却没执行的规则数；停用的不算。
    pub rules_skipped: usize,
}

/// 一次检查的结果（§3.6）。应答外面的 `ok` 由接口层加。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckReport {
    pub version: u32,
    pub rule_set_version: String,
    pub rule_set_digest: String,
    /// Unix 秒。
    pub checked_at: i64,
    pub trigger: Trigger,
    pub scope: Scope,
    pub elapsed_ms: u64,
    pub summary: Summary,
    /// 有没被豁免的错误。
    pub blocked: bool,
    /// 按严重度、规则号、图号、对象键排好。
    pub results: Vec<CheckResult>,
    pub notices: Vec<Notice>,
    /// 图里记着、这次却没有对应结果的豁免。
    pub stale_waivers: Vec<Waiver>,
}

fn waiver_note<S: Serializer>(waiver: &Option<Waiver>, serializer: S) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct Note<'a> {
        reason: &'a str,
        by: &'a str,
        at: i64,
    }
    waiver
        .as_ref()
        .map(|waiver| Note {
            reason: &waiver.reason,
            by: &waiver.by,
            at: waiver.at,
        })
        .serialize(serializer)
}

// ── 错误 ──────────────────────────────────────────────────────────────────────

/// 规则包读不进来的原因（§3.8 规则包那几类）。`at` 是出错的位置，如
/// `rules[3].severity`；整个文件是空串，文件读不出来时是文件路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrcError {
    /// 不是合法 JSON、字段类型不对、取值超出约束，或者文件读不出来。
    Schema { at: String, reason: String },
    /// `version` 比程序认识的高。
    VersionUnsupported { at: String, found: u64 },
    /// 规则缺必填字段（没写、`null` 或空串），或者 `scope` 不是 `项目` 而没写
    /// `scope_ref`。缺的是 `id` 时 `rule_id` 为 `None`。
    RequiredFieldEmpty { at: String, rule_id: Option<String> },
    /// `category` 不是 `设计类` 或 `配置类`。
    KindInvalid {
        at: String,
        rule_id: String,
        found: String,
    },
    /// `severity` 不是 `错误`、`警告` 或 `提示`。
    SeverityInvalid {
        at: String,
        rule_id: String,
        found: String,
    },
    /// 项目包里写内置规则的那一项带了 `id`、`enabled`、`params` 以外的键；`at` 是那个键。
    BuiltinReadonly { at: String, rule_id: String },
    /// 项目包新加的规则用了留给内置规则的编号 `DR-001` – `DR-199`。
    IdReserved { at: String, rule_id: String },
    /// 同一个文件里两条规则编号相同；`at` 是后出现的那一条。
    IdDuplicated { at: String, rule_id: String },
    /// 表达式解析不过。
    Dsl {
        at: String,
        rule_id: String,
        error: DslError,
    },
}

impl DrcError {
    /// 错误码（§3.8）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema { .. } => "RULESET_SCHEMA_ERROR",
            Self::VersionUnsupported { .. } => "RULESET_VERSION_UNSUPPORTED",
            Self::RequiredFieldEmpty { .. } => "REQUIRED_FIELD_EMPTY",
            Self::KindInvalid { .. } => "RULE_KIND_INVALID",
            Self::SeverityInvalid { .. } => "SEVERITY_INVALID",
            Self::BuiltinReadonly { .. } => "RULE_BUILTIN_READONLY",
            Self::IdReserved { .. } => "RULE_ID_RESERVED",
            Self::IdDuplicated { .. } => "RULE_ID_DUPLICATED",
            Self::Dsl { error, .. } => error.code(),
        }
    }

    /// 出错的位置。
    pub fn at(&self) -> &str {
        match self {
            Self::Schema { at, .. }
            | Self::VersionUnsupported { at, .. }
            | Self::RequiredFieldEmpty { at, .. }
            | Self::KindInvalid { at, .. }
            | Self::SeverityInvalid { at, .. }
            | Self::BuiltinReadonly { at, .. }
            | Self::IdReserved { at, .. }
            | Self::IdDuplicated { at, .. }
            | Self::Dsl { at, .. } => at,
        }
    }

    /// 出错的那条规则；文件层的错和缺编号的规则没有。
    pub fn rule_id(&self) -> Option<&str> {
        match self {
            Self::Schema { .. } | Self::VersionUnsupported { .. } => None,
            Self::RequiredFieldEmpty { rule_id, .. } => rule_id.as_deref(),
            Self::KindInvalid { rule_id, .. }
            | Self::SeverityInvalid { rule_id, .. }
            | Self::BuiltinReadonly { rule_id, .. }
            | Self::IdReserved { rule_id, .. }
            | Self::IdDuplicated { rule_id, .. }
            | Self::Dsl { rule_id, .. } => Some(rule_id),
        }
    }
}

impl fmt::Display for DrcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.at().is_empty() {
            "顶层"
        } else {
            self.at()
        };
        write!(f, "{}: {at} ", self.code())?;
        match self {
            Self::Schema { reason, .. } => f.write_str(reason)?,
            Self::VersionUnsupported { found, .. } => write!(f, "是 {found}，比程序认识的高")?,
            Self::RequiredFieldEmpty { .. } => f.write_str("不能为空")?,
            Self::KindInvalid { found, .. } => {
                write!(f, "是「{found}」，只能是设计类或配置类")?;
            }
            Self::SeverityInvalid { found, .. } => {
                write!(f, "是「{found}」，只能是错误、警告或提示")?;
            }
            Self::BuiltinReadonly { .. } => {
                f.write_str("改不了：项目包里的内置规则只能带 id、enabled、params")?;
            }
            Self::IdReserved { .. } => f.write_str("落在留给内置规则的 DR-001 – DR-199 里")?,
            Self::IdDuplicated { .. } => f.write_str("与前面的规则重复")?,
            Self::Dsl { error, .. } => {
                write!(f, "偏移 {} 处应为{}", error.offset, error.expected)?;
            }
        }
        match self.rule_id() {
            Some(id) => write!(f, "（规则 {id}）"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for DrcError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// §3.4 的表：种类名、派生事实、关系和根，逐种对一遍。
    #[test]
    fn the_name_table_is_the_one_in_section_3_4() {
        let names: Vec<&str> = Kind::ALL.iter().map(|kind| kind.name()).collect();
        assert_eq!(
            names,
            [
                "对象实例",
                "设备",
                "阀门",
                "管件",
                "仪表",
                "管线",
                "管路",
                "管段",
                "管口",
                "信号线",
                "页间连接符",
                "仪表回路",
                "图纸",
                "符号实例",
                "标注",
            ]
        );
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
            assert_eq!(Target::parse(kind.name()), Target::Kind(kind));
            assert_eq!(Target::parse(kind.name()).kind(), kind);
        }
        assert_eq!(Kind::from_name("泵"), None);
        assert_eq!(Target::parse("PUMP"), Target::TypeCode("PUMP".to_owned()));
        assert_eq!(Target::parse("PUMP").kind(), Kind::Symbol);

        let object = [
            "位号",
            "位号合规",
            "位号来源",
            "独立编号",
            "种类",
            "类型编码",
            "类型",
            "图层",
            "必填项缺失数",
            "未连接端口数",
        ];
        let with = |extra: &[&'static str]| -> Vec<&'static str> {
            object.iter().chain(extra).copied().collect()
        };
        for (kind, facts) in [
            (Kind::Object, with(&[])),
            (Kind::Equipment, with(&["管口数", "设备类别"])),
            (Kind::Valve, with(&["流向相反"])),
            (Kind::Fitting, with(&[])),
            (Kind::Instrument, with(&[])),
            (Kind::PipeLine, with(&["端口未连接数", "长度", "管路数"])),
            (
                Kind::PipeRun,
                with(&[
                    "后缀",
                    "端口未连接数",
                    "长度",
                    "管段数",
                    "直连异径管路数",
                    "未标记覆盖数",
                ]),
            ),
            (
                Kind::Segment,
                vec!["长度", "端口未连接数", "重叠管段数", "图层"],
            ),
            (Kind::Nozzle, vec!["名称", "方向", "连接状态", "位号前缀"]),
            (
                Kind::SignalLine,
                with(&["起点仪表", "终点仪表", "端口未连接数", "接到设备管口数"]),
            ),
            (
                Kind::Connector,
                vec!["连接符种类", "对端图纸", "对端位号", "配对状态"],
            ),
            (
                Kind::Loop,
                vec!["回路号", "仪表数", "控制器数", "测量元件数"],
            ),
            (Kind::Drawing, vec!["图号", "图例标准", "PBS", "对象数"]),
            (Kind::Symbol, with(&[])),
            (Kind::Annotation, vec![]),
        ] {
            assert_eq!(kind.facts(), facts, "{kind:?}");
            for fact in &facts {
                assert!(kind.has_fact(fact), "{kind:?} {fact}");
            }
        }
        assert!(!Kind::Segment.has_fact("位号"));
        assert!(!Kind::Object.has_fact("长度"));

        let relations = |kind: Kind| -> Vec<&'static str> {
            kind.relations()
                .iter()
                .map(|relation| relation.name)
                .collect()
        };
        let object_relations = [
            "位号",
            "所属管路",
            "所属管线",
            "管道等级",
            "绝热等级",
            "流体介质",
        ];
        for kind in [
            Kind::Object,
            Kind::Equipment,
            Kind::Valve,
            Kind::Fitting,
            Kind::Instrument,
            Kind::PipeLine,
            Kind::PipeRun,
            Kind::SignalLine,
        ] {
            assert_eq!(relations(kind), object_relations, "{kind:?}");
        }
        assert_eq!(
            relations(Kind::Symbol),
            [&object_relations[..], &["符号"]].concat()
        );
        assert_eq!(relations(Kind::Segment), ["所属管路", "所属管线"]);
        assert_eq!(relations(Kind::Nozzle), ["所属设备"]);
        assert_eq!(relations(Kind::Connector), ["所在线", "对端线"]);
        for kind in [Kind::Loop, Kind::Drawing, Kind::Annotation] {
            assert!(relations(kind).is_empty(), "{kind:?}");
        }
        assert_eq!(
            Kind::Object.relation("位号"),
            Some(Next::OneOf(&["长度", "同类型重复数", "跨图重复数"]))
        );
        assert_eq!(
            Kind::Nozzle.relation("所属设备"),
            Some(Next::Object(Kind::Equipment))
        );
        assert_eq!(
            Kind::Segment.relation("所属管线"),
            Some(Next::Object(Kind::PipeLine))
        );
        assert_eq!(
            Kind::Connector.relation("对端线"),
            Some(Next::Object(Kind::PipeLine))
        );
        assert_eq!(Kind::PipeLine.relation("管道等级"), Some(Next::AnyName));
        assert_eq!(Kind::Symbol.relation("符号"), Some(Next::OneOf(&["状态"])));
        assert_eq!(Kind::PipeLine.relation("所属设备"), None);
        assert_eq!(Kind::Object.relation("项目"), None);

        let roots: Vec<&str> = ROOTS.iter().map(|relation| relation.name).collect();
        assert_eq!(roots, ["项目", "图纸", "参数"]);
        assert_eq!(
            root("项目"),
            Some(Next::OneOf(&[
                "管道等级表",
                "公称直径表",
                "绝热等级表",
                "流体介质表",
                "图例标准",
            ]))
        );
        assert_eq!(root("图纸"), Some(Next::OneOf(&["图号", "PBS"])));
        assert_eq!(root("参数"), Some(Next::AnyName));
        assert_eq!(root("位号"), None);
    }
}

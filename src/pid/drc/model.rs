//! 规则检查用到的数据（`SPEC-pid-drc.md` §3）。现在是规则的作用对象（§3.1 的 `target`）
//! 和 §3.4 的名字表：每种对象有哪些派生事实，哪些关系能接 `.` 往下取，以及哪种
//! 对象都能用的根。规则包、检查结果在后面的步骤里加。

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
}

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

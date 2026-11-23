//! P&ID 类型库：类型树、属性分组、数据要素，以及沿树继承的位号/颜色规则。
//!
//! 这是 M3 的第一个 feature，后续的位号生成、属性面板、拓扑校验都从这里
//! 取公共类型。模型是**纯数据 + 纯函数**：不碰 iced、不碰 Scene，构造与
//! `serde_json` 往返在无 GUI 环境下就能完成，只有 [`TypeLib::store`] /
//! [`TypeLib::load`] 需要一张 [`Document`]。
//!
//! 类型树恰好一个根（`parent` 为 `None` 的 `EQUIPMENT`），其余类型沿
//! `parent` 上溯必达根。构造时就校验：唯一根、父类型存在、无环、同一类型
//! 内一个字段不落在两个分组里。规则（位号 / 颜色）为 `None` 表示「本类型
//! 不覆盖」，解析时沿 `parent` 上溯取最近一个非 `None` 的定义，并把**生效
//! 来源类型编码**一起返回 —— 属性面板要显示「这条规则继承自 EQUIPMENT」。
//!
//! 落盘走模型空间 block record 扩展字典里的 XRecord `PID_AUTHOR_TYPE_LIB`，
//! 布局与 [`super::graph_data`] 一致：`kind=` / `key=` 两条前缀后跟数据
//! 条目，超长条目切成 `part=<n>:` 分片。两个写方共用同一 key，必须同一
//! 布局才能互读。整 key 重写，不逐条增删；库为空则删掉记录，不留空记录。

pub use super::tag_rule::{HierarchyMode, TagRule};

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use document::{Document, Handle};

/// 类型库在模型空间 block record 扩展字典里的 XRecord 名。
pub const XRECORD_KEY: &str = "PID_AUTHOR_TYPE_LIB";

/// 编译内置的默认类型库（`assets/pid-types.json`）。
const DEFAULT_TYPE_LIB_JSON: &str = include_str!("../../assets/pid-types.json");

/// The built-in type library as it was compiled in, for the collaboration
/// fingerprint (#97).
pub(crate) fn builtin_source() -> &'static str {
    DEFAULT_TYPE_LIB_JSON
}

/// 一条数据条目正文承载的最大字节数，与 `graph_data::split_text` 的分片
/// 阈值同口径。统一布局后仅供测试断言引用。
#[cfg(test)]
const CHUNK_BYTES: usize = 2000;

/// 承载库内容的 entry 组码（DXF 1 = 任意字符串）。
#[cfg(test)]
const CHUNK_CODE: i32 = 1;

// ── 数据要素 ─────────────────────────────────────────────────────────────

/// 数据要素的取值类型。序列化成契约钉死的五个中文值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueType {
    #[serde(rename = "文本")]
    Text,
    #[serde(rename = "数值")]
    Number,
    #[serde(rename = "枚举")]
    Enum,
    #[serde(rename = "引用")]
    Reference,
    #[serde(rename = "布尔")]
    Boolean,
}

impl ValueType {
    /// 契约里的五个中文取值。
    pub const NAMES: [&'static str; 5] = ["文本", "数值", "枚举", "引用", "布尔"];

    /// 中文值，写回 JSON 时与序列化结果一致。
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "文本",
            Self::Number => "数值",
            Self::Enum => "枚举",
            Self::Reference => "引用",
            Self::Boolean => "布尔",
        }
    }

    /// 按中文值解析；五个值以外返回 `None`。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "文本" => Some(Self::Text),
            "数值" => Some(Self::Number),
            "枚举" => Some(Self::Enum),
            "引用" => Some(Self::Reference),
            "布尔" => Some(Self::Boolean),
            _ => None,
        }
    }
}

/// 一个数据要素：类型上一个可填的属性槽位。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataElement {
    /// 要素编码，例如 `MAT`。
    pub code: String,
    /// 展示名，例如 `材质`。
    pub label: String,
    /// 取值类型。
    pub value_type: ValueType,
    /// 枚举可选项；非枚举为空。
    #[serde(default)]
    pub options: Vec<String>,
}

impl DataElement {
    pub fn new(code: impl Into<String>, label: impl Into<String>, value_type: ValueType) -> Self {
        Self {
            code: code.into(),
            label: label.into(),
            value_type,
            options: Vec::new(),
        }
    }

    /// 枚举要素：带上可选项。
    pub fn enumeration(
        code: impl Into<String>,
        label: impl Into<String>,
        options: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            code: code.into(),
            label: label.into(),
            value_type: ValueType::Enum,
            options: options.into_iter().map(Into::into).collect(),
        }
    }
}

/// 一个属性分组：组名 + 组内字段（保序）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyGroup {
    /// 组名，例如 `基本属性` / `工艺参数`。
    pub name: String,
    /// 组内字段名，顺序即展示顺序。
    #[serde(default)]
    pub fields: Vec<String>,
}

impl PropertyGroup {
    pub fn new(
        name: impl Into<String>,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            fields: fields.into_iter().map(Into::into).collect(),
        }
    }
}

// ── 类型节点 ─────────────────────────────────────────────────────────────

/// 类型树上的一个类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeNode {
    /// 类型编码，例如 `EQUIPMENT` / `VESSEL` / `PUMP` / `TANK`。
    pub type_code: String,
    /// 展示名。
    pub label: String,
    /// 父类型编码；`None` 只对唯一的根成立。
    #[serde(default)]
    pub parent: Option<String>,
    /// 属性分组，顺序即展示顺序。
    #[serde(default)]
    pub groups: Vec<PropertyGroup>,
    /// 数据要素。
    #[serde(default)]
    pub elements: Vec<DataElement>,
    /// 位号规则；`None` = 本类型不覆盖，沿父链继承。
    #[serde(default)]
    pub tag_rule: Option<TagRule>,
    /// 颜色规则；`None` = 本类型不覆盖，沿父链继承。
    #[serde(default)]
    pub color_rule: Option<String>,
}

impl TypeNode {
    /// 只有编码与父类型的类型节点，其余为空。
    pub fn new(type_code: impl Into<String>, parent: Option<impl Into<String>>) -> Self {
        Self {
            type_code: type_code.into(),
            label: String::new(),
            parent: parent.map(Into::into),
            groups: Vec::new(),
            elements: Vec::new(),
            tag_rule: None,
            color_rule: None,
        }
    }

    /// 链式设置展示名。
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 链式追加一个属性分组。
    pub fn with_group(mut self, group: PropertyGroup) -> Self {
        self.groups.push(group);
        self
    }

    /// 链式追加一个数据要素。
    pub fn with_element(mut self, element: DataElement) -> Self {
        self.elements.push(element);
        self
    }

    /// 链式设置位号规则。
    pub fn with_tag_rule(mut self, rule: TagRule) -> Self {
        self.tag_rule = Some(rule);
        self
    }

    /// 链式设置颜色规则。
    pub fn with_color_rule(mut self, color: impl Into<String>) -> Self {
        self.color_rule = Some(color.into());
        self
    }

    /// 本类型所有分组里的字段名，按分组顺序展平。
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.groups
            .iter()
            .flat_map(|group| group.fields.iter().map(String::as_str))
    }
}

// ── 错误 ─────────────────────────────────────────────────────────────────

/// 类型库构造失败的原因。每个变体带一个契约编码（[`TypeLibError::code`]），
/// 断言与日志都用编码而不是展示文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeLibError {
    /// 没有任何 `parent` 为 `None` 的类型。
    NoRoot,
    /// 有多个根。
    MultipleRoots(Vec<String>),
    /// `parent` 指向一个不存在的类型。
    UnknownParent {
        type_code: String,
        parent: String,
    },
    /// 沿 `parent` 上溯成环。
    Cycle {
        type_code: String,
    },
    /// 同一类型里一个字段出现在两个分组（或同一分组两次）。
    FieldDuplicated {
        type_code: String,
        field: String,
    },
    /// 同一个类型编码出现两次。
    DuplicateTypeCode(String),
}

impl TypeLibError {
    /// 契约编码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoRoot => "TYPE_LIB_NO_ROOT",
            Self::MultipleRoots(_) => "TYPE_LIB_MULTIPLE_ROOTS",
            Self::UnknownParent { .. } => "TYPE_LIB_UNKNOWN_PARENT",
            Self::Cycle { .. } => "TYPE_LIB_CYCLE",
            Self::FieldDuplicated { .. } => "TYPE_LIB_FIELD_DUPLICATED",
            Self::DuplicateTypeCode(_) => "TYPE_LIB_TYPE_DUPLICATED",
        }
    }
}

impl fmt::Display for TypeLibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRoot => write!(f, "{}: 没有根类型（parent 为 None）", self.code()),
            Self::MultipleRoots(roots) => {
                write!(f, "{}: 多个根类型 {roots:?}", self.code())
            }
            Self::UnknownParent {
                type_code,
                parent,
            } => write!(f, "{}: {type_code} 的父类型 {parent} 不存在", self.code()),
            Self::Cycle { type_code } => write!(f, "{}: {type_code} 沿 parent 上溯成环", self.code()),
            Self::FieldDuplicated { type_code, field } => {
                write!(f, "{}: {type_code} 的字段 {field} 落在两个分组里", self.code())
            }
            Self::DuplicateTypeCode(code) => {
                write!(f, "{}: 类型编码 {code} 重复", self.code())
            }
        }
    }
}

impl std::error::Error for TypeLibError {}

// ── 类型库 ───────────────────────────────────────────────────────────────

/// 类型库：类型树 + 全局规则表。
///
/// 内部按 `type_code` 的 `BTreeMap` 存，所以遍历与子类型列表天然按编码
/// 字典序、稳定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeLib {
    types: BTreeMap<String, TypeNode>,
    root: String,
}

/// JSON 里的形状：`{ "types": [ ... ] }`，与 `assets/pid-types.json` 同构。
#[derive(Serialize, Deserialize)]
struct TypeLibWire {
    #[serde(default)]
    types: Vec<TypeNode>,
}

impl TypeLib {
    /// 从类型节点构造并校验。任何一条校验失败都返回 `Err`，不做部分写入。
    pub fn new(nodes: impl IntoIterator<Item = TypeNode>) -> Result<Self, TypeLibError> {
        let mut types: BTreeMap<String, TypeNode> = BTreeMap::new();
        for node in nodes {
            if types.contains_key(&node.type_code) {
                return Err(TypeLibError::DuplicateTypeCode(node.type_code));
            }
            types.insert(node.type_code.clone(), node);
        }

        let roots: Vec<String> = types
            .values()
            .filter(|node| node.parent.is_none())
            .map(|node| node.type_code.clone())
            .collect();
        let root = match roots.len() {
            0 => return Err(TypeLibError::NoRoot),
            1 => roots.into_iter().next().expect("len == 1"),
            _ => return Err(TypeLibError::MultipleRoots(roots)),
        };

        for node in types.values() {
            if let Some(parent) = &node.parent {
                if !types.contains_key(parent) {
                    return Err(TypeLibError::UnknownParent {
                        type_code: node.type_code.clone(),
                        parent: parent.clone(),
                    });
                }
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for field in node.field_names() {
                if !seen.insert(field) {
                    return Err(TypeLibError::FieldDuplicated {
                        type_code: node.type_code.clone(),
                        field: field.to_string(),
                    });
                }
            }
        }

        // 环：沿 parent 走，步数超过类型数即必有环。
        for node in types.values() {
            let mut cursor = Some(node);
            let mut steps = 0;
            while let Some(current) = cursor {
                steps += 1;
                if steps > types.len() {
                    return Err(TypeLibError::Cycle {
                        type_code: node.type_code.clone(),
                    });
                }
                cursor = current
                    .parent
                    .as_deref()
                    .and_then(|parent| types.get(parent));
            }
        }

        Ok(Self { types, root })
    }

    /// 内置默认库（`assets/pid-types.json`）。数据由本 crate 的测试保证可解析。
    pub fn embedded() -> Self {
        Self::from_json(DEFAULT_TYPE_LIB_JSON)
            .expect("内置 assets/pid-types.json 必须能构造出类型库")
    }

    /// 从 JSON 文本构造。
    pub fn from_json(json: &str) -> Result<Self, TypeLibError> {
        let wire: TypeLibWire =
            serde_json::from_str(json).map_err(|_| TypeLibError::NoRoot)?;
        Self::new(wire.types)
    }

    /// 序列化成 JSON 文本。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("类型库序列化不应失败")
    }

    /// 序列化成缩进 JSON 文本（导出给人看）。
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("类型库序列化不应失败")
    }

    /// 根类型编码。
    pub fn root_code(&self) -> &str {
        &self.root
    }

    /// 根类型。
    pub fn root(&self) -> &TypeNode {
        self.types
            .get(&self.root)
            .expect("构造时已保证根存在")
    }

    /// 按编码取类型。
    pub fn get(&self, type_code: &str) -> Option<&TypeNode> {
        self.types.get(type_code)
    }

    /// 是否含该类型。
    pub fn contains(&self, type_code: &str) -> bool {
        self.types.contains_key(type_code)
    }

    /// 全部类型，按编码字典序。
    pub fn types(&self) -> impl Iterator<Item = &TypeNode> {
        self.types.values()
    }

    /// 类型个数。
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// 空库。正常构造出的库至少一个根，这里只用于「有没有东西要落盘」的判断。
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// 祖先链：**自底向上、含自身**，到根为止。类型不存在返回空。
    pub fn ancestors(&self, type_code: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut cursor = self.types.get(type_code);
        while let Some(node) = cursor {
            chain.push(node.type_code.clone());
            if chain.len() > self.types.len() {
                // 构造时已排除环，这里是防御：绝不死循环。
                break;
            }
            cursor = node
                .parent
                .as_deref()
                .and_then(|parent| self.types.get(parent));
        }
        chain
    }

    /// 直接子类型编码，按编码字典序稳定排序。
    ///
    /// 排序键是 `type_code`：`EQUIPMENT` 的子类型 `PUMP`、`VESSEL` 依次返回。
    pub fn children(&self, type_code: &str) -> Vec<String> {
        self.types
            .values()
            .filter(|node| node.parent.as_deref() == Some(type_code))
            .map(|node| node.type_code.clone())
            .collect()
    }

    /// 本类型的属性分组（不继承），顺序即定义顺序。
    pub fn property_groups(&self, type_code: &str) -> &[PropertyGroup] {
        self.types
            .get(type_code)
            .map(|node| node.groups.as_slice())
            .unwrap_or(&[])
    }

    /// 本类型的数据要素。
    pub fn data_elements(&self, type_code: &str) -> &[DataElement] {
        self.types
            .get(type_code)
            .map(|node| node.elements.as_slice())
            .unwrap_or(&[])
    }

    /// 按要素编码取数据要素。
    pub fn data_element(&self, type_code: &str, code: &str) -> Option<&DataElement> {
        self.data_elements(type_code)
            .iter()
            .find(|element| element.code == code)
    }

    /// 解析生效的位号规则，返回 `(规则, 生效来源类型编码)`。
    ///
    /// 本类型没有规则就沿 `parent` 上溯，取最近一个非 `None` 的定义；全链
    /// 都没有则 `None`。
    pub fn resolve_tag_rule(&self, type_code: &str) -> Option<(TagRule, String)> {
        self.resolve_with(type_code, |node| node.tag_rule.clone())
    }

    /// 解析生效的颜色规则，返回 `(颜色, 生效来源类型编码)`。
    pub fn resolve_color_rule(&self, type_code: &str) -> Option<(String, String)> {
        self.resolve_with(type_code, |node| node.color_rule.clone())
    }

    /// 沿祖先链（自底向上）取第一个有值的定义。
    fn resolve_with<T>(
        &self,
        type_code: &str,
        take: impl Fn(&TypeNode) -> Option<T>,
    ) -> Option<(T, String)> {
        for code in self.ancestors(type_code) {
            let node = self.types.get(&code)?;
            if let Some(value) = take(node) {
                return Some((value, code));
            }
        }
        None
    }

    // ── 落盘 ─────────────────────────────────────────────────────────────

    /// 把库写进 `doc` 的模型空间 block record 扩展字典，key 为
    /// [`XRECORD_KEY`]。整 key 重写；库为空则删掉记录。
    ///
    /// 布局统一为 [`super::graph_data`]：`kind=` / `key=` 前缀 + 一条数据
    /// 条目（超长则 `part=<n>:` 分片）。`graph_data::store` 与 `dwg_roundtrip`
    /// 写的是同一 key，必须同一布局才能互读。
    pub fn store(&self, doc: &mut Document) {
        if self.is_empty() {
            crate::pid::graph_data::store(doc, XRECORD_KEY, &[]);
            return;
        }
        let value = serde_json::to_value(self).expect("TypeLib 必须可序列化");
        crate::pid::graph_data::store(doc, XRECORD_KEY, &[value]);
    }

    /// 从 `doc` 读回库；没有记录、或记录内容不是合法库，返回 `None`。
    pub fn load(doc: &Document) -> Option<Self> {
        let items = crate::pid::graph_data::load(doc, XRECORD_KEY)?;
        let first = items.into_iter().next()?;
        serde_json::from_value::<Self>(first).ok()
    }

    /// 删掉落盘的库记录，保留扩展字典本身。
    pub fn clear(doc: &mut Document) {
        remove_xrecord(doc, owner(doc));
    }
}

impl Default for TypeLib {
    fn default() -> Self {
        Self::embedded()
    }
}

impl Serialize for TypeLib {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TypeLibWire {
            types: self.types.values().cloned().collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TypeLib {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = TypeLibWire::deserialize(deserializer)?;
        Self::new(wire.types).map_err(|err| serde::de::Error::custom(err.code()))
    }
}

// ── 落盘辅助 ─────────────────────────────────────────────────────────────

/// 记录的所有者：模型空间 block record，每张图恰有一个。
fn owner(doc: &Document) -> Handle {
    doc.header.model_space_block_handle
}

/// 从 `owner` 的扩展字典里删掉 [`XRECORD_KEY`] 这条记录。字典本身留着：
/// 别的记录可能住在里面，空字典也无害。
fn remove_xrecord(doc: &mut Document, owner: Handle) {
    let Some(dictionary_handle) = doc.extension_dictionary_handle(owner) else {
        return;
    };
    let Some(document::objects::ObjectType::Dictionary(dictionary)) =
        doc.objects.get_mut(&dictionary_handle)
    else {
        return;
    };
    let Some(index) = dictionary
        .entries
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(XRECORD_KEY))
    else {
        return;
    };
    let (_, record_handle) = dictionary.entries.remove(index);
    doc.objects.remove(&record_handle);
}

#[cfg(test)]
mod tests;

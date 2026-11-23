//! 项目主数据（`SPEC-pid-master-data.md`）：公称直径、管道等级、绝热等级、
//! 流体介质、PBS 节点树和项目锁定的图例标准。
//!
//! 这里是数据对象与 JSON 形状（§3.1）、默认内容、校验（§3.6，错误带出错
//! 位置）、属性值与主数据的比法（§3.5）、各条目在图上有几个对象用着（§5.3 的
//! `usage`），以及两处存放和取用顺序（§3.2–§3.4、§4）：图内记录、图纸目录里的
//! 项目文件，项目文件优先。

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use document::Document;
use serde_json::{json, Map, Value};

use crate::pid::graph_data;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;
use crate::pid::topology::{TYPE_PIPE_RUN, TYPE_PIPE_SEGMENT};

/// 程序认识的格式版本（§3.1 的 `version`）。
pub const VERSION: u64 = 1;
/// 默认内容的公称直径序列（§3.1）。
pub const DEFAULT_DIAMETERS: [u32; 19] = [
    15, 20, 25, 32, 40, 50, 65, 80, 100, 125, 150, 200, 250, 300, 350, 400, 450, 500, 600,
];
/// 公称直径的单位；本期只有毫米。
pub const DIAMETER_UNIT: &str = "mm";
/// 图内记录的键（§3.4）。
pub const PID_AUTHOR_MASTERDATA: &str = "PID_AUTHOR_MASTERDATA";
/// 项目文件名（§3.3），在图纸所在的目录里。
pub const PROJECT_FILE: &str = "pid-project.json";
/// 程序认识的项目文件格式版本（§3.3 的 `version`）。
pub const PROJECT_FILE_VERSION: u64 = 1;
/// 公称直径标签最多几个字符。
const LABEL_MAX_CHARS: usize = 16;
/// 三张表里的名称最多几个字符。
const NAME_MAX_CHARS: usize = 64;

/// 一份项目主数据（§3.1）。
#[derive(Debug, Clone, PartialEq)]
pub struct MasterData {
    pub version: u64,
    /// 每次写入加 1。
    pub revision: u64,
    /// 最近一次写入的时间，Unix 秒。
    pub updated_at: i64,
    /// 项目锁定的图例标准；`None` 是没有锁定。
    pub legend_standard: Option<String>,
    /// 公称直径表，单位是 [`DIAMETER_UNIT`]。
    pub nominal_diameters: Vec<Diameter>,
    pub pipe_classes: Vec<Entry>,
    pub insulation_classes: Vec<Entry>,
    pub fluid_codes: Vec<Entry>,
    pub pbs: Vec<PbsNode>,
}

/// 公称直径表的一行。
#[derive(Debug, Clone, PartialEq)]
pub struct Diameter {
    pub dn: u32,
    /// 显示标签，不超过 16 个字符。
    pub label: String,
    /// 外径，可不填。
    pub outer_diameter: Option<f64>,
    /// 为真表示本项目不许用这个直径。
    pub hidden: bool,
}

/// 管道等级、绝热等级、流体介质表的一行：名称加扩展属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// 属性名 → 文本，按文件里的顺序。
    pub attrs: Vec<(String, String)>,
}

impl Entry {
    /// 扩展属性 `name` 的值。
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// PBS 节点树的一个节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbsNode {
    pub id: String,
    pub label: String,
    /// 上级节点号；`None` 是根。
    pub parent: Option<String>,
}

/// 名称加扩展属性的那三张表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    PipeClass,
    InsulationClass,
    FluidCode,
}

impl Table {
    /// 这张表在 JSON 里的键。
    pub fn key(self) -> &'static str {
        match self {
            Self::PipeClass => "pipe_classes",
            Self::InsulationClass => "insulation_classes",
            Self::FluidCode => "fluid_codes",
        }
    }
}

// ── 错误 ─────────────────────────────────────────────────────────────────

/// 主数据读不出来、写不进去的原因。每个变体有错误码（[`MasterDataError::code`]）
/// 和出错位置（[`MasterDataError::at`]，如 `pipe_classes[2].name`；整个对象是空串；
/// 读写文件出错时是文件路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterDataError {
    /// JSON 结构不对：缺键、类型不对，或者取值超出约束。
    Schema { at: String, reason: String },
    /// 名称或 PBS 节点号为空。
    RequiredFieldMissing { at: String },
    /// 同一张表里名称、`dn` 或 PBS 节点号重复；`at` 是后出现的那一个。
    Duplicate { at: String, value: String },
    /// PBS 的 `parent` 指向不存在的节点，或者沿上级成环。
    PbsInvalid { at: String, reason: String },
    /// 格式版本比程序认识的高；`at` 是那个 `version` 键的位置。
    VersionUnsupported { at: String, found: u64 },
    /// 写项目文件或图内记录时，那里主数据的修订号已经不是调用方读到的 `expected`：
    /// 有人先写过。`found` 为 `None` 是那里没有主数据，或读不出它的修订号。
    Conflict { expected: u64, found: Option<u64> },
    /// 读写项目文件时文件系统报的错。
    Io { path: String, reason: String },
}

impl MasterDataError {
    /// 错误码：§3.6 的那些，加上读写文件出错的 `MASTERDATA_IO_ERROR`。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema { .. } => "MASTERDATA_SCHEMA_ERROR",
            Self::RequiredFieldMissing { .. } => "REQUIRED_FIELD_MISSING",
            Self::Duplicate { .. } => "MASTERDATA_DUPLICATE",
            Self::PbsInvalid { .. } => "MASTERDATA_PBS_INVALID",
            Self::VersionUnsupported { .. } => "MASTERDATA_VERSION_UNSUPPORTED",
            Self::Conflict { .. } => "MASTERDATA_CONFLICT",
            Self::Io { .. } => "MASTERDATA_IO_ERROR",
        }
    }

    /// 出错位置。
    pub fn at(&self) -> &str {
        match self {
            Self::Schema { at, .. }
            | Self::RequiredFieldMissing { at }
            | Self::Duplicate { at, .. }
            | Self::PbsInvalid { at, .. }
            | Self::VersionUnsupported { at, .. } => at,
            Self::Conflict { .. } => "master_data.revision",
            Self::Io { path, .. } => path,
        }
    }
}

impl fmt::Display for MasterDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = if self.at().is_empty() {
            "顶层"
        } else {
            self.at()
        };
        match self {
            Self::Schema { reason, .. }
            | Self::PbsInvalid { reason, .. }
            | Self::Io { reason, .. } => {
                write!(f, "{}: {at} {reason}", self.code())
            }
            Self::RequiredFieldMissing { .. } => write!(f, "{}: {at} 不能为空", self.code()),
            Self::Duplicate { value, .. } => {
                write!(f, "{}: {at} 的 {value} 与前面的重复", self.code())
            }
            Self::VersionUnsupported { found, .. } => {
                write!(f, "{}: {at} 是 {found}，比程序认识的高", self.code())
            }
            Self::Conflict {
                expected,
                found: Some(found),
            } => write!(
                f,
                "{}: 要写的那份主数据已是修订 {found}，不是读到时的 {expected}",
                self.code()
            ),
            Self::Conflict {
                expected,
                found: None,
            } => write!(
                f,
                "{}: 要写的地方没有主数据的修订号，读到时是修订 {expected}",
                self.code()
            ),
        }
    }
}

impl std::error::Error for MasterDataError {}

// ── 默认内容、比法 ────────────────────────────────────────────────────────

/// §3.1 的默认内容：19 个公称直径（标签 `DN<值>`，都不隐藏，外径不填），
/// 图例标准取 `legend_standard`，其余三张表和 PBS 为空。还没写过，`revision`
/// 与 `updated_at` 都是 0。
pub fn defaults(legend_standard: &str) -> MasterData {
    MasterData {
        version: VERSION,
        revision: 0,
        updated_at: 0,
        legend_standard: Some(legend_standard.to_owned()),
        nominal_diameters: DEFAULT_DIAMETERS
            .iter()
            .map(|&dn| Diameter {
                dn,
                label: format!("DN{dn}"),
                outer_diameter: None,
                hidden: false,
            })
            .collect(),
        pipe_classes: Vec::new(),
        insulation_classes: Vec::new(),
        fluid_codes: Vec::new(),
        pbs: Vec::new(),
    }
}

/// §3.5 的公称直径读法：去掉首尾空白、开头的 `DN`（不分大小写）和它后面的空白，
/// 余下的应当全是 ASCII 数字。`DN80`、`dn 80`、`80` 都读成 80；`八十`、`DN8 0`、
/// `+80`、超出 `u32` 的读不出来，是 `None`。
pub fn parse_dn(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits = match value.get(..2) {
        Some(prefix) if prefix.eq_ignore_ascii_case("dn") => value[2..].trim_start(),
        _ => value,
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

impl MasterData {
    /// §3.1 的 JSON 形状，键按规格里的顺序。
    pub fn to_json(&self) -> Value {
        let items: Vec<Value> = self
            .nominal_diameters
            .iter()
            .map(|diameter| {
                json!({
                    "dn": diameter.dn,
                    "label": diameter.label,
                    "outer_diameter": diameter.outer_diameter,
                    "hidden": diameter.hidden,
                })
            })
            .collect();
        let pbs: Vec<Value> = self
            .pbs
            .iter()
            .map(|node| json!({ "id": node.id, "label": node.label, "parent": node.parent }))
            .collect();
        json!({
            "version": self.version,
            "revision": self.revision,
            "updated_at": self.updated_at,
            "legend_standard": self.legend_standard,
            "nominal_diameters": { "unit": DIAMETER_UNIT, "items": items },
            "pipe_classes": entries_json(&self.pipe_classes),
            "insulation_classes": entries_json(&self.insulation_classes),
            "fluid_codes": entries_json(&self.fluid_codes),
            "pbs": pbs,
        })
    }

    /// §3.5：这个公称直径在不在表里。读不出来、表里没有、表里标了隐藏，都算不在。
    pub fn has_diameter(&self, value: &str) -> bool {
        parse_dn(value).is_some_and(|dn| {
            self.nominal_diameters
                .iter()
                .any(|diameter| diameter.dn == dn && !diameter.hidden)
        })
    }

    /// 一张表的全部行。
    pub fn table(&self, table: Table) -> &[Entry] {
        match table {
            Table::PipeClass => &self.pipe_classes,
            Table::InsulationClass => &self.insulation_classes,
            Table::FluidCode => &self.fluid_codes,
        }
    }

    /// §3.5：名称与 `name`（去掉首尾空白后）逐字相等的那一行，区分大小写。
    pub fn entry(&self, table: Table, name: &str) -> Option<&Entry> {
        let name = name.trim();
        self.table(table).iter().find(|entry| entry.name == name)
    }

    /// 一张表的名称，按表里的顺序。
    pub fn names(&self, table: Table) -> Vec<&str> {
        self.table(table)
            .iter()
            .map(|entry| entry.name.as_str())
            .collect()
    }

    /// PBS 节点 `id` 自身加它的所有下级：先节点后下级，同级按表里的顺序。
    /// 没有这个节点时为空。
    pub fn pbs_subtree(&self, id: &str) -> Vec<&str> {
        let Some(root) = self.pbs.iter().find(|node| node.id == id) else {
            return Vec::new();
        };
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for node in &self.pbs {
            if let Some(parent) = &node.parent {
                children
                    .entry(parent.as_str())
                    .or_default()
                    .push(node.id.as_str());
            }
        }
        // 字段是公开的，没校验过的数据可能成环：走过的节点不再走。
        let mut seen = HashSet::new();
        let mut subtree = Vec::new();
        let mut stack = vec![root.id.as_str()];
        while let Some(node) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            subtree.push(node);
            if let Some(below) = children.get(node) {
                stack.extend(below.iter().rev());
            }
        }
        subtree
    }
}

/// 名称加扩展属性那三张表之一的 JSON 形状。
fn entries_json(rows: &[Entry]) -> Value {
    rows.iter()
        .map(|row| {
            let attrs: Map<String, Value> = row
                .attrs
                .iter()
                .map(|(name, value)| (name.clone(), Value::String(value.clone())))
                .collect();
            json!({ "name": row.name, "attrs": attrs })
        })
        .collect()
}

// ── 使用情况 ─────────────────────────────────────────────────────────────

/// 对象属性里对照三张表的字段（§3.5）。
const TABLE_FIELDS: [(&str, Table); 3] = [
    ("管道等级", Table::PipeClass),
    ("绝热等级", Table::InsulationClass),
    ("流体介质", Table::FluidCode),
];

/// 对象属性里的公称直径，按 [`MasterData::has_diameter`] 的比法对照直径表。
const DIAMETER_FIELD: &str = "公称直径";

/// §5.3 的 `usage`：各条目在图上有几个对象用着。只列用到的条目，按表里的顺序。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub pipe_classes: Vec<(String, usize)>,
    pub insulation_classes: Vec<(String, usize)>,
    pub fluid_codes: Vec<(String, usize)>,
    /// 按 `dn`。
    pub nominal_diameters: Vec<(u32, usize)>,
}

impl Usage {
    /// §5.3 的 JSON 形状：四个对象，键是条目名称（直径是 `dn` 的十进制写法），值是
    /// 对象个数。
    pub fn to_json(&self) -> Value {
        fn counts<K: ToString>(rows: &[(K, usize)]) -> Value {
            rows.iter()
                .map(|(key, count)| (key.to_string(), json!(count)))
                .collect::<Map<String, Value>>()
                .into()
        }
        json!({
            "pipe_classes": counts(&self.pipe_classes),
            "insulation_classes": counts(&self.insulation_classes),
            "fluid_codes": counts(&self.fluid_codes),
            "nominal_diameters": counts(&self.nominal_diameters),
        })
    }
}

/// 图上用着 `data` 里各条目的对象个数（§5.3 的 `usage`）。
///
/// 数的是 P&ID 对象，不是图元：有位号的按「类型 + 位号」算一个（一条管线画成几段
/// 就是几个图元，位号相同），没有位号的按图元算。管路、管段的那几个字段是从上级抄
/// 下来的（`propagate`），没打覆盖标记的那份就是上级的取值，不再算一次；打了标记的
/// 算它自己的。取值按 §3.5 比：名称去掉首尾空白后逐字相等；直径读出 `dn` 再比，
/// 标了隐藏的不算。
pub fn usage(doc: &Document, data: &MasterData) -> Usage {
    let mut users: Vec<Vec<HashSet<ObjectKey>>> = TABLE_FIELDS
        .iter()
        .map(|(_, table)| vec![HashSet::new(); data.table(*table).len()])
        .collect();
    let mut diameter_users = vec![HashSet::new(); data.nominal_diameters.len()];
    for entity in doc.entities() {
        let handle = entity.common().handle;
        let Some(author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        let (props, overrides) = split_props(&author.props);
        let inherits = author.type_code == TYPE_PIPE_RUN || author.type_code == TYPE_PIPE_SEGMENT;
        let own = |field: &str| {
            props
                .get(field)
                .filter(|_| !inherits || overrides.contains(field))
                .map(|value| value.trim())
        };
        let object = if author.tag.is_empty() {
            ObjectKey::Entity(handle.value())
        } else {
            ObjectKey::Tagged(author.type_code.clone(), author.tag.clone())
        };
        for ((field, table), rows) in TABLE_FIELDS.iter().zip(&mut users) {
            let row = own(field).and_then(|value| {
                data.table(*table)
                    .iter()
                    .position(|entry| entry.name == value)
            });
            if let Some(row) = row {
                rows[row].insert(object.clone());
            }
        }
        let row = own(DIAMETER_FIELD).and_then(parse_dn).and_then(|dn| {
            data.nominal_diameters
                .iter()
                .position(|diameter| diameter.dn == dn && !diameter.hidden)
        });
        if let Some(row) = row {
            diameter_users[row].insert(object);
        }
    }
    let used = |table: Table, rows: &[HashSet<ObjectKey>]| -> Vec<(String, usize)> {
        data.table(table)
            .iter()
            .zip(rows)
            .filter(|(_, objects)| !objects.is_empty())
            .map(|(entry, objects)| (entry.name.clone(), objects.len()))
            .collect()
    };
    Usage {
        pipe_classes: used(Table::PipeClass, &users[0]),
        insulation_classes: used(Table::InsulationClass, &users[1]),
        fluid_codes: used(Table::FluidCode, &users[2]),
        nominal_diameters: data
            .nominal_diameters
            .iter()
            .zip(&diameter_users)
            .filter(|(_, objects)| !objects.is_empty())
            .map(|(diameter, objects)| (diameter.dn, objects.len()))
            .collect(),
    }
}

/// [`usage`] 里的一个对象：有位号的是「类型 + 位号」，没有位号的是图元句柄。
#[derive(Clone, PartialEq, Eq, Hash)]
enum ObjectKey {
    Tagged(String, String),
    Entity(u64),
}

// ── 校验 ─────────────────────────────────────────────────────────────────

/// 校验一个 JSON 值，得出主数据；不对时返回第一处错，带 §3.6 的错误码与出错位置。
///
/// 先看 `version`：比程序认识的高就不再往下读。§3.1 的键都要有，只有
/// `legend_standard`、`outer_diameter`、`hidden`、`attrs`、`parent` 可以不写（当作 `null`、
/// `null`、`false`、空对象、`null`）；认不得的键不理。
pub fn validate(value: &Value) -> Result<MasterData, MasterDataError> {
    read_master_data(value, String::new())
}

/// 同 [`validate`]，出错位置从 `at` 算起（主数据在项目文件里时是 `master_data`）。
fn read_master_data(value: &Value, at: String) -> Result<MasterData, MasterDataError> {
    let root = JsonObject::read(value, at)?;
    Ok(MasterData {
        version: root.version(VERSION)?,
        revision: root.unsigned("revision")?,
        updated_at: root.integer("updated_at")?,
        legend_standard: root.optional_text("legend_standard")?.map(str::to_owned),
        nominal_diameters: diameters(&root)?,
        pipe_classes: entries(&root, Table::PipeClass)?,
        insulation_classes: entries(&root, Table::InsulationClass)?,
        fluid_codes: entries(&root, Table::FluidCode)?,
        pbs: pbs_nodes(&root)?,
    })
}

fn diameters(root: &JsonObject<'_>) -> Result<Vec<Diameter>, MasterDataError> {
    let table = root.object("nominal_diameters")?;
    if table.text("unit")? != DIAMETER_UNIT {
        return Err(schema(
            table.at("unit"),
            format!("单位应为 {DIAMETER_UNIT}"),
        ));
    }
    let mut seen = HashSet::new();
    let mut diameters = Vec::new();
    for (at, item) in table.elements("items")? {
        let item = JsonObject::read(item, at)?;
        let dn = u32::try_from(item.unsigned("dn")?)
            .ok()
            .filter(|&dn| dn > 0)
            .ok_or_else(|| schema(item.at("dn"), "应为正整数"))?;
        if !seen.insert(dn) {
            return Err(MasterDataError::Duplicate {
                at: item.at("dn"),
                value: dn.to_string(),
            });
        }
        let label = item.text("label")?;
        if label.chars().count() > LABEL_MAX_CHARS {
            return Err(schema(
                item.at("label"),
                format!("超过 {LABEL_MAX_CHARS} 个字符"),
            ));
        }
        diameters.push(Diameter {
            dn,
            label: label.to_owned(),
            outer_diameter: item.optional_number("outer_diameter")?,
            hidden: item.flag("hidden")?,
        });
    }
    Ok(diameters)
}

fn entries(root: &JsonObject<'_>, table: Table) -> Result<Vec<Entry>, MasterDataError> {
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for (at, row) in root.elements(table.key())? {
        let row = JsonObject::read(row, at)?;
        let name = row.required_text("name")?;
        if name.chars().count() > NAME_MAX_CHARS {
            return Err(schema(
                row.at("name"),
                format!("超过 {NAME_MAX_CHARS} 个字符"),
            ));
        }
        if !seen.insert(name) {
            return Err(MasterDataError::Duplicate {
                at: row.at("name"),
                value: name.to_owned(),
            });
        }
        let attrs = match row.map.get("attrs") {
            None => Vec::new(),
            Some(attrs) => {
                let attrs = JsonObject::read(attrs, row.at("attrs"))?;
                attrs
                    .map
                    .iter()
                    .map(|(key, value)| match value {
                        Value::String(text) => Ok((key.clone(), text.clone())),
                        _ => Err(schema(attrs.at(key), "应为文本")),
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        rows.push(Entry {
            name: name.to_owned(),
            attrs,
        });
    }
    Ok(rows)
}

fn pbs_nodes(root: &JsonObject<'_>) -> Result<Vec<PbsNode>, MasterDataError> {
    let mut index_of = HashMap::new();
    let mut nodes = Vec::new();
    for (at, node) in root.elements("pbs")? {
        let node = JsonObject::read(node, at)?;
        let id = node.required_text("id")?;
        if index_of.insert(id, nodes.len()).is_some() {
            return Err(MasterDataError::Duplicate {
                at: node.at("id"),
                value: id.to_owned(),
            });
        }
        nodes.push(PbsNode {
            id: id.to_owned(),
            label: node.text("label")?.to_owned(),
            parent: node.optional_text("parent")?.map(str::to_owned),
        });
    }
    let parent_at = |index: usize| format!("{}[{index}].parent", root.at("pbs"));
    let mut up = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let Some(parent) = &node.parent else {
            up.push(None);
            continue;
        };
        let Some(&above) = index_of.get(parent.as_str()) else {
            return Err(MasterDataError::PbsInvalid {
                at: parent_at(index),
                reason: format!("上级节点 {parent} 不存在"),
            });
        };
        up.push(Some(above));
    }
    if let Some(cycle) = first_cycle(&up) {
        let mut path: Vec<&str> = cycle
            .iter()
            .map(|&index| nodes[index].id.as_str())
            .collect();
        path.push(path[0]);
        return Err(MasterDataError::PbsInvalid {
            at: parent_at(cycle[0]),
            reason: format!("沿上级成环：{}", path.join(" → ")),
        });
    }
    Ok(nodes)
}

/// 沿上级（`up[i]` 是节点 `i` 的上级）走出来的第一个环，从环上下标最小的节点起。
fn first_cycle(up: &[Option<usize>]) -> Option<Vec<usize>> {
    #[derive(Clone, Copy)]
    enum Mark {
        New,
        OnPath,
        Done,
    }
    let mut marks = vec![Mark::New; up.len()];
    for start in 0..up.len() {
        let mut path = Vec::new();
        let mut next = Some(start);
        while let Some(index) = next {
            match marks[index] {
                Mark::New => {
                    marks[index] = Mark::OnPath;
                    path.push(index);
                    next = up[index];
                }
                Mark::OnPath => {
                    let from = path.iter().position(|&on_path| on_path == index)?;
                    let mut cycle = path.split_off(from);
                    let lowest = (0..cycle.len()).min_by_key(|&at| cycle[at])?;
                    cycle.rotate_left(lowest);
                    return Some(cycle);
                }
                Mark::Done => break,
            }
        }
        for index in path {
            marks[index] = Mark::Done;
        }
    }
    None
}

/// 校验时的一个 JSON 对象和它的位置。
struct JsonObject<'a> {
    map: &'a Map<String, Value>,
    at: String,
}

impl<'a> JsonObject<'a> {
    fn read(value: &'a Value, at: String) -> Result<Self, MasterDataError> {
        match value {
            Value::Object(map) => Ok(Self { map, at }),
            _ => Err(schema(at, "应为对象")),
        }
    }

    /// 键 `key` 的位置。
    fn at(&self, key: &str) -> String {
        if self.at.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.at)
        }
    }

    fn get(&self, key: &str) -> Result<&'a Value, MasterDataError> {
        self.map
            .get(key)
            .ok_or_else(|| schema(self.at(key), "缺少这个键"))
    }

    fn object(&self, key: &str) -> Result<JsonObject<'a>, MasterDataError> {
        JsonObject::read(self.get(key)?, self.at(key))
    }

    /// 键 `key` 下的数组，每个元素配上自己的位置 `key[i]`。
    fn elements(
        &self,
        key: &str,
    ) -> Result<impl Iterator<Item = (String, &'a Value)>, MasterDataError> {
        let at = self.at(key);
        match self.get(key)? {
            Value::Array(items) => Ok(items
                .iter()
                .enumerate()
                .map(move |(index, item)| (format!("{at}[{index}]"), item))),
            _ => Err(schema(at, "应为数组")),
        }
    }

    fn text(&self, key: &str) -> Result<&'a str, MasterDataError> {
        self.get(key)?
            .as_str()
            .ok_or_else(|| schema(self.at(key), "应为文本"))
    }

    /// 不能为空的文本：去掉首尾空白后为空是 `REQUIRED_FIELD_MISSING`。
    fn required_text(&self, key: &str) -> Result<&'a str, MasterDataError> {
        let text = self.text(key)?;
        if text.trim().is_empty() {
            return Err(MasterDataError::RequiredFieldMissing { at: self.at(key) });
        }
        Ok(text)
    }

    /// 可以不写或写 `null` 的文本。
    fn optional_text(&self, key: &str) -> Result<Option<&'a str>, MasterDataError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(schema(self.at(key), "应为文本或 null")),
        }
    }

    /// 可以不写或写 `null` 的数。
    fn optional_number(&self, key: &str) -> Result<Option<f64>, MasterDataError> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_f64()
                .map(Some)
                .ok_or_else(|| schema(self.at(key), "应为数或 null")),
        }
    }

    /// 可以不写的真假值，不写是假。
    fn flag(&self, key: &str) -> Result<bool, MasterDataError> {
        match self.map.get(key) {
            None => Ok(false),
            Some(value) => value
                .as_bool()
                .ok_or_else(|| schema(self.at(key), "应为 true 或 false")),
        }
    }

    fn unsigned(&self, key: &str) -> Result<u64, MasterDataError> {
        self.get(key)?
            .as_u64()
            .ok_or_else(|| schema(self.at(key), "应为非负整数"))
    }

    fn integer(&self, key: &str) -> Result<i64, MasterDataError> {
        self.get(key)?
            .as_i64()
            .ok_or_else(|| schema(self.at(key), "应为整数"))
    }

    /// 格式版本 `version`：比 `known` 高是 `VersionUnsupported`，0 是结构错。
    fn version(&self, known: u64) -> Result<u64, MasterDataError> {
        let version = self.unsigned("version")?;
        if version > known {
            return Err(MasterDataError::VersionUnsupported {
                at: self.at("version"),
                found: version,
            });
        }
        if version == 0 {
            return Err(schema(self.at("version"), "格式版本从 1 起"));
        }
        Ok(version)
    }
}

fn schema(at: String, reason: impl Into<String>) -> MasterDataError {
    MasterDataError::Schema {
        at,
        reason: reason.into(),
    }
}

// ── 两处存放、取用顺序 ───────────────────────────────────────────────────

/// 生效的主数据从哪里来（§3.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 项目文件，带它的路径。
    Project(PathBuf),
    /// 图内记录。
    Drawing,
}

/// [`resolve`] 的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    Configured {
        data: MasterData,
        source: Source,
    },
    /// 该用的那一份读不出来；不退回去用另一处的。
    Unreadable {
        source: Source,
        error: MasterDataError,
    },
    NotConfigured,
}

/// 按 §3.2 找出生效的主数据：图纸所在目录 `drawing_dir` 里的项目文件带主数据就用它，
/// 否则用图内记录，都没有是未配置。项目文件读不出来就是读不出来，不看图内那份。
/// 没保存过的图没有目录，传 `None`。
pub fn resolve(doc: &Document, drawing_dir: Option<&Path>) -> Resolved {
    if let Some(path) = drawing_dir.map(|dir| dir.join(PROJECT_FILE)) {
        match read_project_file(&path) {
            Ok(None) => {}
            Ok(Some(data)) => {
                return Resolved::Configured {
                    data,
                    source: Source::Project(path),
                };
            }
            Err(error) => {
                return Resolved::Unreadable {
                    source: Source::Project(path),
                    error,
                };
            }
        }
    }
    match load_from_drawing(doc) {
        Some(Ok(data)) => Resolved::Configured {
            data,
            source: Source::Drawing,
        },
        Some(Err(error)) => Resolved::Unreadable {
            source: Source::Drawing,
            error,
        },
        None => Resolved::NotConfigured,
    }
}

/// 读项目文件 `path` 里的主数据（§3.3）。没有这个文件，或者文件里没有 `master_data`
/// （或为 `null`），是 `Ok(None)`。开头的 UTF-8 BOM 不算错；出错位置从文件顶层算起，
/// 如 `master_data.pipe_classes[2].name`。
pub fn read_project_file(path: &Path) -> Result<Option<MasterData>, MasterDataError> {
    let Some(file) = project_file_on_disk(path)? else {
        return Ok(None);
    };
    match file.get("master_data") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => read_master_data(value, "master_data".to_owned()).map(Some),
    }
}

/// 把 `data` 原样写进图内记录 [`PID_AUTHOR_MASTERDATA`]（§3.4），整条重写。修订号
/// 和时间由调用方定，撤销快照也由调用方先推（§4）。
pub fn store_in_drawing(doc: &mut Document, data: &MasterData) {
    graph_data::store(doc, PID_AUTHOR_MASTERDATA, &[data.to_json()]);
}

/// 图内记录里的主数据；没有这条记录是 `None`。记录里应当恰好是一个对象。
pub fn load_from_drawing(doc: &Document) -> Option<Result<MasterData, MasterDataError>> {
    let loaded = graph_data::load_reporting(doc, PID_AUTHOR_MASTERDATA)?;
    let read = match (loaded.items.as_slice(), loaded.unreadable.as_slice()) {
        ([value], []) => validate(value),
        _ => Err(schema(String::new(), "图内记录应当恰好是一个 JSON 对象")),
    };
    Some(read)
}

/// 把 `data` 写成图内记录（§3.4），规矩同 [`write_project_file`]：先看图内原有那条记录的
/// 修订号，`expected_revision` 给了而不相等是 `MASTERDATA_CONFLICT`，什么也不写；写进去的
/// 修订号是原有的加 1（还没有是 1），`updated_at` 取当前时间。原有那条读不出来也照样
/// 盖掉，里面还认得出修订号就接着它数。撤销快照由调用方先推。返回写进去的那一份。
pub fn write_drawing_record(
    doc: &mut Document,
    data: &MasterData,
    expected_revision: Option<u64>,
) -> Result<MasterData, MasterDataError> {
    write_drawing_record_at(doc, data, expected_revision, unix_seconds())
}

/// 同 [`write_drawing_record`]，`updated_at` 取 `now`。
fn write_drawing_record_at(
    doc: &mut Document,
    data: &MasterData,
    expected_revision: Option<u64>,
    now: i64,
) -> Result<MasterData, MasterDataError> {
    let on_record = graph_data::load(doc, PID_AUTHOR_MASTERDATA)
        .and_then(|items| items.first()?.get("revision")?.as_u64());
    if let Some(expected) = expected_revision {
        if on_record != Some(expected) {
            return Err(MasterDataError::Conflict {
                expected,
                found: on_record,
            });
        }
    }
    let written = MasterData {
        revision: on_record.map_or(1, |revision| revision.saturating_add(1)),
        updated_at: now,
        ..data.clone()
    };
    store_in_drawing(doc, &written);
    Ok(written)
}

/// 把 `data` 写进目录 `dir` 里的项目文件（§4）。先读磁盘上主数据的修订号：
/// `expected_revision` 给了而不相等是 `MASTERDATA_CONFLICT`，什么也不写。写进去的修订号
/// 是磁盘上的加 1（还没有是 1），`updated_at` 取当前时间；先写旁边的临时文件再改名
/// 盖过去。项目文件里别的顶层键原样留着；文件本身读不出来（不是 JSON、版本更高）
/// 时不写，免得冲掉人手写的内容。返回写进去的那一份。
pub fn write_project_file(
    dir: &Path,
    data: &MasterData,
    expected_revision: Option<u64>,
) -> Result<MasterData, MasterDataError> {
    write_project_file_at(dir, data, expected_revision, unix_seconds())
}

/// 同 [`write_project_file`]，`updated_at` 取 `now`。
fn write_project_file_at(
    dir: &Path,
    data: &MasterData,
    expected_revision: Option<u64>,
    now: i64,
) -> Result<MasterData, MasterDataError> {
    let path = dir.join(PROJECT_FILE);
    let file = project_file_on_disk(&path)?;
    let on_disk = file
        .as_ref()
        .and_then(|file| file.get("master_data")?.get("revision")?.as_u64());
    if let Some(expected) = expected_revision {
        if on_disk != Some(expected) {
            return Err(MasterDataError::Conflict {
                expected,
                found: on_disk,
            });
        }
    }
    let written = MasterData {
        revision: on_disk.map_or(1, |revision| revision.saturating_add(1)),
        updated_at: now,
        ..data.clone()
    };
    replace_file(
        &path,
        &project_file_text(file.unwrap_or_default(), &written),
    )
    .map_err(|error| io_error(&path, &error))?;
    Ok(written)
}

/// 磁盘上的项目文件，顶层对象；没有这个文件是 `None`。顶层的 `version` 要认得。
pub(crate) fn project_file_on_disk(
    path: &Path,
) -> Result<Option<Map<String, Value>>, MasterDataError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(path, &error)),
    };
    let text = bytes
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(bytes.as_slice());
    let file = match serde_json::from_slice(text) {
        Ok(Value::Object(file)) => file,
        Ok(_) => return Err(schema(String::new(), "应为对象")),
        Err(error) => {
            return Err(schema(String::new(), format!("不是合法的 JSON：{error}")));
        }
    };
    JsonObject {
        map: &file,
        at: String::new(),
    }
    .version(PROJECT_FILE_VERSION)?;
    Ok(Some(file))
}

/// 项目文件全文（§3.3）：键按 `version`、`project`、`master_data`、`drawings` 的顺序，
/// 认不得的顶层键照原来的顺序接在后面；缩进 2 格，UTF-8 不带 BOM，末尾换行。
fn project_file_text(file: Map<String, Value>, data: &MasterData) -> String {
    let mut version = json!(PROJECT_FILE_VERSION);
    let mut project = None;
    let mut drawings = None;
    let mut others = Vec::new();
    for (key, value) in file {
        match key.as_str() {
            "version" => version = value,
            "project" => project = Some(value),
            "drawings" => drawings = Some(value),
            "master_data" => {}
            _ => others.push((key, value)),
        }
    }
    let mut out = Map::new();
    out.insert("version".to_owned(), version);
    if let Some(project) = project {
        out.insert("project".to_owned(), project);
    }
    out.insert("master_data".to_owned(), data.to_json());
    if let Some(drawings) = drawings {
        out.insert("drawings".to_owned(), drawings);
    }
    out.extend(others);
    format!("{:#}\n", Value::Object(out))
}

/// 先写到 `path` 旁边的临时文件再改名盖过去：失败时原文件不变，临时文件删掉。
fn replace_file(path: &Path, text: &str) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn io_error(path: &Path, error: &io::Error) -> MasterDataError {
    MasterDataError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(target_arch = "wasm32")]
fn unix_seconds() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use document::DxfVersion;
    use serde_json::json;

    use super::*;

    /// §3.1 的例子，多一个直径和几个 PBS 节点：两个根，一棵三层的树。
    fn sample() -> Value {
        json!({
            "version": 1,
            "revision": 3,
            "updated_at": 1794800000,
            "legend_standard": "内置示例标准",
            "nominal_diameters": {
                "unit": "mm",
                "items": [
                    { "dn": 15, "label": "DN15", "outer_diameter": null, "hidden": false },
                    { "dn": 50, "label": "DN50", "outer_diameter": 60.3, "hidden": false },
                    { "dn": 65, "label": "DN65", "outer_diameter": null, "hidden": true },
                    { "dn": 80, "label": "DN80", "outer_diameter": 88.9, "hidden": false }
                ]
            },
            "pipe_classes": [
                {
                    "name": "A1A",
                    "attrs": {
                        "压力等级": "PN16",
                        "最大公称压力": "16",
                        "材质": "碳钢",
                        "公称直径下限": "15",
                        "公称直径上限": "600"
                    }
                },
                { "name": "B1B", "attrs": {} }
            ],
            "insulation_classes": [ { "name": "H", "attrs": { "说明": "保温" } } ],
            "fluid_codes": [ { "name": "工艺水", "attrs": { "代号": "PW" } } ],
            "pbs": [
                { "id": "1001", "label": "常压蒸馏装置", "parent": null },
                { "id": "1001-01", "label": "原料罐区", "parent": "1001" },
                { "id": "1001-02", "label": "常压塔区", "parent": "1001" },
                { "id": "1001-01-A", "label": "原油罐", "parent": "1001-01" },
                { "id": "2001", "label": "公用工程", "parent": null }
            ]
        })
    }

    /// `sample()` 改一处：`pointer` 是 JSON Pointer。
    fn with(pointer: &str, value: Value) -> Value {
        let mut changed = sample();
        *changed
            .pointer_mut(pointer)
            .unwrap_or_else(|| panic!("{pointer} is in the sample")) = value;
        changed
    }

    /// `sample()` 去掉一个键：`object` 是它所在对象的 JSON Pointer。
    fn without(object: &str, key: &str) -> Value {
        let mut changed = sample();
        changed
            .pointer_mut(object)
            .and_then(Value::as_object_mut)
            .and_then(|map| map.remove(key))
            .unwrap_or_else(|| panic!("{object}/{key} is in the sample"));
        changed
    }

    fn refused(value: &Value) -> (&'static str, String) {
        let error = validate(value).expect_err("the value is refused");
        (error.code(), error.at().to_owned())
    }

    fn read(value: &Value) -> MasterData {
        validate(value).unwrap_or_else(|error| panic!("the value is read: {error}"))
    }

    #[test]
    fn master_data_defaults_hold_the_19_diameters_and_nothing_else() {
        let data = defaults("内置示例标准");
        let dns: Vec<u32> = data.nominal_diameters.iter().map(|d| d.dn).collect();
        assert_eq!(
            dns,
            [
                15, 20, 25, 32, 40, 50, 65, 80, 100, 125, 150, 200, 250, 300, 350, 400, 450, 500,
                600
            ]
        );
        for diameter in &data.nominal_diameters {
            assert_eq!(diameter.label, format!("DN{}", diameter.dn));
            assert_eq!((diameter.outer_diameter, diameter.hidden), (None, false));
        }
        assert_eq!(data.legend_standard.as_deref(), Some("内置示例标准"));
        assert_eq!((data.version, data.revision, data.updated_at), (1, 0, 0));
        assert!(data.pipe_classes.is_empty());
        assert!(data.insulation_classes.is_empty());
        assert!(data.fluid_codes.is_empty());
        assert!(data.pbs.is_empty());
        assert_eq!(read(&data.to_json()), data, "the defaults are valid");
    }

    #[test]
    fn master_data_sample_is_read_and_written_back_as_it_was() {
        let data = read(&sample());
        assert_eq!(
            (data.version, data.revision, data.updated_at),
            (1, 3, 1794800000)
        );
        assert_eq!(data.nominal_diameters[1].outer_diameter, Some(60.3));
        assert!(data.nominal_diameters[2].hidden);
        assert_eq!(data.pipe_classes[0].attr("最大公称压力"), Some("16"));
        assert_eq!(data.pipe_classes[0].attr("材质"), Some("碳钢"));
        assert_eq!(data.pipe_classes[0].attr("不存在"), None);
        assert_eq!(data.pbs[3].parent.as_deref(), Some("1001-01"));

        assert_eq!(data.to_json(), sample(), "written back key for key");
        assert_eq!(read(&data.to_json()), data);
        let text = serde_json::to_string_pretty(&data.to_json()).expect("serializes");
        let parsed: Value = serde_json::from_str(&text).expect("parses");
        assert_eq!(read(&parsed), data, "through text and back");
        let attrs: Vec<&str> = data.pipe_classes[0]
            .attrs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            attrs,
            [
                "压力等级",
                "最大公称压力",
                "材质",
                "公称直径下限",
                "公称直径上限"
            ]
        );

        let mut extra = sample();
        extra["备注"] = json!("认不得的键不理");
        assert_eq!(read(&extra), data);
        let mut terse = without("/nominal_diameters/items/0", "outer_diameter");
        for (object, key) in [
            ("", "legend_standard"),
            ("/nominal_diameters/items/0", "hidden"),
            ("/pipe_classes/1", "attrs"),
            ("/pbs/0", "parent"),
        ] {
            terse
                .pointer_mut(object)
                .and_then(Value::as_object_mut)
                .and_then(|map| map.remove(key))
                .unwrap_or_else(|| panic!("{object}/{key} is in the sample"));
        }
        let terse = read(&terse);
        assert_eq!(terse.legend_standard, None);
        assert_eq!(terse.nominal_diameters[0], data.nominal_diameters[0]);
        assert_eq!(terse.pipe_classes[1], data.pipe_classes[1]);
        assert_eq!(terse.pbs[0], data.pbs[0]);
    }

    #[test]
    fn master_data_schema_errors_say_where() {
        let cases = [
            (json!([]), ""),
            (with("/version", json!("1")), "version"),
            (with("/version", json!(0)), "version"),
            (with("/updated_at", json!(1.5)), "updated_at"),
            (with("/legend_standard", json!(7)), "legend_standard"),
            (without("", "pbs"), "pbs"),
            (
                with("/nominal_diameters/unit", json!("in")),
                "nominal_diameters.unit",
            ),
            (
                with("/nominal_diameters/items", json!({})),
                "nominal_diameters.items",
            ),
            (
                with("/nominal_diameters/items/2/dn", json!("65")),
                "nominal_diameters.items[2].dn",
            ),
            (
                with("/nominal_diameters/items/2/dn", json!(0)),
                "nominal_diameters.items[2].dn",
            ),
            (
                with(
                    "/nominal_diameters/items/0/label",
                    json!("DN15（这个标签超过十六个字符了）"),
                ),
                "nominal_diameters.items[0].label",
            ),
            (
                with("/nominal_diameters/items/1/outer_diameter", json!("60.3")),
                "nominal_diameters.items[1].outer_diameter",
            ),
            (
                with("/nominal_diameters/items/1/hidden", json!(0)),
                "nominal_diameters.items[1].hidden",
            ),
            (
                with("/pipe_classes/1/name", json!(7)),
                "pipe_classes[1].name",
            ),
            (without("/pipe_classes/1", "name"), "pipe_classes[1].name"),
            (
                with("/fluid_codes/0/name", json!("类".repeat(65))),
                "fluid_codes[0].name",
            ),
            (
                with("/pipe_classes/0/attrs/最大公称压力", json!(16)),
                "pipe_classes[0].attrs.最大公称压力",
            ),
            (
                with("/insulation_classes/0/attrs", json!([])),
                "insulation_classes[0].attrs",
            ),
            (with("/pbs/1", json!("1001-01")), "pbs[1]"),
            (with("/pbs/1/parent", json!(1001)), "pbs[1].parent"),
            (without("/pbs/2", "label"), "pbs[2].label"),
        ];
        for (value, at) in cases {
            assert_eq!(
                refused(&value),
                ("MASTERDATA_SCHEMA_ERROR", at.to_owned()),
                "{value}"
            );
        }
        let message = validate(&with("/pipe_classes/1/name", json!(7)))
            .expect_err("refused")
            .to_string();
        assert!(
            message.contains("MASTERDATA_SCHEMA_ERROR") && message.contains("pipe_classes[1].name"),
            "{message}"
        );
        assert_eq!(
            read(&with("/fluid_codes/0/name", json!("类".repeat(64)))).fluid_codes[0]
                .name
                .chars()
                .count(),
            64
        );
        assert_eq!(
            read(&with(
                "/nominal_diameters/items/0/label",
                json!("十六个字符的标签是可以的对吧对吧")
            ))
            .nominal_diameters[0]
                .label
                .chars()
                .count(),
            16
        );
    }

    #[test]
    fn master_data_an_empty_name_is_a_required_field_missing() {
        for (value, at) in [
            (
                with("/insulation_classes/0/name", json!("")),
                "insulation_classes[0].name",
            ),
            (
                with("/pipe_classes/1/name", json!("   ")),
                "pipe_classes[1].name",
            ),
            (with("/pbs/2/id", json!("")), "pbs[2].id"),
        ] {
            assert_eq!(
                refused(&value),
                ("REQUIRED_FIELD_MISSING", at.to_owned()),
                "{value}"
            );
        }
    }

    #[test]
    fn master_data_duplicates_are_refused_where_the_second_one_stands() {
        for (value, at) in [
            (
                with("/pipe_classes/1/name", json!("A1A")),
                "pipe_classes[1].name",
            ),
            (
                with("/nominal_diameters/items/3/dn", json!(50)),
                "nominal_diameters.items[3].dn",
            ),
            (with("/pbs/4/id", json!("1001-02")), "pbs[4].id"),
        ] {
            assert_eq!(
                refused(&value),
                ("MASTERDATA_DUPLICATE", at.to_owned()),
                "{value}"
            );
        }
        // 只在一张表里算重复，而且区分大小写。
        assert_eq!(
            read(&with("/fluid_codes/0/name", json!("A1A"))).fluid_codes[0].name,
            "A1A"
        );
        assert_eq!(
            read(&with("/pipe_classes/1/name", json!("a1a"))).pipe_classes[1].name,
            "a1a"
        );
    }

    #[test]
    fn master_data_pbs_parents_must_exist_and_lead_to_a_root() {
        for (value, at) in [
            (with("/pbs/1/parent", json!("9999")), "pbs[1].parent"),
            // 1001 → 1001-01-A → 1001-01 → 1001：报在环上下标最小的节点。
            (with("/pbs/0/parent", json!("1001-01-A")), "pbs[0].parent"),
            (with("/pbs/4/parent", json!("2001")), "pbs[4].parent"),
        ] {
            assert_eq!(
                refused(&value),
                ("MASTERDATA_PBS_INVALID", at.to_owned()),
                "{value}"
            );
        }
        let message = validate(&with("/pbs/0/parent", json!("1001-01-A")))
            .expect_err("refused")
            .to_string();
        assert!(
            message.contains("1001 → 1001-01-A → 1001-01 → 1001"),
            "{message}"
        );
        assert_eq!(
            read(&sample())
                .pbs
                .iter()
                .filter(|node| node.parent.is_none())
                .count(),
            2
        );
    }

    #[test]
    fn master_data_newer_versions_are_not_read() {
        assert_eq!(
            refused(&with("/version", json!(2))),
            ("MASTERDATA_VERSION_UNSUPPORTED", "version".to_owned())
        );
        let mut newer = with("/version", json!(2));
        newer["pipe_classes"] = json!("新版本可以换写法");
        assert_eq!(
            refused(&newer),
            ("MASTERDATA_VERSION_UNSUPPORTED", "version".to_owned())
        );
    }

    #[test]
    fn master_data_diameters_compare_by_number_after_dn() {
        let data = read(&sample());
        for value in ["DN80", "dn 80", "80", " Dn80 ", "DN  80", "080", "DN50"] {
            assert!(data.has_diameter(value), "{value:?} is in the table");
        }
        for value in [
            "八十", "DN65", "65", "DN100", "DN", "", "80mm", "DN-80", "+80", "DN8 0", "８０",
        ] {
            assert!(!data.has_diameter(value), "{value:?} is not in the table");
        }
        assert_eq!(parse_dn("dn 80"), Some(80));
        assert_eq!(parse_dn("DN65"), Some(65));
        assert_eq!(parse_dn("八十"), None);
        assert_eq!(parse_dn("99999999999"), None);
    }

    #[test]
    fn master_data_names_compare_exactly_and_case_sensitively() {
        let data = read(&sample());
        let class = data
            .entry(Table::PipeClass, "A1A")
            .expect("A1A is a pipe class");
        assert_eq!(class.attr("材质"), Some("碳钢"));
        assert!(
            data.entry(Table::PipeClass, " A1A ").is_some(),
            "the value is trimmed"
        );
        assert!(data.entry(Table::PipeClass, "a1a").is_none());
        assert!(data.entry(Table::PipeClass, "A1").is_none());
        assert!(data.entry(Table::InsulationClass, "A1A").is_none());
        assert!(data.entry(Table::FluidCode, "工艺水").is_some());
        assert_eq!(data.names(Table::PipeClass), ["A1A", "B1B"]);
        assert_eq!(data.names(Table::InsulationClass), ["H"]);
        assert_eq!(data.names(Table::FluidCode), ["工艺水"]);
        assert_eq!(data.table(Table::PipeClass).len(), 2);
    }

    #[test]
    fn master_data_pbs_subtree_is_the_node_and_everything_below() {
        let data = read(&sample());
        assert_eq!(
            data.pbs_subtree("1001"),
            ["1001", "1001-01", "1001-01-A", "1001-02"]
        );
        assert_eq!(data.pbs_subtree("1001-01"), ["1001-01", "1001-01-A"]);
        assert_eq!(data.pbs_subtree("2001"), ["2001"]);
        assert!(data.pbs_subtree("9999").is_empty());
    }

    /// 测试用的临时目录，测完删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：计数器把同一纳秒里建的两个目录分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-master-data-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            fs::create_dir(&path).expect("scratch directory");
            Self(path)
        }

        /// 这个目录里的项目文件。
        fn file(&self) -> PathBuf {
            self.0.join(PROJECT_FILE)
        }

        /// 目录里的文件名，排好序。
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("scratch directory is readable")
                .map(|entry| {
                    entry
                        .expect("directory entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// §3.3 的项目文件例子，主数据换成 `master_data`。
    fn project_text(master_data: &Value) -> String {
        let file = json!({
            "version": 1,
            "project": { "id": "CDU-2026", "name": "常压蒸馏改造" },
            "master_data": master_data,
            "drawings": [ { "file": "PID-1101.dwg", "drawing_id": "PID-1101", "pbs": "1001-01" } ]
        });
        format!("{file:#}\n")
    }

    /// 与 `sample()` 不同的另一份。
    fn other_data() -> MasterData {
        MasterData {
            revision: 9,
            ..defaults("另一套图例")
        }
    }

    #[test]
    fn master_data_resolve_uses_the_project_file_when_it_has_master_data() {
        let scratch = Scratch::new();
        fs::write(scratch.file(), project_text(&sample())).expect("project file");
        let expected = Resolved::Configured {
            data: read(&sample()),
            source: Source::Project(scratch.file()),
        };
        assert_eq!(resolve(&Document::new(), Some(&scratch.0)), expected);

        // 两处都有：用项目文件那份，图内那份还在。
        let mut doc = Document::new();
        store_in_drawing(&mut doc, &other_data());
        assert_eq!(resolve(&doc, Some(&scratch.0)), expected);
        assert_eq!(load_from_drawing(&doc), Some(Ok(other_data())));

        // 开头带 UTF-8 BOM（Windows PowerShell 5.1 存 UTF-8 时会带）也照样读。
        let with_bom = format!("\u{feff}{}", project_text(&sample()));
        fs::write(scratch.file(), with_bom).expect("project file");
        assert_eq!(resolve(&doc, Some(&scratch.0)), expected);
    }

    #[test]
    fn master_data_resolve_falls_back_to_the_drawing_record() {
        let scratch = Scratch::new();
        let mut doc = Document::new();
        assert_eq!(resolve(&doc, Some(&scratch.0)), Resolved::NotConfigured);
        assert_eq!(resolve(&doc, None), Resolved::NotConfigured);

        store_in_drawing(&mut doc, &other_data());
        let expected = Resolved::Configured {
            data: other_data(),
            source: Source::Drawing,
        };
        assert_eq!(resolve(&doc, Some(&scratch.0)), expected, "no project file");
        assert_eq!(resolve(&doc, None), expected, "a drawing never saved");
        // 项目文件里没有 `master_data`（或为 null），等于没有项目文件（§3.3）。
        for text in [
            r#"{ "version": 1, "project": { "id": "CDU-2026", "name": "常压蒸馏改造" } }"#,
            r#"{ "version": 1, "master_data": null }"#,
        ] {
            fs::write(scratch.file(), text).expect("project file");
            assert_eq!(resolve(&doc, Some(&scratch.0)), expected, "{text}");
        }
    }

    #[test]
    fn master_data_a_broken_project_file_is_unreadable_and_the_drawing_record_is_not_used() {
        let scratch = Scratch::new();
        let mut doc = Document::new();
        store_in_drawing(&mut doc, &other_data());
        let without_version = format!("{:#}", json!({ "master_data": sample() }));
        let cases = [
            (
                r#"{ "version": 1, "master_data": "#.to_owned(),
                "MASTERDATA_SCHEMA_ERROR",
                "",
            ),
            ("[]".to_owned(), "MASTERDATA_SCHEMA_ERROR", ""),
            (without_version, "MASTERDATA_SCHEMA_ERROR", "version"),
            (
                r#"{ "version": 2, "master_data": {} }"#.to_owned(),
                "MASTERDATA_VERSION_UNSUPPORTED",
                "version",
            ),
            (
                project_text(&with("/version", json!(2))),
                "MASTERDATA_VERSION_UNSUPPORTED",
                "master_data.version",
            ),
            (
                project_text(&with("/pipe_classes/1/name", json!(7))),
                "MASTERDATA_SCHEMA_ERROR",
                "master_data.pipe_classes[1].name",
            ),
            (
                project_text(&with("/pipe_classes/1/name", json!("A1A"))),
                "MASTERDATA_DUPLICATE",
                "master_data.pipe_classes[1].name",
            ),
            (
                project_text(&with("/pbs/0/parent", json!("1001-01-A"))),
                "MASTERDATA_PBS_INVALID",
                "master_data.pbs[0].parent",
            ),
        ];
        for (text, code, at) in cases {
            fs::write(scratch.file(), &text).expect("project file");
            match resolve(&doc, Some(&scratch.0)) {
                Resolved::Unreadable { source, error } => {
                    assert_eq!(source, Source::Project(scratch.file()), "{text}");
                    assert_eq!((error.code(), error.at()), (code, at), "{text}");
                }
                other => panic!("{text} is unreadable, got {other:?}"),
            }
        }
    }

    #[test]
    fn master_data_a_broken_drawing_record_is_unreadable() {
        let unreadable = |doc: &Document| match resolve(doc, None) {
            Resolved::Unreadable {
                source: Source::Drawing,
                error,
            } => (error.code(), error.at().to_owned()),
            other => panic!("the drawing record is unreadable, got {other:?}"),
        };
        let mut doc = Document::new();
        graph_data::store(&mut doc, PID_AUTHOR_MASTERDATA, &[json!({ "version": 1 })]);
        assert_eq!(
            unreadable(&doc),
            ("MASTERDATA_SCHEMA_ERROR", "revision".to_owned())
        );
        graph_data::store(&mut doc, PID_AUTHOR_MASTERDATA, &[sample(), sample()]);
        assert_eq!(unreadable(&doc), ("MASTERDATA_SCHEMA_ERROR", String::new()));
        graph_data::store(
            &mut doc,
            PID_AUTHOR_MASTERDATA,
            &[with("/version", json!(2))],
        );
        assert_eq!(
            unreadable(&doc),
            ("MASTERDATA_VERSION_UNSUPPORTED", "version".to_owned())
        );
    }

    #[test]
    fn master_data_the_drawing_record_survives_a_dwg_save_and_reopen() {
        let mut doc = Document::new();
        let data = read(&sample());
        store_in_drawing(&mut doc, &data);
        let bytes =
            crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032).expect("save dwg 2018");
        let back = crate::io::load_bytes("master-data.dwg", bytes).expect("load dwg 2018");
        assert_eq!(load_from_drawing(&back), Some(Ok(data)));
        assert!(
            !graph_data::keys().contains(&PID_AUTHOR_MASTERDATA),
            "the graph keys stay as they were"
        );
    }

    /// 写图内记录与写项目文件一个规矩：修订号接着图内原有的数，给了期望的修订号就比，
    /// 对不上什么也不写。
    #[test]
    fn master_data_writing_the_drawing_record_counts_on_from_the_record() {
        let mut doc = Document::new();
        let first = write_drawing_record_at(&mut doc, &read(&sample()), None, 100).unwrap();
        assert_eq!((first.revision, first.updated_at), (1, 100));
        assert_eq!(load_from_drawing(&doc), Some(Ok(first.clone())));

        let second = write_drawing_record_at(&mut doc, &other_data(), Some(1), 200).unwrap();
        assert_eq!((second.revision, second.updated_at), (2, 200));
        assert_eq!(second.legend_standard.as_deref(), Some("另一套图例"));
        assert_eq!(load_from_drawing(&doc), Some(Ok(second.clone())));

        assert_eq!(
            write_drawing_record_at(&mut doc, &first, Some(1), 300),
            Err(MasterDataError::Conflict {
                expected: 1,
                found: Some(2),
            })
        );
        assert_eq!(
            load_from_drawing(&doc),
            Some(Ok(second)),
            "a conflict writes nothing"
        );

        let mut blank = Document::new();
        assert_eq!(
            write_drawing_record_at(&mut blank, &first, Some(1), 300),
            Err(MasterDataError::Conflict {
                expected: 1,
                found: None,
            })
        );
        assert_eq!(load_from_drawing(&blank), None);
    }

    /// 原有的图内记录读不出来也照样盖掉；里面还认得出修订号就接着它数。
    #[test]
    fn master_data_a_broken_drawing_record_is_overwritten() {
        let mut doc = Document::new();
        graph_data::store(
            &mut doc,
            PID_AUTHOR_MASTERDATA,
            &[with("/nominal_diameters", json!("坏了"))],
        );
        assert!(matches!(load_from_drawing(&doc), Some(Err(_))));
        let written = write_drawing_record_at(&mut doc, &other_data(), None, 100).unwrap();
        assert_eq!(written.revision, 4, "the sample is at revision 3");
        assert_eq!(load_from_drawing(&doc), Some(Ok(written)));

        let mut garbled = Document::new();
        graph_data::store(&mut garbled, PID_AUTHOR_MASTERDATA, &[json!("不是对象")]);
        let written = write_drawing_record_at(&mut garbled, &other_data(), None, 100).unwrap();
        assert_eq!(written.revision, 1);
        assert_eq!(load_from_drawing(&garbled), Some(Ok(written)));
    }

    #[test]
    fn master_data_writing_the_project_file_checks_the_revision_on_disk() {
        let scratch = Scratch::new();
        let data = read(&sample());
        let first =
            write_project_file_at(&scratch.0, &data, None, 1_794_900_000).expect("first write");
        assert_eq!(
            (first.revision, first.updated_at),
            (1, 1_794_900_000),
            "a new file starts at revision 1"
        );
        assert_eq!(
            MasterData {
                revision: 3,
                updated_at: 1_794_800_000,
                ..first.clone()
            },
            data,
            "only the revision and the time change"
        );
        assert_eq!(read_project_file(&scratch.file()), Ok(Some(first)));

        let second =
            write_project_file_at(&scratch.0, &data, Some(1), 1_794_900_060).expect("second write");
        assert_eq!((second.revision, second.updated_at), (2, 1_794_900_060));

        let before = fs::read(scratch.file()).expect("project file");
        let conflict = write_project_file_at(&scratch.0, &data, Some(1), 1_794_900_120)
            .expect_err("revision 2 was written since");
        assert_eq!(
            conflict,
            MasterDataError::Conflict {
                expected: 1,
                found: Some(2)
            }
        );
        assert_eq!(conflict.code(), "MASTERDATA_CONFLICT");
        assert_eq!(
            fs::read(scratch.file()).expect("project file"),
            before,
            "the file is byte for byte as it was"
        );
        assert_eq!(scratch.names(), [PROJECT_FILE], "no temporary file is left");

        // 说了读到哪个修订，磁盘上却没有主数据：也是冲突，什么也不写。
        let empty = Scratch::new();
        assert_eq!(
            write_project_file_at(&empty.0, &data, Some(3), 1_794_900_000),
            Err(MasterDataError::Conflict {
                expected: 3,
                found: None
            })
        );
        assert!(empty.names().is_empty());

        // 不带时间的那个取当前时间。
        let earliest = unix_seconds();
        let third = write_project_file(&scratch.0, &data, Some(2)).expect("third write");
        assert_eq!(third.revision, 3);
        assert!((earliest..=unix_seconds()).contains(&third.updated_at));
    }

    #[test]
    fn master_data_writing_the_project_file_keeps_the_other_keys() {
        let scratch = Scratch::new();
        let file = json!({
            "备注": "人手加的说明",
            "drawings": [ { "file": "PID-1101.dwg", "drawing_id": "PID-1101", "pbs": "1001-01" } ],
            "master_data": sample(),
            "version": 1,
            "project": { "id": "CDU-2026", "name": "常压蒸馏改造" },
            "zz_tool": { "seen": true }
        });
        fs::write(scratch.file(), format!("{file:#}")).expect("project file");
        let written = write_project_file_at(&scratch.0, &other_data(), Some(3), 1_794_900_000)
            .expect("write");
        assert_eq!(written.revision, 4);

        let text = fs::read_to_string(scratch.file()).expect("project file");
        let back: Value = serde_json::from_str(&text).expect("still JSON");
        let keys: Vec<&str> = back
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "version",
                "project",
                "master_data",
                "drawings",
                "备注",
                "zz_tool"
            ],
            "the §3.3 order, then the others as they were"
        );
        for key in ["version", "project", "drawings", "备注", "zz_tool"] {
            assert_eq!(back[key], file[key], "{key}");
        }
        assert_eq!(back["master_data"], written.to_json());
    }

    #[test]
    fn master_data_the_same_master_data_writes_the_same_bytes() {
        let data = read(&sample());
        let write = || {
            let scratch = Scratch::new();
            write_project_file_at(&scratch.0, &data, None, 1_794_900_000).expect("write");
            fs::read(scratch.file()).expect("project file")
        };
        let first = write();
        assert_eq!(write(), first);

        let text = String::from_utf8(first).expect("UTF-8");
        assert!(
            text.starts_with(
                "{\n  \"version\": 1,\n  \"master_data\": {\n    \"version\": 1,\n    \"revision\": 1,\n"
            ),
            "two spaces a level, no BOM: {text}"
        );
        assert!(text.ends_with("\n}\n"), "{text}");
    }

    #[test]
    fn master_data_a_failed_write_leaves_no_temporary_file() {
        // 改名这一步失败：项目文件的位置上是个不空的目录。临时文件已经写出来了，要删掉。
        let scratch = Scratch::new();
        fs::create_dir(scratch.file()).expect("a directory in the way");
        fs::write(scratch.file().join("inside.txt"), "x").expect("a file inside it");
        assert!(replace_file(&scratch.file(), "{}\n").is_err());
        assert_eq!(scratch.names(), [PROJECT_FILE]);
        assert_eq!(
            fs::read_to_string(scratch.file().join("inside.txt")).expect("still there"),
            "x"
        );
        assert_eq!(
            write_project_file_at(&scratch.0, &read(&sample()), None, 1_794_900_000)
                .map_err(|error| error.code()),
            Err("MASTERDATA_IO_ERROR")
        );
        assert_eq!(scratch.names(), [PROJECT_FILE]);

        // 目录不在：报错，什么也没建出来。
        let missing = scratch.0.join("没有这个目录");
        let error = write_project_file_at(&missing, &read(&sample()), None, 1_794_900_000)
            .expect_err("nowhere to write");
        assert_eq!(error.code(), "MASTERDATA_IO_ERROR");
        assert_eq!(error.at(), missing.join(PROJECT_FILE).display().to_string());
        assert!(!missing.exists());
    }

    #[cfg(windows)]
    #[test]
    fn master_data_a_project_file_held_open_elsewhere_is_left_as_it_was() {
        use std::os::windows::fs::OpenOptionsExt;

        let scratch = Scratch::new();
        write_project_file_at(&scratch.0, &read(&sample()), None, 1_794_900_000)
            .expect("first write");
        let before = fs::read(scratch.file()).expect("project file");
        // 别的程序开着它，只许别人读（FILE_SHARE_READ），不许删也不许换掉。
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(scratch.file())
            .expect("held open");
        let error = write_project_file_at(&scratch.0, &other_data(), Some(1), 1_794_900_060)
            .expect_err("a held file cannot be replaced");
        drop(held);
        assert_eq!(error.code(), "MASTERDATA_IO_ERROR");
        assert_eq!(fs::read(scratch.file()).expect("project file"), before);
        assert_eq!(scratch.names(), [PROJECT_FILE], "no temporary file is left");
    }

    #[test]
    fn master_data_a_project_file_that_cannot_be_read_is_not_overwritten() {
        let scratch = Scratch::new();
        for (text, code) in [
            (r#"{ "version": 1, "project": "#, "MASTERDATA_SCHEMA_ERROR"),
            (r#"{ "version": 2 }"#, "MASTERDATA_VERSION_UNSUPPORTED"),
        ] {
            fs::write(scratch.file(), text).expect("project file");
            assert_eq!(
                write_project_file_at(&scratch.0, &read(&sample()), None, 1_794_900_000)
                    .map_err(|error| error.code()),
                Err(code),
                "{text}"
            );
            assert_eq!(
                fs::read_to_string(scratch.file()).expect("project file"),
                text
            );
            assert_eq!(scratch.names(), [PROJECT_FILE]);
        }

        // 只是主数据那一段坏了：整份写进去就是修好了。
        let broken = project_text(&with("/pipe_classes/1/name", json!(7)));
        fs::write(scratch.file(), broken).expect("project file");
        let written = write_project_file_at(&scratch.0, &read(&sample()), None, 1_794_900_000)
            .expect("the broken master data is replaced");
        assert_eq!(written.revision, 4);
        assert_eq!(read_project_file(&scratch.file()), Ok(Some(written)));
    }

    /// 契约规模：2,000 行管道等级，每行 5 个扩展属性。
    const PIPE_CLASSES: usize = 2000;
    /// 预算：解析加校验不超过 50 ms（成功标准第 8 条）。
    const BUDGET: Duration = Duration::from_millis(50);
    /// 采样次数：取中位数。
    const RUNS: usize = 3;

    #[test]
    #[ignore]
    fn master_data_2000_pipe_classes_parse_and_validate_within_50_ms() {
        let mut data = defaults("内置示例标准");
        data.pipe_classes = (0..PIPE_CLASSES)
            .map(|index| Entry {
                name: format!("C{index:04}"),
                attrs: [
                    ("压力等级", "PN16"),
                    ("最大公称压力", "16"),
                    ("材质", "碳钢"),
                    ("公称直径下限", "15"),
                    ("公称直径上限", "600"),
                ]
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            })
            .collect();
        let text = serde_json::to_string_pretty(&data.to_json()).expect("serializes");

        let mut samples = Vec::with_capacity(RUNS);
        for run in 1..=RUNS {
            let started = Instant::now();
            let value: Value = serde_json::from_str(&text).expect("parses");
            let parsed = validate(&value);
            let elapsed = started.elapsed();
            let rows = parsed.map(|read| read.pipe_classes.len());
            println!(
                "MD-1 run {run}/{RUNS}: {:.2} ms, {} bytes, rows {rows:?}",
                elapsed.as_secs_f64() * 1e3,
                text.len()
            );
            assert_eq!(rows, Ok(PIPE_CLASSES), "run {run}");
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[RUNS / 2];
        let shown: Vec<String> = samples
            .iter()
            .map(|sample| format!("{:.2}", sample.as_secs_f64() * 1e3))
            .collect();
        println!(
            "MD-1 median {:.2} ms of {shown:?}, budget {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
        assert!(
            median <= BUDGET,
            "median {median:?} of {shown:?} is over {BUDGET:?}"
        );
    }

    /// 图上加一条线段。
    fn add_line(doc: &mut Document) -> document::Handle {
        doc.add_entity(document::EntityType::Line(
            document::entities::Line::from_points(
                document::types::Vector3::new(0.0, 0.0, 0.0),
                document::types::Vector3::new(100.0, 0.0, 0.0),
            ),
        ))
        .expect("the line is added")
    }

    /// 图上放一个带 P&ID 记录的图元：类型、位号、属性。
    fn put(doc: &mut Document, type_code: &str, tag: &str, props: &str) {
        let handle = add_line(doc);
        PidAuthor {
            type_code: type_code.to_owned(),
            tag: tag.to_owned(),
            props: props.to_owned(),
            ..PidAuthor::default()
        }
        .store(doc, handle);
    }

    /// §5.3 的 `usage`：一个 P&ID 对象算一次，管路、管段只算自己改过的字段；表里没有、
    /// 标了隐藏、读不出来的取值不列。
    #[test]
    fn usage_counts_each_object_once_and_skips_values_runs_and_segments_inherit() {
        let data = validate(&sample()).expect("the sample is valid");
        let mut doc = Document::new();
        let line = r#"{"管道等级":"A1A","公称直径":"DN80","流体介质":"工艺水"}"#;
        // 一条管线画成两段：两个图元，同一个位号，算一个对象。
        put(&mut doc, crate::pid::topology::TYPE_PIPE_LINE, "L-1", line);
        put(&mut doc, crate::pid::topology::TYPE_PIPE_LINE, "L-1", line);
        // 管路抄了管线的值，没打覆盖标记：不再算。
        put(&mut doc, TYPE_PIPE_RUN, "L-1-01", line);
        // 两个管段（没有位号）把直径改成 DN50：DN50 各算一个，抄来的管道等级、介质不算。
        let segment = r#"{"管道等级":"A1A","公称直径":"DN50","流体介质":"工艺水","__overrides":["公称直径"]}"#;
        put(&mut doc, TYPE_PIPE_SEGMENT, "", segment);
        put(&mut doc, TYPE_PIPE_SEGMENT, "", segment);
        // 设备各有位号，各算一个；名称带空白、直径写法不同，都按 §3.5 认。
        put(
            &mut doc,
            "EQUIPMENT",
            "V-1",
            r#"{"管道等级":" A1A ","公称直径":"80","绝热等级":"H"}"#,
        );
        put(
            &mut doc,
            "EQUIPMENT",
            "V-2",
            r#"{"管道等级":"B1B","公称直径":"dn 80"}"#,
        );
        // 大小写不同、表里没有、标了隐藏、读不出来的：不列。
        put(
            &mut doc,
            "EQUIPMENT",
            "V-3",
            r#"{"管道等级":"a1a","公称直径":"DN65","流体介质":"海水","绝热等级":"X"}"#,
        );
        put(&mut doc, "EQUIPMENT", "V-4", r#"{"公称直径":"八十"}"#);
        // 没有 P&ID 记录的图元不算。
        add_line(&mut doc);

        let counted = usage(&doc, &data);
        assert_eq!(
            counted.pipe_classes,
            vec![("A1A".to_owned(), 2), ("B1B".to_owned(), 1)]
        );
        assert_eq!(counted.insulation_classes, vec![("H".to_owned(), 1)]);
        assert_eq!(counted.fluid_codes, vec![("工艺水".to_owned(), 1)]);
        assert_eq!(counted.nominal_diameters, vec![(50, 2), (80, 3)]);
        assert_eq!(
            counted.to_json(),
            json!({
                "pipe_classes": { "A1A": 2, "B1B": 1 },
                "insulation_classes": { "H": 1 },
                "fluid_codes": { "工艺水": 1 },
                "nominal_diameters": { "50": 2, "80": 3 }
            })
        );
        assert_eq!(
            usage(&Document::new(), &data).to_json(),
            json!({
                "pipe_classes": {},
                "insulation_classes": {},
                "fluid_codes": {},
                "nominal_diameters": {}
            })
        );
    }
}

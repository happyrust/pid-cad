//! 符号库（`VAL-SEM-030`..`033`）。
//!
//! 由 `assets/pid-symbols.json` 加载：分类、符号定义、几何图元、端口，
//! 以及符号 ↔ 类型的双向多对多映射。加载后把每个符号生成为同名块定义
//! （`PID_SYM_<category>_<symbol>`），端口各生成一个 `EntityType::Point`。
//!
//! 三条设计约束：
//!
//! 1. **端口元数据不进块几何。** `name` / `type` / `direction` 只存在于 JSON
//!    定义里，块内的 `Point` 只承载坐标。这样删掉块内 POINT 之后符号定义
//!    仍然给出完整的端口角色列表（`VAL-SEM-031` 后半段）。
//! 2. **块定义幂等由 `Scene::define_block_raw` 保证。** 同名块二次调用是
//!    no-op（`src/scene/entity.rs:1595-1597`），本模块不另设「已实例化」标志。
//! 3. **图例标准名只在内存里。** 按 D20 不进任何 XRecord：`TypeLib` 的序列化
//!    形状是 `{types:[...]}`，塞顶层字段会被 `TypeLib::store` 整体覆盖。

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use document::entities::{Arc, Circle, Insert, Line, LwPolyline, Point, Text};
use document::types::{Vector2, Vector3};
use document::{Document, EntityType, Handle};

use super::persist::PidAuthor;

/// 符号库文件的覆盖口环境变量。
pub const SYMBOLS_ENV: &str = "OCS_PID_SYMBOLS";

/// 编译内置的默认符号库（`assets/pid-symbols.json`）。
const DEFAULT_SYMBOLS_JSON: &str = include_str!("../../assets/pid-symbols.json");

/// The built-in symbol library as it was compiled in, for the collaboration
/// fingerprint (#97).
pub(crate) fn builtin_source() -> &'static str {
    DEFAULT_SYMBOLS_JSON
}

/// 符号块名的固定前缀（`VAL-SEM-030` 钉死）。
pub const BLOCK_PREFIX: &str = "PID_SYM_";

/// 图元缺省字高。
fn default_text_height() -> f64 {
    2.5
}

/// 符号缺省缩放。
fn default_scale() -> f64 {
    1.0
}

// ── 错误 ─────────────────────────────────────────────────────────────────

/// 符号库加载 / 插入 / 标准切换失败的原因。每个变体带一个契约编码
/// （[`PidError::code`]），断言与日志都用编码而不是展示文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidError {
    /// JSON 语法或形状错误。
    InvalidJson(String),
    /// 顶层 `standard` 为空。
    EmptyStandard,
    /// `category.id` / `symbol.id` 为空或含 `[a-z0-9_]` 之外的字符。
    InvalidId {
        /// 出问题的 id 字面量。
        id: String,
    },
    /// 同一 `(category.id, symbol.id)` 出现两次，块名会撞车。
    DuplicateSymbol {
        /// 撞车的块名。
        block_name: String,
    },
    /// `geometry` 为空数组。
    EmptyGeometry {
        /// 符号块名。
        block_name: String,
    },
    /// 图元参数非法（多段线少于 2 点、半径非正、弧起止角相同）。
    InvalidGeometry {
        /// 符号块名。
        block_name: String,
        /// 具体原因。
        reason: String,
    },
    /// 同一符号内端口角色名重复。
    DuplicatePort {
        /// 符号块名。
        block_name: String,
        /// 重复的端口名。
        port: String,
    },
    /// `symbol_type_map[].types[]` 为空。
    EmptyTypes {
        /// 符号块名。
        block_name: String,
    },
    /// `symbol_type_map[].types[]` 有重复元素。
    DuplicateType {
        /// 符号块名。
        block_name: String,
        /// 重复的类型编码。
        type_code: String,
    },
    /// `symbol_type_map[].symbol` 在分类里找不到。
    UnknownMapping {
        /// 找不到的块名。
        block_name: String,
    },
    /// 插入时符号块名不在库里。
    UnknownSymbol {
        /// 找不到的块名。
        block_name: String,
    },
    /// 插入时 `(符号, 类型)` 不是 `symbol_type_map` 的一条映射。
    UnmappedType {
        /// 符号块名。
        block_name: String,
        /// 类型编码。
        type_code: String,
    },
    /// 文档已有符号实例，标准不可再切换（`VAL-SEM-033`）。
    LegendStandardLocked {
        /// 当前标准名。
        current: String,
        /// 被拒绝的目标标准名。
        target: String,
    },
}

impl PidError {
    /// 契约编码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidJson(_) => "SYMBOL_LIBRARY_INVALID_JSON",
            Self::EmptyStandard => "SYMBOL_LIBRARY_EMPTY_STANDARD",
            Self::InvalidId { .. } => "SYMBOL_LIBRARY_INVALID_ID",
            Self::DuplicateSymbol { .. } => "SYMBOL_LIBRARY_SYMBOL_DUPLICATED",
            Self::EmptyGeometry { .. } => "SYMBOL_LIBRARY_EMPTY_GEOMETRY",
            Self::InvalidGeometry { .. } => "SYMBOL_LIBRARY_INVALID_GEOMETRY",
            Self::DuplicatePort { .. } => "SYMBOL_LIBRARY_PORT_DUPLICATED",
            Self::EmptyTypes { .. } => "SYMBOL_LIBRARY_EMPTY_TYPES",
            Self::DuplicateType { .. } => "SYMBOL_LIBRARY_TYPE_DUPLICATED",
            Self::UnknownMapping { .. } => "SYMBOL_LIBRARY_MAPPING_UNKNOWN_SYMBOL",
            Self::UnknownSymbol { .. } => "SYMBOL_LIBRARY_SYMBOL_UNKNOWN",
            Self::UnmappedType { .. } => "SYMBOL_LIBRARY_TYPE_UNMAPPED",
            Self::LegendStandardLocked { .. } => "LEGEND_STANDARD_LOCKED",
        }
    }
}

impl std::fmt::Display for PidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(reason) => write!(f, "{}: {reason}", self.code()),
            Self::EmptyStandard => write!(f, "{}: 顶层 standard 为空", self.code()),
            Self::InvalidId { id } => write!(f, "{}: id `{id}` 非法", self.code()),
            Self::DuplicateSymbol { block_name } => {
                write!(f, "{}: 符号块名 {block_name} 重复", self.code())
            }
            Self::EmptyGeometry { block_name } => {
                write!(f, "{}: {block_name} 的 geometry 为空", self.code())
            }
            Self::InvalidGeometry { block_name, reason } => {
                write!(f, "{}: {block_name} 的图元非法（{reason}）", self.code())
            }
            Self::DuplicatePort { block_name, port } => {
                write!(f, "{}: {block_name} 的端口 {port} 重复", self.code())
            }
            Self::EmptyTypes { block_name } => {
                write!(f, "{}: {block_name} 的 types 为空", self.code())
            }
            Self::DuplicateType {
                block_name,
                type_code,
            } => write!(
                f,
                "{}: {block_name} 的 types 里 {type_code} 重复",
                self.code()
            ),
            Self::UnknownMapping { block_name } => {
                write!(f, "{}: 映射指向的符号 {block_name} 不存在", self.code())
            }
            Self::UnknownSymbol { block_name } => {
                write!(f, "{}: 符号 {block_name} 不在符号库中", self.code())
            }
            Self::UnmappedType {
                block_name,
                type_code,
            } => write!(
                f,
                "{}: ({block_name}, {type_code}) 不是一条符号↔类型映射",
                self.code()
            ),
            Self::LegendStandardLocked { current, target } => write!(
                f,
                "{}: 当前标准 {current} 已有符号实例，不能切到 {target}",
                self.code()
            ),
        }
    }
}

impl std::error::Error for PidError {}

// ── JSON 形状 ────────────────────────────────────────────────────────────

/// 端口连接类型（`architecture.md` A.6 末段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    /// 等径。
    Equal,
    /// 异径。
    Reducing,
    /// 盲端。
    Blind,
}

/// 端口流向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDir {
    /// 流入。
    In,
    /// 流出。
    Out,
    /// 双向。
    Bidir,
}

/// 一个端口定义。元数据只活在定义里，不进块几何。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortDef {
    /// 端口角色名，如 `in1` / `out1`。
    pub name: String,
    /// 块内坐标 `[x, y]`。
    pub at: [f64; 2],
    /// 连接类型。
    #[serde(rename = "type")]
    pub port_type: PortKind,
    /// 流向。
    pub direction: PortDir,
}

/// 一条几何图元。`kind` 是 serde 内部标签。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Geometry {
    /// 直线。
    Line {
        /// 起点。
        from: [f64; 2],
        /// 终点。
        to: [f64; 2],
    },
    /// 轻量多段线。
    Polyline {
        /// 顶点，至少两个。
        points: Vec<[f64; 2]>,
        /// 是否闭合。
        #[serde(default)]
        closed: bool,
    },
    /// 圆。
    Circle {
        /// 圆心。
        center: [f64; 2],
        /// 半径，必须为正。
        radius: f64,
    },
    /// 圆弧。
    Arc {
        /// 圆心。
        center: [f64; 2],
        /// 半径，必须为正。
        radius: f64,
        /// 起始角，度。
        start_deg: f64,
        /// 终止角，度。
        end_deg: f64,
    },
    /// 单行文字。
    Text {
        /// 文字内容。
        value: String,
        /// 插入点。
        at: [f64; 2],
        /// 字高。
        #[serde(default = "default_text_height")]
        height: f64,
    },
}

fn v2(p: [f64; 2]) -> Vector2 {
    Vector2::new(p[0], p[1])
}

fn v3(p: [f64; 2]) -> Vector3 {
    Vector3::new(p[0], p[1], 0.0)
}

impl Geometry {
    /// 图元 → 实体。坐标是块内坐标，z 恒为 0。
    pub fn to_entity(&self) -> EntityType {
        match self {
            Self::Line { from, to } => {
                EntityType::Line(Line::from_points(v3(*from), v3(*to)))
            }
            Self::Polyline { points, closed } => {
                let mut polyline = LwPolyline::from_points(points.iter().copied().map(v2).collect());
                polyline.is_closed = *closed;
                EntityType::LwPolyline(polyline)
            }
            Self::Circle { center, radius } => {
                EntityType::Circle(Circle::from_center_radius(v3(*center), *radius))
            }
            Self::Arc {
                center,
                radius,
                start_deg,
                end_deg,
            } => EntityType::Arc(Arc::from_center_radius_angles(
                v3(*center),
                *radius,
                start_deg.to_radians(),
                end_deg.to_radians(),
            )),
            Self::Text { value, at, height } => {
                EntityType::Text(Text::with_value(value.clone(), v3(*at)).with_height(*height))
            }
        }
    }

    /// 加载期校验。
    fn validate(&self, block_name: &str) -> Result<(), PidError> {
        match self {
            Self::Polyline { points, .. } if points.len() < 2 => Err(PidError::InvalidGeometry {
                block_name: block_name.to_string(),
                reason: "polyline 少于两个顶点".to_string(),
            }),
            Self::Circle { radius, .. } | Self::Arc { radius, .. } if *radius <= 0.0 => {
                Err(PidError::InvalidGeometry {
                    block_name: block_name.to_string(),
                    reason: "半径非正".to_string(),
                })
            }
            Self::Arc {
                start_deg, end_deg, ..
            } if start_deg == end_deg => Err(PidError::InvalidGeometry {
                block_name: block_name.to_string(),
                reason: "弧起止角相同".to_string(),
            }),
            _ => Ok(()),
        }
    }
}

/// 一个符号定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolDefinition {
    /// 英文小写 id，参与块名。
    pub id: String,
    /// 调色板页签上的显示名。
    pub label: String,
    /// 插入时的缺省缩放。不影响块定义。
    #[serde(default = "default_scale")]
    pub default_scale: f64,
    /// 图元，按数组顺序逐条生成块内实体。
    pub geometry: Vec<Geometry>,
    /// 端口，逐条生成块内 `Point`。
    #[serde(default)]
    pub ports: Vec<PortDef>,
}

impl SymbolDefinition {
    /// 块内实体：先 `geometry` 逐条，再 `ports` 逐条 `Point`。
    pub fn block_entities(&self) -> Vec<EntityType> {
        let mut out = Vec::with_capacity(self.geometry.len() + self.ports.len());
        for geometry in &self.geometry {
            out.push(geometry.to_entity());
        }
        for port in &self.ports {
            out.push(EntityType::Point(Point::at(v3(port.at))));
        }
        out
    }

    /// 端口角色名列表，顺序与 JSON 一致。
    pub fn port_names(&self) -> Vec<String> {
        self.ports.iter().map(|port| port.name.clone()).collect()
    }
}

/// 一个分类。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolCategory {
    /// 英文小写 id，参与块名。
    pub id: String,
    /// 页签显示名。
    pub label: String,
    /// 分类下的符号。
    pub symbols: Vec<SymbolDefinition>,
}

/// 一条符号 ↔ 类型映射。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolTypeMapping {
    /// 符号块名 `PID_SYM_<category>_<symbol>`。
    pub symbol: String,
    /// 类型编码列表。**不校验**是否在 `assets/pid-types.json` 的类型树中（D21）。
    pub types: Vec<String>,
}

/// JSON 顶层形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SymbolLibraryWire {
    standard: String,
    #[serde(default)]
    categories: Vec<SymbolCategory>,
    #[serde(default)]
    symbol_type_map: Vec<SymbolTypeMapping>,
}

/// 符号库：分类 + 符号定义 + 双向映射 + 当前图例标准。
#[derive(Debug, Clone)]
pub struct SymbolLibrary {
    standard: String,
    categories: Vec<SymbolCategory>,
    symbols: HashMap<String, SymbolDefinition>,
    type_to_symbols: HashMap<String, Vec<String>>,
    symbol_to_types: HashMap<String, Vec<String>>,
}

/// 块名约定：`PID_SYM_<category>_<symbol>`。
pub fn block_name(category_id: &str, symbol_id: &str) -> String {
    format!("{BLOCK_PREFIX}{category_id}_{symbol_id}")
}

/// `[a-z0-9_]` 且非空。
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl SymbolLibrary {
    /// 从 JSON 文本构造并校验。任何一条校验失败都返回 `Err`，不做部分写入。
    pub fn load(json: &str) -> Result<Self, PidError> {
        let wire: SymbolLibraryWire =
            serde_json::from_str(json).map_err(|e| PidError::InvalidJson(e.to_string()))?;
        if wire.standard.trim().is_empty() {
            return Err(PidError::EmptyStandard);
        }

        let mut symbols: HashMap<String, SymbolDefinition> = HashMap::new();
        for category in &wire.categories {
            if !valid_id(&category.id) {
                return Err(PidError::InvalidId {
                    id: category.id.clone(),
                });
            }
            for symbol in &category.symbols {
                if !valid_id(&symbol.id) {
                    return Err(PidError::InvalidId {
                        id: symbol.id.clone(),
                    });
                }
                let name = block_name(&category.id, &symbol.id);
                if symbols.contains_key(&name) {
                    return Err(PidError::DuplicateSymbol { block_name: name });
                }
                if symbol.geometry.is_empty() {
                    return Err(PidError::EmptyGeometry { block_name: name });
                }
                for geometry in &symbol.geometry {
                    geometry.validate(&name)?;
                }
                let mut seen: Vec<&str> = Vec::with_capacity(symbol.ports.len());
                for port in &symbol.ports {
                    if port.name.is_empty() {
                        return Err(PidError::InvalidId {
                            id: port.name.clone(),
                        });
                    }
                    if seen.contains(&port.name.as_str()) {
                        return Err(PidError::DuplicatePort {
                            block_name: name,
                            port: port.name.clone(),
                        });
                    }
                    seen.push(&port.name);
                }
                symbols.insert(name, symbol.clone());
            }
        }

        let mut type_to_symbols: HashMap<String, Vec<String>> = HashMap::new();
        let mut symbol_to_types: HashMap<String, Vec<String>> = HashMap::new();
        for mapping in &wire.symbol_type_map {
            if !symbols.contains_key(&mapping.symbol) {
                return Err(PidError::UnknownMapping {
                    block_name: mapping.symbol.clone(),
                });
            }
            if mapping.types.is_empty() {
                return Err(PidError::EmptyTypes {
                    block_name: mapping.symbol.clone(),
                });
            }
            let mut seen: Vec<&str> = Vec::with_capacity(mapping.types.len());
            for type_code in &mapping.types {
                if type_code.trim().is_empty() {
                    return Err(PidError::InvalidId {
                        id: type_code.clone(),
                    });
                }
                if seen.contains(&type_code.as_str()) {
                    return Err(PidError::DuplicateType {
                        block_name: mapping.symbol.clone(),
                        type_code: type_code.clone(),
                    });
                }
                seen.push(type_code);
            }
            let list = symbol_to_types.entry(mapping.symbol.clone()).or_default();
            for type_code in &mapping.types {
                list.push(type_code.clone());
                let reverse = type_to_symbols.entry(type_code.clone()).or_default();
                if !reverse.contains(&mapping.symbol) {
                    reverse.push(mapping.symbol.clone());
                }
            }
        }

        Ok(Self {
            standard: wire.standard,
            categories: wire.categories,
            symbols,
            type_to_symbols,
            symbol_to_types,
        })
    }

    /// 编译内置的默认符号库。签入的数据必须自洽。
    pub fn builtin() -> Self {
        Self::load(DEFAULT_SYMBOLS_JSON)
            .expect("assets/pid-symbols.json is checked in and must validate")
    }

    /// 从 JSON 文件加载。
    pub fn from_path(path: &Path) -> Result<Self, PidError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| PidError::InvalidJson(format!("{}: {e}", path.display())))?;
        Self::load(&text)
    }

    /// 实际使用的符号库：[`SYMBOLS_ENV`] 指向的文件优先，坏了就 `warn` 后回落内置。
    pub fn load_from_env() -> Self {
        match std::env::var_os(SYMBOLS_ENV) {
            Some(path) => match Self::from_path(Path::new(&path)) {
                Ok(library) => library,
                Err(error) => {
                    log::warn!("{SYMBOLS_ENV}: {error}; using the built-in symbol library");
                    Self::builtin()
                }
            },
            None => Self::builtin(),
        }
    }

    /// 当前图例标准名（项目级唯一）。
    pub fn current_standard(&self) -> &str {
        &self.standard
    }

    /// 分类列表。
    pub fn categories(&self) -> &[SymbolCategory] {
        &self.categories
    }

    /// 块名 → 符号定义。
    pub fn symbol(&self, block: &str) -> Option<&SymbolDefinition> {
        self.symbols.get(block)
    }

    /// 库里所有块名，按分类 / 数组顺序。
    pub fn block_names(&self) -> Vec<String> {
        self.categories
            .iter()
            .flat_map(|category| {
                category
                    .symbols
                    .iter()
                    .map(move |symbol| block_name(&category.id, &symbol.id))
            })
            .collect()
    }

    /// 符号 → 类型编码列表（`VAL-SEM-032` 的正向查询）。
    pub fn types_of(&self, block: &str) -> Vec<String> {
        self.symbol_to_types.get(block).cloned().unwrap_or_default()
    }

    /// 类型编码 → 符号块名列表（`VAL-SEM-032` 的反向查询）。
    pub fn symbols_of(&self, type_code: &str) -> Vec<String> {
        self.type_to_symbols
            .get(type_code)
            .cloned()
            .unwrap_or_default()
    }

    /// 符号定义里的端口角色名。**与块几何无关**（`VAL-SEM-031` 后半段）。
    pub fn symbol_ports(&self, block: &str) -> Vec<String> {
        self.symbols
            .get(block)
            .map(SymbolDefinition::port_names)
            .unwrap_or_default()
    }

    /// 对每个符号调 `Scene::define_block_raw`，返回生成的块名。
    ///
    /// 幂等：同名块已存在时 `define_block_raw` 直接 return，块数量不变。
    pub fn instantiate(&self, scene: &mut crate::scene::Scene) -> Vec<String> {
        let mut names = Vec::new();
        for category in &self.categories {
            for symbol in &category.symbols {
                let name = block_name(&category.id, &symbol.id);
                scene.define_block_raw(&name, Vector3::ZERO, symbol.block_entities());
                names.push(name);
            }
        }
        names
    }

    /// 文档里以 [`BLOCK_PREFIX`] 开头的 INSERT 个数。
    ///
    /// 遍历 `doc.entities()` 而不是 `block_records[*].insert_handles`：后者只在
    /// DWG/DXF 读入时填，新建的 INSERT 不会自动进表。
    pub fn symbol_instance_count(&self, doc: &Document) -> usize {
        doc.entities()
            .filter(|entity| match entity {
                EntityType::Insert(insert) => insert.block_name.starts_with(BLOCK_PREFIX),
                _ => false,
            })
            .count()
    }

    /// 切换图例标准（`VAL-SEM-033`）。
    ///
    /// 文档已有符号实例时标准锁定：切到不同的目标一律
    /// `Err(LEGEND_STANDARD_LOCKED)`，当前标准不变。切到当前值本身是 no-op。
    pub fn switch_standard(&mut self, doc: &Document, target: &str) -> Result<(), PidError> {
        if target.trim().is_empty() {
            return Err(PidError::EmptyStandard);
        }
        if self.standard == target {
            return Ok(());
        }
        if self.symbol_instance_count(doc) > 0 {
            return Err(PidError::LegendStandardLocked {
                current: self.standard.clone(),
                target: target.to_string(),
            });
        }
        self.standard = target.to_string();
        Ok(())
    }
}

// ── 端口世界坐标 ─────────────────────────────────────────────────────────

/// 块内点 → 世界坐标。等价于 `io::pid_legend::blocks::place`（那份是
/// `pub(super)`，本模块自己写一份，不改它的可见性）。
pub fn place(
    local: (f64, f64),
    base: (f64, f64),
    scale: (f64, f64),
    rotation: f64,
    at: (f64, f64),
) -> (f64, f64) {
    let x = (local.0 - base.0) * scale.0;
    let y = (local.1 - base.1) * scale.1;
    let (sin, cos) = rotation.sin_cos();
    (x * cos - y * sin + at.0, x * sin + y * cos + at.1)
}

/// 块定义里 `Point` 实体的块内坐标，按 `entity_handles` 顺序。
pub fn block_point_locations(doc: &Document, block: &str) -> Vec<(f64, f64)> {
    doc.entities_in_block(block)
        .filter_map(|entity| match entity {
            EntityType::Point(point) => Some((point.location.x, point.location.y)),
            _ => None,
        })
        .collect()
}

/// 一次插入实际落地的端口世界坐标：从块几何的 `Point` 推导。
///
/// 块内没有 `Point`（被删掉了）时返回空 —— 端口元数据仍然在符号定义里。
pub fn placed_ports(doc: &Document, insert: &Insert) -> Vec<(f64, f64)> {
    let base = doc
        .block_records
        .get(&insert.block_name)
        .map(|record| (record.base_point.x, record.base_point.y))
        .unwrap_or((0.0, 0.0));
    let scale = (insert.x_scale(), insert.y_scale());
    let at = (insert.insert_point.x, insert.insert_point.y);
    block_point_locations(doc, &insert.block_name)
        .into_iter()
        .map(|local| place(local, base, scale, insert.rotation, at))
        .collect()
}

// ── 符号插入 ─────────────────────────────────────────────────────────────

/// 在 `at` 插入一个符号实例，并把 `(块名, 类型)` 写进 `PID_AUTHOR` XDATA。
///
/// 九键编码复用 [`PidAuthor`]（`super::persist`），不自己拼顺序。
pub fn insert_symbol(
    scene: &mut crate::scene::Scene,
    library: &SymbolLibrary,
    block: &str,
    at: Vector3,
    scale: f64,
    rotation: f64,
    type_code: &str,
) -> Result<Handle, PidError> {
    let symbol = library
        .symbol(block)
        .ok_or_else(|| PidError::UnknownSymbol {
            block_name: block.to_string(),
        })?;
    if !library.types_of(block).iter().any(|code| code == type_code) {
        return Err(PidError::UnmappedType {
            block_name: block.to_string(),
            type_code: type_code.to_string(),
        });
    }
    if !scene.document.block_records.contains(block) {
        library.instantiate(scene);
    }

    let insert = Insert::new(block, at)
        .with_uniform_scale(scale)
        .with_rotation(rotation);
    let handle = scene.add_entity(EntityType::Insert(insert));
    if handle.is_null() {
        return Err(PidError::UnknownSymbol {
            block_name: block.to_string(),
        });
    }

    let author = PidAuthor {
        type_code: type_code.to_string(),
        ports: symbol.port_names().join(","),
        tag_source: super::tag_engine::TAG_SOURCE_AUTO.to_string(),
        tag_status: super::tag_engine::TAG_STATUS_OK.to_string(),
        props: "{}".to_string(),
        ..PidAuthor::default()
    };
    author.store(&mut scene.document, handle);
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use document::xdata::XDataValue;

    const VESSEL: &str = "PID_SYM_equipment_vessel";
    const PUMP: &str = "PID_SYM_equipment_pump";
    const GATE: &str = "PID_SYM_valve_gate_valve";
    const GATE_ALT: &str = "PID_SYM_valve_gate_valve_alt";

    fn builtin_library() -> SymbolLibrary {
        SymbolLibrary::builtin()
    }

    fn block_count(scene: &crate::scene::Scene) -> usize {
        scene.document.block_records.len()
    }

    /// `VAL-SEM-030`：加载 JSON 后每个符号生成块定义，重复加载幂等。
    #[test]
    fn pid_symbols_load_generates_block_definitions() {
        let library = builtin_library();
        let mut scene = crate::scene::Scene::new();
        library.instantiate(&mut scene);

        for name in [VESSEL, PUMP, GATE] {
            assert!(
                scene.document.block_records.get(name).is_some(),
                "{name} 没有生成块定义；现有块名 {:?}",
                scene
                    .document
                    .block_records
                    .iter()
                    .map(|record| record.name.clone())
                    .collect::<Vec<_>>()
            );
        }

        for name in library.block_names() {
            let symbol = library.symbol(&name).expect("块名来自定义");
            let expected = symbol.geometry.len() + symbol.ports.len();
            let actual = scene.document.entities_in_block(&name).count();
            assert_eq!(
                actual, expected,
                "{name} 块内实体数应为 geometry({}) + ports({})，实得 {actual}",
                symbol.geometry.len(),
                symbol.ports.len()
            );
            let record = scene.document.block_records.get(&name).expect("块存在");
            assert_eq!(
                record.base_point,
                Vector3::ZERO,
                "{name} 的 block_record.base_point 应为原点"
            );
            let block = scene
                .document
                .get_entity(record.block_entity_handle)
                .and_then(|entity| match entity {
                    EntityType::Block(block) => Some(block.clone()),
                    _ => None,
                })
                .expect("块定义必有 Block 实体");
            assert_eq!(
                block.base_point,
                Vector3::ZERO,
                "{name} 的 Block 实体 base_point 应为原点"
            );
        }

        // vessel 的图元：polyline + text，另有 2 个端口 POINT。
        let vessel_geometry = library.symbol(VESSEL).expect("vessel 存在").geometry.len();
        assert_eq!(vessel_geometry, 2, "vessel 的 geometry 应为 polyline + text");
        assert_eq!(
            block_point_locations(&scene.document, VESSEL).len(),
            2,
            "vessel 块内应有 2 个端口 POINT"
        );

        let before = block_count(&scene);
        library.instantiate(&mut scene);
        assert_eq!(
            block_count(&scene),
            before,
            "重复加载不得新增块；二次加载后块数 {} → {}",
            before,
            block_count(&scene)
        );
    }

    /// `VAL-SEM-031`：端口从符号定义推导，元数据与块几何两条路径独立。
    #[test]
    fn pid_symbol_ports_derive_from_definition() {
        let library = builtin_library();
        let mut scene = crate::scene::Scene::new();
        library.instantiate(&mut scene);

        let points = block_point_locations(&scene.document, VESSEL);
        assert_eq!(
            points.len(),
            2,
            "vessel 块内 EntityType::Point 应恰为 2 个，实得 {points:?}"
        );
        assert_eq!(points[0], (0.0, -12.0), "in1 的块内坐标");
        assert_eq!(points[1], (0.0, 12.0), "out1 的块内坐标");

        let handle = insert_symbol(
            &mut scene,
            &library,
            VESSEL,
            Vector3::new(100.0, 200.0, 0.0),
            1.0,
            0.0,
            "VESSEL",
        )
        .expect("插入 vessel 成功");
        let insert = match scene.document.get_entity(handle).expect("实体存在") {
            EntityType::Insert(insert) => insert.clone(),
            other => panic!("应插入 INSERT，实得 {other:?}"),
        };
        let world = placed_ports(&scene.document, &insert);
        assert_eq!(
            world,
            vec![(100.0, 188.0), (100.0, 212.0)],
            "插入点 (100,200) 无旋转无缩放时端口世界坐标"
        );

        // 删掉块内两个 POINT 后，块几何不再给出端口。
        let doomed: Vec<Handle> = scene
            .document
            .block_records
            .get(VESSEL)
            .expect("块存在")
            .entity_handles
            .iter()
            .copied()
            .filter(|handle| {
                matches!(
                    scene.document.get_entity(*handle),
                    Some(EntityType::Point(_))
                )
            })
            .collect();
        assert_eq!(doomed.len(), 2, "应有 2 个 POINT 可删");
        for handle in doomed {
            scene.document.remove_entity(handle);
        }
        assert!(
            placed_ports(&scene.document, &insert).is_empty(),
            "块内 POINT 删光后端口世界坐标应为空"
        );

        // 但符号定义里的端口角色名仍然完整。
        assert_eq!(
            library.symbol_ports(VESSEL),
            vec!["in1".to_string(), "out1".to_string()],
            "端口元数据来自符号定义，不受块几何影响"
        );

        // 端口元数据的三个字段逐项校验：M3 只断言了名字列表，而接线判定
        // 需要 `type`（等径 / 异径 / 盲端）与 `direction`（流入 / 流出 /
        // 双向）。它们同样只活在定义里，块内 POINT 承载不了。
        let ports = &library.symbol(VESSEL).expect("vessel 存在").ports;
        assert_eq!(ports.len(), 2, "vessel 应定义 2 个端口");
        let summary: Vec<(&str, PortKind, PortDir, [f64; 2])> = ports
            .iter()
            .map(|port| (port.name.as_str(), port.port_type, port.direction, port.at))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("in1", PortKind::Equal, PortDir::In, [0.0, -12.0]),
                ("out1", PortKind::Equal, PortDir::Out, [0.0, 12.0]),
            ],
            "端口元数据应逐项为 (name, type, direction, 块内坐标)"
        );

        // 三个字段在符号库与 JSON 之间是同一套序列化词汇，`type` 落盘为
        // `equal` / `reducing` / `blind`，`direction` 为 `in` / `out` /
        // `bidir` —— 外部编辑 `assets/pid-symbols.json` 时按这套写。
        assert_eq!(
            serde_json::to_value(&ports[0]).expect("端口可序列化"),
            serde_json::json!({
                "name": "in1",
                "at": [0.0, -12.0],
                "type": "equal",
                "direction": "in",
            })
        );
    }

    /// `VAL-SEM-032`：符号与类型多对多，插入写入正确的 `type` 与块名。
    #[test]
    fn pid_symbol_type_many_to_many() {
        let library = builtin_library();

        let types = library.types_of(GATE);
        assert_eq!(
            types.len(),
            2,
            "{} 应映射 2 个类型，实得 {types:?}",
            GATE
        );
        assert!(types.contains(&"VALVE_GATE".to_string()));
        assert!(types.contains(&"VALVE_ISOLATION".to_string()));

        let symbols = library.symbols_of("VALVE_GATE");
        assert_eq!(
            symbols.len(),
            2,
            "VALVE_GATE 应映射 2 个符号，实得 {symbols:?}"
        );
        assert!(symbols.contains(&GATE.to_string()));
        assert!(symbols.contains(&GATE_ALT.to_string()));

        let mut scene = crate::scene::Scene::new();
        library.instantiate(&mut scene);
        let mut pairs = Vec::new();
        for (block, type_code) in [
            (GATE, "VALVE_GATE"),
            (GATE, "VALVE_ISOLATION"),
            (GATE_ALT, "VALVE_GATE"),
        ] {
            let handle = insert_symbol(
                &mut scene,
                &library,
                block,
                Vector3::new(10.0, 20.0, 0.0),
                1.0,
                0.0,
                type_code,
            )
            .unwrap_or_else(|e| panic!("插入 ({block}, {type_code}) 失败：{e}"));
            let insert = match scene.document.get_entity(handle).expect("实体存在") {
                EntityType::Insert(insert) => insert.clone(),
                other => panic!("应插入 INSERT，实得 {other:?}"),
            };
            let author = PidAuthor::load(&scene.document, handle).expect("PID_AUTHOR 记录存在");
            pairs.push((insert.block_name.clone(), author.type_code.clone()));
            assert_eq!(insert.block_name, block, "块名应与映射一致");
            assert_eq!(author.type_code, type_code, "XDATA type 应与映射一致");
            assert_eq!(
                author.ports, "in1,out1",
                "端口角色名应写进 PID_AUTHOR 的 ports 键"
            );
        }
        assert_eq!(
            pairs,
            vec![
                (GATE.to_string(), "VALVE_GATE".to_string()),
                (GATE.to_string(), "VALVE_ISOLATION".to_string()),
                (GATE_ALT.to_string(), "VALVE_GATE".to_string()),
            ],
            "插入后的 (block_name, type) 对"
        );

        // 未映射的组合必须被拒。
        let error = insert_symbol(
            &mut scene,
            &library,
            GATE_ALT,
            Vector3::ZERO,
            1.0,
            0.0,
            "VALVE_ISOLATION",
        )
        .expect_err("gate_valve_alt 不映射 VALVE_ISOLATION");
        assert_eq!(error.code(), "SYMBOL_LIBRARY_TYPE_UNMAPPED");

        // 九键编码复用 persist 的 key=value 形态。
        let handle = insert_symbol(
            &mut scene,
            &library,
            GATE,
            Vector3::ZERO,
            1.0,
            0.0,
            "VALVE_GATE",
        )
        .expect("插入成功");
        let record = scene
            .document
            .get_entity(handle)
            .expect("实体存在")
            .common()
            .extended_data
            .get_record(super::super::persist::PID_AUTHOR_APP)
            .expect("PID_AUTHOR 记录");
        assert!(
            record
                .values
                .iter()
                .any(|value| matches!(value, XDataValue::String(text) if text == "type=VALVE_GATE")),
            "type 键应编码成 `type=VALVE_GATE`，实得 {:?}",
            record.values
        );
    }

    /// `VAL-SEM-033`：有实例后图例标准锁定，空文档可自由切换。
    #[test]
    fn pid_legend_standard_locked_after_first_instance() {
        let base = builtin_library();
        assert_eq!(base.current_standard(), "内置示例标准");

        // 空文档：切换成功。
        let empty = crate::scene::Scene::new();
        let mut free = builtin_library();
        assert_eq!(
            free.symbol_instance_count(&empty.document),
            0,
            "空文档不应有符号实例"
        );
        free.switch_standard(&empty.document, "项目标准B")
            .expect("空文档切换应返回 Ok");
        assert_eq!(free.current_standard(), "项目标准B");

        // 有 1 个实例：切换被拒。
        let mut scene = crate::scene::Scene::new();
        free.instantiate(&mut scene);
        let handle = insert_symbol(
            &mut scene,
            &free,
            VESSEL,
            Vector3::new(100.0, 200.0, 0.0),
            1.0,
            0.0,
            "VESSEL",
        )
        .expect("插入 vessel 成功");
        assert_eq!(
            free.symbol_instance_count(&scene.document),
            1,
            "应恰有 1 个符号实例"
        );

        let mut locked = builtin_library();
        locked
            .switch_standard(&scene.document, "内置示例标准")
            .expect("切到当前值本身是 no-op");
        let error = locked
            .switch_standard(&scene.document, "项目标准B")
            .expect_err("已有实例后切换应被拒");
        assert_eq!(error.code(), "LEGEND_STANDARD_LOCKED");
        assert!(matches!(error, PidError::LegendStandardLocked { .. }));
        assert_eq!(
            locked.current_standard(),
            "内置示例标准",
            "切换失败后当前标准不得改变"
        );

        let block = match scene.document.get_entity(handle).expect("实体存在") {
            EntityType::Insert(insert) => insert.block_name.clone(),
            other => panic!("应插入 INSERT，实得 {other:?}"),
        };
        assert_eq!(block, VESSEL, "已有实例的块名不得改变");

        // 同一目标标准在空文档上仍然 Ok。
        let blank = crate::scene::Scene::new();
        let mut other = builtin_library();
        other
            .switch_standard(&blank.document, "项目标准B")
            .expect("空文档切换同一目标标准返回 Ok");
        assert_eq!(other.current_standard(), "项目标准B");
    }

    /// 校验器必须拒绝会撞块名或产生空块的定义。
    #[test]
    fn pid_symbols_reject_invalid_definitions() {
        let duplicate = r#"{
            "standard": "S",
            "categories": [
                { "id": "valve", "label": "阀门", "symbols": [
                    { "id": "g", "label": "G", "geometry": [
                        { "kind": "line", "from": [0,0], "to": [1,0] } ] },
                    { "id": "g", "label": "G2", "geometry": [
                        { "kind": "line", "from": [0,0], "to": [1,0] } ] }
                ] }
            ]
        }"#;
        assert_eq!(
            SymbolLibrary::load(duplicate)
                .expect_err("重复符号应被拒")
                .code(),
            "SYMBOL_LIBRARY_SYMBOL_DUPLICATED"
        );

        let unmapped = r#"{
            "standard": "S",
            "categories": [
                { "id": "valve", "label": "阀门", "symbols": [
                    { "id": "g", "label": "G", "geometry": [
                        { "kind": "line", "from": [0,0], "to": [1,0] } ] }
                ] }
            ],
            "symbol_type_map": [ { "symbol": "PID_SYM_nope", "types": ["X"] } ]
        }"#;
        assert_eq!(
            SymbolLibrary::load(unmapped)
                .expect_err("映射指向不存在的符号应被拒")
                .code(),
            "SYMBOL_LIBRARY_MAPPING_UNKNOWN_SYMBOL"
        );

        // D21：types[] 里的编码不在类型树中也必须能加载。
        let foreign = r#"{
            "standard": "S",
            "categories": [
                { "id": "valve", "label": "阀门", "symbols": [
                    { "id": "g", "label": "G", "geometry": [
                        { "kind": "line", "from": [0,0], "to": [1,0] } ] }
                ] }
            ],
            "symbol_type_map": [ { "symbol": "PID_SYM_valve_g", "types": ["NOT_IN_TYPES_JSON"] } ]
        }"#;
        let library = SymbolLibrary::load(foreign).expect("D21：不校验类型树成员资格");
        assert_eq!(library.symbols_of("NOT_IN_TYPES_JSON"), vec!["PID_SYM_valve_g"]);
    }
}

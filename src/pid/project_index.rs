//! 项目索引（SPEC-pid-project-index）：一张图的摘要，和由各图摘要合成的项目索引。
//!
//! [`summarize`] 只读一张图：带 `PID_AUTHOR` 记录的实体按 §3.1 的归类表
//! 归成对象，符号实例的每个端口各记一条；管段、管线自己的线和信号线的两端
//! 按 §3.2 现算接到了什么，连接符记下对端是怎么配的。同一张图摘要两次得到
//! 逐字节相同的 JSON，摘要不改图。[`graph`] 由摘要推出 §3.3 的连通图。
//!
//! [`list_drawings`] 列出项目目录里要读的图；[`build_index`] 把各图的摘要合成
//! 项目索引：跨图重号、页间连接符配对（§3.4–§3.6）；[`ProjectIndex::places`] 查
//! 一个位号在哪些图上。这几样都不读图、不写图。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use document::entities::Insert;
use document::types::Vector3;
use document::{Document, EntityType, Handle};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::master_data::{self, MasterDataError, PROJECT_FILE};
use super::persist::PidAuthor;
use super::propagate::split_props;
use super::symbols::{placed_ports, PortDir, SymbolLibrary};
use super::topology::{
    handle_text, parse_handle, ConnectorKind, ConnectorRecord, TopologyTable, FITTING_BREAK,
    ROLE_CONNECTOR_IN, ROLE_CONNECTOR_OUT, ROLE_FITTING, ROLE_PIPE_LINE, ROLE_PIPE_RUN,
    ROLE_PIPE_SEGMENT, ROLE_SIGNAL_LINE, TYPE_FITTING, TYPE_PIPE_LINE, TYPE_PIPE_RUN,
    TYPE_SIGNAL_LINE, TYPE_VALVE,
};
use super::type_lib::TypeLib;

/// 摘要格式的版本。
pub const SUMMARY_VERSION: u32 = 1;

/// 两点算同一处的默认距离（图纸单位），与协同里判「同一处」的口径一致。
pub const DEFAULT_TOLERANCE: f64 = 1e-6;

/// 设备类型树的根：它和它下面的类型都算设备。
const EQUIPMENT: &str = "EQUIPMENT";

const INSTRUMENT: &str = "INSTRUMENT";

/// 以它开头的类型码都算阀门。
const VALVE_PREFIX: &str = "VALVE_";

/// 类型库里没有时用的几个内置类型的名字。
const BUILTIN_LABELS: [(&str, &str); 7] = [
    (TYPE_VALVE, "阀门"),
    ("VALVE_GATE", "闸阀"),
    (INSTRUMENT, "仪表"),
    (TYPE_PIPE_LINE, "管线"),
    (TYPE_PIPE_RUN, "管路"),
    (TYPE_SIGNAL_LINE, "信号线"),
    (TYPE_FITTING, "管件"),
];

/// 方格边长的下限，容差给成 0 或负数时格子也不至于退化。
const MIN_CELL: f64 = 1e-9;

/// 图纸是从哪里读的：打开着的标签页，还是磁盘上的文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarySource {
    Open,
    File,
}

/// 文件大小与修改时间（Unix 秒），用来判断文件变没变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub size: u64,
    pub modified_at: i64,
}

/// 图纸内容以外、摘要要带上的几项，由调用方给。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryMeta {
    pub drawing_id: String,
    pub file: Option<String>,
    pub pbs: Option<String>,
    pub source: SummarySource,
    pub fingerprint: Option<Fingerprint>,
}

/// 一张图的摘要（§3.1）。字段顺序就是 JSON 里的顺序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawingSummary {
    pub version: u32,
    pub drawing_id: String,
    pub file: Option<String>,
    pub source: SummarySource,
    pub fingerprint: Option<Fingerprint>,
    pub pbs: Option<String>,
    pub length_unit: String,
    pub objects: Vec<SummaryObject>,
    pub segments: Vec<SummarySegment>,
    pub signal_ends: Vec<SignalEnd>,
    pub connectors: Vec<SummaryConnector>,
    pub ports: Vec<SummaryPort>,
}

/// 对象的种类，即 §3.1 归类表里的 `kind`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    #[serde(rename = "设备")]
    Equipment,
    #[serde(rename = "阀门")]
    Valve,
    #[serde(rename = "仪表")]
    Instrument,
    #[serde(rename = "管件")]
    Fitting,
    #[serde(rename = "打断符")]
    Break,
    #[serde(rename = "管线")]
    PipeLine,
    #[serde(rename = "管路")]
    PipeRun,
    #[serde(rename = "信号线")]
    SignalLine,
    #[serde(rename = "其他")]
    Other,
}

impl ObjectKind {
    /// 这种对象的位号是不是它自己的：管件和打断符记的是所在管路的位号。
    #[must_use]
    pub fn has_own_tag(self) -> bool {
        !matches!(self, Self::Fitting | Self::Break)
    }
}

/// 一个对象（§3.1 对象表）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryObject {
    pub kind: ObjectKind,
    pub type_code: String,
    pub type_label: String,
    pub tag: String,
    pub own_tag: bool,
    pub tag_source: String,
    pub tag_status: String,
    pub symbol: Option<String>,
    pub symbol_known: bool,
    pub props: BTreeMap<String, String>,
    pub overrides: Vec<String>,
    pub owner: Option<String>,
    pub layer: String,
    pub at: [f64; 2],
    pub bbox: [f64; 4],
    pub handle: String,
}

/// 符号实例的一个端口（§3.1 `ports`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryPort {
    pub owner_kind: ObjectKind,
    pub type_code: String,
    pub tag: String,
    pub port: String,
    pub direction: Option<PortDir>,
    pub at: [f64; 2],
    pub joined: bool,
}

/// 一根线段（§3.1 `segments`）：管路的一段，或管线自己的一根线。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummarySegment {
    pub line: String,
    pub run: Option<String>,
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub length: f64,
    pub joint_a: Option<Joint>,
    pub joint_b: Option<Joint>,
    pub handle: String,
}

/// 信号线的一端（§3.1 `signal_ends`）：它所有线的端点里只出现一次的点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalEnd {
    pub tag: String,
    pub at: [f64; 2],
    pub joint: Option<Joint>,
}

/// 线的一端接到了什么（§3.2），按序号先中先得；什么都没接是 `None`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Joint {
    /// 第 1 条：拓扑表里记着接在管件上，端口名是表里的（「连接点1」）。
    Fitting {
        type_code: String,
        tag: String,
        port: String,
    },
    /// 第 2 条：落在打断符上。打断符是隔断，不算接通，也不算悬空。
    Break,
    /// 第 3 条：与符号的端口重合，端口名是符号库里的。
    Port {
        type_code: String,
        tag: String,
        port: String,
    },
    /// 第 4 条：落在页间连接符的圆里。
    Connector { kind: String, line: String },
    /// 第 5、6 条：接在另一根线的端点上，或者接在它中间（`tee`）。
    Segment {
        line: String,
        run: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        tee: bool,
    },
}

/// 连接符的对端是怎么记的（§3.1 `peer_kind`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerKind {
    /// 什么都没记。
    None,
    /// 只指了目标图，没配对。
    Target,
    /// 协同服务器配的对。
    Server,
    /// 同一个桌面进程里配的对。
    Local,
}

/// 页间连接符（§3.1 `connectors`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryConnector {
    pub kind: String,
    pub line: String,
    pub at: [f64; 2],
    pub radius: f64,
    pub peer_kind: PeerKind,
    pub peer_drawing: Option<String>,
    pub upstream: Option<String>,
    pub downstream: Option<String>,
    pub handle: String,
}

/// 连通图（§3.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<String>,
    pub edges: Vec<[String; 2]>,
}

/// 项目索引格式的版本。
pub const INDEX_VERSION: u32 = 1;

/// 项目目录里要读的一张图（§3.7）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingFile {
    /// 文件名，不带目录。
    pub file: String,
    pub path: PathBuf,
    pub drawing_id: String,
    pub pbs: Option<String>,
}

/// 读不出来、做不了摘要的一张图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedDrawing {
    pub drawing_id: String,
    /// 文件名，不带目录。
    pub file: String,
    pub pbs: Option<String>,
    /// 读不出来的原因。
    pub error: String,
}

/// 项目索引（§3.4）。字段顺序就是 JSON 里的顺序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectIndex {
    pub version: u32,
    pub project_dir: String,
    /// 生成的时间，Unix 秒。
    pub generated_at: i64,
    /// 所有图都读出来了才为真。
    pub complete: bool,
    pub drawings: Vec<IndexDrawing>,
    pub duplicates: Vec<Duplicate>,
    pub links: Vec<Link>,
    pub unresolved: Vec<Unresolved>,
}

/// 索引里的一张图（§3.4 `drawings[]`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDrawing {
    pub drawing_id: String,
    pub file: Option<String>,
    pub source: SummarySource,
    pub pbs: Option<String>,
    /// 对象数；读不出来的图是 `None`。
    pub objects: Option<usize>,
    /// 读不出来的原因。
    pub error: Option<String>,
}

/// 跨图重复的一个位号（§3.5）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Duplicate {
    pub type_code: String,
    pub tag: String,
    pub places: Vec<Place>,
}

/// 一个对象在项目里的一处。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub drawing_id: String,
    pub at: [f64; 2],
    pub handle: String,
}

/// 页间连接符属于哪一族。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkFamily {
    Pipe,
    Signal,
}

/// 配成对的一对连接符（§3.4 `links[]`）：`from` 是流出侧，`to` 是流入侧。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub family: LinkFamily,
    pub from: LinkEnd,
    pub to: LinkEnd,
}

/// 一对连接符里的一个。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEnd {
    pub drawing_id: String,
    pub line: String,
    pub at: [f64; 2],
    pub handle: String,
}

/// 连接符为什么没配成对（§3.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    /// 记着配过对，对面却找不到。
    OneSided,
    /// 记着的对端图不在项目里。
    TargetMissing,
    /// 没配过对。
    Unpaired,
    /// 对面不止一个，分不出是哪个。
    Ambiguous,
}

/// 没配成对的一个连接符（§3.4 `unresolved[]`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unresolved {
    pub drawing_id: String,
    pub kind: String,
    pub line: String,
    pub at: [f64; 2],
    pub handle: String,
    pub status: LinkStatus,
    /// 它指的目标图号；没指是 `None`。
    pub target: Option<String>,
}

/// 列项目图纸、建索引时的错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// 两个文件得出同一个图号（§3.7），索引不生成。`files` 按文件名排。
    DrawingIdDuplicated {
        drawing_id: String,
        files: [String; 2],
    },
    /// 项目文件读不出来。
    ProjectFile(MasterDataError),
    /// 读项目目录时文件系统报的错。
    Io { path: String, reason: String },
}

impl IndexError {
    /// 错误码。项目文件的错用主数据那边的错误码。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::DrawingIdDuplicated { .. } => "INDEX_DRAWING_ID_DUPLICATED",
            Self::ProjectFile(error) => error.code(),
            Self::Io { .. } => "INDEX_IO_ERROR",
        }
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DrawingIdDuplicated {
                drawing_id,
                files: [first, second],
            } => write!(
                f,
                "{}: {first} 和 {second} 的图号都是 {drawing_id}",
                self.code()
            ),
            Self::ProjectFile(error) => write!(f, "{PROJECT_FILE}：{error}"),
            Self::Io { path, reason } => write!(f, "{}: {path} {reason}", self.code()),
        }
    }
}

impl std::error::Error for IndexError {}

/// 摘要一张图（§3.1）。只读，不改 `doc`。
///
/// `tolerance` 是两点算同一处的距离（图纸单位），平时用 [`DEFAULT_TOLERANCE`]。
#[must_use]
pub fn summarize(
    doc: &Document,
    library: &SymbolLibrary,
    meta: SummaryMeta,
    tolerance: f64,
) -> DrawingSummary {
    let types = TypeLib::embedded();
    let mut table = TopologyTable::load(doc);
    table.prune(doc);
    let records = Records::new(&table);
    let mut objects: Vec<(u64, SummaryObject)> = Vec::new();
    let mut ports: Vec<PortDraft> = Vec::new();
    let mut pipe_lines: BTreeMap<String, Vec<Piece>> = BTreeMap::new();
    let mut signal_lines: BTreeMap<String, Vec<Piece>> = BTreeMap::new();
    let mut run_boxes: HashMap<&str, [f64; 4]> = HashMap::new();
    let mut segments: Vec<SummarySegment> = Vec::new();
    let mut signal_wires: Vec<Wire> = Vec::new();
    let mut connectors: Vec<SummaryConnector> = Vec::new();
    for entity in doc.entities() {
        let handle = entity.common().handle;
        let Some(author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        let key = handle_text(handle);
        let is_break = records.breaks.contains(key.as_str());
        let class = classify(&author.topology, &author.type_code, is_break, &types);
        let (at, bbox) = placement(doc, entity);
        match class {
            Class::Connector => {
                let record = records.connectors.get(key.as_str()).copied();
                connectors.push(connector_of(entity, &author, record, at, key));
            }
            Class::Segment => {
                let run = records.segment_run.get(key.as_str()).copied();
                if let Some(run) = run {
                    run_boxes
                        .entry(run)
                        .and_modify(|merged| *merged = merge(*merged, bbox))
                        .or_insert(bbox);
                }
                if let Some(ends) = line_ends(entity) {
                    let run = run.unwrap_or_default();
                    let line = records.line_of_run.get(run).copied().unwrap_or_default();
                    segments.push(segment_of(line, Some(run), ends, key));
                }
            }
            Class::PipeLine | Class::SignalLine => {
                if let Some([a, b]) = line_ends(entity) {
                    if class == Class::PipeLine {
                        segments.push(segment_of(&author.tag, None, [a, b], key));
                    } else {
                        signal_wires.push(Wire {
                            line: author.tag.clone(),
                            run: None,
                            a,
                            b,
                        });
                    }
                }
                let groups = if class == Class::PipeLine {
                    &mut pipe_lines
                } else {
                    &mut signal_lines
                };
                let layer = entity.common().layer.clone();
                groups.entry(author.tag.clone()).or_default().push(Piece {
                    handle,
                    author,
                    layer,
                    at,
                    bbox,
                });
            }
            Class::Object(kind) => {
                let owner = match kind {
                    ObjectKind::PipeRun => records.run_line.get(key.as_str()),
                    ObjectKind::Valve | ObjectKind::Fitting | ObjectKind::Break => {
                        records.fitting_run.get(key.as_str())
                    }
                    _ => None,
                };
                let insert = match entity {
                    EntityType::Insert(insert) => Some(insert),
                    _ => None,
                };
                let (props, overrides) = split_props(&author.props);
                let object = SummaryObject {
                    kind,
                    type_label: type_label(&types, &author.type_code),
                    type_code: author.type_code,
                    tag: author.tag,
                    own_tag: kind.has_own_tag(),
                    tag_source: author.tag_source,
                    tag_status: author.tag_status,
                    symbol: insert.map(|insert| insert.block_name.clone()),
                    symbol_known: insert
                        .is_none_or(|insert| library.symbol(&insert.block_name).is_some()),
                    props,
                    overrides: overrides.into_iter().collect(),
                    owner: owner.map(|tag| (*tag).to_string()),
                    layer: entity.common().layer.clone(),
                    at,
                    bbox,
                    handle: key,
                };
                if let Some(insert) = insert {
                    ports.extend(ports_of(doc, library, insert, &object));
                }
                objects.push((handle.value(), object));
            }
        }
    }
    for (_, object) in objects
        .iter_mut()
        .filter(|(_, object)| object.kind == ObjectKind::PipeRun)
    {
        if let Some(bbox) = run_boxes.get(object.tag.as_str()) {
            object.bbox = *bbox;
        }
    }
    for (tag, pieces) in pipe_lines {
        let main = records.line_handle.get(tag.as_str()).copied();
        objects.push(pipe_line_object(pieces, main, &types));
    }
    for pieces in signal_lines.into_values() {
        objects.push(signal_line_object(pieces, &types));
    }
    objects.sort_by(|(left_handle, left), (right_handle, right)| {
        left.type_code
            .cmp(&right.type_code)
            .then_with(|| left.tag.cmp(&right.tag))
            .then_with(|| compare_at(left.at, right.at))
            .then_with(|| left_handle.cmp(right_handle))
    });
    ports.sort_by(|left, right| {
        let (left, right) = (&left.port, &right.port);
        left.tag
            .cmp(&right.tag)
            .then_with(|| compare_at(left.at, right.at))
            .then_with(|| left.type_code.cmp(&right.type_code))
            .then_with(|| left.port.cmp(&right.port))
    });
    segments.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.run.cmp(&right.run))
            .then_with(|| compare_at(left.a, right.a))
            .then_with(|| compare_at(left.b, right.b))
            .then_with(|| left.handle.cmp(&right.handle))
    });
    signal_wires.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| compare_at(left.a, right.a))
            .then_with(|| compare_at(left.b, right.b))
    });
    connectors.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| compare_at(left.at, right.at))
            .then_with(|| left.handle.cmp(&right.handle))
    });
    let mut signal_ends = signal_ends_of(&signal_wires, tolerance);
    let joined = {
        let lookup = Lookup::new(&objects, &ports, &connectors, tolerance);
        let wires: Vec<Wire> = segments.iter().map(Wire::of).collect();
        let pipes = Family::new(&wires, tolerance);
        let signals = Family::new(&signal_wires, tolerance);
        let mut joined = vec![false; ports.len()];
        for (index, segment) in segments.iter_mut().enumerate() {
            let [start, end] = records
                .segment_peers
                .get(segment.handle.as_str())
                .copied()
                .unwrap_or_default();
            segment.joint_a = lookup.resolve(start, segment.a, &pipes, index, &mut joined);
            segment.joint_b = lookup.resolve(end, segment.b, &pipes, index, &mut joined);
        }
        for (end, own) in &mut signal_ends {
            end.joint = lookup.resolve(None, end.at, &signals, *own, &mut joined);
        }
        lookup.join_coincident(&mut joined);
        joined
    };
    let ports = ports
        .into_iter()
        .zip(joined)
        .map(|(draft, joined)| SummaryPort {
            joined,
            ..draft.port
        })
        .collect();
    DrawingSummary {
        version: SUMMARY_VERSION,
        drawing_id: meta.drawing_id,
        file: meta.file,
        source: meta.source,
        fingerprint: meta.fingerprint,
        pbs: meta.pbs,
        length_unit: length_unit(doc.header.insertion_units).to_string(),
        objects: objects.into_iter().map(|(_, object)| object).collect(),
        segments,
        signal_ends: signal_ends.into_iter().map(|(end, _)| end).collect(),
        connectors,
        ports,
    }
}

/// 由摘要推出连通图（§3.3）。节点是有自己位号的对象和连接符；线段、信号线的
/// 每个端点出一条边「线的主人 — 接到的东西」，哪一头不在图里都不出边。
#[must_use]
pub fn graph(summary: &DrawingSummary) -> Graph {
    let mut nodes: BTreeSet<String> = summary
        .objects
        .iter()
        .filter(|object| object.own_tag && !object.tag.is_empty())
        .map(|object| node(&object.type_code, &object.tag))
        .collect();
    nodes.extend(
        summary
            .connectors
            .iter()
            .map(|connector| connector_node(&connector.kind, &connector.line)),
    );
    let pipe_ends = summary.segments.iter().flat_map(|segment| {
        let owner = wire_owner(&segment.line, segment.run.as_deref());
        [&segment.joint_a, &segment.joint_b]
            .into_iter()
            .flatten()
            .map(move |joint| (owner.clone(), joint, false))
    });
    let signal_ends = summary.signal_ends.iter().filter_map(|end| {
        let joint = end.joint.as_ref()?;
        Some((node(TYPE_SIGNAL_LINE, &end.tag), joint, true))
    });
    let mut edges: BTreeSet<[String; 2]> = BTreeSet::new();
    for (owner, joint, signal) in pipe_ends.chain(signal_ends) {
        let Some(other) = joint_node(joint, signal, &nodes) else {
            continue;
        };
        if owner != other && nodes.contains(&owner) {
            edges.insert(if owner < other {
                [owner, other]
            } else {
                [other, owner]
            });
        }
    }
    Graph {
        nodes: nodes.into_iter().collect(),
        edges: edges.into_iter().collect(),
    }
}

fn node(type_code: &str, tag: &str) -> String {
    format!("{type_code}|{tag}")
}

fn connector_node(kind: &str, line: &str) -> String {
    format!("connector|{line}|{kind}")
}

/// 线段的主人：管路的线段算管路的，管线自己的线算管线的。
fn wire_owner(line: &str, run: Option<&str>) -> String {
    run.map_or_else(
        || node(TYPE_PIPE_LINE, line),
        |run| node(TYPE_PIPE_RUN, run),
    )
}

/// 接点在图里的名字：接到端口的带 `#<端口名>`，接到线上的是那根线的主人（`signal`
/// 时是信号线）。打断符和图里没有的对象没有名字。
fn joint_node(joint: &Joint, signal: bool, nodes: &BTreeSet<String>) -> Option<String> {
    let name = match joint {
        Joint::Fitting {
            type_code,
            tag,
            port,
        }
        | Joint::Port {
            type_code,
            tag,
            port,
        } => {
            let base = node(type_code, tag);
            return nodes.contains(&base).then(|| format!("{base}#{port}"));
        }
        Joint::Break => return None,
        Joint::Connector { kind, line } => connector_node(kind, line),
        Joint::Segment { line, .. } if signal => node(TYPE_SIGNAL_LINE, line),
        Joint::Segment { line, run, .. } => wire_owner(line, run.as_deref()),
    };
    nodes.contains(&name).then_some(name)
}

/// 拓扑表里按句柄、位号查的几样。
struct Records<'a> {
    /// 管路句柄 → 所在管线的位号。
    run_line: HashMap<&'a str, &'a str>,
    /// 管段句柄 → 所在管路的位号。
    segment_run: HashMap<&'a str, &'a str>,
    /// 阀门、管件、打断符的句柄 → 所在管路的位号。
    fitting_run: HashMap<&'a str, &'a str>,
    /// 打断符的句柄。
    breaks: HashSet<&'a str>,
    /// 管线位号 → 拓扑表记的那根线的句柄。
    line_handle: HashMap<&'a str, &'a str>,
    /// 管路位号 → 所在管线的位号。
    line_of_run: HashMap<&'a str, &'a str>,
    /// 管段句柄 → 起点、终点的 `peer`。
    segment_peers: HashMap<&'a str, [Option<&'a str>; 2]>,
    /// 连接符句柄 → 它的记录。
    connectors: HashMap<&'a str, &'a ConnectorRecord>,
}

impl<'a> Records<'a> {
    fn new(table: &'a TopologyTable) -> Self {
        Self {
            run_line: table
                .pipe_runs
                .iter()
                .map(|run| (run.handle.as_str(), run.line_tag.as_str()))
                .collect(),
            segment_run: table
                .pipe_segments
                .iter()
                .map(|segment| (segment.handle.as_str(), segment.run_tag.as_str()))
                .collect(),
            fitting_run: table
                .fittings
                .iter()
                .map(|fitting| (fitting.handle.as_str(), fitting.run_tag.as_str()))
                .collect(),
            breaks: table
                .fittings
                .iter()
                .filter(|fitting| fitting.fitting_type == FITTING_BREAK)
                .map(|fitting| fitting.handle.as_str())
                .collect(),
            line_handle: table
                .pipe_lines
                .iter()
                .map(|line| (line.tag.as_str(), line.handle.as_str()))
                .collect(),
            line_of_run: table
                .pipe_runs
                .iter()
                .map(|run| (run.tag.as_str(), run.line_tag.as_str()))
                .collect(),
            segment_peers: table
                .pipe_segments
                .iter()
                .map(|segment| {
                    let peers = [&segment.start, &segment.end].map(|end| end.peer.as_deref());
                    (segment.handle.as_str(), peers)
                })
                .collect(),
            connectors: table
                .connectors
                .iter()
                .map(|connector| (connector.handle.as_str(), connector))
                .collect(),
        }
    }
}

/// 管线或信号线的一根线。
struct Piece {
    handle: Handle,
    author: PidAuthor,
    layer: String,
    at: [f64; 2],
    bbox: [f64; 4],
}

/// 一条管线：位号相同的几根线合成一个对象。拓扑表记的那根线是主线（不在
/// 就取句柄最小的），属性、定位点和句柄都取主线的。
fn pipe_line_object(
    mut pieces: Vec<Piece>,
    main: Option<&str>,
    types: &TypeLib,
) -> (u64, SummaryObject) {
    pieces.sort_by_key(|piece| piece.handle.value());
    let bbox = merged(&pieces);
    let index = main
        .and_then(|main| {
            pieces
                .iter()
                .position(|piece| handle_text(piece.handle) == main)
        })
        .unwrap_or(0);
    let main = pieces.swap_remove(index);
    let (props, overrides) = split_props(&main.author.props);
    composite(ObjectKind::PipeLine, main, props, overrides, bbox, types)
}

/// 一条信号线：位号相同的几根线合成一个对象。属性逐项取第一个非空值，
/// 句柄取最小的。
fn signal_line_object(mut pieces: Vec<Piece>, types: &TypeLib) -> (u64, SummaryObject) {
    pieces.sort_by_key(|piece| piece.handle.value());
    let bbox = merged(&pieces);
    let mut props: BTreeMap<String, String> = BTreeMap::new();
    let mut overrides = BTreeSet::new();
    for piece in &pieces {
        let (values, marks) = split_props(&piece.author.props);
        for (field, value) in values {
            let slot = props.entry(field).or_default();
            if slot.is_empty() {
                *slot = value;
            }
        }
        overrides.extend(marks);
    }
    let main = pieces.swap_remove(0);
    composite(ObjectKind::SignalLine, main, props, overrides, bbox, types)
}

/// 几根线合成的对象：位号、类型、图层、定位点和句柄取主线的。
fn composite(
    kind: ObjectKind,
    main: Piece,
    props: BTreeMap<String, String>,
    overrides: BTreeSet<String>,
    bbox: [f64; 4],
    types: &TypeLib,
) -> (u64, SummaryObject) {
    let object = SummaryObject {
        kind,
        type_label: type_label(types, &main.author.type_code),
        type_code: main.author.type_code,
        tag: main.author.tag,
        own_tag: true,
        tag_source: main.author.tag_source,
        tag_status: main.author.tag_status,
        symbol: None,
        symbol_known: true,
        props,
        overrides: overrides.into_iter().collect(),
        owner: None,
        layer: main.layer,
        at: main.at,
        bbox,
        handle: handle_text(main.handle),
    };
    (main.handle.value(), object)
}

fn merged(pieces: &[Piece]) -> [f64; 4] {
    pieces
        .iter()
        .map(|piece| piece.bbox)
        .reduce(merge)
        .unwrap_or_default()
}

/// 还没判 `joined` 的端口，带着它所在符号实例的句柄。
struct PortDraft {
    insert: Handle,
    port: SummaryPort,
}

/// 一个符号实例的端口：名字和流向按符号库里定义的顺序，位置是摆好后的世界
/// 坐标；库里没有这个块时按顺序叫 `#1`、`#2`，流向空着。
fn ports_of(
    doc: &Document,
    library: &SymbolLibrary,
    insert: &Insert,
    owner: &SummaryObject,
) -> Vec<PortDraft> {
    let defs = library
        .symbol(&insert.block_name)
        .map(|symbol| symbol.ports.as_slice())
        .unwrap_or_default();
    placed_ports(doc, insert)
        .into_iter()
        .enumerate()
        .map(|(index, (x, y))| {
            let def = defs.get(index);
            PortDraft {
                insert: insert.common.handle,
                port: SummaryPort {
                    owner_kind: owner.kind,
                    type_code: owner.type_code.clone(),
                    tag: owner.tag.clone(),
                    port: def.map_or_else(|| format!("#{}", index + 1), |def| def.name.clone()),
                    direction: def.map(|def| def.direction),
                    at: [x, y],
                    joined: false,
                },
            }
        })
        .collect()
}

/// 判第 5、6 条用的一根线：属于谁，两端在哪。信号线的 `line` 是它的位号。
#[derive(Debug, Clone)]
struct Wire {
    line: String,
    run: Option<String>,
    a: [f64; 2],
    b: [f64; 2],
}

impl Wire {
    fn of(segment: &SummarySegment) -> Self {
        Self {
            line: segment.line.clone(),
            run: segment.run.clone(),
            a: segment.a,
            b: segment.b,
        }
    }
}

/// 一根线段，接点待判。没有拓扑表记录的管段，`line`、`run` 都是空串。
fn segment_of(
    line: &str,
    run: Option<&str>,
    [a, b]: [[f64; 2]; 2],
    handle: String,
) -> SummarySegment {
    SummarySegment {
        line: line.to_string(),
        run: run.map(str::to_string),
        a,
        b,
        length: distance(a, b),
        joint_a: None,
        joint_b: None,
        handle,
    }
}

/// 每条信号线的两端（§3.1）：它所有线的端点里，同一条信号线上别处没有重合点的
/// 那些。带着端点所在的线在 `wires` 里的下标，按位号、坐标排。
fn signal_ends_of(wires: &[Wire], tolerance: f64) -> Vec<(SignalEnd, usize)> {
    let grid = PointGrid::new(wires.iter().flat_map(|wire| [wire.a, wire.b]), tolerance);
    let mut ends = Vec::new();
    for (index, wire) in wires.iter().enumerate() {
        for (slot, at) in [wire.a, wire.b].into_iter().enumerate() {
            let alone = grid
                .near(at)
                .filter(|other| *other != 2 * index + slot)
                .all(|other| wires[other / 2].line != wire.line);
            if alone {
                let end = SignalEnd {
                    tag: wire.line.clone(),
                    at,
                    joint: None,
                };
                ends.push((end, index));
            }
        }
    }
    ends.sort_by(|(left, _), (right, _)| {
        left.tag
            .cmp(&right.tag)
            .then_with(|| compare_at(left.at, right.at))
    });
    ends
}

/// 一个页间连接符：圆心、半径取图上的圆，其余取拓扑表的记录。表里没有它的
/// 记录时，种类按角色和类型编码推，线号空着。
fn connector_of(
    entity: &EntityType,
    author: &PidAuthor,
    record: Option<&ConnectorRecord>,
    at: [f64; 2],
    handle: String,
) -> SummaryConnector {
    let (at, radius) = match entity {
        EntityType::Circle(circle) => (xy(circle.center), circle.radius),
        _ => (at, 0.0),
    };
    let peer_kind = record.map_or(PeerKind::None, peer_kind_of);
    SummaryConnector {
        kind: record.map_or_else(|| guessed_kind(author), |record| record.kind.clone()),
        line: record
            .map(|record| record.line_tag.clone())
            .unwrap_or_default(),
        at,
        radius,
        peer_kind,
        peer_drawing: record
            .filter(|_| matches!(peer_kind, PeerKind::Target | PeerKind::Server))
            .and_then(|record| record.peer_drawing.clone()),
        upstream: record.and_then(|record| record.upstream.clone()),
        downstream: record.and_then(|record| record.downstream.clone()),
        handle,
    }
}

/// §3.1 的判法：记了 `peer` 是同进程配的对；只记了 `peer_drawing` 时，上下游都有是
/// 服务器配的对，否则只指了目标图。
fn peer_kind_of(record: &ConnectorRecord) -> PeerKind {
    if record.peer.is_some() {
        PeerKind::Local
    } else if record.peer_drawing.is_none() {
        PeerKind::None
    } else if record.upstream.is_some() && record.downstream.is_some() {
        PeerKind::Server
    } else {
        PeerKind::Target
    }
}

fn guessed_kind(author: &PidAuthor) -> String {
    [
        ConnectorKind::PipeIn,
        ConnectorKind::PipeOut,
        ConnectorKind::SignalIn,
        ConnectorKind::SignalOut,
    ]
    .into_iter()
    .find(|kind| kind.role() == author.topology && kind.type_code() == author.type_code)
    .map_or_else(String::new, |kind| kind.as_str().to_string())
}

/// 判接点（§3.2）时查的几样，各自分好了桶。几样同时对上时取排在前面的那个。
struct Lookup<'a> {
    tolerance: f64,
    /// 对象句柄 → 对象，第 1 条按管段端点的 `peer` 找管件。
    objects: HashMap<u64, &'a SummaryObject>,
    /// 打断符包围盒的外接圆。
    breaks: Circles,
    ports: &'a [PortDraft],
    port_grid: PointGrid,
    /// 符号实例的句柄 → 它的端口在 `ports` 里的下标。
    ports_of: HashMap<u64, Vec<usize>>,
    connectors: &'a [SummaryConnector],
    connector_circles: Circles,
}

impl<'a> Lookup<'a> {
    fn new(
        objects: &'a [(u64, SummaryObject)],
        ports: &'a [PortDraft],
        connectors: &'a [SummaryConnector],
        tolerance: f64,
    ) -> Self {
        let breaks = objects
            .iter()
            .filter(|(_, object)| object.kind == ObjectKind::Break)
            .map(|(_, object)| {
                let [x0, y0, x1, y1] = object.bbox;
                let center = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
                (center, distance([x0, y0], [x1, y1]) / 2.0)
            })
            .collect();
        let mut ports_of: HashMap<u64, Vec<usize>> = HashMap::new();
        for (index, draft) in ports.iter().enumerate() {
            ports_of
                .entry(draft.insert.value())
                .or_default()
                .push(index);
        }
        let circles = connectors
            .iter()
            .map(|connector| (connector.at, connector.radius))
            .collect();
        Self {
            tolerance,
            objects: objects
                .iter()
                .map(|(handle, object)| (*handle, object))
                .collect(),
            breaks: Circles::new(breaks, tolerance),
            ports,
            port_grid: PointGrid::new(ports.iter().map(|draft| draft.port.at), tolerance),
            ports_of,
            connectors,
            connector_circles: Circles::new(circles, tolerance),
        }
    }

    /// 按 §3.2 的顺序判端点 `at` 接到了什么，先中先得；接上的端口记进 `joined`。
    /// `peer` 是拓扑表里这个管段端点的 `peer`，`own` 是端点所在的线在 `family` 里的
    /// 下标。
    fn resolve(
        &self,
        peer: Option<&str>,
        at: [f64; 2],
        family: &Family,
        own: usize,
        joined: &mut [bool],
    ) -> Option<Joint> {
        if let Some((insert, object, port)) = self.fitting(peer) {
            if let Some(index) = self.nearest_port(insert, at) {
                joined[index] = true;
            }
            return Some(Joint::Fitting {
                type_code: object.type_code.clone(),
                tag: object.tag.clone(),
                port: port.to_string(),
            });
        }
        if self.breaks.hit(at, self.tolerance).is_some() {
            return Some(Joint::Break);
        }
        if let Some(index) = self.port_grid.near(at).min() {
            joined[index] = true;
            let port = &self.ports[index].port;
            return Some(Joint::Port {
                type_code: port.type_code.clone(),
                tag: port.tag.clone(),
                port: port.port.clone(),
            });
        }
        if let Some(index) = self.connector_circles.hit(at, self.tolerance) {
            let connector = &self.connectors[index];
            return Some(Joint::Connector {
                kind: connector.kind.clone(),
                line: connector.line.clone(),
            });
        }
        let (index, tee) = family.hit(own, at, self.tolerance)?;
        let wire = &family.wires[index];
        Some(Joint::Segment {
            line: wire.line.clone(),
            run: wire.run.clone(),
            tee,
        })
    }

    /// 第 1 条：`peer`（`<句柄>:<端口名>`）指着的对象和端口名；摘要里没有那个对象就不算。
    fn fitting<'p>(&self, peer: Option<&'p str>) -> Option<(u64, &'a SummaryObject, &'p str)> {
        let (handle, port) = peer?.split_once(':')?;
        let handle = parse_handle(handle)?.value();
        let object = *self.objects.get(&handle)?;
        Some((handle, object, port))
    }

    /// 管件两侧的管段让开了一道缝，端点不在端口上，表里的端口名（「连接点1」）又
    /// 不是符号库的，所以算离端点最近的那个端口。
    fn nearest_port(&self, insert: u64, at: [f64; 2]) -> Option<usize> {
        let reach = |index: &usize| distance(self.ports[*index].port.at, at);
        self.ports_of
            .get(&insert)?
            .iter()
            .min_by(|left, right| reach(left).total_cmp(&reach(right)))
            .copied()
    }

    /// 与别的符号的端口重合的端口也算接上。
    fn join_coincident(&self, joined: &mut [bool]) {
        for (index, draft) in self.ports.iter().enumerate() {
            if self
                .port_grid
                .near(draft.port.at)
                .any(|other| self.ports[other].insert != draft.insert)
            {
                joined[index] = true;
            }
        }
    }
}

/// 一族线：管段和管线自己的线是一族，信号线是另一族。第 5、6 条只在族内找：
/// 管子的端点不算接在信号线上，反过来也一样。
struct Family<'w> {
    wires: &'w [Wire],
    /// 每根线的两端，第 `i` 根是 `2i`、`2i + 1`。
    ends: PointGrid,
    spans: BoxGrid,
}

impl<'w> Family<'w> {
    fn new(wires: &'w [Wire], tolerance: f64) -> Self {
        Self {
            wires,
            ends: PointGrid::new(wires.iter().flat_map(|wire| [wire.a, wire.b]), tolerance),
            spans: BoxGrid::new(wires.iter().map(|wire| span(wire.a, wire.b)), tolerance),
        }
    }

    /// 第 5 条：与另一根线的端点重合；第 6 条：落在另一根线的中间（`tee`）。
    /// `own` 是端点自己所在的线。
    fn hit(&self, own: usize, at: [f64; 2], tolerance: f64) -> Option<(usize, bool)> {
        let end = self
            .ends
            .near(at)
            .map(|end| end / 2)
            .filter(|index| *index != own)
            .min();
        if let Some(index) = end {
            return Some((index, false));
        }
        self.spans
            .at(at)
            .filter(|index| *index != own)
            .filter(|index| {
                let wire = &self.wires[*index];
                distance_to_wire(at, wire.a, wire.b) <= tolerance
            })
            .min()
            .map(|index| (index, true))
    }
}

/// 一组圆（打断符包围盒的外接圆、连接符），按各自的包围盒分桶。
struct Circles {
    circles: Vec<([f64; 2], f64)>,
    grid: BoxGrid,
}

impl Circles {
    fn new(circles: Vec<([f64; 2], f64)>, tolerance: f64) -> Self {
        let boxes = circles
            .iter()
            .map(|&([x, y], radius)| [x - radius, y - radius, x + radius, y + radius]);
        let grid = BoxGrid::new(boxes, tolerance);
        Self { circles, grid }
    }

    /// 盖住 `at` 的圆（含圆周，再放宽一个容差）里排在最前的那个。
    fn hit(&self, at: [f64; 2], tolerance: f64) -> Option<usize> {
        self.grid
            .at(at)
            .filter(|index| {
                let (center, radius) = self.circles[*index];
                distance(at, center) <= radius + tolerance
            })
            .min()
    }
}

/// 按方格分桶的点：找离某处不超过容差的点只看周围九格，不用两两比。
struct PointGrid {
    cell: f64,
    tolerance: f64,
    points: Vec<[f64; 2]>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl PointGrid {
    fn new(points: impl IntoIterator<Item = [f64; 2]>, tolerance: f64) -> Self {
        let mut grid = Self {
            cell: tolerance.max(MIN_CELL),
            tolerance,
            points: Vec::new(),
            cells: HashMap::new(),
        };
        for point in points {
            let cell = cell_of(grid.cell, point);
            grid.cells.entry(cell).or_default().push(grid.points.len());
            grid.points.push(point);
        }
        grid
    }

    /// 离 `at` 不超过容差的点的下标。
    fn near(&self, at: [f64; 2]) -> impl Iterator<Item = usize> + '_ {
        let (column, row) = cell_of(self.cell, at);
        (column.saturating_sub(1)..=column.saturating_add(1))
            .flat_map(move |x| (row.saturating_sub(1)..=row.saturating_add(1)).map(move |y| (x, y)))
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .filter(move |index| distance(self.points[*index], at) <= self.tolerance)
    }
}

/// 按方格分桶的包围盒：找盖住某点的盒子只看那点所在的一格。格子边长取所有
/// 盒子合起来的范围除以 √n（大致一格一个盒子），不小于两倍容差；一个盒子最多占
/// 范围内的全部格子。
struct BoxGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl BoxGrid {
    /// 盒子各向外放宽一个容差再分桶。
    fn new(boxes: impl IntoIterator<Item = [f64; 4]>, tolerance: f64) -> Self {
        let boxes: Vec<[f64; 4]> = boxes
            .into_iter()
            .map(|[x0, y0, x1, y1]| {
                [
                    x0 - tolerance,
                    y0 - tolerance,
                    x1 + tolerance,
                    y1 + tolerance,
                ]
            })
            .collect();
        let extent = boxes
            .iter()
            .copied()
            .reduce(merge)
            .map_or(0.0, |[x0, y0, x1, y1]| (x1 - x0).max(y1 - y0));
        let per_side = (boxes.len() as f64).sqrt().ceil().max(1.0);
        let mut grid = Self {
            cell: (extent / per_side).max(2.0 * tolerance).max(MIN_CELL),
            cells: HashMap::new(),
        };
        for (index, [x0, y0, x1, y1]) in boxes.into_iter().enumerate() {
            let (left, bottom) = cell_of(grid.cell, [x0, y0]);
            let (right, top) = cell_of(grid.cell, [x1, y1]);
            for column in left..=right {
                for row in bottom..=top {
                    grid.cells.entry((column, row)).or_default().push(index);
                }
            }
        }
        grid
    }

    /// 盒子可能盖住 `at` 的那几个的下标：只按格子粗筛，调用处再细判。
    fn at(&self, at: [f64; 2]) -> impl Iterator<Item = usize> + '_ {
        self.cells
            .get(&cell_of(self.cell, at))
            .into_iter()
            .flatten()
            .copied()
    }
}

/// 点所在的格子。`as` 饱和取整：极远的坐标挤进边上的格子，只是慢些，不会错。
fn cell_of(cell: f64, [x, y]: [f64; 2]) -> (i64, i64) {
    ((x / cell).floor() as i64, (y / cell).floor() as i64)
}

/// 实体的定位点与包围盒 `[minx, miny, maxx, maxy]`：符号取插入点，点取它自己，
/// 线取起点，圆取圆心，别的取包围盒的左下角。
fn placement(doc: &Document, entity: &EntityType) -> ([f64; 2], [f64; 4]) {
    let bounds = match entity {
        EntityType::Insert(insert) => symbol_bounds(doc, insert),
        other => {
            let bounds = other.as_entity().bounding_box();
            Some([bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y])
        }
    };
    let at = match entity {
        EntityType::Insert(insert) => xy(insert.insert_point),
        EntityType::Point(point) => xy(point.location),
        EntityType::Line(line) => xy(line.start),
        EntityType::Circle(circle) => xy(circle.center),
        _ => bounds.map_or([0.0, 0.0], |bounds| [bounds[0], bounds[1]]),
    };
    (at, bounds.unwrap_or([at[0], at[1], at[0], at[1]]))
}

/// 符号的包围盒：块定义的范围摆到插入的位置上。挂在插入上的属性（位号标签）
/// 不算：标签的宽是估的，重号时还会变成 `？`，算进来范围就跟着显示变。
fn symbol_bounds(doc: &Document, insert: &Insert) -> Option<[f64; 4]> {
    let local = crate::app::helpers::block_definition_bounds(doc, &insert.block_name)?;
    let transform = insert.get_transform();
    let mut bbox: Option<[f64; 4]> = None;
    for x in [local.min.x, local.max.x] {
        for y in [local.min.y, local.max.y] {
            for z in [local.min.z, local.max.z] {
                let corner = transform.apply(Vector3::new(x, y, z));
                let corner = [corner.x, corner.y, corner.x, corner.y];
                bbox = Some(bbox.map_or(corner, |bbox| merge(bbox, corner)));
            }
        }
    }
    bbox
}

/// 直线的两个端点；别的实体没有。
fn line_ends(entity: &EntityType) -> Option<[[f64; 2]; 2]> {
    match entity {
        EntityType::Line(line) => Some([xy(line.start), xy(line.end)]),
        _ => None,
    }
}

fn xy(point: Vector3) -> [f64; 2] {
    [point.x, point.y]
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

/// 点到线段 `a`–`b` 的距离。
fn distance_to_wire(at: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(at, [a[0] + dx * t, a[1] + dy * t])
}

/// 两点围成的包围盒。
fn span(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[0].max(b[0]),
        a[1].max(b[1]),
    ]
}

fn merge(left: [f64; 4], right: [f64; 4]) -> [f64; 4] {
    [
        left[0].min(right[0]),
        left[1].min(right[1]),
        left[2].max(right[2]),
        left[3].max(right[3]),
    ]
}

fn compare_at(left: [f64; 2], right: [f64; 2]) -> std::cmp::Ordering {
    left[0]
        .total_cmp(&right[0])
        .then_with(|| left[1].total_cmp(&right[1]))
}

/// 归类表的结果：一个对象，或是对象以外的几样（管段、管线自己的线、
/// 信号线、连接符）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Connector,
    Segment,
    PipeLine,
    SignalLine,
    Object(ObjectKind),
}

/// 按 §3.1 归类表从上往下，第一条对上的算数。`is_break` 是拓扑表里这件
/// 管件的 `fitting_type` 是不是打断符。
fn classify(role: &str, type_code: &str, is_break: bool, types: &TypeLib) -> Class {
    match role {
        ROLE_CONNECTOR_IN | ROLE_CONNECTOR_OUT => return Class::Connector,
        ROLE_PIPE_SEGMENT => return Class::Segment,
        ROLE_PIPE_LINE => return Class::PipeLine,
        ROLE_PIPE_RUN => return Class::Object(ObjectKind::PipeRun),
        ROLE_SIGNAL_LINE => return Class::SignalLine,
        ROLE_FITTING if is_break => return Class::Object(ObjectKind::Break),
        _ => {}
    }
    let kind = if type_code == TYPE_VALVE || type_code.starts_with(VALVE_PREFIX) {
        ObjectKind::Valve
    } else if role == ROLE_FITTING {
        ObjectKind::Fitting
    } else if type_code == INSTRUMENT {
        ObjectKind::Instrument
    } else if types
        .ancestors(type_code)
        .iter()
        .any(|code| code == EQUIPMENT)
    {
        ObjectKind::Equipment
    } else {
        ObjectKind::Other
    };
    Class::Object(kind)
}

/// 类型的中文名：类型库里的名字优先，其次是几个内置类型的名字，都没有就是类型码。
fn type_label(types: &TypeLib, type_code: &str) -> String {
    if let Some(node) = types.get(type_code).filter(|node| !node.label.is_empty()) {
        return node.label.clone();
    }
    BUILTIN_LABELS
        .iter()
        .find(|(code, _)| *code == type_code)
        .map_or(type_code, |(_, label)| *label)
        .to_string()
}

/// `INSUNITS` 的值对应的长度单位。
fn length_unit(insunits: i16) -> &'static str {
    match insunits {
        1 => "in",
        2 => "ft",
        3 => "mi",
        4 => "mm",
        5 => "cm",
        6 => "m",
        7 => "km",
        8 => "uin",
        9 => "mil",
        10 => "yd",
        11 => "angstrom",
        12 => "nm",
        13 => "um",
        14 => "dm",
        15 => "dam",
        16 => "hm",
        17 => "Gm",
        18 => "au",
        19 => "ly",
        20 => "pc",
        21 => "us_ft",
        22 => "us_in",
        23 => "us_yd",
        24 => "us_mi",
        _ => "unitless",
    }
}

// ── 项目索引 ──────────────────────────────────────────────────────────────────

/// 列出项目目录 `project_dir` 里要读的图（§3.7）：直接放在目录里、扩展名是 `.dwg`
/// （不分大小写）的文件，按文件名排；不进子目录，文件名以 `.` 或 `~$` 开头的跳过。
/// 图号和 PBS 节点取项目文件 `drawings[]` 给这个文件写的（文件名不分大小写），没写
/// 图号的取文件名去掉扩展名；`drawings[]` 里有、目录里没有的文件不理。项目文件读不
/// 出来或写错、目录读不了、两个文件得出同一个图号时报错。
pub fn list_drawings(project_dir: &Path) -> Result<Vec<DrawingFile>, IndexError> {
    let listed = listed_drawings(project_dir)?;
    let entries = std::fs::read_dir(project_dir).map_err(|error| io_error(project_dir, &error))?;
    let mut drawings = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| io_error(project_dir, &error))?.path();
        let Some(file) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let is_dwg = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg"));
        if !is_dwg || file.starts_with('.') || file.starts_with("~$") || !path.is_file() {
            continue;
        }
        let given = listed.get(&file.to_lowercase());
        let drawing_id = given
            .and_then(|given| given.drawing_id.clone())
            .or_else(|| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let pbs = given.and_then(|given| given.pbs.clone());
        drawings.push(DrawingFile {
            file,
            path,
            drawing_id,
            pbs,
        });
    }
    drawings.sort_by(|left, right| left.file.cmp(&right.file));
    check_drawing_ids(
        drawings
            .iter()
            .map(|drawing| (drawing.drawing_id.as_str(), drawing.file.as_str())),
    )?;
    Ok(drawings)
}

/// 项目文件 `drawings[]` 给一个文件写的图号和 PBS 节点；空的算没写。
struct Listed {
    drawing_id: Option<String>,
    pbs: Option<String>,
}

/// 项目文件里的 `drawings[]`，按小写的文件名查。没有项目文件、没写 `drawings` 或写
/// 的是 `null` 时是空的。写错时报的错与主数据的同一套，位置从 `drawings` 起。
fn listed_drawings(project_dir: &Path) -> Result<HashMap<String, Listed>, IndexError> {
    let mut listed = HashMap::new();
    let project = master_data::project_file_on_disk(&project_dir.join(PROJECT_FILE))
        .map_err(IndexError::ProjectFile)?;
    let items = match project.as_ref().and_then(|project| project.get("drawings")) {
        None | Some(Value::Null) => return Ok(listed),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(schema_error("drawings".to_string(), "应为数组")),
    };
    for (index, item) in items.iter().enumerate() {
        let at = format!("drawings[{index}]");
        let Value::Object(item) = item else {
            return Err(schema_error(at, "应为对象"));
        };
        let file = match item.get("file") {
            Some(Value::String(file)) if !file.trim().is_empty() => file,
            Some(Value::String(_)) => {
                return Err(IndexError::ProjectFile(
                    MasterDataError::RequiredFieldMissing {
                        at: format!("{at}.file"),
                    },
                ));
            }
            Some(_) => return Err(schema_error(format!("{at}.file"), "应为文本")),
            None => return Err(schema_error(format!("{at}.file"), "缺少这个键")),
        };
        let given = Listed {
            drawing_id: optional_text(item, &at, "drawing_id")?,
            pbs: optional_text(item, &at, "pbs")?,
        };
        if listed.insert(file.to_lowercase(), given).is_some() {
            return Err(IndexError::ProjectFile(MasterDataError::Duplicate {
                at: format!("{at}.file"),
                value: file.clone(),
            }));
        }
    }
    Ok(listed)
}

/// `drawings[]` 一项里的 `key`：可以不写、写 `null` 或写空的文本，都算没写。
fn optional_text(
    item: &serde_json::Map<String, Value>,
    at: &str,
    key: &str,
) -> Result<Option<String>, IndexError> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(schema_error(format!("{at}.{key}"), "应为文本或 null")),
    }
}

fn schema_error(at: String, reason: &str) -> IndexError {
    IndexError::ProjectFile(MasterDataError::Schema {
        at,
        reason: reason.to_string(),
    })
}

fn io_error(path: &Path, error: &std::io::Error) -> IndexError {
    IndexError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

/// 两个文件得出同一个图号时报 [`IndexError::DrawingIdDuplicated`]。`drawings` 是一对对
/// 的（图号，文件名）；报出的两个文件按文件名排。
fn check_drawing_ids<'a>(
    drawings: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<(), IndexError> {
    let mut sorted: Vec<(&str, &str)> = drawings.collect();
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        let ((drawing_id, first), (other, second)) = (pair[0], pair[1]);
        if drawing_id == other {
            return Err(IndexError::DrawingIdDuplicated {
                drawing_id: drawing_id.to_string(),
                files: [first.to_string(), second.to_string()],
            });
        }
    }
    Ok(())
}

/// 把各图的摘要 `summaries` 和读不出来的图 `failed` 合成项目索引（§3.4–§3.6）。
/// 只看给的这些，不读文件。两张图（读不出来的也算）图号相同时报
/// [`IndexError::DrawingIdDuplicated`]，不出索引。
pub fn build_index(
    project_dir: &Path,
    summaries: &[DrawingSummary],
    failed: &[FailedDrawing],
) -> Result<ProjectIndex, IndexError> {
    build_index_at(project_dir, summaries, failed, unix_seconds())
}

/// 同 [`build_index`]，`generated_at` 取 `now`。
fn build_index_at(
    project_dir: &Path,
    summaries: &[DrawingSummary],
    failed: &[FailedDrawing],
    now: i64,
) -> Result<ProjectIndex, IndexError> {
    check_drawing_ids(
        summaries
            .iter()
            .map(|summary| {
                (
                    summary.drawing_id.as_str(),
                    summary.file.as_deref().unwrap_or_default(),
                )
            })
            .chain(
                failed
                    .iter()
                    .map(|drawing| (drawing.drawing_id.as_str(), drawing.file.as_str())),
            ),
    )?;
    let mut drawings: Vec<IndexDrawing> = summaries
        .iter()
        .map(|summary| IndexDrawing {
            drawing_id: summary.drawing_id.clone(),
            file: summary.file.clone(),
            source: summary.source,
            pbs: summary.pbs.clone(),
            objects: Some(summary.objects.len()),
            error: None,
        })
        .chain(failed.iter().map(|drawing| IndexDrawing {
            drawing_id: drawing.drawing_id.clone(),
            file: Some(drawing.file.clone()),
            source: SummarySource::File,
            pbs: drawing.pbs.clone(),
            objects: None,
            error: Some(drawing.error.clone()),
        }))
        .collect();
    drawings.sort_by(|left, right| left.drawing_id.cmp(&right.drawing_id));
    let (links, unresolved) = pair_connectors(summaries, &drawings);
    Ok(ProjectIndex {
        version: INDEX_VERSION,
        project_dir: project_dir.to_string_lossy().into_owned(),
        generated_at: now,
        complete: failed.is_empty(),
        drawings,
        duplicates: duplicates(summaries),
        links,
        unresolved,
    })
}

/// 跨图重号（§3.5）：设备、阀门、仪表、其他里位号非空、有自己编号的，按
/// `(type_code, tag)` 归在一起，落在两张以上的图上才算；列出它所有的出现处，
/// 同一张图上重了的也列。
fn duplicates(summaries: &[DrawingSummary]) -> Vec<Duplicate> {
    let mut seen: BTreeMap<(&str, &str), Vec<Place>> = BTreeMap::new();
    for summary in summaries {
        for object in summary
            .objects
            .iter()
            .filter(|object| counts_across_drawings(object))
        {
            seen.entry((object.type_code.as_str(), object.tag.as_str()))
                .or_default()
                .push(place_of(summary, object));
        }
    }
    seen.into_iter()
        .filter(|(_, places)| {
            let drawings: HashSet<&str> = places
                .iter()
                .map(|place| place.drawing_id.as_str())
                .collect();
            drawings.len() > 1
        })
        .map(|((type_code, tag), mut places)| {
            places.sort_by(compare_places);
            Duplicate {
                type_code: type_code.to_string(),
                tag: tag.to_string(),
                places,
            }
        })
        .collect()
}

fn counts_across_drawings(object: &SummaryObject) -> bool {
    object.own_tag
        && !object.tag.is_empty()
        && matches!(
            object.kind,
            ObjectKind::Equipment | ObjectKind::Valve | ObjectKind::Instrument | ObjectKind::Other
        )
}

fn place_of(summary: &DrawingSummary, object: &SummaryObject) -> Place {
    Place {
        drawing_id: summary.drawing_id.clone(),
        at: object.at,
        handle: object.handle.clone(),
    }
}

/// 出现处按图号、定位点（先 x 后 y）、句柄排。
pub(crate) fn compare_places(left: &Place, right: &Place) -> std::cmp::Ordering {
    left.drawing_id
        .cmp(&right.drawing_id)
        .then_with(|| compare_at(left.at, right.at))
        .then_with(|| compare_handles(&left.handle, &right.handle))
}

fn compare_ends(left: &LinkEnd, right: &LinkEnd) -> std::cmp::Ordering {
    left.drawing_id
        .cmp(&right.drawing_id)
        .then_with(|| left.line.cmp(&right.line))
        .then_with(|| compare_at(left.at, right.at))
        .then_with(|| compare_handles(&left.handle, &right.handle))
}

/// 句柄按数值比；读不出数值的排在前面，再按字面比。
fn compare_handles(left: &str, right: &str) -> std::cmp::Ordering {
    let value = |handle: &str| parse_handle(handle).map(|handle| handle.value());
    value(left).cmp(&value(right)).then_with(|| left.cmp(right))
}

/// 记着配过对的连接符到哪里找对面（§3.6）。
#[derive(Clone, Copy)]
enum Scope<'a> {
    /// 服务器配的对：只在这张图上找。
    Drawing(&'a str),
    /// 同进程配的对：对端图不可知，在其余各图上找。
    Elsewhere,
}

/// 配对时要对上的：族、是不是流入、流出侧线号、流入侧线号。
type PlugKey<'a> = (LinkFamily, bool, &'a str, &'a str);

/// 一个页间连接符和它所在的图。
struct Plug<'a> {
    drawing_id: &'a str,
    connector: &'a SummaryConnector,
    kind: Option<ConnectorKind>,
}

impl<'a> Plug<'a> {
    /// 记着配过对时到哪里找；`target`、`none` 和没配上的 `local` 不找。
    fn scope(&self) -> Option<Scope<'a>> {
        let connector = self.connector;
        match connector.peer_kind {
            PeerKind::Server => connector.peer_drawing.as_deref().map(Scope::Drawing),
            PeerKind::Local if connector.upstream.is_some() && connector.downstream.is_some() => {
                Some(Scope::Elsewhere)
            }
            _ => None,
        }
    }

    fn key(&self) -> Option<PlugKey<'a>> {
        let kind = self.kind?;
        Some((
            link_family(kind),
            kind.is_inflow(),
            self.connector.upstream.as_deref()?,
            self.connector.downstream.as_deref()?,
        ))
    }

    fn end(&self) -> LinkEnd {
        LinkEnd {
            drawing_id: self.drawing_id.to_string(),
            line: self.connector.line.clone(),
            at: self.connector.at,
            handle: self.connector.handle.clone(),
        }
    }
}

fn link_family(kind: ConnectorKind) -> LinkFamily {
    match kind {
        ConnectorKind::PipeIn | ConnectorKind::PipeOut => LinkFamily::Pipe,
        ConnectorKind::SignalIn | ConnectorKind::SignalOut => LinkFamily::Signal,
    }
}

/// 页间连接符配对（§3.6）。对面要方向相反、同族、两个线号都相同，而且自己也
/// 记着配过对。两边都只找到对方才算配成对，记一次，`from` 是流出侧；一边只
/// 找到一个、那一个却不止找到它时（同一线号同进程配对连着跨三张以上的图，
/// 待决问题 I2）两边都是 `ambiguous`。
fn pair_connectors(
    summaries: &[DrawingSummary],
    drawings: &[IndexDrawing],
) -> (Vec<Link>, Vec<Unresolved>) {
    let known: HashSet<&str> = drawings
        .iter()
        .map(|drawing| drawing.drawing_id.as_str())
        .collect();
    let plugs: Vec<Plug<'_>> = summaries
        .iter()
        .flat_map(|summary| {
            summary.connectors.iter().map(move |connector| Plug {
                drawing_id: summary.drawing_id.as_str(),
                connector,
                kind: ConnectorKind::parse(&connector.kind),
            })
        })
        .collect();
    let mut searchable: HashMap<PlugKey<'_>, Vec<usize>> = HashMap::new();
    for (index, plug) in plugs.iter().enumerate() {
        if let (Some(_), Some(key)) = (plug.scope(), plug.key()) {
            searchable.entry(key).or_default().push(index);
        }
    }
    // 每个连接符找到的对面；不找的是 `None`。
    let found: Vec<Option<Vec<usize>>> = plugs
        .iter()
        .map(|plug| {
            let scope = plug.scope()?;
            let Some((family, inflow, upstream, downstream)) = plug.key() else {
                return Some(Vec::new());
            };
            let others = searchable
                .get(&(family, !inflow, upstream, downstream))
                .map_or(&[][..], Vec::as_slice);
            Some(
                others
                    .iter()
                    .copied()
                    .filter(|&other| match scope {
                        Scope::Drawing(drawing_id) => plugs[other].drawing_id == drawing_id,
                        Scope::Elsewhere => plugs[other].drawing_id != plug.drawing_id,
                    })
                    .collect(),
            )
        })
        .collect();
    let mut links = Vec::new();
    let mut unresolved = Vec::new();
    for (index, plug) in plugs.iter().enumerate() {
        let partner = match found[index].as_deref() {
            Some(&[other]) if found[other].as_deref() == Some(&[index][..]) => Some(other),
            _ => None,
        };
        if let Some(other) = partner {
            if let Some(kind) = plug.kind.filter(|kind| !kind.is_inflow()) {
                links.push(Link {
                    family: link_family(kind),
                    from: plug.end(),
                    to: plugs[other].end(),
                });
            }
            continue;
        }
        let status = match (&found[index], plug.scope()) {
            (None, _) => LinkStatus::Unpaired,
            (_, Some(Scope::Drawing(drawing_id))) if !known.contains(drawing_id) => {
                LinkStatus::TargetMissing
            }
            (Some(others), _) if others.is_empty() => LinkStatus::OneSided,
            _ => LinkStatus::Ambiguous,
        };
        let connector = plug.connector;
        unresolved.push(Unresolved {
            drawing_id: plug.drawing_id.to_string(),
            kind: connector.kind.clone(),
            line: connector.line.clone(),
            at: connector.at,
            handle: connector.handle.clone(),
            status,
            target: connector.peer_drawing.clone(),
        });
    }
    links.sort_by(|left, right| {
        compare_ends(&left.from, &right.from).then_with(|| compare_ends(&left.to, &right.to))
    });
    unresolved.sort_by(|left, right| {
        left.drawing_id
            .cmp(&right.drawing_id)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| compare_at(left.at, right.at))
            .then_with(|| compare_handles(&left.handle, &right.handle))
    });
    (links, unresolved)
}

impl ProjectIndex {
    /// 位号 `tag` 在项目里的所有出现处：索引里读得出来的各张图上，有自己编号、
    /// 位号是它的对象；`type_code` 给了就只看这个类型。管线这样跨图同号的也列。
    /// `summaries` 是做这份索引用的摘要，索引里没有的图不算。按图号、定位点、
    /// 句柄排；空位号查不到东西。
    #[must_use]
    pub fn places(
        &self,
        summaries: &[DrawingSummary],
        type_code: Option<&str>,
        tag: &str,
    ) -> Vec<Place> {
        if tag.is_empty() {
            return Vec::new();
        }
        let indexed: HashSet<&str> = self
            .drawings
            .iter()
            .filter(|drawing| drawing.error.is_none())
            .map(|drawing| drawing.drawing_id.as_str())
            .collect();
        let mut places: Vec<Place> = summaries
            .iter()
            .filter(|summary| indexed.contains(summary.drawing_id.as_str()))
            .flat_map(|summary| {
                summary
                    .objects
                    .iter()
                    .filter(move |object| {
                        object.own_tag
                            && object.tag == tag
                            && type_code.is_none_or(|code| object.type_code == code)
                    })
                    .map(move |object| place_of(summary, object))
            })
            .collect();
        places.sort_by(compare_places);
        places
    }
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
pub(crate) use synthetic::synthetic_drawing;

/// 性能预算用的合成图（§7），检查、清单、快照、发布、面板的预算测试共用。
#[cfg(test)]
mod synthetic {
    use document::entities::{Circle, Line, Point};

    use super::*;
    use crate::pid::topology::{FittingRecord, SegmentEnd, TYPE_PIPE_SEGMENT, VALVE_PORT_NAMES};

    /// 一组里的对象数。
    const CELL: usize = 20;
    /// 一行排几组。
    const COLUMNS: usize = 10;
    /// 相邻两组的间距。一组占的范围不到 400 × 100，隔开后组与组碰不上。
    const PITCH: [f64; 2] = [500.0, 200.0];

    const VESSEL: &str = "PID_SYM_equipment_vessel";
    const PUMP: &str = "PID_SYM_equipment_pump";
    const GATE_VALVE: &str = "PID_SYM_valve_gate_valve";
    const CONTROL_VALVE: &str = "PID_SYM_valve_gate_valve_alt";
    const TRANSMITTER: &str = "PID_SYM_instrument_transmitter";
    const CONTROLLER: &str = "PID_SYM_instrument_controller";
    const INDICATOR: &str = "PID_SYM_instrument_indicator";

    /// `objects` 个对象，每 20 个是同样的一组小工艺单元，一行 10 组排开。
    /// 一组里有：
    ///
    /// - 主管线：进料容器 → 进口阀 → 泵 → 出口阀 → 不单独编号的管件 → 调节阀
    ///   → 打断符 → 受液容器，分进口、出口两条管路；管线自己的一根线从受液
    ///   容器的出口接到一个流出连接符。
    /// - 支管线：从出口管路中间三通引出，一条管路上一个排净阀，接到管线自己
    ///   的一根线，线头一个流出连接符。
    /// - 压力变送器 → 流量控制器 → 流出信号连接符两条信号线；一块温度表、一块
    ///   压力表就地安装，不接线。
    ///
    /// 管段和信号线的两头都接着东西，位号照默认规则编、不重复。实体、
    /// `PID_AUTHOR` 和拓扑表的写法与画图命令落盘的一样，只是不经命令、拓扑表
    /// 最后写一次。符号取内置符号库，做摘要时也用 [`SymbolLibrary::builtin`]。
    pub(crate) fn synthetic_drawing(objects: usize) -> Document {
        assert_eq!(
            objects % CELL,
            0,
            "合成图按每组 {CELL} 个对象生成：{objects}"
        );
        let library = SymbolLibrary::builtin();
        let mut scene = crate::scene::Scene::new();
        library.instantiate(&mut scene);
        let mut sheet = Sheet {
            doc: std::mem::replace(&mut scene.document, Document::new()),
            table: TopologyTable::default(),
            library,
            origin: [0.0, 0.0],
        };
        for cell in 0..objects / CELL {
            sheet.origin = [
                (cell % COLUMNS) as f64 * PITCH[0],
                (cell / COLUMNS) as f64 * PITCH[1],
            ];
            sheet.cell(cell);
        }
        sheet.table.store(&mut sheet.doc);
        sheet.doc
    }

    struct Sheet {
        doc: Document,
        table: TopologyTable,
        library: SymbolLibrary,
        origin: [f64; 2],
    }

    /// 建好的一条管路：位号、本体的句柄，和它的管段都带的属性。
    struct Run {
        tag: String,
        owner: String,
        props: BTreeMap<String, String>,
    }

    /// 插好了、两个连接点还没记的管件。
    struct Placed {
        handle: Handle,
        record: FittingRecord,
    }

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    /// 位号自动编、状态正常的一条记录。
    fn author_of(type_code: &str, tag: String, props: &BTreeMap<String, String>) -> PidAuthor {
        PidAuthor {
            type_code: type_code.to_string(),
            tag,
            tag_source: "auto".to_string(),
            tag_status: "ok".to_string(),
            props: serde_json::to_string(props).expect("属性 JSON"),
            ..PidAuthor::default()
        }
    }

    impl Sheet {
        fn at(&self, x: f64, y: f64) -> [f64; 2] {
            [self.origin[0] + x, self.origin[1] + y]
        }

        /// 第 `n` 组。位号：设备 `P-`、阀门 `V-`、管线 `L-` 从 1001 起、信号线 `S-` 从
        /// 2001 起接着前一组编，仪表是功能字母加 `1001 + n`。
        fn cell(&mut self, n: usize) {
            let equipment = |i: usize| format!("P-{}", 1001 + 3 * n + i);
            let valve = |i: usize| format!("V-{}", 1001 + 4 * n + i);
            let signal = |i: usize| format!("S-{}", 2001 + 2 * n + i);
            let main = format!("L-{}", 1001 + 2 * n);
            let branch = format!("L-{}", 1002 + 2 * n);

            let vessel = props(&[
                ("设计压力", "0.6MPa"),
                ("设计温度", "60"),
                ("材质", "S30408"),
            ]);
            let pump = props(&[("流量", "20m3/h"), ("扬程", "40m"), ("材质", "S30408")]);
            self.symbol(
                VESSEL,
                self.at(0.0, -12.0),
                author_of(EQUIPMENT, equipment(0), &vessel),
            );
            self.symbol(
                PUMP,
                self.at(60.0, 0.0),
                author_of(EQUIPMENT, equipment(1), &pump),
            );
            self.symbol(
                VESSEL,
                self.at(320.0, 12.0),
                author_of(EQUIPMENT, equipment(2), &vessel),
            );

            // 主管线。进料容器的出口在 (0, 0)，泵的进出口在 (46, 0) / (74, 0)，
            // 受液容器的进出口在 (320, 0) / (320, 24)。阀门两侧的管段收在落点 ±16
            // 处（端口 14 加 `PIDFITTING` 让开的 2），管件 ±10，打断符的缝 ±2。
            let fields = props(&[
                ("管道等级", "A1A"),
                ("公称直径", "80"),
                ("绝热等级", "N"),
                ("流体介质", "PW"),
                ("管线类型", "P"),
            ]);
            let line = self.pipe_line(&main, [self.at(320.0, 24.0), self.at(380.0, 24.0)], &fields);
            self.connector(ConnectorKind::PipeOut, &main, self.at(380.0, 24.0));

            let inlet = self.run(&main, line, self.at(0.0, 0.0), &fields);
            let valve_in = self.valve(&inlet, GATE_VALVE, self.at(23.0, 0.0), valve(0));
            let first = self.segment(
                &inlet,
                self.at(0.0, 0.0),
                self.at(7.0, 0.0),
                None,
                Some(valve_in.handle),
            );
            let second = self.segment(
                &inlet,
                self.at(39.0, 0.0),
                self.at(46.0, 0.0),
                Some(valve_in.handle),
                None,
            );
            self.bind(valve_in, [first, second]);

            let outlet = self.run(&main, line, self.at(74.0, 0.0), &fields);
            let valve_out = self.valve(&outlet, GATE_VALVE, self.at(110.0, 0.0), valve(1));
            let fitting = self.fitting(&outlet, TRANSMITTER, self.at(160.0, 0.0));
            let control = self.valve(&outlet, CONTROL_VALVE, self.at(210.0, 0.0), valve(2));
            self.mark(&outlet, self.at(270.0, 0.0));
            let pieces = [
                self.segment(
                    &outlet,
                    self.at(74.0, 0.0),
                    self.at(94.0, 0.0),
                    None,
                    Some(valve_out.handle),
                ),
                self.segment(
                    &outlet,
                    self.at(126.0, 0.0),
                    self.at(150.0, 0.0),
                    Some(valve_out.handle),
                    Some(fitting.handle),
                ),
                self.segment(
                    &outlet,
                    self.at(170.0, 0.0),
                    self.at(194.0, 0.0),
                    Some(fitting.handle),
                    Some(control.handle),
                ),
                self.segment(
                    &outlet,
                    self.at(226.0, 0.0),
                    self.at(268.0, 0.0),
                    Some(control.handle),
                    None,
                ),
                self.segment(
                    &outlet,
                    self.at(272.0, 0.0),
                    self.at(320.0, 0.0),
                    None,
                    None,
                ),
            ];
            self.bind(valve_out, [pieces[0], pieces[1]]);
            self.bind(fitting, [pieces[1], pieces[2]]);
            self.bind(control, [pieces[2], pieces[3]]);

            // 支管线：从出口管路第二段中间的 (140, 0) 引出，拐到 y = -40，经排净阀
            // 接到管线自己的一根线。
            let fields = props(&[
                ("管道等级", "B2C"),
                ("公称直径", "25"),
                ("绝热等级", "N"),
                ("流体介质", "PW"),
                ("管线类型", "D"),
            ]);
            let line = self.pipe_line(
                &branch,
                [self.at(200.0, -40.0), self.at(260.0, -40.0)],
                &fields,
            );
            self.connector(ConnectorKind::PipeOut, &branch, self.at(260.0, -40.0));
            let drain = self.run(&branch, line, self.at(140.0, 0.0), &fields);
            let valve_drain = self.valve(&drain, GATE_VALVE, self.at(170.0, -40.0), valve(3));
            self.segment(
                &drain,
                self.at(140.0, 0.0),
                self.at(140.0, -40.0),
                None,
                None,
            );
            let before = self.segment(
                &drain,
                self.at(140.0, -40.0),
                self.at(154.0, -40.0),
                None,
                Some(valve_drain.handle),
            );
            let after = self.segment(
                &drain,
                self.at(186.0, -40.0),
                self.at(200.0, -40.0),
                Some(valve_drain.handle),
                None,
            );
            self.bind(valve_drain, [before, after]);

            for (block, function, [x, y], range) in [
                (TRANSMITTER, "PT", [160.0, 40.0], "0-1.6MPa"),
                (CONTROLLER, "FIC", [210.0, 40.0], "0-30m3/h"),
                (INDICATOR, "TI", [100.0, 40.0], "0-150℃"),
                (INDICATOR, "PI", [60.0, 40.0], "0-1.6MPa"),
            ] {
                let fields = props(&[("仪表功能", function), ("测量范围", range)]);
                let tag = format!("{function}-{}", 1001 + n);
                self.symbol(block, self.at(x, y), author_of(INSTRUMENT, tag, &fields));
            }
            self.signal(signal(0), self.at(168.0, 40.0), self.at(202.0, 40.0));
            self.signal(signal(1), self.at(218.0, 40.0), self.at(250.0, 40.0));
            self.connector(ConnectorKind::SignalOut, &signal(1), self.at(250.0, 40.0));
        }

        fn add(&mut self, entity: EntityType, author: PidAuthor) -> Handle {
            let handle = self.doc.add_entity(entity).expect("合成图的实体");
            author.store(&mut self.doc, handle);
            handle
        }

        fn line(&mut self, a: [f64; 2], b: [f64; 2], author: PidAuthor) -> Handle {
            let line =
                Line::from_points(Vector3::new(a[0], a[1], 0.0), Vector3::new(b[0], b[1], 0.0));
            self.add(EntityType::Line(line), author)
        }

        /// 一个符号实例，不转、不缩放（`PIDINSERT` / `PIDFITTING`）。
        fn symbol(&mut self, block: &str, [x, y]: [f64; 2], author: PidAuthor) -> Handle {
            let ports = self.library.symbol_ports(block).join(",");
            let insert = Insert::new(block, Vector3::new(x, y, 0.0));
            self.add(EntityType::Insert(insert), PidAuthor { ports, ..author })
        }

        /// 一条管线和它自己的一根线（`PIDPIPE`，再用 `PIDSETPROP` 填下传的五项）。
        fn pipe_line(
            &mut self,
            tag: &str,
            [a, b]: [[f64; 2]; 2],
            fields: &BTreeMap<String, String>,
        ) -> Handle {
            let author = PidAuthor {
                topology: ROLE_PIPE_LINE.to_string(),
                ..author_of(TYPE_PIPE_LINE, tag.to_string(), fields)
            };
            let handle = self.line(a, b, author);
            self.table.append_line(tag, handle, fields.clone());
            handle
        }

        /// 一条管路（`PIDPIPE run`）：本体是起点上的一个点，属性是管线号加上从
        /// 管线继承的五项。管段另画。
        fn run(
            &mut self,
            line: &str,
            owner: Handle,
            start: [f64; 2],
            fields: &BTreeMap<String, String>,
        ) -> Run {
            let record = self.table.line_by_tag_mut(line).expect("先有管线");
            let tag = format!("{line}-{:02}", record.next_run_ordinal);
            record.next_run_ordinal += 1;
            let mut props = fields.clone();
            props.insert("管线号".to_string(), line.to_string());
            let author = PidAuthor {
                topology: ROLE_PIPE_RUN.to_string(),
                owner: handle_text(owner),
                ..author_of(TYPE_PIPE_RUN, tag.clone(), &props)
            };
            let point = Point::at(Vector3::new(start[0], start[1], 0.0));
            let handle = self.add(EntityType::Point(point), author);
            self.table
                .append_run(line, &tag, handle, props.clone(), Vec::new());
            Run {
                tag,
                owner: handle_text(handle),
                props,
            }
        }

        /// 管路上的一段。`from` 是起点贴着的管件（接它的连接点 2），`to` 是终点
        /// 贴着的管件（接它的连接点 1），`PIDFITTING` 拆段后就是这样记的。
        fn segment(
            &mut self,
            run: &Run,
            a: [f64; 2],
            b: [f64; 2],
            from: Option<Handle>,
            to: Option<Handle>,
        ) -> Handle {
            let author = PidAuthor {
                topology: ROLE_PIPE_SEGMENT.to_string(),
                owner: run.owner.clone(),
                ..author_of(TYPE_PIPE_SEGMENT, String::new(), &run.props)
            };
            let handle = self.line(a, b, author);
            let end = |point: [f64; 2], fitting: Option<Handle>, port: &str| match fitting {
                Some(fitting) => SegmentEnd::joined(point, fitting, port),
                None => SegmentEnd::open(point),
            };
            let start = end(a, from, VALVE_PORT_NAMES[1]);
            let finish = end(b, to, VALVE_PORT_NAMES[0]);
            self.table.append_segment(&run.tag, handle, start, finish);
            handle
        }

        /// 管路上的一个阀门，单独编号。
        fn valve(&mut self, run: &Run, block: &str, at: [f64; 2], tag: String) -> Placed {
            let size = run.props.get("公称直径").cloned().unwrap_or_default();
            let fields = props(&[("公称直径", size.as_str()), ("压力等级", "PN16")]);
            let author = author_of(TYPE_VALVE, tag.clone(), &fields);
            self.place(run, block, at, author, Some(tag))
        }

        /// 管路上一个不单独编号的管件：记的是所在管路的位号。
        fn fitting(&mut self, run: &Run, block: &str, at: [f64; 2]) -> Placed {
            let author = author_of(TYPE_FITTING, run.tag.clone(), &BTreeMap::new());
            self.place(run, block, at, author, None)
        }

        fn place(
            &mut self,
            run: &Run,
            block: &str,
            at: [f64; 2],
            author: PidAuthor,
            tag: Option<String>,
        ) -> Placed {
            let author = PidAuthor {
                topology: ROLE_FITTING.to_string(),
                owner: run.owner.clone(),
                ..author
            };
            let handle = self.symbol(block, at, author);
            let record = FittingRecord {
                handle: handle_text(handle),
                run_tag: run.tag.clone(),
                fitting_type: block.to_string(),
                tag,
                ports: BTreeMap::new(),
            };
            Placed { handle, record }
        }

        /// 记下管件两个连接点各接哪一段。
        fn bind(&mut self, placed: Placed, [before, after]: [Handle; 2]) {
            let mut record = placed.record;
            record.ports = [(VALVE_PORT_NAMES[0], before), (VALVE_PORT_NAMES[1], after)]
                .into_iter()
                .map(|(port, segment)| (port.to_string(), Some(handle_text(segment))))
                .collect();
            self.table.fittings.push(record);
        }

        /// 管路上的打断符（`PIDBREAK`）：管路沿 x 轴走，缝两侧各让开 2，斜线画在
        /// 缝里；拓扑表里一条不接任何管段的管件记录。
        fn mark(&mut self, run: &Run, [x, y]: [f64; 2]) {
            let author = PidAuthor {
                topology: ROLE_FITTING.to_string(),
                owner: run.owner.clone(),
                ..author_of(TYPE_FITTING, String::new(), &BTreeMap::new())
            };
            let handle = self.line([x - 1.2, y + 2.0], [x + 1.2, y - 2.0], author);
            self.table.fittings.push(FittingRecord {
                handle: handle_text(handle),
                run_tag: run.tag.clone(),
                fitting_type: FITTING_BREAK.to_string(),
                tag: None,
                ports: BTreeMap::new(),
            });
        }

        /// 线头上的一个连接符（`PIDCONNECTOR`），还没配对。
        fn connector(&mut self, kind: ConnectorKind, line: &str, [x, y]: [f64; 2]) {
            let mut circle = Circle::new();
            circle.center = Vector3::new(x, y, 0.0);
            circle.radius = 3.0;
            let author = PidAuthor {
                topology: kind.role().to_string(),
                ..author_of(kind.type_code(), String::new(), &BTreeMap::new())
            };
            let handle = self.add(EntityType::Circle(circle), author);
            let mut record = ConnectorRecord::new(handle_text(handle), kind, line.to_string());
            record.upstream = Some(line.to_string());
            self.table.connectors.push(record);
        }

        /// 一条一段的信号线（`PIDPIPE signal`）。
        fn signal(&mut self, tag: String, a: [f64; 2], b: [f64; 2]) {
            let author = PidAuthor {
                topology: ROLE_SIGNAL_LINE.to_string(),
                ..author_of(TYPE_SIGNAL_LINE, tag, &BTreeMap::new())
            };
            self.line(a, b, author);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenCADStudio;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const PUMP: &str = "PID_SYM_equipment_pump";
    const VESSEL: &str = "PID_SYM_equipment_vessel";
    const GATE_VALVE: &str = "PID_SYM_valve_gate_valve";
    const TRANSMITTER: &str = "PID_SYM_instrument_transmitter";

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    /// 协议 1 的一条控制请求，发给当前的图。
    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        req["request_id"] = json!(format!("ix1-{}", SERIAL.fetch_add(1, Ordering::Relaxed)));
        app.automation_op(&req.to_string())
    }

    fn run(app: &mut OpenCADStudio, cmd: &str) {
        let done = request(app, json!({"op": "run", "cmd": cmd}));
        assert_eq!(done["ok"], true, "{cmd}：{done}");
    }

    /// 起 `cmd`，依次喂点，最后回车。
    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
        run(app, cmd);
        for [x, y] in points {
            let point =
                json!({"op": "input", "kind": "point", "point": [x, y, 0.0], "space": "wcs"});
            let fed = request(app, point);
            assert_eq!(fed["ok"], true, "{cmd} 喂点：{fed}");
        }
        request(app, json!({"op": "input", "kind": "enter"}));
    }

    fn record(doc: &Document, handle: Handle) -> PidAuthor {
        PidAuthor::load(doc, handle).expect("P&ID 记录")
    }

    fn block_of(entity: &EntityType) -> Option<&str> {
        match entity {
            EntityType::Insert(insert) => Some(insert.block_name.as_str()),
            _ => None,
        }
    }

    /// 图上带记录、`pick` 认得的唯一那个实体。
    fn only(doc: &Document, pick: impl Fn(&EntityType, &PidAuthor) -> bool) -> Handle {
        let found: Vec<Handle> = doc
            .entities()
            .filter_map(|entity| {
                let handle = entity.common().handle;
                let author = PidAuthor::load(doc, handle)?;
                pick(entity, &author).then_some(handle)
            })
            .collect();
        assert_eq!(found.len(), 1, "应恰好一个：{found:?}");
        found[0]
    }

    fn ends_of(doc: &Document, handle: Handle) -> ([f64; 2], [f64; 2]) {
        doc.entities()
            .find_map(|entity| match entity {
                EntityType::Line(line) if line.common.handle == handle => {
                    Some(([line.start.x, line.start.y], [line.end.x, line.end.y]))
                }
                _ => None,
            })
            .expect("一根线")
    }

    /// 小图（§7）：一台泵左边接一个不在线上的阀门，一台容器；管线 L-1001
    /// 自己一根线，线头上一个流出连接符；管路 L-1001-01 从泵的出口到容器的
    /// 入口，上面一个阀门、一个不单独编号的管件（变送器符号）、一个打断符；
    /// 信号线 S-2001 两根线接到一台仪表的入口；还有一根挂了别的类型的线。
    fn small_drawing(app: &mut OpenCADStudio) {
        draw(app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(app, "PIDINSERT gate_valve", &[[-28.0, 0.0]]);
        draw(app, "PIDINSERT vessel", &[[200.0, 12.0]]);
        draw(app, "PIDPIPE", &[[200.0, 100.0], [300.0, 100.0]]);
        draw(app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
        draw(app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
        draw(app, "PIDFITTING transmitter", &[[40.0, 0.0]]);
        draw(app, "PIDBREAK", &[[150.0, 0.0]]);
        draw(
            app,
            "PIDPIPE signal",
            &[[0.0, 50.0], [50.0, 50.0], [50.0, 80.0]],
        );
        draw(app, "PIDINSERT transmitter", &[[58.0, 80.0]]);
        draw(app, "PIDCONNECTOR pipe_out", &[[300.0, 100.0]]);
        run(app, "PIDSETPROP L-1001 管道等级 A1A");
        run(app, "PIDSETPROP L-1001-01 公称直径 50");
        let pump = only(app.active_document_for_test(), |entity, _| {
            block_of(entity) == Some(PUMP)
        });
        let tag = record(app.active_document_for_test(), pump).tag;
        run(app, &format!("PIDSETPROP {tag} 设计温度 80"));
        draw(app, "LINE", &[[400.0, 0.0], [450.0, 0.0]]);
        let doc = app.active_document_for_test();
        let note = doc
            .entities()
            .filter(|entity| matches!(entity, EntityType::Line(_)))
            .map(|entity| entity.common().handle)
            .filter(|handle| PidAuthor::load(doc, *handle).is_none())
            .max_by_key(|handle| handle.value())
            .expect("刚画的线");
        let data: Vec<Value> = [
            "type=NOTE",
            "tag=N-1",
            "tag_source=manual",
            "tag_status=ok",
            r#"props={"说明":"图例"}"#,
        ]
        .iter()
        .map(|value| json!({"code": 1000, "value": value}))
        .collect();
        let handles = [handle_text(note)];
        let set = request(
            app,
            json!({"op": "xdata_set", "handles": handles, "app": "PID_AUTHOR", "data": data}),
        );
        assert_eq!(set["ok"], true, "{set}");
    }

    fn meta() -> SummaryMeta {
        SummaryMeta {
            drawing_id: "PID-1101".to_string(),
            file: Some("PID-1101.dwg".to_string()),
            pbs: Some("1001-01".to_string()),
            source: SummarySource::File,
            fingerprint: Some(Fingerprint {
                size: 482_133,
                modified_at: 1_794_800_000,
            }),
        }
    }

    fn summary_of(app: &OpenCADStudio) -> DrawingSummary {
        summarize(
            app.active_document_for_test(),
            &app.pid_symbols,
            meta(),
            DEFAULT_TOLERANCE,
        )
    }

    fn found(summary: &DrawingSummary, handle: Handle) -> &SummaryObject {
        let text = handle_text(handle);
        summary
            .objects
            .iter()
            .find(|object| object.handle == text)
            .unwrap_or_else(|| panic!("摘要里没有 {text}：{:#?}", summary.objects))
    }

    /// 期望的对象：记录原样的四项照抄（类型码先对一下），其余先是没有符号、
    /// 没有属性、不归谁的样子，由调用处改。
    fn plain(
        doc: &Document,
        handle: Handle,
        kind: ObjectKind,
        type_code: &str,
        type_label: &str,
    ) -> SummaryObject {
        let author = record(doc, handle);
        assert_eq!(author.type_code, type_code, "{}", handle_text(handle));
        SummaryObject {
            kind,
            type_code: author.type_code,
            type_label: type_label.to_string(),
            tag: author.tag,
            own_tag: true,
            tag_source: author.tag_source,
            tag_status: author.tag_status,
            symbol: None,
            symbol_known: true,
            props: BTreeMap::new(),
            overrides: Vec::new(),
            owner: None,
            layer: "0".to_string(),
            at: [0.0, 0.0],
            bbox: [0.0; 4],
            handle: handle_text(handle),
        }
    }

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn port(
        owner: &SummaryObject,
        name: &str,
        direction: PortDir,
        at: [f64; 2],
        joined: bool,
    ) -> SummaryPort {
        SummaryPort {
            owner_kind: owner.kind,
            type_code: owner.type_code.clone(),
            tag: owner.tag.clone(),
            port: name.to_string(),
            direction: Some(direction),
            at,
            joined,
        }
    }

    #[test]
    fn a_small_drawing_gives_every_object_its_fields() {
        let mut app = app();
        small_drawing(&mut app);
        let summary = summary_of(&app);
        let doc = app.active_document_for_test();
        let table = TopologyTable::load(doc);

        let text = serde_json::to_string(&summary).expect("JSON");
        let head = format!(
            r#"{{"version":1,"drawing_id":"PID-1101","file":"PID-1101.dwg","source":"file","fingerprint":{{"size":482133,"modified_at":1794800000}},"pbs":"1001-01","length_unit":"{}","objects":[{{"#,
            length_unit(doc.header.insertion_units)
        );
        assert!(text.starts_with(&head), "{text}");
        assert!(text.contains(r#"}],"ports":[{"owner_kind":"#), "{text}");

        let pump = only(doc, |entity, _| block_of(entity) == Some(PUMP));
        assert!(record(doc, pump).tag.starts_with("P-"));
        let want = SummaryObject {
            symbol: Some(PUMP.to_string()),
            props: props(&[("设计温度", "80")]),
            at: [0.0, 0.0],
            bbox: [-14.0, -8.0, 14.0, 8.0],
            ..plain(doc, pump, ObjectKind::Equipment, "EQUIPMENT", "设备")
        };
        assert_eq!(found(&summary, pump), &want);

        let loose = only(doc, |entity, author| {
            block_of(entity) == Some(GATE_VALVE) && author.topology.is_empty()
        });
        let want = SummaryObject {
            symbol: Some(GATE_VALVE.to_string()),
            at: [-28.0, 0.0],
            bbox: [-42.0, -6.0, -14.0, 6.0],
            ..plain(doc, loose, ObjectKind::Valve, "VALVE", "阀门")
        };
        assert_eq!(found(&summary, loose), &want);

        let vessel = only(doc, |entity, _| block_of(entity) == Some(VESSEL));
        let want = SummaryObject {
            symbol: Some(VESSEL.to_string()),
            at: [200.0, 12.0],
            bbox: [192.0, 0.0, 208.0, 24.0],
            ..plain(doc, vessel, ObjectKind::Equipment, "EQUIPMENT", "设备")
        };
        assert_eq!(found(&summary, vessel), &want);

        let line = table.line_by_tag("L-1001").expect("管线 L-1001");
        let line = parse_handle(&line.handle).expect("句柄");
        let want = SummaryObject {
            props: props(&[("管道等级", "A1A")]),
            at: [200.0, 100.0],
            bbox: [200.0, 100.0, 300.0, 100.0],
            ..plain(doc, line, ObjectKind::PipeLine, "PIPE_LINE", "管线")
        };
        assert_eq!(found(&summary, line), &want);

        let run = table
            .pipe_runs
            .iter()
            .find(|run| run.tag == "L-1001-01")
            .expect("管路 L-1001-01");
        let run = parse_handle(&run.handle).expect("句柄");
        let want = SummaryObject {
            props: props(&[
                ("管线号", "L-1001"),
                ("管道等级", "A1A"),
                ("公称直径", "50"),
            ]),
            overrides: vec!["公称直径".to_string()],
            owner: Some("L-1001".to_string()),
            at: [14.0, 0.0],
            bbox: [14.0, 0.0, 200.0, 0.0],
            ..plain(doc, run, ObjectKind::PipeRun, "PIPE_RUN", "管路")
        };
        assert_eq!(found(&summary, run), &want);
        let json = serde_json::to_string(found(&summary, run)).expect("JSON");
        assert!(
            json.starts_with(r#"{"kind":"管路","type_code":"PIPE_RUN","type_label":"管路","tag":"L-1001-01","own_tag":true,"tag_source":"#),
            "{json}"
        );
        let order: Vec<usize> = [
            "kind",
            "type_code",
            "type_label",
            "tag",
            "own_tag",
            "tag_source",
            "tag_status",
            "symbol",
            "symbol_known",
            "props",
            "overrides",
            "owner",
            "layer",
            "at",
            "bbox",
            "handle",
        ]
        .iter()
        .map(|key| {
            json.find(&format!("\"{key}\":"))
                .unwrap_or_else(|| panic!("缺 {key}：{json}"))
        })
        .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "字段顺序：{json}"
        );

        let valve = only(doc, |entity, author| {
            block_of(entity) == Some(GATE_VALVE) && author.topology == ROLE_FITTING
        });
        let want = SummaryObject {
            symbol: Some(GATE_VALVE.to_string()),
            owner: Some("L-1001-01".to_string()),
            at: [100.0, 0.0],
            bbox: [86.0, -6.0, 114.0, 6.0],
            ..plain(doc, valve, ObjectKind::Valve, "VALVE", "阀门")
        };
        assert_eq!(found(&summary, valve), &want);

        let fitting = only(doc, |entity, author| {
            block_of(entity) == Some(TRANSMITTER) && author.topology == ROLE_FITTING
        });
        assert_eq!(
            record(doc, fitting).tag,
            "L-1001-01",
            "不单独编号的管件记管路的位号"
        );
        let want = SummaryObject {
            own_tag: false,
            symbol: Some(TRANSMITTER.to_string()),
            // `PIDSETPROP` 按位号找对象，管件带着管路的位号，一起被写上。
            props: props(&[("公称直径", "50")]),
            owner: Some("L-1001-01".to_string()),
            at: [40.0, 0.0],
            bbox: [32.0, -8.0, 48.0, 8.0],
            ..plain(doc, fitting, ObjectKind::Fitting, "FITTING", "管件")
        };
        assert_eq!(found(&summary, fitting), &want);

        let mark = only(doc, |entity, author| {
            author.topology == ROLE_FITTING && matches!(entity, EntityType::Line(_))
        });
        let (a, b) = ends_of(doc, mark);
        let want = SummaryObject {
            own_tag: false,
            owner: Some("L-1001-01".to_string()),
            at: a,
            bbox: [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ],
            ..plain(doc, mark, ObjectKind::Break, "FITTING", "管件")
        };
        assert_eq!(found(&summary, mark), &want);

        let mut signal: Vec<Handle> = doc
            .entities()
            .map(|entity| entity.common().handle)
            .filter(|handle| {
                PidAuthor::load(doc, *handle)
                    .is_some_and(|author| author.topology == ROLE_SIGNAL_LINE)
            })
            .collect();
        signal.sort_by_key(|handle| handle.value());
        assert_eq!(signal.len(), 2, "信号线画了两根");
        assert_eq!(record(doc, signal[0]).tag, "S-2001");
        let want = SummaryObject {
            at: [0.0, 50.0],
            bbox: [0.0, 50.0, 50.0, 80.0],
            ..plain(
                doc,
                signal[0],
                ObjectKind::SignalLine,
                "SIGNAL_LINE",
                "信号线",
            )
        };
        assert_eq!(found(&summary, signal[0]), &want);

        let instrument = only(doc, |entity, author| {
            block_of(entity) == Some(TRANSMITTER) && author.topology.is_empty()
        });
        let want = SummaryObject {
            symbol: Some(TRANSMITTER.to_string()),
            at: [58.0, 80.0],
            bbox: [50.0, 72.0, 66.0, 88.0],
            ..plain(
                doc,
                instrument,
                ObjectKind::Instrument,
                "INSTRUMENT",
                "仪表",
            )
        };
        assert_eq!(found(&summary, instrument), &want);

        let note = only(doc, |_, author| author.type_code == "NOTE");
        let want = SummaryObject {
            props: props(&[("说明", "图例")]),
            at: [400.0, 0.0],
            bbox: [400.0, 0.0, 450.0, 0.0],
            ..plain(doc, note, ObjectKind::Other, "NOTE", "NOTE")
        };
        assert_eq!(found(&summary, note), &want);

        let connector = only(doc, |_, author| author.topology == ROLE_CONNECTOR_OUT);
        assert_eq!(
            record(doc, connector).type_code,
            "PIPE_LINE",
            "连接符的类型码是管线的"
        );
        let connector = handle_text(connector);
        assert!(
            summary
                .objects
                .iter()
                .all(|object| object.handle != connector),
            "连接符不是对象：{:#?}",
            summary.objects
        );
        assert_eq!(summary.objects.len(), 11, "{:#?}", summary.objects);
        let kinds: Vec<ObjectKind> = summary.objects.iter().map(|object| object.kind).collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == ObjectKind::PipeLine)
                .count(),
            1,
            "连接符不算成管线"
        );
        let keys: Vec<(&str, &str)> = summary
            .objects
            .iter()
            .map(|object| (object.type_code.as_str(), object.tag.as_str()))
            .collect();
        assert!(
            keys.windows(2).all(|pair| pair[0] < pair[1]),
            "按类型码、位号排：{keys:?}"
        );

        let ports = [
            port(
                found(&summary, pump),
                "in1",
                PortDir::In,
                [-14.0, 0.0],
                true,
            ),
            port(
                found(&summary, pump),
                "out1",
                PortDir::Out,
                [14.0, 0.0],
                true,
            ),
            port(
                found(&summary, loose),
                "in1",
                PortDir::In,
                [-42.0, 0.0],
                false,
            ),
            port(
                found(&summary, loose),
                "out1",
                PortDir::Out,
                [-14.0, 0.0],
                true,
            ),
            port(
                found(&summary, vessel),
                "in1",
                PortDir::In,
                [200.0, 0.0],
                true,
            ),
            port(
                found(&summary, vessel),
                "out1",
                PortDir::Out,
                [200.0, 24.0],
                false,
            ),
            port(
                found(&summary, valve),
                "in1",
                PortDir::In,
                [86.0, 0.0],
                true,
            ),
            port(
                found(&summary, valve),
                "out1",
                PortDir::Out,
                [114.0, 0.0],
                true,
            ),
            port(
                found(&summary, fitting),
                "in1",
                PortDir::In,
                [32.0, 0.0],
                true,
            ),
            port(
                found(&summary, fitting),
                "out1",
                PortDir::Out,
                [48.0, 0.0],
                true,
            ),
            port(
                found(&summary, instrument),
                "in1",
                PortDir::In,
                [50.0, 80.0],
                true,
            ),
            port(
                found(&summary, instrument),
                "out1",
                PortDir::Out,
                [66.0, 80.0],
                false,
            ),
        ];
        assert_eq!(summary.ports.len(), ports.len(), "{:#?}", summary.ports);
        for want in &ports {
            assert!(
                summary.ports.contains(want),
                "缺 {want:?}：{:#?}",
                summary.ports
            );
        }
        let order: Vec<(&str, [f64; 2])> = summary
            .ports
            .iter()
            .map(|port| (port.tag.as_str(), port.at))
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] <= pair[1]),
            "端口按位号、坐标排：{order:?}"
        );
    }

    #[test]
    fn props_past_the_record_limit_come_back_whole() {
        let mut app = app();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        let pump = only(app.active_document_for_test(), |entity, _| {
            block_of(entity) == Some(PUMP)
        });
        let tag = record(app.active_document_for_test(), pump).tag;
        let long = "A".repeat(3000);
        run(&mut app, &format!("PIDSETPROP {tag} 设计温度 {long}"));
        let doc = app.active_document_for_test();
        let entity = doc
            .entities()
            .find(|entity| entity.common().handle == pump)
            .expect("泵");
        assert!(!PidAuthor::props_are_inline(entity), "属性应已挪进 XRecord");
        let summary = summary_of(&app);
        assert_eq!(found(&summary, pump).props.get("设计温度"), Some(&long));
    }

    #[test]
    fn the_same_drawing_summarizes_to_the_same_bytes_and_is_left_alone() {
        let mut app = app();
        small_drawing(&mut app);
        let doc = app.active_document_for_test();
        let before = format!("{doc:?}");
        let first = summarize(doc, &app.pid_symbols, meta(), DEFAULT_TOLERANCE);
        let second = summarize(doc, &app.pid_symbols, meta(), DEFAULT_TOLERANCE);
        let first = serde_json::to_string(&first).expect("JSON");
        assert_eq!(first, serde_json::to_string(&second).expect("JSON"));
        assert!(first.contains(r#""objects":[{"#), "摘要不该是空的：{first}");
        assert_eq!(format!("{doc:?}"), before, "摘要不改图");
    }

    #[test]
    fn the_classification_table_reads_top_down() {
        let types = TypeLib::embedded();
        let rows = [
            (ROLE_CONNECTOR_IN, "PIPE_LINE", false, Class::Connector),
            (ROLE_CONNECTOR_OUT, "SIGNAL_LINE", false, Class::Connector),
            (ROLE_PIPE_SEGMENT, "PIPE_SEGMENT", false, Class::Segment),
            (ROLE_PIPE_LINE, "PIPE_LINE", false, Class::PipeLine),
            (
                ROLE_PIPE_RUN,
                "PIPE_RUN",
                false,
                Class::Object(ObjectKind::PipeRun),
            ),
            (ROLE_SIGNAL_LINE, "SIGNAL_LINE", false, Class::SignalLine),
            (
                ROLE_FITTING,
                "FITTING",
                true,
                Class::Object(ObjectKind::Break),
            ),
            (
                ROLE_FITTING,
                "VALVE",
                true,
                Class::Object(ObjectKind::Break),
            ),
            (
                ROLE_FITTING,
                "VALVE",
                false,
                Class::Object(ObjectKind::Valve),
            ),
            ("", "VALVE", false, Class::Object(ObjectKind::Valve)),
            ("", "VALVE_GATE", false, Class::Object(ObjectKind::Valve)),
            (
                ROLE_FITTING,
                "FITTING",
                false,
                Class::Object(ObjectKind::Fitting),
            ),
            (
                ROLE_FITTING,
                "INSTRUMENT",
                false,
                Class::Object(ObjectKind::Fitting),
            ),
            (
                "",
                "INSTRUMENT",
                false,
                Class::Object(ObjectKind::Instrument),
            ),
            ("", "EQUIPMENT", false, Class::Object(ObjectKind::Equipment)),
            ("", "PUMP", false, Class::Object(ObjectKind::Equipment)),
            ("", "TANK", false, Class::Object(ObjectKind::Equipment)),
            ("", "VALVES", false, Class::Object(ObjectKind::Other)),
            ("", "NOTE", false, Class::Object(ObjectKind::Other)),
            ("", "", false, Class::Object(ObjectKind::Other)),
        ];
        for (role, type_code, is_break, want) in rows {
            assert_eq!(
                classify(role, type_code, is_break, &types),
                want,
                "{role:?} {type_code:?} {is_break}"
            );
        }
        assert!(!ObjectKind::Fitting.has_own_tag());
        assert!(!ObjectKind::Break.has_own_tag());
        assert!(ObjectKind::Valve.has_own_tag());
    }

    #[test]
    fn type_labels_come_from_the_library_then_the_builtin_names() {
        let types = TypeLib::embedded();
        for (code, label) in [
            ("EQUIPMENT", "设备"),
            ("PUMP", "泵"),
            ("TANK", "储罐"),
            ("VALVE", "阀门"),
            ("VALVE_GATE", "闸阀"),
            ("INSTRUMENT", "仪表"),
            ("PIPE_LINE", "管线"),
            ("PIPE_RUN", "管路"),
            ("SIGNAL_LINE", "信号线"),
            ("FITTING", "管件"),
            ("NOTE", "NOTE"),
        ] {
            assert_eq!(type_label(&types, code), label, "{code}");
        }
    }

    #[test]
    fn the_length_unit_follows_insunits() {
        for (insunits, unit) in [
            (0, "unitless"),
            (1, "in"),
            (2, "ft"),
            (4, "mm"),
            (5, "cm"),
            (6, "m"),
            (14, "dm"),
            (-1, "unitless"),
            (99, "unitless"),
        ] {
            assert_eq!(length_unit(insunits), unit, "{insunits}");
        }
    }

    /// 两端是 `a`、`b` 的那根线的句柄。
    fn line_between(doc: &Document, a: [f64; 2], b: [f64; 2]) -> String {
        let found: Vec<Handle> = doc
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Line(line) if xy(line.start) == a && xy(line.end) == b => {
                    Some(line.common.handle)
                }
                _ => None,
            })
            .collect();
        assert_eq!(found.len(), 1, "{a:?}–{b:?} 应恰好一根线：{found:?}");
        handle_text(found[0])
    }

    /// 期望的线段：句柄按两端到图上找。
    fn segment(
        doc: &Document,
        (line, run): (&str, Option<&str>),
        [a, b]: [[f64; 2]; 2],
        joint_a: Option<Joint>,
        joint_b: Option<Joint>,
    ) -> SummarySegment {
        SummarySegment {
            line: line.to_string(),
            run: run.map(str::to_string),
            a,
            b,
            length: (b[0] - a[0]).hypot(b[1] - a[1]),
            joint_a,
            joint_b,
            handle: line_between(doc, a, b),
        }
    }

    fn at_fitting(object: &SummaryObject, port: &str) -> Joint {
        Joint::Fitting {
            type_code: object.type_code.clone(),
            tag: object.tag.clone(),
            port: port.to_string(),
        }
    }

    fn at_port(object: &SummaryObject, port: &str) -> Joint {
        Joint::Port {
            type_code: object.type_code.clone(),
            tag: object.tag.clone(),
            port: port.to_string(),
        }
    }

    fn at_connector(kind: &str, line: &str) -> Joint {
        Joint::Connector {
            kind: kind.to_string(),
            line: line.to_string(),
        }
    }

    fn on_line(line: &str, run: Option<&str>, tee: bool) -> Joint {
        Joint::Segment {
            line: line.to_string(),
            run: run.map(str::to_string),
            tee,
        }
    }

    /// 小图里 `block` 的那个符号对象；`on_run` 是插在管路上的那个。
    fn symbol_object(
        summary: &DrawingSummary,
        doc: &Document,
        block: &str,
        on_run: bool,
    ) -> SummaryObject {
        let handle = only(doc, |entity, author| {
            block_of(entity) == Some(block) && (author.topology == ROLE_FITTING) == on_run
        });
        found(summary, handle).clone()
    }

    #[test]
    fn every_end_of_the_small_drawing_says_what_it_is_joined_to() {
        let mut app = app();
        small_drawing(&mut app);
        let summary = summary_of(&app);
        let doc = app.active_document_for_test();
        let pump = symbol_object(&summary, doc, PUMP, false);
        let vessel = symbol_object(&summary, doc, VESSEL, false);
        let valve = symbol_object(&summary, doc, GATE_VALVE, true);
        let fitting = symbol_object(&summary, doc, TRANSMITTER, true);
        let instrument = symbol_object(&summary, doc, TRANSMITTER, false);
        let run = ("L-1001", Some("L-1001-01"));
        let want = vec![
            segment(
                doc,
                ("L-1001", None),
                [[200.0, 100.0], [300.0, 100.0]],
                None,
                Some(at_connector("pipe_out", "L-1001")),
            ),
            segment(
                doc,
                run,
                [[14.0, 0.0], [30.0, 0.0]],
                Some(at_port(&pump, "out1")),
                Some(at_fitting(&fitting, "连接点1")),
            ),
            segment(
                doc,
                run,
                [[50.0, 0.0], [84.0, 0.0]],
                Some(at_fitting(&fitting, "连接点2")),
                Some(at_fitting(&valve, "连接点1")),
            ),
            segment(
                doc,
                run,
                [[116.0, 0.0], [148.0, 0.0]],
                Some(at_fitting(&valve, "连接点2")),
                Some(Joint::Break),
            ),
            segment(
                doc,
                run,
                [[152.0, 0.0], [200.0, 0.0]],
                Some(Joint::Break),
                Some(at_port(&vessel, "in1")),
            ),
        ];
        assert_eq!(summary.segments, want);

        let ends = vec![
            SignalEnd {
                tag: "S-2001".to_string(),
                at: [0.0, 50.0],
                joint: None,
            },
            SignalEnd {
                tag: "S-2001".to_string(),
                at: [50.0, 80.0],
                joint: Some(at_port(&instrument, "in1")),
            },
        ];
        assert_eq!(summary.signal_ends, ends);

        let connector = handle_text(only(doc, |_, author| author.topology == ROLE_CONNECTOR_OUT));
        let want = SummaryConnector {
            kind: "pipe_out".to_string(),
            line: "L-1001".to_string(),
            at: [300.0, 100.0],
            radius: 3.0,
            peer_kind: PeerKind::None,
            peer_drawing: None,
            upstream: Some("L-1001".to_string()),
            downstream: None,
            handle: connector.clone(),
        };
        assert_eq!(summary.connectors, vec![want]);

        let text = serde_json::to_string(&summary).expect("JSON");
        let order: Vec<usize> = [
            "\"objects\":[",
            "\"segments\":[",
            "\"signal_ends\":[",
            "\"connectors\":[",
            "\"ports\":[",
        ]
        .iter()
        .map(|key| text.find(key).unwrap_or_else(|| panic!("缺 {key}：{text}")))
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        let line = line_between(doc, [200.0, 100.0], [300.0, 100.0]);
        let first = format!(
            r#""segments":[{{"line":"L-1001","run":null,"a":[200.0,100.0],"b":[300.0,100.0],"length":100.0,"joint_a":null,"joint_b":{{"to":"connector","kind":"pipe_out","line":"L-1001"}},"handle":"{line}"}},"#
        );
        assert!(text.contains(&first), "{text}");
        let joints = [
            format!(
                r#""joint_a":{{"to":"port","type_code":"EQUIPMENT","tag":"{}","port":"out1"}}"#,
                pump.tag
            ),
            format!(
                r#""joint_b":{{"to":"fitting","type_code":"VALVE","tag":"{}","port":"连接点1"}}"#,
                valve.tag
            ),
            r#""joint_b":{"to":"break"}"#.to_string(),
            format!(
                r#""connectors":[{{"kind":"pipe_out","line":"L-1001","at":[300.0,100.0],"radius":3.0,"peer_kind":"none","peer_drawing":null,"upstream":"L-1001","downstream":null,"handle":"{connector}"}}]"#
            ),
        ];
        for joint in &joints {
            assert!(text.contains(joint.as_str()), "缺 {joint}：{text}");
        }
    }

    /// 管路的拐点、支管接在主管中间、管线接管路、信号线自己分叉，都不算悬空。
    /// 插在管段端点上的管件不拆段，管段的 `peer` 也不记，符号的端口又不在
    /// 端点上，那一端照 §3.2 的顺序判成悬空。
    #[test]
    fn bends_branches_and_tees_are_joined_but_an_endpoint_fitting_is_not() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[-50.0, 0.0], [0.0, 0.0]]);
        draw(
            &mut app,
            "PIDPIPE run",
            &[[0.0, 0.0], [100.0, 0.0], [100.0, 50.0]],
        );
        draw(&mut app, "PIDPIPE run", &[[50.0, 0.0], [50.0, -40.0]]);
        draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 50.0]]);
        draw(
            &mut app,
            "PIDPIPE signal S-3001",
            &[[0.0, 100.0], [100.0, 100.0]],
        );
        draw(
            &mut app,
            "PIDPIPE signal S-3001",
            &[[50.0, 100.0], [50.0, 130.0]],
        );
        let summary = summary_of(&app);
        let doc = app.active_document_for_test();
        let table = TopologyTable::load(doc);
        let runs: Vec<&str> = table.pipe_runs.iter().map(|run| run.tag.as_str()).collect();
        assert_eq!(runs, ["L-1001-01", "L-1001-02"]);

        let main = Some("L-1001-01");
        let want = vec![
            segment(
                doc,
                ("L-1001", None),
                [[-50.0, 0.0], [0.0, 0.0]],
                None,
                Some(on_line("L-1001", main, false)),
            ),
            segment(
                doc,
                ("L-1001", main),
                [[0.0, 0.0], [100.0, 0.0]],
                Some(on_line("L-1001", None, false)),
                Some(on_line("L-1001", main, false)),
            ),
            segment(
                doc,
                ("L-1001", main),
                [[100.0, 0.0], [100.0, 50.0]],
                Some(on_line("L-1001", main, false)),
                None,
            ),
            segment(
                doc,
                ("L-1001", Some("L-1001-02")),
                [[50.0, 0.0], [50.0, -40.0]],
                Some(on_line("L-1001", main, true)),
                None,
            ),
        ];
        assert_eq!(summary.segments, want);
        let text = serde_json::to_string(&summary.segments).expect("JSON");
        assert!(
            text.contains(r#""joint_a":{"to":"segment","line":"L-1001","run":null},"#),
            "{text}"
        );
        assert!(
            text.contains(
                r#""joint_a":{"to":"segment","line":"L-1001","run":"L-1001-01","tee":true},"#
            ),
            "{text}"
        );

        let end = |at: [f64; 2], joint: Option<Joint>| SignalEnd {
            tag: "S-3001".to_string(),
            at,
            joint,
        };
        let ends = vec![
            end([0.0, 100.0], None),
            end([50.0, 100.0], Some(on_line("S-3001", None, true))),
            end([50.0, 130.0], None),
            end([100.0, 100.0], None),
        ];
        assert_eq!(summary.signal_ends, ends);

        let valve = symbol_object(&summary, doc, GATE_VALVE, true);
        let joined: Vec<(&str, bool)> = summary
            .ports
            .iter()
            .filter(|port| port.tag == valve.tag)
            .map(|port| (port.port.as_str(), port.joined))
            .collect();
        assert_eq!(joined, [("in1", false), ("out1", false)]);

        let edges = [
            ["PIPE_LINE|L-1001", "PIPE_RUN|L-1001-01"],
            ["PIPE_RUN|L-1001-01", "PIPE_RUN|L-1001-02"],
        ]
        .map(|edge| edge.map(str::to_string));
        assert_eq!(graph(&summary).edges, edges, "拐点、信号线自己分叉不出边");
    }

    /// `peer_kind` 的四种取值。没记对端、只指了目标图的用命令画；服务器配对、
    /// 同进程配对写下的记录在图的副本上改出来。
    #[test]
    fn connectors_tell_how_their_peer_was_recorded() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0], [100.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 50.0], [100.0, 50.0]]);
        draw(&mut app, "PIDCONNECTOR pipe_out", &[[100.0, 0.0]]);
        draw(&mut app, "PIDCONNECTOR pipe_in PID-1100", &[[0.0, 0.0]]);
        draw(&mut app, "PIDCONNECTOR pipe_out PID-1102", &[[100.0, 50.0]]);
        draw(&mut app, "PIDCONNECTOR pipe_in", &[[0.0, 50.0]]);
        let mut doc = app.active_document_for_test().clone();
        let mut table = TopologyTable::load(&doc);
        assert_eq!(table.connectors.len(), 4, "{:#?}", table.connectors);
        for record in &mut table.connectors {
            match (record.line_tag.as_str(), record.kind.as_str()) {
                ("L-1002", "pipe_out") => record.downstream = Some("L-2001".to_string()),
                ("L-1002", "pipe_in") => {
                    record.peer = Some("7:2A0".to_string());
                    record.peer_drawing = Some("7".to_string());
                }
                _ => {}
            }
        }
        table.store(&mut doc);
        let summary = summarize(&doc, &app.pid_symbols, meta(), DEFAULT_TOLERANCE);

        let connector = |kind: &str,
                         line: &str,
                         at: [f64; 2],
                         peer_kind: PeerKind,
                         peer_drawing: Option<&str>,
                         downstream: Option<&str>| SummaryConnector {
            kind: kind.to_string(),
            line: line.to_string(),
            at,
            radius: 3.0,
            peer_kind,
            peer_drawing: peer_drawing.map(str::to_string),
            upstream: Some(line.to_string()),
            downstream: downstream.map(str::to_string),
            handle: table
                .connectors
                .iter()
                .find(|record| record.line_tag == line && record.kind == kind)
                .expect("连接符记录")
                .handle
                .clone(),
        };
        let want = vec![
            connector(
                "pipe_in",
                "L-1001",
                [0.0, 0.0],
                PeerKind::Target,
                Some("PID-1100"),
                None,
            ),
            connector(
                "pipe_out",
                "L-1001",
                [100.0, 0.0],
                PeerKind::None,
                None,
                None,
            ),
            connector(
                "pipe_in",
                "L-1002",
                [0.0, 50.0],
                PeerKind::Local,
                None,
                None,
            ),
            connector(
                "pipe_out",
                "L-1002",
                [100.0, 50.0],
                PeerKind::Server,
                Some("PID-1102"),
                Some("L-2001"),
            ),
        ];
        assert_eq!(summary.connectors, want);
        let text = serde_json::to_string(&summary.connectors).expect("JSON");
        for kind in ["none", "target", "server"] {
            assert!(text.contains(&format!(r#""peer_kind":"{kind}""#)), "{text}");
        }
        assert!(
            text.contains(r#""peer_kind":"local","peer_drawing":null,"#),
            "同进程配的对不输出文档编号：{text}"
        );

        for line in ["L-1001", "L-1002"] {
            let own = summary
                .segments
                .iter()
                .find(|segment| segment.line == line)
                .expect("管线自己的线");
            assert_eq!(own.joint_a, Some(at_connector("pipe_in", line)));
            assert_eq!(own.joint_b, Some(at_connector("pipe_out", line)));
        }
    }

    #[test]
    fn the_graph_links_each_owner_to_what_its_ends_touch() {
        let mut app = app();
        small_drawing(&mut app);
        let summary = summary_of(&app);
        let doc = app.active_document_for_test();
        let name = |block: &str, on_run: bool| {
            let object = symbol_object(&summary, doc, block, on_run);
            format!("{}|{}", object.type_code, object.tag)
        };
        let pump = name(PUMP, false);
        let vessel = name(VESSEL, false);
        let valve = name(GATE_VALVE, true);
        assert_eq!(
            name(TRANSMITTER, false),
            "INSTRUMENT|",
            "仪表没有位号：不进图，接到它的信号线端点不出边"
        );
        let mut nodes = vec![
            pump.clone(),
            vessel.clone(),
            name(GATE_VALVE, false),
            valve.clone(),
            "NOTE|N-1".to_string(),
            "PIPE_LINE|L-1001".to_string(),
            "PIPE_RUN|L-1001-01".to_string(),
            "SIGNAL_LINE|S-2001".to_string(),
            "connector|L-1001|pipe_out".to_string(),
        ];
        nodes.sort();
        let run = "PIPE_RUN|L-1001-01";
        let mut edges = vec![
            [format!("{pump}#out1"), run.to_string()],
            [format!("{valve}#连接点1"), run.to_string()],
            [format!("{valve}#连接点2"), run.to_string()],
            [format!("{vessel}#in1"), run.to_string()],
            [
                "PIPE_LINE|L-1001".to_string(),
                "connector|L-1001|pipe_out".to_string(),
            ],
        ];
        for edge in &mut edges {
            edge.sort();
        }
        edges.sort();

        let graph = graph(&summary);
        assert_eq!(graph.nodes, nodes);
        assert_eq!(graph.edges, edges);
        let json = serde_json::to_value(&graph).expect("JSON");
        assert_eq!(json["edges"][0], json!(edges[0]), "{json}");
        assert_eq!(json["nodes"][0], json!(nodes[0]), "{json}");
    }

    /// 打断符那根斜线缩在缝里，它的包围盒套不住缝两侧的端点；按包围盒的
    /// 外接圆判，管子朝哪个方向都一样。
    #[test]
    fn both_sides_of_a_break_are_cut_whatever_the_pipe_direction() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, -50.0], [10.0, -50.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0], [60.0, 80.0]]);
        draw(&mut app, "PIDBREAK", &[[30.0, 40.0]]);
        let summary = summary_of(&app);
        let mark = summary
            .objects
            .iter()
            .find(|object| object.kind == ObjectKind::Break)
            .expect("打断符");
        let cut: Vec<&SummarySegment> = summary
            .segments
            .iter()
            .filter(|segment| segment.run.is_some())
            .collect();
        assert_eq!(cut.len(), 2, "{:#?}", summary.segments);
        let [x0, y0, x1, y1] = mark.bbox;
        for (end, joint) in [(cut[0].b, &cut[0].joint_b), (cut[1].a, &cut[1].joint_a)] {
            assert!(
                !(x0..=x1).contains(&end[0]) || !(y0..=y1).contains(&end[1]),
                "端点 {end:?} 本在包围盒 {:?} 外",
                mark.bbox
            );
            assert_eq!(joint, &Some(Joint::Break), "{end:?}");
        }
        assert_eq!(cut[0].joint_a, None);
        assert_eq!(cut[1].joint_b, None);
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：同一纳秒里建的两个靠计数分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-project-index-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("1970 年以后")
                    .as_nanos()
            ));
            std::fs::create_dir(&path).expect("临时目录");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fingerprint_of(path: &Path) -> Fingerprint {
        let metadata = std::fs::metadata(path).expect("文件信息");
        let modified = metadata
            .modified()
            .expect("修改时间")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("1970 年以后");
        Fingerprint {
            size: metadata.len(),
            modified_at: i64::try_from(modified.as_secs()).expect("秒数"),
        }
    }

    /// 往返允许不同的三项抹掉：`source`、`fingerprint` 和各处的句柄。
    fn without_handles(mut summary: DrawingSummary) -> DrawingSummary {
        summary.source = SummarySource::File;
        summary.fingerprint = None;
        for object in &mut summary.objects {
            object.handle.clear();
        }
        for segment in &mut summary.segments {
            segment.handle.clear();
        }
        for connector in &mut summary.connectors {
            connector.handle.clear();
        }
        summary
    }

    /// §7 往返：建图 → 存 DWG → `load_file` → 摘要，与存盘前的摘要只差
    /// `source`、`fingerprint` 和句柄。命令建的小图和合成图各来一遍。
    #[test]
    fn a_drawing_saved_as_dwg_and_read_back_summarizes_the_same() {
        let mut app = app();
        small_drawing(&mut app);
        let synthetic = synthetic_drawing(200);
        let builtin = SymbolLibrary::builtin();
        let drawings = [
            (
                "小图",
                "small.dwg",
                app.active_document_for_test(),
                &app.pid_symbols,
            ),
            ("合成图", "synthetic.dwg", &synthetic, &builtin),
        ];
        let scratch = Scratch::new();
        for (name, file, doc, library) in drawings {
            let open = SummaryMeta {
                source: SummarySource::Open,
                fingerprint: None,
                ..meta()
            };
            let before = summarize(doc, library, open, DEFAULT_TOLERANCE);
            assert!(!before.objects.is_empty(), "{name}的摘要不该是空的");
            let bytes = crate::io::save_to_bytes(doc, "dwg", document::DxfVersion::AC1032)
                .unwrap_or_else(|error| panic!("{name}存 DWG：{error}"));
            let path = scratch.0.join(file);
            std::fs::write(&path, bytes).expect("写文件");
            let back =
                crate::io::load_file(&path).unwrap_or_else(|error| panic!("{name}读回：{error}"));
            let saved = SummaryMeta {
                fingerprint: Some(fingerprint_of(&path)),
                ..meta()
            };
            let after = summarize(&back, library, saved, DEFAULT_TOLERANCE);
            assert_eq!(after.source, SummarySource::File);
            assert_eq!(without_handles(after), without_handles(before), "{name}");
        }
    }

    /// 契约规模：2,000 个对象的合成图。
    const SYNTHETIC_OBJECTS: usize = 2000;
    /// 预算：一次摘要不超过 150 ms（成功标准第 9 条）。
    const BUDGET: Duration = Duration::from_millis(150);
    /// 采样次数：取中位数。
    const RUNS: usize = 3;

    /// 合成图说是多少个对象就是多少个，每组 20 个的种类一样；线头全都接着
    /// 东西，除了悬空那一种，§3.2 的接点都有；位号不重复。
    #[test]
    fn the_synthetic_drawing_has_the_objects_it_says_and_no_loose_ends() {
        let summary = summarize(
            &synthetic_drawing(SYNTHETIC_OBJECTS),
            &SymbolLibrary::builtin(),
            meta(),
            DEFAULT_TOLERANCE,
        );
        let cells = SYNTHETIC_OBJECTS / 20;
        assert_eq!(summary.objects.len(), SYNTHETIC_OBJECTS);
        let mut kinds: HashMap<ObjectKind, usize> = HashMap::new();
        for object in &summary.objects {
            *kinds.entry(object.kind).or_default() += 1;
        }
        let per_cell = [
            (ObjectKind::Equipment, 3),
            (ObjectKind::PipeLine, 2),
            (ObjectKind::PipeRun, 3),
            (ObjectKind::Valve, 4),
            (ObjectKind::Fitting, 1),
            (ObjectKind::Break, 1),
            (ObjectKind::Instrument, 4),
            (ObjectKind::SignalLine, 2),
        ];
        let expected: HashMap<ObjectKind, usize> = per_cell
            .iter()
            .map(|(kind, count)| (*kind, count * cells))
            .collect();
        assert_eq!(kinds, expected);

        let own: Vec<&SummaryObject> = summary
            .objects
            .iter()
            .filter(|object| object.own_tag)
            .collect();
        let tags: HashSet<(&str, &str)> = own
            .iter()
            .map(|object| (object.type_code.as_str(), object.tag.as_str()))
            .collect();
        assert_eq!(tags.len(), own.len(), "位号重复");
        assert!(
            own.iter()
                .all(|object| !object.tag.is_empty() && object.symbol_known),
            "位号都该有、符号都该认得"
        );

        let ends = summary
            .segments
            .iter()
            .flat_map(|segment| [&segment.joint_a, &segment.joint_b])
            .chain(summary.signal_ends.iter().map(|end| &end.joint));
        let mut joints: BTreeMap<&str, usize> = BTreeMap::new();
        for joint in ends {
            let name = match joint {
                None => "悬空",
                Some(Joint::Fitting { .. }) => "管件",
                Some(Joint::Break) => "打断",
                Some(Joint::Port { .. }) => "端口",
                Some(Joint::Connector { .. }) => "连接符",
                Some(Joint::Segment { tee: false, .. }) => "线",
                Some(Joint::Segment { tee: true, .. }) => "三通",
            };
            *joints.entry(name).or_default() += 1;
        }
        let per_cell = [
            ("管件", 10),
            ("打断", 2),
            ("端口", 8),
            ("连接符", 3),
            ("线", 4),
            ("三通", 1),
        ];
        let expected: BTreeMap<&str, usize> = per_cell
            .iter()
            .map(|(name, count)| (*name, count * cells))
            .collect();
        assert_eq!(joints, expected);
        assert_eq!(summary.connectors.len(), 3 * cells);
        assert_eq!(summary.ports.len(), 24 * cells);
        assert_eq!(
            summary.ports.iter().filter(|port| port.joined).count(),
            18 * cells
        );
    }

    #[test]
    #[ignore]
    fn project_index_summary_2000_objects_within_150_ms() {
        let doc = synthetic_drawing(SYNTHETIC_OBJECTS);
        let library = SymbolLibrary::builtin();
        let mut samples = Vec::with_capacity(RUNS);
        for run in 1..=RUNS {
            let started = Instant::now();
            let summary = summarize(&doc, &library, meta(), DEFAULT_TOLERANCE);
            let elapsed = started.elapsed();
            println!(
                "IX-4 run {run}/{RUNS}: {:.2} ms, objects {}, segments {}, ports {}",
                elapsed.as_secs_f64() * 1e3,
                summary.objects.len(),
                summary.segments.len(),
                summary.ports.len()
            );
            assert_eq!(summary.objects.len(), SYNTHETIC_OBJECTS, "run {run}");
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[RUNS / 2];
        let shown: Vec<String> = samples
            .iter()
            .map(|sample| format!("{:.2}", sample.as_secs_f64() * 1e3))
            .collect();
        println!(
            "IX-4 median {:.2} ms of {shown:?}, budget {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
        assert!(
            median <= BUDGET,
            "IX-4 超预算：中位数 {:.2} ms（{shown:?}），预算 {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
    }

    // ── 项目索引 ──

    const NOW: i64 = 1_794_800_000;

    /// 造索引用的一张图的摘要：只带给的对象和连接符，文件名是图号加 `.dwg`。
    fn drawing_with(
        drawing_id: &str,
        objects: Vec<SummaryObject>,
        connectors: Vec<SummaryConnector>,
    ) -> DrawingSummary {
        DrawingSummary {
            version: SUMMARY_VERSION,
            drawing_id: drawing_id.to_string(),
            file: Some(format!("{drawing_id}.dwg")),
            source: SummarySource::File,
            fingerprint: None,
            pbs: None,
            length_unit: "mm".to_string(),
            objects,
            segments: Vec::new(),
            signal_ends: Vec::new(),
            connectors,
            ports: Vec::new(),
        }
    }

    fn object_at(
        kind: ObjectKind,
        type_code: &str,
        tag: &str,
        at: [f64; 2],
        handle: &str,
    ) -> SummaryObject {
        SummaryObject {
            kind,
            type_code: type_code.to_string(),
            type_label: String::new(),
            tag: tag.to_string(),
            own_tag: kind.has_own_tag(),
            tag_source: "auto".to_string(),
            tag_status: "ok".to_string(),
            symbol: None,
            symbol_known: true,
            props: BTreeMap::new(),
            overrides: Vec::new(),
            owner: None,
            layer: "0".to_string(),
            at,
            bbox: [at[0], at[1], at[0], at[1]],
            handle: handle.to_string(),
        }
    }

    /// 一个连接符：`peer` 是对端怎么记的和对端图号，`lines` 是流出侧、流入侧线号。
    fn plug(
        kind: &str,
        line: &str,
        at: [f64; 2],
        handle: &str,
        peer: (PeerKind, Option<&str>),
        lines: [Option<&str>; 2],
    ) -> SummaryConnector {
        SummaryConnector {
            kind: kind.to_string(),
            line: line.to_string(),
            at,
            radius: 3.0,
            peer_kind: peer.0,
            peer_drawing: peer.1.map(str::to_string),
            upstream: lines[0].map(str::to_string),
            downstream: lines[1].map(str::to_string),
            handle: handle.to_string(),
        }
    }

    fn place_on(drawing_id: &str, at: [f64; 2], handle: &str) -> Place {
        Place {
            drawing_id: drawing_id.to_string(),
            at,
            handle: handle.to_string(),
        }
    }

    fn failed(drawing_id: &str, error: &str) -> FailedDrawing {
        FailedDrawing {
            drawing_id: drawing_id.to_string(),
            file: format!("{drawing_id}.dwg"),
            pbs: None,
            error: error.to_string(),
        }
    }

    fn index_of(summaries: &[DrawingSummary], failed: &[FailedDrawing]) -> ProjectIndex {
        build_index_at(Path::new("D:/proj/cdu"), summaries, failed, NOW).expect("索引")
    }

    /// §3.5 的两张图：算的四类（设备、阀门、仪表、其他）各有一个两张图都有的位号，
    /// `V-1001` 在第一张上还有两个；不算的（管线、管路、信号线、不单独编号的管件、打断符、
    /// 没位号的）两张图上也同号；只在一张图里重的 `V-2001`、类型不同的 `X-1` 也在。
    fn tagged_pair() -> [DrawingSummary; 2] {
        use ObjectKind::{
            Break, Equipment, Fitting, Instrument, Other, PipeLine, PipeRun, SignalLine, Valve,
        };
        let first = drawing_with(
            "PID-1101",
            vec![
                object_at(Equipment, "EQUIPMENT", "P-1001", [120.0, 80.0], "2A4"),
                object_at(Valve, "VALVE", "V-1001", [30.0, 0.0], "A1"),
                object_at(Valve, "VALVE", "V-1001", [10.0, 0.0], "A2"),
                object_at(Instrument, "INSTRUMENT", "PT-1001", [0.0, 40.0], "A3"),
                object_at(Other, "NOTE", "N-1", [0.0, 90.0], "A4"),
                object_at(Other, "NOTE", "X-1", [0.0, 95.0], "A5"),
                object_at(Valve, "VALVE", "V-2001", [50.0, 0.0], "A6"),
                object_at(Valve, "VALVE", "V-2001", [60.0, 0.0], "A7"),
                object_at(PipeLine, "PIPE_LINE", "L-1001", [0.0, 0.0], "A8"),
                object_at(PipeRun, "PIPE_RUN", "L-1001-01", [0.0, 0.0], "A9"),
                object_at(SignalLine, "SIGNAL_LINE", "S-2001", [0.0, 40.0], "AA"),
                object_at(Fitting, "FITTING", "L-1001-01", [70.0, 0.0], "AB"),
                object_at(Break, "FITTING", "", [80.0, 0.0], "AC"),
                object_at(Instrument, "INSTRUMENT", "", [0.0, 60.0], "AD"),
            ],
            Vec::new(),
        );
        let second = drawing_with(
            "PID-1102",
            vec![
                object_at(Equipment, "EQUIPMENT", "P-1001", [60.0, 40.0], "1F0"),
                object_at(Valve, "VALVE", "V-1001", [20.0, 0.0], "B1"),
                object_at(Instrument, "INSTRUMENT", "PT-1001", [0.0, 40.0], "B2"),
                object_at(Other, "NOTE", "N-1", [0.0, 90.0], "B3"),
                object_at(Equipment, "EQUIPMENT", "X-1", [0.0, 95.0], "B4"),
                object_at(PipeLine, "PIPE_LINE", "L-1001", [0.0, 0.0], "B5"),
                object_at(PipeRun, "PIPE_RUN", "L-1001-01", [0.0, 0.0], "B6"),
                object_at(SignalLine, "SIGNAL_LINE", "S-2001", [0.0, 40.0], "B7"),
                object_at(Fitting, "FITTING", "L-1001-01", [70.0, 0.0], "B8"),
                object_at(Break, "FITTING", "", [80.0, 0.0], "B9"),
                object_at(Instrument, "INSTRUMENT", "", [0.0, 60.0], "BA"),
            ],
            Vec::new(),
        );
        [first, second]
    }

    /// 两张图各有一台 `P-1001` 报一处，两张图都有的管线 `L-1001` 不报（§3.5、成功
    /// 标准第 5 条）。一处重号列出它所有的出现处。
    #[test]
    fn a_tag_on_two_drawings_is_a_duplicate_but_a_line_number_is_not() {
        let index = index_of(&tagged_pair(), &[]);
        let duplicate = |type_code: &str, tag: &str, places: Vec<Place>| Duplicate {
            type_code: type_code.to_string(),
            tag: tag.to_string(),
            places,
        };
        let expected = vec![
            duplicate(
                "EQUIPMENT",
                "P-1001",
                vec![
                    place_on("PID-1101", [120.0, 80.0], "2A4"),
                    place_on("PID-1102", [60.0, 40.0], "1F0"),
                ],
            ),
            duplicate(
                "INSTRUMENT",
                "PT-1001",
                vec![
                    place_on("PID-1101", [0.0, 40.0], "A3"),
                    place_on("PID-1102", [0.0, 40.0], "B2"),
                ],
            ),
            duplicate(
                "NOTE",
                "N-1",
                vec![
                    place_on("PID-1101", [0.0, 90.0], "A4"),
                    place_on("PID-1102", [0.0, 90.0], "B3"),
                ],
            ),
            duplicate(
                "VALVE",
                "V-1001",
                vec![
                    place_on("PID-1101", [10.0, 0.0], "A2"),
                    place_on("PID-1101", [30.0, 0.0], "A1"),
                    place_on("PID-1102", [20.0, 0.0], "B1"),
                ],
            ),
        ];
        assert_eq!(index.duplicates, expected);
        assert!(index.complete);
    }

    /// 合成图当成两张图：每个设备、阀门、仪表的位号都重了，线号、管路号、信号线号
    /// 不报；合成图的连接符都没配过对。
    #[test]
    fn two_copies_of_the_synthetic_drawing_repeat_every_numbered_tag_but_no_line() {
        let library = SymbolLibrary::builtin();
        let doc = synthetic_drawing(40);
        let summaries: Vec<DrawingSummary> = ["PID-1101", "PID-1102"]
            .into_iter()
            .map(|drawing_id| {
                let meta = SummaryMeta {
                    drawing_id: drawing_id.to_string(),
                    file: Some(format!("{drawing_id}.dwg")),
                    ..meta()
                };
                summarize(&doc, &library, meta, DEFAULT_TOLERANCE)
            })
            .collect();
        let index = index_of(&summaries, &[]);
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for duplicate in &index.duplicates {
            *kinds.entry(duplicate.type_code.as_str()).or_default() += 1;
            let drawings: Vec<&str> = duplicate
                .places
                .iter()
                .map(|place| place.drawing_id.as_str())
                .collect();
            assert_eq!(drawings, ["PID-1101", "PID-1102"], "{}", duplicate.tag);
        }
        assert_eq!(
            kinds,
            BTreeMap::from([("EQUIPMENT", 6), ("INSTRUMENT", 8), ("VALVE", 8)])
        );
        assert!(index.links.is_empty());
        assert_eq!(index.unresolved.len(), 2 * 2 * 3);
        assert!(index
            .unresolved
            .iter()
            .all(|plug| plug.status == LinkStatus::Unpaired && plug.target.is_none()));
    }

    /// §3.6 的配对，只看各图记下的：服务器配的对只在对端图上找，同进程配的对在其余
    /// 各图上找；方向相反、同族、两个线号都相同才算。配成对、`one_sided`、
    /// `target_missing`、`unpaired`、`ambiguous` 各有几例；同一线号同进程配对连着跨三张图
    /// 时四个都报 `ambiguous`（待决问题 I2）。
    #[test]
    fn connectors_pair_across_drawings_only_on_what_was_recorded() {
        let server = |drawing: &'static str| (PeerKind::Server, Some(drawing));
        let local = (PeerKind::Local, None);
        let both = |up: &'static str, down: &'static str| [Some(up), Some(down)];
        let only = |up: &'static str| [Some(up), None];
        let at = [0.0, 0.0];
        let first = drawing_with(
            "PID-1101",
            Vec::new(),
            vec![
                plug(
                    "pipe_out",
                    "L-1001",
                    at,
                    "A1",
                    server("PID-1102"),
                    both("L-1001", "L-2001"),
                ),
                plug(
                    "pipe_out",
                    "L-1003",
                    at,
                    "A2",
                    server("PID-1199"),
                    both("L-1003", "L-3001"),
                ),
                plug(
                    "signal_out",
                    "S-2001",
                    at,
                    "A3",
                    server("PID-1102"),
                    both("S-2001", "S-2002"),
                ),
                plug(
                    "pipe_out",
                    "L-1004",
                    at,
                    "A4",
                    (PeerKind::None, None),
                    only("L-1004"),
                ),
                plug(
                    "pipe_out",
                    "L-1005",
                    at,
                    "A5",
                    (PeerKind::Target, Some("PID-1102")),
                    only("L-1005"),
                ),
                plug(
                    "pipe_out",
                    "L-1006",
                    at,
                    "A6",
                    local,
                    both("L-1006", "L-1006"),
                ),
                plug("signal_out", "S-1", at, "A7", local, both("S-1", "S-9")),
                plug(
                    "pipe_out",
                    "L-1008",
                    at,
                    "A8",
                    server("PID-1104"),
                    both("L-1008", "L-4001"),
                ),
                plug(
                    "pipe_out",
                    "L-1009",
                    at,
                    "A9",
                    server("PID-1102"),
                    both("L-1009", "L-2009"),
                ),
            ],
        );
        let second = drawing_with(
            "PID-1102",
            Vec::new(),
            vec![
                plug(
                    "pipe_in",
                    "L-2001",
                    at,
                    "B1",
                    server("PID-1101"),
                    both("L-1001", "L-2001"),
                ),
                plug(
                    "pipe_in",
                    "L-1006",
                    at,
                    "B2",
                    local,
                    both("L-1006", "L-1006"),
                ),
                plug(
                    "pipe_out",
                    "L-1006",
                    at,
                    "B3",
                    local,
                    both("L-1006", "L-1006"),
                ),
                plug("pipe_in", "L-7", at, "B4", local, only("L-7")),
                plug(
                    "signal_in",
                    "S-2003",
                    at,
                    "B5",
                    server("PID-1101"),
                    both("S-2001", "S-2003"),
                ),
                plug(
                    "signal_in",
                    "L-2009",
                    at,
                    "B6",
                    server("PID-1101"),
                    both("L-1009", "L-2009"),
                ),
            ],
        );
        let third = drawing_with(
            "PID-1103",
            Vec::new(),
            vec![
                plug(
                    "pipe_in",
                    "L-1006",
                    at,
                    "C1",
                    local,
                    both("L-1006", "L-1006"),
                ),
                plug("signal_in", "S-9", at, "C2", local, both("S-1", "S-9")),
            ],
        );
        let index = index_of(&[first, second, third], &[failed("PID-1104", "不是 DWG")]);

        let end = |drawing_id: &str, line: &str, handle: &str| LinkEnd {
            drawing_id: drawing_id.to_string(),
            line: line.to_string(),
            at,
            handle: handle.to_string(),
        };
        let expected = vec![
            Link {
                family: LinkFamily::Pipe,
                from: end("PID-1101", "L-1001", "A1"),
                to: end("PID-1102", "L-2001", "B1"),
            },
            Link {
                family: LinkFamily::Signal,
                from: end("PID-1101", "S-1", "A7"),
                to: end("PID-1103", "S-9", "C2"),
            },
        ];
        assert_eq!(index.links, expected);

        let statuses: BTreeMap<&str, (LinkStatus, Option<&str>)> = index
            .unresolved
            .iter()
            .map(|plug| (plug.handle.as_str(), (plug.status, plug.target.as_deref())))
            .collect();
        let expected = BTreeMap::from([
            ("A2", (LinkStatus::TargetMissing, Some("PID-1199"))),
            ("A3", (LinkStatus::OneSided, Some("PID-1102"))),
            ("A4", (LinkStatus::Unpaired, None)),
            ("A5", (LinkStatus::Unpaired, Some("PID-1102"))),
            ("A6", (LinkStatus::Ambiguous, None)),
            ("A8", (LinkStatus::OneSided, Some("PID-1104"))),
            ("A9", (LinkStatus::OneSided, Some("PID-1102"))),
            ("B2", (LinkStatus::Ambiguous, None)),
            ("B3", (LinkStatus::Ambiguous, None)),
            ("B4", (LinkStatus::Unpaired, None)),
            ("B5", (LinkStatus::OneSided, Some("PID-1101"))),
            ("B6", (LinkStatus::OneSided, Some("PID-1101"))),
            ("C1", (LinkStatus::Ambiguous, None)),
        ]);
        assert_eq!(statuses, expected);
        let order: Vec<&str> = index
            .unresolved
            .iter()
            .map(|plug| plug.handle.as_str())
            .collect();
        assert_eq!(
            order,
            ["A2", "A4", "A5", "A6", "A8", "A9", "A3", "B2", "B3", "B6", "B4", "B5", "C1"]
        );
        assert_eq!(
            index.unresolved.first(),
            Some(&Unresolved {
                drawing_id: "PID-1101".to_string(),
                kind: "pipe_out".to_string(),
                line: "L-1003".to_string(),
                at,
                handle: "A2".to_string(),
                status: LinkStatus::TargetMissing,
                target: Some("PID-1199".to_string()),
            })
        );
        assert!(!index.complete);
    }

    /// 一张图读不出来：它带着原因进 `drawings`，`complete` 为假，其余照常。两个文件
    /// 得出同一个图号就不出索引（§3.7），读不出来的图也占着它的图号。
    #[test]
    fn an_unreadable_drawing_leaves_the_index_incomplete_and_a_repeated_drawing_id_stops_it() {
        let [mut first, second] = tagged_pair();
        first.source = SummarySource::Open;
        first.pbs = Some("1001-01".to_string());
        let mut broken = failed("PID-1100", "不是 DWG 文件");
        broken.pbs = Some("1001-02".to_string());
        let index = index_of(&[first.clone(), second.clone()], &[broken]);
        assert!(!index.complete);
        let expected = vec![
            IndexDrawing {
                drawing_id: "PID-1100".to_string(),
                file: Some("PID-1100.dwg".to_string()),
                source: SummarySource::File,
                pbs: Some("1001-02".to_string()),
                objects: None,
                error: Some("不是 DWG 文件".to_string()),
            },
            IndexDrawing {
                drawing_id: "PID-1101".to_string(),
                file: Some("PID-1101.dwg".to_string()),
                source: SummarySource::Open,
                pbs: Some("1001-01".to_string()),
                objects: Some(first.objects.len()),
                error: None,
            },
            IndexDrawing {
                drawing_id: "PID-1102".to_string(),
                file: Some("PID-1102.dwg".to_string()),
                source: SummarySource::File,
                pbs: None,
                objects: Some(second.objects.len()),
                error: None,
            },
        ];
        assert_eq!(index.drawings, expected);
        assert_eq!(index.duplicates.len(), 4, "读得出来的两张图照常比");

        let dir = Path::new("D:/proj/cdu");
        let mut copy = first.clone();
        copy.file = Some("PID-1101-copy.dwg".to_string());
        let repeated =
            build_index_at(dir, &[first.clone(), second, copy], &[], NOW).expect_err("图号重了");
        assert_eq!(
            repeated,
            IndexError::DrawingIdDuplicated {
                drawing_id: "PID-1101".to_string(),
                files: ["PID-1101-copy.dwg".to_string(), "PID-1101.dwg".to_string()],
            }
        );
        assert_eq!(repeated.code(), "INDEX_DRAWING_ID_DUPLICATED");
        let mut twin = failed("PID-1101", "读不出来");
        twin.file = "PID-1101.DWG".to_string();
        assert_eq!(
            build_index_at(dir, &[first], &[twin], NOW),
            Err(IndexError::DrawingIdDuplicated {
                drawing_id: "PID-1101".to_string(),
                files: ["PID-1101.DWG".to_string(), "PID-1101.dwg".to_string()],
            })
        );
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"").expect("建文件");
    }

    /// 列出的图：文件名、图号、PBS 节点；顺便查 `path` 是目录加文件名。
    fn listed(dir: &Path) -> Vec<(String, String, Option<String>)> {
        list_drawings(dir)
            .expect("列图")
            .into_iter()
            .map(|file| {
                assert_eq!(file.path, dir.join(&file.file));
                (file.file, file.drawing_id, file.pbs)
            })
            .collect()
    }

    fn entry(file: &str, drawing_id: &str, pbs: Option<&str>) -> (String, String, Option<String>) {
        (
            file.to_string(),
            drawing_id.to_string(),
            pbs.map(str::to_string),
        )
    }

    /// §3.7：目录下直接放着的 `.dwg`（扩展名不分大小写），跳过 `.` 和 `~$` 开头的、
    /// 别的扩展名、子目录里的和叫 `.dwg` 的目录，按文件名排。图号和 PBS 节点取项目
    /// 文件 `drawings[]` 给的（文件名不分大小写），没给图号的取文件名去掉扩展名；
    /// `drawings[]` 里目录中没有的文件不理。
    #[test]
    fn list_drawings_takes_the_dwg_files_right_in_the_folder() {
        let scratch = Scratch::new();
        let dir = scratch.0.as_path();
        for name in [
            "PID-1101.dwg",
            "pid-1102.DWG",
            "PID-1103.dwg",
            ".PID-1104.dwg",
            "~$PID-1101.dwg",
            "notes.txt",
            "PID-1105.dwg.bak",
        ] {
            touch(dir, name);
        }
        std::fs::create_dir(dir.join("publish")).expect("子目录");
        touch(&dir.join("publish"), "PID-1106.dwg");
        std::fs::create_dir(dir.join("folder.dwg")).expect("叫 .dwg 的目录");
        assert_eq!(
            listed(dir),
            [
                entry("PID-1101.dwg", "PID-1101", None),
                entry("PID-1103.dwg", "PID-1103", None),
                entry("pid-1102.DWG", "pid-1102", None),
            ]
        );

        let project = json!({
            "version": 1,
            "drawings": [
                { "file": "pid-1101.dwg", "drawing_id": "P&ID-1101", "pbs": "1001-01" },
                { "file": "PID-1103.dwg", "drawing_id": null, "pbs": "1001-02" },
                { "file": "PID-1107.dwg", "drawing_id": "PID-X" }
            ]
        });
        std::fs::write(dir.join(PROJECT_FILE), format!("\u{feff}{project}")).expect("项目文件");
        assert_eq!(
            listed(dir),
            [
                entry("PID-1101.dwg", "P&ID-1101", Some("1001-01")),
                entry("PID-1103.dwg", "PID-1103", Some("1001-02")),
                entry("pid-1102.DWG", "pid-1102", None),
            ]
        );
    }

    /// 项目文件读不出来、`drawings[]` 写错、同一个文件写了两遍、两个文件得出同一个图号、
    /// 目录读不了，都不列。
    #[test]
    fn list_drawings_refuses_a_bad_project_file_and_a_repeated_drawing_id() {
        let scratch = Scratch::new();
        let dir = scratch.0.as_path();
        touch(dir, "PID-1101.dwg");
        touch(dir, "PID-1103.dwg");
        let refused = |drawings: Value| {
            let project = json!({ "version": 1, "drawings": drawings });
            std::fs::write(dir.join(PROJECT_FILE), project.to_string()).expect("项目文件");
            list_drawings(dir).expect_err("该报错")
        };
        let schema = |at: &str, reason: &str| {
            IndexError::ProjectFile(MasterDataError::Schema {
                at: at.to_string(),
                reason: reason.to_string(),
            })
        };
        let wrong_type = refused(json!([{ "file": "PID-1101.dwg" }, { "file": 1103 }]));
        assert_eq!(wrong_type, schema("drawings[1].file", "应为文本"));
        assert_eq!(wrong_type.code(), "MASTERDATA_SCHEMA_ERROR");
        assert_eq!(
            refused(json!({ "file": "PID-1101.dwg" })),
            schema("drawings", "应为数组")
        );
        assert_eq!(
            refused(json!([{ "file": "PID-1101.dwg", "pbs": 1001 }])),
            schema("drawings[0].pbs", "应为文本或 null")
        );
        assert_eq!(
            refused(json!([{ "file": "PID-1101.dwg" }, { "file": "pid-1101.DWG" }])),
            IndexError::ProjectFile(MasterDataError::Duplicate {
                at: "drawings[1].file".to_string(),
                value: "pid-1101.DWG".to_string(),
            })
        );
        let repeated = refused(json!([{ "file": "PID-1101.dwg", "drawing_id": "PID-1103" }]));
        assert_eq!(
            repeated,
            IndexError::DrawingIdDuplicated {
                drawing_id: "PID-1103".to_string(),
                files: ["PID-1101.dwg".to_string(), "PID-1103.dwg".to_string()],
            }
        );
        assert_eq!(repeated.code(), "INDEX_DRAWING_ID_DUPLICATED");

        std::fs::write(dir.join(PROJECT_FILE), r#"{"version":2}"#).expect("项目文件");
        let newer = list_drawings(dir).expect_err("版本太高");
        assert_eq!(newer.code(), "MASTERDATA_VERSION_UNSUPPORTED");
        let missing = list_drawings(&dir.join("nowhere")).expect_err("没有这个目录");
        assert_eq!(missing.code(), "INDEX_IO_ERROR");
    }

    /// 一个位号在项目里的所有出现处：可以限定类型；管线这样跨图同号的也列；不单独
    /// 编号的管件不算它所在管路的出现处；索引里没有的图不算。
    #[test]
    fn places_lists_every_drawing_a_tag_is_on() {
        let [first, second] = tagged_pair();
        let index = index_of(&[first.clone(), second.clone()], &[]);
        let stray = drawing_with(
            "PID-1199",
            vec![object_at(
                ObjectKind::Equipment,
                "EQUIPMENT",
                "P-1001",
                [0.0, 0.0],
                "C1",
            )],
            Vec::new(),
        );
        let summaries = [first, second, stray];
        let handles = |type_code: Option<&str>, tag: &str| -> Vec<String> {
            index
                .places(&summaries, type_code, tag)
                .into_iter()
                .map(|place| format!("{}/{}", place.drawing_id, place.handle))
                .collect()
        };
        assert_eq!(handles(None, "P-1001"), ["PID-1101/2A4", "PID-1102/1F0"]);
        assert_eq!(handles(None, "X-1"), ["PID-1101/A5", "PID-1102/B4"]);
        assert_eq!(handles(Some("EQUIPMENT"), "X-1"), ["PID-1102/B4"]);
        assert_eq!(handles(None, "L-1001"), ["PID-1101/A8", "PID-1102/B5"]);
        assert_eq!(handles(None, "L-1001-01"), ["PID-1101/A9", "PID-1102/B6"]);
        assert_eq!(
            handles(None, "V-1001"),
            ["PID-1101/A2", "PID-1101/A1", "PID-1102/B1"]
        );
        assert!(handles(None, "").is_empty());
        assert!(handles(None, "P-9999").is_empty());
        assert_eq!(
            index
                .places(&summaries, Some("EQUIPMENT"), "P-1001")
                .first(),
            Some(&place_on("PID-1101", [120.0, 80.0], "2A4"))
        );
    }

    /// 索引的 JSON：键序照 §3.4，读得回来。
    #[test]
    fn the_index_json_keeps_the_key_order_of_the_spec() {
        let pair = [Some("L-1001"), Some("L-2001")];
        let mut first = drawing_with(
            "PID-1101",
            vec![object_at(
                ObjectKind::Equipment,
                "EQUIPMENT",
                "P-1001",
                [120.0, 80.0],
                "2A4",
            )],
            vec![
                plug(
                    "pipe_out",
                    "L-1001",
                    [240.0, 80.0],
                    "2C0",
                    (PeerKind::Server, Some("PID-1102")),
                    pair,
                ),
                plug(
                    "pipe_out",
                    "L-1003",
                    [240.0, 20.0],
                    "2D0",
                    (PeerKind::None, None),
                    [Some("L-1003"), None],
                ),
            ],
        );
        first.source = SummarySource::Open;
        first.pbs = Some("1001-01".to_string());
        let second = drawing_with(
            "PID-1102",
            vec![object_at(
                ObjectKind::Equipment,
                "EQUIPMENT",
                "P-1001",
                [60.0, 40.0],
                "1F0",
            )],
            vec![plug(
                "pipe_in",
                "L-2001",
                [0.0, 80.0],
                "1B3",
                (PeerKind::Server, Some("PID-1101")),
                pair,
            )],
        );
        let index = index_of(&[first, second], &[]);
        let expected = concat!(
            r#"{"version":1,"project_dir":"D:/proj/cdu","generated_at":1794800000,"complete":true,"#,
            r#""drawings":[{"drawing_id":"PID-1101","file":"PID-1101.dwg","source":"open","pbs":"1001-01","objects":1,"error":null},"#,
            r#"{"drawing_id":"PID-1102","file":"PID-1102.dwg","source":"file","pbs":null,"objects":1,"error":null}],"#,
            r#""duplicates":[{"type_code":"EQUIPMENT","tag":"P-1001","places":[{"drawing_id":"PID-1101","at":[120.0,80.0],"handle":"2A4"},{"drawing_id":"PID-1102","at":[60.0,40.0],"handle":"1F0"}]}],"#,
            r#""links":[{"family":"pipe","from":{"drawing_id":"PID-1101","line":"L-1001","at":[240.0,80.0],"handle":"2C0"},"to":{"drawing_id":"PID-1102","line":"L-2001","at":[0.0,80.0],"handle":"1B3"}}],"#,
            r#""unresolved":[{"drawing_id":"PID-1101","kind":"pipe_out","line":"L-1003","at":[240.0,20.0],"handle":"2D0","status":"unpaired","target":null}]}"#
        );
        assert_eq!(serde_json::to_string(&index).expect("JSON"), expected);
        let back: ProjectIndex = serde_json::from_str(expected).expect("读回");
        assert_eq!(back, index);
    }
}

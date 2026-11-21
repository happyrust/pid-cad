//! 两份快照的比对（§3.5–§3.7）。谁和谁是同一个对象按键认，不看句柄；两边都有的逐项比，
//! 一边有、一边没有的算新增或删除；连通图的边按名字比，增删记在两头的对象上。纯函数，只看
//! 两份快照的摘要和连通图。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use super::Snapshot;
use crate::pid::project_index::{ObjectKind, PeerKind, SummaryConnector, SummaryObject};
use crate::pid::topology::ConnectorKind;

/// 比对结果格式的版本（§3.7 `version`）。
pub const DIFF_VERSION: u32 = 1;
/// 不传容差时用的容差（§3.6），图纸单位：坐标、长度差得不超过它不算变。
pub const DEFAULT_MOVE_TOLERANCE: f64 = 0.001;
/// 提示：两份快照的图号不同，不是同一张图。
pub const DRAWING_ID_DIFFERS: &str = "DRAWING_ID_DIFFERS";
/// 提示：两份快照的图纸单位不同，长度和位置是按原数比的，没有换算。
pub const LENGTH_UNIT_DIFFERS: &str = "LENGTH_UNIT_DIFFERS";

/// 页间连接符在结果里的种类，同规则检查。
const CONNECTOR: &str = "页间连接符";

/// 比对结果（§3.7），字段顺序就是 JSON 里的顺序。§3.7 例子开头的 `"ok": true` 由应答那一层加。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diff {
    pub version: u32,
    pub a: DiffSide,
    pub b: DiffSide,
    /// 新增、删除、修改和边的增删都没有。
    pub identical: bool,
    pub summary: DiffSummary,
    /// `b` 有而 `a` 没有的对象，位置和句柄取自 `b`。
    pub added: Vec<DiffObject>,
    /// `a` 有而 `b` 没有的对象，位置和句柄取自 `a`。
    pub removed: Vec<DiffObject>,
    /// 两边都有、有不同的对象，位置和句柄取自 `b`。
    pub modified: Vec<DiffObject>,
    pub edges: DiffEdges,
    pub notices: Vec<Notice>,
}

/// 比的是哪一份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffSide {
    /// 请求里怎么指的这一份：序号、文件路径或 `live`。[`diff`] 只知道序号，没有序号时空着，
    /// 由命令层按请求填。
    #[serde(rename = "ref")]
    pub reference: String,
    pub seq: Option<u64>,
    pub drawing_id: String,
    pub created_at: u64,
    /// 对象数。
    pub objects: usize,
}

/// 三组计数、各种变化的对象数和边的增删数（§3.7 `summary`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DiffSummary {
    #[serde(rename = "新增")]
    pub added: usize,
    #[serde(rename = "删除")]
    pub removed: usize,
    #[serde(rename = "修改")]
    pub modified: usize,
    pub by_what: ByWhat,
    pub edges_added: usize,
    pub edges_removed: usize,
}

/// 「修改」组里有某一种变化的对象各有多少个，六种都列。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ByWhat {
    #[serde(rename = "属性")]
    pub property: usize,
    #[serde(rename = "位置")]
    pub position: usize,
    #[serde(rename = "长度")]
    pub length: usize,
    #[serde(rename = "管件")]
    pub fittings: usize,
    #[serde(rename = "连接")]
    pub connection: usize,
    #[serde(rename = "对端")]
    pub peer: usize,
}

/// 结果里的一个对象。新增、删除的没有 `changes`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffObject {
    pub key: String,
    /// 种类：对象的 `kind`；页间连接符是「页间连接符」。
    pub kind: String,
    /// 类型编码：页间连接符是它那一族线的（`PIPE_LINE`、`SIGNAL_LINE`），同规则检查。
    pub type_code: String,
    /// 位号：页间连接符是它所在的线号。
    pub tag: String,
    pub at: [f64; 2],
    /// 只供在图上定位。
    pub handle: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
}

/// 两边都有的对象的一处不同（§3.6）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "what")]
pub enum Change {
    /// `props` 的一个键，或图层、符号、所属、覆盖标记；一边没有是 `None`。
    #[serde(rename = "属性")]
    Property {
        field: String,
        old: Option<String>,
        new: Option<String>,
    },
    /// 符号类对象和页间连接符的定位点，管线、管路、信号线的包围盒。
    #[serde(rename = "位置")]
    Position { old: Vec<f64>, new: Vec<f64> },
    /// 管路、管线名下线段的长度之和。
    #[serde(rename = "长度")]
    Length { old: f64, new: f64 },
    /// 管路上某种不单独编号的管件的个数。
    #[serde(rename = "管件")]
    Fittings {
        symbol: String,
        old: usize,
        new: usize,
    },
    /// 连通图里与这个对象有关的一条边。`with` 是另一头在图里的名字，`port` 是这一头的端口名。
    #[serde(rename = "连接")]
    Connection {
        change: EdgeChange,
        with: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        port: Option<String>,
    },
    /// 页间连接符的 `peer_kind`、`peer_drawing`、`upstream`、`downstream`。
    #[serde(rename = "对端")]
    Peer {
        field: String,
        old: Option<String>,
        new: Option<String>,
    },
}

/// 边是多了还是少了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum EdgeChange {
    #[serde(rename = "新增")]
    Added,
    #[serde(rename = "删除")]
    Removed,
}

/// 连通图里多了、少了的边，各自排好。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffEdges {
    pub added: Vec<[String; 2]>,
    pub removed: Vec<[String; 2]>,
}

/// 结果里的一条提示（§3.7 `notices`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub code: &'static str,
    pub message: String,
}

/// 比对两份快照（§3.5–§3.7）。「新增」是 `b` 有而 `a` 没有的。坐标、长度差得不超过
/// `move_tolerance`（图纸单位）不算变，负数和 NaN 当 0。同样的两份比两次，结果相同。
#[must_use]
pub fn diff(a: &Snapshot, b: &Snapshot, move_tolerance: f64) -> Diff {
    let tolerance = move_tolerance.max(0.0);
    let (old, new) = (Contents::of(a), Contents::of(b));
    let edges = DiffEdges {
        added: missing_from(&b.graph.edges, &a.graph.edges),
        removed: missing_from(&a.graph.edges, &b.graph.edges),
    };
    let mut links = links(&edges, a, b);
    let (mut added, mut removed, mut modified) = (Vec::new(), Vec::new(), Vec::new());
    let keys: BTreeSet<&String> = old.items.keys().chain(new.items.keys()).collect();
    for key in keys {
        let before = old.items.get(key).map_or(&[][..], Vec::as_slice);
        let after = new.items.get(key).map_or(&[][..], Vec::as_slice);
        for (index, (&was, &now)) in before.iter().zip(after).enumerate() {
            let mut changes = was.changes(now, tolerance);
            // 按位号归到名下的长度、管件和边分不出是重号的哪一个的，只记在第一对上。
            if index == 0 {
                changes.extend(totals(&old, &new, now, tolerance));
                changes.extend(links.remove(key.as_str()).unwrap_or_default());
            }
            if !changes.is_empty() {
                changes.sort_by(|x, y| x.order().cmp(&y.order()));
                modified.push(now.entry(key, changes));
            }
        }
        added.extend(
            after
                .iter()
                .skip(before.len())
                .map(|now| now.entry(key, Vec::new())),
        );
        removed.extend(
            before
                .iter()
                .skip(after.len())
                .map(|was| was.entry(key, Vec::new())),
        );
    }
    let identical = added.is_empty()
        && removed.is_empty()
        && modified.is_empty()
        && edges.added.is_empty()
        && edges.removed.is_empty();
    Diff {
        version: DIFF_VERSION,
        a: DiffSide::of(a),
        b: DiffSide::of(b),
        identical,
        summary: DiffSummary {
            added: added.len(),
            removed: removed.len(),
            modified: modified.len(),
            by_what: ByWhat::of(&modified),
            edges_added: edges.added.len(),
            edges_removed: edges.removed.len(),
        },
        added,
        removed,
        modified,
        edges,
        notices: notices(a, b),
    }
}

impl DiffSide {
    fn of(snapshot: &Snapshot) -> Self {
        Self {
            reference: snapshot.seq.map(|seq| seq.to_string()).unwrap_or_default(),
            seq: snapshot.seq,
            drawing_id: snapshot.drawing_id.clone(),
            created_at: snapshot.created_at,
            objects: snapshot.counts.objects,
        }
    }
}

impl ByWhat {
    fn of(modified: &[DiffObject]) -> Self {
        let mut counts = [0; 6];
        for object in modified {
            let mut seen = [false; 6];
            for change in &object.changes {
                seen[change.rank()] = true;
            }
            for (count, seen) in counts.iter_mut().zip(seen) {
                *count += usize::from(seen);
            }
        }
        let [property, position, length, fittings, connection, peer] = counts;
        Self {
            property,
            position,
            length,
            fittings,
            connection,
            peer,
        }
    }
}

impl Change {
    /// 第几种变化，照 §3.6 表里的先后。
    fn rank(&self) -> usize {
        match self {
            Self::Property { .. } => 0,
            Self::Position { .. } => 1,
            Self::Length { .. } => 2,
            Self::Fittings { .. } => 3,
            Self::Connection { .. } => 4,
            Self::Peer { .. } => 5,
        }
    }

    /// 排序用：先按种类，再按字段名、管件的符号或边的另一头，最后按端口名和增删。
    fn order(&self) -> (usize, &str, Option<&str>, Option<EdgeChange>) {
        let (name, port, change) = match self {
            Self::Property { field, .. } | Self::Peer { field, .. } => (field.as_str(), None, None),
            Self::Fittings { symbol, .. } => (symbol.as_str(), None, None),
            Self::Connection { change, with, port } => {
                (with.as_str(), port.as_deref(), Some(*change))
            }
            Self::Position { .. } | Self::Length { .. } => ("", None, None),
        };
        (self.rank(), name, port, change)
    }
}

/// 一份快照里按键认出来的东西，和按位号归到主人名下的线段长度、管件个数（§3.5）。
struct Contents<'s> {
    /// 键 → 这个键的东西，按坐标排好。
    items: BTreeMap<String, Vec<Item<'s>>>,
    /// 管路位号 → 名下管段的长度之和。
    run_lengths: HashMap<&'s str, f64>,
    /// 管线位号 → 它自己的线（不含管路的）长度之和。
    line_lengths: HashMap<&'s str, f64>,
    /// 管路位号 → 上面每种不单独编号的管件各有几个。
    fittings: HashMap<&'s str, BTreeMap<&'s str, usize>>,
}

impl<'s> Contents<'s> {
    fn of(snapshot: &'s Snapshot) -> Self {
        let summary = &snapshot.summary;
        let mut items: BTreeMap<String, Vec<Item<'s>>> = BTreeMap::new();
        let mut fittings: HashMap<&str, BTreeMap<&str, usize>> = HashMap::new();
        for object in &summary.objects {
            match (object.kind, object.own_tag) {
                (ObjectKind::Break, _) => {}
                (_, true) => items
                    .entry(object_key(object))
                    .or_default()
                    .push(Item::Object(object)),
                // 不单独编号的管件。不属于哪条管路的无处可记。
                (_, false) => {
                    if let Some(run) = object.owner.as_deref() {
                        let symbol = object.symbol.as_deref().unwrap_or(&object.type_code);
                        *fittings.entry(run).or_default().entry(symbol).or_default() += 1;
                    }
                }
            }
        }
        for connector in &summary.connectors {
            let key = format!("connector|{}|{}", connector.line, connector.kind);
            items
                .entry(key)
                .or_default()
                .push(Item::Connector(connector));
        }
        for list in items.values_mut() {
            list.sort_by(|x, y| by_place(x.at(), y.at()));
        }
        let mut run_lengths: HashMap<&str, f64> = HashMap::new();
        let mut line_lengths: HashMap<&str, f64> = HashMap::new();
        for segment in &summary.segments {
            let (lengths, owner) = match &segment.run {
                Some(run) => (&mut run_lengths, run.as_str()),
                None => (&mut line_lengths, segment.line.as_str()),
            };
            *lengths.entry(owner).or_insert(0.0) += segment.length;
        }
        Self {
            items,
            run_lengths,
            line_lengths,
            fittings,
        }
    }
}

/// 按键认出来的一个东西：摘要里一个有自己编号的对象，或一个页间连接符。
#[derive(Clone, Copy)]
enum Item<'s> {
    Object(&'s SummaryObject),
    Connector(&'s SummaryConnector),
}

impl Item<'_> {
    fn at(self) -> [f64; 2] {
        match self {
            Self::Object(object) => object.at,
            Self::Connector(connector) => connector.at,
        }
    }

    /// 与另一份里配上对的 `now` 逐项比（§3.6）：对象比属性和位置，连接符比位置和对端。
    fn changes(self, now: Self, tolerance: f64) -> Vec<Change> {
        let mut changes = Vec::new();
        match (self, now) {
            (Self::Object(was), Self::Object(now)) => {
                properties(was, now, &mut changes);
                let (old, new) = if spans(now.kind) {
                    (was.bbox.as_slice(), now.bbox.as_slice())
                } else {
                    (was.at.as_slice(), now.at.as_slice())
                };
                changes.extend(moved(old, new, tolerance));
            }
            (Self::Connector(was), Self::Connector(now)) => {
                changes.extend(moved(&was.at, &now.at, tolerance));
                for (field, old, new) in [
                    (
                        "peer_kind",
                        Some(peer_kind(was.peer_kind)),
                        Some(peer_kind(now.peer_kind)),
                    ),
                    (
                        "peer_drawing",
                        was.peer_drawing.as_deref(),
                        now.peer_drawing.as_deref(),
                    ),
                    ("upstream", was.upstream.as_deref(), now.upstream.as_deref()),
                    (
                        "downstream",
                        was.downstream.as_deref(),
                        now.downstream.as_deref(),
                    ),
                ] {
                    if old != new {
                        changes.push(Change::Peer {
                            field: field.to_owned(),
                            old: old.map(str::to_owned),
                            new: new.map(str::to_owned),
                        });
                    }
                }
            }
            // 类型编码写成 `connector` 的对象才会和连接符的键撞上，没有可比的。
            _ => {}
        }
        changes
    }

    /// 结果里的这一个。
    fn entry(self, key: &str, changes: Vec<Change>) -> DiffObject {
        let (kind, type_code, tag, at, handle) = match self {
            Self::Object(object) => (
                kind_name(object.kind),
                object.type_code.as_str(),
                &object.tag,
                object.at,
                &object.handle,
            ),
            Self::Connector(connector) => {
                let type_code = match ConnectorKind::parse(&connector.kind) {
                    Some(kind) => kind.type_code(),
                    None => connector.kind.as_str(),
                };
                (
                    CONNECTOR,
                    type_code,
                    &connector.line,
                    connector.at,
                    &connector.handle,
                )
            }
        };
        DiffObject {
            key: key.to_owned(),
            kind: kind.to_owned(),
            type_code: type_code.to_owned(),
            tag: tag.clone(),
            at,
            handle: handle.clone(),
            changes,
        }
    }
}

/// 有自己编号的对象的键（§3.5）：位号不空的是「类型编码|位号」，与连通图的节点名相同；
/// 位号为空的是「类型编码|所属|x,y」，坐标保留 3 位小数，与规则检查的对象键写法相同。
fn object_key(object: &SummaryObject) -> String {
    if object.tag.is_empty() {
        format!(
            "{}|{}|{:.3},{:.3}",
            object.type_code,
            object.owner.as_deref().unwrap_or_default(),
            object.at[0],
            object.at[1]
        )
    } else {
        format!("{}|{}", object.type_code, object.tag)
    }
}

/// 先按 x 再按 y。
fn by_place(a: [f64; 2], b: [f64; 2]) -> Ordering {
    a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
}

/// 属性（§3.6）：`props` 的每个键，另加图层、符号、所属、覆盖标记四项。
fn properties(was: &SummaryObject, now: &SummaryObject, changes: &mut Vec<Change>) {
    let fields: BTreeSet<&String> = was.props.keys().chain(now.props.keys()).collect();
    for field in fields {
        let (old, new) = (was.props.get(field), now.props.get(field));
        if old != new {
            changes.push(Change::Property {
                field: field.clone(),
                old: old.cloned(),
                new: new.cloned(),
            });
        }
    }
    let (old_marks, new_marks) = (marks(was), marks(now));
    for (field, old, new) in [
        ("图层", Some(was.layer.as_str()), Some(now.layer.as_str())),
        ("符号", was.symbol.as_deref(), now.symbol.as_deref()),
        ("所属", was.owner.as_deref(), now.owner.as_deref()),
        ("覆盖标记", old_marks.as_deref(), new_marks.as_deref()),
    ] {
        if old != new {
            changes.push(Change::Property {
                field: field.to_owned(),
                old: old.map(str::to_owned),
                new: new.map(str::to_owned),
            });
        }
    }
}

/// 覆盖标记的字段名用顿号连起来；一个都没有是 `None`。
fn marks(object: &SummaryObject) -> Option<String> {
    (!object.overrides.is_empty()).then(|| object.overrides.join("、"))
}

/// 管线、管路、信号线比包围盒，其余比定位点（§3.6）。
fn spans(kind: ObjectKind) -> bool {
    matches!(
        kind,
        ObjectKind::PipeLine | ObjectKind::PipeRun | ObjectKind::SignalLine
    )
}

/// 任一坐标差得超过容差就是挪了。
fn moved(old: &[f64], new: &[f64], tolerance: f64) -> Option<Change> {
    old.iter()
        .zip(new)
        .any(|(&old, &new)| beyond(old, new, tolerance))
        .then(|| Change::Position {
            old: old.to_vec(),
            new: new.to_vec(),
        })
}

/// `old` 和 `new` 差得超过 `tolerance`。差恰好等于容差不算（§3.6）是十进制的说法：
/// 双精度里 7.001 + 7 − 14 比 0.001 大一点，所以按两个数的大小放宽几个最小单位。
fn beyond(old: f64, new: f64, tolerance: f64) -> bool {
    let slack = 4.0 * f64::EPSILON * old.abs().max(new.abs()).max(1.0);
    (old - new).abs() > tolerance + slack
}

/// 按位号归到主人名下的两项（§3.5）：管路、管线的线段长度之和，管路上每种管件的个数。
/// 位号为空的认不出名下有哪些，不比。
fn totals(old: &Contents<'_>, new: &Contents<'_>, item: Item<'_>, tolerance: f64) -> Vec<Change> {
    let mut changes = Vec::new();
    let Item::Object(object) = item else {
        return changes;
    };
    let tag = object.tag.as_str();
    if tag.is_empty() {
        return changes;
    }
    let lengths = match object.kind {
        ObjectKind::PipeRun => Some((&old.run_lengths, &new.run_lengths)),
        ObjectKind::PipeLine => Some((&old.line_lengths, &new.line_lengths)),
        _ => None,
    };
    if let Some((before, after)) = lengths {
        let was = before.get(tag).copied().unwrap_or(0.0);
        let now = after.get(tag).copied().unwrap_or(0.0);
        if beyond(was, now, tolerance) {
            changes.push(Change::Length { old: was, new: now });
        }
    }
    if object.kind == ObjectKind::PipeRun {
        let none = BTreeMap::new();
        let before = old.fittings.get(tag).unwrap_or(&none);
        let after = new.fittings.get(tag).unwrap_or(&none);
        let symbols: BTreeSet<&str> = before.keys().chain(after.keys()).copied().collect();
        for symbol in symbols {
            let was = before.get(symbol).copied().unwrap_or(0);
            let now = after.get(symbol).copied().unwrap_or(0);
            if was != now {
                changes.push(Change::Fittings {
                    symbol: symbol.to_owned(),
                    old: was,
                    new: now,
                });
            }
        }
    }
    changes
}

/// `edges` 里有、`other` 里没有的边，排好。
fn missing_from(edges: &[[String; 2]], other: &[[String; 2]]) -> Vec<[String; 2]> {
    let other: HashSet<&[String; 2]> = other.iter().collect();
    let missing: BTreeSet<&[String; 2]> =
        edges.iter().filter(|edge| !other.contains(edge)).collect();
    missing.into_iter().cloned().collect()
}

/// 增删的边记到两头的对象上（§3.6），按对象的键归好，接到端口的那一头带端口名。一头是
/// 新增或删除的对象时，它那一份配不上对，用不到，只记在另一头。
fn links(edges: &DiffEdges, a: &Snapshot, b: &Snapshot) -> HashMap<String, Vec<Change>> {
    let mut links: HashMap<String, Vec<Change>> = HashMap::new();
    for (list, change, nodes) in [
        (&edges.added, EdgeChange::Added, &b.graph.nodes),
        (&edges.removed, EdgeChange::Removed, &a.graph.nodes),
    ] {
        let nodes: HashSet<&str> = nodes.iter().map(String::as_str).collect();
        for [one, other] in list {
            for (end, with) in [(one, other), (other, one)] {
                let (key, port) = split_end(end, &nodes);
                links
                    .entry(key.to_owned())
                    .or_default()
                    .push(Change::Connection {
                        change,
                        with: with.clone(),
                        port: port.map(str::to_owned),
                    });
            }
        }
    }
    links
}

/// 边的一头在图里的名字拆成对象的键和端口名：接到端口的是「键#端口名」（项目索引规格 §3.3）。位号、
/// 端口名里也可能有 `#`（符号库里没有的块，端口记成 `#1`），所以整个名字不是节点时，从左往右
/// 找第一个前半截是节点的 `#`。
fn split_end<'e>(end: &'e str, nodes: &HashSet<&str>) -> (&'e str, Option<&'e str>) {
    if nodes.contains(end) {
        return (end, None);
    }
    end.match_indices('#')
        .map(|(at, _)| (&end[..at], &end[at + 1..]))
        .find(|(node, _)| nodes.contains(node))
        .map_or((end, None), |(node, port)| (node, Some(port)))
}

/// 两份不是同一张图、图纸单位不同时的提示（§3.7）。
fn notices(a: &Snapshot, b: &Snapshot) -> Vec<Notice> {
    let mut notices = Vec::new();
    if a.drawing_id != b.drawing_id {
        notices.push(Notice {
            code: DRAWING_ID_DIFFERS,
            message: format!(
                "两份快照不是同一张图：「{}」和「{}」",
                a.drawing_id, b.drawing_id
            ),
        });
    }
    let (old, new) = (&a.summary.length_unit, &b.summary.length_unit);
    if old != new {
        notices.push(Notice {
            code: LENGTH_UNIT_DIFFERS,
            message: format!("两份快照的图纸单位不同：「{old}」和「{new}」，长度和位置没有换算"),
        });
    }
    notices
}

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

/// `peer_kind` 在摘要里的写法。
fn peer_kind(kind: PeerKind) -> &'static str {
    match kind {
        PeerKind::None => "none",
        PeerKind::Target => "target",
        PeerKind::Server => "server",
        PeerKind::Local => "local",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;
    use crate::pid::project_index::{graph, DrawingSummary, Joint, SummarySegment};
    use crate::pid::snapshot::capture;
    use crate::pid::snapshot::tests::{drawing, meta};

    /// 一组合成图（20 个对象）的摘要：三台设备；进口、出口、排净三条管路上四个阀门，
    /// 出口管路上还有一个不单独编号的管件和一个打断符；两条管线；四块仪表；两条信号线；
    /// 三个流出连接符。
    fn base() -> DrawingSummary {
        drawing(20).0
    }

    /// 由摘要现算连通图，做一份快照。
    fn snap(summary: &DrawingSummary) -> Snapshot {
        capture(summary.clone(), graph(summary), meta("")).expect("能做快照")
    }

    /// 有自己编号、位号是 `tag` 的对象。
    fn object<'s>(summary: &'s mut DrawingSummary, tag: &str) -> &'s mut SummaryObject {
        summary
            .objects
            .iter_mut()
            .find(|object| object.own_tag && object.tag == tag)
            .unwrap_or_else(|| panic!("合成图里有 {tag}"))
    }

    /// 位号是 `tag` 的阀门所在的管路。
    fn run_of(summary: &mut DrawingSummary, valve: &str) -> String {
        object(summary, valve)
            .owner
            .clone()
            .unwrap_or_else(|| panic!("{valve} 有所属管路"))
    }

    /// 从 `a` 起的那根线段。
    fn segment_from(summary: &mut DrawingSummary, a: [f64; 2]) -> &mut SummarySegment {
        summary
            .segments
            .iter_mut()
            .find(|segment| segment.a == a)
            .unwrap_or_else(|| panic!("合成图里有从 {a:?} 起的线段"))
    }

    /// 信号线 S-2001 接在位号是 `tag` 的仪表上的端口名。
    fn signal_port(summary: &DrawingSummary, tag: &str) -> String {
        summary
            .signal_ends
            .iter()
            .find_map(|end| match &end.joint {
                Some(Joint::Port { tag: to, port, .. }) if end.tag == "S-2001" && to == tag => {
                    Some(port.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("S-2001 接着 {tag}"))
    }

    fn keys(objects: &[DiffObject]) -> Vec<&str> {
        objects.iter().map(|object| object.key.as_str()).collect()
    }

    /// 「修改」组里键是 `key` 的对象的变化。
    fn changes<'d>(result: &'d Diff, key: &str) -> &'d [Change] {
        &result
            .modified
            .iter()
            .find(|object| object.key == key)
            .unwrap_or_else(|| panic!("{key} 在「修改」组里：{result:#?}"))
            .changes
    }

    fn peer(field: &str, old: Option<&str>, new: Option<&str>) -> Change {
        Change::Peer {
            field: field.to_owned(),
            old: old.map(str::to_owned),
            new: new.map(str::to_owned),
        }
    }

    /// §6 第 6 条的五处变化：往出口管路上插一个闸阀 V-1005，删掉压力变送器 PT-1001，泵
    /// P-1002 的流量改了，进料容器 P-1001 往右挪 5，信号线 S-2001 接流量控制器的那一头断开。
    fn five_changes() -> (DrawingSummary, DrawingSummary) {
        let before = base();
        let mut after = before.clone();
        let outlet = run_of(&mut after, "V-1002");
        let mut valve = object(&mut after, "V-1001").clone();
        valve.tag = "V-1005".to_owned();
        valve.owner = Some(outlet);
        valve.at = [296.0, 0.0];
        valve.bbox = [282.0, -7.0, 310.0, 7.0];
        valve.handle = "FFF1".to_owned();
        let port = |name: &str| {
            Some(Joint::Fitting {
                type_code: valve.type_code.clone(),
                tag: valve.tag.clone(),
                port: name.to_owned(),
            })
        };
        // 最后一段 (272, 0)–(320, 0) 从中间断开，两头收在阀门两侧的 (280, 0) 和 (312, 0)。
        let last = segment_from(&mut after, [272.0, 0.0]);
        let tail = SummarySegment {
            a: [312.0, 0.0],
            length: 8.0,
            joint_a: port("连接点2"),
            handle: "FFF2".to_owned(),
            ..last.clone()
        };
        last.b = [280.0, 0.0];
        last.length = 8.0;
        last.joint_b = port("连接点1");
        after.segments.push(tail);
        after.objects.push(valve);
        after.objects.retain(|object| object.tag != "PT-1001");
        object(&mut after, "P-1002")
            .props
            .insert("流量".to_owned(), "25m3/h".to_owned());
        let vessel = object(&mut after, "P-1001");
        vessel.at[0] += 5.0;
        vessel.bbox[0] += 5.0;
        vessel.bbox[2] += 5.0;
        for end in &mut after.signal_ends {
            if end.tag == "S-2001"
                && matches!(&end.joint, Some(Joint::Port { tag, .. }) if tag == "FIC-1001")
            {
                end.joint = None;
            }
        }
        (before, after)
    }

    /// 同一张图做两份快照（时间、原因、序号都不同），比对无差异：三组计数、六种变化、
    /// 边的增删全为 0，各列表都空，没有提示。两边的句柄全换掉也一样：比对不看句柄
    /// （§6 第 7 条）。
    #[test]
    fn the_same_drawing_twice_is_identical() {
        let summary = base();
        let mut a = snap(&summary);
        a.seq = Some(2);
        a.created_at = 1_794_750_000;
        let b = capture(summary.clone(), graph(&summary), meta("送审前")).expect("能做快照");
        let result = diff(&a, &b, DEFAULT_MOVE_TOLERANCE);
        assert!(result.identical, "{result:#?}");
        assert_eq!(
            serde_json::to_value(&result).expect("能写成 JSON"),
            json!({
                "version": 1,
                "a": { "ref": "2", "seq": 2, "drawing_id": "PID-1101", "created_at": 1_794_750_000, "objects": 20 },
                "b": { "ref": "", "seq": null, "drawing_id": "PID-1101", "created_at": 1_794_800_000, "objects": 20 },
                "identical": true,
                "summary": {
                    "新增": 0, "删除": 0, "修改": 0,
                    "by_what": { "属性": 0, "位置": 0, "长度": 0, "管件": 0, "连接": 0, "对端": 0 },
                    "edges_added": 0, "edges_removed": 0
                },
                "added": [],
                "removed": [],
                "modified": [],
                "edges": { "added": [], "removed": [] },
                "notices": []
            })
        );

        let mut renamed = summary;
        for (n, object) in renamed.objects.iter_mut().enumerate() {
            object.handle = format!("F{n:03X}");
        }
        for (n, segment) in renamed.segments.iter_mut().enumerate() {
            segment.handle = format!("E{n:03X}");
        }
        for (n, connector) in renamed.connectors.iter_mut().enumerate() {
            connector.handle = format!("D{n:03X}");
        }
        let result = diff(&a, &snap(&renamed), DEFAULT_MOVE_TOLERANCE);
        assert!(result.identical, "{result:#?}");
    }

    /// §6 第 6 条：新增一个阀门、删一块仪表、改一个属性、挪一台设备、改一处连接，各在各的
    /// 组里，属性变化带旧值和新值。阀门插在管路上：阀门在「新增」，管路在「修改」，带两条
    /// 「连接」新增（另有变短了的长度）。边的增删记在两头，一头是新增或删除的对象时只记另
    /// 一头：变送器删了，它和 S-2001 之间的边只记在 S-2001 上。整份结果照 §3.7 的样子。
    #[test]
    fn five_kinds_of_change_land_in_their_groups() {
        let (mut before, mut after) = five_changes();
        let result = diff(&snap(&before), &snap(&after), DEFAULT_MOVE_TOLERANCE);

        let outlet = run_of(&mut before, "V-1002");
        let run = format!("PIPE_RUN|{outlet}");
        let length: f64 = before
            .segments
            .iter()
            .filter(|segment| segment.run.as_ref() == Some(&outlet))
            .map(|segment| segment.length)
            .sum();
        let valve = format!("{}|V-1005", object(&mut after, "V-1005").type_code);
        let fic_port = signal_port(&before, "FIC-1001");
        let controller = format!("INSTRUMENT|FIC-1001#{fic_port}");
        let transmitter = format!("INSTRUMENT|PT-1001#{}", signal_port(&before, "PT-1001"));
        let entry = |object: &SummaryObject, kind: &str| {
            json!({
                "key": format!("{}|{}", object.type_code, object.tag),
                "kind": kind,
                "type_code": object.type_code,
                "tag": object.tag,
                "at": object.at,
                "handle": object.handle
            })
        };
        let changed = |object: &SummaryObject, kind: &str, changes: Value| {
            let mut value = entry(object, kind);
            value["changes"] = changes;
            value
        };
        let value = serde_json::to_value(&result).expect("能写成 JSON");
        assert_eq!(
            value,
            json!({
                "version": 1,
                "a": { "ref": "", "seq": null, "drawing_id": "PID-1101", "created_at": 1_794_800_000, "objects": 20 },
                "b": { "ref": "", "seq": null, "drawing_id": "PID-1101", "created_at": 1_794_800_000, "objects": 20 },
                "identical": false,
                "summary": {
                    "新增": 1, "删除": 1, "修改": 5,
                    "by_what": { "属性": 1, "位置": 1, "长度": 1, "管件": 0, "连接": 3, "对端": 0 },
                    "edges_added": 2, "edges_removed": 2
                },
                "added": [entry(object(&mut after, "V-1005"), "阀门")],
                "removed": [entry(object(&mut before, "PT-1001"), "仪表")],
                "modified": [
                    changed(object(&mut after, "P-1001"), "设备", json!([
                        { "what": "位置", "old": [0.0, -12.0], "new": [5.0, -12.0] }
                    ])),
                    changed(object(&mut after, "P-1002"), "设备", json!([
                        { "what": "属性", "field": "流量", "old": "20m3/h", "new": "25m3/h" }
                    ])),
                    changed(object(&mut after, "FIC-1001"), "仪表", json!([
                        { "what": "连接", "change": "删除", "with": "SIGNAL_LINE|S-2001", "port": fic_port }
                    ])),
                    changed(object(&mut after, &outlet), "管路", json!([
                        { "what": "长度", "old": length, "new": length - 32.0 },
                        { "what": "连接", "change": "新增", "with": format!("{valve}#连接点1") },
                        { "what": "连接", "change": "新增", "with": format!("{valve}#连接点2") }
                    ])),
                    changed(object(&mut after, "S-2001"), "信号线", json!([
                        { "what": "连接", "change": "删除", "with": controller },
                        { "what": "连接", "change": "删除", "with": transmitter }
                    ]))
                ],
                "edges": {
                    "added": [[run, format!("{valve}#连接点1")], [run, format!("{valve}#连接点2")]],
                    "removed": [[controller, "SIGNAL_LINE|S-2001"], [transmitter, "SIGNAL_LINE|S-2001"]]
                },
                "notices": []
            })
        );
        // JSON 里字段的先后照 §3.7：整份的、新增对象的（没有 `changes`）、一处变化的。
        let order = |value: &Value| -> Vec<String> {
            value
                .as_object()
                .expect("JSON 对象")
                .keys()
                .cloned()
                .collect()
        };
        assert_eq!(
            order(&value),
            [
                "version",
                "a",
                "b",
                "identical",
                "summary",
                "added",
                "removed",
                "modified",
                "edges",
                "notices"
            ]
        );
        assert_eq!(
            order(&value["added"][0]),
            ["key", "kind", "type_code", "tag", "at", "handle"]
        );
        assert_eq!(
            serde_json::to_string(&value["modified"][2]["changes"]).expect("能写成 JSON"),
            format!(
                r#"[{{"what":"连接","change":"删除","with":"SIGNAL_LINE|S-2001","port":"{fic_port}"}}]"#
            )
        );
    }

    /// §3.5 的每种键。位号为空的对象按「类型编码|所属|x,y」认，坐标保留 3 位小数：挪得键
    /// 变了就报一删一增；挪得少、键没变，又没超过容差，不算变。页间连接符按
    /// 「connector|线号|种类」认，比位置和对端。不单独编号的管件不逐个比，算管路上这种符号的个数；
    /// 打断符不比。线段算主人的长度：管路比名下的管段，管线比它自己的线。
    #[test]
    fn each_kind_of_key_from_section_3_5() {
        let mut before = base();
        let position = |summary: &DrawingSummary, tag: &str| {
            summary
                .objects
                .iter()
                .position(|object| object.tag == tag)
                .unwrap_or_else(|| panic!("合成图里有 {tag}"))
        };
        let thermometer = position(&before, "TI-1001");
        let gauge = position(&before, "PI-1001");
        before.objects[thermometer].tag.clear();
        before.objects[gauge].tag.clear();
        let inlet = run_of(&mut before, "V-1001");
        let outlet = run_of(&mut before, "V-1002");
        let mut after = before.clone();
        after.objects[thermometer].at[0] += 0.0001;
        after.objects[gauge].at[0] += 0.4;
        // 连接符：L-1001 的配上了对，L-1002 的挪了。
        let plug = |summary: &mut DrawingSummary, line: &str| -> usize {
            summary
                .connectors
                .iter()
                .position(|connector| connector.line == line)
                .unwrap_or_else(|| panic!("{line} 上有连接符"))
        };
        let main = plug(&mut after, "L-1001");
        after.connectors[main].peer_kind = PeerKind::Server;
        after.connectors[main].peer_drawing = Some("PID-1102".to_owned());
        after.connectors[main].downstream = Some("L-2001".to_owned());
        let branch = plug(&mut after, "L-1002");
        after.connectors[branch].at[1] += 1.0;
        // 出口管路上多一个不单独编号的管件；原有那个的属性改了、打断符挪了，都不比。
        let fitting = after
            .objects
            .iter()
            .position(|object| object.kind == ObjectKind::Fitting)
            .expect("合成图里有管件");
        let symbol = after.objects[fitting].symbol.clone().expect("管件有符号");
        let another = SummaryObject {
            at: [180.0, 0.0],
            handle: "FFF4".to_owned(),
            ..after.objects[fitting].clone()
        };
        after.objects[fitting]
            .props
            .insert("备注".to_owned(), "换过".to_owned());
        after.objects.push(another);
        let mark = after
            .objects
            .iter_mut()
            .find(|object| object.kind == ObjectKind::Break)
            .expect("合成图里有打断符");
        mark.at[0] += 1.0;
        mark.bbox[0] += 1.0;
        mark.bbox[2] += 1.0;
        // 管线 L-1001 自己的线长了 10，进口管路第一段长了 2。
        let own = segment_from(&mut after, [320.0, 24.0]);
        own.b[0] += 10.0;
        own.length += 10.0;
        let first = segment_from(&mut after, [0.0, 0.0]);
        first.b[0] += 2.0;
        first.length += 2.0;

        let result = diff(&snap(&before), &snap(&after), DEFAULT_MOVE_TOLERANCE);
        assert_eq!(keys(&result.added), ["INSTRUMENT||60.400,40.000"]);
        assert_eq!(keys(&result.removed), ["INSTRUMENT||60.000,40.000"]);
        let inlet = format!("PIPE_RUN|{inlet}");
        let outlet = format!("PIPE_RUN|{outlet}");
        assert_eq!(
            keys(&result.modified),
            [
                "PIPE_LINE|L-1001",
                &inlet,
                &outlet,
                "connector|L-1001|pipe_out",
                "connector|L-1002|pipe_out"
            ]
        );
        assert_eq!(
            changes(&result, "PIPE_LINE|L-1001"),
            [Change::Length {
                old: 60.0,
                new: 70.0
            }]
        );
        assert_eq!(
            changes(&result, &inlet),
            [Change::Length {
                old: 14.0,
                new: 16.0
            }]
        );
        assert_eq!(
            changes(&result, &outlet),
            [Change::Fittings {
                symbol,
                old: 1,
                new: 2
            }]
        );
        assert_eq!(
            changes(&result, "connector|L-1001|pipe_out"),
            [
                peer("downstream", None, Some("L-2001")),
                peer("peer_drawing", None, Some("PID-1102")),
                peer("peer_kind", Some("none"), Some("server"))
            ]
        );
        assert_eq!(
            changes(&result, "connector|L-1002|pipe_out"),
            [Change::Position {
                old: vec![260.0, -40.0],
                new: vec![260.0, -39.0]
            }]
        );
        // 页间连接符在结果里：种类「页间连接符」，类型编码是它那一族线的，位号是所在的线号。
        let connector = &result.modified[3];
        assert_eq!(
            (
                connector.kind.as_str(),
                connector.type_code.as_str(),
                connector.tag.as_str(),
                connector.handle.as_str()
            ),
            (
                "页间连接符",
                "PIPE_LINE",
                "L-1001",
                after.connectors[main].handle.as_str()
            )
        );
    }

    /// 同一个键出现多次（重号，同一条线上多个同种连接符）：两边各自按坐标排好，逐个
    /// 配对，多出来的算新增或删除。
    #[test]
    fn duplicate_keys_pair_up_in_order_of_position() {
        let mut before = base();
        let pump = object(&mut before, "P-1002").clone();
        let plug = before
            .connectors
            .iter()
            .find(|connector| connector.line == "L-1002")
            .expect("L-1002 上有连接符")
            .clone();
        before.objects.push(SummaryObject {
            at: [460.0, 0.0],
            handle: "FFF5".to_owned(),
            ..pump.clone()
        });
        for (x, handle) in [(300.0, "FFF6"), (340.0, "FFF7")] {
            before.connectors.push(SummaryConnector {
                at: [x, -40.0],
                handle: handle.to_owned(),
                ..plug.clone()
            });
        }
        let mut after = before.clone();
        after.objects.push(SummaryObject {
            at: [560.0, 0.0],
            handle: "FFF8".to_owned(),
            ..pump
        });
        after
            .connectors
            .retain(|connector| connector.handle != "FFF7");
        for connector in &mut after.connectors {
            if connector.handle == "FFF6" {
                connector.at[1] -= 1.0;
            }
        }

        let result = diff(&snap(&before), &snap(&after), DEFAULT_MOVE_TOLERANCE);
        let places = |objects: &[DiffObject]| -> Vec<(String, [f64; 2], String)> {
            objects
                .iter()
                .map(|object| (object.key.clone(), object.at, object.handle.clone()))
                .collect()
        };
        let place = |key: &str, at: [f64; 2], handle: &str| (key.to_owned(), at, handle.to_owned());
        assert_eq!(
            places(&result.added),
            [place("EQUIPMENT|P-1002", [560.0, 0.0], "FFF8")]
        );
        assert_eq!(
            places(&result.removed),
            [place("connector|L-1002|pipe_out", [340.0, -40.0], "FFF7")]
        );
        assert_eq!(
            places(&result.modified),
            [place("connector|L-1002|pipe_out", [300.0, -41.0], "FFF6")]
        );
        assert_eq!(
            result.modified[0].changes,
            [Change::Position {
                old: vec![300.0, -40.0],
                new: vec![300.0, -41.0]
            }]
        );
    }

    /// 容差边界（§3.6）：差恰好等于容差不算变，超过了才算，坐标和长度都是。十进制写的数
    /// 照十进制算：泵从 60 挪到 60.001、默认容差 0.001，不算挪了。容差是负数或 NaN 时当 0：
    /// 一样的不算变，差一点都算。
    #[test]
    fn a_difference_equal_to_the_tolerance_is_no_change() {
        let before = base();
        let a = snap(&before);
        let pump_moved = |dx: f64| {
            let mut after = before.clone();
            object(&mut after, "P-1002").at[0] += dx;
            snap(&after)
        };
        let first_longer = |dl: f64| {
            let mut after = before.clone();
            let first = segment_from(&mut after, [0.0, 0.0]);
            first.b[0] += dl;
            first.length += dl;
            snap(&after)
        };
        for (what, b, tolerance, changed) in [
            ("挪 0.25", pump_moved(0.25), 0.25, false),
            ("挪 -0.25", pump_moved(-0.25), 0.25, false),
            ("挪 0.5", pump_moved(0.5), 0.25, true),
            ("挪 0.001", pump_moved(0.001), DEFAULT_MOVE_TOLERANCE, false),
            (
                "挪 0.0011",
                pump_moved(0.0011),
                DEFAULT_MOVE_TOLERANCE,
                true,
            ),
            ("长 0.25", first_longer(0.25), 0.25, false),
            ("长 0.5", first_longer(0.5), 0.25, true),
            (
                "长 0.001",
                first_longer(0.001),
                DEFAULT_MOVE_TOLERANCE,
                false,
            ),
            (
                "长 0.0011",
                first_longer(0.0011),
                DEFAULT_MOVE_TOLERANCE,
                true,
            ),
            ("不动", snap(&before), -1.0, false),
            ("不动", snap(&before), f64::NAN, false),
            ("挪 1e-9", pump_moved(1e-9), -1.0, true),
            ("挪 1e-9", pump_moved(1e-9), f64::NAN, true),
        ] {
            let result = diff(&a, &b, tolerance);
            assert_eq!(
                result.summary.modified,
                usize::from(changed),
                "{what}，容差 {tolerance}：{result:#?}"
            );
            assert_eq!(result.identical, !changed, "{what}，容差 {tolerance}");
        }
    }

    /// 两份不是同一张图时提示 `DRAWING_ID_DIFFERS`，图纸单位不同时提示 `LENGTH_UNIT_DIFFERS`；
    /// 内容照样比。
    #[test]
    fn notices_say_when_the_drawing_or_its_unit_differs() {
        let before = base();
        let mut after = before.clone();
        after.drawing_id = "PID-1102".to_owned();
        let result = diff(&snap(&before), &snap(&after), DEFAULT_MOVE_TOLERANCE);
        assert!(result.identical, "{result:#?}");
        assert_eq!(result.b.drawing_id, "PID-1102");
        assert_eq!(
            serde_json::to_value(&result.notices).expect("能写成 JSON"),
            json!([{
                "code": "DRAWING_ID_DIFFERS",
                "message": "两份快照不是同一张图：「PID-1101」和「PID-1102」"
            }])
        );

        after.length_unit = format!("{}-2", before.length_unit);
        object(&mut after, "P-1002")
            .props
            .insert("流量".to_owned(), "25m3/h".to_owned());
        let result = diff(&snap(&before), &snap(&after), DEFAULT_MOVE_TOLERANCE);
        let codes: Vec<&str> = result.notices.iter().map(|notice| notice.code).collect();
        assert_eq!(codes, [DRAWING_ID_DIFFERS, LENGTH_UNIT_DIFFERS]);
        assert_eq!(
            result.notices[1].message,
            format!(
                "两份快照的图纸单位不同：「{}」和「{}」，长度和位置没有换算",
                before.length_unit, after.length_unit
            )
        );
        assert_eq!(keys(&result.modified), ["EQUIPMENT|P-1002"]);
    }

    /// 同样两份快照比两次，结果相同，写成 JSON 也逐字节相同；摘要、连通图里各列表的先后
    /// 倒过来也一样：各列表按键排，`changes` 按种类和字段排。
    #[test]
    fn the_result_does_not_depend_on_order() {
        let (before, after) = five_changes();
        let (a, b) = (snap(&before), snap(&after));
        let first = diff(&a, &b, DEFAULT_MOVE_TOLERANCE);
        let again = diff(&a, &b, DEFAULT_MOVE_TOLERANCE);
        assert_eq!(first, again);
        assert_eq!(
            serde_json::to_vec(&first).expect("能写成 JSON"),
            serde_json::to_vec(&again).expect("能写成 JSON")
        );
        assert_eq!(
            (first.modified.len(), first.edges.added.len()),
            (5, 2),
            "{first:#?}"
        );

        let mut reversed = b.clone();
        reversed.summary.objects.reverse();
        reversed.summary.segments.reverse();
        reversed.summary.signal_ends.reverse();
        reversed.summary.connectors.reverse();
        reversed.graph.nodes.reverse();
        reversed.graph.edges.reverse();
        assert_eq!(diff(&a, &reversed, DEFAULT_MOVE_TOLERANCE), first);
    }
}

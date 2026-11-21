//! 快照（SPEC-pid-snapshot）：把一张图某一刻的摘要和连通图，连同什么时候、谁、为什么
//! 做的，存成一份自带校验和的 JSON（§3.2）。
//!
//! 这里管内容和校验：[`capture`] 由摘要、连通图和 [`SnapshotMeta`] 做出一份快照，
//! [`Snapshot::to_bytes`] 写成文件内容，[`parse`] 读回来并重算校验和；两份快照怎么比
//! （[`diff`]）写在 `compare.rs` 里。都是纯函数；存到哪个目录、怎么编号在 [`store`] 里。
//!
//! 校验和只算正文：`summary`、`graph` 两个值各自按键名排序、不带空白写出来，首尾相接
//! 算 SHA-256。所以文件缩进与否、键的先后都不影响它，正文里任何一个值变了都对不上。
//! 小数要原样读回来才算得对：`serde_json` 的 `float_roundtrip` 是依赖 `jsonschema`
//! 打开的，测试 `numbers_come_back_exactly` 盯着它。

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::project_index::{DrawingSummary, Graph};

mod compare;
pub mod store;

pub use compare::{
    diff, ByWhat, Change, Diff, DiffEdges, DiffObject, DiffSide, DiffSummary, EdgeChange, Notice,
    DEFAULT_MOVE_TOLERANCE, DIFF_VERSION, DRAWING_ID_DIFFERS, LENGTH_UNIT_DIFFERS,
};

/// 快照文件格式的版本。读到更高的版本报 [`SNAPSHOT_VERSION_UNSUPPORTED`]。
pub const SNAPSHOT_VERSION: u32 = 1;
/// 快照文件 `kind` 字段的值，用来认文件。
pub const SNAPSHOT_KIND: &str = "pid-snapshot";
/// 原因最多这么多个字（按字符数，一个汉字、一个字母都算一个）。
pub const MAX_REASON_CHARS: usize = 200;

/// 正文与校验和对不上，或者 `counts`、`drawing_id` 与正文对不上：文件被改过或者坏了。
pub const SNAPSHOT_CORRUPT: &str = "SNAPSHOT_CORRUPT";
/// 不是快照文件：JSON 坏了、缺字段、字段类型不对、`kind` 不对。
pub const SNAPSHOT_FORMAT_INVALID: &str = "SNAPSHOT_FORMAT_INVALID";
/// 快照文件的 `version` 比这个程序认识的高。
pub const SNAPSHOT_VERSION_UNSUPPORTED: &str = "SNAPSHOT_VERSION_UNSUPPORTED";
/// 原因超过 [`MAX_REASON_CHARS`] 个字。
pub const SNAPSHOT_REASON_TOO_LONG: &str = "SNAPSHOT_REASON_TOO_LONG";
/// 往已有的快照文件写：序号已经用过，或者要写的文件已经存在。快照文件只新建、不覆盖。
pub const SNAPSHOT_IMMUTABLE: &str = "SNAPSHOT_IMMUTABLE";
/// 要读的序号或文件不存在。
pub const SNAPSHOT_NOT_FOUND: &str = "SNAPSHOT_NOT_FOUND";
/// 读写快照文件时文件系统报的错（§3.8 没有这个码）。
pub const SNAPSHOT_IO_ERROR: &str = "SNAPSHOT_IO_ERROR";

/// 文件开头的 UTF-8 BOM。读的时候不算错：Windows PowerShell 5.1 存 UTF-8 文件时会加上。
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// 做快照的人给的几项（§3.2）；其余各项由摘要和连通图算出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// 做快照的时间，Unix 秒。
    pub created_at: u64,
    /// 生成人：连着协同时是协同的用户号，否则是操作系统的账户名。
    pub created_by: String,
    /// 原因，可以空，最多 [`MAX_REASON_CHARS`] 个字。
    pub reason: String,
    /// 程序名和版本。
    pub generator: String,
    /// 做快照时图里有没有未保存的修改。
    pub dirty: bool,
}

/// 各类条数（§3.2 `counts`），列表里不用打开正文就能显示。前四项是摘要里 `objects`、
/// `segments`、`connectors`、`ports` 的条数，`edges` 是连通图的边数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub objects: usize,
    pub segments: usize,
    pub connectors: usize,
    pub ports: usize,
    pub edges: usize,
}

impl Counts {
    fn of(summary: &DrawingSummary, graph: &Graph) -> Self {
        Self {
            objects: summary.objects.len(),
            segments: summary.segments.len(),
            connectors: summary.connectors.len(),
            ports: summary.ports.len(),
            edges: graph.edges.len(),
        }
    }
}

/// 一份快照（§3.2）。字段顺序就是文件里的顺序。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 格式版本，现为 [`SNAPSHOT_VERSION`]。
    pub version: u32,
    /// 恒为 [`SNAPSHOT_KIND`]。
    pub kind: String,
    /// 序号，从 1 起。刚做出来还没存的、直接写到指定文件的是 `None`，文件里写 `null`。
    pub seq: Option<u64>,
    /// 图号，与 `summary.drawing_id` 相同。
    pub drawing_id: String,
    pub created_at: u64,
    pub created_by: String,
    pub reason: String,
    pub generator: String,
    pub dirty: bool,
    pub counts: Counts,
    /// 正文（`summary`、`graph`）的校验和，小写十六进制；算法见模块说明。
    pub body_sha256: String,
    /// 图纸摘要全文。
    pub summary: DrawingSummary,
    /// 做快照时算出的连通图。存下来而不是比对时再算，算法以后改了，旧快照的连接关系不跟着变。
    pub graph: Graph,
}

/// 做快照、存取快照时的错误：§3.8 里属于这一层的六个码，加上读写文件出错的
/// [`SNAPSHOT_IO_ERROR`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// 正文与校验和对不上，或者 `counts`、`drawing_id` 与正文对不上；带上哪里不对。
    Corrupt(String),
    /// 不是快照文件，或者摘要写成 JSON 后读不回来；带上原因。
    FormatInvalid(String),
    /// 文件里的版本号，比 [`SNAPSHOT_VERSION`] 高。
    VersionUnsupported(u64),
    /// 原因的字数，超过了 [`MAX_REASON_CHARS`]。
    ReasonTooLong(usize),
    /// 没有写：会盖掉已有的文件，或者快照已经有序号；带上是哪个文件、哪个序号。
    Immutable(String),
    /// 要读的快照不在；带上找的是哪个文件。
    NotFound(String),
    /// 文件系统报的错；带上路径和原因。
    Io(String),
}

impl SnapshotError {
    /// 错误码（§3.8）。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Corrupt(_) => SNAPSHOT_CORRUPT,
            Self::FormatInvalid(_) => SNAPSHOT_FORMAT_INVALID,
            Self::VersionUnsupported(_) => SNAPSHOT_VERSION_UNSUPPORTED,
            Self::ReasonTooLong(_) => SNAPSHOT_REASON_TOO_LONG,
            Self::Immutable(_) => SNAPSHOT_IMMUTABLE,
            Self::NotFound(_) => SNAPSHOT_NOT_FOUND,
            Self::Io(_) => SNAPSHOT_IO_ERROR,
        }
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt(detail)
            | Self::FormatInvalid(detail)
            | Self::Immutable(detail)
            | Self::NotFound(detail)
            | Self::Io(detail) => write!(f, "{}: {detail}", self.code()),
            Self::VersionUnsupported(version) => write!(
                f,
                "{SNAPSHOT_VERSION_UNSUPPORTED}: 快照是第 {version} 版的格式，这个程序只认到第 {SNAPSHOT_VERSION} 版"
            ),
            Self::ReasonTooLong(chars) => write!(
                f,
                "{SNAPSHOT_REASON_TOO_LONG}: 原因有 {chars} 个字，最多 {MAX_REASON_CHARS} 个"
            ),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// 由摘要和连通图做一份快照（序号暂空）。原因超过 [`MAX_REASON_CHARS`] 个字报
/// [`SNAPSHOT_REASON_TOO_LONG`]；摘要里有写成 JSON 就读不回来的数（NaN、无穷大）报
/// [`SNAPSHOT_FORMAT_INVALID`]，免得留下一份读不回来的快照。
pub fn capture(
    summary: DrawingSummary,
    graph: Graph,
    meta: SnapshotMeta,
) -> Result<Snapshot, SnapshotError> {
    check_reason(&meta.reason)?;
    let summary_value = serde_json::to_value(&summary).expect("摘要能写成 JSON");
    if let Err(error) = DrawingSummary::deserialize(&summary_value) {
        return Err(invalid(format!(
            "摘要写成 JSON 后读不回来，多半是有 NaN 或无穷大的数：{error}"
        )));
    }
    let graph_value = serde_json::to_value(&graph).expect("连通图能写成 JSON");
    Ok(Snapshot {
        version: SNAPSHOT_VERSION,
        kind: SNAPSHOT_KIND.to_owned(),
        seq: None,
        drawing_id: summary.drawing_id.clone(),
        created_at: meta.created_at,
        created_by: meta.created_by,
        reason: meta.reason,
        generator: meta.generator,
        dirty: meta.dirty,
        counts: Counts::of(&summary, &graph),
        body_sha256: body_sha256(&summary_value, &graph_value),
        summary,
        graph,
    })
}

/// 解析并校验一份快照文件的字节。先认 `kind` 和 `version`，再重算正文的校验和，然后才按
/// 字段读，最后核对原因的字数、序号，以及 `counts`、`drawing_id` 与正文一致。开头的
/// UTF-8 BOM 不算错。
pub fn parse(bytes: &[u8]) -> Result<Snapshot, SnapshotError> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| invalid(format!("不是 JSON：{error}")))?;
    check_body(&value)?;
    let snapshot: Snapshot =
        serde_json::from_value(value).map_err(|error| invalid(format!("字段不对：{error}")))?;
    check_fields(&snapshot)?;
    Ok(snapshot)
}

/// `kind`、`version` 认得，正文的校验和对得上。校验和在按字段读之前算，所以正文里改了
/// 什么（连键名在内）都报 [`SNAPSHOT_CORRUPT`]，而不是读不进来。
fn check_body(value: &Value) -> Result<(), SnapshotError> {
    let Some(fields) = value.as_object() else {
        return Err(invalid("最外层不是 JSON 对象"));
    };
    match fields.get("kind") {
        Some(Value::String(kind)) if kind == SNAPSHOT_KIND => {}
        Some(kind) => return Err(invalid(format!("`kind` 是 {kind}，不是快照"))),
        None => return Err(invalid("缺 `kind`")),
    }
    let version = fields
        .get("version")
        .ok_or_else(|| invalid("缺 `version`"))?;
    match version.as_u64() {
        Some(known) if known == u64::from(SNAPSHOT_VERSION) => {}
        Some(newer) if newer > u64::from(SNAPSHOT_VERSION) => {
            return Err(SnapshotError::VersionUnsupported(newer));
        }
        _ => return Err(invalid(format!("`version` 应是从 1 起的整数：{version}"))),
    }
    let recorded = match fields.get("body_sha256") {
        Some(Value::String(recorded)) => recorded,
        Some(other) => return Err(invalid(format!("`body_sha256` 不是字符串：{other}"))),
        None => return Err(invalid("缺 `body_sha256`")),
    };
    let part = |key: &str| {
        fields
            .get(key)
            .ok_or_else(|| invalid(format!("缺 `{key}`")))
    };
    let actual = body_sha256(part("summary")?, part("graph")?);
    if actual.eq_ignore_ascii_case(recorded) {
        Ok(())
    } else {
        Err(SnapshotError::Corrupt(format!(
            "正文算出的校验和是 {actual}，文件里记的是 {recorded}"
        )))
    }
}

/// 按字段读进来以后：原因的字数、序号，以及 `drawing_id`、`counts` 与正文一致。
fn check_fields(snapshot: &Snapshot) -> Result<(), SnapshotError> {
    check_reason(&snapshot.reason)?;
    if snapshot.seq == Some(0) {
        return Err(invalid("`seq` 从 1 起，不能是 0"));
    }
    if snapshot.drawing_id != snapshot.summary.drawing_id {
        return Err(SnapshotError::Corrupt(format!(
            "`drawing_id` 是 {}，正文里是 {}",
            snapshot.drawing_id, snapshot.summary.drawing_id
        )));
    }
    let counts = Counts::of(&snapshot.summary, &snapshot.graph);
    if snapshot.counts != counts {
        return Err(SnapshotError::Corrupt(format!(
            "`counts` 记的是 {}，正文里是 {}",
            json!(snapshot.counts),
            json!(counts)
        )));
    }
    Ok(())
}

fn check_reason(reason: &str) -> Result<(), SnapshotError> {
    let chars = reason.chars().count();
    if chars > MAX_REASON_CHARS {
        Err(SnapshotError::ReasonTooLong(chars))
    } else {
        Ok(())
    }
}

fn invalid(detail: impl Into<String>) -> SnapshotError {
    SnapshotError::FormatInvalid(detail.into())
}

impl Snapshot {
    /// 写成文件内容：UTF-8、不带 BOM、不缩进的 JSON，字段按 §3.2 的顺序。
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("快照能写成 JSON")
    }
}

/// 正文的校验和：`summary`、`graph` 各自经 [`write_canonical`] 写出来，首尾相接算 SHA-256，
/// 写成小写十六进制。
fn body_sha256(summary: &Value, graph: &Value) -> String {
    let mut body = Vec::new();
    write_canonical(summary, &mut body);
    write_canonical(graph, &mut body);
    format!("{:x}", Sha256::digest(body))
}

/// 按键名排序、不带空白写出 `value`。键按 UTF-8 字节序排；字符串和数照 `serde_json` 的
/// 写法。规则检查的规则包摘要也有一份同样的写法，这里另写一份：它是快照文件格式的一部分，
/// 不能跟着别处改。
fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by_key(|(key, _)| *key);
            out.push(b'{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_json(key, out);
                out.push(b':');
                write_canonical(value, out);
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical(item, out);
            }
            out.push(b']');
        }
        scalar => write_json(scalar, out),
    }
}

fn write_json<T: Serialize + ?Sized>(value: &T, out: &mut Vec<u8>) {
    serde_json::to_writer(out, value).expect("字符串和数写进内存不会失败");
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map};

    use super::*;
    use crate::pid::project_index::{
        graph, summarize, synthetic_drawing, SummaryMeta, SummarySource, DEFAULT_TOLERANCE,
    };
    use crate::pid::symbols::SymbolLibrary;

    /// 合成图（`objects` 个对象）的摘要和连通图。
    pub(super) fn drawing(objects: usize) -> (DrawingSummary, Graph) {
        let meta = SummaryMeta {
            drawing_id: "PID-1101".to_owned(),
            file: None,
            pbs: None,
            source: SummarySource::Open,
            fingerprint: None,
        };
        let summary = summarize(
            &synthetic_drawing(objects),
            &SymbolLibrary::builtin(),
            meta,
            DEFAULT_TOLERANCE,
        );
        let graph = graph(&summary);
        (summary, graph)
    }

    pub(super) fn meta(reason: &str) -> SnapshotMeta {
        SnapshotMeta {
            created_at: 1_794_800_000,
            created_by: "zhangsan".to_owned(),
            reason: reason.to_owned(),
            generator: "OpenCADStudio 0.9.0".to_owned(),
            dirty: true,
        }
    }

    /// 20 个对象的合成图做的快照。
    fn snapshot() -> Snapshot {
        let (summary, graph) = drawing(20);
        capture(summary, graph, meta("送审前")).expect("能做快照")
    }

    /// 快照写成 JSON、改过外层字段后的文件内容。
    fn edited(snapshot: &Snapshot, edit: impl FnOnce(&mut Map<String, Value>)) -> Vec<u8> {
        let mut file = serde_json::to_value(snapshot).expect("快照能写成 JSON");
        edit(file.as_object_mut().expect("快照是 JSON 对象"));
        serde_json::to_vec(&file).expect("能写成 JSON")
    }

    /// 同一个值，键按字节序重新排过：另一种写法，用来核对 [`write_canonical`]。
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sorted(&map[key])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }

    /// 每一层对象的键都倒过来排。
    fn reverse_keys(value: &mut Value) {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
                entries.reverse();
                for (_, value) in &mut entries {
                    reverse_keys(value);
                }
                *map = entries.into_iter().collect();
            }
            Value::Array(items) => items.iter_mut().for_each(reverse_keys),
            _ => {}
        }
    }

    /// 做出来的快照：§3.2 的各项都在，`counts` 与正文一致；写成的文件是 UTF-8、不带 BOM、
    /// 不缩进、字段按 §3.2 的顺序；读回来与原来相同，再写一遍逐字节相同。
    #[test]
    fn capture_then_parse_gives_the_same_snapshot() {
        let (summary, graph) = drawing(40);
        let counts = Counts {
            objects: summary.objects.len(),
            segments: summary.segments.len(),
            connectors: summary.connectors.len(),
            ports: summary.ports.len(),
            edges: graph.edges.len(),
        };
        assert!(
            counts.objects == 40
                && counts.segments > 0
                && counts.connectors > 0
                && counts.ports > 0
                && counts.edges > 0,
            "合成图里各类都有：{counts:?}"
        );
        let snapshot = capture(summary.clone(), graph.clone(), meta("送审前")).expect("能做快照");
        assert_eq!(
            (
                snapshot.version,
                snapshot.kind.as_str(),
                snapshot.seq,
                snapshot.drawing_id.as_str()
            ),
            (SNAPSHOT_VERSION, SNAPSHOT_KIND, None, "PID-1101")
        );
        assert_eq!(
            (
                snapshot.created_at,
                snapshot.created_by.as_str(),
                snapshot.reason.as_str(),
                snapshot.generator.as_str(),
                snapshot.dirty
            ),
            (
                1_794_800_000,
                "zhangsan",
                "送审前",
                "OpenCADStudio 0.9.0",
                true
            )
        );
        assert_eq!(snapshot.counts, counts);
        assert_eq!(snapshot.summary, summary);
        assert_eq!(snapshot.graph, graph);
        assert!(
            snapshot.body_sha256.len() == 64
                && snapshot
                    .body_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "小写十六进制：{}",
            snapshot.body_sha256
        );

        let bytes = snapshot.to_bytes();
        let text = std::str::from_utf8(&bytes).expect("UTF-8");
        assert!(
            text.starts_with(
                "{\"version\":1,\"kind\":\"pid-snapshot\",\"seq\":null,\"drawing_id\":\"PID-1101\","
            ),
            "{}",
            text.chars().take(120).collect::<String>()
        );
        assert!(!text.contains('\n') && !text.contains("\": "), "不缩进");
        let keys = [
            "version",
            "kind",
            "seq",
            "drawing_id",
            "created_at",
            "created_by",
            "reason",
            "generator",
            "dirty",
            "counts",
            "body_sha256",
            "summary",
            "graph",
        ];
        let at: Vec<usize> = keys
            .iter()
            .map(|key| text.find(&format!("\"{key}\":")).expect(key))
            .collect();
        assert!(
            at.windows(2).all(|pair| pair[0] < pair[1]),
            "字段顺序：{at:?}"
        );

        let parsed = parse(&bytes).expect("读得回来");
        assert_eq!(parsed, snapshot);
        assert_eq!(parsed.to_bytes(), bytes, "再写一遍逐字节相同");
        let again = capture(summary, graph, meta("送审前")).expect("能做快照");
        assert_eq!(again.to_bytes(), bytes, "同样的输入做出同样的文件");
    }

    /// 校验和是 `summary`、`graph` 各自按键名排序、不带空白写出来，首尾相接算的 SHA-256。
    /// 所以文件缩进与否、键的先后都不影响读；开头带 BOM 也不算错。
    #[test]
    fn the_checksum_is_over_the_sorted_compact_body() {
        let mut canonical = Vec::new();
        write_canonical(
            &json!({
                "nodes": ["P-1001", "引号\"和\\"],
                "edges": [["P-1001", "引号\"和\\"]],
                "位号": {"z": 1, "Z": [true, null, 0.5, -2]}
            }),
            &mut canonical,
        );
        assert_eq!(
            String::from_utf8(canonical).expect("UTF-8"),
            r#"{"edges":[["P-1001","引号\"和\\"]],"nodes":["P-1001","引号\"和\\"],"位号":{"Z":[true,null,0.5,-2],"z":1}}"#
        );

        let snapshot = snapshot();
        let sorted_text = |value: Value| serde_json::to_string(&sorted(&value)).expect("JSON");
        let body = sorted_text(serde_json::to_value(&snapshot.summary).expect("JSON"))
            + &sorted_text(serde_json::to_value(&snapshot.graph).expect("JSON"));
        assert_eq!(
            snapshot.body_sha256,
            format!("{:x}", Sha256::digest(body.as_bytes()))
        );

        let pretty = serde_json::to_vec_pretty(&snapshot).expect("JSON");
        assert_eq!(parse(&pretty).expect("缩进不影响"), snapshot);
        let mut reordered = serde_json::to_value(&snapshot).expect("JSON");
        reverse_keys(&mut reordered);
        let reordered = serde_json::to_vec(&reordered).expect("JSON");
        assert_ne!(reordered, snapshot.to_bytes(), "键倒过来了");
        assert_eq!(parse(&reordered).expect("键的先后不影响"), snapshot);
        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend(snapshot.to_bytes());
        assert_eq!(parse(&with_bom).expect("开头的 BOM 不算错"), snapshot);
    }

    /// 正文里逐个字符地改：字符串里的每个字符换成别的字符，数里的每位数字换成别的数字，
    /// `true` 与 `false` 互换，`null` 换成 `0`；每一处都报 `SNAPSHOT_CORRUPT`。数改了一位
    /// 值却没变的（有效数字到 17 位时末位可能落回同一个数）不算改动。校验和本身改一位也一样。
    /// 摘要只取合成图各列表的前几项，连通图用整张图的（快照不核对两者是否相配）：整张图逐字符
    /// 地改要十几秒。
    #[test]
    fn changing_any_character_of_the_body_is_corrupt() {
        let (mut summary, graph) = drawing(20);
        summary.objects.truncate(8);
        summary.segments.truncate(4);
        summary.signal_ends.truncate(2);
        summary.connectors.truncate(1);
        summary.ports.truncate(8);
        assert!(!graph.edges.is_empty(), "合成图的连通图有边");
        let snapshot = capture(summary, graph, meta("送审前")).expect("能做快照");
        let text = String::from_utf8(snapshot.to_bytes()).expect("UTF-8");
        let original: Value = serde_json::from_str(&text).expect("JSON");
        let body = text.find(",\"summary\":").expect("有 summary") + ",\"summary\":".len();
        let graph_key = text.rfind(",\"graph\":").expect("有 graph");
        let graph_key = graph_key..graph_key + ",\"graph\":".len();
        // （位置，换掉几个字节，换成什么，是不是数里的数字）
        let mut changes: Vec<(usize, usize, &str, bool)> = Vec::new();
        let mut in_string = false;
        let mut skip = 0;
        for (offset, ch) in text[body..text.len() - 1].char_indices() {
            let at = body + offset;
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let rest = &text[at..];
            let change = match (in_string, ch) {
                (_, '"') => {
                    in_string = !in_string;
                    None
                }
                (true, '\\') => {
                    skip = if rest[1..].starts_with('u') { 5 } else { 1 };
                    None
                }
                (true, 'x') => Some((1, "y", false)),
                (true, _) => Some((ch.len_utf8(), "x", false)),
                (false, '0'..='9') => {
                    let next = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "8"];
                    Some((1, next[ch.to_digit(10).expect("数字") as usize], true))
                }
                (false, 't') if rest.starts_with("true") => {
                    skip = 3;
                    Some((4, "false", false))
                }
                (false, 'f') if rest.starts_with("false") => {
                    skip = 4;
                    Some((5, "true", false))
                }
                (false, 'n') if rest.starts_with("null") => {
                    skip = 3;
                    Some((4, "0", false))
                }
                _ => None,
            };
            if let Some((len, replacement, in_number)) = change {
                if !graph_key.contains(&at) {
                    changes.push((at, len, replacement, in_number));
                }
            }
        }
        assert!(changes.len() > 1_000, "只找到 {} 处可改", changes.len());

        let mut same_value = 0;
        let mut missed = Vec::new();
        for &(at, len, replacement, in_number) in &changes {
            let changed = format!("{}{replacement}{}", &text[..at], &text[at + len..]);
            if in_number {
                let value: Value = serde_json::from_str(&changed)
                    .unwrap_or_else(|error| panic!("改第 {at} 个字节后不是 JSON 了：{error}"));
                if value["summary"] == original["summary"] && value["graph"] == original["graph"] {
                    same_value += 1;
                    continue;
                }
            }
            match parse(changed.as_bytes()) {
                Err(SnapshotError::Corrupt(_)) => {}
                other => missed.push(format!(
                    "第 {at} 个字节 {:?} 换成 {replacement}：{:?}",
                    &text[at..at + len],
                    other.map(|_| ())
                )),
            }
        }
        assert!(
            missed.is_empty(),
            "{} 处没报 SNAPSHOT_CORRUPT：{:?}",
            missed.len(),
            &missed[..missed.len().min(5)]
        );
        assert!(
            same_value * 100 <= changes.len(),
            "值没变的有 {same_value} 处，共 {} 处",
            changes.len()
        );

        let sha = text.find("\"body_sha256\":\"").expect("有校验和") + "\"body_sha256\":\"".len();
        for (offset, digit) in text[sha..sha + 64].char_indices() {
            let replacement = if digit == '0' { "1" } else { "0" };
            let at = sha + offset;
            let changed = format!("{}{replacement}{}", &text[..at], &text[at + 1..]);
            let error = parse(changed.as_bytes()).expect_err("校验和改了一位");
            assert_eq!(
                error.code(),
                SNAPSHOT_CORRUPT,
                "校验和第 {offset} 位：{error}"
            );
            assert!(
                error.to_string().starts_with("SNAPSHOT_CORRUPT: "),
                "{error}"
            );
        }
    }

    /// 坏文件各报各的码：JSON 坏了、缺字段、字段类型不对、`kind` 不对是
    /// `SNAPSHOT_FORMAT_INVALID`；`version` 更高是 `SNAPSHOT_VERSION_UNSUPPORTED`（只看
    /// `kind` 和 `version`，别的字段不管）；`counts`、`drawing_id` 与正文对不上是
    /// `SNAPSHOT_CORRUPT`；原因超过 200 字是 `SNAPSHOT_REASON_TOO_LONG`。
    #[test]
    fn bad_files_are_refused_with_their_codes() {
        let snapshot = snapshot();
        let bytes = snapshot.to_bytes();
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            ("空的", Vec::new(), SNAPSHOT_FORMAT_INVALID),
            (
                "不是 JSON",
                b"PIDSNAPSHOT".to_vec(),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "只有前一半",
                bytes[..bytes.len() / 2].to_vec(),
                SNAPSHOT_FORMAT_INVALID,
            ),
            ("最外层是数组", b"[]".to_vec(), SNAPSHOT_FORMAT_INVALID),
            (
                "缺 kind",
                edited(&snapshot, |file| {
                    file.remove("kind");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "kind 不对",
                edited(&snapshot, |file| file["kind"] = json!("pid-bom")),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "缺 version",
                edited(&snapshot, |file| {
                    file.remove("version");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "version 是字符串",
                edited(&snapshot, |file| file["version"] = json!("1")),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "version 是 0",
                edited(&snapshot, |file| file["version"] = json!(0)),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "version 是小数",
                edited(&snapshot, |file| file["version"] = json!(1.5)),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "version 是 2",
                edited(&snapshot, |file| file["version"] = json!(2)),
                SNAPSHOT_VERSION_UNSUPPORTED,
            ),
            (
                "更高的版本只认 kind 和 version",
                br#"{"kind":"pid-snapshot","version":7}"#.to_vec(),
                SNAPSHOT_VERSION_UNSUPPORTED,
            ),
            (
                "缺 body_sha256",
                edited(&snapshot, |file| {
                    file.remove("body_sha256");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "body_sha256 是数",
                edited(&snapshot, |file| file["body_sha256"] = json!(0)),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "缺 summary",
                edited(&snapshot, |file| {
                    file.remove("summary");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "缺 graph",
                edited(&snapshot, |file| {
                    file.remove("graph");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "缺 created_at",
                edited(&snapshot, |file| {
                    file.remove("created_at");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "缺 counts",
                edited(&snapshot, |file| {
                    file.remove("counts");
                }),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "created_at 是负数",
                edited(&snapshot, |file| file["created_at"] = json!(-1)),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "dirty 是字符串",
                edited(&snapshot, |file| file["dirty"] = json!("是")),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "seq 是 0",
                edited(&snapshot, |file| file["seq"] = json!(0)),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "seq 是字符串",
                edited(&snapshot, |file| file["seq"] = json!("3")),
                SNAPSHOT_FORMAT_INVALID,
            ),
            (
                "counts 多一个对象",
                edited(&snapshot, |file| {
                    file["counts"]["objects"] = json!(snapshot.counts.objects + 1);
                }),
                SNAPSHOT_CORRUPT,
            ),
            (
                "counts 少一条边",
                edited(&snapshot, |file| {
                    file["counts"]["edges"] = json!(snapshot.counts.edges - 1);
                }),
                SNAPSHOT_CORRUPT,
            ),
            (
                "图号与正文不一样",
                edited(&snapshot, |file| file["drawing_id"] = json!("PID-1102")),
                SNAPSHOT_CORRUPT,
            ),
            (
                "原因 201 字",
                edited(&snapshot, |file| file["reason"] = json!("审".repeat(201))),
                SNAPSHOT_REASON_TOO_LONG,
            ),
        ];
        for (name, bytes, code) in cases {
            match parse(&bytes) {
                Ok(_) => panic!("{name}：读进来了"),
                Err(error) => {
                    assert_eq!(error.code(), code, "{name}：{error}");
                    assert!(
                        error.to_string().starts_with(&format!("{code}: ")),
                        "{name}：{error}"
                    );
                }
            }
        }

        let numbered = parse(&edited(&snapshot, |file| file["seq"] = json!(3))).expect("序号 3");
        assert_eq!(numbered.seq, Some(3));
        let extra = edited(&snapshot, |file| {
            file.insert("note".to_owned(), json!("以后加的字段"));
        });
        assert_eq!(parse(&extra).expect("外层多出来的字段不管"), snapshot);
    }

    /// 原因按字数算：200 个汉字（600 个字节）可以，201 个不行；可以空。
    #[test]
    fn the_reason_is_at_most_200_characters() {
        let (summary, graph) = drawing(20);
        let longest = capture(
            summary.clone(),
            graph.clone(),
            meta(&"审".repeat(MAX_REASON_CHARS)),
        )
        .expect("200 字可以");
        let parsed = parse(&longest.to_bytes()).expect("读得回来");
        assert_eq!(parsed.reason.chars().count(), 200);
        let error = capture(summary.clone(), graph.clone(), meta(&"审".repeat(201)))
            .expect_err("201 字不行");
        assert_eq!(error, SnapshotError::ReasonTooLong(201));
        assert_eq!(error.code(), SNAPSHOT_REASON_TOO_LONG);
        assert_eq!(
            error.to_string(),
            "SNAPSHOT_REASON_TOO_LONG: 原因有 201 个字，最多 200 个"
        );
        assert!(capture(summary, graph, meta("")).is_ok(), "原因可以空");
    }

    /// 小数写出去再读回来一位不差：各种量级的随机双精度数和几个难写的数，读回来逐位相同，
    /// 校验和也对得上。
    #[test]
    fn numbers_come_back_exactly() {
        let (mut summary, graph) = drawing(200);
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let random = move || loop {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let number = f64::from_bits(state);
            if number.is_finite() {
                return number;
            }
        };
        let hard = [
            0.1 + 0.2,
            1.0 / 3.0,
            -0.0,
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            1e21,
            1e-7,
            2.0_f64.sqrt() * 1000.0,
        ];
        let mut numbers = hard.into_iter().chain(std::iter::repeat_with(random));
        let mut next = || numbers.next().expect("要多少有多少");
        for object in &mut summary.objects {
            object.at = [next(), next()];
            object.bbox = [next(), next(), next(), next()];
        }
        for segment in &mut summary.segments {
            segment.a = [next(), next()];
            segment.b = [next(), next()];
            segment.length = next();
        }
        let bits = |summary: &DrawingSummary| -> Vec<u64> {
            let objects = summary
                .objects
                .iter()
                .flat_map(|object| object.at.into_iter().chain(object.bbox));
            let segments = summary.segments.iter().flat_map(|segment| {
                segment
                    .a
                    .into_iter()
                    .chain(segment.b)
                    .chain([segment.length])
            });
            objects.chain(segments).map(f64::to_bits).collect()
        };
        let expected = bits(&summary);
        assert!(expected.len() > 1_500, "{} 个数", expected.len());

        let snapshot = capture(summary, graph, meta("")).expect("能做快照");
        let parsed = parse(&snapshot.to_bytes()).expect("读得回来");
        assert_eq!(bits(&parsed.summary), expected);
        assert_eq!(parsed, snapshot);
    }

    /// NaN、无穷大写成 JSON 是 `null`，读不回来；做快照时就拒绝，不留一份读不回来的快照。
    #[test]
    fn a_summary_that_cannot_be_read_back_is_refused() {
        let (summary, graph) = drawing(20);
        capture(summary.clone(), graph.clone(), meta("")).expect("数都是有限的就能做");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut summary = summary.clone();
            summary.objects[0].bbox[2] = bad;
            let error = capture(summary, graph.clone(), meta("")).expect_err("拒绝");
            assert_eq!(error.code(), SNAPSHOT_FORMAT_INVALID, "{bad}：{error}");
        }
    }
}

//! DWG 往返零丢失（`VAL-SEM-034` / `VAL-SEM-035`）。
//!
//! P&ID 语义落在两种载体上：每实体 `PID_AUTHOR` XDATA（九个键）与模型空间
//! 扩展字典里的四个图级 XRecord。两种载体的 DWG 序列化由 `codec`
//! （`opencaddocument`）负责，本模块不写任何序列化代码，只做三件事：
//!
//! 1. 造一份覆盖全部语义的 fixture（五个实例 + 四个图级表）；
//! 2. 保存前自建 [`Snapshot`]，读回后再抓一次，[`diff`] 出 [`LossItem`]；
//! 3. 断言丢失数为 0，失败时打印逐项明细（handle / 键名 / 原值 / 读回值）。
//!
//! **`io::*` 与 `document::Format` 的 `losses` 恒为空，不能当零丢失证据。**
//! `codec` 的 `Dwg::write` / `Dwg::read` 一律 `Ok((.., Vec::new()))`，那个
//! 字段只说明「转换器没报诊断」，不说明「数据没丢」。真正的判据是本文的
//! 逐项比对。
//!
//! **硬约束：`PID_AUTHOR` 记录里不许出现会随实体变换改写的五种变体。**
//! `ExtendedData::apply_transform` 会换算 `Position3D`(1011) /
//! `Displacement3D`(1012) / `Direction3D`(1013) / `Distance`(1041) /
//! `ScaleFactor`(1042)，一旦换算成功，调用方（`translate.rs` /
//! `transform.rs`）会丢弃该实体的 verbatim `raw_record`，往返就不再无损。
//! [`assert_no_transform_sensitive_variants`] 把这条前提变成可执行断言。
//!
//! ## 七版本（`VAL-SEM-035`）
//!
//! 2018 / 2013 / 2010 / 2007 / 2004 / 2000 / R14，每个版本各自「保存 → 读回」
//! 一次，每个版本独立断言位号丢失 0、属性键值对丢失 0、四个 XRecord key
//! 齐全。范围不得缩小。
//!
//! **R14（`AC1014`）分两层报告，不得混层**：
//!
//! - 第一层（硬断言，闭环）：读回四个 key 都在且条目逐字节相等。这一层是
//!   `VAL-SEM-035` 的判据。
//! - 第二层（软断言，语义，只记录不断言）：`AC1014` 写出的 XRecord 类型码是
//!   规范的固定码 `79` 还是类号 `≥500`。`codec` 在 R14 走
//!   `class_type_code("XRECORD", OBJ_XRECORD)`，类表里有 `XRECORD`（默认类表
//!   无条件含它，`retain_legacy_dwg_classes` 的白名单也保留它）时返回类号，
//!   因此实测为 `≥500`。第三方 CAD 会把这种对象当 proxy，但自写自读闭环不受
//!   影响。这一层**不**参与任何 `assert`，失败也不会让测试变红。

// 本模块是「判据库 + 两条契约测试」：[`Snapshot`] / [`diff`] / [`LossItem`]
// 只在 `#[cfg(test)]` 里被调用，非测试构建下必然全是 dead_code。保留它们
// 供后续 P&ID feature（属性下传、拓扑校验）复用，而不是压成测试私有。
#![allow(dead_code)]

use document::types::Vector3;
use document::{Document, DxfVersion, EntityType, Handle};

use crate::pid::collab_state::{
    PID_AUTHOR_COLLAB, PID_AUTHOR_COLLAB_QUEUE, PID_AUTHOR_COLLAB_UNSYNCED,
};
use crate::pid::graph_data::{
    PID_AUTHOR_CONNECTORS, PID_AUTHOR_TAG_RULES, PID_AUTHOR_TOPOLOGY, PID_AUTHOR_TYPE_LIB,
};
use crate::pid::persist::{self, PidAuthor};
use crate::pid::tag_rule::TagRule;
use crate::pid::type_lib::{TypeLib, TypeNode};

/// 契约顺序：2018 / 2013 / 2010 / 2007 / 2004 / 2000 / R14。
///
/// 与 `src/io/mtext_save_loss_tests.rs` 的 `TARGETS` 同序同集合，照抄不另起。
const TARGETS: [(DxfVersion, &str); 7] = [
    (DxfVersion::AC1032, "2018"),
    (DxfVersion::AC1027, "2013"),
    (DxfVersion::AC1024, "2010"),
    (DxfVersion::AC1021, "2007"),
    (DxfVersion::AC1018, "2004"),
    (DxfVersion::AC1015, "2000"),
    (DxfVersion::AC1014, "R14"),
];

/// 七个图级 key，按契约顺序（`REQ-COLL-015` 的三条协同记录随图保存：
/// 记录、队列、未同步清单；离线规格 §5.1）。
const GRAPH_KEYS: [&str; 7] = [
    PID_AUTHOR_TYPE_LIB,
    PID_AUTHOR_TAG_RULES,
    PID_AUTHOR_TOPOLOGY,
    PID_AUTHOR_CONNECTORS,
    PID_AUTHOR_COLLAB,
    PID_AUTHOR_COLLAB_QUEUE,
    PID_AUTHOR_COLLAB_UNSYNCED,
];

/// 五种会随实体变换被改写、进而让调用方丢弃 verbatim `raw_record` 的变体。
///
/// 判定**按类型名**，不按组码数字猜：`Real` 是 1040、`Distance` 是 1041、
/// `ScaleFactor` 是 1042，组码写错就会漏判。
fn transform_sensitive_name(value: &document::xdata::XDataValue) -> Option<&'static str> {
    use document::xdata::XDataValue;
    match value {
        XDataValue::Position3D(_) => Some("Position3D(1011)"),
        XDataValue::Displacement3D(_) => Some("Displacement3D(1012)"),
        XDataValue::Direction3D(_) => Some("Direction3D(1013)"),
        XDataValue::Distance(_) => Some("Distance(1041)"),
        XDataValue::ScaleFactor(_) => Some("ScaleFactor(1042)"),
        _ => None,
    }
}

/// 前置检查：`PID_AUTHOR` 记录里不许出现那五种变体。
///
/// 命中即说明「往返免费」的前提被破坏 —— 实体一旦被 translate / scale，
/// 记录会被重写、verbatim `raw_record` 被丢弃。直接 panic 并报出位置。
fn assert_no_transform_sensitive_variants(doc: &Document) {
    for entity in doc.entities() {
        let handle = entity.common().handle;
        for record in entity.common().extended_data.records() {
            for value in &record.values {
                if let Some(name) = transform_sensitive_name(value) {
                    panic!(
                        "PID_AUTHOR 记录里出现 {name}（实体 {handle}, app {}）——\
                         实体一旦被 translate/scale，raw_record 会被丢弃，DWG 往返不再无损",
                        record.application_name
                    );
                }
            }
        }
    }
}

// ── 丢失明细 ─────────────────────────────────────────────────────────────

/// 一个丢失项：实体 handle（图级数据用 `None`）+ 键名 + 原值 + 读回值。
#[derive(Debug, Clone, PartialEq, Eq)]
struct LossItem {
    handle: Option<u64>,
    key: String,
    before: String,
    after: String,
}

impl LossItem {
    fn new(handle: Option<Handle>, key: impl Into<String>, before: impl Into<String>, after: impl Into<String>) -> Self {
        Self {
            handle: handle.map(|handle| handle.value()),
            key: key.into(),
            before: before.into(),
            after: after.into(),
        }
    }
}

impl std::fmt::Display for LossItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let handle = match self.handle {
            Some(value) => format!("0x{value:X}"),
            None => "<图级>".to_string(),
        };
        write!(
            f,
            "{handle:<10} {:<16} 原值={:?} 读回={:?}",
            self.key, self.before, self.after
        )
    }
}

/// 丢失明细表：一行一项，第一行是表头。
fn loss_table(losses: &[LossItem]) -> String {
    let mut text = format!("丢失 {} 项\n", losses.len());
    text.push_str(&format!(
        "{:<10} {:<16} {:<28} {}\n",
        "handle", "键名", "原值", "读回值"
    ));
    for item in losses {
        text.push_str(&format!("{item}\n"));
    }
    text
}

// ── 快照 ─────────────────────────────────────────────────────────────────

/// 一个实体的 P&ID 语义快照。
#[derive(Debug, Clone, PartialEq, Eq)]
struct EntitySnapshot {
    handle: Handle,
    /// `Insert` 才有：块名。
    block_name: Option<String>,
    /// 九键，键名 + 值的字符串形式（`None` = 键不存在）。
    nine_keys: Vec<(String, Option<String>)>,
}

/// 整张图的 P&ID 语义快照。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    entities: Vec<EntitySnapshot>,
    /// [`GRAPH_KEYS`] 各 key 的条目（code + 值的规范化文本），`None` = 记录不存在。
    graph: Vec<(String, Option<Vec<String>>)>,
}

/// 一条 XRecord 条目的规范化文本：`<组码>:<类型>:<值>`。
///
/// 用文本而不是 `PartialEq` 直接比，是为了让失败明细能原样打印出差异。
fn entry_repr(entry: &document::objects::XRecordEntry) -> String {
    use document::objects::XRecordValue;
    match &entry.value {
        XRecordValue::String(text) => format!("{}:s:{text}", entry.code),
        XRecordValue::Double(value) => format!("{}:d:{:016X}", entry.code, value.to_bits()),
        XRecordValue::Int16(value) => format!("{}:i16:{value}", entry.code),
        XRecordValue::Int32(value) => format!("{}:i32:{value}", entry.code),
        XRecordValue::Int64(value) => format!("{}:i64:{value}", entry.code),
        XRecordValue::Byte(value) => format!("{}:b:{value}", entry.code),
        XRecordValue::Bool(value) => format!("{}:bool:{value}", entry.code),
        XRecordValue::Handle(value) => format!("{}:h:{value}", entry.code),
        XRecordValue::Point3D(x, y, z) => {
            format!("{}:p:{:016X}/{:016X}/{:016X}", entry.code, x.to_bits(), y.to_bits(), z.to_bits())
        }
        XRecordValue::Chunk(bytes) => format!("{}:chunk:{}", entry.code, hex(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

/// 实体的九键，按 [`persist::XDATA_KEYS`] 顺序。记录不存在时九个键全是 `None`。
fn nine_keys(doc: &Document, handle: Handle) -> Vec<(String, Option<String>)> {
    let record = doc
        .get_entity(handle)
        .and_then(|entity| entity.common().extended_data.get_record(persist::PID_AUTHOR_APP));
    persist::XDATA_KEYS
        .iter()
        .map(|key| {
            let value = record.and_then(|record| {
                record.values.iter().find_map(|value| {
                    let text = persist::as_str(value)?;
                    let (found, val) = text.split_once('=')?;
                    (found == *key).then(|| val.to_string())
                })
            });
            ((*key).to_string(), value)
        })
        .collect()
}

/// [`GRAPH_KEYS`] 各 key 的条目快照。记录不存在时为 `None`。
fn graph_entries(doc: &Document, key: &str) -> Option<Vec<String>> {
    let record = doc.xrecord(doc.header.model_space_block_handle, key)?;
    Some(record.entries.iter().map(entry_repr).collect())
}

fn snapshot(doc: &Document, handles: &[Handle]) -> Snapshot {
    let entities = handles
        .iter()
        .map(|handle| EntitySnapshot {
            handle: *handle,
            block_name: match doc.get_entity(*handle) {
                Some(EntityType::Insert(insert)) => Some(insert.block_name.clone()),
                _ => None,
            },
            nine_keys: nine_keys(doc, *handle),
        })
        .collect();
    let graph = GRAPH_KEYS
        .iter()
        .map(|key| ((*key).to_string(), graph_entries(doc, key)))
        .collect();
    Snapshot { entities, graph }
}

// ── 比对 ─────────────────────────────────────────────────────────────────

/// 判定「丢失」：源快照里有、读回快照里没有或不相等的东西。
///
/// 口径（与契约逐条对应）：
///
/// 1. 实体缺失 → `entity` 一条；
/// 2. `block_name` 变了 → `block_name` 一条；
/// 3. 九键逐键比（顺序敏感）：键消失记「<缺失>」，值变了记原值与读回值；
/// 4. [`GRAPH_KEYS`] 逐条比：记录消失记 `key_missing`，条目不同记
///    `key_changed`，原值与读回值都是整条 entries 的规范化文本。
fn diff(before: &Snapshot, after: &Snapshot) -> Vec<LossItem> {
    let mut losses = Vec::new();
    for expected in &before.entities {
        let Some(actual) = after
            .entities
            .iter()
            .find(|candidate| candidate.handle == expected.handle)
        else {
            losses.push(LossItem::new(
                Some(expected.handle),
                "entity",
                "present",
                "missing",
            ));
            continue;
        };
        if expected.block_name != actual.block_name {
            losses.push(LossItem::new(
                Some(expected.handle),
                "block_name",
                expected.block_name.clone().unwrap_or_default(),
                actual.block_name.clone().unwrap_or_default(),
            ));
        }
        for (index, (key, before_value)) in expected.nine_keys.iter().enumerate() {
            let after_value = actual
                .nine_keys
                .get(index)
                .map(|(_, value)| value)
                .unwrap_or(&None);
            if before_value == after_value {
                continue;
            }
            losses.push(LossItem::new(
                Some(expected.handle),
                key.clone(),
                before_value.clone().unwrap_or_else(|| "<缺失>".to_string()),
                after_value.clone().unwrap_or_else(|| "<缺失>".to_string()),
            ));
        }
    }
    for (key, before_entries) in &before.graph {
        let after_entries = after
            .graph
            .iter()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, entries)| entries.as_ref());
        match (before_entries.as_ref(), after_entries) {
            (Some(expected), Some(actual)) if expected == actual => {}
            (Some(expected), Some(actual)) => losses.push(LossItem::new(
                None,
                format!("{key}:key_changed"),
                expected.join("\u{1}"),
                actual.join("\u{1}"),
            )),
            (Some(expected), None) => losses.push(LossItem::new(
                None,
                format!("{key}:key_missing"),
                expected.join("\u{1}"),
                "<缺失>",
            )),
            (None, Some(actual)) => losses.push(LossItem::new(
                None,
                format!("{key}:key_unexpected"),
                "<缺失>",
                actual.join("\u{1}"),
            )),
            (None, None) => {}
        }
    }
    losses
}

// ── fixture ──────────────────────────────────────────────────────────────

/// 定义符号块：`Block` + 内容 + `BlockEnd`，块记录指向三者。
///
/// 形状照抄 `scene::entity::define_block_raw`，只是不引入 `Scene`（本模块
/// 只测文档层的往返，不需要渲染状态）。符号库 feature 尚未交付，块名按
/// `assets/pid-symbols.json` 的命名约定硬写。
fn define_block(doc: &mut Document, name: &str, entities: Vec<EntityType>) {
    if doc.block_records.get(name).is_some() {
        return;
    }
    let record_handle = doc.allocate_handle();
    let block_handle = doc.allocate_handle();
    let mut record = document::tables::BlockRecord::new(name);
    record.handle = record_handle;
    record.block_entity_handle = block_handle;
    if doc.block_records.add(record).is_err() {
        return;
    }
    let mut block = document::entities::Block::new(name, Vector3::ZERO);
    block.common.handle = block_handle;
    block.common.owner_handle = record_handle;
    let _ = doc.add_entity(EntityType::Block(block));
    for mut entity in entities {
        entity.common_mut().owner_handle = record_handle;
        let _ = doc.add_entity(entity);
    }
    let end_handle = doc.allocate_handle();
    let mut end = document::entities::BlockEnd::new();
    end.common.handle = end_handle;
    end.common.owner_handle = record_handle;
    let _ = doc.add_entity(EntityType::BlockEnd(end));
    if let Some(record) = doc.block_records.get_mut(name) {
        record.block_end_handle = end_handle;
    }
}

/// 一个符号块的占位几何：一个圆，足够让块定义在 DWG 里有实体。
fn symbol_geometry() -> Vec<EntityType> {
    vec![EntityType::Circle(document::entities::Circle::from_center_radius(
        Vector3::ZERO,
        0.5,
    ))]
}

/// 契约里的 fixture：五个实例 + 四个图级表。
///
/// | 实体 | 类型编码 | 位号 | props | 块 |
/// | --- | --- | --- | --- | --- |
/// | 设备 | `EQUIPMENT` | `P-1001` | 3 项 | `PID_SYM_equipment_vessel` |
/// | 管线 | `PIPE_LINE` | `L-1001` | 2 项 | `PID_SYM_pipe_line` |
/// | 管路 | `PIPE_RUN` | `L-1001-01` | — | `PID_SYM_pipe_run` |
/// | 阀门 | `VALVE` | `V-1001` | — | `PID_SYM_valve_gate_valve` |
/// | 仪表 | `INSTRUMENT` | `TI-1001` | — | `PID_SYM_instrument_transmitter` |
///
/// 恰有一个实体带非空 `connector_peer`（管线 `L-1001` = `PID-1102:601`），
/// 管路的 `owner` 指向管线 handle。
fn fixture() -> (Document, Vec<Handle>) {
    let mut doc = Document::new();
    for name in [
        "PID_SYM_equipment_vessel",
        "PID_SYM_pipe_line",
        "PID_SYM_pipe_run",
        "PID_SYM_valve_gate_valve",
        "PID_SYM_instrument_transmitter",
    ] {
        define_block(&mut doc, name, symbol_geometry());
    }

    let insert = |doc: &mut Document, block: &str, x: f64, y: f64| -> Handle {
        doc.add_entity(EntityType::Insert(document::entities::Insert::new(
            block,
            Vector3::new(x, y, 0.0),
        )))
        .expect("insert adds")
    };

    let equipment = insert(&mut doc, "PID_SYM_equipment_vessel", 0.0, 0.0);
    let pipe_line = insert(&mut doc, "PID_SYM_pipe_line", 40.0, 0.0);
    let pipe_run = insert(&mut doc, "PID_SYM_pipe_run", 40.0, 10.0);
    let valve = insert(&mut doc, "PID_SYM_valve_gate_valve", 20.0, 0.0);
    let instrument = insert(&mut doc, "PID_SYM_instrument_transmitter", 60.0, 0.0);

    let mut equipment_author = PidAuthor::new("EQUIPMENT", "P-1001");
    equipment_author.tag_source = "auto".into();
    equipment_author.tag_status = "ok".into();
    equipment_author.props = r#"{"设计压力":"1.6MPa","设计温度":"120C","材质":"碳钢"}"#.into();
    equipment_author.ports = "in1,out1".into();
    equipment_author.topology = "equipment".into();
    equipment_author.store(&mut doc, equipment);

    let mut line_author = PidAuthor::new("PIPE_LINE", "L-1001");
    line_author.tag_source = "auto".into();
    line_author.tag_status = "ok".into();
    line_author.props = r#"{"管道等级":"A1A","公称直径":"DN50"}"#.into();
    line_author.ports = "in,out".into();
    line_author.topology = "pipe_line".into();
    line_author.connector_peer = "PID-1102:601".into();
    line_author.store(&mut doc, pipe_line);

    let mut run_author = PidAuthor::new("PIPE_RUN", "L-1001-01");
    run_author.tag_source = "auto".into();
    run_author.tag_status = "ok".into();
    run_author.owner = format!("{pipe_line}");
    run_author.topology = "pipe_run".into();
    run_author.store(&mut doc, pipe_run);

    let mut valve_author = PidAuthor::new("VALVE", "V-1001");
    valve_author.tag_source = "manual".into();
    valve_author.tag_status = "ok".into();
    valve_author.topology = "valve".into();
    valve_author.store(&mut doc, valve);

    let mut instrument_author = PidAuthor::new("INSTRUMENT", "TI-1001");
    instrument_author.tag_source = "auto".into();
    instrument_author.tag_status = "ok".into();
    instrument_author.topology = "instrument".into();
    instrument_author.store(&mut doc, instrument);

    // 四个图级表。内容用前序 feature 的真实序列化形状：类型库走 TypeLib，
    // 规则表走 TagRule，拓扑与连接符按 architecture.md A.7 的对象形状。
    let type_lib = TypeLib::new([
        TypeNode::new("EQUIPMENT", None::<String>).with_label("设备"),
        TypeNode::new("VESSEL", Some("EQUIPMENT")).with_label("容器"),
    ])
    .expect("两类型类型库可构造");
    let type_lib_json: serde_json::Value =
        serde_json::from_str(&type_lib.to_json()).expect("类型库 JSON 可解析");
    graph_data_store(&mut doc, PID_AUTHOR_TYPE_LIB, &[type_lib_json]);

    let mut equipment_rule = TagRule::new("E-{序号}", 1001);
    equipment_rule.name = "设备位号".into();
    let mut vessel_rule = TagRule::new("V-{序号}", 1001);
    vessel_rule.name = "容器位号".into();
    graph_data_store(
        &mut doc,
        PID_AUTHOR_TAG_RULES,
        &[
            serde_json::to_value(&equipment_rule).expect("规则可序列化"),
            serde_json::to_value(&vessel_rule).expect("规则可序列化"),
        ],
    );

    graph_data_store(
        &mut doc,
        PID_AUTHOR_TOPOLOGY,
        &[serde_json::json!({
            "pipelines": [{"id": "L-1001", "from": "P-1001", "to": "V-1001"}],
            "runs": [{"id": "L-1001-01"}, {"id": "L-1001-02"}],
        })],
    );

    graph_data_store(
        &mut doc,
        PID_AUTHOR_CONNECTORS,
        &[
            serde_json::json!({"peer": "PID-1102:601", "sheet": 1}),
            serde_json::json!({"peer": "PID-1102:602", "sheet": 2}),
        ],
    );

    // `REQ-COLL-015`：这份图对齐过的协同位置（带它已知与房间对不上的标记）、
    // 它重开时要重发的队列，和不会再自动提交的未同步清单（离线规格 §5.1）。
    crate::pid::collab_state::store_state_diverged(
        &mut doc,
        Some(&crate::pid::FileCollabState {
            drawing_id: "PID-1101".into(),
            repo_id: "r-fixture".into(),
            revision: 42,
        }),
        Some(&crate::pid::collab_state::Diverged {
            code: crate::pid::collab_state::UNQUEUED_EDITS.into(),
            message: "做了没有排上队的修改".into(),
            at: 1_794_900_000,
        }),
    );
    crate::pid::collab_state::store_unsynced(
        &mut doc,
        &[serde_json::json!({"state": "unqueued", "label": "改属性",
            "payload": {"changes": []}, "at": 1_794_900_000,
            "message": "不是本次会话的图"})],
    );
    crate::pid::collab_state::store_queue(
        &mut doc,
        &[crate::pid::QueuedOp {
            op_id: "op-s-1-7".into(),
            payload: serde_json::json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
            base_revision: 42,
            on: true,
            label: "改属性".into(),
            user_id: None,
            queued_at: None,
            inverse: Some(vec![serde_json::json!({"kind": "set_property",
                "object_ref": "V-1001", "field": "设计压力", "value": "1.0MPa"})]),
        }],
    );

    (doc, vec![equipment, pipe_line, pipe_run, valve, instrument])
}

/// `graph_data::store` 的薄包装：显式导入避免长路径散落。
fn graph_data_store(doc: &mut Document, key: &str, items: &[serde_json::Value]) {
    crate::pid::graph_data::store(doc, key, items);
}

/// fixture 自身的形状断言（契约原文的数字）。
fn assert_fixture_shape(doc: &Document, handles: &[Handle]) {
    let mut props_counts = Vec::new();
    let mut peers = Vec::new();
    for handle in handles {
        let author = PidAuthor::load(doc, *handle)
            .unwrap_or_else(|| panic!("实体 {handle} 应当有 PID_AUTHOR 记录"));
        props_counts.push((author.tag.clone(), persist::props_len(&author.props)));
        if !author.connector_peer.is_empty() {
            peers.push((author.tag.clone(), author.connector_peer.clone()));
        }
    }
    assert_eq!(
        props_counts,
        vec![
            ("P-1001".to_string(), 3),
            ("L-1001".to_string(), 2),
            ("L-1001-01".to_string(), 0),
            ("V-1001".to_string(), 0),
            ("TI-1001".to_string(), 0),
        ],
        "fixture 的位号与 props 键数必须是契约原文的数字"
    );
    assert_eq!(peers.len(), 1, "恰有一个实体带非空 connector_peer：{peers:?}");
    assert_eq!(peers[0].0, "L-1001", "带 connector_peer 的应当是管线 L-1001");
    for key in GRAPH_KEYS {
        assert!(
            doc.xrecord(doc.header.model_space_block_handle, key).is_some(),
            "fixture 应当已写入图级 key {key}"
        );
    }
}

/// 读回后 [`GRAPH_KEYS`] 的每个 key 是否都在、且都挂在模型空间的扩展字典上。
fn assert_graph_keys_present(doc: &Document) -> Vec<LossItem> {
    let mut losses = Vec::new();
    let dictionary = doc.extension_dictionary_handle(doc.header.model_space_block_handle);
    for key in GRAPH_KEYS {
        match doc.xrecord(doc.header.model_space_block_handle, key) {
            Some(record) => {
                if Some(record.owner) != dictionary {
                    losses.push(LossItem::new(
                        None,
                        format!("{key}:owner"),
                        format!("{dictionary:?}"),
                        format!("{}", record.owner),
                    ));
                }
            }
            None => losses.push(LossItem::new(
                None,
                format!("{key}:key_missing"),
                "present",
                "missing",
            )),
        }
    }
    losses
}

/// 读回文档里 `XRECORD` 的类号，用于 R14 的第二层软断言。
fn xrecord_class_number(doc: &Document) -> Option<i16> {
    doc.classes.get_by_name("XRECORD").map(|class| class.class_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VAL-SEM-034`：DWG 2018 往返位号与属性零丢失。
    #[test]
    fn pid_author_dwg2018_roundtrip_zero_loss() {
        let (doc, handles) = fixture();
        assert_no_transform_sensitive_variants(&doc);
        assert_fixture_shape(&doc, &handles);

        let before = snapshot(&doc, &handles);
        let bytes = crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032)
            .expect("save dwg 2018");
        let back = crate::io::load_bytes("roundtrip.dwg", bytes).expect("load dwg 2018");
        assert_no_transform_sensitive_variants(&back);
        let after = snapshot(&back, &handles);

        let mut losses = diff(&before, &after);
        losses.extend(assert_graph_keys_present(&back));
        let loss_count = losses.len();
        assert_eq!(
            loss_count,
            0,
            "DWG 2018 往返丢失 {loss_count} 项：\n{}",
            loss_table(&losses)
        );

        // 九键逐实体再确认一遍位号与属性（契约点名的那两项）。
        let equipment = PidAuthor::load(&back, handles[0]).expect("设备读回");
        assert_eq!(equipment.tag, "P-1001");
        assert_eq!(equipment.type_code, "EQUIPMENT");
        assert_eq!(persist::props_len(&equipment.props), 3);
        let line = PidAuthor::load(&back, handles[1]).expect("管线读回");
        assert_eq!(line.tag, "L-1001");
        assert_eq!(line.connector_peer, "PID-1102:601");
        assert_eq!(persist::props_len(&line.props), 2);
    }

    /// `REQ-COLL-015` 第一条验收：三项协同状态随图保存，重新打开读回的
    /// 值不变——连同排队未答的 op 一起。
    #[test]
    fn collab_state_survives_a_dwg_roundtrip() {
        let (doc, ..) = fixture();
        let bytes =
            crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032).expect("save dwg 2018");
        let back = crate::io::load_bytes("rt-collab.dwg", bytes).expect("load dwg 2018");

        let state = crate::pid::collab_state::load_state(&back).expect("协同记录读回");
        assert_eq!(state.drawing_id, "PID-1101");
        assert_eq!(state.repo_id, "r-fixture");
        assert_eq!(state.revision, 42);

        let queue = crate::pid::collab_state::load_queue(&back);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].op_id, "op-s-1-7");
        assert_eq!(queue[0].base_revision, 42);
        assert_eq!(queue[0].payload["object_ref"], "V-1001");
        let inverse = queue[0].inverse.as_deref().expect("反向修改读回");
        assert_eq!(inverse.len(), 1);
        assert_eq!(inverse[0]["value"], "1.0MPa");
    }

    /// `OFF-6`：三条协同记录（记录、队列、未同步清单）DWG 与 DXF 各存一次
    /// 再读回，逐项与存之前相等（离线规格 §5.1）。
    #[test]
    fn the_three_collab_records_read_back_alike_from_dwg_and_dxf() {
        let (doc, handles) = fixture();
        assert_fixture_shape(&doc, &handles);
        let keys = [
            PID_AUTHOR_COLLAB,
            PID_AUTHOR_COLLAB_QUEUE,
            PID_AUTHOR_COLLAB_UNSYNCED,
        ];
        for format in ["dwg", "dxf"] {
            let bytes = crate::io::save_to_bytes(&doc, format, DxfVersion::AC1032)
                .unwrap_or_else(|error| panic!("save {format}: {error:?}"));
            let back = crate::io::load_bytes(&format!("rt-collab.{format}"), bytes)
                .unwrap_or_else(|error| panic!("load {format}: {error:?}"));
            for key in keys {
                assert_eq!(
                    crate::pid::graph_data::load(&back, key),
                    crate::pid::graph_data::load(&doc, key),
                    "{format} 读回的 {key}"
                );
            }
        }
    }

    /// 把 fixture 存成一份 DWG 2018 文件，路径由 `OCS_M3_BASELINE_OUT` 给出：
    /// `VAL-CROSS-005` 起步用的「M3 基线文件」。程序里没有哪条命令会把类型库
    /// 或位号规则表写进图纸（它们只经持久化接口写入），所以这份图只能这样造。
    /// 这是生成器，不是检查：平时忽略，要用时带着环境变量、加 `--ignored` 跑。
    ///
    /// 基线图是一份还没有进过任何协同房间的图，所以写出去之前去掉 fixture 里
    /// 那三条协同记录（`REQ-COLL-015`）：带着它们，这份图一连上服务器就会因
    /// 库标识不符被拒（`REPO_MISMATCH`），用它起步的场景就不是从干净的图开始。
    #[test]
    #[ignore = "生成器：把 OCS_M3_BASELINE_OUT 设为要写出的文件路径"]
    fn write_the_m3_baseline_drawing() {
        let path = std::env::var_os("OCS_M3_BASELINE_OUT")
            .expect("把 OCS_M3_BASELINE_OUT 设为要写出的文件路径");
        let (mut doc, handles) = fixture();
        assert_no_transform_sensitive_variants(&doc);
        assert_fixture_shape(&doc, &handles);
        crate::pid::collab_state::store_state(&mut doc, None);
        crate::pid::collab_state::store_queue(&mut doc, &[]);
        crate::pid::collab_state::store_unsynced(&mut doc, &[]);
        let bytes =
            crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1032).expect("save dwg 2018");
        std::fs::write(&path, &bytes).expect("写出基线文件");

        // 写出去的这份读回来是完整的：四个 P&ID 图级记录都在，三条协同记录都不在。
        let back = crate::io::load_bytes("baseline.dwg", bytes).expect("load dwg 2018");
        let losses = diff(&snapshot(&doc, &handles), &snapshot(&back, &handles));
        assert!(
            losses.is_empty(),
            "基线文件读回有丢失：\n{}",
            loss_table(&losses)
        );
        let problems: Vec<String> = assert_graph_keys_present(&back)
            .into_iter()
            .map(|loss| loss.key)
            .collect();
        assert_eq!(
            problems,
            [
                format!("{PID_AUTHOR_COLLAB}:key_missing"),
                format!("{PID_AUTHOR_COLLAB_QUEUE}:key_missing"),
                format!("{PID_AUTHOR_COLLAB_UNSYNCED}:key_missing"),
            ],
            "读回的基线文件：四个 P&ID 图级记录都该在，三条协同记录都不该在"
        );
    }

    /// `VAL-SEM-035`：七个版本往返丢失数为 0。
    ///
    /// 每个版本各做一次「保存 → 读回」（DWG 与 DXF 各一遍，`round_trip`
    /// 模板本来就是双格式），每个版本独立断言：位号丢失 0、属性键值对丢失
    /// 0、[`GRAPH_KEYS`] 的 XRecord 齐全。失败时报告版本号与首个丢失项。
    #[test]
    fn pid_author_dwg_all_versions_zero_loss() {
        let (doc, handles) = fixture();
        assert_no_transform_sensitive_variants(&doc);
        assert_fixture_shape(&doc, &handles);
        let before = snapshot(&doc, &handles);

        let mut failures: Vec<(String, LossItem)> = Vec::new();
        for (version, label) in TARGETS {
            let mut version_failures = 0usize;
            for ext in ["dwg", "dxf"] {
                let bytes = match crate::io::save_to_bytes(&doc, ext, version) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        failures.push((
                            label.to_string(),
                            LossItem::new(None, format!("save {ext}"), "", error),
                        ));
                        version_failures += 1;
                        continue;
                    }
                };
                let back = match crate::io::load_bytes(&format!("rt-{label}.{ext}"), bytes) {
                    Ok(back) => back,
                    Err(error) => {
                        failures.push((
                            label.to_string(),
                            LossItem::new(None, format!("load {ext}"), "", error),
                        ));
                        version_failures += 1;
                        continue;
                    }
                };
                assert_no_transform_sensitive_variants(&back);
                let after = snapshot(&back, &handles);
                let mut losses = diff(&before, &after);
                losses.extend(assert_graph_keys_present(&back));
                for loss in &losses {
                    failures.push((label.to_string(), loss.clone()));
                }
                version_failures += losses.len();
            }
            if version_failures == 0 {
                println!("{label}: loss=0");
            }
        }

        if !failures.is_empty() {
            let mut text = String::new();
            for (label, loss) in &failures {
                text.push_str(&format!("{label}: {loss}\n"));
            }
            panic!(
                "{} 个版本共 {} 项丢失，首项是 {}: {}\n{text}",
                TARGETS.len(),
                failures.len(),
                failures[0].0,
                failures[0].1
            );
        }

        // R14 第二层（软断言，语义，只记录不断言）：写出的 XRecord 类型码。
        let bytes = crate::io::save_to_bytes(&doc, "dwg", DxfVersion::AC1014)
            .expect("save dwg R14");
        let back = crate::io::load_bytes("rt-R14.dwg", bytes).expect("load dwg R14");
        let code = xrecord_class_number(&back);
        let verdict = match code {
            Some(number) if number >= 500 => format!("{number}（类号 ≥500，非规范固定码 79）"),
            Some(number) => format!("{number}（规范固定码 79）"),
            None => "类表里没有 XRECORD（写出走固定码 79）".to_string(),
        };
        println!("R14 软断言（第二层，仅记录，不参与断言）: XRECORD type code = {verdict}");
    }
}

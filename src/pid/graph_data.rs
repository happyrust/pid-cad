//! 图级 P&ID 数据：四个具名 XRecord，挂在模型空间 block record 的扩展字典下。
//!
//! 四个 key 逐字为 [`PID_AUTHOR_TYPE_LIB`]（类型库 JSON，与
//! [`super::type_lib::XRECORD_KEY`] 同一个常量）、[`PID_AUTHOR_TAG_RULES`]
//! （位号规则表）、[`PID_AUTHOR_TOPOLOGY`]（拓扑表）、
//! [`PID_AUTHOR_CONNECTORS`]（连接符表）。本层不解释内容：调用方把表交给
//! [`store`]，本层逐条序列化成 XRecord 条目；[`load`] 原样还回。
//!
//! 三条落盘语义：
//!
//! 1. **一 JSON 对象 = 一条 entry。** 整表一个 JSON 会让「条目数」恒为 1，
//!    既测不出整 key 重写是否残留旧条目，也无法逐条比对两个 key 是否互相
//!    污染。条目顺序即数据顺序，读回不排序、不重建 map。
//! 2. **整 key 重写，不逐条增删。** 先 `entries.clear()` 再重填，形状照抄
//!    `io::pid_view_filter::PidViewFilter::store`。
//! 3. **数据清空时删记录，不留空记录。** 空表走 [`remove_xrecord`]：只摘
//!    扩展字典里该 key 的条目并删掉那条 XRecord 对象，字典本身保留 ——
//!    别的 key 还住在里面，空字典也无害。
//!
//! 前缀条目的理由：四个图级 key 与 [`super::persist`] 写的
//! `PID_AUTHOR_ENTITY_<handle>` 溢出记录共处**同一个**扩展字典。没有
//! `kind=` / `key=` 两条标记，[`collect_orphan_props`] 就分不清「这是图级
//! 数据」还是「这是某实体的 props 溢出」，会误删图级数据。代价是「数据
//! 条目数」的口径要排除这两条前缀，见 [`DATA_PREFIX_ENTRIES`]。
//!
//! 单条文本上限：DWG 侧 `XRecordValue::String` 单条上限 65535 字节，且
//! pre-R2007 非 Unicode 分支经 `encode_legacy_string` 后每字符最多膨胀到
//! 7 字节，所以按 [`MAX_TEXT_BYTES`]（2000，与 `type_lib` 的切块口径一致）
//! 切成多条 `part=<序号>:<片段>` 条目，读回按出现顺序拼回一个对象。正常
//! 图级数据远小于此，分片路径只是上限保险。

use document::objects::{ObjectType, XRecordEntry};
use document::{Document, Handle};

use super::persist::{props_ref_of, PID_AUTHOR_APP};
use super::type_lib::XRECORD_KEY;

/// 类型库的 XRecord key。与 [`super::type_lib::XRECORD_KEY`] 是同一个常量，
/// 不重复定义字面量。
pub const PID_AUTHOR_TYPE_LIB: &str = XRECORD_KEY;

/// 位号规则表的 XRecord key。
pub const PID_AUTHOR_TAG_RULES: &str = "PID_AUTHOR_TAG_RULES";

/// 拓扑表的 XRecord key。
pub const PID_AUTHOR_TOPOLOGY: &str = "PID_AUTHOR_TOPOLOGY";

/// 连接符表的 XRecord key。
pub const PID_AUTHOR_CONNECTORS: &str = "PID_AUTHOR_CONNECTORS";

/// [`super::persist`] 写出的每实体 `props` 溢出记录 key 的前缀。
pub const ENTITY_PROPS_PREFIX: &str = "PID_AUTHOR_ENTITY_";

/// 图级数据的语义标记，与 `kind=entity_props` 区分。
const KIND_GRAPH_DATA: &str = "kind=graph_data";

/// 溢出记录的语义标记，由 `super::persist` 写入。
const KIND_ENTITY_PROPS: &str = "kind=entity_props";

/// 自描述前缀，便于孤儿回收识别「这条记录是图级数据」。
const KEY_PREFIX: &str = "key=";

/// 分片条目的前缀，形如 `part=0:<片段>`。
const PART_PREFIX: &str = "part=";

/// 承载文本的组码（DXF 1 = 任意字符串）。
const TEXT_CODE: i32 = 1;

/// 一条文本条目承载的最大字节数。与 `type_lib` 的 2000 字节切块同口径：
/// 远低于 DWG 的 65535，也容得下 pre-R2007 code page 的膨胀。
const MAX_TEXT_BYTES: usize = 2000;

/// 每条图级 XRecord 里不属于数据的条数：`kind=` 与 `key=` 两条前缀。
///
/// 断言「数据条目数」时从 `entries.len()` 里扣掉它，不要写死 2。
pub const DATA_PREFIX_ENTRIES: usize = 2;

/// 四个图级 key，按契约顺序。
pub fn keys() -> [&'static str; 4] {
    [
        PID_AUTHOR_TYPE_LIB,
        PID_AUTHOR_TAG_RULES,
        PID_AUTHOR_TOPOLOGY,
        PID_AUTHOR_CONNECTORS,
    ]
}

/// 记录的所有者：模型空间 block record，每张图恰有一个。
///
/// 与 `io::pid_view_filter` 的 `owner` 同一选择；扩展字典挂在它下面，
/// XRecord 的 `owner` 是**字典** handle 而不是它，差一层。
fn owner(doc: &Document) -> Handle {
    doc.header.model_space_block_handle
}

/// 整 key 写入 `items`：先清后填。空表删除该 key 的 XRecord。
///
/// 条目布局（顺序固定）：
///
/// ```text
/// [0]  code 1  "kind=graph_data"
/// [1]  code 1  "key=<key>"
/// [2..] code 1  <一个对象的 JSON>，或 "part=<序号>:<片段>" 多条
/// ```
pub fn store(doc: &mut Document, key: &str, items: &[serde_json::Value]) {
    let owner = owner(doc);
    if items.is_empty() {
        remove_xrecord(doc, owner, key);
        return;
    }
    let mut entries = Vec::with_capacity(items.len() + DATA_PREFIX_ENTRIES);
    entries.push(XRecordEntry::string(TEXT_CODE, KIND_GRAPH_DATA));
    entries.push(XRecordEntry::string(TEXT_CODE, format!("{KEY_PREFIX}{key}")));
    for item in items {
        let Ok(json) = serde_json::to_string(item) else {
            continue;
        };
        let chunks = split_text(&json);
        if chunks.len() == 1 {
            if let Some(chunk) = chunks.into_iter().next() {
                entries.push(XRecordEntry::string(TEXT_CODE, chunk));
            }
        } else {
            for (index, chunk) in chunks.into_iter().enumerate() {
                entries.push(XRecordEntry::string(
                    TEXT_CODE,
                    format!("{PART_PREFIX}{index}:{chunk}"),
                ));
            }
        }
    }
    doc.ensure_xrecord(owner, key);
    if let Some(record) = doc.xrecord_mut(owner, key) {
        // 整 key 重写：绝不逐条增删，旧条目必须整体消失。
        record.entries.clear();
        record.entries = entries;
    }
}

/// 读回 `key` 的条目列表。记录不存在返回 `None`（空表与无表同义）。
///
/// 跳过 `kind=` / `key=` 两条前缀，其余按 `entries` 顺序收集。解析失败的
/// 条目被跳过，不会把整表读成 `None`；要知道跳过了哪些，用
/// [`load_reporting`]。
pub fn load(doc: &Document, key: &str) -> Option<Vec<serde_json::Value>> {
    load_reporting(doc, key).map(|loaded| loaded.items)
}

/// [`load_reporting`] 读回的一个 key。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    /// 读得出的条目，按 `entries` 顺序。
    pub items: Vec<serde_json::Value>,
    /// 读不出的条目的原文，按 `entries` 顺序：不是 JSON 的一条，或拼好后
    /// 仍不是 JSON 的分片（拼好的整段）。
    pub unreadable: Vec<String>,
}

/// 同 [`load`]，读不出的条目不丢：原文收在 [`Loaded::unreadable`]。
pub fn load_reporting(doc: &Document, key: &str) -> Option<Loaded> {
    let record = doc.xrecord(owner(doc), key)?;
    let mut loaded = Loaded::default();
    let mut pending = String::new();
    for entry in &record.entries {
        let Some(text) = entry.value.as_string() else {
            continue;
        };
        if text == KIND_GRAPH_DATA || text.starts_with(KEY_PREFIX) {
            continue;
        }
        if let Some(rest) = text.strip_prefix(PART_PREFIX) {
            if let Some((index, chunk)) = rest.split_once(':') {
                // 序号 0 是下一条的头一片：紧挨着的上一条分片条目到此为止。
                if index == "0" {
                    flush_pending(&mut pending, &mut loaded);
                }
                pending.push_str(chunk);
                continue;
            }
        }
        flush_pending(&mut pending, &mut loaded);
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => loaded.items.push(value),
            Err(_) => loaded.unreadable.push(text.to_owned()),
        }
    }
    flush_pending(&mut pending, &mut loaded);
    Some(loaded)
}

/// 数据条目数（排除 `kind=` / `key=` 两条前缀）。记录不存在按 0 算。
pub fn item_count(doc: &Document, key: &str) -> usize {
    doc.xrecord(owner(doc), key)
        .map(|record| record.entries.len().saturating_sub(DATA_PREFIX_ENTRIES))
        .unwrap_or(0)
}

/// 回收孤儿的 `props` 溢出记录，返回删掉几条。
///
/// 三条判据全满足才删（缺一不可）：
///
/// 1. 字典条目的 key 以 [`ENTITY_PROPS_PREFIX`] 开头。前缀匹配大小写敏感 ——
///    `Handle` 的 `Display` 是 `{:#X}`，后缀是大写十六进制。
/// 2. 该 XRecord 的 `entries` 里存在 `kind=entity_props`。同名异用的记录
///    不动。
/// 3. 记录里 `handle=` 解析出的实体**已经不存在**，或该实体**已无**
///    `PID_AUTHOR` 记录，或该实体的 `PID_AUTHOR` 记录里**没有指向本 key 的**
///    `props_ref=`。
///
/// 第 3 条最后一种情形最常见：实体还在、`PID_AUTHOR` 也还在，但 `props`
/// 缩到不再溢出 —— `super::persist` 把 `props_ref=` 换回内联 `props=`，
/// 旧溢出记录必须回收，否则 DWG 会无限膨胀。
///
/// 前提：判据用 [`Document::get_entity`]，它只看 `doc.entities`，看不到块内
/// 实体。P&ID 语义实体全在模型空间，所以够用；将来若支持块内实例，这条
/// 判据要换成遍历 `doc.block_records` 的成员，不能假装通用。
pub fn collect_orphan_props(doc: &mut Document) -> usize {
    let owner = owner(doc);
    let Some(dictionary_handle) = doc.extension_dictionary_handle(owner) else {
        return 0;
    };
    let candidates: Vec<(String, Handle)> = match doc.objects.get(&dictionary_handle) {
        Some(ObjectType::Dictionary(dictionary)) => dictionary
            .entries
            .iter()
            .filter(|(name, _)| name.starts_with(ENTITY_PROPS_PREFIX))
            .cloned()
            .collect(),
        _ => return 0,
    };
    let mut removed = 0;
    for (key, record_handle) in candidates {
        if !is_entity_props_record(doc, record_handle) {
            continue;
        }
        let Some(entity_handle) = props_record_entity(doc, record_handle) else {
            continue;
        };
        if props_ref_still_live(doc, entity_handle, &key) {
            continue;
        }
        let Some(ObjectType::Dictionary(dictionary)) = doc.objects.get_mut(&dictionary_handle)
        else {
            return removed;
        };
        let Some((_, record_handle)) = dictionary.remove_entry(&key) else {
            continue;
        };
        doc.objects.remove(&record_handle);
        removed += 1;
    }
    removed
}

/// 该 XRecord 是不是 `super::persist` 写的溢出记录。
fn is_entity_props_record(doc: &Document, record_handle: Handle) -> bool {
    matches!(
        doc.objects.get(&record_handle),
        Some(ObjectType::XRecord(record))
            if record
                .entries
                .iter()
                .any(|entry| entry.value.as_string() == Some(KIND_ENTITY_PROPS))
    )
}

/// 溢出记录自述的实体 handle（`handle=<u64 十进制>`）。
fn props_record_entity(doc: &Document, record_handle: Handle) -> Option<Handle> {
    let Some(ObjectType::XRecord(record)) = doc.objects.get(&record_handle) else {
        return None;
    };
    record.entries.iter().find_map(|entry| {
        let text = entry.value.as_string()?;
        let value = text.strip_prefix("handle=")?;
        value.parse::<u64>().ok().map(Handle::new)
    })
}

/// 实体还在、`PID_AUTHOR` 记录还在、且该记录仍指向 `key` 这条溢出记录。
fn props_ref_still_live(doc: &Document, entity_handle: Handle, key: &str) -> bool {
    let Some(entity) = doc.get_entity(entity_handle) else {
        return false;
    };
    let Some(record) = entity
        .common()
        .extended_data
        .get_record(PID_AUTHOR_APP)
    else {
        return false;
    };
    props_ref_of(&record.values).as_deref() == Some(key)
}

/// 摘掉 `owner` 扩展字典里 `key` 这条记录并删掉对象。字典本身保留。
///
/// 形状照抄 `io::pid_view_filter::remove_xrecord` 与 `pid::type_lib` 的同名
/// 私有函数：`codec` 没有 `doc.remove_xrecord` 这个 API。
fn remove_xrecord(doc: &mut Document, owner: Handle, key: &str) {
    let Some(dictionary_handle) = doc.extension_dictionary_handle(owner) else {
        return;
    };
    let Some(ObjectType::Dictionary(dictionary)) = doc.objects.get_mut(&dictionary_handle) else {
        return;
    };
    let Some(index) = dictionary
        .entries
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(key))
    else {
        return;
    };
    let (_, record_handle) = dictionary.entries.remove(index);
    dictionary.hard_owner_entries.retain(|name| !name.eq_ignore_ascii_case(key));
    doc.objects.remove(&record_handle);
}

/// 把一条 JSON 按 UTF-8 边界切成不超过 [`MAX_TEXT_BYTES`] 的片段。
fn split_text(json: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in json.chars() {
        if current.len() + ch.len_utf8() > MAX_TEXT_BYTES {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// 把攒下的分片拼回一个对象，解析失败就记作读不出的一条。
fn flush_pending(pending: &mut String, loaded: &mut Loaded) {
    if pending.is_empty() {
        return;
    }
    let text = std::mem::take(pending);
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => loaded.items.push(value),
        Err(_) => loaded.unreadable.push(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::persist::PidAuthor;
    use document::entities::Line;
    use document::types::Vector3;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn line(y: f64) -> document::EntityType {
        document::EntityType::Line(Line::from_points(
            Vector3::new(0.0, y, 0.0),
            Vector3::new(10.0, y, 0.0),
        ))
    }

    /// 契约里的类型库 JSON：`standard` + `types` 两个类型（D2 裁决的形状）。
    fn type_lib_table() -> Vec<serde_json::Value> {
        vec![json!({
            "standard": "内置示例标准",
            "types": [
                {"type_code": "EQUIPMENT", "label": "设备", "parent": null},
                {"type_code": "VESSEL", "label": "容器", "parent": "EQUIPMENT"},
            ],
        })]
    }

    /// 两条位号规则。
    fn tag_rule_table() -> Vec<serde_json::Value> {
        vec![
            json!({"name": "设备位号", "type": "EQUIPMENT", "pattern": "E-{序号}"}),
            json!({"name": "容器位号", "type": "VESSEL", "pattern": "V-{序号}"}),
        ]
    }

    /// 一条管线 + 两条管路，装在一个拓扑对象里。
    fn topology_table() -> Vec<serde_json::Value> {
        vec![json!({
            "pipelines": [{"id": "PL-1", "from": "V-1001", "to": "P-1001"}],
            "runs": [{"id": "R-1"}, {"id": "R-2"}],
        })]
    }

    /// 两条连接符。
    fn connector_table() -> Vec<serde_json::Value> {
        vec![
            json!({"peer": "PID-1102:2F3A", "sheet": 1}),
            json!({"peer": "PID-1102:2F3B", "sheet": 2}),
        ]
    }

    fn dictionary_of(doc: &Document) -> Handle {
        doc.extension_dictionary_handle(doc.header.model_space_block_handle)
            .expect("扩展字典应当存在")
    }

    fn entries_of(doc: &Document, key: &str) -> Vec<XRecordEntry> {
        doc.xrecord(doc.header.model_space_block_handle, key)
            .unwrap_or_else(|| panic!("{key} 的 XRecord 应当存在"))
            .entries
            .clone()
    }

    fn printed(doc: &Document, key: &str) -> String {
        match load(doc, key) {
            Some(items) => format!(
                "{key}: {} 条数据 / {} 条 entries: {}",
                items.len(),
                doc.xrecord(doc.header.model_space_block_handle, key)
                    .map(|record| record.entries.len())
                    .unwrap_or(0),
                serde_json::to_string(&items).unwrap_or_default()
            ),
            None => format!("{key}: 无记录"),
        }
    }

    /// VAL-SEM-026：四个 key 各自写入读回，条目数与内容一致，owner 都是同一
    /// 个扩展字典 handle。
    #[test]
    fn pid_author_xrecord_four_keys_roundtrip() {
        let mut doc = Document::new();
        let tables = [
            (PID_AUTHOR_TYPE_LIB, type_lib_table()),
            (PID_AUTHOR_TAG_RULES, tag_rule_table()),
            (PID_AUTHOR_TOPOLOGY, topology_table()),
            (PID_AUTHOR_CONNECTORS, connector_table()),
        ];
        for (key, items) in &tables {
            store(&mut doc, key, items);
        }

        let dictionary = dictionary_of(&doc);
        for (key, expected) in &tables {
            let back = load(&doc, key)
                .unwrap_or_else(|| panic!("{key} 读回应当是 Some，实际 {}", printed(&doc, key)));
            assert_eq!(
                back.len(),
                expected.len(),
                "{key} 条目数不一致：{}",
                printed(&doc, key)
            );
            for (index, (got, want)) in back.iter().zip(expected.iter()).enumerate() {
                assert_eq!(got, want, "{key} 第 {index} 条内容不一致：{}", printed(&doc, key));
            }
            assert_eq!(
                item_count(&doc, key),
                expected.len(),
                "{key} 数据条目数不一致：{}",
                printed(&doc, key)
            );
            let record = doc
                .xrecord(doc.header.model_space_block_handle, key)
                .unwrap_or_else(|| panic!("{key} 的 XRecord 应当存在"));
            assert_eq!(
                record.owner, dictionary,
                "{key} 的 owner 应当是扩展字典 {dictionary}，实际 {}",
                record.owner
            );
            assert_eq!(
                record.entries.len(),
                expected.len() + DATA_PREFIX_ENTRIES,
                "{key} 的 entries 长度应当是前缀 + 数据：{}",
                printed(&doc, key)
            );
        }

        // D2：类型库是 `standard` + `types` 的顶层形状，两个类型。
        let type_lib = load(&doc, PID_AUTHOR_TYPE_LIB).expect("类型库读回");
        assert_eq!(type_lib[0]["standard"], json!("内置示例标准"));
        assert_eq!(
            type_lib[0]["types"].as_array().map(Vec::len),
            Some(2),
            "类型库应当有 2 个类型：{}",
            printed(&doc, PID_AUTHOR_TYPE_LIB)
        );
    }

    /// VAL-SEM-027：3 条规则整 key 重写成 1 条，旧条目不得残留。
    #[test]
    fn pid_author_xrecord_rewrite_drops_stale_entries() {
        let mut doc = Document::new();
        let three = vec![
            json!({"name": "设备位号", "type": "EQUIPMENT"}),
            json!({"name": "容器位号", "type": "VESSEL"}),
            json!({"name": "泵位号", "type": "PUMP"}),
        ];
        store(&mut doc, PID_AUTHOR_TAG_RULES, &three);
        assert_eq!(item_count(&doc, PID_AUTHOR_TAG_RULES), 3);

        let one = vec![json!({"name": "设备位号", "type": "EQUIPMENT"})];
        store(&mut doc, PID_AUTHOR_TAG_RULES, &one);

        let entries = entries_of(&doc, PID_AUTHOR_TAG_RULES);
        assert_eq!(
            item_count(&doc, PID_AUTHOR_TAG_RULES),
            1,
            "重写后数据条目数应当为 1：{}",
            printed(&doc, PID_AUTHOR_TAG_RULES)
        );
        assert_eq!(
            entries.len(),
            DATA_PREFIX_ENTRIES + 1,
            "重写后 entries 长度应当是 {} + 1，实际 {}：{}",
            DATA_PREFIX_ENTRIES,
            entries.len(),
            printed(&doc, PID_AUTHOR_TAG_RULES)
        );

        let back = load(&doc, PID_AUTHOR_TAG_RULES).expect("读回");
        let text = serde_json::to_string(&back).unwrap_or_default();
        assert!(
            !text.contains("VESSEL"),
            "VESSEL 规则必须消失：{}",
            printed(&doc, PID_AUTHOR_TAG_RULES)
        );
        assert!(
            !text.contains("PUMP"),
            "PUMP 规则必须消失：{}",
            printed(&doc, PID_AUTHOR_TAG_RULES)
        );
        assert_eq!(back, one, "读回内容应当只剩那一条：{}", printed(&doc, PID_AUTHOR_TAG_RULES));
    }

    /// VAL-SEM-028：只重写 `PID_AUTHOR_TOPOLOGY`（改为空拓扑），另外三个
    /// key 的 entries 逐条不变。
    #[test]
    fn pid_author_xrecord_keys_are_independent() {
        let mut doc = Document::new();
        for (key, items) in [
            (PID_AUTHOR_TYPE_LIB, type_lib_table()),
            (PID_AUTHOR_TAG_RULES, tag_rule_table()),
            (PID_AUTHOR_TOPOLOGY, topology_table()),
            (PID_AUTHOR_CONNECTORS, connector_table()),
        ] {
            store(&mut doc, key, &items);
        }

        let untouched = [PID_AUTHOR_TYPE_LIB, PID_AUTHOR_TAG_RULES, PID_AUTHOR_CONNECTORS];
        let before: Vec<Vec<XRecordEntry>> = untouched
            .iter()
            .map(|key| entries_of(&doc, key))
            .collect();

        // 空拓扑：走 VAL-SEM-029 的删除路径，记录整体消失。
        store(&mut doc, PID_AUTHOR_TOPOLOGY, &[]);

        for (key, expected) in untouched.iter().zip(before.iter()) {
            assert_eq!(
                &entries_of(&doc, key),
                expected,
                "{key} 不应当被拓扑重写影响：{}",
                printed(&doc, key)
            );
        }
        assert!(
            doc.xrecord(doc.header.model_space_block_handle, PID_AUTHOR_TOPOLOGY)
                .is_none(),
            "空拓扑应当删掉记录，而不是留一条空记录"
        );
        assert_eq!(
            load(&doc, PID_AUTHOR_TOPOLOGY).map(|items| items.len()),
            None,
            "空拓扑读回应当是 None（无表）"
        );
        assert_eq!(
            load(&doc, PID_AUTHOR_TYPE_LIB).expect("类型库仍在")[0]["types"]
                .as_array()
                .map(Vec::len),
            Some(2),
            "类型库仍是 2 个类型：{}",
            printed(&doc, PID_AUTHOR_TYPE_LIB)
        );
        assert_eq!(load(&doc, PID_AUTHOR_TAG_RULES).map(|items| items.len()), Some(2));
        assert_eq!(
            load(&doc, PID_AUTHOR_CONNECTORS).map(|items| items.len()),
            Some(2)
        );
    }

    /// VAL-SEM-029：写入 2 条后清空再存盘，记录被删除而非留空；其余三个
    /// key 不受影响。同进程与 DWG 往返两种都测。
    #[test]
    fn pid_author_xrecord_removed_when_empty() {
        use document::{Dwg, Format};

        let mut doc = Document::new();
        let handle = doc.add_entity(line(0.0)).unwrap();
        for (key, items) in [
            (PID_AUTHOR_TYPE_LIB, type_lib_table()),
            (PID_AUTHOR_TAG_RULES, tag_rule_table()),
            (PID_AUTHOR_TOPOLOGY, topology_table()),
            (PID_AUTHOR_CONNECTORS, connector_table()),
        ] {
            store(&mut doc, key, &items);
        }
        let untouched = [PID_AUTHOR_TYPE_LIB, PID_AUTHOR_TAG_RULES, PID_AUTHOR_TOPOLOGY];
        let before: Vec<Vec<XRecordEntry>> = untouched
            .iter()
            .map(|key| entries_of(&doc, key))
            .collect();

        store(&mut doc, PID_AUTHOR_CONNECTORS, &[]);

        // 同进程
        assert!(
            doc.xrecord(doc.header.model_space_block_handle, PID_AUTHOR_CONNECTORS)
                .is_none(),
            "清空后记录必须被删除，实际 entries 长度 {:?}",
            doc.xrecord(doc.header.model_space_block_handle, PID_AUTHOR_CONNECTORS)
                .map(|record| record.entries.len())
        );
        assert!(
            doc.extension_dictionary_handle(doc.header.model_space_block_handle)
                .is_some(),
            "扩展字典本身必须保留"
        );
        for (key, expected) in untouched.iter().zip(before.iter()) {
            assert_eq!(
                &entries_of(&doc, key),
                expected,
                "{key} 不应当被连接符清空影响"
            );
        }

        // DWG 往返：证明写出时不会再生成一条空 XRecord。
        let (bytes, losses) = Dwg.write(&doc).expect("DWG write");
        assert!(losses.is_empty(), "{losses:?}");
        let (back, read_losses) = Dwg.read(&bytes).expect("DWG read");
        assert!(read_losses.is_empty(), "{read_losses:?}");
        assert!(
            back.get_entity(handle).is_some(),
            "实体 {handle} 在往返中丢失"
        );
        assert!(
            back
                .xrecord(back.header.model_space_block_handle, PID_AUTHOR_CONNECTORS)
                .is_none(),
            "DWG 往返后连接符记录仍应不存在"
        );
        for (key, expected) in untouched.iter().zip(before.iter()) {
            assert_eq!(
                &entries_of(&back, key),
                expected,
                "{key} 在 DWG 往返后应当逐条不变"
            );
        }
    }

    /// 孤儿回收：3 个孤儿（实体删了 / `PID_AUTHOR` 没了 / props 缩回内联）
    /// 被删，1 个仍在用的留下。
    #[test]
    fn pid_author_xrecord_collects_orphan_props() {
        let mut doc = Document::new();
        // 60 个键、每个 20 字符，稳定超过 XDATA_SOFT_LIMIT。
        let mut props = BTreeMap::new();
        for index in 0..60 {
            props.insert(format!("K{index:019}"), format!("V{index:019}"));
        }
        let props_json = serde_json::to_string(&props).expect("props json");
        let mut author = PidAuthor::new("EQUIPMENT", "E-1001");
        author.props = props_json;

        let handles: Vec<Handle> = (0..4)
            .map(|index| doc.add_entity(line(index as f64)).unwrap())
            .collect();
        for handle in &handles {
            author.store(&mut doc, *handle);
        }
        let keys: Vec<String> = handles
            .iter()
            .map(|handle| crate::pid::persist::props_ref_key(*handle))
            .collect();
        for key in &keys {
            assert!(
                doc.xrecord(doc.header.model_space_block_handle, key).is_some(),
                "溢出记录 {key} 应当已写出"
            );
        }

        // 孤儿 1：实体被删。
        doc.remove_entity(handles[0]);
        // 孤儿 2：实体还在，`PID_AUTHOR` 记录被摘掉。
        crate::scene::view::dispatch::set_entity_xdata(&mut doc, handles[1], PID_AUTHOR_APP, None);
        // 孤儿 3：实体与记录都在，但 props 缩到不再溢出，`props_ref=` 变回
        // 内联 `props=`。
        let mut small = PidAuthor::new("EQUIPMENT", "E-1003");
        small.props = r#"{"公称直径":"DN50"}"#.into();
        small.store(&mut doc, handles[2]);
        // 非孤儿：handles[3] 保持溢出状态。

        let removed = collect_orphan_props(&mut doc);
        assert_eq!(removed, 3, "应当回收 3 条孤儿，实际 {removed}");
        for key in &keys[..3] {
            assert!(
                doc.xrecord(doc.header.model_space_block_handle, key).is_none(),
                "孤儿记录 {key} 应当被删"
            );
        }
        assert!(
            doc.xrecord(doc.header.model_space_block_handle, &keys[3])
                .is_some(),
            "仍在用的溢出记录 {} 必须保留",
            keys[3]
        );
        assert!(
            doc.extension_dictionary_handle(doc.header.model_space_block_handle)
                .is_some(),
            "扩展字典必须保留"
        );
        // 图级数据不会被误判成溢出记录。
        store(&mut doc, PID_AUTHOR_TOPOLOGY, &topology_table());
        assert_eq!(collect_orphan_props(&mut doc), 0, "图级数据不得被回收");
        assert!(
            doc.xrecord(doc.header.model_space_block_handle, PID_AUTHOR_TOPOLOGY)
                .is_some(),
            "图级数据必须还在"
        );
    }

    // ── 与 type_lib 的跨布局往返 ─────────────────────────────────────────

    /// `graph_data::store` 写 + `TypeLib::load` 读：必须往返相等。
    #[test]
    fn type_lib_reads_what_graph_data_wrote() {
        use crate::pid::type_lib::{TypeLib, TypeNode};
        let lib = TypeLib::new(vec![
            TypeNode::new("EQUIPMENT", None::<String>).with_label("设备"),
            TypeNode::new("VESSEL", Some("EQUIPMENT")).with_label("容器"),
        ])
        .expect("夹具库应当构造成功");
        let value = serde_json::to_value(&lib).expect("TypeLib 必须可序列化");

        let mut doc = Document::new();
        store(&mut doc, PID_AUTHOR_TYPE_LIB, &[value]);

        let loaded = TypeLib::load(&doc).expect("graph_data 写的库应当能被 TypeLib 读回");
        assert_eq!(loaded, lib, "跨布局读回应当与原始库相等");
    }

    /// `TypeLib::store` 写 + `graph_data::load` 读：必须往返相等。
    #[test]
    fn graph_data_reads_what_type_lib_wrote() {
        use crate::pid::type_lib::TypeLib;
        let lib = TypeLib::embedded();
        let mut doc = Document::new();
        lib.store(&mut doc);

        let items =
            load(&doc, PID_AUTHOR_TYPE_LIB).expect("TypeLib 写的记录应当能被 graph_data 读回");
        assert_eq!(
            items.len(),
            1,
            "库应当恰有一条数据条目，实际 {}",
            items.len()
        );
        let parsed: TypeLib = serde_json::from_value(items.into_iter().next().expect("至少一条"))
            .expect("条目应当能解析成 TypeLib");
        assert_eq!(parsed, lib, "反向跨布局读回应当与原始库相等");
    }

    /// 大库（>2000 字节）在统一布局下仍分片，且跨布局读回一致。
    #[test]
    fn a_large_type_lib_survives_the_unified_layout() {
        use crate::pid::type_lib::{DataElement, TypeLib, TypeNode};
        let mut equipment = TypeNode::new("EQUIPMENT", None::<String>).with_label("设备");
        for index in 0..200 {
            equipment = equipment.with_element(DataElement::enumeration(
                format!("CODE{index:03}"),
                format!("要素{index:03}"),
                ["碳钢", "不锈钢", "铸铁"],
            ));
        }
        let lib = TypeLib::new(vec![equipment]).expect("大库应当构造成功");
        assert!(
            serde_json::to_string(&lib).expect("可序列化").len() > MAX_TEXT_BYTES,
            "夹具应当超过单条上限"
        );

        let mut doc = Document::new();
        lib.store(&mut doc);
        let record = doc
            .xrecord(doc.header.model_space_block_handle, PID_AUTHOR_TYPE_LIB)
            .expect("应当写入记录");
        assert!(
            record.entries.len() > DATA_PREFIX_ENTRIES + 1,
            "大库应当切成多条 part= 条目，实际 {} 条",
            record.entries.len()
        );

        assert_eq!(TypeLib::load(&doc).expect("分片写读应当一致"), lib);
        let items = load(&doc, PID_AUTHOR_TYPE_LIB).expect("graph_data 也应能读回");
        assert_eq!(
            items.len(),
            1,
            "分片应当拼回一条数据条目，实际 {}",
            items.len()
        );
    }

    /// 相邻两条都超过单条上限、都切成分片：读回仍是两条，按原顺序。
    #[test]
    fn two_split_items_in_a_row_come_back_as_two() {
        let first = serde_json::json!({ "which": "first", "text": "甲".repeat(MAX_TEXT_BYTES) });
        let second = serde_json::json!({ "which": "second", "text": "乙".repeat(MAX_TEXT_BYTES) });
        let mut doc = Document::new();
        store(
            &mut doc,
            PID_AUTHOR_TOPOLOGY,
            &[first.clone(), second.clone()],
        );

        assert_eq!(
            load(&doc, PID_AUTHOR_TOPOLOGY),
            Some(vec![first, second]),
            "相邻两条分片条目应当各自拼回"
        );
    }
}

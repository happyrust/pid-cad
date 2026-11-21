//! 属性下传（`VAL-SEM-036`..`038`）。
//!
//! 管线 → 管路 → 管段三级下传，覆盖标记阻止后续下传，设备改位号后
//! 管口前缀重算并阻断冲突保存。
//!
//! ## 分层
//!
//! 本模块是**纯计算层**：输入文档现值，输出一批 [`PropagationWrite`]，
//! 由调用方一次性应用（[`apply`]）。undo 快照由 app 层用**一个**
//! `push_undo_snapshot` 包住整批写入 —— `push_undo_snapshot` 是
//! `pub(super)`（`src/app/history.rs:453`），`src/pid/` 物理上无法调用，
//! 所以本模块不碰撤销。
//!
//! ## 求值顺序（集成方保证）
//!
//! 位号前缀继承 → 唯一校验 → 规则上溯继承 → **属性下传** → 页间连接属性
//! 传递。本模块只负责第 4 环。
//!
//! ## 覆盖标记
//!
//! 载体是 `props` JSON 内部的 `__overrides` 字符串数组（下划线前缀，不进
//! 属性键值对计数）：
//!
//! ```json
//! {"公称直径":"DN50","管道等级":"B2C","__overrides":["管道等级"]}
//! ```
//!
//! 覆盖是**字段级**：`管道等级` 被覆盖不影响同实体继承 `公称直径`。
//! **只有手工改值才打标记**（[`set_prop_manual`]），下传写入一律不打
//! （否则第一次下传就把子实体锁死）。
//!
//! ## 幂等
//!
//! 值相等即短路，不产生 `PropagationWrite`，因此重复设置同一父值不会
//! 改写任何子实体。

use std::collections::{BTreeMap, BTreeSet};

use document::xdata::XDataValue;
use document::{Document, Handle};

use super::persist::{decode, encode, PidAuthor, PID_AUTHOR_APP};
use super::tag_engine::{
    TagError, DUPLICATED_DISPLAY, TAG_STATUS_DUPLICATED, TAG_STATUS_OK,
};

/// 覆盖标记在 `props` JSON 里的键名。下划线前缀，不参与属性键值对计数。
pub const OVERRIDES_KEY: &str = "__overrides";

/// 下传链的五个字段，顺序即写入顺序（保证 XDATA 记录稳定）。
pub const LINE_FIELDS: [&str; 5] = ["管道等级", "公称直径", "绝热等级", "流体介质", "管线类型"];

/// 管口后缀的补零宽度：固定两位，与 `TagRule::pad` 无关。
const NOZZLE_SUFFIX_WIDTH: usize = 2;

// ── 结果形状 ─────────────────────────────────────────────────────────────

/// 一次下传求值的完整结果。调用方必须用**一个** undo 快照包住整批写入。
#[must_use]
#[derive(Debug, Clone, Default)]
pub struct PropagationBatch {
    /// 按 handle 升序稳定排序，便于断言与批处理。
    pub writes: Vec<PropagationWrite>,
}

impl PropagationBatch {
    /// 本批改了几个实体。
    pub fn len(&self) -> usize {
        self.writes.len()
    }

    /// 本批是否没有任何改动（幂等短路后的常见结果）。
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// 一条待落盘的写入。
///
/// `values` 是**完整重建后的记录值**，不是增量补丁 —— `set_entity_xdata`
/// 是整 APPID 重建（`src/scene/view/dispatch.rs:243`），增量语义不存在。
#[must_use]
#[derive(Debug, Clone)]
pub struct PropagationWrite {
    /// 目标实体 handle。
    pub handle: Handle,
    /// 写入的 XDATA 应用名，恒为 `PID_AUTHOR`。
    pub app: &'static str,
    /// 完整重建后的记录值。
    pub values: Vec<XDataValue>,
    /// 为什么写这一条。
    pub reason: PropagationReason,
    /// 解码后的作者记录，[`apply`] 直接用它落盘（含 `props_ref` 溢出处理）。
    pub author: PidAuthor,
}

/// 写入原因。契约断言与日志都用它，而不是靠值去猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropagationReason {
    /// 管线 → 管路。
    LineToRun {
        /// 下传的字段名。
        field: String,
    },
    /// 管路 → 管段。
    RunToSegment {
        /// 下传的字段名。
        field: String,
    },
    /// 设备 → 管口：位号前缀重算。
    NozzlePrefix {
        /// 重算前的位号。
        old: String,
        /// 重算后的位号。
        new: String,
    },
}

/// 保存门禁的错误：任何 PID 实体 `tag_status == duplicated` 即拒绝保存；规则检查的保存门禁
/// 为 `block` 时，有未豁免的错误也拒绝（`SPEC-pid-drc.md` §4.3）。
///
/// 重复位号的契约编码复用 `TagError::DuplicatedBlocksSave` 的
/// `TAG_DUPLICATED_BLOCKS_SAVE`，不另造一套。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidError {
    /// 存在重复位号，禁止保存。
    Tag(TagError),
    /// 有未豁免的错误，禁止保存。
    RuleBlocked {
        /// 未豁免的错误一共几条。
        errors: usize,
        /// 前 5 条，每条是「规则号 消息」。
        first: Vec<String>,
    },
}

impl PidError {
    /// 契约编码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Tag(error) => error.code(),
            Self::RuleBlocked { .. } => "RULE_BLOCKED",
        }
    }
}

impl std::fmt::Display for PidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tag(error) => error.fmt(f),
            Self::RuleBlocked { errors, first } => {
                write!(
                    f,
                    "{}: 有 {errors} 条未豁免的错误，没有保存：{}",
                    self.code(),
                    first.join("；")
                )?;
                if *errors > first.len() {
                    write!(f, "；另有 {} 条", errors - first.len())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for PidError {}

// ── props 与覆盖标记 ─────────────────────────────────────────────────────

/// `props` JSON → （属性键值对, 覆盖标记集）。
///
/// `__overrides` 单独取走，**不进**属性表：带覆盖标记时属性键值对计数
/// 仍是真实字段数（`VAL-SEM-020` 的口径）。解析失败按空表处理。
pub fn split_props(props_json: &str) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut props = BTreeMap::new();
    let mut marks = BTreeSet::new();
    if props_json.trim().is_empty() {
        return (props, marks);
    }
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(props_json) else {
        return (props, marks);
    };
    for (key, value) in map {
        if key == OVERRIDES_KEY {
            if let serde_json::Value::Array(items) = value {
                for item in items {
                    if let serde_json::Value::String(field) = item {
                        marks.insert(field);
                    }
                }
            }
            continue;
        }
        let text = match value {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        };
        props.insert(key, text);
    }
    (props, marks)
}

/// （属性键值对, 覆盖标记集）→ `props` JSON。字段按字典序，`__overrides`
/// 恒在最后且为空时**不落键**（空集与无集合同义，避免 XDATA 无谓膨胀）。
pub fn join_props(props: &BTreeMap<String, String>, marks: &BTreeSet<String>) -> String {
    let mut map = serde_json::Map::with_capacity(props.len() + 1);
    for (key, value) in props {
        map.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    if !marks.is_empty() {
        let items: Vec<serde_json::Value> = marks
            .iter()
            .map(|field| serde_json::Value::String(field.clone()))
            .collect();
        map.insert(OVERRIDES_KEY.to_string(), serde_json::Value::Array(items));
    }
    serde_json::Value::Object(map).to_string()
}

/// 实体的 `__overrides`；记录不存在返回空集。
#[must_use]
pub fn overrides_of(doc: &Document, handle: Handle) -> BTreeSet<String> {
    match PidAuthor::load(doc, handle) {
        Some(author) => split_props(&author.props).1,
        None => BTreeSet::new(),
    }
}

/// 某字段是否被覆盖。
#[must_use]
pub fn is_overridden(doc: &Document, handle: Handle, field: &str) -> bool {
    overrides_of(doc, handle).contains(field)
}

/// 实体的属性键值对（不含 `__overrides`）。
#[must_use]
pub fn props_of(doc: &Document, handle: Handle) -> BTreeMap<String, String> {
    match PidAuthor::load(doc, handle) {
        Some(author) => split_props(&author.props).0,
        None => BTreeMap::new(),
    }
}

/// 手工改一个属性：写值 **且** 打覆盖标记。此后父实体改值不再下传该字段。
///
/// 与下传写入是**两个不同入口**：下传走 [`PropagationWrite`]，绝不打标记。
pub fn set_prop_manual(
    doc: &mut Document,
    handle: Handle,
    field: &str,
    value: &str,
) -> Result<(), TagError> {
    let mut author = PidAuthor::load(doc, handle).ok_or(TagError::NotPidInstance { handle })?;
    let (mut props, mut marks) = split_props(&author.props);
    props.insert(field.to_string(), value.to_string());
    marks.insert(field.to_string());
    author.props = join_props(&props, &marks);
    author.store(doc, handle);
    Ok(())
}

// ── 下传：管线 → 管路 ────────────────────────────────────────────────────

/// 求一条管线的全部下传结果。
///
/// `runs` 是它的管路 handle 列表（由拓扑表给出，本函数不扫描文档结构）。
/// 未覆盖字段按管线**当前值**覆盖管路值；已覆盖字段原样保留；值相等即
/// 短路，不产生写入。
///
/// 新建管路也走这里：`runs` 只含新 handle 时等价于「创建时全量继承」，
/// 见 [`inherit_on_create`]。
pub fn propagate_line_to_runs(
    doc: &Document,
    line_handle: Handle,
    runs: &[Handle],
) -> PropagationBatch {
    let Some(parent) = PidAuthor::load(doc, line_handle) else {
        return PropagationBatch::default();
    };
    let (parent_props, _) = split_props(&parent.props);
    propagate_fields(doc, &parent_props, runs, |field| {
        PropagationReason::LineToRun {
            field: field.to_string(),
        }
    })
}

/// 新建管路时全量继承管线当前值。等价于 [`propagate_line_to_runs`] 只传
/// 一个新 handle，单独命名是为了让集成点读起来是「继承」而不是「下传」。
pub fn inherit_on_create(
    doc: &Document,
    line_handle: Handle,
    run_handle: Handle,
) -> PropagationBatch {
    propagate_line_to_runs(doc, line_handle, &[run_handle])
}

// ── 下传：管路 → 管段 ────────────────────────────────────────────────────

/// 求一条管路的全部下传结果。
///
/// 管段**不直连管线**：管线改值只经管路下传到管段。管段无覆盖标记时其
/// 五字段始终等于宿主管路**当前值**（`VAL-SEM-037`）。
pub fn propagate_run_to_segments(
    doc: &Document,
    run_handle: Handle,
    segments: &[Handle],
) -> PropagationBatch {
    let Some(parent) = PidAuthor::load(doc, run_handle) else {
        return PropagationBatch::default();
    };
    let (parent_props, _) = split_props(&parent.props);
    propagate_fields(doc, &parent_props, segments, |field| {
        PropagationReason::RunToSegment {
            field: field.to_string(),
        }
    })
}

/// 一条父实体把五字段下传给一批子实体的公共实现。
fn propagate_fields(
    doc: &Document,
    parent_props: &BTreeMap<String, String>,
    children: &[Handle],
    reason: impl Fn(&str) -> PropagationReason,
) -> PropagationBatch {
    let mut writes = Vec::new();
    for &handle in children {
        let Some(mut author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        let (mut props, marks) = split_props(&author.props);
        let mut changed: Option<PropagationReason> = None;
        for field in LINE_FIELDS {
            if marks.contains(field) {
                continue;
            }
            let Some(value) = parent_props.get(field) else {
                continue;
            };
            // 值相等短路：保证幂等，也不污染 undo。
            if props.get(field) == Some(value) {
                continue;
            }
            props.insert(field.to_string(), value.clone());
            if changed.is_none() {
                changed = Some(reason(field));
            }
        }
        let Some(reason) = changed else { continue };
        author.props = join_props(&props, &marks);
        let values = encode(&author);
        writes.push(PropagationWrite {
            handle,
            app: PID_AUTHOR_APP,
            values,
            reason,
            author,
        });
    }
    writes.sort_by_key(|write| write.handle);
    PropagationBatch { writes }
}

// ── 下传：设备 → 管口 ────────────────────────────────────────────────────

/// 设备位号变更后重算管口前缀。
///
/// `nozzles` 的顺序决定后缀序号（`{n:02}`，固定两位补零）。重算后立刻做
/// 唯一校验：`(类型编码, 位号)` 复合键被别的实体占用则该管口
/// `tag_status = duplicated`、显示 `？`。
pub fn recompute_nozzle_prefixes(
    doc: &Document,
    equipment_handle: Handle,
    new_tag: &str,
    nozzles: &[Handle],
) -> PropagationBatch {
    let Some(_equipment) = PidAuthor::load(doc, equipment_handle) else {
        return PropagationBatch::default();
    };
    let mut writes = Vec::new();
    for (index, &handle) in nozzles.iter().enumerate() {
        let Some(mut author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        let old = author.tag.clone();
        let tag = nozzle_tag(new_tag, index + 1);
        let status = if tag.is_empty() || occupant_of(doc, &author.type_code, &tag, handle).is_some()
        {
            TAG_STATUS_DUPLICATED
        } else {
            TAG_STATUS_OK
        };
        if old == tag && author.tag_status == status {
            continue;
        }
        author.tag = tag.clone();
        author.tag_status = status.to_string();
        let values = encode(&author);
        writes.push(PropagationWrite {
            handle,
            app: PID_AUTHOR_APP,
            values,
            reason: PropagationReason::NozzlePrefix {
                old,
                new: tag,
            },
            author,
        });
    }
    writes.sort_by_key(|write| write.handle);
    PropagationBatch { writes }
}

/// 管口位号：`<设备位号>-<两位后缀>`。固定两位补零，与 `TagRule::pad` 无关。
#[must_use]
pub fn nozzle_tag(equipment_tag: &str, ordinal: usize) -> String {
    if equipment_tag.is_empty() || ordinal == 0 {
        return String::new();
    }
    format!(
        "{equipment_tag}-{ordinal:0width$}",
        width = NOZZLE_SUFFIX_WIDTH
    )
}

/// 冲突时的显示串（`？`），与位号引擎的 `display` 一致。
#[must_use]
pub fn display_of(author: &PidAuthor) -> String {
    if author.tag_status == TAG_STATUS_DUPLICATED {
        DUPLICATED_DISPLAY.to_string()
    } else {
        author.tag.clone()
    }
}

/// 文档里占用 `(type_code, tag)` 复合键的**其他**实体。
fn occupant_of(
    doc: &Document,
    type_code: &str,
    tag: &str,
    excluding: Handle,
) -> Option<Handle> {
    if tag.is_empty() {
        return None;
    }
    for entity in doc.entities() {
        let handle = entity.common().handle;
        if handle == excluding {
            continue;
        }
        let Some(author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        if author.type_code == type_code && author.tag == tag {
            return Some(handle);
        }
    }
    None
}

// ── 保存门禁 ─────────────────────────────────────────────────────────────

/// 保存前门禁：任一 P&ID 实体 `tag_status == duplicated` 即拒绝保存。
///
/// 返回 `Err(PidError::Tag(TagError::DuplicatedBlocksSave { .. }))`，
/// 其 [`PidError::code`] 为 `TAG_DUPLICATED_BLOCKS_SAVE`。调用点在保存
/// 路径（`src/io/mod.rs` 的 `save_owned_as_version_inner` /
/// `save_to_bytes`），判定发生在任何写盘动作**之前**，所以冲突时不会
/// 产生新文件，也不会碰同目录已有的 `.bak`（`VAL-SEM-039`）。
///
/// 只解码记录本身（[`decode`]），不走 [`PidAuthor::load`] —— `tag_status`
/// 永远在主记录上，`props_ref` 指向的溢出 XRecord 与门禁无关，省掉一次
/// 每个实体的 XRecord 查找。
pub fn gate_save(doc: &Document) -> Result<(), PidError> {
    for entity in doc.entities() {
        let Some(record) = entity.common().extended_data.get_record(PID_AUTHOR_APP) else {
            continue;
        };
        let Some(author) = decode(&record.values) else {
            continue;
        };
        if author.tag_status == TAG_STATUS_DUPLICATED {
            return Err(PidError::Tag(TagError::DuplicatedBlocksSave {
                tag: author.tag,
                type_code: author.type_code,
            }));
        }
    }
    Ok(())
}

// ── 应用 ─────────────────────────────────────────────────────────────────

/// 把一批写入落盘。调用方负责在调用前后各一次 undo 快照的**一次**包围。
///
/// 走 `PidAuthor::store`，因此 `props` 超软上限时自动挪进 XRecord，与单条
/// 写入路径行为一致。
pub fn apply(doc: &mut Document, batch: &PropagationBatch) {
    for write in &batch.writes {
        write.author.store(doc, write.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::persist::PidAuthor;
    use document::entities::Line;
    use document::types::Vector3;

    const LINE: &str = "PIPE_LINE";
    const RUN: &str = "PIPE_RUN";
    const SEGMENT: &str = "PIPE_SEGMENT";
    const NOZZLE: &str = "NOZZLE";
    const EQUIPMENT: &str = "EQUIPMENT";

    fn line(y: f64) -> document::EntityType {
        document::EntityType::Line(Line::from_points(
            Vector3::new(0.0, y, 0.0),
            Vector3::new(10.0, y, 0.0),
        ))
    }

    /// 造一个带 `PID_AUTHOR` 的实体：给定类型、位号、五字段值。
    fn place(
        doc: &mut Document,
        type_code: &str,
        tag: &str,
        grade: &str,
        diameter: &str,
        insulation: &str,
        medium: &str,
        line_type: &str,
    ) -> Handle {
        let handle = doc.add_entity(line(0.0)).expect("entity added");
        let mut props = BTreeMap::new();
        props.insert(LINE_FIELDS[0].to_string(), grade.to_string());
        props.insert(LINE_FIELDS[1].to_string(), diameter.to_string());
        props.insert(LINE_FIELDS[2].to_string(), insulation.to_string());
        props.insert(LINE_FIELDS[3].to_string(), medium.to_string());
        props.insert(LINE_FIELDS[4].to_string(), line_type.to_string());
        PidAuthor {
            type_code: type_code.to_string(),
            tag: tag.to_string(),
            tag_source: "manual".to_string(),
            tag_status: TAG_STATUS_OK.to_string(),
            props: join_props(&props, &BTreeSet::new()),
            ..PidAuthor::default()
        }
        .store(doc, handle);
        handle
    }

    /// 只带位号的实体（管口 / 冲突占位）。
    fn place_tagged(doc: &mut Document, type_code: &str, tag: &str) -> Handle {
        let handle = doc.add_entity(line(0.0)).expect("entity added");
        PidAuthor {
            type_code: type_code.to_string(),
            tag: tag.to_string(),
            tag_source: "manual".to_string(),
            tag_status: TAG_STATUS_OK.to_string(),
            ..PidAuthor::default()
        }
        .store(doc, handle);
        handle
    }

    fn grade_of(doc: &Document, handle: Handle) -> String {
        props_of(doc, handle)
            .get(LINE_FIELDS[0])
            .cloned()
            .unwrap_or_default()
    }

    fn five_fields(doc: &Document, handle: Handle) -> Vec<String> {
        let props = props_of(doc, handle);
        LINE_FIELDS
            .iter()
            .map(|field| props.get(*field).cloned().unwrap_or_default())
            .collect()
    }

    // ── VAL-SEM-036 ──────────────────────────────────────────────────────

    #[test]
    fn pipe_line_property_propagates_to_runs() {
        let mut doc = Document::new();
        let line_h = place(&mut doc, LINE, "L-1001", "A1A", "DN100", "H", "水", "工艺");
        let run_a = place(&mut doc, RUN, "L-1001-01", "A1A", "DN100", "H", "水", "工艺");
        let run_b = place(&mut doc, RUN, "L-1001-02", "A1A", "DN100", "H", "水", "工艺");

        // 管线改值：`管道等级` A1A → A2B，另四项一起改成新值。
        for (field, value) in [
            (LINE_FIELDS[0], "A2B"),
            (LINE_FIELDS[1], "DN150"),
            (LINE_FIELDS[2], "H2"),
            (LINE_FIELDS[3], "蒸汽"),
            (LINE_FIELDS[4], "公用"),
        ] {
            set_prop_manual(&mut doc, line_h, field, value).expect("manual edit");
        }
        let batch = propagate_line_to_runs(&doc, line_h, &[run_a, run_b]);
        apply(&mut doc, &batch);

        let expected = five_fields(&doc, line_h);
        for run in [run_a, run_b] {
            assert_eq!(
                five_fields(&doc, run),
                expected,
                "run {run} must mirror the line; line={:?} run={:?}",
                five_fields(&doc, line_h),
                five_fields(&doc, run)
            );
            assert_eq!(grade_of(&doc, run), "A2B");
            assert!(
                overrides_of(&doc, run).is_empty(),
                "propagation must not mark overrides; run {run} has {:?}",
                overrides_of(&doc, run)
            );
        }
    }

    // ── VAL-SEM-037 ──────────────────────────────────────────────────────

    #[test]
    fn run_override_blocks_inheritance_and_segment_follows_run() {
        let mut doc = Document::new();
        let line_h = place(&mut doc, LINE, "L-1001", "A1A", "DN100", "H", "水", "工艺");
        let run_a = place(&mut doc, RUN, "L-1001-01", "A1A", "DN100", "H", "水", "工艺");
        let run_b = place(&mut doc, RUN, "L-1001-02", "A1A", "DN100", "H", "水", "工艺");
        let seg_a = place(&mut doc, SEGMENT, "", "A1A", "DN100", "H", "水", "工艺");

        // 第 1 步：手工把 L-1001-01 的管道等级改为 B2C，打上覆盖标记。
        set_prop_manual(&mut doc, run_a, LINE_FIELDS[0], "B2C").expect("manual edit");
        assert!(
            is_overridden(&doc, run_a, LINE_FIELDS[0]),
            "manual edit must mark the field as overridden"
        );

        // 第 2 步：管线改 A3D 并下传。
        set_prop_manual(&mut doc, line_h, LINE_FIELDS[0], "A3D").expect("manual edit");
        let batch = propagate_line_to_runs(&doc, line_h, &[run_a, run_b]);
        apply(&mut doc, &batch);
        assert_eq!(
            grade_of(&doc, run_a),
            "B2C",
            "override must block inheritance: line={:?} run_a={:?} run_b={:?} override={}",
            grade_of(&doc, line_h),
            grade_of(&doc, run_a),
            grade_of(&doc, run_b),
            is_overridden(&doc, run_a, LINE_FIELDS[0])
        );
        assert_eq!(
            grade_of(&doc, run_b),
            "A3D",
            "unmarked run must follow the line: line={:?} run_a={:?} run_b={:?}",
            grade_of(&doc, line_h),
            grade_of(&doc, run_a),
            grade_of(&doc, run_b)
        );

        // 第 3 步：管段无覆盖标记，始终等于宿主管路当前值。
        let batch = propagate_run_to_segments(&doc, run_a, &[seg_a]);
        apply(&mut doc, &batch);
        assert_eq!(
            grade_of(&doc, seg_a),
            grade_of(&doc, run_a),
            "segment must equal its run: run_a={:?} segment={:?} override={}",
            grade_of(&doc, run_a),
            grade_of(&doc, seg_a),
            is_overridden(&doc, seg_a, LINE_FIELDS[0])
        );
        assert_eq!(grade_of(&doc, seg_a), "B2C");

        // 第 4 步：管路再改 C1E，管段立即跟随。
        set_prop_manual(&mut doc, run_a, LINE_FIELDS[0], "C1E").expect("manual edit");
        let batch = propagate_run_to_segments(&doc, run_a, &[seg_a]);
        apply(&mut doc, &batch);
        assert_eq!(
            grade_of(&doc, seg_a),
            "C1E",
            "segment must follow the run immediately: line={:?} run_a={:?} run_b={:?} segment={:?} override={}",
            grade_of(&doc, line_h),
            grade_of(&doc, run_a),
            grade_of(&doc, run_b),
            grade_of(&doc, seg_a),
            is_overridden(&doc, seg_a, LINE_FIELDS[0])
        );
    }

    // ── VAL-SEM-038 ──────────────────────────────────────────────────────

    #[test]
    fn equipment_tag_change_recomputes_nozzles_and_blocks_save() {
        let mut doc = Document::new();
        let equipment = place_tagged(&mut doc, EQUIPMENT, "P-1001");
        let nozzle_a = place_tagged(&mut doc, NOZZLE, "P-1001-01");
        let nozzle_b = place_tagged(&mut doc, NOZZLE, "P-1001-02");

        // P-1001 → P-2001：两个管口前缀立即重算。
        let batch = recompute_nozzle_prefixes(&doc, equipment, "P-2001", &[nozzle_a, nozzle_b]);
        apply(&mut doc, &batch);
        let tags: Vec<String> = [nozzle_a, nozzle_b]
            .iter()
            .map(|handle| PidAuthor::load(&doc, *handle).map(|a| a.tag).unwrap_or_default())
            .collect();
        assert_eq!(tags, vec!["P-2001-01", "P-2001-02"]);

        // 构造冲突：文档里已存在 (NOZZLE, "P-3001-01")。
        let squatter = place_tagged(&mut doc, NOZZLE, "P-3001-01");

        // P-2001 → P-3001：冲突管口变 duplicated、显示 ？。
        let batch = recompute_nozzle_prefixes(&doc, equipment, "P-3001", &[nozzle_a, nozzle_b]);
        apply(&mut doc, &batch);
        let conflicted = PidAuthor::load(&doc, nozzle_a).expect("nozzle loads");
        assert_eq!(conflicted.tag, "P-3001-01");
        assert_eq!(
            conflicted.tag_status, TAG_STATUS_DUPLICATED,
            "conflicting nozzle must be duplicated; tags={:?} squatter={:?}",
            [
                PidAuthor::load(&doc, nozzle_a).map(|a| a.tag),
                PidAuthor::load(&doc, nozzle_b).map(|a| a.tag),
            ],
            PidAuthor::load(&doc, squatter).map(|a| a.tag)
        );
        assert_eq!(display_of(&conflicted), DUPLICATED_DISPLAY);

        // 保存门禁：返回 Err 与契约错误码。
        //
        // 这里**只**断言语义层能证伪的部分。原实现另有两条断言
        // （`!target.exists()` / `backup.exists()`）恒真不可证伪 —— 唯一写
        // target 的动作在 `if outcome.is_ok()` 分支内，而本用例要求它是
        // `Err`，所以该分支永不执行。真实的保存副作用验证在
        // `conflicting_save_is_blocked_on_the_real_save_path`（VAL-SEM-039，
        // M5 `gate_save` 已接入真实保存包装后）。
        let outcome = gate_save(&doc);
        let error = outcome.expect_err("duplicated tag must block save");
        assert_eq!(error.code(), "TAG_DUPLICATED_BLOCKS_SAVE");
    }

    // ── 辅助行为 ─────────────────────────────────────────────────────────

    /// 可写目录：优先 `D:\scratch`（任务约定），不存在时退回系统临时目录。
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let base = if std::path::Path::new("D:\\scratch").is_dir() {
            std::path::PathBuf::from("D:\\scratch")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("ocs-pid-save-gate-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    // ── VAL-SEM-039 ──────────────────────────────────────────────────────

    /// 冲突保存被**真实保存路径**阻断，且不落新文件、不动原有 `.bak`。
    ///
    /// 这是 D30 从 `VAL-SEM-038` 拆出来的那一半：`gate_save` 已接进
    /// `io::save_owned_as_version_atomic` / `io::save_as_version` /
    /// `io::save_to_bytes` 三个真实包装的**前置**位置，所以下面的副作用
    /// 断言都能实际证伪（改前它们是恒真断言，因为根本没有生产调用点）。
    #[test]
    fn conflicting_save_is_blocked_on_the_real_save_path() {
        let dir = scratch_dir("sem-039");
        let target = dir.join("blocked.dwg");
        let backup = target.with_extension("bak");
        let backup_bytes: &[u8] = b"previous backup bytes, must stay byte-identical";
        std::fs::write(&backup, backup_bytes).expect("seed the .bak");
        assert!(!target.exists(), "目标文件一开始就不存在");

        let mut doc = Document::new();
        let _squatter = place_tagged(&mut doc, NOZZLE, "P-3001-01");
        let conflicted = place_tagged(&mut doc, NOZZLE, "P-3001-01");
        let mut author = PidAuthor::load(&doc, conflicted).expect("conflicting nozzle loads");
        author.tag_status = TAG_STATUS_DUPLICATED.to_string();
        author.store(&mut doc, conflicted);

        // (1) 结构化包装：拿到带错误码的失败。
        let failure = crate::io::save_owned_as_version_atomic(
            doc.clone(),
            &target,
            document::DxfVersion::AC1032,
            true,
            None,
            None,
            false,
            false,
            None,
            None,
            true,
        )
        .expect_err("duplicated tag must block the real save path");
        assert_eq!(
            failure.code,
            Some("TAG_DUPLICATED_BLOCKS_SAVE"),
            "保存返回值={failure:?}；目标存在={}；.bak 长度={:?}",
            target.exists(),
            std::fs::metadata(&backup).map(|meta| meta.len())
        );
        assert_eq!(failure.message, failure.message.trim());
        assert!(failure.message.contains("TAG_DUPLICATED_BLOCKS_SAVE"));
        assert!(!target.exists(), "冲突保存不得产生目标文件");
        assert_eq!(
            std::fs::read(&backup).expect("bak readable"),
            backup_bytes,
            "原有 .bak 必须逐字节不变"
        );

        // (2) 字符串包装（`io::save` 的上游）：同样被拒，且错误串带错误码。
        let message = crate::io::save_as_version(&doc, &target, document::DxfVersion::AC1032)
            .expect_err("string save wrapper must refuse too");
        assert!(
            message.contains("TAG_DUPLICATED_BLOCKS_SAVE"),
            "错误串应带错误码：{message}"
        );
        assert!(!target.exists(), "字符串包装也不得落文件");
        assert_eq!(std::fs::read(&backup).expect("bak readable"), backup_bytes);

        // (3) 字节缓冲包装（web 保存路径）同样被拒。
        let bytes_error = crate::io::save_to_bytes(&doc, "dwg", document::DxfVersion::AC1032)
            .expect_err("byte-buffer save wrapper must refuse too");
        assert!(
            bytes_error.contains("TAG_DUPLICATED_BLOCKS_SAVE"),
            "错误串应带错误码：{bytes_error}"
        );

        // (4) 清除冲突后同一包装返回 Ok，目标文件存在且可被 load_bytes 读回。
        let mut author = PidAuthor::load(&doc, conflicted).expect("conflicting nozzle loads");
        author.tag_status = TAG_STATUS_OK.to_string();
        author.store(&mut doc, conflicted);
        assert!(gate_save(&doc).is_ok(), "清除冲突后门禁应放行");

        crate::io::save_owned_as_version_atomic(
            doc.clone(),
            &target,
            document::DxfVersion::AC1032,
            true,
            None,
            None,
            false,
            false,
            None,
            None,
            true,
        )
        .expect("cleared conflict saves through the same wrapper");
        assert!(target.exists(), "成功保存必须落目标文件");
        let saved = std::fs::read(&target).expect("target readable");
        let back = crate::io::load_bytes("blocked.dwg", saved).expect("load_bytes reads it back");
        let restored = PidAuthor::load(&back, conflicted).expect("author record round trips");
        assert_eq!(restored.tag, "P-3001-01");
        assert_eq!(restored.tag_status, TAG_STATUS_OK);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn propagation_is_idempotent_and_overrides_excluded_from_count() {
        let mut doc = Document::new();
        let line_h = place(&mut doc, LINE, "L-1001", "A1A", "DN100", "H", "水", "工艺");
        let run = place(&mut doc, RUN, "L-1001-01", "A1A", "DN100", "H", "水", "工艺");
        assert_eq!(propagate_line_to_runs(&doc, line_h, &[run]).len(), 0);

        set_prop_manual(&mut doc, run, LINE_FIELDS[0], "B2C").expect("manual edit");
        let author = PidAuthor::load(&doc, run).expect("author loads");
        let (props, marks) = split_props(&author.props);
        assert_eq!(props.len(), 5, "overrides must not count as a property");
        assert_eq!(marks.len(), 1);
        // `props_len` 只认 `BTreeMap<String,String>`，遇到数组型的
        // `__overrides` 解析失败返回 0 —— 这正是「覆盖标记不进属性计数」的
        // 旁证：带标记时属性键值对仍是 5。
        let raw: serde_json::Value = serde_json::from_str(&author.props).expect("props parses");
        assert_eq!(
            raw.as_object().map(|map| map.len()),
            Some(6),
            "raw JSON keys include __overrides: {}",
            author.props
        );

        // 父值未变：不产生写入。
        assert!(propagate_line_to_runs(&doc, line_h, &[run]).is_empty());
    }

    #[test]
    fn nozzle_tag_pads_to_two_digits() {
        assert_eq!(nozzle_tag("P-2001", 1), "P-2001-01");
        assert_eq!(nozzle_tag("P-2001", 9), "P-2001-09");
        assert_eq!(nozzle_tag("P-2001", 10), "P-2001-10");
        assert_eq!(nozzle_tag("", 1), "");
    }
}

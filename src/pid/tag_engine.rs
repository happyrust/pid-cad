//! 位号规则引擎：序号空间扫描、模板渲染、后缀补零扩位、唯一校验、
//! `tag_source` / `tag_status` 状态机。
//!
//! 纯数据 + 纯函数，不依赖 iced / Scene。两个入口：
//!
//! - [`generate`] 为新实例求位号，不改动任何既有实例；
//! - [`reevaluate`] 重新求值一个既有实例，`tag_source == manual` 时原样返回。
//!
//! ## 序号空间
//!
//! 序号占用必须来自**文档本身**（XDATA `PID_AUTHOR` 的 `type` 与 `tag`），
//! 不能只靠内存计数器，否则打开已有 DWG 会从起点重来、产生重复位号。
//! 扫描一次全文档建 `HashMap<类型编码, 最大已用序号>` 索引（D11），
//! `generate` 只查表，复杂度随实例数线性而非平方。
//!
//! 三条排除规则：
//!
//! 1. `tag_source == manual` 的实例不占号（`VAL-SEM-012`：存在 `P-9001`
//!    时新实例仍是 `P-1001`）；
//! 2. `tag_status == duplicated` 的实例不占号（它是冲突残值，不是合法占用）；
//! 3. 位号解析不出计数段的不占号（前缀必须与模板骨架匹配，否则该位号属于
//!    另一套模板，不能当本模板的序号用）。
//!
//! ## 失败不消耗序号
//!
//! `VAL-SEM-005`：拼接字段缺失时先算候选、校验全部必填字段，全通过才提交
//! 占用。实现顺序是「扫索引 → 算候选 → 渲染并校验 → 判重 → 出 TagView」，
//! 失败路径在判重之前就 `return`，既不写索引也不推进任何计数器。
//!
//! ## 补零
//!
//! `{序号}` 段按 `TagRule::pad` 补零；`{后缀}` 段**固定两位**
//! （`format!("{n:02}")`），与 `pad` 无关，`-99` 之后是 `-100`，不截断不回绕。

use std::collections::{BTreeMap, HashMap, HashSet};

use document::{Document, Handle};

use super::persist::PidAuthor;
use super::tag_rule::{default_tag_rule, HierarchyMode, TagRule};
use super::type_lib::TypeLib;

/// 位号来源：自动编码。
pub const TAG_SOURCE_AUTO: &str = "auto";
/// 位号来源：手工输入。
pub const TAG_SOURCE_MANUAL: &str = "manual";

/// 位号状态：正常。
pub const TAG_STATUS_OK: &str = "ok";
/// 位号状态：与同类型另一实例重复。
pub const TAG_STATUS_DUPLICATED: &str = "duplicated";

/// 冲突时画布与属性面板显示的占位串。
pub const DUPLICATED_DISPLAY: &str = "？";

/// 重试上限：序号空间被大量手工占位挤满时的兜底，避免死循环。
const MAX_ATTEMPTS: u32 = 100_000;

// ── 视图 ─────────────────────────────────────────────────────────────────

/// 一个实例的位号视图。
///
/// `tag` 是存储值（冲突时保留原值不清空），`display` 是画布 / 面板上显示的串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagView {
    /// 存储值。冲突时保留原值，不清空。
    pub tag: String,
    /// `auto` | `manual`。
    pub tag_source: String,
    /// `ok` | `duplicated`。
    pub tag_status: String,
    /// `duplicated` → `？`；`ok` 且 `tag` 空 → 空串；`ok` 且有值 → `tag`。
    pub display: String,
}

impl TagView {
    /// 由存储值三件套推出显示串。
    pub fn new(
        tag: impl Into<String>,
        tag_source: impl Into<String>,
        tag_status: impl Into<String>,
    ) -> Self {
        let tag = tag.into();
        let tag_source = tag_source.into();
        let tag_status = tag_status.into();
        let display = if tag_status == TAG_STATUS_DUPLICATED {
            DUPLICATED_DISPLAY.to_string()
        } else {
            tag.clone()
        };
        Self {
            tag,
            tag_source,
            tag_status,
            display,
        }
    }

    /// 自动生成且校验通过的视图。
    pub fn auto_ok(tag: impl Into<String>) -> Self {
        Self::new(tag, TAG_SOURCE_AUTO, TAG_STATUS_OK)
    }

    /// 未生成位号（自动编码关闭 / 拼接字段缺失）的视图：空位号、`manual`、
    /// `ok`、显示为空。**不显示 `？`** —— `？` 只留给重复冲突。
    pub fn empty() -> Self {
        Self::new(String::new(), TAG_SOURCE_MANUAL, TAG_STATUS_OK)
    }

    /// 标记为重复冲突：`tag` 保留原值，`tag_status = duplicated`，显示 `？`。
    pub fn duplicated(tag: impl Into<String>, tag_source: impl Into<String>) -> Self {
        Self::new(tag, tag_source, TAG_STATUS_DUPLICATED)
    }
}

// ── 错误 ─────────────────────────────────────────────────────────────────

/// 位号生成 / 校验失败的原因。每个变体带一个契约编码（[`TagError::code`]），
/// 断言与日志都用编码而不是展示文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    /// 拼接字段源属性缺失（`VAL-SEM-005`）。该次失败**不消耗序号**。
    TemplateFieldMissing {
        /// 缺失的属性名，即 `TagRule::join` 声明的那个。
        field: String,
    },
    /// 唯一校验拒绝同类型同位号（`VAL-SEM-010`）。
    Duplicated {
        /// 冲突的位号。
        tag: String,
        /// 冲突的类型编码。
        type_code: String,
        /// 已被占用的实例 handle，用于提示。
        existing: Handle,
    },
    /// 保存前门禁：存在重复位号（本 feature 只定义编码，不实现保存钩子）。
    DuplicatedBlocksSave {
        /// 重复的位号。
        tag: String,
        /// 重复的类型编码。
        type_code: String,
    },
    /// 该类型没有生效的位号规则。
    NoRule {
        /// 类型编码。
        type_code: String,
    },
    /// 类型编码不在类型库里。
    UnknownType {
        /// 类型编码。
        type_code: String,
    },
    /// `handle` 不是带 `PID_AUTHOR` 记录的 P&ID 实例。
    NotPidInstance {
        /// 实体 handle。
        handle: Handle,
    },
}

impl TagError {
    /// 契约编码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::TemplateFieldMissing { .. } => "TAG_TEMPLATE_FIELD_MISSING",
            Self::Duplicated { .. } => "TAG_DUPLICATED",
            Self::DuplicatedBlocksSave { .. } => "TAG_DUPLICATED_BLOCKS_SAVE",
            Self::NoRule { .. } => "TAG_RULE_MISSING",
            Self::UnknownType { .. } => "TAG_TYPE_UNKNOWN",
            Self::NotPidInstance { .. } => "TAG_NOT_PID_INSTANCE",
        }
    }
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TemplateFieldMissing { field } => {
                write!(f, "{}: 拼接字段 {field} 的属性缺失", self.code())
            }
            Self::Duplicated {
                tag,
                type_code,
                existing,
            } => write!(
                f,
                "{}: {type_code} 的位号 {tag} 已被 {existing} 占用",
                self.code()
            ),
            Self::DuplicatedBlocksSave { tag, type_code } => {
                write!(f, "{}: {type_code} 的位号 {tag} 重复，禁止保存", self.code())
            }
            Self::NoRule { type_code } => {
                write!(f, "{}: {type_code} 没有生效的位号规则", self.code())
            }
            Self::UnknownType { type_code } => {
                write!(f, "{}: 类型编码 {type_code} 不在类型库中", self.code())
            }
            Self::NotPidInstance { handle } => {
                write!(f, "{}: {handle} 没有 PID_AUTHOR 记录", self.code())
            }
        }
    }
}

impl std::error::Error for TagError {}

// ── 序号空间索引 ─────────────────────────────────────────────────────────

/// 一次遍历建好的序号占用索引（D11）。
///
/// `max_used` 是「类型编码 → 已用最大计数」，`occupied` 是「类型编码 → 位号
/// → 占用它的 handle」，用于唯一校验（`VAL-SEM-010` / `011` 的复合键口径）。
#[derive(Debug, Default)]
struct SequenceIndex {
    max_used: HashMap<String, i64>,
    occupied: HashMap<String, HashMap<String, Handle>>,
}

impl SequenceIndex {
    /// 扫全文档一次。
    fn scan(lib: &TypeLib, doc: &Document) -> Self {
        let mut index = Self::default();
        for entity in doc.entities() {
            let handle = entity.common().handle;
            let Some(author) = PidAuthor::load(doc, handle) else {
                continue;
            };
            let type_code = author.type_code.as_str();
            if type_code.is_empty() {
                continue;
            }

            // 唯一键是 `(类型编码, 位号)` 复合键，手工位号同样占这个键。
            index.occupy(type_code, &author.tag, handle);

            // 手工位号与冲突残值不参与序号推进（VAL-SEM-012 / VAL-SEM-010）。
            if author.tag_source == TAG_SOURCE_MANUAL
                || author.tag_status == TAG_STATUS_DUPLICATED
                || author.tag.is_empty()
            {
                continue;
            }
            let Some(rule) = resolve_rule(lib, type_code).ok() else {
                continue;
            };
            index.advance(type_code, &author.tag, &rule, &parse_props(&author.props));
        }
        index
    }

    /// 登记 `(类型编码, 位号)` 已被占用。位号为空不登记。
    ///
    /// 先到先得：同一键已有占用者时保留原占用者。批量生成先以
    /// [`Handle::NULL`] 占位，落盘后由 [`TagSequencer::commit`] 换成真实
    /// handle，好让冲突提示指向正确的实例。
    fn occupy(&mut self, type_code: &str, tag: &str, handle: Handle) {
        if tag.is_empty() {
            return;
        }
        let slot = self.occupied.entry(type_code.to_string()).or_default();
        match slot.get_mut(tag) {
            Some(existing) if existing.is_null() => *existing = handle,
            Some(_) => {}
            None => {
                slot.insert(tag.to_string(), handle);
            }
        }
    }

    /// 按规则反解计数段，推进本类型已用最大计数。
    ///
    /// 反解不出计数段时不动：该位号属于另一套模板，不能当本模板序号用。
    fn advance(
        &mut self,
        type_code: &str,
        tag: &str,
        rule: &TagRule,
        props: &BTreeMap<String, String>,
    ) {
        let Some(counter) = parse_counter(rule, tag, props) else {
            return;
        };
        let slot = self.max_used.entry(type_code.to_string()).or_insert(i64::MIN);
        if counter > *slot {
            *slot = counter;
        }
    }

    /// 本类型已用最大计数。
    fn max_for(&self, type_code: &str) -> Option<i64> {
        self.max_used.get(type_code).copied()
    }

    /// `(类型编码, 位号)` 是否已被占用，返回占用者。
    fn occupant(&self, type_code: &str, tag: &str) -> Option<Handle> {
        self.occupied.get(type_code)?.get(tag).copied()
    }
}

/// 解析一个类型编码生效的位号规则。
///
/// 两条口径必须同时成立：
///
/// 1. **类型在库里**：只认类型树（本类型规则 → 沿 `parent` 上溯）。整条链
///    都没有规则仍是 [`TagError::NoRule`] —— 这是 `empty_rule_reports_missing_rule`
///    锁定的既有语义，不能被默认表掩盖。
/// 2. **类型不在库里**：按 `architecture.md` A.9 的默认表兜底
///    （[`default_tag_rule`]）。`VALVE` / `INSTRUMENT` 走这条：它们由符号库
///    的 `symbol_type_map` 作为作者类型写进 `PID_AUTHOR`，但类型库因 D4 /
///    D16 不能收它们。默认表里也没有的类型仍是 [`TagError::UnknownType`]。
pub(crate) fn resolve_rule(lib: &TypeLib, type_code: &str) -> Result<TagRule, TagError> {
    if lib.contains(type_code) {
        return lib
            .resolve_tag_rule(type_code)
            .map(|(rule, _)| rule)
            .ok_or_else(|| TagError::NoRule {
                type_code: type_code.to_string(),
            });
    }
    default_tag_rule(type_code).ok_or_else(|| TagError::UnknownType {
        type_code: type_code.to_string(),
    })
}

/// 模板骨架：把 `join` 声明的属性占位符渲染掉，保留 `{序号}` / `{后缀}`。
///
/// 用它去匹配已有位号，拼接字段的取值就不会把前缀匹配弄错。
fn skeleton(rule: &TagRule, props: &BTreeMap<String, String>) -> Option<String> {
    let mut text = rule.pattern.clone();
    if let Some(field) = rule.join.as_deref() {
        let value = props.get(field)?;
        text = text.replace(&format!("{{{field}}}"), value);
    }
    Some(text)
}

/// 从位号里取出计数段的值。
///
/// 模板含 `{序号}` 时计数段是序号；只有 `{后缀}` 且后缀递增时计数段是后缀。
/// 两者都没有、或位号与骨架不匹配时返回 `None`（该位号不占号）。
fn parse_counter(rule: &TagRule, tag: &str, props: &BTreeMap<String, String>) -> Option<i64> {
    let skel = skeleton(rule, props)?;
    if let Some((head, tail)) = skel.split_once(TagRule::SEQUENCE_PLACEHOLDER) {
        let (mid, after) = match tail.split_once(TagRule::SUFFIX_PLACEHOLDER) {
            Some((mid, after)) => (mid, after),
            None => (tail, ""),
        };
        let rest = tag.strip_prefix(head)?;
        let end = rest.len().checked_sub(after.len())?;
        let middle = rest.get(..end)?;
        if !middle.ends_with(mid) {
            return None;
        }
        let digits = middle.get(..middle.len() - mid.len())?;
        return digits_of(digits);
    }
    if rule.suffix_increment {
        let (head, tail) = skel.split_once(TagRule::SUFFIX_PLACEHOLDER)?;
        let rest = tag.strip_prefix(head)?;
        let end = rest.len().checked_sub(tail.len())?;
        return digits_of(rest.get(..end)?);
    }
    None
}

/// 非空十进制整数段。
fn digits_of(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

// ── 模板渲染 ─────────────────────────────────────────────────────────────

/// 渲染候选位号。返回 `Err(TemplateFieldMissing)` 时**尚未**占用任何序号。
///
/// `counter` 同时驱动 `{序号}` 段与（后缀递增开启时的）`{后缀}` 段。
fn render(
    rule: &TagRule,
    counter: i64,
    props: &BTreeMap<String, String>,
) -> Result<String, TagError> {
    // 先校验拼接字段：缺失就立刻失败，此时序号还没被提交占用（VAL-SEM-005）。
    let join_value = match rule.join.as_deref() {
        Some(field) => match props.get(field) {
            Some(value) => Some(value.clone()),
            None => {
                return Err(TagError::TemplateFieldMissing {
                    field: field.to_string(),
                })
            }
        },
        None => None,
    };

    let mut out = rule.pattern.clone();
    if let (Some(field), Some(value)) = (rule.join.as_deref(), join_value) {
        out = out.replace(&format!("{{{field}}}"), &value);
    }
    out = out.replace(TagRule::SEQUENCE_PLACEHOLDER, &pad_sequence(counter, rule.pad));

    if rule.has_suffix() && rule.suffix_increment {
        out = out.replace(TagRule::SUFFIX_PLACEHOLDER, &pad_suffix(counter));
    } else {
        // 后缀求值为空时，连同紧邻的分隔符一起裁掉：`P-{序号}-{后缀}` → `P-1001`。
        for glued in ["-", "_", ".", "/"] {
            out = out.replace(&format!("{glued}{}", TagRule::SUFFIX_PLACEHOLDER), "");
        }
        out = out.replace(TagRule::SUFFIX_PLACEHOLDER, "");
    }
    Ok(out)
}

/// `{序号}` 段补零：`pad == 0` 原样，否则左补 `0` 到 `pad` 位。
fn pad_sequence(counter: i64, pad: u32) -> String {
    let width = pad as usize;
    if width == 0 {
        return counter.to_string();
    }
    format!("{:0width$}", counter, width = width)
}

/// `{后缀}` 段补零：**固定两位**，`-99` 之后是 `-100`，不截断不回绕。
fn pad_suffix(counter: i64) -> String {
    format!("{counter:02}")
}

// ── 层级：取哪个类型的序号空间 ───────────────────────────────────────────

/// 本次生成要从哪些类型的已用计数里取最大值。
///
/// `当前类型依次递增` 只看本类型；`父类型递增` 看整条祖先链（含自身，
/// `TypeLib::ancestors` 已是自底向上含自身）。
fn sequence_scopes(lib: &TypeLib, type_code: &str, rule: &TagRule) -> Vec<String> {
    match rule.hierarchy() {
        HierarchyMode::CurrentTypeIncrement => vec![type_code.to_string()],
        HierarchyMode::ParentTypeIncrement => {
            let chain = lib.ancestors(type_code);
            if chain.is_empty() {
                vec![type_code.to_string()]
            } else {
                chain
            }
        }
    }
}

/// 各作用域已用最大计数中的最大值。
fn used_max(index: &SequenceIndex, scopes: &[String]) -> Option<i64> {
    scopes.iter().filter_map(|code| index.max_for(code)).max()
}

/// `props` JSON 串 → 键值表。解析失败按空表处理（缺失即「没有属性」）。
fn parse_props(props_json: &str) -> BTreeMap<String, String> {
    if props_json.trim().is_empty() {
        return BTreeMap::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(props_json) else {
        return BTreeMap::new();
    };
    let Some(object) = value.as_object() else {
        return BTreeMap::new();
    };
    object
        .iter()
        .filter(|(key, _)| key.as_str() != "__overrides")
        .map(|(key, value)| {
            let text = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (key.clone(), text)
        })
        .collect()
}

// ── 公开入口 ─────────────────────────────────────────────────────────────

/// 为一个新实例求位号；不修改任何既有实例。
///
/// 失败语义：
/// - 拼接字段缺失 → `Err(TAG_TEMPLATE_FIELD_MISSING)`，**不消耗序号**；
/// - 唯一校验拒绝 → `Err(TAG_DUPLICATED)`，调用方把实例标记为 `duplicated`。
///
/// 自动编码关闭时不生成位号，返回 [`TagView::empty`]（空位号、`manual`、
/// `ok`、显示为空）。
pub fn generate(
    lib: &TypeLib,
    doc: &Document,
    type_code: &str,
    props: &BTreeMap<String, String>,
) -> Result<TagView, TagError> {
    let mut index = SequenceIndex::scan(lib, doc);
    resolve_with_index(lib, &mut index, type_code, props)
}

/// `generate` 与 [`TagSequencer::next`] 共用的核心：查索引定候选、渲染、
/// 判重、推进，成功时把新位号登记进索引。
///
/// 索引由调用方提供：`generate` 每次新建（单实例语义），`TagSequencer`
/// 复用同一份（批量线性化）。除此之外两者完全同一条路径，避免语义漂移。
fn resolve_with_index(
    lib: &TypeLib,
    index: &mut SequenceIndex,
    type_code: &str,
    props: &BTreeMap<String, String>,
) -> Result<TagView, TagError> {
    let rule = resolve_rule(lib, type_code)?;
    // 自动编码关闭：位号为空、来源 manual、状态 ok、显示为空（VAL-SEM-013）。
    if !rule.auto_code {
        return Ok(TagView::empty());
    }

    let scopes = sequence_scopes(lib, type_code, &rule);
    let step = rule.step;

    // 候选计数：已用最大 + 增量；从未占用过则用起始编码。
    let mut counter = match used_max(index, &scopes) {
        Some(max) => max.saturating_add(step),
        None => rule.start,
    };
    if counter < rule.start {
        counter = rule.start;
    }

    let mut attempts = 0u32;
    loop {
        // 先渲染并校验拼接字段；失败路径不推进序号（VAL-SEM-005）。
        let candidate = render(&rule, counter, props)?;
        // 再判唯一：同类型同位号拒绝（VAL-SEM-010），跨类型允许（VAL-SEM-011）。
        match index.occupant(type_code, &candidate) {
            None => {
                // 立即登记，下一次查表就能看见它（批量线性化的关键）。
                index.occupy(type_code, &candidate, Handle::NULL);
                index.advance(type_code, &candidate, &rule, props);
                return Ok(TagView::auto_ok(candidate));
            }
            Some(existing) if !rule.unique => {
                let _ = existing;
                index.occupy(type_code, &candidate, Handle::NULL);
                index.advance(type_code, &candidate, &rule, props);
                return Ok(TagView::auto_ok(candidate));
            }
            Some(existing) => {
                attempts += 1;
                if attempts > MAX_ATTEMPTS {
                    return Err(TagError::Duplicated {
                        tag: candidate,
                        type_code: type_code.to_string(),
                        existing,
                    });
                }
                counter = counter.saturating_add(step);
            }
        }
    }
}

/// 一次扫描、多次生成的序号索引（D22）。
///
/// [`generate`] 每次调用都重扫全文档：2000 个实例就是 O(n²) 次
/// `PidAuthor::load`。批量路径改用本类型：构造时扫一次，之后每次
/// [`next`](Self::next) 只做 O(1) 查表 + 推进，整体线性。
///
/// 语义与 [`generate`] 完全一致（同一条规则解析、同一套 `render`、同一套
/// 唯一校验、同一个重试上限、同样的 `Err` 变体），唯一区别是索引复用而非
/// 重建。
pub struct TagSequencer<'a> {
    lib: &'a TypeLib,
    index: SequenceIndex,
}

impl<'a> TagSequencer<'a> {
    /// 扫全文档一次建索引。
    pub fn new(lib: &'a TypeLib, doc: &Document) -> Self {
        Self {
            lib,
            index: SequenceIndex::scan(lib, doc),
        }
    }

    /// 为一个**尚未落盘**的新实例求位号。
    ///
    /// 成功时立即把该位号计入索引，使下一次调用看到它 —— 这是线性化的
    /// 关键，且不依赖调用方先 `store`。失败（`Err`）与自动编码关闭
    /// （`TagView::empty`）都不登记索引。
    pub fn next(
        &mut self,
        type_code: &str,
        props: &BTreeMap<String, String>,
    ) -> Result<TagView, TagError> {
        resolve_with_index(self.lib, &mut self.index, type_code, props)
    }

    /// 把已落盘位号显式登记进索引，并把占位 handle 换成真实 handle。
    ///
    /// [`next`](Self::next) 已自动登记，正常批量路径不必再调；调用方落盘后
    /// 调一次能让冲突提示指向真实实例。登记口径与
    /// [`SequenceIndex::scan`] 一致：手工位号与冲突残值只占唯一键、不推进
    /// 序号；推进取最大值，重复调用幂等。
    pub fn commit(&mut self, type_code: &str, view: &TagView, handle: Handle) {
        self.index.occupy(type_code, &view.tag, handle);
        if view.tag_source == TAG_SOURCE_MANUAL
            || view.tag_status == TAG_STATUS_DUPLICATED
            || view.tag.is_empty()
        {
            return;
        }
        let Ok(rule) = resolve_rule(self.lib, type_code) else {
            return;
        };
        // 此处拿不到实例属性，按空表反解：模板无拼接字段时照常推进，有拼接
        // 字段时反解不出计数段即为 no-op（该序号已由 `next` 推进过）。
        self.index.advance(type_code, &view.tag, &rule, &BTreeMap::new());
    }
}

/// 重新求值一个既有实例。
///
/// **第一件事**：`tag_source == manual` 直接原样返回，不查规则、不算序号
/// （`VAL-SEM-012` 的判定核心）。
pub fn reevaluate(lib: &TypeLib, doc: &Document, handle: Handle) -> Result<TagView, TagError> {
    let author = PidAuthor::load(doc, handle).ok_or(TagError::NotPidInstance { handle })?;

    // 手工位号永不被覆盖：原样返回，不解析规则、不扫序号。
    if author.tag_source == TAG_SOURCE_MANUAL {
        let status = if author.tag_status.is_empty() {
            TAG_STATUS_OK
        } else {
            author.tag_status.as_str()
        };
        return Ok(TagView::new(
            author.tag.clone(),
            TAG_SOURCE_MANUAL,
            status,
        ));
    }

    let type_code = author.type_code.as_str();
    let rule = resolve_rule(lib, type_code)?;
    // 规则启停：自动编码关闭后，已生成位号的实例回到空位号视图。
    if !rule.auto_code {
        return Ok(TagView::empty());
    }

    let props = parse_props(&author.props);
    let view = generate(lib, doc, type_code, &props)?;

    // 已生成位号与规则变更后的候选不一致时保留既有存储值 ——
    // 规则变更不触发重算（VAL-SEM-003），只刷新状态。
    if !author.tag.is_empty() && author.tag != view.tag {
        return Ok(TagView::new(
            author.tag.clone(),
            TAG_SOURCE_AUTO,
            TAG_STATUS_OK,
        ));
    }
    Ok(view)
}

/// 唯一校验：`(类型编码, 位号)` 复合键是否已被别的实例占用。
///
/// `excluding` 是被校验实例自身（它当然占着自己的位号）。
pub fn check_unique(
    lib: &TypeLib,
    doc: &Document,
    type_code: &str,
    tag: &str,
    excluding: Option<Handle>,
) -> Result<(), TagError> {
    let rule = resolve_rule(lib, type_code)?;
    if !rule.unique || tag.is_empty() {
        return Ok(());
    }
    let index = SequenceIndex::scan(lib, doc);
    match index.occupant(type_code, tag) {
        Some(existing) if Some(existing) != excluding => Err(TagError::Duplicated {
            tag: tag.to_string(),
            type_code: type_code.to_string(),
            existing,
        }),
        _ => Ok(()),
    }
}

/// 把一个实例的位号视图写回 `PID_AUTHOR` 记录（只改 `tag` / `tag_source` /
/// `tag_status` 三键，其余六键原样保留）。
pub fn store_view(doc: &mut Document, handle: Handle, view: &TagView) -> Result<(), TagError> {
    let mut author = PidAuthor::load(doc, handle).ok_or(TagError::NotPidInstance { handle })?;
    author.tag = view.tag.clone();
    author.tag_source = view.tag_source.clone();
    author.tag_status = view.tag_status.clone();
    author.store(doc, handle);
    Ok(())
}

/// 按 `(类型编码, 位号)` 复合键重算全文档的 `tag_status`，返回状态有变化的实例。
///
/// 先到先得：文档顺序在前的实例保留位号，后来者标 `duplicated`。`edited`
/// 里的实例排在最后参与比较 —— 刚被改成冲突值的那个才是冲突方，而不是
/// 原本就持有该位号的实例。冲突解除后状态回到 `ok`；`tag` 本身从不改写。
/// 规则未开启唯一校验、或类型无规则的实例一律视为 `ok`。
pub fn refresh_statuses(lib: &TypeLib, doc: &mut Document, edited: &[Handle]) -> Vec<Handle> {
    let mut records: Vec<(Handle, PidAuthor)> = doc
        .entities()
        .filter_map(|entity| {
            let handle = entity.common().handle;
            PidAuthor::load(doc, handle).map(|author| (handle, author))
        })
        .filter(|(_, author)| !author.type_code.is_empty())
        .collect();
    records.sort_by_key(|(handle, _)| edited.contains(handle));

    let mut unique_of: HashMap<String, bool> = HashMap::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut changed = Vec::new();
    for (handle, author) in records {
        let unique = *unique_of
            .entry(author.type_code.clone())
            .or_insert_with(|| resolve_rule(lib, &author.type_code).is_ok_and(|rule| rule.unique));
        let duplicated = unique
            && !author.tag.is_empty()
            && !seen.insert((author.type_code.clone(), author.tag.clone()));
        let status = if duplicated {
            TAG_STATUS_DUPLICATED
        } else {
            TAG_STATUS_OK
        };
        let current = if author.tag_status.is_empty() {
            TAG_STATUS_OK
        } else {
            author.tag_status.as_str()
        };
        if current == status {
            continue;
        }
        let view = TagView::new(author.tag.clone(), author.tag_source.clone(), status);
        if store_view(doc, handle, &view).is_ok() {
            changed.push(handle);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::type_lib::{TypeNode, ValueType};
    use document::entities::Line;
    use document::types::Vector3;

    // ── 夹具 ─────────────────────────────────────────────────────────────

    fn line(y: f64) -> document::EntityType {
        document::EntityType::Line(Line::from_points(
            Vector3::new(0.0, y, 0.0),
            Vector3::new(10.0, y, 0.0),
        ))
    }

    fn unique(mut rule: TagRule) -> TagRule {
        rule.unique = true;
        rule
    }

    fn rule(pattern: &str, start: i64) -> TagRule {
        TagRule::new(pattern, start)
    }

    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// `EQUIPMENT` 带给定规则的库；VESSEL / TANK / PUMP 不覆盖（继承）。
    fn lib_with(rule: TagRule) -> TypeLib {
        TypeLib::new(vec![
            TypeNode::new("EQUIPMENT", None::<String>)
                .with_label("设备")
                .with_tag_rule(rule),
            TypeNode::new("VESSEL", Some("EQUIPMENT")).with_label("容器"),
            TypeNode::new("TANK", Some("VESSEL")).with_label("储罐"),
            TypeNode::new("PUMP", Some("EQUIPMENT")).with_label("泵"),
        ])
        .expect("fixture type lib must build")
    }

    /// `EQUIPMENT` 与 `VESSEL` 各自定义同一个规则（层级测试用）。
    fn lib_two_types(rule: TagRule) -> TypeLib {
        TypeLib::new(vec![
            TypeNode::new("EQUIPMENT", None::<String>)
                .with_label("设备")
                .with_tag_rule(rule.clone()),
            TypeNode::new("VESSEL", Some("EQUIPMENT"))
                .with_label("容器")
                .with_tag_rule(rule),
        ])
        .expect("fixture type lib must build")
    }

    /// `EQUIPMENT` 与 `INSTRUMENT` 各定义同一个规则（跨类型同位号测试用）。
    fn lib_two_codes(rule: TagRule) -> TypeLib {
        TypeLib::new(vec![
            TypeNode::new("EQUIPMENT", None::<String>)
                .with_label("设备")
                .with_tag_rule(rule.clone()),
            TypeNode::new("INSTRUMENT", Some("EQUIPMENT"))
                .with_label("仪表")
                .with_tag_rule(rule),
        ])
        .expect("fixture type lib must build")
    }

    /// 直接写一条带位号的实例记录，返回 handle。
    fn place(
        doc: &mut Document,
        type_code: &str,
        tag: &str,
        tag_source: &str,
        tag_status: &str,
    ) -> Handle {
        let handle = doc.add_entity(line(0.0)).expect("entity added");
        PidAuthor {
            type_code: type_code.to_string(),
            tag: tag.to_string(),
            tag_source: tag_source.to_string(),
            tag_status: tag_status.to_string(),
            ..PidAuthor::default()
        }
        .store(doc, handle);
        handle
    }

    /// 模拟「插入一个新实例」：先求位号，成功则落盘（含 props）。
    fn insert(
        lib: &TypeLib,
        doc: &mut Document,
        type_code: &str,
        props: &BTreeMap<String, String>,
    ) -> (Handle, Result<TagView, TagError>) {
        let handle = doc.add_entity(line(0.0)).expect("entity added");
        let view = generate(lib, doc, type_code, props);
        if let Ok(view) = &view {
            PidAuthor {
                type_code: type_code.to_string(),
                tag: view.tag.clone(),
                tag_source: view.tag_source.clone(),
                tag_status: view.tag_status.clone(),
                props: serde_json::to_string(props).unwrap_or_default(),
                ..PidAuthor::default()
            }
            .store(doc, handle);
        }
        (handle, view)
    }

    /// 连续生成 n 个，返回位号序列。
    fn sequence(lib: &TypeLib, doc: &mut Document, type_code: &str, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                let (_, view) = insert(lib, doc, type_code, &BTreeMap::new());
                view.expect("generation succeeds").tag
            })
            .collect()
    }

    // ── VAL-SEM-001 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_increment_one_sequence() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        let mut got = Vec::new();
        for _ in 0..3 {
            let (_, view) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
            let view = view.expect("generation succeeds");
            got.push((view.tag, view.tag_source, view.tag_status));
        }
        let expected = vec![
            ("P-1001".to_string(), "auto".to_string(), "ok".to_string()),
            ("P-1002".to_string(), "auto".to_string(), "ok".to_string()),
            ("P-1003".to_string(), "auto".to_string(), "ok".to_string()),
        ];
        assert_eq!(got, expected);
    }

    // ── VAL-SEM-002 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_increment_five_sequence() {
        let mut r = unique(rule("P-{序号}", 1001));
        r.step = 5;
        let lib = lib_with(r);
        let mut doc = Document::new();
        let got = sequence(&lib, &mut doc, "EQUIPMENT", 3);
        assert_eq!(got, vec!["P-1001", "P-1006", "P-1011"]);
    }

    // ── VAL-SEM-003 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_start_and_increment_are_independent() {
        let mut r = unique(rule("P-{序号}", 2001));
        r.step = 2;
        let lib = lib_with(r);
        let mut doc = Document::new();

        let handles: Vec<Handle> = (0..3)
            .map(|_| insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new()).0)
            .collect();
        let tags = |doc: &Document| -> Vec<String> {
            handles
                .iter()
                .map(|h| PidAuthor::load(doc, *h).unwrap().tag)
                .collect()
        };
        assert_eq!(tags(&doc), vec!["P-2001", "P-2003", "P-2005"]);

        // 改起始编码为 3001，但不触发重算：三个既有位号不变。
        let mut changed = unique(rule("P-{序号}", 3001));
        changed.step = 2;
        let lib2 = lib_with(changed);
        for handle in &handles {
            let view = reevaluate(&lib2, &doc, *handle).expect("reevaluate succeeds");
            assert_eq!(view.tag, PidAuthor::load(&doc, *handle).unwrap().tag);
        }
        assert_eq!(tags(&doc), vec!["P-2001", "P-2003", "P-2005"]);
    }

    // ── VAL-SEM-004 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_concat_field_uses_instance_property() {
        let mut r = unique(rule("P-{序号}-{管道等级}", 1001));
        r.join = Some("管道等级".to_string());
        let lib = lib_with(r);
        let mut doc = Document::new();

        let (_, a) = insert(&lib, &mut doc, "EQUIPMENT", &props(&[("管道等级", "A1A")]));
        let (_, b) = insert(&lib, &mut doc, "EQUIPMENT", &props(&[("管道等级", "A1B")]));
        let a = a.expect("first generation succeeds");
        let b = b.expect("second generation succeeds");
        assert_eq!(
            vec![(a.tag, "A1A".to_string()), (b.tag, "A1B".to_string())],
            vec![
                ("P-1001-A1A".to_string(), "A1A".to_string()),
                ("P-1002-A1B".to_string(), "A1B".to_string())
            ]
        );
    }

    // ── VAL-SEM-005 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_missing_concat_field_rejects_without_consuming_sequence() {
        let mut r = unique(rule("P-{序号}-{管道等级}", 1001));
        r.join = Some("管道等级".to_string());
        let lib = lib_with(r);
        let mut doc = Document::new();

        let err = generate(&lib, &doc, "EQUIPMENT", &BTreeMap::new())
            .expect_err("missing join field must fail");
        assert_eq!(err.code(), "TAG_TEMPLATE_FIELD_MISSING", "got {err:?}");
        assert_eq!(
            err,
            TagError::TemplateFieldMissing {
                field: "管道等级".to_string()
            }
        );

        // 失败后首个成功实例仍是 P-1001-A1A，而不是 P-1002-A1A。
        let (_, ok) = insert(&lib, &mut doc, "EQUIPMENT", &props(&[("管道等级", "A1A")]));
        let ok = ok.expect("generation succeeds after a failed attempt");
        assert_eq!(ok.tag, "P-1001-A1A");
        assert_eq!(ok.tag_source, "auto");
        assert_eq!(ok.tag_status, "ok");
    }

    // ── VAL-SEM-006 ──────────────────────────────────────────────────────

    /// 管路库：`PIPE` 挂 `EQUIPMENT` 下，宿主位号由 `管线号` 属性提供。
    fn lib_pipe(suffix_increment: bool) -> TypeLib {
        let mut r = unique(rule("P-{管线号}-{后缀}", 1));
        r.suffix_increment = suffix_increment;
        r.join = Some("管线号".to_string());
        TypeLib::new(vec![
            TypeNode::new("EQUIPMENT", None::<String>)
                .with_label("设备")
                .with_tag_rule(unique(rule("P-{序号}", 1001))),
            TypeNode::new("PIPE", Some("EQUIPMENT"))
                .with_label("管路")
                .with_tag_rule(r),
        ])
        .expect("fixture type lib must build")
    }

    #[test]
    fn tag_rule_suffix_increment_pipe_run_sequence() {
        let lib = lib_pipe(true);
        let mut doc = Document::new();
        let host = props(&[("管线号", "1001")]);

        let mut tags = Vec::new();
        for _ in 0..3 {
            let (_, view) = insert(&lib, &mut doc, "PIPE", &host);
            tags.push(view.expect("pipe run generation succeeds").tag);
        }
        assert_eq!(tags, vec!["P-1001-01", "P-1001-02", "P-1001-03"]);

        // 一路生成到第 100 条：P-1001-100，超两位不截断不回绕。
        let mut last = tags[2].clone();
        for _ in 4..=100 {
            let (_, view) = insert(&lib, &mut doc, "PIPE", &host);
            last = view.expect("pipe run generation succeeds").tag;
        }
        assert_eq!(last, "P-1001-100");
    }

    // ── VAL-SEM-007 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_suffix_disabled_marks_duplicate() {
        let lib = lib_pipe(false);
        let mut doc = Document::new();
        let host = props(&[("管线号", "1001")]);

        let (_, first) = insert(&lib, &mut doc, "PIPE", &host);
        let first = first.expect("first pipe run generation succeeds");
        assert_eq!(first.tag, "P-1001");
        assert_eq!(first.display, "P-1001");

        // 第二条与第一条位号相同：唯一校验拒绝，调用方标记 duplicated。
        let err = generate(&lib, &doc, "PIPE", &host).expect_err("duplicate must be rejected");
        assert_eq!(err.code(), "TAG_DUPLICATED", "got {err:?}");

        let view = TagView::duplicated("P-1001", TAG_SOURCE_AUTO);
        assert_eq!(
            (view.tag.as_str(), view.tag_status.as_str(), view.display.as_str()),
            ("P-1001", "duplicated", "？")
        );
    }

    // ── VAL-SEM-008 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_hierarchy_self_increment() {
        let mut r = unique(rule("P-{序号}", 1001));
        r.level = Some(HierarchyMode::CurrentTypeIncrement);
        let lib = lib_two_types(r);
        let mut doc = Document::new();

        let (_, equipment) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        assert_eq!(equipment.unwrap().tag, "P-1001");

        let (_, v1) = insert(&lib, &mut doc, "VESSEL", &BTreeMap::new());
        let (_, v2) = insert(&lib, &mut doc, "VESSEL", &BTreeMap::new());
        assert_eq!(v1.unwrap().tag, "P-1001");
        assert_eq!(v2.unwrap().tag, "P-1002");
    }

    // ── VAL-SEM-009 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_hierarchy_parent_increment() {
        let mut r = unique(rule("P-{序号}", 1001));
        r.level = Some(HierarchyMode::ParentTypeIncrement);
        let lib = lib_two_types(r);
        let mut doc = Document::new();

        let mut equipment = Vec::new();
        for _ in 0..3 {
            let (_, view) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
            equipment.push(view.unwrap().tag);
        }
        assert_eq!(equipment, vec!["P-1001", "P-1002", "P-1003"]);

        let mut vessel = Vec::new();
        for _ in 0..2 {
            let (_, view) = insert(&lib, &mut doc, "VESSEL", &BTreeMap::new());
            vessel.push(view.unwrap().tag);
        }
        assert_eq!(vessel, vec!["P-1004", "P-1005"]);
    }

    // ── VAL-SEM-010 ──────────────────────────────────────────────────────

    #[test]
    fn tag_unique_rejects_same_type_same_tag() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();

        let existing = place(&mut doc, "EQUIPMENT", "P-1001", "auto", "ok");
        let other = place(&mut doc, "EQUIPMENT", "", "manual", "ok");

        let err = check_unique(&lib, &doc, "EQUIPMENT", "P-1001", Some(other))
            .expect_err("same type same tag must be rejected");
        assert_eq!(err.code(), "TAG_DUPLICATED");
        match err {
            TagError::Duplicated {
                tag,
                type_code,
                existing: found,
            } => {
                assert_eq!(tag, "P-1001");
                assert_eq!(type_code, "EQUIPMENT");
                assert_eq!(found, existing);
            }
            other => panic!("unexpected error {other:?}"),
        }

        // 冲突时 tag 保留原值不清空，状态 duplicated，显示 ？。
        let view = TagView::duplicated("P-1001", TAG_SOURCE_MANUAL);
        store_view(&mut doc, other, &view).expect("view stored");
        let author = PidAuthor::load(&doc, other).unwrap();
        assert_eq!(author.tag, "P-1001");
        assert_eq!(author.tag_status, "duplicated");
        assert_eq!(view.display, "？");
    }

    // ── VAL-SEM-011 ──────────────────────────────────────────────────────

    #[test]
    fn tag_unique_allows_cross_type_same_tag() {
        let lib = lib_two_codes(unique(rule("X-{序号}", 1001)));
        let mut doc = Document::new();

        let (eq_handle, equipment) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        let (in_handle, instrument) = insert(&lib, &mut doc, "INSTRUMENT", &BTreeMap::new());
        let equipment = equipment.expect("equipment generation succeeds");
        let instrument = instrument.expect("instrument generation succeeds");

        assert_eq!(
            vec![
                ("EQUIPMENT".to_string(), equipment.tag, equipment.tag_status),
                (
                    "INSTRUMENT".to_string(),
                    instrument.tag,
                    instrument.tag_status
                ),
            ],
            vec![
                (
                    "EQUIPMENT".to_string(),
                    "X-1001".to_string(),
                    "ok".to_string()
                ),
                (
                    "INSTRUMENT".to_string(),
                    "X-1001".to_string(),
                    "ok".to_string()
                ),
            ]
        );
        assert_eq!(equipment.display, "X-1001");
        assert_eq!(instrument.display, "X-1001");

        // 两次校验都 Ok：唯一键是 (类型编码, 位号) 复合键。
        assert!(check_unique(&lib, &doc, "EQUIPMENT", "X-1001", Some(eq_handle)).is_ok());
        assert!(check_unique(&lib, &doc, "INSTRUMENT", "X-1001", Some(in_handle)).is_ok());
    }

    // ── VAL-SEM-012 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_never_overwrites_manual_tag() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();

        let manual = place(&mut doc, "EQUIPMENT", "P-9001", "manual", "ok");
        let manual_state = |doc: &Document| {
            let author = PidAuthor::load(doc, manual).unwrap();
            (author.tag, author.tag_source)
        };

        // 触发一：插入新实例。
        let (_, fresh) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        let fresh = fresh.expect("generation succeeds");
        assert_eq!(manual_state(&doc), ("P-9001".into(), "manual".into()));
        // 手工 P-9001 不占号：新实例仍是 P-1001。
        assert_eq!(fresh.tag, "P-1001");

        // 触发二：类型规则变更（起始编码 5001）。
        let changed = lib_with(unique(rule("P-{序号}", 5001)));
        let view = reevaluate(&changed, &doc, manual).expect("reevaluate succeeds");
        assert_eq!(
            (view.tag.as_str(), view.tag_source.as_str()),
            ("P-9001", "manual")
        );
        assert_eq!(manual_state(&doc), ("P-9001".into(), "manual".into()));

        // 触发三：规则启停（自动编码关闭）。
        let mut off = unique(rule("P-{序号}", 1001));
        off.auto_code = false;
        let disabled = lib_with(off);
        let view = reevaluate(&disabled, &doc, manual).expect("reevaluate succeeds");
        assert_eq!(
            (view.tag.as_str(), view.tag_source.as_str()),
            ("P-9001", "manual")
        );
        assert_eq!(manual_state(&doc), ("P-9001".into(), "manual".into()));
    }

    // ── VAL-SEM-013 ──────────────────────────────────────────────────────

    #[test]
    fn tag_rule_auto_code_disabled_leaves_tag_empty() {
        let mut r = unique(rule("P-{序号}", 1001));
        r.auto_code = false;
        let lib = lib_with(r);
        let mut doc = Document::new();

        let mut got = Vec::new();
        let mut handles = Vec::new();
        for _ in 0..2 {
            let (handle, view) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
            let view = view.expect("generation succeeds");
            got.push((view.tag, view.tag_source, view.tag_status, view.display));
            handles.push(handle);
        }
        let empty = (
            String::new(),
            "manual".to_string(),
            "ok".to_string(),
            String::new(),
        );
        assert_eq!(got, vec![empty.clone(), empty]);

        // 手工把第 1 个设为 P-1001，第 3 个仍为空，第 1 个 tag_source 保持 manual。
        let first = handles[0];
        store_view(
            &mut doc,
            first,
            &TagView::new("P-1001", TAG_SOURCE_MANUAL, TAG_STATUS_OK),
        )
        .expect("view stored");

        let (_, third) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        let third = third.expect("generation succeeds");
        assert_eq!(third.tag, "");
        assert_eq!(third.display, "");

        let first_author = PidAuthor::load(&doc, first).unwrap();
        assert_eq!(first_author.tag_source, "manual");
        assert_eq!(first_author.tag, "P-1001");
    }

    // ── 辅助单元测试 ─────────────────────────────────────────────────────

    #[test]
    fn sequence_index_excludes_manual_and_duplicated() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        place(&mut doc, "EQUIPMENT", "P-1005", "auto", "ok");
        place(&mut doc, "EQUIPMENT", "P-9001", "manual", "ok");
        place(&mut doc, "EQUIPMENT", "P-8001", "auto", "duplicated");

        let index = SequenceIndex::scan(&lib, &doc);
        assert_eq!(index.max_for("EQUIPMENT"), Some(1005));

        let (_, view) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        assert_eq!(view.unwrap().tag, "P-1006");
    }

    #[test]
    fn suffix_padding_keeps_two_digits_then_grows() {
        assert_eq!(pad_suffix(1), "01");
        assert_eq!(pad_suffix(9), "09");
        assert_eq!(pad_suffix(10), "10");
        assert_eq!(pad_suffix(99), "99");
        assert_eq!(pad_suffix(100), "100");
        assert_eq!(pad_suffix(1000), "1000");
    }

    #[test]
    fn sequence_padding_follows_pad_field_only() {
        let mut r = unique(rule("P-{序号}", 1001));
        r.pad = 6;
        let lib = lib_with(r);
        let mut doc = Document::new();
        let (_, view) = insert(&lib, &mut doc, "EQUIPMENT", &BTreeMap::new());
        assert_eq!(view.unwrap().tag, "P-001001");
    }

    #[test]
    fn empty_rule_reports_missing_rule() {
        let lib = TypeLib::new(vec![TypeNode::new("EQUIPMENT", None::<String>)]).unwrap();
        let doc = Document::new();
        let err = generate(&lib, &doc, "EQUIPMENT", &BTreeMap::new())
            .expect_err("no rule must fail");
        assert_eq!(err.code(), "TAG_RULE_MISSING");
        let err = generate(&lib, &doc, "NOPE", &BTreeMap::new()).expect_err("unknown type");
        assert_eq!(err.code(), "TAG_TYPE_UNKNOWN");
    }

    #[test]
    fn value_type_is_reachable_from_engine_tests() {
        // 保证类型库模块的导出可用（避免 unused import 警告）。
        assert_eq!(ValueType::Text.name(), "文本");
    }

    // ── parse_props 对覆盖标记的容忍 ─────────────────────────────────────────

    /// 覆盖标记 `__overrides` 是字符串数组，不得让整条 `props` 解析失败。
    #[test]
    fn parse_props_excludes_overrides_marker() {
        let parsed = parse_props(r#"{"公称直径":"DN50","管道等级":"B2C","__overrides":["管道等级"]}"#);
        assert_eq!(
            parsed,
            props(&[("公称直径", "DN50"), ("管道等级", "B2C")]),
            "覆盖标记应当被排除，实际 {parsed:?}"
        );
        assert!(
            !parsed.contains_key("__overrides"),
            "解析结果不得含 __overrides 键"
        );
    }

    /// 非字符串标量按 `to_string()` 归一，不得让整条 `props` 解析失败。
    #[test]
    fn parse_props_normalises_non_string_scalars() {
        let parsed = parse_props(r#"{"a":"1","b":2,"c":true}"#);
        assert_eq!(parsed.get("a").map(String::as_str), Some("1"));
        assert_eq!(parsed.get("b").map(String::as_str), Some("2"), "整数 2 应当归一成 \"2\"");
        assert_eq!(parsed.get("c").map(String::as_str), Some("true"));
    }

    // ── A.9 默认规则表兜底 ───────────────────────────────────────────────

    /// 类型库只认 `EQUIPMENT`（M3 的 `assets/pid-types.json` 就是这样），
    /// 而阀门符号落点的作者类型是 `VALVE`：库外类型按 A.9 默认表兜底，
    /// 首个阀门得到 `V-1001`，第二个 `V-1002`。
    #[test]
    fn a_valve_outside_the_type_lib_takes_the_default_rule() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        let got = sequence(&lib, &mut doc, "VALVE", 2);
        assert_eq!(got, vec!["V-1001", "V-1002"]);
    }

    /// 阀门与设备的序号空间互不相干：设备用到 `P-1003` 后，阀门仍从 `V-1001` 起。
    #[test]
    fn valve_and_equipment_sequences_are_independent() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        assert_eq!(
            sequence(&lib, &mut doc, "EQUIPMENT", 3),
            vec!["P-1001", "P-1002", "P-1003"]
        );
        assert_eq!(sequence(&lib, &mut doc, "VALVE", 1), vec!["V-1001"]);
        assert_eq!(sequence(&lib, &mut doc, "EQUIPMENT", 1), vec!["P-1004"]);
    }

    /// 仪表默认模板取 `仪表功能` 属性做前缀（`TI-1001`）；属性缺失时按
    /// `TemplateFieldMissing` 失败，不消耗序号。
    #[test]
    fn an_instrument_outside_the_type_lib_joins_its_function_prefix() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        let (_, view) = insert(&lib, &mut doc, "INSTRUMENT", &props(&[("仪表功能", "TI")]));
        assert_eq!(view.expect("仪表位号生成成功").tag, "TI-1001");

        let err = generate(&lib, &doc, "INSTRUMENT", &BTreeMap::new())
            .expect_err("缺 仪表功能 必须失败");
        assert_eq!(err.code(), "TAG_TEMPLATE_FIELD_MISSING");
    }

    /// 默认表兜底不掩盖库内语义：库里有的类型仍走类型树，整链无规则报
    /// `TAG_RULE_MISSING`；默认表里也没有的编码仍是 `TAG_TYPE_UNKNOWN`。
    #[test]
    fn the_default_table_does_not_shadow_type_lib_semantics() {
        let lib = TypeLib::new(vec![TypeNode::new("EQUIPMENT", None::<String>)]).unwrap();
        let doc = Document::new();
        assert_eq!(
            generate(&lib, &doc, "EQUIPMENT", &BTreeMap::new())
                .expect_err("库里无规则")
                .code(),
            "TAG_RULE_MISSING"
        );
        assert_eq!(
            generate(&lib, &doc, "NOPE", &BTreeMap::new())
                .expect_err("默认表也没有")
                .code(),
            "TAG_TYPE_UNKNOWN"
        );
    }

    /// 手工把第 2 个实例改成第 1 个的位号：被改的那个标 `duplicated`（显示
    /// `？`、位号原值保留），原持有者仍 `ok`；改回不冲突的值后两者都回到 `ok`。
    #[test]
    fn refresh_marks_the_edited_instance_duplicated_and_clears_it_again() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        let first = place(&mut doc, "EQUIPMENT", "P-1001", TAG_SOURCE_AUTO, TAG_STATUS_OK);
        let second = place(&mut doc, "EQUIPMENT", "P-1001", TAG_SOURCE_MANUAL, TAG_STATUS_OK);

        let changed = refresh_statuses(&lib, &mut doc, &[second]);
        assert_eq!(changed, vec![second]);
        let edited = PidAuthor::load(&doc, second).unwrap();
        assert_eq!(edited.tag, "P-1001", "冲突时位号原值保留");
        assert_eq!(edited.tag_status, TAG_STATUS_DUPLICATED);
        assert_eq!(
            TagView::new(edited.tag, edited.tag_source, edited.tag_status).display,
            DUPLICATED_DISPLAY
        );
        assert_eq!(PidAuthor::load(&doc, first).unwrap().tag_status, TAG_STATUS_OK);

        let mut fixed = PidAuthor::load(&doc, second).unwrap();
        fixed.tag = "P-9001".into();
        fixed.store(&mut doc, second);
        assert_eq!(refresh_statuses(&lib, &mut doc, &[second]), vec![second]);
        assert_eq!(PidAuthor::load(&doc, second).unwrap().tag_status, TAG_STATUS_OK);
        assert!(refresh_statuses(&lib, &mut doc, &[]).is_empty(), "无变化时不改写");
    }

    /// 不同类型同位号不冲突（复合键）。
    #[test]
    fn refresh_keys_on_type_and_tag() {
        let lib = lib_with(unique(rule("P-{序号}", 1001)));
        let mut doc = Document::new();
        place(&mut doc, "EQUIPMENT", "X-1", TAG_SOURCE_MANUAL, TAG_STATUS_OK);
        place(&mut doc, "VALVE", "X-1", TAG_SOURCE_MANUAL, TAG_STATUS_OK);
        assert!(refresh_statuses(&lib, &mut doc, &[]).is_empty());
    }

    /// 默认规则表覆盖 A.9 的五类，且都是唯一校验开启。
    #[test]
    fn default_rules_cover_the_documented_categories() {
        for code in ["EQUIPMENT", "VALVE", "INSTRUMENT", "PIPE_LINE", "PIPE_RUN"] {
            let rule = default_tag_rule(code).unwrap_or_else(|| panic!("{code} 应有默认规则"));
            assert!(rule.unique, "{code} 的默认规则必须开启唯一校验");
            assert!(rule.auto_code, "{code} 的默认规则必须开启自动编码");
        }
        assert_eq!(default_tag_rule("VALVE").unwrap().pattern, "V-{序号}");
        assert_eq!(default_tag_rule("PIPE_LINE").unwrap().pattern, "L-{序号}");
        assert_eq!(
            default_tag_rule("INSTRUMENT").unwrap().join.as_deref(),
            Some("仪表功能")
        );
        assert!(default_tag_rule("PIPE_RUN").unwrap().suffix_increment);
        assert!(default_tag_rule("NOT_A_CATEGORY").is_none());
        // 大小写不敏感：作者类型一律大写，但手写记录不该因此失效。
        assert!(default_tag_rule("valve").is_some());
    }
}

//! 材料清单（SPEC-pid-bom）：由范围内各图的摘要算出一份清单。
//!
//! [`build`] 只吃摘要，不碰图纸和文件（§4）：设备、阀门、仪表和有自己编号的其他对象
//! 每个（类型编码，位号）一行，不单独编号的管件按所在管路合成一行（§3.1）。范围内同一个
//! 键有两处以上时整份清单不产出，返回 [`BomError::DuplicateTag`]；产出的清单里键不重复，
//! 出口处断言。管线行、管长与副表「管道汇总」由 `BOM-2` 补，在那之前 `pipe_summary`、
//! `notices` 总是空的。

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::project_index::{
    compare_places, DrawingSummary, Duplicate, ObjectKind, Place, SummaryObject,
};
use super::symbols::SymbolLibrary;

/// 清单格式的版本（§3.2 `version`）。
pub const BOM_VERSION: u32 = 1;

/// 范围里有重复的（类型编码，位号）（§3.5）。
pub const BOM_DUPLICATE_TAG: &str = "BOM_DUPLICATE_TAG";

/// 一条管路上不单独编号的管件合成的那一行的类型编码。
pub const FITTING_ROW: &str = "FITTING";

/// 名称、规格取自这几个属性（§3.1）。
const PROP_NAME: &str = "名称";
const PROP_MODEL: &str = "型号";
const PROP_SPEC: &str = "规格";

/// 清单的范围（§3.2 `scope`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomScope {
    pub kind: ScopeKind,
    pub drawing_id: Option<String>,
    pub pbs: Option<String>,
}

impl BomScope {
    /// 一张图。
    #[must_use]
    pub fn drawing(drawing_id: &str) -> Self {
        Self {
            kind: ScopeKind::Drawing,
            drawing_id: Some(drawing_id.to_string()),
            pbs: None,
        }
    }

    /// 一个 PBS 节点。
    #[must_use]
    pub fn pbs(node: &str) -> Self {
        Self {
            kind: ScopeKind::Pbs,
            drawing_id: None,
            pbs: Some(node.to_string()),
        }
    }

    /// 整个项目。
    #[must_use]
    pub fn project() -> Self {
        Self {
            kind: ScopeKind::Project,
            drawing_id: None,
            pbs: None,
        }
    }
}

/// 范围的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Drawing,
    Pbs,
    Project,
}

/// 从哪里算（§3.2 `source`、`snapshot_seq`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BomSource {
    /// 当前图纸。
    Live,
    /// 这张图已存的一份快照，带快照序号。
    Snapshot(u64),
}

/// [`build`] 的选项。
#[derive(Debug, Clone, PartialEq)]
pub struct BomOptions {
    pub scope: BomScope,
    pub source: BomSource,
    /// 清单的 `generated_at`（Unix 秒）。由调用方给，`build` 不读时钟。
    pub generated_at: i64,
    /// 符号块名到显示名，管件行的名称用（[`symbol_labels`]）。
    pub symbol_labels: BTreeMap<String, String>,
}

/// §3.2 `source` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Live,
    Snapshot,
}

/// 一份清单（§3.2）。字段顺序就是 JSON 里的顺序；`ok` 由应答那一层加。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bom {
    pub version: u32,
    pub generated_at: i64,
    pub scope: BomScope,
    pub source: SourceKind,
    pub snapshot_seq: Option<u64>,
    pub length_unit_assumed: bool,
    pub rows: Vec<BomRow>,
    pub pipe_summary: Vec<PipeSummaryRow>,
    pub skipped: Vec<Skipped>,
    pub totals: Totals,
    pub notices: Vec<Notice>,
}

/// 主表的一行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BomRow {
    pub seq: usize,
    pub kind: ObjectKind,
    pub type_code: String,
    pub tag: String,
    pub name: String,
    pub spec: String,
    pub quantity: Quantity,
    pub unit: String,
    /// 这一行由几个实例算出：管线行是线段条数，其余是对象个数。
    pub instances: usize,
    pub drawings: Vec<String>,
}

/// 数量：管线行是米，保留 2 位小数；其余是个数，写成整数。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Quantity {
    Count(usize),
    Length(f64),
}

/// 副表「管道汇总」的一行（§3.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipeSummaryRow {
    pub pipe_class: String,
    pub nominal_diameter: String,
    pub insulation_class: String,
    pub length: f64,
    pub unit: String,
    pub lines: Vec<String>,
}

/// 没出行的一个对象（§3.2 `skipped`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub reason: SkipReason,
    pub kind: ObjectKind,
    pub type_code: String,
    pub drawing_id: String,
    pub at: [f64; 2],
    pub handle: String,
}

/// 没出行的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// 位号为空。
    EmptyTag,
}

/// §3.2 `totals`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub rows: usize,
    pub by_kind: ByKind,
}

/// 各种类几行，按行序的种类排，没有的不写。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByKind {
    #[serde(rename = "管线", default, skip_serializing_if = "is_zero")]
    pub pipe_line: usize,
    #[serde(rename = "设备", default, skip_serializing_if = "is_zero")]
    pub equipment: usize,
    #[serde(rename = "阀门", default, skip_serializing_if = "is_zero")]
    pub valve: usize,
    #[serde(rename = "仪表", default, skip_serializing_if = "is_zero")]
    pub instrument: usize,
    #[serde(rename = "管件", default, skip_serializing_if = "is_zero")]
    pub fitting: usize,
    #[serde(rename = "其他", default, skip_serializing_if = "is_zero")]
    pub other: usize,
}

/// 清单里的一条提示（§3.2 `notices`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub code: String,
    pub message: String,
}

/// 算不出清单的原因。
#[derive(Debug, Clone, PartialEq)]
pub enum BomError {
    /// 这些（类型编码，位号）在范围内各有两处以上（`BOM_DUPLICATE_TAG`）。
    DuplicateTag(Vec<Duplicate>),
}

impl BomError {
    /// §3.5 的错误码。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::DuplicateTag(_) => BOM_DUPLICATE_TAG,
        }
    }
}

impl fmt::Display for BomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTag(duplicates) => {
                let keys: Vec<String> = duplicates
                    .iter()
                    .map(|duplicate| format!("{} {}", duplicate.type_code, duplicate.tag))
                    .collect();
                write!(
                    f,
                    "{BOM_DUPLICATE_TAG}: 范围内这些（类型编码，位号）不止一处：{}；清单不产出",
                    keys.join("、")
                )
            }
        }
    }
}

impl std::error::Error for BomError {}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// 符号库里每个符号的块名和显示名，给 [`BomOptions::symbol_labels`]。
#[must_use]
pub fn symbol_labels(library: &SymbolLibrary) -> BTreeMap<String, String> {
    library
        .block_names()
        .into_iter()
        .filter_map(|block| {
            let label = library.symbol(&block)?.label.clone();
            Some((block, label))
        })
        .collect()
}

/// 算清单（§3.1）。同样的摘要得到同样的清单，与图的先后无关。
///
/// # Errors
///
/// 设备、阀门、仪表、其他这四类里同一个（类型编码，位号）有两处以上时返回
/// [`BomError::DuplicateTag`]；这四类的键撞上一条管路的管件行（`FITTING` 加管路位号）也算。
pub fn build(drawings: &[DrawingSummary], options: &BomOptions) -> Result<Bom, BomError> {
    let mut own = OwnGroups::new();
    let mut runs = RunGroups::new();
    let mut skipped = Vec::new();
    for drawing in drawings {
        for object in &drawing.objects {
            let item = Item { drawing, object };
            match object.kind {
                ObjectKind::Equipment
                | ObjectKind::Valve
                | ObjectKind::Instrument
                | ObjectKind::Other => {
                    if !object.tag.trim().is_empty() {
                        let key = (object.type_code.as_str(), object.tag.as_str());
                        own.entry(key).or_default().push(item);
                    } else if object.kind != ObjectKind::Other {
                        skipped.push(item);
                    }
                }
                ObjectKind::Fitting => match run_of(object) {
                    Some(run) => runs.entry(run).or_default().push(item),
                    None => skipped.push(item),
                },
                ObjectKind::Break
                | ObjectKind::PipeLine
                | ObjectKind::PipeRun
                | ObjectKind::SignalLine => {}
            }
        }
    }
    let duplicates = duplicates_of(&own, &runs);
    if !duplicates.is_empty() {
        return Err(BomError::DuplicateTag(duplicates));
    }
    let mut rows: Vec<BomRow> = own
        .values()
        .map(|items| own_row(&items[0]))
        .chain(
            runs.iter()
                .map(|(run, items)| fitting_row(run, items, &options.symbol_labels)),
        )
        .collect();
    rows.sort_by(|left, right| {
        rank(left.kind)
            .cmp(&rank(right.kind))
            .then_with(|| left.tag.cmp(&right.tag))
            .then_with(|| left.type_code.cmp(&right.type_code))
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.seq = index + 1;
    }
    assert_unique(&rows);
    let mut skipped: Vec<(Place, Skipped)> = skipped
        .iter()
        .map(|item| (item.place(), item.skipped()))
        .collect();
    skipped.sort_by(|(left, _), (right, _)| compare_places(left, right));
    let (source, snapshot_seq) = match options.source {
        BomSource::Live => (SourceKind::Live, None),
        BomSource::Snapshot(seq) => (SourceKind::Snapshot, Some(seq)),
    };
    Ok(Bom {
        version: BOM_VERSION,
        generated_at: options.generated_at,
        scope: options.scope.clone(),
        source,
        snapshot_seq,
        length_unit_assumed: false,
        totals: totals_of(&rows),
        rows,
        pipe_summary: Vec::new(),
        skipped: skipped.into_iter().map(|(_, skipped)| skipped).collect(),
        notices: Vec::new(),
    })
}

/// 摘要里的一个对象，连同它所在的图。
struct Item<'a> {
    drawing: &'a DrawingSummary,
    object: &'a SummaryObject,
}

impl Item<'_> {
    fn place(&self) -> Place {
        Place {
            drawing_id: self.drawing.drawing_id.clone(),
            at: self.object.at,
            handle: self.object.handle.clone(),
        }
    }

    fn skipped(&self) -> Skipped {
        Skipped {
            reason: SkipReason::EmptyTag,
            kind: self.object.kind,
            type_code: self.object.type_code.clone(),
            drawing_id: self.drawing.drawing_id.clone(),
            at: self.object.at,
            handle: self.object.handle.clone(),
        }
    }
}

/// 四类自己编号的对象按（类型编码，位号）归拢。
type OwnGroups<'a> = BTreeMap<(&'a str, &'a str), Vec<Item<'a>>>;

/// 不单独编号的管件按所在管路的位号归拢。
type RunGroups<'a> = BTreeMap<&'a str, Vec<Item<'a>>>;

/// 管件所在的管路：记录的归属，没有就是记录里的位号（不单独编号的管件记的就是管路的位号）。
/// 两样都空时不在任何管路上。
fn run_of(object: &SummaryObject) -> Option<&str> {
    [object.owner.as_deref(), Some(object.tag.as_str())]
        .into_iter()
        .flatten()
        .find(|run| !run.trim().is_empty())
}

/// 重复的键（§3.1）：四类里同一个键有两处以上，或者键撞上了一条管路的管件行。各处按图、
/// 定位点、句柄排，各键按键排。
fn duplicates_of(own: &OwnGroups<'_>, runs: &RunGroups<'_>) -> Vec<Duplicate> {
    own.iter()
        .filter_map(|(&(type_code, tag), items)| {
            let clash = runs.get(tag).filter(|_| type_code == FITTING_ROW);
            if items.len() < 2 && clash.is_none() {
                return None;
            }
            let mut places: Vec<Place> = items
                .iter()
                .chain(clash.into_iter().flatten())
                .map(Item::place)
                .collect();
            places.sort_by(compare_places);
            Some(Duplicate {
                type_code: type_code.to_string(),
                tag: tag.to_string(),
                places,
            })
        })
        .collect()
}

/// 四类对象的一行：数量 1。
fn own_row(item: &Item<'_>) -> BomRow {
    let object = item.object;
    BomRow {
        seq: 0,
        kind: object.kind,
        type_code: object.type_code.clone(),
        tag: object.tag.clone(),
        name: prop(object, PROP_NAME)
            .unwrap_or(object.type_label.as_str())
            .to_string(),
        spec: [PROP_MODEL, PROP_SPEC]
            .into_iter()
            .filter_map(|key| prop(object, key))
            .collect::<Vec<_>>()
            .join(" "),
        quantity: Quantity::Count(1),
        unit: unit_of(object.kind).to_string(),
        instances: 1,
        drawings: vec![item.drawing.drawing_id.clone()],
    }
}

/// 一条管路上不单独编号的管件合成的一行（§3.1）。名称写各种符号及个数，个数多的在前，
/// 一样多的按显示名排；显示名取符号库的，库里没有写块名，不是符号的写类型的显示名。
fn fitting_row(run: &str, items: &[Item<'_>], labels: &BTreeMap<String, String>) -> BomRow {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for object in items.iter().map(|item| item.object) {
        let label = match object.symbol.as_deref() {
            Some(block) => labels.get(block).map_or(block, String::as_str),
            None => object.type_label.as_str(),
        };
        *counts.entry(label).or_default() += 1;
    }
    let mut counts: Vec<(&str, usize)> = counts.into_iter().collect();
    counts.sort_by_key(|&(_, count)| Reverse(count));
    let drawings: BTreeSet<&str> = items
        .iter()
        .map(|item| item.drawing.drawing_id.as_str())
        .collect();
    BomRow {
        seq: 0,
        kind: ObjectKind::Fitting,
        type_code: FITTING_ROW.to_string(),
        tag: run.to_string(),
        name: counts
            .iter()
            .map(|(label, count)| format!("{label}×{count}"))
            .collect::<Vec<_>>()
            .join("、"),
        spec: String::new(),
        quantity: Quantity::Count(items.len()),
        unit: unit_of(ObjectKind::Fitting).to_string(),
        instances: items.len(),
        drawings: drawings.into_iter().map(str::to_string).collect(),
    }
}

/// 一个属性的值，去掉两头空白；没有或只有空白时是 `None`。
fn prop<'a>(object: &'a SummaryObject, key: &str) -> Option<&'a str> {
    object
        .props
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

/// §3.1 表里的单位。
fn unit_of(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Equipment | ObjectKind::Instrument => "台",
        ObjectKind::PipeLine => "m",
        _ => "个",
    }
}

/// 行序里种类的先后：管线、设备、阀门、仪表、管件、其他。
fn rank(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::PipeLine => 0,
        ObjectKind::Equipment => 1,
        ObjectKind::Valve => 2,
        ObjectKind::Instrument => 3,
        ObjectKind::Fitting => 4,
        _ => 5,
    }
}

fn totals_of(rows: &[BomRow]) -> Totals {
    let mut by_kind = ByKind::default();
    for row in rows {
        let count = match row.kind {
            ObjectKind::PipeLine => &mut by_kind.pipe_line,
            ObjectKind::Equipment => &mut by_kind.equipment,
            ObjectKind::Valve => &mut by_kind.valve,
            ObjectKind::Instrument => &mut by_kind.instrument,
            ObjectKind::Fitting => &mut by_kind.fitting,
            _ => &mut by_kind.other,
        };
        *count += 1;
    }
    Totals {
        rows: rows.len(),
        by_kind,
    }
}

/// §6 第 5 条：产出的清单里（类型编码，位号）不重复。重复的键前面已经拦下，这里是最后一道。
fn assert_unique(rows: &[BomRow]) {
    let mut keys = BTreeSet::new();
    for row in rows {
        assert!(
            keys.insert((row.type_code.as_str(), row.tag.as_str())),
            "清单里（类型编码，位号）重复：{} {}",
            row.type_code,
            row.tag
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::pid::project_index::{SummarySource, SUMMARY_VERSION};

    fn label(type_code: &str) -> String {
        match type_code {
            "PUMP" => "泵",
            "VALVE" => "阀门",
            "VALVE_GATE" => "闸阀",
            "INSTRUMENT" => "仪表",
            "FITTING" => "管件",
            other => other,
        }
        .to_string()
    }

    /// 一个对象：只填清单用得到的几项，其余是空的。
    fn object(kind: ObjectKind, type_code: &str, tag: &str, handle: &str) -> SummaryObject {
        SummaryObject {
            kind,
            type_code: type_code.to_string(),
            type_label: label(type_code),
            tag: tag.to_string(),
            own_tag: kind.has_own_tag(),
            tag_source: String::new(),
            tag_status: String::new(),
            symbol: None,
            symbol_known: true,
            props: BTreeMap::new(),
            overrides: Vec::new(),
            owner: None,
            layer: "0".to_string(),
            at: [0.0, 0.0],
            bbox: [0.0; 4],
            handle: handle.to_string(),
        }
    }

    fn placed(object: SummaryObject, at: [f64; 2]) -> SummaryObject {
        SummaryObject { at, ..object }
    }

    fn with_props(object: SummaryObject, pairs: &[(&str, &str)]) -> SummaryObject {
        SummaryObject {
            props: pairs
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
            ..object
        }
    }

    /// 管路 `run` 上一个不单独编号的管件：记录里的位号就是管路的，归属也是它。
    fn fitting(run: &str, symbol: Option<&str>, handle: &str) -> SummaryObject {
        SummaryObject {
            symbol: symbol.map(str::to_string),
            owner: Some(run.to_string()),
            ..object(ObjectKind::Fitting, "FITTING", run, handle)
        }
    }

    fn drawing(drawing_id: &str, objects: Vec<SummaryObject>) -> DrawingSummary {
        DrawingSummary {
            version: SUMMARY_VERSION,
            drawing_id: drawing_id.to_string(),
            file: None,
            source: SummarySource::Open,
            fingerprint: None,
            pbs: None,
            length_unit: "mm".to_string(),
            objects,
            segments: Vec::new(),
            signal_ends: Vec::new(),
            connectors: Vec::new(),
            ports: Vec::new(),
        }
    }

    fn options() -> BomOptions {
        BomOptions {
            scope: BomScope::drawing("PID-1101"),
            source: BomSource::Live,
            generated_at: 1_794_800_000,
            symbol_labels: [
                ("PID_SYM_FITTINGS_ELBOW", "弯头"),
                ("PID_SYM_FITTINGS_TEE", "三通"),
            ]
            .iter()
            .map(|(block, label)| ((*block).to_string(), (*label).to_string()))
            .collect(),
        }
    }

    fn place(drawing_id: &str, at: [f64; 2], handle: &str) -> Place {
        Place {
            drawing_id: drawing_id.to_string(),
            at,
            handle: handle.to_string(),
        }
    }

    /// 一行里要对的几项：序号、种类、类型编码、位号、名称、规格、数量、单位、实例数。
    type Line = (
        usize,
        ObjectKind,
        String,
        String,
        String,
        String,
        Quantity,
        String,
        usize,
    );

    fn lines(bom: &Bom) -> Vec<Line> {
        bom.rows
            .iter()
            .map(|row| {
                (
                    row.seq,
                    row.kind,
                    row.type_code.clone(),
                    row.tag.clone(),
                    row.name.clone(),
                    row.spec.clone(),
                    row.quantity,
                    row.unit.clone(),
                    row.instances,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn line(
        seq: usize,
        kind: ObjectKind,
        type_code: &str,
        tag: &str,
        name: &str,
        spec: &str,
        count: usize,
        unit: &str,
    ) -> Line {
        (
            seq,
            kind,
            type_code.to_string(),
            tag.to_string(),
            name.to_string(),
            spec.to_string(),
            Quantity::Count(count),
            unit.to_string(),
            count,
        )
    }

    /// §3.1 的表与§6 第 1 条：阀门每个位号一行，数量 1；一条管路上的三个管件合成一行、数量 3，
    /// 名称写各种符号及个数（个数多的在前；符号库里没有的写块名，没有符号的写类型的显示名）；
    /// 泵 `P-1001` 与阀门 `P-1001` 各占一行；有编号的其他对象一行；管路、管线、打断符、信号线不出行。
    /// 名称取 `名称` 属性，没填用类型的显示名；规格是 `型号`、`规格` 里非空的（去掉两头空白）。
    /// 行序先按种类（设备、阀门、仪表、管件、其他）再按位号，序号从 1 连续编。
    #[test]
    fn rows_are_one_per_tag_and_fittings_merge_per_run() {
        let objects = vec![
            object(ObjectKind::Valve, "VALVE", "V-1002", "10"),
            with_props(
                object(ObjectKind::Valve, "VALVE_GATE", "V-1001", "11"),
                &[("名称", "闸阀 DN50"), ("型号", "Z41H"), ("规格", " PN16 ")],
            ),
            with_props(
                object(ObjectKind::Equipment, "PUMP", "P-1001", "12"),
                &[("名称", "进料泵"), ("型号", "  ")],
            ),
            object(ObjectKind::Valve, "VALVE", "P-1001", "13"),
            object(ObjectKind::Instrument, "INSTRUMENT", "TI-1001", "14"),
            object(ObjectKind::Other, "STRAINER", "S-1", "15"),
            fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "16"),
            fitting("L-1001-01", Some("PID_SYM_FITTINGS_TEE"), "17"),
            fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "18"),
            fitting("L-1002-01", Some("PID_SYM_UNKNOWN"), "19"),
            fitting("L-1002-01", None, "1A"),
            object(ObjectKind::PipeRun, "PIPE_RUN", "L-1001-01", "1B"),
            object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "1C"),
            SummaryObject {
                owner: Some("L-1001-01".to_string()),
                ..object(ObjectKind::Break, "FITTING", "", "1D")
            },
            object(ObjectKind::SignalLine, "SIGNAL_LINE", "S-01", "1E"),
        ];
        let bom = build(&[drawing("PID-1101", objects)], &options()).expect("清单");
        assert_eq!(
            lines(&bom),
            vec![
                line(
                    1,
                    ObjectKind::Equipment,
                    "PUMP",
                    "P-1001",
                    "进料泵",
                    "",
                    1,
                    "台"
                ),
                line(2, ObjectKind::Valve, "VALVE", "P-1001", "阀门", "", 1, "个"),
                line(
                    3,
                    ObjectKind::Valve,
                    "VALVE_GATE",
                    "V-1001",
                    "闸阀 DN50",
                    "Z41H PN16",
                    1,
                    "个"
                ),
                line(4, ObjectKind::Valve, "VALVE", "V-1002", "阀门", "", 1, "个"),
                line(
                    5,
                    ObjectKind::Instrument,
                    "INSTRUMENT",
                    "TI-1001",
                    "仪表",
                    "",
                    1,
                    "台"
                ),
                line(
                    6,
                    ObjectKind::Fitting,
                    "FITTING",
                    "L-1001-01",
                    "弯头×2、三通×1",
                    "",
                    3,
                    "个"
                ),
                line(
                    7,
                    ObjectKind::Fitting,
                    "FITTING",
                    "L-1002-01",
                    "PID_SYM_UNKNOWN×1、管件×1",
                    "",
                    2,
                    "个"
                ),
                line(
                    8,
                    ObjectKind::Other,
                    "STRAINER",
                    "S-1",
                    "STRAINER",
                    "",
                    1,
                    "个"
                ),
            ]
        );
        assert!(bom.rows.iter().all(|row| row.drawings == ["PID-1101"]));
        assert_eq!(bom.version, BOM_VERSION);
        assert_eq!(bom.generated_at, 1_794_800_000);
        assert_eq!(bom.scope, BomScope::drawing("PID-1101"));
        assert_eq!((bom.source, bom.snapshot_seq), (SourceKind::Live, None));
        assert!(!bom.length_unit_assumed);
        assert!(bom.pipe_summary.is_empty() && bom.skipped.is_empty() && bom.notices.is_empty());
        assert_eq!(
            bom.totals,
            Totals {
                rows: 8,
                by_kind: ByKind {
                    equipment: 1,
                    valve: 3,
                    instrument: 1,
                    fitting: 2,
                    other: 1,
                    ..ByKind::default()
                },
            }
        );
    }

    /// §3.1：不同图上管路位号相同的管件合成一行，`drawings` 列出各图；图的先后不影响结果。
    /// 范围和来源照选项写进清单。
    #[test]
    fn fittings_of_one_run_merge_across_drawings() {
        let first = drawing(
            "PID-1101",
            vec![
                fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "20"),
                fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "21"),
                object(ObjectKind::Valve, "VALVE", "V-1001", "22"),
            ],
        );
        let second = drawing(
            "PID-1102",
            vec![
                fitting("L-1001-01", Some("PID_SYM_FITTINGS_TEE"), "30"),
                object(ObjectKind::Valve, "VALVE", "V-1002", "31"),
            ],
        );
        let options = BomOptions {
            scope: BomScope::project(),
            source: BomSource::Snapshot(7),
            ..options()
        };
        let bom = build(&[first.clone(), second.clone()], &options).expect("清单");
        assert_eq!(
            lines(&bom),
            vec![
                line(1, ObjectKind::Valve, "VALVE", "V-1001", "阀门", "", 1, "个"),
                line(2, ObjectKind::Valve, "VALVE", "V-1002", "阀门", "", 1, "个"),
                line(
                    3,
                    ObjectKind::Fitting,
                    "FITTING",
                    "L-1001-01",
                    "弯头×2、三通×1",
                    "",
                    3,
                    "个"
                ),
            ]
        );
        assert_eq!(bom.rows[2].drawings, ["PID-1101", "PID-1102"]);
        assert_eq!(bom.rows[1].drawings, ["PID-1102"]);
        assert_eq!(bom.scope, BomScope::project());
        assert_eq!(
            (bom.source, bom.snapshot_seq),
            (SourceKind::Snapshot, Some(7))
        );
        assert_eq!(build(&[second, first], &options), Ok(bom));
    }

    /// §3.1、§6 第 2 条：同一类型下两个实例同位号，同图、跨图都报 `BOM_DUPLICATE_TAG`，没有清单；
    /// 列出各个键和各处位置。四类对象的键撞上一条管路的管件行（`FITTING` 加管路位号）也算。
    #[test]
    fn duplicate_tags_produce_no_bom() {
        let same_drawing = drawing(
            "PID-1101",
            vec![
                placed(
                    object(ObjectKind::Valve, "VALVE", "V-1001", "40"),
                    [10.0, 0.0],
                ),
                placed(
                    object(ObjectKind::Valve, "VALVE", "V-1001", "41"),
                    [20.0, 0.0],
                ),
                placed(
                    object(ObjectKind::Equipment, "PUMP", "P-1001", "42"),
                    [30.0, 0.0],
                ),
            ],
        );
        let other_drawing = drawing(
            "PID-1102",
            vec![placed(
                object(ObjectKind::Equipment, "PUMP", "P-1001", "50"),
                [5.0, 5.0],
            )],
        );
        let error = build(&[other_drawing.clone(), same_drawing.clone()], &options())
            .expect_err("有重复的键");
        assert_eq!(error.code(), BOM_DUPLICATE_TAG);
        assert_eq!(
            error,
            BomError::DuplicateTag(vec![
                Duplicate {
                    type_code: "PUMP".to_string(),
                    tag: "P-1001".to_string(),
                    places: vec![
                        place("PID-1101", [30.0, 0.0], "42"),
                        place("PID-1102", [5.0, 5.0], "50"),
                    ],
                },
                Duplicate {
                    type_code: "VALVE".to_string(),
                    tag: "V-1001".to_string(),
                    places: vec![
                        place("PID-1101", [10.0, 0.0], "40"),
                        place("PID-1101", [20.0, 0.0], "41"),
                    ],
                },
            ])
        );
        assert!(
            error.to_string().contains("PUMP P-1001、VALVE V-1001"),
            "{error}"
        );

        let error = build(&[same_drawing], &options()).expect_err("同图重复");
        assert!(
            matches!(&error, BomError::DuplicateTag(found) if found.len() == 1 && found[0].tag == "V-1001"),
            "{error:?}"
        );

        let clash = drawing(
            "PID-1101",
            vec![
                object(ObjectKind::Other, "FITTING", "L-1001-01", "60"),
                fitting("L-1001-01", Some("PID_SYM_FITTINGS_TEE"), "61"),
            ],
        );
        assert_eq!(
            build(&[clash], &options()),
            Err(BomError::DuplicateTag(vec![Duplicate {
                type_code: "FITTING".to_string(),
                tag: "L-1001-01".to_string(),
                places: vec![
                    place("PID-1101", [0.0, 0.0], "60"),
                    place("PID-1101", [0.0, 0.0], "61"),
                ],
            }]))
        );
    }

    /// §3.1、§6 第 2 条：位号为空（或只有空白）的设备、阀门、仪表不出行，列在 `skipped` 里
    /// （原因 `empty_tag`，按图、位置、句柄排），清单照常产出；不在任何管路上的管件也列在那里；
    /// 没有编号的其他对象不是清单里的东西，不出行也不列。
    #[test]
    fn empty_tags_are_skipped_and_listed() {
        let objects = vec![
            placed(
                object(ObjectKind::Instrument, "INSTRUMENT", "", "70"),
                [300.0, 40.0],
            ),
            placed(
                object(ObjectKind::Equipment, "PUMP", "  ", "71"),
                [10.0, 0.0],
            ),
            placed(object(ObjectKind::Valve, "VALVE", "", "72"), [10.0, 0.0]),
            placed(
                object(ObjectKind::Other, "ANNOTATION", "", "73"),
                [0.0, 0.0],
            ),
            placed(
                SummaryObject {
                    owner: None,
                    ..fitting("", None, "74")
                },
                [50.0, 50.0],
            ),
            object(ObjectKind::Valve, "VALVE", "V-1001", "75"),
        ];
        let bom = build(&[drawing("PID-1101", objects)], &options()).expect("清单照常产出");
        assert_eq!(
            lines(&bom),
            vec![line(
                1,
                ObjectKind::Valve,
                "VALVE",
                "V-1001",
                "阀门",
                "",
                1,
                "个"
            )]
        );
        let skipped = |kind, type_code: &str, at, handle: &str| Skipped {
            reason: SkipReason::EmptyTag,
            kind,
            type_code: type_code.to_string(),
            drawing_id: "PID-1101".to_string(),
            at,
            handle: handle.to_string(),
        };
        assert_eq!(
            bom.skipped,
            vec![
                skipped(ObjectKind::Equipment, "PUMP", [10.0, 0.0], "71"),
                skipped(ObjectKind::Valve, "VALVE", [10.0, 0.0], "72"),
                skipped(ObjectKind::Fitting, "FITTING", [50.0, 50.0], "74"),
                skipped(ObjectKind::Instrument, "INSTRUMENT", [300.0, 40.0], "70"),
            ]
        );
    }

    /// §3.2：清单 JSON 的字段和顺序；个数写成整数；种类写中文名；`by_kind` 按行序的种类、
    /// 没有的不写；读回来是同一份清单。
    #[test]
    fn bom_json_follows_section_3_2() {
        let objects = vec![
            object(ObjectKind::Valve, "VALVE", "V-1001", "2E0"),
            fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "2E1"),
            placed(
                object(ObjectKind::Instrument, "INSTRUMENT", "", "2E2"),
                [300.0, 40.0],
            ),
        ];
        let bom = build(&[drawing("PID-1101", objects)], &options()).expect("清单");
        let json = serde_json::to_string(&bom).expect("JSON");
        assert_eq!(
            json,
            concat!(
                r#"{"version":1,"generated_at":1794800000,"#,
                r#""scope":{"kind":"drawing","drawing_id":"PID-1101","pbs":null},"#,
                r#""source":"live","snapshot_seq":null,"length_unit_assumed":false,"rows":["#,
                r#"{"seq":1,"kind":"阀门","type_code":"VALVE","tag":"V-1001","name":"阀门","#,
                r#""spec":"","quantity":1,"unit":"个","instances":1,"drawings":["PID-1101"]},"#,
                r#"{"seq":2,"kind":"管件","type_code":"FITTING","tag":"L-1001-01","name":"弯头×1","#,
                r#""spec":"","quantity":1,"unit":"个","instances":1,"drawings":["PID-1101"]}],"#,
                r#""pipe_summary":[],"skipped":[{"reason":"empty_tag","kind":"仪表","#,
                r#""type_code":"INSTRUMENT","drawing_id":"PID-1101","at":[300.0,40.0],"handle":"2E2"}],"#,
                r#""totals":{"rows":2,"by_kind":{"阀门":1,"管件":1}},"notices":[]}"#,
            )
        );
        let back: Bom = serde_json::from_str(&json).expect("读回");
        assert_eq!(back, bom);
    }

    /// 管件行名称用的显示名来自符号库：每个块名对应它的 `label`。
    #[test]
    fn symbol_labels_come_from_the_symbol_library() {
        let library = SymbolLibrary::builtin();
        let labels = symbol_labels(&library);
        assert!(!library.block_names().is_empty());
        assert_eq!(labels.len(), library.block_names().len());
        for (block, label) in &labels {
            assert_eq!(&library.symbol(block).expect("块名").label, label);
        }
    }

    /// xorshift64，测试自带的伪随机数（不加依赖）。
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }

    /// 照§3.1 的定义数出来的重复键，不经过 `build`。
    fn duplicate_keys(drawings: &[DrawingSummary]) -> BTreeSet<(String, String)> {
        let mut own: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut runs = BTreeSet::new();
        for object in drawings.iter().flat_map(|drawing| &drawing.objects) {
            match object.kind {
                ObjectKind::Equipment
                | ObjectKind::Valve
                | ObjectKind::Instrument
                | ObjectKind::Other
                    if !object.tag.trim().is_empty() =>
                {
                    *own.entry((object.type_code.clone(), object.tag.clone()))
                        .or_default() += 1;
                }
                ObjectKind::Fitting => {
                    let run = object
                        .owner
                        .clone()
                        .filter(|run| !run.trim().is_empty())
                        .unwrap_or_else(|| object.tag.clone());
                    runs.insert(run);
                }
                _ => {}
            }
        }
        own.into_iter()
            .filter(|((type_code, tag), count)| {
                *count > 1 || (type_code == FITTING_ROW && runs.contains(tag))
            })
            .map(|(key, _)| key)
            .collect()
    }

    /// §6 第 5 条：随机生成的摘要（几张图，含重复位号、空位号、同一条管路的管件、键撞上管件行的），
    /// 要么报 `BOM_DUPLICATE_TAG`，要么产出的清单里键不重复；报不报、报哪些键和照定义数出来的
    /// 一致；同样的输入两次结果相同，图的先后不影响结果。
    #[test]
    fn random_summaries_either_report_duplicates_or_have_unique_keys() {
        const KINDS: [ObjectKind; 9] = [
            ObjectKind::Equipment,
            ObjectKind::Valve,
            ObjectKind::Instrument,
            ObjectKind::Fitting,
            ObjectKind::Break,
            ObjectKind::PipeLine,
            ObjectKind::PipeRun,
            ObjectKind::SignalLine,
            ObjectKind::Other,
        ];
        const TYPES: [&str; 5] = ["PUMP", "VALVE", "INSTRUMENT", "FITTING", "TANK"];
        const TAGS: [&str; 7] = ["", " ", "P-1", "V-1", "V-2", "L-1", "L-2"];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut reported, mut produced) = (0, 0);
        for _ in 0..600 {
            let mut drawings = Vec::new();
            for index in 0..1 + rng.below(3) {
                let mut objects = Vec::new();
                for number in 0..rng.below(7) {
                    let kind = KINDS[rng.below(KINDS.len())];
                    let type_code = TYPES[rng.below(TYPES.len())];
                    let tag = TAGS[rng.below(TAGS.len())];
                    let mut item = object(kind, type_code, tag, &format!("{:X}", number + 1));
                    if rng.below(2) == 0 {
                        item.owner = Some(TAGS[rng.below(TAGS.len())].to_string());
                    }
                    item.at = [rng.below(3) as f64, 0.0];
                    objects.push(item);
                }
                drawings.push(drawing(&format!("PID-{index}"), objects));
            }
            let expected = duplicate_keys(&drawings);
            let result = build(&drawings, &options());
            match &result {
                Err(BomError::DuplicateTag(found)) => {
                    reported += 1;
                    let keys: BTreeSet<(String, String)> = found
                        .iter()
                        .map(|duplicate| (duplicate.type_code.clone(), duplicate.tag.clone()))
                        .collect();
                    assert_eq!(keys, expected, "{drawings:?}");
                    assert!(found.iter().all(|duplicate| duplicate.places.len() >= 2));
                }
                Ok(bom) => {
                    produced += 1;
                    assert!(expected.is_empty(), "应报重复：{expected:?}");
                    let mut keys = BTreeSet::new();
                    assert!(
                        bom.rows
                            .iter()
                            .all(|row| keys.insert((row.type_code.clone(), row.tag.clone()))),
                        "键重复：{bom:?}"
                    );
                    assert!(bom
                        .rows
                        .iter()
                        .enumerate()
                        .all(|(index, row)| row.seq == index + 1));
                    assert_eq!(bom.totals.rows, bom.rows.len());
                }
            }
            assert_eq!(build(&drawings, &options()), result);
            drawings.reverse();
            assert_eq!(build(&drawings, &options()), result);
        }
        assert!(
            reported > 50 && produced > 50,
            "报 {reported}，产出 {produced}"
        );
    }
}

//! 材料清单（SPEC-pid-bom）：由范围内各图的摘要算出一份清单。
//!
//! [`build`] 只吃摘要，不碰图纸和文件（§4）：设备、阀门、仪表和有自己编号的其他对象
//! 每个（类型编码，位号）一行，不单独编号的管件按所在管路合成一行，每条管线一行管长
//! （§3.1）；管长另按（管道等级，公称直径，绝热等级）归成副表「管道汇总」（§3.3）。范围内
//! 同一个键有两处以上时整份清单不产出，返回 [`BomError::DuplicateTag`]；产出的清单里键不
//! 重复，出口处断言。
//!
//! [`rows`]、[`pipe_summary_rows`] 把主表、副表转成「第一行列名，其后每行一条」的文本表，
//! [`csv_bytes`] 再写成带 BOM 头的 CSV 字节（§3.4）；写文件在命令层。

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::project_index::{
    compare_places, DrawingSummary, Duplicate, ObjectKind, Place, SummaryObject, SummarySegment,
};
use super::symbols::SymbolLibrary;
use crate::app::annotation_data::rows_to_csv;

/// 清单格式的版本（§3.2 `version`）。
pub const BOM_VERSION: u32 = 1;

/// 范围里有重复的（类型编码，位号）（§3.5）。
pub const BOM_DUPLICATE_TAG: &str = "BOM_DUPLICATE_TAG";

/// 导出时写了不认识的列名（§3.5）。
pub const BOM_COLUMN_UNKNOWN: &str = "BOM_COLUMN_UNKNOWN";

/// 一条管路上不单独编号的管件合成的那一行的类型编码。
pub const FITTING_ROW: &str = "FITTING";

/// 一条管线那一行的类型编码。
pub const PIPE_LINE_ROW: &str = "PIPE_LINE";

/// 提示：有图没设长度单位（或单位不在 §3.1 的五种之列），管长按毫米算。
pub const LENGTH_UNIT_ASSUMED: &str = "LENGTH_UNIT_ASSUMED";

/// 提示：管长是图面长度，不是实际管长（§9 B1）。
pub const LENGTH_FROM_DRAWING: &str = "LENGTH_FROM_DRAWING";

/// 名称、规格取自这几个属性（§3.1）。
const PROP_NAME: &str = "名称";
const PROP_MODEL: &str = "型号";
const PROP_SPEC: &str = "规格";

/// 管线行的规格、副表的分组取自这几个属性（§3.1、§3.3）。
const PROP_PIPE_CLASS: &str = "管道等级";
const PROP_DIAMETER: &str = "公称直径";
const PROP_INSULATION: &str = "绝热等级";

/// 没有管线对象可取显示名时管线行的名称（管线类型的显示名）。
const PIPE_LINE_LABEL: &str = "管线";

/// 一毫米是多少米；没设单位的图也按它算（§3.1）。
const MILLIMETRE: f64 = 0.001;

/// 副表导出的六列（§3.4）。
const PIPE_SUMMARY_COLUMNS: [&str; 6] =
    ["管道等级", "公称直径", "绝热等级", "长度", "单位", "管线"];

/// 一格里有几张图（或几条管线）时用它隔开（§3.4）。
const LIST_SEPARATOR: &str = ";";

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

/// 算不出清单、导不出表的原因。
#[derive(Debug, Clone, PartialEq)]
pub enum BomError {
    /// 这些（类型编码，位号）在范围内各有两处以上（`BOM_DUPLICATE_TAG`）。
    DuplicateTag(Vec<Duplicate>),
    /// 选列里这些列名不认识（`BOM_COLUMN_UNKNOWN`）。
    ColumnUnknown(Vec<String>),
}

impl BomError {
    /// §3.5 的错误码。
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::DuplicateTag(_) => BOM_DUPLICATE_TAG,
            Self::ColumnUnknown(_) => BOM_COLUMN_UNKNOWN,
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
            Self::ColumnUnknown(names) => {
                let names: Vec<String> = names.iter().map(|name| format!("「{name}」")).collect();
                write!(
                    f,
                    "{BOM_COLUMN_UNKNOWN}: 不认识这些列名：{}；可用的列：{}",
                    names.join("、"),
                    Column::ALL.map(Column::header).join("、")
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
/// [`BomError::DuplicateTag`]；这四类的键撞上一条管路的管件行（`FITTING` 加管路位号）或
/// 一条管线的管线行（`PIPE_LINE` 加管线位号）也算。
pub fn build(drawings: &[DrawingSummary], options: &BomOptions) -> Result<Bom, BomError> {
    let mut ordered: Vec<&DrawingSummary> = drawings.iter().collect();
    ordered.sort_by(|left, right| left.drawing_id.cmp(&right.drawing_id));
    let groups = Groups::of(&ordered);
    let duplicates = groups.duplicates();
    if !duplicates.is_empty() {
        return Err(BomError::DuplicateTag(duplicates));
    }
    let mut rows: Vec<BomRow> = groups
        .pipes
        .iter()
        .map(|(tag, pipe)| pipe_row(tag, pipe))
        .chain(groups.own.values().map(|items| own_row(&items[0])))
        .chain(
            groups
                .fittings
                .iter()
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
    let assumed = groups.assumed_units();
    let notices = notices_of(&assumed, !groups.pipes.is_empty());
    let pipe_summary = groups.pipe_summary();
    let mut skipped = groups.skipped;
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
        length_unit_assumed: !assumed.is_empty(),
        totals: totals_of(&rows),
        rows,
        pipe_summary,
        skipped: skipped.into_iter().map(|(_, skipped)| skipped).collect(),
        notices,
    })
}

/// 主表转成「第一行列名，其后每行一条」的文本表，给 CSV / XLSX 用（§3.4）。`columns` 是要的列名
/// 和先后（去掉两头空白再认，同一列可以要几次）；不给或给空的是默认九列。
///
/// # Errors
///
/// `columns` 里有不认识的列名时返回 [`BomError::ColumnUnknown`]，列出全部不认识的。
pub fn rows(bom: &Bom, columns: Option<&[String]>) -> Result<Vec<Vec<String>>, BomError> {
    let names = columns.unwrap_or_default();
    let unknown: Vec<String> = names
        .iter()
        .filter(|name| Column::named(name).is_none())
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(BomError::ColumnUnknown(unknown));
    }
    let picked: Vec<Column> = if names.is_empty() {
        Column::ALL.to_vec()
    } else {
        names
            .iter()
            .filter_map(|name| Column::named(name))
            .collect()
    };
    let header = picked
        .iter()
        .map(|column| column.header().to_string())
        .collect();
    Ok(std::iter::once(header)
        .chain(
            bom.rows
                .iter()
                .map(|row| picked.iter().map(|column| column.cell(row)).collect()),
        )
        .collect())
}

/// 副表同理（§3.4）：管道等级、公称直径、绝热等级、长度、单位、管线六列。
#[must_use]
pub fn pipe_summary_rows(bom: &Bom) -> Vec<Vec<String>> {
    let header = PIPE_SUMMARY_COLUMNS.map(str::to_string).to_vec();
    std::iter::once(header)
        .chain(bom.pipe_summary.iter().map(|row| {
            vec![
                row.pipe_class.clone(),
                row.nominal_diameter.clone(),
                row.insulation_class.clone(),
                metres_text(row.length),
                row.unit.clone(),
                row.lines.join(LIST_SEPARATOR),
            ]
        }))
        .collect()
}

/// CSV 字节（§3.4）：UTF-8，文件头带 BOM（不然 Excel 打开中文是乱码）；逗号分隔、转义与提取表
/// 导出的 CSV 相同。
#[must_use]
pub fn csv_bytes(rows: &[Vec<String>]) -> Vec<u8> {
    format!("\u{feff}{}", rows_to_csv(rows)).into_bytes()
}

/// 主表导出的一列（§3.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Seq,
    Kind,
    Tag,
    Name,
    Quantity,
    Unit,
    Spec,
    TypeCode,
    Drawings,
}

impl Column {
    /// 默认九列，前六列是需求定的标准表头，顺序不变。
    const ALL: [Self; 9] = [
        Self::Seq,
        Self::Kind,
        Self::Tag,
        Self::Name,
        Self::Quantity,
        Self::Unit,
        Self::Spec,
        Self::TypeCode,
        Self::Drawings,
    ];

    fn header(self) -> &'static str {
        match self {
            Self::Seq => "序号",
            Self::Kind => "类型",
            Self::Tag => "位号",
            Self::Name => "名称",
            Self::Quantity => "数量",
            Self::Unit => "单位",
            Self::Spec => "规格",
            Self::TypeCode => "类型编码",
            Self::Drawings => "图纸",
        }
    }

    /// 列名去掉两头空白后是哪一列。
    fn named(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL.into_iter().find(|column| column.header() == name)
    }

    fn cell(self, row: &BomRow) -> String {
        match self {
            Self::Seq => row.seq.to_string(),
            Self::Kind => kind_label(row.kind).to_string(),
            Self::Tag => row.tag.clone(),
            Self::Name => row.name.clone(),
            Self::Quantity => match row.quantity {
                Quantity::Count(count) => count.to_string(),
                Quantity::Length(metres) => metres_text(metres),
            },
            Self::Unit => row.unit.clone(),
            Self::Spec => row.spec.clone(),
            Self::TypeCode => row.type_code.clone(),
            Self::Drawings => row.drawings.join(LIST_SEPARATOR),
        }
    }
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

/// 摘要里的一根线段，连同它所在的图。
struct Segment<'a> {
    drawing: &'a DrawingSummary,
    segment: &'a SummarySegment,
}

impl Segment<'_> {
    /// 按这张图的长度单位换算成的米数（§3.1）。
    fn metres(&self) -> f64 {
        let factor = metres_per_unit(&self.drawing.length_unit).unwrap_or(MILLIMETRE);
        self.segment.length * factor
    }

    fn place(&self) -> Place {
        Place {
            drawing_id: self.drawing.drawing_id.clone(),
            at: self.segment.a,
            handle: self.segment.handle.clone(),
        }
    }

    /// 管线位号为空的线段在 `skipped` 里的样子：种类记管线，位置是线段起点。
    fn skipped(&self) -> Skipped {
        Skipped {
            reason: SkipReason::EmptyTag,
            kind: ObjectKind::PipeLine,
            type_code: PIPE_LINE_ROW.to_string(),
            drawing_id: self.drawing.drawing_id.clone(),
            at: self.segment.a,
            handle: self.segment.handle.clone(),
        }
    }
}

/// 一条管线：画它的管线对象和它名下的线段，都按图号排。
#[derive(Default)]
struct Pipe<'a> {
    objects: Vec<Item<'a>>,
    segments: Vec<Segment<'a>>,
}

impl<'a> Pipe<'a> {
    /// 这条管线在哪里：画它的管线对象；一个也没有时是它名下的线段。
    fn places(&self) -> Vec<Place> {
        if self.objects.is_empty() {
            self.segments.iter().map(Segment::place).collect()
        } else {
            self.objects.iter().map(Item::place).collect()
        }
    }

    /// 这条管线在某张图上的管线对象；那张图上没有就取第一个。
    fn object_on(&self, drawing_id: &str) -> Option<&'a SummaryObject> {
        self.objects
            .iter()
            .find(|item| item.drawing.drawing_id == drawing_id)
            .or_else(|| self.objects.first())
            .map(|item| item.object)
    }
}

/// 范围内的对象和线段按行归拢的结果。
#[derive(Default)]
struct Groups<'a> {
    /// 四类自己编号的对象，按（类型编码，位号）。
    own: BTreeMap<(&'a str, &'a str), Vec<Item<'a>>>,
    /// 不单独编号的管件，按所在管路的位号。
    fittings: BTreeMap<&'a str, Vec<Item<'a>>>,
    /// 管线，按管线位号。
    pipes: BTreeMap<&'a str, Pipe<'a>>,
    /// 管路对象，按（图号，管路位号）；副表取管段的值用。
    runs: BTreeMap<(&'a str, &'a str), &'a SummaryObject>,
    /// 没出行的对象和线段，带着排序用的位置。
    skipped: Vec<(Place, Skipped)>,
}

impl<'a> Groups<'a> {
    /// 按传进来的顺序（`build` 已按图号排好）归拢各图的对象和线段。
    fn of(drawings: &[&'a DrawingSummary]) -> Self {
        let mut groups = Self::default();
        for &drawing in drawings {
            for object in &drawing.objects {
                groups.add_object(Item { drawing, object });
            }
            for segment in &drawing.segments {
                groups.add_segment(Segment { drawing, segment });
            }
        }
        groups
    }

    fn add_object(&mut self, item: Item<'a>) {
        let (drawing, object) = (item.drawing, item.object);
        let tagged = !object.tag.trim().is_empty();
        match object.kind {
            ObjectKind::Equipment
            | ObjectKind::Valve
            | ObjectKind::Instrument
            | ObjectKind::Other => {
                if tagged {
                    let key = (object.type_code.as_str(), object.tag.as_str());
                    self.own.entry(key).or_default().push(item);
                } else if object.kind != ObjectKind::Other {
                    self.skipped.push((item.place(), item.skipped()));
                }
            }
            ObjectKind::Fitting => match run_of(object) {
                Some(run) => self.fittings.entry(run).or_default().push(item),
                None => self.skipped.push((item.place(), item.skipped())),
            },
            ObjectKind::PipeLine if tagged => {
                let pipe = self.pipes.entry(object.tag.as_str()).or_default();
                pipe.objects.push(item);
            }
            ObjectKind::PipeRun => {
                let key = (drawing.drawing_id.as_str(), object.tag.as_str());
                self.runs.entry(key).or_insert(object);
            }
            ObjectKind::PipeLine | ObjectKind::Break | ObjectKind::SignalLine => {}
        }
    }

    /// 线段归到它的管线名下；管线位号为空的不算进管长，列进 `skipped`。
    fn add_segment(&mut self, segment: Segment<'a>) {
        let summary = segment.segment;
        if summary.line.trim().is_empty() {
            self.skipped.push((segment.place(), segment.skipped()));
        } else {
            let pipe = self.pipes.entry(summary.line.as_str()).or_default();
            pipe.segments.push(segment);
        }
    }

    /// 重复的键（§3.1）：四类里同一个键有两处以上，或者键撞上了一条管路的管件行、一条管线的
    /// 管线行。各处按图、定位点、句柄排，各键按键排。
    fn duplicates(&self) -> Vec<Duplicate> {
        self.own
            .iter()
            .filter_map(|(&(type_code, tag), items)| {
                let clash: Option<Vec<Place>> = match type_code {
                    FITTING_ROW => self
                        .fittings
                        .get(tag)
                        .map(|items| items.iter().map(Item::place).collect()),
                    PIPE_LINE_ROW => self.pipes.get(tag).map(Pipe::places),
                    _ => None,
                };
                if items.len() < 2 && clash.is_none() {
                    return None;
                }
                let mut places: Vec<Place> = items
                    .iter()
                    .map(Item::place)
                    .chain(clash.into_iter().flatten())
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

    /// 长度单位按毫米假定、又真有线段算进管长的图：图号 → 摘要里写的单位。
    fn assumed_units(&self) -> BTreeMap<&'a str, &'a str> {
        self.pipes
            .values()
            .flat_map(|pipe| &pipe.segments)
            .map(|segment| segment.drawing)
            .filter(|drawing| metres_per_unit(&drawing.length_unit).is_none())
            .map(|drawing| (drawing.drawing_id.as_str(), drawing.length_unit.as_str()))
            .collect()
    }

    /// 副表「管道汇总」（§3.3）：每根线段取它主人的（管道等级，公称直径，绝热等级），三个值都
    /// 相同的归成一行，换算成米累加后留 2 位小数；各行按三个值排，`lines` 是涉及的管线。
    fn pipe_summary(&self) -> Vec<PipeSummaryRow> {
        let mut sums: BTreeMap<[&str; 3], (f64, BTreeSet<&str>)> = BTreeMap::new();
        for (&line, pipe) in &self.pipes {
            for segment in &pipe.segments {
                let values = self.owner_of(pipe, segment).map_or([""; 3], pipe_values);
                let (metres, lines) = sums.entry(values).or_default();
                *metres += segment.metres();
                lines.insert(line);
            }
        }
        sums.into_iter()
            .map(|(values, (metres, lines))| {
                let [pipe_class, nominal_diameter, insulation_class] = values.map(str::to_string);
                PipeSummaryRow {
                    pipe_class,
                    nominal_diameter,
                    insulation_class,
                    length: round_metres(metres),
                    unit: unit_of(ObjectKind::PipeLine).to_string(),
                    lines: lines.into_iter().map(str::to_string).collect(),
                }
            })
            .collect()
    }

    /// 线段的主人（§3.3）：管段是这张图上它所在的那条管路；管线自己的线是这条管线（这张图上的
    /// 管线对象，没有就取第一个），这张图上找不到那条管路的管段也退到这条管线。
    fn owner_of(&self, pipe: &Pipe<'a>, segment: &Segment<'a>) -> Option<&'a SummaryObject> {
        let drawing_id = segment.drawing.drawing_id.as_str();
        segment
            .segment
            .run
            .as_deref()
            .filter(|run| !run.trim().is_empty())
            .and_then(|run| self.runs.get(&(drawing_id, run)).copied())
            .or_else(|| pipe.object_on(drawing_id))
    }
}

/// 管件所在的管路：记录的归属，没有就是记录里的位号（不单独编号的管件记的就是管路的位号）。
/// 两样都空时不在任何管路上。
fn run_of(object: &SummaryObject) -> Option<&str> {
    [object.owner.as_deref(), Some(object.tag.as_str())]
        .into_iter()
        .flatten()
        .find(|run| !run.trim().is_empty())
}

/// 一条管线的一行（§3.1）：名下各线段的长度按各自图纸的单位换算成米，先累加再留 2 位小数；
/// 名称、规格逐项取（按图号）第一个填了的管线对象，名称都没填用类型的显示名。
fn pipe_row(tag: &str, pipe: &Pipe<'_>) -> BomRow {
    let first = |key| pipe.objects.iter().find_map(|item| prop(item.object, key));
    let name = first(PROP_NAME)
        .or_else(|| {
            pipe.objects
                .first()
                .map(|item| item.object.type_label.as_str())
        })
        .unwrap_or(PIPE_LINE_LABEL);
    let spec: Vec<String> = [
        first(PROP_PIPE_CLASS).map(str::to_string),
        first(PROP_DIAMETER).map(diameter_spec),
    ]
    .into_iter()
    .flatten()
    .collect();
    let drawings: BTreeSet<&str> = pipe
        .objects
        .iter()
        .map(|item| item.drawing)
        .chain(pipe.segments.iter().map(|segment| segment.drawing))
        .map(|drawing| drawing.drawing_id.as_str())
        .collect();
    let metres: f64 = pipe.segments.iter().map(Segment::metres).sum();
    BomRow {
        seq: 0,
        kind: ObjectKind::PipeLine,
        type_code: PIPE_LINE_ROW.to_string(),
        tag: tag.to_string(),
        name: name.to_string(),
        spec: spec.join(" "),
        quantity: Quantity::Length(round_metres(metres)),
        unit: unit_of(ObjectKind::PipeLine).to_string(),
        instances: pipe.segments.len(),
        drawings: drawings.into_iter().map(str::to_string).collect(),
    }
}

/// 规格里的公称直径写成 `DN<公称直径>`；值本来就以 `DN` 开头的（不分大小写）照写。
fn diameter_spec(value: &str) -> String {
    if value
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("DN"))
    {
        value.to_string()
    } else {
        format!("DN{value}")
    }
}

/// 副表分组用的（管道等级，公称直径，绝热等级）：去掉两头空白，没填的是空串。
fn pipe_values(object: &SummaryObject) -> [&str; 3] {
    [PROP_PIPE_CLASS, PROP_DIAMETER, PROP_INSULATION]
        .map(|key| prop(object, key).unwrap_or_default())
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

/// 「类型」一列：种类的名字，与 `totals.by_kind` 的写法一样。
#[must_use]
pub fn kind_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::PipeLine => "管线",
        ObjectKind::Equipment => "设备",
        ObjectKind::Valve => "阀门",
        ObjectKind::Instrument => "仪表",
        ObjectKind::Fitting => "管件",
        _ => "其他",
    }
}

/// 表里的米数写满两位小数（§3.1 留 2 位小数）。
fn metres_text(metres: f64) -> String {
    format!("{metres:.2}")
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

/// 图纸长度单位换算成米的系数（§3.1 的五种）。没设单位（`unitless`）或不在五种之列的是
/// `None`，按毫米算。
fn metres_per_unit(unit: &str) -> Option<f64> {
    match unit {
        "mm" => Some(MILLIMETRE),
        "cm" => Some(0.01),
        "m" => Some(1.0),
        "in" => Some(0.0254),
        "ft" => Some(0.3048),
        _ => None,
    }
}

/// 米数留 2 位小数（§3.1）：舍入前加 `1e-9`，免得 24.505 这样的数因为浮点误差舍成 24.50。
fn round_metres(metres: f64) -> f64 {
    ((metres + 1e-9) * 100.0).round() / 100.0
}

/// 清单的提示（§3.2）：长度单位是假定的那些图（带摘要里写的单位）；有管线行时说明管长是
/// 图面长度（§9 B1）。
fn notices_of(assumed: &BTreeMap<&str, &str>, has_pipes: bool) -> Vec<Notice> {
    let mut notices = Vec::new();
    if !assumed.is_empty() {
        let drawings: Vec<String> = assumed
            .iter()
            .map(|(drawing, unit)| format!("{drawing}（{unit}）"))
            .collect();
        notices.push(Notice {
            code: LENGTH_UNIT_ASSUMED.to_string(),
            message: format!(
                "这些图没设长度单位或单位不在 mm、cm、m、in、ft 之列，管长按毫米算：{}",
                drawings.join("、")
            ),
        });
    }
    if has_pipes {
        notices.push(Notice {
            code: LENGTH_FROM_DRAWING.to_string(),
            message: "管长为图面长度：按图上线段的几何长度换算累加，不是实际管长".to_string(),
        });
    }
    notices
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
    use std::time::{Duration, Instant};

    use super::*;
    use crate::pid::project_index::{
        summarize, synthetic_drawing, SummaryMeta, SummarySource, DEFAULT_TOLERANCE,
        SUMMARY_VERSION,
    };

    fn label(type_code: &str) -> String {
        match type_code {
            "PUMP" => "泵",
            "VALVE" => "阀门",
            "VALVE_GATE" => "闸阀",
            "INSTRUMENT" => "仪表",
            "FITTING" => "管件",
            "PIPE_LINE" => "管线",
            "PIPE_RUN" => "管路",
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

    /// 一张图，带长度单位和线段。
    fn drawing_in(
        drawing_id: &str,
        unit: &str,
        objects: Vec<SummaryObject>,
        segments: Vec<SummarySegment>,
    ) -> DrawingSummary {
        DrawingSummary {
            length_unit: unit.to_string(),
            segments,
            ..drawing(drawing_id, objects)
        }
    }

    /// 一根线段：从原点沿 x 轴画 `length` 个图纸单位；`run` 是所在管路，管线自己的线是 `None`。
    fn segment(line: &str, run: Option<&str>, length: f64, handle: &str) -> SummarySegment {
        SummarySegment {
            line: line.to_string(),
            run: run.map(str::to_string),
            a: [0.0, 0.0],
            b: [length, 0.0],
            length,
            joint_a: None,
            joint_b: None,
            handle: handle.to_string(),
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

    /// 管线行要对的几项；数量是米。
    fn pipe(seq: usize, tag: &str, name: &str, spec: &str, metres: f64, instances: usize) -> Line {
        (
            seq,
            ObjectKind::PipeLine,
            PIPE_LINE_ROW.to_string(),
            tag.to_string(),
            name.to_string(),
            spec.to_string(),
            Quantity::Length(metres),
            "m".to_string(),
            instances,
        )
    }

    fn codes(bom: &Bom) -> Vec<&str> {
        bom.notices
            .iter()
            .map(|notice| notice.code.as_str())
            .collect()
    }

    /// §3.1 的表与§6 第 1 条：阀门每个位号一行，数量 1；一条管路上的三个管件合成一行、数量 3，
    /// 名称写各种符号及个数（个数多的在前；符号库里没有的写块名，没有符号的写类型的显示名）；
    /// 泵 `P-1001` 与阀门 `P-1001` 各占一行；有编号的其他对象一行；管路、打断符、信号线不出行。
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
    /// 列出各个键和各处位置。四类对象的键撞上一条管路的管件行（`FITTING` 加管路位号）或一条管线的
    /// 管线行（`PIPE_LINE` 加管线位号）也算；只画了线段的管线，位置是它的线段。
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

        let pipe_clash = drawing_in(
            "PID-1101",
            "m",
            vec![
                object(ObjectKind::Other, "PIPE_LINE", "L-1001", "62"),
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "63"),
                object(ObjectKind::Other, "PIPE_LINE", "L-2", "64"),
            ],
            vec![
                segment("L-1001", None, 5.0, "65"),
                segment("L-2", Some("L-2-01"), 1.0, "66"),
            ],
        );
        assert_eq!(
            build(&[pipe_clash], &options()),
            Err(BomError::DuplicateTag(vec![
                Duplicate {
                    type_code: "PIPE_LINE".to_string(),
                    tag: "L-1001".to_string(),
                    places: vec![
                        place("PID-1101", [0.0, 0.0], "62"),
                        place("PID-1101", [0.0, 0.0], "63"),
                    ],
                },
                Duplicate {
                    type_code: "PIPE_LINE".to_string(),
                    tag: "L-2".to_string(),
                    places: vec![
                        place("PID-1101", [0.0, 0.0], "64"),
                        place("PID-1101", [0.0, 0.0], "66"),
                    ],
                },
            ]))
        );
    }

    /// §3.1、§6 第 2 条：位号为空（或只有空白）的设备、阀门、仪表不出行，列在 `skipped` 里
    /// （原因 `empty_tag`，按图、位置、句柄排），清单照常产出；不在任何管路上的管件也列在那里；
    /// 没有编号的其他对象不是清单里的东西，不出行也不列。管线位号为空的线段（管路不在任何管线
    /// 名下，或管线没有位号）不算进管长，也列在那里：种类记管线，位置是线段起点。
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
        let segments = vec![
            segment("", None, 5.0, "76"),
            segment(" ", Some("L-9-01"), 2.0, "77"),
        ];
        let bom = build(
            &[drawing_in("PID-1101", "mm", objects, segments)],
            &options(),
        )
        .expect("清单照常产出");
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
                skipped(ObjectKind::PipeLine, "PIPE_LINE", [0.0, 0.0], "76"),
                skipped(ObjectKind::PipeLine, "PIPE_LINE", [0.0, 0.0], "77"),
                skipped(ObjectKind::Equipment, "PUMP", [10.0, 0.0], "71"),
                skipped(ObjectKind::Valve, "VALVE", [10.0, 0.0], "72"),
                skipped(ObjectKind::Fitting, "FITTING", [50.0, 50.0], "74"),
                skipped(ObjectKind::Instrument, "INSTRUMENT", [300.0, 40.0], "70"),
            ]
        );
        assert!(bom.pipe_summary.is_empty() && bom.notices.is_empty());
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

    /// §3.1 管长、§6 第 3 条：管线自己的线 10.005 米、两条管路上的管段 8.5、6 米合成一行，数量
    /// 24.51、实例数 3；名称取 `名称`，规格写 `<管道等级> DN<公称直径>`。舍入前加 1e-9：24.505 →
    /// 24.51，24.504999 → 24.5，0.005 → 0.01。没填属性的管线名称用类型的显示名、规格是空的。
    /// 管线行排在最前；有管线行就带提示 `LENGTH_FROM_DRAWING`。JSON 与§3.2 例子里的管线行相同。
    #[test]
    fn pipe_line_rows_sum_segment_lengths_then_round() {
        let objects = vec![
            with_props(
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "80"),
                &[
                    ("名称", "工艺水进料管线"),
                    ("管道等级", "A1A"),
                    ("公称直径", "50"),
                ],
            ),
            object(ObjectKind::PipeRun, "PIPE_RUN", "L-1001-01", "81"),
            object(ObjectKind::PipeRun, "PIPE_RUN", "L-1001-02", "82"),
            object(ObjectKind::Valve, "VALVE", "V-1001", "83"),
            object(ObjectKind::PipeLine, "PIPE_LINE", "L-2", "84"),
            object(ObjectKind::PipeLine, "PIPE_LINE", "L-3", "85"),
            object(ObjectKind::PipeLine, "PIPE_LINE", "L-4", "86"),
        ];
        let segments = vec![
            segment("L-1001", None, 10.005, "90"),
            segment("L-1001", Some("L-1001-01"), 8.5, "91"),
            segment("L-1001", Some("L-1001-02"), 6.0, "92"),
            segment("L-2", None, 24.505, "93"),
            segment("L-3", None, 24.504999, "94"),
            segment("L-4", None, 0.005, "95"),
        ];
        let summary = drawing_in("PID-1101", "m", objects, segments);
        let bom = build(&[summary], &options()).expect("清单");
        assert_eq!(
            lines(&bom),
            vec![
                pipe(1, "L-1001", "工艺水进料管线", "A1A DN50", 24.51, 3),
                pipe(2, "L-2", "管线", "", 24.51, 1),
                pipe(3, "L-3", "管线", "", 24.5, 1),
                pipe(4, "L-4", "管线", "", 0.01, 1),
                line(5, ObjectKind::Valve, "VALVE", "V-1001", "阀门", "", 1, "个"),
            ]
        );
        assert_eq!(codes(&bom), [LENGTH_FROM_DRAWING]);
        assert!(!bom.length_unit_assumed);
        assert_eq!(
            bom.totals.by_kind,
            ByKind {
                pipe_line: 4,
                valve: 1,
                ..ByKind::default()
            }
        );
        assert_eq!(
            serde_json::to_string(&bom.rows[0]).expect("JSON"),
            concat!(
                r#"{"seq":1,"kind":"管线","type_code":"PIPE_LINE","tag":"L-1001","#,
                r#""name":"工艺水进料管线","spec":"A1A DN50","quantity":24.51,"unit":"m","#,
                r#""instances":3,"drawings":["PID-1101"]}"#,
            )
        );
        let json = serde_json::to_string(&bom).expect("JSON");
        assert_eq!(serde_json::from_str::<Bom>(&json).expect("读回"), bom);
    }

    /// §3.1 单位、§6 第 4 条：五种单位各按各的换算，毫米图上 24510 个图纸单位是 24.51 米；没设
    /// 单位（`unitless`）和不在五种之列的（`km`）按毫米算，`length_unit_assumed` 为真，提示
    /// `LENGTH_UNIT_ASSUMED` 列出这些图和单位；同一条管线画在两张单位不同的图上，各自换算后相加，
    /// `drawings` 列出两张图。只有线段、没有管线对象的管线名称用类型的显示名。图的先后不影响结果。
    #[test]
    fn pipe_lengths_follow_each_drawings_unit() {
        let one = |drawing_id: &str, unit: &str, segments: Vec<SummarySegment>| {
            drawing_in(drawing_id, unit, Vec::new(), segments)
        };
        let drawings = vec![
            one(
                "PID-1",
                "mm",
                vec![
                    segment("L-MM", None, 24510.0, "A0"),
                    segment("L-X", None, 1000.0, "A1"),
                ],
            ),
            one("PID-2", "cm", vec![segment("L-CM", None, 2451.0, "A2")]),
            one("PID-3", "m", vec![segment("L-M", None, 24.51, "A3")]),
            one("PID-4", "in", vec![segment("L-IN", None, 1000.0, "A4")]),
            one(
                "PID-5",
                "ft",
                vec![
                    segment("L-FT", None, 100.0, "A5"),
                    segment("L-X", Some("L-X-01"), 10.0, "A6"),
                ],
            ),
            one(
                "PID-6",
                "unitless",
                vec![segment("L-U", None, 1500.0, "A7")],
            ),
            one("PID-7", "km", vec![segment("L-K", None, 2000.0, "A8")]),
        ];
        let bom = build(&drawings, &options()).expect("清单");
        assert_eq!(
            lines(&bom),
            vec![
                pipe(1, "L-CM", "管线", "", 24.51, 1),
                pipe(2, "L-FT", "管线", "", 30.48, 1),
                pipe(3, "L-IN", "管线", "", 25.4, 1),
                pipe(4, "L-K", "管线", "", 2.0, 1),
                pipe(5, "L-M", "管线", "", 24.51, 1),
                pipe(6, "L-MM", "管线", "", 24.51, 1),
                pipe(7, "L-U", "管线", "", 1.5, 1),
                pipe(8, "L-X", "管线", "", 4.05, 2),
            ]
        );
        assert_eq!(bom.rows[7].drawings, ["PID-1", "PID-5"]);
        assert!(bom.length_unit_assumed);
        assert_eq!(codes(&bom), [LENGTH_UNIT_ASSUMED, LENGTH_FROM_DRAWING]);
        let message = &bom.notices[0].message;
        assert!(
            message.contains("PID-6（unitless）、PID-7（km）"),
            "{message}"
        );
        let mut reversed = drawings.clone();
        reversed.reverse();
        assert_eq!(build(&reversed, &options()), Ok(bom));
    }

    /// §3.3、§6 第 6 条：副表按线段主人的（管道等级，公称直径，绝热等级）归行——管段取所在管路的
    /// 当前值（管路覆盖了直径的单独成组），这张图上找不到那条管路的取管线的；管线自己的线取管线
    /// 的值。没填的值记空串、单独成组；各组按三个值排，`lines` 列出涉及的管线；各组长度之和与
    /// 主表管线行长度之和相同。公称直径本来就写成 `DN80` 的，规格里照写。
    #[test]
    fn pipe_summary_groups_segments_by_their_owners_values() {
        let inherited = [("管道等级", "A1A"), ("公称直径", "50")];
        let objects = vec![
            with_props(
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "C0"),
                &inherited,
            ),
            with_props(
                object(ObjectKind::PipeRun, "PIPE_RUN", "L-1001-01", "C1"),
                &inherited,
            ),
            with_props(
                object(ObjectKind::PipeRun, "PIPE_RUN", "L-1001-02", "C2"),
                &[("管道等级", "A1A"), ("公称直径", "40")],
            ),
            with_props(
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1002", "C3"),
                &inherited,
            ),
            object(ObjectKind::PipeLine, "PIPE_LINE", "L-1003", "C4"),
            with_props(
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1004", "C5"),
                &[("管道等级", "B2B"), ("公称直径", "DN80"), ("绝热等级", "H")],
            ),
        ];
        let segments = vec![
            segment("L-1001", None, 4.0, "D0"),
            segment("L-1001", Some("L-1001-01"), 3.0, "D1"),
            segment("L-1001", Some("L-1001-02"), 2.0, "D2"),
            segment("L-1002", None, 1.0, "D3"),
            segment("L-1002", Some("L-1002-01"), 0.5, "D4"),
            segment("L-1003", None, 0.25, "D5"),
            segment("L-1004", None, 1.25, "D6"),
        ];
        let summary = drawing_in("PID-1101", "m", objects, segments);
        let bom = build(&[summary], &options()).expect("清单");
        assert_eq!(
            lines(&bom),
            vec![
                pipe(1, "L-1001", "管线", "A1A DN50", 9.0, 3),
                pipe(2, "L-1002", "管线", "A1A DN50", 1.5, 2),
                pipe(3, "L-1003", "管线", "", 0.25, 1),
                pipe(4, "L-1004", "管线", "B2B DN80", 1.25, 1),
            ]
        );
        let group = |class: &str, diameter: &str, insulation: &str, length, lines: &[&str]| {
            PipeSummaryRow {
                pipe_class: class.to_string(),
                nominal_diameter: diameter.to_string(),
                insulation_class: insulation.to_string(),
                length,
                unit: "m".to_string(),
                lines: lines.iter().map(|line| (*line).to_string()).collect(),
            }
        };
        assert_eq!(
            bom.pipe_summary,
            vec![
                group("", "", "", 0.25, &["L-1003"]),
                group("A1A", "40", "", 2.0, &["L-1001"]),
                group("A1A", "50", "", 8.5, &["L-1001", "L-1002"]),
                group("B2B", "DN80", "H", 1.25, &["L-1004"]),
            ]
        );
        let main_total: f64 = bom
            .rows
            .iter()
            .filter_map(|row| match row.quantity {
                Quantity::Length(metres) => Some(metres),
                Quantity::Count(_) => None,
            })
            .sum();
        let summary_total: f64 = bom.pipe_summary.iter().map(|row| row.length).sum();
        assert!(
            (main_total - summary_total).abs() < 1e-9,
            "{main_total} {summary_total}"
        );
        assert_eq!(
            serde_json::to_string(&bom.pipe_summary[2]).expect("JSON"),
            concat!(
                r#"{"pipe_class":"A1A","nominal_diameter":"50","insulation_class":"","#,
                r#""length":8.5,"unit":"m","lines":["L-1001","L-1002"]}"#,
            )
        );
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_string()).collect()
    }

    fn texts(table: &[&[&str]]) -> Vec<Vec<String>> {
        table.iter().map(|cells| names(cells)).collect()
    }

    /// §3.4 的默认九列。
    const HEADER: [&str; 9] = [
        "序号",
        "类型",
        "位号",
        "名称",
        "数量",
        "单位",
        "规格",
        "类型编码",
        "图纸",
    ];

    /// §3.4 例子里的两行（管线 `L-1001`、泵 `P-1001`），再加一条管路上分画在两张图里的两个弯头。
    fn exported() -> Bom {
        let first = drawing_in(
            "PID-1101",
            "mm",
            vec![
                with_props(
                    object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "E0"),
                    &[
                        ("名称", "工艺水进料管线"),
                        ("管道等级", "A1A"),
                        ("公称直径", "50"),
                    ],
                ),
                with_props(
                    object(ObjectKind::Equipment, "PUMP", "P-1001", "E1"),
                    &[("名称", "离心泵")],
                ),
                fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "E2"),
            ],
            vec![segment("L-1001", None, 24510.0, "E3")],
        );
        let second = drawing(
            "PID-1102",
            vec![fitting("L-1001-01", Some("PID_SYM_FITTINGS_ELBOW"), "F0")],
        );
        build(&[first, second], &options()).expect("清单")
    }

    /// §3.4、§6：默认九列，列名与先后同 §3.4 的表，前两行就是那里的例子；数量是米的写满两位
    /// 小数，「图纸」一格里几张图用分号隔开；`columns` 给空的也是默认九列。
    #[test]
    fn default_rows_are_the_nine_columns_of_section_3_4() {
        let bom = exported();
        let table = rows(&bom, None).expect("默认列");
        assert_eq!(
            table,
            texts(&[
                &HEADER,
                &[
                    "1",
                    "管线",
                    "L-1001",
                    "工艺水进料管线",
                    "24.51",
                    "m",
                    "A1A DN50",
                    "PIPE_LINE",
                    "PID-1101"
                ],
                &[
                    "2",
                    "设备",
                    "P-1001",
                    "离心泵",
                    "1",
                    "台",
                    "",
                    "PUMP",
                    "PID-1101"
                ],
                &[
                    "3",
                    "管件",
                    "L-1001-01",
                    "弯头×2",
                    "2",
                    "个",
                    "",
                    "FITTING",
                    "PID-1101;PID-1102"
                ],
            ])
        );
        assert_eq!(rows(&bom, Some(&[])).expect("空的选列"), table);

        let short = drawing_in(
            "PID-1",
            "m",
            vec![
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1", "A0"),
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-2", "A1"),
            ],
            vec![segment("L-1", None, 2.5, "B0")],
        );
        let table = rows(&build(&[short], &options()).expect("清单"), None).expect("默认列");
        let quantities: Vec<&str> = table[1..].iter().map(|cells| cells[4].as_str()).collect();
        assert_eq!(quantities, ["2.50", "0.00"]);
    }

    /// §3.4：`columns` 选列、排列，列名去掉两头空白再认，同一列可以要两次；有不认识的列名报
    /// `BOM_COLUMN_UNKNOWN`，列出全部不认识的，不出表。
    #[test]
    fn columns_pick_and_order_the_cells_and_unknown_names_are_refused() {
        let bom = exported();
        assert_eq!(
            rows(&bom, Some(&names(&["图纸", "位号", "数量"]))).expect("选列"),
            texts(&[
                &["图纸", "位号", "数量"],
                &["PID-1101", "L-1001", "24.51"],
                &["PID-1101", "P-1001", "1"],
                &["PID-1101;PID-1102", "L-1001-01", "2"],
            ])
        );
        assert_eq!(
            rows(&bom, Some(&names(&[" 名称 ", "名称"]))).expect("选列"),
            texts(&[
                &["名称", "名称"],
                &["工艺水进料管线", "工艺水进料管线"],
                &["离心泵", "离心泵"],
                &["弯头×2", "弯头×2"],
            ])
        );
        let error =
            rows(&bom, Some(&names(&["位号", "Tag", "重量", " "]))).expect_err("不认识的列名");
        assert_eq!(error, BomError::ColumnUnknown(names(&["Tag", "重量", " "])));
        assert_eq!(error.code(), BOM_COLUMN_UNKNOWN);
        assert!(
            error.to_string().contains("「Tag」、「重量」、「 」"),
            "{error}"
        );
    }

    /// §3.4：副表六列（管道等级、公称直径、绝热等级、长度、单位、管线），长度写满两位小数，几条
    /// 管线用分号隔开；没有管线时只有列名一行，主表也是。
    #[test]
    fn pipe_summary_rows_have_six_columns() {
        const SUMMARY_HEADER: [&str; 6] =
            ["管道等级", "公称直径", "绝热等级", "长度", "单位", "管线"];
        let classed = [("管道等级", "A1A"), ("公称直径", "50")];
        let summary = drawing_in(
            "PID-1101",
            "m",
            vec![
                with_props(
                    object(ObjectKind::PipeLine, "PIPE_LINE", "L-1001", "G0"),
                    &classed,
                ),
                with_props(
                    object(ObjectKind::PipeLine, "PIPE_LINE", "L-1002", "G1"),
                    &classed,
                ),
                object(ObjectKind::PipeLine, "PIPE_LINE", "L-1003", "G2"),
            ],
            vec![
                segment("L-1001", None, 2.5, "H0"),
                segment("L-1002", None, 1.0, "H1"),
                segment("L-1003", None, 0.25, "H2"),
            ],
        );
        let bom = build(&[summary], &options()).expect("清单");
        assert_eq!(
            pipe_summary_rows(&bom),
            texts(&[
                &SUMMARY_HEADER,
                &["", "", "", "0.25", "m", "L-1003"],
                &["A1A", "50", "", "3.50", "m", "L-1001;L-1002"],
            ])
        );
        let empty = build(&[drawing("PID-1101", Vec::new())], &options()).expect("空清单");
        assert_eq!(pipe_summary_rows(&empty), texts(&[&SUMMARY_HEADER]));
        assert_eq!(rows(&empty, None).expect("默认列"), texts(&[&HEADER]));
    }

    /// §3.4、§6：CSV 字节以 BOM 头 `EF BB BF` 开始，其后是 UTF-8；逗号分隔，每行以换行结束；
    /// 含逗号、引号、换行（含回车）的格子加双引号、引号成双，别的照写。
    #[test]
    fn csv_bytes_start_with_a_byte_order_mark_and_escape_cells() {
        let bytes = csv_bytes(&texts(&[
            &["名称", "规格"],
            &["泵, 离心", "3\" 法兰"],
            &["两行\n文字", "回车\r"],
            &["", "普通"],
        ]));
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "{bytes:02X?}");
        assert_eq!(
            std::str::from_utf8(&bytes[3..]).expect("UTF-8"),
            "名称,规格\n\"泵, 离心\",\"3\"\" 法兰\"\n\"两行\n文字\",\"回车\r\"\n,普通\n"
        );
        assert_eq!(
            csv_bytes(&rows(&exported(), None).expect("默认列")),
            concat!(
                "\u{feff}序号,类型,位号,名称,数量,单位,规格,类型编码,图纸\n",
                "1,管线,L-1001,工艺水进料管线,24.51,m,A1A DN50,PIPE_LINE,PID-1101\n",
                "2,设备,P-1001,离心泵,1,台,,PUMP,PID-1101\n",
                "3,管件,L-1001-01,弯头×2,2,个,,FITTING,PID-1101;PID-1102\n",
            )
            .as_bytes()
        );
        assert_eq!(csv_bytes(&[]), [0xEF, 0xBB, 0xBF]);
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
        let mut pipes = BTreeSet::new();
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
                ObjectKind::PipeLine => {
                    pipes.insert(object.tag.clone());
                }
                _ => {}
            }
        }
        pipes.extend(
            drawings
                .iter()
                .flat_map(|drawing| &drawing.segments)
                .map(|segment| segment.line.clone()),
        );
        own.into_iter()
            .filter(|((type_code, tag), count)| {
                *count > 1
                    || (type_code == FITTING_ROW && runs.contains(tag))
                    || (type_code == PIPE_LINE_ROW && pipes.contains(tag))
            })
            .map(|(key, _)| key)
            .collect()
    }

    /// §6 第 5、6 条：随机生成的摘要（几张图，单位各异；含重复位号、空位号、同一条管路的管件、
    /// 各种线段，键撞上管件行或管线行的），要么报 `BOM_DUPLICATE_TAG`，要么产出的清单里键不重复；
    /// 报不报、报哪些键和照定义数出来的一致。产出的清单里副表长度之和与管线行长度之和相差不过
    /// 每行 0.005 米，管线行的实例数合起来是有管线位号的线段数，两条提示与 `length_unit_assumed`、
    /// 有没有管线行对得上；默认列的文本表比清单多一行列名、每行九格、序号一列就是行序，副表的
    /// 文本表也多一行列名，CSV 的行数与文本表相同（这些格子里没有要转义的字符）。同样的输入
    /// 两次结果相同，图的先后不影响结果。
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
        const TYPES: [&str; 6] = [
            "PUMP",
            "VALVE",
            "INSTRUMENT",
            "FITTING",
            "TANK",
            "PIPE_LINE",
        ];
        const UNITS: [&str; 4] = ["mm", "m", "ft", "unitless"];
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
                let mut segments = Vec::new();
                for number in 0..rng.below(4) {
                    let line = TAGS[rng.below(TAGS.len())];
                    let run = (rng.below(2) == 0).then(|| TAGS[rng.below(TAGS.len())]);
                    let length = rng.below(100_000) as f64 / 7.0;
                    let handle = format!("{:X}", 0x100 + number);
                    segments.push(segment(line, run, length, &handle));
                }
                let unit = UNITS[rng.below(UNITS.len())];
                drawings.push(drawing_in(&format!("PID-{index}"), unit, objects, segments));
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
                    let metres: Vec<f64> = bom
                        .rows
                        .iter()
                        .filter_map(|row| match row.quantity {
                            Quantity::Length(metres) => Some(metres),
                            Quantity::Count(_) => None,
                        })
                        .collect();
                    let summary: f64 = bom.pipe_summary.iter().map(|row| row.length).sum();
                    let slack = 0.005 * (metres.len() + bom.pipe_summary.len()) as f64 + 1e-9;
                    assert!(
                        (metres.iter().sum::<f64>() - summary).abs() <= slack,
                        "{bom:?}"
                    );
                    let counted = drawings
                        .iter()
                        .flat_map(|drawing| &drawing.segments)
                        .filter(|segment| !segment.line.trim().is_empty())
                        .count();
                    let instances: usize = bom
                        .rows
                        .iter()
                        .filter(|row| row.kind == ObjectKind::PipeLine)
                        .map(|row| row.instances)
                        .sum();
                    assert_eq!(instances, counted);
                    let has = |code| bom.notices.iter().any(|notice| notice.code == code);
                    assert_eq!(has(LENGTH_UNIT_ASSUMED), bom.length_unit_assumed);
                    assert_eq!(has(LENGTH_FROM_DRAWING), !metres.is_empty());
                    let table = rows(bom, None).expect("默认列");
                    assert_eq!(table.len(), bom.rows.len() + 1);
                    assert!(table.iter().all(|cells| cells.len() == 9));
                    assert!(bom
                        .rows
                        .iter()
                        .zip(&table[1..])
                        .all(|(row, cells)| cells[0] == row.seq.to_string()));
                    assert_eq!(pipe_summary_rows(bom).len(), bom.pipe_summary.len() + 1);
                    let csv = csv_bytes(&table);
                    let text = std::str::from_utf8(&csv[3..]).expect("UTF-8");
                    assert_eq!(text.lines().count(), table.len());
                }
                Err(error) => panic!("算清单只会报重复键：{error}"),
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

    /// 契约规模：2,000 个对象的合成图（`IX-4`）。
    const SYNTHETIC_OBJECTS: usize = 2000;
    /// 预算：一张图算一份清单（含做摘要）不超过 300 ms（成功标准第 9 条）。
    const BUDGET: Duration = Duration::from_millis(300);
    /// 采样次数：取中位数。
    const RUNS: usize = 3;

    /// 计时的是做摘要、取符号的显示名、算清单，`BOM-4` 的入口函数每次都做这三件事。合成图
    /// 每组 20 个对象出 14 行：两条管线、三台设备、四个阀门、四块仪表、一条管路上的管件；没有跳过的。
    #[test]
    #[ignore]
    fn bom_build_2000_objects_within_300_ms() {
        let doc = synthetic_drawing(SYNTHETIC_OBJECTS);
        let library = SymbolLibrary::builtin();
        let meta = || SummaryMeta {
            drawing_id: "PID-1101".to_owned(),
            file: None,
            pbs: None,
            source: SummarySource::Open,
            fingerprint: None,
        };
        let cells = SYNTHETIC_OBJECTS / 20;
        let expected = ByKind {
            pipe_line: 2 * cells,
            equipment: 3 * cells,
            valve: 4 * cells,
            instrument: 4 * cells,
            fitting: cells,
            other: 0,
        };
        let mut samples = Vec::with_capacity(RUNS);
        for run in 1..=RUNS {
            let started = Instant::now();
            let summary = summarize(&doc, &library, meta(), DEFAULT_TOLERANCE);
            let summarized = started.elapsed();
            let options = BomOptions {
                symbol_labels: symbol_labels(&library),
                ..options()
            };
            let bom = build(std::slice::from_ref(&summary), &options).expect("合成图的位号不重复");
            let elapsed = started.elapsed();
            println!(
                "BOM-3 run {run}/{RUNS}: {:.2} ms (summary {:.2} ms), rows {}, pipe summary {}, skipped {}",
                elapsed.as_secs_f64() * 1e3,
                summarized.as_secs_f64() * 1e3,
                bom.rows.len(),
                bom.pipe_summary.len(),
                bom.skipped.len()
            );
            assert_eq!(bom.totals.by_kind, expected, "run {run}");
            assert!(
                bom.skipped.is_empty(),
                "run {run}: {:?}",
                bom.skipped.first()
            );
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[RUNS / 2];
        let shown: Vec<String> = samples
            .iter()
            .map(|sample| format!("{:.2}", sample.as_secs_f64() * 1e3))
            .collect();
        println!(
            "BOM-3 median {:.2} ms of {shown:?}, budget {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
        assert!(
            median <= BUDGET,
            "BOM-3 超预算：中位数 {:.2} ms（{shown:?}），预算 {} ms",
            median.as_secs_f64() * 1e3,
            BUDGET.as_millis()
        );
    }
}

//! 发布包（SPEC-pid-publish）：包名与修订号（§3.1）、重复位号与四项门禁（§3.4）、图里有同步
//! 记录时的协同前提（SPEC-collab-offline-sync §5.7），以及写包、落位、校验（§3.1 – §3.3、§3.5、§4）。
//!
//! 出包的流程（取摘要、检查、存快照、出图）在命令层；这里只管文件：[`write_package`] 往调用方
//! 建的临时目录里写除两张出图以外的文件和发布清单，[`commit`] 自检后把临时目录改名成包目录，
//! [`verify_package`] 校验一个包。临时目录怎么建、哪一步失败后删掉它，归调用方。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::bom::{self, Bom};
use super::drc::CheckReport;
use super::master_data::{self, MasterData, Resolved};
use super::project_index::{
    compare_places, DrawingSummary, Duplicate, LinkStatus, ObjectKind, Place, ProjectIndex,
    SummaryObject,
};
use super::snapshot::{store, Snapshot};

/// 门禁全过时 [`Gates::result`] 的值。
pub const PASSED: &str = "放行";
/// 门禁没过（§3.6，HTTP 409）。
pub const PUBLISH_BLOCKED: &str = "PUBLISH_BLOCKED";
/// 图里有同步记录，队列不空（离线规格 §5.7）。
pub const COLLAB_UNSYNCED: &str = "COLLAB_UNSYNCED";
/// 图里有同步记录，标了不一致，或者这次连接对不上。
pub const COLLAB_DIVERGED: &str = "COLLAB_DIVERGED";
/// 图里有同步记录，没连着服务器或者没补齐，也没带「允许未对账」。
pub const COLLAB_OFFLINE: &str = "COLLAB_OFFLINE";

// ── 包名与修订号 ───────────────────────────────────────────────────────────────

/// 目标目录里这张图的一个包。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    /// 包名，也是目录名，如 `PID-1101-r003`。
    pub package_id: String,
    pub revision: u32,
    pub path: PathBuf,
}

/// 第 `revision` 版的包名（§3.1）：`<图号>-r<三位修订号>`，修订号不足三位补零、超过不截。
/// 图号里不能做文件名的字符换成 `_`，与快照目录同一条规则（[`store::dir_for`]）。
#[must_use]
pub fn package_name(drawing_id: &str, revision: u32) -> String {
    name_of(&file_stem(drawing_id), revision)
}

/// 目标目录 `target_dir` 里这张图的包，按修订号从小到大。只认名字恰好是 [`package_name`]
/// 写法的目录，修订号从 1 起：别的写法、同名的文件、别的图的包都不算。目录不存在或读不了
/// 时是空的。
#[must_use]
pub fn list_packages(target_dir: &Path, drawing_id: &str) -> Vec<PackageInfo> {
    let Ok(entries) = fs::read_dir(target_dir) else {
        return Vec::new();
    };
    let stem = file_stem(drawing_id);
    let mut packages: Vec<PackageInfo> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let package_id = entry.file_name().into_string().ok()?;
            let revision = revision_of(&package_id, &stem)?;
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            Some(PackageInfo {
                package_id,
                revision,
                path,
            })
        })
        .collect();
    packages.sort_by_key(|package| package.revision);
    packages
}

/// 不指定修订号时用的下一版（§3.1）：目标目录里这张图已有的最大修订号加一，一个都没有时是 1。
#[must_use]
pub fn next_revision(target_dir: &Path, drawing_id: &str) -> u32 {
    list_packages(target_dir, drawing_id)
        .last()
        .map_or(1, |package| package.revision.saturating_add(1))
}

/// 图号做成文件名的样子：快照目录 `pid-snapshots/<图号>` 的最后一段。
fn file_stem(drawing_id: &str) -> String {
    store::dir_for(Path::new(""), drawing_id)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("换过字符的图号是一段完整的 UTF-8 文件名")
        .to_owned()
}

fn name_of(stem: &str, revision: u32) -> String {
    format!("{stem}-r{revision:03}")
}

/// 目录名 `name` 是这张图（图号做成文件名是 `stem`）的第几版；不是它的包时是 `None`。
fn revision_of(name: &str, stem: &str) -> Option<u32> {
    let revision: u32 = name.strip_prefix(stem)?.strip_prefix("-r")?.parse().ok()?;
    (revision > 0 && name_of(stem, revision) == name).then_some(revision)
}

// ── 重复位号 ──────────────────────────────────────────────────────────────────

/// 数重复位号（§3.4 第二项），不经过规则：本图 `current` 里同一个（类型编码，位号）出现两次
/// 以上，或者本图的某个（类型编码，位号）在项目的另一张图上也有，各算一个。只数有自己编号的
/// 设备、阀门、仪表、其他；位号去掉首尾空白再比，空的不数。
///
/// 别的图上的出现处从项目索引 `index` 的跨图重号和别的图的摘要 `others` 里都找，两处都有的
/// 同一处（图号、句柄都相同）只记一次。图号与本图相同的不看：本图以 `current` 为准，索引里
/// 的可能是改动之前的。每个重复位号列出全部出现处，按图号、定位点、句柄排；各个之间按类型
/// 编码、位号排。
#[must_use]
pub fn duplicate_tags(
    current: &DrawingSummary,
    index: &ProjectIndex,
    others: &[DrawingSummary],
) -> Vec<Duplicate> {
    let mut found: BTreeMap<(&str, &str), Vec<Place>> = BTreeMap::new();
    for object in current.objects.iter().filter(|object| counted(object)) {
        found
            .entry((object.type_code.as_str(), object.tag.trim()))
            .or_default()
            .push(place(&current.drawing_id, object));
    }
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut add = |places: &mut Vec<Place>, place: Place| {
        if seen.insert((place.drawing_id.clone(), place.handle.clone())) {
            places.push(place);
        }
    };
    for duplicate in &index.duplicates {
        let Some(places) = found.get_mut(&(duplicate.type_code.as_str(), duplicate.tag.trim()))
        else {
            continue;
        };
        for place in duplicate
            .places
            .iter()
            .filter(|place| place.drawing_id != current.drawing_id)
        {
            add(places, place.clone());
        }
    }
    for summary in others
        .iter()
        .filter(|summary| summary.drawing_id != current.drawing_id)
    {
        for object in summary.objects.iter().filter(|object| counted(object)) {
            if let Some(places) = found.get_mut(&(object.type_code.as_str(), object.tag.trim())) {
                add(places, place(&summary.drawing_id, object));
            }
        }
    }
    found
        .into_iter()
        .filter(|(_, places)| places.len() > 1)
        .map(|((type_code, tag), mut places)| {
            places.sort_by(compare_places);
            Duplicate {
                type_code: type_code.to_owned(),
                tag: tag.to_owned(),
                places,
            }
        })
        .collect()
}

/// 数重复位号时算的对象：有自己编号的设备、阀门、仪表、其他，位号去掉首尾空白后不空。
fn counted(object: &SummaryObject) -> bool {
    object.own_tag
        && !object.tag.trim().is_empty()
        && matches!(
            object.kind,
            ObjectKind::Equipment | ObjectKind::Valve | ObjectKind::Instrument | ObjectKind::Other
        )
}

fn place(drawing_id: &str, object: &SummaryObject) -> Place {
    Place {
        drawing_id: drawing_id.to_owned(),
        at: object.at,
        handle: object.handle.clone(),
    }
}

// ── 门禁 ──────────────────────────────────────────────────────────────────────

/// 生效的主数据读得怎么样（§3.4 第三项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MasterDataState {
    Configured,
    /// 不单独拦。
    NotConfigured,
    /// 拦。
    Unreadable,
}

impl From<&Resolved> for MasterDataState {
    fn from(resolved: &Resolved) -> Self {
        match resolved {
            Resolved::Configured { .. } => Self::Configured,
            Resolved::NotConfigured => Self::NotConfigured,
            Resolved::Unreadable { .. } => Self::Unreadable,
        }
    }
}

/// 四项门禁和各项计数（§3.4）。字段顺序就是 JSON 里的顺序（§5.3 的 `gates`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gates {
    /// 没被豁免的错误。
    pub drc_errors: usize,
    pub drc_warnings: usize,
    pub drc_hints: usize,
    /// 已豁免的结果，不拦。
    pub waived: usize,
    pub duplicate_tags: usize,
    pub master_data: MasterDataState,
    /// 项目里的图都读出来了。
    pub index_complete: bool,
    /// 四项都过是 [`PASSED`]，否则是 [`PUBLISH_BLOCKED`]。
    pub result: &'static str,
}

impl Gates {
    /// 四项都过了。
    #[must_use]
    pub fn passed(&self) -> bool {
        self.result == PASSED
    }
}

/// 算四项门禁（§3.4）：检查报告 `report` 里没有未豁免的错误、重复位号 `duplicates` 为 0、
/// 主数据 `master` 不是读不出来、项目索引完整（`index_complete`），四项都满足才放行。警告、
/// 提示、已豁免的只计数；主数据没配置不单独拦。
#[must_use]
pub fn gates(
    report: &CheckReport,
    duplicates: usize,
    master: &Resolved,
    index_complete: bool,
) -> Gates {
    let summary = &report.summary;
    let master_data = MasterDataState::from(master);
    let passed = summary.errors == 0
        && duplicates == 0
        && master_data != MasterDataState::Unreadable
        && index_complete;
    Gates {
        drc_errors: summary.errors,
        drc_warnings: summary.warnings,
        drc_hints: summary.infos,
        waived: summary.waived,
        duplicate_tags: duplicates,
        master_data,
        index_complete,
        result: if passed { PASSED } else { PUBLISH_BLOCKED },
    }
}

// ── 协同前提 ──────────────────────────────────────────────────────────────────

/// 协同前提要看的几样（离线规格 §5.7），由调用方从图里的同步记录和这次连接取。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollabInput {
    /// 图里有同步记录。没有时别的几项都不看。
    pub has_record: bool,
    /// 队列里还没回答的修改条数。
    pub queued: usize,
    /// 图里标了不一致，或者这次连接对不上。
    pub diverged: bool,
    /// 正连着服务器，并且已经补齐。
    pub caught_up: bool,
    /// 带了「允许未对账」（命令的 `ALLOW_OFFLINE`、接口的 `allow_offline: true`）。
    pub allow_offline: bool,
}

/// [`collab_preconditions`] 的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollabVerdict {
    /// 图里没有同步记录，不查；清单的 `collab` 是 `null`。
    NotApplicable,
    /// 放行；`reconciled` 记进清单的 `collab`。
    Pass { reconciled: bool },
    /// 拦，带错误码：[`COLLAB_UNSYNCED`]、[`COLLAB_DIVERGED`] 或 [`COLLAB_OFFLINE`]。
    Refused(&'static str),
}

/// 协同前提（离线规格 §5.7），只对图里有同步记录的图：按队列是空的、没有不一致、连着并且
/// 补齐的顺序查，报先不满足的那一条。「允许未对账」只放过最后一条，这时 `reconciled` 为假；
/// 前两条带了也拦。
#[must_use]
pub fn collab_preconditions(input: CollabInput) -> CollabVerdict {
    if !input.has_record {
        CollabVerdict::NotApplicable
    } else if input.queued > 0 {
        CollabVerdict::Refused(COLLAB_UNSYNCED)
    } else if input.diverged {
        CollabVerdict::Refused(COLLAB_DIVERGED)
    } else if input.caught_up {
        CollabVerdict::Pass { reconciled: true }
    } else if input.allow_offline {
        CollabVerdict::Pass { reconciled: false }
    } else {
        CollabVerdict::Refused(COLLAB_OFFLINE)
    }
}

// ── 写包、落位、校验 ─────────────────────────────────────────────────────────────

/// 发布清单的文件名（§3.2）。
pub const MANIFEST_FILE: &str = "manifest.json";
/// 发布清单的 `kind`。
pub const MANIFEST_KIND: &str = "pid-publish-manifest";
/// 发布清单格式的版本。
pub const MANIFEST_VERSION: u32 = 1;
/// `topology.json` 格式的版本。
pub const TOPOLOGY_VERSION: u32 = 1;

/// 目标目录里已经有这个包（§3.1）：已发布的包不覆盖。
pub const PUBLISH_TARGET_EXISTS: &str = "PUBLISH_TARGET_EXISTS";
/// 出包的某一步失败了，哪一步见 [`PublishError::step`]（§3.6）。
pub const PUBLISH_FAILED: &str = "PUBLISH_FAILED";
/// 要校验的目录不存在，或者里面没有发布清单（§3.5）。
pub const PACKAGE_NOT_FOUND: &str = "PACKAGE_NOT_FOUND";
/// 包里缺了清单列的文件，或者清单里缺了某一类（§3.5）。
pub const PACKAGE_INCOMPLETE: &str = "PACKAGE_INCOMPLETE";
/// 包里的文件与清单记的字节数或校验和不同，或者清单本身读不出来（§3.5）。
pub const PACKAGE_CORRUPT: &str = "PACKAGE_CORRUPT";

const DRAWING: &str = "图纸文件";
const BILL: &str = "材料清单";
const REPORTS: &str = "报表";
const MASTER_DATA: &str = "项目主数据快照";
const CHECK_REPORT: &str = "校验结果报告";
/// 完整的包每一类至少有一个文件（§3.1）。
const KINDS: [&str; 5] = [DRAWING, BILL, REPORTS, MASTER_DATA, CHECK_REPORT];

const BOM_CSV: &str = "bom.csv";
const BOM_JSON: &str = "bom.json";
const PIPE_SUMMARY_CSV: &str = "pipe-summary.csv";
const TOPOLOGY_JSON: &str = "topology.json";
const SNAPSHOT_JSON: &str = "snapshot.json";
const MASTER_DATA_JSON: &str = "master-data.json";
const DRC_REPORT_JSON: &str = "drc-report.json";

/// 两张出图在包里的文件名：PDF 和 SVG，主名与包名同一规则（§3.1）。命令层出图时照这两个
/// 名字写进临时目录，[`write_package`] 再去那里取。
#[must_use]
pub fn plot_files(drawing_id: &str) -> [String; 2] {
    let stem = file_stem(drawing_id);
    [format!("{stem}.pdf"), format!("{stem}.svg")]
}

/// 清单的 `project`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
}

/// 清单的 `collab`：图里的同步记录（离线规格 §5.7）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollabRecord {
    pub drawing_id: String,
    pub repo_id: String,
    pub revision: i64,
    /// 协同前提是连着补齐放行的；带「允许未对账」放行的为假。
    pub reconciled: bool,
}

/// 清单的 `source`：发布时的源文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// 源文件自己的文件名，不带目录。
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}

/// 清单 `contents[]` 的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    /// 属于哪一类（§3.1）。
    pub kind: String,
    /// 包里的文件名。
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// 发布清单（§3.2）。字段顺序就是 `manifest.json` 里的顺序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub version: u32,
    pub kind: &'static str,
    pub package_id: String,
    pub project: Option<ProjectRef>,
    pub drawing_id: String,
    pub revision: u32,
    /// 出包的时间，Unix 秒。
    pub published_at: i64,
    pub published_by: String,
    pub generator: String,
    pub source: SourceFile,
    pub snapshot_seq: u64,
    pub snapshot_body_sha256: String,
    pub rule_set_version: String,
    pub rule_set_digest: String,
    /// 主数据没配置时为 `None`。
    pub master_data_revision: Option<u64>,
    pub gates: Gates,
    /// 图里没有同步记录时为 `None`。
    pub collab: Option<CollabRecord>,
    pub contents: Vec<Content>,
}

/// [`write_package`] 要的东西，都由命令层备好：快照已经存了（有序号），清单是从这份快照算的，
/// 门禁是用这份检查报告算的。
#[derive(Debug, Clone, Copy)]
pub struct PackageInput<'a> {
    pub drawing_id: &'a str,
    pub revision: u32,
    pub project: Option<&'a ProjectRef>,
    /// 出包的时间，Unix 秒。由调用方给，这里不读时钟。
    pub published_at: i64,
    pub published_by: &'a str,
    pub generator: &'a str,
    /// 发布时的源文件，原样拷进包里。
    pub source: &'a Path,
    pub snapshot: &'a Snapshot,
    pub bom: &'a Bom,
    /// 包里的连接符配对状态从这里取。
    pub index: &'a ProjectIndex,
    /// 主数据没配置时为 `None`。
    pub master_data: Option<&'a MasterData>,
    pub report: &'a CheckReport,
    pub gates: &'a Gates,
    /// 图里没有同步记录时为 `None`。
    pub collab: Option<&'a CollabRecord>,
}

/// `topology.json`：这张图的连通图，加上各个页间连接符的配对状态（§3.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Topology {
    pub version: u32,
    pub drawing_id: String,
    pub nodes: Vec<String>,
    pub edges: Vec<[String; 2]>,
    pub connectors: Vec<TopologyConnector>,
}

/// 一个页间连接符。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopologyConnector {
    pub kind: String,
    pub line: String,
    pub status: Pairing,
    pub peer_drawing: Option<String>,
    pub upstream: Option<String>,
    pub downstream: Option<String>,
}

/// 连接符在项目索引里配没配成对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pairing {
    Paired,
    OneSided,
    TargetMissing,
    Ambiguous,
    Unpaired,
}

impl From<LinkStatus> for Pairing {
    fn from(status: LinkStatus) -> Self {
        match status {
            LinkStatus::OneSided => Self::OneSided,
            LinkStatus::TargetMissing => Self::TargetMissing,
            LinkStatus::Unpaired => Self::Unpaired,
            LinkStatus::Ambiguous => Self::Ambiguous,
        }
    }
}

/// 由快照和项目索引做 `topology.json`：点和边就是快照里的连通图；连接符按快照摘要里的顺序，
/// 状态按句柄在索引里找这张图的那一个：在配好的对里是 `paired`，在没配成对的里取那里的
/// 状态，都找不到算 `unpaired`。
#[must_use]
pub fn topology(snapshot: &Snapshot, index: &ProjectIndex) -> Topology {
    let drawing_id = snapshot.drawing_id.as_str();
    let mut status: HashMap<&str, Pairing> = HashMap::new();
    for link in &index.links {
        for end in [&link.from, &link.to] {
            if end.drawing_id == drawing_id {
                status.insert(end.handle.as_str(), Pairing::Paired);
            }
        }
    }
    for unresolved in &index.unresolved {
        if unresolved.drawing_id == drawing_id {
            status.insert(unresolved.handle.as_str(), unresolved.status.into());
        }
    }
    let connectors = snapshot
        .summary
        .connectors
        .iter()
        .map(|connector| TopologyConnector {
            kind: connector.kind.clone(),
            line: connector.line.clone(),
            status: status
                .get(connector.handle.as_str())
                .copied()
                .unwrap_or(Pairing::Unpaired),
            peer_drawing: connector.peer_drawing.clone(),
            upstream: connector.upstream.clone(),
            downstream: connector.downstream.clone(),
        })
        .collect();
    Topology {
        version: TOPOLOGY_VERSION,
        drawing_id: drawing_id.to_owned(),
        nodes: snapshot.graph.nodes.clone(),
        edges: snapshot.graph.edges.clone(),
        connectors,
    }
}

/// [`verify_package`] 的结果（§3.5）。应答外面的 `ok` 由接口层加。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    pub package_id: String,
    pub revision: u32,
    /// 目录里的文件数，含清单和多出来的文件。
    pub files: usize,
    /// 清单列了、包里却没有的文件；清单里一个文件都没有的类别也记在这里，记类别名。
    pub missing: Vec<String>,
    /// 字节数或校验和与清单不同，或者读不出来的文件。
    pub mismatched: Vec<String>,
    /// 清单没列的文件，只记不算失败。
    pub extra: Vec<String>,
}

/// 写包、落位、校验的错。错误码见 [`PublishError::code`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// 门禁没过。
    Blocked,
    /// 快照还没存，没有序号。
    SnapshotUnsaved,
    /// 临时目录里缺一张出图，或者读不出来。
    PlotMissing(PathBuf),
    /// 读写文件失败：`step` 是 `write`（写包）或 `commit`（落位）。
    Io {
        step: &'static str,
        path: PathBuf,
        reason: String,
    },
    /// 落位前的自检没过，带着自检报的错。
    SelfCheck(Box<PublishError>),
    /// 要落位的目录已经有了。
    TargetExists(PathBuf),
    /// 要校验的目录不存在，或者里面没有发布清单。
    NotFound(PathBuf),
    /// 发布清单读不出来，或者不是这个程序认得的。
    ManifestInvalid(String),
    Incomplete(Box<VerifyReport>),
    Corrupt(Box<VerifyReport>),
}

impl PublishError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Blocked => PUBLISH_BLOCKED,
            Self::SnapshotUnsaved | Self::PlotMissing(_) | Self::Io { .. } | Self::SelfCheck(_) => {
                PUBLISH_FAILED
            }
            Self::TargetExists(_) => PUBLISH_TARGET_EXISTS,
            Self::NotFound(_) => PACKAGE_NOT_FOUND,
            Self::ManifestInvalid(_) | Self::Corrupt(_) => PACKAGE_CORRUPT,
            Self::Incomplete(_) => PACKAGE_INCOMPLETE,
        }
    }

    /// [`PUBLISH_FAILED`] 时是哪一步（§3.6 的 `step`）。
    #[must_use]
    pub fn step(&self) -> Option<&'static str> {
        match self {
            Self::SnapshotUnsaved => Some("snapshot"),
            Self::PlotMissing(_) => Some("plot"),
            Self::Io { step, .. } => Some(*step),
            Self::SelfCheck(_) => Some("verify"),
            _ => None,
        }
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Self::Blocked => write!(f, "{code}: 门禁没过，不出包"),
            Self::SnapshotUnsaved => write!(f, "{code}: 快照还没存，没有序号"),
            Self::PlotMissing(path) => {
                write!(f, "{code}: 临时目录里没有出图 {}", path.display())
            }
            Self::Io { path, reason, .. } => write!(f, "{code}: {}：{reason}", path.display()),
            Self::SelfCheck(error) => write!(f, "{code}: 落位前自检没过（{error}）"),
            Self::TargetExists(path) => {
                write!(f, "{code}: {} 已经有了，已发布的包不覆盖", path.display())
            }
            Self::NotFound(path) => write!(
                f,
                "{code}: {} 不存在，或者里面没有 {MANIFEST_FILE}",
                path.display()
            ),
            Self::ManifestInvalid(detail) => write!(f, "{code}: 发布清单不对：{detail}"),
            Self::Incomplete(report) => write!(f, "{code}: 缺 {}", report.missing.join("、")),
            Self::Corrupt(report) => write!(
                f,
                "{code}: 与清单记的字节数或校验和不同：{}",
                report.mismatched.join("、")
            ),
        }
    }
}

impl std::error::Error for PublishError {}

/// 往临时目录 `staging` 里写一个包（§3.1、§3.2）。两张出图要先由调用方按 [`plot_files`]
/// 的名字写好；这里拷源文件，写材料清单、报表、主数据、检查报告，最后写发布清单。
///
/// 门禁没过、快照没存、出图或源文件读不到时什么都不写；写到一半失败的，临时目录由调用方删。
pub fn write_package(staging: &Path, input: &PackageInput) -> Result<Manifest, PublishError> {
    if !input.gates.passed() {
        return Err(PublishError::Blocked);
    }
    let snapshot_seq = input.snapshot.seq.ok_or(PublishError::SnapshotUnsaved)?;
    let mut plots = Vec::with_capacity(2);
    for name in plot_files(input.drawing_id) {
        let path = staging.join(&name);
        match fs::read(&path) {
            Ok(bytes) => plots.push(listing(DRAWING, &name, &bytes)),
            Err(_) => return Err(PublishError::PlotMissing(path)),
        }
    }
    let source =
        fs::read(input.source).map_err(|error| io_failure("write", input.source, &error))?;
    let stem = file_stem(input.drawing_id);
    let copied = match input.source.extension() {
        Some(extension) => format!("{stem}.{}", extension.to_string_lossy()),
        None => stem,
    };
    let drawing = put(staging, DRAWING, &copied, &source)?;
    let source = SourceFile {
        file: input
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        bytes: drawing.bytes,
        sha256: drawing.sha256.clone(),
    };
    let mut contents = vec![drawing];
    contents.extend(plots);
    let rows = bom::rows(input.bom, None).expect("默认的列总取得到");
    for (kind, name, bytes) in [
        (BILL, BOM_CSV, bom::csv_bytes(&rows)),
        (BILL, BOM_JSON, pretty_ok(input.bom)),
        (
            REPORTS,
            PIPE_SUMMARY_CSV,
            bom::csv_bytes(&bom::pipe_summary_rows(input.bom)),
        ),
        (
            REPORTS,
            TOPOLOGY_JSON,
            pretty(&topology(input.snapshot, input.index)),
        ),
        (REPORTS, SNAPSHOT_JSON, input.snapshot.to_bytes()),
        (
            MASTER_DATA,
            MASTER_DATA_JSON,
            master_data_bytes(input.master_data),
        ),
        (CHECK_REPORT, DRC_REPORT_JSON, pretty_ok(input.report)),
    ] {
        contents.push(put(staging, kind, name, &bytes)?);
    }
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        kind: MANIFEST_KIND,
        package_id: package_name(input.drawing_id, input.revision),
        project: input.project.cloned(),
        drawing_id: input.drawing_id.to_owned(),
        revision: input.revision,
        published_at: input.published_at,
        published_by: input.published_by.to_owned(),
        generator: input.generator.to_owned(),
        source,
        snapshot_seq,
        snapshot_body_sha256: input.snapshot.body_sha256.clone(),
        rule_set_version: input.report.rule_set_version.clone(),
        rule_set_digest: input.report.rule_set_digest.clone(),
        master_data_revision: input.master_data.map(|data| data.revision),
        gates: input.gates.clone(),
        collab: input.collab.cloned(),
        contents,
    };
    let path = staging.join(MANIFEST_FILE);
    fs::write(&path, pretty(&manifest)).map_err(|error| io_failure("write", &path, &error))?;
    Ok(manifest)
}

/// 把写好的临时目录 `staging` 落位成 `final_dir`（§3.1、§4）：目标已存在报
/// [`PUBLISH_TARGET_EXISTS`]；先用 [`verify_package`] 自检，清单里的包名还要与目标目录同名，
/// 过了才改名；改名后把包里的文件设成只读。失败时临时目录原样留着，由调用方删。
pub fn commit(staging: &Path, final_dir: &Path) -> Result<(), PublishError> {
    if fs::symlink_metadata(final_dir).is_ok() {
        return Err(PublishError::TargetExists(final_dir.to_owned()));
    }
    let report =
        verify_package(staging).map_err(|error| PublishError::SelfCheck(Box::new(error)))?;
    let name = final_dir.file_name().map(|name| name.to_string_lossy());
    if name.as_deref() != Some(report.package_id.as_str()) {
        let detail = format!(
            "清单里的包名是 {}，要落位的目录是 {}",
            report.package_id,
            final_dir.display()
        );
        return Err(PublishError::SelfCheck(Box::new(
            PublishError::ManifestInvalid(detail),
        )));
    }
    if let Err(error) = fs::rename(staging, final_dir) {
        return Err(if fs::symlink_metadata(final_dir).is_ok() {
            PublishError::TargetExists(final_dir.to_owned())
        } else {
            io_failure("commit", final_dir, &error)
        });
    }
    make_read_only(final_dir);
    Ok(())
}

/// 校验一个包（§3.5）：清单列的每个文件都在、字节数和校验和都对，五类都有。缺了的报
/// [`PACKAGE_INCOMPLETE`]，否则有不对的报 [`PACKAGE_CORRUPT`]；清单没列的文件只记在 `extra`。
/// 清单自己没有校验和，改动它只有在读不出来或对不上文件时才看得出。
pub fn verify_package(dir: &Path) -> Result<VerifyReport, PublishError> {
    let manifest = dir.join(MANIFEST_FILE);
    if !manifest.is_file() {
        return Err(PublishError::NotFound(dir.to_owned()));
    }
    let listed = read_manifest(&manifest)?;
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    for content in &listed.contents {
        let path = dir.join(&content.path);
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() as u64 != content.bytes
                    || !sha256(&bytes).eq_ignore_ascii_case(&content.sha256)
                {
                    mismatched.push(content.path.clone());
                }
            }
            Err(_) if !path.is_file() => missing.push(content.path.clone()),
            Err(_) => mismatched.push(content.path.clone()),
        }
    }
    for kind in KINDS {
        if !listed.contents.iter().any(|content| content.kind == kind) {
            missing.push(kind.to_owned());
        }
    }
    let named: HashSet<&str> = listed
        .contents
        .iter()
        .map(|content| content.path.as_str())
        .collect();
    let entries = fs::read_dir(dir).map_err(|_| PublishError::NotFound(dir.to_owned()))?;
    let mut files = 0;
    let mut extra = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() {
            files += 1;
        }
        if name != MANIFEST_FILE && !named.contains(name.as_str()) {
            extra.push(name);
        }
    }
    extra.sort();
    let report = VerifyReport {
        package_id: listed.package_id,
        revision: listed.revision,
        files,
        missing,
        mismatched,
        extra,
    };
    if !report.missing.is_empty() {
        Err(PublishError::Incomplete(Box::new(report)))
    } else if !report.mismatched.is_empty() {
        Err(PublishError::Corrupt(Box::new(report)))
    } else {
        Ok(report)
    }
}

/// 校验时从发布清单里读的几项。
#[derive(Deserialize)]
struct Listed {
    version: u32,
    kind: String,
    package_id: String,
    revision: u32,
    contents: Vec<Content>,
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// 读发布清单：要解析得了，`kind`、`version` 是这个程序写的，列的每一项都是包里的一个
/// 文件名（不带目录，不是清单自己）。
fn read_manifest(path: &Path) -> Result<Listed, PublishError> {
    let bytes = fs::read(path).map_err(|error| PublishError::ManifestInvalid(error.to_string()))?;
    let text = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
    let listed: Listed = serde_json::from_slice(text)
        .map_err(|error| PublishError::ManifestInvalid(error.to_string()))?;
    if listed.kind != MANIFEST_KIND {
        return Err(PublishError::ManifestInvalid(format!(
            "`kind` 是 `{}`，不是发布清单",
            listed.kind
        )));
    }
    if listed.version != MANIFEST_VERSION {
        return Err(PublishError::ManifestInvalid(format!(
            "格式版本是 {}，这里只认 {MANIFEST_VERSION}",
            listed.version
        )));
    }
    if let Some(content) = listed
        .contents
        .iter()
        .find(|content| !plain_name(&content.path))
    {
        return Err(PublishError::ManifestInvalid(format!(
            "`contents` 里的 `{}` 不是包里的文件名",
            content.path
        )));
    }
    Ok(listed)
}

/// 包是平铺的：清单列的只能是一段文件名，不带目录，也不是清单自己。
fn plain_name(path: &str) -> bool {
    let mut components = Path::new(path).components();
    let single = matches!(components.next(), Some(Component::Normal(name)) if name == path);
    single && components.next().is_none() && path != MANIFEST_FILE
}

/// 把包里的文件设成只读。设不成不算失败：包已经落位了。
fn make_read_only(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(true);
            let _ = fs::set_permissions(&path, permissions);
        }
    }
}

/// 把 `bytes` 写成写包目录里的 `name`，返回它在清单里的那一项。
fn put(staging: &Path, kind: &str, name: &str, bytes: &[u8]) -> Result<Content, PublishError> {
    let path = staging.join(name);
    fs::write(&path, bytes).map_err(|error| io_failure("write", &path, &error))?;
    Ok(listing(kind, name, bytes))
}

fn listing(kind: &str, path: &str, bytes: &[u8]) -> Content {
    Content {
        kind: kind.to_owned(),
        path: path.to_owned(),
        bytes: bytes.len() as u64,
        sha256: sha256(bytes),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 缩进两格的 JSON，末尾带换行。
fn pretty<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("能写成 JSON");
    bytes.push(b'\n');
    bytes
}

/// 与导出文件同一写法：外面加上 `ok: true`。
fn pretty_ok<T: Serialize>(body: &T) -> Vec<u8> {
    #[derive(Serialize)]
    struct WithOk<'a, T> {
        ok: bool,
        #[serde(flatten)]
        body: &'a T,
    }
    pretty(&WithOk { ok: true, body })
}

/// `master-data.json`：生效的主数据全文；没配置时只写格式版本和 `configured: false`（§3.1）。
fn master_data_bytes(master: Option<&MasterData>) -> Vec<u8> {
    match master {
        Some(data) => pretty(&data.to_json()),
        None => pretty(&json!({ "version": master_data::VERSION, "configured": false })),
    }
}

fn io_failure(step: &'static str, path: &Path, error: &io::Error) -> PublishError {
    PublishError::Io {
        step,
        path: path.to_owned(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use super::*;
    use crate::pid::bom::{BomOptions, BomScope, BomSource};
    use crate::pid::drc::{Scope, ScopeKind, Summary, Trigger, REPORT_VERSION};
    use crate::pid::master_data::{self, MasterDataError, Source};
    use crate::pid::project_index::{
        self, build_index, Joint, Link, LinkEnd, LinkFamily, PeerKind, SummaryConnector,
        SummarySegment, SummarySource, Unresolved, INDEX_VERSION, SUMMARY_VERSION,
    };
    use crate::pid::snapshot::{self, SnapshotMeta};
    use CollabVerdict::{NotApplicable, Pass, Refused};

    /// 测试用的临时目录，测完删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：计数器把同一纳秒里建的两个目录分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-publish-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            fs::create_dir(&path).expect("scratch directory");
            Self(path)
        }

        /// 在临时目录里建一个叫 `name` 的子目录。
        fn dir(&self, name: &str) {
            fs::create_dir(self.0.join(name)).expect("能建目录");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 列出的各个包的修订号和包名。
    fn listed(dir: &Path, drawing_id: &str) -> Vec<(u32, String)> {
        list_packages(dir, drawing_id)
            .into_iter()
            .map(|package| {
                assert_eq!(package.path, dir.join(&package.package_id));
                (package.revision, package.package_id)
            })
            .collect()
    }

    /// 一个有自己编号的对象，别的字段取空。
    fn object(
        kind: ObjectKind,
        type_code: &str,
        tag: &str,
        at: [f64; 2],
        handle: &str,
    ) -> SummaryObject {
        SummaryObject {
            kind,
            type_code: type_code.to_owned(),
            type_label: String::new(),
            tag: tag.to_owned(),
            own_tag: true,
            tag_source: String::new(),
            tag_status: String::new(),
            symbol: None,
            symbol_known: false,
            props: BTreeMap::new(),
            overrides: Vec::new(),
            owner: None,
            layer: String::new(),
            at,
            bbox: [at[0], at[1], at[0], at[1]],
            handle: handle.to_owned(),
        }
    }

    /// 同一个对象，位号不是它自己的。
    fn borrowed(mut object: SummaryObject) -> SummaryObject {
        object.own_tag = false;
        object
    }

    fn drawing(drawing_id: &str, objects: Vec<SummaryObject>) -> DrawingSummary {
        DrawingSummary {
            version: SUMMARY_VERSION,
            drawing_id: drawing_id.to_owned(),
            file: Some(format!("{drawing_id}.dwg")),
            source: SummarySource::File,
            fingerprint: None,
            pbs: None,
            length_unit: "mm".to_owned(),
            objects,
            segments: Vec::new(),
            signal_ends: Vec::new(),
            connectors: Vec::new(),
            ports: Vec::new(),
        }
    }

    /// 一个重复位号：类型编码、位号和各个出现处的（图号，句柄）。
    type Found<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);

    /// 各个重复位号的类型编码、位号和出现处。
    fn found(duplicates: &[Duplicate]) -> Vec<Found<'_>> {
        duplicates
            .iter()
            .map(|duplicate| {
                let places = duplicate
                    .places
                    .iter()
                    .map(|place| (place.drawing_id.as_str(), place.handle.as_str()))
                    .collect();
                (duplicate.type_code.as_str(), duplicate.tag.as_str(), places)
            })
            .collect()
    }

    /// 计数是这几样的检查报告，别的取空。
    fn report(errors: usize, warnings: usize, infos: usize, waived: usize) -> CheckReport {
        CheckReport {
            version: REPORT_VERSION,
            rule_set_version: "1".to_owned(),
            rule_set_digest: String::new(),
            checked_at: 1_794_800_000,
            trigger: Trigger::BeforePublish,
            scope: Scope {
                kind: ScopeKind::Drawing,
                drawing_id: Some("PID-1101".to_owned()),
                pbs: None,
                cross_drawing: true,
            },
            elapsed_ms: 0,
            summary: Summary {
                errors,
                warnings,
                infos,
                waived,
                ..Summary::default()
            },
            blocked: errors > 0,
            results: Vec::new(),
            notices: Vec::new(),
            stale_waivers: Vec::new(),
        }
    }

    fn configured() -> Resolved {
        Resolved::Configured {
            data: master_data::defaults("GB/T 6567"),
            source: Source::Drawing,
        }
    }

    fn unreadable() -> Resolved {
        Resolved::Unreadable {
            source: Source::Project(PathBuf::from("pid-project.json")),
            error: MasterDataError::Io {
                path: "pid-project.json".to_owned(),
                reason: "拒绝访问".to_owned(),
            },
        }
    }

    /// 包名是「图号-r三位修订号」，图号里不能做文件名的字符换成 `_`（与快照目录同一规则）；
    /// 修订号按目标目录里这张图已有的包推，列表只列这张图的包（§3.1）。
    #[test]
    fn package_names_and_revisions_come_from_the_target_directory() {
        assert_eq!(package_name("PID-1101", 3), "PID-1101-r003");
        assert_eq!(package_name("PID-1101", 1234), "PID-1101-r1234");
        assert_eq!(package_name("PID/11:01*", 2), "PID_11_01_-r002");
        assert_eq!(package_name(" PID-1101.", 1), "_PID-1101_-r001");
        assert_eq!(package_name("CON", 1), "_CON-r001");

        let scratch = Scratch::new();
        let dir = scratch.0.as_path();
        assert_eq!(next_revision(dir, "PID-1101"), 1, "空目录");
        assert_eq!(
            next_revision(&dir.join("没有"), "PID-1101"),
            1,
            "目录不存在"
        );
        assert!(list_packages(&dir.join("没有"), "PID-1101").is_empty());

        for name in [
            "PID-1101-r003",
            "PID-1101-r001",
            "PID-1102-r007",
            "PID-1101-r001-r002",
            "PID-1101-r0005",
            "PID-1101-r000",
            "PID-1101-r+06",
            "PID-1101-r08",
            ".PID-1101-r009.tmp",
            "PID_11_01_-r004",
        ] {
            scratch.dir(name);
        }
        fs::write(dir.join("PID-1101-r010"), b"").expect("能写");

        assert_eq!(
            listed(dir, "PID-1101"),
            [
                (1, "PID-1101-r001".to_owned()),
                (3, "PID-1101-r003".to_owned())
            ]
        );
        assert_eq!(next_revision(dir, "PID-1101"), 4);
        assert_eq!(listed(dir, "PID-1102"), [(7, "PID-1102-r007".to_owned())]);
        assert_eq!(next_revision(dir, "PID-1102"), 8);
        assert_eq!(
            listed(dir, "PID-1101-r001"),
            [(2, "PID-1101-r001-r002".to_owned())]
        );
        assert_eq!(
            listed(dir, "PID/11:01*"),
            [(4, "PID_11_01_-r004".to_owned())]
        );
        assert_eq!(next_revision(dir, "PID-1103"), 1);
    }

    /// 重复位号的数法（§3.4 第二项）：本图内、跨图各算；不同类型同位号不算；没有自己编号的
    /// 不算；管线、管路、信号线不算。别的图上的出现处从索引和别的图的摘要里都找，同一处只记一次；
    /// 索引里的本图是旧的，不看。
    #[test]
    fn duplicate_tags_count_within_and_across_drawings() {
        use ObjectKind::{
            Equipment, Fitting, Instrument, Other, PipeLine, PipeRun, SignalLine, Valve,
        };

        let current = drawing(
            "PID-1101",
            vec![
                object(Equipment, "EQUIPMENT", "P-1001", [100.0, 0.0], "A2"),
                object(Equipment, "EQUIPMENT", "P-1001", [0.0, 0.0], "A1"),
                object(Valve, "VALVE_GATE", "V-1", [10.0, 0.0], "A3"),
                object(Instrument, "INSTRUMENT", "FT-7", [20.0, 0.0], "A4"),
                object(Other, "SAMPLE_POINT", "SP-1", [30.0, 0.0], "A5"),
                object(Other, "SAMPLE_POINT", "SP-1", [30.0, 9.0], "A6"),
                object(Equipment, "EQUIPMENT", " E-9", [0.0, 50.0], "A7"),
                object(Equipment, "EQUIPMENT", "E-9 ", [100.0, 50.0], "A8"),
                // 不同类型同位号不算：阀门 X-1 不并进设备 X-1，设备 T-1 与 1102 的阀门 T-1 也不算。
                object(Equipment, "EQUIPMENT", "X-1", [40.0, 0.0], "A9"),
                object(Valve, "VALVE_GATE", "X-1", [41.0, 0.0], "A10"),
                object(Equipment, "EQUIPMENT", "T-1", [42.0, 0.0], "A11"),
                // 没有自己编号的。
                object(Fitting, "FITTING", "L-100", [50.0, 0.0], "A12"),
                object(Fitting, "FITTING", "L-100", [51.0, 0.0], "A13"),
                borrowed(object(Other, "SAMPLE_POINT", "SP-2", [52.0, 0.0], "A14")),
                borrowed(object(Other, "SAMPLE_POINT", "SP-2", [53.0, 0.0], "A15")),
                // 管线、管路、信号线。
                object(PipeLine, "PIPE_LINE", "L-100", [60.0, 0.0], "A16"),
                object(PipeLine, "PIPE_LINE", "L-100", [61.0, 0.0], "A17"),
                object(PipeRun, "PIPE_RUN", "L-100-1", [62.0, 0.0], "A18"),
                object(PipeRun, "PIPE_RUN", "L-100-1", [63.0, 0.0], "A19"),
                object(SignalLine, "SIGNAL_LINE", "S-1", [64.0, 0.0], "A20"),
                object(SignalLine, "SIGNAL_LINE", "S-1", [65.0, 0.0], "A21"),
                // 空位号。
                object(Equipment, "EQUIPMENT", "", [70.0, 0.0], "A22"),
                object(Equipment, "EQUIPMENT", "  ", [71.0, 0.0], "A23"),
            ],
        );
        // 建索引时的本图：之后删掉了 K-1，V-1 也换了一个。
        let stale = drawing(
            "PID-1101",
            vec![
                object(Valve, "VALVE_GATE", "V-1", [10.0, 0.0], "OLD1"),
                object(Equipment, "EQUIPMENT", "K-1", [0.0, 0.0], "OLD2"),
                object(Instrument, "INSTRUMENT", "FT-7", [20.0, 0.0], "A4"),
                object(Equipment, "EQUIPMENT", "X-1", [40.0, 0.0], "A9"),
            ],
        );
        let pid1102 = drawing(
            "PID-1102",
            vec![
                object(Valve, "VALVE_GATE", "V-1", [5.0, 5.0], "B1"),
                object(Valve, "VALVE_GATE", "T-1", [6.0, 5.0], "B2"),
                object(Equipment, "EQUIPMENT", "K-1", [7.0, 5.0], "B3"),
                object(Equipment, "EQUIPMENT", "X-1", [8.0, 5.0], "B4"),
                borrowed(object(Instrument, "INSTRUMENT", "FT-8", [9.0, 5.0], "B5")),
            ],
        );
        let pid1103 = drawing(
            "PID-1103",
            vec![object(Instrument, "INSTRUMENT", "FT-7", [1.0, 1.0], "C1")],
        );
        let index = build_index(Path::new("proj"), &[stale, pid1102.clone(), pid1103], &[])
            .expect("能建索引");

        let duplicates = duplicate_tags(&current, &index, std::slice::from_ref(&pid1102));
        assert_eq!(
            found(&duplicates),
            [
                (
                    "EQUIPMENT",
                    "E-9",
                    vec![("PID-1101", "A7"), ("PID-1101", "A8")]
                ),
                (
                    "EQUIPMENT",
                    "P-1001",
                    vec![("PID-1101", "A1"), ("PID-1101", "A2")]
                ),
                (
                    "EQUIPMENT",
                    "X-1",
                    vec![("PID-1101", "A9"), ("PID-1102", "B4")]
                ),
                (
                    "INSTRUMENT",
                    "FT-7",
                    vec![("PID-1101", "A4"), ("PID-1103", "C1")]
                ),
                (
                    "SAMPLE_POINT",
                    "SP-1",
                    vec![("PID-1101", "A5"), ("PID-1101", "A6")]
                ),
                (
                    "VALVE_GATE",
                    "V-1",
                    vec![("PID-1101", "A3"), ("PID-1102", "B1")]
                ),
            ]
        );
        assert_eq!(duplicates[1].places[0].at, [0.0, 0.0]);

        // 没有别的图的摘要时只靠索引，结果相同；索引里没有别的图时只靠摘要。
        assert_eq!(duplicate_tags(&current, &index, &[]), duplicates);
        let alone =
            build_index(Path::new("proj"), std::slice::from_ref(&current), &[]).expect("能建索引");
        let only_1102 = duplicate_tags(&current, &alone, &[pid1102]);
        assert_eq!(
            found(&only_1102)
                .into_iter()
                .map(|(type_code, tag, _)| (type_code, tag))
                .collect::<Vec<_>>(),
            [
                ("EQUIPMENT", "E-9"),
                ("EQUIPMENT", "P-1001"),
                ("EQUIPMENT", "X-1"),
                ("SAMPLE_POINT", "SP-1"),
                ("VALVE_GATE", "V-1"),
            ]
        );
        // 只有本图、没有重的。
        let clean = drawing(
            "PID-1101",
            vec![object(Equipment, "EQUIPMENT", "P-1001", [0.0, 0.0], "A1")],
        );
        assert!(duplicate_tags(&clean, &alone, &[]).is_empty());
    }

    /// 四项门禁的组合（§3.4）：全过时放行；未豁免的错误、重复位号、主数据读不出来、索引不完整
    /// 任一项都拦；警告、提示、已豁免的不拦；主数据没配置不单独拦。JSON 的字段和顺序照 §5.3。
    #[test]
    fn gates_block_on_any_of_the_four() {
        let open = gates(&report(0, 4, 2, 1), 0, &configured(), true);
        assert!(open.passed());
        assert_eq!(
            serde_json::to_string(&open).expect("能写成 JSON"),
            r#"{"drc_errors":0,"drc_warnings":4,"drc_hints":2,"waived":1,"duplicate_tags":0,"master_data":"configured","index_complete":true,"result":"放行"}"#
        );

        let not_configured = gates(&report(0, 0, 0, 0), 0, &Resolved::NotConfigured, true);
        assert!(not_configured.passed());
        assert_eq!(not_configured.master_data, MasterDataState::NotConfigured);
        assert_eq!(
            serde_json::to_value(not_configured.master_data).expect("能写成 JSON"),
            "not_configured"
        );

        let blocked = [
            gates(&report(1, 4, 2, 1), 0, &configured(), true),
            gates(&report(0, 4, 2, 1), 2, &configured(), true),
            gates(&report(0, 4, 2, 1), 0, &unreadable(), true),
            gates(&report(0, 4, 2, 1), 0, &configured(), false),
        ];
        for gates in &blocked {
            assert!(!gates.passed(), "{gates:?}");
            assert_eq!(gates.result, PUBLISH_BLOCKED);
            assert_eq!(
                (gates.drc_warnings, gates.drc_hints, gates.waived),
                (4, 2, 1)
            );
        }
        assert_eq!(blocked[0].drc_errors, 1);
        assert_eq!(blocked[1].duplicate_tags, 2);
        assert_eq!(blocked[2].master_data, MasterDataState::Unreadable);
        assert_eq!(
            serde_json::to_value(blocked[2].master_data).expect("能写成 JSON"),
            "unreadable"
        );
        assert!(!blocked[3].index_complete);

        let all_bad = gates(&report(3, 0, 0, 0), 1, &unreadable(), false);
        assert_eq!(all_bad.result, PUBLISH_BLOCKED);
    }

    /// 协同前提（离线规格 §5.7）：没有同步记录不查；队列不空、不一致、没连着或没补齐各有各的码；
    /// 「允许未对账」只放过最后一种，这时 `reconciled` 为假。
    #[test]
    fn collab_preconditions_follow_the_offline_rules() {
        let synced = CollabInput {
            has_record: true,
            caught_up: true,
            ..CollabInput::default()
        };
        let cases = [
            (CollabInput::default(), NotApplicable),
            (
                CollabInput {
                    has_record: false,
                    queued: 3,
                    diverged: true,
                    ..CollabInput::default()
                },
                NotApplicable,
            ),
            (synced, Pass { reconciled: true }),
            (
                CollabInput {
                    allow_offline: true,
                    ..synced
                },
                Pass { reconciled: true },
            ),
            (
                CollabInput {
                    queued: 2,
                    ..synced
                },
                Refused(COLLAB_UNSYNCED),
            ),
            (
                CollabInput {
                    queued: 2,
                    allow_offline: true,
                    ..synced
                },
                Refused(COLLAB_UNSYNCED),
            ),
            (
                CollabInput {
                    diverged: true,
                    ..synced
                },
                Refused(COLLAB_DIVERGED),
            ),
            (
                CollabInput {
                    diverged: true,
                    allow_offline: true,
                    caught_up: false,
                    ..synced
                },
                Refused(COLLAB_DIVERGED),
            ),
            (
                CollabInput {
                    queued: 1,
                    diverged: true,
                    caught_up: false,
                    ..synced
                },
                Refused(COLLAB_UNSYNCED),
            ),
            (
                CollabInput {
                    caught_up: false,
                    ..synced
                },
                Refused(COLLAB_OFFLINE),
            ),
            (
                CollabInput {
                    caught_up: false,
                    allow_offline: true,
                    ..synced
                },
                Pass { reconciled: false },
            ),
        ];
        for (input, verdict) in cases {
            assert_eq!(collab_preconditions(input), verdict, "{input:?}");
        }
    }

    // ── 写包、落位、校验 ──

    /// 源文件和两张出图的占位内容。
    const SOURCE: &[u8] = b"AC1032\x00\x01\x02\xff not a real drawing";
    const PDF: &[u8] = b"%PDF-1.7 placeholder";
    const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";

    /// 包里清单以外的十个文件，按清单里的顺序，带类别。
    const LISTED: [(&str, &str); 10] = [
        ("图纸文件", "PID-1101.dwg"),
        ("图纸文件", "PID-1101.pdf"),
        ("图纸文件", "PID-1101.svg"),
        ("材料清单", "bom.csv"),
        ("材料清单", "bom.json"),
        ("报表", "pipe-summary.csv"),
        ("报表", "topology.json"),
        ("报表", "snapshot.json"),
        ("项目主数据快照", "master-data.json"),
        ("校验结果报告", "drc-report.json"),
    ];

    /// 写好的包里的十一个文件，按名字排。
    const WRITTEN: [&str; 11] = [
        "PID-1101.dwg",
        "PID-1101.pdf",
        "PID-1101.svg",
        "bom.csv",
        "bom.json",
        "drc-report.json",
        "manifest.json",
        "master-data.json",
        "pipe-summary.csv",
        "snapshot.json",
        "topology.json",
    ];

    /// 一个页间连接符；记了对端图的算协同服务器配的对。
    fn connector(
        kind: &str,
        line: &str,
        handle: &str,
        peer: Option<&str>,
        upstream: Option<&str>,
        downstream: Option<&str>,
    ) -> SummaryConnector {
        SummaryConnector {
            kind: kind.to_owned(),
            line: line.to_owned(),
            at: [0.0, 0.0],
            radius: 5.0,
            peer_kind: if peer.is_some() {
                PeerKind::Server
            } else {
                PeerKind::None
            },
            peer_drawing: peer.map(str::to_owned),
            upstream: upstream.map(str::to_owned),
            downstream: downstream.map(str::to_owned),
            handle: handle.to_owned(),
        }
    }

    /// 要发布的图：泵 P-1001 的出口经管线 L-1001 的一段接到连接符 C1；另有 C2、C3、C4 三个
    /// 连接符。
    fn sheet() -> DrawingSummary {
        let mut sheet = drawing(
            "PID-1101",
            vec![
                object(ObjectKind::Equipment, "PUMP", "P-1001", [0.0, 0.0], "A1"),
                object(ObjectKind::Valve, "VALVE_GATE", "V-1001", [50.0, 0.0], "A2"),
                object(
                    ObjectKind::PipeLine,
                    "PIPE_LINE",
                    "L-1001",
                    [50.0, 0.0],
                    "A3",
                ),
            ],
        );
        sheet.segments.push(SummarySegment {
            line: "L-1001".to_owned(),
            run: None,
            a: [0.0, 0.0],
            b: [100.0, 0.0],
            length: 100.0,
            joint_a: Some(Joint::Port {
                type_code: "PUMP".to_owned(),
                tag: "P-1001".to_owned(),
                port: "out1".to_owned(),
            }),
            joint_b: Some(Joint::Connector {
                kind: "pipe_out".to_owned(),
                line: "L-1001".to_owned(),
            }),
            handle: "A4".to_owned(),
        });
        sheet.connectors = vec![
            connector(
                "pipe_out",
                "L-1001",
                "C1",
                Some("PID-1102"),
                Some("P-1001"),
                None,
            ),
            connector(
                "pipe_in",
                "L-2001",
                "C2",
                Some("PID-9999"),
                None,
                Some("T-2001"),
            ),
            connector("pipe_in", "L-3001", "C3", None, None, None),
            connector(
                "pipe_in",
                "L-4001",
                "C4",
                Some("PID-1103"),
                None,
                Some("E-4001"),
            ),
        ];
        sheet
    }

    fn end(drawing_id: &str, line: &str, handle: &str) -> LinkEnd {
        LinkEnd {
            drawing_id: drawing_id.to_owned(),
            line: line.to_owned(),
            at: [0.0, 0.0],
            handle: handle.to_owned(),
        }
    }

    fn unresolved(drawing_id: &str, handle: &str, status: LinkStatus) -> Unresolved {
        Unresolved {
            drawing_id: drawing_id.to_owned(),
            kind: "pipe_in".to_owned(),
            line: String::new(),
            at: [0.0, 0.0],
            handle: handle.to_owned(),
            status,
            target: None,
        }
    }

    /// 项目索引：C1 与 PID-1102 配成了对（本图是流出侧），C4 与 PID-1103 配成了对（本图是流入侧），
    /// C2 指的图不在项目里；PID-1102 上也有个句柄是 C3 的，与本图的 C3 无关。
    fn index() -> ProjectIndex {
        ProjectIndex {
            version: INDEX_VERSION,
            project_dir: "D:/projects/CDU-01".to_owned(),
            generated_at: 1_794_800_050,
            complete: true,
            drawings: Vec::new(),
            duplicates: Vec::new(),
            links: vec![
                Link {
                    family: LinkFamily::Pipe,
                    from: end("PID-1101", "L-1001", "C1"),
                    to: end("PID-1102", "L-1001", "D1"),
                },
                Link {
                    family: LinkFamily::Pipe,
                    from: end("PID-1103", "L-4001", "E1"),
                    to: end("PID-1101", "L-4001", "C4"),
                },
            ],
            unresolved: vec![
                unresolved("PID-1101", "C2", LinkStatus::TargetMissing),
                unresolved("PID-1102", "C3", LinkStatus::Ambiguous),
            ],
        }
    }

    /// 出一个包要的东西：临时目录里的源文件，小图存好的快照（序号 7）和从它算的材料清单，
    /// 项目索引，第 3 版主数据，没有错误的检查报告和放行的门禁。
    struct Fixture {
        scratch: Scratch,
        source: PathBuf,
        project: ProjectRef,
        snapshot: Snapshot,
        bom: Bom,
        index: ProjectIndex,
        master: MasterData,
        report: CheckReport,
        gates: Gates,
    }

    impl Fixture {
        fn new() -> Self {
            let scratch = Scratch::new();
            let source = scratch.0.join("PID-1101 送审版.dwg");
            fs::write(&source, SOURCE).expect("能写源文件");
            let summary = sheet();
            let options = BomOptions {
                scope: BomScope::drawing("PID-1101"),
                source: BomSource::Snapshot(7),
                generated_at: 1_794_800_090,
                symbol_labels: BTreeMap::new(),
            };
            let bill = bom::build(std::slice::from_ref(&summary), &options).expect("能出清单");
            let graph = project_index::graph(&summary);
            let meta = SnapshotMeta {
                created_at: 1_794_800_080,
                created_by: "zhangsan".to_owned(),
                reason: "发布".to_owned(),
                generator: "OpenCADStudio 0.9.0".to_owned(),
                dirty: false,
            };
            let mut stored = snapshot::capture(summary, graph, meta).expect("能做快照");
            stored.seq = Some(7);
            let mut master = master_data::defaults("GB/T 6567");
            master.revision = 3;
            let mut checked = report(0, 4, 2, 1);
            checked.rule_set_digest = "a1b2c3d4e5f60718".to_owned();
            let passed = gates(&checked, 0, &configured(), true);
            assert!(passed.passed());
            Self {
                scratch,
                source,
                project: ProjectRef {
                    id: "CDU-01".to_owned(),
                    name: "常减压装置".to_owned(),
                },
                snapshot: stored,
                bom: bill,
                index: index(),
                master,
                report: checked,
                gates: passed,
            }
        }

        /// 出 PID-1101 第 3 版；图里没有同步记录。
        fn input(&self) -> PackageInput<'_> {
            PackageInput {
                drawing_id: "PID-1101",
                revision: 3,
                project: Some(&self.project),
                published_at: 1_794_800_100,
                published_by: "zhangsan",
                generator: "OpenCADStudio 0.9.0",
                source: &self.source,
                snapshot: &self.snapshot,
                bom: &self.bom,
                index: &self.index,
                master_data: Some(&self.master),
                report: &self.report,
                gates: &self.gates,
                collab: None,
            }
        }

        /// 临时目录里叫 `name` 的一个写包目录，两张出图已经照 [`plot_files`] 的名字放好。
        fn staging(&self, name: &str) -> PathBuf {
            let staging = self.scratch.0.join(name);
            fs::create_dir(&staging).expect("能建写包目录");
            fs::write(staging.join("PID-1101.pdf"), PDF).expect("能放出图");
            fs::write(staging.join("PID-1101.svg"), SVG).expect("能放出图");
            staging
        }
    }

    /// 目录里的各项，按名字排。
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("能列目录")
            .map(|entry| {
                let entry = entry.expect("目录项");
                entry.file_name().to_string_lossy().into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// 目录里各个文件的名字和内容。
    fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
        names(dir)
            .into_iter()
            .map(|name| {
                let bytes = fs::read(dir.join(&name)).expect("能读");
                (name, bytes)
            })
            .collect()
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).expect("能读")).expect("是 JSON")
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// 写出的包：十一个文件，清单列十个，顺序、类别、字节数、校验和都对；图纸文件与源文件逐字节
    /// 相同，`source` 记源文件自己的名字；清单的字段和顺序照 §3.2（§3.1、§3.2）。
    #[test]
    fn write_package_writes_eleven_files_and_lists_ten() {
        assert_eq!(plot_files("PID-1101"), ["PID-1101.pdf", "PID-1101.svg"]);
        assert_eq!(
            plot_files("PID/11:01*"),
            ["PID_11_01_.pdf", "PID_11_01_.svg"]
        );

        let fixture = Fixture::new();
        let staging = fixture.staging("stage");
        let manifest = write_package(&staging, &fixture.input()).expect("能写包");
        assert_eq!(names(&staging), WRITTEN);
        let listed: Vec<(&str, &str)> = manifest
            .contents
            .iter()
            .map(|content| (content.kind.as_str(), content.path.as_str()))
            .collect();
        assert_eq!(listed, LISTED);
        for content in &manifest.contents {
            let bytes = fs::read(staging.join(&content.path)).expect("列的文件都在");
            assert_eq!(content.bytes, bytes.len() as u64, "{}", content.path);
            assert_eq!(content.sha256, digest(&bytes), "{}", content.path);
        }
        assert_eq!(
            fs::read(staging.join("PID-1101.dwg")).expect("图纸"),
            SOURCE
        );
        assert_eq!(fs::read(staging.join("PID-1101.pdf")).expect("出图"), PDF);

        let written = read_json(&staging.join(MANIFEST_FILE));
        assert_eq!(
            written,
            serde_json::to_value(&manifest).expect("清单能写成 JSON")
        );
        let keys: Vec<&str> = written
            .as_object()
            .expect("清单是对象")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "version",
                "kind",
                "package_id",
                "project",
                "drawing_id",
                "revision",
                "published_at",
                "published_by",
                "generator",
                "source",
                "snapshot_seq",
                "snapshot_body_sha256",
                "rule_set_version",
                "rule_set_digest",
                "master_data_revision",
                "gates",
                "collab",
                "contents",
            ]
        );
        assert_eq!(written["version"], 1);
        assert_eq!(written["kind"], "pid-publish-manifest");
        assert_eq!(written["package_id"], "PID-1101-r003");
        assert_eq!(
            written["project"],
            json!({ "id": "CDU-01", "name": "常减压装置" })
        );
        assert_eq!(written["drawing_id"], "PID-1101");
        assert_eq!(written["revision"], 3);
        assert_eq!(written["published_at"], 1_794_800_100);
        assert_eq!(written["published_by"], "zhangsan");
        assert_eq!(written["generator"], "OpenCADStudio 0.9.0");
        assert_eq!(
            written["source"],
            json!({
                "file": "PID-1101 送审版.dwg",
                "bytes": SOURCE.len(),
                "sha256": digest(SOURCE),
            })
        );
        assert_eq!(written["snapshot_seq"], 7);
        assert_eq!(
            written["snapshot_body_sha256"],
            fixture.snapshot.body_sha256.as_str()
        );
        assert_eq!(written["rule_set_version"], "1");
        assert_eq!(written["rule_set_digest"], "a1b2c3d4e5f60718");
        assert_eq!(written["master_data_revision"], 3);
        assert_eq!(
            written["gates"],
            json!({
                "drc_errors": 0,
                "drc_warnings": 4,
                "drc_hints": 2,
                "waived": 1,
                "duplicate_tags": 0,
                "master_data": "configured",
                "index_complete": true,
                "result": "放行",
            })
        );
        assert_eq!(written["collab"], Value::Null);
    }

    /// 包里的几份文件：快照就是存下的那份；`topology.json` 的点和边就是快照的连通图，连接符的
    /// 配对状态取自项目索引里这张图的那一个；两份 CSV 与材料清单模块导出的一样；`bom.json`、
    /// `drc-report.json` 与导出文件一样带 `ok`；没配主数据时主数据文件只说没配。
    #[test]
    fn package_files_agree_with_the_snapshot_and_the_bill() {
        let fixture = Fixture::new();
        let staging = fixture.staging("stage");
        write_package(&staging, &fixture.input()).expect("能写包");

        let stored = fs::read(staging.join("snapshot.json")).expect("快照");
        assert_eq!(stored, fixture.snapshot.to_bytes());
        let snapshot: Value = serde_json::from_slice(&stored).expect("快照是 JSON");
        assert!(!fixture.snapshot.graph.edges.is_empty(), "图里有边");
        let links = read_json(&staging.join("topology.json"));
        assert_eq!(links["version"], 1);
        assert_eq!(links["drawing_id"], "PID-1101");
        assert_eq!(links["nodes"], snapshot["graph"]["nodes"]);
        assert_eq!(links["edges"], snapshot["graph"]["edges"]);
        assert_eq!(
            links["connectors"],
            json!([
                {
                    "kind": "pipe_out",
                    "line": "L-1001",
                    "status": "paired",
                    "peer_drawing": "PID-1102",
                    "upstream": "P-1001",
                    "downstream": null,
                },
                {
                    "kind": "pipe_in",
                    "line": "L-2001",
                    "status": "target_missing",
                    "peer_drawing": "PID-9999",
                    "upstream": null,
                    "downstream": "T-2001",
                },
                {
                    "kind": "pipe_in",
                    "line": "L-3001",
                    "status": "unpaired",
                    "peer_drawing": null,
                    "upstream": null,
                    "downstream": null,
                },
                {
                    "kind": "pipe_in",
                    "line": "L-4001",
                    "status": "paired",
                    "peer_drawing": "PID-1103",
                    "upstream": null,
                    "downstream": "E-4001",
                },
            ])
        );
        assert_eq!(
            links,
            serde_json::to_value(topology(&fixture.snapshot, &fixture.index)).expect("能写成 JSON")
        );

        let rows = bom::rows(&fixture.bom, None).expect("默认的列");
        assert_eq!(
            fs::read(staging.join("bom.csv")).expect("材料清单"),
            bom::csv_bytes(&rows)
        );
        assert_eq!(
            fs::read(staging.join("pipe-summary.csv")).expect("管线汇总"),
            bom::csv_bytes(&bom::pipe_summary_rows(&fixture.bom))
        );
        let bill = read_json(&staging.join("bom.json"));
        assert_eq!(bill["ok"], true);
        assert_eq!(bill["snapshot_seq"], 7);
        assert_eq!(
            serde_json::from_value::<Bom>(bill).expect("读回清单"),
            fixture.bom
        );

        let master = read_json(&staging.join("master-data.json"));
        assert_eq!(master, fixture.master.to_json());

        let checked = read_json(&staging.join("drc-report.json"));
        let mut expected = serde_json::to_value(&fixture.report).expect("能写成 JSON");
        expected["ok"] = Value::Bool(true);
        assert_eq!(checked, expected);
        let first = checked.as_object().and_then(|report| report.keys().next());
        assert_eq!(first.map(String::as_str), Some("ok"));

        let bare = fixture.staging("bare");
        let not_configured = gates(&fixture.report, 0, &Resolved::NotConfigured, true);
        let input = PackageInput {
            master_data: None,
            gates: &not_configured,
            ..fixture.input()
        };
        let manifest = write_package(&bare, &input).expect("没配主数据也能出包");
        assert_eq!(
            fs::read_to_string(bare.join("master-data.json")).expect("主数据"),
            "{\n  \"version\": 1,\n  \"configured\": false\n}\n"
        );
        assert_eq!(manifest.master_data_revision, None);
        let written = read_json(&bare.join(MANIFEST_FILE));
        assert_eq!(written["master_data_revision"], Value::Null);
        assert_eq!(written["gates"]["master_data"], "not_configured");
        assert_eq!(names(&bare), WRITTEN);
    }

    /// 清单的 `collab`：图里没有同步记录时是 `null`，有时就是那四项；包里的文件不因此多少
    /// （§3.2，离线规格 §5.7）。
    #[test]
    fn manifest_collab_is_null_without_a_sync_record() {
        let fixture = Fixture::new();
        let plain = fixture.staging("plain");
        let manifest = write_package(&plain, &fixture.input()).expect("能写包");
        assert_eq!(manifest.collab, None);
        assert_eq!(manifest.contents.len(), 10);
        assert_eq!(read_json(&plain.join(MANIFEST_FILE))["collab"], Value::Null);

        for reconciled in [true, false] {
            let record = CollabRecord {
                drawing_id: "PID-1101".to_owned(),
                repo_id: "7f3a9c".to_owned(),
                revision: 42,
                reconciled,
            };
            let staging = fixture.staging(if reconciled { "online" } else { "offline" });
            let input = PackageInput {
                collab: Some(&record),
                ..fixture.input()
            };
            let manifest = write_package(&staging, &input).expect("能写包");
            assert_eq!(manifest.collab.as_ref(), Some(&record));
            assert_eq!(manifest.contents.len(), 10);
            assert_eq!(names(&staging), WRITTEN);
            assert_eq!(
                read_json(&staging.join(MANIFEST_FILE))["collab"],
                json!({
                    "drawing_id": "PID-1101",
                    "repo_id": "7f3a9c",
                    "revision": 42,
                    "reconciled": reconciled,
                })
            );
        }
    }

    /// 门禁没过、快照没存、少一张出图、源文件不在：各报各的错和步骤，写包目录里什么都没写
    /// （§3.4、§3.6）。
    #[test]
    fn write_package_writes_nothing_when_it_cannot_make_a_whole_package() {
        let fixture = Fixture::new();
        let staging = fixture.staging("stage");
        let plots = ["PID-1101.pdf", "PID-1101.svg"];

        let blocked = gates(&report(1, 0, 0, 0), 0, &configured(), true);
        let input = PackageInput {
            gates: &blocked,
            ..fixture.input()
        };
        let error = write_package(&staging, &input).expect_err("门禁没过");
        assert_eq!(error, PublishError::Blocked);
        assert_eq!((error.code(), error.step()), (PUBLISH_BLOCKED, None));
        assert_eq!(names(&staging), plots);

        let mut unsaved = fixture.snapshot.clone();
        unsaved.seq = None;
        let input = PackageInput {
            snapshot: &unsaved,
            ..fixture.input()
        };
        let error = write_package(&staging, &input).expect_err("快照没存");
        assert_eq!(error, PublishError::SnapshotUnsaved);
        assert_eq!(
            (error.code(), error.step()),
            (PUBLISH_FAILED, Some("snapshot"))
        );
        assert_eq!(names(&staging), plots);

        let missing = fixture.scratch.0.join("没有.dwg");
        let input = PackageInput {
            source: &missing,
            ..fixture.input()
        };
        let error = write_package(&staging, &input).expect_err("源文件不在");
        assert_eq!(
            (error.code(), error.step()),
            (PUBLISH_FAILED, Some("write"))
        );
        assert_eq!(names(&staging), plots);

        fs::remove_file(staging.join("PID-1101.svg")).expect("能删");
        let error = write_package(&staging, &fixture.input()).expect_err("少一张出图");
        assert_eq!(
            error,
            PublishError::PlotMissing(staging.join("PID-1101.svg"))
        );
        assert_eq!((error.code(), error.step()), (PUBLISH_FAILED, Some("plot")));
        assert_eq!(names(&staging), ["PID-1101.pdf"]);
    }

    /// 校验（§3.5）：完好的包通过；任一文件改一个字节报 `PACKAGE_CORRUPT`、删掉报
    /// `PACKAGE_INCOMPLETE`，并列出是哪个；多出来的文件只记在 `extra`；清单里少了一类报缺那一类；
    /// 目录或清单不在报 `PACKAGE_NOT_FOUND`；清单读不出来、不是发布清单、列的路径出了包，报
    /// `PACKAGE_CORRUPT`。
    #[test]
    fn verify_package_finds_changed_missing_and_extra_files() {
        let fixture = Fixture::new();
        let package = fixture.staging("PID-1101-r003");
        write_package(&package, &fixture.input()).expect("能写包");
        let intact = VerifyReport {
            package_id: "PID-1101-r003".to_owned(),
            revision: 3,
            files: 11,
            missing: Vec::new(),
            mismatched: Vec::new(),
            extra: Vec::new(),
        };
        assert_eq!(verify_package(&package), Ok(intact.clone()));

        fs::write(package.join("备注.txt"), "审核意见").expect("能写");
        fs::create_dir(package.join("附件")).expect("能建目录");
        let report = verify_package(&package).expect("多出来的文件不算失败");
        assert_eq!(report.files, 12);
        assert_eq!(report.extra, ["备注.txt", "附件"]);
        fs::remove_file(package.join("备注.txt")).expect("能删");
        fs::remove_dir(package.join("附件")).expect("能删");

        for (_, path) in LISTED {
            let file = package.join(path);
            let original = fs::read(&file).expect("能读");
            let mut changed = original.clone();
            changed[original.len() / 2] ^= 0x01;
            fs::write(&file, &changed).expect("能改");
            match verify_package(&package) {
                Err(PublishError::Corrupt(report)) => {
                    assert_eq!(report.mismatched, [path]);
                    assert!(report.missing.is_empty(), "{path}");
                }
                other => panic!("改了 {path} 的一个字节：{other:?}"),
            }

            fs::remove_file(&file).expect("能删");
            match verify_package(&package) {
                Err(PublishError::Incomplete(report)) => {
                    assert_eq!(report.missing, [path]);
                    assert!(report.mismatched.is_empty(), "{path}");
                    assert_eq!(report.files, 10, "{path}");
                }
                other => panic!("删了 {path}：{other:?}"),
            }
            fs::write(&file, &original).expect("能改回去");
        }
        assert_eq!(verify_package(&package), Ok(intact));

        let nowhere = fixture.scratch.0.join("没有");
        assert_eq!(
            verify_package(&nowhere),
            Err(PublishError::NotFound(nowhere.clone()))
        );
        let manifest_path = package.join(MANIFEST_FILE);
        let manifest = fs::read(&manifest_path).expect("能读清单");
        fs::remove_file(&manifest_path).expect("能删");
        assert_eq!(
            verify_package(&package),
            Err(PublishError::NotFound(package.clone()))
        );

        fs::write(&manifest_path, b"{").expect("能写");
        let error = verify_package(&package).expect_err("清单不是 JSON");
        assert_eq!(error.code(), PACKAGE_CORRUPT);
        assert!(matches!(error, PublishError::ManifestInvalid(_)));

        let original: Value = serde_json::from_slice(&manifest).expect("清单是 JSON");
        let mut edited = original.clone();
        edited["contents"]
            .as_array_mut()
            .expect("文件列表")
            .retain(|content| content["path"] != "master-data.json");
        fs::write(&manifest_path, edited.to_string()).expect("能写");
        match verify_package(&package) {
            Err(PublishError::Incomplete(report)) => {
                assert_eq!(report.missing, ["项目主数据快照"]);
                assert!(report.mismatched.is_empty());
                assert_eq!(report.extra, ["master-data.json"]);
            }
            other => panic!("清单里少了主数据：{other:?}"),
        }

        for (field, value) in [
            ("kind", json!("pid-snapshot")),
            ("version", json!(2)),
            ("package_id", json!(null)),
        ] {
            let mut edited = original.clone();
            edited[field] = value;
            fs::write(&manifest_path, edited.to_string()).expect("能写");
            let error = verify_package(&package).expect_err(field);
            assert!(matches!(error, PublishError::ManifestInvalid(_)), "{field}");
        }
        for path in ["../PID-1101.dwg", "sub/bom.csv", "", MANIFEST_FILE] {
            let mut edited = original.clone();
            edited["contents"][0]["path"] = json!(path);
            fs::write(&manifest_path, edited.to_string()).expect("能写");
            let error = verify_package(&package).expect_err(path);
            assert!(matches!(error, PublishError::ManifestInvalid(_)), "{path}");
            assert_eq!(error.code(), PACKAGE_CORRUPT);
        }

        fs::write(&manifest_path, &manifest).expect("能写");
        assert!(verify_package(&package).is_ok(), "清单改回去又通过");
    }

    /// 落位（§3.1、§4）：自检过了才改名，改名后文件只读；目标已存在时不动原包；自检不过、包名
    /// 与目录名对不上时不改名；失败时写包目录都原样留给调用方。
    #[test]
    fn commit_renames_a_checked_package_once_and_makes_it_read_only() {
        let fixture = Fixture::new();
        let staging = fixture.staging(".PID-1101-r003-1");
        write_package(&staging, &fixture.input()).expect("能写包");
        let package = fixture.scratch.0.join("PID-1101-r003");
        commit(&staging, &package).expect("能落位");
        assert!(!staging.exists(), "写包目录改名成了包");
        assert_eq!(names(&package), WRITTEN);
        for name in WRITTEN {
            let metadata = fs::metadata(package.join(name)).expect("文件在");
            assert!(metadata.permissions().readonly(), "{name}");
        }
        assert_eq!(verify_package(&package).expect("落位后校验通过").files, 11);
        let published = files_in(&package);

        let again = fixture.staging(".PID-1101-r003-2");
        write_package(&again, &fixture.input()).expect("能再写一次");
        let error = commit(&again, &package).expect_err("已发布的包不覆盖");
        assert_eq!(error, PublishError::TargetExists(package.clone()));
        assert_eq!((error.code(), error.step()), (PUBLISH_TARGET_EXISTS, None));
        assert_eq!(files_in(&package), published, "原包不变");
        assert_eq!(names(&again), WRITTEN, "写包目录留着");

        let occupied = fixture.scratch.0.join("PID-1101-r003.txt");
        fs::write(&occupied, b"").expect("能写");
        assert_eq!(
            commit(&again, &occupied),
            Err(PublishError::TargetExists(occupied.clone()))
        );

        let misnamed = fixture.scratch.0.join("PID-1101-r006");
        let error = commit(&again, &misnamed).expect_err("清单里是 r003");
        assert_eq!(
            (error.code(), error.step()),
            (PUBLISH_FAILED, Some("verify"))
        );
        let PublishError::SelfCheck(inner) = &error else {
            panic!("{error:?}");
        };
        assert!(
            matches!(**inner, PublishError::ManifestInvalid(_)),
            "{inner:?}"
        );
        assert!(!misnamed.exists());
        assert_eq!(names(&again), WRITTEN);

        let fourth = fixture.staging(".PID-1101-r004-1");
        let input = PackageInput {
            revision: 4,
            ..fixture.input()
        };
        write_package(&fourth, &input).expect("能写第 4 版");
        fs::write(fourth.join("bom.csv"), b"changed").expect("能改");
        let target = fixture.scratch.0.join("PID-1101-r004");
        let error = commit(&fourth, &target).expect_err("自检不过");
        assert_eq!(
            (error.code(), error.step()),
            (PUBLISH_FAILED, Some("verify"))
        );
        let PublishError::SelfCheck(inner) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(inner.code(), PACKAGE_CORRUPT, "{inner:?}");
        assert!(!target.exists());
        assert_eq!(names(&fourth), WRITTEN);
    }
}

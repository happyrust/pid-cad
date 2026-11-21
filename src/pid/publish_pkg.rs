//! 发布包（SPEC-pid-publish）：包名与修订号（§3.1）、重复位号与四项门禁（§3.4），以及图里有
//! 同步记录时的协同前提（SPEC-collab-offline-sync §5.7）。除了列目标目录，都不碰文件：
//! 不读图、不写包。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::drc::CheckReport;
use super::master_data::Resolved;
use super::project_index::{
    compare_places, DrawingSummary, Duplicate, ObjectKind, Place, ProjectIndex, SummaryObject,
};
use super::snapshot::store;

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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::pid::drc::{Scope, ScopeKind, Summary, Trigger, REPORT_VERSION};
    use crate::pid::master_data::{self, MasterDataError, Source};
    use crate::pid::project_index::{build_index, SummarySource, SUMMARY_VERSION};
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
}

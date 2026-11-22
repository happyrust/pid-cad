//! 「校验与交付」面板 — docked (`PIDVERIFY`, `SPEC-pid-verify-deliver-panel.md`).
//!
//! One panel with three tabs over the results of the modules before it:
//! **规则检查** (the rule check and its waivers), **材料清单** (the bill of
//! materials) and **快照与发布** (snapshots, the diff of two of them, and
//! publishing). The panel has no logic of its own: each button calls the
//! entry point the command and the automation op call, so the panel and the
//! command line give the same result.
//!
//! The split is the collaboration panel's: the app turns its state into plain
//! data with the pure [`build_view`], and [`view`] only draws that
//! [`VerifyView`]. The interface reads the same `VerifyView`, so what it
//! reports is what a person sees.
//!
//! The texts are Chinese literals, as in the collaboration panel (§9 V2).

use crate::app::Message;
use crate::pid::bom::{self, Bom, BomRow, BomScope, PipeSummaryRow, Quantity};
use crate::pid::drc::{
    self, CheckReport, CheckResult, Scope, ScopeKind, Severity, Summary, Waiver,
    WAIVER_REASON_LIMIT,
};
use crate::pid::project_index::Duplicate;
use crate::pid::publish_pkg::{Gates, MasterDataState};
use crate::pid::snapshot::store::{SnapshotInfo, SnapshotStatus};
use crate::pid::snapshot::{self, Change, Diff, DiffObject, DiffSide, DiffSummary, EdgeChange};
use crate::ui::dock::{Chrome, PanelId};
use crate::ui::kit::{tooltip, TipPosition};
use crate::ui::wrap_bar::PosReport;
use iced::widget::{
    button, checkbox, column, container, progress_bar, row, scrollable, text, text_input,
};
use iced::{Element, Fill, Length, Theme};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The panel's tabs, in the order they are drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Check,
    Bom,
    Deliver,
}

impl Tab {
    pub const ALL: [Self; 3] = [Self::Check, Self::Bom, Self::Deliver];

    /// What the tab reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Check => "规则检查",
            Self::Bom => "材料清单",
            Self::Deliver => "快照与发布",
        }
    }

    /// The tab's name in `PIDVERIFY <tab>`, in the panel state and in the
    /// panel's actions (§3.2, §3.3).
    pub fn key(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Bom => "bom",
            Self::Deliver => "deliver",
        }
    }

    /// The tab `key` names, in any case.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tab| tab.key().eq_ignore_ascii_case(key))
    }

    /// `PosReport` id of the tab (§3.4).
    pub fn bounds_id(self) -> &'static str {
        match self {
            Self::Check => "pid-verify-tab-check",
            Self::Bom => "pid-verify-tab-bom",
            Self::Deliver => "pid-verify-tab-deliver",
        }
    }
}

/// What the rule-check tab says instead of a list (§4.2): never checked,
/// checked and nothing found, and everything found filtered out.
pub const NEVER_CHECKED: &str = "还没有检查过。点上面的按钮开始。";
pub const NOTHING_FOUND: &str = "没有发现问题。";
pub const ALL_FILTERED: &str = "没有符合筛选条件的结果。";

/// What the status line adds while the result shown may be out of date
/// (§3.1).
pub const STALE: &str = "图纸已改动，结果可能过期";

/// What the message bar says when a result clicked names an object with a
/// tag of its own that is no longer on its drawing (§4.3).
pub const GONE: &str = "对象已不在图上，结果可能过期";

/// Widget id of the reason box under a row (§4.4): 【豁免】 puts the cursor
/// in it.
pub const REASON_ID: &str = "pid-verify-waiver-reason";

/// What the material-list tab says instead of a table (§4.5): nothing
/// listed yet, a bill with no rows, and every row searched away.
pub const NEVER_LISTED: &str = "还没有生成材料清单。点上面的按钮开始。";
pub const NOTHING_LISTED: &str = "清单里没有行：范围内没有要计入的对象。";
pub const NONE_SEARCHED: &str = "没有符合搜索的行。";

/// What the material-list tab adds under its status line while the bill
/// shown may be out of date (§4.5).
pub const BOM_STALE: &str = "图纸已改动，清单可能过期";

/// What the material-list tab says instead of a table when tags repeat,
/// and when drawings of the scope did not read (§4.5).
pub const DUPLICATE_SAID: &str = "有重复位号，清单没有生成";
pub const UNREADABLE_SAID: &str = "范围内有图读不出来，清单没有生成";

/// What 「快照与发布」 says in place of its lists (§4.6): with no drawing in
/// front, and while the drawing in front was never saved, so it has no
/// folder to number snapshots in and no project to publish from.
pub const NO_DRAWING_SAID: &str = "当前没有图纸。";
pub const UNSAVED_SAID: &str = "图纸还没有保存，不能存带序号的快照，也不能发布";

/// The row on top of the snapshot list, the drawing as it is now, and
/// what a row says when its file is gone (§4.6).
pub const LIVE_SAID: &str = "当前图（未存快照）";
pub const MISSING_SAID: &str = "文件缺失";

/// What a diff says in place of its counts when the two sides are the
/// same (§4.6).
pub const IDENTICAL_SAID: &str = "无差异";

/// What a condition of the publish half says (§4.6 item 3): held, not
/// held, and a gate before 【检查发布条件】 is pressed.
pub const MET_SAID: &str = "满足";
pub const UNMET_SAID: &str = "不满足";
pub const UNCHECKED_SAID: &str = "未检查";

/// What 【校验】 says of a package with every file its manifest lists, each
/// of the size and checksum listed (§4.6 item 6).
pub const INTACT_SAID: &str = "完好";

/// What the publish half adds under its conditions while a drawing the
/// gates read changed since they were worked out (§4.6).
pub const GATES_STALE: &str = "图纸已改动，门禁的结果可能过期";

/// What the progress line says while the project is read, before how far
/// it is, and what the message bar says once 【取消】 stopped it (§4.7).
pub const BUSY_SAID: &str = "正在读项目图纸";
pub const CANCELLED_SAID: &str = "已取消";

/// Where the conditions of errors not waived and of repeated tags are among
/// [`PublishView::conditions`]: what they lead to when they do not hold.
const ERRORS_CONDITION: usize = 4;
const DUPLICATES_CONDITION: usize = 5;

/// What the user did in the panel: one message per button, tag and list
/// header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PidVerifyMsg {
    /// Put the panel on screen, as its switch among the dock's panels does.
    Show,
    /// Take it off screen, as its close button does.
    Hide,
    /// Bring a tab forward.
    Tab(Tab),
    /// 【检查当前图】.
    CheckDrawing,
    /// 【检查全项目】.
    CheckProject,
    /// A severity tag: on turns off, off turns on.
    Severity(Severity),
    /// A kind tag, the same way; `None` clears the kind filter.
    Kind(Option<String>),
    /// 「显示已豁免」 on or off.
    ShowWaived(bool),
    /// A group's header: folded opens, open folds.
    Fold(Group),
    /// 【再显示 200 条】 under the rule-check list.
    MoreChecks,
    /// 【导出…】: ask where to write the check result.
    PickExport,
    /// Write the check result to this file, as `PIDCHECK EXPORT` does.
    Export(String),
    /// A row of the rule-check list clicked: find its object (§4.3).
    Locate(usize),
    /// 【豁免】 at the end of a row: open the reason box under it (§4.4).
    AskWaiver(usize),
    /// What is typed in the reason box.
    WaiverReason(String),
    /// 【确认】 under the reason box: waive the row of this index for this
    /// reason.
    Waive(usize, String),
    /// 【取消】 under the reason box.
    CancelWaiver,
    /// 【取消豁免】 at the end of a waived row.
    Unwaive(usize),
    /// 「有 N 条豁免已经对不上」: list those waivers, or fold the list.
    StaleWaivers,
    /// 【删除】 beside a waiver no result matches: take it off the drawing
    /// in front.
    DropWaiver { rule_id: String, object_key: String },
    /// 【当前图】 on the material-list tab.
    BomDrawing,
    /// 【全项目】 on the material-list tab.
    BomProject,
    /// What is typed in the material list's search box.
    BomSearch(String),
    /// A kind's header in the material list: folded opens, open folds.
    BomFold(String),
    /// 【再显示 200 条】 under the material list.
    MoreBom,
    /// 【导出…】 under the material list: ask where to write the bill.
    PickBomExport,
    /// Write the bill shown to this file, as `PIDBOM EXPORT` writes a bill.
    BomExport(String),
    /// A row of the material list clicked: find its object (§4.5).
    BomLocate(usize),
    /// 「有 N 个对象没有位号」: list those objects, or fold the list.
    BomSkipped,
    /// One of the objects without a tag clicked: look at where it is.
    BomSkippedLocate(usize),
    /// One place of a tag found in more than one clicked: the index of the
    /// repeated (type code, tag) in the list, and of the place under it.
    BomDuplicateLocate(usize, usize),
    /// 「管道汇总」: open it, or fold it.
    BomSummary,
    /// What is typed in the reason box of 【存快照】 (§4.6).
    SnapshotReason(String),
    /// 【存快照】: a numbered snapshot of the drawing in front, for this
    /// reason.
    SnapshotSave(String),
    /// A row of the snapshot list ticked, or unticked.
    Pick(Pick),
    /// 【比对】: the two rows ticked, the older first (§4.6).
    Diff(Pick, Pick),
    /// An object of the diff clicked: its group, and its index there.
    DiffLocate(DiffGroup, usize),
    /// 【再显示 200 条】 under the diff.
    MoreDiff,
    /// 【导出比对结果】: ask where to write the diff.
    PickDiffExport,
    /// Write the diff shown to this file, as `PIDDIFF … EXPORT` writes one.
    DiffExport(String),
    /// What is typed in the publish folder box (§4.6); empty is the
    /// default folder.
    PublishDir(String),
    /// 【浏览…】 beside it: ask for a folder.
    PickPublishDir,
    /// 【检查发布条件】.
    PublishCheck,
    /// 【发布】: ask in the panel first.
    AskPublish,
    /// 【确认】 under the question: publish, as `PIDPUBLISH` does.
    Publish,
    /// 【取消】 under the question.
    CancelPublish,
    /// The condition of errors not waived clicked: the rule-check tab,
    /// listing errors only.
    PublishErrors,
    /// The condition of repeated tags clicked: list where each is, or fold
    /// the list.
    PublishDuplicates,
    /// One place of a repeated tag under it clicked: the index of the tag
    /// in the list, and of the place under it.
    PublishDuplicateLocate(usize, usize),
    /// 【校验】 on the record of this revision.
    PackageVerify(u32),
    /// 【取消】 on the progress line: stop reading the project (§4.7).
    Cancel,
}

impl PidVerifyMsg {
    pub(crate) fn message(self) -> Message {
        Message::PidVerify(self)
    }
}

/// A row of the snapshot list (§4.6): a numbered snapshot, or the drawing
/// as it is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    Live,
    Seq(u64),
}

impl Pick {
    /// How `deliver.picked` and the sides of `deliver.diff` write it
    /// (§3.2): the snapshot's number, or `"live"`.
    pub fn state(self) -> Value {
        match self {
            Self::Live => json!("live"),
            Self::Seq(seq) => json!(seq),
        }
    }

    /// The `PosReport` id of its row (§3.4).
    pub fn bounds_id(self) -> String {
        match self {
            Self::Live => format!("{SNAPSHOT_ROW_PREFIX}live"),
            Self::Seq(seq) => format!("{SNAPSHOT_ROW_PREFIX}{seq}"),
        }
    }

    /// The row a side of a diff the panel made was ticked as.
    fn of(side: &DiffSide) -> Self {
        side.seq.map_or(Self::Live, Self::Seq)
    }
}

/// The diff's groups (§4.6), in the order they are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffGroup {
    Added,
    Removed,
    Modified,
}

impl DiffGroup {
    pub const ALL: [Self; 3] = [Self::Added, Self::Removed, Self::Modified];

    /// What its header reads, before the count.
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "新增",
            Self::Removed => "删除",
            Self::Modified => "修改",
        }
    }

    /// What `diff_locate` names it by (§3.3), as the diff's JSON does.
    pub fn key(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Modified => "modified",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.key() == key)
    }

    /// The `PosReport` id of its object of `index`.
    pub fn item_id(self, index: usize) -> String {
        format!("{DIFF_ITEM_PREFIX}{}-{index}", self.key())
    }

    /// Its objects in `diff`.
    pub fn objects(self, diff: &Diff) -> &[DiffObject] {
        match self {
            Self::Added => &diff.added,
            Self::Removed => &diff.removed,
            Self::Modified => &diff.modified,
        }
    }
}

/// The rule-check list's groups (§4.2), in the order they are drawn: one per
/// severity, and the waived results' while they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Error,
    Warning,
    Info,
    Waived,
}

impl Group {
    pub const ALL: [Self; 4] = [Self::Error, Self::Warning, Self::Info, Self::Waived];

    /// What its header reads, before the count.
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "错误",
            Self::Warning => "警告",
            Self::Info => "提示",
            Self::Waived => "已豁免",
        }
    }

    /// The group a result is listed in.
    fn of(result: &CheckResult) -> Self {
        match (result.waived, result.severity) {
            (true, _) => Self::Waived,
            (false, Severity::Error) => Self::Error,
            (false, Severity::Warning) => Self::Warning,
            (false, Severity::Info) => Self::Info,
        }
    }

    /// Whether `ui` lists the group: a severity's while its tag is on, the
    /// waived results' while 「显示已豁免」 is.
    fn listed(self, ui: &VerifyUi) -> bool {
        match self {
            Self::Error => ui.severities.contains(&Severity::Error),
            Self::Warning => ui.severities.contains(&Severity::Warning),
            Self::Info => ui.severities.contains(&Severity::Info),
            Self::Waived => ui.show_waived,
        }
    }
}

/// `PosReport` id of the panel body (§3.4).
pub const PANEL_BOUNDS_ID: &str = "pid-verify-panel";

/// What every `PosReport` id of the panel starts with: closing the panel
/// forgets them all.
pub const BOUNDS_PREFIX: &str = "pid-verify-";

/// `PosReport` ids of 【检查当前图】 and 【检查全项目】, and what the id of
/// each row of the rule-check list starts with, its index following (§3.4).
pub const CHECK_DRAWING_ID: &str = "pid-verify-check-drawing";
pub const CHECK_PROJECT_ID: &str = "pid-verify-check-project";
pub const CHECK_ROW_PREFIX: &str = "pid-verify-check-row-";

/// What the id of each row of the material list starts with, its index
/// following (§3.4), and the `PosReport` ids of 【当前图】, 【全项目】, the
/// search box and 【导出…】 on its tab.
pub const BOM_ROW_PREFIX: &str = "pid-verify-bom-row-";
pub const BOM_DRAWING_ID: &str = "pid-verify-bom-drawing";
pub const BOM_PROJECT_ID: &str = "pid-verify-bom-project";
pub const BOM_SEARCH_ID: &str = "pid-verify-bom-search";
pub const BOM_EXPORT_ID: &str = "pid-verify-bom-export";

/// What the id of each row of the snapshot list starts with, the
/// snapshot's number or `live` following, and the `PosReport` id of
/// 【比对】 (§3.4); those of the reason box, 【存快照】 and 【导出比对结果】,
/// and what the id of each object of the diff starts with, its group's
/// key and its index there following.
pub const SNAPSHOT_ROW_PREFIX: &str = "pid-verify-snapshot-row-";
pub const DIFF_ID: &str = "pid-verify-diff";
pub const SNAPSHOT_REASON_ID: &str = "pid-verify-snapshot-reason";
pub const SNAPSHOT_SAVE_ID: &str = "pid-verify-snapshot-save";
pub const DIFF_EXPORT_ID: &str = "pid-verify-diff-export";
pub const DIFF_ITEM_PREFIX: &str = "pid-verify-diff-item-";
pub const PUBLISH_DIR_ID: &str = "pid-verify-publish-dir";
pub const PUBLISH_CHECK_ID: &str = "pid-verify-publish-check";
pub const PUBLISH_ID: &str = "pid-verify-publish";
pub const PUBLISH_CONFIRM_ID: &str = "pid-verify-publish-confirm";
pub const PACKAGE_ROW_PREFIX: &str = "pid-verify-package-";

/// `PosReport` ids of the progress bar, drawn only while something runs
/// (§3.4), and of 【取消】 beside it.
pub const BUSY_ID: &str = "pid-verify-busy";
pub const CANCEL_ID: &str = "pid-verify-cancel";

/// How many rows of each list the panel state returns when the request
/// does not say (`limit`, §3.2), and how many rows each group of the
/// rule-check list draws before 【再显示 200 条】 (§4.2).
pub const ROWS_PER_PAGE: usize = 200;

/// The panel's interface state (§3.1): one for the whole program, so the
/// panel stays as it was left across document switches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VerifyUi {
    pub tab: Tab,
    /// The severity tags switched on: all three at first.
    pub severities: Vec<Severity>,
    /// The kind tags switched on; none lists every kind.
    pub kinds: Vec<String>,
    /// Whether the waived results are listed, as a group of their own.
    pub show_waived: bool,
    /// The groups folded shut.
    pub folded: Vec<Group>,
    /// How many times 【再显示 200 条】 was pressed since the filters last
    /// changed: each group draws 200 rows more for each.
    pub more: usize,
    /// The last message (§3.1): what came of the last thing pressed.
    pub notice: Option<Notice>,
    /// The row last clicked (§3.2 `check.located`): the serial of the check
    /// it is a row of, and its index in that check's results.
    pub located: Option<(u64, usize)>,
    /// The reason box open under a row (§4.4), at most one.
    pub waiving: Option<Waiving>,
    /// Whether the waivers no result matches are listed.
    pub stale_open: bool,
    /// What is typed in the material list's search box (§4.5).
    pub bom_search: String,
    /// The kinds whose groups of the material list are folded shut.
    pub bom_folded: Vec<String>,
    /// How many times 【再显示 200 条】 was pressed under the material list
    /// since its search last changed.
    pub bom_more: usize,
    /// The row of the material list last clicked: the serial of the bill it
    /// is a row of, and its index in that bill's rows.
    pub bom_located: Option<(u64, usize)>,
    /// Whether the objects left out for having no tag are listed.
    pub skipped_open: bool,
    /// Whether 「管道汇总」 is open.
    pub summary_open: bool,
    /// What is typed in the reason box of 【存快照】 (§4.6).
    pub snapshot_reason: String,
    /// The rows of the snapshot list ticked, in the order they were (§3.1):
    /// another drawing in front clears them.
    pub picked: Vec<Pick>,
    /// How many times 【再显示 200 条】 was pressed under the diff since it
    /// was made.
    pub diff_more: usize,
    /// The object of the diff last clicked: its group and its index there.
    pub diff_located: Option<(DiffGroup, usize)>,
    /// What is typed in the publish folder box (§4.6): empty is the default
    /// folder.
    pub publish_dir: String,
    /// Whether 【发布】 asks its question under the buttons (§4.6 item 4).
    pub confirming: bool,
    /// Whether the condition of repeated tags lists where each tag is.
    pub duplicates_open: bool,
}

impl Default for VerifyUi {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            severities: Severity::ALL.to_vec(),
            kinds: Vec::new(),
            show_waived: false,
            folded: Vec::new(),
            more: 0,
            notice: None,
            located: None,
            waiving: None,
            stale_open: false,
            bom_search: String::new(),
            bom_folded: Vec::new(),
            bom_more: 0,
            bom_located: None,
            skipped_open: false,
            summary_open: false,
            snapshot_reason: String::new(),
            picked: Vec::new(),
            diff_more: 0,
            diff_located: None,
            publish_dir: String::new(),
            confirming: false,
            duplicates_open: false,
        }
    }
}

/// A reason box open under a row (§4.4): the serial of the check the row
/// is of, the row's index in its results, and what is typed in the box.
/// Another check shown closes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Waiving {
    pub serial: u64,
    pub index: usize,
    pub reason: String,
}

/// The panel's message bar (§3.2 `notice`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Notice {
    /// An error, or only news.
    pub error: bool,
    /// The module's error code, when it named one.
    pub code: Option<String>,
    pub text: String,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            error: false,
            code: None,
            text: text.into(),
        }
    }

    pub fn error(code: Option<String>, text: impl Into<String>) -> Self {
        Self {
            error: true,
            code,
            text: text.into(),
        }
    }

    /// As `notice` reads in the panel state.
    fn state(&self) -> Value {
        json!({
            "level": if self.error { "error" } else { "info" },
            "code": self.code,
            "text": self.text,
        })
    }
}

/// What the panel shows the state of besides its results (§5.1). The
/// program fills it in each time it rebuilds the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// Whether the panel is on screen.
    pub visible: bool,
    /// The drawing tab in front; `None` while the Start page is.
    pub document_id: Option<u64>,
    /// That drawing's number as the rule check names it: the project
    /// file's, else the file name; empty for a drawing never saved.
    pub drawing_id: Option<String>,
    /// Whether the session may change that drawing. Waiving and publishing
    /// are refused when it may not.
    pub writable: bool,
    /// Whether that drawing was saved: only then is it in a project folder
    /// to check.
    pub saved: bool,
    /// Whether that drawing has changes not saved: it is not published
    /// then.
    pub dirty: bool,
    /// What runs in the background (§4.7); `None` while nothing does. The
    /// buttons that would start more are grey meanwhile.
    pub busy: Option<Busy>,
    /// The waivers that drawing keeps, as (rule, object key): of the
    /// waivers no result matches, only these can be taken off from here.
    pub waivers: Vec<(String, String)>,
}

/// A scan of a project folder running in the background (§4.7): what is to
/// be done once it is read — `check`, `bom`, `publish_check`, `publish`, or
/// a command's `index` and `locate`; `None` when only an automation op
/// reads — and how many of the drawings it reads are back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub then: Option<&'static str>,
    pub done: usize,
    pub total: usize,
}

/// A rule check the program keeps (§3.1), lent to [`build_view`].
#[derive(Debug, Clone, Copy)]
pub struct KeptCheck<'a> {
    pub report: &'a CheckReport,
    /// Which check of the program's it was: of two, the one with the larger
    /// serial is the newer.
    pub serial: u64,
    /// Whether a drawing it read changed since (§3.1).
    pub stale: bool,
}

/// Why a bill of materials did not come out, kept in its place to be
/// listed (§4.5): the code and what it says, the (type code, tag) pairs
/// found in more than one place (`BOM_DUPLICATE_TAG`), and the drawings of
/// the scope that did not read (`BOM_SCOPE_INCOMPLETE`).
#[derive(Debug, Clone, PartialEq)]
pub struct BomFailed {
    pub code: String,
    pub text: String,
    pub duplicates: Vec<Duplicate>,
    pub unreadable: Vec<Unreadable>,
}

/// A drawing of the scope that did not read: its number, its file and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    pub drawing_id: String,
    pub file: Option<String>,
    pub why: String,
}

/// A bill of materials the program keeps for the panel (§3.1), or why it
/// did not come out, lent to [`build_view`].
#[derive(Debug, Clone, Copy)]
pub struct KeptBom<'a> {
    pub outcome: Result<&'a Bom, &'a BomFailed>,
    /// What it covers: the bill's own scope, or the one asked for when it
    /// did not come out.
    pub scope: &'a BomScope,
    /// How many drawings it read.
    pub drawings: usize,
    /// When it was made, Unix seconds.
    pub made_at: i64,
    /// Which bill of the program's it was: of two, the one with the larger
    /// serial is the newer.
    pub serial: u64,
    /// Whether a drawing it read changed since (§3.1).
    pub stale: bool,
}

/// The results kept with the drawing in front, on its tab (§3.1, §5.1):
/// its own rule check and bill of materials so far, its numbered snapshots
/// and the diff of two of them, and what publishing it found.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrawingResults<'a> {
    pub check: Option<KeptCheck<'a>>,
    pub bom: Option<KeptBom<'a>>,
    /// The snapshots as last read from the drawing's folder, by number;
    /// `None` before they are read, and for a drawing never saved.
    pub snapshots: Option<&'a [SnapshotInfo]>,
    /// The diff made last, while the two rows it compared stay ticked.
    pub diff: Option<&'a Diff>,
    /// What the publish half keeps of it.
    pub publish: KeptPublish<'a>,
}

/// The publish records of a drawing as last read from its target folder
/// (§3.1, `pid_publications`): the drawing's number as publishing names
/// it, the folder, the revision the next publish makes, and the packages
/// there, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Records {
    pub drawing_id: String,
    pub target_dir: String,
    pub next_revision: u32,
    pub packages: Vec<Package>,
}

/// A package of the drawing in its target folder (§4.6 item 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub package_id: String,
    pub revision: u32,
    /// Its folder.
    pub path: String,
    /// When it was published, Unix seconds; `None` when its manifest does
    /// not read.
    pub published_at: Option<i64>,
    pub published_by: Option<String>,
    /// Whether it was made of the drawing's file as that is on disk: of
    /// those, the newest only.
    pub current: bool,
}

/// What 【校验】 found of a package (§4.6 item 6): whole, or what is missing
/// or does not match, said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub intact: bool,
    pub text: String,
}

/// The gates the last 【检查发布条件】 worked out, or a publish was stopped
/// by (§4.6 items 3 and 5), with the tags they found repeated.
#[derive(Debug, Clone, PartialEq)]
pub struct GatesFound {
    pub gates: Gates,
    pub duplicates: Vec<Duplicate>,
    /// Whether the preconditions held and the gates passed then.
    pub can_publish: bool,
}

/// What the publish half keeps of the drawing in front, on its tab (§3.1),
/// lent to [`build_view`].
#[derive(Debug, Clone, Copy, Default)]
pub struct KeptPublish<'a> {
    /// The records as last read, or why they could not be; `None` before
    /// they are read.
    pub records: Option<Result<&'a Records, &'a str>>,
    /// Whether the file on disk was the one the program last read or wrote
    /// when the records were read.
    pub file_unchanged: bool,
    /// The last gates worked out, and whether a drawing they read changed
    /// since.
    pub gates: Option<(&'a GatesFound, bool)>,
    /// What 【校验】 found, by revision.
    pub verified: &'a [(u32, Verified)],
    /// The package folder the last publish of this run made of it.
    pub last_package: Option<&'a str>,
}

/// The results kept for the folder that drawing is in (§3.1, §5.1): the
/// project's or a PBS node's rule check and bill of materials so far.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectResults<'a> {
    pub check: Option<KeptCheck<'a>>,
    pub bom: Option<KeptBom<'a>>,
}

/// Whether each of the panel's buttons can be pressed now (§3.2, §4.8).
/// The buttons on a row say so on the row (`can_waive`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Buttons {
    pub check_drawing: bool,
    pub check_project: bool,
    pub check_export: bool,
    pub bom_drawing: bool,
    pub bom_project: bool,
    pub bom_export: bool,
    pub snapshot_save: bool,
    pub diff: bool,
    pub diff_export: bool,
    pub publish_check: bool,
    pub publish: bool,
}

/// The panel's buttons that start something, export or publish (§4.8), as
/// [`VerifyView::grey`] names them. Those on a row of the rule-check list
/// are the row's ([`RowView::grey`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    CheckDrawing,
    CheckProject,
    CheckExport,
    BomDrawing,
    BomProject,
    BomExport,
    SnapshotSave,
    Diff,
    DiffExport,
    PublishCheck,
    Publish,
    /// 【校验】 on a package's row.
    PackageVerify,
}

impl Button {
    pub const ALL: [Self; 12] = [
        Self::CheckDrawing,
        Self::CheckProject,
        Self::CheckExport,
        Self::BomDrawing,
        Self::BomProject,
        Self::BomExport,
        Self::SnapshotSave,
        Self::Diff,
        Self::DiffExport,
        Self::PublishCheck,
        Self::Publish,
        Self::PackageVerify,
    ];
}

impl Buttons {
    /// The flags of `view`'s buttons: each can be pressed while it is not
    /// grey ([`VerifyView::grey`]).
    fn of(view: &VerifyView) -> Self {
        let up = |button| view.grey(button).is_none();
        Self {
            check_drawing: up(Button::CheckDrawing),
            check_project: up(Button::CheckProject),
            check_export: up(Button::CheckExport),
            bom_drawing: up(Button::BomDrawing),
            bom_project: up(Button::BomProject),
            bom_export: up(Button::BomExport),
            snapshot_save: up(Button::SnapshotSave),
            diff: up(Button::Diff),
            diff_export: up(Button::DiffExport),
            publish_check: up(Button::PublishCheck),
            publish: up(Button::Publish),
        }
    }
}

/// The rule-check tab worked out (§4.2), which its block of the panel state
/// reads (§3.2 `check`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckView {
    /// The check shown — the newer of the drawing's and its project's — and
    /// what the filters leave of it; `None` before either.
    pub shown: Option<Shown>,
    /// The severity tags switched on, in their order.
    pub severities: Vec<Severity>,
    /// The kind tags switched on, of the kinds the check found; none lists
    /// every kind.
    pub kinds_on: Vec<String>,
    pub show_waived: bool,
}

/// What the rule-check tab shows of the check it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// Whether a drawing it read changed since.
    pub stale: bool,
    pub scope: ScopeKind,
    /// The status line's words for the scope, as the command line says it:
    /// 「图 PID-1101」「全项目」「PBS 节点 U-100」.
    pub scope_said: String,
    /// Unix seconds.
    pub checked_at: i64,
    pub rule_set_version: String,
    pub elapsed_ms: u64,
    /// The check's own counts, whatever the filters.
    pub summary: Summary,
    /// How many results it found, waived ones included.
    pub found: usize,
    /// The kinds of object it found results on, for the kind tags.
    pub kinds: Vec<String>,
    pub notices: Vec<drc::Notice>,
    /// The waivers no result matches any more.
    pub stale_waivers: Vec<StaleWaiver>,
    /// Whether they are listed.
    pub stale_open: bool,
    /// The groups with rows the filters leave, in order.
    pub groups: Vec<GroupView>,
    /// Every row the filters leave, group after group, folded groups'
    /// included.
    pub rows: Vec<RowView>,
    /// Which check of the program's it is ([`KeptCheck::serial`]).
    pub serial: u64,
    /// The index of the row last clicked, while this check is shown.
    pub located: Option<usize>,
}

/// A waiver no result of the check shown matches (§4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleWaiver {
    pub rule_id: String,
    pub object_key: String,
    pub reason: String,
    pub by: String,
    /// Unix seconds.
    pub at: i64,
    /// Whether 【删除】 can be pressed: the drawing in front keeps it and
    /// the session may change that drawing.
    pub can_drop: bool,
}

/// One group of the rule-check list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupView {
    pub group: Group,
    /// Its rows the filters leave: the header's count.
    pub count: usize,
    /// Where its rows start in [`Shown::rows`].
    pub first: usize,
    /// How many of them are drawn: none while folded, else 200 and 200 more
    /// for each 【再显示 200 条】.
    pub drawn: usize,
    pub folded: bool,
}

impl GroupView {
    /// Whether 【再显示 200 条】 is drawn under it.
    pub fn has_more(&self) -> bool {
        !self.folded && self.drawn < self.count
    }
}

/// One row of the rule-check list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    /// Where the result is in the check's `results[]`: what the panel's
    /// actions name the row by.
    pub index: usize,
    pub rule_id: String,
    pub severity: Severity,
    pub object_key: String,
    pub drawing_id: String,
    /// The object's name (§3.2 `label`): its tag, else 「种类（所属）」.
    pub label: String,
    pub message: String,
    pub fix: String,
    pub fields: Vec<String>,
    pub waived: bool,
    /// Why, by whom and when (Unix seconds) it was waived, on waived rows.
    pub waiver: Option<(String, String, i64)>,
    /// Whether the object is on the drawing in front; when it is not, the
    /// first line names the drawing it is on.
    pub here: bool,
    pub can_waive: bool,
    /// Whether 【取消豁免】 can be pressed: on a waived row, on the terms
    /// of `can_waive` (§4.4).
    pub can_unwaive: bool,
    /// Whether it is the row last clicked.
    pub located: bool,
    /// What is typed in its reason box, while that is open.
    pub reason: Option<String>,
}

impl RowView {
    /// The row's first line (§4.2): rule and object, then the drawing when
    /// the object is on another one.
    pub fn headline(&self) -> String {
        if self.here || self.drawing_id.is_empty() {
            format!("{} {}", self.rule_id, self.label)
        } else {
            format!("{} {} · {}", self.rule_id, self.label, self.drawing_id)
        }
    }

    /// The row's `PosReport` id (§3.4).
    pub fn bounds_id(&self) -> String {
        format!("{CHECK_ROW_PREFIX}{}", self.index)
    }

    /// Why 【豁免】 on the row is grey, or with `unwaive` 【取消豁免】, in a
    /// session that may write or not (§4.8); `None` while it can be pressed.
    pub fn grey(&self, writable: bool, unwaive: bool) -> Option<&'static str> {
        if !writable {
            Some("readonly")
        } else if !self.here {
            Some("not_on_this_drawing")
        } else if unwaive && !self.waived {
            Some("not_waived")
        } else if !unwaive && self.waived {
            Some("already_waived")
        } else {
            None
        }
    }
}

/// The material-list tab worked out (§4.5), which its block of the panel
/// state reads (§3.2 `bom`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BomView {
    /// The bill shown — the newer of the drawing's and its project's — or
    /// why it did not come out; `None` before either.
    pub shown: Option<BomShown>,
    /// What is typed in the search box.
    pub search: String,
}

/// What the material-list tab shows of the bill it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct BomShown {
    /// Whether a drawing it read changed since.
    pub stale: bool,
    pub scope: bom::ScopeKind,
    /// The status line's words for the scope, as the command line says
    /// them: 「图 PID-1101」「全项目 2 张图」「PBS 节点 U-100 1 张图」.
    pub scope_said: String,
    /// When it was made, Unix seconds.
    pub made_at: i64,
    /// Which bill of the program's it is ([`KeptBom::serial`]).
    pub serial: u64,
    /// The table, or why there is none.
    pub table: Result<BomTable, BomFailed>,
}

/// The table of a bill that came out (§4.5).
#[derive(Debug, Clone, PartialEq)]
pub struct BomTable {
    /// The rows of the whole bill, whatever the search: the total at the
    /// bottom.
    pub total: usize,
    /// Its rows of each kind, in its order, kinds without rows left out.
    pub by_kind: Vec<(&'static str, usize)>,
    pub notices: Vec<bom::Notice>,
    /// The objects left out for having no tag.
    pub skipped: Vec<SkippedView>,
    /// Whether they are listed.
    pub skipped_open: bool,
    /// The groups with rows the search leaves, in the bill's kind order.
    pub groups: Vec<BomGroup>,
    /// Every row the search leaves, group after group, folded groups'
    /// included.
    pub rows: Vec<BomRowView>,
    /// The side table: pipe length by (pipe class, nominal diameter,
    /// insulation class).
    pub pipe_summary: Vec<PipeSummaryRow>,
    /// Whether 「管道汇总」 is open.
    pub summary_open: bool,
    /// Whether the rows name the drawings they are on: they do in the
    /// project's and a node's bills.
    pub across: bool,
    /// The index of the row last clicked, while this bill is shown.
    pub located: Option<usize>,
}

/// One group of the material list: the rows of one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BomGroup {
    pub kind: &'static str,
    /// Its rows the search leaves: the header's count.
    pub count: usize,
    /// Where its rows start in [`BomTable::rows`].
    pub first: usize,
    /// How many of them are drawn: none while folded, else 200 and 200 more
    /// for each 【再显示 200 条】.
    pub drawn: usize,
    pub folded: bool,
}

impl BomGroup {
    /// Whether 【再显示 200 条】 is drawn under it.
    pub fn has_more(&self) -> bool {
        !self.folded && self.drawn < self.count
    }
}

/// One row of the material list.
#[derive(Debug, Clone, PartialEq)]
pub struct BomRowView {
    /// Where the row is in the bill's `rows[]`: what the panel's actions
    /// name it by.
    pub index: usize,
    pub row: BomRow,
    /// Whether it is the row last clicked.
    pub located: bool,
}

impl BomRowView {
    /// The quantity and its unit: metres to two places, counts whole.
    pub fn quantity(&self) -> String {
        match self.row.quantity {
            Quantity::Length(metres) => format!("{metres:.2} {}", self.row.unit),
            Quantity::Count(count) => format!("{count} {}", self.row.unit),
        }
    }

    /// The row's `PosReport` id (§3.4).
    pub fn bounds_id(&self) -> String {
        format!("{BOM_ROW_PREFIX}{}", self.index)
    }
}

/// An object left out of the bill for having no tag (§4.5).
#[derive(Debug, Clone, PartialEq)]
pub struct SkippedView {
    pub kind: &'static str,
    pub type_code: String,
    pub drawing_id: String,
    pub at: [f64; 2],
}

/// What the panel draws, worked out by [`build_view`]: plain data, so the
/// interface can report it and a test can compare it.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyView {
    /// The tab in front.
    pub tab: Tab,
    /// What the tab in front says instead of a list; `None` while it draws
    /// one.
    pub empty: Option<&'static str>,
    /// Whether the panel is on screen.
    pub visible: bool,
    /// The drawing tab the panel shows the results of, and its number
    /// ([`Context`]).
    pub document_id: Option<u64>,
    pub drawing_id: Option<String>,
    /// Whether the session may change that drawing.
    pub writable: bool,
    /// What runs in the background ([`Context::busy`]).
    pub busy: Option<Busy>,
    /// Which buttons can be pressed.
    pub buttons: Buttons,
    /// The message bar.
    pub notice: Option<Notice>,
    /// The rule-check tab.
    pub check: CheckView,
    /// The material-list tab.
    pub bom: BomView,
    /// 「快照与发布」.
    pub deliver: DeliverView,
}

/// 「快照与发布」 worked out (§4.6), which the `deliver` block of the panel
/// state reads (§3.2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeliverView {
    /// Whether the drawing in front was saved: only then has it a folder to
    /// number snapshots in, and a list.
    pub saved: bool,
    /// What is typed in the reason box.
    pub reason: String,
    /// The snapshots, newest first, under the row of the drawing as it is.
    pub rows: Vec<SnapshotRow>,
    /// Whether that row is ticked.
    pub live_picked: bool,
    /// The rows ticked that can be, in the order they were.
    pub picked: Vec<Pick>,
    /// The diff shown.
    pub diff: Option<DiffView>,
    /// The publish half; `None` with no drawing in front.
    pub publish: Option<PublishView>,
}

impl DeliverView {
    /// The two rows ticked as 【比对】 compares them, the older first: of
    /// two snapshots the lower number, the drawing as it is always second.
    /// `None` unless exactly two are ticked.
    pub fn pair(&self) -> Option<(Pick, Pick)> {
        let &[one, other] = self.picked.as_slice() else {
            return None;
        };
        Some(match (one, other) {
            (Pick::Seq(a), Pick::Seq(b)) if b < a => (other, one),
            (Pick::Live, _) => (other, one),
            _ => (one, other),
        })
    }
}

/// A snapshot in the list (§4.6).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotRow {
    pub info: SnapshotInfo,
    pub picked: bool,
}

impl SnapshotRow {
    /// Whether its file is gone: it cannot be ticked.
    pub fn missing(&self) -> bool {
        self.info.status == SnapshotStatus::Missing
    }
}

/// A diff shown (§4.6).
#[derive(Debug, Clone, PartialEq)]
pub struct DiffView {
    /// The rows compared, the older first.
    pub a: Pick,
    pub b: Pick,
    pub identical: bool,
    pub summary: DiffSummary,
    /// What the diff warns of, said under its summary.
    pub notices: Vec<snapshot::Notice>,
    /// The groups with objects, in order.
    pub groups: Vec<DiffGroupView>,
}

/// A group of the diff: its header and its objects.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffGroupView {
    pub group: DiffGroup,
    /// How many objects it has: its header's count.
    pub count: usize,
    /// How many of them are drawn: 200, and 200 more for each 【再显示 200
    /// 条】.
    pub drawn: usize,
    pub items: Vec<DiffItem>,
}

/// An object of the diff.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffItem {
    /// Its index in its group: what `diff_locate` names it by.
    pub index: usize,
    /// Its kind, type code and tag.
    pub label: String,
    /// What changed on it, a line each: modified objects only.
    pub changes: Vec<String>,
    /// Whether it was the last clicked.
    pub located: bool,
}

/// The publish half of 「快照与发布」 worked out (§4.6), which
/// `deliver.publish` reads (§3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct PublishView {
    /// 「未发布」「已发布，修订 N」 or 「修订 N 之后有改动」; `None` while the
    /// records are not read.
    pub state: Option<String>,
    /// Why the records could not be read.
    pub unlisted: Option<String>,
    pub target_dir: Option<String>,
    pub next_revision: Option<u32>,
    /// What is typed in the folder box.
    pub dir: String,
    pub preconditions: Preconditions,
    /// The last gates worked out.
    pub gates: Option<GatesFound>,
    /// Whether a drawing they read changed since.
    pub stale: bool,
    /// Whether the condition of repeated tags lists where each tag is.
    pub duplicates_open: bool,
    /// The records, newest first.
    pub packages: Vec<PackageRow>,
    pub last_package: Option<String>,
    /// The question 【发布】 asks, while it asks it.
    pub asking: Option<String>,
}

/// The four preconditions of publishing (§4.6 item 3), worked out again
/// each time the view is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Preconditions {
    pub saved: bool,
    pub clean: bool,
    pub file_unchanged: bool,
    pub writable_session: bool,
}

impl Preconditions {
    /// All four hold: 【发布】 can be pressed (§4.8).
    pub fn held(self) -> bool {
        self.saved && self.clean && self.file_unchanged && self.writable_session
    }
}

/// A record of the publish half: the package — current only while the
/// drawing has no changes unsaved — and what 【校验】 found of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRow {
    pub package: Package,
    pub verified: Option<Verified>,
}

impl PublishView {
    /// The eight conditions in the order they are drawn (§4.6 item 3), each
    /// said and whether it holds; a gate before 【检查发布条件】 is not known.
    pub fn conditions(&self) -> Vec<(String, Option<bool>)> {
        let preconditions = self.preconditions;
        let gates = self.gates.as_ref().map(|found| &found.gates);
        let counted = |said: &str, count: Option<usize>, unit: &str| match count {
            Some(count) if count > 0 => format!("{said}（{count} {unit}）"),
            _ => said.to_owned(),
        };
        let master = match gates.map(|gates| gates.master_data) {
            Some(MasterDataState::NotConfigured) => "主数据读得出来（没有配置）",
            _ => "主数据读得出来",
        };
        vec![
            ("已保存".to_owned(), Some(preconditions.saved)),
            ("没有未存的修改".to_owned(), Some(preconditions.clean)),
            (
                "文件没有在别处被改过".to_owned(),
                Some(preconditions.file_unchanged),
            ),
            ("会话可写".to_owned(), Some(preconditions.writable_session)),
            (
                counted(
                    "没被豁免的错误为 0",
                    gates.map(|gates| gates.drc_errors),
                    "条",
                ),
                gates.map(|gates| gates.drc_errors == 0),
            ),
            (
                counted(
                    "重复位号为 0",
                    gates.map(|gates| gates.duplicate_tags),
                    "个",
                ),
                gates.map(|gates| gates.duplicate_tags == 0),
            ),
            (
                master.to_owned(),
                gates.map(|gates| gates.master_data != MasterDataState::Unreadable),
            ),
            (
                "项目里的图都读得出来".to_owned(),
                gates.map(|gates| gates.index_complete),
            ),
        ]
    }
}

impl PackageRow {
    /// The `PosReport` id of its row.
    pub fn bounds_id(&self) -> String {
        format!("{PACKAGE_ROW_PREFIX}{}", self.package.revision)
    }
}

impl Default for VerifyView {
    fn default() -> Self {
        build_view(&VerifyUi::default(), None, None, &Context::default())
    }
}

impl VerifyView {
    /// Why `button` is grey now (§4.8), as the action pressing it is
    /// refused (§3.7); `None` while it can be pressed. Checking, listing
    /// and 【校验】 need a drawing in front, and the project's, a snapshot
    /// and 【检查发布条件】 a saved one; none of them while something runs.
    /// 【发布】 needs the four preconditions of publishing held. Exporting
    /// needs a result shown, whatever runs — of a bill, one that came out —
    /// and 【比对】 exactly two rows ticked.
    pub fn grey(&self, button: Button) -> Option<&'static str> {
        let drawing = if self.document_id.is_none() {
            Some("no_drawing")
        } else if self.busy.is_some() {
            Some("busy")
        } else {
            None
        };
        let project = drawing.or((!self.deliver.saved).then_some("no_project_dir"));
        let result = |shown: bool| (!shown).then_some("no_result");
        match button {
            Button::CheckDrawing | Button::BomDrawing | Button::PackageVerify => drawing,
            Button::CheckProject
            | Button::BomProject
            | Button::SnapshotSave
            | Button::PublishCheck => project,
            Button::CheckExport => result(self.check.shown.is_some()),
            Button::BomExport => result(
                self.bom
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.table.is_ok()),
            ),
            Button::Diff => self.deliver.pair().is_none().then_some("not_two_picked"),
            Button::DiffExport => result(self.deliver.diff.is_some()),
            Button::Publish => {
                let held = self
                    .deliver
                    .publish
                    .as_ref()
                    .is_some_and(|publish| publish.preconditions.held());
                let why = if self.writable {
                    "precondition"
                } else {
                    "readonly"
                };
                drawing.or((!held).then_some(why))
            }
        }
    }
}

/// What the tip of a grey button says, for the `reason` it is grey for.
pub fn grey_said(reason: &str) -> &'static str {
    match reason {
        "no_drawing" => "当前没有图纸",
        "busy" => "正在读项目图纸；读完或者取消以后再按",
        "no_project_dir" => "图还没保存过，不属于哪个项目；存盘以后再按",
        "no_result" => "还没有可以导出的结果",
        "not_two_picked" => "要恰好勾两项才能比对",
        "readonly" => "这个会话只读",
        "precondition" => "四个发布前提没有都满足，见上面的条件",
        "not_on_this_drawing" => "这条不在当前图上；到它所在的图上再改",
        "already_waived" => "已经豁免了",
        "not_waived" => "没有豁免",
        _ => "现在按不了",
    }
}

/// The tip of 【豁免】 while its row's reason box is open (§4.4).
const WAIVING_SAID: &str = "理由填在下面，按【确认】豁免";

/// Interface state, the results kept with the drawing in front and with its
/// project folder, and the program's state around them → what to draw
/// (§4.1): which check is shown, what the filters leave of it and how it is
/// grouped, what each row is called, which buttons can be pressed.
pub fn build_view(
    ui: &VerifyUi,
    drawing: Option<&DrawingResults<'_>>,
    project: Option<&ProjectResults<'_>>,
    context: &Context,
) -> VerifyView {
    let kept = newer(
        drawing.and_then(|results| results.check),
        project.and_then(|results| results.check),
        |kept| kept.serial,
    );
    let check = check_view(ui, kept, context);
    let kept = newer(
        drawing.and_then(|results| results.bom),
        project.and_then(|results| results.bom),
        |kept| kept.serial,
    );
    let bom = bom_view(ui, kept);
    let deliver = deliver_view(
        ui,
        drawing.and_then(|results| results.snapshots),
        drawing.and_then(|results| results.diff),
        &drawing.map(|results| results.publish).unwrap_or_default(),
        context,
    );
    let empty = match ui.tab {
        Tab::Check => check_empty(&check),
        Tab::Bom => bom_empty(&bom),
        Tab::Deliver => deliver_empty(&deliver, context),
    };
    let mut view = VerifyView {
        tab: ui.tab,
        empty,
        visible: context.visible,
        document_id: context.document_id,
        drawing_id: context.drawing_id.clone(),
        writable: context.writable,
        busy: context.busy,
        buttons: Buttons::default(),
        notice: ui.notice.clone(),
        check,
        bom,
        deliver,
    };
    view.buttons = Buttons::of(&view);
    view
}

/// Of the drawing's result and its project's, the newer by `serial`
/// (§3.1).
fn newer<T>(drawing: Option<T>, project: Option<T>, serial: impl Fn(&T) -> u64) -> Option<T> {
    match (drawing, project) {
        (Some(drawing), Some(project)) if serial(&project) > serial(&drawing) => Some(project),
        (Some(drawing), _) => Some(drawing),
        (None, project) => project,
    }
}

/// What changed on an object of a diff, as its line under the object reads
/// (§4.6).
fn change_said(change: &Change) -> String {
    match change {
        Change::Property { field, old, new } => format!(
            "属性 {field}：{} → {}",
            value_said(old.as_deref()),
            value_said(new.as_deref())
        ),
        Change::Position { old, new } => {
            format!("位置：{} → {}", numbers_said(old), numbers_said(new))
        }
        Change::Length { old, new } => {
            format!("长度：{} → {}", number_said(*old), number_said(*new))
        }
        Change::Fittings { symbol, old, new } => format!("管件 {symbol}：{old} → {new} 个"),
        Change::Connection { change, with, port } => {
            let change = match change {
                EdgeChange::Added => "新增",
                EdgeChange::Removed => "删除",
            };
            let port = port
                .as_deref()
                .map(|port| format!("（本端 {port}）"))
                .unwrap_or_default();
            format!("连接：{change}，接到 {with}{port}")
        }
        Change::Peer { field, old, new } => format!(
            "对端 {field}：{} → {}",
            value_said(old.as_deref()),
            value_said(new.as_deref())
        ),
    }
}

/// A property's value, or a page connector's peer, as a change reads it:
/// none, or empty, is 「（空）」.
fn value_said(value: Option<&str>) -> &str {
    value.filter(|said| !said.is_empty()).unwrap_or("（空）")
}

/// A length or a coordinate to the thousandth, as short as that reads: no
/// zeros after the last digit that counts, but a decimal at least.
fn number_said(number: f64) -> String {
    let said = format!("{number:.3}");
    let said = said.trim_end_matches('0');
    match said.strip_suffix('.') {
        Some(whole) => format!("{whole}.0"),
        None => said.to_owned(),
    }
}

/// A point, each coordinate as [`number_said`] writes it.
fn numbers_said(numbers: &[f64]) -> String {
    let said: Vec<String> = numbers.iter().copied().map(number_said).collect();
    format!("({})", said.join(", "))
}

/// The rule-check tab of `kept` under the filters of `ui` (§4.2). Kind tags
/// switched on for kinds this check did not find filter nothing; when none
/// of the kinds switched on was found, every kind is listed.
fn check_view(ui: &VerifyUi, kept: Option<KeptCheck<'_>>, context: &Context) -> CheckView {
    let severities: Vec<Severity> = Severity::ALL
        .into_iter()
        .filter(|severity| ui.severities.contains(severity))
        .collect();
    let Some(kept) = kept else {
        return CheckView {
            shown: None,
            severities,
            kinds_on: Vec::new(),
            show_waived: ui.show_waived,
        };
    };
    let report = kept.report;
    let kinds: Vec<String> = report
        .results
        .iter()
        .map(|result| result.object.kind.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let kinds_on: Vec<String> = kinds
        .iter()
        .filter(|kind| ui.kinds.contains(kind))
        .cloned()
        .collect();
    let drawn = ROWS_PER_PAGE.saturating_mul(ui.more.saturating_add(1));
    let located = ui
        .located
        .filter(|(serial, _)| *serial == kept.serial)
        .map(|(_, index)| index);
    let waiving = ui
        .waiving
        .as_ref()
        .filter(|waiving| waiving.serial == kept.serial);
    let mut groups = Vec::new();
    let mut rows = Vec::new();
    for group in Group::ALL.into_iter().filter(|group| group.listed(ui)) {
        let first = rows.len();
        rows.extend(
            report
                .results
                .iter()
                .enumerate()
                .filter(|(_, result)| {
                    Group::of(result) == group
                        && (kinds_on.is_empty() || kinds_on.contains(&result.object.kind))
                })
                .map(|(index, result)| {
                    let mut row = row_view(index, result, context);
                    row.located = located == Some(index);
                    row.reason = waiving
                        .filter(|waiving| waiving.index == index && row.can_waive)
                        .map(|waiving| waiving.reason.clone());
                    row
                }),
        );
        let count = rows.len() - first;
        if count == 0 {
            continue;
        }
        let folded = ui.folded.contains(&group);
        groups.push(GroupView {
            group,
            count,
            first,
            drawn: if folded { 0 } else { count.min(drawn) },
            folded,
        });
    }
    CheckView {
        shown: Some(Shown {
            stale: kept.stale,
            scope: report.scope.kind,
            scope_said: scope_said(&report.scope),
            checked_at: report.checked_at,
            rule_set_version: report.rule_set_version.clone(),
            elapsed_ms: report.elapsed_ms,
            summary: report.summary,
            found: report.results.len(),
            kinds,
            notices: report.notices.clone(),
            stale_waivers: report
                .stale_waivers
                .iter()
                .map(|waiver| stale_waiver(waiver, context))
                .collect(),
            stale_open: ui.stale_open,
            groups,
            rows,
            serial: kept.serial,
            located,
        }),
        severities,
        kinds_on,
        show_waived: ui.show_waived,
    }
}

/// What the rule-check tab says instead of a list (§4.2).
fn check_empty(check: &CheckView) -> Option<&'static str> {
    match &check.shown {
        None => Some(NEVER_CHECKED),
        Some(shown) if shown.found == 0 => Some(NOTHING_FOUND),
        Some(shown) if shown.rows.is_empty() => Some(ALL_FILTERED),
        Some(_) => None,
    }
}

/// The material-list tab of `kept` under the search and the folds of `ui`
/// (§4.5).
fn bom_view(ui: &VerifyUi, kept: Option<KeptBom<'_>>) -> BomView {
    BomView {
        shown: kept.map(|kept| BomShown {
            stale: kept.stale,
            scope: kept.scope.kind,
            scope_said: bom_scope_said(kept.scope, kept.drawings),
            made_at: kept.made_at,
            serial: kept.serial,
            table: kept
                .outcome
                .map(|bill| bom_table(ui, bill, kept.serial))
                .map_err(BomFailed::clone),
        }),
        search: ui.bom_search.clone(),
    }
}

/// The table of `bill` under the search and the folds of `ui`: the rows of
/// each kind the search leaves, a group per kind in the bill's own kind
/// order, while the totals are the whole bill's.
fn bom_table(ui: &VerifyUi, bill: &Bom, serial: u64) -> BomTable {
    let search = ui.bom_search.to_lowercase();
    let drawn = ROWS_PER_PAGE.saturating_mul(ui.bom_more.saturating_add(1));
    let located = ui
        .bom_located
        .filter(|(clicked, _)| *clicked == serial)
        .map(|(_, index)| index);
    let by_kind = kinds_counted(&bill.rows);
    let mut groups = Vec::new();
    let mut rows = Vec::new();
    for &(kind, _) in &by_kind {
        let first = rows.len();
        rows.extend(
            bill.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| bom::kind_label(row.kind) == kind && search_finds(row, &search))
                .map(|(index, row)| BomRowView {
                    index,
                    row: row.clone(),
                    located: located == Some(index),
                }),
        );
        let count = rows.len() - first;
        if count == 0 {
            continue;
        }
        let folded = ui.bom_folded.iter().any(|folded| folded == kind);
        groups.push(BomGroup {
            kind,
            count,
            first,
            drawn: if folded { 0 } else { count.min(drawn) },
            folded,
        });
    }
    BomTable {
        total: bill.rows.len(),
        by_kind,
        notices: bill.notices.clone(),
        skipped: bill
            .skipped
            .iter()
            .map(|skipped| SkippedView {
                kind: bom::kind_label(skipped.kind),
                type_code: skipped.type_code.clone(),
                drawing_id: skipped.drawing_id.clone(),
                at: skipped.at,
            })
            .collect(),
        skipped_open: ui.skipped_open,
        groups,
        rows,
        pipe_summary: bill.pipe_summary.clone(),
        summary_open: ui.summary_open,
        across: bill.scope.kind != bom::ScopeKind::Drawing,
        located,
    }
}

/// Whether the lower-cased `search` is in the row's tag, name, spec or type
/// code, in any case (§4.5). An empty search finds every row.
fn search_finds(row: &BomRow, search: &str) -> bool {
    [&row.tag, &row.name, &row.spec, &row.type_code]
        .into_iter()
        .any(|column| column.to_lowercase().contains(search))
}

/// How many of `rows` are of each kind, in the order the kinds first come,
/// as `totals.by_kind` counts them.
fn kinds_counted(rows: &[BomRow]) -> Vec<(&'static str, usize)> {
    let mut counted: Vec<(&'static str, usize)> = Vec::new();
    for kind in rows.iter().map(|row| bom::kind_label(row.kind)) {
        match counted.iter_mut().find(|(known, _)| *known == kind) {
            Some((_, count)) => *count += 1,
            None => counted.push((kind, 1)),
        }
    }
    counted
}

/// What a bill covers, as the command line says it: the drawing, or the
/// project or the node and how many drawings it read — when that is known.
fn bom_scope_said(scope: &BomScope, drawings: usize) -> String {
    let read = if drawings == 0 {
        String::new()
    } else {
        format!(" {drawings} 张图")
    };
    match scope.kind {
        bom::ScopeKind::Drawing => match scope.drawing_id.as_deref() {
            Some(id) if !id.is_empty() => format!("图 {id}"),
            _ => "当前图".to_owned(),
        },
        bom::ScopeKind::Pbs => format!(
            "PBS 节点 {}{read}",
            scope.pbs.as_deref().unwrap_or_default()
        ),
        bom::ScopeKind::Project => format!("全项目{read}"),
    }
}

/// What the material-list tab says instead of a table (§4.5). A bill that
/// did not come out has none: what stopped it is listed instead.
fn bom_empty(bom: &BomView) -> Option<&'static str> {
    match bom.shown.as_ref().map(|shown| &shown.table) {
        None => Some(NEVER_LISTED),
        Some(Err(_)) => None,
        Some(Ok(table)) if table.total == 0 => Some(NOTHING_LISTED),
        Some(Ok(table)) if table.rows.is_empty() => Some(NONE_SEARCHED),
        Some(Ok(_)) => None,
    }
}

/// The snapshot half of 「快照与发布」 (§4.6) of the snapshots and the diff
/// kept on the drawing in front. A drawing never saved has no list, so
/// nothing on it is ticked; of the rows ticked, a snapshot whose file is
/// gone, or which the list does not have, does not count.
fn deliver_view(
    ui: &VerifyUi,
    snapshots: Option<&[SnapshotInfo]>,
    diff: Option<&Diff>,
    publish: &KeptPublish<'_>,
    context: &Context,
) -> DeliverView {
    let saved = context.document_id.is_some() && context.saved;
    let listed = snapshots.filter(|_| saved).unwrap_or_default();
    let tickable = |pick: &Pick| match *pick {
        Pick::Live => saved,
        Pick::Seq(seq) => listed
            .iter()
            .any(|info| info.seq == seq && info.status == SnapshotStatus::Present),
    };
    let picked: Vec<Pick> = ui.picked.iter().copied().filter(tickable).collect();
    let mut rows: Vec<SnapshotRow> = listed
        .iter()
        .map(|info| SnapshotRow {
            info: info.clone(),
            picked: picked.contains(&Pick::Seq(info.seq)),
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.info.seq));
    DeliverView {
        saved,
        reason: ui.snapshot_reason.clone(),
        rows,
        live_picked: picked.contains(&Pick::Live),
        picked,
        diff: diff.filter(|_| saved).map(|diff| diff_view(ui, diff)),
        publish: publish_view(ui, publish, context),
    }
}

/// The publish half (§4.6) of what is kept on the drawing in front: the
/// records as last read — a drawing never saved has none — each package
/// current only while the drawing has no changes unsaved; the four
/// preconditions as they are now; and the question 【发布】 asks, while the
/// drawing can be published.
fn publish_view(ui: &VerifyUi, kept: &KeptPublish<'_>, context: &Context) -> Option<PublishView> {
    context.document_id?;
    let saved = context.saved;
    let preconditions = Preconditions {
        saved,
        clean: !context.dirty,
        file_unchanged: saved && kept.file_unchanged,
        writable_session: context.writable,
    };
    let records = kept.records.filter(|_| saved);
    let listed = records.and_then(Result::ok);
    let packages: Vec<PackageRow> = listed
        .map_or(&[][..], |records| records.packages.as_slice())
        .iter()
        .map(|package| PackageRow {
            package: Package {
                current: package.current && !context.dirty,
                ..package.clone()
            },
            verified: kept
                .verified
                .iter()
                .find(|(revision, _)| *revision == package.revision)
                .map(|(_, verified)| verified.clone()),
        })
        .collect();
    let state = listed.map(|_| match packages.first() {
        None => "未发布".to_owned(),
        Some(latest) if latest.package.current => {
            format!("已发布，修订 {}", latest.package.revision)
        }
        Some(latest) => format!("修订 {} 之后有改动", latest.package.revision),
    });
    let asking = listed
        .filter(|_| ui.confirming && preconditions.held() && context.busy.is_none())
        .map(|records| {
            format!(
                "把 {} 发布为修订 {} 到 {}？",
                records.drawing_id, records.next_revision, records.target_dir
            )
        });
    Some(PublishView {
        state,
        unlisted: records.and_then(Result::err).map(str::to_owned),
        target_dir: listed.map(|records| records.target_dir.clone()),
        next_revision: listed.map(|records| records.next_revision),
        dir: ui.publish_dir.clone(),
        preconditions,
        gates: kept.gates.map(|(found, _)| found.clone()),
        stale: kept.gates.is_some_and(|(_, stale)| stale),
        duplicates_open: ui.duplicates_open,
        packages,
        last_package: kept.last_package.map(str::to_owned),
        asking,
    })
}

/// What the snapshot half says instead of its list (§4.6 item 7).
fn deliver_empty(deliver: &DeliverView, context: &Context) -> Option<&'static str> {
    if context.document_id.is_none() {
        Some(NO_DRAWING_SAID)
    } else if !deliver.saved {
        Some(UNSAVED_SAID)
    } else {
        None
    }
}

/// `diff` as the snapshot half draws it (§4.6): the groups with objects,
/// each drawing 200 of them and 200 more for each 【再显示 200 条】, and
/// what changed on each modified object, a line each.
fn diff_view(ui: &VerifyUi, diff: &Diff) -> DiffView {
    let drawn = ROWS_PER_PAGE.saturating_mul(ui.diff_more.saturating_add(1));
    let groups = DiffGroup::ALL
        .into_iter()
        .filter_map(|group| {
            let objects = group.objects(diff);
            if objects.is_empty() {
                return None;
            }
            let items = objects
                .iter()
                .enumerate()
                .map(|(index, object)| DiffItem {
                    index,
                    label: diff_label(object),
                    changes: object.changes.iter().map(change_said).collect(),
                    located: ui.diff_located == Some((group, index)),
                })
                .collect();
            Some(DiffGroupView {
                group,
                count: objects.len(),
                drawn: drawn.min(objects.len()),
                items,
            })
        })
        .collect();
    DiffView {
        a: Pick::of(&diff.a),
        b: Pick::of(&diff.b),
        identical: diff.identical,
        summary: diff.summary,
        notices: diff.notices.clone(),
        groups,
    }
}

/// An object of a diff as its line reads: kind, type code and tag.
fn diff_label(object: &DiffObject) -> String {
    let tag = if object.tag.is_empty() {
        "（无位号）"
    } else {
        object.tag.as_str()
    };
    format!("{} {} {tag}", object.kind, object.type_code)
}

/// The row of the result at `index`. It can be waived where the session may
/// write, the object is on the drawing in front and it is not waived
/// already, and unwaived on the same terms once it is (§4.8).
fn row_view(index: usize, result: &CheckResult, context: &Context) -> RowView {
    let here = context.drawing_id.as_deref() == Some(result.object.drawing_id.as_str());
    RowView {
        index,
        rule_id: result.rule_id.clone(),
        severity: result.severity,
        object_key: result.object_key.clone(),
        drawing_id: result.object.drawing_id.clone(),
        label: label_of(result),
        message: result.message.clone(),
        fix: result.fix.clone(),
        fields: result.fields.clone(),
        waived: result.waived,
        waiver: result
            .waiver
            .as_ref()
            .map(|waiver| (waiver.reason.clone(), waiver.by.clone(), waiver.at)),
        here,
        can_waive: context.writable && here && !result.waived,
        can_unwaive: context.writable && here && result.waived,
        located: false,
        reason: None,
    }
}

/// A waiver no result matches, as listed. 【删除】 takes it off the drawing
/// in front, so only one that drawing keeps can be (§4.4).
fn stale_waiver(waiver: &Waiver, context: &Context) -> StaleWaiver {
    let kept = context
        .waivers
        .iter()
        .any(|(rule_id, key)| *rule_id == waiver.rule_id && *key == waiver.object_key);
    StaleWaiver {
        rule_id: waiver.rule_id.clone(),
        object_key: waiver.object_key.clone(),
        reason: waiver.reason.clone(),
        by: waiver.by.clone(),
        at: waiver.at,
        can_drop: context.writable && kept,
    }
}

/// The object's name on a row (§3.2 `label`): its tag; without one
/// 「种类（所属）」; with neither, its kind.
fn label_of(result: &CheckResult) -> String {
    let object = &result.object;
    match (object.tag.as_deref(), object.owner.as_deref()) {
        (Some(tag), _) if !tag.is_empty() => tag.to_owned(),
        (_, Some(owner)) if !owner.is_empty() => format!("{}（{owner}）", object.kind),
        _ => object.kind.clone(),
    }
}

/// What a check looked at, as the command line says it.
fn scope_said(scope: &Scope) -> String {
    match scope.kind {
        ScopeKind::Drawing => match scope.drawing_id.as_deref() {
            Some(id) if !id.is_empty() => format!("图 {id}"),
            _ => "当前图".to_owned(),
        },
        ScopeKind::Pbs => format!("PBS 节点 {}", scope.pbs.as_deref().unwrap_or_default()),
        ScopeKind::Project => "全项目".to_owned(),
    }
}

/// Unix seconds as the UTC date and minute, marked UTC, as the snapshots
/// write them: the standard library has no local time zone.
fn utc_said(seconds: i64) -> String {
    let minutes = seconds.rem_euclid(86_400) / 60;
    // Days since 1970-01-01 as a civil date (Howard Hinnant's
    // civil_from_days), written out here: `ui` may not reach up into
    // `patreon` for it.
    let z = seconds.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minutes / 60,
        minutes % 60
    )
}

/// The panel state (§3.2) the `pid_verify_panel` op answers, read off the
/// same view the panel draws, shown or hidden. Each list gives `limit` rows
/// from `offset` on, while its `total` counts them all. `busy` is what runs
/// in the background and how far it is (§4.7), `null` while nothing does.
/// `deliver.publish` stays `null` until the publish half is drawn: it is
/// read from disk and has no empty form.
pub fn panel_state(view: &VerifyView, limit: usize, offset: usize) -> Value {
    json!({
        "ok": true,
        "visible": view.visible,
        "tab": view.tab.key(),
        "document_id": view.document_id,
        "drawing_id": view.drawing_id,
        "writable": view.writable,
        "busy": view.busy.map(busy_state),
        "notice": view.notice.as_ref().map(Notice::state),
        "buttons": view.buttons,
        "check": paged(check_state(&view.check), limit, offset),
        "bom": paged(bom_state(&view.bom), limit, offset),
        "deliver": deliver_state(&view.deliver)
    })
}

/// `busy` (§3.2): a scan of the project folder, what is to be done once it
/// is read, and how far it is; it can always be cancelled.
fn busy_state(busy: Busy) -> Value {
    json!({
        "what": "scan",
        "then": busy.then,
        "text": BUSY_SAID,
        "done": busy.done,
        "total": busy.total,
        "cancellable": true,
    })
}

/// The `check` block (§3.2), read off the rule-check tab as worked out: the
/// counts are the check's own, whatever the filters, and `rows` are all the
/// filters leave, folded groups' included.
fn check_state(check: &CheckView) -> Value {
    let filters = json!({
        "severity": check.severities.iter().map(|severity| severity.name()).collect::<Vec<_>>(),
        "kinds": check.kinds_on,
        "show_waived": check.show_waived,
    });
    let Some(shown) = &check.shown else {
        return json!({
            "has_result": false,
            "stale": false,
            "scope": null,
            "checked_at": null,
            "rule_set_version": null,
            "counts": {"错误": 0, "警告": 0, "提示": 0, "已豁免": 0},
            "filters": filters,
            "kinds": [],
            "notices": [],
            "stale_waivers": 0,
            "total": 0,
            "rows": [],
            "located": null
        });
    };
    let counts = shown.summary;
    json!({
        "has_result": true,
        "stale": shown.stale,
        "scope": scope_key(shown.scope),
        "checked_at": shown.checked_at,
        "rule_set_version": shown.rule_set_version,
        "counts": {
            "错误": counts.errors,
            "警告": counts.warnings,
            "提示": counts.infos,
            "已豁免": counts.waived
        },
        "filters": filters,
        "kinds": shown.kinds,
        "notices": shown.notices,
        "stale_waivers": shown.stale_waivers.len(),
        "total": shown.rows.len(),
        "rows": shown.rows.iter().map(row_state).collect::<Vec<_>>(),
        "located": shown.located
    })
}

/// One row as `check.rows[]` gives it (§3.2).
fn row_state(row: &RowView) -> Value {
    json!({
        "index": row.index,
        "rule_id": row.rule_id,
        "severity": row.severity.name(),
        "object_key": row.object_key,
        "drawing_id": row.drawing_id,
        "label": row.label,
        "message": row.message,
        "fields": row.fields,
        "waived": row.waived,
        "can_waive": row.can_waive
    })
}

/// The scope as `check.scope` names it.
fn scope_key(kind: ScopeKind) -> &'static str {
    match kind {
        ScopeKind::Drawing => "drawing",
        ScopeKind::Pbs => "pbs",
        ScopeKind::Project => "project",
    }
}

/// The `bom` block (§3.2), read off the material-list tab as worked out:
/// `rows` are all the search leaves, folded groups' included, each as the
/// bill's `rows[]` writes it with its index. A bill that did not come out
/// says why in `error`, with no rows.
fn bom_state(bom: &BomView) -> Value {
    let Some(shown) = &bom.shown else {
        return json!({
            "has_result": false,
            "stale": false,
            "scope": null,
            "search": bom.search,
            "error": null,
            "total": 0,
            "groups": [],
            "rows": []
        });
    };
    let (error, groups, rows): (Value, Vec<Value>, Vec<Value>) = match &shown.table {
        Ok(table) => (
            Value::Null,
            table
                .groups
                .iter()
                .map(|group| {
                    json!({ "kind": group.kind, "rows": group.count, "collapsed": group.folded })
                })
                .collect(),
            table.rows.iter().map(bom_row_state).collect(),
        ),
        Err(failed) => (
            json!({ "code": failed.code, "text": failed.text, "duplicates": failed.duplicates }),
            Vec::new(),
            Vec::new(),
        ),
    };
    json!({
        "has_result": true,
        "stale": shown.stale,
        "scope": bom_scope_key(shown.scope),
        "search": bom.search,
        "error": error,
        "total": rows.len(),
        "groups": groups,
        "rows": rows
    })
}

/// One row as `bom.rows[]` gives it (§3.2): the bill's row and its index.
fn bom_row_state(row: &BomRowView) -> Value {
    let mut state = serde_json::to_value(&row.row).unwrap_or_default();
    state["index"] = json!(row.index);
    state
}

/// The scope as `bom.scope` names it.
fn bom_scope_key(kind: bom::ScopeKind) -> &'static str {
    match kind {
        bom::ScopeKind::Drawing => "drawing",
        bom::ScopeKind::Pbs => "pbs",
        bom::ScopeKind::Project => "project",
    }
}

/// The `deliver` block (§3.2), read off the snapshot half as worked out:
/// the snapshots newest first, each as `pid_snapshots` lists it and
/// whether it is ticked; the rows ticked, in the order they were; the diff
/// shown, by its sides and counts; the publish half, as [`publish_state`]
/// reads it.
fn deliver_state(deliver: &DeliverView) -> Value {
    let snapshots: Vec<Value> = deliver
        .rows
        .iter()
        .map(|row| {
            let mut said = json!(row.info);
            said["picked"] = json!(row.picked);
            said
        })
        .collect();
    let picked: Vec<Value> = deliver.picked.iter().copied().map(Pick::state).collect();
    let diff = deliver.diff.as_ref().map(|diff| {
        json!({
            "a": diff.a.state(),
            "b": diff.b.state(),
            "identical": diff.identical,
            "新增": diff.summary.added,
            "删除": diff.summary.removed,
            "修改": diff.summary.modified,
        })
    });
    json!({
        "snapshots": snapshots,
        "picked": picked,
        "diff": diff,
        "publish": publish_state(deliver.publish.as_ref()),
    })
}

/// The `publish` block of `deliver` (§3.2), read off the publish half as
/// worked out; `null` with no drawing in front. Before 【检查发布条件】 the
/// gates, and whether the drawing could be published, are not known.
fn publish_state(publish: Option<&PublishView>) -> Value {
    let Some(publish) = publish else {
        return Value::Null;
    };
    let packages: Vec<Value> = publish
        .packages
        .iter()
        .map(|row| {
            let package = &row.package;
            let verified = row
                .verified
                .as_ref()
                .map(|verified| json!({ "intact": verified.intact, "text": verified.text }));
            json!({
                "package_id": package.package_id,
                "revision": package.revision,
                "path": package.path,
                "published_at": package.published_at,
                "published_by": package.published_by,
                "current": package.current,
                "verified": verified,
            })
        })
        .collect();
    let found = publish.gates.as_ref();
    json!({
        "state": publish.state,
        "target_dir": publish.target_dir,
        "next_revision": publish.next_revision,
        "preconditions": publish.preconditions,
        "checked": found.is_some(),
        "stale": publish.stale,
        "can_publish": found.map(|found| found.can_publish),
        "gates": found.map(|found| &found.gates),
        "packages": packages,
        "last_package": publish.last_package,
    })
}

/// `block` with its `rows` cut to `limit` of them from `offset` on.
fn paged(mut block: Value, limit: usize, offset: usize) -> Value {
    if let Some(rows) = block.get_mut("rows").and_then(Value::as_array_mut) {
        let page: Vec<Value> = rows.drain(..).skip(offset).take(limit).collect();
        *rows = page;
    }
    block
}

/// Why an action (§3.3) pressed nothing (§3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// The button is grey now; `reason` is one of §3.7's (`busy`,
    /// `readonly`, `no_result` …).
    Unavailable { reason: &'static str },
    /// The row index, snapshot number or revision named is not in the
    /// panel's list now; the text says which.
    RowNotFound(String),
    /// The key is not one the panel has, or its value is not written the
    /// way the key takes it; the text says what to write.
    InvalidValue(String),
}

impl PanelError {
    /// The code the interface answers with.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "PANEL_ACTION_UNAVAILABLE",
            Self::RowNotFound(_) => "PANEL_ROW_NOT_FOUND",
            Self::InvalidValue(_) => "invalid_value",
        }
    }

    /// Why the button is grey; only `PANEL_ACTION_UNAVAILABLE` says.
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Unavailable { reason } => Some(*reason),
            Self::RowNotFound(_) | Self::InvalidValue(_) => None,
        }
    }
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { reason } => write!(f, "按钮现在是灰的，没有按下去（{reason}）"),
            Self::RowNotFound(said) | Self::InvalidValue(said) => f.write_str(said),
        }
    }
}

/// The keys of [`parse_action`] that take no value.
const BARE_KEYS: [&str; 6] = ["show", "hide", "diff", "publish_check", "publish", "cancel"];

/// The `value` of an `action` op named `pid_verify` (§3.3), one key or
/// key=value, as the message pressing that control in the panel sends.
/// `view` is what the panel draws now, so a key whose button is grey
/// there, or whose row is not in its list, is refused rather than pressed.
/// A value is read in this order: how it is written, then whether its
/// button is grey, then whether what it names is in the panel's lists. A
/// row's own buttons are grey or not by the row, so for `waive` and
/// `unwaive` that comes last: a session that may not write is refused
/// first, then a row the list does not have, then a row whose button is
/// grey. `pick` the same way: a drawing never saved has no list to tick
/// in, a number the list does not have is no row of it, and a snapshot
/// whose file is gone cannot be ticked. `publish` is 【确认】 under the
/// question 【发布】 asks: it publishes without asking. `cancel` is 【取消】
/// on the progress line, refused (`not_busy`) while nothing runs.
pub fn parse_action(value: &str, view: &VerifyView) -> Result<PidVerifyMsg, PanelError> {
    let (key, written) = match value.split_once('=') {
        Some((key, written)) => (key, Some(written)),
        None => (value, None),
    };
    let invalid =
        |write: &str| PanelError::InvalidValue(format!("{key} 写{write}，实得 \"{value}\""));
    let shown = view.check.shown.as_ref();
    let checked = || {
        shown.ok_or(PanelError::Unavailable {
            reason: "no_result",
        })
    };
    let listed = || {
        view.bom
            .shown
            .as_ref()
            .and_then(|shown| shown.table.as_ref().ok())
            .ok_or(PanelError::Unavailable {
                reason: "no_result",
            })
    };
    let compared = || {
        view.deliver.diff.as_ref().ok_or(PanelError::Unavailable {
            reason: "no_result",
        })
    };
    let pressable = |button: Button| match view.grey(button) {
        Some(reason) => Err(PanelError::Unavailable { reason }),
        None => Ok(()),
    };
    let unsaved = || {
        let reason = if view.document_id.is_none() {
            "no_drawing"
        } else {
            "no_project_dir"
        };
        PanelError::Unavailable { reason }
    };
    if written.is_some() && BARE_KEYS.contains(&key) {
        return Err(PanelError::InvalidValue(format!(
            "{key} 不带值，实得 \"{value}\""
        )));
    }
    match key {
        "show" => Ok(PidVerifyMsg::Show),
        "hide" => Ok(PidVerifyMsg::Hide),
        "tab" => {
            let name = written.unwrap_or_default();
            Tab::from_key(name).map(PidVerifyMsg::Tab).ok_or_else(|| {
                PanelError::InvalidValue(format!(
                    "tab 写 check、bom、deliver 中的一个，实得 \"{name}\""
                ))
            })
        }
        "check" | "bom" => {
            let (pressed, button) = match (key, written) {
                ("check", Some("drawing")) => (PidVerifyMsg::CheckDrawing, Button::CheckDrawing),
                ("check", Some("project")) => (PidVerifyMsg::CheckProject, Button::CheckProject),
                (_, Some("drawing")) => (PidVerifyMsg::BomDrawing, Button::BomDrawing),
                (_, Some("project")) => (PidVerifyMsg::BomProject, Button::BomProject),
                _ => return Err(invalid(" drawing 或 project")),
            };
            pressable(button).map(|()| pressed)
        }
        "severity" => {
            let severity = written
                .and_then(Severity::from_name)
                .ok_or_else(|| invalid(" 错误、警告、提示 中的一个"))?;
            checked()?;
            Ok(PidVerifyMsg::Severity(severity))
        }
        "kind" => {
            let kind = written.ok_or_else(|| invalid("成 kind=<种类>，或者 kind= 清掉种类筛选"))?;
            let found = &checked()?.kinds;
            if kind.is_empty() {
                return Ok(PidVerifyMsg::Kind(None));
            }
            if !found.iter().any(|known| known == kind) {
                return Err(PanelError::InvalidValue(format!(
                    "这次检查的结果里没有种类「{kind}」；有的是：{}",
                    found.join("、")
                )));
            }
            Ok(PidVerifyMsg::Kind(Some(kind.to_owned())))
        }
        "show_waived" => {
            let on = match written {
                Some("1") => true,
                Some("0") => false,
                _ => return Err(invalid(" 1 或 0")),
            };
            checked()?;
            Ok(PidVerifyMsg::ShowWaived(on))
        }
        "check_export" => {
            let path = written
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| invalid("成 check_export=<文件>"))?;
            pressable(Button::CheckExport)?;
            Ok(PidVerifyMsg::Export(path.to_owned()))
        }
        "more" => match written {
            Some("check") => checked().map(|_| PidVerifyMsg::MoreChecks),
            Some("bom") => listed().map(|_| PidVerifyMsg::MoreBom),
            Some("diff") => compared().map(|_| PidVerifyMsg::MoreDiff),
            _ => Err(invalid(" check、bom 或 diff（那个列表再显示 200 条）")),
        },
        "bom_search" => {
            let search = written
                .ok_or_else(|| invalid("成 bom_search=<文字>，或者 bom_search= 清掉搜索"))?;
            listed()?;
            Ok(PidVerifyMsg::BomSearch(search.to_owned()))
        }
        "bom_group" => {
            let kind = written
                .filter(|kind| !kind.is_empty())
                .ok_or_else(|| invalid("成 bom_group=<种类>"))?;
            let kinds: Vec<&str> = listed()?.by_kind.iter().map(|(known, _)| *known).collect();
            if !kinds.contains(&kind) {
                return Err(PanelError::InvalidValue(format!(
                    "清单里没有种类「{kind}」的组；有的是：{}",
                    kinds.join("、")
                )));
            }
            Ok(PidVerifyMsg::BomFold(kind.to_owned()))
        }
        "bom_locate" => {
            let index = written
                .and_then(|number| number.trim().parse::<usize>().ok())
                .ok_or_else(|| invalid("成 bom_locate=<下标>"))?;
            if !listed()?.rows.iter().any(|row| row.index == index) {
                return Err(PanelError::RowNotFound(format!(
                    "材料清单的列表里现在没有下标 {index} 的一行（搜索筛掉的不在列表里）"
                )));
            }
            Ok(PidVerifyMsg::BomLocate(index))
        }
        "bom_export" => {
            let path = written
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| invalid("成 bom_export=<文件>"))?;
            pressable(Button::BomExport)?;
            Ok(PidVerifyMsg::BomExport(path.to_owned()))
        }
        "snapshot_save" => {
            let reason = written
                .ok_or_else(|| invalid("成 snapshot_save=<原因>，或者 snapshot_save= 不填原因"))?;
            pressable(Button::SnapshotSave)?;
            Ok(PidVerifyMsg::SnapshotSave(reason.to_owned()))
        }
        "pick" => {
            let form = "成 pick=<序号> 或 pick=live";
            let pick = match written.map(str::trim) {
                Some(live) if live.eq_ignore_ascii_case("live") => Pick::Live,
                Some(number) => number
                    .parse::<u64>()
                    .map(Pick::Seq)
                    .map_err(|_| invalid(form))?,
                None => return Err(invalid(form)),
            };
            let deliver = &view.deliver;
            if !deliver.saved {
                let reason = if view.document_id.is_none() {
                    "no_drawing"
                } else {
                    "no_project_dir"
                };
                return Err(PanelError::Unavailable { reason });
            }
            if let Pick::Seq(seq) = pick {
                let row = deliver
                    .rows
                    .iter()
                    .find(|row| row.info.seq == seq)
                    .ok_or_else(|| {
                        PanelError::RowNotFound(format!("快照列表里现在没有第 {seq} 份"))
                    })?;
                if row.missing() {
                    return Err(PanelError::Unavailable { reason: "missing" });
                }
            }
            Ok(PidVerifyMsg::Pick(pick))
        }
        "diff" => view
            .deliver
            .pair()
            .map(|(a, b)| PidVerifyMsg::Diff(a, b))
            .ok_or(PanelError::Unavailable {
                reason: "not_two_picked",
            }),
        "diff_locate" => {
            let (group, index) = written
                .and_then(|written| written.split_once(':'))
                .and_then(|(group, number)| {
                    let group = DiffGroup::from_key(group.trim())?;
                    Some((group, number.trim().parse::<usize>().ok()?))
                })
                .ok_or_else(|| {
                    invalid("成 diff_locate=<组>:<下标>，组是 added、removed、modified 中的一个")
                })?;
            let count = compared()?
                .groups
                .iter()
                .find(|shown| shown.group == group)
                .map_or(0, |shown| shown.count);
            if index >= count {
                return Err(PanelError::RowNotFound(format!(
                    "比对结果的「{}」组里现在没有下标 {index} 的一项",
                    group.label()
                )));
            }
            Ok(PidVerifyMsg::DiffLocate(group, index))
        }
        "diff_export" => {
            let path = written
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .ok_or_else(|| invalid("成 diff_export=<文件>"))?;
            pressable(Button::DiffExport)?;
            Ok(PidVerifyMsg::DiffExport(path.to_owned()))
        }
        "publish_dir" => {
            let dir = written
                .ok_or_else(|| invalid("成 publish_dir=<目录>，或者 publish_dir= 用默认目录"))?;
            if !view.deliver.saved {
                return Err(unsaved());
            }
            Ok(PidVerifyMsg::PublishDir(dir.trim().to_owned()))
        }
        "publish_check" => pressable(Button::PublishCheck).map(|()| PidVerifyMsg::PublishCheck),
        "publish" => pressable(Button::Publish).map(|()| PidVerifyMsg::Publish),
        "package_verify" => {
            let revision = written
                .and_then(|number| number.trim().parse::<u32>().ok())
                .ok_or_else(|| invalid("成 package_verify=<修订号>"))?;
            pressable(Button::PackageVerify)?;
            let packages = view
                .deliver
                .publish
                .as_ref()
                .map(|publish| publish.packages.as_slice())
                .unwrap_or_default();
            if !packages.iter().any(|row| row.package.revision == revision) {
                return Err(PanelError::RowNotFound(format!(
                    "发布记录里现在没有修订 {revision} 的包"
                )));
            }
            Ok(PidVerifyMsg::PackageVerify(revision))
        }
        "locate" | "waive" | "unwaive" => {
            let form = match key {
                "locate" => "成 locate=<下标>",
                "waive" => "成 waive=<下标>:<理由>",
                _ => "成 unwaive=<下标>",
            };
            let written = written.unwrap_or_default();
            let (number, reason) = match written.split_once(':') {
                Some((number, reason)) if key == "waive" => (number, reason),
                _ if key == "waive" => return Err(invalid(form)),
                _ => (written, ""),
            };
            let index = number.trim().parse::<usize>().map_err(|_| invalid(form))?;
            let shown = checked()?;
            if key != "locate" && !view.writable {
                return Err(PanelError::Unavailable { reason: "readonly" });
            }
            let row = shown
                .rows
                .iter()
                .find(|row| row.index == index)
                .ok_or_else(|| {
                    PanelError::RowNotFound(format!(
                        "规则检查的列表里现在没有下标 {index} 的一行（筛掉的和没显示的已豁免结果不在列表里）"
                    ))
                })?;
            if key == "locate" {
                return Ok(PidVerifyMsg::Locate(index));
            }
            match row.grey(view.writable, key == "unwaive") {
                Some(grey) => Err(PanelError::Unavailable { reason: grey }),
                None if key == "waive" => Ok(PidVerifyMsg::Waive(index, reason.to_owned())),
                None => Ok(PidVerifyMsg::Unwaive(index)),
            }
        }
        "cancel" => view
            .busy
            .map(|_| PidVerifyMsg::Cancel)
            .ok_or(PanelError::Unavailable { reason: "not_busy" }),
        _ => Err(PanelError::InvalidValue(format!(
            "面板没有 \"{key}\" 这个键"
        ))),
    }
}

fn muted(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.65)),
    }
}

fn warning(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().warning.base.color),
    }
}

fn danger(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.palette().danger.base.color),
    }
}

/// Build the docked panel: the tab strip, then the tab in front.
pub fn view<'a>(view: &'a VerifyView, width: f32, chrome: Chrome) -> Element<'a, Message> {
    let title_bar = crate::ui::dock::title_bar(
        PanelId::PidVerifyDeliver,
        PanelId::PidVerifyDeliver.title().to_string(),
        chrome,
    );
    let mut tabs = row![].spacing(2);
    for tab in Tab::ALL {
        let selected = tab == view.tab;
        tabs = tabs.push(PosReport::new(
            tab.bounds_id(),
            button(text(tab.label()).size(11))
                .on_press(PidVerifyMsg::Tab(tab).message())
                .padding([3, 8])
                .style(move |theme: &Theme, status| {
                    if selected {
                        button::primary(theme, status)
                    } else {
                        button::secondary(theme, status)
                    }
                }),
        ));
    }
    let content = match view.tab {
        Tab::Check => check_tab(view),
        Tab::Bom => bom_tab(view),
        Tab::Deliver => deliver_tab(view),
    };
    let mut body = column![tabs].spacing(6).height(Fill);
    if let Some(busy) = view.busy {
        body = body.push(busy_line(busy));
    }
    let reported = PosReport::new(PANEL_BOUNDS_ID, body.push(content));
    crate::ui::dock::frame(title_bar, reported, width)
}

/// The progress line (§4.7): what is read and how far, the bar, and
/// 【取消】.
fn busy_line<'a>(busy: Busy) -> Element<'a, Message> {
    let said = format!("{BUSY_SAID} {} / {}", busy.done, busy.total);
    let range = 0.0..=busy.total.max(1) as f32;
    let bar = progress_bar(range, busy.done as f32).girth(6.0);
    row![
        text(said).size(11),
        PosReport::new(BUSY_ID, bar),
        PosReport::new(CANCEL_ID, small_button("取消", Some(PidVerifyMsg::Cancel))),
    ]
    .spacing(6)
    .align_y(iced::Center)
    .into()
}

/// What a tab says instead of a list, in the middle of it.
fn empty_text(said: &str) -> Element<'_, Message> {
    container(text(said).size(12).style(muted))
        .center_x(Fill)
        .center_y(Fill)
        .width(Fill)
        .height(Fill)
        .into()
}

/// The rule-check tab (§4.2): its buttons and the message bar, then what
/// the check shown found — or what the tab says instead.
fn check_tab(view: &VerifyView) -> Element<'_, Message> {
    let grey = |button| view.grey(button).map(grey_said);
    let actions = row![
        PosReport::new(
            CHECK_DRAWING_ID,
            greyable(
                "检查当前图",
                Some(PidVerifyMsg::CheckDrawing),
                grey(Button::CheckDrawing)
            ),
        ),
        PosReport::new(
            CHECK_PROJECT_ID,
            greyable(
                "检查全项目",
                Some(PidVerifyMsg::CheckProject),
                grey(Button::CheckProject)
            ),
        ),
        greyable(
            "导出…",
            Some(PidVerifyMsg::PickExport),
            grey(Button::CheckExport)
        ),
    ]
    .spacing(4);
    let mut tab = column![actions].spacing(6).height(Fill);
    if let Some(notice) = &view.notice {
        tab = tab.push(notice_line(notice));
    }
    let check = &view.check;
    let Some(shown) = &check.shown else {
        return tab
            .push(empty_text(view.empty.unwrap_or(NEVER_CHECKED)))
            .into();
    };
    tab = tab.push(text(status_line(shown)).size(11).style(muted));
    if shown.stale {
        tab = tab.push(text(STALE).size(11).style(warning));
    }
    let counts = shown.summary;
    let mut severities = row![].spacing(4);
    for (severity, count) in [
        (Severity::Error, counts.errors),
        (Severity::Warning, counts.warnings),
        (Severity::Info, counts.infos),
    ] {
        severities = severities.push(tag(
            format!("{} {count}", severity.name()),
            check.severities.contains(&severity),
            PidVerifyMsg::Severity(severity),
        ));
    }
    severities = severities.push(tag(
        format!("显示已豁免（{}）", counts.waived),
        check.show_waived,
        PidVerifyMsg::ShowWaived(!check.show_waived),
    ));
    tab = tab.push(severities.wrap().vertical_spacing(4));
    if !shown.kinds.is_empty() {
        let mut kinds = row![].spacing(4);
        for kind in &shown.kinds {
            kinds = kinds.push(tag(
                kind.clone(),
                check.kinds_on.contains(kind),
                PidVerifyMsg::Kind(Some(kind.clone())),
            ));
        }
        tab = tab.push(kinds.wrap().vertical_spacing(4));
    }
    for notice in &shown.notices {
        tab = tab.push(text(notice.message.as_str()).size(11).style(muted));
    }
    if !shown.stale_waivers.is_empty() {
        tab = tab.push(stale_waivers(shown, view.writable));
    }
    match view.empty {
        Some(said) => tab.push(empty_text(said)).into(),
        None => tab
            .push(scrollable(check_list(shown, view.writable)).height(Fill))
            .into(),
    }
}

/// 「有 N 条豁免已经对不上」, which lists them or folds the list, and each
/// listed with 【删除】: grey, saying why, in a session that may not write
/// or on a waiver another drawing keeps.
fn stale_waivers(shown: &Shown, writable: bool) -> Element<'_, Message> {
    let arrow = if shown.stale_open { "▼" } else { "▶" };
    let header = button(
        text(format!(
            "{arrow} 有 {} 条豁免已经对不上",
            shown.stale_waivers.len()
        ))
        .size(11)
        .style(warning),
    )
    .on_press(PidVerifyMsg::StaleWaivers.message())
    .padding([2, 4])
    .style(button::text);
    let mut list = column![header].spacing(2);
    if !shown.stale_open {
        return list.into();
    }
    for waiver in &shown.stale_waivers {
        let drop = PidVerifyMsg::DropWaiver {
            rule_id: waiver.rule_id.clone(),
            object_key: waiver.object_key.clone(),
        };
        let tip = match (waiver.can_drop, writable) {
            (true, _) => None,
            (false, true) => Some("这条豁免记在别的图上；到那张图上再删"),
            (false, false) => Some(grey_said("readonly")),
        };
        let said = format!(
            "{} {} · {}（{}，{}）",
            waiver.rule_id,
            waiver.object_key,
            waiver.reason,
            waiver.by,
            utc_said(waiver.at)
        );
        list = list.push(
            row![
                text(said).size(11).style(muted).width(Fill),
                greyable("删除", Some(drop), tip)
            ]
            .spacing(4)
            .align_y(iced::Center),
        );
    }
    list.into()
}

/// The status line (§4.2): scope, when, rule version, how long.
fn status_line(shown: &Shown) -> String {
    format!(
        "{} · {} · 规则 {} · 用时 {} ms",
        shown.scope_said,
        utc_said(shown.checked_at),
        shown.rule_set_version,
        shown.elapsed_ms
    )
}

/// The message bar: an error in the error colour with its code, news
/// muted.
fn notice_line(notice: &Notice) -> Element<'_, Message> {
    let said = match &notice.code {
        Some(code) if !notice.text.contains(code.as_str()) => format!("{code}：{}", notice.text),
        _ => notice.text.clone(),
    };
    let style: fn(&Theme) -> iced::widget::text::Style = if notice.error { danger } else { muted };
    text(said).size(11).style(style).into()
}

/// A filter tag, drawn pressed while it is on.
fn tag<'a>(label: String, on: bool, press: PidVerifyMsg) -> Element<'a, Message> {
    button(text(label).size(11))
        .on_press(press.message())
        .padding([2, 6])
        .style(move |theme: &Theme, status| {
            if on {
                button::primary(theme, status)
            } else {
                button::secondary(theme, status)
            }
        })
        .into()
}

/// One of the panel's buttons, grey while `press` is `None`.
fn small_button(label: &str, press: Option<PidVerifyMsg>) -> Element<'_, Message> {
    button(text(label).size(11))
        .on_press_maybe(press.map(PidVerifyMsg::message))
        .padding([3, 8])
        .style(button::secondary)
        .into()
}

/// [`small_button`] sending `press`, grey with a tip saying why while `tip`
/// says something (§4.8).
fn greyable<'a>(
    label: &'a str,
    press: Option<PidVerifyMsg>,
    tip: Option<&'static str>,
) -> Element<'a, Message> {
    match tip {
        Some(tip) => tooltip(small_button(label, None), tip, TipPosition::Bottom),
        None => small_button(label, press),
    }
}

/// The list (§4.2): each group's header, which folds it, its rows drawn,
/// and 【再显示 200 条】 while it has more.
fn check_list(shown: &Shown, writable: bool) -> Element<'_, Message> {
    let mut list = column![].spacing(4).width(Fill);
    for group in &shown.groups {
        let arrow = if group.folded { "▶" } else { "▼" };
        list = list.push(
            button(
                text(format!(
                    "{arrow} {}（{}）",
                    group.group.label(),
                    group.count
                ))
                .size(12),
            )
            .on_press(PidVerifyMsg::Fold(group.group).message())
            .padding([2, 4])
            .style(button::text),
        );
        for row in &shown.rows[group.first..group.first + group.drawn] {
            list = list.push(PosReport::owned(row.bounds_id(), result_row(row, writable)));
        }
        if group.has_more() {
            list = list.push(small_button(
                "再显示 200 条",
                Some(PidVerifyMsg::MoreChecks),
            ));
        }
    }
    list.into()
}

/// One result (§4.2): rule and object, the message, the fix in a lighter
/// colour, and on a waived row why, by whom and when. Clicking it finds its
/// object (§4.3); at its end 【豁免】 or 【取消豁免】, and under it the reason
/// box while that is open (§4.4).
fn result_row(row: &RowView, writable: bool) -> Element<'_, Message> {
    let mut lines = column![
        text(row.headline()).size(12),
        text(row.message.as_str()).size(12)
    ]
    .spacing(1);
    if !row.fix.is_empty() {
        lines = lines.push(text(row.fix.as_str()).size(11).style(muted));
    }
    if let Some((reason, by, at)) = &row.waiver {
        lines = lines.push(
            text(format!("豁免：{reason}（{by}，{}）", utc_said(*at)))
                .size(11)
                .style(muted),
        );
    }
    let located = row.located;
    let body = button(lines)
        .on_press(PidVerifyMsg::Locate(row.index).message())
        .padding([2, 8])
        .width(Fill)
        .style(move |theme: &Theme, status| {
            if located {
                button::secondary(theme, status)
            } else {
                button::text(theme, status)
            }
        });
    let end = if row.waived {
        let tip = row.grey(writable, true).map(grey_said);
        greyable("取消豁免", Some(PidVerifyMsg::Unwaive(row.index)), tip)
    } else {
        let tip = match row.grey(writable, false) {
            Some(reason) => Some(grey_said(reason)),
            None => row.reason.is_some().then_some(WAIVING_SAID),
        };
        greyable("豁免", Some(PidVerifyMsg::AskWaiver(row.index)), tip)
    };
    let line = row![body, end].spacing(4).align_y(iced::Center);
    let Some(reason) = &row.reason else {
        return line.into();
    };
    let confirm = PidVerifyMsg::Waive(row.index, reason.clone());
    let hint = format!("豁免理由（必填，最多 {WAIVER_REASON_LIMIT} 字）");
    let reason_box = row![
        text_input(&hint, reason)
            .id(iced::widget::Id::new(REASON_ID))
            .on_input(|typed| PidVerifyMsg::WaiverReason(typed).message())
            .on_submit(confirm.clone().message())
            .size(12)
            .padding([3, 6])
            .width(Fill),
        small_button("确认", Some(confirm)),
        small_button("取消", Some(PidVerifyMsg::CancelWaiver))
    ]
    .spacing(4)
    .align_y(iced::Center);
    column![line, container(reason_box).padding([0, 8])]
        .spacing(2)
        .into()
}

/// The material-list tab (§4.5): its buttons and the message bar, the
/// status line, then the bill shown — the search box, the notices, the list
/// and 「管道汇总」, with the totals and 【导出…】 under them — or what
/// stopped it, or what the tab says instead.
fn bom_tab(view: &VerifyView) -> Element<'_, Message> {
    let grey = |button| view.grey(button).map(grey_said);
    let actions = row![
        PosReport::new(
            BOM_DRAWING_ID,
            greyable(
                "当前图",
                Some(PidVerifyMsg::BomDrawing),
                grey(Button::BomDrawing)
            ),
        ),
        PosReport::new(
            BOM_PROJECT_ID,
            greyable(
                "全项目",
                Some(PidVerifyMsg::BomProject),
                grey(Button::BomProject)
            ),
        ),
    ]
    .spacing(4);
    let mut tab = column![actions].spacing(6).height(Fill);
    if let Some(notice) = &view.notice {
        tab = tab.push(notice_line(notice));
    }
    let Some(shown) = &view.bom.shown else {
        return tab
            .push(empty_text(view.empty.unwrap_or(NEVER_LISTED)))
            .into();
    };
    tab = tab.push(text(bom_status_line(shown)).size(11).style(muted));
    if shown.stale {
        tab = tab.push(text(BOM_STALE).size(11).style(warning));
    }
    let table = match &shown.table {
        Ok(table) => table,
        Err(failed) => {
            return tab
                .push(scrollable(bom_failure(failed)).height(Fill))
                .into()
        }
    };
    tab = tab.push(PosReport::new(
        BOM_SEARCH_ID,
        text_input("搜索位号、名称、规格、类型编码", &view.bom.search)
            .on_input(|typed| PidVerifyMsg::BomSearch(typed).message())
            .size(12)
            .padding([3, 6])
            .width(Fill),
    ));
    for notice in &table.notices {
        tab = tab.push(text(notice.message.as_str()).size(11).style(muted));
    }
    let mut body = column![].spacing(4).width(Fill);
    if !table.skipped.is_empty() {
        body = body.push(skipped_list(table));
    }
    body = match view.empty {
        Some(said) => body.push(text(said).size(12).style(muted)),
        None => body.push(bom_list(table)),
    };
    if !table.pipe_summary.is_empty() {
        body = body.push(pipe_summary(table));
    }
    let bottom = row![
        text(bom_totals(table)).size(11).style(muted).width(Fill),
        PosReport::new(
            BOM_EXPORT_ID,
            greyable(
                "导出…",
                Some(PidVerifyMsg::PickBomExport),
                grey(Button::BomExport)
            ),
        ),
    ]
    .spacing(4)
    .align_y(iced::Center);
    tab.push(scrollable(body).height(Fill)).push(bottom).into()
}

/// The status line (§4.5): scope, when, how many rows.
fn bom_status_line(shown: &BomShown) -> String {
    let rows = match &shown.table {
        Ok(table) => format!("{} 行", table.total),
        Err(_) => "没有生成".to_owned(),
    };
    format!(
        "{} · {} · {rows}",
        shown.scope_said,
        utc_said(shown.made_at)
    )
}

/// What stopped the bill, in place of its table (§4.5): the tags found in
/// more than one place, each place clicked to look at it, or the drawings
/// of the scope that did not read and why.
fn bom_failure(failed: &BomFailed) -> Element<'_, Message> {
    let said = if failed.code == bom::BOM_DUPLICATE_TAG {
        DUPLICATE_SAID
    } else {
        UNREADABLE_SAID
    };
    let mut list = column![text(said).size(12).style(danger)]
        .spacing(2)
        .width(Fill);
    for (repeated, duplicate) in failed.duplicates.iter().enumerate() {
        list = list.push(
            text(format!(
                "{} {}（{} 处）",
                duplicate.type_code,
                duplicate.tag,
                duplicate.places.len()
            ))
            .size(12),
        );
        for (at, place) in duplicate.places.iter().enumerate() {
            list = list.push(place_button(
                format!("{} · {}", place.drawing_id, point_said(place.at)),
                PidVerifyMsg::BomDuplicateLocate(repeated, at),
            ));
        }
    }
    for drawing in &failed.unreadable {
        let file = drawing
            .file
            .as_deref()
            .map(|file| format!("（{file}）"))
            .unwrap_or_default();
        list = list.push(
            text(format!("{}{file}：{}", drawing.drawing_id, drawing.why))
                .size(11)
                .style(muted),
        );
    }
    list.into()
}

/// 「有 N 个对象没有位号，没有计入」, which lists them or folds the list;
/// each listed is clicked to look at where it is.
fn skipped_list(table: &BomTable) -> Element<'_, Message> {
    let arrow = if table.skipped_open { "▼" } else { "▶" };
    let header = button(
        text(format!(
            "{arrow} 有 {} 个对象没有位号，没有计入",
            table.skipped.len()
        ))
        .size(11)
        .style(warning),
    )
    .on_press(PidVerifyMsg::BomSkipped.message())
    .padding([2, 4])
    .style(button::text);
    let mut list = column![header].spacing(2);
    if !table.skipped_open {
        return list.into();
    }
    for (at, skipped) in table.skipped.iter().enumerate() {
        list = list.push(place_button(
            format!(
                "{} {} · {} · {}",
                skipped.kind,
                skipped.type_code,
                skipped.drawing_id,
                point_said(skipped.at)
            ),
            PidVerifyMsg::BomSkippedLocate(at),
        ));
    }
    list.into()
}

/// A place to look at, under what it is a place of, clicked like a row.
fn place_button<'a>(label: String, press: PidVerifyMsg) -> Element<'a, Message> {
    button(text(label).size(11))
        .on_press(press.message())
        .padding([1, 16])
        .style(button::text)
        .into()
}

/// A point on the drawing, to the drawing unit.
fn point_said(at: [f64; 2]) -> String {
    format!("({:.0}, {:.0})", at[0], at[1])
}

/// The list (§4.5): each kind's header, which folds it, its rows drawn, and
/// 【再显示 200 条】 while it has more.
fn bom_list(table: &BomTable) -> Element<'_, Message> {
    let mut list = column![].spacing(4).width(Fill);
    for group in &table.groups {
        let arrow = if group.folded { "▶" } else { "▼" };
        list = list.push(
            button(text(format!("{arrow} {}（{}）", group.kind, group.count)).size(12))
                .on_press(PidVerifyMsg::BomFold(group.kind.to_owned()).message())
                .padding([2, 4])
                .style(button::text),
        );
        for row in &table.rows[group.first..group.first + group.drawn] {
            list = list.push(PosReport::owned(
                row.bounds_id(),
                bill_row(row, table.across),
            ));
        }
        if group.has_more() {
            list = list.push(small_button("再显示 200 条", Some(PidVerifyMsg::MoreBom)));
        }
    }
    list.into()
}

/// One row (§4.5): tag, name and quantity, and under it in a lighter colour
/// the spec, with the drawings it is on in a bill of more than one drawing.
/// Clicking it finds its object.
fn bill_row(row: &BomRowView, across: bool) -> Element<'_, Message> {
    let first = row![
        text(format!("{} {}", row.row.tag, row.row.name))
            .size(12)
            .width(Fill),
        text(row.quantity()).size(12)
    ]
    .spacing(8);
    let mut second = row.row.spec.clone();
    if across {
        let drawings = row.row.drawings.join("、");
        second = if second.is_empty() {
            drawings
        } else {
            format!("{second} · {drawings}")
        };
    }
    let mut lines = column![first].spacing(1);
    if !second.is_empty() {
        lines = lines.push(text(second).size(11).style(muted));
    }
    let located = row.located;
    button(lines)
        .on_press(PidVerifyMsg::BomLocate(row.index).message())
        .padding([2, 8])
        .width(Fill)
        .style(move |theme: &Theme, status| {
            if located {
                button::secondary(theme, status)
            } else {
                button::text(theme, status)
            }
        })
        .into()
}

/// 「管道汇总」 (§4.5), which opens or folds the bill's side table: pipe
/// length by pipe class, nominal diameter and insulation class, and the
/// lines of each.
fn pipe_summary(table: &BomTable) -> Element<'_, Message> {
    let arrow = if table.summary_open { "▼" } else { "▶" };
    let header = button(text(format!("{arrow} 管道汇总（{}）", table.pipe_summary.len())).size(12))
        .on_press(PidVerifyMsg::BomSummary.message())
        .padding([2, 4])
        .style(button::text);
    let mut list = column![header].spacing(2);
    if !table.summary_open {
        return list.into();
    }
    for summed in &table.pipe_summary {
        let class = [
            &summed.pipe_class,
            &summed.nominal_diameter,
            &summed.insulation_class,
        ]
        .map(|said| {
            if said.is_empty() {
                "—"
            } else {
                said.as_str()
            }
        })
        .join(" / ");
        list = list.push(
            column![
                row![
                    text(class).size(12).width(Fill),
                    text(format!("{:.2} {}", summed.length, summed.unit)).size(12)
                ]
                .spacing(8),
                text(summed.lines.join("、")).size(11).style(muted)
            ]
            .spacing(1)
            .padding([2, 8]),
        );
    }
    list.into()
}

/// The totals at the bottom (§4.5): the whole bill's rows, by kind.
fn bom_totals(table: &BomTable) -> String {
    let by_kind: Vec<String> = table
        .by_kind
        .iter()
        .map(|(kind, count)| format!("{kind} {count}"))
        .collect();
    if by_kind.is_empty() {
        format!("合计 {} 行", table.total)
    } else {
        format!("合计 {} 行：{}", table.total, by_kind.join("，"))
    }
}

/// The snapshot half of 「快照与发布」 (§4.6): the reason box and
/// 【存快照】 and the message bar, then 【比对】 and 【导出比对结果】 over
/// the list — the drawing as it is on top, the snapshots newest first —
/// and the diff shown under it; or what the half says instead.
fn deliver_tab(view: &VerifyView) -> Element<'_, Message> {
    let grey = |button| view.grey(button).map(grey_said);
    let deliver = &view.deliver;
    let saving = row![
        PosReport::new(
            SNAPSHOT_REASON_ID,
            text_input("快照原因（可不填，最多 200 字）", &deliver.reason)
                .on_input(|typed| PidVerifyMsg::SnapshotReason(typed).message())
                .size(12)
                .padding([3, 6])
                .width(Fill),
        ),
        PosReport::new(
            SNAPSHOT_SAVE_ID,
            greyable(
                "存快照",
                Some(PidVerifyMsg::SnapshotSave(deliver.reason.clone())),
                grey(Button::SnapshotSave)
            ),
        ),
    ]
    .spacing(4)
    .align_y(iced::Center);
    let mut tab = column![saving].spacing(6).height(Fill);
    if let Some(notice) = &view.notice {
        tab = tab.push(notice_line(notice));
    }
    if let Some(said) = view.empty {
        return tab.push(empty_text(said)).into();
    }
    let compare = deliver.pair().map(|(a, b)| PidVerifyMsg::Diff(a, b));
    let actions = row![
        PosReport::new(DIFF_ID, greyable("比对", compare, grey(Button::Diff))),
        PosReport::new(
            DIFF_EXPORT_ID,
            greyable(
                "导出比对结果",
                Some(PidVerifyMsg::PickDiffExport),
                grey(Button::DiffExport)
            ),
        ),
    ]
    .spacing(4);
    let live = ticked(
        deliver.live_picked,
        Some(Pick::Live),
        text(LIVE_SAID).size(12).into(),
    );
    let mut body = column![PosReport::owned(Pick::Live.bounds_id(), live)]
        .spacing(4)
        .width(Fill);
    for row in &deliver.rows {
        body = body.push(snapshot_row(row));
    }
    if let Some(diff) = &deliver.diff {
        body = body.push(diff_part(diff));
    }
    let tab = tab.push(actions);
    let Some(publish) = &deliver.publish else {
        return tab.push(scrollable(body).height(Fill)).into();
    };
    tab.push(scrollable(body).height(Length::FillPortion(2)))
        .push(publish_part(view, publish))
        .into()
}

/// The publish half (§4.6): the status line, or why the records do not
/// read; the folder box with 【浏览…】 and the revision the next publish
/// makes; the eight conditions; 【检查发布条件】 and 【发布】, and the
/// question 【发布】 asks with 【确认】【取消】 under it; then the records,
/// newest first.
fn publish_part<'a>(view: &VerifyView, publish: &'a PublishView) -> Element<'a, Message> {
    let status = match (&publish.state, &publish.unlisted) {
        (Some(state), _) => text(format!("发布 · {state}")).size(12),
        (None, Some(why)) => text(format!("发布 · 发布记录读不出来：{why}"))
            .size(12)
            .style(danger),
        (None, None) => text("发布").size(12),
    };
    let mut folder = row![
        PosReport::new(
            PUBLISH_DIR_ID,
            text_input("发布目录（空着就是图纸目录下的 pid-publish）", &publish.dir)
                .on_input(|typed| PidVerifyMsg::PublishDir(typed).message())
                .size(12)
                .padding([3, 6])
                .width(Fill),
        ),
        small_button("浏览…", Some(PidVerifyMsg::PickPublishDir)),
    ]
    .spacing(4)
    .align_y(iced::Center);
    if let Some(next) = publish.next_revision {
        folder = folder.push(text(format!("下一个修订号：{next}")).size(11));
    }
    let mut part = column![status, folder].spacing(4).width(Fill);
    if let Some(target) = &publish.target_dir {
        part = part.push(text(format!("发布到 {target}")).size(11).style(muted));
    }
    let grey = |button| view.grey(button).map(grey_said);
    let actions = row![
        PosReport::new(
            PUBLISH_CHECK_ID,
            greyable(
                "检查发布条件",
                Some(PidVerifyMsg::PublishCheck),
                grey(Button::PublishCheck)
            ),
        ),
        PosReport::new(
            PUBLISH_ID,
            greyable(
                "发布",
                Some(PidVerifyMsg::AskPublish),
                grey(Button::Publish)
            ),
        ),
    ]
    .spacing(4);
    part = part.push(conditions_part(publish)).push(actions);
    if let Some(question) = &publish.asking {
        let answers = row![
            PosReport::new(
                PUBLISH_CONFIRM_ID,
                small_button("确认", Some(PidVerifyMsg::Publish)),
            ),
            small_button("取消", Some(PidVerifyMsg::CancelPublish)),
        ]
        .spacing(4);
        part = part.push(text(question.as_str()).size(12)).push(answers);
    }
    if !publish.packages.is_empty() {
        part = part.push(text("发布记录").size(11).style(muted));
    }
    for row in &publish.packages {
        part = part.push(package_row(row, grey(Button::PackageVerify)));
    }
    scrollable(part).height(Length::FillPortion(3)).into()
}

/// The eight conditions of the publish half (§4.6 item 3), two to a line;
/// under them, while the gates may be out of date, that they may; and while
/// the condition of repeated tags lists them, where each tag repeated is,
/// each place clicked to look at it.
fn conditions_part(publish: &PublishView) -> Element<'_, Message> {
    let conditions = publish.conditions();
    let mut part = column![].spacing(2).width(Fill);
    for (line, pair) in conditions.chunks(2).enumerate() {
        let mut cells = row![].spacing(8);
        for (offset, (said, met)) in pair.iter().enumerate() {
            let at = line * 2 + offset;
            cells = cells.push(condition_cell(
                at,
                said.clone(),
                *met,
                publish.duplicates_open,
            ));
        }
        part = part.push(cells);
    }
    if publish.stale {
        part = part.push(text(GATES_STALE).size(11).style(warning));
    }
    let listed = publish
        .gates
        .as_ref()
        .filter(|_| publish.duplicates_open)
        .map_or(&[][..], |found| found.duplicates.as_slice());
    for (repeated, duplicate) in listed.iter().enumerate() {
        part = part.push(
            text(format!(
                "{} {}（{} 处）",
                duplicate.type_code,
                duplicate.tag,
                duplicate.places.len()
            ))
            .size(11),
        );
        for (at, place) in duplicate.places.iter().enumerate() {
            part = part.push(place_button(
                format!("{} · {}", place.drawing_id, point_said(place.at)),
                PidVerifyMsg::PublishDuplicateLocate(repeated, at),
            ));
        }
    }
    part.into()
}

/// A condition of the publish half (§4.6 item 3), said and whether it
/// holds. When it does not, that of errors not waived is clicked to list
/// them on the rule-check tab, and that of repeated tags to list where each
/// is, or fold the list.
fn condition_cell<'a>(
    at: usize,
    said: String,
    met: Option<bool>,
    open: bool,
) -> Element<'a, Message> {
    let press = match (at, met) {
        (ERRORS_CONDITION, Some(false)) => Some(PidVerifyMsg::PublishErrors),
        (DUPLICATES_CONDITION, Some(false)) => Some(PidVerifyMsg::PublishDuplicates),
        _ => None,
    };
    let said = match press {
        Some(PidVerifyMsg::PublishDuplicates) => {
            format!("{} {said}", if open { "▼" } else { "▶" })
        }
        _ => said,
    };
    let verdict = match met {
        Some(true) => text(MET_SAID).size(11),
        Some(false) => text(UNMET_SAID).size(11).style(danger),
        None => text(UNCHECKED_SAID).size(11).style(muted),
    };
    let line = row![text(said).size(11).width(Fill), verdict].spacing(4);
    let Some(press) = press else {
        return container(line).padding([1, 4]).width(Fill).into();
    };
    button(line)
        .on_press(press.message())
        .padding([1, 4])
        .width(Fill)
        .style(button::text)
        .into()
}

/// A record of the publish half (§4.6 item 6): revision, when and by whom,
/// 「当前」 on the package made of the file on disk, 【校验】 — grey while
/// `grey` says why; under them its folder and what 【校验】 found.
fn package_row<'a>(row: &'a PackageRow, grey: Option<&'static str>) -> Element<'a, Message> {
    let package = &row.package;
    let when = package
        .published_at
        .map_or_else(|| "时间不明".to_owned(), utc_said);
    let by = package.published_by.as_deref().unwrap_or("发布人不明");
    let mut first = row![text(format!("修订 {} · {when} · {by}", package.revision)).size(12)]
        .spacing(8)
        .align_y(iced::Center);
    if package.current {
        first = first.push(text("当前").size(11));
    }
    let verify = PidVerifyMsg::PackageVerify(package.revision);
    first = first.push(greyable("校验", Some(verify), grey));
    let mut lines = column![first, text(package.path.as_str()).size(11).style(muted)].spacing(1);
    if let Some(verified) = &row.verified {
        let found = text(verified.text.as_str()).size(11);
        lines = lines.push(if verified.intact {
            found
        } else {
            found.style(danger)
        });
    }
    PosReport::owned(row.bounds_id(), Element::from(lines)).into()
}

/// A row of the snapshot list (§4.6): number, time, author and object
/// count, and the reason under them in a lighter colour; one whose file is
/// gone says so and cannot be ticked.
fn snapshot_row(row: &SnapshotRow) -> Element<'_, Message> {
    let info = &row.info;
    let made = utc_said(i64::try_from(info.created_at).unwrap_or_default());
    let said = format!(
        "第 {} 份 · {made} · {} · {} 个对象",
        info.seq, info.created_by, info.objects
    );
    let mut first = row![text(said).size(12)].spacing(8);
    if row.missing() {
        first = first.push(text(MISSING_SAID).size(11).style(danger));
    }
    let mut lines = column![first].spacing(1);
    if !info.reason.is_empty() {
        lines = lines.push(text(info.reason.as_str()).size(11).style(muted));
    }
    let pick = Pick::Seq(info.seq);
    let tick = ticked(row.picked, (!row.missing()).then_some(pick), lines.into());
    PosReport::owned(pick.bounds_id(), tick).into()
}

/// A tick box before `lines` that ticks or unticks `pick`; grey while
/// `pick` is `None`.
fn ticked<'a>(on: bool, pick: Option<Pick>, lines: Element<'a, Message>) -> Element<'a, Message> {
    let mut tick = checkbox(on).size(12);
    if let Some(pick) = pick {
        tick = tick.on_toggle(move |_| PidVerifyMsg::Pick(pick).message());
    }
    row![tick, lines].spacing(6).align_y(iced::Center).into()
}

/// The diff shown (§4.6): the rows it compared and its counts, or
/// 「无差异」, and what it warns of; then each group's header with its count
/// and its objects drawn, with 【再显示 200 条】 while it has more.
fn diff_part(diff: &DiffView) -> Element<'_, Message> {
    let sides = format!("{} → {}", pick_said(diff.a), pick_said(diff.b));
    let summary = if diff.identical {
        format!("{sides}：{IDENTICAL_SAID}")
    } else {
        let counted = diff.summary;
        format!(
            "{sides}：新增 {}，删除 {}，修改 {}",
            counted.added, counted.removed, counted.modified
        )
    };
    let mut part = column![text(summary).size(12)].spacing(4).width(Fill);
    for notice in &diff.notices {
        part = part.push(text(notice.message.as_str()).size(11).style(warning));
    }
    for group in &diff.groups {
        part = part.push(text(format!("{}（{}）", group.group.label(), group.count)).size(12));
        for item in &group.items[..group.drawn] {
            part = part.push(PosReport::owned(
                group.group.item_id(item.index),
                diff_item(group.group, item),
            ));
        }
        if group.drawn < group.count {
            part = part.push(small_button("再显示 200 条", Some(PidVerifyMsg::MoreDiff)));
        }
    }
    part.into()
}

/// A side of the diff as its summary names it.
fn pick_said(pick: Pick) -> String {
    match pick {
        Pick::Live => "当前图".to_owned(),
        Pick::Seq(seq) => format!("第 {seq} 份"),
    }
}

/// An object of the diff (§4.6): kind, type code and tag, and under it in a
/// lighter colour what changed on it, a line each. Clicking it finds it on
/// the drawing in front.
fn diff_item(group: DiffGroup, item: &DiffItem) -> Element<'_, Message> {
    let mut lines = column![text(item.label.as_str()).size(12)].spacing(1);
    for change in &item.changes {
        lines = lines.push(text(change.as_str()).size(11).style(muted));
    }
    let located = item.located;
    button(lines)
        .on_press(PidVerifyMsg::DiffLocate(group, item.index).message())
        .padding([2, 8])
        .width(Fill)
        .style(move |theme: &Theme, status| {
            if located {
                button::secondary(theme, status)
            } else {
                button::text(theme, status)
            }
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pid::project_index::ObjectKind;

    #[test]
    fn the_panel_starts_on_the_rule_check_tab() {
        let view = VerifyView::default();
        assert_eq!(view.tab, Tab::Check);
        assert_eq!(view.empty, Some("还没有检查过。点上面的按钮开始。"));
    }

    #[test]
    fn what_is_drawn_follows_the_tab() {
        let drawn: Vec<(Tab, Option<&str>)> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let ui = VerifyUi {
                    tab,
                    ..VerifyUi::default()
                };
                let view = build_view(&ui, None, None, &Context::default());
                (view.tab, view.empty)
            })
            .collect();
        assert_eq!(
            drawn,
            [
                (Tab::Check, Some("还没有检查过。点上面的按钮开始。")),
                (Tab::Bom, Some(NEVER_LISTED)),
                (Tab::Deliver, Some(NO_DRAWING_SAID)),
            ]
        );
    }

    #[test]
    fn the_tabs_read_as_the_spec_names_them() {
        let labels: Vec<&str> = Tab::ALL.into_iter().map(Tab::label).collect();
        assert_eq!(labels, ["规则检查", "材料清单", "快照与发布"]);
    }

    #[test]
    fn the_controls_report_under_the_ids_the_spec_gives_them() {
        assert!(PANEL_BOUNDS_ID.starts_with(BOUNDS_PREFIX));
        let keys: Vec<&str> = Tab::ALL.into_iter().map(Tab::key).collect();
        assert_eq!(keys, ["check", "bom", "deliver"]);
        let ids: Vec<&str> = Tab::ALL.into_iter().map(Tab::bounds_id).collect();
        assert_eq!(
            ids,
            [
                "pid-verify-tab-check",
                "pid-verify-tab-bom",
                "pid-verify-tab-deliver",
            ]
        );
        assert_eq!(
            [CHECK_DRAWING_ID, CHECK_PROJECT_ID],
            ["pid-verify-check-drawing", "pid-verify-check-project"]
        );
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(shown.rows[2].bounds_id(), "pid-verify-check-row-2");
        for id in [CHECK_DRAWING_ID, CHECK_PROJECT_ID, CHECK_ROW_PREFIX] {
            assert!(id.starts_with(BOUNDS_PREFIX), "{id}");
        }
    }

    /// A drawing in front that the session may not change, never saved, the
    /// panel on screen.
    fn read_only_in_front() -> Context {
        Context {
            visible: true,
            document_id: Some(7),
            drawing_id: Some("PID-1101".to_owned()),
            writable: false,
            saved: false,
            dirty: false,
            busy: None,
            waivers: Vec::new(),
        }
    }

    #[test]
    fn pid_verify_panel_answers_the_common_fields_and_the_three_blocks() {
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        let view = build_view(&ui, None, None, &read_only_in_front());
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        let mut keys: Vec<&str> = state
            .as_object()
            .map(|fields| fields.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort_unstable();
        let mut common = [
            "ok",
            "visible",
            "tab",
            "document_id",
            "drawing_id",
            "writable",
            "busy",
            "notice",
            "buttons",
            "check",
            "bom",
            "deliver",
        ];
        common.sort_unstable();
        assert_eq!(keys, common, "{state}");
        assert_eq!(state["ok"], true);
        assert_eq!(state["visible"], true);
        assert_eq!(state["tab"], "bom");
        assert_eq!(state["document_id"], 7);
        assert_eq!(state["drawing_id"], "PID-1101");
        assert_eq!(state["writable"], false);
        assert_eq!(state["busy"], Value::Null);
        assert_eq!(state["notice"], Value::Null);
        // Only 【检查当前图】 and 【当前图】 can be pressed: the drawing was
        // never saved, and nothing is checked or listed yet.
        let mut buttons: Vec<(&str, Option<bool>)> = state["buttons"]
            .as_object()
            .map(|named| {
                named
                    .iter()
                    .map(|(name, can)| (name.as_str(), can.as_bool()))
                    .collect()
            })
            .unwrap_or_default();
        buttons.sort_unstable();
        let mut names = [
            "check_drawing",
            "check_project",
            "check_export",
            "bom_drawing",
            "bom_project",
            "bom_export",
            "snapshot_save",
            "diff",
            "diff_export",
            "publish_check",
            "publish",
        ];
        names.sort_unstable();
        let expected: Vec<(&str, Option<bool>)> = names
            .into_iter()
            .map(|name| (name, Some(matches!(name, "check_drawing" | "bom_drawing"))))
            .collect();
        assert_eq!(buttons, expected, "{state}");
        // Nothing checked, listed or kept yet.
        assert_eq!(
            state["check"],
            json!({
                "has_result": false,
                "stale": false,
                "scope": null,
                "checked_at": null,
                "rule_set_version": null,
                "counts": { "错误": 0, "警告": 0, "提示": 0, "已豁免": 0 },
                "filters": {
                    "severity": ["错误", "警告", "提示"],
                    "kinds": [],
                    "show_waived": false
                },
                "kinds": [],
                "notices": [],
                "stale_waivers": 0,
                "total": 0,
                "rows": [],
                "located": null
            })
        );
        assert_eq!(
            state["bom"],
            json!({
                "has_result": false,
                "stale": false,
                "scope": null,
                "search": "",
                "error": null,
                "total": 0,
                "groups": [],
                "rows": []
            })
        );
        // A drawing never saved has no records to read and cannot be
        // published; nothing was checked.
        assert_eq!(
            state["deliver"],
            json!({
                "snapshots": [],
                "picked": [],
                "diff": null,
                "publish": {
                    "state": null,
                    "target_dir": null,
                    "next_revision": null,
                    "preconditions": {
                        "saved": false,
                        "clean": true,
                        "file_unchanged": false,
                        "writable_session": false
                    },
                    "checked": false,
                    "stale": false,
                    "can_publish": null,
                    "gates": null,
                    "packages": [],
                    "last_package": null
                }
            })
        );
    }

    #[test]
    fn pid_verify_panel_with_no_drawing_in_front_names_none() {
        let state = panel_state(&VerifyView::default(), ROWS_PER_PAGE, 0);
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "check", "{state}");
        assert_eq!(state["document_id"], Value::Null, "{state}");
        assert_eq!(state["drawing_id"], Value::Null, "{state}");
    }

    #[test]
    fn pid_verify_panel_pages_each_list() {
        let block = json!({ "total": 5, "rows": [0, 1, 2, 3, 4] });
        assert_eq!(
            paged(block.clone(), 2, 1),
            json!({ "total": 5, "rows": [1, 2] })
        );
        assert_eq!(paged(block.clone(), 3, 4)["rows"], json!([4]));
        assert_eq!(paged(block.clone(), 3, 9)["rows"], json!([]));
        assert_eq!(paged(block.clone(), ROWS_PER_PAGE, 0), block);
        assert_eq!(ROWS_PER_PAGE, 200);
    }

    #[test]
    fn pid_verify_action_reads_show_hide_and_the_tabs() {
        let view = VerifyView::default();
        for (value, pressed) in [
            ("show", PidVerifyMsg::Show),
            ("hide", PidVerifyMsg::Hide),
            ("tab=check", PidVerifyMsg::Tab(Tab::Check)),
            ("tab=bom", PidVerifyMsg::Tab(Tab::Bom)),
            ("tab=deliver", PidVerifyMsg::Tab(Tab::Deliver)),
            // The tab is named in any case, as `PIDVERIFY` takes it.
            ("tab=Deliver", PidVerifyMsg::Tab(Tab::Deliver)),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_names_a_wrong_key_or_value_invalid() {
        let view = VerifyView::default();
        for value in [
            "",
            "open",
            "SHOW",
            "show=1",
            "hide=",
            "tab",
            "tab=",
            "tab=report",
            "tab=bom=1",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert_eq!(refused.reason(), None, "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        // The refusal says what to write.
        assert_eq!(
            parse_action("tab=report", &view).map_err(|refused| refused.to_string()),
            Err("tab 写 check、bom、deliver 中的一个，实得 \"report\"".to_owned())
        );
    }

    #[test]
    fn pid_verify_action_errors_carry_the_codes_of_section_3_7() {
        let grey = PanelError::Unavailable { reason: "busy" };
        assert_eq!(
            (grey.code(), grey.reason()),
            ("PANEL_ACTION_UNAVAILABLE", Some("busy"))
        );
        assert!(grey.to_string().contains("busy"), "{grey}");
        let gone = PanelError::RowNotFound("规则检查的列表里没有下标 7".to_owned());
        assert_eq!((gone.code(), gone.reason()), ("PANEL_ROW_NOT_FOUND", None));
        assert_eq!(gone.to_string(), "规则检查的列表里没有下标 7");
        let wrong = PanelError::InvalidValue("tab 写 check、bom、deliver 中的一个".to_owned());
        assert_eq!((wrong.code(), wrong.reason()), ("invalid_value", None));
        assert_eq!(wrong.to_string(), "tab 写 check、bom、deliver 中的一个");
    }

    /// A result of rule `rule` on an object of `kind` tagged `tag` — none
    /// when it is empty — on drawing `drawing`.
    fn found(rule: &str, severity: Severity, kind: &str, tag: &str, drawing: &str) -> CheckResult {
        CheckResult {
            rule_id: rule.to_owned(),
            rule_name: format!("{rule} 的名字"),
            severity,
            object: drc::ObjectRef {
                kind: kind.to_owned(),
                type_code: "T".to_owned(),
                tag: (!tag.is_empty()).then(|| tag.to_owned()),
                drawing_id: drawing.to_owned(),
                owner: None,
                port: None,
                at: [0.0, 0.0],
                handle: None,
            },
            object_key: format!("{kind}|{tag}"),
            message: format!("{tag} 不对"),
            fix: format!("改 {tag}"),
            points: Vec::new(),
            fields: vec!["管道等级".to_owned()],
            waived: false,
            waiver: None,
        }
    }

    /// `result` waived by 张工 because it is a vent.
    fn waived(mut result: CheckResult) -> CheckResult {
        result.waiver = Some(drc::Waiver {
            rule_id: result.rule_id.clone(),
            object_key: result.object_key.clone(),
            reason: "放空管".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_800_000,
        });
        result.waived = true;
        result
    }

    /// A check of `scope` that found `results`, counted as the checker
    /// counts them.
    fn report(scope: ScopeKind, results: Vec<CheckResult>) -> CheckReport {
        let mut summary = Summary::default();
        for result in &results {
            let count = match (result.waived, result.severity) {
                (true, _) => &mut summary.waived,
                (false, Severity::Error) => &mut summary.errors,
                (false, Severity::Warning) => &mut summary.warnings,
                (false, Severity::Info) => &mut summary.infos,
            };
            *count += 1;
        }
        let drawing = scope == ScopeKind::Drawing;
        CheckReport {
            version: drc::REPORT_VERSION,
            rule_set_version: "1.0.0".to_owned(),
            rule_set_digest: String::new(),
            checked_at: 1_794_800_000,
            trigger: drc::Trigger::OnDemand,
            scope: Scope {
                kind: scope,
                drawing_id: drawing.then(|| "PID-1101".to_owned()),
                pbs: None,
                cross_drawing: !drawing,
            },
            elapsed_ms: 12,
            summary,
            blocked: summary.errors > 0,
            results,
            notices: Vec::new(),
            stale_waivers: Vec::new(),
        }
    }

    /// Five results on PID-1101 in the checker's order: two errors, on a
    /// pipe line and an instrument; a warning on a pipe line and a waived
    /// one on a valve; a note on a pump.
    fn five() -> CheckReport {
        report(
            ScopeKind::Drawing,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-005", Severity::Error, "仪表", "FT-1001", "PID-1101"),
                found("DR-007", Severity::Warning, "管线", "L-1002", "PID-1101"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
                found("DR-010", Severity::Info, "设备", "P-1001", "PID-1101"),
            ],
        )
    }

    /// PID-1101 in front, saved, in a session that may write, the panel on
    /// screen.
    fn in_front() -> Context {
        Context {
            visible: true,
            document_id: Some(7),
            drawing_id: Some("PID-1101".to_owned()),
            writable: true,
            saved: true,
            dirty: false,
            busy: None,
            waivers: Vec::new(),
        }
    }

    /// What the panel draws with `report` kept on the drawing in front.
    fn viewed(ui: &VerifyUi, report: &CheckReport) -> VerifyView {
        let drawing = DrawingResults {
            check: Some(KeptCheck {
                report,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        build_view(ui, Some(&drawing), None, &in_front())
    }

    /// The groups drawn with their counts, and the results listed by index.
    fn listed(view: &VerifyView) -> (Vec<(Group, usize)>, Vec<usize>) {
        let shown = view.check.shown.as_ref().expect("a result shown");
        (
            shown
                .groups
                .iter()
                .map(|group| (group.group, group.count))
                .collect(),
            shown.rows.iter().map(|row| row.index).collect(),
        )
    }

    #[test]
    fn the_list_is_grouped_by_severity_and_each_header_counts_what_the_filters_leave() {
        use Group::{Error, Info, Warning};
        let five = five();
        let all = VerifyUi::default();
        assert_eq!(
            listed(&viewed(&all, &five)),
            (vec![(Error, 2), (Warning, 1), (Info, 1)], vec![0, 1, 2, 4])
        );
        let no_warnings = VerifyUi {
            severities: vec![Severity::Error, Severity::Info],
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&no_warnings, &five)),
            (vec![(Error, 2), (Info, 1)], vec![0, 1, 4])
        );
        // The waived results come last, as a group of their own, which the
        // severity tags leave alone.
        let waived_too = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&waived_too, &five)),
            (
                vec![(Error, 2), (Warning, 1), (Info, 1), (Group::Waived, 1)],
                vec![0, 1, 2, 4, 3]
            )
        );
        let errors_and_waived = VerifyUi {
            severities: vec![Severity::Error],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&errors_and_waived, &five)),
            (vec![(Error, 2), (Group::Waived, 1)], vec![0, 1, 3])
        );
        // A kind switched on lists that kind only, in every group.
        let pipes = VerifyUi {
            kinds: vec!["管线".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&pipes, &five)),
            (vec![(Error, 1), (Warning, 1)], vec![0, 2])
        );
        let valves = VerifyUi {
            kinds: vec!["阀门".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&viewed(&valves, &five)),
            (vec![(Group::Waived, 1)], vec![3])
        );
        // A kind this check did not find has no tag to switch it off, so it
        // filters nothing.
        let gone = VerifyUi {
            kinds: vec!["换热器".to_owned()],
            ..VerifyUi::default()
        };
        let view = viewed(&gone, &five);
        assert_eq!(listed(&view).1, vec![0, 1, 2, 4]);
        assert!(view.check.kinds_on.is_empty());
        // The kind tags are the kinds found, the waived results' too.
        let shown = viewed(&all, &five).check.shown.expect("a result shown");
        assert_eq!(shown.kinds, ["仪表", "管线", "设备", "阀门"]);
        // A folded group keeps its count and draws none of its rows; the
        // panel state lists them all the same.
        let folded = VerifyUi {
            folded: vec![Error],
            ..VerifyUi::default()
        };
        let view = viewed(&folded, &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            shown.groups[0],
            GroupView {
                group: Error,
                count: 2,
                first: 0,
                drawn: 0,
                folded: true,
            }
        );
        assert_eq!(shown.groups[1].drawn, 1);
        assert_eq!(panel_state(&view, ROWS_PER_PAGE, 0)["check"]["total"], 4);
    }

    #[test]
    fn a_row_names_its_object_by_tag_else_by_kind_and_owner() {
        let tagged = found("DR-001", Severity::Error, "管线", "L-1003", "PID-1101");
        assert_eq!(label_of(&tagged), "L-1003");
        let mut segment = found("DR-004", Severity::Warning, "管段", "", "PID-1101");
        segment.object.owner = Some("L-1003-01".to_owned());
        assert_eq!(label_of(&segment), "管段（L-1003-01）");
        let bare = found("DR-009", Severity::Error, "图纸", "", "PID-1101");
        assert_eq!(label_of(&bare), "图纸");
        // The first line: rule and object, then the drawing when the object
        // is on another one.
        let project = report(
            ScopeKind::Project,
            vec![
                tagged,
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
                segment,
            ],
        );
        let kept = ProjectResults {
            bom: None,
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
        };
        let view = build_view(&VerifyUi::default(), None, Some(&kept), &in_front());
        let shown = view.check.shown.expect("a result shown");
        let lines: Vec<String> = shown.rows.iter().map(RowView::headline).collect();
        assert_eq!(
            lines,
            [
                "DR-001 L-1003",
                "DR-102 P-1001 · PID-1102",
                "DR-004 管段（L-1003-01）"
            ]
        );
        let labels: Vec<&str> = shown.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["L-1003", "P-1001", "管段（L-1003-01）"]);
    }

    #[test]
    fn of_the_drawing_s_and_the_project_s_result_the_newer_is_shown() {
        let mine = report(
            ScopeKind::Drawing,
            vec![found(
                "DR-001",
                Severity::Error,
                "管线",
                "L-1001",
                "PID-1101",
            )],
        );
        let theirs = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
            ],
        );
        let shown = |drawing: u64, project: u64| {
            let drawing = DrawingResults {
                check: Some(KeptCheck {
                    report: &mine,
                    serial: drawing,
                    stale: false,
                }),
                ..DrawingResults::default()
            };
            let project = ProjectResults {
                bom: None,
                check: Some(KeptCheck {
                    report: &theirs,
                    serial: project,
                    stale: true,
                }),
            };
            let view = build_view(
                &VerifyUi::default(),
                Some(&drawing),
                Some(&project),
                &in_front(),
            );
            let shown = view.check.shown.expect("a result shown");
            (shown.scope, shown.stale, shown.rows.len())
        };
        assert_eq!(shown(2, 5), (ScopeKind::Project, true, 2));
        assert_eq!(shown(6, 5), (ScopeKind::Drawing, false, 1));
        // With one kept, that one; with none, none.
        let project = ProjectResults {
            bom: None,
            check: Some(KeptCheck {
                report: &theirs,
                serial: 1,
                stale: false,
            }),
        };
        let view = build_view(
            &VerifyUi::default(),
            Some(&DrawingResults::default()),
            Some(&project),
            &in_front(),
        );
        assert_eq!(
            view.check.shown.map(|shown| shown.scope),
            Some(ScopeKind::Project)
        );
        let view = build_view(&VerifyUi::default(), None, None, &in_front());
        assert_eq!(view.check.shown, None);
    }

    #[test]
    fn each_group_draws_200_rows_and_200_more_for_each_press() {
        let many = report(
            ScopeKind::Drawing,
            (0..450)
                .map(|n| {
                    found(
                        "DR-001",
                        Severity::Error,
                        "管线",
                        &format!("L-{n:04}"),
                        "PID-1101",
                    )
                })
                .collect(),
        );
        let drawn = |more: usize| {
            let ui = VerifyUi {
                more,
                ..VerifyUi::default()
            };
            let view = viewed(&ui, &many);
            let group = view.check.shown.expect("a result shown").groups[0];
            (group.count, group.drawn, group.has_more())
        };
        assert_eq!(drawn(0), (450, 200, true));
        assert_eq!(drawn(1), (450, 400, true));
        assert_eq!(drawn(2), (450, 450, false));
        assert_eq!(drawn(3), (450, 450, false));
        // The panel state pages them all, whatever is drawn.
        let view = viewed(&VerifyUi::default(), &many);
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        assert_eq!(state["check"]["total"], 450);
        assert_eq!(state["check"]["rows"].as_array().map(Vec::len), Some(200));
        let state = panel_state(&view, ROWS_PER_PAGE, 400);
        assert_eq!(state["check"]["rows"].as_array().map(Vec::len), Some(50));
        assert_eq!(state["check"]["rows"][0]["index"], 400);
    }

    #[test]
    fn the_rule_check_tab_says_why_it_has_no_list() {
        let ui = VerifyUi::default();
        // Never checked.
        let view = build_view(&ui, None, None, &in_front());
        assert_eq!(view.empty, Some(NEVER_CHECKED));
        assert_eq!(NEVER_CHECKED, "还没有检查过。点上面的按钮开始。");
        // Checked and nothing found: the status line and the notices are
        // there all the same.
        let mut clean = report(ScopeKind::Drawing, Vec::new());
        clean.notices.push(drc::Notice {
            code: "PROJECT_RULES_SKIPPED".to_owned(),
            message: "只检查了当前图".to_owned(),
        });
        let view = viewed(&ui, &clean);
        assert_eq!(view.empty, Some("没有发现问题。"));
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            (shown.notices.len(), shown.scope_said.as_str()),
            (1, "图 PID-1101")
        );
        // Found, but all filtered out: by the tags, or for being waived.
        let none_on = VerifyUi {
            severities: Vec::new(),
            ..VerifyUi::default()
        };
        assert_eq!(
            viewed(&none_on, &five()).empty,
            Some("没有符合筛选条件的结果。")
        );
        let all_waived = report(
            ScopeKind::Drawing,
            vec![waived(found(
                "DR-008",
                Severity::Warning,
                "阀门",
                "V-1001",
                "PID-1101",
            ))],
        );
        assert_eq!(viewed(&ui, &all_waived).empty, Some(ALL_FILTERED));
        let waived_too = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        assert_eq!(viewed(&waived_too, &all_waived).empty, None);
        // A broken rule pack: its error in the message bar, the result kept
        // as it was.
        let broken = VerifyUi {
            notice: Some(Notice::error(
                Some("DSL_PARSE_ERROR".to_owned()),
                "PIDCHECK: 项目包有错",
            )),
            ..VerifyUi::default()
        };
        let five = five();
        let view = viewed(&broken, &five);
        assert_eq!(view.empty, None);
        assert_eq!(listed(&view).1, vec![0, 1, 2, 4]);
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        assert_eq!(
            state["notice"],
            json!({ "level": "error", "code": "DSL_PARSE_ERROR", "text": "PIDCHECK: 项目包有错" })
        );
        assert_eq!(state["check"]["total"], 4);
        // The other tabs say their own.
        let bom = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        assert_eq!(viewed(&bom, &five).empty, Some(NEVER_LISTED));
    }

    /// Whether 【检查当前图】【检查全项目】【导出…】 can be pressed.
    fn pressable(context: &Context, kept: Option<&DrawingResults<'_>>) -> (bool, bool, bool) {
        let buttons = build_view(&VerifyUi::default(), kept, None, context).buttons;
        (
            buttons.check_drawing,
            buttons.check_project,
            buttons.check_export,
        )
    }

    #[test]
    fn the_rule_check_buttons_are_grey_as_section_4_8_says() {
        let five = five();
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        assert_eq!(pressable(&in_front(), Some(&kept)), (true, true, true));
        // Nothing checked: nothing to export.
        assert_eq!(pressable(&in_front(), None), (true, true, false));
        // Busy: no new check, but the result can be exported.
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        assert_eq!(pressable(&busy, Some(&kept)), (false, false, true));
        // Never saved: in no project folder.
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        assert_eq!(pressable(&unsaved, Some(&kept)), (true, false, true));
        // A read-only session checks and exports all the same (§5.4).
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(pressable(&read_only, Some(&kept)), (true, true, true));
        // The Start page has no drawing to check.
        assert_eq!(pressable(&Context::default(), None), (false, false, false));
        // The material list's buttons, 【存快照】 and the publish half's are
        // tested with their tabs.
        let buttons = build_view(&VerifyUi::default(), Some(&kept), None, &in_front()).buttons;
        assert_eq!(
            Buttons {
                check_drawing: false,
                check_project: false,
                check_export: false,
                bom_drawing: false,
                bom_project: false,
                bom_export: false,
                snapshot_save: false,
                publish_check: false,
                publish: false,
                ..buttons
            },
            Buttons::default()
        );
    }

    #[test]
    fn a_row_can_be_waived_on_this_drawing_unwaived_by_a_session_that_may_write() {
        let project = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
            ],
        );
        let kept = ProjectResults {
            bom: None,
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
        };
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let waivable = |context: &Context| -> Vec<(usize, bool)> {
            let view = build_view(&ui, None, Some(&kept), context);
            let shown = view.check.shown.expect("a result shown");
            shown
                .rows
                .iter()
                .map(|row| (row.index, row.can_waive))
                .collect()
        };
        assert_eq!(waivable(&in_front()), [(0, true), (1, false), (2, false)]);
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(waivable(&read_only), [(0, false), (1, false), (2, false)]);
    }

    #[test]
    fn the_check_block_reads_off_the_tab_as_worked_out() {
        let mut five = five();
        five.notices.push(drc::Notice {
            code: "MASTER_DATA_MISSING".to_owned(),
            message: "项目主数据没有配置".to_owned(),
        });
        five.stale_waivers.push(drc::Waiver {
            rule_id: "DR-003".to_owned(),
            object_key: "管线|L-0999".to_owned(),
            reason: "已拆除".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_700_000,
        });
        let ui = VerifyUi {
            severities: vec![Severity::Error],
            kinds: vec!["管线".to_owned()],
            show_waived: true,
            ..VerifyUi::default()
        };
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 3,
                stale: true,
            }),
            ..DrawingResults::default()
        };
        let view = build_view(&ui, Some(&kept), None, &in_front());
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        let check = &state["check"];
        assert_eq!(check["has_result"], true, "{check}");
        assert_eq!(check["stale"], true, "{check}");
        assert_eq!(check["scope"], "drawing", "{check}");
        assert_eq!(check["checked_at"], 1_794_800_000_i64, "{check}");
        assert_eq!(check["rule_set_version"], "1.0.0", "{check}");
        // The counts are the check's own, whatever the filters.
        assert_eq!(
            check["counts"],
            json!({ "错误": 2, "警告": 1, "提示": 1, "已豁免": 1 })
        );
        assert_eq!(
            check["filters"],
            json!({ "severity": ["错误"], "kinds": ["管线"], "show_waived": true })
        );
        assert_eq!(check["kinds"], json!(["仪表", "管线", "设备", "阀门"]));
        assert_eq!(
            check["notices"],
            json!([{ "code": "MASTER_DATA_MISSING", "message": "项目主数据没有配置" }])
        );
        assert_eq!(check["stale_waivers"], 1);
        assert_eq!(check["total"], 1);
        assert_eq!(
            check["rows"],
            json!([{
                "index": 0,
                "rule_id": "DR-001",
                "severity": "错误",
                "object_key": "管线|L-1001",
                "drawing_id": "PID-1101",
                "label": "L-1001",
                "message": "L-1001 不对",
                "fields": ["管道等级"],
                "waived": false,
                "can_waive": true
            }])
        );
        assert_eq!(check["located"], Value::Null);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(
            status_line(shown),
            "图 PID-1101 · 2026-11-16 03:33 UTC · 规则 1.0.0 · 用时 12 ms"
        );
    }

    #[test]
    fn pid_verify_action_reads_the_rule_check_keys() {
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        for (value, pressed) in [
            ("check=drawing", PidVerifyMsg::CheckDrawing),
            ("check=project", PidVerifyMsg::CheckProject),
            ("severity=错误", PidVerifyMsg::Severity(Severity::Error)),
            ("severity=提示", PidVerifyMsg::Severity(Severity::Info)),
            ("kind=管线", PidVerifyMsg::Kind(Some("管线".to_owned()))),
            ("kind=", PidVerifyMsg::Kind(None)),
            ("show_waived=1", PidVerifyMsg::ShowWaived(true)),
            ("show_waived=0", PidVerifyMsg::ShowWaived(false)),
            (
                "check_export=D:/out/drc.json",
                PidVerifyMsg::Export("D:/out/drc.json".to_owned()),
            ),
            ("more=check", PidVerifyMsg::MoreChecks),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_rule_check_button_with_the_reason() {
        let five = five();
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        let ui = VerifyUi::default();
        let busy_context = Context {
            busy: SCANNING,
            ..in_front()
        };
        let busy = build_view(&ui, Some(&kept), None, &busy_context);
        let unsaved_context = Context {
            saved: false,
            ..in_front()
        };
        let unsaved = build_view(&ui, Some(&kept), None, &unsaved_context);
        let unchecked = build_view(&ui, None, None, &in_front());
        // The Start page in front.
        let no_drawing = build_view(&ui, None, None, &Context::default());
        for (view, value, reason) in [
            (&busy, "check=drawing", "busy"),
            (&busy, "check=project", "busy"),
            (&unsaved, "check=project", "no_project_dir"),
            (&no_drawing, "check=drawing", "no_drawing"),
            (&no_drawing, "check=project", "no_drawing"),
            (&unchecked, "check_export=D:/out/drc.json", "no_result"),
            (&unchecked, "kind=管线", "no_result"),
        ] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        // Filtering, paging and exporting stay open while something runs.
        for value in [
            "severity=警告",
            "show_waived=1",
            "kind=管线",
            "kind=",
            "more=check",
            "check_export=drc.json",
        ] {
            assert!(parse_action(value, &busy).is_ok(), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_names_a_wrong_rule_check_value_invalid() {
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        for value in [
            "check",
            "check=",
            "check=all",
            "severity",
            "severity=",
            "severity=严重",
            "kind",
            "kind=换热器",
            "show_waived",
            "show_waived=2",
            "show_waived=true",
            "check_export",
            "check_export=",
            "check_export=  ",
            "more",
            "more=",
            "more=list",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            parse_action("kind=换热器", &view).map_err(|refused| refused.to_string()),
            Err("这次检查的结果里没有种类「换热器」；有的是：仪表、管线、设备、阀门".to_owned())
        );
    }

    #[test]
    fn the_row_last_clicked_is_located_while_its_check_is_shown() {
        let five = five();
        let ui = VerifyUi {
            located: Some((1, 2)),
            ..VerifyUi::default()
        };
        let view = viewed(&ui, &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(shown.located, Some(2));
        let located: Vec<usize> = shown
            .rows
            .iter()
            .filter(|row| row.located)
            .map(|row| row.index)
            .collect();
        assert_eq!(located, [2]);
        assert_eq!(panel_state(&view, ROWS_PER_PAGE, 0)["check"]["located"], 2);
        // A row of another check: nothing is located in this one.
        let other = VerifyUi {
            located: Some((0, 2)),
            ..VerifyUi::default()
        };
        let view = viewed(&other, &five);
        let shown = view.check.shown.as_ref().expect("a result shown");
        assert_eq!(shown.located, None);
        assert!(shown.rows.iter().all(|row| !row.located));
        assert_eq!(
            panel_state(&view, ROWS_PER_PAGE, 0)["check"]["located"],
            Value::Null
        );
    }

    #[test]
    fn the_reason_box_is_open_under_a_row_that_can_be_waived_while_its_check_is_shown() {
        let five = five();
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        let open = |serial: u64, index: usize, context: &Context| -> Vec<(usize, String)> {
            let ui = VerifyUi {
                show_waived: true,
                waiving: Some(Waiving {
                    serial,
                    index,
                    reason: "另有联锁".to_owned(),
                }),
                ..VerifyUi::default()
            };
            let view = build_view(&ui, Some(&kept), None, context);
            let shown = view.check.shown.expect("a result shown");
            shown
                .rows
                .into_iter()
                .filter_map(|row| Some((row.index, row.reason?)))
                .collect()
        };
        assert_eq!(open(1, 1, &in_front()), [(1, "另有联锁".to_owned())]);
        // Not under a row of another check, a waived row, or in a session
        // that may not write.
        assert!(open(2, 1, &in_front()).is_empty());
        assert!(open(1, 3, &in_front()).is_empty());
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert!(open(1, 1, &read_only).is_empty());
    }

    #[test]
    fn a_waived_row_can_be_unwaived_on_this_drawing_by_a_session_that_may_write() {
        let project = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                waived(found(
                    "DR-102",
                    Severity::Error,
                    "设备",
                    "P-1001",
                    "PID-1102",
                )),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
            ],
        );
        let kept = ProjectResults {
            bom: None,
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
        };
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let unwaivable = |context: &Context| -> Vec<(usize, bool)> {
            let view = build_view(&ui, None, Some(&kept), context);
            let shown = view.check.shown.expect("a result shown");
            shown
                .rows
                .iter()
                .map(|row| (row.index, row.can_unwaive))
                .collect()
        };
        assert_eq!(unwaivable(&in_front()), [(0, false), (1, false), (2, true)]);
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(unwaivable(&read_only), [(0, false), (1, false), (2, false)]);
    }

    #[test]
    fn the_waivers_no_result_matches_are_listed_and_only_those_the_drawing_keeps_can_be_dropped() {
        let mut five = five();
        for (key, reason) in [("管线|L-0999", "已拆除"), ("管线|L-0998", "改走地下")] {
            five.stale_waivers.push(drc::Waiver {
                rule_id: "DR-003".to_owned(),
                object_key: key.to_owned(),
                reason: reason.to_owned(),
                by: "张工".to_owned(),
                at: 1_794_700_000,
            });
        }
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        // The drawing in front keeps only the first.
        let keeps = Context {
            waivers: vec![("DR-003".to_owned(), "管线|L-0999".to_owned())],
            ..in_front()
        };
        let listed = |ui: &VerifyUi, context: &Context| -> (bool, Vec<(String, bool)>) {
            let view = build_view(ui, Some(&kept), None, context);
            let shown = view.check.shown.expect("a result shown");
            let waivers = shown
                .stale_waivers
                .iter()
                .map(|waiver| (waiver.object_key.clone(), waiver.can_drop))
                .collect();
            (shown.stale_open, waivers)
        };
        let open = VerifyUi {
            stale_open: true,
            ..VerifyUi::default()
        };
        assert_eq!(
            listed(&open, &keeps),
            (
                true,
                vec![
                    ("管线|L-0999".to_owned(), true),
                    ("管线|L-0998".to_owned(), false)
                ]
            )
        );
        assert!(!listed(&VerifyUi::default(), &keeps).0);
        let read_only = Context {
            writable: false,
            ..keeps.clone()
        };
        assert_eq!(
            listed(&open, &read_only).1,
            [
                ("管线|L-0999".to_owned(), false),
                ("管线|L-0998".to_owned(), false)
            ]
        );
        // The panel state counts them all.
        let view = build_view(&open, Some(&kept), None, &keeps);
        assert_eq!(
            panel_state(&view, ROWS_PER_PAGE, 0)["check"]["stale_waivers"],
            2
        );
    }

    #[test]
    fn pid_verify_action_reads_the_row_keys() {
        let five = five();
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let view = viewed(&ui, &five);
        for (value, pressed) in [
            ("locate=0", PidVerifyMsg::Locate(0)),
            ("locate=3", PidVerifyMsg::Locate(3)),
            (
                "waive=0:放空管",
                PidVerifyMsg::Waive(0, "放空管".to_owned()),
            ),
            // The waiver's entry refuses an empty reason, in the message bar.
            ("waive=4:", PidVerifyMsg::Waive(4, String::new())),
            (
                "waive=1:见联锁表:第 3 行=甲",
                PidVerifyMsg::Waive(1, "见联锁表:第 3 行=甲".to_owned()),
            ),
            ("unwaive=3", PidVerifyMsg::Unwaive(3)),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_row_button_with_the_reason() {
        let five = five();
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &five,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        let writable = build_view(&ui, Some(&kept), None, &in_front());
        let read_only_context = Context {
            writable: false,
            ..in_front()
        };
        let read_only = build_view(&ui, Some(&kept), None, &read_only_context);
        let unchecked = build_view(&ui, None, None, &in_front());
        let project = report(
            ScopeKind::Project,
            vec![
                found("DR-102", Severity::Error, "设备", "P-1001", "PID-1102"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1102",
                )),
            ],
        );
        let kept_elsewhere = ProjectResults {
            bom: None,
            check: Some(KeptCheck {
                report: &project,
                serial: 2,
                stale: false,
            }),
        };
        let elsewhere = build_view(&ui, None, Some(&kept_elsewhere), &in_front());
        for (view, value, reason) in [
            (&unchecked, "locate=0", "no_result"),
            (&unchecked, "waive=0:放空管", "no_result"),
            (&unchecked, "unwaive=0", "no_result"),
            (&read_only, "waive=0:放空管", "readonly"),
            (&read_only, "unwaive=3", "readonly"),
            // Grey on every row in a session that may not write.
            (&read_only, "waive=9:放空管", "readonly"),
            (&elsewhere, "waive=0:放空管", "not_on_this_drawing"),
            (&elsewhere, "unwaive=1", "not_on_this_drawing"),
            (&writable, "waive=3:放空管", "already_waived"),
            (&writable, "unwaive=0", "not_waived"),
        ] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        // Finding an object changes no drawing.
        assert_eq!(
            parse_action("locate=0", &read_only),
            Ok(PidVerifyMsg::Locate(0))
        );
        // Rows the list does not have now: no such result, or a waived one
        // while the waived are not shown.
        let unshown = viewed(&VerifyUi::default(), &five);
        for (view, value) in [
            (&writable, "locate=9"),
            (&writable, "waive=5:放空管"),
            (&unshown, "locate=3"),
            (&unshown, "unwaive=3"),
        ] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(refused.code(), "PANEL_ROW_NOT_FOUND", "{value}");
            assert_eq!(refused.reason(), None, "{value}");
        }
        assert_eq!(
            parse_action("locate=9", &writable).map_err(|refused| refused.to_string()),
            Err(
                "规则检查的列表里现在没有下标 9 的一行（筛掉的和没显示的已豁免结果不在列表里）"
                    .to_owned()
            )
        );
    }

    #[test]
    fn pid_verify_action_names_a_wrong_row_value_invalid() {
        let five = five();
        let view = viewed(&VerifyUi::default(), &five);
        for value in [
            "locate",
            "locate=",
            "locate=a",
            "locate=-1",
            "locate=1.5",
            "waive",
            "waive=",
            "waive=0",
            "waive=:放空管",
            "waive=a:放空管",
            "unwaive",
            "unwaive=",
            "unwaive=x",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            parse_action("waive=0", &view).map_err(|refused| refused.to_string()),
            Err("waive 写成 waive=<下标>:<理由>，实得 \"waive=0\"".to_owned())
        );
    }

    /// A row of a bill: one of `kind` written `type_code`, `tag` and `name`,
    /// with no spec, counted one, on PID-1101.
    fn bom_row(kind: ObjectKind, type_code: &str, tag: &str, name: &str) -> BomRow {
        BomRow {
            seq: 0,
            kind,
            type_code: type_code.to_owned(),
            tag: tag.to_owned(),
            name: name.to_owned(),
            spec: String::new(),
            quantity: Quantity::Count(1),
            unit: "个".to_owned(),
            instances: 1,
            drawings: vec!["PID-1101".to_owned()],
        }
    }

    /// A bill of `scope` with `rows` numbered in order and counted by kind.
    fn bill(scope: BomScope, rows: Vec<BomRow>) -> Bom {
        let mut by_kind = bom::ByKind::default();
        for row in &rows {
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
        let rows: Vec<BomRow> = rows
            .into_iter()
            .enumerate()
            .map(|(at, row)| BomRow { seq: at + 1, ..row })
            .collect();
        Bom {
            version: bom::BOM_VERSION,
            generated_at: 1_794_800_000,
            scope,
            source: bom::SourceKind::Live,
            snapshot_seq: None,
            length_unit_assumed: false,
            totals: bom::Totals {
                rows: rows.len(),
                by_kind,
            },
            rows,
            pipe_summary: Vec::new(),
            skipped: Vec::new(),
            notices: Vec::new(),
        }
    }

    /// The bill of PID-1101 the material-list tests read: a pipe line, two
    /// pumps, a gate valve, a pressure transmitter and the fittings of a
    /// run, in the kind order `bom::build` gives.
    fn plant() -> Bom {
        let line = BomRow {
            spec: "A1A DN50".to_owned(),
            quantity: Quantity::Length(12.5),
            unit: "m".to_owned(),
            ..bom_row(
                ObjectKind::PipeLine,
                bom::PIPE_LINE_ROW,
                "L-1001",
                "工艺管线",
            )
        };
        let pump = BomRow {
            spec: "IS80-65-160".to_owned(),
            unit: "台".to_owned(),
            ..bom_row(ObjectKind::Equipment, "PUMP", "P-1001", "离心泵")
        };
        let spare = BomRow {
            unit: "台".to_owned(),
            ..bom_row(ObjectKind::Equipment, "PUMP", "P-1002", "备用泵")
        };
        let fittings = BomRow {
            quantity: Quantity::Count(2),
            instances: 2,
            ..bom_row(
                ObjectKind::Fitting,
                bom::FITTING_ROW,
                "L-1001-01",
                "异径管×2",
            )
        };
        bill(
            BomScope::drawing("PID-1101"),
            vec![
                line,
                pump,
                spare,
                bom_row(ObjectKind::Valve, "GATE_VALVE", "V-1001", "闸阀"),
                bom_row(ObjectKind::Instrument, "PT", "PT-1001", "压力变送器"),
                fittings,
            ],
        )
    }

    /// `bom` kept as the program's bill of `serial`, read off one drawing.
    fn kept_bill(bom: &Bom, serial: u64) -> KeptBom<'_> {
        KeptBom {
            outcome: Ok(bom),
            scope: &bom.scope,
            drawings: 1,
            made_at: bom.generated_at,
            serial,
            stale: false,
        }
    }

    /// Why the tests' bill did not come out: P-1001 twice on PID-1101.
    fn repeated() -> BomFailed {
        let place = |x: f64, handle: &str| crate::pid::project_index::Place {
            drawing_id: "PID-1101".to_owned(),
            at: [x, 0.0],
            handle: handle.to_owned(),
        };
        BomFailed {
            code: bom::BOM_DUPLICATE_TAG.to_owned(),
            text:
                "BOM_DUPLICATE_TAG: 范围内这些（类型编码，位号）不止一处：PUMP P-1001；清单不产出"
                    .to_owned(),
            duplicates: vec![Duplicate {
                type_code: "PUMP".to_owned(),
                tag: "P-1001".to_owned(),
                places: vec![place(0.0, "1A"), place(300.0, "2B")],
            }],
            unreadable: Vec::new(),
        }
    }

    /// `failed` kept as the program's first bill, of `scope`.
    fn kept_failure<'a>(failed: &'a BomFailed, scope: &'a BomScope) -> KeptBom<'a> {
        KeptBom {
            outcome: Err(failed),
            scope,
            drawings: 1,
            made_at: 1_794_800_000,
            serial: 1,
            stale: false,
        }
    }

    /// What the panel draws on the material-list tab under `ui` with `bom`
    /// kept on the drawing in front.
    fn with_bill(ui: &VerifyUi, bom: &Bom) -> VerifyView {
        let drawing = DrawingResults {
            bom: Some(kept_bill(bom, 1)),
            ..DrawingResults::default()
        };
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..ui.clone()
        };
        build_view(&ui, Some(&drawing), None, &in_front())
    }

    /// [`with_bill`] searched for `search`.
    fn searched(bom: &Bom, search: &str) -> VerifyView {
        let ui = VerifyUi {
            bom_search: search.to_owned(),
            ..VerifyUi::default()
        };
        with_bill(&ui, bom)
    }

    /// The table the material-list tab draws.
    fn table_of(view: &VerifyView) -> &BomTable {
        let shown = view.bom.shown.as_ref().expect("a bill shown");
        shown.table.as_ref().expect("a table")
    }

    /// The groups drawn with their counts, and the rows listed by index.
    fn bom_listed(view: &VerifyView) -> (Vec<(&'static str, usize)>, Vec<usize>) {
        let table = table_of(view);
        (
            table
                .groups
                .iter()
                .map(|group| (group.kind, group.count))
                .collect(),
            table.rows.iter().map(|row| row.index).collect(),
        )
    }

    #[test]
    fn the_material_list_is_grouped_by_kind_and_searched_in_four_columns() {
        let plant = plant();
        let view = with_bill(&VerifyUi::default(), &plant);
        assert_eq!(view.empty, None);
        let every = (
            vec![
                ("管线", 1),
                ("设备", 2),
                ("阀门", 1),
                ("仪表", 1),
                ("管件", 1),
            ],
            vec![0, 1, 2, 3, 4, 5],
        );
        assert_eq!(bom_listed(&view), every);
        let table = table_of(&view);
        assert_eq!(table.total, 6);
        assert!(
            !table.across,
            "a drawing's bill names no drawing on its rows"
        );
        assert_eq!(table.rows[0].quantity(), "12.50 m");
        assert_eq!(table.rows[1].quantity(), "1 台");
        // Tag, name, spec and type code, in any case.
        for (search, groups, rows) in [
            ("p-10", vec![("设备", 2)], vec![1, 2]),
            ("闸", vec![("阀门", 1)], vec![3]),
            ("is80", vec![("设备", 1)], vec![1]),
            ("Gate", vec![("阀门", 1)], vec![3]),
            ("l-1001", vec![("管线", 1), ("管件", 1)], vec![0, 5]),
        ] {
            let view = searched(&plant, search);
            assert_eq!(bom_listed(&view), (groups, rows), "{search}");
            assert_eq!(view.bom.search, search);
            // The totals at the bottom are the whole bill's.
            let table = table_of(&view);
            assert_eq!(table.total, 6, "{search}");
            assert_eq!(table.by_kind, every.0, "{search}");
        }
        let view = searched(&plant, "没有这个");
        assert_eq!(bom_listed(&view), (Vec::new(), Vec::new()));
        assert_eq!(view.empty, Some(NONE_SEARCHED));
        // A folded group keeps its count and its rows, and draws none.
        let ui = VerifyUi {
            bom_folded: vec!["设备".to_owned()],
            ..VerifyUi::default()
        };
        let view = with_bill(&ui, &plant);
        assert_eq!(bom_listed(&view), every);
        let folds: Vec<(&str, bool, usize)> = table_of(&view)
            .groups
            .iter()
            .map(|group| (group.kind, group.folded, group.drawn))
            .collect();
        assert_eq!(
            folds,
            [
                ("管线", false, 1),
                ("设备", true, 0),
                ("阀门", false, 1),
                ("仪表", false, 1),
                ("管件", false, 1)
            ]
        );
    }

    #[test]
    fn each_kind_of_the_material_list_draws_200_rows_and_200_more_for_each_press() {
        let mut rows = vec![bom_row(
            ObjectKind::PipeLine,
            bom::PIPE_LINE_ROW,
            "L-1001",
            "工艺管线",
        )];
        rows.extend(
            (1..=450).map(|n| bom_row(ObjectKind::Equipment, "PUMP", &format!("P-{n:04}"), "泵")),
        );
        let many = bill(BomScope::drawing("PID-1101"), rows);
        for (more, drawn, has_more) in [
            (0, 200, true),
            (1, 400, true),
            (2, 450, false),
            (3, 450, false),
        ] {
            let ui = VerifyUi {
                bom_more: more,
                ..VerifyUi::default()
            };
            let view = with_bill(&ui, &many);
            let pumps = &table_of(&view).groups[1];
            assert_eq!(
                (
                    pumps.kind,
                    pumps.count,
                    pumps.first,
                    pumps.drawn,
                    pumps.has_more()
                ),
                ("设备", 450, 1, drawn, has_more),
                "{more}"
            );
        }
    }

    #[test]
    fn the_material_list_says_why_it_has_no_table() {
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        let view = build_view(&ui, None, None, &in_front());
        assert_eq!(
            (view.bom.shown.is_none(), view.empty),
            (true, Some(NEVER_LISTED))
        );
        let empty = bill(BomScope::drawing("PID-1101"), Vec::new());
        let view = with_bill(&VerifyUi::default(), &empty);
        assert_eq!(view.empty, Some(NOTHING_LISTED));
        assert_eq!(table_of(&view).total, 0);
        // Tags found twice are listed in place of the table.
        let failed = repeated();
        let scope = BomScope::drawing("PID-1101");
        let drawing = DrawingResults {
            bom: Some(kept_failure(&failed, &scope)),
            ..DrawingResults::default()
        };
        let view = build_view(&ui, Some(&drawing), None, &in_front());
        let shown = view.bom.shown.as_ref().expect("the failure shown");
        assert_eq!(shown.table, Err(failed.clone()));
        assert_eq!(shown.scope_said, "图 PID-1101");
        assert_eq!(view.empty, None, "what repeats is listed instead");
        // Objects without a tag are listed apart, by kind and place.
        let mut skipping = plant();
        skipping.skipped = vec![bom::Skipped {
            reason: bom::SkipReason::EmptyTag,
            kind: ObjectKind::Instrument,
            type_code: "PT".to_owned(),
            drawing_id: "PID-1101".to_owned(),
            at: [58.0, 80.0],
            handle: "3C".to_owned(),
        }];
        let view = with_bill(&VerifyUi::default(), &skipping);
        assert_eq!(
            table_of(&view).skipped,
            [SkippedView {
                kind: "仪表",
                type_code: "PT".to_owned(),
                drawing_id: "PID-1101".to_owned(),
                at: [58.0, 80.0],
            }]
        );
    }

    #[test]
    fn the_material_list_buttons_are_grey_as_section_4_8_says() {
        let plant = plant();
        let kept = DrawingResults {
            bom: Some(kept_bill(&plant, 1)),
            ..DrawingResults::default()
        };
        let failed = repeated();
        let scope = BomScope::drawing("PID-1101");
        let failing = DrawingResults {
            bom: Some(kept_failure(&failed, &scope)),
            ..DrawingResults::default()
        };
        let pressable = |context: &Context, kept: Option<&DrawingResults<'_>>| {
            let buttons = build_view(&VerifyUi::default(), kept, None, context).buttons;
            (buttons.bom_drawing, buttons.bom_project, buttons.bom_export)
        };
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        assert_eq!(pressable(&in_front(), None), (true, true, false));
        assert_eq!(pressable(&in_front(), Some(&kept)), (true, true, true));
        assert_eq!(pressable(&unsaved, Some(&kept)), (true, false, true));
        assert_eq!(pressable(&busy, Some(&kept)), (false, false, true));
        assert_eq!(
            pressable(&read_only, Some(&kept)),
            (true, true, true),
            "a bill changes no drawing"
        );
        assert_eq!(pressable(&in_front(), Some(&failing)), (true, true, false));
        assert_eq!(pressable(&Context::default(), None), (false, false, false));
    }

    #[test]
    fn the_bom_block_reads_off_the_tab_as_worked_out() {
        let plant = plant();
        let row = |index: usize| {
            let mut row = serde_json::to_value(&plant.rows[index]).expect("a row");
            row["index"] = json!(index);
            row
        };
        let ui = VerifyUi {
            bom_search: "p-10".to_owned(),
            bom_folded: vec!["设备".to_owned()],
            ..VerifyUi::default()
        };
        let state = panel_state(&with_bill(&ui, &plant), ROWS_PER_PAGE, 0);
        assert_eq!(
            state["bom"],
            json!({
                "has_result": true,
                "stale": false,
                "scope": "drawing",
                "search": "p-10",
                "error": null,
                "total": 2,
                "groups": [{ "kind": "设备", "rows": 2, "collapsed": true }],
                "rows": [row(1), row(2)]
            })
        );
        let state = panel_state(&with_bill(&VerifyUi::default(), &plant), 2, 4);
        assert_eq!(state["bom"]["total"], 6);
        assert_eq!(state["bom"]["rows"], json!([row(4), row(5)]));
        // A bill that did not come out: why, and no rows.
        let failed = repeated();
        let scope = BomScope::project();
        let project = ProjectResults {
            bom: Some(KeptBom {
                stale: true,
                ..kept_failure(&failed, &scope)
            }),
            ..ProjectResults::default()
        };
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        let view = build_view(&ui, None, Some(&project), &in_front());
        assert_eq!(
            panel_state(&view, ROWS_PER_PAGE, 0)["bom"],
            json!({
                "has_result": true,
                "stale": true,
                "scope": "project",
                "search": "",
                "error": {
                    "code": "BOM_DUPLICATE_TAG",
                    "text": failed.text,
                    "duplicates": failed.duplicates
                },
                "total": 0,
                "groups": [],
                "rows": []
            })
        );
    }

    #[test]
    fn of_the_drawing_s_and_the_project_s_bill_the_newer_is_shown() {
        let plant = plant();
        let mut whole = plant.clone();
        whole.scope = BomScope::project();
        whole.rows[1].drawings = vec!["PID-1101".to_owned(), "PID-1102".to_owned()];
        let ui = VerifyUi {
            tab: Tab::Bom,
            ..VerifyUi::default()
        };
        let shown = |own: u64, folder: u64| {
            let drawing = DrawingResults {
                bom: Some(kept_bill(&plant, own)),
                ..DrawingResults::default()
            };
            let project = ProjectResults {
                bom: Some(KeptBom {
                    drawings: 2,
                    ..kept_bill(&whole, folder)
                }),
                ..ProjectResults::default()
            };
            let view = build_view(&ui, Some(&drawing), Some(&project), &in_front());
            let shown = view.bom.shown.expect("a bill shown");
            let across = shown.table.as_ref().ok().map(|table| table.across);
            (shown.scope, shown.serial, shown.scope_said, across)
        };
        assert_eq!(
            shown(1, 2),
            (
                bom::ScopeKind::Project,
                2,
                "全项目 2 张图".to_owned(),
                Some(true)
            )
        );
        assert_eq!(
            shown(3, 2),
            (
                bom::ScopeKind::Drawing,
                3,
                "图 PID-1101".to_owned(),
                Some(false)
            )
        );
    }

    #[test]
    fn the_material_row_last_clicked_is_located_while_its_bill_is_shown() {
        let plant = plant();
        let located = |clicked: Option<(u64, usize)>| {
            let ui = VerifyUi {
                bom_located: clicked,
                ..VerifyUi::default()
            };
            let view = with_bill(&ui, &plant);
            let table = table_of(&view);
            let marked: Vec<usize> = table
                .rows
                .iter()
                .filter(|row| row.located)
                .map(|row| row.index)
                .collect();
            (table.located, marked)
        };
        assert_eq!(located(Some((1, 4))), (Some(4), vec![4]));
        assert_eq!(
            located(Some((2, 4))),
            (None, Vec::new()),
            "a row of another bill"
        );
        assert_eq!(located(None), (None, Vec::new()));
    }

    #[test]
    fn pid_verify_action_reads_the_material_list_keys() {
        let plant = plant();
        let view = with_bill(&VerifyUi::default(), &plant);
        for (value, pressed) in [
            ("bom=drawing", PidVerifyMsg::BomDrawing),
            ("bom=project", PidVerifyMsg::BomProject),
            (
                "bom_search=P-10",
                PidVerifyMsg::BomSearch("P-10".to_owned()),
            ),
            ("bom_search=", PidVerifyMsg::BomSearch(String::new())),
            ("bom_search=a=b", PidVerifyMsg::BomSearch("a=b".to_owned())),
            ("bom_group=设备", PidVerifyMsg::BomFold("设备".to_owned())),
            ("bom_locate=5", PidVerifyMsg::BomLocate(5)),
            (
                "bom_export=D:/out/bom.csv",
                PidVerifyMsg::BomExport("D:/out/bom.csv".to_owned()),
            ),
            ("more=bom", PidVerifyMsg::MoreBom),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
        let unlisted = build_view(&VerifyUi::default(), None, None, &in_front());
        assert_eq!(
            parse_action("bom=drawing", &unlisted),
            Ok(PidVerifyMsg::BomDrawing)
        );
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_material_list_button_with_the_reason() {
        let plant = plant();
        let unlisted = build_view(&VerifyUi::default(), None, None, &in_front());
        let failed = repeated();
        let scope = BomScope::drawing("PID-1101");
        let drawing = DrawingResults {
            bom: Some(kept_failure(&failed, &scope)),
            ..DrawingResults::default()
        };
        let failing = build_view(&VerifyUi::default(), Some(&drawing), None, &in_front());
        for (view, value) in [
            (&unlisted, "bom_search=P"),
            (&unlisted, "bom_group=设备"),
            (&unlisted, "bom_locate=0"),
            (&unlisted, "bom_export=a.csv"),
            (&unlisted, "more=bom"),
            (&failing, "bom_search=P"),
            (&failing, "bom_locate=0"),
            (&failing, "bom_export=a.csv"),
        ] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some("no_result")),
                "{value}"
            );
        }
        for (context, value, reason) in [
            (Context::default(), "bom=drawing", "no_drawing"),
            (Context::default(), "bom=project", "no_drawing"),
            (
                Context {
                    busy: SCANNING,
                    ..in_front()
                },
                "bom=drawing",
                "busy",
            ),
            (
                Context {
                    saved: false,
                    ..in_front()
                },
                "bom=project",
                "no_project_dir",
            ),
        ] {
            let view = build_view(&VerifyUi::default(), None, None, &context);
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        let whole = with_bill(&VerifyUi::default(), &plant);
        let pumps = searched(&plant, "p-10");
        for (view, value) in [(&whole, "bom_locate=6"), (&pumps, "bom_locate=0")] {
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ROW_NOT_FOUND", None),
                "{value}"
            );
        }
        assert_eq!(
            parse_action("bom_locate=0", &pumps).map_err(|refused| refused.to_string()),
            Err("材料清单的列表里现在没有下标 0 的一行（搜索筛掉的不在列表里）".to_owned())
        );
    }

    #[test]
    fn pid_verify_action_names_a_wrong_material_list_value_invalid() {
        let plant = plant();
        let view = with_bill(&VerifyUi::default(), &plant);
        for value in [
            "bom",
            "bom=",
            "bom=all",
            "bom_search",
            "bom_group",
            "bom_group=",
            "bom_locate",
            "bom_locate=a",
            "bom_locate=-1",
            "bom_export",
            "bom_export=",
            "bom_export= ",
            "more",
            "more=publish",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
        assert_eq!(
            parse_action("bom_group=泵", &view).map_err(|refused| refused.to_string()),
            Err("清单里没有种类「泵」的组；有的是：管线、设备、阀门、仪表、管件".to_owned())
        );
    }

    /// A snapshot of PID-1101 numbered `seq`, saved for `reason`, its file
    /// there or gone.
    fn info(seq: u64, reason: &str, status: SnapshotStatus) -> SnapshotInfo {
        SnapshotInfo {
            seq,
            file: format!("{seq:04}.json"),
            created_at: 1_794_800_000 + seq * 3_600,
            created_by: "设计员".to_owned(),
            reason: reason.to_owned(),
            objects: 12,
            bytes: 4_096,
            sha256: "0".repeat(64),
            status,
        }
    }

    /// The snapshots of PID-1101 the snapshot tests list: the third's file
    /// is gone.
    fn three() -> Vec<SnapshotInfo> {
        vec![
            info(1, "送审前", SnapshotStatus::Present),
            info(2, "", SnapshotStatus::Present),
            info(3, "改版", SnapshotStatus::Missing),
        ]
    }

    /// What the panel draws on 「快照与发布」 with `picked` ticked, and
    /// `snapshots` and `diff` kept on the drawing in front, in `context`.
    fn delivered(
        picked: &[Pick],
        snapshots: &[SnapshotInfo],
        diff: Option<&Diff>,
        context: &Context,
    ) -> VerifyView {
        let drawing = DrawingResults {
            snapshots: Some(snapshots),
            diff,
            ..DrawingResults::default()
        };
        let ui = VerifyUi {
            tab: Tab::Deliver,
            picked: picked.to_vec(),
            ..VerifyUi::default()
        };
        build_view(&ui, Some(&drawing), None, context)
    }

    /// An object of PID-1101 in a diff: `kind`, `type_code` and `tag` at
    /// `at`, with `changes`.
    fn diffed(
        kind: &str,
        type_code: &str,
        tag: &str,
        at: [f64; 2],
        changes: Vec<Change>,
    ) -> DiffObject {
        DiffObject {
            key: format!("{type_code}|{tag}"),
            kind: kind.to_owned(),
            type_code: type_code.to_owned(),
            tag: tag.to_owned(),
            at,
            handle: "1A".to_owned(),
            changes,
        }
    }

    /// A side of a diff of PID-1101: snapshot `seq`, or the drawing as it is.
    fn side(seq: Option<u64>) -> DiffSide {
        DiffSide {
            reference: seq.map_or_else(|| "live".to_owned(), |seq| seq.to_string()),
            seq,
            drawing_id: "PID-1101".to_owned(),
            created_at: 1_794_800_000,
            objects: 12,
        }
    }

    /// Counts of a diff: `added`, `removed` and `modified` objects.
    fn counted(added: usize, removed: usize, modified: usize) -> DiffSummary {
        DiffSummary {
            added,
            removed,
            modified,
            by_what: snapshot::ByWhat::default(),
            edges_added: 0,
            edges_removed: 0,
        }
    }

    /// The diff of snapshot 1 and the drawing as it is that the snapshot
    /// tests read: a transmitter added, a valve of no tag removed, the pump
    /// given another design pressure and the line made longer and joined to
    /// the valve V-1002.
    fn compared() -> Diff {
        let pressure = Change::Property {
            field: "设计压力".to_owned(),
            old: Some("1.6MPa".to_owned()),
            new: Some("2.5MPa".to_owned()),
        };
        let length = Change::Length {
            old: 12.5,
            new: 14.0,
        };
        let joined = Change::Connection {
            change: EdgeChange::Added,
            with: "V-1002".to_owned(),
            port: Some("N2".to_owned()),
        };
        Diff {
            version: snapshot::DIFF_VERSION,
            a: side(Some(1)),
            b: side(None),
            identical: false,
            summary: counted(1, 1, 2),
            added: vec![diffed("仪表", "PT", "PT-1001", [60.0, 80.0], Vec::new())],
            removed: vec![diffed("阀门", "GATE_VALVE", "", [-100.0, 60.0], Vec::new())],
            modified: vec![
                diffed("设备", "PUMP", "P-1001", [0.0, 0.0], vec![pressure]),
                diffed(
                    "管线",
                    "PIPE_LINE",
                    "L-1001",
                    [100.0, 0.0],
                    vec![length, joined],
                ),
            ],
            edges: snapshot::DiffEdges::default(),
            notices: vec![snapshot::Notice {
                code: snapshot::LENGTH_UNIT_DIFFERS,
                message: "两份快照的图纸单位不同".to_owned(),
            }],
        }
    }

    #[test]
    fn the_snapshot_controls_report_under_the_ids_the_spec_gives_them() {
        assert_eq!(Pick::Seq(3).bounds_id(), "pid-verify-snapshot-row-3");
        assert_eq!(Pick::Live.bounds_id(), "pid-verify-snapshot-row-live");
        assert_eq!(DIFF_ID, "pid-verify-diff");
        assert_eq!(
            DiffGroup::Modified.item_id(0),
            "pid-verify-diff-item-modified-0"
        );
        for id in [
            SNAPSHOT_ROW_PREFIX,
            SNAPSHOT_REASON_ID,
            SNAPSHOT_SAVE_ID,
            DIFF_EXPORT_ID,
            DIFF_ITEM_PREFIX,
        ] {
            assert!(id.starts_with(BOUNDS_PREFIX), "{id}");
        }
    }

    #[test]
    fn the_snapshot_list_is_newest_first_and_a_file_gone_cannot_be_ticked() {
        let three = three();
        // 3 was ticked before its file went: it no longer counts, nor does
        // a number the list does not have.
        let ticked = [Pick::Seq(3), Pick::Seq(9), Pick::Seq(1)];
        let view = delivered(&ticked, &three, None, &in_front());
        assert_eq!(view.empty, None);
        let deliver = &view.deliver;
        assert!(deliver.saved);
        let rows: Vec<(u64, bool, bool)> = deliver
            .rows
            .iter()
            .map(|row| (row.info.seq, row.picked, row.missing()))
            .collect();
        assert_eq!(
            rows,
            [(3, false, true), (2, false, false), (1, true, false)]
        );
        assert_eq!(deliver.picked, [Pick::Seq(1)]);
        assert!(!deliver.live_picked);
        let view = delivered(&[Pick::Live], &three, None, &in_front());
        assert!(view.deliver.live_picked);
        assert_eq!(view.deliver.picked, [Pick::Live]);
    }

    #[test]
    fn diff_can_be_pressed_with_two_rows_ticked_and_compares_the_older_first() {
        use Pick::{Live, Seq};
        let three = three();
        let view = |picked: &[Pick]| delivered(picked, &three, None, &in_front());
        let pressable = |picked: &[Pick]| view(picked).buttons.diff;
        assert!(!pressable(&[]));
        assert!(!pressable(&[Seq(1)]));
        assert!(pressable(&[Seq(2), Seq(1)]));
        assert!(!pressable(&[Seq(2), Seq(1), Live]));
        // A file gone ticked is not ticked.
        assert!(!pressable(&[Seq(3), Seq(1)]));
        let pair = |picked: &[Pick]| view(picked).deliver.pair();
        assert_eq!(pair(&[Seq(2), Seq(1)]), Some((Seq(1), Seq(2))));
        assert_eq!(pair(&[Seq(1), Seq(2)]), Some((Seq(1), Seq(2))));
        assert_eq!(pair(&[Live, Seq(2)]), Some((Seq(2), Live)));
        assert_eq!(pair(&[Seq(1), Live]), Some((Seq(1), Live)));
        assert_eq!(pair(&[Live]), None);
        assert_eq!(pair(&[Seq(1), Seq(2), Live]), None);
    }

    #[test]
    fn the_snapshot_buttons_are_grey_as_section_4_8_says() {
        let three = three();
        let diff = compared();
        let saving =
            |context: &Context| delivered(&[], &three, None, context).buttons.snapshot_save;
        assert!(saving(&in_front()));
        // A read-only session keeps snapshots all the same (§5.4).
        assert!(saving(&Context {
            writable: false,
            ..in_front()
        }));
        assert!(!saving(&Context {
            busy: SCANNING,
            ..in_front()
        }));
        assert!(!saving(&Context {
            saved: false,
            ..in_front()
        }));
        assert!(!saving(&Context::default()));
        // Only a diff shown can be exported.
        let picked = [Pick::Seq(1), Pick::Live];
        let view = delivered(&picked, &three, None, &in_front());
        assert!(!view.buttons.diff_export);
        let view = delivered(&picked, &three, Some(&diff), &in_front());
        assert!(view.buttons.diff_export);
    }

    #[test]
    fn the_snapshot_half_says_why_it_has_no_list() {
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        let view = delivered(&[], &[], None, &unsaved);
        assert_eq!(view.empty, Some(UNSAVED_SAID));
        assert!(!view.deliver.saved);
        let view = delivered(&[], &[], None, &Context::default());
        assert_eq!(view.empty, Some(NO_DRAWING_SAID));
        // A saved drawing with no snapshot yet lists the drawing as it is.
        let view = delivered(&[], &[], None, &in_front());
        assert_eq!(view.empty, None);
        assert!(view.deliver.saved);
        assert!(view.deliver.rows.is_empty());
    }

    #[test]
    fn the_diff_is_counted_and_listed_in_three_groups_a_line_for_each_change() {
        let three = three();
        let diff = compared();
        let picked = [Pick::Seq(1), Pick::Live];
        let view = delivered(&picked, &three, Some(&diff), &in_front());
        let shown = view.deliver.diff.as_ref().expect("a diff shown");
        assert_eq!((shown.a, shown.b), (Pick::Seq(1), Pick::Live));
        assert!(!shown.identical);
        assert_eq!(shown.summary, diff.summary);
        assert_eq!(shown.notices, diff.notices);
        let groups: Vec<(DiffGroup, usize, usize)> = shown
            .groups
            .iter()
            .map(|group| (group.group, group.count, group.drawn))
            .collect();
        assert_eq!(
            groups,
            [
                (DiffGroup::Added, 1, 1),
                (DiffGroup::Removed, 1, 1),
                (DiffGroup::Modified, 2, 2)
            ]
        );
        let items: Vec<(&str, Vec<&str>)> = shown
            .groups
            .iter()
            .flat_map(|group| &group.items)
            .map(|item| {
                let changes = item.changes.iter().map(String::as_str).collect();
                (item.label.as_str(), changes)
            })
            .collect();
        assert_eq!(
            items,
            [
                ("仪表 PT PT-1001", vec![]),
                ("阀门 GATE_VALVE （无位号）", vec![]),
                ("设备 PUMP P-1001", vec!["属性 设计压力：1.6MPa → 2.5MPa"]),
                (
                    "管线 PIPE_LINE L-1001",
                    vec!["长度：12.5 → 14.0", "连接：新增，接到 V-1002（本端 N2）"]
                ),
            ]
        );
        // Two sides alike say so, with no group.
        let alike = Diff {
            identical: true,
            summary: counted(0, 0, 0),
            added: Vec::new(),
            removed: Vec::new(),
            modified: Vec::new(),
            notices: Vec::new(),
            ..compared()
        };
        let view = delivered(&picked, &three, Some(&alike), &in_front());
        let shown = view.deliver.diff.as_ref().expect("a diff shown");
        assert!(shown.identical);
        assert!(shown.groups.is_empty());
    }

    #[test]
    fn each_change_reads_as_section_4_6_writes_it() {
        let said: Vec<String> = [
            Change::Property {
                field: "设计温度".to_owned(),
                old: None,
                new: Some("80".to_owned()),
            },
            Change::Position {
                old: vec![0.0, 0.0],
                new: vec![10.0, 2.5],
            },
            Change::Length {
                old: 100.0,
                new: 100.25,
            },
            Change::Fittings {
                symbol: "弯头".to_owned(),
                old: 2,
                new: 3,
            },
            Change::Connection {
                change: EdgeChange::Removed,
                with: "P-1001".to_owned(),
                port: None,
            },
            Change::Peer {
                field: "peer_drawing".to_owned(),
                old: Some("PID-1102".to_owned()),
                new: Some(String::new()),
            },
        ]
        .iter()
        .map(change_said)
        .collect();
        assert_eq!(
            said,
            [
                "属性 设计温度：（空） → 80",
                "位置：(0.0, 0.0) → (10.0, 2.5)",
                "长度：100.0 → 100.25",
                "管件 弯头：2 → 3 个",
                "连接：删除，接到 P-1001",
                "对端 peer_drawing：PID-1102 → （空）",
            ]
        );
    }

    #[test]
    fn each_group_of_the_diff_draws_200_objects_and_200_more_for_each_press() {
        let three = three();
        let many = (0..450)
            .map(|n| {
                let tag = format!("P-{n:04}");
                diffed("设备", "PUMP", &tag, [f64::from(n), 0.0], Vec::new())
            })
            .collect();
        let diff = Diff {
            added: many,
            ..compared()
        };
        let drawing = DrawingResults {
            snapshots: Some(&three),
            diff: Some(&diff),
            ..DrawingResults::default()
        };
        let drawn = |more: usize| {
            let ui = VerifyUi {
                tab: Tab::Deliver,
                picked: vec![Pick::Seq(1), Pick::Live],
                diff_more: more,
                diff_located: Some((DiffGroup::Added, 201)),
                ..VerifyUi::default()
            };
            let view = build_view(&ui, Some(&drawing), None, &in_front());
            let shown = view.deliver.diff.expect("a diff shown");
            let added = &shown.groups[0];
            let located = added.items.iter().filter(|item| item.located);
            let located: Vec<usize> = located.map(|item| item.index).collect();
            (added.count, added.drawn, located)
        };
        assert_eq!(drawn(0), (450, 200, vec![201]));
        assert_eq!(drawn(1).1, 400);
        assert_eq!(drawn(2).1, 450);
    }

    #[test]
    fn the_deliver_block_reads_off_the_tab_as_worked_out() {
        let three = three();
        let diff = compared();
        let view = delivered(
            &[Pick::Live, Pick::Seq(1)],
            &three,
            Some(&diff),
            &in_front(),
        );
        let state = panel_state(&view, ROWS_PER_PAGE, 0);
        let deliver = &state["deliver"];
        let rows = deliver["snapshots"].as_array().cloned().unwrap_or_default();
        let seqs: Vec<Option<u64>> = rows.iter().map(|row| row["seq"].as_u64()).collect();
        assert_eq!(seqs, [Some(3), Some(2), Some(1)], "{state}");
        let mut first = serde_json::to_value(&three[0]).expect("a snapshot as JSON");
        first["picked"] = json!(true);
        assert_eq!(rows[2], first, "{state}");
        assert_eq!(rows[0]["status"], "missing", "{state}");
        assert_eq!(rows[0]["picked"], false, "{state}");
        assert_eq!(deliver["picked"], json!(["live", 1]), "{state}");
        assert_eq!(
            deliver["diff"],
            json!({"a": 1, "b": "live", "identical": false, "新增": 1, "删除": 1, "修改": 2}),
            "{state}"
        );
        // Its publish records were not read: no status line.
        assert!(deliver["publish"].is_object(), "{state}");
        assert_eq!(deliver["publish"]["state"], Value::Null, "{state}");
        for button in ["snapshot_save", "diff", "diff_export"] {
            assert_eq!(state["buttons"][button], true, "{button}: {state}");
        }
    }

    #[test]
    fn pid_verify_action_reads_the_snapshot_keys() {
        use Pick::{Live, Seq};
        let three = three();
        let diff = compared();
        let view = delivered(&[Seq(1), Live], &three, Some(&diff), &in_front());
        for (value, pressed) in [
            (
                "snapshot_save=送审前",
                PidVerifyMsg::SnapshotSave("送审前".to_owned()),
            ),
            ("snapshot_save=", PidVerifyMsg::SnapshotSave(String::new())),
            ("pick=2", PidVerifyMsg::Pick(Seq(2))),
            ("pick=live", PidVerifyMsg::Pick(Live)),
            ("pick=LIVE", PidVerifyMsg::Pick(Live)),
            ("diff", PidVerifyMsg::Diff(Seq(1), Live)),
            (
                "diff_locate=modified:1",
                PidVerifyMsg::DiffLocate(DiffGroup::Modified, 1),
            ),
            (
                "diff_locate=removed:0",
                PidVerifyMsg::DiffLocate(DiffGroup::Removed, 0),
            ),
            (
                "diff_export=D:/out/比对.json",
                PidVerifyMsg::DiffExport("D:/out/比对.json".to_owned()),
            ),
            ("more=diff", PidVerifyMsg::MoreDiff),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_snapshot_button_with_the_reason() {
        let three = three();
        let diff = compared();
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        for (context, value, reason) in [
            (Context::default(), "snapshot_save=送审前", "no_drawing"),
            (busy, "snapshot_save=送审前", "busy"),
            (unsaved.clone(), "snapshot_save=送审前", "no_project_dir"),
            (Context::default(), "pick=live", "no_drawing"),
            (unsaved.clone(), "pick=live", "no_project_dir"),
            (unsaved, "pick=1", "no_project_dir"),
        ] {
            let view = delivered(&[], &three, None, &context);
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        // 【比对】 wants exactly two rows ticked.
        let ticked: [&[Pick]; 3] = [
            &[],
            &[Pick::Seq(1)],
            &[Pick::Seq(1), Pick::Seq(2), Pick::Live],
        ];
        for picked in ticked {
            let view = delivered(picked, &three, None, &in_front());
            let refused = parse_action("diff", &view).expect_err("diff");
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some("not_two_picked")),
                "{picked:?}"
            );
        }
        // A file gone cannot be ticked; a number the list does not have is
        // no row of it.
        let view = delivered(&[], &three, None, &in_front());
        let refused = parse_action("pick=3", &view).expect_err("pick=3");
        assert_eq!(
            (refused.code(), refused.reason()),
            ("PANEL_ACTION_UNAVAILABLE", Some("missing"))
        );
        assert_eq!(
            parse_action("pick=9", &view).map_err(|refused| (refused.code(), refused.to_string())),
            Err((
                "PANEL_ROW_NOT_FOUND",
                "快照列表里现在没有第 9 份".to_owned()
            ))
        );
        // Nothing compared: nothing to find, page or export.
        for value in ["diff_locate=added:0", "diff_export=a.json", "more=diff"] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some("no_result")),
                "{value}"
            );
        }
        let shown = delivered(
            &[Pick::Seq(1), Pick::Live],
            &three,
            Some(&diff),
            &in_front(),
        );
        assert_eq!(
            parse_action("diff_locate=added:1", &shown)
                .map_err(|refused| (refused.code(), refused.to_string())),
            Err((
                "PANEL_ROW_NOT_FOUND",
                "比对结果的「新增」组里现在没有下标 1 的一项".to_owned()
            ))
        );
    }

    #[test]
    fn pid_verify_action_names_a_wrong_snapshot_value_invalid() {
        let three = three();
        let diff = compared();
        let view = delivered(
            &[Pick::Seq(1), Pick::Live],
            &three,
            Some(&diff),
            &in_front(),
        );
        for value in [
            "snapshot_save",
            "pick",
            "pick=",
            "pick=first",
            "pick=-1",
            "diff=1",
            "diff_locate",
            "diff_locate=",
            "diff_locate=modified",
            "diff_locate=changed:0",
            "diff_locate=modified:x",
            "diff_export",
            "diff_export=",
            "diff_export= ",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
    }

    /// The package of PID-1101 of `revision` in its default target folder,
    /// made of the file on disk or not.
    fn package(revision: u32, current: bool) -> Package {
        let package_id = format!("PID-1101-R{revision}");
        Package {
            path: format!("D:/proj/pid-publish/{package_id}"),
            package_id,
            revision,
            published_at: Some(1_794_900_000 + i64::from(revision) * 3_600),
            published_by: Some("设计员".to_owned()),
            current,
        }
    }

    /// The records of PID-1101 in its default target folder: `packages`,
    /// newest first.
    fn records(packages: Vec<Package>) -> Records {
        let next_revision = packages.first().map_or(1, |latest| latest.revision + 1);
        Records {
            drawing_id: "PID-1101".to_owned(),
            target_dir: "D:/proj/pid-publish".to_owned(),
            next_revision,
            packages,
        }
    }

    /// Gates with `errors` errors not waived and `repeated` tags found more
    /// than once; the master data not set up, every drawing read.
    fn gates(errors: usize, repeated: usize) -> Gates {
        let passed = errors == 0 && repeated == 0;
        Gates {
            drc_errors: errors,
            drc_warnings: 1,
            drc_hints: 0,
            waived: 0,
            duplicate_tags: repeated,
            master_data: MasterDataState::NotConfigured,
            index_complete: true,
            result: if passed {
                crate::pid::publish_pkg::PASSED
            } else {
                crate::pid::publish_pkg::PUBLISH_BLOCKED
            },
        }
    }

    /// `records` read, the file on disk the one the program read or wrote.
    fn read(records: &Records) -> KeptPublish<'_> {
        KeptPublish {
            records: Some(Ok(records)),
            file_unchanged: true,
            ..KeptPublish::default()
        }
    }

    /// What the panel draws under `ui` with `kept` of publishing on the
    /// drawing in front, in `context`.
    fn published(kept: KeptPublish<'_>, ui: &VerifyUi, context: &Context) -> VerifyView {
        let drawing = DrawingResults {
            publish: kept,
            ..DrawingResults::default()
        };
        build_view(ui, Some(&drawing), None, context)
    }

    /// The interface state with 「快照与发布」 in front.
    fn on_deliver() -> VerifyUi {
        VerifyUi {
            tab: Tab::Deliver,
            ..VerifyUi::default()
        }
    }

    /// The publish half of `view`.
    fn half(view: VerifyView) -> PublishView {
        view.deliver.publish.expect("a publish half")
    }

    #[test]
    fn the_publish_controls_report_under_the_ids_the_spec_gives_them() {
        assert_eq!(PUBLISH_ID, "pid-verify-publish");
        let row = PackageRow {
            package: package(3, false),
            verified: None,
        };
        assert_eq!(row.bounds_id(), "pid-verify-package-3");
        for id in [
            PUBLISH_DIR_ID,
            PUBLISH_CHECK_ID,
            PUBLISH_CONFIRM_ID,
            PACKAGE_ROW_PREFIX,
        ] {
            assert!(id.starts_with(BOUNDS_PREFIX), "{id}");
        }
    }

    #[test]
    fn the_status_line_follows_the_records_and_the_changes_not_saved() {
        let ui = on_deliver();
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let said = |records: &Records, context: &Context| {
            let publish = half(published(read(records), &ui, context));
            let current: Vec<bool> = publish
                .packages
                .iter()
                .map(|row| row.package.current)
                .collect();
            (publish.state, current)
        };
        let none = records(Vec::new());
        assert_eq!(
            said(&none, &in_front()),
            (Some("未发布".to_owned()), vec![])
        );
        let two = records(vec![package(2, true), package(1, false)]);
        assert_eq!(
            said(&two, &in_front()),
            (Some("已发布，修订 2".to_owned()), vec![true, false])
        );
        assert_eq!(
            said(&two, &dirty),
            (Some("修订 2 之后有改动".to_owned()), vec![false, false])
        );
        // The file on disk is the one revision 1 was made of: it changed
        // since revision 2.
        let back = records(vec![package(2, false), package(1, true)]);
        assert_eq!(
            said(&back, &in_front()),
            (Some("修订 2 之后有改动".to_owned()), vec![false, true])
        );
        let publish = half(published(read(&two), &ui, &in_front()));
        assert_eq!(
            (publish.target_dir.as_deref(), publish.next_revision),
            (Some("D:/proj/pid-publish"), Some(3))
        );
        // Not read yet, or not readable: no status line, and why.
        let unread = half(published(KeptPublish::default(), &ui, &in_front()));
        assert_eq!(
            (unread.state, unread.target_dir, unread.next_revision),
            (None, None, None)
        );
        let why = "当前图不在项目的图纸里";
        let kept = KeptPublish {
            records: Some(Err(why)),
            ..KeptPublish::default()
        };
        let unlisted = half(published(kept, &ui, &in_front()));
        assert_eq!(
            (unlisted.state, unlisted.unlisted.as_deref()),
            (None, Some(why))
        );
        // No drawing in front: no publish half.
        let none_in_front = published(read(&two), &ui, &Context::default());
        assert_eq!(none_in_front.deliver.publish, None);
    }

    #[test]
    fn publish_waits_for_the_four_preconditions_and_the_check_for_a_saved_drawing() {
        let one = records(vec![package(1, true)]);
        let ui = on_deliver();
        let pressable = |kept: KeptPublish<'_>, context: &Context| {
            let buttons = published(kept, &ui, context).buttons;
            (buttons.publish_check, buttons.publish)
        };
        let changed = KeptPublish {
            file_unchanged: false,
            ..read(&one)
        };
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let read_only = Context {
            writable: false,
            ..in_front()
        };
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        assert_eq!(pressable(read(&one), &in_front()), (true, true));
        assert_eq!(pressable(changed, &in_front()), (true, false));
        assert_eq!(pressable(read(&one), &dirty), (true, false));
        assert_eq!(pressable(read(&one), &read_only), (true, false));
        assert_eq!(pressable(read(&one), &busy), (false, false));
        assert_eq!(pressable(read(&one), &unsaved), (false, false));
        assert_eq!(pressable(read(&one), &Context::default()), (false, false));
        // The gates do not grey it: publishing checks them again.
        let found = GatesFound {
            gates: gates(2, 0),
            duplicates: Vec::new(),
            can_publish: false,
        };
        let blocked = KeptPublish {
            gates: Some((&found, false)),
            ..read(&one)
        };
        assert_eq!(pressable(blocked, &in_front()), (true, true));
        // A drawing never saved: no file of it was read or written either,
        // and it has no records.
        let publish = half(published(read(&one), &ui, &unsaved));
        assert_eq!(
            publish.preconditions,
            Preconditions {
                saved: false,
                clean: true,
                file_unchanged: false,
                writable_session: true
            }
        );
        assert_eq!(publish.packages, Vec::new());
    }

    #[test]
    fn the_eight_conditions_say_whether_each_holds_and_the_gates_wait_for_a_check() {
        let one = records(vec![package(1, true)]);
        let ui = on_deliver();
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let unchecked = half(published(read(&one), &ui, &dirty));
        let said = |pairs: &[(&str, Option<bool>)]| -> Vec<(String, Option<bool>)> {
            pairs
                .iter()
                .map(|&(label, met)| (label.to_owned(), met))
                .collect()
        };
        assert_eq!(
            unchecked.conditions(),
            said(&[
                ("已保存", Some(true)),
                ("没有未存的修改", Some(false)),
                ("文件没有在别处被改过", Some(true)),
                ("会话可写", Some(true)),
                ("没被豁免的错误为 0", None),
                ("重复位号为 0", None),
                ("主数据读得出来", None),
                ("项目里的图都读得出来", None),
            ])
        );
        let mut unreadable = gates(2, 1);
        unreadable.master_data = MasterDataState::Unreadable;
        unreadable.index_complete = false;
        let found = GatesFound {
            gates: unreadable,
            duplicates: repeated().duplicates,
            can_publish: false,
        };
        let kept = KeptPublish {
            gates: Some((&found, false)),
            ..read(&one)
        };
        let blocked = half(published(kept, &ui, &in_front()));
        assert_eq!(
            blocked.conditions()[4..],
            said(&[
                ("没被豁免的错误为 0（2 条）", Some(false)),
                ("重复位号为 0（1 个）", Some(false)),
                ("主数据读得出来", Some(false)),
                ("项目里的图都读得出来", Some(false)),
            ])[..]
        );
        // Master data not set up does not block on its own.
        let found = GatesFound {
            gates: gates(0, 0),
            duplicates: Vec::new(),
            can_publish: true,
        };
        let kept = KeptPublish {
            gates: Some((&found, false)),
            ..read(&one)
        };
        let passed = half(published(kept, &ui, &in_front()));
        assert_eq!(
            passed.conditions()[4..],
            said(&[
                ("没被豁免的错误为 0", Some(true)),
                ("重复位号为 0", Some(true)),
                ("主数据读得出来（没有配置）", Some(true)),
                ("项目里的图都读得出来", Some(true)),
            ])[..]
        );
    }

    #[test]
    fn the_publish_block_reads_off_the_half_as_worked_out() {
        let one = records(vec![package(1, true)]);
        let found = GatesFound {
            gates: gates(2, 1),
            duplicates: repeated().duplicates,
            can_publish: false,
        };
        let verified = [(
            1,
            Verified {
                intact: true,
                text: INTACT_SAID.to_owned(),
            },
        )];
        let kept = KeptPublish {
            gates: Some((&found, true)),
            verified: &verified,
            last_package: Some("D:/proj/pid-publish/PID-1101-R1"),
            ..read(&one)
        };
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let state = panel_state(&published(kept, &on_deliver(), &dirty), ROWS_PER_PAGE, 0);
        let first = &one.packages[0];
        assert_eq!(
            state["deliver"]["publish"],
            json!({
                "state": "修订 1 之后有改动",
                "target_dir": "D:/proj/pid-publish",
                "next_revision": 2,
                "preconditions": {
                    "saved": true,
                    "clean": false,
                    "file_unchanged": true,
                    "writable_session": true
                },
                "checked": true,
                "stale": true,
                "can_publish": false,
                "gates": found.gates,
                "packages": [{
                    "package_id": first.package_id,
                    "revision": 1,
                    "path": first.path,
                    "published_at": first.published_at,
                    "published_by": "设计员",
                    "current": false,
                    "verified": { "intact": true, "text": "完好" }
                }],
                "last_package": "D:/proj/pid-publish/PID-1101-R1"
            }),
            "{state}"
        );
        assert_eq!(state["buttons"]["publish_check"], true, "{state}");
        assert_eq!(state["buttons"]["publish"], false, "{state}");
        // Never checked: the gates, and whether it could be published, are
        // not known.
        let unchecked = published(read(&one), &on_deliver(), &in_front());
        let state = panel_state(&unchecked, ROWS_PER_PAGE, 0);
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["checked"], false, "{state}");
        assert_eq!(publish["stale"], false, "{state}");
        assert_eq!(publish["can_publish"], Value::Null, "{state}");
        assert_eq!(publish["gates"], Value::Null, "{state}");
        assert_eq!(publish["packages"][0]["verified"], Value::Null, "{state}");
        assert_eq!(publish["packages"][0]["current"], true, "{state}");
        assert_eq!(state["buttons"]["publish"], true, "{state}");
        // No drawing in front.
        let none = published(read(&one), &on_deliver(), &Context::default());
        let state = panel_state(&none, ROWS_PER_PAGE, 0);
        assert_eq!(state["deliver"]["publish"], Value::Null, "{state}");
    }

    #[test]
    fn publish_asks_in_the_panel_naming_the_drawing_the_revision_and_the_folder() {
        let two = records(vec![package(2, true)]);
        let asking = VerifyUi {
            confirming: true,
            ..on_deliver()
        };
        let asked =
            |ui: &VerifyUi, context: &Context| half(published(read(&two), ui, context)).asking;
        assert_eq!(
            asked(&asking, &in_front()).as_deref(),
            Some("把 PID-1101 发布为修订 3 到 D:/proj/pid-publish？")
        );
        assert_eq!(asked(&on_deliver(), &in_front()), None);
        // Asked of a drawing that cannot be published now: no question.
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        assert_eq!(asked(&asking, &dirty), None);
        assert_eq!(asked(&asking, &busy), None);
    }

    #[test]
    fn pid_verify_action_reads_the_publish_keys() {
        let two = records(vec![package(2, true), package(1, false)]);
        let view = published(read(&two), &on_deliver(), &in_front());
        for (value, pressed) in [
            (
                "publish_dir=D:/out",
                PidVerifyMsg::PublishDir("D:/out".to_owned()),
            ),
            (
                "publish_dir= 发布包 ",
                PidVerifyMsg::PublishDir("发布包".to_owned()),
            ),
            ("publish_dir=", PidVerifyMsg::PublishDir(String::new())),
            ("publish_check", PidVerifyMsg::PublishCheck),
            ("publish", PidVerifyMsg::Publish),
            ("package_verify=1", PidVerifyMsg::PackageVerify(1)),
            ("package_verify= 2", PidVerifyMsg::PackageVerify(2)),
        ] {
            assert_eq!(parse_action(value, &view), Ok(pressed), "{value}");
        }
    }

    #[test]
    fn pid_verify_action_refuses_a_grey_publish_button_with_the_reason() {
        let one = records(vec![package(1, true)]);
        let ui = on_deliver();
        let busy = Context {
            busy: SCANNING,
            ..in_front()
        };
        let unsaved = Context {
            saved: false,
            ..in_front()
        };
        let dirty = Context {
            dirty: true,
            ..in_front()
        };
        let read_only = Context {
            writable: false,
            dirty: true,
            ..in_front()
        };
        let changed = KeptPublish {
            file_unchanged: false,
            ..read(&one)
        };
        let none = Context::default();
        for (kept, context, value, reason) in [
            (read(&one), &none, "publish_dir=D:/out", "no_drawing"),
            (read(&one), &none, "publish_check", "no_drawing"),
            (read(&one), &none, "publish", "no_drawing"),
            (read(&one), &none, "package_verify=1", "no_drawing"),
            (read(&one), &busy, "publish_check", "busy"),
            (read(&one), &busy, "publish", "busy"),
            (read(&one), &busy, "package_verify=1", "busy"),
            (read(&one), &unsaved, "publish_check", "no_project_dir"),
            (read(&one), &unsaved, "publish", "precondition"),
            (read(&one), &dirty, "publish", "precondition"),
            (changed, &in_front(), "publish", "precondition"),
            (read(&one), &read_only, "publish", "readonly"),
        ] {
            let view = published(kept, &ui, context);
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value} {context:?}"
            );
        }
        // A revision the records do not have.
        let view = published(read(&one), &ui, &in_front());
        assert_eq!(
            parse_action("package_verify=3", &view)
                .map_err(|refused| (refused.code(), refused.to_string())),
            Err((
                "PANEL_ROW_NOT_FOUND",
                "发布记录里现在没有修订 3 的包".to_owned()
            ))
        );
    }

    #[test]
    fn pid_verify_action_names_a_wrong_publish_value_invalid() {
        let one = records(vec![package(1, true)]);
        let view = published(read(&one), &on_deliver(), &in_front());
        for value in [
            "publish_dir",
            "publish_check=1",
            "publish=now",
            "package_verify",
            "package_verify=",
            "package_verify=x",
            "package_verify=-1",
            "more=publish",
        ] {
            let refused = parse_action(value, &view).expect_err(value);
            assert_eq!(refused.code(), "invalid_value", "{value}");
            assert!(!refused.to_string().is_empty(), "{value}");
        }
    }

    /// Something running in the background: a project check with 12 of the
    /// 40 drawings it reads back.
    const SCANNING: Option<Busy> = Some(Busy {
        then: Some("check"),
        done: 12,
        total: 40,
    });

    /// What the panel draws with nothing kept while `busy` runs.
    fn running(busy: Option<Busy>) -> VerifyView {
        let context = Context { busy, ..in_front() };
        build_view(&VerifyUi::default(), None, None, &context)
    }

    #[test]
    fn busy_says_what_runs_how_far_it_is_and_what_waits_for_it() {
        let busy = |view: &VerifyView| panel_state(view, ROWS_PER_PAGE, 0)["busy"].clone();
        assert_eq!(
            busy(&running(SCANNING)),
            json!({
                "what": "scan",
                "then": "check",
                "text": "正在读项目图纸",
                "done": 12,
                "total": 40,
                "cancellable": true
            })
        );
        // An automation op reads with nothing waiting for the scan.
        let read = busy(&running(Some(Busy {
            then: None,
            done: 0,
            total: 3,
        })));
        assert_eq!(
            (&read["then"], &read["total"]),
            (&Value::Null, &json!(3)),
            "{read}"
        );
        assert_eq!(busy(&running(None)), Value::Null);
        assert_eq!(
            (BUSY_ID, CANCEL_ID),
            ("pid-verify-busy", "pid-verify-cancel")
        );
    }

    #[test]
    fn pid_verify_action_cancels_only_while_something_runs() {
        let busy = running(SCANNING);
        assert_eq!(parse_action("cancel", &busy), Ok(PidVerifyMsg::Cancel));
        let written = parse_action("cancel=1", &busy).expect_err("cancel=1");
        assert_eq!(written.code(), "invalid_value", "{written}");
        let idle = parse_action("cancel", &running(None)).expect_err("cancel");
        assert_eq!(
            (idle.code(), idle.reason()),
            ("PANEL_ACTION_UNAVAILABLE", Some("not_busy"))
        );
    }

    /// What the panel draws on 「快照与发布」 in `context` with something of
    /// each kind kept on the drawing in front — the five results, a bill,
    /// the three snapshots with `picked` ticked and a diff, a package
    /// published — so that only `context` and `picked` grey its buttons.
    fn everything(context: &Context, picked: &[Pick]) -> VerifyView {
        let (checked, bill, snapshots, diff) = (five(), plant(), three(), compared());
        let one = records(vec![package(1, true)]);
        let drawing = DrawingResults {
            check: Some(KeptCheck {
                report: &checked,
                serial: 1,
                stale: false,
            }),
            bom: Some(kept_bill(&bill, 2)),
            snapshots: Some(&snapshots),
            diff: Some(&diff),
            publish: read(&one),
        };
        let ui = VerifyUi {
            tab: Tab::Deliver,
            picked: picked.to_vec(),
            ..VerifyUi::default()
        };
        build_view(&ui, Some(&drawing), None, context)
    }

    /// The action that presses `button`, and whether [`Buttons`] has it
    /// pressable: 【校验】 is on each package's row, with no flag there.
    fn pressing(button: Button, buttons: Buttons) -> (&'static str, Option<bool>) {
        match button {
            Button::CheckDrawing => ("check=drawing", Some(buttons.check_drawing)),
            Button::CheckProject => ("check=project", Some(buttons.check_project)),
            Button::CheckExport => ("check_export=drc.json", Some(buttons.check_export)),
            Button::BomDrawing => ("bom=drawing", Some(buttons.bom_drawing)),
            Button::BomProject => ("bom=project", Some(buttons.bom_project)),
            Button::BomExport => ("bom_export=bom.csv", Some(buttons.bom_export)),
            Button::SnapshotSave => ("snapshot_save=", Some(buttons.snapshot_save)),
            Button::Diff => ("diff", Some(buttons.diff)),
            Button::DiffExport => ("diff_export=diff.json", Some(buttons.diff_export)),
            Button::PublishCheck => ("publish_check", Some(buttons.publish_check)),
            Button::Publish => ("publish", Some(buttons.publish)),
            Button::PackageVerify => ("package_verify=1", None),
        }
    }

    /// §4.8 row by row: each button grey for each of its conditions, for the
    /// reason the action pressing it is refused with; with none of them every
    /// button can be pressed.
    #[test]
    fn each_button_is_grey_for_each_of_its_conditions_and_says_why() {
        let two = [Pick::Seq(1), Pick::Live];
        let ready = everything(&in_front(), &two);
        let none = everything(&Context::default(), &two);
        let busy = everything(
            &Context {
                busy: SCANNING,
                ..in_front()
            },
            &two,
        );
        let unsaved = everything(
            &Context {
                saved: false,
                ..in_front()
            },
            &two,
        );
        let dirty = everything(
            &Context {
                dirty: true,
                ..in_front()
            },
            &two,
        );
        let read_only = everything(
            &Context {
                writable: false,
                ..in_front()
            },
            &two,
        );
        let one_ticked = everything(&in_front(), &[Pick::Seq(1)]);
        let bare = build_view(&VerifyUi::default(), None, None, &in_front());
        let (failed, scope) = (repeated(), BomScope::drawing("PID-1101"));
        let failure = DrawingResults {
            bom: Some(kept_failure(&failed, &scope)),
            ..DrawingResults::default()
        };
        let unlisted = build_view(&VerifyUi::default(), Some(&failure), None, &in_front());
        for button in Button::ALL {
            let (value, pressable) = pressing(button, ready.buttons);
            assert_eq!(ready.grey(button), None, "{button:?}");
            assert_ne!(pressable, Some(false), "{button:?}");
            assert!(parse_action(value, &ready).is_ok(), "{value}");
        }
        let cases = [
            (&none, Button::CheckDrawing, "no_drawing"),
            (&busy, Button::CheckDrawing, "busy"),
            (&none, Button::BomDrawing, "no_drawing"),
            (&busy, Button::BomDrawing, "busy"),
            (&none, Button::CheckProject, "no_drawing"),
            (&busy, Button::CheckProject, "busy"),
            (&unsaved, Button::CheckProject, "no_project_dir"),
            (&none, Button::BomProject, "no_drawing"),
            (&busy, Button::BomProject, "busy"),
            (&unsaved, Button::BomProject, "no_project_dir"),
            (&bare, Button::CheckExport, "no_result"),
            (&bare, Button::BomExport, "no_result"),
            (&unlisted, Button::BomExport, "no_result"),
            (&bare, Button::DiffExport, "no_result"),
            (&none, Button::SnapshotSave, "no_drawing"),
            (&busy, Button::SnapshotSave, "busy"),
            (&unsaved, Button::SnapshotSave, "no_project_dir"),
            (&one_ticked, Button::Diff, "not_two_picked"),
            (&none, Button::PublishCheck, "no_drawing"),
            (&busy, Button::PublishCheck, "busy"),
            (&unsaved, Button::PublishCheck, "no_project_dir"),
            (&none, Button::Publish, "no_drawing"),
            (&busy, Button::Publish, "busy"),
            (&read_only, Button::Publish, "readonly"),
            (&dirty, Button::Publish, "precondition"),
            (&unsaved, Button::Publish, "precondition"),
            (&none, Button::PackageVerify, "no_drawing"),
            (&busy, Button::PackageVerify, "busy"),
        ];
        for (view, button, reason) in cases {
            let (value, pressable) = pressing(button, view.buttons);
            assert_eq!(view.grey(button), Some(reason), "{button:?}");
            assert_ne!(pressable, Some(true), "{button:?} {reason}");
            let refused = parse_action(value, view).expect_err(value);
            assert_eq!(
                (refused.code(), refused.reason()),
                ("PANEL_ACTION_UNAVAILABLE", Some(reason)),
                "{value}"
            );
        }
        for button in Button::ALL {
            assert!(
                cases.iter().any(|(_, case, _)| *case == button),
                "{button:?} has no case"
            );
        }
        // What runs greys neither exporting nor 【比对】, a session that may
        // not write checks the conditions and verifies all the same, and
        // 【校验】 does not ask for a saved drawing (§4.8).
        for (view, button) in [
            (&busy, Button::CheckExport),
            (&busy, Button::BomExport),
            (&busy, Button::Diff),
            (&busy, Button::DiffExport),
            (&read_only, Button::PublishCheck),
            (&read_only, Button::PackageVerify),
            (&unsaved, Button::PackageVerify),
        ] {
            assert_eq!(view.grey(button), None, "{button:?}");
        }
        // The flags of the panel state say what the reasons say.
        for view in [
            &ready,
            &none,
            &busy,
            &unsaved,
            &dirty,
            &read_only,
            &one_ticked,
            &bare,
            &unlisted,
        ] {
            for button in Button::ALL {
                if let (_, Some(pressable)) = pressing(button, view.buttons) {
                    assert_eq!(pressable, view.grey(button).is_none(), "{button:?}");
                }
            }
        }
    }

    /// §4.8's rows for 【豁免】 and 【取消豁免】: grey in a session that may
    /// not write, on a result not on the drawing in front and — waiving —
    /// on one already waived, for the reason the action is refused with.
    #[test]
    fn the_buttons_on_a_row_are_grey_for_each_of_their_conditions_and_say_why() {
        let project = report(
            ScopeKind::Project,
            vec![
                found("DR-001", Severity::Error, "管线", "L-1001", "PID-1101"),
                waived(found(
                    "DR-008",
                    Severity::Warning,
                    "阀门",
                    "V-1001",
                    "PID-1101",
                )),
                found("DR-005", Severity::Error, "仪表", "FT-2001", "PID-1102"),
                waived(found(
                    "DR-007",
                    Severity::Warning,
                    "管线",
                    "L-2002",
                    "PID-1102",
                )),
            ],
        );
        let kept = DrawingResults {
            check: Some(KeptCheck {
                report: &project,
                serial: 1,
                stale: false,
            }),
            ..DrawingResults::default()
        };
        let ui = VerifyUi {
            show_waived: true,
            ..VerifyUi::default()
        };
        let viewed_in = |writable: bool| {
            let context = Context {
                writable,
                ..in_front()
            };
            build_view(&ui, Some(&kept), None, &context)
        };
        let greys = |view: &VerifyView| {
            let shown = view.check.shown.as_ref().expect("the rows");
            let mut greys: Vec<(usize, Option<&str>, Option<&str>)> = shown
                .rows
                .iter()
                .map(|row| {
                    let writable = view.writable;
                    (
                        row.index,
                        row.grey(writable, false),
                        row.grey(writable, true),
                    )
                })
                .collect();
            greys.sort_unstable();
            greys
        };
        let away = Some("not_on_this_drawing");
        assert_eq!(
            greys(&viewed_in(true)),
            [
                (0, None, Some("not_waived")),
                (1, Some("already_waived"), None),
                (2, away, away),
                (3, away, away),
            ]
        );
        let readonly = Some("readonly");
        assert_eq!(
            greys(&viewed_in(false)),
            [
                (0, readonly, readonly),
                (1, readonly, readonly),
                (2, readonly, readonly),
                (3, readonly, readonly),
            ]
        );
        for view in [viewed_in(true), viewed_in(false)] {
            let shown = view.check.shown.as_ref().expect("the rows");
            for row in &shown.rows {
                let grey = |unwaive: bool| row.grey(view.writable, unwaive);
                assert_eq!(row.can_waive, grey(false).is_none(), "{}", row.index);
                assert_eq!(row.can_unwaive, grey(true).is_none(), "{}", row.index);
                for (value, unwaive) in [
                    (format!("waive={}:放空管", row.index), false),
                    (format!("unwaive={}", row.index), true),
                ] {
                    let refused = parse_action(&value, &view).err();
                    assert_eq!(
                        refused.map(|refused| refused.reason()),
                        grey(unwaive).map(Some),
                        "{value}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_grey_button_says_why_in_its_tip() {
        let reasons = [
            "no_drawing",
            "busy",
            "no_project_dir",
            "no_result",
            "not_two_picked",
            "readonly",
            "precondition",
            "not_on_this_drawing",
            "already_waived",
            "not_waived",
        ];
        let said: BTreeSet<&str> = reasons.into_iter().map(grey_said).collect();
        assert_eq!(said.len(), reasons.len(), "{said:?}");
        assert!(!said.contains(""), "{said:?}");
        let busy = grey_said("busy");
        assert!(busy.starts_with(BUSY_SAID), "{busy}");
    }
}

//! `PIDVERIFY` — shows, hides and turns the 「校验与交付」 panel
//! (`SPEC-pid-verify-deliver-panel.md` §5.2), and the program's side of the
//! panel: its messages, the view it draws and the panel state the
//! `pid_verify_panel` op reads off that view (§3.2).
//!
//! The panel's interface state lives on the program (`pid_verify_ui`) and
//! the panel draws the plain data `build_view` makes of it
//! (`pid_verify_view`), so whatever changes that state ends in
//! [`OpenCADStudio::refresh_pid_verify_view`]. What the view shows besides
//! — the checks kept for the drawing in front and whether a drawing they
//! read changed since, whether a scan runs — changes under other messages
//! too: while the panel is open, every message ends by rebuilding the view
//! when any of that changed ([`OpenCADStudio::sync_pid_verify_view`]). The
//! command never changes the drawing, so a read-only session may run every
//! form of it. The `action` op named `pid_verify` presses the panel with
//! these same messages (§3.3).
//!
//! The rule-check tab's buttons call what `pid_check` calls: the check of
//! the drawing in front, the check of its project after `PIDINDEX`'s scan,
//! and the export `PIDCHECK EXPORT` writes. What stops them is said in the
//! panel's message bar rather than on the command line, so an `action`
//! pressing them is not answered as failed for it (§3.7).
//!
//! A row clicked finds its object the way `PIDLOCATE` does, on whichever
//! drawing it is (`pidindex.rs`), and the waive buttons call what
//! `PIDCHECK WAIVE` and `PIDCHECK UNWAIVE` call; they too say in the
//! message bar what came of it.
//!
//! The material-list tab's buttons call what `pid_bom` calls: the bill of
//! the drawing in front, kept on its tab, and the bill of its project after
//! `PIDINDEX`'s scan, kept for the folder ([`PanelBoms`]). Its export writes
//! the bill shown as `PIDBOM EXPORT` writes it, and a row clicked finds its
//! object as `PIDLOCATE` finds its tag. Only the bills made here are shown:
//! `PIDBOM` and `pid_bom` keep none.
//!
//! The snapshot half of 「快照与发布」 calls what `PIDSNAPSHOT SAVE` and
//! `PIDDIFF` call: 【存快照】 numbers a snapshot in the drawing's folder, and
//! 【比对】 compares the two rows ticked, the older first, the diff kept on the
//! drawing's tab ([`PanelSnapshots`]) until the rows ticked change or another
//! drawing comes in front. The list is read from the folder when the panel
//! is turned or opened to that tab, when another drawing comes in front
//! there, and after each numbered snapshot, wherever it was saved. An object
//! of the diff clicked is found the way a place of a repeated tag is, and
//! the export writes what `PIDDIFF … EXPORT` writes.
//!
//! The publish half calls what `PIDPUBLISH CHECK` and `PIDPUBLISH` call, to
//! the folder typed in its box ([`OpenCADStudio::pidpublish_for_panel`]):
//! 【检查发布条件】 works out the gates whatever the preconditions, which the
//! panel works out itself, and 【发布】, asked in the panel first, publishes
//! as the command does. The gates are kept on the drawing's tab
//! ([`PanelPublish`]) with each open drawing's edit count then, and the
//! check they made becomes the drawing's, so the errors that stopped a
//! publish are on the rule-check tab. What came of it is said in the
//! message bar; the command line keeps the rest of what the command says.
//! The records are read from the target folder when the panel is turned or
//! opened to that tab, when another drawing comes in front there, when the
//! folder typed changes, after each check or publish, and once the
//! drawing's file was written or read since; 【校验】 verifies a package as
//! `PIDPUBLISH VERIFY` does.

use super::pidbom::{unix_now, BomFailure, Sheets};
use super::piddrc::Checked;
use super::pidindex::{same_file, Landed, Target, LOCATE_NOT_FOUND};
use super::pidprops::{PROPS_FIELD_PREFIX, TAG_FIELD};
use super::pidpublish::{why_of, Publications};
use super::pidsnapshot::{snapshot_said, Failure, Side};
use crate::app::{Message, OpenCADStudio};
use crate::io::edit_lock::FileFingerprint;
use crate::pid::bom::{self, Bom, BomError, BomScope};
use crate::pid::drc::{self, CheckResult};
use crate::pid::publish_pkg;
use crate::pid::snapshot::store::SnapshotInfo;
use crate::pid::snapshot::{Diff, CONNECTOR, DEFAULT_MOVE_TOLERANCE};
use crate::ui::dock::{DockMsg, PanelId};
use crate::ui::window::pid_verify_deliver::{
    self, BomFailed, Busy, Context, DiffGroup, DrawingResults, GatesFound, KeptBom, KeptCheck,
    KeptPublish, Notice, Package, Pick, PidVerifyMsg, ProjectResults, Records, Tab, Unreadable,
    Verified, Waiving, CANCELLED_SAID, GONE, INTACT_SAID, REASON_ID, ROWS_PER_PAGE,
};
use iced::Task;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the panel's view was built from besides its interface state: whether
/// it is on screen, the drawing tab in front and where that drawing was
/// saved, each open drawing's edit count, the last check and the last bill
/// the program made, whether the session may write, what runs in the
/// background and how far it is, and whether the drawing in front has
/// changes not saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Inputs {
    visible: bool,
    front: Option<u64>,
    source: Option<String>,
    revisions: Vec<(u64, u64)>,
    last_check: u64,
    last_bom: u64,
    writable: bool,
    busy: Option<Busy>,
    dirty: bool,
}

/// How a result's `fields` name an object's tag (§3.5); its row in the
/// property table is `pid_tag`.
pub(in crate::app) const TAG_NAME: &str = "位号";

/// The properties of one object a clicked rule-check result names (§3.6),
/// by the tab its drawing is open in: the property table highlights their
/// rows while that object is the one selected there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct Highlight {
    pub(in crate::app) document_id: u64,
    pub(in crate::app) handle: document::Handle,
    pub(in crate::app) fields: Vec<String>,
}

/// Whether the property row `row_id` is the row of one of `fields`
/// (§3.6): `pid_props_<property>`, or `pid_tag` for [`TAG_NAME`].
pub(in crate::app) fn highlights_row(fields: &[String], row_id: &str) -> bool {
    match row_id.strip_prefix(PROPS_FIELD_PREFIX) {
        Some(name) => fields.iter().any(|field| field == name),
        None => row_id == TAG_FIELD && fields.iter().any(|field| field == TAG_NAME),
    }
}

/// A bill of materials the panel made (§3.1, §4.5), or why it did not come
/// out, with which bill of the program's it is, what it covers and how many
/// drawings it read, when it was made, and each open drawing's edit count
/// then (tab number, `edit_revision`).
pub(in crate::app) struct PanelBom {
    serial: u64,
    scope: BomScope,
    drawings: usize,
    made_at: i64,
    outcome: Result<Bom, BomFailed>,
    revisions: Vec<(u64, u64)>,
}

impl PanelBom {
    /// The edit count of the drawing on tab `id` when the bill was made; a
    /// drawing opened since counts from 0, as it was opened.
    fn revision_at(&self, id: u64) -> u64 {
        self.revisions
            .iter()
            .find(|(tab, _)| *tab == id)
            .map_or(0, |&(_, revision)| revision)
    }

    /// The bill lent to the view, `stale` or not.
    fn kept(&self, stale: bool) -> KeptBom<'_> {
        KeptBom {
            outcome: self.outcome.as_ref(),
            scope: &self.scope,
            drawings: self.drawings,
            made_at: self.made_at,
            serial: self.serial,
            stale,
        }
    }
}

/// The bills of whole projects the panel made, the latest per project
/// folder; a drawing's own is kept on its tab. The serials of both are
/// given here, so the newer of a drawing's and its folder's can be told.
#[derive(Default)]
pub(in crate::app) struct PanelBoms {
    serial: u64,
    projects: HashMap<PathBuf, PanelBom>,
}

impl PanelBoms {
    /// A bill just made, numbered after the last.
    fn made(
        &mut self,
        scope: BomScope,
        drawings: usize,
        made_at: i64,
        outcome: Result<Bom, BomFailed>,
        revisions: Vec<(u64, u64)>,
    ) -> PanelBom {
        self.serial += 1;
        PanelBom {
            serial: self.serial,
            scope,
            drawings,
            made_at,
            outcome,
            revisions,
        }
    }
}

/// What 「快照与发布」 keeps of a drawing, on its tab (§3.1): its numbered
/// snapshots as last read from its folder, and the last diff the panel made
/// while the two rows it compared stay ticked.
#[derive(Default)]
pub(in crate::app) struct PanelSnapshots {
    listed: Option<Vec<SnapshotInfo>>,
    diff: Option<Diff>,
}

/// What the publish half of 「快照与发布」 keeps of a drawing, on its tab
/// (§3.1, §4.6): its publish records as last read from the target folder,
/// or why there are none, with the tab's fingerprint of its file then and
/// whether the file on disk was that one; the last gates worked out, with
/// each open drawing's edit count then; what 【校验】 found of each revision;
/// and the package folder the last publish here made.
#[derive(Default)]
pub(in crate::app) struct PanelPublish {
    records: Option<Result<Records, String>>,
    read_with: Option<FileFingerprint>,
    file_unchanged: bool,
    gates: Option<(GatesFound, Vec<(u64, u64)>)>,
    verified: Vec<(u32, Verified)>,
    last_package: Option<String>,
}

impl OpenCADStudio {
    /// Rebuild what the panel draws from its interface state and the
    /// program's state around it. A check or a bill made since the last
    /// build starts its list over at its first 200 rows; another drawing in
    /// front is followed first ([`Self::pid_verify_follow_front`]), and so is
    /// the file of the one in front written or read
    /// ([`Self::pid_verify_follow_disk`]).
    pub(in crate::app) fn refresh_pid_verify_view(&mut self) {
        self.pid_verify_follow_front();
        self.pid_verify_follow_disk();
        let inputs = self.pid_verify_inputs();
        let built = self.pid_verify_built_from.as_ref();
        if built.is_some_and(|built| built.last_check != inputs.last_check) {
            self.pid_verify_ui.more = 0;
        }
        if built.is_some_and(|built| built.last_bom != inputs.last_bom) {
            self.pid_verify_ui.bom_more = 0;
        }
        let context = self.pid_verify_context();
        let view = {
            let (drawing, project) = self.pid_verify_kept();
            let (drawing_bom, project_bom) = self.pid_verify_kept_bom();
            let (snapshots, diff) = self.pid_verify_kept_snapshots();
            pid_verify_deliver::build_view(
                &self.pid_verify_ui,
                Some(&DrawingResults {
                    check: drawing,
                    bom: drawing_bom,
                    snapshots,
                    diff,
                    publish: self.pid_verify_kept_publish(),
                }),
                Some(&ProjectResults {
                    check: project,
                    bom: project_bom,
                }),
                &context,
            )
        };
        self.pid_verify_view = view;
        self.pid_verify_built_from = Some(inputs);
    }

    /// After a message, rebuild the view if the panel is open and anything
    /// it was built from changed: a check made or followed, a drawing
    /// edited, saved or read again, another tab in front, a scan started or
    /// done. Another drawing in front is followed whether the panel is open
    /// or not, and so is the highlight a clicked result left in the property
    /// table.
    pub(in crate::app) fn sync_pid_verify_view(&mut self) {
        self.pid_verify_follow_front();
        self.pid_verify_highlight_follow();
        if !self.dock_panel_wanted(PanelId::PidVerifyDeliver) {
            return;
        }
        let reread = self.pid_verify_follow_disk();
        if reread || self.pid_verify_built_from.as_ref() != Some(&self.pid_verify_inputs()) {
            self.refresh_pid_verify_view();
        }
    }

    fn pid_verify_inputs(&self) -> Inputs {
        let front = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start);
        Inputs {
            visible: self.dock_panel_visible(PanelId::PidVerifyDeliver),
            front: front.map(|tab| tab.id),
            source: front.and_then(|tab| tab.scene.document.source_path.clone()),
            revisions: self
                .tabs
                .iter()
                .map(|tab| (tab.id, tab.edit_revision))
                .collect(),
            last_check: self.pid_checks.last_serial(),
            last_bom: self.pid_boms.serial,
            writable: !self.readonly_refuses_command("PIDPUBLISH"),
            busy: self.pid_verify_busy(),
            dirty: front.is_some_and(|tab| tab.dirty),
        }
    }

    /// The program's state the panel shows: whether it is on screen, the
    /// drawing tab in front and that drawing's number as the rule check
    /// gives it ([`drc::drawing_of`]), none on the Start page, whether the
    /// session may change that drawing, whether it was saved, whether a
    /// scan runs, and the waivers that drawing keeps.
    fn pid_verify_context(&self) -> Context {
        let drawing = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start);
        Context {
            visible: self.dock_panel_visible(PanelId::PidVerifyDeliver),
            document_id: drawing.map(|tab| tab.id),
            drawing_id: drawing.map(|tab| {
                let path = tab.scene.document.source_path.as_deref().map(Path::new);
                drc::drawing_of(path).0
            }),
            // The panel has no permissions of its own (§5.4): publishing and
            // waiving pass this gate, which only the session's own drawing
            // closes.
            writable: !self.readonly_refuses_command("PIDPUBLISH"),
            saved: drawing.is_some_and(|tab| crate::io::drawing_dir(&tab.scene.document).is_some()),
            dirty: drawing.is_some_and(|tab| tab.dirty),
            // One scan runs at a time, and the project check starts one.
            busy: self.pid_verify_busy(),
            waivers: drawing
                .map(|tab| {
                    let kept = drc::waivers(&tab.scene.document);
                    kept.into_iter()
                        .map(|waiver| (waiver.rule_id, waiver.object_key))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// What runs in the background, as the panel shows it (§4.7).
    fn pid_verify_busy(&self) -> Option<Busy> {
        let (then, done, total) = self.pid_index.progress()?;
        Some(Busy { then, done, total })
    }

    /// The snapshots and the diff kept with the drawing in front (§3.1);
    /// none on the Start page.
    fn pid_verify_kept_snapshots(&self) -> (Option<&[SnapshotInfo]>, Option<&Diff>) {
        let Some(tab) = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start) else {
            return (None, None);
        };
        let kept = &tab.pid_snapshots;
        (kept.listed.as_deref(), kept.diff.as_ref())
    }

    /// Another drawing in front (§3.1), or the one in front another file now
    /// (saved under another name, replaced in its tab): the snapshot rows
    /// ticked are cleared, and the diff of the drawing left drops with them;
    /// the question 【发布】 asked goes and the repeated tags fold; on
    /// 「快照与发布」, open, the list and the publish records of the drawing
    /// now in front are read.
    fn pid_verify_follow_front(&mut self) {
        let front = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start);
        let followed = match (front, &self.pid_verify_front) {
            (Some(tab), Some((id, source))) => {
                tab.id == *id && tab.scene.document.source_path == *source
            }
            (front, followed) => front.is_none() && followed.is_none(),
        };
        if followed {
            return;
        }
        let front = front.map(|tab| (tab.id, tab.scene.document.source_path.clone()));
        let left = std::mem::replace(&mut self.pid_verify_front, front);
        if let Some(tab) = left.and_then(|(id, _)| self.tabs.iter_mut().find(|tab| tab.id == id)) {
            tab.pid_snapshots.diff = None;
        }
        let ui = &mut self.pid_verify_ui;
        ui.picked.clear();
        ui.diff_more = 0;
        ui.diff_located = None;
        ui.confirming = false;
        ui.duplicates_open = false;
        if ui.tab == Tab::Deliver && self.dock_panel_wanted(PanelId::PidVerifyDeliver) {
            self.pid_verify_reread_snapshots(self.active_tab);
            self.pid_verify_reread_publish(self.active_tab);
        }
    }

    /// Read the snapshot list of the drawing on tab `i` from its folder again
    /// (§3.1). A drawing never saved has none, which goes unsaid; what else
    /// stops it is said in the message bar, and the list read before stays.
    fn pid_verify_reread_snapshots(&mut self, i: usize) {
        if self.tabs.get(i).is_none_or(|tab| tab.is_start) {
            return;
        }
        match self.pidsnapshot_listed(i) {
            Ok((listed, _, _)) => self.tabs[i].pid_snapshots.listed = Some(listed),
            Err(Failure::NoProjectDir) => self.tabs[i].pid_snapshots.listed = None,
            Err(failure) => {
                let code = failure.code().map(str::to_owned);
                self.pid_verify_ui.notice = Some(Notice::error(code, failure.to_string()));
            }
        }
    }

    /// A numbered snapshot of the drawing on tab `i` saved elsewhere than in
    /// the panel (`PIDSNAPSHOT SAVE`, `pid_snapshot_save`, the one a publish
    /// saves first): the list the panel read of that drawing is read again,
    /// and the panel, open, redrawn.
    pub(super) fn pid_verify_snapshots_saved(&mut self, i: usize) {
        if self
            .tabs
            .get(i)
            .is_some_and(|tab| tab.pid_snapshots.listed.is_some())
        {
            self.pid_verify_reread_snapshots(i);
        }
        if self.dock_panel_wanted(PanelId::PidVerifyDeliver) {
            self.refresh_pid_verify_view();
        }
    }

    /// A publish of the drawing on tab `i` made elsewhere than in the panel
    /// (`PIDPUBLISH`), and the package folder it made if it made one: that
    /// folder is the last package published (§3.2), the publish records the
    /// panel read of that drawing are read again, and so is its snapshot list
    /// ([`Self::pid_verify_snapshots_saved`]).
    pub(super) fn pid_verify_published(&mut self, i: usize, package: Option<&Path>) {
        if let (Some(tab), Some(package)) = (self.tabs.get_mut(i), package) {
            tab.pid_publish.last_package = Some(package.display().to_string());
        }
        if self
            .tabs
            .get(i)
            .is_some_and(|tab| tab.pid_publish.records.is_some())
        {
            self.pid_verify_reread_publish(i);
        }
        self.pid_verify_snapshots_saved(i);
    }

    /// What a check or a publish pressed in the panel came to (§4.6), for
    /// the drawing on tab `i`: the gates worked out, kept with each open
    /// drawing's edit count now; the package folder a publish made; what the
    /// message bar says. The publish records and the snapshot list are read
    /// again, and the panel, open, redrawn.
    pub(super) fn pid_verify_publish_done(
        &mut self,
        i: usize,
        found: Option<GatesFound>,
        package: Option<&Path>,
        notice: Option<Notice>,
    ) {
        let revisions = self
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.edit_revision))
            .collect();
        let kept = &mut self.tabs[i].pid_publish;
        if let Some(found) = found {
            kept.gates = Some((found, revisions));
        }
        if let Some(package) = package {
            kept.last_package = Some(package.display().to_string());
        }
        self.pid_verify_ui.notice = notice;
        self.pid_verify_reread_publish(i);
        self.pid_verify_snapshots_saved(i);
    }

    /// What the publish half keeps of the drawing in front (§3.1, §4.6), the
    /// gates stale once a drawing of its folder changed since they were
    /// worked out; nothing on the Start page.
    fn pid_verify_kept_publish(&self) -> KeptPublish<'_> {
        let i = self.active_tab;
        let Some(tab) = self.tabs.get(i).filter(|tab| !tab.is_start) else {
            return KeptPublish::default();
        };
        let kept = &tab.pid_publish;
        let dir = crate::io::drawing_dir(&tab.scene.document);
        KeptPublish {
            records: kept
                .records
                .as_ref()
                .map(|read| read.as_ref().map_err(String::as_str)),
            file_unchanged: kept.file_unchanged,
            gates: kept.gates.as_ref().map(|(found, revisions)| {
                let revision_at = |id: u64| {
                    revisions
                        .iter()
                        .find(|(tab, _)| *tab == id)
                        .map_or(0, |&(_, revision)| revision)
                };
                (found, self.pid_verify_stale(revision_at, i, dir))
            }),
            verified: &kept.verified,
            last_package: kept.last_package.as_deref(),
        }
    }

    /// Read the publish records of the drawing on tab `i` again (§3.1, §4.6)
    /// from the folder typed in the panel, the default one when none is, and
    /// whether the file on disk is the one the program last read or wrote,
    /// with the tab's fingerprint of it now. A drawing never saved has none.
    /// Another target folder than before drops what 【校验】 found.
    fn pid_verify_reread_publish(&mut self, i: usize) {
        if self.tabs.get(i).is_none_or(|tab| tab.is_start) {
            return;
        }
        let dir = Some(self.pid_verify_ui.publish_dir.trim()).filter(|dir| !dir.is_empty());
        let records = self.tabs[i].current_path.is_some().then(|| {
            self.pidpublish_listed_on_disk(i, dir)
                .map(records_of)
                .map_err(|(_, why)| why)
        });
        let file_unchanged = self.tabs[i]
            .current_path
            .as_deref()
            .is_some_and(|path| self.pidpublish_file_unchanged(i, path));
        let tab = &mut self.tabs[i];
        let kept = &mut tab.pid_publish;
        if target_of(kept.records.as_ref()) != target_of(records.as_ref()) {
            kept.verified.clear();
        }
        kept.records = records;
        kept.read_with = tab.disk_fingerprint.clone();
        kept.file_unchanged = file_unchanged;
    }

    /// The file of the drawing in front written or read since its publish
    /// records were read (saved, saved as, read again): on 「快照与发布」,
    /// open, they are read again, so 「当前」 and whether the file was changed
    /// elsewhere follow it. True when they were.
    fn pid_verify_follow_disk(&mut self) -> bool {
        let i = self.active_tab;
        let moved = self
            .tabs
            .get(i)
            .filter(|tab| !tab.is_start)
            .is_some_and(|tab| tab.pid_publish.read_with != tab.disk_fingerprint);
        if !moved
            || self.pid_verify_ui.tab != Tab::Deliver
            || !self.dock_panel_wanted(PanelId::PidVerifyDeliver)
        {
            return false;
        }
        self.pid_verify_reread_publish(i);
        true
    }

    /// The checks kept for the drawing in front (§3.1): its own, on its tab,
    /// and its project folder's; none on the Start page.
    fn pid_verify_kept(&self) -> (Option<KeptCheck<'_>>, Option<KeptCheck<'_>>) {
        let i = self.active_tab;
        let Some(tab) = self.tabs.get(i).filter(|tab| !tab.is_start) else {
            return (None, None);
        };
        let dir = crate::io::drawing_dir(&tab.scene.document);
        let project = dir.and_then(|dir| self.pid_checks.of_project(dir));
        (
            tab.pid_check
                .as_ref()
                .map(|checked| self.pid_verify_kept_check(checked, i, dir)),
            project.map(|checked| self.pid_verify_kept_check(checked, i, dir)),
        )
    }

    fn pid_verify_kept_check<'a>(
        &self,
        checked: &'a Checked,
        i: usize,
        dir: Option<&Path>,
    ) -> KeptCheck<'a> {
        KeptCheck {
            report: &checked.report,
            serial: checked.serial,
            stale: self.pid_check_stale(checked, i, dir),
        }
    }

    /// The bills kept for the drawing in front (§3.1): its own, on its tab,
    /// and its project folder's; none on the Start page.
    fn pid_verify_kept_bom(&self) -> (Option<KeptBom<'_>>, Option<KeptBom<'_>>) {
        let i = self.active_tab;
        let Some(tab) = self.tabs.get(i).filter(|tab| !tab.is_start) else {
            return (None, None);
        };
        let dir = crate::io::drawing_dir(&tab.scene.document);
        let own = tab
            .pid_bom
            .as_ref()
            .map(|made| made.kept(self.pid_verify_stale(|id| made.revision_at(id), i, None)));
        let project = dir
            .and_then(|dir| self.pid_boms.projects.get(&same_file(dir)))
            .map(|made| made.kept(self.pid_verify_stale(|id| made.revision_at(id), i, dir)));
        (own, project)
    }

    /// Whether a drawing `checked` read changed since (§3.1): a check across
    /// drawings read the project folder `dir` of the drawing on tab `i`, any
    /// other only that drawing ([`Self::pid_verify_stale`]). Waiving follows
    /// the result in place and leaves it fresh; undoing that does not (§4.4).
    fn pid_check_stale(&self, checked: &Checked, i: usize, dir: Option<&Path>) -> bool {
        let across = dir.filter(|_| checked.report.scope.cross_drawing);
        self.pid_verify_stale(|id| checked.revision_at_check(id), i, across)
    }

    /// Whether a drawing a result read changed since it was made (§3.1),
    /// `revision_at` giving each tab's edit count then. A result of the
    /// drawing on tab `i` alone goes stale when that drawing changes; one
    /// that read its project folder `across`, when any drawing open in that
    /// folder does, one opened since counting from its opening.
    fn pid_verify_stale(
        &self,
        revision_at: impl Fn(u64) -> u64,
        i: usize,
        across: Option<&Path>,
    ) -> bool {
        let changed = |id: u64, revision: u64| revision_at(id) != revision;
        let Some(dir) = across else {
            return changed(self.tabs[i].id, self.tabs[i].edit_revision);
        };
        let dir = same_file(dir);
        self.tabs
            .iter()
            .filter(|tab| !tab.is_start)
            .filter(|tab| {
                crate::io::drawing_dir(&tab.scene.document)
                    .is_some_and(|other| same_file(other) == dir)
            })
            .any(|tab| changed(tab.id, tab.edit_revision))
    }

    /// Show the panel the way its switch among the dock's panels does:
    /// docked on the right the first time, where the user left it after. On
    /// 「快照与发布」 the snapshot list and the publish records are read again.
    pub(in crate::app) fn show_pid_verify_panel(&mut self) -> Task<Message> {
        let task = self.dock_set_open(PanelId::PidVerifyDeliver, true);
        if self.pid_verify_ui.tab == Tab::Deliver {
            self.pid_verify_reread_snapshots(self.active_tab);
            self.pid_verify_reread_publish(self.active_tab);
        }
        self.refresh_pid_verify_view();
        task
    }

    /// `pid_verify_panel` (§3.2): what the panel shows, shown or hidden. The
    /// view is rebuilt first, so it answers for the drawing in front now;
    /// `limit` (200 when omitted, [`ROWS_PER_PAGE`]) and `offset` page each
    /// list.
    pub(in crate::app) fn control_pid_verify_panel(&mut self, req: &Value) -> Value {
        let (limit, offset) = match paging(req) {
            Ok(paging) => paging,
            Err(refused) => return refused,
        };
        self.refresh_pid_verify_view();
        pid_verify_deliver::panel_state(&self.pid_verify_view, limit, offset)
    }

    /// `PIDVERIFY` (§5.2): bare or `ON` shows the panel, `OFF` hides it, and
    /// `CHECK`, `BOM` or `DELIVER` shows it on that tab. Each run says on the
    /// command line what it did.
    pub(super) fn dispatch_pidverify(&mut self, cmd: &str, _i: usize) -> Option<Task<Message>> {
        let line = cmd.trim();
        let (verb, arg) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if !verb.eq_ignore_ascii_case("PIDVERIFY") {
            return None;
        }
        let arg = arg.trim();
        if arg.eq_ignore_ascii_case("OFF") {
            let task = self.on_dock(DockMsg::Close(PanelId::PidVerifyDeliver));
            self.command_line
                .push_info("PIDVERIFY: 已隐藏「校验与交付」面板。");
            return Some(task);
        }
        if !arg.is_empty() && !arg.eq_ignore_ascii_case("ON") {
            let Some(tab) = Tab::from_key(arg) else {
                self.command_line.push_error(&format!(
                    "PIDVERIFY: 只接受 ON、OFF、CHECK、BOM、DELIVER 中的一个，实得 \"{arg}\"。"
                ));
                return Some(Task::none());
            };
            self.pid_verify_ui.tab = tab;
        }
        let task = self.show_pid_verify_panel();
        let label = self.pid_verify_ui.tab.label();
        self.command_line.push_info(&format!(
            "PIDVERIFY: 已显示「校验与交付」面板，当前页签「{label}」。"
        ));
        Some(task)
    }

    /// What the user did in the panel. Hiding takes the close button's path,
    /// so the controls' positions go with the panel. A filter changed or a
    /// check pressed starts the rule-check list over at its first 200 rows,
    /// and a search typed or a bill pressed the material list. Turning to
    /// 「快照与发布」 reads its snapshot list and publish records again.
    pub(in crate::app) fn on_pid_verify(&mut self, msg: PidVerifyMsg) -> Task<Message> {
        let ui = &mut self.pid_verify_ui;
        let task = match msg {
            PidVerifyMsg::Show => return self.show_pid_verify_panel(),
            PidVerifyMsg::Hide => {
                let task = self.on_dock(DockMsg::Close(PanelId::PidVerifyDeliver));
                self.refresh_pid_verify_view();
                return task;
            }
            PidVerifyMsg::PickExport => return self.pid_verify_pick_export(),
            PidVerifyMsg::PickBomExport => return self.pid_verify_pick_bom_export(),
            PidVerifyMsg::PickDiffExport => return self.pid_verify_pick_diff_export(),
            PidVerifyMsg::PickPublishDir => return self.pid_verify_pick_publish_dir(),
            PidVerifyMsg::Tab(tab) => {
                ui.tab = tab;
                if tab == Tab::Deliver {
                    self.pid_verify_reread_snapshots(self.active_tab);
                    self.pid_verify_reread_publish(self.active_tab);
                }
                Task::none()
            }
            PidVerifyMsg::CheckDrawing => {
                ui.more = 0;
                self.pid_verify_check_drawing();
                Task::none()
            }
            PidVerifyMsg::CheckProject => {
                ui.more = 0;
                self.pid_verify_check_project()
            }
            PidVerifyMsg::Severity(severity) => {
                toggle(&mut ui.severities, severity);
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::Kind(kind) => {
                match kind {
                    Some(kind) => toggle(&mut ui.kinds, kind),
                    None => ui.kinds.clear(),
                }
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::ShowWaived(on) => {
                ui.show_waived = on;
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::Fold(group) => {
                toggle(&mut ui.folded, group);
                Task::none()
            }
            PidVerifyMsg::MoreChecks => {
                ui.more = ui.more.saturating_add(1);
                Task::none()
            }
            PidVerifyMsg::Export(name) => {
                self.pid_verify_export(&name);
                Task::none()
            }
            PidVerifyMsg::Locate(index) => self.pid_verify_locate(index),
            PidVerifyMsg::AskWaiver(index) => {
                let shown = self.pid_verify_view.check.shown.as_ref();
                ui.waiving = shown.map(|shown| Waiving {
                    serial: shown.serial,
                    index,
                    reason: String::new(),
                });
                iced::widget::operation::focus(iced::widget::Id::new(REASON_ID))
            }
            PidVerifyMsg::WaiverReason(typed) => {
                if let Some(waiving) = ui.waiving.as_mut() {
                    waiving.reason = typed;
                }
                Task::none()
            }
            PidVerifyMsg::Waive(index, reason) => {
                self.pid_verify_waive(index, reason);
                Task::none()
            }
            PidVerifyMsg::CancelWaiver => {
                ui.waiving = None;
                Task::none()
            }
            PidVerifyMsg::Unwaive(index) => {
                self.pid_verify_unwaive(index);
                Task::none()
            }
            PidVerifyMsg::StaleWaivers => {
                ui.stale_open = !ui.stale_open;
                Task::none()
            }
            PidVerifyMsg::DropWaiver {
                rule_id,
                object_key,
            } => {
                self.pid_verify_drop_waiver(&rule_id, &object_key);
                Task::none()
            }
            PidVerifyMsg::BomDrawing => {
                ui.bom_more = 0;
                self.pid_verify_bom_drawing();
                Task::none()
            }
            PidVerifyMsg::BomProject => {
                ui.bom_more = 0;
                self.pid_verify_bom_project()
            }
            PidVerifyMsg::BomSearch(typed) => {
                ui.bom_search = typed;
                ui.bom_more = 0;
                Task::none()
            }
            PidVerifyMsg::BomFold(kind) => {
                toggle(&mut ui.bom_folded, kind);
                Task::none()
            }
            PidVerifyMsg::MoreBom => {
                ui.bom_more = ui.bom_more.saturating_add(1);
                Task::none()
            }
            PidVerifyMsg::BomExport(name) => {
                self.pid_verify_bom_export(&name);
                Task::none()
            }
            PidVerifyMsg::BomLocate(index) => self.pid_verify_bom_locate(index),
            PidVerifyMsg::BomSkipped => {
                ui.skipped_open = !ui.skipped_open;
                Task::none()
            }
            PidVerifyMsg::BomSkippedLocate(at) => self.pid_verify_bom_skipped_locate(at),
            PidVerifyMsg::BomDuplicateLocate(repeated, at) => {
                self.pid_verify_bom_duplicate_locate(repeated, at)
            }
            PidVerifyMsg::BomSummary => {
                ui.summary_open = !ui.summary_open;
                Task::none()
            }
            PidVerifyMsg::SnapshotReason(typed) => {
                ui.snapshot_reason = typed;
                Task::none()
            }
            PidVerifyMsg::SnapshotSave(reason) => {
                self.pid_verify_snapshot_save(&reason);
                Task::none()
            }
            PidVerifyMsg::Pick(pick) => {
                self.pid_verify_pick_row(pick);
                Task::none()
            }
            PidVerifyMsg::Diff(a, b) => {
                self.pid_verify_diff(a, b);
                Task::none()
            }
            PidVerifyMsg::DiffLocate(group, index) => self.pid_verify_diff_locate(group, index),
            PidVerifyMsg::MoreDiff => {
                ui.diff_more = ui.diff_more.saturating_add(1);
                Task::none()
            }
            PidVerifyMsg::DiffExport(name) => {
                self.pid_verify_diff_export(&name);
                Task::none()
            }
            PidVerifyMsg::PublishDir(typed) => {
                ui.publish_dir = typed;
                ui.confirming = false;
                self.pid_verify_reread_publish(self.active_tab);
                Task::none()
            }
            PidVerifyMsg::PublishCheck => {
                ui.confirming = false;
                self.pid_verify_publish(true)
            }
            PidVerifyMsg::AskPublish => {
                let listed = self
                    .pid_verify_view
                    .deliver
                    .publish
                    .as_ref()
                    .is_some_and(|publish| publish.target_dir.is_some());
                if listed {
                    ui.confirming = true;
                    Task::none()
                } else {
                    self.pid_verify_publish(false)
                }
            }
            PidVerifyMsg::Publish => {
                ui.confirming = false;
                self.pid_verify_publish(false)
            }
            PidVerifyMsg::CancelPublish => {
                ui.confirming = false;
                Task::none()
            }
            PidVerifyMsg::PublishErrors => {
                ui.tab = Tab::Check;
                ui.severities = vec![drc::Severity::Error];
                ui.kinds.clear();
                ui.show_waived = false;
                ui.more = 0;
                Task::none()
            }
            PidVerifyMsg::PublishDuplicates => {
                ui.duplicates_open = !ui.duplicates_open;
                Task::none()
            }
            PidVerifyMsg::PublishDuplicateLocate(repeated, at) => {
                self.pid_verify_publish_duplicate_locate(repeated, at)
            }
            PidVerifyMsg::PackageVerify(revision) => {
                self.pid_verify_package_verify(revision);
                Task::none()
            }
            PidVerifyMsg::Cancel => {
                self.pid_verify_cancel();
                Task::none()
            }
        };
        self.refresh_pid_verify_view();
        task
    }

    /// 【检查当前图】 (§4.2): what `pid_check` does with `scope: drawing`, the
    /// result kept on the drawing's tab. A rule pack that does not read is
    /// said in the message bar, under its code, and the result there was
    /// stays (§4.1).
    fn pid_verify_check_drawing(&mut self) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        match self.pidcheck_load_rules(i) {
            Ok(rules) => {
                let checked = self.pidcheck_this_drawing(i, &rules);
                self.tabs[i].pid_check = Some(checked);
                self.pid_verify_ui.notice = None;
            }
            Err((code, why)) => {
                self.pid_verify_ui.notice = Some(Notice::error(Some(code.to_owned()), why));
            }
        }
    }

    /// 【检查全项目】 (§4.2): `PIDCHECK PROJECT` — `PIDINDEX`'s scan of the
    /// drawing's folder, then the check, kept for the folder. What stops it
    /// before the scan is said in the message bar; the scan and the check
    /// say on the command line what they say for the command. Closed while
    /// its folder is read, the drawing gets no check (§4.7).
    fn pid_verify_check_project(&mut self) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Task::none();
        }
        match self.pidcheck_prepare(i, None) {
            Ok(mut check) => {
                self.pid_verify_ui.notice = None;
                check.panel = Some(self.tabs[i].id);
                self.pidindex_check(i, check)
            }
            Err((code, why)) => {
                self.pid_verify_ui.notice = Some(Notice::error(Some(code.to_owned()), why));
                Task::none()
            }
        }
    }

    /// 【取消】 on the progress line (§4.7): `PIDINDEX CANCEL`. The scan
    /// stops and what it was read for is not done; the results there were
    /// stay, and the message bar says it was cancelled.
    fn pid_verify_cancel(&mut self) {
        if self.pid_index.scanning() {
            self.pidindex_cancel();
            self.pid_verify_ui.notice = Some(Notice::info(CANCELLED_SAID));
        }
    }

    /// 【导出…】 on the rule-check tab: ask where to write the result shown,
    /// offering `<drawing>-drc.json` beside the drawing.
    fn pid_verify_pick_export(&self) -> Task<Message> {
        self.pid_verify_pick(
            "导出检查结果",
            "drc.json",
            &[("JSON", "json")],
            PidVerifyMsg::Export,
        )
    }

    /// 【导出…】 under the material list (§4.5): ask where to write the bill
    /// shown, offering `<drawing>-bom.csv` beside the drawing, and CSV, XLSX
    /// or JSON.
    fn pid_verify_pick_bom_export(&self) -> Task<Message> {
        self.pid_verify_pick(
            "导出材料清单",
            "bom.csv",
            &[("CSV", "csv"), ("XLSX", "xlsx"), ("JSON", "json")],
            PidVerifyMsg::BomExport,
        )
    }

    /// Ask where to write what the panel shows: the system's save dialog
    /// titled `title`, in the drawing's folder, offering `<drawing>-<name>`
    /// and the `kinds` of file. The file picked is pressed as `write`;
    /// nothing is when none is.
    fn pid_verify_pick(
        &self,
        title: &'static str,
        name: &str,
        kinds: &'static [(&'static str, &'static str)],
        write: fn(String) -> PidVerifyMsg,
    ) -> Task<Message> {
        let dir = self
            .tabs
            .get(self.active_tab)
            .filter(|tab| !tab.is_start)
            .and_then(|tab| crate::io::drawing_dir(&tab.scene.document))
            .map(Path::to_path_buf);
        let drawing = self
            .pid_verify_view
            .drawing_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or("PID");
        let name = format!("{drawing}-{name}");
        Task::perform(
            async move {
                let mut dialog = crate::sys::file_dialog()
                    .set_title(title)
                    .set_file_name(&name);
                for &(kind, extension) in kinds {
                    dialog = dialog.add_filter(kind, &[extension]);
                }
                if let Some(dir) = dir {
                    dialog = dialog.set_directory(dir);
                }
                dialog
                    .save_file()
                    .await
                    .map(|picked| crate::sys::handle_path(&picked))
            },
            move |picked| match picked {
                Some(path) => write(path.to_string_lossy().into_owned()).message(),
                None => Message::Noop,
            },
        )
    }

    /// Write the result shown to `name` as `PIDCHECK EXPORT` writes it, and
    /// say in the message bar where it went or why it did not.
    fn pid_verify_export(&mut self, name: &str) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        self.pid_verify_ui.notice = Some(match self.pidcheck_write_export(i, name) {
            Ok(said) => Notice::info(said),
            Err(why) => Notice::error(None, why),
        });
    }

    /// The result at `index` of the check the panel shows for the drawing in
    /// front — the newer of its own and its folder's (§3.1) — and that
    /// check's serial.
    fn pid_verify_result(&self, index: usize) -> Option<(u64, CheckResult)> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return None;
        }
        let checked = self.pidcheck_latest(i)?;
        let result = checked.report.results.get(index)?;
        Some((checked.serial, result.clone()))
    }

    /// A row clicked (§4.3): its object found on the drawing it is on — the
    /// one in front, the tab open on it, or opened — the way `PIDLOCATE`
    /// finds it, without a word on the command line. What came of it is
    /// said in the message bar ([`Self::pid_verify_landed`]).
    fn pid_verify_locate(&mut self, index: usize) -> Task<Message> {
        let Some((serial, result)) = self.pid_verify_result(index) else {
            return Task::none();
        };
        self.pid_verify_ui.located = Some((serial, index));
        let here = self.pidcheck_drawing_id(self.active_tab);
        self.pid_verify_go(Target::of_result(&result), &here)
    }

    /// Find `target` (`pidindex.rs`): on the drawing in front when that is
    /// the one `here` names, else on the drawing of its folder the target
    /// names, switched to or opened.
    fn pid_verify_go(&mut self, target: Target, here: &str) -> Task<Message> {
        let i = self.active_tab;
        if target.drawing_id() == here {
            let landed = self.pidlocate_target(i, &target);
            self.pid_verify_landed(landed);
            return Task::none();
        }
        let Some(dir) = crate::io::drawing_dir(&self.tabs[i].scene.document) else {
            return Task::none();
        };
        let dir = dir.to_path_buf();
        self.pidlocate_elsewhere(&dir, target)
    }

    /// What came of finding a row's object, in the message bar: nothing when
    /// it was found or has no tag of its own to find it by, [`GONE`] when
    /// its tag is no longer on its drawing.
    pub(super) fn pid_verify_landed(&mut self, landed: Landed) {
        self.pid_verify_ui.notice = match landed {
            Landed::Gone => Some(Notice::error(Some(LOCATE_NOT_FOUND.to_owned()), GONE)),
            Landed::Found | Landed::Spot => None,
        };
        self.refresh_pid_verify_view();
    }

    /// Why a row's object was not found — its drawing gone from the folder,
    /// or not opened — in the message bar.
    pub(super) fn pid_verify_not_located(&mut self, code: Option<&str>, why: String) {
        self.pid_verify_ui.notice = Some(Notice::error(code.map(str::to_owned), why));
        self.refresh_pid_verify_view();
    }

    /// A row's object found on tab `i` as `found` (§3.6): one object, of
    /// which the row names properties, and those are the rows the property
    /// table highlights, in place of any it highlighted before. Other finds
    /// leave the highlight as it was; the selection they change drops it
    /// ([`Self::pid_verify_highlight_follow`]).
    pub(in crate::app) fn pid_verify_highlight_found(
        &mut self,
        i: usize,
        found: &[document::Handle],
        fields: &[String],
    ) {
        if let ([handle], false) = (found, fields.is_empty()) {
            self.pid_verify_highlight = Some(Highlight {
                document_id: self.tabs[i].id,
                handle: *handle,
                fields: fields.to_vec(),
            });
        }
    }

    /// The highlight dropped once what it is for is gone (§3.6): its drawing
    /// closed, or its object no longer the one selected there.
    pub(in crate::app) fn pid_verify_highlight_follow(&mut self) {
        let Some(highlight) = &self.pid_verify_highlight else {
            return;
        };
        let kept = self
            .tabs
            .iter()
            .position(|tab| tab.id == highlight.document_id)
            .is_some_and(|i| !self.pid_verify_highlighted(i, highlight.handle).is_empty());
        if !kept {
            self.pid_verify_highlight = None;
        }
    }

    /// The properties of `handle` the property table of tab `i` highlights
    /// (§3.6): the highlight's, while `handle` is its object and the only one
    /// selected there; none otherwise.
    pub(in crate::app) fn pid_verify_highlighted(
        &self,
        i: usize,
        handle: document::Handle,
    ) -> &[String] {
        let tab = &self.tabs[i];
        let selected = &tab.scene.selected;
        match &self.pid_verify_highlight {
            Some(highlight)
                if highlight.document_id == tab.id
                    && highlight.handle == handle
                    && selected.len() == 1
                    && selected.contains(&handle) =>
            {
                &highlight.fields
            }
            _ => &[],
        }
    }

    /// Whether the row `row_id` of the property table of tab `i` is
    /// highlighted (§3.6, [`highlights_row`]).
    pub(in crate::app) fn pid_verify_row_highlighted(&self, i: usize, row_id: &str) -> bool {
        self.pid_verify_highlight.as_ref().is_some_and(|highlight| {
            highlights_row(self.pid_verify_highlighted(i, highlight.handle), row_id)
        })
    }

    /// The property `name` of `handles` on tab `i` filled with `value`
    /// (§3.6): its row is no longer highlighted, the others stay, and with
    /// none left the highlight goes. A blank value fills nothing.
    pub(in crate::app) fn pid_verify_highlight_filled(
        &mut self,
        i: usize,
        handles: &[document::Handle],
        name: &str,
        value: &str,
    ) {
        if value.trim().is_empty() {
            return;
        }
        let id = self.tabs[i].id;
        let Some(highlight) = &mut self.pid_verify_highlight else {
            return;
        };
        if highlight.document_id != id || !handles.contains(&highlight.handle) {
            return;
        }
        highlight.fields.retain(|field| field != name);
        if highlight.fields.is_empty() {
            self.pid_verify_highlight = None;
        }
    }

    /// 【确认】 under a row's reason box (§4.4): the row waived for `reason`
    /// by the entry `PIDCHECK WAIVE` calls, which follows the result in
    /// place without checking again. What the entry refuses is said in the
    /// message bar and the box stays open with what was typed; waived, the
    /// box closes.
    fn pid_verify_waive(&mut self, index: usize, reason: String) {
        let Some((serial, result)) = self.pid_verify_result(index) else {
            return;
        };
        let i = self.active_tab;
        let added = self.pidcheck_load_rules(i).and_then(|rules| {
            self.pidcheck_add_waiver(i, &rules, &result.rule_id, &result.object_key, &reason)
        });
        let ui = &mut self.pid_verify_ui;
        match added {
            Ok(_) => {
                ui.waiving = None;
                ui.notice = None;
            }
            Err((code, said)) => {
                ui.waiving = Some(Waiving {
                    serial,
                    index,
                    reason,
                });
                ui.notice = Some(Notice::error(Some(code.to_owned()), said));
            }
        }
    }

    /// 【取消豁免】 (§4.4): the row's waiver taken off the drawing in front.
    fn pid_verify_unwaive(&mut self, index: usize) {
        if let Some((_, result)) = self.pid_verify_result(index) {
            self.pid_verify_drop_waiver(&result.rule_id, &result.object_key);
        }
    }

    /// The waiver of `rule_id` on `key` taken off the drawing in front by
    /// the entry `PIDCHECK UNWAIVE` calls — one undo step, the results kept
    /// followed in place — for 【取消豁免】 and for 【删除】 beside a waiver no
    /// result matches. What the entry refuses is said in the message bar.
    fn pid_verify_drop_waiver(&mut self, rule_id: &str, key: &str) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        let dropped = self.pidcheck_remove_waiver(i, rule_id, key);
        self.pid_verify_ui.notice = dropped
            .err()
            .map(|(code, said)| Notice::error(Some(code.to_owned()), said));
    }

    /// 【当前图】 (§4.5): what `pid_bom` does with `scope: drawing`, the bill
    /// kept on the drawing's tab. Tags found in more than one place are kept
    /// in its place to be listed; what else stops it is said in the message
    /// bar, under its code, and the bill there was stays.
    fn pid_verify_bom_drawing(&mut self) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        let revisions = vec![(self.tabs[i].id, self.tabs[i].edit_revision)];
        let made = match self.pidbom_compute(Sheets::Current(i)) {
            Ok((bill, drawings)) => {
                let (scope, made_at) = (bill.scope.clone(), bill.generated_at);
                self.pid_boms
                    .made(scope, drawings, made_at, Ok(bill), revisions)
            }
            Err(failure) => {
                let Some(failed) = bom_failed(&failure) else {
                    let code = Some(failure.code().to_owned());
                    self.pid_verify_ui.notice = Some(Notice::error(code, failure.to_string()));
                    return;
                };
                let scope = BomScope::drawing(&self.pid_verify_bom_drawing_id(i));
                self.pid_boms
                    .made(scope, 1, unix_now(), Err(failed), revisions)
            }
        };
        self.tabs[i].pid_bom = Some(made);
        self.pid_verify_ui.notice = None;
    }

    /// 【全项目】 (§4.5): `PIDBOM PROJECT` — `PIDINDEX`'s scan of the
    /// drawing's folder, then the bill, kept for the folder
    /// ([`Self::pid_verify_bom_scanned`]). The scan and the bill say on the
    /// command line what they say for the command.
    fn pid_verify_bom_project(&mut self) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Task::none();
        }
        self.pid_verify_ui.notice = None;
        self.pidbom_for_panel(i).unwrap_or_else(Task::none)
    }

    /// The bill 【全项目】 asked for, made on the scan of `project_dir`
    /// (`pidbom.rs`): kept for that folder with each open drawing's edit
    /// count. Tags found in more than one place and drawings that did not
    /// read are kept in its place to be listed; what else stops it is said
    /// in the message bar, and the bill there was stays.
    pub(super) fn pid_verify_bom_scanned(
        &mut self,
        project_dir: &Path,
        outcome: Result<(Bom, usize), BomFailure>,
    ) {
        let revisions = self
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.edit_revision))
            .collect();
        let made = match outcome {
            Ok((bill, drawings)) => {
                let (scope, made_at) = (bill.scope.clone(), bill.generated_at);
                self.pid_boms
                    .made(scope, drawings, made_at, Ok(bill), revisions)
            }
            Err(failure) => {
                let Some(failed) = bom_failed(&failure) else {
                    let code = Some(failure.code().to_owned());
                    self.pid_verify_ui.notice = Some(Notice::error(code, failure.to_string()));
                    self.refresh_pid_verify_view();
                    return;
                };
                self.pid_boms
                    .made(BomScope::project(), 0, unix_now(), Err(failed), revisions)
            }
        };
        self.pid_boms.projects.insert(same_file(project_dir), made);
        self.pid_verify_ui.notice = None;
        self.refresh_pid_verify_view();
    }

    /// The bill the panel shows for the drawing in front — the newer of its
    /// own and its folder's (§3.1) — or why it did not come out.
    fn pid_verify_made(&self) -> Option<&PanelBom> {
        let tab = self.tabs.get(self.active_tab).filter(|tab| !tab.is_start)?;
        let folder = crate::io::drawing_dir(&tab.scene.document)
            .and_then(|dir| self.pid_boms.projects.get(&same_file(dir)));
        match (tab.pid_bom.as_ref(), folder) {
            (Some(own), Some(folder)) if folder.serial > own.serial => Some(folder),
            (Some(own), _) => Some(own),
            (None, folder) => folder,
        }
    }

    /// The drawing on tab `i` as a bill names it: the project file's number,
    /// else the file name ([`Self::drawing_meta`]); as the rule check names
    /// it when the project file does not read.
    fn pid_verify_bom_drawing_id(&self, i: usize) -> String {
        self.drawing_meta(i)
            .map_or_else(|_| self.pidcheck_drawing_id(i), |meta| meta.drawing_id)
    }

    /// Write the bill shown to `name` as `PIDBOM EXPORT` writes a bill — all
    /// of it, whatever is searched or folded — and say in the message bar
    /// where it went or why it did not.
    fn pid_verify_bom_export(&mut self, name: &str) {
        let i = self.active_tab;
        let Some(bill) = self
            .pid_verify_made()
            .and_then(|made| made.outcome.as_ref().ok())
        else {
            return;
        };
        self.pid_verify_ui.notice = Some(match self.pidbom_write(i, bill, name) {
            Ok(said) => Notice::info(said),
            Err((code, why)) => Notice::error(code.map(str::to_owned), why),
        });
    }

    /// A row of the material list clicked (§4.5): its object found as
    /// `PIDLOCATE <tag>` finds it, on the drawing in front when the row is on
    /// it, else on the first drawing the row names. The fittings of a run
    /// make one row under the run's tag, which finds the run.
    fn pid_verify_bom_locate(&mut self, index: usize) -> Task<Message> {
        let Some(made) = self.pid_verify_made() else {
            return Task::none();
        };
        let serial = made.serial;
        let Some(row) = made
            .outcome
            .as_ref()
            .ok()
            .and_then(|bill| bill.rows.get(index))
        else {
            return Task::none();
        };
        let here = self.pid_verify_bom_drawing_id(self.active_tab);
        let drawing_id = match row.drawings.first() {
            Some(first) if !row.drawings.contains(&here) => first.as_str(),
            _ => here.as_str(),
        };
        let lumped = [bom::FITTING_ROW, bom::PIPE_LINE_ROW].contains(&row.type_code.as_str());
        let type_code = (!lumped).then_some(row.type_code.as_str());
        let target = Target::tagged(drawing_id, &row.tag, type_code);
        self.pid_verify_ui.bom_located = Some((serial, index));
        self.pid_verify_go(target, &here)
    }

    /// One of the objects without a tag clicked (§4.5): with nothing to
    /// find it by, the view goes to where it is, on its drawing.
    fn pid_verify_bom_skipped_locate(&mut self, at: usize) -> Task<Message> {
        let bill = self
            .pid_verify_made()
            .and_then(|made| made.outcome.as_ref().ok());
        let Some(skipped) = bill.and_then(|bill| bill.skipped.get(at)) else {
            return Task::none();
        };
        let target = Target::spot(&skipped.drawing_id, skipped.at);
        let here = self.pid_verify_bom_drawing_id(self.active_tab);
        self.pid_verify_go(target, &here)
    }

    /// One place of a tag found more than once clicked (§4.5): the object of
    /// that type and tag nearest the place, on its drawing.
    fn pid_verify_bom_duplicate_locate(&mut self, repeated: usize, at: usize) -> Task<Message> {
        let failed = self
            .pid_verify_made()
            .and_then(|made| made.outcome.as_ref().err());
        let Some(duplicate) = failed.and_then(|failed| failed.duplicates.get(repeated)) else {
            return Task::none();
        };
        let Some(place) = duplicate.places.get(at) else {
            return Task::none();
        };
        let target = Target::placed(
            &place.drawing_id,
            &duplicate.type_code,
            &duplicate.tag,
            place.at,
        );
        let here = self.pid_verify_bom_drawing_id(self.active_tab);
        self.pid_verify_go(target, &here)
    }

    /// 【存快照】 (§4.6): a numbered snapshot of the drawing in front for
    /// `reason`, by the entry `PIDSNAPSHOT SAVE` calls. Saved, the message
    /// bar says which it is and the reason box empties; what stops it is said
    /// there, and what was typed stays. The list is read again either way.
    fn pid_verify_snapshot_save(&mut self, reason: &str) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        let notice = match self.pidsnapshot_store(i, None, reason, snapshot_said) {
            Ok((saved, said)) => {
                self.pid_verify_ui.snapshot_reason.clear();
                let seq = saved.seq.unwrap_or_default();
                Notice::info(format!("已存第 {seq} 份快照（{said}）。"))
            }
            Err(failure) => Notice::error(failure.code().map(str::to_owned), failure.to_string()),
        };
        self.pid_verify_ui.notice = Some(notice);
        self.pid_verify_reread_snapshots(i);
    }

    /// A row of the snapshot list ticked or unticked (§4.6): the rows the
    /// panel shows ticked, with this one switched. The diff shown compared
    /// the rows ticked before, so it goes.
    fn pid_verify_pick_row(&mut self, pick: Pick) {
        let mut picked = self.pid_verify_view.deliver.picked.clone();
        toggle(&mut picked, pick);
        let ui = &mut self.pid_verify_ui;
        ui.picked = picked;
        ui.diff_more = 0;
        ui.diff_located = None;
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.pid_snapshots.diff = None;
        }
    }

    /// 【比对】 (§4.6): the two rows ticked compared by the entry `PIDDIFF`
    /// calls, `a` the older, at the tolerance `PIDDIFF` takes by default. The
    /// diff is kept on the drawing's tab and its list starts over; what stops
    /// it is said in the message bar, and no diff is shown then.
    fn pid_verify_diff(&mut self, a: Pick, b: Pick) {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return;
        }
        let side = |pick: Pick| match pick {
            Pick::Live => Side::Live,
            Pick::Seq(seq) => Side::Seq(seq),
        };
        let compared = self.piddiff_compare(i, &side(a), &side(b), DEFAULT_MOVE_TOLERANCE);
        let ui = &mut self.pid_verify_ui;
        ui.diff_more = 0;
        ui.diff_located = None;
        ui.notice = compared
            .as_ref()
            .err()
            .map(|failure| Notice::error(failure.code().map(str::to_owned), failure.to_string()));
        self.tabs[i].pid_snapshots.diff = compared.ok();
    }

    /// An object of the diff clicked (§4.6): on the drawing in front, the
    /// object of its type and tag nearest to where the diff puts it, found as
    /// `PIDLOCATE` finds a tag and framed. A removed object, and a page
    /// connector, have nothing to find them by there: the view goes to where
    /// they were, or are.
    fn pid_verify_diff_locate(&mut self, group: DiffGroup, index: usize) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Task::none();
        }
        let here = self.pid_verify_bom_drawing_id(i);
        let diff = self.tabs[i].pid_snapshots.diff.as_ref();
        let Some(object) = diff.and_then(|diff| group.objects(diff).get(index)) else {
            return Task::none();
        };
        let target = if group == DiffGroup::Removed || object.kind == CONNECTOR {
            Target::spot(&here, object.at)
        } else {
            Target::placed(&here, &object.type_code, &object.tag, object.at)
        };
        self.pid_verify_ui.diff_located = Some((group, index));
        self.pid_verify_go(target, &here)
    }

    /// 【导出比对结果】 (§4.6): ask where to write the diff shown, offering
    /// `<drawing>-diff.json` beside the drawing.
    fn pid_verify_pick_diff_export(&self) -> Task<Message> {
        self.pid_verify_pick(
            "导出比对结果",
            "diff.json",
            &[("JSON", "json")],
            PidVerifyMsg::DiffExport,
        )
    }

    /// Write the diff shown to `name` as `PIDDIFF … EXPORT` writes it, and
    /// say in the message bar where it went or why it did not.
    fn pid_verify_diff_export(&mut self, name: &str) {
        let i = self.active_tab;
        let Some(diff) = self
            .tabs
            .get(i)
            .filter(|tab| !tab.is_start)
            .and_then(|tab| tab.pid_snapshots.diff.as_ref())
        else {
            return;
        };
        let notice = match self.piddiff_write(i, name, diff) {
            Ok((path, bytes)) => Notice::info(format!(
                "已把比对结果写成 {}（{bytes} 字节）。",
                path.display()
            )),
            Err(failure) => Notice::error(failure.code().map(str::to_owned), failure.to_string()),
        };
        self.pid_verify_ui.notice = Some(notice);
    }

    /// 【检查发布条件】 (`only_check`) and 【确认】 under the question 【发布】
    /// asks (§4.6): what `PIDPUBLISH CHECK` and `PIDPUBLISH` call, to the
    /// folder typed in the panel, after `PIDINDEX`'s scan of the drawing's
    /// folder ([`Self::pidpublish_for_panel`]). What stops it before the scan
    /// is said in the message bar; the scan says on the command line what it
    /// says for the command.
    fn pid_verify_publish(&mut self, only_check: bool) -> Task<Message> {
        let i = self.active_tab;
        if self.tabs[i].is_start {
            return Task::none();
        }
        let dir = Some(self.pid_verify_ui.publish_dir.trim())
            .filter(|dir| !dir.is_empty())
            .map(str::to_owned);
        // A scan with nothing to read from disk finishes before this returns,
        // having said in the message bar what came of it.
        self.pid_verify_ui.notice = None;
        match self.pidpublish_for_panel(i, only_check, dir) {
            Ok(task) => task,
            Err((code, why)) => {
                self.pid_verify_ui.notice = Some(Notice::error(Some(code.to_owned()), why));
                Task::none()
            }
        }
    }

    /// 【浏览…】 beside the publish folder (§4.6): ask for a folder, starting
    /// in the drawing's; the one picked is typed in the box.
    fn pid_verify_pick_publish_dir(&self) -> Task<Message> {
        let dir = self
            .tabs
            .get(self.active_tab)
            .filter(|tab| !tab.is_start)
            .and_then(|tab| crate::io::drawing_dir(&tab.scene.document))
            .map(Path::to_path_buf);
        Task::perform(
            async move {
                let mut dialog = crate::sys::file_dialog().set_title("发布目录");
                if let Some(dir) = dir {
                    dialog = dialog.set_directory(dir);
                }
                dialog
                    .pick_folder()
                    .await
                    .map(|picked| crate::sys::handle_path(&picked))
            },
            |picked| match picked {
                Some(path) => {
                    PidVerifyMsg::PublishDir(path.to_string_lossy().into_owned()).message()
                }
                None => Message::Noop,
            },
        )
    }

    /// A place of a tag repeated in the project, clicked under its condition
    /// (§4.6): looked at as a place of a repeated tag in the material list is.
    fn pid_verify_publish_duplicate_locate(&mut self, repeated: usize, at: usize) -> Task<Message> {
        let i = self.active_tab;
        let Some(target) = self
            .tabs
            .get(i)
            .and_then(|tab| tab.pid_publish.gates.as_ref())
            .and_then(|(found, _)| found.duplicates.get(repeated))
            .and_then(|duplicate| {
                let place = duplicate.places.get(at)?;
                Some(Target::placed(
                    &place.drawing_id,
                    &duplicate.type_code,
                    &duplicate.tag,
                    place.at,
                ))
            })
        else {
            return Task::none();
        };
        let here = self.pid_verify_bom_drawing_id(i);
        self.pid_verify_go(target, &here)
    }

    /// 【校验】 on the record of `revision` (§4.6): its package verified as
    /// `PIDPUBLISH VERIFY` verifies one, and what was found kept on the
    /// drawing's tab: 「完好」, or what is missing or does not match.
    fn pid_verify_package_verify(&mut self, revision: u32) {
        let Some(tab) = self
            .tabs
            .get_mut(self.active_tab)
            .filter(|tab| !tab.is_start)
        else {
            return;
        };
        let kept = &mut tab.pid_publish;
        let Some(Ok(records)) = &kept.records else {
            return;
        };
        let Some(package) = records
            .packages
            .iter()
            .find(|package| package.revision == revision)
        else {
            return;
        };
        let verified = match publish_pkg::verify_package(Path::new(&package.path)) {
            Ok(_) => Verified {
                intact: true,
                text: INTACT_SAID.to_owned(),
            },
            Err(error) => Verified {
                intact: false,
                text: why_of(&error),
            },
        };
        kept.verified.retain(|(listed, _)| *listed != revision);
        kept.verified.push((revision, verified));
    }
}

/// What the panel keeps in a bill's place when it did not come out (§4.5):
/// the tags found in more than one place, or the drawings of the scope that
/// did not read. What else stops a bill is `None`: the message bar says it.
fn bom_failed(failure: &BomFailure) -> Option<BomFailed> {
    let (duplicates, unreadable) = match failure {
        BomFailure::Bom(BomError::DuplicateTag(duplicates)) => (duplicates.clone(), Vec::new()),
        BomFailure::Incomplete(drawings) => {
            let unreadable = drawings
                .iter()
                .map(|drawing| Unreadable {
                    drawing_id: drawing.drawing_id.clone(),
                    file: drawing.file.clone(),
                    why: drawing.error.clone().unwrap_or_default(),
                })
                .collect();
            (Vec::new(), unreadable)
        }
        BomFailure::Index(_)
        | BomFailure::Bom(_)
        | BomFailure::NoProjectDir
        | BomFailure::Snapshot(_) => return None,
    };
    Some(BomFailed {
        code: failure.code().to_owned(),
        text: failure.to_string(),
        duplicates,
        unreadable,
    })
}

/// The records the publish half lists (§4.6) of what `pid_publications`
/// lists: each package with its folder, and the revision the next publish
/// makes.
fn records_of(listed: Publications) -> Records {
    let next_revision = publish_pkg::next_revision(&listed.target_dir, &listed.drawing_id);
    let packages = listed
        .packages
        .into_iter()
        .map(|package| Package {
            path: listed
                .target_dir
                .join(&package.package_id)
                .display()
                .to_string(),
            package_id: package.package_id,
            revision: package.revision,
            published_at: package.published_at,
            published_by: package.published_by,
            current: package.current,
        })
        .collect();
    Records {
        drawing_id: listed.drawing_id,
        target_dir: listed.target_dir.display().to_string(),
        next_revision,
        packages,
    }
}

/// The target folder publish records were read from, if they were.
fn target_of(records: Option<&Result<Records, String>>) -> Option<&str> {
    records?
        .as_ref()
        .ok()
        .map(|records| records.target_dir.as_str())
}

/// Switch `item` in `list`: off when it is on, on when it is off.
fn toggle<T: PartialEq>(list: &mut Vec<T>, item: T) {
    match list.iter().position(|on| *on == item) {
        Some(at) => {
            list.remove(at);
        }
        None => list.push(item),
    }
}

/// `limit` and `offset` of a `pid_verify_panel` request: rows per list, at
/// least one, and rows skipped in each. Anything but such whole numbers is
/// `invalid_request`.
fn paging(req: &Value) -> Result<(usize, usize), Value> {
    let limit = count(req, "limit", 1, "正整数（每个列表最多给几行）")?;
    let offset = count(req, "offset", 0, "不小于 0 的整数（每个列表跳过几行）")?;
    Ok((limit.unwrap_or(ROWS_PER_PAGE), offset.unwrap_or(0)))
}

/// A whole number of at least `least` that the request may leave out; the
/// refusal says what to `write`.
fn count(req: &Value, key: &str, least: u64, write: &str) -> Result<Option<usize>, Value> {
    match &req[key] {
        Value::Null => Ok(None),
        asked => match asked.as_u64() {
            Some(n) if n >= least => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
            _ => Err(refuse(
                "invalid_request",
                format!("{key} 写{write}，实得 {asked}"),
            )),
        },
    }
}

/// The control channel's refusal.
fn refuse(code: &str, error: impl Into<String>) -> Value {
    json!({ "ok": false, "status": "failed", "code": code, "error": error.into() })
}

#[cfg(test)]
mod tests {
    use crate::app::config::DockSide;
    use crate::app::{Message, OpenCADStudio};
    use crate::pid::drc;
    use crate::pid::drc::builtin_rules::PROJECT_PACK_FILE;
    use crate::ui::dock::{DockMsg, PanelId};
    use crate::ui::window::pid_verify_deliver::{
        BomRowView, BomTable, DiffGroup, Group, Pick, PidVerifyMsg, RowView, Tab, BOM_DRAWING_ID,
        BOM_EXPORT_ID, BOM_PROJECT_ID, BOM_SEARCH_ID, BOUNDS_PREFIX, BUSY_ID, CANCEL_ID,
        CHECK_DRAWING_ID, CHECK_PROJECT_ID, DIFF_EXPORT_ID, DIFF_ID, NEVER_CHECKED,
        PANEL_BOUNDS_ID, PUBLISH_CHECK_ID, PUBLISH_CONFIRM_ID, PUBLISH_DIR_ID, PUBLISH_ID,
        SNAPSHOT_REASON_ID, SNAPSHOT_SAVE_ID, UNSAVED_SAID,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    const PANEL: PanelId = PanelId::PidVerifyDeliver;

    /// A program with a new drawing open and the default dock layout, which
    /// has no place for the panel yet.
    fn fresh() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        // Tests load the user's persisted config; reset the dock to a known
        // state so this stays hermetic.
        app.dock = Default::default();
        app.dock.ensure_settings();
        app
    }

    /// What one command line said, and the error it ended on.
    struct Ran {
        said: String,
        error: Option<String>,
    }

    /// Run `line` as if typed at the command line, with whatever it starts.
    fn run(app: &mut OpenCADStudio, line: &str) -> Ran {
        app.command_line.history.clear();
        app.command_line.last_error = None;
        let task = app.run_command_line(line);
        app.drive_headless_task(task).expect("the command's task");
        Ran {
            said: app
                .command_line
                .history
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            error: app.command_line.last_error.clone(),
        }
    }

    /// Whether the panel is on screen, and the tab it has in front.
    fn shown(app: &OpenCADStudio) -> (bool, Tab) {
        (app.dock_panel_visible(PANEL), app.pid_verify_view.tab)
    }

    fn undo_steps(app: &OpenCADStudio) -> usize {
        app.tabs[app.active_tab].history.undo_stack.len()
    }

    fn reported_ids() -> Vec<String> {
        crate::ui::wrap_bar::recorded_bounds()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Record bounds for two of the panel's controls and for another
    /// panel's body, as drawing them would.
    fn pretend_drawn() {
        let drawn = iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        for id in [PANEL_BOUNDS_ID, "pid-verify-tab-bom", "pid-collab-panel"] {
            crate::ui::wrap_bar::set_drawn_bounds(id, drawn);
        }
    }

    /// What `pid_verify_panel` answers over the control channel, with the
    /// request's other fields `more` (`,"limit":5` and the like).
    fn panel(app: &mut OpenCADStudio, more: &str) -> Value {
        app.automation_op(&format!(
            r#"{{"protocol":1,"op":"pid_verify_panel"{more}}}"#
        ))
    }

    /// Bring drawing `id` to the front through the control channel, driving
    /// the switch to its end.
    fn activate(app: &mut OpenCADStudio, id: u64, request_id: &str) {
        let (switched, task) = app.control_request(json!({
            "protocol": 1,
            "op": "activate",
            "document_id": id,
            "request_id": request_id,
        }));
        app.drive_headless_task(task).expect("the switch's task");
        assert_eq!(switched["ok"], true, "{switched}");
        assert_eq!(app.tabs[app.active_tab].id, id);
    }

    /// Press `value` in the panel through the `action` op (§3.3), as a
    /// script does, driving whatever it starts to its end.
    fn act(app: &mut OpenCADStudio, value: &str) -> Value {
        static PRESSED: AtomicU64 = AtomicU64::new(0);
        let request_id = format!(
            "pid-verify-action-{}",
            PRESSED.fetch_add(1, Ordering::Relaxed)
        );
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let (answer, task) = app.control_request(json!({
            "protocol": 1,
            "op": "action",
            "name": "pid_verify",
            "value": value,
            "document_id": state["document_id"],
            "request_id": request_id,
        }));
        app.drive_headless_task(task).expect("the action's task");
        if matches!(answer["status"].as_str(), Some("accepted" | "running")) {
            return app
                .control_request(json!({"op": "operation", "request_id": request_id}))
                .0;
        }
        answer
    }

    /// The ids the real view reports once it is drawn.
    fn drawn_ids(app: &OpenCADStudio) -> Vec<String> {
        crate::ui::wrap_bar::forget_bounds("");
        let size = iced::Size::new(1600.0, 900.0);
        let mut simulator =
            iced_test::Simulator::with_size(iced::Settings::default(), size, app.view_main());
        simulator
            .snapshot(&app.active_theme)
            .expect("draw the view");
        reported_ids()
    }

    /// Click the text `label` in the real view, drawn at 1600 × 900, where
    /// the pointer would, and press what the view publishes as [`click`]
    /// does.
    fn click_drawn(app: &mut OpenCADStudio, label: &str) {
        let size = iced::Size::new(1600.0, 900.0);
        let published = {
            let mut simulator =
                iced_test::Simulator::with_size(iced::Settings::default(), size, app.view_main());
            simulator.click(label).expect(label);
            simulator.into_messages().collect::<Vec<_>>()
        };
        assert!(!published.is_empty(), "clicking {label} published nothing");
        for message in published {
            let task = app.update(message);
            app.drive_headless_task(task).expect("the click's task");
        }
    }

    #[test]
    fn pidverify_shows_hides_and_turns_the_panel_to_a_tab() {
        let mut app = fresh();
        assert_eq!(shown(&app), (false, Tab::Check));
        for (line, visible, tab) in [
            ("PIDVERIFY", true, Tab::Check),
            ("PIDVERIFY BOM", true, Tab::Bom),
            ("PIDVERIFY deliver", true, Tab::Deliver),
            ("PIDVERIFY OFF", false, Tab::Deliver),
            // Shown again, it is on the tab it was left on.
            ("PIDVERIFY ON", true, Tab::Deliver),
            ("PIDVERIFY CHECK", true, Tab::Check),
        ] {
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
            assert_eq!(shown(&app), (visible, tab), "{line}");
            let says = if visible {
                format!(
                    "PIDVERIFY: 已显示「校验与交付」面板，当前页签「{}」。",
                    tab.label()
                )
            } else {
                "PIDVERIFY: 已隐藏「校验与交付」面板。".to_owned()
            };
            assert!(ran.said.contains(&says), "{line}: {}", ran.said);
        }
        // The first time, it docked on the right, as the pallet menu docks it.
        assert_eq!(
            app.dock.location(PANEL).map(|(side, _)| side),
            Some(DockSide::Right)
        );
    }

    #[test]
    fn pidverify_bom_typed_at_the_command_line_runs_once() {
        // Off the whole-line list, the command line would try the bare verb
        // first and then run the whole line: two reports, the first one on
        // the rule-check tab.
        let mut app = fresh();
        let ran = run(&mut app, "PIDVERIFY BOM");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(ran.said.matches("PIDVERIFY:").count(), 1, "{}", ran.said);
        assert_eq!(shown(&app), (true, Tab::Bom));
    }

    #[test]
    fn pidverify_names_the_words_it_takes_and_shows_nothing_for_others() {
        let mut app = fresh();
        for line in ["PIDVERIFY SHOW", "PIDVERIFY BOM CHECK"] {
            let error = run(&mut app, line).error.unwrap_or_default();
            assert!(
                error.starts_with("PIDVERIFY: 只接受 ON、OFF、CHECK、BOM、DELIVER 中的一个"),
                "{line}: {error}"
            );
        }
        assert_eq!(shown(&app), (false, Tab::Check));
    }

    #[test]
    fn closing_the_panel_forgets_where_its_controls_were() {
        let mut app = fresh();
        for how in ["PIDVERIFY OFF", "the close button"] {
            let _ = run(&mut app, "PIDVERIFY");
            pretend_drawn();
            if how == "the close button" {
                let _ = app.on_dock(DockMsg::Close(PANEL));
            } else {
                let _ = run(&mut app, how);
            }
            assert!(!app.dock_panel_visible(PANEL), "{how}");
            let left = reported_ids();
            assert!(
                !left.iter().any(|id| id.starts_with(BOUNDS_PREFIX)),
                "{how}: {left:?}"
            );
            // Only its own: what another panel reported stays.
            assert!(
                left.iter().any(|id| id == "pid-collab-panel"),
                "{how}: {left:?}"
            );
        }
        crate::ui::wrap_bar::forget_bounds("pid-collab-panel");
    }

    #[test]
    fn a_read_only_session_runs_every_form() {
        let mut app = fresh();
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for line in [
            "PIDVERIFY",
            "PIDVERIFY BOM",
            "PIDVERIFY DELIVER",
            "PIDVERIFY OFF",
            "PIDVERIFY ON",
            "PIDVERIFY CHECK",
        ] {
            assert!(!app.readonly_refuses_command(line), "{line}");
            let ran = run(&mut app, line);
            assert_eq!(ran.error, None, "{line}: {}", ran.said);
        }
        assert_eq!(shown(&app), (true, Tab::Check));
        // Showing the panel changed nothing in the drawing.
        assert_eq!(undo_steps(&app), steps);
    }

    #[test]
    fn a_tab_turned_in_the_panel_is_where_the_command_finds_it() {
        let mut app = fresh();
        let _ = run(&mut app, "PIDVERIFY");
        let _ = app.update(Message::PidVerify(PidVerifyMsg::Tab(Tab::Deliver)));
        assert_eq!(shown(&app), (true, Tab::Deliver));
        let _ = run(&mut app, "PIDVERIFY OFF");
        let ran = run(&mut app, "PIDVERIFY");
        assert_eq!(shown(&app), (true, Tab::Deliver));
        assert!(ran.said.contains("当前页签「快照与发布」"), "{}", ran.said);
    }

    #[test]
    fn pid_verify_panel_reads_the_panel_shown_or_hidden() {
        let mut app = fresh();
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        // Never shown, it answers all the same.
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["visible"], false, "{answer}");
        assert_eq!(answer["tab"], "check", "{answer}");
        assert_eq!(answer["document_id"], state["document_id"], "{answer}");
        // A drawing never saved has no number yet, as the rule check says.
        assert_eq!(answer["drawing_id"], "", "{answer}");
        assert_eq!(answer["writable"], true, "{answer}");
        for block in ["check", "bom", "deliver"] {
            assert!(answer[block].is_object(), "{block}: {answer}");
        }
        for (line, visible, tab) in [
            ("PIDVERIFY BOM", true, "bom"),
            ("PIDVERIFY OFF", false, "bom"),
            ("PIDVERIFY DELIVER", true, "deliver"),
        ] {
            let _ = run(&mut app, line);
            let answer = panel(&mut app, "");
            assert_eq!(answer["visible"], visible, "{line}: {answer}");
            assert_eq!(answer["tab"], tab, "{line}: {answer}");
        }
    }

    #[test]
    fn pid_verify_panel_follows_the_drawing_tab_in_front() {
        let mut app = fresh();
        let first = app.tabs[app.active_tab].id;
        let second = app.push_test_document();
        // Where the files came from, as opening them leaves it. With no
        // project file beside them, each is numbered by its file name.
        let folder =
            std::env::temp_dir().join(format!("ocs-pid-verify-panel-{}", std::process::id()));
        for (id, file) in [(first, "PID-1101.dwg"), (second, "PID-1102.dwg")] {
            let tab = app
                .tabs
                .iter_mut()
                .find(|tab| tab.id == id)
                .expect("the tab");
            tab.scene.document.source_path = Some(folder.join(file).to_string_lossy().into_owned());
        }
        for (request_id, id, number) in [
            ("pid-verify-first", first, "PID-1101"),
            ("pid-verify-second", second, "PID-1102"),
            ("pid-verify-first-again", first, "PID-1101"),
        ] {
            activate(&mut app, id, request_id);
            let answer = panel(&mut app, "");
            assert_eq!(answer["document_id"], id, "{answer}");
            assert_eq!(answer["drawing_id"], number, "{answer}");
        }
    }

    #[test]
    fn pid_verify_panel_says_whether_the_session_may_write() {
        let mut app = fresh();
        for (readonly, writable) in [(false, true), (true, false), (false, true)] {
            app.collab.readonly = readonly;
            let answer = panel(&mut app, "");
            assert_eq!(
                answer["writable"], writable,
                "read-only {readonly}: {answer}"
            );
        }
    }

    #[test]
    fn pid_verify_panel_answers_on_the_start_page() {
        let mut app = OpenCADStudio::new_for_test();
        assert!(app.tabs[app.active_tab].is_start);
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["visible"], false, "{answer}");
        assert_eq!(answer["document_id"], Value::Null, "{answer}");
        assert_eq!(answer["drawing_id"], Value::Null, "{answer}");
    }

    #[test]
    fn pid_verify_panel_pages_by_whole_numbers_only() {
        let mut app = fresh();
        for more in [
            r#","limit":5"#,
            r#","offset":3"#,
            r#","limit":1,"offset":0"#,
        ] {
            let answer = panel(&mut app, more);
            assert_eq!(answer["ok"], true, "{more}: {answer}");
        }
        for more in [
            r#","limit":0"#,
            r#","limit":-1"#,
            r#","limit":2.5"#,
            r#","limit":"many""#,
            r#","offset":-1"#,
            r#","offset":"next""#,
        ] {
            let answer = panel(&mut app, more);
            assert_eq!(answer["code"], "invalid_request", "{more}: {answer}");
        }
    }

    #[test]
    fn pid_verify_panel_is_a_query_listed_among_the_capabilities() {
        let mut app = fresh();
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let listed = state["capabilities"]
            .as_array()
            .is_some_and(|names| names.iter().any(|name| name == "pid_verify_panel"));
        assert!(listed, "{}", state["capabilities"]);
        // A query answers while automation is stopped.
        let stop = json!({
            "protocol": 1,
            "op": "stop",
            "document_id": state["document_id"],
            "revision": state["revision"],
            "client_id": "test",
            "request_id": "pid-verify-stop",
        });
        let stopped = app.automation_op(&stop.to_string());
        assert_eq!(stopped["ok"], true, "{stopped}");
        let answer = panel(&mut app, "");
        assert_eq!(answer["ok"], true, "{answer}");
    }

    #[test]
    fn pid_verify_action_shows_hides_and_turns_the_panel() {
        let mut app = fresh();
        for (value, visible, tab) in [
            ("show", true, "check"),
            ("tab=bom", true, "bom"),
            ("tab=deliver", true, "deliver"),
            ("hide", false, "deliver"),
            // Hidden, the panel turns all the same, and shows on that tab.
            ("tab=check", false, "check"),
            ("show", true, "check"),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(answer["status"], "completed", "{value}: {answer}");
            // The action only presses; what it did is read off the panel.
            assert_eq!(answer["result"], json!({}), "{value}: {answer}");
            let state = panel(&mut app, "");
            assert_eq!(state["visible"], visible, "{value}: {state}");
            assert_eq!(state["tab"], tab, "{value}: {state}");
        }
        // The first time, it docked on the right, as `PIDVERIFY` docks it.
        assert_eq!(
            app.dock.location(PANEL).map(|(side, _)| side),
            Some(DockSide::Right)
        );
    }

    #[test]
    fn pid_verify_action_refuses_what_the_panel_does_not_have() {
        let mut app = fresh();
        for value in ["open", "show=1", "tab=report"] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], false, "{value}: {answer}");
            assert_eq!(answer["status"], "failed", "{value}: {answer}");
            assert_eq!(answer["code"], "invalid_value", "{value}: {answer}");
            assert!(
                answer["error"]
                    .as_str()
                    .is_some_and(|said| !said.is_empty()),
                "{value}: {answer}"
            );
            assert_eq!(answer.get("reason"), None, "{value}: {answer}");
        }
        // With no value there is nothing to press, as for every action.
        let answer = act(&mut app, "");
        assert_eq!(answer["code"], "invalid_request", "{answer}");
        // Nothing was pressed.
        let state = panel(&mut app, "");
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "check", "{state}");
    }

    #[test]
    fn pid_verify_action_is_open_to_a_read_only_session() {
        let mut app = fresh();
        app.collab.readonly = true;
        let steps = undo_steps(&app);
        for value in ["show", "tab=bom", "hide"] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
        }
        let state = panel(&mut app, "");
        assert_eq!(state["visible"], false, "{state}");
        assert_eq!(state["tab"], "bom", "{state}");
        // Pressing the panel changed nothing in the drawing.
        assert_eq!(undo_steps(&app), steps);
    }

    #[test]
    fn the_drawn_panel_reports_its_body_and_tabs_until_it_closes() {
        // Building and drawing the real view is deeper than a default test
        // thread's stack, and two headless renderers made at once crash the
        // process.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let mut app = fresh();
                app.win_size = (1600.0, 900.0);
                let _ = run(&mut app, "PIDVERIFY BOM");
                let drawn = drawn_ids(&app);
                for id in [
                    PANEL_BOUNDS_ID,
                    "pid-verify-tab-check",
                    "pid-verify-tab-bom",
                    "pid-verify-tab-deliver",
                ] {
                    assert!(drawn.iter().any(|seen| seen == id), "{id}: {drawn:?}");
                }
                // Closed before the next frame, the panel reports nothing.
                let _ = run(&mut app, "PIDVERIFY OFF");
                let left = reported_ids();
                assert!(
                    !left.iter().any(|id| id.starts_with(BOUNDS_PREFIX)),
                    "{left:?}"
                );
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// A folder of one test's own, removed after it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // Tests run in parallel: a count tells apart those of one process.
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-pid-verify-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("the scratch folder");
            Self(path)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A control request of protocol 1 to the drawing in front.
    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        req["protocol"] = json!(1);
        req["document_id"] = state["document_id"].clone();
        req["revision"] = state["revision"].clone();
        req["client_id"] = json!("test");
        req["request_id"] = json!(format!(
            "pid-verify-request-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        app.automation_op(&req.to_string())
    }

    /// Start `cmd`, feed it `points`, then press Enter.
    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 2]]) {
        let started = request(app, json!({"op": "run", "cmd": cmd}));
        assert_eq!(started["ok"], true, "{cmd}: {started}");
        for [x, y] in points {
            let point =
                json!({"op": "input", "kind": "point", "point": [x, y, 0.0], "space": "wcs"});
            let fed = request(app, point);
            assert_eq!(fed["ok"], true, "{cmd}: {fed}");
        }
        request(app, json!({"op": "input", "kind": "enter"}));
    }

    /// Type `line` at the command line and press Enter, as a person does:
    /// both are messages the program handles.
    fn typed(app: &mut OpenCADStudio, line: &str) {
        let _ = app.update(Message::CommandInput(line.to_owned()));
        let task = app.update(Message::CommandSubmit);
        app.drive_headless_task(task).expect("the command's task");
    }

    /// A rule of the panel tests' own set: `expr` on each object of kind
    /// `target`.
    fn rule(id: &str, target: &str, severity: &str, expr: &str) -> Value {
        json!({
            "id": id, "name": format!("面板验收 {id}"), "set": "面板验收",
            "category": "设计类", "target": target, "severity": severity,
            "expr": expr, "scope": "项目", "source": "新建"
        })
    }

    /// The project rule pack the panel tests check with: the built-in sets
    /// off and three rules of a set of its own, so that a pump and a pipe
    /// line make one error, one warning and one note.
    fn pack() -> Value {
        json!({
            "version": 1,
            "pack": "面板验收",
            "rule_set_version": "T1",
            "sets": [
                {"name": "基础规则", "enabled": false},
                {"name": "工程扩展", "enabled": false},
                {"name": "面板验收", "enabled": true}
            ],
            "rules": [
                rule("DR-201", "管线", "错误", "管道等级 = NULL"),
                rule("DR-202", "设备", "警告", "流量 = NULL"),
                rule("DR-203", "管线", "提示", "设计压力 = NULL")
            ]
        })
    }

    /// A pump and a pipe line saved as PID-1101.dwg in `scratch`, beside
    /// [`pack`]: checked, the line has an error (DR-201) and a note (DR-203)
    /// and the pump a warning (DR-202). The panel is open.
    fn sheet(scratch: &Scratch) -> OpenCADStudio {
        let mut app = fresh();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
        fs::write(scratch.path(PROJECT_PACK_FILE), pack().to_string()).expect("the pack");
        let path = scratch.path("PID-1101.dwg");
        let bytes = crate::io::save_to_bytes(
            app.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("the drawing as DWG");
        fs::write(&path, bytes).expect("the drawing");
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(path.to_string_lossy().into_owned());
        let _ = run(&mut app, "PIDVERIFY");
        app
    }

    /// Waive the note on the line at the command line, keyed as the check
    /// kept on the drawing's tab found it.
    fn waive_the_note(app: &mut OpenCADStudio) {
        let i = app.active_tab;
        let key = app.tabs[i]
            .pid_check
            .as_ref()
            .and_then(|checked| {
                let mut found = checked.report.results.iter();
                found.find(|result| result.rule_id == "DR-203")
            })
            .map(|result| result.object_key.clone())
            .expect("the note in the check kept on the tab");
        let ran = run(app, &format!("PIDCHECK WAIVE DR-203 \"{key}\" 放空管"));
        assert_eq!(ran.error, None, "{}", ran.said);
    }

    /// The rules of the rows the panel state lists, in order.
    fn rows(state: &Value) -> Vec<String> {
        state["check"]["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| row["rule_id"].as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the check the panel draws now is marked stale.
    fn drawn_stale(app: &OpenCADStudio) -> bool {
        let shown = app.pid_verify_view.check.shown.as_ref();
        shown.expect("a check drawn").stale
    }

    /// [`sheet`], with PID-1102.dwg beside it holding a pump of its own:
    /// checked as a project, each pump has a warning (DR-202).
    fn two_sheets(scratch: &Scratch) -> OpenCADStudio {
        let mut other = fresh();
        draw(&mut other, "PIDINSERT pump", &[[0.0, 0.0]]);
        let bytes = crate::io::save_to_bytes(
            other.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("the drawing as DWG");
        fs::write(scratch.path("PID-1102.dwg"), bytes).expect("the drawing");
        sheet(scratch)
    }

    /// The index and the label of the row of rule `rule` on drawing
    /// `drawing` in the panel state.
    fn row_of(state: &Value, rule: &str, drawing: &str) -> (usize, String) {
        let rows = state["check"]["rows"].as_array().expect("the rows");
        let row = rows
            .iter()
            .find(|row| row["rule_id"] == rule && row["drawing_id"] == drawing)
            .unwrap_or_else(|| panic!("{rule} on {drawing}: {state}"));
        let index = row["index"].as_u64().expect("the index");
        (
            usize::try_from(index).expect("an index"),
            row["label"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// The handles `get_selection` reads off the drawing in front.
    fn selection(app: &mut OpenCADStudio) -> Vec<String> {
        let answer = app.automation_op(r#"{"protocol":1,"op":"get_selection"}"#);
        let entities = answer["result"]["entities"].as_array();
        entities
            .expect("the selection")
            .iter()
            .map(|entity| entity["handle"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Where the view of the drawing in front looks.
    fn looking_at(app: &OpenCADStudio) -> glam::DVec3 {
        app.tabs[app.active_tab].scene.camera.borrow().target
    }

    /// Select nothing on the drawing in front and look far away from it.
    fn look_away(app: &mut OpenCADStudio) {
        let i = app.active_tab;
        app.tabs[i].scene.deselect_all();
        app.tabs[i].scene.camera.borrow_mut().target = glam::DVec3::new(5000.0, 5000.0, 0.0);
    }

    /// What the command line said since it was last cleared mentions
    /// `PIDLOCATE`.
    fn pidlocate_said(app: &OpenCADStudio) -> bool {
        let history = &app.command_line.history;
        history.iter().any(|entry| entry.text.contains("PIDLOCATE"))
    }

    /// The rows of the waived group the panel draws now.
    fn drawn_waived(app: &OpenCADStudio) -> Vec<usize> {
        let shown = app.pid_verify_view.check.shown.as_ref();
        let shown = shown.expect("a check drawn");
        let groups = shown.groups.iter();
        groups
            .filter(|group| group.group == Group::Waived)
            .flat_map(|group| &shown.rows[group.first..group.first + group.count])
            .map(|row| row.index)
            .collect()
    }

    /// What is typed in the reason box under the row of `index`, while it
    /// is open.
    fn drawn_reason(app: &OpenCADStudio, index: usize) -> Option<String> {
        let shown = app.pid_verify_view.check.shown.as_ref();
        let mut rows = shown.expect("a check drawn").rows.iter();
        rows.find(|row| row.index == index)
            .and_then(|row| row.reason.clone())
    }

    #[test]
    fn the_rule_check_tab_counts_as_pid_check_does_and_filters_its_list() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        let summary = app.automation_op(r#"{"protocol":1,"op":"pid_check"}"#)["summary"].clone();
        for count in ["错误", "警告", "提示", "已豁免"] {
            assert_eq!(
                state["check"]["counts"][count], summary[count],
                "{count}: {state}"
            );
        }
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 0, "已豁免": 1}),
            "{state}"
        );
        assert_eq!(rows(&state), ["DR-201", "DR-202"]);
        for (value, listed) in [
            ("severity=警告", vec!["DR-201"]),
            ("severity=警告", vec!["DR-201", "DR-202"]),
            ("show_waived=1", vec!["DR-201", "DR-202", "DR-203"]),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(rows(&panel(&mut app, "")), listed, "{value}");
        }
    }

    #[test]
    fn a_check_typed_at_the_command_line_shows_unpressed_and_goes_stale_with_an_edit() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        assert_eq!(app.pid_verify_view.empty, Some(NEVER_CHECKED));
        typed(&mut app, "PIDCHECK");
        // What the panel draws shows it already: nothing pressed, nothing
        // asked of the panel.
        let shown = app
            .pid_verify_view
            .check
            .shown
            .clone()
            .expect("the check drawn");
        let counts = shown.summary;
        assert_eq!((counts.errors, counts.warnings, counts.infos), (1, 1, 1));
        assert!(!shown.stale);
        // Another pump: the check is stale in the view drawn after the next
        // message, and in the panel state.
        draw(&mut app, "PIDINSERT pump", &[[0.0, -60.0]]);
        let _ = app.update(Message::Noop);
        assert!(drawn_stale(&app));
        assert_eq!(panel(&mut app, "")["check"]["stale"], true);
        // Checked again, it is fresh, and has found that pump.
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(!drawn_stale(&app));
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(state["check"]["counts"]["警告"], 2, "{state}");
    }

    #[test]
    fn waiving_leaves_the_check_fresh_and_undoing_the_waiver_does_not() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 1, "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        // The result follows the undo in place, but the panel cannot tell
        // what was undone.
        let _ = app.automation_op(r#"{"op":"undo"}"#);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 0, "{state}");
        assert_eq!(state["check"]["stale"], true, "{state}");
    }

    #[test]
    fn the_project_check_pressed_in_the_panel_is_shown_until_a_drawing_of_the_folder_changes() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let answer = act(&mut app, "check=project");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(app.pid_checks.of_project(&scratch.0).is_some());
        // Newer than the drawing's own, the folder's check is shown.
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        // The scan done, the button is pressable again.
        assert_eq!(state["buttons"]["check_project"], true, "{state}");
        // Waiving keeps it fresh; editing a drawing of the folder does not.
        waive_the_note(&mut app);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["counts"]["已豁免"], 1, "{state}");
        assert_eq!(state["check"]["stale"], false, "{state}");
        draw(&mut app, "PIDINSERT pump", &[[0.0, -60.0]]);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["stale"], true, "{state}");
    }

    #[test]
    fn the_panel_refuses_a_grey_rule_check_button_with_the_reason() {
        // Never saved, the drawing is in no project folder.
        let mut app = fresh();
        let _ = run(&mut app, "PIDVERIFY");
        for (value, reason) in [
            ("check=project", "no_project_dir"),
            ("check_export=drc.json", "no_result"),
            ("severity=警告", "no_result"),
        ] {
            let answer = act(&mut app, value);
            assert_eq!(
                answer["code"], "PANEL_ACTION_UNAVAILABLE",
                "{value}: {answer}"
            );
            assert_eq!(answer["reason"], reason, "{value}: {answer}");
        }
        // The drawing itself checks all the same.
        let answer = act(&mut app, "check=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["has_result"], true, "{state}");
        assert_eq!(state["buttons"]["check_project"], false, "{state}");
        assert_eq!(state["buttons"]["check_export"], true, "{state}");
    }

    #[test]
    fn the_panel_exports_the_check_shown_as_pidcheck_export_writes_it() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let _ = act(&mut app, "check=drawing");
        let answer = act(&mut app, "check_export=panel.json");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        let said = state["notice"]["text"].as_str().unwrap_or_default();
        assert!(said.contains("panel.json"), "{state}");
        let ran = run(&mut app, "PIDCHECK EXPORT command.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let written = |name: &str| fs::read(scratch.path(name)).expect("the file written");
        assert_eq!(written("panel.json"), written("command.json"));
        // What the export refuses is said in the message bar; the press was
        // made all the same.
        app.command_line.last_error = None;
        let answer = act(&mut app, "check_export=panel.txt");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.command_line.last_error, None);
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        assert!(!scratch.path("panel.txt").exists());
    }

    #[test]
    fn a_rule_pack_that_does_not_read_is_said_in_the_message_bar_and_the_check_stays() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let _ = act(&mut app, "check=drawing");
        let kept = panel(&mut app, "")["check"].clone();
        let mut broken = pack();
        broken["rules"][0]["expr"] = json!("管道等级 =");
        fs::write(scratch.path(PROJECT_PACK_FILE), broken.to_string()).expect("the pack");
        for value in ["check=drawing", "check=project"] {
            app.command_line.last_error = None;
            let answer = act(&mut app, value);
            assert_eq!(answer["ok"], true, "{value}: {answer}");
            assert_eq!(app.command_line.last_error, None, "{value}");
            let state = panel(&mut app, "");
            assert_eq!(state["notice"]["level"], "error", "{value}: {state}");
            assert_eq!(
                state["notice"]["code"],
                drc::DSL_PARSE_ERROR,
                "{value}: {state}"
            );
            assert_eq!(state["check"], kept, "{value}: {state}");
        }
        // Mended, the next check clears the message.
        fs::write(scratch.path(PROJECT_PACK_FILE), pack().to_string()).expect("the pack");
        let _ = act(&mut app, "check=drawing");
        assert_eq!(panel(&mut app, "")["notice"], Value::Null);
    }

    #[test]
    fn locating_a_row_selects_and_frames_its_object_as_pidlocate_does() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let (pump, tag) = row_of(&panel(&mut app, ""), "DR-202", "PID-1101");
        assert!(!tag.is_empty());
        let ran = run(&mut app, &format!("PIDLOCATE {tag}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        let (selected, framed) = (selection(&mut app), looking_at(&app));
        assert_eq!(selected.len(), 1, "{}", ran.said);
        look_away(&mut app);
        app.command_line.history.clear();
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(selection(&mut app), selected);
        let looking = looking_at(&app);
        assert!(looking.distance(framed) < 1e-6, "{looking} ≠ {framed}");
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["located"], pump, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        // The panel finds it without a word on the command line.
        assert!(!pidlocate_said(&app));
        let answer = act(&mut app, "locate=99");
        assert_eq!(answer["code"], "PANEL_ROW_NOT_FOUND", "{answer}");
    }

    #[test]
    fn locating_a_row_on_another_drawing_opens_it_and_keeps_the_project_check_shown() {
        let scratch = Scratch::new();
        let mut app = two_sheets(&scratch);
        let first = app.active_tab;
        let tabs = app.tabs.len();
        let answer = act(&mut app, "check=project");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        let total = state["check"]["total"].clone();
        let (pump, tag) = row_of(&state, "DR-202", "PID-1102");
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let other = scratch.path("PID-1102.dwg");
        assert_eq!(app.tabs.len(), tabs + 1);
        assert_eq!(app.tab_showing(&other), Some(app.active_tab));
        let selected = selection(&mut app);
        assert_eq!(selected.len(), 1, "{selected:?}");
        let framed = looking_at(&app);
        // The project's check is still the one shown, the row clicked
        // located in it.
        let state = panel(&mut app, "");
        assert_eq!(state["drawing_id"], "PID-1102", "{state}");
        assert_eq!(state["check"]["scope"], "project", "{state}");
        assert_eq!(state["check"]["total"], total, "{state}");
        assert_eq!(state["check"]["located"], pump, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        // What PIDLOCATE finds there.
        look_away(&mut app);
        let ran = run(&mut app, &format!("PIDLOCATE {tag} PID-1102"));
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(selection(&mut app), selected, "{}", ran.said);
        assert!(looking_at(&app).distance(framed) < 1e-6, "{}", ran.said);
        // From PID-1101 again: the tab open on it is brought forward, no
        // second one opened.
        let _ = app.update(Message::TabSwitch(first));
        let i = app.tab_showing(&other).expect("the tab on PID-1102");
        app.tabs[i].scene.deselect_all();
        let answer = act(&mut app, &format!("locate={pump}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.tabs.len(), tabs + 1);
        assert_eq!(app.active_tab, i);
        assert_eq!(selection(&mut app), selected);
    }

    /// [`sheet`] checked with the built-in rules on beside the panel tests'
    /// own, equipment required to carry two properties no object has, and a
    /// rule of the panel tests' own reading the tag of equipment: the pump
    /// has a `DR-010` naming both, and a `DR-204` naming `位号`; the line a
    /// `DR-001` naming `管道等级`.
    fn highlight_sheet(scratch: &Scratch) -> OpenCADStudio {
        let mut app = sheet(scratch);
        let pack = json!({
            "version": 1,
            "pack": "高亮验收",
            "rule_set_version": "H1",
            "required_fields": {"EQUIPMENT": ["铭牌号", "出厂编号"]},
            "sets": [
                {"name": "基础规则", "enabled": true},
                {"name": "工程扩展", "enabled": false},
                {"name": "面板验收", "enabled": true}
            ],
            "rules": [rule("DR-204", "设备", "提示", "位号.长度 > 3")]
        });
        fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("the pack");
        typed(&mut app, "PIDCHECK");
        app
    }

    /// The row of rule `rule` on the object whose key starts with `key` in
    /// the panel state: its index, the properties it names and its label.
    fn row_naming(state: &Value, rule: &str, key: &str) -> (usize, Vec<String>, String) {
        let rows = state["check"]["rows"].as_array().expect("the rows");
        let row = rows
            .iter()
            .find(|row| {
                let object = row["object_key"].as_str().unwrap_or_default();
                row["rule_id"] == rule && object.starts_with(key)
            })
            .unwrap_or_else(|| panic!("{rule} on {key}: {state}"));
        let index = row["index"].as_u64().expect("the index");
        let fields = row["fields"].as_array().expect("the fields");
        (
            usize::try_from(index).expect("an index"),
            fields
                .iter()
                .map(|field| field.as_str().unwrap_or_default().to_owned())
                .collect(),
            row["label"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// What `properties` answers for the selection on the drawing in front.
    fn properties(app: &mut OpenCADStudio) -> Value {
        app.automation_op(r#"{"protocol":1,"op":"properties"}"#)
    }

    /// The rows of what `properties` answered, every section's in order.
    fn property_rows(answer: &Value) -> Vec<Value> {
        let sections = answer["result"]["sections"].as_array();
        let sections = sections.or_else(|| answer["sections"].as_array());
        sections
            .unwrap_or_else(|| panic!("the sections: {answer}"))
            .iter()
            .flat_map(|section| {
                section["properties"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }

    /// The ids of the rows `properties` marks highlighted, in order.
    fn highlighted(answer: &Value) -> Vec<String> {
        property_rows(answer)
            .iter()
            .filter(|row| row["highlighted"] == true)
            .map(|row| row["id"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// The row `id` of what `properties` answered.
    fn property_row(answer: &Value, id: &str) -> Value {
        let rows = property_rows(answer);
        let row = rows.into_iter().find(|row| row["id"] == id);
        row.unwrap_or_else(|| panic!("no row {id}: {answer}"))
    }

    /// Commit `value` in the row `id` of the selection through `property`,
    /// the way the property table commits what is typed in a row.
    fn fill(app: &mut OpenCADStudio, id: &str, value: &str) -> Value {
        static FILLED: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let (answer, task) = app.control_request(json!({
            "protocol": 1,
            "op": "property",
            "field": id,
            "value": value,
            "document_id": state["document_id"],
            "revision": state["revision"],
            "request_id": format!("pid-verify-fill-{}", FILLED.fetch_add(1, Ordering::Relaxed)),
        }));
        app.drive_headless_task(task).expect("the commit's task");
        answer
    }

    /// §3.6: the rows a highlight names are `pid_props_<property>` for the
    /// properties it lists, and `pid_tag` when it lists the tag.
    #[test]
    fn pid_verify_highlight_names_rows_by_their_ids() {
        let fields = ["管道等级".to_owned(), "位号".to_owned()];
        assert!(super::highlights_row(&fields, "pid_props_管道等级"));
        assert!(super::highlights_row(&fields, "pid_tag"));
        assert!(!super::highlights_row(&fields, "pid_props_设计压力"));
        assert!(!super::highlights_row(&fields, "pid_type"));
        assert!(!super::highlights_row(&fields, "管道等级"));
        assert!(!super::highlights_row(&fields[..1], "pid_tag"));
        assert!(!super::highlights_row(&[], "pid_props_"));
    }

    /// §3.6: a row of a result naming properties, clicked, highlights their
    /// rows in what `properties` answers — a property its object does not
    /// carry yet in an empty row of its own — and a rule reading the tag
    /// highlights the tag's row.
    #[test]
    fn pid_verify_highlight_marks_the_rows_a_clicked_result_names() {
        let scratch = Scratch::new();
        let mut app = highlight_sheet(&scratch);
        let state = panel(&mut app, "");
        let (missing, fields, _) = row_naming(&state, "DR-010", "EQUIPMENT|");
        assert_eq!(fields, ["铭牌号", "出厂编号"], "{state}");
        let answer = act(&mut app, &format!("locate={missing}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let rows = properties(&mut app);
        let ids = ["pid_props_铭牌号", "pid_props_出厂编号"];
        assert_eq!(highlighted(&rows), ids, "{rows}");
        for id in ids {
            let row = property_row(&rows, id);
            assert_eq!(
                (&row["kind"], &row["value"]),
                (&json!("text"), &json!("")),
                "{row}"
            );
        }
        let (grade, fields, _) = row_naming(&state, "DR-001", "PIPE_LINE|");
        assert_eq!(fields, ["管道等级"], "{state}");
        let answer = act(&mut app, &format!("locate={grade}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let rows = properties(&mut app);
        assert_eq!(highlighted(&rows), ["pid_props_管道等级"], "{rows}");
        let (tag, fields, _) = row_naming(&state, "DR-204", "EQUIPMENT|");
        assert_eq!(fields, ["位号"], "{state}");
        let answer = act(&mut app, &format!("locate={tag}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let rows = properties(&mut app);
        assert_eq!(highlighted(&rows), ["pid_tag"], "{rows}");
    }

    /// §3.6: a value typed in the empty row of a highlighted property is
    /// committed as in any row, creating the key, and that row alone is no
    /// longer highlighted; both filled, the result is gone from the next
    /// check. Left empty, the row writes nothing.
    #[test]
    fn pid_verify_highlight_fills_its_empty_rows_as_any_row_is_filled() {
        let scratch = Scratch::new();
        let mut app = highlight_sheet(&scratch);
        let (missing, ..) = row_naming(&panel(&mut app, ""), "DR-010", "EQUIPMENT|");
        let answer = act(&mut app, &format!("locate={missing}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let i = app.active_tab;
        app.tabs[i].dirty = false;
        let (steps, revision) = (undo_steps(&app), app.tabs[i].edit_revision);
        let answer = fill(&mut app, "pid_props_铭牌号", "");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(!app.tabs[i].dirty);
        assert_eq!(
            (undo_steps(&app), app.tabs[i].edit_revision),
            (steps, revision)
        );
        let rows = properties(&mut app);
        let ids = ["pid_props_铭牌号", "pid_props_出厂编号"];
        assert_eq!(highlighted(&rows), ids, "{rows}");

        let answer = fill(&mut app, "pid_props_铭牌号", "NP-0001");
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(app.tabs[i].dirty);
        assert_eq!(undo_steps(&app), steps + 1);
        let rows = properties(&mut app);
        assert_eq!(property_row(&rows, "pid_props_铭牌号")["value"], "NP-0001");
        assert_eq!(highlighted(&rows), ["pid_props_出厂编号"], "{rows}");
        let answer = fill(&mut app, "pid_props_出厂编号", "2026-11");
        assert_eq!(answer["ok"], true, "{answer}");
        let rows = properties(&mut app);
        assert_eq!(highlighted(&rows), Vec::<String>::new(), "{rows}");
        assert_eq!(app.pid_verify_highlight, None);

        typed(&mut app, "PIDCHECK");
        let state = panel(&mut app, "");
        let rows = state["check"]["rows"].as_array().expect("the rows");
        let left = rows.iter().any(|row| {
            let object = row["object_key"].as_str().unwrap_or_default();
            row["rule_id"] == "DR-010" && object.starts_with("EQUIPMENT|")
        });
        assert!(!left, "{state}");
    }

    /// §3.6: the highlight goes once something else is selected, another
    /// row is clicked or its drawing closes; with none, `properties` answers
    /// as it did before there was any.
    #[test]
    fn pid_verify_highlight_goes_with_the_selection_another_row_and_the_drawing() {
        let scratch = Scratch::new();
        let mut app = highlight_sheet(&scratch);
        let state = panel(&mut app, "");
        let (missing, _, pump) = row_naming(&state, "DR-010", "EQUIPMENT|");
        let (tag, ..) = row_naming(&state, "DR-204", "EQUIPMENT|");
        let (_, _, line) = row_naming(&state, "DR-001", "PIPE_LINE|");
        let ran = run(&mut app, &format!("PIDLOCATE {pump}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        let plain = property_rows(&properties(&mut app));
        assert!(
            plain.iter().all(|row| row.get("highlighted").is_none()),
            "{plain:?}"
        );

        let answer = act(&mut app, &format!("locate={missing}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_ne!(highlighted(&properties(&mut app)), Vec::<String>::new());
        let ran = run(&mut app, &format!("PIDLOCATE {line}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        let rows = properties(&mut app);
        assert_eq!(highlighted(&rows), Vec::<String>::new(), "{rows}");
        assert_eq!(app.pid_verify_highlight, None);
        let ran = run(&mut app, &format!("PIDLOCATE {pump}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(property_rows(&properties(&mut app)), plain);

        let answer = act(&mut app, &format!("locate={missing}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let answer = act(&mut app, &format!("locate={tag}"));
        assert_eq!(answer["ok"], true, "{answer}");
        let rows = properties(&mut app);
        assert_eq!(highlighted(&rows), ["pid_tag"], "{rows}");
        assert!(property_rows(&rows)
            .iter()
            .all(|row| row["id"] != "pid_props_铭牌号"));

        let id = app.tabs[app.active_tab].id;
        let (closed, task) = app.control_request(json!({
            "protocol": 1,
            "op": "close",
            "request_id": "pid-verify-highlight-close",
            "document_id": id,
            "discard": true,
        }));
        app.drive_headless_task(task).expect("the close's task");
        assert_eq!(closed["ok"], true, "{closed}");
        assert_eq!(app.pid_verify_highlight, None);
    }

    /// The real view drawn at 1600 × 900 as pixels, how many of them make a
    /// logical one, and where each of `labels` is drawn in it: `None` for one
    /// it does not show.
    fn drawn_pixels(
        app: &OpenCADStudio,
        labels: &[&str],
    ) -> (image::RgbaImage, f32, Vec<Option<iced::Rectangle>>) {
        let size = iced::Size::new(1600.0, 900.0);
        let mut simulator =
            iced_test::Simulator::with_size(iced::Settings::default(), size, app.view_main());
        let found = labels
            .iter()
            .map(|label| {
                let text = simulator.find(*label).ok();
                text.and_then(|text| text.visible_bounds())
            })
            .collect();
        // With no picture to compare with, `matches_image` writes it, under a
        // name of its own.
        let scratch = Scratch::new();
        simulator
            .snapshot(&app.active_theme)
            .expect("draw the view")
            .matches_image(scratch.path("view.png"))
            .expect("the picture is written");
        let written = fs::read_dir(&scratch.0)
            .expect("the picture's folder")
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "png"))
            .expect("one picture");
        let image = image::open(written).expect("the picture reads back");
        let image = image.to_rgba8();
        let scale = image.width() as f32 / size.width;
        (image, scale, found)
    }

    fn pixel(image: &image::RgbaImage, scale: f32, x: f32, y: f32) -> [u8; 4] {
        image.get_pixel((x * scale) as u32, (y * scale) as u32).0
    }

    /// The colour of the classic panel's row whose name is drawn at `name`:
    /// its name cell's, just before the name.
    fn row_ground(image: &image::RgbaImage, scale: f32, name: iced::Rectangle) -> [u8; 4] {
        pixel(image, scale, name.x - 2.0, name.center_y())
    }

    /// The colour of the box of the shell editor's row whose label is drawn
    /// at `label`. The box is 6 below the label and its text 10 in from its
    /// edge, which the label starts at.
    fn field_ground(image: &image::RgbaImage, scale: f32, label: iced::Rectangle) -> [u8; 4] {
        let top = label.y + label.height + 6.0;
        pixel(image, scale, label.x + 4.0, top + 8.0)
    }

    /// How far `color` is from `to`, summed over red, green and blue.
    fn distance(color: [u8; 4], to: iced::Color) -> u32 {
        let to = to.into_rgba8();
        (0..3).map(|c| u32::from(color[c].abs_diff(to[c]))).sum()
    }

    /// §3.6: the classic Properties panel draws the rows a clicked result
    /// names on the warning colour, each until it is filled in, and the
    /// others as before; for a rule reading the tag, the tag's row.
    #[test]
    fn pid_verify_highlight_is_drawn_on_the_classic_properties_panel() {
        // Building and drawing the real view is deeper than a default test
        // thread's stack, and two headless renderers made at once crash the
        // process.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = highlight_sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                let warning = app.active_theme.palette().warning.base.color;
                let state = panel(&mut app, "");
                let (missing, ..) = row_naming(&state, "DR-010", "EQUIPMENT|");
                let (tag, ..) = row_naming(&state, "DR-204", "EQUIPMENT|");
                let answer = act(&mut app, &format!("locate={missing}"));
                assert_eq!(answer["ok"], true, "{answer}");
                // The Properties panel gets the edge to itself.
                let _ = run(&mut app, "PIDVERIFY OFF");
                assert!(app.dock_panel_visible(PanelId::Properties));
                let source = crate::t!("Tag source");
                let rows = ["铭牌号", "出厂编号", source.as_ref()];
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let ground = |at: Option<iced::Rectangle>| {
                    row_ground(&image, scale, at.expect("the row is drawn"))
                };
                let (plate, serial, plain) = (ground(at[0]), ground(at[1]), ground(at[2]));
                assert_eq!(plate, serial);
                assert!(
                    distance(plate, warning) + 30 < distance(plain, warning),
                    "{plate:?} is not on the warning colour beside {plain:?}"
                );

                let answer = fill(&mut app, "pid_props_铭牌号", "NP-0001");
                assert_eq!(answer["ok"], true, "{answer}");
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let ground = |at: Option<iced::Rectangle>| {
                    row_ground(&image, scale, at.expect("the row is drawn"))
                };
                let drawn = [ground(at[0]), ground(at[1]), ground(at[2])];
                assert_eq!(drawn, [plain, plate, plain]);

                let _ = run(&mut app, "PIDVERIFY");
                let answer = act(&mut app, &format!("locate={tag}"));
                assert_eq!(answer["ok"], true, "{answer}");
                let _ = run(&mut app, "PIDVERIFY OFF");
                let item_tag = crate::t!("Item tag");
                let rows = [item_tag.as_ref(), "铭牌号", source.as_ref()];
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let ground = |at: Option<iced::Rectangle>| {
                    row_ground(&image, scale, at.expect("the row is drawn"))
                };
                let drawn = [ground(at[0]), ground(at[1]), ground(at[2])];
                assert_eq!(drawn, [plate, plain, plain]);
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// §3.6: the shell's property editor gives each property a clicked
    /// result names that its object has no value for an empty row, and draws
    /// the box of each row the result names on the warning colours, until it
    /// is filled in; for a rule reading the tag, the tag's box.
    #[test]
    fn pid_verify_highlight_is_drawn_in_the_shells_property_editor() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                use crate::app::pid_shell::PidScreen;
                use crate::ui::style::pid_tokens::ColorToken;
                let scratch = Scratch::new();
                let mut app = highlight_sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                assert!(app.show_pid_screen(PidScreen::Editor));
                let amber = ColorToken::Amber.color(app.pid_shell.theme);
                let state = panel(&mut app, "");
                let (missing, ..) = row_naming(&state, "DR-010", "EQUIPMENT|");
                let (tag, ..) = row_naming(&state, "DR-204", "EQUIPMENT|");
                let answer = act(&mut app, &format!("locate={missing}"));
                assert_eq!(answer["ok"], true, "{answer}");
                let rows = ["铭牌号", "出厂编号", "名称"];
                let (image, scale, at) = drawn_pixels(&app, &rows);
                assert!(
                    at.iter().all(Option::is_some),
                    "the editor has a row for each property the result names: {at:?}"
                );
                let ground = |at: Option<iced::Rectangle>| {
                    field_ground(&image, scale, at.expect("the row is drawn"))
                };
                let (plate, serial, plain) = (ground(at[0]), ground(at[1]), ground(at[2]));
                assert_eq!(plate, serial);
                assert!(
                    distance(plate, amber) + 30 < distance(plain, amber),
                    "{plate:?} is not on the warning colours beside {plain:?}"
                );

                let answer = fill(&mut app, "pid_props_铭牌号", "NP-0001");
                assert_eq!(answer["ok"], true, "{answer}");
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let ground = |at: Option<iced::Rectangle>| {
                    field_ground(&image, scale, at.expect("the row is drawn"))
                };
                let drawn = [ground(at[0]), ground(at[1]), ground(at[2])];
                assert_eq!(drawn, [plain, plate, plain]);

                let answer = act(&mut app, &format!("locate={tag}"));
                assert_eq!(answer["ok"], true, "{answer}");
                let rows = ["位号", "铭牌号", "名称"];
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let ground = |at: Option<iced::Rectangle>| {
                    field_ground(&image, scale, at.expect("the row is drawn"))
                };
                let drawn = [ground(at[0]), ground(at[1]), ground(at[2])];
                assert_eq!(drawn, [plate, plain, plain]);
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// §3.6: the property table looks the highlight up as it draws. Moved to
    /// an object on another drawing, the rows it lit on the first are drawn
    /// as any other once that drawing is in front again, though its table
    /// was not built again.
    #[test]
    fn pid_verify_highlight_is_not_drawn_on_a_drawing_it_has_left() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = two_sheets(&scratch);
                app.win_size = (1600.0, 900.0);
                let first = app.active_tab;
                let warning = app.active_theme.palette().warning.base.color;
                let answer = act(&mut app, "check=project");
                assert_eq!(answer["ok"], true, "{answer}");
                let state = panel(&mut app, "");
                let (there, _) = row_of(&state, "DR-202", "PID-1102");
                let (here, _) = row_of(&state, "DR-202", "PID-1101");
                let answer = act(&mut app, &format!("locate={there}"));
                assert_eq!(answer["ok"], true, "{answer}");
                let other = app.active_tab;
                assert_ne!(other, first);
                let _ = run(&mut app, "PIDVERIFY OFF");
                let source = crate::t!("Tag source");
                let rows = ["流量", source.as_ref()];
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let flow = row_ground(&image, scale, at[0].expect("the row of 流量"));
                let plain = row_ground(&image, scale, at[1].expect("the tag source's row"));
                assert!(
                    distance(flow, warning) + 30 < distance(plain, warning),
                    "{flow:?} is not on the warning colour beside {plain:?}"
                );

                let _ = app.update(Message::TabSwitch(first));
                let _ = run(&mut app, "PIDVERIFY");
                let answer = act(&mut app, &format!("locate={here}"));
                assert_eq!(answer["ok"], true, "{answer}");
                assert_eq!(app.active_tab, first);
                let _ = run(&mut app, "PIDVERIFY OFF");
                let _ = app.update(Message::TabSwitch(other));
                let (image, scale, at) = drawn_pixels(&app, &rows);
                let flow = at[0].expect("the row of 流量, as the table was built");
                let plain = row_ground(&image, scale, at[1].expect("the tag source's row"));
                assert_eq!(row_ground(&image, scale, flow), plain);
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    #[test]
    fn waiving_a_row_in_the_panel_moves_it_to_the_waived_group_without_checking_again() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        let checks = app.pid_checks.last_serial();
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        // No reason: the waiver's entry refuses it, said in the message bar,
        // and the reason box stays open.
        let answer = act(&mut app, &format!("waive={note}:"));
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        assert_eq!(state["notice"]["code"], "WAIVER_REASON_REQUIRED", "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        assert_eq!(drawn_reason(&app, note), Some(String::new()));
        // With one: waived at once, counted, nothing checked again.
        let answer = act(&mut app, &format!("waive={note}:放空管"));
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"], Value::Null, "{state}");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 0, "已豁免": 1}),
            "{state}"
        );
        assert_eq!(state["check"]["stale"], false, "{state}");
        assert_eq!(rows(&state), ["DR-201", "DR-202"]);
        assert_eq!(app.pid_checks.last_serial(), checks);
        assert_eq!(drawn_reason(&app, note), None);
        let _ = act(&mut app, "show_waived=1");
        assert_eq!(drawn_waived(&app), [note]);
        // One undo step takes the waiver off the drawing.
        let i = app.active_tab;
        assert_eq!(drc::waivers(&app.tabs[i].scene.document).len(), 1);
        let _ = app.automation_op(r#"{"op":"undo"}"#);
        assert!(drc::waivers(&app.tabs[i].scene.document).is_empty());
    }

    #[test]
    fn unwaiving_a_row_in_the_panel_takes_the_waiver_off_the_drawing() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let _ = act(&mut app, "show_waived=1");
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        let answer = act(&mut app, &format!("unwaive={note}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(drc::waivers(&app.tabs[app.active_tab].scene.document).is_empty());
        let state = panel(&mut app, "");
        assert_eq!(
            state["check"]["counts"],
            json!({"错误": 1, "警告": 1, "提示": 1, "已豁免": 0}),
            "{state}"
        );
        assert_eq!(state["check"]["stale"], false, "{state}");
        let answer = act(&mut app, &format!("unwaive={note}"));
        assert_eq!(answer["reason"], "not_waived", "{answer}");
    }

    #[test]
    fn the_panel_refuses_a_grey_waive_button_with_the_reason() {
        let scratch = Scratch::new();
        let mut app = two_sheets(&scratch);
        typed(&mut app, "PIDCHECK");
        waive_the_note(&mut app);
        let _ = act(&mut app, "show_waived=1");
        let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
        let answer = act(&mut app, &format!("waive={note}:放空管"));
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "already_waived", "{answer}");
        let _ = act(&mut app, "check=project");
        let (pump, _) = row_of(&panel(&mut app, ""), "DR-202", "PID-1102");
        let answer = act(&mut app, &format!("waive={pump}:备用泵"));
        assert_eq!(answer["reason"], "not_on_this_drawing", "{answer}");
        // A session that may not write waives nothing.
        app.collab.readonly = true;
        let state = panel(&mut app, "");
        let listed = state["check"]["rows"].as_array().expect("the rows");
        assert!(
            listed.iter().all(|row| row["can_waive"] == false),
            "{state}"
        );
        let (line, _) = row_of(&state, "DR-201", "PID-1101");
        let answer = act(&mut app, &format!("waive={line}:临时线"));
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "readonly", "{answer}");
        let i = app.active_tab;
        assert_eq!(drc::waivers(&app.tabs[i].scene.document).len(), 1);
    }

    #[test]
    fn a_waiver_no_result_matches_is_listed_and_dropped_from_the_panel() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let i = app.active_tab;
        let gone = drc::Waiver {
            rule_id: "DR-203".to_owned(),
            object_key: "管线|L-0999".to_owned(),
            reason: "已拆除".to_owned(),
            by: "张工".to_owned(),
            at: 1_794_700_000,
        };
        drc::set_waivers(&mut app.tabs[i].scene.document, std::slice::from_ref(&gone));
        typed(&mut app, "PIDCHECK");
        assert_eq!(panel(&mut app, "")["check"]["stale_waivers"], 1);
        let _ = app.update(Message::PidVerify(PidVerifyMsg::StaleWaivers));
        let shown = app.pid_verify_view.check.shown.clone();
        let shown = shown.expect("the check drawn");
        assert!(shown.stale_open);
        let listed: Vec<(&str, bool)> = shown
            .stale_waivers
            .iter()
            .map(|waiver| (waiver.object_key.as_str(), waiver.can_drop))
            .collect();
        assert_eq!(listed, [("管线|L-0999", true)]);
        let steps = undo_steps(&app);
        let _ = app.update(Message::PidVerify(PidVerifyMsg::DropWaiver {
            rule_id: gone.rule_id.clone(),
            object_key: gone.object_key.clone(),
        }));
        assert!(drc::waivers(&app.tabs[i].scene.document).is_empty());
        assert_eq!(undo_steps(&app), steps + 1);
        let state = panel(&mut app, "");
        assert_eq!(state["check"]["stale_waivers"], 0, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
    }

    #[test]
    fn the_drawn_rule_check_tab_draws_the_reason_box_and_the_stale_waivers() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                let i = app.active_tab;
                let gone = drc::Waiver {
                    rule_id: "DR-203".to_owned(),
                    object_key: "管线|L-0999".to_owned(),
                    reason: "已拆除".to_owned(),
                    by: "张工".to_owned(),
                    at: 1_794_700_000,
                };
                drc::set_waivers(&mut app.tabs[i].scene.document, &[gone]);
                typed(&mut app, "PIDCHECK");
                let (note, _) = row_of(&panel(&mut app, ""), "DR-203", "PID-1101");
                for msg in [
                    PidVerifyMsg::StaleWaivers,
                    PidVerifyMsg::Locate(note),
                    PidVerifyMsg::AskWaiver(note),
                    PidVerifyMsg::WaiverReason("放空".to_owned()),
                ] {
                    let task = app.update(Message::PidVerify(msg));
                    app.drive_headless_task(task).expect("the press's task");
                }
                assert_eq!(drawn_reason(&app, note), Some("放空".to_owned()));
                let drawn = drawn_ids(&app);
                let shown = app.pid_verify_view.check.shown.clone();
                let shown = shown.expect("the check drawn");
                assert!(shown.stale_open);
                for row in &shown.rows {
                    assert!(drawn.contains(&row.bounds_id()), "{drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    #[test]
    fn the_drawn_rule_check_tab_reports_its_buttons_and_rows() {
        // As for the panel's body and tabs above.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                typed(&mut app, "PIDCHECK");
                let drawn = drawn_ids(&app);
                let shown = app
                    .pid_verify_view
                    .check
                    .shown
                    .clone()
                    .expect("the check drawn");
                assert_eq!(shown.rows.len(), 3);
                let mut ids = vec![CHECK_DRAWING_ID.to_owned(), CHECK_PROJECT_ID.to_owned()];
                ids.extend(shown.rows.iter().map(RowView::bounds_id));
                for id in ids {
                    assert!(drawn.contains(&id), "{id}: {drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// Save the drawing in front as `path` and have it read from there.
    fn save_as(app: &mut OpenCADStudio, path: &Path) {
        let bytes = crate::io::save_to_bytes(
            app.active_document_for_test(),
            "dwg",
            document::DxfVersion::AC1032,
        )
        .expect("the drawing as DWG");
        fs::write(path, bytes).expect("the drawing");
        let i = app.active_tab;
        app.tabs[i].scene.document.source_path = Some(path.to_string_lossy().into_owned());
    }

    /// The plant the material-list tests list, saved as PID-1101.dwg in
    /// `scratch` with the panel open: a pump with a gate valve beside it, a
    /// vessel, a pipe line of its own, a run from the pump to the vessel
    /// with a gate valve and a fitting of no tag of its own on it, and a
    /// transmitter that gets no tag.
    fn plant(scratch: &Scratch) -> OpenCADStudio {
        let mut app = fresh();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDINSERT gate_valve", &[[-28.0, 0.0]]);
        draw(&mut app, "PIDINSERT vessel", &[[200.0, 12.0]]);
        draw(&mut app, "PIDPIPE", &[[200.0, 100.0], [300.0, 100.0]]);
        draw(&mut app, "PIDPIPE run", &[[14.0, 0.0], [200.0, 0.0]]);
        draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0]]);
        draw(&mut app, "PIDFITTING transmitter", &[[40.0, 0.0]]);
        draw(&mut app, "PIDINSERT transmitter", &[[58.0, 80.0]]);
        save_as(&mut app, &scratch.path("PID-1101.dwg"));
        let _ = run(&mut app, "PIDVERIFY");
        app
    }

    /// [`sheet`], with PID-1102.dwg beside it holding a pump tagged `tag`.
    fn beside(scratch: &Scratch, tag: &str) -> OpenCADStudio {
        let mut other = fresh();
        draw(&mut other, "PIDINSERT pump", &[[0.0, 0.0]]);
        let ran = run(&mut other, &format!("PIDTAG P-1001 {tag}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        save_as(&mut other, &scratch.path("PID-1102.dwg"));
        sheet(scratch)
    }

    /// The rows of the material list the panel state lists.
    fn bom_rows(state: &Value) -> Vec<Value> {
        state["bom"]["rows"].as_array().cloned().unwrap_or_default()
    }

    /// The index and the tag of the first row of the material list in the
    /// panel state that `pick` picks.
    fn bom_row_of(state: &Value, pick: impl Fn(&Value) -> bool) -> (usize, String) {
        let rows = bom_rows(state);
        let row = rows
            .iter()
            .find(|row| pick(row))
            .unwrap_or_else(|| panic!("no such row: {state}"));
        let index = row["index"].as_u64().expect("the index");
        (
            usize::try_from(index).expect("an index"),
            row["tag"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// What `PIDLOCATE words` selects on the drawing in front and where it
    /// looks then; afterwards nothing is selected, the view looks away and
    /// the command line is cleared.
    fn pidlocate(app: &mut OpenCADStudio, words: &str) -> (Vec<String>, glam::DVec3) {
        let ran = run(app, &format!("PIDLOCATE {words}"));
        assert_eq!(ran.error, None, "{}", ran.said);
        let found = (selection(app), looking_at(app));
        assert!(!found.0.is_empty(), "{}", ran.said);
        look_away(app);
        app.command_line.history.clear();
        found
    }

    /// Press `pressed` in the panel as a click does, driving whatever it
    /// starts to its end.
    fn click(app: &mut OpenCADStudio, pressed: PidVerifyMsg) {
        let task = app.update(pressed.message());
        app.drive_headless_task(task).expect("the click's task");
    }

    /// The table of the material list the panel draws now.
    fn drawn_bill(app: &OpenCADStudio) -> BomTable {
        let shown = app.pid_verify_view.bom.shown.clone();
        shown.expect("a bill drawn").table.expect("a table")
    }

    #[test]
    fn the_material_list_tab_lists_what_pid_bom_answers_and_searches_it() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let steps = undo_steps(&app);
        let answer = act(&mut app, "bom=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(undo_steps(&app), steps, "a bill changes no drawing");
        let listed = request(&mut app, json!({"op": "pid_bom"}));
        assert_eq!(listed["ok"], true, "{listed}");
        let rows: Vec<Value> = listed["rows"]
            .as_array()
            .expect("the rows")
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let mut row = row.clone();
                row["index"] = json!(index);
                row
            })
            .collect();
        let state = panel(&mut app, "");
        let bom = &state["bom"];
        assert_eq!(bom["has_result"], true, "{state}");
        assert_eq!(bom["stale"], false, "{state}");
        assert_eq!(bom["scope"], "drawing", "{state}");
        assert_eq!(bom["error"], Value::Null, "{state}");
        assert_eq!(bom["total"], rows.len(), "{state}");
        assert_eq!(bom_rows(&state), rows, "{state}");
        // Only the rows with P-10 in their tag, name, spec or type code.
        let answer = act(&mut app, "bom_search=P-10");
        assert_eq!(answer["ok"], true, "{answer}");
        let has = |row: &&Value| {
            ["tag", "name", "spec", "type_code"].iter().any(|column| {
                let text = row[*column].as_str().unwrap_or_default();
                text.to_lowercase().contains("p-10")
            })
        };
        let wanted: Vec<Value> = rows.iter().filter(has).cloned().collect();
        assert!(!wanted.is_empty() && wanted.len() < rows.len(), "{listed}");
        let state = panel(&mut app, "");
        assert_eq!(state["bom"]["search"], "P-10", "{state}");
        assert_eq!(state["bom"]["total"], wanted.len(), "{state}");
        assert_eq!(bom_rows(&state), wanted, "{state}");
        // An edit leaves the bill shown, marked stale.
        draw(&mut app, "PIDINSERT pump", &[[300.0, 0.0]]);
        let state = panel(&mut app, "");
        assert_eq!(state["bom"]["has_result"], true, "{state}");
        assert_eq!(state["bom"]["stale"], true, "{state}");
    }

    #[test]
    fn the_panel_exports_the_bill_shown_as_pidbom_export_writes_it() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let answer = act(&mut app, "bom_export=early.csv");
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "no_result", "{answer}");
        let _ = act(&mut app, "bom=drawing");
        // Searched and folded, the whole bill is written all the same.
        let _ = act(&mut app, "bom_search=P-10");
        let _ = act(&mut app, "bom_group=设备");
        let answer = act(&mut app, "bom_export=panel.csv");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        let said = state["notice"]["text"].as_str().unwrap_or_default();
        assert!(said.contains("panel.csv"), "{state}");
        let ran = run(&mut app, "PIDBOM EXPORT command.csv");
        assert_eq!(ran.error, None, "{}", ran.said);
        let written = |name: &str| fs::read(scratch.path(name)).expect("the file written");
        assert_eq!(written("panel.csv"), written("command.csv"));
        assert_eq!(
            written("panel-pipe-summary.csv"),
            written("command-pipe-summary.csv")
        );
        // What the export refuses is said in the message bar.
        app.command_line.last_error = None;
        let answer = act(&mut app, "bom_export=panel.txt");
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.command_line.last_error, None);
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        assert_eq!(state["notice"]["code"], "BOM_FORMAT_UNSUPPORTED", "{state}");
        assert!(!scratch.path("panel.txt").exists());
    }

    #[test]
    fn a_tag_found_twice_shows_no_table_and_lists_where_it_is() {
        let scratch = Scratch::new();
        let mut app = two_sheets(&scratch);
        let tabs = app.tabs.len();
        app.command_line.history.clear();
        let answer = act(&mut app, "bom=project");
        assert_eq!(answer["ok"], true, "{answer}");
        // The command line says why, as `PIDBOM PROJECT` does, but not as an
        // error, which would fail the panel's action.
        let history = &app.command_line.history;
        let said: Vec<&str> = history.iter().map(|entry| entry.text.as_str()).collect();
        assert!(
            said.iter().any(|line| line.contains("BOM_DUPLICATE_TAG")),
            "{said:?}"
        );
        assert!(
            said.iter().any(|line| line.contains("EQUIPMENT P-1001")),
            "{said:?}"
        );
        let error = crate::ui::command_line::EntryKind::Error;
        assert!(history.iter().all(|entry| entry.kind != error), "{said:?}");
        let state = panel(&mut app, "");
        let bom = &state["bom"];
        assert_eq!(bom["has_result"], true, "{state}");
        assert_eq!(bom["scope"], "project", "{state}");
        assert_eq!(bom["error"]["code"], "BOM_DUPLICATE_TAG", "{state}");
        assert_eq!(bom["total"], 0, "{state}");
        assert_eq!(bom_rows(&state), Vec::<Value>::new(), "{state}");
        assert_eq!(state["buttons"]["bom_export"], false, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        let duplicates = bom["error"]["duplicates"]
            .as_array()
            .expect("the duplicates");
        assert_eq!(duplicates.len(), 1, "{state}");
        let places = duplicates[0]["places"].as_array().expect("the places");
        let drawings: Vec<&str> = places
            .iter()
            .map(|place| place["drawing_id"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(drawings, ["PID-1101", "PID-1102"], "{state}");
        let handle = places[1]["handle"].as_str().unwrap_or_default().to_owned();
        let answer = act(&mut app, "bom_export=repeated.csv");
        assert_eq!(answer["reason"], "no_result", "{answer}");
        // The place on PID-1102, clicked, opens it and selects the pump
        // there; the project's bill is still the one shown.
        click(&mut app, PidVerifyMsg::BomDuplicateLocate(0, 1));
        assert_eq!(app.tabs.len(), tabs + 1);
        let other = scratch.path("PID-1102.dwg");
        assert_eq!(app.tab_showing(&other), Some(app.active_tab));
        assert_eq!(selection(&mut app), [handle]);
        let state = panel(&mut app, "");
        assert_eq!(
            state["bom"]["error"]["code"], "BOM_DUPLICATE_TAG",
            "{state}"
        );
        assert_eq!(state["notice"], Value::Null, "{state}");
    }

    #[test]
    fn locating_a_row_of_the_material_list_selects_its_object_and_a_fitting_row_its_run() {
        let scratch = Scratch::new();
        let mut app = plant(&scratch);
        let answer = act(&mut app, "bom=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        let pump = bom_row_of(&state, |row| row["tag"] == "P-1001");
        let line = bom_row_of(&state, |row| row["type_code"] == "PIPE_LINE");
        // A fitting with no tag of its own is listed by its run's tag.
        let fittings = bom_row_of(&state, |row| row["type_code"] == "FITTING");
        for (index, tag) in [pump, line, fittings] {
            let (selected, framed) = pidlocate(&mut app, &tag);
            let answer = act(&mut app, &format!("bom_locate={index}"));
            assert_eq!(answer["ok"], true, "{answer}");
            assert_eq!(selection(&mut app), selected, "{tag}");
            let looking = looking_at(&app);
            assert!(
                looking.distance(framed) < 1e-6,
                "{tag}: {looking} ≠ {framed}"
            );
            assert_eq!(drawn_bill(&app).located, Some(index), "{tag}");
            assert_eq!(panel(&mut app, "")["notice"], Value::Null, "{tag}");
            assert!(!pidlocate_said(&app), "{tag}");
            look_away(&mut app);
        }
        // An object without a tag is shown where it is, nothing selected.
        let skipped = drawn_bill(&app).skipped;
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        click(&mut app, PidVerifyMsg::BomSkippedLocate(0));
        assert_eq!(selection(&mut app), Vec::<String>::new());
        let looking = looking_at(&app);
        let at = glam::DVec3::new(skipped[0].at[0], skipped[0].at[1], 0.0);
        assert!(looking.distance(at) < 1e-6, "{looking} ≠ {at}");
        let answer = act(&mut app, "bom_locate=99");
        assert_eq!(answer["code"], "PANEL_ROW_NOT_FOUND", "{answer}");
    }

    #[test]
    fn the_project_bill_pressed_in_the_panel_is_kept_for_the_folder_until_a_drawing_changes() {
        let scratch = Scratch::new();
        let mut app = beside(&scratch, "P-2001");
        let first = app.active_tab;
        let tabs = app.tabs.len();
        let answer = act(&mut app, "bom=project");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["bom"]["scope"], "project", "{state}");
        assert_eq!(state["bom"]["error"], Value::Null, "{state}");
        assert_eq!(state["bom"]["stale"], false, "{state}");
        assert_eq!(state["busy"], Value::Null, "{state}");
        let (other, _) = bom_row_of(&state, |row| row["tag"] == "P-2001");
        assert_eq!(bom_rows(&state)[other]["drawings"], json!(["PID-1102"]));
        // A row on the other drawing opens it; the bill stays.
        let answer = act(&mut app, &format!("bom_locate={other}"));
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(app.tabs.len(), tabs + 1);
        let path = scratch.path("PID-1102.dwg");
        assert_eq!(app.tab_showing(&path), Some(app.active_tab));
        assert_eq!(selection(&mut app).len(), 1);
        let state = panel(&mut app, "");
        assert_eq!(state["drawing_id"], "PID-1102", "{state}");
        assert_eq!(state["bom"]["scope"], "project", "{state}");
        assert_eq!(drawn_bill(&app).located, Some(other));
        // The drawing's own bill made later is the one shown, until the
        // project's is made again.
        let _ = app.update(Message::TabSwitch(first));
        let _ = act(&mut app, "bom=drawing");
        assert_eq!(panel(&mut app, "")["bom"]["scope"], "drawing");
        let _ = act(&mut app, "bom=project");
        assert_eq!(panel(&mut app, "")["bom"]["scope"], "project");
        // An edit to a drawing of the folder makes it stale.
        draw(&mut app, "PIDINSERT pump", &[[300.0, 300.0]]);
        let state = panel(&mut app, "");
        assert_eq!(state["bom"]["scope"], "project", "{state}");
        assert_eq!(state["bom"]["stale"], true, "{state}");
    }

    #[test]
    fn the_panel_refuses_a_grey_material_list_button_with_the_reason() {
        let mut app = fresh();
        let answer = act(&mut app, "bom=project");
        assert_eq!(answer["code"], "PANEL_ACTION_UNAVAILABLE", "{answer}");
        assert_eq!(answer["reason"], "no_project_dir", "{answer}");
        for value in [
            "bom_search=P",
            "bom_group=设备",
            "bom_locate=0",
            "bom_export=a.csv",
            "more=bom",
        ] {
            let answer = act(&mut app, value);
            assert_eq!(
                answer["code"], "PANEL_ACTION_UNAVAILABLE",
                "{value}: {answer}"
            );
            assert_eq!(answer["reason"], "no_result", "{value}: {answer}");
        }
        // A drawing never saved is listed all the same.
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        let answer = act(&mut app, "bom=drawing");
        assert_eq!(answer["ok"], true, "{answer}");
        let state = panel(&mut app, "");
        assert_eq!(state["bom"]["has_result"], true, "{state}");
        assert_eq!(state["bom"]["total"], 1, "{state}");
        assert_eq!(state["buttons"]["bom_project"], false, "{state}");
    }

    #[test]
    fn the_drawn_material_list_tab_reports_its_buttons_and_rows() {
        // As for the panel's body and tabs above.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                let _ = act(&mut app, "tab=bom");
                let _ = act(&mut app, "bom=drawing");
                let drawn = drawn_ids(&app);
                let table = drawn_bill(&app);
                assert_eq!(table.rows.len(), 2);
                let mut ids: Vec<String> =
                    [BOM_DRAWING_ID, BOM_PROJECT_ID, BOM_SEARCH_ID, BOM_EXPORT_ID]
                        .map(str::to_owned)
                        .to_vec();
                ids.extend(table.rows.iter().map(BomRowView::bounds_id));
                for id in ids {
                    assert!(drawn.contains(&id), "{id}: {drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// Press `value` in the panel, which takes it.
    fn pressed(app: &mut OpenCADStudio, value: &str) {
        let answer = act(app, value);
        assert_eq!(answer["ok"], true, "{value}: {answer}");
    }

    /// Press `value` in the panel, which refuses it as a grey button for
    /// `reason`.
    fn refused_for(app: &mut OpenCADStudio, value: &str, reason: &str) {
        let answer = act(app, value);
        assert_eq!(
            answer["code"], "PANEL_ACTION_UNAVAILABLE",
            "{value}: {answer}"
        );
        assert_eq!(answer["reason"], reason, "{value}: {answer}");
    }

    /// The numbers of the snapshots the panel state lists, in its order.
    fn listed_seqs(state: &Value) -> Vec<u64> {
        let rows = state["deliver"]["snapshots"].as_array();
        let rows = rows.cloned().unwrap_or_default();
        rows.iter().filter_map(|row| row["seq"].as_u64()).collect()
    }

    #[test]
    fn a_snapshot_saved_in_the_panel_is_listed_and_two_ticked_are_compared() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        pressed(&mut app, "tab=deliver");
        let state = panel(&mut app, "");
        assert_eq!(listed_seqs(&state), Vec::<u64>::new(), "{state}");
        assert_eq!(state["buttons"]["snapshot_save"], true, "{state}");
        let steps = undo_steps(&app);
        pressed(&mut app, "snapshot_save=送审前");
        assert_eq!(undo_steps(&app), steps, "a snapshot changes no drawing");
        let state = panel(&mut app, "");
        assert_eq!(listed_seqs(&state), [1], "{state}");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        // The row is what pid_snapshots lists, ticked or not.
        let listed = request(&mut app, json!({"op": "pid_snapshots"}));
        let mut row = listed["snapshots"][0].clone();
        assert_eq!(row["reason"], "送审前", "{listed}");
        row["picked"] = json!(false);
        assert_eq!(state["deliver"]["snapshots"][0], row, "{state}");
        // 【比对】 wants two rows ticked: not none, one or three.
        assert_eq!(state["buttons"]["diff"], false, "{state}");
        refused_for(&mut app, "diff", "not_two_picked");
        pressed(&mut app, "pick=1");
        refused_for(&mut app, "diff", "not_two_picked");
        pressed(&mut app, "snapshot_save=");
        pressed(&mut app, "pick=2");
        let state = panel(&mut app, "");
        assert_eq!(listed_seqs(&state), [2, 1], "{state}");
        assert_eq!(state["deliver"]["picked"], json!([1, 2]), "{state}");
        assert_eq!(state["buttons"]["diff"], true, "{state}");
        pressed(&mut app, "pick=live");
        let state = panel(&mut app, "");
        assert_eq!(state["buttons"]["diff"], false, "{state}");
        refused_for(&mut app, "diff", "not_two_picked");
        pressed(&mut app, "pick=live");
        // Two snapshots of a drawing that did not change are alike.
        pressed(&mut app, "diff");
        let state = panel(&mut app, "");
        let alike = json!({"a": 1, "b": 2, "identical": true, "新增": 0, "删除": 0, "修改": 0});
        assert_eq!(state["deliver"]["diff"], alike, "{state}");
    }

    #[test]
    fn a_snapshot_against_the_drawing_as_it_is_counts_as_pid_diff_and_finds_what_changed() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        pressed(&mut app, "tab=deliver");
        pressed(&mut app, "snapshot_save=送审前");
        // The pump gets a design temperature, and a transmitter comes in.
        let ran = run(&mut app, "PIDSETPROP P-1001 设计温度 80");
        assert_eq!(ran.error, None, "{}", ran.said);
        draw(&mut app, "PIDINSERT transmitter", &[[300.0, 300.0]]);
        pressed(&mut app, "pick=live");
        pressed(&mut app, "pick=1");
        pressed(&mut app, "diff");
        let state = panel(&mut app, "");
        let compared = request(&mut app, json!({"op": "pid_diff", "a": 1}));
        assert_eq!(compared["ok"], true, "{compared}");
        let summary = &compared["summary"];
        let counts = ["新增", "删除", "修改"].map(|group| summary[group].as_u64());
        assert_eq!(counts, [Some(1), Some(0), Some(1)], "{compared}");
        let wanted = json!({
            "a": 1, "b": "live", "identical": false,
            "新增": 1, "删除": 0, "修改": 1
        });
        assert_eq!(state["deliver"]["diff"], wanted, "{state}");
        // What changed on the pump reads under it.
        let shown = app
            .pid_verify_view
            .deliver
            .diff
            .clone()
            .expect("a diff drawn");
        let mut groups = shown.groups.iter();
        let modified = groups.find(|group| group.group == DiffGroup::Modified);
        let pump = &modified.expect("the modified group").items[0];
        let said = "属性 设计温度：（空） → 80".to_owned();
        assert!(pump.changes.contains(&said), "{:?}", pump.changes);
        // Clicked, the pump is selected and framed as PIDLOCATE does.
        let found = pidlocate(&mut app, "P-1001");
        pressed(&mut app, "diff_locate=modified:0");
        assert_eq!((selection(&mut app), looking_at(&app)), found);
        let state = panel(&mut app, "");
        assert_eq!(state["notice"], Value::Null, "{state}");
        assert!(!pidlocate_said(&app), "the panel finds in silence");
    }

    #[test]
    fn a_drawing_never_saved_keeps_no_numbered_snapshot_in_the_panel() {
        let mut app = fresh();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        pressed(&mut app, "show");
        pressed(&mut app, "tab=deliver");
        let state = panel(&mut app, "");
        assert_eq!(state["buttons"]["snapshot_save"], false, "{state}");
        assert_eq!(state["deliver"]["snapshots"], json!([]), "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        assert_eq!(app.pid_verify_view.empty, Some(UNSAVED_SAID));
        refused_for(&mut app, "snapshot_save=送审前", "no_project_dir");
        refused_for(&mut app, "pick=live", "no_project_dir");
    }

    #[test]
    fn ticking_otherwise_drops_the_diff_and_another_drawing_in_front_the_ticks() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let first = app.tabs[app.active_tab].id;
        pressed(&mut app, "tab=deliver");
        pressed(&mut app, "snapshot_save=送审前");
        pressed(&mut app, "pick=1");
        pressed(&mut app, "pick=live");
        pressed(&mut app, "diff");
        let state = panel(&mut app, "");
        assert_ne!(state["deliver"]["diff"], Value::Null, "{state}");
        // Unticked, the diff goes; ticked again, it does not come back.
        pressed(&mut app, "pick=live");
        let state = panel(&mut app, "");
        assert_eq!(state["deliver"]["diff"], Value::Null, "{state}");
        assert_eq!(state["buttons"]["diff_export"], false, "{state}");
        pressed(&mut app, "pick=live");
        let state = panel(&mut app, "");
        assert_eq!(state["deliver"]["diff"], Value::Null, "{state}");
        pressed(&mut app, "diff");
        // Another drawing in front: nothing ticked, no diff; nor when the
        // first comes back, its list read again.
        let _ = app.update(Message::TabNew);
        assert_ne!(app.tabs[app.active_tab].id, first);
        let state = panel(&mut app, "");
        assert_eq!(state["deliver"]["picked"], json!([]), "{state}");
        assert_eq!(state["deliver"]["diff"], Value::Null, "{state}");
        activate(&mut app, first, "pid-verify-back-to-the-first");
        let state = panel(&mut app, "");
        assert_eq!(state["deliver"]["picked"], json!([]), "{state}");
        assert_eq!(state["deliver"]["diff"], Value::Null, "{state}");
        assert_eq!(listed_seqs(&state), [1], "{state}");
        // Another drawing in the same tab is another drawing too: the ticks
        // go, not only out of sight.
        pressed(&mut app, "pick=1");
        let _ = app.automation_op(r#"{"op":"new"}"#);
        let _ = panel(&mut app, "");
        assert_eq!(app.pid_verify_ui.picked, Vec::<Pick>::new());
    }

    #[test]
    fn the_snapshot_list_is_read_again_on_turning_to_it_and_after_each_save() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        let ran = run(&mut app, "PIDSNAPSHOT SAVE 送审前");
        assert_eq!(ran.error, None, "{}", ran.said);
        pressed(&mut app, "tab=deliver");
        assert_eq!(listed_seqs(&panel(&mut app, "")), [1]);
        let ran = run(&mut app, "PIDSNAPSHOT SAVE 改版");
        assert_eq!(ran.error, None, "{}", ran.said);
        assert_eq!(listed_seqs(&panel(&mut app, "")), [2, 1]);
        let saved = request(
            &mut app,
            json!({"op": "pid_snapshot_save", "reason": "交付"}),
        );
        assert_eq!(saved["ok"], true, "{saved}");
        assert_eq!(listed_seqs(&panel(&mut app, "")), [3, 2, 1]);
        // What is typed in the box is the reason; saved, the box empties.
        click(&mut app, PidVerifyMsg::SnapshotReason("出图".to_owned()));
        assert_eq!(app.pid_verify_view.deliver.reason, "出图");
        click(&mut app, PidVerifyMsg::SnapshotSave("出图".to_owned()));
        assert_eq!(app.pid_verify_view.deliver.reason, "");
        let state = panel(&mut app, "");
        assert_eq!(listed_seqs(&state), [4, 3, 2, 1], "{state}");
        assert_eq!(
            state["deliver"]["snapshots"][0]["reason"], "出图",
            "{state}"
        );
    }

    #[test]
    fn the_panel_exports_the_diff_shown_as_piddiff_export_writes_it() {
        let scratch = Scratch::new();
        let mut app = sheet(&scratch);
        pressed(&mut app, "tab=deliver");
        refused_for(&mut app, "diff_export=early.json", "no_result");
        pressed(&mut app, "snapshot_save=送审前");
        draw(&mut app, "PIDINSERT transmitter", &[[300.0, 300.0]]);
        pressed(&mut app, "snapshot_save=改版");
        pressed(&mut app, "pick=2");
        pressed(&mut app, "pick=1");
        pressed(&mut app, "diff");
        pressed(&mut app, "diff_export=panel.json");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        let ran = run(&mut app, "PIDDIFF 1 2 EXPORT command.json");
        assert_eq!(ran.error, None, "{}", ran.said);
        let written = |name: &str| {
            fs::read(scratch.path(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
        };
        assert_eq!(written("panel.json"), written("command.json"));
    }

    #[test]
    fn the_drawn_snapshot_tab_reports_its_buttons_and_rows() {
        // As for the panel's body and tabs above.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheet(&scratch);
                app.win_size = (1600.0, 900.0);
                for value in ["tab=deliver", "snapshot_save=送审前"] {
                    pressed(&mut app, value);
                }
                draw(&mut app, "PIDINSERT transmitter", &[[300.0, 300.0]]);
                for value in ["pick=1", "pick=live", "diff"] {
                    pressed(&mut app, value);
                }
                let drawn = drawn_ids(&app);
                let mut ids: Vec<String> = [
                    SNAPSHOT_REASON_ID,
                    SNAPSHOT_SAVE_ID,
                    DIFF_ID,
                    DIFF_EXPORT_ID,
                ]
                .map(str::to_owned)
                .to_vec();
                ids.extend([
                    Pick::Live.bounds_id(),
                    Pick::Seq(1).bounds_id(),
                    DiffGroup::Added.item_id(0),
                ]);
                for id in ids {
                    assert!(drawn.contains(&id), "{id}: {drawn:?}");
                }
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// The project rule pack the publish tests publish under: the built-in
    /// sets off and `rules` of the panel tests' own set. A drawing with
    /// errors still saves.
    fn publish_pack(scratch: &Scratch, rules: Vec<Value>) {
        let pack = json!({
            "version": 1,
            "pack": "发布面板",
            "rule_set_version": "T2",
            "save_gate": "warn",
            "sets": [
                {"name": "基础规则", "enabled": false},
                {"name": "工程扩展", "enabled": false},
                {"name": "面板验收", "enabled": true}
            ],
            "rules": rules,
        });
        fs::write(scratch.path(PROJECT_PACK_FILE), pack.to_string()).expect("the pack");
    }

    /// Save the drawing in front as `path` by the save op, as saving in the
    /// program does: the tab keeps the path and the file's fingerprint, and
    /// has no change not saved.
    fn save(app: &mut OpenCADStudio, path: &Path) {
        app.finish_all_pending_history();
        let saved = request(app, json!({"op": "save", "path": path, "overwrite": true}));
        assert_eq!(saved["ok"], true, "{saved}");
        let tab = &app.tabs[app.active_tab];
        assert!(!tab.dirty, "{saved}");
        assert!(tab.disk_fingerprint.is_some(), "{saved}");
    }

    /// A pump and a pipe line saved as PID-1101.dwg in `scratch`, beside
    /// [`publish_pack`] of `rules`, the panel open on 「快照与发布」.
    fn publishable(scratch: &Scratch, rules: Vec<Value>) -> OpenCADStudio {
        let mut app = fresh();
        draw(&mut app, "PIDINSERT pump", &[[0.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 100.0], [100.0, 100.0]]);
        publish_pack(scratch, rules);
        save(&mut app, &scratch.path("PID-1101.dwg"));
        let ran = run(&mut app, "PIDVERIFY DELIVER");
        assert_eq!(ran.error, None, "{}", ran.said);
        app
    }

    /// The publish block of the panel state now.
    fn publish_block(app: &mut OpenCADStudio) -> Value {
        panel(app, "")["deliver"]["publish"].clone()
    }

    /// The question the publish half asks now, if it asks one.
    fn asked(app: &OpenCADStudio) -> Option<String> {
        let publish = app.pid_verify_view.deliver.publish.as_ref();
        publish.and_then(|publish| publish.asking.clone())
    }

    #[test]
    fn a_change_not_saved_greys_publish_and_the_conditions_can_still_be_checked() {
        let scratch = Scratch::new();
        let mut app = publishable(&scratch, Vec::new());
        let state = panel(&mut app, "");
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["state"], "未发布", "{state}");
        assert_eq!(publish["next_revision"], 1, "{state}");
        let target = scratch.path("pid-publish").display().to_string();
        assert_eq!(publish["target_dir"], target.as_str(), "{state}");
        assert_eq!(publish["checked"], false, "{state}");
        assert_eq!(state["buttons"]["publish"], true, "{state}");
        draw(&mut app, "PIDINSERT pump", &[[200.0, 0.0]]);
        let state = panel(&mut app, "");
        assert_eq!(state["buttons"]["publish"], false, "{state}");
        assert_eq!(state["buttons"]["publish_check"], true, "{state}");
        let preconditions = &state["deliver"]["publish"]["preconditions"];
        assert_eq!(preconditions["clean"], false, "{state}");
        refused_for(&mut app, "publish", "precondition");
        pressed(&mut app, "publish_check");
        let state = panel(&mut app, "");
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["checked"], true, "{state}");
        assert_eq!(publish["stale"], false, "{state}");
        assert_eq!(publish["gates"]["drc_errors"], 0, "{state}");
        assert_eq!(publish["gates"]["duplicate_tags"], 0, "{state}");
        assert_eq!(publish["gates"]["index_complete"], true, "{state}");
        // The gates pass, but the drawing has a change not saved.
        assert_eq!(publish["can_publish"], false, "{state}");
        assert_eq!(state["notice"], Value::Null, "{state}");
        // Checking writes nothing.
        assert!(!scratch.path("pid-publish").exists());
        // Edited again, the gates worked out are stale.
        draw(&mut app, "PIDINSERT pump", &[[300.0, 0.0]]);
        assert_eq!(publish_block(&mut app)["stale"], true);
    }

    #[test]
    fn an_error_not_waived_blocks_the_publish_and_its_condition_leads_to_it() {
        let scratch = Scratch::new();
        let error = rule("DR-201", "管线", "错误", "管道等级 = NULL");
        let mut app = publishable(&scratch, vec![error]);
        assert_eq!(panel(&mut app, "")["buttons"]["publish"], true);
        pressed(&mut app, "publish");
        let state = panel(&mut app, "");
        assert_eq!(state["notice"]["code"], "PUBLISH_BLOCKED", "{state}");
        assert_eq!(state["notice"]["level"], "error", "{state}");
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["checked"], true, "{state}");
        assert_eq!(publish["gates"]["drc_errors"], 1, "{state}");
        assert_eq!(publish["can_publish"], false, "{state}");
        assert_eq!(publish["state"], "未发布", "{state}");
        let shown = app.pid_verify_view.deliver.publish.as_ref();
        let conditions = shown.expect("a publish half").conditions();
        let errors = ("没被豁免的错误为 0（1 条）".to_owned(), Some(false));
        assert_eq!(conditions[4], errors, "{conditions:?}");
        // Nothing was written: no target folder, no snapshot.
        assert!(!scratch.path("pid-publish").exists());
        assert_eq!(listed_seqs(&state), Vec::<u64>::new(), "{state}");
        // The errors are what the rule-check tab shows, errors only.
        click(&mut app, PidVerifyMsg::PublishErrors);
        let state = panel(&mut app, "");
        assert_eq!(state["tab"], "check", "{state}");
        let severities = &state["check"]["filters"]["severity"];
        assert_eq!(*severities, json!(["错误"]), "{state}");
        assert_eq!(rows(&state), ["DR-201"], "{state}");
    }

    #[test]
    fn what_passes_is_published_listed_with_its_snapshot_and_followed_as_the_file_changes() {
        let scratch = Scratch::new();
        let mut app = publishable(&scratch, Vec::new());
        let before = listed_seqs(&panel(&mut app, "")).len();
        pressed(&mut app, "publish");
        let state = panel(&mut app, "");
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["state"], "已发布，修订 1", "{state}");
        assert_eq!(publish["next_revision"], 2, "{state}");
        let packages = publish["packages"].as_array().cloned().unwrap_or_default();
        assert_eq!(packages.len(), 1, "{state}");
        assert_eq!(packages[0]["revision"], 1, "{state}");
        assert_eq!(packages[0]["current"], true, "{state}");
        let path = packages[0]["path"].as_str().unwrap_or_default().to_owned();
        assert!(Path::new(&path).is_dir(), "{path}");
        assert_eq!(publish["last_package"], path.as_str(), "{state}");
        assert_eq!(listed_seqs(&state).len(), before + 1, "{state}");
        assert_eq!(state["notice"]["level"], "info", "{state}");
        let said = state["notice"]["text"].as_str().unwrap_or_default();
        assert!(said.starts_with("已发布") && said.contains(&path), "{said}");
        // The package is the one pid_publications lists.
        let listed = request(&mut app, json!({"op": "pid_publications"}));
        assert_eq!(listed["packages"][0]["path"], path.as_str(), "{listed}");
        pressed(&mut app, "package_verify=1");
        let verified = publish_block(&mut app)["packages"][0]["verified"].clone();
        assert_eq!(verified, json!({"intact": true, "text": "完好"}));
        // Changed, then saved: the drawing is no longer what was published.
        draw(&mut app, "PIDINSERT pump", &[[200.0, 0.0]]);
        assert_eq!(publish_block(&mut app)["state"], "修订 1 之后有改动");
        save(&mut app, &scratch.path("PID-1101.dwg"));
        let publish = publish_block(&mut app);
        assert_eq!(publish["state"], "修订 1 之后有改动", "{publish}");
        assert_eq!(publish["packages"][0]["current"], false, "{publish}");
        assert_eq!(publish["preconditions"]["clean"], true, "{publish}");
        assert_eq!(
            publish["preconditions"]["file_unchanged"], true,
            "{publish}"
        );
        // A file of the package gone: verified again, it says which.
        fs::remove_file(Path::new(&path).join("PID-1101.pdf")).expect("the package's PDF");
        pressed(&mut app, "package_verify=1");
        let verified = publish_block(&mut app)["packages"][0]["verified"].clone();
        assert_eq!(verified["intact"], false, "{verified}");
        let text = verified["text"].as_str().unwrap_or_default();
        assert!(text.contains("PID-1101.pdf"), "{verified}");
    }

    #[test]
    fn the_folder_typed_is_where_the_records_are_read_and_the_package_goes() {
        let scratch = Scratch::new();
        let mut app = publishable(&scratch, Vec::new());
        pressed(&mut app, "publish_dir=发布包");
        let publish = publish_block(&mut app);
        let typed = scratch.path("发布包");
        assert_eq!(
            publish["target_dir"],
            typed.display().to_string(),
            "{publish}"
        );
        assert_eq!(publish["state"], "未发布", "{publish}");
        pressed(&mut app, "publish");
        let publish = publish_block(&mut app);
        assert_eq!(publish["state"], "已发布，修订 1", "{publish}");
        let path = publish["packages"][0]["path"].as_str().unwrap_or_default();
        assert!(Path::new(path).starts_with(&typed), "{publish}");
        assert!(!scratch.path("pid-publish").exists());
        // Emptied, the box stands for the default folder again.
        pressed(&mut app, "publish_dir=");
        let publish = publish_block(&mut app);
        let default = scratch.path("pid-publish").display().to_string();
        assert_eq!(publish["target_dir"], default.as_str(), "{publish}");
        assert_eq!(publish["state"], "未发布", "{publish}");
        assert_eq!(publish["packages"], json!([]), "{publish}");
    }

    #[test]
    fn a_publish_typed_is_listed_at_once_and_its_folder_is_the_last_package() {
        let scratch = Scratch::new();
        let mut app = publishable(&scratch, Vec::new());
        assert_eq!(publish_block(&mut app)["state"], "未发布");
        let ran = run(&mut app, "PIDPUBLISH");
        assert_eq!(ran.error, None, "{}", ran.said);
        let publish = publish_block(&mut app);
        assert_eq!(publish["state"], "已发布，修订 1", "{publish}");
        let path = publish["packages"][0]["path"].as_str().unwrap_or_default();
        assert!(Path::new(path).is_dir(), "{publish}");
        assert_eq!(publish["last_package"], path, "{publish}");
    }

    #[test]
    fn a_publish_by_the_op_is_listed_at_once_with_its_snapshot_and_its_folder() {
        let scratch = Scratch::new();
        let mut app = publishable(&scratch, Vec::new());
        let before = listed_seqs(&panel(&mut app, "")).len();
        let ask = json!({"op": "pid_publish"});
        let mut wrote = request(&mut app, ask.clone());
        for _ in 0..5 {
            if wrote["status"] != "scanning" {
                break;
            }
            wrote = request(&mut app, ask.clone());
        }
        assert_eq!(wrote["ok"], true, "{wrote}");
        let path = &wrote["result"]["path"];
        let state = panel(&mut app, "");
        let publish = &state["deliver"]["publish"];
        assert_eq!(publish["state"], "已发布，修订 1", "{state}");
        assert_eq!(publish["packages"][0]["path"], *path, "{state}");
        assert_eq!(publish["last_package"], *path, "{state}");
        assert_eq!(listed_seqs(&state).len(), before + 1, "{state}");
    }

    #[test]
    fn a_tag_repeated_in_the_project_is_listed_under_its_condition_and_found_where_it_is() {
        let scratch = Scratch::new();
        let mut other = fresh();
        draw(&mut other, "PIDINSERT pump", &[[40.0, 60.0]]);
        save_as(&mut other, &scratch.path("PID-1102.dwg"));
        let mut app = publishable(&scratch, Vec::new());
        let tabs = app.tabs.len();
        pressed(&mut app, "publish_check");
        let publish = publish_block(&mut app);
        assert_eq!(publish["gates"]["duplicate_tags"], 1, "{publish}");
        assert_eq!(publish["can_publish"], false, "{publish}");
        let half = || app.pid_verify_view.deliver.publish.clone();
        let found = half().and_then(|half| half.gates).expect("the gates");
        let repeated = found.duplicates[0].clone();
        let mut drawings: Vec<&str> = repeated
            .places
            .iter()
            .map(|place| place.drawing_id.as_str())
            .collect();
        drawings.sort_unstable();
        assert_eq!(drawings, ["PID-1101", "PID-1102"]);
        assert!(!half().is_some_and(|half| half.duplicates_open));
        click(&mut app, PidVerifyMsg::PublishDuplicates);
        let open = app.pid_verify_view.deliver.publish.as_ref();
        assert!(open.is_some_and(|half| half.duplicates_open));
        // The place on PID-1102, clicked, opens it and selects the pump there.
        let there = repeated
            .places
            .iter()
            .position(|place| place.drawing_id == "PID-1102")
            .expect("a place on PID-1102");
        click(&mut app, PidVerifyMsg::PublishDuplicateLocate(0, there));
        assert_eq!(app.tabs.len(), tabs + 1);
        let opened = scratch.path("PID-1102.dwg");
        assert_eq!(app.tab_showing(&opened), Some(app.active_tab));
        assert_eq!(selection(&mut app), [repeated.places[there].handle.clone()]);
    }

    #[test]
    fn the_drawn_publish_half_asks_before_it_publishes_and_reports_its_controls() {
        // As for the panel's body and tabs above.
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = publishable(&scratch, Vec::new());
                app.win_size = (1600.0, 900.0);
                click_drawn(&mut app, "发布");
                let target = scratch.path("pid-publish").display().to_string();
                let question = format!("把 PID-1101 发布为修订 1 到 {target}？");
                assert_eq!(asked(&app), Some(question));
                let drawn = drawn_ids(&app);
                for id in [
                    PUBLISH_DIR_ID,
                    PUBLISH_CHECK_ID,
                    PUBLISH_ID,
                    PUBLISH_CONFIRM_ID,
                ] {
                    assert!(drawn.iter().any(|drawn| drawn == id), "{id}: {drawn:?}");
                }
                // Cancelled, nothing is published.
                click_drawn(&mut app, "取消");
                assert_eq!(asked(&app), None);
                assert!(!scratch.path("pid-publish").exists());
                // Asked again and confirmed, it is.
                click_drawn(&mut app, "发布");
                click_drawn(&mut app, "确认");
                assert_eq!(asked(&app), None);
                let publish = publish_block(&mut app);
                assert_eq!(publish["state"], "已发布，修订 1", "{publish}");
                let drawn = drawn_ids(&app);
                let row = "pid-verify-package-1";
                assert!(drawn.iter().any(|drawn| drawn == row), "{drawn:?}");
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// Copies of PID-1102.dwg under `names` in `scratch`.
    fn copy_pump(scratch: &Scratch, names: &[&str]) {
        let pump = fs::read(scratch.path("PID-1102.dwg")).expect("PID-1102.dwg");
        for name in names {
            fs::write(scratch.path(name), &pump).expect("a copy of PID-1102.dwg");
        }
    }

    /// [`two_sheets`], with PID-1103.dwg and PID-1104.dwg beside them, each
    /// a copy of PID-1102: PID-1101 being open, a project scan reads three
    /// drawings from disk.
    fn sheets(scratch: &Scratch) -> OpenCADStudio {
        let app = two_sheets(scratch);
        copy_pump(scratch, &["PID-1103.dwg", "PID-1104.dwg"]);
        app
    }

    /// Press `value` in the panel through the `action` op, as [`act`] does,
    /// and hold on to the reads of the scan it starts: the test hands them
    /// over one at a time ([`deliver`]). The press answers at once (§3.3).
    fn press_and_hold(
        app: &mut OpenCADStudio,
        value: &str,
    ) -> impl iced::futures::Stream<Item = iced_runtime::Action<Message>> + Unpin {
        static HELD: AtomicU64 = AtomicU64::new(0);
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let (answer, task) = app.control_request(json!({
            "protocol": 1,
            "op": "action",
            "name": "pid_verify",
            "value": value,
            "document_id": state["document_id"],
            "request_id": format!("pid-verify-held-{}", HELD.fetch_add(1, Ordering::Relaxed)),
        }));
        assert_eq!(answer["status"], "completed", "{value}: {answer}");
        iced_runtime::task::into_stream(task).expect("the reads of the scan")
    }

    /// Hand the next read of `reads` to the program, as the interface thread
    /// gets it; `false` once the scan handed back all it had.
    fn deliver(
        app: &mut OpenCADStudio,
        reads: &mut (impl iced::futures::Stream<Item = iced_runtime::Action<Message>> + Unpin),
    ) -> bool {
        let next = iced::futures::StreamExt::next(reads);
        match iced::futures::executor::block_on(next) {
            Some(iced_runtime::Action::Output(message)) => {
                let task = app.update(message);
                app.drive_headless_task(task)
                    .expect("what the read started");
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Whether the real view, drawn at 1600 × 900, shows the text `said`.
    fn drawn_text(app: &OpenCADStudio, said: &str) -> bool {
        let size = iced::Size::new(1600.0, 900.0);
        let mut simulator =
            iced_test::Simulator::with_size(iced::Settings::default(), size, app.view_main());
        simulator.find(said).is_ok()
    }

    /// What the command line said since its history was last cleared.
    fn said(app: &OpenCADStudio) -> String {
        let history = app.command_line.history.iter();
        let lines: Vec<&str> = history.map(|entry| entry.text.as_str()).collect();
        lines.join("\n")
    }

    /// §4.7: while a project check reads the drawings, the panel state and
    /// the progress line say how far it is; what would start more is refused
    /// as busy while the filters and locating work. The last drawing back,
    /// the check is shown and the line is gone.
    #[test]
    fn a_project_check_under_way_shows_how_far_it_is_and_greys_what_would_start_more() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheets(&scratch);
                app.win_size = (1600.0, 900.0);
                typed(&mut app, "PIDCHECK");
                let mut reads = press_and_hold(&mut app, "check=project");
                let state = panel(&mut app, "");
                let reading = json!({
                    "what": "scan",
                    "then": "check",
                    "text": "正在读项目图纸",
                    "done": 0,
                    "total": 3,
                    "cancellable": true
                });
                assert_eq!(state["busy"], reading, "{state}");
                for flag in [
                    "check_drawing",
                    "check_project",
                    "bom_drawing",
                    "bom_project",
                    "snapshot_save",
                    "publish_check",
                    "publish",
                ] {
                    assert_eq!(state["buttons"][flag], false, "{flag}: {state}");
                }
                for value in [
                    "check=drawing",
                    "check=project",
                    "bom=drawing",
                    "bom=project",
                    "snapshot_save=",
                    "publish_check",
                    "publish",
                    "package_verify=1",
                ] {
                    let answer = act(&mut app, value);
                    assert_eq!(
                        answer["code"], "PANEL_ACTION_UNAVAILABLE",
                        "{value}: {answer}"
                    );
                    assert_eq!(answer["reason"], "busy", "{value}: {answer}");
                }
                // Filtering and locating start nothing.
                let filtered = act(&mut app, "severity=错误");
                assert_eq!(filtered["ok"], true, "{filtered}");
                let (index, _) = row_of(&panel(&mut app, ""), "DR-202", "PID-1101");
                let located = act(&mut app, &format!("locate={index}"));
                assert_eq!(located["ok"], true, "{located}");
                assert!(drawn_text(&app, "正在读项目图纸 0 / 3"));
                let drawn = drawn_ids(&app);
                for id in [BUSY_ID, CANCEL_ID] {
                    assert!(drawn.iter().any(|seen| seen == id), "{id}: {drawn:?}");
                }
                assert!(deliver(&mut app, &mut reads));
                assert_eq!(panel(&mut app, "")["busy"]["done"], 1);
                assert!(drawn_text(&app, "正在读项目图纸 1 / 3"));
                while deliver(&mut app, &mut reads) {}
                let state = panel(&mut app, "");
                assert_eq!(state["busy"], Value::Null, "{state}");
                assert_eq!(state["check"]["scope"], "project", "{state}");
                assert_eq!(state["buttons"]["check_project"], true, "{state}");
                assert!(app.pid_checks.of_project(&scratch.0).is_some());
                let drawn = drawn_ids(&app);
                assert!(!drawn.iter().any(|seen| seen == BUSY_ID), "{drawn:?}");
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// §4.7: 【取消】 stops reading. Nothing runs, the results there were
    /// stay, the message bar says 「已取消」, and what is read back after is
    /// not kept as a check. Sent as the action `cancel`, refused while
    /// nothing runs, or clicked on the progress line.
    #[test]
    fn cancelling_a_project_check_keeps_the_results_there_were() {
        let _one = crate::headless_ui_lock();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let scratch = Scratch::new();
                let mut app = sheets(&scratch);
                app.win_size = (1600.0, 900.0);
                typed(&mut app, "PIDCHECK");
                let before = panel(&mut app, "")["check"].clone();
                let mut reads = press_and_hold(&mut app, "check=project");
                assert!(deliver(&mut app, &mut reads));
                app.command_line.history.clear();
                let answer = act(&mut app, "cancel");
                assert_eq!(answer["ok"], true, "{answer}");
                let state = panel(&mut app, "");
                assert_eq!(state["busy"], Value::Null, "{state}");
                let cancelled = json!({"level": "info", "code": null, "text": "已取消"});
                assert_eq!(state["notice"], cancelled, "{state}");
                assert_eq!(state["check"], before, "{state}");
                assert!(said(&app).contains("取消了"), "{}", said(&app));
                while deliver(&mut app, &mut reads) {}
                assert!(app.pid_checks.of_project(&scratch.0).is_none());
                assert_eq!(panel(&mut app, "")["check"], before);
                let again = act(&mut app, "cancel");
                assert_eq!(again["code"], "PANEL_ACTION_UNAVAILABLE", "{again}");
                assert_eq!(again["reason"], "not_busy", "{again}");
                // Two drawings more to read, and 【取消】 clicked where it is
                // drawn.
                copy_pump(&scratch, &["PID-1105.dwg", "PID-1106.dwg"]);
                let mut reads = press_and_hold(&mut app, "check=project");
                assert_ne!(panel(&mut app, "")["busy"], Value::Null);
                click_drawn(&mut app, "取消");
                let state = panel(&mut app, "");
                assert_eq!(state["busy"], Value::Null, "{state}");
                assert_eq!(state["notice"], cancelled, "{state}");
                while deliver(&mut app, &mut reads) {}
                assert!(app.pid_checks.of_project(&scratch.0).is_none());
            })
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
    }

    /// [`sheets`] with a new drawing open beside PID-1101: `value` pressed in
    /// the panel on PID-1101, PID-1101 closed while the project is read, then
    /// the rest read back.
    fn closed_meanwhile(scratch: &Scratch, value: &str) -> OpenCADStudio {
        let mut app = sheets(scratch);
        let origin = app.tabs[app.active_tab].id;
        let _ = app.update(Message::TabNew);
        assert_ne!(app.tabs[app.active_tab].id, origin);
        activate(&mut app, origin, "pid-verify-origin");
        let mut reads = press_and_hold(&mut app, value);
        let (closed, task) = app.control_request(json!({
            "protocol": 1,
            "op": "close",
            "request_id": "pid-verify-close-origin",
            "document_id": origin,
            "discard": true,
        }));
        app.drive_headless_task(task).expect("the close's task");
        assert_eq!(closed["ok"], true, "{closed}");
        assert!(app.tabs.iter().all(|tab| tab.id != origin));
        while deliver(&mut app, &mut reads) {}
        app
    }

    /// §4.7: a project check or bill whose drawing was closed while the
    /// project was read is dropped, not kept for the folder.
    #[test]
    fn a_project_job_whose_drawing_closed_meanwhile_is_dropped() {
        let scratch = Scratch::new();
        let app = closed_meanwhile(&scratch, "check=project");
        assert!(app.pid_checks.of_project(&scratch.0).is_none());
        let scratch = Scratch::new();
        let app = closed_meanwhile(&scratch, "bom=project");
        assert!(app.pid_boms.projects.is_empty());
    }
}

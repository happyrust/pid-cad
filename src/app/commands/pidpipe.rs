//! `PIDPIPE` / `PIDFITTING` / `PIDBREAK` —— P&ID 管线与管件的作者命令。
//!
//! ```text
//! PIDPIPE                    建管线（pipe_line），两点 + Enter
//! PIDPIPE run [<line_tag>]   建管路（pipe_run），两点以上 + Enter
//! PIDPIPE signal [<tag>]     建信号线（signal_line），两点以上 + Enter
//! PIDFITTING <symbol_id>     在命中的管段上插管件，一点 + Enter
//! PIDBREAK                   在命中的管段上插打断符，一点 + Enter
//! PIDTOPOLOGY                读回拓扑表（调试/验证用）
//! ```
//!
//! 三种子模式用**同一个命令名 + 首参数**表达（`architecture.md` A.7 裁决），
//! `run` / `signal` 两个关键字优先于把首参数当管线号解析。
//!
//! 分工：命令只收点（[`PidPipeCommand`] / [`PidFittingCommand`] /
//! [`PidBreakCommand`]），落盘由 [`OpenCADStudio::commit_pid_pipeline`] 完成 ——
//! 建实体、写 `PID_AUTHOR`、维护 `PID_AUTHOR_TOPOLOGY` 都需要 `&mut Document`，
//! 而 `CadCommand` 没有文档访问权。

use super::pidpalette::{resolve_symbol_for_batch, sync_pid_tag_attribute};
use super::*;
use crate::command::{CadCommand, CmdResult, InputKind};
use crate::pid::persist::PidAuthor;
use crate::pid::tag_engine::{TagSequencer, TagView};
use crate::pid::topology::{
    handle_text, parse_handle, BreakRequest, FittingRecord, FittingRequest, PipeCommit, PipeMode,
    PipeRequest, SegmentEnd, TopologyTable, ROLE_FITTING, ROLE_PIPE_LINE, ROLE_PIPE_RUN,
    ROLE_PIPE_SEGMENT, ROLE_SIGNAL_LINE, TYPE_FITTING, TYPE_PIPE_LINE, TYPE_PIPE_RUN,
    TYPE_PIPE_SEGMENT, TYPE_SIGNAL_LINE, TYPE_VALVE, VALVE_PORT_NAMES,
};
use crate::pid::type_lib::TypeLib;
use crate::scene::ChangeKind;
use glam::DVec3;
use serde_json::{json, Value};

/// 信号线自动位号的起始序号（`S-2001` 起）。
const SIGNAL_START: i64 = 2001;

/// 打断点两侧各让开的世界距离。
///
/// 两段内侧端点因此严格有序（`end_A.x < start_B.x`），缝隙里正好放打断符
/// 的斜线，截图可见管线不再贯通。与 [`FITTING_CLEARANCE`] 同一量级但语义
/// 不同：那是阀门外形宽度，这是打断标记自身占的宽度。
const BREAK_CLEARANCE: f64 = 2.0;

/// 打断符斜线在缝隙内缩进的比例（0..0.5），留出可见的空白。
const BREAK_MARK_INSET: f64 = 0.2;

/// 打断符斜线的高度（世界单位），沿管段法向。
const BREAK_MARK_HEIGHT: f64 = 4.0;

/// 打断点不在任何管段上。
const BREAK_NO_SEGMENT: &str = "BREAK_NO_SEGMENT";

/// 打断点落在管段端点上（会留下零长度管段）。
const BREAK_AT_ENDPOINT: &str = "BREAK_AT_ENDPOINT";

/// 打断点离管段端点太近，让开 `BREAK_CLEARANCE` 后某一侧会退化。
const BREAK_TOO_CLOSE: &str = "BREAK_TOO_CLOSE";

// ── 命令：PIDPIPE ─────────────────────────────────────────────────────────

/// `PIDPIPE` 的三种子模式共用这一个命令。
pub struct PidPipeCommand {
    /// 子模式。
    mode: PipeMode,
    /// 已拾取的点。
    points: Vec<DVec3>,
    /// 光标位置，供橡皮筋预览。
    cursor: Option<DVec3>,
}

impl PidPipeCommand {
    /// 建管线：两点 + Enter。
    pub fn line() -> Self {
        Self {
            mode: PipeMode::Line,
            points: Vec::new(),
            cursor: None,
        }
    }

    /// 建管路：两点以上 + Enter。
    pub fn run(line_tag: Option<String>) -> Self {
        Self {
            mode: PipeMode::Run { line_tag },
            points: Vec::new(),
            cursor: None,
        }
    }

    /// 建信号线：两点以上 + Enter。
    pub fn signal(tag: Option<String>) -> Self {
        Self {
            mode: PipeMode::Signal { tag },
            points: Vec::new(),
            cursor: None,
        }
    }

    /// 本命令的提交请求；点数不够时返回 `None`。
    fn request(&self) -> Option<PipeCommit> {
        let needed = match self.mode {
            PipeMode::Line => 2,
            PipeMode::Run { .. } | PipeMode::Signal { .. } => 2,
        };
        if self.points.len() < needed {
            return None;
        }
        Some(PipeCommit::Pipe(PipeRequest {
            mode: self.mode.clone(),
            points: self
                .points
                .iter()
                .map(|p| [p.x, p.y, p.z])
                .collect::<Vec<_>>(),
        }))
    }

    /// 提交请求，供宿主在 `CmdResult` 里取用。
    pub fn take_commit(&self) -> Option<PipeCommit> {
        self.request()
    }
}

impl CadCommand for PidPipeCommand {
    fn name(&self) -> &'static str {
        "PIDPIPE"
    }

    fn tool_line(&self) -> Option<String> {
        Some(
            match self.mode {
                PipeMode::Line => "PIDPIPE",
                PipeMode::Run { .. } => "PIDPIPE run",
                PipeMode::Signal { .. } => "PIDPIPE signal",
            }
            .to_owned(),
        )
    }

    fn prompt(&self) -> String {
        match self.mode {
            PipeMode::Line => match self.points.len() {
                0 => "PIDPIPE  指定管线起点:".to_string(),
                _ => "PIDPIPE  指定管线终点  [Enter 结束]:".to_string(),
            },
            PipeMode::Run { .. } => match self.points.len() {
                0 => "PIDPIPE run  指定管路起点:".to_string(),
                _ => "PIDPIPE run  指定下一点  [Enter 结束]:".to_string(),
            },
            PipeMode::Signal { .. } => match self.points.len() {
                0 => "PIDPIPE signal  指定信号线起点:".to_string(),
                _ => "PIDPIPE signal  指定下一点  [Enter 结束]:".to_string(),
            },
        }
    }

    fn input_kind(&self) -> InputKind {
        InputKind::Point
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.points.push(pt);
        self.cursor = Some(pt);
        CmdResult::NeedPoint
    }

    fn on_preview_wires(&mut self, pt: DVec3) -> Vec<crate::scene::WireModel> {
        let Some(&last) = self.points.last() else {
            return Vec::new();
        };
        if !pt.is_finite() {
            return Vec::new();
        }
        vec![crate::scene::WireModel::solid_f64(
            "pid_pipe_rubber_band".to_string(),
            vec![[last.x, last.y, last.z], [pt.x, pt.y, pt.z]],
            crate::scene::WireModel::CYAN,
            false,
        )]
    }

    fn on_enter(&mut self) -> CmdResult {
        match self.take_commit() {
            Some(commit) => CmdResult::PidPipeCommit(Box::new(commit)),
            None => CmdResult::CancelWithMessage(
                "PIDPIPE: 至少需要两个点。".to_string(),
            ),
        }
    }
}

// ── 命令：PIDFITTING ─────────────────────────────────────────────────────

/// `PIDFITTING <symbol_id>`：在命中的管段上插管件（一点 + Enter）。
pub struct PidFittingCommand {
    /// 已解析的符号块名。
    block: String,
    /// 符号 id，写进拓扑表的 `fitting_type`。
    symbol_id: String,
    /// 符号类型映射的原始类型编码（`VALVE_GATE`），仅作记录。
    #[cfg_attr(not(test), allow(dead_code))]
    type_code: String,
    /// 拾取到的落点。
    point: Option<DVec3>,
}

impl PidFittingCommand {
    /// 由解析后的符号构造。
    pub fn new(block: String, symbol_id: String, type_code: String) -> Self {
        Self {
            block,
            symbol_id,
            type_code,
            point: None,
        }
    }

    /// 已解析的符号块名。
    #[cfg(test)]
    pub fn block(&self) -> &str {
        &self.block
    }
}

impl CadCommand for PidFittingCommand {
    fn name(&self) -> &'static str {
        "PIDFITTING"
    }

    fn prompt(&self) -> String {
        format!("PIDFITTING {}  在管段上指定插入点:", self.block)
    }

    fn input_kind(&self) -> InputKind {
        InputKind::Point
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.point = Some(pt);
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        let Some(point) = self.point else {
            return CmdResult::CancelWithMessage(
                "PIDFITTING: 需要先指定插入点。".to_string(),
            );
        };
        CmdResult::PidPipeCommit(Box::new(PipeCommit::Fitting(FittingRequest {
            symbol: self.symbol_id.clone(),
            block: self.block.clone(),
            point: [point.x, point.y, point.z],
        })))
    }
}

// `CmdResult` 的载荷在 `crate::command` 里已由 `PidPipeCommit(Box<PipeCommit>)`
// 表达；本模块不再需要额外包装类型。

// ── 命令：PIDBREAK ───────────────────────────────────────────────────────

/// `PIDBREAK`：在命中的管段上插打断符（一点 + Enter）。
///
/// **无参数**（`architecture.md` 附录 A.7）：打断点由拾取的那一点给出，
/// 打断符不引用符号库里的任何符号 —— 它是一段独立画出的隔断标记。
pub struct PidBreakCommand {
    /// 拾取到的打断点。
    point: Option<DVec3>,
}

impl PidBreakCommand {
    /// 一个等待打断点的命令。
    pub fn new() -> Self {
        Self { point: None }
    }
}

impl CadCommand for PidBreakCommand {
    fn name(&self) -> &'static str {
        "PIDBREAK"
    }

    fn prompt(&self) -> String {
        "PIDBREAK  在管段上指定打断点:".to_string()
    }

    fn input_kind(&self) -> InputKind {
        InputKind::Point
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        self.point = Some(pt);
        CmdResult::NeedPoint
    }

    fn on_enter(&mut self) -> CmdResult {
        let Some(point) = self.point else {
            return CmdResult::CancelWithMessage(
                "PIDBREAK: 需要先指定打断点。".to_string(),
            );
        };
        CmdResult::PidPipeCommit(Box::new(PipeCommit::Break(BreakRequest {
            point: [point.x, point.y, point.z],
        })))
    }
}

// ── 宿主侧：提交 ─────────────────────────────────────────────────────────

impl OpenCADStudio {
    /// 一次 P&ID 拓扑提交：建实体、写 `PID_AUTHOR`、维护拓扑表，全部在
    /// **一个** undo 步里。
    ///
    /// 三条口径：
    ///
    /// 1. 拓扑表先 `prune` 再写 —— 删除实体没有钩子，用「文档里找不到
    ///    handle」当唯一判据，悬空引用不是事实。
    /// 2. 位号走 [`TagSequencer`]：一次扫描、多次生成，避免 O(n²)。
    /// 3. 管段/管件继承宿主位号，只有阀门另有 `V-{序号}`。
    pub(in crate::app) fn commit_pid_pipeline(
        &mut self,
        i: usize,
        commit: PipeCommit,
    ) -> Option<String> {
        match commit {
            PipeCommit::Pipe(request) => self.commit_pid_pipe(i, request),
            PipeCommit::Fitting(request) => self.commit_pid_fitting(i, request),
            PipeCommit::Connector(request) => self.commit_pid_connector(i, request),
            PipeCommit::Break(request) => self.commit_pid_break(i, request),
        }
    }

    /// `PIDPIPE` 三种子模式的落盘。
    fn commit_pid_pipe(&mut self, i: usize, request: PipeRequest) -> Option<String> {
        if self.tabs[i].is_start {
            return None;
        }
        let points: Vec<DVec3> = request
            .points
            .iter()
            .map(|p| DVec3::new(p[0], p[1], p[2]))
            .collect();
        if points.len() < 2 {
            return None;
        }
        let types = TypeLib::embedded();
        let mut table = TopologyTable::load(&self.tabs[i].scene.document);
        table.prune(&self.tabs[i].scene.document);
        // 表里的 `props` 是 XDATA 的副本。新建管路要继承管线**当前**值
        // （`VAL-PIPE-012`），所以先把副本对齐到文档现值。
        table.sync_props_from_document(&self.tabs[i].scene.document);

        // 一条折线代表一次拾取序列：每段一条 `Line`，这样每段都能独立
        // 被阀门拆分、被移动补位（M5 后续 feature 的前提）。
        let segment_entities = |points: &[DVec3]| -> Vec<document::EntityType> {
            points
                .windows(2)
                .map(|pair| {
                    document::EntityType::Line(document::entities::Line::from_points(
                        document::types::Vector3::new(pair[0].x, pair[0].y, pair[0].z),
                        document::types::Vector3::new(pair[1].x, pair[1].y, pair[1].z),
                    ))
                })
                .collect()
        };

        self.push_undo_snapshot(i, "PIDPIPE");

        let props = std::collections::BTreeMap::new();
        let mut changed: Vec<(document::Handle, ChangeKind)> = Vec::new();
        let mut all_handles: Vec<document::Handle> = Vec::new();

        let receipt = match &request.mode {
            PipeMode::Line => {
                // 管线只是逻辑分组，没有管段概念：每段一条 `Line`，同一位号。
                let handles = self.tabs[i].scene.add_entities(segment_entities(&points));
                if handles.is_empty() {
                    self.discard_last_undo_entry(i);
                    return None;
                }
                let mut sequence = TagSequencer::new(&types, &self.tabs[i].scene.document);
                let view = sequence
                    .next(TYPE_PIPE_LINE, &props)
                    .unwrap_or_else(|_| TagView::empty());
                let line_handle = handles[0];
                all_handles.extend(handles.iter().copied());
                for handle in handles.iter().copied() {
                    PidAuthor {
                        type_code: TYPE_PIPE_LINE.to_string(),
                        tag: view.tag.clone(),
                        tag_source: view.tag_source.clone(),
                        tag_status: view.tag_status.clone(),
                        topology: ROLE_PIPE_LINE.to_string(),
                        props: "{}".to_string(),
                        ..PidAuthor::default()
                    }
                    .store(&mut self.tabs[i].scene.document, handle);
                    changed.push((handle, ChangeKind::Modified));
                }
                table.append_line(&view.tag, line_handle, props.clone());
                format!("PIDPIPE: 管线 {} ({} 段)", view.tag, handles.len())
            }
            PipeMode::Run { line_tag } => {
                // 管线号省略时取最后创建的那条；解析失败要回滚这次 undo 步。
                let Some(line) = resolve_line_tag(&table, line_tag.as_deref()) else {
                    self.discard_last_undo_entry(i);
                    return Some("PIDPIPE: 文档里还没有管线，先 PIDPIPE 建一条。".to_string());
                };
                let line_tag = line.tag.clone();
                let line_handle_text = line.handle.clone();
                // 管路后缀来自**宿主管线**的持久化序号：跨管线互不干扰，
                // 且删除管路后序号不回退（`VAL-PIPE-003` / `004`）。
                let ordinal = line.next_run_ordinal.max(1);
                let run_tag = format!("{line_tag}-{ordinal:02}");

                // 管路本体单独承载位号（放在起点），每两点一段 `Line` 作管段，
                // 这样「2 点 → 1 管段」「3 点 → 2 管段」与契约口径一致。
                let mut entities = vec![document::EntityType::Point(document::entities::Point::at(
                    document::types::Vector3::new(points[0].x, points[0].y, points[0].z),
                ))];
                entities.extend(segment_entities(&points));
                let handles = self.tabs[i].scene.add_entities(entities);
                if handles.len() < 2 {
                    self.discard_last_undo_entry(i);
                    self.tabs[i].scene.erase_entities(&handles);
                    return None;
                }
                let run_handle = handles[0];
                let segments: Vec<document::Handle> = handles.iter().copied().skip(1).collect();
                all_handles.extend(handles.iter().copied());

                let mut run_props = std::collections::BTreeMap::new();
                run_props.insert("管线号".to_string(), line_tag.clone());
                let run_props_json =
                    serde_json::to_string(&run_props).unwrap_or_else(|_| "{}".into());
                let view = match crate::pid::tag_engine::check_unique(
                    &types,
                    &self.tabs[i].scene.document,
                    TYPE_PIPE_RUN,
                    &run_tag,
                    None,
                ) {
                    Ok(()) => TagView::auto_ok(run_tag.clone()),
                    Err(_) => TagView::duplicated(run_tag.clone(), "auto"),
                };

                PidAuthor {
                    type_code: TYPE_PIPE_RUN.to_string(),
                    tag: view.tag.clone(),
                    tag_source: view.tag_source.clone(),
                    tag_status: view.tag_status.clone(),
                    topology: ROLE_PIPE_RUN.to_string(),
                    props: run_props_json.clone(),
                    owner: line_handle_text.clone(),
                    ..PidAuthor::default()
                }
                .store(&mut self.tabs[i].scene.document, run_handle);
                changed.push((run_handle, ChangeKind::Modified));

                // 管路创建时**全量继承管线当前值**（`VAL-PIPE-007`），并且
                // 继承的是管线的现值而不是创建初值（`VAL-PIPE-012`）——
                // `inherit_on_create` 读的就是管线此刻的 `PID_AUTHOR`。
                // 继承写入一律不打覆盖标记，所以新管路 `overrides` 为空。
                if let Some(line_handle) = parse_handle(&line_handle_text) {
                    let batch = crate::pid::propagate::inherit_on_create(
                        &self.tabs[i].scene.document,
                        line_handle,
                        run_handle,
                    );
                    crate::pid::propagate::apply(&mut self.tabs[i].scene.document, &batch);
                }
                let run_author =
                    PidAuthor::load(&self.tabs[i].scene.document, run_handle).unwrap_or_default();
                let (run_props_now, run_marks) =
                    crate::pid::propagate::split_props(&run_author.props);

                table.append_run(
                    &line_tag,
                    &view.tag,
                    run_handle,
                    run_props_now.clone(),
                    run_marks.iter().cloned().collect(),
                );
                if let Some(line) = table.line_by_tag_mut(&line_tag) {
                    // 序号只前进不回退：删除管路后再建是 `-03` 而不是 `-02`。
                    line.next_run_ordinal = ordinal.saturating_add(1);
                }
                let run_owner = handle_text(run_handle);
                // 管段**不直连管线**：它取管路当前值（继承之后的完整 props）。
                let segment_props = run_author.props.clone();
                for (index, handle) in segments.iter().copied().enumerate() {
                    PidAuthor {
                        type_code: TYPE_PIPE_SEGMENT.to_string(),
                        tag: String::new(),
                        tag_source: "auto".to_string(),
                        tag_status: "ok".to_string(),
                        topology: ROLE_PIPE_SEGMENT.to_string(),
                        props: segment_props.clone(),
                        owner: run_owner.clone(),
                        ..PidAuthor::default()
                    }
                    .store(&mut self.tabs[i].scene.document, handle);
                    changed.push((handle, ChangeKind::Modified));
                    let start = points[index];
                    let end = points[index + 1];
                    table.append_segment(
                        &view.tag,
                        handle,
                        SegmentEnd::open([start.x, start.y]),
                        SegmentEnd::open([end.x, end.y]),
                    );
                }
                format!(
                    "PIDPIPE run: 管路 {} ({} 段) 挂在 {}",
                    view.tag,
                    segments.len(),
                    line_tag
                )
            }
            PipeMode::Signal { tag } => {
                let handles = self.tabs[i].scene.add_entities(segment_entities(&points));
                if handles.is_empty() {
                    self.discard_last_undo_entry(i);
                    return None;
                }
                let tag = tag
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| next_signal_tag(&self.tabs[i].scene.document));
                all_handles.extend(handles.iter().copied());
                for handle in handles.iter().copied() {
                    PidAuthor {
                        type_code: TYPE_SIGNAL_LINE.to_string(),
                        tag: tag.clone(),
                        tag_source: "auto".to_string(),
                        tag_status: "ok".to_string(),
                        topology: ROLE_SIGNAL_LINE.to_string(),
                        props: "{}".to_string(),
                        ..PidAuthor::default()
                    }
                    .store(&mut self.tabs[i].scene.document, handle);
                    changed.push((handle, ChangeKind::Modified));
                }
                format!("PIDPIPE signal: 信号线 {} ({} 段)", tag, handles.len())
            }
        };

        table.store(&mut self.tabs[i].scene.document);
        for handle in all_handles.iter().copied() {
            sync_pid_tag_attribute(&mut self.tabs[i].scene.document, handle);
        }
        if !changed.is_empty() {
            self.tabs[i].scene.bump_entities(&changed);
        }
        self.tabs[i].dirty = true;
        self.post_ref_op(i);
        // `VAL-PIPE-001` 的 `added` 判据：本次提交新建了几个实体。
        self.set_control_result(json!({
            "added": all_handles.len(),
            "handles": all_handles.iter().map(|h| handle_text(*h)).collect::<Vec<_>>(),
            "message": receipt,
        }));
        Some(receipt)
    }

    /// `PIDFITTING` 的落盘：定位命中管段、按落点拆分、绑定阀门端口。
    fn commit_pid_fitting(&mut self, i: usize, request: FittingRequest) -> Option<String> {
        if self.tabs[i].is_start {
            return None;
        }
        let point = DVec3::new(request.point[0], request.point[1], request.point[2]);
        let mut table = TopologyTable::load(&self.tabs[i].scene.document);
        table.prune(&self.tabs[i].scene.document);

        let Some(hit) = find_segment_at(&table, point) else {
            return Some("PIDFITTING: 落点不在任何管段上。".to_string());
        };
        let Some(segment) = table.segment_by_handle(hit.segment).cloned() else {
            return Some("PIDFITTING: 命中的管段已不在拓扑表里。".to_string());
        };
        let run_tag = segment.run_tag.clone();

        // 管件实体：符号块必须先定义。
        let Some(symbol) = resolve_symbol_for_batch(&self.pid_symbols, &request.symbol) else {
            return Some(format!("PIDFITTING: 未知符号 \"{}\"", request.symbol));
        };
        if !self.tabs[i]
            .scene
            .document
            .block_records
            .contains(&symbol.block)
        {
            self.pid_symbols.instantiate(&mut self.tabs[i].scene);
        }

        let types = TypeLib::embedded();
        let mut sequence = TagSequencer::new(&types, &self.tabs[i].scene.document);
        let own_tag = symbol.author_type.eq_ignore_ascii_case(TYPE_VALVE);
        let view = if own_tag {
            sequence
                .next(&symbol.author_type, &std::collections::BTreeMap::new())
                .unwrap_or_else(|_| TagView::empty())
        } else {
            TagView::new(
                run_tag.clone(),
                "auto",
                "ok",
            )
        };

        self.push_undo_snapshot(i, "PIDFITTING");
        let fitting_entity = document::EntityType::Insert(
            document::entities::Insert::new(
                symbol.block.clone(),
                document::types::Vector3::new(point.x, point.y, point.z),
            )
            .with_uniform_scale(symbol.scale),
        );
        let fitting_handle = self.tabs[i].scene.add_entity(fitting_entity);

        let ports_text = self.pid_symbols.symbol_ports(&symbol.block).join(",");
        PidAuthor {
            type_code: if own_tag {
                TYPE_VALVE.to_string()
            } else {
                TYPE_FITTING.to_string()
            },
            tag: view.tag.clone(),
            tag_source: view.tag_source.clone(),
            tag_status: view.tag_status.clone(),
            ports: ports_text,
            owner: table
                .pipe_runs
                .iter()
                .find(|run| run.tag == run_tag)
                .map(|run| run.handle.clone())
                .unwrap_or_default(),
            topology: ROLE_FITTING.to_string(),
            props: "{}".to_string(),
            ..PidAuthor::default()
        }
        .store(&mut self.tabs[i].scene.document, fitting_handle);

        // 拆分：原管段 handle 作为第一段，新建实体作为第二段。
        let mut fitting_ports: std::collections::BTreeMap<String, Option<String>> =
            std::collections::BTreeMap::new();
        let at_endpoint = point_is_endpoint(&segment, point);
        if at_endpoint {
            // 端点插管件不拆分（`VAL-PIPE-018`）：只绑一个端口。
            let segment_handle = parse_handle(&segment.handle).unwrap_or(document::Handle::NULL);
            fitting_ports.insert(VALVE_PORT_NAMES[0].to_string(), Some(handle_text(segment_handle)));
            fitting_ports.insert(VALVE_PORT_NAMES[1].to_string(), None);
        } else {
            let first_handle = parse_handle(&segment.handle).unwrap_or(document::Handle::NULL);
            // 第二段继承原管段的 props（`PidAuthor::load` 在 `add_entity`
            // 之后仍读得到第一段自己的记录，先取出来避免在构造表达式里
            // 借文档）。
            let first_props = PidAuthor::load(&self.tabs[i].scene.document, first_handle)
                .map(|author| author.props)
                .unwrap_or_else(|| "{}".to_string());

            // 阀门两侧的内侧端点收在**端口坐标**上，不是落点本身：
            // `VAL-PIPE-016` 要求 `end_A.x < start_B.x`，中间那段宽度就是
            // 阀门外形（闸阀两个端口各在 ±14）。
            let (inner_a, inner_b) = fitting_port_points(
                &self.pid_symbols,
                &symbol.block,
                symbol.scale,
                point,
                segment.start.point,
            );

            let second = self.tabs[i].scene.add_entity(document::EntityType::Line(
                document::entities::Line::from_points(
                    document::types::Vector3::new(inner_b[0], inner_b[1], 0.0),
                    document::types::Vector3::new(segment.end.point[0], segment.end.point[1], 0.0),
                ),
            ));
            PidAuthor {
                type_code: TYPE_PIPE_SEGMENT.to_string(),
                tag: String::new(),
                tag_source: "auto".to_string(),
                tag_status: "ok".to_string(),
                topology: ROLE_PIPE_SEGMENT.to_string(),
                props: first_props,
                owner: table
                    .pipe_runs
                    .iter()
                    .find(|run| run.tag == run_tag)
                    .map(|run| run.handle.clone())
                    .unwrap_or_default(),
                ..PidAuthor::default()
            }
            .store(&mut self.tabs[i].scene.document, second);

            fitting_ports.insert(VALVE_PORT_NAMES[0].to_string(), Some(handle_text(first_handle)));
            fitting_ports.insert(VALVE_PORT_NAMES[1].to_string(), Some(handle_text(second)));

            // 第一段：起点不动，终点收在端口 1 上并指向它。
            if let Some(first) = table.segment_by_handle_mut(first_handle) {
                first.end = SegmentEnd::joined(inner_a, fitting_handle, VALVE_PORT_NAMES[0]);
            }
            // 第二段：起点指向端口 2，终点保持原管段终点。
            table.append_segment(
                &run_tag,
                second,
                SegmentEnd::joined(inner_b, fitting_handle, VALVE_PORT_NAMES[1]),
                segment.end.clone(),
            );
            hand_over_far_end(&mut table, &segment, second);
            // 几何同步：第一段的 Line 终点收到端口 1。
            if let Some(document::EntityType::Line(line)) =
                self.tabs[i].scene.document.get_entity_mut(first_handle)
            {
                line.end = document::types::Vector3::new(inner_a[0], inner_a[1], 0.0);
            }
            self.tabs[i]
                .scene
                .bump_entities(&[(first_handle, ChangeKind::Modified), (second, ChangeKind::Modified)]);
        }

        let fitting_tag = if own_tag { Some(view.tag.clone()) } else { None };
        table.fittings.push(FittingRecord {
            handle: handle_text(fitting_handle),
            run_tag: run_tag.clone(),
            fitting_type: request.symbol.clone(),
            tag: fitting_tag,
            ports: fitting_ports,
        });
        table.store(&mut self.tabs[i].scene.document);
        sync_pid_tag_attribute(&mut self.tabs[i].scene.document, fitting_handle);
        self.tabs[i]
            .scene
            .bump_entities(&[(fitting_handle, ChangeKind::Modified)]);
        self.tabs[i].dirty = true;
        self.post_ref_op(i);
        self.set_control_result(json!({
            "added": if at_endpoint { 1 } else { 2 },
            "handle": handle_text(fitting_handle),
        }));

        Some(if own_tag {
            format!("PIDFITTING: {} {} 插入在 {} 上", request.symbol, view.tag, run_tag)
        } else {
            format!("PIDFITTING: {} 插入在 {} 上", request.symbol, run_tag)
        })
    }

    /// `PIDBREAK` 的落盘：在命中的管段上插打断符，管段被分为**不连通**的两段。
    ///
    /// 四条口径（`architecture.md` A.7 / `VAL-PIPE-031` / `VAL-PIPE-032`）：
    ///
    /// 1. 打断符生成**独立实体**，`PID_AUTHOR` 的 `topology` 为 `fitting`，
    ///    并在拓扑表 `fittings` 里留一条记录。
    /// 2. **原管段 handle 作为第一段保留**，新建实体作第二段。
    /// 3. 两段沿管段方向留出 [`BREAK_CLEARANCE`] 的间隙，区间严格不重叠。
    /// 4. 两段内侧端点的 `peer` 都是 `None` —— 打断符**不**出现在任何管段的
    ///    `peer` 里，隔断而非导通。
    fn commit_pid_break(&mut self, i: usize, request: BreakRequest) -> Option<String> {
        if self.tabs[i].is_start {
            return None;
        }
        let point = DVec3::new(request.point[0], request.point[1], request.point[2]);
        let mut table = TopologyTable::load(&self.tabs[i].scene.document);
        table.prune(&self.tabs[i].scene.document);

        let Some(hit) = find_segment_at(&table, point) else {
            self.command_line
                .push_error_coded(BREAK_NO_SEGMENT, "PIDBREAK: 落点不在任何管段上。");
            return None;
        };
        let Some(segment) = table.segment_by_handle(hit.segment).cloned() else {
            self.command_line
                .push_error_coded(BREAK_NO_SEGMENT, "PIDBREAK: 命中的管段已不在拓扑表里。");
            return None;
        };
        if point_is_endpoint(&segment, point) {
            self.command_line.push_error_coded(
                BREAK_AT_ENDPOINT,
                "PIDBREAK: 打断点不能落在管段端点上（会留下零长度管段）。",
            );
            return None;
        }
        let run_tag = segment.run_tag.clone();
        let run_handle_text = table
            .pipe_runs
            .iter()
            .find(|run| run.tag == run_tag)
            .map(|run| run.handle.clone())
            .unwrap_or_default();

        let first_handle = parse_handle(&segment.handle).unwrap_or(document::Handle::NULL);
        // 第二段继承原管段的 `props` / `owner`：管段无覆盖标记，始终取管路
        // 当前值，所以直接复制第一段现值即可（`VAL-PIPE-010`）。
        let first_author = PidAuthor::load(&self.tabs[i].scene.document, first_handle);
        let segment_props = first_author
            .as_ref()
            .map(|author| author.props.clone())
            .unwrap_or_else(|| "{}".to_string());
        let segment_owner = first_author
            .as_ref()
            .map(|author| author.owner.clone())
            .filter(|owner| !owner.is_empty())
            .unwrap_or(run_handle_text);

        // 打断点两侧各让开 `BREAK_CLEARANCE`：区间严格不重叠，且打断符的
        // 斜线落在缝隙里，截图可见管线不再贯通。
        let Some((inner_a, inner_b)) = break_points(segment.start.point, segment.end.point, point)
        else {
            self.command_line.push_error_coded(
                BREAK_TOO_CLOSE,
                "PIDBREAK: 打断点离管段端点太近，打断后会留下零长度管段。",
            );
            return None;
        };

        self.push_undo_snapshot(i, "PIDBREAK");

        // 打断符实体：一条跨过缝隙的斜线，独立实体、不引用符号库。
        let (mark_from, mark_to) = break_mark_points(inner_a, inner_b);
        let marker = document::EntityType::Line(document::entities::Line::from_points(
            document::types::Vector3::new(mark_from[0], mark_from[1], 0.0),
            document::types::Vector3::new(mark_to[0], mark_to[1], 0.0),
        ));
        let break_handle = self.tabs[i].scene.add_entity(marker);
        PidAuthor {
            type_code: TYPE_FITTING.to_string(),
            tag: String::new(),
            tag_source: "auto".to_string(),
            tag_status: "ok".to_string(),
            topology: ROLE_FITTING.to_string(),
            owner: segment_owner.clone(),
            props: "{}".to_string(),
            ..PidAuthor::default()
        }
        .store(&mut self.tabs[i].scene.document, break_handle);

        // 第二段：新实体，起点在打断点右侧（无 peer），终点保持原管段终点。
        let second = self.tabs[i].scene.add_entity(document::EntityType::Line(
            document::entities::Line::from_points(
                document::types::Vector3::new(inner_b[0], inner_b[1], 0.0),
                document::types::Vector3::new(segment.end.point[0], segment.end.point[1], 0.0),
            ),
        ));
        PidAuthor {
            type_code: TYPE_PIPE_SEGMENT.to_string(),
            tag: String::new(),
            tag_source: "auto".to_string(),
            tag_status: "ok".to_string(),
            topology: ROLE_PIPE_SEGMENT.to_string(),
            props: segment_props,
            owner: segment_owner,
            ..PidAuthor::default()
        }
        .store(&mut self.tabs[i].scene.document, second);

        // 第一段：起点不动，终点收在打断点左侧，**peer 保持 null**。
        if let Some(first) = table.segment_by_handle_mut(first_handle) {
            first.end = SegmentEnd::open(inner_a);
        }
        table.append_segment(
            &run_tag,
            second,
            SegmentEnd::open(inner_b),
            segment.end.clone(),
        );
        hand_over_far_end(&mut table, &segment, second);
        // 几何同步：第一段的 Line 终点收到打断点左侧。
        if let Some(document::EntityType::Line(line)) =
            self.tabs[i].scene.document.get_entity_mut(first_handle)
        {
            line.end = document::types::Vector3::new(inner_a[0], inner_a[1], 0.0);
        }

        // 打断符在 `fittings` 里有一条记录，但 `ports` 为空 —— 它不绑定任何
        // 管段，也就不会出现在任何管段的 `peer` 里（`VAL-PIPE-032`）。
        table.fittings.push(FittingRecord {
            handle: handle_text(break_handle),
            run_tag: run_tag.clone(),
            fitting_type: crate::pid::topology::FITTING_BREAK.to_string(),
            tag: None,
            ports: std::collections::BTreeMap::new(),
        });
        table.store(&mut self.tabs[i].scene.document);

        self.tabs[i].scene.bump_entities(&[
            (first_handle, ChangeKind::Modified),
            (second, ChangeKind::Modified),
            (break_handle, ChangeKind::Modified),
        ]);
        self.tabs[i].dirty = true;
        self.post_ref_op(i);
        self.set_control_result(json!({
            "added": 2,
            "handle": handle_text(break_handle),
            "first": handle_text(first_handle),
            "second": handle_text(second),
        }));

        Some(format!(
            "PIDBREAK: 打断符 {} 插在 {} 上，管段分为 {} 与 {}",
            handle_text(break_handle),
            run_tag,
            handle_text(first_handle),
            handle_text(second)
        ))
    }

    /// `PIDTOPOLOGY`：把拓扑表写进 `result.topology`（调试/验证用）。
    pub(in crate::app) fn control_pid_topology(&mut self) -> Value {
        let table = TopologyTable::load(&self.tabs[self.active_tab].scene.document);
        self.set_control_result(json!({ "topology": table }));
        json!({"ok":true,"status":"completed"})
    }
}

/// 按 `line_tag` 找管线；省略时取**最后创建**的那条。
fn resolve_line_tag<'a>(
    table: &'a TopologyTable,
    line_tag: Option<&str>,
) -> Option<&'a crate::pid::topology::PipeLineRecord> {
    match line_tag.map(str::trim).filter(|tag| !tag.is_empty()) {
        Some(tag) => table.line_by_tag(tag),
        None => table.last_line(),
    }
}

/// 文档里已有的最大 `S-{n}`，加一。信号线不占位号序号空间（它不是
/// `PIPE_LINE`），所以按文档扫描自算。
fn next_signal_tag(doc: &document::Document) -> String {
    let mut max = SIGNAL_START - 1;
    for entity in doc.entities() {
        let Some(author) = PidAuthor::load(doc, entity.common().handle) else {
            continue;
        };
        if !author.type_code.eq_ignore_ascii_case(TYPE_SIGNAL_LINE) {
            continue;
        }
        if let Some(rest) = author.tag.strip_prefix("S-") {
            if let Ok(value) = rest.parse::<i64>() {
                max = max.max(value);
            }
        }
    }
    format!("S-{}", max + 1)
}

/// 落点命中的管段。
struct SegmentHit {
    segment: document::Handle,
}

/// 落点到各管段的最近距离；超出容差返回 `None`。
fn find_segment_at(table: &TopologyTable, point: DVec3) -> Option<SegmentHit> {
    let mut best: Option<(f64, SegmentHit)> = None;
    for record in &table.pipe_segments {
        let Some(handle) = parse_handle(&record.handle) else {
            continue;
        };
        let a = DVec3::new(record.start.point[0], record.start.point[1], 0.0);
        let b = DVec3::new(record.end.point[0], record.end.point[1], 0.0);
        let (_t, distance) = project_on_segment(a, b, point);
        if best.as_ref().is_some_and(|(d, _)| distance >= *d) {
            continue;
        }
        best = Some((distance, SegmentHit { segment: handle }));
    }
    // 命中容差取 1 个单位：符号端口到 14 个单位，管线宽度通常 0。
    best.filter(|(distance, _)| *distance <= 1.0)
        .map(|(_, hit)| hit)
}

/// 点在线段上的投影参数与距离。
fn project_on_segment(a: DVec3, b: DVec3, p: DVec3) -> (f64, f64) {
    let ab = b - a;
    let length_sq = ab.length_squared();
    if length_sq <= 1e-12 {
        return (0.0, (p - a).length());
    }
    let t = ((p - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    (t, (a + ab * t - p).length())
}

/// 落点是否落在管段端点上（容差 1e-6）。
fn point_is_endpoint(segment: &crate::pid::topology::PipeSegmentRecord, point: DVec3) -> bool {
    let near = |p: [f64; 2]| (p[0] - point.x).abs() <= 1e-6 && (p[1] - point.y).abs() <= 1e-6;
    near(segment.start.point) || near(segment.end.point)
}

/// 拆段后原管段的终点归第二段：那一头接着管件时，管件记录里指着原管段的端口改指第二段。
fn hand_over_far_end(
    table: &mut TopologyTable,
    original: &crate::pid::topology::PipeSegmentRecord,
    second: document::Handle,
) {
    let Some(peer) = original.end.peer.as_deref() else {
        return;
    };
    let Some((fitting, port)) = peer.split_once(':') else {
        return;
    };
    let stale = Some(original.handle.clone());
    let slot = table
        .fittings
        .iter_mut()
        .find(|record| record.handle == fitting)
        .and_then(|record| record.ports.get_mut(port))
        .filter(|slot| **slot == stale);
    if let Some(slot) = slot {
        *slot = Some(handle_text(second));
    }
}

/// 管段内侧端点相对阀门端口再让开的世界距离。
///
/// `VAL-PIPE-017` 的截图判据按实体**包围盒**比对 `screen_bounds`，而 P&ID
/// 符号的包围盒含位号 ATTRIB 文字（`AGENTS.md`「`capture` 的坐标语义」：
/// `world_center` 是含 ATTRIB 的包围盒中点），文字比阀体宽出一点。留这道
/// 缝既让管线不压在阀体上，也让「段A → 阀门 → 段B」在像素上严格有序。
const FITTING_CLEARANCE: f64 = 2.0;

/// 管件两个端口的世界坐标，按「沿管段方向、以落点为中心」摆放。
///
/// 半宽取符号定义里端口 `at[0]` 的绝对值（闸阀是 14），乘插入缩放，再加
/// [`FITTING_CLEARANCE`]。方向取「管段起点 → 落点」，于是返回的第一个点
/// 总在靠起点侧、第二个点在靠终点侧 —— 拆开后 `end_A` 恒在 `start_B` 之前，
/// 中间那段就是阀门外形（`VAL-PIPE-016` 的 `end_A.x < start_B.x`）。
fn fitting_port_points(
    library: &crate::pid::symbols::SymbolLibrary,
    block: &str,
    scale: f64,
    center: DVec3,
    segment_start: [f64; 2],
) -> ([f64; 2], [f64; 2]) {
    let half = library
        .symbol(block)
        .map(|definition| {
            definition
                .ports
                .iter()
                .map(|port| port.at[0].abs().max(port.at[1].abs()))
                .fold(0.0_f64, f64::max)
        })
        .unwrap_or(0.0)
        * scale.abs()
        + FITTING_CLEARANCE;
    let mut direction = DVec3::new(
        center.x - segment_start[0],
        center.y - segment_start[1],
        0.0,
    );
    if direction.length_squared() <= 1e-12 {
        direction = DVec3::X;
    }
    let direction = direction.normalize();
    let first = center - direction * half;
    let second = center + direction * half;
    ([first.x, first.y], [second.x, second.y])
}

/// 打断点两侧的缝隙端点，按「管段起点 → 终点」方向。
///
/// 返回 `None` 表示打断点离某一侧端点不足 [`BREAK_CLEARANCE`]，让开后那一
/// 侧会退化成零长度管段 —— 调用方按 [`BREAK_TOO_CLOSE`] 拒绝。
fn break_points(
    start: [f64; 2],
    end: [f64; 2],
    point: DVec3,
) -> Option<([f64; 2], [f64; 2])> {
    let a = DVec3::new(start[0], start[1], 0.0);
    let b = DVec3::new(end[0], end[1], 0.0);
    let ab = b - a;
    let length = ab.length();
    if length <= 1e-9 {
        return None;
    }
    let direction = ab / length;
    let first = point - direction * BREAK_CLEARANCE;
    let second = point + direction * BREAK_CLEARANCE;
    // 让开后两侧都必须仍有正长度，且方向不反转。
    let first_span = (first - a).dot(direction);
    let second_span = (b - second).dot(direction);
    if first_span <= 1e-9 || second_span <= 1e-9 {
        return None;
    }
    Some(([first.x, first.y], [second.x, second.y]))
}

/// 打断符的两个端点：一条斜线跨过缝隙，落在两段之间。
///
/// 斜线从缝隙起点侧的上方拉到终点侧的下方，因此整条标记都画在
/// `[inner_a, inner_b]` 之内，不与任何管段重叠（`VAL-PIPE-031` 的「区间不
/// 重叠」判据不会把它算进管段区间）。
fn break_mark_points(inner_a: [f64; 2], inner_b: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let a = DVec3::new(inner_a[0], inner_a[1], 0.0);
    let b = DVec3::new(inner_b[0], inner_b[1], 0.0);
    let span = b - a;
    let length = span.length();
    let direction = if length <= 1e-9 {
        DVec3::X
    } else {
        span / length
    };
    let normal = DVec3::new(-direction.y, direction.x, 0.0);
    let half = BREAK_MARK_HEIGHT * 0.5;
    let inset = length * BREAK_MARK_INSET;
    let from = a + direction * inset + normal * half;
    let to = b - direction * inset - normal * half;
    ([from.x, from.y], [to.x, to.y])
}

// ── 命令分发 ─────────────────────────────────────────────────────────────

impl OpenCADStudio {
    /// `PIDPIPE` / `PIDFITTING` / `PIDBREAK` / `PIDTOPOLOGY` 的分发入口。
    ///
    /// 四个名字同属一个 family：它们共享拓扑表与符号解析，分开注册会让
    /// 「谁拥有拓扑表」变成四处约定。
    pub(super) fn dispatch_pidpipe(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        let verb = words.next()?;
        if verb.eq_ignore_ascii_case("PIDTOPOLOGY") {
            self.control_pid_topology();
            self.command_line
                .push_info("PIDTOPOLOGY: 拓扑表已读回（见 result.topology）。");
            return Some(Task::none());
        }
        if verb.eq_ignore_ascii_case("PIDBREAK") {
            let command = PidBreakCommand::new();
            let prompt = command.prompt();
            self.reset_command_start_state(i);
            self.command_line.push_info(&prompt);
            self.tabs[i].active_cmd = Some(Box::new(command));
            return Some(self.finish_dispatch(cmd));
        }
        if verb.eq_ignore_ascii_case("PIDPIPE") {
            let arg = words.next();
            let second = words.next();
            let command = match arg.map(str::to_ascii_lowercase).as_deref() {
                // `run` / `signal` 两个关键字优先于把首参数当管线号解析。
                Some("run") => PidPipeCommand::run(second.map(str::to_string)),
                Some("signal") => PidPipeCommand::signal(second.map(str::to_string)),
                // 无参，或首参数是一条管线号（未支持显式指定，按无参处理）。
                _ => PidPipeCommand::line(),
            };
            let prompt = command.prompt();
            self.reset_command_start_state(i);
            self.command_line.push_info(&prompt);
            self.tabs[i].active_cmd = Some(Box::new(command));
            return Some(self.finish_dispatch(cmd));
        }
        if verb.eq_ignore_ascii_case("PIDFITTING") {
            let Some(name) = words.next() else {
                self.command_line
                    .push_info("PIDFITTING  symbol id <gate_valve>:");
                return Some(Task::none());
            };
            let Some(symbol) = resolve_symbol_for_batch(&self.pid_symbols, name) else {
                self.command_line.push_error_coded(
                    "unknown_symbol",
                    &format!(
                        "PIDFITTING: unknown symbol \"{name}\" -- the palette lists what \
                         assets/pid-symbols.json defines."
                    ),
                );
                return Some(Task::none());
            };
            let symbol_id = symbol
                .block
                .strip_prefix(crate::pid::symbols::BLOCK_PREFIX)
                .and_then(|rest| rest.split_once('_'))
                .map(|(_, id)| id.to_string())
                .unwrap_or_else(|| name.to_string());
            let command = PidFittingCommand::new(symbol.block, symbol_id, symbol.type_code);
            let prompt = command.prompt();
            self.reset_command_start_state(i);
            self.command_line.push_info(&prompt);
            self.tabs[i].active_cmd = Some(Box::new(command));
            return Some(self.finish_dispatch(cmd));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenCADStudio;

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        if let Some(object) = req.as_object_mut() {
            object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
            if !object.contains_key("request_id") {
                // 控制面要求每条请求带唯一 id（幂等去重用），测试里按序号生成。
                let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                object.insert("request_id".into(), json!(format!("pidpipe-test-{serial}")));
            }
        }
        app.control_request(req).0
    }

    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// 起 `cmd`，依次喂点，最后回车。
    fn draw(app: &mut OpenCADStudio, cmd: &str, points: &[[f64; 3]]) -> Value {
        let started = request(app, json!({"op":"run","cmd":cmd}));
        assert_eq!(started["ok"], true, "{cmd} 应启动：{started}");
        for point in points {
            let fed = request(
                app,
                json!({"op":"input","kind":"point","point":point,"space":"wcs"}),
            );
            assert_eq!(fed["ok"], true, "喂点应被接受：{fed}");
        }
        request(app, json!({"op":"input","kind":"enter"}))
    }

    fn topology(app: &OpenCADStudio) -> TopologyTable {
        TopologyTable::load(&app.tabs[app.active_tab].scene.document)
    }

    fn author_of(app: &OpenCADStudio, handle: document::Handle) -> Option<PidAuthor> {
        PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle)
    }

    /// 文档里的全部实体 handle（按创建序）。
    fn handles(app: &OpenCADStudio) -> Vec<document::Handle> {
        app.tabs[app.active_tab]
            .scene
            .document
            .entities()
            .map(|entity| entity.common().handle)
            .collect()
    }

    /// `VAL-PIPE-001`：`PIDPIPE` 两点画线后新增一个 `topology=pipe_line`、
    /// `tag=L-1001` 的实体，拓扑表 `pipe_lines` 长度 1 且 `runs` 为空。
    #[test]
    fn pidpipe_creates_a_pipe_line_tagged_l_1001() {
        let mut app = app();
        let done = draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(done["status"], "completed", "{done}");

        let handles = handles(&app);
        assert_eq!(handles.len(), 1, "两点画一条线：{handles:?}");
        let author = author_of(&app, handles[0]).expect("带 PID_AUTHOR");
        assert_eq!(author.topology, "pipe_line");
        assert_eq!(author.tag, "L-1001");
        assert_eq!(author.tag_source, "auto");

        let table = topology(&app);
        assert_eq!(table.pipe_lines.len(), 1);
        assert_eq!(table.pipe_lines[0].tag, "L-1001");
        assert!(table.pipe_lines[0].runs.is_empty());
        assert_eq!(table.pipe_lines[0].handle, handle_text(handles[0]));
    }

    /// `VAL-PIPE-001` 的 `added` 判据：`run` 响应报告本次新建的实体数。
    #[test]
    fn the_run_response_reports_how_many_entities_were_added() {
        let mut app = app();
        let done = draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        assert_eq!(done["status"], "completed", "{done}");
        assert_eq!(done["result"]["added"], 1, "{done}");
        assert_eq!(done["result"]["handles"].as_array().map(Vec::len), Some(1));
    }

    /// 第二条管线是 `L-1002`，与第一条互不干扰。
    #[test]
    fn a_second_line_gets_the_next_line_number() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 500.0, 0.0], [200.0, 500.0, 0.0]]);
        let table = topology(&app);
        let tags: Vec<&str> = table.pipe_lines.iter().map(|l| l.tag.as_str()).collect();
        assert_eq!(tags, vec!["L-1001", "L-1002"]);
    }

    /// `VAL-PIPE-002`：在管线上建管路得到 `L-1001-01`，`owner` 指向管线
    /// handle，拓扑表 `pipe_lines[0].runs == ["L-1001-01"]`。
    #[test]
    fn a_run_on_the_line_is_tagged_l_1001_01() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let line_handle = handles(&app)[0];
        let done = draw(
            &mut app,
            "PIDPIPE run",
            &[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 50.0, 0.0]],
        );
        assert_eq!(done["ok"], true, "{done}");

        let table = topology(&app);
        assert_eq!(table.pipe_runs.len(), 1);
        assert_eq!(table.pipe_runs[0].tag, "L-1001-01");
        assert_eq!(table.pipe_lines[0].runs, vec!["L-1001-01".to_string()]);
        assert_eq!(table.pipe_runs[0].line_tag, "L-1001");
        // 两个点 → 一段，三个点 → 两段。
        assert_eq!(table.pipe_segments.len(), 2);

        // 管路实体 owner 指向管线 handle。
        let run_handle = parse_handle(&table.pipe_runs[0].handle).expect("handle");
        let author = author_of(&app, run_handle).expect("带 PID_AUTHOR");
        assert_eq!(author.topology, "pipe_run");
        assert_eq!(author.tag, "L-1001-01");
        assert_eq!(author.owner, handle_text(line_handle));
    }

    /// `VAL-PIPE-003`：第二条管路 `L-1001-02`；删掉后再建是 `L-1001-03`。
    #[test]
    fn run_suffix_increments_and_deleted_numbers_are_not_reused() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 100.0, 0.0], [200.0, 100.0, 0.0]]);
        let table = topology(&app);
        assert_eq!(
            table.pipe_lines[0].runs,
            vec!["L-1001-01".to_string(), "L-1001-02".to_string()]
        );

        // 删除第二条管路的全部实体。
        let second = table.pipe_runs[1].clone();
        let mut to_erase: Vec<document::Handle> = vec![parse_handle(&second.handle).unwrap()];
        to_erase.extend(
            second
                .segments
                .iter()
                .filter_map(|h| parse_handle(h)),
        );
        for handle in &to_erase {
            app.tabs[app.active_tab].scene.document.remove_entity(*handle);
        }
        // 拓扑表在下次创建时 prune。
        draw(&mut app, "PIDPIPE run", &[[0.0, 200.0, 0.0], [200.0, 200.0, 0.0]]);
        let table = topology(&app);
        let tags: Vec<&str> = table.pipe_runs.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(tags, vec!["L-1001-01", "L-1001-03"], "不复用已删编号");
        assert_eq!(
            table.pipe_lines[0].runs,
            vec!["L-1001-01".to_string(), "L-1001-03".to_string()]
        );
    }

    /// `VAL-PIPE-004`：不同管线的管路位号互不干扰。
    #[test]
    fn runs_of_different_lines_do_not_interfere() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE", &[[0.0, 500.0, 0.0], [200.0, 500.0, 0.0]]);
        // 省略管线号 → 取最后创建的 L-1002。
        draw(&mut app, "PIDPIPE run", &[[0.0, 500.0, 0.0], [200.0, 500.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_lines.len(), 2);
        assert_eq!(table.pipe_lines[0].runs, vec!["L-1001-01".to_string()]);
        assert_eq!(table.pipe_lines[1].runs, vec!["L-1002-01".to_string()]);
        assert_eq!(table.pipe_runs[1].line_tag, "L-1002");
    }

    /// `VAL-PIPE-005`：管段无独立位号 —— XDATA 里 `tag` 为空，拓扑表
    /// `pipe_segments[]` 无 `tag` 字段，`run_tag` 指向宿主管路。
    #[test]
    fn a_segment_has_no_tag_of_its_own() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 1);
        let segment = &table.pipe_segments[0];
        assert_eq!(segment.run_tag, "L-1001-01");
        let json = serde_json::to_value(segment).unwrap();
        assert!(json.get("tag").is_none(), "管段记录不该有 tag：{json}");

        let handle = parse_handle(&segment.handle).expect("handle");
        let author = author_of(&app, handle).expect("带 PID_AUTHOR");
        assert_eq!(author.topology, "pipe_segment");
        assert!(author.tag.is_empty(), "管段 XDATA 的 tag 必须为空：{:?}", author.tag);
    }

    /// `VAL-PIPE-006`：阀门继承宿主管路位号作为 `owner`，另有独立位号
    /// `V-1001`；同一管路上第二个闸阀是 `V-1002`。
    #[test]
    fn a_valve_has_its_own_tag_and_an_owner_pointing_at_the_run() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let run_handle = parse_handle(&topology(&app).pipe_runs[0].handle).unwrap();

        let done = draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");

        let table = topology(&app);
        assert_eq!(table.fittings.len(), 1);
        let fitting = &table.fittings[0];
        assert_eq!(fitting.fitting_type, "gate_valve");
        assert_eq!(fitting.tag.as_deref(), Some("V-1001"));
        assert_eq!(fitting.run_tag, "L-1001-01");

        let handle = parse_handle(&fitting.handle).expect("handle");
        let author = author_of(&app, handle).expect("带 PID_AUTHOR");
        assert_eq!(author.topology, "fitting");
        assert_eq!(author.tag, "V-1001");
        assert_eq!(author.owner, handle_text(run_handle), "管件 owner 指向宿主管路");
        assert_ne!(author.tag, "L-1001-01");

        // 同一管路上第二个闸阀：V-1002。
        let done = draw(&mut app, "PIDFITTING gate_valve", &[[50.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");
        let table = topology(&app);
        let tags: Vec<&str> = table
            .fittings
            .iter()
            .filter_map(|f| f.tag.as_deref())
            .collect();
        assert_eq!(tags, vec!["V-1001", "V-1002"]);
    }

    /// `VAL-PIPE-028` 的一半：拓扑表落在 `PID_AUTHOR_TOPOLOGY` XRecord 上，
    /// 四个集合齐全，且**不寄生在管段实体上**。
    #[test]
    fn the_topology_lives_in_the_xrecord_and_not_on_the_segment_entity() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0, 0.0]]);

        let document = &app.tabs[app.active_tab].scene.document;
        let record = document
            .xrecord(
                document.header.model_space_block_handle,
                crate::pid::graph_data::PID_AUTHOR_TOPOLOGY,
            )
            .expect("拓扑表 XRecord 必须存在");
        assert!(!record.entries.is_empty());

        let table = topology(&app);
        assert_eq!(table.pipe_lines.len(), 1);
        assert_eq!(table.pipe_runs.len(), 1);
        assert_eq!(table.pipe_segments.len(), 2, "阀门把一段拆成两段");
        assert_eq!(table.fittings.len(), 1);

        // 管段 XDATA 只有轻量键，不含完整拓扑表。
        let segment_handle = parse_handle(&table.pipe_segments[0].handle).unwrap();
        let raw = document
            .get_entity(segment_handle)
            .unwrap()
            .common()
            .extended_data
            .get_record(crate::pid::persist::PID_AUTHOR_APP)
            .expect("管段带 PID_AUTHOR");
        let text: Vec<String> = raw
            .values
            .iter()
            .filter_map(|v| crate::pid::persist::as_str(v).map(str::to_string))
            .collect();
        assert!(
            !text.iter().any(|t| t.contains("pipe_segments")),
            "管段 XDATA 不该带完整拓扑表：{text:?}"
        );
    }

    /// 端点插阀门不拆分（`VAL-PIPE-018`）：管段数保持 1，端口 1 绑该段、
    /// 端口 2 为 `null`。
    #[test]
    fn a_fitting_on_a_segment_endpoint_does_not_split() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDFITTING gate_valve", &[[0.0, 0.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 1, "端点插管件不拆分");
        let fitting = &table.fittings[0];
        assert_eq!(
            fitting.ports[VALVE_PORT_NAMES[0]],
            Some(table.pipe_segments[0].handle.clone())
        );
        assert_eq!(fitting.ports[VALVE_PORT_NAMES[1]], None);
    }

    /// 中点插阀门拆成两段：第一段保留原 handle，两段各连阀门一个端口。
    #[test]
    fn a_fitting_at_the_midpoint_splits_the_segment_in_two() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let original = topology(&app).pipe_segments[0].handle.clone();
        draw(&mut app, "PIDFITTING gate_valve", &[[100.0, 0.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 2);
        assert_eq!(
            table.pipe_runs[0].segments.len(),
            2,
            "管路的 segments 也要两条"
        );
        assert_eq!(
            table.pipe_segments[0].handle, original,
            "原管段 handle 作为第一段"
        );
        let fitting = &table.fittings[0];
        assert_eq!(fitting.ports[VALVE_PORT_NAMES[0]].as_deref(), Some(original.as_str()));
        assert_eq!(
            fitting.ports[VALVE_PORT_NAMES[1]].as_deref(),
            Some(table.pipe_segments[1].handle.as_str())
        );
        // 第一段终点指向阀门端口 1，第二段起点指向端口 2。
        assert_eq!(
            table.pipe_segments[0].end.peer.as_deref(),
            Some(format!("{}:连接点1", fitting.handle).as_str())
        );
        assert_eq!(
            table.pipe_segments[1].start.peer.as_deref(),
            Some(format!("{}:连接点2", fitting.handle).as_str())
        );
        // 外侧端点不动。
        assert_eq!(table.pipe_segments[0].start.point, [0.0, 0.0]);
        assert_eq!(table.pipe_segments[1].end.point, [200.0, 0.0]);
        // 沿 X 严格不重叠。
        assert!(table.pipe_segments[0].end.point[0] < table.pipe_segments[1].start.point[0]);
    }

    /// 在终点已经接着阀门的管段上再插阀门：拆出来的第二段接过原管段的终点，
    /// 连同 `peer`；先放的阀门记录里那个端口改指第二段。
    #[test]
    fn a_second_fitting_keeps_the_first_one_bound() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(
            &mut app,
            "PIDPIPE run",
            &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]],
        );
        draw(&mut app, "PIDFITTING gate_valve", &[[150.0, 0.0, 0.0]]);
        let first = topology(&app).fittings[0].handle.clone();
        let done = draw(&mut app, "PIDFITTING gate_valve", &[[50.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");

        let table = topology(&app);
        let peer = format!("{first}:{}", VALVE_PORT_NAMES[0]);
        let bound: Vec<_> = table
            .pipe_segments
            .iter()
            .filter(|segment| segment.end.peer.as_deref() == Some(peer.as_str()))
            .collect();
        assert_eq!(bound.len(), 1, "先放的阀门入口还有一段连着：{table:?}");
        assert_eq!(bound[0].end.point, [134.0, 0.0], "终点还是原管段的终点");
        let record = table
            .fittings
            .iter()
            .find(|fitting| fitting.handle == first)
            .unwrap();
        assert_eq!(
            record.ports[VALVE_PORT_NAMES[0]].as_deref(),
            Some(bound[0].handle.as_str()),
            "阀门记录指着连着它的那段"
        );
    }

    /// `PIDPIPE signal` 自动生成 `S-2001`，第二条是 `S-2002`。
    #[test]
    fn signal_lines_get_s_2001_and_above() {
        let mut app = app();
        draw(&mut app, "PIDPIPE signal", &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE signal", &[[0.0, 20.0, 0.0], [50.0, 20.0, 0.0]]);
        let tags: Vec<String> = handles(&app)
            .into_iter()
            .filter_map(|h| author_of(&app, h))
            .filter(|a| a.topology == "signal_line")
            .map(|a| a.tag)
            .collect();
        assert_eq!(tags, vec!["S-2001".to_string(), "S-2002".to_string()]);
    }

    /// 未定义符号被拒（`code=unknown_symbol`），不建任何实体。
    #[test]
    fn an_unknown_fitting_symbol_is_refused() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let before = handles(&app).len();
        let done = request(&mut app, json!({"op":"run","cmd":"PIDFITTING not_a_symbol"}));
        assert_eq!(done["ok"], false, "未知符号必须被拒：{done}");
        assert_eq!(done["code"], "unknown_symbol", "{done}");
        assert_eq!(handles(&app).len(), before, "不该新增实体");
        assert!(topology(&app).fittings.is_empty());
    }

    // ── 属性下传链（VAL-PIPE-007 .. 012）────────────────────────────────

    use crate::pid::propagate::{overrides_of, props_of, LINE_FIELDS};

    /// 五字段的一套值，按 `LINE_FIELDS` 顺序。
    fn five(grade: &str, diameter: &str, insulation: &str, medium: &str, kind: &str) -> Vec<String> {
        vec![
            grade.to_string(),
            diameter.to_string(),
            insulation.to_string(),
            medium.to_string(),
            kind.to_string(),
        ]
    }

    /// 某实体五字段的现值，按 `LINE_FIELDS` 顺序；缺键为空串。
    fn five_of(app: &OpenCADStudio, handle: document::Handle) -> Vec<String> {
        let props = props_of(&app.tabs[app.active_tab].scene.document, handle);
        LINE_FIELDS
            .iter()
            .map(|field| props.get(*field).cloned().unwrap_or_default())
            .collect()
    }

    fn marks_of(app: &OpenCADStudio, handle: document::Handle) -> Vec<String> {
        overrides_of(&app.tabs[app.active_tab].scene.document, handle)
            .into_iter()
            .collect()
    }

    /// 契约的属性写入口：`PUT /api/v1/entities/{h}/xdata/PID_AUTHOR`，
    /// 载荷只带 `props`（`VAL-PIPE-007` 的「等价 REST 通道」）。
    fn put_props(app: &mut OpenCADStudio, handle: document::Handle, pairs: &[(&str, &str)]) {
        let mut props = serde_json::Map::new();
        for (key, value) in pairs {
            props.insert((*key).to_string(), json!(value));
        }
        let payload = json!(serde_json::Value::Object(props)).to_string();
        let done = request(
            app,
            json!({
                "op":"xdata_set",
                "handles":[handle_text(handle)],
                "app":"PID_AUTHOR",
                "data":[{"code":1000,"value":format!("props={payload}")}],
            }),
        );
        assert_eq!(done["ok"], true, "写 props 应成功：{done}");
    }

    /// 画一条管线并返回它的 handle。
    fn draw_line(app: &mut OpenCADStudio) -> document::Handle {
        draw(app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let table = topology(app);
        parse_handle(&table.pipe_lines.last().expect("管线已建").handle).expect("handle")
    }

    /// 在最后一条管线上建一条管路，返回（管路 handle, 管段 handles）。
    fn draw_run(app: &mut OpenCADStudio) -> (document::Handle, Vec<document::Handle>) {
        draw(app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let table = topology(app);
        let run = table.pipe_runs.last().expect("管路已建");
        let run_handle = parse_handle(&run.handle).expect("handle");
        let segments = table
            .pipe_segments
            .iter()
            .filter(|segment| segment.run_tag == run.tag)
            .filter_map(|segment| parse_handle(&segment.handle))
            .collect();
        (run_handle, segments)
    }

    /// 协同：别人改了管线的字段，到这边只是「管线 L 的某字段 = 某值」这一处
    /// 变更。管路与管段怎样跟着变，是本图自己的事，同本地改完之后一样由下传
    /// 重算——管段没有位号，别人说不出它是哪一个，也不该替本图说。
    #[test]
    fn a_remote_line_change_is_carried_down_to_runs_and_segments() {
        let mut app = app();
        let line = draw_line(&mut app);
        let (run, segments) = draw_run(&mut app);
        assert!(!segments.is_empty(), "管路下应有管段");
        let line_tag = crate::pid::persist::PidAuthor::load(
            &app.tabs[app.active_tab].scene.document,
            line,
        )
        .expect("管线的作者记录")
        .tag;

        app.collab.apply(crate::app::collab::CollabEvent::OpApplied {
            op_id: "op-peer-line".to_owned(),
            revision: 1,
            changes: vec![json!({
                "kind": "set_property",
                "object_ref": line_tag,
                "field": "公称直径",
                "value": "DN80",
            })],
        });
        app.apply_remote_changes();

        let diameter = |app: &OpenCADStudio, handle: document::Handle| {
            props_of(&app.tabs[app.active_tab].scene.document, handle)
                .get("公称直径")
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(diameter(&app, line), "DN80", "管线自身");
        assert_eq!(diameter(&app, run), "DN80", "管路应跟随管线");
        for segment in &segments {
            assert_eq!(diameter(&app, *segment), "DN80", "管段应跟随管路");
        }
        assert_eq!(app.collab.last_apply_error, None);
    }

    /// 协同：一条拐了弯的管线是好几段实体，每段都带同一个位号。别人说的仍然
    /// 是「管线 L 的某字段 = 某值」，只有位号，没有句柄（不带图纸的客户端、
    /// 或者拿着另一份文件的客户端）。此前这种变更因为「带这个位号的实体不止
    /// 一个」整条不应用；它说的是这条管线，应该落到管线上并照常下传。
    #[test]
    fn a_remote_change_to_a_line_drawn_in_several_stretches_reaches_the_line() {
        let mut app = app();
        draw(
            &mut app,
            "PIDPIPE",
            &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0], [200.0, 100.0, 0.0]],
        );
        let table = topology(&app);
        let record = table.pipe_lines.last().expect("管线已建").clone();
        let line = parse_handle(&record.handle).expect("handle");
        let (run, segments) = draw_run(&mut app);
        let pieces: Vec<document::Handle> = {
            let document = &app.tabs[app.active_tab].scene.document;
            document
                .entities()
                .map(|entity| entity.common().handle)
                .filter(|handle| {
                    crate::pid::persist::PidAuthor::load(document, *handle)
                        .is_some_and(|author| author.tag == record.tag)
                })
                .collect()
        };
        assert_eq!(pieces.len(), 2, "三个点的管线是两段实体，都带管线位号");

        app.collab.apply(crate::app::collab::CollabEvent::OpApplied {
            op_id: "op-peer-line".to_owned(),
            revision: 1,
            changes: vec![json!({
                "kind": "set_property",
                "object_ref": record.tag,
                "field": "公称直径",
                "value": "DN80",
            })],
        });
        app.apply_remote_changes();

        assert_eq!(app.collab.last_apply_error, None);
        assert_eq!(app.collab.diverged, None);
        let diameter = |app: &OpenCADStudio, handle: document::Handle| {
            props_of(&app.tabs[app.active_tab].scene.document, handle)
                .get("公称直径")
                .cloned()
                .unwrap_or_default()
        };
        for piece in &pieces {
            assert_eq!(diameter(&app, *piece), "DN80", "管线的每一段");
        }
        assert_eq!(diameter(&app, line), "DN80", "拓扑表读值的那一段");
        assert_eq!(diameter(&app, run), "DN80", "管路应跟随管线");
        for segment in &segments {
            assert_eq!(diameter(&app, *segment), "DN80", "管段应跟随管路");
        }
    }

    /// 协同：撤销自己的一步，会把这一步碰过的实体放回当时的样子。管路和管段
    /// 的属性有一部分是从管线下传来的；别人后来改了管线的字段，它们「当时的
    /// 样子」里没有这个值。撤销之后要照管线现在的值重新下传，否则管路和管段
    /// 悄悄退回旧值，而管线上是新值。
    #[test]
    fn undoing_ones_own_run_edit_keeps_what_a_peer_later_set_on_the_line() {
        let mut app = app();
        let line = draw_line(&mut app);
        let (run, segments) = draw_run(&mut app);
        let line_tag = crate::pid::persist::PidAuthor::load(
            &app.tabs[app.active_tab].scene.document,
            line,
        )
        .expect("管线的作者记录")
        .tag;
        let value = |app: &OpenCADStudio, handle: document::Handle, key: &str| {
            props_of(&app.tabs[app.active_tab].scene.document, handle)
                .get(key)
                .cloned()
                .unwrap_or_default()
        };

        // 自己的一步：改管路自己的一个字段（整份 props 一起写回）。
        let mut mine = props_of(&app.tabs[app.active_tab].scene.document, run);
        mine.insert("绝热等级".to_string(), "H".to_string());
        let pairs: Vec<(&str, &str)> = mine
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        put_props(&mut app, run, &pairs);
        app.finish_all_pending_history();
        assert_eq!(value(&app, run, "绝热等级"), "H");

        // 别人随后改了管线的公称直径，下传到管路与管段。
        app.collab.apply(crate::app::collab::CollabEvent::OpApplied {
            op_id: "op-peer-line".to_owned(),
            revision: 1,
            changes: vec![json!({
                "kind": "set_property",
                "object_ref": line_tag,
                "field": "公称直径",
                "value": "DN80",
            })],
        });
        app.apply_remote_changes();
        assert_eq!(value(&app, run, "公称直径"), "DN80");

        let _ = app.automation_op(r#"{"op":"undo"}"#);
        assert_ne!(value(&app, run, "绝热等级"), "H", "自己的那一步撤回了");
        assert_eq!(value(&app, line, "公称直径"), "DN80", "管线上是别人的值");
        assert_eq!(value(&app, run, "公称直径"), "DN80", "管路仍跟随管线现在的值");
        for segment in &segments {
            assert_eq!(value(&app, *segment, "公称直径"), "DN80", "管段也是");
        }
    }

    /// `VAL-PIPE-007`：管路创建时全量继承管线五个字段，且不打覆盖标记。
    #[test]
    fn a_new_run_inherits_all_five_line_fields() {
        let mut app = app();
        let line = draw_line(&mut app);
        let expected = five("A1A", "DN50", "H", "工艺水", "工艺管线");
        put_props(
            &mut app,
            line,
            &[
                ("管道等级", "A1A"),
                ("公称直径", "DN50"),
                ("绝热等级", "H"),
                ("流体介质", "工艺水"),
                ("管线类型", "工艺管线"),
            ],
        );
        assert_eq!(five_of(&app, line), expected, "管线自身先要有五个值");

        let (run, segments) = draw_run(&mut app);
        assert_eq!(five_of(&app, run), expected, "管路必须全量继承管线五个字段");
        assert!(marks_of(&app, run).is_empty(), "继承写入不得打覆盖标记");
        let table = topology(&app);
        assert!(table.pipe_runs[0].overrides.is_empty(), "{:?}", table.pipe_runs[0]);
        // 管段同样取到这套值。
        for segment in &segments {
            assert_eq!(five_of(&app, *segment), expected);
        }
    }

    /// `VAL-PIPE-008`：管路手工改 `公称直径` 后打覆盖标记，且只标这一个。
    #[test]
    fn a_manual_run_edit_marks_the_field_as_overridden() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(
            &mut app,
            line,
            &[
                ("管道等级", "A1A"),
                ("公称直径", "DN50"),
                ("绝热等级", "H"),
                ("流体介质", "工艺水"),
                ("管线类型", "工艺管线"),
            ],
        );
        let (run, _) = draw_run(&mut app);
        assert!(marks_of(&app, run).is_empty());

        put_props(&mut app, run, &[("公称直径", "DN80")]);
        assert_eq!(five_of(&app, run)[1], "DN80", "手工值必须落地");
        assert_eq!(marks_of(&app, run), vec!["公称直径".to_string()], "只标被改的字段");
        // 其余四个字段原样保留且未被标记。
        let props = five_of(&app, run);
        assert_eq!(props[0], "A1A");
        assert_eq!(props[2], "H");
        assert_eq!(props[3], "工艺水");
        assert_eq!(props[4], "工艺管线");
        let table = topology(&app);
        assert_eq!(table.pipe_runs[0].overrides, vec!["公称直径".to_string()]);
        assert_eq!(table.pipe_runs[0].props.get("公称直径").map(String::as_str), Some("DN80"));
    }

    /// `VAL-PIPE-009`：已打标记的字段不被管线改值覆盖，未标记的仍下传。
    #[test]
    fn an_overridden_field_stops_taking_the_line_value() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(
            &mut app,
            line,
            &[("管道等级", "A1A"), ("公称直径", "DN50")],
        );
        let (run, _) = draw_run(&mut app);
        put_props(&mut app, run, &[("公称直径", "DN80")]);
        assert_eq!(marks_of(&app, run), vec!["公称直径".to_string()]);

        put_props(&mut app, line, &[("公称直径", "DN150"), ("管道等级", "A2B")]);
        assert_eq!(five_of(&app, run)[1], "DN80", "已覆盖字段保持手工值");
        assert_eq!(five_of(&app, run)[0], "A2B", "未覆盖字段仍下传");
        assert_eq!(marks_of(&app, run), vec!["公称直径".to_string()], "不新增标记");
        assert_eq!(five_of(&app, line)[1], "DN150", "管线自身改成了 DN150");
    }

    /// `VAL-PIPE-010`：管段始终取管路当前值，且管段没有覆盖标记。
    #[test]
    fn a_segment_follows_its_run_and_carries_no_override_mark() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(&mut app, line, &[("公称直径", "DN50")]);
        let (run, segments) = draw_run(&mut app);
        put_props(&mut app, run, &[("公称直径", "DN80")]);
        assert_eq!(five_of(&app, run)[1], "DN80");
        for segment in &segments {
            assert_eq!(five_of(&app, *segment)[1], "DN80", "管段先跟随到 DN80");
        }

        put_props(&mut app, run, &[("公称直径", "DN100")]);
        assert_eq!(five_of(&app, run)[1], "DN100");
        for segment in &segments {
            assert_eq!(five_of(&app, *segment)[1], "DN100", "管段跟随管路到 DN100");
            assert!(marks_of(&app, *segment).is_empty(), "管段无覆盖标记机制");
            let author = author_of(&app, *segment).expect("管段带 PID_AUTHOR");
            assert!(
                !author.props.contains(crate::pid::propagate::OVERRIDES_KEY),
                "管段 props 不得出现 __overrides：{}",
                author.props
            );
        }
        let table = topology(&app);
        for segment in &table.pipe_segments {
            assert!(
                !segment.handle.is_empty() && !table.pipe_segments.is_empty(),
                "管段记录仍在表里"
            );
        }
        assert!(
            !serde_json::to_string(&table.pipe_segments[0])
                .unwrap()
                .contains("overrides"),
            "拓扑表管段记录无 overrides 字段"
        );
    }

    /// `VAL-PIPE-011`：管段不直连管线。管路有覆盖标记后，管线改值不能穿透。
    #[test]
    fn a_line_edit_never_reaches_a_segment_behind_an_overridden_run() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(&mut app, line, &[("管道等级", "A1A")]);
        let (run, segments) = draw_run(&mut app);
        let segment = segments[0];
        // 九宫格起点：三层都是 A1A。
        assert_eq!(five_of(&app, line)[0], "A1A");
        assert_eq!(five_of(&app, run)[0], "A1A");
        assert_eq!(five_of(&app, segment)[0], "A1A");

        // (1) 管路手工改 A3C：产生覆盖标记，管段取管路当前值。
        put_props(&mut app, run, &[("管道等级", "A3C")]);
        assert_eq!(five_of(&app, run)[0], "A3C");
        assert_eq!(five_of(&app, segment)[0], "A3C", "管段取管路当前值");
        assert_eq!(marks_of(&app, run), vec!["管道等级".to_string()]);

        // (2) 管线改 A9Z：管路被覆盖标记挡住，管段**不得**跳到 A9Z。
        put_props(&mut app, line, &[("管道等级", "A9Z")]);
        assert_eq!(five_of(&app, line)[0], "A9Z");
        assert_eq!(five_of(&app, run)[0], "A3C", "覆盖标记挡住管线改值");
        assert_eq!(five_of(&app, segment)[0], "A3C", "管段不直连管线，仍取管路值");
    }

    /// `VAL-PIPE-012`：新建管路继承的是管线**当前**值，不是创建初值。
    #[test]
    fn a_late_run_inherits_the_lines_current_value() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(&mut app, line, &[("管道等级", "A1A")]);
        // 建管路之前把管线改成 A5E。
        put_props(&mut app, line, &[("管道等级", "A5E")]);
        assert!(topology(&app).pipe_runs.is_empty(), "此时还没有任何管路");

        let (run, segments) = draw_run(&mut app);
        assert_eq!(five_of(&app, run)[0], "A5E", "继承管线当前值，不是 A1A");
        assert!(marks_of(&app, run).is_empty(), "继承不打标记");
        assert_eq!(five_of(&app, segments[0])[0], "A5E");

        // 继承之后仍受下传约束。
        put_props(&mut app, line, &[("管道等级", "A6F")]);
        assert_eq!(five_of(&app, run)[0], "A6F", "未覆盖字段跟随管线");
        assert_eq!(five_of(&app, segments[0])[0], "A6F", "管段跟随管路");
    }

    // ── 打断符（VAL-PIPE-031 / 032）────────────────────────────────────

    /// `VAL-PIPE-031`：`PIDBREAK` 在 `(0,0)-(200,0)` 的中点打断后，管段数
    /// 从 1 变 2、沿 X 区间不重叠、打断符是独立实体且 `topology=fitting`、
    /// 原管段 handle 作为第一段保留、`pipe_segments` 与 `pipe_runs[0].segments`
    /// 的 handle 集合一致。
    #[test]
    fn pidbreak_splits_the_segment_into_two_non_overlapping_pieces() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let before = topology(&app);
        assert_eq!(before.pipe_segments.len(), 1, "前置：1 个管段");
        let original = before.pipe_segments[0].handle.clone();

        let done = draw(&mut app, "PIDBREAK", &[[100.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");
        assert_eq!(done["status"], "completed", "{done}");

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 2, "管段被分为两段");

        // 原管段 handle 作为第一段保留。
        assert_eq!(table.pipe_segments[0].handle, original, "原管段 handle 作为第一段");

        // 沿 X 区间不重叠。
        let a = &table.pipe_segments[0];
        let b = &table.pipe_segments[1];
        assert_eq!(a.start.point, [0.0, 0.0], "第一段起点不动");
        assert_eq!(b.end.point, [200.0, 0.0], "第二段终点不动");
        assert!(
            a.end.point[0] < b.start.point[0],
            "两段沿 X 必须严格不重叠：{a:?} vs {b:?}"
        );
        // 长度守恒：两段之和 + 缝隙 == 原长。
        let covered = (a.end.point[0] - a.start.point[0]) + (b.end.point[0] - b.start.point[0]);
        let gap = b.start.point[0] - a.end.point[0];
        assert!(
            (covered + gap - 200.0).abs() <= 1e-6,
            "长度守恒失败：covered={covered} gap={gap}"
        );

        // 打断符是独立实体，`topology=fitting`。
        let fitting = &table.fittings[0];
        assert_eq!(fitting.fitting_type, crate::pid::topology::FITTING_BREAK);
        assert_eq!(fitting.run_tag, "L-1001-01");
        assert!(fitting.tag.is_none(), "打断符无独立位号");
        assert!(fitting.ports.is_empty(), "打断符不绑定管段端口");
        let break_handle = parse_handle(&fitting.handle).expect("handle");
        let author = author_of(&app, break_handle).expect("打断符带 PID_AUTHOR");
        assert_eq!(author.topology, "fitting");
        assert!(handles(&app).contains(&break_handle), "打断符是文档里的独立实体");

        // `pipe_segments` 与 `pipe_runs[0].segments` 的 handle 集合一致。
        let mut from_segments: Vec<String> =
            table.pipe_segments.iter().map(|s| s.handle.clone()).collect();
        let mut from_run: Vec<String> = table.pipe_runs[0].segments.clone();
        from_segments.sort();
        from_run.sort();
        assert_eq!(from_segments, from_run, "两处 handle 集合必须一致");
        // 无悬空引用：每个 handle 在文档里都存在。
        let live: Vec<String> = handles(&app).iter().map(|h| handle_text(*h)).collect();
        for text in &from_segments {
            assert!(live.contains(text), "悬空引用：{text}");
        }
    }

    /// `VAL-PIPE-032`：打断后两段**不连通** —— 段 A 的 `end.peer` 与段 B 的
    /// `start.peer` 都是 `null`，且互不指向对方；打断符不出现在任何 `peer` 里。
    #[test]
    fn pidbreak_leaves_both_inner_ends_disconnected() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDBREAK", &[[100.0, 0.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 2);
        let a = &table.pipe_segments[0];
        let b = &table.pipe_segments[1];
        assert_eq!(a.end.peer, None, "段 A 的 end.peer 必须为 null");
        assert_eq!(b.start.peer, None, "段 B 的 start.peer 必须为 null");

        // 互不指向：任何一个 peer 都不指向另一段的 handle。
        let a_handle = a.handle.clone();
        let b_handle = b.handle.clone();
        for segment in &table.pipe_segments {
            for end in [&segment.start, &segment.end] {
                if let Some(peer) = &end.peer {
                    assert!(
                        !peer.starts_with(&a_handle) && !peer.starts_with(&b_handle),
                        "打断后两段不得互指：{peer}"
                    );
                }
            }
        }

        // 打断符自身不出现在任何管段的 peer 里（隔断而非导通）。
        let break_handle = table.fittings[0].handle.clone();
        for segment in &table.pipe_segments {
            for end in [&segment.start, &segment.end] {
                assert!(
                    end.peer
                        .as_deref()
                        .is_none_or(|peer| !peer.starts_with(&break_handle)),
                    "打断符不得出现在任何管段的 peer 里：{:?}",
                    end.peer
                );
            }
        }
        assert_eq!(table.fittings.len(), 1, "打断符在 fittings 里有一条记录");
    }

    /// 在终点接着阀门的管段上打断：第二段接过原管段的终点，连同 `peer`；
    /// 阀门记录里那个端口改指第二段。
    #[test]
    fn pidbreak_keeps_the_far_end_bound_to_its_fitting() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(
            &mut app,
            "PIDPIPE run",
            &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]],
        );
        draw(&mut app, "PIDFITTING gate_valve", &[[150.0, 0.0, 0.0]]);
        let valve = topology(&app).fittings[0].handle.clone();
        let done = draw(&mut app, "PIDBREAK", &[[50.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], true, "{done}");

        let table = topology(&app);
        let peer = format!("{valve}:{}", VALVE_PORT_NAMES[0]);
        let bound: Vec<_> = table
            .pipe_segments
            .iter()
            .filter(|segment| segment.end.peer.as_deref() == Some(peer.as_str()))
            .collect();
        assert_eq!(bound.len(), 1, "阀门入口还有一段连着：{table:?}");
        assert_eq!(bound[0].end.point, [134.0, 0.0], "终点还是原管段的终点");
        assert_eq!(bound[0].start.peer, None, "打断那一侧不连");
        let record = table
            .fittings
            .iter()
            .find(|fitting| fitting.handle == valve)
            .unwrap();
        assert_eq!(
            record.ports[VALVE_PORT_NAMES[0]].as_deref(),
            Some(bound[0].handle.as_str()),
            "阀门记录指着连着它的那段"
        );
    }

    /// 打断点不在任何管段上时拒绝，且不新增实体、不动拓扑表。
    #[test]
    fn pidbreak_off_the_pipe_is_refused_without_changes() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let before_handles = handles(&app).len();
        let before = topology(&app);

        let done = draw(&mut app, "PIDBREAK", &[[0.0, 500.0, 0.0]]);
        assert_eq!(done["ok"], false, "落点不在管段上必须被拒：{done}");
        assert_eq!(done["code"], BREAK_NO_SEGMENT, "{done}");
        assert_eq!(handles(&app).len(), before_handles, "不该新增实体");
        assert_eq!(topology(&app), before, "拓扑表不得变化");
    }

    /// 打断点落在管段端点上时拒绝（否则会留下零长度管段）。
    #[test]
    fn pidbreak_on_a_segment_endpoint_is_refused() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let before = topology(&app);

        let done = draw(&mut app, "PIDBREAK", &[[0.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], false, "端点打断必须被拒：{done}");
        assert_eq!(done["code"], BREAK_AT_ENDPOINT, "{done}");
        assert_eq!(topology(&app), before, "拓扑表不得变化");
    }

    /// 打断点离端点不足让开距离时拒绝，且不动拓扑表。
    #[test]
    fn pidbreak_too_close_to_an_endpoint_is_refused() {
        let mut app = app();
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(&mut app, "PIDPIPE run", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let before = topology(&app);

        let done = draw(&mut app, "PIDBREAK", &[[1.0, 0.0, 0.0]]);
        assert_eq!(done["ok"], false, "太靠近端点必须被拒：{done}");
        assert_eq!(done["code"], BREAK_TOO_CLOSE, "{done}");
        assert_eq!(topology(&app), before, "拓扑表不得变化");
    }

    /// 打断后两段继承原管段的 `props` / `owner`（管段无覆盖标记）。
    #[test]
    fn both_pieces_of_a_break_keep_the_run_props_and_owner() {
        let mut app = app();
        let line = draw_line(&mut app);
        put_props(&mut app, line, &[("管道等级", "A1A")]);
        let (run, _segments) = draw_run(&mut app);
        draw(&mut app, "PIDBREAK", &[[100.0, 0.0, 0.0]]);

        let table = topology(&app);
        assert_eq!(table.pipe_segments.len(), 2);
        for segment in &table.pipe_segments {
            let handle = parse_handle(&segment.handle).expect("handle");
            assert_eq!(five_of(&app, handle)[0], "A1A", "两段都继承管路等级");
            let author = author_of(&app, handle).expect("带 PID_AUTHOR");
            assert_eq!(author.topology, "pipe_segment");
            assert_eq!(
                author.owner,
                handle_text(run),
                "两段的 owner 都指向宿主管路"
            );
            assert!(
                !author.props.contains("__overrides"),
                "管段无覆盖标记：{}",
                author.props
            );
        }
    }
}

//! `PIDCONNECTOR` / `PIDPAIR` —— 页间连接符（管道/信号线 × 流入/流出）。
//!
//! ```text
//! PIDCONNECTOR <pipe_in|pipe_out|signal_in|signal_out> [<drawing_id>:<handle> | <目标图号>]
//! PIDPAIR <本图 handle> <对端 drawing_id>:<handle>
//! ```
//!
//! 第二个参数带冒号时是本进程另一张图上的对端（`<document_id>:<handle>`）；
//! 不带冒号时是协同里的目标图号——另一端在服务器的哪张图上，由服务器配对
//! （`src/app/collab/connectors.rs`，`VAL-CROSS-009`）。
//!
//! 四条硬约束（`architecture.md` 6.5 / `validation-contract.md` E 区）：
//!
//! 1. **四种 `kind` 可区分**：`topology` 只有 `connector_in` / `connector_out`
//!    两个取值，管道与信号线靠实体 XDATA 的 `type` 键区分
//!    （`PIPE_LINE` vs `SIGNAL_LINE`）—— `VAL-PIPE-023` 要求存在可读字段把
//!    四者分开，做不到就 FAIL。
//! 2. **同一线号同图至多一个流入连接符**，第 2 个返回
//!    [`DUPLICATE_INFLOW_CONNECTOR`]（HTTP 409）。
//! 3. **管道连接符选到信号线上**（及反向）返回
//!    [`CONNECTOR_KIND_MISMATCH`]（HTTP 400）。
//! 4. **两种 `peer` 不混用**：连接符的 `peer` 与 XDATA `connector_peer` 是
//!    `<drawing_id>:<handle>`（跨图）；管段端点的 `peer` 是
//!    `<本图 handle>:<端口名>`（同图）。
//!
//! 校验全部在任何写入之前完成 —— 被拒时实体数与拓扑表逐字节不变
//! （`VAL-PIPE-024` / `025`）。

use super::*;
use crate::command::{CadCommand, CmdResult, InputKind};
use crate::pid::persist::PidAuthor;
use crate::pid::topology::{
    handle_text, parse_handle, parse_peer, peer_text, ConnectorKind, ConnectorRecord,
    ConnectorRequest, PipeCommit, TopologyTable, CONNECTOR_KIND_MISMATCH,
    CONNECTOR_NO_LINE, CONNECTOR_PEER_INVALID, DUPLICATE_INFLOW_CONNECTOR,
};
use glam::DVec3;
use serde_json::json;

use crate::scene::ChangeKind;

/// 连接符实体的示意圆半径（只作可见标记，不参与拓扑判定）。
const CONNECTOR_RADIUS: f64 = 3.0;

/// 落点到线体的命中容差（世界单位）。P&ID 符号半宽 14，取 5 足够宽容又不
/// 会跨线误命中。
const CONNECTOR_PICK_TOLERANCE: f64 = 5.0;

/// 管道连接符下传的五个字段。
const PIPE_FIELDS: [&str; 5] = crate::pid::propagate::LINE_FIELDS;

/// 信号线连接符下传的两个字段。**不含 `管道等级`**（`VAL-PIPE-027`）。
/// 与协同服务器沿连接符跨图传递时用的是同一份清单。
const SIGNAL_FIELDS: [&str; 2] = ocs_collab::link::SIGNAL_FIELDS;

// ── 命令：PIDCONNECTOR ───────────────────────────────────────────────────

/// `PIDCONNECTOR <kind> [<peer>]`：在命中的管线/信号线上插页间连接符
/// （一点 + Enter）。
pub struct PidConnectorCommand {
    kind: ConnectorKind,
    peer: Option<String>,
    point: Option<DVec3>,
}

impl PidConnectorCommand {
    /// 由种类与可选对端构造。
    pub fn new(kind: ConnectorKind, peer: Option<String>) -> Self {
        Self {
            kind,
            peer,
            point: None,
        }
    }
}

impl CadCommand for PidConnectorCommand {
    fn name(&self) -> &'static str {
        "PIDCONNECTOR"
    }

    fn tool_line(&self) -> Option<String> {
        Some(format!("PIDCONNECTOR {}", self.kind.as_str()))
    }

    fn prompt(&self) -> String {
        format!(
            "PIDCONNECTOR {}  在管线/信号线上指定插入点:",
            self.kind.as_str()
        )
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
                "PIDCONNECTOR: 需要先指定插入点。".to_string(),
            );
        };
        CmdResult::PidPipeCommit(Box::new(PipeCommit::Connector(ConnectorRequest {
            kind: self.kind,
            point: [point.x, point.y, point.z],
            peer: self.peer.clone(),
        })))
    }
}

// ── 落点命中 ─────────────────────────────────────────────────────────────

/// 落点命中的线体。
struct LineHit {
    /// 该线体的位号（管线的 `L-…` 或信号线的 `S-…`）。
    tag: String,
    /// `pipe_line` 或 `signal_line`。
    role: String,
}

/// 找落点附近最近的一条管线 / 信号线。
///
/// 判据是实体的 `PID_AUTHOR` 记录：`topology` 为 `pipe_line` 或
/// `signal_line` 的 `Line` 实体。两者都不进 `pipe_segments`，所以不能靠
/// 拓扑表反查。
fn find_line_at(doc: &document::Document, point: DVec3) -> Option<LineHit> {
    let mut best: Option<(f64, LineHit)> = None;
    for entity in doc.entities() {
        let document::EntityType::Line(line) = entity else {
            continue;
        };
        let handle = entity.common().handle;
        let Some(author) = PidAuthor::load(doc, handle) else {
            continue;
        };
        let role = author.topology.as_str();
        if role != crate::pid::topology::ROLE_PIPE_LINE
            && role != crate::pid::topology::ROLE_SIGNAL_LINE
        {
            continue;
        }
        let a = DVec3::new(line.start.x, line.start.y, 0.0);
        let b = DVec3::new(line.end.x, line.end.y, 0.0);
        let distance = distance_to_segment(a, b, point);
        if best.as_ref().is_some_and(|(d, _)| distance >= *d) {
            continue;
        }
        best = Some((
            distance,
            LineHit {
                tag: author.tag.clone(),
                role: role.to_string(),
            },
        ));
    }
    best.filter(|(distance, _)| *distance <= CONNECTOR_PICK_TOLERANCE)
        .map(|(_, hit)| hit)
}

/// 点到线段的距离（平面）。
fn distance_to_segment(a: DVec3, b: DVec3, p: DVec3) -> f64 {
    let ab = b - a;
    let length_sq = ab.length_squared();
    if length_sq <= 1e-12 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    (a + ab * t - p).length()
}

// ── 宿主侧：提交 ─────────────────────────────────────────────────────────

impl OpenCADStudio {
    /// `PIDCONNECTOR` 的落盘。
    ///
    /// 校验（落点命中 → 种类匹配 → 流入唯一）全部在任何写入之前完成，
    /// 被拒时实体数与拓扑表不变。
    pub(in crate::app) fn commit_pid_connector(
        &mut self,
        i: usize,
        request: ConnectorRequest,
    ) -> Option<String> {
        if self.tabs[i].is_start {
            return None;
        }
        let point = DVec3::new(request.point[0], request.point[1], request.point[2]);
        let document = &self.tabs[i].scene.document;

        let Some(hit) = find_line_at(document, point) else {
            self.command_line.push_error_coded(
                CONNECTOR_NO_LINE,
                "PIDCONNECTOR: 落点不在任何管线或信号线上。",
            );
            return None;
        };
        // 管道连接符必须挂在 pipe_line 上，信号线连接符必须挂在 signal_line
        // 上；交叉即 `CONNECTOR_KIND_MISMATCH`（`VAL-PIPE-025` 两个方向）。
        if hit.role != request.kind.host_role() {
            let wanted = if request.kind.host_role() == crate::pid::topology::ROLE_PIPE_LINE {
                "管线"
            } else {
                "信号线"
            };
            let msg = format!(
                "PIDCONNECTOR {}: 该连接符只能插在{wanted}上，落点命中 {}。",
                request.kind.as_str(),
                hit.tag
            );
            self.command_line
                .push_error_coded(CONNECTOR_KIND_MISMATCH, &msg);
            self.collab.push_toast(
                crate::app::collab::toasts::ToastKind::Error,
                format!("页间连接符种类与线不符：{msg}"),
            );
            return None;
        }

        let mut table = TopologyTable::load(document);
        table.prune(document);
        if request.kind.is_inflow() && table.has_inflow_connector(&hit.tag) {
            let msg = format!(
                "PIDCONNECTOR {}: {} 上已有流入连接符，同一线号同图至多一个。",
                request.kind.as_str(),
                hit.tag
            );
            self.command_line
                .push_error_coded(DUPLICATE_INFLOW_CONNECTOR, &msg);
            self.collab.push_toast(
                crate::app::collab::toasts::ToastKind::Error,
                format!("页间连接符重复流入：{msg}"),
            );
            return None;
        }

        // 第二个参数有两种读法。带冒号的是本进程里另一张图上的对端
        // `<document_id>:<handle>`，照旧校验、创建即配对。不带冒号的是
        // **目标图号**：另一端在协同服务器的哪张图上（`VAL-CROSS-009`）——
        // 那张图不在本进程里，配对由服务器做，这里只把图号记在连接符上，
        // 随 `create_connector` 说给房间。
        let (peer, target_drawing) = match request.peer.as_deref().map(str::trim) {
            Some(text) if text.contains(':') => (Some(text.to_string()), None),
            Some(text) if !text.is_empty() => (None, Some(text.to_string())),
            _ => (None, None),
        };

        // 对端格式与可配对性在**写入之前**校验：被拒时实体数不变
        // （`VAL-PIPE-024` / `025` 的「实体数不变」判据同样适用于这里）。
        if let Some(peer) = peer.as_deref() {
            if let Err((code, message)) = self.precheck_peer(i, request.kind, peer) {
                self.command_line.push_error_coded(code, &message);
                return None;
            }
        }

        self.push_undo_snapshot(i, "PIDCONNECTOR");
        let mut circle = document::entities::Circle::new();
        circle.center = document::types::Vector3::new(point.x, point.y, point.z);
        circle.radius = CONNECTOR_RADIUS;
        let handle = self.tabs[i]
            .scene
            .add_entity(document::EntityType::Circle(circle));

        // `type` 键是四者可区分性的唯一载体（`VAL-PIPE-023`）：管道两枚
        // 落 `PIPE_LINE`，信号线两枚落 `SIGNAL_LINE`；`topology` 只分流入/流出。
        PidAuthor {
            type_code: request.kind.type_code().to_string(),
            tag: String::new(),
            tag_source: "auto".to_string(),
            tag_status: "ok".to_string(),
            topology: request.kind.role().to_string(),
            connector_peer: peer.clone().unwrap_or_default(),
            props: "{}".to_string(),
            ..PidAuthor::default()
        }
        .store(&mut self.tabs[i].scene.document, handle);

        let peer_drawing = peer
            .as_deref()
            .and_then(parse_peer)
            .map(|(drawing, _)| drawing.to_string())
            // 没有同进程对端时，记的是目标图号（服务器配上之后会再写一遍）。
            .or(target_drawing);
        let mut record = ConnectorRecord::new(
            handle_text(handle),
            request.kind,
            hit.tag.clone(),
        );
        record.peer = peer.clone();
        record.peer_drawing = peer_drawing;
        // 「连接管路头尾信息」：本侧宿主线的位号，配对后再补对端。
        record.upstream = Some(hit.tag.clone());
        table.upsert_connector(record);
        table.store(&mut self.tabs[i].scene.document);

        self.tabs[i]
            .scene
            .bump_entities(&[(handle, ChangeKind::Modified)]);
        self.tabs[i].dirty = true;
        self.post_ref_op(i);
        self.set_control_result(json!({
            "added": 1,
            "handle": handle_text(handle),
            "kind": request.kind.as_str(),
            "line_tag": hit.tag,
        }));

        // 创建即配对（`PIDCONNECTOR <kind> <peer>`）：写对端并传递属性。
        if let Some(peer) = peer {
            if let Err((code, message)) = self.pair_connectors(i, handle, &peer) {
                self.command_line.push_error_coded(code, &message);
                return None;
            }
        }

        Some(format!(
            "PIDCONNECTOR: {} 连接符 {} 插在 {} 上",
            request.kind.as_str(),
            handle_text(handle),
            hit.tag
        ))
    }

    /// 创建前的对端校验：格式合法、且（对端图纸在本进程时）能配成一对。
    ///
    /// 只读，不写任何字节 —— 被拒时实体数与拓扑表都不变。
    fn precheck_peer(
        &self,
        i: usize,
        kind: ConnectorKind,
        peer: &str,
    ) -> Result<(), (&'static str, String)> {
        let Some((peer_drawing, peer_handle)) = parse_peer(peer) else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDCONNECTOR: 对端 \"{peer}\" 不是 <drawing_id>:<handle>。"),
            ));
        };
        let Some(j) = self.tabs.iter().position(|tab| tab.id == peer_drawing) else {
            return Ok(());
        };
        let peer_table = TopologyTable::load(&self.tabs[j].scene.document);
        let Some(theirs) = peer_table.connector_by_handle(peer_handle) else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDCONNECTOR: 对端图纸里没有连接符 {}", handle_text(peer_handle)),
            ));
        };
        let Some(their_kind) = theirs.kind_of() else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDCONNECTOR: 对端连接符的 kind 无法识别：{}", theirs.kind),
            ));
        };
        if kind.is_inflow() == their_kind.is_inflow()
            || kind.host_role() != their_kind.host_role()
        {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!(
                    "PIDCONNECTOR: {} 与 {} 不能配对（需一入一出且同族）。",
                    kind.as_str(),
                    their_kind.as_str()
                ),
            ));
        }
        let _ = i;
        Ok(())
    }

    /// 把本图 `handle` 连接符与对端 `peer`（`<drawing_id>:<handle>`）配对：
    /// 两侧写 `peer` / `peer_drawing` / `upstream` / `downstream`，并把流出侧
    /// 宿主线的属性传到流入侧（`VAL-PIPE-027`）。
    ///
    /// 对端不在本进程的标签页里时只写本侧（跨进程对端无从访问），不报错。
    pub(in crate::app) fn pair_connectors(
        &mut self,
        i: usize,
        handle: document::Handle,
        peer: &str,
    ) -> Result<(), (&'static str, String)> {
        let Some((peer_drawing, peer_handle)) = parse_peer(peer) else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDPAIR: 对端 \"{peer}\" 不是 <drawing_id>:<handle>。"),
            ));
        };
        let Some(j) = self.tabs.iter().position(|tab| tab.id == peer_drawing) else {
            // 对端图纸不在本进程：本侧只记对端地址。
            let mut table = TopologyTable::load(&self.tabs[i].scene.document);
            if let Some(record) = table.connector_by_handle_mut(handle) {
                record.peer = Some(peer.to_string());
                record.peer_drawing = Some(peer_drawing.to_string());
            }
            table.store(&mut self.tabs[i].scene.document);
            return Ok(());
        };

        let mut table = TopologyTable::load(&self.tabs[i].scene.document);
        let Some(mine) = table.connector_by_handle(handle).cloned() else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDPAIR: {} 不是拓扑表里的连接符。", handle_text(handle)),
            ));
        };
        let Some(my_kind) = mine.kind_of() else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDPAIR: {} 的 kind 无法识别。", mine.kind),
            ));
        };

        let mut peer_table = TopologyTable::load(&self.tabs[j].scene.document);
        let Some(theirs) = peer_table.connector_by_handle(peer_handle).cloned() else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDPAIR: 对端图纸里没有连接符 {}", handle_text(peer_handle)),
            ));
        };
        let Some(their_kind) = theirs.kind_of() else {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!("PIDPAIR: 对端连接符的 kind 无法识别：{}", theirs.kind),
            ));
        };

        // 必须是「一入一出」且同族（都管道或都信号线）。都流入/都流出、
        // 或管道对信号线都不构成一对。
        if my_kind.is_inflow() == their_kind.is_inflow()
            || my_kind.host_role() != their_kind.host_role()
        {
            return Err((
                CONNECTOR_PEER_INVALID,
                format!(
                    "PIDPAIR: {} 与 {} 不能配对（需一入一出且同族）。",
                    my_kind.as_str(),
                    their_kind.as_str()
                ),
            ));
        }

        // 源 = 流出侧，目标 = 流入侧。
        let (source_i, source_line, target_i, target_line) = if my_kind.is_inflow() {
            (j, theirs.line_tag.clone(), i, mine.line_tag.clone())
        } else {
            (i, mine.line_tag.clone(), j, theirs.line_tag.clone())
        };

        // 本侧记录写对端；对端记录反向写本侧。
        let (my_peer, my_peer_drawing, my_up, my_down) = if my_kind.is_inflow() {
            (
                peer.to_string(),
                peer_drawing.to_string(),
                theirs.line_tag.clone(),
                mine.line_tag.clone(),
            )
        } else {
            (
                peer.to_string(),
                peer_drawing.to_string(),
                mine.line_tag.clone(),
                theirs.line_tag.clone(),
            )
        };
        if let Some(record) = table.connector_by_handle_mut(handle) {
            record.peer = Some(my_peer);
            record.peer_drawing = Some(my_peer_drawing);
            record.upstream = Some(my_up);
            record.downstream = Some(my_down);
        }
        table.store(&mut self.tabs[i].scene.document);

        let back = peer_text(self.tabs[i].id, handle);
        let their_up = if their_kind.is_inflow() {
            source_line.clone()
        } else {
            target_line.clone()
        };
        let their_down = if their_kind.is_inflow() {
            target_line.clone()
        } else {
            source_line.clone()
        };
        if let Some(record) = peer_table.connector_by_handle_mut(peer_handle) {
            record.peer = Some(back.clone());
            record.peer_drawing = Some(self.tabs[i].id.to_string());
            record.upstream = Some(their_up);
            record.downstream = Some(their_down);
        }
        peer_table.store(&mut self.tabs[j].scene.document);

        // 两侧 XDATA 的 `connector_peer` 与拓扑表同步（`VAL-PIPE-026` 要求
        // 两者一致）：本侧写对端地址，对侧写回本侧地址。
        self.write_connector_peer(i, handle, peer);
        if j != i {
            self.write_connector_peer(j, peer_handle, &back);
        }

        // 属性传递：流出侧宿主线的字段 → 流入侧宿主线。
        self.transfer_connector_props(
            source_i,
            source_line,
            target_i,
            target_line,
            their_kind.host_role() == crate::pid::topology::ROLE_SIGNAL_LINE,
        );
        Ok(())
    }

    /// 给一个连接符实体写 `connector_peer` XDATA（只改该键）。
    fn write_connector_peer(&mut self, i: usize, handle: document::Handle, peer: &str) {
        let Some(mut author) = PidAuthor::load(&self.tabs[i].scene.document, handle) else {
            return;
        };
        if author.connector_peer == peer {
            return;
        }
        author.connector_peer = peer.to_string();
        author.store(&mut self.tabs[i].scene.document, handle);
        self.tabs[i]
            .scene
            .bump_entities(&[(handle, ChangeKind::Modified)]);
        self.tabs[i].dirty = true;
    }

    /// 把 `source_i` 图纸 `source_line` 线体的属性复制到 `target_i` 图纸
    /// `target_line` 线体，并让目标侧下传链跟随。
    ///
    /// 管道传五个下传字段；信号线只传 `起点仪表` / `终点仪表`，**不传
    /// `管道等级`**（`VAL-PIPE-027`）。
    fn transfer_connector_props(
        &mut self,
        source_i: usize,
        source_line: String,
        target_i: usize,
        target_line: String,
        signal: bool,
    ) {
        let Some(source_handle) =
            line_entity_of_tag(&self.tabs[source_i].scene.document, &source_line)
        else {
            return;
        };
        let Some(target_handle) =
            line_entity_of_tag(&self.tabs[target_i].scene.document, &target_line)
        else {
            return;
        };
        let source_props = crate::pid::propagate::props_of(
            &self.tabs[source_i].scene.document,
            source_handle,
        );
        let fields: &[&str] = if signal { &SIGNAL_FIELDS } else { &PIPE_FIELDS };
        let mut pairs: Vec<(String, String)> = Vec::new();
        for field in fields {
            if let Some(value) = source_props.get(*field) {
                if value.is_empty() {
                    continue;
                }
                pairs.push(((*field).to_string(), value.clone()));
            }
        }
        if pairs.is_empty() {
            return;
        }
        if target_i != source_i {
            self.push_undo_snapshot(target_i, "PIDPAIR");
        }
        let Some(mut author) = PidAuthor::load(&self.tabs[target_i].scene.document, target_handle)
        else {
            return;
        };
        let (mut props, marks) = crate::pid::propagate::split_props(&author.props);
        for (key, value) in pairs {
            props.insert(key, value);
        }
        author.props = crate::pid::propagate::join_props(&props, &marks);
        author.store(&mut self.tabs[target_i].scene.document, target_handle);
        self.tabs[target_i]
            .scene
            .bump_entities(&[(target_handle, ChangeKind::Modified)]);
        self.tabs[target_i].dirty = true;
        // 目标侧管路/管段跟随（唯一重算入口）。
        self.propagate_pid_chain(target_i, &[target_handle]);
    }

    /// `PIDPAIR`：事后配对两个已存在的连接符。
    fn control_pid_pair(&mut self, i: usize, handle: document::Handle, peer: &str) -> Option<String> {
        self.push_undo_snapshot(i, "PIDPAIR");
        match self.pair_connectors(i, handle, peer) {
            Ok(()) => Some(format!(
                "PIDPAIR: {} ↔ {peer}",
                handle_text(handle)
            )),
            Err((code, message)) => {
                self.discard_last_undo_entry(i);
                self.command_line.push_error_coded(code, &message);
                None
            }
        }
    }

    /// `xdata_set` 写了连接符的 `connector_peer` 之后，把拓扑表与属性传递
    /// 一起对齐。非连接符实体一律跳过。
    pub(in crate::app) fn sync_connectors_after_xdata(&mut self, i: usize, handles: &[document::Handle]) {
        for &handle in handles {
            let Some(author) = PidAuthor::load(&self.tabs[i].scene.document, handle) else {
                continue;
            };
            if author.connector_peer.is_empty() {
                continue;
            }
            let table = TopologyTable::load(&self.tabs[i].scene.document);
            if table.connector_by_handle(handle).is_none() {
                continue;
            }
            let peer = author.connector_peer.clone();
            let _ = self.pair_connectors(i, handle, &peer);
        }
    }
}

/// 按位号找本图里承载该位号的线体实体（管线或信号线）。
fn line_entity_of_tag(doc: &document::Document, tag: &str) -> Option<document::Handle> {
    doc.entities().find_map(|entity| {
        let document::EntityType::Line(_) = entity else {
            return None;
        };
        let handle = entity.common().handle;
        let author = PidAuthor::load(doc, handle)?;
        let role = author.topology.as_str();
        if role != crate::pid::topology::ROLE_PIPE_LINE
            && role != crate::pid::topology::ROLE_SIGNAL_LINE
        {
            return None;
        }
        (author.tag == tag).then_some(handle)
    })
}

// ── 命令分发 ─────────────────────────────────────────────────────────────

impl OpenCADStudio {
    /// `PIDCONNECTOR` / `PIDPAIR` 的分发入口。
    pub(super) fn dispatch_pidconnector(&mut self, cmd: &str, i: usize) -> Option<Task<Message>> {
        let mut words = cmd.split_whitespace();
        let verb = words.next()?;
        if verb.eq_ignore_ascii_case("PIDPAIR") {
            let Some(handle_text_arg) = words.next() else {
                self.command_line
                    .push_info("PIDPAIR  <本图 handle> <drawing_id>:<handle>:");
                return Some(Task::none());
            };
            let Some(peer) = words.next() else {
                self.command_line
                    .push_info("PIDPAIR  <本图 handle> <drawing_id>:<handle>:");
                return Some(Task::none());
            };
            let Some(handle) = parse_handle(handle_text_arg) else {
                self.command_line.push_error_coded(
                    "invalid_handle",
                    &format!("PIDPAIR: \"{handle_text_arg}\" 不是十六进制 handle。"),
                );
                return Some(Task::none());
            };
            if let Some(receipt) = self.control_pid_pair(i, handle, peer) {
                self.command_line.push_output(&receipt);
            }
            return Some(Task::none());
        }
        if !verb.eq_ignore_ascii_case("PIDCONNECTOR") {
            return None;
        }
        let Some(kind_text) = words.next() else {
            self.command_line.push_info(
                "PIDCONNECTOR  <pipe_in|pipe_out|signal_in|signal_out|JUMP> [<drawing_id>:<handle>]:",
            );
            return Some(Task::none());
        };
        if kind_text.eq_ignore_ascii_case("JUMP") {
            let handle = words.next().and_then(parse_handle);
            let _ = self.jump_to_connector_peer(i, handle);
            return Some(Task::none());
        }
        let Some(kind) = ConnectorKind::parse(kind_text) else {
            self.command_line.push_error_coded(
                "invalid_connector_kind",
                &format!(
                    "PIDCONNECTOR: \"{kind_text}\" 不是 pipe_in / pipe_out / signal_in / signal_out。"
                ),
            );
            return Some(Task::none());
        };
        let command = PidConnectorCommand::new(kind, words.next().map(str::to_string));
        let prompt = command.prompt();
        self.reset_command_start_state(i);
        self.command_line.push_info(&prompt);
        self.tabs[i].active_cmd = Some(Box::new(command));
        Some(self.finish_dispatch(cmd))
    }

    /// Jump from a page connector on tab `i` to its paired peer drawing and
    /// line (#64). Used by both `PIDCONNECTOR JUMP` and the Inspector's
    /// 【跳转】 button.
    pub(crate) fn jump_to_connector_peer(
        &mut self,
        i: usize,
        explicit_handle: Option<document::Handle>,
    ) -> Result<String, String> {
        let handle = explicit_handle.or_else(|| {
            self.tabs
                .get(i)?
                .scene
                .selected_handles_in_order()
                .first()
                .copied()
        });
        let Some(handle) = handle else {
            let msg = "PIDCONNECTOR JUMP: 请先选中一个页间连接符，或指定其 handle。".to_owned();
            self.command_line.push_info(&msg);
            return Err(msg);
        };
        let doc = &self.tabs[i].scene.document;
        let table = TopologyTable::load(doc);
        let htext = handle_text(handle);
        let Some(record) = table
            .connectors
            .iter()
            .find(|c| c.handle.eq_ignore_ascii_case(&htext))
            .cloned()
        else {
            let msg = format!("PIDCONNECTOR JUMP: 实体 {htext} 不是页间连接符。");
            self.command_line.push_info(&msg);
            return Err(msg);
        };

        let peer_drawing = record.peer_drawing.clone().or_else(|| {
            record
                .peer
                .as_deref()
                .and_then(|p| p.split_once(':'))
                .map(|(d, _)| d.to_owned())
        });
        let Some(peer_drawing) = peer_drawing.filter(|d| !d.trim().is_empty()) else {
            let msg = format!(
                "页间连接符（线号 {}）尚未配对，没有可跳转的对端。",
                record.line_tag
            );
            self.command_line.push_info(&msg);
            self.collab
                .push_toast(crate::app::collab::toasts::ToastKind::Info, msg.clone());
            return Err(msg);
        };

        // Parse optional `<drawing>:<handle_or_line>` suffix.
        let peer_suffix = record
            .peer
            .as_deref()
            .and_then(|p| p.split_once(':'))
            .map(|(_, rhs)| rhs.to_owned());

        // Find the open tab matching `peer_drawing` (by numeric tab id, file
        // stem, display title, or collab_state drawing_id).
        let target_tab = self.tabs.iter().enumerate().find_map(|(idx, tab)| {
            if tab.is_start {
                return None;
            }
            if tab.id.to_string() == peer_drawing {
                return Some(idx);
            }
            if let Some(stem) = tab
                .current_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy())
            {
                if stem.eq_ignore_ascii_case(&peer_drawing) {
                    return Some(idx);
                }
            }
            if tab.tab_display_name().eq_ignore_ascii_case(&peer_drawing) {
                return Some(idx);
            }
            if let Some(state) = crate::pid::collab_state::load_state(&tab.scene.document) {
                if state.drawing_id.eq_ignore_ascii_case(&peer_drawing) {
                    return Some(idx);
                }
            }
            None
        });

        let Some(target_idx) = target_tab else {
            let msg = format!(
                "对端图纸「{peer_drawing}」未在本机打开；请先通过「打开」或 OPEN 打开该图纸文件后再跳转。"
            );
            self.command_line.push_info(&msg);
            self.collab
                .push_toast(crate::app::collab::toasts::ToastKind::Warning, msg.clone());
            return Err(msg);
        };

        // Switch to target tab and focus the peer connector or host line.
        self.active_tab = target_idx;
        let peer_handles: Vec<document::Handle> = {
            let dest_doc = &self.tabs[target_idx].scene.document;
            let mut found = Vec::new();
            if let Some(rhs) = peer_suffix.as_deref() {
                if let Some(peer_h) = parse_handle(rhs) {
                    if dest_doc.get_entity(peer_h).is_some() {
                        found.push(peer_h);
                    }
                }
            }
            if found.is_empty() {
                for entity in dest_doc.entities() {
                    let h = entity.common().handle;
                    if let Some(author) = PidAuthor::load(dest_doc, h) {
                        if author.tag.eq_ignore_ascii_case(&record.line_tag) {
                            found.push(h);
                        }
                    }
                }
            }
            found
        };

        if let Some((&first, rest)) = peer_handles.split_first() {
            self.tabs[target_idx].scene.select_entity(first, true);
            if !rest.is_empty() {
                self.tabs[target_idx].scene.select_entities(rest);
            }
            let _ = self.tabs[target_idx].scene.zoom_to_entities(&peer_handles);
        }
        self.refresh_properties();
        let msg = format!(
            "已跳转到对端图纸「{peer_drawing}」的线号 {}。",
            record.line_tag
        );
        self.command_line.push_info(&msg);
        self.collab
            .push_toast(crate::app::collab::toasts::ToastKind::Success, msg.clone());
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenCADStudio;
    use serde_json::Value;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SERIAL: AtomicU64 = AtomicU64::new(0);

    fn app() -> OpenCADStudio {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app
    }

    fn request(app: &mut OpenCADStudio, mut req: Value) -> Value {
        if let Some(object) = req.as_object_mut() {
            if !object.contains_key("document_id") {
                object.insert("document_id".into(), json!(app.tabs[app.active_tab].id));
            }
            if !object.contains_key("request_id") {
                let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
                object.insert("request_id".into(), json!(format!("pidconn-{serial}")));
            }
        }
        let (response, task) = app.control_request(req);
        // `TabNew` / `TabSwitch` 返回需要驱动的 Task（切标签页是异步的），
        // 不驱动的话活动文档不会真的换过去。
        app.drive_headless_task(task).expect("任务可无 GUI 驱动");
        response
    }

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

    /// 新建一张图并返回它的 `document_id`（也是 `peer` 里的 drawing_id）。
    fn new_doc(app: &mut OpenCADStudio) -> u64 {
        let opened = request(app, json!({"op":"new"}));
        assert_eq!(opened["ok"], true, "{opened}");
        app.tabs[app.active_tab].id
    }

    fn activate(app: &mut OpenCADStudio, id: u64) {
        let switched = request(app, json!({"op":"activate","document_id":id}));
        assert_eq!(switched["ok"], true, "{switched}");
        assert_eq!(app.tabs[app.active_tab].id, id);
    }

    fn topology(app: &OpenCADStudio) -> TopologyTable {
        TopologyTable::load(&app.tabs[app.active_tab].scene.document)
    }

    fn entity_count(app: &OpenCADStudio) -> usize {
        app.tabs[app.active_tab].scene.document.entities().count()
    }

    fn author_of(app: &OpenCADStudio, handle: document::Handle) -> PidAuthor {
        PidAuthor::load(&app.tabs[app.active_tab].scene.document, handle)
            .expect("带 PID_AUTHOR")
    }

    /// 按位号找本图线体实体。
    fn line_of(app: &OpenCADStudio, tag: &str) -> document::Handle {
        line_entity_of_tag(&app.tabs[app.active_tab].scene.document, tag)
            .unwrap_or_else(|| panic!("文档里没有线体 {tag}"))
    }

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

    fn props_of(app: &OpenCADStudio, handle: document::Handle) -> std::collections::BTreeMap<String, String> {
        crate::pid::propagate::props_of(&app.tabs[app.active_tab].scene.document, handle)
    }

    /// 一张图里画好 `L-1001`（管线）与 `S-2001`（信号线）。
    fn scene_with_both_lines(app: &mut OpenCADStudio) {
        draw(app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        draw(app, "PIDPIPE signal", &[[0.0, 100.0, 0.0], [200.0, 100.0, 0.0]]);
    }

    // ── VAL-PIPE-023 ─────────────────────────────────────────────────────

    /// 四种连接符各自创建成功，且 `topology` + `type` 能把管道与信号线分开。
    #[test]
    fn four_connector_kinds_are_created_and_distinguishable() {
        let mut app = app();
        scene_with_both_lines(&mut app);

        let cases = [
            ("PIDCONNECTOR pipe_in", [40.0, 0.0, 0.0], "connector_in", "PIPE_LINE"),
            ("PIDCONNECTOR pipe_out", [80.0, 0.0, 0.0], "connector_out", "PIPE_LINE"),
            ("PIDCONNECTOR signal_in", [40.0, 100.0, 0.0], "connector_in", "SIGNAL_LINE"),
            ("PIDCONNECTOR signal_out", [80.0, 100.0, 0.0], "connector_out", "SIGNAL_LINE"),
        ];
        let mut handles: Vec<document::Handle> = Vec::new();
        for (cmd, point, role, type_code) in cases {
            let done = draw(&mut app, cmd, &[point]);
            assert_eq!(done["status"], "completed", "{cmd}: {done}");
            assert_eq!(done["result"]["added"], 1, "{cmd}: {done}");
            let handle = parse_handle(done["result"]["handle"].as_str().expect("handle"))
                .expect("十六进制 handle");
            let author = author_of(&app, handle);
            assert_eq!(author.topology, role, "{cmd}");
            assert_eq!(author.type_code, type_code, "{cmd}");
            handles.push(handle);
        }

        // 四者的 `(topology, type)` 两两不同 —— 这就是「可区分」的判据。
        let mut seen: Vec<(String, String)> = Vec::new();
        for handle in &handles {
            let author = author_of(&app, *handle);
            let key = (author.topology.clone(), author.type_code.clone());
            assert!(!seen.contains(&key), "四者必须可区分，重复：{key:?}");
            seen.push(key);
        }

        let table = topology(&app);
        assert_eq!(table.connectors.len(), 4);
        let kinds: Vec<&str> = table.connectors.iter().map(|c| c.kind.as_str()).collect();
        let mut sorted = kinds.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec!["pipe_in", "pipe_out", "signal_in", "signal_out"],
            "四条记录的 kind 必须互不相同"
        );
    }

    // ── VAL-PIPE-024 ─────────────────────────────────────────────────────

    /// 同一线号同图第二个流入连接符被拒（`DUPLICATE_INFLOW_CONNECTOR`），
    /// 实体数与拓扑表都不变；不同线号与流出方向不受此约束。
    #[test]
    fn a_second_inflow_connector_on_the_same_line_is_refused() {
        let mut app = app();
        scene_with_both_lines(&mut app);

        let first = draw(&mut app, "PIDCONNECTOR pipe_in", &[[40.0, 0.0, 0.0]]);
        assert_eq!(first["status"], "completed", "{first}");
        let count_before = entity_count(&app);
        let connectors_before = topology(&app).connectors.len();
        assert_eq!(connectors_before, 1);

        let refused = draw(&mut app, "PIDCONNECTOR pipe_in", &[[60.0, 0.0, 0.0]]);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["status"], "failed", "{refused}");
        assert_eq!(
            refused["code"], "DUPLICATE_INFLOW_CONNECTOR",
            "错误码必须逐字匹配：{refused}"
        );
        assert_eq!(entity_count(&app), count_before, "实体数不得变化");
        assert_eq!(topology(&app).connectors.len(), connectors_before);

        // 对照一：不同线号可以再来一个流入。
        let other_line = draw(&mut app, "PIDCONNECTOR signal_in", &[[40.0, 100.0, 0.0]]);
        assert_eq!(other_line["status"], "completed", "{other_line}");
        assert_eq!(topology(&app).connectors.len(), 2);

        // 对照二：同一线号的第 2 个**流出**不受约束。
        let outflow = draw(&mut app, "PIDCONNECTOR pipe_out", &[[60.0, 0.0, 0.0]]);
        assert_eq!(outflow["status"], "completed", "{outflow}");
        assert_eq!(topology(&app).connectors.len(), 3);
    }

    // ── VAL-PIPE-025 ─────────────────────────────────────────────────────

    /// 管道连接符选到信号线上、信号线连接符选到管线上都返回
    /// `CONNECTOR_KIND_MISMATCH`，实体数与拓扑表不变；正确配对则成功。
    #[test]
    fn a_connector_on_the_wrong_line_kind_is_refused() {
        let mut app = app();
        scene_with_both_lines(&mut app);
        let count_before = entity_count(&app);

        let wrong = draw(&mut app, "PIDCONNECTOR pipe_in", &[[100.0, 100.0, 0.0]]);
        assert_eq!(wrong["ok"], false, "{wrong}");
        assert_eq!(
            wrong["code"], "CONNECTOR_KIND_MISMATCH",
            "错误码必须逐字匹配：{wrong}"
        );
        assert_eq!(entity_count(&app), count_before);
        assert_eq!(topology(&app).connectors.len(), 0);

        let reverse = draw(&mut app, "PIDCONNECTOR signal_in", &[[100.0, 0.0, 0.0]]);
        assert_eq!(reverse["ok"], false, "{reverse}");
        assert_eq!(reverse["code"], "CONNECTOR_KIND_MISMATCH", "{reverse}");
        assert_eq!(entity_count(&app), count_before);
        assert_eq!(topology(&app).connectors.len(), 0);

        let good = draw(&mut app, "PIDCONNECTOR pipe_in", &[[100.0, 0.0, 0.0]]);
        assert_eq!(good["status"], "completed", "{good}");
        assert_eq!(topology(&app).connectors.len(), 1);
    }

    /// 落点不在任何线体上时不落任何东西（不报 mismatch，也不建实体）。
    #[test]
    fn a_connector_away_from_every_line_is_refused() {
        let mut app = app();
        scene_with_both_lines(&mut app);
        let count_before = entity_count(&app);
        let done = draw(&mut app, "PIDCONNECTOR pipe_in", &[[100.0, 400.0, 0.0]]);
        assert_eq!(done["ok"], false, "{done}");
        assert_eq!(done["code"], "CONNECTOR_NO_LINE", "{done}");
        assert_eq!(entity_count(&app), count_before);
        assert_eq!(topology(&app).connectors.len(), 0);
    }

    // ── VAL-PIPE-026 ─────────────────────────────────────────────────────

    /// 配对后两侧 `connector_peer` 都是 `<drawing_id>:<handle>`，互为对端，
    /// 且与拓扑表 `connectors[].peer` 一致。
    #[test]
    fn pairing_writes_cross_drawing_peers_on_both_sides() {
        let mut app = app();
        let first = app.tabs[app.active_tab].id;
        scene_with_both_lines(&mut app);
        let out = draw(&mut app, "PIDCONNECTOR pipe_out", &[[80.0, 0.0, 0.0]]);
        assert_eq!(out["status"], "completed", "{out}");
        let out_handle = parse_handle(out["result"]["handle"].as_str().unwrap()).unwrap();

        let second = new_doc(&mut app);
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let inflow = draw(&mut app, "PIDCONNECTOR pipe_in", &[[80.0, 0.0, 0.0]]);
        assert_eq!(inflow["status"], "completed", "{inflow}");
        let in_handle = parse_handle(inflow["result"]["handle"].as_str().unwrap()).unwrap();

        // 在本图（第二张）配对：本侧流入 ↔ 第一张图的流出。
        let peer_arg = peer_text(first, out_handle);
        let paired = request(
            &mut app,
            json!({"op":"run","cmd":format!("PIDPAIR {} {peer_arg}", handle_text(in_handle))}),
        );
        assert_eq!(paired["ok"], true, "{paired}");

        // 本侧（第二张）：peer 指向第一张图的流出连接符。
        let mine = author_of(&app, in_handle);
        assert_eq!(
            mine.connector_peer,
            format!("{first}:{}", handle_text(out_handle))
        );
        let table_second = topology(&app);
        let record = table_second
            .connector_by_handle(in_handle)
            .expect("拓扑表有本侧连接符");
        assert_eq!(
            record.peer.as_deref(),
            Some(format!("{first}:{}", handle_text(out_handle)).as_str()),
            "拓扑表的 peer 必须与 XDATA 一致"
        );
        assert_eq!(record.peer_drawing.as_deref(), Some(first.to_string().as_str()));

        // 对侧（第一张）：反向指回第二张图。
        activate(&mut app, first);
        let theirs = author_of(&app, out_handle);
        assert_eq!(
            theirs.connector_peer,
            format!("{second}:{}", handle_text(in_handle))
        );
        let table_first = topology(&app);
        let back = table_first
            .connector_by_handle(out_handle)
            .expect("拓扑表有对侧连接符");
        assert_eq!(
            back.peer.as_deref(),
            Some(format!("{second}:{}", handle_text(in_handle)).as_str())
        );

        // 格式：`^[^:\s]+:[0-9A-Fa-f]+$`。
        for text in [&theirs.connector_peer, &mine.connector_peer] {
            let (drawing, handle) = text.split_once(':').expect("有冒号");
            assert!(drawing.chars().all(|c| !c.is_whitespace()));
            assert!(!handle.is_empty());
            assert!(handle.chars().all(|c| c.is_ascii_hexdigit()), "{text}");
        }
    }

    /// 一对流出/流入建立后属性从流出侧传到流入侧（`VAL-PIPE-027`）。
    #[test]
    fn pairing_transfers_pipe_properties_to_the_inflow_side() {
        let mut app = app();
        let first = app.tabs[app.active_tab].id;
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let source_line = line_of(&app, "L-1001");
        put_props(
            &mut app,
            source_line,
            &[
                ("管道等级", "A1A"),
                ("公称直径", "DN50"),
                ("绝热等级", "H"),
                ("流体介质", "工艺水"),
                ("管线类型", "工艺管线"),
            ],
        );
        let out = draw(&mut app, "PIDCONNECTOR pipe_out", &[[80.0, 0.0, 0.0]]);
        assert_eq!(out["status"], "completed", "{out}");
        let out_handle = parse_handle(out["result"]["handle"].as_str().unwrap()).unwrap();

        let _second = new_doc(&mut app);
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let target_line = line_of(&app, "L-1001");
        assert!(props_of(&app, target_line).is_empty(), "对端初始无这些属性");
        let inflow = draw(&mut app, "PIDCONNECTOR pipe_in", &[[80.0, 0.0, 0.0]]);
        assert_eq!(inflow["status"], "completed", "{inflow}");
        let in_handle = parse_handle(inflow["result"]["handle"].as_str().unwrap()).unwrap();

        let paired = request(
            &mut app,
            json!({"op":"run","cmd":format!("PIDPAIR {} {}", handle_text(in_handle), peer_text(first, out_handle))}),
        );
        assert_eq!(paired["ok"], true, "{paired}");

        let props = props_of(&app, target_line);
        assert_eq!(props.get("管道等级").map(String::as_str), Some("A1A"), "{props:?}");
        assert_eq!(props.get("公称直径").map(String::as_str), Some("DN50"), "{props:?}");

        // 连接符条目含可读的图纸跳转信息与头尾信息。
        let table = topology(&app);
        let record = table.connector_by_handle(in_handle).expect("连接符记录");
        assert_eq!(record.peer_drawing.as_deref(), Some(first.to_string().as_str()));
        assert!(record.upstream.is_some(), "缺上游头尾信息：{record:?}");
        assert!(record.downstream.is_some(), "缺下游头尾信息：{record:?}");
    }

    /// 信号线连接符传 `起点仪表` / `终点仪表`，且对端 `props` **不含**
    /// `管道等级`（`VAL-PIPE-027` 第 2 条）。
    #[test]
    fn signal_connectors_transfer_instruments_and_never_a_pipe_grade() {
        let mut app = app();
        let first = app.tabs[app.active_tab].id;
        draw(&mut app, "PIDPIPE signal", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let source_line = line_of(&app, "S-2001");
        put_props(
            &mut app,
            source_line,
            &[("起点仪表", "TI-1001"), ("终点仪表", "TI-1002")],
        );
        let out = draw(&mut app, "PIDCONNECTOR signal_out", &[[80.0, 0.0, 0.0]]);
        assert_eq!(out["status"], "completed", "{out}");
        let out_handle = parse_handle(out["result"]["handle"].as_str().unwrap()).unwrap();

        let _second = new_doc(&mut app);
        draw(&mut app, "PIDPIPE signal", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let target_line = line_of(&app, "S-2001");
        let inflow = draw(&mut app, "PIDCONNECTOR signal_in", &[[80.0, 0.0, 0.0]]);
        assert_eq!(inflow["status"], "completed", "{inflow}");
        let in_handle = parse_handle(inflow["result"]["handle"].as_str().unwrap()).unwrap();

        let paired = request(
            &mut app,
            json!({"op":"run","cmd":format!("PIDPAIR {} {}", handle_text(in_handle), peer_text(first, out_handle))}),
        );
        assert_eq!(paired["ok"], true, "{paired}");

        let props = props_of(&app, target_line);
        assert_eq!(props.get("起点仪表").map(String::as_str), Some("TI-1001"), "{props:?}");
        assert_eq!(props.get("终点仪表").map(String::as_str), Some("TI-1002"), "{props:?}");
        assert!(
            props
                .get("管道等级")
                .is_none_or(|value| value.is_empty()),
            "信号线不得携带管道等级：{props:?}"
        );
    }

    /// 一入一出才能配对：两个流出或两个流入都返回 `CONNECTOR_PEER_INVALID`。
    #[test]
    fn pairing_two_connectors_of_the_same_direction_is_refused() {
        let mut app = app();
        let first = app.tabs[app.active_tab].id;
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let out = draw(&mut app, "PIDCONNECTOR pipe_out", &[[80.0, 0.0, 0.0]]);
        let out_handle = parse_handle(out["result"]["handle"].as_str().unwrap()).unwrap();

        let _second = new_doc(&mut app);
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let other = draw(&mut app, "PIDCONNECTOR pipe_out", &[[80.0, 0.0, 0.0]]);
        let other_handle = parse_handle(other["result"]["handle"].as_str().unwrap()).unwrap();

        let refused = request(
            &mut app,
            json!({"op":"run","cmd":format!("PIDPAIR {} {}", handle_text(other_handle), peer_text(first, out_handle))}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["code"], "CONNECTOR_PEER_INVALID", "{refused}");
        // 被拒后本侧连接符仍无对端。
        assert!(author_of(&app, other_handle).connector_peer.is_empty());
    }

    /// `PIDCONNECTOR <kind> <peer>` 创建即配对。
    #[test]
    fn creating_a_connector_with_a_peer_pairs_it_immediately() {
        let mut app = app();
        let first = app.tabs[app.active_tab].id;
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let out = draw(&mut app, "PIDCONNECTOR pipe_out", &[[80.0, 0.0, 0.0]]);
        let out_handle = parse_handle(out["result"]["handle"].as_str().unwrap()).unwrap();

        let _second = new_doc(&mut app);
        draw(&mut app, "PIDPIPE", &[[0.0, 0.0, 0.0], [200.0, 0.0, 0.0]]);
        let peer = peer_text(first, out_handle);
        let done = draw(&mut app, &format!("PIDCONNECTOR pipe_in {peer}"), &[[80.0, 0.0, 0.0]]);
        assert_eq!(done["status"], "completed", "{done}");
        let in_handle = parse_handle(done["result"]["handle"].as_str().unwrap()).unwrap();
        assert_eq!(author_of(&app, in_handle).connector_peer, peer);
    }

    /// `PIDCONNECTOR` 的 kind 字面量非法时直接拒绝，不进入拾取。
    #[test]
    fn an_unknown_connector_kind_is_refused() {
        let mut app = app();
        scene_with_both_lines(&mut app);
        let count_before = entity_count(&app);
        let done = request(&mut app, json!({"op":"run","cmd":"PIDCONNECTOR pipe_sideways"}));
        assert_eq!(done["ok"], false, "{done}");
        assert_eq!(done["code"], "invalid_connector_kind", "{done}");
        assert_eq!(entity_count(&app), count_before);
        assert_eq!(topology(&app).connectors.len(), 0);
    }
}


//! The three-tier permission chain: role matrix × PBS ACL inheritance ×
//! drawing state (see `architecture.md` §4 and appendix A.3).
//!
//! The chain short-circuits: the first tier that denies wins, and its level
//! plus error code are returned unchanged. All of it is a pure function over
//! plain data, so the same decision can be made by the server or asserted by a
//! test without a database.

use serde::{Deserialize, Serialize};

use crate::protocol::codes;

/// The five seeded roles. Serde keeps the Chinese literals used by the
/// protocol and the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "系统管理员")]
    SystemAdmin,
    #[serde(rename = "项目管理员")]
    ProjectAdmin,
    #[serde(rename = "工艺设计人员")]
    ProcessDesigner,
    #[serde(rename = "校对人员")]
    Proofreader,
    #[serde(rename = "只读查看者")]
    ReadOnlyViewer,
}

impl Role {
    /// Every role, in matrix order.
    pub const ALL: [Role; 5] = [
        Role::SystemAdmin,
        Role::ProjectAdmin,
        Role::ProcessDesigner,
        Role::Proofreader,
        Role::ReadOnlyViewer,
    ];

    /// The database / wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::SystemAdmin => "系统管理员",
            Role::ProjectAdmin => "项目管理员",
            Role::ProcessDesigner => "工艺设计人员",
            Role::Proofreader => "校对人员",
            Role::ReadOnlyViewer => "只读查看者",
        }
    }

    pub fn parse(text: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.as_str() == text)
    }
}

/// The eight operations of the role × operation matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operation {
    #[serde(rename = "查看图纸")]
    ViewDrawing,
    #[serde(rename = "编辑图纸")]
    EditDrawing,
    #[serde(rename = "导出图纸")]
    ExportDrawing,
    #[serde(rename = "删除图纸")]
    DeleteDrawing,
    #[serde(rename = "发布签核")]
    PublishSignOff,
    #[serde(rename = "授权配置")]
    ConfigureAuthorization,
    #[serde(rename = "规则检查")]
    RuleCheck,
    #[serde(rename = "主数据维护")]
    MaintainMasterData,
}

impl Operation {
    pub const ALL: [Operation; 8] = [
        Operation::ViewDrawing,
        Operation::EditDrawing,
        Operation::ExportDrawing,
        Operation::DeleteDrawing,
        Operation::PublishSignOff,
        Operation::ConfigureAuthorization,
        Operation::RuleCheck,
        Operation::MaintainMasterData,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Operation::ViewDrawing => "查看图纸",
            Operation::EditDrawing => "编辑图纸",
            Operation::ExportDrawing => "导出图纸",
            Operation::DeleteDrawing => "删除图纸",
            Operation::PublishSignOff => "发布签核",
            Operation::ConfigureAuthorization => "授权配置",
            Operation::RuleCheck => "规则检查",
            Operation::MaintainMasterData => "主数据维护",
        }
    }

    pub fn parse(text: &str) -> Option<Operation> {
        Operation::ALL
            .into_iter()
            .find(|operation| operation.as_str() == text)
    }

    /// Collapses the eight operations onto the four `pbs_acl.permission`
    /// values (appendix A.3).
    pub fn acl_permission(self) -> AclPermission {
        match self {
            Operation::ViewDrawing | Operation::RuleCheck => AclPermission::View,
            Operation::EditDrawing
            | Operation::PublishSignOff
            | Operation::ConfigureAuthorization
            | Operation::MaintainMasterData => AclPermission::Edit,
            Operation::ExportDrawing => AclPermission::Export,
            Operation::DeleteDrawing => AclPermission::Delete,
        }
    }

    /// Whether the operation mutates the drawing. `只读` rejects exactly these.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            Operation::EditDrawing
                | Operation::DeleteDrawing
                | Operation::PublishSignOff
                | Operation::ConfigureAuthorization
                | Operation::MaintainMasterData
        )
    }
}

/// The four ACL permission values stored in `pbs_acl.permission`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AclPermission {
    #[serde(rename = "查看")]
    View,
    #[serde(rename = "编辑")]
    Edit,
    #[serde(rename = "导出")]
    Export,
    #[serde(rename = "删除")]
    Delete,
}

impl AclPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            AclPermission::View => "查看",
            AclPermission::Edit => "编辑",
            AclPermission::Export => "导出",
            AclPermission::Delete => "删除",
        }
    }
}

/// The three drawing states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DrawingState {
    #[serde(rename = "单人编辑")]
    SingleEditor,
    #[serde(rename = "协同编辑")]
    Collaborative,
    #[serde(rename = "只读")]
    ReadOnly,
}

impl DrawingState {
    pub const ALL: [DrawingState; 3] = [
        DrawingState::SingleEditor,
        DrawingState::Collaborative,
        DrawingState::ReadOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DrawingState::SingleEditor => "单人编辑",
            DrawingState::Collaborative => "协同编辑",
            DrawingState::ReadOnly => "只读",
        }
    }

    pub fn parse(text: &str) -> Option<DrawingState> {
        DrawingState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
    }

    /// All three states interconvert (appendix A.3).
    pub fn can_transition_to(self, next: DrawingState) -> bool {
        DrawingState::ALL.contains(&next)
    }
}

/// Which tier produced the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenialLevel {
    Role,
    Pbs,
    State,
}

impl DenialLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            DenialLevel::Role => "role",
            DenialLevel::Pbs => "pbs",
            DenialLevel::State => "state",
        }
    }

    /// The `PERM_*` code that belongs to this tier.
    pub fn code(self) -> &'static str {
        match self {
            DenialLevel::Role => codes::PERM_DENIED_ROLE,
            DenialLevel::Pbs => codes::PERM_DENIED_PBS,
            DenialLevel::State => codes::PERM_DENIED_STATE,
        }
    }
}

/// One ACL row: `subject` is either a user id or a role name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclEntry {
    pub subject: String,
    pub permission: AclPermission,
    pub allowed: bool,
}

/// A PBS node plus its parent link and its explicit ACL rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclNode {
    pub node_id: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub entries: Vec<AclEntry>,
}

/// Everything the chain needs, with no database in sight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest<'a> {
    pub user_id: &'a str,
    pub role: Role,
    pub operation: Operation,
    pub drawing_state: DrawingState,
    /// The PBS node the drawing is filed under.
    pub node_id: &'a str,
    /// The whole PBS tree; the chain walks parents through this.
    pub nodes: &'a [AclNode],
}

/// The ACL row that decided the PBS tier (#74).
///
/// On one node, a user row and a role row for the same permission that
/// disagree resolve to the denying row (`rule` = `拒绝为准`). A nearer node
/// still wins over an ancestor, so an exception is configured on the child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecidingRow {
    pub subject: String,
    /// `user` when `subject` is a user id, `role` when it is a role name.
    pub subject_kind: String,
    pub permission: String,
    pub allowed: bool,
    /// `拒绝为准`, `两行一致`, or `唯一授权行`.
    pub rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_allowed: Option<bool>,
}

/// The chain's answer, matching appendix A.3's JSON shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub allowed: bool,
    pub denied_level: Option<DenialLevel>,
    pub code: Option<String>,
    pub effective_node_id: Option<String>,
    pub effective_node_path: Vec<String>,
    pub inherited: bool,
    /// Present when the PBS tier ran and a row on some node decided it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deciding_row: Option<DecidingRow>,
}

impl PermissionDecision {
    fn allow(node_id: &str, path: Vec<String>, inherited: bool, row: DecidingRow) -> Self {
        PermissionDecision {
            allowed: true,
            denied_level: None,
            code: None,
            effective_node_id: Some(node_id.to_owned()),
            effective_node_path: path,
            inherited,
            deciding_row: Some(row),
        }
    }

    fn deny_pbs(node_id: &str, path: Vec<String>, inherited: bool, row: DecidingRow) -> Self {
        PermissionDecision {
            allowed: false,
            denied_level: Some(DenialLevel::Pbs),
            code: Some(DenialLevel::Pbs.code().to_owned()),
            effective_node_id: Some(node_id.to_owned()),
            effective_node_path: path,
            inherited,
            deciding_row: Some(row),
        }
    }

    fn deny(level: DenialLevel) -> Self {
        PermissionDecision {
            allowed: false,
            denied_level: Some(level),
            code: Some(level.code().to_owned()),
            effective_node_id: None,
            effective_node_path: Vec::new(),
            inherited: false,
            deciding_row: None,
        }
    }

    fn deny_without_level(code: &str) -> Self {
        PermissionDecision {
            allowed: false,
            denied_level: None,
            code: Some(code.to_owned()),
            effective_node_id: None,
            effective_node_path: Vec::new(),
            inherited: false,
            deciding_row: None,
        }
    }
}

/// The 5 × 8 role matrix. `true` is 允许, `false` is 拒绝.
pub fn role_allows(role: Role, operation: Operation) -> bool {
    use Operation::*;
    use Role::*;
    match (role, operation) {
        (SystemAdmin, _) => true,
        (ProjectAdmin, MaintainMasterData) => false,
        (ProjectAdmin, _) => true,
        (ProcessDesigner, ViewDrawing | EditDrawing | ExportDrawing | RuleCheck) => true,
        (ProcessDesigner, _) => false,
        (Proofreader, ViewDrawing | ExportDrawing | RuleCheck) => true,
        (Proofreader, _) => false,
        (ReadOnlyViewer, ViewDrawing | RuleCheck) => true,
        (ReadOnlyViewer, _) => false,
    }
}

/// The drawing state tier. `只读` rejects every write operation.
pub fn state_allows(state: DrawingState, operation: Operation) -> bool {
    match state {
        DrawingState::ReadOnly => !operation.is_write(),
        DrawingState::SingleEditor | DrawingState::Collaborative => true,
    }
}

/// One node's combined user/role answer, plus the row a probe should name.
struct AclHit {
    allowed: bool,
    node_id: String,
    path: Vec<String>,
    inherited: bool,
    row: DecidingRow,
}

/// Result of the ACL lookup, before the state tier runs.
struct AclLookup {
    /// `None` when nothing along the path is configured (which denies).
    decision: Option<AclHit>,
}

/// Same-node user row and role row. Disagreement is deny-wins (#74).
fn combine_same_node<'a>(
    user_row: Option<&'a AclEntry>,
    role_row: Option<&'a AclEntry>,
) -> Option<(
    &'a AclEntry,
    &'static str,
    &'static str,
    Option<&'a AclEntry>,
)> {
    match (user_row, role_row) {
        (Some(user), Some(role)) if user.allowed != role.allowed => {
            if user.allowed {
                Some((role, "role", "拒绝为准", Some(user)))
            } else {
                Some((user, "user", "拒绝为准", Some(role)))
            }
        }
        (Some(user), Some(role)) => Some((user, "user", "两行一致", Some(role))),
        (Some(user), None) => Some((user, "user", "唯一授权行", None)),
        (None, Some(role)) => Some((role, "role", "唯一授权行", None)),
        (None, None) => None,
    }
}

fn lookup_acl(request: &PermissionRequest<'_>) -> AclLookup {
    let permission = request.operation.acl_permission();

    // Step 1: collect the whole ancestor chain from the target node upwards,
    // then flip it to root → target. This is the node's position in the tree
    // and does not depend on which level ends up deciding.
    let mut chain = Vec::new();
    let mut cursor = Some(request.node_id.to_owned());
    let mut guard = 0usize;
    while let Some(node_id) = cursor {
        guard += 1;
        if guard > 64 {
            break;
        }
        chain.push(node_id.clone());
        cursor = request
            .nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .and_then(|node| node.parent_id.clone());
    }
    chain.reverse();

    // Step 2: walk the same chain back from the target node, so the nearest
    // configured ancestor wins. `inherited` is true once we leave the target.
    // On that node, a user row and a role row that disagree both lose to deny.
    let mut inherited = false;
    let mut deciding: Option<AclHit> = None;
    for node_id in chain.iter().rev() {
        let node = match request.nodes.iter().find(|node| node.node_id == *node_id) {
            Some(node) => node,
            None => break,
        };

        let user_row = node
            .entries
            .iter()
            .find(|entry| entry.subject == request.user_id && entry.permission == permission);
        let role_row = node
            .entries
            .iter()
            .find(|entry| entry.subject == request.role.as_str() && entry.permission == permission);
        if let Some((row, kind, rule, other)) = combine_same_node(user_row, role_row) {
            deciding = Some(AclHit {
                allowed: row.allowed,
                node_id: node_id.clone(),
                path: chain.clone(),
                inherited,
                row: DecidingRow {
                    subject: row.subject.clone(),
                    subject_kind: kind.to_owned(),
                    permission: row.permission.as_str().to_owned(),
                    allowed: row.allowed,
                    rule: rule.to_owned(),
                    other_subject: other.map(|entry| entry.subject.clone()),
                    other_allowed: other.map(|entry| entry.allowed),
                },
            });
            break;
        }

        inherited = true;
    }

    AclLookup { decision: deciding }
}

/// Runs the chain: role → PBS ACL → drawing state.
pub fn decide(request: &PermissionRequest<'_>) -> PermissionDecision {
    let decision = decide_without_state(request);
    if decision.allowed && !state_allows(request.drawing_state, request.operation) {
        return PermissionDecision::deny(DenialLevel::State);
    }
    decision
}

/// The first two tiers only: role → PBS ACL. Switching a drawing's state is
/// judged this way, whatever state the drawing is in (REQ-COLL-004 rule 1).
pub fn decide_without_state(request: &PermissionRequest<'_>) -> PermissionDecision {
    if !role_allows(request.role, request.operation) {
        return PermissionDecision::deny(DenialLevel::Role);
    }

    match lookup_acl(request) {
        AclLookup { decision: None } => PermissionDecision::deny(DenialLevel::Pbs),
        AclLookup {
            decision: Some(hit),
        } if !hit.allowed => {
            PermissionDecision::deny_pbs(&hit.node_id, hit.path, hit.inherited, hit.row)
        }
        AclLookup {
            decision: Some(hit),
        } => PermissionDecision::allow(&hit.node_id, hit.path, hit.inherited, hit.row),
    }
}

/// Fails a request whose `operation` string is not one of the eight, or whose
/// drawing state string is not one of the three (appendix A.3 codes).
pub fn decide_from_strings(
    request: &PermissionRequest<'_>,
    operation: &str,
    state: &str,
) -> PermissionDecision {
    if Operation::parse(operation).is_none() {
        return PermissionDecision::deny_without_level(codes::PERM_UNKNOWN_OPERATION);
    }
    if DrawingState::parse(state).is_none() {
        return PermissionDecision::deny_without_level(codes::PERM_INVALID_STATE);
    }
    decide(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, parent: Option<&str>, entries: Vec<AclEntry>) -> AclNode {
        AclNode {
            node_id: id.to_owned(),
            parent_id: parent.map(str::to_owned),
            entries,
        }
    }

    fn allow_row(subject: &str, permission: AclPermission) -> AclEntry {
        AclEntry {
            subject: subject.to_owned(),
            permission,
            allowed: true,
        }
    }

    fn deny_row(subject: &str, permission: AclPermission) -> AclEntry {
        AclEntry {
            subject: subject.to_owned(),
            permission,
            allowed: false,
        }
    }

    fn tree_with_acl_on_unit() -> Vec<AclNode> {
        vec![
            node(
                "n-plant",
                None,
                vec![deny_row("u1", AclPermission::Edit)],
            ),
            node(
                "n-unit-a",
                Some("n-plant"),
                vec![allow_row("u1", AclPermission::Edit)],
            ),
            node("n-cell-a1", Some("n-unit-a"), vec![]),
        ]
    }

    #[test]
    fn role_matrix_all_40_cells_are_decided() {
        // Rows follow `Role::ALL`, columns follow `Operation::ALL`.
        // 系统管理员 / 项目管理员 / 工艺设计人员 / 校对人员 / 只读查看者
        const MATRIX: [[bool; 8]; 5] = [
            // 查看 编辑 导出 删除 发布 授权 规则 主数据
            [true, true, true, true, true, true, true, true],
            [true, true, true, true, true, true, true, false],
            [true, true, true, false, false, false, true, false],
            [true, false, true, false, false, false, true, false],
            [true, false, false, false, false, false, true, false],
        ];

        for (role_index, role) in Role::ALL.into_iter().enumerate() {
            for (operation_index, operation) in Operation::ALL.into_iter().enumerate() {
                let expected = MATRIX[role_index][operation_index];
                assert_eq!(
                    role_allows(role, operation),
                    expected,
                    "cell {role:?} ({}) / {operation:?} ({}) must be {expected}",
                    role.as_str(),
                    operation.as_str()
                );
            }
        }
        assert_eq!(Role::ALL.len() * Operation::ALL.len(), 40);
    }

    #[test]
    fn role_matrix_matches_the_architecture_table() {
        use Operation::*;
        use Role::*;

        // 系统管理员: everything allowed.
        for operation in Operation::ALL {
            assert!(role_allows(SystemAdmin, operation));
        }
        // 项目管理员: all but 主数据维护.
        assert!(role_allows(ProjectAdmin, EditDrawing));
        assert!(role_allows(ProjectAdmin, ConfigureAuthorization));
        assert!(!role_allows(ProjectAdmin, MaintainMasterData));
        // 工艺设计人员.
        assert!(role_allows(ProcessDesigner, EditDrawing));
        assert!(!role_allows(ProcessDesigner, DeleteDrawing));
        assert!(!role_allows(ProcessDesigner, PublishSignOff));
        // 校对人员.
        assert!(!role_allows(Proofreader, EditDrawing));
        assert!(role_allows(Proofreader, ExportDrawing));
        assert!(role_allows(Proofreader, RuleCheck));
        // 只读查看者.
        assert!(role_allows(ReadOnlyViewer, ViewDrawing));
        assert!(role_allows(ReadOnlyViewer, RuleCheck));
        assert!(!role_allows(ReadOnlyViewer, ExportDrawing));
    }

    #[test]
    fn proofreader_cannot_edit_drawing() {
        let nodes = tree_with_acl_on_unit();
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::Proofreader,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-cell-a1",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.denied_level, Some(DenialLevel::Role));
        assert_eq!(decision.code.as_deref(), Some(codes::PERM_DENIED_ROLE));
    }

    #[test]
    fn acl_inherits_nearest_ancestor_not_root() {
        let nodes = tree_with_acl_on_unit();
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-cell-a1",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert!(decision.allowed, "the unit node allows edit: {decision:?}");
        assert_eq!(decision.effective_node_id.as_deref(), Some("n-unit-a"));
        assert_eq!(
            decision.effective_node_path,
            vec!["n-plant", "n-unit-a", "n-cell-a1"],
            "path is the full root-to-target chain; the deciding node is separate"
        );
        assert!(decision.inherited, "the deciding node is an ancestor");
    }

    #[test]
    fn acl_path_is_complete_when_the_target_node_decides() {
        let nodes = vec![
            node("n-plant", None, vec![]),
            node("n-unit-a", Some("n-plant"), vec![]),
            node(
                "n-cell-a1",
                Some("n-unit-a"),
                vec![allow_row("u1", AclPermission::Edit)],
            ),
        ];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-cell-a1",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert!(
            decision.allowed,
            "the target node allows edit: {decision:?}"
        );
        assert_eq!(decision.effective_node_id.as_deref(), Some("n-cell-a1"));
        assert_eq!(
            decision.effective_node_path,
            vec!["n-plant", "n-unit-a", "n-cell-a1"],
            "path stays the full root-to-target chain when the target decides"
        );
        assert!(
            !decision.inherited,
            "the deciding node is the target itself"
        );
    }

    #[test]
    fn acl_on_the_target_node_is_not_marked_inherited() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![allow_row("u1", AclPermission::Edit)],
        )];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert!(decision.allowed);
        assert!(!decision.inherited);
    }

    #[test]
    fn acl_path_without_any_config_denies() {
        let nodes = vec![
            node("n-plant", None, vec![]),
            node("n-unit-a", Some("n-plant"), vec![]),
            node("n-cell-a1", Some("n-unit-a"), vec![]),
        ];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-cell-a1",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert!(!decision.allowed);
        assert_eq!(decision.denied_level, Some(DenialLevel::Pbs));
        assert_eq!(decision.code.as_deref(), Some(codes::PERM_DENIED_PBS));
    }

    fn edit_as_designer<'a>(nodes: &'a [AclNode]) -> PermissionRequest<'a> {
        PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-unit-a",
            nodes,
        }
    }

    #[test]
    fn same_node_user_deny_and_role_allow_is_deny() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![
                deny_row("u1", AclPermission::Edit),
                allow_row("工艺设计人员", AclPermission::Edit),
            ],
        )];
        let decision = decide(&edit_as_designer(&nodes));
        assert!(!decision.allowed);
        assert_eq!(decision.code.as_deref(), Some(codes::PERM_DENIED_PBS));
        let row = decision.deciding_row.expect("the denying row");
        assert_eq!(row.subject, "u1");
        assert_eq!(row.subject_kind, "user");
        assert!(!row.allowed);
        assert_eq!(row.rule, "拒绝为准");
        assert_eq!(row.other_subject.as_deref(), Some("工艺设计人员"));
        assert_eq!(row.other_allowed, Some(true));
    }

    #[test]
    fn same_node_role_deny_and_user_allow_is_deny() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![
                allow_row("u1", AclPermission::Edit),
                deny_row("工艺设计人员", AclPermission::Edit),
            ],
        )];
        let decision = decide(&edit_as_designer(&nodes));
        assert!(
            !decision.allowed,
            "deny wins; a user allow cannot override a role deny on the same node"
        );
        let row = decision.deciding_row.expect("the denying row");
        assert_eq!(row.subject, "工艺设计人员");
        assert_eq!(row.subject_kind, "role");
        assert!(!row.allowed);
        assert_eq!(row.rule, "拒绝为准");
        assert_eq!(row.other_subject.as_deref(), Some("u1"));
        assert_eq!(row.other_allowed, Some(true));
    }

    #[test]
    fn nearer_user_allow_still_overrides_an_ancestor_role_deny() {
        let nodes = vec![
            node(
                "n-plant",
                None,
                vec![deny_row("工艺设计人员", AclPermission::Edit)],
            ),
            node(
                "n-unit-a",
                Some("n-plant"),
                vec![allow_row("u1", AclPermission::Edit)],
            ),
        ];
        let decision = decide(&edit_as_designer(&nodes));
        assert!(decision.allowed);
        assert_eq!(decision.effective_node_id.as_deref(), Some("n-unit-a"));
        let row = decision.deciding_row.expect("the child row");
        assert_eq!(row.rule, "唯一授权行");
        assert_eq!(row.subject, "u1");
    }

    #[test]
    fn role_level_acl_applies_when_no_user_row_exists() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![allow_row("工艺设计人员", AclPermission::Edit)],
        )];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ProcessDesigner,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        assert!(decide(&request).allowed);
    }

    #[test]
    fn a_state_switch_passes_on_the_first_two_tiers_in_every_state() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![allow_row("u1", AclPermission::Edit)],
        )];
        let request = |user_id, role, drawing_state| PermissionRequest {
            user_id,
            role,
            operation: Operation::ConfigureAuthorization,
            drawing_state,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        for drawing_state in DrawingState::ALL {
            let admin = request("u1", Role::ProjectAdmin, drawing_state);
            assert!(decide_without_state(&admin).allowed, "{drawing_state:?}");
            assert_eq!(
                decide(&admin).allowed,
                drawing_state != DrawingState::ReadOnly,
                "the full chain still applies the state tier: {drawing_state:?}"
            );
            assert_eq!(
                decide_without_state(&request("u1", Role::ReadOnlyViewer, drawing_state)).denied_level,
                Some(DenialLevel::Role)
            );
            assert_eq!(
                decide_without_state(&request("u2", Role::ProjectAdmin, drawing_state)).denied_level,
                Some(DenialLevel::Pbs)
            );
        }
    }

    #[test]
    fn readonly_state_denies_all_five_write_operations() {
        // Grant every ACL permission so the state tier is the only one that
        // can deny: otherwise the ACL tier would short-circuit first for
        // operations that map onto 导出 / 删除.
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![
                allow_row("u1", AclPermission::View),
                allow_row("u1", AclPermission::Edit),
                allow_row("u1", AclPermission::Export),
                allow_row("u1", AclPermission::Delete),
            ],
        )];
        let mut denied = 0;
        for operation in Operation::ALL {
            if !operation.is_write() {
                continue;
            }
            denied += 1;
            let request = PermissionRequest {
                user_id: "u1",
                role: Role::SystemAdmin,
                operation,
                drawing_state: DrawingState::ReadOnly,
                node_id: "n-unit-a",
                nodes: &nodes,
            };
            let decision = decide(&request);
            assert!(!decision.allowed, "{operation:?} must be denied in 只读");
            assert_eq!(decision.denied_level, Some(DenialLevel::State));
            assert_eq!(decision.code.as_deref(), Some(codes::PERM_DENIED_STATE));
        }
        assert_eq!(denied, 5, "只读 rejects exactly the five write operations");
    }

    #[test]
    fn readonly_state_still_allows_viewing() {
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![allow_row("u1", AclPermission::View)],
        )];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ReadOnlyViewer,
            operation: Operation::ViewDrawing,
            drawing_state: DrawingState::ReadOnly,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        assert!(decide(&request).allowed);
    }

    #[test]
    fn role_denial_short_circuits_before_the_acl_tier() {
        // The ACL would allow, but the role tier denies first.
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![allow_row("u1", AclPermission::Edit)],
        )];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::ReadOnlyViewer,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        let decision = decide(&request);
        assert_eq!(decision.denied_level, Some(DenialLevel::Role));
        assert!(decision.effective_node_id.is_none());
    }

    #[test]
    fn acl_denial_short_circuits_before_the_state_tier() {
        // 只读 would deny too, but the ACL tier runs first.
        let nodes = vec![node(
            "n-unit-a",
            None,
            vec![deny_row("u1", AclPermission::Edit)],
        )];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::SystemAdmin,
            operation: Operation::EditDrawing,
            drawing_state: DrawingState::ReadOnly,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        assert_eq!(
            decide(&request).denied_level,
            Some(DenialLevel::Pbs),
            "the ACL tier must be reported, not the state tier"
        );
    }

    #[test]
    fn operation_maps_onto_the_four_acl_permissions() {
        assert_eq!(
            Operation::ViewDrawing.acl_permission(),
            AclPermission::View
        );
        assert_eq!(Operation::RuleCheck.acl_permission(), AclPermission::View);
        assert_eq!(Operation::EditDrawing.acl_permission(), AclPermission::Edit);
        assert_eq!(
            Operation::PublishSignOff.acl_permission(),
            AclPermission::Edit
        );
        assert_eq!(
            Operation::ConfigureAuthorization.acl_permission(),
            AclPermission::Edit
        );
        assert_eq!(
            Operation::MaintainMasterData.acl_permission(),
            AclPermission::Edit
        );
        assert_eq!(
            Operation::ExportDrawing.acl_permission(),
            AclPermission::Export
        );
        assert_eq!(
            Operation::DeleteDrawing.acl_permission(),
            AclPermission::Delete
        );
    }

    #[test]
    fn all_three_drawing_states_interconvert() {
        for from in DrawingState::ALL {
            for to in DrawingState::ALL {
                assert!(from.can_transition_to(to), "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn unknown_operation_and_invalid_state_get_their_own_codes() {
        let nodes: Vec<AclNode> = vec![];
        let request = PermissionRequest {
            user_id: "u1",
            role: Role::SystemAdmin,
            operation: Operation::ViewDrawing,
            drawing_state: DrawingState::Collaborative,
            node_id: "n-unit-a",
            nodes: &nodes,
        };
        assert_eq!(
            decide_from_strings(&request, "飞到月球", "协同编辑").code.as_deref(),
            Some(codes::PERM_UNKNOWN_OPERATION)
        );
        assert_eq!(
            decide_from_strings(&request, "查看图纸", "四态")
                .code
                .as_deref(),
            Some(codes::PERM_INVALID_STATE)
        );
    }

    #[test]
    fn role_and_operation_strings_round_trip() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        for operation in Operation::ALL {
            assert_eq!(Operation::parse(operation.as_str()), Some(operation));
        }
        for state in DrawingState::ALL {
            assert_eq!(DrawingState::parse(state.as_str()), Some(state));
        }
    }
}

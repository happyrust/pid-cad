//! axum HTTP control plane.
//!
//! The scaffold serves the endpoints later milestones build on: `/health`,
//! `/session`, and a read-only `/audit`. `PUT` / `PATCH` / `DELETE` on `/audit`
//! answer `405` because the audit log is append-only, and so does every
//! method on `/audit/{seq}`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use ocs_collab::protocol::codes;
use ocs_collab::{DrawingState, Operation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::perm;
use crate::store::{AuditQuery, SessionRow};
use crate::ws;
use crate::AppState;

/// Builds the router. Integration tests use this same function.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/session", post(create_session))
        .route("/ws", get(ws::upgrade))
        .route(
            "/audit",
            get(audit_list)
                .put(audit_immutable)
                .patch(audit_immutable)
                .delete(audit_immutable),
        )
        .route("/audit/{seq}", any(audit_immutable))
        .route("/permission/check", post(permission_check))
        .route("/drawing/{id}/state", post(drawing_state))
        .route("/drawing/{id}/objects", get(drawing_objects))
        .route("/drawing/{id}/topology", get(drawing_topology))
        .route("/conflicts", get(conflict_list))
        .route("/conflicts/{id}/resolve", post(conflict_resolve))
        .with_state(state)
}

/// `GET /health` → 200 `{"ok":true,"db":"ok","version":"..."}`, or 503 when the
/// database is unusable (appendix A.1).
async fn health(State(state): State<Arc<AppState>>) -> Response {
    if state.db_healthy() {
        (
            StatusCode::OK,
            Json(json!({ "ok": true, "db": "ok", "version": state.version })),
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "ok": false, "db": "error", "version": state.version })),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
struct CreateSessionBody {
    user_id: String,
    drawing_id: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    workspace: String,
}

fn default_mode() -> String {
    "协同编辑".to_owned()
}

#[derive(Debug, Serialize)]
struct CreateSessionResponse {
    session_id: String,
    base_revision: i64,
}

/// `POST /session` → 201 `{session_id, base_revision}`.
///
/// Two rules beyond persistence: on a `单人编辑` drawing a non-`只读` request
/// is refused with 403 + `PERM_DENIED_STATE` + `holder` while somebody else's
/// editing session is still live, and the endpoint is idempotent for a
/// `(user_id, drawing_id)` pair — the session id is deterministic, so a repost
/// upserts the same row (`ON CONFLICT` refreshes `last_heartbeat`) instead of
/// stacking a duplicate session.
async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::SESSION_UNKNOWN, "数据库不可用");
    };

    if let Err(err) = store.ensure_drawing(&body.drawing_id) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::UNKNOWN_DRAWING, &err);
    }

    let (base_revision, drawing_state) = match store.drawing(&body.drawing_id) {
        Ok(Some((_, state, revision))) => (revision, DrawingState::parse(&state)),
        Ok(None) => (0, None),
        Err(err) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::UNKNOWN_DRAWING, &err)
        }
    };

    let now = chrono::Utc::now().timestamp();

    // The single-editor cap: somebody else's live editing session holds a
    // 单人编辑 drawing. A failed query is only logged — never a panic and
    // never a verdict flip — the same fail-open stance `active_editors_on`
    // takes.
    if drawing_state == Some(DrawingState::SingleEditor) && body.mode != "只读" {
        match store.active_editing_sessions(
            &body.drawing_id,
            now,
            state.session_config.heartbeat_timeout,
        ) {
            Ok(rows) => {
                if let Some((session_id, _)) = rows
                    .iter()
                    .find(|(_, holder_user)| holder_user != &body.user_id)
                {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({
                            "code": codes::PERM_DENIED_STATE,
                            "message": "该图面已有活动的编辑会话",
                            "holder": session_id,
                        })),
                    )
                        .into_response();
                }
            }
            Err(err) => {
                eprintln!(
                    "[ocs_collab_server] active_editing_sessions({}) 失败：{err}",
                    body.drawing_id
                );
            }
        }
    }

    let session_id = session_id_for(&body.user_id, &body.drawing_id);
    let row = SessionRow {
        session_id: session_id.clone(),
        drawing_id: body.drawing_id.clone(),
        user_id: body.user_id.clone(),
        mode: body.mode.clone(),
        status: "活跃".to_owned(),
        base_revision,
        last_heartbeat: now,
        created_at: now,
        workspace: body.workspace.clone(),
    };
    if let Err(err) = store.upsert_session(&row) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err);
    }

    (
        StatusCode::CREATED,
        Json(CreateSessionResponse {
            session_id,
            base_revision,
        }),
    )
        .into_response()
}

/// The id `POST /session` mints for a `(user_id, drawing_id)` pair.
///
/// Unlike `new_session_id()` this is deterministic: a repost of the same pair
/// lands on the same `sessions` row, so `upsert_session`'s `ON CONFLICT` is
/// what refreshes the session — no dedicated lookup is needed, and a stale
/// row whose heartbeat already expired is still reused rather than replaced.
fn session_id_for(user_id: &str, drawing_id: &str) -> String {
    // FNV-1a 64 over the pair under two seeds → 128 bits rendered as an
    // RFC-4122 v4 UUID, keeping the `s-<uuid>` silhouette `new_session_id()`
    // produces for WebSocket sessions.
    let fnv = |seed: u64| {
        user_id
            .bytes()
            .chain([0x1f])
            .chain(drawing_id.bytes())
            .fold(0xcbf2_9ce4_8422_2325_u64 ^ seed, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    };
    let mut bytes =
        ((u128::from(fnv(0x9e37_79b9_7f4a_7c15)) << 64) | u128::from(fnv(0))).to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!("s-{}", Uuid::from_bytes(bytes))
}

#[derive(Debug, Deserialize)]
struct PermissionCheckBody {
    user_id: Option<String>,
    drawing_id: Option<String>,
    operation: Option<String>,
}

/// `POST /permission/check` → the chain's decision. The body is the six
/// `PermissionDecision` fields plus `holder` when a 单人编辑 drawing is held
/// by somebody else's live editing session.
async fn permission_check(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PermissionCheckBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::INVALID_MESSAGE,
            "数据库不可用",
        );
    };
    let (Some(user_id), Some(drawing_id), Some(operation)) =
        (body.user_id, body.drawing_id, body.operation)
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_MESSAGE,
            "缺少必填字段",
        );
    };

    let active_editors = active_editors_on(&state, &drawing_id).await;
    match perm::evaluate(&store, &user_id, &drawing_id, &operation, &active_editors) {
        Ok(outcome) => (outcome.status, Json(decision_json(&outcome))).into_response(),
        Err(err) => perm_error_response(&err),
    }
}

#[derive(Debug, Deserialize)]
struct DrawingStateBody {
    user_id: Option<String>,
    state: Option<String>,
}

/// `POST /drawing/{id}/state` — gated on 授权配置. The requested state value
/// is validated before anything is written; an illegal value is a 400 and
/// nothing is persisted.
async fn drawing_state(
    State(state): State<Arc<AppState>>,
    Path(drawing_id): Path<String>,
    Json(body): Json<DrawingStateBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::INVALID_MESSAGE,
            "数据库不可用",
        );
    };
    let Some(user_id) = body.user_id else {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_MESSAGE,
            "缺少 user_id",
        );
    };

    // Switching state is 授权配置, judged without the state tier: a 只读
    // drawing switches back, and a 单人编辑 one unlocks while held.
    match perm::evaluate_state_switch(&store, &user_id, &drawing_id) {
        Ok(outcome) if outcome.decision.allowed => {}
        Ok(outcome) => return (outcome.status, Json(decision_json(&outcome))).into_response(),
        Err(err) => return perm_error_response(&err),
    }

    let Some(state_text) = body.state else {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_MESSAGE,
            "缺少 state",
        );
    };
    let Some(next_state) = DrawingState::parse(&state_text) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_STATE_TRANSITION,
            "状态值非法",
        );
    };

    // `evaluate` already 404s an unknown drawing; this re-read is for the
    // transition check on the current state.
    let row = match store.drawing(&drawing_id) {
        Ok(row) => row,
        Err(err) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err)
        }
    };
    let Some((_node, current_text, _revision)) = row else {
        return error_response(
            StatusCode::NOT_FOUND,
            codes::PERM_UNKNOWN_SUBJECT,
            "图纸不存在",
        );
    };

    // All three states currently interconvert, so this never fires today; the
    // 409 stays for the day a real transition rule exists.
    if DrawingState::parse(&current_text)
        .is_some_and(|current| !current.can_transition_to(next_state))
    {
        return error_response(
            StatusCode::CONFLICT,
            codes::INVALID_STATE_TRANSITION,
            "状态迁移不允许",
        );
    }

    if let Err(err) = store.set_drawing_state(&drawing_id, &state_text) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err);
    }

    if next_state == DrawingState::ReadOnly {
        settle_readonly(&state, &drawing_id, &user_id).await;
    } else {
        // No longer 只读: there is no edit right to hold.
        state
            .holders
            .lock()
            .expect("holders mutex poisoned")
            .remove(&drawing_id);
    }

    (
        StatusCode::OK,
        Json(json!({ "drawing_id": drawing_id, "state": state_text })),
    )
        .into_response()
}

/// Settles who writes on a drawing that just became 只读, and tells every
/// live session on it (`VAL-CLI-011`).
///
/// One session keeps the edit right and goes on writing: the editing session
/// that has been on the drawing longest (`access::first_editor` — join
/// order, live, editing, and with a socket to be told on). It is named by
/// its session id, so the same user's second session is not mistaken for it.
/// With nobody editing there is no holder, and nobody writes.
///
/// 契约未规定，待用户确认：`VAL-CLI-011` 只说「`u1`（编辑权持有者）」，没有
/// 说以「把图纸状态改成只读」触发降级时谁算持有者。上面这条规则（切换那
/// 一刻在图上时间最长的活跃编辑会话）是实现方的选择，见状态文档 §13.4 之 2
/// 与 §14.3。
///
/// Choosing the holder, recording it and telling the sessions are one step,
/// under the registry's lock. They used to be read at two moments — the
/// editors before the permission check, the sessions to tell after the
/// write — and a holder that left in between was still named, with nobody
/// able to write.
///
/// Every session is told, the holder too: the message names the holder, and
/// a session that finds its own id there knows the edit right is its own.
/// (It used to be told nothing — and a session downgraded once, then made
/// holder by a later switch, went on believing it could not write.) The
/// message is about one session's access, so it goes out on each session's
/// own channel rather than to the room.
///
/// The record of the holder is what the op gate reads (`access::write_access`)
/// and what a session joining later is told.
async fn settle_readonly(state: &AppState, drawing_id: &str, changed_by: &str) {
    let registry = state.sessions.lock().await;
    let direct = state.direct.lock().expect("direct mutex poisoned");
    let holder = registry
        .all()
        .iter()
        .find(|session| {
            session.drawing_id == drawing_id
                && session.is_active()
                && session.is_editing()
                && direct.contains_key(&session.session_id)
        })
        .map(|session| session.session_id.clone());
    {
        let mut holders = state.holders.lock().expect("holders mutex poisoned");
        match &holder {
            Some(holder) => holders.insert(drawing_id.to_owned(), holder.clone()),
            None => holders.remove(drawing_id),
        };
    }
    let message = crate::access::Denied {
        level: ocs_collab::DenialLevel::State,
        reason: "图纸已切换为只读".to_owned(),
        holder: holder.clone(),
    }
    .downgrade();
    let holder_text = holder.as_deref().unwrap_or("(无)");
    let told = registry.active_peers(drawing_id);
    println!(
        "[ocs_collab_server] 图纸切只读 drawing={drawing_id} by={changed_by} holder={holder_text} -> 通知 {} 个会话 at={}",
        told.len(),
        chrono::Utc::now().timestamp()
    );
    for session in &told {
        // A session with no socket (opened over HTTP, or just disconnected)
        // has nobody to tell; it learns its access the next time it asks.
        let delivered = direct
            .get(&session.session_id)
            .is_some_and(|sender| sender.send(message.clone()).is_ok());
        println!(
            "[ocs_collab_server] 单播 readonly_downgrade session={} user={} holder={holder_text} delivered={delivered}",
            session.session_id, session.user_id
        );
    }
}

/// `(session_id, user_id)` of every live editing session on the drawing —
/// the input the single-editor half of the state tier needs.
///
/// The `sessions` table is authoritative (appendix A.3): a session created by
/// `POST /session` lives there and never touches the WebSocket registry, so
/// reading the registry alone made `VAL-PERM-017` unreachable. The in-memory
/// registry is unioned in as well, because a socket that has just opened may
/// not have been mirrored to the table yet; a session present in both is
/// deduplicated by id.
pub(crate) async fn active_editors_on(state: &AppState, drawing_id: &str) -> Vec<(String, String)> {
    let mut live: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    if let Some(store) = state.store.as_ref() {
        let now = chrono::Utc::now().timestamp();
        match store.active_editing_sessions(
            drawing_id,
            now,
            state.session_config.heartbeat_timeout,
        ) {
            Ok(rows) => {
                for (session_id, user_id) in rows {
                    if seen.insert(session_id.clone()) {
                        live.push((session_id, user_id));
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "[ocs_collab_server] active_editing_sessions({drawing_id}) 失败：{err}"
                );
            }
        }
    }

    let registry = state.sessions.lock().await;
    for session in registry.all() {
        if session.drawing_id != drawing_id
            || !session.is_active()
            || !session.is_editing()
        {
            continue;
        }
        if seen.insert(session.session_id.clone()) {
            live.push((session.session_id.clone(), session.user_id.clone()));
        }
    }
    live
}

/// The decision's six serialized fields plus `holder` when the single-editor
/// override produced one.
fn decision_json(outcome: &perm::EvalOutcome) -> serde_json::Value {
    let mut value = serde_json::to_value(&outcome.decision)
        .unwrap_or_else(|_| json!({ "allowed": false }));
    if let Some(holder) = &outcome.holder {
        value["holder"] = json!(holder);
    }
    value
}

fn perm_error_response(err: &perm::PermError) -> Response {
    error_response(err.status_code(), err.code(), &err.message())
}

#[derive(Debug, Deserialize)]
struct AuditParams {
    object_ref: Option<String>,
    tag: Option<String>,
    user_id: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `GET /audit` → 200 `{"items":[...],"total":N}` ordered by `at DESC`.
async fn audit_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditParams>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };

    let query = AuditQuery {
        object_ref: params.object_ref.or(params.tag),
        user_id: params.user_id,
        from: params.from,
        to: params.to,
        limit: params.limit,
        offset: params.offset,
    };
    match store.audit_query(&query) {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

#[derive(Debug, Deserialize)]
struct ObjectsParams {
    object_ref: Option<String>,
}

/// `GET /drawing/{id}/objects[?object_ref=TAG]` → 200
/// `{"drawing_id","revision","items":[{"object_ref","fields":{...}}],"total",
/// "reported_tags":[…]}`.
///
/// The server's own picture of the drawing: every object an op has written,
/// by the tag it goes by now, with the value each field holds, the revision
/// that put it there and whose op it was. It is what a client with no drawing
/// of its own reads a field back from (`VAL-CONF-001`).
///
/// `reported_tags` are the tags the server holds on a client's word alone:
/// in use in that client's own copy of the drawing, told of by no op
/// (`hello.existing_tags`). `total` does not count them.
async fn drawing_objects(
    State(state): State<Arc<AppState>>,
    Path(drawing_id): Path<String>,
    Query(params): Query<ObjectsParams>,
) -> Response {
    let object_ref = params.object_ref.as_deref();
    let (revision, items) = crate::ops::objects(&state, &drawing_id, object_ref);
    let reported_tags = crate::ops::reported_tags(&state, &drawing_id, object_ref);
    let total = items.len();
    (
        StatusCode::OK,
        Json(json!({
            "drawing_id": drawing_id,
            "revision": revision,
            "items": items,
            "total": total,
            "reported_tags": reported_tags,
        })),
    )
        .into_response()
}

/// `GET /drawing/{id}/topology` → 200
/// `{"drawing_id","revision","pipe_lines":[…],"pipe_runs":[…],"connectors":[…]}`:
/// how the drawing's pipe lines are connected up, as the ops that took
/// effect built it (`VAL-CONF-009`).
async fn drawing_topology(
    State(state): State<Arc<AppState>>,
    Path(drawing_id): Path<String>,
) -> Response {
    let (revision, mut topology) = crate::ops::topology(&state, &drawing_id);
    topology["drawing_id"] = json!(drawing_id);
    topology["revision"] = json!(revision);
    (StatusCode::OK, Json(topology)).into_response()
}

#[derive(Debug, Deserialize)]
struct ConflictParams {
    drawing_id: Option<String>,
    status: Option<String>,
}

/// `GET /conflicts[?drawing_id=…&status=…]` → 200 `{"items":[…],"total":N}`
/// (appendix A.1), each item a `conflicts` row.
async fn conflict_list(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ConflictParams>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    match store.list_conflicts(params.drawing_id.as_deref(), params.status.as_deref()) {
        Ok(items) => {
            let total = items.len();
            (StatusCode::OK, Json(json!({ "items": items, "total": total }))).into_response()
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

#[derive(Debug, Deserialize)]
struct ResolveBody {
    resolution: Option<String>,
    /// Who rules. `user_id` is taken as well, as the other write endpoints
    /// spell it.
    resolved_by: Option<String>,
    user_id: Option<String>,
    /// The value for 人工改值: `value` as the validation contract writes the
    /// payload, `resolved_value` as appendix A.1 does.
    value: Option<String>,
    resolved_value: Option<String>,
}

/// `POST /conflicts/{id}/resolve` — body `{resolution, resolved_by, value?}`
/// → 200 `{conflict_id, status:"已裁决", …}`.
///
/// Ruling on a conflict is 授权配置 in the role matrix, judged on the
/// drawing the entry belongs to like any other write; a refusal carries the
/// tier that refused (`level`). An entry ruled on before answers 409 and is
/// left as it is. The value the ruling picks is written to the drawing and
/// broadcast (`ops::resolve`).
async fn conflict_resolve(
    State(state): State<Arc<AppState>>,
    Path(conflict_id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    let Some(resolved_by) = body.resolved_by.or(body.user_id) else {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "缺少 resolved_by");
    };
    let Some(resolution) = body.resolution else {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "缺少 resolution");
    };
    let row = match store.conflict(&conflict_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, codes::CONFLICT_UNKNOWN, "没有这个冲突条目")
        }
        Err(err) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err)
        }
    };

    let active_editors = active_editors_on(&state, &row.drawing_id).await;
    match perm::evaluate(
        &store,
        &resolved_by,
        &row.drawing_id,
        Operation::ConfigureAuthorization.as_str(),
        &active_editors,
    ) {
        Ok(outcome) if outcome.decision.allowed => {}
        Ok(outcome) => {
            // The decision names the tier as `denied_level`; the contract
            // reads it as `level` (`VAL-CONF-015`). Both are given.
            let mut refusal = decision_json(&outcome);
            refusal["level"] = refusal["denied_level"].clone();
            return (outcome.status, Json(refusal)).into_response();
        }
        Err(err) => return perm_error_response(&err),
    }

    let custom = body.value.or(body.resolved_value);
    let resolved = match crate::ops::resolve(
        &state,
        &conflict_id,
        &resolution,
        custom.as_deref(),
        &resolved_by,
        ocs_collab::now_unix(),
    ) {
        Ok(resolved) => resolved,
        Err(crate::ops::ResolveError::Unknown) => {
            return error_response(StatusCode::NOT_FOUND, codes::CONFLICT_UNKNOWN, "没有这个冲突条目")
        }
        Err(crate::ops::ResolveError::AlreadyResolved(row)) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "ok": false,
                    "code": codes::CONFLICT_ALREADY_RESOLVED,
                    "message": "该冲突条目已经裁决过，裁决只做一次",
                    "conflict_id": row.conflict_id,
                    "status": row.status,
                    "resolution": row.resolution,
                    "resolved_by": row.resolved_by,
                })),
            )
                .into_response()
        }
        Err(crate::ops::ResolveError::Invalid(message)) => {
            return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, &message)
        }
        Err(crate::ops::ResolveError::Store(message)) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &message)
        }
    };

    // The ruling took effect as an op: the room has heard it (`ops::resolve`
    // announces it in its turn), and the sessions' baseline follows the
    // revision as it does for any op.
    state
        .sessions
        .lock()
        .await
        .advance_base_revision(&resolved.row.drawing_id, resolved.revision);
    (
        StatusCode::OK,
        Json(json!({
            "conflict_id": resolved.row.conflict_id,
            "status": resolved.row.status,
            "resolution": resolved.row.resolution,
            "resolved_by": resolved.row.resolved_by,
            "resolved_value": resolved.row.resolved_value,
            "object_ref": resolved.row.object_ref,
            "field": resolved.row.field,
            "value": resolved.final_value,
            "revision": resolved.revision,
        })),
    )
        .into_response()
}

/// The audit log has no mutation path at all (invariant 7).
async fn audit_immutable() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({
            "code": "AUDIT_APPEND_ONLY",
            "message": "审计日志只追加，不支持修改或删除",
        })),
    )
        .into_response()
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "code": code, "message": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{AuditPage, NewAuditRow, Store};
    use ocs_collab::SessionConfig;

    fn state_with_memory_store() -> Arc<AppState> {
        let store = Store::open_in_memory().expect("memory store");
        Arc::new(AppState::new(
            Some(store),
            None,
            ":memory:",
            SessionConfig::default(),
        ))
    }

    fn state_without_store() -> Arc<AppState> {
        Arc::new(AppState::new(
            None,
            Some("open failed".into()),
            "bad.db",
            SessionConfig::default(),
        ))
    }

    #[tokio::test]
    async fn health_reports_ok_with_a_working_database() {
        let state = state_with_memory_store();
        assert!(state.db_healthy());
    }

    #[tokio::test]
    async fn health_reports_failure_without_a_database() {
        let state = state_without_store();
        assert!(!state.db_healthy());
    }

    #[test]
    fn the_router_builds_with_every_documented_route() {
        let state = state_with_memory_store();
        let _router = router(state);
    }

    #[tokio::test]
    async fn audit_mutation_returns_405() {
        let response = audit_immutable().await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    /// `?tag=X` must filter exactly like `?object_ref=X` instead of being
    /// silently dropped into an unfiltered dump.
    #[tokio::test]
    async fn audit_tag_filters_the_same_as_object_ref() {
        let state = state_with_memory_store();
        let store = state.store.clone().expect("store");
        for (object_ref, at) in [("AAA-1", 1), ("BBB-2", 2)] {
            store
                .audit_append(&NewAuditRow {
                    module: "P&ID".into(),
                    action: "set_property".into(),
                    at,
                    object_ref: object_ref.into(),
                    user_id: "u1".into(),
                    company: "甲设计院".into(),
                    ip: "127.0.0.1".into(),
                    workspace: "plant-a".into(),
                    change_text: format!("set_property {object_ref}.field: (未知) -> v"),
                })
                .expect("append audit");
        }

        let page = |response: Response| async move {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body bytes");
            serde_json::from_slice::<AuditPage>(&bytes).expect("audit page json")
        };
        let params = |tag: Option<&str>, object_ref: Option<&str>| AuditParams {
            tag: tag.map(str::to_owned),
            object_ref: object_ref.map(str::to_owned),
            user_id: None,
            from: None,
            to: None,
            limit: None,
            offset: None,
        };

        let by_object_ref = page(
            audit_list(State(state.clone()), Query(params(None, Some("AAA-1")))).await,
        )
        .await;
        let by_tag =
            page(audit_list(State(state), Query(params(Some("AAA-1"), None))).await).await;

        assert_eq!(by_object_ref.total, 1);
        assert_eq!(by_tag.total, 1, "?tag must filter like ?object_ref");
        assert_eq!(by_tag.items, by_object_ref.items);
        assert_eq!(by_tag.items[0].object_ref, "AAA-1");
    }

    /// `/audit/<seq>` answers 405 with the append-only body for every
    /// method, and the rejections leave the log's row count and max `seq`
    /// untouched (VAL-CONF-018). Driven over real TCP because `tower` is
    /// not a dependency and `oneshot` is out of reach.
    #[tokio::test]
    async fn audit_seq_requests_return_405_and_leave_the_log_untouched() {
        let state = state_with_memory_store();
        let store = state.store.clone().expect("store");
        let max_seq = store
            .audit_append(&NewAuditRow {
                module: "P&ID".into(),
                action: "set_property".into(),
                at: 1,
                object_ref: "AAA-1".into(),
                user_id: "u1".into(),
                company: "甲设计院".into(),
                ip: "127.0.0.1".into(),
                workspace: "plant-a".into(),
                change_text: "set_property AAA-1.field: (未知) -> v".into(),
            })
            .expect("append audit");
        let count_before = store.audit_count().expect("count");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(state)).await;
        });

        for method in ["GET", "PUT", "PATCH", "DELETE"] {
            let (status, body) = http_exchange(port, method, "/audit/1").await;
            assert_eq!(status, 405, "{method} /audit/1");
            assert_eq!(body["code"], "AUDIT_APPEND_ONLY", "{method} /audit/1");
        }

        assert_eq!(store.audit_count().expect("count"), count_before);
        let page = store
            .audit_query(&AuditQuery::default())
            .expect("audit query");
        assert_eq!(page.items.iter().map(|row| row.seq).max(), Some(max_seq));
    }

    /// One raw HTTP/1.1 exchange with `Connection: close`, read to EOF.
    /// `io-util` is not enabled, so the stream uses `try_read`/`try_write`.
    async fn http_exchange(port: u16, method: &str, path: &str) -> (u16, serde_json::Value) {
        let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        );
        let mut written = 0;
        while written < request.len() {
            stream.writable().await.expect("writable");
            match stream.try_write(&request.as_bytes()[written..]) {
                Ok(n) => written += n,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) => panic!("write: {err}"),
            }
        }
        let mut buf = vec![0u8; 4096];
        let mut raw = Vec::new();
        loop {
            stream.readable().await.expect("readable");
            match stream.try_read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) => panic!("read: {err}"),
            }
        }
        let text = String::from_utf8(raw).expect("utf8 response");
        let (head, body) = text.split_once("\r\n\r\n").expect("header/body split");
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .expect("status code");
        (status, serde_json::from_str(body).expect("json body"))
    }

    /// A store seeded with three users, one root ACL row each, and the two
    /// drawings the permission tests need (协同编辑 + 单人编辑).
    fn seeded_state() -> Arc<AppState> {
        let store = Store::open_in_memory().expect("memory store");
        let doc: crate::store::SeedDoc = serde_json::from_str(
            r#"{
              "users":[
                {"user_id":"u-admin","display_name":"管理员","role":"系统管理员"},
                {"user_id":"u-proc","display_name":"工艺甲","role":"工艺设计人员"},
                {"user_id":"u-view","display_name":"查看者","role":"只读查看者"}
              ],
              "pbs_nodes":[
                {"node_id":"n-plant","name":"工厂","node_type":"工厂"},
                {"node_id":"n-cell-a1","name":"单元A1","node_type":"单元","parent_id":"n-plant"},
                {"node_id":"n-cell-a2","name":"单元A2","node_type":"单元","parent_id":"n-plant"}
              ],
              "pbs_acl":[
                {"node_id":"n-plant","subject":"u-admin","permission":"编辑","allowed":true},
                {"node_id":"n-plant","subject":"u-proc","permission":"编辑","allowed":true},
                {"node_id":"n-plant","subject":"u-view","permission":"查看","allowed":true}
              ],
              "drawings":[
                {"drawing_id":"d-a1-coop","file_name":"a1.dwg","pbs_node_id":"n-cell-a1","state":"协同编辑"},
                {"drawing_id":"d-a2-solo","file_name":"a2.dwg","pbs_node_id":"n-cell-a2","state":"单人编辑"}
              ]
            }"#,
        )
        .expect("seed json");
        store.seed(&doc).expect("seed applies");
        Arc::new(AppState::new(
            Some(store),
            None,
            ":memory:",
            SessionConfig::default(),
        ))
    }

    #[tokio::test]
    async fn permission_check_allows_a_permitted_write() {
        let state = seeded_state();
        let response = permission_check(
            State(state),
            Json(PermissionCheckBody {
                user_id: Some("u-proc".into()),
                drawing_id: Some("d-a2-solo".into()),
                operation: Some("编辑图纸".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn permission_check_maps_errors_to_their_statuses() {
        let state = seeded_state();
        let unknown_drawing = permission_check(
            State(state.clone()),
            Json(PermissionCheckBody {
                user_id: Some("u-admin".into()),
                drawing_id: Some("d-missing".into()),
                operation: Some("查看图纸".into()),
            }),
        )
        .await;
        assert_eq!(unknown_drawing.status(), StatusCode::NOT_FOUND);

        let unknown_user = permission_check(
            State(state.clone()),
            Json(PermissionCheckBody {
                user_id: Some("u-ghost".into()),
                drawing_id: Some("d-a1-coop".into()),
                operation: Some("查看图纸".into()),
            }),
        )
        .await;
        assert_eq!(unknown_user.status(), StatusCode::BAD_REQUEST);

        let bad_operation = permission_check(
            State(state),
            Json(PermissionCheckBody {
                user_id: Some("u-admin".into()),
                drawing_id: Some("d-a1-coop".into()),
                operation: Some("打印图纸".into()),
            }),
        )
        .await;
        assert_eq!(bad_operation.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn permission_check_without_required_fields_is_a_bad_request() {
        let state = seeded_state();
        let response = permission_check(
            State(state),
            Json(PermissionCheckBody {
                user_id: None,
                drawing_id: Some("d-a1-coop".into()),
                operation: Some("查看图纸".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn permission_check_denies_a_second_editor_on_a_solo_drawing() {
        let state = seeded_state();
        {
            let mut sessions = state.sessions.lock().await;
            sessions.open(ocs_collab::OpenRequest {
                session_id: "s-holder",
                drawing_id: "d-a2-solo",
                user_id: "u-admin",
                display_name: "管理员",
                mode: "单人编辑",
                workspace: "",
                base_revision: 0,
                now: 1_000,
            });
        }
        let response = permission_check(
            State(state),
            Json(PermissionCheckBody {
                user_id: Some("u-proc".into()),
                drawing_id: Some("d-a2-solo".into()),
                operation: Some("编辑图纸".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// `VAL-PERM-017` 的真实路径：会话由 `POST /session` 建立，**从不经过**
    /// WebSocket 注册表。`/permission/check` 必须仍能看见它并返回 403 +
    /// `denied_level="state"` + `holder` 等于该 `session_id`。
    ///
    /// 回归背景：`active_editors_on` 曾只读内存注册表，而 `create_session`
    /// 只写库不注册，于是这条断言永远走不到 403。
    #[tokio::test]
    async fn a_session_created_over_http_blocks_a_second_editor() {
        let state = seeded_state();

        let created = create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                user_id: "u-proc".into(),
                drawing_id: "d-a2-solo".into(),
                mode: "单人编辑".into(),
                workspace: String::new(),
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "POST /session 应当建成");
        let body = axum::body::to_bytes(created.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("session response is json");
        let holder = payload["session_id"]
            .as_str()
            .expect("session_id present")
            .to_owned();

        // 内存注册表必须为空 —— 证明下面的 403 来自数据库，不是注册表。
        assert_eq!(
            state.sessions.lock().await.len(),
            0,
            "POST /session 不应写内存注册表，否则这条测试证明不了任何事"
        );

        let response = permission_check(
            State(state),
            Json(PermissionCheckBody {
                user_id: Some("u-admin".into()),
                drawing_id: Some("d-a2-solo".into()),
                operation: Some("编辑图纸".into()),
            }),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "单人编辑图纸上的第 2 个编辑会话必须被拒（VAL-PERM-017）"
        );
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let decision: serde_json::Value =
            serde_json::from_slice(&body).expect("decision is json");
        assert_eq!(
            decision["denied_level"].as_str(),
            Some("state"),
            "拒绝层级应当是 state，实际 {decision}"
        );
        assert_eq!(
            decision["holder"].as_str(),
            Some(holder.as_str()),
            "holder 应当指向 POST /session 建的会话，实际 {decision}"
        );
    }

    /// The HTTP entry enforces the same 单人编辑 cap the permission chain
    /// does: a second live editing session from another user is refused at
    /// `POST /session` with 403 + `PERM_DENIED_STATE` + `holder` naming the
    /// occupying `session_id` — not a fresh row silently coexisting.
    #[tokio::test]
    async fn a_second_editor_is_refused_at_the_http_session_entry() {
        let state = seeded_state();

        let created = create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                user_id: "u-proc".into(),
                drawing_id: "d-a2-solo".into(),
                mode: "单人编辑".into(),
                workspace: String::new(),
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(created.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("session response is json");
        let holder = payload["session_id"]
            .as_str()
            .expect("session_id present")
            .to_owned();

        // Somebody else's editing request on the held 单人编辑 drawing.
        let refused = create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                user_id: "u-admin".into(),
                drawing_id: "d-a2-solo".into(),
                mode: "单人编辑".into(),
                workspace: String::new(),
            }),
        )
        .await;
        assert_eq!(
            refused.status(),
            StatusCode::FORBIDDEN,
            "单人编辑图纸上的第 2 个编辑会话必须被 HTTP 入口拒绝"
        );
        let body = axum::body::to_bytes(refused.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let refusal: serde_json::Value =
            serde_json::from_slice(&body).expect("refusal is json");
        assert_eq!(
            refusal["code"].as_str(),
            Some(codes::PERM_DENIED_STATE),
            "拒绝 code 应当是 PERM_DENIED_STATE，实际 {refusal}"
        );
        assert_eq!(
            refusal["holder"].as_str(),
            Some(holder.as_str()),
            "holder 应当指向占用图纸的会话，实际 {refusal}"
        );

        // The cap is per drawing: the same user opens a different drawing.
        let other = create_session(
            State(state),
            Json(CreateSessionBody {
                user_id: "u-admin".into(),
                drawing_id: "d-a1-coop".into(),
                mode: "单人编辑".into(),
                workspace: String::new(),
            }),
        )
        .await;
        assert_eq!(other.status(), StatusCode::CREATED);
    }

    /// `POST /session` is idempotent for a `(user_id, drawing_id)` pair: a
    /// repost refreshes `last_heartbeat` on the existing row instead of
    /// stacking a duplicate behind a new id. The row is staled to
    /// `last_heartbeat = 0` first so `active_editing_sessions` no longer
    /// returns it — the repost must still find the same session.
    #[tokio::test]
    async fn reposting_the_same_session_refreshes_instead_of_duplicating() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");

        let body = || CreateSessionBody {
            user_id: "u-proc".into(),
            drawing_id: "d-a2-solo".into(),
            mode: "单人编辑".into(),
            workspace: String::new(),
        };

        let first = create_session(State(state.clone()), Json(body())).await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(first.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let payload: serde_json::Value =
            serde_json::from_slice(&bytes).expect("session response is json");
        let session_id = payload["session_id"]
            .as_str()
            .expect("session_id present")
            .to_owned();

        // Age the row out of the heartbeat window so the repost cannot be
        // resolved through the active-session query.
        let mut stale = store
            .session_row(&session_id)
            .expect("lookup")
            .expect("session row exists");
        stale.last_heartbeat = 0;
        store.upsert_session(&stale).expect("stale the heartbeat");

        let before = chrono::Utc::now().timestamp() - 1;
        let second = create_session(State(state), Json(body())).await;
        assert_eq!(
            second.status(),
            StatusCode::CREATED,
            "重复 POST 同一 (user_id, drawing_id) 仍然返回 201"
        );
        let bytes = axum::body::to_bytes(second.into_body(), 64 * 1024)
            .await
            .expect("body readable");
        let repost: serde_json::Value =
            serde_json::from_slice(&bytes).expect("session response is json");
        assert_eq!(
            repost["session_id"].as_str(),
            Some(session_id.as_str()),
            "重复 POST 必须复用已有 session_id，实际 {repost}"
        );

        let row = store
            .session_row(&session_id)
            .expect("lookup")
            .expect("session row exists");
        assert!(
            row.last_heartbeat >= before,
            "last_heartbeat 应被刷新到 repost 时刻，实际 {}",
            row.last_heartbeat
        );
    }

    #[test]
    fn decision_json_merges_the_holder_into_the_six_decision_fields() {
        let outcome = perm::EvalOutcome {
            decision: ocs_collab::PermissionDecision {
                allowed: false,
                denied_level: Some(ocs_collab::DenialLevel::State),
                code: Some("PERM_DENIED_STATE".into()),
                effective_node_id: None,
                effective_node_path: Vec::new(),
                inherited: false,
            },
            status: StatusCode::FORBIDDEN,
            holder: Some("s-9".into()),
        };
        let body = decision_json(&outcome);
        assert_eq!(body["allowed"], false);
        assert_eq!(body["denied_level"], "state");
        assert_eq!(body["code"], "PERM_DENIED_STATE");
        assert!(body.get("effective_node_id").is_some());
        assert!(body.get("effective_node_path").is_some());
        assert!(body.get("inherited").is_some());
        assert_eq!(body["holder"], "s-9");
    }

    #[tokio::test]
    async fn state_change_persists_a_legal_value() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");
        let response = drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("只读".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let (_, stored, _) = store
            .drawing("d-a1-coop")
            .expect("lookup")
            .expect("exists");
        assert_eq!(stored, "只读");
    }

    #[tokio::test]
    async fn state_change_with_an_illegal_value_is_rejected_without_persisting() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");
        let response = drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("四态".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let (_, stored, _) = store
            .drawing("d-a1-coop")
            .expect("lookup")
            .expect("exists");
        assert_eq!(
            stored, "协同编辑",
            "an illegal value must not be persisted"
        );
    }

    #[tokio::test]
    async fn state_change_without_user_id_is_a_bad_request() {
        let state = seeded_state();
        let response = drawing_state(
            State(state),
            Path("d-a1-coop".into()),
            Json(DrawingStateBody {
                user_id: None,
                state: Some("只读".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn state_change_requires_configure_authorization_permission() {
        let state = seeded_state();
        // VAL-PERM-020(a): u-view's role denies 授权配置 before anything else.
        let response = drawing_state(
            State(state),
            Path("d-a1-coop".into()),
            Json(DrawingStateBody {
                user_id: Some("u-view".into()),
                state: Some("只读".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// REQ-COLL-004 acceptance 3: a 只读 drawing switches back to 协同编辑,
    /// after which a write by somebody with 编辑 is no longer a state denial.
    #[tokio::test]
    async fn a_readonly_drawing_switches_back_to_collaborative() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");
        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);
        let refused = perm::evaluate(&store, "u-proc", "d-a1-coop", "编辑图纸", &[]).expect("evaluate");
        assert_eq!(refused.decision.code.as_deref(), Some(codes::PERM_DENIED_STATE));

        assert_eq!(switch_state(&state, "协同编辑").await, StatusCode::OK);
        let (_, stored, _) = store
            .drawing("d-a1-coop")
            .expect("lookup")
            .expect("exists");
        assert_eq!(stored, "协同编辑");
        let allowed = perm::evaluate(&store, "u-proc", "d-a1-coop", "编辑图纸", &[]).expect("evaluate");
        assert!(allowed.decision.allowed);
    }

    /// REQ-COLL-004 rule 1: somebody else's live session on a 单人编辑
    /// drawing does not stop it being unlocked.
    #[tokio::test]
    async fn a_locked_drawing_unlocks_while_somebody_else_edits_it() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");
        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);
        let (_holder, _rx) = live_session(&state, "u-proc", "协同编辑", ocs_collab::now_unix()).await;
        assert_eq!(
            active_editors_on(&state, "d-a1-coop").await.len(),
            1,
            "u-proc holds the drawing"
        );

        assert_eq!(switch_state(&state, "协同编辑").await, StatusCode::OK);
        let (_, stored, _) = store
            .drawing("d-a1-coop")
            .expect("lookup")
            .expect("exists");
        assert_eq!(stored, "协同编辑");
    }

    #[tokio::test]
    async fn state_change_on_an_unknown_drawing_is_not_found() {
        let state = seeded_state();
        let response = drawing_state(
            State(state),
            Path("d-missing".into()),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("只读".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Opens a session on `d-a1-coop` the way a socket does, and hands back
    /// the unicast channel `handle_socket` registers once the handshake is
    /// through.
    async fn live_session(
        state: &AppState,
        user_id: &str,
        mode: &str,
        now: i64,
    ) -> (
        String,
        tokio::sync::mpsc::UnboundedReceiver<ocs_collab::ServerMessage>,
    ) {
        let mut conn = crate::ws::Connection::new("127.0.0.1");
        crate::ws::handle_message(
            state,
            &mut conn,
            ocs_collab::ClientMessage::Hello {
                user_id: user_id.into(),
                drawing_id: "d-a1-coop".into(),
                mode: mode.into(),
                client_version: "test".into(),
                workspace: "plant-a".into(),
                since: None,
                existing_tags: Vec::new(),

                repo_id: None,
            },
            now,
        )
        .await;
        let session_id = conn.session.expect("handshaken").session_id;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        state
            .direct
            .lock()
            .expect("direct mutex")
            .insert(session_id.clone(), sender);
        (session_id, receiver)
    }

    async fn switch_state(state: &Arc<AppState>, next: &str) -> StatusCode {
        drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some(next.into()),
            }),
        )
        .await
        .status()
    }

    /// VAL-CLI-011: a drawing switched to 只读 downgrades every live session on
    /// it except the one holding the edit right — the editing session that
    /// has been on it longest — which goes on writing. Every session is told
    /// who that is, the holder too: finding its own id there is how it knows.
    #[tokio::test]
    async fn switching_to_readonly_downgrades_everyone_but_the_holder() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (_viewer, mut viewer_rx) = live_session(&state, "u-view", "只读", now - 30).await;
        let (holder, mut holder_rx) = live_session(&state, "u-proc", "协同编辑", now - 20).await;
        let (_second, mut second_rx) =
            live_session(&state, "u-admin", "协同编辑", now - 10).await;
        let mut room = state.bus.subscribe("d-a1-coop");

        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        for (who, receiver) in [
            ("the second editor", &mut second_rx),
            ("the session that was read-only already", &mut viewer_rx),
            ("the holder itself", &mut holder_rx),
        ] {
            match receiver.try_recv() {
                Ok(ocs_collab::ServerMessage::ReadonlyDowngrade {
                    reason,
                    holder: named,
                    level,
                    code,
                }) => {
                    assert!(!reason.is_empty(), "{who}: the reason is stated");
                    assert_eq!(named.as_deref(), Some(holder.as_str()), "{who}");
                    assert_eq!(level.as_deref(), Some("state"), "{who}");
                    assert_eq!(code.as_deref(), Some(codes::PERM_DENIED_STATE), "{who}");
                }
                other => panic!("{who} must be told, got {other:?}"),
            }
            assert!(receiver.try_recv().is_err(), "{who} is told once");
        }
        assert_eq!(
            crate::access::holder_of(&state, "d-a1-coop").as_deref(),
            Some(holder.as_str()),
            "and the server remembers who holds it"
        );
        assert!(
            room.try_recv().is_none(),
            "a downgrade is addressed to each session, never published to the room"
        );
    }

    /// Opens a session the way a socket does and keeps the connection, so
    /// the test can go on speaking for it.
    async fn live_conn(
        state: &AppState,
        user_id: &str,
        now: i64,
    ) -> (crate::ws::Connection, Vec<crate::ws::Reaction>) {
        let mut conn = crate::ws::Connection::new("127.0.0.1");
        let reactions = crate::ws::handle_message(
            state,
            &mut conn,
            ocs_collab::ClientMessage::Hello {
                user_id: user_id.into(),
                drawing_id: "d-a1-coop".into(),
                mode: "协同编辑".into(),
                client_version: "test".into(),
                workspace: "plant-a".into(),
                since: None,
                existing_tags: Vec::new(),

                repo_id: None,
            },
            now,
        )
        .await;
        let session_id = conn.session.as_ref().expect("handshaken").session_id.clone();
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        state
            .direct
            .lock()
            .expect("direct mutex")
            .insert(session_id, sender);
        (conn, reactions)
    }

    async fn write(
        state: &AppState,
        conn: &mut crate::ws::Connection,
        op_id: &str,
        value: &str,
    ) -> Vec<crate::ws::Reaction> {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let base_revision = crate::ws::current_revision(state, "d-a1-coop");
        crate::ws::handle_message(
            state,
            conn,
            ocs_collab::ClientMessage::Op {
                session_id,
                base_revision,
                op_id: op_id.into(),
                payload: json!({
                    "kind": "set_property", "object_ref": "P-1001",
                    "field": "设计压力", "value": value,
                }),
            },
            ocs_collab::now_unix(),
        )
        .await
    }

    fn downgrade_in(reactions: &[crate::ws::Reaction]) -> Option<(Option<String>, Option<String>, Option<String>)> {
        reactions.iter().find_map(|reaction| match reaction {
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::ReadonlyDowngrade {
                holder,
                level,
                code,
                ..
            }) => Some((holder.clone(), level.clone(), code.clone())),
            _ => None,
        })
    }

    fn refusal_in(reactions: &[crate::ws::Reaction]) -> Option<(String, Option<String>)> {
        reactions.iter().find_map(|reaction| match reaction {
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, level, .. }) => {
                Some((code.clone(), level.clone()))
            }
            _ => None,
        })
    }

    fn applied_in(reactions: &[crate::ws::Reaction]) -> bool {
        reactions.iter().any(|reaction| {
            matches!(
                reaction,
                crate::ws::Reaction::Published {
                    message: ocs_collab::ServerMessage::OpApplied { .. },
                    ..
                }
            )
        })
    }

    fn denied_rows(state: &AppState, user_id: &str) -> Vec<crate::store::AuditRow> {
        state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                user_id: Some(user_id.to_owned()),
                ..AuditQuery::default()
            })
            .expect("audit")
            .items
            .into_iter()
            .filter(|row| row.action == crate::ws::ACTION_WRITE_DENIED)
            .collect()
    }

    /// `VAL-CROSS-004`: a user whose role does not edit drawings joins as a
    /// viewer whatever mode the client asked for, and is told so as it comes
    /// in — by tier, so its client can answer a refused write the same way.
    #[tokio::test]
    async fn a_role_that_does_not_edit_joins_as_a_viewer_and_is_told() {
        let state = seeded_state();
        let (conn, reactions) = live_conn(&state, "u-view", ocs_collab::now_unix()).await;
        assert_eq!(conn.session.as_ref().expect("session").mode, "只读");
        assert_eq!(
            downgrade_in(&reactions),
            Some((
                None,
                Some("role".to_owned()),
                Some(codes::PERM_DENIED_ROLE.to_owned())
            )),
            "{reactions:?}"
        );
        // And somebody who does edit is told nothing of the kind.
        let (editor, reactions) = live_conn(&state, "u-proc", ocs_collab::now_unix()).await;
        assert_eq!(editor.session.as_ref().expect("session").mode, "协同编辑");
        assert_eq!(downgrade_in(&reactions), None);
    }

    /// The server's own gate. A session that may not write and sends an op
    /// all the same — its client never heard, reconnected, or is another
    /// program — is refused by tier, nothing happens to the drawing, and the
    /// attempt is on record under the object it was aimed at.
    #[tokio::test]
    async fn an_op_from_a_session_that_may_not_write_is_refused_and_put_on_record() {
        let state = seeded_state();
        let (mut checker, _) = live_conn(&state, "u-view", ocs_collab::now_unix()).await;
        let mut room = state.bus.subscribe("d-a1-coop");

        let reactions = write(&state, &mut checker, "op-1", "2.5MPa").await;
        assert_eq!(
            refusal_in(&reactions),
            Some((codes::PERM_DENIED_ROLE.to_owned(), Some("role".to_owned()))),
            "{reactions:?}"
        );
        assert!(!applied_in(&reactions));
        assert!(room.try_recv().is_none(), "the room hears nothing");
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 0);

        let rows = denied_rows(&state, "u-view");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].object_ref, "P-1001");
        assert!(rows[0].change_text.contains("设计压力"), "{}", rows[0].change_text);
        assert!(rows[0].change_text.contains("只读"), "{}", rows[0].change_text);
    }

    /// One `claim` or `release` from `conn`, as the socket would hand it in.
    async fn claim_frame(
        state: &AppState,
        conn: &mut crate::ws::Connection,
        tag: &str,
        claim: bool,
    ) -> Vec<crate::ws::Reaction> {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let tag = tag.to_owned();
        let message = if claim {
            ocs_collab::ClientMessage::Claim { session_id, tag }
        } else {
            ocs_collab::ClientMessage::Release { session_id, tag }
        };
        crate::ws::handle_message(state, conn, message, ocs_collab::now_unix()).await
    }

    /// The `(code, holder)` of the error among `reactions`.
    fn error_in(reactions: &[crate::ws::Reaction]) -> Option<(String, Option<String>)> {
        reactions.iter().find_map(|reaction| match reaction {
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, holder, .. }) => {
                Some((code.clone(), holder.clone()))
            }
            _ => None,
        })
    }

    /// `VAL-CROSS-002`: a tag somebody holds is theirs to change. Another
    /// session's op on it is refused whole — naming the holder, changing
    /// nothing, telling the room nothing, and going on record — until the
    /// holder lets go. The holder's own ops go through all the while. A
    /// claim used to be a note in a table: it refused a second claim and
    /// nothing else.
    #[tokio::test]
    async fn an_op_on_a_tag_another_session_holds_is_refused_until_it_is_released() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut holder, _) = live_conn(&state, "u-proc", now - 10).await;
        let (mut other, _) = live_conn(&state, "u-admin", now).await;
        let holder_id = holder.session.as_ref().expect("session").session_id.clone();
        let granted = claim_frame(&state, &mut holder, "P-1001", true).await;
        assert!(
            granted.iter().any(|reaction| matches!(
                reaction,
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::ClaimGranted { .. })
            )),
            "{granted:?}"
        );
        let mut room = state.bus.subscribe("d-a1-coop");

        assert!(applied_in(&write(&state, &mut holder, "op-h1", "1.6MPa").await));
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 1);
        while room.try_recv().is_some() {}

        let refused = write(&state, &mut other, "op-o1", "2.5MPa").await;
        assert_eq!(
            error_in(&refused),
            Some((codes::CLAIM_HELD.to_owned(), Some(holder_id.clone()))),
            "{refused:?}"
        );
        assert!(!applied_in(&refused));
        assert!(room.try_recv().is_none(), "the room hears nothing");
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 1);
        let rows = denied_rows(&state, "u-admin");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].object_ref, "P-1001");
        assert!(rows[0].change_text.contains("占用"), "{}", rows[0].change_text);
        assert!(rows[0].change_text.contains("u-proc"), "{}", rows[0].change_text);

        // Nor may another object be given the tag that is held.
        let session_id = other.session.as_ref().expect("session").session_id.clone();
        let retag = crate::ws::handle_message(
            &state,
            &mut other,
            ocs_collab::ClientMessage::Op {
                session_id,
                base_revision: 1,
                op_id: "op-o2".into(),
                payload: json!({
                    "kind": "set_tag", "object_ref": "V-2001", "field": "tag", "value": "P-1001",
                }),
            },
            ocs_collab::now_unix(),
        )
        .await;
        assert_eq!(
            error_in(&retag),
            Some((codes::CLAIM_HELD.to_owned(), Some(holder_id))),
            "{retag:?}"
        );

        // Let go of, it is anybody's again.
        claim_frame(&state, &mut holder, "P-1001", false).await;
        assert!(applied_in(&write(&state, &mut other, "op-o3", "2.5MPa").await));
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 2);
    }

    /// `VAL-CROSS-003` beside `-002`: a pipe line somebody holds is theirs
    /// to change in how it is connected up, too. Another session's run on
    /// it, fitting in one of its runs, or new account of such a run is
    /// refused like a write to the line's fields — the claim is on the
    /// line's tag (or the run's), which none of the three names as its
    /// subject. Once the line is let go of they go through.
    #[tokio::test]
    async fn collab_pipe_ops_a_held_line_refuses_changes_to_its_connections() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut holder, _) = live_conn(&state, "u-proc", now - 10).await;
        let (mut other, _) = live_conn(&state, "u-admin", now).await;
        let holder_id = holder.session.as_ref().expect("session").session_id.clone();
        claim_frame(&state, &mut holder, "L-1001", true).await;
        let op = |conn: &crate::ws::Connection, op_id: &str, base: i64, payload: serde_json::Value| {
            ocs_collab::ClientMessage::Op {
                session_id: conn.session.as_ref().expect("session").session_id.clone(),
                base_revision: base,
                op_id: op_id.into(),
                payload,
            }
        };
        let run = json!({"kind": "add_pipe_run", "line_tag": "L-1001", "tag": "L-1001-01",
            "points": [[0, 0], [200, 0]]});
        let fitting = json!({"kind": "insert_fitting", "line_tag": "L-1001", "run_tag": "L-1001-01",
            "fitting_type": "gate_valve", "at": [100, 0], "tag": "V-1001"});
        let update = json!({"kind": "update_pipe_run", "line_tag": "L-1001", "run_tag": "L-1001-01",
            "run": {"segments": [], "fittings": []}});
        for (index, payload) in [&run, &fitting, &update].into_iter().enumerate() {
            let message = op(&other, &format!("op-o{index}"), 0, payload.clone());
            let refused = crate::ws::handle_message(&state, &mut other, message, now).await;
            assert_eq!(
                error_in(&refused),
                Some((codes::CLAIM_HELD.to_owned(), Some(holder_id.clone()))),
                "{payload}: {refused:?}"
            );
        }
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 0);
        // The holder's own go through, and so does anybody's on another line.
        let message = op(&holder, "op-h1", 0, run.clone());
        assert!(applied_in(&crate::ws::handle_message(&state, &mut holder, message, now).await));
        let elsewhere = json!({"kind": "add_pipe_run", "line_tag": "L-1002", "tag": "L-1002-01", "points": []});
        let message = op(&other, "op-o9", 1, elsewhere);
        assert!(applied_in(&crate::ws::handle_message(&state, &mut other, message, now).await));
        // A claim on the run alone stands against a change to that run.
        claim_frame(&state, &mut holder, "L-1001", false).await;
        claim_frame(&state, &mut holder, "L-1001-01", true).await;
        let message = op(&other, "op-o10", 2, fitting.clone());
        let refused = crate::ws::handle_message(&state, &mut other, message, now).await;
        assert_eq!(error_in(&refused).map(|(code, _)| code), Some(codes::CLAIM_HELD.to_owned()));
        claim_frame(&state, &mut holder, "L-1001-01", false).await;
        let message = op(&other, "op-o11", 2, fitting);
        assert!(applied_in(&crate::ws::handle_message(&state, &mut other, message, now).await));
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 3);
    }

    /// `VAL-CROSS-010`: two sessions each place a valve and each calls it
    /// `V-1001` — neither had seen the other's. Both ops take effect; the
    /// second symbol goes to the room as `V-1002`, saying the tag it came
    /// with, and the server knows two objects. No conflict entry: this is
    /// settled by numbering, not by a ruling.
    #[tokio::test]
    async fn two_symbols_placed_under_one_tag_end_up_with_two_tags() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut first, _) = live_conn(&state, "u-proc", now - 10).await;
        let (mut second, _) = live_conn(&state, "u-admin", now).await;
        let place = |conn: &mut crate::ws::Connection, op_id: &str, handle: &str, x: i64| {
            let session_id = conn.session.as_ref().expect("session").session_id.clone();
            ocs_collab::ClientMessage::Op {
                session_id,
                // Both made against the drawing as it was before either.
                base_revision: 0,
                op_id: op_id.into(),
                payload: json!({
                    "kind": "insert_symbol", "object_ref": "V-1001", "tag": "V-1001",
                    "type": "VALVE", "block": "PID_SYM_valve_gate_valve",
                    "position": [x, 200, 0], "handle": handle, "props": {},
                }),
            }
        };
        let changes_of = |reactions: &[crate::ws::Reaction]| -> Vec<serde_json::Value> {
            reactions
                .iter()
                .find_map(|reaction| match reaction {
                    crate::ws::Reaction::Published {
                        message: ocs_collab::ServerMessage::OpApplied { changes, .. },
                        ..
                    } => Some(changes.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };

        let message = place(&mut first, "op-1", "3A1", 100);
        let one = crate::ws::handle_message(&state, &mut first, message, now).await;
        let message = place(&mut second, "op-2", "3A1", 300);
        let two = crate::ws::handle_message(&state, &mut second, message, now).await;

        let (one, two) = (changes_of(&one), changes_of(&two));
        assert_eq!(one.len(), 1, "{one:?}");
        assert_eq!(one[0]["object_ref"], "V-1001");
        assert!(one[0].get("reassigned_from").is_none(), "{one:?}");
        assert_eq!(two.len(), 1, "{two:?}");
        assert_eq!(two[0]["object_ref"], "V-1002");
        assert_eq!(two[0]["tag"], "V-1002");
        assert_eq!(two[0]["reassigned_from"], "V-1001");
        assert_eq!(two[0]["handle"], "3A1");
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 2);

        let store = state.store.as_ref().expect("store");
        let entries = store
            .list_conflicts(Some("d-a1-coop"), None)
            .expect("conflicts");
        assert!(entries.is_empty(), "numbering, not a ruling: {entries:?}");
        let inserted: Vec<String> = store
            .audit_query(&AuditQuery::default())
            .expect("audit")
            .items
            .into_iter()
            .filter(|row| row.action == "insert_symbol")
            .map(|row| format!("{}:{}", row.user_id, row.object_ref))
            .collect();
        assert_eq!(inserted.len(), 2, "{inserted:?}");
        assert!(inserted.contains(&"u-proc:V-1001".to_owned()), "{inserted:?}");
        assert!(inserted.contains(&"u-admin:V-1002".to_owned()), "{inserted:?}");
    }

    /// The body of `GET /drawing/{drawing_id}/objects[?object_ref=…]`.
    async fn objects_listing(
        state: &Arc<AppState>,
        drawing_id: &str,
        object_ref: Option<&str>,
    ) -> serde_json::Value {
        let params = ObjectsParams {
            object_ref: object_ref.map(str::to_owned),
        };
        let (state, path) = (State(state.clone()), Path(drawing_id.to_owned()));
        let response = drawing_objects(state, path, Query(params)).await;
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    /// `GET /drawing/{id}/objects` names the tags the server holds on a
    /// client's word alone (`hello.existing_tags`), beside the objects the
    /// ops told of: the two together are what a new symbol is not called.
    /// With `?object_ref=` it says of one tag whether it is one of them.
    #[tokio::test]
    async fn collab_existing_tags_the_objects_listing_names_the_tags_held_on_a_clients_word() {
        let state = seeded_state();
        let mut conn = crate::ws::Connection::new("127.0.0.1");
        let hello = ocs_collab::ClientMessage::Hello {
            user_id: "u-proc".into(),
            drawing_id: "d-a1-coop".into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "plant-a".into(),
            since: None,
            existing_tags: vec!["V-1002".into(), "P-2001".into()],

            repo_id: None,
        };
        crate::ws::handle_message(&state, &mut conn, hello, ocs_collab::now_unix()).await;

        let all = objects_listing(&state, "d-a1-coop", None).await;
        assert_eq!(all["reported_tags"], json!(["P-2001", "V-1002"]), "{all}");
        assert_eq!(all["total"], 0, "no op told of an object: {all}");
        let one = objects_listing(&state, "d-a1-coop", Some("V-1002")).await;
        assert_eq!(one["reported_tags"], json!(["V-1002"]), "{one}");
        let other = objects_listing(&state, "d-a1-coop", Some("V-1009")).await;
        assert_eq!(other["reported_tags"], json!([]), "{other}");
        // Another drawing's room heard nothing of the kind.
        let elsewhere = objects_listing(&state, "d-a2-solo", None).await;
        assert_eq!(elsewhere["reported_tags"], json!([]), "{elsewhere}");
    }

    /// A claim is on a tag of one drawing. The same tag on another drawing
    /// is another object, and nobody holds that.
    #[tokio::test]
    async fn a_claim_made_on_another_drawing_holds_nothing_on_this_one() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut writer, _) = live_conn(&state, "u-proc", now).await;
        state.occupancy.lock().await.claim(
            "P-1001",
            "d-a2-solo",
            "s-elsewhere",
            "u-admin",
            now,
            300,
        );
        assert!(applied_in(&write(&state, &mut writer, "op-1", "1.6MPa").await));
    }

    /// 只读 has one writer: the holder goes on writing, everybody else is
    /// refused at the state tier and told who holds it. A session that joins
    /// afterwards is told the same as it comes in (it used to be told
    /// nothing, and to write). Out of 只读 again, everybody writes.
    #[tokio::test]
    async fn on_a_readonly_drawing_only_the_holder_writes() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut holder, _) = live_conn(&state, "u-proc", now - 20).await;
        let (mut second, _) = live_conn(&state, "u-admin", now - 10).await;
        let holder_id = holder.session.as_ref().expect("session").session_id.clone();
        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        assert!(applied_in(&write(&state, &mut holder, "op-h", "1.6MPa").await));
        let refused = write(&state, &mut second, "op-s", "2.5MPa").await;
        assert_eq!(
            refusal_in(&refused),
            Some((codes::PERM_DENIED_STATE.to_owned(), Some("state".to_owned()))),
            "{refused:?}"
        );
        assert_eq!(denied_rows(&state, "u-admin").len(), 1);

        // A second session of a user who does edit, joining afterwards.
        let (mut late, reactions) = live_conn(&state, "u-admin", now).await;
        assert_eq!(
            downgrade_in(&reactions),
            Some((
                Some(holder_id),
                Some("state".to_owned()),
                Some(codes::PERM_DENIED_STATE.to_owned())
            )),
            "{reactions:?}"
        );
        assert!(refusal_in(&write(&state, &mut late, "op-l", "4.0MPa").await).is_some());
    }

    /// The holder is a session somebody is behind: one opened over HTTP has
    /// no socket and is passed over, however early it was. And among those
    /// with a socket it is the one that joined first — two sessions opened
    /// in the same second used to be told apart by their random ids.
    #[tokio::test]
    async fn the_holder_is_the_first_to_have_joined_among_sessions_with_a_socket() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        // An editing session with no socket, earlier than everybody.
        {
            let mut registry = state.sessions.lock().await;
            registry.open(ocs_collab::OpenRequest {
                session_id: "s-http-only",
                drawing_id: "d-a1-coop",
                user_id: "u-proc",
                display_name: "u-proc",
                mode: "协同编辑",
                workspace: "plant-a",
                base_revision: 0,
                now: now - 60,
            });
        }
        // Two with a socket, in the same second.
        let (first, _) = live_conn(&state, "u-proc", now).await;
        let (_second, _) = live_conn(&state, "u-admin", now).await;
        let first_id = first.session.as_ref().expect("session").session_id.clone();

        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);
        assert_eq!(
            crate::access::holder_of(&state, "d-a1-coop").as_deref(),
            Some(first_id.as_str())
        );
    }

    /// An `attempt` is a client saying it turned a write away itself. Nothing
    /// happens to the drawing; the record is kept, with the server's own
    /// reading of why the session may not write.
    #[tokio::test]
    async fn an_attempt_is_put_on_record_and_changes_nothing() {
        let state = seeded_state();
        let (mut checker, _) = live_conn(&state, "u-view", ocs_collab::now_unix()).await;
        let session_id = checker.session.as_ref().expect("session").session_id.clone();
        let mut room = state.bus.subscribe("d-a1-coop");

        let reactions = crate::ws::handle_message(
            &state,
            &mut checker,
            ocs_collab::ClientMessage::Attempt {
                session_id,
                object_ref: "P-1001".into(),
                text: "PIDTAG P-1001 P-1009".into(),
            },
            ocs_collab::now_unix(),
        )
        .await;
        assert!(refusal_in(&reactions).is_none(), "{reactions:?}");
        assert!(room.try_recv().is_none());
        assert_eq!(crate::ws::current_revision(&state, "d-a1-coop"), 0);

        let rows = denied_rows(&state, "u-view");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].object_ref, "P-1001");
        assert!(rows[0].change_text.contains("PIDTAG P-1001 P-1009"));
        assert!(rows[0].change_text.contains("角色"), "{}", rows[0].change_text);
    }

    /// A session that may write turns a write away for one other reason —
    /// the tag is somebody else's for now — and that is what goes on record
    /// when it says so: who holds it, not "read-only".
    #[tokio::test]
    async fn an_attempt_on_a_held_tag_is_recorded_with_who_holds_it() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut holder, _) = live_conn(&state, "u-proc", now - 10).await;
        let (mut other, _) = live_conn(&state, "u-admin", now).await;
        claim_frame(&state, &mut holder, "P-1001", true).await;
        let session_id = other.session.as_ref().expect("session").session_id.clone();

        let reactions = crate::ws::handle_message(
            &state,
            &mut other,
            ocs_collab::ClientMessage::Attempt {
                session_id,
                object_ref: "P-1001".into(),
                text: "set_property P-1001.设计压力 = 2.5MPa".into(),
            },
            ocs_collab::now_unix(),
        )
        .await;
        assert!(error_in(&reactions).is_none(), "{reactions:?}");
        let rows = denied_rows(&state, "u-admin");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].object_ref, "P-1001");
        assert!(rows[0].change_text.contains("设计压力"), "{}", rows[0].change_text);
        assert!(rows[0].change_text.contains("占用"), "{}", rows[0].change_text);
        assert!(rows[0].change_text.contains("u-proc"), "{}", rows[0].change_text);
        assert!(!rows[0].change_text.contains("只读"), "{}", rows[0].change_text);
    }

    /// 单人编辑 with two users on the drawing: the one that was there first
    /// writes, the other does not. The chain alone says "somebody else is
    /// editing" of both of them.
    #[tokio::test]
    async fn on_a_single_editor_drawing_the_first_editor_writes_and_the_second_does_not() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut first, _) = live_conn(&state, "u-proc", now - 20).await;
        let (mut second, _) = live_conn(&state, "u-admin", now - 10).await;
        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);

        assert!(applied_in(&write(&state, &mut first, "op-1", "1.6MPa").await));
        let refused = write(&state, &mut second, "op-2", "2.5MPa").await;
        assert_eq!(
            refusal_in(&refused),
            Some((codes::PERM_DENIED_STATE.to_owned(), Some("state".to_owned()))),
            "{refused:?}"
        );
    }

    /// With nobody editing there is no edit right to hold: everybody on the
    /// drawing is downgraded and the message names no holder.
    #[tokio::test]
    async fn switching_to_readonly_with_nobody_editing_names_no_holder() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (_viewer, mut viewer_rx) = live_session(&state, "u-view", "只读", now - 5).await;

        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        match viewer_rx.try_recv() {
            Ok(ocs_collab::ServerMessage::ReadonlyDowngrade { holder, .. }) => {
                assert_eq!(holder, None)
            }
            other => panic!("the viewer must be downgraded, got {other:?}"),
        }
    }

    /// Only 只读 takes write access away; any other state leaves every session
    /// as it was.
    #[tokio::test]
    async fn a_state_other_than_readonly_downgrades_nobody() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (_first, mut first_rx) = live_session(&state, "u-proc", "协同编辑", now - 20).await;
        let (_second, mut second_rx) =
            live_session(&state, "u-admin", "协同编辑", now - 10).await;

        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);

        assert!(first_rx.try_recv().is_err());
        assert!(second_rx.try_recv().is_err());
    }

    // ── Conflict adjudication (`VAL-CONF-010` … `016`) ──────────────────

    const PID: &str = "PID-1101";

    /// A server with a tie window, the contract's four users, and one
    /// session each for u1 and u2 on PID-1101.
    async fn adjudication_state() -> (Arc<AppState>, ws::Connection, ws::Connection) {
        let store = Store::open_in_memory().expect("memory store");
        let doc: crate::store::SeedDoc = serde_json::from_str(
            r#"{
              "users":[
                {"user_id":"u1","display_name":"工艺甲","role":"工艺设计人员","company":"甲方工程公司"},
                {"user_id":"u2","display_name":"工艺乙","role":"工艺设计人员","company":"甲方工程公司"},
                {"user_id":"u3","display_name":"校对丙","role":"校对人员","company":"甲方工程公司"},
                {"user_id":"u4","display_name":"管理员丁","role":"项目管理员","company":"甲方工程公司"}
              ],
              "pbs_nodes":[{"node_id":"P-ROOT","name":"工厂","node_type":"工厂"}],
              "pbs_acl":[
                {"node_id":"P-ROOT","subject":"u1","permission":"编辑","allowed":true},
                {"node_id":"P-ROOT","subject":"u2","permission":"编辑","allowed":true},
                {"node_id":"P-ROOT","subject":"u3","permission":"查看","allowed":true},
                {"node_id":"P-ROOT","subject":"u4","permission":"编辑","allowed":true}
              ],
              "drawings":[
                {"drawing_id":"PID-1101","file_name":"PID-1101.dwg","pbs_node_id":"P-ROOT","state":"协同编辑"}
              ]
            }"#,
        )
        .expect("seed json");
        store.seed(&doc).expect("seed applies");
        let mut state = AppState::new(Some(store), None, ":memory:", SessionConfig::default());
        state.tie_window = std::time::Duration::from_millis(500);
        let state = Arc::new(state);
        let mut sessions = Vec::new();
        for user in ["u1", "u2"] {
            let mut conn = ws::Connection::new("10.0.0.7");
            let hello = ocs_collab::ClientMessage::Hello {
                user_id: user.into(),
                drawing_id: PID.into(),
                mode: "协同编辑".into(),
                client_version: "test".into(),
                workspace: "plant-a".into(),
                since: None,
                existing_tags: Vec::new(),

                repo_id: None,
            };
            ws::handle_message(&state, &mut conn, hello, 1_700_000_000).await;
            sessions.push(conn);
        }
        let s2 = sessions.pop().expect("s2");
        let s1 = sessions.pop().expect("s1");
        (state, s1, s2)
    }

    async fn submit(state: &AppState, conn: &mut ws::Connection, op_id: &str, payload: serde_json::Value) {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let base_revision = crate::ops::objects(state, PID, None).0;
        let message = ocs_collab::ClientMessage::Op {
            session_id,
            base_revision,
            op_id: op_id.into(),
            payload,
        };
        ws::handle_message(state, conn, message, 1_700_000_001).await;
    }

    /// Have u1 and u2 tie on `field` of P-1001 and return the entry's id.
    async fn tie_on(
        state: &AppState,
        s1: &mut ws::Connection,
        s2: &mut ws::Connection,
        field: &str,
        (a, b): (&str, &str),
    ) -> String {
        let write = |value: &str| {
            json!({"kind": "set_property", "object_ref": "P-1001", "field": field, "value": value})
        };
        let before: Vec<String> = pending(state);
        submit(state, s1, &format!("{field}-a"), write(a)).await;
        submit(state, s2, &format!("{field}-b"), write(b)).await;
        pending(state)
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("the two writes tied")
    }

    fn pending(state: &AppState) -> Vec<String> {
        state
            .store
            .as_ref()
            .expect("store")
            .list_conflicts(Some(PID), Some("待裁决"))
            .expect("conflicts")
            .into_iter()
            .map(|row| row.conflict_id)
            .collect()
    }

    async fn rule(
        state: &Arc<AppState>,
        conflict_id: &str,
        by: &str,
        resolution: &str,
        value: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let response = conflict_resolve(
            State(state.clone()),
            Path(conflict_id.to_owned()),
            Json(ResolveBody {
                resolution: Some(resolution.into()),
                resolved_by: Some(by.into()),
                user_id: None,
                value: value.map(str::to_owned),
                resolved_value: None,
            }),
        )
        .await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, serde_json::from_slice(&bytes).expect("json body"))
    }

    fn held_value(state: &AppState, field: &str) -> Option<String> {
        crate::ops::objects(state, PID, Some("P-1001"))
            .1
            .first()
            .and_then(|item| item["fields"][field]["value"].as_str().map(str::to_owned))
    }

    fn entry(state: &AppState, conflict_id: &str) -> crate::store::ConflictRow {
        state
            .store
            .as_ref()
            .expect("store")
            .conflict(conflict_id)
            .expect("read")
            .expect("the entry")
    }

    // `VAL-CONF-010`, `-011`, `-012`, `-013`: each of the three rulings
    // marks the entry, writes the value it picks to the drawing — as an op
    // the room hears — and leaves a record carrying both values and the one
    // that stood.
    #[tokio::test]
    async fn a_ruling_writes_the_value_it_picks_and_is_on_record() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        let mut room = state.bus.subscribe(PID);
        let cases = [
            ("设计压力", "采用甲方", None, "1.6MPa"),
            ("公称直径", "采用乙方", None, "2.5MPa"),
            ("绝热等级", "人工改值", Some("2.0MPa"), "2.0MPa"),
        ];
        for (field, resolution, custom, expected) in cases {
            let id = tie_on(&state, &mut s1, &mut s2, field, ("1.6MPa", "2.5MPa")).await;
            assert_eq!(held_value(&state, field), None, "nothing stands before the ruling");
            while room.try_recv().is_some() {}

            let (status, body) = rule(&state, &id, "u4", resolution, custom).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["status"], "已裁决");
            assert_eq!(body["conflict_id"], id.as_str());

            let row = entry(&state, &id);
            assert_eq!(row.status, "已裁决");
            assert_eq!(row.resolution.as_deref(), Some(resolution));
            assert_eq!(row.resolved_by.as_deref(), Some("u4"));
            assert_eq!(row.resolved_value.as_deref(), custom, "only a value given by hand is kept");
            assert_eq!(held_value(&state, field).as_deref(), Some(expected), "{resolution}");

            match room.try_recv() {
                Some(ocs_collab::ServerMessage::OpApplied { changes, revision, .. }) => {
                    assert_eq!(changes[0]["field"], field);
                    assert_eq!(changes[0]["value"], expected);
                    assert_eq!(body["revision"], revision);
                }
                other => panic!("the room should hear the ruling as an op: {other:?}"),
            }
            let rows = state
                .store
                .as_ref()
                .expect("store")
                .audit_query(&AuditQuery {
                    user_id: Some("u4".into()),
                    ..AuditQuery::default()
                })
                .expect("audit")
                .items;
            let record = rows
                .iter()
                .find(|row| row.change_text.contains(&id))
                .expect("the ruling is on record");
            assert_eq!(record.object_ref, "P-1001");
            for value in ["1.6MPa", "2.5MPa", expected] {
                assert!(record.change_text.contains(value), "{}", record.change_text);
            }
        }
    }

    // `VAL-CONF-014`: a ruling is made once.
    #[tokio::test]
    async fn a_second_ruling_is_refused_and_changes_nothing() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        let id = tie_on(&state, &mut s1, &mut s2, "设计压力", ("1.6MPa", "2.5MPa")).await;
        let (status, _) = rule(&state, &id, "u4", "采用甲方", None).await;
        assert_eq!(status, StatusCode::OK);
        let store = state.store.as_ref().expect("store");
        let audit_before = store.audit_count().expect("count");

        let (status, body) = rule(&state, &id, "u4", "采用乙方", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["ok"], false);
        assert_eq!(body["code"], codes::CONFLICT_ALREADY_RESOLVED);
        let row = entry(&state, &id);
        assert_eq!(row.resolution.as_deref(), Some("采用甲方"));
        assert_eq!(row.resolved_by.as_deref(), Some("u4"));
        assert_eq!(store.audit_count().expect("count"), audit_before);
        assert_eq!(held_value(&state, "设计压力").as_deref(), Some("1.6MPa"));
    }

    // `VAL-CONF-015`: ruling takes 授权配置. A reviewer's role does not have
    // it; the refusal says which tier refused, and the entry stays pending.
    #[tokio::test]
    async fn a_ruling_by_someone_who_may_not_configure_is_refused_at_the_role_tier() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        let id = tie_on(&state, &mut s1, &mut s2, "设计压力", ("1.6MPa", "2.5MPa")).await;
        let (status, body) = rule(&state, &id, "u3", "采用甲方", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], codes::PERM_DENIED_ROLE);
        assert_eq!(body["level"], "role");
        let row = entry(&state, &id);
        assert_eq!(row.status, "待裁决");
        assert_eq!(row.resolution, None);
        assert_eq!(held_value(&state, "设计压力"), None);
    }

    // `VAL-CONF-016`: entries are listed by drawing and by status.
    #[tokio::test]
    async fn conflicts_are_listed_by_drawing_and_status() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        for field in ["设计压力", "公称直径", "绝热等级"] {
            let id = tie_on(&state, &mut s1, &mut s2, field, ("a", "b")).await;
            rule(&state, &id, "u4", "采用甲方", None).await;
        }
        tie_on(&state, &mut s1, &mut s2, "流体介质", ("水", "油")).await;

        let list = |status: &'static str, drawing: &'static str| {
            let state = state.clone();
            async move {
                let response = conflict_list(
                    State(state),
                    Query(ConflictParams {
                        drawing_id: Some(drawing.into()),
                        status: Some(status.into()),
                    }),
                )
                .await;
                assert_eq!(response.status(), StatusCode::OK);
                let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .expect("body");
                serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")
            }
        };
        let waiting = list("待裁决", PID).await;
        assert_eq!(waiting["total"], 1);
        let item = &waiting["items"][0];
        for key in ["conflict_id", "object_ref", "field", "status"] {
            assert!(item[key].as_str().is_some_and(|text| !text.is_empty()), "{item}");
        }
        assert_eq!(list("已裁决", PID).await["items"].as_array().map(Vec::len), Some(3));
        assert_eq!(list("待裁决", "PID-9999").await["total"], 0);
    }

    // A ruling on a pipe line's connections applies the change it picks.
    #[tokio::test]
    async fn a_ruling_on_a_topology_entry_applies_the_change_it_picks() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        submit(&state, &mut s1, "run", json!({"kind": "add_pipe_run", "line_tag": "L-1001", "points": [[0, 0], [100, 0]]})).await;
        submit(&state, &mut s2, "fit", json!({"kind": "insert_fitting", "run_tag": "L-1001-01", "fitting_type": "gate_valve", "at": [50, 0]})).await;
        let id = pending(&state).pop().expect("the two changes tied");
        assert_eq!(crate::ops::topology(&state, PID).1["pipe_runs"], json!([]));

        let (status, body) = rule(&state, &id, "u4", "采用甲方", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, topology) = crate::ops::topology(&state, PID);
        assert_eq!(topology["pipe_runs"][0]["tag"], "L-1001-01");
        assert_eq!(topology["pipe_runs"][0]["points"], json!([[0, 0], [100, 0]]));
    }

    #[tokio::test]
    async fn a_ruling_that_cannot_be_read_is_refused_without_touching_the_entry() {
        let (state, mut s1, mut s2) = adjudication_state().await;
        let id = tie_on(&state, &mut s1, &mut s2, "设计压力", ("1.6MPa", "2.5MPa")).await;
        let (status, body) = rule(&state, "c-no-such-entry", "u4", "采用甲方", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], codes::CONFLICT_UNKNOWN);
        let (status, _) = rule(&state, &id, "u4", "人工改值", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a value by hand needs the value");
        let (status, _) = rule(&state, &id, "u4", "掷硬币", None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(entry(&state, &id).status, "待裁决");
    }

    // ── Co-editing sessions and the audit log (`VAL-PERM-018`, `-025`,
    // `-028`, `-030`, `-031`, `-032`) ───────────────────────────────────

    /// The company the users table holds for `u-proc`.
    const COMPANY: &str = "甲方工程公司";

    /// [`seeded_state`] with what the audit rows are checked against added
    /// to the users table: a company for `u-proc`, and `u-check`, who
    /// proofreads and so may look at `d-a1-coop` and not change it.
    fn audited_state() -> Arc<AppState> {
        let state = seeded_state();
        let doc: crate::store::SeedDoc = serde_json::from_str(
            r#"{
              "users":[
                {"user_id":"u-proc","display_name":"工艺甲","role":"工艺设计人员","company":"甲方工程公司"},
                {"user_id":"u-check","display_name":"校对乙","role":"校对人员","company":"甲方工程公司"}
              ],
              "pbs_acl":[
                {"node_id":"n-plant","subject":"u-check","permission":"查看","allowed":true}
              ]
            }"#,
        )
        .expect("seed json");
        let store = state.store.as_ref().expect("store");
        store.seed(&doc).expect("seed applies");
        state
    }

    /// Serves `state` on a loopback port of the system's choosing and says
    /// which.
    async fn serve(state: &Arc<AppState>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let state = state.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router(state)).await;
        });
        port
    }

    /// The rows `GET /audit?<query>` answers with over the wire, newest
    /// first. Any status but 200, or a body without a list of rows, fails
    /// the test.
    async fn audit_rows(port: u16, query: &str) -> Vec<serde_json::Value> {
        let path = format!("/audit?{query}");
        let (status, body) = http_exchange(port, "GET", &path).await;
        assert_eq!(status, 200, "GET {path}: {body}");
        match body["items"].as_array() {
            Some(rows) => rows.clone(),
            None => panic!("GET {path} answers with rows: {body}"),
        }
    }

    /// `frame` read the way the socket loop reads a text frame.
    fn off_the_wire(frame: serde_json::Value) -> ocs_collab::ClientMessage {
        serde_json::from_str(&frame.to_string()).expect("a client message")
    }

    /// Opens `user_id`'s session on `d-a1-coop` the way a socket's `hello`
    /// does, from `peer` and with `workspace` declared.
    async fn session_from(
        state: &AppState,
        user_id: &str,
        peer: &str,
        workspace: &str,
        now: i64,
    ) -> crate::ws::Connection {
        let mut conn = crate::ws::Connection::new(peer);
        let hello = ocs_collab::ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: "d-a1-coop".into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: workspace.into(),
            since: None,
            existing_tags: Vec::new(),

            repo_id: None,
        };
        crate::ws::handle_message(state, &mut conn, hello, now).await;
        assert!(conn.session.is_some(), "{user_id} is let in");
        conn
    }

    /// One `set_property` from `conn` on `d-a1-coop`, made against the
    /// revision the drawing is at, as the socket would hand it in. The
    /// payload says what the field becomes and not what it was.
    async fn set_field(
        state: &AppState,
        conn: &mut crate::ws::Connection,
        (object_ref, field, value): (&str, &str, &str),
        now: i64,
    ) -> Vec<crate::ws::Reaction> {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let op = ocs_collab::ClientMessage::Op {
            session_id,
            base_revision: crate::ws::current_revision(state, "d-a1-coop"),
            op_id: format!("{object_ref}.{field}={value}"),
            payload: json!({
                "kind": "set_property", "object_ref": object_ref,
                "field": field, "value": value,
            }),
        };
        crate::ws::handle_message(state, conn, op, now).await
    }

    /// `VAL-PERM-018`: a 协同编辑 drawing does not stop at one editing
    /// session. With the drawing at revision 3, two users open a session on
    /// it — both on that baseline — and each of them may still edit with
    /// the other one there: allowed, no tier named, nobody holding it.
    #[tokio::test]
    async fn a_coediting_drawing_takes_a_second_editing_session_and_both_may_edit() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let mut writer = session_from(&state, "u-proc", "10.0.0.7", "ws-alpha", now).await;
        for value in ["1.0MPa", "1.6MPa", "2.0MPa"] {
            let change = ("V-1001", "设计压力", value);
            let reactions = set_field(&state, &mut writer, change, now).await;
            assert!(applied_in(&reactions), "{reactions:?}");
        }

        let mut sessions = Vec::new();
        for user in ["u-proc", "u-admin"] {
            let created = create_session(
                State(state.clone()),
                Json(CreateSessionBody {
                    user_id: user.into(),
                    drawing_id: "d-a1-coop".into(),
                    mode: "协同编辑".into(),
                    workspace: String::new(),
                }),
            )
            .await;
            assert_eq!(
                created.status(),
                StatusCode::CREATED,
                "{user} opens a session"
            );
            let body = axum::body::to_bytes(created.into_body(), 64 * 1024)
                .await
                .expect("body readable");
            let session: serde_json::Value =
                serde_json::from_slice(&body).expect("session response is json");
            assert_eq!(session["base_revision"], 3, "{user}: {session}");
            sessions.push(session["session_id"].clone());
        }
        assert_ne!(sessions[0], sessions[1], "two sessions, not one");

        for user in ["u-proc", "u-admin"] {
            let response = permission_check(
                State(state.clone()),
                Json(PermissionCheckBody {
                    user_id: Some(user.into()),
                    drawing_id: Some("d-a1-coop".into()),
                    operation: Some("编辑图纸".into()),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{user} may edit");
            let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body readable");
            let decision: serde_json::Value =
                serde_json::from_slice(&body).expect("decision is json");
            assert_eq!(decision["allowed"], true, "{user}: {decision}");
            assert!(decision["denied_level"].is_null(), "{user}: {decision}");
            assert!(decision.get("holder").is_none(), "{user}: {decision}");
        }
    }

    /// `VAL-PERM-025`: the row an edit leaves states all nine things, none
    /// of them empty, and each as the server has it: the company from the
    /// users table, the address the connection came from, the workspace the
    /// session declared, the time the op arrived.
    #[tokio::test]
    async fn an_edit_leaves_an_audit_row_with_all_nine_fields_stated() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();
        let mut conn = session_from(&state, "u-proc", "10.0.0.7", "ws-alpha", now).await;
        for value in ["1.6MPa", "2.0MPa"] {
            let change = ("V-1001", "设计压力", value);
            let reactions = set_field(&state, &mut conn, change, now).await;
            assert!(applied_in(&reactions), "{reactions:?}");
        }

        let rows = audit_rows(port, "tag=V-1001").await;
        let row = rows.first().expect("the edits are on record");
        for field in [
            "module",
            "action",
            "object_ref",
            "user_id",
            "company",
            "ip",
            "workspace",
            "change_text",
        ] {
            let stated = row[field].as_str().is_some_and(|text| !text.is_empty());
            assert!(stated, "{field} is stated: {row}");
        }
        assert_eq!(row["at"], now, "{row}");
        assert_eq!(row["object_ref"], "V-1001", "{row}");
        assert_eq!(row["user_id"], "u-proc", "{row}");
        assert_eq!(row["company"], COMPANY, "{row}");
        assert_eq!(row["ip"], "10.0.0.7", "{row}");
        assert_eq!(row["workspace"], "ws-alpha", "{row}");
        let text = row["change_text"].as_str().unwrap_or_default();
        assert!(
            text.contains("2.0MPa"),
            "the newest row is the last edit: {row}"
        );
    }

    /// `VAL-PERM-028`: `GET /audit?user_id=` answers with that user's rows
    /// and nobody else's. `u-proc` changes two objects; `u-check`, whose
    /// role does not edit, tries two others and is turned away, which goes
    /// on record under each of them. Each one's query names exactly their
    /// own two objects, so the two answers share nothing.
    #[tokio::test]
    async fn audit_user_id_answers_with_that_users_rows_and_no_others() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();
        let mut editor = session_from(&state, "u-proc", "10.0.0.7", "ws-alpha", now).await;
        let mut checker = session_from(&state, "u-check", "10.0.0.8", "ws-alpha", now).await;
        for tag in ["V-3001", "V-3002"] {
            let change = (tag, "设计压力", "1.6MPa");
            let reactions = set_field(&state, &mut editor, change, now).await;
            assert!(applied_in(&reactions), "{tag}: {reactions:?}");
        }
        for tag in ["V-3003", "V-3004"] {
            let change = (tag, "设计压力", "1.6MPa");
            let reactions = set_field(&state, &mut checker, change, now).await;
            assert!(refusal_in(&reactions).is_some(), "{tag}: {reactions:?}");
            assert!(!applied_in(&reactions), "{tag}: {reactions:?}");
        }

        for (user, theirs) in [
            ("u-proc", ["V-3001", "V-3002"]),
            ("u-check", ["V-3003", "V-3004"]),
        ] {
            let rows = audit_rows(port, &format!("user_id={user}")).await;
            assert!(
                rows.iter().all(|row| row["user_id"] == user),
                "{user}: {rows:?}"
            );
            let mut objects: Vec<&str> = rows
                .iter()
                .filter_map(|row| row["object_ref"].as_str())
                .collect();
            objects.sort_unstable();
            assert_eq!(objects, theirs, "{user}: {rows:?}");
        }
    }

    /// `VAL-PERM-030`: the row a change leaves says what the field was and
    /// what it became, by name — the old value from what the server held,
    /// since the op itself only says the new one.
    #[tokio::test]
    async fn a_change_is_recorded_with_the_old_value_the_new_value_and_the_field() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();
        let mut conn = session_from(&state, "u-proc", "10.0.0.7", "ws-alpha", now).await;
        for value in ["DN50", "DN80"] {
            let change = ("V-5001", "公称直径", value);
            let reactions = set_field(&state, &mut conn, change, now).await;
            assert!(applied_in(&reactions), "{reactions:?}");
        }

        let rows = audit_rows(port, "tag=V-5001").await;
        let newest = rows.first().expect("the changes are on record");
        let text = newest["change_text"].as_str().unwrap_or_default();
        for part in ["DN50", "DN80", "公称直径"] {
            assert!(text.contains(part), "{part} is in the record: {text}");
        }
    }

    /// `VAL-PERM-031`: a query nothing matches is answered 200 with an
    /// empty list — not 404, not an error, not `null` — for a tag no row
    /// names and for a user who is not in the users table alike. The log
    /// itself is not empty when they are asked.
    #[tokio::test]
    async fn an_audit_query_nothing_matches_answers_200_with_an_empty_list() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();
        let mut conn = session_from(&state, "u-proc", "10.0.0.7", "ws-alpha", now).await;
        let change = ("V-7001", "材质", "Q345R");
        let reactions = set_field(&state, &mut conn, change, now).await;
        assert!(applied_in(&reactions), "{reactions:?}");
        assert_eq!(audit_rows(port, "tag=V-7001").await.len(), 1);

        for query in ["tag=NOT-EXIST-9999", "user_id=u-ghost"] {
            let path = format!("/audit?{query}");
            let (status, body) = http_exchange(port, "GET", &path).await;
            assert_eq!(status, 200, "{path}: {body}");
            assert_eq!(body["items"], json!([]), "{path}: {body}");
            assert_eq!(body["total"], 0, "{path}: {body}");
        }
    }

    /// `VAL-PERM-032`: what a client says of itself beyond the protocol's
    /// own fields is not read. A `hello` that also names an address and a
    /// company, and an op that names an address, a workspace and a company
    /// — beside its payload and inside it — leave a row with the address
    /// the connection came from, the company the users table has, and the
    /// workspace of a session that declared none. The forged workspace
    /// rides on the op and not on the `hello`: `workspace` is one of
    /// `hello`'s own fields, where a session declares the one it works in
    /// (`VAL-PERM-025`).
    #[tokio::test]
    async fn an_audit_row_takes_nothing_from_keys_the_client_slipped_in() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();
        let mut conn = crate::ws::Connection::new("10.0.0.7");
        let hello = off_the_wire(json!({
            "type": "hello", "user_id": "u-proc", "drawing_id": "d-a1-coop",
            "mode": "协同编辑", "ip": "10.9.9.9", "company": "伪造公司",
        }));
        crate::ws::handle_message(&state, &mut conn, hello, now).await;
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let op = off_the_wire(json!({
            "type": "op", "session_id": session_id, "base_revision": 0, "op_id": "op-forged",
            "ip": "10.9.9.9", "workspace": "ws-forged", "company": "伪造公司",
            "payload": {
                "kind": "set_property", "object_ref": "V-6001",
                "field": "设计压力", "value": "1.6MPa",
                "ip": "10.9.9.9", "workspace": "ws-forged", "company": "伪造公司",
            },
        }));
        let reactions = crate::ws::handle_message(&state, &mut conn, op, now).await;
        assert!(applied_in(&reactions), "{reactions:?}");

        let rows = audit_rows(port, "tag=V-6001").await;
        let row = rows.first().expect("the edit is on record");
        assert_eq!(row["ip"], "10.0.0.7", "{row}");
        assert_eq!(row["company"], COMPANY, "{row}");
        assert_eq!(row["workspace"], "默认工作区", "{row}");
    }
}

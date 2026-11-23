//! axum HTTP control plane.
//!
//! The scaffold serves the endpoints later milestones build on: `/health`,
//! `/session`, and a read-only `/audit`. `PUT` / `PATCH` / `DELETE` on `/audit`
//! answer `405` because the audit log is append-only, and so does every
//! method on `/audit/{seq}`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
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
        .route(
            "/metrics",
            get(metrics_snapshot)
                .post(readonly_endpoint)
                .put(readonly_endpoint)
                .patch(readonly_endpoint)
                .delete(readonly_endpoint),
        )
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
        .route("/users", get(user_list))
        .route("/users/{id}/role", post(user_role_set))
        .route("/roles", get(role_list))
        .route("/pbs/nodes", get(pbs_node_list))
        .route(
            "/pbs/acl",
            get(pbs_acl_list).put(pbs_acl_put).delete(pbs_acl_delete),
        )
        .route("/drawing/{id}/state", post(drawing_state))
        .route("/drawing/{id}/objects", get(drawing_objects))
        .route("/drawing/{id}/topology", get(drawing_topology))
        .route(
            "/drawing/{id}/unpaired-connectors",
            get(drawing_unpaired_connectors),
        )
        .route("/conflicts", get(conflict_list))
        .route("/conflicts/{id}/resolve", post(conflict_resolve))
        .route("/admin/backup", post(admin_backup))
        .with_state(state)
}

/// `GET /health` → 200 `{"ok":true,"db":"ok","version":"...","open_mode":...,"auth_enabled":...,"tls_enabled":...}`,
/// or 503 when the database is unusable (appendix A.1, #67, #68, #69).
async fn health(State(state): State<Arc<AppState>>) -> Response {
    if state.db_healthy() {
        (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "db": "ok",
                "version": state.version,
                "open_mode": state.open_mode,
                "auth_enabled": state.require_auth,
                "tls_enabled": state.tls_enabled,
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "db": "error",
                "version": state.version,
                "open_mode": state.open_mode,
                "auth_enabled": state.require_auth,
                "tls_enabled": state.tls_enabled,
            })),
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

    // #67: In strict mode (default), reject unregistered users and drawings.
    if !state.open_mode {
        match store.user_auth(&body.user_id) {
            Ok(Some(user)) if !user.active => {
                return error_response(StatusCode::FORBIDDEN, codes::USER_DISABLED, "账号已停用");
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    codes::UNKNOWN_USER,
                    "用户未登记",
                );
            }
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    codes::INVALID_MESSAGE,
                    &err,
                );
            }
        }
        match store.drawing(&body.drawing_id) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return error_response(
                    StatusCode::NOT_FOUND,
                    codes::UNKNOWN_DRAWING,
                    "图纸未登记",
                );
            }
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    codes::UNKNOWN_DRAWING,
                    &err,
                );
            }
        }
    } else if let Err(err) = store.ensure_drawing(&body.drawing_id) {
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
    #[serde(default)]
    credential: Option<String>,
}

/// Extracts and verifies the HTTP caller's identity for privileged mutations
/// (`POST /drawing/{id}/state`, `POST /conflicts/{id}/resolve`, #68).
///
/// Accepts credentials via:
/// - `Authorization: Bearer <user_id>:<credential>`
/// - `Authorization: Basic <base64(user_id:credential)>`
/// - `X-Collab-User` + `X-Collab-Credential` headers
/// - JSON body `credential` field alongside `user_id` / `resolved_by`
///
/// Returns the verified `user_id` so the operation actor always comes from the
/// authenticated identity. Never logs credentials.
#[allow(clippy::result_large_err)]
fn authenticate_http_actor(
    state: &AppState,
    store: &crate::store::Store,
    headers: &HeaderMap,
    body_user: Option<String>,
    body_credential: Option<String>,
    missing_user_msg: &str,
) -> Result<String, Response> {
    let mut header_user: Option<String> = None;
    let mut header_secret: Option<String> = None;

    if let Some(auth_val) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(token) = auth_val.strip_prefix("Bearer ").map(str::trim) {
            if let Some((u, s)) = token.split_once(':') {
                header_user = Some(u.to_owned());
                header_secret = Some(s.to_owned());
            } else if !token.is_empty() {
                header_secret = Some(token.to_owned());
            }
        } else if let Some(b64) = auth_val.strip_prefix("Basic ").map(str::trim) {
            if let Some(raw) = crate::auth::base64_decode(b64) {
                if let Ok(text) = String::from_utf8(raw) {
                    if let Some((u, s)) = text.split_once(':') {
                        header_user = Some(u.to_owned());
                        header_secret = Some(s.to_owned());
                    }
                }
            }
        }
    }
    if header_user.is_none() {
        header_user = headers
            .get("x-collab-user")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
    }
    if header_secret.is_none() {
        header_secret = headers
            .get("x-collab-credential")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
    }

    // If both header and body name a user and they disagree, reject as AUTH_FAILED.
    if let (Some(hu), Some(bu)) = (&header_user, &body_user) {
        if hu != bu {
            return Err(error_response(
                StatusCode::UNAUTHORIZED,
                codes::AUTH_FAILED,
                "身份核对失败：请求头身份与正文操作人不一致",
            ));
        }
    }

    let actor = match header_user.or(body_user) {
        Some(u) if !u.is_empty() => u,
        _ => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                codes::INVALID_MESSAGE,
                missing_user_msg,
            ))
        }
    };
    let secret = header_secret.or(body_credential);

    match store.user_auth(&actor) {
        Ok(Some(user)) => {
            if !user.active {
                return Err(error_response(
                    StatusCode::FORBIDDEN,
                    codes::USER_DISABLED,
                    "账号已停用",
                ));
            }
            if state.require_auth || !user.credential_hash.is_empty() {
                let ok = secret
                    .as_deref()
                    .is_some_and(|s| crate::auth::verify_credential(s, &user.credential_hash));
                if !ok {
                    return Err(error_response(
                        StatusCode::UNAUTHORIZED,
                        codes::AUTH_FAILED,
                        "身份核对失败",
                    ));
                }
            }
            Ok(user.user_id)
        }
        Ok(None) => {
            if state.require_auth {
                Err(error_response(
                    StatusCode::UNAUTHORIZED,
                    codes::AUTH_FAILED,
                    "身份核对失败：用户未登记",
                ))
            } else {
                // Let downstream perm::evaluate return its contract 400 PERM_UNKNOWN_SUBJECT.
                Ok(actor)
            }
        }
        Err(err) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            codes::INVALID_MESSAGE,
            &err,
        )),
    }
}

/// `POST /drawing/{id}/state` — gated on 授权配置. The requested state value
/// is validated before anything is written; an illegal value is a 400 and
/// nothing is persisted.
async fn drawing_state(
    State(state): State<Arc<AppState>>,
    Path(drawing_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DrawingStateBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::INVALID_MESSAGE,
            "数据库不可用",
        );
    };
    let user_id = match authenticate_http_actor(
        &state,
        &store,
        &headers,
        body.user_id,
        body.credential,
        "缺少 user_id",
    ) {
        Ok(uid) => uid,
        Err(resp) => return resp,
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

    // #26: Audit entry for drawing state switch (who, from -> to, object = drawing_id).
    {
        let actor_dn = state.display_name_of(&user_id);
        let actor_label = ocs_collab::protocol::format_user_label(&actor_dn, &user_id);
        let company = store
            .user_role(&user_id)
            .ok()
            .flatten()
            .map(|(_, company)| company)
            .filter(|company| !company.is_empty())
            .unwrap_or_else(|| "未知".to_owned());
        let audit_entry = crate::store::NewAuditRow {
            module: "协同编辑".to_owned(),
            action: "drawing_state_changed".to_owned(),
            at: ocs_collab::now_unix(),
            object_ref: drawing_id.clone(),
            user_id: user_id.clone(),
            company,
            ip: "127.0.0.1".to_owned(),
            workspace: "默认工作区".to_owned(),
            change_text: format!(
                "图纸状态切换：{actor_label} 将 {drawing_id} 从 {current_text} 切换为 {state_text}"
            ),
        };
        if let Err(err) = store.audit_append(&audit_entry) {
            eprintln!("[ocs_collab_server] 写入图纸状态切换审计失败：{err}");
        }
    }

    let prev_state = DrawingState::parse(&current_text);
    settle_state_change(&state, &drawing_id, prev_state, next_state, &user_id).await;

    (
        StatusCode::OK,
        Json(json!({ "drawing_id": drawing_id, "state": state_text })),
    )
        .into_response()
}

/// Settles session write permissions whenever a drawing changes state (#70, #71, #73):
/// - Switching to `只读` clears any holder (`#73`) and sends `ReadonlyDowngrade { holder: None }`
///   to every active session on the drawing (nobody can write).
/// - Switching to `单人编辑` keeps only the earliest active editor writable and
///   downgrades all other active sessions naming that holder; if leaving `只读`,
///   notifies the writable editor with `WriteRestored` (`#71`).
/// - Switching to `协同编辑` notifies every active session that now has write
///   access with `WriteRestored` (`#71`) so clients recover without reconnecting.
async fn settle_state_change(
    state: &AppState,
    drawing_id: &str,
    prev_state: Option<DrawingState>,
    next_state: DrawingState,
    changed_by: &str,
) {
    // #73: Read-only drawings never retain an edit holder.
    state
        .holders
        .lock()
        .expect("holders mutex poisoned")
        .remove(drawing_id);

    let active_sessions: Vec<ocs_collab::Session> = {
        let registry = state.sessions.lock().await;
        registry.active_peers(drawing_id)
    };

    match next_state {
        DrawingState::ReadOnly => {
            let message = crate::access::Denied {
                level: ocs_collab::DenialLevel::State,
                reason: "图纸已切换为只读".to_owned(),
                holder: None,
                effective_node: None,
            }
            .downgrade();
            println!(
                "[ocs_collab_server] 图纸切只读 drawing={drawing_id} by={changed_by} holder=(无) -> 通知 {} 个会话 at={}",
                active_sessions.len(),
                chrono::Utc::now().timestamp()
            );
            let direct = state.direct.lock().expect("direct mutex poisoned");
            for session in &active_sessions {
                let delivered = direct
                    .get(&session.session_id)
                    .is_some_and(|sender| sender.send(message.clone()).is_ok());
                println!(
                    "[ocs_collab_server] 单播 readonly_downgrade session={} user={} holder=(无) delivered={delivered}",
                    session.session_id, session.user_id
                );
            }
        }
        DrawingState::SingleEditor => {
            for session in &active_sessions {
                let msg = match crate::access::write_access(state, session).await {
                    Some(denied) => Some(denied.downgrade()),
                    None if prev_state == Some(DrawingState::ReadOnly) => {
                        Some(ocs_collab::ServerMessage::WriteRestored {
                            reason: "图纸已切换为单人编辑，您已获得编辑权，写权限已恢复"
                                .to_owned(),
                        })
                    }
                    None => None,
                };
                if let Some(msg) = msg {
                    let tx = state
                        .direct
                        .lock()
                        .expect("direct mutex poisoned")
                        .get(&session.session_id)
                        .cloned();
                    if let Some(tx) = tx {
                        let _ = tx.send(msg);
                    }
                }
            }
        }
        DrawingState::Collaborative => {
            if prev_state == Some(DrawingState::Collaborative) {
                return;
            }
            for session in &active_sessions {
                if crate::access::write_access(state, session).await.is_none() {
                    let tx = state
                        .direct
                        .lock()
                        .expect("direct mutex poisoned")
                        .get(&session.session_id)
                        .cloned();
                    if let Some(tx) = tx {
                        let _ = tx.send(ocs_collab::ServerMessage::WriteRestored {
                            reason: "图纸已切换为协同编辑，写权限已恢复".to_owned(),
                        });
                    }
                }
            }
        }
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
    let (_, unpaired) = crate::ops::unpaired_connectors(&state, &drawing_id);
    topology["drawing_id"] = json!(drawing_id);
    topology["revision"] = json!(revision);
    topology["unpaired_connectors"] = json!(unpaired);
    (StatusCode::OK, Json(topology)).into_response()
}

/// `GET /drawing/{id}/unpaired-connectors` → 200
/// `{"drawing_id","revision","items":[…],"total":N}` (#32).
async fn drawing_unpaired_connectors(
    State(state): State<Arc<AppState>>,
    Path(drawing_id): Path<String>,
) -> Response {
    let (revision, items) = crate::ops::unpaired_connectors(&state, &drawing_id);
    let total = items.len();
    (
        StatusCode::OK,
        Json(json!({
            "drawing_id": drawing_id,
            "revision": revision,
            "items": items,
            "total": total,
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct ConflictParams {
    drawing_id: Option<String>,
    status: Option<String>,
}

/// `GET /users` → the registered users (#75).
async fn user_list(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    match store.list_users() {
        Ok(rows) => {
            let items: Vec<_> = rows
                .into_iter()
                .map(|(user_id, display_name, role, active)| {
                    json!({
                        "user_id": user_id,
                        "display_name": display_name,
                        "role": role,
                        "active": active,
                    })
                })
                .collect();
            let total = items.len();
            (StatusCode::OK, Json(json!({ "items": items, "total": total }))).into_response()
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

/// `GET /roles` → the five roles and the 5×8 matrix (#75).
async fn role_list() -> Response {
    let items: Vec<_> = ocs_collab::Role::ALL
        .into_iter()
        .map(|role| {
            let operations: Vec<_> = ocs_collab::Operation::ALL
                .into_iter()
                .map(|operation| {
                    json!({
                        "operation": operation.as_str(),
                        "allowed": ocs_collab::permission::role_allows(role, operation),
                    })
                })
                .collect();
            json!({ "role": role.as_str(), "operations": operations })
        })
        .collect();
    (StatusCode::OK, Json(json!({ "items": items, "total": items.len() }))).into_response()
}

/// `GET /pbs/nodes`.
async fn pbs_node_list(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    match store.list_pbs_node_details() {
        Ok(rows) => {
            let items: Vec<_> = rows
                .into_iter()
                .map(|(node_id, name, node_type, parent_id)| {
                    json!({
                        "node_id": node_id,
                        "name": name,
                        "node_type": node_type,
                        "parent_id": parent_id,
                    })
                })
                .collect();
            let total = items.len();
            (StatusCode::OK, Json(json!({ "items": items, "total": total }))).into_response()
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

/// `GET /pbs/acl`.
async fn pbs_acl_list(State(state): State<Arc<AppState>>) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    match store.list_pbs_acl() {
        Ok(rows) => {
            let items: Vec<_> = rows
                .into_iter()
                .map(|(node_id, subject, permission, allowed)| {
                    json!({
                        "node_id": node_id,
                        "subject": subject,
                        "permission": permission,
                        "allowed": allowed,
                    })
                })
                .collect();
            let total = items.len();
            (StatusCode::OK, Json(json!({ "items": items, "total": total }))).into_response()
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

#[derive(Debug, Deserialize)]
struct AclWriteBody {
    user_id: Option<String>,
    node_id: Option<String>,
    subject: Option<String>,
    permission: Option<String>,
    allowed: Option<bool>,
    #[serde(default)]
    credential: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoleWriteBody {
    user_id: Option<String>,
    role: Option<String>,
    node_id: Option<String>,
    #[serde(default)]
    credential: Option<String>,
}

fn acl_permission_known(text: &str) -> bool {
    matches!(text, "查看" | "编辑" | "导出" | "删除")
}

/// 授权配置 on `node_id`, skipping the drawing-state tier (#75).
fn require_configure(store: &crate::store::Store, actor: &str, node_id: &str) -> Result<(), Response> {
    let role_text = match store.user_role(actor) {
        Ok(Some((role, _))) => role,
        Ok(None) => {
            return Err(error_response(
                StatusCode::BAD_REQUEST,
                codes::PERM_UNKNOWN_SUBJECT,
                "用户不存在",
            ))
        }
        Err(err) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                codes::INVALID_MESSAGE,
                &err,
            ))
        }
    };
    let Some(role) = ocs_collab::Role::parse(&role_text) else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            codes::PERM_UNKNOWN_SUBJECT,
            "用户角色非法",
        ));
    };
    let nodes = match perm::load_acl_tree(store) {
        Ok(nodes) => nodes,
        Err(err) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                codes::INVALID_MESSAGE,
                &err,
            ))
        }
    };
    if !nodes.iter().any(|node| node.node_id == node_id) {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            codes::INVALID_MESSAGE,
            "节点不存在",
        ));
    }
    let decision = ocs_collab::decide_without_state(&ocs_collab::PermissionRequest {
        user_id: actor,
        role,
        operation: Operation::ConfigureAuthorization,
        drawing_state: DrawingState::Collaborative,
        node_id,
        nodes: &nodes,
    });
    if decision.allowed {
        Ok(())
    } else {
        let outcome = perm::EvalOutcome {
            decision,
            status: StatusCode::FORBIDDEN,
            holder: None,
        };
        Err((outcome.status, Json(decision_json(&outcome))).into_response())
    }
}

fn audit_admin_change(
    store: &crate::store::Store,
    actor: &str,
    action: &str,
    object_ref: &str,
    change_text: String,
) {
    let company = store
        .user_role(actor)
        .ok()
        .flatten()
        .map(|(_, company)| company)
        .filter(|company| !company.is_empty())
        .unwrap_or_else(|| "未知".to_owned());
    let row = crate::store::NewAuditRow {
        module: "协同编辑".to_owned(),
        action: action.to_owned(),
        at: ocs_collab::now_unix(),
        object_ref: object_ref.to_owned(),
        user_id: actor.to_owned(),
        company,
        ip: "127.0.0.1".to_owned(),
        workspace: "默认工作区".to_owned(),
        change_text,
    };
    if let Err(err) = store.audit_append(&row) {
        eprintln!("[ocs_collab_server] 写入授权变更审计失败：{err}");
    }
}

async fn writable_snapshot(state: &AppState) -> Vec<(ocs_collab::Session, bool)> {
    let sessions: Vec<ocs_collab::Session> = {
        let registry = state.sessions.lock().await;
        registry
            .all()
            .iter()
            .filter(|session| session.is_active())
            .cloned()
            .collect()
    };
    let mut rows = Vec::with_capacity(sessions.len());
    for session in sessions {
        let writable = crate::access::write_access(state, &session).await.is_none();
        rows.push((session, writable));
    }
    rows
}

async fn notify_write_changes(
    state: &AppState,
    before: Vec<(ocs_collab::Session, bool)>,
    restored_reason: &str,
) -> usize {
    let mut pending = Vec::new();
    for (session, was_writable) in before {
        let denied = crate::access::write_access(state, &session).await;
        let now_writable = denied.is_none();
        if was_writable == now_writable {
            continue;
        }
        let message = match denied {
            Some(denied) => denied.downgrade(),
            None => ocs_collab::ServerMessage::WriteRestored {
                reason: restored_reason.to_owned(),
            },
        };
        pending.push((session.session_id, message));
    }
    let direct = state.direct.lock().expect("direct mutex poisoned");
    let mut notified = 0usize;
    for (session_id, message) in pending {
        if direct
            .get(&session_id)
            .is_some_and(|sender| sender.send(message).is_ok())
        {
            notified += 1;
        }
    }
    notified
}

fn root_node_id(store: &crate::store::Store) -> Result<Option<String>, String> {
    Ok(store
        .list_pbs_nodes()?
        .into_iter()
        .find(|(_, parent)| parent.is_none())
        .map(|(node_id, _)| node_id))
}

/// `PUT /pbs/acl` — add or replace one row. Online sessions recompute (#75).
async fn pbs_acl_put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AclWriteBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    let actor = match authenticate_http_actor(
        &state,
        &store,
        &headers,
        body.user_id,
        body.credential,
        "缺少 user_id",
    ) {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let (Some(node_id), Some(subject), Some(permission), Some(allowed)) =
        (body.node_id, body.subject, body.permission, body.allowed)
    else {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "缺少必填字段");
    };
    if subject.trim().is_empty() || !acl_permission_known(&permission) {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_MESSAGE,
            "subject 或 permission 非法",
        );
    }
    if let Err(response) = require_configure(&store, &actor, &node_id) {
        return response;
    }
    let before = writable_snapshot(&state).await;
    if let Err(err) = store.upsert_pbs_acl(&node_id, &subject, &permission, allowed) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err);
    }
    let verb = if allowed { "允许" } else { "拒绝" };
    audit_admin_change(
        &store,
        &actor,
        "acl_upsert",
        &node_id,
        format!("{actor} 将节点 {node_id} 上 {subject} 的{permission}设为{verb}"),
    );
    let notified = notify_write_changes(
        &state,
        before,
        &format!("节点 {node_id} 的授权行已变更"),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "node_id": node_id,
            "subject": subject,
            "permission": permission,
            "allowed": allowed,
            "notified": notified,
        })),
    )
        .into_response()
}

/// `DELETE /pbs/acl` — remove one row. Online sessions recompute (#75).
async fn pbs_acl_delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AclWriteBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    let actor = match authenticate_http_actor(
        &state,
        &store,
        &headers,
        body.user_id,
        body.credential,
        "缺少 user_id",
    ) {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let (Some(node_id), Some(subject), Some(permission)) =
        (body.node_id, body.subject, body.permission)
    else {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "缺少必填字段");
    };
    if let Err(response) = require_configure(&store, &actor, &node_id) {
        return response;
    }
    let before = writable_snapshot(&state).await;
    let removed = match store.delete_pbs_acl(&node_id, &subject, &permission) {
        Ok(removed) => removed,
        Err(err) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err)
        }
    };
    if !removed {
        return error_response(StatusCode::NOT_FOUND, codes::INVALID_MESSAGE, "授权行不存在");
    }
    audit_admin_change(
        &store,
        &actor,
        "acl_delete",
        &node_id,
        format!("{actor} 删除节点 {node_id} 上 {subject} 的{permission}授权行"),
    );
    let notified = notify_write_changes(
        &state,
        before,
        &format!("节点 {node_id} 的授权行已删除"),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "node_id": node_id,
            "subject": subject,
            "permission": permission,
            "notified": notified,
        })),
    )
        .into_response()
}

/// `POST /users/{id}/role` — change a user's role. Online sessions recompute (#75).
async fn user_role_set(
    State(state): State<Arc<AppState>>,
    Path(target): Path<String>,
    headers: HeaderMap,
    Json(body): Json<RoleWriteBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    let actor = match authenticate_http_actor(
        &state,
        &store,
        &headers,
        body.user_id,
        body.credential,
        "缺少 user_id",
    ) {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(role) = body.role else {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "缺少 role");
    };
    if ocs_collab::Role::parse(&role).is_none() {
        return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "role 非法");
    }
    let node_id = match body.node_id {
        Some(node_id) => node_id,
        None => match root_node_id(&store) {
            Ok(Some(node_id)) => node_id,
            Ok(None) => {
                return error_response(StatusCode::BAD_REQUEST, codes::INVALID_MESSAGE, "没有 PBS 节点可用来判定授权配置")
            }
            Err(err) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err)
            }
        },
    };
    if let Err(response) = require_configure(&store, &actor, &node_id) {
        return response;
    }
    let before = writable_snapshot(&state).await;
    let updated = match store.set_user_role(&target, &role) {
        Ok(updated) => updated,
        Err(err) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err)
        }
    };
    if !updated {
        return error_response(StatusCode::NOT_FOUND, codes::PERM_UNKNOWN_SUBJECT, "用户不存在");
    }
    audit_admin_change(
        &store,
        &actor,
        "role_changed",
        &target,
        format!("{actor} 将 {target} 的角色改为 {role}"),
    );
    let notified = notify_write_changes(
        &state,
        before,
        &format!("{target} 的角色已改为 {role}"),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "user_id": target,
            "role": role,
            "notified": notified,
        })),
    )
        .into_response()
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
    #[serde(default)]
    credential: Option<String>,
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
    headers: HeaderMap,
    Json(body): Json<ResolveBody>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, codes::INVALID_MESSAGE, "数据库不可用");
    };
    let resolved_by = match authenticate_http_actor(
        &state,
        &store,
        &headers,
        body.resolved_by.or(body.user_id),
        body.credential,
        "缺少 resolved_by",
    ) {
        Ok(uid) => uid,
        Err(resp) => return resp,
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
    // revision as it does for any op — including downstream drawings updated
    // via off-page connectors (#34).
    {
        let mut sessions = state.sessions.lock().await;
        sessions.advance_base_revision(&resolved.row.drawing_id, resolved.revision);
        for (downstream_id, downstream_rev) in &resolved.downstream_revisions {
            sessions.advance_base_revision(downstream_id, *downstream_rev);
        }
    }
    (
        StatusCode::OK,
        Json(json!({
            "conflict_id": resolved.row.conflict_id,
            "status": resolved.row.status,
            "resolution": resolved.row.resolution,
            "resolved_by": resolved.row.resolved_by,
            "resolved_by_display_name": resolved.row.resolved_by_display_name,
            "resolved_value": resolved.row.resolved_value,
            "object_ref": resolved.row.object_ref,
            "field": resolved.row.field,
            "value": resolved.final_value,
            "revision": resolved.revision,
        })),
    )
        .into_response()
}

/// `GET /metrics` → 200 read-only runtime observability snapshot (#39):
/// per-drawing & total active editing sessions, claims, ops/min, denied writes,
/// pending conflicts, op-log rows, and database size on disk.
async fn metrics_snapshot(State(state): State<Arc<AppState>>) -> Response {
    let now = ocs_collab::now_unix();
    let mut drawing_ids = std::collections::BTreeSet::<String>::new();

    // Active sessions per drawing.
    let mut sessions_by_drawing: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    {
        let registry = state.sessions.lock().await;
        for s in registry.all() {
            if s.is_active() {
                drawing_ids.insert(s.drawing_id.clone());
                *sessions_by_drawing.entry(s.drawing_id.clone()).or_insert(0) += 1;
            }
        }
    }

    // Active claims per drawing.
    let mut claims_by_drawing: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    {
        let mut occ = state.occupancy.lock().await;
        for c in occ.entries(now) {
            drawing_ids.insert(c.drawing_id.clone());
            *claims_by_drawing.entry(c.drawing_id.clone()).or_insert(0) += 1;
        }
    }

    let ops_per_min = state.metrics.ops_per_minute_by_drawing(now);
    let denied_by_drawing = state.metrics.denied_writes_by_drawing();
    for did in ops_per_min.keys().chain(denied_by_drawing.keys()) {
        drawing_ids.insert(did.clone());
    }

    let (ops_rows_by_drawing, conflicts_by_drawing, db_size_bytes) = match &state.store {
        Some(store) => {
            if let Ok(ids) = store.drawing_ids() {
                for id in ids {
                    drawing_ids.insert(id);
                }
            }
            (
                store.ops_count_by_drawing().unwrap_or_default(),
                store.pending_conflicts_by_drawing().unwrap_or_default(),
                store.db_size_bytes(),
            )
        }
        None => (Default::default(), Default::default(), 0),
    };

    let mut total_sessions = 0u64;
    let mut total_claims = 0u64;
    let mut total_ops_per_min = 0u64;
    let mut total_pending_conflicts = 0i64;
    let mut total_ops_rows = 0i64;

    let drawings: Vec<serde_json::Value> = drawing_ids
        .into_iter()
        .map(|did| {
            let active_sessions = sessions_by_drawing.get(&did).copied().unwrap_or(0);
            let claims = claims_by_drawing.get(&did).copied().unwrap_or(0);
            let opm = ops_per_min.get(&did).copied().unwrap_or(0);
            let denied = denied_by_drawing.get(&did).copied().unwrap_or(0);
            let pending_conflicts = conflicts_by_drawing.get(&did).copied().unwrap_or(0);
            let ops_log_rows = ops_rows_by_drawing.get(&did).copied().unwrap_or(0);
            total_sessions += active_sessions;
            total_claims += claims;
            total_ops_per_min += opm;
            total_pending_conflicts += pending_conflicts;
            total_ops_rows += ops_log_rows;
            json!({
                "drawing_id": did,
                "active_sessions": active_sessions,
                "claims": claims,
                "ops_per_minute": opm,
                "denied_writes": denied,
                "pending_conflicts": pending_conflicts,
                "ops_log_rows": ops_log_rows,
            })
        })
        .collect();

    let uptime_seconds = (now - state.metrics.started_at()).max(0);
    (
        StatusCode::OK,
        Json(json!({
            "open_mode": state.open_mode,
            "auth_enabled": state.require_auth,
            "tls_enabled": state.tls_enabled,
            "uptime_seconds": uptime_seconds,
            "totals": {
                "active_sessions": total_sessions,
                "claims": total_claims,
                "applied_ops_total": state.metrics.applied_ops_total(),
                "ops_per_minute": total_ops_per_min,
                "denied_writes_total": state.metrics.denied_writes_total(),
                "pending_conflicts": total_pending_conflicts,
                "ops_log_rows": total_ops_rows,
                "db_size_bytes": db_size_bytes,
            },
            "drawings": drawings,
        })),
    )
        .into_response()
}

#[derive(Debug, Default, Deserialize)]
struct BackupBody {
    user_id: Option<String>,
    credential: Option<String>,
    dest_dir: Option<String>,
}

/// `POST /admin/backup` → 200 `BackupInfo` (#105): triggers an immediate
/// online SQLite snapshot into `dest_dir` or the configured `--backup-dir`.
async fn admin_backup(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<BackupBody>>,
) -> Response {
    let Some(store) = state.store.clone() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            codes::INVALID_MESSAGE,
            "数据库不可用",
        );
    };
    let body = body.map(|Json(b)| b).unwrap_or_default();
    if state.require_auth || body.user_id.is_some() || headers.contains_key(axum::http::header::AUTHORIZATION) {
        let actor = match authenticate_http_actor(
            &state,
            &store,
            &headers,
            body.user_id,
            body.credential,
            "缺少 user_id",
        ) {
            Ok(uid) => uid,
            Err(resp) => return resp,
        };
        if let Ok(Some((role, _))) = store.user_role(&actor) {
            if !matches!(
                ocs_collab::Role::parse(&role),
                Some(ocs_collab::Role::SystemAdmin | ocs_collab::Role::ProjectAdmin)
            ) {
                return error_response(
                    StatusCode::FORBIDDEN,
                    codes::PERM_DENIED_ROLE,
                    "只有系统管理员或项目管理员可以触发在线备份",
                );
            }
        }
    }
    let dir = body
        .dest_dir
        .map(std::path::PathBuf::from)
        .or_else(|| state.backup_dir.clone());
    let Some(dir) = dir else {
        return error_response(
            StatusCode::BAD_REQUEST,
            codes::INVALID_MESSAGE,
            "未配置 --backup-dir 且请求未提供 dest_dir",
        );
    };
    static BACKUP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = BACKUP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match store.backup_into_dir(&dir, seq) {
        Ok(info) => {
            state.logger.emit(
                "info",
                "backup_completed",
                json!({
                    "path": info.path,
                    "size_bytes": info.size_bytes,
                    "total_ops": info.total_ops,
                    "max_op_at": info.max_op_at,
                }),
            );
            (StatusCode::OK, Json(info)).into_response()
        }
        Err(err) => error_response(StatusCode::INTERNAL_SERVER_ERROR, codes::INVALID_MESSAGE, &err),
    }
}

async fn readonly_endpoint() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({
            "code": "READONLY_ENDPOINT",
            "message": "该接口为只读接口",
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
        Arc::new(
            AppState::new(
                Some(store),
                None,
                ":memory:",
                SessionConfig::default(),
            )
            .with_open_mode(true),
        )
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

    /// #74: on one node, opposite user and role rows deny the write, and the
    /// probe names the denying row. Both rows allowing lets the op through.
    #[tokio::test]
    async fn opposite_user_and_role_rows_on_one_node_block_the_write() {
        let state = seeded_state();
        let store = state.store.clone().expect("store");
        let now = ocs_collab::now_unix();
        let (mut conn, _) = live_conn(&state, "u-proc", now).await;

        let user_deny = serde_json::from_value(json!({
            "pbs_acl": [
                {"node_id": "n-cell-a1", "subject": "u-proc", "permission": "编辑", "allowed": false},
                {"node_id": "n-cell-a1", "subject": "工艺设计人员", "permission": "编辑", "allowed": true}
            ]
        }))
        .expect("acl json");
        store.seed(&user_deny).expect("seed user deny");

        let denied = submit_property(&state, &mut conn, "op-user-deny", "1.0MPa", now + 1).await;
        let (code, level) = refusal_in(&denied).expect("user deny must refuse the op");
        assert_eq!(code, codes::PERM_DENIED_PBS);
        assert_eq!(level.as_deref(), Some("pbs"));
        let revision = store.drawing("d-a1-coop").expect("drawing").expect("row").2;
        assert_eq!(revision, 0, "a refused write does not advance the drawing");

        let probe = permission_check(
            State(state.clone()),
            Json(PermissionCheckBody {
                user_id: Some("u-proc".into()),
                drawing_id: Some("d-a1-coop".into()),
                operation: Some("编辑图纸".into()),
            }),
        )
        .await;
        assert_eq!(probe.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(probe.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(body["deciding_row"]["subject"], "u-proc");
        assert_eq!(body["deciding_row"]["rule"], "拒绝为准");
        assert_eq!(body["deciding_row"]["subject_kind"], "user");

        let role_deny = serde_json::from_value(json!({
            "pbs_acl": [
                {"node_id": "n-cell-a1", "subject": "u-proc", "permission": "编辑", "allowed": true},
                {"node_id": "n-cell-a1", "subject": "工艺设计人员", "permission": "编辑", "allowed": false}
            ]
        }))
        .expect("acl json");
        store.seed(&role_deny).expect("seed role deny");
        let denied = submit_property(&state, &mut conn, "op-role-deny", "1.6MPa", now + 2).await;
        let (code, _) = refusal_in(&denied).expect("role deny must refuse the op");
        assert_eq!(code, codes::PERM_DENIED_PBS);
        let probe = permission_check(
            State(state.clone()),
            Json(PermissionCheckBody {
                user_id: Some("u-proc".into()),
                drawing_id: Some("d-a1-coop".into()),
                operation: Some("编辑图纸".into()),
            }),
        )
        .await;
        let body = axum::body::to_bytes(probe.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(body["deciding_row"]["subject"], "工艺设计人员");
        assert_eq!(body["deciding_row"]["subject_kind"], "role");
        assert_eq!(body["allowed"], false);

        let both_allow = serde_json::from_value(json!({
            "pbs_acl": [
                {"node_id": "n-cell-a1", "subject": "u-proc", "permission": "编辑", "allowed": true},
                {"node_id": "n-cell-a1", "subject": "工艺设计人员", "permission": "编辑", "allowed": true}
            ]
        }))
        .expect("acl json");
        store.seed(&both_allow).expect("seed both allow");
        let applied = submit_property(&state, &mut conn, "op-both-allow", "2.5MPa", now + 3).await;
        assert!(
            applied_in(&applied),
            "both rows allowing must let the write through: {applied:?}"
        );
        let revision = store.drawing("d-a1-coop").expect("drawing").expect("row").2;
        assert_eq!(revision, 1);
    }

    /// #75: an admin adds a deny row, the designer becomes read-only without
    /// reconnecting; deleting the row restores the write. A role change does
    /// the same, and each change is one audit row.
    #[tokio::test]
    async fn acl_and_role_changes_take_effect_on_the_live_session() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (session_id, mut rx) = live_session(&state, "u-proc", "协同编辑", now).await;

        let refused = pbs_acl_put(
            State(state.clone()),
            HeaderMap::new(),
            Json(AclWriteBody {
                user_id: Some("u-proc".into()),
                node_id: Some("n-cell-a1".into()),
                subject: Some("u-proc".into()),
                permission: Some("编辑".into()),
                allowed: Some(false),
                credential: None,
            }),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        let put = pbs_acl_put(
            State(state.clone()),
            HeaderMap::new(),
            Json(AclWriteBody {
                user_id: Some("u-admin".into()),
                node_id: Some("n-cell-a1".into()),
                subject: Some("u-proc".into()),
                permission: Some("编辑".into()),
                allowed: Some(false),
                credential: None,
            }),
        )
        .await;
        assert_eq!(put.status(), StatusCode::OK);
        match rx.try_recv().expect("designer is told at once") {
            ocs_collab::ServerMessage::ReadonlyDowngrade { level, code, .. } => {
                assert_eq!(level.as_deref(), Some("pbs"));
                assert_eq!(code.as_deref(), Some(codes::PERM_DENIED_PBS));
            }
            other => panic!("expected a downgrade, got {other:?}"),
        }

        let session = {
            let registry = state.sessions.lock().await;
            registry.get(&session_id).cloned().expect("session")
        };
        let mut conn = crate::ws::Connection::new("127.0.0.1");
        conn.session = Some(session);
        let denied = submit_property(&state, &mut conn, "op-after-deny", "1.0MPa", now + 1).await;
        let (code, _) = refusal_in(&denied).expect("the live write is refused");
        assert_eq!(code, codes::PERM_DENIED_PBS);

        let listed = pbs_acl_list(State(state.clone())).await;
        assert_eq!(listed.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(listed.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert!(body["items"].as_array().unwrap().iter().any(|row| {
            row["node_id"] == "n-cell-a1"
                && row["subject"] == "u-proc"
                && row["permission"] == "编辑"
                && row["allowed"] == false
        }));

        let deleted = pbs_acl_delete(
            State(state.clone()),
            HeaderMap::new(),
            Json(AclWriteBody {
                user_id: Some("u-admin".into()),
                node_id: Some("n-cell-a1".into()),
                subject: Some("u-proc".into()),
                permission: Some("编辑".into()),
                allowed: None,
                credential: None,
            }),
        )
        .await;
        assert_eq!(deleted.status(), StatusCode::OK);
        match rx.try_recv().expect("write comes back") {
            ocs_collab::ServerMessage::WriteRestored { .. } => {}
            other => panic!("expected write restored, got {other:?}"),
        }
        let applied = submit_property(&state, &mut conn, "op-after-delete", "1.6MPa", now + 2).await;
        assert!(applied_in(&applied), "deleting the row restores the write: {applied:?}");

        let role = user_role_set(
            State(state.clone()),
            Path("u-proc".into()),
            HeaderMap::new(),
            Json(RoleWriteBody {
                user_id: Some("u-admin".into()),
                role: Some("校对人员".into()),
                node_id: Some("n-plant".into()),
                credential: None,
            }),
        )
        .await;
        assert_eq!(role.status(), StatusCode::OK);
        match rx.try_recv().expect("role change downgrades") {
            ocs_collab::ServerMessage::ReadonlyDowngrade { level, .. } => {
                assert_eq!(level.as_deref(), Some("role"));
            }
            other => panic!("expected a role downgrade, got {other:?}"),
        }
        let users = user_list(State(state.clone())).await;
        let bytes = axum::body::to_bytes(users.into_body(), 64 * 1024)
            .await
            .expect("body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert!(body["items"].as_array().unwrap().iter().any(|row| {
            row["user_id"] == "u-proc" && row["role"] == "校对人员"
        }));
        let roles = role_list().await;
        assert_eq!(roles.status(), StatusCode::OK);
        let nodes = pbs_node_list(State(state.clone())).await;
        assert_eq!(nodes.status(), StatusCode::OK);

        let store = state.store.clone().expect("store");
        let page = store
            .audit_query(&AuditQuery {
                user_id: Some("u-admin".into()),
                ..AuditQuery::default()
            })
            .expect("audit");
        let actions: Vec<_> = page.items.iter().map(|row| row.action.as_str()).collect();
        assert!(actions.contains(&"acl_upsert"), "{actions:?}");
        assert!(actions.contains(&"acl_delete"), "{actions:?}");
        assert!(actions.contains(&"role_changed"), "{actions:?}");
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
                deciding_row: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("只读".into()),
                credential: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("四态".into()),
                credential: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: None,
                state: Some("只读".into()),
                credential: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-view".into()),
                state: Some("只读".into()),
                credential: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("只读".into()),
                credential: None,
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
                protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
                resume_session_id: None,
                credential: None,
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
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some(next.into()),
                credential: None,
            }),
        )
        .await
        .status()
    }

    /// #73: Switching a drawing to 只读 downgrades EVERY live session with
    /// `holder = None` (read-only drawings no longer retain an edit holder).
    #[tokio::test]
    async fn switching_to_readonly_downgrades_everyone_with_no_holder() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (_viewer, mut viewer_rx) = live_session(&state, "u-view", "只读", now - 30).await;
        let (_first, mut first_rx) = live_session(&state, "u-proc", "协同编辑", now - 20).await;
        let (_second, mut second_rx) =
            live_session(&state, "u-admin", "协同编辑", now - 10).await;
        let mut room = state.bus.subscribe("d-a1-coop");

        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        for (who, receiver) in [
            ("the second editor", &mut second_rx),
            ("the session that was read-only already", &mut viewer_rx),
            ("the first editor", &mut first_rx),
        ] {
            match receiver.try_recv() {
                Ok(ocs_collab::ServerMessage::ReadonlyDowngrade {
                    reason,
                    holder: named,
                    level,
                    code,
                    ..
                }) => {
                    assert!(!reason.is_empty(), "{who}: the reason is stated");
                    assert_eq!(named, None, "{who}: readonly has no holder (#73)");
                    assert_eq!(level.as_deref(), Some("state"), "{who}");
                    assert_eq!(code.as_deref(), Some(codes::PERM_DENIED_STATE), "{who}");
                }
                other => panic!("{who} must be told, got {other:?}"),
            }
            assert!(receiver.try_recv().is_err(), "{who} is told once");
        }
        assert_eq!(
            crate::access::holder_of(&state, "d-a1-coop"),
            None,
            "readonly drawing retains no edit holder (#73)"
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
                protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
                resume_session_id: None,
                credential: None,
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

    async fn submit_property(
        state: &AppState,
        conn: &mut crate::ws::Connection,
        op_id: &str,
        value: &str,
        now: i64,
    ) -> Vec<crate::ws::Reaction> {
        let session_id = conn.session.as_ref().expect("session").session_id.clone();
        let op = off_the_wire(json!({
            "type": "op",
            "session_id": session_id,
            "base_revision": 0,
            "op_id": op_id,
            "payload": {
                "kind": "set_property",
                "object_ref": "V-7401",
                "field": "设计压力",
                "value": value,
            },
        }));
        crate::ws::handle_message(state, conn, op, now).await
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
                crate::ws::Reaction::Publish {
                    message: ocs_collab::ServerMessage::ClaimGranted { .. },
                    ..
                }
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
            protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
            resume_session_id: None,
            credential: None,
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
    /// #73: On a 只读 drawing NOBODY may write (including the editor who was
    /// active at the moment of the switch), and `holder` is always `None`.
    #[tokio::test]
    async fn on_a_readonly_drawing_nobody_writes_and_no_holder_is_retained() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut first, _) = live_conn(&state, "u-proc", now - 20).await;
        let (mut second, _) = live_conn(&state, "u-admin", now - 10).await;
        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        for (who, conn) in [("first", &mut first), ("second", &mut second)] {
            let refused = write(&state, conn, &format!("op-{who}"), "1.6MPa").await;
            assert_eq!(
                refusal_in(&refused),
                Some((codes::PERM_DENIED_STATE.to_owned(), Some("state".to_owned()))),
                "{who}: {refused:?}"
            );
        }
        assert_eq!(denied_rows(&state, "u-proc").len(), 1);
        assert_eq!(denied_rows(&state, "u-admin").len(), 1);

        // A session joining afterwards gets ReadonlyDowngrade with holder = None.
        let (mut late, reactions) = live_conn(&state, "u-admin", now).await;
        assert_eq!(
            downgrade_in(&reactions),
            Some((
                None,
                Some("state".to_owned()),
                Some(codes::PERM_DENIED_STATE.to_owned())
            )),
            "{reactions:?}"
        );
        assert!(refusal_in(&write(&state, &mut late, "op-l", "4.0MPa").await).is_some());
    }

    /// #73: On a 单人编辑 drawing, the edit holder is the earliest active
    /// editing session with a socket (skipping HTTP-only sessions).
    #[tokio::test]
    async fn single_editor_holder_is_first_editor_with_a_socket() {
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
        let (mut first, _) = live_conn(&state, "u-proc", now).await;
        let (mut second, _) = live_conn(&state, "u-admin", now).await;
        let first_id = first.session.as_ref().expect("session").session_id.clone();

        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);
        assert_eq!(
            crate::access::first_editor(&state, "d-a1-coop").await.as_deref(),
            Some(first_id.as_str())
        );
        assert!(applied_in(&write(&state, &mut first, "op-s1", "1.6MPa").await));
        assert!(refusal_in(&write(&state, &mut second, "op-s2", "2.5MPa").await).is_some());
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

    /// #73: Switching to 单人编辑 leaves the earliest editor writable and
    /// downgrades the second editor naming the first as holder.
    #[tokio::test]
    async fn switching_to_single_editor_downgrades_second_editor_naming_first_holder() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (first, mut first_rx) = live_session(&state, "u-proc", "协同编辑", now - 20).await;
        let (_second, mut second_rx) =
            live_session(&state, "u-admin", "协同编辑", now - 10).await;

        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);

        assert!(first_rx.try_recv().is_err(), "first editor remains writable");
        match second_rx.try_recv() {
            Ok(ocs_collab::ServerMessage::ReadonlyDowngrade { holder, code, .. }) => {
                assert_eq!(holder.as_deref(), Some(first.as_str()));
                assert_eq!(code.as_deref(), Some(codes::PERM_DENIED_STATE));
            }
            other => panic!("second editor must be downgraded naming first holder: {other:?}"),
        }
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
                protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
                resume_session_id: None,
                credential: None,
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
            HeaderMap::new(),
            Json(ResolveBody {
                resolution: Some(resolution.into()),
                resolved_by: Some(by.into()),
                user_id: None,
                value: value.map(str::to_owned),
                resolved_value: None,
                credential: None,
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
            protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
            resume_session_id: None,
            credential: None,
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

    /// #25: ClaimGranted is broadcast to all editing sessions on the drawing with `expires_at`.
    #[tokio::test]
    async fn claim_granted_broadcasts_to_all_sessions_with_expiry() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();
        let (mut editor_a, _) = live_conn(&state, "u-proc", now).await;
        let (mut editor_b, _) = live_conn(&state, "u-admin", now + 1).await;
        let mut sub_b = editor_b.joined.take().expect("b subscribed");
        let a_sid = editor_a.session.as_ref().expect("a session").session_id.clone();

        let reactions = claim_frame(&state, &mut editor_a, "P-1001", true).await;
        // Apply publish reactions to the bus as the WS loop does.
        for reaction in &reactions {
            if let crate::ws::Reaction::Publish { drawing_id, message } = reaction {
                state.bus.publish(drawing_id, message);
            }
        }
        // Peer B receives ClaimGranted with holder and expires_at.
        let mut heard_grant = None;
        while let Some(msg) = sub_b.try_recv() {
            if let ocs_collab::ServerMessage::ClaimGranted { .. } = msg {
                heard_grant = Some(msg);
            }
        }
        match heard_grant.expect("peer B must receive ClaimGranted broadcast") {
            ocs_collab::ServerMessage::ClaimGranted {
                tag,
                session_id,
                user_id,
                expires_at,
                ..
            } => {
                assert_eq!(tag, "P-1001");
                assert_eq!(session_id, a_sid);
                assert_eq!(user_id.as_deref(), Some("u-proc"));
                assert!(expires_at.expect("expires_at must be present") > now);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// #29: Read-only sessions (denied at role, PBS, or state tier) cannot claim a tag;
    /// an editable designer can then claim the same tag.
    #[tokio::test]
    async fn readonly_sessions_cannot_claim_tag_at_any_tier() {
        let state = audited_state();
        // Add a designer with no PBS ACL (`u-nobody`) and a read-only drawing (`d-b1-ro`).
        let extra: crate::store::SeedDoc = serde_json::from_value(json!({
            "users": [
                {"user_id": "u-nobody", "display_name": "无节点权限甲", "role": "工艺设计人员"}
            ],
            "drawings": [
                {"drawing_id": "d-b1-ro", "file_name": "d-b1-ro.dwg", "pbs_node_id": "n-cell-a1", "state": "只读"}
            ]
        }))
        .expect("extra seed");
        state.store.as_ref().expect("store").seed(&extra).expect("seed extra");
        let now = ocs_collab::now_unix();

        // 1. Role tier denial: proofreader (`u-check`) joins and tries to claim P-1001.
        let (mut proofreader, _) = live_conn(&state, "u-check", now).await;
        let reactions = claim_frame(&state, &mut proofreader, "P-1001", true).await;
        let err = reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, level, .. }) => {
                    Some((code.clone(), level.clone()))
                }
                _ => None,
            })
            .expect("proofreader claim must be denied");
        assert_eq!(err.0, codes::PERM_DENIED_ROLE);
        assert_eq!(err.1.as_deref(), Some("role"));
        assert!(
            state.occupancy.lock().await.is_empty(),
            "occupancy must remain empty after denied claim"
        );

        // 2. PBS tier denial: `u-nobody` (ProcessDesigner with no PBS ACL) tries to claim P-1001.
        let (mut no_pbs, _) = live_conn(&state, "u-nobody", now + 1).await;
        let reactions = claim_frame(&state, &mut no_pbs, "P-1001", true).await;
        let err = reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, level, .. }) => {
                    Some((code.clone(), level.clone()))
                }
                _ => None,
            })
            .expect("no-PBS user claim must be denied");
        assert_eq!(err.0, codes::PERM_DENIED_PBS);
        assert_eq!(err.1.as_deref(), Some("pbs"));
        assert!(state.occupancy.lock().await.is_empty());

        // 3. State tier denial: designer joins read-only drawing `d-b1-ro` and tries to claim.
        let mut ro_conn = crate::ws::Connection::new("127.0.0.1");
        let ro_hello = off_the_wire(json!({
            "type": "hello", "user_id": "u-proc", "drawing_id": "d-b1-ro", "mode": "协同编辑"
        }));
        crate::ws::handle_message(&state, &mut ro_conn, ro_hello, now + 2).await;
        let reactions = claim_frame(&state, &mut ro_conn, "P-1001", true).await;
        let err = reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, level, .. }) => {
                    Some((code.clone(), level.clone()))
                }
                _ => None,
            })
            .expect("readonly drawing claim must be denied");
        assert_eq!(err.0, codes::PERM_DENIED_STATE);
        assert_eq!(err.1.as_deref(), Some("state"));
        assert!(state.occupancy.lock().await.is_empty());

        // 4. Designer with write access claims P-1001 cleanly.
        let (mut designer, _) = live_conn(&state, "u-proc", now + 3).await;
        let granted = claim_frame(&state, &mut designer, "P-1001", true).await;
        assert!(
            granted.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::Publish {
                    message: ocs_collab::ServerMessage::ClaimGranted { tag, .. },
                    ..
                } if tag == "P-1001"
            )),
            "designer must succeed claiming P-1001: {granted:?}"
        );
    }

    /// #27 + #28: `display_name` is resolved from `users` and carried across
    /// welcome, peers, claims, yield/delete errors, conflicts, audit, and objects.
    #[tokio::test]
    async fn display_name_flows_through_members_claims_errors_conflicts_audit_and_objects() {
        let state = audited_state();
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();

        // 1. u-proc ("工艺甲") joins and claims P-1001.
        let (mut a, a_reactions) = live_conn(&state, "u-proc", now).await;
        let a_welcome = a_reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Welcome { display_name, .. }) => {
                    Some(display_name.clone())
                }
                _ => None,
            })
            .expect("a welcome");
        assert_eq!(a_welcome, "工艺甲");

        let claim_a = claim_frame(&state, &mut a, "P-1001", true).await;
        assert!(claim_a.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Publish {
                message: ocs_collab::ServerMessage::ClaimGranted { display_name: Some(dn), .. },
                ..
            } if dn == "工艺甲"
        )));

        // 2. u-admin ("管理员") joins and sees u-proc's display_name in peers & claims, and own display_name in welcome.
        let (mut b, b_reactions) = live_conn(&state, "u-admin", now + 1).await;
        let (b_dn, b_peers, b_claims) = b_reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Welcome {
                    display_name,
                    peers,
                    claims,
                    ..
                }) => Some((display_name.clone(), peers.clone(), claims.clone())),
                _ => None,
            })
            .expect("b welcome");
        assert_eq!(b_dn, "管理员");
        assert_eq!(b_peers.len(), 1);
        assert_eq!(b_peers[0].display_name, "工艺甲");
        assert_eq!(b_claims.len(), 1);
        assert_eq!(b_claims[0].display_name, "工艺甲");

        // 3. u-admin tries to claim P-1001 -> ClaimDenied includes holder_display_name = "工艺甲".
        let denied_claim = claim_frame(&state, &mut b, "P-1001", true).await;
        assert!(denied_claim.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::ClaimDenied {
                holder_user_id: Some(uid),
                holder_display_name: Some(dn),
                ..
            }) if uid == "u-proc" && dn == "工艺甲"
        )));

        // Release P-1001 so both can write.
        let _ = claim_frame(&state, &mut a, "P-1001", false).await;

        // 4. u-proc writes P-1001.设计压力 = 1.6MPa (rev 0 -> 1); u-admin writes stale rev 0 -> OP_SUPERSEDED carries "工艺甲（u-proc）".
        let a_sid = a.session.as_ref().expect("a").session_id.clone();
        let b_sid = b.session.as_ref().expect("b").session_id.clone();
        let _ = crate::ws::handle_message(
            &state,
            &mut a,
            ocs_collab::ClientMessage::Op {
                session_id: a_sid.clone(),
                base_revision: 0,
                op_id: "op-dn-1".into(),
                payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "1.6MPa"}),
            },
            now + 2,
        )
        .await;
        let yielded = crate::ws::handle_message(
            &state,
            &mut b,
            ocs_collab::ClientMessage::Op {
                session_id: b_sid.clone(),
                base_revision: 0,
                op_id: "op-dn-2".into(),
                payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa"}),
            },
            now + 3,
        )
        .await;
        assert!(
            yielded.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error {
                    code,
                    message,
                    actor_user_id: Some(uid),
                    actor_display_name: Some(dn),
                    ..
                }) if code == codes::OP_SUPERSEDED
                    && uid == "u-proc"
                    && dn == "工艺甲"
                    && message.contains("工艺甲（u-proc）")
            )),
            "yield error must include display_name: {yielded:?}"
        );

        // 5. u-proc deletes P-1001 (rev 1 -> 2); u-admin writes stale rev 1 -> OBJECT_DELETED carries "工艺甲（u-proc）".
        let _ = crate::ws::handle_message(
            &state,
            &mut a,
            ocs_collab::ClientMessage::Op {
                session_id: a_sid.clone(),
                base_revision: 1,
                op_id: "op-dn-3".into(),
                payload: json!({"kind": "delete_object", "object_ref": "P-1001"}),
            },
            now + 4,
        )
        .await;
        let deleted_err = crate::ws::handle_message(
            &state,
            &mut b,
            ocs_collab::ClientMessage::Op {
                session_id: b_sid.clone(),
                base_revision: 1,
                op_id: "op-dn-4".into(),
                payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "公称直径", "value": "DN80"}),
            },
            now + 5,
        )
        .await;
        assert!(
            deleted_err.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error {
                    code,
                    message,
                    actor_user_id: Some(uid),
                    actor_display_name: Some(dn),
                    ..
                }) if code == codes::OBJECT_DELETED
                    && uid == "u-proc"
                    && dn == "工艺甲"
                    && message.contains("工艺甲（u-proc）")
            )),
            "deleted error must include display_name: {deleted_err:?}"
        );

        // 6. u-proc writes P-2001 -> GET /drawing/{id}/objects & GET /audit both include display_name = "工艺甲".
        let _ = crate::ws::handle_message(
            &state,
            &mut a,
            ocs_collab::ClientMessage::Op {
                session_id: a_sid,
                base_revision: 2,
                op_id: "op-dn-5".into(),
                payload: json!({"kind": "set_property", "object_ref": "P-2001", "field": "公称直径", "value": "DN50"}),
            },
            now + 6,
        )
        .await;
        let objs = objects_listing(&state, "d-a1-coop", Some("P-2001")).await;
        assert_eq!(objs["items"][0]["fields"]["公称直径"]["display_name"], "工艺甲");

        let rows = audit_rows(port, "tag=P-2001").await;
        assert_eq!(rows[0]["display_name"], "工艺甲");

        // 7. Unregistered user in open mode gets empty display_name (never empty parentheses).
        let open_state = AppState::new(
            Some(Store::open_in_memory().expect("store")),
            None,
            ":memory:",
            SessionConfig::default(),
        )
        .with_open_mode(true);
        let (unreg, unreg_reactions) = live_conn(&open_state, "u-unregistered", now + 7).await;
        let unreg_dn = unreg_reactions
            .iter()
            .find_map(|r| match r {
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::Welcome { display_name, .. }) => {
                    Some(display_name.clone())
                }
                _ => None,
            })
            .expect("unreg welcome");
        assert_eq!(unreg_dn, "");
        assert_eq!(unreg.session.as_ref().expect("s").display_name, "");
        assert_eq!(ocs_collab::protocol::format_user_label(&unreg_dn, "u-unregistered"), "u-unregistered");
    }

    /// #26: Audit entries are written for claim expiry, session takeover, and
    /// drawing state switch; HTTP `/audit` and direct store reads match 100%.
    #[tokio::test]
    async fn audit_records_claim_expiry_session_takeover_and_drawing_state_switch() {
        let state = audited_state();
        let port = serve(&state).await;
        let t0 = 1_700_000_000_i64;

        // 1. Claim expiry audit row (object_ref = P-9001).
        let (mut editor, _) = live_conn(&state, "u-proc", t0).await;
        let sid = editor.session.as_ref().expect("session").session_id.clone();
        let _ = crate::ws::handle_message(
            &state,
            &mut editor,
            ocs_collab::ClientMessage::Claim {
                session_id: sid.clone(),
                tag: "P-9001".into(),
            },
            t0,
        )
        .await;
        // Sweep after claim TTL (300s) -> claim expires and writes `claim_expired` audit.
        let _ = crate::ws::sweep_expired(&state, t0 + 301).await;

        let store_claim_rows = state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                object_ref: Some("P-9001".into()),
                ..AuditQuery::default()
            })
            .expect("store audit query");
        let http_claim_rows = audit_rows(port, "tag=P-9001").await;
        assert_eq!(store_claim_rows.items.len(), 1);
        assert_eq!(http_claim_rows.len(), 1);
        assert_eq!(store_claim_rows.items[0].action, "claim_expired");
        assert_eq!(http_claim_rows[0]["action"], "claim_expired");
        assert!(store_claim_rows.items[0].change_text.contains("工艺甲（u-proc）"));
        assert!(store_claim_rows.items[0]
            .change_text
            .contains(&(t0 + 300).to_string()));

        // 2. Session takeover audit row (object_ref = d-a1-coop): `editor` became stale above;
        // a new session on `d-a1-coop` takes it over.
        let (_new_editor, _) = live_conn(&state, "u-admin", t0 + 302).await;

        // 3. Drawing state switch audit row (object_ref = d-a1-coop).
        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);

        let store_drawing_rows = state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                object_ref: Some("d-a1-coop".into()),
                ..AuditQuery::default()
            })
            .expect("store drawing audit");
        let http_drawing_rows = audit_rows(port, "object_ref=d-a1-coop").await;
        assert_eq!(store_drawing_rows.items.len(), http_drawing_rows.len());

        let takeover = store_drawing_rows
            .items
            .iter()
            .find(|r| r.action == "session_taken_over")
            .expect("session_taken_over audit row must exist");
        assert!(takeover.change_text.contains(&sid));
        assert!(takeover.change_text.contains("工艺甲（u-proc）"));

        let switched = store_drawing_rows
            .items
            .iter()
            .find(|r| r.action == "drawing_state_changed")
            .expect("drawing_state_changed audit row must exist");
        assert_eq!(switched.user_id, "u-admin");
        assert!(switched.change_text.contains("管理员（u-admin）"));
        assert!(switched.change_text.contains("协同编辑"));
        assert!(switched.change_text.contains("只读"));
    }

    /// #31 + #32 over HTTP: deleting a pipe line unpairs the downstream
    /// connector, and `GET /drawing/{id}/unpaired-connectors` reports all 3
    /// reasons and clears paired connectors.
    #[tokio::test]
    async fn http_unpaired_connectors_endpoint_and_line_deletion_unpair() {
        let state = audited_state();
        let extra: crate::store::SeedDoc = serde_json::from_value(json!({
            "drawings": [
                {"drawing_id": "d-a2", "file_name": "d-a2.dwg", "pbs_node_id": "n-cell-a1", "state": "协同编辑"},
                {"drawing_id": "d-a3", "file_name": "d-a3.dwg", "pbs_node_id": "n-cell-a1", "state": "协同编辑"}
            ]
        }))
        .expect("extra seed");
        state.store.as_ref().expect("store").seed(&extra).expect("seed");
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();

        let join_drawing = |drawing_id: &'static str, user_id: &'static str| {
            let state = state.clone();
            async move {
                let mut conn = crate::ws::Connection::new("127.0.0.1");
                let h = off_the_wire(json!({
                    "type": "hello", "user_id": user_id, "drawing_id": drawing_id, "mode": "协同编辑"
                }));
                crate::ws::handle_message(&state, &mut conn, h, now).await;
                conn
            }
        };

        let mut c1 = join_drawing("d-a1-coop", "u-proc").await;
        let mut c2 = join_drawing("d-a2", "u-admin").await;
        let mut c3 = join_drawing("d-a3", "u-proc").await;

        async fn send_op(
            state: &AppState,
            conn: &mut crate::ws::Connection,
            op_id: &str,
            payload: serde_json::Value,
            now: i64,
        ) {
            let sid = conn.session.as_ref().expect("s").session_id.clone();
            let did = conn.session.as_ref().expect("s").drawing_id.clone();
            let rev = crate::ws::current_revision(state, &did);
            let _ = crate::ws::handle_message(
                state,
                conn,
                ocs_collab::ClientMessage::Op {
                    session_id: sid,
                    base_revision: rev,
                    op_id: op_id.into(),
                    payload,
                },
                now + 1,
            )
            .await;
        }

        // 1. Unregistered target drawing
        send_op(
            &state,
            &mut c1,
            "op-u1",
            json!({"kind": "create_connector", "line_tag": "L-UNREG", "connector": "pipe_out", "at": [10, 0], "target_drawing": "d-ghost"}),
            now,
        )
        .await;
        // 2. Registered target drawing d-a2, no matching inflow yet
        send_op(
            &state,
            &mut c1,
            "op-u2",
            json!({"kind": "create_connector", "line_tag": "L-MISS", "connector": "pipe_out", "at": [20, 0], "target_drawing": "d-a2"}),
            now,
        )
        .await;
        // 3. Registered target drawing d-a2 already paired with d-a3 on L-TAKEN
        send_op(
            &state,
            &mut c3,
            "op-u3",
            json!({"kind": "create_connector", "line_tag": "L-TAKEN", "connector": "pipe_out", "at": [30, 0], "target_drawing": "d-a2"}),
            now,
        )
        .await;
        send_op(
            &state,
            &mut c2,
            "op-u4",
            json!({"kind": "create_connector", "line_tag": "L-TAKEN", "connector": "pipe_in", "at": [5, 0]}),
            now,
        )
        .await;
        send_op(
            &state,
            &mut c1,
            "op-u5",
            json!({"kind": "create_connector", "line_tag": "L-TAKEN", "connector": "pipe_out", "at": [40, 0], "target_drawing": "d-a2"}),
            now,
        )
        .await;

        let (status, body) = http_exchange(port, "GET", "/drawing/d-a1-coop/unpaired-connectors").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["total"], 3, "{body}");
        let items = body["items"].as_array().expect("items array");
        let reason_of = |tag: &str| {
            items
                .iter()
                .find(|i| i["line_tag"] == tag)
                .and_then(|i| i["reason_code"].as_str())
        };
        assert_eq!(reason_of("L-UNREG"), Some("target_drawing_unregistered"));
        assert_eq!(reason_of("L-MISS"), Some("no_matching_connector"));
        assert_eq!(reason_of("L-TAKEN"), Some("target_already_paired"));

        // Pair L-MISS -> disappears from unpaired-connectors.
        send_op(
            &state,
            &mut c2,
            "op-u6",
            json!({"kind": "create_connector", "line_tag": "L-MISS", "connector": "pipe_in", "at": [15, 0]}),
            now,
        )
        .await;
        let (_, body_after) = http_exchange(port, "GET", "/drawing/d-a1-coop/unpaired-connectors").await;
        assert_eq!(body_after["total"], 2, "{body_after}");

        // Delete entire line L-MISS on d-a1-coop -> d-a2's inflow returns to unpaired (#31)!
        send_op(
            &state,
            &mut c1,
            "op-u7",
            json!({"kind": "delete_object", "object_ref": "L-MISS"}),
            now,
        )
        .await;
        let (_, topo_b) = http_exchange(port, "GET", "/drawing/d-a2/topology").await;
        let miss_in = topo_b["connectors"]
            .as_array()
            .expect("connectors")
            .iter()
            .find(|c| c["line_tag"] == "L-MISS")
            .expect("L-MISS inflow on d-a2");
        assert!(miss_in["peer_drawing"].is_null(), "peer must be cleared: {miss_in}");
    }

    /// #70 + #71 + #72: Read-only drawing switches back to 协同编辑, downgraded
    /// sessions automatically receive `WriteRestored` without reconnecting,
    /// `RequestWrite` answers immediately with `WriteStatus`, and single-editor
    /// handoff restores write access when the prior holder expires.
    #[tokio::test]
    async fn readonly_restore_request_write_and_single_editor_handoff() {
        let state = seeded_state();
        let now = ocs_collab::now_unix();

        let (mut editor_a, _) = live_conn(&state, "u-proc", now - 20).await;
        let a_sid = editor_a.session.as_ref().expect("a").session_id.clone();
        let (a_tx, mut a_rx) = tokio::sync::mpsc::unbounded_channel();
        state.direct.lock().expect("direct").insert(a_sid.clone(), a_tx);

        let (_viewer, mut viewer_rx) = live_session(&state, "u-view", "只读", now - 15).await;

        // Switch to 只读 -> editor_a gets ReadonlyDowngrade(holder=None), write refused, RequestWrite -> writable=false.
        assert_eq!(switch_state(&state, "只读").await, StatusCode::OK);
        assert!(matches!(
            a_rx.try_recv(),
            Ok(ocs_collab::ServerMessage::ReadonlyDowngrade { holder: None, .. })
        ));
        let _ = viewer_rx.try_recv();
        assert!(refusal_in(&write(&state, &mut editor_a, "op-ro", "1.0MPa").await).is_some());

        // #72: RequestWrite while 只读 -> writable: false with state tier reason.
        let req_denied = crate::ws::handle_message(
            &state,
            &mut editor_a,
            ocs_collab::ClientMessage::RequestWrite {
                session_id: a_sid.clone(),
            },
            now,
        )
        .await;
        assert!(
            req_denied.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::WriteStatus {
                    writable: false,
                    level: Some(lvl),
                    code: Some(code),
                    ..
                }) if lvl == "state" && code == codes::PERM_DENIED_STATE
            )),
            "{req_denied:?}"
        );

        // #70: Designer cannot switch state back (role tier), admin can!
        let denied_switch = drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-proc".into()),
                state: Some("协同编辑".into()),
                credential: None,
            }),
        )
        .await;
        assert_eq!(denied_switch.status(), StatusCode::FORBIDDEN);
        assert_eq!(switch_state(&state, "协同编辑").await, StatusCode::OK);

        // #71: editor_a automatically receives WriteRestored; viewer does NOT.
        assert!(matches!(
            a_rx.try_recv(),
            Ok(ocs_collab::ServerMessage::WriteRestored { .. })
        ));
        assert!(viewer_rx.try_recv().is_err(), "viewer must not get WriteRestored");

        // #72: RequestWrite now returns writable: true immediately!
        let req_ok = crate::ws::handle_message(
            &state,
            &mut editor_a,
            ocs_collab::ClientMessage::RequestWrite {
                session_id: a_sid.clone(),
            },
            now + 1,
        )
        .await;
        assert!(
            req_ok.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::Send(ocs_collab::ServerMessage::WriteStatus {
                    writable: true,
                    ..
                })
            )),
            "{req_ok:?}"
        );
        // And editor_a can write immediately without reconnecting!
        assert!(applied_in(&write(&state, &mut editor_a, "op-back", "1.6MPa").await));

        // #71 part 2: Single-editor handoff on holder expiry!
        let (mut editor_b, _) = live_conn(&state, "u-admin", now + 2).await;
        let b_sid = editor_b.session.as_ref().expect("b").session_id.clone();
        let (b_tx, mut b_rx) = tokio::sync::mpsc::unbounded_channel();
        state.direct.lock().expect("direct").insert(b_sid.clone(), b_tx);

        assert_eq!(switch_state(&state, "单人编辑").await, StatusCode::OK);
        assert!(matches!(
            b_rx.try_recv(),
            Ok(ocs_collab::ServerMessage::ReadonlyDowngrade { holder: Some(ref h), .. }) if h == &a_sid
        ));

        // Keep b alive with heartbeat, let a expire -> sweep emits SendTo(b, WriteRestored)!
        let _ = crate::ws::handle_message(
            &state,
            &mut editor_b,
            ocs_collab::ClientMessage::Heartbeat {
                session_id: b_sid.clone(),
            },
            now + 50,
        )
        .await;
        let swept = crate::ws::sweep_expired(&state, now + 100).await;
        assert!(
            swept.iter().any(|r| matches!(
                r,
                crate::ws::Reaction::SendTo {
                    session_id,
                    message: ocs_collab::ServerMessage::WriteRestored { .. }
                } if session_id == &b_sid
            )),
            "second editor must receive WriteRestored when first holder expires: {swept:?}"
        );
        assert!(applied_in(&write(&state, &mut editor_b, "op-handoff", "2.0MPa").await));
    }

    /// #67: Strict mode is default: unregistered users get `UNKNOWN_USER` and
    /// unregistered drawings get `UNKNOWN_DRAWING`, with no session or drawing
    /// row created. `--open-mode` explicitly enables open access and is
    /// reported in both `Welcome.open_mode` and `GET /health`.
    #[tokio::test]
    async fn strict_mode_rejects_unregistered_users_and_drawings_and_open_mode_reports_status() {
        let state = seeded_state();
        assert!(!state.open_mode, "strict mode must be default");
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();

        let (status, health) = http_exchange(port, "GET", "/health").await;
        assert_eq!(status, 200);
        assert_eq!(health["open_mode"], false);

        let before_drawings = state
            .store
            .as_ref()
            .expect("store")
            .drawing_ids()
            .expect("ids")
            .len();

        // Unregistered user -> UNKNOWN_USER + Close, no session.
        let mut c1 = crate::ws::Connection::new("127.0.0.1");
        let r1 = crate::ws::handle_message(
            &state,
            &mut c1,
            off_the_wire(json!({
                "type": "hello", "user_id": "u-stranger", "drawing_id": "d-a1-coop", "mode": "协同编辑"
            })),
            now,
        )
        .await;
        assert!(c1.session.is_none());
        assert!(r1.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, .. })
                if code == codes::UNKNOWN_USER
        )));
        assert!(r1.iter().any(|r| matches!(r, crate::ws::Reaction::Close)));

        // Registered user + unregistered drawing -> UNKNOWN_DRAWING + Close, no drawing auto-created.
        let mut c2 = crate::ws::Connection::new("127.0.0.1");
        let r2 = crate::ws::handle_message(
            &state,
            &mut c2,
            off_the_wire(json!({
                "type": "hello", "user_id": "u-proc", "drawing_id": "d-unregistered", "mode": "协同编辑"
            })),
            now,
        )
        .await;
        assert!(c2.session.is_none());
        assert!(r2.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, .. })
                if code == codes::UNKNOWN_DRAWING
        )));
        assert_eq!(
            state
                .store
                .as_ref()
                .expect("store")
                .drawing_ids()
                .expect("ids")
                .len(),
            before_drawings,
            "strict mode must not insert unregistered drawing rows"
        );

        // Registered user + registered drawing -> Welcome with open_mode = Some(false).
        let (c3, r3) = live_conn(&state, "u-proc", now).await;
        assert!(c3.session.is_some());
        assert!(r3.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Welcome {
                open_mode: Some(false),
                ..
            })
        )));

        // Explicit open_mode -> Welcome & /health report open_mode = true.
        let open = Arc::new(
            AppState::new(
                Some(Store::open_in_memory().expect("store")),
                None,
                ":memory:",
                SessionConfig::default(),
            )
            .with_open_mode(true),
        );
        let open_port = serve(&open).await;
        let (_, open_health) = http_exchange(open_port, "GET", "/health").await;
        assert_eq!(open_health["open_mode"], true);
        let (c4, r4) = live_conn(&open, "u-anyone", now).await;
        assert!(c4.session.is_some());
        assert!(r4.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Welcome {
                open_mode: Some(true),
                ..
            })
        )));
    }

    /// #68: Credential authentication on WS join and HTTP privileged endpoints
    /// (`/drawing/{id}/state`, `/conflicts/{id}/resolve`), disabled account
    /// rejection, hash-only SQLite storage, and verified identity derivation.
    #[tokio::test]
    async fn credential_authentication_guards_ws_and_privileged_http_endpoints() {
        let store = Store::open_in_memory().expect("store");
        let seed: crate::store::SeedDoc = serde_json::from_value(json!({
            "users": [
                {"user_id": "u-admin", "display_name": "管理员", "role": "系统管理员", "credential": "admin-secret-42", "active": true},
                {"user_id": "u-proc", "display_name": "工艺甲", "role": "工艺设计人员", "credential": "proc-secret-99", "active": true},
                {"user_id": "u-disabled", "display_name": "离职员工", "role": "工艺设计人员", "credential": "old-secret", "active": false}
            ],
            "pbs_nodes": [{"node_id": "n-plant", "name": "全厂", "node_type": "项目"}],
            "pbs_acl": [
                {"node_id": "n-plant", "subject": "u-admin", "permission": "编辑", "allowed": true},
                {"node_id": "n-plant", "subject": "u-proc", "permission": "编辑", "allowed": true}
            ],
            "drawings": [
                {"drawing_id": "d-a1-coop", "file_name": "d-a1-coop.dwg", "pbs_node_id": "n-plant", "state": "协同编辑"}
            ]
        }))
        .expect("seed");
        store.seed(&seed).expect("apply seed");

        // Verify SQLite stores ONLY hash, never plaintext secret!
        let rec = store.user_auth("u-admin").expect("auth").expect("exists");
        assert!(rec.credential_hash.starts_with("pbkdf2_sha256$"));
        assert!(!rec.credential_hash.contains("admin-secret-42"));

        let state = Arc::new(
            AppState::new(Some(store), None, ":memory:", SessionConfig::default())
                .with_require_auth(true),
        );
        let now = ocs_collab::now_unix();

        // 1. Wrong credential -> AUTH_FAILED + Close.
        let mut bad = crate::ws::Connection::new("127.0.0.1");
        let r_bad = crate::ws::handle_message(
            &state,
            &mut bad,
            off_the_wire(json!({
                "type": "hello", "user_id": "u-proc", "drawing_id": "d-a1-coop",
                "mode": "协同编辑", "credential": "wrong-pass"
            })),
            now,
        )
        .await;
        assert!(bad.session.is_none());
        assert!(r_bad.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, .. })
                if code == codes::AUTH_FAILED
        )));
        assert!(r_bad.iter().any(|r| matches!(r, crate::ws::Reaction::Close)));

        // 2. Disabled account -> USER_DISABLED + Close (even with valid credential!).
        let mut dis = crate::ws::Connection::new("127.0.0.1");
        let r_dis = crate::ws::handle_message(
            &state,
            &mut dis,
            off_the_wire(json!({
                "type": "hello", "user_id": "u-disabled", "drawing_id": "d-a1-coop",
                "mode": "协同编辑", "credential": "old-secret"
            })),
            now,
        )
        .await;
        assert!(dis.session.is_none());
        assert!(r_dis.iter().any(|r| matches!(
            r,
            crate::ws::Reaction::Send(ocs_collab::ServerMessage::Error { code, .. })
                if code == codes::USER_DISABLED
        )));

        // 3. Valid credential -> Welcome!
        let mut ok_conn = crate::ws::Connection::new("127.0.0.1");
        let r_ok = crate::ws::handle_message(
            &state,
            &mut ok_conn,
            off_the_wire(json!({
                "type": "hello", "user_id": "u-proc", "drawing_id": "d-a1-coop",
                "mode": "协同编辑", "credential": "proc-secret-99"
            })),
            now,
        )
        .await;
        assert!(ok_conn.session.is_some(), "{r_ok:?}");

        // 4. HTTP /drawing/{id}/state without credential -> 401 AUTH_FAILED.
        let unauth = drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            HeaderMap::new(),
            Json(DrawingStateBody {
                user_id: Some("u-admin".into()),
                state: Some("只读".into()),
                credential: None,
            }),
        )
        .await;
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // 5. HTTP /drawing/{id}/state with Bearer <user>:<secret> (no user_id in body!) -> 200,
        // and audit actor is derived from verified Bearer identity ("u-admin")!
        let mut auth_headers = HeaderMap::new();
        auth_headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer u-admin:admin-secret-42".parse().expect("hdr"),
        );
        let authed = drawing_state(
            State(state.clone()),
            Path("d-a1-coop".into()),
            auth_headers.clone(),
            Json(DrawingStateBody {
                user_id: None,
                state: Some("只读".into()),
                credential: None,
            }),
        )
        .await;
        assert_eq!(authed.status(), StatusCode::OK);
        let audit = state
            .store
            .as_ref()
            .expect("store")
            .audit_query(&AuditQuery {
                object_ref: Some("d-a1-coop".into()),
                ..AuditQuery::default()
            })
            .expect("audit");
        assert_eq!(audit.items[0].user_id, "u-admin");
        assert!(!audit.items[0].change_text.contains("admin-secret-42"));

        // 6. HTTP /conflicts/{id}/resolve without credential -> 401 AUTH_FAILED.
        let unauth_resolve = conflict_resolve(
            State(state.clone()),
            Path("c-nonexistent".into()),
            HeaderMap::new(),
            Json(ResolveBody {
                resolution: Some("采用甲方".into()),
                resolved_by: Some("u-admin".into()),
                user_id: None,
                value: None,
                resolved_value: None,
                credential: None,
            }),
        )
        .await;
        assert_eq!(unauth_resolve.status(), StatusCode::UNAUTHORIZED);
    }

    /// #39: Read-only `GET /metrics` reports per-drawing & total sessions,
    /// claims, ops/min, denied writes, pending conflicts, op-log rows, and DB
    /// size; 1 edit + 1 denied write increments each by 1; structured log file
    /// writes one JSON object per line without secrets.
    #[tokio::test]
    async fn metrics_endpoint_and_structured_log_file_track_ops_and_denials() {
        let state = seeded_state();
        let log_path = std::env::temp_dir().join(format!("ocs-log-{}.jsonl", uuid::Uuid::new_v4()));
        state.logger.set_file(&log_path).expect("attach log file");
        let port = serve(&state).await;
        let now = ocs_collab::now_unix();

        // Read-only check: POST /metrics -> 405.
        let (method_status, _) = http_exchange(port, "POST", "/metrics").await;
        assert_eq!(method_status, 405);

        let (mut editor, _) = live_conn(&state, "u-proc", now).await;
        let (mut viewer, _) = live_conn(&state, "u-view", now).await;
        let _ = claim_frame(&state, &mut editor, "P-1001", true).await;

        let (status0, m0) = http_exchange(port, "GET", "/metrics").await;
        assert_eq!(status0, 200, "{m0}");
        let applied_before = m0["totals"]["applied_ops_total"].as_u64().unwrap();
        let denied_before = m0["totals"]["denied_writes_total"].as_u64().unwrap();
        assert_eq!(m0["totals"]["active_sessions"], 2);
        assert_eq!(m0["totals"]["claims"], 1);

        // 1 successful property edit + 1 denied write.
        assert!(applied_in(&write(&state, &mut editor, "op-m1", "1.6MPa").await));
        assert!(refusal_in(&write(&state, &mut viewer, "op-m2", "9.9MPa").await).is_some());

        let (status1, m1) = http_exchange(port, "GET", "/metrics").await;
        assert_eq!(status1, 200, "{m1}");
        assert_eq!(m1["totals"]["applied_ops_total"], applied_before + 1);
        assert_eq!(m1["totals"]["denied_writes_total"], denied_before + 1);
        assert_eq!(m1["totals"]["ops_log_rows"], 1);
        assert!(m1["totals"]["db_size_bytes"].is_number());
        let d_entry = m1["drawings"]
            .as_array()
            .expect("drawings")
            .iter()
            .find(|d| d["drawing_id"] == "d-a1-coop")
            .expect("d-a1-coop row");
        assert_eq!(d_entry["ops_per_minute"], 1);
        assert_eq!(d_entry["denied_writes"], 1);
        assert_eq!(d_entry["ops_log_rows"], 1);
        assert_eq!(d_entry["pending_conflicts"], 0);

        // Structured log file has one JSON line per event.
        let log_content = std::fs::read_to_string(&log_path).expect("read log file");
        let lines: Vec<serde_json::Value> = log_content
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid JSON line"))
            .collect();
        assert!(lines.iter().any(|l| l["event"] == "op_applied"), "{lines:?}");
        assert!(lines.iter().any(|l| l["event"] == "write_denied"), "{lines:?}");
        let _ = std::fs::remove_file(&log_path);
    }
}

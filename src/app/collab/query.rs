//! Synchronous HTTP query client for the collaboration server (#40, #41, #42, #66).
//!
//! Deliberately synchronous (`ureq` on desktop, stub on wasm) so nothing under
//! `src/app/collab/` pulls in an async runtime (`VAL-CLI-019`). All network
//! calls carry bounded timeouts and can be swapped with an in-memory stub via
//! [`CollabQueryClient`] for fast unit tests without a live server.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{PendingConflict, UnpairedConnectorInfo, UnpairedReason};

/// Convert a WebSocket server URL (`ws://host:port/ws`) to its HTTP control
/// plane base URL (`http://host:port`), stripping any userinfo credentials.
pub fn http_base_from_ws(ws_url: &str) -> Result<String, String> {
    let trimmed = ws_url.trim();
    if trimmed.is_empty() {
        return Err("未配置协同服务器地址".to_owned());
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("服务器地址无效：{e}"))?;
    let scheme = match parsed.scheme() {
        "ws" | "http" => "http",
        "wss" | "https" => "https",
        other => return Err(format!("不支持的协议：{other}")),
    };
    let host = parsed
        .host_str()
        .ok_or_else(|| "服务器地址缺少主机名".to_owned())?;
    match parsed.port() {
        Some(port) => Ok(format!("{scheme}://{host}:{port}")),
        None => Ok(format!("{scheme}://{host}")),
    }
}

/// Structured error returned by the collaboration server HTTP control plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub level: Option<String>,
    pub detail: Option<String>,
}

impl QueryError {
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            status: 0,
            code: "TRANSPORT_ERROR".to_owned(),
            message: message.into(),
            level: None,
            detail: None,
        }
    }

    pub fn from_json(status: u16, body: &Value) -> Self {
        let code = body["code"]
            .as_str()
            .unwrap_or("SERVER_ERROR")
            .to_owned();
        let message = body["message"]
            .as_str()
            .or_else(|| body["reason"].as_str())
            .unwrap_or("服务器返回错误")
            .to_owned();
        let level = body["level"]
            .as_str()
            .or_else(|| body["denied_level"].as_str())
            .map(str::to_owned);
        let detail = body["resolved_by"]
            .as_str()
            .map(|by| {
                let res = body["resolution"].as_str().unwrap_or("");
                if res.is_empty() {
                    format!("已由 {by} 裁决")
                } else {
                    format!("已由 {by} 裁决（{res}）")
                }
            });
        Self {
            status,
            code,
            message,
            level,
            detail,
        }
    }

    /// Human-readable Chinese summary for command line and toast notifications.
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if let Some(level) = &self.level {
            let level_zh = match level.as_str() {
                "role" => "角色权限",
                "pbs" => "PBS 节点 ACL",
                "state" => "图纸状态",
                other => other,
            };
            out.push_str(&format!("[{level_zh}] "));
        }
        out.push_str(&self.message);
        if let Some(detail) = &self.detail {
            out.push_str(&format!("（{detail}）"));
        }
        if !self.code.is_empty() && self.code != "TRANSPORT_ERROR" {
            out.push_str(&format!(" [{}]", self.code));
        }
        out
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// Server response for `POST /conflicts/{id}/resolve`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveOutcome {
    pub conflict_id: String,
    pub status: String,
    pub resolution: String,
    pub resolved_by: String,
    #[serde(default)]
    pub object_ref: String,
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub revision: i64,
}

/// Filter parameters for `GET /audit`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditFilter {
    pub object_ref: Option<String>,
    pub user_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
}

/// One row from `GET /audit`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditItem {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub at: i64,
    #[serde(default)]
    pub object_ref: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub change_text: String,
}

impl AuditItem {
    /// Human-readable Chinese label for the audit action.
    pub fn action_label(&self) -> &str {
        match self.action.as_str() {
            "set_property" | "op_applied" => "修改属性",
            "set_tag" => "修改位号",
            "insert_symbol" => "放置符号",
            "delete_object" => "删除对象",
            "change_superseded" => "让位（被覆盖）",
            "change_rejected_deleted" => "被拒（对象已删）",
            "delete_refused" => "删除被拒",
            "write_refused" | "attempt_refused" => "写入被拒",
            "conflict_created" => "产生冲突",
            "conflict_resolved" => "冲突裁决",
            "claim_expired" => "占用到期",
            "connector_propagated" => "上游传递",
            other => other,
        }
    }
}

/// Last writer metadata for one field of one object (`GET /drawing/{id}/objects`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldLastWriter {
    pub value: String,
    pub revision: i64,
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
}

/// One object's field map from `GET /drawing/{id}/objects`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub object_ref: String,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldLastWriter>,
}

/// Response of `GET /drawing/{id}/objects`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrawingObjectsPage {
    pub drawing_id: String,
    pub revision: i64,
    pub items: Vec<ObjectSnapshot>,
}

/// Pluggable synchronous query interface for the collaboration server.
///
/// The desktop build uses [`HttpQueryClient`]; tests can inject a stub via
/// `State::set_query_client` so conflicts, resolution, history, and last-writer
/// UI can be verified deterministically without spawning a server (#40).
pub trait CollabQueryClient: Send + Sync + fmt::Debug {
    fn fetch_pending_conflicts(
        &self,
        base_url: &str,
        drawing_id: &str,
    ) -> Result<Vec<PendingConflict>, QueryError>;

    fn resolve_conflict(
        &self,
        base_url: &str,
        conflict_id: &str,
        resolution: &str,
        resolved_by: &str,
        value: Option<&str>,
    ) -> Result<ResolveOutcome, QueryError>;

    fn fetch_audit(
        &self,
        base_url: &str,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditItem>, QueryError>;

    fn fetch_drawing_objects(
        &self,
        base_url: &str,
        drawing_id: &str,
        object_ref: Option<&str>,
    ) -> Result<DrawingObjectsPage, QueryError>;

    /// Query unpaired off-page connectors for `drawing_id` (`GET /drawing/{id}/unpaired-connectors`, #32/#65).
    fn fetch_unpaired_connectors(
        &self,
        _base_url: &str,
        _drawing_id: &str,
    ) -> Result<Vec<UnpairedConnectorInfo>, QueryError> {
        Ok(Vec::new())
    }
}

/// Wrapper so `State` can keep deriving `Clone` and `Debug`.
#[derive(Clone)]
pub struct QueryClientSlot(pub Arc<dyn CollabQueryClient>);

impl fmt::Debug for QueryClientSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("QueryClientSlot").field(&self.0).finish()
    }
}

/// Real HTTP query client backed by `ureq` (blocking, bounded timeout).
#[derive(Debug, Default)]
pub struct HttpQueryClient;

#[cfg(not(target_arch = "wasm32"))]
fn http_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(3)))
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_json_response(
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<(u16, Value), QueryError> {
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| QueryError::transport(format!("读取服务器响应失败：{e}")))?;
    let json = serde_json::from_str::<Value>(&text)
        .unwrap_or_else(|_| serde_json::json!({ "message": text }));
    Ok((status, json))
}

/// Minimal percent-encoding for query parameter values (ASCII safe + UTF-8 bytes).
pub fn encode_query_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if safe {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl CollabQueryClient for HttpQueryClient {
    fn fetch_pending_conflicts(
        &self,
        base_url: &str,
        drawing_id: &str,
    ) -> Result<Vec<PendingConflict>, QueryError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (base_url, drawing_id);
            Err(QueryError::transport("网页版不支持协同查询"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let url = format!(
                "{base_url}/conflicts?drawing_id={}&status={}",
                encode_query_component(drawing_id),
                encode_query_component("待裁决"),
            );
            let resp = http_agent()
                .get(&url)
                .call()
                .map_err(|e| QueryError::transport(format!("查询待裁决冲突失败：{e}")))?;
            let (status, body) = read_json_response(resp)?;
            if !(200..300).contains(&status) {
                return Err(QueryError::from_json(status, &body));
            }
            let items = body["items"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|row| {
                            Some(PendingConflict {
                                conflict_id: row["conflict_id"].as_str()?.to_owned(),
                                object_ref: row["object_ref"].as_str()?.to_owned(),
                                field: row["field"].as_str()?.to_owned(),
                                value_a: row["value_a"].as_str()?.to_owned(),
                                value_b: row["value_b"].as_str()?.to_owned(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(items)
        }
    }

    fn resolve_conflict(
        &self,
        base_url: &str,
        conflict_id: &str,
        resolution: &str,
        resolved_by: &str,
        value: Option<&str>,
    ) -> Result<ResolveOutcome, QueryError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (base_url, conflict_id, resolution, resolved_by, value);
            Err(QueryError::transport("网页版不支持提交冲突裁决"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let url = format!(
                "{base_url}/conflicts/{}/resolve",
                encode_query_component(conflict_id)
            );
            let mut payload = serde_json::json!({
                "resolution": resolution,
                "resolved_by": resolved_by,
            });
            if let Some(val) = value {
                payload["value"] = Value::String(val.to_owned());
            }
            let resp = http_agent()
                .post(&url)
                .header("Content-Type", "application/json")
                .send(payload.to_string())
                .map_err(|e| QueryError::transport(format!("提交冲突裁决失败：{e}")))?;
            let (status, body) = read_json_response(resp)?;
            if !(200..300).contains(&status) {
                return Err(QueryError::from_json(status, &body));
            }
            serde_json::from_value(body)
                .map_err(|e| QueryError::transport(format!("解析裁决响应失败：{e}")))
        }
    }

    fn fetch_audit(
        &self,
        base_url: &str,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditItem>, QueryError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (base_url, filter);
            Err(QueryError::transport("网页版不支持查询修改历史"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut params: Vec<String> = Vec::new();
            if let Some(obj) = &filter.object_ref {
                if !obj.trim().is_empty() {
                    params.push(format!("object_ref={}", encode_query_component(obj.trim())));
                }
            }
            if let Some(user) = &filter.user_id {
                if !user.trim().is_empty() {
                    params.push(format!("user_id={}", encode_query_component(user.trim())));
                }
            }
            if let Some(from) = filter.from {
                params.push(format!("from={from}"));
            }
            if let Some(to) = filter.to {
                params.push(format!("to={to}"));
            }
            if let Some(limit) = filter.limit {
                params.push(format!("limit={limit}"));
            }
            let url = if params.is_empty() {
                format!("{base_url}/audit")
            } else {
                format!("{base_url}/audit?{}", params.join("&"))
            };
            let resp = http_agent()
                .get(&url)
                .call()
                .map_err(|e| QueryError::transport(format!("查询修改历史失败：{e}")))?;
            let (status, body) = read_json_response(resp)?;
            if !(200..300).contains(&status) {
                return Err(QueryError::from_json(status, &body));
            }
            let items = body["items"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|row| serde_json::from_value::<AuditItem>(row.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            Ok(items)
        }
    }

    fn fetch_drawing_objects(
        &self,
        base_url: &str,
        drawing_id: &str,
        object_ref: Option<&str>,
    ) -> Result<DrawingObjectsPage, QueryError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (base_url, drawing_id, object_ref);
            Err(QueryError::transport("网页版不支持查询对象最后写入者"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut url = format!(
                "{base_url}/drawing/{}/objects",
                encode_query_component(drawing_id)
            );
            if let Some(tag) = object_ref {
                if !tag.trim().is_empty() {
                    url.push_str(&format!("?object_ref={}", encode_query_component(tag.trim())));
                }
            }
            let resp = http_agent()
                .get(&url)
                .call()
                .map_err(|e| QueryError::transport(format!("查询对象字段写入者失败：{e}")))?;
            let (status, body) = read_json_response(resp)?;
            if !(200..300).contains(&status) {
                return Err(QueryError::from_json(status, &body));
            }
            let items = body["items"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|row| serde_json::from_value::<ObjectSnapshot>(row.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            Ok(DrawingObjectsPage {
                drawing_id: body["drawing_id"]
                    .as_str()
                    .unwrap_or(drawing_id)
                    .to_owned(),
                revision: body["revision"].as_i64().unwrap_or(0),
                items,
            })
        }
    }

    fn fetch_unpaired_connectors(
        &self,
        base_url: &str,
        drawing_id: &str,
    ) -> Result<Vec<UnpairedConnectorInfo>, QueryError> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (base_url, drawing_id);
            Err(QueryError::transport("网页版不支持查询未配对页间连接符"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let url = format!(
                "{base_url}/drawing/{}/unpaired-connectors",
                encode_query_component(drawing_id)
            );
            let resp = http_agent()
                .get(&url)
                .call()
                .map_err(|e| QueryError::transport(format!("查询未配对页间连接符失败：{e}")))?;
            let (status, body) = read_json_response(resp)?;
            if !(200..300).contains(&status) {
                return Err(QueryError::from_json(status, &body));
            }
            let items = body["items"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|row| {
                            let line_tag = row["line_tag"].as_str()?.to_owned();
                            let connector = row["connector"].as_str()?.to_owned();
                            let target_drawing = row["target_drawing"]
                                .as_str()
                                .unwrap_or("")
                                .to_owned();
                            let reason = row["reason_code"]
                                .as_str()
                                .and_then(UnpairedReason::parse)
                                .or_else(|| {
                                    row["reason"].as_str().and_then(UnpairedReason::parse)
                                })?;
                            Some(UnpairedConnectorInfo {
                                line_tag,
                                connector,
                                target_drawing,
                                reason,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(items)
        }
    }
}

/// Reply from a background query thread delivered back to `OpenCADStudio::update`.
#[derive(Clone, Debug)]
pub enum CollabQueryReply {
    PendingConflicts(Result<Vec<PendingConflict>, QueryError>),
    FieldWriters {
        tag: String,
        result: Result<DrawingObjectsPage, QueryError>,
    },
    AuditHistory {
        tag: String,
        result: Result<Vec<AuditItem>, QueryError>,
    },
    UnpairedConnectors(Result<Vec<UnpairedConnectorInfo>, QueryError>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_base_strips_ws_path_and_credentials() {
        assert_eq!(
            http_base_from_ws("ws://127.0.0.1:3240/ws").unwrap(),
            "http://127.0.0.1:3240"
        );
        assert_eq!(
            http_base_from_ws("ws://user:secret@192.168.1.5:8080/ws").unwrap(),
            "http://192.168.1.5:8080"
        );
        assert_eq!(
            http_base_from_ws("wss://collab.example.com/ws").unwrap(),
            "https://collab.example.com"
        );
        assert!(http_base_from_ws("").is_err());
    }

    #[test]
    fn query_error_describes_permission_tier_and_already_resolved() {
        let perm = QueryError::from_json(
            403,
            &serde_json::json!({
                "code": "PERM_DENIED_ROLE",
                "message": "角色不允许授权配置",
                "level": "role"
            }),
        );
        let desc = perm.describe();
        assert!(desc.contains("角色权限"), "{desc}");
        assert!(desc.contains("PERM_DENIED_ROLE"), "{desc}");

        let dup = QueryError::from_json(
            409,
            &serde_json::json!({
                "code": "CONFLICT_ALREADY_RESOLVED",
                "message": "该冲突条目已经裁决过",
                "resolved_by": "u4",
                "resolution": "采用甲方"
            }),
        );
        let dup_desc = dup.describe();
        assert!(dup_desc.contains("u4"), "{dup_desc}");
        assert!(dup_desc.contains("采用甲方"), "{dup_desc}");
    }
}

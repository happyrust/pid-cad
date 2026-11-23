//! Collaboration server library surface.
//!
//! `main.rs` only parses the CLI and starts the runtime; everything testable
//! lives here so integration tests can build the same router the binary serves.

pub mod access;
pub mod auth;
pub mod bus;
pub mod http;
pub mod metrics;
pub mod ops;
pub mod perm;
pub mod proxy;
pub mod store;
pub mod tls;
pub mod ws;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ocs_collab::{OccupancyTable, ServerMessage, SessionConfig, SessionRegistry};

pub use bus::Bus;
pub use store::Store;

/// Default cap for editing sessions on one drawing and on the server (#98).
/// The single-project figure in `docs/pid-platform/architecture/non-functional.md`.
pub const DEFAULT_MAX_EDITING_SESSIONS: usize = 20;

/// The server's shared state.
pub struct AppState {
    /// `None` when the database could not be opened; `GET /health` then
    /// reports 503 instead of pretending everything is fine.
    pub store: Option<Arc<Store>>,
    pub db_error: Option<String>,
    /// Set by the binary when `--db` could not be opened: every `hello` is
    /// then answered `STORE_UNAVAILABLE` and its connection closed, since
    /// nothing a session did would be kept (`SPEC-collab-offline-sync.md`
    /// §6). A state built in code without a store — the in-memory mode the
    /// tests use — leaves it unset and takes sessions as before.
    pub store_unavailable: bool,
    pub db_path: String,
    pub bus: Arc<Bus>,
    pub session_config: SessionConfig,
    /// Live session registry — the in-memory authority mirrored into the
    /// `sessions` table. Behind a tokio mutex because handlers await while
    /// deciding.
    pub sessions: Arc<tokio::sync::Mutex<SessionRegistry>>,
    /// Live tag-occupancy table — the in-memory authority mirrored into the
    /// `claims` table.
    pub occupancy: Arc<tokio::sync::Mutex<OccupancyTable>>,
    /// Per-drawing revision counter used only when `store` is `None`. With a
    /// database, `drawings.revision` is authoritative; without one, ops still
    /// need a monotonically increasing revision for `STALE_BASE_REVISION`
    /// checking to keep working.
    pub revisions: Arc<Mutex<HashMap<String, i64>>>,
    /// Per-session unicast channel, used for messages that must not reach the
    /// rest of the drawing room (targeted errors).
    pub direct: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<ServerMessage>>>>,
    /// What the server knows of each drawing — the value every field was last
    /// given, rebuilt from the op log — and the ops it is holding inside the
    /// tie window. One std mutex: an op is judged, committed and recorded
    /// under it with no await in between.
    pub drawings: Arc<Mutex<HashMap<String, ops::DrawingOps>>>,
    /// For each drawing that is 只读: the session that held the edit right
    /// when it went 只读, and goes on writing. Absent for a drawing nobody
    /// was editing at that moment, and for every drawing that is not 只读.
    /// In memory only: sessions do not outlive the process either.
    pub holders: Arc<Mutex<HashMap<String, String>>>,
    /// How long an op made against the current revision is held before it
    /// commits, so that another made against the same revision can be seen
    /// to tie with it (`--tie-window-ms`). Zero — the default — holds
    /// nothing: every op commits on arrival and no two can tie.
    pub tie_window: Duration,
    /// The repository's own id (`meta.repo_id`). A drawing file keeps a
    /// copy so a `hello` can prove it is coming back to the repository it
    /// aligned against (`REQ-COLL-015`). With a database it is the stored
    /// id — stable across restarts until a start with `--new-repo-id`
    /// replaces it; without one it is generated fresh each start, so an
    /// in-memory run never stands in for a previous one.
    pub repo_id: String,
    /// Minimum wire protocol version accepted by this server.
    pub min_protocol_version: u32,
    /// Current wire protocol version spoken by this server.
    pub protocol_version: u32,
    /// When `false` (default, #67), unregistered users get `UNKNOWN_USER` and
    /// unregistered drawings get `UNKNOWN_DRAWING`. Pass `--open-mode` to
    /// enable the legacy open demo/test behavior.
    pub open_mode: bool,
    /// When `true` (#68), every session join and privileged HTTP mutation must
    /// present valid credentials verified against `users.credential_hash`.
    pub require_auth: bool,
    /// Whether TLS is active on the server listener (#69).
    pub tls_enabled: bool,
    /// Runtime metrics counters (#39).
    pub metrics: Arc<metrics::Metrics>,
    /// Structured single-line JSON logger with optional file sink (#39).
    pub logger: Arc<metrics::StructuredLogger>,
    /// Optional directory where periodic/on-demand online backups are written (#105).
    pub backup_dir: Option<std::path::PathBuf>,
    /// Active editing sessions allowed on one drawing. Default 20, the
    /// single-project figure in `non-functional.md` (#38, #98).
    pub max_editing_sessions_per_drawing: usize,
    /// Active editing sessions allowed on this server. Same default.
    pub max_editing_sessions: usize,
    /// Write a snapshot of what the server knows of a drawing every this
    /// many applied ops, then drop the ops the snapshot already contains
    /// (#103). 20 is an unmeasured default; #38 did not report a size.
    pub snapshot_every: i64,
    pub version: &'static str,
}

impl AppState {
    pub fn new(
        store: Option<Store>,
        db_error: Option<String>,
        db_path: impl Into<String>,
        session_config: SessionConfig,
    ) -> AppState {
        let repo_id = store
            .as_ref()
            .and_then(|store| store.repo_id().ok())
            .unwrap_or_else(ocs_collab::ids::new_repo_id);
        AppState {
            store: store.map(Arc::new),
            db_error,
            store_unavailable: false,
            db_path: db_path.into(),
            bus: Arc::new(Bus::default()),
            session_config,
            sessions: Arc::new(tokio::sync::Mutex::new(SessionRegistry::new(session_config))),
            occupancy: Arc::new(tokio::sync::Mutex::new(OccupancyTable::new())),
            revisions: Arc::new(Mutex::new(HashMap::new())),
            direct: Arc::new(Mutex::new(HashMap::new())),
            drawings: Arc::new(Mutex::new(HashMap::new())),
            holders: Arc::new(Mutex::new(HashMap::new())),
            tie_window: Duration::ZERO,
            repo_id,
            min_protocol_version: ocs_collab::protocol::MIN_PROTOCOL_VERSION,
            protocol_version: ocs_collab::protocol::PROTOCOL_VERSION,
            open_mode: false,
            require_auth: false,
            tls_enabled: false,
            metrics: Arc::new(metrics::Metrics::default()),
            logger: Arc::new(metrics::StructuredLogger::default()),
            backup_dir: None,
            max_editing_sessions_per_drawing: DEFAULT_MAX_EDITING_SESSIONS,
            max_editing_sessions: DEFAULT_MAX_EDITING_SESSIONS,
            snapshot_every: 20,
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// `--max-sessions-per-drawing` and `--max-sessions` (#98).
    pub fn with_session_caps(mut self, per_drawing: usize, server: usize) -> Self {
        self.max_editing_sessions_per_drawing = per_drawing.max(1);
        self.max_editing_sessions = server.max(1);
        self
    }

    /// Builder helper to explicitly enable or disable `--open-mode` (#67).
    pub fn with_open_mode(mut self, open_mode: bool) -> Self {
        self.open_mode = open_mode;
        self
    }

    /// Builder helper to explicitly enable or disable `--require-auth` (#68).
    pub fn with_require_auth(mut self, require_auth: bool) -> Self {
        self.require_auth = require_auth;
        self
    }

    /// Whether the database answered a trivial query just now.
    pub fn db_healthy(&self) -> bool {
        self.store.as_ref().is_some_and(|store| store.health_check())
    }

    /// Registered display name of `user_id` from `users`, or empty when not
    /// registered (or no database is attached). Never falls back to `user_id`
    /// so callers can render plain `user_id` without empty parentheses (#27).
    pub fn display_name_of(&self, user_id: &str) -> String {
        self.store
            .as_ref()
            .and_then(|store| store.user_display_name(user_id).ok().flatten())
            .map(|name| name.trim().to_owned())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_storeless_state_reports_an_unhealthy_database() {
        let state = AppState::new(None, Some("open failed".into()), "bad.db", SessionConfig::default());
        assert!(!state.db_healthy());
        assert_eq!(state.db_error.as_deref(), Some("open failed"));
    }

    #[test]
    fn a_memory_store_reports_a_healthy_database() {
        let store = Store::open_in_memory().expect("memory store");
        let state = AppState::new(Some(store), None, ":memory:", SessionConfig::default());
        assert!(state.db_healthy());
        assert_eq!(state.session_config.heartbeat_timeout, 90);
        assert_eq!(state.session_config.claim_ttl, 300);
    }
}

//! Collaboration server library surface.
//!
//! `main.rs` only parses the CLI and starts the runtime; everything testable
//! lives here so integration tests can build the same router the binary serves.

pub mod access;
pub mod bus;
pub mod http;
pub mod ops;
pub mod perm;
pub mod store;
pub mod ws;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ocs_collab::{OccupancyTable, ServerMessage, SessionConfig, SessionRegistry};

pub use bus::Bus;
pub use store::Store;

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
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Whether the database answered a trivial query just now.
    pub fn db_healthy(&self) -> bool {
        self.store.as_ref().is_some_and(|store| store.health_check())
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

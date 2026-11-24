//! Session lifecycle state machine.
//!
//! A session becomes `Stale` once `heartbeat_timeout` seconds pass without a
//! heartbeat, and a later session on the same drawing and mode takes it over.
//! Disconnecting the socket does *not* end a session: only the heartbeat
//! timeout does (appendix A.2).

use serde::{Deserialize, Serialize};

use crate::protocol::codes;

/// Tunables the server takes from its CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Seconds without a heartbeat before a session is stale.
    pub heartbeat_timeout: i64,
    /// Seconds a tag claim lives.
    pub claim_ttl: i64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            heartbeat_timeout: 90,
            claim_ttl: 300,
        }
    }
}

/// Where a session sits in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    #[serde(rename = "活跃")]
    Active,
    #[serde(rename = "失效")]
    Stale,
    #[serde(rename = "已接管")]
    TakenOver,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Active => "活跃",
            SessionStatus::Stale => "失效",
            SessionStatus::TakenOver => "已接管",
        }
    }
}

/// One session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub drawing_id: String,
    pub user_id: String,
    pub display_name: String,
    pub mode: String,
    pub workspace: String,
    pub status: SessionStatus,
    pub base_revision: i64,
    pub last_heartbeat: i64,
    pub created_at: i64,
    /// Unix timestamp when the underlying socket dropped abruptly without an
    /// explicit close frame (#37). `None` while connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnected_at: Option<i64>,
    /// Built-in library fingerprints this session joined with (#97).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libraries: Option<crate::protocol::Libraries>,
    /// Which of those differ from an editing session already on the drawing.
    /// A non-empty list means this session joined read-only for that reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library_mismatch: Vec<String>,
}

impl Session {
    /// Whether this session may still be used for reads and writes.
    pub fn is_active(&self) -> bool {
        self.status == SessionStatus::Active
    }

    /// Whether the session is an editing session (as opposed to 只读).
    pub fn is_editing(&self) -> bool {
        self.mode != "只读"
    }
}

/// Result of opening a session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum OpenOutcome {
    /// Session created; `taken_over` lists sessions it displaced.
    Opened {
        session: Session,
        taken_over: Vec<String>,
    },
    /// Refused: a live single-editor session already owns this drawing.
    Rejected { code: String, holder: String },
}

/// The fields needed to open a session. Grouped into a struct so the call site
/// names what each string means instead of relying on positional order.
#[derive(Debug, Clone, Copy)]
pub struct OpenRequest<'a> {
    pub session_id: &'a str,
    pub drawing_id: &'a str,
    pub user_id: &'a str,
    pub display_name: &'a str,
    pub mode: &'a str,
    pub workspace: &'a str,
    pub base_revision: i64,
    pub now: i64,
}

/// The in-memory session registry. The server mirrors it into SQLite.
#[derive(Debug, Clone)]
pub struct SessionRegistry {
    sessions: Vec<Session>,
    config: SessionConfig,
}

impl SessionRegistry {
    pub fn new(config: SessionConfig) -> Self {
        SessionRegistry {
            sessions: Vec::new(),
            config,
        }
    }

    pub fn config(&self) -> SessionConfig {
        self.config
    }

    pub fn set_config(&mut self, config: SessionConfig) {
        self.config = config;
    }

    pub fn all(&self) -> &[Session] {
        &self.sessions
    }

    pub fn get(&self, session_id: &str) -> Option<&Session> {
        self.sessions
            .iter()
            .find(|session| session.session_id == session_id)
    }

    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions
            .iter_mut()
            .find(|session| session.session_id == session_id)
    }

    /// Peers currently active on one drawing.
    pub fn active_peers(&self, drawing_id: &str) -> Vec<Session> {
        self.sessions
            .iter()
            .filter(|session| session.drawing_id == drawing_id && session.is_active())
            .cloned()
            .collect()
    }

    /// Opens a session, applying the takeover and single-editor rules.
    pub fn open(&mut self, request: OpenRequest<'_>) -> OpenOutcome {
        let OpenRequest {
            session_id,
            drawing_id,
            user_id,
            display_name,
            mode,
            workspace,
            base_revision,
            now,
        } = request;

        self.sweep(now);

        let same_target: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                session.drawing_id == drawing_id && session.mode == mode && session.is_active()
            })
            .map(|(index, _)| index)
            .collect();

        // 单人编辑 allows exactly one live editing session (appendix A.3).
        if mode == "单人编辑" {
            if let Some(&index) = same_target.first() {
                let holder = self.sessions[index].session_id.clone();
                return OpenOutcome::Rejected {
                    code: codes::PERM_DENIED_STATE.to_owned(),
                    holder,
                };
            }
        }

        // Sessions on the same drawing and mode that already went stale are
        // superseded by this one.
        let mut taken_over = Vec::new();
        for session in self.sessions.iter_mut() {
            if session.drawing_id == drawing_id
                && session.mode == mode
                && session.status == SessionStatus::Stale
            {
                session.status = SessionStatus::TakenOver;
                taken_over.push(session.session_id.clone());
            }
        }

        let session = Session {
            session_id: session_id.to_owned(),
            drawing_id: drawing_id.to_owned(),
            user_id: user_id.to_owned(),
            display_name: display_name.to_owned(),
            mode: mode.to_owned(),
            workspace: workspace.to_owned(),
            status: SessionStatus::Active,
            base_revision,
            last_heartbeat: now,
            created_at: now,
            disconnected_at: None,
            libraries: None,
            library_mismatch: Vec::new(),
        };
        self.sessions.push(session.clone());
        OpenOutcome::Opened {
            session,
            taken_over,
        }
    }

    /// Marks an active session as network-disconnected (#37) while keeping it
    /// active until `heartbeat_timeout`. Returns `(session, hold_until)`.
    pub fn mark_disconnected(&mut self, session_id: &str, now: i64) -> Option<(Session, i64)> {
        let timeout = self.config.heartbeat_timeout;
        let session = self.get_mut(session_id)?;
        if session.status != SessionStatus::Active {
            return None;
        }
        session.disconnected_at = Some(now);
        let hold_until = session.last_heartbeat + timeout;
        Some((session.clone(), hold_until))
    }

    /// Resumes an existing session belonging to the same `(user_id, drawing_id)`
    /// on reconnect (#36): marks the previous session `TakenOver` immediately so
    /// the caller can transfer its claims and open the replacement session.
    pub fn resume(
        &mut self,
        old_session_id: &str,
        user_id: &str,
        drawing_id: &str,
    ) -> Option<Session> {
        let session = self.get_mut(old_session_id)?;
        if session.user_id != user_id
            || session.drawing_id != drawing_id
            || session.status == SessionStatus::TakenOver
        {
            return None;
        }
        session.status = SessionStatus::TakenOver;
        Some(session.clone())
    }

    /// Records a heartbeat. Returns the session when it is still usable.
    pub fn heartbeat(&mut self, session_id: &str, now: i64) -> Result<&Session, String> {
        match self.get_mut(session_id) {
            None => Err(codes::SESSION_UNKNOWN.to_owned()),
            Some(session) => match session.status {
                SessionStatus::Active => {
                    session.last_heartbeat = now;
                    session.disconnected_at = None;
                    Ok(session)
                }
                SessionStatus::Stale => Err(codes::SESSION_EXPIRED.to_owned()),
                SessionStatus::TakenOver => Err(codes::SESSION_TAKEN_OVER.to_owned()),
            },
        }
    }

    /// Moves every live session on `drawing_id` to `revision`, returning how
    /// many moved.
    ///
    /// The sessions of a drawing share one baseline (`architecture.md` §3.3):
    /// an applied op is broadcast to the whole room, so the moment it lands
    /// every live session on the drawing is at its revision. A session that
    /// already went stale or was taken over stays where it stopped.
    pub fn advance_base_revision(&mut self, drawing_id: &str, revision: i64) -> usize {
        let mut moved = 0;
        for session in self.sessions.iter_mut() {
            if session.drawing_id == drawing_id && session.is_active() {
                session.base_revision = revision;
                moved += 1;
            }
        }
        moved
    }

    /// Marks every active session whose heartbeat is older than the timeout as
    /// stale, returning the affected session ids.
    pub fn sweep(&mut self, now: i64) -> Vec<String> {
        let timeout = self.config.heartbeat_timeout;
        let mut stale = Vec::new();
        for session in self.sessions.iter_mut() {
            if session.status == SessionStatus::Active && now - session.last_heartbeat > timeout {
                session.status = SessionStatus::Stale;
                stale.push(session.session_id.clone());
            }
        }
        stale
    }

    /// Marks one live session stale because its client said goodbye.
    ///
    /// A `Close` frame is a deliberate exit, unlike a socket that merely dies
    /// (appendix A.2 keeps that session alive for the heartbeat timeout), so
    /// the caller may retire it at once and tell the room. Returns the retired
    /// session only when it actually transitioned: a session that was already
    /// stale has had its `peer_left` broadcast, and repeating it would shrink
    /// the roster twice.
    pub fn retire(&mut self, session_id: &str) -> Option<Session> {
        let session = self.get_mut(session_id)?;
        if session.status != SessionStatus::Active {
            return None;
        }
        session.status = SessionStatus::Stale;
        Some(session.clone())
    }

    /// Drops a session outright (used on restart or explicit close).
    pub fn remove(&mut self, session_id: &str) -> Option<Session> {
        match self.sessions.iter().position(|s| s.session_id == session_id) {
            Some(index) => Some(self.sessions.remove(index)),
            None => None,
        }
    }

    /// Clears the registry (server restart semantics: old ids are unknown).
    pub fn clear(&mut self) {
        self.sessions.clear();
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> SessionRegistry {
        SessionRegistry::new(SessionConfig::default())
    }

    fn open(registry: &mut SessionRegistry, id: &str, mode: &str, now: i64) -> OpenOutcome {
        registry.open(OpenRequest {
            session_id: id,
            drawing_id: "PID-1101",
            user_id: "u1",
            display_name: "工艺甲",
            mode,
            workspace: "plant-a",
            base_revision: 0,
            now,
        })
    }

    fn open_as(
        registry: &mut SessionRegistry,
        id: &str,
        user_id: &str,
        mode: &str,
        now: i64,
    ) -> OpenOutcome {
        registry.open(OpenRequest {
            session_id: id,
            drawing_id: "PID-1101",
            user_id,
            display_name: "工艺乙",
            mode,
            workspace: "",
            base_revision: 0,
            now,
        })
    }

    #[test]
    fn opening_a_session_starts_it_active() {
        let mut registry = registry();
        match open(&mut registry, "s-1", "协同编辑", 1_000) {
            OpenOutcome::Opened { session, taken_over } => {
                assert_eq!(session.status, SessionStatus::Active);
                assert!(taken_over.is_empty());
                assert_eq!(session.base_revision, 0);
                assert_eq!(session.workspace, "plant-a");
            }
            other => panic!("expected opened, got {other:?}"),
        }
    }

    #[test]
    fn heartbeat_within_the_timeout_keeps_the_session_active() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        for step in 1..=3 {
            let now = 1_000 + step * 15;
            assert!(registry.heartbeat("s-1", now).is_ok());
        }
        assert!(registry.sweep(1_045).is_empty());
        assert_eq!(
            registry.get("s-1").map(|s| s.status),
            Some(SessionStatus::Active)
        );
    }

    #[test]
    fn heartbeat_after_the_timeout_marks_the_session_stale() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        let stale = registry.sweep(1_091);
        assert_eq!(stale, vec!["s-1".to_owned()]);
        assert_eq!(
            registry.get("s-1").map(|s| s.status),
            Some(SessionStatus::Stale)
        );
    }

    #[test]
    fn heartbeat_on_a_stale_session_reports_expired() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        registry.sweep(1_200);
        assert_eq!(
            registry.heartbeat("s-1", 1_201),
            Err(codes::SESSION_EXPIRED.to_owned())
        );
    }

    #[test]
    fn heartbeat_on_an_unknown_session_reports_unknown() {
        let mut registry = registry();
        assert_eq!(
            registry.heartbeat("s-missing", 1_000),
            Err(codes::SESSION_UNKNOWN.to_owned())
        );
    }

    #[test]
    fn same_drawing_same_mode_takes_over_a_stale_session() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        registry.sweep(1_200);
        match open(&mut registry, "s-2", "协同编辑", 1_201) {
            OpenOutcome::Opened { session, taken_over } => {
                assert_eq!(session.session_id, "s-2");
                assert_eq!(taken_over, vec!["s-1".to_owned()]);
            }
            other => panic!("expected opened, got {other:?}"),
        }
        assert_eq!(
            registry.get("s-1").map(|s| s.status),
            Some(SessionStatus::TakenOver)
        );
    }

    #[test]
    fn collaborative_mode_allows_two_live_sessions_on_one_drawing() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        let outcome = open_as(&mut registry, "s-2", "u2", "协同编辑", 1_001);
        match outcome {
            OpenOutcome::Opened { taken_over, .. } => {
                assert!(taken_over.is_empty(), "a live peer is not taken over")
            }
            other => panic!("expected opened, got {other:?}"),
        }
        assert_eq!(registry.active_peers("PID-1101").len(), 2);
    }

    #[test]
    fn single_editor_rejects_a_second_live_session_with_the_holder() {
        let mut registry = registry();
        open(&mut registry, "s-1", "单人编辑", 1_000);
        let outcome = open_as(&mut registry, "s-2", "u2", "单人编辑", 1_005);
        assert_eq!(
            outcome,
            OpenOutcome::Rejected {
                code: codes::PERM_DENIED_STATE.to_owned(),
                holder: "s-1".to_owned(),
            }
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn single_editor_accepts_a_session_after_the_first_goes_stale() {
        let mut registry = registry();
        open(&mut registry, "s-1", "单人编辑", 1_000);
        registry.sweep(1_200);
        match open_as(&mut registry, "s-2", "u2", "单人编辑", 1_201) {
            OpenOutcome::Opened { taken_over, .. } => assert_eq!(taken_over, vec!["s-1".to_owned()]),
            other => panic!("expected opened, got {other:?}"),
        }
    }

    #[test]
    fn the_same_user_twice_yields_two_independent_sessions() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        open(&mut registry, "s-2", "协同编辑", 1_001);
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("s-1").map(|s| s.user_id.as_str()), Some("u1"));
        assert_eq!(registry.get("s-2").map(|s| s.user_id.as_str()), Some("u1"));
    }

    #[test]
    fn a_close_frame_retires_a_live_session() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        let retired = registry.retire("s-1").expect("a live session retires");
        assert_eq!(retired.status, SessionStatus::Stale);
        assert_eq!(
            registry.get("s-1").map(|s| s.status),
            Some(SessionStatus::Stale)
        );
        assert!(registry.active_peers("PID-1101").is_empty());
    }

    #[test]
    fn retiring_is_idempotent_so_peer_left_is_not_repeated() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        assert!(registry.retire("s-1").is_some());
        assert!(
            registry.retire("s-1").is_none(),
            "an already stale session must not retire twice"
        );
    }

    #[test]
    fn retiring_an_unknown_or_swept_session_yields_nothing() {
        let mut registry = registry();
        assert!(registry.retire("s-missing").is_none());
        open(&mut registry, "s-1", "协同编辑", 1_000);
        registry.sweep(1_100);
        assert!(
            registry.retire("s-1").is_none(),
            "the sweep already announced this departure"
        );
    }

    #[test]
    fn a_heartbeat_timeout_from_config_drives_the_sweep() {
        let mut registry = SessionRegistry::new(SessionConfig {
            heartbeat_timeout: 5,
            claim_ttl: 5,
        });
        open(&mut registry, "s-1", "协同编辑", 1_000);
        assert!(registry.sweep(1_005).is_empty(), "5s is not yet over 5s");
        assert_eq!(registry.sweep(1_006), vec!["s-1".to_owned()]);
    }

    #[test]
    fn default_config_matches_the_architecture_numbers() {
        let config = SessionConfig::default();
        assert_eq!(config.heartbeat_timeout, 90);
        assert_eq!(config.claim_ttl, 300);
    }

    #[test]
    fn active_peers_ignore_stale_and_taken_over_sessions() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        open(&mut registry, "s-2", "协同编辑", 1_000);
        registry.sweep(1_200);
        assert!(registry.active_peers("PID-1101").is_empty());
    }

    /// An applied op moves the baseline of the drawing's live sessions and of
    /// nothing else: not another drawing's, not one that already went stale.
    #[test]
    fn advancing_the_baseline_moves_the_live_sessions_of_that_drawing_only() {
        let mut registry = registry();
        open(&mut registry, "s-stale", "协同编辑", 1_000);
        registry.sweep(1_200);
        open(&mut registry, "s-1", "协同编辑", 1_200);
        open(&mut registry, "s-2", "只读", 1_200);
        registry.open(OpenRequest {
            session_id: "s-elsewhere",
            drawing_id: "PID-2202",
            user_id: "u1",
            display_name: "工艺甲",
            mode: "协同编辑",
            workspace: "plant-a",
            base_revision: 0,
            now: 1_200,
        });

        assert_eq!(registry.advance_base_revision("PID-1101", 3), 2);

        let base = |id: &str| registry.get(id).expect("registered").base_revision;
        assert_eq!(base("s-1"), 3);
        assert_eq!(base("s-2"), 3, "a read-only session follows the room too");
        assert_eq!(base("s-elsewhere"), 0);
        assert_eq!(base("s-stale"), 0);
    }

    #[test]
    fn clearing_the_registry_forgets_every_session() {
        let mut registry = registry();
        open(&mut registry, "s-1", "协同编辑", 1_000);
        registry.clear();
        assert!(registry.is_empty());
        assert_eq!(
            registry.heartbeat("s-1", 1_001),
            Err(codes::SESSION_UNKNOWN.to_owned())
        );
    }

    #[test]
    fn status_literals_match_the_database_values() {
        assert_eq!(SessionStatus::Active.as_str(), "活跃");
        assert_eq!(SessionStatus::Stale.as_str(), "失效");
        assert_eq!(SessionStatus::TakenOver.as_str(), "已接管");
    }
}

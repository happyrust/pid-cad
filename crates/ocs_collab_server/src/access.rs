//! Whether a session may write to its drawing.
//!
//! The permission chain (`perm::evaluate`) answers for a user and a drawing;
//! a collaboration session adds two things the chain does not know: a 只读
//! drawing still has one writer — the session that held the edit right when
//! it went 只读 — and a session that joined as a viewer does not write at
//! all.
//!
//! This is the server's side of the read-only gate. A client turns its own
//! writes away when it is told it may not write, but that is a courtesy: a
//! client that never heard, reconnected, or is not this program at all sends
//! its ops regardless, and they are refused here.

use ocs_collab::{DenialLevel, Operation, ServerMessage, Session};

use crate::perm::{self, PermError};
use crate::AppState;

/// Why a session may not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    /// The tier of the permission chain that refused.
    pub level: DenialLevel,
    /// What to tell a person.
    pub reason: String,
    /// The session holding the edit right, when the refusal is about one.
    pub holder: Option<String>,
}

impl Denied {
    /// The `PERM_DENIED_*` code of the tier.
    pub fn code(&self) -> &'static str {
        self.level.code()
    }

    /// The `readonly_downgrade` that tells the session.
    pub fn downgrade(&self) -> ServerMessage {
        ServerMessage::ReadonlyDowngrade {
            reason: self.reason.clone(),
            holder: self.holder.clone(),
            level: Some(self.level.as_str().to_owned()),
            code: Some(self.code().to_owned()),
        }
    }

    /// The `error` that answers a write the session sent all the same.
    pub fn error(&self, what: &str) -> ServerMessage {
        ServerMessage::Error {
            code: self.code().to_owned(),
            message: format!("{what}被拒绝：{}", self.reason),
            level: Some(self.level.as_str().to_owned()),
            holder: self.holder.clone(),
            op_id: None,
            rejected: None,
            actor_user_id: None,
            actor_display_name: None,
        }
    }
}

/// The session holding the edit right of `drawing_id` while it is 只读.
pub fn holder_of(state: &AppState, drawing_id: &str) -> Option<String> {
    state
        .holders
        .lock()
        .expect("holders mutex poisoned")
        .get(drawing_id)
        .cloned()
}

/// The editing session that has been on `drawing_id` longest, among those
/// that can be told anything: live, editing, and with a socket.
///
/// Join order is the registry's own order — not `created_at`, which is in
/// seconds and ties, and not the session id, which is random. A session
/// opened over HTTP has no socket: nobody is behind it to write or to be
/// told it holds anything, so it is never the one.
pub async fn first_editor(state: &AppState, drawing_id: &str) -> Option<String> {
    let registry = state.sessions.lock().await;
    let direct = state.direct.lock().expect("direct mutex poisoned");
    registry
        .all()
        .iter()
        .find(|session| {
            session.drawing_id == drawing_id
                && session.is_active()
                && session.is_editing()
                && direct.contains_key(&session.session_id)
        })
        .map(|session| session.session_id.clone())
}

/// Whether `user_id`'s role edits drawings at all: `None` when the user is
/// not in the `users` table (or there is no database), which is a session
/// the permission tables say nothing about.
pub fn role_edits(state: &AppState, user_id: &str) -> Option<bool> {
    let store = state.store.as_ref()?;
    let (role, _company) = store.user_role(user_id).ok().flatten()?;
    let role = ocs_collab::Role::parse(&role)?;
    Some(ocs_collab::permission::role_allows(
        role,
        Operation::EditDrawing,
    ))
}

/// `None` when `session` may write to its drawing now.
///
/// A session whose user or drawing the permission tables do not know is not
/// governed by them and writes as before: the tables are what an
/// installation fills in to restrict, and an unseeded server restricts
/// nothing.
pub async fn write_access(state: &AppState, session: &Session) -> Option<Denied> {
    let store = state.store.as_ref()?;
    // The live editing sessions on the drawing: the single-editor half of
    // the state tier needs them.
    let editors = crate::http::active_editors_on(state, &session.drawing_id).await;
    let outcome = match perm::evaluate(
        store,
        &session.user_id,
        &session.drawing_id,
        Operation::EditDrawing.as_str(),
        &editors,
    ) {
        Ok(outcome) => outcome,
        Err(PermError::UnknownSubject | PermError::UnknownDrawing) => {
            // Not under the tables. A viewer session still does not write.
            return viewer(session);
        }
        Err(err) => {
            eprintln!(
                "[ocs_collab_server] 判定会话 {} 的写权限失败：{}",
                session.session_id,
                err.message()
            );
            return viewer(session);
        }
    };
    if outcome.decision.allowed {
        return viewer(session);
    }
    let level = outcome.decision.denied_level.unwrap_or(DenialLevel::State);
    // A drawing that was never placed in the PBS tree — a row `hello`
    // created for a drawing nobody seeded — has no node for an ACL to hang
    // on. That is "not under the tables", not "forbidden to everybody".
    if level == DenialLevel::Pbs {
        let placed = store
            .drawing(&session.drawing_id)
            .ok()
            .flatten()
            .is_some_and(|(node_id, _, _)| !node_id.trim().is_empty());
        if !placed {
            return viewer(session);
        }
    }
    let holder = if level != DenialLevel::State {
        None
    } else if outcome.holder.is_some() {
        // 单人编辑, and somebody else is editing too. The chain says that
        // of every one of them; the one that may write is the one that was
        // there first.
        first_editor(state, &session.drawing_id).await
    } else {
        holder_of(state, &session.drawing_id)
    };
    // The drawing's one writer is this session.
    if level == DenialLevel::State && holder.as_deref() == Some(session.session_id.as_str()) {
        return None;
    }
    let reason = match level {
        DenialLevel::Role => "权限不足：该用户的角色不允许编辑图纸".to_owned(),
        DenialLevel::Pbs => "权限不足：该用户在这张图所在的节点上没有编辑权限".to_owned(),
        DenialLevel::State => match &holder {
            Some(holder) => format!("图纸状态不允许编辑，编辑权在会话 {holder}"),
            None => "图纸状态为只读".to_owned(),
        },
    };
    Some(Denied {
        level,
        reason,
        holder,
    })
}

/// A session opened as a viewer does not write, whatever the tables allow
/// its user.
fn viewer(session: &Session) -> Option<Denied> {
    (!session.is_editing()).then(|| Denied {
        level: DenialLevel::Role,
        reason: "只读会话不能修改图纸".to_owned(),
        holder: None,
    })
}

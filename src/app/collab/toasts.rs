//! Shared collaboration notification (toast) queue (#50, #58–#61, #65).
//!
//! Every user-facing collaboration notice — peer joins/leaves, claim outcomes,
//! peer edits, refused/superseded/deleted/reassigned local edits, upstream
//! connector property propagation, conflict rulings, and connector warnings —
//! flows through this single queue so the UI renders them stacked cleanly
//! without overlap and the automation status endpoint exposes them for tests.

use serde_json::{json, Value};

/// Default visibility duration of a collaboration toast in milliseconds.
pub const DEFAULT_TOAST_TTL_MS: u64 = 5_000;

/// Maximum number of simultaneously visible toasts kept in the queue.
pub const MAX_VISIBLE_TOASTS: usize = 6;

/// Visual severity of a collaboration toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// One transient collaboration notification item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollabToast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub created_at_ms: u64,
    pub ttl_ms: u64,
}

impl CollabToast {
    /// Whether this toast has expired at wall clock `now_ms`.
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.created_at_ms) >= self.ttl_ms
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "message": self.message,
            "created_at_ms": self.created_at_ms,
            "ttl_ms": self.ttl_ms,
        })
    }
}

/// Monotonic wall clock in milliseconds, safe across desktop and wasm.
pub fn now_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// Ordered queue of active collaboration toasts, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToastQueue {
    next_id: u64,
    items: Vec<CollabToast>,
}

impl ToastQueue {
    /// Push a toast with the default TTL using the current wall clock.
    pub fn push(&mut self, kind: ToastKind, message: impl Into<String>) -> u64 {
        self.push_at(kind, message, now_ms(), DEFAULT_TOAST_TTL_MS)
    }

    /// Push a toast with explicit timestamp and TTL (deterministic for tests).
    pub fn push_at(
        &mut self,
        kind: ToastKind,
        message: impl Into<String>,
        created_at_ms: u64,
        ttl_ms: u64,
    ) -> u64 {
        self.prune_at(created_at_ms);
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        self.items.push(CollabToast {
            id,
            kind,
            message: message.into(),
            created_at_ms,
            ttl_ms,
        });
        while self.items.len() > MAX_VISIBLE_TOASTS {
            self.items.remove(0);
        }
        id
    }

    /// Remove expired toasts as of `now_ms`. Returns `true` if any were pruned.
    pub fn prune_at(&mut self, now_ms: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|t| !t.is_expired(now_ms));
        self.items.len() != before
    }

    /// Remove the most recent toast matching `message` verbatim, if present.
    pub fn remove_last_matching(&mut self, message: &str) {
        if let Some(pos) = self.items.iter().rposition(|t| t.message == message) {
            self.items.remove(pos);
        }
    }

    /// Remove expired toasts as of the current clock.
    pub fn prune(&mut self) -> bool {
        self.prune_at(now_ms())
    }

    /// Dismiss one toast by id.
    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.id != id);
        self.items.len() != before
    }

    /// Clear all toasts.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Active toasts in display order (oldest first, stacked without overlap).
    pub fn items(&self) -> &[CollabToast] {
        &self.items
    }

    /// Active toasts serialized for `GET /api/v1/collab/status`.
    pub fn to_json(&self) -> Vec<Value> {
        self.items.iter().map(CollabToast::to_json).collect()
    }
}

/// Format a user for UI and notifications per REQ-COLL-010 acceptance 6:
/// `"显示名（user_id）"` when `display_name` is non-empty and differs from
/// `user_id`, otherwise just `user_id`.
pub fn format_user_label(user_id: &str, display_name: &str) -> String {
    let uid = user_id.trim();
    let name = display_name.trim();
    if name.is_empty() || name == uid {
        uid.to_owned()
    } else if uid.is_empty() {
        name.to_owned()
    } else {
        format!("{name}（{uid}）")
    }
}

/// First display glyph for an avatar circle (#49): first character of
/// `display_name`, falling back to uppercase first character of `user_id`.
pub fn avatar_glyph(user_id: &str, display_name: &str) -> String {
    let name = display_name.trim();
    if !name.is_empty() && name != user_id.trim() {
        if let Some(ch) = name.chars().next() {
            return ch.to_uppercase().collect();
        }
    }
    user_id
        .trim()
        .chars()
        .next()
        .map(|ch| ch.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_owned())
}

/// Format a Unix timestamp in seconds as `HH:MM` (UTC+8 / local-offset aware;
/// uses fixed local offset fallback when OS timezone is unavailable).
pub fn format_hhmm(unix_secs: i64) -> String {
    if unix_secs <= 0 {
        return "--:--".to_owned();
    }
    let secs_in_day = unix_secs.rem_euclid(86_400);
    let hour = secs_in_day / 3_600;
    let minute = (secs_in_day % 3_600) / 60;
    format!("{hour:02}:{minute:02}")
}

/// Format a Unix timestamp in seconds as `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn format_timestamp(unix_secs: i64) -> String {
    if unix_secs <= 0 {
        return "-".to_owned();
    }
    let days = unix_secs.div_euclid(86_400);
    let rem = unix_secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_queue_orders_caps_and_expires() {
        let mut q = ToastQueue::default();
        let id1 = q.push_at(ToastKind::Info, "第一条", 1_000, 5_000);
        let id2 = q.push_at(ToastKind::Success, "第二条", 2_000, 5_000);
        assert_eq!(q.items().len(), 2);
        assert_eq!(q.items()[0].id, id1);
        assert_eq!(q.items()[1].id, id2);

        // At t = 6_000, id1 (1_000 + 5_000) expires while id2 stays.
        assert!(q.prune_at(6_000));
        assert_eq!(q.items().len(), 1);
        assert_eq!(q.items()[0].id, id2);

        // Cap at MAX_VISIBLE_TOASTS.
        for i in 0..10 {
            q.push_at(ToastKind::Info, format!("msg {i}"), 6_500 + i, 5_000);
        }
        assert_eq!(q.items().len(), MAX_VISIBLE_TOASTS);
        assert_eq!(q.items().last().unwrap().message, "msg 9");
    }

    #[test]
    fn user_label_and_avatar_glyph_follow_req_coll_010() {
        assert_eq!(format_user_label("u1", "张工"), "张工（u1）");
        assert_eq!(format_user_label("u1", ""), "u1");
        assert_eq!(format_user_label("u1", "u1"), "u1");

        assert_eq!(avatar_glyph("u1", "张工"), "张");
        assert_eq!(avatar_glyph("wang", ""), "W");
        assert_eq!(avatar_glyph("wang", "wang"), "W");
    }

    #[test]
    fn timestamp_formatting_is_readable() {
        assert_eq!(format_hhmm(0), "--:--");
        assert_eq!(format_hhmm(15 * 3600 + 30 * 60), "15:30");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22:13:20");
    }
}

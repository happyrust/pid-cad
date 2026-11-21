//! Identifier and timestamp helpers shared by both sides.
//!
//! These are the only places in this crate that look at the wall clock or
//! generate randomness, and they are deliberately thin: the state machines in
//! `session`, `occupancy` and `conflict` still take `now` as a parameter, so
//! they stay deterministic under test. Nothing here touches the network or the
//! filesystem — `uuid` and `chrono` are the only crates involved.

use uuid::Uuid;

/// Current unix time in whole seconds. Every `at` / `expires_at` /
/// `last_heartbeat` / `created_at` in the protocol uses this unit.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A fresh session id, prefixed so log lines and database rows are greppable.
pub fn new_session_id() -> String {
    format!("s-{}", Uuid::new_v4())
}

/// A fresh conflict id.
pub fn new_conflict_id() -> String {
    format!("c-{}", Uuid::new_v4())
}

/// A fresh claim id, for callers that need to name one without a tag.
pub fn new_claim_id() -> String {
    format!("cl-{}", Uuid::new_v4())
}

/// A fresh repository id: the identity of one server database, recorded in
/// `meta.repo_id` once and read back by every later start (`REQ-COLL-015`).
/// A drawing file stores it so a `hello` can say which repository its
/// `since` belongs to.
pub fn new_repo_id() -> String {
    format!("r-{}", Uuid::new_v4())
}

/// A fresh run id: drawn once each time the desktop client starts, it names
/// the ops of that run ([`op_id`]). It is there before any session is, so a
/// step made offline is named as surely as one made online, and no other
/// run — the same person's next, or anybody's — names a step alike.
pub fn new_run_id() -> String {
    format!("r-{}", Uuid::new_v4())
}

/// The id of op `n` of run `run_id`: `op-<run id>-<n>`, counted from 1 in
/// each run. Opaque to both ends: the server echoes it, and only the client
/// that made it reads it, to know its own op when the room answers.
pub fn op_id(run_id: &str, n: u64) -> String {
    format!("op-{run_id}-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_unix_is_a_plausible_second_timestamp() {
        // Later than 2024-01-01 and earlier than 2100: catches a unit mix-up
        // (milliseconds would be ~1000x too large).
        let now = now_unix();
        assert!(now > 1_704_067_200, "got {now}");
        assert!(now < 4_102_444_800, "got {now}");
    }

    #[test]
    fn generated_ids_are_unique_and_prefixed() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert!(a.starts_with("s-"));
        assert!(new_conflict_id().starts_with("c-"));
        assert!(new_claim_id().starts_with("cl-"));
    }

    #[test]
    fn run_ids_are_unique_and_name_the_ops_of_their_run() {
        let a = new_run_id();
        let b = new_run_id();
        assert_ne!(a, b);
        assert!(a.starts_with("r-"), "{a}");
        assert_eq!(op_id(&a, 1), format!("op-{a}-1"));
        assert_ne!(op_id(&a, 1), op_id(&a, 2));
        // Two runs that have counted alike still name their ops apart.
        assert_ne!(op_id(&a, 1), op_id(&b, 1));
    }
}

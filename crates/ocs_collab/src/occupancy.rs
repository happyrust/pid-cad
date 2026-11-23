//! Tag-level occupancy state machine.
//!
//! One tag has at most one owner at a time. Claims carry an absolute expiry
//! (unix seconds); expiry is evaluated lazily against a caller-supplied `now`,
//! which keeps the machine deterministic and testable without a clock.

use serde::{Deserialize, Serialize};

use crate::protocol::release_reasons;

/// One outstanding claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occupancy {
    pub tag: String,
    pub drawing_id: String,
    pub session_id: String,
    pub user_id: String,
    pub expires_at: i64,
}

/// Result of a `claim` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The tag is now owned by the requester.
    Granted(Occupancy),
    /// Somebody else still owns it.
    Denied {
        holder: String,
        expires_at: i64,
    },
}

/// Result of a `release` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The claim was removed.
    Released,
    /// The tag was not owned by this session (or not owned at all).
    NotHeld,
}

/// The in-memory occupancy table. The server mirrors it into SQLite.
#[derive(Debug, Clone, Default)]
pub struct OccupancyTable {
    claims: Vec<Occupancy>,
}

impl OccupancyTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// All live claims (expired ones are pruned first).
    ///
    /// Left as-is on purpose: the pruned rows are dropped silently here. No
    /// caller exists today, and the two paths that *do* matter (`claim` and
    /// `for_drawing`) report what they expired so the socket layer can
    /// broadcast `claim_released`. Anything that starts using `entries` and
    /// needs that broadcast must switch to the tuple-returning shape.
    pub fn entries(&mut self, now: i64) -> &[Occupancy] {
        self.expire(now);
        &self.claims
    }

    /// Claims on one drawing, for the `welcome` payload.
    ///
    /// Returns `(active_on_drawing, expired_anywhere)`. The expired half is
    /// whatever `expire(now)` reclaimed during this call — the caller owes the
    /// room a `claim_released` for each of them, otherwise a listing request
    /// would swallow the expiry notification the sweep would have sent.
    pub fn for_drawing(
        &mut self,
        drawing_id: &str,
        now: i64,
    ) -> (Vec<Occupancy>, Vec<Occupancy>) {
        let expired = self.expire(now);
        let active = self
            .claims
            .iter()
            .filter(|claim| claim.drawing_id == drawing_id)
            .cloned()
            .collect();
        (active, expired)
    }

    pub fn get(&self, tag: &str) -> Option<&Occupancy> {
        self.claims.iter().find(|claim| claim.tag == tag)
    }

    /// The claim that keeps `session_id` from writing to `tag` on
    /// `drawing_id` at `now`: one on that tag of that drawing, held by
    /// another session, that has not run out.
    ///
    /// Reads only. A claim past its time is not removed here — that is
    /// `expire`'s to do, which also says so to the room — it just does not
    /// count. A claim made from another drawing does not count either: the
    /// same tag there is another object.
    pub fn held_by_another(
        &self,
        tag: &str,
        drawing_id: &str,
        session_id: &str,
        now: i64,
    ) -> Option<&Occupancy> {
        self.claims.iter().find(|claim| {
            claim.tag == tag
                && claim.drawing_id == drawing_id
                && claim.session_id != session_id
                && claim.expires_at > now
        })
    }

    /// Requests ownership of `tag` until `now + ttl`.
    ///
    /// An expired claim is discarded first, so a stale owner never blocks a
    /// new request. Re-claiming a tag you already own refreshes the expiry.
    ///
    /// Returns `(outcome, expired)`: the second element is whatever
    /// `expire(now)` reclaimed while serving this request. The caller owes the
    /// room a `claim_released { reason: expired }` for each of them — dropping
    /// them here is exactly how a claim request used to swallow the expiry
    /// notification the sweep would otherwise have sent.
    pub fn claim(
        &mut self,
        tag: &str,
        drawing_id: &str,
        session_id: &str,
        user_id: &str,
        now: i64,
        ttl: i64,
    ) -> (ClaimOutcome, Vec<Occupancy>) {
        let expired = self.expire(now);

        if let Some(existing) = self.claims.iter().find(|claim| claim.tag == tag) {
            if existing.session_id != session_id {
                return (
                    ClaimOutcome::Denied {
                        holder: existing.session_id.clone(),
                        expires_at: existing.expires_at,
                    },
                    expired,
                );
            }
        }

        let claim = Occupancy {
            tag: tag.to_owned(),
            drawing_id: drawing_id.to_owned(),
            session_id: session_id.to_owned(),
            user_id: user_id.to_owned(),
            expires_at: now + ttl,
        };
        match self.claims.iter_mut().find(|claim| claim.tag == tag) {
            Some(slot) => *slot = claim.clone(),
            None => self.claims.push(claim.clone()),
        }
        (ClaimOutcome::Granted(claim), expired)
    }

    /// Drops a claim the session owns.
    pub fn release(&mut self, tag: &str, session_id: &str) -> ReleaseOutcome {
        match self
            .claims
            .iter()
            .position(|claim| claim.tag == tag && claim.session_id == session_id)
        {
            Some(index) => {
                self.claims.remove(index);
                ReleaseOutcome::Released
            }
            None => ReleaseOutcome::NotHeld,
        }
    }

    /// Transfers every non-expired claim owned by `from_session` to
    /// `to_session` (used when a user reconnects and resumes their prior
    /// session, #36). Returns `(transferred, expired)`.
    pub fn transfer_session(
        &mut self,
        from_session: &str,
        to_session: &str,
        now: i64,
    ) -> (Vec<Occupancy>, Vec<Occupancy>) {
        let expired = self.expire(now);
        let mut transferred = Vec::new();
        for claim in &mut self.claims {
            if claim.session_id == from_session {
                claim.session_id = to_session.to_owned();
                transferred.push(claim.clone());
            }
        }
        (transferred, expired)
    }

    /// Drops every claim held by a session (used when it becomes stale).
    pub fn release_session(&mut self, session_id: &str) -> Vec<Occupancy> {
        let mut removed = Vec::new();
        self.claims.retain(|claim| {
            if claim.session_id == session_id {
                removed.push(claim.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    /// Removes expired claims and returns them, so the caller can broadcast
    /// `claim_released` with reason `expired`.
    pub fn expire(&mut self, now: i64) -> Vec<Occupancy> {
        let mut expired = Vec::new();
        self.claims.retain(|claim| {
            if claim.expires_at <= now {
                expired.push(claim.clone());
                false
            } else {
                true
            }
        });
        expired
    }

    /// Replaces the whole table (server restart reload).
    pub fn replace_all(&mut self, claims: Vec<Occupancy>) {
        self.claims = claims;
    }

    pub fn len(&self) -> usize {
        self.claims.len()
    }

    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }
}

/// The `reason` string for an expired claim release.
pub fn expired_reason() -> &'static str {
    release_reasons::EXPIRED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_claim_is_granted_and_visible() {
        let mut table = OccupancyTable::new();
        let (outcome, _) = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        match outcome {
            ClaimOutcome::Granted(claim) => {
                assert_eq!(claim.tag, "V-1001");
                assert_eq!(claim.expires_at, 1_300);
            }
            other => panic!("expected granted, got {other:?}"),
        }
        assert_eq!(table.get("V-1001").map(|c| c.session_id.as_str()), Some("s-1"));
    }

    /// Who a claim keeps out: every other session on that drawing, while it
    /// lasts. Not its own session, not anybody once it has run out — though
    /// nothing has removed it yet — and not a session on another drawing.
    #[test]
    fn a_claim_keeps_other_sessions_of_its_drawing_out_while_it_lasts() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let held = |session: &str, drawing: &str, now: i64| {
            table
                .held_by_another("V-1001", drawing, session, now)
                .map(|claim| claim.session_id.clone())
        };
        assert_eq!(held("s-2", "PID-1101", 1_010), Some("s-1".to_owned()));
        assert_eq!(held("s-1", "PID-1101", 1_010), None, "its own");
        assert_eq!(held("s-2", "PID-1101", 1_299), Some("s-1".to_owned()));
        assert_eq!(held("s-2", "PID-1101", 1_300), None, "run out");
        assert_eq!(held("s-2", "PID-1102", 1_010), None, "another drawing");
        assert_eq!(
            table.held_by_another("V-1002", "PID-1101", "s-2", 1_010),
            None,
            "another tag"
        );
        assert_eq!(table.len(), 1, "reading removes nothing");
    }

    #[test]
    fn second_claimant_is_denied_with_holder_and_expiry() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let (outcome, _) = table.claim("V-1001", "PID-1101", "s-2", "u2", 1_010, 300);
        assert_eq!(
            outcome,
            ClaimOutcome::Denied {
                holder: "s-1".into(),
                expires_at: 1_300,
            }
        );
        assert_eq!(
            table.get("V-1001").map(|c| c.session_id.as_str()),
            Some("s-1"),
            "the original owner must be untouched"
        );
    }

    #[test]
    fn re_claiming_your_own_tag_refreshes_the_expiry() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let (outcome, _) = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_100, 300);
        match outcome {
            ClaimOutcome::Granted(claim) => assert_eq!(claim.expires_at, 1_400),
            other => panic!("expected granted, got {other:?}"),
        }
        assert_eq!(table.len(), 1, "refresh must not duplicate the row");
    }

    #[test]
    fn release_frees_the_tag_for_the_next_claimant() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        assert_eq!(table.release("V-1001", "s-1"), ReleaseOutcome::Released);
        assert!(matches!(
            table.claim("V-1001", "PID-1101", "s-2", "u2", 1_005, 300).0,
            ClaimOutcome::Granted(_)
        ));
    }

    #[test]
    fn release_by_a_non_owner_is_rejected() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        assert_eq!(table.release("V-1001", "s-2"), ReleaseOutcome::NotHeld);
        assert_eq!(
            table.get("V-1001").map(|c| c.session_id.as_str()),
            Some("s-1")
        );
    }

    #[test]
    fn ttl_expiry_releases_the_tag_and_reports_it() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let expired = table.expire(1_300);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].tag, "V-1001");
        assert!(matches!(
            table.claim("V-1001", "PID-1101", "s-2", "u2", 1_301, 300).0,
            ClaimOutcome::Granted(_)
        ));
    }

    #[test]
    fn claim_after_expiry_succeeds_without_an_explicit_sweep() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let (outcome, _) = table.claim("V-1001", "PID-1101", "s-2", "u2", 1_400, 300);
        assert!(
            matches!(outcome, ClaimOutcome::Granted(_)),
            "an expired claim must not block: {outcome:?}"
        );
    }

    #[test]
    fn expiring_a_session_drops_all_of_its_claims() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let _ = table.claim("V-1002", "PID-1101", "s-1", "u1", 1_000, 300);
        let _ = table.claim("V-1003", "PID-1101", "s-2", "u2", 1_000, 300);
        let dropped = table.release_session("s-1");
        assert_eq!(dropped.len(), 2);
        assert_eq!(table.len(), 1);
        assert!(table.get("V-1003").is_some());
    }

    #[test]
    fn claims_are_filtered_by_drawing_for_welcome() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let _ = table.claim("P-2001", "PID-1102", "s-3", "u3", 1_000, 300);
        let (on_drawing, _) = table.for_drawing("PID-1101", 1_010);
        assert_eq!(on_drawing.len(), 1);
        assert_eq!(on_drawing[0].tag, "V-1001");
    }

    #[test]
    fn expired_claims_are_hidden_from_drawing_listings() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        assert!(table.for_drawing("PID-1101", 1_500).0.is_empty());
    }

    #[test]
    fn claim_returns_the_entries_it_expired() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        // 1_400 is past V-1001's 1_300 expiry, so serving the V-1002 request
        // reclaims V-1001; the caller must learn about it to broadcast.
        let (outcome, expired) = table.claim("V-1002", "PID-1101", "s-2", "u2", 1_400, 300);
        assert!(
            matches!(outcome, ClaimOutcome::Granted(_)),
            "the new tag is free: {outcome:?}"
        );
        assert_eq!(expired.len(), 1, "the reclaim must be reported: {expired:?}");
        assert_eq!(expired[0].tag, "V-1001");
        assert_eq!(expired[0].session_id, "s-1");
        assert_eq!(expired[0].expires_at, 1_300);
    }

    #[test]
    fn for_drawing_returns_the_entries_it_expired() {
        let mut table = OccupancyTable::new();
        let _ = table.claim("V-1001", "PID-1101", "s-1", "u1", 1_000, 300);
        let (active, expired) = table.for_drawing("PID-1101", 1_500);
        assert!(active.is_empty(), "expired rows must not be listed: {active:?}");
        assert_eq!(expired.len(), 1, "the reclaim must be reported: {expired:?}");
        assert_eq!(expired[0].tag, "V-1001");
    }

    #[test]
    fn expired_reason_is_the_protocol_literal() {
        assert_eq!(expired_reason(), "expired");
    }
}

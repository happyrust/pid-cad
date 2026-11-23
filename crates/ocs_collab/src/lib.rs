//! Shared collaboration protocol and pure state machines.
//!
//! Everything in this crate is deterministic and free of I/O: the server owns
//! persistence and sockets, the desktop client owns its transport, and both
//! sides agree on the shapes declared here. That keeps the wire format and the
//! permission/occupancy rules in exactly one place.

pub mod conflict;
pub mod ids;
pub mod link;
pub mod occupancy;
pub mod permission;
pub mod protocol;
pub mod session;

pub use conflict::{collides, Change, Deleted, DrawingModel, FieldState, ObjectState, Verdict};
pub use ids::{new_claim_id, new_conflict_id, new_session_id, now_unix};
pub use occupancy::{ClaimOutcome, Occupancy, OccupancyTable, ReleaseOutcome};
pub use permission::{
    decide, decide_without_state, AclEntry, AclNode, AclPermission, DenialLevel, DrawingState,
    Operation, PermissionDecision, PermissionRequest, Role,
};
pub use protocol::{ClientMessage, ServerMessage};
pub use session::{OpenOutcome, OpenRequest, Session, SessionConfig, SessionRegistry, SessionStatus};

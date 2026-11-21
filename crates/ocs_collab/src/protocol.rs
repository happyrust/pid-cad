//! Wire protocol shared by the collaboration server and the desktop client.
//!
//! Every frame is a JSON object with a `type` discriminator (see
//! `architecture.md` §3.1). The server only understands P&ID *semantics*; it
//! never looks inside DWG bytes, so `Op` payloads are carried as opaque JSON.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message sent by a client to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Opens a session. Answered with [`ServerMessage::Welcome`].
    Hello {
        user_id: String,
        drawing_id: String,
        mode: String,
        #[serde(default)]
        client_version: String,
        /// Source workspace, copied into every audit row this session writes.
        #[serde(default)]
        workspace: String,
        /// The revision this client's drawing has taken of this room, when
        /// it was in a session on it before: the server answers with what
        /// came after ([`ServerMessage::Welcome::replay_from`]). Absent on a
        /// first join, where the drawing is taken to be what the room has.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<i64>,
        /// Which repository the `since` is of: the `repo_id` this client's
        /// drawing recorded of the room it was in (its own saved copy, or
        /// the last session's `welcome`). Naming it asks for the drawing to
        /// be taken as consistent with the room, and the server checks
        /// that; a refusal comes in the `welcome`
        /// ([`ServerMessage::Welcome::align_refused`]). A `hello` that names
        /// none is not checked: a `since` alone is replayed from as far as
        /// the op log allows, as for a client that predates this field, and
        /// with no `since` either the join is a first one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo_id: Option<String>,
        /// The tags the objects of this client's own copy of the drawing
        /// carry, as far as no op of its own is still on its way to say so.
        /// Two people who each opened a file of their own need not have the
        /// same objects in it, and the server knows a drawing only by the
        /// ops it applied: this is how it learns which tags are in use in a
        /// copy it never saw, so that it gives none of them to a symbol
        /// somebody else places. Absent when there is none to list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        existing_tags: Vec<String>,
    },
    /// Keep-alive, expected every 15 s.
    Heartbeat { session_id: String },
    /// Requests exclusive ownership of one tag.
    Claim { session_id: String, tag: String },
    /// Gives up an owned tag voluntarily.
    Release { session_id: String, tag: String },
    /// Submits one P&ID semantic operation.
    Op {
        session_id: String,
        base_revision: i64,
        op_id: String,
        payload: Value,
    },
    /// Reports a write the client itself refused: a session that may not
    /// write turned a command away before it ran, or took an edit back.
    /// Nothing changes on the drawing; the server keeps the record
    /// (`audit_log`) of who tried what on which object.
    Attempt {
        session_id: String,
        /// The tag of the object the write was aimed at, when it had one.
        #[serde(default)]
        object_ref: String,
        /// What was tried, as the person would recognise it.
        #[serde(default)]
        text: String,
    },
}

/// A message sent by the server to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Session established.
    Welcome {
        session_id: String,
        base_revision: i64,
        peers: Vec<Peer>,
        claims: Vec<ClaimInfo>,
        /// Which repository this room is of. It goes into the drawing file
        /// on save and comes back in the next `hello`, which is how a copy
        /// tells "the room I was in" from "a room under the same address"
        /// (`REQ-COLL-015`). Empty from a server that does not know it.
        #[serde(default)]
        repo_id: String,
        /// Present when the server replays what the session missed: right
        /// behind this message, and before anything else on the socket, come
        /// the `op_applied` of revisions `replay_from + 1 ..= base_revision`,
        /// each as it was first told to the room. It is the `since` the
        /// `hello` gave. Absent: nothing is replayed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replay_from: Option<i64>,
        /// Present when the `hello` named a repository and the server cannot
        /// take the drawing as consistent with the room: the repository is
        /// another, the revision is ahead of what the room has, or the op log
        /// no longer holds what a replay needs (`REQ-COLL-015`). The session
        /// is still established — the person sees the room — but nothing is
        /// replayed, and every `op` it sends is answered `NOT_ALIGNED`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        align_refused: Option<AlignRefusal>,
    },
    /// Structured failure. `level` names the first denying tier when the
    /// failure came out of the permission chain.
    Error {
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        holder: Option<String>,
        /// The op this answers, when it answers one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        op_id: Option<String>,
        /// `true` when nothing of that op took effect: its sender takes the
        /// step back. Absent when the error is about one change of an op
        /// whose other changes may have taken effect (they come as
        /// `op_applied`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rejected: Option<bool>,
    },
    /// Tag ownership granted.
    ClaimGranted { tag: String, session_id: String },
    /// Tag ownership refused; `holder` still owns it until `expires_at`.
    ClaimDenied {
        tag: String,
        holder: String,
        expires_at: i64,
    },
    /// Tag ownership ended. `reason` is `released` or `expired`.
    ClaimReleased { tag: String, reason: String },
    /// Operation applied; broadcast to every session on the drawing.
    OpApplied {
        op_id: String,
        revision: i64,
        changes: Vec<Value>,
    },
    /// A peer joined this drawing.
    PeerJoined(Peer),
    /// A peer left this drawing.
    PeerLeft(Peer),
    /// The session may not write. Sent to a session when its drawing goes
    /// 只读, and after `welcome` to one that joins without write access —
    /// its user's role does not edit, or the drawing is 只读 already.
    ///
    /// `holder` is the session that holds the edit right of a 只读 drawing,
    /// when there is one. It is sent this message too, naming itself: that
    /// is how it learns the edit right is its own.
    ///
    /// `level` and `code` say which tier of the permission chain refused
    /// (`role` / `pbs` / `state`, and the `PERM_DENIED_*` code that goes
    /// with it), so the client can answer a refused write the same way the
    /// server would.
    ReadonlyDowngrade {
        reason: String,
        holder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    /// A conflict entry was created and awaits adjudication.
    Conflict {
        conflict_id: String,
        object_ref: String,
        field: String,
        value_a: String,
        value_b: String,
        /// The two ops that tied. Neither gets an `op_applied` or an
        /// `error` for the change that tied; this is how each one's sender
        /// knows its op went into the entry.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        op_ids: Vec<String>,
    },
}

/// One connected participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub session_id: String,
    pub user_id: String,
    #[serde(default)]
    pub display_name: String,
}

/// One outstanding tag claim, as advertised in `welcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimInfo {
    pub tag: String,
    pub session_id: String,
    pub user_id: String,
    pub expires_at: i64,
}

/// Why a `welcome` did not take the drawing as consistent with the room:
/// `code` is one of `REPO_MISMATCH`, `REVISION_AHEAD`, `REPLAY_GAP`, and
/// `message` the reason in the user's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignRefusal {
    pub code: String,
    pub message: String,
}

/// Error codes the protocol may return. Kept as constants so the client and
/// the server can never drift on spelling.
pub mod codes {
    pub const SESSION_UNKNOWN: &str = "SESSION_UNKNOWN";
    pub const SESSION_EXPIRED: &str = "SESSION_EXPIRED";
    pub const SESSION_TAKEN_OVER: &str = "SESSION_TAKEN_OVER";
    pub const HELLO_REQUIRED: &str = "HELLO_REQUIRED";
    pub const UNKNOWN_MESSAGE_TYPE: &str = "UNKNOWN_MESSAGE_TYPE";
    pub const INVALID_MESSAGE: &str = "INVALID_MESSAGE";
    pub const UNKNOWN_DRAWING: &str = "UNKNOWN_DRAWING";
    pub const UNKNOWN_USER: &str = "UNKNOWN_USER";
    pub const PERM_DENIED_ROLE: &str = "PERM_DENIED_ROLE";
    pub const PERM_DENIED_PBS: &str = "PERM_DENIED_PBS";
    pub const PERM_DENIED_STATE: &str = "PERM_DENIED_STATE";
    pub const PERM_UNKNOWN_SUBJECT: &str = "PERM_UNKNOWN_SUBJECT";
    pub const PERM_UNKNOWN_OPERATION: &str = "PERM_UNKNOWN_OPERATION";
    pub const PERM_INVALID_STATE: &str = "PERM_INVALID_STATE";
    /// A deletion refused because other objects still hang on the object.
    pub const OBJECT_REFERENCED: &str = "OBJECT_REFERENCED";
    /// The drawing's `hello` named a repository the server is not. The join
    /// itself stands; the copy is not taken as consistent.
    pub const REPO_MISMATCH: &str = "REPO_MISMATCH";
    /// The `since` a `hello` named is ahead of the revision the room has
    /// reached (or not a revision at all): the copy knows of a room this
    /// server does not.
    pub const REVISION_AHEAD: &str = "REVISION_AHEAD";
    /// The `since` a `hello` named can no longer be replayed: the op log
    /// does not hold every revision in between (or there is no log).
    pub const REPLAY_GAP: &str = "REPLAY_GAP";
    /// An `op` from a session whose `welcome` refused its alignment. It is
    /// not judged, logged or told to the room, and it is not refused
    /// either: the step stays with its sender for a connection that aligns.
    pub const NOT_ALIGNED: &str = "NOT_ALIGNED";
    /// The server's database could not be opened, so it takes no session:
    /// a `hello` is answered with this and the connection closed. What the
    /// session sent would be kept nowhere, and judged against a repository
    /// id made up for this one run (`SPEC-collab-offline-sync.md` §6).
    pub const STORE_UNAVAILABLE: &str = "STORE_UNAVAILABLE";
    /// A change to an object another session deleted after the change's
    /// baseline. The deletion stands (`architecture.md` §3.3, object level).
    pub const OBJECT_DELETED: &str = "OBJECT_DELETED";
    pub const INVALID_STATE_TRANSITION: &str = "INVALID_STATE_TRANSITION";
    /// The op was made against a revision the drawing has left, and it is of
    /// a kind the server cannot merge. Catch up and send it again.
    pub const STALE_BASE_REVISION: &str = "STALE_BASE_REVISION";
    /// No conflict entry has that id.
    pub const CONFLICT_UNKNOWN: &str = "CONFLICT_UNKNOWN";
    /// The conflict entry was ruled on before; a ruling is made once.
    pub const CONFLICT_ALREADY_RESOLVED: &str = "CONFLICT_ALREADY_RESOLVED";
    /// A field write that was behind: another session wrote the same field
    /// after the op's baseline, and that value stays (`architecture.md` §3.3
    /// step 2). Not to be sent again.
    pub const OP_SUPERSEDED: &str = "OP_SUPERSEDED";
    /// A write to an object whose tag another session has claimed, or one
    /// that would give an object such a tag. The claim stands until its
    /// holder releases it or it runs out (`VAL-CROSS-002`).
    pub const CLAIM_HELD: &str = "CLAIM_HELD";
    /// An inflow connector on a line of a drawing that has one already
    /// (`VAL-PIPE-024`, `VAL-CROSS-009`). The editor refuses it before it is
    /// made; the server refuses the second of two made at once.
    pub const DUPLICATE_INFLOW_CONNECTOR: &str = "DUPLICATE_INFLOW_CONNECTOR";
}

/// Reasons carried by [`ServerMessage::ClaimReleased`].
pub mod release_reasons {
    /// The owner sent `release`.
    pub const RELEASED: &str = "released";
    /// The claim's TTL elapsed.
    pub const EXPIRED: &str = "expired";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips_through_json_with_workspace() {
        let raw = r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101",
                      "mode":"协同编辑","client_version":"1","workspace":"plant-a"}"#;
        let msg: ClientMessage = serde_json::from_str(raw).expect("hello must parse");
        match &msg {
            ClientMessage::Hello {
                user_id,
                drawing_id,
                workspace,
                ..
            } => {
                assert_eq!(user_id, "u1");
                assert_eq!(drawing_id, "PID-1101");
                assert_eq!(workspace, "plant-a");
            }
            other => panic!("expected hello, got {other:?}"),
        }
        let back = serde_json::to_string(&msg).expect("hello must serialize");
        assert!(back.contains("\"type\":\"hello\""));
    }

    #[test]
    fn hello_tolerates_missing_optional_fields() {
        let raw = r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑"}"#;
        let msg: ClientMessage = serde_json::from_str(raw).expect("optional fields default");
        match msg {
            ClientMessage::Hello {
                client_version,
                workspace,
                ..
            } => {
                assert_eq!(client_version, "");
                assert_eq!(workspace, "");
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    #[test]
    fn op_payload_is_opaque_to_the_protocol_layer() {
        let raw = r#"{"type":"op","session_id":"s-1","base_revision":0,"op_id":"op-001",
                      "payload":{"kind":"set_property","object_ref":"V-1001",
                                 "field":"设计压力","value":"1.6MPa"}}"#;
        let msg: ClientMessage = serde_json::from_str(raw).expect("op must parse");
        match msg {
            ClientMessage::Op { payload, op_id, .. } => {
                assert_eq!(op_id, "op-001");
                assert_eq!(payload["kind"], "set_property");
                assert_eq!(payload["value"], "1.6MPa");
            }
            other => panic!("expected op, got {other:?}"),
        }
    }

    #[test]
    fn unknown_message_type_fails_to_deserialize() {
        let raw = r#"{"type":"totally_unknown_type","session_id":"s-1"}"#;
        assert!(serde_json::from_str::<ClientMessage>(raw).is_err());
    }

    #[test]
    fn welcome_serializes_peers_and_claims_as_arrays() {
        let msg = ServerMessage::Welcome {
            session_id: "s-1".into(),
            base_revision: 0,
            peers: vec![Peer {
                session_id: "s-1".into(),
                user_id: "u1".into(),
                display_name: "工艺甲".into(),
            }],
            claims: vec![ClaimInfo {
                tag: "V-1001".into(),
                session_id: "s-1".into(),
                user_id: "u1".into(),
                expires_at: 1_700_000_000,
            }],
            repo_id: "r-test".into(),
            replay_from: None,
            align_refused: None,
        };
        let value: Value = serde_json::to_value(&msg).expect("welcome must serialize");
        assert_eq!(value["type"], "welcome");
        assert!(value["peers"].is_array());
        assert!(value["claims"].is_array());
        assert_eq!(value["claims"][0]["tag"], "V-1001");
    }

    #[test]
    fn error_omits_absent_optional_fields() {
        let msg = ServerMessage::Error {
            code: codes::HELLO_REQUIRED.into(),
            message: "send hello first".into(),
            level: None,
            holder: None,
            op_id: None,
            rejected: None,
        };
        let text = serde_json::to_string(&msg).expect("error must serialize");
        assert!(!text.contains("level"));
        assert!(!text.contains("holder"));
    }

    /// `VAL-CROSS-008`: what a session coming back says and is told. Each of
    /// the four additions is optional on the wire — a frame without it
    /// reads as it always did, and a frame is written without it when there
    /// is nothing to say.
    #[test]
    fn collab_offline_queue_the_frames_of_coming_back_are_optional_additions() {
        // `hello.since`.
        let first: ClientMessage = serde_json::from_str(
            r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑"}"#,
        )
        .expect("a first hello");
        assert!(matches!(first, ClientMessage::Hello { since: None, .. }));
        assert!(!serde_json::to_string(&first).expect("serialize").contains("since"));
        let back: ClientMessage = serde_json::from_str(
            r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑","since":6}"#,
        )
        .expect("a hello that comes back");
        assert!(matches!(back, ClientMessage::Hello { since: Some(6), .. }));
        assert!(serde_json::to_string(&back).expect("serialize").contains(r#""since":6"#));

        // `welcome.replay_from`.
        let plain: ServerMessage = serde_json::from_str(
            r#"{"type":"welcome","session_id":"s-1","base_revision":9,"peers":[],"claims":[]}"#,
        )
        .expect("a welcome as it has always been");
        assert!(matches!(plain, ServerMessage::Welcome { replay_from: None, .. }));
        assert!(!serde_json::to_string(&plain).expect("serialize").contains("replay_from"));
        let replaying = ServerMessage::Welcome {
            session_id: "s-2".into(),
            base_revision: 9,
            peers: Vec::new(),
            claims: Vec::new(),
            repo_id: "r-test".into(),
            replay_from: Some(6),
            align_refused: None,
        };
        let value: Value = serde_json::to_value(&replaying).expect("serialize");
        assert_eq!(value["replay_from"], 6);

        // `error.op_id` and `error.rejected`.
        let refused = ServerMessage::Error {
            code: codes::OP_SUPERSEDED.into(),
            message: "…".into(),
            level: None,
            holder: None,
            op_id: Some("op-s1-12".into()),
            rejected: Some(true),
        };
        let value: Value = serde_json::to_value(&refused).expect("serialize");
        assert_eq!((&value["op_id"], &value["rejected"]), (&Value::from("op-s1-12"), &Value::from(true)));
        let old: ServerMessage =
            serde_json::from_str(r#"{"type":"error","code":"STALE_BASE_REVISION","message":"…"}"#)
                .expect("an error as it has always been");
        assert!(matches!(old, ServerMessage::Error { op_id: None, rejected: None, .. }));

        // `conflict.op_ids`.
        let old: ServerMessage = serde_json::from_str(
            r#"{"type":"conflict","conflict_id":"c-1","object_ref":"P-1001","field":"设计压力",
                "value_a":"1.6MPa","value_b":"2.5MPa"}"#,
        )
        .expect("a conflict as it has always been");
        assert!(matches!(&old, ServerMessage::Conflict { op_ids, .. } if op_ids.is_empty()));
        assert!(!serde_json::to_string(&old).expect("serialize").contains("op_ids"));
        let tied = ServerMessage::Conflict {
            conflict_id: "c-1".into(),
            object_ref: "P-1001".into(),
            field: "设计压力".into(),
            value_a: "1.6MPa".into(),
            value_b: "2.5MPa".into(),
            op_ids: vec!["op-a".into(), "op-b".into()],
        };
        assert_eq!(
            serde_json::to_value(&tied).expect("serialize")["op_ids"],
            serde_json::json!(["op-a", "op-b"])
        );
    }

    /// `REQ-COLL-015`: the repo identity rides `hello` and `welcome`, and
    /// so does a refused alignment (`OFF-2`: in the `welcome`, not a frame
    /// of its own) — all optional additions, so an old frame still reads.
    #[test]
    fn collab_align_repo_id_and_align_refused_are_optional_additions() {
        let plain: ClientMessage = serde_json::from_str(
            r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑"}"#,
        )
        .expect("a hello without a repo claim");
        assert!(matches!(plain, ClientMessage::Hello { repo_id: None, .. }));
        assert!(!serde_json::to_string(&plain)
            .expect("serialize")
            .contains("repo_id"));

        let claiming: ClientMessage = serde_json::from_str(
            r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑",
                "repo_id":"r-9","since":6}"#,
        )
        .expect("a hello that claims a repo");
        assert!(matches!(
            claiming,
            ClientMessage::Hello { repo_id: Some(ref r), since: Some(6), .. } if r == "r-9"
        ));
        assert!(serde_json::to_string(&claiming)
            .expect("serialize")
            .contains(r#""repo_id":"r-9"#));

        let old_welcome: ServerMessage = serde_json::from_str(
            r#"{"type":"welcome","session_id":"s-1","base_revision":9,"peers":[],"claims":[]}"#,
        )
        .expect("a welcome without repo_id still parses");
        assert!(
            matches!(old_welcome, ServerMessage::Welcome { ref repo_id, .. } if repo_id.is_empty())
        );

        assert!(serde_json::to_value(&old_welcome)
            .expect("serialize")
            .get("align_refused")
            .is_none());

        let refusing: ServerMessage = serde_json::from_str(
            r#"{"type":"welcome","session_id":"s-1","base_revision":9,"peers":[],"claims":[],
                "repo_id":"r-9","align_refused":{"code":"REPO_MISMATCH","message":"这份文件属于另一个服务器库"}}"#,
        )
        .expect("a welcome that refuses the claim");
        let value = serde_json::to_value(&refusing).expect("serialize");
        assert_eq!(value["align_refused"]["code"], "REPO_MISMATCH", "{value}");
        assert_eq!(
            value["align_refused"]["message"], "这份文件属于另一个服务器库",
            "{value}"
        );
        assert!(
            serde_json::from_str::<ServerMessage>(
                r#"{"type":"align_refused","code":"REPO_MISMATCH","message":"x"}"#
            )
            .is_err(),
            "a refusal is no frame of its own"
        );
    }

    /// `hello.existing_tags`: the tags the objects of the client's own copy
    /// of the drawing carry. One more optional addition — a hello without
    /// it reads as it always did, and one with nothing to report is written
    /// without it.
    #[test]
    fn collab_existing_tags_a_hello_lists_the_tags_of_its_drawing_only_when_it_has_some() {
        // A hello as the client wrote it, read and written back.
        let read = |text: &str| -> Value {
            let hello: ClientMessage = serde_json::from_str(text).expect("a hello");
            serde_json::to_value(&hello).expect("serialize")
        };
        let plain =
            read(r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑"}"#);
        assert!(plain.get("existing_tags").is_none(), "not written");

        let telling = read(
            r#"{"type":"hello","user_id":"u1","drawing_id":"PID-1101","mode":"协同编辑",
                "existing_tags":["P-1001","V-1001"]}"#,
        );
        let listed = serde_json::json!(["P-1001", "V-1001"]);
        assert_eq!(telling["existing_tags"], listed);
    }

    #[test]
    fn peer_joined_carries_the_peer_inline() {
        let msg = ServerMessage::PeerJoined(Peer {
            session_id: "s-2".into(),
            user_id: "u2".into(),
            display_name: "工艺乙".into(),
        });
        let value: Value = serde_json::to_value(&msg).expect("peer_joined must serialize");
        assert_eq!(value["type"], "peer_joined");
        assert_eq!(value["session_id"], "s-2");
        assert_eq!(value["display_name"], "工艺乙");
    }

    /// A downgrade as it has always been written — `reason` and `holder`
    /// only — still reads, and the tier it came from is said only when
    /// there is one to say.
    #[test]
    fn a_downgrade_says_its_tier_only_when_it_has_one() {
        let plain: ServerMessage =
            serde_json::from_str(r#"{"type":"readonly_downgrade","reason":"图纸已切换为只读","holder":"s-1"}"#)
                .expect("the two-field form must parse");
        assert_eq!(
            plain,
            ServerMessage::ReadonlyDowngrade {
                reason: "图纸已切换为只读".into(),
                holder: Some("s-1".into()),
                level: None,
                code: None,
            }
        );
        let text = serde_json::to_string(&plain).expect("serialize");
        assert!(!text.contains("level") && !text.contains("code"), "{text}");

        let by_role = ServerMessage::ReadonlyDowngrade {
            reason: "角色不允许编辑图纸".into(),
            holder: None,
            level: Some("role".into()),
            code: Some(codes::PERM_DENIED_ROLE.into()),
        };
        let value: Value = serde_json::to_value(&by_role).expect("serialize");
        assert_eq!(value["level"], "role");
        assert_eq!(value["code"], "PERM_DENIED_ROLE");
        assert!(value["holder"].is_null());
    }

    /// An attempt carries what was tried and on what; both may be absent.
    #[test]
    fn an_attempt_round_trips_and_tolerates_missing_fields() {
        let raw = r#"{"type":"attempt","session_id":"s-1","object_ref":"P-1001","text":"PIDTAG P-1001 P-1009"}"#;
        let msg: ClientMessage = serde_json::from_str(raw).expect("attempt must parse");
        assert_eq!(
            msg,
            ClientMessage::Attempt {
                session_id: "s-1".into(),
                object_ref: "P-1001".into(),
                text: "PIDTAG P-1001 P-1009".into(),
            }
        );
        let bare: ClientMessage =
            serde_json::from_str(r#"{"type":"attempt","session_id":"s-1"}"#).expect("bare attempt");
        assert!(matches!(
            bare,
            ClientMessage::Attempt { object_ref, text, .. } if object_ref.is_empty() && text.is_empty()
        ));
    }

    #[test]
    fn claim_denied_reports_holder_and_expiry() {
        let msg = ServerMessage::ClaimDenied {
            tag: "V-1001".into(),
            holder: "s-1".into(),
            expires_at: 1_700_000_300,
        };
        let value: Value = serde_json::to_value(&msg).expect("claim_denied must serialize");
        assert_eq!(value["type"], "claim_denied");
        assert_eq!(value["holder"], "s-1");
        assert_eq!(value["expires_at"], 1_700_000_300_i64);
    }
}

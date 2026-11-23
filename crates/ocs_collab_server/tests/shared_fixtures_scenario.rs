//! Verifies that server-side scenario tests can drive `ocs_collab_server`
//! directly from the shared `ocs_collab::protocol::FIXTURE_JSON` fixtures (#21).

use ocs_collab::protocol::FIXTURE_JSON;
use ocs_collab::{ClientMessage, ServerMessage, SessionConfig};
use ocs_collab_server::store::Store;
use ocs_collab_server::ws::{handle_message, Connection, Reaction};
use ocs_collab_server::AppState;
use serde_json::Value;

fn fixture_client(name: &str) -> Value {
    let doc: Value = serde_json::from_str(FIXTURE_JSON).expect("valid fixture json");
    for group in ["client", "legacy_client"] {
        if let Some(arr) = doc[group].as_array() {
            if let Some(found) = arr.iter().find(|item| item["name"] == name) {
                return found["frame"].clone();
            }
        }
    }
    panic!("fixture `{name}` not found");
}

#[tokio::test]
async fn server_scenario_drives_session_from_shared_protocol_fixtures() {
    let store = Store::open_in_memory().expect("memory store");
    let state = AppState::new(Some(store), None, ":memory:", SessionConfig::default());
    let mut conn = Connection::new("127.0.0.1");

    // 1. Hello from legacy fixture (no repo_id, no since -> first join).
    let hello: ClientMessage =
        serde_json::from_value(fixture_client("legacy_hello_v1")).expect("parse hello fixture");
    let reactions = handle_message(&state, &mut conn, hello, 1_700_000_000).await;
    assert!(
        reactions
            .iter()
            .any(|r| matches!(r, Reaction::Send(ServerMessage::Welcome { .. }))),
        "welcome expected: {reactions:?}"
    );
    let sid = conn.session.as_ref().expect("session").session_id.clone();

    // 2. Claim from shared fixture (patching session_id to live session).
    let mut claim_val = fixture_client("claim");
    claim_val["session_id"] = Value::String(sid.clone());
    let claim_msg: ClientMessage = serde_json::from_value(claim_val).expect("parse claim");
    let reactions = handle_message(&state, &mut conn, claim_msg, 1_700_000_001).await;
    assert!(
        reactions.iter().any(|r| matches!(
            r,
            Reaction::Send(ServerMessage::ClaimGranted { .. })
                | Reaction::Publish {
                    message: ServerMessage::ClaimGranted { .. },
                    ..
                }
        )),
        "claim_granted expected: {reactions:?}"
    );

    // 3. Op from shared fixture.
    let mut op_val = fixture_client("op_set_property");
    op_val["session_id"] = Value::String(sid.clone());
    let op_msg: ClientMessage = serde_json::from_value(op_val).expect("parse op");
    let reactions = handle_message(&state, &mut conn, op_msg, 1_700_000_002).await;
    assert!(
        reactions.iter().any(|r| matches!(
            r,
            Reaction::Published {
                message: ServerMessage::OpApplied { revision: 1, .. },
                ..
            }
        )),
        "op_applied expected: {reactions:?}"
    );

    // 4. Release from shared fixture.
    let mut rel_val = fixture_client("release");
    rel_val["session_id"] = Value::String(sid);
    let rel_msg: ClientMessage = serde_json::from_value(rel_val).expect("parse release");
    let reactions = handle_message(&state, &mut conn, rel_msg, 1_700_000_003).await;
    assert!(
        reactions.iter().any(|r| matches!(
            r,
            Reaction::Publish {
                message: ServerMessage::ClaimReleased { .. },
                ..
            }
        )),
        "claim_released expected: {reactions:?}"
    );
}

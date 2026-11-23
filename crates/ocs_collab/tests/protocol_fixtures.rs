//! Shared protocol frame fixture round-trip and coverage tests (#21).
//!
//! Both `ocs_collab` and `ocs_collab_server` read `fixtures/protocol_frames.json`
//! so protocol tests and scenario harnesses share one canonical set of frames.

use std::collections::BTreeSet;

use ocs_collab::protocol::{codes, FIXTURE_JSON};
use ocs_collab::{ClientMessage, ServerMessage};
use serde_json::Value;

fn load_fixtures() -> Value {
    serde_json::from_str(FIXTURE_JSON).expect("protocol_frames.json must be valid JSON")
}

#[test]
fn client_frame_fixtures_round_trip_and_cover_all_types() {
    let doc = load_fixtures();
    let mut seen_types = BTreeSet::new();

    for group in ["client", "legacy_client"] {
        let entries = doc[group]
            .as_array()
            .unwrap_or_else(|| panic!("missing array {group}"));
        assert!(!entries.is_empty(), "{group} must not be empty");
        for entry in entries {
            let name = entry["name"].as_str().expect("fixture name");
            let raw = &entry["frame"];
            let msg: ClientMessage = serde_json::from_value(raw.clone())
                .unwrap_or_else(|e| panic!("client fixture {name} failed to deserialize: {e}"));
            let encoded = serde_json::to_value(&msg)
                .unwrap_or_else(|e| panic!("client fixture {name} failed to serialize: {e}"));
            let decoded: ClientMessage = serde_json::from_value(encoded.clone())
                .unwrap_or_else(|e| panic!("client fixture {name} failed second decode: {e}"));
            assert_eq!(msg, decoded, "client fixture {name} did not round-trip");
            if let Some(ty) = encoded["type"].as_str() {
                seen_types.insert(ty.to_owned());
            }
        }
    }

    for required in ["hello", "heartbeat", "claim", "release", "op", "attempt", "request_write"] {
        assert!(
            seen_types.contains(required),
            "client fixtures missing message type `{required}`"
        );
    }
}

#[test]
fn server_frame_fixtures_round_trip_and_cover_all_types_and_error_codes() {
    let doc = load_fixtures();
    let mut seen_types = BTreeSet::new();
    let mut seen_codes = BTreeSet::new();

    for group in ["server", "legacy_server"] {
        let entries = doc[group]
            .as_array()
            .unwrap_or_else(|| panic!("missing array {group}"));
        assert!(!entries.is_empty(), "{group} must not be empty");
        for entry in entries {
            let name = entry["name"].as_str().expect("fixture name");
            let raw = &entry["frame"];
            let msg: ServerMessage = serde_json::from_value(raw.clone())
                .unwrap_or_else(|e| panic!("server fixture {name} failed to deserialize: {e}"));
            let encoded = serde_json::to_value(&msg)
                .unwrap_or_else(|e| panic!("server fixture {name} failed to serialize: {e}"));
            let decoded: ServerMessage = serde_json::from_value(encoded.clone())
                .unwrap_or_else(|e| panic!("server fixture {name} failed second decode: {e}"));
            assert_eq!(msg, decoded, "server fixture {name} did not round-trip");
            if let Some(ty) = encoded["type"].as_str() {
                seen_types.insert(ty.to_owned());
            }
        }
    }

    let error_entries = doc["error_codes"]
        .as_array()
        .expect("missing error_codes array");
    for entry in error_entries {
        let expected_code = entry["code"].as_str().expect("error code name");
        let raw = &entry["frame"];
        let msg: ServerMessage = serde_json::from_value(raw.clone())
            .unwrap_or_else(|e| panic!("error fixture {expected_code} failed to deserialize: {e}"));
        let encoded = serde_json::to_value(&msg)
            .unwrap_or_else(|e| panic!("error fixture {expected_code} failed to serialize: {e}"));
        let decoded: ServerMessage = serde_json::from_value(encoded.clone())
            .unwrap_or_else(|e| panic!("error fixture {expected_code} failed second decode: {e}"));
        assert_eq!(msg, decoded, "error fixture {expected_code} did not round-trip");
        assert_eq!(encoded["type"], "error");
        assert_eq!(encoded["code"], expected_code);
        seen_types.insert("error".to_owned());
        seen_codes.insert(expected_code.to_owned());
    }

    for required in [
        "welcome",
        "error",
        "claim_granted",
        "claim_denied",
        "claim_released",
        "op_applied",
        "peer_joined",
        "peer_disconnected",
        "peer_left",
        "readonly_downgrade",
        "write_restored",
        "write_status",
        "conflict",
    ] {
        assert!(
            seen_types.contains(required),
            "server fixtures missing message type `{required}`"
        );
    }

    for code in codes::ALL {
        assert!(
            seen_codes.contains(*code),
            "error_codes fixtures missing registered code `{code}`"
        );
    }
}

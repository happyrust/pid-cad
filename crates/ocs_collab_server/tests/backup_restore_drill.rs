//! Automated online SQLite backup & restore drill (#105).
//!
//! Exercises:
//! 1. Online WAL-safe backup via `Store::backup_into_dir` (`VACUUM INTO`) while
//!    two active clients are collaborating.
//! 2. Additional edits committed AFTER the backup snapshot (`+12s`).
//! 3. Server stop -> restore from latest backup -> restart with `--new-repo-id`.
//! 4. Both clients reconnect with their saved `(repo_id, since)` -> pre-backup
//!    ops survive intact, post-backup edits are NEVER silently dropped (both
//!    clients get explicit `REPO_MISMATCH` flagging divergence), and exact data
//!    loss window (RPO seconds) is computed and reported.

use std::path::PathBuf;
use std::sync::Arc;

use ocs_collab::protocol::codes;
use ocs_collab::{ClientMessage, ServerMessage, SessionConfig};
use ocs_collab_server::ops;
use ocs_collab_server::store::Store;
use ocs_collab_server::ws::{handle_message, Connection, Reaction};
use ocs_collab_server::AppState;
use serde_json::json;

fn hello(user_id: &str, repo_id: Option<String>, since: Option<i64>) -> ClientMessage {
    ClientMessage::Hello {
        user_id: user_id.into(),
        drawing_id: "PID-1101".into(),
        mode: "协同编辑".into(),
        client_version: "drill".into(),
        workspace: "plant-a".into(),
        since,
        repo_id,
        existing_tags: Vec::new(),
        protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
        resume_session_id: None,
        credential: None,
    }
}

#[tokio::test]
async fn online_backup_and_restore_drill_reports_exact_rpo_without_silent_loss() {
    let root = std::env::temp_dir().join(format!("ocs-backup-drill-{}", uuid::Uuid::new_v4()));
    let live_db = root.join("live.db");
    let backup_dir = root.join("backups");
    let restored_db = root.join("restored.db");
    std::fs::create_dir_all(&root).expect("temp dir");

    let t0 = 1_700_000_000_i64;

    // ── Phase 1: Live server + 2 clients commit pre-backup ops ──────────
    let live_store = Store::open(&live_db).expect("open live db");
    let old_repo_id = live_store.repo_id().expect("repo_id");
    let mut state = AppState::new(
        Some(live_store),
        None,
        live_db.display().to_string(),
        SessionConfig::default(),
    )
    .with_open_mode(true);
    state.backup_dir = Some(backup_dir.clone());
    let state = Arc::new(state);

    let mut client_a = Connection::new("127.0.0.1");
    let mut client_b = Connection::new("127.0.0.1");
    handle_message(&state, &mut client_a, hello("u1", None, None), t0).await;
    handle_message(&state, &mut client_b, hello("u2", None, None), t0).await;
    let sid_a = client_a.session.as_ref().expect("a").session_id.clone();
    let sid_b = client_b.session.as_ref().expect("b").session_id.clone();

    // Pre-backup edits at t0 + 1 and t0 + 2.
    handle_message(
        &state,
        &mut client_a,
        ClientMessage::Op {
            session_id: sid_a.clone(),
            base_revision: 0,
            op_id: "op-pre-1".into(),
            payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "1.6MPa"}),
        },
        t0 + 1,
    )
    .await;
    handle_message(
        &state,
        &mut client_b,
        ClientMessage::Op {
            session_id: sid_b.clone(),
            base_revision: 1,
            op_id: "op-pre-2".into(),
            payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "公称直径", "value": "DN50"}),
        },
        t0 + 2,
    )
    .await;

    // ── Phase 2: Online backup while sessions remain active ─────────────
    let backup = state
        .store
        .as_ref()
        .expect("store")
        .backup_into_dir(&backup_dir, 1)
        .expect("online backup");
    assert!(backup.size_bytes > 0, "backup file must be non-empty");
    assert_eq!(backup.total_ops, 2);
    assert_eq!(backup.max_op_at, Some(t0 + 2));

    // ── Phase 3: Post-backup edits occur (+12s after last backed-up op) ─
    let post_backup_ts = t0 + 14;
    handle_message(
        &state,
        &mut client_a,
        ClientMessage::Op {
            session_id: sid_a,
            base_revision: 2,
            op_id: "op-post-3".into(),
            payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计温度", "value": "120C"}),
        },
        t0 + 10,
    )
    .await;
    handle_message(
        &state,
        &mut client_b,
        ClientMessage::Op {
            session_id: sid_b,
            base_revision: 3,
            op_id: "op-post-4".into(),
            payload: json!({"kind": "set_property", "object_ref": "V-1001", "field": "材质", "value": "316L"}),
        },
        post_backup_ts,
    )
    .await;
    // Both clients remember `(old_repo_id, revision = 4)` on disk.
    let client_saved_rev = 4_i64;
    let last_live_op_at = post_backup_ts;

    // ── Phase 4: Stop live server & restore from latest backup + --new-repo-id
    drop((client_a, client_b, state));
    std::fs::copy(PathBuf::from(&backup.path), &restored_db).expect("copy backup to restored db");

    let restored_store = Store::open(&restored_db).expect("open restored db");
    let new_repo_id = restored_store.renew_repo_id().expect("renew repo_id");
    assert_ne!(new_repo_id, old_repo_id);
    restored_store.clear_ephemeral().expect("clear ephemeral");
    let restored_state = AppState::new(
        Some(restored_store),
        None,
        restored_db.display().to_string(),
        SessionConfig::default(),
    )
    .with_open_mode(true);

    // Pre-backup ops survived intact at revision 2.
    let (rev, objs) = ops::objects(&restored_state, "PID-1101", Some("P-1001"));
    assert_eq!(rev, 2);
    assert_eq!(objs[0]["fields"]["设计压力"]["value"], "1.6MPa");
    assert_eq!(objs[0]["fields"]["公称直径"]["value"], "DN50");
    assert!(objs[0]["fields"].get("设计温度").is_none());

    // ── Phase 5: Both clients reconnect -> explicit REPO_MISMATCH (no silent loss)
    for user in ["u1", "u2"] {
        let mut reconn = Connection::new("127.0.0.1");
        let reactions = handle_message(
            &restored_state,
            &mut reconn,
            hello(user, Some(old_repo_id.clone()), Some(client_saved_rev)),
            post_backup_ts + 5,
        )
        .await;
        let refusal = reactions.iter().find_map(|r| match r {
            Reaction::Send(ServerMessage::Welcome { align_refused, .. }) => align_refused.clone(),
            _ => None,
        });
        assert_eq!(
            refusal.as_ref().map(|a| a.code.as_str()),
            Some(codes::REPO_MISMATCH),
            "reconnecting client {user} must be explicitly flagged REPO_MISMATCH: {reactions:?}"
        );
        assert!(reconn.not_aligned);
    }

    // ── Phase 6: Measure and verify exact RPO (data-loss duration in seconds)
    let backup_max_op_at = backup.max_op_at.expect("backup had pre-backup ops");
    let data_loss_seconds = (last_live_op_at - backup_max_op_at).max(0);
    assert_eq!(data_loss_seconds, 12);
    println!(
        "[backup_restore_drill] backup_file={} size={}B pre_backup_ops={} restored_rev={} rpo_data_loss_seconds={}s",
        backup.path, backup.size_bytes, backup.total_ops, rev, data_loss_seconds
    );

    drop(restored_state);
    let _ = std::fs::remove_dir_all(&root);
}

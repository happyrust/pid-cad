//! Load & latency measurement harness with 20 editing sessions + 1 observer
//! submitting 200 operations against a real WAL-backed server (#38).
//!
//! Measures and prints:
//! - Join handshake latency (p50, p95, max ms)
//! - Claim response latency (p50, p95, max ms)
//! - Broadcast fan-out latency across all 21 sessions for 200 ops (p50, p95, max ms)
//! - SQLite database + WAL/SHM size on disk

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ocs_collab::{ClientMessage, ServerMessage, SessionConfig};
use ocs_collab_server::http;
use ocs_collab_server::store::Store;
use ocs_collab_server::AppState;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct WsClient {
    stream: TcpStream,
    session_id: String,
}

impl WsClient {
    async fn connect(addr: SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.set_nodelay(true).expect("nodelay");
        let req = format!(
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.expect("ws upgrade");
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.expect("ws head");
            head.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 101"));
        stream
    }

    async fn send_frame(stream: &mut TcpStream, msg: &ClientMessage) {
        let text = serde_json::to_string(msg).expect("serialize");
        let payload = text.as_bytes();
        let mask = [0x5a, 0xa5, 0x3c, 0xc3];
        let mut frame = vec![0x80 | 0x1];
        if payload.len() < 126 {
            frame.push(0x80 | (payload.len() as u8));
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        stream.write_all(&frame).await.expect("write ws frame");
    }

    async fn recv_frame(stream: &mut TcpStream) -> ServerMessage {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let mut head = [0u8; 2];
                stream.read_exact(&mut head).await.expect("read head");
                let opcode = head[0] & 0x0f;
                let len = match head[1] & 0x7f {
                    126 => {
                        let mut b = [0u8; 2];
                        stream.read_exact(&mut b).await.expect("len16");
                        usize::from(u16::from_be_bytes(b))
                    }
                    127 => {
                        let mut b = [0u8; 8];
                        stream.read_exact(&mut b).await.expect("len64");
                        usize::try_from(u64::from_be_bytes(b)).expect("len64 fits")
                    }
                    n => usize::from(n),
                };
                let mut payload = vec![0u8; len];
                stream.read_exact(&mut payload).await.expect("read payload");
                if opcode == 0x1 {
                    return serde_json::from_slice(&payload).expect("valid ServerMessage");
                }
            }
        })
        .await
        .expect("timed out waiting for server frame")
    }

    async fn recv_until_op_revision(&mut self, expected_rev: i64) {
        loop {
            match Self::recv_frame(&mut self.stream).await {
                ServerMessage::OpApplied { revision, .. } if revision == expected_rev => return,
                ServerMessage::PeerJoined(_)
                | ServerMessage::ClaimGranted { .. }
                | ServerMessage::OpApplied { .. } => continue,
                other => panic!("unexpected message while waiting rev {expected_rev}: {other:?}"),
            }
        }
    }

    async fn recv_until_claim(&mut self, expected_tag: &str) {
        loop {
            match Self::recv_frame(&mut self.stream).await {
                ServerMessage::ClaimGranted { tag, .. } if tag == expected_tag => return,
                ServerMessage::PeerJoined(_) | ServerMessage::ClaimGranted { .. } => continue,
                other => panic!("unexpected message while waiting claim {expected_tag}: {other:?}"),
            }
        }
    }
}

fn percentile_ms(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    let idx = (((samples.len() as f64) * p).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[idx]
}

#[tokio::test]
async fn load_20_editing_sessions_and_200_ops_measures_latencies_and_db_size() {
    const EDITORS: usize = 20;
    const TOTAL_CLIENTS: usize = EDITORS + 1; // 20 script editors + 1 GUI observer
    const TOTAL_OPS: usize = 200;

    let dir = std::env::temp_dir().join(format!("ocs-load20-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db_path = dir.join("load20.db");

    let store = Store::open(&db_path).expect("open WAL store");
    let state = Arc::new(
        AppState::new(
            Some(store),
            None,
            db_path.display().to_string(),
            SessionConfig::default(),
        )
        .with_open_mode(true)
        // The default cap is 20 editing sessions (#98). This harness is
        // 20 editors plus one observer, all joined as editing sessions.
        .with_session_caps(TOTAL_CLIENTS, TOTAL_CLIENTS),
    );
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let router = http::router(state.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    // 1. Join 21 sessions (20 editors + 1 observer) and measure join latency.
    let mut clients: Vec<WsClient> = Vec::with_capacity(TOTAL_CLIENTS);
    let mut join_latencies_ms = Vec::with_capacity(TOTAL_CLIENTS);
    for i in 0..TOTAL_CLIENTS {
        let t_start = Instant::now();
        let mut stream = WsClient::connect(addr).await;
        WsClient::send_frame(
            &mut stream,
            &ClientMessage::Hello {
                user_id: format!("u-{i:02}"),
                drawing_id: "PID-LOAD-01".into(),
                mode: "协同编辑".into(),
                client_version: "load-bench".into(),
                workspace: "bench".into(),
                since: None,
                repo_id: None,
                existing_tags: Vec::new(),
                protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
                resume_session_id: None,
                credential: None,
                libraries: None,
            },
        )
        .await;
        let sid = match WsClient::recv_frame(&mut stream).await {
            ServerMessage::Welcome { session_id, .. } => session_id,
            other => panic!("expected welcome for client {i}, got {other:?}"),
        };
        join_latencies_ms.push(t_start.elapsed().as_secs_f64() * 1000.0);
        clients.push(WsClient {
            stream,
            session_id: sid,
        });
    }

    // 2. Claim 20 distinct tags across the 20 editors and measure claim latency.
    let mut claim_latencies_ms = Vec::with_capacity(EDITORS);
    for (i, client) in clients.iter_mut().enumerate().take(EDITORS) {
        let tag = format!("P-{:04}", 1001 + i);
        let t_start = Instant::now();
        let sid = client.session_id.clone();
        WsClient::send_frame(
            &mut client.stream,
            &ClientMessage::Claim {
                session_id: sid,
                tag: tag.clone(),
            },
        )
        .await;
        client.recv_until_claim(&tag).await;
        claim_latencies_ms.push(t_start.elapsed().as_secs_f64() * 1000.0);
    }

    // 3. Submit 200 ops round-robin across the 20 editors and measure full
    // fan-out broadcast latency to all 21 connected sessions.
    let mut broadcast_latencies_ms = Vec::with_capacity(TOTAL_OPS);
    for step in 0..TOTAL_OPS {
        let editor_idx = step % EDITORS;
        let tag = format!("P-{:04}", 1001 + editor_idx);
        let expected_rev = (step + 1) as i64;
        let sid = clients[editor_idx].session_id.clone();

        let t_start = Instant::now();
        WsClient::send_frame(
            &mut clients[editor_idx].stream,
            &ClientMessage::Op {
                session_id: sid,
                base_revision: step as i64,
                op_id: format!("op-load-{expected_rev:04}"),
                payload: json!({
                    "kind": "set_property",
                    "object_ref": tag,
                    "field": "设计压力",
                    "value": format!("{:.2}MPa", 1.0 + (step as f64) * 0.01),
                }),
            },
        )
        .await;
        // Wait for ALL 21 sessions to receive the OpApplied frame for `expected_rev`.
        for c in &mut clients {
            c.recv_until_op_revision(expected_rev).await;
        }
        broadcast_latencies_ms.push(t_start.elapsed().as_secs_f64() * 1000.0);
    }

    let join_p50 = percentile_ms(&mut join_latencies_ms, 0.50);
    let join_p95 = percentile_ms(&mut join_latencies_ms, 0.95);
    let join_max = *join_latencies_ms.last().unwrap();

    let claim_p50 = percentile_ms(&mut claim_latencies_ms, 0.50);
    let claim_p95 = percentile_ms(&mut claim_latencies_ms, 0.95);
    let claim_max = *claim_latencies_ms.last().unwrap();

    let bc_p50 = percentile_ms(&mut broadcast_latencies_ms, 0.50);
    let bc_p95 = percentile_ms(&mut broadcast_latencies_ms, 0.95);
    let bc_max = *broadcast_latencies_ms.last().unwrap();

    let db_size = state.store.as_ref().expect("store").db_size_bytes();
    let ops_rows = state
        .store
        .as_ref()
        .expect("store")
        .ops_count_by_drawing()
        .expect("ops count")
        .get("PID-LOAD-01")
        .copied()
        .unwrap_or(0);
    // #103 drops ops already folded into a snapshot, so the table does not
    // still hold all 200. What remains, plus the snapshot, is the drawing.
    let snapshot_rev = state
        .store
        .as_ref()
        .expect("store")
        .latest_snapshot("PID-LOAD-01")
        .expect("snapshot")
        .map(|(revision, _)| revision)
        .unwrap_or(0);
    assert_eq!(snapshot_rev + ops_rows, TOTAL_OPS as i64);
    assert_eq!(state.metrics.applied_ops_total(), TOTAL_OPS as u64);
    assert!(db_size > 0);

    println!(
        "\n=== #38 Load Benchmark (20 editors + 1 observer, 200 ops) ===\n\
         join_ms:      p50={join_p50:.2} p95={join_p95:.2} max={join_max:.2}\n\
         claim_ms:     p50={claim_p50:.2} p95={claim_p95:.2} max={claim_max:.2}\n\
         broadcast_ms: p50={bc_p50:.2} p95={bc_p95:.2} max={bc_max:.2}\n\
         db_size_bytes: {db_size} (ops_log_rows={ops_rows})\n"
    );

    drop(clients);
    server.abort();
    drop(state);
    let _ = std::fs::remove_dir_all(&dir);
}

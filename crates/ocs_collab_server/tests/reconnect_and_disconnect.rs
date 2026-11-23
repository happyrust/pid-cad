//! End-to-end TCP/WebSocket integration tests for:
//! - #22: Network-only disconnect via `TcpFaultProxy` proving reconnect without
//!   resume gets blocked by own previous session's claim (`CLAIM_HELD`).
//! - #36: Reconnect with `resume_session_id` takes over prior session immediately,
//!   transfers claims, keeps 1 roster entry, and commits offline edits.
//! - #37: Distinguishes member `disconnected` (`peer_disconnected` + `hold_until`)
//!   vs `left` (`peer_left` only on expiry or explicit Close frame).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ocs_collab::protocol::codes;
use ocs_collab::{ClientMessage, ServerMessage, SessionConfig};
use ocs_collab_server::http;
use ocs_collab_server::proxy::TcpFaultProxy;
use ocs_collab_server::store::Store;
use ocs_collab_server::AppState;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct RawWs {
    stream: TcpStream,
}

impl RawWs {
    async fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
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
        let head_str = String::from_utf8_lossy(&head);
        assert!(head_str.starts_with("HTTP/1.1 101"), "{head_str}");
        Self { stream }
    }

    async fn send(&mut self, msg: &ClientMessage) {
        let text = serde_json::to_string(msg).expect("serialize client msg");
        let payload = text.as_bytes();
        let mask = [0x11, 0x22, 0x33, 0x44];
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
        self.stream.write_all(&frame).await.expect("send ws frame");
    }

    async fn recv_msg(&mut self) -> ServerMessage {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let mut head = [0u8; 2];
                self.stream.read_exact(&mut head).await.expect("ws frame head");
                let opcode = head[0] & 0x0f;
                let len = match head[1] & 0x7f {
                    126 => {
                        let mut b = [0u8; 2];
                        self.stream.read_exact(&mut b).await.expect("len16");
                        usize::from(u16::from_be_bytes(b))
                    }
                    127 => {
                        let mut b = [0u8; 8];
                        self.stream.read_exact(&mut b).await.expect("len64");
                        usize::try_from(u64::from_be_bytes(b)).expect("len64 fits")
                    }
                    n => usize::from(n),
                };
                let mut payload = vec![0u8; len];
                self.stream.read_exact(&mut payload).await.expect("payload");
                if opcode == 0x1 {
                    return serde_json::from_slice::<ServerMessage>(&payload)
                        .expect("valid ServerMessage JSON");
                }
            }
        })
        .await
        .expect("timed out waiting for ServerMessage")
    }
}

async fn start_server(config: SessionConfig) -> (SocketAddr, Arc<AppState>, tokio::task::JoinHandle<()>) {
    let store = Store::open_in_memory().expect("memory store");
    let state = Arc::new(AppState::new(Some(store), None, ":memory:", config).with_open_mode(true));
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let router = http::router(state.clone());
    let task = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    (addr, state, task)
}

fn hello_msg(user_id: &str, resume: Option<String>) -> ClientMessage {
    ClientMessage::Hello {
        user_id: user_id.into(),
        drawing_id: "PID-1101".into(),
        mode: "协同编辑".into(),
        client_version: "test".into(),
        workspace: "plant-a".into(),
        since: None,
        repo_id: None,
        existing_tags: Vec::new(),
        protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
        resume_session_id: resume,
        credential: None,
        libraries: None,
    }
}

/// #22: Using the local TCP fault proxy (network-only disconnect while server
/// stays running), proves that reconnecting WITHOUT `resume_session_id` gets
/// blocked by the user's OWN previous session's claim (`CLAIM_HELD`).
#[tokio::test]
async fn network_only_disconnect_without_resume_blocks_own_claim() {
    let (srv_addr, _state, server) = start_server(SessionConfig {
        heartbeat_timeout: 30,
        claim_ttl: 300,
    })
    .await;
    let proxy = TcpFaultProxy::start(srv_addr).await.expect("proxy");

    // 1. Client A connects through proxy and claims P-1001.
    let mut a1 = RawWs::connect(proxy.addr()).await;
    a1.send(&hello_msg("u1", None)).await;
    let s1 = match a1.recv_msg().await {
        ServerMessage::Welcome { session_id, .. } => session_id,
        other => panic!("expected welcome, got {other:?}"),
    };
    a1.send(&ClientMessage::Claim {
        session_id: s1.clone(),
        tag: "P-1001".into(),
    })
    .await;
    assert!(matches!(
        a1.recv_msg().await,
        ServerMessage::ClaimGranted { ref tag, .. } if tag == "P-1001"
    ));

    // 2. Network-only cut (server process keeps running!).
    assert_eq!(proxy.cut_all(), 1);
    drop(a1);
    tokio::time::sleep(Duration::from_millis(80)).await;

    // 3. Client A reconnects WITHOUT `resume_session_id` before heartbeat timeout,
    // and submits an edit to P-1001 -> blocked by own prior session `s1`!
    let mut a2 = RawWs::connect(proxy.addr()).await;
    a2.send(&hello_msg("u1", None)).await;
    let s2 = match a2.recv_msg().await {
        ServerMessage::Welcome { session_id, .. } => session_id,
        other => panic!("expected welcome, got {other:?}"),
    };
    assert_ne!(s1, s2);
    a2.send(&ClientMessage::Op {
        session_id: s2,
        base_revision: 0,
        op_id: "op-offline-1".into(),
        payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa"}),
    })
    .await;
    match a2.recv_msg().await {
        ServerMessage::Error { code, holder, .. } => {
            assert_eq!(code, codes::CLAIM_HELD);
            assert_eq!(holder.as_deref(), Some(s1.as_str()));
        }
        other => panic!("expected CLAIM_HELD by own previous session, got {other:?}"),
    }

    server.abort();
}

/// #36 + #37: Network-only disconnect marks peer `disconnected` (with
/// `hold_until` and claims retained); reconnecting with `resume_session_id`
/// ends the old session immediately, transfers claims, clears the disconnected
/// state, keeps 1 roster entry, and commits offline edits!
#[tokio::test]
async fn reconnect_with_resume_transfers_claim_and_distinguishes_disconnected_vs_left() {
    let (srv_addr, state, server) = start_server(SessionConfig {
        heartbeat_timeout: 2,
        claim_ttl: 300,
    })
    .await;
    let proxy_a = TcpFaultProxy::start(srv_addr).await.expect("proxy A");

    // Peer B connects directly to server so proxy A only affects A.
    let mut b = RawWs::connect(srv_addr).await;
    b.send(&hello_msg("u2", None)).await;
    let _b_sid = match b.recv_msg().await {
        ServerMessage::Welcome { session_id, .. } => session_id,
        other => panic!("expected welcome for B, got {other:?}"),
    };

    // Peer A connects through proxy A and claims P-1001.
    let mut a1 = RawWs::connect(proxy_a.addr()).await;
    a1.send(&hello_msg("u1", None)).await;
    let s1 = match a1.recv_msg().await {
        ServerMessage::Welcome { session_id, .. } => session_id,
        other => panic!("expected welcome for A, got {other:?}"),
    };
    // B hears PeerJoined(s1).
    assert!(matches!(
        b.recv_msg().await,
        ServerMessage::PeerJoined(ref p) if p.session_id == s1
    ));

    a1.send(&ClientMessage::Claim {
        session_id: s1.clone(),
        tag: "P-1001".into(),
    })
    .await;
    assert!(matches!(a1.recv_msg().await, ServerMessage::ClaimGranted { .. }));
    assert!(matches!(b.recv_msg().await, ServerMessage::ClaimGranted { .. }));

    // #37: Cut A's network via proxy -> B receives PeerDisconnected (NOT PeerLeft) with hold_until!
    proxy_a.cut_all();
    drop(a1);
    match b.recv_msg().await {
        ServerMessage::PeerDisconnected(peer) => {
            assert_eq!(peer.session_id, s1);
            assert_eq!(peer.status.as_deref(), Some("disconnected"));
            assert!(peer.hold_until.is_some(), "hold_until must be set");
        }
        other => panic!("expected PeerDisconnected on abrupt drop, got {other:?}"),
    }
    // Claim on P-1001 is STILL held while disconnected.
    assert!(state
        .occupancy
        .lock()
        .await
        .get("PID-1101", "P-1001")
        .is_some());

    // #36: A reconnects before expiry with `resume_session_id = Some(s1)`.
    let mut a2 = RawWs::connect(proxy_a.addr()).await;
    a2.send(&hello_msg("u1", Some(s1.clone()))).await;
    let (s2, a2_claims) = match a2.recv_msg().await {
        ServerMessage::Welcome {
            session_id, claims, ..
        } => (session_id, claims),
        other => panic!("expected welcome for A2, got {other:?}"),
    };
    assert_ne!(s1, s2);
    assert_eq!(a2_claims.len(), 1);
    assert_eq!(a2_claims[0].session_id, s2, "claim transferred to s2");

    // Roster has only 1 active entry for u1 (s1 is TakenOver).
    {
        let sessions = state.sessions.lock().await;
        let active_u1: Vec<_> = sessions
            .active_peers("PID-1101")
            .into_iter()
            .filter(|s| s.user_id == "u1")
            .collect();
        assert_eq!(active_u1.len(), 1);
        assert_eq!(active_u1[0].session_id, s2);
        assert!(active_u1[0].disconnected_at.is_none());
    }

    // B receives PeerLeft(s1), ClaimGranted(s2), PeerJoined(s2) (disconnected mark cleared!).
    let mut saw_left_s1 = false;
    let mut saw_joined_s2 = false;
    for _ in 0..3 {
        match b.recv_msg().await {
            ServerMessage::PeerLeft(p) if p.session_id == s1 => saw_left_s1 = true,
            ServerMessage::PeerJoined(p) if p.session_id == s2 => {
                assert!(p.status.is_none(), "reconnected peer has no disconnected mark");
                saw_joined_s2 = true;
            }
            ServerMessage::ClaimGranted { session_id, .. } if session_id == s2 => {}
            other => panic!("unexpected frame on B: {other:?}"),
        }
    }
    assert!(saw_left_s1 && saw_joined_s2);

    // A's offline edit to P-1001 now SUCCEEDS!
    a2.send(&ClientMessage::Op {
        session_id: s2.clone(),
        base_revision: 0,
        op_id: "op-resumed-1".into(),
        payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "2.5MPa"}),
    })
    .await;
    loop {
        match a2.recv_msg().await {
            ServerMessage::OpApplied { revision: 1, .. } => break,
            ServerMessage::PeerLeft(_) | ServerMessage::ClaimGranted { .. } => continue,
            other => panic!("expected OpApplied(1) on resumed session A2, got {other:?}"),
        }
    }
    // The edit renews the held claim (#87), so ClaimGranted can sit ahead of
    // the disconnect on B's socket.
    let mut saw_applied = false;
    let mut saw_disconnected = false;
    for _ in 0..4 {
        match b.recv_msg().await {
            ServerMessage::OpApplied { revision: 1, .. } => saw_applied = true,
            ServerMessage::ClaimGranted { tag, .. } if tag == "P-1001" => {}
            other if !saw_applied => panic!("expected OpApplied(1) on B, got {other:?}"),
            other => panic!("expected PeerDisconnected after the resumed edit, got {other:?}"),
        }
        if saw_applied {
            break;
        }
    }
    assert!(saw_applied, "B must hear the resumed edit");

    // #37 part 2: Now cut A2 and do NOT reconnect -> after heartbeat_timeout (2s),
    // B receives PeerDisconnected first, then ClaimReleased(expired) + PeerLeft(s2)!
    proxy_a.cut_all();
    drop(a2);
    for _ in 0..4 {
        match b.recv_msg().await {
            ServerMessage::ClaimGranted { tag, .. } if tag == "P-1001" => {}
            ServerMessage::PeerDisconnected(peer) if peer.session_id == s2 => {
                saw_disconnected = true;
                break;
            }
            other => panic!("expected PeerDisconnected on second cut, got {other:?}"),
        }
    }
    assert!(saw_disconnected, "B must hear PeerDisconnected for the resumed session");
    let mut saw_released = false;
    let mut saw_left_s2 = false;
    for _ in 0..2 {
        match b.recv_msg().await {
            ServerMessage::ClaimReleased { tag, reason } if tag == "P-1001" && reason == "expired" => {
                saw_released = true;
            }
            ServerMessage::PeerLeft(p) if p.session_id == s2 => saw_left_s2 = true,
            other => panic!("expected ClaimReleased or PeerLeft after timeout, got {other:?}"),
        }
    }
    assert!(saw_released && saw_left_s2);

    // Proxy latency control sanity check (#22).
    proxy_a.set_latency(Duration::from_millis(5));
    proxy_a.drop_next_chunks(0);
    proxy_a.set_accepting(false);
    proxy_a.set_accepting(true);

    server.abort();
}

//! End-to-end HTTPS + WSS TLS and non-loopback `--bind` tests (#69).

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use ocs_collab::{ClientMessage, ServerMessage, SessionConfig};
use ocs_collab_server::http;
use ocs_collab_server::store::{SeedDoc, Store};
use ocs_collab_server::tls::{load_server_config, TlsListener};
use ocs_collab_server::AppState;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde_json::json;
use tokio::net::TcpListener;

fn generate_self_signed_cert(
    dir: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let ca_key = dir.join("ca.key");
    let ca_cert = dir.join("ca.crt");
    let srv_key = dir.join("server.key");
    let srv_csr = dir.join("server.csr");
    let srv_cert = dir.join("server.crt");
    let ext_file = dir.join("ext.cnf");
    std::fs::write(
        &ext_file,
        "basicConstraints=critical,CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n",
    )
    .expect("write ext.cnf");

    // 1. Self-signed root CA
    let s1 = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            ca_key.to_str().expect("ca_key"),
            "-out",
            ca_cert.to_str().expect("ca_cert"),
            "-days",
            "2",
            "-subj",
            "/CN=OCS-Test-CA",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
        ])
        .status()
        .expect("openssl ca");
    assert!(s1.success(), "openssl CA generation failed");

    // 2. Server leaf CSR + CA-signed certificate (CA:FALSE, SAN=localhost,127.0.0.1)
    let s2 = Command::new("openssl")
        .args([
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            srv_key.to_str().expect("srv_key"),
            "-out",
            srv_csr.to_str().expect("srv_csr"),
            "-subj",
            "/CN=localhost",
        ])
        .status()
        .expect("openssl csr");
    assert!(s2.success(), "openssl CSR generation failed");

    let s3 = Command::new("openssl")
        .args([
            "x509",
            "-req",
            "-in",
            srv_csr.to_str().expect("srv_csr"),
            "-CA",
            ca_cert.to_str().expect("ca_cert"),
            "-CAkey",
            ca_key.to_str().expect("ca_key"),
            "-CAcreateserial",
            "-out",
            srv_cert.to_str().expect("srv_cert"),
            "-days",
            "2",
            "-extfile",
            ext_file.to_str().expect("ext_file"),
        ])
        .status()
        .expect("openssl sign");
    assert!(s3.success(), "openssl leaf sign failed");

    (ca_cert, srv_cert, srv_key)
}

fn tls_client_config(cert_path: &std::path::Path) -> Arc<ClientConfig> {
    let cert_bytes = std::fs::read(cert_path).expect("read cert");
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&cert_bytes) {
        roots.add(cert.expect("valid cert")).expect("add root");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(cfg)
}

struct TlsWsClient {
    stream: StreamOwned<ClientConnection, TcpStream>,
}

impl TlsWsClient {
    fn connect(addr: SocketAddr, client_cfg: Arc<ClientConfig>) -> Self {
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("tcp connect");
        tcp.set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let server_name = ServerName::try_from(IpAddr::V4(Ipv4Addr::LOCALHOST)).expect("sni");
        let conn = ClientConnection::new(client_cfg, server_name).expect("tls client conn");
        let mut tls = StreamOwned::new(conn, tcp);

        let req = format!(
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        tls.write_all(req.as_bytes()).expect("ws upgrade write");
        tls.flush().expect("ws upgrade flush");

        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            tls.read_exact(&mut byte).expect("ws upgrade head");
            head.push(byte[0]);
        }
        let head_str = String::from_utf8_lossy(&head);
        assert!(head_str.starts_with("HTTP/1.1 101"), "{head_str}");
        Self { stream: tls }
    }

    fn send(&mut self, msg: &ClientMessage) {
        let text = serde_json::to_string(msg).expect("serialize");
        let payload = text.as_bytes();
        let mask = [0x31, 0x41, 0x59, 0x26];
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
        self.stream.write_all(&frame).expect("write frame");
        self.stream.flush().expect("flush frame");
    }

    fn recv_msg(&mut self) -> ServerMessage {
        loop {
            let mut head = [0u8; 2];
            self.stream.read_exact(&mut head).expect("read head");
            let opcode = head[0] & 0x0f;
            let len = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    self.stream.read_exact(&mut b).expect("len16");
                    usize::from(u16::from_be_bytes(b))
                }
                127 => {
                    let mut b = [0u8; 8];
                    self.stream.read_exact(&mut b).expect("len64");
                    usize::try_from(u64::from_be_bytes(b)).expect("len64 fits")
                }
                n => usize::from(n),
            };
            let mut payload = vec![0u8; len];
            self.stream.read_exact(&mut payload).expect("read payload");
            if opcode == 0x1 {
                return serde_json::from_slice(&payload).expect("valid ServerMessage");
            }
        }
    }
}

fn https_get(addr: SocketAddr, path: &str, client_cfg: Arc<ClientConfig>) -> String {
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("tcp connect");
    tcp.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let server_name = ServerName::try_from(IpAddr::V4(Ipv4Addr::LOCALHOST)).expect("sni");
    let conn = ClientConnection::new(client_cfg, server_name).expect("tls conn");
    let mut tls = StreamOwned::new(conn, tcp);
    write!(
        tls,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .expect("write https");
    tls.flush().expect("flush https");
    let mut resp = String::new();
    let _ = tls.read_to_string(&mut resp);
    resp
}

/// #69: Server binds non-loopback (`0.0.0.0:0`) with `--require-auth` and a
/// self-signed TLS certificate; two clients connect over HTTPS/WSS,
/// authenticate with credentials, and see each other's claim and property edits.
#[tokio::test]
async fn tls_https_and_wss_non_loopback_collaboration_with_self_signed_cert() {
    let dir = std::env::temp_dir().join(format!("ocs-tls-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let (ca_path, cert_path, key_path) = generate_self_signed_cert(&dir);
    let server_tls = load_server_config(&cert_path, &key_path).expect("load server tls");
    let client_tls = tls_client_config(&ca_path);

    let store = Store::open_in_memory().expect("store");
    let seed: SeedDoc = serde_json::from_value(json!({
        "users": [
            {"user_id": "u1", "display_name": "张工", "role": "工艺设计人员", "credential": "pass-u1"},
            {"user_id": "u2", "display_name": "李工", "role": "工艺设计人员", "credential": "pass-u2"}
        ],
        "pbs_nodes": [{"node_id": "n-plant", "name": "全厂", "node_type": "项目"}],
        "pbs_acl": [
            {"node_id": "n-plant", "subject": "工艺设计人员", "permission": "编辑", "allowed": true}
        ],
        "drawings": [
            {"drawing_id": "PID-1101", "file_name": "PID-1101.dwg", "pbs_node_id": "n-plant", "state": "协同编辑"}
        ]
    }))
    .expect("seed");
    store.seed(&seed).expect("apply seed");

    let mut state = AppState::new(Some(store), None, ":memory:", SessionConfig::default())
        .with_require_auth(true);
    state.tls_enabled = true;
    let state = Arc::new(state);

    // Bind non-loopback 0.0.0.0:0 with TLS!
    let tcp_listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0)))
        .await
        .expect("bind 0.0.0.0");
    let port = tcp_listener.local_addr().expect("local addr").port();
    let connect_addr = SocketAddr::from(([127, 0, 0, 1], port));
    let tls_listener = TlsListener::new(tcp_listener, server_tls);
    let router = http::router(state);
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            tls_listener,
            router.into_make_service_with_connect_info::<ocs_collab_server::tls::PeerAddr>(),
        )
        .await;
    });

    let client_tls_clone = client_tls.clone();
    tokio::task::spawn_blocking(move || {
        // 1. HTTPS GET /health reports tls_enabled: true & auth_enabled: true.
        let health = https_get(connect_addr, "/health", client_tls_clone.clone());
        assert!(health.starts_with("HTTP/1.1 200"), "{health}");
        assert!(health.contains("\"tls_enabled\":true"), "{health}");
        assert!(health.contains("\"auth_enabled\":true"), "{health}");

        // 2. Two WSS clients authenticate and join PID-1101.
        let mut c1 = TlsWsClient::connect(connect_addr, client_tls_clone.clone());
        c1.send(&ClientMessage::Hello {
            user_id: "u1".into(),
            drawing_id: "PID-1101".into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "lan".into(),
            since: None,
            repo_id: None,
            existing_tags: Vec::new(),
            protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
            resume_session_id: None,
            credential: Some("pass-u1".into()),
        });
        let s1 = match c1.recv_msg() {
            ServerMessage::Welcome {
                session_id,
                display_name,
                ..
            } => {
                assert_eq!(display_name, "张工");
                session_id
            }
            other => panic!("expected welcome for c1, got {other:?}"),
        };

        let mut c2 = TlsWsClient::connect(connect_addr, client_tls_clone);
        c2.send(&ClientMessage::Hello {
            user_id: "u2".into(),
            drawing_id: "PID-1101".into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: "lan".into(),
            since: None,
            repo_id: None,
            existing_tags: Vec::new(),
            protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
            resume_session_id: None,
            credential: Some("pass-u2".into()),
        });
        let _s2 = match c2.recv_msg() {
            ServerMessage::Welcome {
                session_id, peers, ..
            } => {
                assert_eq!(peers.len(), 1);
                assert_eq!(peers[0].display_name, "张工");
                session_id
            }
            other => panic!("expected welcome for c2, got {other:?}"),
        };
        assert!(matches!(c1.recv_msg(), ServerMessage::PeerJoined(_)));

        // 3. c1 claims P-1001 -> c2 sees ClaimGranted broadcast over WSS!
        c1.send(&ClientMessage::Claim {
            session_id: s1.clone(),
            tag: "P-1001".into(),
        });
        assert!(matches!(
            c1.recv_msg(),
            ServerMessage::ClaimGranted { ref tag, .. } if tag == "P-1001"
        ));
        assert!(matches!(
            c2.recv_msg(),
            ServerMessage::ClaimGranted { ref tag, .. } if tag == "P-1001"
        ));

        // 4. c1 edits P-1001 -> c2 receives OpApplied over WSS!
        c1.send(&ClientMessage::Op {
            session_id: s1,
            base_revision: 0,
            op_id: "op-tls-1".into(),
            payload: json!({"kind": "set_property", "object_ref": "P-1001", "field": "设计压力", "value": "3.2MPa"}),
        });
        assert!(matches!(
            c1.recv_msg(),
            ServerMessage::OpApplied { revision: 1, .. }
        ));
        assert!(matches!(
            c2.recv_msg(),
            ServerMessage::OpApplied { revision: 1, .. }
        ));
    })
    .await
    .expect("tls client task");

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

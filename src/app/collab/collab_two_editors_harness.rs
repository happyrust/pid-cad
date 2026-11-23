//! Real-server + two-headless-editors test harness (#20, #33, #41, #78).
//!
//! Spawns the real `ocs_collab_server` binary on an ephemeral port with a
//! seeded SQLite database, and connects two (or more) windowless
//! `OpenCADStudio::new_for_test()` editors, each running its own real
//! synchronous `transport::run_client` thread over an isolated
//! `transport::Link`. Zero mocks or stand-ins between client and server, and
//! zero async runtime in the desktop crate (`VAL-CLI-019`).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc;
use serde_json::{json, Value};

use crate::app::collab::{transport, CollabEvent};
use crate::app::OpenCADStudio;
use crate::pid::persist::PidAuthor;
use crate::pid::propagate::split_props;

static SERIAL: AtomicU64 = AtomicU64::new(1);

fn next_serial() -> u64 {
    SERIAL.fetch_add(1, Ordering::Relaxed)
}

/// Locate or build the workspace's `ocs_collab_server` binary once per test run.
fn server_binary() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let exe_name = if cfg!(windows) {
            "ocs_collab_server.exe"
        } else {
            "ocs_collab_server"
        };
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let candidate = manifest_dir.join("target").join("debug").join(exe_name);
        if candidate.is_file() {
            return candidate;
        }
        // Also check sibling directory of the current test executable (handles
        // custom CARGO_TARGET_DIR).
        if let Ok(cur) = std::env::current_exe() {
            if let Some(debug_dir) = cur.parent().and_then(|p| p.parent()) {
                let alt = debug_dir.join(exe_name);
                if alt.is_file() {
                    return alt;
                }
            }
        }
        let status = Command::new("cargo")
            .args([
                "build",
                "--offline",
                "--locked",
                "-p",
                "ocs_collab_server",
            ])
            .current_dir(&manifest_dir)
            .status()
            .expect("spawn cargo build for ocs_collab_server");
        assert!(
            status.success(),
            "cargo build -p ocs_collab_server failed with {status}"
        );
        assert!(
            candidate.is_file(),
            "ocs_collab_server binary missing at {}",
            candidate.display()
        );
        candidate
    })
    .clone()
}

/// A real `ocs_collab_server` child process bound to a free localhost port.
pub(crate) struct RealServer {
    child: Child,
    pub port: u16,
    db_path: PathBuf,
    seed_path: PathBuf,
}

impl RealServer {
    /// Start `ocs_collab_server` with the given seed JSON, tie window (ms), and
    /// claim TTL (seconds). Waits until `GET /health` returns 200 OK.
    pub(crate) fn start(seed: Value, tie_window_ms: u64, claim_ttl: u64) -> Self {
        let bin = server_binary();
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind ephemeral port")
            .local_addr()
            .expect("local addr")
            .port();
        let id = next_serial();
        let pid = std::process::id();
        let tmp = std::env::temp_dir();
        let db_path = tmp.join(format!("ocs-collab-harness-{pid}-{id}.db"));
        let seed_path = tmp.join(format!("ocs-collab-harness-{pid}-{id}.seed.json"));
        std::fs::write(&seed_path, seed.to_string()).expect("write seed file");

        let mut cmd = Command::new(&bin);
        cmd.args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--db",
            &db_path.to_string_lossy(),
            "--seed",
            &seed_path.to_string_lossy(),
            "--claim-ttl",
            &claim_ttl.to_string(),
            "--tie-window-ms",
            &tie_window_ms.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());

        let child = cmd.spawn().expect("spawn ocs_collab_server");
        let server = Self {
            child,
            port,
            db_path,
            seed_path,
        };
        server.wait_healthy(Duration::from_secs(10));
        server
    }

    pub(crate) fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/ws", self.port)
    }

    pub(crate) fn http_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn wait_healthy(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let url = format!("{}/health", self.http_url());
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(500)))
            .http_status_as_error(false)
            .build()
            .new_agent();
        while Instant::now() < deadline {
            if let Ok(resp) = agent.get(&url).call() {
                if resp.status().as_u16() == 200 {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("ocs_collab_server on port {} did not become healthy", self.port);
    }

    /// Query `GET /audit` directly on the real server.
    pub(crate) fn audit_items(&self, query: &str) -> Vec<Value> {
        let url = if query.is_empty() {
            format!("{}/audit", self.http_url())
        } else {
            format!("{}/audit?{query}", self.http_url())
        };
        let mut resp = ureq::get(&url).call().expect("GET /audit");
        let text = resp.body_mut().read_to_string().expect("read /audit body");
        let body: Value = serde_json::from_str(&text).expect("parse /audit json");
        body["items"].as_array().cloned().unwrap_or_default()
    }
}

impl Drop for RealServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.seed_path);
        let _ = std::fs::remove_file(&self.db_path);
        let _ = std::fs::remove_file(format!("{}-wal", self.db_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", self.db_path.display()));
    }
}

/// One headless `OpenCADStudio` editor connected to a [`RealServer`] via its
/// own real synchronous `transport::run_client` thread.
pub(crate) struct HeadlessClient {
    pub app: OpenCADStudio,
    rx: mpsc::Receiver<CollabEvent>,
    link: transport::Link,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl HeadlessClient {
    pub(crate) fn connect(ws_url: &str, user_id: &str, drawing_id: &str) -> Self {
        let mut app = OpenCADStudio::new_for_test();
        let _ = app.automation_op(r#"{"op":"new"}"#);
        app.collab.server_url = ws_url.to_owned();
        app.collab.user_id = user_id.to_owned();
        app.collab.drawing_id = drawing_id.to_owned();
        app.collab.drawing_given = true;
        app.collab.opens_settled = true;
        app.collab.document = Some(app.tabs[app.active_tab].id);
        let link = app.collab.attach_detached_link();
        let (mut tx, rx) = mpsc::channel::<CollabEvent>(128);
        let thread_link = link.clone();
        let worker = std::thread::spawn(move || {
            let _ = transport::run_client(&mut tx, &thread_link);
        });
        Self {
            app,
            rx,
            link,
            worker: Some(worker),
        }
    }

    /// Drain every event currently ready from the transport thread into the
    /// editor (equivalent to iced's subscription dispatch).
    pub(crate) fn pump(&mut self) -> usize {
        let mut count = 0;
        while let Ok(event) = self.rx.try_recv() {
            let task = self.app.update(crate::app::Message::CollabEvent(event));
            let _ = self.app.drive_headless_task(task);
            count += 1;
        }
        self.app.answer_collab_tags();
        count
    }

    /// Pump until `pred(&self.app)` holds or `timeout` elapses.
    pub(crate) fn wait_until<F>(&mut self, timeout: Duration, mut pred: F) -> bool
    where
        F: FnMut(&OpenCADStudio) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump();
            if pred(&self.app) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) fn run_cmd(&mut self, cmd: &str) -> Value {
        let serial = next_serial();
        let req = json!({
            "op": "run",
            "cmd": cmd,
            "document_id": self.app.tabs[self.app.active_tab].id,
            "request_id": format!("harness-cmd-{serial}"),
        });
        let (resp, task) = self.app.control_request(req);
        let _ = self.app.drive_headless_task(task);
        self.app.finish_all_pending_history();
        resp
    }

    pub(crate) fn draw_points(&mut self, cmd: &str, points: &[[f64; 2]]) -> Value {
        let started = self.run_cmd(cmd);
        assert_eq!(started["ok"], true, "{cmd}: {started}");
        for pt in points {
            let serial = next_serial();
            let req = json!({
                "op": "input",
                "kind": "point",
                "point": [pt[0], pt[1], 0.0],
                "space": "wcs",
                "document_id": self.app.tabs[self.app.active_tab].id,
                "request_id": format!("harness-pt-{serial}"),
            });
            let (resp, task) = self.app.control_request(req);
            let _ = self.app.drive_headless_task(task);
            assert_eq!(resp["ok"], true, "{cmd} point {pt:?}: {resp}");
        }
        let serial = next_serial();
        let req = json!({
            "op": "input",
            "kind": "enter",
            "document_id": self.app.tabs[self.app.active_tab].id,
            "request_id": format!("harness-enter-{serial}"),
        });
        let (done, task) = self.app.control_request(req);
        let _ = self.app.drive_headless_task(task);
        self.app.finish_all_pending_history();
        done
    }

    pub(crate) fn read_prop(&self, tag: &str, field: &str) -> Option<String> {
        let doc = &self.app.tabs[self.app.active_tab].scene.document;
        for entity in doc.entities() {
            let Some(author) = PidAuthor::load(doc, entity.common().handle) else {
                continue;
            };
            if author.tag == tag {
                let (props, _) = split_props(&author.props);
                if let Some(val) = props.get(field) {
                    return Some(val.clone());
                }
            }
        }
        None
    }

    pub(crate) fn read_all_props(&self, tag: &str) -> std::collections::BTreeMap<String, String> {
        let doc = &self.app.tabs[self.app.active_tab].scene.document;
        let mut out = std::collections::BTreeMap::new();
        for entity in doc.entities() {
            let Some(author) = PidAuthor::load(doc, entity.common().handle) else {
                continue;
            };
            if author.tag == tag {
                let (props, _) = split_props(&author.props);
                out.extend(props);
            }
        }
        out
    }
}

impl Drop for HeadlessClient {
    fn drop(&mut self) {
        self.link.request_stop();
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !worker.is_finished() && Instant::now() < deadline {
                let _ = self.rx.try_recv();
                std::thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

/// Pump two clients in lockstep until `pred(&a.app, &b.app)` holds.
pub(crate) fn wait_both<F>(
    a: &mut HeadlessClient,
    b: &mut HeadlessClient,
    timeout: Duration,
    mut pred: F,
) -> bool
where
    F: FnMut(&OpenCADStudio, &OpenCADStudio) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        a.pump();
        b.pump();
        if pred(&a.app, &b.app) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Standard seed covering 甲 (u1, 工艺设计人员), 乙 (u2, 工艺设计人员),
/// 丙 (u3, 校对人员), 丁 (u4, 项目管理员), and 戊 (u5, explicitly denied on PBS node).
pub(crate) fn standard_seed() -> Value {
    json!({
        "users": [
            {"user_id": "u1", "display_name": "设计甲", "role": "工艺设计人员", "company": "一院"},
            {"user_id": "u2", "display_name": "设计乙", "role": "工艺设计人员", "company": "一院"},
            {"user_id": "u3", "display_name": "校对丙", "role": "校对人员", "company": "一院"},
            {"user_id": "u4", "display_name": "管理丁", "role": "项目管理员", "company": "一院"},
            {"user_id": "u5", "display_name": "受限戊", "role": "工艺设计人员", "company": "一院"}
        ],
        "pbs_nodes": [
            {"node_id": "n-root", "name": "装置一", "node_type": "装置", "parent_id": null},
            {"node_id": "n-area", "name": "单元A", "node_type": "区域", "parent_id": "n-root"}
        ],
        "pbs_acl": [
            {"node_id": "n-root", "subject": "工艺设计人员", "permission": "编辑", "allowed": true},
            {"node_id": "n-root", "subject": "项目管理员", "permission": "编辑", "allowed": true},
            {"node_id": "n-root", "subject": "校对人员", "permission": "查看", "allowed": true},
            {"node_id": "n-area", "subject": "u5", "permission": "编辑", "allowed": false}
        ],
        "drawings": [
            {"drawing_id": "PID-1101", "file_name": "PID-1101.dwg", "pbs_node_id": "n-area", "state": "协同编辑"},
            {"drawing_id": "PID-1102", "file_name": "PID-1102.dwg", "pbs_node_id": "n-area", "state": "协同编辑"},
            {"drawing_id": "PID-1103", "file_name": "PID-1103.dwg", "pbs_node_id": "n-area", "state": "协同编辑"}
        ]
    })
}

/// Place a pump symbol on `client` and wait for it to reach `tag`.
fn place_pump(client: &mut HeadlessClient) -> String {
    let before: Vec<String> = {
        let doc = &client.app.tabs[client.app.active_tab].scene.document;
        doc.entities()
            .filter_map(|e| PidAuthor::load(doc, e.common().handle).map(|a| a.tag))
            .collect()
    };
    client.draw_points("PIDINSERT equipment", &[[100.0, 100.0]]);
    let doc = &client.app.tabs[client.app.active_tab].scene.document;
    doc.entities()
        .filter_map(|e| PidAuthor::load(doc, e.common().handle).map(|a| a.tag))
        .find(|tag| !tag.is_empty() && !before.contains(tag))
        .expect("placed equipment must carry a tag")
}

// ── #20: First 4 real-server + two-headless-editors tests ────────────────

#[test]
fn harness_20_t1_designer_a_edits_property_and_designer_b_reads_it() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1101");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.connected && b.collab.connected && a.collab.peers.len() == 1
        }),
        "both editors must connect and see each other"
    );

    let tag = place_pump(&mut a);
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.base_revision >= 1 && b.collab.base_revision >= 1
        }),
        "placed pump must replicate to B"
    );

    let res = a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 2.5MPa"));
    assert_eq!(res["ok"], true, "PIDSETPROP on A: {res}");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.base_revision >= 2 && b.collab.base_revision >= 2
        }),
        "property write must reach B"
    );
    assert_eq!(b.read_prop(&tag, "设计压力").as_deref(), Some("2.5MPa"));
}

#[test]
fn harness_20_t2_reviewer_write_is_refused_and_audit_gains_a_row() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut c = HeadlessClient::connect(&server.ws_url(), "u3", "PID-1101");
    assert!(
        wait_both(&mut a, &mut c, Duration::from_secs(5), |a, c| {
            a.collab.connected && c.collab.connected && c.collab.readonly
        }),
        "reviewer u3 must join and be downgraded to readonly"
    );
    assert_eq!(c.app.collab.readonly_level.as_deref(), Some("role"));

    let tag = place_pump(&mut a);
    assert!(wait_both(&mut a, &mut c, Duration::from_secs(5), |a, c| {
        a.collab.base_revision >= 1 && c.collab.base_revision >= 1
    }));

    let audit_before = server.audit_items("").len();
    let refused = c.run_cmd(&format!("PIDSETPROP {tag} 设计压力 9.9MPa"));
    assert_eq!(refused["ok"], false, "reviewer write must be refused");
    // Wait for the client's `attempt` frame to be audited by the server.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut audit_after = audit_before;
    while Instant::now() < deadline {
        c.pump();
        audit_after = server.audit_items("").len();
        if audit_after > audit_before {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        audit_after > audit_before,
        "audit log must gain a row for reviewer's refused write (before={audit_before}, after={audit_after})"
    );
    assert_ne!(c.read_prop(&tag, "设计压力").as_deref(), Some("9.9MPa"));
}

#[test]
fn harness_20_t3_claimed_tag_refuses_peer_write_and_rolls_it_back() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1101");
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.connected && b.collab.connected
    }));

    let tag = place_pump(&mut a);
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.base_revision >= 1 && b.collab.base_revision >= 1
    }));
    a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 1.0MPa"));
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.base_revision >= 2 && b.collab.base_revision >= 2
    }));
    assert_eq!(b.read_prop(&tag, "设计压力").as_deref(), Some("1.0MPa"));

    // A claims the tag.
    let _ = a.run_cmd(&format!("PIDCLAIM {tag}"));
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab.claims.get(&tag).map(String::as_str) == Some("u1")
        }),
        "A must receive claim grant"
    );

    // B writes to A's claimed tag -> server refuses with CLAIM_HELD and B rolls back locally!
    let _ = b.run_cmd(&format!("PIDSETPROP {tag} 设计压力 8.8MPa"));
    assert!(
        b.wait_until(Duration::from_secs(5), |app| {
            !app.collab.rejection_log.is_empty()
                && app
                    .collab
                    .rejection_log
                    .iter()
                    .any(|r| r["code"] == "CLAIM_HELD")
        }),
        "B's write to A's claimed tag must be rejected with CLAIM_HELD; log={:?}",
        b.app.collab.rejection_log
    );
    assert_eq!(
        b.read_prop(&tag, "设计压力").as_deref(),
        Some("1.0MPa"),
        "B must roll back its rejected edit to the room's value"
    );
}

#[test]
fn harness_20_t4_pbs_denied_user_joins_as_readonly() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut denied = HeadlessClient::connect(&server.ws_url(), "u5", "PID-1101");
    assert!(
        denied.wait_until(Duration::from_secs(5), |app| {
            app.collab.connected && app.collab.readonly
        }),
        "user u5 explicitly denied on PBS node n-area must join as readonly"
    );
    assert_eq!(denied.app.collab.readonly_level.as_deref(), Some("pbs"));
    assert_eq!(
        denied.app.collab.readonly_code.as_deref(),
        Some("PERM_DENIED_PBS")
    );
    // #52: Typed effective_node flows from server ReadonlyDowngrade into client state & 3-tier UI.
    assert_eq!(
        denied.app.collab.readonly_effective_node.as_deref(),
        Some("n-area")
    );
    let tiers = crate::app::pid_shell::collab_ui::readonly_tiers(&denied.app.collab);
    assert!(
        tiers[1]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("n-area")),
        "PBS tier detail must cite effective node n-area; got {:?}",
        tiers
    );
}

// ── #33: Signal-line cross-drawing page connector scenario ───────────────

#[test]
fn harness_33_signal_line_cross_drawing_connector_propagates_instruments_without_pipe_grade() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut upstream = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut downstream = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1102");
    assert!(
        wait_both(&mut upstream, &mut downstream, Duration::from_secs(5), |a, b| {
            a.collab.connected && b.collab.connected
        }),
        "both drawing sessions must connect"
    );

    // Draw signal line S-2001 on both drawings.
    upstream.draw_points("PIDPIPE signal S-2001", &[[0.0, 0.0], [100.0, 0.0]]);
    downstream.draw_points("PIDPIPE signal S-2001", &[[0.0, 0.0], [100.0, 0.0]]);
    assert!(wait_both(
        &mut upstream,
        &mut downstream,
        Duration::from_secs(5),
        |a, b| a.collab.base_revision >= 1 && b.collab.base_revision >= 1
    ));

    // Set signal fields + a pipe-only field on upstream S-2001.
    upstream.run_cmd("PIDSETPROP S-2001 起点仪表 PT-101");
    upstream.run_cmd("PIDSETPROP S-2001 终点仪表 PIC-101");
    upstream.run_cmd("PIDSETPROP S-2001 管道等级 A1A");
    assert!(upstream.wait_until(Duration::from_secs(5), |a| a.collab.outbox.is_empty()));

    // Place outflow signal connector on PID-1101 targeting PID-1102, and inflow on PID-1102.
    upstream.draw_points("PIDCONNECTOR signal_out PID-1102", &[[100.0, 0.0]]);
    downstream.draw_points("PIDCONNECTOR signal_in PID-1101", &[[0.0, 0.0]]);

    // Wait for server to pair and propagate signal fields to downstream PID-1102.
    assert!(
        wait_both(
            &mut upstream,
            &mut downstream,
            Duration::from_secs(5),
            |_, b| {
                let doc = &b.tabs[b.active_tab].scene.document;
                doc.entities().any(|e| {
                    let Some(author) = PidAuthor::load(doc, e.common().handle) else {
                        return false;
                    };
                    if author.tag != "S-2001" {
                        return false;
                    }
                    let (props, _) = split_props(&author.props);
                    props.get("起点仪表").map(String::as_str) == Some("PT-101")
                        && props.get("终点仪表").map(String::as_str) == Some("PIC-101")
                })
            }
        ),
        "downstream S-2001 must receive 起点仪表 and 终点仪表; got {:?}",
        downstream.read_all_props("S-2001")
    );

    let ds_props = downstream.read_all_props("S-2001");
    assert_eq!(ds_props.get("起点仪表").map(String::as_str), Some("PT-101"));
    assert_eq!(ds_props.get("终点仪表").map(String::as_str), Some("PIC-101"));
    assert!(
        !ds_props.contains_key("管道等级"),
        "signal connector must NOT propagate pipe-only field 管道等级; got {ds_props:?}"
    );
}

// ── #40: Query channel stub + join/reconnect pending conflict fetch ──────

#[derive(Debug)]
struct StubQueryClient {
    conflicts: Vec<crate::app::collab::PendingConflict>,
}

impl crate::app::collab::query::CollabQueryClient for StubQueryClient {
    fn fetch_pending_conflicts(
        &self,
        _base_url: &str,
        _drawing_id: &str,
    ) -> Result<Vec<crate::app::collab::PendingConflict>, crate::app::collab::query::QueryError> {
        Ok(self.conflicts.clone())
    }

    fn resolve_conflict(
        &self,
        _base_url: &str,
        _conflict_id: &str,
        _resolution: &str,
        _resolved_by: &str,
        _value: Option<&str>,
    ) -> Result<crate::app::collab::query::ResolveOutcome, crate::app::collab::query::QueryError> {
        Err(crate::app::collab::query::QueryError::transport("stub"))
    }

    fn fetch_audit(
        &self,
        _base_url: &str,
        _filter: &crate::app::collab::query::AuditFilter,
    ) -> Result<Vec<crate::app::collab::query::AuditItem>, crate::app::collab::query::QueryError> {
        Ok(Vec::new())
    }

    fn fetch_drawing_objects(
        &self,
        _base_url: &str,
        drawing_id: &str,
        _object_ref: Option<&str>,
    ) -> Result<
        crate::app::collab::query::DrawingObjectsPage,
        crate::app::collab::query::QueryError,
    > {
        Ok(crate::app::collab::query::DrawingObjectsPage {
            drawing_id: drawing_id.to_owned(),
            revision: 0,
            items: Vec::new(),
        })
    }
}

#[test]
fn query_40_stub_fetches_pending_conflicts_on_join_and_reconnect_without_server() {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = "PID-1101".to_owned();
    let expected = vec![crate::app::collab::PendingConflict {
        conflict_id: "c-stub-1".to_owned(),
        object_ref: "P-1001".to_owned(),
        field: "设计压力".to_owned(),
        value_a: "1.6MPa".to_owned(),
        value_b: "2.5MPa".to_owned(),
    }];
    app.collab.set_query_client(std::sync::Arc::new(StubQueryClient {
        conflicts: expected.clone(),
    }));

    let task = app.update(crate::app::Message::CollabEvent(CollabEvent::Connected {
        session_id: "s-1".to_owned(),
        base_revision: 3,
        repo_id: "r-stub".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    }));
    let _ = app.drive_headless_task(task);
    assert_eq!(app.collab.conflicts, expected);
    assert_eq!(app.collab.status_json()["conflicts"][0]["conflict_id"], "c-stub-1");

    // Disconnect and reconnect: conflicts remain fetched and identical.
    let _ = app.update(crate::app::Message::CollabEvent(CollabEvent::Disconnected {
        reason: "drop".into(),
        will_retry: true,
    }));
    let task2 = app.update(crate::app::Message::CollabEvent(CollabEvent::Connected {
        session_id: "s-2".to_owned(),
        base_revision: 3,
        repo_id: "r-stub".into(),
        peers: Vec::new(),
        claims: Vec::new(),
    }));
    let _ = app.drive_headless_task(task2);
    assert_eq!(app.collab.conflicts, expected);
}

// ── #41 + #40: Real server conflict tie, late joiner query, client resolve ──

#[test]
fn harness_40_41_conflict_tie_late_joiner_fetches_and_admin_resolves_from_client() {
    // Tie window 800ms so A and B editing the same field at the same baseline tie.
    let server = RealServer::start(standard_seed(), 800, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1101");
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.connected && b.collab.connected
    }));

    let tag = place_pump(&mut a);
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(6), |a, b| {
        a.collab.base_revision >= 1 && b.collab.base_revision >= 1
    }));

    // Both A and B edit the same field concurrently against revision 1.
    a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 1.6MPa"));
    b.run_cmd(&format!("PIDSETPROP {tag} 设计压力 2.5MPa"));

    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(6), |a, b| {
            !a.collab.conflicts.is_empty() && !b.collab.conflicts.is_empty()
        }),
        "both A and B must receive the tied conflict entry"
    );
    let conflict_id = a.app.collab.conflicts[0].conflict_id.clone();

    // #40: Late joiner D (项目管理员) joins AFTER the conflict happened and
    // automatically fetches the pending conflict via HTTP query channel!
    let mut d = HeadlessClient::connect(&server.ws_url(), "u4", "PID-1101");
    assert!(
        d.wait_until(Duration::from_secs(5), |app| {
            app.collab.connected
                && app
                    .collab
                    .conflicts
                    .iter()
                    .any(|c| c.conflict_id == conflict_id)
        }),
        "late joiner D must fetch pending conflict {conflict_id} on join; got {:?}",
        d.app.collab.conflicts
    );

    // #41: Reviewer C (校对人员) tries to resolve -> refused at role tier!
    let mut c = HeadlessClient::connect(&server.ws_url(), "u3", "PID-1101");
    assert!(c.wait_until(Duration::from_secs(5), |app| app.collab.connected));
    c.run_cmd(&format!("PIDCOLLAB RESOLVE {conflict_id} 采用甲方"));
    let c_tail = c.app.command_line.history_plain_text();
    assert!(
        c_tail.contains("PERM_DENIED_ROLE") || c_tail.contains("角色权限"),
        "reviewer resolve must explain role denial; got: {c_tail}"
    );

    // #41: Admin D resolves with 人工改值 3.2MPa -> both A & B converge, conflict vanishes, audit +1!
    let audit_before = server.audit_items("").len();
    d.run_cmd(&format!("PIDCOLLAB RESOLVE {conflict_id} 人工改值 3.2MPa"));
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(6), |a, b| {
            a.collab.conflicts.is_empty() && b.collab.conflicts.is_empty()
        }),
        "conflict must disappear on both A and B after D resolves"
    );
    assert_eq!(a.read_prop(&tag, "设计压力").as_deref(), Some("3.2MPa"));
    assert_eq!(b.read_prop(&tag, "设计压力").as_deref(), Some("3.2MPa"));
    assert!(
        server.audit_items("").len() > audit_before,
        "audit log must record the conflict ruling"
    );

    // #41: Resolving the already-resolved conflict again reports CONFLICT_ALREADY_RESOLVED.
    d.run_cmd(&format!("PIDCOLLAB RESOLVE {conflict_id} 采用乙方"));
    let d_tail = d.app.command_line.history_plain_text();
    assert!(
        d_tail.contains("CONFLICT_ALREADY_RESOLVED") || d_tail.contains("已经裁决"),
        "second resolve must report already resolved; got: {d_tail}"
    );
}

// ── #42: Client history & field last-writer queries ──────────────────────

#[test]
fn harness_42_history_and_last_writer_queries_online_and_offline() {
    // Offline query gives a clear message and does not block.
    let mut offline = OpenCADStudio::new_for_test();
    let _ = offline.automation_op(r#"{"op":"new"}"#);
    let _ = offline.run_command_line("PIDCOLLAB HISTORY P-1001");
    let _ = offline.run_command_line("PIDCOLLAB LASTWRITER P-1001 设计压力");
    let tail = offline.command_line.history_plain_text();
    assert!(tail.contains("未连接协同服务器"), "{tail}");

    // Online query against real server.
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    assert!(a.wait_until(Duration::from_secs(5), |app| app.collab.connected));

    let tag = place_pump(&mut a);
    assert!(a.wait_until(Duration::from_secs(5), |app| app.collab.base_revision >= 1));
    a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 4.0MPa"));
    assert!(a.wait_until(Duration::from_secs(5), |app| app.collab.base_revision >= 2));

    a.run_cmd(&format!("PIDCOLLAB HISTORY {tag} 10"));
    assert!(
        !a.app.collab.history_rows.is_empty(),
        "PIDCOLLAB HISTORY must populate history_rows from real server"
    );

    a.run_cmd(&format!("PIDCOLLAB LASTWRITER {tag} 设计压力"));
    let writer = a
        .app
        .collab
        .field_writers
        .get(&(tag.clone(), "设计压力".to_owned()))
        .expect("field_writers must cache last writer for 设计压力");
    assert_eq!(writer.user_id, "u1");
    assert_eq!(writer.value, "4.0MPa");
    assert!(writer.revision >= 2);
}

// ── #78: Recovery after divergence (PIDCOLLAB REALIGN) ───────────────────

#[test]
fn harness_78_realign_recovers_diverged_session_and_resumes_submissions() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1101");
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.connected && b.collab.connected
    }));

    let tag = place_pump(&mut a);
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.base_revision >= 1 && b.collab.base_revision >= 1
    }));

    // Artificially diverge B (simulating a missed revision / unapplied op).
    b.app.collab.mark_diverged(
        crate::pid::collab_state::REVISION_GAP,
        "模拟中断导致的不一致".to_owned(),
    );
    assert!(b.app.collab.diverged.is_some());

    // While B is diverged, A advances the drawing to revision 2.
    a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 1.8MPa"));
    assert!(a.wait_until(Duration::from_secs(5), |app| app.collab.base_revision >= 2));

    // B runs PIDCOLLAB REALIGN -> reconnects at its last aligned revision,
    // receives server replay, clears diverged, and can submit again!
    b.run_cmd("PIDCOLLAB REALIGN");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(6), |_, b| {
            !b.collab.pending_realign
                && b.collab.connected
                && b.collab.diverged.is_none()
                && b.collab.not_aligned.is_none()
                && b.collab.base_revision >= 2
        }),
        "B must realign cleanly; pending={}, diverged={:?}, not_aligned={:?}, rev={}",
        b.app.collab.pending_realign,
        b.app.collab.diverged,
        b.app.collab.not_aligned,
        b.app.collab.base_revision
    );

    // Now B submits a new edit and A receives it!
    b.run_cmd(&format!("PIDSETPROP {tag} 设计温度 120℃"));
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.base_revision >= 3 && b.collab.base_revision >= 3
        }),
        "after realign, B's new edit must submit and reach A"
    );
    assert_eq!(a.read_prop(&tag, "设计温度").as_deref(), Some("120℃"));
}

#[test]
fn harness_78_realign_refused_reports_clear_reason_and_next_step() {
    let mut app = OpenCADStudio::new_for_test();
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app.collab.server_url = "ws://127.0.0.1:3240/ws".to_owned();
    app.collab.user_id = "u1".to_owned();
    app.collab.drawing_id = "PID-1101".to_owned();
    app.collab.pending_realign = true;

    app.collab.apply(CollabEvent::AlignRefused {
        code: ocs_collab::protocol::codes::REPO_MISMATCH.to_owned(),
        message: "这份文件属于另一个服务器库".to_owned(),
    });
    app.collab.apply(CollabEvent::Connected {
        session_id: "s-1".to_owned(),
        base_revision: 5,
        repo_id: "r-other".to_owned(),
        peers: Vec::new(),
        claims: Vec::new(),
    });

    assert!(app.collab.not_aligned.is_some());
    assert!(!app.collab.pending_realign);
    let toasts = app.collab.toasts.items();
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("重新对齐失败") && t.message.contains("取得最新图纸")),
        "failed realign must explain reason and next step; got {toasts:?}"
    );
}

// ── End-to-end server↔client integration hooks (#37, #48, #49, #52, #55, #59, #65) ──

#[test]
fn harness_hooks_display_name_peer_author_toast_room_claim_and_peer_disconnected() {
    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1101");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.connected && b.collab.connected && a.collab.peers.len() == 1
        }),
        "both editors must connect"
    );

    // #48/#49: Welcome populates local user display_name on both editors.
    assert_eq!(a.app.collab.display_name, "设计甲");
    assert_eq!(b.app.collab.display_name, "设计乙");

    // #59: A places pump and edits property -> B's remote-edit toast names 设计甲（u1）, not 同事.
    let tag = place_pump(&mut a);
    assert!(wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
        a.collab.base_revision >= 1 && b.collab.base_revision >= 1
    }));
    b.app.collab.toasts.clear();
    let res = a.run_cmd(&format!("PIDSETPROP {tag} 设计压力 3.6MPa"));
    assert_eq!(res["ok"], true, "{res}");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.base_revision >= 2 && b.collab.base_revision >= 2
        }),
        "property write must reach B"
    );
    let b_toasts = b.app.collab.toasts.items();
    assert!(
        b_toasts.iter().any(|t| {
            t.message.contains("设计甲（u1）")
                && t.message.contains("设计压力")
                && t.message.contains("3.6MPa")
        }),
        "B's peer-edit toast must display real author 设计甲（u1）; got {b_toasts:?}"
    );

    // #55: A claims the tag -> room-wide ClaimGranted immediately updates B's claims,
    // claim_expiry, and canvas claim badge with 设计甲（u1）, without a spurious own-claim toast.
    b.app.collab.toasts.clear();
    let _ = a.run_cmd(&format!("PIDCLAIM {tag}"));
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.claims.get(&tag).map(String::as_str) == Some("u1")
                && b.collab.claims.get(&tag).map(String::as_str) == Some("u1")
                && b.collab.claim_expiry.get(&tag).copied().unwrap_or(0) > 0
        }),
        "B must immediately receive A's room-wide claim and expiry"
    );
    assert!(
        !b.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message == format!("已占用位号 {tag}")),
        "B must not get a spurious own-claim toast; got {:?}",
        b.app.collab.toasts.items()
    );
    assert!(
        b.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message.contains("设计甲（u1）") && t.message.contains(&tag)),
        "B must get peer-claim toast naming 设计甲（u1）; got {:?}",
        b.app.collab.toasts.items()
    );
    let badges = crate::app::pid_shell::collab_ui::canvas_claim_badges(&b.app);
    assert!(
        badges
            .iter()
            .any(|badge| badge.tag == tag && badge.label.contains("设计甲（u1）") && badge.label.contains("到期")),
        "B's canvas claim marker must show holder and expiry; got {badges:?}"
    );

    // #37: A third raw WebSocket peer (u4) drops TCP abruptly without a Close frame ->
    // real server broadcasts PeerDisconnected with hold_until, and A marks peer disconnected.
    let (mut raw_ws, _) = tungstenite::connect(server.ws_url()).expect("raw ws connect");
    let hello = serde_json::to_string(&ocs_collab::ClientMessage::Hello {
        user_id: "u4".into(),
        drawing_id: "PID-1101".into(),
        mode: "协同编辑".into(),
        client_version: "test".into(),
        workspace: String::new(),
        since: None,
        repo_id: None,
        existing_tags: Vec::new(),
        protocol_version: Some(ocs_collab::protocol::PROTOCOL_VERSION),
        resume_session_id: None,
        credential: None,
    })
    .unwrap();
    raw_ws
        .send(tungstenite::Message::Text(hello.into()))
        .expect("send hello");
    let _ = raw_ws.read();
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab.peers.iter().any(|p| p.user_id == "u4" && !p.disconnected)
        }),
        "A must see u4 join"
    );
    // Drop TCP socket without Close frame to simulate network drop.
    drop(raw_ws);
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab
                .peers
                .iter()
                .any(|p| p.user_id == "u4" && p.disconnected && p.hold_until.is_some())
        }),
        "A must mark u4 disconnected with hold_until; peers={:?}",
        a.app.collab.peers
    );
    assert!(
        a.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message.contains("管理丁（u4）") && t.message.contains("连接中断")),
        "A must emit disconnected toast for u4; got {:?}",
        a.app.collab.toasts.items()
    );
}

#[test]
fn harness_65_all_three_unpaired_connector_reasons_flow_from_server_to_client_toasts() {
    use crate::app::collab::UnpairedReason;

    let server = RealServer::start(standard_seed(), 0, 300);
    let mut a = HeadlessClient::connect(&server.ws_url(), "u1", "PID-1101");
    let mut b = HeadlessClient::connect(&server.ws_url(), "u2", "PID-1102");
    let mut c = HeadlessClient::connect(&server.ws_url(), "u4", "PID-1103");
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, b| {
            a.collab.connected && b.collab.connected
        }) && c.wait_until(Duration::from_secs(5), |c| c.collab.connected),
        "all three drawing editors must connect"
    );

    // Reason 1: Target drawing is not registered (PID-9999).
    a.draw_points("PIDPIPE signal S-UNREG", &[[0.0, 0.0], [100.0, 0.0]]);
    a.draw_points("PIDCONNECTOR signal_out PID-9999", &[[100.0, 0.0]]);
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab.unpaired_connectors.iter().any(|u| {
                u.line_tag == "S-UNREG" && u.reason == UnpairedReason::TargetDrawingNotRegistered
            })
        }),
        "must detect TargetDrawingNotRegistered; got {:?}",
        a.app.collab.unpaired_connectors
    );
    assert!(
        a.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message.contains("S-UNREG") && t.message.contains("目标图纸没有登记")),
        "toast must report unregistered target drawing; got {:?}",
        a.app.collab.toasts.items()
    );

    // Reason 2: Target drawing PID-1102 is registered, but has no matching inflow connector.
    a.draw_points("PIDPIPE signal S-NOMATCH", &[[0.0, 50.0], [100.0, 50.0]]);
    a.draw_points("PIDCONNECTOR signal_out PID-1102", &[[100.0, 50.0]]);
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab.unpaired_connectors.iter().any(|u| {
                u.line_tag == "S-NOMATCH" && u.reason == UnpairedReason::NoMatchingPeerConnector
            })
        }),
        "must detect NoMatchingPeerConnector; got {:?}",
        a.app.collab.unpaired_connectors
    );
    assert!(
        a.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message.contains("S-NOMATCH") && t.message.contains("没有同一线号的流入页间连接符")),
        "toast must report missing matching connector; got {:?}",
        a.app.collab.toasts.items()
    );

    // Reason 3: Pair S-TAKEN between PID-1103 -> PID-1102, then A targets PID-1102 on S-TAKEN.
    c.draw_points("PIDPIPE signal S-TAKEN", &[[0.0, 0.0], [100.0, 0.0]]);
    c.draw_points("PIDCONNECTOR signal_out PID-1102", &[[100.0, 0.0]]);
    b.draw_points("PIDPIPE signal S-TAKEN", &[[0.0, 0.0], [100.0, 0.0]]);
    b.draw_points("PIDCONNECTOR signal_in PID-1103", &[[0.0, 0.0]]);
    assert!(
        wait_both(&mut c, &mut b, Duration::from_secs(5), |c, b| {
            c.collab.base_revision >= 3 && b.collab.base_revision >= 3
        }),
        "PID-1103 <-> PID-1102 S-TAKEN must pair first"
    );

    a.draw_points("PIDPIPE signal S-TAKEN", &[[0.0, 100.0], [100.0, 100.0]]);
    a.draw_points("PIDCONNECTOR signal_out PID-1102", &[[100.0, 100.0]]);
    assert!(
        a.wait_until(Duration::from_secs(5), |app| {
            app.collab.unpaired_connectors.iter().any(|u| {
                u.line_tag == "S-TAKEN" && u.reason == UnpairedReason::PeerAlreadyPaired
            })
        }),
        "must detect PeerAlreadyPaired; got {:?}",
        a.app.collab.unpaired_connectors
    );
    assert!(
        a.app
            .collab
            .toasts
            .items()
            .iter()
            .any(|t| t.message.contains("S-TAKEN") && t.message.contains("对方已经另有配对")),
        "toast must report peer already paired; got {:?}",
        a.app.collab.toasts.items()
    );

    // Pairing S-NOMATCH clears it from A's unpaired list automatically!
    b.draw_points("PIDPIPE signal S-NOMATCH", &[[0.0, 50.0], [100.0, 50.0]]);
    b.draw_points("PIDCONNECTOR signal_in PID-1101", &[[0.0, 50.0]]);
    assert!(
        wait_both(&mut a, &mut b, Duration::from_secs(5), |a, _| {
            !a.collab
                .unpaired_connectors
                .iter()
                .any(|u| u.line_tag == "S-NOMATCH")
        }),
        "once paired, S-NOMATCH must clear from A's unpaired_connectors; got {:?}",
        a.app.collab.unpaired_connectors
    );
}


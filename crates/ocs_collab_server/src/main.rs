//! CLI and startup for the collaboration server.
//!
//! Flags follow `architecture.md` appendix A.2: `--port` (3240), `--db`,
//! `--heartbeat-timeout` (90), `--claim-ttl` (300) and an optional `--seed`.
//! `--bind` (127.0.0.1) came after that appendix: the address listened on,
//! which it had fixed to this machine's loopback. Any other address lets
//! other machines in, and the server neither authenticates nor encrypts, so
//! that is for a trusted network only; the usage line and a warning at
//! startup say so. `--new-repo-id` gives the database a new repository id
//! before it is served: the one start after a backup was restored.
//!
//! Two startup behaviours matter beyond "it binds a port":
//! * a port already in use must fail with a non-zero exit code and say so on
//!   stderr, never silently pick another port (VAL-SRV-026);
//! * a database that cannot be opened does not abort the process — the server
//!   still listens so `GET /health` can report 503 (VAL-SRV-002) — but it
//!   takes no session: every `hello` is answered `STORE_UNAVAILABLE`
//!   (`SPEC-collab-offline-sync.md` §6).
//!
//! The order of those two steps is load-bearing: the port is bound *before* the
//! database is opened. `build_state` clears the ephemeral session and claim
//! rows, so opening first would let a start that is about to fail on a taken
//! port erase the state of the instance already serving on it.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;

use ocs_collab::SessionConfig;
use ocs_collab_server::store::Store;
use ocs_collab_server::AppState;
use ocs_collab_server::{http, ops};

/// Default listen port, matching `services.yaml`.
const DEFAULT_PORT: u16 = 3240;
/// Default listen address: this machine only.
const DEFAULT_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const DEFAULT_HEARTBEAT_TIMEOUT: i64 = 90;
const DEFAULT_CLAIM_TTL: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cli {
    /// Address listened on. Other machines reach any but a loopback one.
    bind: IpAddr,
    port: u16,
    db: String,
    heartbeat_timeout: i64,
    claim_ttl: i64,
    seed: Option<String>,
    /// Milliseconds an op made against the current revision is held so that
    /// another made against the same revision can tie with it. 0 holds
    /// nothing.
    tie_window_ms: u64,
    /// Give the database a new repository id before serving it: the start
    /// after a backup has been restored.
    new_repo_id: bool,
}

fn default_db_path() -> String {
    // %APPDATA%\OpenCADStudio\collab.db on Windows; a plain fallback elsewhere
    // keeps the binary usable in tests without an environment lookup.
    match std::env::var("APPDATA") {
        Ok(appdata) if !appdata.is_empty() => {
            format!("{appdata}\\OpenCADStudio\\collab.db")
        }
        _ => "collab.db".to_owned(),
    }
}

fn usage() -> &'static str {
    "ocs_collab_server --port <port> --db <path> [--bind <IP 地址>] \
     [--heartbeat-timeout <seconds>] [--claim-ttl <seconds>] [--seed <json>] \
     [--tie-window-ms <milliseconds>] [--new-repo-id]\n\
     --bind 默认 127.0.0.1，只有本机能连。服务器没有登录验证、不加密，\
     监听别的地址（如 0.0.0.0）只适合可信内网。\n\
     --new-repo-id 给库换一个新的库标识后照常启动。从备份恢复库文件之后用它启动一次，\
     记着旧库标识的图纸加入时得到 REPO_MISMATCH；每次带它启动都会再换一次。"
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut cli = Cli {
        bind: DEFAULT_BIND,
        port: DEFAULT_PORT,
        db: default_db_path(),
        heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
        claim_ttl: DEFAULT_CLAIM_TTL,
        seed: None,
        tie_window_ms: 0,
        new_repo_id: false,
    };

    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| -> Result<String, String> {
            iter.next()
                .ok_or_else(|| format!("{name} 需要一个值；用法：{}", usage()))
        };
        match flag.as_str() {
            "--bind" => {
                let raw = value("--bind")?;
                cli.bind = raw.parse().map_err(|_| {
                    format!("--bind 需要一个 IP 地址，收到 {raw}；用法：{}", usage())
                })?;
            }
            "--port" => {
                let raw = value("--port")?;
                cli.port = raw
                    .parse()
                    .map_err(|_| format!("--port 需要 1-65535 的整数，收到 {raw}"))?;
            }
            "--db" => cli.db = value("--db")?,
            "--heartbeat-timeout" => {
                let raw = value("--heartbeat-timeout")?;
                cli.heartbeat_timeout = raw.parse().map_err(|_| {
                    format!("--heartbeat-timeout 需要正整数秒数，收到 {raw}")
                })?;
                if cli.heartbeat_timeout <= 0 {
                    return Err("--heartbeat-timeout 必须大于 0".to_owned());
                }
            }
            "--claim-ttl" => {
                let raw = value("--claim-ttl")?;
                cli.claim_ttl = raw
                    .parse()
                    .map_err(|_| format!("--claim-ttl 需要正整数秒数，收到 {raw}"))?;
                if cli.claim_ttl <= 0 {
                    return Err("--claim-ttl 必须大于 0".to_owned());
                }
            }
            "--seed" => cli.seed = Some(value("--seed")?),
            "--tie-window-ms" => {
                let raw = value("--tie-window-ms")?;
                cli.tie_window_ms = raw
                    .parse()
                    .map_err(|_| format!("--tie-window-ms 需要非负整数毫秒数，收到 {raw}"))?;
            }
            "--new-repo-id" => cli.new_repo_id = true,
            "--help" | "-h" => return Err(usage().to_owned()),
            other => return Err(format!("未知参数 {other}；用法：{}", usage())),
        }
    }

    Ok(cli)
}

fn build_state(cli: &Cli) -> AppState {
    let session_config = SessionConfig {
        heartbeat_timeout: cli.heartbeat_timeout,
        claim_ttl: cli.claim_ttl,
    };

    match Store::open(&cli.db) {
        Ok(store) => {
            if cli.new_repo_id {
                match store.renew_repo_id() {
                    Ok(repo_id) => println!("[ocs_collab_server] 库标识已换成 {repo_id}"),
                    Err(err) => {
                        // Serving the old id after a restore is what the
                        // flag is there to prevent.
                        eprintln!("[ocs_collab_server] 换库标识失败：{err}");
                        return unavailable(cli, err, session_config);
                    }
                }
            }
            // Sessions and claims are ephemeral across restarts (appendix A.2).
            if let Err(err) = store.clear_ephemeral() {
                eprintln!("[ocs_collab_server] 清理旧会话失败：{err}");
            }
            if let Some(seed) = &cli.seed {
                match store.seed_from_file(seed) {
                    Ok(()) => println!("[ocs_collab_server] 已从 {seed} 播种"),
                    Err(err) => eprintln!("[ocs_collab_server] 播种失败：{err}"),
                }
            }
            match store.journal_mode() {
                Ok(mode) => println!("[ocs_collab_server] journal_mode = {mode}"),
                Err(err) => eprintln!("[ocs_collab_server] 读取 journal_mode 失败：{err}"),
            }
            println!("[ocs_collab_server] 数据库 {}", cli.db);
            AppState::new(Some(store), None, cli.db.clone(), session_config)
        }
        Err(err) => {
            eprintln!("[ocs_collab_server] 数据库初始化失败：{err}");
            unavailable(cli, err, session_config)
        }
    }
}

/// What a start serves when its database cannot be used: it listens and
/// reports 503 on `GET /health`, and takes no session.
fn unavailable(cli: &Cli, err: String, session_config: SessionConfig) -> AppState {
    eprintln!("[ocs_collab_server] 不接受会话：hello 一律答 STORE_UNAVAILABLE，GET /health 报 503");
    let mut state = AppState::new(None, Some(err), cli.db.clone(), session_config);
    state.store_unavailable = true;
    state
}

/// Where the server listens: the address and the port asked for.
fn listen_addr(cli: &Cli) -> SocketAddr {
    SocketAddr::new(cli.bind, cli.port)
}

/// Bind `addr`, and say which address the socket holds. That one is what
/// startup prints: asked for port 0, only the socket knows the port.
async fn bind(addr: SocketAddr) -> std::io::Result<(tokio::net::TcpListener, SocketAddr)> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    Ok((listener, bound))
}

/// The line startup prints once the server listens on `bound`.
fn listening_line(bound: SocketAddr, cli: &Cli) -> String {
    format!(
        "[ocs_collab_server] v{} 监听 http://{bound}（heartbeat_timeout={}s, claim_ttl={}s, tie_window={}ms）",
        env!("CARGO_PKG_VERSION"),
        cli.heartbeat_timeout,
        cli.claim_ttl,
        cli.tie_window_ms
    )
}

/// What startup says when other machines can reach `bound`; nothing when
/// only this one can.
fn exposure_warning(bound: SocketAddr) -> Option<&'static str> {
    (!bound.ip().is_loopback()).then_some(
        "[ocs_collab_server] 注意：监听的不是本机回环地址，别的机器可以连进来。\
         服务器没有登录验证、不加密，只适合可信内网。",
    )
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    // Bind **before** the database is opened. `build_state` calls
    // `clear_ephemeral`, which drops every session and claim row in the shared
    // database; opening the store first would therefore let a start that is
    // about to fail on a taken port wipe the sessions of the instance that is
    // already serving on it. Ordering is the fix — the cleanup itself stays,
    // because a restarted server must not honour pre-restart `session_id`s.
    let addr = listen_addr(&cli);
    let (listener, bound) = match bind(addr).await {
        Ok(listening) => listening,
        Err(err) => {
            // Never fall back to another port or address: a silent rebind
            // would make the whole verification surface lie about where the
            // server is.
            eprintln!("[ocs_collab_server] 无法监听 {addr}：{err}");
            return ExitCode::FAILURE;
        }
    };

    let mut state = build_state(&cli);
    state.tie_window = std::time::Duration::from_millis(cli.tie_window_ms);
    let state = Arc::new(state);
    // With a tie window, ops made against the current revision wait it out
    // before they commit; this is what commits them.
    if !state.tie_window.is_zero() {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(ops::flush_interval(state.tie_window));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                ops::flush_and_deliver(&state).await;
            }
        });
    }
    let app = http::router(state);

    println!("{}", listening_line(bound, &cli));
    if let Some(warning) = exposure_warning(bound) {
        eprintln!("{warning}");
    }

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        println!("[ocs_collab_server] 收到中断，正在退出");
    };

    if let Err(err) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    {
        eprintln!("[ocs_collab_server] 服务异常退出：{err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    use ocs_collab::protocol::codes;
    use ocs_collab::{ClientMessage, ServerMessage};
    use ocs_collab_server::ws::{handle_message, Connection, Reaction};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn defaults_match_the_architecture_numbers() {
        let cli = parse_args(args(&[])).expect("no args is legal");
        assert_eq!(cli.port, 3240);
        assert_eq!(cli.heartbeat_timeout, 90);
        assert_eq!(cli.claim_ttl, 300);
        assert!(cli.seed.is_none());
        assert!(!cli.db.is_empty(), "the default db path is never empty");
    }

    #[test]
    fn port_and_db_are_taken_from_the_command_line() {
        let cli = parse_args(args(&["--port", "3241", "--db", "D:\\scratch\\collab\\dev.db"]))
            .expect("flags parse");
        assert_eq!(cli.port, 3241);
        assert_eq!(cli.db, "D:\\scratch\\collab\\dev.db");
    }

    #[test]
    fn heartbeat_timeout_and_claim_ttl_override_the_defaults() {
        let cli = parse_args(args(&[
            "--heartbeat-timeout",
            "5",
            "--claim-ttl",
            "5",
        ]))
        .expect("flags parse");
        assert_eq!(cli.heartbeat_timeout, 5);
        assert_eq!(cli.claim_ttl, 5);
    }

    #[test]
    fn a_zero_or_negative_timeout_is_rejected() {
        assert!(parse_args(args(&["--heartbeat-timeout", "0"])).is_err());
        assert!(parse_args(args(&["--claim-ttl", "-1"])).is_err());
    }

    #[test]
    fn a_missing_flag_value_is_rejected() {
        let err = parse_args(args(&["--port"])).expect_err("missing value must fail");
        assert!(err.contains("--port"), "{err}");
    }

    #[test]
    fn an_unknown_flag_is_rejected_with_the_usage_line() {
        let err = parse_args(args(&["--bogus"])).expect_err("unknown flag must fail");
        assert!(err.contains("--bogus"), "{err}");
        assert!(err.contains("--heartbeat-timeout"), "{err}");
    }

    #[test]
    fn a_non_numeric_port_is_rejected() {
        assert!(parse_args(args(&["--port", "abc"])).is_err());
    }

    // `--bind` is the address listened on. Left out, it is this machine's
    // loopback, as it was before there was the flag.
    #[test]
    fn without_bind_the_server_listens_on_this_machine_only() {
        let cli = parse_args(args(&[])).expect("no args is legal");
        assert_eq!(listen_addr(&cli), SocketAddr::from(([127, 0, 0, 1], 3240)));
        let cli = parse_args(args(&["--port", "3241"])).expect("flags parse");
        assert_eq!(listen_addr(&cli), SocketAddr::from(([127, 0, 0, 1], 3241)));
    }

    #[test]
    fn bind_names_the_address_listened_on() {
        let cli = parse_args(args(&["--bind", "0.0.0.0"])).expect("an address parses");
        assert_eq!(listen_addr(&cli), SocketAddr::from(([0, 0, 0, 0], 3240)));
        let cli =
            parse_args(args(&["--bind", "192.0.2.20", "--port", "3241"])).expect("flags parse");
        assert_eq!(listen_addr(&cli), SocketAddr::from(([192, 0, 2, 20], 3241)));
        let cli = parse_args(args(&["--bind", "::1"])).expect("an IPv6 address parses");
        assert_eq!(listen_addr(&cli).to_string(), "[::1]:3240");
    }

    #[test]
    fn a_bind_that_is_not_an_address_is_rejected_with_the_usage_line() {
        for bad in ["abc", "localhost", "127.0.0.1:3240", "256.0.0.1", ""] {
            let err = parse_args(args(&["--bind", bad])).expect_err("not an address must fail");
            assert!(
                err.contains(&format!("--bind 需要一个 IP 地址，收到 {bad}")),
                "{err}"
            );
            assert!(
                err.contains("--heartbeat-timeout"),
                "no usage line in {err}"
            );
        }
        let err = parse_args(args(&["--bind"])).expect_err("missing value must fail");
        assert!(err.contains("--bind"), "{err}");
    }

    // The usage line is where its reader learns what the flag opens up.
    #[test]
    fn the_usage_line_says_what_listening_beyond_this_machine_exposes() {
        let usage = usage();
        for said in [
            "--bind <IP 地址>",
            "默认 127.0.0.1",
            "没有登录验证",
            "不加密",
            "可信内网",
        ] {
            assert!(usage.contains(said), "{said} is not in {usage}");
        }
    }

    // Asked for port 0 the system picks one, so what was asked for and what
    // was bound differ: the address reported is the one a client reaches.
    #[tokio::test]
    async fn the_address_reported_at_startup_is_the_one_bound() {
        let (listener, bound) = bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("binds");
        assert_ne!(bound.port(), 0, "what was asked for, not what was bound");
        assert_eq!(bound, listener.local_addr().expect("local address"));

        let (accepted, dialed) =
            tokio::join!(listener.accept(), tokio::net::TcpStream::connect(bound));
        let (_, peer) = accepted.expect("accepts");
        let dialed = dialed.expect("a client reaches the address reported");
        assert_eq!(dialed.local_addr().expect("client address"), peer);

        let cli = parse_args(args(&[])).expect("no args is legal");
        let line = listening_line(bound, &cli);
        assert!(line.contains(&format!("监听 http://{bound}（")), "{line}");
    }

    #[test]
    fn listening_beyond_this_machine_is_warned_of_at_startup() {
        for local in ["127.0.0.1:3240", "[::1]:3240"] {
            let bound: SocketAddr = local.parse().expect("address");
            assert_eq!(exposure_warning(bound), None, "{local}");
        }
        for open in ["0.0.0.0:3240", "192.0.2.20:3240", "[::]:3240"] {
            let bound: SocketAddr = open.parse().expect("address");
            let Some(warning) = exposure_warning(bound) else {
                panic!("{open} is not warned of");
            };
            for said in ["没有登录验证", "不加密", "可信内网"] {
                assert!(warning.contains(said), "{said} is not in {warning}");
            }
        }
    }

    // No window unless one is asked for: every op then commits on arrival,
    // as it did before there was a window.
    #[test]
    fn the_tie_window_is_off_unless_asked_for() {
        assert_eq!(parse_args(args(&[])).expect("no args").tie_window_ms, 0);
        let cli = parse_args(args(&["--tie-window-ms", "250"])).expect("flag parses");
        assert_eq!(cli.tie_window_ms, 250);
        assert!(parse_args(args(&["--tie-window-ms", "-1"])).is_err());
        assert!(parse_args(args(&["--tie-window-ms", "soon"])).is_err());
    }

    #[test]
    fn seed_is_optional_and_captured_when_present() {
        let cli = parse_args(args(&["--seed", "D:\\scratch\\seed.json"])).expect("flags parse");
        assert_eq!(cli.seed.as_deref(), Some("D:\\scratch\\seed.json"));
    }

    #[test]
    fn the_session_config_reflects_the_flags() {
        let cli = parse_args(args(&["--heartbeat-timeout", "7", "--claim-ttl", "9"])).expect("parse");
        let config = SessionConfig {
            heartbeat_timeout: cli.heartbeat_timeout,
            claim_ttl: cli.claim_ttl,
        };
        assert_eq!(config.heartbeat_timeout, 7);
        assert_eq!(config.claim_ttl, 9);
    }

    #[test]
    fn a_database_path_that_is_a_directory_yields_an_unhealthy_state() {
        let dir = std::env::temp_dir().join("ocs_collab_cli_dir");
        std::fs::create_dir_all(&dir).expect("create dir");
        let cli = Cli {
            bind: DEFAULT_BIND,
            port: 3240,
            db: dir.display().to_string(),
            heartbeat_timeout: 90,
            claim_ttl: 300,
            seed: None,
            tie_window_ms: 0,
            new_repo_id: false,
        };
        let state = build_state(&cli);
        assert!(state.store.is_none());
        assert!(state.db_error.is_some());
    }

    #[test]
    fn a_writable_path_yields_a_wal_backed_store() {
        let path = std::env::temp_dir()
            .join("ocs_collab_cli_ok")
            .join(format!("{}.db", uuid::Uuid::new_v4()));
        let cli = Cli {
            bind: DEFAULT_BIND,
            port: 3240,
            db: path.display().to_string(),
            heartbeat_timeout: 5,
            claim_ttl: 5,
            seed: None,
            tie_window_ms: 0,
            new_repo_id: false,
        };
        let state = build_state(&cli);
        assert!(state.db_healthy());
        assert_eq!(state.session_config.claim_ttl, 5);
        let store = state.store.clone().expect("store");
        assert_eq!(store.journal_mode().expect("mode"), "wal");
        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    // `--new-repo-id` takes no value; left out, the id is left alone. The
    // usage line says what it is for.
    #[test]
    fn new_repo_id_is_a_flag_without_a_value() {
        assert!(!parse_args(args(&[])).expect("no args is legal").new_repo_id);
        let cli = parse_args(args(&["--new-repo-id", "--port", "3241"])).expect("flags parse");
        assert!(cli.new_repo_id);
        assert_eq!(cli.port, 3241, "the flag takes no value");
        let usage = usage();
        for said in ["[--new-repo-id]", "恢复", "REPO_MISMATCH"] {
            assert!(usage.contains(said), "{said} is not in {usage}");
        }
    }

    const T0: i64 = 1_700_000_000;

    /// What a start on `db` is given, with `--new-repo-id` or without.
    fn cli_on(db: &str, new_repo_id: bool) -> Cli {
        Cli {
            new_repo_id,
            ..parse_args(args(&["--db", db])).expect("flags parse")
        }
    }

    fn temp_db(name: &str) -> String {
        let dir = std::env::temp_dir().join("ocs_collab_cli_ok");
        std::fs::create_dir_all(&dir).expect("create dir");
        dir.join(format!("{name}-{}.db", uuid::Uuid::new_v4()))
            .display()
            .to_string()
    }

    fn remove_db(db: &str) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{db}{suffix}"));
        }
    }

    fn hello(user_id: &str, repo_id: Option<&str>, since: Option<i64>) -> ClientMessage {
        ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: "PID-1101".into(),
            mode: "协同编辑".into(),
            client_version: "test".into(),
            workspace: String::new(),
            since,
            existing_tags: Vec::new(),
            repo_id: repo_id.map(str::to_owned),
        }
    }

    /// The `welcome` among `reactions`, as it goes on the wire.
    fn welcome_of(reactions: &[Reaction]) -> serde_json::Value {
        reactions
            .iter()
            .find_map(|reaction| match reaction {
                Reaction::Send(message) => serde_json::to_value(message)
                    .ok()
                    .filter(|wire| wire["type"] == "welcome"),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no welcome in {reactions:?}"))
    }

    /// `OFF-11`: started once with `--new-repo-id`, the server serves its
    /// database under a new repository id, with the op log it had; started
    /// again without the flag, it keeps the new id. A drawing that
    /// remembers the old id is of another repository now.
    #[tokio::test]
    async fn new_repo_id_at_startup_changes_the_id_once_and_it_stays() {
        let db = temp_db("new-repo-id");
        let before = build_state(&cli_on(&db, false));
        let old = before.repo_id.clone();
        let mut editor = Connection::new("127.0.0.1");
        handle_message(&before, &mut editor, hello("u1", None, None), T0).await;
        let session_id = editor
            .session
            .as_ref()
            .expect("handshaken")
            .session_id
            .clone();
        let op = ClientMessage::Op {
            session_id,
            base_revision: 0,
            op_id: "op-1".into(),
            payload: serde_json::json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        handle_message(&before, &mut editor, op, T0 + 1).await;
        drop((editor, before));

        let restarted = build_state(&cli_on(&db, true));
        let renewed = restarted.repo_id.clone();
        assert_ne!(renewed, old, "a new id");
        drop(restarted);

        let after = build_state(&cli_on(&db, false));
        assert_eq!(after.repo_id, renewed, "and it stays");
        let (revision, objects) = ops::objects(&after, "PID-1101", Some("V-1001"));
        assert_eq!(revision, 1, "the room is where it was");
        assert_eq!(
            objects[0]["fields"]["设计压力"]["value"], "1.6MPa",
            "{objects:?}"
        );

        let mut remembering = Connection::new("127.0.0.1");
        let hello_from_before = hello("u2", Some(old.as_str()), Some(1));
        let reactions = handle_message(&after, &mut remembering, hello_from_before, T0 + 2).await;
        let welcome = welcome_of(&reactions);
        assert_eq!(
            welcome["align_refused"]["code"],
            codes::REPO_MISMATCH,
            "{welcome}"
        );

        let mut current = Connection::new("127.0.0.1");
        let hello_now = hello("u3", Some(renewed.as_str()), Some(0));
        let reactions = handle_message(&after, &mut current, hello_now, T0 + 3).await;
        let welcome = welcome_of(&reactions);
        assert!(welcome.get("align_refused").is_none(), "{welcome}");
        assert_eq!(welcome["replay_from"], 0, "{welcome}");
        assert!(
            reactions.iter().any(|reaction| matches!(
                reaction,
                Reaction::Send(ServerMessage::OpApplied { op_id, .. }) if op_id == "op-1"
            )),
            "the op log is as it was: {reactions:?}"
        );
        drop((remembering, current, after));
        remove_db(&db);
    }

    const PATIENCE: Duration = Duration::from_secs(5);
    const TEXT: u8 = 0x1;
    const CLOSE: u8 = 0x8;

    fn connect(addr: SocketAddr) -> std::io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&addr, PATIENCE)?;
        stream.set_read_timeout(Some(PATIENCE))?;
        Ok(stream)
    }

    /// `GET path` on `addr`: the whole response.
    fn http_get(addr: SocketAddr, path: &str) -> std::io::Result<String> {
        let mut stream = connect(addr)?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }

    /// A WebSocket on `addr`'s `/ws`, opened by hand: the crate has no
    /// client, and frames are all a test needs. The response head is read
    /// a byte at a time so that no frame goes with it.
    fn ws_connect(addr: SocketAddr) -> std::io::Result<(String, TcpStream)> {
        let mut stream = connect(addr)?;
        write!(
            stream,
            "GET /ws HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte)?;
            head.push(byte[0]);
        }
        Ok((String::from_utf8_lossy(&head).into_owned(), stream))
    }

    /// A client's text frame; a client masks what it sends.
    fn send_text(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
        let payload = text.as_bytes();
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![0x80 | TEXT];
        match u8::try_from(payload.len()) {
            Ok(len) if len < 126 => frame.push(0x80 | len),
            _ => {
                let len = u16::try_from(payload.len()).expect("a short frame");
                frame.push(0x80 | 126);
                frame.extend_from_slice(&len.to_be_bytes());
            }
        }
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .zip(mask.iter().cycle())
                .map(|(byte, mask)| byte ^ mask),
        );
        stream.write_all(&frame)
    }

    /// One frame from the server, which masks nothing: opcode and payload.
    fn read_frame(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
        let mut head = [0u8; 2];
        stream.read_exact(&mut head)?;
        let len = match head[1] & 0x7f {
            126 => {
                let mut len = [0u8; 2];
                stream.read_exact(&mut len)?;
                usize::from(u16::from_be_bytes(len))
            }
            127 => {
                let mut len = [0u8; 8];
                stream.read_exact(&mut len)?;
                usize::try_from(u64::from_be_bytes(len)).expect("a frame that fits")
            }
            len => usize::from(len),
        };
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload)?;
        Ok((head[0] & 0x0f, payload))
    }

    /// The opcodes of the frames that come next, up to the end of the
    /// stream; `None` if it has not ended within [`PATIENCE`].
    fn frames_until_closed(stream: &mut TcpStream) -> Option<Vec<u8>> {
        let mut opcodes = Vec::new();
        loop {
            match read_frame(stream) {
                Ok((opcode, _)) => opcodes.push(opcode),
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::UnexpectedEof
                            | ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                    ) =>
                {
                    return Some(opcodes)
                }
                Err(_) => return None,
            }
        }
    }

    /// `OFF-11`, over a real socket: started on a database it cannot open,
    /// the server listens and says so on `/health` (503). A `hello` is
    /// answered `STORE_UNAVAILABLE`, and then the connection is closed.
    #[tokio::test]
    async fn store_unavailable_a_database_that_cannot_be_opened_takes_no_session() {
        let dir = std::env::temp_dir().join("ocs_collab_cli_dir");
        std::fs::create_dir_all(&dir).expect("create dir");
        let state = build_state(&cli_on(&dir.display().to_string(), false));
        let (listener, bound) = bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("binds");
        let service =
            http::router(Arc::new(state)).into_make_service_with_connect_info::<SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, service).await });

        let hello = serde_json::to_string(&hello("u1", None, None)).expect("hello");
        let client = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
            let health = http_get(bound, "/health")?;
            let (upgraded, mut socket) = ws_connect(bound)?;
            send_text(&mut socket, &hello)?;
            let answer = read_frame(&mut socket)?;
            Ok((health, upgraded, answer, frames_until_closed(&mut socket)))
        });
        let (health, upgraded, (opcode, payload), rest) = client
            .await
            .expect("the client ran")
            .expect("the server answered");
        server.abort();

        assert!(health.starts_with("HTTP/1.1 503"), "{health}");
        assert!(upgraded.starts_with("HTTP/1.1 101"), "{upgraded}");
        assert_eq!(opcode, TEXT);
        let answer: serde_json::Value = serde_json::from_slice(&payload).expect("JSON");
        assert_eq!(answer["type"], "error", "{answer}");
        assert_eq!(answer["code"], codes::STORE_UNAVAILABLE, "{answer}");
        assert_eq!(
            rest,
            Some(vec![CLOSE]),
            "a close, then the end of the stream"
        );
    }
}

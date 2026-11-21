//! REST transport — the automation API over plain localhost HTTP.
//!
//! `OpenCADStudio --http 8090` serves the same operation dispatcher as
//! `--serve` behind resource-oriented JSON endpoints that any HTTP client —
//! curl, Python `urllib`/`requests`, JS `fetch`, C# `HttpClient` — can call
//! with no SDK and no AI-specific machinery. The REST layer is a thin
//! adapter: every route builds the protocol-1 JSON request and feeds the
//! shared `automation_op`, so GUI, stdio, TCP, MCP, wasm and HTTP all run
//! the identical handlers with identical validation, undo and idempotency.
//!
//! Conventions: one request per connection (`Connection: close`), JSON
//! bodies, loopback bind only, and every request carries
//! `Authorization: Bearer <token>` (see [`api_token`]). Loopback is shared by
//! every process and user on the machine, and a web page can aim requests at
//! it too, so the port alone proves nothing; no CORS headers are sent, so a
//! browser page cannot read an answer either.
//! Connections are served one thread each: a slow op (plot, open a large
//! drawing) or a client that connects and stalls must never keep other
//! callers — `state`, `get_selection`, `cancel`, `operation` — from being
//! answered. Mutation endpoints get a server-generated `request_id` and the
//! active `document_id`; a stale-state refusal refreshes state and retries
//! once.
//!
//! The route table is shared: `plan` resolves (method, path) to a `Plan`
//! that this headless server executes directly and the GUI-hosted bridge
//! executes over the live editor's envelope queue (`app::control::http_bridge`),
//! so both `--http` modes serve the identical surface.

use crate::app::OpenCADStudio;
use crate::io::line_read::{is_line_too_long, read_line_capped};
use serde_json::{json, Value};
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU16, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_BODY: usize = 16 * 1024 * 1024;

/// Per-line cap for the request line and each header line. `BufRead::read_line`
/// grows its `String` by doubling, so a client streaming `X: AAAA…` with no
/// CRLF could otherwise push a single line to gigabytes until the allocation
/// aborts the process — which is not a catchable panic. 8 KiB matches the
/// usual reverse-proxy header-line allowance.
const MAX_LINE: usize = 8 * 1024;

/// Cap on the whole header block: every individual line may sit under
/// [`MAX_LINE`] while their count is infinite. 64 KiB matches common
/// reverse-proxy header-buffer budgets.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Serial for request ids stamped onto `POST /api/v1/{op}` passthroughs that
/// arrive without one — the protocol-1 envelope requires a unique id, and
/// the caller's own id always wins when present.
static PASSTHROUGH_SERIAL: AtomicU64 = AtomicU64::new(0);

/// Connections served at once. A client that connects and stalls pins one
/// thread until its idle timeout, not the whole listener.
const MAX_CONNECTIONS: usize = 8;

/// How long one connection may sit between bytes before the server gives up
/// on it (a client that opened a socket and never sent the request). Bounded
/// so a stuck client cannot pin a thread forever.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Ops that go through the protocol-1 control envelope when posted via the
/// generic `POST /api/v1/{op}` passthrough. Everything else runs legacy.
const ENVELOPE_OPS: &[&str] = &[
    "entities_create",
    "entities_set",
    "xref_reload",
    "xref_unload",
    "xref_detach",
    "xref_bind",
    "xref_set_path",
    "xref_set_type",
    "entities_delete",
    "entities_transform",
    "node_graph",
    "block_define",
    "block_delete",
    "file_identity",
    "xdata_set",
    "view_focus",
    "wblock",
    "plot",
    "embed_image",
    "dgn_import",
    "set_properties",
    "property",
    "activate",
    "get_selection",
    "getpoint",
    "layout_create",
    "layout_rename",
    "layout_activate",
    "layout_delete",
    "page_setup_set",
];

/// Loopback REST channel hosted by the GUI process: `OpenCADStudio.exe
/// "<file>" --http <port>` boots the editor with a bridge that forwards
/// requests into the live app (see `app::control::http_bridge`), so a client
/// can read what the person at the screen selected. `--http` without files
/// keeps the headless server above.
pub fn set_gui_http_port(port: u16) {
    crate::app::control::http_bridge::set_gui_http_port(port);
}

/// Failure codes worth one state refresh + retry (the request never started).
pub(crate) const RETRYABLE: &[&str] = &[
    "document_required",
    "document_closed",
    "document_not_active",
    "stale_state",
    "session_changed",
];

/// The private session a `--http` listener owns. `new_headless` lands on
/// the welcome (Start) tab, whose `is_start` flag makes the control gate
/// refuse `run`/`start`/`input` with `no_document` — the listener
/// answered `ready` and even cached a document id while every drawing
/// command was dropped. The legacy `new` op exists for this half-state:
/// it turns that tab into a real drawing in place.
fn serve_app(read_only: bool) -> OpenCADStudio {
    let mut app = OpenCADStudio::new_headless(read_only);
    let _ = app.automation_op(r#"{"op":"new"}"#);
    app
}

pub fn serve(port: u16, read_only: bool) {
    listen(serve_app(read_only), port, Arc::new(AtomicU16::new(0)));
}

/// The secret every TCP automation client presents: `OCS_API_TOKEN` when the
/// launcher set one (so a script knows it in advance), otherwise 32 random
/// bytes printed once on stderr when a listener starts. The GUI's private
/// channel keeps its own per-session token in the user-only descriptor.
pub fn api_token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| {
        if let Some(token) = std::env::var("OCS_API_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
        {
            return token.trim().to_owned();
        }
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).expect("system random source");
        secret.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

/// Tells a listener's user how to authenticate. A token the launcher chose
/// is not echoed back.
pub(crate) fn announce_token() {
    if std::env::var("OCS_API_TOKEN").map_or(true, |token| token.trim().is_empty()) {
        eprintln!(
            "API token (or set OCS_API_TOKEN before launch): {}",
            api_token()
        );
    }
}

/// Whether `presented` is the session's token, compared in constant time.
pub(crate) fn token_matches(presented: Option<&str>) -> bool {
    let expected = api_token().as_bytes();
    let Some(presented) = presented.map(str::as_bytes) else {
        return false;
    };
    presented.len() == expected.len()
        && presented
            .iter()
            .zip(expected)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// The answer to a request without the session's token.
pub(crate) fn unauthorized() -> Value {
    json!({
        "ok": false,
        "code": "unauthorized",
        "error": "Send the session token (printed at startup, or OCS_API_TOKEN): Authorization: Bearer <token> over HTTP, \"token\" in each --serve --port request",
    })
}

pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    query: Vec<(String, String)>,
    body: Vec<u8>,
    bearer: Option<String>,
}

impl HttpRequest {
    fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    pub(crate) fn authorized(&self) -> bool {
        token_matches(self.bearer.as_deref())
    }
}

/// One request handed from a connection thread to the dispatcher that owns
/// the private app. `OpenCADStudio` is not `Send` (it holds `Rc`/`RefCell`
/// scene state), so — exactly like the GUI-hosted bridge queues envelopes
/// into the editor — connections funnel JSON through this channel and the
/// dispatcher (the thread that called `listen`) answers on the per-request
/// reply channel.
struct Job {
    request: HttpRequest,
    reply: std::sync::mpsc::Sender<(u16, Value)>,
}

fn listen(mut app: OpenCADStudio, port: u16, bound_port: Arc<AtomicU16>) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("--http: cannot bind 127.0.0.1:{port}: {error}");
            return;
        }
    };
    let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    bound_port.store(bound, Ordering::SeqCst);
    eprintln!("OpenCADStudio REST listening on http://127.0.0.1:{bound}/api/v1");
    announce_token();
    let (jobs, incoming) = std::sync::mpsc::channel::<Job>();
    // Connections are served one thread each: a slow op (plot, open a large
    // drawing) or a client that connects and stalls must never keep other
    // callers — `state`, `get_selection`, `cancel`, `operation` — from being
    // accepted and answered.
    let clients = Arc::new(AtomicUsize::new(0));
    let acceptor = jobs.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if clients.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                clients.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let jobs = acceptor.clone();
            let clients = clients.clone();
            std::thread::spawn(move || {
                serve_connection(stream, jobs);
                clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    // The dispatcher keeps the private app on the thread that called
    // `listen` and answers one request at a time — the same one-session
    // pipeline every transport shares; concurrency lives in the connection
    // threads above, not in the app.
    let mut document_id: Option<u64> = None;
    let mut counter: u64 = 0;
    // Between requests, plugins' worker threads are still served, as the
    // window's timer does in the GUI.
    loop {
        match incoming.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(job) => {
                let (status, body) = route(&mut app, &job.request, &mut document_id, &mut counter);
                let _ = job.reply.send((status, body));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => app.pump_plugin_requests(),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Read one request off a connection, hand it to the dispatcher, write the
/// answer. Everything here is per-connection I/O; the app is never touched.
fn serve_connection(mut stream: TcpStream, jobs: std::sync::mpsc::Sender<Job>) {
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IDLE_TIMEOUT));
    let Ok(Some(request)) = read_request(&mut stream) else {
        return;
    };
    if !request.authorized() {
        let _ = write_response(&mut stream, 401, &unauthorized());
        return;
    }
    let (reply, answer) = std::sync::mpsc::channel();
    if jobs.send(Job { request, reply }).is_err() {
        return;
    }
    if let Ok((status, mut body)) = answer.recv() {
        if let Some(object) = body.as_object_mut() {
            object.insert("http_status".into(), json!(status));
        }
        let _ = write_response(&mut stream, status, &body);
    }
}

/// Answer a peer whose request line or header block blew a size cap — RFC
/// 9110's `431` — then hand the caller `Ok(None)` so it closes without
/// dispatching. Writing the refusal from here keeps the headless `--http`
/// server and the GUI-hosted bridge (`app::control::http_bridge`), which both
/// call `read_request`, answering identically.
fn refuse_oversized(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let body = json!({
        "ok": false,
        "code": "header_too_large",
        "error": "Request line or headers exceed the size limit.",
    });
    let _ = write_response(stream, 431, &body);
    Ok(None)
}

/// Parse one HTTP/1.1 request. `Ok(None)` = the peer hung up cleanly, or the
/// request blew a size cap (a `431` has already been written in that case).
pub(crate) fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    match read_line_capped(&mut reader, &mut line, MAX_LINE) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) if is_line_too_long(&error) => return refuse_oversized(stream),
        Err(error) => return Err(error),
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target, Vec::new()),
    };

    let mut content_length = 0usize;
    let mut header_bytes = 0usize;
    let mut bearer = None;
    loop {
        let mut header = String::new();
        match read_line_capped(&mut reader, &mut header, MAX_LINE) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error) if is_line_too_long(&error) => return refuse_oversized(stream),
            Err(error) => return Err(error),
        }
        header_bytes += header.len();
        if header_bytes > MAX_HEADER_BYTES {
            return refuse_oversized(stream);
        }
        let header = header.trim();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header
            .strip_prefix("Content-Length:")
            .or_else(|| header.strip_prefix("content-length:"))
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("authorization") {
                bearer = value
                    .trim()
                    .strip_prefix("Bearer ")
                    .map(|token| token.trim().to_owned());
            }
        }
    }
    if content_length > MAX_BODY {
        return Ok(Some(HttpRequest {
            method,
            path,
            query,
            body: Vec::new(),
            bearer,
        }));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        body,
        bearer,
    }))
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (percent_decode(key), percent_decode(value)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if hex.len() == 2 {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: &Value,
) -> std::io::Result<()> {
    let body = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(body).unwrap_or_default()
    };
    let reason = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(&body)?;
    }
    stream.flush()
}

/// Contract aliases layered onto a read answer. The automation ops answer
/// with their own field names (`documents[].id`, `entities.count`, and the
/// `result` wrapper around a capture); the REST contract names the same
/// values `document_id`, `total` and top-level capture fields. Aliasing here
/// keeps both spellings present instead of renaming the op output every
/// other client already reads.
#[derive(Clone, Copy)]
pub(crate) enum Alias {
    /// `documents[]` gains `document_id` beside `id`.
    Documents,
    /// The entity count gains `total` beside `count`.
    Entities,
    /// The capture `result` fields are hoisted to the top level.
    Capture,
}

/// One resolved REST call, shared by the two `--http` transports: this
/// headless server executes it against its private app, and the GUI-hosted
/// bridge (`app::control::http_bridge`) forwards the same request into the
/// live editor's automation queue. Building the request here keeps both
/// surfaces identical — same URLs, same ops, same document-id handling.
pub(crate) enum Plan {
    /// Answer without the automation pipeline (OpenAPI, unknown routes).
    Local(u16, Value),
    /// `GET /ready`: a state read rebuilt as the readiness snapshot.
    Ready,
    /// Run one prebuilt protocol-1 request as-is — reads, and callers that
    /// manage `document_id` themselves (the MCP-style clients).
    Run { request: Value, created: u16 },
    /// A read whose answer also carries the contract's field spelling (see
    /// [`Alias`]); everything else behaves like [`Plan::Run`].
    Aliased {
        request: Value,
        created: u16,
        alias: Alias,
    },
    /// Run one op through the mutation wrapper: envelope with a request id,
    /// the cached `document_id` when the caller omitted it, one state-refresh
    /// retry after a stale refusal, and the bulky `state` snapshot stripped
    /// from the answer.
    Mutate {
        op: String,
        fields: Value,
        created: u16,
    },
}

impl Plan {
    /// The automation op this plan runs, for transports that vary connection
    /// behavior per op (the GUI bridge parks the interactive picks).
    pub(crate) fn op(&self) -> Option<&str> {
        match self {
            Plan::Local(..) | Plan::Ready => None,
            Plan::Run { request, .. } | Plan::Aliased { request, .. } => request["op"].as_str(),
            Plan::Mutate { op, .. } => Some(op),
        }
    }
}

/// Wrap a body in the protocol-1 envelope for one op: the named op wins, the
/// caller's `request_id` survives, an unstamped body gets a serial one. This
/// is what every `POST` passthrough does, so an op that only exists on the
/// envelope path (`capture`, `cancel`) reaches it instead of dying as
/// "unknown op" on the legacy path.
fn stamp_envelope(op: &str, mut body: Value) -> Value {
    body["op"] = json!(op);
    body["protocol"] = json!(1);
    if body["request_id"].as_str().is_none_or(str::is_empty) {
        let serial = PASSTHROUGH_SERIAL.fetch_add(1, Ordering::Relaxed);
        body["request_id"] = json!(format!("rest-{serial}"));
    }
    body
}

/// Layer the contract's field spelling onto a read answer (see [`Alias`]).
/// Values the op did not produce are left alone: an alias never invents a
/// field out of a failure body.
pub(crate) fn apply_alias(alias: Alias, response: &mut Value) {
    match alias {
        Alias::Documents => {
            if let Some(documents) = response["documents"].as_array_mut() {
                for document in documents {
                    let id = document["id"].clone();
                    if !id.is_null() {
                        document["document_id"] = id;
                    }
                }
            }
        }
        Alias::Entities => {
            let count = response["count"].clone();
            if count.is_u64() {
                response["total"] = count;
            }
        }
        Alias::Capture => {
            let hoisted_result = response["result"].clone();
            let Some(result) = hoisted_result.as_object() else {
                return;
            };
            let hoisted: Vec<(String, Value)> = result
                .iter()
                .filter(|(key, _)| key.as_str() != "state")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            if let Some(object) = response.as_object_mut() {
                for (key, value) in hoisted {
                    object.insert(key, value);
                }
            }
        }
    }
}

/// The whole REST surface: map (method, path) to the automation request it
/// runs. The caller splits `/api/v1` off already; the remaining segments
/// select the arm.
pub(crate) fn plan(method: &str, rest: &[&str], request: &HttpRequest) -> Plan {
    let raw = request.json();
    let body = if raw.is_object() { raw } else { json!({}) };
    match (method, rest) {
        ("GET", ["ready"]) | ("GET", []) | ("GET", [""]) => Plan::Ready,
        ("GET", ["capabilities"]) => Plan::Run {
            request: json!({"op":"capabilities"}),
            created: 200,
        },
        ("GET", ["state"]) => Plan::Run {
            request: json!({"protocol":1,"op":"state"}),
            created: 200,
        },
        // Collaboration session status (architecture.md A.5): what the desktop
        // client's transport thread last reported.
        ("GET", ["collab", "status"]) => Plan::Run {
            request: json!({"protocol":1,"op":"collab_status"}),
            created: 200,
        },
        // P&ID project master data (SPEC-pid-master-data §5.3): which copy is
        // in effect, and with `usage=true` how many objects use each entry.
        ("GET", ["pid", "master-data"]) => Plan::Run {
            request: json!({
                "protocol": 1,
                "op": "pid_masterdata",
                "usage": request.param("usage").is_some_and(|v| v == "true" || v == "1"),
            }),
            created: 200,
        },
        // Replaces the whole master data; `expected_revision` keeps a write
        // from silently undoing somebody else's.
        ("PUT", ["pid", "master-data"]) => Plan::Mutate {
            op: "pid_masterdata_set".into(),
            fields: body,
            created: 200,
        },
        // P&ID project index (SPEC-pid-project-index §5.3): the current
        // drawing's summary, and the index of its folder, which answers
        // `status: scanning` while the project's files are still being read.
        ("GET", ["pid", "summary"]) => Plan::Run {
            request: json!({"protocol": 1, "op": "pid_summary"}),
            created: 200,
        },
        ("GET", ["pid", "index"]) => {
            let mut index = json!({"protocol": 1, "op": "pid_index"});
            for key in ["tag", "type_code"] {
                if let Some(value) = request.param(key) {
                    index[key] = json!(value);
                }
            }
            Plan::Run {
                request: index,
                created: 200,
            }
        }
        // Selects a tag and zooms to it, switching to or opening the drawing
        // it is on.
        ("POST", ["pid", "locate"]) => Plan::Mutate {
            op: "pid_locate".into(),
            fields: body,
            created: 200,
        },
        // Same state read, plus `documents[].document_id` — the spelling the
        // REST contract (and every validator) reads.
        ("GET", ["documents"]) => Plan::Aliased {
            request: json!({"protocol":1,"op":"state"}),
            created: 200,
            alias: Alias::Documents,
        },
        ("POST", ["documents"]) => {
            if let Some(template) = body["template"].as_str() {
                Plan::Mutate {
                    op: "new".into(),
                    fields: json!({"template": template}),
                    created: 201,
                }
            } else if let Some(path) = body["path"].as_str() {
                Plan::Run {
                    request: json!({"op":"open","path":path}),
                    created: 201,
                }
            } else {
                // A fresh untitled document in
                // its own tab, so cross-document operations have a target.
                Plan::Mutate {
                    op: "new".into(),
                    fields: json!({}),
                    created: 201,
                }
            }
        }
        ("DELETE", ["documents", id]) => {
            let mut fields = json!({});
            if let Ok(id) = id.parse::<u64>() {
                fields["document_id"] = json!(id);
            }
            if request
                .param("discard")
                .is_some_and(|v| v == "true" || v == "1")
            {
                fields["discard"] = json!(true);
            }
            Plan::Mutate {
                op: "close".into(),
                fields,
                created: 200,
            }
        }
        ("GET", ["sysvars"]) => {
            let names = request
                .param("names")
                .map(|csv| json!(csv.split(',').collect::<Vec<_>>()))
                .unwrap_or_else(|| json!([]));
            Plan::Mutate {
                op: "sysvar".into(),
                fields: json!({"get": names}),
                created: 200,
            }
        }
        ("POST", ["sysvars"]) => Plan::Mutate {
            op: "sysvar".into(),
            fields: body,
            created: 200,
        },
        ("POST", ["layouts"]) => Plan::Mutate {
            op: "layout_create".into(),
            fields: body,
            created: 201,
        },
        ("GET", ["layouts"]) => Plan::Run {
            request: json!({"op":"layouts"}),
            created: 200,
        },
        ("GET", ["xrefs"]) => Plan::Run {
            request: json!({"op":"xrefs"}),
            created: 200,
        },
        ("PATCH", ["layouts", name]) => Plan::Mutate {
            op: "layout_rename".into(),
            fields: json!({"name": percent_decode(name), "new_name": body["new_name"]}),
            created: 200,
        },
        ("DELETE", ["layouts", name]) => Plan::Mutate {
            op: "layout_delete".into(),
            fields: json!({"name": percent_decode(name)}),
            created: 200,
        },
        ("POST", ["layouts", name, "activate"]) => Plan::Mutate {
            op: "layout_activate".into(),
            fields: json!({"name": percent_decode(name)}),
            created: 200,
        },
        ("GET", ["layouts", layout, "page-setup"]) => Plan::Run {
            request: json!({"op":"page_setup_get","layout":percent_decode(layout)}),
            created: 200,
        },
        ("PUT", ["layouts", layout, "page-setup"]) => {
            let mut fields = body;
            fields["layout"] = json!(percent_decode(layout));
            Plan::Mutate {
                op: "page_setup_set".into(),
                fields,
                created: 200,
            }
        }
        ("POST", ["entities", "copy-to"]) => Plan::Mutate {
            op: "entities_copy_to".into(),
            fields: body,
            created: 201,
        },
        ("POST", ["groups"]) => Plan::Mutate {
            op: "group_create".into(),
            fields: body,
            created: 201,
        },
        ("POST", ["selection-sets"]) => Plan::Mutate {
            op: "selection_set_save".into(),
            fields: body,
            created: 201,
        },
        ("GET", ["selection-sets", name]) => {
            let select = request
                .param("select")
                .is_some_and(|v| v == "true" || v == "1");
            Plan::Mutate {
                op: "selection_set_load".into(),
                fields: json!({"name": name, "select": select}),
                created: 200,
            }
        }
        ("GET", ["entities"]) => {
            let mut request_json = json!({"op":"query"});
            if let Some(v) = request.param("type") {
                request_json["type"] = json!(v);
            }
            if let Some(v) = request.param("layer") {
                request_json["layer"] = json!(v);
            }
            if let Some(v) = request.param("xdata_app") {
                request_json["xdata_app"] = json!(v);
            }
            if let Some(v) = request.param("detail") {
                request_json["detail"] = json!(v);
            }
            if let Some(v) = request.param("offset") {
                request_json["offset"] = json!(v);
            }
            if let Some(v) = request.param("limit") {
                request_json["limit"] = json!(v);
            }
            if let Some(v) = request.param("fields") {
                request_json["fields"] = json!(v.split(',').collect::<Vec<_>>());
            }
            if let Some(v) = request.param("handles") {
                request_json["handles"] = json!(v.split(',').collect::<Vec<_>>());
            }
            if let Some(v) = request.param("bounds") {
                let numbers: Vec<f64> = v.split(',').filter_map(|n| n.parse().ok()).collect();
                if numbers.len() == 4 {
                    request_json["bounds"] = json!(numbers);
                }
            }
            if let Some(v) = request.param("near") {
                let numbers: Vec<f64> = v.split(',').filter_map(|n| n.parse().ok()).collect();
                request_json["near"] = json!(numbers);
            }
            if let Some(v) = request.param("contains_point") {
                let numbers: Vec<f64> = v.split(',').filter_map(|n| n.parse().ok()).collect();
                request_json["contains_point"] = json!(numbers);
            }
            if let Some(v) = request.param("intersections") {
                request_json["intersections"] = json!(v.split(',').collect::<Vec<_>>());
            }
            if let Some(v) = request.param("where") {
                match serde_json::from_str::<Value>(v) {
                    Ok(filters) => request_json["where"] = filters,
                    Err(error) => {
                        return Plan::Local(
                            400,
                            json!({"ok": false, "code": "invalid_where", "error": format!("where must be a JSON array: {error}")}),
                        )
                    }
                }
            }
            Plan::Aliased {
                request: request_json,
                created: 200,
                alias: Alias::Entities,
            }
        }
        ("GET", ["entities", handle]) => Plan::Run {
            request: json!({"op":"query","handles":[handle],"detail":"full"}),
            created: 200,
        },
        ("POST", ["entities"]) => Plan::Mutate {
            op: "entities_create".into(),
            fields: body,
            created: 201,
        },
        ("DELETE", ["entities"]) => {
            let handles = match request.param("handles") {
                Some(csv) => json!(csv.split(',').collect::<Vec<_>>()),
                None => body["handles"].clone(),
            };
            Plan::Mutate {
                op: "entities_delete".into(),
                fields: json!({"handles": handles}),
                created: 200,
            }
        }
        ("POST", ["entities", "transform"]) => Plan::Mutate {
            op: "entities_transform".into(),
            fields: body,
            created: 200,
        },
        ("GET", ["entities", handle, "xdata"]) => {
            let mut request_json = json!({"protocol":1,"op":"xdata_get","handles":[handle]});
            if let Some(app_name) = request.param("app") {
                request_json["app"] = json!(app_name);
            }
            Plan::Run {
                request: request_json,
                created: 200,
            }
        }
        ("PUT", ["entities", handle, "xdata", app_name]) => {
            let data = request.json();
            let data = if data.is_null() { json!([]) } else { data };
            Plan::Mutate {
                op: "xdata_set".into(),
                fields: json!({"handles":[handle], "app": app_name, "data": data}),
                created: 200,
            }
        }
        ("DELETE", ["entities", handle, "xdata", app_name]) => Plan::Mutate {
            op: "xdata_set".into(),
            fields: json!({"handles":[handle], "app": app_name, "data": []}),
            created: 200,
        },
        ("POST", ["blocks"]) => Plan::Mutate {
            op: "block_define".into(),
            fields: body,
            created: 201,
        },
        ("DELETE", ["blocks", name]) => {
            // `?force=true` mirrors the control op: it erases the referencing
            // inserts instead of refusing a referenced definition.
            let mut fields = json!({"name": name});
            if request
                .param("force")
                .is_some_and(|value| value == "true" || value == "1")
            {
                fields["force"] = json!(true);
            }
            Plan::Mutate {
                op: "block_delete".into(),
                fields,
                created: 200,
            }
        }
        ("POST", ["file-identity"]) => Plan::Mutate {
            op: "file_identity".into(),
            fields: body,
            created: 200,
        },
        ("POST", ["plot"]) => Plan::Mutate {
            op: "plot".into(),
            fields: body,
            created: 200,
        },
        ("POST", ["wblock"]) => Plan::Mutate {
            op: "wblock".into(),
            fields: body,
            created: 200,
        },
        ("POST", ["images"]) => Plan::Mutate {
            op: "embed_image".into(),
            fields: body,
            created: 201,
        },
        // `POST /run` carries its own op. The P&ID contract cancels a live
        // insertion with `{"document_id":D,"op":"cancel"}` on this route
        // (`VAL-DRAW-017`), and a route that rewrote the op to `run` — as the
        // generic passthrough does with the path segment — would answer
        // `command_busy` while a command is live instead of escaping it. The
        // body's op wins; a body without one is the plain `run` of `cmd`.
        ("POST", ["run"]) => {
            let op = body["op"]
                .as_str()
                .filter(|op| !op.is_empty())
                .unwrap_or("run")
                .to_owned();
            Plan::Run {
                request: stamp_envelope(&op, body),
                created: 200,
            }
        }
        ("POST", ["commands"]) => {
            // The GUI channel refuses `run` without the tab's document_id.
            let mut request = json!({"op":"run","cmd":body["cmd"]});
            if let Some(id) = body["document_id"].as_u64() {
                request["document_id"] = json!(id);
            }
            Plan::Run {
                request,
                created: 200,
            }
        }
        ("POST", ["undo"]) | ("POST", ["redo"]) => {
            let op = if rest == ["undo"] { "undo" } else { "redo" };
            let mut request = json!({"op":op});
            if let Some(id) = body["document_id"].as_u64() {
                request["document_id"] = json!(id);
            }
            Plan::Run {
                request,
                created: 200,
            }
        }
        ("POST", ["save"]) => {
            let mut fields = json!({});
            if let Some(path) = body["path"].as_str() {
                fields["path"] = json!(path);
            }
            if let Some(id) = body["document_id"].as_u64() {
                fields["document_id"] = json!(id);
            }
            Plan::Mutate {
                op: "save".into(),
                fields,
                created: 200,
            }
        }
        ("GET", ["layers"]) => Plan::Run {
            request: json!({"op":"layers"}),
            created: 200,
        },
        ("GET", ["header"]) => Plan::Run {
            request: json!({"op":"header"}),
            created: 200,
        },
        ("GET", ["records"]) => {
            let mut request_json = json!({"op":"records"});
            if let Some(v) = request.param("collection") {
                request_json["collection"] = json!(v);
            }
            if let Some(v) = request.param("type") {
                request_json["type"] = json!(v);
            }
            if let Some(v) = request.param("offset") {
                request_json["offset"] = json!(v);
            }
            if let Some(v) = request.param("limit") {
                request_json["limit"] = json!(v);
            }
            Plan::Run {
                request: request_json,
                created: 200,
            }
        }
        // `capture` answers through the operation envelope, so its fields
        // arrive under `result`; the contract reads `path`, `width`,
        // `height`, `world_to_pixel_matrix` and `annotations` at the top
        // level, so hoist them. The envelope stamp matches the generic
        // passthrough: without `protocol` the headless executor would take
        // the legacy path, where `capture` does not exist.
        ("POST", ["capture"]) => Plan::Aliased {
            request: stamp_envelope("capture", body),
            created: 200,
            alias: Alias::Capture,
        },
        ("GET", ["openapi"]) => Plan::Local(
            200,
            serde_json::from_str(include_str!("rest_openapi.json")).unwrap_or(json!({})),
        ),
        ("POST", [op]) if ENVELOPE_OPS.contains(op) => Plan::Mutate {
            op: op.to_string(),
            fields: body,
            created: 200,
        },
        ("POST", [op]) => {
            let mut request_json = body;
            request_json["op"] = json!(op);
            // Stamp protocol and a request id the way the GUI bridge's
            // `prepare` does: the passthrough exists for ops only the
            // protocol-1 envelope knows (`user_select`, `operation`,
            // `cancel`), and unstamped they die as "unknown op" on the
            // legacy path instead of answering with their real contract.
            request_json["protocol"] = json!(1);
            if request_json["request_id"]
                .as_str()
                .is_none_or(str::is_empty)
            {
                let serial = PASSTHROUGH_SERIAL.fetch_add(1, Ordering::Relaxed);
                request_json["request_id"] = json!(format!("rest-{serial}"));
            }
            Plan::Run {
                request: request_json,
                created: 200,
            }
        }
        (method, _) if matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") => {
            let path = request.path.trim_end_matches('/');
            let path = if path.is_empty() { "/" } else { path };
            Plan::Local(
                404,
                json!({"ok": false, "code": "unknown_route", "error": format!("No route {method} {path}")}),
            )
        }
        _ => Plan::Local(
            405,
            json!({"ok": false, "code": "method_not_allowed", "error": format!("{method} not supported here")}),
        ),
    }
}

/// The headless executor: run a resolved plan against this server's private
/// app. The GUI-hosted bridge runs the same plans over its envelope queue.
fn route(
    app: &mut OpenCADStudio,
    request: &HttpRequest,
    document_id: &mut Option<u64>,
    counter: &mut u64,
) -> (u16, Value) {
    let path = request.path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.first() != Some(&"api") || segments.get(1) != Some(&"v1") {
        return (
            404,
            json!({"ok": false, "code": "unknown_route", "error": "Use /api/v1/…"}),
        );
    }
    match plan(&request.method, &segments[2..], request) {
        Plan::Local(status, body) => (status, body),
        Plan::Ready => {
            let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
            harvest(document_id, &state);
            // A file opened from the command line loads on the app's own
            // schedule: until its tab becomes the active document, the
            // active tab is still the welcome (Start) tab, whose id every
            // drawing command refuses. Answering `ready` there hands a
            // client a document id that cannot draw anything, so say
            // "not yet" and let the caller keep polling.
            if state["documents"]
                .as_array()
                .and_then(|documents| {
                    documents
                        .iter()
                        .find(|document| document["id"] == state["document_id"])
                })
                .and_then(|document| document["start"].as_bool())
                == Some(true)
            {
                return (
                    503,
                    json!({
                        "ok": false,
                        "ready": false,
                        "code": "not_ready",
                        "version": state["version"],
                        "session_id": state["session_id"],
                        "document_id": state["document_id"],
                    }),
                );
            }
            (
                200,
                json!({
                    "ok": true,
                    "ready": true,
                    "version": state["version"],
                    "session_id": state["session_id"],
                    "document_id": state["document_id"],
                }),
            )
        }
        Plan::Aliased {
            request,
            created,
            alias,
        } => {
            let mut response = app.automation_op(&request.to_string());
            harvest(document_id, &response);
            apply_alias(alias, &mut response);
            (map_status(response.clone(), created == 201), response)
        }
        Plan::Run { request, created } => {
            let response = app.automation_op(&request.to_string());
            harvest(document_id, &response);
            (map_status(response.clone(), created == 201), response)
        }
        Plan::Mutate {
            op,
            fields,
            created,
        } => run_mutation(app, &op, fields, document_id, counter, created),
    }
}

/// Track the active document from automation responses so later mutations
/// address it without the caller repeating `document_id`.
fn harvest(document_id: &mut Option<u64>, response: &Value) {
    let id = response
        .get("document_id")
        .and_then(Value::as_u64)
        .or_else(|| response["state"]["document_id"].as_u64());
    if let Some(id) = id {
        *document_id = Some(id);
    }
}

/// Wrap an operation in the protocol-1 envelope with a server-generated
/// request_id, address the active document, and — when the dispatcher
/// refuses because our cached state is behind — refresh once and retry.
fn run_mutation(
    app: &mut OpenCADStudio,
    op: &str,
    mut fields: Value,
    document_id: &mut Option<u64>,
    counter: &mut u64,
    created: u16,
) -> (u16, Value) {
    let mut attempt = |app: &mut OpenCADStudio, id: u64, document_id: &mut Option<u64>| -> Value {
        *counter += 1;
        let cached = *document_id;
        let mut envelope = json!({
            "protocol": 1,
            "op": op,
            "request_id": format!("http-{id}-{}", *counter),
        });
        if let Some(id) = cached {
            envelope["document_id"] = json!(id);
        }
        if let Some(object) = fields.as_object_mut() {
            for (key, value) in object.clone() {
                envelope[key] = value;
            }
        }
        let response = app.automation_op(&envelope.to_string());
        // Track the active document before the snapshot is stripped below.
        harvest(document_id, &response);
        compact(response)
    };

    let id = u64::from(std::process::id());
    let mut response = attempt(app, id, document_id);
    let code = response["code"].as_str().unwrap_or("");
    if response["ok"] == false && RETRYABLE.contains(&code) {
        if let Some(state) = fetch_state(app) {
            *document_id = Some(state);
        }
        response = attempt(app, id, document_id);
    }
    let status = map_status(response.clone(), created == 201);
    (if status == 200 { created } else { status }, response)
}

/// Drop the bulky `state` snapshot from settle responses; REST clients read
/// it explicitly from GET /state.
fn compact(response: Value) -> Value {
    if let Some(object) = response.as_object() {
        let trimmed: serde_json::Map<String, Value> = object
            .iter()
            .filter(|(key, _)| key.as_str() != "state")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        return Value::Object(trimmed);
    }
    response
}

fn fetch_state(app: &mut OpenCADStudio) -> Option<u64> {
    let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
    state["document_id"].as_u64()
}

/// ok:true → 200 (the caller may upgrade to 201 for resource creation);
/// failures map to the closest HTTP status so ordinary clients can branch
/// on the status code, with the symbolic `code` preserved in the body.
pub(crate) fn map_status(response: Value, created: bool) -> u16 {
    if response["ok"] == true {
        return if created { 201 } else { 200 };
    }
    match response["code"].as_str().unwrap_or("") {
        "entity_absent" | "layout_missing" | "xref_missing" | "unknown_route" => 404,
        // 要定位的位号哪张图上都没有（项目索引规格 §5.3）。
        "LOCATE_NOT_FOUND" => 404,
        // 同一线号同图第二个流入连接符是「与现有资源冲突」，不是请求格式错。
        // `CONNECTOR_KIND_MISMATCH` 保持 400（契约 A.7 明文）。
        "stale_state"
        | "session_changed"
        | "document_not_active"
        | "gui_required"
        | "DUPLICATE_INFLOW_CONNECTOR"
        // 位号被别的会话占用：与现有的占用冲突，占用释放后同一请求即可成功。
        | "CLAIM_HELD"
        // 主数据与现有的对不上：修订号变了、名称重复、目标已有、还没配置（主数据规格 §3.6）。
        | "MASTERDATA_CONFLICT"
        | "MASTERDATA_DUPLICATE"
        | "MASTERDATA_EXISTS"
        | "MASTERDATA_NOT_CONFIGURED"
        // 项目索引（项目索引规格 §5.3）：图没存过、两个文件同号、位号在几张图上都有，
        // 都是与现在的图纸和项目对不上。
        | "INDEX_NO_PROJECT_DIR"
        | "INDEX_DRAWING_ID_DUPLICATED"
        | "LOCATE_AMBIGUOUS" => 409,
        // 协同会话不可写：请求本身没有错，是这个会话不被允许做这件事。
        "COLLAB_READONLY" | "PERM_DENIED_ROLE" | "PERM_DENIED_PBS" | "PERM_DENIED_STATE" => 403,
        // 项目文件读写失败：请求和图纸都没错，是桌面端这台机器的文件系统报了错。
        "MASTERDATA_IO_ERROR" | "INDEX_IO_ERROR" => 500,
        "busy" => 503,
        _ => 400,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::OpenCADStudio;
    use std::io::BufRead;

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: Vec::new(),
            body: body.as_bytes().to_vec(),
            bearer: None,
        }
    }

    #[test]
    fn percent_decode_handles_escaped_and_plus_spaces() {
        assert_eq!(percent_decode("Walls%20and%2BFloors"), "Walls and+Floors");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%2"), "bad%2");
    }

    #[test]
    fn layout_routes_decode_names_and_build_core_requests() {
        let read = |method: &str, segments: &[&str], path: &str| match plan(
            method,
            segments,
            &request(method, path, ""),
        ) {
            Plan::Run { request, created } => {
                assert_eq!(created, 200);
                request
            }
            _ => panic!("layout read must plan a direct automation request"),
        };
        let mutate = |method: &str, segments: &[&str], path: &str, body: &str| match plan(
            method,
            segments,
            &request(method, path, body),
        ) {
            Plan::Mutate {
                op,
                fields,
                created,
            } => {
                assert_eq!(created, 200);
                (op, fields)
            }
            _ => panic!("layout mutation must use the protocol-1 mutation wrapper"),
        };

        assert_eq!(
            read("GET", &["layouts"], "/api/v1/layouts"),
            json!({"op":"layouts"})
        );
        assert_eq!(
            read(
                "GET",
                &["layouts", "Floor%20Plan", "page-setup"],
                "/api/v1/layouts/Floor%20Plan/page-setup",
            ),
            json!({"op":"page_setup_get","layout":"Floor Plan"})
        );

        let (op, fields) = mutate(
            "PATCH",
            &["layouts", "Floor%20Plan"],
            "/api/v1/layouts/Floor%20Plan",
            r#"{"new_name":"Second Floor"}"#,
        );
        assert_eq!(op, "layout_rename");
        assert_eq!(
            fields,
            json!({"name":"Floor Plan","new_name":"Second Floor"})
        );

        let (op, fields) = mutate(
            "POST",
            &["layouts", "Floor%20Plan", "activate"],
            "/api/v1/layouts/Floor%20Plan/activate",
            "",
        );
        assert_eq!(op, "layout_activate");
        assert_eq!(fields, json!({"name":"Floor Plan"}));

        let (op, fields) = mutate(
            "DELETE",
            &["layouts", "Floor%20Plan"],
            "/api/v1/layouts/Floor%20Plan",
            "",
        );
        assert_eq!(op, "layout_delete");
        assert_eq!(fields, json!({"name":"Floor Plan"}));

        let (op, fields) = mutate(
            "PUT",
            &["layouts", "Floor%20Plan", "page-setup"],
            "/api/v1/layouts/Floor%20Plan/page-setup",
            r#"{"paper":"A4"}"#,
        );
        assert_eq!(op, "page_setup_set");
        assert_eq!(fields, json!({"layout":"Floor Plan","paper":"A4"}));
    }

    #[test]
    fn openapi_documents_structured_automation_routes() {
        let document: Value = serde_json::from_str(include_str!("rest_openapi.json"))
            .expect("REST OpenAPI document parses");
        for (path, method) in [
            ("/api/v1/layouts", "get"),
            ("/api/v1/layouts", "post"),
            ("/api/v1/layouts/{name}", "patch"),
            ("/api/v1/layouts/{name}", "delete"),
            ("/api/v1/layouts/{name}/activate", "post"),
            ("/api/v1/layouts/{name}/page-setup", "get"),
            ("/api/v1/layouts/{name}/page-setup", "put"),
            ("/api/v1/entities_set", "post"),
            ("/api/v1/dgn_import", "post"),
            ("/api/v1/xrefs", "get"),
            ("/api/v1/xref_reload", "post"),
            ("/api/v1/xref_unload", "post"),
            ("/api/v1/xref_detach", "post"),
            ("/api/v1/xref_bind", "post"),
            ("/api/v1/xref_set_path", "post"),
            ("/api/v1/xref_set_type", "post"),
            ("/api/v1/pid/master-data", "get"),
            ("/api/v1/pid/master-data", "put"),
            ("/api/v1/pid/summary", "get"),
            ("/api/v1/pid/index", "get"),
            ("/api/v1/pid/locate", "post"),
        ] {
            assert!(
                document["paths"][path][method].is_object(),
                "missing OpenAPI operation {method} {path}"
            );
        }
        for status in ["403", "409", "500"] {
            assert!(
                document["paths"]["/api/v1/pid/master-data"]["put"]["responses"][status]
                    .is_object(),
                "missing {status} response for put /api/v1/pid/master-data"
            );
        }
        for status in ["404", "409"] {
            assert!(
                document["paths"]["/api/v1/pid/locate"]["post"]["responses"][status].is_object(),
                "missing {status} response for post /api/v1/pid/locate"
            );
        }
        for (path, method) in [
            ("/api/v1/layouts/{name}", "patch"),
            ("/api/v1/layouts/{name}", "delete"),
            ("/api/v1/layouts/{name}/activate", "post"),
            ("/api/v1/layouts/{name}/page-setup", "get"),
            ("/api/v1/layouts/{name}/page-setup", "put"),
        ] {
            assert!(
                document["paths"][path][method]["responses"]["404"].is_object(),
                "missing layout_missing response for {method} {path}"
            );
        }
        for operation in [
            "xref_reload",
            "xref_unload",
            "xref_detach",
            "xref_bind",
            "xref_set_path",
            "xref_set_type",
        ] {
            let path = format!("/api/v1/{operation}");
            assert!(
                document["paths"][&path]["post"]["responses"]["404"].is_object(),
                "missing xref_missing response for POST {path}"
            );
        }
    }

    /// One connection that never sends a request must not keep the listener
    /// from answering the next caller — the whole point of
    /// thread-per-connection (a wedged client used to freeze every read).
    #[test]
    fn listener_answers_while_one_connection_stalls() {
        let bound = Arc::new(AtomicU16::new(0));
        let listen_bound = bound.clone();
        std::thread::spawn(move || {
            // A real drawing, so `ready` answers 200: the point here is the
            // stalled connection, not the welcome tab's not-ready refusal.
            listen(serve_app(true), 0, listen_bound);
        });
        let mut port = 0;
        for _ in 0..50 {
            port = bound.load(Ordering::SeqCst);
            if port != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_ne!(port, 0, "listener never bound a port");

        let status = |authorization: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .write_all(
                    format!("GET /api/v1/ready HTTP/1.1\r\nHost: 127.0.0.1\r\n{authorization}\r\n")
                        .as_bytes(),
                )
                .unwrap();
            let mut status_line = String::new();
            BufReader::new(stream)
                .read_line(&mut status_line)
                .expect("readiness answered");
            status_line
        };
        // Stalls the thread that accepted it: no request line, ever.
        let _stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let answered = status(&format!("Authorization: Bearer {}\r\n", api_token()));
        assert!(answered.contains("200"), "{answered}");
        // Anything else on the machine that finds the port is turned away.
        assert!(status("").contains("401"));
        assert!(status("Authorization: Bearer guess\r\n").contains("401"));
    }

    #[test]
    fn commands_forward_document_id() {
        let plan_for =
            |body: &str| match plan("POST", &["commands"], &request("POST", "/commands", body)) {
                Plan::Run { request, .. } => request,
                _ => panic!("commands must plan a run"),
            };
        assert_eq!(
            plan_for(r#"{"cmd":"LINE 0,0 1,1 ","document_id":2}"#)["document_id"],
            2
        );
        assert!(plan_for(r#"{"cmd":"AUDIT N"}"#)
            .get("document_id")
            .is_none());
    }

    #[test]
    fn failure_codes_map_to_http_statuses() {
        assert_eq!(map_status(json!({"ok": true}), false), 200);
        assert_eq!(map_status(json!({"ok": true}), true), 201);
        assert_eq!(
            map_status(json!({"ok": false, "code": "entity_absent"}), false),
            404
        );
        assert_eq!(
            map_status(json!({"ok": false, "code": "layout_missing"}), false),
            404
        );
        assert_eq!(
            map_status(json!({"ok": false, "code": "xref_missing"}), false),
            404
        );
        assert_eq!(
            map_status(json!({"ok": false, "code": "stale_state"}), false),
            409
        );
        assert_eq!(map_status(json!({"ok": false, "code": "busy"}), false), 503);
        assert_eq!(
            map_status(json!({"ok": false, "code": "invalid_point"}), false),
            400
        );
        // A write refused because the collaboration session may not write is
        // forbidden, not malformed (`VAL-CROSS-004`: "写命令 403 响应").
        for code in [
            "COLLAB_READONLY",
            "PERM_DENIED_ROLE",
            "PERM_DENIED_PBS",
            "PERM_DENIED_STATE",
        ] {
            assert_eq!(
                map_status(json!({"ok": false, "code": code}), false),
                403,
                "{code}"
            );
        }
        // And one refused because somebody else holds the tag conflicts with
        // that claim: the same request succeeds once it is released.
        assert_eq!(
            map_status(json!({"ok": false, "code": "CLAIM_HELD"}), false),
            409
        );
        // Master data (SPEC-pid-master-data §3.6): a clash with what is
        // there is 409; a project file the desktop could not read or write is
        // the server's failure, not the request's; the rest are malformed.
        for code in [
            "MASTERDATA_DUPLICATE",
            "MASTERDATA_CONFLICT",
            "MASTERDATA_EXISTS",
            "MASTERDATA_NOT_CONFIGURED",
        ] {
            assert_eq!(
                map_status(json!({"ok": false, "code": code}), false),
                409,
                "{code}"
            );
        }
        assert_eq!(
            map_status(json!({"ok": false, "code": "MASTERDATA_IO_ERROR"}), false),
            500
        );
        for code in [
            "MASTERDATA_SCHEMA_ERROR",
            "REQUIRED_FIELD_MISSING",
            "MASTERDATA_PBS_INVALID",
            "MASTERDATA_VERSION_UNSUPPORTED",
        ] {
            assert_eq!(
                map_status(json!({"ok": false, "code": code}), false),
                400,
                "{code}"
            );
        }
    }

    /// SPEC-pid-master-data §5.3 over HTTP: GET says which copy is in effect
    /// (with `usage=true`, how many objects use each entry); PUT replaces the
    /// whole master data. A stale `expected_revision` is 409
    /// `MASTERDATA_CONFLICT`, a duplicate name 409, a malformed body 400, and
    /// none of them writes anything.
    #[test]
    fn pid_master_data_routes_read_and_replace() {
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let mut app = serve_app(true);
        let master_data = json!({
            "version": 1,
            "revision": 0,
            "updated_at": 0,
            "legend_standard": null,
            "nominal_diameters": { "unit": "mm", "items": [ { "dn": 80, "label": "DN80" } ] },
            "pipe_classes": [ { "name": "A1A" }, { "name": "B1B" } ],
            "insulation_classes": [],
            "fluid_codes": [],
            "pbs": []
        });
        let put = |body: Value| request("PUT", "/api/v1/pid/master-data", &body.to_string());

        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/pid/master-data", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["state"], "not_configured", "{body}");

        let (status, body) = route(
            &mut app,
            &put(json!({ "target": "drawing", "master_data": master_data })),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["source"], "drawing", "{body}");
        assert_eq!(body["result"]["revision"], 1, "{body}");

        let mut get = request("GET", "/api/v1/pid/master-data", "");
        get.query = vec![("usage".into(), "true".into())];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["state"], "configured", "{body}");
        assert_eq!(body["source"], "drawing", "{body}");
        assert_eq!(
            body["master_data"]["pipe_classes"][1]["name"], "B1B",
            "{body}"
        );
        assert_eq!(body["usage"]["pipe_classes"], json!({}), "{body}");

        let mut duplicate = master_data.clone();
        duplicate["pipe_classes"][1]["name"] = json!("A1A");
        for (request_body, expected_status, code) in [
            (
                json!({ "master_data": master_data, "expected_revision": 0 }),
                409,
                "MASTERDATA_CONFLICT",
            ),
            (
                json!({ "master_data": duplicate }),
                409,
                "MASTERDATA_DUPLICATE",
            ),
            (
                json!({ "master_data": { "version": 1 } }),
                400,
                "MASTERDATA_SCHEMA_ERROR",
            ),
            // An unsaved drawing has no folder to hold a project file.
            (
                json!({ "target": "project", "master_data": master_data }),
                400,
                "invalid_request",
            ),
        ] {
            let (status, body) =
                route(&mut app, &put(request_body), &mut document_id, &mut counter);
            assert_eq!(status, expected_status, "{body}");
            assert_eq!(body["code"], code, "{body}");
        }

        let (_, body) = route(
            &mut app,
            &request("GET", "/api/v1/pid/master-data", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(body["master_data"]["revision"], 1, "{body}");
        assert_eq!(
            body["master_data"]["pipe_classes"][1]["name"], "B1B",
            "{body}"
        );
        assert!(body.get("usage").is_none(), "{body}");
    }

    /// SPEC-pid-project-index §5.3 over HTTP: GET /pid/summary is the current
    /// drawing's summary, GET /pid/index the project index (`tag` and
    /// `type_code` from the query), POST /pid/locate a write that may switch
    /// tabs. An unsaved drawing belongs to no project: 409
    /// `INDEX_NO_PROJECT_DIR`. A tag nowhere is 404, on several drawings 409,
    /// a project folder that cannot be read 500.
    #[test]
    fn pid_index_rest_routes_and_status_codes() {
        let mut index = request("GET", "/api/v1/pid/index", "");
        index.query = vec![
            ("tag".into(), "P-1001".into()),
            ("type_code".into(), "EQUIPMENT".into()),
        ];
        let Plan::Run {
            request: asked,
            created,
        } = plan("GET", &["pid", "index"], &index)
        else {
            panic!("GET /pid/index is not a read");
        };
        assert_eq!(created, 200);
        assert_eq!(
            asked,
            json!({"protocol": 1, "op": "pid_index", "tag": "P-1001", "type_code": "EQUIPMENT"})
        );
        let bare = request("GET", "/api/v1/pid/index", "");
        let Plan::Run { request: asked, .. } = plan("GET", &["pid", "index"], &bare) else {
            panic!("GET /pid/index is not a read");
        };
        assert_eq!(asked, json!({"protocol": 1, "op": "pid_index"}));
        let summary = request("GET", "/api/v1/pid/summary", "");
        let Plan::Run { request: asked, .. } = plan("GET", &["pid", "summary"], &summary) else {
            panic!("GET /pid/summary is not a read");
        };
        assert_eq!(asked, json!({"protocol": 1, "op": "pid_summary"}));
        let locate = request(
            "POST",
            "/api/v1/pid/locate",
            r#"{"tag":"P-1001","drawing_id":"PID-1102"}"#,
        );
        let Plan::Mutate {
            op,
            fields,
            created,
        } = plan("POST", &["pid", "locate"], &locate)
        else {
            panic!("POST /pid/locate is not a write");
        };
        assert_eq!((op.as_str(), created), ("pid_locate", 200));
        assert_eq!(fields, json!({"tag": "P-1001", "drawing_id": "PID-1102"}));

        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let mut app = serve_app(true);
        let (status, body) = route(&mut app, &summary, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["summary"]["drawing_id"], "", "{body}");
        let mut typed = request("GET", "/api/v1/pid/index", "");
        typed.query = vec![("type_code".into(), "EQUIPMENT".into())];
        for (asked, expected_status, code) in [
            (bare, 409, "INDEX_NO_PROJECT_DIR"),
            (typed, 400, "invalid_request"),
            (locate, 409, "INDEX_NO_PROJECT_DIR"),
            (
                request("POST", "/api/v1/pid/locate", "{}"),
                400,
                "invalid_request",
            ),
        ] {
            let (status, body) = route(&mut app, &asked, &mut document_id, &mut counter);
            let what = format!("{} {}?{:?} {body}", asked.method, asked.path, asked.query);
            assert_eq!(status, expected_status, "{what}");
            assert_eq!(body["code"], code, "{what}");
        }

        for (code, expected_status) in [
            ("INDEX_NO_PROJECT_DIR", 409),
            ("INDEX_DRAWING_ID_DUPLICATED", 409),
            ("LOCATE_AMBIGUOUS", 409),
            ("LOCATE_NOT_FOUND", 404),
            ("INDEX_IO_ERROR", 500),
        ] {
            assert_eq!(
                map_status(json!({"ok": false, "code": code}), false),
                expected_status,
                "{code}"
            );
        }
    }

    /// `ready` must not advertise the welcome (Start) tab: that tab answers
    /// `ready` but every drawing command refuses its id, so a client that
    /// trusted the answer would see `n1 == n0` on every run. The listener
    /// says `503 not_ready` until a real drawing is the active document.
    #[test]
    fn ready_refuses_the_start_tab_until_a_drawing_is_active() {
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let mut app = OpenCADStudio::new_headless(true);
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/ready", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 503, "{body}");
        assert_eq!(body["ready"], false, "{body}");
        assert_eq!(body["code"], "not_ready", "{body}");

        // Once the welcome tab is a real drawing, the same call is ready.
        let mut app = serve_app(true);
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/ready", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ready"], true, "{body}");
    }

    /// `GET /documents` answers the state read, but the contract reads
    /// `documents[].document_id`; both spellings must be present.
    #[test]
    fn documents_carries_document_id_alongside_id() {
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let mut app = serve_app(true);
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/documents", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        let documents = body["documents"].as_array().expect("documents array");
        assert!(!documents.is_empty(), "{body}");
        for document in documents {
            assert_eq!(document["document_id"], document["id"], "{body}");
        }
        let ready_id = body["document_id"].as_u64().expect("active document id");
        assert!(
            documents
                .iter()
                .any(|document| document["document_id"].as_u64() == Some(ready_id)),
            "{body}"
        );
    }

    /// `GET /entities` counts entities under `count`; the contract reads
    /// `total`, so the route answers both.
    #[test]
    fn entities_answers_total_alongside_count() {
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let mut app = serve_app(true);
        app.automation_op(r#"{"op":"run","cmd":"CIRCLE 0,0 5"}"#);
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/entities", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["total"], body["count"], "{body}");
        assert_eq!(body["total"], 1, "{body}");
    }

    /// `POST /capture` answers through the operation envelope, so its fields
    /// arrive under `result`; the contract reads `path`/`width`/`height`/
    /// `world_to_pixel_matrix`/`annotations` at the top level.
    #[test]
    fn capture_hoists_its_envelope_fields_to_the_top_level() {
        let plan = plan(
            "POST",
            &["capture"],
            &request(
                "POST",
                "/api/v1/capture",
                r#"{"document_id":2,"annotate":true,"path":"D:\\scratch\\x.png"}"#,
            ),
        );
        let Plan::Aliased { request, alias, .. } = plan else {
            panic!("POST /capture plans an aliased run")
        };
        assert!(matches!(alias, Alias::Capture));
        assert_eq!(request["op"], "capture");
        assert_eq!(request["protocol"], 1);
        assert!(request["request_id"].is_string(), "{request}");

        let mut response = json!({
            "ok": true,
            "status": "completed",
            "result": {
                "path": "D:\\scratch\\x.png",
                "width": 1600,
                "height": 795,
                "world_to_pixel_matrix": [1.0, 0.0, 0.0, -1.0, 10.0, 20.0],
                "annotations": [{"handle": "1A", "screen_pixel": [10, 20]}],
                "state": {"ok": true}
            }
        });
        apply_alias(Alias::Capture, &mut response);
        assert_eq!(response["path"], "D:\\scratch\\x.png");
        assert_eq!(response["width"], 1600);
        assert_eq!(response["world_to_pixel_matrix"].as_array().unwrap().len(), 6);
        assert_eq!(response["annotations"].as_array().unwrap().len(), 1);
        assert!(response.get("state").is_none(), "{response}");
    }

    /// `POST /run` keeps the op its body names. The generic passthrough
    /// overwrites `op` with the path segment, which turned
    /// `{"op":"cancel"}` into a `run` and made it die on `command_busy`
    /// while an insertion was live (`VAL-DRAW-017`); the dedicated route
    /// must carry `cancel` through untouched.
    #[test]
    fn run_route_keeps_the_op_the_body_names() {
        let Plan::Run { request: cancel, .. } = plan(
            "POST",
            &["run"],
            &request(
                "POST",
                "/api/v1/run",
                r#"{"document_id":7,"op":"cancel"}"#,
            ),
        ) else {
            panic!("POST /run plans a run request")
        };
        assert_eq!(cancel["op"], "cancel", "{cancel}");
        assert_eq!(cancel["document_id"], 7, "{cancel}");
        assert_eq!(cancel["protocol"], 1, "{cancel}");
        assert!(
            cancel["request_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "{cancel}"
        );

        // A body with no op is still the plain command run it always was.
        let Plan::Run {
            request: plain, ..
        } = plan(
            "POST",
            &["run"],
            &request("POST", "/api/v1/run", r#"{"cmd":"LINE 0,0 10,0"}"#),
        ) else {
            panic!("POST /run plans a run request")
        };
        assert_eq!(plain["op"], "run", "{plain}");
        assert_eq!(plain["cmd"], "LINE 0,0 10,0", "{plain}");
    }

    /// `run` is not a query: the dispatcher refuses it without
    /// `document_id`, so the convenience route must forward the one the
    /// caller sent — same handling as undo/redo.
    #[test]
    fn commands_run_forwards_document_id() {
        let Plan::Run { request, .. } = plan(
            "POST",
            &["commands"],
            &request(
                "POST",
                "/api/v1/commands",
                r#"{"cmd":"LINE 0,0 200,200","document_id":2}"#,
            ),
        ) else {
            panic!("POST /commands plans a run request")
        };
        assert_eq!(request["op"], "run");
        assert_eq!(request["cmd"], "LINE 0,0 200,200");
        assert_eq!(request["document_id"], 2);
    }

    /// `--http` without a file listens on `serve_app`: `new_headless`
    /// alone lands on the welcome (Start) tab, whose `is_start` flag makes
    /// the protocol-1 gate refuse `run` with `no_document` — a client saw
    /// `ready` answer with a document id while every command was dropped.
    /// The session `serve()` builds has that tab converted to a real
    /// drawing, so the identical request draws.
    #[test]
    fn headless_session_runs_commands_after_start_tab_conversion() {
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let entities = |app: &mut OpenCADStudio| -> u64 {
            let summary = app.automation_op(r#"{"op":"entities"}"#);
            summary["total"].as_u64().unwrap_or(0)
        };

        // The session before serve()'s conversion: readiness refuses to call
        // the welcome tab ready, because every drawing command would drop.
        let mut app = OpenCADStudio::new_headless(true);
        let (status, ready) = route(
            &mut app,
            &request("GET", "/api/v1/ready", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 503, "{ready}");
        assert_eq!(ready["code"], "not_ready", "{ready}");
        // The id is still discoverable through the state read, and `run`
        // against it dies at the Start-tab gate — the half-state the listener
        // must not advertise.
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        let doc = state["document_id"].as_u64().expect("a document id exists");
        let before = entities(&mut app);
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/run",
                &format!(r#"{{"document_id":{doc},"op":"run","cmd":"CIRCLE 0,0 5"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["ok"], false, "{body}");
        assert_eq!(body["code"], "no_document", "{body}");
        assert_eq!(entities(&mut app), before, "{body}");

        // The session serve() hands to listen(): the same discovery, the
        // same request — and the circle lands.
        let mut app = serve_app(true);
        let (status, ready) = route(
            &mut app,
            &request("GET", "/api/v1/ready", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{ready}");
        let doc = document_id.expect("ready caches the document id");
        let before = entities(&mut app);
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/run",
                &format!(r#"{{"document_id":{doc},"op":"run","cmd":"CIRCLE 0,0 5"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], true, "{body}");
        assert_eq!(body["status"], "completed", "{body}");
        assert_eq!(entities(&mut app), before + 1, "{body}");
    }

    /// `VAL-DRAW-017` through the route the contract names: while an
    /// insertion is live, `POST /api/v1/run` with `{"op":"cancel"}` must
    /// escape it and answer `cancelled` — not `command_busy`, which is what
    /// the generic passthrough produced by rewriting the op to `run`.
    #[test]
    fn run_cancel_escapes_a_live_command() {
        let mut counter: u64 = 0;
        let mut app = serve_app(true);
        let doc = app.automation_op(r#"{"protocol":1,"op":"state"}"#)["document_id"]
            .as_u64()
            .expect("a document id exists");
        let mut document_id: Option<u64> = Some(doc);
        let entities = |app: &mut OpenCADStudio| -> u64 {
            app.automation_op(r#"{"op":"entities"}"#)["total"]
                .as_u64()
                .unwrap_or(0)
        };
        let blocks = |app: &mut OpenCADStudio| -> u64 {
            app.automation_op(r#"{"op":"entities"}"#)["by_type"]["Block Reference"]
                .as_u64()
                .unwrap_or(0)
        };
        let before = entities(&mut app);
        let blocks_before = blocks(&mut app);

        // A live command: `PIDINSERT` parks on its point step.
        let (status, started) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/run",
                &format!(r#"{{"document_id":{doc},"op":"run","cmd":"PIDINSERT vessel"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{started}");
        assert_eq!(started["ok"], true, "{started}");
        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        assert!(
            !state["command"].is_null(),
            "PIDINSERT 应停在取点步骤：{state}"
        );

        // The point without the confirming Enter: the preview hangs here.
        let (status, picked) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/input",
                &format!(
                    r#"{{"document_id":{doc},"op":"input","kind":"point","point":[200,150,0]}}"#
                ),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{picked}");
        assert_eq!(picked["ok"], true, "{picked}");

        // The contract's cancel: same route, op in the body.
        let (status, cancelled) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/run",
                &format!(r#"{{"document_id":{doc},"op":"cancel"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{cancelled}");
        assert_eq!(cancelled["ok"], true, "{cancelled}");
        assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
        assert_ne!(cancelled["code"], "command_busy", "{cancelled}");

        let state = app.automation_op(r#"{"protocol":1,"op":"state"}"#);
        assert!(state["command"].is_null(), "取消后无活动命令：{state}");
        assert_eq!(entities(&mut app), before, "取消后实体数不变");
        assert_eq!(blocks(&mut app), blocks_before, "取消后 Block Reference 计数不变");

        // A later insertion still commits: the escape left nothing behind.
        let (status, started) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/run",
                &format!(r#"{{"document_id":{doc},"op":"run","cmd":"PIDINSERT vessel"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{started}");
        let (status, picked) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/input",
                &format!(
                    r#"{{"document_id":{doc},"op":"input","kind":"point","point":[250,250,0]}}"#
                ),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{picked}");
        let (status, entered) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/input",
                &format!(r#"{{"document_id":{doc},"op":"input","kind":"enter"}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{entered}");
        assert_eq!(entered["ok"], true, "{entered}");
        assert_eq!(entities(&mut app), before + 1, "取消后再插入应提交一个实体");
        assert_eq!(blocks(&mut app), blocks_before + 1, "新实体是一个 Block Reference");
    }

    #[test]
    fn xrefs_listing_uses_the_structured_read() {
        match plan("GET", &["xrefs"], &request("GET", "/api/v1/xrefs", "")) {
            Plan::Run { request, created } => {
                assert_eq!(created, 200);
                assert_eq!(request, json!({"op":"xrefs"}));
            }
            _ => panic!("xrefs must plan a direct read"),
        }
    }

    #[test]
    fn entity_and_dgn_passthroughs_use_protocol_envelopes() {
        for (operation, body) in [
            ("entities_set", r#"{"handles":["2A"],"color":"ByLayer"}"#),
            ("dgn_import", r#"{"path":"site.dgn"}"#),
            ("xref_set_type", r#"{"id":"2A","reference_type":"overlay"}"#),
        ] {
            let path = format!("/api/v1/{operation}");
            match plan("POST", &[operation], &request("POST", &path, body)) {
                Plan::Mutate { op, .. } => assert_eq!(op, operation),
                _ => panic!("{operation} must use the protocol-1 mutation wrapper"),
            }
        }
    }

    /// The generic `POST /api/v1/{op}` passthrough must reach the protocol-1
    /// envelope: ops like `user_select`/`operation`/`cancel` only exist
    /// there, and unstamped they died as "unknown op" on the legacy path.
    #[test]
    fn passthrough_reaches_envelope_only_ops() {
        let mut app = OpenCADStudio::new();
        app.automation_op(r#"{"op":"new"}"#);
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/user_select",
                r#"{"request_id":"us-headless"}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 409, "{body}");
        assert_eq!(body["code"], "gui_required", "{body}");

        // `operation` is a query: an unknown id answers `unknown_operation`,
        // not "unknown op".
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/operation", r#"{"request_id":"gone"}"#),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["code"], "unknown_operation", "{body}");
    }

    #[test]
    fn get_selection_reports_selected_entities_read_only() {
        let mut app = OpenCADStudio::new();
        let mut document_id: Option<u64> = None;
        let mut counter: u64 = 0;
        let (_, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/entities",
                r#"{"entities":[
                    {"type":"Line","start":[0,0],"end":[10,0]},
                    {"type":"Text","value":"PAGE-01","position":[1,1],"height":2.5}
                ]}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        let handles = body["result"]["handles"].as_array().unwrap().clone();
        assert_eq!(handles.len(), 2);

        // The person "picks" the line and the text; the read mirrors the
        // selection order and matches the `query` field formats.
        app.automation_op(&format!(
            r#"{{"op":"select","handles":["{}","{}"]}}"#,
            handles[0].as_str().unwrap(),
            handles[1].as_str().unwrap()
        ));
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/get_selection", "{}"),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["status"], "completed");
        assert_eq!(body["result"]["count"], 2);
        assert_eq!(
            body["result"]["entities"][0]["handle"],
            handles[0].as_str().unwrap()
        );
        assert_eq!(body["result"]["entities"][0]["type"], "Line");
        assert_eq!(body["result"]["entities"][0]["text"], Value::Null);
        let sample = &body["result"]["entities"][1];
        assert_eq!(sample["type"], "Text");
        assert_eq!(sample["text"], "PAGE-01");
        assert_eq!(sample["value"], "PAGE-01");
        assert!(sample["bounds"].is_object(), "{sample}");
        // A second read is identical — the read is passive.
        let (_, again) = route(
            &mut app,
            &request("POST", "/api/v1/get_selection", "{}"),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(again, body);
    }

    #[test]
    fn rest_surface_round_trips_real_entities() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        let mut document_id = None;
        let mut counter = 0;

        // Readiness exposes the version and session.
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/ready", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert!(body["session_id"].as_str().is_some());

        // Create: a REST resource POST returns 201 with handles.
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/entities",
                r#"{"entities":[
                    {"type":"Line","start":[0,0],"end":[10,0],"layer":"Walls"},
                    {"type":"Circle","center":[5,5],"radius":2},
                    {"type":"Text","value":"PAGE-01","position":[1,1],"height":2.5}
                ]}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        let handles = body["result"]["handles"].as_array().unwrap().clone();
        assert_eq!(handles.len(), 3);
        assert!(document_id.is_some(), "the server cached the document id");

        // Query through plain GET with query parameters.
        let mut get = request("GET", "/api/v1/entities", "");
        get.query = vec![
            ("type".into(), "Line".into()),
            ("detail".into(), "full".into()),
        ];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200);
        assert_eq!(body["entities"][0]["start"], json!([0.0, 0.0, 0.0]));

        // One entity by handle.
        let (status, body) = route(
            &mut app,
            &request(
                "GET",
                &format!("/api/v1/entities/{}", handles[1].as_str().unwrap()),
                "",
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200);
        assert_eq!(body["count"], 1);
        assert_eq!(body["entities"][0]["type"], "Circle");

        // Transform through the dedicated endpoint.
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/entities/transform",
                &format!(
                    r#"{{"handles":["{}"],"action":"move","vector":[5,0]}}"#,
                    handles[1].as_str().unwrap()
                ),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        let mut get = request("GET", "/api/v1/entities", "");
        get.query = vec![
            ("type".into(), "Circle".into()),
            ("detail".into(), "full".into()),
        ];
        let (_, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(body["entities"][0]["center"], json!([10.0, 5.0, 0.0]));

        // XData write via PUT, read via GET.
        let handle = handles[0].as_str().unwrap().to_owned();
        let (status, body) = route(
            &mut app,
            &request(
                "PUT",
                &format!("/api/v1/entities/{handle}/xdata/SPM"),
                r#"[{"code":1000,"value":"PAGE-01"}]"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        let mut get = request("GET", &format!("/api/v1/entities/{handle}/xdata"), "");
        get.query = vec![("app".into(), "SPM".into())];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["items"][0]["xdata"]["SPM"][0], "PAGE-01");

        // Unknown handles are 404 with the symbolic code preserved.
        let mut delete = request("DELETE", "/api/v1/entities", "");
        delete.query = vec![("handles".into(), "FF".into())];
        let (status, body) = route(&mut app, &delete, &mut document_id, &mut counter);
        assert_eq!(status, 404);
        assert_eq!(body["code"], "entity_absent");

        // The OpenAPI document is served and parses as JSON.
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/openapi", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200);
        assert_eq!(body["openapi"], "3.0.3");

        // Unknown routes are 404, not panics.
        let (status, _) = route(
            &mut app,
            &request("GET", "/api/v1/nope", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 404);
    }

    #[test]
    fn rest_p1_document_surface_routes_end_to_end() {
        let mut app = OpenCADStudio::new_for_test();
        app.automation_op(r#"{"op":"new"}"#);
        app.automation_op(r#"{"op":"run","cmd":"CIRCLE 0,0 1"}"#);
        app.automation_op(r#"{"op":"run","cmd":"CIRCLE 20,0 5"}"#);
        let mut document_id = None;
        let mut counter = 0;

        // GET /state primes the document-id cache the mutations address.
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/state", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200);
        let doc = document_id.expect("state caches the document id");
        assert_eq!(body["document_id"].as_u64(), Some(doc));

        // Where filters travel as the JSON-encoded `where` query parameter.
        let mut get = request("GET", "/api/v1/entities", "");
        get.query = vec![
            ("type".into(), "Circle".into()),
            ("detail".into(), "geometry".into()),
            (
                "where".into(),
                r#"[{"path":"/radius","op":"gt","value":2}]"#.into(),
            ),
        ];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["count"], 1);
        assert_eq!(body["entities"][0]["radius"], 5.0);

        // A malformed filter is refused before touching the dispatcher.
        let mut get = request("GET", "/api/v1/entities", "");
        get.query = vec![("where".into(), "not-json".into())];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 400);
        assert_eq!(body["code"], "invalid_where");

        // Sysvars read through GET with a `names` CSV, write through POST.
        let mut get = request("GET", "/api/v1/sysvars", "");
        get.query = vec![("names".into(), "ltscale,mirrtext".into())];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert!(body["result"]["values"]["ltscale"].is_number());
        assert_eq!(body["result"]["values"]["mirrtext"], 0);

        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/sysvars", r#"{"set":{"ltscale":2.5}}"#),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        let mut get = request("GET", "/api/v1/sysvars", "");
        get.query = vec![("names".into(), "ltscale".into())];
        let (_, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(body["result"]["values"]["ltscale"], 2.5);

        // A layout is created (201) and its page setup patched via PUT.
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/layouts", r#"{"name":"PLAN"}"#),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        assert!(body["result"]["layouts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "PLAN"));
        let (status, body) = route(
            &mut app,
            &request(
                "PUT",
                "/api/v1/layouts/PLAN/page-setup",
                r#"{"paper":"ISO_A4_(210.00_x_297.00_MM)","orientation":"landscape","fit":true,"center":true}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["layout"], "PLAN");
        assert!(body["result"]["paper"].as_str().unwrap().contains("ISO_A4"));
        assert_eq!(body["result"]["scale"]["fit"], true);

        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/layouts", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert!(body["layouts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|layout| layout["name"] == "PLAN"));
        let (status, body) = route(
            &mut app,
            &request(
                "PATCH",
                "/api/v1/layouts/plan",
                r#"{"new_name":"Floor Plan"}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["new_name"], "Floor Plan");
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/layouts/floor%20plan/activate", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["current"], "Floor Plan");
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/layouts/Floor%20Plan/page-setup", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["layout"], "Floor Plan");
        assert_eq!(body["page_setup"]["orientation"], "landscape");
        let (status, body) = route(
            &mut app,
            &request("DELETE", "/api/v1/layouts/Floor%20Plan", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["current"], "Model");
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/layouts/Floor%20Plan/page-setup", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["code"], "layout_missing");

        // Groups and selection sets address the drawing's entities (the
        // created layout also owns a sheet viewport, so filter to circles).
        let q = app.automation_op(r#"{"op":"query","type":"Circle","detail":"summary"}"#);
        let joined = q["entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("\"{}\"", e["handle"].as_str().unwrap()))
            .collect::<Vec<_>>()
            .join(",");
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/groups",
                &format!(r#"{{"name":"FRAME","handles":[{joined}]}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/selection-sets",
                &format!(r#"{{"name":"rest-set","handles":[{joined}]}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        app.automation_op(r#"{"op":"select","clear":true}"#);
        // Default GET recalls without touching the selection; ?select=true
        // additionally makes the set the current selection.
        let (status, body) = route(
            &mut app,
            &request("GET", "/api/v1/selection-sets/rest-set", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["handles"].as_array().unwrap().len(), 2);
        assert_eq!(body["result"]["selected"], 0);
        let mut get = request("GET", "/api/v1/selection-sets/rest-set", "");
        get.query = vec![("select".into(), "true".into())];
        let (status, body) = route(&mut app, &get, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["selected"], 2);

        // Copy between documents: the body names the target document.
        let target = app.push_test_document();
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/entities/copy-to",
                &format!(r#"{{"handles":[{joined}],"document_id":{target}}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        assert_eq!(body["result"]["count"], 2);

        // Blocks: define → replace → delete; the drawing carries a stable
        // file identity GUID.
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/blocks",
                &format!(r#"{{"name":"MARK","base":[0,0,0],"handles":[{joined}]}}"#),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        // Replace re-blocks fresh content under the same name — the define
        // consumed the original circles, SPM-style re-import draws new
        // content and wraps it again.
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/entities",
                r#"{"entities":[{"type":"Point","location":[9,9]}]}"#,
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        let fresh = body["result"]["handles"][0].as_str().unwrap().to_owned();
        let (status, body) = route(
            &mut app,
            &request(
                "POST",
                "/api/v1/blocks",
                &format!(
                    r#"{{"name":"MARK","base":[0,0,0],"handles":["{fresh}"],"replace":true}}"#
                ),
            ),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        // A definition the insert still points at is refused without force.
        let (status, body) = route(
            &mut app,
            &request("DELETE", "/api/v1/blocks/MARK", ""),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["code"], "block_referenced", "{body}");
        // `?force=true` erases the referencing insert in the same step.
        let mut forced = request("DELETE", "/api/v1/blocks/MARK", "");
        forced.query = vec![("force".into(), "true".into())];
        let (status, body) = route(&mut app, &forced, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["references"], 1, "{body}");
        assert!(body["result"]["erased"].as_u64().unwrap() >= 1);
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/file-identity", "{}"),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["identity"].as_str().unwrap().len(), 36);

        // An empty POST /documents opens a fresh
        // untitled document in its own tab — a copy-to target.
        let (status, body) = route(
            &mut app,
            &request("POST", "/api/v1/documents", "{}"),
            &mut document_id,
            &mut counter,
        );
        assert_eq!(status, 201, "{body}");
        let fresh = document_id.expect("fresh document cached");
        assert_ne!(fresh, doc, "a second document opened");

        // Close with discard: the dirty document is dropped, 200 returned.
        let mut delete = request("DELETE", &format!("/api/v1/documents/{doc}"), "");
        delete.query = vec![("discard".into(), "true".into())];
        let (status, body) = route(&mut app, &delete, &mut document_id, &mut counter);
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"]["closed"], true);
    }

    /// A connected pair: the test writes the request from `client`,
    /// `read_request` parses it from `server`.
    fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    /// Stream `payload` from a clone of `client` on its own thread: the
    /// bytes exceed any socket buffer, so a single thread would deadlock
    /// writing before `read_request` drains the other end. Write failures
    /// after a refusal are expected and ignored.
    fn spawn_writer(client: &TcpStream, payload: Vec<u8>) -> std::thread::JoinHandle<()> {
        let mut writer = client.try_clone().unwrap();
        std::thread::spawn(move || {
            let _ = writer.write_all(&payload);
            let _ = writer.shutdown(std::net::Shutdown::Write);
        })
    }

    /// Read the status line of the answer the server just wrote.
    fn read_status(client: TcpStream) -> String {
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(client);
        let mut status = String::new();
        reader.read_line(&mut status).expect("server answers");
        status
    }

    /// A request line streamed without CRLF must be refused at the cap
    /// instead of growing one `String` to gigabytes — an allocation failure
    /// aborts the process and is not catchable.
    #[test]
    fn oversize_request_line_is_refused_with_431() {
        let (client, mut server) = loopback_pair();
        let mut payload = b"GET /".to_vec();
        payload.extend(std::iter::repeat(b'A').take(1024 * 1024));
        payload.extend_from_slice(b" HTTP/1.1\r\n\r\n");
        let writer = spawn_writer(&client, payload);

        let parsed =
            read_request(&mut server).expect("an over-cap line is a refusal, not an I/O error");
        assert!(
            parsed.is_none(),
            "over-cap request line must be refused, got a parsed request"
        );

        let status = read_status(client);
        assert!(status.contains("431"), "expected 431, got: {status:?}");
        drop(server);
        writer.join().unwrap();
    }

    /// Individually-fine headers must not add up to an unbounded header
    /// block: a client can send an unlimited number of small lines.
    #[test]
    fn oversize_header_block_is_refused_with_431() {
        let (client, mut server) = loopback_pair();
        let mut payload = b"GET /api/v1/state HTTP/1.1\r\n".to_vec();
        for _ in 0..80 {
            payload.extend_from_slice(b"X-Pad: ");
            payload.extend(std::iter::repeat(b'a').take(1024));
            payload.extend_from_slice(b"\r\n");
        }
        payload.extend_from_slice(b"\r\n");
        let writer = spawn_writer(&client, payload);

        let parsed = read_request(&mut server)
            .expect("an over-cap header block is a refusal, not an I/O error");
        assert!(
            parsed.is_none(),
            "over-cap header block must be refused, got a parsed request"
        );

        let status = read_status(client);
        assert!(status.contains("431"), "expected 431, got: {status:?}");
        drop(server);
        writer.join().unwrap();
    }
}

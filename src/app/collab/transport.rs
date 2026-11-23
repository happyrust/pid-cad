//! Synchronous WebSocket transport for the desktop client.
//!
//! `tungstenite` in blocking mode over a `TcpStream` with an explicit connect
//! timeout, driven from a `std::thread` and reported through an iced futures
//! channel. No async runtime is involved (`VAL-CLI-019`).
//!
//! Five constraints are load-bearing, each of them learned from a hang:
//!
//! 1. `set_read_timeout(100ms)` after connecting — without it `read()` blocks
//!    forever, the stop flag is never observed and `join()` never returns.
//! 2. A read timeout (`WouldBlock` / `TimedOut`) is a normal branch: `continue`.
//! 3. `Message::Close` must `break` immediately — `Close` is delivered once and
//!    the next `read()` reports `ConnectionAborted` instead.
//! 4. A peer killed with no close frame surfaces as
//!    `Io(ConnectionReset)`, not `ConnectionClosed`; every `Err` means the
//!    connection is gone and must go through the retry path (`VAL-CLI-020`).
//! 5. `TcpStream::connect_timeout` + `tungstenite::client` — never
//!    `tungstenite::connect`, which has no timeout (21 s against a black hole).

use super::messages::{read, Frame};
use super::{ClientConfig, CollabEvent, TagsSaid};
use iced::futures::channel::mpsc;
use ocs_collab::ClientMessage;
use std::io::{ErrorKind, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

/// Read timeout once connected. Short enough that a cancelled subscription is
/// noticed promptly, long enough that an idle session costs no CPU.
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Connect timeout. A refused connection returns at once on Windows (~2 s);
/// this bounds the black-hole case.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Heartbeat cadence the server expects (see `architecture.md` §3.1).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// Reconnect backoff: 200 ms doubling to a 5 s ceiling, matching the
/// documented retry ladder.
const BACKOFF_START: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How long one wait-for-retry slice sleeps before re-checking whether the
/// subscription is still alive.
const BACKOFF_SLICE: Duration = Duration::from_millis(50);

/// How long [`emit`] waits before offering the UI an event again when its
/// channel was full.
const EMIT_RETRY: Duration = Duration::from_millis(2);

/// How long a dial waits for the UI to say which tags the session's drawing
/// carries before its `hello` goes without them. The UI answers within a
/// frame once the drawing is there; the wait is for a drawing the launch is
/// still loading, and a large one takes seconds.
const TAGS_WAIT: Duration = Duration::from_secs(20);

/// How often that wait looks for the answer.
const TAGS_POLL: Duration = Duration::from_millis(5);

/// How long a session that is left on purpose, to join again, stays on the
/// line for the server to take its goodbye ([`leave`]). The server takes it
/// within milliseconds; the wait is for one that is busy, and one that
/// never answers holds the client no longer than a few retries would.
const LEAVE_WAIT: Duration = Duration::from_secs(2);

/// The outbound queue: requests the UI wants on the wire, oldest first.
type Outbox = Mutex<std::collections::VecDeque<Outbound>>;

/// Where the UI leaves its answer to the transport's question about the
/// drawing's tags ([`CollabEvent::TagsWanted`]): the last thing it said,
/// with the number of the question that answers.
type TagsSlot = Mutex<Option<(u64, TagsSaid)>>;

/// How a client asks the UI which tags the session's drawing carries: where
/// the answer is left, and how long a dial waits for it.
#[derive(Clone)]
struct TagsLine {
    said: Arc<TagsSlot>,
    wait: Duration,
}

impl TagsLine {
    /// What the UI has said to question `ask`, once it has said anything to
    /// it. An answer to another question is not one to this.
    fn answer(&self, ask: u64) -> Option<TagsSaid> {
        let slot = self.said.lock().ok()?;
        match &*slot {
            Some((answered, said)) if *answered == ask => Some(said.clone()),
            _ => None,
        }
    }
}

/// What one transport thread works with: where its session parameters come
/// from, the requests waiting to go out, the signal that tells it to quit, and
/// its heartbeat cadence.
///
/// The app's client is wired to the two process-wide slots ([`Link::app`]):
/// iced starts the subscription worker as a bare `fn`, so a slot is the only
/// way its parameters can reach it. Those slots are the app client's and
/// nobody else's. In a test binary every app that gets built republishes its
/// (empty) config to them and any test may queue a claim on them, so a client
/// is always handed its link, and one that is not the app's gets a link that
/// shares nothing ([`Link::detached`]).
#[derive(Clone)]
pub(super) struct Link {
    config: Arc<Mutex<ClientConfig>>,
    outbox: Arc<Outbox>,
    /// Raised by [`shutdown`]. Unlike a dropped subscription it asks the
    /// thread to say goodbye on the wire. It belongs to this link alone: it
    /// used to be one process-wide flag that `shutdown` set and nothing
    /// cleared, which read as "the app is quitting" to every client started
    /// after the first quit.
    stop: Arc<AtomicBool>,
    heartbeat: Duration,
    /// How long a session that is left to join again stays on the line for
    /// the server to take its goodbye ([`leave`]).
    leave_wait: Duration,
    /// The way to ask the UI which tags the session's drawing carries, for
    /// a client that has a UI to ask. One that has none says `hello` without
    /// them and waits for nobody.
    tags: Option<TagsLine>,
}

impl Link {
    /// The link of the app's own client: the published config, the queue the
    /// UI fills, and a quit signal of its own.
    pub(super) fn app() -> Self {
        Self {
            config: Arc::clone(app_config()),
            outbox: Arc::clone(app_outbox()),
            stop: Arc::new(AtomicBool::new(false)),
            heartbeat: HEARTBEAT_INTERVAL,
            leave_wait: LEAVE_WAIT,
            tags: Some(TagsLine {
                said: Arc::clone(app_tags()),
                wait: TAGS_WAIT,
            }),
        }
    }

    /// A link that shares nothing with the app's client.
    #[cfg(test)]
    fn detached(config: ClientConfig) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            outbox: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            stop: Arc::new(AtomicBool::new(false)),
            heartbeat: HEARTBEAT_INTERVAL,
            leave_wait: LEAVE_WAIT,
            tags: None,
        }
    }

    /// The session parameters in effect right now.
    fn config(&self) -> ClientConfig {
        self.config
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default()
    }

    /// Whether this client was asked to quit.
    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// What [`shutdown`] needs to end the app's transport thread politely: the
/// thread, and the link whose quit signal that thread watches.
struct Worker {
    handle: std::thread::JoinHandle<()>,
    link: Link,
}

/// The app's transport thread, kept so [`shutdown`] can ask it to say goodbye
/// and wait until it has.
static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// Records the thread [`shutdown`] stops and waits on, with the link it was
/// started on. Called from the subscription worker, which is the only place
/// that spawns it.
pub(super) fn register_worker(handle: std::thread::JoinHandle<()>, link: Link) {
    if let Ok(mut slot) = WORKER.lock() {
        *slot = Some(Worker { handle, link });
    }
}

/// Asks the transport to close its socket and waits for the thread to finish.
///
/// Unlike a dropped subscription, this asks the thread to say goodbye on the
/// wire: a process that just dies leaves the server waiting out the heartbeat
/// timeout, while a `Close` frame retires the session at once (`VAL-CLI-016`).
///
/// With no transport thread running there is nobody to say goodbye for, and
/// nothing is left behind for a later client to trip over.
///
/// Returns once the worker is gone or the budget elapses. The budget exists so
/// a wedged socket cannot hang the quit path; the thread's 100 ms read timeout
/// means the normal case returns almost immediately.
pub(super) fn shutdown() {
    // Taken out of the slot, so the join does not leak on a later restart of
    // the subscription (iced may rebuild it if the window reopens).
    let Some(worker) = WORKER.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    log("退出：请求关闭协同连接");
    stop_worker(worker, SHUTDOWN_BUDGET);
}

/// How long [`shutdown`] waits for the transport thread to say goodbye.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);

/// Ask `worker` to say goodbye and wait up to `budget` for it to finish.
/// Returns whether it did.
///
/// A thread that has not finished by then is left behind, not joined: it is
/// in a name lookup or a connect with a longer timeout of its own, and a join
/// would wait that out with the quit behind it. The process is exiting and
/// the thread goes with it.
fn stop_worker(worker: Worker, budget: Duration) -> bool {
    worker.link.stop.store(true, Ordering::Relaxed);
    let deadline = Instant::now() + budget;
    while !worker.handle.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !worker.handle.is_finished() {
        log("退出：协同线程没有在预算内结束，不再等它");
        return false;
    }
    let _ = worker.handle.join();
    true
}

/// Session config published for the app's client. The worker is spawned by
/// iced with no arguments, so the value travels through here.
fn app_config() -> &'static Arc<Mutex<ClientConfig>> {
    static SLOT: OnceLock<Arc<Mutex<ClientConfig>>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(ClientConfig::default())))
}

/// Publish the session parameters the app's transport thread should use.
pub(super) fn set_client_config(config: ClientConfig) {
    publish_config(app_config(), config);
}

/// Replace what a config slot holds; its client reads it on the next attempt.
fn publish_config(slot: &Mutex<ClientConfig>, config: ClientConfig) {
    if let Ok(mut slot) = slot.lock() {
        *slot = config;
    }
}

/// The UI's answers to the app client's question about the drawing's tags.
/// A slot for the same reason as [`app_config`]: the worker is started with
/// no arguments, and the UI has no handle on its thread.
fn app_tags() -> &'static Arc<TagsSlot> {
    static SLOT: OnceLock<Arc<TagsSlot>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// The UI's answer to question `ask` of the app client's about the tags the
/// session's drawing carries ([`CollabEvent::TagsWanted`]).
pub(super) fn answer_tags(ask: u64, said: TagsSaid) {
    leave_answer(app_tags(), ask, said);
}

/// Leave `said` in `slot` as the answer to question `ask`. Only the last
/// answer is kept: the client reads the one to the question it has out, and
/// an answer to an earlier question is of no use to it.
fn leave_answer(slot: &TagsSlot, ask: u64, said: TagsSaid) {
    if let Ok(mut slot) = slot.lock() {
        *slot = Some((ask, said));
    }
}

/// The number of the next question about the drawing's tags. One sequence
/// for the whole process, so that an answer left for an earlier client's
/// question — iced may start the subscription over — is never read as the
/// answer to a later one.
fn next_ask() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One outbound request the UI wants on the wire. The transport thread owns the
/// socket, so the UI never writes to it directly: it enqueues here and the read
/// loop drains the queue between reads (the 100 ms read timeout is the tick).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Outbound {
    /// `claim`: ask the server for exclusive ownership of `tag`.
    Claim { tag: String },
    /// `release`: give `tag` back.
    Release { tag: String },
    /// `op`: submit one P&ID semantic change. `base_revision` is captured when
    /// the request is queued, not when it is sent, so a request that waits
    /// behind a broadcast still states what the client believed.
    ///
    /// `session_id` is the session it was handed over for, when it is for
    /// one and no other. The UI gives its ops to the session it knows to be
    /// up, and gives them again to the next one if that one ended first
    /// (`State::pump`). When this thread is already on a later session than
    /// the UI has heard of, the op is not sent on it: the UI will hand it
    /// over again when it gets there, and the room would have it twice.
    /// `None` goes out on whatever session is up.
    Op {
        session_id: Option<String>,
        base_revision: i64,
        op_id: String,
        payload: serde_json::Value,
    },
    /// `attempt`: a write this session refused itself, for the server's
    /// record.
    Attempt { object_ref: String, text: String },
}

impl Outbound {
    /// How the request reads in a message ("claim P-1001").
    pub(super) fn describe(&self) -> String {
        match self {
            Outbound::Claim { tag } => format!("claim {tag}"),
            Outbound::Release { tag } => format!("release {tag}"),
            Outbound::Attempt { object_ref, .. } => format!("attempt object_ref={object_ref}"),
            Outbound::Op {
                op_id,
                base_revision,
                ..
            } => format!("op {op_id} base_revision={base_revision}"),
        }
    }

    /// Whether the request was handed over for a session other than
    /// `session_id`, the one that is up.
    fn is_for_another_session(&self, session_id: &str) -> bool {
        matches!(
            self,
            Outbound::Op { session_id: Some(meant), .. } if meant != session_id
        )
    }
}

/// One line on stderr, stamped with the wall clock in milliseconds.
///
/// What the client did on the wire is otherwise only visible from the far end
/// — and not at all while the far end is down, which is exactly when the
/// retry ladder's cadence has to be read off something (`VAL-CLI-014`).
fn log(line: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    eprintln!("[collab] t={at} {line}");
}

/// How much of the traffic the client writes to stderr.
///
/// The lifecycle — connecting, dropped, retrying, quitting — is always
/// written: it is what tells a person why nothing is arriving. The frames
/// are another matter, and `OCS_COLLAB_LOG` decides: `quiet` writes nothing
/// per frame; `frames` writes every frame as it was on the wire, next to
/// the one-line reading of it; anything else, or nothing, writes the
/// readings alone (what it has always done).
///
/// The wire text is for runs that keep evidence. A reading is this client's
/// account of a frame, and an account can be wrong where the frame is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LogLevel {
    Quiet,
    Events,
    Frames,
}

/// The level `OCS_COLLAB_LOG` names, read as leniently as an environment
/// variable set by hand deserves.
fn level_named(value: Option<&str>) -> LogLevel {
    match value.map(|value| value.trim().to_ascii_lowercase()).as_deref() {
        Some("quiet" | "off" | "0") => LogLevel::Quiet,
        Some("frames" | "frame" | "2") => LogLevel::Frames,
        _ => LogLevel::Events,
    }
}

/// The level this process logs at, read once.
fn log_level() -> LogLevel {
    static LEVEL: OnceLock<LogLevel> = OnceLock::new();
    *LEVEL.get_or_init(|| level_named(std::env::var("OCS_COLLAB_LOG").ok().as_deref()))
}

/// What is written for one frame at `level`: the frame itself, in the
/// direction it travelled, when frames are kept; then the reading of it,
/// when it has one and readings are written.
fn frame_lines(level: LogLevel, inbound: bool, wire: &str, reading: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    if level >= LogLevel::Frames {
        lines.push(format!("帧 {} {wire}", if inbound { "<-" } else { "->" }));
    }
    if level >= LogLevel::Events {
        lines.extend(reading.map(str::to_owned));
    }
    lines
}

/// Write what [`frame_lines`] gives for one frame at this process's level.
fn log_frame(inbound: bool, wire: &str, reading: Option<&str>) {
    for line in frame_lines(log_level(), inbound, wire, reading) {
        log(&line);
    }
}

/// Outbound queue of the app's client, shared between the UI thread
/// (producer) and the socket thread (consumer). A `Mutex<VecDeque>` is enough:
/// claim traffic is human-paced, never a hot path.
fn app_outbox() -> &'static Arc<Outbox> {
    static OUTBOX: OnceLock<Arc<Outbox>> = OnceLock::new();
    OUTBOX.get_or_init(|| Arc::new(Mutex::new(std::collections::VecDeque::new())))
}

fn push_outbound(outbox: &Outbox, request: Outbound) {
    if let Ok(mut queue) = outbox.lock() {
        queue.push_back(request);
    }
}

/// Take everything queued since the last drain.
fn drain_outbound(outbox: &Outbox) -> Vec<Outbound> {
    outbox
        .lock()
        .map(|mut queue| queue.drain(..).collect())
        .unwrap_or_default()
}

/// Forget queued requests. Done when a session starts, so a reconnect never
/// replays a claim the user made against the old session.
fn clear_outbound(outbox: &Outbox) {
    if let Ok(mut queue) = outbox.lock() {
        queue.clear();
    }
}

/// Put a `claim` for `tag` on `outbox`. An empty tag never gets there.
fn queue_claim(outbox: &Outbox, tag: &str) -> Result<(), String> {
    if tag.trim().is_empty() {
        return Err("位号为空".to_owned());
    }
    push_outbound(
        outbox,
        Outbound::Claim {
            tag: tag.trim().to_owned(),
        },
    );
    Ok(())
}

/// Put a `release` for `tag` on `outbox`. An empty tag never gets there.
fn queue_release(outbox: &Outbox, tag: &str) -> Result<(), String> {
    if tag.trim().is_empty() {
        return Err("位号为空".to_owned());
    }
    push_outbound(
        outbox,
        Outbound::Release {
            tag: tag.trim().to_owned(),
        },
    );
    Ok(())
}

/// Queue a `claim` for `tag` for the app's client. The `session_id` is checked
/// against the live session by the caller (`State::request_claim`), so the
/// queue itself only carries the intent.
pub(super) fn request_claim(session_id: &str, tag: &str) -> Result<(), String> {
    let _ = session_id;
    queue_claim(app_outbox(), tag)
}

/// Queue a `release` for `tag` for the app's client.
pub(super) fn request_release(session_id: &str, tag: &str) -> Result<(), String> {
    let _ = session_id;
    queue_release(app_outbox(), tag)
}

/// Put an `op` carrying one semantic change on `outbox`. `base_revision` is
/// the revision the change was made against (`VAL-CLI-010`). An op with no id
/// never gets there: the id is how its echo is told from a peer's change.
/// `session` is the one session it may go out on, if it is for one
/// ([`Outbound::Op`]).
fn queue_op(
    outbox: &Outbox,
    session: Option<&str>,
    base_revision: i64,
    op_id: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    if op_id.trim().is_empty() {
        return Err("操作 id 为空".to_owned());
    }
    push_outbound(
        outbox,
        Outbound::Op {
            session_id: session.map(str::to_owned),
            base_revision,
            op_id: op_id.to_owned(),
            payload,
        },
    );
    Ok(())
}

/// Queue an `op` for the app's client, to go out on session `session_id`
/// and on no later one.
pub(super) fn request_op(
    session_id: &str,
    base_revision: i64,
    op_id: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    queue_op(app_outbox(), Some(session_id), base_revision, op_id, payload)
}

/// Queue an `attempt` for the app's client: a write the session refused
/// itself, reported so the server can put it on record.
pub(super) fn request_attempt(session_id: &str, object_ref: &str, text: &str) {
    let _ = session_id;
    push_outbound(
        app_outbox(),
        Outbound::Attempt {
            object_ref: object_ref.to_owned(),
            text: text.to_owned(),
        },
    );
}

/// Turn one queued request into the wire message for `session_id`.
fn outbound_message(request: &Outbound, session_id: &str) -> ClientMessage {
    match request {
        Outbound::Attempt { object_ref, text } => ClientMessage::Attempt {
            session_id: session_id.to_owned(),
            object_ref: object_ref.clone(),
            text: text.clone(),
        },
        Outbound::Claim { tag } => ClientMessage::Claim {
            session_id: session_id.to_owned(),
            tag: tag.clone(),
        },
        Outbound::Release { tag } => ClientMessage::Release {
            session_id: session_id.to_owned(),
            tag: tag.clone(),
        },
        Outbound::Op {
            base_revision,
            op_id,
            payload,
            ..
        } => ClientMessage::Op {
            session_id: session_id.to_owned(),
            base_revision: *base_revision,
            op_id: op_id.clone(),
            payload: payload.clone(),
        },
    }
}

/// Why one session ended.
enum Ended {
    /// The server closed the conversation cleanly.
    Closed(String),
    /// The app is quitting: the socket was closed on purpose.
    Quit,
    /// The subscription is gone; nothing may reconnect.
    SubscriptionGone,
}

/// Whether a read failure is just the 100 ms timeout elapsing.
pub(super) fn is_read_timeout(error: &tungstenite::Error) -> bool {
    matches!(
        error,
        tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
    )
}

/// One-line description of a read failure, including the OS error kind the
/// validation contract looks for (`ConnectionReset`).
pub(super) fn describe_read_error(error: &tungstenite::Error) -> String {
    match error {
        tungstenite::Error::Io(io) => format!("read error: {:?} ({io})", io.kind()),
        tungstenite::Error::ConnectionClosed => "read error: ConnectionClosed".to_owned(),
        other => format!("read error: {other}"),
    }
}

/// How one server frame reads in the log: its wire name and what it is about.
fn describe_event(event: &CollabEvent) -> String {
    match event {
        CollabEvent::Connected {
            session_id,
            base_revision,
            peers,
            claims,
            ..
        } => format!(
            "welcome session={session_id} base_revision={base_revision} peers={} claims={}",
            peers.len(),
            claims.len()
        ),
        CollabEvent::AlignRefused { code, message } => {
            format!("welcome.align_refused code={code} reason={message}")
        }
        CollabEvent::PeerJoined(peer) => format!(
            "peer_joined session={} user={}",
            peer.session_id, peer.user_id
        ),
        CollabEvent::PeerLeft { session_id } => format!("peer_left session={session_id}"),
        CollabEvent::ClaimGranted { tag } => format!("claim_granted tag={tag}"),
        CollabEvent::ClaimDenied {
            tag,
            holder,
            expires_at,
        } => format!("claim_denied tag={tag} holder={holder} expires_at={expires_at}"),
        CollabEvent::ClaimReleased { tag, reason } => {
            format!("claim_released tag={tag} reason={reason}")
        }
        CollabEvent::ReadonlyDowngrade { reason, holder, .. } => format!(
            "readonly_downgrade holder={} reason={reason}",
            holder.as_deref().unwrap_or("(无)")
        ),
        CollabEvent::OpApplied {
            op_id,
            revision,
            changes,
        } => format!(
            "op_applied op_id={op_id} revision={revision} changes={}",
            changes.len()
        ),
        CollabEvent::ServerError { code, message } => format!("error code={code} {message}"),
        CollabEvent::OpRefused {
            op_id,
            code,
            message,
            rejected,
        } => format!("error code={code} op_id={op_id} rejected={rejected} {message}"),
        CollabEvent::Resuming { from } => format!("welcome replay_from={from}"),
        CollabEvent::ClaimsExpire(claims) => format!("welcome claims expire: {}", claims.len()),
        CollabEvent::OpsInConflict { op_ids } => format!("conflict op_ids={}", op_ids.join(",")),
        CollabEvent::Conflict(entry) => format!(
            "conflict conflict_id={} object_ref={} field={} value_a={} value_b={}",
            entry.conflict_id, entry.object_ref, entry.field, entry.value_a, entry.value_b
        ),
        // Not frames: the transport makes these up itself.
        CollabEvent::Connecting { .. }
        | CollabEvent::Disconnected { .. }
        | CollabEvent::TagsWanted { .. } => "(not a server frame)".to_owned(),
    }
}

/// Retry delay for the given zero-based attempt.
pub(super) fn backoff_delay(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
    BACKOFF_START
        .saturating_mul(factor)
        .min(BACKOFF_MAX)
}

/// Keep the GUI informed, and report whether the subscription still exists.
///
/// `TrySendError::Full` means the UI thread is behind. For a status line
/// (`Connecting`) dropping it is strictly better than blocking the socket
/// thread. For anything that changes what the session is — a peer's change,
/// the welcome, a claim, a downgrade — it is not: the UI's picture, and for
/// `op_applied` the drawing itself, would be short of something the server
/// counts as delivered. Those wait for room — for as long as there is
/// somebody to deliver to: a quit ends the wait (the UI thread is then
/// waiting on this one and will not read the channel again), and so does a
/// cancelled subscription.
/// `TrySendError::Disconnected` means iced cancelled the subscription, so the
/// thread must exit or it leaks.
fn emit(
    sender: &mut mpsc::Sender<CollabEvent>,
    link: &Link,
    event: CollabEvent,
) -> Result<(), ()> {
    let droppable = matches!(event, CollabEvent::Connecting { .. });
    let mut event = event;
    loop {
        match sender.try_send(event) {
            Ok(()) => return Ok(()),
            Err(error) if error.is_disconnected() => return Err(()),
            Err(_full) if droppable => return Ok(()),
            Err(full) => {
                if link.stop_requested() {
                    return Err(());
                }
                event = full.into_inner();
                std::thread::sleep(EMIT_RETRY);
            }
        }
    }
}

/// Sleep out a retry delay, aborting early when the subscription disappears or
/// the app starts quitting.
fn wait_before_retry(
    sender: &mpsc::Sender<CollabEvent>,
    delay: Duration,
    link: &Link,
) -> Result<(), ()> {
    let deadline = Instant::now() + delay;
    while Instant::now() < deadline {
        if sender.is_closed() || link.stop_requested() {
            return Err(());
        }
        std::thread::sleep(BACKOFF_SLICE.min(deadline.saturating_duration_since(Instant::now())));
    }
    Ok(())
}

/// What the UI said to the question a dial put to it ([`ask_for_tags`]).
struct Told {
    /// The number of the question.
    ask: u64,
    /// The tags to list in the `hello`.
    tags: Vec<String>,
    /// What the drawing file was saved aligned to, for the `hello` to
    /// claim when no session has been up yet (`REQ-COLL-015`).
    file: Option<crate::app::collab::FileClaim>,
    /// The UI has not said the tags: it has no drawing to speak for yet, or
    /// it did not answer in time. They are owed, and come under the same
    /// number.
    owed: bool,
    /// The copy holds itself apart ([`TagsSaid::Apart`]): the `hello`
    /// claims nothing, and the connection does not move `seen` on.
    apart: bool,
    /// The copy was taken off the room it was in ([`TagsSaid::Detached`]):
    /// where the last session left the drawing is forgotten, and the
    /// `hello` claims nothing.
    forget: bool,
}

/// Ask the UI which tags the session's drawing carries, and wait for what
/// it says: the tags, or that the `hello` need not wait for them. A client
/// with no UI to ask ([`Link::tags`]) asks nobody: `None`.
///
/// The wait is bounded ([`TagsLine::wait`]). A UI that has not answered by
/// then is taken to have said "not yet": the session comes up without the
/// tags rather than not at all. `Err` when there is nobody left to dial for
/// — the app is quitting, or the subscription is gone.
fn ask_for_tags(sender: &mut mpsc::Sender<CollabEvent>, link: &Link) -> Result<Option<Told>, ()> {
    let Some(line) = &link.tags else {
        return Ok(None);
    };
    let ask = next_ask();
    emit(sender, link, CollabEvent::TagsWanted { ask })?;
    let deadline = Instant::now() + line.wait;
    loop {
        match line.answer(ask) {
            Some(TagsSaid::Tags(tags, file)) => {
                return Ok(Some(Told {
                    ask,
                    tags,
                    file,
                    owed: false,
                    apart: false,
                    forget: false,
                }));
            }
            Some(TagsSaid::Detached(tags)) => {
                return Ok(Some(Told {
                    ask,
                    tags,
                    file: None,
                    owed: false,
                    apart: false,
                    forget: true,
                }));
            }
            Some(TagsSaid::Apart(_)) => {
                return Ok(Some(Told {
                    ask,
                    tags: Vec::new(),
                    file: None,
                    owed: false,
                    apart: true,
                    forget: false,
                }));
            }
            Some(TagsSaid::NotYet) => break,
            None => {}
        }
        if sender.is_closed() || link.stop_requested() {
            return Err(());
        }
        if Instant::now() >= deadline {
            log("界面还没有说出本图已有的位号：先不带位号加入，等它说了再重新加入");
            break;
        }
        std::thread::sleep(TAGS_POLL);
    }
    Ok(Some(Told {
        ask,
        tags: Vec::new(),
        file: None,
        owed: true,
        apart: false,
        forget: false,
    }))
}

/// The transport thread body: connect, hold the session, reconnect on failure.
///
/// Returns `Ok(())` once the subscription is cancelled (or the client was never
/// configured); errors are reported through the channel rather than returned,
/// because there is no one left to return them to.
///
/// Everything the client reads or drains comes through `link`: its session
/// parameters, its outbound queue, and its quit signal ([`shutdown`] raises the
/// one of the link it was registered with). Once that is set the session says
/// goodbye on the wire and the client does not reconnect.
///
/// Before each dial the UI is asked which tags the session's drawing carries
/// ([`ask_for_tags`]), for the `hello` to list them: asked anew every time,
/// since the drawing may be another, or have changed, since the last.
pub(super) fn run_client(
    sender: &mut mpsc::Sender<CollabEvent>,
    link: &Link,
) -> std::io::Result<()> {
    let mut attempt = 0u32;
    // Where the drawing was when the last session ended, to say in the next
    // `hello`.
    let mut seen: Option<Seen> = None;
    loop {
        if link.stop_requested() || sender.is_closed() {
            return Ok(());
        }
        let config = link.config();
        if config.server_url.trim().is_empty() {
            // The subscription is gated on a configured address, so this is
            // only reachable in the window between the worker starting and the
            // boot path publishing the CLI address. Wait for it instead of
            // exiting: returning here would leave the subscription alive but
            // permanently silent, with no way to ever reconnect.
            if sender.is_closed() {
                return Ok(());
            }
            std::thread::sleep(BACKOFF_SLICE);
            continue;
        }
        if emit(sender, link, CollabEvent::Connecting { attempt }).is_err() {
            return Ok(());
        }
        log(&format!(
            "连接 {}（第 {} 次尝试）",
            super::shown_address(&config.server_url),
            attempt.saturating_add(1)
        ));
        let Ok(told) = ask_for_tags(sender, link) else {
            return Ok(());
        };
        if told.as_ref().is_some_and(|told| told.forget) {
            seen = None;
        }
        // The answer may have settled the drawing number: the one the
        // drawing's sync record names, when none was given for this run.
        let config = link.config();
        let mut came_up = false;
        let session = run_session(
            sender,
            &config,
            link,
            &mut came_up,
            &mut seen,
            told.as_ref(),
        );
        let reason = match session {
            Ok(Ended::SubscriptionGone) => return Ok(()),
            Ok(Ended::Quit) => return Ok(()),
            Ok(Ended::Closed(reason)) => reason,
            Err(error) => error,
        };
        // The ladder counts failures in a row. A session that came up ends
        // the row: the next drop starts from the first rung again, instead
        // of every drop after the fifth waiting out the full five seconds.
        if came_up {
            attempt = 0;
        }
        let delay = backoff_delay(attempt);
        log(&format!("断开：{reason}；{} 毫秒后重连", delay.as_millis()));
        if emit(
            sender,
            link,
            CollabEvent::Disconnected {
                reason,
                will_retry: true,
            },
        )
        .is_err()
        {
            return Ok(());
        }
        if wait_before_retry(sender, delay, link).is_err() {
            return Ok(());
        }
        attempt = attempt.saturating_add(1);
    }
}

/// The last revision this client handed to the UI, and of which room: what
/// the next `hello` says (`since`), so that the server can send what was
/// missed in between (`VAL-CROSS-008`).
///
/// Kept by the thread that owns the socket rather than read from the UI's
/// state: every `op_applied` passes through here on its way to the UI, in
/// order, and the UI has applied each by the time it sees the next
/// `welcome` — whereas the UI may not have got round to the event that says
/// the socket dropped when this thread is already dialling again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    /// Which repository the room is of, as its `welcome` said. A server
    /// that names none (one from before `welcome.repo_id`) leaves this
    /// empty, and the next `hello` then gives its `since` alone, which is
    /// replayed from but not judged.
    repo_id: String,
    drawing_id: String,
    revision: i64,
}

impl Seen {
    /// What the UI's baseline is once it has taken `event`.
    fn take(
        seen: &mut Option<Seen>,
        config: &ClientConfig,
        replay_from: Option<i64>,
        event: &CollabEvent,
    ) {
        match event {
            CollabEvent::Connected {
                base_revision,
                repo_id,
                ..
            } => {
                // With a replay the session is taken where its drawing is —
                // the `since` the `hello` gave, which the replay starts from,
                // whether it came from the last session or from the file —
                // and each replayed op moves it on; without one, at the
                // room's revision.
                let revision = replay_from.unwrap_or(*base_revision);
                *seen = Some(Seen {
                    repo_id: repo_id.clone(),
                    drawing_id: config.drawing_id.clone(),
                    revision,
                });
            }
            CollabEvent::OpApplied { revision, .. } => {
                if let Some(seen) = seen.as_mut() {
                    seen.revision = seen.revision.max(*revision);
                }
            }
            _ => {}
        }
    }
}

/// One connection: handshake, `hello`, then the read loop until it ends.
/// `came_up` is set once the server has answered with `welcome`; `seen`
/// follows the revision handed to the UI, across sessions — those that take
/// the drawing as the room's.
///
/// `told` is what the UI said of the tags the session's drawing carries,
/// and the `hello` lists them (`existing_tags`). Tags it still owes are
/// looked for while the session is up. `hello` is the only frame that
/// carries them, so when they come the session is left — on purpose, with a
/// close frame the server is given the time to take ([`leave`]) — and the
/// retry ladder joins again, asking the UI anew. (At the server the claims
/// the session held go with it; the UI asks for them again once the next
/// session is up — `State::say_tags`.) A drawing that turns out to carry
/// none has nothing to tell, and the session stays.
fn run_session(
    sender: &mut mpsc::Sender<CollabEvent>,
    config: &ClientConfig,
    link: &Link,
    came_up: &mut bool,
    seen: &mut Option<Seen>,
    told: Option<&Told>,
) -> Result<Ended, String> {
    // A request queued against a session that has since ended must not be
    // replayed into this one: the tag may belong to somebody else by now.
    // (An op is another matter, and is not kept here: the UI holds its ops
    // until the room has answered each, and hands them over one at a time.)
    clear_outbound(&link.outbox);
    let mut ws = connect(&config.server_url)?;
    // What this copy claims to be (`REQ-COLL-015`): of the room the last
    // session was in — `seen`, fresher than the file — or, when no session
    // has been up yet, of what the drawing file was saved aligned to. The
    // server judges a claim that names its repository; a `since` of a room
    // that named none (a server from before `welcome.repo_id`) goes out
    // alone and is only replayed from. A copy that holds itself apart
    // ([`TagsSaid::Apart`]) claims nothing.
    let apart = told.is_some_and(|told| told.apart);
    let claim = seen
        .as_ref()
        .filter(|seen| !apart && seen.drawing_id == config.drawing_id)
        .map(|seen| (seen.repo_id.clone(), seen.revision))
        .or_else(|| {
            told.and_then(|told| told.file.as_ref())
                .filter(|file| file.drawing_id == config.drawing_id)
                .map(|file| (file.repo_id.clone(), file.revision))
        });
    let claimed = claim.as_ref().map(|(_, revision)| *revision);
    let hello = ClientMessage::Hello {
        user_id: config.user_id.clone(),
        drawing_id: config.drawing_id.clone(),
        mode: config.mode.clone(),
        client_version: config.client_version.clone(),
        workspace: String::new(),
        since: claimed,
        existing_tags: told.map(|told| told.tags.clone()).unwrap_or_default(),
        repo_id: claim
            .map(|(repo_id, _)| repo_id)
            .filter(|repo_id| !repo_id.is_empty()),
    };
    let wire = send_json(&mut ws, &hello)?;
    log_frame(false, &wire, None);

    let mut last_heartbeat = Instant::now();
    let mut session_id: Option<String> = None;
    // The question the UI still owes the tags to, with where it answers.
    let mut owed = match (told, &link.tags) {
        (Some(told), Some(line)) if told.owed => Some((told.ask, line)),
        _ => None,
    };
    // Whether this connection took the drawing as the room's. One whose
    // `welcome` refused it, or that came back to a room that moved on past
    // the claimed revision with no replay, or of a copy that holds itself
    // apart, is held apart by the UI: what it hears does not move the
    // drawing on, and the next `hello` claims again what the drawing is.
    let mut aligned = !apart;

    loop {
        if link.stop_requested() {
            // The app is quitting: say goodbye on the wire so the server
            // retires the session now instead of after the 90 s timeout.
            close(&mut ws);
            log("退出：已发送 close 帧");
            return Ok(Ended::Quit);
        }
        if sender.is_closed() {
            close(&mut ws);
            return Ok(Ended::SubscriptionGone);
        }
        match owed.and_then(|(ask, line)| line.answer(ask)) {
            Some(TagsSaid::Tags(tags, _)) => {
                owed = None;
                if !tags.is_empty() {
                    log(&format!(
                        "界面说出了本图已有的 {} 个位号：断开后重新加入，随 hello 上报",
                        tags.len()
                    ));
                    leave(&mut ws, link.leave_wait, || {
                        link.stop_requested() || sender.is_closed()
                    });
                    return Ok(Ended::Closed(
                        "要向服务器上报本图已有的位号，正在重新加入会话".to_owned(),
                    ));
                }
            }
            // The drawing came, and the copy holds itself apart: this
            // connection claimed what the drawing is, and moves `seen` on
            // with what it hears. It is left, and the next claims nothing.
            Some(TagsSaid::Apart(_)) => {
                log("界面说本图不对齐：断开后重新加入，不声称图纸对齐到哪里");
                leave(&mut ws, link.leave_wait, || {
                    link.stop_requested() || sender.is_closed()
                });
                return Ok(Ended::Closed(
                    "本图与房间对不上，正在不带对齐位置重新加入会话".to_owned(),
                ));
            }
            // The drawing came, and it was taken off the room it was in:
            // this connection claimed where the last session left it. That
            // is forgotten, the connection is left, and the next claims
            // nothing.
            Some(TagsSaid::Detached(_)) => {
                *seen = None;
                log("界面说本图已去掉同步记录：断开后重新加入，不声称图纸对齐到哪里");
                leave(&mut ws, link.leave_wait, || {
                    link.stop_requested() || sender.is_closed()
                });
                return Ok(Ended::Closed(
                    "本图已去掉同步记录，正在不带对齐位置重新加入会话".to_owned(),
                ));
            }
            Some(TagsSaid::NotYet) | None => {}
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                let said = read(text.as_str());
                let reading = match &said.frame {
                    Frame::Event(event) => Some(format!("收到 {}", describe_event(event))),
                    _ => None,
                };
                log_frame(true, text.as_str(), reading.as_deref());
                // Where a replay starts, when the frame is a `welcome` that
                // announces one.
                let replay_from = said.before.iter().find_map(|event| match event {
                    CollabEvent::Resuming { from } => Some(*from),
                    _ => None,
                });
                let refused = said
                    .before
                    .iter()
                    .any(|event| matches!(event, CollabEvent::AlignRefused { .. }));
                // What goes with the frame's own event, ahead of it and
                // behind it, in that order.
                let mut events = said.before;
                let mut ending = None;
                match said.frame {
                    Frame::Event(event) => events.push(event),
                    Frame::Close => ending = Some("server closed the session".to_owned()),
                    Frame::Rejoin(code) => {
                        // The server has let go of the session and will take
                        // no second `hello` on this socket: drop it and join
                        // again, saying where the drawing is.
                        log(&format!("服务器已不认这个会话（{code}），断开后重新加入"));
                        ending = Some(format!("session ended by the server ({code}); joining again"));
                    }
                    Frame::Unreadable(error) => {
                        log(&format!("无法解析服务器帧：{error}"));
                    }
                }
                events.extend(said.after);
                for event in events {
                    // `welcome` carries the session id every later frame
                    // references, so remember it before forwarding.
                    if let CollabEvent::Connected {
                        session_id: id,
                        base_revision,
                        ..
                    } = &event
                    {
                        session_id = Some(id.clone());
                        *came_up = true;
                        let moved_on = replay_from.is_none()
                            && claimed.is_some_and(|since| *base_revision > since);
                        aligned = !apart && !refused && !moved_on;
                    }
                    if aligned {
                        Seen::take(seen, config, replay_from, &event);
                    }
                    if emit(sender, link, event).is_err() {
                        close(&mut ws);
                        return Ok(Ended::SubscriptionGone);
                    }
                }
                if let Some(reason) = ending {
                    close(&mut ws);
                    return Ok(Ended::Closed(reason));
                }
            }
            Ok(Message::Binary(_)) => {}
            Ok(Message::Ping(payload)) => {
                // tungstenite queues the pong; flushing pushes it out now
                // rather than on the next write.
                let _ = ws.send(Message::Pong(payload));
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) => {
                // Constraint 3: `Close` arrives once. Anything after it is
                // `ConnectionAborted`, so leave the loop here.
                close(&mut ws);
                return Ok(Ended::Closed("close frame received".to_owned()));
            }
            Ok(Message::Frame(_)) => {}
            Err(error) if is_read_timeout(&error) => {
                // Constraint 2: the 100 ms timeout is the loop's clock.
            }
            Err(error) => {
                // Constraint 4: everything else — including
                // `Io(ConnectionReset)` from a killed server — means the
                // connection is gone and the retry ladder takes over.
                let reason = describe_read_error(&error);
                close(&mut ws);
                return Ok(Ended::Closed(reason));
            }
        }

        // Drain the outbound queue: claim / release requests the UI queued
        // since the last tick. Sending here (rather than from the UI thread)
        // keeps every write to the socket on the thread that owns it.
        if let Some(session_id) = session_id.clone() {
            for request in drain_outbound(&link.outbox) {
                if request.is_for_another_session(&session_id) {
                    // Handed over by a UI that had not heard yet that the
                    // session it names is over. It hands it over again once
                    // it has heard of this one.
                    log(&format!(
                        "不发送 {}：它是交给上一个会话的，界面会在知道本会话之后重新交来",
                        request.describe()
                    ));
                    continue;
                }
                let message = outbound_message(&request, &session_id);
                match send_json(&mut ws, &message) {
                    Ok(wire) => {
                        let reading = format!("发送 {}", request.describe());
                        log_frame(false, &wire, Some(&reading));
                    }
                    Err(error) => {
                        // A failed write means the socket is gone; the retry
                        // ladder takes over and the queued request is dropped
                        // with it.
                        let reason = format!("{} 发送失败：{error}", request.describe());
                        close(&mut ws);
                        return Ok(Ended::Closed(reason));
                    }
                }
            }
        }

        if last_heartbeat.elapsed() >= link.heartbeat {
            let Some(session_id) = session_id.clone() else {
                continue;
            };
            let beat = ClientMessage::Heartbeat {
                session_id: session_id.clone(),
            };
            match send_json(&mut ws, &beat) {
                Ok(wire) => log_frame(false, &wire, None),
                Err(error) => {
                    let reason = format!("heartbeat failed: {error}");
                    close(&mut ws);
                    return Ok(Ended::Closed(reason));
                }
            }
            last_heartbeat = Instant::now();
        }
    }
}

/// Open the socket with a bounded connect and complete the WS handshake.
fn connect(url: &str) -> Result<WebSocket<TcpStream>, String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("服务器地址无效：{error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "服务器地址缺少主机名".to_owned())?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    let mut last_error = None;
    let mut stream = None;
    for addr in (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("无法解析 {host}:{port}：{error}"))?
    {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(socket) => {
                stream = Some(socket);
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let stream = stream.ok_or_else(|| {
        format!(
            "连接 {host}:{port} 失败：{}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "没有可用地址".to_owned())
        )
    })?;
    // The read timeout is what makes the loop stoppable at all (constraint 1).
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|error| format!("设置读超时失败：{error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("设置写超时失败：{error}"))?;
    let _ = stream.set_nodelay(true);

    // `client`, not `connect`: the socket already has a deadline and a
    // configured timeout, so the handshake inherits both.
    let (ws, _response) =
        tungstenite::client(url, stream).map_err(|error| format!("WebSocket 握手失败：{error}"))?;
    Ok(ws)
}

/// Serialise one client message and flush it. Returns the text that went
/// out, for the frame log.
fn send_json<S>(ws: &mut WebSocket<S>, message: &ClientMessage) -> Result<String, String>
where
    S: std::io::Read + Write,
{
    let text = serde_json::to_string(message).map_err(|error| format!("序列化失败：{error}"))?;
    ws.send(Message::text(text.clone()))
        .map_err(|error| format!("发送失败：{error}"))?;
    Ok(text)
}

/// Say goodbye politely. `close` only queues the frame; without the flush the
/// peer waits out the TCP timeout instead of seeing the close.
fn close<S>(ws: &mut WebSocket<S>)
where
    S: std::io::Read + Write,
{
    let _ = ws.close(None);
    let _ = ws.flush();
}

/// Leave a session on purpose, to join again: say goodbye, and stay on the
/// line until the server has taken it — it answers the close frame, or
/// hangs up — or `wait` is up, or `gone` says nobody waits for the next
/// session any more.
///
/// Hanging up the moment the close frame is out loses the goodbye whenever
/// the server has something on its way here — the answer to a claim made
/// just before: what it writes meets a closed socket, the connection is
/// reset, and the close frame behind it is never read. The session then
/// lives on at the server, with its claims, until its heartbeat times out,
/// and the session that replaces it is refused the claims it comes to take
/// back.
///
/// What arrives meanwhile is logged and not handed on. The session is over
/// as far as the UI goes: an answer to it is to nobody, and what the room
/// did in the meantime the next `hello` asks for (`since`).
fn leave<S>(ws: &mut WebSocket<S>, wait: Duration, gone: impl Fn() -> bool)
where
    S: std::io::Read + Write,
{
    close(ws);
    let deadline = Instant::now() + wait;
    while !gone() {
        if Instant::now() >= deadline {
            log("离开：服务器没有在时限内收下 close 帧，不再等");
            return;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                let reading = match read(text.as_str()).frame {
                    Frame::Event(event) => {
                        format!("离开途中收到 {}：不交给界面", describe_event(&event))
                    }
                    _ => "离开途中收到一帧：不交给界面".to_owned(),
                };
                log_frame(true, text.as_str(), Some(&reading));
            }
            // Constraint 3: the answer to the goodbye comes once.
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(error) if is_read_timeout(&error) => {}
            // The server hung up — it does once it has taken the goodbye —
            // or the connection is gone some other way.
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// The parameters every test client joins with, pointed at `server_url`.
    fn config_for(server_url: String) -> ClientConfig {
        ClientConfig {
            server_url,
            user_id: "u1".to_owned(),
            drawing_id: "PID-1101".to_owned(),
            mode: "协同编辑".to_owned(),
            client_version: "test".to_owned(),
        }
    }

    /// A one-connection WebSocket server that answers `hello` with `welcome`
    /// and then goes quiet, so the client's read loop idles on its timeout.
    fn quiet_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("ws://{addr}/ws");
        let handle = std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut ws = match tungstenite::accept(stream) {
                Ok(ws) => ws,
                Err(_) => return,
            };
            let _ = ws.read();
            let welcome = r#"{"type":"welcome","session_id":"s-test","base_revision":0,"peers":[],"claims":[]}"#;
            if ws.send(Message::text(welcome)).is_err() {
                return;
            }
            // Hold the socket open and ignore whatever arrives until the peer
            // hangs up, so the client keeps waiting on its read timeout.
            while ws.read().is_ok() {}
        });
        (url, handle)
    }

    /// A one-connection server that records every client frame it reads, so a
    /// test can assert what the client actually put on the wire.
    fn recording_server() -> (
        String,
        std::sync::Arc<Mutex<Vec<String>>>,
        std::sync::Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("ws://{addr}/ws");
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let closed = std::sync::Arc::new(AtomicBool::new(false));
        let seen_writer = std::sync::Arc::clone(&seen);
        let closed_writer = std::sync::Arc::clone(&closed);
        let handle = std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut ws = match tungstenite::accept(stream) {
                Ok(ws) => ws,
                Err(_) => return,
            };
            let _ = ws.read();
            let welcome = r#"{"type":"welcome","session_id":"s-test","base_revision":0,"peers":[],"claims":[]}"#;
            if ws.send(Message::text(welcome)).is_err() {
                return;
            }
            loop {
                match ws.read() {
                    Ok(Message::Text(text)) => {
                        if let Ok(mut log) = seen_writer.lock() {
                            log.push(text.to_string());
                        }
                    }
                    Ok(Message::Close(_)) => {
                        closed_writer.store(true, Ordering::Relaxed);
                        let _ = ws.flush();
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        });
        (url, seen, closed, handle)
    }

    /// A server that takes `sessions` connections one after the other, answers
    /// each `hello` with `welcome` and then drops the socket; after the last
    /// one nobody listens any more.
    fn dropping_server(sessions: usize) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("ws://{addr}/ws");
        let handle = std::thread::spawn(move || {
            for _ in 0..sessions {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let Ok(mut ws) = tungstenite::accept(stream) else {
                    continue;
                };
                let _ = ws.read();
                let welcome = r#"{"type":"welcome","session_id":"s-test","base_revision":0,"peers":[],"claims":[]}"#;
                let _ = ws.send(Message::text(welcome));
                // Dropped here, with no close frame: a server that died.
            }
        });
        (url, handle)
    }

    /// The retry ladder counts failures in a row, and a session that came up
    /// ends the row. It used to count every drop of the run: after a handful
    /// of them each new one waited out the full five seconds, however long
    /// the sessions in between had lasted.
    #[test]
    fn a_session_that_came_up_starts_the_retry_ladder_again() {
        let (url, server) = dropping_server(2);
        let link = Link::detached(config_for(url));
        let (mut receiver, worker) = start_client(&link);

        // Which retry the first try after each session that was up is. The
        // very first connection is try 0; a retry after one failure is 1.
        let mut after_a_session = Vec::new();
        let mut was_up = false;
        let deadline = Instant::now() + Duration::from_secs(20);
        while after_a_session.len() < 2 && Instant::now() < deadline {
            match receiver.try_recv() {
                Ok(CollabEvent::Connected { .. }) => was_up = true,
                Ok(CollabEvent::Connecting { attempt }) if was_up => {
                    after_a_session.push(attempt);
                    was_up = false;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Closed) => break,
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert_eq!(
            after_a_session,
            [1, 1],
            "each drop of a session that was up is followed by the first retry \
             again (it read [1, 2]: the second drop went on up the ladder)"
        );
    }

    /// How long a test waits for the client's session to come up. A loopback
    /// handshake takes a few milliseconds and the wait ends the moment
    /// `Connected` arrives, so the bound is only ever spent on a failure — and
    /// it is wide enough that a busy machine cannot turn a slow start into one.
    const SESSION_WAIT: Duration = Duration::from_secs(10);

    /// Start a client on `link`, on a thread of its own.
    fn start_client(link: &Link) -> (mpsc::Receiver<CollabEvent>, std::thread::JoinHandle<()>) {
        let (mut sender, receiver) = mpsc::channel::<CollabEvent>(64);
        let link = link.clone();
        let worker = std::thread::spawn(move || {
            let _ = run_client(&mut sender, &link);
        });
        (receiver, worker)
    }

    /// Wait until the client reports its session. `false` once
    /// [`SESSION_WAIT`] has run out, or as soon as the client thread is gone
    /// (its end of the channel dropped) without ever having connected.
    fn session_came_up(receiver: &mut mpsc::Receiver<CollabEvent>) -> bool {
        let started = Instant::now();
        loop {
            match receiver.try_recv() {
                Ok(CollabEvent::Connected { .. }) => {
                    eprintln!("[transport-test] session after {:?}", started.elapsed());
                    return true;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Closed) => return false,
                Err(mpsc::TryRecvError::Empty) => {
                    if started.elapsed() >= SESSION_WAIT {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// VAL-CLI-002: the read loop really emits `heartbeat` frames on its
    /// cadence, and every one carries the session id from `welcome`.
    #[test]
    fn the_session_emits_heartbeats_carrying_its_session_id() {
        let (url, seen, _closed, server) = recording_server();
        // A cadence short enough to watch several beats go by without sitting
        // through a real minute; the app's client always runs on
        // `HEARTBEAT_INTERVAL`.
        let mut link = Link::detached(config_for(url));
        link.heartbeat = Duration::from_millis(120);

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        // Four cadence periods is enough for at least three beats.
        let deadline = Instant::now() + Duration::from_secs(2);
        let beats = loop {
            let beats: Vec<String> = seen
                .lock()
                .expect("frame log")
                .iter()
                .filter(|frame| frame.contains("\"type\":\"heartbeat\""))
                .cloned()
                .collect();
            if beats.len() >= 3 || Instant::now() >= deadline {
                break beats;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            beats.len() >= 3,
            "expected at least 3 heartbeats, saw {}: {beats:?}",
            beats.len()
        );
        for beat in &beats {
            assert!(
                beat.contains("s-test"),
                "a heartbeat must carry the session id from welcome: {beat}"
            );
        }

        drop(receiver);
        let mut exited = false;
        for _ in 0..60 {
            if worker.is_finished() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(exited, "the transport thread leaked after cancellation");
        let _ = worker.join();
        let _ = server.join();
    }

    /// VAL-CLI-016: quitting closes the socket instead of vanishing, and the
    /// thread is joined rather than leaked.
    #[test]
    fn quitting_sends_a_close_frame_and_joins_the_thread() {
        let (url, _seen, closed, server) = recording_server();
        let link = Link::detached(config_for(url));

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );
        // Registered once the session is up: the registration slot is the one
        // the whole process shares, so a test elsewhere that quits an app takes
        // whatever is in it. From here on that would only say this client's
        // goodbye early, which is the very thing asserted below.
        register_worker(worker, link);

        let started = Instant::now();
        shutdown();
        let elapsed = started.elapsed();
        assert!(
            elapsed <= Duration::from_secs(3),
            "shutdown took {elapsed:?}, over its budget"
        );

        // The server saw the close frame, not a dead socket.
        let mut saw_close = false;
        for _ in 0..40 {
            if closed.load(Ordering::Relaxed) {
                saw_close = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_close, "the client never sent a close frame on quit");
        let _ = server.join();
        drop(receiver);
    }

    /// A quit earlier in the process must not stop a client that starts later.
    ///
    /// Tests elsewhere in this binary walk the quit path (`EXIT` at the end of
    /// a batch run lands in `exit_app()`), and `exit_app()` says goodbye to
    /// the collaboration server on its way out. With no transport thread
    /// running there is nobody to say goodbye for, so nothing may be left
    /// behind that a later client would read as "the app is quitting".
    #[test]
    fn a_quit_with_no_transport_thread_leaves_a_later_client_alone() {
        let mut app = crate::app::OpenCADStudio::new_for_test();
        let _ = app.exit_app();

        let (url, server) = quiet_server();
        let link = Link::detached(config_for(url));
        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
    }

    /// What an app publishes for its own client must not reach anybody
    /// else's.
    ///
    /// Every app that applies a config republishes its collaboration settings
    /// to the app client's slot, and with collaboration unconfigured that is
    /// an empty address — which a client reads as "wait until one is
    /// configured". A client on a link of its own keeps the address it was
    /// started with, however many apps come and go meanwhile.
    #[test]
    fn what_an_app_publishes_does_not_reach_a_client_on_its_own_link() {
        let (url, server) = quiet_server();
        let link = Link::detached(config_for(url));

        // What `apply_config` does in every app this binary builds.
        crate::app::OpenCADStudio::new_for_test()
            .collab
            .sync_client_config();
        set_client_config(ClientConfig::default());

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
    }

    #[test]
    fn a_read_timeout_is_not_an_error_branch() {
        let would_block =
            tungstenite::Error::Io(std::io::Error::from(ErrorKind::WouldBlock));
        let timed_out = tungstenite::Error::Io(std::io::Error::from(ErrorKind::TimedOut));
        assert!(is_read_timeout(&would_block));
        assert!(is_read_timeout(&timed_out));

        let reset = tungstenite::Error::Io(std::io::Error::from(ErrorKind::ConnectionReset));
        assert!(!is_read_timeout(&reset));
    }

    #[test]
    fn a_connection_reset_is_reported_as_a_read_error() {
        let reset = tungstenite::Error::Io(std::io::Error::from(ErrorKind::ConnectionReset));
        let described = describe_read_error(&reset);
        assert!(described.starts_with("read error:"));
        assert!(described.contains("ConnectionReset"), "{described}");
    }

    #[test]
    fn a_clean_close_is_not_confused_with_a_reset() {
        let described = describe_read_error(&tungstenite::Error::ConnectionClosed);
        assert!(described.contains("ConnectionClosed"));
        assert!(!described.contains("ConnectionReset"));
    }

    #[test]
    fn the_retry_ladder_doubles_then_holds_at_the_ceiling() {
        assert_eq!(backoff_delay(0), Duration::from_millis(200));
        assert_eq!(backoff_delay(1), Duration::from_millis(400));
        assert_eq!(backoff_delay(2), Duration::from_millis(800));
        assert_eq!(backoff_delay(3), Duration::from_millis(1600));
        assert_eq!(backoff_delay(4), Duration::from_millis(3200));
        assert_eq!(backoff_delay(5), BACKOFF_MAX);
        assert_eq!(backoff_delay(64), BACKOFF_MAX);
    }

    #[test]
    fn the_heartbeat_cadence_lands_inside_the_contract_window() {
        // VAL-CLI-002: 60 s must carry 3–5 beats, 13–17 s apart. The cadence
        // itself is the only input, so assert the arithmetic the contract
        // measures rather than waiting a minute for it.
        assert_eq!(HEARTBEAT_INTERVAL, Duration::from_secs(15));
        let window = Duration::from_secs(60);
        let beats = window.as_secs() / HEARTBEAT_INTERVAL.as_secs();
        assert!(
            (3..=5).contains(&beats),
            "60 s at 15 s per beat yields {beats} beats, outside 3..=5"
        );
        let gap = HEARTBEAT_INTERVAL.as_secs();
        assert!((13..=17).contains(&gap), "beat gap {gap}s outside 13..=17");
        // The server's timeout must be several beats wide, or one dropped beat
        // would retire a live session.
        assert!(
            HEARTBEAT_INTERVAL.as_secs() * 3 <= 90,
            "90 s timeout must tolerate several missed beats"
        );
        // And that cadence is the one the app's client is started on.
        assert_eq!(Link::app().heartbeat, HEARTBEAT_INTERVAL);
    }

    #[test]
    fn a_cancelled_subscription_stops_the_thread_instead_of_leaking() {
        // `try_send` on a channel whose receiver is gone is the signal the read
        // thread exits on; assert the distinction the loop relies on.
        let (mut sender, receiver) = mpsc::channel::<CollabEvent>(4);
        drop(receiver);
        let error = sender
            .try_send(CollabEvent::Connecting { attempt: 0 })
            .expect_err("a dropped receiver must refuse the send");
        assert!(error.is_disconnected());
    }

    #[test]
    fn a_full_channel_drops_a_status_event_rather_than_blocking() {
        // futures' bounded channel holds `size` messages plus the slot the
        // sender owns, so fill it by sending until it refuses rather than
        // assuming an exact capacity.
        let link = Link::detached(ClientConfig::default());
        let (mut sender, _receiver) = mpsc::channel::<CollabEvent>(1);
        let mut attempt = 0u32;
        let full = loop {
            match sender.try_send(CollabEvent::Connecting { attempt }) {
                Ok(()) => attempt += 1,
                Err(error) => break error,
            }
            assert!(attempt < 16, "the channel never filled");
        };
        assert!(!full.is_disconnected(), "full is not disconnected");
        // And the loop's `emit` treats that as "keep going".
        assert!(emit(&mut sender, &link, CollabEvent::Connecting { attempt }).is_ok());
    }

    /// The three levels, and what each writes for a frame: nothing; the
    /// one-line reading; the frame as it was on the wire and then the
    /// reading. A frame with no reading — a heartbeat, a `hello` — is
    /// written only where frames are.
    #[test]
    fn a_frame_is_written_by_the_level_the_log_is_at() {
        assert_eq!(level_named(None), LogLevel::Events, "what it has always done");
        assert_eq!(level_named(Some("")), LogLevel::Events);
        assert_eq!(level_named(Some("events")), LogLevel::Events);
        assert_eq!(level_named(Some(" Quiet ")), LogLevel::Quiet);
        assert_eq!(level_named(Some("off")), LogLevel::Quiet);
        assert_eq!(level_named(Some("FRAMES")), LogLevel::Frames);

        let wire = r#"{"type":"op_applied","op_id":"op-1","revision":2,"changes":[{"value":"2.5MPa"}]}"#;
        let reading = "收到 op_applied op_id=op-1 revision=2 changes=1";
        assert!(frame_lines(LogLevel::Quiet, true, wire, Some(reading)).is_empty());
        assert_eq!(frame_lines(LogLevel::Events, true, wire, Some(reading)), [reading]);
        assert_eq!(
            frame_lines(LogLevel::Frames, true, wire, Some(reading)),
            [format!("帧 <- {wire}"), reading.to_owned()],
            "the frame goes in as it was, not as it was read"
        );

        let beat = r#"{"type":"heartbeat","session_id":"s-1"}"#;
        assert!(frame_lines(LogLevel::Events, false, beat, None).is_empty());
        assert_eq!(frame_lines(LogLevel::Frames, false, beat, None), [format!("帧 -> {beat}")]);
    }

    /// What is published to a link's config slot is what its client reads on
    /// its next attempt.
    #[test]
    fn the_published_config_round_trips_through_the_slot() {
        let link = Link::detached(ClientConfig::default());
        assert!(link.config().server_url.is_empty());
        publish_config(
            &link.config,
            ClientConfig {
                server_url: "ws://127.0.0.1:3240/ws".to_owned(),
                user_id: "u1".to_owned(),
                drawing_id: "PID-1101".to_owned(),
                mode: "协同编辑".to_owned(),
                client_version: "test".to_owned(),
            },
        );
        let config = link.config();
        assert_eq!(config.server_url, "ws://127.0.0.1:3240/ws");
        assert_eq!(config.user_id, "u1");
        assert_eq!(config.drawing_id, "PID-1101");
    }

    /// End-to-end: a real socket, a real `welcome`, then the subscription is
    /// cancelled and the thread must notice within its 100 ms read timeout.
    /// This is the "no leak, no hang" contract of the transport.
    #[test]
    fn cancelling_the_subscription_ends_the_thread_promptly() {
        let (url, server) = quiet_server();
        let link = Link::detached(config_for(url));

        // Wait for the session to come up, so the read loop is definitely
        // spinning on its timeout when the receiver goes away.
        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        drop(receiver);
        let started = Instant::now();
        // The thread must observe the closed channel through its 100 ms read
        // timeout and exit; 200 ms is the contract, 2 s is the harness limit.
        let mut joined = false;
        for _ in 0..40 {
            if worker.is_finished() {
                joined = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let elapsed = started.elapsed();
        assert!(joined, "the transport thread leaked after cancellation");
        assert!(
            elapsed <= Duration::from_millis(200),
            "cancellation took {elapsed:?}, over the 200 ms budget"
        );
        let _ = server.join();
    }

    /// VAL-CLI-004 / 007: the queued `claim` and `release` reach the wire as
    /// the protocol's frames, carrying the session id from `welcome`.
    #[test]
    fn queued_claims_and_releases_reach_the_wire_with_the_session_id() {
        let (url, seen, _closed, server) = recording_server();
        let link = Link::detached(config_for(url));

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        queue_claim(&link.outbox, "P-1001").expect("claim queues");
        queue_release(&link.outbox, "P-1001").expect("release queues");

        let deadline = Instant::now() + Duration::from_secs(2);
        let frames = loop {
            let frames: Vec<String> = seen.lock().expect("frame log").clone();
            let claim = frames
                .iter()
                .any(|frame| frame.contains("\"type\":\"claim\""));
            let release = frames
                .iter()
                .any(|frame| frame.contains("\"type\":\"release\""));
            if (claim && release) || Instant::now() >= deadline {
                break frames;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let claim = frames
            .iter()
            .find(|frame| frame.contains("\"type\":\"claim\""))
            .unwrap_or_else(|| panic!("no claim frame on the wire: {frames:?}"));
        assert!(claim.contains("P-1001"), "{claim}");
        assert!(claim.contains("s-test"), "{claim}");
        let release = frames
            .iter()
            .find(|frame| frame.contains("\"type\":\"release\""))
            .unwrap_or_else(|| panic!("no release frame on the wire: {frames:?}"));
        assert!(release.contains("P-1001"), "{release}");
        assert!(release.contains("s-test"), "{release}");

        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
    }

    /// A session that ends before its queued request is sent must not carry the
    /// request into the next session: the tag may belong to somebody else by
    /// then.
    #[test]
    fn a_new_session_starts_with_an_empty_outbox() {
        let outbox = Outbox::default();
        queue_claim(&outbox, "P-1001").expect("claim queues");
        clear_outbound(&outbox);
        assert!(drain_outbound(&outbox).is_empty());
    }

    /// A request queued before the session exists never reaches the wire: the
    /// session clears the queue as it starts.
    #[test]
    fn a_request_queued_before_the_session_is_not_replayed_into_it() {
        let (url, seen, _closed, server) = recording_server();
        let link = Link::detached(config_for(url));
        queue_claim(&link.outbox, "P-0001").expect("claim queues");

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );
        // One queued after the session is up does go out; once it is on the
        // wire, anything the session was going to send from before is too.
        queue_claim(&link.outbox, "P-1001").expect("claim queues");
        let deadline = Instant::now() + Duration::from_secs(2);
        let frames = loop {
            let frames: Vec<String> = seen.lock().expect("frame log").clone();
            if frames.iter().any(|frame| frame.contains("P-1001")) || Instant::now() >= deadline {
                break frames;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            frames.iter().any(|frame| frame.contains("P-1001")),
            "the claim made in the session never went out: {frames:?}"
        );
        assert!(
            !frames.iter().any(|frame| frame.contains("P-0001")),
            "a claim from before the session was replayed into it: {frames:?}"
        );

        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
    }

    #[test]
    fn an_empty_tag_is_refused_before_it_reaches_the_queue() {
        let outbox = Outbox::default();
        assert!(queue_claim(&outbox, "   ").is_err());
        assert!(queue_release(&outbox, "").is_err());
        assert!(drain_outbound(&outbox).is_empty());
        // The app's own entry points refuse it the same way.
        assert!(request_claim("s-1", "   ").is_err());
        assert!(request_release("s-1", "").is_err());
    }

    /// VAL-CLI-009 / 010: a queued `op` reaches the wire as the protocol's
    /// frame — the session id from `welcome`, the baseline it was queued
    /// against, its id, and the payload untouched.
    #[test]
    fn a_queued_op_reaches_the_wire_with_the_baseline_it_was_queued_against() {
        let (url, seen, _closed, server) = recording_server();
        let link = Link::detached(config_for(url));

        let (mut receiver, worker) = start_client(&link);
        assert!(
            session_came_up(&mut receiver),
            "the client never reported a session"
        );

        let payload = serde_json::json!({
            "kind": "set_tag",
            "object_ref": "P-1001",
            "field": "tag",
            "value": "P-1009",
        });
        queue_op(&link.outbox, None, 7, "op-test-1", payload.clone()).expect("op queues");

        let deadline = Instant::now() + Duration::from_secs(2);
        let frame = loop {
            let found = seen
                .lock()
                .expect("frame log")
                .iter()
                .find(|frame| frame.contains("\"type\":\"op\""))
                .cloned();
            if found.is_some() || Instant::now() >= deadline {
                break found;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let frame = frame.expect("no op frame on the wire");
        let sent: serde_json::Value = serde_json::from_str(&frame).expect("the frame is JSON");
        assert_eq!(sent["session_id"], "s-test", "{frame}");
        assert_eq!(sent["base_revision"], 7, "{frame}");
        assert_eq!(sent["op_id"], "op-test-1", "{frame}");
        assert_eq!(sent["payload"], payload, "{frame}");

        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
    }

    /// An op with no id could not be told from a peer's when it comes back, so
    /// it never gets as far as the queue. (Ids are the UI's to hand out, one
    /// run's apart from every other's: `collab_op_ids`.)
    #[test]
    fn an_op_needs_an_id() {
        let outbox = Outbox::default();
        assert!(queue_op(&outbox, None, 0, "  ", serde_json::json!({})).is_err());
        assert!(drain_outbound(&outbox).is_empty());
        assert!(request_op("s-1", 0, "", serde_json::json!({})).is_err());
    }

    /// A status line may be dropped when the UI is behind. A peer's change
    /// may not: the baseline moves with it, so dropping it leaves the drawing
    /// short a change it claims to have. It waits for room instead.
    #[test]
    fn a_peers_change_waits_for_room_in_the_channel_instead_of_being_dropped() {
        let link = Link::detached(ClientConfig::default());
        let (mut sender, mut receiver) = mpsc::channel::<CollabEvent>(1);
        while sender
            .try_send(CollabEvent::Connecting { attempt: 0 })
            .is_ok()
        {}
        // Full: a status event is given up without blocking.
        assert!(emit(&mut sender, &link, CollabEvent::Connecting { attempt: 1 }).is_ok());

        let change = CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: Vec::new(),
        };
        let waiting = std::thread::spawn(move || emit(&mut sender, &link, change));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!waiting.is_finished(), "it must wait, not drop");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut arrived = false;
        while Instant::now() < deadline && !arrived {
            match receiver.try_next() {
                Ok(Some(CollabEvent::OpApplied { op_id, .. })) => arrived = op_id == "op-peer",
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(arrived, "the change never reached the UI");
        assert_eq!(waiting.join().expect("emit thread"), Ok(()));
    }

    /// With nobody left to listen the wait ends: the thread must not hang on
    /// a subscription that is gone.
    #[test]
    fn a_waiting_change_gives_up_when_the_subscription_is_gone() {
        let link = Link::detached(ClientConfig::default());
        let (mut sender, receiver) = mpsc::channel::<CollabEvent>(1);
        while sender
            .try_send(CollabEvent::Connecting { attempt: 0 })
            .is_ok()
        {}
        let change = CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: Vec::new(),
        };
        let waiting = std::thread::spawn(move || emit(&mut sender, &link, change));
        std::thread::sleep(Duration::from_millis(30));
        drop(receiver);
        assert_eq!(waiting.join().expect("emit thread"), Err(()));
    }

    /// The quit waits for the transport thread only as long as its budget. A
    /// thread that is not done by then — here one that never looks at the
    /// quit signal — is left behind; joining it would hold the quit for as
    /// long as whatever it is stuck in takes.
    #[test]
    fn a_wedged_worker_does_not_hold_the_quit_past_its_budget() {
        let link = Link::detached(ClientConfig::default());
        let (release, wedged) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            // Stuck until the test lets go, whatever the quit signal says.
            let _ = wedged.recv();
        });
        let worker = Worker {
            handle,
            link: link.clone(),
        };
        // Returning at all is the assertion: a join here would never come back.
        assert!(!stop_worker(worker, Duration::from_millis(50)));
        assert!(link.stop_requested(), "it was asked to stop");
        drop(release);
    }

    /// One that does stop is waited for and joined.
    #[test]
    fn a_worker_that_stops_is_joined_within_the_budget() {
        let link = Link::detached(ClientConfig::default());
        let handle = {
            let link = link.clone();
            std::thread::spawn(move || {
                while !link.stop_requested() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
        };
        assert!(stop_worker(Worker { handle, link }, Duration::from_secs(30)));
    }

    /// Quitting ends the wait as well. The UI thread asks the transport to
    /// stop and then waits for it; it will not read the channel again, so a
    /// thread still offering it an event would hold the quit up for good.
    #[test]
    fn a_waiting_change_gives_up_when_the_app_is_quitting() {
        let link = Link::detached(ClientConfig::default());
        let (mut sender, _receiver) = mpsc::channel::<CollabEvent>(1);
        while sender
            .try_send(CollabEvent::Connecting { attempt: 0 })
            .is_ok()
        {}
        let change = CollabEvent::OpApplied {
            op_id: "op-peer".to_owned(),
            revision: 1,
            changes: Vec::new(),
        };
        let waiting = {
            let link = link.clone();
            std::thread::spawn(move || emit(&mut sender, &link, change))
        };
        std::thread::sleep(Duration::from_millis(30));
        assert!(!waiting.is_finished(), "nobody has read the channel");
        link.stop.store(true, Ordering::Relaxed);
        assert_eq!(waiting.join().expect("emit thread"), Err(()));
    }

    /// What the log says of an op is the two things `VAL-CLI-010` reads off
    /// it: which op, and the baseline it claims.
    #[test]
    fn an_op_describes_itself_by_id_and_baseline() {
        let op = Outbound::Op {
            session_id: None,
            base_revision: 1,
            op_id: "op-9-3".to_owned(),
            payload: serde_json::json!({}),
        };
        assert_eq!(op.describe(), "op op-9-3 base_revision=1");
    }

    // ── Coming back (`VAL-CROSS-008`) ────────────────────────────────────

    /// What one connection of a [`scripted_server`] does once it has read
    /// the `hello`: the frames it says, and whether it then drops the socket
    /// (a server that died) or holds it until the client hangs up.
    struct Scene {
        frames: Vec<String>,
        then_drop: bool,
    }

    /// Every frame each client of a [`scripted_server`] sent, the `hello`
    /// first: one list per connection, in the order they were taken.
    type Heard = std::sync::Arc<Mutex<Vec<Vec<String>>>>;

    /// The next connection to `listener` — one that can be polled — to be
    /// read the ordinary way. `None` when none has come in ten seconds, so
    /// that a client that never comes back fails its test instead of
    /// holding it.
    fn accepted(listener: &TcpListener) -> Option<TcpStream> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream.set_nonblocking(false).ok().map(|()| stream),
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return None,
            }
        }
    }

    /// A server that takes one connection per scene, one after the other,
    /// and keeps what each client sent. It waits ten seconds for each
    /// connection and then gives up, so that a client that never comes
    /// back fails its test instead of holding it.
    fn scripted_server(scenes: Vec<Scene>) -> (String, Heard, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        listener.set_nonblocking(true).expect("a listener that can be polled");
        let addr = listener.local_addr().expect("local addr");
        let url = format!("ws://{addr}/ws");
        let heard: Heard = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&heard);
        let keep = move |at: usize, text: String| {
            if let Ok(mut log) = log.lock() {
                while log.len() <= at {
                    log.push(Vec::new());
                }
                log[at].push(text);
            }
        };
        let handle = std::thread::spawn(move || {
            for (at, scene) in scenes.into_iter().enumerate() {
                let Some(stream) = accepted(&listener) else {
                    return;
                };
                let Ok(mut ws) = tungstenite::accept(stream) else {
                    return;
                };
                if let Ok(Message::Text(hello)) = ws.read() {
                    keep(at, hello.to_string());
                }
                for frame in &scene.frames {
                    if ws.send(Message::text(frame.clone())).is_err() {
                        break;
                    }
                }
                if scene.then_drop {
                    // Dropped here, with no close frame: a server that died.
                    continue;
                }
                loop {
                    match ws.read() {
                        Ok(Message::Text(text)) => keep(at, text.to_string()),
                        Ok(Message::Close(_)) => {
                            let _ = ws.flush();
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
        });
        (url, heard, handle)
    }

    /// Everything the client reports until `last` says it has seen enough,
    /// or twenty seconds have gone by.
    fn events_until(
        receiver: &mut mpsc::Receiver<CollabEvent>,
        last: impl Fn(&CollabEvent) -> bool,
    ) -> Vec<CollabEvent> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut events = Vec::new();
        while Instant::now() < deadline {
            match receiver.try_recv() {
                Ok(event) => {
                    let done = last(&event);
                    events.push(event);
                    if done {
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Closed) => break,
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        events
    }

    fn hello_of(heard: &Heard, connection: usize) -> serde_json::Value {
        let log = heard.lock().expect("log");
        let hello = log
            .get(connection)
            .and_then(|frames| frames.first())
            .unwrap_or_else(|| panic!("connection {connection} sent no hello: {log:?}"));
        serde_json::from_str(hello).expect("the hello is JSON")
    }

    /// A client that comes back says where its drawing is: the last
    /// revision it handed to the UI, in the `hello` of the next session —
    /// so that the server can send what was missed. The first `hello` of a
    /// run says nothing of the kind. What the server answers — where the
    /// replay starts — reaches the UI ahead of the session it belongs to.
    #[test]
    fn collab_offline_queue_the_next_hello_says_where_the_drawing_is() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":4,"peers":[],"claims":[]}"#.to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-5","revision":5,"changes":[]}"#.to_owned(),
            ],
            then_drop: true,
        };
        let second = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":7,"peers":[],"claims":[],"replay_from":5}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-6","revision":6,"changes":[]}"#.to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-7","revision":7,"changes":[]}"#.to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let link = Link::detached(config_for(url));
        let (mut receiver, worker) = start_client(&link);
        let events = events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::OpApplied { revision: 7, .. }),
        );
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(
            hello_of(&heard, 0).get("since").is_none(),
            "the first hello of a run has no revision to say"
        );
        assert_eq!(hello_of(&heard, 1)["since"], 5, "{events:?}");

        let resumed = events
            .iter()
            .position(|event| matches!(event, CollabEvent::Resuming { from: 5 }))
            .unwrap_or_else(|| panic!("the UI was not told where the replay starts: {events:?}"));
        assert!(
            matches!(
                events.get(resumed + 1),
                Some(CollabEvent::Connected { session_id, base_revision: 7, .. }) if session_id == "s-2"
            ),
            "{events:?}"
        );
        let revisions: Vec<i64> = events
            .iter()
            .filter_map(|event| match event {
                CollabEvent::OpApplied { revision, .. } => Some(*revision),
                _ => None,
            })
            .collect();
        assert_eq!(revisions, [5, 6, 7], "{events:?}");
    }

    /// `SESSION_EXPIRED`: the server has let go of the session, and takes
    /// no second `hello` on the socket. The client gives the socket up and
    /// joins again, as after any drop — it used to report an error and stay
    /// on a socket nothing could be done on.
    #[test]
    fn collab_offline_queue_a_session_the_server_let_go_of_is_joined_again() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":3,"peers":[],"claims":[]}"#.to_owned(),
                r#"{"type":"error","code":"SESSION_EXPIRED","message":"会话已过期"}"#.to_owned(),
            ],
            // Held: it is the client that has to hang up.
            then_drop: false,
        };
        let second = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":3,"peers":[],"claims":[],"replay_from":3}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let link = Link::detached(config_for(url));
        let (mut receiver, worker) = start_client(&link);
        let events = events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::Connected { session_id, .. } if session_id == "s-2"),
        );
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(
            matches!(
                events.last(),
                Some(CollabEvent::Connected { session_id, .. }) if session_id == "s-2"
            ),
            "the client did not join again: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                CollabEvent::Disconnected { reason, will_retry: true } if reason.contains("SESSION_EXPIRED")
            )),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|event| matches!(event, CollabEvent::ServerError { .. })),
            "it is not an error to report and sit on: {events:?}"
        );
        assert_eq!(hello_of(&heard, 1)["since"], 3);
    }

    /// An op is handed over for one session. The UI learns of a drop after
    /// this thread does, and may hand an op over for a session that is
    /// already gone; it hands it over again once it has heard of the next
    /// one. Sent on the session that is up, the room would have it twice.
    #[test]
    fn collab_offline_queue_an_op_handed_over_for_a_session_that_is_over_is_not_sent() {
        let (url, seen, _closed, server) = recording_server();
        let link = Link::detached(config_for(url));
        let (mut receiver, worker) = start_client(&link);
        assert!(session_came_up(&mut receiver), "the client never reported a session");

        let payload = serde_json::json!({"kind": "set_property", "object_ref": "P-1001"});
        queue_op(&link.outbox, Some("s-gone"), 3, "op-stale", payload.clone()).expect("op queues");
        queue_op(&link.outbox, Some("s-test"), 3, "op-live", payload).expect("op queues");

        let deadline = Instant::now() + Duration::from_secs(2);
        let sent = loop {
            let ops: Vec<String> = seen
                .lock()
                .expect("frame log")
                .iter()
                .filter(|frame| frame.contains("\"type\":\"op\""))
                .cloned()
                .collect();
            if ops.iter().any(|frame| frame.contains("op-live")) || Instant::now() >= deadline {
                break ops;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("op-live"), "{sent:?}");
    }

    // ── The drawing's own tags (`hello.existing_tags`) ───────────────────

    /// A link whose client asks which tags the drawing carries, as the
    /// app's does, with a slot of its own for the answers and `wait` for how
    /// long a dial holds for one.
    fn link_with_tags(url: String, wait: Duration) -> (Link, Arc<TagsSlot>) {
        let said: Arc<TagsSlot> = Arc::new(Mutex::new(None));
        let mut link = Link::detached(config_for(url));
        link.tags = Some(TagsLine {
            said: Arc::clone(&said),
            wait,
        });
        (link, said)
    }

    /// The client's next question about the drawing's tags: its number, and
    /// everything the client reported up to it.
    fn question(receiver: &mut mpsc::Receiver<CollabEvent>) -> (u64, Vec<CollabEvent>) {
        let events = events_until(receiver, |event| {
            matches!(event, CollabEvent::TagsWanted { .. })
        });
        match events.last() {
            Some(CollabEvent::TagsWanted { ask }) => (*ask, events),
            _ => panic!("the client did not ask which tags the drawing carries: {events:?}"),
        }
    }

    /// Everything the client reports in `time`.
    fn events_for(receiver: &mut mpsc::Receiver<CollabEvent>, time: Duration) -> Vec<CollabEvent> {
        let deadline = Instant::now() + time;
        let mut events = Vec::new();
        while Instant::now() < deadline {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Closed) => break,
                Err(mpsc::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        events
    }

    fn tags(list: &[&str]) -> TagsSaid {
        TagsSaid::Tags(list.iter().map(|tag| (*tag).to_owned()).collect(), None)
    }

    /// The `hello` lists the tags the UI said the session's drawing carries
    /// (`hello.existing_tags`) — and the dial holds until the UI has said
    /// them: a `hello` that went out first would have nothing to list. It
    /// used to list nothing at all, and the server took the room to be
    /// empty: a symbol a peer put in could be given a tag this drawing
    /// already had.
    #[test]
    fn collab_existing_tags_the_hello_lists_what_the_ui_said_the_drawing_carries() {
        let scene = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":0,"peers":[],"claims":[]}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![scene]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, before) = question(&mut receiver);
        // The question is out: nothing is dialled until it is answered.
        std::thread::sleep(Duration::from_millis(300));
        let dialled = heard.lock().expect("log").len();
        leave_answer(&said, ask, tags(&["P-1001", "V-1001"]));
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(
            before
                .iter()
                .any(|event| matches!(event, CollabEvent::Connecting { .. })),
            "the UI is told a connection is being made before it is asked: {before:?}"
        );
        assert_eq!(dialled, 0, "the hello went out before the UI had answered");
        assert!(up, "the client never reported a session");
        assert_eq!(
            hello_of(&heard, 0)["existing_tags"],
            serde_json::json!(["P-1001", "V-1001"])
        );
    }

    /// A session that came up before there was a drawing to speak for — an
    /// editor started on no file — said `hello` with nothing to list. When
    /// the UI then says which tags the drawing carries, the client leaves
    /// the session and joins again: `hello` is the only frame that carries
    /// them. It says where the drawing is, as on any other way back, and it
    /// asks again first — the answer may have changed.
    #[test]
    fn collab_existing_tags_tags_said_after_the_hello_are_told_by_joining_again() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":4,"peers":[],"claims":[]}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-5","revision":5,"changes":[]}"#.to_owned(),
            ],
            // Held: it is the client that hangs up.
            then_drop: false,
        };
        let second = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":5,"peers":[],"claims":[],"replay_from":5}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(&said, ask, TagsSaid::NotYet);
        let mut events = events_until(&mut receiver, |event| {
            matches!(event, CollabEvent::OpApplied { revision: 5, .. })
        });
        // The drawing is there now.
        leave_answer(&said, ask, tags(&["P-1001"]));
        let (again, more) = question(&mut receiver);
        events.extend(more);
        leave_answer(&said, again, tags(&["P-1001", "P-1002"]));
        events.extend(events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::Connected { session_id, .. } if session_id == "s-2"),
        ));
        // It stays: what it was told this time went with the `hello`.
        let after = events_for(&mut receiver, Duration::from_millis(500));
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(
            hello_of(&heard, 0).get("existing_tags").is_none(),
            "the first hello had nothing to list"
        );
        assert_ne!(again, ask, "each question has a number of its own");
        let rejoined = hello_of(&heard, 1);
        assert_eq!(
            rejoined["existing_tags"],
            serde_json::json!(["P-1001", "P-1002"]),
            "{events:?}"
        );
        assert_eq!(rejoined["since"], 5, "{events:?}");
        assert!(
            events.iter().any(|event| matches!(
                event,
                CollabEvent::Disconnected { reason, will_retry: true } if reason.contains("位号")
            )),
            "the UI is told why the session was left: {events:?}"
        );
        assert!(
            !after
                .iter()
                .any(|event| matches!(event, CollabEvent::Disconnected { .. })),
            "it left the session a second time: {after:?}"
        );
        assert_eq!(heard.lock().expect("log").len(), 2);
    }

    /// How the first connection of a [`claim_answering_server`] ended.
    #[derive(Clone, Debug, Default)]
    struct Goodbye {
        /// The client's close frame was read, not a dead socket in its place.
        taken: bool,
        /// The client was still on the line once it had been read.
        waited: bool,
        /// What ended the connection, when it was not a close frame.
        instead: Option<String>,
    }

    /// A server whose answer to a `claim` crosses the client's goodbye, as
    /// the real one's does when the claim is the first use of a drawing: it
    /// reads the claim, answers once the close frame behind it is on its
    /// way, and reads on only then. It takes two connections, one after the
    /// other, keeps what each client sent, and says how the first one ended.
    fn claim_answering_server() -> (
        String,
        Heard,
        Arc<Mutex<Goodbye>>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("a listener that can be polled");
        let url = format!("ws://{}/ws", listener.local_addr().expect("local addr"));
        let heard: Heard = Arc::new(Mutex::new(vec![Vec::new(), Vec::new()]));
        let goodbye = Arc::new(Mutex::new(Goodbye::default()));
        let (log, ending) = (Arc::clone(&heard), Arc::clone(&goodbye));
        let handle = std::thread::spawn(move || {
            for at in 0..2 {
                let Some(stream) = accepted(&listener) else {
                    return;
                };
                let Ok(mut ws) = tungstenite::accept(stream) else {
                    return;
                };
                let keep = |text: String| {
                    if let Ok(mut log) = log.lock() {
                        log[at].push(text);
                    }
                };
                if let Ok(Message::Text(hello)) = ws.read() {
                    keep(hello.to_string());
                }
                let welcome = format!(
                    r#"{{"type":"welcome","session_id":"s-{}","base_revision":0,"peers":[],"claims":[]}}"#,
                    at + 1
                );
                if ws.send(Message::text(welcome)).is_err() {
                    return;
                }
                loop {
                    match ws.read() {
                        Ok(Message::Text(text)) => {
                            let claim = text.as_str().contains(r#""type":"claim""#);
                            keep(text.to_string());
                            if !claim {
                                continue;
                            }
                            // The claim is being written down, and meanwhile
                            // the client says goodbye: the answer goes out
                            // once its close frame is here to be read, and
                            // before it is read.
                            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(5)));
                            let _ = ws.get_ref().peek(&mut [0u8; 1]);
                            let _ = ws.get_ref().set_read_timeout(None);
                            std::thread::sleep(Duration::from_millis(50));
                            let granted = format!(
                                r#"{{"type":"claim_granted","tag":"P-1001","session_id":"s-{}"}}"#,
                                at + 1
                            );
                            let _ = ws.send(Message::text(granted));
                            // Time for the answer to get there — and, from a
                            // client that has hung up, for the reset to come
                            // back.
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        Ok(Message::Close(_)) => {
                            // A client that waits for the answer to its
                            // goodbye has nothing more to say: a read that
                            // times out finds it there, one that ends finds
                            // it gone.
                            let _ = ws
                                .get_ref()
                                .set_read_timeout(Some(Duration::from_millis(200)));
                            let waited = matches!(
                                ws.get_ref().peek(&mut [0u8; 1]),
                                Err(error) if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::TimedOut
                                )
                            );
                            if at == 0 {
                                if let Ok(mut ending) = ending.lock() {
                                    ending.taken = true;
                                    ending.waited = waited;
                                }
                            }
                            let _ = ws.flush();
                            break;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            if at == 0 {
                                if let Ok(mut ending) = ending.lock() {
                                    ending.instead = Some(error.to_string());
                                }
                            }
                            break;
                        }
                    }
                }
            }
        });
        (url, heard, goodbye, handle)
    }

    /// A session that is left to tell the drawing's tags is over at the
    /// server before the next one starts — the server has read its close
    /// frame — also when the server had an answer on its way: to a claim the
    /// session made just before it left. (That is the first use of a drawing
    /// opened into a session that had none: the claim is made and the tags
    /// are said in one go.) The client used to hang up the moment its close
    /// frame was out. The answer then met a closed socket, the connection
    /// was reset, and the close frame behind it was never read: the session
    /// lived on at the server, holding the claim, until its heartbeat timed
    /// out — and the session that replaced it was refused the claim it came
    /// to take back.
    #[test]
    fn collab_existing_tags_a_session_left_to_tell_them_is_over_before_the_next_starts() {
        let (url, heard, goodbye, server) = claim_answering_server();
        let (mut link, said) = link_with_tags(url, Duration::from_secs(30));
        // Longer than the test waits for the next session: it is the
        // server's answer that ends the leave here, not the time running out.
        link.leave_wait = Duration::from_secs(60);
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(&said, ask, TagsSaid::NotYet);
        let up = session_came_up(&mut receiver);
        // The first use of the drawing: a claim, and the tags it carries.
        queue_claim(&link.outbox, "P-1001").expect("a claim to make");
        let claimed = (0..1000).any(|_| {
            std::thread::sleep(Duration::from_millis(5));
            heard.lock().expect("log")[0]
                .iter()
                .any(|frame| frame.contains(r#""type":"claim""#))
        });
        leave_answer(&said, ask, tags(&["P-1001"]));
        let (again, mut events) = question(&mut receiver);
        leave_answer(&said, again, tags(&["P-1001"]));
        events.extend(events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::Connected { session_id, .. } if session_id == "s-2"),
        ));
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        let goodbye = goodbye.lock().expect("how it ended").clone();

        assert!(up, "the client never reported a session");
        assert!(claimed, "the claim never reached the session");
        assert!(
            goodbye.taken,
            "the server never read the close frame of the session that was left: {:?}",
            goodbye.instead
        );
        assert!(
            goodbye.waited,
            "the client had hung up before the server answered its goodbye"
        );
        assert!(
            matches!(
                events.last(),
                Some(CollabEvent::Connected { session_id, .. }) if session_id == "s-2"
            ),
            "the client did not join again: {events:?}"
        );
        // The answer that crossed the goodbye is to a session that is over:
        // the next one asks again, and the UI is not handed this one.
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, CollabEvent::ClaimGranted { .. })),
            "{events:?}"
        );
        assert_eq!(
            hello_of(&heard, 1)["existing_tags"],
            serde_json::json!(["P-1001"])
        );
    }

    /// A server that does not take the goodbye — one that is wedged — holds
    /// the client only so long: when the wait is up it hangs up and joins
    /// again, as it did before there was a wait.
    #[test]
    fn collab_existing_tags_a_server_that_does_not_take_the_goodbye_holds_the_leave_only_so_long() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("a listener that can be polled");
        let url = format!("ws://{}/ws", listener.local_addr().expect("local addr"));
        let heard: Heard = Arc::new(Mutex::new(vec![Vec::new(), Vec::new()]));
        let log = Arc::clone(&heard);
        let server = std::thread::spawn(move || {
            let mut held = Vec::new();
            for at in 0..2 {
                let Some(stream) = accepted(&listener) else {
                    return;
                };
                let Ok(mut ws) = tungstenite::accept(stream) else {
                    return;
                };
                if let Ok(Message::Text(hello)) = ws.read() {
                    if let Ok(mut log) = log.lock() {
                        log[at].push(hello.to_string());
                    }
                }
                let welcome = format!(
                    r#"{{"type":"welcome","session_id":"s-{}","base_revision":0,"peers":[],"claims":[]}}"#,
                    at + 1
                );
                if ws.send(Message::text(welcome)).is_err() {
                    return;
                }
                if at == 0 {
                    // Wedged: nothing more is read on this connection, and
                    // it is not closed either.
                    held.push(ws);
                } else {
                    while ws.read().is_ok() {}
                }
            }
        });
        let wait = Duration::from_millis(600);
        let (mut link, said) = link_with_tags(url, Duration::from_secs(30));
        link.leave_wait = wait;
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(&said, ask, TagsSaid::NotYet);
        let up = session_came_up(&mut receiver);
        let told = Instant::now();
        leave_answer(&said, ask, tags(&["P-1001"]));
        let left = events_until(&mut receiver, |event| {
            matches!(event, CollabEvent::Disconnected { .. })
        });
        let stayed = told.elapsed();
        let (again, _) = question(&mut receiver);
        leave_answer(&said, again, tags(&["P-1001"]));
        let events = events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::Connected { session_id, .. } if session_id == "s-2"),
        );
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(up, "the client never reported a session");
        assert!(
            matches!(
                left.last(),
                Some(CollabEvent::Disconnected { reason, will_retry: true }) if reason.contains("位号")
            ),
            "the session was not left to tell the tags: {left:?}"
        );
        assert!(
            stayed >= wait,
            "the client hung up after {stayed:?}, without giving the server {wait:?} to take its goodbye"
        );
        assert!(
            stayed < wait + Duration::from_secs(5),
            "the client stayed {stayed:?} on a server that was given {wait:?} to take its goodbye"
        );
        assert!(
            matches!(
                events.last(),
                Some(CollabEvent::Connected { session_id, .. }) if session_id == "s-2"
            ),
            "the client did not join again: {events:?}"
        );
        assert_eq!(
            hello_of(&heard, 1)["existing_tags"],
            serde_json::json!(["P-1001"])
        );
    }

    /// A drawing with no tags has nothing to tell the server: the session
    /// that came up before the drawing was there stays as it is.
    #[test]
    fn collab_existing_tags_a_drawing_with_no_tags_does_not_join_again() {
        let scene = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":0,"peers":[],"claims":[]}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![scene]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(&said, ask, TagsSaid::NotYet);
        let up = session_came_up(&mut receiver);
        leave_answer(&said, ask, TagsSaid::Tags(Vec::new(), None));
        let after = events_for(&mut receiver, Duration::from_millis(600));
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(up, "the client never reported a session");
        assert!(
            !after.iter().any(|event| matches!(
                event,
                CollabEvent::Disconnected { .. } | CollabEvent::Connecting { .. }
            )),
            "{after:?}"
        );
        assert_eq!(
            heard.lock().expect("log").len(),
            1,
            "one session, one hello"
        );
    }

    /// A UI that does not answer — a drawing that takes long to load — holds
    /// the dial only so long: the session comes up without the tags, as it
    /// did before there was a question to ask, and they are told by joining
    /// again when the answer does come.
    #[test]
    fn collab_existing_tags_a_question_nobody_answers_holds_the_dial_only_so_long() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":0,"peers":[],"claims":[]}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let second = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":0,"peers":[],"claims":[]}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let (link, said) = link_with_tags(url, Duration::from_millis(200));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        let up = session_came_up(&mut receiver);
        leave_answer(&said, ask, tags(&["P-1001"]));
        let (again, _) = question(&mut receiver);
        leave_answer(&said, again, tags(&["P-1001"]));
        let events = events_until(
            &mut receiver,
            |event| matches!(event, CollabEvent::Connected { session_id, .. } if session_id == "s-2"),
        );
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();

        assert!(up, "the session did not come up without the answer");
        assert!(
            hello_of(&heard, 0).get("existing_tags").is_none(),
            "nothing had been said when the first hello went out"
        );
        assert!(
            matches!(
                events.last(),
                Some(CollabEvent::Connected { session_id, .. }) if session_id == "s-2"
            ),
            "the client did not join again: {events:?}"
        );
        assert_eq!(
            hello_of(&heard, 1)["existing_tags"],
            serde_json::json!(["P-1001"])
        );
    }

    /// The wait for the UI's answer ends when the app quits: the UI thread
    /// is then waiting on this one, and will say nothing more.
    #[test]
    fn collab_existing_tags_a_quit_ends_the_wait_for_the_answer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("a listener that can be polled");
        let url = format!("ws://{}/ws", listener.local_addr().expect("local addr"));
        let (link, _said) = link_with_tags(url, Duration::from_secs(60));
        let (mut receiver, worker) = start_client(&link);
        let _ = question(&mut receiver);
        link.stop.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            worker.is_finished(),
            "the client went on waiting for the UI"
        );
        let _ = worker.join();
        assert!(listener.accept().is_err(), "it dialled without the answer");
    }

    /// The app's client asks the UI; one that shares nothing with the app —
    /// it has no UI — does not, and waits for nobody. What the UI says
    /// reaches the slot the app's client reads.
    #[test]
    fn collab_existing_tags_the_apps_client_asks_the_ui_and_hears_what_it_says() {
        let app = Link::app();
        let line = app.tags.as_ref().expect("the app's client asks the UI");
        assert_eq!(line.wait, TAGS_WAIT);
        assert!(Link::detached(config_for(String::new())).tags.is_none());

        // Other tests of this process answer through the same slot, and one
        // of theirs may land in between: said again until it is read back.
        let ask = u64::MAX - 0x7A65;
        let said = tags(&["P-1001"]);
        let heard = (0..200).any(|_| {
            answer_tags(ask, said.clone());
            line.answer(ask).as_ref() == Some(&said)
        });
        assert!(heard, "what the UI said did not reach the app's client");
    }

    /// An answer is to one question: what the UI said to an earlier one —
    /// of the dial before, or of a client iced has since started over — is
    /// not an answer to this. Of what it said to this one, the last stands.
    #[test]
    fn collab_existing_tags_an_answer_to_another_question_is_not_one_to_this() {
        let (link, said) = link_with_tags(String::new(), Duration::from_secs(1));
        let line = link.tags.as_ref().expect("a link that asks");
        assert_eq!(line.answer(7), None, "nothing said yet");
        leave_answer(&said, 6, tags(&["P-1001"]));
        assert_eq!(line.answer(7), None);
        leave_answer(&said, 7, TagsSaid::NotYet);
        assert_eq!(line.answer(7), Some(TagsSaid::NotYet));
        leave_answer(&said, 7, tags(&["V-1001"]));
        assert_eq!(line.answer(7), Some(tags(&["V-1001"])));
    }

    /// `REQ-COLL-015`: a `hello` claims what the drawing file was saved
    /// aligned to — repository and revision together, never a revision it
    /// cannot say of which repository. A copy that was never in a room
    /// claims nothing.
    #[test]
    fn collab_align_the_hello_claims_what_the_file_was_saved_with() {
        let scene = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":50,"peers":[],"claims":[],"repo_id":"r-9"}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![scene]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(
            &said,
            ask,
            TagsSaid::Tags(
                Vec::new(),
                Some(crate::app::collab::FileClaim {
                    drawing_id: "PID-1101".into(),
                    repo_id: "r-9".into(),
                    revision: 42,
                }),
            ),
        );
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the session did not come up");
        let hello = hello_of(&heard, 0);
        assert_eq!(hello["repo_id"], "r-9");
        assert_eq!(hello["since"], 42);
    }

    /// A claim of another drawing says nothing: the file's `drawing_id`
    /// does not match the session's, so the `hello` goes out bare.
    #[test]
    fn collab_align_a_claim_of_another_drawing_is_not_made() {
        let scene = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":0,"peers":[],"claims":[],"repo_id":"r-9"}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![scene]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(
            &said,
            ask,
            TagsSaid::Tags(
                Vec::new(),
                Some(crate::app::collab::FileClaim {
                    drawing_id: "PID-OTHER".into(),
                    repo_id: "r-9".into(),
                    revision: 42,
                }),
            ),
        );
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the session did not come up");
        let hello = hello_of(&heard, 0);
        assert!(hello.get("repo_id").is_none(), "{hello}");
        assert!(hello.get("since").is_none(), "{hello}");
    }

    /// `REQ-COLL-015`: a refusal reaches the UI as its own event, right
    /// ahead of the session it belongs to — the session is up, and the word
    /// that this copy is not taken as consistent is neither folded into a
    /// generic server error nor said after the UI could have acted on the
    /// session (`OFF-2`: it rides in the `welcome`).
    #[test]
    fn collab_align_refused_reaches_the_ui_ahead_of_its_session() {
        let scene = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":50,"peers":[],"claims":[],"repo_id":"r-9","align_refused":{"code":"REPO_MISMATCH","message":"这份文件属于另一个服务器库"}}"#
                    .to_owned(),
            ],
            then_drop: true,
        };
        let (url, _heard, server) = scripted_server(vec![scene]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let (ask, _) = question(&mut receiver);
        leave_answer(&said, ask, TagsSaid::NotYet);
        let events = events_until(&mut receiver, |event| {
            matches!(event, CollabEvent::Connected { .. })
        });
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        let refused = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    CollabEvent::AlignRefused { code, .. } if code == "REPO_MISMATCH"
                )
            })
            .unwrap_or_else(|| panic!("the refusal did not arrive: {events:?}"));
        assert!(
            matches!(events.get(refused + 1), Some(CollabEvent::Connected { .. })),
            "{events:?}"
        );
    }

    /// What the file says the drawing is: aligned to `r-9`, at 42.
    fn file_r9_at_42() -> TagsSaid {
        TagsSaid::Tags(
            Vec::new(),
            Some(crate::app::collab::FileClaim {
                drawing_id: "PID-1101".into(),
                repo_id: "r-9".into(),
                revision: 42,
            }),
        )
    }

    /// `OFF-2` (R1 of `SPEC-collab-offline-sync.md` §11.2): on the first
    /// session of a run, a replay starts where the file said the drawing
    /// was, and each replayed op moves it on. A replay cut short is asked
    /// for again from the last op handed to the UI — not from the room's
    /// revision, which the drawing never reached.
    #[test]
    fn collab_align_refused_a_replay_cut_short_is_asked_for_again_from_where_it_stopped() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":45,"peers":[],"claims":[],"repo_id":"r-9","replay_from":42}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-43","revision":43,"changes":[]}"#.to_owned(),
            ],
            then_drop: true,
        };
        let second = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":45,"peers":[],"claims":[],"repo_id":"r-9","replay_from":43}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        for _ in 0..2 {
            let (ask, _) = question(&mut receiver);
            leave_answer(&said, ask, file_r9_at_42());
        }
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the second session did not come up");
        assert_eq!(hello_of(&heard, 0)["since"], 42);
        let again = hello_of(&heard, 1);
        assert_eq!(again["since"], 43, "{again}");
        assert_eq!(again["repo_id"], "r-9", "{again}");
    }

    /// `OFF-2`: a session the server would not take as consistent says
    /// nothing of where the drawing is. Its `welcome` names the room's
    /// repository and revision, which the drawing never took: the next
    /// `hello` claims what the drawing is — the file's word — again.
    #[test]
    fn collab_align_refused_the_next_hello_claims_what_the_drawing_is_again() {
        let refusing = r#"{"type":"welcome","session_id":"s-1","base_revision":50,"peers":[],"claims":[],"repo_id":"r-other","align_refused":{"code":"REPO_MISMATCH","message":"协同库标识不一致"}}"#;
        let first = Scene {
            frames: vec![refusing.to_owned()],
            then_drop: true,
        };
        let second = Scene {
            frames: vec![refusing.replace("s-1", "s-2")],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, second]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        for _ in 0..2 {
            let (ask, _) = question(&mut receiver);
            leave_answer(&said, ask, file_r9_at_42());
        }
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the second session did not come up");
        let again = hello_of(&heard, 1);
        assert_eq!(again["repo_id"], "r-9", "{again}");
        assert_eq!(again["since"], 42, "{again}");
    }

    /// `OFF-3`: a copy that holds itself apart before its `hello`
    /// ([`TagsSaid::Apart`]) claims nothing — neither the file's word nor
    /// where the last session left the drawing — and what it hears on that
    /// connection does not move on what the next `hello` claims.
    #[test]
    fn collab_op_ids_a_copy_held_apart_claims_nothing_and_takes_nothing_from_its_session() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":42,"peers":[],"claims":[],"repo_id":"r-9","replay_from":42}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-43","revision":43,"changes":[]}"#.to_owned(),
            ],
            then_drop: true,
        };
        let apart = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":50,"peers":[],"claims":[],"repo_id":"r-9"}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-51","revision":51,"changes":[]}"#.to_owned(),
            ],
            then_drop: true,
        };
        let third = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-3","base_revision":51,"peers":[],"claims":[],"repo_id":"r-9","replay_from":43}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, apart, third]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        let held_apart = TagsSaid::Apart(crate::app::collab::NotAligned {
            code: "QUEUED_BY_OTHERS".into(),
            message: "图里带着 u1 排队未提交的修改".into(),
        });
        for answer in [file_r9_at_42(), held_apart, file_r9_at_42()] {
            let (ask, _) = question(&mut receiver);
            leave_answer(&said, ask, answer);
        }
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the third session did not come up");
        let apart = hello_of(&heard, 1);
        assert!(apart.get("repo_id").is_none(), "{apart}");
        assert!(apart.get("since").is_none(), "{apart}");
        let again = hello_of(&heard, 2);
        assert_eq!(again["since"], 43, "{again}");
        assert_eq!(again["repo_id"], "r-9", "{again}");
    }

    /// `OFF-9`: a copy taken off the room it was in (`PIDCOLLAB DETACH`,
    /// [`TagsSaid::Detached`]) claims nothing — not where the last session
    /// left the drawing either — and the session it then joins is where
    /// the next `hello` starts from.
    #[test]
    fn collab_unsynced_a_detached_copy_claims_nothing_of_where_the_last_session_left_it() {
        let first = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-1","base_revision":42,"peers":[],"claims":[],"repo_id":"r-9","replay_from":42}"#
                    .to_owned(),
                r#"{"type":"op_applied","op_id":"op-peer-43","revision":43,"changes":[]}"#.to_owned(),
            ],
            then_drop: true,
        };
        let detached = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-2","base_revision":50,"peers":[],"claims":[],"repo_id":"r-9"}"#
                    .to_owned(),
            ],
            then_drop: true,
        };
        let third = Scene {
            frames: vec![
                r#"{"type":"welcome","session_id":"s-3","base_revision":50,"peers":[],"claims":[],"repo_id":"r-9","replay_from":50}"#
                    .to_owned(),
            ],
            then_drop: false,
        };
        let (url, heard, server) = scripted_server(vec![first, detached, third]);
        let (link, said) = link_with_tags(url, Duration::from_secs(30));
        let (mut receiver, worker) = start_client(&link);
        for answer in [
            file_r9_at_42(),
            TagsSaid::Detached(Vec::new()),
            TagsSaid::Tags(Vec::new(), None),
        ] {
            let (ask, _) = question(&mut receiver);
            leave_answer(&said, ask, answer);
        }
        let up = session_came_up(&mut receiver);
        drop(receiver);
        let _ = worker.join();
        let _ = server.join();
        assert!(up, "the third session did not come up");
        let detached = hello_of(&heard, 1);
        assert!(detached.get("since").is_none(), "{detached}");
        assert!(detached.get("repo_id").is_none(), "{detached}");
        let again = hello_of(&heard, 2);
        assert_eq!(again["since"], 50, "{again}");
        assert_eq!(again["repo_id"], "r-9", "{again}");
    }
}

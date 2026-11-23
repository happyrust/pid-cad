//! WebSocket data plane (`GET /ws`).
//!
//! A connection runs three phases: a `hello` handshake under a 10 s deadline,
//! a `select!` loop over client frames, room broadcasts and a 1 s housekeeping
//! tick, and a disconnect path that leaves the session and its claims for the
//! heartbeat timeout to reap — per appendix A.2 a dropped socket must not lose
//! the drawing's occupancy (VAL-SRV-027).
//!
//! The protocol logic lives in [`handle_message`] and [`sweep_expired`], which
//! return [`Reaction`]s instead of touching the socket; the thin
//! [`apply_reactions`] executor is the only code that needs a live `WebSocket`,
//! so the whole protocol is unit-testable without one.
//!
//! Room fan-out note: every room message — `peer_joined` included — is
//! published to the whole room, including its originator. [`should_forward`]
//! stops a client from seeing its own `peer_joined` / `peer_left`, while
//! `op_applied` and `claim_released` deliberately still reach the sender
//! (VAL-SRV-016, VAL-SRV-012). Clients therefore must ignore self-addressed
//! peer events on their side too.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::response::Response;
use ocs_collab::protocol::{codes, release_reasons, AlignRefusal, ClaimInfo, Peer};
use ocs_collab::{
    new_session_id, now_unix, ClaimOutcome, ClientMessage, Occupancy, OpenOutcome, OpenRequest,
    ReleaseOutcome, ServerMessage, Session, SessionStatus,
};
use serde_json::Value;
use tokio::time::{interval, timeout, MissedTickBehavior};

use crate::bus::Subscription;
use crate::store::{ClaimRow, NewAuditRow, SessionRow, Store};
use crate::AppState;

/// How long a socket may sit before sending `hello`. Keeps dead TCP peers
/// from occupying the map of open connections forever.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Housekeeping cadence for stale-session sweeps and claim expiry.
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// `GET /ws` — upgrades the request and hands the socket to [`handle_socket`].
///
/// The peer address feeds the `ip` audit column; the router is served through
/// `into_make_service_with_connect_info` so `ConnectInfo` is always present.
pub async fn upgrade(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
) -> Response {
    let peer_addr = addr.ip().to_string();
    ws.on_upgrade(move |socket| handle_socket(socket, state, peer_addr))
}

/// Per-socket bookkeeping: which session the connection speaks for and the
/// peer address stamped on its audit rows.
#[derive(Debug)]
pub struct Connection {
    /// The handshaken session; `None` until `hello` succeeds. Only the
    /// immutable identity fields are read from this copy — liveness checks
    /// always go through the registry, which owns the current status.
    pub session: Option<Session>,
    /// Peer IP for audit rows (loopback when the socket had no address).
    pub peer_addr: String,
    /// The room subscription `hello` took out in the same step in which it
    /// read the drawing's revision, for the socket loop to take over. See
    /// [`hello`] for why it cannot wait for the reactions to be applied.
    pub joined: Option<Subscription>,
    /// Set when the `welcome` refused this connection's alignment: every
    /// `op` it sends is answered `NOT_ALIGNED` and goes no further.
    pub not_aligned: bool,
}

impl Connection {
    pub fn new(peer_addr: impl Into<String>) -> Self {
        Connection {
            session: None,
            peer_addr: peer_addr.into(),
            joined: None,
            not_aligned: false,
        }
    }
}

/// One side effect the socket layer performs for a handled message. Keeping
/// the effects as data is what makes the protocol unit-testable.
#[derive(Debug, Clone, PartialEq)]
pub enum Reaction {
    /// Reply to this connection only.
    Send(ServerMessage),
    /// Deliver to one session only, wherever its socket lives.
    ///
    /// The room bus is per-drawing, so it cannot express "this peer and nobody
    /// else"; targeted messages (the takeover error) need the per-session
    /// unicast channel on `AppState::direct`.
    SendTo {
        session_id: String,
        message: ServerMessage,
    },
    /// Fan out to every subscriber of `drawing_id`, the originator included.
    Publish { drawing_id: String, message: ServerMessage },
    /// Already fanned out by whoever produced it; listed so the caller can
    /// see what went to the room. An `op_applied` is one: it is published
    /// inside the critical section that moved the revision
    /// (`ops::commit`), so the room hears revisions in the order they were
    /// made. Left to the caller, two ops committed a moment apart on two
    /// connections could reach the room newest first.
    Published { drawing_id: String, message: ServerMessage },
    /// Join the drawing's broadcast room. Emitted by a successful `hello`
    /// *before* the `peer_joined` publish, so no room traffic is missed.
    Subscribe { drawing_id: String },
    /// Shut the connection down (after pending `Send`s are flushed).
    Close,
    /// Message consumed; nothing further to do.
    Nothing,
}

/// One socket's full lifecycle.
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>, peer_addr: String) {
    // One line per socket, before anything is said on it: a client that must
    // not dial out at all (`VAL-CLI-017`) is shown by this line never
    // appearing, which the session-level lines below cannot show.
    println!(
        "[ocs_collab_server] 入站连接 peer={peer_addr} at={}",
        now_unix()
    );
    let mut conn = Connection::new(peer_addr);
    let mut subscription: Option<Subscription> = None;

    let handshaken = timeout(
        HELLO_TIMEOUT,
        wait_for_hello(&mut socket, &state, &mut conn, &mut subscription),
    )
    .await
    .unwrap_or(false);
    if !handshaken {
        return;
    }

    // The unicast sender is registered only now: `hello` is what produces the
    // session id. Every early return below must go through the deregistration
    // at the tail of this function, otherwise `state.direct` leaks one entry
    // per closed socket.
    let direct_session_id = conn
        .session
        .as_ref()
        .map(|session| session.session_id.clone());
    let mut direct_rx = direct_session_id.as_ref().map(|session_id| {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        state
            .direct
            .lock()
            .expect("direct mutex poisoned")
            .insert(session_id.clone(), sender);
        receiver
    });

    let mut ticker = interval(SWEEP_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Set when the client said goodbye with a `Close` frame: the session is
    // retired here and the room has already been told, so the disconnect tail
    // must not repeat either step.
    let mut retired = false;
    loop {
        tokio::select! {
            frame = socket.recv() => {
                match frame {
                    Some(Ok(message)) => {
                        if let Message::Ping(payload) = &message {
                            if socket.send(Message::Pong(payload.clone())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        match classify(&message) {
                            Incoming::Message(client_message) => {
                                let reactions =
                                    handle_message(&state, &mut conn, client_message, now_unix())
                                        .await;
                                if let Some(joined) = conn.joined.take() {
                                    subscription = Some(joined);
                                }
                                if !apply_reactions(
                                    &mut socket,
                                    &state,
                                    &mut subscription,
                                    reactions,
                                )
                                .await
                                {
                                    break;
                                }
                            }
                            Incoming::Bad => {
                                if !send_message(&mut socket, &invalid_message_error()).await {
                                    break;
                                }
                            }
                            Incoming::Close => {
                                // A close frame is a deliberate goodbye, not a
                                // dead socket: retire the session now instead
                                // of waiting out the heartbeat timeout, so a
                                // client that exits cleanly disappears from the
                                // roster (VAL-CLI-016) as promptly as it left.
                                retired = close_session(&state, &conn).await;
                                break;
                            }
                            Incoming::Ignore => {}
                        }
                    }
                    Some(Err(_)) | None => break,
                }
            }
            room = async {
                match subscription.as_mut() {
                    Some(subscription) => subscription.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(message) = room {
                    let own = conn
                        .session
                        .as_ref()
                        .map(|session| session.session_id.as_str())
                        .unwrap_or_default();
                    if should_forward(own, &message) && !send_message(&mut socket, &message).await {
                        break;
                    }
                }
            }
            direct = async {
                match direct_rx.as_mut() {
                    Some(receiver) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match direct {
                    Some(message) => {
                        let own = conn
                            .session
                            .as_ref()
                            .map(|session| session.session_id.as_str())
                            .unwrap_or_default();
                        let earlier = match subscription.as_mut() {
                            Some(subscription) => earlier_in_the_room(subscription, own),
                            None => Vec::new(),
                        };
                        let mut sent = true;
                        for message in earlier.iter().chain([&message]) {
                            if !send_message(&mut socket, message).await {
                                sent = false;
                                break;
                            }
                        }
                        if !sent {
                            break;
                        }
                    }
                    // Every sender for this session is gone (the registry entry
                    // was reaped), so nothing can be addressed here any more.
                    // Ending the loop beats spinning on a permanently ready
                    // `None`.
                    None => break,
                }
            }
            _ = ticker.tick() => {
                let reactions = sweep_expired(&state, now_unix()).await;
                if !apply_reactions(&mut socket, &state, &mut subscription, reactions).await {
                    break;
                }
            }
        }
    }

    if let Some(session_id) = &direct_session_id {
        state
            .direct
            .lock()
            .expect("direct mutex poisoned")
            .remove(session_id);
    }

    // The session and its claims survive a dropped socket: the heartbeat
    // timeout, not the socket close, owns teardown (VAL-SRV-027). A session
    // already retired by an explicit `Close` frame needs neither step again —
    // `close_session` wrote the row and told the room.
    if !retired {
        if let Some(session) = &conn.session {
            state.bus.publish(
                &session.drawing_id,
                &ServerMessage::PeerLeft(peer_of(session)),
            );
        }
    }
}

/// Retires the connection's session after an explicit `Close` frame.
///
/// Returns whether the room was told, so the caller can skip the disconnect
/// tail. A session that is already stale, taken over or unknown is left alone:
/// its `peer_left` went out when it went stale, and repeating it would make the
/// roster shrink twice.
async fn close_session(state: &AppState, conn: &Connection) -> bool {
    let Some(session) = &conn.session else {
        return false;
    };
    let retired = {
        let mut sessions = state.sessions.lock().await;
        sessions.retire(&session.session_id)
    };
    let Some(retired) = retired else {
        return false;
    };
    println!(
        "[ocs_collab_server] 入站 close 帧 session={} user={} -> 会话判失效",
        retired.session_id, retired.user_id
    );
    if let Some(store) = &state.store {
        if let Err(err) = store.upsert_session(&session_row(&retired)) {
            eprintln!("[ocs_collab_server] 写入已退出会话失败：{err}");
        }
    }
    for occupancy in state
        .occupancy
        .lock()
        .await
        .release_session(&retired.session_id)
    {
        if let Some(store) = &state.store {
            if let Err(err) = store.delete_claim(&occupancy.tag) {
                eprintln!("[ocs_collab_server] 清理已退出会话的占用失败：{err}");
            }
        }
        state.bus.publish(
            &occupancy.drawing_id,
            &ServerMessage::ClaimReleased {
                tag: occupancy.tag,
                reason: release_reasons::EXPIRED.to_owned(),
            },
        );
    }
    state
        .bus
        .publish(&retired.drawing_id, &ServerMessage::PeerLeft(peer_of(&retired)));
    println!(
        "[ocs_collab_server] 广播 peer_left session={} user={} drawing={} reason=close-frame",
        retired.session_id, retired.user_id, retired.drawing_id
    );
    true
}

/// Reads frames until `hello` establishes a session. Returns `false` when the
/// peer goes away or the socket errors before that happens.
async fn wait_for_hello(
    socket: &mut WebSocket,
    state: &AppState,
    conn: &mut Connection,
    subscription: &mut Option<Subscription>,
) -> bool {
    while let Some(frame) = socket.recv().await {
        let Ok(message) = frame else { return false };
        if let Message::Ping(payload) = &message {
            if socket.send(Message::Pong(payload.clone())).await.is_err() {
                return false;
            }
            continue;
        }
        match classify(&message) {
            Incoming::Message(client_message) => {
                let reactions = handle_message(state, conn, client_message, now_unix()).await;
                let opened = conn.session.is_some();
                if let Some(joined) = conn.joined.take() {
                    *subscription = Some(joined);
                }
                if !apply_reactions(socket, state, subscription, reactions).await {
                    return false;
                }
                if opened {
                    return true;
                }
            }
            Incoming::Bad => {
                if !send_message(socket, &invalid_message_error()).await {
                    return false;
                }
            }
            Incoming::Close => return false,
            Incoming::Ignore => {}
        }
    }
    false
}

/// One inbound frame, classified for the two read loops.
enum Incoming {
    Message(ClientMessage),
    /// A text/binary frame that is not a `ClientMessage`.
    Bad,
    Close,
    /// Pongs and anything else the protocol does not consume.
    Ignore,
}

fn classify(message: &Message) -> Incoming {
    match message {
        Message::Text(text) => match serde_json::from_str::<ClientMessage>(text) {
            Ok(client_message) => Incoming::Message(client_message),
            Err(_) => Incoming::Bad,
        },
        Message::Binary(bytes) => match serde_json::from_slice::<ClientMessage>(bytes) {
            Ok(client_message) => Incoming::Message(client_message),
            Err(_) => Incoming::Bad,
        },
        Message::Close(_) => Incoming::Close,
        _ => Incoming::Ignore,
    }
}

/// Executes reactions in order; returns `false` once the socket must close.
async fn apply_reactions(
    socket: &mut WebSocket,
    state: &AppState,
    subscription: &mut Option<Subscription>,
    reactions: Vec<Reaction>,
) -> bool {
    for reaction in reactions {
        match reaction {
            Reaction::Send(message) => {
                if !send_message(socket, &message).await {
                    return false;
                }
            }
            Reaction::SendTo {
                session_id,
                message,
            } => {
                let sender = state
                    .direct
                    .lock()
                    .expect("direct mutex poisoned")
                    .get(&session_id)
                    .cloned();
                match sender {
                    Some(tx) => {
                        if tx.send(message).is_err() {
                            // The receiving socket is gone; drop the stale key so
                            // the map does not accumulate dead sessions.
                            state
                                .direct
                                .lock()
                                .expect("direct mutex poisoned")
                                .remove(&session_id);
                        }
                    }
                    None => {
                        // Session already disconnected: nobody to deliver to.
                    }
                }
            }
            Reaction::Publish {
                drawing_id,
                message,
            } => {
                state.bus.publish(&drawing_id, &message);
            }
            Reaction::Published { .. } => {}
            Reaction::Subscribe { drawing_id } => {
                // `hello` joins the room itself (`Connection::joined`), and
                // that subscription is the one that has missed nothing.
                let already = subscription
                    .as_ref()
                    .is_some_and(|joined| joined.drawing_id() == drawing_id);
                if !already {
                    *subscription = Some(state.bus.subscribe(&drawing_id));
                }
            }
            Reaction::Close => {
                let _ = socket.send(Message::Close(None)).await;
                return false;
            }
            Reaction::Nothing => {}
        }
    }
    true
}

/// Serializes and sends one server message. Returns `false` when the write
/// failed and the loop should end the connection.
async fn send_message(socket: &mut WebSocket, message: &ServerMessage) -> bool {
    match serde_json::to_string(message) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(err) => {
            eprintln!("[ocs_collab_server] 序列化出站消息失败：{err}");
            true
        }
    }
}

/// Whether a room message should reach `own_session_id`'s socket. A client
/// never sees its own join/leave — it already knows — while everything else
/// (`op_applied`, `claim_released`, errors) reaches the originator too.
pub fn should_forward(own_session_id: &str, message: &ServerMessage) -> bool {
    match message {
        ServerMessage::PeerJoined(peer) | ServerMessage::PeerLeft(peer) => {
            peer.session_id != own_session_id
        }
        _ => true,
    }
}

/// What `own_session_id`'s room was told and its socket has not been sent
/// yet, as [`should_forward`] lets it through. Sent ahead of what is said to
/// that session alone (`OFF-4`): an answer to an op sent again names the
/// revision it took effect at, and a copy that heard it before the
/// revisions in front of it would take them for a gap.
pub fn earlier_in_the_room(
    subscription: &mut Subscription,
    own_session_id: &str,
) -> Vec<ServerMessage> {
    std::iter::from_fn(|| subscription.try_recv())
        .filter(|message| should_forward(own_session_id, message))
        .collect()
}

/// Applies one client message to the shared state and returns what the socket
/// layer must do next. `now` is unix seconds; callers pass [`now_unix`] so the
/// function itself stays deterministic.
pub async fn handle_message(
    state: &AppState,
    conn: &mut Connection,
    message: ClientMessage,
    now: i64,
) -> Vec<Reaction> {
    if let ClientMessage::Hello {
        user_id,
        drawing_id,
        mode,
        workspace,
        since,
        existing_tags,
        repo_id,
        ..
    } = &message
    {
        if state.store_unavailable {
            println!(
                "[ocs_collab_server] 拒绝 hello user={user_id} drawing={drawing_id}：库打不开，不接受会话"
            );
            return vec![
                Reaction::Send(error_message(
                    codes::STORE_UNAVAILABLE,
                    "服务器的库打不开，不接受会话：请管理员检查服务器的库文件后重启服务器",
                )),
                Reaction::Close,
            ];
        }
        if conn.session.is_some() {
            return vec![Reaction::Send(error_message(
                codes::INVALID_MESSAGE,
                "会话已完成 hello，重复的 hello 被拒绝",
            ))];
        }
        return hello(
            state,
            conn,
            user_id,
            drawing_id,
            mode,
            workspace,
            repo_id.as_deref(),
            *since,
            existing_tags,
            now,
        )
        .await;
    }

    let Some(session) = conn.session.clone() else {
        return vec![Reaction::Send(error_message(
            codes::HELLO_REQUIRED,
            "请先发送 hello 建立会话",
        ))];
    };

    // The wire's session_id is advisory only: the authoritative binding is the
    // handshaken session on this connection, so one client cannot act as
    // another by guessing its session id.
    if message_session_id(&message) != session.session_id {
        return vec![Reaction::Send(error_message(
            codes::SESSION_UNKNOWN,
            "session_id 与本连接的会话不一致",
        ))];
    }

    match message {
        ClientMessage::Heartbeat { .. } => heartbeat(state, &session, now).await,
        ClientMessage::Claim { tag, .. } => claim(state, &session, tag, now).await,
        ClientMessage::Release { tag, .. } => release(state, &session, tag).await,
        ClientMessage::Op { op_id, .. } if conn.not_aligned => not_aligned(&session, op_id),
        ClientMessage::Op {
            base_revision,
            op_id,
            payload,
            ..
        } => op(state, &session, &conn.peer_addr, base_revision, op_id, payload, now).await,
        ClientMessage::Attempt {
            object_ref, text, ..
        } => attempt(state, &session, &conn.peer_addr, &object_ref, &text, now).await,
        ClientMessage::Hello { .. } => unreachable!("hello is handled before the session check"),
    }
}

/// Answers an `op` from a connection whose `welcome` refused its alignment.
/// The op is not judged, logged or told to the room; its sender keeps it
/// for a connection that aligns.
fn not_aligned(session: &Session, op_id: String) -> Vec<Reaction> {
    println!(
        "[ocs_collab_server] 入站 op session={} user={} op_id={op_id} -> NOT_ALIGNED（这次连接没有对上房间，不裁决、不入日志、不广播）",
        session.session_id, session.user_id
    );
    vec![Reaction::Send(ServerMessage::Error {
        code: codes::NOT_ALIGNED.to_owned(),
        message: "这次连接与房间对不上：这一步没有提交，需要取得最新图纸".to_owned(),
        level: None,
        holder: None,
        op_id: Some(op_id),
        rejected: None,
    })]
}

/// What a session that was at revision `since` of `drawing_id` missed, when
/// the server can say all of it: the op of every revision from `since + 1`
/// to `current`, in order, as the op log has them. `None` when it cannot —
/// the session did not ask, it names a revision the drawing has not reached
/// (it was on another database), there is no op log, or the log does not
/// hold every revision in between. A session whose `hello` named its
/// repository is then refused (`REPLAY_GAP`); one that named none is
/// welcomed with no replay, to find out for itself that it is behind.
fn missed_ops(
    state: &AppState,
    drawing_id: &str,
    since: Option<i64>,
    current: i64,
) -> Option<Vec<crate::store::StoredOp>> {
    let since = since?;
    if since < 0 || since > current {
        return None;
    }
    if since == current {
        return Some(Vec::new());
    }
    let ops = state.store.as_ref()?.load_ops(drawing_id, since).ok()?;
    let whole = ops.len() as i64 == current - since
        && ops
            .iter()
            .zip(since + 1..)
            .all(|(op, revision)| op.revision == revision);
    whole.then_some(ops)
}

/// Opens a session for a `hello`: ensures the drawing row, applies the
/// takeover rules and answers `welcome` + `peer_joined`. A session that
/// says where its drawing was (`since`) is sent, right behind the
/// `welcome`, every op the room applied after that (`VAL-CROSS-008`). One
/// that also names its repository (`repo_id`) is checked against the room,
/// and the `welcome` says when it is not taken as consistent. One that
/// lists the tags the objects of its own copy carry (`existing_tags`) has
/// them registered, so that no symbol placed by anybody is given one of
/// them.
#[allow(clippy::too_many_arguments)]
async fn hello(
    state: &AppState,
    conn: &mut Connection,
    user_id: &str,
    drawing_id: &str,
    mode: &str,
    workspace: &str,
    repo_id: Option<&str>,
    since: Option<i64>,
    existing_tags: &[String],
    now: i64,
) -> Vec<Reaction> {
    if let Some(store) = &state.store {
        // A failed ensure must not block the handshake (appendix A.1); only
        // the audit writer truly needs the row.
        if let Err(err) = store.ensure_drawing(drawing_id) {
            eprintln!("[ocs_collab_server] ensure_drawing({drawing_id}) 失败：{err}");
        }
    }
    // Join the room and read its revision as one step, with the drawing
    // locked. An op commits and is announced under that lock, so every op is
    // either inside the revision read here or reaches this subscription.
    // Reading first and subscribing when the reactions are applied left a
    // gap: an op that took effect in between was in neither, and the session
    // started one change short with nothing to tell it so.
    //
    // What a returning session missed is read in the same step, for the
    // same reason: every op is then in the replay, or reaches the
    // subscription, and none is in both.
    //
    // The tags of the client's own copy are registered in that step as
    // well, whether or not the session is then let in: they are judged
    // against the drawing as it is at the revision read here, and a symbol
    // placed right behind this `hello` already finds them taken.
    // A `hello` that names a repository claims to be a copy that was here
    // before (`REQ-COLL-015`). Three ways that claim cannot hold: the
    // repository is another, the revision is ahead of what the room has
    // reached, or the op log no longer holds every revision the copy would
    // need replayed. The session is still opened — the person sees the
    // room — but the copy is not taken as consistent: the `welcome` says
    // so, and its `existing_tags` are not registered (a diverged copy's
    // tags protect nobody). A `hello` that names only a revision claims
    // nothing: it is replayed from when the log allows and welcomed with no
    // replay when it does not, as a client from before the check expects.
    let (joined, base_revision, missed, refused, registered) = {
        let mut drawings = state.drawings.lock().expect("drawings mutex poisoned");
        let joined = state.bus.subscribe(drawing_id);
        let base_revision = current_revision(state, drawing_id);
        let claims_align = repo_id.is_some();
        let mut refused = None;
        if claims_align {
            if repo_id != Some(state.repo_id.as_str()) {
                refused = Some((
                    codes::REPO_MISMATCH,
                    "协同库标识不一致：这份文件对齐的是另一个服务器库".to_owned(),
                ));
            } else if let Some(s) = since {
                if s < 0 || s > base_revision {
                    refused = Some((
                        codes::REVISION_AHEAD,
                        format!("文件记的应用修订 {s} 比服务器当前修订 {base_revision} 新"),
                    ));
                }
            }
        }
        let missed = if refused.is_some() {
            None
        } else {
            missed_ops(state, drawing_id, since, base_revision)
        };
        if claims_align && refused.is_none() && since.is_some() && missed.is_none() {
            refused = Some((
                codes::REPLAY_GAP,
                "操作日志不完整，无法补发该修订之后的全部提交".to_owned(),
            ));
        }
        // A copy that is sent a replay is at `since` until it has applied
        // it: what the replay deletes or retags, it lists and will not keep.
        let seen = missed.as_ref().and(since).unwrap_or(base_revision);
        let registered = if refused.is_some() {
            0
        } else {
            crate::ops::report_tags(
                state,
                &mut drawings,
                drawing_id,
                existing_tags,
                seen,
                base_revision,
            )
        };
        (joined, base_revision, missed, refused, registered)
    };
    if !existing_tags.is_empty() {
        println!(
            "[ocs_collab_server] 入站 hello user={user_id} drawing={drawing_id} 上报本机已有位号 {} 个 -> 新登记 {registered} 个（图面修订 {base_revision}）",
            existing_tags.len()
        );
    }
    let session_id = new_session_id();
    // A user whose role does not edit drawings joins as a viewer, whatever
    // mode the client asked for. The session then counts as no editor
    // anywhere: it takes no 单人编辑 slot, and is never the one left
    // holding the edit right of a drawing that goes 只读.
    let mode = if crate::access::role_edits(state, user_id) == Some(false) {
        "只读"
    } else {
        mode
    };
    let outcome = {
        let mut sessions = state.sessions.lock().await;
        sessions.open(OpenRequest {
            session_id: &session_id,
            drawing_id,
            user_id,
            display_name: user_id,
            mode,
            workspace,
            base_revision,
            now,
        })
    };

    match outcome {
        OpenOutcome::Rejected { code, holder } => {
            println!(
                "[ocs_collab_server] 入站 hello user={user_id} drawing={drawing_id} mode={mode} -> 拒绝 code={code} holder={holder} at={now}"
            );
            vec![
                Reaction::Send(ServerMessage::Error {
                    code,
                    message: "该图面已有活动的编辑会话".to_owned(),
                    level: Some("state".to_owned()),
                    holder: Some(holder),
                    op_id: None,
                    rejected: None,
                }),
                Reaction::Close,
            ]
        }
        OpenOutcome::Opened {
            session,
            taken_over,
        } => {
            let mut reactions = Vec::new();
            for old_session_id in &taken_over {
                let released = state
                    .occupancy
                    .lock()
                    .await
                    .release_session(old_session_id);
                let old_session = state.sessions.lock().await.get(old_session_id).cloned();
                for occupancy in released {
                    if let Some(store) = &state.store {
                        if let Err(err) = store.delete_claim(&occupancy.tag) {
                            eprintln!("[ocs_collab_server] 清理被接管会话的占用失败：{err}");
                        }
                    }
                    reactions.push(Reaction::Publish {
                        drawing_id: occupancy.drawing_id.clone(),
                        message: ServerMessage::ClaimReleased {
                            tag: occupancy.tag,
                            reason: release_reasons::EXPIRED.to_owned(),
                        },
                    });
                }
                if let (Some(store), Some(old)) = (&state.store, &old_session) {
                    if let Err(err) = store.upsert_session(&session_row(old)) {
                        eprintln!("[ocs_collab_server] 更新被接管会话失败：{err}");
                    }
                }
                // The displaced connection learns it was taken over; actually
                // dropping that socket is driven by its own heartbeat failure.
                // Unicast on purpose: a third peer on the same drawing is not
                // part of the takeover and must not see this error.
                reactions.push(Reaction::SendTo {
                    session_id: old_session_id.clone(),
                    message: ServerMessage::Error {
                        code: codes::SESSION_TAKEN_OVER.to_owned(),
                        message: "会话已被新连接接管".to_owned(),
                        level: None,
                        holder: None,
                        op_id: None,
                        rejected: None,
                    },
                });
            }

            if let Some(store) = &state.store {
                if let Err(err) = store.upsert_session(&session_row(&session)) {
                    eprintln!("[ocs_collab_server] 写入会话失败：{err}");
                }
            }

            // The others on the drawing. The session being welcomed is not
            // its own peer: the first one in finds the list empty, the
            // second finds the first (`VAL-CLI-013`, `VAL-CROSS-006`,
            // `architecture.md` A.5).
            let peers: Vec<Peer> = {
                let sessions = state.sessions.lock().await;
                sessions
                    .active_peers(drawing_id)
                    .iter()
                    .filter(|peer| peer.session_id != session.session_id)
                    .map(peer_of)
                    .collect()
            };
            let (claims, expired) = {
                let mut occupancy = state.occupancy.lock().await;
                occupancy.for_drawing(drawing_id, now)
            };
            // A listing request may be the first thing to notice an expired
            // claim; the room still owes a `claim_released` for it, exactly as
            // `sweep_expired` would have sent.
            reactions.extend(expired_release_reactions(state, expired));
            let claims: Vec<ClaimInfo> = claims.iter().map(claim_info_of).collect();
            println!(
                "[ocs_collab_server] 入站 hello user={user_id} drawing={drawing_id} mode={mode} -> welcome session={session_id} base_revision={base_revision} peers={} claims={} at={now}",
                peers.len(),
                claims.len()
            );
            if let Some(since) = since {
                println!(
                    "[ocs_collab_server] hello session={session_id} user={user_id} since={since} -> {}",
                    match &missed {
                        Some(ops) => format!("补发 {} 条 op_applied（修订 {since} 之后到 {base_revision}）", ops.len()),
                        None => format!("不补发（当前修订 {base_revision}，日志里没有完整的一段）"),
                    }
                );
            }

            // The session stands either way; what a refused alignment takes
            // back is the "consistent" part: nothing is replayed, and every
            // `op` this connection sends is answered `NOT_ALIGNED`
            // (`REQ-COLL-015`).
            if let Some((code, message)) = &refused {
                println!(
                    "[ocs_collab_server] hello session={} user={user_id} 对账被拒 code={code}: {message}",
                    session.session_id
                );
            }
            conn.session = Some(session.clone());
            conn.joined = Some(joined);
            conn.not_aligned = refused.is_some();
            reactions.extend([
                Reaction::Subscribe {
                    drawing_id: drawing_id.to_owned(),
                },
                Reaction::Send(ServerMessage::Welcome {
                    session_id,
                    base_revision,
                    peers,
                    claims,
                    repo_id: state.repo_id.clone(),
                    replay_from: missed.as_ref().and(since),
                    align_refused: refused.map(|(code, message)| AlignRefusal {
                        code: code.to_owned(),
                        message,
                    }),
                }),
            ]);
            // What the session missed, each op as the room first heard it,
            // before anything else reaches this socket.
            reactions.extend(missed.unwrap_or_default().into_iter().map(|op| {
                Reaction::Send(ServerMessage::OpApplied {
                    op_id: op.op_id,
                    revision: op.revision,
                    changes: op.changes,
                })
            }));
            reactions.push(Reaction::Publish {
                drawing_id: drawing_id.to_owned(),
                message: ServerMessage::PeerJoined(peer_of(&session)),
            });
            // A session that may not write is told so as it comes in: its
            // user's role does not edit, or the drawing is 只读 already (a
            // downgrade used to reach only the sessions that were there
            // when the state changed).
            if let Some(denied) = crate::access::write_access(state, &session).await {
                println!(
                    "[ocs_collab_server] 单播 readonly_downgrade session={} user={user_id} level={} holder={} reason={} (hello)",
                    session.session_id,
                    denied.level.as_str(),
                    denied.holder.as_deref().unwrap_or("(无)"),
                    denied.reason
                );
                reactions.push(Reaction::Send(denied.downgrade()));
            }
            reactions
        }
    }
}

/// Keeps the session alive and mirrors the fresh heartbeat into SQLite.
async fn heartbeat(state: &AppState, session: &Session, now: i64) -> Vec<Reaction> {
    let beat = {
        let mut sessions = state.sessions.lock().await;
        sessions.heartbeat(&session.session_id, now).cloned()
    };
    match beat {
        Ok(fresh) => {
            if let Some(store) = &state.store {
                if let Err(err) = store.upsert_session(&session_row(&fresh)) {
                    eprintln!("[ocs_collab_server] 心跳写库失败：{err}");
                }
            }
            // One line per inbound beat: the heartbeat cadence is a contract
            // (15 s, VAL-CLI-002) and the timestamp sequence is its evidence.
            println!(
                "[ocs_collab_server] 入站 heartbeat session={} at={now}",
                session.session_id
            );
            vec![Reaction::Nothing]
        }
        // The connection stays open: the client decides whether to re-hello.
        Err(code) => vec![Reaction::Send(error_message(&code, "心跳被会话状态拒绝"))],
    }
}

/// One `claim` request against the occupancy table.
async fn claim(state: &AppState, session: &Session, tag: String, now: i64) -> Vec<Reaction> {
    if let Some(code) = session_unavailable(state, &session.session_id).await {
        return vec![Reaction::Send(error_message(&code, "会话已失效，无法占用位号"))];
    }
    let ttl = state.session_config.claim_ttl;
    let (outcome, expired) = state.occupancy.lock().await.claim(
        &tag,
        &session.drawing_id,
        &session.session_id,
        &session.user_id,
        now,
        ttl,
    );
    // Serving this request reclaimed whatever had already timed out; announce
    // each of them so a claim request cannot silently swallow an expiry that
    // the housekeeping sweep would have broadcast.
    let mut reactions = expired_release_reactions(state, expired);
    match outcome {
        ClaimOutcome::Granted(occupancy) => {
            if let Some(store) = &state.store {
                if let Err(err) = store.insert_claim(&ClaimRow {
                    tag: occupancy.tag.clone(),
                    drawing_id: occupancy.drawing_id.clone(),
                    session_id: occupancy.session_id.clone(),
                    user_id: occupancy.user_id.clone(),
                    expires_at: occupancy.expires_at,
                }) {
                    eprintln!("[ocs_collab_server] 写入占用失败：{err}");
                }
            }
            println!(
                "[ocs_collab_server] 入站 claim session={} user={} tag={tag} -> claim_granted expires_at={} at={now}",
                session.session_id, session.user_id, occupancy.expires_at
            );
            reactions.push(Reaction::Send(ServerMessage::ClaimGranted {
                tag,
                session_id: session.session_id.clone(),
            }));
            reactions
        }
        ClaimOutcome::Denied { holder, expires_at } => {
            println!(
                "[ocs_collab_server] 入站 claim session={} user={} tag={tag} -> claim_denied holder={holder} expires_at={expires_at} at={now}",
                session.session_id, session.user_id
            );
            reactions.push(Reaction::Send(ServerMessage::ClaimDenied {
                tag,
                holder,
                expires_at,
            }));
            reactions
        }
    }
}

/// Turns expired occupancy rows into the `claim_released` publishes the room
/// owes them, mirroring the store first. Shared by every path that can notice
/// an expiry (`claim`, `welcome`'s listing) so the reason string and the mirror
/// write cannot drift apart.
fn expired_release_reactions(state: &AppState, expired: Vec<Occupancy>) -> Vec<Reaction> {
    let mut reactions = Vec::with_capacity(expired.len());
    for occupancy in expired {
        if let Some(store) = &state.store {
            if let Err(err) = store.delete_claim(&occupancy.tag) {
                eprintln!("[ocs_collab_server] 清理过期占用失败：{err}");
            }
        }
        log_claim_expired(&occupancy);
        reactions.push(Reaction::Publish {
            drawing_id: occupancy.drawing_id.clone(),
            message: ServerMessage::ClaimReleased {
                tag: occupancy.tag,
                reason: release_reasons::EXPIRED.to_owned(),
            },
        });
    }
    reactions
}

/// The line a claim's TTL running out leaves in the log, whichever path
/// noticed it (`VAL-CLI-006`).
fn log_claim_expired(occupancy: &Occupancy) {
    println!(
        "[ocs_collab_server] 占用到期回收 tag={} drawing={} session={} user={} expires_at={} -> 广播 claim_released reason={} at={}",
        occupancy.tag,
        occupancy.drawing_id,
        occupancy.session_id,
        occupancy.user_id,
        occupancy.expires_at,
        release_reasons::EXPIRED,
        now_unix()
    );
}

/// One voluntary `release`; broadcast so the originator sees it too
/// (VAL-SRV-012).
async fn release(state: &AppState, session: &Session, tag: String) -> Vec<Reaction> {
    if let Some(code) = session_unavailable(state, &session.session_id).await {
        return vec![Reaction::Send(error_message(&code, "会话已失效，无法释放位号"))];
    }
    match state.occupancy.lock().await.release(&tag, &session.session_id) {
        ReleaseOutcome::Released => {
            if let Some(store) = &state.store {
                if let Err(err) = store.delete_claim(&tag) {
                    eprintln!("[ocs_collab_server] 删除占用失败：{err}");
                }
            }
            println!(
                "[ocs_collab_server] 入站 release session={} user={} tag={tag} -> 广播 claim_released reason={} at={}",
                session.session_id,
                session.user_id,
                release_reasons::RELEASED,
                now_unix()
            );
            vec![Reaction::Publish {
                drawing_id: session.drawing_id.clone(),
                message: ServerMessage::ClaimReleased {
                    tag,
                    reason: release_reasons::RELEASED.to_owned(),
                },
            }]
        }
        ReleaseOutcome::NotHeld => vec![Reaction::Send(error_message(
            codes::INVALID_MESSAGE,
            "该会话未持有此位号",
        ))],
    }
}

/// One `op`: revision-checked, persisted, audited, then broadcast to the whole
/// room including the sender (VAL-SRV-016). Conflict detection arrives in M7;
/// this version applies every well-formed op.
///
/// The check → increment → audit insert is a single critical section inside
/// [`Store::commit_op`] (or, without a database, inside the `revisions` mutex).
/// Doing it step by step let two concurrent ops both pass the check at the same
/// revision and both increment, so the room saw two `op_applied` frames sharing
/// one revision while `drawings.revision` advanced once.
async fn op(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    base_revision: i64,
    op_id: String,
    payload: Value,
    now: i64,
) -> Vec<Reaction> {
    if let Some(code) = session_unavailable(state, &session.session_id).await {
        return vec![Reaction::Send(refusing(
            error_message(&code, "会话已失效，无法提交操作"),
            &op_id,
        ))];
    }

    // An op the drawing has had before is answered with what came of it
    // (`OFF-4`), before what may be written and by whom is asked: it was
    // allowed when it came, and a refusal now would have its sender take
    // back a step the room has.
    if let Some(outcome) = crate::ops::heard_before(
        state,
        session,
        peer_addr,
        base_revision,
        &op_id,
        &payload,
        now,
    ) {
        return outcome.reactions;
    }

    // A session that may not write does not write, whatever its client
    // believes: the client's own gate is a courtesy, and is not there at all
    // in a client that reconnected before it heard, or is another program.
    if let Some(denied) = crate::access::write_access(state, session).await {
        let first = op_changes(&payload).into_iter().next().unwrap_or(Value::Null);
        let kind = first["kind"].as_str().unwrap_or("op").to_owned();
        let object_ref = first["object_ref"].as_str().unwrap_or_default().to_owned();
        let tried = describe_change(&kind, &object_ref, &first);
        record_denied_write(state, session, peer_addr, &object_ref, &tried, &denied, now);
        println!(
            "[ocs_collab_server] 入站 op session={} user={} op_id={op_id} base_revision={base_revision} object_ref={object_ref} -> error {} level={} at={now}",
            session.session_id,
            session.user_id,
            denied.code(),
            denied.level.as_str()
        );
        return vec![Reaction::Send(refusing(
            denied.error(&format!("操作 {op_id} ")),
            &op_id,
        ))];
    }

    // A tag somebody else holds is theirs to change until they let go of it
    // (`VAL-CROSS-002`). One such change refuses the op whole, like every
    // other refusal: an op is one step of somebody's, and half a step is
    // not something they made.
    if let Some((change, held)) = held_by_another(state, session, &payload, now).await {
        let tried = describe_change(&change.kind, &change.subject(), &change.raw);
        let text = format!(
            "{tried} 被拒绝（位号 {} 由 {} 占用，到期 {}）",
            held.tag, held.user_id, held.expires_at
        );
        record_refused_write(state, session, peer_addr, &held.tag, text, now);
        println!(
            "[ocs_collab_server] 入站 op session={} user={} op_id={op_id} base_revision={base_revision} object_ref={} -> error {} tag={} holder={} at={now}",
            session.session_id,
            session.user_id,
            change.subject(),
            codes::CLAIM_HELD,
            held.tag,
            held.session_id
        );
        return vec![Reaction::Send(ServerMessage::Error {
            code: codes::CLAIM_HELD.to_owned(),
            message: format!(
                "操作 {op_id}：位号 {} 已被 {} 占用，到期 {}",
                held.tag, held.user_id, held.expires_at
            ),
            level: None,
            holder: Some(held.session_id),
            op_id: Some(op_id),
            rejected: Some(true),
        })];
    }

    // The judgment, the commit and the record of it are one synchronous
    // critical section inside `ops::receive`.
    let outcome = crate::ops::receive(
        state,
        session,
        peer_addr,
        base_revision,
        op_id,
        payload,
        now,
    );

    // The drawing's live sessions share its baseline, and the broadcast is
    // what moves them: every one of them is at `revision` from here on
    // (`VAL-CLI-010`). The commit moved the table rows inside its own
    // transaction; this moves the registry those rows are rewritten from, so
    // the next heartbeat does not put the old baseline back.
    if let Some(revision) = outcome.revision {
        state
            .sessions
            .lock()
            .await
            .advance_base_revision(&session.drawing_id, revision);
    }
    // An op that made a connector, or wrote a field of a line that flows
    // out along one, was followed by ops of the server's own — on this
    // drawing, on the drawing at the other end — each told to its room
    // already. The sessions of those drawings are at those revisions now.
    for (drawing_id, revision) in crate::ops::revisions_reached(&outcome.reactions) {
        state
            .sessions
            .lock()
            .await
            .advance_base_revision(&drawing_id, revision);
    }
    outcome.reactions
}

/// The first change of `payload` that touches a tag another session holds
/// on this drawing, with the claim.
///
/// A change touches the tag of the object it is about, and a retag also the
/// tag it would give: taking a held tag for another object is as much the
/// holder's business as changing the object that has it. A change to how a
/// pipe line is connected up touches the line and the run it names as well
/// as whatever it says it creates.
async fn held_by_another(
    state: &AppState,
    session: &Session,
    payload: &Value,
    now: i64,
) -> Option<(ocs_collab::conflict::Change, Occupancy)> {
    let changes = ocs_collab::conflict::Change::list(payload);
    let occupancy = state.occupancy.lock().await;
    changes.into_iter().find_map(|change| {
        let mut tags = vec![change.subject()];
        if change.kind == ocs_collab::conflict::KIND_SET_TAG {
            tags.push(change.value.clone());
        }
        // A change to how a pipe line is connected up is a change to the
        // line and to the run it names: a claim on either stands against it.
        if change.changes_topology() {
            tags.extend(change.text("line_tag"));
            tags.extend(change.text("run_tag"));
        }
        tags.iter()
            .filter(|tag| !tag.is_empty())
            .find_map(|tag| {
                occupancy
                    .held_by_another(tag, &session.drawing_id, &session.session_id, now)
                    .cloned()
            })
            .map(|held| (change, held))
    })
}

/// `audit_log.action` of a write that was refused because the session may
/// not write.
pub const ACTION_WRITE_DENIED: &str = "write_denied";

/// Puts a refused write on record: who tried what on which object, and why
/// it did not happen (`VAL-CROSS-004`, `-011`).
fn record_denied_write(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    object_ref: &str,
    tried: &str,
    denied: &crate::access::Denied,
    now: i64,
) {
    let text = format!(
        "{tried} 被拒绝（只读，{}）：{}",
        denied.level.as_str(),
        denied.reason
    );
    record_refused_write(state, session, peer_addr, object_ref, text, now);
}

/// Puts a refused write on record under `text`, which says what was tried
/// and why it did not happen.
fn record_refused_write(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    object_ref: &str,
    text: String,
    now: i64,
) {
    let Some(store) = &state.store else {
        return;
    };
    let row = audit_row(
        Some(store),
        session,
        ACTION_WRITE_DENIED,
        object_ref,
        text,
        peer_addr,
        now,
    );
    if let Err(err) = store.audit_append(&row) {
        eprintln!("[ocs_collab_server] 写入拒绝审计失败：{err}");
    }
}

/// One `attempt`: a client reporting a write it turned away itself. The
/// drawing is untouched; the record is kept. The reason on record is the
/// server's own reading of the session's access, not the client's word.
async fn attempt(
    state: &AppState,
    session: &Session,
    peer_addr: &str,
    object_ref: &str,
    text: &str,
    now: i64,
) -> Vec<Reaction> {
    if let Some(code) = session_unavailable(state, &session.session_id).await {
        return vec![Reaction::Send(error_message(&code, "会话已失效"))];
    }
    let tried = if text.trim().is_empty() {
        "(未说明的写操作)"
    } else {
        text.trim()
    };
    let denied = crate::access::write_access(state, session).await;
    // A session that may write turns a write away for one other reason: the
    // tag is somebody else's for now.
    let held = match &denied {
        Some(_) => None,
        None => state
            .occupancy
            .lock()
            .await
            .held_by_another(object_ref, &session.drawing_id, &session.session_id, now)
            .cloned(),
    };
    let why = match (&denied, &held) {
        (Some(denied), _) => denied.level.as_str().to_owned(),
        (None, Some(held)) => format!("claim holder={}", held.session_id),
        (None, None) => "state".to_owned(),
    };
    match (denied, held) {
        (Some(denied), _) => {
            record_denied_write(state, session, peer_addr, object_ref, tried, &denied, now)
        }
        (None, Some(held)) => record_refused_write(
            state,
            session,
            peer_addr,
            object_ref,
            format!(
                "{tried} 被拒绝（位号 {} 由 {} 占用，到期 {}）",
                held.tag, held.user_id, held.expires_at
            ),
            now,
        ),
        (None, None) => record_denied_write(
            state,
            session,
            peer_addr,
            object_ref,
            tried,
            &crate::access::Denied {
                // The server would have let it through; the client refused
                // on what it had been told. Said as it is.
                level: ocs_collab::DenialLevel::State,
                reason: "客户端处于只读，自行拒绝（服务器此刻判定该会话可写）".to_owned(),
                holder: None,
            },
            now,
        ),
    }
    println!(
        "[ocs_collab_server] 入站 attempt session={} user={} object_ref={object_ref} level={why} -> 记审计 {ACTION_WRITE_DENIED} at={now}",
        session.session_id, session.user_id
    );
    vec![Reaction::Nothing]
}

/// One housekeeping pass: mark timed-out sessions stale and expire claims.
/// Returns the publishes the caller must fan out; mutations of the shared
/// state and the store mirror happen inside.
pub async fn sweep_expired(state: &AppState, now: i64) -> Vec<Reaction> {
    let stale = state.sessions.lock().await.sweep(now);
    let mut reactions = Vec::new();

    for session_id in &stale {
        let Some(session) = ({
            let sessions = state.sessions.lock().await;
            sessions.get(session_id).cloned()
        }) else {
            continue;
        };
        if let Some(store) = &state.store {
            if let Err(err) = store.upsert_session(&session_row(&session)) {
                eprintln!("[ocs_collab_server] 写入失效会话失败：{err}");
            }
        }
        let released = state.occupancy.lock().await.release_session(session_id);
        for occupancy in released {
            if let Some(store) = &state.store {
                if let Err(err) = store.delete_claim(&occupancy.tag) {
                    eprintln!("[ocs_collab_server] 清理失效会话的占用失败：{err}");
                }
            }
            reactions.push(Reaction::Publish {
                drawing_id: occupancy.drawing_id.clone(),
                message: ServerMessage::ClaimReleased {
                    tag: occupancy.tag,
                    reason: release_reasons::EXPIRED.to_owned(),
                },
            });
        }
        reactions.push(Reaction::Publish {
            drawing_id: session.drawing_id.clone(),
            message: ServerMessage::PeerLeft(peer_of(&session)),
        });
        println!(
            "[ocs_collab_server] 广播 peer_left session={} user={} drawing={} reason=heartbeat-timeout",
            session.session_id, session.user_id, session.drawing_id
        );
    }

    let expired = state.occupancy.lock().await.expire(now);
    for occupancy in expired {
        if let Some(store) = &state.store {
            if let Err(err) = store.delete_claim(&occupancy.tag) {
                eprintln!("[ocs_collab_server] 清理过期占用失败：{err}");
            }
        }
        log_claim_expired(&occupancy);
        reactions.push(Reaction::Publish {
            drawing_id: occupancy.drawing_id.clone(),
            message: ServerMessage::ClaimReleased {
                tag: occupancy.tag,
                reason: release_reasons::EXPIRED.to_owned(),
            },
        });
    }

    reactions
}

/// `Some(code)` when the session may no longer read or write.
async fn session_unavailable(state: &AppState, session_id: &str) -> Option<String> {
    let sessions = state.sessions.lock().await;
    match sessions.get(session_id).map(|session| session.status) {
        None => Some(codes::SESSION_UNKNOWN.to_owned()),
        Some(SessionStatus::Active) => None,
        Some(SessionStatus::Stale) => Some(codes::SESSION_EXPIRED.to_owned()),
        Some(SessionStatus::TakenOver) => Some(codes::SESSION_TAKEN_OVER.to_owned()),
    }
}

/// The drawing's committed revision: `drawings.revision` when a database is
/// attached, otherwise the in-memory counter so revision checks still work.
///
/// Only used to stamp `welcome.base_revision`; commits go through
/// [`Store::commit_op`] (or the `revisions` mutex) so they never split the
/// read from the write.
pub(crate) fn current_revision(state: &AppState, drawing_id: &str) -> i64 {
    match &state.store {
        Some(store) => match store.drawing(drawing_id) {
            Ok(Some((_, _, revision))) => revision,
            _ => 0,
        },
        None => state
            .revisions
            .lock()
            .expect("revisions mutex poisoned")
            .get(drawing_id)
            .copied()
            .unwrap_or(0),
    }
}

/// Normalizes an op payload into the `changes` array on `op_applied`: object
/// payloads keep every key and get the canonical fields backfilled with
/// `null` where absent; anything else is wrapped verbatim.
pub(crate) fn op_changes(payload: &Value) -> Vec<Value> {
    match payload {
        Value::Object(map) => {
            let mut change = map.clone();
            for key in ["kind", "object_ref", "field", "value"] {
                change.entry(key.to_owned()).or_insert(Value::Null);
            }
            vec![Value::Object(change)]
        }
        _ => vec![payload.clone()],
    }
}

fn error_message(code: &str, message: &str) -> ServerMessage {
    ServerMessage::Error {
        code: code.to_owned(),
        message: message.to_owned(),
        level: None,
        holder: None,
        op_id: None,
        rejected: None,
    }
}

/// `error`, as the answer to op `op_id` nothing of which took effect: its
/// sender takes the step back (`VAL-CROSS-008`).
fn refusing(error: ServerMessage, op_id: &str) -> ServerMessage {
    match error {
        ServerMessage::Error {
            code,
            message,
            level,
            holder,
            ..
        } => ServerMessage::Error {
            code,
            message,
            level,
            holder,
            op_id: Some(op_id.to_owned()),
            rejected: Some(true),
        },
        other => other,
    }
}

fn invalid_message_error() -> ServerMessage {
    error_message(codes::INVALID_MESSAGE, "消息无法解析或不是合法的客户端消息")
}

/// The `session_id` a client message claims to speak for.
fn message_session_id(message: &ClientMessage) -> &str {
    match message {
        ClientMessage::Heartbeat { session_id }
        | ClientMessage::Claim { session_id, .. }
        | ClientMessage::Release { session_id, .. }
        | ClientMessage::Op { session_id, .. }
        | ClientMessage::Attempt { session_id, .. } => session_id,
        ClientMessage::Hello { .. } => "",
    }
}

fn peer_of(session: &Session) -> Peer {
    Peer {
        session_id: session.session_id.clone(),
        user_id: session.user_id.clone(),
        display_name: session.display_name.clone(),
    }
}

fn claim_info_of(occupancy: &Occupancy) -> ClaimInfo {
    ClaimInfo {
        tag: occupancy.tag.clone(),
        session_id: occupancy.session_id.clone(),
        user_id: occupancy.user_id.clone(),
        expires_at: occupancy.expires_at,
    }
}

fn session_row(session: &Session) -> SessionRow {
    SessionRow {
        session_id: session.session_id.clone(),
        drawing_id: session.drawing_id.clone(),
        user_id: session.user_id.clone(),
        mode: session.mode.clone(),
        status: session.status.as_str().to_owned(),
        base_revision: session.base_revision,
        last_heartbeat: session.last_heartbeat,
        created_at: session.created_at,
        workspace: session.workspace.clone(),
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// A readable one-liner for the audit `change_text` column. Always non-empty:
/// `kind` and `object_ref` are already defaulted by the caller.
pub(crate) fn describe_change(kind: &str, object_ref: &str, payload: &Value) -> String {
    let field = payload
        .get("field")
        .and_then(Value::as_str)
        .filter(|field| !field.is_empty());
    let old = payload
        .get("old_value")
        .map(value_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(未知)".to_owned());
    let new = payload
        .get("value")
        .map(value_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(未知)".to_owned());
    match field {
        Some(field) => format!("{kind} {object_ref}.{field}: {old} -> {new}"),
        None => format!("{kind} {object_ref}"),
    }
}

/// Builds the audit row for one applied op. VAL-SRV-020 requires all nine
/// fields non-empty, so every source has a fallback.
pub(crate) fn op_audit_row(
    store: &Store,
    session: &Session,
    payload: &Value,
    op_id: &str,
    peer_addr: &str,
    now: i64,
) -> NewAuditRow {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("op");
    let object_ref = payload
        .get("object_ref")
        .and_then(Value::as_str)
        .filter(|object_ref| !object_ref.is_empty())
        .or(if op_id.is_empty() {
            None
        } else {
            Some(op_id)
        })
        .unwrap_or("未知对象");
    audit_row(
        Some(store),
        session,
        kind,
        object_ref,
        describe_change(kind, object_ref, payload),
        peer_addr,
        now,
    )
}

/// One audit row for something `session` did to `object_ref`. Every column
/// has a fallback, so all nine are non-empty whatever the session carries
/// (`VAL-SRV-020`).
pub(crate) fn audit_row(
    store: Option<&Store>,
    session: &Session,
    action: &str,
    object_ref: &str,
    change_text: String,
    peer_addr: &str,
    now: i64,
) -> NewAuditRow {
    let company = store
        .and_then(|store| store.user_role(&session.user_id).ok().flatten())
        .map(|(_, company)| company)
        .filter(|company| !company.is_empty())
        .unwrap_or_else(|| "未知".to_owned());
    let user_id = if session.user_id.is_empty() {
        "未知用户".to_owned()
    } else {
        session.user_id.clone()
    };
    let ip = if peer_addr.is_empty() {
        "127.0.0.1".to_owned()
    } else {
        peer_addr.to_owned()
    };
    let workspace = if session.workspace.is_empty() {
        "默认工作区".to_owned()
    } else {
        session.workspace.clone()
    };
    NewAuditRow {
        module: "协同编辑".to_owned(),
        action: action.to_owned(),
        at: now,
        object_ref: if object_ref.is_empty() {
            "未知对象".to_owned()
        } else {
            object_ref.to_owned()
        },
        user_id,
        company,
        ip,
        workspace,
        change_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AuditQuery;
    use ocs_collab::SessionConfig;
    use serde_json::json;

    const T0: i64 = 1_700_000_000;

    fn state() -> AppState {
        AppState::new(
            Some(Store::open_in_memory().expect("memory store")),
            None,
            ":memory:",
            SessionConfig::default(),
        )
    }

    fn storeless_state() -> AppState {
        AppState::new(
            None,
            Some("no database".into()),
            "missing.db",
            SessionConfig::default(),
        )
    }

    fn store_of(state: &AppState) -> &Store {
        state.store.as_ref().expect("test state has a store")
    }

    fn conn() -> Connection {
        Connection::new("10.0.0.7")
    }

    fn hello(user_id: &str, mode: &str, workspace: &str) -> ClientMessage {
        ClientMessage::Hello {
            user_id: user_id.into(),
            drawing_id: "PID-1101".into(),
            mode: mode.into(),
            client_version: "test".into(),
            workspace: workspace.into(),
            since: None,
            existing_tags: Vec::new(),
            repo_id: None,
        }
    }

    /// [`hello`], from a client whose copy was aligned against this
    /// repository before: it claims `state.repo_id` and the revision it was
    /// at (`REQ-COLL-015`).
    fn hello_back(state: &AppState, user_id: &str, since: Option<i64>) -> ClientMessage {
        let mut hello = hello(user_id, "协同编辑", "");
        if let ClientMessage::Hello {
            repo_id, since: s, ..
        } = &mut hello
        {
            *repo_id = Some(state.repo_id.clone());
            *s = since;
        }
        hello
    }

    /// [`hello`], from a client whose own copy of the drawing has `tags`.
    fn hello_with(user_id: &str, mode: &str, tags: &[&str]) -> ClientMessage {
        let mut hello = hello(user_id, mode, "");
        if let ClientMessage::Hello { existing_tags, .. } = &mut hello {
            *existing_tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        }
        hello
    }

    /// What the `welcome` of an answered `hello` said against the copy's
    /// claim to be aligned, `(code, message)`, if it said anything (`OFF-2`:
    /// the refusal rides in the `welcome`).
    fn refused(reactions: &[Reaction]) -> Option<(String, String)> {
        sends(reactions).iter().find_map(|message| {
            let wire = serde_json::to_value(message).ok()?;
            if wire["type"] != "welcome" {
                return None;
            }
            let refusal = wire.get("align_refused")?;
            Some((
                refusal["code"].as_str()?.to_owned(),
                refusal["message"].as_str()?.to_owned(),
            ))
        })
    }

    /// The wire form of every frame `reactions` sends anywhere: to this
    /// connection, to one session, or to the room.
    fn wire_of(reactions: &[Reaction]) -> Vec<serde_json::Value> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Send(message)
                | Reaction::SendTo { message, .. }
                | Reaction::Publish { message, .. }
                | Reaction::Published { message, .. } => serde_json::to_value(message).ok(),
                _ => None,
            })
            .collect()
    }

    fn session_id(conn: &Connection) -> String {
        conn.session
            .as_ref()
            .expect("handshaken")
            .session_id
            .clone()
    }

    fn sends(reactions: &[Reaction]) -> Vec<ServerMessage> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Send(message) => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    /// What the reactions put in front of the room: what is still to be
    /// published, and what was published as it happened.
    fn publishes(reactions: &[Reaction]) -> Vec<ServerMessage> {
        reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::Publish { message, .. } | Reaction::Published { message, .. } => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect()
    }

    async fn opened_conn(state: &AppState, user_id: &str, now: i64) -> Connection {
        let mut conn = conn();
        let reactions =
            handle_message(state, &mut conn, hello(user_id, "协同编辑", "plant-a"), now).await;
        assert!(
            sends(&reactions)
                .iter()
                .any(|message| matches!(message, ServerMessage::Welcome { .. })),
            "hello must yield a welcome: {reactions:?}"
        );
        conn
    }

    #[tokio::test]
    async fn an_explicit_close_retires_the_session_and_tells_the_room_once() {
        // VAL-CLI-016: a client that says goodbye must disappear from the
        // roster at once, not 90 s later, and the row must say 失效.
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let id = session_id(&conn);
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;

        let mut room = state.bus.subscribe("PID-1101");
        let retired = close_session(&state, &conn).await;
        assert!(retired, "a live session must be retired on close");

        let mut seen = Vec::new();
        while let Some(message) = room.try_recv() {
            seen.push(message);
        }
        assert!(
            seen.iter()
                .any(|message| matches!(message, ServerMessage::PeerLeft(peer) if peer.session_id == id)),
            "the room must learn the peer left: {seen:?}"
        );
        assert!(
            seen.iter().any(|message| matches!(message,
                ServerMessage::ClaimReleased { tag, reason } if tag == "V-1001" && reason == "expired")),
            "the retired session's claims must be released: {seen:?}"
        );

        let row = store_of(&state)
            .session_row(&id)
            .expect("lookup")
            .expect("session row");
        assert_eq!(row.status, "失效");
        assert!(state
            .occupancy
            .lock()
            .await
            .for_drawing("PID-1101", T0 + 1)
            .0
            .is_empty());
    }

    #[tokio::test]
    async fn closing_twice_does_not_repeat_peer_left() {
        // The disconnect tail runs after the read loop, so a second retire
        // would shrink the roster twice on one departure.
        let state = state();
        let conn = opened_conn(&state, "u1", T0).await;
        assert!(close_session(&state, &conn).await);
        assert!(
            !close_session(&state, &conn).await,
            "an already stale session must not be retired twice"
        );
    }

    #[tokio::test]
    async fn a_swept_session_is_not_retired_again_on_close() {
        // The heartbeat timeout already broadcast `peer_left`; a late close
        // frame from the same socket must not repeat it.
        let state = state();
        let conn = opened_conn(&state, "u1", T0).await;
        sweep_expired(&state, T0 + 91).await;
        assert!(
            !close_session(&state, &conn).await,
            "a stale session's departure was already announced"
        );
    }

    #[tokio::test]
    async fn a_connection_without_a_session_has_nothing_to_retire() {
        let state = state();
        let conn = conn();
        assert!(!close_session(&state, &conn).await);
    }

    #[tokio::test]
    async fn a_storeless_close_still_retires_and_announces() {
        let state = storeless_state();
        let conn = opened_conn(&state, "u1", T0).await;
        assert!(close_session(&state, &conn).await);
        let sessions = state.sessions.lock().await;
        assert_eq!(
            sessions.get(&session_id(&conn)).map(|s| s.status),
            Some(SessionStatus::Stale)
        );
    }

    #[tokio::test]
    async fn hello_opens_a_session_and_returns_welcome() {
        let state = state();
        let mut conn = conn();
        let reactions =
            handle_message(&state, &mut conn, hello("u1", "协同编辑", "plant-a"), T0).await;
        let session_id = session_id(&conn);
        assert!(
            reactions
                .iter()
                .any(|reaction| matches!(reaction, Reaction::Subscribe { drawing_id } if drawing_id == "PID-1101")),
            "hello must subscribe the room before peer_joined: {reactions:?}"
        );
        match &sends(&reactions)[0] {
            ServerMessage::Welcome {
                session_id: welcomed,
                base_revision,
                peers,
                claims,
                ..
            } => {
                assert_eq!(welcomed, &session_id);
                assert_eq!(*base_revision, 0);
                // The first session on a drawing has no peers; it is not
                // its own (`VAL-CLI-013`: "`welcome.peers` 为空数组").
                assert!(peers.is_empty(), "{peers:?}");
                assert!(claims.is_empty());
            }
            other => panic!("expected welcome, got {other:?}"),
        }
        let published = publishes(&reactions);
        assert!(
            matches!(published.last(), Some(ServerMessage::PeerJoined(peer)) if peer.session_id == session_id),
            "the room must hear about the join: {published:?}"
        );
    }

    #[tokio::test]
    async fn welcome_lists_active_peers_and_outstanding_claims() {
        let state = state();
        let mut a = opened_conn(&state, "u1", T0).await;
        let a_id = session_id(&a);
        handle_message(
            &state,
            &mut a,
            ClientMessage::Claim {
                session_id: a_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;

        let mut b = opened_conn(&state, "u2", T0 + 1).await;
        let b_id = session_id(&b);
        let reactions = handle_message(
            &state,
            &mut b,
            ClientMessage::Heartbeat {
                session_id: b_id.clone(),
            },
            T0 + 2,
        )
        .await;
        assert_eq!(reactions, vec![Reaction::Nothing]);

        // Re-issue hello assertions against B's stored welcome indirectly:
        // the welcome B already received must have listed both peers and A's
        // claim. The registry + table state is the observable truth here.
        let peers = state.sessions.lock().await.active_peers("PID-1101");
        assert_eq!(peers.len(), 2);
        let (claims, _) = state
            .occupancy
            .lock()
            .await
            .for_drawing("PID-1101", T0 + 2);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].tag, "V-1001");
        assert_eq!(claims[0].session_id, a_id);
        assert_eq!(claims[0].user_id, "u1");
    }

    #[tokio::test]
    async fn welcome_payload_contains_peers_and_claims() {
        let state = state();
        let mut a = opened_conn(&state, "u1", T0).await;
        let a_id = session_id(&a);
        handle_message(
            &state,
            &mut a,
            ClientMessage::Claim {
                session_id: a_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;

        let mut b = conn();
        let reactions =
            handle_message(&state, &mut b, hello("u2", "协同编辑", "plant-b"), T0 + 1).await;
        let b_id = session_id(&b);
        match &sends(&reactions)[0] {
            ServerMessage::Welcome { peers, claims, .. } => {
                // The others on the drawing: A, and not B itself.
                assert_eq!(peers.len(), 1, "{peers:?}");
                assert_eq!(peers[0].session_id, a_id);
                assert!(peers.iter().all(|peer| peer.session_id != b_id));
                assert_eq!(claims.len(), 1);
                assert_eq!(claims[0].tag, "V-1001");
                assert_eq!(claims[0].session_id, a_id);
            }
            other => panic!("expected welcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_message_before_hello_is_refused() {
        let state = state();
        let mut conn = conn();
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: "s-whatever".into(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        assert_eq!(
            sends(&reactions),
            vec![error_message(codes::HELLO_REQUIRED, "请先发送 hello 建立会话")],
            "a claim before hello must be refused, not crash"
        );
    }

    #[tokio::test]
    async fn a_repeated_hello_is_an_invalid_message() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let first_session_id = session_id(&conn);
        let reactions =
            handle_message(&state, &mut conn, hello("u1", "协同编辑", "plant-a"), T0 + 1).await;
        assert_eq!(
            sends(&reactions),
            vec![error_message(
                codes::INVALID_MESSAGE,
                "会话已完成 hello，重复的 hello 被拒绝"
            )]
        );
        assert_eq!(session_id(&conn), first_session_id, "the session survives");
    }

    #[tokio::test]
    async fn single_editor_mode_rejects_the_second_session() {
        let state = state();
        let mut a = conn();
        handle_message(&state, &mut a, hello("u1", "单人编辑", ""), T0).await;
        let a_id = session_id(&a);

        let mut b = conn();
        let reactions =
            handle_message(&state, &mut b, hello("u2", "单人编辑", ""), T0 + 1).await;
        assert_eq!(
            reactions,
            vec![
                Reaction::Send(ServerMessage::Error {
                    code: codes::PERM_DENIED_STATE.to_owned(),
                    message: "该图面已有活动的编辑会话".to_owned(),
                    level: Some("state".to_owned()),
                    holder: Some(a_id),
                    op_id: None,
                    rejected: None,
                }),
                Reaction::Close,
            ],
            "a rejected hello must close the socket"
        );
        assert!(b.session.is_none());
    }

    /// `hello.existing_tags` are registered in the step that reads the
    /// drawing's revision, whoever the `hello` is from. A session that only
    /// views has those objects in its copy as much as one that edits; and
    /// so has one that is refused — the drawing has its one editor — and
    /// will be back. A `hello` that lists nothing leaves the drawing unread.
    #[tokio::test]
    async fn collab_existing_tags_are_registered_whoever_the_hello_is_from() {
        let state = state();
        let mut editor = conn();
        handle_message(&state, &mut editor, hello("u1", "单人编辑", ""), T0).await;
        let unread = state.drawings.lock().expect("drawings").is_empty();
        assert!(unread, "nothing was listed");

        let mut viewer = conn();
        let listing = hello_with("u2", "只读", &["P-2001"]);
        handle_message(&state, &mut viewer, listing, T0).await;
        assert!(viewer.session.is_some(), "a viewer is let in");
        let mut refused = conn();
        let listing = hello_with("u3", "单人编辑", &["V-1001", ""]);
        let reactions = handle_message(&state, &mut refused, listing, T0).await;
        assert!(reactions.contains(&Reaction::Close), "{reactions:?}");

        let held = crate::ops::reported_tags(&state, "PID-1101", None);
        assert_eq!(held, ["P-2001", "V-1001"]);
    }

    #[tokio::test]
    async fn a_storeless_state_still_handshakes() {
        let state = storeless_state();
        let mut conn = conn();
        let reactions =
            handle_message(&state, &mut conn, hello("u1", "协同编辑", ""), T0).await;
        match &sends(&reactions)[0] {
            ServerMessage::Welcome { base_revision, .. } => assert_eq!(*base_revision, 0),
            other => panic!("expected welcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn claim_after_hello_is_granted_and_mirrored() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: session_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        assert_eq!(
            sends(&reactions),
            vec![ServerMessage::ClaimGranted {
                tag: "V-1001".into(),
                session_id: session_id.clone(),
            }]
        );
        let rows = store_of(&state)
            .claims_on_drawing("PID-1101")
            .expect("claims query");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, session_id);
        assert_eq!(rows[0].expires_at, T0 + 300);
    }

    #[tokio::test]
    async fn a_second_session_claiming_the_tag_is_denied() {
        let state = state();
        let mut a = opened_conn(&state, "u1", T0).await;
        let a_id = session_id(&a);
        handle_message(
            &state,
            &mut a,
            ClientMessage::Claim {
                session_id: a_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;

        let mut b = opened_conn(&state, "u2", T0 + 1).await;
        let b_id = session_id(&b);
        let reactions = handle_message(
            &state,
            &mut b,
            ClientMessage::Claim {
                session_id: b_id,
                tag: "V-1001".into(),
            },
            T0 + 1,
        )
        .await;
        assert_eq!(
            sends(&reactions),
            vec![ServerMessage::ClaimDenied {
                tag: "V-1001".into(),
                holder: a_id,
                expires_at: T0 + 300,
            }]
        );
    }

    #[tokio::test]
    async fn release_broadcasts_claim_released_to_the_room() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: session_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Release {
                session_id,
                tag: "V-1001".into(),
            },
            T0 + 1,
        )
        .await;
        // VAL-SRV-012: the publisher sees its own claim_released too.
        assert_eq!(
            publishes(&reactions),
            vec![ServerMessage::ClaimReleased {
                tag: "V-1001".into(),
                reason: "released".into(),
            }]
        );
        assert!(store_of(&state)
            .claims_on_drawing("PID-1101")
            .expect("claims query")
            .is_empty());
    }

    #[tokio::test]
    async fn releasing_an_unheld_tag_is_an_error() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let sid = session_id(&conn);
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Release {
                session_id: sid,
                tag: "V-9999".into(),
            },
            T0,
        )
        .await;
        assert_eq!(
            sends(&reactions),
            vec![error_message(codes::INVALID_MESSAGE, "该会话未持有此位号")]
        );
    }

    #[tokio::test]
    async fn a_foreign_session_id_is_rejected() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: "s-not-mine".into(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        assert_eq!(
            sends(&reactions),
            vec![error_message(
                codes::SESSION_UNKNOWN,
                "session_id 与本连接的会话不一致"
            )]
        );
    }

    #[tokio::test]
    async fn ops_advance_the_revision_monotonically() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        let payload = json!({
            "kind": "set_property",
            "object_ref": "V-1001",
            "field": "设计压力",
            "value": "1.6MPa",
        });
        for (index, expected) in [(0, 1_i64), (1, 2_i64)] {
            let reactions = handle_message(
                &state,
                &mut conn,
                ClientMessage::Op {
                    session_id: session_id.clone(),
                    base_revision: index,
                    op_id: format!("op-{index}"),
                    payload: payload.clone(),
                },
                T0 + index,
            )
            .await;
            let published = publishes(&reactions);
            match &published[0] {
                ServerMessage::OpApplied {
                    op_id,
                    revision,
                    changes,
                } => {
                    assert_eq!(op_id, &format!("op-{index}"));
                    assert_eq!(*revision, expected);
                    assert_eq!(changes[0]["object_ref"], "V-1001");
                    assert_eq!(changes[0]["field"], "设计压力");
                    assert_eq!(changes[0]["value"], "1.6MPa");
                    assert_eq!(changes[0]["kind"], "set_property");
                }
                other => panic!("expected op_applied, got {other:?}"),
            }
        }
        let (_, _, revision) = store_of(&state)
            .drawing("PID-1101")
            .expect("lookup")
            .expect("drawing exists");
        assert_eq!(revision, 2);
    }

    #[tokio::test]
    async fn a_stale_base_revision_is_rejected() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        let payload = json!({"kind": "set_property", "object_ref": "V-1001"});
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Op {
                session_id: session_id.clone(),
                base_revision: 0,
                op_id: "op-0".into(),
                payload: payload.clone(),
            },
            T0,
        )
        .await;
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Op {
                session_id,
                base_revision: 0,
                op_id: "op-1".into(),
                payload,
            },
            T0 + 1,
        )
        .await;
        match &sends(&reactions)[0] {
            ServerMessage::Error { code, .. } => {
                assert_eq!(code, codes::STALE_BASE_REVISION)
            }
            other => panic!("expected stale error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ops_work_without_a_store_via_the_in_memory_revisions() {
        let state = storeless_state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        let payload = json!({"kind": "set_property", "object_ref": "V-1001"});
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Op {
                session_id: session_id.clone(),
                base_revision: 0,
                op_id: "op-0".into(),
                payload: payload.clone(),
            },
            T0,
        )
        .await;
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Op {
                session_id,
                base_revision: 1,
                op_id: "op-1".into(),
                payload,
            },
            T0 + 1,
        )
        .await;
        match &publishes(&reactions)[0] {
            ServerMessage::OpApplied { revision, .. } => assert_eq!(*revision, 2),
            other => panic!("expected op_applied, got {other:?}"),
        }
    }

    /// VAL-CLI-010: the sessions of a drawing share one baseline, so an applied
    /// op moves every live one of them to its revision — in the table, and in
    /// the registry the table is rewritten from.
    #[tokio::test]
    async fn an_applied_op_moves_every_live_session_on_the_drawing_to_its_revision() {
        let state = state();
        let mut alice = opened_conn(&state, "u1", T0).await;
        let mut bob = opened_conn(&state, "u2", T0 + 1).await;
        // Somebody on another drawing shares nothing with this op.
        let mut elsewhere = conn();
        handle_message(
            &state,
            &mut elsewhere,
            ClientMessage::Hello {
                user_id: "u3".into(),
                drawing_id: "PID-2202".into(),
                mode: "协同编辑".into(),
                client_version: "test".into(),
                workspace: "plant-a".into(),
                since: None,
                existing_tags: Vec::new(),
                repo_id: None,
            },
            T0 + 2,
        )
        .await;

        let alice_id = session_id(&alice);
        let reactions = handle_message(
            &state,
            &mut alice,
            ClientMessage::Op {
                session_id: alice_id,
                base_revision: 0,
                op_id: "op-1".into(),
                payload: json!({
                    "kind": "set_tag",
                    "object_ref": "P-1001",
                    "field": "tag",
                    "value": "P-1009",
                }),
            },
            T0 + 3,
        )
        .await;
        assert!(
            matches!(
                publishes(&reactions).first(),
                Some(ServerMessage::OpApplied { revision: 1, .. })
            ),
            "the op must apply: {reactions:?}"
        );

        let stored = |conn: &Connection| {
            store_of(&state)
                .session_row(&session_id(conn))
                .expect("lookup")
                .expect("session row")
                .base_revision
        };
        assert_eq!(stored(&alice), 1, "the session that sent the op");
        assert_eq!(stored(&bob), 1, "its peer on the drawing");
        assert_eq!(stored(&elsewhere), 0, "a session on another drawing");

        // A heartbeat rewrites the row from the registry; it must not put the
        // old baseline back.
        let bob_id = session_id(&bob);
        handle_message(
            &state,
            &mut bob,
            ClientMessage::Heartbeat { session_id: bob_id },
            T0 + 4,
        )
        .await;
        assert_eq!(stored(&bob), 1, "a heartbeat must not rewind the baseline");
    }

    /// The same without a database: the registry is all there is.
    #[tokio::test]
    async fn an_applied_op_moves_the_registry_baseline_without_a_store() {
        let state = storeless_state();
        let mut alice = opened_conn(&state, "u1", T0).await;
        let bob = opened_conn(&state, "u2", T0 + 1).await;
        let alice_id = session_id(&alice);
        handle_message(
            &state,
            &mut alice,
            ClientMessage::Op {
                session_id: alice_id,
                base_revision: 0,
                op_id: "op-1".into(),
                payload: json!({"kind": "set_property", "object_ref": "V-1001"}),
            },
            T0 + 2,
        )
        .await;
        let registry = state.sessions.lock().await;
        for conn in [&alice, &bob] {
            assert_eq!(
                registry
                    .get(&session_id(conn))
                    .expect("registered")
                    .base_revision,
                1
            );
        }
    }

    #[tokio::test]
    async fn an_op_writes_a_complete_audit_row() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Op {
                session_id,
                base_revision: 0,
                op_id: "op-1".into(),
                payload: json!({
                    "kind": "set_property",
                    "object_ref": "V-1001",
                    "field": "设计压力",
                    "value": "1.6MPa",
                }),
            },
            T0,
        )
        .await;

        let page = store_of(&state)
            .audit_query(&AuditQuery::default())
            .expect("audit query");
        assert_eq!(page.total, 1);
        let row = &page.items[0];
        // VAL-SRV-020: all nine fields non-empty.
        assert!(!row.module.is_empty());
        assert_eq!(row.action, "set_property");
        assert_eq!(row.at, T0);
        assert_eq!(row.object_ref, "V-1001");
        assert_eq!(row.user_id, "u1");
        assert_eq!(row.company, "未知", "no users table seeded");
        assert_eq!(row.ip, "10.0.0.7");
        assert_eq!(row.workspace, "plant-a");
        assert_eq!(
            row.change_text,
            "set_property V-1001.设计压力: (未知) -> 1.6MPa"
        );
    }

    #[tokio::test]
    async fn a_heartbeat_on_a_stale_session_reports_expired() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        sweep_expired(&state, T0 + 91).await;
        let reactions = handle_message(
            &state,
            &mut conn,
            ClientMessage::Heartbeat { session_id },
            T0 + 92,
        )
        .await;
        match &sends(&reactions)[0] {
            ServerMessage::Error { code, .. } => assert_eq!(code, codes::SESSION_EXPIRED),
            other => panic!("expected expired error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sweeping_a_stale_session_releases_claims_and_broadcasts_peer_left() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        for tag in ["V-1001", "V-1002"] {
            handle_message(
                &state,
                &mut conn,
                ClientMessage::Claim {
                    session_id: session_id.clone(),
                    tag: tag.into(),
                },
                T0,
            )
            .await;
        }

        let reactions = sweep_expired(&state, T0 + 91).await;
        let published = publishes(&reactions);
        assert_eq!(published.len(), 3);
        assert!(matches!(&published[0], ServerMessage::ClaimReleased { tag, reason } if tag == "V-1001" && reason == "expired"));
        assert!(matches!(&published[1], ServerMessage::ClaimReleased { tag, reason } if tag == "V-1002" && reason == "expired"));
        assert!(matches!(&published[2], ServerMessage::PeerLeft(peer) if peer.session_id == session_id));

        assert!(store_of(&state)
            .claims_on_drawing("PID-1101")
            .expect("claims query")
            .is_empty());
        let row = store_of(&state)
            .session_row(&session_id)
            .expect("lookup")
            .expect("session row");
        assert_eq!(row.status, "失效");
    }

    #[tokio::test]
    async fn an_expired_claim_broadcasts_claim_released() {
        let state = state();
        let mut conn = opened_conn(&state, "u1", T0).await;
        let session_id = session_id(&conn);
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Claim {
                session_id: session_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        // Keep the session alive past the claim TTL (300 s).
        handle_message(
            &state,
            &mut conn,
            ClientMessage::Heartbeat {
                session_id: session_id.clone(),
            },
            T0 + 250,
        )
        .await;
        let reactions = sweep_expired(&state, T0 + 301).await;
        assert_eq!(
            publishes(&reactions),
            vec![ServerMessage::ClaimReleased {
                tag: "V-1001".into(),
                reason: "expired".into(),
            }]
        );
        assert!(store_of(&state)
            .claims_on_drawing("PID-1101")
            .expect("claims query")
            .is_empty());
    }

    #[tokio::test]
    async fn takeover_releases_the_old_sessions_claims_and_notifies_the_room() {
        let state = state();
        let mut a = opened_conn(&state, "u1", T0).await;
        let a_id = session_id(&a);
        handle_message(
            &state,
            &mut a,
            ClientMessage::Claim {
                session_id: a_id.clone(),
                tag: "V-1001".into(),
            },
            T0,
        )
        .await;
        // A goes stale; B's hello on the same drawing+mode takes it over.
        let reactions = sweep_expired(&state, T0 + 91).await;
        assert_eq!(publishes(&reactions).len(), 2, "claim release + peer left");

        let mut b = conn();
        let reactions =
            handle_message(&state, &mut b, hello("u2", "协同编辑", ""), T0 + 92).await;
        // The takeover error is targeted: it names A and nobody else. It must
        // not ride the room bus, or every peer on the drawing would see an
        // error that has nothing to do with them.
        assert!(
            reactions.iter().any(|reaction| matches!(reaction,
                Reaction::SendTo { session_id, message }
                    if session_id == &a_id
                        && matches!(message, ServerMessage::Error { code, .. }
                            if code == codes::SESSION_TAKEN_OVER))),
            "the displaced session must be told directly: {reactions:?}"
        );
        assert!(
            !reactions.iter().any(|reaction| matches!(reaction,
                Reaction::Publish { message: ServerMessage::Error { code, .. }, .. }
                    if code == codes::SESSION_TAKEN_OVER)),
            "the takeover error must not be published to the room: {reactions:?}"
        );
        assert!(state
            .occupancy
            .lock()
            .await
            .for_drawing("PID-1101", T0 + 93)
            .0
            .is_empty());
    }

    #[tokio::test]
    async fn takeover_error_does_not_reach_a_third_peer() {
        let state = state();
        let a = opened_conn(&state, "u1", T0).await;
        let a_id = session_id(&a);
        // D is an unrelated active peer on the same drawing throughout; it must
        // never be handed the takeover error meant for A.
        let mut d = opened_conn(&state, "u4", T0).await;
        let d_id = session_id(&d);
        assert_ne!(a_id, d_id);

        // Only A goes stale (D keeps heartbeating).
        let _ = handle_message(
            &state,
            &mut d,
            ClientMessage::Heartbeat {
                session_id: d_id.clone(),
            },
            T0 + 90,
        )
        .await;
        sweep_expired(&state, T0 + 91).await;

        let mut b = conn();
        let reactions =
            handle_message(&state, &mut b, hello("u2", "协同编辑", ""), T0 + 92).await;

        let targeted: Vec<&String> = reactions
            .iter()
            .filter_map(|reaction| match reaction {
                Reaction::SendTo { session_id, message } => match message {
                    ServerMessage::Error { code, .. } if code == codes::SESSION_TAKEN_OVER => {
                        Some(session_id)
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            targeted,
            vec![&a_id],
            "exactly one takeover error, addressed to A: {reactions:?}"
        );
        assert!(
            !reactions.iter().any(|reaction| matches!(reaction,
                Reaction::Publish { message: ServerMessage::Error { code, .. }, .. }
                    if code == codes::SESSION_TAKEN_OVER)),
            "no publish may carry the takeover error: {reactions:?}"
        );
    }

    #[test]
    fn unparseable_frames_map_to_invalid_message() {
        assert!(serde_json::from_str::<ClientMessage>("{not json").is_err());
        assert!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"totally_unknown"}"#).is_err()
        );
        match invalid_message_error() {
            ServerMessage::Error { code, .. } => assert_eq!(code, codes::INVALID_MESSAGE),
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn own_peer_events_are_not_forwarded() {
        let own = Peer {
            session_id: "s-1".into(),
            user_id: "u1".into(),
            display_name: "u1".into(),
        };
        let other = Peer {
            session_id: "s-2".into(),
            user_id: "u2".into(),
            display_name: "u2".into(),
        };
        assert!(!should_forward("s-1", &ServerMessage::PeerJoined(own.clone())));
        assert!(!should_forward("s-1", &ServerMessage::PeerLeft(own)));
        assert!(should_forward("s-1", &ServerMessage::PeerJoined(other.clone())));
        assert!(should_forward("s-1", &ServerMessage::PeerLeft(other)));
        assert!(should_forward(
            "s-1",
            &ServerMessage::OpApplied {
                op_id: "op-1".into(),
                revision: 1,
                changes: vec![],
            }
        ));
        assert!(should_forward(
            "s-1",
            &ServerMessage::ClaimReleased {
                tag: "V-1001".into(),
                reason: "released".into(),
            }
        ));
    }

    /// `OFF-4`: what is said to one session alone goes after what its room
    /// was told before. The answer to an op sent again names the revision it
    /// took effect at; the revisions in front of it reach the copy first, in
    /// the order the room was told them, each once.
    #[test]
    fn duplicate_op_what_the_room_was_told_before_goes_out_first() {
        let bus = crate::bus::Bus::new(8);
        let mut room = bus.subscribe("PID-1101");
        let applied = |revision: i64| ServerMessage::OpApplied {
            op_id: format!("op-r-1-{revision}"),
            revision,
            changes: vec![],
        };
        let me = Peer {
            session_id: "s-1".into(),
            user_id: "u1".into(),
            display_name: "u1".into(),
        };
        bus.publish("PID-1101", &applied(1));
        bus.publish("PID-1101", &ServerMessage::PeerJoined(me));
        bus.publish("PID-1101", &applied(2));

        assert_eq!(
            earlier_in_the_room(&mut room, "s-1"),
            [applied(1), applied(2)]
        );
        assert!(
            earlier_in_the_room(&mut room, "s-1").is_empty(),
            "each once"
        );
    }

    /// `REQ-COLL-015`: a `hello` whose copy claims alignment is judged. A
    /// matching repository and a replayable `since` is welcomed with the
    /// room's `repo_id` and the ops it missed; the three ways the claim
    /// cannot hold are each said in the `welcome` itself (`align_refused`)
    /// — the session still stands, the copy is not taken as consistent.
    #[tokio::test]
    async fn a_hello_that_claims_alignment_is_judged() {
        let state = state();
        // Put one op on the room so a `since` has something to be behind.
        let mut first = conn();
        handle_message(&state, &mut first, hello("u1", "协同编辑", ""), T0).await;
        let first_id = session_id(&first);
        let op = ClientMessage::Op {
            session_id: first_id,
            base_revision: 0,
            op_id: "op-1".into(),
            payload: serde_json::json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        handle_message(&state, &mut first, op, T0 + 1).await;

        // A copy of this repository at revision 0: welcomed with the repo
        // id, sent the op it missed, refused nothing.
        let mut back = conn();
        let reactions =
            handle_message(&state, &mut back, hello_back(&state, "u2", Some(0)), T0 + 2).await;
        assert!(back.session.is_some(), "the session stands");
        assert_eq!(refused(&reactions), None);
        let sent = sends(&reactions);
        let welcome = sent
            .iter()
            .find_map(|message| match message {
                ServerMessage::Welcome {
                    repo_id,
                    replay_from,
                    ..
                } => Some((repo_id.clone(), *replay_from)),
                _ => None,
            })
            .expect("a welcome");
        assert_eq!(welcome, (state.repo_id.clone(), Some(0)));
        assert!(sent.iter().any(|message| matches!(
            message,
            ServerMessage::OpApplied { op_id, .. } if op_id == "op-1"
        )));

        // A copy of another repository: same session treatment, plus a
        // refusal in its welcome.
        let mut alien = conn();
        let mut alien_hello = hello("u3", "协同编辑", "");
        if let ClientMessage::Hello { repo_id, since, .. } = &mut alien_hello {
            *repo_id = Some("r-somebody-else".into());
            *since = Some(0);
        }
        let reactions = handle_message(&state, &mut alien, alien_hello, T0 + 3).await;
        assert!(alien.session.is_some(), "the session still stands");
        assert_eq!(
            refused(&reactions).map(|(code, _)| code),
            Some(codes::REPO_MISMATCH.to_owned())
        );
        // There is no frame of its own: it rides in the welcome.
        let types: Vec<serde_json::Value> = wire_of(&reactions)
            .into_iter()
            .map(|wire| wire["type"].clone())
            .collect();
        assert!(!types.contains(&json!("align_refused")), "{types:?}");

        // A copy ahead of what this repository has reached.
        let mut ahead = conn();
        let reactions = handle_message(
            &state,
            &mut ahead,
            hello_back(&state, "u4", Some(9)),
            T0 + 4,
        )
        .await;
        assert_eq!(
            refused(&reactions).map(|(code, _)| code),
            Some(codes::REVISION_AHEAD.to_owned())
        );

        // No database: an op still moves the revision, but there is no log
        // to replay from, so a copy behind it cannot be proved consistent.
        let bare = storeless_state();
        let mut first = conn();
        handle_message(&bare, &mut first, hello("u1", "协同编辑", ""), T0 + 5).await;
        let first_id = session_id(&first);
        let op = ClientMessage::Op {
            session_id: first_id,
            base_revision: 0,
            op_id: "op-1".into(),
            payload: serde_json::json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        handle_message(&bare, &mut first, op, T0 + 6).await;
        let mut orphan = conn();
        let reactions =
            handle_message(&bare, &mut orphan, hello_back(&bare, "u5", Some(0)), T0 + 7).await;
        assert_eq!(
            refused(&reactions).map(|(code, _)| code),
            Some(codes::REPLAY_GAP.to_owned())
        );
        // But "caught up to the current revision" needs no log at all.
        let mut empty = conn();
        let reactions =
            handle_message(&bare, &mut empty, hello_back(&bare, "u6", Some(1)), T0 + 8).await;
        assert_eq!(refused(&reactions), None);
    }

    /// `REQ-COLL-015`: a refused copy's `existing_tags` are not registered —
    /// what a diverged drawing lists protects nobody.
    #[tokio::test]
    async fn a_refused_copys_existing_tags_are_not_registered() {
        let state = state();
        let mut alien = conn();
        let mut hello = hello_with("u1", "协同编辑", &["V-1099"]);
        if let ClientMessage::Hello { repo_id, .. } = &mut hello {
            *repo_id = Some("r-somebody-else".into());
        }
        let reactions = handle_message(&state, &mut alien, hello, T0).await;
        assert_eq!(
            refused(&reactions).map(|(code, _)| code),
            Some(codes::REPO_MISMATCH.to_owned())
        );
        assert!(
            store_of(&state)
                .reported_tags("PID-1101")
                .expect("read reported tags")
                .is_empty(),
            "a refused copy registers no tags"
        );
    }

    /// `OFF-2`: on a connection whose `hello` was refused, an op is answered
    /// `NOT_ALIGNED` — by its `op_id`, and not `rejected`: the copy keeps it
    /// queued and takes nothing back. Nothing reaches the room, and the
    /// revision does not move.
    #[tokio::test]
    async fn an_op_on_a_connection_not_taken_as_aligned_is_answered_not_aligned() {
        let state = state();
        let mut alien = conn();
        let mut alien_hello = hello("u1", "协同编辑", "");
        if let ClientMessage::Hello { repo_id, since, .. } = &mut alien_hello {
            *repo_id = Some("r-somebody-else".into());
            *since = Some(0);
        }
        handle_message(&state, &mut alien, alien_hello, T0).await;
        let op = ClientMessage::Op {
            session_id: session_id(&alien),
            base_revision: 0,
            op_id: "op-alien".into(),
            payload: json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        let reactions = handle_message(&state, &mut alien, op, T0 + 1).await;
        let wire = wire_of(&reactions);
        assert_eq!(wire.len(), 1, "{wire:?}");
        assert_eq!(wire[0]["type"], "error", "{wire:?}");
        assert_eq!(wire[0]["code"], "NOT_ALIGNED", "{wire:?}");
        assert_eq!(wire[0]["op_id"], "op-alien", "{wire:?}");
        assert!(wire[0].get("rejected").is_none(), "{wire:?}");
        assert_eq!(
            current_revision(&state, "PID-1101"),
            0,
            "the op took no effect"
        );
        assert!(
            store_of(&state)
                .load_ops("PID-1101", 0)
                .expect("read the op log")
                .is_empty(),
            "the op log has no new row"
        );
    }

    /// `OFF-2` (R3 of `SPEC-collab-offline-sync.md` §11.2): a `hello` that
    /// names no repository is not judged — it is served as it was before
    /// the repository was named (protocol §7, rule 2): its `since` is
    /// replayed from, and its tags are registered.
    #[tokio::test]
    async fn alignment_is_not_judged_for_a_hello_that_names_no_repository() {
        let state = state();
        let mut first = conn();
        handle_message(&state, &mut first, hello("u1", "协同编辑", ""), T0).await;
        let op = ClientMessage::Op {
            session_id: session_id(&first),
            base_revision: 0,
            op_id: "op-1".into(),
            payload: json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        handle_message(&state, &mut first, op, T0 + 1).await;

        let mut back = conn();
        let mut since_only = hello_with("u2", "协同编辑", &["V-1099"]);
        if let ClientMessage::Hello { since, .. } = &mut since_only {
            *since = Some(0);
        }
        let reactions = handle_message(&state, &mut back, since_only, T0 + 2).await;
        assert_eq!(refused(&reactions), None);
        let wire = wire_of(&reactions);
        let welcome = wire
            .iter()
            .find(|frame| frame["type"] == "welcome")
            .expect("a welcome");
        assert_eq!(welcome["replay_from"], 0, "{welcome}");
        assert!(
            wire.iter()
                .any(|frame| frame["type"] == "op_applied" && frame["op_id"] == "op-1"),
            "{wire:?}"
        );
        assert!(
            wire.iter().all(|frame| frame["type"] != "align_refused"),
            "{wire:?}"
        );
        assert_eq!(
            store_of(&state)
                .reported_tags("PID-1101")
                .expect("read reported tags")
                .len(),
            1,
            "its tags are registered"
        );
    }

    /// What the binary builds on a database it could not open.
    fn unavailable_state() -> AppState {
        let mut state = storeless_state();
        state.store_unavailable = true;
        state
    }

    /// `OFF-11`: with no database the server would keep nothing a session
    /// did, so it takes none. The `hello` is answered `STORE_UNAVAILABLE`
    /// and the connection closed: no session, nothing told to the room, no
    /// tag registered.
    #[tokio::test]
    async fn store_unavailable_a_hello_is_refused_and_the_connection_closed() {
        let state = unavailable_state();
        let mut conn = conn();
        let listing = hello_with("u1", "协同编辑", &["V-1001"]);
        let reactions = handle_message(&state, &mut conn, listing, T0).await;
        let [Reaction::Send(ServerMessage::Error { code, message, .. }), Reaction::Close] =
            reactions.as_slice()
        else {
            panic!("an error, then the close: {reactions:?}");
        };
        assert_eq!(code, codes::STORE_UNAVAILABLE);
        assert!(message.contains("库"), "{message}");
        assert!(
            !message.contains(&state.db_path),
            "the path stays on the server: {message}"
        );
        assert!(conn.session.is_none(), "no session");
        assert!(crate::ops::reported_tags(&state, "PID-1101", None).is_empty());
    }

    /// `OFF-11`: a state built in code without a store — what these tests
    /// call storeless — is not one whose database failed to open. It takes
    /// sessions as it did, whatever its `db_error` says.
    #[tokio::test]
    async fn store_unavailable_is_not_what_a_storeless_state_built_in_code_is() {
        let bare = AppState::new(None, None, ":memory:", SessionConfig::default());
        for state in [storeless_state(), bare] {
            assert!(!state.store_unavailable);
            let mut conn = conn();
            let reactions =
                handle_message(&state, &mut conn, hello("u1", "协同编辑", ""), T0).await;
            assert!(conn.session.is_some(), "{reactions:?}");
            assert!(!reactions.contains(&Reaction::Close), "{reactions:?}");
        }
    }

    /// `OFF-11`: after a restore from backup the server is started once
    /// with `--new-repo-id`. A copy that remembers the id from before is
    /// then of another repository — `REPO_MISMATCH` — even where the room
    /// has moved on to the very revision it names, which is what made the
    /// old id dangerous. The same claim under the new id passes.
    #[tokio::test]
    async fn new_repo_id_a_drawing_that_remembers_the_old_id_gets_repo_mismatch() {
        let store = Store::open_in_memory().expect("memory store");
        let old = store.repo_id().expect("repo id");
        let renewed = store.renew_repo_id().expect("renewed");
        let state = AppState::new(Some(store), None, ":memory:", SessionConfig::default());
        assert_eq!(state.repo_id, renewed);

        let mut first = conn();
        handle_message(&state, &mut first, hello("u1", "协同编辑", ""), T0).await;
        let op = ClientMessage::Op {
            session_id: session_id(&first),
            base_revision: 0,
            op_id: "op-1".into(),
            payload: json!({"kind": "set_property", "object_ref": "V-1001",
                "field": "设计压力", "value": "1.6MPa"}),
        };
        handle_message(&state, &mut first, op, T0 + 1).await;

        let mut remembering = conn();
        let mut hello_from_before = hello("u2", "协同编辑", "");
        if let ClientMessage::Hello { repo_id, since, .. } = &mut hello_from_before {
            *repo_id = Some(old);
            *since = Some(1);
        }
        let reactions = handle_message(&state, &mut remembering, hello_from_before, T0 + 2).await;
        assert_eq!(
            refused(&reactions).map(|(code, _)| code),
            Some(codes::REPO_MISMATCH.to_owned()),
            "{reactions:?}"
        );

        let mut current = conn();
        let hello_now = hello_back(&state, "u3", Some(1));
        let reactions = handle_message(&state, &mut current, hello_now, T0 + 3).await;
        assert_eq!(refused(&reactions), None, "{reactions:?}");
    }
}

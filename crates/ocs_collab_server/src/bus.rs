//! Broadcast bus: fan-out of server messages to every session on a drawing.
//!
//! One `tokio::sync::broadcast` channel per drawing gives each subscriber its
//! own queue, so a slow client cannot stall the others. Rooms are created on
//! first subscribe and dropped when the last subscriber goes away, which keeps
//! an idle server's map empty.

use std::collections::HashMap;
use std::sync::Mutex;

use ocs_collab::ServerMessage;
use tokio::sync::broadcast;

/// How many messages a slow subscriber may fall behind before it starts losing
/// them. Losing a change notification is recoverable (the client re-syncs from
/// `base_revision`); blocking the writer is not.
pub const DEFAULT_CAPACITY: usize = 256;

/// A handle onto one drawing's channel.
pub struct Subscription {
    drawing_id: String,
    receiver: broadcast::Receiver<ServerMessage>,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("drawing_id", &self.drawing_id)
            .finish_non_exhaustive()
    }
}

impl Subscription {
    pub fn drawing_id(&self) -> &str {
        &self.drawing_id
    }

    /// Next message, or `None` when the room closed or the stream lagged far
    /// enough that the runtime dropped frames.
    pub async fn recv(&mut self) -> Option<ServerMessage> {
        loop {
            match self.receiver.recv().await {
                Ok(message) => return Some(message),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // Skipped frames are a slow consumer, not a dead room.
                    eprintln!("[bus] subscriber lagged, dropped {skipped} message(s)");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Non-blocking variant, used when draining during shutdown.
    pub fn try_recv(&mut self) -> Option<ServerMessage> {
        match self.receiver.try_recv() {
            Ok(message) => Some(message),
            Err(broadcast::error::TryRecvError::Lagged(_)) => None,
            Err(broadcast::error::TryRecvError::Empty) => None,
            Err(broadcast::error::TryRecvError::Closed) => None,
        }
    }
}

/// The per-drawing fan-out.
pub struct Bus {
    capacity: usize,
    rooms: Mutex<HashMap<String, broadcast::Sender<ServerMessage>>>,
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        Bus {
            capacity: capacity.max(1),
            rooms: Mutex::new(HashMap::new()),
        }
    }

    /// Joins a drawing's room, creating it when needed.
    pub fn subscribe(&self, drawing_id: &str) -> Subscription {
        let mut rooms = self.rooms.lock().expect("bus mutex poisoned");
        let sender = rooms
            .entry(drawing_id.to_owned())
            .or_insert_with(|| broadcast::channel(self.capacity).0);
        Subscription {
            drawing_id: drawing_id.to_owned(),
            receiver: sender.subscribe(),
        }
    }

    /// Publishes to every subscriber of one drawing. Returns how many
    /// receivers accepted the message (0 when the room has nobody listening).
    pub fn publish(&self, drawing_id: &str, message: &ServerMessage) -> usize {
        let mut rooms = self.rooms.lock().expect("bus mutex poisoned");
        let Some(sender) = rooms.get(drawing_id) else {
            return 0;
        };
        let delivered = sender.send(message.clone()).unwrap_or(0);
        // Drop the room once nobody is listening; otherwise every drawing ever
        // opened would leak a channel for the life of the process.
        if sender.receiver_count() == 0 {
            rooms.remove(drawing_id);
        }
        delivered
    }

    /// Live subscriber count for one drawing.
    pub fn subscriber_count(&self, drawing_id: &str) -> usize {
        let rooms = self.rooms.lock().expect("bus mutex poisoned");
        rooms
            .get(drawing_id)
            .map(|sender| sender.receiver_count())
            .unwrap_or(0)
    }

    /// Number of rooms currently held.
    pub fn room_count(&self) -> usize {
        self.rooms.lock().expect("bus mutex poisoned").len()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Bus::new(DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(session_id: &str, user_id: &str) -> ServerMessage {
        ServerMessage::PeerJoined(ocs_collab::protocol::Peer {
            session_id: session_id.into(),
            user_id: user_id.into(),
            display_name: String::new(),
            status: None,
            hold_until: None,
        })
    }

    #[test]
    fn a_message_reaches_every_subscriber_of_the_drawing() {
        let bus = Bus::new(8);
        let mut a = bus.subscribe("PID-1101");
        let mut b = bus.subscribe("PID-1101");
        let delivered = bus.publish("PID-1101", &peer("s-1", "u1"));
        assert_eq!(delivered, 2);
        assert_eq!(a.try_recv(), Some(peer("s-1", "u1")));
        assert_eq!(b.try_recv(), Some(peer("s-1", "u1")));
    }

    #[test]
    fn a_message_does_not_cross_drawings() {
        let bus = Bus::new(8);
        let mut on_1101 = bus.subscribe("PID-1101");
        let mut on_1102 = bus.subscribe("PID-1102");
        bus.publish("PID-1101", &peer("s-1", "u1"));
        assert_eq!(on_1101.try_recv(), Some(peer("s-1", "u1")));
        assert_eq!(on_1102.try_recv(), None, "other drawings must not see it");
    }

    #[test]
    fn publishing_to_an_empty_room_delivers_nothing() {
        let bus = Bus::new(8);
        assert_eq!(bus.publish("PID-1101", &peer("s-1", "u1")), 0);
        assert_eq!(bus.room_count(), 0);
    }

    #[test]
    fn a_room_disappears_once_its_last_subscriber_drops() {
        let bus = Bus::new(8);
        {
            let _sub = bus.subscribe("PID-1101");
            assert_eq!(bus.room_count(), 1);
            assert_eq!(bus.subscriber_count("PID-1101"), 1);
        }
        // Publishing with nobody listening removes the idle room.
        assert_eq!(bus.publish("PID-1101", &peer("s-1", "u1")), 0);
        assert_eq!(bus.room_count(), 0);
    }

    #[test]
    fn subscriber_count_tracks_join_and_leave() {
        let bus = Bus::new(8);
        let a = bus.subscribe("PID-1101");
        let b = bus.subscribe("PID-1101");
        assert_eq!(bus.subscriber_count("PID-1101"), 2);
        drop(a);
        assert_eq!(bus.subscriber_count("PID-1101"), 1);
        drop(b);
        assert_eq!(bus.subscriber_count("PID-1101"), 0);
    }

    #[tokio::test]
    async fn an_async_subscriber_receives_the_published_message() {
        let bus = Bus::new(8);
        let mut sub = bus.subscribe("PID-1101");
        bus.publish("PID-1101", &peer("s-9", "u9"));
        let received = sub.recv().await.expect("message must arrive");
        assert_eq!(received, peer("s-9", "u9"));
    }
}

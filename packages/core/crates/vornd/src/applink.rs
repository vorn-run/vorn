//! What vornd and the app's server tell each other outside any client's
//! connection.
//!
//! vornd tells the server what only the server can act on: a broadcast to
//! every client (the server holds the registry of them, and some clients
//! connect to it directly) and a device token just revoked (the server
//! closes the sockets holding it). The server tells vornd where it is bound,
//! which decides the addresses a browser on the network can use.
//!
//! Notes travel on the app's channel ([`crate::control`]) to every
//! connection that has subscribed; with none subscribed, [`AppLink::tell`]
//! says so and the caller leaves the call to the server.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};

/// Notes queued for slow subscribers before the oldest are dropped.
const BACKLOG: usize = 256;

#[derive(Debug)]
pub struct AppLink {
    notes: broadcast::Sender<Value>,
    listening: AtomicUsize,
    server_host: Mutex<Option<String>>,
    reached: Notify,
}

impl Default for AppLink {
    fn default() -> Self {
        AppLink {
            notes: broadcast::channel(BACKLOG).0,
            listening: AtomicUsize::new(0),
            server_host: Mutex::new(None),
            reached: Notify::new(),
        }
    }
}

impl AppLink {
    /// Sends the server `method` with `params`. False when no server is
    /// listening, so nothing was sent.
    pub fn tell(&self, method: &str, params: Value) -> bool {
        if !self.listening() {
            return false;
        }
        let note = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.notes.send(note).is_ok()
    }

    /// Whether a server has subscribed and is still connected.
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::Acquire) > 0
    }

    /// The notes for one subscribed connection. Dropping the guard ends it.
    pub fn listen(self: &Arc<Self>) -> (broadcast::Receiver<Value>, Listening) {
        let rx = self.notes.subscribe();
        self.listening.fetch_add(1, Ordering::AcqRel);
        (rx, Listening(Arc::clone(self)))
    }

    /// The address the server is bound to, as it last said: `0.0.0.0` when
    /// it takes connections from the network. `None` until it says.
    pub fn server_host(&self) -> Option<String> {
        self.server_host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Where the server is bound, which it says when it subscribes and
    /// whenever its settings change.
    pub fn set_server_host(&self, host: String) {
        *self.server_host.lock().unwrap_or_else(|e| e.into_inner()) = Some(host);
        self.reached.notify_one();
    }

    /// Resolves once the server has said where it is since the last time
    /// this resolved: the moment to read again what depends on it.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }
}

/// A subscribed connection, counted while it lives.
#[derive(Debug)]
pub struct Listening(Arc<AppLink>);

impl Drop for Listening {
    fn drop(&mut self) {
        self.0.listening.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_nothing_with_nobody_listening() {
        let link = Arc::new(AppLink::default());
        assert!(!link.tell("vornd:broadcast", json!({})));
        let (mut rx, guard) = link.listen();
        assert!(link.tell("vornd:broadcast", json!({ "a": 1 })));
        assert_eq!(
            rx.try_recv().unwrap(),
            json!({ "jsonrpc": "2.0", "method": "vornd:broadcast", "params": { "a": 1 } })
        );
        drop(guard);
        assert!(!link.listening());
    }
}

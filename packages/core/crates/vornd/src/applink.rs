//! What vornd and the app's server tell each other outside any client's
//! connection.
//!
//! vornd tells the server what only the server can act on: a broadcast to
//! every client (the server holds the registry of them, and some clients
//! connect to it directly), a device token just revoked (the server closes
//! the sockets holding it) and how many sessions run, which keeps it from
//! stopping as idle. The server tells vornd where it is bound, which
//! decides the addresses a browser on the network can use; whether it is
//! winding down, when vornd may start no new terminal ([`Closing`]); the
//! conversations its own starts claim ([`Claims`]); and the spawns it asks
//! for, which the comparison of vornd's plans reads ([`AppLink::spawned`]).
//!
//! Notes travel on the app's channel ([`crate::control`]) to every
//! connection that has subscribed; with none subscribed, [`AppLink::tell`]
//! says so and the caller leaves the call to the server.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};

use crate::claims::Claims;
use crate::native::script::Scripts;

/// Notes queued for slow subscribers before the oldest are dropped.
const BACKLOG: usize = 256;

/// Spawns kept for the comparison of vornd's plans with them: more than
/// are ever asked for at once.
const SPAWNS_KEPT: usize = 64;

/// The server's refusals while it winds down, word for word.
pub const DRAINING_MESSAGE: &str = "This server no longer holds the local endpoint and is finishing its remaining sessions. Reopen Vorn to start new ones.";
pub const HANDOVER_MESSAGE: &str =
    "Vorn is moving your terminals to the updated server. Try again in a moment.";

/// Whether the server is winding down, as `vornd:draining` last said.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Closing {
    #[default]
    Open,
    /// It lost its endpoint and is finishing what it has.
    Draining,
    /// It is handing its sessions to a server taking over.
    HandingOver,
}

impl Closing {
    /// The refusal for a new session, while there is one. Draining is
    /// asked first, as the server asks it.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Closing::Open => None,
            Closing::Draining => Some(DRAINING_MESSAGE),
            Closing::HandingOver => Some(HANDOVER_MESSAGE),
        }
    }
}

#[derive(Debug)]
pub struct AppLink {
    notes: broadcast::Sender<Value>,
    listening: AtomicUsize,
    server_host: Mutex<Option<String>>,
    reached: Notify,
    /// Whether vornd answers the clients' `terminal:create` and the rest
    /// itself, which `vornd:hello` tells the server.
    terminals: AtomicBool,
    /// The same for `headless:create` and `headless:kill`.
    headless: AtomicBool,
    /// What runs or compares the server's scripts, while vornd does.
    scripts: OnceLock<Arc<Scripts>>,
    closing: Mutex<Closing>,
    /// The conversations being started, by vornd's creates and the
    /// server's own starts alike.
    claims: Claims,
    /// The latest spawns the server asked for, by the name it gave each.
    spawns: Mutex<VecDeque<(String, Value)>>,
    spawned: Notify,
    /// The work model, which takes the triggers the server delivers.
    work: OnceLock<Arc<crate::native::work::Work>>,
    /// vornd's own calls, for the server's relays when no work model runs.
    native: OnceLock<std::sync::Weak<crate::native::Native>>,
}

impl Default for AppLink {
    fn default() -> Self {
        AppLink {
            notes: broadcast::channel(BACKLOG).0,
            listening: AtomicUsize::new(0),
            server_host: Mutex::new(None),
            reached: Notify::new(),
            terminals: AtomicBool::new(false),
            headless: AtomicBool::new(false),
            scripts: OnceLock::new(),
            closing: Mutex::new(Closing::Open),
            claims: Claims::default(),
            spawns: Mutex::new(VecDeque::new()),
            spawned: Notify::new(),
            work: OnceLock::new(),
            native: OnceLock::new(),
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

    /// vornd answers the terminal calls that create and change sessions.
    pub fn set_creates_terminals(&self) {
        self.terminals.store(true, Ordering::Release);
    }

    pub fn creates_terminals(&self) -> bool {
        self.terminals.load(Ordering::Acquire)
    }

    /// vornd answers the calls that start and stop headless agents.
    pub fn set_creates_headless(&self) {
        self.headless.store(true, Ordering::Release);
    }

    pub fn creates_headless(&self) -> bool {
        self.headless.load(Ordering::Acquire)
    }

    /// What runs the server's scripts, which `vornd:hello` tells it. Only
    /// the first one given is kept.
    pub fn set_scripts(&self, scripts: Arc<Scripts>) {
        let _ = self.scripts.set(scripts);
    }

    pub fn scripts(&self) -> Option<&Arc<Scripts>> {
        self.scripts.get()
    }

    /// vornd owns the session records between runs, and answers the calls
    /// that list and resume the sessions of earlier runs: it creates both
    /// terminals and headless agents, so every record is its own.
    pub fn restores(&self) -> bool {
        self.creates_terminals() && self.creates_headless()
    }

    /// The work model. Only the first one given is kept.
    pub fn set_work(&self, work: Arc<crate::native::work::Work>) {
        let _ = self.work.set(work);
    }

    pub fn work(&self) -> Option<&Arc<crate::native::work::Work>> {
        self.work.get()
    }

    pub fn set_native(&self, native: &Arc<crate::native::Native>) {
        let _ = self.native.set(Arc::downgrade(native));
    }

    pub fn native(&self) -> Option<Arc<crate::native::Native>> {
        self.native.get()?.upgrade()
    }

    /// What the server said of its winding down.
    pub fn set_closing(&self, closing: Closing) {
        *self.closing.lock().unwrap_or_else(|e| e.into_inner()) = closing;
    }

    pub fn closing(&self) -> Closing {
        *self.closing.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The conversations being started.
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// The server asked vornd to spawn `params` under `name`.
    pub fn record_spawn(&self, name: &str, params: &Value) {
        let mut spawns = self.spawns.lock().unwrap_or_else(|e| e.into_inner());
        if spawns.len() == SPAWNS_KEPT {
            spawns.pop_front();
        }
        spawns.push_back((name.to_owned(), params.clone()));
        drop(spawns);
        self.spawned.notify_waiters();
    }

    /// The spawn the server asked for under `name`, waiting up to `wait`
    /// for it: the server answers a create before it asks for the spawn.
    pub async fn spawned(&self, name: &str, wait: Duration) -> Option<Value> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Listening before looking, so a spawn recorded in between wakes it.
            let next = self.spawned.notified();
            tokio::pin!(next);
            next.as_mut().enable();
            let found = self
                .spawns
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, p)| p.clone());
            if found.is_some() {
                return found;
            }
            if tokio::time::timeout_at(deadline, next).await.is_err() {
                return None;
            }
        }
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

    #[tokio::test]
    async fn finds_a_spawn_asked_for_before_or_after_it_looks() {
        let link = Arc::new(AppLink::default());
        link.record_spawn("a", &json!({ "argv": ["sh"] }));
        assert_eq!(
            link.spawned("a", Duration::from_millis(1)).await,
            Some(json!({ "argv": ["sh"] }))
        );
        let later = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { link.spawned("b", Duration::from_secs(5)).await })
        };
        tokio::task::yield_now().await;
        link.record_spawn("b", &json!(1));
        assert_eq!(later.await.unwrap(), Some(json!(1)));
        assert_eq!(link.spawned("c", Duration::from_millis(5)).await, None);
    }
}

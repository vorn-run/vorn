//! The clients of vornd as the server, and the notifications each is sent.
//!
//! A client may name the topics it wants (`?topics=` on the upgrade, or
//! `subscribe:set`): a method by name, every method of a namespace (`ns:*`),
//! or one session's (`method#id`). A client that names none gets every
//! notification. When a client last said anything is kept too, which the idle
//! watch reads ([`super::idle`]).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::streams::Forwarder;

/// The close code for a credential refused, or revoked while in use.
pub const CLOSE_CREDENTIAL_REJECTED: u16 = 4002;
/// The close code for a socket that never authenticated.
pub const CLOSE_UNAUTHENTICATED: u16 = 4001;

/// The notifications a client asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topics {
    exact: HashSet<String>,
    prefixes: Vec<String>,
}

impl Topics {
    /// `None` for an empty list, which asks for everything.
    pub fn of(topics: &[String]) -> Option<Topics> {
        let mut t = Topics::default();
        for topic in topics.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            match topic.strip_suffix('*') {
                Some(prefix) => t.prefixes.push(prefix.to_owned()),
                None => {
                    t.exact.insert(topic.to_owned());
                }
            }
        }
        (!t.exact.is_empty() || !t.prefixes.is_empty()).then_some(t)
    }

    /// The topics in a `?topics=a,b` query, if it names any.
    pub fn from_query(query: Option<&str>) -> Option<Topics> {
        let raw = query?
            .split('&')
            .find_map(|kv| kv.strip_prefix("topics="))?;
        let decoded = url::form_urlencoded::parse(format!("t={raw}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())?;
        let list: Vec<String> = decoded.split(',').map(str::to_owned).collect();
        Topics::of(&list)
    }

    /// The topics a `subscribe:set` names: `None` when it names none, and
    /// `Some(None)` (everything) for an empty or malformed list.
    pub fn from_params(params: &Value) -> Option<Option<Topics>> {
        let topics = params.get("topics")?;
        let Some(list) = topics.as_array() else {
            warn!("ignoring a malformed topic list; this client will receive everything");
            return Some(None);
        };
        let names: Option<Vec<String>> =
            list.iter().map(|t| t.as_str().map(str::to_owned)).collect();
        match names {
            Some(names) => Some(Topics::of(&names)),
            None => {
                warn!("ignoring a malformed topic list; this client will receive everything");
                Some(None)
            }
        }
    }

    fn wants(&self, method: &str, scope: Option<&str>) -> bool {
        self.exact.contains(method)
            || scope.is_some_and(|s| self.exact.contains(&format!("{method}#{s}")))
            || self.prefixes.iter().any(|p| method.starts_with(p.as_str()))
    }
}

#[derive(Debug)]
struct Client {
    out: Forwarder,
    /// Ends its socket's read loop, so a client that ignores the close is cut off.
    ended: Arc<Notify>,
    /// `None` for every notification.
    topics: Option<Topics>,
    /// The device token it authenticated with, closed when that is revoked.
    token: Option<String>,
}

/// Every admitted client.
#[derive(Debug)]
pub struct Clients {
    clients: Mutex<HashMap<u64, Client>>,
    last_activity: Mutex<Instant>,
}

impl Default for Clients {
    fn default() -> Self {
        Clients {
            clients: Mutex::new(HashMap::new()),
            last_activity: Mutex::new(Instant::now()),
        }
    }
}

impl Clients {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Client>> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Admits connection `id`, sent what `topics` names.
    pub fn add(
        &self,
        id: u64,
        out: Forwarder,
        ended: Arc<Notify>,
        topics: Option<Topics>,
        token: Option<String>,
    ) {
        let mut clients = self.lock();
        clients.insert(
            id,
            Client {
                out,
                ended,
                topics,
                token,
            },
        );
        info!(total = clients.len(), "a client connected");
    }

    pub fn remove(&self, id: u64) {
        let mut clients = self.lock();
        if clients.remove(&id).is_some() {
            info!(total = clients.len(), "a client disconnected");
        }
    }

    /// How many clients are admitted.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// What connection `id` is sent from now on.
    pub fn set_topics(&self, id: u64, topics: Option<Topics>) {
        if let Some(client) = self.lock().get_mut(&id) {
            client.topics = topics;
        }
    }

    /// Sends `method` with `params` to every client that wants it; `scope`
    /// is the session it is about, for clients that asked for one session's.
    pub fn broadcast(&self, method: &str, params: Value, scope: Option<&str>) {
        let clients = self.lock();
        let mut frame: Option<Message> = None;
        for client in clients.values() {
            if client
                .topics
                .as_ref()
                .is_some_and(|t| !t.wants(method, scope))
            {
                continue;
            }
            let msg = frame.get_or_insert_with(|| {
                let note = json!({ "jsonrpc": "2.0", "method": method, "params": params });
                Message::text(note.to_string())
            });
            client.out.send_message_now(msg.clone());
        }
    }

    /// A client said something: the server is in use.
    pub fn touch(&self) {
        *self.last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// How long since a client last said anything.
    pub fn quiet_for(&self) -> Duration {
        self.last_activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    /// Closes every socket that authenticated with device token `token`;
    /// answers how many.
    pub fn disconnect_token(&self, token: &str) -> usize {
        let clients = self.lock();
        let mut closed = 0;
        for client in clients
            .values()
            .filter(|c| c.token.as_deref() == Some(token))
        {
            client
                .out
                .send_message_now(close(CLOSE_CREDENTIAL_REJECTED, "token revoked"));
            client.ended.notify_one();
            closed += 1;
        }
        if closed > 0 {
            info!(token, closed, "closed the sockets of a revoked token");
        }
        closed
    }
}

/// A close frame with `code` and `reason`.
pub fn close(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::from(code),
        reason: reason.to_owned().into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streams::Streams;

    fn texts(conn: &mut crate::streams::ClientConn) -> Vec<Value> {
        conn.drain_now()
            .into_iter()
            .filter_map(|m| match m {
                Message::Text(t) => serde_json::from_str(t.as_str()).ok(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn reads_topics_as_the_server_did() {
        let t = Topics::of(&[
            "session:created".into(),
            "terminal:data#a".into(),
            "pairing:*".into(),
        ])
        .unwrap();
        assert!(t.wants("session:created", None));
        assert!(t.wants("terminal:data", Some("a")));
        assert!(!t.wants("terminal:data", Some("b")));
        assert!(t.wants("pairing:requested", None));
        assert!(!t.wants("config:changed", None));
        assert_eq!(Topics::of(&[]), None);
        assert_eq!(Topics::of(&[" ".into()]), None);
        assert_eq!(
            Topics::from_query(Some("x=1&topics=a%3Ab,c")),
            Topics::of(&["a:b".into(), "c".into()])
        );
        assert_eq!(Topics::from_query(Some("x=1")), None);
        assert_eq!(Topics::from_params(&json!({})), None);
        assert_eq!(
            Topics::from_params(&json!({ "topics": "nope" })),
            Some(None)
        );
        assert_eq!(Topics::from_params(&json!({ "topics": [1] })), Some(None));
        assert_eq!(
            Topics::from_params(&json!({ "topics": ["a"] })),
            Some(Topics::of(&["a".into()]))
        );
    }

    #[test]
    fn sends_each_client_what_it_asked_for() {
        let streams = Streams::new();
        let (mut all, mut some) = (streams.connect(), streams.connect());
        let clients = Clients::default();
        clients.add(1, all.forwarder(), Arc::default(), None, None);
        clients.add(
            2,
            some.forwarder(),
            Arc::default(),
            Topics::of(&["session:*".into()]),
            None,
        );
        clients.broadcast("config:changed", json!({}), None);
        clients.broadcast("session:updated", json!({ "id": "t" }), Some("t"));
        assert_eq!(texts(&mut all).len(), 2);
        assert_eq!(
            texts(&mut some),
            [json!({ "jsonrpc": "2.0", "method": "session:updated", "params": { "id": "t" } })]
        );
        clients.set_topics(2, None);
        clients.broadcast("config:changed", json!({}), None);
        assert_eq!(texts(&mut some).len(), 1);
        clients.remove(1);
        assert_eq!(clients.len(), 1);
    }

    #[test]
    fn closes_the_sockets_of_a_revoked_token() {
        let streams = Streams::new();
        let (mut a, mut b) = (streams.connect(), streams.connect());
        let clients = Clients::default();
        let ended: Arc<Notify> = Arc::default();
        clients.add(
            1,
            a.forwarder(),
            Arc::clone(&ended),
            None,
            Some("tok".into()),
        );
        clients.add(2, b.forwarder(), Arc::default(), None, Some("other".into()));
        assert_eq!(clients.disconnect_token("tok"), 1);
        assert!(matches!(
            a.drain_now().as_slice(),
            [Message::Close(Some(f))] if u16::from(f.code) == CLOSE_CREDENTIAL_REJECTED
        ));
        assert!(b.drain_now().is_empty());
        // Its read loop is told to stop too, whether or not the client closes.
        assert!(futures_util::FutureExt::now_or_never(ended.notified()).is_some());
    }

    #[test]
    fn keeps_when_a_client_last_spoke() {
        let clients = Clients::default();
        clients.touch();
        assert!(clients.quiet_for() < Duration::from_secs(5));
    }
}

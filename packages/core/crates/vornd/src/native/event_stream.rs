//! `events:subscribe`: a client's own queue on the public event bus.
//!
//! The same [`Bus`] the extensions and core modules read, with a queue of this
//! connection's own and the same coalescing ([`next_batch`]), so a slow client
//! costs the others nothing and sees what an extension would see. Events go
//! out as `events:event` notifications in one envelope, `{v, event, at,
//! session?, data}`, after the answer that says the queue is in place.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use vorn_extensions::events::{
    next_batch, Bus, Owner, AGENT_HOOK, GIT_CHANGED, SESSION_CREATED, SESSION_EXITED, SESSION_MARK,
    SESSION_PROGRAM, SESSION_STATUS, TASK_CREATED, TASK_STATUS_CHANGED,
};

use crate::streams::Forwarder;

/// The call that opens the stream.
pub const SUBSCRIBE: &str = "events:subscribe";
/// The notification each event arrives as.
pub const EVENT: &str = "events:event";
/// The envelope's version; a field is only ever added within one.
pub const VERSION: u8 = 1;

/// The events a client may ask for. Permission requests carry a tool's input, so they stay inside.
pub const STREAMED: &[&str] = &[
    SESSION_CREATED,
    SESSION_STATUS,
    SESSION_EXITED,
    SESSION_MARK,
    SESSION_PROGRAM,
    AGENT_HOOK,
    GIT_CHANGED,
    TASK_CREATED,
    TASK_STATUS_CHANGED,
];

/// How many events one batch takes off the queue.
const BATCH: usize = 256;

/// What a connection holds while it streams: leaving the bus when dropped.
#[derive(Debug)]
pub struct Subscription {
    bus: Arc<Bus>,
    owner: Owner,
    task: JoinHandle<()>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.task.abort();
        self.bus.unsubscribe(&self.owner);
    }
}

/// What a client asked to hear: the events by name, and one session's only.
#[derive(Debug, PartialEq, Eq)]
struct Wanted {
    events: Vec<&'static str>,
    session: Option<String>,
}

fn wanted(params: &Value) -> Result<Wanted, String> {
    let events = match params.get("events") {
        None | Some(Value::Null) => STREAMED.to_vec(),
        Some(Value::Array(names)) => {
            let mut chosen = Vec::with_capacity(names.len());
            for name in names {
                let name = name.as_str().unwrap_or_default();
                let known = STREAMED.iter().find(|e| **e == name).ok_or_else(|| {
                    format!("unknown event \"{name}\". Try: {}", STREAMED.join(", "))
                })?;
                if !chosen.contains(known) {
                    chosen.push(*known);
                }
            }
            if chosen.is_empty() {
                STREAMED.to_vec()
            } else {
                chosen
            }
        }
        Some(_) => return Err("events must be a list of names".to_owned()),
    };
    let session = match params.get("session") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => return Err("session must be an id".to_owned()),
    };
    Ok(Wanted { events, session })
}

/// One event in the envelope clients read.
fn envelope(name: &str, data: &Value, at: u64) -> Value {
    let mut out = json!({ "v": VERSION, "event": name, "at": at });
    if let Some(session) = data.get("sessionId").filter(|s| s.is_string()) {
        out["session"] = session.clone();
    }
    out["data"] = data.clone();
    json!({ "jsonrpc": "2.0", "method": EVENT, "params": out })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Puts a queue for connection `conn` on `bus` and starts forwarding to `reply`.
///
/// The queue is in place when this returns, so an event emitted after the
/// answer the caller sends next is not missed.
pub fn open(
    bus: &Arc<Bus>,
    conn: u64,
    reply: &Forwarder,
    params: &Value,
) -> Result<(Subscription, Vec<&'static str>), String> {
    let Wanted { events, session } = wanted(params)?;
    let name = format!("client-{conn}");
    let mut rx = bus.subscribe(&name, &events);
    let out = reply.clone();
    let task = tokio::spawn(async move {
        while let Some((batch, _)) = next_batch(&mut rx, BATCH).await {
            let at = now_ms();
            for event in batch {
                let sid = event.data.get("sessionId").and_then(Value::as_str);
                if session.as_deref().is_some_and(|wanted| sid != Some(wanted)) {
                    continue;
                }
                out.send(Message::text(
                    envelope(&event.name, &event.data, at).to_string(),
                ))
                .await;
            }
            if out.is_closed() {
                return;
            }
        }
    });
    let subscription = Subscription {
        bus: Arc::clone(bus),
        owner: Owner::Core(name),
        task,
    };
    Ok((subscription, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_everything_unless_told_otherwise() {
        let all = wanted(&json!({})).unwrap();
        assert_eq!(all.events, STREAMED);
        assert_eq!(all.session, None);
        assert_eq!(wanted(&json!({ "events": [] })).unwrap().events, STREAMED);
    }

    #[test]
    fn keeps_the_names_asked_for_once_and_refuses_the_rest() {
        let w = wanted(&json!({
            "events": ["session.exited", "session.created", "session.exited"],
            "session": "abc",
        }))
        .unwrap();
        assert_eq!(w.events, ["session.exited", "session.created"]);
        assert_eq!(w.session.as_deref(), Some("abc"));

        let err = wanted(&json!({ "events": ["agent.permission"] })).unwrap_err();
        assert!(err.starts_with("unknown event \"agent.permission\""));
        assert!(wanted(&json!({ "events": "session.created" })).is_err());
        assert!(wanted(&json!({ "session": 3 })).is_err());
    }

    #[test]
    fn names_the_session_in_the_envelope_when_the_event_has_one() {
        let e = envelope(
            SESSION_EXITED,
            &json!({ "sessionId": "s1", "exitCode": 3 }),
            9,
        );
        assert_eq!(e["params"]["v"], 1);
        assert_eq!(e["params"]["session"], "s1");
        assert_eq!(e["params"]["at"], 9);
        assert_eq!(e["params"]["data"]["exitCode"], 3);
        let git = envelope(GIT_CHANGED, &json!({ "worktreePath": "/w" }), 9);
        assert!(git["params"].get("session").is_none());
    }
}

//! What clients are told of the terminals and headless agents, from what
//! the sessions do and what the registry records (`ptyManager`'s and
//! `headlessManager`'s `client-message`s and `announceSession`).
//!
//! The session holder's effects move the records: a terminal's program
//! ending makes it idle and takes it out of the order, and offers the
//! worktree's cleanup when it was the last session there; a shell's cwd is
//! kept; a notification is told once, however often it is delivered. The
//! registry's notes then tell clients: a terminal new to them
//! (`session:created`), one that changed (`session:updated`), an order a
//! person set (`session:reordered`), and a headless agent's end
//! (`headless:exit`), whose record goes a while later. The app's server is
//! told how many sessions run (`vornd:live`), which keeps it from stopping as
//! idle. Each terminal's HEAD is read again every thirty seconds.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};
use vorn_engine::Effect;

use super::Native;
use crate::engine::{Engine, Event};
use crate::registry::{SessionRegistry, Stamp};

/// How long an ended headless agent's record is kept, as the server kept it.
const HEADLESS_KEPT: Duration = Duration::from_secs(30);
/// How often each terminal's HEAD is read again (`HEAD_REFRESH_MS`).
const HEAD_EVERY: Duration = Duration::from_secs(30);

/// Follows the engine and the registry until either goes.
pub async fn follow(native: Weak<Native>, engine: Arc<Engine>) {
    let registry = Arc::clone(engine.registry());
    let mut events = engine.subscribe();
    let mut notes = registry.subscribe();
    let mut told = Told::default();
    if let Some(snapshot) = registry.read(|r| {
        r.terminals()
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
    }) {
        told.terminals.extend(snapshot);
    }
    let mut heads = tokio::time::interval(HEAD_EVERY);
    heads.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let Some(n) = native.upgrade() else { return };
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(Event::Effect(fx, effect)) => effected(&n, &registry, &fx, effect),
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            note = notes.recv() => match note {
                Ok(note) => {
                    told.note(&n, &registry, &note);
                    told.live(&n, registry.live());
                }
                Err(RecvError::Lagged(missed)) => warn!(missed, "clients missed changes to the sessions"),
                Err(RecvError::Closed) => return,
            },
            _ = heads.tick() => refresh_heads(&n, &registry).await,
        }
    }
}

/// An effect of a session's program, for the terminal it is about.
fn effected(
    native: &Native,
    registry: &SessionRegistry,
    fx: &vorn_engine::EffectId,
    effect: Effect,
) {
    let id = fx.session.as_str();
    let terminal = registry.read(|r| r.terminal(id).is_some()).unwrap_or(false);
    match effect {
        Effect::Exit { code, signal } if terminal => {
            let code = i32::try_from(crate::streams::exit_code(code, signal)).unwrap_or(i32::MAX);
            let Some(offer) = registry.terminal_exit(id, code, Stamp::from(fx)) else {
                return;
            };
            // vornd's own clients hear it from the stream; a server's behind vornd, from this.
            if native.clients().is_none() {
                native.broadcast_to(
                    "terminal:exit",
                    json!({ "id": id, "exitCode": code }),
                    Some(id),
                );
            }
            if let Some(offer) = offer {
                native.broadcast_to("worktree:confirmCleanup", offer, Some(id));
            }
        }
        // Closed while it ran: its record is gone, and clients are told it ended.
        Effect::Exit { code, signal } if native.sessions.take_hung_up(id) => {
            let code = crate::streams::exit_code(code, signal);
            if native.clients().is_none() {
                native.broadcast_to(
                    "terminal:exit",
                    json!({ "id": id, "exitCode": code }),
                    Some(id),
                );
            }
        }
        Effect::Cwd(cwd) if terminal => registry.terminal_cwd(id, &cwd),
        Effect::Notify { title, body } => {
            let key = crate::control::effect_key(fx);
            if first_notice(native, &key) {
                let note = json!({ "id": id, "title": title, "body": body, "effectId": key });
                native.broadcast_to("terminal:notify", note, Some(id));
            }
        }
        _ => {}
    }
}

/// Whether notice `key` has not been shown before; a database that fails shows it.
fn first_notice(native: &Native, key: &str) -> bool {
    let Some(db) = native.database() else {
        return true;
    };
    let claimed = vorn_store::Store::open_beside(db)
        .ok()
        .flatten()
        .and_then(|mut s| s.call("claimEffect", json!([key, "notify", null])).ok());
    claimed.is_none_or(|v| v.as_bool() != Some(false))
}

/// What clients have been told of, to tell a new terminal from a changed one.
#[derive(Debug, Default)]
struct Told {
    terminals: HashSet<String>,
    exited: HashSet<String>,
    live: Value,
}

impl Told {
    /// Tells the app's server how many sessions run, when that changed.
    fn live(&mut self, native: &Native, live: Value) {
        if live == self.live {
            return;
        }
        if let Some(link) = native.link.get() {
            link.tell("vornd:live", live.clone());
        }
        self.live = live;
    }

    fn note(&mut self, native: &Native, registry: &Arc<SessionRegistry>, note: &Value) {
        let kind = note.get("kind").and_then(Value::as_str);
        match note.get("op").and_then(Value::as_str) {
            Some("snapshot") => {
                for t in note
                    .get("terminals")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = t.get("id").and_then(Value::as_str) {
                        self.terminals.insert(id.to_owned());
                    }
                }
            }
            Some("upsert") if kind == Some("terminal") => {
                let Some(record) = note.get("record") else {
                    return;
                };
                let Some(id) = record.get("id").and_then(Value::as_str) else {
                    return;
                };
                let shown = client_record(record);
                if self.terminals.insert(id.to_owned()) {
                    native.broadcast("session:created", shown);
                } else {
                    native.broadcast_to("session:updated", shown, Some(id));
                }
            }
            Some("remove") if kind == Some("terminal") => {
                if let Some(id) = note.get("id").and_then(Value::as_str) {
                    self.terminals.remove(id);
                }
            }
            Some("order") if note.get("reordered").and_then(Value::as_bool) == Some(true) => {
                native.broadcast(
                    "session:reordered",
                    note.get("order").cloned().unwrap_or(json!([])),
                );
            }
            Some("upsert") if kind == Some("headless") => {
                let Some(record) = note.get("record") else {
                    return;
                };
                let Some(id) = record.get("id").and_then(Value::as_str) else {
                    return;
                };
                if record.get("status").and_then(Value::as_str) == Some("exited")
                    && self.exited.insert(id.to_owned())
                {
                    let code = record.get("exitCode").cloned().unwrap_or(Value::Null);
                    native.broadcast_to(
                        "headless:exit",
                        json!({ "id": id, "exitCode": code }),
                        Some(id),
                    );
                    let (registry, id) = (Arc::downgrade(registry), id.to_owned());
                    tokio::spawn(async move {
                        tokio::time::sleep(HEADLESS_KEPT).await;
                        if let Some(registry) = registry.upgrade() {
                            registry.forget_headless(&id);
                        }
                    });
                }
            }
            Some("remove") if kind == Some("headless") => {
                if let Some(id) = note.get("id").and_then(Value::as_str) {
                    self.exited.remove(id);
                }
            }
            _ => {}
        }
    }
}

/// A record as clients are sent it: without the registry's own bookkeeping.
fn client_record(record: &Value) -> Value {
    let mut shown = record.clone();
    if let Some(map) = shown.as_object_mut() {
        for key in ["rev", "statusAt", "exitAt"] {
            map.remove(key);
        }
    }
    shown
}

/// Reads each live local terminal's HEAD, and keeps the ones that moved.
async fn refresh_heads(native: &Arc<Native>, registry: &Arc<SessionRegistry>) {
    let Some(dirs) = registry.read(|r| {
        r.live_terminals()
            .filter(|t| t.remote_host_id.is_none())
            .map(|t| {
                (
                    t.id.clone(),
                    t.worktree_path
                        .clone()
                        .unwrap_or_else(|| t.project_path.clone()),
                )
            })
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    if dirs.is_empty() {
        return;
    }
    let (n, r) = (Arc::clone(native), Arc::clone(registry));
    let read = tokio::task::spawn_blocking(move || {
        let git = super::remote::Place::Local.git(&n);
        for (id, dir) in dirs {
            if let Some(head) = git.head(Path::new(&dir)) {
                r.set_head_commit(&id, &head);
            }
        }
    });
    if let Err(err) = read.await {
        debug!(%err, "could not read the terminals' HEADs");
    }
}

#[cfg(test)]
mod tests {
    use super::super::sessions::tests::fed;
    use super::*;
    use vorn_engine::EffectId;

    /// The broadcasts and notes told to the app's server since the last look.
    fn told(rx: &mut tokio::sync::broadcast::Receiver<Value>) -> Vec<Value> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|mut v| {
                v.as_object_mut().map(|o| o.remove("jsonrpc"));
                v
            })
            .collect()
    }

    fn fx(session: &str, rseq: u64) -> EffectId {
        EffectId {
            session: session.to_owned(),
            epoch: 1,
            rseq,
            index: 0,
        }
    }

    #[test]
    fn tells_a_new_terminal_then_its_changes_and_an_order_a_person_set() {
        let fed = fed();
        let (mut rx, _listening) = fed.link.listen();
        let mut t = Told::default();
        let record = json!({ "id": "t", "status": "running", "rev": 4, "statusAt": [1, 2, 0] });
        let upsert = json!({ "op": "upsert", "kind": "terminal", "record": record });
        t.note(&fed.native, &fed.registry, &upsert);
        t.note(&fed.native, &fed.registry, &upsert);
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "order", "order": ["t"] }),
        );
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "order", "order": ["t"], "reordered": true }),
        );
        let shown = json!({ "id": "t", "status": "running" });
        assert_eq!(
            told(&mut rx),
            [
                json!({ "method": "vornd:broadcast", "params": { "method": "session:created", "params": shown } }),
                json!({ "method": "vornd:broadcast", "params": { "method": "session:updated", "params": shown, "scope": "t" } }),
                json!({ "method": "vornd:broadcast", "params": { "method": "session:reordered", "params": ["t"] } }),
            ]
        );
        // Gone and back: new to clients again.
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "remove", "kind": "terminal", "id": "t" }),
        );
        t.note(&fed.native, &fed.registry, &upsert);
        assert_eq!(
            told(&mut rx)[0]["params"]["method"],
            json!("session:created")
        );
    }

    #[test]
    fn tells_the_server_how_many_run_only_when_that_changes() {
        let fed = fed();
        let (mut rx, _listening) = fed.link.listen();
        let mut t = Told::default();
        t.live(&fed.native, fed.registry.live());
        t.live(&fed.native, fed.registry.live());
        assert_eq!(
            told(&mut rx),
            [json!({ "method": "vornd:live", "params": { "sessions": 3, "headless": 0 } })]
        );
    }

    #[test]
    fn an_ended_program_idles_its_terminal_and_offers_its_worktree_once() {
        let fed = fed();
        let (mut rx, _listening) = fed.link.listen();
        // `b` is idle already, so `a` ending leaves nothing at work in `/w`.
        let exit = Effect::Exit {
            code: Some(2),
            signal: None,
        };
        effected(&fed.native, &fed.registry, &fx("a", 5), exit.clone());
        effected(&fed.native, &fed.registry, &fx("a", 5), exit);
        assert_eq!(
            told(&mut rx),
            [
                json!({
                    "method": "vornd:broadcast",
                    "params": { "method": "terminal:exit", "params": { "id": "a", "exitCode": 2 }, "scope": "a" },
                }),
                json!({
                    "method": "vornd:broadcast",
                    "params": {
                        "method": "worktree:confirmCleanup",
                        "params": { "id": "a", "projectPath": "/p", "worktreePath": "/w" },
                        "scope": "a",
                    },
                }),
            ]
        );
        let a = fed
            .registry
            .read(|r| r.terminal("a").unwrap().0.clone())
            .unwrap();
        assert_eq!(a.status, crate::registry::AgentStatus::Idle);
        // A shell keeps where it went; an effect for no terminal changes nothing.
        effected(
            &fed.native,
            &fed.registry,
            &fx("sh", 6),
            Effect::Cwd("/tmp".into()),
        );
        effected(
            &fed.native,
            &fed.registry,
            &fx("x", 6),
            Effect::Cwd("/tmp".into()),
        );
        let sh = fed
            .registry
            .read(|r| r.terminal("sh").unwrap().0.clone())
            .unwrap();
        assert_eq!(sh.shell_cwd.as_deref(), Some("/tmp"));
        assert!(told(&mut rx).is_empty());
    }

    #[test]
    fn a_terminal_closed_while_it_ran_is_told_ended_once_its_program_is() {
        let fed = fed();
        assert_eq!(
            super::super::sessions::call(&fed.native, "terminal:kill", &json!("a")),
            super::super::Answer::Void
        );
        let (mut rx, _listening) = fed.link.listen();
        let exit = Effect::Exit {
            code: None,
            signal: Some(1),
        };
        effected(&fed.native, &fed.registry, &fx("a", 9), exit.clone());
        effected(&fed.native, &fed.registry, &fx("a", 9), exit);
        let code = crate::streams::exit_code(None, Some(1));
        assert_eq!(
            told(&mut rx),
            [json!({
                "method": "vornd:broadcast",
                "params": { "method": "terminal:exit", "params": { "id": "a", "exitCode": code }, "scope": "a" },
            })]
        );
    }

    #[test]
    fn a_notice_is_told_with_its_effect_id() {
        let fed = fed();
        let (mut rx, _listening) = fed.link.listen();
        let notify = Effect::Notify {
            title: "done".into(),
            body: "built".into(),
        };
        effected(&fed.native, &fed.registry, &fx("a", 7), notify);
        let key = crate::control::effect_key(&fx("a", 7));
        assert_eq!(
            told(&mut rx),
            [json!({
                "method": "vornd:broadcast",
                "params": {
                    "method": "terminal:notify",
                    "params": { "id": "a", "title": "done", "body": "built", "effectId": key },
                    "scope": "a",
                },
            })]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_headless_agents_end_is_told_once_and_its_record_goes_later() {
        let fed = fed();
        let (mut rx, _listening) = fed.link.listen();
        let mut t = Told::default();
        let ended = json!({
            "op": "upsert", "kind": "headless",
            "record": { "id": "h", "status": "exited", "exitCode": 3 },
        });
        t.note(&fed.native, &fed.registry, &ended);
        t.note(&fed.native, &fed.registry, &ended);
        assert_eq!(
            told(&mut rx),
            [json!({
                "method": "vornd:broadcast",
                "params": { "method": "headless:exit", "params": { "id": "h", "exitCode": 3 }, "scope": "h" },
            })]
        );
        tokio::time::sleep(HEADLESS_KEPT + Duration::from_secs(1)).await;
        assert!(told(&mut rx).is_empty());
    }
}

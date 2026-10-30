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
//! (`headless:exit`), whose record goes a while later. Each terminal's HEAD is
//! read again every thirty seconds.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};
use vorn_engine::Effect;
use vorn_extensions::events::{
    GIT_CHANGED, SESSION_ACTIVITY, SESSION_CREATED, SESSION_EXITED, SESSION_MARK, SESSION_PROGRAM,
    SESSION_STATUS, WORKTREE_IDLE,
};
use vorn_tree::view::CardStatus;

use super::Native;
use crate::engine::{Engine, Event};
use crate::registry::{EntryState, ProgramStatus, SessionRegistry, Stamp, WaitKind};

/// How long an ended headless agent's record is kept.
const HEADLESS_KEPT: Duration = Duration::from_secs(30);
/// How often each terminal's HEAD is read again (`HEAD_REFRESH_MS`).
const HEAD_EVERY: Duration = Duration::from_secs(30);
/// At most one notification a session's program status raises in this long.
const REPORT_COOLDOWN: Duration = Duration::from_secs(10);

/// Follows the engine and the registry until either goes.
pub async fn follow(native: Weak<Native>, engine: Arc<Engine>) {
    let registry = Arc::clone(engine.registry());
    let mut events = engine.subscribe();
    let mut notes = registry.subscribe();
    let mut told = Told::default();
    let mut reports = Reports::default();
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
                Ok(Event::Effect(fx, Effect::Program(p))) => {
                    program_event(&n, &fx.session, &p);
                    reports.heard(&n, &registry, &fx, &p, std::time::Instant::now());
                }
                Ok(Event::Effect(fx, effect)) => effected(&n, &registry, &fx, effect),
                Ok(Event::Activity(id)) => {
                    if at_work(&registry, &id) {
                        printed(&n, &id, u64::try_from(crate::registry::now_ms()).unwrap_or(0));
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            note = notes.recv() => match note {
                Ok(note) => {
                    told.note(&n, &registry, &note);
                }
                Err(RecvError::Lagged(missed)) => warn!(missed, "clients missed changes to the sessions"),
                Err(RecvError::Closed) => return,
            },
            _ = heads.tick() => refresh_heads(&n, &registry).await,
        }
    }
}

/// The last session at work in a worktree ended: `worktree.idle`, for whoever subscribed.
pub(super) fn worktree_idle(native: &Native, offer: &Value) {
    native.events().emit(WORKTREE_IDLE, || {
        json!({
            "sessionId": offer["id"],
            "projectPath": offer["projectPath"],
            "worktreePath": offer["worktreePath"],
        })
    });
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

            // Clients hear the exit itself from the stream.
            if let Some(offer) = offer {
                worktree_idle(native, &offer);
            }
        }
        Effect::Cwd(cwd) if terminal => registry.terminal_cwd(id, &cwd),
        Effect::Command(mark) if terminal => {
            let note = command_note(id, &mark);
            native.events().emit(SESSION_MARK, || {
                let cwd = registry
                    .read(|r| r.terminal(id).and_then(|(t, _)| t.shell_cwd.clone()))
                    .flatten();
                mark_event(&note, cwd)
            });
            native.broadcast_to("terminal:command", note, Some(id));
        }
        Effect::Notify { title, body } => {
            let key = crate::journal::effect_key(fx);
            if first_notice(native, &key) {
                let note = json!({ "id": id, "title": title, "body": body, "effectId": key });
                native.broadcast_to("terminal:notify", note, Some(id));
            }
        }
        _ => {}
    }
}

/// What each terminal's program last reported of itself, so that it
/// becoming blocked or finished is told once, at most once a cooldown.
#[derive(Debug, Default)]
struct Reports {
    states: HashMap<String, EntryState>,
    told: HashMap<String, std::time::Instant>,
}

impl Reports {
    fn heard(
        &mut self,
        native: &Native,
        registry: &SessionRegistry,
        fx: &vorn_engine::EffectId,
        p: &vorn_engine::Programs,
        now: std::time::Instant,
    ) {
        let id = fx.session.as_str();
        let terminal = registry.read(|r| r.terminal(id).is_some()).unwrap_or(false);
        let Some(body) = self.notice(id, p, now).filter(|_| terminal) else {
            return;
        };
        let key = crate::journal::effect_key(fx);
        if first_notice(native, &key) {
            let note = json!({ "id": id, "title": "", "body": body, "effectId": key });
            native.broadcast_to("terminal:notify", note, Some(id));
        }
    }

    /// The body of the notification `p` raises for session `id`, if any.
    fn notice(
        &mut self,
        id: &str,
        p: &vorn_engine::Programs,
        now: std::time::Instant,
    ) -> Option<String> {
        let status = ProgramStatus::read(p, None).filter(|s| s.native);
        let Some(state) = status.as_ref().and_then(ProgramStatus::state) else {
            self.states.remove(id);
            return None;
        };
        if self.states.insert(id.to_owned(), state) == Some(state) {
            return None;
        }
        let status = status.as_ref()?;
        let msg = status.message().map(str::to_owned);
        let body = match state {
            EntryState::Blocked => {
                let wants = match status.waiting_for() {
                    Some(WaitKind::Permission) => "Needs permission",
                    Some(WaitKind::Question) => "Has a question",
                    Some(WaitKind::Auth) => "Needs you to sign in",
                    None => "Needs your input",
                };
                msg.map_or_else(|| wants.to_owned(), |m| format!("{wants}: {m}"))
            }
            EntryState::Done => {
                msg.map_or_else(|| "Finished".to_owned(), |m| format!("Finished: {m}"))
            }
            _ => return None,
        };
        if self
            .told
            .get(id)
            .is_some_and(|&at| now.saturating_duration_since(at) < REPORT_COOLDOWN)
        {
            return None;
        }
        self.told.insert(id.to_owned(), now);
        Some(body)
    }
}

/// A terminal's project and worktree, the worktree being the project when it has none of its own.
fn place_of(t: &crate::registry::TerminalSession) -> (String, String) {
    let worktree = t
        .worktree_path
        .clone()
        .unwrap_or_else(|| t.project_path.clone());
    (t.project_path.clone(), worktree)
}

/// `session.mark` from what clients hear of the mark: the same fields, the session named as events name it.
fn mark_event(note: &Value, cwd: Option<String>) -> Value {
    let mut event = note.clone();
    if let Some(map) = event.as_object_mut() {
        if let Some(id) = map.shift_remove("id") {
            map.insert("sessionId".into(), id);
        }
        if let Some(cwd) = cwd {
            map.insert("cwd".into(), Value::String(cwd));
        }
    }
    event
}

/// `session.program`: what a session's program now reports of itself (OSC 7501).
fn program_event(native: &Native, id: &str, p: &vorn_engine::Programs) {
    native.events().emit(SESSION_PROGRAM, || {
        let status = ProgramStatus::read(p, None);
        json!({
            "sessionId": id,
            "state": status.as_ref().and_then(ProgramStatus::state),
            "message": status.as_ref().and_then(ProgramStatus::message),
            "waitingFor": status.as_ref().and_then(ProgramStatus::waiting_for),
        })
    });
}

/// What clients hear of a shell's prompt or command mark, stamped when vornd read it.
fn command_note(id: &str, mark: &vorn_engine::Mark) -> Value {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    match mark {
        vorn_engine::Mark::Prompt => json!({ "id": id, "phase": "prompt", "at": at }),
        vorn_engine::Mark::Start { command } => {
            json!({ "id": id, "phase": "start", "command": command, "at": at })
        }
        vorn_engine::Mark::End {
            exit_code,
            duration_ms,
        } => json!({
            "id": id,
            "phase": "end",
            "exitCode": exit_code,
            "durationMs": duration_ms,
            "at": at,
        }),
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
    headless: HashSet<String>,
    exited: HashSet<String>,
    /// Each session's status as last told in `session.status`.
    statuses: HashMap<String, String>,
    /// Each terminal's project and worktree, for its `session.exited` after its record goes.
    places: HashMap<String, (Value, Value)>,
}

/// A record's project and worktree, the worktree being the project when it has none of its own.
fn place_in(record: &Value) -> (Value, Value) {
    let project = record.get("projectPath").cloned().unwrap_or(Value::Null);
    let worktree = match record.get("worktreePath") {
        None | Some(Value::Null) => project.clone(),
        Some(w) => w.clone(),
    };
    (project, worktree)
}

/// `session.activity`: what session `id` just did, for subscribers that declare `sessions.events`.
fn activity(native: &Native, id: &str, what: CardStatus) {
    if !native.activity_news(id, what) {
        return;
    }
    native.events().emit(
        SESSION_ACTIVITY,
        || json!({ "sessionId": id, "activity": what.key() }),
    );
}

/// Whether session `id` is still at work: output that arrives after it ended is not a burst.
fn at_work(registry: &SessionRegistry, id: &str) -> bool {
    registry
        .read(|r| {
            r.terminal(id).is_some_and(|(_, ended)| !ended)
                || r.headless_record(id)
                    .is_some_and(|h| h.status != crate::registry::HeadlessStatus::Exited)
        })
        .unwrap_or(false)
}

/// Session `id` printed at `now` (epoch ms): `session.activity` once per burst.
fn printed(native: &Native, id: &str, now: u64) {
    if native.printed(id, now) {
        activity(native, id, CardStatus::Printing);
    }
}

/// `session.created` for a record clients were just told of.
fn created_event(native: &Native, kind: &str, record: &Value) {
    native.events().emit(SESSION_CREATED, || {
        let text = |k: &str| record.get(k).cloned().unwrap_or(Value::Null);
        let (project, worktree) = place_in(record);
        let mut data = json!({
            "sessionId": text("id"), "kind": kind, "projectPath": project,
            "worktreePath": worktree, "agent": text("agentType"),
        });
        let model = record
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| native.model_of(id, false));
        if let Some(model) = model {
            data["model"] = json!(model);
        }
        data
    });
    if let Some(id) = record.get("id").and_then(Value::as_str) {
        activity(native, id, CardStatus::Started);
    }
}

impl Told {
    /// `session.exited` for terminal `id`, once, whether it ended or was closed first.
    fn terminal_exited(&mut self, native: &Native, id: &str, code: Value) {
        native.model_of(id, true);
        let Some((project, worktree)) = self.places.remove(id) else {
            return;
        };
        activity(native, id, CardStatus::Finished);
        native.printed_at(id, true);
        native.events().emit(SESSION_EXITED, || {
            json!({
                "sessionId": id, "kind": "terminal", "exitCode": code,
                "projectPath": project, "worktreePath": worktree,
            })
        });
    }

    /// `session.status` when `record`'s status moved since it was last told.
    fn status(&mut self, native: &Native, id: &str, record: &Value) {
        let Some(status) = record.get("status").and_then(Value::as_str) else {
            return;
        };
        let from = self.statuses.insert(id.to_owned(), status.to_owned());
        if from.as_deref() == Some(status) {
            return;
        }
        native.events().emit(
            SESSION_STATUS,
            || json!({ "sessionId": id, "status": status }),
        );
        if let Some(what) = vorn_tree::view::moved(from.as_deref(), status) {
            activity(native, id, what);
        }
    }
}

impl Told {
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
                        self.places.insert(id.to_owned(), place_in(t));
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
                    self.places.insert(id.to_owned(), place_in(record));
                    created_event(native, "terminal", record);
                    native.broadcast("session:created", shown);
                } else {
                    native.broadcast_to("session:updated", shown, Some(id));
                }
                self.status(native, id, record);
                if let Some(code) = note.get("exited") {
                    self.terminal_exited(native, id, code.clone());
                }
            }
            Some("remove") if kind == Some("terminal") => {
                if let Some(id) = note.get("id").and_then(Value::as_str) {
                    if self.terminals.remove(id) {
                        // Every client drops it, not only the one that closed it.
                        native.broadcast("session:removed", json!({ "id": id }));
                    }
                    self.statuses.remove(id);
                    self.terminal_exited(native, id, Value::Null);
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
                if self.headless.insert(id.to_owned()) {
                    created_event(native, "headless", record);
                }
                self.status(native, id, record);
                if record.get("status").and_then(Value::as_str) == Some("exited")
                    && self.exited.insert(id.to_owned())
                {
                    let code = record.get("exitCode").cloned().unwrap_or(Value::Null);
                    activity(native, id, CardStatus::Finished);
                    native.printed_at(id, true);
                    native.events().emit(SESSION_EXITED, || {
                        let (project, worktree) = place_in(record);
                        json!({
                            "sessionId": id, "kind": "headless", "exitCode": code,
                            "projectPath": project, "worktreePath": worktree,
                        })
                    });
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
                    self.headless.remove(id);
                    self.statuses.remove(id);
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
                let (project, worktree) = place_of(t);
                (t.id.clone(), project, worktree)
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
        for (id, project, dir) in dirs {
            let Some(head) = git.head(Path::new(&dir)) else {
                continue;
            };
            if r.set_head_commit(&id, &head) {
                n.events().emit(
                    GIT_CHANGED,
                    || json!({ "projectPath": project, "worktreePath": dir, "head": head }),
                );
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

    /// What a client was told since the last look.
    fn told(rx: &mut super::super::sessions::tests::Notes) -> Vec<Value> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
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
        let mut rx = fed.notes();
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
                json!({ "method": "session:created", "params": shown }),
                json!({ "method": "session:updated", "params": shown }),
                json!({ "method": "session:reordered", "params": ["t"] }),
            ]
        );
        // Gone and back: new to clients again.
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "remove", "kind": "terminal", "id": "t" }),
        );
        t.note(&fed.native, &fed.registry, &upsert);
        let methods: Vec<Value> = told(&mut rx).iter().map(|n| n["method"].clone()).collect();
        assert_eq!(
            methods[..2],
            [json!("session:removed"), json!("session:created")]
        );
    }

    #[test]
    fn tells_every_client_a_closed_terminal_is_gone_once() {
        let fed = fed();
        let mut rx = fed.notes();
        let mut t = Told::default();
        let upsert = json!({ "op": "upsert", "kind": "terminal", "record": { "id": "t", "status": "running" } });
        let remove = json!({ "op": "remove", "kind": "terminal", "id": "t" });
        t.note(&fed.native, &fed.registry, &upsert);
        told(&mut rx);
        t.note(&fed.native, &fed.registry, &remove);
        t.note(&fed.native, &fed.registry, &remove);
        let removed: Vec<Value> = told(&mut rx)
            .into_iter()
            .filter(|n| n["method"] == "session:removed")
            .collect();
        assert_eq!(
            removed,
            [json!({ "method": "session:removed", "params": { "id": "t" } })]
        );
    }

    #[tokio::test]
    async fn tells_subscribers_a_terminal_started_moved_and_ended_once_each() {
        let fed = fed();
        let mut events = fed.native.events().subscribe(
            "test",
            &[
                SESSION_CREATED,
                SESSION_STATUS,
                SESSION_EXITED,
                SESSION_MARK,
            ],
        );
        let mut t = Told::default();
        let record = |status: &str| json!({ "id": "t", "status": status, "projectPath": "/p", "agentType": "acme" });
        let upsert =
            |status: &str| json!({ "op": "upsert", "kind": "terminal", "record": record(status) });
        t.note(&fed.native, &fed.registry, &upsert("running"));
        t.note(&fed.native, &fed.registry, &upsert("running"));
        t.note(&fed.native, &fed.registry, &upsert("waiting"));
        let mut exited = upsert("idle");
        exited["exited"] = json!(2);
        t.note(&fed.native, &fed.registry, &exited);
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "remove", "kind": "terminal", "id": "t" }),
        );
        let mark = Effect::Command(vorn_engine::Mark::Prompt);
        effected(&fed.native, &fed.registry, &fx("sh", 3), mark);
        let mut seen = Vec::new();
        while let Ok(e) = events.try_recv() {
            seen.push((e.name.clone(), e.data.clone()));
        }
        let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                SESSION_CREATED,
                SESSION_STATUS,
                SESSION_STATUS,
                SESSION_STATUS,
                SESSION_EXITED,
                SESSION_MARK
            ]
        );
        assert_eq!(
            seen[0].1,
            json!({ "sessionId": "t", "kind": "terminal", "projectPath": "/p", "worktreePath": "/p", "agent": "acme" })
        );
        assert_eq!(seen[4].1["exitCode"], 2);
        assert_eq!(seen[5].1["sessionId"], "sh");
        assert_eq!(seen[5].1["phase"], "prompt");
    }

    #[tokio::test]
    async fn activity_is_told_once_per_burst_and_output_never_moves_a_status() {
        let fed = fed();
        let mut events = fed
            .native
            .events()
            .subscribe("test", &[SESSION_ACTIVITY, SESSION_STATUS]);
        let mut t = Told::default();
        let upsert = |status: &str| json!({ "op": "upsert", "kind": "terminal", "record": { "id": "t", "status": status, "projectPath": "/p" } });
        t.note(&fed.native, &fed.registry, &upsert("idle"));
        printed(&fed.native, "t", 10_000);
        printed(&fed.native, "t", 11_000);
        printed(&fed.native, "t", 12_500);
        printed(&fed.native, "t", 12_500 + vorn_tree::view::QUIET_MS);
        t.note(&fed.native, &fed.registry, &upsert("running"));
        t.note(&fed.native, &fed.registry, &upsert("waiting"));
        t.note(&fed.native, &fed.registry, &upsert("running"));
        t.note(&fed.native, &fed.registry, &upsert("idle"));
        assert_eq!(fed.native.printed_at("t", false), Some(14_500));
        t.note(
            &fed.native,
            &fed.registry,
            &json!({ "op": "remove", "kind": "terminal", "id": "t" }),
        );
        let mut seen = Vec::new();
        while let Ok(e) = events.try_recv() {
            seen.push(match e.name.as_str() {
                SESSION_ACTIVITY => format!("activity:{}", e.data["activity"].as_str().unwrap()),
                _ => format!("status:{}", e.data["status"].as_str().unwrap()),
            });
        }
        assert_eq!(
            seen,
            [
                "activity:started",
                "status:idle",
                "activity:printing",
                "activity:printing",
                "status:running",
                "status:waiting",
                "activity:waiting",
                "status:running",
                "status:idle",
                "activity:finished",
            ]
        );
        assert_eq!(
            fed.native.printed_at("t", false),
            None,
            "its bursts go with it"
        );
    }

    #[test]
    fn a_programs_own_report_is_an_event() {
        use vorn_engine::{BlockedKind, ProgramRecord, ProgramState, Programs};
        let fed = fed();
        let mut events = fed.native.events().subscribe("test", &[SESSION_PROGRAM]);
        let blocked = Programs {
            native: true,
            records: vec![ProgramRecord {
                id: String::new(),
                state: ProgramState::Blocked,
                kind: Some(BlockedKind::Permission),
                progress: None,
                app: "agent".into(),
                title: String::new(),
                msg: "May I?".into(),
                from_progress: false,
            }],
        };
        program_event(&fed.native, "a", &blocked);
        let e = events.try_recv().expect("an event");
        assert_eq!(
            e.data,
            json!({ "sessionId": "a", "state": "blocked", "message": "May I?", "waitingFor": "permission" })
        );
    }

    #[test]
    fn output_after_a_terminal_ended_starts_no_burst() {
        let fed = fed();
        assert!(at_work(&fed.registry, "a"));
        let exit = Effect::Exit {
            code: Some(0),
            signal: None,
        };
        effected(&fed.native, &fed.registry, &fx("a", 5), exit);
        assert!(!at_work(&fed.registry, "a"), "a late print is not news");
        assert!(!at_work(&fed.registry, "nobody"));
    }

    #[test]
    fn an_ended_program_idles_its_terminal_and_offers_its_worktree_once() {
        let fed = fed();
        let mut rx = fed.notes();
        let mut idle = fed.native.events().subscribe("test", &[WORKTREE_IDLE]);
        // `b` is idle already, so `a` ending leaves nothing at work in `/w`.
        let exit = Effect::Exit {
            code: Some(2),
            signal: None,
        };
        effected(&fed.native, &fed.registry, &fx("a", 5), exit.clone());
        effected(&fed.native, &fed.registry, &fx("a", 5), exit);
        assert!(told(&mut rx).is_empty());
        assert_eq!(
            idle.try_recv().unwrap().data,
            json!({ "sessionId": "a", "projectPath": "/p", "worktreePath": "/w" })
        );
        assert!(idle.try_recv().is_err());
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

    fn report(
        state: vorn_engine::ProgramState,
        kind: Option<vorn_engine::BlockedKind>,
        msg: &str,
    ) -> vorn_engine::Programs {
        vorn_engine::Programs {
            native: true,
            records: vec![vorn_engine::ProgramRecord {
                id: String::new(),
                state,
                kind,
                progress: None,
                app: String::new(),
                title: String::new(),
                msg: msg.into(),
                from_progress: false,
            }],
        }
    }

    #[test]
    fn blocked_and_done_reports_notify_once_each_within_a_cooldown() {
        use vorn_engine::{BlockedKind, ProgramState};
        let mut r = Reports::default();
        let t0 = std::time::Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let blocked = report(
            ProgramState::Blocked,
            Some(BlockedKind::Auth),
            "Sign in to deploy",
        );
        assert_eq!(
            r.notice("a", &report(ProgramState::Working, None, ""), at(0)),
            None
        );
        assert_eq!(
            r.notice("a", &blocked, at(1)).as_deref(),
            Some("Needs you to sign in: Sign in to deploy")
        );
        // Told again, the same state: nothing new.
        assert_eq!(r.notice("a", &blocked, at(2)), None);
        assert_eq!(
            r.notice("a", &report(ProgramState::Working, None, ""), at(3)),
            None
        );
        // Finished within the cooldown: not told.
        assert_eq!(
            r.notice("a", &report(ProgramState::Done, None, "Built"), at(4)),
            None
        );
        assert_eq!(
            r.notice("a", &report(ProgramState::Working, None, ""), at(20)),
            None
        );
        assert_eq!(
            r.notice("a", &report(ProgramState::Done, None, "Built"), at(21))
                .as_deref(),
            Some("Finished: Built")
        );
        // Another session counts on its own.
        assert_eq!(
            r.notice(
                "b",
                &report(ProgramState::Blocked, Some(BlockedKind::Permission), ""),
                at(21)
            )
            .as_deref(),
            Some("Needs permission")
        );
    }

    #[test]
    fn a_notice_is_told_with_its_effect_id() {
        let fed = fed();
        let mut rx = fed.notes();
        let notify = Effect::Notify {
            title: "done".into(),
            body: "built".into(),
        };
        effected(&fed.native, &fed.registry, &fx("a", 7), notify);
        let key = crate::journal::effect_key(&fx("a", 7));
        assert_eq!(
            told(&mut rx),
            [json!({
                "method": "terminal:notify",
                "params": { "id": "a", "title": "done", "body": "built", "effectId": key },
            })]
        );
    }

    #[test]
    fn a_shells_command_marks_are_told_to_its_viewers() {
        let fed = fed();
        let mut rx = fed.notes();
        let start = Effect::Command(vorn_engine::Mark::Start {
            command: Some("ls".into()),
        });
        effected(&fed.native, &fed.registry, &fx("sh", 8), start.clone());
        effected(&fed.native, &fed.registry, &fx("x", 8), start);
        let end = Effect::Command(vorn_engine::Mark::End {
            exit_code: Some(1),
            duration_ms: None,
        });
        effected(&fed.native, &fed.registry, &fx("sh", 9), end);
        let mut notes = told(&mut rx);
        for n in &mut notes {
            assert!(n["params"]["at"].as_u64().is_some_and(|at| at > 0), "{n}");
            n["params"].as_object_mut().unwrap().remove("at");
        }
        assert_eq!(
            notes,
            [
                json!({
                    "method": "terminal:command",
                    "params": { "id": "sh", "phase": "start", "command": "ls" },
                }),
                json!({
                    "method": "terminal:command",
                    "params": { "id": "sh", "phase": "end", "exitCode": 1, "durationMs": null },
                }),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_headless_agents_end_is_told_once_and_its_record_goes_later() {
        let fed = fed();
        let mut rx = fed.notes();
        let mut t = Told::default();
        let ended = json!({
            "op": "upsert", "kind": "headless",
            "record": { "id": "h", "status": "exited", "exitCode": 3 },
        });
        t.note(&fed.native, &fed.registry, &ended);
        t.note(&fed.native, &fed.registry, &ended);
        assert_eq!(
            told(&mut rx),
            [json!({ "method": "headless:exit", "params": { "id": "h", "exitCode": 3 } })]
        );
        tokio::time::sleep(HEADLESS_KEPT + Duration::from_secs(1)).await;
        assert!(told(&mut rx).is_empty());
    }
}

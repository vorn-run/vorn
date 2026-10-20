//! Public events: what the core tells whoever subscribed, extensions and the
//! core's own modules alike, and nobody else.
//!
//! A subscriber names the events it wants; the [`Bus`] keeps an index from
//! each name to its subscribers' queues, so an event nobody subscribed to
//! costs one lookup and its data is never built. Each subscriber has one
//! queue, read by one task, so it receives every event once and in the order
//! it was emitted.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use serde_json::Value;
use tokio::sync::{mpsc, Notify};
use tracing::warn;

use crate::api::types::Event;

/// A session started: `{sessionId, kind, projectPath, worktreePath, agent, model?}`; `model` when one was chosen.
pub const SESSION_CREATED: &str = "session.created";
/// A session's status changed: `{sessionId, status}`; only the latest per session in a batch.
pub const SESSION_STATUS: &str = "session.status";
/// A session's program ended: `{sessionId, kind, projectPath, exitCode}`.
pub const SESSION_EXITED: &str = "session.exited";
/// A shell's OSC 133 mark: `{sessionId, phase, at, cwd?, command?, exitCode?, durationMs?}`; `at` in epoch ms, `cwd` the folder the shell last reported.
pub const SESSION_MARK: &str = "session.mark";
/// What a session's program reports of itself (OSC 7501): `{sessionId, state, message?, waitingFor?}`.
pub const SESSION_PROGRAM: &str = "session.program";
/// An agent's hook fired: `{sessionId, event, tool?}`.
pub const AGENT_HOOK: &str = "agent.hook";
/// A terminal's agent started a conversation: `{sessionId, conversationId}`.
pub const AGENT_CONVERSATION: &str = "agent.conversation";
/// A task was created: the trigger it fires, `{effectId, kind, task}`.
pub const TASK_CREATED: &str = "task.created";
/// A task's status moved: the trigger it fires, `{effectId, kind, task, from, to}`.
pub const TASK_STATUS_CHANGED: &str = "task.statusChanged";
/// An agent asks to use a tool: `{requestId, sessionId, tool, input, description?}`.
pub const AGENT_PERMISSION: &str = "agent.permission";
/// A permission request was answered or withdrawn: `{requestId, sessionId}`.
pub const AGENT_PERMISSION_CLOSED: &str = "agent.permissionClosed";
/// The last session in a worktree ended: `{projectPath, worktreePath, sessionId}`.
pub const WORKTREE_IDLE: &str = "worktree.idle";
/// The workflows changed: one was made, edited, switched or removed: `{}`.
pub const WORKFLOW_CHANGED: &str = "workflow.changed";
/// A run's approval gates waiting for a person changed: `{runId, gates}`, each gate
/// `{id, title, subtitle, message, workflow, run, node, at}`.
pub const WORKFLOW_GATES: &str = "workflow.gates";
/// A worktree's HEAD moved or vornd committed in it: `{worktreePath, projectPath?, head?}`; only the latest per worktree in a batch.
pub const GIT_CHANGED: &str = "git.changed";

/// Every event the core emits, which a manifest may subscribe to by name.
pub const CORE_EVENTS: &[&str] = &[
    SESSION_CREATED,
    SESSION_STATUS,
    SESSION_EXITED,
    SESSION_MARK,
    SESSION_PROGRAM,
    AGENT_HOOK,
    AGENT_CONVERSATION,
    AGENT_PERMISSION,
    AGENT_PERMISSION_CLOSED,
    WORKTREE_IDLE,
    GIT_CHANGED,
    TASK_CREATED,
    TASK_STATUS_CHANGED,
    WORKFLOW_CHANGED,
    WORKFLOW_GATES,
];

/// The namespaces the core's events live in; another extension's events live in its own.
pub const CORE_NAMESPACES: &[&str] = &[
    "session.",
    "agent.",
    "git.",
    "task.",
    "worktree.",
    "workflow.",
];

/// Whether `name` is in a core namespace without being a core event: a typo a manifest should hear about.
pub fn unknown_core_event(name: &str) -> bool {
    CORE_NAMESPACES.iter().any(|ns| name.starts_with(ns)) && !CORE_EVENTS.contains(&name)
}

/// How many events wait for one subscriber before newer ones are dropped.
pub const QUEUE: usize = 4096;

/// Who a queue belongs to, for the log line when it overflows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Owner {
    /// One of the core's own modules, by name.
    Core(String),
    /// An extension, by id.
    Extension(String),
}

/// How far a queue's reader is behind it: what was queued against what it handled.
#[derive(Debug, Default)]
pub struct Tally {
    queued: AtomicU64,
    handled: AtomicU64,
    moved: Notify,
}

impl Tally {
    /// The reader handled `n` more events.
    pub fn handled(&self, n: usize) {
        self.handled
            .fetch_add(u64::try_from(n).unwrap_or(u64::MAX), Ordering::SeqCst);
        self.moved.notify_waiters();
    }

    /// Resolves once every event queued before the call has been handled.
    pub async fn settled(&self) {
        let want = self.queued.load(Ordering::SeqCst);
        loop {
            let moved = self.moved.notified();
            if self.handled.load(Ordering::SeqCst) >= want {
                return;
            }
            moved.await;
        }
    }
}

/// A subscriber's queue, and its tally when someone waits on it.
#[derive(Debug, Clone)]
pub struct Queue {
    pub tx: mpsc::Sender<Arc<Event>>,
    pub tally: Option<Arc<Tally>>,
}

#[derive(Debug, Clone)]
struct Sink {
    owner: Owner,
    queue: Queue,
}

/// Event name to the queues of whoever subscribed to it.
#[derive(Debug, Default)]
struct Index {
    by_name: HashMap<String, Vec<Sink>>,
}

impl Index {
    fn add(&mut self, names: &[String], sink: &Sink) {
        for name in names {
            let sinks = self.by_name.entry(name.clone()).or_default();
            if !sinks.iter().any(|s| s.owner == sink.owner) {
                sinks.push(sink.clone());
            }
        }
    }

    fn remove_where(&mut self, gone: impl Fn(&Owner) -> bool) {
        for sinks in self.by_name.values_mut() {
            sinks.retain(|s| !gone(&s.owner));
        }
        self.by_name.retain(|_, sinks| !sinks.is_empty());
    }
}

/// The public events and who subscribed to each.
#[derive(Debug, Default)]
pub struct Bus {
    index: RwLock<Index>,
}

impl Bus {
    pub fn new() -> Arc<Bus> {
        Arc::new(Bus::default())
    }

    /// Whether anyone subscribed to `name`: the check before building an event's data.
    pub fn wants(&self, name: &str) -> bool {
        self.read(|i| i.by_name.contains_key(name))
    }

    /// Queues `name` for every subscriber, building its data only when there is one; answers how many it reached.
    pub fn emit(&self, name: &str, data: impl FnOnce() -> Value) -> usize {
        let sinks = self.read(|i| i.by_name.get(name).cloned());
        let Some(sinks) = sinks else {
            return 0;
        };
        let event = Arc::new(Event {
            name: name.to_owned(),
            data: data(),
        });
        let mut reached = 0;
        for sink in sinks {
            match sink.queue.tx.try_send(Arc::clone(&event)) {
                Ok(()) => {
                    if let Some(tally) = &sink.queue.tally {
                        tally.queued.fetch_add(1, Ordering::SeqCst);
                    }
                    reached += 1;
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(owner = ?sink.owner, event = name, "an event queue is full; the event is dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
        reached
    }

    /// A queue of `names` for one of the core's own modules.
    pub fn subscribe(&self, owner: &str, names: &[&str]) -> mpsc::Receiver<Arc<Event>> {
        let (tx, rx) = mpsc::channel(QUEUE);
        let names: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        let queue = Queue { tx, tally: None };
        self.subscribe_with(Owner::Core(owner.to_owned()), &names, queue);
        rx
    }

    /// Adds `queue` as `owner`'s for `names`, beside whatever it already had.
    pub fn subscribe_with(&self, owner: Owner, names: &[String], queue: Queue) {
        let sink = Sink { owner, queue };
        self.write(|i| i.add(names, &sink));
    }

    /// Drops every queue `owner` holds.
    pub fn unsubscribe(&self, owner: &Owner) {
        self.write(|i| i.remove_where(|o| o == owner));
    }

    /// Replaces every extension's subscriptions with `subscribed`: each id's
    /// event names and queue. A queue kept from before keeps its order.
    pub fn set_extensions(&self, subscribed: Vec<(String, Vec<String>, Queue)>) {
        self.write(|i| {
            i.remove_where(|o| matches!(o, Owner::Extension(_)));
            for (id, names, queue) in subscribed {
                i.add(
                    &names,
                    &Sink {
                        owner: Owner::Extension(id),
                        queue,
                    },
                );
            }
        });
    }

    /// The ids of the extensions subscribed to `name`.
    pub fn extensions_for(&self, name: &str) -> Vec<String> {
        self.read(|i| {
            i.by_name
                .get(name)
                .into_iter()
                .flatten()
                .filter_map(|s| match &s.owner {
                    Owner::Extension(id) => Some(id.clone()),
                    Owner::Core(_) => None,
                })
                .collect()
        })
    }

    fn read<T>(&self, f: impl FnOnce(&Index) -> T) -> T {
        f(&self.index.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn write<T>(&self, f: impl FnOnce(&mut Index) -> T) -> T {
        f(&mut self.index.write().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The key an event coalesces by: a later one with the same key replaces it within a batch.
fn coalesce_key(event: &Event) -> Option<(&str, &str)> {
    let field = match event.name.as_str() {
        SESSION_STATUS => "sessionId",
        GIT_CHANGED => "worktreePath",
        _ => return None,
    };
    event
        .data
        .get(field)
        .and_then(Value::as_str)
        .map(|v| (event.name.as_str(), v))
}

/// Drops each coalescing event a later one in `batch` replaces, keeping the rest in order.
pub fn coalesce(batch: &mut Vec<Arc<Event>>) {
    let mut last: HashMap<(&str, &str), usize> = HashMap::new();
    for (at, event) in batch.iter().enumerate() {
        if let Some(key) = coalesce_key(event) {
            last.insert(key, at);
        }
    }
    if last.is_empty() {
        return;
    }
    let keep: Vec<bool> = batch
        .iter()
        .enumerate()
        .map(|(at, e)| coalesce_key(e).is_none_or(|k| last.get(&k) == Some(&at)))
        .collect();
    let mut flags = keep.into_iter();
    batch.retain(|_| flags.next().unwrap_or(true));
}

/// Waits for the next event on `rx`, then takes what else is already queued, up to `max`: one
/// batch, coalesced, and how many events it took off the queue.
pub async fn next_batch(
    rx: &mut mpsc::Receiver<Arc<Event>>,
    max: usize,
) -> Option<(Vec<Arc<Event>>, usize)> {
    let first = rx.recv().await?;
    let mut batch = vec![first];
    while batch.len() < max {
        match rx.try_recv() {
            Ok(event) => batch.push(event),
            Err(_) => break,
        }
    }
    let taken = batch.len();
    coalesce(&mut batch);
    Some((batch, taken))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    fn names(batch: &[Arc<Event>]) -> Vec<String> {
        batch
            .iter()
            .map(|e| format!("{}:{}", e.name, e.data))
            .collect()
    }

    #[test]
    fn builds_nothing_for_an_event_nobody_wants() {
        let bus = Bus::new();
        let built = Cell::new(false);
        assert_eq!(
            bus.emit(SESSION_MARK, || {
                built.set(true);
                Value::Null
            }),
            0
        );
        assert!(!built.get());
        assert!(!bus.wants(SESSION_MARK));
    }

    #[tokio::test]
    async fn each_subscriber_gets_each_event_once_in_order() {
        let bus = Bus::new();
        let mut a = bus.subscribe("a", &[SESSION_MARK, SESSION_EXITED]);
        let mut b = bus.subscribe("b", &[SESSION_EXITED]);
        for n in 0..5 {
            bus.emit(SESSION_MARK, || json!(n));
        }
        assert_eq!(bus.emit(SESSION_EXITED, || json!("x")), 2);
        let (got, taken) = next_batch(&mut a, 100).await.unwrap();
        assert_eq!(taken, 6);
        assert_eq!(
            names(&got),
            [
                "session.mark:0",
                "session.mark:1",
                "session.mark:2",
                "session.mark:3",
                "session.mark:4",
                "session.exited:\"x\""
            ]
        );
        assert_eq!(
            names(&next_batch(&mut b, 100).await.unwrap().0),
            ["session.exited:\"x\""]
        );
        assert!(a.try_recv().is_err());
    }

    #[tokio::test]
    async fn replacing_the_extensions_keeps_the_core_and_drops_the_gone() {
        let bus = Bus::new();
        let _core = bus.subscribe("tasks", &[AGENT_CONVERSATION]);
        let (tx, _rx) = mpsc::channel(4);
        let queue = Queue { tx, tally: None };
        bus.set_extensions(vec![("x".into(), vec![AGENT_CONVERSATION.into()], queue)]);
        assert_eq!(bus.extensions_for(AGENT_CONVERSATION), ["x"]);
        bus.set_extensions(Vec::new());
        assert!(bus.extensions_for(AGENT_CONVERSATION).is_empty());
        assert!(bus.wants(AGENT_CONVERSATION));
    }

    #[test]
    fn a_batch_keeps_only_the_latest_status_of_each_session() {
        let ev = |name: &str, data: Value| {
            Arc::new(Event {
                name: name.into(),
                data,
            })
        };
        let mut batch = vec![
            ev(
                SESSION_STATUS,
                json!({ "sessionId": "a", "status": "working" }),
            ),
            ev(SESSION_MARK, json!({ "sessionId": "a" })),
            ev(
                SESSION_STATUS,
                json!({ "sessionId": "b", "status": "idle" }),
            ),
            ev(
                SESSION_STATUS,
                json!({ "sessionId": "a", "status": "idle" }),
            ),
            ev(GIT_CHANGED, json!({ "worktreePath": "/w" })),
            ev(GIT_CHANGED, json!({ "worktreePath": "/w" })),
        ];
        coalesce(&mut batch);
        assert_eq!(
            names(&batch),
            [
                "session.mark:{\"sessionId\":\"a\"}",
                "session.status:{\"sessionId\":\"b\",\"status\":\"idle\"}",
                "session.status:{\"sessionId\":\"a\",\"status\":\"idle\"}",
                "git.changed:{\"worktreePath\":\"/w\"}",
            ]
        );
    }

    #[test]
    fn tells_a_misspelt_core_event_from_another_extensions_own() {
        assert!(unknown_core_event("session.exit"));
        assert!(!unknown_core_event("session.exited"));
        assert!(!unknown_core_event("task.statusChanged"));
        assert!(unknown_core_event("task.moved"));
        assert!(!unknown_core_event("issues.opened"));
    }
}

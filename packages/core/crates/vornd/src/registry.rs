//! vornd's copy of the server's session registry.
//!
//! The server owns its terminals' and headless agents' records: their names,
//! groups, agents, worktrees and statuses. While vornd runs native work it
//! keeps a copy, fed by the server over the app's channel
//! ([`crate::control`]) as `vornd:record` notes: a whole snapshot when the
//! server connects, then each record the server changes, the order of its
//! terminals and the workspaces it holds while a session is being prepared.
//! Nothing here decides anything yet; the copy answers the calls that only
//! read the registry in shadow mode, so the two can be compared and any
//! place the server changes a record without saying so shows up.
//!
//! Every change the copy takes moves its revision ([`Rev`]) on by one and is
//! told to subscribers as a `vornd:session` note carrying the generation
//! ([`Gen`]) and the revision. The generation is drawn at random when vornd
//! starts, so a subscriber that sees a new one, or a revision it skipped,
//! knows to ask for the whole registry again rather than patch a stale copy.
//!
//! A record's status and exit can carry the record cursor of the effect that
//! set them ([`Stamp`]). A record that arrives with an older stamp than the
//! one held keeps the held status or exit: a state told again after a
//! reconnect never overwrites a newer one.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;

/// Notes kept for a subscriber that is behind; one further behind sees a gap
/// in the revisions and asks for the whole registry.
pub const NOTES_KEPT: usize = 1024;

/// Which vornd the revisions belong to: drawn at random when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gen(pub u64);

impl Gen {
    /// A generation no earlier vornd is likely to have had. `RandomState` is
    /// seeded from the OS for each process, which is all the randomness this
    /// needs; the time and pid are mixed in for platforms where it is not.
    pub fn draw() -> Gen {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        h.write_u128(now);
        h.write_u32(std::process::id());
        Gen(h.finish())
    }
}

/// As a string: a JavaScript number cannot hold every `u64`.
impl Serialize for Gen {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&format_args!("{:016x}", self.0))
    }
}

/// How many changes the registry has taken in this generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Rev(pub u64);

/// The record cursor of the effect that set a state: its session's epoch,
/// the record's sequence number and the effect's index in that record.
/// Ordered as cursors are, epoch first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub epoch: u64,
    pub rseq: u64,
    pub index: u64,
}

/// Whether `incoming` was set before what is `held`: only then is it stale.
/// A state with no stamp came from somewhere other than the session's
/// output (a hook, a timer, the person) and is the newest word on it.
fn stale(incoming: Option<Stamp>, held: Option<Stamp>) -> bool {
    matches!((incoming, held), (Some(a), Some(b)) if a < b)
}

/// The two kinds of session the server keeps a record of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Terminal,
    Headless,
}

/// `AgentStatus` in packages/shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Running,
    Waiting,
    Idle,
    Error,
}

/// `TerminalSession['statusSource']` in packages/shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusSource {
    Hooks,
    Pattern,
}

/// `HeadlessSession['status']` in packages/shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeadlessStatus {
    Running,
    Exited,
}

/// `TerminalSession` in packages/shared.
///
/// `agentType` stays a string: the list of agents grows on the server's
/// side, and a session of a kind this build has not heard of is still a
/// session. A field this mirror does not name yet is carried in `other`, so
/// it reaches readers unchanged rather than being dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub id: String,
    pub agent_type: String,
    pub project_name: String,
    pub project_path: String,
    pub status: AgentStatus,
    pub created_at: i64,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_worktree: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_source: Option<StatusSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_by_person: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<i64>,
    /// The registry revision of this record. Set only on what the registry
    /// tells subscribers: the server's own records do not carry one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<u64>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// `HeadlessSession` in packages/shared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessSession {
    pub id: String,
    pub pid: u32,
    pub agent_type: String,
    pub project_name: String,
    pub project_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_worktree: Option<bool>,
    pub status: HeadlessStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_command: Option<String>,
    /// As on [`TerminalSession::rev`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<u64>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// What a record kind has in common for the registry: an id, a status and
/// an exit that an older stamp must not overwrite, and the revision it is
/// told with.
trait Record: Clone + PartialEq + Serialize {
    fn id(&self) -> &str;
    /// Takes the status from `held`, which is newer.
    fn keep_status(&mut self, held: &Self);
    /// Takes how it ended from `held`, which is newer.
    fn keep_exit(&mut self, held: &Self);
    fn set_rev(&mut self, rev: Option<u64>);
}

impl Record for TerminalSession {
    fn id(&self) -> &str {
        &self.id
    }
    fn keep_status(&mut self, held: &Self) {
        self.status = held.status;
    }
    fn keep_exit(&mut self, held: &Self) {
        self.shell_exit_code = held.shell_exit_code;
    }
    fn set_rev(&mut self, rev: Option<u64>) {
        self.rev = rev;
    }
}

impl Record for HeadlessSession {
    fn id(&self) -> &str {
        &self.id
    }
    // A headless agent's status is only ever running or exited, and exited
    // is how it ended: both go with the exit.
    fn keep_status(&mut self, _: &Self) {}
    fn keep_exit(&mut self, held: &Self) {
        self.status = held.status;
        self.exit_code = held.exit_code;
        self.ended_at = held.ended_at;
    }
    fn set_rev(&mut self, rev: Option<u64>) {
        self.rev = rev;
    }
}

#[derive(Clone, Debug)]
struct Row<R> {
    record: R,
    status_at: Option<Stamp>,
    exit_at: Option<Stamp>,
    rev: Rev,
}

impl<R: Record> Row<R> {
    /// The record as subscribers are told it, with its revision.
    fn told(&self) -> R {
        let mut r = self.record.clone();
        r.set_rev(Some(self.rev.0));
        r
    }
}

/// One kind's records, in the order the server first registered them, which
/// is the order its maps iterate in. A server holds dozens of sessions, not
/// thousands, so a vector searched in place is the simplest thing that keeps
/// that order.
#[derive(Clone, Debug)]
struct Table<R> {
    rows: Vec<Row<R>>,
}

impl<R> Default for Table<R> {
    fn default() -> Self {
        Table { rows: Vec::new() }
    }
}

impl<R: Record> Table<R> {
    fn get(&self, id: &str) -> Option<&Row<R>> {
        self.rows.iter().find(|r| r.record.id() == id)
    }

    /// Puts `record` in, keeping a newer status or exit than the one it
    /// brings. Answers whether anything changed.
    fn upsert(
        &mut self,
        mut record: R,
        mut status_at: Option<Stamp>,
        mut exit_at: Option<Stamp>,
        rev: Rev,
    ) -> bool {
        let Some(row) = self.rows.iter_mut().find(|r| r.record.id() == record.id()) else {
            self.rows.push(Row {
                record,
                status_at,
                exit_at,
                rev,
            });
            return true;
        };
        if stale(status_at, row.status_at) {
            record.keep_status(&row.record);
            status_at = row.status_at;
        }
        if stale(exit_at, row.exit_at) {
            record.keep_exit(&row.record);
            exit_at = row.exit_at;
        }
        if record == row.record && status_at == row.status_at && exit_at == row.exit_at {
            return false;
        }
        *row = Row {
            record,
            status_at,
            exit_at,
            rev,
        };
        true
    }

    fn remove(&mut self, id: &str) -> bool {
        let before = self.rows.len();
        self.rows.retain(|r| r.record.id() != id);
        self.rows.len() != before
    }

    /// Replaced whole, as a snapshot from the server says. Stamps held for a
    /// session the snapshot still has are kept, so a stale state told after
    /// the snapshot is still recognised.
    fn replace(&mut self, records: Vec<R>, rev: Rev) {
        let old = std::mem::take(&mut self.rows);
        self.rows = records
            .into_iter()
            .map(|record| {
                let held = old.iter().find(|r| r.record.id() == record.id());
                Row {
                    status_at: held.and_then(|r| r.status_at),
                    exit_at: held.and_then(|r| r.exit_at),
                    record,
                    rev,
                }
            })
            .collect();
    }

    fn records(&self) -> impl Iterator<Item = &R> {
        self.rows.iter().map(|r| &r.record)
    }
}

/// One change the server makes to its registry, as `vornd:record` carries
/// it in `{op, ...}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A record was created or changed, with the stamps of its status and
    /// exit when the session's output set them.
    Upsert {
        /// Boxed: a record is many times the size of every other change.
        record: Box<Session>,
        status_at: Option<Stamp>,
        exit_at: Option<Stamp>,
    },
    /// The server let go of a record.
    Remove { kind: Kind, id: String },
    /// The order the server lists its terminals in.
    Order(Vec<String>),
    /// The workspaces held while a session is prepared, by path, with how
    /// many preparations hold each.
    Holds(BTreeMap<String, u32>),
    /// Everything, as the server holds it when it connects.
    Snapshot(Snapshot),
}

/// A record of either kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    Terminal(TerminalSession),
    Headless(HeadlessSession),
}

/// The whole registry, as the server sends it and as subscribers are given
/// it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub terminals: Vec<TerminalSession>,
    pub headless: Vec<HeadlessSession>,
    pub order: Vec<String>,
    pub holds: BTreeMap<String, u32>,
}

/// Why a `vornd:record` note was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The note is not one the registry knows, or a field is the wrong shape.
    Malformed { op: String, why: String },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Malformed { op, why } => {
                write!(f, "vornd:record {op}: {why}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// The shape of a `vornd:record` note on the wire.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    op: String,
    kind: Option<Kind>,
    record: Option<Value>,
    id: Option<String>,
    order: Option<Vec<String>>,
    holds: Option<BTreeMap<String, u32>>,
    status_at: Option<Stamp>,
    exit_at: Option<Stamp>,
    terminals: Option<Vec<TerminalSession>>,
    headless: Option<Vec<HeadlessSession>>,
}

impl TryFrom<&Value> for Change {
    type Error = RegistryError;

    fn try_from(v: &Value) -> Result<Change, RegistryError> {
        let op = v
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("(no op)")
            .to_owned();
        let bad = |why: String| RegistryError::Malformed {
            op: op.clone(),
            why,
        };
        let w = Wire::deserialize(v).map_err(|e| bad(e.to_string()))?;
        let missing = |field: &str| bad(format!("needs {field}"));
        Ok(match w.op.as_str() {
            "upsert" => {
                let record = w.record.ok_or_else(|| missing("record"))?;
                let record = match w.kind.ok_or_else(|| missing("kind"))? {
                    Kind::Terminal => Session::Terminal(
                        TerminalSession::deserialize(&record).map_err(|e| bad(e.to_string()))?,
                    ),
                    Kind::Headless => Session::Headless(
                        HeadlessSession::deserialize(&record).map_err(|e| bad(e.to_string()))?,
                    ),
                };
                Change::Upsert {
                    record: Box::new(record),
                    status_at: w.status_at,
                    exit_at: w.exit_at,
                }
            }
            "remove" => Change::Remove {
                kind: w.kind.ok_or_else(|| missing("kind"))?,
                id: w.id.ok_or_else(|| missing("id"))?,
            },
            "order" => Change::Order(w.order.ok_or_else(|| missing("order"))?),
            "holds" => Change::Holds(w.holds.ok_or_else(|| missing("holds"))?),
            "snapshot" => Change::Snapshot(Snapshot {
                terminals: w.terminals.ok_or_else(|| missing("terminals"))?,
                headless: w.headless.ok_or_else(|| missing("headless"))?,
                order: w.order.unwrap_or_default(),
                holds: w.holds.unwrap_or_default(),
            }),
            other => return Err(bad(format!("unknown op `{other}`"))),
        })
    }
}

/// The registry: every record the server holds, as it last told them.
#[derive(Clone, Debug)]
pub struct Registry {
    gen: Gen,
    rev: Rev,
    terminals: Table<TerminalSession>,
    headless: Table<HeadlessSession>,
    order: Vec<String>,
    holds: BTreeMap<String, u32>,
}

impl Registry {
    /// An empty registry in generation `gen`.
    pub fn new(gen: Gen) -> Registry {
        Registry {
            gen,
            rev: Rev::default(),
            terminals: Table::default(),
            headless: Table::default(),
            order: Vec::new(),
            holds: BTreeMap::new(),
        }
    }

    pub fn gen(&self) -> Gen {
        self.gen
    }

    pub fn rev(&self) -> Rev {
        self.rev
    }

    /// Takes one change. Answers the `vornd:session` note to tell
    /// subscribers, or `None` when the change changed nothing (the same
    /// record told twice, or a stale state and nothing else).
    pub fn apply(&mut self, change: Change) -> Option<Value> {
        let next = Rev(self.rev.0 + 1);
        let fields = match change {
            Change::Upsert {
                record,
                status_at,
                exit_at,
            } => match *record {
                Session::Terminal(r) => {
                    let id = r.id.clone();
                    if !self.terminals.upsert(r, status_at, exit_at, next) {
                        return None;
                    }
                    let row = self.terminals.get(&id).expect("the row was just put in");
                    json!({ "op": "upsert", "kind": Kind::Terminal, "record": row.told() })
                }
                Session::Headless(r) => {
                    let id = r.id.clone();
                    if !self.headless.upsert(r, status_at, exit_at, next) {
                        return None;
                    }
                    let row = self.headless.get(&id).expect("the row was just put in");
                    json!({ "op": "upsert", "kind": Kind::Headless, "record": row.told() })
                }
            },
            Change::Remove { kind, id } => {
                let gone = match kind {
                    Kind::Terminal => self.terminals.remove(&id),
                    Kind::Headless => self.headless.remove(&id),
                };
                if !gone {
                    return None;
                }
                json!({ "op": "remove", "kind": kind, "id": id })
            }
            Change::Order(order) => {
                if order == self.order {
                    return None;
                }
                self.order = order;
                json!({ "op": "order", "order": self.order })
            }
            Change::Holds(holds) => {
                if holds == self.holds {
                    return None;
                }
                self.holds = holds;
                json!({ "op": "holds", "holds": self.holds })
            }
            Change::Snapshot(s) => {
                self.terminals.replace(s.terminals, next);
                self.headless.replace(s.headless, next);
                self.order = s.order;
                self.holds = s.holds;
                self.rev = next;
                let mut note = self.snapshot();
                note["op"] = json!("snapshot");
                return Some(note);
            }
        };
        self.rev = next;
        let mut note = json!({ "gen": self.gen, "rev": next });
        if let (Value::Object(note), Value::Object(fields)) = (&mut note, fields) {
            note.extend(fields);
        }
        Some(note)
    }

    /// Everything, with the generation and revision it stands at: what a
    /// subscriber starts from or resyncs to. Each record carries the
    /// revision it last changed at.
    pub fn snapshot(&self) -> Value {
        json!({
            "gen": self.gen,
            "rev": self.rev,
            "terminals": self.terminals.rows.iter().map(Row::told).collect::<Vec<_>>(),
            "headless": self.headless.rows.iter().map(Row::told).collect::<Vec<_>>(),
            "order": self.order,
            "holds": self.holds,
        })
    }

    /// The terminals as `terminal:listActive` lists them: those the order
    /// names first, in that order, then the rest in the order they were
    /// registered (`PtyManager.getActiveSessions`).
    pub fn terminals(&self) -> Vec<&TerminalSession> {
        let mut listed: Vec<&TerminalSession> = Vec::with_capacity(self.terminals.rows.len());
        // An id the order names twice is listed twice, as the server lists it.
        for id in &self.order {
            if let Some(row) = self.terminals.get(id) {
                listed.push(&row.record);
            }
        }
        for s in self.terminals.records() {
            if !self.order.contains(&s.id) {
                listed.push(s);
            }
        }
        listed
    }

    /// The headless agents, in the order they were registered.
    pub fn headless(&self) -> impl Iterator<Item = &HeadlessSession> {
        self.headless.records()
    }

    /// The sessions still at work in the worktree at `path`, terminals
    /// first: a terminal that is not idle, an agent still running
    /// (`worktree:activeSessions`). The path is compared as given, as the
    /// server compares it.
    pub fn active_in_worktree(&self, path: &str) -> Vec<&str> {
        let terminals = self
            .terminals
            .records()
            .filter(|s| s.worktree_path.as_deref() == Some(path) && s.status != AgentStatus::Idle)
            .map(|s| s.id.as_str());
        let headless = self
            .headless
            .records()
            .filter(|s| {
                s.worktree_path.as_deref() == Some(path) && s.status == HeadlessStatus::Running
            })
            .map(|s| s.id.as_str());
        terminals.chain(headless).collect()
    }

    /// The workspaces held while a session is prepared.
    pub fn holds(&self) -> &BTreeMap<String, u32> {
        &self.holds
    }
}

/// The registry as vornd shares it: the app's channel feeds it, the native
/// calls read it, and subscribers are told each change.
#[derive(Debug)]
pub struct SessionRegistry {
    state: Mutex<Fed>,
    notes: broadcast::Sender<Value>,
    wanted: AtomicBool,
}

#[derive(Debug)]
struct Fed {
    registry: Registry,
    /// The app connection whose snapshot the registry holds. Until one has
    /// sent its snapshot, or once it has gone, the copy cannot be trusted
    /// to be the server's and nothing is answered from it.
    feeder: Option<u64>,
}

impl SessionRegistry {
    /// An empty registry, in a generation of its own.
    pub fn new() -> Arc<SessionRegistry> {
        SessionRegistry::with_gen(Gen::draw())
    }

    pub fn with_gen(gen: Gen) -> Arc<SessionRegistry> {
        Arc::new(SessionRegistry {
            state: Mutex::new(Fed {
                registry: Registry::new(gen),
                feeder: None,
            }),
            notes: broadcast::channel(NOTES_KEPT).0,
            wanted: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Fed> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Asks the server for its records: set when vornd has native work that
    /// reads them. `vornd:hello` tells the server, and a server that is not
    /// asked sends nothing.
    pub fn want(&self) {
        self.wanted.store(true, Ordering::Release);
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Acquire)
    }

    /// Takes a `vornd:record` note from app connection `conn`, and tells
    /// subscribers what it changed. A snapshot makes `conn` the feeder.
    pub fn feed(&self, conn: u64, params: &Value) -> Result<(), RegistryError> {
        let change = Change::try_from(params)?;
        let snapshot = matches!(change, Change::Snapshot(_));
        let mut fed = self.lock();
        if snapshot {
            fed.feeder = Some(conn);
        }
        if let Some(note) = fed.registry.apply(change) {
            // Sent under the lock, so notes go out in revision order. No
            // subscriber is not an error.
            let _ = self.notes.send(note);
        }
        Ok(())
    }

    /// App connection `conn` closed. If it fed the registry, the copy is
    /// left as it was but no longer answers until the next snapshot.
    pub fn left(&self, conn: u64) {
        let mut fed = self.lock();
        if fed.feeder == Some(conn) {
            fed.feeder = None;
        }
    }

    /// Every change from now on, as `vornd:session` params.
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.notes.subscribe()
    }

    /// The whole registry ([`Registry::snapshot`]).
    pub fn snapshot(&self) -> Value {
        self.lock().registry.snapshot()
    }

    /// Reads the registry, while it holds the server's records; `None`
    /// before the server has sent them, or after it went.
    pub fn read<T>(&self, f: impl FnOnce(&Registry) -> T) -> Option<T> {
        let fed = self.lock();
        fed.feeder.is_some().then(|| f(&fed.registry))
    }

    /// For `/vornd/health`: where the registry stands, without its records.
    pub fn report(&self) -> Value {
        let fed = self.lock();
        json!({
            "gen": fed.registry.gen,
            "rev": fed.registry.rev,
            "fed": fed.feeder.is_some(),
            "terminals": fed.registry.terminals.rows.len(),
            "headless": fed.registry.headless.rows.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(id: &str) -> Value {
        json!({
            "id": id, "agentType": "shell", "projectName": "p", "projectPath": "/p",
            "status": "running", "createdAt": 1_700_000_000_000_i64, "pid": 0,
        })
    }

    fn headless(id: &str) -> Value {
        json!({
            "id": id, "pid": 7, "agentType": "claude", "projectName": "p", "projectPath": "/p",
            "status": "running", "startedAt": 1,
        })
    }

    fn change(v: Value) -> Change {
        Change::try_from(&v).unwrap()
    }

    fn upsert(record: Value) -> Change {
        change(json!({ "op": "upsert", "kind": "terminal", "record": record }))
    }

    fn stamped(record: Value, epoch: u64, rseq: u64, index: u64) -> Change {
        change(json!({
            "op": "upsert", "kind": "terminal", "record": record,
            "statusAt": { "epoch": epoch, "rseq": rseq, "index": index },
        }))
    }

    fn ids(r: &Registry) -> Vec<&str> {
        r.terminals().iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn every_change_moves_the_revision_on_by_one_and_a_repeat_does_not() {
        let mut r = Registry::new(Gen(9));
        let note = r.apply(upsert(terminal("a"))).unwrap();
        assert_eq!(note["rev"], 1);
        assert_eq!(note["gen"], "0000000000000009");
        assert_eq!(note["op"], "upsert");
        assert_eq!(note["record"]["rev"], 1);
        // The same record again changes nothing and is not told.
        assert!(r.apply(upsert(terminal("a"))).is_none());
        assert_eq!(r.rev(), Rev(1));

        let mut renamed = terminal("a");
        renamed["displayName"] = json!("Shell 1");
        assert_eq!(r.apply(upsert(renamed)).unwrap()["rev"], 2);
        let note = r
            .apply(change(json!({ "op": "order", "order": ["a"] })))
            .unwrap();
        assert_eq!(
            (note["rev"].as_u64(), note["op"].as_str()),
            (Some(3), Some("order"))
        );
        assert!(r
            .apply(change(json!({ "op": "order", "order": ["a"] })))
            .is_none());
        let note = r
            .apply(change(
                json!({ "op": "remove", "kind": "terminal", "id": "a" }),
            ))
            .unwrap();
        assert_eq!(note["rev"], 4);
        // Nothing left to remove.
        assert!(r
            .apply(change(
                json!({ "op": "remove", "kind": "terminal", "id": "a" })
            ))
            .is_none());
        assert_eq!(r.rev(), Rev(4));
    }

    #[test]
    fn a_status_with_an_older_stamp_keeps_the_newer_one() {
        let mut r = Registry::new(Gen(1));
        let mut waiting = terminal("a");
        waiting["status"] = json!("waiting");
        r.apply(stamped(waiting, 2, 40, 0)).unwrap();

        // Told again after a reconnect: an earlier record's running.
        let mut older = terminal("a");
        older["displayName"] = json!("renamed meanwhile");
        let note = r.apply(stamped(older.clone(), 2, 39, 1)).unwrap();
        assert_eq!(note["record"]["status"], "waiting");
        assert_eq!(note["record"]["displayName"], "renamed meanwhile");
        // An earlier epoch is older whatever its sequence number.
        assert!(r.apply(stamped(older, 1, 900, 0)).is_none());
        assert_eq!(r.terminals()[0].status, AgentStatus::Waiting);

        // A later stamp, or none at all (a hook, a timer), sets it.
        let mut idle = terminal("a");
        idle["displayName"] = json!("renamed meanwhile");
        idle["status"] = json!("idle");
        r.apply(upsert(idle.clone())).unwrap();
        assert_eq!(r.terminals()[0].status, AgentStatus::Idle);
        let mut running = idle;
        running["status"] = json!("running");
        r.apply(stamped(running, 2, 39, 1)).unwrap();
        assert_eq!(r.terminals()[0].status, AgentStatus::Running);
    }

    #[test]
    fn an_exit_with_an_older_stamp_keeps_the_newer_one() {
        let mut r = Registry::new(Gen(1));
        let exit = |record: Value, epoch| {
            change(json!({
                "op": "upsert", "kind": "headless", "record": record,
                "exitAt": { "epoch": epoch, "rseq": 0, "index": 0 },
            }))
        };
        let mut ended = headless("h");
        ended["status"] = json!("exited");
        ended["exitCode"] = json!(0);
        ended["endedAt"] = json!(5);
        r.apply(exit(ended, 3)).unwrap();
        // The exit of an earlier run of the same id.
        let mut earlier = headless("h");
        earlier["status"] = json!("exited");
        earlier["exitCode"] = json!(1);
        earlier["endedAt"] = json!(2);
        assert!(r.apply(exit(earlier, 2)).is_none());
        let h = r.headless().next().unwrap();
        assert_eq!((h.status, h.exit_code), (HeadlessStatus::Exited, Some(0)));
    }

    #[test]
    fn a_snapshot_replaces_everything_and_keeps_the_stamps_of_sessions_it_still_has() {
        let mut r = Registry::new(Gen(1));
        let mut waiting = terminal("a");
        waiting["status"] = json!("waiting");
        r.apply(stamped(waiting.clone(), 1, 10, 0)).unwrap();
        r.apply(upsert(terminal("gone"))).unwrap();
        let note = r
            .apply(change(json!({
                "op": "snapshot",
                "terminals": [waiting, terminal("b")],
                "headless": [headless("h")],
                "order": ["b", "a"],
                "holds": { "/w": 1 },
            })))
            .unwrap();
        assert_eq!(note["op"], "snapshot");
        assert_eq!(note["rev"], 3);
        assert_eq!(note["terminals"].as_array().map(Vec::len), Some(2));
        assert_eq!(ids(&r), ["b", "a"]);
        assert_eq!(r.holds()["/w"], 1);
        // The stamp of `a` outlived the snapshot.
        assert!(r.apply(stamped(terminal("a"), 1, 9, 0)).is_none());
    }

    #[test]
    fn lists_terminals_as_the_server_does() {
        let mut r = Registry::new(Gen(1));
        for id in ["a", "b", "c", "d"] {
            r.apply(upsert(terminal(id))).unwrap();
        }
        // No order: as registered.
        assert_eq!(ids(&r), ["a", "b", "c", "d"]);
        // Named first, an id it does not have skipped, the rest after.
        r.apply(change(json!({ "op": "order", "order": ["c", "x", "a"] })))
            .unwrap();
        assert_eq!(ids(&r), ["c", "a", "b", "d"]);
        // Changed in place keeps its place; let go and back goes last.
        let mut b = terminal("b");
        b["pid"] = json!(42);
        r.apply(upsert(b)).unwrap();
        r.apply(change(
            json!({ "op": "remove", "kind": "terminal", "id": "a" }),
        ))
        .unwrap();
        r.apply(upsert(terminal("a"))).unwrap();
        r.apply(change(json!({ "op": "order", "order": [] })))
            .unwrap();
        assert_eq!(ids(&r), ["b", "c", "d", "a"]);
    }

    #[test]
    fn counts_the_sessions_at_work_in_a_worktree() {
        let mut r = Registry::new(Gen(1));
        let at = |id: &str, status: &str| {
            let mut t = terminal(id);
            t["worktreePath"] = json!("/w");
            t["status"] = json!(status);
            upsert(t)
        };
        r.apply(at("t1", "running")).unwrap();
        r.apply(at("t2", "idle")).unwrap();
        r.apply(at("t3", "waiting")).unwrap();
        r.apply(upsert(terminal("elsewhere"))).unwrap();
        let mut h = headless("h1");
        h["worktreePath"] = json!("/w");
        r.apply(change(
            json!({ "op": "upsert", "kind": "headless", "record": h.clone() }),
        ))
        .unwrap();
        h["id"] = json!("h2");
        h["status"] = json!("exited");
        r.apply(change(
            json!({ "op": "upsert", "kind": "headless", "record": h }),
        ))
        .unwrap();
        assert_eq!(r.active_in_worktree("/w"), ["t1", "t3", "h1"]);
        assert!(r.active_in_worktree("/w/").is_empty());
    }

    #[test]
    fn carries_fields_it_does_not_name_and_reads_back_what_it_was_given() {
        let mut t = terminal("a");
        t["groupId"] = json!("g");
        t["somethingNew"] = json!({ "x": [1, 2] });
        let Change::Upsert { record, .. } = upsert(t.clone()) else {
            panic!("an upsert");
        };
        let Session::Terminal(s) = *record else {
            panic!("a terminal");
        };
        assert_eq!(serde_json::to_value(&s).unwrap(), t);
    }

    #[test]
    fn refuses_a_note_it_cannot_read_and_says_why() {
        for (note, says) in [
            (
                json!({ "op": "upsert", "kind": "terminal" }),
                "needs record",
            ),
            (
                json!({ "op": "upsert", "record": terminal("a") }),
                "needs kind",
            ),
            (
                json!({ "op": "upsert", "kind": "terminal", "record": { "id": "a" } }),
                "missing field",
            ),
            (json!({ "op": "remove", "kind": "terminal" }), "needs id"),
            (json!({ "op": "rename" }), "unknown op `rename`"),
            (json!({ "kind": "terminal" }), "missing field `op`"),
        ] {
            let err = Change::try_from(&note).unwrap_err().to_string();
            assert!(err.contains(says), "{note}: {err}");
        }
    }

    #[test]
    fn answers_nothing_until_the_server_has_sent_its_records_or_once_it_left() {
        let shared = SessionRegistry::with_gen(Gen(1));
        let mut notes = shared.subscribe();
        shared
            .feed(
                1,
                &json!({ "op": "upsert", "kind": "terminal", "record": terminal("a") }),
            )
            .unwrap();
        assert!(shared.read(|r| r.terminals().len()).is_none());
        shared
            .feed(
                1,
                &json!({ "op": "snapshot", "terminals": [terminal("a")], "headless": [] }),
            )
            .unwrap();
        assert_eq!(shared.read(|r| r.terminals().len()), Some(1));
        // Another connection closing leaves it be; the feeder's does not.
        shared.left(2);
        assert!(shared.read(|_| ()).is_some());
        shared.left(1);
        assert!(shared.read(|_| ()).is_none());
        // Told in revision order.
        assert_eq!(notes.try_recv().unwrap()["rev"], 1);
        assert_eq!(notes.try_recv().unwrap()["rev"], 2);
        assert!(shared.feed(1, &json!({ "op": "nope" })).is_err());
        assert_eq!(shared.report()["rev"], 2);
    }

    #[test]
    fn a_new_vornd_has_a_new_generation() {
        assert_ne!(Gen::draw(), Gen::draw());
        let a = SessionRegistry::new();
        let b = SessionRegistry::new();
        assert_ne!(a.snapshot()["gen"], b.snapshot()["gen"]);
        assert_eq!(a.snapshot()["rev"], 0);
    }
}

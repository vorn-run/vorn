//! vornd's copy of the server's session registry.
//!
//! The server owns its terminals' and headless agents' records: their names,
//! groups, agents, worktrees and statuses. While vornd runs native work it
//! keeps a copy, fed by the server over the app's channel
//! ([`crate::control`]) as `vornd:record` notes: a whole snapshot when the
//! server connects, then each record the server changes, the order of its
//! terminals and the workspaces it holds while a session is being prepared.
//! The copy answers the calls that only read the registry in shadow mode, so
//! the two can be compared and any place the server changes a record without
//! saying so shows up.
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
//!
//! With the Native server switch on, the registry also decides each
//! terminal's status itself ([`Registry::decide_statuses`]), from what the
//! session's screen shows, from its output going quiet, from input the server
//! writes to it and from what the agent's hooks report, as the server did. The
//! server stops deciding and takes each status from the copy's notes instead,
//! and an upsert it sends no longer moves what the registry decides: the
//! status, where it comes from (`statusSource`) and the hook session linked
//! to it (`hookSessionId`). Every status the registry sets carries a stamp: a
//! screen status the effect's, anything else the session's head at the moment
//! it arrives ([`Stamp::at_head`]), so a hook's word is never undone by an
//! older screen status told again. Once the server says a terminal's program
//! ended, its record is the server's again, whole, but for the fields only
//! `vornd:patch` and vornd's own calls set (a name, a group, the agent's
//! conversation), which stay the registry's whatever an upsert says.
//!
//! With the Native server switch on, vornd also creates, renames, regroups,
//! reorders and closes terminals itself ([`crate::native::sessions`]). Those
//! changes are told with `native: true`, so the server knows they are not the
//! echo of its own: a terminal vornd created (`created`, then `started` once
//! its program is up, or `failed`), one it closed, and the order it set. A
//! terminal vornd closed is remembered until the server lets go of it too,
//! so an upsert the server sent before it heard does not bring it back. The
//! workspaces vornd holds while it prepares a session are told beside the
//! server's own, as `nativeHolds`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
use tokio::time::Instant;
use vorn_term_proto::Cursor;

/// Notes kept for a subscriber that is behind; one further behind sees a gap
/// in the revisions and asks for the whole registry.
pub const NOTES_KEPT: usize = 1024;

/// How long a terminal that stopped printing stays running before it is
/// idle, while its status comes from what it prints.
pub const IDLE_AFTER: Duration = Duration::from_secs(5);

/// The same once hooks report its status: they say when it stops, so this is
/// only the safety net for a hook that never came.
pub const IDLE_AFTER_HOOKS: Duration = Duration::from_secs(30);

/// The fields `vornd:patch` may set. Everything else in a record is the
/// server's to send whole.
pub const PATCHABLE: [&str; 6] = [
    "displayName",
    "renamedByPerson",
    "groupId",
    "hookSessionId",
    "agentSessionId",
    "statusSource",
];

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

impl Stamp {
    /// The stamp of a state told from outside the output (a hook, input, a
    /// timer) while the session's records stand at `head`: after every effect
    /// of the last record applied, and before any of the next one. An
    /// effect's index is a `u32`, so `u32::MAX` is past all of them and still
    /// a whole JavaScript number.
    pub fn at_head(head: &Cursor) -> Stamp {
        let epoch = u64::from(head.epoch);
        match head.next_rseq.checked_sub(1) {
            Some(rseq) => Stamp {
                epoch,
                rseq,
                index: u64::from(u32::MAX),
            },
            None => Stamp {
                epoch,
                rseq: 0,
                index: 0,
            },
        }
    }
}

/// The stamp of a state that arrives now: the session's head, or the stamp
/// of the status held if that is later (an effect the head has not caught up
/// with yet). What arrives last is newer than everything already applied,
/// and older than what the output says after it.
fn receipt(head: Option<Stamp>, held: Option<Stamp>) -> Option<Stamp> {
    head.max(held)
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

impl AgentStatus {
    /// A status effect's code (`vorn_analysis::STATUS_*`), as the server's
    /// `NATIVE_STATUS` reads it: `None` says nothing.
    pub fn from_code(code: u32) -> Option<AgentStatus> {
        match code {
            1 => Some(AgentStatus::Running),
            2 => Some(AgentStatus::Waiting),
            3 => Some(AgentStatus::Error),
            _ => None,
        }
    }
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
    /// What set the status, and how the program ended, when the session's
    /// output or its head did. Told to subscribers only, as `rev` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_at: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_at: Option<Stamp>,
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
    /// As on [`TerminalSession::exit_at`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_at: Option<Stamp>,
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
    /// Takes from `held` what the registry decides while it decides the
    /// statuses, rather than what the server sent.
    fn keep_decided(&mut self, held: &Self);
    /// Takes from `held` the fields only a patch or vornd's own calls set
    /// while it decides, whether or not the program still runs.
    fn keep_patched(&mut self, held: &Self);
    /// Marks the record as subscribers are told it.
    fn tell(&mut self, rev: Rev, status_at: Option<Stamp>, exit_at: Option<Stamp>);
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
    fn keep_decided(&mut self, held: &Self) {
        self.status = held.status;
        self.status_source = held.status_source;
        self.hook_session_id.clone_from(&held.hook_session_id);
    }
    fn keep_patched(&mut self, held: &Self) {
        self.display_name.clone_from(&held.display_name);
        self.renamed_by_person = held.renamed_by_person;
        self.group_id.clone_from(&held.group_id);
        self.agent_session_id.clone_from(&held.agent_session_id);
    }
    fn tell(&mut self, rev: Rev, status_at: Option<Stamp>, exit_at: Option<Stamp>) {
        self.rev = Some(rev.0);
        self.status_at = status_at;
        self.exit_at = exit_at;
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
    // Its status is the server's: running until it exits.
    fn keep_decided(&mut self, _: &Self) {}
    // Nothing patches one.
    fn keep_patched(&mut self, _: &Self) {}
    fn tell(&mut self, rev: Rev, _: Option<Stamp>, exit_at: Option<Stamp>) {
        self.rev = Some(rev.0);
        self.exit_at = exit_at;
    }
}

#[derive(Clone, Debug)]
struct Row<R> {
    record: R,
    status_at: Option<Stamp>,
    exit_at: Option<Stamp>,
    rev: Rev,
    /// The revision the record was put in at: a patch read from an earlier
    /// record under the same id (a resume reuses it) is not for this one.
    created: Rev,
    /// The server said the session's program ended.
    ended: bool,
}

impl<R: Record> Row<R> {
    /// The record as subscribers are told it, with its revision and stamps.
    fn told(&self) -> R {
        let mut r = self.record.clone();
        r.tell(self.rev, self.status_at, self.exit_at);
        r
    }
}

/// A record's stamps, as an upsert brings them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stamps {
    status_at: Option<Stamp>,
    exit_at: Option<Stamp>,
}

/// What [`Table::upsert`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Upserted {
    Unchanged,
    Changed,
    Created,
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

    fn get_mut(&mut self, id: &str) -> Option<&mut Row<R>> {
        self.rows.iter_mut().find(|r| r.record.id() == id)
    }

    /// Puts `record` in, keeping a newer status or exit than the one it
    /// brings, and, when `decided`, what the registry decides of a session
    /// still running.
    fn upsert(
        &mut self,
        mut record: R,
        at: Stamps,
        ended: bool,
        decided: bool,
        rev: Rev,
    ) -> Upserted {
        let Stamps {
            mut status_at,
            mut exit_at,
        } = at;
        let Some(row) = self.rows.iter_mut().find(|r| r.record.id() == record.id()) else {
            self.rows.push(Row {
                record,
                status_at,
                exit_at,
                rev,
                created: rev,
                ended,
            });
            return Upserted::Created;
        };
        // The server's records stop moving what the registry decides, until
        // it says the program ended: from then on the record is its own.
        if decided {
            record.keep_patched(&row.record);
        }
        if decided && !ended && !row.ended {
            record.keep_decided(&row.record);
            status_at = row.status_at;
        }
        if stale(status_at, row.status_at) {
            record.keep_status(&row.record);
            status_at = row.status_at;
        }
        if stale(exit_at, row.exit_at) {
            record.keep_exit(&row.record);
            exit_at = row.exit_at;
        }
        if record == row.record
            && status_at == row.status_at
            && exit_at == row.exit_at
            && ended == row.ended
        {
            return Upserted::Unchanged;
        }
        *row = Row {
            record,
            status_at,
            exit_at,
            rev,
            created: row.created,
            ended,
        };
        Upserted::Changed
    }

    fn remove(&mut self, id: &str) -> bool {
        let before = self.rows.len();
        self.rows.retain(|r| r.record.id() != id);
        self.rows.len() != before
    }

    /// Replaced whole, as a snapshot from the server says, with the ids
    /// whose programs ended. Stamps held for a session the snapshot still
    /// has are kept, so a stale state told after the snapshot is still
    /// recognised, and so, when `decided`, is what the registry decides of
    /// one still running. Answers the ids it did not hold before.
    fn replace(
        &mut self,
        records: Vec<R>,
        ended: &[String],
        decided: bool,
        rev: Rev,
    ) -> Vec<String> {
        let old = std::mem::take(&mut self.rows);
        let mut fresh = Vec::new();
        self.rows = records
            .into_iter()
            .map(|mut record| {
                let held = old.iter().find(|r| r.record.id() == record.id());
                let ended = ended.iter().any(|id| id == record.id());
                match held {
                    Some(h) if decided => {
                        record.keep_patched(&h.record);
                        if !ended && !h.ended {
                            record.keep_decided(&h.record);
                        }
                    }
                    Some(_) => {}
                    None => fresh.push(record.id().to_owned()),
                }
                Row {
                    status_at: held.and_then(|r| r.status_at),
                    exit_at: held.and_then(|r| r.exit_at),
                    created: held.map_or(rev, |r| r.created),
                    record,
                    rev,
                    ended,
                }
            })
            .collect();
        fresh
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
        /// The session's program ended: the server's record is its own
        /// again, status included.
        ended: bool,
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
    /// The terminals whose programs ended, as [`Change::Upsert`]'s `ended`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ended: Vec<String>,
}

/// Why a `vornd:record` note was not taken.
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// The note is not one the registry knows, or a field is the wrong shape.
    Malformed { op: String, why: String },
    /// A call that asks the registry to decide something, while it does not
    /// decide the statuses.
    NotDeciding { call: &'static str },
    /// A call's params are the wrong shape.
    BadCall { call: &'static str, why: String },
    /// No terminal goes by the id a call named.
    NoTerminal { call: &'static str, id: String },
    /// A patch read from a record the id no longer names.
    Replaced { id: String },
    /// A call a client made, refused in the words the server refuses it.
    Refused(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Malformed { op, why } => {
                write!(f, "vornd:record {op}: {why}")
            }
            RegistryError::NotDeciding { call } => {
                write!(f, "{call}: vornd does not decide the session statuses")
            }
            RegistryError::BadCall { call, why } => write!(f, "{call}: {why}"),
            RegistryError::NoTerminal { call, id } => write!(f, "{call}: no terminal {id}"),
            RegistryError::Refused(why) => f.write_str(why),
            RegistryError::Replaced { id } => {
                write!(
                    f,
                    "vornd:patch: terminal {id} was replaced since the patch was made"
                )
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
    ended: Option<Value>,
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
                    ended: w.ended.as_ref().and_then(Value::as_bool) == Some(true),
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
                ended: match w.ended {
                    None | Some(Value::Null) => Vec::new(),
                    Some(ids) => Vec::deserialize(ids).map_err(|e| bad(e.to_string()))?,
                },
            }),
            other => return Err(bad(format!("unknown op `{other}`"))),
        })
    }
}

/// A `vornd:hookStatus` call: the status the server's hook mapper read from
/// an agent's hook, and whether the session's status comes from its hooks
/// from now on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookStatus {
    pub id: String,
    /// Set first, then the promotion, as the server did them.
    pub status: Option<AgentStatus>,
    pub promote: bool,
}

impl TryFrom<&Value> for HookStatus {
    type Error = RegistryError;

    fn try_from(v: &Value) -> Result<HookStatus, RegistryError> {
        const CALL: &str = "vornd:hookStatus";
        let bad = |why: String| RegistryError::BadCall { call: CALL, why };
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("needs an id".to_owned()))?;
        let status = match v.get("status") {
            None | Some(Value::Null) => None,
            Some(s) => Some(AgentStatus::deserialize(s).map_err(|e| bad(e.to_string()))?),
        };
        let promote = match v.get("promote") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad("promote is true or false".to_owned())),
        };
        Ok(HookStatus {
            id: id.to_owned(),
            status,
            promote,
        })
    }
}

/// A `vornd:patch` call: fields of [`PATCHABLE`] to set on a terminal's
/// record, null taking one away.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    pub id: String,
    pub fields: Map<String, Value>,
    /// The revision of the record the patch was made from, when it was made
    /// from one.
    pub base_rev: Option<u64>,
}

impl TryFrom<&Value> for Patch {
    type Error = RegistryError;

    fn try_from(v: &Value) -> Result<Patch, RegistryError> {
        const CALL: &str = "vornd:patch";
        let bad = |why: String| RegistryError::BadCall { call: CALL, why };
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("needs an id".to_owned()))?;
        let fields = v
            .get("fields")
            .and_then(Value::as_object)
            .ok_or_else(|| bad("needs fields".to_owned()))?;
        if let Some(other) = fields.keys().find(|k| !PATCHABLE.contains(&k.as_str())) {
            return Err(bad(format!("`{other}` is not a field it may set")));
        }
        let base_rev = match v.get("baseRev") {
            None | Some(Value::Null) => None,
            Some(r) => Some(
                r.as_u64()
                    .ok_or_else(|| bad("baseRev is a revision".to_owned()))?,
            ),
        };
        Ok(Patch {
            id: id.to_owned(),
            fields: fields.clone(),
            base_rev,
        })
    }
}

/// What the registry keeps to decide the terminals' statuses itself.
#[derive(Clone, Debug, Default)]
struct Statuses {
    /// The last status each session's screen showed, with the effect that
    /// showed it, kept while the engine holds the session whatever its
    /// record says: a record put in later (the server taking on a session it
    /// held before it restarted) starts from it, as the server's did from
    /// the states vornd told it again.
    screens: HashMap<String, (AgentStatus, Stamp)>,
    /// When each terminal that went quiet turns idle.
    idle_at: HashMap<String, Instant>,
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
    /// The workspaces vornd holds itself while it prepares a session, by
    /// path, with how many preparations hold each.
    own_holds: BTreeMap<String, u32>,
    /// Terminals vornd closed that the server has not let go of yet: an
    /// upsert it sent before it heard is not a new terminal.
    closed: HashSet<String>,
    /// Set while the registry decides the terminals' statuses.
    statuses: Option<Statuses>,
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
            own_holds: BTreeMap::new(),
            closed: HashSet::new(),
            statuses: None,
        }
    }

    pub fn gen(&self) -> Gen {
        self.gen
    }

    pub fn rev(&self) -> Rev {
        self.rev
    }

    /// Decides the terminals' statuses from now on, rather than taking the
    /// server's. It cannot be undone: the server is told once, when it
    /// connects, and follows.
    pub fn decide_statuses(&mut self) {
        self.statuses.get_or_insert_with(Statuses::default);
    }

    pub fn decides(&self) -> bool {
        self.statuses.is_some()
    }

    /// Takes one change. Answers the `vornd:session` note to tell
    /// subscribers, or `None` when the change changed nothing (the same
    /// record told twice, or a stale state and nothing else).
    pub fn apply(&mut self, change: Change) -> Option<Value> {
        let next = Rev(self.rev.0 + 1);
        let decided = self.decides();
        let fields = match change {
            Change::Upsert {
                record,
                status_at,
                exit_at,
                ended,
            } => {
                let at = Stamps { status_at, exit_at };
                match *record {
                    // Closed here, and the server has not heard yet.
                    Session::Terminal(r) if self.closed.contains(&r.id) => return None,
                    Session::Terminal(r) => {
                        let id = r.id.clone();
                        match self.terminals.upsert(r, at, ended, decided, next) {
                            Upserted::Unchanged => return None,
                            Upserted::Created => self.seed(&id),
                            Upserted::Changed => {}
                        }
                        if ended {
                            self.quiet(&id);
                        }
                        let row = self.terminals.get(&id).expect("the row was just put in");
                        json!({ "op": "upsert", "kind": Kind::Terminal, "record": row.told() })
                    }
                    Session::Headless(r) => {
                        let id = r.id.clone();
                        if self.headless.upsert(r, at, ended, false, next) == Upserted::Unchanged {
                            return None;
                        }
                        let row = self.headless.get(&id).expect("the row was just put in");
                        json!({ "op": "upsert", "kind": Kind::Headless, "record": row.told() })
                    }
                }
            }
            Change::Remove { kind, id } => {
                // The server letting go of a terminal vornd closed: it heard.
                if kind == Kind::Terminal && self.closed.remove(&id) {
                    return None;
                }
                let gone = match kind {
                    Kind::Terminal => self.terminals.remove(&id),
                    Kind::Headless => self.headless.remove(&id),
                };
                if !gone {
                    return None;
                }
                if kind == Kind::Terminal {
                    self.quiet(&id);
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
                self.holds_fields()
            }
            Change::Snapshot(mut s) => {
                // A terminal vornd closed stays closed; one the server no
                // longer has, it has let go of.
                let still: HashSet<String> = s
                    .terminals
                    .iter()
                    .filter(|t| self.closed.contains(&t.id))
                    .map(|t| t.id.clone())
                    .collect();
                s.terminals.retain(|t| !still.contains(&t.id));
                self.closed = still;
                let fresh = self.terminals.replace(s.terminals, &s.ended, decided, next);
                self.headless.replace(s.headless, &[], false, next);
                for id in &fresh {
                    self.seed(id);
                }
                if let Some(st) = &mut self.statuses {
                    let terminals = &self.terminals;
                    st.idle_at
                        .retain(|id, _| terminals.get(id).is_some_and(|r| !r.ended));
                }
                self.order = s.order;
                self.holds = s.holds;
                self.rev = next;
                let mut note = self.snapshot();
                note["op"] = json!("snapshot");
                return Some(note);
            }
        };
        self.rev = next;
        Some(self.note(fields))
    }

    /// A note at the current revision, with `fields` after the generation and
    /// revision.
    fn note(&self, fields: Value) -> Value {
        let mut note = json!({ "gen": self.gen, "rev": self.rev });
        if let (Value::Object(note), Value::Object(fields)) = (&mut note, fields) {
            note.extend(fields);
        }
        note
    }

    /// Moves the revision on for a change the registry decided to terminal
    /// `id`, and answers its note.
    fn decided(&mut self, id: &str) -> Option<Value> {
        let next = Rev(self.rev.0 + 1);
        let row = self.terminals.get_mut(id)?;
        row.rev = next;
        let record = row.told();
        self.rev = next;
        Some(self.note(json!({ "op": "upsert", "kind": Kind::Terminal, "record": record })))
    }

    /// A new terminal's record starts from what its screen last showed, as
    /// [`Registry::screen_status`] would have set it.
    fn seed(&mut self, id: &str) {
        let Some(&(status, at)) = self.statuses.as_ref().and_then(|st| st.screens.get(id)) else {
            return;
        };
        let Some(row) = self.terminals.get_mut(id) else {
            return;
        };
        if takes_screen(row) && !stale(Some(at), row.status_at) {
            row.record.status = status;
            row.status_at = Some(at);
        }
    }

    /// Terminal `id` no longer goes quiet: its program ended or its record
    /// went.
    fn quiet(&mut self, id: &str) {
        if let Some(st) = &mut self.statuses {
            st.idle_at.remove(id);
        }
    }

    /// Sets terminal `id`'s status at `at`, unless what it holds is newer or
    /// already that. Answers the note when it changed.
    fn set_status(&mut self, id: &str, status: AgentStatus, at: Option<Stamp>) -> Option<Value> {
        let row = self.terminals.get_mut(id)?;
        if row.ended || row.record.status == status || stale(at, row.status_at) {
            return None;
        }
        row.record.status = status;
        row.status_at = at;
        self.decided(id)
    }

    /// The terminal a call from the server names, still running. One whose
    /// program ended keeps the status it ended with.
    fn running(
        &self,
        call: &'static str,
        id: &str,
    ) -> Result<Option<&Row<TerminalSession>>, RegistryError> {
        if !self.decides() {
            return Err(RegistryError::NotDeciding { call });
        }
        match self.terminals.get(id) {
            Some(row) => Ok((!row.ended).then_some(row)),
            None => Err(RegistryError::NoTerminal {
                call,
                id: id.to_owned(),
            }),
        }
    }

    /// A status effect of session `id`'s screen. It sets the terminal's
    /// status unless the terminal is a shell or its hooks report its status,
    /// and is kept either way, for the activity rule and for a record put in
    /// later.
    pub fn screen_status(&mut self, id: &str, code: u32, at: Stamp) -> Option<Value> {
        let status = AgentStatus::from_code(code)?;
        let st = self.statuses.as_mut()?;
        match st.screens.get(id) {
            // Told again by a replay: what is held is newer.
            Some(&(_, held)) if at < held => return None,
            _ => {}
        }
        st.screens.insert(id.to_owned(), (status, at));
        if !self.terminals.get(id).is_some_and(takes_screen) {
            return None;
        }
        self.set_status(id, status, Some(at))
    }

    /// Session `id` printed. A terminal that went idle while its screen still
    /// says running is running again, and its idle timer starts over: 5s, or
    /// 30s while hooks report its status.
    pub fn activity(&mut self, id: &str, head: Option<Stamp>, now: Instant) -> Option<Value> {
        let st = self.statuses.as_ref()?;
        let screen = st.screens.get(id).map(|&(s, _)| s);
        let row = self
            .terminals
            .get(id)
            .filter(|r| !r.ended && !is_shell(r))?;
        let hooks = row.record.status_source == Some(StatusSource::Hooks);
        let wake = row.record.status == AgentStatus::Idle
            && !hooks
            && screen == Some(AgentStatus::Running);
        let at = receipt(head, row.status_at);
        let note = if wake {
            self.set_status(id, AgentStatus::Running, at)
        } else {
            None
        };
        let after = if hooks { IDLE_AFTER_HOOKS } else { IDLE_AFTER };
        if let Some(st) = &mut self.statuses {
            st.idle_at.insert(id.to_owned(), now + after);
        }
        note
    }

    /// The server wrote to terminal `id`: one idle or waiting is running
    /// again, unless its hooks report its status (they say when it runs).
    pub fn input(&mut self, id: &str, head: Option<Stamp>) -> Result<Option<Value>, RegistryError> {
        let Some(row) = self.running("vornd:input", id)? else {
            return Ok(None);
        };
        let woken = row.record.status_source != Some(StatusSource::Hooks)
            && matches!(row.record.status, AgentStatus::Idle | AgentStatus::Waiting);
        let at = receipt(head, row.status_at);
        Ok(if woken {
            self.set_status(id, AgentStatus::Running, at)
        } else {
            None
        })
    }

    /// A status an agent's hook reported, then, with `promote`, the terminal's
    /// status taken from its hooks from now on: what its screen shows no
    /// longer sets it, and its idle timer, if one runs, starts over at 30s.
    pub fn hook_status(
        &mut self,
        call: &HookStatus,
        head: Option<Stamp>,
        now: Instant,
    ) -> Result<Vec<Value>, RegistryError> {
        let id = call.id.as_str();
        let Some(row) = self.running("vornd:hookStatus", id)? else {
            return Ok(Vec::new());
        };
        let at = receipt(head, row.status_at);
        let mut notes = Vec::new();
        if let Some(status) = call.status {
            notes.extend(self.set_status(id, status, at));
        }
        if call.promote {
            if let Some(row) = self.terminals.get_mut(id) {
                if row.record.status_source != Some(StatusSource::Hooks) {
                    row.record.status_source = Some(StatusSource::Hooks);
                    notes.extend(self.decided(id));
                }
            }
            if let Some(timer) = self.statuses.as_mut().and_then(|st| st.idle_at.get_mut(id)) {
                *timer = now + IDLE_AFTER_HOOKS;
            }
        }
        Ok(notes)
    }

    /// Sets fields of [`PATCHABLE`] on a terminal's record.
    pub fn patch(&mut self, call: &Patch) -> Result<Option<Value>, RegistryError> {
        if !self.decides() {
            return Err(RegistryError::NotDeciding {
                call: "vornd:patch",
            });
        }
        Ok(if self.patch_record(call)? {
            self.decided(&call.id)
        } else {
            None
        })
    }

    /// Sets a patch's fields on its record; answers whether it changed.
    fn patch_record(&mut self, call: &Patch) -> Result<bool, RegistryError> {
        const CALL: &str = "vornd:patch";
        let id = call.id.as_str();
        let row = self
            .terminals
            .get_mut(id)
            .ok_or_else(|| RegistryError::NoTerminal {
                call: CALL,
                id: id.to_owned(),
            })?;
        if call.base_rev.is_some_and(|base| base < row.created.0) {
            return Err(RegistryError::Replaced { id: id.to_owned() });
        }
        // Through the record's JSON, so each field is checked as the record
        // reads it, and one that would not read is refused whole.
        let mut v = serde_json::to_value(&row.record).map_err(|e| RegistryError::BadCall {
            call: CALL,
            why: e.to_string(),
        })?;
        if let Value::Object(fields) = &mut v {
            for (k, value) in &call.fields {
                if value.is_null() {
                    fields.remove(k);
                } else {
                    fields.insert(k.clone(), value.clone());
                }
            }
        }
        let patched = TerminalSession::deserialize(&v).map_err(|e| RegistryError::BadCall {
            call: CALL,
            why: e.to_string(),
        })?;
        if patched == row.record {
            return Ok(false);
        }
        row.record = patched;
        Ok(true)
    }

    /// The engine let go of session `id`: what its screen showed goes with
    /// it, so a session started again under the id starts from nothing.
    pub fn session_closed(&mut self, id: &str) {
        if let Some(st) = &mut self.statuses {
            st.screens.remove(id);
        }
    }

    /// When the next terminal turns idle, if one is waiting to.
    pub fn next_idle(&self) -> Option<Instant> {
        self.statuses.as_ref()?.idle_at.values().min().copied()
    }

    /// Turns idle each terminal whose timer ran out by `now` and that is
    /// still running, stamped at its head (`head`).
    pub fn tick(&mut self, now: Instant, head: impl Fn(&str) -> Option<Stamp>) -> Vec<Value> {
        let Some(st) = &mut self.statuses else {
            return Vec::new();
        };
        let mut due: Vec<String> = st
            .idle_at
            .iter()
            .filter(|(_, &at)| at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        due.sort();
        for id in &due {
            st.idle_at.remove(id);
        }
        due.iter()
            .filter_map(|id| {
                let row = self.terminals.get(id)?;
                if row.record.status != AgentStatus::Running {
                    return None;
                }
                let at = receipt(head(id), row.status_at);
                self.set_status(id, AgentStatus::Idle, at)
            })
            .collect()
    }

    /// Everything, with the generation and revision it stands at: what a
    /// subscriber starts from or resyncs to. Each record carries the
    /// revision it last changed at.
    pub fn snapshot(&self) -> Value {
        let mut snapshot = json!({
            "gen": self.gen,
            "rev": self.rev,
            "terminals": self.terminals.rows.iter().map(Row::told).collect::<Vec<_>>(),
            "headless": self.headless.rows.iter().map(Row::told).collect::<Vec<_>>(),
            "order": self.order,
            "holds": self.holds,
        });
        if !self.own_holds.is_empty() {
            snapshot["nativeHolds"] = json!(self.own_holds);
        }
        snapshot
    }

    /// A `holds` note's fields: the server's holds, and vornd's own beside
    /// them while it has any.
    fn holds_fields(&self) -> Value {
        let mut fields = json!({ "op": "holds", "holds": self.holds });
        if !self.own_holds.is_empty() {
            fields["nativeHolds"] = json!(self.own_holds);
        }
        fields
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

    /// The workspaces vornd holds itself while it prepares a session.
    pub fn own_holds(&self) -> &BTreeMap<String, u32> {
        &self.own_holds
    }

    /// Terminal `id`'s record, and whether its program ended.
    pub fn terminal(&self, id: &str) -> Option<(&TerminalSession, bool)> {
        self.terminals.get(id).map(|r| (&r.record, r.ended))
    }

    /// The terminals whose programs still run (`getLiveSessions`).
    pub fn live_terminals(&self) -> impl Iterator<Item = &TerminalSession> {
        self.terminals
            .rows
            .iter()
            .filter(|r| !r.ended)
            .map(|r| &r.record)
    }

    /// How many terminals run a shell, live or not (`createShellPty`
    /// numbers a new shell after them).
    pub fn shells(&self) -> usize {
        self.terminals
            .records()
            .filter(|t| t.agent_type == "shell")
            .count()
    }
}

/// The changes vornd makes to the terminals itself, with the Native server
/// switch on. Each answers the notes to tell, marked `native`; each needs
/// the registry to decide the statuses, which is what makes the patched
/// fields its own.
impl Registry {
    /// A note at the next revision for a change vornd made, marked so.
    fn native_note(&mut self, mut fields: Value) -> Value {
        self.rev = Rev(self.rev.0 + 1);
        fields["native"] = Value::Bool(true);
        self.note(fields)
    }

    /// Terminal `id`'s row, told whole at the next revision with `extra`.
    fn native_upsert(&mut self, id: &str, extra: Value) -> Option<Value> {
        let next = Rev(self.rev.0 + 1);
        let row = self.terminals.get_mut(id)?;
        row.rev = next;
        let mut fields = json!({ "op": "upsert", "kind": Kind::Terminal, "record": row.told() });
        if let (Value::Object(fields), Value::Object(extra)) = (&mut fields, extra) {
            fields.extend(extra);
        }
        Some(self.native_note(fields))
    }

    /// Puts in a terminal vornd is creating, last in the order. Its program
    /// is not up yet: [`Registry::started`] or [`Registry::failed`] says how
    /// that went.
    pub fn create(&mut self, record: TerminalSession) -> Result<Vec<Value>, RegistryError> {
        const CALL: &str = "terminal:create";
        if !self.decides() {
            return Err(RegistryError::NotDeciding { call: CALL });
        }
        let id = record.id.clone();
        if self.terminals.get(&id).is_some() {
            return Err(RegistryError::BadCall {
                call: CALL,
                why: format!("terminal {id} exists already"),
            });
        }
        self.closed.remove(&id);
        let next = Rev(self.rev.0 + 1);
        self.terminals
            .upsert(record, Stamps::default(), false, true, next);
        self.seed(&id);
        let mut notes = Vec::with_capacity(2);
        notes.extend(self.native_upsert(&id, json!({ "created": true })));
        self.order.push(id);
        let order = json!({ "op": "order", "order": self.order });
        notes.push(self.native_note(order));
        Ok(notes)
    }

    /// Terminal `id`'s program is up as `pid`, its records in `epoch`.
    pub fn started(&mut self, id: &str, pid: u32, epoch: u32) -> Option<Value> {
        let row = self.terminals.get_mut(id)?;
        row.record.pid = pid;
        self.native_upsert(id, json!({ "started": { "pid": pid, "epoch": epoch } }))
    }

    /// Terminal `id`'s program could not be started, and why: the server
    /// ends it as it ends one whose spawn failed.
    pub fn failed(&mut self, id: &str, why: &str) -> Option<Value> {
        self.terminals.get(id)?;
        self.native_upsert(id, json!({ "failed": why }))
    }

    /// Closes terminal `id`: its record and its place in the order go, and
    /// an upsert the server sent before it hears is not taken. Answers the
    /// record as it was, whether its program still ran, and the notes.
    pub fn close(
        &mut self,
        id: &str,
    ) -> Result<(TerminalSession, bool, Vec<Value>), RegistryError> {
        const CALL: &str = "terminal:kill";
        if !self.decides() {
            return Err(RegistryError::NotDeciding { call: CALL });
        }
        let Some((record, ended)) = self.terminal(id).map(|(r, e)| (r.clone(), e)) else {
            return Err(RegistryError::NoTerminal {
                call: CALL,
                id: id.to_owned(),
            });
        };
        self.terminals.remove(id);
        self.quiet(id);
        self.closed.insert(id.to_owned());
        let mut notes = Vec::with_capacity(2);
        let removed = json!({ "op": "remove", "kind": Kind::Terminal, "id": id });
        notes.push(self.native_note(removed));
        if self.order.iter().any(|o| o == id) {
            self.order.retain(|o| o != id);
            let order = json!({ "op": "order", "order": self.order });
            notes.push(self.native_note(order));
        }
        Ok((record, !ended, notes))
    }

    /// Sets the order the terminals are listed in (`reorderSessions`): every
    /// id once, each a terminal the registry holds. Told even when it is the
    /// order there was, as the server tells clients, and marked
    /// `reordered`, so the server takes it as a client's order rather than
    /// one that follows a create or a close.
    pub fn reorder(&mut self, ids: Vec<String>) -> Result<Value, RegistryError> {
        const CALL: &str = "terminal:reorder";
        if !self.decides() {
            return Err(RegistryError::NotDeciding { call: CALL });
        }
        self.check_order(&ids)?;
        self.order = ids;
        let order = json!({ "op": "order", "order": self.order, "reordered": true });
        Ok(self.native_note(order))
    }

    /// Whether `ids` is an order the server would take (`reorderSessions`):
    /// every id once, each a terminal held. Refused in the server's words.
    pub fn check_order(&self, ids: &[String]) -> Result<(), RegistryError> {
        let distinct: HashSet<&str> = ids.iter().map(String::as_str).collect();
        if distinct.len() != ids.len() {
            return Err(RegistryError::Refused("Duplicate session IDs".to_owned()));
        }
        if let Some(missing) = ids.iter().find(|id| self.terminals.get(id).is_none()) {
            return Err(RegistryError::Refused(format!(
                "Session not found: {missing}"
            )));
        }
        Ok(())
    }

    /// Whether terminal `id` is one the server would rename or regroup
    /// (`renameSession`, `setSessionGroup`). Refused in the server's words.
    pub fn check_terminal(&self, id: &str) -> Result<(), RegistryError> {
        if self.terminals.get(id).is_none() {
            return Err(RegistryError::Refused(format!("Session not found: {id}")));
        }
        Ok(())
    }

    /// Sets fields of [`PATCHABLE`] on terminal `id` for a call a client
    /// made (`renameSession`, `setSessionGroup`), refused as the server
    /// refuses it when there is no such terminal. Answers the note when the
    /// record changed.
    pub fn set_fields(
        &mut self,
        id: &str,
        fields: Map<String, Value>,
    ) -> Result<Option<Value>, RegistryError> {
        if !self.decides() {
            return Err(RegistryError::NotDeciding {
                call: "vornd:patch",
            });
        }
        self.check_terminal(id)?;
        let call = Patch {
            id: id.to_owned(),
            fields,
            base_rev: None,
        };
        if !self.patch_record(&call)? {
            return Ok(None);
        }
        Ok(self.native_upsert(id, json!({})))
    }

    /// Holds the workspace at `dir` (already normalized) while a session is
    /// prepared in it. Answers the note.
    pub fn hold(&mut self, dir: &str) -> Value {
        *self.own_holds.entry(dir.to_owned()).or_default() += 1;
        let fields = self.holds_fields();
        self.native_note(fields)
    }

    /// Lets go of one hold on `dir`. Answers the note, or `None` when it
    /// was not held.
    pub fn release(&mut self, dir: &str) -> Option<Value> {
        let left = self.own_holds.get_mut(dir)?;
        *left -= 1;
        if *left == 0 {
            self.own_holds.remove(dir);
        }
        let fields = self.holds_fields();
        Some(self.native_note(fields))
    }
}

fn is_shell(row: &Row<TerminalSession>) -> bool {
    row.record.agent_type == "shell"
}

/// Whether a terminal's status follows its screen: an agent still running
/// whose hooks do not report its status.
fn takes_screen(row: &Row<TerminalSession>) -> bool {
    !row.ended && !is_shell(row) && row.record.status_source != Some(StatusSource::Hooks)
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
        let note = fed.registry.apply(change);
        self.tell(note);
        Ok(())
    }

    /// Tells subscribers each note. Called under the lock, so notes go out in
    /// revision order. No subscriber is not an error.
    fn tell(&self, notes: impl IntoIterator<Item = Value>) {
        for note in notes {
            let _ = self.notes.send(note);
        }
    }

    /// Decides the terminals' statuses from now on
    /// ([`Registry::decide_statuses`]).
    pub fn decide_statuses(&self) {
        self.lock().registry.decide_statuses();
    }

    pub fn decides(&self) -> bool {
        self.lock().registry.decides()
    }

    /// [`Registry::screen_status`].
    pub fn screen_status(&self, id: &str, code: u32, at: Stamp) {
        let mut fed = self.lock();
        let note = fed.registry.screen_status(id, code, at);
        self.tell(note);
    }

    /// [`Registry::activity`].
    pub fn activity(&self, id: &str, head: Option<Stamp>, now: Instant) {
        let mut fed = self.lock();
        let note = fed.registry.activity(id, head, now);
        self.tell(note);
    }

    /// [`Registry::input`], for a `vornd:input {id}` from the server.
    pub fn input(&self, id: &str, head: Option<Stamp>) -> Result<(), RegistryError> {
        let mut fed = self.lock();
        let note = fed.registry.input(id, head)?;
        self.tell(note);
        Ok(())
    }

    /// [`Registry::hook_status`].
    pub fn hook_status(
        &self,
        call: &HookStatus,
        head: Option<Stamp>,
        now: Instant,
    ) -> Result<(), RegistryError> {
        let mut fed = self.lock();
        let notes = fed.registry.hook_status(call, head, now)?;
        self.tell(notes);
        Ok(())
    }

    /// [`Registry::patch`].
    pub fn patch(&self, call: &Patch) -> Result<(), RegistryError> {
        let mut fed = self.lock();
        let note = fed.registry.patch(call)?;
        self.tell(note);
        Ok(())
    }

    /// [`Registry::session_closed`].
    pub fn session_closed(&self, id: &str) {
        self.lock().registry.session_closed(id);
    }

    /// [`Registry::next_idle`].
    pub fn next_idle(&self) -> Option<Instant> {
        self.lock().registry.next_idle()
    }

    /// [`Registry::tick`].
    pub fn tick(&self, now: Instant, head: impl Fn(&str) -> Option<Stamp>) {
        let mut fed = self.lock();
        let notes = fed.registry.tick(now, head);
        self.tell(notes);
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

    /// Makes a change of vornd's own ([`Registry::create`] and the rest)
    /// and tells subscribers its notes, while the registry holds the
    /// server's records and decides the statuses; `None` otherwise, when the
    /// call is the server's to make.
    pub fn change<T>(&self, f: impl FnOnce(&mut Registry) -> (T, Vec<Value>)) -> Option<T> {
        let mut fed = self.lock();
        if fed.feeder.is_none() || !fed.registry.decides() {
            return None;
        }
        let (answer, notes) = f(&mut fed.registry);
        self.tell(notes);
        Some(answer)
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
            "decides": fed.registry.decides(),
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

    fn deciding() -> Registry {
        let mut r = Registry::new(Gen(1));
        r.decide_statuses();
        r.apply(change(json!({
            "op": "snapshot", "terminals": [agent("a"), terminal("sh")], "headless": [],
        })))
        .unwrap();
        r
    }

    fn agent(id: &str) -> Value {
        let mut t = terminal(id);
        t["agentType"] = json!("claude");
        t
    }

    fn at(epoch: u64, rseq: u64, index: u64) -> Stamp {
        Stamp { epoch, rseq, index }
    }

    fn status(r: &Registry, id: &str) -> AgentStatus {
        r.terminals().iter().find(|t| t.id == id).unwrap().status
    }

    fn hook(id: &str, status: Option<&str>, promote: bool) -> HookStatus {
        HookStatus::try_from(&json!({ "id": id, "status": status, "promote": promote })).unwrap()
    }

    const RUNNING: u32 = 1;
    const WAITING: u32 = 2;
    const ERROR: u32 = 3;

    #[test]
    fn a_head_stamp_comes_after_every_effect_of_the_last_record_and_before_the_next() {
        let head = Cursor {
            epoch: 2,
            next_rseq: 41,
            next_offset: 0,
        };
        let s = Stamp::at_head(&head);
        assert!(at(2, 40, 3) < s && s < at(2, 41, 0));
        assert!(at(1, 900, 0) < s && s < at(3, 0, 0));
        let start = Stamp::at_head(&Cursor {
            epoch: 2,
            next_rseq: 0,
            next_offset: 0,
        });
        assert_eq!(start, at(2, 0, 0));
    }

    #[test]
    fn the_screen_sets_an_agents_status_but_not_a_shells_and_a_stale_replay_is_ignored() {
        let mut r = deciding();
        let note = r.screen_status("a", WAITING, at(1, 10, 0)).unwrap();
        assert_eq!(note["record"]["status"], "waiting");
        assert_eq!(
            note["record"]["statusAt"],
            json!({ "epoch": 1, "rseq": 10, "index": 0 })
        );
        // Told again after a reconnect: an earlier record's error.
        assert!(r.screen_status("a", ERROR, at(1, 9, 0)).is_none());
        assert_eq!(status(&r, "a"), AgentStatus::Waiting);
        // A code that says nothing changes nothing.
        assert!(r.screen_status("a", 0, at(1, 11, 0)).is_none());
        assert!(r.screen_status("sh", WAITING, at(1, 3, 0)).is_none());
        assert_eq!(status(&r, "sh"), AgentStatus::Running);
        // Not deciding: nothing is decided.
        let mut plain = Registry::new(Gen(1));
        plain.apply(upsert(agent("a"))).unwrap();
        assert!(plain.screen_status("a", WAITING, at(1, 1, 0)).is_none());
    }

    #[test]
    fn a_hook_beats_the_screen_and_an_older_screen_status_told_after_it() {
        let mut r = deciding();
        r.screen_status("a", WAITING, at(1, 10, 0)).unwrap();
        // The hook arrives while the head is still behind the effect it
        // follows: it is stamped after what was applied, and wins.
        let now = Instant::now();
        let notes = r
            .hook_status(
                &hook("a", Some("running"), true),
                Some(at(1, 9, u64::from(u32::MAX))),
                now,
            )
            .unwrap();
        assert_eq!(notes.len(), 2, "the status, then the promotion");
        assert_eq!(notes[0]["record"]["status"], "running");
        assert!(notes[0]["record"].get("statusSource").is_none());
        assert_eq!(notes[1]["record"]["statusSource"], "hooks");
        assert_eq!(
            notes[1]["rev"].as_u64(),
            notes[0]["rev"].as_u64().map(|r| r + 1)
        );
        // The screen no longer sets it once its hooks do, and a replay of an
        // older one would not have anyway.
        assert!(r.screen_status("a", ERROR, at(1, 12, 0)).is_none());
        assert_eq!(status(&r, "a"), AgentStatus::Running);
        // Before promotion, an older screen status told after the hook loses.
        let mut r = deciding();
        r.hook_status(&hook("a", Some("waiting"), false), Some(at(1, 20, 0)), now)
            .unwrap();
        assert!(r.screen_status("a", RUNNING, at(1, 15, 0)).is_none());
        assert_eq!(status(&r, "a"), AgentStatus::Waiting);
        // A newer one, printed after the hook, wins.
        r.screen_status("a", ERROR, at(1, 21, 0)).unwrap();
        assert_eq!(status(&r, "a"), AgentStatus::Error);
    }

    #[test]
    fn idle_after_five_quiet_seconds_or_thirty_on_hooks() {
        let mut r = deciding();
        let t0 = Instant::now();
        r.screen_status("a", RUNNING, at(1, 1, 0));
        assert!(r.activity("a", None, t0).is_none());
        assert_eq!(r.next_idle(), Some(t0 + IDLE_AFTER));
        assert!(r
            .tick(t0 + Duration::from_millis(4_999), |_| None)
            .is_empty());
        // Printing again restarts the timer.
        r.activity("a", None, t0 + Duration::from_secs(3));
        assert!(r.tick(t0 + IDLE_AFTER, |_| None).is_empty());
        let notes = r.tick(t0 + Duration::from_secs(8), |_| Some(at(1, 4, 9)));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0]["record"]["status"], "idle");
        assert_eq!(notes[0]["record"]["statusAt"]["rseq"], 4);
        assert_eq!(r.next_idle(), None);

        // Printing while idle, the screen still saying running: running.
        let t1 = t0 + Duration::from_secs(20);
        let note = r.activity("a", Some(at(1, 5, 0)), t1).unwrap();
        assert_eq!(note["record"]["status"], "running");
        // A shell never goes idle by the clock.
        assert!(r.activity("sh", None, t1).is_none());
        assert_eq!(r.next_idle(), Some(t1 + IDLE_AFTER));

        // Promoted: the timer running starts over at thirty seconds.
        r.hook_status(&hook("a", None, true), None, t1 + Duration::from_secs(1))
            .unwrap();
        assert_eq!(r.next_idle(), Some(t1 + Duration::from_secs(31)));
        assert!(r.tick(t1 + Duration::from_secs(10), |_| None).is_empty());
        r.activity("a", None, t1 + Duration::from_secs(2));
        assert_eq!(r.next_idle(), Some(t1 + Duration::from_secs(32)));
        // Hooks report it idle: the timer finds nothing running to stop.
        r.hook_status(&hook("a", Some("idle"), false), None, t1)
            .unwrap();
        assert!(r.tick(t1 + Duration::from_secs(40), |_| None).is_empty());
        // And printing does not wake it: its hooks say when it runs.
        assert!(r
            .activity("a", None, t1 + Duration::from_secs(41))
            .is_none());
    }

    #[test]
    fn input_wakes_an_idle_or_waiting_terminal_unless_its_hooks_report() {
        let mut r = deciding();
        assert!(r.input("a", None).unwrap().is_none());
        r.screen_status("a", WAITING, at(1, 1, 0));
        assert_eq!(
            r.input("a", None).unwrap().unwrap()["record"]["status"],
            "running"
        );
        r.hook_status(&hook("a", Some("idle"), true), None, Instant::now())
            .unwrap();
        assert!(r.input("a", None).unwrap().is_none());
        assert!(matches!(
            r.input("nope", None),
            Err(RegistryError::NoTerminal { .. })
        ));
        assert!(matches!(
            Registry::new(Gen(1)).input("a", None),
            Err(RegistryError::NotDeciding { .. })
        ));
    }

    #[test]
    fn the_servers_upserts_keep_what_the_registry_decides_until_the_program_ends() {
        let mut r = deciding();
        r.screen_status("a", WAITING, at(1, 5, 0));
        r.patch(
            &Patch::try_from(&json!({ "id": "a", "fields": { "hookSessionId": "conv" } })).unwrap(),
        )
        .unwrap();
        // The server sends its record, which says running and has no link.
        let mut resized = agent("a");
        resized["cols"] = json!(120);
        let note = r.apply(upsert(resized)).unwrap();
        assert_eq!(note["record"]["status"], "waiting");
        assert_eq!(note["record"]["hookSessionId"], "conv");
        assert_eq!(note["record"]["cols"], 120);
        // The same record again is no change.
        let mut resized = agent("a");
        resized["cols"] = json!(120);
        assert!(r.apply(upsert(resized)).is_none());

        // It ended: the server's record is its own again.
        let mut ended = agent("a");
        ended["status"] = json!("idle");
        let note = r
            .apply(change(
                json!({ "op": "upsert", "kind": "terminal", "record": ended, "ended": true }),
            ))
            .unwrap();
        assert_eq!(note["record"]["status"], "idle");
        assert!(note["record"].get("hookSessionId").is_none());
        // And nothing decides it any more.
        assert!(r.screen_status("a", RUNNING, at(1, 9, 0)).is_none());
        assert!(r.input("a", None).unwrap().is_none());
        assert!(r
            .hook_status(&hook("a", Some("running"), true), None, Instant::now())
            .unwrap()
            .is_empty());
        assert_eq!(status(&r, "a"), AgentStatus::Idle);
    }

    #[test]
    fn a_record_put_in_later_starts_from_what_its_screen_showed() {
        let mut r = deciding();
        r.screen_status("late", WAITING, at(3, 2, 0));
        let note = r.apply(upsert(agent("late"))).unwrap();
        assert_eq!(note["record"]["status"], "waiting");
        // Unless the engine let the session go in between.
        r.screen_status("gone", WAITING, at(3, 2, 0));
        r.session_closed("gone");
        assert_eq!(
            r.apply(upsert(agent("gone"))).unwrap()["record"]["status"],
            "running"
        );
        // A snapshot keeps what it decided of the sessions it already had.
        r.screen_status("a", ERROR, at(1, 1, 0));
        r.apply(change(json!({
            "op": "snapshot", "terminals": [agent("a"), agent("late")], "headless": [],
        })))
        .unwrap();
        assert_eq!(status(&r, "a"), AgentStatus::Error);
        assert_eq!(status(&r, "late"), AgentStatus::Waiting);
    }

    #[test]
    fn a_patch_sets_only_the_fields_it_may_and_not_on_a_record_made_since() {
        let mut r = deciding();
        let patch = |v: Value| Patch::try_from(&v);
        let note = r
            .patch(&patch(json!({ "id": "a", "fields": { "groupId": "g", "renamedByPerson": true }, "baseRev": 1 })).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                note["record"]["groupId"].as_str(),
                note["record"]["renamedByPerson"].as_bool()
            ),
            (Some("g"), Some(true))
        );
        // Null takes a field away; the same patch twice is no change.
        let ungroup = patch(json!({ "id": "a", "fields": { "groupId": null } })).unwrap();
        assert!(r.patch(&ungroup).unwrap().is_some());
        assert!(r.patch(&ungroup).unwrap().is_none());
        for (bad, says) in [
            (
                json!({ "id": "a", "fields": { "status": "idle" } }),
                "not a field",
            ),
            (json!({ "id": "a" }), "needs fields"),
            (json!({ "fields": {} }), "needs an id"),
        ] {
            assert!(patch(bad).unwrap_err().to_string().contains(says));
        }
        let wrong = patch(json!({ "id": "a", "fields": { "statusSource": "screen" } })).unwrap();
        assert!(r.patch(&wrong).is_err());
        assert_eq!(r.terminals()[0].status_source, None);
        // The record was let go and made again under the same id.
        r.apply(change(
            json!({ "op": "remove", "kind": "terminal", "id": "a" }),
        ))
        .unwrap();
        r.apply(upsert(agent("a"))).unwrap();
        let old = patch(json!({ "id": "a", "fields": { "groupId": "g" }, "baseRev": 2 })).unwrap();
        assert_eq!(
            r.patch(&old),
            Err(RegistryError::Replaced { id: "a".into() })
        );
    }

    fn record(id: &str) -> TerminalSession {
        serde_json::from_value(agent(id)).unwrap()
    }

    #[test]
    fn a_terminal_vornd_creates_is_told_as_its_own_then_started_or_failed() {
        let mut r = deciding();
        let notes = r.create(record("n")).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(
            (notes[0]["op"].as_str(), notes[0]["created"].as_bool()),
            (Some("upsert"), Some(true))
        );
        assert_eq!(notes[0]["native"], true);
        assert_eq!(notes[1]["order"], json!(["n"]));
        assert_eq!(
            notes[1]["rev"].as_u64(),
            notes[0]["rev"].as_u64().map(|r| r + 1)
        );
        // Twice under one id is refused.
        assert!(r.create(record("n")).is_err());

        let note = r.started("n", 42, 3).unwrap();
        assert_eq!(note["record"]["pid"], 42);
        assert_eq!(note["started"], json!({ "pid": 42, "epoch": 3 }));
        let note = r.failed("n", "no shell").unwrap();
        assert_eq!(note["failed"], "no shell");
        assert!(r.started("gone", 1, 1).is_none());

        // The server's echo of what it was told changes nothing.
        let mut echo = agent("n");
        echo["pid"] = json!(42);
        assert!(r.apply(upsert(echo)).is_none());
        // Not deciding: the server creates its own.
        assert!(Registry::new(Gen(1)).create(record("x")).is_err());
    }

    #[test]
    fn a_terminal_vornd_closed_stays_closed_until_the_server_lets_go_of_it() {
        let mut r = deciding();
        r.apply(change(json!({ "op": "order", "order": ["a", "sh"] })))
            .unwrap();
        let (closed, live, notes) = r.close("a").unwrap();
        assert_eq!((closed.id.as_str(), live), ("a", true));
        assert_eq!(notes[0]["op"], "remove");
        assert_eq!(notes[1]["order"], json!(["sh"]));
        assert!(notes.iter().all(|n| n["native"] == true));
        assert!(matches!(
            r.close("a"),
            Err(RegistryError::NoTerminal { .. })
        ));
        // An upsert the server sent before it heard is not taken.
        assert!(r.apply(upsert(agent("a"))).is_none());
        // Nor kept by a snapshot sent meanwhile.
        r.apply(change(json!({
            "op": "snapshot", "terminals": [agent("a"), terminal("sh")], "headless": [],
        })))
        .unwrap();
        assert!(r.terminal("a").is_none());
        // The server lets go of it: it heard, and the id is free again.
        assert!(r
            .apply(change(
                json!({ "op": "remove", "kind": "terminal", "id": "a" })
            ))
            .is_none());
        assert!(r.apply(upsert(agent("a"))).is_some());

        // A terminal whose program ended is closed without a signal.
        let mut ended = terminal("sh");
        ended["status"] = json!("idle");
        r.apply(change(
            json!({ "op": "upsert", "kind": "terminal", "record": ended, "ended": true }),
        ))
        .unwrap();
        let (_, live, notes) = r.close("sh").unwrap();
        assert!(!live);
        // It was not in the order any more: no order note.
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn a_reorder_is_checked_as_the_server_checks_it_and_always_told() {
        let mut r = deciding();
        let ids = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            r.reorder(ids(&["a", "a"])).unwrap_err().to_string(),
            "Duplicate session IDs"
        );
        assert_eq!(
            r.reorder(ids(&["a", "x"])).unwrap_err().to_string(),
            "Session not found: x"
        );
        let note = r.reorder(ids(&["sh", "a"])).unwrap();
        assert_eq!(
            (note["order"].clone(), note["reordered"].clone()),
            (json!(["sh", "a"]), json!(true))
        );
        // The same order again is told again.
        let again = r.reorder(ids(&["sh", "a"])).unwrap();
        assert_eq!(again["rev"].as_u64(), note["rev"].as_u64().map(|r| r + 1));
        assert_eq!(
            r.terminals()
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            ["sh", "a"]
        );
    }

    #[test]
    fn fields_vornd_sets_stay_whatever_the_server_sends_until_it_sets_them() {
        let mut r = deciding();
        let mut fields = Map::new();
        fields.insert("displayName".into(), json!("mine"));
        fields.insert("renamedByPerson".into(), json!(true));
        let note = r.set_fields("a", fields.clone()).unwrap().unwrap();
        assert_eq!(note["record"]["displayName"], "mine");
        assert_eq!(note["native"], true);
        // The same fields again change nothing.
        assert!(r.set_fields("a", fields).unwrap().is_none());
        assert_eq!(
            r.set_fields("x", Map::new()).unwrap_err().to_string(),
            "Session not found: x"
        );
        // An upsert the server sent with the old name, and a snapshot, and
        // one after the program ended: the name stays.
        let mut stale = agent("a");
        stale["displayName"] = json!("old");
        stale["cols"] = json!(99);
        let note = r.apply(upsert(stale.clone())).unwrap();
        assert_eq!(note["record"]["displayName"], "mine");
        assert_eq!(note["record"]["cols"], 99);
        r.apply(change(json!({
            "op": "snapshot", "terminals": [stale.clone(), terminal("sh")], "headless": [],
        })))
        .unwrap();
        assert_eq!(
            r.terminal("a").unwrap().0.display_name.as_deref(),
            Some("mine")
        );
        r.apply(change(
            json!({ "op": "upsert", "kind": "terminal", "record": stale, "ended": true }),
        ))
        .unwrap();
        let (a, ended) = r.terminal("a").unwrap();
        assert!(ended);
        assert_eq!(a.display_name.as_deref(), Some("mine"));
        assert_eq!(a.renamed_by_person, Some(true));
    }

    #[test]
    fn vornds_own_holds_are_told_beside_the_servers() {
        let mut r = deciding();
        r.apply(change(json!({ "op": "holds", "holds": { "/s": 1 } })))
            .unwrap();
        let note = r.hold("/w");
        assert_eq!(
            (note["holds"].clone(), note["nativeHolds"].clone()),
            (json!({ "/s": 1 }), json!({ "/w": 1 }))
        );
        r.hold("/w");
        assert_eq!(r.snapshot()["nativeHolds"], json!({ "/w": 2 }));
        assert_eq!(r.release("/w").unwrap()["nativeHolds"], json!({ "/w": 1 }));
        let note = r.release("/w").unwrap();
        assert!(note.get("nativeHolds").is_none());
        assert!(r.release("/w").is_none());
        assert!(r.snapshot().get("nativeHolds").is_none());
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

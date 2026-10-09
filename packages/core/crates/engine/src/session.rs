//! One session's actor: everything vornd does with a session's records, as
//! a state machine with no I/O of its own. It is fed what sessiond sends
//! ([`Input`]) and answers with what to send back and what happened
//! ([`Out`]); [`crate::Pool`] runs it on a worker thread, and tests drive it
//! directly.
//!
//! On open it picks a restore base: the newest checkpoint, then the
//! fallback, then the session start. A checkpoint qualifies when this engine
//! reads its format, its CRC matches, it decodes, and the terminal rebuilt
//! from it passes the restore check. The records after the base are then
//! replayed in replay mode, which writes nothing to the program: query
//! replies are dropped, and a bell or clipboard write is skipped when the
//! dead vornd's `sent` cursor includes its record, as that vornd may have
//! acted on it. When no base qualifies, the session carries on from the
//! best it has and is marked [`Fidelity::Approximate`].
//!
//! Every effect is numbered by the record that caused it and its place in
//! that record ([`EffectId`]). Replay from a valid base is the live
//! computation repeated, so the same effect gets the same id, and receivers
//! drop the repeats of at-least-once effects by it.
//!
//! Grid clients are served from here too, on the thread that owns the
//! terminal: the session's [`Hub`] cuts their frames after each batch of
//! records and on the render clock ([`Session::due`], [`Session::frame`]),
//! and answers their requests ([`Input::Grid`]) with [`Out::Grid`].
//!
//! A live session left idle, with no output, request or viewer for
//! [`Config::idle`], puts its terminal away ([`Session::sleep`]): it keeps
//! the checkpoint blob of a checked cut and drops the terminal, the analyzer
//! and the grid. The next record, attach, snapshot or read decodes the blob
//! first, which rebuilds the terminal the cut swapped in, so nothing a
//! client or a recovery sees tells the two apart.

use std::sync::Arc;
use std::time::{Duration, Instant};

use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use vorn_grid::{Ctx as GridCtx, GridIn, Hub, HubConfig, HubOut};
use vorn_pipeline::history::History;
use vorn_screen::{ClipboardTarget, Emulator};
use vorn_sessiond_wire::{AttachFrom, AttachRefusal, Checkpoint, Kind, SessionInfo};
use vorn_term_proto::msg::{self, EventId as WireEventId, EventKind};
use vorn_term_proto::{Cursor, Entry, Record, RecordHeader};

use crate::snapshot::VtSnapshot;
use crate::term::{Fidelity, Packed, Rejected, Term, FORMAT};

/// The size a piped agent's output is parsed at. It has no terminal, so
/// nothing ever resizes it.
pub const PIPED_SIZE: (u16, u16) = (120, 40);

/// When to cut a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
    /// Output since the last checkpoint that earns the next one, at the
    /// next record boundary where one can be cut.
    pub bytes: u64,
    /// Or this long after output goes quiet...
    pub quiet: Duration,
    /// ...with at least this much new since the last one.
    pub quiet_bytes: u64,
}

impl Default for Cadence {
    fn default() -> Self {
        Cadence {
            bytes: 1 << 20,
            quiet: Duration::from_secs(5),
            quiet_bytes: 4 << 10,
        }
    }
}

/// What every session on an engine shares.
#[derive(Debug, Clone)]
pub struct Config {
    /// Ghostty's history limit per terminal, in bytes of page memory.
    /// Checkpoints carry the history, so it costs at every cut.
    pub scrollback: usize,
    pub cadence: Cadence,
    /// Whether the analyzer matches status patterns, or only reads
    /// bracketed paste (hook-driven agents).
    pub analyze: bool,
    /// Where each session's disk history log goes, as `<session>.log`.
    pub history: Option<std::path::PathBuf>,
    /// The cap on one history log segment; a session keeps two at most.
    pub history_cap: u64,
    /// This build, stamped on checkpoints for diagnostics.
    pub build: String,
    /// Grid mode's render clock, hold and credits.
    pub grid: HubConfig,
    /// Hand each batch of records back once applied ([`Out::Applied`]), for
    /// a host that streams them to bytes clients.
    pub stream: bool,
    /// Default foreground and background, as RGB: what OSC 10 and 11
    /// queries are answered with. None leaves them unset, and those queries
    /// unanswered.
    pub colors: Option<([u8; 3], [u8; 3])>,
    /// How long a live session goes with no output, request or viewer
    /// before its terminal is put away as a checkpoint ([`Session::sleep`]).
    /// None keeps every terminal live.
    pub idle: Option<Duration>,
    /// Whether a session has viewers the engine does not see itself (a
    /// host's bytes clients); one with any is never put away.
    pub viewed: Option<Viewed>,
}

/// Asked of a session by id: see [`Config::viewed`].
#[derive(Clone)]
pub struct Viewed(pub Arc<dyn Fn(&str) -> bool + Send + Sync>);

impl std::fmt::Debug for Viewed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Viewed")
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            scrollback: 0,
            cadence: Cadence::default(),
            analyze: true,
            history: None,
            history_cap: vorn_pipeline::history::DEFAULT_CAP,
            build: String::new(),
            grid: HubConfig::default(),
            stream: false,
            colors: None,
            idle: None,
            viewed: None,
        }
    }
}

/// What vornd knows about a session when it takes it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Open {
    pub epoch: u32,
    /// The oldest record sessiond still holds.
    pub oldest: Cursor,
    /// Where replay ends: everything before it arrived while no vornd was
    /// there to see it live.
    pub head: Cursor,
    /// After the last record sessiond wrote to the dead vornd.
    pub sent: Cursor,
    pub newest_cp: Option<Cursor>,
    /// The size now.
    pub size: (u16, u16),
    /// The size it was spawned at, when known. Replay from the session
    /// start is exact only from that size: a session that was resized
    /// before vornd learned it recovers as approximate.
    pub spawn_size: Option<(u16, u16)>,
    /// A PTY, which can be nudged to redraw; a piped agent cannot.
    pub pty: bool,
    /// How the program ended, when it has: a checkpoint cut after the Exit
    /// record leaves no record to learn it from.
    pub exited: Option<(Option<i32>, Option<i32>)>,
}

impl Open {
    /// A session from sessiond's Welcome.
    pub fn from_info(info: &SessionInfo) -> Open {
        let pty = info.kind == Kind::Pty;
        let (size, spawn_size) = if pty {
            ((info.cols, info.rows), None)
        } else {
            (PIPED_SIZE, Some(PIPED_SIZE))
        };
        Open {
            epoch: info.epoch,
            oldest: info.oldest,
            head: info.head,
            sent: info.sent,
            newest_cp: info.newest_cp,
            size,
            spawn_size,
            pty,
            exited: info.exited.map(|x| (x.code, x.signal)),
        }
    }

    /// A session this vornd just spawned: a PTY at `size`, or a piped
    /// agent when `size` is `None`.
    pub fn spawned(start: Cursor, size: Option<(u16, u16)>) -> Open {
        let pty = size.is_some();
        let size = size.unwrap_or(PIPED_SIZE);
        Open {
            epoch: start.epoch,
            oldest: start,
            head: start,
            sent: start,
            newest_cp: None,
            size,
            spawn_size: Some(size),
            pty,
            exited: None,
        }
    }
}

/// What sessiond sent about a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Checkpoint(Checkpoint),
    Refused(AttachRefusal),
    Entries(Vec<Entry>),
    /// A grid client's request, from vornd's grid listener.
    Grid(GridIn),
}

/// The restore base a session runs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Base {
    Newest,
    Fallback,
    SessionStart,
    /// A checkpoint that rebuilt but failed the restore check, used when
    /// nothing qualified.
    Best,
    /// The oldest record retained, into a blank terminal, when nothing
    /// qualified and no checkpoint rebuilt.
    Oldest,
}

impl Base {
    pub fn as_str(self) -> &'static str {
        match self {
            Base::Newest => "newest checkpoint",
            Base::Fallback => "fallback checkpoint",
            Base::SessionStart => "session start",
            Base::Best => "best checkpoint",
            Base::Oldest => "oldest record",
        }
    }
}

/// Names an effect the same way in every replay of the same records.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EffectId {
    pub session: String,
    pub epoch: u32,
    /// The record that caused it.
    pub rseq: u64,
    /// Its place among that record's effects, counting query replies.
    pub index: u32,
}

/// What a session asks of the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// At most once: never for a record the dead vornd was sent.
    Bell,
    /// At most once, like the bell.
    Clipboard {
        location: ClipboardTarget,
        contents: Vec<(String, String)>,
    },
    /// At least once; receivers drop repeats by id.
    Notify { title: String, body: String },
    /// A state: the cwd from here on.
    Cwd(String),
    /// A state: the agent status from here on, one of
    /// `vorn_analysis::STATUS_*`.
    Status(u32),
    /// A state: the program ended.
    Exit {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

/// What the session wants done, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Attach(AttachFrom),
    /// Every record up to here is in the terminal.
    Ack(Cursor),
    /// For sessiond to keep (PutCheckpoint). Recovery from it does not
    /// repeat the effects of the records it covers, so the host stores it
    /// only after delivering every [`Out::Effect`] that came before it.
    Checkpoint(Checkpoint),
    /// Bytes for the program: replies to queries parsed live.
    Write(Vec<u8>),
    /// A resize for the program, sent after output was lost so a
    /// full-screen program repaints: once a row shorter, then back.
    Nudge {
        cols: u16,
        rows: u16,
    },
    Effect(EffectId, Effect),
    /// Replay reached the head: the session is live from here on.
    Ready(Fidelity),
    /// The session cannot go on; the reason is in its summary.
    Lost,
    /// The session has left the engine: its program ended and every record
    /// is applied, or it was lost. How it stood last. Nothing more comes
    /// for it.
    Closed(Box<Summary>),
    /// For grid clients: a message for a connection, or input bytes for
    /// the program.
    Grid(HubOut),
    /// The records of one [`Input::Entries`], given back once applied, when
    /// [`Config::stream`] is set. Some may have been skipped as already
    /// applied; they are real records of the log all the same.
    Applied(Vec<Entry>),
    /// The answer to [`Session::snapshot`] with the same token: `None` when
    /// the session has no terminal to cut one from.
    Snapshot(u64, Option<Box<VtSnapshot>>),
    /// The answer to [`Session::output`] with the same token.
    Output(u64, Option<Vec<String>>),
    /// When [`Config::stream`] is set, once per base, after [`Out::Ready`]
    /// and after the [`Out::Applied`] of the call that went live: where the
    /// terminal stands then, which a host that streams records cannot learn
    /// otherwise when nothing came after the base.
    Live(Cursor),
    /// The session's terminal was put away ([`Session::sleep`]): a host may
    /// drop what it keeps beside it for viewers, who wake it by asking.
    Asleep,
}

/// Where a session is, for the debug report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Attaching,
    Replaying,
    Live,
    /// The program ended and every record is applied.
    Ended,
    Lost,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Attaching => "attaching",
            State::Replaying => "replaying",
            State::Live => "live",
            State::Ended => "ended",
            State::Lost => "lost",
        }
    }
}

/// Where a session is and how it was recovered, never what is on its
/// screen: what the debug report shows. Cheap to take, with no formatting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    pub session: String,
    pub state: State,
    pub base: Option<Base>,
    pub fidelity: Fidelity,
    /// Why the session is approximate or lost.
    pub reason: Option<&'static str>,
    /// Restore bases tried and turned down, in order, and why.
    pub rejected: Vec<String>,
    pub cursor: Option<Cursor>,
    pub cols: u16,
    pub rows: u16,
    pub checkpoints: u64,
    /// Why the last checkpoint due was not cut.
    pub uncut: Option<&'static str>,
    pub exited: Option<(Option<i32>, Option<i32>)>,
    /// Live with its terminal put away until something asks for it.
    pub asleep: bool,
}

/// A session with its contents, for tests and in-process callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub brief: Brief,
    pub title: String,
    pub cwd: String,
    /// The screen as plain text.
    pub screen: String,
    /// The analyzer's last completed lines.
    pub lines: Vec<String>,
    /// The terminal's [`Emulator::state_digest`], once there is a terminal:
    /// what a test compares with a terminal that never died. Hashing the
    /// whole state costs a walk of every cell, so [`Brief`] leaves it out.
    pub digest: Option<u64>,
}

/// A restore base being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Newest,
    Fallback,
    Start,
}

enum Phase {
    /// Waiting for sessiond's answer to an attach at this step.
    Attaching(Step),
    Running(Box<Run>),
    /// Live and idle, with the terminal put away until something asks.
    Asleep(Box<Run<Packed>>),
    Lost,
}

/// A session with a terminal, applying records. Asleep, `term` is the
/// terminal packed; everything else stays as it was.
struct Run<T = Term> {
    term: T,
    base: Base,
    /// After the last record applied.
    cursor: Cursor,
    /// Whether a record has been applied since the base. Until then,
    /// records that do not follow the base are left over from an attach
    /// that was turned down, and are dropped.
    started: bool,
    live: bool,
    size_known: bool,
    /// Checkpoints are cut only past this, so one never replaces a newer
    /// one sessiond holds.
    floor: Option<Cursor>,
    since_cut: u64,
    last_output: Instant,
    /// The last output or request, or when the last viewer went: the idle
    /// clock.
    last_active: Instant,
    /// Seen with a viewer at the last idle check.
    watched: bool,
    history: Option<History>,
    /// A redraw nudge owed to the program once the session is live.
    nudge: bool,
    /// The session's grid clients, carried from one run to the next.
    hub: Hub,
    /// Whether [`Out::Live`] was sent for going live.
    live_reported: bool,
}

impl<T> Run<T> {
    /// The same run with `term` in place of its terminal, and the old one.
    fn with_term<U>(self, term: U) -> (T, Run<U>) {
        let Run {
            term: old,
            base,
            cursor,
            started,
            live,
            size_known,
            floor,
            since_cut,
            last_output,
            last_active,
            watched,
            history,
            nudge,
            hub,
            live_reported,
        } = self;
        let run = Run {
            term,
            base,
            cursor,
            started,
            live,
            size_known,
            floor,
            since_cut,
            last_output,
            last_active,
            watched,
            history,
            nudge,
            hub,
            live_reported,
        };
        (old, run)
    }

    /// Whether the cursor is past the newest checkpoint sessiond holds, so
    /// one cut here would not replace a newer one.
    fn past_floor(&self) -> bool {
        !self
            .floor
            .is_some_and(|f| f.epoch == self.cursor.epoch && self.cursor.next_rseq <= f.next_rseq)
    }
}

pub struct Session {
    id: String,
    cfg: Arc<Config>,
    open: Open,
    phase: Phase,
    /// Bases still to try, the next one last.
    steps: Vec<Step>,
    best: Option<(Box<Term>, Cursor)>,
    rejected: Vec<String>,
    reason: Option<&'static str>,
    checkpoints: u64,
    uncut: Option<&'static str>,
    exited: Option<(Option<i32>, Option<i32>)>,
    /// Reused for each record's effects.
    fx: Vec<vorn_screen::Effect>,
    /// Grid requests that arrived before there was a terminal.
    waiting: Vec<GridIn>,
    /// Snapshots asked for and not cut yet, and when each was asked.
    snapshots: Vec<(u64, Instant)>,
}

impl Session {
    /// Takes on a session and asks for its first restore base.
    pub fn open(
        id: &str,
        cfg: Arc<Config>,
        open: Open,
        now: Instant,
        out: &mut Vec<Out>,
    ) -> Session {
        let steps = if open.newest_cp.is_some() {
            vec![Step::Start, Step::Fallback, Step::Newest]
        } else {
            vec![Step::Start]
        };
        let mut s = Session::new(id, cfg, open, steps);
        s.next_step(now, out);
        s.report_live(out);
        s
    }

    /// A session that starts here, live, at `size`, with nothing before
    /// `start`: what a harness runs.
    pub fn fresh(
        id: &str,
        cfg: Arc<Config>,
        size: (u16, u16),
        start: Cursor,
    ) -> vorn_screen::Result<Session> {
        let term = Term::fresh(size.0, size.1, cfg.scrollback)?;
        let mut s = Session::new(id, cfg, Open::spawned(start, Some(size)), Vec::new());
        s.run(
            term,
            start,
            Base::SessionStart,
            true,
            Instant::now(),
            &mut Vec::new(),
        );
        Ok(s)
    }

    /// A session back from a checkpoint, live: what a harness runs.
    pub fn restored(id: &str, cfg: Arc<Config>, cp: &Checkpoint) -> Result<Session, Rejected> {
        let term = check(cp).map_err(|(why, _)| why)?;
        let size = (term.em.cols(), term.em.rows());
        let mut open = Open::spawned(cp.resume, Some(size));
        open.newest_cp = Some(cp.resume);
        let mut s = Session::new(id, cfg, open, Vec::new());
        s.run(
            term,
            cp.resume,
            Base::Newest,
            true,
            Instant::now(),
            &mut Vec::new(),
        );
        Ok(s)
    }

    fn new(id: &str, cfg: Arc<Config>, open: Open, steps: Vec<Step>) -> Session {
        Session {
            id: id.to_owned(),
            cfg,
            open,
            phase: Phase::Lost,
            steps,
            best: None,
            rejected: Vec::new(),
            reason: None,
            checkpoints: 0,
            uncut: None,
            exited: open.exited,
            fx: Vec::new(),
            waiting: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The terminal, once there is one and while it is not put away.
    pub fn emulator(&self) -> Option<&Emulator> {
        match &self.phase {
            Phase::Running(r) => Some(&r.term.em),
            _ => None,
        }
    }

    /// The terminal, taken out of a session that is done with.
    pub fn into_emulator(self) -> Option<Emulator> {
        match self.phase {
            Phase::Running(r) => Some(r.term.em),
            Phase::Asleep(r) => r.term.wake().ok().map(|t| t.em),
            _ => None,
        }
    }

    pub fn fidelity(&self) -> Fidelity {
        match &self.phase {
            Phase::Running(r) => r.term.fidelity,
            Phase::Asleep(r) => r.term.fidelity,
            _ => Fidelity::Approximate,
        }
    }

    /// Whether the session's terminal is put away ([`Session::sleep`]).
    pub fn asleep(&self) -> bool {
        matches!(self.phase, Phase::Asleep(_))
    }

    /// Puts the terminal away when the session has been idle for
    /// [`Config::idle`]: live, with no output, request or viewer in that
    /// time and nothing waiting on it. What is kept is a checkpoint; one
    /// past sessiond's newest goes to it too ([`Out::Checkpoint`]). Answers
    /// whether it did. Anything that needs the terminal wakes it first.
    pub fn sleep(&mut self, now: Instant, out: &mut Vec<Out>) -> bool {
        let Some(idle) = self.cfg.idle else {
            return false;
        };
        let Phase::Running(run) = &mut self.phase else {
            return false;
        };
        if !run.live || self.exited.is_some() || !self.snapshots.is_empty() {
            return false;
        }
        if !run.watched && now.duration_since(run.last_active) < idle {
            return false;
        }
        // Only a session idle that long is asked about, then at every tick
        // while it has viewers: the idle clock starts again when they go.
        let viewed =
            run.hub.attached() || self.cfg.viewed.as_ref().is_some_and(|v| (v.0)(&self.id));
        if viewed {
            run.watched = true;
            return false;
        }
        if std::mem::take(&mut run.watched) {
            run.last_active = now;
            return false;
        }
        run.hub.before_swap(&run.term.em);
        let packed = run.term.pack();
        run.hub.after_swap(&run.term.em);
        let packed = match packed {
            Ok(p) => p,
            Err(why) => {
                // Tried again after another idle spell, not at every tick.
                self.uncut = Some(why);
                run.last_active = now;
                return false;
            }
        };
        if run.past_floor() {
            out.push(Out::Checkpoint(Checkpoint {
                session: self.id.clone(),
                resume: run.cursor,
                cols: packed.cols,
                rows: packed.rows,
                format: FORMAT,
                vornd_build: self.cfg.build.clone(),
                blob_crc32: crc32fast::hash(&packed.blob),
                blob: packed.blob.clone(),
            }));
            run.floor = Some(run.cursor);
            run.since_cut = 0;
            self.checkpoints += 1;
            self.uncut = None;
        }
        let Phase::Running(run) = std::mem::replace(&mut self.phase, Phase::Lost) else {
            return false;
        };
        let (_, mut run) = run.with_term(packed);
        // The grid is rebuilt from the terminal on the next attach.
        run.hub.rebuilt();
        self.phase = Phase::Asleep(Box::new(run));
        out.push(Out::Asleep);
        true
    }

    /// Brings a terminal put away back, as it was. A blob that will not
    /// decode, which only running out of memory explains, loses the session.
    fn wake(&mut self, now: Instant, out: &mut Vec<Out>) {
        if !self.asleep() {
            return;
        }
        let Phase::Asleep(run) = std::mem::replace(&mut self.phase, Phase::Lost) else {
            return;
        };
        match run.term.wake() {
            Ok(term) => {
                let (_, mut run) = run.with_term(term);
                run.last_active = now;
                self.phase = Phase::Running(Box::new(run));
            }
            Err(_) => self.lose("terminal would not wake", out),
        }
    }

    /// Asks for a [`VtSnapshot`] of the terminal, answered with
    /// [`Out::Snapshot`] under `token`: now when the stream is at a point
    /// where one can be cut, otherwise at the next record boundary that is,
    /// or once it has waited [`crate::snapshot::HOLD`].
    pub fn snapshot(&mut self, token: u64, now: Instant, out: &mut Vec<Out>) {
        self.wake(now, out);
        let Phase::Running(run) = &mut self.phase else {
            out.push(Out::Snapshot(token, None));
            return;
        };
        run.last_active = now;
        self.snapshots.push((token, now));
        answer_snapshots(run, &mut self.snapshots, None, out);
    }

    /// The analyzer's last `lines` completed lines, answered with
    /// [`Out::Output`] under `token`.
    pub fn output(&mut self, token: u64, lines: u32, now: Instant, out: &mut Vec<Out>) {
        self.wake(now, out);
        let lines = match &mut self.phase {
            Phase::Running(r) => {
                r.last_active = now;
                Some(r.term.lines(lines))
            }
            _ => None,
        };
        out.push(Out::Output(token, lines));
    }

    /// Takes one message from sessiond.
    pub fn input(&mut self, input: Input, now: Instant, out: &mut Vec<Out>) {
        match &input {
            // An asleep session has no attachment for these to end.
            Input::Grid(GridIn::Detach { .. } | GridIn::Gone { .. }) if self.asleep() => return,
            Input::Grid(_) => self.wake(now, out),
            _ => {}
        }
        match input {
            Input::Checkpoint(cp) => {
                if let Phase::Attaching(step) = self.phase {
                    self.offered(step, &cp, now, out);
                }
            }
            Input::Refused(why) => match &self.phase {
                Phase::Attaching(step) => {
                    self.rejected.push(format!("{step:?}: {why:?}"));
                    self.next_step(now, out);
                }
                // Refusals answer the attach that chose the base.
                Phase::Running(r) if !r.started => {
                    self.rejected.push(format!("{}: {why:?}", r.base.as_str()));
                    match r.base {
                        Base::SessionStart => self.next_step(now, out),
                        // The best checkpoint's records are gone: the oldest
                        // record is all there is. `best` is spent, so this
                        // asks for it.
                        Base::Best => self.approximate(now, out),
                        _ => self.lose("no record to carry on from", out),
                    }
                }
                _ => {}
            },
            Input::Entries(entries) => {
                self.apply_all(&entries, now, out);
                if self.cfg.stream && !entries.is_empty() {
                    out.push(Out::Applied(entries));
                }
            }
            Input::Grid(m) => self.waiting.push(m),
        }
        self.report_live(out);
        self.serve_grid(now, out);
    }

    /// Sends [`Out::Live`] once the session has gone live on its base.
    fn report_live(&mut self, out: &mut Vec<Out>) {
        if !self.cfg.stream {
            return;
        }
        if let Phase::Running(run) = &mut self.phase {
            if run.live && !run.live_reported {
                run.live_reported = true;
                out.push(Out::Live(run.cursor));
            }
        }
    }

    /// Hands waiting grid requests to the hub once there is a terminal.
    fn serve_grid(&mut self, now: Instant, out: &mut Vec<Out>) {
        if self.waiting.is_empty() {
            return;
        }
        let Phase::Running(run) = &mut self.phase else {
            return;
        };
        run.last_active = now;
        let mut hub_out = Vec::new();
        let ctx = grid_ctx(&run.term, run.cursor, run.live, &self.id);
        for m in self.waiting.drain(..) {
            run.hub.handle(m, &ctx, now, &mut hub_out);
        }
        out.extend(hub_out.into_iter().map(Out::Grid));
    }

    /// When the grid's render clock next wants [`Session::frame`] called.
    pub fn due(&self) -> Option<Instant> {
        match &self.phase {
            Phase::Running(r) => r.hub.due(),
            _ => None,
        }
    }

    /// Cuts the grid frame that was waiting on the render clock, if it is
    /// due by `now`.
    pub fn frame(&mut self, now: Instant, out: &mut Vec<Out>) {
        let Phase::Running(run) = &mut self.phase else {
            return;
        };
        let mut hub_out = Vec::new();
        let ctx = grid_ctx(&run.term, run.cursor, run.live, &self.id);
        run.hub.tick(&ctx, now, &mut hub_out);
        out.extend(hub_out.into_iter().map(Out::Grid));
    }

    /// Applies records in order: what [`Input::Entries`] does.
    pub fn apply_all(&mut self, entries: &[Entry], now: Instant, out: &mut Vec<Out>) {
        if !entries.is_empty() {
            self.wake(now, out);
        }
        let Phase::Running(run) = &mut self.phase else {
            // An answer to an attach that was turned down.
            return;
        };
        let first = out.len();
        let mut applied = false;
        for e in entries {
            if run.cursor.includes(&e.hdr) {
                continue;
            }
            if !run.cursor.is_followed_by(&e.hdr) {
                if !run.started {
                    continue;
                }
                // Records are missing: carry on from the next one, as across
                // a gap.
                run.lost_output(&mut self.reason, "records missing");
                run.cursor = Cursor {
                    epoch: e.hdr.epoch,
                    next_rseq: e.hdr.rseq,
                    next_offset: e.hdr.start_offset,
                };
            }
            run.started = true;
            applied = true;
            let ctx = Ctx {
                id: &self.id,
                cfg: &self.cfg,
                sent: self.open.sent,
            };
            run.apply(&ctx, e, now, &mut self.fx, &mut self.reason, out);
            if let Record::Exit { code, signal } = e.rec {
                self.exited = Some((code, signal));
            }
            run.reach(self.open.head, self.open.pty, out);
            // After the record's effects, never before: a checkpoint covers
            // its record, and recovery from it does not emit them again.
            run.cut_due(&ctx, false, &mut self.checkpoints, &mut self.uncut, out);
            if !self.snapshots.is_empty() {
                answer_snapshots(run, &mut self.snapshots, None, out);
            }
        }
        if applied {
            run.last_active = now;
            out.push(Out::Ack(run.cursor));
            run.grid_records(&self.id, now, first, out);
        }
        debug_assert!(effects_precede_checkpoints(&out[first..]), "{out:?}");
    }

    /// Whether the session is done with: its program ended and every
    /// record is applied, or it was lost. Its host closes it.
    pub fn closed(&self) -> bool {
        match &self.phase {
            Phase::Lost => true,
            Phase::Running(r) => r.live && self.exited.is_some(),
            // Its exit would have woken it.
            Phase::Attaching(_) | Phase::Asleep(_) => false,
        }
    }

    /// Deletes the session's disk history, as when it is released.
    pub fn remove_history(&mut self) -> std::io::Result<()> {
        match &mut self.phase {
            Phase::Running(r) => r.history = None,
            Phase::Asleep(r) => r.history = None,
            _ => {}
        }
        match &self.cfg.history {
            Some(dir) => History::remove(&history_path(dir, &self.id)),
            None => Ok(()),
        }
    }

    /// Cuts a checkpoint if output has gone quiet with enough new since the
    /// last one.
    pub fn tick(&mut self, now: Instant, out: &mut Vec<Out>) {
        let Phase::Running(run) = &mut self.phase else {
            return;
        };
        if !self.snapshots.is_empty() {
            answer_snapshots(run, &mut self.snapshots, Some(now), out);
        }
        let c = &self.cfg.cadence;
        if run.since_cut >= c.quiet_bytes && now.duration_since(run.last_output) >= c.quiet {
            let ctx = Ctx {
                id: &self.id,
                cfg: &self.cfg,
                sent: self.open.sent,
            };
            run.cut_due(&ctx, true, &mut self.checkpoints, &mut self.uncut, out);
        }
    }

    /// Cuts a last checkpoint, as a clean shutdown does: whenever a record
    /// (output, a resize, the exit) has been applied past the newest one,
    /// so the next vornd replays nothing.
    pub fn shutdown(&mut self, out: &mut Vec<Out>) {
        if let Phase::Running(run) = &mut self.phase {
            let ctx = Ctx {
                id: &self.id,
                cfg: &self.cfg,
                sent: self.open.sent,
            };
            run.cut_due(&ctx, true, &mut self.checkpoints, &mut self.uncut, out);
        }
    }

    /// The session with its contents: the screen as text, the title, the
    /// cwd and the analyzer's lines. Formats the screen, so it costs.
    pub fn summary(&self) -> Summary {
        let mut s = Summary {
            brief: self.brief(),
            title: String::new(),
            cwd: String::new(),
            screen: String::new(),
            lines: Vec::new(),
            digest: None,
        };
        // An asleep terminal is read from a copy woken for it.
        let woken;
        let term = match &self.phase {
            Phase::Running(r) => &r.term,
            Phase::Asleep(r) => match r.term.wake() {
                Ok(t) => {
                    woken = t;
                    &woken
                }
                Err(_) => return s,
            },
            _ => return s,
        };
        s.title = term.em.title().to_owned();
        s.cwd = term.em.cwd().to_owned();
        s.screen = plain(&term.em);
        s.lines = term.lines(20);
        s.digest = Some(term.em.state_digest());
        s
    }

    /// Where the session is and how it was recovered.
    pub fn brief(&self) -> Brief {
        let mut s = Brief {
            session: self.id.clone(),
            state: State::Lost,
            base: None,
            fidelity: self.fidelity(),
            reason: self.reason,
            rejected: self.rejected.clone(),
            cursor: None,
            cols: self.open.size.0,
            rows: self.open.size.1,
            checkpoints: self.checkpoints,
            uncut: self.uncut,
            exited: self.exited,
            asleep: self.asleep(),
        };
        match &self.phase {
            Phase::Attaching(_) => s.state = State::Attaching,
            Phase::Lost => {}
            Phase::Running(r) => {
                s.state = match (r.live, self.exited) {
                    (false, _) => State::Replaying,
                    (true, None) => State::Live,
                    (true, Some(_)) => State::Ended,
                };
                s.base = Some(r.base);
                s.cursor = Some(r.cursor);
                s.cols = r.term.em.cols();
                s.rows = r.term.em.rows();
            }
            Phase::Asleep(r) => {
                s.state = State::Live;
                s.base = Some(r.base);
                s.cursor = Some(r.cursor);
                s.cols = r.term.cols;
                s.rows = r.term.rows;
            }
        }
        s
    }

    /// Asks for the next restore base, or settles for the best there is.
    fn next_step(&mut self, now: Instant, out: &mut Vec<Out>) {
        match self.steps.pop() {
            Some(Step::Start) => {
                let size = self.open.spawn_size.unwrap_or(self.open.size);
                out.push(Out::Attach(AttachFrom::SessionStart));
                match Term::fresh(size.0, size.1, self.cfg.scrollback) {
                    Ok(term) => {
                        let start = Cursor::start(self.open.epoch);
                        self.run(
                            term,
                            start,
                            Base::SessionStart,
                            self.open.spawn_size.is_some(),
                            now,
                            out,
                        );
                        // A session with no checkpoint yet gets one at its
                        // start, which keeps the spawn size for later
                        // recoveries that cannot know it.
                        if let Phase::Running(run) = &mut self.phase {
                            if self.open.newest_cp.is_none() && run.size_known {
                                let ctx = Ctx {
                                    id: &self.id,
                                    cfg: &self.cfg,
                                    sent: self.open.sent,
                                };
                                run.cut_due(
                                    &ctx,
                                    true,
                                    &mut self.checkpoints,
                                    &mut self.uncut,
                                    out,
                                );
                            }
                        }
                    }
                    Err(_) => self.lose("no terminal at the session's size", out),
                }
            }
            Some(step) => {
                self.phase = Phase::Attaching(step);
                out.push(Out::Attach(match step {
                    Step::Newest => AttachFrom::NewestCheckpoint,
                    _ => AttachFrom::FallbackCheckpoint,
                }));
            }
            None => self.approximate(now, out),
        }
    }

    /// Nothing qualified: carry on from the checkpoint that rebuilt, or a
    /// blank terminal at the oldest record, and say so.
    fn approximate(&mut self, now: Instant, out: &mut Vec<Out>) {
        self.reason = Some("no valid restore base");
        let (mut term, at, base) = match self.best.take() {
            Some((term, at)) => (*term, at, Base::Best),
            None => {
                let (cols, rows) = self.open.size;
                match Term::fresh(cols, rows, self.cfg.scrollback) {
                    Ok(t) => (t, self.open.oldest, Base::Oldest),
                    Err(_) => return self.lose("no terminal at the session's size", out),
                }
            }
        };
        term.fidelity = Fidelity::Approximate;
        out.push(Out::Attach(AttachFrom::Cursor(at)));
        self.run(term, at, base, true, now, out);
    }

    /// A checkpoint arrived for the base asked for at `step`.
    fn offered(&mut self, step: Step, cp: &Checkpoint, now: Instant, out: &mut Vec<Out>) {
        match check(cp) {
            Ok(term) => {
                let base = match step {
                    Step::Newest => Base::Newest,
                    _ => Base::Fallback,
                };
                if term.fidelity == Fidelity::Approximate {
                    self.reason = Some("cut from an approximate terminal");
                }
                self.best = None;
                self.run(term, cp.resume, base, true, now, out);
            }
            Err((why, rebuilt)) => {
                self.rejected.push(format!("{step:?}: {}", why.as_str()));
                if let (Some(term), None) = (rebuilt, &self.best) {
                    self.best = Some((term, cp.resume));
                }
                self.next_step(now, out);
            }
        }
    }

    fn run(
        &mut self,
        mut term: Term,
        at: Cursor,
        base: Base,
        size_known: bool,
        now: Instant,
        out: &mut Vec<Out>,
    ) {
        let history = self.cfg.history.as_ref().and_then(|dir| {
            std::fs::create_dir_all(dir).ok()?;
            let h = History::open(&history_path(dir, &self.id), at.epoch, at).ok()?;
            Some(h.with_cap(self.cfg.history_cap))
        });
        if self.cfg.history.is_some() && history.is_none() {
            self.reason.get_or_insert("history log unwritable");
        }
        // Grid clients stay attached across runs; the new terminal is a
        // new build for them.
        let mut hub = match std::mem::replace(&mut self.phase, Phase::Lost) {
            Phase::Running(old) => old.hub,
            Phase::Asleep(old) => old.hub,
            _ => Hub::new(self.cfg.grid),
        };
        hub.rebuilt();
        term.set_colors(self.cfg.colors);
        let mut run = Box::new(Run {
            term,
            base,
            cursor: at,
            started: false,
            live: false,
            size_known,
            floor: self.open.newest_cp,
            since_cut: 0,
            last_output: now,
            last_active: now,
            watched: false,
            history,
            nudge: false,
            hub,
            live_reported: false,
        });
        run.reach(self.open.head, self.open.pty, out);
        self.phase = Phase::Running(run);
    }

    fn lose(&mut self, why: &'static str, out: &mut Vec<Out>) {
        self.reason = Some(why);
        self.phase = Phase::Lost;
        out.push(Out::Lost);
    }
}

/// What a running session reads of its owner while applying a record.
struct Ctx<'a> {
    id: &'a str,
    cfg: &'a Config,
    sent: Cursor,
}

impl Run {
    /// After a batch of records: the effects it caused go to every grid
    /// client as events, then a frame is cut if one is due.
    fn grid_records(&mut self, id: &str, now: Instant, first: usize, out: &mut Vec<Out>) {
        let mut hub_out = Vec::new();
        // Nothing to do while no client is attached but remember that the
        // terminal moved on, which is cheap.
        let events = if self.hub.attached() {
            &out[first..]
        } else {
            &[]
        };
        for o in events {
            if let Out::Effect(fx, effect) = o {
                if let Some(kind) = event_kind(effect) {
                    let wire = WireEventId {
                        epoch: fx.epoch,
                        rseq: fx.rseq,
                        index: fx.index,
                    };
                    self.hub.event(wire, kind, &mut hub_out);
                }
            }
        }
        let ended = out[first..]
            .iter()
            .any(|o| matches!(o, Out::Effect(_, Effect::Exit { .. })));
        let ctx = grid_ctx(&self.term, self.cursor, self.live, id);
        self.hub.records(&ctx, now, &mut hub_out);
        if ended {
            // The session leaves the engine once its exit is applied.
            self.hub.finish(&ctx, now, &mut hub_out);
        }
        out.extend(hub_out.into_iter().map(Out::Grid));
    }

    fn apply(
        &mut self,
        ctx: &Ctx<'_>,
        e: &Entry,
        now: Instant,
        fx: &mut Vec<vorn_screen::Effect>,
        reason: &mut Option<&'static str>,
        out: &mut Vec<Out>,
    ) {
        let mut index = 0u32;
        let mut status = None;
        match &e.rec {
            Record::Data { bytes, .. } => {
                status = self.term.feed(bytes, ctx.cfg.analyze, fx);
                self.since_cut += bytes.len() as u64;
                self.last_output = now;
            }
            &Record::Resize { cols, rows, .. } => {
                if !self.size_known {
                    // The output before this was parsed at a size that may
                    // not have been the spawn size.
                    self.size_known = true;
                    self.term.fidelity = Fidelity::Approximate;
                    reason.get_or_insert("spawn size unknown");
                }
                if self
                    .term
                    .em
                    .resize(u32::from(cols), u32::from(rows), fx)
                    .is_err()
                {
                    self.term.fidelity = Fidelity::Approximate;
                    reason.get_or_insert("resize refused");
                }
            }
            Record::Gap { .. } => self.lost_output(reason, "output lost"),
            &Record::Exit { code, signal } => {
                self.emit(ctx, &e.hdr, &mut index, Effect::Exit { code, signal }, out);
            }
        }
        for f in fx.drain(..) {
            let effect = match f {
                vorn_screen::Effect::Reply(bytes) => {
                    // Live only: a query replayed must never reach the program.
                    index += 1;
                    if self.live {
                        out.push(Out::Write(bytes));
                    }
                    continue;
                }
                vorn_screen::Effect::Bell => Effect::Bell,
                vorn_screen::Effect::Clipboard { location, contents } => {
                    Effect::Clipboard { location, contents }
                }
                vorn_screen::Effect::Notify { title, body } => Effect::Notify { title, body },
                vorn_screen::Effect::Cwd(c) => Effect::Cwd(c),
            };
            self.emit(ctx, &e.hdr, &mut index, effect, out);
        }
        if let Some(s) = status {
            self.emit(ctx, &e.hdr, &mut index, Effect::Status(s), out);
        }
        if let Some(h) = &mut self.history {
            if h.append(e).is_err() {
                reason.get_or_insert("history log unwritable");
                self.history = None;
            }
        }
        self.cursor = e.after();
    }

    fn emit(
        &mut self,
        ctx: &Ctx<'_>,
        hdr: &RecordHeader,
        index: &mut u32,
        effect: Effect,
        out: &mut Vec<Out>,
    ) {
        let id = EffectId {
            session: ctx.id.to_owned(),
            epoch: hdr.epoch,
            rseq: hdr.rseq,
            index: *index,
        };
        *index += 1;
        let at_most_once = matches!(effect, Effect::Bell | Effect::Clipboard { .. });
        if at_most_once && ctx.sent.includes(hdr) {
            return;
        }
        out.push(Out::Effect(id, effect));
    }

    /// Output is gone: start again from a blank terminal at this size, and
    /// owe the program a nudge to redraw.
    fn lost_output(&mut self, reason: &mut Option<&'static str>, why: &'static str) {
        if self.term.reset().is_err() {
            self.term.fidelity = Fidelity::Approximate;
        }
        self.hub.rebuilt();
        reason.get_or_insert(why);
        self.nudge = true;
    }

    /// Leaves replay mode once the cursor reaches `head`, and pays a redraw
    /// nudge owed, which only a live session may send.
    fn reach(&mut self, head: Cursor, pty: bool, out: &mut Vec<Out>) {
        if !self.live {
            if self.cursor.epoch == head.epoch && self.cursor.next_rseq < head.next_rseq {
                return;
            }
            self.live = true;
            out.push(Out::Ready(self.term.fidelity));
        }
        if std::mem::take(&mut self.nudge) && pty {
            out.push(Out::Nudge {
                cols: self.term.em.cols(),
                rows: self.term.em.rows(),
            });
        }
    }

    /// Cuts a checkpoint when one is due (or `now` is set), past the floor,
    /// at a point where one can be cut.
    fn cut_due(
        &mut self,
        ctx: &Ctx<'_>,
        now: bool,
        count: &mut u64,
        uncut: &mut Option<&'static str>,
        out: &mut Vec<Out>,
    ) {
        if !now && self.since_cut < ctx.cfg.cadence.bytes {
            return;
        }
        if !self.past_floor() {
            return;
        }
        // Only a sequence too long to carry declines; it is retried at the next record.
        if let Some(why) = self.term.em.uncuttable() {
            *uncut = Some(why);
            return;
        }
        // A cut reads the render state and carries on from a rebuilt
        // terminal: grid line numbers are settled on the old one first.
        self.hub.before_swap(&self.term.em);
        let saved = self.term.save();
        self.hub.after_swap(&self.term.em);
        match saved {
            Ok(blob) => {
                out.push(Out::Checkpoint(Checkpoint {
                    session: ctx.id.to_owned(),
                    resume: self.cursor,
                    cols: self.term.em.cols(),
                    rows: self.term.em.rows(),
                    format: FORMAT,
                    vornd_build: ctx.cfg.build.clone(),
                    blob_crc32: crc32fast::hash(&blob),
                    blob,
                }));
                self.floor = Some(self.cursor);
                *count += 1;
                *uncut = None;
            }
            // A failed restore check costs a whole cut: wait for the next
            // cadence rather than retrying at every record.
            Err(why) => *uncut = Some(why),
        }
        self.since_cut = 0;
    }
}

/// What the grid hub reads of a running session.
fn grid_ctx<'a>(term: &'a Term, resume: Cursor, live: bool, id: &'a str) -> GridCtx<'a> {
    GridCtx {
        em: &term.em,
        session: id,
        resume,
        live,
        fidelity: match term.fidelity {
            Fidelity::Exact => msg::Fidelity::Exact,
            Fidelity::Approximate => msg::Fidelity::Approximate,
        },
    }
}

/// An effect as a grid client's event; states it reads from frames (the
/// cwd) are not events.
fn event_kind(effect: &Effect) -> Option<EventKind> {
    Some(match effect {
        Effect::Bell => EventKind::Bell,
        Effect::Clipboard { contents, .. } => EventKind::Clipboard {
            text: contents
                .iter()
                .find(|(mime, _)| mime.starts_with("text/plain") || mime.is_empty())
                .or(contents.first())
                .map(|(_, data)| data.clone())
                .unwrap_or_default(),
        },
        Effect::Notify { title, body } => EventKind::Notify {
            title: title.clone(),
            body: body.clone(),
        },
        Effect::Status(state) => EventKind::Status { state: *state },
        Effect::Exit { code, signal } => EventKind::Exit {
            code: *code,
            signal: *signal,
        },
        Effect::Cwd(_) => return None,
    })
}

/// Cuts the snapshots waiting when the terminal is at a point where one can
/// be cut, or, given `now`, those that have waited too long wherever it is.
fn answer_snapshots(
    run: &mut Run,
    waiting: &mut Vec<(u64, Instant)>,
    now: Option<Instant>,
    out: &mut Vec<Out>,
) {
    let cuttable = run.term.em.at_ground();
    let mut cut: Option<VtSnapshot> = None;
    waiting.retain(|&(token, asked)| {
        let overdue = now.is_some_and(|n| n.duration_since(asked) >= crate::snapshot::HOLD);
        if !cuttable && !overdue {
            return true;
        }
        let s =
            cut.get_or_insert_with(|| crate::snapshot::cut(&run.term.em, run.cursor, !cuttable));
        out.push(Out::Snapshot(token, Some(Box::new(s.clone()))));
        false
    });
}

fn history_path(dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    dir.join(format!("{id}.log"))
}

/// Whether no effect comes after a checkpoint that covers its record: the
/// order a host relies on to store a checkpoint only once what it covers
/// has been delivered.
fn effects_precede_checkpoints(out: &[Out]) -> bool {
    let mut covered: Option<Cursor> = None;
    out.iter().all(|o| match o {
        Out::Checkpoint(cp) => {
            covered = Some(cp.resume);
            true
        }
        Out::Effect(id, _) => !covered.is_some_and(|c| {
            c.includes(&RecordHeader {
                epoch: id.epoch,
                rseq: id.rseq,
                start_offset: 0,
            })
        }),
        _ => true,
    })
}

/// The restore check, in the order the cheap tests come.
fn check(cp: &Checkpoint) -> Result<Term, (Rejected, Option<Box<Term>>)> {
    if cp.format != FORMAT {
        return Err((Rejected::Format, None));
    }
    if !cp.crc_ok() {
        return Err((Rejected::Crc, None));
    }
    Term::load(&cp.blob)
}

/// The screen as plain text, rows joined by newlines.
fn plain(em: &Emulator) -> String {
    let opts = FormatterOptions::new().with_format(Format::Plain);
    Formatter::new(em.terminal(), opts)
        .and_then(|mut f| f.format_alloc(None))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

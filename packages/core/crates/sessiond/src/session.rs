//! One live session: the process, its I/O, and its record log (RC §3, §7).
//!
//! Reads happen on plain threads, one per output stream, and each read is
//! appended under the session lock, so the log's order is the order sessiond
//! saw things happen. A resize takes the same lock, performs the resize and
//! appends its record before any later output. Exit is appended only once the
//! output has ended *and* the child was reaped, so no data ever follows it.
//!
//! On macOS and Linux a session can be handed to a newer sessiond
//! ([`crate::handoff`]). A reader there waits for output with `poll` and
//! reads it under the session lock, so freezing the session ([`Mode`]) under
//! that lock leaves no byte read and not yet recorded, and the descriptors
//! can go to the newer sessiond with the log exactly where they stand.

#[cfg(unix)]
use std::fs::File;
use std::io::Read;
#[cfg(unix)]
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
#[cfg(unix)]
use std::sync::Weak;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use portable_pty::{Child, ChildKiller};
use tokio::sync::Notify;
#[cfg(unix)]
use vorn_term_proto::Entry;
use vorn_term_proto::{Record, Stream};

use crate::log::{AppendError, Budget, Overflow, SessionLog, SpoolPool};
#[cfg(unix)]
use crate::wire::{Checkpoint, FdRole, Manifest, SpoolState};
use crate::wire::{ExitInfo, Io, Kind, Sig, SpawnSpec, Stdin};

/// How much one read takes.
const READ_BYTES: usize = 64 << 10;
/// How long the exit waits for output to end after the child was reaped. A
/// background job that kept the terminal open would otherwise hold the Exit
/// record back forever; past this, what it prints is refused. The wait does
/// not run out while sessiond itself holds a reader back (a blocking session
/// whose log is full): that output is the program's, and is kept.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);
/// How often an adopted session looks for its program's exit, which the
/// sessiond that started it reaps and writes down.
#[cfg(unix)]
const WATCH_EVERY: Duration = Duration::from_millis(100);
/// How long an adopted session waits for the exit to be written down once
/// its program is gone, before it records the exit as unknown: the sessiond
/// that started it may have died, and then nobody writes it.
#[cfg(unix)]
const UNKNOWN_EXIT_AFTER: Duration = Duration::from_secs(1);

/// What a session's writer thread does next.
enum Input {
    Bytes {
        input_seq: u64,
        bytes: Vec<u8>,
    },
    CloseStdin,
    /// Another sessiond writes to the session from now on: leave without
    /// closing anything for the program.
    #[cfg(unix)]
    Stop,
}

/// Reported when the kernel took a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub input_seq: u64,
    pub written: u32,
}

/// Where a session stands in a handoff to a newer sessiond.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Reading, writing and recording as usual.
    Live,
    /// Being handed over: nothing is read, written or recorded until the
    /// handoff commits or is called off.
    Frozen,
    /// Another sessiond runs it; this one only reaps the program.
    HandedOff,
}

pub struct Session {
    pub id: String,
    pub kind: Kind,
    pub pid: u32,
    state: Mutex<State>,
    /// Signalled when a record is appended.
    pub changed: Notify,
    /// Signalled when a blocked reader may find room, and when the mode
    /// changes.
    room: Condvar,
    input: Mutex<Option<mpsc::Sender<Input>>>,
    /// Written once the session is handed off, so readers waiting for output
    /// wake and let their descriptors go.
    #[cfg(unix)]
    wake: (std::io::PipeReader, std::io::PipeWriter),
}

struct State {
    log: SessionLog,
    master: Option<Master>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Output streams still open.
    open_streams: u8,
    /// Readers holding a record a full blocking log refused, waiting for room.
    held: u8,
    /// Whether any reader was held since the reaper last looked: one that was
    /// only just let go may still have the rest of the pipe to read.
    was_held: bool,
    reaped: Option<ExitInfo>,
    #[cfg(unix)]
    mode: Mode,
    /// Input queued that the writer has not finished with, close included.
    #[cfg(unix)]
    inflight: u32,
    /// Whether this process started the program and so is the one that reaps
    /// it. An adopted session's program is reaped by the sessiond that
    /// started it.
    #[cfg(unix)]
    child: bool,
    /// Whether dropping the session ends its program: not while a handoff
    /// stages it, nor once another sessiond runs it.
    #[cfg(unix)]
    owns_child: bool,
    /// The pipe ends readers and the writer use, for a handoff to pass on.
    #[cfg(unix)]
    fds: Vec<(FdRole, RawFd)>,
    /// Where the exit goes once handed off, for the sessiond that has it.
    #[cfg(unix)]
    exit_file: Option<PathBuf>,
    /// Whether the program was reaped when the session froze, so the
    /// manifest said so already.
    #[cfg(unix)]
    reaped_when_frozen: bool,
}

impl State {
    fn new(
        log: SessionLog,
        master: Option<Master>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        open_streams: u8,
    ) -> State {
        State {
            log,
            master,
            killer,
            open_streams,
            held: 0,
            was_held: false,
            reaped: None,
            #[cfg(unix)]
            mode: Mode::Live,
            #[cfg(unix)]
            inflight: 0,
            #[cfg(unix)]
            child: true,
            #[cfg(unix)]
            owns_child: true,
            #[cfg(unix)]
            fds: Vec::new(),
            #[cfg(unix)]
            exit_file: None,
            #[cfg(unix)]
            reaped_when_frozen: false,
        }
    }

    /// Whether the session runs here as usual, neither frozen nor handed off.
    #[cfg(unix)]
    fn live(&self) -> bool {
        self.mode == Mode::Live
    }

    /// Always: on Windows sessions are never handed over.
    #[cfg(windows)]
    fn live(&self) -> bool {
        true
    }

    /// Count input queued for the writer, which a freeze waits out.
    #[cfg(unix)]
    fn queued(&mut self) {
        self.inflight += 1;
    }

    #[cfg(windows)]
    fn queued(&mut self) {}

    /// The handle a thread that is about to let `fd` go no longer passes on.
    #[cfg(unix)]
    fn forget(&mut self, fd: RawFd) {
        self.fds.retain(|&(_, f)| f != fd);
    }
}

/// A session's output end, as its reader thread holds it.
#[cfg(unix)]
type ReadEnd = File;
#[cfg(windows)]
type ReadEnd = Box<dyn Read + Send>;

/// What a frozen session hands to a newer sessiond: its manifest, then its
/// records in memory and its checkpoints, and the descriptors that go with
/// the manifest, in its `fds` order. The descriptors stay open here.
#[cfg(unix)]
pub(crate) struct Handover {
    pub manifest: Manifest,
    pub ring: Vec<Entry>,
    pub newest: Option<Checkpoint>,
    pub fallback: Option<Checkpoint>,
    pub fds: Vec<RawFd>,
}

/// A session a newer sessiond received and has not started: nothing reads,
/// writes or reaps it yet, and dropping it ends nothing but its own copies
/// of the descriptors.
#[cfg(unix)]
pub(crate) struct Staged {
    session: Arc<Session>,
    reader: Option<File>,
    stdout: Option<File>,
    stderr: Option<File>,
    stdin: Option<File>,
}

impl Session {
    /// Spawn a session. `on_written` hears about every write the kernel took.
    pub fn spawn(
        id: String,
        spec: &SpawnSpec,
        spool_dir: &Path,
        pool: SpoolPool,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) -> std::io::Result<Arc<Session>> {
        let spool = spool_dir.join(format!("{id}.log"));
        match spec.io {
            Io::Pty { cols, rows } => {
                Self::spawn_pty(id, spec, cols, rows, spool, pool, on_written)
            }
            Io::Piped { stdin } => Self::spawn_piped(id, spec, stdin, spool, pool, on_written),
        }
    }

    fn new(id: String, kind: Kind, pid: u32, state: State) -> std::io::Result<Arc<Session>> {
        Ok(Arc::new(Session {
            id,
            kind,
            pid,
            state: Mutex::new(state),
            changed: Notify::new(),
            room: Condvar::new(),
            input: Mutex::new(None),
            #[cfg(unix)]
            wake: std::io::pipe()?,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_pty(
        id: String,
        spec: &SpawnSpec,
        cols: u16,
        rows: u16,
        spool: std::path::PathBuf,
        pool: SpoolPool,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) -> std::io::Result<Arc<Session>> {
        let (master, reader, writer, child) = open_terminal(spec, cols, rows)?;
        let pid = child.process_id().unwrap_or(0);
        let budget = sized(Budget::PTY, spec.ring_bytes);
        let log = SessionLog::new(0, budget, Overflow::Drop, spool, pool, (cols, rows));
        let state = State::new(log, Some(master), child.clone_killer(), 1);
        let session = Self::new(id, Kind::Pty, pid, state)?;
        session.start_writer(writer, on_written);
        session.start_reader(reader, Stream::Pty);
        session.start_reaper(child);
        Ok(session)
    }

    fn spawn_piped(
        id: String,
        spec: &SpawnSpec,
        stdin: Stdin,
        spool: std::path::PathBuf,
        pool: SpoolPool,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) -> std::io::Result<Arc<Session>> {
        let (program, args) = spec
            .argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("empty argv"))?;
        let mut cmd = Command::new(program);
        if is_cmd(program) {
            verbatim(&mut cmd, args);
        } else {
            cmd.args(args);
        }
        cmd.current_dir(&spec.cwd);
        if !spec.env.is_empty() {
            cmd.env_clear().envs(spec.env.iter().cloned());
        }
        cmd.stdin(match stdin {
            Stdin::Pipe => Stdio::piped(),
            Stdin::Null => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        detach(&mut cmd);
        let mut child = cmd.spawn()?;
        let out = child.stdout.take().expect("piped");
        let err = child.stderr.take().expect("piped");
        let input = child.stdin.take();
        let pid = child.id();
        let budget = sized(Budget::PIPED, spec.ring_bytes);
        let log = SessionLog::new(0, budget, Overflow::Block, spool, pool, (0, 0));
        let child: Box<dyn Child + Send + Sync> = Box::new(child);
        let state = State::new(log, None, child.clone_killer(), 2);
        let session = Self::new(id, Kind::Piped, pid, state)?;
        #[cfg(unix)]
        session.track(&[
            (FdRole::Stdout, out.as_raw_fd()),
            (FdRole::Stderr, err.as_raw_fd()),
        ]);
        if let Some(input) = input {
            #[cfg(unix)]
            session.track(&[(FdRole::Stdin, input.as_raw_fd())]);
            session.start_writer(Box::new(input), on_written);
        }
        session.start_reader(read_end(out), Stream::Stdout);
        session.start_reader(read_end(err), Stream::Stderr);
        session.start_reaper(child);
        Ok(session)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start the thread that writes input. A piped session's stdin is
    /// tracked for a handoff to pass on while the thread runs.
    fn start_writer(
        self: &Arc<Self>,
        mut w: Box<dyn std::io::Write + Send>,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Input>();
        *self.input.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let id = self.id.clone();
        // Weak: the session holds the sender this thread waits on.
        #[cfg(unix)]
        let s: Weak<Session> = Arc::downgrade(self);
        thread::Builder::new()
            .name(format!("sessiond-w-{id}"))
            .spawn(move || {
                #[cfg(unix)]
                let mut closing = false;
                for msg in rx {
                    match msg {
                        Input::Bytes { input_seq, bytes } => {
                            // At-most-once: a failed write is reported as what
                            // got through, never retried.
                            let written = write_some(&mut w, &bytes);
                            #[cfg(unix)]
                            if let Some(s) = s.upgrade() {
                                s.lock().inflight -= 1;
                            }
                            on_written(&id, Written { input_seq, written });
                        }
                        Input::CloseStdin => {
                            #[cfg(unix)]
                            {
                                closing = true;
                            }
                            break;
                        }
                        #[cfg(unix)]
                        Input::Stop => break,
                    }
                }
                #[cfg(unix)]
                if let Some(s) = s.upgrade() {
                    let mut st = s.lock();
                    st.fds.retain(|&(role, _)| role != FdRole::Stdin);
                    if closing {
                        st.inflight -= 1;
                    }
                }
                // Dropping the writer closes stdin: the agent sees EOF,
                // unless a newer sessiond holds it too.
                drop(w);
            })
            .expect("spawn writer thread");
    }

    /// Start the thread that reads one output stream.
    fn start_reader(self: &Arc<Self>, r: ReadEnd, stream: Stream) {
        let s = Arc::clone(self);
        thread::Builder::new()
            .name(format!("sessiond-r-{}", self.id))
            .spawn(move || s.read_until_end(r, stream))
            .expect("spawn reader thread");
    }

    /// Read until the stream ends: wait for output, then read and record it
    /// under the session lock, unless the session is frozen or handed off.
    #[cfg(unix)]
    fn read_until_end(&self, mut r: File, stream: Stream) {
        let fd = r.as_raw_fd();
        let mut buf = vec![0u8; READ_BYTES];
        while let Ok(ready) = wait_readable(fd, self.wake.0.as_raw_fd()) {
            let mut st = self.lock();
            while st.mode == Mode::Frozen {
                st = self.room.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            if st.mode == Mode::HandedOff {
                st.forget(fd);
                return;
            }
            if !ready {
                continue;
            }
            // Ready, and this thread is its only reader: the read returns at
            // once, and the lock is held only for that.
            let n = match r.read(&mut buf) {
                Ok(n) if n > 0 => n,
                // Forgotten under this lock, so a freeze never sends an fd about to close.
                _ => {
                    st.forget(fd);
                    self.stream_ended(st);
                    return;
                }
            };
            if !self.record(st, stream, &buf[..n]) {
                self.lock().forget(fd);
                return;
            }
        }
        let mut st = self.lock();
        st.forget(fd);
        self.stream_ended(st);
    }

    #[cfg(windows)]
    fn read_until_end(&self, mut r: ReadEnd, stream: Stream) {
        let mut buf = vec![0u8; READ_BYTES];
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let st = self.lock();
            if !self.record(st, stream, &buf[..n]) {
                return;
            }
        }
        self.stream_ended(self.lock());
    }

    /// Append what a reader read. False once the log has ended.
    fn record(&self, mut st: MutexGuard<'_, State>, stream: Stream, bytes: &[u8]) -> bool {
        let mut rec = Record::Data {
            stream,
            bytes: bytes.to_vec(),
        };
        let mut holding = false;
        let exited = loop {
            match st.log.append(rec) {
                // Kept, or dropped with a Gap to say so.
                Ok(_) => break false,
                // Full and blocking: stop reading until a checkpoint makes
                // room. The program blocks on its next write; nothing is
                // lost. A handoff refuses to freeze while this waits.
                Err(AppendError::Full(back)) => {
                    rec = back;
                    if !holding {
                        holding = true;
                        st.held += 1;
                    }
                    st.was_held = true;
                    st = self
                        .room
                        .wait_timeout(st, Duration::from_millis(500))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                Err(AppendError::Exited) => break true,
            }
        };
        if holding {
            st.held -= 1;
        }
        drop(st);
        if !exited {
            self.changed.notify_waiters();
        }
        !exited
    }

    fn stream_ended(&self, mut st: MutexGuard<'_, State>) {
        st.open_streams -= 1;
        if st.live() {
            self.finish(&mut st);
        }
        drop(st);
        self.changed.notify_waiters();
    }

    fn start_reaper(self: &Arc<Self>, mut child: Box<dyn Child + Send + Sync>) {
        let s = Arc::clone(self);
        thread::Builder::new()
            .name(format!("sessiond-x-{}", self.id))
            .spawn(move || {
                let info = wait(&mut child);
                s.reaped(info);
            })
            .expect("spawn reaper thread");
    }

    /// The program ended: record its exit once output has ended too, or,
    /// for a session handed off, write it down for the sessiond that has it.
    fn reaped(&self, info: ExitInfo) {
        let mut st = self.lock();
        st.reaped = Some(info);
        #[cfg(unix)]
        if st.mode == Mode::HandedOff {
            if let Some(path) = &st.exit_file {
                let _ = write_exit(path, info);
            }
            return;
        }
        // ConPTY: output only ends once the pseudoconsole is closed,
        // which flushes conhost's last frame first. Closing it waits
        // for the reader to drain the pipe, and the reader needs this
        // lock to append, so it is closed after the lock is released.
        let master = if cfg!(windows) {
            st.master.take()
        } else {
            None
        };
        if st.live() {
            self.finish(&mut st);
        }
        drop(st);
        drop(master);
        self.changed.notify_waiters();
        // A background job holding the terminal open must not hold
        // the exit back for good. A reader sessiond holds back is not
        // that: the cutoff waits until no reader has been held for a
        // whole DRAIN_AFTER_EXIT, so a blocking session loses nothing.
        // Nor does it run while a handoff has the session frozen.
        loop {
            thread::sleep(DRAIN_AFTER_EXIT);
            let mut st = self.lock();
            if st.log.exited().is_some() {
                break;
            }
            #[cfg(unix)]
            match st.mode {
                Mode::HandedOff => break,
                Mode::Frozen => continue,
                Mode::Live => {}
            }
            if st.held > 0 || std::mem::take(&mut st.was_held) {
                continue;
            }
            st.open_streams = 0;
            self.finish(&mut st);
            drop(st);
            self.changed.notify_waiters();
            break;
        }
    }

    /// Append Exit once output has ended and the child was reaped.
    fn finish(&self, st: &mut State) {
        if st.open_streams > 0 || st.log.exited().is_some() {
            return;
        }
        if let Some(info) = st.reaped {
            let _ = st.log.append(Record::Exit {
                code: info.code,
                signal: info.signal,
            });
        }
    }

    /// Queue input. At-most-once: it is written by the session's writer
    /// thread, and a session without stdin, or one being handed over,
    /// drops it.
    pub fn write(&self, input_seq: u64, bytes: Vec<u8>) {
        let mut st = self.lock();
        if !st.live() {
            return;
        }
        if let Some(tx) = self
            .input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            if tx.send(Input::Bytes { input_seq, bytes }).is_ok() {
                st.queued();
            }
        }
    }

    /// Close a piped agent's stdin after what was queued before it. Not
    /// while a handoff has the session: that would close it for the newer
    /// sessiond too.
    pub fn close_stdin(&self) {
        let mut st = self.lock();
        if !st.live() {
            return;
        }
        if let Some(tx) = self.input.lock().unwrap_or_else(|e| e.into_inner()).take() {
            if tx.send(Input::CloseStdin).is_ok() {
                st.queued();
            }
        }
    }

    /// Resize the terminal and record it, under the lock the reader appends
    /// with, so output read after the resize lands after its record.
    pub fn resize(&self, req: u64, cols: u16, rows: u16, px_w: u16, px_h: u16) -> bool {
        let mut st = self.lock();
        if !st.live() {
            return false;
        }
        let Some(master) = st.master.as_ref() else {
            return false;
        };
        let ok = resize(master, cols, rows, px_w, px_h).is_ok();
        if ok {
            let _ = st.log.append(Record::Resize {
                cols,
                rows,
                px_w,
                px_h,
                req: Some(req),
            });
        }
        drop(st);
        self.changed.notify_waiters();
        ok
    }

    pub fn signal(&self, sig: Sig) {
        #[cfg(unix)]
        {
            let n = match sig {
                Sig::Int => libc::SIGINT,
                Sig::Term => libc::SIGTERM,
                Sig::Kill => libc::SIGKILL,
                Sig::Hup => libc::SIGHUP,
            };
            let st = self.lock();
            if self.pid != 0 && st.reaped.is_none() && st.mode != Mode::HandedOff {
                // SAFETY: kill(2) with a pid this process spawned, or adopted
                // from the sessiond that did, and not yet reaped.
                unsafe { libc::kill(self.pid as libc::pid_t, n) };
            }
        }
        #[cfg(windows)]
        {
            // Windows has no signals to send another process. A terminal's
            // interrupt is its ^C; everything else ends the process.
            if sig == Sig::Int && self.kind == Kind::Pty {
                self.write(u64::MAX, vec![0x03]);
            } else {
                let _ = self.lock().killer.kill();
            }
        }
    }

    /// Store a checkpoint vornd cut, unless a handoff has the session: the
    /// trim it may cause would change what was handed over.
    pub fn put_checkpoint(&self, cp: crate::wire::Checkpoint) {
        let mut st = self.lock();
        if st.live() {
            // A refused checkpoint changes nothing; vornd cuts another.
            let _ = st.log.put_checkpoint(cp);
        }
        drop(st);
        self.room_made();
    }

    /// Wake a reader blocked on a full log; called after a checkpoint trims.
    pub fn room_made(&self) {
        self.room.notify_all();
    }

    /// Run `f` on the log under the session lock.
    pub fn with_log<R>(&self, f: impl FnOnce(&mut SessionLog) -> R) -> R {
        f(&mut self.lock().log)
    }

    /// Whether this process started the program and has not reaped it yet.
    #[cfg(unix)]
    pub(crate) fn reaping(&self) -> bool {
        let st = self.lock();
        st.child && st.reaped.is_none()
    }

    /// Pipe ends a handoff passes on while their threads run.
    #[cfg(unix)]
    fn track(&self, fds: &[(FdRole, RawFd)]) {
        self.lock().fds.extend_from_slice(fds);
    }
}

/// A child's pipe as a reader thread holds it.
#[cfg(unix)]
fn read_end(pipe: impl Into<OwnedFd>) -> ReadEnd {
    File::from(pipe.into())
}

#[cfg(windows)]
fn read_end(pipe: impl Read + Send + 'static) -> ReadEnd {
    Box::new(pipe)
}

/// The handoff side of a session, macOS and Linux only.
#[cfg(unix)]
impl Session {
    /// Stop reading, writing and recording, and describe the session for a
    /// newer sessiond. Refused while a reader holds output a full log could
    /// not take, or input is still being written: neither could be handed
    /// over exactly.
    pub(crate) fn freeze(&self) -> Result<Handover, String> {
        let mut st = self.lock();
        if st.mode != Mode::Live {
            return Err(format!("{} is already being handed over", self.id));
        }
        if st.held > 0 {
            return Err(format!("{} is waiting for room in a full log", self.id));
        }
        if st.inflight > 0 {
            return Err(format!("{} is still writing input", self.id));
        }
        st.mode = Mode::Frozen;
        st.reaped_when_frozen = st.reaped.is_some();
        let input = self
            .input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let mut roles = Vec::new();
        let mut fds = Vec::new();
        if let Some(m) = &st.master {
            roles.push(FdRole::Master);
            fds.push(m.as_raw_fd());
        }
        for &(role, fd) in &st.fds {
            roles.push(role);
            fds.push(fd);
        }
        let mut manifest = Manifest {
            session: self.id.clone(),
            kind: self.kind,
            pid: self.pid,
            fds: roles,
            open_streams: st.open_streams,
            input,
            reaped: st.reaped,
            epoch: 0,
            ring_budget: 0,
            spool_budget: 0,
            blocking: false,
            head: vorn_term_proto::Cursor::start(0),
            sent: vorn_term_proto::Cursor::start(0),
            delivered: vorn_term_proto::Cursor::start(0),
            retain_from: vorn_term_proto::Cursor::start(0),
            cols: 0,
            rows: 0,
            exit: None,
            clock_ns: 0,
            ring_entries: 0,
            spool: SpoolState {
                bytes: 0,
                count: 0,
                first: None,
                first_offset: None,
                end: 0,
                marks: Vec::new(),
                torn: false,
            },
            newest: false,
            fallback: false,
        };
        st.log.describe(&mut manifest);
        let (newest, fallback) = st.log.checkpoints();
        let (newest, fallback) = (newest.cloned(), fallback.cloned());
        let ring = st.log.ring().iter().cloned().collect();
        Ok(Handover {
            manifest,
            ring,
            newest,
            fallback,
            fds,
        })
    }

    /// Call a handoff off: carry on as before it, recording an exit that
    /// came meanwhile.
    pub(crate) fn thaw(&self) {
        let mut st = self.lock();
        if st.mode != Mode::Frozen {
            return;
        }
        st.mode = Mode::Live;
        self.finish(&mut st);
        drop(st);
        self.room.notify_all();
        self.changed.notify_waiters();
    }

    /// The newer sessiond runs the session now. Let every descriptor go,
    /// so that it alone holds the terminal and the pipes, keep the spool
    /// file it carries on in, and from here on only reap the program,
    /// writing its exit to `exit_file`.
    pub(crate) fn hand_off(&self, exit_file: PathBuf) {
        let mut st = self.lock();
        st.mode = Mode::HandedOff;
        st.owns_child = false;
        st.master = None;
        let mut spent = SessionLog::new(
            0,
            Budget {
                ring_bytes: 0,
                spool_bytes: 0,
            },
            Overflow::Drop,
            PathBuf::new(),
            SpoolPool::new(0),
            (0, 0),
        );
        spent.keep_spool(true);
        let mut log = std::mem::replace(&mut st.log, spent);
        log.keep_spool(true);
        if let (Some(info), false) = (st.reaped, st.reaped_when_frozen) {
            let _ = write_exit(&exit_file, info);
        }
        st.exit_file = Some(exit_file);
        drop(st);
        drop(log);
        if let Some(tx) = self.input.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(Input::Stop);
        }
        let _ = std::io::Write::write(&mut &self.wake.1, &[1]);
        self.room.notify_all();
    }

    /// A session another sessiond described in `m` and passed `fds` for,
    /// with `log` built from what it sent. Checks the descriptors are what
    /// the manifest says; starts nothing.
    pub(crate) fn stage(
        m: &Manifest,
        fds: Vec<OwnedFd>,
        log: SessionLog,
    ) -> std::io::Result<Staged> {
        let bad = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidData, why);
        if fds.len() != m.fds.len() {
            return Err(bad(format!("{}: descriptors missing", m.session)));
        }
        let mut master = None;
        let (mut stdout, mut stdin, mut stderr) = (None, None, None);
        for (&role, fd) in m.fds.iter().zip(fds) {
            let slot = match role {
                FdRole::Master => &mut master,
                FdRole::Stdout => &mut stdout,
                FdRole::Stderr => &mut stderr,
                FdRole::Stdin => &mut stdin,
            };
            if slot.replace(fd).is_some() {
                return Err(bad(format!("{}: {role:?} passed twice", m.session)));
            }
        }
        let outs = u8::from(stdout.is_some()) + u8::from(stderr.is_some());
        let fits = match m.kind {
            // SAFETY: isatty on a descriptor this process owns.
            Kind::Pty => {
                outs == 0
                    && stdin.is_none()
                    && m.open_streams <= 1
                    && master
                        .as_ref()
                        .is_some_and(|f| unsafe { libc::isatty(f.as_raw_fd()) } == 1)
            }
            Kind::Piped => master.is_none() && outs == m.open_streams && stdin.is_some() == m.input,
        };
        if !fits || m.pid == 0 {
            return Err(bad(format!(
                "{}: the descriptors do not fit the manifest",
                m.session
            )));
        }
        let master = master.map(Master::from);
        let (reader, writer) = match &master {
            Some(mst) => (
                (m.open_streams == 1).then(|| mst.reader()).transpose()?,
                m.input.then(|| mst.writer()).transpose()?,
            ),
            None => (None, stdin.map(File::from)),
        };
        let mut state = State::new(log, master, Box::new(PidKiller(m.pid)), m.open_streams);
        state.reaped = m.reaped;
        state.child = false;
        state.owns_child = false;
        let session = Self::new(m.session.clone(), m.kind, m.pid, state)?;
        Ok(Staged {
            session,
            reader,
            stdout: stdout.map(File::from),
            stderr: stderr.map(File::from),
            stdin: writer,
        })
    }
}

#[cfg(unix)]
impl Staged {
    pub(crate) fn id(&self) -> &str {
        &self.session.id
    }

    /// Run the session here: read, write, and watch `exit_file` for the
    /// program's exit, which the sessiond that started it writes there.
    pub(crate) fn start(
        self,
        exit_file: PathBuf,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) -> Arc<Session> {
        let s = self.session;
        {
            let mut st = s.lock();
            st.owns_child = true;
            st.log.keep_spool(false);
        }
        if let Some(w) = self.stdin {
            if s.kind == Kind::Piped {
                s.track(&[(FdRole::Stdin, w.as_raw_fd())]);
            }
            s.start_writer(Box::new(w), on_written);
        }
        if let Some(r) = self.reader {
            s.start_reader(r, Stream::Pty);
        }
        if let Some(r) = self.stdout {
            s.track(&[(FdRole::Stdout, r.as_raw_fd())]);
            s.start_reader(r, Stream::Stdout);
        }
        if let Some(r) = self.stderr {
            s.track(&[(FdRole::Stderr, r.as_raw_fd())]);
            s.start_reader(r, Stream::Stderr);
        }
        let (reaped, exited) = {
            let st = s.lock();
            (st.reaped, st.log.exited().is_some())
        };
        let w = Arc::clone(&s);
        let watch = move || match reaped {
            Some(info) if !exited => w.reaped(info),
            Some(_) => {}
            None => {
                if let Some(info) = w.watch_exit(&exit_file) {
                    w.reaped(info);
                }
            }
        };
        thread::Builder::new()
            .name(format!("sessiond-x-{}", s.id))
            .spawn(watch)
            .expect("spawn reaper thread");
        s
    }
}

#[cfg(unix)]
impl Session {
    /// Wait for an adopted program's exit: the file the sessiond that
    /// started it writes once it reaps it, or, when the program is gone and
    /// no file comes, an exit nobody knows. None once the session was
    /// handed on again, for the next sessiond to watch.
    fn watch_exit(&self, exit_file: &Path) -> Option<ExitInfo> {
        let mut gone_since = None;
        loop {
            {
                let st = self.lock();
                match st.mode {
                    Mode::HandedOff => return None,
                    Mode::Live => {
                        if let Some(info) = read_exit(exit_file) {
                            let _ = std::fs::remove_file(exit_file);
                            return Some(info);
                        }
                    }
                    Mode::Frozen => {}
                }
            }
            if crate::launch::alive(self.pid) {
                gone_since = None;
            } else {
                let since = *gone_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() >= UNKNOWN_EXIT_AFTER && self.lock().mode == Mode::Live {
                    return Some(ExitInfo {
                        code: None,
                        signal: None,
                    });
                }
            }
            thread::sleep(WATCH_EVERY);
        }
    }
}

/// Ends an adopted program, which is not this process's child, by its pid.
#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct PidKiller(u32);

#[cfg(unix)]
impl ChildKiller for PidKiller {
    fn kill(&mut self) -> std::io::Result<()> {
        // SAFETY: kill(2) on a pid; the caller ends a program it holds.
        if unsafe { libc::kill(self.0 as libc::pid_t, libc::SIGKILL) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(*self)
    }
}

/// Wait until `fd` has output, or the wake pipe was written. True when `fd`
/// is ready: has data, or has ended.
#[cfg(unix)]
fn wait_readable(fd: RawFd, wake: RawFd) -> std::io::Result<bool> {
    let mut fds = [
        libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        // SAFETY: poll on two pollfds this frame owns.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n >= 0 {
            if fds[0].revents & libc::POLLNVAL != 0 {
                return Err(std::io::Error::from_raw_os_error(libc::EBADF));
            }
            return Ok(fds[0].revents != 0);
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Write a handed-off program's exit for the sessiond that has its session:
/// to a temporary name first, so it never reads half of it.
#[cfg(unix)]
fn write_exit(path: &Path, info: ExitInfo) -> std::io::Result<()> {
    let num = |v: Option<i32>| v.map(|v| v.to_string()).unwrap_or_default();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(
        &tmp,
        format!("code={}\nsignal={}\n", num(info.code), num(info.signal)),
    )?;
    std::fs::rename(tmp, path)
}

#[cfg(unix)]
fn read_exit(path: &Path) -> Option<ExitInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut info = ExitInfo {
        code: None,
        signal: None,
    };
    for line in text.lines() {
        match line.split_once('=')? {
            ("code", v) => info.code = v.parse().ok(),
            ("signal", v) => info.signal = v.parse().ok(),
            _ => {}
        }
    }
    Some(info)
}

impl Drop for State {
    fn drop(&mut self) {
        // A session dropped while its process runs: end it, unless a handoff
        // gives it to another sessiond.
        #[cfg(unix)]
        if !self.owns_child {
            return;
        }
        if self.reaped.is_none() {
            let _ = self.killer.kill();
        }
    }
}

/// A terminal's master end: kept for resizing, dropped to close it.
#[cfg(unix)]
type Master = crate::pty::Master;
#[cfg(windows)]
type Master = Box<dyn MasterPty + Send>;

/// What a terminal session runs on: its master, the master's reading and
/// writing ends, and the program.
type Terminal = (
    Master,
    ReadEnd,
    Box<dyn std::io::Write + Send>,
    Box<dyn Child + Send + Sync>,
);

/// Start the spec's program on a new terminal. On macOS and Linux sessiond
/// opens the terminal and starts the program itself ([`crate::pty`]).
#[cfg(unix)]
fn open_terminal(spec: &SpawnSpec, cols: u16, rows: u16) -> std::io::Result<Terminal> {
    let (program, args) = spec
        .argv
        .split_first()
        .ok_or_else(|| std::io::Error::other("empty argv"))?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    if !spec.env.is_empty() {
        // The spec carries the whole environment.
        cmd.env_clear().envs(spec.env.iter().cloned());
    }
    let env = |key: &str| -> Option<std::ffi::OsString> {
        if spec.env.is_empty() {
            std::env::var_os(key)
        } else {
            spec.env
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into())
        }
    };
    // A working directory that is gone starts the shell at home instead.
    if Path::new(&spec.cwd).is_dir() {
        cmd.current_dir(&spec.cwd);
    } else if let Some(home) = env("HOME") {
        cmd.current_dir(home);
    }
    // Shells start their own subshells by SHELL.
    if env("SHELL").is_none() {
        cmd.env("SHELL", login_shell());
    }
    let (master, child) = crate::pty::spawn(cmd, cols, rows)?;
    let reader = master.reader()?;
    let writer = master.writer()?;
    Ok((master, reader, Box::new(writer), Box::new(child)))
}

#[cfg(windows)]
fn open_terminal(spec: &SpawnSpec, cols: u16, rows: u16) -> std::io::Result<Terminal> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(std::io::Error::other)?;
    let mut cmd = CommandBuilder::from_argv(spec.argv.iter().map(Into::into).collect());
    cmd.cwd(&spec.cwd);
    if !spec.env.is_empty() {
        // The spec carries the whole environment.
        cmd.env_clear();
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(std::io::Error::other)?;
    // Only the child keeps the slave open, so the master reads EOF when
    // it and everything it started have closed it.
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(std::io::Error::other)?;
    let writer = pair.master.take_writer().map_err(std::io::Error::other)?;
    Ok((pair.master, reader, writer, child))
}

#[cfg(unix)]
fn resize(master: &Master, cols: u16, rows: u16, px_w: u16, px_h: u16) -> std::io::Result<()> {
    master.resize(cols, rows, px_w, px_h)
}

#[cfg(windows)]
fn resize(master: &Master, cols: u16, rows: u16, px_w: u16, px_h: u16) -> std::io::Result<()> {
    master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: px_w,
            pixel_height: px_h,
        })
        .map_err(std::io::Error::other)
}

/// The user's login shell from the password database, else `/bin/sh`.
#[cfg(unix)]
fn login_shell() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0 as libc::c_char; 4096];
    // SAFETY: an all-zero passwd is a valid out-parameter for getpwuid_r.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call, and `buf.len()` is its size.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pw,
            buf.as_mut_ptr(),
            buf.len(),
            &mut found,
        )
    };
    if rc == 0 && !found.is_null() && !pw.pw_shell.is_null() {
        // SAFETY: getpwuid_r found an entry, whose strings live in `buf`.
        let shell = unsafe { std::ffi::CStr::from_ptr(pw.pw_shell) };
        if !shell.is_empty() {
            return std::ffi::OsStr::from_bytes(shell.to_bytes()).to_owned();
        }
    }
    "/bin/sh".into()
}

fn sized(base: Budget, ring: Option<u32>) -> Budget {
    match ring {
        Some(r) if r > 0 => Budget {
            ring_bytes: u64::from(r),
            ..base
        },
        _ => base,
    }
}

fn write_some(w: &mut Box<dyn std::io::Write + Send>, bytes: &[u8]) -> u32 {
    let mut done = 0;
    while done < bytes.len() {
        match w.write(&bytes[done..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => done += n,
        }
    }
    let _ = w.flush();
    done as u32
}

/// Reap the child and say how it ended.
fn wait(child: &mut Box<dyn Child + Send + Sync>) -> ExitInfo {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let any: &mut dyn Child = &mut **child;
        if let Some(c) = any.downcast_mut::<std::process::Child>() {
            return match c.wait() {
                Ok(s) => ExitInfo {
                    code: s.code(),
                    signal: s.signal(),
                },
                Err(_) => ExitInfo {
                    code: None,
                    signal: None,
                },
            };
        }
    }
    match child.wait() {
        Ok(s) if s.signal().is_none() => ExitInfo {
            code: Some(s.exit_code() as i32),
            signal: None,
        },
        _ => ExitInfo {
            code: None,
            signal: None,
        },
    }
}

/// Keep a piped agent out of sessiond's process group and console, so
/// nothing aimed at sessiond's terminal reaches it.
fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches only the child.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Whether `program` is cmd.exe, which reads its own command line rather
/// than by the rules `Command` quotes for. Node's rule for the same choice.
fn is_cmd(program: &str) -> bool {
    let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("cmd.exe")
}

/// Hands cmd.exe its arguments as they are, joined by spaces, as Node does,
/// so `/s /c "<command line>"` reaches it unescaped.
#[cfg(windows)]
fn verbatim(cmd: &mut Command, args: &[String]) {
    use std::os::windows::process::CommandExt;
    for arg in args {
        cmd.raw_arg(arg);
    }
}

#[cfg(not(windows))]
fn verbatim(cmd: &mut Command, args: &[String]) {
    cmd.args(args);
}

#[cfg(test)]
mod cmd_tests {
    use super::is_cmd;

    #[test]
    fn knows_cmd_by_its_name_alone() {
        for p in [
            "cmd",
            "CMD.EXE",
            r"C:\Windows\System32\cmd.exe",
            "C:/Windows/cmd.exe",
        ] {
            assert!(is_cmd(p), "{p}");
        }
        for p in ["cmd2.exe", r"C:\cmd\node.exe", "pwsh.exe", "sh"] {
            assert!(!is_cmd(p), "{p}");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::wire::{AttachFrom, Checkpoint};
    use std::time::Instant;
    use vorn_term_proto::Cursor;

    fn checkpoint(at: Cursor) -> Checkpoint {
        let blob = b"screen".to_vec();
        Checkpoint {
            session: "s".into(),
            resume: at,
            cols: 0,
            rows: 0,
            format: 1,
            vornd_build: "test".into(),
            blob_crc32: crc32fast::hash(&blob),
            blob,
        }
    }

    /// A terminal session reads what the program printed, records a
    /// resize before what follows it, and ends with the program's exit.
    #[test]
    fn a_terminal_session_runs_to_its_exit() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec {
            argv: ["sh", "-c", "printf hi; read _; stty size; exit 4"]
                .map(String::from)
                .to_vec(),
            cwd: dir.path().join("gone").to_string_lossy().into_owned(),
            env: Vec::new(),
            io: Io::Pty { cols: 80, rows: 24 },
            ring_bytes: None,
        };
        let s =
            Session::spawn("t".into(), &spec, dir.path(), SpoolPool::new(0), |_, _| {}).unwrap();
        assert!(s.resize(1, 120, 40, 0, 0));
        s.write(1, b"\n".to_vec());
        let t = Instant::now();
        let (out, exit) = loop {
            assert!(t.elapsed() < Duration::from_secs(10), "exits");
            let done = s.with_log(|l| {
                let (_, recs) = l.attach(AttachFrom::SessionStart).unwrap();
                let mut out = Vec::new();
                let mut exit = None;
                for e in &recs {
                    match &e.rec {
                        Record::Data { bytes, .. } => out.extend(bytes),
                        Record::Exit { code, .. } => exit = Some(*code),
                        _ => {}
                    }
                }
                exit.map(|x| (String::from_utf8_lossy(&out).into_owned(), x))
            });
            if let Some(d) = done {
                break d;
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(exit, Some(4));
        assert!(out.contains("hi"), "{out:?}");
        assert!(out.contains("40 120"), "{out:?}");
    }

    /// A blocking session whose log is full when the program exits keeps
    /// the record its reader holds and the rest of the pipe, however long
    /// vornd takes to make room: the drain cutoff is for a background job
    /// holding the output open, not for sessiond's own backpressure.
    #[test]
    fn a_held_reader_outlasts_the_drain_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let spec = SpawnSpec {
            argv: [
                "sh",
                "-c",
                "printf A; sleep 0.2; printf B; sleep 0.2; printf C; exit 3",
            ]
            .map(String::from)
            .to_vec(),
            cwd: dir.path().to_string_lossy().into_owned(),
            env: Vec::new(),
            io: Io::Piped { stdin: Stdin::Null },
            // Room for one small record and no spool at all: the second
            // record is refused and its reader held.
            ring_bytes: Some(64),
        };
        let s =
            Session::spawn("s".into(), &spec, dir.path(), SpoolPool::new(0), |_, _| {}).unwrap();
        thread::sleep(DRAIN_AFTER_EXIT + Duration::from_secs(1));
        assert!(
            s.with_log(|l| l.exited()).is_none(),
            "no Exit while a reader is held"
        );

        // vornd reads what is there and stores two checkpoints at the head;
        // the older one lets the log drop what is behind it, and the held
        // reader goes on.
        let mut seen = s.with_log(|l| l.head());
        let mut out: Vec<u8> = Vec::new();
        let mut exit = None;
        let t = Instant::now();
        while exit.is_none() {
            assert!(t.elapsed() < Duration::from_secs(10), "exits once drained");
            s.with_log(|l| {
                let (_, new) = l.attach(AttachFrom::Cursor(seen)).unwrap();
                for e in &new {
                    match &e.rec {
                        Record::Data { bytes, .. } => out.extend(bytes),
                        Record::Exit { code, .. } => exit = Some(*code),
                        other => panic!("unexpected {other:?}"),
                    }
                }
                seen = l.head();
                l.put_checkpoint(checkpoint(seen)).unwrap();
                l.put_checkpoint(checkpoint(seen)).unwrap();
            });
            s.room_made();
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(exit, Some(Some(3)));
        assert_eq!(out, b"BC");
    }
}

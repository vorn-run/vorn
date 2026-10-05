//! One live session: the process, its I/O, and its record log (RC §3, §7).
//!
//! Reads happen on plain threads, one per output stream, and each read is
//! appended under the session lock, so the log's order is the order sessiond
//! saw things happen. A resize takes the same lock, performs the resize and
//! appends its record before any later output. Exit is appended only once the
//! output has ended *and* the child was reaped, so no data ever follows it.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::Notify;
use vorn_term_proto::{Record, Stream};

use crate::log::{AppendError, Budget, Overflow, SessionLog, SpoolPool};
use crate::wire::{ExitInfo, Io, Kind, Sig, SpawnSpec, Stdin};

/// How much one read takes.
const READ_BYTES: usize = 64 << 10;
/// How long the exit waits for output to end after the child was reaped. A
/// background job that kept the terminal open would otherwise hold the Exit
/// record back forever; past this, what it prints is refused. The wait does
/// not run out while sessiond itself holds a reader back (a blocking session
/// whose log is full): that output is the program's, and is kept.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);

/// What a session's writer thread does next.
enum Input {
    Bytes { input_seq: u64, bytes: Vec<u8> },
    CloseStdin,
}

/// Reported when the kernel took a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub input_seq: u64,
    pub written: u32,
}

pub struct Session {
    pub id: String,
    pub kind: Kind,
    pub pid: u32,
    state: Mutex<State>,
    /// Signalled when a record is appended.
    pub changed: Notify,
    /// Signalled when a blocked reader may find room.
    room: Condvar,
    input: Mutex<Option<mpsc::Sender<Input>>>,
}

struct State {
    log: SessionLog,
    master: Option<Box<dyn MasterPty + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Output streams still open.
    open_streams: u8,
    /// Readers holding a record a full blocking log refused, waiting for room.
    held: u8,
    /// Whether any reader was held since the reaper last looked: one that was
    /// only just let go may still have the rest of the pipe to read.
    was_held: bool,
    reaped: Option<ExitInfo>,
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
        let pid = child.process_id().unwrap_or(0);
        let budget = sized(Budget::PTY, spec.ring_bytes);
        let log = SessionLog::new(0, budget, Overflow::Drop, spool, pool, (cols, rows));
        let session = Arc::new(Session {
            id,
            kind: Kind::Pty,
            pid,
            state: Mutex::new(State {
                log,
                master: Some(pair.master),
                killer: child.clone_killer(),
                open_streams: 1,
                held: 0,
                was_held: false,
                reaped: None,
            }),
            changed: Notify::new(),
            room: Condvar::new(),
            input: Mutex::new(None),
        });
        session.start_writer(Box::new(writer), on_written);
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
        let session = Arc::new(Session {
            id,
            kind: Kind::Piped,
            pid,
            state: Mutex::new(State {
                log,
                master: None,
                killer: child.clone_killer(),
                open_streams: 2,
                held: 0,
                was_held: false,
                reaped: None,
            }),
            changed: Notify::new(),
            room: Condvar::new(),
            input: Mutex::new(None),
        });
        if let Some(input) = input {
            session.start_writer(Box::new(input), on_written);
        }
        session.start_reader(Box::new(out), Stream::Stdout);
        session.start_reader(Box::new(err), Stream::Stderr);
        session.start_reaper(child);
        Ok(session)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn start_writer(
        self: &Arc<Self>,
        mut w: Box<dyn std::io::Write + Send>,
        on_written: impl Fn(&str, Written) + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Input>();
        *self.input.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let id = self.id.clone();
        thread::Builder::new()
            .name(format!("sessiond-w-{id}"))
            .spawn(move || {
                for msg in rx {
                    match msg {
                        Input::Bytes { input_seq, bytes } => {
                            // At-most-once: a failed write is reported as what
                            // got through, never retried.
                            let written = write_some(&mut w, &bytes);
                            on_written(&id, Written { input_seq, written });
                        }
                        Input::CloseStdin => break,
                    }
                }
                // Dropping the writer closes stdin: the agent sees EOF.
            })
            .expect("spawn writer thread");
    }

    fn start_reader(self: &Arc<Self>, mut r: Box<dyn Read + Send>, stream: Stream) {
        let s = Arc::clone(self);
        thread::Builder::new()
            .name(format!("sessiond-r-{}", self.id))
            .spawn(move || {
                let mut buf = vec![0u8; READ_BYTES];
                loop {
                    let n = match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let mut rec = Record::Data {
                        stream,
                        bytes: buf[..n].to_vec(),
                    };
                    let mut st = s.lock();
                    let mut holding = false;
                    let exited = loop {
                        match st.log.append(rec) {
                            // Kept, or dropped with a Gap to say so.
                            Ok(_) => break false,
                            // Full and blocking: stop reading until a
                            // checkpoint makes room. The program blocks on
                            // its next write; nothing is lost.
                            Err(AppendError::Full(back)) => {
                                rec = back;
                                if !holding {
                                    holding = true;
                                    st.held += 1;
                                }
                                st.was_held = true;
                                st = s
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
                    if exited {
                        return;
                    }
                    drop(st);
                    s.changed.notify_waiters();
                }
                let mut st = s.lock();
                st.open_streams -= 1;
                s.finish(&mut st);
                drop(st);
                s.changed.notify_waiters();
            })
            .expect("spawn reader thread");
    }

    fn start_reaper(self: &Arc<Self>, mut child: Box<dyn Child + Send + Sync>) {
        let s = Arc::clone(self);
        thread::Builder::new()
            .name(format!("sessiond-x-{}", self.id))
            .spawn(move || {
                let info = wait(&mut child);
                let mut st = s.lock();
                st.reaped = Some(info);
                // ConPTY: output only ends once the pseudoconsole is closed,
                // which flushes conhost's last frame first. Closing it waits
                // for the reader to drain the pipe, and the reader needs this
                // lock to append, so it is closed after the lock is released.
                let master = if cfg!(windows) {
                    st.master.take()
                } else {
                    None
                };
                s.finish(&mut st);
                drop(st);
                drop(master);
                s.changed.notify_waiters();
                // A background job holding the terminal open must not hold
                // the exit back for good. A reader sessiond holds back is not
                // that: the cutoff waits until no reader has been held for a
                // whole DRAIN_AFTER_EXIT, so a blocking session loses nothing.
                loop {
                    thread::sleep(DRAIN_AFTER_EXIT);
                    let mut st = s.lock();
                    if st.log.exited().is_some() {
                        break;
                    }
                    if st.held > 0 || std::mem::take(&mut st.was_held) {
                        continue;
                    }
                    st.open_streams = 0;
                    s.finish(&mut st);
                    drop(st);
                    s.changed.notify_waiters();
                    break;
                }
            })
            .expect("spawn reaper thread");
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
    /// thread, and a session without stdin drops it.
    pub fn write(&self, input_seq: u64, bytes: Vec<u8>) {
        if let Some(tx) = self
            .input
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(Input::Bytes { input_seq, bytes });
        }
    }

    /// Close a piped agent's stdin after what was queued before it.
    pub fn close_stdin(&self) {
        if let Some(tx) = self.input.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(Input::CloseStdin);
        }
    }

    /// Resize the terminal and record it, under the lock the reader appends
    /// with, so output read after the resize lands after its record.
    pub fn resize(&self, req: u64, cols: u16, rows: u16, px_w: u16, px_h: u16) -> bool {
        let mut st = self.lock();
        let Some(master) = st.master.as_ref() else {
            return false;
        };
        let ok = master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: px_w,
                pixel_height: px_h,
            })
            .is_ok();
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
            if self.pid != 0 && self.lock().reaped.is_none() {
                // SAFETY: kill(2) with a pid this process spawned and has not reaped.
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

    /// Wake a reader blocked on a full log; called after a checkpoint trims.
    pub fn room_made(&self) {
        self.room.notify_all();
    }

    /// Run `f` on the log under the session lock.
    pub fn with_log<R>(&self, f: impl FnOnce(&mut SessionLog) -> R) -> R {
        f(&mut self.lock().log)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // A session dropped while its process runs: end it.
        if self.reaped.is_none() {
            let _ = self.killer.kill();
        }
    }
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

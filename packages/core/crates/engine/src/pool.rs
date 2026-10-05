//! A fixed set of worker threads, each running the session actors placed on
//! it.
//!
//! A session stays on the thread it was opened on: its terminal is not
//! `Send` (Ghostty's callbacks share state with it), and one thread per
//! session would cost a stack and a wakeup each for sessions that are idle
//! nearly all the time. Messages for one session go through one channel in
//! order, so its records are applied in the order sessiond sent them.
//!
//! What sessions want done goes to one sink, called on the worker thread. A
//! session whose program has ended, or that is lost, is closed by its
//! worker, which says so with [`Out::Closed`].
//!
//! Each worker keeps a [`Brief`] of its sessions in a map the pool shares,
//! so the debug report reads it without waiting behind a worker's queue.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::session::{Brief, Config, Input, Open, Out, Session, State, Summary};
use crate::term::Fidelity;

/// How often a worker looks for sessions gone quiet.
const TICK: Duration = Duration::from_millis(500);

/// Where sessions' outputs go: the session's id and what it wants.
pub type Sink = Arc<dyn Fn(&str, Out) + Send + Sync>;

enum Job {
    Open {
        id: String,
        open: Open,
    },
    Input {
        id: String,
        input: Input,
    },
    Close {
        id: String,
    },
    /// A VT snapshot for a bytes client, answered with `Out::Snapshot`.
    Snapshot {
        id: String,
        token: u64,
    },
    /// The analyzer's lines, answered with `Out::Output`.
    Output {
        id: String,
        token: u64,
        lines: u32,
    },
    Inspect {
        reply: SyncSender<Vec<Summary>>,
    },
    /// A last checkpoint for each session, as before a clean stop.
    Flush {
        reply: SyncSender<()>,
    },
    /// Stops the worker, after a last checkpoint for each session when set.
    Stop {
        checkpoint: bool,
    },
}

/// What the pool and its workers share.
#[derive(Default)]
struct Shared {
    /// Which worker each session is on.
    placed: Mutex<HashMap<String, usize>>,
    briefs: Mutex<HashMap<String, Brief>>,
    /// Set when the pool is dropped: workers stop before their next job
    /// rather than working through their queues.
    stop: AtomicBool,
}

impl Shared {
    fn placed(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.placed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn briefs(&self) -> MutexGuard<'_, HashMap<String, Brief>> {
        self.briefs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn forget(&self, id: &str) {
        self.placed().remove(id);
        self.briefs().remove(id);
    }
}

pub struct Pool {
    workers: Vec<(Sender<Job>, Option<JoinHandle<()>>)>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("workers", &self.workers.len())
            .field("sessions", &self.shared.placed().len())
            .finish()
    }
}

impl Pool {
    /// Starts `threads` workers (at least one) for sessions sharing `cfg`.
    pub fn new(threads: usize, cfg: Config, sink: Sink) -> std::io::Result<Pool> {
        let cfg = Arc::new(cfg);
        let shared = Arc::new(Shared::default());
        let mut workers = Vec::new();
        for n in 0..threads.max(1) {
            let (tx, rx) = mpsc::channel();
            let (cfg, sink, shared) = (Arc::clone(&cfg), Arc::clone(&sink), Arc::clone(&shared));
            // Built on its thread: the sessions it will hold are not Send.
            let handle = std::thread::Builder::new()
                .name(format!("vorn-engine-{n}"))
                .spawn(move || {
                    let w = Worker {
                        n,
                        cfg,
                        sink,
                        shared,
                        sessions: HashMap::new(),
                        out: Vec::new(),
                    };
                    w.run(rx)
                })?;
            workers.push((tx, Some(handle)));
        }
        Ok(Pool { workers, shared })
    }

    fn send(&self, id: &str, job: Job) {
        let Some(&w) = self.shared.placed().get(id) else {
            return;
        };
        // A worker only goes away when the pool does.
        let _ = self.workers[w].0.send(job);
    }

    /// Takes on a session, on the worker with the fewest. Opening one that
    /// is already open starts it over.
    pub fn open(&self, id: &str, open: Open) {
        let w = {
            let mut placed = self.shared.placed();
            if let Some(&w) = placed.get(id) {
                w
            } else {
                let mut load = vec![0usize; self.workers.len()];
                for &w in placed.values() {
                    load[w] += 1;
                }
                let w = (0..load.len()).min_by_key(|&w| load[w]).unwrap_or(0);
                placed.insert(id.to_owned(), w);
                w
            }
        };
        let _ = self.workers[w].0.send(Job::Open {
            id: id.to_owned(),
            open,
        });
    }

    pub fn input(&self, id: &str, input: Input) {
        self.send(
            id,
            Job::Input {
                id: id.to_owned(),
                input,
            },
        );
    }

    /// Asks session `id` for a VT snapshot, answered through the sink with
    /// `Out::Snapshot(token, ..)` in order with the session's other outputs.
    /// False when no such session is open, and nothing will answer.
    pub fn snapshot(&self, id: &str, token: u64) -> bool {
        self.ask(
            id,
            Job::Snapshot {
                id: id.to_owned(),
                token,
            },
        )
    }

    /// Asks session `id` for the analyzer's last `lines` lines, answered
    /// with `Out::Output(token, ..)`. False when no such session is open.
    pub fn output(&self, id: &str, token: u64, lines: u32) -> bool {
        self.ask(
            id,
            Job::Output {
                id: id.to_owned(),
                token,
                lines,
            },
        )
    }

    fn ask(&self, id: &str, job: Job) -> bool {
        let Some(&w) = self.shared.placed().get(id) else {
            return false;
        };
        self.workers[w].0.send(job).is_ok()
    }

    /// Drops a session without a last checkpoint.
    pub fn close(&self, id: &str) {
        self.send(id, Job::Close { id: id.to_owned() });
        self.shared.forget(id);
    }

    /// Every session as it stands, contents included, after everything
    /// sent to it before. Blocks until each worker answers.
    pub fn sessions(&self) -> Vec<Summary> {
        let mut answers = Vec::new();
        for (tx, _) in &self.workers {
            let (reply, rx) = mpsc::sync_channel(1);
            if tx.send(Job::Inspect { reply }).is_ok() {
                answers.push(rx);
            }
        }
        let mut all: Vec<Summary> = answers
            .into_iter()
            .flat_map(|rx| rx.recv().unwrap_or_default())
            .collect();
        all.sort_by(|a, b| a.brief.session.cmp(&b.brief.session));
        all
    }

    /// Every session as its worker last left it, without waiting for any.
    pub fn briefs(&self) -> Vec<Brief> {
        let mut all: Vec<Brief> = self.shared.briefs().values().cloned().collect();
        all.sort_by(|a, b| a.session.cmp(&b.session));
        all
    }

    /// Cuts a last checkpoint for every session, as a clean stop does, and
    /// returns once each has been handed to the sink. The sessions carry on.
    pub fn checkpoint_all(&self) {
        let mut answers = Vec::new();
        for (tx, _) in &self.workers {
            let (reply, rx) = mpsc::sync_channel(1);
            if tx.send(Job::Flush { reply }).is_ok() {
                answers.push(rx);
            }
        }
        for rx in answers {
            let _ = rx.recv();
        }
    }

    /// Stops every worker once it has done what was sent to it, each
    /// session cutting a last checkpoint first when `checkpoint` is set,
    /// and waits for them.
    pub fn shutdown(mut self, checkpoint: bool) {
        self.stop(checkpoint);
    }

    fn stop(&mut self, checkpoint: bool) {
        for (tx, _) in &self.workers {
            let _ = tx.send(Job::Stop { checkpoint });
        }
        for (_, handle) in &mut self.workers {
            if let Some(h) = handle.take() {
                let _ = h.join();
            }
        }
    }
}

impl Drop for Pool {
    /// Like a crash as far as sessiond can tell: no last checkpoint, and
    /// jobs still queued are dropped. Waits for each worker to finish the
    /// job in hand, so an async caller drops a pool off its runtime's
    /// threads.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.stop(false);
    }
}

/// One worker thread and the sessions on it.
struct Worker {
    n: usize,
    cfg: Arc<Config>,
    sink: Sink,
    shared: Arc<Shared>,
    sessions: HashMap<String, Session>,
    /// Reused for every call's outputs.
    out: Vec<Out>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Job>) {
        let mut last_tick = Instant::now();
        loop {
            let job = match rx.recv_timeout(TICK) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if self.shared.stop.load(Ordering::SeqCst) {
                return;
            }
            let now = Instant::now();
            match job {
                Some(Job::Open { id, open }) => {
                    let cfg = Arc::clone(&self.cfg);
                    let opened = self.guarded(&id, |out| Session::open(&id, cfg, open, now, out));
                    match opened {
                        Some(s) => {
                            self.sessions.insert(id.clone(), s);
                            self.settle(&id);
                        }
                        None => self.lost(&id),
                    }
                }
                Some(Job::Input { id, input }) => {
                    if let Some(mut s) = self.sessions.remove(&id) {
                        if self.guarded(&id, |out| s.input(input, now, out)).is_some() {
                            self.sessions.insert(id.clone(), s);
                            self.settle(&id);
                        } else {
                            self.lost(&id);
                        }
                    }
                }
                Some(Job::Close { id }) => {
                    self.sessions.remove(&id);
                }
                Some(Job::Snapshot { id, token }) => match self.sessions.remove(&id) {
                    Some(mut s) => {
                        if self
                            .guarded(&id, |out| s.snapshot(token, now, out))
                            .is_some()
                        {
                            self.sessions.insert(id.clone(), s);
                            self.settle(&id);
                        } else {
                            self.lost(&id);
                        }
                    }
                    // Closed since it was asked: the answer is still owed.
                    None => (self.sink)(&id, Out::Snapshot(token, None)),
                },
                Some(Job::Output { id, token, lines }) => match self.sessions.remove(&id) {
                    Some(mut s) => {
                        if self
                            .guarded(&id, |out| s.output(token, lines, out))
                            .is_some()
                        {
                            self.sessions.insert(id.clone(), s);
                        } else {
                            self.lost(&id);
                        }
                    }
                    None => (self.sink)(&id, Out::Output(token, None)),
                },
                Some(Job::Inspect { reply }) => {
                    let _ = reply.send(self.inspect());
                }
                Some(Job::Flush { reply }) => {
                    self.flush();
                    let _ = reply.send(());
                }
                Some(Job::Stop { checkpoint }) => {
                    if checkpoint {
                        self.flush();
                    }
                    return;
                }
                None => {}
            }
            if now.duration_since(last_tick) >= TICK {
                last_tick = now;
                let ids: Vec<String> = self.sessions.keys().cloned().collect();
                for id in ids {
                    if let Some(mut s) = self.sessions.remove(&id) {
                        if self.guarded(&id, |out| s.tick(now, out)).is_some() {
                            self.sessions.insert(id.clone(), s);
                            self.settle(&id);
                        } else {
                            self.lost(&id);
                        }
                    }
                }
            }
        }
    }

    /// Every session with its contents. A session that panics while being
    /// read is lost, not the worker or the answer.
    fn inspect(&mut self) -> Vec<Summary> {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        let mut all = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(s) = self.sessions.remove(&id) else {
                continue;
            };
            match self.guarded(&id, |_| s.summary()) {
                Some(summary) => {
                    all.push(summary);
                    self.sessions.insert(id, s);
                }
                None => self.lost(&id),
            }
        }
        all
    }

    fn flush(&mut self) {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        for id in ids {
            if let Some(mut s) = self.sessions.remove(&id) {
                if self.guarded(&id, |out| s.shutdown(out)).is_some() {
                    self.sessions.insert(id.clone(), s);
                    self.settle(&id);
                } else {
                    self.lost(&id);
                }
            }
        }
    }

    /// After a call into session `id`: closes it when it is done with, and
    /// otherwise keeps its brief current.
    fn settle(&mut self, id: &str) {
        let Some(s) = self.sessions.get(id) else {
            return;
        };
        if !s.closed() {
            let brief = s.brief();
            self.shared.briefs().insert(id.to_owned(), brief);
            return;
        }
        let Some(mut s) = self.sessions.remove(id) else {
            return;
        };
        let summary = self.guarded(id, |_| {
            let summary = s.summary();
            // A program that ended takes its history with it, as the
            // server's own logs go when a PTY exits.
            if summary.brief.state == State::Ended {
                let _ = s.remove_history();
            }
            summary
        });
        self.close(id, summary);
    }

    /// Session `id` panicked: it is gone, with what can be said about it.
    fn lost(&mut self, id: &str) {
        self.sessions.remove(id);
        self.close(id, None);
    }

    fn close(&mut self, id: &str, summary: Option<Summary>) {
        let summary = summary.unwrap_or_else(|| Summary {
            brief: Brief {
                session: id.to_owned(),
                state: State::Lost,
                base: None,
                fidelity: Fidelity::Approximate,
                reason: Some("the session engine failed on it"),
                rejected: Vec::new(),
                cursor: None,
                cols: 0,
                rows: 0,
                checkpoints: 0,
                uncut: None,
                exited: None,
            },
            title: String::new(),
            cwd: String::new(),
            screen: String::new(),
            lines: Vec::new(),
        });
        {
            let mut placed = self.shared.placed();
            // Unless it has been opened again elsewhere since.
            if placed.get(id) == Some(&self.n) {
                placed.remove(id);
            }
        }
        self.shared.briefs().remove(id);
        (self.sink)(id, Out::Closed(Box::new(summary)));
    }

    /// Runs `f` for session `id` and hands what it wants done to the sink.
    /// A panic in it (a bug, or Ghostty's) loses that session, not the
    /// worker and the other sessions on it: `None`, after an [`Out::Lost`].
    fn guarded<T>(&mut self, id: &str, f: impl FnOnce(&mut Vec<Out>) -> T) -> Option<T> {
        let r = catch_unwind(AssertUnwindSafe(|| f(&mut self.out)));
        for o in self.out.drain(..) {
            (self.sink)(id, o);
        }
        match r {
            Ok(v) => Some(v),
            Err(_) => {
                (self.sink)(id, Out::Lost);
                None
            }
        }
    }
}

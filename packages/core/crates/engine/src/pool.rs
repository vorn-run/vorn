//! A fixed set of worker threads, each running the session actors placed on
//! it.
//!
//! A session stays on the thread it was opened on: its terminal is not
//! `Send` (Ghostty's callbacks share state with it), and one thread per
//! session would cost a stack and a wakeup each for sessions that are idle
//! nearly all the time. Messages for one session go through one channel in
//! order, so its records are applied in the order sessiond sent them.
//!
//! What sessions want done goes to one sink, called on the worker thread.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::session::{Config, Input, Open, Out, Session, Summary};

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

pub struct Pool {
    workers: Vec<(Sender<Job>, Option<JoinHandle<()>>)>,
    placed: Mutex<HashMap<String, usize>>,
}

impl Pool {
    /// Starts `threads` workers (at least one) for sessions sharing `cfg`.
    pub fn new(threads: usize, cfg: Config, sink: Sink) -> std::io::Result<Pool> {
        let cfg = Arc::new(cfg);
        let mut workers = Vec::new();
        for n in 0..threads.max(1) {
            let (tx, rx) = mpsc::channel();
            let (cfg, sink) = (Arc::clone(&cfg), Arc::clone(&sink));
            let handle = std::thread::Builder::new()
                .name(format!("vorn-engine-{n}"))
                .spawn(move || work(rx, cfg, sink))?;
            workers.push((tx, Some(handle)));
        }
        Ok(Pool {
            workers,
            placed: Mutex::new(HashMap::new()),
        })
    }

    fn placed(&self) -> std::sync::MutexGuard<'_, HashMap<String, usize>> {
        self.placed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, id: &str, job: Job) {
        let Some(&w) = self.placed().get(id) else {
            return;
        };
        // A worker only goes away when the pool does.
        let _ = self.workers[w].0.send(job);
    }

    /// Takes on a session, on the worker with the fewest. Opening one that
    /// is already open starts it over.
    pub fn open(&self, id: &str, open: Open) {
        let w = {
            let mut placed = self.placed();
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

    /// Drops a session without a last checkpoint.
    pub fn close(&self, id: &str) {
        self.send(id, Job::Close { id: id.to_owned() });
        self.placed().remove(id);
    }

    /// Every session as it stands, after everything sent to it before.
    /// Blocks until each worker answers.
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

    /// Stops every worker, each session cutting a last checkpoint first
    /// when `checkpoint` is set, and waits for them.
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
    /// Like a crash as far as sessiond can tell: no last checkpoint.
    fn drop(&mut self) {
        self.stop(false);
    }
}

fn work(rx: Receiver<Job>, cfg: Arc<Config>, sink: Sink) {
    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut out = Vec::new();
    let mut last_tick = Instant::now();
    loop {
        let job = match rx.recv_timeout(TICK) {
            Ok(job) => Some(job),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let now = Instant::now();
        match job {
            Some(Job::Open { id, open }) => {
                let cfg = Arc::clone(&cfg);
                let opened = guarded(&id, &sink, &mut out, |out| {
                    Session::open(&id, cfg, open, now, out)
                });
                if let Some(s) = opened {
                    sessions.insert(id, s);
                }
            }
            Some(Job::Input { id, input }) => {
                if let Some(s) = sessions.get_mut(&id) {
                    if guarded(&id, &sink, &mut out, |out| s.input(input, now, out)).is_none() {
                        sessions.remove(&id);
                    }
                }
            }
            Some(Job::Close { id }) => {
                sessions.remove(&id);
            }
            Some(Job::Inspect { reply }) => {
                let _ = reply.send(sessions.values().map(Session::summary).collect());
            }
            Some(Job::Flush { reply }) => {
                flush(&mut sessions, &sink, &mut out);
                let _ = reply.send(());
            }
            Some(Job::Stop { checkpoint }) => {
                if checkpoint {
                    flush(&mut sessions, &sink, &mut out);
                }
                return;
            }
            None => {}
        }
        if now.duration_since(last_tick) >= TICK {
            last_tick = now;
            let mut failed = Vec::new();
            for (id, s) in &mut sessions {
                if guarded(id, &sink, &mut out, |out| s.tick(now, out)).is_none() {
                    failed.push(id.clone());
                }
            }
            for id in failed {
                sessions.remove(&id);
            }
        }
    }
}

fn flush(sessions: &mut HashMap<String, Session>, sink: &Sink, out: &mut Vec<Out>) {
    for (id, s) in sessions.iter_mut() {
        guarded(id, sink, out, |out| s.shutdown(out));
    }
}

/// Runs `f` for session `id` and hands what it wants done to the sink. A
/// panic in it (a bug, or Ghostty's) loses that session, not the worker and
/// the other sessions on it: `None`, after an [`Out::Lost`].
fn guarded<T>(
    id: &str,
    sink: &Sink,
    out: &mut Vec<Out>,
    f: impl FnOnce(&mut Vec<Out>) -> T,
) -> Option<T> {
    let r = catch_unwind(AssertUnwindSafe(|| f(out)));
    for o in out.drain(..) {
        sink(id, o);
    }
    match r {
        Ok(v) => Some(v),
        Err(_) => {
            sink(id, Out::Lost);
            None
        }
    }
}

//! Which agent conversations are being started, before the sessions
//! starting them can say so.
//!
//! A session that names a conversation (a resume, or a create given one)
//! claims it before its workspace is prepared, under the id the session will
//! have, so a second start of the same conversation sees it in flight and
//! does not run a second agent on one transcript. A claim lapses after
//! [`SPAWN_WINDOW`], longer than an agent takes to report the conversation it
//! took, so a spawn that dies never holds one for ever; while its session's
//! workspace is still being prepared it does not lapse at all, since git has
//! no upper bound the window could cover. The server's own starts (a resume,
//! a create for a remote host) claim here too, through `vornd:claim`, so
//! there is one set of claims for both.
//!
//! Creates naming the same conversation while one is preparing share its
//! answer ([`OnePerKey`]): the second wants the session the first is
//! starting, not a workspace of its own to discard.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a claim waits for its session to report the conversation.
pub const SPAWN_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Spawning {
    session: String,
    claimed_at: Instant,
}

#[derive(Debug, Default)]
struct State {
    /// By conversation.
    spawning: HashMap<String, Spawning>,
    /// Sessions whose workspace is being prepared, with how many
    /// preparations each has under way.
    preparing: HashMap<String, u32>,
}

/// One answer several callers wait for.
#[derive(Debug)]
struct Shared<T> {
    answer: Mutex<Option<T>>,
    ready: Condvar,
}

impl<T> Default for Shared<T> {
    fn default() -> Self {
        Shared {
            answer: Mutex::new(None),
            ready: Condvar::new(),
        }
    }
}

/// The conversations claimed, by the sessions starting on them.
#[derive(Debug)]
pub struct Claims {
    state: Mutex<State>,
    window: Duration,
}

impl Default for Claims {
    fn default() -> Self {
        Claims::with_window(SPAWN_WINDOW)
    }
}

impl Claims {
    /// Claims that lapse after `window`; tests use a short one.
    pub fn with_window(window: Duration) -> Claims {
        Claims {
            state: Mutex::default(),
            window,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claims `transcript` for `session`. Answers the session already
    /// starting on it, or `None` when the claim is taken.
    pub fn claim(&self, transcript: &str, session: &str, now: Instant) -> Option<String> {
        let mut st = self.lock();
        evict_lapsed(&mut st, now, self.window);
        if let Some(held) = st.spawning.get(transcript) {
            return Some(held.session.clone());
        }
        st.spawning.insert(
            transcript.to_owned(),
            Spawning {
                session: session.to_owned(),
                claimed_at: now,
            },
        );
        None
    }

    /// Every conversation claimed now (`spawningTranscripts`).
    pub fn held(&self, now: Instant) -> Vec<String> {
        let mut st = self.lock();
        evict_lapsed(&mut st, now, self.window);
        st.spawning.keys().cloned().collect()
    }

    /// The session holding `transcript`, if one does.
    pub fn holder(&self, transcript: &str, now: Instant) -> Option<String> {
        let mut st = self.lock();
        evict_lapsed(&mut st, now, self.window);
        st.spawning.get(transcript).map(|s| s.session.clone())
    }

    /// Lets go of `transcript`, if `session` holds it.
    pub fn release(&self, transcript: &str, session: &str) {
        let mut st = self.lock();
        if st
            .spawning
            .get(transcript)
            .is_some_and(|s| s.session == session)
        {
            st.spawning.remove(transcript);
        }
    }

    /// Lets go of everything `session` holds.
    pub fn release_for(&self, session: &str) {
        self.lock().spawning.retain(|_, s| s.session != session);
    }

    /// Keeps `session`'s claims from lapsing until [`Claims::prepared`] is
    /// called for it as many times.
    pub fn preparing(&self, session: &str) {
        *self.lock().preparing.entry(session.to_owned()).or_default() += 1;
    }

    /// One preparation of `session` is over. When it was the last, its
    /// claims' window starts again: from now on they wait on the agent's
    /// report, as any claim does.
    pub fn prepared(&self, session: &str, now: Instant) {
        let mut st = self.lock();
        let Some(left) = st.preparing.get_mut(session) else {
            return;
        };
        *left -= 1;
        if *left > 0 {
            return;
        }
        st.preparing.remove(session);
        for held in st.spawning.values_mut() {
            if held.session == session {
                held.claimed_at = now;
            }
        }
    }
}

/// Runs under way, by key, and the callers waiting on them.
#[derive(Debug)]
pub struct OnePerKey<T> {
    running: Mutex<HashMap<String, Arc<Shared<T>>>>,
}

impl<T> Default for OnePerKey<T> {
    fn default() -> Self {
        OnePerKey {
            running: Mutex::default(),
        }
    }
}

impl<T> OnePerKey<T> {
    /// Runs `work` for `key`, unless a run for it is under way: then waits
    /// for that one and answers what it answered. The first caller runs on
    /// its own thread; the others block theirs while they wait.
    pub fn run(&self, key: &str, work: impl FnOnce() -> T) -> T
    where
        T: Clone,
    {
        let (shared, first) = {
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            match running.get(key) {
                Some(s) => (Arc::clone(s), false),
                None => {
                    let s = Arc::new(Shared::default());
                    running.insert(key.to_owned(), Arc::clone(&s));
                    (s, true)
                }
            }
        };
        if !first {
            let mut answer = shared.answer.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(a) = answer.as_ref() {
                    return a.clone();
                }
                if !self.under_way(key, &shared) {
                    // The run ended without an answer: it panicked.
                    drop(answer);
                    return self.run(key, work);
                }
                answer = shared.ready.wait(answer).unwrap_or_else(|e| e.into_inner());
            }
        }
        // Taken out of the map whatever `work` does, so a panic in it never
        // leaves the key waiting for ever: a waiter then finds no answer and
        // the guard's drop wakes it to run its own.
        let done = Done {
            runs: self,
            key,
            shared: &shared,
        };
        let answer = work();
        *shared.answer.lock().unwrap_or_else(|e| e.into_inner()) = Some(answer.clone());
        drop(done);
        answer
    }
}

impl<T> OnePerKey<T> {
    /// Whether `shared` is still the run under way for `key`.
    fn under_way(&self, key: &str, shared: &Arc<Shared<T>>) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .is_some_and(|s| Arc::ptr_eq(s, shared))
    }
}

/// Ends a run under way for a key, and wakes those waiting for it.
struct Done<'a, T> {
    runs: &'a OnePerKey<T>,
    key: &'a str,
    shared: &'a Arc<Shared<T>>,
}

impl<T> Drop for Done<'_, T> {
    fn drop(&mut self) {
        let mut running = self.runs.running.lock().unwrap_or_else(|e| e.into_inner());
        if running
            .get(self.key)
            .is_some_and(|s| Arc::ptr_eq(s, self.shared))
        {
            running.remove(self.key);
        }
        drop(running);
        // Under the answer's lock, which a waiter holds from its check until
        // it waits, so the wake cannot fall between the two.
        let _answer = self.shared.answer.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.ready.notify_all();
    }
}

fn evict_lapsed(st: &mut State, now: Instant, window: Duration) {
    let State {
        spawning,
        preparing,
    } = st;
    spawning.retain(|_, held| {
        preparing.contains_key(&held.session) || now.duration_since(held.claimed_at) < window
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_is_held_until_released_or_lapsed_but_not_while_preparing() {
        let claims = Claims::with_window(Duration::from_secs(60));
        let t0 = Instant::now();
        assert_eq!(claims.claim("conv", "a", t0), None);
        assert_eq!(claims.claim("conv", "b", t0), Some("a".to_owned()));
        // Only its holder lets it go.
        claims.release("conv", "b");
        assert_eq!(claims.holder("conv", t0).as_deref(), Some("a"));
        claims.release("conv", "a");
        assert_eq!(claims.holder("conv", t0), None);

        // Preparing for longer than the window: still held.
        claims.preparing("a");
        assert_eq!(claims.claim("conv", "a", t0), None);
        let late = t0 + Duration::from_secs(90);
        assert_eq!(claims.holder("conv", late).as_deref(), Some("a"));
        // Prepared: the window starts again from then.
        claims.prepared("a", late);
        assert_eq!(
            claims
                .holder("conv", late + Duration::from_secs(59))
                .as_deref(),
            Some("a")
        );
        assert_eq!(claims.holder("conv", late + Duration::from_secs(60)), None);

        claims.claim("x", "s", t0);
        claims.claim("y", "s", t0);
        claims.release_for("s");
        assert_eq!(claims.holder("x", t0), None);
        assert_eq!(claims.holder("y", t0), None);
    }

    #[test]
    fn two_preparations_of_one_session_both_end_before_the_window_starts() {
        let claims = Claims::with_window(Duration::from_secs(1));
        let t0 = Instant::now();
        claims.preparing("a");
        claims.preparing("a");
        claims.claim("conv", "a", t0);
        claims.prepared("a", t0);
        let later = t0 + Duration::from_secs(5);
        assert_eq!(claims.holder("conv", later).as_deref(), Some("a"));
        claims.prepared("a", later);
        assert_eq!(claims.holder("conv", later + Duration::from_secs(1)), None);
    }

    #[test]
    fn creates_for_one_key_under_way_together_share_one_answer() {
        use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
        let runs: Arc<OnePerKey<u32>> = Arc::default();
        let count = Arc::new(AtomicU32::new(0));
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let first = {
            let (runs, count) = (Arc::clone(&runs), Arc::clone(&count));
            std::thread::spawn(move || {
                runs.run("conv", || {
                    // Held until the second caller is waiting.
                    rx.recv().unwrap();
                    count.fetch_add(1, SeqCst) + 7
                })
            })
        };
        // Until the first has registered, a second would run its own.
        while runs.running.lock().unwrap().is_empty() {
            std::thread::yield_now();
        }
        let second = {
            let (runs, count) = (Arc::clone(&runs), Arc::clone(&count));
            std::thread::spawn(move || runs.run("conv", || count.fetch_add(1, SeqCst) + 100))
        };
        // The map, the first caller and the second, waiting.
        while runs
            .running
            .lock()
            .unwrap()
            .get("conv")
            .is_some_and(|s| Arc::strong_count(s) < 3)
        {
            std::thread::yield_now();
        }
        tx.send(()).unwrap();
        assert_eq!(first.join().unwrap(), 7);
        assert_eq!(second.join().unwrap(), 7);
        assert_eq!(count.load(SeqCst), 1);
        // Done: the next one runs again.
        assert_eq!(runs.run("conv", || 9), 9);
    }
}

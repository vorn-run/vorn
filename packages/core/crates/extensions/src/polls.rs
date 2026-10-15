//! Poll scheduling as a bridge service: whoever schedules polls asks here,
//! and the extension that answers polls for that kind of trigger runs one.
//!
//! An extension offers the polls of a trigger type (`connectorPoll`) while it
//! is on, and stops offering them when it is switched off. A poll is held to
//! one in flight per key, so a slow one is not stacked by the next tick, and
//! to a deadline, so a hung one frees its key.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What became of a poll asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Polled<T> {
    /// It ran and answered.
    Done(T),
    /// One for the same key was still running; nothing was started.
    InFlight,
    /// No extension answers polls of that trigger type.
    NoProvider,
    /// It ran past the deadline and was dropped.
    TimedOut,
}

/// Who answers each trigger type's polls, and the polls running.
#[derive(Debug, Default)]
pub struct Polls {
    providers: Mutex<HashMap<String, String>>,
    running: Mutex<HashSet<String>>,
}

/// Frees a poll's key however it ends.
struct Running<'a> {
    polls: &'a Polls,
    key: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(&self.polls.running).remove(&self.key);
    }
}

impl Polls {
    /// Extension `owner` answers the polls of `trigger_type` from now on.
    pub fn offer(&self, trigger_type: &str, owner: &str) {
        lock(&self.providers).insert(trigger_type.to_owned(), owner.to_owned());
    }

    /// `owner` no longer answers any polls.
    pub fn withdraw(&self, owner: &str) {
        lock(&self.providers).retain(|_, o| o != owner);
    }

    /// Who answers the polls of `trigger_type`.
    pub fn provider(&self, trigger_type: &str) -> Option<String> {
        lock(&self.providers).get(trigger_type).cloned()
    }

    /// Runs `poll` for `key` with the provider of `trigger_type`, unless
    /// one for `key` is running, within `limit`.
    pub async fn run<T, F, Fut>(
        &self,
        trigger_type: &str,
        key: &str,
        limit: Duration,
        poll: F,
    ) -> Polled<T>
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = T>,
    {
        let Some(owner) = self.provider(trigger_type) else {
            return Polled::NoProvider;
        };
        if !lock(&self.running).insert(key.to_owned()) {
            return Polled::InFlight;
        }
        let _running = Running {
            polls: self,
            key: key.to_owned(),
        };
        match tokio::time::timeout(limit, poll(owner)).await {
            Ok(answer) => Polled::Done(answer),
            Err(_) => Polled::TimedOut,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test(start_paused = true)]
    async fn runs_one_poll_per_key_with_whoever_offers_them() {
        let polls = Arc::new(Polls::default());
        let limit = Duration::from_secs(5);
        assert_eq!(
            polls
                .run("connectorPoll", "wf", limit, |_| async { 1 })
                .await,
            Polled::NoProvider
        );
        polls.offer("connectorPoll", "vorn.connectors");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let slow = {
            let polls = Arc::clone(&polls);
            tokio::spawn(async move {
                polls
                    .run("connectorPoll", "wf", limit, |owner| async move {
                        let _ = rx.await;
                        owner
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            polls
                .run("connectorPoll", "wf", limit, |_| async {
                    "again".to_owned()
                })
                .await,
            Polled::InFlight
        );
        assert_eq!(
            polls
                .run("connectorPoll", "other", limit, |o| async move { o })
                .await,
            Polled::Done("vorn.connectors".to_owned())
        );
        tx.send(()).unwrap();
        assert_eq!(
            slow.await.unwrap(),
            Polled::Done("vorn.connectors".to_owned())
        );
        let hung = polls
            .run("connectorPoll", "wf", limit, |_| {
                std::future::pending::<()>()
            })
            .await;
        assert_eq!(hung, Polled::TimedOut);
        assert_eq!(
            polls
                .run("connectorPoll", "wf", limit, |_| async { 2 })
                .await,
            Polled::Done(2)
        );
        polls.withdraw("vorn.connectors");
        assert_eq!(polls.provider("connectorPoll"), None);
    }
}

//! What the app needs to hear about the sessions vornd holds, kept for an
//! app that was away when it happened.
//!
//! The app does not read a session's output. vornd parses it and tells the
//! app what the output meant, as effects with stable ids (Session Recovery
//! Contract §7): the agent status, the working directory and the exit are
//! states, each carrying the record it reflects; a notification is an event
//! the app shows once, deduplicated by its id. Effects reach a connected app
//! as they happen ([`crate::control`]). This journal keeps the latest state
//! of each session and the last notifications, so an app that connects
//! later, after its own restart or vornd's, is told the same things again
//! and drops what it has already seen. The bell and clipboard writes are not
//! here: they are at-most-once and go to clients directly.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use vorn_engine::{Effect, EffectId};

/// Notifications kept for an app that reconnects.
pub const NOTICES_KEPT: usize = 256;

/// Sessions that ended kept, so an app that was away learns how.
pub const ENDED_KEPT: usize = 64;

/// How often a session that keeps printing is reported active: the app's
/// idle timer counts from the last report.
pub const ACTIVITY_EVERY: Duration = Duration::from_secs(1);

/// What kind of process a session runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pty,
    Piped,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Pty => "pty",
            Kind::Piped => "piped",
        }
    }
}

/// A state and the effect that set it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped<T> {
    pub value: T,
    pub id: EffectId,
}

/// One session as the app is told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub session: String,
    pub kind: Kind,
    pub pid: u32,
    pub status: Option<Stamped<u32>>,
    pub cwd: Option<Stamped<String>>,
    pub exit: Option<Stamped<(Option<i32>, Option<i32>)>>,
}

/// The latest state of every session and the last notifications.
#[derive(Debug, Default)]
pub struct Journal {
    held: HashMap<String, Held>,
    ended: VecDeque<Held>,
    notices: VecDeque<(EffectId, Effect)>,
    active: HashMap<String, Instant>,
}

/// Whether `a` comes after `b` in a session's log.
fn later(a: &EffectId, b: &EffectId) -> bool {
    (a.epoch, a.rseq, a.index) > (b.epoch, b.rseq, b.index)
}

impl Journal {
    /// A session vornd now holds, from sessiond's Welcome or a spawn.
    pub fn opened(&mut self, session: &str, kind: Kind, pid: u32) {
        self.ended.retain(|h| h.session != session);
        self.held
            .entry(session.to_owned())
            .and_modify(|h| {
                h.kind = kind;
                h.pid = pid;
            })
            .or_insert_with(|| Held {
                session: session.to_owned(),
                kind,
                pid,
                status: None,
                cwd: None,
                exit: None,
            });
    }

    /// An effect a session's actor reported. States move only forward, so
    /// one delivered again by a replay changes nothing.
    pub fn record(&mut self, id: &EffectId, effect: &Effect) {
        match effect {
            Effect::Notify { .. } => {
                if self.notices.iter().any(|(seen, _)| seen == id) {
                    return;
                }
                if self.notices.len() == NOTICES_KEPT {
                    self.notices.pop_front();
                }
                self.notices.push_back((id.clone(), effect.clone()));
            }
            Effect::Status(s) => {
                if let Some(h) = self.held.get_mut(&id.session) {
                    set(&mut h.status, *s, id);
                }
            }
            Effect::Cwd(c) => {
                if let Some(h) = self.held.get_mut(&id.session) {
                    set(&mut h.cwd, c.clone(), id);
                }
            }
            Effect::Exit { code, signal } => {
                if let Some(h) = self.held.get_mut(&id.session) {
                    set(&mut h.exit, (*code, *signal), id);
                }
            }
            Effect::Bell | Effect::Clipboard { .. } => {}
        }
    }

    /// The session left the engine. One that ended is kept a while as it
    /// ended; one that was lost stays held, for the next connection to
    /// sessiond to take on again.
    pub fn closed(&mut self, session: &str, ended: bool) {
        self.active.remove(session);
        if !ended {
            return;
        }
        if let Some(h) = self.held.remove(session) {
            if self.ended.len() == ENDED_KEPT {
                self.ended.pop_front();
            }
            self.ended.push_back(h);
        }
    }

    /// Forgets the sessions sessiond no longer holds: they ended while
    /// nothing was there to see how.
    pub fn keep_only(&mut self, held: impl Fn(&str) -> bool) {
        let gone: Vec<String> = self.held.keys().filter(|s| !held(s)).cloned().collect();
        for s in gone {
            self.closed(&s, true);
        }
    }

    /// Whether to report `session` active now: once per
    /// [`ACTIVITY_EVERY`] while it prints.
    pub fn activity(&mut self, session: &str, now: Instant) -> bool {
        match self.active.get(session) {
            Some(&last) if now.duration_since(last) < ACTIVITY_EVERY => false,
            _ => {
                self.active.insert(session.to_owned(), now);
                true
            }
        }
    }

    /// Every session held now.
    pub fn held(&self) -> Vec<Held> {
        let mut v: Vec<Held> = self.held.values().cloned().collect();
        v.sort_by(|a, b| a.session.cmp(&b.session));
        v
    }

    /// The sessions that ended most recently, oldest first.
    pub fn ended(&self) -> Vec<Held> {
        self.ended.iter().cloned().collect()
    }

    /// The notifications kept, oldest first.
    pub fn notices(&self) -> Vec<(EffectId, Effect)> {
        self.notices.iter().cloned().collect()
    }
}

fn set<T>(slot: &mut Option<Stamped<T>>, value: T, id: &EffectId) {
    if slot.as_ref().is_none_or(|s| later(id, &s.id)) {
        *slot = Some(Stamped {
            value,
            id: id.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(session: &str, rseq: u64, index: u32) -> EffectId {
        EffectId {
            session: session.into(),
            epoch: 1,
            rseq,
            index,
        }
    }

    #[test]
    fn states_only_move_forward() {
        let mut j = Journal::default();
        j.opened("a", Kind::Pty, 7);
        j.record(&fx("a", 9, 0), &Effect::Status(2));
        // A replay delivers an older status again: it changes nothing.
        j.record(&fx("a", 4, 0), &Effect::Status(1));
        let h = &j.held()[0];
        assert_eq!(h.status.as_ref().map(|s| s.value), Some(2));
        assert_eq!(h.pid, 7);
    }

    #[test]
    fn a_notification_is_kept_once_and_only_so_many() {
        let mut j = Journal::default();
        let n = Effect::Notify {
            title: "t".into(),
            body: "b".into(),
        };
        j.record(&fx("a", 1, 0), &n);
        j.record(&fx("a", 1, 0), &n);
        assert_eq!(j.notices().len(), 1);
        for i in 0..(NOTICES_KEPT as u64 + 10) {
            j.record(&fx("a", 2 + i, 0), &n);
        }
        assert_eq!(j.notices().len(), NOTICES_KEPT);
    }

    #[test]
    fn an_ended_session_is_remembered_with_its_exit() {
        let mut j = Journal::default();
        j.opened("a", Kind::Piped, 3);
        j.record(
            &fx("a", 5, 0),
            &Effect::Exit {
                code: Some(2),
                signal: None,
            },
        );
        j.closed("a", true);
        assert!(j.held().is_empty());
        let ended = j.ended();
        assert_eq!(
            ended[0].exit.as_ref().map(|e| e.value),
            Some((Some(2), None))
        );
        // A lost session stays held.
        j.opened("b", Kind::Pty, 4);
        j.closed("b", false);
        assert_eq!(j.held().len(), 1);
    }

    #[test]
    fn activity_is_reported_at_most_once_a_second() {
        let mut j = Journal::default();
        let t = Instant::now();
        assert!(j.activity("a", t));
        assert!(!j.activity("a", t + Duration::from_millis(500)));
        assert!(j.activity("a", t + ACTIVITY_EVERY));
    }
}

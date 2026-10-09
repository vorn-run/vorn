//! The size rule as vornd runs it (Terminal State Protocol §10): one
//! [`vorn_size::Policy`] per session, fed by both kinds of client.
//!
//! Bytes clients on the WebSocket report `terminal:viewport` and
//! `terminal:presence`, and their `terminal:write`s are input when a person
//! typed them ([`vorn_size::typed`]); grid clients on the local socket send
//! Viewport, Presence, TakeSize and LockSize, and their Input. Both land
//! here as [`Ev`]s under the client's [`Who`].
//!
//! Who is a desktop is decided from the connection, never from what the
//! client says: every grid client is on this user's local socket, and a
//! bytes connection is the desktop's only when it opened with the desktop's
//! launch token ([`Sizes::desktop`],
//! [`crate::endpoint::is_desktop_credential`]). A phone over the tunnel cannot claim
//! it. So is who opened a session: the connection whose `terminal:create`
//! (or `shell:create`, `sessions:resume`) answered with it ([`Sizes::opened_by`]).
//!
//! The policy decides; the engine's driver sends. It asks [`Sizes::due`]
//! when to look again and [`Sizes::poll`] for the resizes to send, and each
//! one goes to sessiond as a Resize request. The terminal and every client
//! resize when the Resize record comes back, never on the request
//! ([`Sizes::applied`] names who asked for it, for `Resized`).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use tokio::sync::Notify;
use vorn_engine::Peer;
use vorn_size::{Decision, Event, Policy, Presence, Size};

/// A client as the size rule knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Who {
    /// One pane on a bytes connection on the WebSocket. Every window of the
    /// desktop shares the app's one connection, so a pane names itself with
    /// `pane` in what it reports, the way a grid attachment has its sid
    /// (TP §7): two panes showing one session never overwrite each other's
    /// box or presence. `pane` 0 is the connection as a whole, for a client
    /// that names no pane.
    Bytes { conn: u64, pane: u64 },
    /// One grid attachment.
    Grid(Peer),
}

impl Who {
    /// The name a bytes client sees in `terminal:resized`'s `owner`, and is
    /// told as its own `client` when it attaches.
    pub fn name(self) -> String {
        match self {
            Who::Bytes { conn, pane: 0 } => format!("ws:{conn}"),
            Who::Bytes { conn, pane } => format!("ws:{conn}:{pane}"),
            Who::Grid(p) => format!("grid:{}:{}", p.conn, p.sid),
        }
    }
}

/// What a client did, for [`Sizes::on`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ev {
    /// It attached; a client that is already attached keeps what the policy
    /// knows of it, and only takes the viewport given.
    Attach {
        viewport: Option<Size>,
        presence: Presence,
    },
    Viewport(Size),
    Presence(Presence),
    /// A person's input reached the session from it.
    Input,
    /// "Fit to this device", or an older client's `terminal:resize` with the
    /// size it wants.
    TakeSize(Option<Size>),
    LockSize(bool),
    Detach,
}

/// Resizes sent and not answered by a record yet, per session. Past this,
/// the oldest is forgotten: its record then reaches clients without an
/// owner, which costs nothing but the name.
const SENT_KEPT: usize = 16;

/// Sessions asked for and not open yet, with the connection that asked.
/// Past this the oldest is forgotten: a start that failed never opens, and
/// a forgotten opener only means its pane waits for a key to fit.
const OPENERS_KEPT: usize = 64;

#[derive(Debug)]
struct Held {
    policy: Policy<Who>,
    /// The bytes connection that asked for the session: its panes take the
    /// size until someone types ([`vorn_size::Event::Attach`]'s `opener`).
    opener: Option<u64>,
    /// Resizes sent to sessiond, oldest first, for the records that answer
    /// them.
    sent: VecDeque<Decision<Who>>,
    /// Its policy's [`Policy::due`], as filed in [`Inner::due`].
    due: Option<Instant>,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, Held>,
    /// Bytes connections that opened with the desktop's launch token.
    desktops: HashSet<u64>,
    /// Who asked for each session not open yet, oldest first.
    openers: VecDeque<(String, u64)>,
    /// Every session with something due, by when: the driver asks after
    /// every message, so this must not look at every session.
    due: BTreeSet<(Instant, String)>,
}

impl Inner {
    /// Takes the connection that asked for `session` before it opened.
    fn opener(&mut self, session: &str) -> Option<u64> {
        let at = self.openers.iter().position(|(id, _)| id == session)?;
        self.openers.remove(at).map(|(_, conn)| conn)
    }

    /// Files `session` under its policy's due time again, after the policy
    /// changed.
    fn refile(&mut self, session: &str) {
        let Some(h) = self.sessions.get_mut(session) else {
            return;
        };
        let due = h.policy.due();
        if due == h.due {
            return;
        }
        if let Some(at) = std::mem::replace(&mut h.due, due) {
            self.due.remove(&(at, session.to_owned()));
        }
        if let Some(at) = due {
            self.due.insert((at, session.to_owned()));
        }
    }
}

/// Every session's size rule.
#[derive(Debug, Default)]
pub struct Sizes {
    inner: Mutex<Inner>,
    /// Woken by every event, so the driver looks at [`Sizes::due`] again.
    wake: Notify,
}

impl Sizes {
    pub fn new() -> Sizes {
        Sizes::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A session with a PTY of `size` is held. A session already known (a
    /// new connection to sessiond recovering it) keeps its clients and its
    /// history of input, and learns its size.
    pub fn opened(&self, session: &str, size: Size) {
        let mut inner = self.inner();
        let opener = inner.opener(session);
        match inner.sessions.get_mut(session) {
            Some(h) => h.policy.applied(size),
            None => {
                inner.sessions.insert(
                    session.to_owned(),
                    Held {
                        policy: Policy::new(size),
                        opener,
                        sent: VecDeque::new(),
                        due: None,
                    },
                );
            }
        }
        inner.refile(session);
    }

    /// Bytes connection `conn` asked for `session`, which it may not hold
    /// yet: its panes fit the session until someone types into it. A
    /// session already open is only given an opener while nobody is
    /// attached to it or has typed into it, so asking for one that runs
    /// takes nothing from the clients showing it.
    pub fn opened_by(&self, session: &str, conn: u64) {
        let mut inner = self.inner();
        if let Some(h) = inner.sessions.get_mut(session) {
            if !h.policy.typed() && h.policy.clients().next().is_none() {
                h.opener = Some(conn);
            }
            return;
        }
        inner.opener(session);
        if inner.openers.len() == OPENERS_KEPT {
            inner.openers.pop_front();
        }
        inner.openers.push_back((session.to_owned(), conn));
    }

    /// The session ended.
    pub fn closed(&self, session: &str) {
        let mut inner = self.inner();
        inner.opener(session);
        if let Some(at) = inner.sessions.remove(session).and_then(|h| h.due) {
            inner.due.remove(&(at, session.to_owned()));
        }
    }

    /// Bytes connection `conn` opened with the desktop's launch token.
    pub fn desktop(&self, conn: u64) {
        self.inner().desktops.insert(conn);
    }

    /// One client's event for `session`. A client the policy has not seen
    /// is attached first, so an event never depends on an attach arriving
    /// before it. Nothing for a session without a PTY, which has no size.
    pub fn on(&self, session: &str, who: Who, ev: Ev, now: Instant) {
        {
            let mut inner = self.inner();
            let desktop = match who {
                Who::Grid(_) => true,
                Who::Bytes { conn, .. } => inner.desktops.contains(&conn),
            };
            let Some(h) = inner.sessions.get_mut(session) else {
                return;
            };
            let opener = matches!(who, Who::Bytes { conn, .. } if h.opener == Some(conn));
            let p = &mut h.policy;
            if ev == Ev::Detach {
                p.on(Event::Detach { client: who }, now);
            } else {
                if !p.has(who) {
                    let (viewport, presence) = match ev {
                        Ev::Attach { viewport, presence } => (viewport, presence),
                        Ev::Presence(state) => (None, state),
                        _ => (None, Presence::Watching),
                    };
                    p.on(
                        Event::Attach {
                            client: who,
                            desktop,
                            opener,
                            viewport,
                            presence,
                        },
                        now,
                    );
                }
                let event = match ev {
                    Ev::Attach { viewport, .. } => {
                        viewport.map(|size| Event::Viewport { client: who, size })
                    }
                    Ev::Viewport(size) => Some(Event::Viewport { client: who, size }),
                    Ev::Presence(state) => Some(Event::Presence { client: who, state }),
                    Ev::Input => Some(Event::Input { client: who }),
                    Ev::TakeSize(size) => Some(Event::TakeSize { client: who, size }),
                    Ev::LockSize(locked) => Some(Event::LockSize {
                        client: who,
                        locked,
                    }),
                    Ev::Detach => None,
                };
                if let Some(e) = event {
                    p.on(e, now);
                }
            }
            inner.refile(session);
        }
        self.wake.notify_one();
    }

    /// A lock asked for, or released. A lock another client holds is
    /// refused, and the refusal is the answer the client gets; a release by
    /// a client that holds none changes nothing.
    pub fn lock(&self, session: &str, who: Who, locked: bool, now: Instant) -> Result<(), String> {
        if locked {
            let inner = self.inner();
            let held = inner
                .sessions
                .get(session)
                .and_then(|h| h.policy.locked_by());
            if held.is_some_and(|w| w != who) {
                return Err("the size is locked by another client".to_owned());
            }
        }
        self.on(session, who, Ev::LockSize(locked), now);
        Ok(())
    }

    /// Bytes connection `conn` closed: it detaches from every session.
    pub fn bytes_gone(&self, conn: u64, now: Instant) {
        self.gone(
            |w| matches!(w, Who::Bytes { conn: c, .. } if c == conn),
            now,
        );
        self.inner().desktops.remove(&conn);
    }

    /// Grid connection `conn` closed: each of its attachments detaches.
    pub fn grid_gone(&self, conn: u64, now: Instant) {
        self.gone(|w| matches!(w, Who::Grid(p) if p.conn == conn), now);
    }

    fn gone(&self, which: impl Fn(Who) -> bool, now: Instant) {
        {
            let mut inner = self.inner();
            let mut left = Vec::new();
            for (id, h) in &mut inner.sessions {
                let leaving: Vec<Who> = h.policy.clients().filter(|&w| which(w)).collect();
                if !leaving.is_empty() {
                    left.push(id.clone());
                }
                for client in leaving {
                    h.policy.on(Event::Detach { client }, now);
                }
            }
            for id in left {
                inner.refile(&id);
            }
        }
        self.wake.notify_one();
    }

    /// Resolves after the next event. A wake that came while nobody waited
    /// is kept for the next wait.
    pub async fn woken(&self) {
        self.wake.notified().await;
    }

    /// When some session next has a resize to send, without a new event.
    pub fn due(&self) -> Option<Instant> {
        self.inner().due.first().map(|(at, _)| *at)
    }

    /// The resizes to send now, each to its session. Each is remembered for
    /// the record that will answer it.
    pub fn poll(&self, now: Instant) -> Vec<(String, Decision<Who>)> {
        let mut out = Vec::new();
        let mut inner = self.inner();
        let mut ready = Vec::new();
        while inner.due.first().is_some_and(|(at, _)| *at <= now) {
            ready.extend(inner.due.pop_first().map(|(_, id)| id));
        }
        for id in ready {
            let Some(h) = inner.sessions.get_mut(&id) else {
                continue;
            };
            h.due = None;
            if let Some(d) = h.policy.poll(now) {
                if h.sent.len() == SENT_KEPT {
                    h.sent.pop_front();
                }
                h.sent.push_back(d);
                out.push((id.clone(), d));
            }
            inner.refile(&id);
        }
        out
    }

    /// A Resize record for `size` was applied to `session`: the resize it
    /// answers, if vornd sent one. sessiond applies requests in order, so
    /// it is the oldest sent for that size; older ones it passed were
    /// overtaken and never get a record of their own.
    pub fn applied(&self, session: &str, size: Size) -> Option<Decision<Who>> {
        let mut inner = self.inner();
        let h = inner.sessions.get_mut(session)?;
        let found = h.sent.iter().position(|d| d.size == size);
        let decision = found.map(|i| {
            h.sent.drain(..i);
            h.sent.pop_front().expect("position found it")
        });
        // While later requests are on their way, the size they will bring
        // is the one the policy should measure against.
        if h.sent.is_empty() {
            h.policy.applied(size);
        }
        inner.refile(session);
        decision
    }

    /// The grid attachments on `session`, for `Resized`.
    pub fn grid_peers(&self, session: &str) -> Vec<Peer> {
        self.inner()
            .sessions
            .get(session)
            .map_or_else(Vec::new, |h| {
                h.policy
                    .clients()
                    .filter_map(|w| match w {
                        Who::Grid(p) => Some(p),
                        Who::Bytes { .. } => None,
                    })
                    .collect()
            })
    }

    /// The owner and size of `session`, for tests and the report.
    pub fn state(&self, session: &str) -> Option<(Size, Option<Who>)> {
        self.inner()
            .sessions
            .get(session)
            .map(|h| (h.policy.size(), h.policy.owner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::is_desktop_credential;
    use std::time::Duration;
    use vorn_size::Reason;

    const S: &str = "s1";

    fn peer(conn: u64, sid: u32) -> Who {
        Who::Grid(Peer { conn, sid })
    }

    /// TP-T9a's last clause: whether a bytes client is a desktop comes from
    /// the token its connection opened with, never from what it says. A
    /// client without the token is remote whatever it claims, and loses
    /// ties to the desktop.
    #[test]
    fn t9a_a_native_claim_over_the_tunnel_is_still_remote() {
        let token = b"launch-secret";
        assert!(is_desktop_credential(Some(b"Bearer launch-secret"), token));
        for wrong in [
            &b"Bearer launch-secreT"[..],
            b"Bearer launch",
            b"launch-secret",
            b"Basic launch-secret",
            b"Bearer ",
        ] {
            assert!(!is_desktop_credential(Some(wrong), token), "{wrong:?}");
        }
        assert!(!is_desktop_credential(None, token));
        assert!(!is_desktop_credential(Some(b"Bearer "), b""));

        // Connection 1 opened with the token; connection 2, a browser over
        // the tunnel saying it is native, did not.
        let sizes = Sizes::new();
        sizes.opened(S, Size::new(100, 30));
        sizes.desktop(1);
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let view = |cols, rows| Ev::Attach {
            viewport: Some(Size::new(cols, rows)),
            presence: Presence::Watching,
        };
        sizes.on(S, Who::Bytes { conn: 2, pane: 0 }, view(50, 30), t0);
        sizes.on(S, Who::Bytes { conn: 1, pane: 0 }, view(120, 40), t0);
        sizes.on(S, Who::Bytes { conn: 2, pane: 0 }, Ev::Input, t0);
        sizes.on(S, Who::Bytes { conn: 1, pane: 0 }, Ev::Input, at(500));
        sizes.on(S, Who::Bytes { conn: 2, pane: 0 }, Ev::Input, at(1000));
        assert!(sizes
            .poll(at(2000))
            .iter()
            .all(|(_, d)| d.owner == Some(Who::Bytes { conn: 1, pane: 0 })));
        assert_eq!(
            sizes.state(S).unwrap().1,
            Some(Who::Bytes { conn: 1, pane: 0 })
        );
    }

    #[test]
    fn a_record_names_the_resize_it_answers_and_skips_the_overtaken() {
        let sizes = Sizes::new();
        sizes.opened(S, Size::new(80, 24));
        let t0 = Instant::now();
        let g = peer(4, 1);
        sizes.on(
            S,
            g,
            Ev::Attach {
                viewport: Some(Size::new(120, 40)),
                presence: Presence::Watching,
            },
            t0,
        );
        sizes.on(S, g, Ev::Input, t0);
        let sent = sizes.poll(t0 + Duration::from_millis(150));
        assert_eq!(sent.len(), 1);
        sizes.on(
            S,
            g,
            Ev::Viewport(Size::new(140, 50)),
            t0 + Duration::from_secs(1),
        );
        assert_eq!(sizes.poll(t0 + Duration::from_secs(2)).len(), 1);
        // The first request was overtaken: only the second's record comes.
        let d = sizes.applied(S, Size::new(140, 50)).unwrap();
        assert_eq!((d.owner, d.reason), (Some(g), Reason::Input));
        // A record nobody here asked for (a redraw nudge) names no one.
        assert_eq!(sizes.applied(S, Size::new(140, 49)), None);
        assert_eq!(sizes.grid_peers(S), vec![Peer { conn: 4, sid: 1 }]);
        sizes.grid_gone(4, t0 + Duration::from_secs(3));
        assert!(sizes.grid_peers(S).is_empty());
    }

    /// Two panes of the desktop's one connection, one hidden: the other
    /// still watches, so the size stays; and a second lock is refused.
    #[test]
    fn panes_on_one_connection_are_separate_clients_and_a_lock_is_kept() {
        let sizes = Sizes::new();
        sizes.opened(S, Size::new(100, 30));
        sizes.desktop(1);
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let (a, b, phone) = (
            Who::Bytes { conn: 1, pane: 11 },
            Who::Bytes { conn: 1, pane: 12 },
            Who::Bytes { conn: 2, pane: 0 },
        );
        let view = |cols, rows| Ev::Attach {
            viewport: Some(Size::new(cols, rows)),
            presence: Presence::Watching,
        };
        sizes.on(S, a, view(120, 40), t0);
        sizes.on(S, b, view(80, 20), t0);
        sizes.on(S, phone, view(50, 30), t0);
        sizes.on(S, a, Ev::Input, t0);
        assert_eq!(sizes.poll(at(200)).len(), 1);
        sizes.applied(S, Size::new(120, 40));
        // The second window hides its pane; the first still shows the session.
        sizes.on(S, b, Ev::Presence(Presence::Away), at(1_000));
        assert_eq!(sizes.poll(at(20_000)), Vec::new());
        assert_eq!(sizes.state(S).unwrap(), (Size::new(120, 40), Some(a)));
        assert_eq!(a.name(), "ws:1:11");

        assert_eq!(sizes.lock(S, phone, true, at(21_000)), Ok(()));
        assert!(sizes.lock(S, a, true, at(22_000)).is_err());
        assert_eq!(sizes.state(S).unwrap().1, Some(phone));
        assert_eq!(sizes.lock(S, phone, false, at(23_000)), Ok(()));
        assert_eq!(sizes.lock(S, a, true, at(24_000)), Ok(()));
    }

    /// The connection that asked for a session before it started fits it
    /// to its pane without typing; another connection, or asking for a
    /// session already shown, does not.
    #[test]
    fn the_connection_that_opened_a_session_fits_it_to_its_pane() {
        let sizes = Sizes::new();
        sizes.desktop(1);
        sizes.opened_by(S, 1);
        sizes.opened(S, Size::new(80, 24));
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let view = |cols, rows| Ev::Attach {
            viewport: Some(Size::new(cols, rows)),
            presence: Presence::Watching,
        };
        let (pane, other) = (
            Who::Bytes { conn: 1, pane: 7 },
            Who::Bytes { conn: 2, pane: 0 },
        );
        sizes.on(S, other, view(50, 30), t0);
        sizes.on(S, pane, view(200, 50), t0);
        let sent = sizes.poll(at(2_000));
        assert_eq!(sent.len(), 1);
        let d = &sent[0].1;
        assert_eq!(
            (d.size, d.owner, d.reason),
            (Size::new(200, 50), Some(pane), Reason::Launch)
        );

        // A second session, shown by connection 2 before connection 1 asks.
        let s2 = "s2";
        sizes.opened(s2, Size::new(80, 24));
        sizes.on(s2, other, view(50, 30), at(3_000));
        sizes.opened_by(s2, 1);
        sizes.on(s2, pane, view(200, 50), at(3_000));
        assert!(sizes.poll(at(5_000)).iter().all(|(s, _)| s != s2));
        assert_eq!(sizes.state(s2).unwrap(), (Size::new(80, 24), None));

        // An opener for a session that never opened is forgotten with it.
        sizes.opened_by("s3", 1);
        sizes.closed("s3");
        assert!(sizes.inner().openers.is_empty());
    }

    #[test]
    fn events_for_sessions_without_a_size_are_ignored() {
        let sizes = Sizes::new();
        sizes.on(
            "piped",
            Who::Bytes { conn: 1, pane: 0 },
            Ev::Input,
            Instant::now(),
        );
        assert_eq!(sizes.state("piped"), None);
        assert_eq!(sizes.due(), None);
    }

    /// The earliest of many sessions' resizes is the one due, only due
    /// sessions are polled, and a closed session leaves nothing due.
    #[test]
    fn the_earliest_due_session_comes_first_and_a_closed_one_drops_out() {
        let sizes = Sizes::new();
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let ask = |id: &str, ms| {
            let g = peer(1, ms as u32);
            sizes.on(
                id,
                g,
                Ev::Attach {
                    viewport: Some(Size::new(120, 40)),
                    presence: Presence::Watching,
                },
                at(ms),
            );
            sizes.on(id, g, Ev::Input, at(ms));
        };
        for id in ["a", "b", "c"] {
            sizes.opened(id, Size::new(80, 24));
        }
        assert_eq!(sizes.due(), None);
        ask("b", 50);
        ask("a", 0);
        ask("c", 100);
        let first = sizes.due().unwrap();
        assert!(first <= at(150));
        let sent: Vec<String> = sizes.poll(first).into_iter().map(|(id, _)| id).collect();
        assert_eq!(sent, vec!["a".to_owned()]);
        sizes.closed("b");
        let sent: Vec<String> = sizes
            .poll(at(10_000))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(sent, vec!["c".to_owned()]);
        assert_eq!(sizes.due(), None);
    }
}

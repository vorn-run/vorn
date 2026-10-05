//! vorn-size: who decides a shared terminal's size (Terminal State Protocol
//! §10, "The rule").
//!
//! A PTY has one size and several clients may be attached to it at once: the
//! desktop, a phone, a browser. Each resize costs the program a SIGWINCH and a
//! redraw, and for an agent drawing inline on the primary screen that redraw
//! can clear and reprint its output. So the size follows intent, not
//! presence:
//!
//! - clients report, vornd decides: a client's [`Event::Viewport`] says what
//!   fits on it and never asks for a resize; its [`Presence`] says whether it
//!   is in use, on screen or away;
//! - only input takes the size, and a takeover waits until the owner has been
//!   quiet for [`Rules::quiet`] or is away; when both are active the desktop
//!   keeps it;
//! - when the owner has been away for [`Rules::grace`], the size returns to a
//!   client that is watching, a desktop first;
//! - a viewport within [`Rules::near_cols`] and [`Rules::near_rows`] of the
//!   current size is a near miss and resizes nothing: the owner scales its
//!   font instead;
//! - a new size is applied once it has stood for [`Rules::settle`];
//! - a session nobody has typed into keeps the size it was launched with;
//! - any client can take the size explicitly or lock it to its own viewport,
//!   until it releases the lock or detaches.
//!
//! [`Policy`] is one session's rule as a state machine. It does no I/O and
//! reads no clock: every call is given the time, so the tests drive it through
//! minutes of activity in microseconds and every run is the same. Its output is
//! at most one pending [`Decision`], which [`Policy::poll`] hands over once it
//! is due; the host sends it to the session holder and calls
//! [`Policy::applied`] when the resize record comes back.
//!
//! [`fit`] is the other half of the rule, the one clients follow: a client
//! that does not own the size draws the whole grid, scaled and then panned,
//! never clipped. [`typed`] tells a person's typing apart from the reports a
//! terminal sends on its own, for clients that only send bytes.

mod fit;
mod typing;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::time::{Duration, Instant};

pub use fit::{fit, Fit};
pub use typing::typed;

/// A grid size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    pub const fn new(cols: u16, rows: u16) -> Size {
        Size { cols, rows }
    }

    /// The size, unless either side is zero: a client with no box to measure
    /// (a hidden pane, an attach that names no view) reports nothing.
    pub fn usable(self) -> Option<Size> {
        (self.cols > 0 && self.rows > 0).then_some(self)
    }

    /// Whether `other` is close enough to this size that its client fits the
    /// grid by scaling its font rather than by a resize.
    fn near(self, other: Size, rules: &Rules) -> bool {
        self.cols.abs_diff(other.cols) <= rules.near_cols
            && self.rows.abs_diff(other.rows) <= rules.near_rows
    }
}

/// Whether a client is in use (TP §7 Presence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presence {
    /// Keyboard, paste, mouse input or scrolling in this terminal in the last
    /// few seconds.
    Active,
    /// The pane is on screen.
    #[default]
    Watching,
    /// Hidden, the app in the background, the phone locked.
    Away,
}

/// Why the session has the size it has (TP §7 Resized).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The owner typed into it, or its viewport changed while it owned it.
    Input,
    /// The owner went away and the size came back to a watcher.
    Returned,
    /// A client took it with "Fit to this device", or an older client asked
    /// for it with `terminal:resize`.
    Explicit,
    /// A client locked it to its own viewport.
    Locked,
    /// The size the session was launched with. The policy never decides it;
    /// it names the size a session has before anyone has typed into it.
    Launch,
}

impl Reason {
    /// The name clients see.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Input => "input",
            Reason::Returned => "returned",
            Reason::Explicit => "explicit",
            Reason::Locked => "locked",
            Reason::Launch => "launch",
        }
    }
}

/// A resize the policy decided: the size, who owns it, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision<C> {
    pub size: Size,
    pub owner: Option<C>,
    pub reason: Reason,
}

/// The rule's numbers. [`Rules::default`] is TP §10's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// How long the owner must have sent no input before another client's
    /// input takes the size.
    pub quiet: Duration,
    /// How long the owner must have been away (or gone) before the size
    /// returns to a watcher.
    pub grace: Duration,
    /// How long a new size must stand before it is applied.
    pub settle: Duration,
    /// A viewport this many columns or fewer from the current size, and
    /// [`Rules::near_rows`] rows or fewer, resizes nothing.
    pub near_cols: u16,
    pub near_rows: u16,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            quiet: Duration::from_secs(3),
            grace: Duration::from_secs(10),
            settle: Duration::from_millis(150),
            near_cols: 2,
            near_rows: 1,
        }
    }
}

/// What a client did. `C` names the client; the host chooses what it is
/// (a connection, an attachment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<C> {
    /// A client attached. `desktop` is decided by the host from the
    /// connection, never from what the client says about itself.
    Attach {
        client: C,
        desktop: bool,
        viewport: Option<Size>,
        presence: Presence,
    },
    /// What fits on the client at its preferred font. Never a resize request.
    Viewport {
        client: C,
        size: Size,
    },
    Presence {
        client: C,
        state: Presence,
    },
    /// A person's input reached the session from this client: a key, a
    /// paste, a click. Not a focus report, not scrolling.
    Input {
        client: C,
    },
    /// "Fit to this device": the size, exactly, for this client. `size` sets
    /// its viewport first, for an older client whose `terminal:resize` names
    /// the size it wants.
    TakeSize {
        client: C,
        size: Option<Size>,
    },
    /// Lock the size to this client's viewport, or release its lock.
    LockSize {
        client: C,
        locked: bool,
    },
    Detach {
        client: C,
    },
}

/// One client as the policy knows it.
#[derive(Debug, Clone, Copy)]
struct Client {
    desktop: bool,
    viewport: Option<Size>,
    presence: Presence,
    /// Since when it has been away, while it is.
    away_since: Option<Instant>,
    last_input: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Pending<C> {
    decision: Decision<C>,
    due: Instant,
}

/// One session's size rule.
#[derive(Debug, Clone)]
pub struct Policy<C> {
    rules: Rules,
    /// The size the session has, or was last asked to take.
    size: Size,
    launch: Size,
    /// Whether a person has sent input to the session since it launched.
    typed: bool,
    owner: Option<C>,
    /// How the owner came to own the size; its viewport changes carry it on.
    owner_reason: Reason,
    /// Since when the owner has been gone (detached), while nobody owns it.
    orphaned: Option<Instant>,
    lock: Option<C>,
    clients: BTreeMap<C, Client>,
    pending: Option<Pending<C>>,
}

impl<C: Copy + Ord + Debug> Policy<C> {
    /// A session launched at `launch`, with TP §10's rules.
    pub fn new(launch: Size) -> Policy<C> {
        Policy::with_rules(launch, Rules::default())
    }

    pub fn with_rules(launch: Size, rules: Rules) -> Policy<C> {
        Policy {
            rules,
            size: launch,
            launch,
            typed: false,
            owner: None,
            owner_reason: Reason::Launch,
            orphaned: None,
            lock: None,
            clients: BTreeMap::new(),
            pending: None,
        }
    }

    /// The size the session has, or was last asked to take.
    pub fn size(&self) -> Size {
        self.size
    }

    pub fn launch(&self) -> Size {
        self.launch
    }

    /// The client whose size the session follows, if any.
    pub fn owner(&self) -> Option<C> {
        self.owner
    }

    /// The client holding a lock, if any.
    pub fn locked_by(&self) -> Option<C> {
        self.lock
    }

    /// Whether anyone has typed into the session since it launched.
    pub fn typed(&self) -> bool {
        self.typed
    }

    /// Whether `client` is attached.
    pub fn has(&self, client: C) -> bool {
        self.clients.contains_key(&client)
    }

    /// The clients attached, in key order.
    pub fn clients(&self) -> impl Iterator<Item = C> + '_ {
        self.clients.keys().copied()
    }

    /// The resize waiting to settle, if any.
    pub fn pending(&self) -> Option<Decision<C>> {
        self.pending.map(|p| p.decision)
    }

    /// Takes one event. Nothing is decided here that is not also waiting
    /// for [`Policy::poll`]: a resize always settles first.
    pub fn on(&mut self, event: Event<C>, now: Instant) {
        match event {
            Event::Attach {
                client,
                desktop,
                viewport,
                presence,
            } => {
                self.clients.insert(
                    client,
                    Client {
                        desktop,
                        viewport: viewport.and_then(Size::usable),
                        presence,
                        away_since: (presence == Presence::Away).then_some(now),
                        last_input: None,
                    },
                );
            }
            Event::Viewport { client, size } => {
                let Some(c) = self.clients.get_mut(&client) else {
                    return;
                };
                let Some(size) = size.usable() else {
                    return;
                };
                c.viewport = Some(size);
                let away = c.presence == Presence::Away;
                // Looking never resizes: only the owner's viewport moves the
                // size, and not while it is away, when its box means nothing.
                if self.owner == Some(client) && !away {
                    self.want(size, self.owner_reason, now);
                }
            }
            Event::Presence { client, state } => {
                let Some(c) = self.clients.get_mut(&client) else {
                    return;
                };
                c.away_since = match state {
                    Presence::Away => c.away_since.or(Some(now)),
                    _ => None,
                };
                c.presence = state;
            }
            Event::Input { client } => self.input(client, now),
            Event::TakeSize { client, size } => {
                if self.lock.is_some_and(|l| l != client) {
                    return;
                }
                let Some(c) = self.clients.get_mut(&client) else {
                    return;
                };
                if let Some(size) = size.and_then(Size::usable) {
                    c.viewport = Some(size);
                }
                let viewport = c.viewport;
                self.own(client, Reason::Explicit);
                if let Some(v) = viewport {
                    self.want(v, Reason::Explicit, now);
                }
            }
            Event::LockSize { client, locked } => {
                if !locked {
                    if self.lock == Some(client) {
                        self.lock = None;
                        // From here its viewport moves the size the way any
                        // owner's does.
                        self.owner_reason = Reason::Input;
                    }
                    return;
                }
                let Some(c) = self.clients.get(&client) else {
                    return;
                };
                let viewport = c.viewport;
                self.lock = Some(client);
                self.own(client, Reason::Locked);
                if let Some(v) = viewport {
                    self.want(v, Reason::Locked, now);
                }
            }
            Event::Detach { client } => {
                if self.clients.remove(&client).is_none() {
                    return;
                }
                if self.lock == Some(client) {
                    self.lock = None;
                }
                if self.owner == Some(client) {
                    self.owner = None;
                    self.orphaned = Some(now);
                    if self
                        .pending
                        .is_some_and(|p| p.decision.owner == Some(client))
                    {
                        self.pending = None;
                    }
                }
            }
        }
    }

    /// The resize to send now, if one is due. Also returns the size home
    /// once its owner has been away for the grace period.
    pub fn poll(&mut self, now: Instant) -> Option<Decision<C>> {
        if let Some(since) = self.orphaned_since() {
            if now >= since + self.rules.grace {
                if let Some(home) = self.home() {
                    self.own(home, Reason::Returned);
                    if let Some(v) = self.clients.get(&home).and_then(|c| c.viewport) {
                        self.want(v, Reason::Returned, now);
                    }
                }
            }
        }
        let p = self.pending.filter(|p| p.due <= now)?;
        self.pending = None;
        self.size = p.decision.size;
        Some(p.decision)
    }

    /// When [`Policy::poll`] next has something to do, if ever without a
    /// new event.
    pub fn due(&self) -> Option<Instant> {
        let settle = self.pending.map(|p| p.due);
        let home = self
            .orphaned_since()
            .filter(|_| self.home().is_some())
            .map(|t| t + self.rules.grace);
        match (settle, home) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// A resize record for `size` was applied: the session has that size now,
    /// whoever asked for it.
    pub fn applied(&mut self, size: Size) {
        self.size = size;
        if self.pending.is_some_and(|p| p.decision.size == size) {
            self.pending = None;
        }
    }

    fn input(&mut self, client: C, now: Instant) {
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        c.last_input = Some(now);
        c.presence = Presence::Active;
        c.away_since = None;
        let viewport = c.viewport;
        self.typed = true;
        if !self.may_take(client, now) {
            return;
        }
        if self.owner != Some(client) {
            self.own(client, Reason::Input);
        }
        if let Some(v) = viewport {
            self.want(v, self.owner_reason, now);
        }
    }

    /// Whether `client`'s input may take the size now (TP §10, hysteresis).
    fn may_take(&self, client: C, now: Instant) -> bool {
        if let Some(l) = self.lock {
            return l == client;
        }
        let Some(owner) = self.owner.filter(|&o| o != client) else {
            return true;
        };
        let Some(o) = self.clients.get(&owner) else {
            return true;
        };
        let quiet = o
            .last_input
            .is_none_or(|t| now.saturating_duration_since(t) >= self.rules.quiet);
        // Both active: the desktop keeps it, and takes it from a phone.
        let desktop_wins = self.clients.get(&client).is_some_and(|c| c.desktop) && !o.desktop;
        o.presence == Presence::Away || quiet || desktop_wins
    }

    fn own(&mut self, client: C, reason: Reason) {
        self.owner = Some(client);
        self.owner_reason = reason;
        self.orphaned = None;
    }

    /// Since when the size has had nobody using it: its owner away or gone.
    /// Never while it is locked.
    fn orphaned_since(&self) -> Option<Instant> {
        if self.lock.is_some() {
            return None;
        }
        match self.owner {
            Some(o) => self
                .clients
                .get(&o)
                .filter(|c| c.presence == Presence::Away)
                .and_then(|c| c.away_since),
            None => self.orphaned,
        }
    }

    /// Where the size goes home to: a client on screen with a viewport, a
    /// desktop first, then whoever typed last, then the first by key so the
    /// choice is always the same.
    fn home(&self) -> Option<C> {
        self.clients
            .iter()
            .filter(|(&k, c)| {
                Some(k) != self.owner && c.presence != Presence::Away && c.viewport.is_some()
            })
            .max_by_key(|(&k, c)| (c.desktop, c.last_input, Reverse(k)))
            .map(|(&k, _)| k)
    }

    /// Asks for `size`, once it has settled. A near miss (for the automatic
    /// reasons) or the size the session already has cancels what was
    /// waiting instead; the same size again keeps its deadline, so an owner
    /// typing steadily does not hold its own resize back.
    fn want(&mut self, size: Size, reason: Reason, now: Instant) {
        let exact = matches!(reason, Reason::Explicit | Reason::Locked);
        let skip = if exact {
            size == self.size
        } else {
            size.near(self.size, &self.rules)
        };
        if skip {
            self.pending = None;
            return;
        }
        let decision = Decision {
            size,
            owner: self.owner,
            reason,
        };
        match &mut self.pending {
            Some(p) if p.decision.size == size => p.decision = decision,
            _ => {
                self.pending = Some(Pending {
                    decision,
                    due: now + self.rules.settle,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    fn attach(p: &mut Policy<u8>, c: u8, desktop: bool, v: Size, now: Instant) {
        p.on(
            Event::Attach {
                client: c,
                desktop,
                viewport: Some(v),
                presence: Presence::Watching,
            },
            now,
        );
    }

    #[test]
    fn a_zero_sized_viewport_is_no_viewport() {
        let t0 = Instant::now();
        let mut p = Policy::new(Size::new(80, 24));
        attach(&mut p, 1, true, Size::new(0, 30), t0);
        p.on(Event::Input { client: 1 }, t0);
        assert_eq!(p.owner(), Some(1));
        assert_eq!(p.due(), None, "nothing to resize to");
        p.on(
            Event::Viewport {
                client: 1,
                size: Size::new(100, 0),
            },
            t0,
        );
        assert_eq!(p.due(), None);
    }

    #[test]
    fn the_same_target_keeps_its_deadline_and_a_new_one_restarts_it() {
        let t0 = Instant::now();
        let mut p = Policy::new(Size::new(80, 24));
        attach(&mut p, 1, true, Size::new(120, 40), t0);
        p.on(Event::Input { client: 1 }, t0);
        assert_eq!(p.due(), Some(at(t0, 150)));
        p.on(Event::Input { client: 1 }, at(t0, 100));
        assert_eq!(p.due(), Some(at(t0, 150)), "typing does not hold it back");
        p.on(
            Event::Viewport {
                client: 1,
                size: Size::new(130, 40),
            },
            at(t0, 120),
        );
        assert_eq!(p.due(), Some(at(t0, 270)));
        assert_eq!(p.poll(at(t0, 269)), None);
        let d = p.poll(at(t0, 270)).unwrap();
        assert_eq!(
            (d.size, d.owner, d.reason),
            (Size::new(130, 40), Some(1), Reason::Input)
        );
        assert_eq!(p.size(), Size::new(130, 40));
    }

    #[test]
    fn a_lock_holds_until_released_or_its_client_detaches() {
        let t0 = Instant::now();
        let mut p = Policy::new(Size::new(80, 24));
        attach(&mut p, 1, true, Size::new(120, 40), t0);
        attach(&mut p, 2, false, Size::new(50, 30), t0);
        p.on(
            Event::LockSize {
                client: 2,
                locked: true,
            },
            t0,
        );
        let d = p.poll(at(t0, 150)).unwrap();
        assert_eq!((d.size, d.reason), (Size::new(50, 30), Reason::Locked));
        // The desktop types long after: the lock holds.
        p.on(Event::Input { client: 1 }, at(t0, 10_000));
        p.on(
            Event::TakeSize {
                client: 1,
                size: None,
            },
            at(t0, 10_000),
        );
        assert_eq!(p.poll(at(t0, 20_000)), None);
        assert_eq!(p.owner(), Some(2));
        // A release by someone else is nothing; the locker's own releases it.
        p.on(
            Event::LockSize {
                client: 1,
                locked: false,
            },
            at(t0, 20_000),
        );
        assert_eq!(p.locked_by(), Some(2));
        p.on(Event::Detach { client: 2 }, at(t0, 21_000));
        assert_eq!(p.locked_by(), None);
        // Gone, so after the grace period the size goes home.
        assert_eq!(p.due(), Some(at(t0, 31_000)));
        let d = p.poll(at(t0, 31_000));
        assert_eq!(d, None, "it settles first");
        let d = p.poll(at(t0, 31_150)).unwrap();
        assert_eq!(
            (d.size, d.owner, d.reason),
            (Size::new(120, 40), Some(1), Reason::Returned)
        );
    }

    #[test]
    fn a_record_applied_cancels_the_same_size_waiting() {
        let t0 = Instant::now();
        let mut p = Policy::new(Size::new(80, 24));
        attach(&mut p, 1, true, Size::new(120, 40), t0);
        p.on(Event::Input { client: 1 }, t0);
        // An older client's resize to the same size landed first.
        p.applied(Size::new(120, 40));
        assert_eq!(p.poll(at(t0, 500)), None);
        assert_eq!(p.size(), Size::new(120, 40));
    }

    #[test]
    fn events_from_clients_never_attached_change_nothing() {
        let t0 = Instant::now();
        let mut p: Policy<u8> = Policy::new(Size::new(80, 24));
        p.on(Event::Input { client: 9 }, t0);
        p.on(
            Event::TakeSize {
                client: 9,
                size: Some(Size::new(10, 10)),
            },
            t0,
        );
        p.on(
            Event::LockSize {
                client: 9,
                locked: true,
            },
            t0,
        );
        assert!(!p.typed());
        assert_eq!((p.owner(), p.locked_by(), p.due()), (None, None, None));
    }
}

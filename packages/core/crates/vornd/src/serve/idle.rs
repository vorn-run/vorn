//! Whether vornd as the server, with nobody using it, should stop.
//!
//! It outlives the app, so something has to decide when it is done. Exiting
//! while something is live loses it, and never exiting leaves a server this
//! app may refuse to adopt; so it prefers staying up, and stops only after
//! everything has been empty for the whole window.

use std::time::{Duration, Instant};

/// How long everything must stay empty before vornd stops.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(30 * 60);

/// What could hold the server open, read each tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Terminals whose program runs, whatever their status.
    pub sessions: u64,
    /// Headless agents still running.
    pub headless: u64,
    /// Since a client last said anything: a duration, never a count of
    /// sockets, which have no heartbeat and so may never close.
    pub since_client: Duration,
    /// Since an agent's hook last posted: the only trace of an agent run
    /// outside Vorn.
    pub since_hook: Duration,
    pub pending_permissions: u64,
    pub pending_pairings: u64,
    /// Connector work claimed and not finished.
    pub connector_leases: u64,
    /// Schedules this server runs on its own.
    pub schedules: u64,
    /// Whether it is bound to be reached from the network.
    pub serves_others: bool,
}

/// What holds the server open, if anything.
pub fn holding(s: &Snapshot) -> Option<String> {
    if s.serves_others {
        return Some("this server is bound to be reached from the network".into());
    }
    if s.sessions > 0 {
        return Some(format!("{} session(s)", s.sessions));
    }
    if s.headless > 0 {
        return Some(format!("{} headless agent(s)", s.headless));
    }
    if s.pending_permissions > 0 {
        return Some("an agent is waiting on a permission".into());
    }
    if s.pending_pairings > 0 {
        return Some("a pairing is in progress".into());
    }
    if s.connector_leases > 0 {
        return Some("connector work is outstanding".into());
    }
    (s.schedules > 0).then(|| format!("{} enabled schedule(s)", s.schedules))
}

/// The idle decision over time: when the last hold let go, and the window.
#[derive(Debug)]
pub struct Watch {
    window: Duration,
    quiet_since: Option<Instant>,
}

impl Watch {
    pub fn new(window: Duration) -> Watch {
        Watch {
            window,
            quiet_since: None,
        }
    }

    /// How often to look: a fraction of the window, between a quarter second and a minute.
    pub fn every(&self) -> Duration {
        (self.window / 4).clamp(Duration::from_millis(250), Duration::from_secs(60))
    }

    /// Whether to stop now, at `now`. The most recent of the clocks counts: a
    /// hold letting go, a client speaking, a hook posting.
    pub fn tick(&mut self, s: &Snapshot, now: Instant) -> bool {
        if holding(s).is_some() {
            self.quiet_since = None;
            return false;
        }
        let since = *self.quiet_since.get_or_insert(now);
        let quiet = now
            .duration_since(since)
            .min(s.since_client)
            .min(s.since_hook);
        quiet >= self.window
    }
}

/// The window: `VORN_IDLE_TIMEOUT_MS` when it is a positive number, else [`DEFAULT_WINDOW`].
pub fn window(var: Option<&str>) -> Duration {
    var.and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .map_or(DEFAULT_WINDOW, |ms| Duration::from_millis(ms as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> Snapshot {
        Snapshot {
            since_client: Duration::from_secs(3600),
            since_hook: Duration::from_secs(3600),
            ..Snapshot::default()
        }
    }

    #[test]
    fn says_what_holds_it_open() {
        assert_eq!(holding(&quiet()), None);
        let cases = [
            (
                Snapshot {
                    serves_others: true,
                    ..quiet()
                },
                "network",
            ),
            (
                Snapshot {
                    sessions: 2,
                    ..quiet()
                },
                "2 session(s)",
            ),
            (
                Snapshot {
                    headless: 1,
                    ..quiet()
                },
                "1 headless agent(s)",
            ),
            (
                Snapshot {
                    pending_permissions: 1,
                    ..quiet()
                },
                "permission",
            ),
            (
                Snapshot {
                    pending_pairings: 1,
                    ..quiet()
                },
                "pairing",
            ),
            (
                Snapshot {
                    connector_leases: 1,
                    ..quiet()
                },
                "connector",
            ),
            (
                Snapshot {
                    schedules: 3,
                    ..quiet()
                },
                "3 enabled schedule(s)",
            ),
        ];
        for (s, said) in cases {
            assert!(holding(&s).unwrap().contains(said), "{s:?}");
        }
    }

    #[test]
    fn stops_only_after_the_whole_window_is_empty() {
        let mut w = Watch::new(Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(!w.tick(
            &Snapshot {
                sessions: 1,
                ..quiet()
            },
            t0
        ));
        assert!(!w.tick(&quiet(), t0));
        assert!(!w.tick(&quiet(), t0 + Duration::from_secs(59)));
        assert!(w.tick(&quiet(), t0 + Duration::from_secs(60)));
        // A client that spoke a moment ago keeps it up.
        let mut w = Watch::new(Duration::from_secs(60));
        w.tick(&quiet(), t0);
        let talking = Snapshot {
            since_client: Duration::from_secs(1),
            ..quiet()
        };
        assert!(!w.tick(&talking, t0 + Duration::from_secs(120)));
        // Work restarts the countdown.
        assert!(!w.tick(
            &Snapshot {
                headless: 1,
                ..quiet()
            },
            t0 + Duration::from_secs(121)
        ));
        assert!(!w.tick(&quiet(), t0 + Duration::from_secs(150)));
    }

    #[test]
    fn looks_often_enough_for_its_window() {
        assert_eq!(Watch::new(DEFAULT_WINDOW).every(), Duration::from_secs(60));
        assert_eq!(
            Watch::new(Duration::from_millis(400)).every(),
            Duration::from_millis(250)
        );
        assert_eq!(window(Some("1500")), Duration::from_millis(1500));
        assert_eq!(window(Some("nope")), DEFAULT_WINDOW);
        assert_eq!(window(Some("-1")), DEFAULT_WINDOW);
        assert_eq!(window(None), DEFAULT_WINDOW);
    }
}

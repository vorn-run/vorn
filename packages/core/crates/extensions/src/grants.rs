//! Open panes, each named by a nonce only the window drawing it holds.
//!
//! A page proves which pane and session it is by the nonce in its address;
//! a program pane is closed by it. A grant nobody has used for
//! [`GRANT_IDLE_MS`] is closed the next time it is looked up.

use std::collections::HashMap;

use serde::Serialize;

/// How long an unused grant lives; every use pushes it back.
pub const GRANT_IDLE_MS: u64 = 12 * 60 * 60 * 1000;

/// An open pane, as the app is told about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub nonce: String,
    pub extension_id: String,
    pub pane_id: String,
    pub session_id: String,
    pub project_path: String,
    /// Where a page pane is drawn from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The terminal a program pane runs in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
    /// Milliseconds since the epoch.
    pub expires: u64,
}

/// What looking a nonce up found.
#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Live(Grant),
    /// It had gone stale and is now closed; its terminal, if any, must go too.
    Expired(Grant),
    Unknown,
}

/// Every open pane by nonce.
#[derive(Debug, Default)]
pub struct Grants {
    open: HashMap<String, Grant>,
}

impl Grants {
    pub fn insert(&mut self, grant: Grant) {
        self.open.insert(grant.nonce.clone(), grant);
    }

    /// The grant for `nonce` at `now`, its idle time restarted.
    pub fn touch(&mut self, nonce: &str, now: u64) -> Lookup {
        let Some(grant) = self.open.get_mut(nonce) else {
            return Lookup::Unknown;
        };
        if grant.expires <= now {
            return self
                .open
                .remove(nonce)
                .map_or(Lookup::Unknown, Lookup::Expired);
        }
        grant.expires = now + GRANT_IDLE_MS;
        Lookup::Live(grant.clone())
    }

    /// Closes `nonce`, returning what it was.
    pub fn close(&mut self, nonce: &str) -> Option<Grant> {
        self.open.remove(nonce)
    }

    /// Closes every grant `matches` picks, returning them.
    pub fn close_where(&mut self, matches: impl Fn(&Grant) -> bool) -> Vec<Grant> {
        let nonces: Vec<String> = self
            .open
            .values()
            .filter(|g| matches(g))
            .map(|g| g.nonce.clone())
            .collect();
        nonces.iter().filter_map(|n| self.open.remove(n)).collect()
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(nonce: &str, session: &str, ext: &str) -> Grant {
        Grant {
            nonce: nonce.into(),
            extension_id: ext.into(),
            pane_id: "p".into(),
            session_id: session.into(),
            project_path: "/p".into(),
            url: None,
            terminal_id: None,
            expires: 1000,
        }
    }

    #[test]
    fn a_use_pushes_the_expiry_back_and_a_stale_one_closes() {
        let mut g = Grants::default();
        g.insert(grant("n", "s", "e"));
        let Lookup::Live(live) = g.touch("n", 10) else {
            panic!()
        };
        assert_eq!(live.expires, 10 + GRANT_IDLE_MS);
        assert!(matches!(
            g.touch("n", 10 + GRANT_IDLE_MS),
            Lookup::Expired(_)
        ));
        assert_eq!(g.touch("n", 0), Lookup::Unknown);
        assert!(g.is_empty());
    }

    #[test]
    fn closes_by_nonce_or_by_what_it_belongs_to() {
        let mut g = Grants::default();
        g.insert(grant("a", "s", "e"));
        g.insert(grant("b", "s", "f"));
        g.insert(grant("c", "t", "e"));
        assert_eq!(g.close_where(|x| x.session_id == "s").len(), 2);
        assert_eq!(g.len(), 1);
        assert!(g.close("c").is_some());
        assert!(g.close("c").is_none());
    }
}

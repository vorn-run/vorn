//! Which running Vorn registers the hook endpoint (`~/.vorn/hook-owner`).
//!
//! There is one registration per user: the port and token files beside it
//! and the entries in the agents' settings name one live endpoint. A record
//! naming a live process that is not this one is left alone; one naming a
//! dead process is stale and can be taken. A process that cannot be signalled
//! (another user's) counts as alive, so a registration is never taken from
//! under a live Vorn.

use std::path::Path;

use serde_json::{json, Value};

/// The record: the endpoint's port and the process that wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub port: u16,
    pub pid: u32,
}

impl Owner {
    /// The record's text, or `None` for one that is malformed.
    pub fn parse(text: &str) -> Option<Owner> {
        let v: Value = serde_json::from_str(text).ok()?;
        let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
        let pid = u32::try_from(v.get("pid")?.as_u64()?).ok()?;
        Some(Owner { port, pid })
    }

    /// The record on disk; `None` when absent, unreadable or malformed.
    pub fn read(file: &Path) -> Option<Owner> {
        Owner::parse(&std::fs::read_to_string(file).ok()?)
    }

    pub fn to_json(self) -> String {
        json!({ "port": self.port, "pid": self.pid }).to_string()
    }
}

/// Whether process `me` may write the registration that `owner` holds.
pub fn may_claim(owner: Option<Owner>, me: u32, alive: impl Fn(u32) -> bool) -> bool {
    match owner {
        None => true,
        Some(o) if o.pid == me => true,
        Some(o) => !alive(o.pid),
    }
}

/// Whether process `me`, which `installed` the registration or not, may
/// remove it on the way out. An absent record is what its own stop leaves.
pub fn may_release(owner: Option<Owner>, me: u32, installed: bool) -> bool {
    installed && owner.is_none_or(|o| o.pid == me)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_only_what_no_live_vorn_holds() {
        let other = Some(Owner { port: 1, pid: 7 });
        assert!(may_claim(None, 3, |_| true));
        assert!(may_claim(Some(Owner { port: 1, pid: 3 }), 3, |_| true));
        assert!(!may_claim(other, 3, |_| true));
        assert!(may_claim(other, 3, |_| false));
    }

    #[test]
    fn releases_only_what_it_installed_and_still_holds() {
        assert!(!may_release(None, 3, false));
        assert!(may_release(None, 3, true));
        assert!(may_release(Some(Owner { port: 1, pid: 3 }), 3, true));
        assert!(!may_release(Some(Owner { port: 1, pid: 7 }), 3, true));
    }

    #[test]
    fn reads_the_record_or_nothing() {
        assert_eq!(
            Owner::parse(r#"{"port":56432,"pid":9}"#),
            Some(Owner {
                port: 56432,
                pid: 9
            })
        );
        assert_eq!(Owner::parse(r#"{"port":"x","pid":9}"#), None);
        assert_eq!(Owner::parse("nope"), None);
        let o = Owner { port: 5, pid: 6 };
        assert_eq!(Owner::parse(&o.to_json()), Some(o));
    }
}

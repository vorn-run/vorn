//! Which implementation answers each group of calls.
//!
//! A group is the part of a method name before its first colon: `git:status`
//! and `git:diff` are both `git`. Each group is in one of three modes:
//!
//! - **forward**: the Node server answers: every group vornd does not
//!   implement.
//! - **shadow**: the Node server still answers, and the native implementation
//!   is run on the same input so the two can be compared ([`crate::native`]).
//!   Only calls that change nothing are run twice; a commit made twice is not
//!   a comparison.
//! - **native**: vornd answers and the call never reaches Node, except the
//!   calls the server has to keep, which [`crate::native`] forwards one by
//!   one and says why. Every group in [`NATIVE_GROUPS`] starts here.
//!
//! A per-group setting (`--groups` or `VORND_GROUPS`, such as
//! `git=shadow`) is for tests and comparison runs only.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Forward,
    Shadow,
    Native,
}

impl Mode {
    fn parse(text: &str) -> Option<Mode> {
        match text {
            "forward" => Some(Mode::Forward),
            "shadow" => Some(Mode::Shadow),
            "native" => Some(Mode::Native),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Forward => "forward",
            Mode::Shadow => "shadow",
            Mode::Native => "native",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Groups vornd has a native implementation of, and runs natively.
pub const NATIVE_GROUPS: &[&str] = &[
    "git",
    "file",
    "ide",
    "agent",
    "sessions",
    "shell",
    "connection",
    "connector",
    "server",
    "tailscale",
    "token",
    "pairing",
    "auth",
    "mcp",
    "terminal",
    "headless",
    "worktree",
    "script",
    "workflow",
    "workflowRun",
    "scheduler",
    "webhook",
    "artifact",
    "extension",
    "config",
    "credentials",
    "http",
    "browser",
    "device",
    "bridge",
    "task",
    "project",
    "sessionEvent",
    "widget",
];

/// Why vornd may still hand a call to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillForwarded {
    /// Not native yet.
    NotYetNative,
}

impl StillForwarded {
    pub fn name(self) -> &'static str {
        match self {
            StillForwarded::NotYetNative => "not yet native",
        }
    }
}

/// The calls vornd may still forward to the server, as a whole group or one
/// method. Any other call that reaches the server is a fault the health
/// endpoint reports (`unexpectedForwards`) and the tests fail on. It shrinks
/// to nothing.
pub const STILL_FORWARDED: &[(&str, StillForwarded)] = &[
    ("auth:authenticate", StillForwarded::NotYetNative),
    ("subscribe", StillForwarded::NotYetNative),
    ("credential", StillForwarded::NotYetNative),
    ("session", StillForwarded::NotYetNative),
    ("env", StillForwarded::NotYetNative),
    ("core", StillForwarded::NotYetNative),
    ("ssh", StillForwarded::NotYetNative),
    ("permission", StillForwarded::NotYetNative),
    ("script", StillForwarded::NotYetNative),
    ("server", StillForwarded::NotYetNative),
    ("terminal", StillForwarded::NotYetNative),
    ("git", StillForwarded::NotYetNative),
    ("file", StillForwarded::NotYetNative),
    ("worktree", StillForwarded::NotYetNative),
    ("headless", StillForwarded::NotYetNative),
    ("sessions", StillForwarded::NotYetNative),
    ("shell", StillForwarded::NotYetNative),
    ("agent", StillForwarded::NotYetNative),
];

/// Whether neither vornd nor the server has `method`: no native group, and
/// not one the server still answers.
pub fn unknown(method: &str) -> bool {
    !NATIVE_GROUPS.contains(&group_of(method)) && still_forwarded(method).is_none()
}

/// Whether `method` may still go to the server, and why.
pub fn still_forwarded(method: &str) -> Option<StillForwarded> {
    STILL_FORWARDED
        .iter()
        .find(|(entry, _)| *entry == method || *entry == group_of(method))
        .map(|(_, why)| *why)
}

/// The group a method belongs to: everything before the first colon.
pub fn group_of(method: &str) -> &str {
    method.split_once(':').map_or(method, |(group, _)| group)
}

/// What became of one call, for the counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counted {
    /// The Node server answered it.
    Forwarded,
    /// vornd answered it.
    Native,
    /// Shadowed, and vornd's answer was the server's.
    ShadowMatched,
    /// Shadowed, and vornd's answer differed from the server's.
    ShadowMismatched,
    /// In shadow mode, but nothing native ran: no implementation, a call
    /// that changes something, or one only the server can answer.
    ShadowUnported,
    /// Sent before the connection was admitted, which the server refuses.
    BeforeAuth,
}

/// The switches, and how many calls each group has seen.
#[derive(Debug, Default)]
pub struct Groups {
    modes: BTreeMap<String, Mode>,
    seen: Mutex<BTreeMap<String, GroupCounts>>,
    /// Forwarded calls [`STILL_FORWARDED`] does not allow, by method.
    unexpected: Mutex<BTreeMap<String, u64>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupCounts {
    /// Calls the Node server answered, shadowed ones included.
    pub forwarded: u64,
    /// Calls vornd answered itself.
    pub native: u64,
    /// Shadowed calls whose answers agreed.
    pub shadow_matched: u64,
    /// Shadowed calls whose answers differed.
    pub shadow_mismatched: u64,
    /// Shadow calls that had no native answer to compare with.
    pub shadow_unported: u64,
}

impl Groups {
    /// Every group forwarded.
    pub fn all_forward() -> Groups {
        Groups::default()
    }

    /// Every group in [`NATIVE_GROUPS`] native, then `spec`'s `group=mode`
    /// pairs separated by commas, such as `git=shadow`, on top. A group
    /// neither names is forwarded.
    pub fn new(spec: Option<&str>) -> Result<Groups, String> {
        let mut groups = Groups::parse(spec.unwrap_or(""))?;
        for group in NATIVE_GROUPS {
            groups
                .modes
                .entry((*group).to_owned())
                .or_insert(Mode::Native);
        }
        Ok(groups)
    }

    /// Only `spec`'s modes, every other group forwarded: for tests of one group.
    pub fn parse(spec: &str) -> Result<Groups, String> {
        let mut modes = BTreeMap::new();
        for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (group, mode) = pair
                .split_once('=')
                .ok_or_else(|| format!("expected group=mode, got `{pair}`"))?;
            let group = group.trim();
            if group.is_empty() || group.contains(':') {
                return Err(format!("`{group}` is not a group name"));
            }
            let mode = Mode::parse(mode.trim()).ok_or_else(|| {
                format!(
                    "unknown mode `{}` for {group}: use forward, shadow or native",
                    mode.trim()
                )
            })?;
            if mode == Mode::Native && !NATIVE_GROUPS.contains(&group) {
                return Err(format!("{group} has no native implementation yet"));
            }
            modes.insert(group.to_string(), mode);
        }
        Ok(Groups {
            modes,
            seen: Mutex::default(),
            unexpected: Mutex::default(),
        })
    }

    pub fn mode(&self, group: &str) -> Mode {
        self.modes.get(group).copied().unwrap_or(Mode::Forward)
    }

    /// The mode of the group `method` belongs to.
    pub fn route(&self, method: &str) -> Mode {
        self.mode(group_of(method))
    }

    /// Counts one call of `method`'s group. A shadowed call also counts as
    /// forwarded: the server answered it.
    pub fn count(&self, method: &str, what: Counted) {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let counts = seen.entry(group_of(method).to_string()).or_default();
        match what {
            Counted::Forwarded => {
                counts.forwarded += 1;
                if still_forwarded(method).is_none() {
                    *self
                        .unexpected
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .entry(method.to_owned())
                        .or_default() += 1;
                }
            }
            Counted::BeforeAuth => counts.forwarded += 1,
            Counted::Native => counts.native += 1,
            Counted::ShadowMatched => counts.shadow_matched += 1,
            Counted::ShadowMismatched => counts.shadow_mismatched += 1,
            Counted::ShadowUnported => counts.shadow_unported += 1,
        }
    }

    /// The groups with a switch set, and their modes.
    pub fn modes(&self) -> impl Iterator<Item = (&str, Mode)> {
        self.modes.iter().map(|(g, m)| (g.as_str(), *m))
    }

    /// Whether any group is native or shadowed, so vornd has native work to
    /// prepare for.
    pub fn any_native(&self) -> bool {
        self.modes.values().any(|m| *m != Mode::Forward)
    }

    pub fn counts(&self) -> BTreeMap<String, GroupCounts> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The calls forwarded that [`STILL_FORWARDED`] does not allow, by method.
    pub fn unexpected_forwards(&self) -> BTreeMap<String, u64> {
        self.unexpected
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_is_the_method_before_its_first_colon() {
        assert_eq!(group_of("git:status"), "git");
        assert_eq!(group_of("connector:pack:install"), "connector");
        assert_eq!(group_of("ping"), "ping");
    }

    #[test]
    fn every_group_forwards_unless_told_otherwise() {
        let groups = Groups::parse("git=shadow").unwrap();
        assert_eq!(groups.mode("git"), Mode::Shadow);
        assert_eq!(groups.mode("terminal"), Mode::Forward);
        assert_eq!(Groups::all_forward().mode("git"), Mode::Forward);
        assert!(!Groups::all_forward().any_native());
    }

    #[test]
    fn every_implemented_group_runs_natively_and_a_setting_wins() {
        let groups = Groups::new(None).unwrap();
        for group in NATIVE_GROUPS {
            assert_eq!(groups.mode(group), Mode::Native, "{group}");
        }
        assert_eq!(groups.mode("workflow"), Mode::Native);
        assert_eq!(groups.mode("permission"), Mode::Forward);
        let shadowed = Groups::new(Some("git=shadow,file=forward")).unwrap();
        assert_eq!(shadowed.mode("git"), Mode::Shadow);
        assert_eq!(shadowed.mode("file"), Mode::Forward);
        assert_eq!(shadowed.mode("ide"), Mode::Native);
    }

    #[test]
    fn refuses_native_for_a_group_nothing_implements() {
        let err = Groups::parse("permission=native").unwrap_err();
        assert!(err.contains("no native implementation"), "{err}");
        assert_eq!(
            Groups::parse("git=native").unwrap().mode("git"),
            Mode::Native
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(Groups::parse("git").is_err());
        assert!(Groups::parse("git=fast").is_err());
        assert!(Groups::parse("git:status=forward").is_err());
        assert!(Groups::parse(" , ").unwrap().modes().next().is_none());
    }

    #[test]
    fn counts_each_outcome_per_group() {
        let groups = Groups::parse("git=shadow").unwrap();
        assert_eq!(groups.route("git:status"), Mode::Shadow);
        groups.count("git:status", Counted::Forwarded);
        groups.count("git:status", Counted::ShadowMatched);
        groups.count("git:commit", Counted::Forwarded);
        groups.count("git:commit", Counted::ShadowUnported);
        groups.count("file:listDir", Counted::Native);
        groups.count("task:list", Counted::Forwarded);
        let counts = groups.counts();
        assert_eq!(
            counts["git"],
            GroupCounts {
                forwarded: 2,
                shadow_matched: 1,
                shadow_unported: 1,
                ..GroupCounts::default()
            }
        );
        assert_eq!(counts["file"].native, 1);
        assert_eq!(counts["task"].forwarded, 1);
    }

    #[test]
    fn reports_a_forward_the_list_does_not_allow() {
        let groups = Groups::new(None).unwrap();
        groups.count("task:list", Counted::Forwarded);
        groups.count("config:save", Counted::Forwarded);
        groups.count("config:save", Counted::Forwarded);
        groups.count("config:load", Counted::Native);
        assert_eq!(
            groups.unexpected_forwards(),
            BTreeMap::from([("config:save".to_owned(), 2)])
        );
        assert_eq!(
            still_forwarded("task:list"),
            Some(StillForwarded::NotYetNative)
        );
        assert_eq!(still_forwarded("config:load"), None);
        assert_eq!(still_forwarded("browser:navigate"), None);
        groups.count("config:load", Counted::BeforeAuth);
        assert_eq!(groups.unexpected_forwards().len(), 1);
        assert!(unknown("nonexistent:method"));
        assert!(!unknown("config:save"));
        assert!(!unknown("task:list"));
    }

    /// Every call a client can make, from the protocol's request map, is
    /// answered by vornd unless [`STILL_FORWARDED`] names it.
    #[test]
    fn every_protocol_call_is_native_or_listed() {
        let protocol = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../shared/src/protocol.ts");
        let text = std::fs::read_to_string(protocol)
            .expect("the protocol")
            .replace("\r\n", "\n");
        let start = text
            .find("export interface RequestMethods")
            .expect("the request map");
        let body = &text[start..];
        let body = &body[..body.find("\n}\n").expect("its end")];
        let methods: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix('\''))
            .filter_map(|l| l.split_once('\'').map(|(m, _)| m))
            .filter(|m| m.contains(':'))
            .collect();
        assert!(methods.len() > 150, "read {} methods", methods.len());
        let missing: Vec<&str> = methods
            .into_iter()
            .filter(|m| crate::native::effect(m).is_none() && still_forwarded(m).is_none())
            .collect();
        assert!(missing.is_empty(), "forwarded and not listed: {missing:?}");
    }
}

//! Which implementation answers each group of calls.
//!
//! A group is the part of a method name before its first colon: `git:status`
//! and `git:diff` are both `git`. Each group is in one of three modes:
//!
//! - **forward**: the Node server answers. Every group starts here.
//! - **shadow**: the Node server still answers, and the native implementation
//!   is run on the same input so the two can be compared ([`crate::native`]).
//!   Only calls that change nothing are run twice; a commit made twice is not
//!   a comparison.
//! - **native**: vornd answers and the call never reaches Node, except the
//!   calls the server has to keep, which [`crate::native`] forwards one by
//!   one and says why.
//!
//! The Native server switch in Settings › Experimental (`--native-server`)
//! puts every group in [`NATIVE_SERVER_GROUPS`] in native mode. A per-group
//! setting (`--groups` or `VORND_GROUPS`, such as `git=shadow`) is for
//! development and comparison runs, and wins over the switch.

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

/// Groups vornd has a native implementation of, which a per-group setting
/// may put in native mode.
pub const NATIVE_GROUPS: &[&str] = &[
    "git",
    "file",
    "ide",
    "agent",
    "sessions",
    "shell",
    "connection",
    "connector",
];

/// Groups whose reads vornd can answer from its copy of the server's
/// session records ([`crate::registry`]), to compare with the server's in
/// shadow mode, while the server still owns them: `native` is refused.
pub const SHADOW_GROUPS: &[&str] = &["terminal", "headless", "worktree"];

/// Groups the Native server switch runs natively: the one place a group
/// joins the switch. Each is also in [`NATIVE_GROUPS`].
pub const NATIVE_SERVER_GROUPS: &[&str] = &[
    "git",
    "file",
    "ide",
    "agent",
    "sessions",
    "shell",
    "connection",
    "connector",
];

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
}

/// The switches, and how many calls each group has seen.
#[derive(Debug, Default)]
pub struct Groups {
    modes: BTreeMap<String, Mode>,
    seen: Mutex<BTreeMap<String, GroupCounts>>,
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

    /// The modes for a vornd started with the Native server switch on or
    /// off, then `spec`'s `group=mode` pairs separated by commas, such as
    /// `git=shadow,terminal=forward`, on top. A group neither names is
    /// forwarded.
    pub fn new(native_server: bool, spec: Option<&str>) -> Result<Groups, String> {
        let mut modes = BTreeMap::new();
        if native_server {
            for group in NATIVE_SERVER_GROUPS {
                modes.insert((*group).to_owned(), Mode::Native);
            }
        }
        for pair in spec
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
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
        })
    }

    /// `spec`'s modes with the Native server switch off.
    pub fn parse(spec: &str) -> Result<Groups, String> {
        Groups::new(false, Some(spec))
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
            Counted::Forwarded => counts.forwarded += 1,
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
    fn the_switch_runs_every_joined_group_natively_and_a_setting_wins() {
        let on = Groups::new(true, None).unwrap();
        for group in NATIVE_SERVER_GROUPS {
            assert_eq!(on.mode(group), Mode::Native, "{group}");
        }
        assert_eq!(on.mode("worktree"), Mode::Forward);
        assert_eq!(on.mode("task"), Mode::Forward);
        let shadowed = Groups::new(true, Some("git=shadow,file=forward")).unwrap();
        assert_eq!(shadowed.mode("git"), Mode::Shadow);
        assert_eq!(shadowed.mode("file"), Mode::Forward);
        assert_eq!(shadowed.mode("ide"), Mode::Native);
        assert_eq!(Groups::new(false, None).unwrap().mode("git"), Mode::Forward);
    }

    #[test]
    fn only_groups_with_an_implementation_join_the_switch() {
        for group in NATIVE_SERVER_GROUPS {
            assert!(NATIVE_GROUPS.contains(group), "{group}");
        }
    }

    #[test]
    fn a_group_vornd_only_compares_may_be_shadowed_and_not_answered() {
        for group in SHADOW_GROUPS {
            assert!(!NATIVE_GROUPS.contains(group), "{group}");
            assert!(!NATIVE_SERVER_GROUPS.contains(group), "{group}");
            assert_eq!(
                Groups::parse(&format!("{group}=shadow"))
                    .unwrap()
                    .mode(group),
                Mode::Shadow
            );
            assert!(Groups::parse(&format!("{group}=native")).is_err());
        }
    }

    #[test]
    fn refuses_native_for_a_group_nothing_implements() {
        let err = Groups::parse("worktree=native").unwrap_err();
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
}

//! Which implementation answers each group of calls.
//!
//! A group is the part of a method name before its first colon: `git:status`
//! and `git:diff` are both `git`. Each group is in one of three modes:
//!
//! - **forward**: the Node server answers. Every group starts here.
//! - **shadow**: the Node server still answers, and the native implementation
//!   is run on the same input so the two can be compared.
//! - **native**: vornd answers and the call never reaches Node.
//!
//! No group has a native implementation yet, so `native` is refused when the
//! switches are read, and `shadow` forwards and counts what it could not
//! compare. The modes exist now so a group can move from one to the next
//! without changing how the switch is set.

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

/// Groups vornd can answer itself. Empty until the first group is ported.
pub const NATIVE_GROUPS: &[&str] = &[];

/// The group a method belongs to: everything before the first colon.
pub fn group_of(method: &str) -> &str {
    method.split_once(':').map_or(method, |(group, _)| group)
}

/// The switches, and how many calls each group has seen.
#[derive(Debug, Default)]
pub struct Groups {
    modes: BTreeMap<String, Mode>,
    seen: Mutex<BTreeMap<String, GroupCounts>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupCounts {
    /// Calls the Node server answered.
    pub forwarded: u64,
    /// Calls in shadow mode that had no native implementation to compare with.
    pub shadow_unported: u64,
}

impl Groups {
    /// Every group forwarded.
    pub fn all_forward() -> Groups {
        Groups::default()
    }

    /// Reads `group=mode` pairs separated by commas, such as
    /// `git=shadow,terminal=forward`. A group not named is forwarded.
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
        })
    }

    pub fn mode(&self, group: &str) -> Mode {
        self.modes.get(group).copied().unwrap_or(Mode::Forward)
    }

    /// Records a call from a client and says who answers it. Until a group is
    /// ported, that is always the Node server.
    pub fn route(&self, method: &str) -> Mode {
        let group = group_of(method);
        let mode = self.mode(group);
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let counts = seen.entry(group.to_string()).or_default();
        counts.forwarded += 1;
        if mode == Mode::Shadow {
            counts.shadow_unported += 1;
        }
        mode
    }

    /// The groups with a switch set, and their modes.
    pub fn modes(&self) -> impl Iterator<Item = (&str, Mode)> {
        self.modes.iter().map(|(g, m)| (g.as_str(), *m))
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
    }

    #[test]
    fn refuses_native_for_a_group_nothing_implements() {
        let err = Groups::parse("git=native").unwrap_err();
        assert!(err.contains("no native implementation"), "{err}");
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(Groups::parse("git").is_err());
        assert!(Groups::parse("git=fast").is_err());
        assert!(Groups::parse("git:status=forward").is_err());
        assert!(Groups::parse(" , ").unwrap().modes().next().is_none());
    }

    #[test]
    fn counts_calls_per_group_and_what_shadow_could_not_compare() {
        let groups = Groups::parse("git=shadow").unwrap();
        assert_eq!(groups.route("git:status"), Mode::Shadow);
        assert_eq!(groups.route("git:diff"), Mode::Shadow);
        assert_eq!(groups.route("task:list"), Mode::Forward);
        let counts = groups.counts();
        assert_eq!(
            counts["git"],
            GroupCounts {
                forwarded: 2,
                shadow_unported: 2
            }
        );
        assert_eq!(counts["task"].shadow_unported, 0);
    }
}

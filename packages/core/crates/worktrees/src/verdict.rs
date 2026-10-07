//! The safest action for one worktree (`computeVerdict`).

use serde::Serialize;

/// What a worktree is to git.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Git knows it.
    Registered,
    /// On disk under `.vorn-worktrees` only; git has forgotten it.
    OrphanDir,
}

/// The action a verdict names, most protective first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Keep,
    Orphan,
    Review,
    Reclaim,
    Remove,
}

/// The safest action for a worktree, the bytes it frees and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub level: Level,
    pub frees_bytes: u64,
    pub reasons: Vec<String>,
    /// Whether "Select suggested" ticks the row.
    pub auto_select: bool,
}

/// What a verdict is decided from.
#[derive(Clone, Copy, Debug)]
pub struct VerdictInput {
    pub is_main: bool,
    pub kind: Kind,
    pub is_dirty: bool,
    pub is_merged: bool,
    pub has_upstream: bool,
    pub active_sessions: usize,
    pub is_pinned: bool,
    pub size_bytes: u64,
    pub artifact_bytes: u64,
    pub idle_days: Option<u64>,
}

/// Anything that could lose work stops at `review`, and only a merged,
/// clean worktree idle for `idle_days_threshold` days is pre-selected.
pub fn verdict(input: &VerdictInput, idle_days_threshold: f64) -> Verdict {
    let keep = |reason: String| Verdict {
        level: Level::Keep,
        frees_bytes: 0,
        reasons: vec![reason],
        auto_select: false,
    };
    let other = |level, frees_bytes, reasons: &[&str]| Verdict {
        level,
        frees_bytes,
        reasons: reasons.iter().map(|r| (*r).to_owned()).collect(),
        auto_select: false,
    };
    if input.is_main {
        return keep("main worktree".into());
    }
    if input.active_sessions > 0 {
        let n = input.active_sessions;
        return keep(format!(
            "{n} active session{}",
            if n > 1 { "s" } else { "" }
        ));
    }
    if input.is_pinned {
        return keep("pinned".into());
    }
    if input.kind == Kind::OrphanDir {
        return other(
            Level::Orphan,
            input.size_bytes,
            &["not registered with git"],
        );
    }
    if input.is_dirty {
        return other(Level::Review, 0, &["uncommitted changes"]);
    }
    if !input.is_merged && !input.has_upstream {
        return other(Level::Review, 0, &["unmerged and never pushed"]);
    }
    if !input.is_merged {
        return other(
            Level::Reclaim,
            input.artifact_bytes,
            &["unmerged but pushed", "build output can go"],
        );
    }
    let mut reasons = vec!["merged".to_owned()];
    if let Some(days) = input.idle_days {
        reasons.push(format!(
            "idle {days} day{}",
            if days == 1 { "" } else { "s" }
        ));
    }
    Verdict {
        level: Level::Remove,
        frees_bytes: input.size_bytes,
        reasons,
        auto_select: input
            .idle_days
            .is_some_and(|d| d as f64 >= idle_days_threshold),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged() -> VerdictInput {
        VerdictInput {
            is_main: false,
            kind: Kind::Registered,
            is_dirty: false,
            is_merged: true,
            has_upstream: false,
            active_sessions: 0,
            is_pinned: false,
            size_bytes: 100,
            artifact_bytes: 40,
            idle_days: Some(20),
        }
    }

    fn said(input: VerdictInput) -> (Level, u64, Vec<String>, bool) {
        let v = verdict(&input, 14.0);
        (v.level, v.frees_bytes, v.reasons, v.auto_select)
    }

    #[test]
    fn protects_work_before_it_frees_space() {
        let strs = |r: &[&str]| r.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        // Each guard wins over everything after it.
        let main = VerdictInput {
            is_main: true,
            active_sessions: 2,
            ..merged()
        };
        assert_eq!(
            said(main),
            (Level::Keep, 0, strs(&["main worktree"]), false)
        );
        let busy = VerdictInput {
            active_sessions: 2,
            is_dirty: true,
            ..merged()
        };
        assert_eq!(
            said(busy),
            (Level::Keep, 0, strs(&["2 active sessions"]), false)
        );
        let one = VerdictInput {
            active_sessions: 1,
            ..merged()
        };
        assert_eq!(said(one).2, strs(&["1 active session"]));
        let pinned = VerdictInput {
            is_pinned: true,
            kind: Kind::OrphanDir,
            ..merged()
        };
        assert_eq!(said(pinned), (Level::Keep, 0, strs(&["pinned"]), false));
        let orphan = VerdictInput {
            kind: Kind::OrphanDir,
            is_dirty: true,
            ..merged()
        };
        assert_eq!(
            said(orphan),
            (
                Level::Orphan,
                100,
                strs(&["not registered with git"]),
                false
            )
        );
        let dirty = VerdictInput {
            is_dirty: true,
            ..merged()
        };
        assert_eq!(
            said(dirty),
            (Level::Review, 0, strs(&["uncommitted changes"]), false)
        );
        let local = VerdictInput {
            is_merged: false,
            ..merged()
        };
        assert_eq!(
            said(local),
            (
                Level::Review,
                0,
                strs(&["unmerged and never pushed"]),
                false
            )
        );
        let pushed = VerdictInput {
            is_merged: false,
            has_upstream: true,
            ..merged()
        };
        assert_eq!(
            said(pushed),
            (
                Level::Reclaim,
                40,
                strs(&["unmerged but pushed", "build output can go"]),
                false
            )
        );
    }

    #[test]
    fn preselects_a_merged_worktree_only_once_it_has_been_idle_long_enough() {
        assert_eq!(
            said(merged()),
            (
                Level::Remove,
                100,
                vec!["merged".into(), "idle 20 days".into()],
                true
            )
        );
        let recent = VerdictInput {
            idle_days: Some(1),
            ..merged()
        };
        assert_eq!(
            said(recent).2,
            vec!["merged".to_owned(), "idle 1 day".to_owned()]
        );
        assert!(!said(recent).3);
        let unknown = VerdictInput {
            idle_days: None,
            ..merged()
        };
        assert_eq!(
            said(unknown),
            (Level::Remove, 100, vec!["merged".into()], false)
        );
        // A threshold of zero pre-selects every removable worktree.
        assert!(verdict(&recent, 0.0).auto_select);
        assert!(verdict(&recent, 1.0).auto_select);
        assert!(!verdict(&recent, 1.5).auto_select);
    }
}
